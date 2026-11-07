// TRD S21-slot-docking §4-3 「그대로인 것」 · edition 6.3 §5-6 (R26 · research A10 · E5): closing mode — when a tab is closed with the mouse, the width the tab bar uses is fixed so the next tab slides under the cursor,
//   and it is laid out again only after the mouse leaves the tab bar. The Chromium desktop tab bar (closing mode of `TabContainerImpl` ·
//   `views::MouseWatcher`) was ported to a zero-overlap · no pinned tab · no group · left→right tab bar. Do not invent rules (Q23).
//   ★If a TRD sentence and the Chromium source differ, follow the source★ (main decision 2026-10-18 — 「Chrome 소스 그대로」).
// Chromium source is under a BSD-style license (「Copyright The Chromium Authors」 · its LICENSE) — what was ported is the algorithm, not the code
//   sentences. Line numbers = chromium/src main (fetched 2026-10-18): tci = `chrome/browser/ui/views/tabs/tab_container_impl.cc` ·
//   `tab_strip.cc` = `chrome/browser/ui/views/tabs/` · `horizontal_tab_strip_region_view.cc` = `chrome/browser/ui/views/frame/` ·
//   `mouse_watcher.cc` · `mouse_watcher_view_host.cc` = `ui/views/`.
// It is a pure state line — it knows neither clocks · timers nor the DOM. Time is the ms of the caller's clock, and feeding `tick` at the deadline (`deadline`) is also
//   the caller's job.

import { TAB_MIN_ACTIVE_W, TAB_STANDARD_W } from './layout'

/** Margin widened below the tab bar — so that closing while moving across tabs and drifting a little vertically does not exit. Chrome `kTabStripAnimationVSlop` (tci #1576). */
export const CLOSE_SLOP_BELOW_PX = 40

/** Margin widened toward the end (new tab button) — so that the way to `+` during the mode does not exit. Chrome `kTabStripAnimationHSlop` (tci #1580). */
export const CLOSE_SLOP_END_PX = 60

/**
 * Time (ms) from moving outside the boundary or leaving the window until the mode exits — cancelled if it returns within. Chrome `kNotifyListenerTime`
 * (mouse_watcher.cc#26) — leaving the window uses the same value (`notify_on_exit_time_` default #109 · closing mode does not change it — tci has no
 * `set_notify_on_exit_time`).
 */
export const CLOSE_LEAVE_DELAY_MS = 300

/** Rectangle `[x0, x1) × [y0, y1)` — CSS px (that window's viewport). */
export interface Rect {
  x0: number
  y0: number
  x1: number
  y1: number
}

/**
 * Boundary that guards the mode. `bar` = the rectangle of the tab bar view Chrome watches (`TabStrip` — tab_strip.cc#1717-1719) =
 * `[첫 머리 왼쪽, + layoutAvailable(mode, 잰 폭)) × 탭바 줄 높이`. That view's width is `min(선호 폭, 쓸 수 있는 폭)`
 * (horizontal_tab_strip_region_view.cc#205-209 · #988-991 — stretch cap = preferred), and during closing mode the preferred width = the fixed width
 * (tci #911-927 · tab_strip.cc#2206-2218). ★It is not the whole measured tab bar row★ — if the fixed width is narrower than the row width, the empty space after it is outside.
 * Chrome fits that width to the end of the last moving tab during animation — here it is approximated with the value of the stopped state. Top · start side are not
 * widened.
 */
// Ported from Chromium (BSD) tci #1570-1590 `AddMessageLoopObserver`'s `Insets::TLBR(0, 0, 40, 60)` (left→right) · the half-open decision
//   mouse_watcher_view_host.cc#30-41.
export function closingBounds(bar: Rect): Rect {
  return { x0: bar.x0, x1: bar.x1 + CLOSE_SLOP_END_PX, y0: bar.y0, y1: bar.y1 + CLOSE_SLOP_BELOW_PX }
}

/** Is the point inside the boundary — half-open, so below 39 · toward the end 59 are inside · 40 · 60 are outside (AC40). */
export function inClosingBounds(bar: Rect, x: number, y: number): boolean {
  const b = closingBounds(bar)
  return b.x0 <= x && x < b.x1 && b.y0 <= y && y < b.y1
}

/**
 * Width to fix after a tab is removed = `current` − the removed width + overlap (0). `current` = the current fixed width (during the mode) or the tab bar width at entry ·
 * `widths` = the computed widths of all tabs **before** removal (`widths` of `layout.ts` — including hidden tabs) · `closed` = the position of the removed tab (it must not be
 * the last tab — for the last, Chrome does not fix the width · `stepClosingMode` filters it) · `nextActive` = if the removed tab was active, the position of the tab that will
 * become active next (position before removal · different from `closed`). If a position is out of range or the two are equal, `RangeError`.
 */
// Ported from Chromium (BSD) tci #1441-1473. ★In a layout where inactive tabs are narrower than the active one (`kInactiveWidthBelowActiveWidth`), when the active is removed, the removed
//   width = the current width of the next active tab★ — that tab grows to the active minimum width, so what actually becomes free is that width. If the closed width were subtracted, the remaining tabs would shrink
//   further and the next tab would not come under the cursor. That layout = even one computed width is narrower than the active minimum width (`layout.ts` stage B · stage A is all
//   at or above it).
export function frozenAvailable(current: number, widths: number[], closed: number, nextActive?: number): number {
  const n = widths.length
  if (!Number.isInteger(closed) || closed < 0 || closed >= n - 1) {
    throw new RangeError(`frozenAvailable: 빠진 탭 자리 ${closed} 가 0..${n - 2} 밖이다`)
  }
  const badNext = nextActive !== undefined && (!Number.isInteger(nextActive) || nextActive < 0 || nextActive >= n)
  if (badNext || nextActive === closed) {
    throw new RangeError(`frozenAvailable: 다음 활성 자리 ${nextActive} 가 0..${n - 1} 밖이거나 빠진 자리 ${closed} 와 같다`)
  }
  const inactiveBelowActive = widths.some(w => w < TAB_MIN_ACTIVE_W)
  return current - (nextActive !== undefined && inactiveBelowActive ? widths[nextActive] : widths[closed])
}

/**
 * Closing mode. `available` = fixed width (the width cap of `layout.ts` — taken out with `layoutAvailable`) · `deadline` = exits at this time (the caller's clock
 * ms) · `null` = the mouse is inside the boundary so there is no deadline (it does not end by time alone).
 */
export type ClosingMode = { on: false } | { on: true; available: number; deadline: number | null }

export const CLOSING_OFF: ClosingMode = { on: false }

export type ClosingInput =
  /**
   * A tab was removed from the model. `mouse` = its close button was pressed with the mouse to close it — otherwise (keyboard · command · another path) it does not enter the mode,
   * and if in the mode only the width is fixed (Chrome fixes it regardless of the removal path — tci #1419-1474). `measured` = the measured 「머리들이
   * 쓸 수 있는 폭」 (`null` = could not be measured) — the tab bar width is computed on entry. The remaining fields = `frozenAvailable`.
   */
  | { kind: 'closed'; mouse: boolean; widths: number[]; closed: number; measured: number | null; nextActive?: number }
  | { kind: 'move'; x: number; y: number; bar: Rect; now: number }
  | { kind: 'press'; x: number; y: number; bar: Rect }
  /** The mouse left the window. */
  | { kind: 'leaveWindow'; now: number }
  /** The caller's timer — fed in at `deadline`. */
  | { kind: 'tick'; now: number }
  /** A tab was added (`+` · attach · command) — exits immediately (Chrome `StartInsertTabAnimation` · tci #1229-1230). */
  | { kind: 'inserted' }

// Ported from Chromium (BSD) mouse_watcher.cc#63-86 `HandleMouseEvent` · mouse_watcher_view_host.cc#19-26 `Contains`.
export function stepClosingMode(mode: ClosingMode, input: ClosingInput): ClosingMode {
  switch (input.kind) {
    case 'closed':
      return onClosed(mode, input)
    case 'move':
      if (!mode.on) return mode
      if (inClosingBounds(input.bar, input.x, input.y)) return mode.deadline === null ? mode : { ...mode, deadline: null }
      // If a deadline is already set, it is not set again — 300 ms counts from the first time it went outside (mouse_watcher.cc#70 `!HasWeakPtrs()`).
      return mode.deadline !== null ? mode : { ...mode, deadline: input.now + CLOSE_LEAVE_DELAY_MS }
    case 'press':
      if (!mode.on) return mode
      if (!inClosingBounds(input.bar, input.x, input.y)) return CLOSING_OFF
      return mode.deadline === null ? mode : { ...mode, deadline: null }
    case 'leaveWindow':
      if (!mode.on || mode.deadline !== null) return mode
      return { ...mode, deadline: input.now + CLOSE_LEAVE_DELAY_MS }
    case 'tick':
      return mode.on && mode.deadline !== null && input.now >= mode.deadline ? CLOSING_OFF : mode
    case 'inserted':
      return CLOSING_OFF
  }
}

/**
 * Cap of the tab width computation = `min(고정 폭, 잰 폭)` during the mode · otherwise the measured width (`measured` · `null` = could not be measured). Fed in as `widthCap` of
 * `layout.ts` (visibility is measured separately with the measured width) · also used as the tab bar width of `closingBounds`.
 */
// Ported from Chromium (BSD) tci #1220-1227 `CalculateAvailableWidthForTabs`.
export function layoutAvailable(mode: ClosingMode, measured: number | null): number | null {
  if (!mode.on) return measured
  return measured !== null && Number.isFinite(measured) ? Math.min(mode.available, measured) : mode.available
}

// Ported from Chromium (BSD) tci #716-741 `EnterTabClosingMode` · #1419-1474 `UpdateClosingModeOnRemovedTab` · #1258-1272
//   `StartRemoveTabAnimation`. ★Chrome's 「보이는 탭」 is not overflow hiding★ — `IsTabVisible` (tci #1476-1512) looks only at pinned tabs ·
//   groups, so for us every tab is visible. So the tab count · last position · removed width count hidden tabs too.
function onClosed(mode: ClosingMode, input: Extract<ClosingInput, { kind: 'closed' }>): ClosingMode {
  if (!input.mouse && !mode.on) return mode
  const n = input.widths.length
  if (!Number.isInteger(input.closed) || input.closed < 0 || input.closed >= n) {
    throw new RangeError(`stepClosingMode: 빠진 탭 자리 ${input.closed} 가 0..${n - 1} 밖이다`)
  }
  // With one tab or fewer there is no point in fixing (tci #1423-1430).
  if (n <= 1) return CLOSING_OFF
  // If in the mode, the current fixed width (tci #1470-1473) · on entry, the tab bar width at that time (tci #730-733 `width()`).
  const current = mode.on ? mode.available : entryWidth(n, input.measured)
  // If it is the last tab, the width is not fixed — the remaining tabs widen within that width (tci #1432-1437). ★It does not exit the mode★.
  const available =
    input.closed === n - 1 ? current : frozenAvailable(current, input.widths, input.closed, input.nextActive)
  // If the fixed width is at or above the sum of the standard widths of the remaining tabs, the width is no longer being held (tci #1263-1270).
  if (available >= (n - 1) * TAB_STANDARD_W) return CLOSING_OFF
  return { on: true, available, deadline: mode.on ? mode.deadline : null }
}

/**
 * Tab bar width at mode entry (Chrome `width()`) = `min(선호 폭, 잰 폭)` — preferred width outside the mode = the sum of the standard widths of all tabs
 * (tab_strip_layout_helper.cc `CalculatePreferredWidth`). If it cannot be measured, the preferred width.
 */
function entryWidth(n: number, measured: number | null): number {
  const preferred = n * TAB_STANDARD_W
  return measured !== null && Number.isFinite(measured) ? Math.min(preferred, Math.floor(measured)) : preferred
}
