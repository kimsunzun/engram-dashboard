// Splitter 단위테스트 — 드래그·클릭·더블클릭·취소·잠김(ADR-0227, TRD §2f).
// jsdom 은 레이아웃이 없어 루트의 getBoundingClientRect 를 목으로 준다. 포인터 캡처 API 는 jsdom 에 없을 수 있어
// 컴포넌트가 선택적으로 부르고, 여기서는 창에 좌표를 실은 이벤트를 쏜다. rAF 는 손으로 비우는 큐로 바꾼다.

import { act, cleanup, fireEvent, render } from '@testing-library/react'
import { StrictMode, useRef } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { SplitRect } from '../../api/layoutTypes'
import Splitter, { type SplitterProps } from './Splitter'

const ROOT = { left: 100, top: 50, width: 1000, height: 500 }
const ROOT_RECT = {
  ...ROOT,
  x: ROOT.left,
  y: ROOT.top,
  right: ROOT.left + ROOT.width,
  bottom: ROOT.top + ROOT.height,
  toJSON: () => ({}),
} as DOMRect

let frames: Map<number, FrameRequestCallback>
let nextFrame = 0
const flushFrames = () => {
  const cbs = [...frames.values()]
  frames.clear()
  act(() => cbs.forEach(cb => cb(0)))
}

beforeEach(() => {
  frames = new Map()
  vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
    const id = ++nextFrame
    frames.set(id, cb)
    return id
  })
  vi.stubGlobal('cancelAnimationFrame', (id: number) => {
    frames.delete(id)
  })
  document.body.style.cursor = ''
  document.body.style.userSelect = ''
})
afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
})

const LR: SplitRect = { split_id: 'sp-lr', dir: 'left_right', x0: 0, y0: 0, x1: 1, y1: 1, at: 0.5 }
const TB: SplitRect = { split_id: 'sp-tb', dir: 'top_bottom', x0: 0.5, y0: 0.2, x1: 1, y1: 1, at: 0.6 }
const LR_HALF: SplitRect = { split_id: 'sp-lr-half', dir: 'left_right', x0: 0, y0: 0, x1: 0.5, y1: 1, at: 0.25 }
const RANGE = { lo: 0.1, hi: 0.9 }

type Handlers = Pick<SplitterProps, 'onDragStart' | 'onPreview' | 'onRelease' | 'onCancel' | 'onReset'>
const handlers = () => ({
  onDragStart: vi.fn(),
  onPreview: vi.fn(),
  onRelease: vi.fn(),
  onCancel: vi.fn(),
  onReset: vi.fn(),
})

function Harness(props: { split: SplitRect; range: SplitterProps['range'] } & Handlers) {
  const rootRef = useRef<HTMLDivElement>(null)
  return (
    <div ref={rootRef} data-testid="root">
      <Splitter rootRef={rootRef} {...props} />
    </div>
  )
}

function setup(
  split: SplitRect = LR,
  range: SplitterProps['range'] = RANGE,
  opts: { strict?: boolean } = {},
) {
  const h = handlers()
  const tree = (s: SplitRect, r: SplitterProps['range']) => {
    const inner = <Harness split={s} range={r} {...h} />
    return opts.strict ? <StrictMode>{inner}</StrictMode> : inner
  }
  const utils = render(tree(split, range))
  utils.getByTestId('root').getBoundingClientRect = () => ROOT_RECT
  const el = utils.container.querySelector<HTMLElement>(`[data-split-id="${split.split_id}"]`)!
  const rerender = (next: { split?: SplitRect; range?: SplitterProps['range'] }) =>
    utils.rerender(tree(next.split ?? split, next.range === undefined ? range : next.range))
  return { ...h, el, utils, rerender, called: () => callCount(h) }
}

const callCount = (h: ReturnType<typeof handlers>) =>
  h.onDragStart.mock.calls.length +
  h.onPreview.mock.calls.length +
  h.onRelease.mock.calls.length +
  h.onCancel.mock.calls.length +
  h.onReset.mock.calls.length

/** 한 루트 아래 여러 구분선 — 구분선마다 따로 콜백을 단다. */
function setupMany(splits: SplitRect[]) {
  const hs = splits.map(() => handlers())
  function Many() {
    const rootRef = useRef<HTMLDivElement>(null)
    return (
      <div ref={rootRef} data-testid="root">
        {splits.map((s, i) => (
          <Splitter key={s.split_id} rootRef={rootRef} split={s} range={RANGE} {...hs[i]} />
        ))}
      </div>
    )
  }
  const utils = render(<Many />)
  utils.getByTestId('root').getBoundingClientRect = () => ROOT_RECT
  return splits.map((s, i) => ({
    ...hs[i],
    el: utils.container.querySelector<HTMLElement>(`[data-split-id="${s.split_id}"]`)!,
    called: () => callCount(hs[i]),
  }))
}

// 좌우 분할은 x 축 — 루트 left 100·폭 1000 이라 clientX = 100 + 1000·정규화 좌표.
const at = (norm: number) => ROOT.left + ROOT.width * norm
const down = (el: HTMLElement, clientX: number, clientY = 300, extra: PointerEventInit = {}) =>
  fireEvent.pointerDown(el, { clientX, clientY, pointerId: 1, button: 0, ...extra })
// 누른 채 끄는 이동은 주 버튼이 눌린 상태(buttons bit 0)를 싣는다 — 컴포넌트가 버튼 없는 이동을 pointerup 유실로 본다.
const move = (clientX: number, clientY = 300, pointerId = 1, buttons = 1) => {
  let notPrevented = true
  act(() => {
    notPrevented = fireEvent.pointerMove(document.body, { clientX, clientY, pointerId, buttons })
  })
  return notPrevented
}
const up = (clientX: number, clientY = 300, pointerId = 1) =>
  act(() => {
    fireEvent.pointerUp(document.body, { clientX, clientY, pointerId })
  })

describe('Splitter — 표식', () => {
  it('data-split-id·data-dir·방향 커서·user-select 없음을 단다', () => {
    const { el } = setup(LR)
    expect(el.getAttribute('data-split-id')).toBe('sp-lr')
    expect(el.getAttribute('data-dir')).toBe('left_right')
    expect(el.style.cursor).toBe('col-resize')
    expect(el.style.zIndex).toBe('20')
    expect(el.style.userSelect).toBe('none')
    cleanup()
    const tb = setup(TB)
    expect(tb.el.getAttribute('data-dir')).toBe('top_bottom')
    expect(tb.el.style.cursor).toBe('row-resize')
  })
})

describe('Splitter — 드래그', () => {
  it('down/move/up → onDragStart(첫 비율) 1·onRelease 정확히 1 회(범위 안 값), onCancel 0', () => {
    const s = setup()
    down(s.el, at(0.5))
    move(at(0.7))
    expect(s.onDragStart).toHaveBeenCalledTimes(1)
    expect(s.onDragStart).toHaveBeenCalledWith(0.7)
    move(at(0.75))
    flushFrames()
    expect(s.onPreview).toHaveBeenCalledTimes(1)
    expect(s.onPreview).toHaveBeenLastCalledWith(0.75)
    up(at(0.75))
    expect(s.onRelease).toHaveBeenCalledTimes(1)
    expect(s.onRelease).toHaveBeenCalledWith(0.75)
    expect(s.onCancel).not.toHaveBeenCalled()
    // 뗀 뒤의 창 이벤트는 아무것도 부르지 않는다(리스너가 떨어졌다).
    move(at(0.3))
    up(at(0.3))
    flushFrames()
    expect(s.onRelease).toHaveBeenCalledTimes(1)
    expect(s.onPreview).toHaveBeenCalledTimes(1)
    expect(s.onDragStart).toHaveBeenCalledTimes(1)
  })

  it('onDragStart 비율도 범위 안으로 잘리고, 그 값은 onPreview 로 다시 오지 않는다', () => {
    const s = setup(LR, { lo: 0.2, hi: 0.8 })
    down(s.el, at(0.5))
    move(at(0.95))
    flushFrames()
    expect(s.onDragStart).toHaveBeenCalledWith(0.8)
    expect(s.onPreview).not.toHaveBeenCalled()
    up(at(0.95))
    expect(s.onPreview).not.toHaveBeenCalled()
    expect(s.onRelease).toHaveBeenCalledTimes(1)
    expect(s.onRelease).toHaveBeenCalledWith(0.8)
  })

  it('비율은 분할 상자 기준이다(위아래 분할 — y 축·루트 top/height)', () => {
    // TB 상자 y = [0.2,1] → y 정규화 0.84 면 (0.84 − 0.2)/0.8.
    const s = setup(TB)
    fireEvent.pointerDown(s.el, { clientX: 700, clientY: ROOT.top + ROOT.height * 0.6, pointerId: 1, button: 0 })
    move(700, ROOT.top + ROOT.height * 0.84)
    up(700, ROOT.top + ROOT.height * 0.84)
    expect(s.onRelease).toHaveBeenCalledTimes(1)
    expect(s.onRelease.mock.calls[0][0]).toBeCloseTo((0.84 - 0.2) / 0.8, 12)
  })

  it('pointermove 는 프레임당 한 번으로 합쳐지고 마지막 위치를 쓴다', () => {
    const s = setup()
    down(s.el, at(0.5))
    move(at(0.55))
    move(at(0.6))
    move(at(0.65))
    expect(s.onDragStart).toHaveBeenCalledWith(0.55)
    expect(s.onPreview).not.toHaveBeenCalled()
    flushFrames()
    expect(s.onPreview).toHaveBeenCalledTimes(1)
    expect(s.onPreview).toHaveBeenCalledWith(0.65)
    up(at(0.65))
  })

  it('밀린 프레임이 있으면 떼기 전에 그 값을 onPreview 로 먼저 내보낸다', () => {
    const s = setup()
    down(s.el, at(0.5))
    move(at(0.4))
    move(at(0.3))
    up(at(0.3))
    expect(s.onPreview).toHaveBeenCalledTimes(1)
    expect(s.onPreview).toHaveBeenCalledWith(0.3)
    expect(s.onRelease).toHaveBeenCalledWith(0.3)
    expect(s.onPreview.mock.invocationCallOrder[0]).toBeLessThan(s.onRelease.mock.invocationCallOrder[0])
    // 취소된 프레임은 뒤늦게 돌지 않는다.
    flushFrames()
    expect(s.onPreview).toHaveBeenCalledTimes(1)
  })

  it('떼는 값은 마지막 pointermove 위치다(pointerup 좌표가 달라도)', () => {
    const s = setup()
    down(s.el, at(0.5))
    move(at(0.4))
    up(at(0.9))
    expect(s.onRelease).toHaveBeenCalledWith(0.4)
  })

  it('드래그 중엔 body 커서·user-select 를 잠그고 뗀 뒤 되돌린다', () => {
    document.body.style.cursor = 'wait'
    const s = setup()
    down(s.el, at(0.5))
    move(at(0.6))
    expect(document.body.style.cursor).toBe('col-resize')
    expect(document.body.style.userSelect).toBe('none')
    up(at(0.6))
    expect(document.body.style.cursor).toBe('wait')
    expect(document.body.style.userSelect).toBe('')
  })

  it('다른 pointerId 의 이벤트는 무시한다', () => {
    const s = setup()
    down(s.el, at(0.5))
    move(at(0.7), 300, 2)
    up(at(0.7), 300, 2)
    expect(s.called()).toBe(0)
    move(at(0.7))
    up(at(0.7))
    expect(s.onRelease).toHaveBeenCalledWith(0.7)
  })

  it('주 버튼이 아니면 드래그하지 않는다', () => {
    const s = setup()
    down(s.el, at(0.5), 300, { button: 2 })
    move(at(0.7))
    up(at(0.7))
    flushFrames()
    expect(s.called()).toBe(0)
  })

  it('포인터 캡처 소실은 취소가 아니다 — 창 수준 리스너가 떼기까지 받는다', () => {
    const s = setup()
    down(s.el, at(0.5))
    move(at(0.6))
    fireEvent(s.el, new Event('lostpointercapture'))
    move(at(0.65))
    up(at(0.65))
    expect(s.onCancel).not.toHaveBeenCalled()
    expect(s.onRelease).toHaveBeenCalledWith(0.65)
  })

  it('StrictMode(이펙트 이중 실행)에서도 드래그 1 회·body 복원', () => {
    const s = setup(LR, RANGE, { strict: true })
    down(s.el, at(0.5))
    move(at(0.6))
    expect(document.body.style.userSelect).toBe('none')
    up(at(0.6))
    expect(s.onDragStart).toHaveBeenCalledTimes(1)
    expect(s.onRelease).toHaveBeenCalledTimes(1)
    expect(s.onCancel).not.toHaveBeenCalled()
    expect(document.body.style.userSelect).toBe('')
    expect(document.body.style.cursor).toBe('')
  })
})

describe('Splitter — 기본 동작(텍스트 선택·호환 mousedown)', () => {
  it('pointerdown 은 기본 동작을 막지 않는다 — 문서 mousedown 으로 닫히는 메뉴가 계속 닫힌다', () => {
    const s = setup()
    expect(down(s.el, at(0.5))).toBe(true)
    up(at(0.5))
  })

  it('누른 동안의 pointermove 는 기본 동작을 막는다(움직이기 전 포함), 뗀 뒤는 막지 않는다', () => {
    const s = setup()
    down(s.el, at(0.5))
    expect(move(at(0.5), 320)).toBe(false)
    expect(move(at(0.6))).toBe(false)
    up(at(0.6))
    expect(move(at(0.7))).toBe(true)
  })
})

describe('Splitter — 클릭·더블클릭', () => {
  it('움직임 없는 클릭(분할 축 좌표 불변) → 아무 콜백도 없다', () => {
    const s = setup()
    down(s.el, at(0.5), 300)
    move(at(0.5), 320) // 분할 축(x)은 그대로, 직교 축만 움직임
    up(at(0.5), 320)
    fireEvent.click(s.el)
    flushFrames()
    expect(s.called()).toBe(0)
    expect(document.body.style.cursor).toBe('')
  })

  it('더블클릭 → onReset 1 회, 그 사이 클릭 두 번은 아무것도 부르지 않는다', () => {
    const s = setup()
    for (let i = 0; i < 2; i++) {
      down(s.el, at(0.5))
      up(at(0.5))
      fireEvent.click(s.el)
    }
    fireEvent.doubleClick(s.el)
    expect(s.onReset).toHaveBeenCalledTimes(1)
    expect(s.called()).toBe(1)
  })
})

describe('Splitter — 여러 포인터', () => {
  it('누르는 동안 다른 포인터의 pointerdown 은 무시한다(진행 중 드래그 유지)', () => {
    const s = setup()
    down(s.el, at(0.5))
    move(at(0.6))
    down(s.el, at(0.3), 300, { pointerId: 2 })
    move(at(0.3), 300, 2)
    up(at(0.3), 300, 2)
    expect(s.onCancel).not.toHaveBeenCalled()
    move(at(0.7))
    up(at(0.7))
    expect(s.onDragStart).toHaveBeenCalledTimes(1)
    expect(s.onRelease).toHaveBeenCalledTimes(1)
    expect(s.onRelease).toHaveBeenCalledWith(0.7)
  })

  it('같은 포인터가 떼지 않고 다시 누르면(pointerup 유실) 앞 드래그를 취소하고 새로 잡는다', () => {
    const s = setup()
    down(s.el, at(0.5))
    move(at(0.6))
    down(s.el, at(0.4))
    expect(s.onCancel).toHaveBeenCalledTimes(1)
    move(at(0.3))
    up(at(0.3))
    expect(s.onDragStart).toHaveBeenCalledTimes(2)
    expect(s.onRelease).toHaveBeenCalledTimes(1)
    expect(s.onRelease).toHaveBeenCalledWith(0.3)
  })

  it('같은 포인터가 다른 구분선을 누르면 앞 구분선의 드래그를 취소하고, body 잠금은 남은 드래그가 다 끝날 때 풀린다', () => {
    document.body.style.cursor = 'wait'
    const [a, b, c] = setupMany([LR, TB, LR_HALF])

    // C 는 다른 포인터로 끌리는 중 — 아래 교체와 무관하게 끝까지 잠금을 쥔다.
    down(c.el, at(0.25), 300, { pointerId: 2 })
    move(at(0.3), 300, 2)
    down(a.el, at(0.5), 300, { pointerId: 1 })
    move(at(0.6), 300, 1)
    expect(a.onDragStart).toHaveBeenCalledWith(0.6)

    fireEvent.pointerDown(b.el, { clientX: 800, clientY: 350, pointerId: 1, button: 0 })
    expect(a.onCancel).toHaveBeenCalledTimes(1)
    expect(document.body.style.userSelect).toBe('none')

    // x 도 움직여 A 가 아직 살아 있다면 다른 비율로 반응하게 한다.
    move(800, 400, 1)
    move(820, 420, 1)
    flushFrames()
    up(820, 420, 1)
    expect(b.onDragStart).toHaveBeenCalledTimes(1)
    expect(b.onDragStart.mock.calls[0][0]).toBeCloseTo((0.7 - 0.2) / 0.8, 12)
    expect(b.onRelease).toHaveBeenCalledTimes(1)
    expect(b.onRelease.mock.calls[0][0]).toBeCloseTo((0.74 - 0.2) / 0.8, 12)
    expect(b.onCancel).not.toHaveBeenCalled()
    expect(a.onDragStart).toHaveBeenCalledTimes(1)
    expect(a.onPreview).not.toHaveBeenCalled()
    expect(a.onRelease).not.toHaveBeenCalled()
    expect(a.onCancel).toHaveBeenCalledTimes(1)
    expect(c.called()).toBe(1)
    // C 가 아직 끌리는 중이라 잠금이 남는다.
    expect(document.body.style.cursor).toBe('col-resize')
    expect(document.body.style.userSelect).toBe('none')

    up(at(0.3), 300, 2)
    expect(c.onRelease).toHaveBeenCalledWith(0.6)
    expect(c.onCancel).not.toHaveBeenCalled()
    expect(document.body.style.cursor).toBe('wait')
    expect(document.body.style.userSelect).toBe('')
  })

  it('두 구분선을 동시에 끌면 body 스타일은 마지막 드래그가 끝날 때 원래 값으로 돌아간다', () => {
    document.body.style.cursor = 'wait'
    const a = handlers()
    const b = handlers()
    function Two() {
      const rootRef = useRef<HTMLDivElement>(null)
      return (
        <div ref={rootRef} data-testid="root">
          <Splitter rootRef={rootRef} split={LR} range={RANGE} {...a} />
          <Splitter rootRef={rootRef} split={TB} range={RANGE} {...b} />
        </div>
      )
    }
    const utils = render(<Two />)
    utils.getByTestId('root').getBoundingClientRect = () => ROOT_RECT
    const elA = utils.container.querySelector<HTMLElement>('[data-split-id="sp-lr"]')!
    const elB = utils.container.querySelector<HTMLElement>('[data-split-id="sp-tb"]')!

    down(elA, at(0.5), 300, { pointerId: 1 })
    move(at(0.6), 300, 1)
    fireEvent.pointerDown(elB, { clientX: 700, clientY: 350, pointerId: 2, button: 0 })
    move(700, 400, 2)
    expect(document.body.style.cursor).toBe('row-resize')
    up(at(0.6), 300, 1)
    expect(a.onRelease).toHaveBeenCalledTimes(1)
    // B 는 아직 끌리는 중 — 잠금이 남는다.
    expect(document.body.style.userSelect).toBe('none')
    expect(document.body.style.cursor).toBe('row-resize')
    up(700, 400, 2)
    expect(b.onRelease).toHaveBeenCalledTimes(1)
    expect(document.body.style.cursor).toBe('wait')
    expect(document.body.style.userSelect).toBe('')
  })
})

describe('Splitter — 취소', () => {
  it('pointercancel → onCancel 1 회, onRelease 없음, body 스타일 복원', () => {
    const s = setup()
    down(s.el, at(0.5))
    move(at(0.6))
    act(() => {
      fireEvent.pointerCancel(document.body, { pointerId: 1 })
    })
    expect(s.onCancel).toHaveBeenCalledTimes(1)
    up(at(0.6))
    flushFrames()
    expect(s.onRelease).not.toHaveBeenCalled()
    expect(s.onCancel).toHaveBeenCalledTimes(1)
    expect(document.body.style.cursor).toBe('')
    expect(document.body.style.userSelect).toBe('')
  })

  it('주 버튼 없이 온 pointermove(pointerup 유실) → onCancel 1 회, 확정 없음, 리스너·프레임·body 스타일 정리', () => {
    document.body.style.cursor = 'wait'
    const s = setup()
    down(s.el, at(0.5))
    move(at(0.6))
    move(at(0.65))
    expect(document.body.style.cursor).toBe('col-resize')
    expect(move(at(0.7), 300, 1, 0)).toBe(true)
    expect(s.onCancel).toHaveBeenCalledTimes(1)
    expect(s.onRelease).not.toHaveBeenCalled()
    expect(document.body.style.cursor).toBe('wait')
    expect(document.body.style.userSelect).toBe('')
    flushFrames()
    // 리스너가 떨어졌다 — 다시 버튼을 쥔 이동·떼기에도 아무것도 부르지 않고 기본 동작도 막지 않는다.
    expect(move(at(0.8))).toBe(true)
    up(at(0.8))
    flushFrames()
    expect(s.onPreview).not.toHaveBeenCalled()
    expect(s.onRelease).not.toHaveBeenCalled()
    expect(s.onDragStart).toHaveBeenCalledTimes(1)
    expect(s.onCancel).toHaveBeenCalledTimes(1)
  })

  it('움직이기 전 주 버튼 없이 온 pointermove → 아무 콜백 없이 누름이 끝난다', () => {
    const s = setup()
    down(s.el, at(0.5))
    move(at(0.7), 300, 1, 0)
    move(at(0.8))
    up(at(0.8))
    flushFrames()
    expect(s.called()).toBe(0)
    expect(document.body.style.cursor).toBe('')
  })

  it('창 blur → onCancel 1 회, onRelease 없음', () => {
    const s = setup()
    down(s.el, at(0.5))
    move(at(0.6))
    act(() => {
      fireEvent.blur(window)
    })
    up(at(0.6))
    expect(s.onCancel).toHaveBeenCalledTimes(1)
    expect(s.onRelease).not.toHaveBeenCalled()
  })

  it('창 안 요소의 blur 는 드래그를 끊지 않는다', () => {
    const s = setup()
    const input = document.createElement('input')
    document.body.appendChild(input)
    input.focus()
    down(s.el, at(0.5))
    move(at(0.6))
    act(() => {
      input.blur()
    })
    up(at(0.6))
    input.remove()
    expect(s.onCancel).not.toHaveBeenCalled()
    expect(s.onRelease).toHaveBeenCalledWith(0.6)
  })

  it('움직이기 전의 pointercancel·blur 는 아무 콜백도 부르지 않는다', () => {
    const s = setup()
    down(s.el, at(0.5))
    act(() => {
      fireEvent.pointerCancel(document.body, { pointerId: 1 })
    })
    down(s.el, at(0.5))
    act(() => {
      fireEvent.blur(window)
    })
    expect(s.called()).toBe(0)
  })

  it('드래그 중 언마운트 → onCancel 1 회, 리스너·프레임·body 스타일 정리', () => {
    const s = setup()
    down(s.el, at(0.5))
    move(at(0.6))
    move(at(0.65))
    s.utils.unmount()
    expect(s.onCancel).toHaveBeenCalledTimes(1)
    expect(document.body.style.cursor).toBe('')
    expect(document.body.style.userSelect).toBe('')
    flushFrames()
    move(at(0.7))
    up(at(0.7))
    flushFrames()
    expect(s.onPreview).not.toHaveBeenCalled()
    expect(s.onRelease).not.toHaveBeenCalled()
  })

  it('드래그 중 잠기면(range → null) onCancel', () => {
    const s = setup()
    down(s.el, at(0.5))
    move(at(0.6))
    act(() => s.rerender({ range: null }))
    expect(s.onCancel).toHaveBeenCalledTimes(1)
    up(at(0.6))
    expect(s.onRelease).not.toHaveBeenCalled()
  })

  it('드래그 중 같은 split id 의 방향이 바뀌면 onCancel', () => {
    const s = setup()
    down(s.el, at(0.5))
    move(at(0.6))
    act(() => s.rerender({ split: { ...LR, dir: 'top_bottom' } }))
    expect(s.onCancel).toHaveBeenCalledTimes(1)
    up(at(0.6))
    expect(s.onRelease).not.toHaveBeenCalled()
  })

  it('드래그 중 prop 의 split id 가 바뀌면 onCancel — 다른 분할에 확정하지 않는다', () => {
    const s = setup()
    down(s.el, at(0.5))
    move(at(0.6))
    act(() => s.rerender({ split: { ...LR, split_id: 'sp-other' } }))
    expect(s.onCancel).toHaveBeenCalledTimes(1)
    move(at(0.7))
    up(at(0.7))
    expect(s.onRelease).not.toHaveBeenCalled()
  })
})

describe('Splitter — 잠김(range null)', () => {
  it('드래그·더블클릭 모두 아무것도 부르지 않고 방향 커서를 달지 않는다', () => {
    const s = setup(LR, null)
    down(s.el, at(0.5))
    move(at(0.7))
    up(at(0.7))
    fireEvent.doubleClick(s.el)
    flushFrames()
    expect(s.called()).toBe(0)
    expect(s.el.style.cursor).toBe('default')
    expect(document.body.style.cursor).toBe('')
  })
})
