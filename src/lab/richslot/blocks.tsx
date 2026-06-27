// RichSlot 렌더층 — ContentBlock 타입별 렌더러 (React).
//
// 스파이크 골격: 의존성 0(streamdown/Prism 아직 X). 코드블록 강조·diff·리버트 버튼은
// 후속 단계 — 지금은 블록을 "구분해서 보여주는" 것까지만(타입별 박스 + 최소 서식).
// 살은 여기에 붙인다(파싱층 건드릴 필요 없음).

import type { ContentBlock } from './types'

/** ContentBlock 1개 → React 엘리먼트. switch 가 타입별 렌더러를 배정한다(미래 확장점). */
export function renderBlock(block: ContentBlock, key: number) {
  switch (block.type) {
    case 'text':
      return <TextBlock key={key} text={block.text} />
    case 'thinking':
      return <ThinkingBlock key={key} thinking={block.thinking} />
    case 'tool_use':
      return <ToolUseBlock key={key} name={block.name} input={block.input} />
    case 'tool_result':
      return <ToolResultBlock key={key} content={block.content} isError={block.is_error} />
  }
}

function TextBlock({ text }: { text: string }) {
  // TODO(후속): streamdown 으로 Markdown 렌더(코드블록 Prism 강조).
  return <div className="rs-block rs-text">{text}</div>
}

function ThinkingBlock({ thinking }: { thinking: string }) {
  // 접기 기본값 — thinking 은 길고 보조적이라 collapsed(details/summary 네이티브).
  return (
    <details className="rs-block rs-thinking">
      <summary>💭 thinking</summary>
      <pre>{thinking}</pre>
    </details>
  )
}

function ToolUseBlock({ name, input }: { name: string; input: Record<string, unknown> }) {
  // TODO(후속): name==='Edit'|'Write' 면 input.file_path 로 리버트 버튼 + diff 뷰.
  return (
    <div className="rs-block rs-tool-use">
      <span className="rs-tool-name">🔧 {name}</span>
      <pre>{JSON.stringify(input, null, 2)}</pre>
    </div>
  )
}

function ToolResultBlock({
  content,
  isError,
}: {
  content: string | unknown[]
  isError?: boolean | null
}) {
  const text = typeof content === 'string' ? content : JSON.stringify(content, null, 2)
  return (
    <details className={`rs-block rs-tool-result${isError ? ' rs-error' : ''}`}>
      <summary>{isError ? '❌ result (error)' : '✅ result'}</summary>
      <pre>{text}</pre>
    </details>
  )
}
