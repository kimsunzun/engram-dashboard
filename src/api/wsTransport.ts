// WsTransport — Transport 의 WS(데몬) carrier 구현 (ADR-0020 결정3, TRD Stage 3).
//
// DaemonClient(580줄)에서 WS-특정 부분을 추출: discover_daemon → WebSocket 생성 → Auth →
// Hello 대기 → 지수백오프 재연결 → ws.send / ws.onmessage. binary frame·JSON AgentEvent 를
// **InboundMessage 로 정규화**해 ProtocolClient 로 올린다(ProtocolClient 는 carrier 디코드를
// 모른다). Auth/Hello 는 여기서 소비(handshake) — control/output 만 위로 올린다.
//
// 프로토콜 의미론(request_id 매칭·dedup·epoch 가드·resubscribe)은 ProtocolClient 소유.
// 이 transport 는 carrier 만 책임진다: 연결·재연결·프레임 인코딩/디코딩.

import { invoke } from '@tauri-apps/api/core'

import type { ConnectionState } from './agentClient'
import type { InboundMessage, Transport } from './transport'
import { decodeOutputFrame } from './wsFrame'

// ── discover_daemon DTO(discovery.rs DaemonInfoDto 미러) ──────────────────────────
interface DaemonInfoDto {
  pid: number
  host: string
  port: number
  token: string
  protocol_version: number
}

type WireEvent = Record<string, unknown>

export class WsTransport implements Transport {
  private ws: WebSocket | null = null

  private _state: ConnectionState = 'down'
  private stateListeners = new Set<(s: ConnectionState) => void>()

  // 진행 중 연결 시도(중복 연결 방지). resolve 는 Hello 수신(=인증 성공) 시.
  private connectPromise: Promise<void> | null = null

  // 진행 중 openSocket promise 의 reject 핸들(cross-socket). settled 는 한 소켓 안의 double-settle 만
  // 막는다 — 소켓이 detach(cleanupSocket)되면 그 promise 의 resolve/reject 는 닫힌 closure 안에 갇혀
  // 영영 안 불린다. 이 필드로 밖에서 깨워 await 가 hang 하지 않게 한다. settle 시 항상 null 로 비운다.
  private pendingReject: ((e: Error) => void) | null = null

  private closedByUser = false
  private reconnectAttempt = 0
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null

  // ADR-0021: ensure(명시 spawn)와 reconnect(attach-only)를 분리하기 위한 캐시.
  // 최초 명시 연결에서 discover_daemon 으로 얻은 host/port/token 을 보관한다. 재연결 루프는
  // 이 캐시를 출발점으로 쓰되, 매 재시도 전에 read_daemon_info(no-spawn)로 현재 daemon.json 을
  // 재조회해 **옮겨간 데몬(hot-swap·크래시 재spawn)의 새 주소를 따라간다**(아래 openSocket 주석).
  // ★불변식 유지★: reconnect 는 절대 discover/spawn 을 호출하지 않는다 — read_daemon_info 는
  // read-only 라 데몬을 깨우지 않는다. 살아있는 데몬이 없으면 backoff/소진 → 'down'(사용자 명시 start 로 복구).
  private cachedInfo: DaemonInfoDto | null = null

  // ★세대 토큰(Blocker-1 좀비 소켓 가드)★: openSocket 진입마다 ++ 해 "현재 유효한 시도"를 식별한다.
  // openSocket 본체는 await(discover_daemon·read_daemon_info) 에서 yield 하므로, 그 사이 다른
  // start()/close()(=cleanupSocket)가 끼면 이 시도는 supersede 된다. pendingReject 는 outer promise 만
  // settle 할 뿐 await 를 넘어 재개된 run() 본체가 new WebSocket 을 만들고 this.ws 에 박는 것(좀비)은
  // 못 막는다 — 그래서 await 직후·소켓 생성 직전에 myGen!==openGen 으로 stale 재개를 폐기한다.
  // cleanupSocket()도 ++ 해서, 새 openSocket 없이 cleanup 만 하는 close() 경로가 in-flight 시도를 무효화한다.
  private openGen = 0

  // ProtocolClient 가 등록하는 단일 수신 콜백(control/output 정규화 메시지).
  private messageCb: ((msg: InboundMessage) => void) | null = null

  // ── 연결 상태 ──────────────────────────────────────────────────────────────────
  get connectionState(): ConnectionState {
    return this._state
  }

  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void {
    this.stateListeners.add(cb)
    // 등록 즉시 현재 상태 1회 통지(ProtocolClient 가 초기 상태를 알게).
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

  // ── 전송 준비 보장 = attach-only(ADR-0021 B-1) ────────────────────────────────────
  // 명령/구독 경로(ProtocolClient.sendCommand/subscribeOutput/resizePty)가 매 호출 전에 부른다.
  // ★불변식★: 이 경로는 절대 spawn 하지 않는다 — openSocket(false)(캐시 재오픈)만. 그래야 사용자가
  // 데몬을 끈 뒤 키 한 번/창 리사이즈만 해도 데몬이 되살아나는 버그(B-1)가 안 난다. spawn 은 명시
  // start() 에서만. closedByUser/attempt 리셋도 여기서 하지 않는다(그건 명시 start 의 책임).
  ensureReady(): Promise<void> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN && this._state === 'connected') {
      return Promise.resolve()
    }
    if (this.connectPromise) return this.connectPromise
    // 사용자가 명시 종료(close)했거나 재연결이 소진돼 down 인데도 명령이 들어오면 attach 시도조차
    // 하지 않고 즉시 reject — 명령이 데몬을 깨우면 안 된다(꺼진 채 유지). 복구는 명시 start 로만.
    if (this.closedByUser) {
      return Promise.reject(
        new Error('daemon down — daemon_start 로 명시 시작 필요 (ADR-0021: 명령은 respawn 안 함)'),
      )
    }
    if (!this.cachedInfo) {
      return Promise.reject(
        new Error('daemon down — daemon_start 로 명시 시작 필요 (no cached daemon, ADR-0021)'),
      )
    }
    // attach-only: 캐시 host:port 로 소켓만 재오픈. 데몬이 죽었으면 onclose → reject(respawn 안 함).
    this.connectPromise = this.openSocket(false)
    return this.connectPromise
  }

  // ── 명시 spawn 진입점(ADR-0021 §1) ───────────────────────────────────────────────
  // 부팅 연결 / 사용자 daemon_start(DaemonControl.start → client.connect → 여기) 만 호출한다.
  // discover_daemon 으로 데몬을 spawn(없으면) 하고 캐시를 채운다. 이전 close()/소진으로 멈춘
  // 재연결 상태를 리셋(closedByUser 해제 + attempt 0 + 진행 중 타이머 정리)해 다시 살아날 수 있게.
  start(): Promise<void> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN && this._state === 'connected') {
      return Promise.resolve()
    }
    // ★race 가드★: 버려진 in-flight 소켓(this.ws)의 onclose 가 나중에 발화하면 handleClose 가
    // 새 소켓의 this.ws 를 null 로 만들고 reconnect 를 거는 race 가 난다. 새 openSocket(true) 전에
    // cleanupSocket 으로 옛 소켓의 핸들러를 delete(#13133)·close 해 그 onclose 를 떼어낸다.
    this.cleanupSocket()
    this.closedByUser = false
    this.reconnectAttempt = 0
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
    // 명시 start 는 discover 를 강제한다 — 위 cleanupSocket 으로 진행 중 attach 소켓을 명시적으로
    // 정리한 뒤 새 openSocket(true) 로 연결을 다시 연다(옛 소켓 onclose 에 정리를 의존하지 않음).
    this.connectPromise = this.openSocket(true)
    return this.connectPromise
  }

  /**
   * 1회 소켓 열기 + Auth 전송 + Hello 대기. 성공 시 resolve, 실패 시 reject(상위가 처리).
   *
   * ADR-0021 ensure/reconnect 분리:
   *  - allowDiscover=true(명시 연결): discover_daemon 호출(없으면 spawn) → 결과를 cachedInfo 에 보관.
   *  - allowDiscover=false(재연결 루프): **discover/spawn 절대 금지.** 단, read_daemon_info(no-spawn)로
   *    현재 daemon.json 을 재조회해 **옮겨간 데몬을 따라간다**(hot-swap·크래시 재spawn — 새 port/token).
   *    이것은 attach 대상 추적이지 spawn 이 아니다(read-only, 데몬을 깨우지 않음). 살아있는 데몬이 없으면
   *    캐시 host/port 로 마지막 시도하고, 그것도 없으면 reject(소진→down). 재연결은 새 데몬을 만들지 않는다.
   */
  private openSocket(allowDiscover: boolean): Promise<void> {
    // ★Blocker-1 좀비 가드★: 이 시도의 세대 토큰을 모든 await 전에 동기로 캡처한다. 아래 await 들이
    // yield 하는 사이 다른 start()/close()가 openGen 을 올리면(supersede) 재개된 run() 본체가 stale 임을
    // 이 값으로 판별해 new WebSocket 생성 전에 폐기한다(좀비 소켓 차단).
    const myGen = ++this.openGen
    return new Promise<void>((resolve, reject) => {
      // 이 promise 의 reject 를 밖(cleanupSocket)에서도 닿게 보관. 모든 settle 경로에서 null 로 비운다.
      this.pendingReject = reject
      const settleResolve = (): void => {
        this.pendingReject = null
        resolve()
      }
      const settleReject = (e: Error): void => {
        this.pendingReject = null
        reject(e)
      }
      // 이 시도가 다른 start()/close()(cleanupSocket)에 의해 supersede 됐는가. true 면 호출자가 이미
      // settleReject 후 return 해야 한다(WebSocket 생성·this.ws 대입 전 = 좀비 안 생김).
      const isStale = (): boolean => myGen !== this.openGen
      const run = async () => {
        let info: DaemonInfoDto
        if (allowDiscover) {
          // 명시 연결만 discover(없으면 데몬 spawn). 성공 시 캐시 갱신(데몬 재기동 시 port/token 반영).
          info = await invoke<DaemonInfoDto>('discover_daemon')
          // ★await 재개 직후 stale 검사★: discover 가 yield 한 사이 cleanupSocket(openGen++)이 끼면
          // 이 시도는 무효 — settleReject 만 하고 빠진다(소켓 생성 금지, this.ws 미오염).
          if (isStale()) {
            settleReject(new Error('openSocket superseded — stale attempt discarded'))
            return
          }
          this.cachedInfo = info
        } else {
          // 재연결 = attach-only. ★옮겨간 데몬 추적(ADR-0021)★: spawn 대신 read_daemon_info(no-spawn)로
          // 현재 daemon.json 을 재조회한다. hot-swap(stop→start)·크래시 재spawn 으로 데몬이 새
          // port/token 으로 떠도 그 주소를 따라가 attach 한다(read-only — 데몬을 만들거나 깨우지 않음).
          const fresh = await invoke<DaemonInfoDto | null>('read_daemon_info').catch(() => null)
          // ★await 재개 직후 stale 검사(Blocker-1)★: read_daemon_info 가 yield 한 사이 close()/start()의
          // cleanupSocket(openGen++)이 끼면 무효 — 폐기. pendingReject 만으론 outer promise 만 settle 되고
          // 본체가 new WebSocket 까지 진행해 좀비를 만들거나 this.ws 를 hijack 하므로 여기서 끊는다.
          if (isStale()) {
            settleReject(new Error('openSocket superseded — stale attempt discarded'))
            return
          }
          if (fresh) {
            // 살아있는 데몬 발견(옮겨갔을 수 있음) → 캐시 갱신 후 그 주소로 attach.
            this.cachedInfo = fresh
            info = fresh
          } else if (this.cachedInfo) {
            // read_daemon_info 가 None(살아있는 데몬 없음)이지만 캐시가 있다 — 일시적 끊김일 수 있어
            // 옛 주소로 마지막 시도(여전히 attach-only, spawn 아님). 실패하면 onclose→backoff→소진.
            info = this.cachedInfo
          } else {
            // 캐시도 없고 살아있는 데몬도 없다 → 붙을 곳을 모른다. reject(재연결은 새 데몬 안 만듦).
            throw new Error('no live/cached daemon — reconnect cannot discover/spawn (ADR-0021)')
          }
        }
        // ★소켓 생성 직전 최종 stale 검사(Blocker-1)★: await 후 분기에서 동기 시간이 지났을 수 있고,
        // 좀비 차단의 핵심 지점이다 — new WebSocket 은 반드시 "현재 시도"일 때만. 여기서 통과해야
        // this.ws 대입이 정당하다.
        if (isStale()) {
          settleReject(new Error('openSocket superseded — stale attempt discarded'))
          return
        }
        // host 는 일단 127.0.0.1 고정 가정(로컬 IPC).
        const ws = new WebSocket('ws://' + info.host + ':' + info.port)
        ws.binaryType = 'arraybuffer'
        this.ws = ws

        let settled = false

        ws.onopen = () => {
          // 첫 frame = Auth(JSON text). protocol_version 은 discover 가 준 값을 echo.
          ws.send(
            JSON.stringify({
              Auth: { token: info.token, protocol_version: info.protocol_version },
            }),
          )
        }

        ws.onmessage = (event: MessageEvent) => {
          if (typeof event.data === 'string') {
            const msg = JSON.parse(event.data) as WireEvent
            // Hello 수신 = 인증 성공. connect 완료. (handshake 내부 소비 — 위로 안 올림.)
            if ('Hello' in msg && !settled) {
              settled = true
              this.reconnectAttempt = 0
              // connected 전이 → ProtocolClient 가 resubscribeAll(재연결 resume).
              this.setState('connected')
              settleResolve()
              return
            }
            // 인증 전 Error = Auth 실패 → reject(연결은 데몬이 닫는다).
            if ('Error' in msg && !settled) {
              settled = true
              const m = (msg.Error as { message?: string })?.message ?? 'auth failed'
              settleReject(new Error('daemon auth failed: ' + m))
              return
            }
            // control event — ProtocolClient 로 정규화 전달.
            this.messageCb?.({ kind: 'control', event: msg })
          } else if (event.data instanceof ArrayBuffer) {
            // binary output frame — 디코드해 정규화 output 으로 전달.
            const f = decodeOutputFrame(event.data)
            if (!f) return
            this.messageCb?.({
              kind: 'output',
              tag: f.tag, // frame 종류(0 터미널 / 1 구조화) — ProtocolClient 가 소비 경로 분기.
              agentId: f.agentId,
              epoch: f.epoch,
              seq: f.seq,
              bytes: f.payload,
            })
          }
        }

        ws.onerror = () => {
          if (!settled) {
            settled = true
            settleReject(new Error('daemon websocket error'))
          }
        }

        ws.onclose = () => {
          this.handleClose(settled)
          if (!settled) {
            settled = true
            settleReject(new Error('daemon websocket closed before handshake'))
          }
        }
      }
      run().catch((e) => settleReject(e))
    })
  }

  /** 소켓 종료 처리 — 의도적 종료가 아니면 재연결 스케줄. pending reject 는 ProtocolClient 가
   * connectionState 전이(connected→reconnecting)로 처리한다(carrier 무관 위치로 승격). */
  private handleClose(wasHandshakeSettled: boolean): void {
    this.connectPromise = null
    this.ws = null

    if (this.closedByUser) {
      this.setState('down')
      return
    }
    // 핸드셰이크 중 끊김도 재연결 대상(데몬 재기동 대기). 지수 백오프.
    void wasHandshakeSettled
    this.setState('reconnecting')
    this.scheduleReconnect()
  }

  // attach-only 재연결 최대 시도 횟수. ADR-0021: 데몬이 죽으면(graceful stop·kill·크래시) 캐시된
  // host:port 로의 재연결이 전부 실패한다 — 무한 reconnecting 으로 매달리지 않고 이 횟수만큼 시도한
  // 뒤 'down' 으로 정착시킨다(꺼진 채 유지). 일시적 네트워크 끊김은 이 안에서 회복된다. 복구는
  // 사용자의 명시 daemon_start(=start(), discover 허용)로만 — 재연결·명령(ensureReady)은 spawn 금지.
  private static readonly MAX_RECONNECT_ATTEMPTS = 5

  private scheduleReconnect(): void {
    if (this.reconnectTimer) return
    if (this.closedByUser) return
    // attach-only 재시도 소진 → 'down' 정착(데몬이 안 살아남는다). 사용자가 명시로 다시 시작.
    if (this.reconnectAttempt >= WsTransport.MAX_RECONNECT_ATTEMPTS) {
      this.setState('down')
      return
    }
    // 500ms → 1s → 2s → … 최대 10s.
    const delay = Math.min(500 * 2 ** this.reconnectAttempt, 10000)
    this.reconnectAttempt += 1
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null
      if (this.closedByUser) return
      // ★attach-only★: discover/spawn 금지 — 캐시된 host:port 로 소켓만 재오픈(openSocket(false)).
      // 성공하면 connectPromise 가 Hello 로 resolve 되고 reconnectAttempt 가 0 으로 리셋된다.
      // 실패(데몬 죽음 → 연결 거부)하면 onclose 가 와서 다시 scheduleReconnect → 소진 시 'down'.
      this.connectPromise = this.openSocket(false)
      this.connectPromise.catch(() => {
        // openSocket reject(캐시 없음/소켓 onclose 미발화) — 직접 재스케줄(소진 시 'down').
        if (!this.reconnectTimer && !this.closedByUser) {
          this.scheduleReconnect()
        }
      })
    }, delay)
  }

  // ── 명령 전송 ───────────────────────────────────────────────────────────────────
  send(payload: unknown): void {
    const ws = this.ws
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      throw new Error('daemon not connected')
    }
    ws.send(JSON.stringify(payload))
  }

  // ── slot (re)mount fresh replay 재요청(remount 대화 소실 FIX) ──────────────────
  // no-op: resync_output 은 src-tauri 로컬 축 B replay 경로(invoke → replay_slots)라 WsTransport
  // (데몬 직결, src-tauri 없음)엔 해당 경로가 없다. 운영 carrier 는 TauriTransport 고정이고 WsTransport
  // 는 테스트 mock 흔적이라(ADR-0020/0029) no-op 이 안전하다 — remount replay 재요청은 Tauri 에서만 배선.
  resyncOutput(_agentId: string): void {}

  // ── 명시 종료(재연결 중단) ──────────────────────────────────────────────────────
  close(): void {
    this.closedByUser = true
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
    this.cleanupSocket()
    this.setState('down')
  }

  /** #13133: 핸들러는 null 대입이 아니라 delete 로 정리한 뒤 close. */
  private cleanupSocket(): void {
    // in-flight start()를 새 start()/close()가 대체할 때, 옛 소켓의 resolve/reject 는 곧 delete 될
    // onclose/onmessage/onerror closure 안에만 있어 영영 settle 안 된다 → 그 promise 를 await 하던
    // 호출자(DaemonControl.start → client.connect)가 hang 한다. 명시 reject 로 깨운다(handshake 정상
    // 완료 시엔 pendingReject 가 이미 null 이라 healthy close 는 안 깬다).
    //
    // ★ws 생성 전(discover in-flight) 윈도도 포함★: pendingReject 는 openSocket 실행자에서 동기 설정되지만
    // this.ws 는 await invoke('discover_daemon') 이후에야 대입된다. 이 settle 과 connectPromise 정리를
    // 아래 `if (!ws) return` 위로 끌어올려 ws 가 아직 null 인 discover 윈도에서도 항상 실행되게 한다 —
    // 안 그러면 두 번째 start()가 pendingReject 를 덮어써 첫 promise 가 영영 settle 안 되고 호출자가 hang.
    // ★Blocker-1 좀비 가드★: in-flight openSocket 의 세대를 무효화한다. close() 처럼 새 openSocket 없이
    // cleanup 만 하는 경로(또는 start()의 cleanup 단계)가 await yield 중인 시도를 supersede 해, 재개 시
    // myGen!==openGen 으로 폐기되게 한다(pendingReject 는 outer promise 만 깨우지 본체 좀비 생성은 못 막음).
    this.openGen++
    if (this.pendingReject) {
      const rej = this.pendingReject
      this.pendingReject = null
      rej(new Error('connection superseded (start/close) — ADR-0021'))
    }
    this.connectPromise = null
    const ws = this.ws
    if (!ws) return
    delete (ws as { onmessage?: unknown }).onmessage
    delete (ws as { onopen?: unknown }).onopen
    delete (ws as { onerror?: unknown }).onerror
    delete (ws as { onclose?: unknown }).onclose
    try {
      ws.close()
    } catch {
      // 이미 닫힘 — 무시.
    }
    this.ws = null
  }
}
