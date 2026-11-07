// TRD S21-slot-docking §4-1 · §4-2 · edition 6.3 §5-1 (C1 · C2 · C3): pure geometry of the in-tab-bar order move — Chromium desktop tab bar drag
//   (`DraggingTabsSession::MoveAttachedImpl` · `GetInsertionIndexForDraggedBounds` → `CalculateInsertionIndex` of `TabStrip`) is ported
//   as is to our case of one tab · no pinned tabs · no groups · no splits · overlap 0. Do not invent rules (Q23).
//   ★If a TRD sentence and the Chromium source differ, follow the source★ (main decision 2026-10-18 — 「Chrome 소스 그대로」).
// Chromium source is under a BSD-style license (「Copyright The Chromium Authors」 · its LICENSE) — what was ported is the algorithm, not the code
//   sentences. Line numbers = chromium/src main (fetched 2026-10-18): `tab_strip.cc` · `tab_container_impl.cc` = `chrome/browser/ui/views/tabs/` ·
//   `dragging_tabs_session.cc` = `chrome/browser/ui/views/tabs/dragging/` · `tween.cc` · `animation.cc` = `ui/gfx/animation/` ·
//   `bounds_animator.h` = `ui/views/animation/`.
// Coordinates = CSS px · the left of the drag area (= left of the first tab) is 0. Rounding is only in the grab offset · threshold, and the rest is floating point as is (TRD S21-slot-docking edition 6.3 §5-1).

import { TAB_STANDARD_W, type TabStripLayout } from './layout'

/** Reference value of the order threshold — this much for a standard width tab. Chrome `kHorizontalMoveThreshold` (dragging_tabs_session.cc#89). */
export const REORDER_THRESHOLD_PX = 16

/**
 * Time (ms) for a tab other than the dragged tab to go to its new position after the order changes. `RichAnimationDuration(200 ms)` of Chrome `TabContainerImpl::AnimateViewTo`
 * — 0 when rich animations are off (tab_container_impl.cc#1153-1160 · animation.cc#114-116).
 */
export const SLIDE_MS = 200

/**
 * Step-aside curve = Chrome `BoundsAnimator` default tween `EASE_OUT` `1 − (1 − t)²` (bounds_animator.h#205 · tween.cc#72-73) ported to CSS
 * — the exact form that raises the quadratic curve to cubic (control points 1/3 · 2/3 cut at the fourth decimal place).
 */
export const EASE_OUT_CSS = 'cubic-bezier(0.3333, 0.6667, 0.6667, 1)'

/** Ideal bounds `[x0, x1)` of one tab — accumulation of computed widths (Chrome `GetIdealBounds` style · hidden tabs also stand at their own width beyond the visible end). */
export interface TabBounds {
  x0: number
  x1: number
}

/** Ideal bounds of one tab bar — made by `idealBounds`. */
export interface StripBounds {
  /** All tabs (current model order · including the dragged tab). */
  tabs: TabBounds[]
  /** Tabs that are drawn — layout visibility + the dragged tab. */
  visible: boolean[]
  /** Position of the last visible tab (−1 if none) — the insertion position is clamped inside this (tab_strip.cc#589-600). */
  lastVisible: number
}

/**
 * `layout.ts` result → ideal bounds. The bounds are the **accumulation of all tabs' computed widths** — hidden tabs also keep their width and stand beyond the visible end
 * (exactly Chrome's ideal bounds). ★Do not stack hidden tabs on the visible end with width 0★ — then the in-place candidate of the dragged tab after them
 * has the same x as the candidate before a hidden tab, the earlier wins the tie, and the dragged tab is moved one position forward even when it stays still.
 * `dragged` = the position of the dragged tab — counted as a visible tab even when the layout hides it: Chrome always shows the dragged tab unless the drag area clips it
 * (tab_container_impl.cc#1688-1697).
 */
export function idealBounds(layout: TabStripLayout, dragged?: number): StripBounds {
  let x = 0
  const tabs = layout.widths.map(w => {
    const b = { x0: x, x1: x + w }
    x += w
    return b
  })
  const visible = layout.widths.map((_, i) => layout.visible[i] || i === dragged)
  return { tabs, visible, lastVisible: visible.lastIndexOf(true) }
}

/**
 * Tab drag area width `A` — the dragged tab can go anywhere within this width, not only the range the tabs occupy. `available` = the **measured value** of
 * the 「머리들이 쓸 수 있는 폭」 that `layout.ts` receives — ★do not put in the fixed width of closing mode★ (Chrome comment 「needs to ignore tab closing mode」).
 * With the measured value it is as is — in Chrome's `max(쓸 수 있는 폭, 탭바 폭)`, the tab bar width has the stretch cap of the preferred width, so it does not exceed the usable width
 * (horizontal_tab_strip_region_view.cc#205-209). `null` · a non-finite number = could not be measured → sum of visible tab widths.
 */
// Ported from Chromium (BSD) `TabStrip::GetTabDragAreaWidth` (tab_strip.cc#530-544).
export function dragAreaWidth(available: number | null, B: StripBounds): number {
  if (available !== null && Number.isFinite(available)) return available
  return B.tabs.reduce((s, b, i) => s + (B.visible[i] ? b.x1 - b.x0 : 0), 0)
}

/**
 * Width the dragged tab can go to in its own tab bar order move — `dragAreaWidth` clamped to the sum of visible tab widths (the right end of the last position).
 * Passed as `A` of `draggedLeft`. It reaches the ideal x of the last position (= sum − dragged tab width) exactly, so the insertion position is exactly Chrome's.
 */
// User decision 2026-10-22: Chrome sends it to the end of the drag area (`dragAreaWidth`), but we keep the dragged tab from going past the last tab position onto `+`
//   (measured — with two tabs, dragging to the right went beyond `+`).
export function reorderAreaWidth(available: number | null, B: StripBounds): number {
  return Math.min(dragAreaWidth(available, B), dragAreaWidth(null, B))
}

/**
 * Grab offset = `round(r · w0)` — `r` = grabbed width ratio (position of the pressed point inside the head ÷ head width) · `w0` = the dragged tab's width **at the start of the drag**
 * (dragstart). ★Computed once per drag★ — even if the tab width changes during the drag, the offset stays (Chrome sets `mouse_offset_` once in the session
 * constructor — dragging_tabs_session.cc#50).
 */
// Ported to a single tab from Chromium (BSD) `CalculateMouseOffset` (dragging_tabs_session.cc#25-36).
export function grabOffset(r: number, w0: number): number {
  return roundHalfAway(r * w0)
}

/**
 * Left `L` to draw the dragged tab. `p` = cursor x · `offset` = `grabOffset` · `w` = current width of the dragged tab · `A` = `dragAreaWidth`.
 * If the dragged tab is wider than the drag area it is always 0 (Chrome comment · crbug.com/40791711).
 */
// Ported to a single tab from Chromium (BSD) `DraggingTabsSession::GetAttachedDragPoint` (dragging_tabs_session.cc#329-343).
export function draggedLeft(p: number, offset: number, w: number, A: number): number {
  return Math.min(Math.max(p - offset, 0), Math.max(0, A - w))
}

/**
 * Position to insert the dragged tab — **based on the list without the dragged tab** (the dragged tab's position after moving there · the same = in place). `B` = ideal bounds of
 * the current order (including dragged tab `d` — `idealBounds(layout, d)`) · `L` = `draggedLeft`. It does not exceed `B.lastVisible`.
 * 0 if there are no tabs (Chrome) · otherwise `RangeError` when `d` is outside `B`.
 */
// Ported from Chromium (BSD) `CalculateInsertionIndex` · `IsValidInsertionIndex` · `CalculateIdealX` · `GetInsertionIndexForDraggedBounds`
//   (tab_strip.cc#980-1119 · #554-609).
export function insertionIndex(B: StripBounds, d: number, L: number): number {
  const T = B.tabs
  const n = T.length
  if (n === 0) return 0
  if (!Number.isInteger(d) || d < 0 || d >= n) throw new RangeError(`insertionIndex: 끄는 탭 자리 ${d} 가 0..${n - 1} 밖이다`)
  const w = T[d].x1 - T[d].x0

  let best = -1
  let bestDistance = Infinity
  for (let k = 0; k <= n; k += 1) {
    if (k === d + 1) continue
    const idealX = k === 0 ? 0 : T[k - 1].x1 - (k - 1 > d ? w : 0)
    const distance = Math.abs(L - idealX)
    if (distance < bestDistance) {
      bestDistance = distance
      best = k
    }
  }
  if (best > d) best -= 1

  // Chrome: last visible tab position + 1 (0 if none) − number of dragged tabs (1) is not lowered below 0.
  return Math.min(best, Math.max(0, B.lastVisible + 1 - 1))
}

/**
 * Distance the cursor must move before the order changes — proportional to the width of **the tab at the current position `to`** (may not be the dragged tab).
 * `RangeError` if `to` is outside `B`.
 */
// Ported from the threshold of Chromium (BSD) `DraggingTabsSession::MoveAttachedImpl` (dragging_tabs_session.cc#89-94).
export function reorderThreshold(B: StripBounds, to: number): number {
  const T = B.tabs
  if (!Number.isInteger(to) || to < 0 || to >= T.length) {
    throw new RangeError(`reorderThreshold: 자리 ${to} 가 0..${T.length - 1} 밖이다`)
  }
  return roundHalfAway(((T[to].x1 - T[to].x0) / TAB_STANDARD_W) * REORDER_THRESHOLD_PX)
}

/**
 * Is it time to compute an order move — right after attach without the threshold · otherwise when the threshold has been **exceeded** from the point `pLast` where the position last changed.
 * ★The caller moves `pLast` only when the position actually changes★ — Chrome 「Make sure the tabstrip actually changed before updating
 * `last_move_attached_context_loc_`」 (dragging_tabs_session.cc#143-149). If recorded every time the threshold is exceeded, the threshold would have to be exceeded again at the moment the position changes,
 * so the order change would be delayed by up to the threshold.
 */
// Ported to a single tab from Chromium (BSD) dragging_tabs_session.cc#101-104 (the non-contiguous branch does not exist for a single tab).
export function moveDue(pLast: number, p: number, thr: number, justAttached: boolean): boolean {
  return justAttached || Math.abs(p - pLast) > thr
}

/**
 * Left x each tab goes to after the order changes (`B′` = ideal bounds of the new order · `d` = the dragged tab's position in the new order). `null` = a tab that does not step aside —
 * the dragged tab (always drawn at `L` and kept on top while overlapping) · a hidden tab that is not drawn.
 */
export function slideTargets(B: StripBounds, d: number): (number | null)[] {
  return B.tabs.map((b, i) => (i === d || !B.visible[i] ? null : b.x0))
}

/**
 * Step-aside time (ms). `snap` = the first computation just after attaching to another window's tab bar — neighbor tabs do not slide and stand at the new position immediately (Chrome
 * 「otherwise those tabs will animate to those bounds after attach, which looks flickery/bad」 · dragging_tabs_session.cc#157-162 ·
 * the source window's first computation is `initial_move_`, so it does not apply). `reducedMotion` = `prefers-reduced-motion: reduce` (Chrome's rich animations off).
 */
export function slideMs(reducedMotion: boolean, snap: boolean): number {
  return reducedMotion || snap ? 0 : SLIDE_MS
}

/** Chrome `Tween::EASE_OUT` (tween.cc#72-73) — the curve `EASE_OUT_CSS` draws. `t` ∈ [0, 1]. */
export function easeOut(t: number): number {
  return 1 - (1 - t) ** 2
}

// TRD S21-slot-docking §4-3 「다른 창」 · W13: the drop position on another window's tab bar · 2 px vertical line (VS Code style) — tabs of that tab bar do not
//   move. The source window's own tab bar uses Chrome `insertionIndex` above.

/** Heads counted — visible and not `dragged` (current order). Hidden heads stand beyond the visible end, so removing them clips at the visible end (R24). */
function dropHeads(B: StripBounds, dragged: number | null): TabBounds[] {
  return B.tabs.filter((_, i) => B.visible[i] && i !== dragged)
}

/**
 * Drop position on another window's tab bar — the number of visible heads whose middle is left of `x` (if equal to the middle, that head's left = not counted).
 * Empty space · beyond the visible end = does not exceed the number of visible heads (the position after the last visible head — before hidden heads). The result is the `index` of `DropTo.tab_bar`
 * (the position where the dragged tab is inserted in that window's tab list). `x` = CSS px relative to the left of the drag area 0. `dragged` = the position of the dragged tab contained in this tab bar —
 * not counted (`null` for another window).
 */
export function dropIndex(B: StripBounds, x: number, dragged: number | null): number {
  return dropHeads(B, dragged).filter(b => (b.x0 + b.x1) / 2 < x).length
}

/**
 * x of the vertical line at the `dropIndex` position — the right end of the head left of that position (= left end of the right head · heads touch each other). If at the very front, the left
 * end of the first head, and 0 if there are no heads. The line (2 px) is drawn with 1 px at the end of the left head + 1 px at the end of the right head around this x (VS Code
 * `::after` · `::before`). `index` is the answer of `dropIndex` — if out of range, clamped to the nearest end.
 */
export function dropMarkerX(B: StripBounds, index: number, dragged: number | null): number {
  const heads = dropHeads(B, dragged)
  if (heads.length === 0) return 0
  const k = Math.min(Math.max(index, 0), heads.length)
  return k === 0 ? heads[0].x0 : heads[k - 1].x1
}

/** Rounding of Chromium `base::ClampRound` — .5 goes to the side away from 0 (`Math.round` sends negative .5 toward 0). */
function roundHalfAway(v: number): number {
  return Math.sign(v) * Math.round(Math.abs(v))
}
