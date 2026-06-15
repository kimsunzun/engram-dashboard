// DaemonClient — AgentClient 의 WS(데몬) 구현(S12 phase4-2 step2, daemon-design §3-a).
//
// EmbeddedClient(invoke/Channel)와 동일 인터페이스. transport 디테일(WS·binary frame·
// request_id 매칭·재연결·seq dedup)을 전부 여기 캡슐화한다. 인터페이스는 디코드된
// 바이트 청크만 노출한다(§3-a 손발/두뇌 분리: 프론트=순수 I/O).
//
// wire 계약은 crates/engram-dashboard-protocol(messages.rs / codec.rs)과 1:1.
//   - control 경로 = JSON text(externally-tagged enum). unit variant 는 JSON 문자열.
//   - output hot path = WS binary frame([tag:1][agentId:16][epoch:4 BE][seq:8 BE][payload]).

import { invoke } from '@tauri-apps/api/core'

import type { AgentClient, ConnectionState, OutputChunk, OutputSubscription } from './agentClient'
import type { AgentInfo, AgentProfile, AgentStatus, RestoreReport } from './types'

// ── discover_daemon DTO(discovery.rs DaemonInfoDto 미러) ──────────────────────────
interface DaemonInfoDto {
  pid: number
  host: string
  port: number
  token: string
  protocol_version: number
}

// ── codec.rs binary frame 상수(반드시 codec.rs 와 일치) ─────────────────────────────
const FRAME_TAG_TERMINAL_BYTES = 0
const FRAME_HEADER_LEN = 1 + 16 + 4 + 8 // 29

/**
 * binary output frame 디코드 — codec.rs `encode_terminal_frame`/`decode_frame`의 역.
 * 포맷(big-endian): [tag:1][agentId:16][epoch:4 BE][seq:8 BE][raw payload...].
 * 미지원 tag·길이 부족 시 null(무시). 순수 함수 — 테스트·리뷰 용이.
 */
export function decodeOutputFrame(
  buf: ArrayBuffer,
): { tag: number; agentId: string; epoch: number; seq: number; payload: Uint8Array } | null {
  if (buf.byteLength < FRAME_HEADER_LEN) return null
  const view = new DataView(buf)
  const tag = view.getUint8(0)
  // codec.rs: tag != FRAME_TAG_TERMINAL_BYTES 면 UnknownTag — 미지원 출력 variant 는 버린다.
  if (tag !== FRAME_TAG_TERMINAL_BYTES) return null

  // agentId: byte[1..17] = AgentId(Uuid).as_bytes() — RFC4122 network order(표준 바이트 그대로).
  // 16바이트 hex 후 8-4-4-4-12 하이픈 삽입 = 구독 시 보낸 소문자 하이픈 UUID 와 동일 표현.
  const bytes = new Uint8Array(buf, 1, 16)
  const agentId = bytesToUuid(bytes)

  // epoch/seq: codec.rs 가 to_be_bytes — BE 로 읽는다(false=big-endian).
  const epoch = view.getUint32(17, false)
  const seq = Number(view.getBigUint64(21, false)) // seq 는 number 로 유지(설계 결정)

  const payload = new Uint8Array(buf, FRAME_HEADER_LEN)
  return { tag, agentId, epoch, seq, payload }
}

const HEX: string[] = Array.from({ length: 256 }, (_, i) => i.toString(16).padStart(2, '0'))

/** 16바이트 UUID → 소문자 하이픈 문자열(8-4-4-4-12). uuid 표준 바이트 순서 그대로. */
function bytesToUuid(b: Uint8Array): string {
  return (
    HEX[b[0]] + HEX[b[1]] + HEX[b[2]] + HEX[b[3]] + '-' +
    HEX[b[4]] + HEX[b[5]] + '-' +
    HEX[b[6]] + HEX[b[7]] + '-' +
    HEX[b[8]] + HEX[b[9]] + '-' +
    HEX[b[10]] + HEX[b[11]] + HEX[b[12]] + HEX[b[13]] + HEX[b[14]] + HEX[b[15]]
  )
}

// ── 내부 구독 상태 ─────────────────────────────────────────────────────────────────
interface SubState {
  onChunk: (chunk: OutputChunk) => void
  /**
   * 마지막 SubscribeAck.current_epoch. binary frame epoch 매칭용(불일치 frame 폐기) +
   * 재연결 resubscribe wire epoch. undefined = 아직 Ack 못 받음(첫 구독 직후).
   */
  epoch: number | undefined
  /**
   * onChunk 로 **실제 배달한** 최고 seq(high-water). 초기 -1(아무것도 배달 안 함).
   * dedup 기준이자 재연결 after_seq. replay_from 에 의존하지 않는다(replay_from 은
   * "데몬이 보내는 첫 seq"이지 "마지막으로 본 seq"가 아니라 off-by-one 유발 — 버그 B).
   */
  lastDeliveredSeq: number
}

interface Pending {
  resolve: (v: unknown) => void
  reject: (e: unknown) => void
}

/** request_id 없는 broadcast 편승 조회의 일회성 waiter — 끊김/close 시 reject 하려고 reject 동봉. */
interface Waiter<T> {
  resolve: (v: T) => void
  reject: (e: unknown) => void
}

// ── wire helper 타입(좁게) ─────────────────────────────────────────────────────────
type WireEvent = Record<string, unknown>

export class DaemonClient implements AgentClient {
  private ws: WebSocket | null = null

  private _state: ConnectionState = 'down'
  private stateListeners = new Set<(s: ConnectionState) => void>()

  // 진행 중 연결 시도(중복 연결 방지). resolve 는 Hello 수신(=인증 성공) 시.
  private connectPromise: Promise<void> | null = null

  private pending = new Map<string, Pending>()
  private subs = new Map<string, SubState>()

  // request_id 없는 응답을 기다리는 일회성 waiter(getAgents/getSnapshot/listProfiles).
  // 끊김/close 시 reject 하려고 resolve+reject 쌍으로 보관.
  private agentListWaiters: Array<Waiter<AgentInfo[]>> = []
  private profileListWaiters: Array<Waiter<AgentProfile[]>> = []
  private snapshotWaiters = new Map<string, Array<Waiter<unknown[]>>>()

  private closedByUser = false
  private reconnectAttempt = 0
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null

  // 상태/목록/복원 이벤트 콜백 레지스트리(broadcast). EmbeddedClient 의 Tauri listen 과 동일 의미를
  // WS 이벤트(AgentListUpdated/StatusChanged/RestoreResult)로 제공한다 — eventBus 가 소비.
  private agentListCbs = new Set<(agents: AgentInfo[]) => void>()
  private statusCbs = new Set<(id: string, status: AgentStatus, epoch: number) => void>()
  private restoreCbs = new Set<(report: RestoreReport) => void>()

  // ── 연결 상태 ──────────────────────────────────────────────────────────────────
  get connectionState(): ConnectionState {
    return this._state
  }

  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void {
    this.stateListeners.add(cb)
    // Embedded 와 동일 UX: 등록 즉시 현재 상태 1회 통지.
    cb(this._state)
    return () => {
      this.stateListeners.delete(cb)
    }
  }

  private setState(s: ConnectionState): void {
    if (this._state === s) return
    this._state = s
    for (const cb of this.stateListeners) cb(s)
  }

  // ── 연결 수립(lazy, 중복 방지) ──────────────────────────────────────────────────
  private ensureConnected(): Promise<void> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN && this._state === 'connected') {
      return Promise.resolve()
    }
    if (this.connectPromise) return this.connectPromise
    this.connectPromise = this.openSocket()
    return this.connectPromise
  }

  /** 1회 소켓 열기 + Auth 전송 + Hello 대기. 성공 시 resolve, 실패 시 reject(상위가 처리). */
  private openSocket(): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      const run = async () => {
        // discover_daemon 은 매 연결마다 호출(데몬 재기동 시 port/token 바뀔 수 있음).
        const info = await invoke<DaemonInfoDto>('discover_daemon')
        // host 는 일단 127.0.0.1 고정 가정(로컬 IPC).
        const ws = new WebSocket('ws://127.0.0.1:' + info.port)
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
            // Hello 수신 = 인증 성공. connect 완료.
            if ('Hello' in msg && !settled) {
              settled = true
              this.reconnectAttempt = 0
              this.setState('connected')
              // 재연결이면 구독을 resume 재전송.
              this.resubscribeAll()
              resolve()
              return
            }
            // 인증 전 Error = Auth 실패 → reject(연결은 데몬이 닫는다).
            if ('Error' in msg && !settled) {
              settled = true
              const m = (msg.Error as { message?: string })?.message ?? 'auth failed'
              reject(new Error('daemon auth failed: ' + m))
              return
            }
            this.handleEvent(msg)
          } else if (event.data instanceof ArrayBuffer) {
            this.handleBinary(event.data)
          }
        }

        ws.onerror = () => {
          if (!settled) {
            settled = true
            reject(new Error('daemon websocket error'))
          }
        }

        ws.onclose = () => {
          this.handleClose(settled)
          if (!settled) {
            settled = true
            reject(new Error('daemon websocket closed before handshake'))
          }
        }
      }
      run().catch((e) => reject(e))
    })
  }

  /** 소켓 종료 처리 — pending 전부 reject, 의도적 종료가 아니면 재연결 스케줄. */
  private handleClose(wasHandshakeSettled: boolean): void {
    // 진행 중 명령은 전부 reject(connection lost). spawn/kill 등 1회성이라 자동 재전송은
    // 중복 부작용(중복 spawn) 위험 — 호출자가 catch 후 재시도하는 게 단순·안전(설계 택일).
    const lost = new Error('connection lost')
    for (const p of this.pending.values()) p.reject(lost)
    this.pending.clear()
    // 조회 waiter 도 reject — 빈 배열 resolve 면 호출자가 "조회 성공, 0건"(에이전트 전체 삭제)으로
    // 오인한다. "조회 실패"와 "빈 목록"을 구분하기 위해 pending 명령과 동일하게 reject.
    this.rejectAllWaiters(lost)

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

  private scheduleReconnect(): void {
    if (this.reconnectTimer) return
    if (this.closedByUser) return
    // 500ms → 1s → 2s → … 최대 10s.
    const delay = Math.min(500 * 2 ** this.reconnectAttempt, 10000)
    this.reconnectAttempt += 1
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null
      if (this.closedByUser) return
      // ensureConnected 가 새 connectPromise 를 만든다. 실패하면 다시 onclose→scheduleReconnect.
      this.ensureConnected().catch(() => {
        // openSocket reject(예: discover 실패) — 소켓 onclose 가 안 왔을 수 있으니 직접 재스케줄.
        if (!this.reconnectTimer && !this.closedByUser) {
          this.setState('reconnecting')
          this.scheduleReconnect()
        }
      })
    }, delay)
  }

  /**
   * 재연결 성공 후 모든 구독 재전송. 버그 A 수정: epoch=null 을 보내면 안 된다.
   * 데몬(ws.rs:1522-1524)은 requested_epoch==Some(current_epoch) 만 일치로 보고
   * None(null)은 불일치 취급 → FromOldest 전체 replay(이미 본 프레임 중복). 그래서
   * **마지막으로 알려진 epoch(st.epoch)을 wire 로 그대로 전송**해 데몬이 Resume(tail-only)
   * 하게 한다. after_seq=lastDeliveredSeq → 데몬이 seq>lastDeliveredSeq 만 송신 →
   * 클라 가드(seq<=lastDeliveredSeq drop)와 정합(무손실·무중복). epoch·lastDeliveredSeq
   * 는 보존(리셋 금지) — epoch 가드는 stale frame 폐기를 계속 수행하고, epoch 가 실제로
   * 바뀌면 새 SubscribeAck 가 lastDeliveredSeq 를 리셋한다.
   */
  private resubscribeAll(): void {
    for (const [agentId, st] of this.subs) {
      this.sendJson({
        Subscribe: {
          agent_id: agentId,
          epoch: st.epoch ?? null,
          after_seq: st.lastDeliveredSeq >= 0 ? st.lastDeliveredSeq : null,
        },
      })
    }
  }

  // ── JSON event 처리 ───────────────────────────────────────────────────────────
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
      // request_id 없는 Error 는 전역 통지 경로 없음 — 로그만(인터페이스 한계).
      else console.warn('[DaemonClient] daemon error:', e.message)
      return
    }
    if ('SubscribeAck' in msg) {
      const a = msg.SubscribeAck as {
        agent_id: string
        current_epoch: number
        replay_from: number
        truncated: boolean
      }
      const st = this.subs.get(a.agent_id)
      if (st) {
        // 버그 B 수정: replay_from 으로 dedup 기준(lastDeliveredSeq)을 건드리지 않는다.
        // replay_from 은 "데몬이 보내는 첫 seq"(resume 시 after_seq+1)이지 "마지막으로 본
        // seq"가 아니다 — 그걸 dedup 기준으로 쓰면 첫 정상 프레임(seq==replay_from)을 버린다.
        // dedup 은 클라 high-water(lastDeliveredSeq) 기준으로만 하고 replay_from 은 정보용.
        //
        // epoch 이 바뀌면(데몬 재기동·재시작) 새 스트림 → high-water 리셋. 첫 Ack(epoch
        // undefined)은 리셋 불필요(이미 초기 -1).
        if (st.epoch !== undefined && a.current_epoch !== st.epoch) {
          st.lastDeliveredSeq = -1
        }
        st.epoch = a.current_epoch
        // truncated 면 앞부분 손실 — 향후 UI 경고 자리(현재 인터페이스 없어 로그만).
        if (a.truncated) console.warn('[DaemonClient] output truncated for', a.agent_id)
      }
      return
    }
    if ('ReplayComplete' in msg) {
      // 라이브 전환 신호 — 현재 특별 처리 불필요(seq dedup 으로 충분).
      return
    }
    if ('AgentListUpdated' in msg) {
      const agents = (msg.AgentListUpdated as { agents: AgentInfo[] }).agents
      // broadcast 구독자(트리·상태바)에게 갱신 전달 — getAgents 편승 waiter 와 별개로 항상 호출.
      for (const cb of this.agentListCbs) cb(agents)
      const waiters = this.agentListWaiters
      this.agentListWaiters = []
      for (const w of waiters) w.resolve(agents)
      return
    }
    if ('ProfileListUpdated' in msg) {
      const profiles = (msg.ProfileListUpdated as { profiles: AgentProfile[] }).profiles
      const waiters = this.profileListWaiters
      this.profileListWaiters = []
      for (const w of waiters) w.resolve(profiles)
      return
    }
    if ('Snapshot' in msg) {
      const s = msg.Snapshot as { agent_id: string; chunks: unknown[] }
      const arr = this.snapshotWaiters.get(s.agent_id)
      if (arr && arr.length > 0) {
        const w = arr.shift()!
        if (arr.length === 0) this.snapshotWaiters.delete(s.agent_id)
        w.resolve(s.chunks)
      }
      return
    }
    if ('StatusChanged' in msg) {
      // wire 필드명: agent_id/status/epoch → cb 시그니처 (id, status, epoch).
      const s = msg.StatusChanged as { agent_id: string; status: AgentStatus; epoch: number }
      for (const cb of this.statusCbs) cb(s.agent_id, s.status, s.epoch)
      return
    }
    if ('RestoreResult' in msg) {
      const r = (msg.RestoreResult as { report: RestoreReport }).report
      for (const cb of this.restoreCbs) cb(r)
      return
    }
    // InputLeaseChanged/Output 등은 이벤트 버스 배선 전까지 여기서 소비하지 않는다(별건). 무시.
  }

  private handleBinary(buf: ArrayBuffer): void {
    const f = decodeOutputFrame(buf)
    if (!f) return
    const st = this.subs.get(f.agentId)
    if (!st) return
    // epoch 불일치 frame 은 옛 세션 잔여 — 버린다(SubscribeAck.current_epoch 기준).
    if (st.epoch !== undefined && f.epoch !== st.epoch) return
    // dedup — 클라가 실제 배달한 high-water(lastDeliveredSeq) 기준. 재연결 경계 중복 방어.
    if (f.seq <= st.lastDeliveredSeq) return
    st.lastDeliveredSeq = f.seq
    st.onChunk({ seq: f.seq, bytes: f.payload })
  }

  // ── request_id pending 헬퍼 ──────────────────────────────────────────────────
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

  /** list/snapshot 조회 waiter 를 전부 reject + 비움(끊김/close 공용). */
  private rejectAllWaiters(err: unknown): void {
    for (const w of this.agentListWaiters) w.reject(err)
    this.agentListWaiters = []
    for (const w of this.profileListWaiters) w.reject(err)
    this.profileListWaiters = []
    for (const arr of this.snapshotWaiters.values()) for (const w of arr) w.reject(err)
    this.snapshotWaiters.clear()
  }

  /** side-effect 명령 전송 + request_id 등록 → 응답(Ack/Created/Spawned/Error)으로 resolve. */
  private async sendCommand<T>(build: (requestId: string) => unknown): Promise<T> {
    await this.ensureConnected()
    const requestId = crypto.randomUUID()
    return new Promise<T>((resolve, reject) => {
      this.pending.set(requestId, { resolve: resolve as (v: unknown) => void, reject })
      try {
        this.sendJson(build(requestId))
      } catch (e) {
        this.pending.delete(requestId)
        reject(e)
      }
    })
  }

  private sendJson(payload: unknown): void {
    const ws = this.ws
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      throw new Error('daemon not connected')
    }
    ws.send(JSON.stringify(payload))
  }

  // ── 출력 구독 ──────────────────────────────────────────────────────────────────
  async subscribeOutput(
    agentId: string,
    onChunk: (chunk: OutputChunk) => void,
  ): Promise<OutputSubscription> {
    await this.ensureConnected()
    // 같은 agentId 재구독 시 이전 상태는 덮는다(컴포넌트가 epoch 바뀌면 재구독).
    // epoch=undefined(Ack 전), lastDeliveredSeq=-1(아무것도 배달 안 함).
    this.subs.set(agentId, { onChunk, epoch: undefined, lastDeliveredSeq: -1 })
    // 첫 구독 — 둘 다 null(FromOldest, 전부 받음).
    this.sendJson({ Subscribe: { agent_id: agentId, epoch: null, after_seq: null } })
    return {
      unsubscribe: () => {
        this.subs.delete(agentId)
        // 소켓 살아있을 때만 Unsubscribe 전송(끊겼으면 데몬측 구독도 이미 정리됨).
        if (this.ws && this.ws.readyState === WebSocket.OPEN) {
          try {
            this.sendJson({ Unsubscribe: { agent_id: agentId } })
          } catch {
            // 전송 실패는 무시 — 재연결 시 subs 에 없으므로 재구독 안 함.
          }
        }
      },
    }
  }

  // ── 명령(인터페이스 → wire) ──────────────────────────────────────────────────────
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
    // Resize 는 protocol 에 request_id 없음 → Ack 안 옴. fire-and-forget(전송만 하고 resolve).
    await this.ensureConnected()
    this.sendJson({ Resize: { agent_id: agentId, cols, rows, viewport_id: null } })
  }
  // v1 한계: 응답(AgentListUpdated)에 request_id 가 없어 broadcast 편승 매칭을 한다 — 다음
  // 도착하는 1건으로 resolve. 동시 호출/타 연결 트리거(다른 클라가 CRUD 유발) 시 오매칭 가능.
  // 전체 목록이라 값 자체는 정확하나, 정석 수정은 protocol 에 request_id 동봉 응답 variant 추가(추적 후속).
  async getAgents(): Promise<AgentInfo[]> {
    await this.ensureConnected()
    // ListAgents = unit variant → JSON 문자열 "ListAgents".
    return new Promise<AgentInfo[]>((resolve, reject) => {
      this.agentListWaiters.push({ resolve, reject })
      this.sendJson('ListAgents')
    })
  }
  // v1 한계: 응답(Snapshot)에 request_id 가 없어 agent_id 로만 매칭한다 — 같은 agent_id 에 대한
  // 동시 호출/타 연결 트리거 시 오매칭 가능. 정석 수정은 protocol 에 request_id 동봉 응답 variant 추가(추적 후속).
  async getSnapshot(agentId: string): Promise<unknown[]> {
    await this.ensureConnected()
    return new Promise<unknown[]>((resolve, reject) => {
      const w: Waiter<unknown[]> = { resolve, reject }
      const arr = this.snapshotWaiters.get(agentId)
      if (arr) arr.push(w)
      else this.snapshotWaiters.set(agentId, [w])
      this.sendCommandFireAndRegister(agentId)
    })
  }
  /** GetSnapshot 은 request_id 동봉이나 응답 Snapshot 엔 request_id 없음 → agent_id 로 매칭(위 waiter). */
  private sendCommandFireAndRegister(agentId: string): void {
    // request_id 는 보내되(프로토콜 요구) 응답 매칭은 agent_id waiter 로. Ack/Error 가 따로
    // 오지 않으므로 pending 엔 등록하지 않는다(Snapshot 만 응답).
    this.sendJson({ GetSnapshot: { agent_id: agentId, request_id: crypto.randomUUID() } })
  }

  // ── 프로필 CRUD ────────────────────────────────────────────────────────────────
  // v1 한계: 응답(ProfileListUpdated)에 request_id 가 없어 broadcast 편승 매칭을 한다 — 다음
  // 도착하는 1건으로 resolve. 동시 호출/타 연결 트리거 시 오매칭 가능. 전체 목록이라 값은 정확하나,
  // 정석 수정은 protocol 에 request_id 동봉 응답 variant 추가(추적 후속).
  async listProfiles(): Promise<AgentProfile[]> {
    await this.ensureConnected()
    return new Promise<AgentProfile[]>((resolve, reject) => {
      this.profileListWaiters.push({ resolve, reject })
      this.sendJson('ListProfiles')
    })
  }
  createClaudeProfile(
    name: string,
    cwd: string,
    extraArgs: string[],
    env: [string, string][],
    autoRestore: boolean,
  ): Promise<AgentProfile> {
    return this.sendCommand<AgentProfile>((request_id) => ({
      CreateProfile: {
        name,
        cwd,
        extra_args: extraArgs,
        env,
        auto_restore: autoRestore,
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

  // ── 상태/목록/복원 이벤트 — WS 이벤트 라우팅(레지스트리 등록 + remove disposer) ──────
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

  // ── 명시 종료(재연결 중단) ──────────────────────────────────────────────────────
  close(): void {
    this.closedByUser = true
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
    // in-flight 정리 — pending 명령/조회 waiter 를 reject 하지 않으면 promise leak.
    // cleanupSocket 이 onclose 핸들러를 delete 하므로 handleClose 가 안 불릴 수 있다 → 여기서 직접 정리.
    const closed = new Error('client closed')
    for (const p of this.pending.values()) p.reject(closed)
    this.pending.clear()
    this.rejectAllWaiters(closed)
    this.cleanupSocket()
    this.setState('down')
  }

  /** #13133: 핸들러는 null 대입이 아니라 delete 로 정리한 뒤 close. */
  private cleanupSocket(): void {
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
    this.connectPromise = null
  }
}
