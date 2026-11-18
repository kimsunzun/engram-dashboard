// WebGL seat visibility-linked lifecycle (ADR-0056) regression — only a visible slot holds a WebGL seat.
// Why: terminal tabs keep their mount (keep-alive, hidden = display:none) and WebView2/Chromium caps concurrently active WebGL contexts (measured 16), so the addon attaches when visible and returns when hidden (loseContext→dispose); the Terminal instance (buffer) always lives.
// ★jsdom has no real WebGL/IntersectionObserver★ — only the logic layer is checked, with a controllable IntersectionObserver mock and xterm Terminal / FitAddon / WebglAddon stubs observed via a static holder.
// loseContext: TerminalSlot scans canvases in the container at attach time and grabs the context via getContext('webgl2'|'webgl') (unmount ref-null defense), so plant a canvas *before* visible and spy on its getContext.

import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { OutputChunk } from '../../api/agentClient'

// jsdom has no ResizeObserver — the callback is captured so the Fix 3 check (skip fit/resize while hidden) can fire it.
type ROEntry = { contentRect: { width: number; height: number } }
const roState = vi.hoisted(() => ({
  instances: [] as Array<{ cb: (entries: ROEntry[]) => void }>,
}))
globalThis.ResizeObserver = class {
  cb: (entries: ROEntry[]) => void
  constructor(cb: (entries: ROEntry[]) => void) {
    this.cb = cb
    roState.instances.push(this)
  }
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver

function fireResize(width: number, height: number): void {
  const ro = roState.instances[roState.instances.length - 1]
  if (!ro) throw new Error('no ResizeObserver instance')
  act(() => ro.cb([{ contentRect: { width, height } }]))
}

// ── Controllable IntersectionObserver mock (a real IO fires on observation and on each transition; the test drives firing directly) ──
type IOEntry = { isIntersecting: boolean; intersectionRatio: number }
const ioState = vi.hoisted(() => ({
  instances: [] as Array<{ cb: (entries: IOEntry[]) => void; disconnected: boolean }>,
}))
class MockIntersectionObserver {
  cb: (entries: IOEntry[]) => void
  disconnected = false
  constructor(cb: (entries: IOEntry[]) => void) {
    this.cb = cb
    ioState.instances.push(this)
  }
  observe() {}
  unobserve() {}
  disconnect() {
    this.disconnected = true
  }
  takeRecords() {
    return []
  }
}
globalThis.IntersectionObserver = MockIntersectionObserver as unknown as typeof IntersectionObserver

function fireVisibility(visible: boolean): void {
  const io = ioState.instances[ioState.instances.length - 1]
  if (!io) throw new Error('no IntersectionObserver instance')
  act(() => io.cb([{ isIntersecting: visible, intersectionRatio: visible ? 1 : 0 }]))
}

// ── xterm/addon stub — a WebglAddon is created anew on every attach, so creation list · dispose · onContextLoss and the shared call log (order) live in a hoisted holder. ──
const order = vi.hoisted(() => ({ log: [] as string[] }))
const webglState = vi.hoisted(() => ({
  instances: [] as Array<{ dispose: ReturnType<typeof vi.fn>; onContextLoss: ReturnType<typeof vi.fn> }>,
}))
vi.mock('@xterm/addon-webgl', () => ({
  WebglAddon: class {
    onContextLoss = vi.fn()
    dispose = vi.fn(() => {
      order.log.push('dispose')
    })
    constructor() {
      webglState.instances.push(this)
    }
  },
}))

const fitState = vi.hoisted(() => ({ fit: vi.fn() }))
vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    fit = fitState.fit
  },
}))

// Terminal stub — observes loadAddon/refresh/dispose. cols/rows are fixed values for checking the refresh(0, rows-1) arguments.
const termState = vi.hoisted(() => ({
  loadAddon: vi.fn(),
  refresh: vi.fn(),
  dispose: vi.fn(),
  reset: vi.fn(),
  write: vi.fn(),
  // Input handler capture — to observe input blocking when absent (ADR-0148), data must actually be fed in.
  onDataCb: null as ((data: string) => void) | null,
  options: null as unknown,
}))
vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    constructor(options?: unknown) {
      termState.options = options
    }
    loadAddon = termState.loadAddon
    open = vi.fn()
    reset = termState.reset
    write = termState.write
    refresh = termState.refresh
    onData = (cb: (data: string) => void) => {
      termState.onDataCb = cb
      return { dispose: vi.fn() }
    }
    dispose = termState.dispose
    cols = 80
    rows = 24
  },
}))
vi.mock('@xterm/xterm/css/xterm.css', () => ({}))

// ── agentClient stub — observe output subscription unsubscribe. ──────────────────────────────────────
const captured = vi.hoisted(() => ({
  onChunk: null as ((c: OutputChunk) => void) | null,
  unsubscribe: null as ReturnType<typeof vi.fn> | null,
}))
// Connection state surface — real-like subscribers (one notification on registration); the test drives transitions to reproduce the connection-only absence (agent still alive in the registry).
const conn = vi.hoisted(() => ({
  state: 'connected' as 'connected' | 'reconnecting' | 'down',
  cbs: new Set<(s: string) => void>(),
}))
vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    subscribeOutput: vi.fn(
      async (_viewId: string, _agentId: string, onChunk: (c: OutputChunk) => void) => {
        captured.onChunk = onChunk
        const unsubscribe = vi.fn()
        captured.unsubscribe = unsubscribe
        return { unsubscribe }
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

// ── agentStore stub — empty list = alive; agentsLoaded=false (default) = registry not received, so an empty list is not mistaken for absent (ADR-0148 guard). ──
const agentStoreState = vi.hoisted(() => ({ agents: [] as unknown[], agentsLoaded: false }))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
}))

// ── Test subject ────────────────────────────────────────────────────────────────────
import TerminalSlot from './TerminalSlot'

const AGENT = 'aaaa-bbbb-cccc-dddd'

/** Flushes microtasks until subscribeOutput registers its callback (async .then) right after mount. */
async function flushSubscribe(): Promise<void> {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

/**
 * Plants a WebGL canvas in the container so TerminalSlot grabs the GL context at attach time; exposes getContext/loseContext as spies. TerminalSlot gets the context by scanning the DOM canvas, not the addon (version-robust), so this stand-in reproduces that path.
 * ★Call before fireVisibility(true)★: the capture happens in attachWebgl (the visible transition). (Returning with the captured context even if containerRef is null after unmount is Fix 1.)
 */
function installWebglCanvas(): { loseContext: ReturnType<typeof vi.fn> } {
  const loseContext = vi.fn(() => {
    order.log.push('loseContext')
  })
  const gl = { getExtension: vi.fn((name: string) => (name === 'WEBGL_lose_context' ? { loseContext } : null)) }
  const canvas = document.createElement('canvas')
  canvas.getContext = vi.fn((type: string) => (type === 'webgl2' ? gl : null)) as unknown as typeof canvas.getContext
  // The container is TerminalSlot's containerRef div (a childless leaf inside the outer padding div); attachWebgl scans only containerRef.current, so plant the canvas in exactly that place.
  const outer = document.querySelector('div[style*="padding"]')
  const container = outer?.querySelector('div')
  if (!container) throw new Error('containerRef div not found')
  container.appendChild(canvas)
  return { loseContext }
}

beforeEach(() => {
  ioState.instances = []
  roState.instances = []
  webglState.instances = []
  order.log = []
  captured.onChunk = null
  captured.unsubscribe = null
  agentStoreState.agents = []
  agentStoreState.agentsLoaded = false
  conn.state = 'connected'
  conn.cbs.clear()
  termState.onDataCb = null
  termState.options = null
  termState.loadAddon.mockReset()
  termState.refresh.mockClear()
  termState.dispose.mockClear()
  termState.reset.mockClear()
  termState.write.mockClear()
  fitState.fit.mockClear()
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

describe('TerminalSlot — WebGL seat visibility coupling (ADR-0056)', () => {
  it('mounting alone does not create WebGL — the seat is unoccupied until the first "visible" callback (initial-hidden safe)', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    expect(webglState.instances).toHaveLength(0)
    expect(ioState.instances.length).toBeGreaterThan(0)
  })

  it('visible transition → WebglAddon created + loadAddon + fit() + refresh()', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fireVisibility(true)

    expect(webglState.instances).toHaveLength(1)
    expect(termState.loadAddon).toHaveBeenCalledWith(webglState.instances[0])
    expect(fitState.fit).toHaveBeenCalled()
    expect(termState.refresh).toHaveBeenCalledWith(0, 23) // rows(24) - 1
  })

  it('hidden transition → loseContext() first, then addon.dispose() in that order + no new addon + fit() not called', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    const { loseContext } = installWebglCanvas()
    fireVisibility(true)
    expect(webglState.instances).toHaveLength(1)

    fitState.fit.mockClear()
    order.log = []

    fireVisibility(false)

    // ★load-bearing order★: dispose alone does not free the GPU seat until GC → loseContext must come *first*.
    expect(loseContext).toHaveBeenCalled()
    expect(order.log).toEqual(['loseContext', 'dispose'])
    expect(webglState.instances).toHaveLength(1)
    expect(fitState.fit).not.toHaveBeenCalled()
  })

  it('a visibility toggle does not dispose the Terminal or cut the output subscription (only on unmount)', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    expect(captured.unsubscribe).toBeTruthy()

    installWebglCanvas()
    fireVisibility(true)
    fireVisibility(false)
    fireVisibility(true)

    expect(termState.dispose).not.toHaveBeenCalled()
    expect(captured.unsubscribe).not.toHaveBeenCalled()
  })

  it('visible → hidden → visible re-attach → a second WebglAddon is newly created (seat re-acquired)', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    installWebglCanvas()
    fireVisibility(true)
    fireVisibility(false)
    fireVisibility(true)

    expect(webglState.instances).toHaveLength(2)
  })

  // ── Fix 1: return the seat on unmount too ──
  it('unmount → calls loseContext on the held GL context (it is returned even if containerRef becomes null)', async () => {
    const { unmount } = render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    const { loseContext } = installWebglCanvas()
    fireVisibility(true)
    expect(webglState.instances).toHaveLength(1)

    order.log = []
    act(() => unmount())

    // Not depending on containerRef is the point (React clears it to null at commit).
    expect(loseContext).toHaveBeenCalled()
    expect(order.log).toEqual(['loseContext', 'dispose'])
    expect(termState.dispose).toHaveBeenCalled()
  })

  // ── Fix 2: loadAddon throw → dispose the partial addon + ref null + no duplicate attach ──
  it('if loadAddon throws on visible → the created addon is disposed in catch, the ref is empty, and on the next visible only 1 new addon is attached', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    installWebglCanvas()
    termState.loadAddon.mockImplementationOnce(() => {
      throw new Error('loadAddon boom')
    })

    fireVisibility(true)

    expect(webglState.instances).toHaveLength(1)
    expect(webglState.instances[0].dispose).toHaveBeenCalled()

    // The second attach proves the ref was empty — a failed addon left in webglAddonRef.current would trip the attach guard.
    fireVisibility(true)
    expect(webglState.instances).toHaveLength(2)
    expect(termState.loadAddon).toHaveBeenLastCalledWith(webglState.instances[1])
  })

  // ── Fix 3: RO fires while hidden → fit()/resizePty skipped ──
  it('ResizeObserver fires at 0 size (hidden) → does not call fit() and resizePty', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fitState.fit.mockClear()
    ;(agentClient.resizePty as ReturnType<typeof vi.fn>).mockClear()

    // 0 width·height = reproduces the display:none collapse. offsetParent is also null in jsdom, so the hidden signal holds.
    fireResize(0, 0)

    expect(fitState.fit).not.toHaveBeenCalled()
    expect(agentClient.resizePty).not.toHaveBeenCalled()
  })
})

// ★ADR-0148: absent = found in terminal state ∪ unresolved even after the registry is received★
// A kill is collected by the reaper and **removed from the registry**; deciding by status alone would release the absent flag then, sending input to a dead agent (refusal swallowed, seen only as no response) and dropping the curtain.
// ★The curtain is shared by three slots and has no text (ADR-0148 decision 4)★ — presence is decided by a shared marker, not text.
function veil(): HTMLElement | null {
  return document.querySelector('[data-slot-dead="1"]')
}

function setConnection(state: 'connected' | 'reconnecting' | 'down'): void {
  act(() => {
    conn.state = state
    for (const cb of conn.cbs) cb(state)
  })
}

describe('TerminalSlot — agent absence verdict (ADR-0148)', () => {
  it('an agent that cannot be resolved after the roster is received blocks input and keeps the absence veil', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    agentStoreState.agents = []
    agentStoreState.agentsLoaded = true

    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    expect(veil()).not.toBeNull()
    ;(agentClient.writeStdin as ReturnType<typeof vi.fn>).mockClear()
    act(() => termState.onDataCb!('x'))
    expect(agentClient.writeStdin).not.toHaveBeenCalled()
  })

  // ★Carry a real live → absent transition★: rendering only the never-there state hooks the subscription once, so "does collection resubscribe" cannot be measured. Mount alive, consume subscription · reset, then reproduce the collection.
  it('if reaped after mounting, input is blocked and resubscribe · reset do not run again', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Running' }, epoch: 2 }]
    agentStoreState.agentsLoaded = true

    const { rerender } = render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    expect(veil()).toBeNull()
    const subCalls = (agentClient.subscribeOutput as ReturnType<typeof vi.fn>).mock.calls.length
    const resetCalls = termState.reset.mock.calls.length

    agentStoreState.agents = []
    rerender(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    expect(veil()).not.toBeNull()
    ;(agentClient.writeStdin as ReturnType<typeof vi.fn>).mockClear()
    act(() => termState.onDataCb!('x'))
    expect(agentClient.writeStdin).not.toHaveBeenCalled()
    // The subscription trigger is unchanged, so the effect does not run again (if it ran, the terminal buffer would be reset).
    expect((agentClient.subscribeOutput as ReturnType<typeof vi.fn>).mock.calls.length).toBe(subCalls)
    expect(termState.reset.mock.calls.length).toBe(resetCalls)
  })

  // ★Every condition for raising the curtain is a condition for blocking input (ADR-0148 decision 1 / ADR-0149 decision 4)★: the curtain is pointer-events-none, so an xterm that kept focus keeps eating keys behind it, and the failure is swallowed (seen only as no response). Here the agent stays alive and only the connection is cut.
  it('while the veil is up due to a dropped connection, input does not go out even if the agent is alive', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Running' }, epoch: 0 }]
    agentStoreState.agentsLoaded = true

    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    expect(veil()).toBeNull()

    // Control — while attached it goes out as is (to tell it apart from a guard that always blocks).
    ;(agentClient.writeStdin as ReturnType<typeof vi.fn>).mockClear()
    act(() => termState.onDataCb!('a'))
    expect(agentClient.writeStdin).toHaveBeenCalledTimes(1)

    setConnection('down')
    expect(veil()).not.toBeNull()
    ;(agentClient.writeStdin as ReturnType<typeof vi.fn>).mockClear()
    act(() => termState.onDataCb!('x'))
    expect(agentClient.writeStdin).not.toHaveBeenCalled()

    // It is released when reattached — if the guard sticks locked, the slot becomes permanently input-less.
    setConnection('connected')
    act(() => termState.onDataCb!('y'))
    expect(agentClient.writeStdin).toHaveBeenCalledTimes(1)
  })

  it('the span where the roster is not yet received is not seen as absent — input goes out as is', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    agentStoreState.agents = []
    agentStoreState.agentsLoaded = false

    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    expect(veil()).toBeNull()
    ;(agentClient.writeStdin as ReturnType<typeof vi.fn>).mockClear()
    act(() => termState.onDataCb!('x'))
    expect(agentClient.writeStdin).toHaveBeenCalledTimes(1)
  })
})

describe('TerminalSlot — cursor default', () => {
  // The cursor is a blinking white bar (thickness left as is); the inactive cursor is also a bar, since xterm's default 'outline' draws an empty box over the whole cell in an unfocused slot.
  it('built with a white bar · blinking cursor, inactive is also a bar, thickness uses the xterm default', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    expect(termState.options).toMatchObject({
      cursorStyle: 'bar',
      cursorInactiveStyle: 'bar',
      cursorBlink: true,
      theme: expect.objectContaining({ cursor: '#ffffff' }),
    })
    expect(termState.options).not.toHaveProperty('cursorWidth')
  })
})
