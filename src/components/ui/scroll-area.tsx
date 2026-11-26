//! ScrollArea — the app-wide overlay scroll area seam (ADR-0053).
//!
//! ★Role★: a **swap point** thinly wrapping the Radix ScrollArea (@radix-ui/react-scroll-area) primitive. Every frontend
//!   scroll surface (chat, tree, popup lists, DOM observer, etc.) goes through this seam instead of raw `overflow:auto` —
//!   changing scrollbar look/behavior in one place (this component + the theme.css --scrollbar-* tokens) propagates to all
//!   screens (single source). Components and stores depend only on this seam and do not handle Radix Root/Viewport
//!   directly (§5 hands/brain separation — even if the scrollbar implementation is swapped wholesale later the consumer code is unchanged). Directly
//!   exposing the Radix Root is forbidden (ADR-0053 decision).
//!
//!   ★Exception = xterm★: TerminalSlot's xterm owns its own `.xterm-viewport` scrollbar and cannot be wrapped by this React component
//!   → instead a global rule in index.css styles it with the same --scrollbar-* tokens for visual unity
//!   (the component and CSS paths reference the same tokens = single source preserved).
//!
//! ★Invariants (do not change — basis ADR-0053)★:
//!   - TRUE overlay: native scrollbar hidden (the Radix viewport removes `::-webkit-scrollbar`) + the custom
//!     scrollbar floats absolute over the content with gutter 0 (content width unchanged).
//!   - Appears only while scrolling (type="scroll") · hides after scrollHideDelay (500ms) once scrolling stops. It does not react to
//!     hover (ADR-0053 user decision — resolves the scrollbar showing too early from whole-screen hover).
//!   - ★Scroll target = ScrollArea.Viewport (not Root)★: the real overflow/scrollTop is the viewport DOM node.
//!     RichSlot/DomSlot's scroll-follow (ADR-0242 — writes to the bottom only while attached) must measure and write this node,
//!     so the ref is forwarded to the Viewport. Aiming at the Root makes scrolling not work (regression warning).
//!   - ★children's absolute overlay attaches to the Root★: only the Root is positioned (relative), and the Viewport and the content
//!     wrapper inside it must not be positioned. That way an `absolute inset-0` veil that children lay on does not scroll
//!     and covers exactly this area — RichSlot's history-wait loading veil relies on this (ADR-0226). Giving the Viewport
//!     a position makes the veil scroll with the content, and giving it to the wrapper shrinks it to the content height.

import { forwardRef, type ReactNode } from 'react'
import * as RadixScrollArea from '@radix-ui/react-scroll-area'

import { cn } from '@/lib/utils'
import './scroll-area.css'

/**
  * Lingering before the scrollbar hides after scrolling stops — ADR-0053 requirement (4) "hide after ~0.5s".
 *
  * ★Why it is public = so a second implementation does not copy the same value★: xterm cannot be wrapped by this component and
  *   implements the visibility rule separately with CSS + `nativeScrollActivity` (ADR-0053 "exception = xterm"), and writing the hide delay again as
  *   a number there would let only one side change and silently diverge the two surfaces' rules. This is the single source.
 */
export const SCROLL_HIDE_DELAY_MS = 500

export interface ScrollAreaProps {
  children: ReactNode
  className?: string
  viewportClassName?: string
  style?: React.CSSProperties
  viewportStyle?: React.CSSProperties
  /** When a horizontal list/code view appears, extend here (keep the single seam). */
  orientation?: 'vertical' | 'horizontal'
  /** Marker to check mount presence and target in cdp eval and tests. */
  'data-testid'?: string
}

// ADR-0053: the ref is forwarded to the Viewport (the real scroll node) — RichSlot/DomSlot plug scroll-follow (ADR-0242)
//   into this ref (header invariants above).
export const ScrollArea = forwardRef<HTMLDivElement, ScrollAreaProps>(function ScrollArea(
  { children, className, viewportClassName, style, viewportStyle, orientation = 'vertical', ...rest },
  viewportRef,
) {
  return (
    <RadixScrollArea.Root
      type="scroll"
      // ★scrollHideDelay★: 500ms lingering — vanishing right after scrolling ends feels abrupt, so it is left briefly
      scrollHideDelay={SCROLL_HIDE_DELAY_MS}
      className={cn('relative overflow-hidden', className)}
      style={style}
      data-testid={rest['data-testid']}
    >
      <RadixScrollArea.Viewport
        ref={viewportRef}
        // ★h-full w-full required★: the Radix Viewport must receive the parent (Root) height for scrolling to appear when the inner content overflows.
        //   The Root gets height as a flex child (min-h-0 flex-1) and the Viewport fills it.
        className={cn('h-full w-full', viewportClassName)}
        style={viewportStyle}
      >
        {children}
      </RadixScrollArea.Viewport>
      <RadixScrollArea.Scrollbar
        orientation={orientation}
        // With type="scroll" Radix mounts it only while scrolling and unmounts it after scrollHideDelay
        className="engram-scrollbar"
      >
        <RadixScrollArea.Thumb className="engram-scrollbar-thumb" />
      </RadixScrollArea.Scrollbar>
      <RadixScrollArea.Corner />
    </RadixScrollArea.Root>
  )
})

export default ScrollArea
