// Tool row bundling · summary count · decline marker switch test (ADR-0239 · ADR-0241 · TRD S21-chat-ux §4-6 · §4-7 ⑨ frontend).

import { describe, expect, it } from 'vitest'

import { t } from '../../../i18n'
import { StructuredEventAccumulator, type StructuredItem, type ToolItem } from '../structuredAccumulator'
import type { ChatRowKind } from './railPositions'
import {
  DECLINED_MARK,
  declinedReasonKey,
  groupToolRuns,
  isToolGroupKey,
  summarizeGroup,
  summaryParts,
  toolCallVerdict,
  vendorErrorIdsOf,
  type DisplayRow,
  type ToolGroupSummary,
} from './toolRuns'

// ── Fixtures ──

/**
 * A decision standing in for `StructuredTextView`'s `rowKindOf` — all the bundling function receives is its result. The same three as the real one are treated as
 * `'skip'`: usage · the vendor result carrier row (`tool_result`) · empty thoughts.
 */
function rowKind(item: StructuredItem): ChatRowKind {
  switch (item.kind) {
    case 'usage':
      return 'skip'
    case 'separator':
      return 'boundary'
    case 'structured': {
      const parsed = JSON.parse(item.json) as { type?: unknown; thinking?: unknown }
      if (parsed.type === 'tool_result') return 'skip'
      if (item.label === 'user') return 'boundary'
      if (item.label === 'thinking' && String(parsed.thinking ?? '').trim() === '') return 'skip'
      return 'assistant'
    }
    default:
      return 'assistant'
  }
}

let nextItemId = 0
function tool(id: string | null, extra: Partial<ToolItem> = {}): ToolItem {
  return { kind: 'tool', name: 'Read', argsJson: '{}', id, category: 'Read', resultMark: null, itemId: nextItemId++, ...extra }
}
const text = (s: string): StructuredItem => ({ kind: 'text', text: s, itemId: nextItemId++ })
const thought = (s: string): StructuredItem => ({
  kind: 'structured',
  label: 'thinking',
  json: JSON.stringify({ thinking: s }),
  itemId: nextItemId++,
})
const carrier = (toolUseId: string, isError = false): StructuredItem => ({
  kind: 'structured',
  label: 'user',
  json: JSON.stringify({ type: 'tool_result', tool_use_id: toolUseId, content: 'out', is_error: isError }),
  itemId: nextItemId++,
})
const bubble = (s: string): StructuredItem => ({
  kind: 'structured',
  label: 'user',
  json: JSON.stringify({ type: 'text', text: s }),
  itemId: nextItemId++,
})
const usage = (): StructuredItem => ({ kind: 'usage', inputTokens: 1, outputTokens: 1, itemId: nextItemId++ })
const separator = (): StructuredItem => ({ kind: 'separator', itemId: nextItemId++ })
const outcome = (): StructuredItem => ({ kind: 'outcome', outcome: 'interrupted', detail: null, itemId: nextItemId++ })
const errorRow = (): StructuredItem => ({ kind: 'error', message: 'boom', itemId: nextItemId++ })
const unsupported = (): StructuredItem => ({ kind: 'unsupported', count: 1, itemId: nextItemId++ })
const escape = (): StructuredItem => ({ kind: 'structured', label: 'plan', json: '{"steps":[]}', itemId: nextItemId++ })

/** Line list in an easy-to-compare shape — item line = itemId · bundle = key · member itemId · live. */
function shape(rows: DisplayRow[]): unknown[] {
  return rows.map(r =>
    r.kind === 'item'
      ? r.item.itemId
      : { key: r.key, members: r.members.map(m => m.itemId), calls: r.calls.map(c => c.itemId), live: r.live },
  )
}

function groupsOf(rows: DisplayRow[]): Extract<DisplayRow, { kind: 'toolGroup' }>[] {
  return rows.filter((r): r is Extract<DisplayRow, { kind: 'toolGroup' }> => r.kind === 'toolGroup')
}

// ── groupToolRuns ──

describe('groupToolRuns — building groups', () => {
  it('one tool is not grouped — that row as is', () => {
    const a = tool('a')
    const after = thought('then')
    expect(shape(groupToolRuns([a, after], true, rowKind))).toEqual([a.itemId, after.itemId])
  })

  it('two consecutive tools make one group — key = backend id of the first call', () => {
    const a = tool('a')
    const b = tool('b')
    expect(shape(groupToolRuns([a, b], false, rowKind))).toEqual([
      { key: 'tool:a', members: [a.itemId, b.itemId], calls: [a.itemId, b.itemId], live: false },
    ])
  })

  it('even with non-drawn rows in between (vendor result carrier · usage · empty thought) it continues, and they appear in neither members nor lines', () => {
    const a = tool('a')
    const b = tool('b')
    const c = tool('c')
    const items = [a, carrier('a'), usage(), b, thought('   '), carrier('b'), c]
    expect(shape(groupToolRuns(items, false, rowKind))).toEqual([
      { key: 'tool:a', members: [a.itemId, b.itemId, c.itemId], calls: [a.itemId, b.itemId, c.itemId], live: false },
    ])
  })

  it('a non-empty thought between tools is absorbed as a member (U5)', () => {
    const a = tool('a')
    const mid = thought('look at b next')
    const b = tool('b')
    expect(shape(groupToolRuns([a, mid, b], false, rowKind))).toEqual([
      { key: 'tool:a', members: [a.itemId, mid.itemId, b.itemId], calls: [a.itemId, b.itemId], live: false },
    ])
  })

  it.each([
    ['text', text],
    ['user bubble', bubble],
    ['turn divider', separator],
    ['outcome', outcome],
    ['error', errorRow],
    ['unknown event', unsupported],
    ['other escape-hatch item', escape],
  ] as const)('a %s in between breaks it — one tool on each side so there is no group', (_label, make) => {
    const a = tool('a')
    const mid = (make as (s: string) => StructuredItem)('x')
    const b = tool('b')
    expect(shape(groupToolRuns([a, mid, b], true, rowKind))).toEqual([a.itemId, mid.itemId, b.itemId])
  })

  it('if the rows on both sides of a breaker each have two or more it makes two groups — different keys', () => {
    const [a, b] = [tool('a'), tool('b')]
    const mid = text('mid')
    const [c, d] = [tool('c'), tool('d')]
    const rows = groupToolRuns([a, b, mid, c, d], false, rowKind)
    expect(rows.map(r => (r.kind === 'item' ? r.item.itemId : r.key))).toEqual(['tool:a', mid.itemId, 'tool:c'])
  })

  it('thoughts before the first tool and after the last tool are not members and stay as lines in place', () => {
    const before = thought('plan')
    const [a, b] = [tool('a'), tool('b')]
    const after = thought('summarize')
    const hiddenAfter = usage()
    const rows = groupToolRuns([before, a, b, after, hiddenAfter], false, rowKind)
    expect(shape(rows)).toEqual([
      before.itemId,
      { key: 'tool:a', members: [a.itemId, b.itemId], calls: [a.itemId, b.itemId], live: false },
      after.itemId,
      hiddenAfter.itemId,
    ])
  })

  it('removing the non-drawn rows inside a group, every item comes out once in the original order', () => {
    const items = [text('hi'), tool('a'), carrier('a'), tool('b'), thought('t'), text('done'), separator(), tool('c')]
    const seen = groupToolRuns(items, false, rowKind).flatMap(r =>
      r.kind === 'item' ? [r.item.itemId] : r.members.map(m => m.itemId),
    )
    const carrierId = items[2].itemId
    expect(seen).toEqual(items.map(i => i.itemId).filter(id => id !== carrierId))
  })
})

describe('groupToolRuns — live (running — head tense)', () => {
  it('mid-turn · if the group is the tail it is live', () => {
    const rows = groupToolRuns([text('go'), tool('a'), tool('b')], true, rowKind)
    expect(groupsOf(rows)[0].live).toBe(true)
  })

  it('mid-turn · if only thoughts and non-drawn rows follow it stays live — a thought arriving is not read as finished', () => {
    const rows = groupToolRuns([tool('a'), tool('b'), thought('hmm'), usage(), thought('  ')], true, rowKind)
    expect(groupsOf(rows)[0].live).toBe(true)
  })

  it('if a breaker row comes after it is not live', () => {
    const rows = groupToolRuns([tool('a'), tool('b'), thought('hmm'), text('answer')], true, rowKind)
    expect(groupsOf(rows)[0].live).toBe(false)
  })

  it('if the turn is closed even the tail is not live', () => {
    const rows = groupToolRuns([tool('a'), tool('b')], false, rowKind)
    expect(groupsOf(rows)[0].live).toBe(false)
  })

  it('only the tail group is live — earlier groups are finished', () => {
    const rows = groupToolRuns([tool('a'), tool('b'), text('x'), tool('c'), tool('d')], true, rowKind)
    expect(groupsOf(rows).map(g => g.live)).toEqual([false, true])
  })

  it('even as the running tail group grows its key stays — if a tool follows a thought that thought becomes a member', () => {
    const [a, b] = [tool('a'), tool('b')]
    const mid = thought('next')
    const c = tool('c')
    const before = groupsOf(groupToolRuns([a, b, mid], true, rowKind))[0]
    const after = groupsOf(groupToolRuns([a, b, mid, c], true, rowKind))[0]
    expect(before.key).toBe(after.key)
    expect(before.members.map(m => m.itemId)).toEqual([a.itemId, b.itemId])
    expect(after.members.map(m => m.itemId)).toEqual([a.itemId, b.itemId, mid.itemId, c.itemId])
    expect([before.live, after.live]).toEqual([true, true])
  })
})

describe('groupToolRuns — key', () => {
  it('if the first call has no id it is item:<itemId>', () => {
    const a = tool(null)
    expect(groupsOf(groupToolRuns([a, tool('b')], false, rowKind))[0].key).toBe(`item:${a.itemId}`)
  })

  it('an empty-string id is also seen as absent', () => {
    const a = tool('')
    expect(groupsOf(groupToolRuns([a, tool('b')], false, rowKind))[0].key).toBe(`item:${a.itemId}`)
  })

  it('if the first calls of two groups share the same id the later group is item:<itemId> — keys do not collide', () => {
    const first = tool('dup')
    const second = tool('dup')
    const keys = groupsOf(groupToolRuns([first, tool('x'), text('-'), second, tool('y')], false, rowKind)).map(g => g.key)
    expect(keys).toEqual(['tool:dup', `item:${second.itemId}`])
  })

  it('isToolGroupKey — accepts only the built key shape', () => {
    expect(isToolGroupKey('tool:call_1')).toBe(true)
    expect(isToolGroupKey('item:12')).toBe(true)
    expect(isToolGroupKey('call_1')).toBe(false)
    expect(isToolGroupKey('tool:')).toBe(false)
    expect(isToolGroupKey('item:x')).toBe(false)
    expect(isToolGroupKey('')).toBe(false)
  })
})

describe('groupToolRuns — replay (same event sequence = same result)', () => {
  const frames = [
    { type: 'TextDelta', text: 'start', turn_id: null, message_id: null },
    { type: 'ToolCall', name: 'Grep', args_json: '{}', id: 'g1', turn_id: null, message_id: null, category: 'Search' },
    { type: 'Structured', kind: 'thinking', json: JSON.stringify({ thinking: 'narrow it' }) },
    { type: 'ToolCall', name: 'Read', args_json: '{}', id: 'r1', turn_id: null, message_id: null, category: 'Read' },
    { type: 'ToolResult', id: 'r1', outcome: 'Failed' },
    { type: 'TextDelta', text: 'done', turn_id: null, message_id: null },
    { type: 'MessageDone', turn_id: null, message_id: null },
    { type: 'ToolCall', name: 'x', args_json: '{}', id: null, turn_id: null, message_id: null },
    { type: 'ToolCall', name: 'y', args_json: '{}', id: null, turn_id: null, message_id: null },
  ].map(f => JSON.stringify(f))

  function feedAll(acc: StructuredEventAccumulator): DisplayRow[] {
    for (const f of frames) acc.feed(f)
    return groupToolRuns(acc.snapshot(), !acc.isTurnDone(), rowKind)
  }

  it('refeeding after reset gives the same lines · keys · members — the chosen expansion survives replay', () => {
    const acc = new StructuredEventAccumulator()
    const first = feedAll(acc)
    acc.reset()
    const again = feedAll(acc)
    expect(again).toEqual(first)
    expect(groupsOf(again).map(g => g.key)).toEqual(['tool:g1', 'item:6'])
  })
})

// ── summarizeGroup · decision ──

describe('summarizeGroup — count per kind', () => {
  it('fixed order (search · read · list · edit · command · web · agent · MCP · other) · kinds with 0 drop out', () => {
    const calls = (['Other', 'Command', 'Search', 'Read', 'Search', 'Mcp'] as const).map(category =>
      tool(null, { category }),
    )
    expect(summarizeGroup(calls, new Set()).counts).toEqual([
      ['Search', 2],
      ['Read', 1],
      ['Command', 1],
      ['Mcp', 1],
      ['Other', 1],
    ])
  })

  it('if the kind field is missing or an unknown word (including prototype names) it is other', () => {
    const calls = [
      tool(null, { category: undefined }),
      tool(null, { category: 'Teleport' as never }),
      tool(null, { category: 'toString' as never }),
    ]
    expect(summarizeGroup(calls, new Set()).counts).toEqual([['Other', 3]])
  })
})

describe('summarizeGroup — error · rejection count', () => {
  it('codex failure = marker failed', () => {
    const s = summarizeGroup([tool('a', { resultMark: 'failed' }), tool('b')], new Set())
    expect([s.errors, s.declined]).toEqual([1, 0])
  })

  it('claude parse error = a call whose buildToolResultMap isError (vendorErrorIdsOf)', () => {
    const results = new Map([
      ['a', { content: 'no such file', isError: true }],
      ['b', { content: 'ok', isError: false }],
    ])
    const ids = vendorErrorIdsOf(results)
    expect([...ids]).toEqual(['a'])
    const s = summarizeGroup([tool('a'), tool('b'), tool(null)], ids)
    expect([s.errors, s.declined]).toEqual([1, 0])
  })

  it('rejection count = refused + declined — not counted toward the error count', () => {
    const s = summarizeGroup([tool('a', { resultMark: 'refused' }), tool('b', { resultMark: 'declined' }), tool('c')], new Set())
    expect([s.errors, s.declined]).toEqual([0, 2])
  })

  it('no marker · no field is not counted — it is unknown, not success', () => {
    const s = summarizeGroup([tool('a', { resultMark: undefined }), tool('b', { resultMark: null })], new Set())
    expect([s.errors, s.declined]).toEqual([0, 0])
  })

  it('one call goes in one cell only — the rejection marker comes before the vendor error', () => {
    const s = summarizeGroup([tool('a', { resultMark: 'refused' })], new Set(['a']))
    expect([s.errors, s.declined]).toEqual([0, 1])
  })

  it('a later result is counted as the value overwriting the earlier marker (via the accumulator)', () => {
    const acc = new StructuredEventAccumulator()
    const call = (id: string) =>
      acc.feed(JSON.stringify({ type: 'ToolCall', name: 'Bash', args_json: '{}', id, turn_id: null, message_id: null }))
    const result = (id: string, outcome: string) => acc.feed(JSON.stringify({ type: 'ToolResult', id, outcome }))
    call('ok-now')
    call('refused-now')
    call('failed-now')
    result('ok-now', 'Failed')
    result('ok-now', 'Completed')
    result('refused-now', 'Failed')
    result('refused-now', 'Refused')
    result('failed-now', 'Refused')
    result('failed-now', 'Failed')
    const calls = acc.snapshot().filter((i): i is ToolItem => i.kind === 'tool')
    const s = summarizeGroup(calls, new Set())
    expect([s.errors, s.declined]).toEqual([1, 1])
  })
})

describe('DECLINED_MARK — two values', () => {
  const calls = [tool('a', { resultMark: 'refused' }), tool('b', { resultMark: 'declined' }), tool('c', { resultMark: 'failed' })]

  it("the confirmed value is 'own' (user decision U2-a)", () => {
    expect(DECLINED_MARK).toBe('own')
  })

  it("'own' — rejection separately: error 1 · rejected 2 · reason key per source", () => {
    const s = summarizeGroup(calls, new Set(), 'own')
    expect([s.errors, s.declined]).toEqual([1, 2])
    expect(toolCallVerdict('refused', false, 'own')).toBe('declined')
    expect(declinedReasonKey('refused', 'own')).toBe('chat.toolRefusedReason')
    expect(declinedReasonKey('declined', 'own')).toBe('chat.toolDeclinedReason')
  })

  it("'error' — both rejections as errors: error 3 · rejected 0 · no reason line", () => {
    const s = summarizeGroup(calls, new Set(), 'error')
    expect([s.errors, s.declined]).toEqual([3, 0])
    expect(toolCallVerdict('refused', false, 'error')).toBe('error')
    expect(toolCallVerdict('declined', false, 'error')).toBe('error')
    expect(declinedReasonKey('refused', 'error')).toBeNull()
    expect(declinedReasonKey('declined', 'error')).toBeNull()
  })

  it('the default argument follows DECLINED_MARK', () => {
    expect(summarizeGroup(calls, new Set())).toEqual(summarizeGroup(calls, new Set(), DECLINED_MARK))
  })
})

describe('toolCallVerdict · declinedReasonKey', () => {
  it('failure · vendor error = error · no marker = null', () => {
    expect(toolCallVerdict('failed', false)).toBe('error')
    expect(toolCallVerdict(null, true)).toBe('error')
    expect(toolCallVerdict(undefined, true)).toBe('error')
    expect(toolCallVerdict(null, false)).toBeNull()
    expect(toolCallVerdict(undefined, false)).toBeNull()
  })

  it('if not a rejection there is no reason key', () => {
    expect(declinedReasonKey('failed')).toBeNull()
    expect(declinedReasonKey(null)).toBeNull()
    expect(declinedReasonKey(undefined)).toBeNull()
  })
})

describe('summaryParts — summary line', () => {
  const line = (s: ToolGroupSummary): string => summaryParts(s).map(p => t(p.key, { count: String(p.count) })).join(' · ')

  it('order kind → error → rejection — 「… · error 1 · rejected 1」', () => {
    const calls = [
      tool('a', { category: 'Search', resultMark: 'declined' }),
      tool('b', { category: 'Read', resultMark: 'failed' }),
      tool('c', { category: 'Search' }),
    ]
    const s = summarizeGroup(calls, new Set())
    expect(line(s)).toBe('검색 2 · 읽기 1 · 오류 1 · 거부 1')
    expect(summaryParts(s).map(p => p.tone)).toEqual(['default', 'default', 'error', 'declined'])
  })

  it('with only rejections there is no 「error」 cell', () => {
    const s = summarizeGroup([tool('a', { resultMark: 'refused' }), tool('b', { resultMark: 'declined' })], new Set())
    expect(line(s)).toBe('읽기 2 · 거부 2')
  })

  it("if DECLINED_MARK='error' rejections go to the error cell — 「error 2」 · no rejection cell", () => {
    const s = summarizeGroup([tool('a', { resultMark: 'refused' }), tool('b', { resultMark: 'declined' })], new Set(), 'error')
    expect(line(s)).toBe('읽기 2 · 오류 2')
  })

  it('all nine kinds have a text key', () => {
    const all = (['Search', 'Read', 'List', 'Edit', 'Command', 'Web', 'Agent', 'Mcp', 'Other'] as const).map(category =>
      tool(null, { category }),
    )
    const texts = summaryParts(summarizeGroup(all, new Set())).map(p => t(p.key, { count: String(p.count) }))
    expect(texts).toEqual(['검색 1', '읽기 1', '목록 1', '편집 1', '명령 1', '웹 1', '에이전트 1', 'MCP 1', '기타 1'])
  })
})
