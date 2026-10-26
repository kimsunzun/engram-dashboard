// TRD S21-slot-docking §4-1 `measure.ts`: 탭바의 DOM 재기 하나 — 탭바 줄 사각형 · 머리들이 쓸 수 있는 폭 · `+` 폭(+ 그것으로
//   셈한 탭바 구역의 바탕 — `stripZoneArea`).
//   ★머리는 재지 않는다★ — 탭 경계는 `layout.ts` 폭의 누적(이상적 경계)이다: 끄는 중에는 머리가 비켜남 · 끄는 탭 x 로
//   옮겨져 그려져 잰 머리가 이상적 자리와 다르다.

import type { RectCss } from '../dock/overlayTypes'
import { tabStripLayout } from './layout'
import { idealBounds, type StripBounds } from './reorderGeometry'

/**
 * 잴 요소. `row` = 탭바 줄 — 그 사각형의 왼쪽이 첫 탭 왼쪽이고 위 · 아래가 탭바 줄이다(왼쪽 패딩 · 테두리 없는 줄).
 * `add` = `+` 단추(없으면 `null` — 그 폭을 빼지 않는다).
 */
export interface StripDom {
  row: Element
  add: Element | null
}

/** 한 번 잰 값 — 좌표는 뷰포트 CSS px. */
export interface StripMeasure {
  row: RectCss
  /** 첫 탭 왼쪽 = 끌기 영역 왼쪽(= 탭바 안 순서 이동 좌표의 0). */
  left: number
  /** 머리들이 쓸 수 있는 폭 = 줄 폭 − `+` 폭(`layout.ts` · `dragAreaWidth` 의 `available`). */
  available: number
  /** `+` 상자 폭(없으면 0) — 탭바 구역의 오른 끝(`stripZoneArea`). */
  addWidth: number
}

export function measureStrip(dom: StripDom): StripMeasure {
  const r = dom.row.getBoundingClientRect()
  const addW = dom.add === null ? 0 : dom.add.getBoundingClientRect().width
  return {
    row: { x0: r.left, y0: r.top, x1: r.right, y1: r.bottom },
    left: r.left,
    available: r.width - addW,
    addWidth: addW,
  }
}

/**
 * `+` 가 서는 x(탭바 좌표 — 첫 탭 왼쪽 0) = 보이는 마지막 머리의 오른 끝 · 보이는 머리가 없으면 0. 탭바가 `+` 를 그리는 자리와
 * 탭바 구역의 오른 끝(`stripZoneArea`)이 이 값 하나를 쓴다.
 */
export function tabsEnd(B: StripBounds): number {
  return B.lastVisible < 0 ? 0 : B.tabs[B.lastVisible].x1
}

/**
 * 탭바 구역의 바탕 사각형(뷰포트 CSS px — `bands.stripZone` 이 위아래로 넓힌다). 가로 = 첫 탭 왼쪽 ~ `+` 오른 끝(`+` 가 없으면
 * 보이는 마지막 머리 오른 끝 · 보이는 머리가 없으면 `+` 만) — 탭바 줄 오른 끝에서 자른다(그 너머는 탭바가 가로로 잘라 보이지
 * 않는다). 그 오른쪽(빈 곳 · 끌 빈 자리 · 번호 표식 · 창 단추)은 구역이 아니다 — 놓으면 새 창이다(자기 탭이면 떼어 낸다).
 * 세로 = `title`(탭바를 품은 제목 줄 — 위 여백 포함)이 있으면 그 위 ~ 아래 · 없으면 탭바 줄.
 */
// 사용자 결정 2026-10-26(T-66) — 탭이 있는 곳(첫 탭 ~ `+`)만 탭바 구역. 자기 탭도 `+` 를 넘으면 구역 밖(떼어짐) · `+` 오른쪽
//   밑 캔버스 위 끝은 바깥 위 띠다.
// ADR-0303 — 크롬식 제목 표시줄
export function stripZoneArea(
  m: StripMeasure,
  bounds: StripBounds,
  title: { top: number; bottom: number } | null,
): RectCss {
  const x1 = Math.max(m.left, Math.min(m.row.x1, m.left + tabsEnd(bounds) + m.addWidth))
  return title === null
    ? { x0: m.left, y0: m.row.y0, x1, y1: m.row.y1 }
    : { x0: m.left, y0: title.top, x1, y1: title.bottom }
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
