// TRD S21-slot-docking §3 「TS — tabStrip」 `measure.ts`: 탭바의 DOM 재기 하나 — 탭바 줄 사각형 · 머리들이 쓸 수 있는 폭 ·
//   캔버스 원점. ★머리는 재지 않는다★ — 탭 경계는 `layout.ts` 폭의 누적(이상적 경계)이다: 끄는 중에는 머리가 비켜남 ·
//   끄는 탭 x 로 옮겨져 그려져 잰 머리가 이상적 자리와 다르다.
// 탭 끌기 영역 사각형(`tab_drag_begin.area`)과 크롬 보고의 `tab_bar` 는 둘 다 여기서 낸 `DragAreaMeasure` 를
//   `frameReport.dragAreaRect` 에 넣어 만든다 — 셸은 둘에서 같은 탭바 구역을 셈한다.

import type { RectCss } from '../../../../src-tauri/bindings/RectCss'
import type { DragAreaMeasure, FrameMeasure } from '../dock/frameReport'
import { tabStripLayout } from './layout'
import { idealBounds, type StripBounds } from './reorderGeometry'

/**
 * 잴 요소. `row` = 탭바 줄 — 그 사각형의 왼쪽이 첫 탭 왼쪽이고 위 · 아래가 탭바 줄이다(왼쪽 패딩 · 테두리 없는 줄).
 * `add` = `+` 단추(없으면 `null` — 그 폭을 빼지 않는다) · `canvas` = 탭 캔버스 영역(크롬 보고에만 쓴다).
 */
export interface StripDom {
  row: Element
  add: Element | null
  canvas?: Element | null
}

/** 한 번 잰 값 — 좌표는 뷰포트 CSS px. */
export interface StripMeasure {
  row: RectCss
  /** 첫 탭 왼쪽 = 끌기 영역 왼쪽(= 탭바 안 순서 이동 좌표의 0). */
  left: number
  /** 머리들이 쓸 수 있는 폭 = 줄 폭 − `+` 폭(`layout.ts` · `dragAreaWidth` 의 `available`). */
  available: number
  /** 캔버스 왼쪽 위 — `canvas` 를 주지 않았으면 `null`. */
  canvas: { left: number; top: number } | null
}

export function measureStrip(dom: StripDom): StripMeasure {
  const r = dom.row.getBoundingClientRect()
  const addW = dom.add === null ? 0 : dom.add.getBoundingClientRect().width
  const c = dom.canvas ? dom.canvas.getBoundingClientRect() : null
  return {
    row: { x0: r.left, y0: r.top, x1: r.right, y1: r.bottom },
    left: r.left,
    available: r.width - addW,
    canvas: c === null ? null : { left: c.left, top: c.top },
  }
}

export interface StripBoundsInput {
  available: number | null
  count: number
  /** 활성으로 그리는 탭의 자리(없으면 −1). */
  active: number
  /** 끄는 탭의 자리 — 레이아웃이 숨겨도 보이는 탭으로 센다(`idealBounds`). */
  dragged?: number
  /** 닫기 모드의 고정 폭(`closingMode.layoutAvailable`) — 폭 셈에만 건다. */
  widthCap?: number | null
}

/** 탭바의 이상적 경계 — `layout.ts` 폭 · 숨김 → `idealBounds`. */
export function stripBounds(input: StripBoundsInput): StripBounds {
  return idealBounds(tabStripLayout(input.available, input.count, input.active, input.widthCap), input.dragged)
}

/** `frameReport.dragAreaRect` 의 재료. */
export function dragAreaOf(m: StripMeasure, bounds: StripBounds): DragAreaMeasure {
  return { left: m.left, top: m.row.y0, bottom: m.row.y1, available: m.available, bounds }
}

/** 크롬 보고(`frameReport.submitFrame` · `scheduleFrameReport`)의 재료 — 캔버스를 재지 않았으면 `null`. */
export function frameMeasureOf(m: StripMeasure, bounds: StripBounds): FrameMeasure | null {
  return m.canvas === null ? null : { canvas: m.canvas, area: dragAreaOf(m, bounds) }
}
