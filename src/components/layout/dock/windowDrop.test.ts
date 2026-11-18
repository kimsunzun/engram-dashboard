// TRD S21-slot-docking §8-2 the receiving side's share of `windowDrop` (receive · draw · drop command · clear) · §8-3 target window order test (fast-check `fc.scheduler()` mixes clearing events — drop · window leave · deadline · mousemove deadline — with late dragover). The outside is a fake port (ADR-0012); canvas · layout is the golden "left-right halves" (canvas 1200 × 800 · origin (0, 28) · area 1 | area 2).

import fc from 'fast-check'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { TabDrop } from '../../../../src-tauri/bindings/TabDrop'
import type { TabDropReply } from '../../../../src-tauri/bindings/TabDropReply'
import { stripZoneArea } from '../tabStrip/measure'
import { idealBounds } from '../tabStrip/reorderGeometry'
import * as bands from './bands'
import { createLocalDrag, type LocalDrag, type StripGeometry } from './localDrag'
import { buildTabDragType, buildTabDragValue } from './tabDragData'
import { goldenCase } from './testing/bandsGolden'
import {
  createWindowDrop,
  GUIDE_IDLE_MS,
  type DropModel,
  type DropView,
  type RootDragEvent,
  type WindowDrop,
  type WindowDropPorts,
} from './windowDrop'

vi.mock('./bands', async importOriginal => {
  const m = await importOriginal<typeof import('./bands')>()
  return { ...m, windowBands: vi.fn(m.windowBands) }
})

const APP = 'com.engram.test'
const DRAG = 'drag-1'
const TYPE = buildTabDragType({ app: APP, drag: DRAG, original: { w: 400, h: 300 }, shape: 'l' })
const VALUE = buildTabDragValue({
  drag: DRAG,
  view: 'X',
  from: 'slot-popup-1',
  initial: 'W',
  size: { w: 800, h: 600 },
  grab: { x: 80, y: 14 },
})

const HALVES = goldenCase('left-right halves')
const SLOT_1 = '00000000-0000-0000-0000-000000000001'
const VIEW_T: DropView = { layout: HALVES.layout, ratioMin: 0.01, ratioMax: 0.99 }
const ROW = { x0: 0, y0: 0, x1: 600, y1: 28 }

// Point (viewport) — canvas origin (0, 28).
const OUTER_LEFT = { x: 5, y: 428 }
const SLOT_TOP = { x: 300, y: 48 }
const CENTRE = { x: 300, y: 428 }
const OUTSIDE = { x: 300, y: 900 }
const STRIP = { x: 250, y: 10 }

/** Another window's tab bar — two heads of 160 (T 0..160 · U 160..320). */
function otherStrip(tabs: string[] = ['T', 'U']): StripGeometry {
  return {
    left: 0,
    bounds: idealBounds({ widths: tabs.map(() => 160), visible: tabs.map(() => true) }),
    available: 600,
    zoneViewport: bands.stripZone(ROW),
    tabs,
  }
}

interface State {
  app: string | null
  shown: string | null
  views: Record<string, DropView>
  model: DropModel
  canvas: { left: number; top: number; width: number; height: number } | null
  geometry: () => StripGeometry | null
}

interface Harness {
  ctl: WindowDrop
  local: LocalDrag
  state: State
  sends: TabDrop[]
  ports: WindowDropPorts
  /** Emits the size-change notice of the observed canvas (changes `state.canvas` first). */
  resize(): void
  /** Number currently observing the canvas. */
  watching(): number
}

function harness(over: Partial<State> = {}, extra: Partial<WindowDropPorts> = {}): Harness {
  const local = createLocalDrag()
  const sends: TabDrop[] = []
  const watchers = new Set<() => void>()
  const state: State = {
    app: APP,
    shown: 'T',
    views: { T: VIEW_T },
    model: { tabs: ['T', 'U'], active: 'T' },
    canvas: { left: 0, top: 28, width: 1200, height: 800 },
    geometry: () => otherStrip(),
    ...over,
  }
  local.registerStripGeometry(() => state.geometry())
  const ports: WindowDropPorts = {
    app: () => state.app,
    local,
    canvasRect: () => state.canvas,
    shownTab: () => state.shown,
    view: id => state.views[id] ?? null,
    model: () => state.model,
    send: drop => {
      sends.push(drop)
      return Promise.resolve<TabDropReply>({ type: 'applied', window: 'main', window_id: 'main', number: 1, reused: false })
    },
    setTimer: (fn, ms) => setTimeout(fn, ms),
    clearTimer: h => clearTimeout(h as ReturnType<typeof setTimeout>),
    watchCanvas: cb => {
      const entry = (): void => cb()
      watchers.add(entry)
      return () => {
        watchers.delete(entry)
      }
    },
    ...extra,
  }
  return {
    ctl: createWindowDrop(ports),
    local,
    state,
    sends,
    ports,
    resize: () => {
      for (const cb of [...watchers]) cb()
    },
    watching: () => watchers.size,
  }
}

interface FakeDragEvent extends RootDragEvent {
  calls: string[]
  dataTransfer: { types: string[]; dropEffect: DataTransfer['dropEffect']; getData(t: string): string }
}

function ev(
  point: { x: number; y: number },
  o: { types?: string[]; related?: EventTarget | null; data?: string } = {},
): FakeDragEvent {
  const calls: string[] = []
  let effect: DataTransfer['dropEffect'] = 'none'
  const types = o.types ?? [TYPE]
  return {
    clientX: point.x,
    clientY: point.y,
    relatedTarget: o.related ?? null,
    calls,
    dataTransfer: {
      types,
      get dropEffect() {
        return effect
      },
      set dropEffect(v) {
        calls.push(`dropEffect=${v}`)
        effect = v
      },
      getData: t => {
        calls.push('getData')
        return t === TYPE ? (o.data ?? VALUE) : ''
      },
    },
    preventDefault: () => void calls.push('preventDefault'),
    stopPropagation: () => void calls.push('stopPropagation'),
  }
}

const enter = (h: Harness, p: { x: number; y: number }) => h.ctl.dragEnter(ev(p))
const over = (h: Harness, p: { x: number; y: number }) => h.ctl.dragOver(ev(p))
const windowBands = vi.mocked(bands.windowBands)

beforeEach(() => {
  vi.useFakeTimers()
  windowBands.mockClear()
})

afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

// ── receive ───────────────────────────────────────────────────────────────────────

describe('receiving — only our drag', () => {
  it('dragenter · dragover of our drag = preventDefault → move → cut', () => {
    const h = harness()
    const a = ev(CENTRE)
    h.ctl.dragEnter(a)
    const b = ev(CENTRE)
    h.ctl.dragOver(b)
    expect(a.calls).toEqual(['preventDefault', 'dropEffect=move', 'stopPropagation'])
    expect(b.calls).toEqual(['preventDefault', 'dropEffect=move', 'stopPropagation'])
  })

  it.each([
    ['file', ['Files']],
    ['URL', ['text/uri-list']],
    ['agent tree row (react-dnd)', ['application/x-tree-row']],
    ['tab drag of another version', [buildTabDragType({ app: 'com.engram.other', drag: DRAG, original: null, shape: null })]],
  ])('a %s drag is not touched — neither blocked, cut nor drawn', (_name, types) => {
    const h = harness()
    const events = [ev(OUTER_LEFT, { types }), ev(OUTER_LEFT, { types }), ev(OUTER_LEFT, { types }), ev(OUTER_LEFT, { types })]
    h.ctl.dragEnter(events[0])
    h.ctl.dragOver(events[1])
    h.ctl.dragLeave(events[2])
    h.ctl.drop(events[3])
    for (const e of events) expect(e.calls).toEqual([])
    expect(h.ctl.getGuide()).toBeNull()
    expect(h.sends).toEqual([])
  })

  it('before the identifier answer even our drag is not ours', () => {
    const h = harness({ app: null })
    const e = ev(OUTER_LEFT)
    h.ctl.dragEnter(e)
    expect(e.calls).toEqual([])
    expect(h.ctl.getGuide()).toBeNull()
  })
})

// ── draw ─────────────────────────────────────────────────────────────────────

describe('drawing — the canvas previews only on a valid band', () => {
  it('outer band = the rectangle to attach + the layout frame (valid outer band region) · no zone lines', () => {
    const h = harness()
    enter(h, OUTER_LEFT)
    const g = h.ctl.getGuide()
    expect(g?.preview).toEqual({ x0: 0, y0: 0, x1: 400, y1: 800 })
    expect(g?.outer.length).toBeGreaterThan(0)
    expect(g?.bands).toEqual([])
    expect(g?.slot).toBeNull()
  })

  it('area band = the rectangle to attach + that area\'s zone lines · no frame', () => {
    const h = harness()
    enter(h, SLOT_TOP)
    const g = h.ctl.getGuide()
    expect(g?.preview).toEqual({ x0: 0, y0: 0, x1: 600, y1: 300 })
    expect(g?.outer).toEqual([])
    expect(g?.slot).toEqual(HALVES.slots.find(s => s.id === SLOT_1)!.zones)
  })

  it('area center = zone lines only (no preview) · outside the canvas = not drawn', () => {
    const h = harness()
    enter(h, CENTRE)
    expect(h.ctl.getGuide()).toEqual({ bands: [], outer: [], preview: null, slot: HALVES.slots[0].zones })
    over(h, OUTSIDE)
    expect(h.ctl.getGuide()).toBeNull()
  })

  it('a blocked band is not painted (R8 — where a zero-area area would form)', () => {
    const c = goldenCase('a dragged tab that would leave a zero-area slot on the sides')
    const type = buildTabDragType({ app: APP, drag: DRAG, original: c.original, shape: c.shape })
    const h = harness({
      views: { T: { layout: c.layout, ratioMin: 0.01, ratioMax: 0.99 } },
      canvas: { left: c.canvas_origin.x, top: c.canvas_origin.y, width: c.canvas.w, height: c.canvas.h },
      geometry: () => null,
    })
    // Blocked = outer left · right · area left · right bands (golden `blocked`) · the top band survives.
    h.ctl.dragEnter(ev({ x: 600, y: 30 }, { types: [type] }))
    expect(h.ctl.getGuide()?.preview).not.toBeNull()
    for (const p of [{ x: 5, y: 428 }, { x: 1150, y: 428 }]) {
      h.ctl.dragOver(ev(p, { types: [type] }))
      expect(h.ctl.getGuide()?.preview ?? null).toBeNull()
    }
  })

  it('another window\'s tab bar = vertical line only — does not touch the source-verdict field (tabs do not move)', () => {
    const h = harness()
    const seen = vi.fn()
    h.local.subscribe(seen)
    enter(h, STRIP)
    expect(h.local.getTargetMarker()).toBe(320)
    expect(h.ctl.getGuide()).toBeNull()
    over(h, { x: 100, y: 10 })
    expect(h.local.getTargetMarker()).toBe(160)
    expect(h.local.getSourceStrip()).toBeNull()
    over(h, CENTRE)
    expect(h.local.getTargetMarker()).toBeNull()
    expect(h.local.getSourceStrip()).toBeNull()
  })

  it('where the dragged tab itself is the visible canvas (a source with only one tab · SelfTarget) = neither band nor zone lines', () => {
    const h = harness({ shown: 'S', views: { S: VIEW_T }, model: { tabs: ['S'], active: 'S' }, geometry: () => null })
    h.local.setOutgoing({ drag: DRAG, window: 'slot-popup-1', view: 'S', grabOffset: 80 })
    enter(h, OUTER_LEFT)
    expect(h.ctl.getGuide()).toBeNull()
    over(h, CENTRE)
    expect(h.ctl.getGuide()).toBeNull()
  })

  it('the band counts only when dragenter · the visible tab · its cache · the canvas changes — each dragover is only point picking', () => {
    const h = harness()
    enter(h, OUTER_LEFT)
    for (let i = 0; i < 20; i++) over(h, i % 2 === 0 ? SLOT_TOP : CENTRE)
    expect(windowBands).toHaveBeenCalledTimes(1)
    h.state.views = { ...h.state.views, T: { ...VIEW_T } }
    over(h, CENTRE)
    expect(windowBands).toHaveBeenCalledTimes(2)
    h.state.views = { ...h.state.views, U: VIEW_T }
    h.state.shown = 'U'
    over(h, CENTRE)
    expect(windowBands).toHaveBeenCalledTimes(3)
    enter(h, CENTRE)
    expect(windowBands).toHaveBeenCalledTimes(3)
    h.state.canvas = { left: 0, top: 28, width: 1000, height: 800 }
    over(h, CENTRE)
    expect(windowBands).toHaveBeenCalledTimes(3)
    enter(h, CENTRE)
    expect(windowBands).toHaveBeenCalledTimes(4)
  })

  it('with the same verdict the guide object is not replaced — subscribers are not woken on every dragover', () => {
    const h = harness()
    const seen = vi.fn()
    h.ctl.subscribe(seen)
    enter(h, OUTER_LEFT)
    const g = h.ctl.getGuide()
    over(h, { x: 6, y: 430 })
    over(h, { x: 7, y: 431 })
    expect(h.ctl.getGuide()).toBe(g)
    expect(seen).toHaveBeenCalledTimes(1)
  })
})

// ── canvas size change during a drag (ADR-0297 — window size is not blocked during a drag) ─────────────────────────────

describe('canvas size change while dragging — the band is recounted with the new size', () => {
  it('when the size changes, without dragover it re-judges with the last point so the preview follows the new size', () => {
    const h = harness()
    const seen = vi.fn()
    h.ctl.subscribe(seen)
    enter(h, OUTER_LEFT)
    const before = h.ctl.getGuide()
    expect(before?.preview).toEqual({ x0: 0, y0: 0, x1: 400, y1: 800 })
    h.state.canvas = { left: 0, top: 28, width: 600, height: 800 }
    h.resize()
    expect(windowBands).toHaveBeenCalledTimes(2)
    expect(windowBands).toHaveBeenLastCalledWith(expect.objectContaining({ canvas: { w: 600, h: 800 } }))
    const after = h.ctl.getGuide()
    expect(after).not.toBe(before)
    expect(after?.preview).not.toEqual(before?.preview)
    expect(after?.preview?.x1).toBeLessThanOrEqual(600)
    expect(seen).toHaveBeenCalledTimes(2)
  })

  it('a notice of the same size (such as the first observation) neither recounts nor redraws', () => {
    const h = harness()
    const seen = vi.fn()
    h.ctl.subscribe(seen)
    enter(h, OUTER_LEFT)
    h.resize()
    h.resize()
    expect(windowBands).toHaveBeenCalledTimes(1)
    expect(seen).toHaveBeenCalledTimes(1)
  })

  it('watching is one per round — it stops when the round closes', () => {
    const h = harness()
    expect(h.watching()).toBe(0)
    enter(h, OUTER_LEFT)
    enter(h, SLOT_TOP)
    over(h, CENTRE)
    expect(h.watching()).toBe(1)
    h.ctl.dragLeave(ev(CENTRE))
    expect(h.watching()).toBe(0)
  })

  it('a size notice arriving late after the round closed does nothing', () => {
    let late: (() => void) | null = null
    const h = harness({}, { watchCanvas: cb => ((late = cb), () => {}) })
    enter(h, OUTER_LEFT)
    h.ctl.dragLeave(ev(CENTRE))
    h.state.canvas = { left: 0, top: 28, width: 600, height: 800 }
    expect(late).not.toBeNull()
    late!()
    expect(windowBands).toHaveBeenCalledTimes(1)
    expect(h.ctl.getGuide()).toBeNull()
  })

  it('it stops even when closed by drop · deadline — a drop arriving with no open round does not open watching', () => {
    const h = harness()
    enter(h, CENTRE)
    h.ctl.drop(ev(CENTRE))
    expect(h.watching()).toBe(0)
    enter(h, CENTRE)
    vi.advanceTimersByTime(GUIDE_IDLE_MS)
    expect(h.watching()).toBe(0)
    h.ctl.drop(ev(SLOT_TOP))
    expect(h.watching()).toBe(0)
  })
})

// ── drop ───────────────────────────────────────────────────────────────────────

describe('drop — one command from the last verdict', () => {
  it('the handler\'s first call is preventDefault · the next is stopPropagation', () => {
    const h = harness()
    enter(h, CENTRE)
    const e = ev(CENTRE)
    h.ctl.drop(e)
    expect(e.calls.slice(0, 2)).toEqual(['preventDefault', 'stopPropagation'])
  })

  it('another window\'s tab bar = TabBar{index} · body is the value of the drag data', () => {
    const h = harness()
    enter(h, STRIP)
    h.ctl.drop(ev(STRIP))
    expect(h.sends).toEqual([
      {
        drag: DRAG,
        view: 'X',
        from: 'slot-popup-1',
        initial: 'W',
        to: { type: 'tab_bar', index: 2 },
      },
    ])
  })

  it('band = Seam{target = visible tab, anchor, side, share}', () => {
    const h = harness({ model: { tabs: ['U', 'T'], active: 'U' } })
    enter(h, SLOT_TOP)
    h.ctl.drop(ev(SLOT_TOP))
    expect(h.sends.map(s => s.to)).toEqual([
      { type: 'seam', target: 'T', anchor: { type: 'slot', id: SLOT_1 }, side: 'Top', share: 0.375 },
    ])
  })

  it('otherwise (area center · outside the canvas) = NewWindow{by: Drop, size, grab = value}', () => {
    for (const p of [CENTRE, OUTSIDE]) {
      const h = harness()
      enter(h, p)
      h.ctl.drop(ev(p))
      expect(h.sends.map(s => s.to)).toEqual([
        { type: 'new_window', by: { type: 'drop' }, size: { w: 800, h: 600 }, grab: { x: 80, y: 14 } },
      ])
    }
  })

  it('drop carries the target of the last verdict (preview) — the visible tab after the last dragover · even if the model changes it does not re-judge', () => {
    const h = harness({ views: { T: VIEW_T, U: VIEW_T } })
    enter(h, SLOT_TOP)
    h.state.shown = 'U'
    h.state.model = { tabs: ['T', 'U', 'V'], active: 'U' }
    h.ctl.drop(ev(SLOT_TOP))
    expect(h.sends.map(s => s.to)).toEqual([
      { type: 'seam', target: 'T', anchor: { type: 'slot', id: SLOT_1 }, side: 'Top', share: 0.375 },
    ])
  })

  it('a drop erases the guide · vertical line · if the value cannot be read nothing is sent', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const h = harness()
    enter(h, STRIP)
    h.ctl.drop(ev(STRIP, { data: '{broken' }))
    expect(h.sends).toEqual([])
    expect(h.local.getTargetMarker()).toBeNull()
    expect(h.ctl.getGuide()).toBeNull()
    expect(warn).toHaveBeenCalled()
  })

  it('a drop arriving with no open round (the deadline closed it) is also not discarded — that point is judged now', () => {
    const h = harness()
    enter(h, CENTRE)
    vi.advanceTimersByTime(GUIDE_IDLE_MS)
    expect(h.ctl.getGuide()).toBeNull()
    h.ctl.drop(ev(SLOT_TOP))
    expect(h.sends.map(s => s.to.type)).toEqual(['seam'])
  })
})

// ── tab bar zone inside the title row ───────────────────────────────────────────────────────

// T-66 · TRD S26-window-chrome §7-9: only where tabs are (first tab ~ `+`) is the tab bar zone. Base = first tab left ~ `+` right end × title row (top margin 6 + tab bar 28 — `tabStrip/measure.ts` `stripZoneArea`). Title row = x 0..1200 · y 0..34 · tabs T · U (head 160) 0..320 · `+` 320..346 · right of it: empty space · empty draggable area · number badge · buttons (1062..1200) · canvas origin (0, 34).
describe('tab bar zone inside the title row — drop on a tab · + = tab bar · drop on its right (empty part · marker · button) = new window', () => {
  const ZONE = { x0: 0, y0: 0, x1: 346, y1: 34 }
  const titleStrip = (): StripGeometry => ({ ...otherStrip(), zoneViewport: bands.stripZone(ZONE) })
  const canvas = { left: 0, top: 34, width: 1200, height: 800 }
  const ADD = { x: 333, y: 20 }
  const PAST_ADD = { x: 346, y: 20 }
  const EMPTY = { x: 620, y: 20 }
  const MARKER = { x: 1040, y: 20 }
  const CLOSE_BUTTON = { x: 1180, y: 20 }
  const TOP_GAP_EMPTY = { x: 900, y: 2 }
  const NEW_WINDOW = { type: 'new_window', by: { type: 'drop' }, size: { w: 800, h: 600 }, grab: { x: 80, y: 14 } }

  it('drop on + = TabBar{end number} · vertical line = after the last tab', () => {
    const h = harness({ canvas, geometry: titleStrip })
    enter(h, ADD)
    expect(h.local.getTargetMarker()).toBe(320)
    h.ctl.drop(ev(ADD))
    expect(h.sends.map(s => s.to)).toEqual([{ type: 'tab_bar', index: 2 }])
  })

  it.each([
    ['+ right end (half-open — outside the zone)', PAST_ADD],
    ['empty place after +', EMPTY],
    ['number marker', MARKER],
    ['window button', CLOSE_BUTTON],
    ['top margin (above an empty place)', TOP_GAP_EMPTY],
  ])('drop on %s = new window · no vertical line', (_name, p) => {
    const h = harness({ canvas, geometry: titleStrip })
    enter(h, p)
    expect(h.local.getTargetMarker()).toBeNull()
    h.ctl.drop(ev(p))
    expect(h.sends.map(s => s.to)).toEqual([NEW_WINDOW])
  })

  it('a drop on a tab in the top margin picks the position by x as now · outside the title row (canvas) is band · otherwise as now', () => {
    const h = harness({ canvas, geometry: titleStrip })
    enter(h, { x: 100, y: 2 })
    h.ctl.drop(ev({ x: 100, y: 2 }))
    const centre = { x: 300, y: 434 }
    const h2 = harness({ canvas, geometry: titleStrip })
    enter(h2, centre)
    h2.ctl.drop(ev(centre))
    expect([...h.sends, ...h2.sends].map(s => s.to)).toEqual([{ type: 'tab_bar', index: 1 }, NEW_WINDOW])
  })

  // T-66 · D-b: the top 0..15 px of the canvas under the right of `+`, now outside the zone, is the outer top band (the space above is no longer the tab bar). The zone is built by the rule (`stripZoneArea`) rather than by hand, and the bands by the real `windowBands`.
  it('top end of the canvas below the right of + (0..15) = outer top band (splits above the whole layout · preview) · under a tab of the same height = tab bar', () => {
    const strip = otherStrip()
    const m = { row: { x0: 0, y0: 6, x1: 1000, y1: 34 }, left: 0, available: 974, addWidth: 26 }
    const zoneViewport = bands.stripZone(stripZoneArea(m, strip.bounds, { top: 0, bottom: 34 }))
    const geometry = (): StripGeometry => ({ ...strip, zoneViewport })
    // Canvas origin (0, 34) — both at canvas y 5.
    const BELOW_EMPTY = { x: 600, y: 39 }
    const BELOW_TAB = { x: 100, y: 39 }

    const h = harness({ canvas, geometry })
    enter(h, BELOW_EMPTY)
    expect(windowBands).toHaveBeenCalled()
    expect(h.local.getTargetMarker()).toBeNull()
    const g = h.ctl.getGuide()
    expect(g?.preview).not.toBeNull()
    expect(g?.outer.length).toBeGreaterThan(0)
    h.ctl.drop(ev(BELOW_EMPTY))

    const h2 = harness({ canvas, geometry })
    enter(h2, BELOW_TAB)
    expect(h2.local.getTargetMarker()).toBe(160)
    expect(h2.ctl.getGuide()).toBeNull()
    h2.ctl.drop(ev(BELOW_TAB))

    expect([...h.sends, ...h2.sends].map(s => s.to)).toEqual([
      { type: 'seam', target: 'T', anchor: { type: 'outer' }, side: 'Top', share: 0.375 },
      { type: 'tab_bar', index: 1 },
    ])
  })

  it('removing a band also sees the same zone', () => {
    const h = harness({ canvas, geometry: titleStrip })
    enter(h, MARKER)
    expect(windowBands.mock.calls[windowBands.mock.calls.length - 1]?.[0].zone).toEqual(bands.zoneInCanvas(bands.stripZone(ZONE), { x: 0, y: 34 }))
  })

  // Source window — tabs A · S · C (head 160 · 0..480) · `+` 480..506 · S is dragged (grabbed point = 80 from the left of the head).
  describe('source window\'s own tab bar', () => {
    const tabs = ['A', 'S', 'C']
    const source = (): Harness => {
      const h = harness({
        canvas,
        shown: 'S',
        views: { A: VIEW_T, S: VIEW_T },
        model: { tabs, active: 'S' },
        geometry: () => ({
          left: 0,
          bounds: idealBounds({ widths: tabs.map(() => 160), visible: tabs.map(() => true) }, 1),
          available: 600,
          zoneViewport: bands.stripZone({ x0: 0, y0: 0, x1: 506, y1: 34 }),
          tabs,
        }),
      })
      h.local.setOutgoing({ drag: DRAG, window: 'main', view: 'S', grabOffset: 80 })
      return h
    }

    it('above + is inside the zone — the dragged tab stays at the end position', () => {
      const h = source()
      enter(h, { x: 490, y: 20 })
      expect(h.local.getSourceStrip()).toEqual({ index: 2, left: 320, x: 490 })
    })

    it.each([
      ['empty place after +', EMPTY],
      ['window button', CLOSE_BUTTON],
    ])('over %s is outside the zone (swaps to the remaining tab) · dropping detaches into a new window', (_name, p) => {
      const h = source()
      enter(h, { x: 490, y: 20 })
      expect(h.local.getSourceStrip()).not.toBeNull()
      over(h, p)
      expect(h.local.getSourceStrip()).toBeNull()
      h.ctl.drop(ev(p))
      expect(h.sends.map(s => s.to)).toEqual([NEW_WINDOW])
    })
  })
})

// ── source window ────────────────────────────────────────────────────────────────────

describe('source window — own tab bar verdict · in-place drop', () => {
  // Tabs A · S · C (160 heads each) · S is dragged · grabbed point = 80 from the left of the head.
  const SOURCE_TABS = ['A', 'S', 'C']
  const sourceStrip = (tabs = SOURCE_TABS): StripGeometry => ({
    left: 0,
    bounds: idealBounds({ widths: tabs.map(() => 160), visible: tabs.map(() => true) }, tabs.indexOf('S')),
    available: 600,
    zoneViewport: bands.stripZone(ROW),
    tabs,
  })

  function source(over: Partial<State> = {}, extra: Partial<WindowDropPorts> = {}): Harness {
    const h = harness(
      {
        shown: 'S',
        views: { A: VIEW_T, S: VIEW_T },
        model: { tabs: SOURCE_TABS, active: 'S' },
        geometry: () => sourceStrip(),
        ...over,
      },
      extra,
    )
    h.local.setOutgoing({ drag: DRAG, window: 'main', view: 'S', grabOffset: 80 })
    return h
  }

  it('on each dragover inside the zone a new object {index, left, x = cursor − tab bar left} · outside the zone = null · no vertical line', () => {
    const h = source()
    enter(h, { x: 244, y: 10 })
    const first = h.local.getSourceStrip()
    expect(first).toMatchObject({ x: 244 })
    over(h, { x: 244, y: 10 })
    const second = h.local.getSourceStrip()
    expect(second).toEqual(first)
    expect(second).not.toBe(first)
    over(h, { x: 404, y: 10 })
    // Cursor − grabbed 80 = 324 but it stops at the last position (three tabs' width 480 − 160 = 320) (`reorderAreaWidth`).
    expect(h.local.getSourceStrip()).toEqual({ index: 2, left: 320, x: 404 })
    expect(h.local.getTargetMarker()).toBeNull()
    over(h, CENTRE)
    expect(h.local.getSourceStrip()).toBeNull()
  })

  it('dragenter does not clear the verdict · dragleave leaving the window = null · dragleave between elements as is', () => {
    const h = source()
    const first = { index: 1, left: 160, x: 244 }
    h.local.setSourceStrip(first)
    enter(h, { x: 244, y: 10 })
    expect(h.local.getSourceStrip()).not.toBeNull()
    h.ctl.dragLeave(ev(CENTRE, { related: {} as EventTarget }))
    expect(h.local.getSourceStrip()).not.toBeNull()
    h.ctl.dragLeave(ev(CENTRE))
    expect(h.local.getSourceStrip()).toBeNull()
  })

  it('if the leaving drag id differs it is not seen as the source (the previous drag mark remains) — another window\'s path', () => {
    const h = source()
    h.local.setOutgoing({ drag: 'older', window: 'main', view: 'S', grabOffset: 80 })
    enter(h, STRIP)
    expect(h.local.getSourceStrip()).toBeNull()
    expect(h.local.getTargetMarker()).not.toBeNull()
  })

  it('if leaving the zone makes the remaining tab visible, in that same dragover the remaining tab\'s band is drawn (R15)', () => {
    // Visible tab = the remaining tab A if the source decision is 「outside the zone」 · otherwise the dragged tab S (`dragSource` swaps immediately to match that timing).
    let local: LocalDrag | null = null
    const h = source({}, { shownTab: () => (local?.getSourceStrip() === null ? 'A' : 'S') })
    local = h.local
    h.local.setSourceStrip({ index: 1, left: 160, x: 244 })
    enter(h, { x: 244, y: 10 })
    expect(h.ctl.getGuide()).toBeNull()
    over(h, OUTER_LEFT)
    expect(h.local.getSourceStrip()).toBeNull()
    expect(h.ctl.getGuide()?.preview).toEqual({ x0: 0, y0: 0, x1: 400, y1: 800 })
  })

  it('while the dragged tab is visible (inside the zone) the verdict over the canvas has no band — it is that tab itself', () => {
    const h = source({ geometry: () => null })
    enter(h, OUTER_LEFT)
    expect(h.ctl.getGuide()).toBeNull()
  })

  it('own tab bar in place = sends nothing and records an in-place mark on that drag', () => {
    const h = source()
    enter(h, { x: 244, y: 10 })
    h.ctl.drop(ev({ x: 244, y: 10 }))
    expect(h.sends).toEqual([])
    expect(h.local.takeLocalDrop(DRAG)).toBe(true)
  })

  it('own tab bar reorder = TabBar{index = the dragged tab\'s position in the order drawn at drop}', () => {
    // Geometry after the tab bar has drawn the order move the source confirmed (S to the end).
    let tabs = SOURCE_TABS
    const h = source({ geometry: () => sourceStrip(tabs) })
    enter(h, { x: 404, y: 10 })
    tabs = ['A', 'C', 'S']
    h.ctl.drop(ev({ x: 404, y: 10 }))
    expect(h.sends.map(s => s.to)).toEqual([{ type: 'tab_bar', index: 2 }])
    expect(h.local.takeLocalDrop(DRAG)).toBe(false)
  })

  it('if after the last dragover the dragged tab went to another window (LLM) it is not seen as in place — sends and does not record an in-place mark', () => {
    // Both when the tab bar still draws the old order (drawing is later than the store) · and after redrawing with the dragged tab removed.
    for (const redrawn of [false, true]) {
      let tabs = SOURCE_TABS
      const h = source({ geometry: () => sourceStrip(tabs) })
      enter(h, { x: 244, y: 10 })
      h.state.model = { tabs: ['A', 'C'], active: 'A' }
      if (redrawn) tabs = ['A', 'C']
      h.ctl.drop(ev({ x: 244, y: 10 }))
      expect(h.sends.map(s => s.to)).toEqual([{ type: 'tab_bar', index: 1 }])
      expect(h.local.takeLocalDrop(DRAG)).toBe(false)
    }
  })
})

// ── clear ─────────────────────────────────────────────────────────────────────

describe('clearing — drop · leaving the window · 300 ms without dragover · 300 ms after mousemove', () => {
  it('dragleave leaving the window (no related target) = erases the guide · vertical line · dragleave between elements does not', () => {
    const h = harness()
    enter(h, STRIP)
    h.ctl.dragLeave(ev(STRIP, { related: {} as EventTarget }))
    expect(h.local.getTargetMarker()).toBe(320)
    const leave = ev(STRIP)
    h.ctl.dragLeave(leave)
    expect(leave.calls).toEqual(['stopPropagation'])
    expect(h.local.getTargetMarker()).toBeNull()
  })

  it('while dragover keeps coming it stays and erases when there is none for 300 ms', () => {
    const h = harness()
    enter(h, OUTER_LEFT)
    for (let i = 0; i < 10; i++) {
      vi.advanceTimersByTime(GUIDE_IDLE_MS - 1)
      over(h, OUTER_LEFT)
    }
    expect(h.ctl.getGuide()).not.toBeNull()
    vi.advanceTimersByTime(GUIDE_IDLE_MS)
    expect(h.ctl.getGuide()).toBeNull()
  })

  it('erases 300 ms after mousemove (no button) — even if dragover comes in between · a move with a button held is not looked at', () => {
    const h = harness()
    enter(h, STRIP)
    h.ctl.mouseMove(1)
    vi.advanceTimersByTime(GUIDE_IDLE_MS - 1)
    over(h, STRIP)
    vi.advanceTimersByTime(GUIDE_IDLE_MS - 1)
    over(h, STRIP)
    expect(h.local.getTargetMarker()).toBe(320)
    h.ctl.mouseMove(0)
    vi.advanceTimersByTime(GUIDE_IDLE_MS - 1)
    over(h, STRIP)
    expect(h.local.getTargetMarker()).toBe(320)
    vi.advanceTimersByTime(1)
    expect(h.local.getTargetMarker()).toBeNull()
  })

  it('a late dragover after erasing does not revive it (receiving as is) · a new dragenter draws again', () => {
    const h = harness()
    enter(h, OUTER_LEFT)
    h.ctl.dragLeave(ev(OUTER_LEFT))
    const late = ev(OUTER_LEFT)
    h.ctl.dragOver(late)
    expect(late.calls).toEqual(['preventDefault', 'dropEffect=move', 'stopPropagation'])
    expect(h.ctl.getGuide()).toBeNull()
    enter(h, OUTER_LEFT)
    expect(h.ctl.getGuide()).not.toBeNull()
  })

  it('release (reset) = closes the open round and reaps the deadline', () => {
    const h = harness()
    enter(h, STRIP)
    h.ctl.reset()
    expect(h.local.getTargetMarker()).toBeNull()
    expect(vi.getTimerCount()).toBe(0)
  })
})

// ── order test ──────────────────────────────────────────────────────────────────

type Clear = 'drop' | 'leave' | 'mouse' | 'idle'

describe('order test (fc.scheduler · target window) — a late dragover after erasing does not revive without a new drag', () => {
  for (const clear of ['drop', 'leave', 'mouse', 'idle'] as Clear[]) {
    for (const reenter of [false, true]) {
      it(`${clear}${reenter ? ' · re-entered' : ''}`, async () => {
        vi.useRealTimers()
        const debug = vi.spyOn(console, 'debug').mockImplementation(() => {})
        let honored = 0
        try {
          await fc.assert(
            fc.asyncProperty(
              fc.scheduler(),
              fc.array(fc.boolean(), { minLength: 2, maxLength: 5 }),
              async (s, kinds) => {
                // Deadline = scheduler job — only ones not cleared fire (that much time passing without dragover = that job runs first).
                const ports: Partial<WindowDropPorts> = {
                  setTimer: fn => {
                    const t = { cancelled: false }
                    void s.schedule(Promise.resolve(), 'timer').then(() => {
                      if (t.cancelled) return
                      fn()
                      step('timer-ran')
                    })
                    return t
                  },
                  clearTimer: t => void ((t as { cancelled: boolean }).cancelled = true),
                }
                const h = harness({}, ports)
                // Open = the last boundary is dragenter · closed = drop · window leave · a fired deadline. Mismatches are collected and asserted at the end — throwing inside a scheduler job vanishes as a discarded promise's rejection.
                let open = false
                let lastOver: 'band' | 'strip' | null = null
                const violations: string[] = []
                const step = (label: string, p?: { x: number; y: number }) => {
                  if (label === 'enter') open = true
                  if (label === 'drop' || label === 'leave' || label === 'timer-ran') open = false
                  if (p !== undefined) lastOver = p === STRIP ? 'strip' : 'band'
                  const g = h.ctl.getGuide()
                  const marker = h.local.getTargetMarker()
                  if (!open) {
                    if (g !== null || marker !== null) violations.push(`${label}: closed but there is a guide · vertical line`)
                  } else if (label === 'enter' || label === 'over') {
                    // The decision of an open round is drawn immediately — so the test does not pass vacuously.
                    honored++
                    const drawn = lastOver === 'band' ? (g?.preview ?? null) !== null : marker !== null
                    if (!drawn) violations.push(`${label}: an open round but nothing drawn`)
                  }
                }
                const at = (strip: boolean) => (strip ? STRIP : OUTER_LEFT)

                h.ctl.dragEnter(ev(OUTER_LEFT))
                step('enter', OUTER_LEFT)
                for (const strip of kinds) {
                  void s.schedule(Promise.resolve(), strip ? 'over:strip' : 'over:band').then(() => {
                    h.ctl.dragOver(ev(at(strip)))
                    step('over', open ? at(strip) : undefined)
                  })
                }
                if (clear === 'drop') {
                  void s.schedule(Promise.resolve(), 'drop').then(() => {
                    h.ctl.drop(ev(OUTER_LEFT))
                    step('drop')
                  })
                } else if (clear === 'leave') {
                  void s.schedule(Promise.resolve(), 'leave').then(() => {
                    h.ctl.dragLeave(ev(OUTER_LEFT))
                    step('leave')
                  })
                } else if (clear === 'mouse') {
                  void s.schedule(Promise.resolve(), 'mousemove').then(() => {
                    h.ctl.mouseMove(0)
                    step('mousemove')
                  })
                }
                if (reenter) {
                  void s.schedule(Promise.resolve(), 'reenter').then(() => {
                    h.ctl.dragEnter(ev(OUTER_LEFT))
                    step('enter', OUTER_LEFT)
                  })
                }
                await s.waitIdle()
                expect(violations, s.report().map(r => r.label).join(' → ')).toEqual([])
                // The deadline does fire eventually — at the end it is always closed.
                expect(h.ctl.getGuide()).toBeNull()
                expect(h.local.getTargetMarker()).toBeNull()
                expect(h.sends.length).toBe(clear === 'drop' ? 1 : 0)
              },
            ),
            { numRuns: 150 },
          )
        } finally {
          debug.mockRestore()
        }
        expect(honored).toBeGreaterThan(0)
      })
    }
  }
})
