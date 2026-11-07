// Regression for PTY size re-propagation after reconnect — once the socket is up, re-emit the resize that was dropped while disconnected.
//
// Background (ADR-0195): a command that arrives while there is no socket is not queued but fails on the spot — a decision meant to keep commands
//   from crossing the connection boundary. resize is fire-and-forget, so that failure is silently swallowed, nobody re-emits it, and the PTY
//   stays stuck at its pre-disconnect size (reconnect window = 500ms~10s. If the divider is dragged in between,
//   that size never reaches the agent).
//
// ★The two opposite sides this regression guards★
//   ① Fixing the same symptom by mixing connection state into the subscription trigger (viewId·agentId deps) splits the subscription at every reconnect,
//      so the screen is wiped before the replay arrives and the incarnation marker is lost too (ADR-0164) — so we also pin that the re-propagation
//      happened **without resubscribing**.
//   ② An implementation that fires on the **value**, not the transition (fires whenever the effect runs while connection state is true)
//      fires overlapping the subscription path at mount and agentId swap — the 'mount' · 'agentId swap' · StrictMode cases below are
//      built to fail on that implementation.

import { StrictMode } from 'react'
import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { OutputChunk, ViewPhase } from '../../api/agentClient'

// Two observers jsdom does not provide — no-ops that never fire the callback, so only the connection path remains (so they do not mix with the propagation that size changes ·
//   visibility transitions produce).
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
// jsdom has no layout, so it returns offsetParent **always** null and getBoundingClientRect **always** 0. Those two are exactly the slot's two
//   hide-decision branches (ADR-0056), so untouched, the visible-slot branch can never be stepped on in this
//   harness. ★Why it is planted on the prototype★: if attached after render(), it is still hidden **at the mount commit**,
//   so even an implementation that fires overlapping at mount is caught by the guard and silently passes (that trap was this file's
//   earlier edition).
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

// Size is read through a holder — to tell by value whether it was read after fit() actually reflected a measurement.
//   Initial value 80×24 = xterm constructor default (must be the same value as the real one — see the fit mock comment below).
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
  // ★This is the mock of the fit contract (checked against the `@xterm/addon-fit` source)★. `proposeDimensions()` falls through to undefined
  //   under only three conditions — no terminal · no `element.parentElement` · **cell metrics are 0**.
  //   ★The container box size is not among the three★: metrics are measured by xterm from the font, so they stay alive
  //   regardless of the box, and a collapsed box yields a negative width, clamped to the `Math.max(2,·)`/`Math.max(1,·)` floor 2×1, and **a real resize
  //   is issued**. The no-op in the display:none branch is the mock's placeholder — the real one is not a no-op (measured 11×6 — TerminalSlot helper comment).
  //   If the mock breaks this contract the test invents values the library cannot produce, and a guard built on that becomes code that can
  //   never fire — the earlier edition of this file was exactly that.
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

describe('TerminalSlot — 재연결 뒤 PTY 치수 재전파', () => {
  it('끊겼다 다시 붙으면 지금 터미널 치수를 PTY 에 다시 밀어 넣는다', async () => {
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

  it('끊긴 동안에는 아무것도 보내지 않고, 다시 붙는 순간 정확히 한 번 보낸다', async () => {
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

  // ★What tells value-watching from transition-watching is the agentId swap block below alone★. Re-notifying the same state is not a
  //   trigger — setState filters the same value so no render happens and deps stay the same, so an implementation watching only the value does not fire there either.
  //   The earlier re-notify · re-render assertions record that contract, and the power to tell implementations apart lies only in the swap block
  //   — ★if that block is deleted as 「중복」 (duplicate) this test can tell nothing apart★.
  it('붙어 있는 동안에는 발화하지 않는다 — 재통지·리렌더·agentId 교체 어느 것도 계기가 아니다', async () => {
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

  it('붙은 채 마운트하면 구독 경로 1회뿐 — 이 경로가 겹쳐 쏘지 않는다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)
    expect(agentClient.resizePty).toHaveBeenCalledWith(AGENT, 137, 25)
  })

  // The app starts under StrictMode (main.tsx) — the dev double mount runs the effect twice. The transition baseline is a ref, so
  //   the second run must read as "already attached". ★If you strip out the mount share and measure only reconnect, that claim
  //   is lost★ — set up the mount assertion first.
  it('StrictMode 이중 마운트에서도 마운트 0 · 재연결 1 이다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(
      <StrictMode>
        <TerminalSlot viewId="v1" agentId={AGENT} />
      </StrictMode>,
    )
    await flushSubscribe()

    // Because of the double mount the subscription is made twice, but only the surviving subscription emits a size in .then (the earlier one
    //   is dropped by the cancelled guard). The total is 1 only if this path emits nothing at mount.
    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)

    resizeCalls(agentClient).mockClear()
    setConnection('reconnecting')
    setConnection('connected')
    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)
  })

  // Hidden slot (tab keep-alive = display:none). When shown again, the visibility effect emits the same propagation.
  it('숨은 슬롯은 재연결에서도 치수를 전파하지 않는다', async () => {
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

  // ★Second branch of the hide decision★: a slot that is displayed (offsetParent alive) but whose box alone collapsed to 0.
  //   Here fit() is not skipped — cell metrics are fine, so the negative width is clamped to the Math.max floor 2×1 and a real resize
  //   is issued at that size. If the guard had only the offsetParent branch, that 2×1 would go out to the PTY.
  //   ★Checking the size cannot block this case★ (2×1 is indistinguishable from a normal size) — what blocks it is
  //   the "when to send" side, so it must drop out before reaching fit(). So the assertion hangs on fit() not being called too.
  it('표시됐지만 박스가 붕괴한 슬롯은 fit() 에 닿기 전에 빠진다', async () => {
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
