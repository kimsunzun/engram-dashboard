// usageStore — receiving usage values (TRD S21 usage-limit-slot §4 rows 「frontend value receipt」·「frontend store · broadcast」).
//
// Measures three layers:
//   ① the rules of `merge` (revision · socketEpoch) — the store alone.
//   ② pull · ⟳ — a mock agentClient (whether the store calls nothing but `agentClient` · 0 `invoke`).
//   ③ eventBus hookup — the order of hookup and pull (recording mock) · disconnect·Ack through the real ProtocolClient (value · revision unchanged).
// The label filter (only broadcasts addressed to its own window) is the carrier's job, so `api/tauriTransport.test.ts` measures it.

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

// agentClient = an object the test plants. The accessed names are gathered to assert 「the surface the usage path calls」.
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

// The layout subscription and boot pull of eventBus are unrelated to this test (they call Tauri).
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

// ── ① merge — revision (same socket) ─────────────────────────────────────────────────────
describe('merge — revision', () => {
  it('drops a smaller revision and takes a larger one', () => {
    store().merge(snap('claude', 5), 1)
    store().merge(snap('claude', 3), 1)
    expect(revisionOf('claude')).toBe(5)
    expect(usedOf('claude')).toBe(5)
    store().merge(snap('claude', 7), 1)
    expect(revisionOf('claude')).toBe(7)
    expect(usedOf('claude')).toBe(7)
  })

  it('for the same revision it takes the copy that arrived late — the relative time basis (receivedAt) is set anew', () => {
    const now = vi.spyOn(performance, 'now').mockReturnValue(1_000)
    store().merge(snap('claude', 5, 40, 30), 1)
    now.mockReturnValue(2_000)
    store().merge(snap('claude', 5, 40, 2), 1)
    const held = store().vendors.claude!
    expect(held.receivedAt).toBe(2_000)
    expect(held.snapshot.five_hour?.age_secs).toBe(2)
  })

  it('revisions are independent between vendors', () => {
    store().merge(snap('claude', 10), 1)
    store().merge(snap('codex', 1), 1)
    expect(revisionOf('codex')).toBe(1)
    store().merge(snap('claude', 5), 1)
    store().merge(snap('codex', 2), 1)
    expect(revisionOf('claude')).toBe(10)
    expect(revisionOf('codex')).toBe(2)
  })

  it('does not know on/off — every vendor received goes in (filtering is the drawing side\'s job)', () => {
    store().merge(snap('codex', 1), 1)
    store().merge(snap('claude', 1), 1)
    expect(Object.keys(store().vendors).sort()).toEqual(['claude', 'codex'])
  })
})

// ── ① merge — socketEpoch(ADR-0195) ────────────────────────────────────────────────────
describe('merge — socketEpoch', () => {
  it('a larger marker → forgets all revisions and takes a smaller revision (without a connection state transition — generation succession) · the value stays', () => {
    store().merge(snap('claude', 50), 1)
    store().merge(snap('codex', 9), 1)
    store().merge(snap('claude', 1, 77), 2)
    expect(store().socketEpoch).toBe(2)
    expect(revisionOf('claude')).toBe(1)
    expect(usedOf('claude')).toBe(77)
    // A vendor that has not come yet on the new socket: the value stays, only the revision is forgotten → revision 1 of the new socket is taken too.
    expect(usedOf('codex')).toBe(9)
    expect(revisionOf('codex')).toBeNull()
    store().merge(snap('codex', 1, 3), 2)
    expect(revisionOf('codex')).toBe(1)
    expect(usedOf('codex')).toBe(3)
  })

  it('a smaller marker → dropped (from a pushed-out socket)', () => {
    store().merge(snap('claude', 4), 3)
    store().merge(snap('claude', 100), 2)
    expect(store().socketEpoch).toBe(3)
    expect(revisionOf('claude')).toBe(4)
  })

  it('marker 0 → the held marker is not changed and only the revision rule applies', () => {
    store().merge(snap('claude', 5), 4)
    store().merge(snap('claude', 9), 0)
    expect(store().socketEpoch).toBe(4)
    expect(revisionOf('claude')).toBe(9)
    store().merge(snap('claude', 2), 0)
    expect(revisionOf('claude')).toBe(9)
  })

  it('a pull return with marker 0 (the shell has no socket — empty list) → marker and value unchanged', async () => {
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
  it('the returned snapshots go through merge with the returned marker', async () => {
    holder.client = {
      getUsageSnapshot: vi.fn(async () => ({ socketEpoch: 3, snapshots: [snap('claude', 2), snap('codex', 5)] })),
    }
    await store().pull()
    expect(store().socketEpoch).toBe(3)
    expect(revisionOf('claude')).toBe(2)
    expect(revisionOf('codex')).toBe(5)
  })

  it('a pull return that came back late after a broadcast (smaller revision) is dropped', async () => {
    const reply = deferred<UsageSnapshotPull>()
    holder.client = { getUsageSnapshot: vi.fn(() => reply.promise) }
    const pulling = store().pull()
    store().merge(snap('claude', 8), 3) // a broadcast that arrived while the pull was running
    reply.resolve({ socketEpoch: 3, snapshots: [snap('claude', 6)] })
    await pulling
    expect(revisionOf('claude')).toBe(8)
    expect(usedOf('claude')).toBe(8)
  })

  it('failure → bounded retries · even when exhausted it does not throw and keeps the held value', async () => {
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

  it('when it succeeds after a transient failure, it takes that return', async () => {
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
  it('calls refreshUsageLimits(vendor) and is pending only while it runs', async () => {
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

  it('two overlapping ⟳ — pending clears only when both finish', async () => {
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

  it('does not throw even on failure and clears pending', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    holder.client = { refreshUsageLimits: vi.fn(async () => Promise.reject(new Error('connection lost'))) }
    await expect(store().refresh('claude')).resolves.toBeUndefined()
    expect(store().pending.claude).toBe(0)
  })

  it("the store calls only agentClient's two usage methods — 0 invoke", async () => {
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

// ── ③ eventBus hookup ──────────────────────────────────────────────────────────────────
// initEventBus holds a promise that runs only once module-wide, so each test reads the module anew (the store is set up anew too).
describe('eventBus — broadcast hookup first, pull later', () => {
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
      onPeerStateUpdated: off,
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

  it('one pull after the hookup is set', async () => {
    const { initEventBus } = await freshModules()
    const rec = recordingClient()
    holder.client = rec.client
    await initEventBus()
    await flush()
    expect(rec.log).toEqual(['listen', 'pull(listening)'])
  })

  it('even if a slot mounted before the hookup pulled, one more pull right after the hookup', async () => {
    const { initEventBus, useUsageStore: usage } = await freshModules()
    const rec = recordingClient()
    holder.client = rec.client
    await usage.getState().pull() // a usage slot mounted first during boot
    await initEventBus()
    await flush()
    expect(rec.log).toEqual(['pull(not listening)', 'listen', 'pull(listening)'])
  })

  it('a mount after the hookup → exactly one pull · the broadcast listener comes before that pull', async () => {
    const { initEventBus, useUsageStore: usage } = await freshModules()
    const rec = recordingClient()
    holder.client = rec.client
    await initEventBus()
    await flush()
    rec.log.length = 0
    await usage.getState().pull()
    expect(rec.log).toEqual(['pull(listening)'])
  })

  it('broadcast → into the store through merge · the pull return goes through the same merge', async () => {
    const { initEventBus, useUsageStore: usage } = await freshModules()
    const rec = recordingClient({ socketEpoch: 2, snapshots: [snap('codex', 4)] })
    holder.client = rec.client
    await initEventBus()
    await flush()
    expect(usage.getState().vendors.codex?.revision).toBe(4)
    rec.broadcast(snap('claude', 3), 2)
    rec.broadcast(snap('codex', 2), 2) // a smaller revision — dropped
    expect(usage.getState().vendors.claude?.revision).toBe(3)
    expect(usage.getState().vendors.codex?.revision).toBe(4)
  })

  it('there is no interest report surface — 0 invoke anywhere in hookup · pull · ⟳ · broadcast · only three usage surfaces', async () => {
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

  // Reconnect pull — fills the case where the boot pull used up its retries and failed, or broadcasts were missed while disconnected.
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

  it('re-transition to connected → one more pull (with the broadcast listener set) · the first connected · disconnect transitions do not add one', async () => {
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

  it('the reconnect pull return goes through merge too — values missed while disconnected come in', async () => {
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

  // ── through the real ProtocolClient — disconnect transition · Ack ──
  class FakeTransport implements Transport {
    private _state: ConnectionState = 'connected'
    private stateCbs = new Set<(s: ConnectionState) => void>()
    private msgCb: ((m: InboundMessage) => void) | null = null
    sent: unknown[] = []
    usagePull: UsageSnapshotPull = { socketEpoch: 0, snapshots: [] }

    get connectionState(): ConnectionState {
      return this._state
    }
    get connectionBand(): null {
      return null
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

  it('⟳ answer Ack → only clears pending (value · revision · socketEpoch unchanged)', async () => {
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

  it('disconnect transition → only clears pending · a transition arriving late does not forget the revision (a late old copy with the same marker is dropped)', async () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const { useUsageStore: usage, transport } = await wiredToProtocolClient()
    transport.usage(snap('claude', 50), 1)
    transport.usage(snap('claude', 3), 2) // payload rev N=3 with the new socket marker
    const refreshing = usage.getState().refresh('claude')
    await flush()
    transport.setState('reconnecting') // the transition of an earlier disconnect arriving late after that
    await refreshing
    expect(usage.getState().pending.claude).toBe(0)
    expect(usage.getState().vendors.claude?.revision).toBe(3)
    expect(usage.getState().vendors.claude?.snapshot.five_hour?.used_pct).toBe(3)
    transport.usage(snap('claude', 2), 2) // rev < N with the same marker, arriving late
    expect(usage.getState().vendors.claude?.revision).toBe(3)
    expect(usage.getState().socketEpoch).toBe(2)
  })
})
