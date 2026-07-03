// ProtocolClient — AgentClient 의 carrier-무관 구현 (ADR-0020 결정3, TRD Stage 3).
//
// 프로토콜 의미론(request_id 매칭 · seq high-water dedup · epoch 가드 · resubscribe resume ·
// on* 이벤트 라우팅)을 **한 곳**에 모은다. carrier(전송)는 Transport 가 추상화한다 —
// WsTransport(WS+재연결) / InProcTransport(invoke+Channel, 항상 connected). 이 클래스는
// DaemonClient(580줄)에서 carrier-무관 로직만 승격한 것이고, WS-특정(openSocket/Auth/Hello/
// scheduleReconnect/ws.send·onmessage/binary frame 디코드)은 WsTransport 로 분리됐다.
//
// ★InProc 무해 수렴★: dedup·resubscribe·epoch 가드는 InProc 에서도 그대로 돈다 — 재연결이
// 없어 connectionState 가 connected 에서 안 바뀌고(resubscribe 첫 1회는 subs 비어 무해),
// Channel 은 순서 보존이라 dedup 이 항상 통과한다. `if (inproc) skip` 우회 분기 없음(ADR-0020).

import type {
  AgentClient,
  ConnectionState,
  OutputChunk,
  OutputSubscription,
} from './agentClient'
import type { InboundMessage, Transport } from './transport'
import type {
  AgentInfo,
  AgentProfile,
  AgentStatus,
  ClaudeOutputFormat,
  RestoreReport,
} from './types'

// ── 내부 구독 상태(DaemonClient.SubState 승격) ──────────────────────────────────────
interface SubState {
  onChunk: (chunk: OutputChunk) => void
  /**
   * 마지막 SubscribeAck.current_epoch. output frame epoch 매칭용(불일치 frame 폐기) +
   * 재연결 resubscribe wire epoch. undefined = 아직 Ack 못 받음(첫 구독 직후).
   */
  epoch: number | undefined
  /**
   * onChunk 로 **실제 배달한** 최고 seq(high-water). 초기 -1(아무것도 배달 안 함).
   * dedup 기준이자 재연결 after_seq. replay_from 에 의존하지 않는다(replay_from 은
   * "데몬이 보내는 첫 seq"이지 "마지막으로 본 seq"가 아니라 off-by-one 유발 — 버그 B).
   */
  lastDeliveredSeq: number
  /**
   * stale-unsubscribe 가드용 고유 번호표(subscribeOutput 마다 ++subSeq). 반환한 unsubscribe 는
   * "현재 subs 엔트리가 내 token 일 때만 delete" 로 가드한다 — 재구독(epoch 교체/StrictMode)으로
   * 새 SubState 가 들어온 뒤 늦게 온 옛 unsubscribe 가 산 구독을 지우는 걸 막는다.
   */
  token: number
}

interface Pending {
  resolve: (v: unknown) => void
  reject: (e: unknown) => void
}

/**
 * ★구독 전 도착 프레임 버퍼(ADR-0043 deliverable 게이트의 프론트 등가물)★.
 *
 * 리로드 시 창 출력 Channel 재등록(→ Rust replay flush)이 React 컴포넌트 마운트보다 *먼저* 온다 —
 * 그때 이 agentId 에 등록된 구독자(subs 엔트리)가 아직 없어 handleOutput 이 프레임을 버리면, 그 창의
 * 슬롯은 리로드 후 빈 화면이 된다(TerminalSlot·RichSlot 공통 레이스). 구독자가 없을 때 프레임을 여기
 * 보류했다가, 첫 subscribeOutput 이 오면 seq 순서대로 flush 한다("Channel 등록된 뒤 replay flush"의 프론트
 * 등가 — Rust 는 창 단위 게이트라 개별 React 슬롯 마운트를 못 안다).
 *
 * ★epoch 인지★: epoch 가 바뀌면(재시작·데몬 재기동) 옛 스트림 잔여 프레임은 새 세션에 무의미 → 버퍼를
 *   비운다(handleOutput 의 epoch 리셋). ★bound★: agent 당 대략 2MB 넘으면 가장 오래된 프레임부터 버린다
 *   (drop-oldest) — 구독 없이 무한 성장 방지. 앞부분 손실은 어차피 리로드 후 xterm 이 전체 replay 를 새로
 *   받는 상황이라 치명적이지 않다(bound 초과는 비정상적으로 큰 버퍼 = 구독자가 영영 안 붙는 경우).
 */
interface PendingBuffer {
  /**
   * 이 버퍼가 담고 있는 프레임들의 epoch. 다른 epoch 프레임이 오면 통째 교체(stale 폐기). bufferPending
   * 내부 구분용일 뿐 — flush 시 st.epoch 로 심지 않는다(epoch 권위 = SubscribeAck 단독, ADR-0007).
   */
  epoch: number
  /**
   * 보류된 프레임(도착 순). flush 는 seq 오름차순으로 정렬 후 배달(out-of-order 도착 방어 — FIX 2).
   * tag(0 터미널/1 구조화)를 보존한다 — flush 시 handleOutput 과 동일하게 onChunk 로 실어 보내야
   * 소비자가 렌더 경로를 가른다(tag0/tag1 은 같은 seq 공간이라 버퍼는 tag 를 몰라도 담고 흘리기만).
   */
  frames: Array<{ tag: number; seq: number; bytes: Uint8Array }>
  /** 현재 담긴 바이트 총량(bound 판정용). drop-oldest 마다 감산. */
  bytes: number
  /** 이 agent 에 대해 bound 초과 warn 을 이미 냈는지(한 번만 경고 — 로그 스팸 방지). */
  warned: boolean
}

/** agent 당 pre-subscribe 버퍼 상한(대략) — 이 이상 쌓이면 가장 오래된 프레임부터 버린다. */
const PENDING_BUFFER_MAX_BYTES = 2 * 1024 * 1024

type WireEvent = Record<string, unknown>

export class ProtocolClient implements AgentClient {
  private readonly transport: Transport

  // 조회(getAgents/listProfiles/getSnapshot)와 side-effect(spawn/kill 등) 응답을 request_id 로
  // 매칭하는 단일 pending map. 조회도 전용 reply variant(AgentList/ProfileList/Snapshot)가
  // request_id 를 echo 하므로 편승 매칭 없이 정확히 짝지어진다(protocol v2).
  private pending = new Map<string, Pending>()
  private subs = new Map<string, SubState>()
  // ★구독 전 도착 프레임 버퍼(ADR-0043 프론트 등가)★: agentId → PendingBuffer. handleOutput 이 구독자
  //   없는 프레임을 여기 보류하고, subscribeOutput 이 첫 구독 시 flush 한다. transport 리셋/disconnect·
  //   close 에서 비운다(stale 방지).
  private pendingBuffers = new Map<string, PendingBuffer>()
  // 구독마다 발급하는 단조증가 번호표 — stale-unsubscribe 가드(SubState.token). 인스턴스 내 비교만
  // 하므로 UUID 불필요, 정수로 충분.
  private subSeq = 0

  // 상태/목록/복원/프로필 이벤트 콜백 레지스트리(broadcast). eventBus 가 소비.
  private agentListCbs = new Set<(agents: AgentInfo[]) => void>()
  private statusCbs = new Set<(id: string, status: AgentStatus, epoch: number) => void>()
  private restoreCbs = new Set<(report: RestoreReport) => void>()
  private profileListCbs = new Set<(profiles: AgentProfile[]) => void>()

  // transport 구독 해제 핸들.
  private offMessage: (() => void) | null = null
  private offState: (() => void) | null = null

  // resubscribe 재진입 가드 — connected 전이 중복 통지 시 1회만 의미 있게 동작(idempotent 지만 명료히).
  private lastState: ConnectionState

  constructor(transport: Transport) {
    this.transport = transport
    this.lastState = transport.connectionState
    // 단일 수신 라우터 등록 — control/output 정규화 메시지를 carrier 무관하게 라우팅.
    this.offMessage = transport.onMessage((msg) => this.route(msg))
    // 연결 상태가 connected 로 (재)전이하면 resubscribeAll. carrier 별 재연결 메커니즘은
    // transport 내부에 숨고, ProtocolClient 는 "연결됨" 신호만 본다(WS 재연결 = 이 경로로 resume).
    this.offState = transport.onConnectionStateChange((s) => {
      const prev = this.lastState
      this.lastState = s
      // down/reconnecting → connected 재전이에서만 resubscribe(첫 connected 도 포함되나 subs 비어 무해).
      if (s === 'connected' && prev !== 'connected') {
        this.resubscribeAll()
      }
      // connected → 비connected 전이 = 연결 끊김. 진행 중 명령은 전부 reject(connection lost).
      // spawn/kill 등 1회성이라 자동 재전송은 중복 부작용 위험 — 호출자가 catch 후 재시도가
      // 단순·안전(DaemonClient.handleClose 의 pending reject 를 carrier-무관 위치로 승격).
      // InProc 은 connected 에서 안 벗어나므로 이 경로가 안 불린다(무해).
      else if (s !== 'connected' && prev === 'connected') {
        const lost = new Error('connection lost')
        for (const p of this.pending.values()) p.reject(lost)
        this.pending.clear()
        // ★pre-subscribe 버퍼 폐기(transport 리셋/끊김)★: 연결이 끊기면 보류 프레임은 stale 이다 — 재연결
        //   후엔 데몬이 after_seq 기준 replay 를 새로 주므로, 옛 보류분을 남기면 재구독 flush 때 낡은 프레임이
        //   섞인다. 끊김 경계에서 비운다(끊긴 창은 어차피 재등록 시 전체 replay 를 새로 받음).
        this.pendingBuffers.clear()
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
  /** 명시 spawn 연결 — transport.start 위임. 부팅/daemon_start 만 호출(명령 경로와 분리). */
  connect(): Promise<void> {
    return this.transport.start()
  }
  /** 명시 연결 해제 — transport.close 위임(재연결 중단). ProtocolClient 구조는 유지(재연결 가능). */
  disconnect(): void {
    this.transport.close()
  }

  // ── 수신 라우팅(정규화 메시지) ───────────────────────────────────────────────────
  private route(msg: InboundMessage): void {
    if (msg.kind === 'output') {
      this.handleOutput(msg)
      return
    }
    this.handleEvent(msg.event)
  }

  /**
   * 정규화 output frame — epoch 가드 + high-water dedup 후 구독자 배달(DaemonClient.handleBinary 승격).
   *
   * ★tag 무관 공통 규율★: epoch 가드·seq dedup·high-water 전진은 tag(0 터미널/1 구조화)를 안 본다 —
   *   tag0/tag1 은 core OutputCore 의 **같은 seq 공간**을 공유하므로(한 pump 가 발급) tag 별로 쪼개면
   *   dedup high-water 가 두 벌이 돼 순서 보장이 깨진다. tag 는 배달 시 onChunk 에 실어 소비자(TerminalSlot/
   *   RichSlot)가 렌더 경로만 가른다.
   */
  private handleOutput(f: {
    tag: number
    agentId: string
    epoch: number
    seq: number
    bytes: Uint8Array
  }): void {
    const st = this.subs.get(f.agentId)
    if (!st) {
      // ★구독자 없음 = 보류(ADR-0043 deliverable 게이트의 프론트 등가)★: 리로드 시 창 Channel 재등록으로
      //   온 replay 프레임이 React 슬롯 마운트(subscribeOutput)보다 먼저 도착한 경우다. 그냥 버리면 그 창
      //   슬롯이 빈 화면이 되므로(옛 결함), 첫 구독자가 붙을 때까지 버퍼에 담는다. epoch 인지 + bound.
      this.bufferPending(f)
      return
    }
    // epoch 불일치 frame 은 옛 세션 잔여 — 버린다(SubscribeAck.current_epoch 기준).
    if (st.epoch !== undefined && f.epoch !== st.epoch) return
    // dedup — 클라가 실제 배달한 high-water(lastDeliveredSeq) 기준. 재연결 경계 중복 방어.
    // InProc(순서 보존)에선 항상 seq>high-water 라 무해 통과(no-op 수렴).
    if (f.seq <= st.lastDeliveredSeq) return
    st.lastDeliveredSeq = f.seq
    st.onChunk({ tag: f.tag, seq: f.seq, bytes: f.bytes })
  }

  /**
   * 구독자 없는 프레임을 pre-subscribe 버퍼에 보류한다(handleOutput 의 st 미존재 분기).
   *
   * ★epoch 인지★: 담긴 버퍼와 다른 epoch 프레임이 오면 옛 스트림은 무의미 → 통째 교체(stale 폐기). 재시작
   *   (epoch++)로 옛 replay 와 새 replay 가 섞여 순서가 꼬이는 걸 막는다. ★bound★: 총량이 상한을 넘으면 가장
   *   오래된 프레임부터 버린다(drop-oldest, agent 당 warn 1회). 앞부분 손실은 flush 대상이 어차피 리로드 후
   *   전체 replay 를 새로 받는 xterm 이라 치명적 아님(비정상적으로 큰 버퍼 = 구독자 영영 미부착).
   */
  private bufferPending(f: {
    tag: number
    agentId: string
    epoch: number
    seq: number
    bytes: Uint8Array
  }): void {
    let buf = this.pendingBuffers.get(f.agentId)
    if (!buf || buf.epoch !== f.epoch) {
      // 첫 프레임이거나 epoch 교체 — 새 버퍼로 시작(옛 epoch 잔여는 폐기).
      buf = { epoch: f.epoch, frames: [], bytes: 0, warned: false }
      this.pendingBuffers.set(f.agentId, buf)
    }
    buf.frames.push({ tag: f.tag, seq: f.seq, bytes: f.bytes })
    buf.bytes += f.bytes.length
    // bound 초과 → drop-oldest. flush 는 seq dedup 이 걸리므로 앞부분을 버려도 뒤 프레임 순서는 유지된다.
    if (buf.bytes > PENDING_BUFFER_MAX_BYTES) {
      if (!buf.warned) {
        console.warn(
          `[ProtocolClient] pre-subscribe 버퍼 상한 초과(agent=${f.agentId}) — 오래된 프레임 drop`,
        )
        buf.warned = true
      }
      while (buf.bytes > PENDING_BUFFER_MAX_BYTES && buf.frames.length > 1) {
        const dropped = buf.frames.shift()!
        buf.bytes -= dropped.bytes.length
      }
      // 마지막 1개만 남았는데도 상한 초과 = 단일 프레임이 bound 보다 크다. 그것도 버려 버퍼를 비운다
      //   (bound = 메모리 보호가 목적 — 한 프레임이라도 무한 잔존하면 상한이 무의미). 앞부분 손실은
      //   치명적 아님: 다음 마운트는 데몬 fresh replay 로 전체를 새로 받는다.
      if (buf.bytes > PENDING_BUFFER_MAX_BYTES && buf.frames.length === 1) {
        buf.frames.length = 0
        buf.bytes = 0
      }
    }
  }

  // ── JSON control event 처리(DaemonClient.handleEvent 승격) ────────────────────────
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
      else console.warn('[ProtocolClient] backend error:', e.message)
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
        //
        // epoch 이 바뀌면(데몬 재기동·재시작) 새 스트림 → high-water 리셋. 첫 Ack(epoch
        // undefined)은 리셋 불필요(이미 초기 -1).
        if (st.epoch !== undefined && a.current_epoch !== st.epoch) {
          st.lastDeliveredSeq = -1
        }
        st.epoch = a.current_epoch
        // truncated 면 앞부분 손실 — 향후 UI 경고 자리(현재 인터페이스 없어 로그만).
        if (a.truncated) console.warn('[ProtocolClient] output truncated for', a.agent_id)
      }
      return
    }
    if ('ReplayComplete' in msg) {
      // 라이브 전환 신호 — 현재 특별 처리 불필요(seq dedup 으로 충분).
      return
    }
    if ('AgentList' in msg) {
      // ListAgents 전용 reply(request_id echo) — getAgents 호출과 정확히 매칭(편승 매칭 제거).
      const a = msg.AgentList as { request_id: string; agents: AgentInfo[] }
      this.resolvePending(a.request_id, a.agents)
      return
    }
    if ('AgentListUpdated' in msg) {
      // broadcast — 트리·상태바 실시간 갱신 전용(request_id 없음). 조회 응답이 아니므로 pending
      // 과 무관하게 항상 콜백만 호출(두 경로 공존: 조회=AgentList / 갱신=AgentListUpdated).
      const agents = (msg.AgentListUpdated as { agents: AgentInfo[] }).agents
      for (const cb of this.agentListCbs) cb(agents)
      return
    }
    if ('ProfileList' in msg) {
      // ListProfiles 전용 reply(request_id echo) — listProfiles 호출과 매칭.
      const p = msg.ProfileList as { request_id: string; profiles: AgentProfile[] }
      this.resolvePending(p.request_id, p.profiles)
      return
    }
    if ('ProfileListUpdated' in msg) {
      // broadcast — 프로필 미러 라이브 갱신(깡통/예약, ADR-0018 후속). request_id 없음 → 콜백만.
      const profiles = (msg.ProfileListUpdated as { profiles: AgentProfile[] }).profiles
      for (const cb of this.profileListCbs) cb(profiles)
      return
    }
    if ('Snapshot' in msg) {
      // GetSnapshot 전용 reply(request_id echo) — getSnapshot 호출과 매칭(agent_id 편승 제거).
      const s = msg.Snapshot as { request_id: string; agent_id: string; chunks: unknown[] }
      this.resolvePending(s.request_id, s.chunks)
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
    // Hello/InputLeaseChanged 등은 여기서 소비하지 않는다(Hello=transport handshake 내부 소비). 무시.
  }

  // ── resubscribe(재연결 resume, DaemonClient.resubscribeAll 승격) ─────────────────
  /**
   * connected 재전이 후 모든 구독 재전송. 버그 A 수정: epoch=null 을 보내면 안 된다.
   * 데몬은 requested_epoch==Some(current_epoch) 만 일치로 보고 None(null)은 불일치 취급 →
   * FromOldest 전체 replay(이미 본 프레임 중복). 그래서 **마지막으로 알려진 epoch(st.epoch)을
   * wire 로 그대로 전송**해 데몬이 Resume(tail-only) 하게 한다. after_seq=lastDeliveredSeq →
   * 데몬이 seq>lastDeliveredSeq 만 송신 → 클라 가드(seq<=lastDeliveredSeq drop)와 정합. epoch·
   * lastDeliveredSeq 는 보존(리셋 금지). InProc 은 재연결이 없어 이 경로가 사실상 안 불린다(무해).
   */
  private resubscribeAll(): void {
    for (const [agentId, st] of this.subs) {
      this.transport.send({
        Subscribe: {
          agent_id: agentId,
          epoch: st.epoch ?? null,
          after_seq: st.lastDeliveredSeq >= 0 ? st.lastDeliveredSeq : null,
        },
      })
    }
  }

  // ── request_id pending 헬퍼(DaemonClient 승격) ───────────────────────────────────
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

  /** side-effect 명령 전송 + request_id 등록 → 응답(Ack/Created/Spawned/Error)으로 resolve. */
  private async sendCommand<T>(build: (requestId: string) => unknown): Promise<T> {
    await this.transport.ensureReady()
    const requestId = crypto.randomUUID()
    return new Promise<T>((resolve, reject) => {
      this.pending.set(requestId, { resolve: resolve as (v: unknown) => void, reject })
      try {
        // send 가 동기 throw(미연결 등)면 즉시 정리. async send 의 거부는 transport 내부 정책.
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

  // ── 출력 구독(DaemonClient.subscribeOutput 승격, transport.send 로 일반화) ───────────
  async subscribeOutput(
    agentId: string,
    onChunk: (chunk: OutputChunk) => void,
  ): Promise<OutputSubscription> {
    await this.transport.ensureReady()
    // 같은 agentId 재구독 시 이전 상태는 덮는다(컴포넌트가 epoch 바뀌면 재구독).
    // epoch=undefined(Ack 전), lastDeliveredSeq=-1(아무것도 배달 안 함).
    // token 은 이 구독의 고유 번호표 — 아래 unsubscribe 가 stale 여부를 이걸로 판별한다.
    const token = ++this.subSeq
    const st: SubState = { onChunk, epoch: undefined, lastDeliveredSeq: -1, token }
    this.subs.set(agentId, st)
    // ★pre-subscribe 버퍼 flush(ADR-0043 프론트 등가)★: 구독 전 도착해 보류된 프레임을 이 첫 구독자에게
    //   seq 순서대로 배달한다 — 라이브 프레임보다 *먼저*(리로드 replay 를 놓치지 않게). handleOutput 과 같은
    //   dedup 규율(seq<=lastDeliveredSeq drop)을 적용하고 high-water 를 전진시켜, flush 뒤 도착하는 라이브
    //   프레임과 이음매 없이 이어진다. flush 후 버퍼는 제거(1회성 — 재구독은 새로 채워짐).
    const buf = this.pendingBuffers.get(agentId)
    if (buf) {
      this.pendingBuffers.delete(agentId)
      // ★epoch 조기 확정 금지(ADR-0007)★: 버퍼 epoch 를 st.epoch 로 심지 않는다 — epoch 권위는
      //   SubscribeAck(데몬) 단독이다. 버퍼는 관측치일 뿐이라, 멀티 재시작+리로드 창에서 데몬 권위와
      //   어긋나면(SubscribeAck 이 다른 epoch) high-water 리셋이 이미 렌더한 프레임과 어긋난다. st.epoch
      //   는 undefined(= "아직 모름")로 두고, 첫 SubscribeAck 이 권위적으로 확정하게 한다. flush 중
      //   epoch 가드(st.epoch undefined)는 자연 통과.
      // ★seq 오름차순 정렬 후 flush(out-of-order 방어)★: 프레임은 도착 순으로 버퍼링되나, 도착 순서가
      //   seq 순서와 다를 수 있다(예: [2,0,1]). 배열 순서대로 flush 하면 2 를 배달해 high-water 를 2 로
      //   올린 뒤 0,1 이 dedup(seq<=high-water)으로 탈락한다. seq 오름차순으로 정렬해 이를 막는다.
      //   같은 seq 는 dedup(seq<=lastDeliveredSeq)이 자연 제거한다.
      const ordered = [...buf.frames].sort((a, b) => a.seq - b.seq)
      for (const frame of ordered) {
        if (frame.seq <= st.lastDeliveredSeq) continue
        st.lastDeliveredSeq = frame.seq
        // handleOutput 과 동일하게 tag 를 실어 배달 — flush 프레임도 소비자가 렌더 경로를 가른다.
        st.onChunk({ tag: frame.tag, seq: frame.seq, bytes: frame.bytes })
      }
    }
    // ★데몬 Subscribe 를 여기서 보내지 않는다(ADR-0035/0037 — BLOCK-1)★: 데몬 구독/재구독 소유는
    //   src-tauri 단독이다. 프론트가 `Subscribe{after_seq:null}`(FromOldest)를 데몬에 forward 하면,
    //   같은 agent 를 N 창이 보면 데몬이 FromOldest 를 N번 replay → src-tauri 공유 버퍼에 낮은 seq 가
    //   다시 append 돼 seq 단조(무손실 전제)가 붕괴한다. 데몬 구독은 layout 구독 델타(ViewManager 권위,
    //   src-tauri send_subscription_delta)가 `after_seq=버퍼 최신 seq`(축 A)로 단독 트리거한다.
    //   여기 subs(JS 콜백)는 렌더러 등록만 — output Channel 로 raw bytes 가 오면 onChunk 로 배달한다.
    return {
      unsubscribe: () => {
        // ★현재 subs 엔트리가 내 토큰일 때만 delete — 재구독(epoch 교체/StrictMode)으로 새 SubState 가
        //   들어온 뒤 늦게 온 옛 unsubscribe 가 새 구독을 지우는 stale-unsubscribe 방지.
        //   subscribeOutput 이 async(ensureReady await)라 옛 unsubscribe 가 새 set 뒤에 도착할 수 있다.
        if (this.subs.get(agentId)?.token === token) {
          this.subs.delete(agentId)
          // ★buffer refill 누수 방지★: 데몬(layout 구독 소유)은 이 창에 계속 프레임을 fan-out 하므로,
          //   subs 만 지우면 handleOutput 의 "구독자 없음" 분기가 다시 버퍼에 무한 재적재한다(언마운트 agent
          //   당 최대 2MB 상주 누수). 언마운트 시 버퍼를 비운다 — 다음 마운트는 어차피 데몬 fresh replay 로
          //   복구된다. unsubscribe 이후 도착 프레임은 pre-subscribe 케이스로 새 버퍼를 시작하나 bound 로
          //   묶여 안전하다.
          this.pendingBuffers.delete(agentId)
        }
        // ★Unsubscribe 도 데몬에 forward 안 함(BLOCK-1)★: 데몬 구독 해제도 layout 델타(1→0)가 소유한다.
        //   여기선 JS 콜백만 떼어 더는 이 agent frame 을 렌더하지 않게 한다(렌더러 역할 한정).
      },
    }
  }

  // ── 명령(인터페이스 → wire, DaemonClient 승격) ───────────────────────────────────
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
    // kill_agents 는 데몬 v1 에서 무시(always-kill, Job Object 가 자식 정리). force 만 의미 있음.
    // 데몬은 Ack 후 연결을 닫는다 — sendCommand 가 Ack 로 resolve 되고, 이후 onclose 는
    // attach-only 재연결로 가되 데몬이 죽어 못 붙어 'down' 정착(ADR-0021).
    return this.sendCommand<void>((request_id) => ({
      StopDaemon: { force, kill_agents: true, request_id },
    }))
  }

  // ── 프로필 CRUD(DaemonClient 승격) ────────────────────────────────────────────────
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
        // ADR-0044: StreamJson 이면 데몬이 json 모드 프로필로 저장 → SpawnProfile 시 StdioTransport.
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

  // ── 상태/목록/복원/프로필 이벤트 — 레지스트리 등록 + remove disposer(DaemonClient 승격) ──
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

  // ── 명시 종료 ───────────────────────────────────────────────────────────────────
  close(): void {
    // in-flight 정리 — pending 을 reject 하지 않으면 promise leak.
    const closed = new Error('client closed')
    for (const p of this.pending.values()) p.reject(closed)
    this.pending.clear()
    this.subs.clear()
    this.pendingBuffers.clear()
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
