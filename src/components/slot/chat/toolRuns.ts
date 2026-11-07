// ADR-0239: pure logic of chat tool row bundling — bundle building (`groupToolRuns`) · bundle summary count (`summarizeGroup`) · bundle key.
//   It is derived during render so it has no state — neither the accumulator nor the backend makes bundles, and expansion state is held by `store/toolGroupStore.ts`.
// ADR-0241: the decision of whether one call is an error or a decline (`toolCallVerdict`) and the decline marker switch (`DECLINED_MARK`) are also kept in this one
//   place — the bundle summary line and the expanded row's badge must use the same decision so the two do not diverge.
//
// ★The vendor result body is not parsed here★ — the row kind (`rowKind`) and vendor errors (`vendorErrorIds`) take the results of
//   `StructuredTextView`'s `rowKindOf` · `buildToolResultMap`. Keeping the same decision in two places would split the rail calculation and the DOM (ADR-0051).

import {
  normalizeToolCategory,
  type StructuredItem,
  type ToolCategory,
  type ToolItem,
  type ToolResultMark,
} from '../structuredAccumulator'
import type { ChatRowKind } from './railPositions'

/**
 * How a decline marker (`'declined'` · `'refused'`) is shown. `'own'` = separate from errors (orange 「거부됨」 (declined) badge · reason line by source ·
 * summary 「거부 N」 (N declined) — does not count toward the error number) · `'error'` = counted together as errors (red badge · no reason line · counts in the summary 「오류 N」 (N errors)).
 */
export type DeclinedMarkMode = 'own' | 'error'

// ADR-0241: user decision U2-a = shown separately. `'error'` is the switch to go back to the rejected alternative (counting a decline as an error too).
export const DECLINED_MARK: DeclinedMarkMode = 'own'

/**
 * One line of the conversation list — either one item as it is, or one bundle that folds two or more consecutive tool calls.
 * - `members` = only the items to draw when the bundle is expanded (tool rows · non-empty thoughts), in original order. There are no items that are not drawn.
 * - `calls` = the tool rows among `members`.
 * - `key` = the key of the expansion state (`tool:<첫 호출의 백엔드 id>` — backend id of the first call · otherwise `item:<itemId>`). The same event sequence has the same key — even after a replay
 *   that rewinds the same ring from the start. ★A resubscribe replay that starts after being pushed out of the ring has a different event sequence★ — `itemId` shifts forward so
 *   the `item:` key changes, and if the bundle's head call is pushed out the `tool:` key changes too. That bundle's chosen expansion is lost — ★and since the store is
 *   not cleared on resubscribe, if the shifted `item:<n>` overlaps the key of a **different** bundle, that bundle inherits the old bundle's expansion★.
 * - `live` = the turn is open and the only item drawn after this bundle is thoughts — the head is progressive, and with no chosen expansion the last call as one
 *   line is shown under the head (ADR-0263 decision 1).
 */
export type DisplayRow =
  | { kind: 'item'; item: StructuredItem }
  | { kind: 'toolGroup'; key: string; members: StructuredItem[]; calls: ToolItem[]; live: boolean }

type RunRole = 'hidden' | 'thought' | 'break'

function runRoleOf(item: StructuredItem, rowKind: (item: StructuredItem) => ChatRowKind): RunRole {
  if (rowKind(item) === 'skip') return 'hidden'
  // ADR-0239: user decision U5 — non-empty thoughts between tools are absorbed into the bundle. With the alternative (cut) this line is 'break'.
  if (item.kind === 'structured' && item.label === 'thinking') return 'thought'
  return 'break'
}

/**
 * Item list to conversation line list — if two or more tool rows continue, they fold into one bundle.
 *
 * A bundle runs from the first tool row to the last tool row. Items not drawn between them (`rowKind` is `'skip'`) and non-empty thoughts
 * do not break the bundle, and other drawn items (text · user bubble · turn divider · outcome · interruption display · error · unknown event · other
 * escape-hatch items) do. Thoughts before the first tool · after the last tool are not members.
 *
 * Items not drawn inside a bundle are absent from the return. All other items appear once each in original order.
 *
 * @param turnOpen is the turn running — `StructuredTextView`'s `streaming`.
 * @param rowKind the row kind of an item — pass `StructuredTextView`'s `rowKindOf` as it is.
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
    // ADR-0239: it is not seen as finished because a thought came after — if a tool follows it, the progressive indicator and last call line would flicker off and
    //   on.
    const live = turnOpen && j === items.length
    rows.push({ kind: 'toolGroup', key: groupKeyOf(first, usedKeys), members, calls, live })
    i = lastCall + 1
  }
  return rows
}

function groupKeyOf(first: ToolItem, used: Set<string>): string {
  // ADR-0239: the accumulator rebuilds the same event sequence with the same backend id · same `itemId` — so even after a replay the key is the same and
  //   the chosen expansion survives (limit = `key` of `DisplayRow`). The `itemId` fallback applies when the first call has no id and when an earlier bundle already uses that id key (if two
  //   bundles shared one key, expansion state and React key would tangle — it has never been observed that the backend puts one id on two calls).
  const byId = typeof first.id === 'string' && first.id.length > 0 ? `tool:${first.id}` : null
  const key = byId !== null && !used.has(byId) ? byId : `item:${first.itemId}`
  used.add(key)
  return key
}

/** Is it the shape of a bundle key that `groupToolRuns` builds — `tool:<백엔드 id>` (backend id) or `item:<itemId>`. */
export function isToolGroupKey(value: string): boolean {
  return /^(?:tool:[\s\S]+|item:\d+)$/.test(value)
}

/**
 * What one call looks like in the bundle summary and row badge. `null` = there is no outcome to show — ★it does not mean success★ (a call
 * whose end is unknown or whose end has not come falls in here too).
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
  // ADR-0241: a decline was not executed so it is not an error. The marker comes before the vendor body because the marker is a neutral end result
  //   and vendor body parsing is the substitute for backends that do not yet carry that result.
  if (mark === 'declined' || mark === 'refused') return declinedMark === 'error' ? 'error' : 'declined'
  return vendorError ? 'error' : null
}

/**
 * Message key of the reason line of a decline row — split by source (`'refused'` = our refusal · `'declined'` = any other decline). If it is not a decline
 * or `declinedMark` is `'error'`, `null` (no reason line).
 * ★The source is told apart by this key — do not tell it apart by wording★: the wording of the other-decline reason is the same as the tail of our refusal reason wording.
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
  /** Number of calls per kind — kinds with 0 are left out, and the order is search · read · list · edit · command · web · agent · MCP · other. */
  counts: ReadonlyArray<readonly [ToolCategory, number]>
  errors: number
  /** Calls that were not executed — they do not overlap with `errors`. If `declinedMark` is `'error'` it is always 0 and that count goes into `errors`. */
  declined: number
}

/**
 * Counts in the bundle head summary line — one call falls into at most one of `errors` · `declined` (`toolCallVerdict`).
 * @param vendorErrorIds call ids whose vendor result body is an error (`vendorErrorIdsOf`). codex is not here and comes as a marker.
 */
export function summarizeGroup(
  calls: readonly ToolItem[],
  vendorErrorIds: ReadonlySet<string>,
  declinedMark: DeclinedMarkMode = DECLINED_MARK,
): ToolGroupSummary {
  // ADR-0239: key order is the summary order. It is a `Record`, so if a word is added to or removed from `ToolCategory`, tsc goes red here.
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
  // ADR-0241: declines come after errors — counted separately without entering the error number (user decision U2-a).
  if (summary.errors > 0) parts.push({ key: 'chat.toolGroupErrors', count: summary.errors, tone: 'error' })
  if (summary.declined > 0) parts.push({ key: 'chat.toolGroupDeclined', count: summary.declined, tone: 'declined' })
  return parts
}
