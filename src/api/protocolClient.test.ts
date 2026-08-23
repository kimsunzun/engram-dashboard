// ProtocolClient 단위테스트 — carrier-무관 프로토콜 의미론 + 뷰 직결 replay 상태기계(ADR-0046).
//
// WS-특정(Auth/Hello/재연결 타이밍)은 wsTransport.test 가 본다.
//
// ★TRD §5 시나리오 고정★: 엇갈린 mount(남의 마커 무시→자기 gen flush) · 같은 agent 2뷰 fan-out+dedup
//   (버그 B 회귀) · live frame 이 replay 보다 먼저 와도 sort+dedup 복원 · 마커 token 불일치 무시(StrictMode)
//   · 마커가 myGen 보다 먼저 도착(held→flush) · epoch 회전 중 buffering(폐기+재요청·구 epoch 마커 무시)
//   · 끊김=detached(요청 없음)/부착 계기=명부(같은 세션 이어보기 vs 새 세션 재구성) · 실패 마커→사다리→
//   3회 후 error · watchdog 재요청(fake timers)
//   · 붙어 있는 뷰의 회전(끊긴 적 없는 재spawn) · 표식 없는 명부 항목 = 회전 취급 · 명부 부재 →
//   붙어 있던 뷰도 detached · 표식은 프레임이 아니라 성공 마커가 정한다(앞 화신 늦은 프레임 무해)
//   · 뷰별 dedup 독립 · unsubscribe 청소.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { ProtocolClient } from './protocolClient'
import type { ConnectionState, OutputChunk } from './agentClient'
import type { InboundMessage, Transport } from './transport'
import type { AgentInfo, AgentProfile, Preset, RestoreReport } from './types'

class MockTransport implements Transport {
  sent: unknown[] = []
  private _state: ConnectionState
  private stateCbs = new Set<(s: ConnectionState) => void>()
  private msgCb: ((m: InboundMessage) => void) | null = null
  ensureReadyCalls = 0
  startCalls = 0
  closed = false
  replayCalls: Array<{ agentId: string; gen: bigint }> = []
  private replayGenCounter = 0n
  /**
   * requestReplay 반환 제어. 함수를 심으면 그 반환 Promise 를 쓴다 — myGen 확정 지연(마커 먼저 도착) 재현.
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

  it('renamePreset(id, name) → RenamePreset{preset_id,name,request_id} 전송 + Ack 로 void resolve', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.renamePreset('pr1', '내 프리셋')
    await Promise.resolve()
    const sent = t.lastSent<{ request_id: string; preset_id: string; name: string | null }>('RenamePreset')!
    expect(sent.preset_id).toBe('pr1')
    expect(sent.name).toBe('내 프리셋')
    t.control({ Ack: { request_id: sent.request_id } })
    await expect(p).resolves.toBeUndefined()
  })

  it('renamePreset(id, null) → name=null 로 전송(override 해제)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.renamePreset('pr1', null)
    await Promise.resolve()
    const sent = t.lastSent<{ request_id: string; preset_id: string; name: string | null }>('RenamePreset')!
    expect(sent.name).toBeNull()
    t.control({ Ack: { request_id: sent.request_id } })
    await expect(p).resolves.toBeUndefined()
  })

  it('renameProfile(id, name) → RenameProfile{profile_id,name,request_id} 전송 + Ack 로 void resolve', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const p = c.renameProfile('a1', '내 에이전트')
    await Promise.resolve()
    const sent = t.lastSent<{ request_id: string; profile_id: string; name: string | null }>('RenameProfile')!
    expect(sent.profile_id).toBe('a1')
    expect(sent.name).toBe('내 에이전트')
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
    expect(t.replayCalls.map((r) => r.agentId)).toEqual([AGENT])
    const subs = t.sent.filter((m) => !!m && typeof m === 'object' && 'Subscribe' in (m as object))
    expect(subs.length).toBe(0)
  })

  it('성공 마커 전엔 buffering(직행 배달 없음), 마커 후 live 전환+flush', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    const gen = t.replayCalls[0].gen
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    expect(got).toEqual([])
    t.marker(AGENT, 1, gen)
    expect(got).toEqual([0, 1])
    t.output(AGENT, 1, 2)
    expect(got).toEqual([0, 1, 2])
  })

  it('tag 를 onChunk 로 그대로 전달(tag0/tag1) + 한 seq 공간 dedup', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const chunks: OutputChunk[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => chunks.push(chunk))
    const gen = t.replayCalls[0].gen
    t.output(AGENT, 1, 0, new Uint8Array([1]), 0)
    t.output(AGENT, 1, 1, new Uint8Array([2]), 1)
    t.marker(AGENT, 1, gen)
    t.output(AGENT, 1, 1, new Uint8Array([2]), 0)
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
    const g1got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => g1got.push(chunk.seq))
    const gen1 = t.replayCalls[0].gen
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    const g2got: number[] = []
    await c.subscribeOutput(V2, AGENT, (chunk) => g2got.push(chunk.seq))
    const gen2 = t.replayCalls[1].gen
    t.output(AGENT, 1, 2) // V1 replay 꼬리 = V2 버퍼 머리
    t.marker(AGENT, 1, gen1)
    expect(g1got).toEqual([0, 1, 2])
    expect(g2got).toEqual([])
    // V2 자기 replay 전체(single-flight 병합 후 전량 재replay) → seq 0,1,2 재전송 + 종결 gen2 마커.
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.output(AGENT, 1, 2)
    t.marker(AGENT, 1, gen2)
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
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, gen1)
    t.marker(AGENT, 1, gen2)
    expect(g1).toEqual([0, 1])
    expect(g2).toEqual([0, 1])
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
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, gen1)
    expect(g1).toEqual([0])
    await c.subscribeOutput(V2, AGENT, (chunk) => g2.push(chunk.seq))
    const gen2 = t.replayCalls[1].gen
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, gen2)
    expect(g2).toEqual([0, 1])
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
    const p1 = c.subscribeOutput(V1, AGENT, (chunk) => g1.push(chunk.seq))
    const p2 = c.subscribeOutput(V1, AGENT, (chunk) => g2.push(chunk.seq))
    await Promise.all([p1, p2])
    expect(t.replayCalls.length).toBe(1)
    const gen = t.replayCalls[0].gen
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, gen)
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
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, gen)
    expect(got).toEqual([])
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
    t.marker(AGENT, 1, myGen - 1n)
    t.marker(AGENT, 1, myGen)
    releaseGen(myGen)
    await Promise.resolve()
    await Promise.resolve()
    expect(got).toEqual([0])
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
    expect(got).toEqual([0, 1])
    expect(c.getViewOutputState(V1)?.phase).toBe('live')
    expect(states).toContain('live')
  })
})

// ── 화신 표식 = 불투명(대소 비교 금지) ────────────────────────────────────────────────
//
// ★사용자 결정 2026-08-20★: 표식은 화신마다 뽑은 난수라 두 값의 대소가 "더 새 것" 을 뜻하지 않는다.
//   그래서 프레임 쪽 규칙은 **불일치 = 내 것 아님(drop)** 하나뿐이고, 화신이 갈렸다는 선언은 권위인
//   명부만 낸다. 여기 대소 비교를 되살리면 표식이 우연히 작은 새 화신이 통째로 무시된다.
describe('화신 표식이 다른 프레임은 배달하지 않는다(대소를 보지 않는다)', () => {
  it('표식이 **큰** 프레임도 배달·회전 없이 떨어진다(옛 "epoch 상승=재spawn" 경로 제거)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    const states: string[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq), (s) => states.push(s))
    const gen1 = t.replayCalls[0].gen
    await Promise.resolve() // myGen=gen1 확정
    t.output(AGENT, 1, 0) // 이 프레임이 뷰의 표식을 1 로 고정한다
    t.output(AGENT, 2, 5) // 다른 화신 — 떨어뜨린다(재요청도 통지도 없다)
    expect(t.replayCalls.length).toBe(1)
    expect(states).toEqual([])

    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, gen1)
    expect(got).toEqual([0, 1]) // 표식 1 의 프레임만
  })

  it('표식이 **작은** 프레임도 같은 규칙으로 떨어진다', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    const gen1 = t.replayCalls[0].gen
    await Promise.resolve()
    t.output(AGENT, 7, 0)
    t.output(AGENT, 3, 1) // 더 작은 표식 — 옛 규칙에서도 drop 이었고 지금도 drop
    t.marker(AGENT, 7, gen1)
    expect(got).toEqual([0])
  })

  it('live 뷰도 표식이 다른 프레임을 먹지 않는다', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    t.output(AGENT, 4, 0)
    t.marker(AGENT, 4, t.replayCalls[0].gen)
    expect(c.getViewOutputState(V1)?.phase).toBe('live')

    t.output(AGENT, 9, 1) // 큰 표식
    t.output(AGENT, 2, 2) // 작은 표식
    expect(got).toEqual([0])
  })
})

// ── 끊김 = detached · 부착 계기 = 명부 ────────────────────────────────────────────────
//
// ★사용자 결정 2026-08-20 — 구독 계기는 소켓이 아니라 명부다★: 끊김은 detached 로 내려앉기만 하고,
//   요청은 "그 에이전트가 명부에 있다"는 관측에서만 나간다. 소켓 재전이를 계기로 삼으면, 재기동한 데몬의
//   빈 명부(부팅 자동 복원 없음)에 대고 모든 뷰가 전량 Subscribe 를 쏘아 전부 거절당한다 — 되살리지 말 것.
describe('연결이 끊기면 뷰는 detached — 화면·진도를 지키고 아무것도 안 보낸다', () => {
  it("끊김에 재요청이 나가지 않고 onState('detached') 만 온다", async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const states: string[] = []
    await c.subscribeOutput(V1, AGENT, () => {}, (s) => states.push(s))
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(states).toEqual(['live'])
    const callsBefore = t.replayCalls.length

    t.setState('reconnecting')
    expect(t.replayCalls.length).toBe(callsBefore)
    expect(states).toEqual(['live', 'detached'])
    expect(c.getViewOutputState(V1)?.phase).toBe('detached')
  })

  it('소켓이 다시 서는 것만으로는 재요청하지 않는다(명부를 기다린다)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    await c.subscribeOutput(V1, AGENT, () => {})
    const callsBefore = t.replayCalls.length

    t.setState('reconnecting')
    t.setState('connected')
    expect(t.replayCalls.length).toBe(callsBefore)
    expect(c.getViewOutputState(V1)?.phase).toBe('detached')
  })

  // ★거절 폭풍의 뿌리★: 재기동한 데몬의 명부엔 그 에이전트가 없다. 없는 것에 Subscribe 를 쏘면 전부 거절
  //   되므로, 없다고 확인된 동안은 아무것도 보내지 않고 대기 상태를 표면에 남긴다.
  it('명부에 없는 에이전트의 뷰는 detached 로 남아 Subscribe 를 보내지 않는다', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    await c.subscribeOutput(V1, AGENT, () => {})
    const callsBefore = t.replayCalls.length

    t.setState('reconnecting')
    t.setState('connected')
    t.control({ AgentListUpdated: { agents: [{ id: 'other-agent' }] } })

    expect(t.replayCalls.length).toBe(callsBefore)
    expect(c.getViewOutputState(V1)?.phase).toBe('detached')
  })

  it('detached 뷰는 남의 replay fan-out 프레임을 먹지 않는다(진도 오염 방지)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(got).toEqual([0])

    t.setState('reconnecting')
    t.output(AGENT, 1, 1) // 같은 agent 를 보는 다른 뷰의 replay 가 fan-out 으로 닿는 상황
    expect(got).toEqual([0])
  })

  // ★mount 도 재부착과 같은 한 규칙을 쓴다(사용자 결정 2026-08-20)★: 옛 mount 는 명부를 보지 않고 무조건
  //   쏘았고, 그래서 없는 에이전트를 가리킨 슬롯이 거절 → 사다리 → error 로 굳었다. 아래 세 케이스가 그
  //   한 규칙의 전부다 — 특히 "모른다" 를 "없다" 로 접으면 부팅·리로드·조회 실패에서 출력이 막힌다.
  it('mount: 명부를 아직 모르면 그대로 보낸다(모름 ≠ 부재)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const states: string[] = []
    await c.subscribeOutput(V1, AGENT, () => {}, (s) => states.push(s))
    expect(t.replayCalls.length).toBe(1)
    expect(states).toEqual([]) // 대기 표시도 없다 — 슬롯은 그냥 지금 모습 그대로 있는다
    expect(c.getViewOutputState(V1)?.phase).toBe('buffering')
  })

  it('mount: 명부가 알려져 있고 그 에이전트가 있으면 보낸다', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 4 }] } })
    await c.subscribeOutput(V1, AGENT, () => {})
    expect(t.replayCalls.length).toBe(1)
    expect(c.getViewOutputState(V1)?.phase).toBe('buffering')
  })

  it('mount: 명부가 알려져 있고 그 에이전트가 없으면 아무것도 안 보낸다', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const states: string[] = []
    t.control({ AgentListUpdated: { agents: [{ id: 'other-agent', epoch: 1 }] } })
    await c.subscribeOutput(V1, AGENT, () => {}, (s) => states.push(s))
    expect(t.replayCalls.length).toBe(0)
    expect(states).toEqual(['detached'])
    expect(c.getViewOutputState(V1)?.phase).toBe('detached')

    // 그리고 그 에이전트가 명부에 나타나면 그때 붙는다.
    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 1 }] } })
    expect(t.replayCalls.length).toBe(1)
    expect(c.getViewOutputState(V1)?.phase).toBe('buffering')
  })

  // 끊기면 명부 지식도 함께 버린다 — 죽은 데몬의 명부를 산 데몬의 사실로 쓰면 재기동 뒤 사라진
  //   에이전트를 살아 있다고 읽는다.
  it('끊기면 명부 지식을 버려 그 뒤 mount 는 다시 "모름" 규칙을 탄다', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    t.control({ AgentListUpdated: { agents: [{ id: 'other-agent', epoch: 1 }] } })
    t.setState('reconnecting')
    t.setState('connected')
    await c.subscribeOutput(V1, AGENT, () => {})
    expect(t.replayCalls.length).toBe(1)
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

describe('명부 부착 — 같은 화신 이어보기 vs 다른 화신 재구성', () => {
  // 소켓만 깜빡인 경우다(명부의 표식이 그대로) → ADR-0046 본래 동작으로 복귀: 커서를 지키고 전량
  //   재replay 를 내면 dedup 이 겹치는 앞부분을 흡수해 끊긴 동안 놓친 프레임만 그려진다.
  it('명부 표식이 그대로면 커서를 지킨 채 이어붙인다(비우기 신호 없음)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    const states: string[] = []
    let resets = 0
    await c.subscribeOutput(
      V1,
      AGENT,
      (chunk) => got.push(chunk.seq),
      (s) => states.push(s),
      () => (resets += 1),
    )
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(got).toEqual([0, 1])

    t.setState('reconnecting')
    t.setState('connected')
    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 1 }] } })

    expect(t.replayCalls.length).toBe(2)
    const gen2 = t.replayCalls[1].gen
    await Promise.resolve() // myGen=gen2 확정
    t.output(AGENT, 1, 0) // 전량 재replay 의 앞부분 — 이미 그렸으므로 dedup 탈락
    t.output(AGENT, 1, 1)
    t.output(AGENT, 1, 2) // 끊긴 동안 놓친 프레임만 새로 배달
    t.marker(AGENT, 1, gen2)
    expect(got).toEqual([0, 1, 2])
    // 비우기 신호가 한 번도 없어야 한다 = 소비자는 화면을 지우지 않는다.
    expect(resets).toBe(0)
    expect(states).toEqual(['live', 'detached', 'buffering', 'live'])
  })

  // ★데몬 재기동·재spawn·선택 복원의 공통 결말★: 다시 나타난 에이전트는 다른 화신이라 프레임 번호가 0
  //   부터 다시 매겨진다. 커서를 안 버리면 전량이 옛 high-water 밑에 깔려 통째로 탈락하고, 그 슬롯은 앱을
  //   다시 띄울 때까지 한 바이트도 못 그린다(껍데기 슬롯 — 실측 2026-08-20).
  it('명부 표식이 갈리면 커서를 버리고 다시 세운다(비우기 신호 = 배달 직전)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    const states: string[] = []
    // 비우기와 배달을 **한 줄에 섞어** 기록한다 — 따로 세면 "비우기가 먼저였나" 를 잴 수 없고, 그 순서가
    //   바로 "중간에 빈 화면이 비치지 않는다" 의 내용이다.
    const order: string[] = []
    await c.subscribeOutput(
      V1,
      AGENT,
      (chunk) => {
        got.push(chunk.seq)
        order.push(`chunk:${chunk.seq}`)
      },
      (s) => states.push(s),
      () => order.push('reset'),
    )
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.output(AGENT, 1, 2)
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(got).toEqual([0, 1, 2]) // 커서 = 2

    t.setState('reconnecting')
    t.setState('connected')
    t.control({ AgentListUpdated: { agents: [] } }) // 재기동한 데몬의 빈 명부 = 부재
    expect(t.replayCalls.length).toBe(1) // 아직 아무것도 안 보낸다
    // ★표식이 갈린 채 돌아왔다★ — 순서가 아니라 **다름**이 판정 근거다(여기선 일부러 더 작은 값).
    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 0 }] } })

    expect(t.replayCalls.length).toBe(2)
    // 요청 시점엔 아직 안 비운다 — 이 replay 가 실제로 올지 모른다.
    expect(states).toEqual(['live', 'detached', 'buffering'])
    expect(order).toEqual(['chunk:0', 'chunk:1', 'chunk:2'])
    const gen2 = t.replayCalls[1].gen
    await Promise.resolve() // myGen=gen2 확정
    t.output(AGENT, 0, 0) // 새 화신 — 번호가 0 부터 다시 시작한다
    t.output(AGENT, 0, 1)
    expect(got).toEqual([0, 1, 2]) // 아직 buffering — 화면은 그대로
    t.marker(AGENT, 0, gen2)
    // 도착한 그 순간에 비우기 신호가 나가고, **곧바로 이어서** 새 이력이 배달된다(사이에 빈 화면 없음).
    expect(order).toEqual(['chunk:0', 'chunk:1', 'chunk:2', 'reset', 'chunk:0', 'chunk:1'])
    expect(states).toEqual(['live', 'detached', 'buffering', 'live'])
    expect(got).toEqual([0, 1, 2, 0, 1])
    expect(c.getViewOutputState(V1)?.phase).toBe('live')
    // 커서가 새 화신 기준으로 다시 서야 이후 live 프레임도 이어진다(버리기만 하고 전진을 잃으면 안 된다).
    t.output(AGENT, 0, 2)
    t.output(AGENT, 0, 2) // 같은 seq 재도착은 여전히 dedup
    expect(got).toEqual([0, 1, 2, 0, 1, 2])
  })

  // ★비우기를 요청 시점에 하면 안 되는 이유의 회귀★: 데몬을 막 재기동한 직후가 정확히 이 경우다 —
  //   부착은 나가지만 replay 가 거절돼 돌아오지 않는다. 그때 화면을 이미 지웠으면 그 슬롯은 영구히 빈
  //   채로 남는다.
  it('부착한 replay 가 실패하면 화면을 비우라고 하지 않는다(앞 화신 화면 유지)', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      const got: number[] = []
      const states: string[] = []
      let resets = 0
      await c.subscribeOutput(
        V1,
        AGENT,
        (chunk) => got.push(chunk.seq),
        (s) => states.push(s),
        () => (resets += 1),
      )
      t.output(AGENT, 1, 0)
      t.marker(AGENT, 1, t.replayCalls[0].gen)
      expect(got).toEqual([0])

      t.setState('reconnecting')
      t.setState('connected')
      t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 2 }] } }) // 표식이 갈렸다
      await Promise.resolve()
      // 사다리를 끝까지 소진시킨다 — 어느 단계에서도 실패는 비우기를 부르지 않는다.
      for (let i = 0; i < 4; i++) {
        t.marker(AGENT, 2, t.replayCalls[t.replayCalls.length - 1].gen, { failed: true })
        await vi.advanceTimersByTimeAsync(5000)
      }
      expect(c.getViewOutputState(V1)?.phase).toBe('error')
      expect(resets).toBe(0)
      expect(got).toEqual([0]) // 그려 둔 것은 그대로
    } finally {
      vi.useRealTimers()
    }
  })

  // 붙어 있는 뷰의 회전도 명부가 낸다 — 프레임 쪽은 불일치를 그냥 떨어뜨리므로 여기 말고 알아볼 자리가 없다.
  it('끊긴 적 없이 재spawn 돼도 명부의 새 표식이 live 뷰를 다시 세운다', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(chunk.seq))
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(c.getViewOutputState(V1)?.phase).toBe('live')

    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 77 }] } })
    expect(t.replayCalls.length).toBe(2)
    expect(c.getViewOutputState(V1)?.phase).toBe('buffering')
    const gen2 = t.replayCalls[1].gen
    await Promise.resolve()
    t.output(AGENT, 77, 0)
    t.marker(AGENT, 77, gen2)
    expect(got).toEqual([0, 0])
  })

  // ★비우기 신호가 **이미 열려 있는 뷰**에도 닿아야 한다★: 재구독 트리거에서 화신을 뺀 뒤로, 재spawn 을
  //   알아보고 커서를 돌리는 자리는 여기 하나뿐이다. 위 케이스가 "끊겼다 온" 경로라면 이건 소켓이 한 번도
  //   안 끊긴 경로 — 슬롯 입장에선 아무 일도 안 일어난 것처럼 보이는 구간이라 놓치기 쉽다.
  it('끊긴 적 없는 live 뷰도 재spawn 이면 비우기 신호를 받고 새 이력을 그린다', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const order: string[] = []
    await c.subscribeOutput(
      V1,
      AGENT,
      (chunk) => order.push(`chunk:${chunk.seq}`),
      undefined,
      () => order.push('reset'),
    )
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(order).toEqual(['chunk:0', 'chunk:1'])

    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 77 }] } })
    expect(t.replayCalls.length).toBe(2)
    const gen2 = t.replayCalls[1].gen
    await Promise.resolve()
    // 새 화신 — 번호가 0 부터 다시 온다. 커서를 안 버렸으면 전부 dedup 탈락한다.
    t.output(AGENT, 77, 0)
    t.output(AGENT, 77, 1)
    expect(order).toEqual(['chunk:0', 'chunk:1']) // 아직 buffering — 화면 그대로
    t.marker(AGENT, 77, gen2)
    expect(order).toEqual(['chunk:0', 'chunk:1', 'reset', 'chunk:0', 'chunk:1'])
  })

  // ★표식 없는 명부 항목을 "같은 화신" 으로 단정하지 않는다★: 그 단정이 곧 원래 결함이다 — 커서를 지킨
  //   채 0 부터 오는 새 화신을 통째로 dedup 탈락시켜 슬롯이 영영 빈 채로 남는다. 잘못 회전한 대가는
  //   전량 replay 1회뿐이고 그건 화면에 티도 안 난다(비우기와 다시 그리기가 같은 틱).
  it('명부 항목에 표식이 없으면 회전으로 취급한다(조용히 커서를 지키지 않는다)', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const order: string[] = []
    await c.subscribeOutput(
      V1,
      AGENT,
      (chunk) => order.push(`chunk:${chunk.seq}`),
      undefined,
      () => order.push('reset'),
    )
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(order).toEqual(['chunk:0', 'chunk:1'])

    t.control({ AgentListUpdated: { agents: [{ id: AGENT }] } }) // 표식 없음
    expect(t.replayCalls.length).toBe(2)
    const gen2 = t.replayCalls[1].gen
    await Promise.resolve()
    t.output(AGENT, 3, 0)
    t.marker(AGENT, 3, gen2)
    expect(order).toEqual(['chunk:0', 'chunk:1', 'reset', 'chunk:0'])
  })

  // ★명부가 "없다" 고 말하면 붙어 있던 뷰도 내려앉힌다★: 안 내리면 수거된 에이전트를 보는 슬롯이
  //   getViewOutputState 에 계속 live 로 보고돼(§5 자동화 계약) 화면과 표면이 어긋난다. 단 화면·커서는
  //   그대로 둔다 — 다시 나타나면 이어보기다.
  it('명부에서 사라지면 live 뷰도 detached 로 내려앉고 화면·커서는 지킨다', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: number[] = []
    const states: string[] = []
    let resets = 0
    await c.subscribeOutput(
      V1,
      AGENT,
      (chunk) => got.push(chunk.seq),
      (s) => states.push(s),
      () => (resets += 1),
    )
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 1)
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    expect(c.getViewOutputState(V1)?.phase).toBe('live')

    t.control({ AgentListUpdated: { agents: [{ id: 'other-agent', epoch: 1 }] } })
    expect(c.getViewOutputState(V1)?.phase).toBe('detached')
    expect(states).toEqual(['live', 'detached'])
    expect(resets).toBe(0) // 화면은 건드리지 않는다
    expect(t.replayCalls.length).toBe(1) // 없는 것에 요청하지 않는다

    // 같은 표식으로 돌아오면 이어보기 — 커서가 남아 있어 겹치는 앞부분은 dedup 이 흡수한다.
    t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 1 }] } })
    const gen2 = t.replayCalls[1].gen
    await Promise.resolve()
    t.output(AGENT, 1, 0)
    t.output(AGENT, 1, 2)
    t.marker(AGENT, 1, gen2)
    expect(got).toEqual([0, 1, 2])
    expect(resets).toBe(0)
  })

  // ★표식은 프레임이 아니라 성공 마커가 정한다★: 표식 미상으로 부착한 뷰가 먼저 온 프레임으로 자기
  //   화신을 정하면, 앞 화신의 늦은 프레임 하나가 그 자리를 차지해 **진짜 새 화신의 프레임이 전부**
  //   불일치로 떨어지고 뷰는 사다리를 소진해 error 로 앉는다(한 프레임이 슬롯을 죽인다).
  it('부착 직후 앞 화신의 늦은 프레임이 섞여도 새 화신 이력을 온전히 그린다', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const got: string[] = []
    const dec = new TextDecoder()
    await c.subscribeOutput(V1, AGENT, (chunk) => got.push(dec.decode(chunk.bytes)))
    await Promise.resolve() // myGen 확정
    const gen = t.replayCalls[0].gen

    // 앞 화신(표식 5)의 꼬리가 먼저 도착한다 — 이 뷰가 청구한 replay 의 것이 아니다.
    t.output(AGENT, 5, 0, new TextEncoder().encode('stale'))
    // 이어서 진짜 새 화신(표식 6)의 전량 replay.
    t.output(AGENT, 6, 0, new TextEncoder().encode('fresh-0'))
    t.output(AGENT, 6, 1, new TextEncoder().encode('fresh-1'))
    t.marker(AGENT, 6, gen)

    expect(c.getViewOutputState(V1)?.phase).toBe('live')
    expect(got).toEqual(['fresh-0', 'fresh-1'])
  })

  // ★끌어오는 문도 부착시킨다(회귀 대상)★: 재연결 뒤 첫 명부는 브로드캐스트가 아니라 이 조회로 온다
  //   (eventBus.resyncAfterReconnect). reply 는 request_id pending 으로 회수돼 브로드캐스트 라우팅을
  //   지나지 않으므로, 이 문을 빠뜨리면 주 복구 경로가 영영 부착되지 않는다.
  it('명부 조회(getAgents) 결과로도 부착한다', async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    await c.subscribeOutput(V1, AGENT, () => {})
    t.setState('reconnecting')
    t.setState('connected')
    expect(c.getViewOutputState(V1)?.phase).toBe('detached')

    const p = c.getAgents()
    await Promise.resolve()
    const rid = t.lastSent<{ request_id: string }>('ListAgents')!.request_id
    t.control({ AgentList: { request_id: rid, agents: [{ id: AGENT }] } })
    await p

    expect(t.replayCalls.length).toBe(2)
    expect(c.getViewOutputState(V1)?.phase).toBe('buffering')
  })

  // ADR-0145: 부착도 복원 구간의 시작이다 — 통지 없이는 소비자가 그 사이를 '복원 완료' 로 오인한 채
  //   화면을 유지하다 재flush 에 뒤집힌다.
  it("부착한 뷰는 재flush 뒤 다시 'live' 통지를 받는다", async () => {
    const t = new MockTransport()
    const c = new ProtocolClient(t)
    const states: string[] = []
    await c.subscribeOutput(V1, AGENT, () => {}, (s) => states.push(s))
    t.marker(AGENT, 1, t.replayCalls[0].gen)
    t.setState('reconnecting')
    t.setState('connected')
    t.control({ AgentListUpdated: { agents: [{ id: AGENT }] } })

    const gen2 = t.replayCalls[1].gen
    await Promise.resolve() // myGen=gen2 확정
    t.marker(AGENT, 1, gen2)
    expect(states).toEqual(['live', 'detached', 'buffering', 'live'])
  })

  // 사다리를 소진해 error 로 앉은 뷰도 "에이전트가 실재한다"가 확인되면 회복 기회를 받는다(옛 재연결
  //   전량 리셋이 주던 회복을 명부 부착이 대신한다).
  it('error 로 앉은 뷰도 명부 부착에서 사다리를 리셋하고 되살아난다', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      await c.subscribeOutput(V1, AGENT, () => {})
      await Promise.resolve()
      for (let i = 0; i < 3; i++) {
        t.marker(AGENT, 1, t.replayCalls[t.replayCalls.length - 1].gen, { failed: true })
        await vi.advanceTimersByTimeAsync(5000)
      }
      t.marker(AGENT, 1, t.replayCalls[t.replayCalls.length - 1].gen, { failed: true })
      expect(c.getViewOutputState(V1)?.phase).toBe('error')

      t.setState('reconnecting')
      t.setState('connected')
      t.control({ AgentListUpdated: { agents: [{ id: AGENT }] } })
      expect(c.getViewOutputState(V1)?.phase).toBe('buffering')
      expect(c.getViewOutputState(V1)?.attempts).toBe(0)
    } finally {
      vi.useRealTimers()
    }
  })
})

// ── 연결 실패 이유(ADR-0134) ────────────────────────────────────────────────────────
describe('연결 실패 이유가 연결 상태 표면에 함께 실린다', () => {
  it('상태가 그대로여도 이유가 생기면 구독자에게 통지된다', () => {
    const t = new MockTransport('down')
    const c = new ProtocolClient(t)
    const seen: Array<ConnectionState> = []
    c.onConnectionStateChange((s) => seen.push(s))
    expect(seen).toEqual(['down']) // 등록 즉시 1회
    expect(c.connectionError).toBeNull()

    c.reportConnectionError('데이터 폴더에 쓸 수 없음(C:\\x) — 쓰기 가능한 위치에 압축을 풀어 주세요')

    // 상태 문자열은 'down' 그대로지만 다시 통지돼야 화면이 이유를 그릴 수 있다.
    expect(seen).toEqual(['down', 'down'])
    expect(c.connectionError).toContain('쓰기 가능한 위치')
  })

  it('같은 이유를 다시 보고하면 통지하지 않는다', () => {
    const t = new MockTransport('down')
    const c = new ProtocolClient(t)
    const seen: Array<ConnectionState> = []
    c.onConnectionStateChange((s) => seen.push(s))
    c.reportConnectionError('같은 이유')
    c.reportConnectionError('같은 이유')
    expect(seen.length).toBe(2) // 최초 1 + 변화 1
  })

  it('connected 로 전이하면 이유가 지워진다', () => {
    const t = new MockTransport('down')
    const c = new ProtocolClient(t)
    c.reportConnectionError('폴더 문제')
    expect(c.connectionError).toBe('폴더 문제')
    t.setState('connected')
    expect(c.connectionError).toBeNull()
  })

  // ★R4 회귀 방지★: 이미 connected 인 상태에서 이유를 기록하면 지울 전이가 영영 오지 않아 배너가
  //   고착된다. 이 표면은 window.__ENGRAM_AGENT__ 로 공개돼 LLM·cdp 호출로도 닿는다.
  it('이미 연결된 상태에서 보고된 이유는 남지 않는다', () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    const seen: Array<ConnectionState> = []
    c.onConnectionStateChange((s) => seen.push(s))

    c.reportConnectionError('연결돼 있는데 들어온 이유')

    expect(c.connectionError).toBeNull()
    expect(seen).toEqual(['connected']) // 등록 1회뿐 — 통지도 없다
  })

  it('끊긴 뒤 기록한 이유는 재연결 전까지 유지된다', () => {
    const t = new MockTransport('connected')
    const c = new ProtocolClient(t)
    t.setState('down')
    c.reportConnectionError('폴더 문제')
    expect(c.connectionError).toBe('폴더 문제')
    t.setState('connected')
    expect(c.connectionError).toBeNull()
  })

  it('구독 해제 후에는 통지가 가지 않는다', () => {
    const t = new MockTransport('down')
    const c = new ProtocolClient(t)
    const seen: Array<ConnectionState> = []
    const off = c.onConnectionStateChange((s) => seen.push(s))
    off()
    c.reportConnectionError('이유')
    t.setState('connected')
    expect(seen).toEqual(['down'])
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
      expect(t.replayCalls.length).toBe(1)
      let gen = t.replayCalls[t.replayCalls.length - 1].gen
      // 실패 마커 → 사다리 1단계(백오프 1s 뒤 재요청).
      t.marker(AGENT, 1, gen, { failed: true })
      await vi.advanceTimersByTimeAsync(1000)
      expect(t.replayCalls.length).toBe(2)
      gen = t.replayCalls[t.replayCalls.length - 1].gen
      // 실패 마커 → 사다리 2단계(2s).
      t.marker(AGENT, 1, gen, { failed: true })
      await vi.advanceTimersByTimeAsync(2000)
      expect(t.replayCalls.length).toBe(3)
      gen = t.replayCalls[t.replayCalls.length - 1].gen
      // 실패 마커 → 사다리 3단계(4s).
      t.marker(AGENT, 1, gen, { failed: true })
      await vi.advanceTimersByTimeAsync(4000)
      expect(t.replayCalls.length).toBe(4)
      gen = t.replayCalls[t.replayCalls.length - 1].gen
      // 4번째 실패 마커 → 상한 소진 → error(재요청 없음).
      t.marker(AGENT, 1, gen, { failed: true })
      await vi.advanceTimersByTimeAsync(10000)
      expect(t.replayCalls.length).toBe(4)
      expect(states).toContain('error')
      expect(c.getViewOutputState(V1)?.phase).toBe('error')
    } finally {
      vi.useRealTimers()
    }
  })

  // ★실패 마커는 epoch 펜스보다 앞이다(회귀 대상)★: 실패엔 SubscribeAck 이 없어 보내는 쪽이 그 세대의
  //   epoch 를 확정할 수 없고, 마지막으로 알려진 값(없으면 0)을 최선치로 싣는다(src-tauri
  //   plan_subscribe_refusal · deadline sweep). 그 값에 펜스를 걸면 마커가 조용히 버려지고 뷰는 10초
  //   watchdog 을 기다린다 — 즉시 사다리를 밀려고 마커를 낸 이유 자체가 사라진다.
  // ★표식을 아는 뷰에서만 성립하는 케이스다★: 뷰가 자기 화신을 아는 구간은 "같은 화신 이어보기" 로
  //   재부착한 buffering 뿐이다(새 화신 부착은 표식을 비우고 시작하고, 프레임으로는 표식을 줍지 않는다).
  //   그래서 이어보기 상태를 만든 뒤 재본다.
  it('epoch 가 어긋난 실패 마커도 사다리를 민다(성공 마커는 여전히 펜스에 걸린다)', async () => {
    vi.useFakeTimers()
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      await c.subscribeOutput(V1, AGENT, () => {})
      await Promise.resolve() // myGen 확정
      t.output(AGENT, 9, 0)
      t.marker(AGENT, 9, t.replayCalls[0].gen) // 성공 마커가 표식 9 를 채택시킨다
      expect(c.getViewOutputState(V1)?.phase).toBe('live')

      // 같은 표식으로 재부착 = 이어보기 → 뷰는 9 를 든 채 buffering 으로 돌아간다.
      t.setState('reconnecting')
      t.setState('connected')
      t.control({ AgentListUpdated: { agents: [{ id: AGENT, epoch: 9 }] } })
      await Promise.resolve() // myGen 확정
      const gen = t.replayCalls[1].gen
      expect(c.getViewOutputState(V1)?.phase).toBe('buffering')

      // 성공 마커는 epoch 가 어긋나면 여전히 무시된다(불완전 버퍼 조기 flush 금지).
      t.marker(AGENT, 0, gen)
      expect(c.getViewOutputState(V1)?.phase).toBe('buffering')

      // 실패 마커는 같은 epoch 불일치에도 통과해 즉시 사다리를 민다.
      const before = t.replayCalls.length
      t.marker(AGENT, 0, gen, { failed: true })
      await vi.advanceTimersByTimeAsync(1000)
      expect(t.replayCalls.length).toBe(before + 1)
    } finally {
      vi.useRealTimers()
    }
  })

  // ★거절의 진단은 남는다★: 거절이 `Error` 를 떠나면서 backend-error warn 이 이걸 더는 못 잡는다.
  //   이 가지가 없으면 `SubscribeFailed` 는 handleEvent 의 조용한 꼬리로 흘러 흔적 없이 사라진다.
  //   ★상태기계는 건드리지 않는다★ — 같은 사건이 replayBoundary 로도 오므로(carrier 가 정규화) 여기서
  //   또 만지면 사다리를 두 번 민다.
  it('SubscribeFailed control 이벤트는 경고만 남기고 뷰 상태를 건드리지 않는다', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const t = new MockTransport()
      const c = new ProtocolClient(t)
      await c.subscribeOutput(V1, AGENT, () => {})
      const before = t.replayCalls.length

      t.control({ SubscribeFailed: { agent_id: AGENT, reason: `agent ${AGENT} not found` } })

      expect(warn).toHaveBeenCalled()
      expect(warn.mock.calls.some((a) => String(a[0]).includes('subscribe refused'))).toBe(true)
      expect(t.replayCalls.length).toBe(before)
      expect(c.getViewOutputState(V1)?.phase).toBe('buffering')
    } finally {
      warn.mockRestore()
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
      expect(c.getViewOutputState(V1)?.buffered).toBe(0)
      // 사다리 백오프(1s) 후 재요청 발행. requestReplay 회수(microtask) 후 myGen=gen2 확정.
      await vi.advanceTimersByTimeAsync(1000)
      expect(t.replayCalls.length).toBe(2)
      const gen2 = t.replayCalls[1].gen
      // ★stale flush 금지★: pre-overflow gen(gen1) 성공 마커가 와도 폐기된 5MB 프레임을 flush 하지 않는다.
      //   (gen1 < myGen=gen2 라 gen 펜스로도 무시되지만, buffer 자체가 비어 flush 할 것도 없다.)
      t.marker(AGENT, 1, gen1)
      expect(got).toEqual([])
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
      expect(c.getViewOutputState(V1)?.buffered).toBe(0)

      // ★핵심: 백오프(1s) 발화 *전* 에 구 gen(gen1) 성공 마커 도착★. FIX-A 로 myGen 이 무효화됐으므로
      //   이 마커는 evalMarker 의 myGen===undefined 분기로 held 만 되고 flush 하지 않는다(펜스 통과 불가).
      t.marker(AGENT, 1, gen1)
      expect(got).toEqual([])
      expect(states).not.toContain('live')
      expect(c.getViewOutputState(V1)?.phase).toBe('buffering')

      // 백오프 발화 → 재요청 발행(재요청이 취소되지 않았음을 증명). requestReplay 회수 후 myGen=gen2 확정.
      await vi.advanceTimersByTimeAsync(1000)
      expect(t.replayCalls.length).toBe(2)
      const gen2 = t.replayCalls[1].gen

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
      t.output(AGENT, 1, 0)
      t.output(AGENT, 1, 1)
      // watchdog 만료(10s) → 재요청. ★flush 아님★: got 은 여전히 비어야.
      await vi.advanceTimersByTimeAsync(10000)
      expect(got).toEqual([])
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
    await c.subscribeOutput(V1, AGENT, (chunk) => g2.push(chunk.seq))
    const gen = t.replayCalls[t.replayCalls.length - 1].gen
    sub1.unsubscribe()
    t.output(AGENT, 1, 0)
    t.marker(AGENT, 1, gen)
    expect(g2).toEqual([0])
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
    t.output(AGENT, 2, 1)
    t.output(AGENT, 1, 1)
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
    expect(seen).toEqual([presets])
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
