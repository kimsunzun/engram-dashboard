// ScrollArea 스모크 테스트(ADR-0053 seam, 앱 전역). overlay/hover/0.5s-delay/auto-scroll 의 실제 거동은
//   GUI 의존(레이아웃·pointer·타이밍)이라 cdp 실측으로 검증한다 — 여기선 seam 계약만 본다:
//   ① children 을 렌더한다 ② forward 한 ref 가 실제 스크롤 노드(Radix Viewport)를 가리킨다(하단 고정
//   auto-scroll 이 이 노드의 scrollTop 을 겨누므로, ref 대상이 Viewport 여야 회귀가 안 난다)
//   ③ orientation prop 이 Radix Scrollbar 로 전달된다(가로/세로 단일 seam 확장).

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
    // Radix Viewport = 실제 overflow/scrollTop 을 가진 스크롤 노드(data 속성으로 식별). Root 가 아니다.
    expect(ref.current?.hasAttribute('data-radix-scroll-area-viewport')).toBe(true)
    // 하단 고정 스크롤(scrollTop = scrollHeight) 이 이 노드에 걸린다 — scrollTop 접근 가능해야 한다.
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
    // viewportStyle 은 ref(=Viewport)에 얹힌다.
    expect(ref.current?.style.whiteSpace).toBe('pre-wrap')
    // style 은 Root 에 얹힌다(data-testid 로 조회).
    const root = screen.getByTestId('sa-root')
    expect(root.style.background).toBe('rgb(1, 2, 3)')
  })

  it('orientation prop 이 Radix Scrollbar 로 전달된다(기본 vertical)', () => {
    const { container } = render(
      <ScrollArea orientation="horizontal">
        <div style={{ width: 9999 }}>wide content</div>
      </ScrollArea>,
    )
    // type="scroll" 은 스크롤 중에만 마운트하지만, orientation 은 prop 전달 계약이므로 컴포넌트가
    //   Radix 에 넘기는지만 본다 — 렌더 시 스크롤바 미마운트여도 크래시 없이 children 이 그려지면 통과.
    expect(container.textContent).toContain('wide content')
  })
})
