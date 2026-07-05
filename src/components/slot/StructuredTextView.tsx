// ADR-0048: 구조화 채팅 렌더 dispatch — Cline(Apache-2.0)의 채팅 leaf 컴포넌트를 그대로 이식(verbatim port)해
//   우리 데이터 모델(StructuredItem 스트림)을 그 컴포넌트들에 먹인다. 앞선 ADR-0047 자체구현(첫 시안)이
//   사용자에게 근사치로 반려돼, 실제 Cline JSX+Tailwind 를 옮겨 온 것으로 교체한다.
//   이식 컴포넌트: ./cline/{MarkdownRow,MarkdownBlock,ThinkingRow,CopyButton}
//   + ui/button. 라이선스/귀속은 각 파일 헤더 + LICENSES/cline-Apache-2.0.txt.
//
// ★이 파일의 책임★: items 스트림을 종류별로 위 이식 컴포넌트로 dispatch 하는 **순수 렌더**(구독/누적은
//   RichSlot 소관). props = { items, streaming } 만 받는다.
//
// ★레이아웃(ADR-0048 — 층 분리)★: 좌측 점선 타임라인 레일 스캐폴드(Row)는 당분간 유지한다. 이번 과업의
//   핵심은 *콘텐츠 렌더러*를 Cline 실물로 교체하는 것이지 레이아웃 재설계가 아니다(레이아웃은 이후 튜닝).
//
// ★안전 파서 헬퍼(pretty/extractText/contentToText/parseToolResult/buildToolResultMap/shortArgs)★는
//   ADR-0047 시안에서 그대로 승계한다 — 우리 데이터-어댑터 로직이며 **절대 throw 하지 않는다**(bad json 폴백).

import { useState, type ReactNode } from 'react'
import { AlertTriangle } from 'lucide-react'

import { cn } from '@/lib/utils'
import type { StructuredItem } from './structuredAccumulator'
// ADR-0048: Cline 이식 leaf 들(verbatim/adapt). 상세·귀속은 각 파일 헤더 참조.
//   ★MarkdownBlock(전체 마크다운) 은 assistant text 에만 쓴다(MarkdownRow 경유). 도구 IN/OUT·탈출구 json 은
//   신뢰할 수 없는 텍스트라 마크다운 파싱을 태우지 않고 InertCode(리터럴 <pre>)로만 그린다(FIX 2 — 아래 주석).
import { MarkdownRow } from './cline/MarkdownRow'
import { ThinkingRow } from './cline/ThinkingRow'

// ── 안전 파서 헬퍼(절대 throw 금지 — bad json 폴백) ────────────────────────────────

/** args_json/json 을 읽기 좋게 pretty-print(파싱 실패 시 원문 그대로). 절대 throw 하지 않는다. */
function pretty(json: string): string {
  try {
    return JSON.stringify(JSON.parse(json), null, 2)
  } catch {
    return json
  }
}

/**
 * json 문자열에서 텍스트를 추출한다. 절대 throw 하지 않는다.
 * - 'user': `.text` 우선, 없으면 `.thinking` · 'thinking': `.thinking` 우선, 없으면 `.text`.
 * - 실패/필드 부재 시 raw json 문자열.
 */
function extractText(json: string, mode: 'thinking' | 'user'): string {
  try {
    const parsed: unknown = JSON.parse(json)
    if (typeof parsed === 'string') return parsed
    if (parsed !== null && typeof parsed === 'object') {
      const obj = parsed as Record<string, unknown>
      if (mode === 'user') {
        if (typeof obj['text'] === 'string') return obj['text']
        if (typeof obj['thinking'] === 'string') return obj['thinking']
      } else {
        if (typeof obj['thinking'] === 'string') return obj['thinking']
        if (typeof obj['text'] === 'string') return obj['text']
      }
    }
    return json
  } catch {
    return json
  }
}

/**
 * Anthropic content 블록(문자열 | 블록 배열)에서 표시용 텍스트를 뽑는다. 절대 throw 하지 않는다.
 * content 가 문자열이면 그대로, 배열이면 `type === "text"` 블록의 `.text` 만 이어붙인다.
 */
function contentToText(content: unknown): string {
  if (typeof content === 'string') return content
  if (Array.isArray(content)) {
    const parts: string[] = []
    for (const block of content) {
      if (typeof block === 'string') {
        parts.push(block)
      } else if (block !== null && typeof block === 'object') {
        const b = block as Record<string, unknown>
        if (b['type'] === 'text' && typeof b['text'] === 'string') parts.push(b['text'])
      }
    }
    return parts.join('\n')
  }
  if (content !== null && typeof content === 'object') {
    const b = content as Record<string, unknown>
    if (b['type'] === 'text' && typeof b['text'] === 'string') return b['text']
  }
  return ''
}

/** tool_result 페어(도구 결과 본문 + 에러 여부). tool_use_id 로 도구 호출과 짝짓는다. */
type ToolResult = { content: string; isError: boolean }

/**
 * structured item 의 json 이 tool_result 면 { toolUseId, result } 를, 아니면 null. 절대 throw 하지 않는다.
 */
function parseToolResult(json: string): { toolUseId: string; result: ToolResult } | null {
  try {
    const parsed: unknown = JSON.parse(json)
    if (parsed === null || typeof parsed !== 'object') return null
    const obj = parsed as Record<string, unknown>
    if (obj['type'] !== 'tool_result') return null
    const toolUseId = typeof obj['tool_use_id'] === 'string' ? obj['tool_use_id'] : ''
    if (!toolUseId) return null
    return {
      toolUseId,
      result: { content: contentToText(obj['content']), isError: obj['is_error'] === true },
    }
  } catch {
    return null
  }
}

/**
 * items 를 한 번 훑어 tool_use_id → tool_result 맵을 만든다(pre-scan). 절대 throw 하지 않는다.
 * 도구 호출(tool item)은 이 맵에서 자기 id 로 결과(OUT)를 찾아 함께 그린다.
 * 같은 tool_use_id 가 중복되면 last-write-wins(Map.set) — tool_use id 는 Anthropic 이 고유 보장하고
 * 상류(누산기)가 seq dedup 하므로 실전 중복은 없다. 있어도 마지막 결과로 덮는 것이 안전한 폴백.
 */
function buildToolResultMap(items: StructuredItem[]): Map<string, ToolResult> {
  const map = new Map<string, ToolResult>()
  for (const item of items) {
    if (item.kind !== 'structured') continue
    const hit = parseToolResult(item.json)
    if (hit) map.set(hit.toolUseId, hit.result)
  }
  return map
}

/** tool args JSON 의 첫 문자열 값을 잘라 1줄 힌트(무슨 파일/명령인지). 절대 throw 하지 않는다. */
function shortArgs(argsJson: string): string {
  try {
    const parsed: unknown = JSON.parse(argsJson)
    if (parsed !== null && typeof parsed === 'object' && !Array.isArray(parsed)) {
      const obj = parsed as Record<string, unknown>
      for (const val of Object.values(obj)) {
        if (typeof val === 'string' && val.length > 0) {
          return val.length > 64 ? val.slice(0, 64) + '…' : val
        }
      }
    }
    return ''
  } catch {
    return ''
  }
}

// ── 타임라인 레일 래퍼(ADR-0048: 레이아웃 스캐폴드는 당분간 유지) ───────────────────

type DotTone = 'default' | 'accent' | 'error' | 'none'
const DOT: Record<Exclude<DotTone, 'none'>, string> = {
  default: 'bg-muted',
  accent: 'bg-accent',
  error: 'bg-red-500',
}

/**
 * 좌측 점선 타임라인 레일 — 각 행이 자기 몫의 레일 세그먼트(세로 점선 border)를 그린다.
 * 행들이 세로로 맞붙어(컨테이너 gap 없음, 간격은 콘텐츠 pb-4 로) 세그먼트가 이어져 연속된 점선이 된다.
 */
function Row({ dot = 'default', children }: { dot?: DotTone; children: ReactNode }) {
  return (
    <div className="relative flex gap-2.5">
      <div aria-hidden className="relative flex w-3.5 flex-none justify-center">
        <div className="pointer-events-none absolute inset-y-0 left-1/2 border-l border-dashed border-border" />
        {dot !== 'none' && (
          <div className={cn('relative mt-[9px] h-1.5 w-1.5 rounded-full', DOT[dot])} />
        )}
      </div>
      <div className="min-w-0 flex-1 pb-4">{children}</div>
    </div>
  )
}

// ── 이식 컴포넌트 어댑터 행 ──────────────────────────────────────────────────────────

/**
 * ★FIX 2 (fenced-code escape 방어)★: 도구 IN(args)/OUT(result)·탈출구 json 은 신뢰할 수 없는 텍스트다.
 *   이전 시안은 CodeAccordian → MarkdownBlock(react-markdown) 로 코드 펜스 문자열을 먹였는데, 내용에 삼중
 *   백틱(```) 줄이 있으면 펜스가 조기 종료돼 나머지가 마크다운으로 파싱된다(활성 링크/이미지·heading 주입).
 *   그래서 이 콘텐츠는 마크다운을 **절대 태우지 않고** 리터럴 <pre><code> 로만 그린다 — React 텍스트 자식은
 *   자동 이스케이프되므로 삼중 백틱·`# heading` 이 있어도 태그로 승격되지 않는다(inert). 전체 마크다운은
 *   assistant text(MarkdownRow) 에만 허용.
 */
function InertCode({ code }: { code: string }) {
  return (
    <pre className="overflow-x-auto rounded-xs border border-border bg-surface px-2.5 py-2 text-xs">
      <code className="whitespace-pre-wrap break-words font-mono text-foreground">{code}</code>
    </pre>
  )
}

/**
 * thinking 접힘 행 — Cline ThinkingRow(제목 토글 + 펼침 애니 본문)를 우리 데이터로 감싼다.
 * 로컬 expand state(itemId key 로 스트리밍 중에도 유지). 빈 reasoning 은 상위 dispatch 에서 이미 걸러진다.
 */
function ThinkingItemRow({ content }: { content: string }) {
  const [expanded, setExpanded] = useState(false)
  return (
    <ThinkingRow
      showTitle
      isVisible
      isExpanded={expanded}
      onToggle={() => setExpanded((o) => !o)}
      reasoningContent={content}
    />
  )
}

/**
 * 도구 호출 행 — 헤더(이름 + arg 힌트, 에러 배지) 접힘, 펼치면 IN(args)/OUT(result) 를 그린다.
 * IN/OUT 은 신뢰할 수 없는 텍스트이므로 InertCode(리터럴 <pre>)로만 렌더 — 마크다운 파싱 금지(FIX 2).
 */
function ToolItemRow({
  name,
  argsJson,
  result,
}: {
  name: string
  argsJson: string
  result: ToolResult | null
}) {
  const [open, setOpen] = useState(false)
  const hint = shortArgs(argsJson)
  const isErr = result?.isError === true
  return (
    <div className={cn('rounded-lg border', isErr ? 'border-red-500/60' : 'border-border')}>
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        className="flex w-full items-center gap-2 px-3 py-2 text-left"
      >
        <span className="flex-none font-mono text-[13px] font-medium text-foreground">{name}</span>
        {hint && <span className="truncate font-mono text-xs text-muted">{hint}</span>}
        {isErr && (
          <span className="ml-auto flex-none rounded border border-red-500 px-1.5 text-[10px] text-red-500">
            Error
          </span>
        )}
      </button>
      {open && (
        <div className="space-y-2 border-t border-border px-3 py-2">
          <div>
            <div className="mb-1 text-[10px] uppercase tracking-wide text-muted">In</div>
            {/* args JSON 을 리터럴 <pre> 로 — 마크다운 파싱 없이 원문 그대로(FIX 2). */}
            <InertCode code={pretty(argsJson)} />
          </div>
          {result && (
            <div>
              <div
                className={cn(
                  'mb-1 text-[10px] uppercase tracking-wide',
                  isErr ? 'text-red-500' : 'text-muted',
                )}
              >
                Out
              </div>
              {/* 결과 본문을 리터럴 <pre> 로 — 삼중 백틱이 있어도 inert(FIX 2). */}
              <InertCode code={result.content || '(빈 결과)'} />
            </div>
          )}
        </div>
      )}
    </div>
  )
}

/** 탈출구(알 수 없는 label) 이벤트 — 접힘 raw json. tool 행과 동형 룩. json 은 InertCode(리터럴, FIX 2). */
function GenericItemRow({ label, json }: { label: string; json: string }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="rounded-lg border border-border">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        className="flex w-full items-center gap-2 px-3 py-2 text-left"
      >
        <span className="truncate font-mono text-[13px] text-muted">{label}</span>
      </button>
      {open && (
        <div className="border-t border-border px-3 py-2">
          {/* raw json 을 리터럴 <pre> 로 — 마크다운 파싱 없이 원문 그대로(FIX 2). */}
          <InertCode code={pretty(json)} />
        </div>
      )}
    </div>
  )
}

// ── 항목 dispatch ───────────────────────────────────────────────────────────────────

/** 한 item 렌더. key 는 여기서 부여(itemId). standalone tool_result·빈 thinking 은 null 로 제외. */
function renderItem(item: StructuredItem, results: Map<string, ToolResult>): ReactNode {
  const k = item.itemId
  switch (item.kind) {
    case 'text':
      // assistant 본문 — Cline MarkdownRow(react-markdown + remark/rehype).
      return (
        <Row key={k} dot="default">
          <MarkdownRow markdown={item.text} />
        </Row>
      )

    case 'structured':
      // ★FIX 1 (tool_result 흡수 — label 무관)★: json.type==='tool_result' 인 structured 는 label 이
      //   무엇이든(user 든 아니든) 매칭 도구의 OUT 에 흡수되므로 독립 렌더하지 않는다. 이 검사는 label
      //   분기보다 **먼저** 와야 한다 — 이전엔 user 분기 안에만 있어 다른 label 의 tool_result 가 standalone
      //   으로 새 나갔다(계약 위반). 매칭 tool 이 없어도 흡수 규칙은 동일(어디에도 안 그린다).
      if (parseToolResult(item.json)) return null

      if (item.label === 'user') {
        // 사용자 발화 — Cline UserMessage 는 gRPC 편집기라 이식 불가 → 은은한 보더 박스 + MarkdownRow.
        return (
          <Row key={k} dot="accent">
            <div className="rounded-lg border border-border bg-surface px-3 py-2">
              <MarkdownRow markdown={extractText(item.json, 'user')} />
            </div>
          </Row>
        )
      }
      if (item.label === 'thinking') {
        // 빈 reasoning 은 렌더 안 함(빈 thinking 행 방지).
        const content = extractText(item.json, 'thinking')
        if (!content || !content.trim()) return null
        return (
          <Row key={k} dot="default">
            <ThinkingItemRow content={content} />
          </Row>
        )
      }
      // 기타 label(탈출구) — 접힘 generic 블록.
      return (
        <Row key={k} dot="default">
          <GenericItemRow label={item.label} json={item.json} />
        </Row>
      )

    case 'tool': {
      const result = item.id ? results.get(item.id) ?? null : null
      return (
        <Row key={k} dot="default">
          <ToolItemRow name={item.name} argsJson={item.argsJson} result={result} />
        </Row>
      )
    }

    case 'usage':
      // 토큰 사용량 — Cline 대응물 없음. de-emphasized muted 배지 행 유지.
      return (
        <Row key={k} dot="default">
          <div className="text-xs text-muted">
            in {item.inputTokens} · out {item.outputTokens}
          </div>
        </Row>
      )

    case 'error':
      // 에러 — Cline ErrorRow 는 billing 특화라 부적합. 붉은 강조 행 + AlertTriangle 유지.
      return (
        <Row key={k} dot="error">
          <div className="flex items-start gap-1.5 text-[13px] text-red-500">
            <AlertTriangle size={14} className="mt-0.5 flex-none" />
            <span className="whitespace-pre-wrap break-words">{item.message}</span>
          </div>
        </Row>
      )

    case 'separator':
      // 턴 경계 — 콘텐츠 컬럼의 아주 흐린 full-width divider(레일은 게터에서 계속 이어짐).
      return (
        <Row key={k} dot="none">
          <div aria-hidden className="border-t border-border opacity-25" />
        </Row>
      )
  }
}

/**
 * ADR-0048: 구조화 채팅 렌더 — 좌측 점선 타임라인 레일 + 항목별 Cline 이식 컴포넌트.
 * items 를 한 번 pre-scan 해 tool_use_id → tool_result 맵을 만들고(도구 OUT 흡수), standalone tool_result
 * 는 제외. streaming(턴 활성)이면 스트림 끝에 Cline ThinkingRow(isStreaming shimmer)를 붙인다.
 * 순수 렌더(props in, DOM out).
 */
export function StructuredTextView({
  items,
  streaming = false,
}: {
  items: StructuredItem[]
  streaming?: boolean
}) {
  const results = buildToolResultMap(items)
  return (
    <div className="flex flex-col px-3 py-3 font-sans text-foreground">
      {items.map((item) => renderItem(item, results))}
      {streaming && (
        // ★streaming 라이브 신호★ — Cline ThinkingRow 의 isStreaming(shimmer 제목) affordance. 스트림 끝에서만.
        //   ★FIX 3★: 안정 key — 없으면 streaming 토글 시 직전 실 item 이 이 행과 자리 매칭돼 remount 되며
        //   로컬 expand state 를 잃는다. 리스트 밖 고정 노드라 상수 key 로 정체성을 못박는다.
        <Row key="__streaming__" dot="default">
          <ThinkingRow showTitle isVisible isExpanded={false} isStreaming title="Thinking" />
        </Row>
      )}
    </div>
  )
}
