// TRD S21-slot-docking §8-2 `dragSize`: 셸 `layout/drag_size.rs` 와 **같은 골든 파일**의 새 창 크기 표 + Chrome C5 차례 · 복원 크기
//   없음 · 탭 하나 팝아웃 · `grab`(r × W1 · 탭 하나 → 누른 점).

import { describe, expect, it } from 'vitest'

import { DOCK_CONSTANTS } from '../../../../src-tauri/bindings/DockConstants'
import { TAB_STANDARD_W, TAB_MIN_ACTIVE_W } from '../tabStrip/layout'
import { dragGrab, draggedSize, dragSizes, type GrabInput, type SourceWindow } from './dragSize'
import { bandsGolden } from './testing/bandsGolden'

describe('골든 — 셸 drag_size.rs 와 한 파일', () => {
  it('크기 줄이 여덟 이상이다', () => {
    expect(bandsGolden.drag_size.length).toBeGreaterThanOrEqual(8)
  })

  for (const c of bandsGolden.drag_size) {
    it(`골든: ${c.name}`, () => {
      const { name: _, size, original, ...input } = c
      expect(dragSizes(input)).toEqual({ size, original })
    })
  }
})

const FRAME = { t: 31, r: 8, b: 8, l: 8 }
const BASE: SourceWindow = {
  restored: { w: 1000, h: 700 },
  maximized: false,
  insets: FRAME,
  viewport: { w: 1000, h: 700 },
  work: { w: 1920, h: 1032 },
  chrome: { w: 0, h: 36 },
  single: false,
}

describe('새 창 크기 (R22 · Chrome C5)', () => {
  it('작업 영역을 넘는 변은 작업 영역 − 양쪽 여백 → 그 뒤 최대화 절반 하한이 이긴다', () => {
    const inset = 2 * DOCK_CONSTANTS.MAXIMIZED_INSET_DIP
    expect(draggedSize({ w: 3000, h: 3000 }, null, { w: 800, h: 600 })).toEqual({ w: 800 - inset, h: 600 - inset })
    expect(draggedSize({ w: 3000, h: 3000 }, { w: 2560, h: 1369 }, { w: 800, h: 600 })).toEqual({ w: 1280, h: 684.5 })
    expect(draggedSize({ w: 5, h: 5 }, null, { w: 4, h: 4 })).toEqual({ w: 0, h: 0 })
  })

  it('복원 크기를 모르면 지금 크기에서 · 원래 크기 = 새 창 − 크롬', () => {
    expect(dragSizes({ ...BASE, restored: null, viewport: { w: 900, h: 500 } })).toEqual({
      size: { w: 900, h: 500 },
      original: { w: 900, h: 464 },
    })
  })

  it('탭 하나뿐인 팝아웃 = 크기 없음(그 창을 옮김) · 원래 크기 = (최대화면 복원 · 아니면 지금) − 크롬', () => {
    expect(dragSizes({ ...BASE, single: true, viewport: { w: 640, h: 480 } })).toEqual({
      size: null,
      original: { w: 640, h: 444 },
    })
    expect(dragSizes({ ...BASE, single: true, maximized: true, viewport: { w: 1920, h: 1001 } })).toEqual({
      size: null,
      original: { w: 1000, h: 664 },
    })
  })

  it('틀은 window:frame 의 inset 이다 — 작업 영역에서 틀을 빼고 견준다', () => {
    const r = dragSizes({ ...BASE, restored: { w: 1910, h: 1000 } })
    expect(r.size).toEqual({ w: 1920 - 16 - 2 * DOCK_CONSTANTS.MAXIMIZED_INSET_DIP, h: 1032 - 39 - 2 * DOCK_CONSTANTS.MAXIMIZED_INSET_DIP })
  })
})

describe('grab — 새 창에서 커서가 올 점', () => {
  // 끄는 머리 100..200 의 가운데를 누름(r = 1/2) · 끌기 영역 왼쪽 4.
  const G: GrabInput = {
    pressed: { x: 150, y: 14 },
    head: { x0: 100, x1: 200 },
    stripLeft: 4,
    headRoom: 1000,
    single: false,
    maximized: false,
  }

  it('= (탭바 왼쪽 + r × W1, 누른 y) — W1 = 새 창 탭 하나 머리 폭', () => {
    expect(dragGrab(G)).toEqual({ x: 4 + TAB_STANDARD_W / 2, y: 14 })
    expect(dragGrab({ ...G, headRoom: null })).toEqual({ x: 4 + TAB_STANDARD_W / 2, y: 14 })
    expect(dragGrab({ ...G, headRoom: TAB_MIN_ACTIVE_W })).toEqual({ x: 4 + TAB_MIN_ACTIVE_W / 2, y: 14 })
  })

  it('탭 하나뿐인 팝아웃 = 누른 점 · 최대화돼 있으면 복원 창 폭의 W1 로 센다', () => {
    expect(dragGrab({ ...G, single: true })).toEqual({ x: 150, y: 14 })
    expect(dragGrab({ ...G, single: true, maximized: true })).toEqual({ x: 4 + TAB_STANDARD_W / 2, y: 14 })
  })
})
