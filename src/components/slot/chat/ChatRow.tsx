// ChatRow — 채팅 메시지 행의 바깥 컨테이너 leaf(ADR-0051 rail / ADR-0053 구조 분할로 StructuredTextView
//   에서 분리). 순수 컴포넌트(props in, DOM out) — dispatch 는 StructuredTextView 소관.
//
// 간격·폰트는 CSS 변수(--chat-*)로 참조한다(ADR-0051 — LLM 제어 + localStorage 영속. 값 권위 =
//   chatStyleStore, 여기선 var() 소비만). StructuredTextView 컨테이너가 이 행들을 담는다.
//
// rail 모드(기본 false — user 버블·하위호환): assistant-side 행에 좌측 thread 구조를 준다.
//   flex 행 = [고정폭 gutter + 점 마커] + [콘텐츠 flex-1 min-w-0]. 콘텐츠 컬럼은 min-w-0 라 긴 토큰/
//   wrap-anywhere 가 넘치지 않는다.
//
// ★연결선 clean-ends(ADR-0051)★: runPos 로 선 geometry 를 분기한다 — top=dot 에서 아래로만,
//   mid=관통(위 offset ~ 아래), bottom=위 offset ~ dot 에서 멈춤, single=선 없음. 오프셋은 outer
//   top-padding(--chat-rail-row-pt) 과 커플링된 --chat-rail-line-offset 을 참조(기존 top-[-12px]↔pt-3
//   암묵 커플링을 변수로 명시화). runPos 미지정(하위호환·streaming tail) 시 관통(mid)로 폴백.

import type { ReactNode } from 'react'

import { cn } from '@/lib/utils'
import type { RailRunPosition } from './railPositions'

export function ChatRow({
  children,
  rail = false,
  tone = 'default',
  runPos,
}: {
  children: ReactNode
  rail?: boolean
  tone?: 'default' | 'tool' | 'error'
  runPos?: RailRunPosition
}) {
  if (rail) {
    // 점 색 = 행 종류 신호(확장 룩 벤치마크): tool(실행)=초록 · error=red · 그 외(추론/본문)=muted.
    const dotColor = tone === 'tool' ? 'bg-green-500' : tone === 'error' ? 'bg-red-500' : 'bg-muted'
    const pos = runPos ?? 'mid'
    // 연결선 top/bottom — position 별 clean-ends. CSS 변수 참조(inline style, var() 로 런타임 반영).
    //   dot 위치 = --chat-rail-dot-top, 위 이어짐 오프셋 = --chat-rail-line-offset(보통 음수).
    const lineStyle: Record<string, string> =
      pos === 'top'
        ? { top: 'var(--chat-rail-dot-top)', bottom: '0' } // 최상단 dot 아래로만(위 stub 제거)
        : pos === 'bottom'
          ? {
              top: 'var(--chat-rail-line-offset)',
              bottom: 'calc(100% - var(--chat-rail-dot-top))', // 위에서 내려와 이 dot 에서 멈춤
            }
          : { top: 'var(--chat-rail-line-offset)', bottom: '0' } // mid — 관통
    return (
      <div
        className="relative flex px-4"
        style={{ paddingTop: 'var(--chat-rail-row-pt)' }}
      >
        {/* gutter — 세로 thread 선 + 점 마커. 둘 다 span 에만 aria-hidden(순수 장식) — gutter div 에 얹으면
            separator 스페이서(div[aria-hidden]) 셀렉터와 충돌한다. 점은 콘텐츠 첫 줄 근처(--chat-rail-dot-top
            center)에 절대배치해 선 위에 올린다. single 은 선을 아예 그리지 않는다(고립 dot). */}
        <div
          className="relative flex-none"
          style={{ width: 'var(--chat-rail-gutter)' }}
        >
          {pos !== 'single' && (
            <span
              aria-hidden
              className="absolute left-1/2 w-px -translate-x-1/2 bg-border"
              style={lineStyle}
            />
          )}
          <span
            aria-hidden
            className={cn(
              'absolute left-1/2 size-1.5 -translate-x-1/2 -translate-y-1/2 rounded-full',
              dotColor,
            )}
            style={{ top: 'var(--chat-rail-dot-top)' }}
          />
        </div>
        {/* 콘텐츠 컬럼 — flex-1 min-w-0 로 긴 토큰/wrap-anywhere 오버플로 방지. */}
        <div className="min-w-0 flex-1">{children}</div>
      </div>
    )
  }
  return (
    <div className="relative px-4" style={{ paddingTop: 'var(--chat-plain-row-pt)' }}>
      {children}
    </div>
  )
}

export default ChatRow
