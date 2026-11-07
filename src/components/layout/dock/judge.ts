// TRD S21-slot-docking §4-3: the decision of every root dragover of our windows — a point + that window's context → tab bar position · band · other. Pure.
// Order of precedence (R6a) = tab bar zone > outer band > area band > other. Bands look only at the effective ones `bands.ts` pre-computed at dragenter — blocked bands are
//   neither painted nor chosen (R8). The rules · numbers of the tab bar zone · bands are a set with the shell `layout/bands.rs` and the golden ties them (`judge.test.ts`).

import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import {
  draggedLeft,
  dropIndex,
  dropMarkerX,
  insertionIndex,
  reorderAreaWidth,
  type StripBounds,
} from '../tabStrip/reorderGeometry'
import { choose, contains, zonesAt, type WindowBands } from './bands'
import type { RectCss } from './overlayTypes'

/**
 * That window's tab bar — tab bar coordinate 0 = left of the tab drag area (`left` · viewport x). `source` = the source window's own tab bar containing the dragged tab (Chrome order move —
 * R23) · `other` = another window's tab bar (vertical line — W13).
 */
export type JudgeStrip =
  | {
      role: 'source'
      left: number
      /** Ideal bounds of the current order (including the dragged tab — `idealBounds(layout, dragged)`). */
      bounds: StripBounds
      /** Index of the dragged tab in `bounds.tabs` — if outside the list (−1 · length or more = the dragged tab left this tab bar), it is counted like another window's tab bar. */
      dragged: number
      /** `grabOffset` — the value computed once at dragstart. */
      grabOffset: number
      /** `available` of `reorderAreaWidth` — the measured value (`null` = could not be measured). */
      available: number | null
    }
  | { role: 'other'; left: number; bounds: StripBounds }

/** Window context of the decision — rebuilt only when dragenter · the visible tab · the snapshot changes. */
export interface JudgeWindow {
  /** Canvas top-left (viewport CSS px). */
  canvasOrigin: PointCss
  /**
   * Tab bar — `zone` = tab bar zone (value of `bands.canvasZone` · canvas coordinates — it must be the same value as `BandsInput.zone` of the band computation to mesh with the band regions)
   * · `strip` = the tab bar that computes that position. `null` = tab bar unknown.
   */
  tabBar: { zone: RectCss; strip: JudgeStrip } | null
  /** Bands of the visible tab — `null` = the canvas where the dragged tab itself is visible (a single-tab source window · `SelfTarget` — no bands). */
  bands: WindowBands | null
}

export type Judgement =
  /**
   * Tab bar zone. `index` = for the source, Chrome `insertionIndex` (position in the list without the dragged tab) · for another window, `dropIndex` (insertion position in that window's list).
   * `left` = left of the source's dragged head (tab bar coordinates · `draggedLeft`) · `marker` = x of the vertical line in another window (tab bar coordinates · `dropMarkerX`).
   */
  | { type: 'strip'; index: number; left: number | null; marker: number | null }
  /** Band — `band` = index in `bands.bands` (preview = its `landing`) · `zones` = index in `bands.frames` to draw zone lines for (`null` for an outer band). */
  | { type: 'band'; band: number; zones: number | null }
  /** Other (area center · our UI outside the canvas) — a drop gives a new window. For an area center, that area's zone lines (`zones`) are drawn. */
  | { type: 'elsewhere'; zones: number | null }

/** Decision for point `point` (that window's viewport CSS px). */
export function judge(w: JudgeWindow, point: PointCss): Judgement {
  const p = { x: point.x - w.canvasOrigin.x, y: point.y - w.canvasOrigin.y }
  if (w.tabBar !== null && contains(w.tabBar.zone, p)) return stripAt(w.tabBar.strip, point.x - w.tabBar.strip.left)
  if (w.bands === null) return { type: 'elsewhere', zones: null }
  const band = choose(w.bands.bands, p)
  const zones = zonesAt(w.bands, p, band)
  return band === null ? { type: 'elsewhere', zones } : { type: 'band', band, zones }
}

function stripAt(s: JudgeStrip, x: number): Judgement {
  const head = s.role === 'source' ? s.bounds.tabs[s.dragged] : undefined
  if (s.role === 'other' || head === undefined) {
    const index = dropIndex(s.bounds, x, null)
    return { type: 'strip', index, left: null, marker: dropMarkerX(s.bounds, index, null) }
  }
  const left = draggedLeft(x, s.grabOffset, head.x1 - head.x0, reorderAreaWidth(s.available, s.bounds))
  return { type: 'strip', index: insertionIndex(s.bounds, s.dragged, left), left, marker: null }
}
