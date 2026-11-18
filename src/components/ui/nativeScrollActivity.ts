//! nativeScrollActivity — an installer giving **native** scroll containers outside the seam the same scrollbar visibility rule as ScrollArea (ADR-0053 "exception = xterm").
//!
//! ★Why★: every scroll surface goes through the `ScrollArea` seam (Radix, `type="scroll"`) and shows the scrollbar **only while scrolling** (hides after `SCROLL_HIDE_DELAY_MS`, no hover reaction). Only the xterm terminal owns its `.xterm-viewport` and cannot be wrapped (ADR-0053), so it had an always-shown native scrollbar and the two output surfaces diverged.
//! ★What it does★: attaches `data-scroll-active` to the scroll node only while scrolling; CSS (the `.xterm .xterm-viewport` rule in `src/index.css`) gates the thumb by it — this makes **only the state** "is it scrolling now", no drawing. CSS alone is not enough: a native scrollbar is not a DOM element to attach/detach like Radix, and CSS has no "currently scrolling" selector.
//! ★Scroll event source is not told apart — intentional★: a programmatic `scrollTop` assignment (bottom pinning on terminal output) counts as "scrolling" like a user wheel — Radix `type="scroll"` behaves that way (RichSlot's bottom-pinned auto-scroll shows the seam's scrollbar), and telling them apart only here would make the two surfaces differ during streaming.
//! ★Does not touch scroll *behavior*★ — only reads events; no preventDefault, no scrollTop writes.

import { SCROLL_HIDE_DELAY_MS } from './scroll-area'

/**
 * Targets for the marker = native scrollers outside the seam; currently only the xterm viewport (ADR-0053 exception list). Before adding a selector, check whether that surface can go through the `ScrollArea` seam — if it can be wrapped, the seam is the answer and this list does not grow.
 */
const NATIVE_SCROLLER_SELECTOR = '.xterm-viewport'

/** The marker with which CSS (`index.css`) gates thumb visibility. If you rename it, change that rule too. */
export const SCROLL_ACTIVE_ATTR = 'data-scroll-active'

/**
 * Hook scroll watching on the whole document; returns the uninstaller (prevents accumulation on effect cleanup / HMR).
 * `capture: true` is required — `scroll` **does not bubble**; only a capture-phase listener sees all scroll nodes from one place, without intruding into the creation · disposal lifetime of xterm instances (the terminal slot's subscription · replay · resize wiring is sensitive).
 */
export function installNativeScrollActivity(doc: Document = document): () => void {
  // Per-node hide timer, pushed back while scrolling continues and firing once after the last scroll.
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
