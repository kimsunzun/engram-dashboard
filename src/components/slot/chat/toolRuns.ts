// ADR-0239: pure logic of chat tool row bundling — `groupToolRuns` · `summarizeGroup` · bundle key. Derived during render (no state; neither the accumulator nor the backend makes bundles; expansion state lives in `store/toolGroupStore.ts`).
// ADR-0241: the error/decline decision of one call (`toolCallVerdict`) and the decline marker switch (`DECLINED_MARK`) live here so the summary line and the expanded row's badge use one decision.
// ★The vendor result body is not parsed here★ — `rowKind` and `vendorErrorIds` take the results of `StructuredTextView`'s `rowKindOf` · `buildToolResultMap`; deciding in two places would split the rail calculation and the DOM (ADR-0051).

import {
  normalizeToolCategory,
  type StructuredItem,
  type ToolCategory,
  type ToolItem,
  type ToolResultMark,
} from '../structuredAccumulator'
import type { ChatRowKind } from './railPositions'

/**
 * How a decline marker (`'declined'` · `'refused'`) is shown. `'own'` = separate from errors (orange badge · reason line by source · summary "N declined", not counted as errors); `'error'` = counted as errors (red badge · no reason line · summary "N errors").
 */
export type DeclinedMarkMode = 'own' | 'error'

// ADR-0241: shown separately; `'error'` switches back to the rejected alternative (counting a decline as an error too).
export const DECLINED_MARK: DeclinedMarkMode = 'own'

/**
 * One line of the conversation list — one item as is, or one bundle folding two or more consecutive tool calls.
 * - `members` = items drawn when expanded (tool rows · non-empty thoughts), original order; undrawn items are absent.
 * - `calls` = the tool rows among `members`.
 * - `key` = expansion-state key (`tool:<backend id of the first call>`, else `item:<itemId>`); the same event sequence gives the same key, also after a replay that rewinds the same ring. ★A resubscribe replay starting after the ring pushed items out has a different event sequence★ — `itemId` shifts so the `item:` key changes (and `tool:` too if the head call was pushed out), losing the chosen expansion — ★and since the store is not cleared on resubscribe, a shifted `item:<n>` overlapping a **different** bundle's key makes that bundle inherit the old expansion★.
 * - `live` = the turn is open and only thoughts are drawn after this bundle — the head is progressive and, with no chosen expansion, the last call is shown as one line under it (ADR-0263 decision 1).
 */
export type DisplayRow =
  | { kind: 'item'; item: StructuredItem }
  | { kind: 'toolGroup'; key: string; members: StructuredItem[]; calls: ToolItem[]; live: boolean }

type RunRole = 'hidden' | 'thought' | 'break'

function runRoleOf(item: StructuredItem, rowKind: (item: StructuredItem) => ChatRowKind): RunRole {
  if (rowKind(item) === 'skip') return 'hidden'
  // ADR-0239: non-empty thoughts between tools are absorbed into the bundle (the alternative, cutting, would make this 'break').
  if (item.kind === 'structured' && item.label === 'thinking') return 'thought'
  return 'break'
}

/**
 * Item list → conversation line list; two or more continuing tool rows fold into one bundle.
 * A bundle runs from the first to the last tool row. Undrawn items (`rowKind` `'skip'`) and non-empty thoughts do not break it; other drawn items (text · user bubble · turn divider · outcome · interruption display · error · unknown event · escape-hatch items) do. Thoughts before the first / after the last tool are not members. Undrawn items inside a bundle are absent from the return; all others appear once, in original order.
 * @param turnOpen is the turn running — `StructuredTextView`'s `streaming`.
 * @param rowKind row kind of an item — pass `StructuredTextView`'s `rowKindOf` as is.
 */
export function groupToolRuns(
  items: readonly StructuredItem[],
  turnOpen: boolean,
  rowKind: (item: StructuredItem) => ChatRowKind,
): DisplayRow[] {
  const rows: DisplayRow[] = []
  const usedKeys = new Set<string>()
  let i = 0
  while (i < items.length) {
    const first = items[i]
    if (first.kind !== 'tool') {
      rows.push({ kind: 'item', item: first })
      i += 1
      continue
    }
    const members: StructuredItem[] = [first]
    const calls: ToolItem[] = [first]
    let lastCall = i
    // A thought that came after the previous tool — it becomes a member only if another tool comes after it.
    let thoughts: StructuredItem[] = []
    let j = i + 1
    for (; j < items.length; j += 1) {
      const item = items[j]
      if (item.kind === 'tool') {
        members.push(...thoughts, item)
        thoughts = []
        calls.push(item)
        lastCall = j
        continue
      }
      const role = runRoleOf(item, rowKind)
      if (role === 'break') break
      if (role === 'thought') thoughts.push(item)
    }
    if (calls.length < 2) {
      rows.push({ kind: 'item', item: first })
      i += 1
      continue
    }
    // ADR-0239: a trailing thought does not mean finished — a following tool would flicker the progressive indicator and last call line.
    const live = turnOpen && j === items.length
    rows.push({ kind: 'toolGroup', key: groupKeyOf(first, usedKeys), members, calls, live })
    i = lastCall + 1
  }
  return rows
}

function groupKeyOf(first: ToolItem, used: Set<string>): string {
  // ADR-0239: the accumulator rebuilds the same event sequence with the same backend id · `itemId`, so the key survives a replay (limit = `DisplayRow.key`). The `itemId` fallback applies when the first call has no id or an earlier bundle already uses that id key (two bundles sharing a key would tangle expansion state and React key; the backend has never put one id on two calls).
  const byId = typeof first.id === 'string' && first.id.length > 0 ? `tool:${first.id}` : null
  const key = byId !== null && !used.has(byId) ? byId : `item:${first.itemId}`
  used.add(key)
  return key
}

/** Is it the shape of a bundle key that `groupToolRuns` builds — `tool:<backend id>` or `item:<itemId>`. */
export function isToolGroupKey(value: string): boolean {
  return /^(?:tool:[\s\S]+|item:\d+)$/.test(value)
}

/**
 * One call's look in the bundle summary and row badge. `null` = no outcome to show — ★not success★ (an unknown or not-yet-come end falls here too).
 */
export type ToolCallVerdict = 'error' | 'declined' | null

/**
 * @param mark the end marker the accumulator attached (a missing field is the same as no marker).
 * @param vendorError did the vendor result body say it is an error — `isError` of `buildToolResultMap` (claude).
 */
export function toolCallVerdict(
  mark: ToolResultMark | null | undefined,
  vendorError: boolean,
  declinedMark: DeclinedMarkMode = DECLINED_MARK,
): ToolCallVerdict {
  if (mark === 'failed') return 'error'
  // ADR-0241: a decline was not executed, so it is not an error. The marker precedes the vendor body: it is the neutral end result, and vendor body parsing substitutes for backends not yet carrying it.
  if (mark === 'declined' || mark === 'refused') return declinedMark === 'error' ? 'error' : 'declined'
  return vendorError ? 'error' : null
}

/**
 * Message key of a decline row's reason line, by source (`'refused'` = our refusal, `'declined'` = any other). `null` if not a decline or `declinedMark` is `'error'`.
 * ★Tell the source by this key, not by wording★: the other-decline reason wording equals the tail of our refusal reason.
 */
export function declinedReasonKey(
  mark: ToolResultMark | null | undefined,
  declinedMark: DeclinedMarkMode = DECLINED_MARK,
): 'chat.toolRefusedReason' | 'chat.toolDeclinedReason' | null {
  if (declinedMark === 'error') return null
  if (mark === 'refused') return 'chat.toolRefusedReason'
  if (mark === 'declined') return 'chat.toolDeclinedReason'
  return null
}

/** Call ids whose vendor result body said error — `results` = the result of `StructuredTextView`'s `buildToolResultMap`. */
export function vendorErrorIdsOf(results: ReadonlyMap<string, { readonly isError: boolean }>): Set<string> {
  const ids = new Set<string>()
  for (const [id, result] of results) if (result.isError) ids.add(id)
  return ids
}

export interface ToolGroupSummary {
  /** Calls per kind (0 omitted); order: search · read · list · edit · command · web · agent · MCP · other. */
  counts: ReadonlyArray<readonly [ToolCategory, number]>
  errors: number
  /** Calls that were not executed — they do not overlap with `errors`. If `declinedMark` is `'error'` it is always 0 and that count goes into `errors`. */
  declined: number
}

/**
 * Bundle head summary counts — a call falls into at most one of `errors` · `declined` (`toolCallVerdict`).
 * @param vendorErrorIds call ids whose vendor result body is an error (`vendorErrorIdsOf`); codex arrives as a marker instead.
 */
export function summarizeGroup(
  calls: readonly ToolItem[],
  vendorErrorIds: ReadonlySet<string>,
  declinedMark: DeclinedMarkMode = DECLINED_MARK,
): ToolGroupSummary {
  // ADR-0239: key order = summary order; a `Record`, so adding/removing a `ToolCategory` word turns tsc red here.
  const table: Record<ToolCategory, number> = {
    Search: 0,
    Read: 0,
    List: 0,
    Edit: 0,
    Command: 0,
    Web: 0,
    Agent: 0,
    Mcp: 0,
    Other: 0,
  }
  let errors = 0
  let declined = 0
  for (const call of calls) {
    table[normalizeToolCategory(call.category)] += 1
    const vendorError = typeof call.id === 'string' && vendorErrorIds.has(call.id)
    const verdict = toolCallVerdict(call.resultMark, vendorError, declinedMark)
    if (verdict === 'error') errors += 1
    else if (verdict === 'declined') declined += 1
  }
  const counts = (Object.keys(table) as ToolCategory[])
    .filter(category => table[category] > 0)
    .map(category => [category, table[category]] as const)
  return { counts, errors, declined }
}

/** Tone of one cell of the summary line — `'error'` = red tone · `'declined'` = orange warning token (`var(--status-blocked)`) · `'default'` = default. */
export type SummaryTone = 'default' | 'error' | 'declined'

export interface SummaryPart {
  key: `chat.toolGroup${ToolCategory}` | 'chat.toolGroupErrors' | 'chat.toolGroupDeclined'
  count: number
  tone: SummaryTone
}

/** Cells of the summary line — order = per-kind count (`counts` order) → error → decline. There is no cell with 0. Cells are joined with ` · `. */
export function summaryParts(summary: ToolGroupSummary): SummaryPart[] {
  const parts: SummaryPart[] = []
  for (const [category, count] of summary.counts) {
    parts.push({ key: `chat.toolGroup${category}` as const, count, tone: 'default' })
  }
  // ADR-0241: declines follow errors, counted separately from the error number.
  if (summary.errors > 0) parts.push({ key: 'chat.toolGroupErrors', count: summary.errors, tone: 'error' })
  if (summary.declined > 0) parts.push({ key: 'chat.toolGroupDeclined', count: summary.declined, tone: 'declined' })
  return parts
}
