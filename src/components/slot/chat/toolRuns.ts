// ADR-0239: 채팅 도구 행 묶기의 순수 로직 — 묶음 짓기(`groupToolRuns`) · 묶음 요약 셈(`summarizeGroup`) · 묶음 키.
//   렌더 중 파생이라 상태가 없다 — 누산기도 백엔드도 묶음을 만들지 않고, 펼침 상태는 `store/toolGroupStore.ts` 가 진다.
// ADR-0241: 한 호출이 오류인가 · 거부인가의 판정(`toolCallVerdict`)과 거부 표식 스위치(`DECLINED_MARK`)도 여기 한
//   곳에 둔다 — 묶음 요약 줄과 펼친 행의 배지가 같은 판정을 써야 둘이 어긋나지 않는다.
//
// ★벤더 결과 본문을 여기서 파싱하지 않는다★ — 행 종류(`rowKind`)와 벤더 오류(`vendorErrorIds`)는 `StructuredTextView`
//   의 `rowKindOf` · `buildToolResultMap` 결과를 받아 쓴다. 같은 판정을 두 곳에 두면 레일 계산과 DOM 이 갈린다(ADR-0051).

import {
  normalizeToolCategory,
  type StructuredItem,
  type ToolCategory,
  type ToolItem,
  type ToolResultMark,
} from '../structuredAccumulator'
import type { ChatRowKind } from './railPositions'

/**
 * 거부 표식(`'declined'` · `'refused'`)을 어떻게 보이나. `'own'` = 오류와 따로(주황 「거부됨」 배지 · 출처별 사유 줄 ·
 * 요약 「거부 N」 — 오류 수에 안 든다) · `'error'` = 오류로 함께 센다(붉은 배지 · 사유 줄 없음 · 요약 「오류 N」에 든다).
 */
export type DeclinedMarkMode = 'own' | 'error'

// ADR-0241: 사용자 결정 U2-a = 따로 표기. `'error'` 는 거부한 대안(거부를 오류로 함께 센다)으로 되돌리는 스위치다.
export const DECLINED_MARK: DeclinedMarkMode = 'own'

/**
 * 대화 목록의 한 줄 — 항목 하나 그대로이거나, 연속된 도구 호출 둘 이상을 접은 묶음 하나.
 * - `members` = 묶음이 펼쳐졌을 때 그릴 항목(도구 행 · 비지 않은 생각)만 원래 순서로. 그리지 않는 항목은 없다.
 * - `calls` = `members` 중 도구 행.
 * - `key` = 펼침 상태의 키(`tool:<첫 호출의 백엔드 id>` · 없으면 `item:<itemId>`). 같은 사건열은 같은 키다 — 같은 링을
 *   처음부터 되감는 replay 뒤에도. ★링에서 밀려난 뒤 시작하는 재구독 replay 는 사건열이 다르다★ — `itemId` 가 앞당겨져
 *   `item:` 키가 바뀌고, 묶음 머리 호출이 밀려나면 `tool:` 키도 바뀐다. 그 묶음의 고른 펼침은 잃는다 — ★그리고 스토어는
 *   재구독에 비워지지 않으므로 앞당겨진 `item:<n>` 이 **다른** 묶음의 키와 겹치면 그 묶음이 옛 묶음의 펼침을 물려받는다★.
 * - `live` = 턴이 열려 있고 이 묶음 뒤에 그리는 항목이 생각뿐이다 — 고른 값이 없으면 이 값대로 펼친다.
 */
export type DisplayRow =
  | { kind: 'item'; item: StructuredItem }
  | { kind: 'toolGroup'; key: string; members: StructuredItem[]; calls: ToolItem[]; live: boolean }

type RunRole = 'hidden' | 'thought' | 'break'

function runRoleOf(item: StructuredItem, rowKind: (item: StructuredItem) => ChatRowKind): RunRole {
  if (rowKind(item) === 'skip') return 'hidden'
  // ADR-0239: 사용자 결정 U5 — 도구 사이의 비지 않은 생각은 묶음에 흡수한다. 대안(끊는다)이면 이 줄이 'break' 다.
  if (item.kind === 'structured' && item.label === 'thinking') return 'thought'
  return 'break'
}

/**
 * 항목 목록을 대화 줄 목록으로 — 도구 행이 둘 이상 이어지면 묶음 하나로 접는다.
 *
 * 묶음은 첫 도구 행에서 마지막 도구 행까지다. 그 사이의 그리지 않는 항목(`rowKind` 가 `'skip'`)과 비지 않은 생각은
 * 묶음을 끊지 않고, 그 밖의 그리는 항목(글 · 사용자 말풍선 · 턴 구분선 · 결말 · 끊김 표시 · 오류 · 모르는 사건 · 그 밖의
 * 탈출구 항목)은 끊는다. 첫 도구 앞 · 마지막 도구 뒤의 생각은 멤버가 아니다.
 *
 * 묶음 안의 그리지 않는 항목은 반환에 없다. 그 밖의 항목은 전부 원래 순서로 한 번씩 나온다.
 *
 * @param turnOpen 턴이 도는 중인가 — `StructuredTextView` 의 `streaming`.
 * @param rowKind 항목의 행 종류 — `StructuredTextView` 의 `rowKindOf` 를 그대로 넘긴다.
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
    // 앞 도구 뒤에 온 생각 — 뒤에 도구가 또 와야 멤버가 된다.
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
    // ADR-0239: 뒤에 생각이 왔다고 접지 않는다 — 그 뒤에 도구가 이어지면 다시 펼쳐지는 깜빡임이 된다.
    const live = turnOpen && j === items.length
    rows.push({ kind: 'toolGroup', key: groupKeyOf(first, usedKeys), members, calls, live })
    i = lastCall + 1
  }
  return rows
}

function groupKeyOf(first: ToolItem, used: Set<string>): string {
  // ADR-0239: 누산기는 같은 사건열을 같은 백엔드 id · 같은 `itemId` 로 재구성한다 — 그래서 replay 뒤에도 같은 키이고
  //   고른 펼침이 살아남는다(한계 = `DisplayRow` 의 `key`). `itemId` 폴백은 첫 호출에 id 가 없을 때와, 앞 묶음이 이미 그 id 키를 쓸 때다(한 키를
  //   두 묶음이 나눠 쓰면 펼침 상태와 React key 가 엉킨다 — 백엔드가 한 id 를 두 호출에 싣는지는 관측된 적 없다).
  const byId = typeof first.id === 'string' && first.id.length > 0 ? `tool:${first.id}` : null
  const key = byId !== null && !used.has(byId) ? byId : `item:${first.itemId}`
  used.add(key)
  return key
}

/** `groupToolRuns` 가 짓는 묶음 키의 모양인가 — `tool:<백엔드 id>` 또는 `item:<itemId>`. */
export function isToolGroupKey(value: string): boolean {
  return /^(?:tool:[\s\S]+|item:\d+)$/.test(value)
}

/**
 * 한 호출이 묶음 요약과 행 배지에서 무엇으로 보이나. `null` = 표시할 결말이 없다 — ★성공이라는 뜻이 아니다★(끝을
 * 모르는 호출 · 끝이 안 온 호출도 여기 든다).
 */
export type ToolCallVerdict = 'error' | 'declined' | null

/**
 * @param mark 누산기가 붙인 끝 표식(칸이 없으면 표식 없음과 같다).
 * @param vendorError 벤더 결과 본문이 오류라고 말했나 — `buildToolResultMap` 의 `isError`(claude).
 */
export function toolCallVerdict(
  mark: ToolResultMark | null | undefined,
  vendorError: boolean,
  declinedMark: DeclinedMarkMode = DECLINED_MARK,
): ToolCallVerdict {
  if (mark === 'failed') return 'error'
  // ADR-0241: 거부는 실행되지 않은 것이라 오류가 아니다. 표식이 벤더 본문보다 먼저인 것은 표식이 중립 끝 결과이고
  //   벤더 본문 파싱은 그 결과를 아직 안 싣는 백엔드의 대체이기 때문이다.
  if (mark === 'declined' || mark === 'refused') return declinedMark === 'error' ? 'error' : 'declined'
  return vendorError ? 'error' : null
}

/**
 * 거부 행의 사유 줄 문구 키 — 출처별로 갈린다(`'refused'` = 우리 거절 · `'declined'` = 그 밖의 거부). 거부가
 * 아니거나 `declinedMark` 가 `'error'` 면 `null`(사유 줄 없음).
 * ★출처는 이 키로 가른다 — 문구로 가르지 말 것★: 그 밖의 거부 사유 문구는 우리 거절 사유 문구의 꼬리와 같다.
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

/** 벤더 결과 본문이 오류라고 말한 호출 id — `results` = `StructuredTextView` 의 `buildToolResultMap` 결과. */
export function vendorErrorIdsOf(results: ReadonlyMap<string, { readonly isError: boolean }>): Set<string> {
  const ids = new Set<string>()
  for (const [id, result] of results) if (result.isError) ids.add(id)
  return ids
}

export interface ToolGroupSummary {
  /** 종류별 호출 수 — 0 인 종류는 빠지고, 순서는 검색 · 읽기 · 목록 · 편집 · 명령 · 웹 · 에이전트 · MCP · 기타. */
  counts: ReadonlyArray<readonly [ToolCategory, number]>
  errors: number
  /** 실행되지 않은 호출 — `errors` 와 겹치지 않는다. `declinedMark` 가 `'error'` 면 늘 0 이고 그 수는 `errors` 에 든다. */
  declined: number
}

/**
 * 묶음 머리 요약 줄의 수 — 한 호출은 `errors` · `declined` 중 많아야 하나에 든다(`toolCallVerdict`).
 * @param vendorErrorIds 벤더 결과 본문이 오류인 호출 id(`vendorErrorIdsOf`). codex 는 여기 없고 표식으로 온다.
 */
export function summarizeGroup(
  calls: readonly ToolItem[],
  vendorErrorIds: ReadonlySet<string>,
  declinedMark: DeclinedMarkMode = DECLINED_MARK,
): ToolGroupSummary {
  // ADR-0239: 키 순서가 곧 요약 순서다. `Record` 라 `ToolCategory` 에 낱말이 늘거나 줄면 여기서 tsc 가 빨갛다.
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

/** 요약 줄 한 칸의 톤 — `'error'` = 붉은 톤 · `'declined'` = 주황 경고 토큰(`var(--status-blocked)`) · `'default'` = 기본. */
export type SummaryTone = 'default' | 'error' | 'declined'

export interface SummaryPart {
  key: `chat.toolGroup${ToolCategory}` | 'chat.toolGroupErrors' | 'chat.toolGroupDeclined'
  count: number
  tone: SummaryTone
}

/** 요약 줄의 칸 — 순서 = 종류별 수(`counts` 순서) → 오류 → 거부. 0 인 칸은 없다. 칸 사이는 ` · ` 로 잇는다. */
export function summaryParts(summary: ToolGroupSummary): SummaryPart[] {
  const parts: SummaryPart[] = []
  for (const [category, count] of summary.counts) {
    parts.push({ key: `chat.toolGroup${category}` as const, count, tone: 'default' })
  }
  // ADR-0241: 거부는 오류 뒤 — 오류 수에 안 든 채 따로 센다(사용자 결정 U2-a).
  if (summary.errors > 0) parts.push({ key: 'chat.toolGroupErrors', count: summary.errors, tone: 'error' })
  if (summary.declined > 0) parts.push({ key: 'chat.toolGroupDeclined', count: summary.declined, tone: 'declined' })
  return parts
}
