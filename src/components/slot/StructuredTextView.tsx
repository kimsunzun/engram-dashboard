// ADR-0050: 구조화 채팅 렌더 dispatch. 벤치마크 룩 = Claude Code VSCode 확장(1차 근사치, 사용자가
//   스크린샷으로 후속 조정). 이전 라운드에서 도입했던 외부(Apache-2.0) 이식물은 전부 제거하고 자체
//   구현으로 대체했다.
//
// ★이 파일의 책임★: **순수 렌더** — 구독/누적은 RichSlot 소관.

import { useState, type ComponentType, type ReactNode } from 'react'
import {
  AlertTriangle,
  Braces,
  ChevronDown,
  ChevronRight,
  FileCode2,
  FileMinus2,
  FilePlus2,
  FolderOpen,
  Globe,
  List,
  Pencil,
  Search,
  SquareTerminal,
  Wrench,
} from 'lucide-react'

import { cn } from '@/lib/utils'
import { t } from '../../i18n'
import type { StructuredItem } from './structuredAccumulator'
import { Markdown } from './chat/Markdown'
import { ThoughtRow } from './chat/ThoughtRow'
import { WaitRow } from './chat/WaitRow'
// ADR-0053 구조 분할: 이 파일은 dispatch 오케스트레이터로만 남긴다(순수 로직 ↔ 컴포넌트 경계).
import { ChatRow } from './chat/ChatRow'
import {
  computeRailRunPositions,
  type ChatRowKind,
  type RailRunPosition,
} from './chat/railPositions'

// ── 안전 파서 헬퍼(절대 throw 금지 — bad json 폴백) ────────────────────────────────

function pretty(json: string): string {
  try {
    return JSON.stringify(JSON.parse(json), null, 2)
  } catch {
    return json
  }
}

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

/** Anthropic content 블록(문자열 | 블록 배열) 스키마에 맞춘 추출. */
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

type ToolResult = { content: string; isError: boolean }

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

// ── 채팅 룩 프리미티브 ────────────────────────────────────────────────────────────

const HEADER_CLASSNAMES = 'flex items-center gap-2.5 mb-3'

type LucideIcon = ComponentType<{ className?: string }>

/** 도구 헤더 아이콘 휴리스틱 — 우리 tool item 은 generic(name 만) 이라 도구 종류 판별자가 없다. */
function toolIconFor(name: string): LucideIcon {
  const n = name.toLowerCase()
  if (n.includes('multiedit') || n.includes('edit') || n.includes('write') || n.includes('replace'))
    return Pencil
  if (n.includes('create') || n.includes('new')) return FilePlus2
  if (n.includes('delete') || n.includes('remove') || n.includes('rm')) return FileMinus2
  if (n.includes('read') || n.includes('cat') || n.includes('view')) return FileCode2
  if (n.includes('glob') || n.includes('ls') || n.includes('list') || n.includes('dir'))
    return FolderOpen
  if (n.includes('grep') || n.includes('search') || n.includes('find')) return Search
  if (n.includes('bash') || n.includes('shell') || n.includes('exec') || n.includes('command'))
    return SquareTerminal
  if (n.includes('web') || n.includes('fetch') || n.includes('http') || n.includes('url'))
    return Globe
  if (n.includes('todo') || n.includes('task') || n.includes('plan')) return List
  return Wrench
}

/** 아이콘은 size-3.5(≈14px)로 우리 폰트 스케일에 맞춘다. */
function RowHeader({
  icon: Icon,
  title,
  tone = 'default',
}: {
  icon: LucideIcon
  title: ReactNode
  tone?: 'default' | 'error'
}) {
  return (
    <div className={HEADER_CLASSNAMES}>
      <Icon className={cn('size-3.5 flex-none', tone === 'error' ? 'text-red-500' : 'text-foreground')} />
      <span className={cn('font-bold', tone === 'error' ? 'text-red-500' : 'text-foreground')}>
        {title}
      </span>
    </div>
  )
}

// ── 어댑터 행 ──────────────────────────────────────────────────────────

/**
 * ★FIX 2 (fenced-code escape 방어)★: 도구 IN(args)/OUT(result)·탈출구 json 은 신뢰할 수 없는 텍스트다.
 *   이 콘텐츠를 마크다운 렌더러에 먹이면 내용에 삼중 백틱(```) 줄이 있을 때 펜스가 조기 종료돼 나머지가
 *   마크다운으로 파싱된다(활성 링크/이미지·heading 주입). 그래서 이 콘텐츠는 마크다운을 **절대 태우지 않고**
 *   리터럴 <pre><code> 로만 그린다 — React 텍스트 자식은 자동 이스케이프되므로 삼중 백틱·`# heading` 이
 *   있어도 태그로 승격되지 않는다(inert). 전체 마크다운은 assistant text(Markdown) 에만 허용.
 */
function InertCode({ code }: { code: string }) {
  return (
    <pre className="overflow-x-auto rounded-xs border border-border bg-surface px-2.5 py-2 text-xs">
      <code className="whitespace-pre-wrap break-words font-mono text-foreground">{code}</code>
    </pre>
  )
}

/**
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
  const Icon = toolIconFor(name)
  return (
    <div>
      <RowHeader icon={Icon} title={name} tone={isErr ? 'error' : 'default'} />
      <div
        className={cn(
          'bg-surface rounded-sm overflow-hidden border',
          isErr ? 'border-red-500/60' : 'border-border',
        )}
      >
        {/* aria-label 에 도구명을 실어 접근성 이름을 헤더와 일치시킨다(sub-header 텍스트는 인자 힌트라
            도구명이 없으므로, 스크린리더/테스트가 "어느 도구의 세부인지" 식별하게 name 을 명시). */}
        <button
          type="button"
          onClick={() => setOpen((o) => !o)}
          aria-expanded={open}
          aria-label={name}
          className="flex w-full items-center gap-2 cursor-pointer select-none py-2 px-2.5 text-left text-muted"
        >
          {open ? (
            <ChevronDown className="size-3.5 flex-none" />
          ) : (
            <ChevronRight className="size-3.5 flex-none" />
          )}
          {hint ? (
            <span className="truncate font-mono text-xs">{hint}</span>
          ) : (
            <span className="truncate font-mono text-xs opacity-70">arguments</span>
          )}
          {isErr && (
            <span className="ml-auto flex-none rounded border border-red-500 px-1.5 text-[10px] text-red-500">
              Error
            </span>
          )}
        </button>
        {open && (
          <div className="space-y-2 border-t border-border px-2.5 py-2">
            <div>
              <div className="mb-1 text-[10px] uppercase tracking-wide text-muted">In</div>
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
                <InertCode code={result.content || t('common.emptyResult')} />
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  )
}

function GenericItemRow({ label, json }: { label: string; json: string }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="bg-surface rounded-sm overflow-hidden border border-border">
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        aria-expanded={open}
        className="flex w-full items-center gap-2 cursor-pointer select-none py-2 px-2.5 text-left text-muted"
      >
        {open ? (
          <ChevronDown className="size-3.5 flex-none" />
        ) : (
          <ChevronRight className="size-3.5 flex-none" />
        )}
        <Braces className="size-3.5 flex-none" />
        <span className="truncate font-mono text-xs">{label}</span>
      </button>
      {open && (
        <div className="border-t border-border px-2.5 py-2">
          <InertCode code={pretty(json)} />
        </div>
      )}
    </div>
  )
}

// ── 항목 dispatch ───────────────────────────────────────────────────────────────────

/**
 * ADR-0051: 공백만 = opus 암호화 thinking(signature 만 옴, 평문 없음). rowKindOf 와 renderItem 이 **같은**
 * 판정을 써야 rail 계산과 실제 DOM 이 일치한다(빈 thinking = skip = 렌더 안 함).
 */
function isEmptyThinking(json: string): boolean {
  return extractText(json, 'thinking').trim() === ''
}

/**
 * ADR-0051: renderItem 의 null 반환 규칙과 반드시 일치해야 한다(흡수된 tool_result·usage·빈 thinking =
 * skip = DOM 없음).
 */
function rowKindOf(item: StructuredItem): ChatRowKind {
  switch (item.kind) {
    case 'text':
    case 'tool':
    case 'error':
      return 'assistant'
    case 'usage':
      return 'skip'
    case 'separator':
      return 'boundary'
    case 'structured':
      if (parseToolResult(item.json)) return 'skip'
      if (item.label === 'user') return 'boundary'
      if (item.label === 'thinking' && isEmptyThinking(item.json)) return 'skip'
      return 'assistant'
  }
}

/** runPos(ADR-0051): rail 행의 run 내 위치 — 연결선 clean-ends. */
function renderItem(
  item: StructuredItem,
  results: Map<string, ToolResult>,
  runPos: RailRunPosition | null,
): ReactNode {
  const k = item.itemId
  const pos = runPos ?? undefined
  switch (item.kind) {
    case 'text':
      // 긴 토큰(URL·경로)이 컨테이너를 넘지 않게 행 컨테이너에 wrap-anywhere overflow-hidden.
      return (
        <ChatRow key={k} rail runPos={pos}>
          <div className="wrap-anywhere overflow-hidden">
            <Markdown markdown={item.text} />
          </div>
        </ChatRow>
      )

    case 'structured':
      // ★FIX 1 (tool_result 흡수 — label 무관)★: json.type==='tool_result' 인 structured 는 label 이
      //   무엇이든(user 든 아니든) 매칭 도구의 OUT 에 흡수되므로 독립 렌더하지 않는다. 이 검사는 label
      //   분기보다 **먼저** 와야 한다 — 이전엔 user 분기 안에만 있어 다른 label 의 tool_result 가 standalone
      //   으로 새 나갔다(계약 위반). 매칭 tool 이 없어도 흡수 규칙은 동일(어디에도 안 그린다).
      if (parseToolResult(item.json)) return null

      if (item.label === 'user') {
        return (
          <ChatRow key={k}>
            <div
              className="rounded-[0.75rem] border border-border bg-elevated whitespace-pre-line break-words text-foreground"
              style={{
                marginLeft: '0.75rem',
                marginRight: '0.75rem',
                paddingTop: 'var(--chat-user-py)',
                paddingBottom: 'var(--chat-user-py)',
                paddingLeft: 'var(--chat-user-px)',
                paddingRight: 'var(--chat-user-px)',
                marginTop: 'var(--chat-user-my)',
                marginBottom: 'var(--chat-user-my)',
              }}
            >
              {extractText(item.json, 'user')}
            </div>
          </ChatRow>
        )
      }
      if (item.label === 'thinking') {
        // 내용이 비면(opus 암호화 thinking — signature 만 옴) 빈 "Thought" 클러터가 매 응답마다 뜨므로
        //   아무것도 그리지 않는다(null). rowKindOf 도 같은 isEmptyThinking 검사로 'skip' 을 반환해야
        //   rail 계산과 DOM 이 일치한다(ADR-0051).
        if (isEmptyThinking(item.json)) return null
        const content = extractText(item.json, 'thinking')
        return (
          <ChatRow key={k} rail runPos={pos}>
            <ThoughtRow content={content} />
          </ChatRow>
        )
      }
      return (
        <ChatRow key={k} rail runPos={pos}>
          <GenericItemRow label={item.label} json={item.json} />
        </ChatRow>
      )

    case 'tool': {
      const result = item.id ? results.get(item.id) ?? null : null
      return (
        <ChatRow key={k} rail tone="tool" runPos={pos}>
          <ToolItemRow name={item.name} argsJson={item.argsJson} result={result} />
        </ChatRow>
      )
    }

    case 'usage':
      // 메시지별 토큰 칩은 표시하지 않는다(누적 item 종류 자체는 유지 — 렌더만 생략).
      return null

    case 'error':
      return (
        <ChatRow key={k} rail tone="error" runPos={pos}>
          <RowHeader icon={AlertTriangle} title="Error" tone="error" />
          <div className="text-red-500 whitespace-pre-wrap break-words">{item.message}</div>
        </ChatRow>
      )

    case 'separator':
      // 턴 경계 — 점선 레일/구분선 없이 아주 옅은 세로 스페이서만(눈에 띄는 divider 지양).
      return <div key={k} aria-hidden className="h-3" />
  }
}

export function StructuredTextView({
  items,
  streaming = false,
}: {
  items: StructuredItem[]
  streaming?: boolean
}) {
  const results = buildToolResultMap(items)
  // ★showTail = streaming★: 콘텐츠 유무 게이트 없이 streaming 이면 곧바로 대기 인디케이터(WaitRow)를 붙인다 —
  //   전송 즉시(awaiting=true, items 아직 빔) 인디케이터가 뜬다("첫 바이트 전엔 무표시" 갭 제거). fresh/idle
  //   슬롯 오작동은 상류 streaming 파생(awaiting || (!turnDone && items.length>0), RichSlot FIX 5)이 이미
  //   막으므로(never-sent 슬롯 = streaming=false) 여기서 재게이트 불필요.
  const showTail = streaming
  // ADR-0051: rail run 위치를 순수 계산으로 미리 뽑는다(렌더 중 파생 — state/effect 아님, ADR-0050 순수성
  //   유지). streaming tail(WaitRow)도 마지막 assistant 행으로 함께 계산해, 직전 실 행이 tail 과 연결선으로
  //   이어지게 한다(tail 이 없으면 bottom/single 로 clean-end).
  const kinds = items.map(rowKindOf)
  if (showTail) kinds.push('assistant')
  const positions = computeRailRunPositions(kinds)
  const tailPos = showTail ? (positions[positions.length - 1] ?? 'single') : 'single'
  return (
    // 채팅 루트 폰트/줄간격을 여기에만 스코프한다(트리·터미널 슬롯 등 앱 나머지는 영향 없음).
    //   CSS 변수로 뺀 건 LLM 제어용(ADR-0051).
    <div
      className="flex flex-col pb-3 font-sans text-foreground"
      style={{ fontSize: 'var(--chat-font-size)', lineHeight: 'var(--chat-line-height)' }}
    >
      {items.map((item, i) => renderItem(item, results, positions[i]))}
      {showTail && (
        // 구 "Thinking…" pulse 라벨을 임시 "Wait + 점 + 경과 초" 로 대체(임시·추후 재설계 — WaitRow 헤더 참조).
        //   ★FIX 3(안정 key)★: key="__streaming__" — 없으면 streaming 토글/리렌더 시 직전 실 item 이 이 행과
        //   자리 매칭돼 remount 되며 WaitRow 타이머(경과 초)가 턴 도중 리셋된다. 리스트 밖 고정 노드라 상수
        //   key 로 정체성을 못박는다(변경 금지).
        <ChatRow key="__streaming__" rail runPos={tailPos}>
          <WaitRow />
        </ChatRow>
      )}
      {showTail && (
        // 대기 tail 하단 여백 — 일반 메시지는 턴 종료 시 뒤에 깔리는 separator(h-3)로 입력창과 간격이 생기지만,
        //   awaiting Wait 은 아직 turnDone 이 아니라(응답 대기) separator 가 없어 입력창에 딱 붙는다. 같은 높이의
        //   빈 스페이서로 일반 메시지와 동일한 하단 간격(12px)을 준다. ★패딩이 아니라 실제 높이 블록★: Radix
        //   ScrollArea 의 display:table 래퍼가 마지막 요소의 하단 패딩을 scrollHeight 에 안 넣어(+ 하단 고정
        //   auto-scroll) 패딩은 뷰포트 밖으로 밀려 안 보인다. 높이 가진 블록은 표가 세므로 정상 반영된다.
        <div aria-hidden className="h-3" />
      )}
    </div>
  )
}
