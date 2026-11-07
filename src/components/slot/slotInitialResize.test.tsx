// Regression for the hidden guard on the initial size report right after subscribing — a size that cannot be measured is not sent, and another path emits it once visible.
//
// Background: when the subscription resolves the slot emits its current size to the PTY once (ADR-0036). If that spot sends without a guard in a hidden tab (keep-alive = display:none,
//   ADR-0056) or an area whose box has collapsed, an untrustworthy grid produced by fit() in an unmeasurable state goes out and
//   overwrites the PTY size a visible slot viewing the same agent had set. Industry rule = do not send a size that cannot be measured
//   and keep the last size (docs/research/hidden-tab-terminal-subscription-survey-2026-09-25.md).
//
// ★Measuring the guard alone also passes an implementation that 「영영 안 보낸다」 (never sends)★ — so we also measure that, on the transition to visible after skipping, the visibility effect
//   (WebGL attach) emits it, and that if the attach fails, RO emits the size.

import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── Two controllable observers — the test fires them directly (to measure the recovery paths one at a time). ─────────
type ROEntry = { contentRect: { width: number; height: number } }
const roState = vi.hoisted(() => ({ instances: [] as Array<{ cb: (entries: ROEntry[]) => void }> }))
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

type IOEntry = { isIntersecting: boolean; intersectionRatio: number }
const ioState = vi.hoisted(() => ({ instances: [] as Array<{ cb: (entries: IOEntry[]) => void }> }))
globalThis.IntersectionObserver = class {
  cb: (entries: IOEntry[]) => void
  constructor(cb: (entries: IOEntry[]) => void) {
    this.cb = cb
    ioState.instances.push(this)
  }
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return []
  }
} as unknown as typeof IntersectionObserver

function fireVisibility(visible: boolean): void {
  const io = ioState.instances[ioState.instances.length - 1]
  if (!io) throw new Error('no IntersectionObserver instance')
  act(() => io.cb([{ isIntersecting: visible, intersectionRatio: visible ? 1 : 0 }]))
}

function fireResize(width: number, height: number): void {
  const ro = roState.instances[roState.instances.length - 1]
  if (!ro) throw new Error('no ResizeObserver instance')
  act(() => ro.cb([{ contentRect: { width, height } }]))
}

// ── Layout stand-in — why it is planted on the prototype: the same section in slotReconnectResize.test.tsx is canonical. ─────────
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

// The test holds when the subscription resolves — 「풀리는 순간 보이나」 (is it visible the moment it resolves) is this guard's decision point.
const pending = vi.hoisted(() => ({ resolvers: [] as Array<() => void> }))
const conn = vi.hoisted(() => ({ cbs: new Set<(s: string) => void>() }))
vi.mock('../../api/clientFactory', () => ({
  agentClient: {
    subscribeOutput: vi.fn(
      () =>
        new Promise<{ unsubscribe: () => void }>((res) => {
          pending.resolvers.push(() => res({ unsubscribe: vi.fn() }))
        }),
    ),
    writeStdin: vi.fn(async () => undefined),
    resizePty: vi.fn(async () => undefined),
    connectionState: 'connected',
    onConnectionStateChange: (cb: (s: string) => void) => {
      conn.cbs.add(cb)
      cb('connected')
      return () => conn.cbs.delete(cb)
    },
  },
  getAgentClient: vi.fn(),
}))

const agentStoreState = vi.hoisted(() => ({ agents: [] as unknown[], agentsLoaded: false }))
vi.mock('../../store/agentStore', () => ({
  useAgentStore: (selector: (s: typeof agentStoreState) => unknown) => selector(agentStoreState),
}))

const xtermDims = vi.hoisted(() => ({ cols: 80, rows: 24 }))
const loadAddon = vi.hoisted(() => vi.fn())
vi.mock('@xterm/xterm', () => ({
  Terminal: class {
    loadAddon = loadAddon
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
const measured = { cols: 137, rows: 25 }

async function resolveSubscriptions(): Promise<void> {
  await act(async () => {
    for (const resolve of pending.resolvers.splice(0)) resolve()
    await Promise.resolve()
    await Promise.resolve()
  })
}

function containers(): HTMLElement[] {
  return [...document.querySelectorAll('div[style*="padding"]')].map((outer) => {
    const container = outer.querySelector('div')
    if (!container) throw new Error('containerRef div not found')
    return container
  })
}

beforeEach(() => {
  roState.instances = []
  ioState.instances = []
  pending.resolvers = []
  conn.cbs.clear()
  layout.visible = true
  layout.width = 800
  layout.height = 400
  xtermDims.cols = 80
  xtermDims.rows = 24
  loadAddon.mockReset()
  // Mock of the fit contract — the rationale is canonical in the same mock comment in slotReconnectResize.test.tsx.
  fitAddonFit.mockReset()
  fitAddonFit.mockImplementation(() => {
    if (!layout.visible) return
    if (layout.width === 0 || layout.height === 0) {
      xtermDims.cols = 2
      xtermDims.rows = 1
      return
    }
    xtermDims.cols = measured.cols
    xtermDims.rows = measured.rows
  })
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
  vi.restoreAllMocks()
  vi.useRealTimers()
})

describe('TerminalSlot — 구독 직후 초기 치수의 숨김 가드', () => {
  it('보이는 슬롯은 구독이 풀리는 순간 실측 치수를 정확히 한 번 보낸다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    fitAddonFit.mockClear()

    await resolveSubscriptions()

    expect(fitAddonFit).toHaveBeenCalled()
    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)
    expect(agentClient.resizePty).toHaveBeenCalledWith(AGENT, 137, 25)
  })

  // The value real fit() produces under display:none (measured 11×6 — TerminalSlot helper comment) is not imitated by the mock, so
  //   we decide by fit() not being called rather than by size.
  it('숨은 슬롯(display:none)에서 구독이 풀리면 fit() 도 치수도 보내지 않는다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    layout.visible = false
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    fitAddonFit.mockClear()

    await resolveSubscriptions()

    expect(fitAddonFit).not.toHaveBeenCalled()
    expect(agentClient.resizePty).not.toHaveBeenCalled()
  })

  // On a collapsed axis fit() clamps to the floor value (2 columns · 1 row) (FitAddon source) — that grid is indistinguishable from a normal size by value, so
  //   the assertion hangs on fit() not being called. ★The two cases with only one axis at 0 weigh the same as both axes at 0★.
  it.each([
    [0, 0],
    [0, 400],
    [800, 0],
  ])('표시됐지만 박스가 %i×%i 로 붕괴한 채 구독이 풀리면 fit() 에 닿기 전에 빠진다', async (width, height) => {
    const { agentClient } = await import('../../api/clientFactory')
    layout.width = width
    layout.height = height
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    fitAddonFit.mockClear()

    await resolveSubscriptions()

    expect(fitAddonFit).not.toHaveBeenCalled()
    expect(agentClient.resizePty).not.toHaveBeenCalled()
  })

  // The actual harm the guard blocks — when two views watch one PTY, the hidden view does not decide the size.
  it('같은 에이전트를 보이는 슬롯과 숨은 슬롯이 함께 보면 보이는 쪽만 보낸다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    render(
      <>
        <TerminalSlot viewId="v1" agentId={AGENT} />
        <TerminalSlot viewId="v2" agentId={AGENT} />
      </>,
    )
    Object.defineProperty(containers()[1], 'offsetParent', { value: null, configurable: true })
    expect(agentClient.subscribeOutput).toHaveBeenCalledTimes(2)

    await resolveSubscriptions()

    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)
    expect(agentClient.resizePty).toHaveBeenCalledWith(AGENT, 137, 25)
  })
})

describe('TerminalSlot — 숨은 채 건너뛴 치수는 보이게 될 때 다른 경로가 낸다', () => {
  it('보임 전이의 가시성 effect(WebGL 부착)가 실측 치수를 낸다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    layout.visible = false
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    fireVisibility(false)
    await resolveSubscriptions()
    expect(agentClient.resizePty).not.toHaveBeenCalled()

    layout.visible = true
    fireVisibility(true)

    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)
    expect(agentClient.resizePty).toHaveBeenCalledWith(AGENT, 137, 25)
  })

  // If the WebGL attach fails, the size propagation bundled inside it is dropped too — only RO is left. Leaving hiding
  //   is itself a container size change (0×0 → real), so RO fires.
  it('WebGL 부착이 실패해도 컨테이너가 커지며 울리는 RO 가 치수를 낸다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    layout.visible = false
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    fireVisibility(false)
    await resolveSubscriptions()

    layout.visible = true
    loadAddon.mockImplementationOnce(() => {
      throw new Error('webgl unavailable')
    })
    fireVisibility(true)
    expect(agentClient.resizePty).not.toHaveBeenCalled()

    vi.useFakeTimers()
    fireResize(800, 400)
    act(() => {
      vi.advanceTimersByTime(50) // RO firing's debounce
    })

    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)
    expect(agentClient.resizePty).toHaveBeenCalledWith(AGENT, 137, 25)
  })
})
