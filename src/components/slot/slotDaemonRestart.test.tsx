// How a daemon restart treats the slot screen — a regression running **through the real ProtocolClient** to the real slot.
// ★Why onReset is not fired by hand★: a hand-fired clear signal measures only "does the slot react to the signal", not **does the signal arrive**. That is how the defect hid — the slot reacted well, but the parent passed the epoch down as a prop, so **before the signal came** the subscription was swapped wholesale and the screen erased (that swap also lost the incarnation marker so the rotation verdict could not stand), and the signal never came. Measuring that span needs the chain parent renderer → subscription → transport as one line.
// Pinned: ① a daemon restart does not erase the slot **before the replacement replay arrives**. ② If that replay ends in failure the previous incarnation's screen **stays** (only the absence mark is laid on it).
// Observation slot = DomSlot — draws the same stream as plain <pre>, read via textContent (the terminal is a canvas).

import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { ConnectionState } from '../../api/agentClient'
import type { InboundMessage, Transport } from '../../api/transport'
import { ProtocolClient } from '../../api/protocolClient'
import type { AgentInfo, Capabilities } from '../../api/types'
import type { LayoutNode } from '../../api/layoutTypes'

// ── Tauri stub — viewStore (the layout authority mirror) hooks invoke/listen at boot. ──
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async () => undefined),
  Channel: class {
    onmessage: unknown = null
  },
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => vi.fn()) }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ label: 'main' }),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(async () => null) }))

// ── agentClient = **the real ProtocolClient** (made fresh per test) ──────────────────────
// Wrapped in a Proxy so every access goes to the instance of that moment — the mock factory is hoisted and runs before the instance.
const holder = vi.hoisted(() => ({ client: null as unknown as Record<string, unknown> }))
vi.mock('../../api/clientFactory', () => ({
  agentClient: new Proxy(
    {},
    {
      get(_t, prop: string) {
        const c = holder.client as unknown as Record<string, unknown>
        const v = c[prop]
        return typeof v === 'function' ? (v as (...a: unknown[]) => unknown).bind(c) : v
      },
    },
  ),
  getAgentClient: vi.fn(),
}))

// ── agentStore stub — the parent renderer's three-way gate (ADR-0148/0149) looks here. ──
const agentStoreState = vi.hoisted(() => ({
  agents: [] as unknown[],
  agentsLoaded: false,
  profiles: [] as unknown[],
  profilesLoaded: false,
  presets: [] as unknown[],
}))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: Object.assign(
    (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
    { getState: () => agentStoreState },
  ),
}))

// Heavy siblings unrelated to the slot branch (tree · palette) are not stood up — this test's node is a single agent slot.
vi.mock('../agent/AgentList', () => ({ default: () => <div /> }))
vi.mock('./PresetPalette', () => ({ default: () => <div /> }))

import ViewLayoutRenderer from '../layout/ViewLayoutRenderer'
import { useViewStore } from '../../store/viewStore'

// ── Minimal Transport — the test pushes frames, markers and connection state directly. ─────────────────────
class FakeTransport implements Transport {
  private _state: ConnectionState = 'connected'
  private stateCbs = new Set<(s: ConnectionState) => void>()
  private msgCb: ((m: InboundMessage) => void) | null = null
  replayCalls: bigint[] = []
  private genCounter = 0n

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
  send(): void {}
  ensureReady(): Promise<void> {
    return Promise.resolve()
  }
  start(): Promise<void> {
    return Promise.resolve()
  }
  close(): void {}
  requestReplay(): Promise<bigint> {
    const gen = ++this.genCounter
    this.replayCalls.push(gen)
    return Promise.resolve(gen)
  }
  getUsageSnapshot(): Promise<{ socketEpoch: number; snapshots: [] }> {
    return Promise.resolve({ socketEpoch: 0, snapshots: [] })
  }

  // ── Drive ──
  setState(s: ConnectionState): void {
    this._state = s
    act(() => {
      for (const cb of this.stateCbs) cb(s)
    })
  }
  roster(agents: Array<{ id: string; epoch?: number }>): void {
    act(() => this.msgCb?.({ kind: 'control', event: { AgentListUpdated: { agents } } }))
  }
  output(epoch: number, seq: number, text: string): void {
    act(() =>
      this.msgCb?.({
        kind: 'output',
        tag: 0,
        agentId: AGENT,
        epoch,
        seq,
        bytes: new TextEncoder().encode(text),
      }),
    )
  }
  marker(epoch: number, gen: bigint, failed = false): void {
    act(() =>
      this.msgCb?.({
        kind: 'replayBoundary',
        agentId: AGENT,
        epoch,
        gen,
        truncated: false,
        failed,
        continuesConversation: false,
        replayFrom: 0,
      }),
    )
  }
  get lastGen(): bigint {
    return this.replayCalls[this.replayCalls.length - 1]
  }
}

const AGENT = 'aaaa-bbbb-cccc-dddd'
const SLOT = 's1'

function caps(): Capabilities {
  return {
    input: { raw: true, message: false, attachment: false },
    output: { terminal_bytes: true, structured: false, markdown: false, tool_events: false, usage: false },
    control: { resize: true, interrupt: true, cancel: false, graceful_shutdown: true },
    session: { resume: true, snapshot: false, cwd_env: true },
    model: { select: false, temperature: false, max_tokens: false },
  }
}

function agentInfo(epoch: number): AgentInfo {
  return {
    id: AGENT,
    name: AGENT,
    cwd: '/tmp',
    status: { type: 'Running' },
    cols: 80,
    rows: 24,
    epoch,
    capabilities: caps(),
  }
}

/**
 * ★The profile's epoch is not a live value★: the backend no longer updates it. The parent renderer once passed it down as a slot prop, so on a daemon restart the value dropped and resubscribe and screen erasure ran. It is seeded deliberately mismatched with the real value so a revival of that path shows at once.
 */
function profile(): unknown {
  return { id: AGENT, name: AGENT, cwd: '/tmp', display_name: null, parent_id: null, created_at: 0, epoch: 0 }
}

const node: LayoutNode = { type: 'slot', id: SLOT, content: { type: 'agent', agent_id: AGENT } }

function domText(): string {
  return (document.querySelector('[data-dom-mode="1"]') as HTMLElement | null)?.textContent ?? ''
}

function veil(): HTMLElement | null {
  return document.querySelector('[data-slot-dead="1"]')
}

async function flush(): Promise<void> {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

let t: FakeTransport
let client: ProtocolClient

beforeEach(() => {
  t = new FakeTransport()
  client = new ProtocolClient(t)
  holder.client = client as unknown as Record<string, unknown>
  agentStoreState.agents = [agentInfo(7)]
  agentStoreState.agentsLoaded = true
  agentStoreState.profiles = [profile()]
  agentStoreState.profilesLoaded = true
  // Pinned to the observable renderer (DomSlot) — draws the same stream as plain text (§5 observation means).
  useViewStore.setState({ renderModeOverride: { [SLOT]: 'dom' } })
})

afterEach(() => {
  cleanup()
  client.close()
  useViewStore.setState({ renderModeOverride: {} })
  vi.restoreAllMocks()
})

/** Advance to the state where the first incarnation (marker 7)'s history is drawn. */
async function mountWithHistory() {
  const view = render(<ViewLayoutRenderer node={node} focusedSlotId={null} />)
  await flush()
  t.output(7, 0, 'run-one output')
  t.marker(7, t.lastGen)
  expect(domText()).toBe('run-one output')
  return view
}

/** The span where the daemon goes down and that agent vanishes from the roster (during the restart). */
function daemonGoesDown(rerender: (ui: React.ReactElement) => void): void {
  t.setState('reconnecting')
  agentStoreState.agents = [] // the reaper reaps it = it vanishes from the roster
  act(() => rerender(<ViewLayoutRenderer node={node} focusedSlotId={null} />))
}

describe('daemon restart — the slot is not erased before the replacement replay arrives', () => {
  it('disconnection + vanishing from the roster alone leaves the screen, and only the absence mark is laid on', async () => {
    const { rerender } = await mountWithHistory()

    daemonGoesDown(rerender)

    // ★headline★ nothing has come yet — there is no ground to erase.
    expect(domText()).toBe('run-one output')
    expect(veil()).not.toBeNull()
  })

  it('even if the attached replay ends in failure the previous incarnation\'s screen stays as is', async () => {
    vi.useFakeTimers()
    try {
      const { rerender } = await mountWithHistory()
      daemonGoesDown(rerender)

      // The socket stands again, and the roster speaks of that agent with a **different marker** (re-spawn).
      t.setState('connected')
      t.roster([{ id: AGENT, epoch: 8 }])
      await act(async () => {
        await Promise.resolve()
      })

      // Exhaust the ladder to the end — a failure at any step does not empty the screen.
      for (let i = 0; i < 4; i++) {
        t.marker(8, t.lastGen, /*failed*/ true)
        await act(async () => {
          await vi.advanceTimersByTimeAsync(5000)
        })
      }

      expect(client.getViewOutputState(SLOT)?.phase).toBe('error')
      expect(domText()).toBe('run-one output')
      // ★The exhausted ladder also remains on the surface★: without this it becomes an empty plate with neither signal nor recovery path.
      expect(veil()).not.toBeNull()
    } finally {
      vi.useRealTimers()
    }
  })

  it('when the new incarnation\'s replay arrives it is swapped in all at once then', async () => {
    const { rerender } = await mountWithHistory()
    daemonGoesDown(rerender)

    t.setState('connected')
    t.roster([{ id: AGENT, epoch: 8 }])
    await act(async () => {
      await Promise.resolve()
    })
    // Attaching alone is still as is (no clearing at request time).
    expect(domText()).toBe('run-one output')

    // The new incarnation really came — the roster and store are also updated that way.
    agentStoreState.agents = [agentInfo(8)]
    t.output(8, 0, 'run-two output') // numbering restarts from 0
    t.marker(8, t.lastGen)
    act(() => rerender(<ViewLayoutRenderer node={node} focusedSlotId={null} />))

    // Neither stacked on top (leftover of the previous incarnation) nor dropped entirely (empty screen) — it remains as a single set only.
    expect(domText()).toBe('run-two output')
    expect(veil()).toBeNull()
    expect(client.getViewOutputState(SLOT)?.phase).toBe('live')
  })

  // ★A re-spawn where the socket never dropped★: for the slot nothing seems to have happened, so roster observation is the only place to notice the rotation; without this branch an open slot cannot draw a byte after the re-spawn.
  it('even without a disconnect the roster\'s new marker reaches the open slot', async () => {
    await mountWithHistory()

    t.roster([{ id: AGENT, epoch: 9 }])
    await act(async () => {
      await Promise.resolve()
    })
    t.output(9, 0, 'respawned output')
    t.marker(9, t.lastGen)

    expect(domText()).toBe('respawned output')
  })
})
