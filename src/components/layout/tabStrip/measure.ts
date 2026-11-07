// TRD S21-slot-docking §4-1 `measure.ts`: the tab bar's single DOM measuring — tab bar row rectangle · width the heads can use · `+` width (+ the base of the tab bar zone
//   computed from it — `stripZoneArea`).
//   ★Heads are not measured★ — tab bounds are the accumulation of `layout.ts` widths (ideal bounds): during a drag heads are drawn moved by stepping aside · the dragged tab x,
//   so the measured head differs from the ideal position.

import type { RectCss } from '../dock/overlayTypes'
import { tabStripLayout } from './layout'
import { idealBounds, type StripBounds } from './reorderGeometry'

/**
 * Elements to measure. `row` = the tab bar row — the left of its rectangle is the left of the first tab and top · bottom are the tab bar row (a row without left padding · border).
 * `add` = the `+` button (`null` if absent — that width is not subtracted).
 */
export interface StripDom {
  row: Element
  add: Element | null
}

/** Values measured once — coordinates are viewport CSS px. */
export interface StripMeasure {
  row: RectCss
  /** Left of the first tab = left of the drag area (= 0 of the order-move coordinates inside the tab bar). */
  left: number
  /** Width the heads can use = row width − `+` width (`layout.ts` · `available` of `dragAreaWidth`). */
  available: number
  /** `+` box width (0 if absent) — the right end of the tab bar zone (`stripZoneArea`). */
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
 * The x where `+` stands (tab bar coordinates — left of the first tab 0) = the right end of the last visible head · 0 if there is no visible head. The position where the tab bar draws `+` and
 * the right end of the tab bar zone (`stripZoneArea`) use this one value.
 */
export function tabsEnd(B: StripBounds): number {
  return B.lastVisible < 0 ? 0 : B.tabs[B.lastVisible].x1
}

/**
 * Base rectangle of the tab bar zone (viewport CSS px — `bands.stripZone` widens it above and below). Horizontal = left of the first tab ~ right end of `+` (if there is no `+`,
 * the right end of the last visible head · if there is no visible head, `+` only) — cut at the right end of the tab bar row (beyond it the tab bar clips horizontally so it is not
 * visible). To its right (empty space · empty draggable area · number badge · window buttons) is not the zone — dropping gives a new window (its own tab is detached).
 * Vertical = from the top to the bottom of `title` (the title row holding the tab bar — including the top margin) if present · otherwise the tab bar row.
 */
// User decision 2026-10-26 (T-66) — only where tabs are (first tab ~ `+`) is the tab bar zone. Its own tab too, past `+`, is outside the zone (detached) · the top end of the canvas
//   under the right of `+` is the outer top band.
// ADR-0303 — Chrome-style title bar
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
  /** Position of the tab drawn as active (−1 if none). */
  active: number
  /** Position of the dragged tab — counted as a visible tab even when the layout hides it (`idealBounds`). */
  dragged?: number
  /** Fixed width of closing mode (`closingMode.layoutAvailable`) — applied only to the width computation. */
  widthCap?: number | null
}

/** Ideal bounds of the tab bar — `layout.ts` width · hiding → `idealBounds`. */
export function stripBounds(input: StripBoundsInput): StripBounds {
  return idealBounds(tabStripLayout(input.available, input.count, input.active, input.widthCap), input.dragged)
}
