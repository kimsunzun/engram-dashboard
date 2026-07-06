// 추론(thinking) 행 — Claude Code VSCode 확장 룩(우리 자체 구현).
//   접힘: "Thought" 한 줄(muted, 13px) + 작은 chevron. 펼치면 얇은 좌측 보더 라인과 함께 추론 텍스트를
//   스크롤 박스(max-h ~200px)로 보여준다.
//
// ★빈 content 케이스(load-bearing)★: opus 계열은 암호화된 thinking 을 내보낸다 — signature 만 오고 평문
//   추론 텍스트는 없다(ADR-0049 근거절). 이때도 thinking 이 "있었다"는 존재는 보여야 하므로, 접힌 "Thought"
//   라벨은 그리되 펼칠 내용이 없으므로 비-인터랙티브로 둔다(chevron 없음, cursor-default, title="내용 비공개").
import { ChevronDown, ChevronRight } from 'lucide-react'
import { useState } from 'react'

import { cn } from '@/lib/utils'

interface ThoughtRowProps {
  /** 추론 평문. 비어 있으면(암호화 thinking) 비-인터랙티브 라벨만 렌더. */
  content?: string
  /** 라이브 스트리밍 중이면 라벨에 은은한 pulse. 기본 라벨은 "Thinking…". */
  streaming?: boolean
  /** 라벨 텍스트(기본 "Thought"). streaming 신호는 "Thinking…" 을 넘긴다. */
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
        title={interactive ? undefined : '내용 비공개'}
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
        <div className="mt-1 border-l border-border pl-3 max-h-[200px] overflow-y-auto text-[13px] text-muted whitespace-pre-wrap break-words">
          {content}
        </div>
      )}
    </div>
  )
}

export default ThoughtRow
