// nativeScrollActivity — verification of the state half (marker) of the "scrollbar only while scrolling" rule.
//
// ★What is measured here★: **what the marker attaches in response to and when it comes off**. That is the core of the difference the user pointed out (the terminal
//   showed the scrollbar on screen hover alone), and we also measure that the delay is the same constant as the seam (ScrollArea).
// ★What cannot be measured here★: whether the thumb is actually painted (jsdom does not compute the ::-webkit-scrollbar cascade)
//   — the CSS side is covered by the source gate in index.css.test.ts + cdp measurement.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { installNativeScrollActivity, SCROLL_ACTIVE_ATTR } from './nativeScrollActivity'
import { SCROLL_HIDE_DELAY_MS } from './scroll-area'

/** Reproduce at minimum the structure xterm makes (.xterm > .xterm-viewport). */
function mountViewport(): HTMLDivElement {
  const term = document.createElement('div')
  term.className = 'xterm'
  const viewport = document.createElement('div')
  viewport.className = 'xterm-viewport'
  term.appendChild(viewport)
  document.body.appendChild(term)
  return viewport
}

let dispose: (() => void) | null = null

beforeEach(() => {
  vi.useFakeTimers()
  dispose = installNativeScrollActivity()
})

afterEach(() => {
  dispose?.()
  dispose = null
  vi.useRealTimers()
  document.body.innerHTML = ''
})

describe('installNativeScrollActivity (ADR-0053: visibility rule of native scrollers outside the seam)', () => {
  it('the marker attaches only when scrolling — merely having the pointer come up does not attach it', () => {
    const vp = mountViewport()

    vp.dispatchEvent(new Event('mouseover', { bubbles: true }))
    vp.dispatchEvent(new Event('mouseenter'))
    vp.dispatchEvent(new Event('pointerover', { bubbles: true }))
    expect(vp.hasAttribute(SCROLL_ACTIVE_ATTR)).toBe(false)

    vp.dispatchEvent(new Event('scroll'))
    expect(vp.hasAttribute(SCROLL_ACTIVE_ATTR)).toBe(true)
  })

  it('after the last scroll it stays for the same delay as the seam and then drops', () => {
    const vp = mountViewport()
    vp.dispatchEvent(new Event('scroll'))

    vi.advanceTimersByTime(SCROLL_HIDE_DELAY_MS - 1)
    expect(vp.hasAttribute(SCROLL_ACTIVE_ATTR)).toBe(true)

    vi.advanceTimersByTime(1)
    expect(vp.hasAttribute(SCROLL_ACTIVE_ATTR)).toBe(false)
  })

  it('while scrolling continues hiding is postponed (prevents flicker during continuous scrolling)', () => {
    const vp = mountViewport()

    vp.dispatchEvent(new Event('scroll'))
    vi.advanceTimersByTime(SCROLL_HIDE_DELAY_MS - 1)
    vp.dispatchEvent(new Event('scroll'))
    vi.advanceTimersByTime(SCROLL_HIDE_DELAY_MS - 1)

    expect(vp.hasAttribute(SCROLL_ACTIVE_ATTR)).toBe(true)
  })

  it('a scroller not on the outside-seam target list is not touched', () => {
    const other = document.createElement('div')
    other.className = 'some-other-scroller'
    document.body.appendChild(other)

    other.dispatchEvent(new Event('scroll'))
    expect(other.hasAttribute(SCROLL_ACTIVE_ATTR)).toBe(false)
  })

  it('the marker and the remaining marker are removed together by the releaser', () => {
    const vp = mountViewport()
    vp.dispatchEvent(new Event('scroll'))
    expect(vp.hasAttribute(SCROLL_ACTIVE_ATTR)).toBe(true)

    dispose?.()
    dispose = null
    // If a marker attached at teardown time remains, the scrollbar hardens into always shown.
    expect(vp.hasAttribute(SCROLL_ACTIVE_ATTR)).toBe(false)

    vp.dispatchEvent(new Event('scroll'))
    expect(vp.hasAttribute(SCROLL_ACTIVE_ATTR)).toBe(false)
  })
})
