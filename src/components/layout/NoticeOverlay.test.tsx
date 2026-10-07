// NoticeOverlay — 알림을 레이아웃 위에 덮는 층(ADR-0276 · ADR-0277).
//
// jsdom 은 크기를 재지 못한다 — 「밀지 않는다 · 줄만큼만 덮는다」를 계산된 위치와 구조로 단언한다.

import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import NoticeOverlay from './NoticeOverlay'

afterEach(cleanup)

function Nothing() {
  return null
}

const overlay = () => screen.getByTestId('notice-overlay')
const stack = () => screen.getByTestId('notice-stack')

describe('NoticeOverlay', () => {
  it('계산된 위치가 absolute 이고 기준 칸의 맨 위 · 좌우 끝에 붙으며 z 40 이다', () => {
    render(
      <NoticeOverlay>
        <div role="status">a</div>
      </NoticeOverlay>,
    )
    const style = getComputedStyle(overlay())
    expect(style.position).toBe('absolute')
    expect([style.top, style.left, style.right]).toEqual(['0px', '0px', '0px'])
    expect(style.zIndex).toBe('40')
    expect(overlay().contains(screen.getByRole('status'))).toBe(true)
  })

  it('줄만큼만 덮는다 — 높이 · 아래 끝을 정하지 않는다', () => {
    render(
      <NoticeOverlay>
        <div role="status">a</div>
      </NoticeOverlay>,
    )
    expect(overlay().style.height).toBe('')
    expect(overlay().style.bottom).toBe('')
    expect(overlay().className).not.toMatch(/\binset-0\b|\bbottom-0\b|\bh-full\b|\bh-screen\b/)
  })

  it('알림이 하나도 안 그려지면 층에 내용이 없다', () => {
    render(
      <NoticeOverlay>
        <Nothing />
        <Nothing />
      </NoticeOverlay>,
    )
    expect(stack().childElementCount).toBe(0)
    expect(stack().textContent).toBe('')
  })

  it('넘치면 스크롤한다 — 스크롤 노드에 높이 상한과 세로 스크롤', () => {
    render(
      <NoticeOverlay>
        <div role="status">a</div>
      </NoticeOverlay>,
    )
    const viewport = overlay().querySelector<HTMLElement>('[data-radix-scroll-area-viewport]')
    expect(viewport).not.toBeNull()
    expect(viewport?.className).toContain('max-h-[33vh]')
    expect(viewport?.style.overflowY).toBe('scroll')
    expect(viewport?.contains(stack())).toBe(true)
  })

  it('여럿이면 준 차례대로 쌓고, 줄 사이를 1px 틈의 앱 바탕으로 가른다', () => {
    render(
      <NoticeOverlay>
        <div role="alert">first</div>
        <Nothing />
        <div role="status">second</div>
      </NoticeOverlay>,
    )
    expect([...stack().children]).toEqual([screen.getByRole('alert'), screen.getByRole('status')])
    expect(stack().className).toMatch(/\bflex-col\b/)
    expect(stack().className).toMatch(/\bgap-px\b/)
    expect(stack().className).toMatch(/\bbg-background\b/)
  })
})
