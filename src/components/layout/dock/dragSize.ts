// TRD S21-slot-docking §4-2 "size": sizes carried by the source dragstart — new window inner size (R22 · Chrome `CalculateDraggedWindowSize`) · the dragged tab's original size (R7 — material for the target's attach share) · the point `grab` where the cursor will be in the new window. Unit = logical (DIP) px = CSS px. Pure.
// ★Mirror of the shell `src-tauri/src/layout/drag_size.rs`★ — the LLM command default (T-65) · the drop validation use the shell version; both read the `drag_size` table of the golden `bands_golden.json` (`dragSize.test.ts`). Margin numbers come from the generated `DOCK_CONSTANTS`.

import type { Insets } from '../../../../src-tauri/bindings/Insets'
import { DOCK_CONSTANTS } from '../../../../src-tauri/bindings/DockConstants'
import type { PointCss } from '../../../../src-tauri/bindings/PointCss'
import type { SizeCss } from '../../../../src-tauri/bindings/SizeCss'
import { tabStripLayout } from '../tabStrip/layout'

const { MAXIMIZED_INSET_DIP } = DOCK_CONSTANTS

/** Facts of the source window at drag start (shell `SourceWindow`). */
export interface SourceWindow {
  /** Restored (normal) inner size (`window:frame.restored`) — `null` = unknown (treated as the current size). */
  restored: SizeCss | null
  maximized: boolean
  /** Window frame = outer − inner (`window:frame.insets`). */
  insets: Insets
  /** Current inner size (viewport) — if maximized, the maximized inner size. */
  viewport: SizeCss
  /** Work area of that window's monitor (`screen.availWidth` · `availHeight` — the limit of the outer size). */
  work: SizeCss
  /**
   * Chrome of the new window (it comes up in normal state) = viewport − tab canvas + the title row height difference (normal − the source's current; non-zero if the source is maximized — TRD S26-window-chrome §7-6). The caller adds that difference (`rowGrowth` of `useTabDragSource`).
   */
  chrome: SizeCss
  /** A single-tab popout — that window is moved without a new window (Q34). */
  single: boolean
}

export interface DragSizes {
  /** New window inner size — `null` = single-tab popout (that window is moved at its size as is). */
  size: SizeCss | null
  /** The dragged tab's original size = canvas size (negative becomes 0). */
  original: SizeCss
}

/**
 * Shell `dragged_size` — from `restored`: ① a side larger than the work area `work` becomes `max(0, work − 2 × MAXIMIZED_INSET_DIP)` ② if the source is maximized (`maximized` = that size), each side is at least half of it. ★② comes last so it wins (C5)★ — do not reorder. The three sizes must be on the same side (all outer · all inner).
 */
// ADR-0295
export function draggedSize(restored: SizeCss, maximized: SizeCss | null, work: SizeCss): SizeCss {
  let { w, h } = restored
  if (w > work.w) w = fmax(work.w - 2 * MAXIMIZED_INSET_DIP, 0)
  if (h > work.h) h = fmax(work.h - 2 * MAXIMIZED_INSET_DIP, 0)
  if (maximized !== null) {
    w = fmax(w, maximized.w / 2)
    h = fmax(h, maximized.h / 2)
  }
  return { w, h }
}

/**
 * Shell `drag_sizes` — with two or more tabs: `size` = `draggedSize` converted to an inner size (restore size · else current size / work area − frame / maximized: its inner size), `original` = `size` − chrome. Single-tab popout: no `size`; `original` = (restore size if maximized, else current size) − chrome.
 */
export function dragSizes(s: SourceWindow): DragSizes {
  const canvas = (v: SizeCss): SizeCss => ({ w: fmax(v.w - s.chrome.w, 0), h: fmax(v.h - s.chrome.h, 0) })
  if (s.single) {
    return { size: null, original: canvas(s.maximized && s.restored !== null ? s.restored : s.viewport) }
  }
  const work = { w: s.work.w - (s.insets.l + s.insets.r), h: s.work.h - (s.insets.t + s.insets.b) }
  const size = draggedSize(s.restored ?? s.viewport, s.maximized ? s.viewport : null, work)
  return { size, original: canvas(size) }
}

/** Input of `dragGrab` — coordinates are source viewport CSS px. */
export interface GrabInput {
  /** Pressed point — if the vertical position of the new window's tab bar differs from the source (a maximized source — the new window is in normal state), the caller shifts it by that amount and passes it. */
  pressed: PointCss
  /** Horizontal range of the dragged head at press time. */
  head: { x0: number; x1: number }
  /** Left of the tab drag area (left of the first tab). */
  stripLeft: number
  /**
   * Width the heads can use in the new window's tab bar (`available` of `layout.ts`) — `null` = unknown (standard width). Two or more tabs: from the new window width; maximized single-tab popout: from the restored window width (TRD §3-5).
   */
  headRoom: number | null
  single: boolean
  maximized: boolean
}

/**
 * The point where the cursor will be in the new window viewport (`DropTo.new_window.grab` · Chrome `GetTabOffsetForDetachedWindow`) = (tab bar left + r × W1, pressed y) — r = the width ratio of the pressed point inside the head, W1 = the single-tab head width in the new window's tab bar. A non-maximized single-tab popout is the pressed point as is (already the first tab, W1 = current head width — the shell `Reuse` moves the window by the drag amount · Q34).
 */
// The new window's tab bar left · vertical position is taken as the source tab bar's — the new window is a popout, so with main as source that chrome may differ ([untested]; if off the new window comes up shifted from the cursor by that amount).
export function dragGrab(g: GrabInput): PointCss {
  if (g.single && !g.maximized) return { x: g.pressed.x, y: g.pressed.y }
  const width = g.head.x1 - g.head.x0
  const r = width > 0 ? (g.pressed.x - g.head.x0) / width : 0
  const w1 = tabStripLayout(g.headRoom, 1, 0).widths[0]
  return { x: g.stripLeft + r * w1, y: g.pressed.y }
}

/** Shell `f64::max` — returns the non-NaN side (`Math.max` gives NaN). */
function fmax(a: number, b: number): number {
  if (Number.isNaN(a)) return b
  if (Number.isNaN(b)) return a
  return Math.max(a, b)
}
