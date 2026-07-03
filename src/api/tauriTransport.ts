// TauriTransport — Transport 의 Tauri(app.emit/invoke/Channel) carrier 구현 (T7c, ADR-0036, Fix-B/Fix-C).
//
// WsTransport 가 창 단위로 데몬 WS 에 직접 붙던 구조를 src-tauri DaemonClient 로 끌어올린
// 뒤, 프론트는 이 transport 를 통해 Rust 쪽과 통신한다. 3개 통신 평면:
//
//  ① control 방송(목록/상태/프로필) : Rust `app.emit(event)` → `listen` → control InboundMessage.
//  ② command 상행                  : `invoke('forward_daemon_command', { cmd })` → Rust → 데몬 WS.
//  ③-a reply 하행(②의 답)          : forward_daemon_command 반환(reply AgentEvent) → control InboundMessage.
//  ③-b output 하행(터미널 출력)     : per-window Tauri Channel(`subscribe_output` invoke) → raw bytes →
//                                    decodeOutputFrame → output InboundMessage.
//
// ★WsTransport 동형(계약 미러)★: WsTransport 가 WS Text frame→control / WS binary frame→output 으로
//   올리는 것과 동일한 InboundMessage 형태로 reply·output 을 올린다. 그래서 ProtocolClient 는 carrier
//   가 WS 인지 Tauri 인지 모른 채 기존 로직(pending 매칭 / handleOutput)으로 처리한다(ProtocolClient 무수정).
//
// ★분기점★: clientFactory.ts 에서 WsTransport → TauriTransport 로 교체하면 프론트가 Rust 연결 단일화
//   경로를 탄다(창이 몇 개든 데몬엔 Rust 연결 1개 — ADR-0036 목표).
//
// ## ★연결 상태 단일 진실원 = Rust emit (Fix-C ①)★
// 연결 상태(connected/reconnecting/down)는 **Rust DaemonClient 가 보내는 `daemon-connection-state`
// 이벤트(u5)** 가 단일 진실원이다. 프론트(doConnect)는 절대 임의로 connected 로 만들지 않는다 — Rust
// 가 Hello 수신/재연결 성공/끊김을 권위적으로 알고 emit 하므로, 프론트가 invoke resolve 만 보고
// connected 를 추정하면 Rust 의 실제 상태(예: stale 폐기·재연결 중)와 어긋난다. doConnect 는 연결
// "시도"(invoke + 출력 Channel 등록)만 하고, 상태 전이는 u5 가 반영한다.

import { Channel, invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

import type { ConnectionState } from './agentClient'
import type { InboundMessage, Transport } from './transport'
import { decodeOutputFrame } from './wsFrame'

export class TauriTransport implements Transport {
  private _state: ConnectionState = 'down'
  private stateListeners = new Set<(s: ConnectionState) => void>()
  private messageCb: ((msg: InboundMessage) => void) | null = null

  // Tauri event 리스너 해제 함수 목록.
  private unlisten: Array<() => void> = []

  // ★출력 Channel(③-b)★: 이 창의 모든 agent 출력을 운반하는 per-window Channel(spike §7 D4 — 프레임에
  //   agent_id 태그 내장). connected 시 subscribe_output invoke 로 Rust registry 에 1회 등록한다.
  //   raw bytes(Response::new) 를 받아 decodeOutputFrame → output InboundMessage 로 올린다(WsTransport
  //   binary arm 과 동형). #13133: 정리는 null 대입이 아니라 delete channel.onmessage.
  private outputChannel: Channel<ArrayBuffer> | null = null

  // ★출력 Channel 등록 single-flight(FIX 6)★: 겹치는 등록(예: self-heal pull + 전이 이벤트가 근접)이
  //   동시에 돌면 각자 새 Channel 을 만들어 옛 onmessage 를 delete 한 뒤 subscribe_output 을 await 한다.
  //   더 오래된 invoke 가 Rust 에 *나중에* 도착하면 Rust 는 그 (onmessage 이미 delete 된) Channel 을
  //   마지막 window_label 로 붙잡아 출력이 조용히 두절된다 — 프론트는 어느 invoke 가 Rust 에 마지막으로
  //   닿을지 통제 못 한다. 그래서 등록을 직렬화한다: 진행 중이면 이 promise 를 공유(재사용)하고, 진행 중에
  //   또 등록 요청이 들어오면 pending 플래그만 세워 완료 후 정확히 1회 재등록한다(동시 invoke 0 → Rust 가
  //   붙드는 Channel 이 항상 마지막 완료분과 일치).
  private outputChannelInflight: Promise<void> | null = null
  // 진행 중 등록이 완료된 뒤 재등록이 필요한지(진행 중에 추가 요청이 들어왔음). 여러 요청을 1회로 합친다.
  private outputChannelRerun = false

  // 연결 중복 방지(connect/ensureReady 동시 호출). 진행 중이면 모든 호출자가 이 promise 를 공유한다.
  private connectPromise: Promise<void> | null = null

  // 명시 종료 플래그 — ensureReady 가 down 인데 재연결을 시도하지 않게.
  private closedByUser = false

  // ★연결 세대(close 세대 가드, Fix-C ①)★: close() 가 호출될 때마다 +1. in-flight doConnect 는 진입
  //   시점 세대(myGen)를 캡처하고, invoke resolve 후 출력 Channel 등록 전에 자기 세대가 여전히 current
  //   인지(=그 사이 close() 가 끼지 않았는지) 재확인한다. 뒤늦게 완료된 stale doConnect 가 출력 Channel
  //   을 등록(좀비)하거나 연결을 부활시키지 않게 한다. ★상태(connected) 부활 자체는 Rust emit 단일
  //   진실원이라 doConnect 가 setState 를 안 하므로 구조적으로 막히지만, 출력 Channel 등록·side-effect
  //   는 이 가드로 막는다.
  private generation = 0

  // ★상태 적용 버전(selfHeal pull-vs-event 레이스 가드, FIX 5)★: applyConnectionState 가 실제로 상태를
  //   반영할 때마다 +1. selfHeal 은 invoke 전 이 값을 캡처하고, pull 결과를 적용하기 직전 값이 그대로인지
  //   본다 — 그 사이 실제 daemon-connection-state 이벤트가 하나라도 끼어 applyConnectionState 를 돌렸으면
  //   pull 스냅샷은 이벤트보다 오래된 것이라 폐기한다. (기존 last-write 는 순서가 pull 뒤 이벤트일 때만
  //   맞고, 이벤트 뒤 pull 이면 낡은 스냅샷이 새 상태를 덮어써 역전된다.)
  private stateVersion = 0

  get connectionState(): ConnectionState {
    return this._state
  }

  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void {
    this.stateListeners.add(cb)
    // 등록 즉시 현재 상태 1회 통지(WsTransport 동형 — ProtocolClient 초기 상태 인식).
    cb(this._state)
    return () => {
      this.stateListeners.delete(cb)
    }
  }

  onMessage(cb: (msg: InboundMessage) => void): () => void {
    this.messageCb = cb
    return () => {
      if (this.messageCb === cb) this.messageCb = null
    }
  }

  private setState(s: ConnectionState): void {
    if (this._state === s) return
    this._state = s
    for (const cb of this.stateListeners) cb(s)
  }

  // ── 연결 상태 반영(단일 진실원 핸들러) ───────────────────────────────────────
  // `daemon-connection-state` 이벤트 핸들러와 self-heal pull 조회가 *공유* 하는 단일 경로다.
  // raw 문자열(Rust emit / pull command 반환 — 같은 어휘: connected/reconnecting/down)을 받아
  // ConnectionState 로 정규화하고, 비-connected → connected (재)전이 시에만 출력 Channel 을 (재)등록한다.
  // 첫 연결·Rust 내부 재연결·리로드 self-heal 을 한 경로로 통일한다(중복 등록 없음).
  //
  // ★멱등(더블 등록/이벤트 레이스 가드)★: 이미 connected 였으면(wasConnected) 출력 Channel 등록을
  //   생략한다 — self-heal pull 과 실제 전이 이벤트가 겹쳐도 등록은 정확히 전이당 1회다. self-heal 은
  //   조회 결과를 캐시하지 않고 이 핸들러에 즉시 흘려보내므로(아래 selfHeal 참조), 조회와 응답 사이에
  //   'down' 이벤트가 끼면 그 이벤트가 마지막에 이겨 setState('down')이 유지된다(last-write).
  private applyConnectionState(raw: string): void {
    let state: ConnectionState
    if (raw === 'connected') {
      state = 'connected'
    } else if (raw === 'reconnecting') {
      state = 'reconnecting'
    } else if (raw === 'down') {
      state = 'down'
    } else {
      // ★미지 어휘 방어(retrofit 함정)★: Rust 가 나중에 새 상태(예: 'connecting')를 emit 하면 조용히
      //   오역하지 않게, 알 수 없는 문자열은 안전측(down)으로 강등하고 1회 경고한다. 셋 중 하나로 좁혀야
      //   ConnectionState 계약이 깨지지 않는다.
      console.warn(`[TauriTransport] 알 수 없는 연결 상태 '${raw}' — down 으로 처리`)
      state = 'down'
    }
    // ★버전 bump(FIX 5)★: 상태를 실제로 반영하는 매 호출마다 +1 — selfHeal 이 이걸로 "내 pull 이후
    //   이벤트가 끼었나"를 판별한다. setState 가 no-op(동일 상태)이어도 이벤트가 온 사실 자체를 표시해야
    //   하므로 setState 여부와 무관하게 올린다.
    this.stateVersion += 1
    const wasConnected = this._state === 'connected'
    this.setState(state)
    // 비-connected → connected (재)전이 시 출력 Channel (재)등록. 첫 연결·재연결·self-heal 단일 경로.
    if (state === 'connected' && !wasConnected) {
      this.registerOutputChannel().catch((err: unknown) => {
        console.warn('[TauriTransport] 출력 Channel 등록 실패:', err)
      })
    }
  }

  // ── 리로드 자가복구(Fix-D — pull 조회 1회) ───────────────────────────────────
  // ★왜 필요한가(결함)★: Rust DaemonClient 는 `daemon-connection-state` 를 상태 *전이* 시에만 emit
  //   한다 — connect()/ensure() 는 이미 Connected 면 emit 없이 Ok 로 단락한다(mod.rs connect/ensure
  //   진입 단축 + connection.rs 는 전이에서만 app.emit). 그래서 웹뷰가 리로드되면 TauriTransport 가
  //   _state='down' 으로 재생성되는데, 데몬이 이미 Connected 라 어떤 전이 이벤트도 오지 않는다 →
  //   이 창은 연결을 영영 못 알아채 출력 Channel(subscribe_output)을 등록하지 못한다 → 그 창의 모든
  //   slot 에서 replay/live 출력이 전부 두절된다(창 단위 사각지대).
  //
  //   전이 emit(push) 모델의 사각지대를 pull 조회로 1회 메운다: 리스너 등록이 끝난 뒤 현재 상태를
  //   직접 물어, connected 인데 _state 가 아직 그렇지 않으면 이벤트 핸들러와 *같은 경로*
  //   (applyConnectionState)로 흘려보내 setState('connected') + 출력 Channel 등록을 유발한다.
  //
  // ★레이스 가드(critical, FIX 5)★: 조회와 응답 사이에 실제 `daemon-connection-state` 이벤트가 끼어들
  //   수 있다(진짜 전이). 두 순서 다 방어한다: (1) event→pull(이벤트 먼저): stateVersion 캡처-비교로
  //   pull 결과를 폐기한다 — invoke 대기 중 이벤트가 applyConnectionState 를 돌리면 버전이 바뀌어 낡은
  //   pull 스냅샷이 최신 상태를 덮어쓰지 못한다(이벤트 승). (2) pull→event(pull 먼저): pull 이 반영된 뒤
  //   온 이벤트는 정상 경로로 그대로 이긴다(pull 은 no-op 아니라 실제 전이 이벤트가 나중에 반영). 또한
  //   applyConnectionState 의 wasConnected 가드가 이미 connected 면 출력 Channel 재등록을 생략하므로
  //   이중 등록도 없다. 조회가 non-connected 를 반환하면 아무 것도 하지 않는다(이후 전이 이벤트가 처리).
  private async selfHeal(): Promise<void> {
    // ★레이스: pull 스냅샷은 이벤트보다 오래됐을 수 있다(FIX 5)★. invoke await 동안 실제
    //   daemon-connection-state 이벤트가 끼면(예: 'down' 전이) pull 결과는 그보다 낡은 스냅샷이다.
    //   invoke 전 stateVersion 을 캡처해, 응답을 적용하기 직전 값이 그대로면(이벤트가 하나도 안 낌)
    //   적용하고, 바뀌었으면(이벤트가 끼어 최신 상태를 이미 반영) pull 결과를 폐기한다. 이벤트가 항상
    //   이긴다 — 순서가 event→pull 이어도 낡은 스냅샷이 새 상태를 덮어쓰지 않는다.
    const versionBefore = this.stateVersion
    let raw: string
    try {
      raw = await invoke<string>('daemon_connection_state')
    } catch (e) {
      // 조회 실패(초기화 레이스 등)는 치명적 아님 — 이후 실제 전이 이벤트가 상태를 채운다.
      console.warn('[TauriTransport] 연결 상태 self-heal 조회 실패:', e)
      return
    }
    if (this.stateVersion !== versionBefore) {
      // invoke 대기 중 실제 이벤트가 끼었다 — pull 스냅샷은 stale 이므로 폐기(이벤트가 최신 권위).
      return
    }
    // 이벤트가 안 낀 경우에만 조회 결과를 핸들러에 흘린다(캐시 없이). 이미 connected 면 applyConnectionState
    //   가 wasConnected 가드로 등록 no-op — 이중 등록 차단.
    this.applyConnectionState(raw)
  }

  // ── Tauri 이벤트 구독 등록 ──────────────────────────────────────────────────
  // Rust DaemonClient 가 `app.emit(event, payload)` 로 push 하는 이벤트를 수신해
  // InboundMessage(control)로 정규화해 ProtocolClient 로 올린다.
  //
  // ★멱등(MED-1)★: 이미 등록돼 있으면(unlisten 비어있지 않음) no-op. doConnect 가 매 연결마다 부르되
  //   close()→cleanupListeners 로 비워진 뒤의 재연결에서만 실제 재등록한다(이중 리스너 방지).
  //
  // ★부분 등록 정리(Fix-C ①, low)★: listen() 중간에 하나라도 실패하면 *앞서 등록한* 리스너를 즉시
  //   해제하고 throw 한다. 이전엔 5개 전부 성공해야 unlisten 에 저장했어서, 중간 실패 시 앞 리스너가
  //   해제 경로 없이 누수됐다. 부분 등록분을 모아 두고 실패 시 전부 off 한다.
  //
  // ★in-flight close 가드(Fix-C ①)★: listen() 은 async 라, registerListeners 가 await 중인 사이 close()
  //   가 동기로 끼면 close 의 cleanupListeners 는 아직 빈 `this.unlisten` 을 본다 → 그 후 등록이 완료돼
  //   `this.unlisten = registered` 가 되면 *close 후에 좀비 리스너가 살아남는다*(close 가 못 정리한 것).
  //   그래서 등록 완료 시점에 진입 세대(myGen)가 여전히 current 인지 본다 — close() 가 generation 을
  //   올렸으면(stale) 방금 등록한 것을 즉시 해제하고 this.unlisten 에 저장하지 않는다.
  private async registerListeners(): Promise<void> {
    if (this.unlisten.length > 0) return
    const myGen = this.generation
    const registered: Array<() => void> = []
    try {
      registered.push(
        await listen<unknown>('agent-list-updated', (e) => {
          this.messageCb?.({
            kind: 'control',
            event: { AgentListUpdated: { agents: e.payload } },
          })
        }),
      )
      registered.push(
        await listen<{ agentId: string; status: unknown; epoch: number }>('status-changed', (e) => {
          const { agentId, status, epoch } = e.payload
          this.messageCb?.({
            kind: 'control',
            event: {
              StatusChanged: {
                agent_id: agentId,
                status,
                epoch,
              },
            },
          })
        }),
      )
      registered.push(
        await listen<{ result: unknown }>('restore-result', (e) => {
          this.messageCb?.({
            kind: 'control',
            event: {
              RestoreResult: { report: e.payload.result },
            },
          })
        }),
      )
      registered.push(
        await listen<unknown>('profile-list-updated', (e) => {
          this.messageCb?.({
            kind: 'control',
            event: { ProfileListUpdated: { profiles: e.payload } },
          })
        }),
      )
      // ★연결 상태 동기화(단일 진실원)★: Rust 쪽 연결 task 가 상태 전이(connected/reconnecting/down)
      //   시 이 이벤트를 emit 한다. 프론트는 이 이벤트로만 상태를 바꾼다(doConnect 임의 전이 없음).
      //
      // ★출력 Channel 등록 = 이 전이 단일 경로(Fix-C ④)★: 비-connected → connected 재전이를 감지하면
      //   registerOutputChannel 을 호출한다 — **첫 연결·Rust 내부 재연결(reconnecting→connected) 모두**
      //   이 경로로 등록한다.
      //   왜 doConnect 가 아니라 여기인가: Rust 는 connect Ok 시 `connected` emit 을 invoke resolve *보다
      //   먼저* 한다(connection.rs: app.emit("connected") → ready_tx.send → invoke resolve). 그래서 이
      //   전이 핸들러가 doConnect 의 invoke resolve 보다 *항상 먼저* 돈다 → 여기서 등록하면 "등록 전 도착
      //   출력 유실 갭"이 더 일찍 닫히고(Fix-C ④ 순서 목적), 첫 연결/재연결을 한 경로로 통일해 doConnect
      //   와의 멱등 *중복* 등록도 없앤다. Rust 주도 재연결도 doConnect 를 안 거치므로(디커플) 이 단일
      //   경로가 그 갭까지 메운다. 멱등이라(같은 window_label 덮어쓰기, 옛 onmessage delete) 안전.
      registered.push(
        await listen<string>('daemon-connection-state', (e) => {
          this.applyConnectionState(e.payload)
        }),
      )
      // ★in-flight close 가드★: await 중 close() 가 끼었으면(세대 증가) 이 등록은 stale 이다 — 저장하지
      //   않고 즉시 해제한다(close 가 못 정리한 좀비 리스너 방지). 다음 연결이 새로 등록한다.
      if (myGen !== this.generation) {
        for (const off of registered) off()
        return
      }
      this.unlisten = registered
    } catch (e) {
      // 부분 등록분 정리 후 실패 전파(누수 방지).
      for (const off of registered) off()
      throw e
    }
  }

  private cleanupListeners(): void {
    for (const off of this.unlisten) off()
    this.unlisten = []
  }

  // ── 출력 Channel 등록(③-b, HIGH-1) ───────────────────────────────────────────
  // 이 창의 per-window 출력 Channel 을 만들어 subscribe_output invoke 로 Rust registry 에 등록한다.
  // Rust 연결 task 가 이 Channel 로 그 창의 모든 agent 출력을 raw bytes(Response::new)로 fan-out 하면,
  // decodeOutputFrame 으로 풀어 output InboundMessage 로 올린다(WsTransport binary arm 과 동형).
  //
  // ★멱등(Fix-C ①·④)★: connected 마다(doConnect / u5 재전이) 불릴 수 있으나, 새 Channel 을 만들어
  //   재등록하면 Rust 가 같은 window_label 로 덮어쓴다(agent.rs subscribe_output: 같은 라벨 재등록은
  //   옛 WindowEntry drop). ★이전 outputChannel onmessage 는 먼저 delete 로 정리(#13133 — null 대입
  //   아님)★ 해 동시 doConnect 두 개가 각각 Channel 을 만들어 좀비 콜백을 남기는 일을 막는다. 재연결
  //   후에도 출력이 끊기지 않게 한다.
  //
  // ★single-flight(FIX 6)★: 이 메서드는 등록을 직렬화하는 게이트다. 진행 중이면 그 promise 를 재사용하고
  //   재실행 요청만 기록한다 — 동시에 두 개의 subscribe_output invoke 가 Rust 에 떠 있는 상황(어느 게
  //   마지막으로 닿을지 통제 불가)을 원천 차단한다. 실제 등록 작업은 doRegisterOutputChannel.
  private registerOutputChannel(): Promise<void> {
    if (this.outputChannelInflight) {
      // 진행 중 — 새 invoke 를 겹쳐 띄우지 않는다. 대신 "완료 후 1회 재등록" 플래그만 세운다(여러 요청
      //   합침). 진행 중인 등록이 이미 최신 상태를 반영할 수도 있으나, 안전측으로 1회 재등록해 재연결
      //   경계에서 확실히 살아있는 Channel 로 수렴시킨다.
      this.outputChannelRerun = true
      return this.outputChannelInflight
    }
    const run = async (): Promise<void> => {
      try {
        do {
          this.outputChannelRerun = false
          await this.doRegisterOutputChannel()
          // 진행 중 들어온 추가 요청이 있으면(rerun) 한 번 더 — 그 사이에도 겹친 invoke 는 없다(직렬).
        } while (this.outputChannelRerun)
      } finally {
        this.outputChannelInflight = null
      }
    }
    const p = run()
    this.outputChannelInflight = p
    return p
  }

  private async doRegisterOutputChannel(): Promise<void> {
    // 옛 Channel 정리(#13133: null 대입 아님 — delete). 멱등성의 핵심 — 좀비 onmessage 제거.
    if (this.outputChannel) {
      delete (this.outputChannel as { onmessage?: unknown }).onmessage
      this.outputChannel = null
    }
    const channel = new Channel<ArrayBuffer>()
    channel.onmessage = (raw: ArrayBuffer) => {
      // raw = Rust Response::new(frame bytes) — [tag][agentId:16][epoch:4][seq:8][payload].
      const f = decodeOutputFrame(raw)
      if (!f) return
      this.messageCb?.({
        kind: 'output',
        agentId: f.agentId,
        epoch: f.epoch,
        seq: f.seq,
        bytes: f.payload,
      })
    }
    this.outputChannel = channel
    // window_label 은 Rust 가 호출 webview 에서 자동 주입(agent.rs subscribe_output: tauri::Window).
    await invoke('subscribe_output', { channel })
  }

  // ── 전송 준비 보장 = attach-only(ADR-0021) ─────────────────────────────────
  // WsTransport.ensureReady 와 대응: 이미 connected 면 resolve, 아니면 Rust connect 호출.
  // Tauri transport 는 Rust DaemonClient 가 이미 연결 단일화를 담당하므로, 여기선 연결 상태만
  // 확인하거나 invoke('daemon_ensure') 를 부르는 방식으로 동작한다.
  ensureReady(): Promise<void> {
    if (this._state === 'connected') return Promise.resolve()
    if (this.connectPromise) return this.connectPromise
    if (this.closedByUser) {
      return Promise.reject(
        new Error('daemon down — daemon_start 로 명시 시작 필요 (ADR-0021: 명령은 respawn 안 함)'),
      )
    }
    // attach-only: Rust ensure(no-spawn) 호출.
    this.connectPromise = this.doConnect(false)
    return this.connectPromise
  }

  // ── 명시 spawn 진입점(ADR-0021 §1) ──────────────────────────────────────────
  // Rust DaemonClient.connect()(spawn 가능)를 invoke 로 호출한다.
  //
  // ★재진입 가드(Fix-C ①)★: 진행 중인 doConnect 가 있으면(connectPromise) 그 promise 를 재사용한다 —
  //   확인 없이 connectPromise 를 덮어쓰면 중복 `daemon_connect` invoke 가 나가 Rust 가 세대를 두 번
  //   올린다(불필요한 승계). 단 start 의 의미(명시 spawn 의도 = closedByUser 리셋)는 진행 중이든 아니든
  //   유지한다 — 사용자가 닫은 뒤 다시 start 하면 재연결이 다시 가능해야 하므로.
  start(): Promise<void> {
    if (this._state === 'connected') return Promise.resolve()
    // closedByUser 리셋은 항상(재진입 여부 무관) — start = "다시 연결을 허용한다"는 명시 의도.
    this.closedByUser = false
    // 진행 중인 연결이 있으면 그것을 재사용(중복 daemon_connect 방지). 단 진행 중인 게 ensure(no-spawn)
    //   였더라도, start 의 spawn 의도는 closedByUser 리셋으로 이미 반영됐다 — 그 ensure 가 실패해
    //   재시도하면 다음 호출이 spawn 경로를 탄다.
    if (this.connectPromise) return this.connectPromise
    this.connectPromise = this.doConnect(true)
    return this.connectPromise
  }

  // allowSpawn=true → invoke('daemon_connect'), false → invoke('daemon_ensure').
  //
  // ★연결 시도만(MED-1 + Fix-C ①·④)★: doConnect 는 (1)control 리스너 멱등 등록 (2)Rust connect/ensure
  //   invoke 만 한다. 상태 전이와 출력 Channel 등록은 *둘 다* Rust `daemon-connection-state` emit(u5
  //   리스너)이 단일 권위로 담당한다 — doConnect 는 이 둘을 직접 하지 않는다.
  //   - registerListeners(멱등): close()→cleanupListeners 후 재연결하면 control 이벤트(목록/상태/프로필)
  //     리스너가 비어 전부 유실된다 → 매 연결마다 멱등 재등록으로 보장(이미 등록돼 있으면 no-op).
  //
  // ★상태 전이 안 함(Fix-C ①)★: 이전 구현은 invoke resolve 후 `setState('connected')` 를 했으나, 이는
  //   Rust 의 실제 상태(stale 폐기·재연결)와 어긋날 수 있어 제거했다. 상태는 u5 가 단일 진실원이다.
  //
  // ★출력 Channel 등록도 u5(Fix-C ④)★: doConnect 가 아니라 u5 의 connected 전이 핸들러가 등록한다
  //   (더 이른 등록 + 첫 연결/재연결 단일 경로 통일). 근거(connect Ok 시 emit↔resolve 순서)·상세 =
  //   registerListeners 의 u5 주석 정본 참조.
  //
  // ★close 세대 가드(Fix-C ①)★: invoke await 동안 close() 가 끼면 myGen != generation. 상태·출력 Channel
  //   부활은 이미 구조적으로 막혀 있다(아래 본문 주석) — 가드는 stale doConnect 가 connectPromise 를
  //   잘못 비우지 않게 하는 데 쓴다(finally).
  // 실패는 그대로 전파(ensureReady/start 호출자가 catch). connectPromise 정리는 finally(current 세대만).
  private async doConnect(allowSpawn: boolean): Promise<void> {
    const myGen = this.generation
    try {
      // 리스너를 invoke *전에* 멱등 등록한다(★순서 load-bearing★): Rust 는 connect 성공 시
      // `daemon-connection-state="connected"` 를 emit 한 *뒤* invoke 를 resolve 한다. invoke resolve
      // 후에 리스너를 달면 그 초기 connected 이벤트를 놓쳐 _state 가 'down' 에 고착될 수 있다(상태
      // 단일 진실원이 u5 라 더욱 치명적). 먼저 달아두면 connect 가 발행하는 첫 전이(connected)부터
      // 빠짐없이 받는다(이미 등록돼 있으면 no-op). ★출력 Channel 등록도 그 connected 전이 핸들러(u5)가
      // 한다★ — doConnect 는 등록을 직접 하지 않는다(중복 제거, registerListeners 주석 참조).
      await this.registerListeners()
      await invoke(allowSpawn ? 'daemon_connect' : 'daemon_ensure')
      // ★세대 가드★: invoke await 동안 close() 가 끼었으면(generation 증가) 이 연결은 stale 이다.
      //   상태·출력 Channel 부활은 구조적으로 이미 막혀 있다 — 상태는 u5(Rust emit) 단일 진실원이고
      //   doConnect 는 setState 를 안 하며, 출력 Channel 등록도 u5 가 하는데 close() 가 cleanupListeners
      //   로 u5 를 제거했으므로 close 후 도착하는 connected emit 은 무시된다. 여기선 추가 side-effect 가
      //   없어 별도 분기는 불필요하나, 가독성을 위해 stale 이면 조용히 빠진다.
      if (myGen !== this.generation || this.closedByUser) {
        return
      }
    } finally {
      // 이 doConnect 가 current 세대의 것일 때만 connectPromise 를 비운다 — close() 가 이미 다른
      //   connectPromise(=null)로 만들었을 수 있어, stale doConnect 가 새 promise 를 지우면 안 된다.
      if (myGen === this.generation) this.connectPromise = null
    }
  }

  // ── 명령 전송 + reply 하행(③-a, HIGH-2) ──────────────────────────────────────
  // ProtocolClient 가 AgentCommand wire 객체를 넘기면 invoke 로 Rust DaemonClient 에 전달한다.
  // Rust forward_daemon_command 는 request_id 있는 명령의 데몬 reply(AgentEvent)를 그대로 직렬화해
  // 반환한다 — 그 반환을 control InboundMessage 로 올려 ProtocolClient.handleEvent 가 pending 을 깬다
  // (WsTransport 가 데몬 Text frame 을 control 로 올리는 것과 동형). request_id 없는 명령(Resize/
  // Subscribe/Unsubscribe)은 null 을 반환하므로 올리지 않는다(fire-and-forget).
  //
  // ★Promise 반환★: ProtocolClient.sendCommand 는 send 가 Promise 면 그 .catch 만 본다(성공 반환은
  //   무시). 그래서 reply 는 여기서 직접 onMessage 로 올리고, invoke 실패(연결 끊김/Rust Err/reply
  //   타임아웃)는 reject 해 ProtocolClient 가 해당 pending 을 reject 하게 한다(영구 hang 차단). reply
  //   자체는 onMessage 경로로 resolve 되므로 이 Promise 의 resolve 값은 ProtocolClient 가 쓰지 않는다.
  send(payload: unknown): void | Promise<void> {
    return invoke<unknown>('forward_daemon_command', { cmd: payload }).then((reply) => {
      // reply 가 있으면(request_id 명령의 데몬 응답) control 로 올린다. null/undefined = 올릴 것 없음.
      if (reply != null && typeof reply === 'object') {
        this.messageCb?.({ kind: 'control', event: reply as Record<string, unknown> })
      }
    })
  }

  // ★세대 가드(Fix-C ①)★: generation++ 으로 in-flight doConnect 를 stale 화한다 — 뒤늦게 resolve 된
  //   doConnect 가 출력 Channel 을 등록하거나 connectPromise 를 건드리지 못하게 한다. connectPromise=null
  //   로 비워 다음 ensureReady/start 가 새 연결을 시작하게 한다(closedByUser 가 막지만 start 가 리셋).
  close(): void {
    this.closedByUser = true
    this.generation += 1
    this.connectPromise = null
    this.cleanupListeners()
    // 출력 Channel 정리(#13133: null 대입 아님 — delete onmessage). 재연결 시 doConnect 가 새로 등록.
    if (this.outputChannel) {
      delete (this.outputChannel as { onmessage?: unknown }).onmessage
      this.outputChannel = null
    }
    // ★single-flight 재등록 취소(FIX 6)★: 진행 중 등록의 "완료 후 재등록" 플래그를 끈다 — close 후
    //   좀비 Channel 을 다시 붙이지 않게. in-flight promise 자체는 완료되며 스스로 null 로 정리한다.
    this.outputChannelRerun = false
    invoke('daemon_close').catch((e: unknown) => {
      console.warn('[TauriTransport] daemon_close 실패:', e)
    })
    this.setState('down')
  }

  // ProtocolClient 는 transport 를 생성자에서 받는다. 리스너 등록은 async 이므로 별도 init 을
  // 호출하거나 clientFactory 에서 await 로 처리한다. doConnect 도 멱등 registerListeners 를 부르므로,
  // 부팅 시 init→connect 순서든 connect 단독이든 control 리스너는 정확히 1벌 등록된다.
  async init(): Promise<void> {
    await this.registerListeners()
    // ★리로드 자가복구(Fix-D)★: 리스너 등록이 끝난 뒤 현재 연결 상태를 1회 pull 조회해 리로드 웹뷰의
    //   전이 emit 사각지대를 메운다. 결함·pull 전략·레이스 가드 = selfHeal 주석 정본 참조.
    await this.selfHeal()
  }
}
