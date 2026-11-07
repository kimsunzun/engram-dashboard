// WebGL seat visibility-linked lifecycle (ADR-0056) regression — only a visible slot holds a WebGL seat.
//
// Background: every terminal tab keeps its mount with keep-alive (WindowLayout puts hidden tabs at display:none).
//   The browser (WebView2/Chromium) limits concurrently active WebGL contexts with a hard cap (measured 16), so
//   if every mounted slot held WebGL the cap would be exceeded and old contexts lost. So the WebGL addon is tied to
//   visibility — attach when visible, return when hidden (loseContext→dispose). The Terminal instance (buffer) always lives.
//
// ★Verification core★: jsdom has no real WebGL/IntersectionObserver, so only the logic layer is checked.
//
// Strategy: lay down IntersectionObserver as a controllable mock so the test fires visible/hidden callbacks directly.
//   Stub xterm Terminal / FitAddon / WebglAddon and observe calls via a static holder. loseContext:
//   TerminalSlot scans canvases inside the container at attach time and grabs the context via getContext('webgl2'|'webgl')
//   (unmount ref-null defense), so *before* it becomes visible, plant a canvas in the container and spy on that canvas's getContext
//   to check the loseContext call and order on the grabbed context.

import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { OutputChunk } from '../../api/agentClient'

// jsdom does not provide ResizeObserver — TerminalSlot does new ResizeObserver on mount.
//   For the Fix 3 check (skip fit/resize while hidden) the callback is captured so the test can fire it directly.
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

// ── Controllable IntersectionObserver mock ────────────────────────────────────────────
// A real IO fires the callback immediately on observation (reflecting current visibility) + on every transition. Here the test drives firing directly
//   to reproduce initial-hidden/visible/hidden transitions.
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

// ── xterm/addon stub — observe construction and calls via a static holder. ──────────────────────────────
// A WebglAddon instance is created anew on every attach, so the creation list · dispose · onContextLoss are gathered in a hoisted holder
//   for the test to check count/order. The call log (order) is shared for the loseContext order check.
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
// Connection state surface — holds subscribers the same as the real one and notifies them on transitions (including one notification on registration).
//   The test must be able to drive transitions to reproduce the "connection-only" absence (the agent is still alive in the registry).
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
    // The slot reads it to decide the absent curtain (one notification on registration + returns a disposer).
    onConnectionStateChange: (cb: (s: string) => void) => {
      conn.cbs.add(cb)
      cb(conn.state)
      return () => conn.cbs.delete(cb)
    },
  },
  getAgentClient: vi.fn(),
}))

// ── agentStore stub — the slot queries useAgentStore(s => s.agents) for the exit decision. Empty list = alive. ──
// agentsLoaded=false by default = authoritative registry not received → an empty list is not mistaken for absent (ADR-0148 guard).
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
 * Plants a WebGL canvas inside the container so TerminalSlot grabs the GL context at attach time, and exposes its
 * getContext/loseContext as spies. A stand-in for the canvas a real WebglAddon appends — TerminalSlot gets the context by scanning the DOM canvas,
 * not the addon (version-robust path), so this stand-in reproduces that path.
 *
 * ★Call timing★: the context capture happens in attachWebgl (= the visible transition), so this canvas must be planted
 *   *before* fireVisibility(true) to be captured. (Structurally, returning with the captured context even if containerRef is null after unmount is Fix 1.)
 */
function installWebglCanvas(): { loseContext: ReturnType<typeof vi.fn> } {
  const loseContext = vi.fn(() => {
    order.log.push('loseContext')
  })
  const gl = { getExtension: vi.fn((name: string) => (name === 'WEBGL_lose_context' ? { loseContext } : null)) }
  const canvas = document.createElement('canvas')
  // Override so getContext('webgl2') returns our stand-in gl (jsdom returns null).
  canvas.getContext = vi.fn((type: string) => (type === 'webgl2' ? gl : null)) as unknown as typeof canvas.getContext
  // The container is TerminalSlot's containerRef div = a child div (a leaf with no children) inside the outer (padding) div.
  //   A real WebglAddon appends the canvas to this div, so the stand-in plants it in exactly the same place —
  //   attachWebgl scans for canvases only inside containerRef.current, so if the place is wrong the context cannot be grabbed.
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

describe('TerminalSlot — WebGL 좌석 가시성 연동(ADR-0056)', () => {
  it('마운트만으론 WebGL 을 만들지 않는다 — 첫 "보임" 콜백 전까지 좌석 미점유(초기-숨김 안전)', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    expect(webglState.instances).toHaveLength(0)
    expect(ioState.instances.length).toBeGreaterThan(0)
  })

  it('보임 전이 → WebglAddon 생성 + loadAddon + fit() + refresh()', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    fireVisibility(true)

    expect(webglState.instances).toHaveLength(1)
    expect(termState.loadAddon).toHaveBeenCalledWith(webglState.instances[0])
    expect(fitState.fit).toHaveBeenCalled()
    expect(termState.refresh).toHaveBeenCalledWith(0, 23) // rows(24) - 1
  })

  it('숨김 전이 → loseContext() 를 먼저, addon.dispose() 를 다음 순서로 호출 + 새 addon 미생성 + fit() 미호출', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    // Capture is at attach time → the canvas must be planted *before* it becomes visible for the context to be grabbed.
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
    // It cannot be measured while hidden, so fit() is not called (prevents garbage dimensions).
    expect(fitState.fit).not.toHaveBeenCalled()
  })

  it('가시성 토글이 Terminal 을 dispose 하거나 출력 구독을 끊지 않는다(언마운트에서만)', async () => {
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

  it('보임 → 숨김 → 보임 재부착 → 두 번째 WebglAddon 이 새로 생성된다(좌석 재획득)', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    installWebglCanvas()
    fireVisibility(true)
    fireVisibility(false)
    // After hiding the addon ref is empty, so a new one is made at the next visible.
    fireVisibility(true)

    expect(webglState.instances).toHaveLength(2)
  })

  // ── Fix 1 (HIGH): return the seat on unmount too ──────────────────────────────────────────
  it('언마운트 → 잡아둔 GL 컨텍스트로 loseContext 호출(containerRef 가 null 이 돼도 반납된다)', async () => {
    const { unmount } = render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    // Plant the canvas before visible to capture at attach time → even if containerRef is null at unmount, return with this context.
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

  // ── Fix 2 (HIGH): loadAddon throw → dispose the partially created addon + ref null + no duplicate attach ──
  it('보임 시 loadAddon 이 throw → 생성된 addon 을 catch 에서 dispose, ref 는 비고, 다음 보임에서 새 addon 1개만 부착', async () => {
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()

    installWebglCanvas()
    // Make loadAddon throw at the first visible (reproduces the partial-creation path).
    termState.loadAddon.mockImplementationOnce(() => {
      throw new Error('loadAddon boom')
    })

    fireVisibility(true)

    expect(webglState.instances).toHaveLength(1)
    expect(webglState.instances[0].dispose).toHaveBeenCalled()

    // At the next visible (normal loadAddon) exactly one new addon attaches — if the earlier failed one had stayed in the ref, the guard at the start of attach
    //   (webglAddonRef.current) would keep a new one from attaching. So the second one attaching = the ref was empty.
    fireVisibility(true)
    expect(webglState.instances).toHaveLength(2)
    expect(termState.loadAddon).toHaveBeenLastCalledWith(webglState.instances[1])
  })

  // ── Fix 3 (MEDIUM): RO fires while hidden → fit()/resizePty skipped ─────────────────────
  it('ResizeObserver 가 0 크기(숨김)에서 발화 → fit() 과 resizePty 를 호출하지 않는다', async () => {
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
//
// The real outcome of a kill is that the reaper collects the session and **removes it from the registry**. Deciding by status alone
// releases the absent flag at that moment, so input goes out to a dead agent (the refusal is quietly swallowed and seen only as no response)
// and the absent curtain vanishes too.
//
// ★The curtain is shared by three slots and has no text (ADR-0148 decision 4, user decision 2026-08-20)★ — so the presence decision is made by
// a shared marker, not by text.
function veil(): HTMLElement | null {
  return document.querySelector('[data-slot-dead="1"]')
}

/** Connection state transition (same shape as the path where the real ProtocolClient notifies all subscribers). */
function setConnection(state: 'connected' | 'reconnecting' | 'down'): void {
  act(() => {
    conn.state = state
    for (const cb of conn.cbs) cb(state)
  })
}

describe('TerminalSlot — 에이전트 부재 판정(ADR-0148)', () => {
  it('명부 수신 후 해석 안 되는 에이전트면 입력을 막고 부재 막을 유지한다', async () => {
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

  // ★Actually carry the live → absent transition★: rendering only in the never-there state hooks the subscription just once, so
  //   "does collection call a resubscribe" cannot be measured. Mount alive, consume the subscription · reset, then reproduce the collection.
  it('마운트된 뒤 수거되면 입력이 막히고, 재구독·reset 이 다시 돌지 않는다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    agentStoreState.agents = [{ id: AGENT, cwd: 'C:/x', status: { type: 'Running' }, epoch: 2 }]
    agentStoreState.agentsLoaded = true

    const { rerender } = render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    await flushSubscribe()
    expect(veil()).toBeNull()
    const subCalls = (agentClient.subscribeOutput as ReturnType<typeof vi.fn>).mock.calls.length
    const resetCalls = termState.reset.mock.calls.length

    // Collection = gone from the registry. The props the parent passes (viewId·agentId) are unchanged.
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

  // ★Every condition for raising the curtain is a condition for blocking input (ADR-0148 decision 1 / ADR-0149 decision 4)★: the curtain is
  //   pointer-events-none, so an xterm that kept focus keeps eating keys even with the curtain up. Blocking on exit alone sends input
  //   from a disconnected slot behind a screen that says "없다" (it is gone), and that failure is quietly swallowed and seen by the user only as
  //   no response. Here the agent is **kept alive** in the registry and only the connection is cut, to measure that branch.
  it('연결이 끊겨 막이 뜬 동안은 에이전트가 살아 있어도 입력이 나가지 않는다', async () => {
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

  it('명부를 아직 못 받은 구간은 부재로 보지 않는다 — 입력이 그대로 나간다', async () => {
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

describe('TerminalSlot — 커서 기본값', () => {
  // User decision 2026-09-25 — the cursor becomes a blinking white bar and the thickness is left as it is.
  // The inactive cursor is also a bar to fit that choice — the xterm default 'outline' draws an entire empty box
  //   over the whole cell in an unfocused slot.
  it('흰 막대·깜빡임 커서로 만들고 비활성도 막대, 굵기는 xterm 기본값을 쓴다', async () => {
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
