// TRD S21-slot-docking §4-3 · edition 6.3 §5-6 (R26 · research A10 · E5): closing mode — after a tab is closed with the mouse, the tab bar width is fixed so the next tab slides under the cursor, and layout resumes only after the mouse leaves the tab bar. Ported from the Chromium desktop tab bar (`TabContainerImpl` closing mode · `views::MouseWatcher`) for a zero-overlap · no pinned/group · left→right bar. Do not invent rules (Q23).
// ★If a TRD sentence and the Chromium source differ, follow the source.★
// Chromium is BSD-licensed — the algorithm is ported, not the code. Line refs (fetched 2026-10-18): tci = `chrome/browser/ui/views/tabs/tab_container_impl.cc`; `tab_strip.cc` in the same dir; `horizontal_tab_strip_region_view.cc` in `chrome/browser/ui/views/frame/`; `mouse_watcher.cc` · `mouse_watcher_view_host.cc` in `ui/views/`.
// Pure state line — no clocks · timers · DOM. Time = ms of the caller's clock; feeding `tick` at `deadline` is the caller's job.

import { TAB_MIN_ACTIVE_W, TAB_STANDARD_W } from './layout'

/** Margin widened below the tab bar so a vertical drift while closing across tabs does not exit. Chrome `kTabStripAnimationVSlop` (tci #1576). */
export const CLOSE_SLOP_BELOW_PX = 40

/** Margin widened toward the end so the way to `+` does not exit. Chrome `kTabStripAnimationHSlop` (tci #1580). */
export const CLOSE_SLOP_END_PX = 60

/** Time (ms) from leaving the boundary or window until the mode exits (cancelled if it returns). Chrome `kNotifyListenerTime` (mouse_watcher.cc#26); leaving the window uses the same value (`notify_on_exit_time_` default #109; tci has no `set_notify_on_exit_time`). */
export const CLOSE_LEAVE_DELAY_MS = 300

/** Rectangle `[x0, x1) × [y0, y1)` — CSS px (that window's viewport). */
export interface Rect {
  x0: number
  y0: number
  x1: number
  y1: number
}

/**
 * Boundary guarding the mode. `bar` = the tab bar view Chrome watches (`TabStrip`, tab_strip.cc#1717-1719) = `[first head's left, + layoutAvailable(mode, measured width)) × row height`. Its width is `min(preferred, usable)` (horizontal_tab_strip_region_view.cc#205-209 · #988-991), and in closing mode preferred = the fixed width (tci #911-927 · tab_strip.cc#2206-2218). ★Not the whole measured row★ — space after a narrower fixed width is outside.
 * Chrome fits the width to the last moving tab's end during animation; here the stopped-state value approximates it. Top · start are not widened.
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
 * Width to fix after a tab is removed = `current` − removed width + overlap (0). `current` = current fixed width (in mode) or the tab bar width at entry; `widths` = computed widths of all tabs **before** removal (`layout.ts`, including hidden); `closed` = removed position (not the last tab — Chrome does not fix the width for the last; `stepClosingMode` filters it); `nextActive` = position (before removal, ≠ `closed`) of the tab that becomes active if the removed one was active. Out-of-range or equal positions → `RangeError`.
 */
// Ported from Chromium (BSD) tci #1441-1473. ★When inactive tabs are narrower than the active one (`kInactiveWidthBelowActiveWidth`) and the active is removed, the freed width = the next active tab's current width★ — that tab grows to the active minimum, so subtracting the closed width would shrink the rest further and the next tab would miss the cursor. That layout = any computed width below the active minimum (`layout.ts` stage B; stage A is all at or above it).
export function frozenAvailable(current: number, widths: number[], closed: number, nextActive?: number): number {
  const n = widths.length
  if (!Number.isInteger(closed) || closed < 0 || closed >= n - 1) {
    throw new RangeError(`frozenAvailable: removed tab index ${closed} is outside 0..${n - 2}`)
  }
  const badNext = nextActive !== undefined && (!Number.isInteger(nextActive) || nextActive < 0 || nextActive >= n)
  if (badNext || nextActive === closed) {
    throw new RangeError(`frozenAvailable: next active index ${nextActive} is outside 0..${n - 1} or equals the removed index ${closed}`)
  }
  const inactiveBelowActive = widths.some(w => w < TAB_MIN_ACTIVE_W)
  return current - (nextActive !== undefined && inactiveBelowActive ? widths[nextActive] : widths[closed])
}

/** Closing mode. `available` = fixed width (`layout.ts` width cap, taken via `layoutAvailable`); `deadline` = exit time (caller's clock ms), `null` = mouse inside the boundary (never ends by time alone). */
export type ClosingMode = { on: false } | { on: true; available: number; deadline: number | null }

export const CLOSING_OFF: ClosingMode = { on: false }

export type ClosingInput =
  /**
   * A tab was removed. `mouse` = closed via its close button with the mouse — otherwise (keyboard · command · other path) the mode is not entered, and if already in it only the width is fixed (Chrome fixes it regardless of path — tci #1419-1474). `measured` = measured width heads can use (`null` = unmeasurable), used to compute the tab bar width on entry. Remaining fields = `frozenAvailable`.
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
 * Cap of the tab width computation = `min(frozen, measured)` in the mode, otherwise measured (`null` = unmeasurable). Fed as `widthCap` of `layout.ts` (visibility uses the measured width) and as the bar width of `closingBounds`.
 */
// Ported from Chromium (BSD) tci #1220-1227 `CalculateAvailableWidthForTabs`.
export function layoutAvailable(mode: ClosingMode, measured: number | null): number | null {
  if (!mode.on) return measured
  return measured !== null && Number.isFinite(measured) ? Math.min(mode.available, measured) : mode.available
}

// Ported from Chromium (BSD) tci #716-741 `EnterTabClosingMode` · #1419-1474 `UpdateClosingModeOnRemovedTab` · #1258-1272 `StartRemoveTabAnimation`. ★Chrome's "visible tab" is not overflow hiding★ — `IsTabVisible` (tci #1476-1512) checks only pinned · groups, so every tab is visible for us; tab count · last position · removed width count hidden tabs too.
function onClosed(mode: ClosingMode, input: Extract<ClosingInput, { kind: 'closed' }>): ClosingMode {
  if (!input.mouse && !mode.on) return mode
  const n = input.widths.length
  if (!Number.isInteger(input.closed) || input.closed < 0 || input.closed >= n) {
    throw new RangeError(`stepClosingMode: removed tab index ${input.closed} is outside 0..${n - 1}`)
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

/** Tab bar width at mode entry (Chrome `width()`) = `min(preferred, measured)`; preferred outside the mode = sum of all tabs' standard widths (tab_strip_layout_helper.cc `CalculatePreferredWidth`); if unmeasurable, the preferred width. */
function entryWidth(n: number, measured: number | null): number {
  const preferred = n * TAB_STANDARD_W
  return measured !== null && Number.isFinite(measured) ? Math.min(preferred, Math.floor(measured)) : preferred
}
