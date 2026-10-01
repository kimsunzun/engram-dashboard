// useScrollFollow — DOM 배선 시험(ADR-0242 · TRD §2-3 · §2-7). 판정 규칙 자체는 followCore.test.ts 가 잰다 —
//   여기는 입력이 코어에 제대로 먹히고 그 결과가 노드·관측 표면·손잡이 맵에 닿는지만 잰다.
//
// 전략: jsdom 엔 레이아웃도 ResizeObserver 도 없다. 가짜 RO 를 seam 으로 꽂고, `data-vp` 가 붙은 노드만
//   scrollHeight · clientHeight 를 갖게 Element 원형의 접근자를 바꾼다(scrollTop 은 노드마다 저장 + 바닥 클램프).
//   브라우저는 위치가 바뀐 노드에 **다음 프레임에** scroll 이벤트를 하나 낸다 — `frame()` 이 그 자리다.

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

/** 다음 프레임 — 위치가 바뀐 뷰포트마다 scroll 이벤트 하나. */
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

/** 붙이고 우리 첫 쓰기의 scroll 이벤트까지 받는다. */
function mount(ui: ReactElement): ReturnType<typeof render> {
  const out = render(ui)
  frame()
  return out
}

/** 사용자가 그 자리로 스크롤했다 — 앞선 쓰기의 이벤트가 먼저 오고, 그다음 이 움직임의 이벤트. */
function userScroll(top: number): void {
  frame()
  vp().scrollTop = top
  frame()
}

/** 스크롤할 것이 있는 상태에서 작은 위 휠로 따라가기를 푼다. */
function wheelUp(target: Element = vp()): void {
  fireEvent.wheel(target, { deltaY: -40 })
}

describe('붙기 · 따라가기', () => {
  it('붙으면 바닥으로 쓰고 data-scroll-follow="pinned"', () => {
    mount(<Harness />)
    expect(vp().scrollTop).toBe(bottom())
    expect(followAttr()).toBe('pinned')
    expect(latest!.pinned).toBe(true)
  })

  it('붙어 있으면 내용 성장을 바닥까지 따라간다', () => {
    mount(<Harness />)
    geo.height = 1600
    liveRO()[0].fire()
    expect(vp().scrollTop).toBe(1400)
  })

  it('뷰포트가 줄어도(입력창 위 대기 목록) 바닥에 남는다', () => {
    mount(<Harness />)
    geo.client = 120
    liveRO()[0].fire()
    expect(vp().scrollTop).toBe(880)
  })
})

describe('위로 가는 입력', () => {
  it('위 휠은 풀고(data-scroll-follow="free"), 떨어진 동안 성장은 끌어내리지 않는다', () => {
    mount(<Harness />)
    wheelUp()
    expect(followAttr()).toBe('free')
    expect(latest!.pinned).toBe(false)

    userScroll(bottom() - 10) // 그 휠이 만든 위 scroll(문턱 안) — 걸쇠가 되붙이지 않는다.
    geo.height = 1600
    liveRO()[0].fire()
    expect(vp().scrollTop).toBe(790)
    expect(latest!.pinned).toBe(false)
    expect(latest!.unseenGrowth).toBe(true)
  })

  it('아래 휠 · ctrl 휠(확대/축소)은 풀지 않는다', () => {
    mount(<Harness />)
    fireEvent.wheel(vp(), { deltaY: 40 })
    fireEvent.wheel(vp(), { deltaY: -40, ctrlKey: true })
    expect(followAttr()).toBe('pinned')
  })

  it('안쪽 Radix 뷰포트가 먹는 위 휠은 무시하고, 안쪽이 맨 위면 바깥의 위 의도로 읽는다', () => {
    mount(<Harness />)
    const inner = screen.getByTestId('inner')
    inner.scrollTop = 30
    wheelUp(screen.getByTestId('inner-text'))
    expect(followAttr()).toBe('pinned')

    inner.scrollTop = 0
    wheelUp(screen.getByTestId('inner-text'))
    expect(followAttr()).toBe('free')
  })

  it.each(['PageUp', 'Home', 'ArrowUp'])('키 %s 는 푼다', key => {
    mount(<Harness />)
    fireEvent.keyDown(vp(), { key })
    expect(followAttr()).toBe('free')
  })

  it('편집 칸 안의 키 · 조합 중 키 · 다른 키는 풀지 않는다', () => {
    mount(<Harness />)
    fireEvent.keyDown(screen.getByTestId('editable'), { key: 'ArrowUp' })
    fireEvent.keyDown(vp(), { key: 'ArrowUp', isComposing: true })
    fireEvent.keyDown(vp(), { key: 'ArrowDown' })
    expect(followAttr()).toBe('pinned')
  })

  // 뷰포트가 클릭으로 포커스를 받으므로(tabIndex -1) 키는 뷰포트를 과녁으로 온다.
  it('포커스를 받은 뷰포트의 ArrowUp 은 푼다', () => {
    mount(<Harness />)
    vp().focus()
    expect(document.activeElement).toBe(vp())
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowUp' })
    expect(followAttr()).toBe('free')
  })

  it('떨어진 채 아래로 스크롤해 문턱에 들면 되붙는다', () => {
    mount(<Harness />)
    wheelUp()
    userScroll(300)
    userScroll(bottom() - 5)
    expect(followAttr()).toBe('pinned')
    expect(latest!.pinned).toBe(true)
  })
})

describe('우리 쓰기와 사용자 스크롤이 겹칠 때', () => {
  // 성장 쓰기(800 → 1400)의 scroll 이벤트가 오기 전에 스크롤바를 끌었다 — 브라우저는 둘을 이벤트 하나로 합친다.
  it('쓰기의 이벤트가 오기 전 위로 끌면 푼다 — 다음 성장이 끌어내리지 않는다', () => {
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

describe('scrollable — 버튼 조건', () => {
  it('내용이 뷰포트보다 크면 참으로 시작한다', () => {
    mount(<Harness />)
    expect(latest!.scrollable).toBe(true)
  })

  it('떨어진 채 뷰포트가 커져 다 들어오면 거짓 — 붙음은 다시 재지 않는다(ADR-0242 결정 2)', () => {
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

  it('숨은 동안(client 0)은 마지막으로 잰 값을 그대로 둔다', () => {
    mount(<Harness />)
    geo.client = 0
    liveRO()[0].fire()
    expect(latest!.scrollable).toBe(true)
  })
})

describe('손잡이 pin · unpin', () => {
  it('pin 은 붙이고 바닥으로 쓴다 · unpin 은 위치를 그대로 둔다', () => {
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

  it('리렌더 뒤에도 viewportRef · pin · unpin 이 같은 함수다', () => {
    const { rerender } = mount(<Harness />)
    const first = latest!
    wheelUp() // pinned 상태가 바뀌어 리렌더가 한 번 돈다
    rerender(<Harness />)
    expect(latest).not.toBe(first)
    expect(latest!.viewportRef).toBe(first.viewportRef)
    expect(latest!.pin).toBe(first.pin)
    expect(latest!.unpin).toBe(first.unpin)
  })
})

describe('숨은 탭(display:none — client 0)', () => {
  it('얼어 있는 동안 pin 은 쓰지 않고 붙음만 세운다 — 보이면 바닥', () => {
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

  it('떨어진 채 숨었다 돌아오면 읽던 자리를 되살린다(보존 안 된 경우)', () => {
    mount(<Harness />)
    wheelUp()
    userScroll(300)
    geo.client = 0
    liveRO()[0].fire()
    tops.set(vp(), 0) // display:none 을 지나며 위치를 잃었다고 친다(보존되는지는 GUI 실측 몫 — TRD §8-2 F1 ①)
    geo.client = 200
    liveRO()[0].fire()
    expect(vp().scrollTop).toBe(300)
    expect(followAttr()).toBe('free')
  })

  it('숨은 동안 오는 scroll(높이 0 → 가짜 「바닥」)은 떨어진 상태를 되붙이지 않는다', () => {
    mount(<Harness />)
    wheelUp()
    userScroll(300)
    geo.client = 0
    geo.height = 0
    // 얼지 않고 읽으면 「아래로 움직여 바닥 거리 ≤ 문턱」이라 되붙는 값이다.
    tops.set(vp(), 310)
    fireEvent.scroll(vp())
    expect(followAttr()).toBe('free')
  })
})

describe('노드 수명', () => {
  it('노드가 바뀌면 옛 노드의 청취자·관찰자를 떼고 새 노드에 붙는다', () => {
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

    fireEvent.wheel(oldNode, { deltaY: -40 }) // 떼어 낸 옛 노드의 입력은 코어에 닿지 않는다
    expect(latest!.pinned).toBe(true)
  })

  it('뷰포트가 내려갔다 다시 붙어도(빈 상태 ↔ 대화) 붙음 상태를 이어 간다', () => {
    const { rerender } = mount(<Harness />)
    wheelUp()
    rerender(<Harness show={false} />)
    expect(liveRO()).toHaveLength(0)
    rerender(<Harness show />)
    expect(followAttr()).toBe('free')
  })

  it('붙으면 뷰포트가 클릭 포커스를 받게 하고(tabIndex -1 · 테두리 없음), 떼면 되돌린다', () => {
    const { rerender } = mount(<Harness />)
    const node = vp()
    expect(node.getAttribute('tabindex')).toBe('-1')
    expect(node.style.outline).toBe('none')
    rerender(<Harness nodeKey="b" />)
    expect(node.hasAttribute('tabindex')).toBe(false)
    expect(node.style.outline).toBe('')
  })

  it('원래 tabIndex 가 있던 노드는 떼면 그 값으로 되돌린다', () => {
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

  it('언마운트하면 청취자를 떼고 관찰자를 끊는다', () => {
    const { unmount } = mount(<Harness />)
    const node = vp()
    const removed = vi.spyOn(node, 'removeEventListener')
    const ro = liveRO()[0]
    unmount()
    expect(ro.disconnected).toBe(true)
    expect(removed.mock.calls.map(c => c[0]).sort()).toEqual(['keydown', 'scroll', 'wheel'])
  })

  it('첫 자식(내용 래퍼)이 없으면 경고한다 — 내용 성장을 못 본다', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    render(<Harness withContent={false} />)
    expect(warn).toHaveBeenCalledWith(expect.stringContaining('[scrollFollow]'))
  })

  // ★Radix 구조 자물쇠(TRD §9)★: 내용 성장은 Radix 가 Viewport 안에 두는 첫 자식 래퍼로 본다. Radix 가 그 구조를
  //   바꾸면 여기가 먼저 깨진다.
  it('실 ScrollArea 의 Viewport 첫 자식(Radix 래퍼)을 관찰한다', () => {
    vi.stubGlobal('ResizeObserver', FakeRO) // Radix 내부가 쓸 수 있다 — 우리 것은 아래 하위 클래스로 가른다.
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

describe('손잡이 맵(followRegistry)', () => {
  it('마운트하면 올리고, 언마운트하면 지운다', () => {
    const { unmount } = render(<Harness slotId="r1" />)
    expect(getFollow('r1')!.pin).toBe(latest!.pin)
    unmount()
    expect(getFollow('r1')).toBeUndefined()
  })

  it('같은 슬롯의 새 마운트가 먼저 올렸으면 옛 마운트의 내리기가 그것을 지우지 않는다', () => {
    const a = render(<Harness slotId="r2" />)
    render(<Harness slotId="r2" />)
    const newer = latest!
    a.unmount()
    expect(getFollow('r2')!.pin).toBe(newer.pin)
  })

  it('StrictMode 이중 마운트 뒤에도 올라 있다', () => {
    render(
      <StrictMode>
        <Harness slotId="r3" />
      </StrictMode>,
    )
    expect(getFollow('r3')).toBeDefined()
    expect(getFollow('r3')!.pinned).toBe(true)
  })

  it('맵의 손잡이 pinned 는 지금 상태를 읽는다', () => {
    render(<Harness slotId="r4" />)
    wheelUp()
    expect(getFollow('r4')!.pinned).toBe(false)
  })
})
