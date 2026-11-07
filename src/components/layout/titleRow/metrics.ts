// TRD S26-window-chrome §7-6: the source of truth for title row size values (CSS px = DIP · zoom 1). The shell receives them only through the minimum window width report (§7-7).

import { MIN_PANE_PX } from '../splitPreview'
import { TAB_ADD_GAP, TAB_BUTTON_SIZE, TAB_MIN_ACTIVE_W } from '../tabStrip/layout'

export const TAB_BAR_H = 28

// The active · hover surface sits this far down on the head — the bar must show above it so the tab reads as a piece rising from the bar (Chrome). The head box is the row
//   height as is. The head content area and the `+` box are also stood from this height to the row bottom so the vertical centers of the × circle and `+` circle match, and the number badge also
//   stands its text in the same area to match the vertical center with the tab text. These 4 px are not a draggable area — the head press wins (§7-2 · R19).
export const SURFACE_TOP = 4

// Normal state top margin — Chrome's 「최대화 아닐 때 비활성 탭 위 6 px」 (slot docking R25), and also the share that keeps a fast upward drag path inside the source window
//   so it does not leave the window before the OS drag starts (W13). The value is decided by S13.
// ADR-0303 — Chrome-style title bar
export const TOP_GAP = 6
// Chrome does not extend the drag handle above tabs when maximized (condensed) (Chromium `TabStrip::CanExtendDragHandle`).
export const TOP_GAP_MAXIMIZED = 0

// Empty draggable area minimum = Chromium `FrameGrabHandle::CalculatePreferredSize` — leaves a grabbable spot even when the tab strip is full. It is not
//   replaced by the number badge (main decision M10).
export const GRAB_MIN_W = 42

/** Number badge width = max(`MARKER_MIN_W`, text width + 2 × `MARKER_PAD`) — one digit = 28 (main decision). It grows with the number of digits and is not clipped. */
export const MARKER_MIN_W = 28
export const MARKER_PAD = 8

// Windows 11 caption button width (convention — untested · S11). Height = row height.
export const CAPTION_BUTTON_W = 46

export function titleRowHeight(maximized: boolean): number {
  return (maximized ? TOP_GAP_MAXIMIZED : TOP_GAP) + TAB_BAR_H
}

export interface MinWindowSizeInput {
  /** Tab strip left margin — `leading_inset` of the chrome shape. */
  leadingInset: number
  /** Measured width of the number badge — `null` = the badge is not drawn (waiting mode). */
  markerW: number | null
  /** Whether the three window buttons are drawn. */
  buttons: boolean
}

/**
 * Minimum window size (TRD §7-7 · integer CSS px — rounded up). Width = left margin + one active tab + `+` box + empty draggable area + badge + three buttons · height =
 * normal state row height + area minimum. It is based on normal state — the shell applies it as the OS minimum size only to a normal-size window and widens it once on opening
 * (`src-tauri/src/chrome` — during maximize it is left in place without withdrawing · the maximized rectangle is the work area · R20). The badge width is not counted below `MARKER_MIN_W`
 * (a 0 width that could not be measured is also that value).
 */
export function minWindowSize(input: MinWindowSizeInput): { width: number; height: number } {
  const marker = input.markerW === null ? 0 : Math.max(MARKER_MIN_W, input.markerW)
  const width =
    input.leadingInset +
    TAB_MIN_ACTIVE_W +
    TAB_ADD_GAP +
    TAB_BUTTON_SIZE +
    GRAB_MIN_W +
    marker +
    (input.buttons ? 3 * CAPTION_BUTTON_W : 0)
  return { width: Math.ceil(width), height: titleRowHeight(false) + MIN_PANE_PX }
}
