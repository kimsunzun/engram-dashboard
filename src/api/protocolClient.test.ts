// ProtocolClient 단위테스트 — carrier-무관 프로토콜 의미론 + 뷰 직결 replay 상태기계(ADR-0046).
//
// MockTransport 로 carrier 를 대체한다 — ProtocolClient 가 보내는 wire 명령을 기록하고, 테스트가
// control/output/replayBoundary InboundMessage 를 주입해 라우팅·dedup·epoch·gen 펜스·전이표를 검증한다.
// 실제 WS/Channel/Tauri 접속 0. WS-특정(Auth/Hello/재연결 타이밍)은 wsTransport.test 가 본다.
//
// ★TRD §5 시나리오 고정★: 엇갈린 mount(남의 마커 무시→자기 gen flush) · 같은 agent 2뷰 fan-out+dedup
//   (버그 B 회귀) · live frame 이 replay 보다 먼저 와도 sort+dedup 복원 · 마커 token 불일치 무시(StrictMode)
//   · 마커가 myGen 보다 먼저 도착(held→flush) · epoch 회전 중 buffering(폐기+재요청·구 epoch 마커 무시)
//   · 재연결 중 buffering(폐기+재요청) · 실패 마커→사다리→3회 후 error · watchdog 재요청(fake timers)
//   · 뷰별 dedup 독립 · unsubscribe 청소.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { ProtocolClient } from './protocolClient'
import type { ConnectionState, OutputChunk } from './agentClient'
import type { InboundMessage, Transport } from './transport'
import type { AgentInfo, AgentProfile, Preset, RestoreReport } from './types'

/**
 * 제어 가능한 Transport mock. ProtocolClient 가 send 한 wire 객체를 sent 에 기록하고, deliver(...)로
 * 수신 메시지를 올린다. requestReplay 는 per-agent gen 을 부여(replayCalls 기록)하고, replayGenImpl 로
 * 회수 타이밍(지연·즉시)을 제어한다.
 */
class MockTransport implements Transport {
  sent: unknown[] = []
  private _state: ConnectionState
  private stateCbs = new Set<(s: ConnectionState) => void>()
  private msgCb: ((m: InboundMessage) => void) | null = null
  ensureReadyCalls = 0
  startCalls = 0
  closed = false
  // requestReplay(agentId) 호출 기록 — 부여한 gen 순서로.
  replayCalls: Array<{ agentId: string; gen: bigint }> = []
  private replayGenCounter = 0n
  /**
   * requestReplay 반환 제어. null 이면 즉시 resolve(부여 gen). 함수를 심으면 그 반환 Promise 를 쓴다 —
   * myGen 확정 지연(마커 먼저 도착) 재현. gen 은 항상 replayCalls 에 기록된다(호출 사실은 즉시).
   */
  replayGenImpl: ((agentId: string, gen: bigint) => Promise<bigint>) | null = null

  ensureReadyImpl: (() => Promise<void>) | null = null

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
    if (this.ensureReadyImpl) return this.ensureReadyImpl()
    return Promise.resolve()
  }
  start(): Promise<void> {
    this.startCalls += 1
    return Promise.resolve()
  }
  close(): void {
    this.closed = true
  }
  requestReplay(agentId: string): Promise<bigint> {
    const gen = ++this.replayGenCounter
    this.replayCalls.push({ agentId, gen })
    if (this.replayGenImpl) return this.replayGenImpl(agentId, gen)
    return Promise.resolve(gen)
  }

  // ── 테스트 구동 ──
  deliver(msg: InboundMessage): void {
    this.msgCb?.(msg)
  }
  control(event: Record<string, unknown>): void {
    this.deliver({ kind: 'control', event })
  }
  output(agentId: string, epoch: number, seq: number, bytes = new Uint8Array([seq & 0xff]), tag = 0): void {
    this.deliver({ kind: 'output', tag, agentId, epoch, seq, bytes })
  }
  // replay 경계 마커 주입(성공 기본). failed/truncated 는 명시 인자.
  marker(
    agentId: string,
    epoch: number,
    gen: bigint,
    opts: { failed?: boolean; truncated?: boolean } = {},
  ): void {
    this.deliver({
      kind: 'replayBoundary',
      agentId,
      epoch,
      gen,
      truncated: opts.truncated ?? false,
      failed: opts.failed ?? false,
    })
  }
  setState(s: ConnectionState): void {
    this._state = s
    for (const cb of this.stateCbs) cb(s)
  }
  lastSent<T = Record<string, unknown>>(key: string): T | undefined {
    for (let i = this.sent.length - 1; i >= 0; i--) {
      const m = this.sent[i]
      if (m && typeof m === 'object' && key in (m as object)) return (m as Record<string, T>)[key]
    }
    return undefined
  }
}

const AGENT = '12345678-9abc-def0-1234-56789abcdef0'
const V1 = 'view-1'
const V2 = 'view-2'

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

// ── request_id pending 매칭(carrier 무관 — ADR-0046 무영향) ─────────────────────────────
describe('request_id pending 매칭', () => {
  it('spawnAgent → SpawnByCwd{request_id} 전송 + Spawned{request_id,agent} resolve', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.spawnAgent('C:/work')
    await Promise.resolve()
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
    t.control({ Snapshot: { request_id: rid2, agent_id: AGENT, chunks: [{ seq: 2 }] } })
    t.control({ Snapshot: { request_id: rid1, agent_id: AGENT, chunks: [{ seq: 1 }] } })
    await expect(p1).resolves.toEqual([{ seq: 1 }])
    await expect(p2).resolves.toEqual([{ seq: 2 }])
  })
})

// ── 프리셋 CRUD(ADR-0061 — wire 명령/reply 매칭) ─────────────────────────────────────
describe('프리셋 CRUD(ADR-0061)', () => {
  it('listPresets → ListPresets{request_id} 전송 + PresetList{request_id,presets} resolve', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.listPresets()
    await Promise.resolve()
    const rid = t.lastSent<{ request_id: string }>('ListPresets')!.request_id
    const presets = [{ id: 'pr1', cwd: 'C:/proj' }] as Preset[]
    t.control({ PresetList: { request_id: rid, presets } })
    expect(await p).toEqual(presets)
  })

  it('createPreset(cwd) → CreatePreset{cwd,request_id} 전송 + Ack 로 void resolve', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.createPreset('C:/work')
    await Promise.resolve()
    const sent = t.lastSent<{ request_id: string; cwd: string }>('CreatePreset')!
    expect(sent.cwd).toBe('C:/work')
    t.control({ Ack: { request_id: sent.request_id } })
    await expect(p).resolves.toBeUndefined()
  })

  it('deletePreset(id) → DeletePreset{preset_id,request_id} 전송 + Ack 로 void resolve', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.deletePreset('pr1')
    await Promise.resolve()
    const sent = t.lastSent<{ request_id: string; preset_id: string }>('DeletePreset')!
    expect(sent.preset_id).toBe('pr1')
    t.control({ Ack: { request_id: sent.request_id } })
    await expect(p).resolves.toBeUndefined()
  })
})

// ── subscribeOutput 기본 배선(뷰 단위, ADR-0046) ─────────────────────────────────────
describe('subscribeOutput 기본(뷰 단위 replay)', () => {
  it('subscribe → requestReplay 발행(뷰당 1회) + wire Subscribe 는 안 보낸다(BLOCK-1)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    await c.subscribeOutput(V1, AGENT, () => {})
    // 뷰가 requestReplay 를 정확히 1회 발행(gen 부여).
    expect(t.replayCalls.map((r) => r.agentId)).toEqual([AGENT])
    // ★BLOCK-1★: 프론트는 wire Subscribe 를 어떤 경로로도 안 보낸다(request_replay 가 carrier 내부에서 냄).
    const subs = t.sent.filter((m) => !!m && typeof m === 'object' && 'Subscribe' in (m as object))
    expect(subs.length).toBe(0)
  })

  it('성공 마커 전엔 buffering(직행 배달 없음), 마커 후 live 전환+flush', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    const gen = t.replayCalls[0].gen
    // buffering — 프레임은 축적만(직행 배달 안 함).
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    expect(got).toEqual([])
    // 성공 마커(gen 일치, epoch 1) → sort+dedup flush → live.
    t.marker(AGENT, 1, gen)
    expect(got).toEqual([0, 1])
    // 이후 라이브 프레임은 직행 배달.
    t.output(AGENT, 1, 2)
    expect(got).toEqual([0, 1, 2])
  })

  it('tag 를 onChunk 로 그대로 전달(tag0/tag1) + 한 seq 공간 dedup', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const chunks: OutputChunk[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => chunks.push(chunk))
    const gen = t.replayCalls[0].gen
    t.output(AGENT, 1, 0, new Uint8Array([1]), 0) // tag0
    t.output(AGENT, 1, 1, new Uint8Array([2]), 1) // tag1
    t.marker(AGENT, 1, gen)
    // live 이후 dedup(한 seq 공간, tag 무관).
    t.output(AGENT, 1, 1, new Uint8Array([2]), 0) // seq 1 중복 → drop
    t.output(AGENT, 1, 2, new Uint8Array([3]), 1)
    expect(chunks.map((x) => x.seq)).toEqual([0, 1, 2])
    expect(chunks.map((x) => x.tag)).toEqual([0, 1, 1])
  })
})

// ── 엇갈린 mount(진행 중 replay 꼬리만 받은 뷰가 남의 마커 무시 → 자기 gen 마커에 완전 flush) ──────
describe('엇갈린 mount — 남의 마커 무시, 자기 gen 마커에 완전 flush(gen 펜스)', () => {
  it('먼저 mount 한 뷰의 replay 꼬리 + 남의 마커(gen<myGen)는 무시, 자기 gen 마커에 전량 flush', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    // V1 이 먼저 mount(gen=1). V1 replay 진행 중.
    const g1got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => g1got.push(chunk.seq))
    const gen1 = t.replayCalls[0].gen
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    // V2 가 mid-replay 로 늦게 mount(gen=2). V1 replay 꼬리(seq 2)를 V2 도 fan-out 으로 받는다.
    const g2got: number[] = []
    await c.subscribeOutput(V2, AGENT, (chunk) => g2got.push(chunk.seq))
    const gen2 = t.replayCalls[1].gen
    t.output(AGENT, 1, 2) // V1 replay 꼬리 = V2 버퍼 머리
    // V1 replay 종결 마커(gen1) — V1 은 자기 gen 이라 flush, V2 는 남의(gen1<gen2) 마커라 무시.
    t.marker(AGENT, 1, gen1)
    expect(g1got).toEqual([0, 1, 2])
    expect(g2got).toEqual([]) // V2 는 아직 buffering(자기 gen2 마커 안 옴)
    // V2 자기 replay 전체(single-flight 병합 후 전량 재replay) → seq 0,1,2 재전송 + 종결 gen2 마커.
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.output(AGENT, 1, 2)
    t.marker(AGENT, 1, gen2)
    // V2 는 자기 gen 마커에 sort+dedup flush = 완전(0,1,2). V1 은 이미 live 라 마커 무시.
    expect(g2got).toEqual([0, 1, 2])
    expect(g1got).toEqual([0, 1, 2])
  })
})

// ── 같은 agent 2뷰 독립 fan-out + dedup(버그 B 회귀) ──────────────────────────────────
describe('같은 agent 2뷰 독립 fan-out + 뷰별 dedup(버그 B 회귀)', () => {
  it('두 뷰가 각자 독립 진도로 전량 수신(한 뷰가 다른 뷰 진도를 오염 안 함)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const g1: number[] = []
    const g2: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => g1.push(chunk.seq))
    const gen1 = t.replayCalls[0].gen
    await c.subscribeOutput(V2, AGENT, (chunk) => g2.push(chunk.seq))
    const gen2 = t.replayCalls[1].gen
    // 두 뷰 모두 buffering — 프레임 fan-out.
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    // 각 뷰가 자기 gen 마커에 flush(독립).
    t.marker(AGENT, 1, gen1)
    t.marker(AGENT, 1, gen2)
    expect(g1).toEqual([0, 1])
    expect(g2).toEqual([0, 1])
    // 이후 라이브 프레임 — 두 뷰 모두 각자 dedup·독립 배달.
    t.output(AGENT, 1, 2)
    expect(g1).toEqual([0, 1, 2])
    expect(g2).toEqual([0, 1, 2])
  })

  it('뷰별 dedup 독립 — 한 뷰가 live 여도 다른 뷰의 dedup high-water 와 무관', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const g1: number[] = []
    const g2: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => g1.push(chunk.seq))
    const gen1 = t.replayCalls[0].gen
    // V1 만 먼저 live.
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, gen1)
    expect(g1).toEqual([0])
    // V2 가 나중에 mount — 자기 buffering 에서 seq 0 부터 다시 받는다(V1 high-water 와 독립).
    await c.subscribeOutput(V2, AGENT, (chunk) => g2.push(chunk.seq))
    const gen2 = t.replayCalls[1].gen
    t.output(AGENT, 1, 0) // V1 은 dedup drop, V2 는 buffer 축적
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, gen2)
    expect(g2).toEqual([0, 1]) // V2 독립 진도로 전량
    expect(g1).toEqual([0, 1]) // V1 은 live 라 seq 1 만 새로(0 은 dedup)
  })
})

// ── live frame 이 replay(마커)보다 먼저 와도 sort+dedup 복원 ────────────────────────────
describe('out-of-order 프레임 sort+dedup(순서 복원)', () => {
  it('버퍼에 out-of-order[2,0,1] 도착 → 마커 flush 시 seq 순서(0,1,2)로 배달', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    const gen = t.replayCalls[0].gen
    // 도착 순서가 seq 순서와 다름 — 정렬 없이 배열 순서 flush 하면 2 를 먼저 배달해 0,1 dedup 탈락.
    t.output(AGENT, 1, 2)
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, gen)
    expect(got).toEqual([0, 1, 2])
  })
})

// ── 마커 token 불일치 무시(StrictMode 사망 구독) ──────────────────────────────────────
describe('마커 token 불일치 무시(StrictMode 재구독)', () => {
  it('재구독으로 교체된 옛 구독은 마커를 소비하지 않는다(생존 구독만 flush)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const g1: number[] = []
    const g2: number[] = []
    // 같은 viewId 로 급속 재구독(StrictMode) — 두 번째가 생존(subs 엔트리 교체).
    const p1 = c.subscribeOutput(V1, AGENT, (chunk) => g1.push(chunk.seq))
    const p2 = c.subscribeOutput(V1, AGENT, (chunk) => g2.push(chunk.seq))
    await Promise.all([p1, p2])
    // 생존 구독(두 번째)만 requestReplay 를 냈다(옛 st 는 token 불일치라 skip).
    expect(t.replayCalls.length).toBe(1)
    const gen = t.replayCalls[0].gen
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, gen)
    // 생존 구독만 flush — 옛 콜백(g1)은 subs 에서 교체돼 fan-out 대상 아님.
    expect(g2).toEqual([0])
    expect(g1).toEqual([])
  })
})

// ── 마커가 myGen 확정보다 먼저 도착(held → 재평가 flush) — NEW-3 ─────────────────────────
describe('마커가 myGen 확정보다 먼저 도착(held → flush)', () => {
  it('requestReplay 회수 지연 중 마커 도착 → 보관 후 myGen 확정 시 flush', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    // requestReplay 회수를 게이트로 지연 — 마커가 myGen 보다 먼저 오는 파이프 교차 재현.
    let releaseGen!: (gen: bigint) => void
    t.replayGenImpl = () => new Promise<bigint>((r) => (releaseGen = r))
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    const gen = t.replayCalls[0].gen
    // 마커·프레임이 myGen 확정 전에 도착 — 마커는 held, 프레임은 buffer.
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, gen)
    expect(got).toEqual([]) // 아직 myGen 미확정 — held.
    // myGen 확정 → held 재평가 → flush.
    releaseGen(gen)
    await Promise.resolve()
    await Promise.resolve()
    expect(got).toEqual([0, 1])
  })

  it('held 는 최고 gen 1개만 보관 — 낮은 gen 마커가 높은 걸 덮지 않는다', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    let releaseGen!: (gen: bigint) => void
    t.replayGenImpl = () => new Promise<bigint>((r) => (releaseGen = r))
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    const myGen = t.replayCalls[0].gen
    t.output(AGENT, 1, 0)
    // 남의(낮은 gen) 마커가 먼저, 그 뒤 자기(높은 gen) 마커 — held 는 최고 gen 유지.
    t.marker(AGENT, 1, myGen - 1n)
    t.marker(AGENT, 1, myGen)
    releaseGen(myGen)
    await Promise.resolve()
    await Promise.resolve()
    expect(got).toEqual([0]) // 최고 gen(=myGen) held 로 flush
  })

  // ── FIX-3: 같은 gen failed→success 교체(좀비 late-Complete 복구) ──────────────────────
  it('같은 gen 의 held failed 마커를 뒤이은 성공 마커가 교체 → myGen 확정 시 flush(사다리 아님)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    let releaseGen!: (gen: bigint) => void
    t.replayGenImpl = () => new Promise<bigint>((r) => (releaseGen = r))
    const got: number[] = []
    const states: string[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq), (s) => states.push(s))
    const myGen = t.replayCalls[0].gen
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    // myGen 미확정 중: 같은 gen 의 실패 마커(deadline) 먼저 → held. 이어 같은 gen 의 성공 마커(늦은
    //   Complete) → FIX-3 교체 규칙(같은 gen && held.failed && !m.failed)으로 성공이 실패를 밀어낸다.
    t.marker(AGENT, 1, myGen, { failed: true })
    t.marker(AGENT, 1, myGen, { failed: false })
    // myGen 확정 → held(성공) 재평가 → flush(live). 실패가 이겼으면 사다리로 빠져 buffering 유지·got 비어야.
    releaseGen(myGen)
    await Promise.resolve()
    await Promise.resolve()
    expect(got).toEqual([0, 1]) // 성공 마커로 flush
    expect(c.getViewOutputState(V1)?.phase).toBe('live')
    expect(states).toContain('live')
  })
})

// ── epoch 회전 중 buffering(폐기+재요청, 구 epoch 마커 무시) — NEW-5 ───────────────────────
describe('epoch 회전 중 buffering(폐기 + 재요청, 구 epoch 마커 무시)', () => {
  it('더 높은 epoch frame 도착 → buffer 폐기 + 재요청(새 myGen), 구 epoch 마커는 무시', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    const gen1 = t.replayCalls[0].gen
    // epoch 1 프레임 buffering.
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    // epoch 2 프레임 도착(재시작) → buffer 폐기 + 재요청(gen2). requestReplay 회수는 microtask 라
    //   myGen 확정을 위해 한 틱 양보한다(마커 held→재평가 대신 정상 gen 비교 경로 검증).
    t.output(AGENT, 2, 0)
    expect(t.replayCalls.length).toBe(2) // 재요청 발행
    const gen2 = t.replayCalls[1].gen
    await Promise.resolve() // myGen=gen2 확정
    // 구 epoch(1) 대상 gen1 마커는 이제 무효 — epoch 불일치로 무시.
    t.marker(AGENT, 1, gen1)
    expect(got).toEqual([])
    // 새 epoch 2 replay 전량 + gen2 마커 → flush.
    t.output(AGENT, 2, 1)
    t.marker(AGENT, 2, gen2)
    expect(got).toEqual([0, 1]) // epoch 2 프레임만
  })
})

// ── 재연결 중 buffering(폐기 + 재요청) ─────────────────────────────────────────────────
describe('재연결(connected 재전이) → 모든 뷰 buffering 리셋 + 재요청', () => {
  it('live 뷰도 재연결 시 buffering 리셋 후 재요청·재flush', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    const gen1 = t.replayCalls[0].gen
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, gen1)
    expect(got).toEqual([0])
    // 끊김 → 재연결.
    t.setState('reconnecting')
    t.setState('connected')
    // 재연결이 재요청 발행(전량 재replay). requestReplay 회수(microtask) 후 myGen 확정.
    expect(t.replayCalls.length).toBe(2)
    const gen2 = t.replayCalls[1].gen
    await Promise.resolve() // myGen=gen2 확정
    // buffering 리셋됐으므로 재연결 후 프레임은 축적 후 마커에 flush(dedup 로 seq 0 은 이미 배달).
    t.output(AGENT, 1, 0) // seq 0 <= high-water → dedup drop
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, gen2)
    expect(got).toEqual([0, 1])
  })

  it('connected→비connected 전이 시 pending 명령 reject(connection lost)', async () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    const p = c.killAgent('a1')
    await Promise.resolve()
    t.setState('reconnecting')
    await expect(p).rejects.toThrow('connection lost')
  })
})

// ── 실패 마커 → 사다리 → 3회 후 error ─────────────────────────────────────────────────
describe('실패 마커 → 재요청 사다리 → 상한(3) 도달 시 error', () => {
  it('실패 마커마다 백오프 재요청, 3회 소진 후 error 상태 + onState 통지', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      const states: string[] = []
      await c.subscribeOutput(V1, AGENT, () => {}, (s) => states.push(s))
      // 초기 발행 1회.
      expect(t.replayCalls.length).toBe(1)
      let gen = t.replayCalls[t.replayCalls.length - 1].gen
      // 실패 마커 → 사다리 1단계(백오프 1s 뒤 재요청).
      t.marker(AGENT, 1, gen, { failed: true })
      await vi.advanceTimersByTimeAsync(1000)
      expect(t.replayCalls.length).toBe(2) // 재요청 1
      gen = t.replayCalls[t.replayCalls.length - 1].gen
      // 실패 마커 → 사다리 2단계(2s).
      t.marker(AGENT, 1, gen, { failed: true })
      await vi.advanceTimersByTimeAsync(2000)
      expect(t.replayCalls.length).toBe(3) // 재요청 2
      gen = t.replayCalls[t.replayCalls.length - 1].gen
      // 실패 마커 → 사다리 3단계(4s).
      t.marker(AGENT, 1, gen, { failed: true })
      await vi.advanceTimersByTimeAsync(4000)
      expect(t.replayCalls.length).toBe(4) // 재요청 3(상한)
      gen = t.replayCalls[t.replayCalls.length - 1].gen
      // 4번째 실패 마커 → 상한 소진 → error(재요청 없음).
      t.marker(AGENT, 1, gen, { failed: true })
      await vi.advanceTimersByTimeAsync(10000)
      expect(t.replayCalls.length).toBe(4) // 더 안 늘어남
      expect(states).toContain('error')
      // LLM 제어 표면으로 error 조회 가능.
      expect(c.getViewOutputState(V1)?.phase).toBe('error')
    } finally {
      vi.useRealTimers()
    }
  })
})

// ── FIX-2: buffer 상한 초과 → buffer 폐기 후 재요청(부분 flush 금지) ─────────────────────
describe('buffer 상한 초과 → 폐기 + 재요청(FIX-2)', () => {
  it('상한 초과로 buffer 폐기 → pre-overflow gen 성공 마커는 stale 프레임 flush 안 함, 재요청 replay 가 완전 flush', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      const got: number[] = []
      await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
      const gen1 = t.replayCalls[0].gen
      // 상한(4MB) 초과: 1MB 프레임 5개(=5MB). 초과 시 pushBuffer 가 buffer 폐기 + 사다리 재요청(백오프 1s).
      const big = new Uint8Array(1024 * 1024)
      for (let seq = 0; seq < 5; seq++) t.output(AGENT, 1, seq, big)
      // ★buffer 비워짐★: LLM 제어 표면으로 관측 — buffered=0(부분 유지 아님).
      expect(c.getViewOutputState(V1)?.buffered).toBe(0)
      // 사다리 백오프(1s) 후 재요청 발행. requestReplay 회수(microtask) 후 myGen=gen2 확정.
      await vi.advanceTimersByTimeAsync(1000)
      expect(t.replayCalls.length).toBe(2)
      const gen2 = t.replayCalls[1].gen
      // ★stale flush 금지★: pre-overflow gen(gen1) 성공 마커가 와도 폐기된 5MB 프레임을 flush 하지 않는다.
      //   (gen1 < myGen=gen2 라 gen 펜스로도 무시되지만, buffer 자체가 비어 flush 할 것도 없다.)
      t.marker(AGENT, 1, gen1)
      expect(got).toEqual([])
      // 재요청 replay 전량(작은 프레임으로) + gen2 성공 마커 → 완전 flush.
      t.output(AGENT, 1, 0, new Uint8Array([0]))
      t.output(AGENT, 1, 1, new Uint8Array([1]))
      t.output(AGENT, 1, 2, new Uint8Array([2]))
      t.marker(AGENT, 1, gen2)
      expect(got).toEqual([0, 1, 2])
    } finally {
      vi.useRealTimers()
    }
  })
})

// ── FIX-A: overflow 후 구 gen 성공 마커가 백오프 전에 와도 flush 금지(gen 펜스 무효화) ────────
describe('buffer 상한 초과 후 구 gen 성공 마커(백오프 전 도착) → flush 금지 + 재요청 유지(FIX-A)', () => {
  it('overflow 폐기 → myGen 무효화로 구 gen 성공 마커가 빈 buffer 를 flush(내용 유실) 못 함, 재요청 이어서 완전 flush', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      const got: number[] = []
      const states: string[] = []
      await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq), (s) => states.push(s))
      // myGen 은 subscribeOutput await 로 이미 gen1 확정(초기 발행 즉시 resolve).
      const gen1 = t.replayCalls[0].gen
      // 상한(4MB) 초과 → pushBuffer 가 buffer 폐기 + 사다리 재요청 예약(백오프 1s). ★FIX-A 이전★엔 이때
      //   myGen 이 gen1 로 남아, 백오프 발화 전 도착한 gen1 성공 마커가 gen 펜스를 통과해 빈 buffer 로
      //   flushToLive → live 전이(내용 유실) + clearTimers 로 예약된 재요청 취소.
      const big = new Uint8Array(1024 * 1024)
      for (let seq = 0; seq < 5; seq++) t.output(AGENT, 1, seq, big)
      expect(c.getViewOutputState(V1)?.buffered).toBe(0) // buffer 폐기됨

      // ★핵심: 백오프(1s) 발화 *전* 에 구 gen(gen1) 성공 마커 도착★. FIX-A 로 myGen 이 무효화됐으므로
      //   이 마커는 evalMarker 의 myGen===undefined 분기로 held 만 되고 flush 하지 않는다(펜스 통과 불가).
      t.marker(AGENT, 1, gen1)
      expect(got).toEqual([]) // flush 안 됨
      expect(states).not.toContain('live') // live 전이 안 됨 — 여전히 buffering
      expect(c.getViewOutputState(V1)?.phase).toBe('buffering')

      // 백오프 발화 → 재요청 발행(재요청이 취소되지 않았음을 증명). requestReplay 회수 후 myGen=gen2 확정.
      await vi.advanceTimersByTimeAsync(1000)
      expect(t.replayCalls.length).toBe(2)
      const gen2 = t.replayCalls[1].gen

      // 재요청한 replay 의 새 gen 프레임 전량 + gen2 성공 마커 → 완전 flush(내용 복원).
      t.output(AGENT, 1, 0, new Uint8Array([0]))
      t.output(AGENT, 1, 1, new Uint8Array([1]))
      t.output(AGENT, 1, 2, new Uint8Array([2]))
      t.marker(AGENT, 1, gen2)
      expect(got).toEqual([0, 1, 2])
      expect(c.getViewOutputState(V1)?.phase).toBe('live')
    } finally {
      vi.useRealTimers()
    }
  })
})

// ── FIX-6: myGen 확정 후 실패(G)→성공(G)이 백오프 전에 오면 flush + 백오프 정리 ──────────
describe('실패 마커(myGen 확정) 뒤 같은 gen 성공 마커가 백오프 전에 도착 → flush(백오프 정리)', () => {
  it('failed(G) 사다리 예약 후, 백오프 만료 전 success(G) → complete buffer flush + 백오프 타이머 정리', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      const got: number[] = []
      // myGen 즉시 확정(replayGenImpl null = 즉시 resolve). subscribeOutput await 로 확정 보장.
      await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
      const gen = t.replayCalls[0].gen
      t.output(AGENT, 1, 0)
      t.output(AGENT, 1, 1)
      // 실패 마커(gen 일치, myGen 확정) → 사다리 예약(백오프 1s). buffer 는 유지(flush 금지).
      t.marker(AGENT, 1, gen, { failed: true })
      expect(got).toEqual([])
      // ★백오프 만료 전★ 같은 gen 의 성공 마커(늦은 Complete) → flushToLive(buffer 완전 flush).
      t.marker(AGENT, 1, gen, { failed: false })
      expect(got).toEqual([0, 1])
      expect(c.getViewOutputState(V1)?.phase).toBe('live')
      // ★백오프 타이머 정리 확인★: flush 가 clearTimers 로 예약된 재요청을 취소했으므로, 시간이 흘러도
      //   stray 재요청이 없다(재요청은 초기 발행 1회에서 멈춤).
      await vi.advanceTimersByTimeAsync(5000)
      expect(t.replayCalls.length).toBe(1)
    } finally {
      vi.useRealTimers()
    }
  })
})

// ── watchdog = 재요청이지 flush 아님(fake timers) ───────────────────────────────────────
describe('watchdog 만료 → 재요청(flush 아님)', () => {
  it('성공 마커 없이 10s 경과 → buffer flush 하지 않고 재요청', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      const got: number[] = []
      await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
      expect(t.replayCalls.length).toBe(1)
      // 프레임만 버퍼링(성공 마커 안 옴).
      t.output(AGENT, 1, 0)
      t.output(AGENT, 1, 1)
      // watchdog 만료(10s) → 재요청. ★flush 아님★: got 은 여전히 비어야.
      await vi.advanceTimersByTimeAsync(10000)
      expect(got).toEqual([]) // flush 금지 확인
      // watchdog → 사다리 재요청(백오프 1s 후).
      await vi.advanceTimersByTimeAsync(1000)
      expect(t.replayCalls.length).toBe(2)
    } finally {
      vi.useRealTimers()
    }
  })
})

// ── FIX-1: 같은 viewId 재구독 시 옛 SubState 타이머 정리 ─────────────────────────────────
describe('재구독(같은 viewId) → 옛 watchdog/backoff 타이머 정리(FIX-1)', () => {
  it('buffering(watchdog 무장) 중 같은 viewId 재구독 → 옛 watchdog 만료해도 stray 재요청 없음', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      // 1) V1 최초 구독 → buffering, watchdog 무장(초기 발행 1회).
      await c.subscribeOutput(V1, AGENT, () => {})
      expect(t.replayCalls.length).toBe(1)
      // 2) 같은 viewId 재구독(새 token) → 옛 SubState 교체. 재구독 발행 1회(총 2회).
      await c.subscribeOutput(V1, AGENT, () => {})
      expect(t.replayCalls.length).toBe(2)
      // 3) 옛 watchdog(10s)이 살아있으면 만료 시 옛 st 로 ladderRerequest 예약 → 백오프(1s) 후 stray 재요청.
      //    FIX-1: 재구독이 옛 타이머를 clear 했으면 그런 재요청이 없어야 한다.
      await vi.advanceTimersByTimeAsync(11000)
      // 생존 구독(새 st)의 watchdog 은 정상 동작 = 재요청 1회(총 3회). 옛 watchdog 이 추가로 발화하면 4회 이상.
      expect(t.replayCalls.length).toBe(3)
    } finally {
      vi.useRealTimers()
    }
  })
})

// ── unsubscribe 청소(타이머 정리 + fan-out 중단) ────────────────────────────────────────
describe('unsubscribe 청소', () => {
  it('unsubscribe 후 프레임/마커가 그 뷰로 안 감(subs 제거) + 타이머 정리', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      const got: number[] = []
      const sub = await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
      const gen = t.replayCalls[0].gen
      t.output(AGENT, 1, 0)
      t.marker(AGENT, 1, gen)
      expect(got).toEqual([0])
      // unsubscribe — subs 제거 + 타이머 정리. 이후 프레임은 이 뷰로 안 온다.
      sub.unsubscribe()
      t.output(AGENT, 1, 1)
      expect(got).toEqual([0])
      expect(c.getViewOutputState(V1)).toBeNull()
      // 대기 중 타이머가 있었어도(정리됐으므로) 추가 재요청 없음.
      const before = t.replayCalls.length
      await vi.advanceTimersByTimeAsync(20000)
      expect(t.replayCalls.length).toBe(before)
    } finally {
      vi.useRealTimers()
    }
  })

  it('stale unsubscribe(재구독 뒤 늦게 온 옛 unsubscribe)는 산 구독을 안 지운다', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const g1: number[] = []
    const g2: number[] = []
    const sub1 = await c.subscribeOutput(V1, AGENT, (chunk) => g1.push(chunk.seq))
    await c.subscribeOutput(V1, AGENT, (chunk) => g2.push(chunk.seq)) // 같은 viewId 재구독 → 교체
    const gen = t.replayCalls[t.replayCalls.length - 1].gen
    sub1.unsubscribe() // stale — token 불일치라 무시돼야
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, gen)
    expect(g2).toEqual([0]) // 생존 구독 정상
    expect(g1).toEqual([])
  })
})

// ── epoch 가드(live) ──────────────────────────────────────────────────────────────────
describe('epoch 가드(live)', () => {
  it('live 뷰에 더 높은 epoch frame → drop([agentId,epoch] remount 흐름이 처리)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: Array<[number, number]> = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push([chunk.seq, chunk.bytes[0]]))
    const gen = t.replayCalls[0].gen
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, gen) // epoch 1 채택, live
    t.output(AGENT, 2, 1) // 더 높은 epoch → drop(remount 가 재구독)
    t.output(AGENT, 1, 1) // 같은 epoch → 배달
    expect(got.map((x) => x[0])).toEqual([0, 1])
  })
})

// ── FIX: ensureReady reject → 좀비 구독 롤백 ────────────────────────────────────────────
describe('ensureReady reject → 좀비 구독 롤백', () => {
  it('ensureReady reject → subs 에 좀비 안 남음 + rethrow', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    t.ensureReadyImpl = () => Promise.reject(new Error('daemon down'))
    const got: number[] = []
    await expect(c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))).rejects.toThrow(
      'daemon down',
    )
    // 좀비 없음 — 이후 프레임이 죽은 구독으로 안 샌다.
    t.output(AGENT, 1, 0)
    expect(got).toEqual([])
    expect(c.getViewOutputState(V1)).toBeNull()
  })
})

// ── 이벤트 라우팅(eventBus 공통 표면) ──────────────────────────────────────────────────
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

  it('ProfileListUpdated → onProfileListUpdated cb 가 profiles 수신', () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const seen: AgentProfile[][] = []
    const off = c.onProfileListUpdated((p) => seen.push(p))
    const profiles = [{ id: 'p1' }] as unknown as AgentProfile[]
    t.control({ ProfileListUpdated: { profiles } })
    expect(seen).toEqual([profiles])
    off()
    t.control({ ProfileListUpdated: { profiles: [{ id: 'p2' }] } })
    expect(seen).toEqual([profiles])
  })

  it('PresetListUpdated → onPresetListUpdated cb 가 presets 수신(ADR-0061)', () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const seen: Preset[][] = []
    const off = c.onPresetListUpdated((p) => seen.push(p))
    const presets = [{ id: 'pr1', cwd: 'C:/proj' }] as Preset[]
    t.control({ PresetListUpdated: { presets } })
    expect(seen).toEqual([presets])
    off()
    t.control({ PresetListUpdated: { presets: [{ id: 'pr2', cwd: 'C:/x' }] } })
    expect(seen).toEqual([presets]) // 해제 후 미수신
  })

  it('getAgents 진행 중 broadcast AgentListUpdated 편승 안 함', async () => {
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
    expect(settled).toBe(false)
    expect(broadcasts).toEqual([other])
    const mine = [{ id: 'mine' }] as unknown as AgentInfo[]
    t.control({ AgentList: { request_id: rid, agents: mine } })
    await expect(p).resolves.toEqual(mine)
  })
})

// ── close ───────────────────────────────────────────────────────────────────────────
describe('close', () => {
  it('close() → pending reject + transport.close 호출 + 타이머 정리', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      await c.subscribeOutput(V1, AGENT, () => {})
      const p = c.killAgent('a1')
      await Promise.resolve()
      c.close()
      await expect(p).rejects.toThrow('client closed')
      expect(t.closed).toBe(true)
      // close 후 타이머 없음(watchdog 정리) — 재요청 안 늘어남.
      const before = t.replayCalls.length
      await vi.advanceTimersByTimeAsync(20000)
      expect(t.replayCalls.length).toBe(before)
    } finally {
      vi.useRealTimers()
    }
  })
})

// ── connect/disconnect (ADR-0021 §1·note3) ────────────────────────────────────────────
describe('connect/disconnect (ADR-0021 §1·note3)', () => {
  it('connect() → transport.start 위임(명시 spawn)', async () => {
    const t = new MockTransport('down')
    const c = new ProtocolClient(t)
    await c.connect()
    expect(t.startCalls).toBe(1)
    expect(t.ensureReadyCalls).toBe(0)
  })

  it('disconnect() → transport.close 위임', () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    c.disconnect()
    expect(t.closed).toBe(true)
  })
})
