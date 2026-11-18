// useSplitDrag unit test — gesture token, commit, reconcile, pull timer, allowed range, view switch (ADR-0227, TRD §2f). Runs on the real viewStore cache and the real canvas-report module; the only behavior-imitating mock is invoke (set_split_ratio · get_view · report_window_canvas). The event and window modules are empty shells to unblock viewStore's imports. Divider callbacks are called directly instead of through Splitter.

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

// sp-a: left-right [0,1]×[0,1] (a = s1, b = sp-b)
// sp-b: top-bottom [a,1]×[0,1] at 0.5 (a = sp-c, b = s4)
// sp-c: left-right [a,1]×[0,0.5] (a = s2, b = s3) — with withC=false it is closed and s2 fills its place.
function snap(version: number, p: { a?: number; cAt?: number; withC?: boolean } = {}): ViewSnapshot {
  const a = p.a ?? 0.5
  const withC = p.withC ?? true
  const cAt = p.cAt ?? a + (1 - a) * 0.5
  return {
    view_id: VIEW,
    layout: { type: 'slot', id: 's1', content: { type: 'empty' } },
    focused_slot_id: null,
    foreign_slots: [],
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

// A different view: a single left-right split (w-sp) — w1 | w2.
function snapV2(version: number, at = 0.5): ViewSnapshot {
  return {
    view_id: VIEW2,
    layout: { type: 'slot', id: 'w1', content: { type: 'empty' } },
    focused_slot_id: null,
    foreign_slots: [],
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

/** Same shape as the remap formula — the boundary of ratio `r` inside the split box `[x0,x1]`. */
const boundary = (x0: number, x1: number, r: number): number => x0 + (x1 - x0) * r

const cached = (viewId = VIEW) => useViewStore.getState().layouts[viewId]

/** Host render count — measures whether a no-op dispatch triggers a render (StrictMode counts double). */
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

// set_split_ratio is finished by hand in the test. get_view returns `pullReply` (when it settles, if it is a promise).
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

describe('useSplitDrag — commit and reconciliation', () => {
  it.each([false, true])('dragging draws a preview, releasing commits once, and the preview is held until an Applied(N) snapshot arrives %#', async strict => {
    const r = mount({ strict })
    act(() => props(r, 'sp-a').onDragStart(0.3))
    expect(slotX1(r, 's1')).toBe(0.3)
    act(() => props(r, 'sp-a').onPreview(0.4))
    expect(slotX1(r, 's1')).toBe(0.4)

    act(() => props(r, 'sp-a').onRelease(0.4))
    expect(ratioCalls()).toEqual([{ viewId: VIEW, splitId: 'sp-a', ratio: 0.4 }])

    // Snapshot before the reply (rule 3) — the rest is applied, but the dragged split keeps its preview.
    apply(snap(11))
    expect(slotX1(r, 's1')).toBe(0.4)
    await respond(0, 'Applied', 13)
    expect(slotX1(r, 's1')).toBe(0.4)
    // A snapshot below the awaited version cannot discard the preview either.
    apply(snap(12))
    expect(slotX1(r, 's1')).toBe(0.4)

    apply(snap(13, { a: 0.4 }))
    expect(r.result.current.slotRects).toBe(cached().slotRects)
    expect(r.result.current.splitRects).toBe(cached().splitRects)
    // Once the preview is really discarded, later snapshots (LLM writes etc.) are followed as they are.
    apply(snap(14, { a: 0.6 }))
    expect(slotX1(r, 's1')).toBe(0.6)
    expect(ratioCalls()).toHaveLength(1)
  })

  it.each<SplitRatioOutcome>(['Unchanged', 'TooSmall'])('on a %s response the preview is discarded at once', async outcome => {
    vi.useFakeTimers()
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.3))
    act(() => props(r, 'sp-a').onRelease(0.3))
    expect(slotX1(r, 's1')).toBe(0.3)

    await respond(0, outcome, 99, 0.5)
    expect(r.result.current.slotRects).toBe(cached().slotRects)
    expect(slotX1(r, 's1')).toBe(0.5)
    // The outcome sends no snapshot, so no pull happens either.
    await advance(COMMIT_PULL_DELAY_MS * 4)
    expect(getViewCalls()).toBe(0)
  })

  it('if the command fails it discards the preview and leaves console.error', async () => {
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

  it('even if the Applied(N) snapshot comes before the response it is discarded at the response moment — reads the cache version at response time', async () => {
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.4))
    act(() => props(r, 'sp-a').onRelease(0.4))
    // Before the reply the preview is held. The snapshot value must differ from the preview to show it is held.
    apply(snap(12, { a: 0.45 }))
    expect(slotX1(r, 's1')).toBe(0.4)

    await respond(0, 'Applied', 12)
    expect(slotX1(r, 's1')).toBe(0.45)
    expect(r.result.current.slotRects).toBe(cached().slotRects)
  })

  it('a snapshot during dragging reflects everything else and keeps the preview only for the dragged split (rule 1)', () => {
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

  it('if the split being dragged vanishes from the snapshot the drag is reaped — releasing afterwards does not commit', () => {
    const r = mount()
    const c = props(r, 'sp-c')
    act(() => c.onDragStart(0.8))
    expect(slotX1(r, 's2')).toBe(boundary(0.5, 1, 0.8))

    apply(snap(11, { withC: false }))
    expect(r.result.current.slotRects).toBe(cached().slotRects)
    act(() => c.onRelease(0.8))
    expect(ratioCalls()).toEqual([])

    // Other dividers keep working.
    act(() => props(r, 'sp-a').onDragStart(0.3))
    act(() => props(r, 'sp-a').onRelease(0.3))
    expect(ratioCalls()).toEqual([{ viewId: VIEW, splitId: 'sp-a', ratio: 0.3 }])
  })

  it('double-click sends 0.5 once without a preview', async () => {
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

describe('useSplitDrag — gesture token per divider', () => {
  it('if B\'s new drag takes the preview, releasing A does not commit, and B keeps going', () => {
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

  it('A\'s cancel · late preview does not touch B\'s preview', () => {
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

  it('if a new drag starts while awaiting commit the old response is ignored (rule 5)', async () => {
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.3))
    act(() => props(r, 'sp-a').onRelease(0.3))
    act(() => props(r, 'sp-a').onDragStart(0.6))

    await respond(0, 'Unchanged', 99, 0.5)
    expect(slotX1(r, 's1')).toBe(0.6)
  })

  // The three below are called in one act — ownership must be decided right even with no render between them (a ref synced to render gets it wrong).
  it('if B takes the preview and A releases in the same tick, A does not commit', () => {
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

  it('if A releases first and B starts in the same tick, A commits once', () => {
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

  it('start and release within one tick also commit once', () => {
    const r = mount()
    const p = props(r, 'sp-a')
    act(() => {
      p.onDragStart(0.3)
      p.onRelease(0.3)
    })
    expect(ratioCalls()).toEqual([{ viewId: VIEW, splitId: 'sp-a', ratio: 0.3 }])
  })
})

describe('useSplitDrag — pull after commit (rule 6)', () => {
  it.each([false, true])('if no snapshot comes after Applied %#: get_view is pulled exactly once at 500ms', async strict => {
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

  it('a snapshot below the awaited version does not set the timer again — it pulls 500ms after the response', async () => {
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

  it('if unmounted while the pull is in flight that response is not put into the cache', async () => {
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

  it('if the snapshot comes first it does not pull', async () => {
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

  it('on unmount the timer is cleared', async () => {
    vi.useFakeTimers()
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.4))
    act(() => props(r, 'sp-a').onRelease(0.4))
    await respond(0, 'Applied', 12)
    expect(vi.getTimerCount()).toBe(1)

    r.unmount()
    // The pull side has an unmount guard too, so the call count alone does not show whether the timer was cleared.
    expect(vi.getTimerCount()).toBe(0)
    await advance(COMMIT_PULL_DELAY_MS * 4)
    expect(getViewCalls()).toBe(0)
  })

  // The reply is not blocked by unmount state — measures that a dispatch that lands does no harm (does not lean on the guard).
  it('even if unmounted mid-commit a late response is harmless', async () => {
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

  it('a response arriving while effects are cut (<Activity hidden>) is also received — when visible again the snapshot releases the preview', async () => {
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

describe('useSplitDrag — view switch', () => {
  beforeEach(() => {
    useViewStore.getState().applyLayoutUpdated(snapV2(5))
  })

  it('if viewId changes it discards the preview and in-progress gestures — no old preview on return, and releasing the old divider does not commit', () => {
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

  it('if viewId changes mid-commit the old response makes neither a preview nor a pull on the new view', async () => {
    vi.useFakeTimers()
    const r = mount()
    act(() => props(r, 'sp-a').onDragStart(0.4))
    act(() => props(r, 'sp-a').onRelease(0.4))
    r.rerender({ viewId: VIEW2 })

    // The new view's cache version (5) is lower than the old reply's version (12) — if the old reply leaks in, a pull gets scheduled.
    await respond(0, 'Applied', 12)
    expect(r.result.current.slotRects).toBe(cached(VIEW2).slotRects)
    expect(vi.getTimerCount()).toBe(0)
    await advance(COMMIT_PULL_DELAY_MS * 4)
    expect(getViewCalls()).toBe(0)

    // A drag in the new view keeps working.
    act(() => props(r, 'w-sp').onDragStart(0.7))
    expect(slotX1(r, 'w1')).toBe(0.7)
    act(() => props(r, 'w-sp').onRelease(0.7))
    expect(ratioCalls()).toEqual([
      { viewId: VIEW, splitId: 'sp-a', ratio: 0.4 },
      { viewId: VIEW2, splitId: 'w-sp', ratio: 0.7 },
    ])
  })
})

describe('useSplitDrag — idle dispatch', () => {
  it('an event that does not change state does not trigger a render', () => {
    const r = mount()
    const a = props(r, 'sp-a')
    act(() => a.onDragStart(0.3))
    act(() => props(r, 'sp-c').onDragStart(0.8))

    // A preview that is ignored because the token belongs to someone else.
    let before = renders
    act(() => a.onPreview(0.35))
    expect(renders - before).toBe(0)

    // A snapshot in which the dragged split remains — painted just once, by the cache change.
    before = renders
    apply(snap(11, { a: 0.5, cAt: 0.75 }))
    expect(renders - before).toBe(1)
  })
})

describe('useSplitDrag — allowed range', () => {
  it('it is derived from the reported canvas × the snapshot split box width — not the box remapped by the preview', async () => {
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

  it.each([false, true])('when the canvas report changes %#: it is derived again even if the host does not redraw', async strict => {
    const r = mount({ strict })
    await reportCanvas(200, 100)
    expect(props(r, 'sp-a').range).toEqual(rangeFor(200, 1))
    await reportCanvas(1000, 500)
    expect(props(r, 'sp-a').range).toEqual(rangeFor(1000, 1))
    expect(props(r, 'sp-a').range).toEqual({ lo: 0.1, hi: 0.9 })
  })

  it('before a report it uses the root size rounded, and when a report comes it uses the report', async () => {
    const r = mount({ root: rootWithRect(199.6, 100.4) })
    expect(props(r, 'sp-a').range).toEqual(rangeFor(200, 1))
    expect(props(r, 'sp-b').range).toEqual(rangeFor(100, 1))

    await reportCanvas(1000, 500)
    expect(props(r, 'sp-a').range).toEqual(rangeFor(1000, 1))
  })

  it('it is locked if L < 2·MIN_PANE_PX', async () => {
    const r = mount()
    await reportCanvas(100, 100)
    // L of sp-c = 100 × 0.5 = 50
    expect(props(r, 'sp-c').range).toBeNull()
    expect(props(r, 'sp-a').range).toEqual(rangeFor(100, 1))
  })

  it('it is locked if there is neither report nor root, if the root has 0 size, or if the split id is not in the snapshot', async () => {
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

describe('useSplitDrag — reference of the rectangles to draw', () => {
  it('with no preview it is the cache array as is, and with the same input redrawing gives the same array', () => {
    const r = mount()
    expect(r.result.current.slotRects).toBe(cached().slotRects)
    expect(r.result.current.splitRects).toBe(cached().splitRects)
    r.rerender({ viewId: VIEW })
    expect(r.result.current.slotRects).toBe(cached().slotRects)
  })

  it('rectangles outside the preview box are the cache objects as is', () => {
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
