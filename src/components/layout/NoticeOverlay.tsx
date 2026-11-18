// ADR-0306 — notice band area (below the title bar)
// The layer that covers the layout with notices without pushing it: the work happens in a fixed arrangement, so a notice must not push it on appearing, and covering the canvas top is acceptable — a notice closes with ✕. Several stack and overflow scrolls (ADR-0276 · connection band = ADR-0277).
// ★The title bar is not covered★ — covered window buttons · draggable area would leave the window neither movable nor closable. So the call site passes this layer as `WindowLayout`'s `notices` to attach it to the canvas area below the title bar (T-66).
// Every non-popup notice (band) goes on this layer and its height equals the drawn notice rows — which events are bands vs popups is ADR-0180 decision 2.
// ADR-0287
// ★The positioning reference is laid by the call site★ — this layer attaches to the very top of the nearest positioned ancestor; the call site supplies one, and this layer does not change its size (out of flow).

import type { ReactNode } from 'react'

import { ScrollArea } from '../ui/scroll-area'

/**
 * `children` = the notices to stack, in order from the top. No drawn notice (all `null`) → height 0.
 * ★A notice row must have an opaque background★ — the layer lays its own background to draw the gaps between rows (see `notice-stack` below); a transparent row would be drawn over it.
 * ★The notice's text area must wrap at any position (`wrap-anywhere`)★ — Radix Viewport wraps children in `display: table; min-width: 100%` (`@radix-ui/react-scroll-area` 1.2.11); a long word with no break point (a path) would widen the table beyond the window, clip the horizontal side and push the ✕ off screen (static read — not measured in the GUI).
 */
export default function NoticeOverlay({ children }: { children: ReactNode }) {
  return (
    // ★The position is taken by the wrapping div★ — the Root of Radix ScrollArea plants an inline `position: relative` that a class cannot override.
    // ★The layer's height equals the notice rows★ — a transparent full-height curtain would eat clicks outside the rows. Upper bound = 1/3 of the window height (stacked, 2/3 of the arrangement stays visible). The shadow is a `drop-shadow` following the drawn rows' shape, so it is absent with no rows.
    // z 40 = above the split lines · area curtain (z-20) · below the restore modal (z-50 — main only) · menus (1000 and above).
    <div
      data-testid="notice-overlay"
      className="drop-shadow-md"
      style={{ position: 'absolute', top: 0, left: 0, right: 0, zIndex: 40 }}
    >
      <ScrollArea viewportClassName="max-h-[33vh]">
        {/* Let the app background show through the 1px gap between rows — in dark, the notice background (`--surface-elevated`) and border (`--border`) are nearly the same, so overlapped row boundaries would be invisible. */}
        <div data-testid="notice-stack" className="flex flex-col gap-px bg-background">
          {children}
        </div>
      </ScrollArea>
    </div>
  )
}
