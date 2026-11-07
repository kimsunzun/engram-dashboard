// ★FIX-1 regression safety net★: the height cap of the expanded reasoning box (max-h-200px) must sit on the Radix Viewport (= the real scroll node)
//   (not the Root). If it sits only on the Root, the Viewport (height:100%) resolves to auto against an indefinite parent, grows to the content
//   height, and the Root overflow-hidden cuts it so scrolling cannot reach it (the original bug). Real scroll behavior (reaching the
//   last line by wheel/drag) depends on the layout engine so jsdom cannot — cdp measurement (manual check). Here we assert only which DOM
//   node (Viewport vs Root) carries max-h.

import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'

import { ThoughtRow } from './ThoughtRow'

// jsdom does not provide ResizeObserver — the Radix ScrollArea internals refer to it, so a no-op stub.
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
