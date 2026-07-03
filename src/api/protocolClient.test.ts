// ProtocolClient 단위테스트 — carrier-무관 프로토콜 의미론(ADR-0020 R2/R3, Stage 3).
//
// MockTransport 로 carrier 를 대체한다 — ProtocolClient 가 보내는 wire 명령을 기록하고,
// 테스트가 control/output InboundMessage 를 주입해 라우팅·dedup·epoch·resubscribe 를 검증한다.
// 실제 WS/Channel/Tauri 접속 0. WS-특정(Auth/Hello/재연결 타이밍)은 wsTransport.test 가 본다.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { ProtocolClient } from './protocolClient'
import type { ConnectionState, OutputChunk } from './agentClient'
import type { InboundMessage, Transport } from './transport'
import type { AgentInfo, AgentProfile, RestoreReport } from './types'

/**
 * 제어 가능한 Transport mock. ProtocolClient 가 send 한 wire 객체를 sent 에 기록하고,
 * deliver(...)로 수신 메시지를 ProtocolClient 로 올린다. setState 로 연결 전이를 흉내낸다.
 */
class MockTransport implements Transport {
  sent: unknown[] = []
  private _state: ConnectionState
  private stateCbs = new Set<(s: ConnectionState) => void>()
  private msgCb: ((m: InboundMessage) => void) | null = null
  ensureReadyCalls = 0
  startCalls = 0
  closed = false

  constructor(initial: ConnectionState = 'connected') {
    this._state = initial
  }

  get connectionState(): ConnectionState {
    return this._state
  }
  onConnectionStateChange(cb: (s: ConnectionState) => void): () => void {
    this.stateCbs.add(cb)
    cb(this._state)
    return () => this.stateCbs.delete(cb)
  }
  onMessage(cb: (m: InboundMessage) => void): () => void {
    this.msgCb = cb
    return () => {
      if (this.msgCb === cb) this.msgCb = null
    }
  }
  send(payload: unknown): void {
    this.sent.push(payload)
  }
  ensureReady(): Promise<void> {
    this.ensureReadyCalls += 1
    return Promise.resolve()
  }
  start(): Promise<void> {
    this.startCalls += 1
    return Promise.resolve()
  }
  close(): void {
    this.closed = true
  }

  // ── 테스트 구동 ──
  deliver(msg: InboundMessage): void {
    this.msgCb?.(msg)
  }
  control(event: Record<string, unknown>): void {
    this.deliver({ kind: 'control', event })
  }
  // tag 기본 0(터미널) — tag1 케이스는 명시 인자로. 대부분 기존 테스트는 seq/epoch/dedup 만 보므로 tag0.
  output(
    agentId: string,
    epoch: number,
    seq: number,
    bytes = new Uint8Array([seq & 0xff]),
    tag = 0,
  ): void {
    this.deliver({ kind: 'output', tag, agentId, epoch, seq, bytes })
  }
  setState(s: ConnectionState): void {
    this._state = s
    for (const cb of this.stateCbs) cb(s)
  }
  /** 마지막으로 send 된 객체 중 주어진 variant key 를 가진 것의 inner 반환. */
  lastSent<T = Record<string, unknown>>(key: string): T | undefined {
    for (let i = this.sent.length - 1; i >= 0; i--) {
      const m = this.sent[i]
      if (m && typeof m === 'object' && key in (m as object)) return (m as Record<string, T>)[key]
    }
    return undefined
  }
}

const AGENT = '12345678-9abc-def0-1234-56789abcdef0'

let uuidCounter = 0
beforeEach(() => {
  uuidCounter = 0
  vi.spyOn(globalThis.crypto, 'randomUUID').mockImplementation(
    () => `req-${++uuidCounter}` as `${string}-${string}-${string}-${string}-${string}`,
  )
})
afterEach(() => {
  vi.restoreAllMocks()
})

describe('request_id pending 매칭', () => {
  it('spawnAgent → SpawnByCwd{request_id} 전송 + Spawned{request_id,agent} resolve', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.spawnAgent('C:/work')
    await Promise.resolve() // ensureReady await 통과
    const sent = t.lastSent<{ request_id: string; cwd: string }>('SpawnByCwd')!
    expect(sent.cwd).toBe('C:/work')
    t.control({ Spawned: { request_id: sent.request_id, agent: { id: 'a1' } } })
    expect(await p).toEqual({ id: 'a1' })
  })

  it('killAgent → Ack{request_id} 로 void resolve', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.killAgent('a1')
    await Promise.resolve()
    const rid = t.lastSent<{ request_id: string }>('Kill')!.request_id
    t.control({ Ack: { request_id: rid } })
    await expect(p).resolves.toBeUndefined()
  })

  it('Error{request_id} 로 reject', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.killAgent('a1')
    await Promise.resolve()
    const rid = t.lastSent<{ request_id: string }>('Kill')!.request_id
    t.control({ Error: { request_id: rid, message: 'boom' } })
    await expect(p).rejects.toThrow('boom')
  })

  it('잘못된 request_id 의 응답은 무시(pending 유지)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.killAgent('a1')
    await Promise.resolve()
    let settled = false
    void p.then(() => (settled = true)).catch(() => (settled = true))
    t.control({ Ack: { request_id: 'nonexistent' } })
    await Promise.resolve()
    expect(settled).toBe(false)
  })

  it('조회 전용 reply 매칭 — getAgents 진행 중 broadcast AgentListUpdated 편승 안 함', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const broadcasts: AgentInfo[][] = []
    c.onAgentListUpdated((a) => broadcasts.push(a))
    const p = c.getAgents()
    await Promise.resolve()
    const rid = t.lastSent<{ request_id: string }>('ListAgents')!.request_id
    const other = [{ id: 'other' }] as unknown as AgentInfo[]
    t.control({ AgentListUpdated: { agents: other } })
    let settled = false
    void p.then(() => (settled = true)).catch(() => (settled = true))
    await Promise.resolve()
    expect(settled).toBe(false) // 편승 안 함
    expect(broadcasts).toEqual([other]) // broadcast 는 정상 라우팅
    const mine = [{ id: 'mine' }] as unknown as AgentInfo[]
    t.control({ AgentList: { request_id: rid, agents: mine } })
    await expect(p).resolves.toEqual(mine)
  })

  it('동시 2개 getSnapshot(같은 agent_id)도 request_id 로 정확 매칭', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p1 = c.getSnapshot(AGENT)
    const p2 = c.getSnapshot(AGENT)
    await Promise.resolve()
    const sent = t.sent.filter(
      (m): m is { GetSnapshot: { request_id: string } } =>
        !!m && typeof m === 'object' && 'GetSnapshot' in m,
    )
    expect(sent.length).toBe(2)
    const rid1 = sent[0].GetSnapshot.request_id
    const rid2 = sent[1].GetSnapshot.request_id
    expect(rid1).not.toBe(rid2)
    t.control({ Snapshot: { request_id: rid2, agent_id: AGENT, chunks: [{ seq: 2 }] } })
    t.control({ Snapshot: { request_id: rid1, agent_id: AGENT, chunks: [{ seq: 1 }] } })
    await expect(p1).resolves.toEqual([{ seq: 1 }])
    await expect(p2).resolves.toEqual([{ seq: 2 }])
  })
})

describe('seq dedup / epoch 가드(R2)', () => {
  async function subscribed(): Promise<{ t: MockTransport; received: number[] }> {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const received: number[] = []
    await c.subscribeOutput(AGENT, (chunk: OutputChunk) => received.push(chunk.seq))
    return { t, received }
  }

  it('같은 seq 재수신 → drop(high-water 기준)', async () => {
    const { t, received } = await subscribed()
    t.control({ SubscribeAck: { agent_id: AGENT, current_epoch: 1, replay_from: 0, truncated: false } })
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 0) // 중복
    t.output(AGENT, 1, 1)
    expect(received).toEqual([0, 1])
  })

  it('seq <= high-water drop, replay_from 은 dedup 기준 아님(버그 B 회귀)', async () => {
    const { t, received } = await subscribed()
    // replay_from=5 이지만 dedup 기준은 high-water(-1) — 첫 frame seq=0 이 버려지면 안 됨.
    t.control({ SubscribeAck: { agent_id: AGENT, current_epoch: 1, replay_from: 5, truncated: false } })
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    expect(received).toEqual([0, 1])
  })

  it('epoch 안 맞는 frame → drop(stale 세션)', async () => {
    const { t, received } = await subscribed()
    t.control({ SubscribeAck: { agent_id: AGENT, current_epoch: 5, replay_from: 0, truncated: false } })
    t.output(AGENT, 4, 0) // 옛 epoch
    t.output(AGENT, 5, 0) // 맞는 epoch
    expect(received).toEqual([0])
  })

  it('SubscribeAck.current_epoch 변경 → high-water 리셋 → 새 스트림 낮은 seq 배달(R3)', async () => {
    const { t, received } = await subscribed()
    t.control({ SubscribeAck: { agent_id: AGENT, current_epoch: 10, replay_from: 0, truncated: false } })
    t.output(AGENT, 10, 0)
    t.output(AGENT, 10, 1)
    t.output(AGENT, 10, 2)
    expect(received).toEqual([0, 1, 2])
    // epoch 11 → 리셋. 새 스트림 seq 0 이 다시 배달돼야.
    t.control({ SubscribeAck: { agent_id: AGENT, current_epoch: 11, replay_from: 0, truncated: false } })
    t.output(AGENT, 11, 0)
    t.output(AGENT, 11, 1)
    expect(received).toEqual([0, 1, 2, 0, 1])
  })

  it('SubscribeAck 전 frame(epoch undefined) → epoch 가드 통과(배달)', async () => {
    const { t, received } = await subscribed()
    t.output(AGENT, 99, 0) // Ack 전 — st.epoch undefined
    expect(received).toEqual([0])
  })
})

// ── S15/ADR-0045: tag(0 터미널/1 구조화) 전달 + tag0/tag1 혼재 dedup·epoch 회귀 ──────────────
describe('tag 전달 + tag0/tag1 혼재(S15 구조화 출력)', () => {
  async function subscribedChunks(): Promise<{ t: MockTransport; chunks: OutputChunk[] }> {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const chunks: OutputChunk[] = []
    await c.subscribeOutput(AGENT, (chunk) => chunks.push(chunk))
    return { t, chunks }
  }

  it('tag 를 onChunk 로 그대로 전달(tag0/tag1 구분)', async () => {
    const { t, chunks } = await subscribedChunks()
    t.control({ SubscribeAck: { agent_id: AGENT, current_epoch: 1, replay_from: 0, truncated: false } })
    t.output(AGENT, 1, 0, new Uint8Array([1]), 0) // tag0 터미널
    t.output(AGENT, 1, 1, new Uint8Array([2]), 1) // tag1 구조화
    expect(chunks.map((c) => c.tag)).toEqual([0, 1])
    expect(chunks.map((c) => c.seq)).toEqual([0, 1])
  })

  it('tag0/tag1 혼재도 seq dedup 은 한 seq 공간 공통(tag 무관 high-water)', async () => {
    const { t, chunks } = await subscribedChunks()
    t.control({ SubscribeAck: { agent_id: AGENT, current_epoch: 1, replay_from: 0, truncated: false } })
    t.output(AGENT, 1, 0, new Uint8Array([0]), 0) // tag0
    t.output(AGENT, 1, 1, new Uint8Array([0]), 1) // tag1
    t.output(AGENT, 1, 1, new Uint8Array([0]), 0) // seq 1 중복(다른 tag 라도 같은 seq 공간 → drop)
    t.output(AGENT, 1, 0, new Uint8Array([0]), 1) // seq 0 <= high-water → drop
    t.output(AGENT, 1, 2, new Uint8Array([0]), 1) // 신규 → 배달
    expect(chunks.map((c) => c.seq)).toEqual([0, 1, 2])
    expect(chunks.map((c) => c.tag)).toEqual([0, 1, 1])
  })

  it('tag1 도 epoch 가드 공통(옛 epoch 구조화 frame drop)', async () => {
    const { t, chunks } = await subscribedChunks()
    t.control({ SubscribeAck: { agent_id: AGENT, current_epoch: 5, replay_from: 0, truncated: false } })
    t.output(AGENT, 4, 0, new Uint8Array([0]), 1) // 옛 epoch tag1 → drop
    t.output(AGENT, 5, 0, new Uint8Array([0]), 1) // 맞는 epoch tag1 → 배달
    expect(chunks.map((c) => c.seq)).toEqual([0])
    expect(chunks[0].tag).toBe(1)
  })

  it('pre-subscribe 버퍼도 tag 보존해 flush(구독 전 도착 tag1)', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    // 구독자 없이 tag0/tag1 혼재 도착 → 버퍼.
    t.output(AGENT, 1, 0, new Uint8Array([0]), 0)
    t.output(AGENT, 1, 1, new Uint8Array([0]), 1)
    const chunks: OutputChunk[] = []
    await c.subscribeOutput(AGENT, (chunk) => chunks.push(chunk))
    // flush 시 tag 가 보존돼야(handleOutput 배달과 동형).
    expect(chunks.map((c) => c.seq)).toEqual([0, 1])
    expect(chunks.map((c) => c.tag)).toEqual([0, 1])
  })
})

describe('stale-unsubscribe 가드(owner token) — 재구독 시 산 구독 보호', () => {
  it('옛 구독의 unsubscribe 가 재구독(덮어쓴) 새 구독을 지우지 않는다', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    const got1: number[] = []
    const got2: number[] = []
    // sub1 → sub2 재구독(같은 agentId): subs 엔트리를 덮어쓴다.
    const sub1 = await c.subscribeOutput(AGENT, (chunk) => got1.push(chunk.seq))
    await c.subscribeOutput(AGENT, (chunk) => got2.push(chunk.seq))
    // stale: 옛 구독(sub1)의 unsubscribe 가 새 구독(sub2) 뒤늦게 실행 → token 가드로 무시돼야.
    sub1.unsubscribe()
    // 'A' 프레임 주입(Ack 전 → st.epoch undefined 로 epoch 가드 통과, seq>high-water(-1) → dedup 통과).
    t.output(AGENT, 0, 0)
    t.output(AGENT, 0, 1)
    expect(got2).toEqual([0, 1]) // 새 구독 생존 — 정상 배달(stale delete 가 안 지움)
    expect(got1).toEqual([]) // 옛 콜백은 덮여서 애초에 배달 대상 아님
  })

  it('정상 경로: 살아있는 구독의 unsubscribe 는 실제로 제거(그 뒤 배달 안 됨)', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    const got: number[] = []
    const sub = await c.subscribeOutput(AGENT, (chunk) => got.push(chunk.seq))
    t.output(AGENT, 0, 0)
    expect(got).toEqual([0])
    sub.unsubscribe() // 현재 엔트리 token == 내 token → 실제 delete
    t.output(AGENT, 0, 1) // 구독 제거됨 → handleOutput 이 st 없음으로 무시
    expect(got).toEqual([0])
  })
})

describe('resubscribe resume(R3) — connected 재전이', () => {
  it('재연결(reconnecting→connected) 시 알려진 epoch + after_seq=마지막배달seq 로 resubscribe', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    const received: number[] = []
    await c.subscribeOutput(AGENT, (chunk) => received.push(chunk.seq))
    const E = 5
    t.control({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false } })
    t.output(AGENT, E, 0)
    t.output(AGENT, E, 1)
    t.output(AGENT, E, 2)
    expect(received).toEqual([0, 1, 2])

    // 끊김 → 재연결.
    t.setState('reconnecting')
    t.sent = [] // resubscribe 만 관측하려 초기화
    t.setState('connected')

    // ★핵심★: resubscribe 가 epoch=E(null 아님) + after_seq=2(마지막 배달 seq).
    const resub = t.lastSent<{ agent_id: string; epoch: number | null; after_seq: number | null }>(
      'Subscribe',
    )!
    expect(resub.agent_id).toBe(AGENT)
    expect(resub.epoch).toBe(E) // 버그 A: null 이면 FromOldest 중복
    expect(resub.after_seq).toBe(2) // 버그 B: replay_from/null 이면 off-by-one

    // 데몬 Resume → seq 3. 무손실·무중복.
    t.control({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 3, truncated: false } })
    t.output(AGENT, E, 3)
    expect(received).toEqual([0, 1, 2, 3])
  })

  it('재연결 후 데몬이 이미 본 seq(0,1,2) 재전송해도 dedup', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    const received: number[] = []
    await c.subscribeOutput(AGENT, (chunk) => received.push(chunk.seq))
    const E = 2
    t.control({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false } })
    t.output(AGENT, E, 0)
    t.output(AGENT, E, 1)
    t.output(AGENT, E, 2)
    t.setState('reconnecting')
    t.setState('connected')
    t.control({ SubscribeAck: { agent_id: AGENT, current_epoch: E, replay_from: 0, truncated: false } })
    // 데몬이 0,1,2 재전송 → dedup, 3 만 새로.
    t.output(AGENT, E, 0)
    t.output(AGENT, E, 1)
    t.output(AGENT, E, 2)
    t.output(AGENT, E, 3)
    expect(received).toEqual([0, 1, 2, 3])
  })

  it('connected→reconnecting 전이 시 pending 명령 reject(connection lost)', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    const p = c.killAgent('a1')
    await Promise.resolve()
    t.setState('reconnecting')
    await expect(p).rejects.toThrow('connection lost')
  })
})

describe('이벤트 라우팅(eventBus 공통 표면)', () => {
  it('StatusChanged → (id, status, epoch) 정확히 수신', () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const calls: Array<[string, unknown, number]> = []
    const off = c.onStatusChanged((id, status, epoch) => calls.push([id, status, epoch]))
    const status = { type: 'Running' }
    t.control({ StatusChanged: { agent_id: 'agent-7', status, epoch: 3 } })
    expect(calls).toEqual([['agent-7', status, 3]])
    off()
    t.control({ StatusChanged: { agent_id: 'agent-7', status, epoch: 4 } })
    expect(calls).toEqual([['agent-7', status, 3]])
  })

  it('RestoreResult{report} → cb 가 report 수신', () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const seen: RestoreReport[] = []
    c.onRestoreResult((r) => seen.push(r))
    const report = { agent_id: 'a9', epoch: 1, outcome: { type: 'Resumed' } } as RestoreReport
    t.control({ RestoreResult: { report } })
    expect(seen).toEqual([report])
  })

  it('ProfileListUpdated → onProfileListUpdated cb 가 profiles 수신(ADR-0018 후속)', () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const seen: AgentProfile[][] = []
    const off = c.onProfileListUpdated((p) => seen.push(p))
    const profiles = [{ id: 'p1' }] as unknown as AgentProfile[]
    t.control({ ProfileListUpdated: { profiles } })
    expect(seen).toEqual([profiles])
    off()
    t.control({ ProfileListUpdated: { profiles: [{ id: 'p2' }] } })
    expect(seen).toEqual([profiles]) // unsubscribe 후 미수신
  })
})

describe('InProc no-op 수렴(항상 connected·순서보존)', () => {
  // InProc 류 carrier: 항상 connected, reconnecting 전이 없음, frame 순서 보존.
  // dedup/epoch/resubscribe 가 무해 통과하는지(우회 분기 없이 자연수렴) 검증.
  it('연결 전이 없이도 정상 출력 배달(★BLOCK-1: subscribeOutput 은 Subscribe 를 안 보낸다★)', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    const received: number[] = []
    await c.subscribeOutput(AGENT, (chunk) => received.push(chunk.seq))
    t.control({ SubscribeAck: { agent_id: AGENT, current_epoch: 0, replay_from: 0, truncated: false } })
    // 순서 보존된 frame — dedup 이 절대 막지 않아야(전부 배달).
    for (let s = 0; s < 5; s++) t.output(AGENT, 0, s)
    expect(received).toEqual([0, 1, 2, 3, 4])
    // ★BLOCK-1(데몬 구독 소유 = src-tauri 단독, ADR-0035/0037)★: subscribeOutput 첫 구독은 데몬에
    //   Subscribe 를 forward 하지 않는다(데몬 구독은 layout 델타가 단독 트리거 — 프론트가 N창에서
    //   FromOldest 를 보내면 공유 버퍼 seq 단조 붕괴). 여기 subs(JS 콜백) 등록만 하고 dedup/epoch
    //   가드는 그대로 — SubscribeAck/output 처리(위 배달)는 변함없이 동작한다. 재연결도 없으니 0개.
    const subs = t.sent.filter((m) => !!m && typeof m === 'object' && 'Subscribe' in (m as object))
    expect(subs.length).toBe(0)
  })

  // ── BLOCKER 1 회귀: embedded carrier 의 실제 wire(epoch≥1)를 재현 ──────────────────────────
  //    resume-fallback 세션은 SubscribeAck.current_epoch=1 로 온다(manager.agent_epoch). embedded
  //    output frame 도 PtyEvent.epoch=1 을 실어야 가드를 통과한다. 옛 버그는 output epoch 을 0 으로
  //    고정해 SubscribeAck epoch=1 과 불일치 → 전멸시켰다. 아래 두 케이스가 그 가드 동작을 박는다:
  it('SubscribeAck epoch=1 + output epoch=1 → 배달(전멸 안 됨)', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    const received: number[] = []
    await c.subscribeOutput(AGENT, (chunk) => received.push(chunk.seq))
    // resume-fallback 세션: SubscribeAck/output 모두 epoch 1 → 가드 일치 통과.
    t.control({ SubscribeAck: { agent_id: AGENT, current_epoch: 1, replay_from: 0, truncated: false } })
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    expect(received).toEqual([0, 1])
  })

  it('SubscribeAck epoch=1 + output epoch=0(옛 버그 재현) → epoch 가드가 전부 drop', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    const received: number[] = []
    await c.subscribeOutput(AGENT, (chunk) => received.push(chunk.seq))
    // SubscribeAck 은 실제 epoch 1, 그러나 carrier 가 epoch 을 0 으로 버린 경우(BLOCKER 1 버그).
    t.control({ SubscribeAck: { agent_id: AGENT, current_epoch: 1, replay_from: 0, truncated: false } })
    t.output(AGENT, 0, 0)
    t.output(AGENT, 0, 1)
    // 0 !== 1 → epoch 가드가 전부 버린다. 이게 실버그의 증상 — carrier 가 epoch 을 0 으로 버리면
    // 출력이 화면에 0건 도달한다(이 단언이 가드 동작과 버그 메커니즘을 동시에 박제).
    expect(received).toEqual([])
  })

  it('epoch 0 output + epoch 0 SubscribeAck 정합(fresh 세션 epoch=0)', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    const received: number[] = []
    await c.subscribeOutput(AGENT, (chunk) => received.push(chunk.seq))
    // fresh 세션(epoch 0): SubscribeAck/output 모두 epoch 0 → 가드 통과.
    t.control({ SubscribeAck: { agent_id: AGENT, current_epoch: 0, replay_from: 0, truncated: false } })
    t.output(AGENT, 0, 0)
    t.output(AGENT, 0, 1)
    expect(received).toEqual([0, 1])
  })
})

// ── ADR-0043 프론트 등가: 구독 전 도착 프레임 버퍼 → 첫 구독 flush ──────────────────────
//    리로드 시 창 Channel 재등록(→ Rust replay flush)이 React 슬롯 마운트(subscribeOutput)보다
//    먼저 온다. 구독자 없는 프레임을 버렸던 옛 결함(RichSlot 빈 화면)을 버퍼링으로 메운다.
describe('pre-subscribe 버퍼(ADR-0043 deliverable 게이트 프론트 등가)', () => {
  it('구독 전 도착 프레임 → 버퍼 → 첫 구독 시 seq 순서대로 flush', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    // 구독자 없는 상태로 replay 프레임 3개 도착(리로드 시 Channel 재등록이 마운트보다 먼저 온 상황).
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.output(AGENT, 1, 2)
    // 이제 React 슬롯이 마운트되며 구독 — 보류분이 순서대로 flush 돼야.
    const received: number[] = []
    await c.subscribeOutput(AGENT, (chunk) => received.push(chunk.seq))
    expect(received).toEqual([0, 1, 2])
  })

  it('flush 후 라이브 프레임과 이음매 없이 이어짐(high-water 전진 dedup)', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    const received: number[] = []
    await c.subscribeOutput(AGENT, (chunk) => received.push(chunk.seq))
    expect(received).toEqual([0, 1]) // flush
    // flush 로 high-water=1 전진 → 라이브 seq 1 재수신은 dedup, seq 2 는 배달.
    t.output(AGENT, 1, 1)
    t.output(AGENT, 1, 2)
    expect(received).toEqual([0, 1, 2])
  })

  it('epoch 교체 → 옛 버퍼 폐기(새 epoch 프레임만 flush)', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    // 옛 epoch 1 프레임 보류 → 재시작으로 epoch 2 프레임 도착(옛 스트림 무의미 → 통째 교체 by bufferPending).
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.output(AGENT, 2, 0)
    t.output(AGENT, 2, 1)
    const received: number[] = []
    await c.subscribeOutput(AGENT, (chunk) => received.push(chunk.seq))
    // 옛 epoch 1 은 버려지고 epoch 2 프레임만 flush(seq 0,1) — 버퍼 교체가 옛 epoch 잔여를 이미 폐기.
    expect(received).toEqual([0, 1])
    // ★FIX 4(ADR-0007)★: flush 는 st.epoch 를 심지 않는다(epoch 권위 = SubscribeAck 단독). 그래서 지금
    //   st.epoch 는 undefined — SubscribeAck 이 오기 전 라이브 프레임은 epoch 가드를 통과한다(seq dedup 만).
    //   SubscribeAck 이 데몬 권위로 epoch 2 를 확정한 뒤에야 epoch 1 프레임이 stale 로 걸러진다.
    t.control({ SubscribeAck: { agent_id: AGENT, current_epoch: 2, replay_from: 0, truncated: false } })
    t.output(AGENT, 1, 5) // SubscribeAck 후 stale epoch → drop
    t.output(AGENT, 2, 2)
    expect(received).toEqual([0, 1, 2])
  })

  it('버퍼 상한 초과 → 오래된 프레임 drop + warn 1회', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    // 1MB 프레임 3개(총 3MB > 2MB 상한) — 구독자 없이 도착. drop-oldest 로 앞 프레임(seq 0)이 밀려남.
    const oneMB = new Uint8Array(1024 * 1024)
    t.deliver({ kind: 'output', tag: 0, agentId: AGENT, epoch: 1, seq: 0, bytes: oneMB })
    t.deliver({ kind: 'output', tag: 0, agentId: AGENT, epoch: 1, seq: 1, bytes: oneMB })
    t.deliver({ kind: 'output', tag: 0, agentId: AGENT, epoch: 1, seq: 2, bytes: oneMB })
    const received: number[] = []
    await c.subscribeOutput(AGENT, (chunk) => received.push(chunk.seq))
    // seq 0 은 drop-oldest 로 버려짐 → 뒤쪽만 flush(정확한 잔존 개수는 상한 산술에 달렸으나 seq 0 은 없어야).
    expect(received).not.toContain(0)
    expect(received.length).toBeGreaterThan(0)
    // warn 은 이 agent 에 대해 정확히 1회(로그 스팸 방지).
    const bufferWarns = warnSpy.mock.calls.filter(
      (call) => typeof call[0] === 'string' && call[0].includes('pre-subscribe 버퍼 상한 초과'),
    )
    expect(bufferWarns.length).toBe(1)
    warnSpy.mockRestore()
  })

  it('연결 끊김(connected→비connected) → 버퍼 폐기(재연결 후 낡은 프레임 미flush)', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    // 끊김 — 보류분은 stale(재연결 후 데몬이 replay 새로 줌) → 폐기돼야.
    t.setState('reconnecting')
    t.setState('connected')
    const received: number[] = []
    await c.subscribeOutput(AGENT, (chunk) => received.push(chunk.seq))
    // 폐기됐으므로 구독 시 flush 될 게 없음(빈 배열). 이후 라이브만 배달.
    expect(received).toEqual([])
    t.output(AGENT, 1, 2)
    expect(received).toEqual([2])
  })

  it('구독 후 도착 프레임은 버퍼 안 거치고 즉시 배달(회귀 — 기존 경로 불변)', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    const received: number[] = []
    await c.subscribeOutput(AGENT, (chunk) => received.push(chunk.seq))
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    expect(received).toEqual([0, 1])
  })

  // ── FIX 2: out-of-order 도착 프레임 → seq 오름차순 flush ──────────────────────────────
  it('out-of-order 도착[2,0,1] → seq 순서(0,1,2)로 flush(high-water drop 방지)', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    // 도착 순서가 seq 순서와 다름 — 정렬 없이 배열 순서 flush 하면 2 를 먼저 배달해 high-water=2 →
    //   0,1 이 dedup 탈락한다. FIX 2 는 seq 오름차순 정렬 후 flush 해 전부 배달한다.
    t.output(AGENT, 1, 2)
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    const received: number[] = []
    await c.subscribeOutput(AGENT, (chunk) => received.push(chunk.seq))
    expect(received).toEqual([0, 1, 2])
  })

  // ── FIX 3: 단일 초대형 프레임(> bound) → 버퍼 비움 ──────────────────────────────────
  it('단일 3MB 프레임(> 2MB 상한) → 버퍼 비움 + warn 1회(bound 무한잔존 방지)', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    // 단일 프레임이 상한보다 큼 — drop-oldest 루프는 frames.length>1 에서 멈추므로 옛 코드는 이 1개를
    //   영영 남긴다. FIX 3 은 마지막 1개도 bound 초과면 버려 버퍼를 비운다.
    const threeMB = new Uint8Array(3 * 1024 * 1024)
    t.deliver({ kind: 'output', tag: 0, agentId: AGENT, epoch: 1, seq: 0, bytes: threeMB })
    const received: number[] = []
    await c.subscribeOutput(AGENT, (chunk) => received.push(chunk.seq))
    // 버퍼가 비었으므로 flush 될 게 없다.
    expect(received).toEqual([])
    const bufferWarns = warnSpy.mock.calls.filter(
      (call) => typeof call[0] === 'string' && call[0].includes('pre-subscribe 버퍼 상한 초과'),
    )
    expect(bufferWarns.length).toBe(1)
    warnSpy.mockRestore()
  })

  // ── FIX 1: unsubscribe → 버퍼 폐기(refill 누수 방지) ─────────────────────────────────
  it('subscribe→unsubscribe→프레임 도착→remount → post-unsubscribe 프레임만 flush(stale 없음)', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    const got1: number[] = []
    const sub = await c.subscribeOutput(AGENT, (chunk) => got1.push(chunk.seq))
    t.output(AGENT, 1, 0) // 구독 중 배달
    expect(got1).toEqual([0])
    // 언마운트 — subs 삭제 + 버퍼 폐기. 이후 데몬이 계속 보내는 프레임은 새 pre-subscribe 버퍼로 감.
    sub.unsubscribe()
    t.output(AGENT, 1, 1) // unsubscribe 후 도착 — 새 버퍼 시작(옛 stale 아님)
    t.output(AGENT, 1, 2)
    // remount(재구독) — 새 버퍼(seq 1,2)만 flush. 옛 seq 0 은 이미 배달됐고 버퍼에 남지 않아 중복 없음.
    const got2: number[] = []
    await c.subscribeOutput(AGENT, (chunk) => got2.push(chunk.seq))
    expect(got2).toEqual([1, 2])
  })
})

describe('close', () => {
  it('close() → pending reject + transport.close 호출', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.killAgent('a1')
    await Promise.resolve()
    c.close()
    await expect(p).rejects.toThrow('client closed')
    expect(t.closed).toBe(true)
  })
})

// ── ADR-0021: connect(명시 spawn)/disconnect(재연결 중단) 위임 ────────────────────────
describe('connect/disconnect (ADR-0021 §1·note3)', () => {
  it('connect() → transport.start 위임(명시 spawn, ensureReady 와 분리)', async () => {
    const t = new MockTransport('down')
    const c = new ProtocolClient(t)
    await c.connect()
    expect(t.startCalls).toBe(1)
    // 명령 경로(ensureReady)와 달리 spawn 은 start 만 — connect 가 ensureReady 를 부르지 않는다.
    expect(t.ensureReadyCalls).toBe(0)
  })

  it('disconnect() → transport.close 위임(ProtocolClient 구조는 유지, close 와 다름)', () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    c.disconnect()
    expect(t.closed).toBe(true)
  })
})
