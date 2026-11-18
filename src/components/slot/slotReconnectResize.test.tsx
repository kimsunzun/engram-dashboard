// Regression for PTY size re-propagation after reconnect — once the socket is up, re-emit the resize dropped while disconnected.
// Why (ADR-0195): a command arriving with no socket fails on the spot (not queued); resize is fire-and-forget, so the failure is swallowed and the PTY stays stuck at its pre-disconnect size (reconnect window 500ms~10s; a divider drag in between never reaches the agent).
// ★Two opposite sides guarded★: ① mixing connection state into the subscription trigger (viewId·agentId deps) would split the subscription on every reconnect — screen wiped before the replay and incarnation marker lost (ADR-0164) — so we also pin that re-propagation happened **without resubscribing**. ② An implementation firing on the **value**, not the transition, overlaps the subscription path at mount and agentId swap — the 'mount' · 'agentId swap' · StrictMode cases below fail on it.

import { StrictMode } from 'react'
import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { OutputChunk, ViewPhase } from '../../api/agentClient'

// Two observers jsdom lacks — no-ops that never fire, leaving only the connection path (no mixing with size-change / visibility propagation).
globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver
globalThis.IntersectionObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return []
  }
} as unknown as typeof IntersectionObserver

// ── Layout stand-in ───────────────────────────────────────────────────────────────────
// jsdom returns offsetParent **always** null and getBoundingClientRect **always** 0 — exactly the slot's two hide-decision branches (ADR-0056), so untouched the visible-slot branch can never run. ★Plant it on the prototype★: attached after render() the slot is still hidden **at the mount commit**, so an implementation firing at mount would be caught by the guard and silently pass.
const layout = vi.hoisted(() => ({ visible: true, width: 800, height: 400 }))
Object.defineProperty(HTMLElement.prototype, 'offsetParent', {
  configurable: true,
  get(): Element | null {
    return layout.visible ? document.body : null
  },
})
HTMLElement.prototype.getBoundingClientRect = function (): DOMRect {
  return {
    width: layout.width,
    height: layout.height,
    top: 0,
    left: 0,
    right: layout.width,
    bottom: layout.height,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  } as DOMRect
}

const captured = vi.hoisted(() => ({
  onChunk: null as ((c: OutputChunk) => void) | null,
  onState: null as ((s: ViewPhase) => void) | null,
}))

// Connection-state surface — holds subscribers like the real one and notifies them on transitions (including one notification right at registration).
const conn = vi.hoisted(() => ({
  state: 'connected' as 'connected' | 'reconnecting' | 'down',
  cbs: new Set<(s: string) => void>(),
}))

vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    subscribeOutput: vi.fn(
      async (
        _viewId: string,
        _agentId: string,
        onChunk: (c: OutputChunk) => void,
        onState?: (s: ViewPhase) => void,
      ) => {
        captured.onChunk = onChunk
        captured.onState = onState ?? null
        return { unsubscribe: vi.fn() }
      },
    ),
    writeStdin: vi.fn(async () => undefined),
    resizePty: vi.fn(async () => undefined),
    get connectionState() {
      return conn.state
    },
    onConnectionStateChange: (cb: (s: string) => void) => {
      conn.cbs.add(cb)
      cb(conn.state)
      return () => conn.cbs.delete(cb)
    },
  },
  getAgentClient: vi.fn(),
}))

const agentStoreState = vi.hoisted(() => ({ agents: [] as unknown[], agentsLoaded: false }))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
}))

// Size read through a holder — to tell by value whether it was read after fit() reflected a measurement. Initial 80×24 = xterm constructor default (must equal the real one — see the fit mock comment below).
const xtermDims = vi.hoisted(() => ({ cols: 80, rows: 24 }))
vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    loadAddon = vi.fn()
    open = vi.fn()
    reset = vi.fn()
    write = vi.fn()
    onData = vi.fn(() => ({ dispose: vi.fn() }))
    dispose = vi.fn()
    refresh = vi.fn()
    get cols() {
      return xtermDims.cols
    }
    get rows() {
      return xtermDims.rows
    }
  },
}))
const fitAddonFit = vi.hoisted(() => vi.fn())
vi.mock('@xterm/addon-fit', () => ({ FitAddon: class { fit = fitAddonFit } }))
vi.mock('@xterm/addon-webgl', () => ({
  WebglAddon: class {
    onContextLoss = vi.fn()
    dispose = vi.fn()
  },
}))
vi.mock('@xterm/xterm/css/xterm.css', () => ({}))

import TerminalSlot from './TerminalSlot'

const AGENT = 'aaaa-bbbb-cccc-dddd'
const OTHER_AGENT = 'eeee-ffff-0000-1111'
/** The size of a "measured live slot" in this harness. */
const measured = { cols: 137, rows: 25 }

async function flushSubscribe(): Promise<void> {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

/** Connection-state transition (same shape as the path where the real ProtocolClient notifies all subscribers). */
function setConnection(state: 'connected' | 'reconnecting' | 'down'): void {
  act(() => {
    conn.state = state
    for (const cb of conn.cbs) cb(state)
  })
}

function resizeCalls(client: { resizePty: unknown }): ReturnType<typeof vi.fn> {
  return client.resizePty as ReturnType<typeof vi.fn>
}

function subscribeCalls(client: { subscribeOutput: unknown }): number {
  return (client.subscribeOutput as ReturnType<typeof vi.fn>).mock.calls.length
}

beforeEach(() => {
  captured.onChunk = null
  captured.onState = null
  conn.state = 'connected'
  conn.cbs.clear()
  layout.visible = true
  layout.width = 800
  layout.height = 400
  xtermDims.cols = 80
  xtermDims.rows = 24
  // ★Mock of the fit contract (checked against the `@xterm/addon-fit` source)★: `proposeDimensions()` returns undefined only with no terminal · no `element.parentElement` · **cell metrics 0**. ★The box size is not among them★: xterm measures metrics from the font, so a collapsed box gives a negative width clamped to the floor 2×1 and **a real resize is issued**. The no-op in the display:none branch is the mock's placeholder — the real one is not a no-op (measured 11×6 — TerminalSlot helper comment). A mock that breaks this contract invents values the library cannot produce, and a guard built on it can never fire.
  fitAddonFit.mockReset()
  fitAddonFit.mockImplementation(() => {
    if (!layout.visible) return
    if (layout.width === 0 || layout.height === 0) {
      xtermDims.cols = 2
      xtermDims.rows = 1
      return
    }
    xtermDims.cols = Math.max(2, measured.cols)
    xtermDims.rows = Math.max(1, measured.rows)
  })
  agentStoreState.agents = [
    { id: AGENT, cwd: 'C:/x', status: { type: 'Running' }, epoch: 7 },
    { id: OTHER_AGENT, cwd: 'C:/y', status: { type: 'Running' }, epoch: 8 },
  ]
  agentStoreState.agentsLoaded = true
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('TerminalSlot — PTY size repropagation after reconnect', () => {
  it('after disconnecting and reattaching it pushes the current terminal size to the PTY again', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    const subCalls = subscribeCalls(agentClient)
    fitAddonFit.mockClear()
    resizeCalls(agentClient).mockClear()

    setConnection('reconnecting')
    setConnection('connected')

    // Reading the size before fit yields the construction default (80×24), not the measured value.
    expect(fitAddonFit).toHaveBeenCalled()
    expect(agentClient.resizePty).toHaveBeenCalledWith(AGENT, 137, 25)
    // ★Without resubscribing★ — if the subscription is split, the screen is wiped before the replay arrives (ADR-0164).
    expect(subscribeCalls(agentClient)).toBe(subCalls)
  })

  it('while disconnected it sends nothing, and at the moment of reattaching it sends exactly once', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    resizeCalls(agentClient).mockClear()

    setConnection('reconnecting')
    setConnection('down')
    expect(agentClient.resizePty).not.toHaveBeenCalled()

    setConnection('connected')
    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)
  })

  // ★Only the agentId swap block below tells value-watching from transition-watching★: re-notifying the same state is no trigger (setState filters the same value, so no render and deps stay the same — a value-watching implementation does not fire there either); the re-notify · re-render assertions record that contract. ★Deleting the swap block as "duplicate" leaves this test unable to tell anything apart★.
  it('while attached it does not fire — re-notify · re-render · agentId swap are none of them a trigger', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    const { rerender } = render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    resizeCalls(agentClient).mockClear()

    setConnection('connected')
    rerender(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    expect(agentClient.resizePty).not.toHaveBeenCalled()

    // The new agent's initial one-time emit belongs to the subscription .then — emitting here too would overlap it.
    rerender(<TerminalSlot viewId="v1" agentId={OTHER_AGENT} />)
    await flushSubscribe()
    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)
    expect(agentClient.resizePty).toHaveBeenCalledWith(OTHER_AGENT, 137, 25)
  })

  it('mounting attached gives only the one subscription path — this path does not fire twice', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)
    expect(agentClient.resizePty).toHaveBeenCalledWith(AGENT, 137, 25)
  })

  // The app starts under StrictMode (main.tsx) — the dev double mount runs the effect twice; the transition baseline is a ref, so the second run must read as "already attached". ★Without the mount share, measuring only reconnect loses this claim★ — set up the mount assertion first.
  it('even under a StrictMode double mount it is mount 0 · reconnect 1', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(
      <StrictMode>
        <TerminalSlot viewId="v1" agentId={AGENT} />
      </StrictMode>,
    )
    await flushSubscribe()

    // The double mount subscribes twice but only the surviving subscription emits a size in .then (the earlier is dropped by the cancelled guard) — total 1 only if this path emits nothing at mount.
    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)

    resizeCalls(agentClient).mockClear()
    setConnection('reconnecting')
    setConnection('connected')
    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)
  })

  // Hidden slot (tab keep-alive = display:none). When shown again, the visibility effect emits the same propagation.
  it('a hidden slot does not propagate the size even on reconnect', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    layout.visible = false
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    fitAddonFit.mockClear()
    resizeCalls(agentClient).mockClear()

    setConnection('down')
    setConnection('connected')

    expect(fitAddonFit).not.toHaveBeenCalled()
    expect(agentClient.resizePty).not.toHaveBeenCalled()
  })

  // ★Second branch of the hide decision★: a displayed slot (offsetParent alive) whose box collapsed to 0. fit() is not skipped here (cell metrics are fine), so the negative width clamps to the floor 2×1 and a real resize goes out — an offsetParent-only guard would send 2×1 to the PTY. ★A size check cannot block it★ (2×1 looks like a normal size); the "when to send" side must drop out before fit(), so fit() not being called is asserted too.
  it('a slot that was shown but whose box collapsed drops out before reaching fit()', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    layout.width = 0
    layout.height = 0
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    fitAddonFit.mockClear()
    resizeCalls(agentClient).mockClear()

    setConnection('down')
    setConnection('connected')

    expect(fitAddonFit).not.toHaveBeenCalled()
    expect(agentClient.resizePty).not.toHaveBeenCalled()
  })
})
