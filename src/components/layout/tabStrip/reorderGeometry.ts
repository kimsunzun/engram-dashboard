// TRD S21-slot-docking §4-1 · §4-2 · edition 6.3 §5-1 (C1 · C2 · C3): pure geometry of the in-tab-bar order move, ported from Chromium's desktop tab bar drag (`DraggingTabsSession::MoveAttachedImpl` · `TabStrip::GetInsertionIndexForDraggedBounds`) for one tab · no pinned/groups/splits · overlap 0. Do not invent rules (Q23).
// ★If a TRD sentence and the Chromium source differ, follow the source.★
// Chromium is BSD-licensed — the algorithm is ported, not the code. `#NNN` = line refs in chromium/src main (fetched 2026-10-18): `tab_strip.cc` · `tab_container_impl.cc` (chrome/browser/ui/views/tabs/), `dragging_tabs_session.cc` (…/tabs/dragging/), `tween.cc` · `animation.cc` (ui/gfx/animation/), `bounds_animator.h` (ui/views/animation/).
// Coordinates = CSS px, drag-area left (= first tab left) is 0. Only grab offset · threshold are rounded.

import { TAB_STANDARD_W, type TabStripLayout } from './layout'

/** Order threshold for a standard width tab. Chrome `kHorizontalMoveThreshold` (dragging_tabs_session.cc#89). */
export const REORDER_THRESHOLD_PX = 16

/** Time (ms) for non-dragged tabs to slide after an order change. Chrome `RichAnimationDuration` (tab_container_impl.cc#1153-1160 · animation.cc#114-116); 0 when rich animations are off. */
export const SLIDE_MS = 200

/** Step-aside curve: Chrome `BoundsAnimator` default `EASE_OUT` `1 − (1 − t)²` (bounds_animator.h#205 · tween.cc#72-73) as the exact cubic-bezier. */
export const EASE_OUT_CSS = 'cubic-bezier(0.3333, 0.6667, 0.6667, 1)'

/** Ideal bounds `[x0, x1)` of one tab; hidden tabs keep their width beyond the visible end. */
export interface TabBounds {
  x0: number
  x1: number
}

export interface StripBounds {
  /** All tabs (current model order · including the dragged tab). */
  tabs: TabBounds[]
  /** Tabs that are drawn — layout visibility + the dragged tab. */
  visible: boolean[]
  /** Position of the last visible tab (−1 if none) — the insertion position is clamped inside this (tab_strip.cc#589-600). */
  lastVisible: number
}

/**
 * `layout.ts` result → ideal bounds: accumulation of ALL tabs' computed widths — hidden tabs keep their width beyond the visible end (as Chrome).
 * ★Do not stack hidden tabs at the visible end with width 0★ — the in-place candidate after them would tie in x with the candidate before a hidden tab, the earlier wins, and a still dragged tab moves one position forward.
 * `dragged` = dragged tab position, counted visible even if the layout hides it (tab_container_impl.cc#1688-1697).
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
 * Tab drag area width `A`: the dragged tab may go anywhere within it, not only over the tabs' range. `available` = measured width heads can use (from `layout.ts`) — ★never the fixed closing-mode width★ (Chrome: "needs to ignore tab closing mode").
 * Chrome's `max(usable width, tab bar width)` reduces to `available` since the tab bar width is capped by preferred width (horizontal_tab_strip_region_view.cc#205-209). `null` / non-finite → sum of visible tab widths.
 */
// Ported from Chromium (BSD) `TabStrip::GetTabDragAreaWidth` (tab_strip.cc#530-544).
export function dragAreaWidth(available: number | null, B: StripBounds): number {
  if (available !== null && Number.isFinite(available)) return available
  return B.tabs.reduce((s, b, i) => s + (B.visible[i] ? b.x1 - b.x0 : 0), 0)
}

/** Width the dragged tab can reach in its own tab bar: `dragAreaWidth` clamped to the sum of visible tab widths; passed as `A` of `draggedLeft`. */
// Deliberate deviation from Chrome (which allows the whole drag area): the dragged tab must not go past the last tab position onto `+`.
export function reorderAreaWidth(available: number | null, B: StripBounds): number {
  return Math.min(dragAreaWidth(available, B), dragAreaWidth(null, B))
}

/** Grab offset = `round(r · w0)`: `r` = grabbed width ratio, `w0` = dragged tab width at dragstart. ★Computed once per drag★ even if the width changes (dragging_tabs_session.cc#50). */
// Ported to a single tab from Chromium (BSD) `CalculateMouseOffset` (dragging_tabs_session.cc#25-36).
export function grabOffset(r: number, w0: number): number {
  return roundHalfAway(r * w0)
}

/** Left `L` of the dragged tab. `p` = cursor x, `offset` = `grabOffset`, `w` = current dragged tab width, `A` = `dragAreaWidth`. Wider than the drag area → always 0 (crbug.com/40791711). */
// Ported to a single tab from Chromium (BSD) `DraggingTabsSession::GetAttachedDragPoint` (dragging_tabs_session.cc#329-343).
export function draggedLeft(p: number, offset: number, w: number, A: number): number {
  return Math.min(Math.max(p - offset, 0), Math.max(0, A - w))
}

/**
 * Insertion position, **based on the list without the dragged tab** (same as its own position = in place). `B` = ideal bounds including dragged tab `d`, `L` = `draggedLeft`. Never exceeds `B.lastVisible`.
 * 0 if no tabs; `RangeError` when `d` is outside `B`.
 */
// Ported from Chromium (BSD) `CalculateInsertionIndex` · `IsValidInsertionIndex` · `CalculateIdealX` · `GetInsertionIndexForDraggedBounds`
//   (tab_strip.cc#980-1119 · #554-609).
export function insertionIndex(B: StripBounds, d: number, L: number): number {
  const T = B.tabs
  const n = T.length
  if (n === 0) return 0
  if (!Number.isInteger(d) || d < 0 || d >= n) throw new RangeError(`insertionIndex: dragged tab index ${d} is outside 0..${n - 1}`)
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

/** Cursor distance before the order changes — proportional to the width of the tab at the current position `to` (may not be the dragged one). `RangeError` if `to` is outside `B`. */
// Ported from the threshold of Chromium (BSD) `DraggingTabsSession::MoveAttachedImpl` (dragging_tabs_session.cc#89-94).
export function reorderThreshold(B: StripBounds, to: number): number {
  const T = B.tabs
  if (!Number.isInteger(to) || to < 0 || to >= T.length) {
    throw new RangeError(`reorderThreshold: index ${to} is outside 0..${T.length - 1}`)
  }
  return roundHalfAway(((T[to].x1 - T[to].x0) / TAB_STANDARD_W) * REORDER_THRESHOLD_PX)
}

/**
 * Whether to compute an order move: right after attach (no threshold) or when the threshold is **exceeded** from `pLast`.
 * ★The caller moves `pLast` only when the position actually changes★ (dragging_tabs_session.cc#143-149) — recording on every exceed would delay the order change by up to the threshold.
 */
// Ported to a single tab from Chromium (BSD) dragging_tabs_session.cc#101-104 (the non-contiguous branch does not exist for a single tab).
export function moveDue(pLast: number, p: number, thr: number, justAttached: boolean): boolean {
  return justAttached || Math.abs(p - pLast) > thr
}

/** Left x of each tab after the order change (`B′` = new-order bounds, `d` = dragged position there). `null` = no step-aside: the dragged tab (drawn at `L`, on top) or a hidden tab. */
export function slideTargets(B: StripBounds, d: number): (number | null)[] {
  return B.tabs.map((b, i) => (i === d || !B.visible[i] ? null : b.x0))
}

/**
 * Step-aside time (ms). `snap` = first computation right after attaching to another window's tab bar — neighbors jump to the new position instead of sliding (dragging_tabs_session.cc#157-162; the source window's first computation is `initial_move_`, so it does not apply). `reducedMotion` = `prefers-reduced-motion: reduce`.
 */
export function slideMs(reducedMotion: boolean, snap: boolean): number {
  return reducedMotion || snap ? 0 : SLIDE_MS
}

/** Chrome `Tween::EASE_OUT` (tween.cc#72-73) — the curve `EASE_OUT_CSS` draws. `t` ∈ [0, 1]. */
export function easeOut(t: number): number {
  return 1 - (1 - t) ** 2
}

// TRD S21-slot-docking §4-3 · W13: drop position on another window's tab bar — 2 px vertical line (VS Code style); that bar's tabs do not move. The source window's own bar uses `insertionIndex` above.

/** Heads counted — visible and not `dragged` (current order). Hidden heads stand beyond the visible end, so removing them clips at the visible end (R24). */
function dropHeads(B: StripBounds, dragged: number | null): TabBounds[] {
  return B.tabs.filter((_, i) => B.visible[i] && i !== dragged)
}

/**
 * Drop position on another window's tab bar: number of visible heads whose middle is left of `x` (equal = not counted); capped at the visible head count (before hidden heads). Result = `index` of `DropTo.tab_bar`. `x` = CSS px from drag-area left; `dragged` = position of the dragged tab if in this bar (not counted), `null` for another window.
 */
export function dropIndex(B: StripBounds, x: number, dragged: number | null): number {
  return dropHeads(B, dragged).filter(b => (b.x0 + b.x1) / 2 < x).length
}

/**
 * x of the vertical line at a `dropIndex` position: the right end of the head left of it (= left of the right head); first position → left of the first head; no heads → 0. The 2 px line is drawn as 1 px each side of x (VS Code `::after` · `::before`). Out-of-range `index` clamps.
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
