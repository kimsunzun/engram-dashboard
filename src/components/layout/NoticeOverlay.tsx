// ADR-0306 — notice band area (below the title bar)
// The layer that covers the layout with notices — it does not push the layout. Since the work happens in a fixed arrangement, a notice must not push it when it appears,
// and covering the top of the canvas is acceptable — a notice is closed with ✕. Several stack and overflow scrolls (user decision 2026-10-06 · ADR-0276 ·
// connection band = ADR-0277).
// ★The title bar is not covered★ — if window buttons · the draggable area were covered by a notice, the window could neither be moved nor closed. So the call site passes this layer
//   as `WindowLayout`'s `notices` to attach it to the canvas area below the title bar (user decision 2026-10-26 · T-66 — before that
//   covering the tab bar was acceptable).
// Every non-popup notice (band) goes on this layer and the layer height equals the drawn notice rows (user decision 2026-10-08) — which events are
//   bands and which are popups is decided by ADR-0180 decision 2.
// ADR-0287
// ★The positioning reference is laid by the call site★ — this layer attaches to the very top of the nearest positioned ancestor. The call site
//   supplies a positioned ancestor, and this layer does not change that ancestor's size (the layer is out of flow).

import type { ReactNode } from 'react'

import { ScrollArea } from '../ui/scroll-area'

/**
 * `children` = the notices to stack, in that order from the top. If no notice is drawn (all `null`) the layer's height is 0.
 *
 * ★A notice row must have an opaque background★ — the layer lays its own background to draw the gaps between rows (the comment on `notice-stack` below).
 * A transparent row is drawn over that background.
 *
 * ★The notice's text area must wrap at any position (`wrap-anywhere`)★ — Radix Viewport wraps children in
 * `display: table; min-width: 100%` (`@radix-ui/react-scroll-area` 1.2.11). If a long word with no break point (a path)
 * widens the table beyond the window, the horizontal side is clipped and the ✕ is pushed off screen (a static read — not measured in the GUI).
 */
export default function NoticeOverlay({ children }: { children: ReactNode }) {
  return (
    // ★The position is taken by the wrapping div★ — the Root of Radix ScrollArea plants an inline `position: relative` that a class cannot override.
    // ★The layer's height equals the notice rows★ — laying a transparent curtain of full height would eat clicks outside the rows too. Upper bound = 1/3 of the window height (session
    //   judgment — even stacked, 2/3 of the arrangement is visible). The shadow is a `drop-shadow` that follows the shape of the drawn rows, so it is not drawn with no rows.
    // z 40 = above the split lines · area curtain (z-20) · below the restore modal (z-50 — main only) · menus (1000 and above).
    <div
      data-testid="notice-overlay"
      className="drop-shadow-md"
      style={{ position: 'absolute', top: 0, left: 0, right: 0, zIndex: 40 }}
    >
      <ScrollArea viewportClassName="max-h-[33vh]">
        {/* Let the app background show through the 1px gap between rows — in dark, the notice background (`--surface-elevated`) and the border (`--border`) are nearly the same, so
              the boundary of overlapped rows is not visible (session judgment). */}
        <div data-testid="notice-stack" className="flex flex-col gap-px bg-background">
          {children}
        </div>
      </ScrollArea>
    </div>
  )
}
