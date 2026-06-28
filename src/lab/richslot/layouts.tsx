// RichSlot 레이아웃 후보 4종 — 같은 RichMessage[] 를 서로 다른 스타일로 렌더(탭 비교용).
// 시장조사(2026-06-27, Claude+Codex 교차) 결과 좁은 슬롯 적합 Top4: 타임라인/풀너비스트림/터미널로그/블록카드.
// 스파이크 골격 — Markdown(streamdown)·코드강조(Prism)·리버트버튼은 후속. 지금은 "레이아웃 형태" 비교가 목적.
//
// ★층 분리★: 입력은 RichMessage[](파싱층 산출)만. 각 레이아웃은 블록 타입별 표현 방식만 다르다.

import type { ContentBlock, RichMessage } from './types'
import { Markdown } from './MarkdownView'
import { CatalogView } from './CatalogView'
import { useRenderSettings, looksLikeDiff } from './renderSettings'
import { LazyMonacoDiff } from './MonacoLazy'
import './layouts.css'

// ── 공통 헬퍼 ──────────────────────────────────────────────────────────────
/** role 태그 붙여 블록을 순서대로 평탄화(레이아웃은 대부분 평탄 스트림으로 다룬다). */
function flatten(messages: RichMessage[]): { role: 'assistant' | 'user'; block: ContentBlock }[] {
  return messages.flatMap((m) => m.blocks.map((block) => ({ role: m.role, block })))
}
/** tool_result.content(string | array) → 표시용 텍스트. */
function asText(content: string | unknown[]): string {
  return typeof content === 'string' ? content : JSON.stringify(content, null, 2)
}
/** tool_use.input → 한 줄 요약(터미널/스트림용). */
function fmtInput(input: Record<string, unknown>): string {
  const parts = Object.entries(input).map(([k, v]) => `${k}=${typeof v === 'string' ? v : JSON.stringify(v)}`)
  return parts.join(' ')
}

// ── ① 타임라인형 — 좌측 컬러 바 + 행마다 아이콘 + 접기 ──────────────────────
export function TimelineLayout({ messages }: { messages: RichMessage[] }) {
  return (
    <div className="lay-timeline">
      {flatten(messages).map(({ block }, i) => {
        switch (block.type) {
          case 'text':
            return <div key={i} className="tl-row tl-text">{block.text}</div>
          case 'thinking':
            return (
              <details key={i} className="tl-row tl-thinking">
                <summary>💭 thinking</summary>
                <pre>{block.thinking}</pre>
              </details>
            )
          case 'tool_use':
            return (
              <details key={i} className="tl-row tl-tool">
                <summary>🔧 {block.name}</summary>
                <pre>{JSON.stringify(block.input, null, 2)}</pre>
              </details>
            )
          case 'tool_result':
            return (
              <details key={i} className={`tl-row tl-result${block.is_error ? ' lay-error' : ''}`}>
                <summary>{block.is_error ? '❌ result' : '✅ result'}</summary>
                <pre>{asText(block.content)}</pre>
              </details>
            )
        }
      })}
    </div>
  )
}

// ── ② 풀너비 단일 스트림형 — 구분 최소, 흐르듯 ───────────────────────────────
export function StreamLayout({ messages }: { messages: RichMessage[] }) {
  return (
    <div className="lay-stream">
      {flatten(messages).map(({ block }, i) => {
        switch (block.type) {
          case 'text':
            return <p key={i} className="st-text">{block.text}</p>
          case 'thinking':
            return (
              <details key={i} className="st-thinking">
                <summary>thinking</summary>
                <pre>{block.thinking}</pre>
              </details>
            )
          case 'tool_use':
            return <div key={i} className="st-tool">{block.name}({fmtInput(block.input)})</div>
          case 'tool_result':
            return <div key={i} className={`st-result${block.is_error ? ' lay-error' : ''}`}>→ {asText(block.content).slice(0, 200)}</div>
        }
      })}
    </div>
  )
}

// ── ③ 터미널 로그형 — 모노스페이스, prefix, 고밀도 ──────────────────────────
export function TerminalLogLayout({ messages }: { messages: RichMessage[] }) {
  return (
    <div className="lay-tlog">
      {flatten(messages).map(({ block }, i) => {
        switch (block.type) {
          case 'text':
            return <div key={i} className="tlog-text">● {block.text}</div>
          case 'thinking':
            return (
              <details key={i} className="tlog-thinking">
                <summary># thinking</summary>
                <pre>{block.thinking}</pre>
              </details>
            )
          case 'tool_use':
            return <div key={i} className="tlog-cmd">$ {block.name} {fmtInput(block.input)}</div>
          case 'tool_result':
            return <pre key={i} className={`tlog-out${block.is_error ? ' lay-error' : ''}`}>{asText(block.content)}</pre>
        }
      })}
    </div>
  )
}

// ── ④ 블록 카드형 — tool_use↔tool_result 를 id 로 묶어 한 카드 ───────────────
export function CardLayout({ messages }: { messages: RichMessage[] }) {
  const rows = flatten(messages)
  // tool_result 를 tool_use_id 로 인덱싱 → 해당 tool_use 카드에 흡수.
  const resultById = new Map<string, Extract<ContentBlock, { type: 'tool_result' }>>()
  for (const { block } of rows) {
    if (block.type === 'tool_result') resultById.set(block.tool_use_id, block)
  }
  return (
    <div className="lay-card">
      {rows.map(({ block }, i) => {
        if (block.type === 'tool_result') return null // 카드에 흡수됨
        if (block.type === 'tool_use') {
          const res = resultById.get(block.id)
          return (
            <div key={i} className={`cd-tool${res?.is_error ? ' lay-error' : ''}`}>
              <div className="cd-head">🔧 {block.name}</div>
              <pre className="cd-input">{JSON.stringify(block.input, null, 2)}</pre>
              {res && <pre className="cd-result">{asText(res.content)}</pre>}
            </div>
          )
        }
        if (block.type === 'thinking') {
          return (
            <details key={i} className="cd-thinking">
              <summary>💭 thinking</summary>
              <pre>{block.thinking}</pre>
            </details>
          )
        }
        return <div key={i} className="cd-text">{block.text}</div>
      })}
    </div>
  )
}

// ── ⑤ 대화형(chat) — 사용자 선택 레이아웃 ──────────────────────────────────
// 규칙: text=Markdown(가독 본문, prominent) · thinking/tool(+result)=흐릿한 1줄 접이행.
// tool_use↔tool_result 는 id 로 페어링해 한 <details> 안에 input+result 를 같이 편다.
//
// ★스트리밍 라이브 규칙(실연결 시)★: 현재 스트리밍 중인 블록은 펼쳐진 채로 보이고,
// 턴이 다음 블록으로 넘어가면 이 1줄 형태로 자동 접힌다(이후에도 클릭으로 다시 펼침).
// fixture 는 완료 세션이라 전부 접힌 상태로 렌더된다 — 정상(아직 라이브 미연결).
export function ChatLayout({ messages }: { messages: RichMessage[] }) {
  const { diffRender } = useRenderSettings()
  const rows = flatten(messages)
  // tool_result 를 id 로 인덱싱 → 짝 tool_use 접이행에 흡수(리뷰/리버트 근거).
  const resultById = new Map<string, Extract<ContentBlock, { type: 'tool_result' }>>()
  for (const { block } of rows) {
    if (block.type === 'tool_result') resultById.set(block.tool_use_id, block)
  }
  return (
    <div className="lay-chat">
      {rows.map(({ role, block }, i) => {
        switch (block.type) {
          case 'text':
            // 본문 = Markdown(prominent). user 텍스트(질문)는 살짝 구분.
            return (
              <div key={i} className={`chat-text${role === 'user' ? ' chat-user' : ''}`}>
                <Markdown text={block.text} />
              </div>
            )
          case 'thinking':
            return (
              <details key={i} className="chat-aside chat-thinking">
                <summary>💭 thinking</summary>
                <pre>{block.thinking}</pre>
              </details>
            )
          case 'tool_use': {
            const res = resultById.get(block.id)
            const resText = res ? asText(res.content) : ''
            // diffRender=monaco + 결과 본문이 diff(예: Bash `git diff` 결과) → Monaco diff.
            // (collapsed 여도 details 자식은 DOM 에 마운트됨 — 토글이 monaco 일 때만 비용 발생.)
            const showMonacoDiff = !!res && diffRender === 'monaco' && looksLikeDiff(resText)
            return (
              <details key={i} className={`chat-aside chat-tool${res?.is_error ? ' lay-error' : ''}`}>
                <summary>
                  🔧 {block.name} <span className="chat-tool-arg">{fmtInput(block.input)}</span>
                </summary>
                <pre className="chat-tool-input">{JSON.stringify(block.input, null, 2)}</pre>
                {res &&
                  (showMonacoDiff ? (
                    <LazyMonacoDiff diff={resText} />
                  ) : (
                    <pre className="chat-tool-result">{resText}</pre>
                  ))}
              </details>
            )
          }
          case 'tool_result':
            return null // 짝 tool_use 접이행에 흡수됨
        }
      })}
    </div>
  )
}

export const LAYOUTS = {
  catalog: { label: '스타일 카탈로그', Comp: CatalogView }, // 스타일 견본(각 1개·라벨) — 모드 차이 보기용
  chat: { label: '대화형', Comp: ChatLayout },
  timeline: { label: '타임라인', Comp: TimelineLayout },
  stream: { label: '풀너비 스트림', Comp: StreamLayout },
  tlog: { label: '터미널 로그', Comp: TerminalLogLayout },
  card: { label: '블록 카드', Comp: CardLayout },
} as const

export type LayoutKey = keyof typeof LAYOUTS
