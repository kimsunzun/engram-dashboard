// TRD S21-slot-docking §8-2 「훅 · 컴포넌트」의 받는 쪽 몫(정하는 곳 `windowDrop` — 받기 · 그리기 · drop 의 명령 · 지우기) ·
//   §8-3 대상 창 순서 시험(fast-check `fc.scheduler()` — 지우는 사건(drop · 창 떠남 · 시한 · mousemove 시한)과 늦은 dragover 를
//   섞는다). 바깥은 가짜 포트(ADR-0012) · 캔버스 · 레이아웃은 골든 「left-right halves」(캔버스 1200 × 800 · 원점 (0, 28) · 칸 1 | 칸 2).

import fc from 'fast-check'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { TabDrop } from '../../../../src-tauri/bindings/TabDrop'
import type { TabDropReply } from '../../../../src-tauri/bindings/TabDropReply'
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

// 점(뷰포트) — 캔버스 원점 (0, 28).
const OUTER_LEFT = { x: 5, y: 428 }
const SLOT_TOP = { x: 300, y: 48 }
const CENTRE = { x: 300, y: 428 }
const OUTSIDE = { x: 300, y: 900 }
const STRIP = { x: 250, y: 10 }

/** 다른 창 탭바 — 머리 160 둘(T 0..160 · U 160..320). */
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
  /** 지켜보는 캔버스의 크기 바뀜 알림을 낸다(`state.canvas` 를 먼저 바꾼다). */
  resize(): void
  /** 지금 캔버스를 지켜보는 수. */
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
      return Promise.resolve<TabDropReply>({ type: 'applied', window: 'main', reused: false })
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

// ── 받기 ───────────────────────────────────────────────────────────────────────

describe('받기 — 우리 끌기만', () => {
  it('우리 끌기의 dragenter · dragover = preventDefault → move → 끊기', () => {
    const h = harness()
    const a = ev(CENTRE)
    h.ctl.dragEnter(a)
    const b = ev(CENTRE)
    h.ctl.dragOver(b)
    expect(a.calls).toEqual(['preventDefault', 'dropEffect=move', 'stopPropagation'])
    expect(b.calls).toEqual(['preventDefault', 'dropEffect=move', 'stopPropagation'])
  })

  it.each([
    ['파일', ['Files']],
    ['URL', ['text/uri-list']],
    ['에이전트 트리 행(react-dnd)', ['application/x-tree-row']],
    ['다른 판의 탭 끌기', [buildTabDragType({ app: 'com.engram.other', drag: DRAG, original: null, shape: null })]],
  ])('%s 끌기는 건드리지 않는다 — 막지도 끊지도 그리지도 않는다', (_name, types) => {
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

  it('identifier 답 전에는 우리 끌기도 우리 것이 아니다', () => {
    const h = harness({ app: null })
    const e = ev(OUTER_LEFT)
    h.ctl.dragEnter(e)
    expect(e.calls).toEqual([])
    expect(h.ctl.getGuide()).toBeNull()
  })
})

// ── 그리기 ─────────────────────────────────────────────────────────────────────

describe('그리기 — 캔버스는 유효한 띠에서만 미리보기', () => {
  it('바깥 띠 = 붙을 사각형 + 레이아웃 틀(유효 바깥 띠 영역) · 구역 선 없음', () => {
    const h = harness()
    enter(h, OUTER_LEFT)
    const g = h.ctl.getGuide()
    expect(g?.preview).toEqual({ x0: 0, y0: 0, x1: 400, y1: 800 })
    expect(g?.outer.length).toBeGreaterThan(0)
    expect(g?.bands).toEqual([])
    expect(g?.slot).toBeNull()
  })

  it('칸 띠 = 붙을 사각형 + 그 칸의 구역 선 · 틀 없음', () => {
    const h = harness()
    enter(h, SLOT_TOP)
    const g = h.ctl.getGuide()
    expect(g?.preview).toEqual({ x0: 0, y0: 0, x1: 600, y1: 300 })
    expect(g?.outer).toEqual([])
    expect(g?.slot).toEqual(HALVES.slots.find(s => s.id === SLOT_1)!.zones)
  })

  it('칸 가운데 = 구역 선만(미리보기 없음) · 캔버스 밖 = 그리지 않음', () => {
    const h = harness()
    enter(h, CENTRE)
    expect(h.ctl.getGuide()).toEqual({ bands: [], outer: [], preview: null, slot: HALVES.slots[0].zones })
    over(h, OUTSIDE)
    expect(h.ctl.getGuide()).toBeNull()
  })

  it('막힌 띠는 칠하지 않는다(R8 — 면적 0 칸이 생길 자리)', () => {
    const c = goldenCase('a dragged tab that would leave a zero-area slot on the sides')
    const type = buildTabDragType({ app: APP, drag: DRAG, original: c.original, shape: c.shape })
    const h = harness({
      views: { T: { layout: c.layout, ratioMin: 0.01, ratioMax: 0.99 } },
      canvas: { left: c.canvas_origin.x, top: c.canvas_origin.y, width: c.canvas.w, height: c.canvas.h },
      geometry: () => null,
    })
    // 막힌 것 = 바깥 왼쪽 · 오른쪽 · 칸 왼쪽 · 오른쪽 띠(골든 `blocked`) · 위 띠는 산다.
    h.ctl.dragEnter(ev({ x: 600, y: 30 }, { types: [type] }))
    expect(h.ctl.getGuide()?.preview).not.toBeNull()
    for (const p of [{ x: 5, y: 428 }, { x: 1150, y: 428 }]) {
      h.ctl.dragOver(ev(p, { types: [type] }))
      expect(h.ctl.getGuide()?.preview ?? null).toBeNull()
    }
  })

  it('다른 창 탭바 = 세로선만 — 출처 판정 칸을 건드리지 않는다(탭은 움직이지 않음)', () => {
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

  it('끄는 탭 자신이 보이는 캔버스(탭 하나뿐인 출처 · SelfTarget) = 띠도 구역 선도 없다', () => {
    const h = harness({ shown: 'S', views: { S: VIEW_T }, model: { tabs: ['S'], active: 'S' }, geometry: () => null })
    h.local.setOutgoing({ drag: DRAG, window: 'slot-popup-1', view: 'S', grabOffset: 80 })
    enter(h, OUTER_LEFT)
    expect(h.ctl.getGuide()).toBeNull()
    over(h, CENTRE)
    expect(h.ctl.getGuide()).toBeNull()
  })

  it('띠는 dragenter · 보이는 탭 · 그 캐시 · 캔버스가 바뀔 때만 센다 — dragover 마다는 점 고르기만', () => {
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

  it('같은 판정이면 안내 객체를 바꾸지 않는다 — 구독자를 dragover 마다 깨우지 않는다', () => {
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

// ── 끄는 동안 캔버스 크기 바뀜(ADR-0297 — 끄는 동안 창 크기를 막지 않는다) ─────────────────────────────

describe('끄는 동안 캔버스 크기 바뀜 — 띠를 새 크기로 다시 센다', () => {
  it('크기가 바뀌면 dragover 없이 마지막 점으로 다시 판정해 미리보기가 새 크기를 따른다', () => {
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

  it('같은 크기 알림(첫 관측 따위)은 다시 세지도 그리지도 않는다', () => {
    const h = harness()
    const seen = vi.fn()
    h.ctl.subscribe(seen)
    enter(h, OUTER_LEFT)
    h.resize()
    h.resize()
    expect(windowBands).toHaveBeenCalledTimes(1)
    expect(seen).toHaveBeenCalledTimes(1)
  })

  it('지켜보기는 회차 하나에 하나 — 회차가 닫히면 멈춘다', () => {
    const h = harness()
    expect(h.watching()).toBe(0)
    enter(h, OUTER_LEFT)
    enter(h, SLOT_TOP)
    over(h, CENTRE)
    expect(h.watching()).toBe(1)
    h.ctl.dragLeave(ev(CENTRE))
    expect(h.watching()).toBe(0)
  })

  it('회차가 닫힌 뒤 늦게 닿은 크기 알림은 아무것도 하지 않는다', () => {
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

  it('drop · 시한으로 닫혀도 멈춘다 — 열린 회차 없이 온 drop 은 지켜보기를 열지 않는다', () => {
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

describe('drop — 마지막 판정으로 명령 하나', () => {
  it('처리기의 첫 호출이 preventDefault · 다음이 stopPropagation', () => {
    const h = harness()
    enter(h, CENTRE)
    const e = ev(CENTRE)
    h.ctl.drop(e)
    expect(e.calls.slice(0, 2)).toEqual(['preventDefault', 'stopPropagation'])
  })

  it('다른 창 탭바 = TabBar{index} · 몸은 끌기 자료의 값', () => {
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

  it('띠 = Seam{target = 보이는 탭, anchor, side, share}', () => {
    const h = harness({ model: { tabs: ['U', 'T'], active: 'U' } })
    enter(h, SLOT_TOP)
    h.ctl.drop(ev(SLOT_TOP))
    expect(h.sends.map(s => s.to)).toEqual([
      { type: 'seam', target: 'T', anchor: { type: 'slot', id: SLOT_1 }, side: 'Top', share: 0.375 },
    ])
  })

  it('그 밖(칸 가운데 · 캔버스 밖) = NewWindow{by: Drop, size, grab = 값}', () => {
    for (const p of [CENTRE, OUTSIDE]) {
      const h = harness()
      enter(h, p)
      h.ctl.drop(ev(p))
      expect(h.sends.map(s => s.to)).toEqual([
        { type: 'new_window', by: { type: 'drop' }, size: { w: 800, h: 600 }, grab: { x: 80, y: 14 } },
      ])
    }
  })

  it('drop 은 마지막 판정(미리보기)의 대상을 싣는다 — 마지막 dragover 뒤 보이는 탭 · 모델이 바뀌어도 다시 판정하지 않는다', () => {
    const h = harness({ views: { T: VIEW_T, U: VIEW_T } })
    enter(h, SLOT_TOP)
    h.state.shown = 'U'
    h.state.model = { tabs: ['T', 'U', 'V'], active: 'U' }
    h.ctl.drop(ev(SLOT_TOP))
    expect(h.sends.map(s => s.to)).toEqual([
      { type: 'seam', target: 'T', anchor: { type: 'slot', id: SLOT_1 }, side: 'Top', share: 0.375 },
    ])
  })

  it('놓기가 안내 · 세로선을 지운다 · 값을 못 읽으면 보내지 않는다', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    const h = harness()
    enter(h, STRIP)
    h.ctl.drop(ev(STRIP, { data: '{broken' }))
    expect(h.sends).toEqual([])
    expect(h.local.getTargetMarker()).toBeNull()
    expect(h.ctl.getGuide()).toBeNull()
    expect(warn).toHaveBeenCalled()
  })

  it('열린 회차 없이 온 drop(시한이 닫았다)도 버리지 않는다 — 그 점을 지금 판정한다', () => {
    const h = harness()
    enter(h, CENTRE)
    vi.advanceTimersByTime(GUIDE_IDLE_MS)
    expect(h.ctl.getGuide()).toBeNull()
    h.ctl.drop(ev(SLOT_TOP))
    expect(h.sends.map(s => s.to.type)).toEqual(['seam'])
  })
})

// ── 출처 창 ────────────────────────────────────────────────────────────────────

describe('출처 창 — 자기 탭바 판정 · 제자리 놓기', () => {
  // 탭 A · S · C(머리 160 씩) · S 를 끈다 · 잡은 점 = 머리 왼쪽에서 80.
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

  it('구역 안 dragover 마다 새 객체 {index, left, x = 커서 − 탭바 왼쪽} · 구역 밖 = null · 세로선은 없다', () => {
    const h = source()
    enter(h, { x: 244, y: 10 })
    const first = h.local.getSourceStrip()
    expect(first).toMatchObject({ x: 244 })
    over(h, { x: 244, y: 10 })
    const second = h.local.getSourceStrip()
    expect(second).toEqual(first)
    expect(second).not.toBe(first)
    over(h, { x: 404, y: 10 })
    expect(h.local.getSourceStrip()).toEqual({ index: 2, left: 324, x: 404 })
    expect(h.local.getTargetMarker()).toBeNull()
    over(h, CENTRE)
    expect(h.local.getSourceStrip()).toBeNull()
  })

  it('dragenter 는 판정을 비우지 않는다 · 창을 떠난 dragleave = null · 요소 사이 dragleave 는 그대로', () => {
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

  it('나간 끌기 id 가 다르면 출처로 보지 않는다(앞 끌기 표시가 남아 있음) — 다른 창 길', () => {
    const h = source()
    h.local.setOutgoing({ drag: 'older', window: 'main', view: 'S', grabOffset: 80 })
    enter(h, STRIP)
    expect(h.local.getSourceStrip()).toBeNull()
    expect(h.local.getTargetMarker()).not.toBeNull()
  })

  it('구역을 떠나 남은 탭이 보이게 되면 그 같은 dragover 에서 남은 탭의 띠를 그린다(R15)', () => {
    // 보이는 탭 = 출처 판정이 「구역 밖」이면 남은 탭 A · 아니면 끄는 탭 S(`dragSource` 가 그 적기에 맞춰 곧바로 바꾼다).
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

  it('끄는 탭이 보이는 동안(구역 안)의 캔버스 위 판정은 띠가 없다 — 그 탭 자신이다', () => {
    const h = source({ geometry: () => null })
    enter(h, OUTER_LEFT)
    expect(h.ctl.getGuide()).toBeNull()
  })

  it('자기 탭바 제자리 = 보내지 않고 그 끌기에 제자리 표시를 적는다', () => {
    const h = source()
    enter(h, { x: 244, y: 10 })
    h.ctl.drop(ev({ x: 244, y: 10 }))
    expect(h.sends).toEqual([])
    expect(h.local.takeLocalDrop(DRAG)).toBe(true)
  })

  it('자기 탭바 순서 바꿈 = TabBar{index = drop 때 그린 순서의 끄는 탭 자리}', () => {
    // 출처가 확정한 순서 이동(S 를 끝으로)을 탭바가 그린 뒤의 기하.
    let tabs = SOURCE_TABS
    const h = source({ geometry: () => sourceStrip(tabs) })
    enter(h, { x: 404, y: 10 })
    tabs = ['A', 'C', 'S']
    h.ctl.drop(ev({ x: 404, y: 10 }))
    expect(h.sends.map(s => s.to)).toEqual([{ type: 'tab_bar', index: 2 }])
    expect(h.local.takeLocalDrop(DRAG)).toBe(false)
  })

  it('마지막 dragover 뒤 끄는 탭이 다른 창으로 갔으면(LLM) 제자리로 보지 않는다 — 보내고 제자리 표시를 적지 않는다', () => {
    // 탭바가 아직 옛 순서를 그린 때(그림이 스토어보다 늦다) · 다시 그려 끄는 탭이 빠진 때 둘 다.
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

// ── 지우기 ─────────────────────────────────────────────────────────────────────

describe('지우기 — drop · 창 떠남 · dragover 없는 300 ms · mousemove 뒤 300 ms', () => {
  it('창을 떠난 dragleave(관련 대상 없음) = 안내 · 세로선을 지운다 · 요소 사이 dragleave 는 아니다', () => {
    const h = harness()
    enter(h, STRIP)
    h.ctl.dragLeave(ev(STRIP, { related: {} as EventTarget }))
    expect(h.local.getTargetMarker()).toBe(320)
    const leave = ev(STRIP)
    h.ctl.dragLeave(leave)
    expect(leave.calls).toEqual(['stopPropagation'])
    expect(h.local.getTargetMarker()).toBeNull()
  })

  it('dragover 가 오는 동안은 남고 300 ms 동안 없으면 지운다', () => {
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

  it('mousemove(버튼 없음) 뒤 300 ms 에 지운다 — 그사이 dragover 가 와도 · 버튼을 쥔 이동은 보지 않는다', () => {
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

  it('지운 뒤 늦은 dragover 는 되살리지 않는다(받기는 그대로) · 새 dragenter 는 다시 그린다', () => {
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

  it('떼기(reset) = 열린 회차를 닫고 시한을 거둔다', () => {
    const h = harness()
    enter(h, STRIP)
    h.ctl.reset()
    expect(h.local.getTargetMarker()).toBeNull()
    expect(vi.getTimerCount()).toBe(0)
  })
})

// ── 순서 시험 ──────────────────────────────────────────────────────────────────

type Clear = 'drop' | 'leave' | 'mouse' | 'idle'

describe('순서 시험(fc.scheduler · 대상 창) — 지운 뒤 늦은 dragover 는 새 끌기 없이 되살리지 않는다', () => {
  for (const clear of ['drop', 'leave', 'mouse', 'idle'] as Clear[]) {
    for (const reenter of [false, true]) {
      it(`${clear}${reenter ? ' · 다시 들어옴' : ''}`, async () => {
        vi.useRealTimers()
        const debug = vi.spyOn(console, 'debug').mockImplementation(() => {})
        let honored = 0
        try {
          await fc.assert(
            fc.asyncProperty(
              fc.scheduler(),
              fc.array(fc.boolean(), { minLength: 2, maxLength: 5 }),
              async (s, kinds) => {
                // 시한 = 스케줄러 일감 — 지우지 않은 것만 울린다(dragover 없이 그만큼 지남 = 그 일감이 먼저 돎).
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
                // 열림 = 마지막 경계가 dragenter · 닫힘 = drop · 창 떠남 · 울린 시한. 어긋남은 모아 두었다가 끝에 단언한다 —
                //   스케줄러 일감 안에서 던지면 버린 약속의 거절로 사라진다.
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
                    if (g !== null || marker !== null) violations.push(`${label}: 닫혔는데 안내 · 세로선이 있다`)
                  } else if (label === 'enter' || label === 'over') {
                    // 열린 회차의 판정은 곧바로 그린다 — 시험이 빈 참으로 통과하지 않게.
                    honored++
                    const drawn = lastOver === 'band' ? (g?.preview ?? null) !== null : marker !== null
                    if (!drawn) violations.push(`${label}: 열린 회차인데 그리지 않았다`)
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
                // 시한은 끝내 울린다 — 끝에는 늘 닫혀 있다.
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
