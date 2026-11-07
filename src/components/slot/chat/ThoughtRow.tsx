// Reasoning (thinking) row — Claude Code VSCode extension look (our own implementation).
//
// ★Empty content case (load-bearing)★: the opus family emits encrypted thinking — only a signature comes and there is no plain-text
//   reasoning text (the basis section of ADR-0049). Even then the existence of thinking "having happened" must show, so the collapsed label is
//   drawn but, with nothing to expand, kept non-interactive.
import { ChevronDown, ChevronRight } from 'lucide-react'
import { useState } from 'react'

import { cn } from '@/lib/utils'
import { ScrollArea } from '@/components/ui/scroll-area'
import { t } from '../../../i18n'

interface ThoughtRowProps {
  content?: string
  streaming?: boolean
  label?: string
}

export function ThoughtRow({ content, streaming = false, label = 'Thought' }: ThoughtRowProps) {
  const [expanded, setExpanded] = useState(false)
  const hasContent = !!content && content.trim().length > 0
  const interactive = hasContent

  return (
    <div className="my-1">
      <button
        type="button"
        onClick={interactive ? () => setExpanded((o) => !o) : undefined}
        aria-expanded={interactive ? expanded : undefined}
        title={interactive ? undefined : t('common.contentPrivate')}
        className={cn(
          'flex items-center gap-1 text-[13px] text-muted select-none',
          interactive ? 'cursor-pointer' : 'cursor-default',
        )}
      >
        {interactive &&
          (expanded ? (
            <ChevronDown className="size-3 flex-none" />
          ) : (
            <ChevronRight className="size-3 flex-none" />
          ))}
        <span className={cn(streaming && 'animate-pulse')}>{label}</span>
      </button>

      {interactive && expanded && (
        // ★max-h goes on the Viewport (the real scroll node), not the Root★: the Radix Viewport has overflowY:scroll
        //   inline but without a height cap it stretches to the content height and the scroll range becomes 0 — if max-h is put only on the Root,
        //   the Viewport (height:100%) resolves to auto against an *indefinite* parent (Root = max-height only) and
        //   still grows to the content height, and the Root's overflow-hidden cuts at 200px so scrolling cannot reach it (regression).
        //   Only with max-height directly on the Viewport does it mesh with overflowY:scroll and become a real scroll container
        //   (~100 lines of reasoning → box stays ≤200px + the last line is reachable by wheel·drag scroll). border/padding is a visual
        //   frame so it goes on the Root.
        <ScrollArea
          className="mt-1 border-l border-border pl-3"
          viewportClassName="max-h-[200px]"
        >
          <div className="text-[13px] text-muted whitespace-pre-wrap break-words">{content}</div>
        </ScrollArea>
      )}
    </div>
  )
}

export default ThoughtRow
