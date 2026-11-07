// ★Connector offset★: --chat-rail-line-offset is coupled to the outer top-padding (--chat-rail-row-pt)
//   (the existing implicit coupling top-[-12px]↔pt-3 made explicit with a variable).

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
    // Dot color = the extension look benchmark.
    const dotColor = tone === 'tool' ? 'bg-green-500' : tone === 'error' ? 'bg-red-500' : 'bg-muted'
    const pos = runPos ?? 'mid'
    const lineStyle: Record<string, string> =
      pos === 'top'
        ? { top: 'var(--chat-rail-dot-top)', bottom: '0' } // The stub above removed
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
        {/* aria-hidden is put only on the span (pure decoration) — putting it on the gutter div collides with the separator spacer
            (div[aria-hidden]) selector. */}
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
        {/* min-w-0 — prevents long token/wrap-anywhere overflow. */}
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
