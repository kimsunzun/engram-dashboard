// ★FIX-1 회귀 안전망★: 펼친 추론 박스의 높이 상한(max-h-200px)이 Radix Viewport(=실제 스크롤 노드)에
//   얹혀야 한다(Root 아님). Root 에만 얹으면 Viewport(height:100%)가 비확정 부모에 대해 auto 로 풀려 콘텐츠
//   높이로 자라고 Root overflow-hidden 이 잘라 스크롤로 닿지 못한다(원 버그). 실제 스크롤 거동(휠/드래그로
//   마지막 줄 도달)은 레이아웃 엔진 의존이라 jsdom 불가 — cdp 실측(수동 검증). 여기선 max-h 가 붙는 DOM
//   노드(Viewport vs Root)만 단언한다.

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import { ThoughtRow } from './ThoughtRow'

// jsdom 은 ResizeObserver 를 제공하지 않는다 — Radix ScrollArea 내부가 참조하므로 no-op stub.
globalThis.ResizeObserver ||= class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver

afterEach(() => cleanup())

describe('ThoughtRow', () => {
  it('content 있으면 인터랙티브 — 클릭으로 펼치면 추론 텍스트가 보인다', () => {
    render(<ThoughtRow content="line-a\nline-b" />)
    expect(screen.queryByText(/line-a/)).toBeNull()
    fireEvent.click(screen.getByRole('button'))
    expect(screen.getByText(/line-a/)).toBeTruthy()
  })

  it('빈 content(암호화 thinking) → 비-인터랙티브 라벨만(펼칠 내용 없음)', () => {
    render(<ThoughtRow content="" />)
    const btn = screen.getByRole('button')
    fireEvent.click(btn)
    expect(document.querySelector('[data-radix-scroll-area-viewport]')).toBeNull()
  })

  it('★FIX-1★ 펼친 박스의 max-h 상한이 Viewport(스크롤 노드)에 붙는다 — Root 아님', () => {
    render(<ThoughtRow content="x" />)
    fireEvent.click(screen.getByRole('button'))
    const viewport = document.querySelector('[data-radix-scroll-area-viewport]') as HTMLElement
    expect(viewport).toBeTruthy()
    expect(viewport.className).toContain('max-h-[200px]')
    const root = viewport.parentElement as HTMLElement
    expect(root.className).not.toContain('max-h-[200px]')
  })
})
