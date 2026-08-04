// ScrollArea 스모크 테스트(ADR-0053 seam, 앱 전역). overlay/hover/0.5s-delay/auto-scroll 의 실제 거동은
//   GUI 의존(레이아웃·pointer·타이밍)이라 cdp 실측으로 검증한다 — 여기선 seam 계약만 본다.

import { cleanup, render, screen } from '@testing-library/react'
import { createRef } from 'react'
import { afterEach, describe, expect, it } from 'vitest'

import { ScrollArea } from './scroll-area'

// jsdom 은 ResizeObserver 를 제공하지 않는다 — Radix ScrollArea 내부가 참조하므로 no-op stub 을 깐다.
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
    // Radix Viewport = 실제 overflow/scrollTop 을 가진 스크롤 노드. Root 가 아니다.
    expect(ref.current?.hasAttribute('data-radix-scroll-area-viewport')).toBe(true)
    // 하단 고정 스크롤(scrollTop = scrollHeight) 이 이 노드에 걸린다.
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
    // type="scroll" 은 스크롤 중에만 마운트하므로 스크롤바 자체는 렌더되지 않는다 — 컴포넌트가
    //   orientation 을 Radix 에 넘기고 크래시 없이 그리는지만 본다.
    expect(container.textContent).toContain('wide content')
  })
})
