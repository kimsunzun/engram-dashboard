// 추론(thinking) 행 — Claude Code VSCode 확장 룩(우리 자체 구현).
//
// ★빈 content 케이스(load-bearing)★: opus 계열은 암호화된 thinking 을 내보낸다 — signature 만 오고 평문
//   추론 텍스트는 없다(ADR-0049 근거절). 이때도 thinking 이 "있었다"는 존재는 보여야 하므로, 접힌 라벨은
//   그리되 펼칠 내용이 없으므로 비-인터랙티브로 둔다.
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
        // ★max-h 는 Viewport(실제 스크롤 노드)에 얹는다(Root 아님)★: Radix Viewport 는 overflowY:scroll 을
        //   inline 으로 갖지만 높이 상한이 없으면 콘텐츠 높이만큼 늘어나 스크롤 범위가 0이 된다 — Root 에만
        //   max-h 를 걸면 Viewport(height:100%)가 *비확정* 부모(Root=max-height only)에 대해 auto 로 풀려
        //   여전히 콘텐츠 높이로 자라고, Root 의 overflow-hidden 이 200px 에서 잘라 스크롤로 닿지 못한다(회귀).
        //   Viewport 에 직접 max-height 를 걸어야 overflowY:scroll 과 맞물려 진짜 스크롤 컨테이너가 된다
        //   (~100줄 추론 → 박스 ≤200px 유지 + 마지막 줄까지 휠·드래그 스크롤 도달). border/padding 은 시각
        //   프레임이라 Root 에 둔다.
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
