// useScrollFollow — DOM wiring test (ADR-0242 · TRD §2-3 · §2-7). The decision rules are measured by followCore.test.ts; here only that input is fed into the core correctly and the result reaches the node, the observation surface and the handle map.
// Strategy: jsdom has neither layout nor ResizeObserver — a fake RO is plugged in as a seam and the Element prototype accessors are replaced so only nodes with `data-vp` have scrollHeight · clientHeight (scrollTop stored per node, clamped at the bottom). The browser emits one scroll event on a node whose position changed **on the next frame** — `frame()` is that.

import { StrictMode, type ReactElement } from 'react'
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { ScrollArea } from '../../ui/scroll-area'
import { getFollow } from './followRegistry'
import { useScrollFollow, type ScrollFollow } from './useScrollFollow'

class FakeRO {
  static all: FakeRO[] = []
  observed: Element[] = []
  disconnected = false
  constructor(private readonly cb: ResizeObserverCallback) {
    FakeRO.all.push(this)
  }
  observe(el: Element): void {
    this.observed.push(el)
  }
  unobserve(): void {}
  disconnect(): void {
    this.disconnected = true
  }
  fire(): void {
    act(() => this.cb([], this as unknown as ResizeObserver))
  }
}
const RO = FakeRO as unknown as typeof ResizeObserver
const liveRO = (): FakeRO[] => FakeRO.all.filter(r => !r.disconnected)

const geo = { height: 1000, client: 200 }
const bottom = (): number => Math.max(0, geo.height - geo.client)
const tops = new WeakMap<Element, number>()
const moved = new Set<Element>()
const isVp = (el: Element): boolean => el.hasAttribute('data-vp')

/** Next frame — one scroll event per viewport whose position changed. */
function frame(): void {
  for (const el of [...moved]) {
    moved.delete(el)
    fireEvent.scroll(el)
  }
}

beforeEach(() => {
  FakeRO.all = []
  moved.clear()
  geo.height = 1000
  geo.client = 200
  vi.spyOn(Element.prototype, 'scrollHeight', 'get').mockImplementation(function (this: Element) {
    return isVp(this) ? geo.height : 0
  })
  vi.spyOn(Element.prototype, 'clientHeight', 'get').mockImplementation(function (this: Element) {
    return isVp(this) ? geo.client : 0
  })
  vi.spyOn(Element.prototype, 'scrollTop', 'get').mockImplementation(function (this: Element) {
    return tops.get(this) ?? 0
  })
  vi.spyOn(Element.prototype, 'scrollTop', 'set').mockImplementation(function (this: Element, v: number) {
    const next = isVp(this) ? Math.min(Math.max(0, v), bottom()) : v
    if (isVp(this) && next !== (tops.get(this) ?? 0)) moved.add(this)
    tops.set(this, next)
  })
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

let latest: ScrollFollow | null = null

function Harness({
  slotId = 's1',
  show = true,
  nodeKey = 'a',
  withContent = true,
}: {
  slotId?: string
  show?: boolean
  nodeKey?: string
  withContent?: boolean
}) {
  const follow = useScrollFollow(slotId, { ResizeObserver: RO })
  latest = follow
  if (!show) return null
  return (
    <div key={nodeKey} data-vp="" data-testid="vp" ref={follow.viewportRef}>
      {withContent && (
        <div data-testid="content">
          <div data-radix-scroll-area-viewport="" data-testid="inner">
            <p data-testid="inner-text">생각</p>
          </div>
          <textarea data-testid="editable" />
        </div>
      )}
    </div>
  )
}

const vp = (): HTMLElement => screen.getByTestId('vp')
const followAttr = (): string | null => vp().getAttribute('data-scroll-follow')

/** Pin and receive the scroll event of our first write. */
function mount(ui: ReactElement): ReturnType<typeof render> {
  const out = render(ui)
  frame()
  return out
}

/** The user scrolled to that position — the earlier write's event arrives first, then this move's event. */
function userScroll(top: number): void {
  frame()
  vp().scrollTop = top
  frame()
}

/** With something to scroll, release following with a small up-wheel. */
function wheelUp(target: Element = vp()): void {
  fireEvent.wheel(target, { deltaY: -40 })
}

describe('attach · follow', () => {
  it('on attach it writes to the bottom and data-scroll-follow="pinned"', () => {
    mount(<Harness />)
    expect(vp().scrollTop).toBe(bottom())
    expect(followAttr()).toBe('pinned')
    expect(latest!.pinned).toBe(true)
  })

  it('while attached it follows content growth to the bottom', () => {
    mount(<Harness />)
    geo.height = 1600
    liveRO()[0].fire()
    expect(vp().scrollTop).toBe(1400)
  })

  it('even if the viewport shrinks (waiting list above the input box) it stays at the bottom', () => {
    mount(<Harness />)
    geo.client = 120
    liveRO()[0].fire()
    expect(vp().scrollTop).toBe(880)
  })
})

describe('upward input', () => {
  it('an upward wheel releases (data-scroll-follow="free"), and growth while detached does not pull down', () => {
    mount(<Harness />)
    wheelUp()
    expect(followAttr()).toBe('free')
    expect(latest!.pinned).toBe(false)

    userScroll(bottom() - 10) // The up scroll that wheel produced (within the threshold) — the latch does not re-pin.
    geo.height = 1600
    liveRO()[0].fire()
    expect(vp().scrollTop).toBe(790)
    expect(latest!.pinned).toBe(false)
    expect(latest!.unseenGrowth).toBe(true)
  })

  it('a downward wheel · ctrl wheel (zoom) does not release', () => {
    mount(<Harness />)
    fireEvent.wheel(vp(), { deltaY: 40 })
    fireEvent.wheel(vp(), { deltaY: -40, ctrlKey: true })
    expect(followAttr()).toBe('pinned')
  })

  it('an upward wheel consumed by the inner Radix viewport is ignored, and if the inner is at the top it is read as the outer upward intent', () => {
    mount(<Harness />)
    const inner = screen.getByTestId('inner')
    inner.scrollTop = 30
    wheelUp(screen.getByTestId('inner-text'))
    expect(followAttr()).toBe('pinned')

    inner.scrollTop = 0
    wheelUp(screen.getByTestId('inner-text'))
    expect(followAttr()).toBe('free')
  })

  it.each(['PageUp', 'Home', 'ArrowUp'])('key %s releases', key => {
    mount(<Harness />)
    fireEvent.keyDown(vp(), { key })
    expect(followAttr()).toBe('free')
  })

  it('a key inside an edit field · a key during composition · other keys do not release', () => {
    mount(<Harness />)
    fireEvent.keyDown(screen.getByTestId('editable'), { key: 'ArrowUp' })
    fireEvent.keyDown(vp(), { key: 'ArrowUp', isComposing: true })
    fireEvent.keyDown(vp(), { key: 'ArrowDown' })
    expect(followAttr()).toBe('pinned')
  })

  // The viewport takes focus on click (tabIndex -1), so keys arrive with the viewport as the target.
  it('ArrowUp on a focused viewport releases', () => {
    mount(<Harness />)
    vp().focus()
    expect(document.activeElement).toBe(vp())
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp' })
    expect(followAttr()).toBe('free')
  })

  it('scrolling downward while detached and entering the threshold re-attaches', () => {
    mount(<Harness />)
    wheelUp()
    userScroll(300)
    userScroll(bottom() - 5)
    expect(followAttr()).toBe('pinned')
    expect(latest!.pinned).toBe(true)
  })
})

describe('when our write and user scroll overlap', () => {
  // The scrollbar was dragged before the scroll event of the growth write (800 → 1400) arrived — the browser merges the two into one event.
  it('dragging up before the write\'s event comes releases — the next growth does not pull down', () => {
    mount(<Harness />)
    geo.height = 1600
    liveRO()[0].fire()
    expect(vp().scrollTop).toBe(1400)
    vp().scrollTop = 1300
    frame()
    expect(followAttr()).toBe('free')

    geo.height = 2000
    liveRO()[0].fire()
    expect(vp().scrollTop).toBe(1300)
  })
})

describe('scrollable — button condition', () => {
  it('starts true if content is larger than the viewport', () => {
    mount(<Harness />)
    expect(latest!.scrollable).toBe(true)
  })

  it('if the viewport grows while detached and everything fits it is false — attachment is not re-measured (ADR-0242 decision 2)', () => {
    mount(<Harness />)
    wheelUp()
    geo.client = 1200
    liveRO()[0].fire()
    expect(latest!.scrollable).toBe(false)
    expect(latest!.pinned).toBe(false)

    geo.client = 200
    liveRO()[0].fire()
    expect(latest!.scrollable).toBe(true)
  })

  it('while hidden (client 0) the last measured value is kept as is', () => {
    mount(<Harness />)
    geo.client = 0
    liveRO()[0].fire()
    expect(latest!.scrollable).toBe(true)
  })
})

describe('handle pin · unpin', () => {
  it('pin attaches and writes to the bottom · unpin leaves the position as is', () => {
    mount(<Harness />)
    wheelUp()
    userScroll(300)
    act(() => latest!.pin())
    expect(vp().scrollTop).toBe(bottom())
    expect(latest!.pinned).toBe(true)

    act(() => latest!.unpin())
    expect(vp().scrollTop).toBe(bottom())
    expect(followAttr()).toBe('free')
  })

  it('after a re-render viewportRef · pin · unpin are the same function', () => {
    const { rerender } = mount(<Harness />)
    const first = latest!
    wheelUp() // The pinned state changes, so one re-render runs
    rerender(<Harness />)
    expect(latest).not.toBe(first)
    expect(latest!.viewportRef).toBe(first.viewportRef)
    expect(latest!.pin).toBe(first.pin)
    expect(latest!.unpin).toBe(first.unpin)
  })
})

describe('hidden tab (display:none — client 0)', () => {
  it('while frozen pin does not write and only raises attachment — bottom when shown', () => {
    mount(<Harness />)
    wheelUp()
    userScroll(300)
    geo.client = 0
    liveRO()[0].fire()
    act(() => latest!.pin())
    expect(vp().scrollTop).toBe(300)
    expect(followAttr()).toBe('pinned')

    geo.client = 200
    liveRO()[0].fire()
    expect(vp().scrollTop).toBe(bottom())
  })

  it('returning after hiding while detached restores the place being read (the not-preserved case)', () => {
    mount(<Harness />)
    wheelUp()
    userScroll(300)
    geo.client = 0
    liveRO()[0].fire()
    tops.set(vp(), 0) // Assume the position was lost passing through display:none (whether it is preserved is for GUI measurement — TRD §8-2 F1 ①)
    geo.client = 200
    liveRO()[0].fire()
    expect(vp().scrollTop).toBe(300)
    expect(followAttr()).toBe('free')
  })

  it('a scroll arriving while hidden (height 0 → a fake 「bottom」) does not re-attach a detached state', () => {
    mount(<Harness />)
    wheelUp()
    userScroll(300)
    geo.client = 0
    geo.height = 0
    // Read without freezing, it is a value that re-pins as 「moved down, bottom distance ≤ threshold」.
    tops.set(vp(), 310)
    fireEvent.scroll(vp())
    expect(followAttr()).toBe('free')
  })
})

describe('node lifetime', () => {
  it('when the node changes it detaches the old node\'s listeners · observers and attaches to the new node', () => {
    const { rerender } = render(<Harness nodeKey="a" />)
    const oldNode = vp()
    const oldRO = liveRO()[0]
    rerender(<Harness nodeKey="b" />)
    const newNode = vp()
    expect(newNode).not.toBe(oldNode)
    expect(oldRO.disconnected).toBe(true)
    const [newRO] = liveRO()
    expect(newRO.observed).toEqual([newNode, newNode.firstElementChild])
    expect(newNode.getAttribute('data-scroll-follow')).toBe('pinned')
    expect(newNode.scrollTop).toBe(bottom())

    fireEvent.wheel(oldNode, { deltaY: -40 }) // Input from a detached old node does not reach the core
    expect(latest!.pinned).toBe(true)
  })

  it('even if the viewport comes down and attaches again (empty state ↔ conversation) it continues the attached state', () => {
    const { rerender } = mount(<Harness />)
    wheelUp()
    rerender(<Harness show={false} />)
    expect(liveRO()).toHaveLength(0)
    rerender(<Harness show />)
    expect(followAttr()).toBe('free')
  })

  it('on attach it lets the viewport receive click focus (tabIndex -1 · no border), and on detach it restores', () => {
    const { rerender } = mount(<Harness />)
    const node = vp()
    expect(node.getAttribute('tabindex')).toBe('-1')
    expect(node.style.outline).toBe('none')
    rerender(<Harness nodeKey="b" />)
    expect(node.hasAttribute('tabindex')).toBe(false)
    expect(node.style.outline).toBe('')
  })

  it('a node that had its own tabIndex restores that value on detach', () => {
    function WithTab({ nodeKey }: { nodeKey: string }) {
      const follow = useScrollFollow('tab', { ResizeObserver: RO })
      return <div key={nodeKey} data-vp="" data-testid="vp" tabIndex={0} ref={follow.viewportRef} />
    }
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    const { rerender } = render(<WithTab nodeKey="a" />)
    const node = vp()
    expect(node.getAttribute('tabindex')).toBe('-1')
    rerender(<WithTab nodeKey="b" />)
    expect(node.getAttribute('tabindex')).toBe('0')
  })

  it('on unmount it detaches listeners and cuts observers', () => {
    const { unmount } = mount(<Harness />)
    const node = vp()
    const removed = vi.spyOn(node, 'removeEventListener')
    const ro = liveRO()[0]
    unmount()
    expect(ro.disconnected).toBe(true)
    expect(removed.mock.calls.map(c => c[0]).sort()).toEqual(['keydown', 'scroll', 'wheel'])
  })

  it('it warns if there is no first child (content wrapper) — content growth cannot be seen', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<Harness withContent={false} />)
    expect(warn).toHaveBeenCalledWith(expect.stringContaining('[scrollFollow]'))
  })

  // ★Radix structure lock (TRD §9)★: content growth is watched through the first child wrapper Radix puts inside the Viewport; if Radix changes that structure, this breaks first.
  it('it observes the Viewport first child (the Radix wrapper) of the real ScrollArea', () => {
    vi.stubGlobal('ResizeObserver', FakeRO) // Radix internals may use it — ours is told apart by the subclass below.
    class OurRO extends FakeRO {}
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    function Real() {
      const follow = useScrollFollow('real', { ResizeObserver: OurRO as unknown as typeof ResizeObserver })
      return (
        <ScrollArea ref={follow.viewportRef}>
          <p>본문</p>
        </ScrollArea>
      )
    }
    render(<Real />)
    const viewport = document.querySelector('[data-radix-scroll-area-viewport]')!
    const ours = FakeRO.all.find(r => r instanceof OurRO)!
    expect(ours.observed).toEqual([viewport, viewport.firstElementChild])
    expect(viewport.getAttribute('data-scroll-follow')).toBe('pinned')
    expect(warn).not.toHaveBeenCalledWith(expect.stringContaining('[scrollFollow]'))
  })
})

describe('handle map (followRegistry)', () => {
  it('mount raises it, unmount erases it', () => {
    const { unmount } = render(<Harness slotId="r1" />)
    expect(getFollow('r1')!.pin).toBe(latest!.pin)
    unmount()
    expect(getFollow('r1')).toBeUndefined()
  })

  it('if a new mount of the same slot raised it first the old mount\'s lowering does not erase it', () => {
    const a = render(<Harness slotId="r2" />)
    render(<Harness slotId="r2" />)
    const newer = latest!
    a.unmount()
    expect(getFollow('r2')!.pin).toBe(newer.pin)
  })

  it('it is still up after a StrictMode double mount', () => {
    render(
      <StrictMode>
        <Harness slotId="r3" />
      </StrictMode>,
    )
    expect(getFollow('r3')).toBeDefined()
    expect(getFollow('r3')!.pinned).toBe(true)
  })

  it('the map handle pinned reads the current state', () => {
    render(<Harness slotId="r4" />)
    wheelUp()
    expect(getFollow('r4')!.pinned).toBe(false)
  })
})
