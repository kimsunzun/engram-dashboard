// nativeScrollActivity — "스크롤 중에만 스크롤바" 규칙의 상태 절반(표식) 검증.
//
// ★여기서 재는 것★: 표식이 **무엇에 반응해 붙고 언제 떨어지나**. 그게 사용자가 지적한 차이(터미널은
//   화면 hover 만으로 스크롤바가 보였다)의 본체이고, 지연이 seam(ScrollArea)과 같은 상수인지도 함께 잰다.
// ★여기서 못 재는 것★: thumb 이 실제로 칠해지는지(jsdom 은 ::-webkit-scrollbar 캐스케이드를 계산하지
//   않는다) — CSS 쪽은 index.css.test.ts 의 소스 게이트 + cdp 실측이 맡는다.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { installNativeScrollActivity, SCROLL_ACTIVE_ATTR } from './nativeScrollActivity'
import { SCROLL_HIDE_DELAY_MS } from './scroll-area'

/** xterm 이 만드는 구조(.xterm > .xterm-viewport)를 최소로 재현한다. */
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
    // 해제 시점에 붙어 있던 표식이 남으면 스크롤바가 상시 표시로 굳는다.
    expect(vp.hasAttribute(SCROLL_ACTIVE_ATTR)).toBe(false)

    vp.dispatchEvent(new Event('scroll'))
    expect(vp.hasAttribute(SCROLL_ACTIVE_ATTR)).toBe(false)
  })
})
