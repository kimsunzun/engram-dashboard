// usageStore — 사용량 값 받기(TRD S21 usage-limit-slot §4 「프론트 값 받기」·「프론트 스토어·방송」 행).
//
// 세 층을 잰다:
//   ① `merge` 의 규칙(revision · socketEpoch) — 스토어 단독.
//   ② pull · ⟳ — 모의 agentClient(스토어가 `agentClient` 말고는 아무것도 안 부르는지 · `invoke` 0).
//   ③ eventBus 잇기 — 잇기와 pull 의 순서(기록형 모의) · 실 ProtocolClient 를 거친 끊김·Ack(값·revision 불변).
// label 거름(자기 창에 온 방송만)은 carrier 몫이라 `api/tauriTransport.test.ts` 가 잰다.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { UsageLimitSnapshot } from '../../crates/engram-dashboard-protocol/bindings/UsageLimitSnapshot'
import type { ConnectionState, UsageSnapshotPull } from '../api/agentClient'
import { ProtocolClient } from '../api/protocolClient'
import type { InboundMessage, Transport } from '../api/transport'
import type { AgentBackendKind } from '../api/types'

const invokeSpy = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeSpy,
  Channel: class {
    onmessage: unknown = null
  },
}))

// agentClient = 테스트가 심는 객체. 접근한 이름을 모아 「사용량 경로가 부르는 표면」을 단언한다.
const holder = vi.hoisted(() => ({
  client: null as unknown as Record<string, unknown>,
  accessed: new Set<string>(),
}))
vi.mock('../api/clientFactory', () => ({
  agentClient: new Proxy(
    {},
    {
      get(_t, prop: string) {
        holder.accessed.add(prop)
        const v = holder.client[prop]
        return typeof v === 'function' ? (v as (...a: unknown[]) => unknown).bind(holder.client) : v
      },
    },
  ),
}))

// eventBus 의 레이아웃 구독·부팅 pull 은 이 시험과 무관하다(Tauri 를 부른다).
vi.mock('./viewStore', () => ({
  subscribeViewEvents: () => ({ dispose: () => {}, ready: Promise.resolve() }),
  initMainWindowFromBackend: async () => {},
}))

import { useUsageStore } from './usageStore'

function snap(vendor: AgentBackendKind, revision: number, usedPct = revision, ageSecs = 0): UsageLimitSnapshot {
  return {
    vendor,
    account_key: 'default',
    five_hour: { used_pct: usedPct, resets_at: 1_900_000_000, age_secs: ageSecs, expired: false },
    weekly: null,
    model_scoped: [],
    plan: null,
    in_flight: false,
    state: { kind: 'Ready' },
    revision,
  }
}

function deferred<T>(): { promise: Promise<T>; resolve: (v: T) => void; reject: (e: unknown) => void } {
  let resolve!: (v: T) => void
  let reject!: (e: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

async function flush(ticks = 8): Promise<void> {
  for (let i = 0; i < ticks; i++) await Promise.resolve()
}

const store = () => useUsageStore.getState()
const revisionOf = (vendor: AgentBackendKind) => store().vendors[vendor]?.revision
const usedOf = (vendor: AgentBackendKind) => store().vendors[vendor]?.snapshot.five_hour?.used_pct

function resetStore(): void {
  useUsageStore.setState({ vendors: {}, socketEpoch: 0, pending: {} })
}

beforeEach(() => {
  resetStore()
  invokeSpy.mockClear()
  holder.accessed.clear()
  holder.client = {}
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.useRealTimers()
})

// ── ① merge — revision(같은 소켓) ─────────────────────────────────────────────────────
describe('merge — revision', () => {
  it('더 작은 revision 은 버리고 더 큰 것은 받는다', () => {
    store().merge(snap('claude', 5), 1)
    store().merge(snap('claude', 3), 1)
    expect(revisionOf('claude')).toBe(5)
    expect(usedOf('claude')).toBe(5)
    store().merge(snap('claude', 7), 1)
    expect(revisionOf('claude')).toBe(7)
    expect(usedOf('claude')).toBe(7)
  })

  it('같은 revision 은 늦게 닿은 사본을 받는다 — 상대 시간 기준(receivedAt)이 새로 선다', () => {
    const now = vi.spyOn(performance, 'now').mockReturnValue(1_000)
    store().merge(snap('claude', 5, 40, 30), 1)
    now.mockReturnValue(2_000)
    store().merge(snap('claude', 5, 40, 2), 1)
    const held = store().vendors.claude!
    expect(held.receivedAt).toBe(2_000)
    expect(held.snapshot.five_hour?.age_secs).toBe(2)
  })

  it('회사끼리 revision 은 독립이다', () => {
    store().merge(snap('claude', 10), 1)
    store().merge(snap('codex', 1), 1)
    expect(revisionOf('codex')).toBe(1)
    store().merge(snap('claude', 5), 1)
    store().merge(snap('codex', 2), 1)
    expect(revisionOf('claude')).toBe(10)
    expect(revisionOf('codex')).toBe(2)
  })

  it('켜고 끔을 모른다 — 받은 회사는 전부 든다(거름은 그리는 쪽)', () => {
    store().merge(snap('codex', 1), 1)
    store().merge(snap('claude', 1), 1)
    expect(Object.keys(store().vendors).sort()).toEqual(['claude', 'codex'])
  })
})

// ── ① merge — socketEpoch(ADR-0195) ────────────────────────────────────────────────────
describe('merge — socketEpoch', () => {
  it('더 큰 표식 → revision 을 전부 잊고 작은 revision 을 받는다(연결 상태 전이 없이 — 세대 승계) · 값은 유지', () => {
    store().merge(snap('claude', 50), 1)
    store().merge(snap('codex', 9), 1)
    store().merge(snap('claude', 1, 77), 2)
    expect(store().socketEpoch).toBe(2)
    expect(revisionOf('claude')).toBe(1)
    expect(usedOf('claude')).toBe(77)
    // 새 소켓에서 아직 안 온 회사: 값은 그대로, revision 만 잊었다 → 새 소켓의 revision 1 도 받는다.
    expect(usedOf('codex')).toBe(9)
    expect(revisionOf('codex')).toBeNull()
    store().merge(snap('codex', 1, 3), 2)
    expect(revisionOf('codex')).toBe(1)
    expect(usedOf('codex')).toBe(3)
  })

  it('더 작은 표식 → 버린다(밀려난 소켓의 것)', () => {
    store().merge(snap('claude', 4), 3)
    store().merge(snap('claude', 100), 2)
    expect(store().socketEpoch).toBe(3)
    expect(revisionOf('claude')).toBe(4)
  })

  it('표식 0 → 들고 있는 표식을 바꾸지 않고 revision 규칙만 탄다', () => {
    store().merge(snap('claude', 5), 4)
    store().merge(snap('claude', 9), 0)
    expect(store().socketEpoch).toBe(4)
    expect(revisionOf('claude')).toBe(9)
    store().merge(snap('claude', 2), 0)
    expect(revisionOf('claude')).toBe(9)
  })

  it('표식 0 pull 반환(셸에 소켓 없음 — 빈 목록) → 표식·값 불변', async () => {
    store().merge(snap('claude', 5), 4)
    const before = store().vendors
    holder.client = { getUsageSnapshot: vi.fn(async () => ({ socketEpoch: 0, snapshots: [] })) }
    await store().pull()
    expect(store().socketEpoch).toBe(4)
    expect(store().vendors).toBe(before)
  })
})

// ── ② pull ────────────────────────────────────────────────────────────────────────────
describe('pull', () => {
  it('반환 스냅숏은 반환된 표식과 함께 merge 를 지난다', async () => {
    holder.client = {
      getUsageSnapshot: vi.fn(async () => ({ socketEpoch: 3, snapshots: [snap('claude', 2), snap('codex', 5)] })),
    }
    await store().pull()
    expect(store().socketEpoch).toBe(3)
    expect(revisionOf('claude')).toBe(2)
    expect(revisionOf('codex')).toBe(5)
  })

  it('방송 뒤에 늦게 돌아온 pull 반환(더 작은 revision)은 버린다', async () => {
    const reply = deferred<UsageSnapshotPull>()
    holder.client = { getUsageSnapshot: vi.fn(() => reply.promise) }
    const pulling = store().pull()
    store().merge(snap('claude', 8), 3) // pull 이 도는 사이 닿은 방송
    reply.resolve({ socketEpoch: 3, snapshots: [snap('claude', 6)] })
    await pulling
    expect(revisionOf('claude')).toBe(8)
    expect(usedOf('claude')).toBe(8)
  })

  it('실패 → 유계 재시도 · 소진해도 throw 하지 않고 들고 있는 값을 둔다', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    store().merge(snap('claude', 5), 2)
    const before = store().vendors
    const getUsageSnapshot = vi.fn(async () => {
      throw new Error('ipc not ready')
    })
    holder.client = { getUsageSnapshot }
    const pulling = store().pull()
    await vi.runAllTimersAsync()
    await expect(pulling).resolves.toBeUndefined()
    expect(getUsageSnapshot).toHaveBeenCalledTimes(4)
    expect(store().vendors).toBe(before)
    expect(store().socketEpoch).toBe(2)
  })

  it('일시 실패 뒤 성공하면 그 반환을 받는다', async () => {
    vi.useFakeTimers()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const getUsageSnapshot = vi
      .fn<() => Promise<UsageSnapshotPull>>()
      .mockRejectedValueOnce(new Error('ipc not ready'))
      .mockResolvedValue({ socketEpoch: 1, snapshots: [snap('codex', 3)] })
    holder.client = { getUsageSnapshot }
    const pulling = store().pull()
    await vi.runAllTimersAsync()
    await pulling
    expect(getUsageSnapshot).toHaveBeenCalledTimes(2)
    expect(revisionOf('codex')).toBe(3)
  })
})

// ── ② refresh(⟳) ─────────────────────────────────────────────────────────────────────
describe('refresh(⟳)', () => {
  it('refreshUsageLimits(vendor) 를 부르고 도는 동안만 pending', async () => {
    const answer = deferred<void>()
    const refreshUsageLimits = vi.fn(() => answer.promise)
    holder.client = { refreshUsageLimits }
    const refreshing = store().refresh('codex')
    expect(refreshUsageLimits).toHaveBeenCalledWith('codex')
    expect(store().pending.codex).toBe(1)
    expect(store().pending.claude).toBeUndefined()
    answer.resolve()
    await refreshing
    expect(store().pending.codex).toBe(0)
  })

  it('겹친 ⟳ 둘 — 둘 다 끝나야 pending 이 풀린다', async () => {
    const a = deferred<void>()
    const b = deferred<void>()
    holder.client = { refreshUsageLimits: vi.fn().mockReturnValueOnce(a.promise).mockReturnValueOnce(b.promise) }
    const first = store().refresh('claude')
    const second = store().refresh('claude')
    a.resolve()
    await first
    expect(store().pending.claude).toBe(1)
    b.resolve()
    await second
    expect(store().pending.claude).toBe(0)
  })

  it('실패해도 throw 하지 않고 pending 을 푼다', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    holder.client = { refreshUsageLimits: vi.fn(async () => Promise.reject(new Error('connection lost'))) }
    await expect(store().refresh('claude')).resolves.toBeUndefined()
    expect(store().pending.claude).toBe(0)
  })

  it('스토어는 agentClient 의 사용량 두 메서드만 부른다 — invoke 0', async () => {
    holder.client = {
      getUsageSnapshot: vi.fn(async () => ({ socketEpoch: 1, snapshots: [snap('claude', 1)] })),
      refreshUsageLimits: vi.fn(async () => {}),
    }
    await store().pull()
    await store().refresh('claude')
    store().merge(snap('codex', 1), 1)
    expect(invokeSpy).not.toHaveBeenCalled()
    expect([...holder.accessed].sort()).toEqual(['getUsageSnapshot', 'refreshUsageLimits'])
  })
})

// ── ③ eventBus 잇기 ──────────────────────────────────────────────────────────────────
// initEventBus 는 모듈 전역에 한 번만 도는 약속을 쥐므로 시험마다 모듈을 새로 읽는다(스토어도 함께 새로 선다).
describe('eventBus — 방송 잇기 먼저, pull 나중', () => {
  type UsageCb = (snapshot: UsageLimitSnapshot, socketEpoch: number) => void

  async function freshModules() {
    vi.resetModules()
    const bus = await import('./eventBus')
    const usage = await import('./usageStore')
    return { initEventBus: bus.initEventBus, useUsageStore: usage.useUsageStore }
  }

  function recordingClient(reply: UsageSnapshotPull = { socketEpoch: 1, snapshots: [] }) {
    const log: string[] = []
    let usageCb: UsageCb | null = null
    const off = () => () => {}
    const client = {
      connectionState: 'connected' as ConnectionState,
      onAgentListUpdated: off,
      onStatusChanged: off,
      onRestoreResult: off,
      onProfileListUpdated: off,
      onPresetListUpdated: off,
      onConnectionStateChange: (cb: (s: ConnectionState) => void) => {
        cb('connected')
        return () => {}
      },
      onUsageLimitsUpdated: (cb: UsageCb) => {
        log.push('listen')
        usageCb = cb
        return () => {
          usageCb = null
        }
      },
      getUsageSnapshot: vi.fn(async () => {
        log.push(usageCb ? 'pull(listening)' : 'pull(not listening)')
        return reply
      }),
      refreshUsageLimits: vi.fn(async () => {}),
    }
    return { log, client, broadcast: (s: UsageLimitSnapshot, e: number) => usageCb?.(s, e) }
  }

  afterEach(() => {
    delete (globalThis as Record<string, unknown>).__engramCmd
  })

  it('잇기가 선 뒤에 pull 한 번', async () => {
    const { initEventBus } = await freshModules()
    const rec = recordingClient()
    holder.client = rec.client
    await initEventBus()
    await flush()
    expect(rec.log).toEqual(['listen', 'pull(listening)'])
  })

  it('잇기 전에 마운트된 슬롯이 pull 했어도 잇기 직후 pull 이 한 번 더', async () => {
    const { initEventBus, useUsageStore: usage } = await freshModules()
    const rec = recordingClient()
    holder.client = rec.client
    await usage.getState().pull() // 부팅 중 먼저 마운트된 사용량 슬롯
    await initEventBus()
    await flush()
    expect(rec.log).toEqual(['pull(not listening)', 'listen', 'pull(listening)'])
  })

  it('잇기 뒤의 마운트 → pull 정확히 한 번 · 그 pull 보다 방송 리스너가 먼저', async () => {
    const { initEventBus, useUsageStore: usage } = await freshModules()
    const rec = recordingClient()
    holder.client = rec.client
    await initEventBus()
    await flush()
    rec.log.length = 0
    await usage.getState().pull()
    expect(rec.log).toEqual(['pull(listening)'])
  })

  it('방송 → merge 로 스토어에 든다 · pull 반환도 같은 merge 를 지난다', async () => {
    const { initEventBus, useUsageStore: usage } = await freshModules()
    const rec = recordingClient({ socketEpoch: 2, snapshots: [snap('codex', 4)] })
    holder.client = rec.client
    await initEventBus()
    await flush()
    expect(usage.getState().vendors.codex?.revision).toBe(4)
    rec.broadcast(snap('claude', 3), 2)
    rec.broadcast(snap('codex', 2), 2) // 더 작은 revision — 버림
    expect(usage.getState().vendors.claude?.revision).toBe(3)
    expect(usage.getState().vendors.codex?.revision).toBe(4)
  })

  it('관심 보고 표면이 없다 — 잇기·pull·⟳·방송 어디서도 invoke 0 · 사용량 표면은 셋뿐', async () => {
    const { initEventBus, useUsageStore: usage } = await freshModules()
    const rec = recordingClient()
    holder.client = rec.client
    await initEventBus()
    await usage.getState().pull()
    await usage.getState().refresh('claude')
    rec.broadcast(snap('claude', 1), 1)
    await flush()
    expect(invokeSpy).not.toHaveBeenCalled()
    const usageSurface = [...holder.accessed].filter(p => /usage/i.test(p)).sort()
    expect(usageSurface).toEqual(['getUsageSnapshot', 'onUsageLimitsUpdated', 'refreshUsageLimits'])
  })

  // 재연결 pull — 부팅 pull 이 재시도를 다 쓰고 실패했거나, 끊긴 동안 놓친 방송을 메운다(메인 결정 2026-09-29).
  function reconnectableClient(reply: UsageSnapshotPull = { socketEpoch: 1, snapshots: [] }) {
    const rec = recordingClient(reply)
    let stateCb: ((s: ConnectionState) => void) | null = null
    const client = {
      ...rec.client,
      getAgents: vi.fn(async () => []),
      listProfiles: vi.fn(async () => []),
      listPresets: vi.fn(async () => []),
      onConnectionStateChange: (cb: (s: ConnectionState) => void) => {
        stateCb = cb
        cb('connected')
        return () => {}
      },
    }
    return { ...rec, client, setState: (s: ConnectionState) => stateCb?.(s) }
  }

  it('connected 로 재전이 → pull 한 번 더(방송 리스너가 선 채) · 첫 connected · 끊김 전이는 더하지 않는다', async () => {
    const { initEventBus } = await freshModules()
    const rec = reconnectableClient()
    holder.client = rec.client
    await initEventBus()
    await flush()
    expect(rec.log).toEqual(['listen', 'pull(listening)'])
    rec.setState('reconnecting')
    await flush()
    expect(rec.log).toEqual(['listen', 'pull(listening)'])
    rec.setState('connected')
    await flush()
    expect(rec.log).toEqual(['listen', 'pull(listening)', 'pull(listening)'])
  })

  it('재연결 pull 의 반환도 merge 를 지난다 — 끊긴 동안 놓친 값이 든다', async () => {
    const { initEventBus, useUsageStore: usage } = await freshModules()
    const reply: UsageSnapshotPull = { socketEpoch: 1, snapshots: [] }
    const rec = reconnectableClient(reply)
    holder.client = rec.client
    await initEventBus()
    await flush()
    expect(usage.getState().vendors.claude).toBeUndefined()
    rec.setState('down')
    reply.socketEpoch = 2
    reply.snapshots = [snap('claude', 7)]
    rec.setState('connected')
    await flush()
    expect(usage.getState().vendors.claude?.revision).toBe(7)
    expect(usage.getState().socketEpoch).toBe(2)
  })

  // ── 실 ProtocolClient 를 거쳐 — 끊김 전이 · Ack ──
  class FakeTransport implements Transport {
    private _state: ConnectionState = 'connected'
    private stateCbs = new Set<(s: ConnectionState) => void>()
    private msgCb: ((m: InboundMessage) => void) | null = null
    sent: unknown[] = []
    usagePull: UsageSnapshotPull = { socketEpoch: 0, snapshots: [] }

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
      return Promise.resolve()
    }
    start(): Promise<void> {
      return Promise.resolve()
    }
    close(): void {}
    requestReplay(): Promise<bigint> {
      return Promise.resolve(1n)
    }
    getUsageSnapshot(): Promise<UsageSnapshotPull> {
      return Promise.resolve(this.usagePull)
    }
    setState(s: ConnectionState): void {
      this._state = s
      for (const cb of this.stateCbs) cb(s)
    }
    control(event: Record<string, unknown>): void {
      this.msgCb?.({ kind: 'control', event })
    }
    usage(snapshot: UsageLimitSnapshot, socketEpoch: number): void {
      this.control({ UsageLimitsUpdated: { snapshot, socketEpoch } })
    }
  }

  async function wiredToProtocolClient() {
    const modules = await freshModules()
    const transport = new FakeTransport()
    holder.client = new ProtocolClient(transport) as unknown as Record<string, unknown>
    await modules.initEventBus()
    await flush()
    return { ...modules, transport }
  }

  it('⟳ 답 Ack → pending 만 푼다(값 · revision · socketEpoch 그대로)', async () => {
    const { useUsageStore: usage, transport } = await wiredToProtocolClient()
    transport.usage(snap('claude', 5), 2)
    const vendorsBefore = usage.getState().vendors
    const refreshing = usage.getState().refresh('claude')
    await flush()
    expect(usage.getState().pending.claude).toBe(1)
    const sent = transport.sent as Array<{ RefreshUsageLimits?: { vendor: string; request_id: string } }>
    expect(sent).toHaveLength(1)
    expect(sent[0].RefreshUsageLimits?.vendor).toBe('claude')
    transport.control({ Ack: { request_id: sent[0].RefreshUsageLimits!.request_id } })
    await refreshing
    expect(usage.getState().pending.claude).toBe(0)
    expect(usage.getState().vendors).toBe(vendorsBefore)
    expect(usage.getState().socketEpoch).toBe(2)
  })

  it('끊김 전이 → pending 만 푼다 · 늦게 닿은 전이는 revision 을 잊지 않는다(같은 표식의 늦은 옛 사본 버림)', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const { useUsageStore: usage, transport } = await wiredToProtocolClient()
    transport.usage(snap('claude', 50), 1)
    transport.usage(snap('claude', 3), 2) // 새 소켓 표식의 payload rev N=3
    const refreshing = usage.getState().refresh('claude')
    await flush()
    transport.setState('reconnecting') // 그 뒤 늦게 닿은 앞선 끊김의 전이
    await refreshing
    expect(usage.getState().pending.claude).toBe(0)
    expect(usage.getState().vendors.claude?.revision).toBe(3)
    expect(usage.getState().vendors.claude?.snapshot.five_hour?.used_pct).toBe(3)
    transport.usage(snap('claude', 2), 2) // 같은 표식의 늦게 닿은 rev < N
    expect(usage.getState().vendors.claude?.revision).toBe(3)
    expect(usage.getState().socketEpoch).toBe(2)
  })
})
