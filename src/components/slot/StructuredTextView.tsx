// ADR-0050: structured chat render dispatch. Benchmark look = the Claude Code VSCode extension (first approximation, the user
//   adjusts afterward from screenshots). All external (Apache-2.0) ports introduced in an earlier round were removed and replaced with
//   our own implementation.
//
// ★This file's responsibility★: **pure render** — subscription/accumulation is RichSlot's concern.

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
// ADR-0053 structural split: this file is left only as the dispatch orchestrator (the boundary between pure logic ↔ components).
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

// ── Safe parser helpers (never throw — bad json fallback) ────────────────────────────────

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

/** Extraction matched to the Anthropic content block (string | block array) schema. */
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
 * If the same tool_use_id is duplicated, last-write-wins (Map.set) — Anthropic guarantees tool_use ids are unique
 * and upstream (the accumulator) does seq dedup, so there are no real duplicates. Even if there were, overwriting with the last result is a safe fallback.
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

// ── Chat look primitives ────────────────────────────────────────────────────────────

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

// ADR-0239: if the kind is unknown (`'Other'` — an old daemon falls in here too), use the name heuristic — it looks exactly as today on an old daemon.
function toolIconOf(category: ToolCategory, name: string): LucideIcon {
  // The filtering is so that a word outside the type (an item that got in by cast) does not break the render with an `undefined` component.
  const known = normalizeToolCategory(category)
  return known === 'Other' ? toolIconFor(name) : CATEGORY_ICON[known]
}

/** Icon chosen by tool name alone — used only when the kind (`category`) is `'Other'`. */
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

/** Icons are size-3.5 (≈14px) to match our font scale. */
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

// ── Adapter rows ──────────────────────────────────────────────────────────

/**
 * ★FIX 2 (fenced-code escape defense)★: tool IN(args)/OUT(result) and the escape-hatch json are untrusted text.
 *   Feeding this content to a markdown renderer ends the fence early if the content has a triple-backtick (```) line, and the rest
 *   is parsed as markdown (active link/image · heading injection). So this content is **never put through markdown**
 *   and is drawn only as a literal <pre><code> — React text children are auto-escaped, so even with triple backticks or `# heading`
 *   they are not promoted to tags (inert). Full markdown is allowed only for assistant text (Markdown).
 */
function InertCode({ code }: { code: string }) {
  return (
    <pre className="overflow-x-auto rounded-xs border border-border bg-surface px-2.5 py-2 text-xs">
      <code className="whitespace-pre-wrap break-words font-mono text-foreground">{code}</code>
    </pre>
  )
}

// ADR-0241: the orange warning token of a decline row — we do not make a new token (e-ink neutralization is already in it · ADR-0173).
const DECLINED_COLOR = 'var(--status-blocked)'
const DECLINED_BOX_BORDER = 'color-mix(in srgb, var(--status-blocked) 60%, transparent)'

/**
 * `data-tool-mark` on the line root — comes straight from the accumulator marker (not from the decision `toolCallVerdict`): claude's vendor error
 * body is not a marker and has no attribute. Our refusal (`refused`) is also `"declined"` and the source is told apart by
 * `data-tool-declined-reason` on the reason line (ADR-0241).
 */
function toolMarkAttr(mark: ToolResultMark | null): 'failed' | 'declined' | undefined {
  if (mark === 'failed') return 'failed'
  if (mark === 'declined' || mark === 'refused') return 'declined'
  return undefined
}

/**
 * IN/OUT is untrusted text, so it is rendered only with InertCode (a literal <pre>) — no markdown parsing (FIX 2).
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
  // ADR-0241: the badge is the same decision as the bundle summary — if two places split separately, the head's 「오류 N」 (N errors) and the expanded badge diverge.
  const verdict = toolCallVerdict(mark, result?.isError === true, DECLINED_MARK)
  const isErr = verdict === 'error'
  const isDeclined = verdict === 'declined'
  const reasonKey = declinedReasonKey(mark, DECLINED_MARK)
  const Icon = toolIconOf(category, name)
  return (
    <div data-tool-mark={toolMarkAttr(mark)}>
      {/* ADR-0241: the head of a decline row is the default tone — a call that did not run is not painted red. */}
      <RowHeader icon={Icon} title={name} tone={isErr ? 'error' : 'default'} />
      <div
        className={cn(
          'bg-surface rounded-sm overflow-hidden border',
          isErr ? 'border-red-500/60' : !isDeclined && 'border-border',
        )}
        style={isDeclined ? { borderColor: DECLINED_BOX_BORDER } : undefined}
      >
        {/* Puts the tool name in aria-label to make the accessible name match the header (the sub-header text is an argument hint and
            has no tool name, so name is stated explicitly so screen readers/tests can identify "which tool's detail this is"). */}
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
          // ADR-0241: the reason line is inside this line's box — it is neither a new item nor a rail row (ADR-0051) · it shows even if the detail is collapsed.
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
 * Turn outcome marker — failure/interrupt/unknown, the three ★must be told apart on screen★ (that distinction is the outcome field's reason to exist).
 * Three axes tell them apart: icon · text · tone. ★Do not lean on color alone★ — the e-ink theme neutralizes color, so
 * if only color differed the three would become one shape.
 * ★Interrupt and unknown are not painted red★ — an interrupt is a normal path the user cut, and unknown means 「모른다」 (we do not know), not
 * 「실패했다」 (it failed). Only failure uses the error tone (the same vocabulary as the Error row).
 */
/**
 * The interrupt emphasis line (U8 — bold · accent color · `CircleStop`). Used by both the interrupt branch of the outcome row and the interruption display row — the two rows must have the same
 * shape (ADR-0243).
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
  // ADR-0237: user decision U8 — interrupt is bold + accent color. It is distinct from failure's red · decline's orange, and unknown stays muted as is.
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
        // The reason is untrusted text from the other side — drawn only as a literal, without markdown (the same discipline as FIX 2).
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
 * The interruption display the backend writes on an interrupted turn (where it is emitted today = the claude synthetic line) — the text is the original and is not put through markdown (the same
 * discipline as FIX 2).
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
 * A marker that an event this shell does not know has arrived (the accumulator's default arm).
 *
 * ★Do not draw the event name or payload★ — protocol words are not put on the user's screen (trd-phase2a §6-2).
 *   The accumulator sends the name needed for diagnosis to the console only.
 * ★It must not look like assistant body text★ — keep it to one muted-tone line so it reads as a 「우리 쪽 한계 고지」 (a notice of our own limit).
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

// ── Item dispatch ───────────────────────────────────────────────────────────────────

/**
 * ADR-0051: whitespace only = opus encrypted thinking (only a signature comes, no plain text). rowKindOf and renderItem must use the **same**
 * decision so the rail calculation and the real DOM agree (empty thinking = skip = not rendered).
 */
function isEmptyThinking(json: string): boolean {
  return extractText(json, 'thinking').trim() === ''
}

/**
 * ADR-0051: must match renderItem's null return rule (absorbed tool_result · usage · empty thinking =
 * skip = no DOM).
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
 * Does this item draw a row on screen — exactly the skip decision of `rowKindOf` (one body with the ADR-0051 null return rule).
 * The turn divider (`separator`) is true too — an empty spacer, but a DOM row. The "did history arrive?" decision of ADR-0226
 * takes the divider out separately above this (RichSlot).
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
 * One member line inside an expanded bundle — only the row component, without the rail (`ChatRow`) (ADR-0051 — the bundle is one rail row). Members are only tool rows and
 * non-empty thoughts (the contract of `groupToolRuns`) — if that contract widens, add a branch here.
 */
function renderGroupMember(item: StructuredItem, results: Map<string, ToolResult>): ReactNode {
  if (item.kind === 'tool') return toolRowOf(item, results)
  if (item.kind === 'structured' && item.label === 'thinking') {
    return <ThoughtRow content={extractText(item.json, 'thinking')} />
  }
  return null
}

/** runPos (ADR-0051): the position of a rail row within its run — connector clean-ends. */
function renderItem(
  item: StructuredItem,
  results: Map<string, ToolResult>,
  runPos: RailRunPosition | null,
): ReactNode {
  const k = item.itemId
  const pos = runPos ?? undefined
  switch (item.kind) {
    case 'text':
      // wrap-anywhere overflow-hidden on the row container so a long token (URL · path) does not go beyond the container.
      return (
        <ChatRow key={k} rail runPos={pos}>
          <div className="wrap-anywhere overflow-hidden">
            <Markdown markdown={item.text} />
          </div>
        </ChatRow>
      )

    case 'structured':
      // ★FIX 1 (tool_result absorption — label-independent)★: a structured with json.type==='tool_result' is absorbed into the matching tool's OUT
      //   whatever the label (user or not), so it is not rendered independently. This check must come **before** the label
      //   branch — before, it was only inside the user branch, so a tool_result with another label leaked out as standalone
      //   (contract violation). Even with no matching tool the absorption rule is the same (it is drawn nowhere).
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
        // When the content is empty (opus encrypted thinking — only a signature comes) an empty "Thought" clutter would show on every response, so
        //   nothing is drawn (null). rowKindOf must also return 'skip' by the same isEmptyThinking check so that
        //   the rail calculation and the DOM agree (ADR-0051).
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
      // The per-message token chip is not shown (the accumulated item kind itself is kept — only the render is omitted).
      return null

    case 'error':
      return (
        <ChatRow key={k} rail tone="error" runPos={pos}>
          <RowHeader icon={AlertTriangle} title="Error" tone="error" />
          <div className="text-red-500 whitespace-pre-wrap break-words">{item.message}</div>
        </ChatRow>
      )

    case 'outcome':
      // ★A normal completion does not come here★ — the accumulator leaves only a divider with no marker (so an ordinary ending does not
      //   look like a warning). So what this row draws is only the three: failure · interrupt · unknown.
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
      // Turn boundary — no dotted rail/divider, only a very faint vertical spacer (avoid a conspicuous divider).
      return <div key={k} aria-hidden className="h-3" />
  }
}

export function StructuredTextView({
  items,
  streaming = false,
  slotId,
  onGroupToggle,
}: {
  items: StructuredItem[]
  streaming?: boolean
  /** The slot (`store/toolGroupStore.ts`) that holds the tool bundle expansion state — without it the bundle is drawn with no chosen value and pressing the head changes nothing. */
  slotId?: string
  /**
   * A human pressed the tool bundle head to expand it — not called on collapse. `isLast` = is that bundle the last bundle of the list.
   * TRD S21-chat-ux §4-5: expanding a non-last bundle releases bottom-follow (if kept pinned, the pressed head is pushed up off the screen).
   */
  onGroupToggle?: (isLast: boolean) => void
}) {
  const results = buildToolResultMap(items)
  // ADR-0241: the vendor error id is built once per render and shared by the summaries of all bundles.
  const vendorErrorIds = vendorErrorIdsOf(results)
  // ADR-0239: bundling is derived during render — neither the accumulator nor the backend makes bundles, and `ToolGroupRow` reads the expansion state.
  const rows = groupToolRuns(items, streaming, rowKindOf)
  // ADR-0051: the rail run positions are extracted in advance by pure computation (derived during render — not state/effect, keeping ADR-0050 purity).
  //   ★Counted by row list, not by item★ — a bundle is one rail row (assistant) and members do not draw the rail.
  const kinds = rows.map((row) => (row.kind === 'toolGroup' ? 'assistant' : rowKindOf(row.item)))
  const positions = computeRailRunPositions(kinds)
  let lastGroup = -1
  let lastDrawn = -1
  rows.forEach((row, i) => {
    if (row.kind === 'toolGroup') lastGroup = i
    if (kinds[i] !== 'skip') lastDrawn = i
  })
  const lastRow = lastDrawn >= 0 ? rows[lastDrawn] : undefined
  // Margin below the last row — if the turn divider (h-3) is the end it is the margin, otherwise an empty block of the same height is placed. The moment the turn ends and the divider
  //   attaches, it just swaps places with this block and the list height stays the same. ★A real height block, not padding★: Radix ScrollArea's
  //   display:table wrapper does not count the last element's bottom padding in scrollHeight, so while following pinned to the bottom (ADR-0242) the padding is
  //   pushed out of the viewport and not seen (measured 2026-07-08).
  const bottomSpacer = lastRow !== undefined && !(lastRow.kind === 'item' && lastRow.item.kind === 'separator')
  const renderMember = (item: StructuredItem) => renderGroupMember(item, results)
  return (
    // Scopes the chat root font/line-height to here only (the rest of the app, such as the tree and terminal slots, is unaffected).
    //   Pulling them out as CSS variables is for LLM control (ADR-0051).
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
      {bottomSpacer && <div aria-hidden data-chat-bottom-spacer="1" className="h-3" />}
    </div>
  )
}
