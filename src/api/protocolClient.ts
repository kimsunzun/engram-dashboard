// ProtocolClient — AgentClient 의 carrier-무관 구현 (ADR-0020 결정3, TRD Stage 3 · ADR-0046 뷰 직결 replay).
//
// ★ADR-0046 재설계(S16) — 뷰 직결 replay★: src-tauri 미러 버퍼를 제거하고, remount/리로드/재연결은
//   데몬 ring 전량 재replay 로 대체했다. 진도 상태의 유일한 거처 = **웹뷰 뷰(slot) 단위**. 그래서 subs 를
//   agentId 가 아니라 **viewId(slot id)** 로 re-key 한다(버그 B 구조 해소 — 같은 agent 를 N 뷰가 봐도
//   각자 독립 진도). replay 경계는 transport 가 올리는 replayBoundary 제어 이벤트(tag=255 마커의 정규화)
//   로 판정하고, 뷰는 자기 requestReplay 가 반환한 myGen 이상의 성공 마커에만 sort+dedup flush 한다.
//
// ★부착 계기 = 명부이지 소켓이 아니다(ADR-0046 amend, 사용자 결정 2026-08-20)★: 연결이 끊기면 뷰는
//   `detached` 로 내려앉기만 한다(화면·커서 유지, 요청 없음). 다시 요청을 내는 계기는 **권위 명부에 그
//   에이전트가 있다**는 관측 하나뿐이다(observeRoster). 재기동한 데몬의 명부는 비어 있으므로, 소켓 재전이를
//   계기로 삼으면 살아 있지도 않은 에이전트에 전량 Subscribe 를 쏘아 전부 거절당한다(구조적 거절 폭풍).
//
// ★"같은 세션인가" 는 추론하지 않고 **명부가 말한 화신 표식으로 대조한다**(사용자 결정 2026-08-20)★:
//   명부 항목의 표식이 뷰가 들고 있던 것과 다르면 다른 화신이고, 같으면 같은 화신이다. 그 표식은 화신마다
//   뽑은 난수라 **대소에 뜻이 없다** — 이 파일 어디에서도 두 표식을 크기로 견주지 않는다(견주면 "더 새
//   것" 이라는 없는 사실을 지어내게 된다). 프레임 쪽 판정도 같은 규칙이라 **불일치 = 내 것 아님(drop)**
//   이고, 화신이 갈렸다는 판정은 권위인 명부만 낸다. 떨어뜨린 프레임은 잃는 게 아니다 — 부착이 전량
//   replay 를 다시 청구한다.

import type {
  AgentClient,
  ConnectionState,
  OutputChunk,
  OutputSubscription,
  ViewOutputState,
  ViewPhase,
  ViewResetFn,
} from './agentClient'
import type { InboundMessage, Transport } from './transport'
import type {
  AgentInfo,
  AgentProfile,
  AgentStatus,
  ClaudeOutputFormat,
  Preset,
  RestoreReport,
} from './types'

// ── replay 상태기계 상수(ADR-0046 §2·§4) ────────────────────────────────────────────
/** 재요청 사다리 최대 시도(bounded — 재검증 NEW-4). 소진 시 뷰를 error 상태로 전이(무한 폭주 금지). */
const LADDER_MAX_ATTEMPTS = 3
const LADDER_BACKOFF_MS = [1000, 2000, 4000]
/** watchdog 만료(ms) — buffering 에서 이 시간 내 성공 마커가 안 오면 재요청(flush 아님, §2). */
const WATCHDOG_MS = 10_000
/**
 * 뷰 buffering 버퍼 상한 — ring 상한의 2배(§4). 버퍼가 "이전 replay 꼬리 + 자기 replay 전체"를 담을 수
 * 있어야 하므로(Codex 재리뷰 #5). 초과 시 부분 유지(drop-oldest) 금지 → buffer 폐기 + 재요청.
 */
const VIEW_BUFFER_MAX_BYTES = 4 * 1024 * 1024
const VIEW_BUFFER_MAX_FRAMES = 8192

interface BufferedFrame {
  tag: number
  seq: number
  bytes: Uint8Array
  /**
   * 이 프레임을 낸 화신의 표식. buffering 중에는 어느 화신의 replay 를 기다리는지 **아직 모를 수 있어**
   * (표식 미상 부착) 두 화신의 프레임이 한 버퍼에 섞일 수 있다 — flush 때 성공 마커가 말한 표식만 남기고
   * 나머지를 버리는 데 쓴다.
   */
  epoch: number
}

/**
 * ★replayBoundary 마커의 최소 표현★: buffering 중 myGen 미확정 시 최고 gen 1개만 보관(§2 NEW-3).
 * epoch 는 gen 펜스 재평가에 함께 필요(구 epoch 마커 무효).
 */
interface HeldMarker {
  epoch: number
  gen: bigint
  truncated: boolean
  failed: boolean
}

// ── 내부 구독 상태(뷰 단위, ADR-0046 F1) ──────────────────────────────────────────────
interface SubState {
  agentId: string
  onChunk: (chunk: OutputChunk) => void
  onState?: (state: ViewPhase) => void
  onReset?: ViewResetFn
  phase: ViewPhase
  buffer: BufferedFrame[]
  bufferBytes: number
  /**
   * **미확정(undefined)**: invoke 응답 전(Channel 이 먼저 올 수 있음 — NEW-3). gen 펜스: 자기 myGen
   * 이상의 성공 마커에만 flush(남의/이전 replay 조기 flush 차단).
   */
  myGen: bigint | undefined
  /**
   * myGen 미확정 중 도착한 마커를 버리지 않고 최고 gen 1개 보관(§2 NEW-3). myGen 확정 시 재평가한다.
   */
  heldMarker: HeldMarker | undefined
  lastDeliveredSeq: number
  /**
   * 이 뷰가 읽고 있는 화신의 표식(불투명 — 대소 비교 금지). undefined = 아직 어느 화신인지 모른다.
   * ★채택하는 자리는 성공 마커 하나뿐이다(flushToLive)★ — 프레임에서 주워 담지 않는다(그 이유는
   * handleOutput 의 해당 주석).
   */
  epoch: number | undefined
  /**
   * 다음 flush 에서 **진도 커서를 버리고 소비자에게 화면을 비우라고 알려야 하나**. 새 화신 부착이 이 값을
   * 세우고, 실제 flush 가 그때 한 번에 거둔다.
   *
   * ★요청 시점이 아니라 결과 시점에 비우는 이유(load-bearing)★: 요청 시점에 비우면 거절·타임아웃으로
   * replay 가 안 오는 경우 슬롯이 **영구히 빈 채로** 남는다 — 데몬을 막 재기동한 직후가 정확히 그 상황이다.
   * 결과 시점에 비우면 실패한 replay 는 이전 화면을 그대로 두고, 성공한 replay 는 비우기와 다시 그리기가
   * 같은 틱에 붙어 중간에 빈 화면이 비치지 않는다.
   * ★한번 서면 flush 전엔 안 내려간다★: 그 사이 같은 화신으로 재부착(newSession=false)이 끼어들어도
   * 유지한다 — 커서를 버리기로 한 결정을 취소하면 0 부터 다시 오는 프레임이 옛 high-water 밑에 깔린다.
   */
  restartPending: boolean
  token: number
  attempts: number
  backoffTimer: ReturnType<typeof setTimeout> | null
  watchdogTimer: ReturnType<typeof setTimeout> | null
}

interface Pending {
  resolve: (v: unknown) => void
  reject: (e: unknown) => void
}

type WireEvent = Record<string, unknown>

// ★ViewOutputState(§5 LLM 제어 표면 — ADR-0046)★는 AgentClient 인터페이스가 정본(agentClient.ts) —
//   getViewOutputState 가 인터페이스 메서드라 타입도 거기 둔다(LLM 이 타입으로 발견).

export class ProtocolClient implements AgentClient {
  private readonly transport: Transport

  private pending = new Map<string, Pending>()
  // ★viewId(slot id) 키(ADR-0046)★: agentId 가 아니라 뷰 단위 — 같은 agent 를 N 뷰가 봐도 각자 독립
  //   진도(버그 B 구조 해소).
  private subs = new Map<string, SubState>()
  private subSeq = 0

  /**
   * 마지막으로 관측한 권위 명부(agentId → 화신 표식). ★`null` = **아직 모른다**이지 "비었다" 가 아니다★ —
   * 그 둘을 합치면 부팅·리로드·명부 조회 실패 구간이 전부 "그 에이전트는 없다" 로 읽혀 출력이 막힌다.
   * 끊기면 다시 `null` 로 돌린다(죽은 데몬의 명부를 산 데몬의 사실로 쓰지 않는다).
   */
  private roster: Map<string, number> | null = null

  private agentListCbs = new Set<(agents: AgentInfo[]) => void>()
  private statusCbs = new Set<(id: string, status: AgentStatus, epoch: number) => void>()
  private restoreCbs = new Set<(report: RestoreReport) => void>()
  private profileListCbs = new Set<(profiles: AgentProfile[]) => void>()
  private presetListCbs = new Set<(presets: Preset[]) => void>()

  private offMessage: (() => void) | null = null
  private offState: (() => void) | null = null

  private lastState: ConnectionState

  // ADR-0134: 연결 상태와 같은 표면에 실리는 실패 이유. 구독자 집합을 여기서 갖는 이유는, 상태
  //   문자열이 안 바뀌어도(이유만 밝혀져도) 통지해야 하기 때문이다 — transport 는 상태 전이만 안다.
  private connError: string | null = null
  private stateCbs = new Set<(state: ConnectionState) => void>()

  constructor(transport: Transport) {
    this.transport = transport
    this.lastState = transport.connectionState
    this.offMessage = transport.onMessage((msg) => this.route(msg))
    this.offState = transport.onConnectionStateChange((s) => {
      const prev = this.lastState
      this.lastState = s
      if (s === 'connected' && prev !== 'connected') {
        // 연결이 살아났으면 옛 실패 이유는 더 이상 참이 아니다.
        this.connError = null
        // ★여기서 뷰를 되살리지 않는다★: 소켓이 섰다는 것과 그 에이전트가 있다는 것은 다른 사실이다.
        //   재요청은 명부 관측(observeRoster)이 낸다 — 그 명부는 브로드캐스트로 밀려오거나
        //   eventBus.resyncAfterReconnect 의 getAgents 로 끌려온다.
      } else if (s !== 'connected' && prev === 'connected') {
        const lost = new Error('connection lost')
        for (const p of this.pending.values()) p.reject(lost)
        this.pending.clear()
        this.detachAllViews()
      }
      this.notifyState()
    })
  }

  // ── 연결 상태(transport 위임) ───────────────────────────────────────────────────
  get connectionState(): ConnectionState {
    return this.transport.connectionState
  }

  get connectionError(): string | null {
    return this.connError
  }

  reportConnectionError(reason: string | null): void {
    // ★연결돼 있으면 이유를 기록하지 않는다(load-bearing)★: 지우는 계기가 'connected' 로의 *전이*
    //   하나뿐이라, 이미 connected 인 상태에서 기록을 허용하면 지울 전이가 영영 오지 않아 배너가
    //   고착된다. 이 표면은 window.__ENGRAM_AGENT__ 로 공개돼 있어 LLM·cdp 호출로도 닿는다.
    const next = this.transport.connectionState === 'connected' ? null : reason
    if (this.connError === next) return
    this.connError = next
    this.notifyState()
  }

  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void {
    this.stateCbs.add(cb)
    // 등록 즉시 현재 상태 1회 통지(transport 동형 — 구독자가 초기 상태를 놓치지 않는다).
    cb(this.transport.connectionState)
    return () => {
      this.stateCbs.delete(cb)
    }
  }

  private notifyState(): void {
    const s = this.transport.connectionState
    for (const cb of this.stateCbs) cb(s)
  }

  // ── 명시 연결/해제(ADR-0021 §1·note3, transport 위임) ─────────────────────────────
  connect(): Promise<void> {
    return this.transport.start()
  }
  disconnect(): void {
    this.transport.close()
  }

  // ── 수신 라우팅(정규화 메시지) ───────────────────────────────────────────────────
  private route(msg: InboundMessage): void {
    if (msg.kind === 'output') {
      this.handleOutput(msg)
      return
    }
    if (msg.kind === 'replayBoundary') {
      this.handleReplayBoundary(msg)
      return
    }
    this.handleEvent(msg.event)
  }

  private *viewsForAgent(agentId: string): Generator<SubState> {
    for (const st of this.subs.values()) {
      if (st.agentId === agentId) yield st
    }
  }

  /**
   * 정규화 output frame — agent 를 보는 모든 뷰로 fan-out(ADR-0046 §2 상태전이표).
   *
   * ★tag 무관 공통 규율★: epoch 가드·seq dedup 은 tag(0 터미널/1 구조화)를 안 본다 — tag0/tag1 은 core
   *   OutputCore 의 같은 seq 공간을 공유한다(한 pump 발급). tag 는 배달 시 onChunk 에 실어 소비자가 렌더
   *   경로만 가른다.
   */
  private handleOutput(f: {
    tag: number
    agentId: string
    epoch: number
    seq: number
    bytes: Uint8Array
  }): void {
    for (const st of this.viewsForAgent(f.agentId)) {
      // detached = 이 뷰는 아무것도 요청하지 않았다. 같은 agent 를 보는 다른 뷰의 replay 가 fan-out 으로
      //   여기까지 오지만, 부착 판정 전에 받아 두면 그 프레임이 어느 화신 것인지 모른 채 커서를 흔든다.
      if (st.phase === 'error' || st.phase === 'detached') continue
      // ★표식 불일치 = 내 것 아님(live·buffering 공통)★: 표식은 난수라 대소가 "더 새 것" 을 뜻하지
      //   않으므로, 여기서 화신 회전을 판정하지 않고 그냥 떨어뜨린다. 회전을 선언하는 곳은 권위 명부
      //   하나뿐이고(observeRoster), 그 부착이 전량 replay 를 다시 청구하므로 이 프레임은 잃지 않는다.
      if (st.epoch !== undefined && f.epoch !== st.epoch) continue
      if (st.phase === 'live') {
        if (f.seq <= st.lastDeliveredSeq) continue
        st.lastDeliveredSeq = f.seq
        st.onChunk({ tag: f.tag, seq: f.seq, bytes: f.bytes })
        continue
      }
      // ★프레임으로는 표식을 채택하지 않는다(load-bearing)★: 표식 미상으로 부착한 뷰가 **먼저 온 프레임**
      //   으로 자기 화신을 정하면, 앞 화신의 늦은 프레임 하나가 그 자리를 차지해 진짜 새 화신의 프레임이
      //   전부 위 불일치 가드에 걸려 떨어지고 뷰는 사다리를 소진해 error 로 앉는다. 표식은 **권위**
      //   (성공 마커)가 정하고(flushToLive), 그때 이 버퍼에서 다른 화신 프레임을 걸러낸다.
      this.pushBuffer(st, f)
    }
  }

  private pushBuffer(
    st: SubState,
    f: { tag: number; epoch: number; seq: number; bytes: Uint8Array },
  ): void {
    st.buffer.push({ tag: f.tag, seq: f.seq, bytes: f.bytes, epoch: f.epoch })
    st.bufferBytes += f.bytes.length
    if (st.bufferBytes > VIEW_BUFFER_MAX_BYTES || st.buffer.length > VIEW_BUFFER_MAX_FRAMES) {
      // ★부분 flush 금지(§4·상태전이표 "buffer 폐기 + requestReplay 재발행")★: drop-oldest 가 아니라
      //   buffer 통째 폐기 *후* 재요청 — 병리 케이스 방어용(정상 도달 불가). ★폐기가 재요청보다 먼저★:
      //   버리지 않으면 pre-overflow gen 성공 마커가 남아 stale·불완전 프레임을 flush 하고, 재요청한
      //   replay 의 완전한 내용은 dedup high-water 뒤에 갇혀 유실된다(FIX-2). 폐기하면 재요청 replay 가
      //   전량(full-from-oldest)으로 완전히 다시 채운다.
      console.warn(`[ProtocolClient] 뷰 buffer 상한 초과(agent=${st.agentId}) — 폐기 후 재요청`)
      st.buffer = []
      st.bufferBytes = 0
      // ★FIX-A: overflow 는 gen 펜스도 무효화(실패/watchdog 경로와 다르다)★. 실패 마커·watchdog 경로는
      //   buffer 를 유지하므로 구 gen 성공 마커가 뒤늦게 와 그 온전한 buffer 를 flush 하는 게 정당하다(좀비
      //   복구). 반면 overflow 는 buffer 를 통째 폐기했다 — 여기서 myGen/heldMarker 를 남겨두면, 재요청
      //   백오프가 발화하기 전에 구 gen 의 성공 마커가 도착할 경우 evalMarker 가 이를 수용하고 flushToLive 가
      //   *빈* buffer 로 live 전이(내용 유실)하며 clearTimers 가 예약된 재요청까지 취소한다. 폐기 = 구 gen
      //   flush = 데이터 손실이므로, 재요청한 replay 의 새 gen 이 확정될 때까지 어떤 마커도 flush 못 하도록
      //   펜스를 무효화한다.
      st.myGen = undefined
      st.heldMarker = undefined
      this.ladderRerequest(st)
    }
  }

  /**
   * ★replay 경계 마커(ADR-0046 §2)★ — transport 가 tag=255 마커를 정규화해 올린 제어 이벤트.
   */
  private handleReplayBoundary(m: {
    agentId: string
    epoch: number
    gen: bigint
    truncated: boolean
    failed: boolean
  }): void {
    for (const st of this.viewsForAgent(m.agentId)) {
      this.evalMarker(st, m)
    }
  }

  /**
   * 한 뷰에 대한 마커 판정(§2 상태전이표 — 마커 행 전부). ★평가는 마커 도착 시점★ — token/gen/epoch 를
   * 이 순간의 SubState 로 본다(리뷰 finding: 등록 시점 아님).
   */
  private evalMarker(
    st: SubState,
    m: { epoch: number; gen: bigint; truncated: boolean; failed: boolean },
  ): void {
    // live·error 뷰: 마커(어떤 gen이든) 무시 — fan-out 으로 도달하는 남의 replay 경계. live 는 dedup 만으로 충분(§2).
    if (st.phase !== 'buffering') return
    // ★myGen 미확정(NEW-3)★: 마커를 버리지 않고 최고 gen 1개 보관 → myGen 확정 시 재평가(resolveHeldMarker).
    //   교체 규칙(FIX-3): (a) 더 높은 gen 이면 교체 · (b) 같은 gen 인데 보관분은 failed 이고 신규는 성공이면
    //   교체. (b)가 없으면 좀비 late-Complete 복구가 깨진다 — 같은 gen 의 실패 마커(deadline)가 먼저 오고
    //   그 gen 의 성공 마커(늦은 Complete)가 뒤따를 때, 성공이 실패에 눌려 버려져 flush 못 하고 사다리로
    //   빠진다. 성공이 우선(같은 gen 이면 성공 마커가 이 replay 의 최종 결말).
    if (st.myGen === undefined) {
      const held = st.heldMarker
      const replace =
        held === undefined || m.gen > held.gen || (m.gen === held.gen && held.failed && !m.failed)
      if (replace) {
        st.heldMarker = { epoch: m.epoch, gen: m.gen, truncated: m.truncated, failed: m.failed }
      }
      return
    }
    // ★gen 펜스(ADR-0046)★: 자기 myGen 미만 마커 = 남의/이전 replay → 무시. epoch 불일치도 무시(구세대/구 epoch).
    //   왜 gen≥myGen 에만 flush 하나: 같은 agent 의 후속 replay 는 항상 이전의 누적 상위집합(full-from-oldest)
    //   이라, 늦게 mount 한 뷰의 버퍼(이전 replay 꼬리 + 자기 replay 전체)를 자기 gen 마커에 sort+dedup 하면
    //   완전하다. 남의(이전) gen 마커에 조기 flush 하면 자기 replay 머리가 dedup 유실된다(버그 B 재유입 경로).
    if (m.gen < st.myGen) return
    if (m.failed) {
      // ★실패 마커(§2)★: flush 금지 — buffer 는 유지한 채(sort+dedup 가 중복 흡수, 폐기 불필요) 재요청
      //   사다리. 왜 buffer 유지: 이 replay 는 미완결이나 다음 replay 가 full-from-oldest 라 겹치는 앞부분을
      //   dedup 가 흡수한다 — 버리면 오히려 다시 받아야 할 프레임을 손해.
      // ★epoch 펜스보다 앞이다(load-bearing)★: 실패 마커의 epoch 는 **권위값이 아니다** — 실패엔
      //   SubscribeAck 이 없어 보내는 쪽이 그 세대가 어느 세션의 것인지 확정할 수 없고, 그래서 마지막으로
      //   알려진 값(없으면 0)을 최선치로 싣는다(src-tauri plan_subscribe_refusal · deadline sweep).
      //   그 값에 펜스를 걸면 마커가 조용히 버려지고 뷰는 10초 watchdog 을 기다린다 — 즉시 사다리를 밀려고
      //   마커를 낸 이유 자체가 사라진다. 오배달 방어는 위 gen 펜스가 이미 진다(자기 세대 이상만 본다).
      this.ladderRerequest(st)
      return
    }
    // 성공 마커는 epoch 를 채택하므로(flushToLive) 여기서 걸러야 한다 — 구세대/구 epoch replay 의 경계를
    //   자기 것으로 오인하면 불완전한 버퍼가 flush 된다.
    if (st.epoch !== undefined && m.epoch !== st.epoch) return
    this.flushToLive(st, m.epoch, m.truncated)
  }

  private flushToLive(st: SubState, epoch: number, truncated: boolean): void {
    // ★epoch 채택★: 성공 마커의 epoch 로 확정(src-tauri decide_epoch 1차 필터를 통과한 값 — ADR-0046 은
    //   ADR-0007 "epoch 권위=SubscribeAck 단독"을 amends: src-tauri 필터 + 프론트는 필터된 frame/마커 채택).
    st.epoch = epoch
    // ★새 화신의 replay 가 **실제로 도착한** 지금 비운다(요청 시점이 아니라 — `restartPending` 주석)★:
    //   커서 폐기와 소비자 신호를 아래 배달 **직전**에 둬, 비우기와 다시 그리기가 같은 틱에 붙는다.
    if (st.restartPending) {
      st.restartPending = false
      st.lastDeliveredSeq = -1
      st.onReset?.()
    }
    // ★다른 화신의 프레임은 여기서 버린다★: 표식 미상으로 부착한 뷰의 버퍼에는 앞 화신의 늦은 프레임이
    //   섞여 있을 수 있고, 그 둘은 seq 공간이 겹쳐(둘 다 0 부터) dedup 로는 갈라지지 않는다.
    // seq 오름차순 정렬 후 flush(out-of-order 도착 방어): 배열 순서대로면 큰 seq 를 먼저 배달해 high-water 를
    //   올린 뒤 작은 seq 가 dedup 탈락한다.
    const ordered = st.buffer.filter((f) => f.epoch === epoch).sort((a, b) => a.seq - b.seq)
    st.buffer = []
    st.bufferBytes = 0
    for (const frame of ordered) {
      if (frame.seq <= st.lastDeliveredSeq) continue
      st.lastDeliveredSeq = frame.seq
      st.onChunk({ tag: frame.tag, seq: frame.seq, bytes: frame.bytes })
    }
    if (truncated) console.warn('[ProtocolClient] output truncated for', st.agentId)
    st.phase = 'live'
    st.attempts = 0
    this.clearTimers(st)
    st.onState?.('live')
  }

  /**
   * 재요청 사다리(bounded — §2·§4). 소진 시 phase='error' 전이(무한 폭주 금지) + onState 표면화.
   * remount·connected 전이가 사다리 리셋.
   */
  private ladderRerequest(st: SubState): void {
    // 진행 중 백오프가 있으면 중복 예약 안 함(한 사다리 단계는 한 타이머).
    if (st.backoffTimer) return
    if (st.attempts >= LADDER_MAX_ATTEMPTS) {
      st.phase = 'error'
      st.buffer = []
      st.bufferBytes = 0
      this.clearTimers(st)
      st.onState?.('error')
      return
    }
    st.attempts += 1
    const delay = LADDER_BACKOFF_MS[Math.min(st.attempts - 1, LADDER_BACKOFF_MS.length - 1)]
    // watchdog 은 백오프 대기 동안 무의미(재요청 예정) → 정리하고 재요청 후 재무장.
    this.clearWatchdog(st)
    st.backoffTimer = setTimeout(() => {
      st.backoffTimer = null
      // 재요청 시점에 이 뷰가 여전히 buffering 인지 확인(그 사이 성공 마커로 live 됐을 수 있음).
      if (st.phase !== 'buffering') return
      this.issueReplay(st)
    }, delay)
  }

  /**
   * 뷰를 buffering 으로 돌리고 full-from-oldest replay 를 재발행한다.
   *
   * `newSession` = 이 replay 가 **번호를 0 부터 다시 매기는 화신**의 것인가. 참이면 진도 커서를 버려야
   * 한다 — 안 버리면 새 화신의 프레임이 통째로 옛 high-water 밑에 깔려 dedup 탈락하고 그 뷰는 앱을 다시
   * 띄울 때까지 한 바이트도 못 그린다(실측 2026-08-20). ★단 그 폐기는 여기서 하지 않고 예약만 한다★ —
   * 실제 폐기·화면 비우기는 replay 가 도착한 flush 시점이다(`restartPending` 주석이 그 근거의 정본).
   * 거짓이면(같은 화신 이어보기) 커서를 그대로 둬 겹치는 앞부분이 dedup 에 흡수되고 끊긴 동안 놓친
   * 뒷부분만 배달된다.
   *
   * `epoch` = 이 뷰가 앞으로 받아들일 표식. 새 화신이면 `undefined` 를 넘겨 새 replay 의 첫 프레임에서
   * 다시 채택하게 한다 — 명부가 한 박자 늦어 다른 표식을 말했더라도 그 자리에서 자가 교정된다.
   */
  private startBuffering(
    st: SubState,
    epoch: number | undefined,
    resetLadder: boolean,
    newSession: boolean,
  ): void {
    st.phase = 'buffering'
    st.buffer = []
    st.bufferBytes = 0
    if (newSession) st.restartPending = true
    st.epoch = epoch
    st.myGen = undefined
    st.heldMarker = undefined
    if (resetLadder) st.attempts = 0
    this.clearTimers(st)
    // ★buffering 재시작을 통지한다(ADR-0145)★: 통지가 없으면 소비자의 '복원 완료' 판정이 열린 채 남아,
    //   그 사이 쌓인 이력이 재flush 될 때 화면이 뒤집힌다(챗 슬롯 빈 상태 → 대화 깜빡임).
    //   위 phase 대입 뒤에 둔다 — 콜백이 getViewOutputState 를 읽어도 정합.
    st.onState?.('buffering')
    // 명부 부착은 즉시 재요청(백오프 없음).
    this.issueReplay(st)
  }

  private issueReplay(st: SubState): void {
    st.myGen = undefined
    st.heldMarker = undefined
    this.armWatchdog(st)
    const token = st.token
    const viewId = this.findViewId(st)
    this.transport
      .requestReplay(st.agentId)
      .then((gen) => {
        // ★token/생존 가드★: 회수 사이 unsubscribe/재구독으로 이 SubState 가 교체됐으면 심지 않는다.
        if (viewId === null || this.subs.get(viewId)?.token !== token) return
        if (st.phase !== 'buffering') return // 이미 live/error 로 전이(늦은 회수) — 무시.
        st.myGen = gen
        // ★myGen 확정 시 held 마커 재평가(NEW-3)★: 마커가 invoke 응답보다 먼저 온 경우, 지금 판정한다.
        this.resolveHeldMarker(st)
      })
      .catch(() => {
        // requestReplay reject(미연결 등) — 마커가 안 온다. watchdog 이 재요청을 구동(정상 경로). 여기선
        //   추가 처리 불필요 — 정말 끊긴 것이면 뒤이은 detach 가 이 뷰의 타이머를 걷고 detached 로 앉히며,
        //   되살리는 건 명부 부착이다.
      })
  }

  private resolveHeldMarker(st: SubState): void {
    const held = st.heldMarker
    if (!held || st.myGen === undefined) return
    st.heldMarker = undefined
    // gen 펜스·failed·epoch 규칙을 held 에 그대로 적용(evalMarker 의 myGen 확정 이후 분기와 **같은 순서** —
    //   실패 마커가 epoch 펜스보다 앞이다. 사유는 그 분기 주석).
    if (held.gen < st.myGen) return
    if (held.failed) {
      this.ladderRerequest(st)
      return
    }
    if (st.epoch !== undefined && held.epoch !== st.epoch) return
    this.flushToLive(st, held.epoch, held.truncated)
  }

  private armWatchdog(st: SubState): void {
    this.clearWatchdog(st)
    st.watchdogTimer = setTimeout(() => {
      st.watchdogTimer = null
      if (st.phase !== 'buffering') return
      // ★flush 금지★: watchdog 은 재요청이지 부분 flush 가 아니다(§2). 사다리로 재발행(새 myGen).
      this.ladderRerequest(st)
    }, WATCHDOG_MS)
  }

  private clearWatchdog(st: SubState): void {
    if (st.watchdogTimer) {
      clearTimeout(st.watchdogTimer)
      st.watchdogTimer = null
    }
  }

  private clearTimers(st: SubState): void {
    this.clearWatchdog(st)
    if (st.backoffTimer) {
      clearTimeout(st.backoffTimer)
      st.backoffTimer = null
    }
  }

  /**
   * 지금 이 에이전트에 요청을 내도 되나.
   *
   * ★명부 미상(`roster === null`)은 **부재가 아니다** — 보낸다★: 부팅·웹뷰 리로드·명부 조회 실패가 전부
   * 여기 든다. 그걸 부재로 취급하면 조회 한 번 실패한 대가로 출력이 통째로 막히고, 정상 부팅에서도 첫
   * 그림이 명부 왕복만큼 늦어진다. 잘못 보내 봐야 최악이 거절 한 번인데, 그건 이미 흡수된다
   * (거절 → 실패 마커 → 사다리, 그리고 데몬 거절이 뷰의 single-flight 슬롯을 좀비로 남기지 않는다).
   */
  private canAttach(agentId: string): boolean {
    return this.roster === null || this.roster.has(agentId)
  }

  /** subs 는 작아 선형 탐색 무해. */
  private findViewId(target: SubState): string | null {
    for (const [viewId, st] of this.subs) {
      if (st === target) return viewId
    }
    return null
  }

  /**
   * 연결이 끊기면 모든 뷰를 detached 로 내려앉힌다 — **화면도 진도 커서도 건드리지 않는다**.
   *
   * 파이프만 사라졌지 "이 슬롯이 이 에이전트를 본다"는 사용자 의도는 그대로다. 버퍼만 버리는 이유 =
   * 그건 끊긴 세대의 미완결 replay 조각이라, 다음 부착이 새 화신이면 커서를 되돌린 뒤 그 조각이 새
   * 프레임 사이에 섞여 flush 된다. epoch 는 남긴다 — 그게 다음 명부와 대조할 "내가 읽던 화신" 이다.
   *
   * 명부도 함께 버린다: 지금 아는 명부는 방금 끊긴 데몬의 것이라, 그걸 산 데몬의 사실로 쓰면 재기동
   * 뒤에 사라진 에이전트를 살아 있다고 읽는다.
   */
  private detachAllViews(): void {
    this.roster = null
    for (const st of this.subs.values()) this.detachView(st)
  }

  /** 뷰 하나를 detached 로 내려앉힌다(멱등). 화면·진도 커서·표식은 그대로 둔다 — 위 detachAllViews 주석. */
  private detachView(st: SubState): void {
    if (st.phase === 'detached') return
    st.phase = 'detached'
    st.buffer = []
    st.bufferBytes = 0
    st.myGen = undefined
    st.heldMarker = undefined
    this.clearTimers(st)
    st.onState?.('detached')
  }

  /**
   * 권위 명부 1회 관측 — **부착과 화신 회전의 유일한 계기**(ADR-0046 amend).
   *
   * ★들어오는 문과 끌어오는 문이 둘 다 여기로 모여야 한다★: 브로드캐스트(AgentListUpdated)와 조회
   * (getAgents). 재연결 뒤 첫 명부는 조회로 오므로(eventBus.resyncAfterReconnect) 그쪽을 빠뜨리면 주
   * 복구 경로가 영영 부착되지 않는다.
   *
   * 명부에 그 에이전트가 없으면 뷰를 `detached` 로 내려앉힌다 — 아무것도 보내지 않고 기다린다(재기동한
   * 데몬의 빈 명부에 대고 전량 Subscribe 를 쏘던 거절 폭풍이 여기서 끊긴다). ★붙어 있던 뷰도 내린다★:
   * 안 내리면 수거된 에이전트를 보는 슬롯이 `getViewOutputState` 에 계속 `live` 로 보고돼, 화면은 멈춰
   * 있는데 자동화·LLM 은 살아 있다고 읽는다(그 표면은 §5 제어 계약이다).
   *
   * ★붙어 있는 뷰의 회전도 여기서 낸다★: 프레임 쪽은 표식 불일치를 그냥 떨어뜨리므로(handleOutput),
   * 재spawn 을 알아보는 자리가 여기 말고 없다. 이 갈래를 지우면 살아 있는 슬롯이 재spawn 이후 한
   * 바이트도 못 그린다.
   */
  private observeRoster(agents: AgentInfo[]): void {
    const roster = new Map(agents.map((a) => [a.id, a.epoch]))
    this.roster = roster
    for (const st of this.subs.values()) {
      // ★존재 판정은 `has`, 표식은 `get`★ — 둘을 `get() !== undefined` 로 합치면 표식을 못 실은 명부
      //   항목이 "그 에이전트는 없다" 로 읽혀 슬롯이 영영 안 붙는다.
      if (!roster.has(st.agentId)) {
        this.detachView(st)
        continue
      }
      const tag = roster.get(st.agentId)
      // 표식이 갈렸으면 다른 화신이다 — 대소는 보지 않는다(난수라 뜻이 없다). 뷰가 아직 어느 화신인지
      //   모르면(undefined) 지킬 진도도 없으므로 "같은 화신 이어보기" 로 취급한다.
      // ★명부 항목에 표식이 없으면 **같은 화신이라고 단정하지 않는다**★: 그 단정이 곧 원래 결함이다
      //   (커서를 지킨 채 0 부터 오는 새 화신을 통째로 dedup 탈락시켜 슬롯이 영영 빈 채로 남는다).
      //   반대로 잘못 회전하면 대가는 불필요한 전량 replay 1회 — 비우기와 다시 그리기가 같은 틱이라
      //   화면에는 티도 안 난다. 조용한 영구 정지보다 시끄러운 재replay 를 고른다.
      const newSession = st.epoch !== undefined && tag !== st.epoch
      // ★error 도 detached 와 같이 붙인다★: 사다리를 소진한 뷰의 유일한 회복 계기가 이것이다(끊김을
      //   거치지 않고 error 로 앉는 경로가 있어 detached 만 보면 그 뷰는 영영 못 돌아온다).
      if (st.phase === 'detached' || st.phase === 'error' || newSession) {
        this.startBuffering(st, newSession ? undefined : st.epoch, /*resetLadder*/ true, newSession)
      }
    }
  }

  // ── JSON control event 처리 ────────────────────────────────────────────────────
  private handleEvent(msg: WireEvent): void {
    if ('Ack' in msg) {
      this.resolvePending((msg.Ack as { request_id: string }).request_id, undefined)
      return
    }
    if ('Created' in msg) {
      const c = msg.Created as { request_id: string; profile: AgentProfile }
      this.resolvePending(c.request_id, c.profile)
      return
    }
    if ('Spawned' in msg) {
      const s = msg.Spawned as { request_id: string; agent: AgentInfo }
      this.resolvePending(s.request_id, s.agent)
      return
    }
    if ('Error' in msg) {
      const e = msg.Error as { request_id?: string | null; message: string }
      if (e.request_id) this.rejectPending(e.request_id, new Error(e.message))
      else console.warn('[ProtocolClient] backend error:', e.message)
      return
    }
    if ('SubscribeFailed' in msg) {
      // ★진단만 남긴다 — 상태기계는 건드리지 않는다★: 이 거절은 carrier 가 replay 경계(실패 마커)로
      //   정규화해 올린다(Tauri = src-tauri single-flight, 직결 WS = wsTransport.observeReplayWire).
      //   여기서 뷰 상태까지 만지면 같은 사건이 두 경로로 들어와 사다리를 두 번 민다.
      //   ★로그를 지우지 말 것★: 거절이 `Error` 를 떠나면서 아래 backend error warn 이 이걸 더는 못 잡는다.
      const s = msg.SubscribeFailed as { agent_id: string; reason: string }
      console.warn('[ProtocolClient] subscribe refused:', s.agent_id, s.reason)
      return
    }
    if ('SubscribeAck' in msg) {
      // ADR-0046: epoch 권위는 src-tauri decide_epoch 필터 + 성공 마커 epoch 채택으로 옮겼다. SubscribeAck 은
      //   프론트 상태기계 입력이 아니다(마커가 replay 경계·epoch 를 나른다) — 무시(관측만). truncated 는
      //   마커 flags 로 전달된다.
      return
    }
    if ('ReplayComplete' in msg) {
      // ADR-0046: 경계 판정은 replayBoundary(마커) 단독. 이 control 은 무시(carrier 가 마커로 정규화).
      return
    }
    if ('AgentList' in msg) {
      const a = msg.AgentList as { request_id: string; agents: AgentInfo[] }
      this.resolvePending(a.request_id, a.agents)
      return
    }
    if ('AgentListUpdated' in msg) {
      const agents = (msg.AgentListUpdated as { agents: AgentInfo[] }).agents
      for (const cb of this.agentListCbs) cb(agents)
      // 밀어주는 명부 문 — 끌어오는 문은 getAgents(그쪽은 이 라우팅을 지나지 않는다).
      this.observeRoster(agents)
      return
    }
    if ('ProfileList' in msg) {
      const p = msg.ProfileList as { request_id: string; profiles: AgentProfile[] }
      this.resolvePending(p.request_id, p.profiles)
      return
    }
    if ('ProfileListUpdated' in msg) {
      const profiles = (msg.ProfileListUpdated as { profiles: AgentProfile[] }).profiles
      for (const cb of this.profileListCbs) cb(profiles)
      return
    }
    if ('PresetList' in msg) {
      const p = msg.PresetList as { request_id: string; presets: Preset[] }
      this.resolvePending(p.request_id, p.presets)
      return
    }
    if ('PresetListUpdated' in msg) {
      const presets = (msg.PresetListUpdated as { presets: Preset[] }).presets
      for (const cb of this.presetListCbs) cb(presets)
      return
    }
    if ('Snapshot' in msg) {
      const s = msg.Snapshot as { request_id: string; agent_id: string; chunks: unknown[] }
      this.resolvePending(s.request_id, s.chunks)
      return
    }
    if ('StatusChanged' in msg) {
      const s = msg.StatusChanged as { agent_id: string; status: AgentStatus; epoch: number }
      for (const cb of this.statusCbs) cb(s.agent_id, s.status, s.epoch)
      return
    }
    if ('RestoreResult' in msg) {
      const r = (msg.RestoreResult as { report: RestoreReport }).report
      for (const cb of this.restoreCbs) cb(r)
      return
    }
    // Hello/InputLeaseChanged 등은 여기서 소비하지 않는다. 무시.
  }

  // ── request_id pending 헬퍼 ──────────────────────────────────────────────────────
  private resolvePending(requestId: string, value: unknown): void {
    const p = this.pending.get(requestId)
    if (p) {
      this.pending.delete(requestId)
      p.resolve(value)
    }
  }
  private rejectPending(requestId: string, err: unknown): void {
    const p = this.pending.get(requestId)
    if (p) {
      this.pending.delete(requestId)
      p.reject(err)
    }
  }

  private async sendCommand<T>(build: (requestId: string) => unknown): Promise<T> {
    await this.transport.ensureReady()
    const requestId = crypto.randomUUID()
    return new Promise<T>((resolve, reject) => {
      this.pending.set(requestId, { resolve: resolve as (v: unknown) => void, reject })
      try {
        const r = this.transport.send(build(requestId))
        if (r && typeof (r as Promise<void>).catch === 'function') {
          ;(r as Promise<void>).catch((e) => {
            this.pending.delete(requestId)
            reject(e)
          })
        }
      } catch (e) {
        this.pending.delete(requestId)
        reject(e)
      }
    })
  }

  // ── 출력 구독(뷰 단위, ADR-0046 F1) ─────────────────────────────────────────────────
  async subscribeOutput(
    viewId: string,
    agentId: string,
    onChunk: (chunk: OutputChunk) => void,
    onState?: (state: ViewPhase) => void,
    onReset?: ViewResetFn,
  ): Promise<OutputSubscription> {
    const token = ++this.subSeq
    const st: SubState = {
      agentId,
      onChunk,
      onState,
      onReset,
      phase: 'buffering',
      buffer: [],
      bufferBytes: 0,
      myGen: undefined,
      heldMarker: undefined,
      lastDeliveredSeq: -1,
      epoch: undefined,
      restartPending: false,
      token,
      attempts: 0,
      backoffTimer: null,
      watchdogTimer: null,
    }
    // ★기존 SubState 타이머 정리(FIX-1)★: 같은 viewId 재구독은 아래 subs.set 이 옛 SubState 를 맵에서
    //   교체하지만, 옛 SubState 가 무장한 watchdog/backoff 타이머는 clear 하지 않으면 살아남아 만료 시
    //   ladderRerequest 로 stray requestReplay 를 낸다(옛 st 는 issueReplay 의 token 가드로 재발행은 못
    //   막지만, 이미 예약된 타이머의 콜백은 그 가드 앞에서 실행돼 재요청 storm 을 유발). 교체 전 정리한다.
    const prev = this.subs.get(viewId)
    if (prev) this.clearTimers(prev)
    // ★subs.set 을 await *이전* 에 동기 실행(StrictMode 이중구독 레이스 차단)★: 이 함수는 async 라
    //   `await ensureReady()` 에서 microtask yield 한다. StrictMode(dev 이중 마운트)는 같은 구독
    //   effect 를 급속 2회 돌린다 — 다른 viewId 면 서로 다른 SubState(공존, 정상), 같은 viewId(재구독)면
    //   set 을 await 앞으로 끌어올려 최종 생존 SubState 를 확정한다. 아래 replay 발행 가드의 전제.
    this.subs.set(viewId, st)
    // ★ensureReady 실패 시 좀비 구독 롤백★: set 을 await 앞으로 옮긴 탓에 ensureReady reject/hang 시 st 가
    //   subs 에 잔존해 프레임이 죽은 구독으로 샌다. 실패 시 자기 등록만 롤백(token 가드로 정상 재구독 보호).
    try {
      await this.transport.ensureReady()
    } catch (e) {
      if (this.subs.get(viewId)?.token === token) {
        this.clearTimers(st)
        this.subs.delete(viewId)
      }
      throw e
    }
    // ★생존 구독자만 replay 발행(StrictMode 중복 invoke 억제)★: subs 엔트리가 내 token 일 때만 requestReplay
    //   를 낸다 — 교체된 옛 st 는 skip(중복 재요청 storm 방지). single-flight 가 병합하므로 정상 mount 의
    //   배정 트리거 replay 와 겹쳐도 안전하다.
    if (this.subs.get(viewId)?.token === token) {
      // ★mount 도 재부착과 **같은 한 규칙**을 쓴다(사용자 결정 2026-08-20)★: 옛 mount 는 명부를 안 보고
      //   무조건 쏘았고, 그래서 없는 에이전트를 가리킨 슬롯이 거절 → 사다리 → error 로 굳었다. 판정은
      //   `canAttach` 하나로 모은다 — 규칙이 둘이면 같은 질문에 두 답이 생긴다.
      if (this.canAttach(st.agentId)) {
        this.issueReplay(st)
      } else {
        // 명부가 "없다" 고 말한 동안은 아무것도 보내지 않는다. 되살리는 건 명부 관측(observeRoster).
        st.phase = 'detached'
        st.onState?.('detached')
      }
    } else {
      // 교체된 옛 st — 타이머 없이 조용히 빠진다(생존 구독자가 발행). buffer 도 안 채워짐(fan-out 은 산 st 만
      //   맞지만, 이 st 는 subs 에서 이미 교체돼 viewsForAgent 에 안 잡힌다).
      this.clearTimers(st)
    }
    return {
      unsubscribe: () => {
        // ★현재 subs 엔트리가 내 token 일 때만 delete(stale-unsubscribe 가드)★. 재구독으로 새 SubState 가
        //   들어온 뒤 늦게 온 옛 unsubscribe 가 산 구독을 지우는 걸 막는다.
        if (this.subs.get(viewId)?.token === token) {
          this.clearTimers(st)
          this.subs.delete(viewId)
        }
        // ★BLOCK-1(ADR-0046)★: wire Subscribe/Unsubscribe 를 어떤 경로로도 안 보낸다. 데몬 구독 정리는
        //   라우터 Unsubscribe(prune) 단독. 여기선 JS 콜백만 떼어 더는 이 agent frame 을 렌더하지 않게 한다.
      },
    }
  }

  /**
   * ★LLM 제어 표면(§5)★ — 뷰별 replay 상태 조회. error 소진(재요청 3회 실패) 등을 LLM/자동화가 관측·재구동
   * 판단에 쓴다.
   */
  getViewOutputState(viewId: string): ViewOutputState | null {
    const st = this.subs.get(viewId)
    if (!st) return null
    return { agentId: st.agentId, phase: st.phase, buffered: st.buffer.length, attempts: st.attempts }
  }

  // ── 명령(인터페이스 → wire) ───────────────────────────────────────────────────────
  spawnAgent(cwd: string): Promise<AgentInfo> {
    return this.sendCommand<AgentInfo>((request_id) => ({ SpawnByCwd: { cwd, request_id } }))
  }
  killAgent(agentId: string): Promise<void> {
    return this.sendCommand<void>((request_id) => ({ Kill: { agent_id: agentId, request_id } }))
  }
  interruptAgent(agentId: string): Promise<void> {
    return this.sendCommand<void>((request_id) => ({
      Interrupt: { agent_id: agentId, request_id },
    }))
  }
  writeStdin(agentId: string, data: Uint8Array): Promise<void> {
    return this.sendCommand<void>((request_id) => ({
      WriteStdin: { agent_id: agentId, data: Array.from(data), request_id },
    }))
  }
  async resizePty(agentId: string, cols: number, rows: number): Promise<void> {
    await this.transport.ensureReady()
    this.transport.send({ Resize: { agent_id: agentId, cols, rows, viewport_id: null } })
  }
  async getAgents(): Promise<AgentInfo[]> {
    const agents = await this.sendCommand<AgentInfo[]>((request_id) => ({ ListAgents: { request_id } }))
    // ★끌어오는 명부 문★: reply 는 request_id pending 으로 회수돼 handleEvent 의 브로드캐스트 분기를
    //   지나지 않는다. 재연결 직후 첫 명부가 이 경로로 오므로 여기서 직접 먹인다(observeRoster 주석).
    this.observeRoster(agents)
    return agents
  }
  getSnapshot(agentId: string): Promise<unknown[]> {
    return this.sendCommand<unknown[]>((request_id) => ({
      GetSnapshot: { agent_id: agentId, request_id },
    }))
  }
  stopDaemon(force: boolean): Promise<void> {
    return this.sendCommand<void>((request_id) => ({
      StopDaemon: { force, kill_agents: true, request_id },
    }))
  }

  // ── 프로필 CRUD ────────────────────────────────────────────────────────────────
  listProfiles(): Promise<AgentProfile[]> {
    return this.sendCommand<AgentProfile[]>((request_id) => ({ ListProfiles: { request_id } }))
  }
  createClaudeProfile(
    name: string,
    cwd: string,
    extraArgs: string[],
    env: [string, string][],
    autoRestore: boolean,
    outputFormat: ClaudeOutputFormat = 'Terminal',
  ): Promise<AgentProfile> {
    return this.sendCommand<AgentProfile>((request_id) => ({
      CreateProfile: {
        name,
        cwd,
        extra_args: extraArgs,
        env,
        auto_restore: autoRestore,
        output_format: outputFormat,
        request_id,
      },
    }))
  }
  deleteProfile(agentId: string): Promise<void> {
    return this.sendCommand<void>((request_id) => ({
      DeleteProfile: { profile_id: agentId, request_id },
    }))
  }
  spawnProfile(agentId: string, resume: boolean): Promise<AgentInfo> {
    return this.sendCommand<AgentInfo>((request_id) => ({
      SpawnProfile: { profile_id: agentId, resume, request_id },
    }))
  }
  setProfileAutoRestore(agentId: string, autoRestore: boolean): Promise<void> {
    return this.sendCommand<void>((request_id) => ({
      SetProfileAutoRestore: { profile_id: agentId, auto_restore: autoRestore, request_id },
    }))
  }
  renameProfile(agentId: string, name: string | null): Promise<void> {
    // 백엔드 reply=Ack(void). 표시명 반영은 뒤이은 ProfileListUpdated broadcast(낙관 갱신 X, ADR-0061).
    return this.sendCommand<void>((request_id) => ({
      RenameProfile: { profile_id: agentId, name, request_id },
    }))
  }
  reparentProfile(childId: string, parentId: string | null): Promise<void> {
    // 백엔드 reply=Ack(void). 계층 반영은 뒤이은 ProfileListUpdated broadcast(낙관 갱신 X, ADR-0072).
    //   invalid move(cycle/self/2단/존재하지 않는 parent)는 백엔드가 Error → sendCommand reject.
    return this.sendCommand<void>((request_id) => ({
      ReparentProfile: { child_id: childId, parent_id: parentId, request_id },
    }))
  }

  // ── 프리셋 CRUD(ADR-0061) ──────────────────────────────────────────────────────
  listPresets(): Promise<Preset[]> {
    return this.sendCommand<Preset[]>((request_id) => ({ ListPresets: { request_id } }))
  }
  createPreset(cwd: string): Promise<void> {
    // 백엔드 reply=Ack(void). 생성된 프리셋은 뒤이은 PresetListUpdated broadcast 로 store 에 들어온다
    //   (createClaudeProfile 이 Created{profile} 를 돌려주는 것과 다름 — 프리셋은 이름을 안 실어 reply 가 Ack).
    return this.sendCommand<void>((request_id) => ({ CreatePreset: { cwd, request_id } }))
  }
  deletePreset(id: string): Promise<void> {
    return this.sendCommand<void>((request_id) => ({ DeletePreset: { preset_id: id, request_id } }))
  }
  renamePreset(id: string, name: string | null): Promise<void> {
    // 백엔드 reply=Ack(void). 표시명 반영은 뒤이은 PresetListUpdated broadcast(낙관 갱신 X, ADR-0061).
    return this.sendCommand<void>((request_id) => ({ RenamePreset: { preset_id: id, name, request_id } }))
  }

  // ── 상태/목록/복원/프로필 이벤트 — 레지스트리 등록 + remove disposer ──────────────────
  onAgentListUpdated(cb: (agents: AgentInfo[]) => void): () => void {
    this.agentListCbs.add(cb)
    return () => {
      this.agentListCbs.delete(cb)
    }
  }
  onStatusChanged(cb: (id: string, status: AgentStatus, epoch: number) => void): () => void {
    this.statusCbs.add(cb)
    return () => {
      this.statusCbs.delete(cb)
    }
  }
  onRestoreResult(cb: (report: RestoreReport) => void): () => void {
    this.restoreCbs.add(cb)
    return () => {
      this.restoreCbs.delete(cb)
    }
  }
  onProfileListUpdated(cb: (profiles: AgentProfile[]) => void): () => void {
    this.profileListCbs.add(cb)
    return () => {
      this.profileListCbs.delete(cb)
    }
  }
  onPresetListUpdated(cb: (presets: Preset[]) => void): () => void {
    this.presetListCbs.add(cb)
    return () => {
      this.presetListCbs.delete(cb)
    }
  }

  // ── 명시 종료 ───────────────────────────────────────────────────────────────────
  close(): void {
    const closed = new Error('client closed')
    for (const p of this.pending.values()) p.reject(closed)
    this.pending.clear()
    for (const st of this.subs.values()) this.clearTimers(st)
    this.subs.clear()
    if (this.offMessage) {
      this.offMessage()
      this.offMessage = null
    }
    if (this.offState) {
      this.offState()
      this.offState = null
    }
    this.transport.close()
  }
}
