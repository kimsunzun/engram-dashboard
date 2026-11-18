// TRD S21-slot-docking §4-1: TS-specific types used by drag decision · guide.
// ★Do not import `RectCss` from the generated bindings★ — the shell `RectCss` is slated to be detached from the ts export (§5-4), so linking it would break these users when it is withdrawn (that withdrawal's end-condition rg counts the path string, so the path is not written in comments either). The shape equals that binding; the golden `src-tauri/src/layout/bands_golden.json` ties it to the shell version.

import type { PxRect } from './dockedTree'

/**
 * Rectangle — CSS px boundary form (left · top · right · bottom of `getBoundingClientRect`). `x0 <= x1` · `y0 <= y1` · half-open (left · top
 * included). The origin (viewport · that window's canvas) is decided by where it is used.
 */
export interface RectCss {
  x0: number
  y0: number
  x1: number
  y1: number
}

/**
 * Zone lines of one area — what to draw = the `centre` border + four lines joining the same-side corners of `slot` and `centre` (the boundaries of two adjacent area bands).
 * Coordinates = that window's canvas. `slot` = the frame the area bands stand in (smaller than the area frame if the area touches a canvas edge) · `centre` is inside `slot` and its width or height
 * may be 0 (a segment).
 */
export interface SlotZones {
  slot: PxRect
  centre: RectCss
}

/**
 * What to draw in the guide (`DockOverlayView` of `DockOverlay`) — coordinates = guide layer CSS px (the layer covers the target window canvas, so those are canvas coordinates).
 * Rectangles are half-open.
 */
export interface OverlayView {
  /** Effective area band regions of the area under the cursor — not drawn (area band boundaries show through the zone lines of `slot`). */
  bands: RectCss[]
  /**
   * Effective outer band regions — carried only when the cursor is inside an outer band, else an empty list (the outer frame appears only on contact). Non-empty → draw the layout frame (layer border = canvas frame); the region itself is not painted.
   */
  outer: RectCss[]
  /** Rectangle to attach for the chosen band — `null` = no band chosen. */
  preview: RectCss | null
  /** Zone lines of the area under the cursor — none · `null` = not drawn (the cursor is inside an outer band · that area has no effective area band). */
  slot?: SlotZones | null
}
