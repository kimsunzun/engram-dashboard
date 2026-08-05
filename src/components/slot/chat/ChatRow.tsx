// ★연결선 오프셋★: --chat-rail-line-offset 은 outer top-padding(--chat-rail-row-pt) 과 커플링돼 있다
//   (기존 top-[-12px]↔pt-3 암묵 커플링을 변수로 명시화).

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
    // 점 색 = 확장 룩 벤치마크.
    const dotColor = tone === 'tool' ? 'bg-green-500' : tone === 'error' ? 'bg-red-500' : 'bg-muted'
    const pos = runPos ?? 'mid'
    const lineStyle: Record<string, string> =
      pos === 'top'
        ? { top: 'var(--chat-rail-dot-top)', bottom: '0' } // 위 stub 제거
        : pos === 'bottom'
          ? {
              top: 'var(--chat-rail-line-offset)',
              bottom: 'calc(100% - var(--chat-rail-dot-top))',
            }
          : { top: 'var(--chat-rail-line-offset)', bottom: '0' }
    return (
      <div
        className="relative flex px-4"
        style={{ paddingTop: 'var(--chat-rail-row-pt)' }}
      >
        {/* aria-hidden 은 span 에만 얹는다(순수 장식) — gutter div 에 얹으면 separator 스페이서
            (div[aria-hidden]) 셀렉터와 충돌한다. */}
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
        {/* min-w-0 — 긴 토큰/wrap-anywhere 오버플로 방지. */}
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
