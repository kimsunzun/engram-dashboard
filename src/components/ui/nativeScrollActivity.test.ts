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

describe('installNativeScrollActivity (ADR-0053: seam 밖 네이티브 스크롤러 가시성 규칙)', () => {
  it('스크롤할 때만 표식이 붙는다 — 포인터가 올라오는 것만으로는 안 붙는다', () => {
    const vp = mountViewport()

    vp.dispatchEvent(new Event('mouseover', { bubbles: true }))
    vp.dispatchEvent(new Event('mouseenter'))
    vp.dispatchEvent(new Event('pointerover', { bubbles: true }))
    expect(vp.hasAttribute(SCROLL_ACTIVE_ATTR)).toBe(false)

    vp.dispatchEvent(new Event('scroll'))
    expect(vp.hasAttribute(SCROLL_ACTIVE_ATTR)).toBe(true)
  })

  it('마지막 스크롤 뒤 seam 과 같은 지연만큼 남았다가 떨어진다', () => {
    const vp = mountViewport()
    vp.dispatchEvent(new Event('scroll'))

    vi.advanceTimersByTime(SCROLL_HIDE_DELAY_MS - 1)
    expect(vp.hasAttribute(SCROLL_ACTIVE_ATTR)).toBe(true)

    vi.advanceTimersByTime(1)
    expect(vp.hasAttribute(SCROLL_ACTIVE_ATTR)).toBe(false)
  })

  it('스크롤이 이어지는 동안 숨김이 미뤄진다(연속 스크롤 중 깜빡임 방지)', () => {
    const vp = mountViewport()

    vp.dispatchEvent(new Event('scroll'))
    vi.advanceTimersByTime(SCROLL_HIDE_DELAY_MS - 1)
    vp.dispatchEvent(new Event('scroll'))
    vi.advanceTimersByTime(SCROLL_HIDE_DELAY_MS - 1)

    expect(vp.hasAttribute(SCROLL_ACTIVE_ATTR)).toBe(true)
  })

  it('seam 밖 대상 명단에 없는 스크롤러는 건드리지 않는다', () => {
    const other = document.createElement('div')
    other.className = 'some-other-scroller'
    document.body.appendChild(other)

    other.dispatchEvent(new Event('scroll'))
    expect(other.hasAttribute(SCROLL_ACTIVE_ATTR)).toBe(false)
  })

  it('해제기가 리스너와 남은 표식을 함께 거둔다', () => {
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
