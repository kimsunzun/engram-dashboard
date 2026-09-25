// 구독 직후 초기 치수 보고의 숨김 가드 회귀 — 잴 수 없는 치수는 보내지 않고, 보이게 되면 다른 경로가 낸다.
//
// 배경: 구독이 풀리면 슬롯은 지금 치수를 PTY 에 한 번 낸다(ADR-0036). 숨은 탭(keep-alive = display:none,
//   ADR-0056)이나 박스가 붕괴한 칸에서 그 자리가 가드 없이 보내면 잴 수 없는 상태의 fit() 이 낸 믿을 수 없는
//   격자가 나가, 같은 에이전트를 보는 보이는 슬롯이 맞춰 둔 PTY 치수를 덮는다. 업계 규칙 = 잴 수 없는 크기는
//   보내지 않고 마지막 크기를 유지한다(docs/research/hidden-tab-terminal-subscription-survey-2026-09-25.md).
//
// ★가드만 재면 「영영 안 보낸다」 구현도 통과한다★ — 그래서 건너뛴 뒤 보이게 되는 전이에서 가시성 effect
//   (WebGL 부착)가, 그 부착이 실패하면 RO 가 치수를 낸다는 것까지 함께 잰다.

import { act, cleanup, render } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

// ── 제어 가능한 관측자 2종 — 발화는 테스트가 직접 몬다(복구 경로를 하나씩 떼어 재기 위해서다). ─────────
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

// ── 레이아웃 대역 — 프로토타입에 심는 이유는 slotReconnectResize.test.tsx 의 같은 절이 정본이다. ─────────
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

// 구독 해소 시점을 테스트가 쥔다 — 「풀리는 순간 보이나」가 이 가드의 판정 시점이다.
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
  // fit 계약의 목 — 근거는 slotReconnectResize.test.tsx 의 같은 목 주석이 정본이다.
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

  // display:none 하위에서 실물 fit() 이 내는 값(실측 11×6 — TerminalSlot 헬퍼 주석)은 목이 흉내 내지 않으므로
  //   치수가 아니라 fit() 미호출로 판정한다.
  it('숨은 슬롯(display:none)에서 구독이 풀리면 fit() 도 치수도 보내지 않는다', async () => {
    const { agentClient } = await import('../../api/clientFactory')
    layout.visible = false
    render(<TerminalSlot viewId="v1" agentId={AGENT} />)
    fitAddonFit.mockClear()

    await resolveSubscriptions()

    expect(fitAddonFit).not.toHaveBeenCalled()
    expect(agentClient.resizePty).not.toHaveBeenCalled()
  })

  // 붕괴한 축은 fit() 이 바닥값(열 2 · 행 1)으로 누른다(FitAddon 소스) — 그 격자는 정상 치수와 값으로
  //   구별되지 않아 단언을 fit() 미호출에 건다. ★한 축만 0 인 두 경우가 두 축 모두 0 과 같은 무게다★.
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

  // 가드가 막는 실제 피해 — 한 PTY 를 두 뷰가 볼 때 숨은 뷰는 크기를 정하지 않는다.
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

  // WebGL 부착이 실패하면 그 안에 묶인 치수 전파도 함께 빠진다 — 남는 것은 RO 뿐이다. 숨김을 벗어나는 일이
  //   곧 컨테이너 크기 변화(0×0 → 실제)라 RO 가 울린다.
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
      vi.advanceTimersByTime(50) // RO 발화의 debounce
    })

    expect(agentClient.resizePty).toHaveBeenCalledTimes(1)
    expect(agentClient.resizePty).toHaveBeenCalledWith(AGENT, 137, 25)
  })
})
