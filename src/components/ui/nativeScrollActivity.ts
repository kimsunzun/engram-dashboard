//! nativeScrollActivity — an installer that gives **native** scroll containers outside the seam the same scrollbar visibility
//! rule as ScrollArea (ADR-0053 "exception = xterm").
//!
//! ★Why it exists★: every scroll surface of the app goes through the `ScrollArea` seam (Radix, `type="scroll"`) and shows the
//!   scrollbar **only while scrolling** (after it stops it hides following `SCROLL_HIDE_DELAY_MS`, no reaction to hover). Only the xterm terminal
//!   owns its own `.xterm-viewport` and cannot be wrapped in that component (ADR-0053), so until now it had an **always-shown**
//!   native scrollbar — the rules of the two output surfaces diverged (user remark: "the terminal shows it just by hovering").
//!
//! ★What it does★: attaches `data-scroll-active` to the scroll node only while scrolling happens. The side that gates the thumb
//!   by that marker is CSS (the `.xterm .xterm-viewport` rule in `src/index.css`) — here we make **only the state**
//!   "is it scrolling right now" and no drawing.
//!   Why CSS alone is not enough: a native scrollbar is not a DOM element so it cannot be attached and detached like Radix, and CSS has
//!   no "currently scrolling" state selector (there are hover · overflow ones, but both are different rules).
//!
//! ★It does not tell the source of scroll events apart — intentional★: a programmatic `scrollTop` assignment (bottom
//!   pinning on terminal output) is counted as "scrolling" the same as a user wheel. Radix `type="scroll"` behaves exactly that way
//!   (measured that RichSlot's bottom-pinned auto-scroll shows the seam's scrollbar — 2026-08-21), and if only here the
//!   source were told apart, the two surfaces would look different during streaming.
//!
//! ★Does not touch scroll *behavior*★ — this module only reads events and does not call preventDefault or write scrollTop.
//!   All that changes is "when the scrollbar is visible".

import { SCROLL_HIDE_DELAY_MS } from './scroll-area'

/**
 * Targets to attach the marker to = native scrollers outside the seam. Currently only the xterm viewport (the ADR-0053 exception list).
 * Before adding a selector here, first check whether that surface cannot go through the `ScrollArea` seam —
 * if it can be wrapped, the seam is the right answer and this list does not grow.
 */
const NATIVE_SCROLLER_SELECTOR = '.xterm-viewport'

/** The marker with which CSS (`index.css`) gates thumb visibility. If you rename it, change that rule too. */
export const SCROLL_ACTIVE_ATTR = 'data-scroll-active'

/**
 * Hook scroll watching on the whole document. Return value = the uninstaller (prevents duplicate accumulation in the install site's effect cleanup / HMR).
 *
 * `capture: true` is required — the `scroll` event **does not bubble**. Only a capture-phase listener can see all scroll nodes from one place on the document,
 * and that way this code does not intrude into the creation · disposal lifetime of xterm instances
 * (meaning it does not touch the terminal slot's subscription · replay · resize wiring — that side is sensitive for other reasons).
 */
export function installNativeScrollActivity(doc: Document = document): () => void {
  // Per-node hide timer. It keeps being pushed back while scrolling continues and fires only once after the last scroll.
  const timers = new Map<Element, ReturnType<typeof setTimeout>>()

  const onScroll = (e: Event): void => {
    const el = e.target
    // Document scroll (target = document) · non-target scrollers are simply let through.
    if (!(el instanceof Element) || !el.matches(NATIVE_SCROLLER_SELECTOR)) return

    el.setAttribute(SCROLL_ACTIVE_ATTR, '1')

    const pending = timers.get(el)
    if (pending !== undefined) clearTimeout(pending)
    timers.set(
      el,
      setTimeout(() => {
        timers.delete(el)
        el.removeAttribute(SCROLL_ACTIVE_ATTR)
      }, SCROLL_HIDE_DELAY_MS),
    )
  }

  doc.addEventListener('scroll', onScroll, true)

  return () => {
    doc.removeEventListener('scroll', onScroll, true)
    // Leave no marker — if it stays attached after teardown the scrollbar hardens into a permanently visible state.
    for (const [el, id] of timers) {
      clearTimeout(id)
      el.removeAttribute(SCROLL_ACTIVE_ATTR)
    }
    timers.clear()
  }
}
