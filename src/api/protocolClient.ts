// ProtocolClient — AgentClient 의 carrier-무관 구현 (ADR-0020 결정3, TRD Stage 3 · ADR-0046 뷰 직결 replay).
//
// 프로토콜 의미론(request_id 매칭 · 뷰별 seq dedup · epoch 가드 · replay 경계 gen 펜스 · on* 이벤트
// 라우팅)을 **한 곳**에 모은다. carrier(전송)는 Transport 가 추상화한다 —
// WsTransport(WS+재연결) / TauriTransport(invoke+Channel). 이 클래스는 DaemonClient 에서 carrier-무관
// 로직만 승격한 것이고, WS-특정(openSocket/Auth/Hello/scheduleReconnect/binary frame 디코드)은 transport 로
// 분리됐다.
//
// ★ADR-0046 재설계(S16) — 뷰 직결 replay★: src-tauri 미러 버퍼를 제거하고, remount/리로드/재연결은
//   데몬 ring 전량 재replay 로 대체했다. 진도 상태의 유일한 거처 = **웹뷰 뷰(slot) 단위**. 그래서 subs 를
//   agentId 가 아니라 **viewId(slot id)** 로 re-key 한다(버그 B 구조 해소 — 같은 agent 를 N 뷰가 봐도
//   각자 독립 진도). replay 경계는 transport 가 올리는 replayBoundary 제어 이벤트(tag=255 마커의 정규화)
//   로 판정하고, 뷰는 자기 requestReplay 가 반환한 myGen 이상의 성공 마커에만 sort+dedup flush 한다.

import type {
  AgentClient,
  ConnectionState,
  OutputChunk,
  OutputSubscription,
  ViewOutputState,
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
/** 사다리 백오프(ms) — 시도별 지수. attempts=1→1s, 2→2s, 3→4s 뒤 재요청. */
const LADDER_BACKOFF_MS = [1000, 2000, 4000]
/** watchdog 만료(ms) — buffering 에서 이 시간 내 성공 마커가 안 오면 재요청(flush 아님, §2). */
const WATCHDOG_MS = 10_000
/**
 * 뷰 buffering 버퍼 상한 — ring 상한의 2배(§4). 버퍼가 "이전 replay 꼬리 + 자기 replay 전체"를 담을 수
 * 있어야 하므로(Codex 재리뷰 #5). 초과 시 부분 유지(drop-oldest) 금지 → buffer 폐기 + 재요청.
 */
const VIEW_BUFFER_MAX_BYTES = 4 * 1024 * 1024
const VIEW_BUFFER_MAX_FRAMES = 8192

type ViewPhase = 'buffering' | 'live' | 'error'

/** 버퍼링된 frame(도착 순). flush 는 seq 오름차순 정렬 후 배달(out-of-order 방어). tag 보존. */
interface BufferedFrame {
  tag: number
  seq: number
  bytes: Uint8Array
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
  /** 이 뷰가 보는 agent. fan-out 은 이 값으로 대상 뷰를 고른다(같은 agent → 모든 뷰). */
  agentId: string
  onChunk: (chunk: OutputChunk) => void
  /** 상태 변화 통지(옵션) — 슬롯이 error 표면화·streaming 힌트에 쓴다(LLM 제어 표면과 짝). */
  onState?: (state: ViewPhase) => void
  phase: ViewPhase
  /** buffering 중 축적 프레임(도착 순). live 전이 시 sort+dedup flush 후 비운다. */
  buffer: BufferedFrame[]
  /** buffer 총 바이트(상한 판정). */
  bufferBytes: number
  /**
   * 이 뷰의 requestReplay 가 반환한 gen. **미확정(undefined)**: invoke 응답 전(Channel 이 먼저 올 수
   * 있음 — NEW-3). gen 펜스: 자기 myGen 이상의 성공 마커에만 flush(남의/이전 replay 조기 flush 차단).
   */
  myGen: bigint | undefined
  /**
   * myGen 미확정 중 도착한 마커를 버리지 않고 최고 gen 1개 보관(§2 NEW-3). myGen 확정 시 재평가한다.
   */
  heldMarker: HeldMarker | undefined
  /** onChunk 로 실제 배달한 최고 seq(high-water). 초기 -1. dedup 기준(live·flush 공통). */
  lastDeliveredSeq: number
  /**
   * buffering 대상 epoch. undefined = 아직 모름(첫 frame/마커가 확정). frame(epoch 더 높음)이 오면
   * 버퍼 폐기 + 재요청(§2). live 전이 시 성공 마커 epoch 를 채택한다.
   */
  epoch: number | undefined
  /** stale-unsubscribe 가드용 고유 번호표(subscribeOutput 마다 ++subSeq). */
  token: number
  /** 재요청 사다리 시도 횟수(watchdog/실패마커/상한 초과 누적). remount·connected 는 0 리셋. */
  attempts: number
  /** 진행 중인 백오프 타이머(사다리 재요청 예약). unsubscribe·전이 시 clear. */
  backoffTimer: ReturnType<typeof setTimeout> | null
  /** buffering watchdog 타이머(성공 마커 없이 만료 → 재요청). live/error 전이·flush 시 clear. */
  watchdogTimer: ReturnType<typeof setTimeout> | null
}

interface Pending {
  resolve: (v: unknown) => void
  reject: (e: unknown) => void
}

type WireEvent = Record<string, unknown>

// ★ViewOutputState(§5 LLM 제어 표면 — ADR-0046)★는 AgentClient 인터페이스가 정본(agentClient.ts) —
//   getViewOutputState 가 인터페이스 메서드라 타입도 거기 둔다(LLM 이 타입으로 발견). 여기선 import 만.

export class ProtocolClient implements AgentClient {
  private readonly transport: Transport

  // 조회(getAgents/listProfiles/getSnapshot)와 side-effect(spawn/kill 등) 응답을 request_id 로
  // 매칭하는 단일 pending map.
  private pending = new Map<string, Pending>()
  // ★viewId(slot id) 키(ADR-0046)★: agentId 가 아니라 뷰 단위 — 같은 agent 를 N 뷰가 봐도 각자 독립
  //   진도(버그 B 구조 해소). frame 은 agentId 로 대상 뷰들을 골라 fan-out 한다.
  private subs = new Map<string, SubState>()
  // 구독마다 발급하는 단조증가 번호표 — stale-unsubscribe 가드(SubState.token).
  private subSeq = 0

  // 상태/목록/복원/프로필 이벤트 콜백 레지스트리(broadcast). eventBus 가 소비.
  private agentListCbs = new Set<(agents: AgentInfo[]) => void>()
  private statusCbs = new Set<(id: string, status: AgentStatus, epoch: number) => void>()
  private restoreCbs = new Set<(report: RestoreReport) => void>()
  private profileListCbs = new Set<(profiles: AgentProfile[]) => void>()
  private presetListCbs = new Set<(presets: Preset[]) => void>()

  // transport 구독 해제 핸들.
  private offMessage: (() => void) | null = null
  private offState: (() => void) | null = null

  // connected 재전이 판정용(중복 통지 방어).
  private lastState: ConnectionState

  constructor(transport: Transport) {
    this.transport = transport
    this.lastState = transport.connectionState
    // 단일 수신 라우터 등록 — control/output/replayBoundary 정규화 메시지를 carrier 무관하게 라우팅.
    this.offMessage = transport.onMessage((msg) => this.route(msg))
    // 연결 상태가 connected 로 (재)전이하면 모든 뷰를 buffering 리셋 + 재요청(ADR-0046 §2: 재연결 =
    //   전량 재replay·마커 재장전, buffering 고착 자가 복구). 비-connected 전이는 pending 명령 reject.
    this.offState = transport.onConnectionStateChange((s) => {
      const prev = this.lastState
      this.lastState = s
      if (s === 'connected' && prev !== 'connected') {
        // ADR-0046: pendingBuffers/resubscribeAll(wire Subscribe 송신) 삭제 — 뷰 buffering 리셋+재요청으로 대체.
        this.reconnectResetAllViews()
      } else if (s !== 'connected' && prev === 'connected') {
        const lost = new Error('connection lost')
        for (const p of this.pending.values()) p.reject(lost)
        this.pending.clear()
      }
    })
  }

  // ── 연결 상태(transport 위임) ───────────────────────────────────────────────────
  get connectionState(): ConnectionState {
    return this.transport.connectionState
  }

  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void {
    return this.transport.onConnectionStateChange(cb)
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

  /** agentId 가 f.agentId 인 모든 뷰 SubState 를 순회(fan-out 대상 — ADR-0046 뷰별 독립 진도). */
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
  // ADR-0046: 뷰 상태전이표(frame 행) — buffering(epoch 규칙 push) / live(epoch·seq dedup 직행).
  private handleOutput(f: {
    tag: number
    agentId: string
    epoch: number
    seq: number
    bytes: Uint8Array
  }): void {
    for (const st of this.viewsForAgent(f.agentId)) {
      if (st.phase === 'error') continue // error 뷰는 재요청 소진 상태 — 프레임 무시(remount/재연결이 리셋).
      if (st.phase === 'live') {
        // live: frame(epoch 더 높음) → drop([agentId,epoch] remount 흐름이 처리). epoch 일치·seq 진도면 배달.
        if (st.epoch !== undefined && f.epoch !== st.epoch) continue
        if (f.seq <= st.lastDeliveredSeq) continue // dedup(중복 replay 흡수)
        st.lastDeliveredSeq = f.seq
        st.onChunk({ tag: f.tag, seq: f.seq, bytes: f.bytes })
        continue
      }
      // buffering: epoch 규칙(§2 상태전이표).
      if (st.epoch !== undefined && f.epoch > st.epoch) {
        // frame(epoch 더 높음): buffer 폐기 → 새 epoch 로 buffering + requestReplay 재발행(새 myGen).
        //   구 epoch 대상이던 기존 myGen 마커는 무효(NEW-5). 사다리는 이 재장전이 자연 리셋(startBuffering).
        this.startBuffering(st, f.epoch, /*resetLadder*/ true)
        // 이 frame 자체도 새 buffer 에 담는다(첫 프레임).
        this.pushBuffer(st, f)
        continue
      }
      if (st.epoch !== undefined && f.epoch < st.epoch) {
        // 구 epoch 잔여 frame — buffering 대상 아님(무시). (상태전이표 "epoch=대기 epoch 또는 미정"만 push)
        continue
      }
      // epoch 일치 또는 미정 → buffer push(상한 초과는 pushBuffer 가 폐기+재요청).
      if (st.epoch === undefined) st.epoch = f.epoch // 첫 frame 이 대기 epoch 를 확정(마커가 최종 채택).
      this.pushBuffer(st, f)
    }
  }

  /** buffering 뷰의 buffer 에 frame push. 상한 초과 시 부분 유지 금지 → buffer 폐기 + 재요청(§2·§4). */
  private pushBuffer(
    st: SubState,
    f: { tag: number; seq: number; bytes: Uint8Array },
  ): void {
    st.buffer.push({ tag: f.tag, seq: f.seq, bytes: f.bytes })
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

  // ADR-0046: 뷰 상태전이표(마커 행) — gen 펜스 + 성공/실패/held 판정.
  /**
   * ★replay 경계 마커(ADR-0046 §2)★ — transport 가 tag=255 마커를 정규화해 올린 제어 이벤트.
   *   agent 를 보는 모든 뷰로 fan-out 하되, 각 뷰가 자기 gen 펜스로 판정한다(남의 replay 경계는 무시).
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
    // live 뷰: 마커(어떤 gen이든) 무시 — fan-out 으로 도달하는 남의 replay 경계. dedup 만으로 충분(§2).
    if (st.phase !== 'buffering') return
    // ★myGen 미확정(NEW-3)★: 마커를 버리지 않고 최고 gen 1개 보관 → myGen 확정 시 재평가(resolveMyGen).
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
    if (st.epoch !== undefined && m.epoch !== st.epoch) return
    if (m.failed) {
      // ★실패 마커(§2)★: flush 금지 — buffer 는 유지한 채(sort+dedup 가 중복 흡수, 폐기 불필요) 재요청
      //   사다리. 왜 buffer 유지: 이 replay 는 미완결이나 다음 replay 가 full-from-oldest 라 겹치는 앞부분을
      //   dedup 가 흡수한다 — 버리면 오히려 다시 받아야 할 프레임을 손해.
      this.ladderRerequest(st)
      return
    }
    // ★성공 마커(gen≥myGen, epoch 일치)★: sort+seq dedup flush → live 전이(epoch 채택, high-water=꼬리).
    this.flushToLive(st, m.epoch, m.truncated)
  }

  /** buffering → live: buffer 를 seq 오름차순 정렬 후 dedup 배달, epoch 채택, 타이머 정리. */
  private flushToLive(st: SubState, epoch: number, truncated: boolean): void {
    // ★epoch 채택★: 성공 마커의 epoch 로 확정(src-tauri decide_epoch 1차 필터를 통과한 값 — ADR-0046 은
    //   ADR-0007 "epoch 권위=SubscribeAck 단독"을 amends: src-tauri 필터 + 프론트는 필터된 frame/마커 채택).
    st.epoch = epoch
    // seq 오름차순 정렬 후 flush(out-of-order 도착 방어): 배열 순서대로면 큰 seq 를 먼저 배달해 high-water 를
    //   올린 뒤 작은 seq 가 dedup 탈락한다. 정렬로 전부 배달. 같은 seq 는 dedup 이 자연 제거.
    const ordered = [...st.buffer].sort((a, b) => a.seq - b.seq)
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
   * 재요청 사다리(bounded — §2·§4). watchdog/실패 마커/상한 초과가 부른다. 시도 상한(3) + 지수 백오프.
   * 소진 시 phase='error' 전이(무한 폭주 금지) + onState 표면화. remount·connected 전이가 사다리 리셋.
   */
  private ladderRerequest(st: SubState): void {
    // 진행 중 백오프가 있으면 중복 예약 안 함(한 사다리 단계는 한 타이머). watchdog 도 재무장은 requestReplay 후.
    if (st.backoffTimer) return
    if (st.attempts >= LADDER_MAX_ATTEMPTS) {
      // 소진 → error. buffer·타이머 정리, LLM 제어 표면·슬롯에 표면화.
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
   * buffering 재시작 — buffer 폐기·타이머 정리, epoch 대기값 설정. resetLadder 면 attempts 0(remount/
   * epoch 회전/connected). requestReplay 는 호출자가 별도로(issueReplay) — epoch 회전은 즉시, 사다리는
   * 백오프 뒤.
   */
  private startBuffering(st: SubState, epoch: number | undefined, resetLadder: boolean): void {
    st.phase = 'buffering'
    st.buffer = []
    st.bufferBytes = 0
    st.epoch = epoch
    st.myGen = undefined
    st.heldMarker = undefined
    if (resetLadder) st.attempts = 0
    this.clearTimers(st)
    // epoch 회전(§2 상태전이표)은 즉시 재요청(백오프 없음). issueReplay 가 watchdog 재무장.
    this.issueReplay(st)
  }

  /**
   * 이 뷰의 requestReplay 발행 + myGen 회수 + watchdog 무장. token 가드로 stale 재개를 막는다(그 사이
   * unsubscribe/재구독으로 SubState 가 교체됐으면 myGen 을 심지 않는다). 발행마다 myGen·heldMarker 리셋.
   */
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
        //   추가 처리 불필요(connected 재전이가 별도로 전량 리셋).
      })
  }

  /** myGen 확정 후 보관 마커 재평가(§2 NEW-3). held 를 소비하고 evalMarker 규칙 재적용. */
  private resolveHeldMarker(st: SubState): void {
    const held = st.heldMarker
    if (!held || st.myGen === undefined) return
    st.heldMarker = undefined
    // gen 펜스·epoch·failed 규칙을 held 에 그대로 적용(evalMarker 의 myGen 확정 이후 분기와 동일).
    if (held.gen < st.myGen) return
    if (st.epoch !== undefined && held.epoch !== st.epoch) return
    if (held.failed) {
      this.ladderRerequest(st)
      return
    }
    this.flushToLive(st, held.epoch, held.truncated)
  }

  /** buffering watchdog 무장(재무장 시 기존 타이머 교체). 만료 = flush 금지·재요청(§2). */
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

  /** SubState → viewId 역참조(issueReplay 의 token 가드용). subs 는 작아 선형 탐색 무해. */
  private findViewId(target: SubState): string | null {
    for (const [viewId, st] of this.subs) {
      if (st === target) return viewId
    }
    return null
  }

  /** connected 재전이(재연결) → 모든 뷰 buffering 리셋(buffer 폐기) + 재요청(§2). 사다리 리셋. */
  private reconnectResetAllViews(): void {
    for (const st of this.subs.values()) {
      // error 뷰도 재연결에선 회복 기회를 준다(연결이 새로 났으므로 사다리 리셋 + buffering 재시작).
      this.startBuffering(st, undefined, /*resetLadder*/ true)
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
    // 프리셋(ADR-0061) — ProfileList/ProfileListUpdated 와 동형. PresetList=전용 reply(request_id 매칭),
    //   PresetListUpdated=CRUD 후 broadcast(콜백 fan-out).
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
  /**
   * 뷰(slot) 단위 출력 구독. viewId = 슬롯 id(컴포넌트가 이미 가진 값). onState 는 옵션(슬롯이 error·
   * streaming 표면화에 쓴다). frame 은 agentId 로 fan-out 되고, 이 뷰는 buffering→(gen 펜스 성공 마커)→
   * live 로 전이하며 그때부터 onChunk 로 직행 배달한다.
   */
  async subscribeOutput(
    viewId: string,
    agentId: string,
    onChunk: (chunk: OutputChunk) => void,
    onState?: (state: ViewPhase) => void,
  ): Promise<OutputSubscription> {
    const token = ++this.subSeq
    const st: SubState = {
      agentId,
      onChunk,
      onState,
      phase: 'buffering',
      buffer: [],
      bufferBytes: 0,
      myGen: undefined,
      heldMarker: undefined,
      lastDeliveredSeq: -1,
      epoch: undefined,
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
    //   `await ensureReady()` 에서 microtask yield 한다. StrictMode(dev 이중 마운트)는 같은 [agentId,epoch]
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
      this.issueReplay(st)
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
   * 판단에 쓴다. 없는 viewId 면 null. (최소 노출 — phase·buffered·attempts 만.)
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
  getAgents(): Promise<AgentInfo[]> {
    return this.sendCommand<AgentInfo[]>((request_id) => ({ ListAgents: { request_id } }))
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
