//! ScrollArea — 앱 전역 오버레이 스크롤 영역 seam (ADR-0053).
//!
//! ★역할★: Radix ScrollArea(@radix-ui/react-scroll-area) primitive 를 얇게 감싼 **교체점**. 프론트의
//!   모든 스크롤 표면(채팅·트리·프리셋·팝업 목록·DOM 관측기 등)은 raw `overflow:auto` 대신 이 seam 을
//!   지난다 — 스크롤바 룩/거동을 한 곳(이 컴포넌트 + theme.css --scrollbar-* 토큰)에서 바꾸면 전 화면에
//!   전파된다(단일 출처). 컴포넌트·스토어는 이 seam 에만 의존하고 Radix Root/Viewport 를 직접 다루지
//!   않는다(§5 손발/두뇌 분리 — 스크롤바 구현을 나중에 통째로 갈아끼워도 소비자 코드는 불변). 직접
//!   Radix Root 노출 금지(ADR-0053 결정).
//!
//!   ★예외 = xterm★: TerminalSlot 의 xterm 은 자체 `.xterm-viewport` 스크롤바를 소유해 이 React 컴포넌트로
//!   감쌀 수 없다 → 대신 index.css 의 전역 규칙이 같은 --scrollbar-* 토큰으로 스타일해 시각 통일한다
//!   (컴포넌트 vs CSS 두 경로가 같은 토큰을 참조 = 단일 출처 유지).
//!
//! ★불변식(변경 금지 — 근거 ADR-0053)★:
//!   - TRUE overlay: 네이티브 스크롤바 숨김(Radix viewport 가 `::-webkit-scrollbar` 제거) + custom
//!     scrollbar 가 콘텐츠 위에 absolute 로 떠 gutter 0(콘텐츠 폭 불변).
//!   - 스크롤 중에만 등장(type="scroll") · 스크롤 멈춤 후 scrollHideDelay(500ms) 뒤 숨김. hover 에는
//!     반응하지 않는다(ADR-0053 사용자 결정 — 전체 화면 hover 로 스크롤바가 너무 일찍 보이는 문제 해소).
//!   - ★스크롤 대상 = ScrollArea.Viewport(Root 아님)★: 실제 overflow/scrollTop 은 viewport DOM 노드다.
//!     RichSlot/DomSlot 의 하단 고정 auto-scroll(scrollTop = scrollHeight)이 이 노드를 겨눠야 하므로 ref 를
//!     Viewport 로 forward 한다. Root 로 겨누면 스크롤이 동작하지 않는다(회귀 주의).

import { forwardRef, type ReactNode } from 'react'
import * as RadixScrollArea from '@radix-ui/react-scroll-area'

import { cn } from '@/lib/utils'
import './scroll-area.css'

export interface ScrollAreaProps {
  children: ReactNode
  /** Root(스크롤 영역 바깥 컨테이너)에 얹을 클래스 — flex/크기 배치용(예: min-h-0 flex-1). */
  className?: string
  /** Viewport(실제 스크롤 엘리먼트)에 얹을 클래스 — 콘텐츠 컨테이너 스타일. */
  viewportClassName?: string
  /** Root(바깥 컨테이너)에 얹을 인라인 스타일 — 변수-only 스타일 소비자(트리·팝업 등)가 크기/배경을 준다. */
  style?: React.CSSProperties
  /** Viewport(실제 스크롤 엘리먼트)에 얹을 인라인 스타일 — <pre> 등 콘텐츠 스타일(whiteSpace 등). */
  viewportStyle?: React.CSSProperties
  /** 스크롤 방향(기본 vertical). horizontal 목록/코드뷰가 나오면 여기로 확장(단일 seam 유지). */
  orientation?: 'vertical' | 'horizontal'
  /** cdp eval·테스트에서 마운트 여부·대상 확인용 마커(소비자가 넘긴 data-* 을 Root 에 얹는다). */
  'data-testid'?: string
}

// ADR-0053: Radix ScrollArea 위의 얇은 seam. ref 는 Viewport(실제 스크롤 노드)로 forward 한다 —
//   RichSlot/DomSlot 이 이 ref 로 하단 고정 스크롤을 건다(위 헤더 불변식).
export const ScrollArea = forwardRef<HTMLDivElement, ScrollAreaProps>(function ScrollArea(
  { children, className, viewportClassName, style, viewportStyle, orientation = 'vertical', ...rest },
  viewportRef,
) {
  return (
    <RadixScrollArea.Root
      type="scroll"
      // ★scrollHideDelay★: 스크롤 멈춤 뒤 스크롤바를 얼마 있다 감출지(ms). 500ms 린거(lingering) — 스크롤을
      //   끝낸 직후 바로 사라지면 어색하므로 잠깐 남긴다(hover 표시 지연과 무관 — 구 CSS animation-delay 제거됨).
      scrollHideDelay={500}
      className={cn('relative overflow-hidden', className)}
      style={style}
      data-testid={rest['data-testid']}
    >
      <RadixScrollArea.Viewport
        ref={viewportRef}
        // ★h-full w-full 필수★: Radix Viewport 는 부모(Root) 높이를 받아야 내부 콘텐츠가 넘칠 때 스크롤이
        //   생긴다. Root 는 flex 자식으로 높이를 받고(min-h-0 flex-1) Viewport 가 그걸 채운다.
        className={cn('h-full w-full', viewportClassName)}
        style={viewportStyle}
      >
        {children}
      </RadixScrollArea.Viewport>
      <RadixScrollArea.Scrollbar
        orientation={orientation}
        // 스타일은 CSS 클래스로(scroll-area.css). type="scroll" 이므로 Radix 가 스크롤 중에만 마운트하고
        //   scrollHideDelay 뒤 언마운트한다 — hover 기반 show-delay animation 는 제거됨.
        className="engram-scrollbar"
      >
        <RadixScrollArea.Thumb className="engram-scrollbar-thumb" />
      </RadixScrollArea.Scrollbar>
      <RadixScrollArea.Corner />
    </RadixScrollArea.Root>
  )
})

export default ScrollArea
