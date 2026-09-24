// useSplitDrag 단위테스트 — 제스처 token·확정·화해·당김 타이머·허용 범위·뷰 전환(ADR-0227, TRD §2f).
// 실물 viewStore 캐시와 실물 캔버스 보고 모듈 위에서 돈다. 동작을 흉내 내는 목은 invoke 하나다 — 확정
//   (set_split_ratio)·당김(get_view)·캔버스 보고(report_window_canvas)가 모두 그것을 지난다. event·window 모듈은
//   viewStore 의 import 를 풀려고 빈 껍데기로만 둔다. 구분선 콜백은 Splitter 대신 직접 부른다.

import { act, cleanup, renderHook } from '@testing-library/react'
import { Activity, StrictMode, type ReactNode, type RefObject } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const invokeMock = vi.fn(async (_cmd: string, _args?: unknown): Promise<unknown> => undefined)
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: unknown) => invokeMock(cmd, args),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ close: vi.fn(async () => undefined), label: () => 'main' }),
}))

import type { SplitRatioApplied, SplitRatioOutcome, SplitRect, ViewSnapshot } from '../../api/layoutTypes'
import { useViewStore } from '../../store/viewStore'
import { COMMIT_PULL_DELAY_MS, MIN_PANE_PX, ratioRange, remapRects } from './splitPreview'
import { useSplitDrag } from './useSplitDrag'
import { __resetCanvasReportForTest, getCanvasReport, submitCanvasSize } from './windowCanvasReport'

const VIEW = 'v1'

// sp-a: 좌우 [0,1]×[0,1] (a = s1, b = sp-b)
// sp-b: 위아래 [a,1]×[0,1] at 0.5 (a = sp-c, b = s4)
// sp-c: 좌우 [a,1]×[0,0.5] (a = s2, b = s3) — withC=false 면 닫혀 s2 가 그 자리를 채운다.
function snap(version: number, p: { a?: number; cAt?: number; withC?: boolean } = {}): ViewSnapshot {
  const a = p.a ?? 0.5
  const withC = p.withC ?? true
  const cAt = p.cAt ?? a + (1 - a) * 0.5
  return {
    view_id: VIEW,
    layout: { type: 'slot', id: 's1', content: { type: 'empty' } },
    focused_slot_id: null,
    slot_spatial: [],
    slot_rects: [
      { slot_id: 's1', x0: 0, y0: 0, x1: a, y1: 1 },
      ...(withC
        ? [
            { slot_id: 's2', x0: a, y0: 0, x1: cAt, y1: 0.5 },
            { slot_id: 's3', x0: cAt, y0: 0, x1: 1, y1: 0.5 },
          ]
        : [{ slot_id: 's2', x0: a, y0: 0, x1: 1, y1: 0.5 }]),
      { slot_id: 's4', x0: a, y0: 0.5, x1: 1, y1: 1 },
    ],
    split_rects: [
      { split_id: 'sp-a', dir: 'left_right', x0: 0, y0: 0, x1: 1, y1: 1, at: a },
      { split_id: 'sp-b', dir: 'top_bottom', x0: a, y0: 0, x1: 1, y1: 1, at: 0.5 },
      ...(withC
        ? [{ split_id: 'sp-c', dir: 'left_right' as const, x0: a, y0: 0, x1: 1, y1: 0.5, at: cAt }]
        : []),
    ],
    ratio_min: 0.1,
    ratio_max: 0.9,
    version,
  }
}

const VIEW2 = 'v2'

// 다른 뷰: 좌우 분할 하나(w-sp) — w1 | w2.
function snapV2(version: number, at = 0.5): ViewSnapshot {
  return {
    view_id: VIEW2,
    layout: { type: 'slot', id: 'w1', content: { type: 'empty' } },
    focused_slot_id: null,
    slot_spatial: [],
    slot_rects: [
      { slot_id: 'w1', x0: 0, y0: 0, x1: at, y1: 1 },
      { slot_id: 'w2', x0: at, y0: 0, x1: 1, y1: 1 },
    ],
    split_rects: [{ split_id: 'w-sp', dir: 'left_right', x0: 0, y0: 0, x1: 1, y1: 1, at }],
    ratio_min: 0.1,
    ratio_max: 0.9,
    version,
  }
}

/** 재사상 식과 같은 모양 — 분할 상자 `[x0,x1]` 안의 비율 `r` 의 경계. */
const boundary = (x0: number, x1: number, r: number): number => x0 + (x1 - x0) * r

const cached = (viewId = VIEW) => useViewStore.getState().layouts[viewId]

/** 호스트 렌더 수 — 헛 디스패치가 렌더를 부르는지 잰다(StrictMode 면 두 배로 센다). */
let renders = 0

function useHarness(rootRef: RefObject<HTMLElement | null>, viewId: string) {
  renders += 1
  const c = useViewStore(s => s.layouts[viewId])
  return useSplitDrag({
    viewId,
    slotRects: c.slotRects,
    splitRects: c.splitRects,
    ratioMin: c.ratioMin,
    ratioMax: c.ratioMax,
    version: c.version,
    rootRef,
  })
}

function mount(
  opts: {
    strict?: boolean
    root?: HTMLElement | null
    wrapper?: (p: { children: ReactNode }) => ReactNode
  } = {},
) {
  const rootRef = { current: opts.root ?? null }
  return renderHook(({ viewId }: { viewId: string }) => useHarness(rootRef, viewId), {
    initialProps: { viewId: VIEW },
    wrapper: opts.wrapper ?? (opts.strict ? StrictMode : undefined),
  })
}

type Hook = ReturnType<typeof mount>

const displaySplit = (r: Hook, id: string): SplitRect =>
  r.result.current.splitRects.find(s => s.split_id === id)!
const props = (r: Hook, id: string) => r.result.current.splitterProps(displaySplit(r, id))
const slotX1 = (r: Hook, id: string): number => r.result.current.slotRects.find(s => s.slot_id === id)!.x1

const ticks = async (): Promise<void> => {
  for (let i = 0; i < 20; i++) await Promise.resolve()
}
const apply = (s: ViewSnapshot) => act(() => useViewStore.getState().applyLayoutUpdated(s))
const advance = (ms: number) =>
  act(async () => {
    await vi.advanceTimersByTimeAsync(ms)
    await ticks()
  })

async function reportCanvas(w: number, h: number): Promise<void> {
  await act(async () => {
    submitCanvasSize(getCanvasReport(), { w, h })
    await ticks()
  })
  expect(getCanvasReport().lastSent).toEqual({ w, h })
}

const rangeFor = (axisPx: number, boxExtent: number) =>
  ratioRange({ ratioMin: 0.1, ratioMax: 0.9, minPanePx: MIN_PANE_PX, axisPx, boxExtent })

function rootWithRect(width: number, height: number): HTMLElement {
  const el = document.createElement('div')
  el.getBoundingClientRect = () =>
    ({ x: 0, y: 0, left: 0, top: 0, right: width, bottom: height, width, height, toJSON: () => ({}) }) as DOMRect
  return el
}

// set_split_ratio 는 테스트가 손수 끝낸다. get_view 는 `pullReply` 를 돌려준다(프라미스면 그것이 끝날 때).
let pendingRatio: Array<{ resolve: (v: SplitRatioApplied) => void; reject: (e: Error) => void }>
let pullReply: ViewSnapshot | Promise<ViewSnapshot> | null

const ratioCalls = () =>
  invokeMock.mock.calls.filter(c => c[0] === 'set_split_ratio').map(c => c[1])
const getViewCalls = () => invokeMock.mock.calls.filter(c => c[0] === 'get_view').length

const respond = (i: number, outcome: SplitRatioOutcome, version: number, ratio = 0.4) =>
  act(async () => {
    pendingRatio[i].resolve({ ratio, outcome, version })
    await ticks()
  })

beforeEach(() => {
  renders = 0
  pendingRatio = []
  pullReply = null
  invokeMock.mockReset()
  invokeMock.mockImplementation(async (cmd: string) => {
    if (cmd === 'set_split_ratio') {
      return new Promise<SplitRatioApplied>((resolve, reject) => pendingRatio.push({ resolve, reject }))
    }
    if (cmd === 'get_view') {
      if (pullReply === null) throw new Error('no pull reply')
      return pullReply
    }
    return undefined
  })
  __resetCanvasReportForTest()
  useViewStore.setState({ layouts: {}, windows: {}, renderModeOverride: {} })
  useViewStore.getState().applyLayoutUpdated(snap(10))
})
afterEach(() => {
  cleanup()
  vi.useRealTimers()
  vi.restoreAllMocks()
})

describe('useSplitDrag — 확정과 화해', () => {
  it.each([false, true])('끌면 미리보기를 그리고, 떼면 한 번 확정하고, Applied(N) 스냅샷이 올 때까지 미리보기를 쥔다 %#', async strict => {
    const r = mount({ strict })
    act(() => props(r, 'sp-a').onDragStart(0.3))
    expect(slotX1(r, 's1')).toBe(0.3)
    act(() => props(r, 'sp-a').onPreview(0.4))
    expect(slotX1(r, 's1')).toBe(0.4)

    act(() => props(r, 'sp-a').onRelease(0.4))
    expect(ratioCalls()).toEqual([{ viewId: VIEW, splitId: 'sp-a', ratio: 0.4 }])

    // 응답 전 스냅샷(규칙 3) — 나머지는 반영하되 끌린 분할은 미리보기 그대로.
    apply(snap(11))
    expect(slotX1(r, 's1')).toBe(0.4)
    await respond(0, 'Applied', 13)
    expect(slotX1(r, 's1')).toBe(0.4)
    // 기다리는 version 미만의 스냅샷도 미리보기를 못 버린다.
    apply(snap(12))
    expect(slotX1(r, 's1')).toBe(0.4)

    apply(snap(13, { a: 0.4 }))
    expect(r.result.current.slotRects).toBe(cached().slotRects)
    expect(r.result.current.splitRects).toBe(cached().splitRects)
    // 미리보기가 정말 버려졌으면 그 뒤 스냅샷(LLM 쓰기 등)을 그대로 따른다.
    apply(snap(14, { a: 0.6 }))
    expect(slotX1(r, 's1')).toBe(0.6)
    expect(ratioCalls()).toHaveLength(1)
  })

  it.each<SplitRatioOutcome>(['Unchanged', 'TooSmall'])('%s 응답이면 미리보기를 곧바로 버린다', async outcome => {
    vi.useFakeTimers()
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.3))
    act(() => props(r, 'sp-a').onRelease(0.3))
    expect(slotX1(r, 's1')).toBe(0.3)

    await respond(0, outcome, 99, 0.5)
    expect(r.result.current.slotRects).toBe(cached().slotRects)
    expect(slotX1(r, 's1')).toBe(0.5)
    // 스냅샷이 오지 않는 결말이라 당기지도 않는다.
    await advance(COMMIT_PULL_DELAY_MS * 4)
    expect(getViewCalls()).toBe(0)
  })

  it('명령이 실패하면 미리보기를 버리고 console.error 를 남긴다', async () => {
    const err = vi.spyOn(console, 'error').mockImplementation(() => {})
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.3))
    act(() => props(r, 'sp-a').onRelease(0.3))

    await act(async () => {
      pendingRatio[0].reject(new Error('boom'))
      await ticks()
    })
    expect(slotX1(r, 's1')).toBe(0.5)
    expect(err).toHaveBeenCalledWith(expect.stringContaining('set_split_ratio(sp-a, 0.3)'), expect.any(Error))
  })

  it('Applied(N) 스냅샷이 응답보다 먼저 와도 응답 순간 버린다 — 응답 때의 캐시 version 을 읽는다', async () => {
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.4))
    act(() => props(r, 'sp-a').onRelease(0.4))
    // 응답 전이라 미리보기를 쥔다. 스냅샷 값이 미리보기와 달라야 쥐고 있는지 보인다.
    apply(snap(12, { a: 0.45 }))
    expect(slotX1(r, 's1')).toBe(0.4)

    await respond(0, 'Applied', 12)
    expect(slotX1(r, 's1')).toBe(0.45)
    expect(r.result.current.slotRects).toBe(cached().slotRects)
  })

  it('끄는 중 스냅샷은 나머지를 전부 반영하고 끌린 분할만 미리보기를 유지한다(규칙 1)', () => {
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.3))
    apply(snap(11, { cAt: 0.8 }))

    const expected = remapRects(cached().slotRects, cached().splitRects, {
      token: 0,
      splitId: 'sp-a',
      ratio: 0.3,
      phase: 'drag',
    })
    expect(r.result.current.slotRects).toEqual(expected.slotRects)
    expect(r.result.current.splitRects).toEqual(expected.splitRects)
    expect(slotX1(r, 's1')).toBe(0.3)
  })

  it('끌던 분할이 스냅샷에서 사라지면 드래그를 거둔다 — 그 뒤 떼어도 확정하지 않는다', () => {
    const r = mount()
    const c = props(r, 'sp-c')
    act(() => c.onDragStart(0.8))
    expect(slotX1(r, 's2')).toBe(boundary(0.5, 1, 0.8))

    apply(snap(11, { withC: false }))
    expect(r.result.current.slotRects).toBe(cached().slotRects)
    act(() => c.onRelease(0.8))
    expect(ratioCalls()).toEqual([])

    // 다른 구분선은 그대로 돈다.
    act(() => props(r, 'sp-a').onDragStart(0.3))
    act(() => props(r, 'sp-a').onRelease(0.3))
    expect(ratioCalls()).toEqual([{ viewId: VIEW, splitId: 'sp-a', ratio: 0.3 }])
  })

  it('더블클릭은 미리보기 없이 0.5 를 한 번 보낸다', async () => {
    const err = vi.spyOn(console, 'error').mockImplementation(() => {})
    const r = mount()
    const before = r.result.current.slotRects
    act(() => props(r, 'sp-b').onReset())

    expect(ratioCalls()).toEqual([{ viewId: VIEW, splitId: 'sp-b', ratio: 0.5 }])
    expect(r.result.current.slotRects).toBe(before)

    await act(async () => {
      pendingRatio[0].reject(new Error('boom'))
      await ticks()
    })
    expect(err).toHaveBeenCalledWith(expect.stringContaining('set_split_ratio(sp-b, 0.5)'), expect.any(Error))
    expect(ratioCalls()).toHaveLength(1)
  })
})

describe('useSplitDrag — 제스처 token 은 구분선마다', () => {
  it('B 의 새 드래그가 미리보기를 가져가면 A 를 떼어도 확정하지 않고, B 는 그대로 돈다', () => {
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.3))
    act(() => props(r, 'sp-c').onDragStart(0.8))
    expect(slotX1(r, 's1')).toBe(0.5)
    expect(slotX1(r, 's2')).toBe(boundary(0.5, 1, 0.8))

    act(() => props(r, 'sp-a').onRelease(0.3))
    expect(ratioCalls()).toEqual([])
    expect(slotX1(r, 's2')).toBe(boundary(0.5, 1, 0.8))

    act(() => props(r, 'sp-c').onPreview(0.85))
    expect(slotX1(r, 's2')).toBe(boundary(0.5, 1, 0.85))
    act(() => props(r, 'sp-c').onRelease(0.85))
    expect(ratioCalls()).toEqual([{ viewId: VIEW, splitId: 'sp-c', ratio: 0.85 }])
  })

  it('A 의 취소·늦은 미리보기는 B 의 미리보기를 건드리지 않는다', () => {
    const r = mount()
    const a = props(r, 'sp-a')
    act(() => a.onDragStart(0.3))
    act(() => props(r, 'sp-c').onDragStart(0.8))

    act(() => a.onPreview(0.35))
    expect(slotX1(r, 's1')).toBe(0.5)
    act(() => a.onCancel())
    expect(slotX1(r, 's2')).toBe(boundary(0.5, 1, 0.8))

    act(() => props(r, 'sp-c').onRelease(0.8))
    expect(ratioCalls()).toEqual([{ viewId: VIEW, splitId: 'sp-c', ratio: 0.8 }])
  })

  it('확정을 기다리는 중 새 드래그가 시작되면 옛 응답은 무시된다(규칙 5)', async () => {
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.3))
    act(() => props(r, 'sp-a').onRelease(0.3))
    act(() => props(r, 'sp-a').onDragStart(0.6))

    await respond(0, 'Unchanged', 99, 0.5)
    expect(slotX1(r, 's1')).toBe(0.6)
  })

  // 아래 셋은 한 act 안에서 부른다 — 사이에 렌더가 없어도 소유 판정이 맞아야 한다(렌더에 맞춘 ref 로는 틀린다).
  it('같은 틱에 B 가 미리보기를 가져가고 A 가 떼면 A 는 확정하지 않는다', () => {
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.3))
    const a = props(r, 'sp-a')
    const c = props(r, 'sp-c')
    act(() => {
      c.onDragStart(0.8)
      a.onRelease(0.3)
    })
    expect(ratioCalls()).toEqual([])
  })

  it('같은 틱에 A 가 먼저 떼고 B 가 시작하면 A 는 한 번 확정한다', () => {
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.3))
    const a = props(r, 'sp-a')
    const c = props(r, 'sp-c')
    act(() => {
      a.onRelease(0.3)
      c.onDragStart(0.8)
    })
    expect(ratioCalls()).toEqual([{ viewId: VIEW, splitId: 'sp-a', ratio: 0.3 }])
  })

  it('한 틱 안에서 시작하고 떼도 한 번 확정한다', () => {
    const r = mount()
    const p = props(r, 'sp-a')
    act(() => {
      p.onDragStart(0.3)
      p.onRelease(0.3)
    })
    expect(ratioCalls()).toEqual([{ viewId: VIEW, splitId: 'sp-a', ratio: 0.3 }])
  })
})

describe('useSplitDrag — 확정 뒤 당김(규칙 6)', () => {
  it.each([false, true])('Applied 뒤 스냅샷이 안 오면 %#: 500ms 에 get_view 를 정확히 한 번 당긴다', async strict => {
    vi.useFakeTimers()
    const r = mount({ strict })
    act(() => props(r, 'sp-a').onDragStart(0.4))
    act(() => props(r, 'sp-a').onRelease(0.4))
    await respond(0, 'Applied', 12)

    await advance(COMMIT_PULL_DELAY_MS - 1)
    expect(getViewCalls()).toBe(0)

    pullReply = snap(12, { a: 0.45 })
    await advance(1)
    expect(getViewCalls()).toBe(1)
    expect(invokeMock.mock.calls.find(c => c[0] === 'get_view')![1]).toEqual({ viewId: VIEW })
    expect(cached().version).toBe(12)
    expect(slotX1(r, 's1')).toBe(0.45)

    await advance(COMMIT_PULL_DELAY_MS * 10)
    expect(getViewCalls()).toBe(1)
  })

  it('기다리는 version 미만의 스냅샷은 타이머를 다시 걸지 않는다 — 응답 뒤 500ms 에 당긴다', async () => {
    vi.useFakeTimers()
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.4))
    act(() => props(r, 'sp-a').onRelease(0.4))
    await respond(0, 'Applied', 12)

    await advance(300)
    apply(snap(11))
    await advance(COMMIT_PULL_DELAY_MS - 300 - 1)
    expect(getViewCalls()).toBe(0)
    pullReply = snap(12, { a: 0.4 })
    await advance(1)
    expect(getViewCalls()).toBe(1)
  })

  it('당김이 날아가는 사이 언마운트되면 그 응답을 캐시에 넣지 않는다', async () => {
    vi.useFakeTimers()
    let finish: (s: ViewSnapshot) => void = () => {}
    pullReply = new Promise<ViewSnapshot>(res => {
      finish = res
    })
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.4))
    act(() => props(r, 'sp-a').onRelease(0.4))
    await respond(0, 'Applied', 12)
    await advance(COMMIT_PULL_DELAY_MS)
    expect(getViewCalls()).toBe(1)

    r.unmount()
    await act(async () => {
      finish(snap(12, { a: 0.4 }))
      await ticks()
    })
    expect(cached().version).toBe(10)
  })

  it('스냅샷이 먼저 오면 당기지 않는다', async () => {
    vi.useFakeTimers()
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.4))
    act(() => props(r, 'sp-a').onRelease(0.4))
    await respond(0, 'Applied', 12)

    await advance(COMMIT_PULL_DELAY_MS / 2)
    apply(snap(12, { a: 0.4 }))
    await advance(COMMIT_PULL_DELAY_MS * 4)
    expect(getViewCalls()).toBe(0)
  })

  it('언마운트하면 타이머를 걷는다', async () => {
    vi.useFakeTimers()
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.4))
    act(() => props(r, 'sp-a').onRelease(0.4))
    await respond(0, 'Applied', 12)
    expect(vi.getTimerCount()).toBe(1)

    r.unmount()
    // 당김 쪽에도 언마운트 가드가 있어 호출 수만으로는 타이머를 걷었는지 안 보인다.
    expect(vi.getTimerCount()).toBe(0)
    await advance(COMMIT_PULL_DELAY_MS * 4)
    expect(getViewCalls()).toBe(0)
  })

  // 응답은 언마운트 여부로 막지 않는다 — 디스패치가 닿아도 해가 없음을 잰다(가드에 기대지 않는다).
  it('확정 도중 언마운트돼도 늦은 응답은 해가 없다', async () => {
    vi.useFakeTimers()
    const err = vi.spyOn(console, 'error').mockImplementation(() => {})
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.4))
    act(() => props(r, 'sp-a').onRelease(0.4))
    r.unmount()

    await respond(0, 'Applied', 12)
    expect(vi.getTimerCount()).toBe(0)
    await advance(COMMIT_PULL_DELAY_MS * 4)
    expect(getViewCalls()).toBe(0)
    expect(cached().version).toBe(10)
    expect(err).not.toHaveBeenCalled()
  })

  it('효과가 끊긴 사이(<Activity hidden>) 온 응답도 받는다 — 다시 보이면 스냅샷으로 미리보기가 풀린다', async () => {
    let mode: 'visible' | 'hidden' = 'visible'
    const wrapper = ({ children }: { children: ReactNode }) => <Activity mode={mode}>{children}</Activity>
    const r = mount({ wrapper })
    act(() => props(r, 'sp-a').onDragStart(0.4))
    act(() => props(r, 'sp-a').onRelease(0.4))

    mode = 'hidden'
    r.rerender({ viewId: VIEW })
    await respond(0, 'Applied', 12)
    apply(snap(12, { a: 0.45 }))

    mode = 'visible'
    r.rerender({ viewId: VIEW })
    expect(slotX1(r, 's1')).toBe(0.45)
    expect(r.result.current.slotRects).toBe(cached().slotRects)
  })
})

describe('useSplitDrag — 뷰 전환', () => {
  beforeEach(() => {
    useViewStore.getState().applyLayoutUpdated(snapV2(5))
  })

  it('viewId 가 바뀌면 미리보기와 진행 중 제스처를 버린다 — 돌아와도 옛 미리보기가 없고, 옛 구분선을 떼도 확정하지 않는다', () => {
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.3))
    const a = props(r, 'sp-a')

    r.rerender({ viewId: VIEW2 })
    expect(r.result.current.slotRects).toBe(cached(VIEW2).slotRects)
    act(() => a.onRelease(0.3))
    expect(ratioCalls()).toEqual([])

    r.rerender({ viewId: VIEW })
    expect(r.result.current.slotRects).toBe(cached().slotRects)
    expect(slotX1(r, 's1')).toBe(0.5)
  })

  it('확정 도중 viewId 가 바뀌면 옛 응답은 새 뷰에 미리보기도 당김도 만들지 않는다', async () => {
    vi.useFakeTimers()
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.4))
    act(() => props(r, 'sp-a').onRelease(0.4))
    r.rerender({ viewId: VIEW2 })

    // 새 뷰의 캐시 version(5) 이 옛 응답의 version(12) 보다 낮다 — 옛 응답이 새어 들면 당김이 걸린다.
    await respond(0, 'Applied', 12)
    expect(r.result.current.slotRects).toBe(cached(VIEW2).slotRects)
    expect(vi.getTimerCount()).toBe(0)
    await advance(COMMIT_PULL_DELAY_MS * 4)
    expect(getViewCalls()).toBe(0)

    // 새 뷰의 드래그는 그대로 돈다.
    act(() => props(r, 'w-sp').onDragStart(0.7))
    expect(slotX1(r, 'w1')).toBe(0.7)
    act(() => props(r, 'w-sp').onRelease(0.7))
    expect(ratioCalls()).toEqual([
      { viewId: VIEW, splitId: 'sp-a', ratio: 0.4 },
      { viewId: VIEW2, splitId: 'w-sp', ratio: 0.7 },
    ])
  })
})

describe('useSplitDrag — 헛 디스패치', () => {
  it('상태를 바꾸지 않는 이벤트는 렌더를 부르지 않는다', () => {
    const r = mount()
    const a = props(r, 'sp-a')
    act(() => a.onDragStart(0.3))
    act(() => props(r, 'sp-c').onDragStart(0.8))

    // 남의 token 이라 무시되는 미리보기.
    let before = renders
    act(() => a.onPreview(0.35))
    expect(renders - before).toBe(0)

    // 끌리는 분할이 남아 있는 스냅샷 — 캐시 변경으로 한 번만 그린다.
    before = renders
    apply(snap(11, { a: 0.5, cAt: 0.75 }))
    expect(renders - before).toBe(1)
  })
})

describe('useSplitDrag — 허용 범위', () => {
  it('보고한 캔버스 × 스냅샷 분할 상자 폭으로 구한다 — 미리보기로 재사상된 상자가 아니다', async () => {
    const r = mount()
    await reportCanvas(200, 100)
    expect(props(r, 'sp-a').range).toEqual(rangeFor(200, 1))
    expect(props(r, 'sp-a').range!.lo).toBeCloseTo(0.15)
    expect(props(r, 'sp-b').range).toEqual(rangeFor(100, 1))
    expect(props(r, 'sp-c').range).toEqual(rangeFor(200, 0.5))

    act(() => props(r, 'sp-a').onDragStart(0.3))
    const c = displaySplit(r, 'sp-c')
    expect(c.x1 - c.x0).not.toBe(0.5)
    expect(r.result.current.splitterProps(c).range).toEqual(rangeFor(200, 0.5))
    expect(rangeFor(200, c.x1 - c.x0)).not.toEqual(rangeFor(200, 0.5))
  })

  it.each([false, true])('캔버스 보고가 바뀌면 %#: 호스트가 다시 그리지 않아도 다시 구한다', async strict => {
    const r = mount({ strict })
    await reportCanvas(200, 100)
    expect(props(r, 'sp-a').range).toEqual(rangeFor(200, 1))
    await reportCanvas(1000, 500)
    expect(props(r, 'sp-a').range).toEqual(rangeFor(1000, 1))
    expect(props(r, 'sp-a').range).toEqual({ lo: 0.1, hi: 0.9 })
  })

  it('보고 전에는 루트 크기를 반올림해 쓰고, 보고가 오면 보고를 쓴다', async () => {
    const r = mount({ root: rootWithRect(199.6, 100.4) })
    expect(props(r, 'sp-a').range).toEqual(rangeFor(200, 1))
    expect(props(r, 'sp-b').range).toEqual(rangeFor(100, 1))

    await reportCanvas(1000, 500)
    expect(props(r, 'sp-a').range).toEqual(rangeFor(1000, 1))
  })

  it('L < 2·MIN_PANE_PX 면 잠긴다', async () => {
    const r = mount()
    await reportCanvas(100, 100)
    // sp-c 의 L = 100 × 0.5 = 50
    expect(props(r, 'sp-c').range).toBeNull()
    expect(props(r, 'sp-a').range).toEqual(rangeFor(100, 1))
  })

  it('보고도 루트도 없으면, 루트가 0 크기면, 스냅샷에 없는 split id 면 잠긴다', async () => {
    const none = mount()
    expect(props(none, 'sp-a').range).toBeNull()
    none.unmount()

    const zero = mount({ root: rootWithRect(0, 0) })
    expect(props(zero, 'sp-a').range).toBeNull()
    expect(props(zero, 'sp-b').range).toBeNull()

    await reportCanvas(1000, 500)
    const ghost = { ...displaySplit(zero, 'sp-a'), split_id: 'ghost' }
    expect(zero.result.current.splitterProps(ghost).range).toBeNull()
    expect(props(zero, 'sp-a').range).not.toBeNull()
  })
})

describe('useSplitDrag — 그릴 사각형의 참조', () => {
  it('미리보기가 없으면 캐시 배열 그대로, 입력이 같으면 다시 그려도 같은 배열이다', () => {
    const r = mount()
    expect(r.result.current.slotRects).toBe(cached().slotRects)
    expect(r.result.current.splitRects).toBe(cached().splitRects)
    r.rerender({ viewId: VIEW })
    expect(r.result.current.slotRects).toBe(cached().slotRects)
  })

  it('미리보기 상자 밖 사각형은 캐시 객체 그대로다', () => {
    const r = mount()
    act(() => props(r, 'sp-c').onDragStart(0.8))
    const shown = r.result.current.slotRects
    const byId = (id: string) => cached().slotRects.find(s => s.slot_id === id)
    expect(shown.find(s => s.slot_id === 's1')).toBe(byId('s1'))
    expect(shown.find(s => s.slot_id === 's4')).toBe(byId('s4'))
    expect(shown.find(s => s.slot_id === 's2')).not.toBe(byId('s2'))

    r.rerender({ viewId: VIEW })
    expect(r.result.current.slotRects).toBe(shown)
  })
})
