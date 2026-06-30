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
  output(agentId: string, epoch: number, seq: number, bytes = new Uint8Array([seq & 0xff])): void {
    this.deliver({ kind: 'output', agentId, epoch, seq, bytes })
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
