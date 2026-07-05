// RichSlot 라이브 렌더 — StructuredEventAccumulator 의 순서 보존 item 스트림을 그린다.
// (ADR-0045 §52 렌더, 사용자 결정 2026-07-05: 비-텍스트 이벤트 = 칩+클릭 펼침, 턴 경계 = 구분선.)
//
// ★층 분리★: 이 컴포넌트는 StructuredItem[] 만 받는다(누적/구독은 RichSlot). text=Markdown,
//   ToolCall/Usage/Error/Structured=접힌 한 줄 칩(<button>, 클릭 펼침·펼침 상태는 칩별 로컬),
//   MessageDone=수평 구분선. 백엔드측 LLM(§5)이 같은 item 을 다른 표면으로도 소비 가능.

import { useState } from 'react'

import { Markdown } from '../../lab/richslot/MarkdownView'
import type { StructuredItem } from './structuredAccumulator'
import './structuredItems.css'

/** 접힌 한 줄 칩 — 클릭하면 detail(펼침 본문)을 토글한다(펼침 상태는 이 칩 로컬). */
function Chip({
  className,
  label,
  summary,
  detail,
}: {
  className: string
  label: string
  summary: string
  detail: string
}) {
  const [open, setOpen] = useState(false)
  return (
    <>
      <button
        type="button"
        className={`si-chip ${className}${open ? ' si-open' : ''}`}
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <span className="si-caret">{open ? '▾' : '▸'}</span>
        <span className="si-label">{label}</span>
        {summary && <span> {summary}</span>}
      </button>
      {open && <span className="si-detail">{detail}</span>}
    </>
  )
}

/** args_json/json 을 읽기 좋게 pretty-print(파싱 실패 시 원문 그대로). */
function pretty(json: string): string {
  try {
    return JSON.stringify(JSON.parse(json), null, 2)
  } catch {
    return json
  }
}

/** 한 item 렌더. 순서 보존 배열을 위에서 아래로 그대로 그린다. */
function renderItem(item: StructuredItem) {
  // itemId 는 누산기가 할당한 단조 id — 배열 index 대신 stable key 로 사용(FIX-4).
  const k = item.itemId
  switch (item.kind) {
    case 'text':
      return (
        <div key={k} className="si-text">
          <Markdown text={item.text} />
        </div>
      )
    case 'tool':
      return (
        <Chip
          key={k}
          className="si-tool"
          label={`🔧 ${item.name}`}
          summary={item.id ? `#${item.id}` : ''}
          detail={pretty(item.argsJson)}
        />
      )
    case 'usage':
      return (
        <Chip
          key={k}
          className="si-usage"
          label="📊 usage"
          summary={`in ${item.inputTokens} · out ${item.outputTokens}`}
          detail={`input_tokens: ${item.inputTokens}\noutput_tokens: ${item.outputTokens}`}
        />
      )
    case 'error':
      return (
        <Chip
          key={k}
          className="si-error"
          label="⚠ error"
          summary={item.message}
          detail={item.message}
        />
      )
    case 'structured':
      return (
        <Chip
          key={k}
          className="si-structured"
          label={`⋯ ${item.label}`}
          summary=""
          detail={pretty(item.json)}
        />
      )
    case 'separator':
      // ADR-0045 — 턴 경계 구분선.
      return <hr key={k} className="si-separator" />
  }
}

export function StructuredItemStream({ items }: { items: StructuredItem[] }) {
  return <div className="si-stream">{items.map(renderItem)}</div>
}
