// ADR-0050: 구조화 채팅 렌더 dispatch. 벤치마크 룩 = Claude Code VSCode 확장(1차 근사치, 사용자가
//   스크린샷으로 후속 조정). 이전 라운드에서 도입했던 외부(Apache-2.0) 이식물은 전부 제거하고 자체
//   구현으로 대체했다.
//
// ★이 파일의 책임★: **순수 렌더** — 구독/누적은 RichSlot 소관.

import { useState, type ComponentType, type ReactNode } from 'react'
import {
  AlertTriangle,
  Bot,
  Braces,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  CircleHelp,
  CircleStop,
  FileCode2,
  FileMinus2,
  FilePlus2,
  FolderOpen,
  Globe,
  List,
  Pencil,
  Plug,
  Search,
  SquareTerminal,
  Wrench,
} from 'lucide-react'

import { cn } from '@/lib/utils'
import { t } from '../../i18n'
import {
  normalizeToolCategory,
  type StructuredItem,
  type ToolCategory,
  type ToolItem,
  type ToolResultMark,
  type TurnOutcomeMark,
} from './structuredAccumulator'
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
import { ToolGroupRow } from './chat/ToolGroupRow'
import {
  DECLINED_MARK,
  declinedReasonKey,
  groupToolRuns,
  toolCallVerdict,
  vendorErrorIdsOf,
} from './chat/toolRuns'

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

const CATEGORY_ICON: Record<Exclude<ToolCategory, 'Other'>, LucideIcon> = {
  Read: FileCode2,
  Search: Search,
  List: FolderOpen,
  Edit: Pencil,
  Command: SquareTerminal,
  Web: Globe,
  Agent: Bot,
  Mcp: Plug,
}

// ADR-0239: 종류를 모르면(`'Other'` — 옛 데몬도 여기 든다) 이름 휴리스틱으로 — 옛 데몬에서 오늘 모습 그대로다.
function toolIconOf(category: ToolCategory, name: string): LucideIcon {
  // 거르기는 타입 밖 낱말(캐스트로 든 항목)이 `undefined` 컴포넌트로 렌더를 깨뜨리지 않게 하려는 것이다.
  const known = normalizeToolCategory(category)
  return known === 'Other' ? toolIconFor(name) : CATEGORY_ICON[known]
}

/** 도구 이름만 보고 고르는 아이콘 — 종류(`category`)가 `'Other'` 일 때만 쓴다. */
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

// ADR-0241: 거부 행의 주황 경고 토큰 — 새 토큰을 짓지 않는다(e-ink 무력화가 이미 들어 있다 · ADR-0173).
const DECLINED_COLOR = 'var(--status-blocked)'
const DECLINED_BOX_BORDER = 'color-mix(in srgb, var(--status-blocked) 60%, transparent)'

/**
 * 줄 뿌리의 `data-tool-mark` — 누산기 표식에서 곧바로 온다(판정 `toolCallVerdict` 가 아니다): claude 의 벤더 오류
 * 본문은 표식이 아니라 속성이 없다. 우리 거절(`refused`)도 `"declined"` 이고 출처는 사유 줄의 `data-tool-declined-reason`
 * 이 가른다(ADR-0241).
 */
function toolMarkAttr(mark: ToolResultMark | null): 'failed' | 'declined' | undefined {
  if (mark === 'failed') return 'failed'
  if (mark === 'declined' || mark === 'refused') return 'declined'
  return undefined
}

/**
 * IN/OUT 은 신뢰할 수 없는 텍스트이므로 InertCode(리터럴 <pre>)로만 렌더 — 마크다운 파싱 금지(FIX 2).
 */
function ToolItemRow({
  name,
  argsJson,
  category,
  result,
  mark,
}: {
  name: string
  argsJson: string
  category: ToolCategory
  result: ToolResult | null
  mark: ToolResultMark | null
}) {
  const [open, setOpen] = useState(false)
  const hint = shortArgs(argsJson)
  // ADR-0241: 배지는 묶음 요약과 같은 판정이다 — 두 곳이 따로 가르면 머리의 「오류 N」과 펼친 배지가 어긋난다.
  const verdict = toolCallVerdict(mark, result?.isError === true, DECLINED_MARK)
  const isErr = verdict === 'error'
  const isDeclined = verdict === 'declined'
  const reasonKey = declinedReasonKey(mark, DECLINED_MARK)
  const Icon = toolIconOf(category, name)
  return (
    <div data-tool-mark={toolMarkAttr(mark)}>
      {/* ADR-0241: 거부 행의 머리는 기본 톤이다 — 실행되지 않은 호출을 붉게 칠하지 않는다. */}
      <RowHeader icon={Icon} title={name} tone={isErr ? 'error' : 'default'} />
      <div
        className={cn(
          'bg-surface rounded-sm overflow-hidden border',
          isErr ? 'border-red-500/60' : !isDeclined && 'border-border',
        )}
        style={isDeclined ? { borderColor: DECLINED_BOX_BORDER } : undefined}
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
          {isDeclined && (
            <span
              className="ml-auto flex-none rounded border px-1.5 text-[10px]"
              style={{ color: DECLINED_COLOR, borderColor: DECLINED_COLOR }}
            >
              {t('chat.toolDeclined')}
            </span>
          )}
        </button>
        {reasonKey !== null && (
          // ADR-0241: 사유 줄은 이 줄 상자 안이다 — 새 항목도 레일 행도 아니다(ADR-0051) · 세부를 접어도 보인다.
          <div data-tool-declined-reason={reasonKey} className="px-2.5 pb-2 text-xs text-muted">
            {t(reasonKey)}
          </div>
        )}
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

/**
 * 턴 결말 표식 — 실패/중단/모름 셋이 ★화면에서 서로 구별돼야 한다★(그 구별이 결말 칸의 존재 이유다).
 * 가르는 축 셋: 아이콘 · 문구 · 톤. ★색 하나에 기대지 않는다★ — e-ink 테마는 색을 무력화하므로
 * 색만 다르면 셋이 한 모양이 된다.
 * ★중단·모름은 붉게 칠하지 않는다★ — 중단은 사용자가 끊은 정상 경로이고, 모름은 「모른다」이지
 * 「실패했다」가 아니다. 실패만 error 톤을 쓴다(Error 행과 같은 어휘).
 */
/**
 * 중단의 강조 줄(U8 — 굵게 · 강조색 · `CircleStop`). 결말 행의 중단 갈래와 끊김 표시 행이 함께 쓴다 — 두 행이 같은
 * 모양이어야 한다(ADR-0243).
 */
// ADR-0237
function InterruptedLine({ children, note = false }: { children: ReactNode; note?: boolean }) {
  return (
    <div data-interrupt-note={note ? '' : undefined} className="flex items-center gap-2.5 text-accent">
      <CircleStop className="size-3.5 flex-none" />
      <span className="font-bold whitespace-pre-wrap break-words">{children}</span>
    </div>
  )
}

function OutcomeRow({ outcome, detail }: { outcome: TurnOutcomeMark; detail: string | null }) {
  const isErr = outcome === 'failed'
  // ADR-0237: 사용자 결정 U8 — 중단은 굵게 + 강조색. 실패의 빨강 · 거부의 주황과 갈리고, 모름은 그대로 muted 다.
  const Icon = isErr ? AlertTriangle : CircleHelp
  return (
    <div className="my-1">
      {outcome === 'interrupted' ? (
        <InterruptedLine>{t('chat.turnInterrupted')}</InterruptedLine>
      ) : (
        <div className={cn('flex items-center gap-2.5', isErr ? 'text-red-500' : 'text-muted')}>
          <Icon className="size-3.5 flex-none" />
          <span className={cn(isErr && 'font-bold')}>
            {isErr ? t('chat.turnFailed') : t('chat.turnUnknown')}
          </span>
        </div>
      )}
      {detail !== null && detail !== '' && (
        // 사유는 상대가 준 신뢰할 수 없는 텍스트 — 마크다운을 태우지 않고 리터럴로만 그린다(FIX 2 와 같은 규율).
        <div
          className={cn(
            'mt-1 whitespace-pre-wrap break-words',
            isErr ? 'text-red-500' : 'text-muted',
          )}
        >
          {detail}
        </div>
      )}
    </div>
  )
}

/**
 * 백엔드가 끊긴 턴에 적는 끊김 표시(오늘 내는 곳 = claude 합성 줄) — 글은 원문이고 마크다운을 태우지 않는다(FIX 2 와
 * 같은 규율).
 */
// ADR-0243
function InterruptNoteRow({ text }: { text: string }) {
  return (
    <div className="my-1">
      <InterruptedLine note>{text}</InterruptedLine>
    </div>
  )
}

/**
 * 이 셸이 모르는 이벤트가 왔다는 표식(누산기 default arm).
 *
 * ★이벤트 이름·payload 를 그리지 않는다★ — 프로토콜 낱말을 사용자 화면에 올리지 않는다(trd-phase2a §6-2).
 *   진단에 필요한 이름은 누산기가 console 로만 내보낸다.
 * ★assistant 본문처럼 보이면 안 된다★ — muted 톤 한 줄로 두어 「우리 쪽 한계 고지」로 읽히게 한다.
 */
function UnsupportedRow({ count }: { count: number }) {
  return (
    <div className="my-1 flex items-center gap-2.5 text-muted">
      <CircleAlert className="size-3.5 flex-none" />
      <span>{t('chat.unsupportedEvent', { count: String(count) })}</span>
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
    case 'outcome':
    case 'interruptNote':
    case 'unsupported':
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

/**
 * 이 항목이 화면에 행을 그리나 — `rowKindOf` 의 skip 판정 그대로다(ADR-0051 null 반환 규칙과 한 몸).
 * 턴 구분선(`separator`)도 참이다 — 빈 스페이서지만 DOM 행이다. ADR-0226 의 "이력이 도착했나" 판정은
 * 이 위에서 구분선을 따로 뺀다(RichSlot).
 */
export function isRenderedItem(item: StructuredItem): boolean {
  return rowKindOf(item) !== 'skip'
}

function toolRowOf(item: ToolItem, results: Map<string, ToolResult>): ReactNode {
  const result = item.id ? results.get(item.id) ?? null : null
  return (
    <ToolItemRow
      name={item.name}
      argsJson={item.argsJson}
      category={item.category}
      result={result}
      mark={item.resultMark}
    />
  )
}

/**
 * 펼친 묶음 안의 멤버 한 줄 — 레일(`ChatRow`) 없이 행 컴포넌트만(ADR-0051 — 묶음이 레일 한 행이다). 멤버는 도구 행과
 * 비지 않은 생각뿐이다(`groupToolRuns` 의 계약) — 그 계약이 넓어지면 여기 갈래를 더한다.
 */
function renderGroupMember(item: StructuredItem, results: Map<string, ToolResult>): ReactNode {
  if (item.kind === 'tool') return toolRowOf(item, results)
  if (item.kind === 'structured' && item.label === 'thinking') {
    return <ThoughtRow content={extractText(item.json, 'thinking')} />
  }
  return null
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

    case 'tool':
      return (
        <ChatRow key={k} rail tone="tool" runPos={pos}>
          {toolRowOf(item, results)}
        </ChatRow>
      )

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

    case 'outcome':
      // ★정상 완료는 여기 오지 않는다★ — 누산기가 표식 없이 구분선만 남긴다(평범한 끝맺음이 경고처럼
      //   보이지 않게). 그래서 이 행이 그리는 것은 실패·중단·모름 셋뿐이다.
      return (
        <ChatRow key={k} rail tone={item.outcome === 'failed' ? 'error' : 'default'} runPos={pos}>
          <OutcomeRow outcome={item.outcome} detail={item.detail} />
        </ChatRow>
      )

    case 'interruptNote':
      return (
        <ChatRow key={k} rail runPos={pos}>
          <InterruptNoteRow text={item.text} />
        </ChatRow>
      )

    case 'unsupported':
      return (
        <ChatRow key={k} rail runPos={pos}>
          <UnsupportedRow count={item.count} />
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
  slotId,
  onGroupToggle,
  interrupting = false,
}: {
  items: StructuredItem[]
  streaming?: boolean
  /** 도구 묶음 펼침 상태를 둘 슬롯(`store/toolGroupStore.ts`) — 없으면 묶음은 자동 규칙으로만 펼치고 접힌다. */
  slotId?: string
  /**
   * 사람이 도구 묶음 머리를 눌러 펼쳤다 — 접을 때는 부르지 않는다. `isLast` = 그 묶음이 목록의 마지막 묶음인가.
   * TRD S21-chat-ux §4-5: 마지막이 아닌 묶음을 펼치면 바닥 따라가기를 푼다(붙은 채면 누른 머리가 화면 위로 밀려난다).
   */
  onGroupToggle?: (isLast: boolean) => void
  /**
   * 끊기를 보내고 그 턴이 끝나기를 기다린다 — 대기 꼬리가 「중단하는 중」을 그린다(ADR-0244). `streaming` 이 아니면 그릴
   * 꼬리가 없어 읽지 않는다.
   */
  interrupting?: boolean
}) {
  const results = buildToolResultMap(items)
  // ADR-0241: 벤더 오류 id 는 한 렌더에 한 번 짓고 모든 묶음의 요약이 나눠 쓴다.
  const vendorErrorIds = vendorErrorIdsOf(results)
  // ADR-0239: 묶기는 렌더 중 파생이다 — 누산기도 백엔드도 묶음을 만들지 않고, 펼침 상태는 `ToolGroupRow` 가 읽는다.
  const rows = groupToolRuns(items, streaming, rowKindOf)
  // ★showTail = streaming★: 콘텐츠 유무 게이트 없이 streaming 이면 곧바로 대기 인디케이터(WaitRow)를 붙인다 —
  //   전송 즉시(awaiting=true, items 아직 빔) 인디케이터가 뜬다("첫 바이트 전엔 무표시" 갭 제거). fresh/idle
  //   슬롯 오작동은 상류 streaming 파생(awaiting || (!turnDone && items.length>0), RichSlot FIX 5)이 이미
  //   막으므로(never-sent 슬롯 = streaming=false) 여기서 재게이트 불필요.
  const showTail = streaming
  // ADR-0051: rail run 위치를 순수 계산으로 미리 뽑는다(렌더 중 파생 — state/effect 아님, ADR-0050 순수성
  //   유지). streaming tail(WaitRow)도 마지막 assistant 행으로 함께 계산해, 직전 실 행이 tail 과 연결선으로
  //   이어지게 한다(tail 이 없으면 bottom/single 로 clean-end). ★항목이 아니라 행 목록으로 센다★ — 묶음은
  //   레일 한 행(assistant)이고 멤버는 레일을 그리지 않는다.
  const kinds = rows.map((row) => (row.kind === 'toolGroup' ? 'assistant' : rowKindOf(row.item)))
  if (showTail) kinds.push('assistant')
  const positions = computeRailRunPositions(kinds)
  const tailPos = showTail ? (positions[positions.length - 1] ?? 'single') : 'single'
  let lastGroup = -1
  rows.forEach((row, i) => {
    if (row.kind === 'toolGroup') lastGroup = i
  })
  const renderMember = (item: StructuredItem) => renderGroupMember(item, results)
  return (
    // 채팅 루트 폰트/줄간격을 여기에만 스코프한다(트리·터미널 슬롯 등 앱 나머지는 영향 없음).
    //   CSS 변수로 뺀 건 LLM 제어용(ADR-0051).
    <div
      className="flex flex-col pb-3 font-sans text-foreground"
      style={{ fontSize: 'var(--chat-font-size)', lineHeight: 'var(--chat-line-height)' }}
    >
      {rows.map((row, i) =>
        row.kind === 'item' ? (
          renderItem(row.item, results, positions[i])
        ) : (
          <ToolGroupRow
            key={row.key}
            row={row}
            vendorErrorIds={vendorErrorIds}
            runPos={positions[i] ?? undefined}
            isLast={i === lastGroup}
            renderMember={renderMember}
            slotId={slotId}
            onGroupToggle={onGroupToggle}
          />
        ),
      )}
      {showTail && (
        // 구 "Thinking…" pulse 라벨을 임시 "Wait + 점 + 경과 초" 로 대체(임시·추후 재설계 — WaitRow 헤더 참조).
        //   ★FIX 3(안정 key)★: key="__streaming__" — 없으면 streaming 토글/리렌더 시 직전 실 item 이 이 행과
        //   자리 매칭돼 remount 되며 WaitRow 타이머(경과 초)가 턴 도중 리셋된다. 리스트 밖 고정 노드라 상수
        //   key 로 정체성을 못박는다(변경 금지).
        <ChatRow key="__streaming__" rail runPos={tailPos}>
          <WaitRow interrupting={interrupting} />
        </ChatRow>
      )}
      {showTail && (
        // 대기 tail 하단 여백 — 일반 메시지는 턴 종료 시 뒤에 깔리는 separator(h-3)로 입력창과 간격이 생기지만,
        //   awaiting Wait 은 아직 turnDone 이 아니라(응답 대기) separator 가 없어 입력창에 딱 붙는다. 같은 높이의
        //   빈 스페이서로 일반 메시지와 동일한 하단 간격(12px)을 준다. ★패딩이 아니라 실제 높이 블록★: Radix
        //   ScrollArea 의 display:table 래퍼가 마지막 요소의 하단 패딩을 scrollHeight 에 안 넣어, 바닥에 붙어
        //   따라가는 동안(ADR-0242) 패딩은 뷰포트 밖으로 밀려 안 보인다. 높이 가진 블록은 표가 세므로 정상 반영된다.
        <div aria-hidden className="h-3" />
      )}
    </div>
  )
}
