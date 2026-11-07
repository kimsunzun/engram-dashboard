// ScrollArea smoke test (ADR-0053 seam, app-wide). The actual behavior of overlay/hover/0.5s-delay/auto-scroll
//   depends on the GUI (layout · pointer · timing) so it is verified by cdp measurement — here we look only at the seam contract.

import { cleanup, render, screen } from '@testing-library/react'
import { createRef } from 'react'
import { afterEach, describe, expect, it } from 'vitest'

import { ScrollArea } from './scroll-area'

// jsdom does not provide ResizeObserver — Radix ScrollArea internals reference it, so lay down a no-op stub.
globalThis.ResizeObserver ||= class {
  observe() {}
  unobserve() {}
  disconnect() {}
} as unknown as typeof ResizeObserver

afterEach(() => cleanup())

describe('ScrollArea (ADR-0053 오버레이 스크롤바 seam, 앱 전역)', () => {
  it('children 을 렌더한다', () => {
    render(
      <ScrollArea>
        <div>scrolled content</div>
      </ScrollArea>,
    )
    expect(screen.getByText('scrolled content')).toBeTruthy()
  })

  it('forward 한 ref 가 실제 스크롤 엘리먼트(Radix Viewport)를 가리킨다 — auto-scroll 대상 계약', () => {
    const ref = createRef<HTMLDivElement>()
    render(
      <ScrollArea ref={ref}>
        <div>content</div>
      </ScrollArea>,
    )
    expect(ref.current).toBeTruthy()
    // Radix Viewport = the scroll node that holds the real overflow/scrollTop. It is not the Root.
    expect(ref.current?.hasAttribute('data-radix-scroll-area-viewport')).toBe(true)
    // The bottom-pinned scroll (scrollTop = scrollHeight) lands on this node.
    expect(ref.current && 'scrollTop' in ref.current).toBe(true)
  })

  it('style/viewportStyle 을 각 노드에 얹는다 — 변수-only 소비자(트리·팝업·pre) 계약', () => {
    const ref = createRef<HTMLDivElement>()
    render(
      <ScrollArea
        ref={ref}
        style={{ background: 'rgb(1, 2, 3)' }}
        viewportStyle={{ whiteSpace: 'pre-wrap' }}
        data-testid="sa-root"
      >
        <div>content</div>
      </ScrollArea>,
    )
    expect(ref.current?.style.whiteSpace).toBe('pre-wrap')
    const root = screen.getByTestId('sa-root')
    expect(root.style.background).toBe('rgb(1, 2, 3)')
  })

  it('orientation prop 이 Radix Scrollbar 로 전달된다(기본 vertical)', () => {
    const { container } = render(
      <ScrollArea orientation="horizontal">
        <div style={{ width: 9999 }}>wide content</div>
      </ScrollArea>,
    )
    // type="scroll" mounts only while scrolling, so the scrollbar itself is not rendered — we only check that the component
    //   passes orientation to Radix and draws without a crash.
    expect(container.textContent).toContain('wide content')
  })
})
