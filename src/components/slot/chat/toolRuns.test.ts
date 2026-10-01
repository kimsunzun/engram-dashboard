// 도구 행 묶기 · 요약 셈 · 거부 표식 스위치 시험(ADR-0239 · ADR-0241 · TRD S21-chat-ux §4-6 · §4-7 ⑨ 프론트).

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

// ── 픽스처 ──

/**
 * `StructuredTextView` 의 `rowKindOf` 를 대신하는 판정 — 묶기 함수가 받는 것은 그 결과뿐이다. 실물과 같은 셋을
 * `'skip'` 으로 본다: usage · 벤더 결과 운반 행(`tool_result`) · 빈 생각.
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

/** 줄 목록을 비교하기 쉬운 모양으로 — 항목 줄 = itemId · 묶음 = 키 · 멤버 itemId · live. */
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

describe('groupToolRuns — 묶음 짓기', () => {
  it('도구 하나는 묶지 않는다 — 그 행 그대로', () => {
    const a = tool('a')
    const after = thought('then')
    expect(shape(groupToolRuns([a, after], true, rowKind))).toEqual([a.itemId, after.itemId])
  })

  it('도구 둘이 이어지면 묶음 하나 — 키 = 첫 호출의 백엔드 id', () => {
    const a = tool('a')
    const b = tool('b')
    expect(shape(groupToolRuns([a, b], false, rowKind))).toEqual([
      { key: 'tool:a', members: [a.itemId, b.itemId], calls: [a.itemId, b.itemId], live: false },
    ])
  })

  it('그리지 않는 행(벤더 결과 운반 · usage · 빈 생각)이 끼어도 이어지고 멤버 · 줄 어디에도 안 나온다', () => {
    const a = tool('a')
    const b = tool('b')
    const c = tool('c')
    const items = [a, carrier('a'), usage(), b, thought('   '), carrier('b'), c]
    expect(shape(groupToolRuns(items, false, rowKind))).toEqual([
      { key: 'tool:a', members: [a.itemId, b.itemId, c.itemId], calls: [a.itemId, b.itemId, c.itemId], live: false },
    ])
  })

  it('도구 사이의 비지 않은 생각은 멤버로 흡수한다(U5)', () => {
    const a = tool('a')
    const mid = thought('look at b next')
    const b = tool('b')
    expect(shape(groupToolRuns([a, mid, b], false, rowKind))).toEqual([
      { key: 'tool:a', members: [a.itemId, mid.itemId, b.itemId], calls: [a.itemId, b.itemId], live: false },
    ])
  })

  it.each([
    ['글', text],
    ['사용자 말풍선', bubble],
    ['턴 구분선', separator],
    ['결말', outcome],
    ['오류', errorRow],
    ['모르는 사건', unsupported],
    ['그 밖의 탈출구 항목', escape],
  ] as const)('%s 이 끼면 끊는다 — 양쪽이 도구 하나씩이라 묶음이 없다', (_label, make) => {
    const a = tool('a')
    const mid = (make as (s: string) => StructuredItem)('x')
    const b = tool('b')
    expect(shape(groupToolRuns([a, mid, b], true, rowKind))).toEqual([a.itemId, mid.itemId, b.itemId])
  })

  it('끊는 행 양쪽이 각각 둘 이상이면 묶음 둘 — 키가 다르다', () => {
    const [a, b] = [tool('a'), tool('b')]
    const mid = text('mid')
    const [c, d] = [tool('c'), tool('d')]
    const rows = groupToolRuns([a, b, mid, c, d], false, rowKind)
    expect(rows.map(r => (r.kind === 'item' ? r.item.itemId : r.key))).toEqual(['tool:a', mid.itemId, 'tool:c'])
  })

  it('첫 도구 앞 · 마지막 도구 뒤의 생각은 멤버가 아니고 제자리에 줄로 남는다', () => {
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

  it('묶음 안의 그리지 않는 행을 빼면 모든 항목이 원래 순서로 한 번씩 나온다', () => {
    const items = [text('hi'), tool('a'), carrier('a'), tool('b'), thought('t'), text('done'), separator(), tool('c')]
    const seen = groupToolRuns(items, false, rowKind).flatMap(r =>
      r.kind === 'item' ? [r.item.itemId] : r.members.map(m => m.itemId),
    )
    const carrierId = items[2].itemId
    expect(seen).toEqual(items.map(i => i.itemId).filter(id => id !== carrierId))
  })
})

describe('groupToolRuns — live(도는 중 — 머리 시제)', () => {
  it('턴 중 · 묶음이 꼬리면 live', () => {
    const rows = groupToolRuns([text('go'), tool('a'), tool('b')], true, rowKind)
    expect(groupsOf(rows)[0].live).toBe(true)
  })

  it('턴 중 · 뒤가 생각 · 그리지 않는 행뿐이면 live 그대로 — 생각이 왔다고 끝난 것으로 보지 않는다', () => {
    const rows = groupToolRuns([tool('a'), tool('b'), thought('hmm'), usage(), thought('  ')], true, rowKind)
    expect(groupsOf(rows)[0].live).toBe(true)
  })

  it('끊는 행이 뒤에 오면 live 가 아니다', () => {
    const rows = groupToolRuns([tool('a'), tool('b'), thought('hmm'), text('answer')], true, rowKind)
    expect(groupsOf(rows)[0].live).toBe(false)
  })

  it('턴이 닫혔으면 꼬리여도 live 가 아니다', () => {
    const rows = groupToolRuns([tool('a'), tool('b')], false, rowKind)
    expect(groupsOf(rows)[0].live).toBe(false)
  })

  it('꼬리 묶음만 live — 앞 묶음은 끝난 것이다', () => {
    const rows = groupToolRuns([tool('a'), tool('b'), text('x'), tool('c'), tool('d')], true, rowKind)
    expect(groupsOf(rows).map(g => g.live)).toEqual([false, true])
  })

  it('도는 꼬리 묶음이 자라도 키는 그대로다 — 생각 뒤에 도구가 이어지면 그 생각이 멤버가 된다', () => {
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

describe('groupToolRuns — 키', () => {
  it('첫 호출에 id 가 없으면 item:<itemId>', () => {
    const a = tool(null)
    expect(groupsOf(groupToolRuns([a, tool('b')], false, rowKind))[0].key).toBe(`item:${a.itemId}`)
  })

  it('빈 문자열 id 도 없는 것으로 본다', () => {
    const a = tool('')
    expect(groupsOf(groupToolRuns([a, tool('b')], false, rowKind))[0].key).toBe(`item:${a.itemId}`)
  })

  it('두 묶음의 첫 호출이 같은 id 면 뒤 묶음은 item:<itemId> — 키가 겹치지 않는다', () => {
    const first = tool('dup')
    const second = tool('dup')
    const keys = groupsOf(groupToolRuns([first, tool('x'), text('-'), second, tool('y')], false, rowKind)).map(g => g.key)
    expect(keys).toEqual(['tool:dup', `item:${second.itemId}`])
  })

  it('isToolGroupKey — 지은 키 모양만 받는다', () => {
    expect(isToolGroupKey('tool:call_1')).toBe(true)
    expect(isToolGroupKey('item:12')).toBe(true)
    expect(isToolGroupKey('call_1')).toBe(false)
    expect(isToolGroupKey('tool:')).toBe(false)
    expect(isToolGroupKey('item:x')).toBe(false)
    expect(isToolGroupKey('')).toBe(false)
  })
})

describe('groupToolRuns — replay(같은 사건열 = 같은 결과)', () => {
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

  it('reset 뒤 다시 먹여도 줄 · 키 · 멤버가 같다 — 고른 펼침이 replay 를 넘는다', () => {
    const acc = new StructuredEventAccumulator()
    const first = feedAll(acc)
    acc.reset()
    const again = feedAll(acc)
    expect(again).toEqual(first)
    expect(groupsOf(again).map(g => g.key)).toEqual(['tool:g1', 'item:6'])
  })
})

// ── summarizeGroup · 판정 ──

describe('summarizeGroup — 종류별 수', () => {
  it('고정 순서(검색 · 읽기 · 목록 · 편집 · 명령 · 웹 · 에이전트 · MCP · 기타) · 0 인 종류는 빠진다', () => {
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

  it('종류 칸이 없거나 모르는 낱말(프로토타입 이름 포함)이면 기타', () => {
    const calls = [
      tool(null, { category: undefined }),
      tool(null, { category: 'Teleport' as never }),
      tool(null, { category: 'toString' as never }),
    ]
    expect(summarizeGroup(calls, new Set()).counts).toEqual([['Other', 3]])
  })
})

describe('summarizeGroup — 오류 · 거부 셈', () => {
  it('codex 실패 = 표식 failed', () => {
    const s = summarizeGroup([tool('a', { resultMark: 'failed' }), tool('b')], new Set())
    expect([s.errors, s.declined]).toEqual([1, 0])
  })

  it('claude 파싱 오류 = buildToolResultMap 의 isError 인 호출(vendorErrorIdsOf)', () => {
    const results = new Map([
      ['a', { content: 'no such file', isError: true }],
      ['b', { content: 'ok', isError: false }],
    ])
    const ids = vendorErrorIdsOf(results)
    expect([...ids]).toEqual(['a'])
    const s = summarizeGroup([tool('a'), tool('b'), tool(null)], ids)
    expect([s.errors, s.declined]).toEqual([1, 0])
  })

  it('거부 셈 = refused + declined — 오류 수에 안 든다', () => {
    const s = summarizeGroup([tool('a', { resultMark: 'refused' }), tool('b', { resultMark: 'declined' }), tool('c')], new Set())
    expect([s.errors, s.declined]).toEqual([0, 2])
  })

  it('표식 없음 · 칸 없음은 셈에 안 든다 — 성공이 아니라 모름이다', () => {
    const s = summarizeGroup([tool('a', { resultMark: undefined }), tool('b', { resultMark: null })], new Set())
    expect([s.errors, s.declined]).toEqual([0, 0])
  })

  it('한 호출은 한 칸에만 — 거부 표식이 벤더 오류보다 먼저다', () => {
    const s = summarizeGroup([tool('a', { resultMark: 'refused' })], new Set(['a']))
    expect([s.errors, s.declined]).toEqual([0, 1])
  })

  it('뒤에 온 결과가 앞 표식을 덮은 값으로 센다(누산기 경유)', () => {
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

describe('DECLINED_MARK — 두 값', () => {
  const calls = [tool('a', { resultMark: 'refused' }), tool('b', { resultMark: 'declined' }), tool('c', { resultMark: 'failed' })]

  it("확정값은 'own'(사용자 결정 U2-a)", () => {
    expect(DECLINED_MARK).toBe('own')
  })

  it("'own' — 거부는 따로: 오류 1 · 거부 2 · 사유 키가 출처별", () => {
    const s = summarizeGroup(calls, new Set(), 'own')
    expect([s.errors, s.declined]).toEqual([1, 2])
    expect(toolCallVerdict('refused', false, 'own')).toBe('declined')
    expect(declinedReasonKey('refused', 'own')).toBe('chat.toolRefusedReason')
    expect(declinedReasonKey('declined', 'own')).toBe('chat.toolDeclinedReason')
  })

  it("'error' — 거부 둘 다 오류로: 오류 3 · 거부 0 · 사유 줄 없음", () => {
    const s = summarizeGroup(calls, new Set(), 'error')
    expect([s.errors, s.declined]).toEqual([3, 0])
    expect(toolCallVerdict('refused', false, 'error')).toBe('error')
    expect(toolCallVerdict('declined', false, 'error')).toBe('error')
    expect(declinedReasonKey('refused', 'error')).toBeNull()
    expect(declinedReasonKey('declined', 'error')).toBeNull()
  })

  it('기본 인자는 DECLINED_MARK 를 따른다', () => {
    expect(summarizeGroup(calls, new Set())).toEqual(summarizeGroup(calls, new Set(), DECLINED_MARK))
  })
})

describe('toolCallVerdict · declinedReasonKey', () => {
  it('실패 · 벤더 오류 = error · 표식 없음 = null', () => {
    expect(toolCallVerdict('failed', false)).toBe('error')
    expect(toolCallVerdict(null, true)).toBe('error')
    expect(toolCallVerdict(undefined, true)).toBe('error')
    expect(toolCallVerdict(null, false)).toBeNull()
    expect(toolCallVerdict(undefined, false)).toBeNull()
  })

  it('거부가 아니면 사유 키 없음', () => {
    expect(declinedReasonKey('failed')).toBeNull()
    expect(declinedReasonKey(null)).toBeNull()
    expect(declinedReasonKey(undefined)).toBeNull()
  })
})

describe('summaryParts — 요약 줄', () => {
  const line = (s: ToolGroupSummary): string => summaryParts(s).map(p => t(p.key, { count: String(p.count) })).join(' · ')

  it('종류 → 오류 → 거부 순서 — 「… · 오류 1 · 거부 1」', () => {
    const calls = [
      tool('a', { category: 'Search', resultMark: 'declined' }),
      tool('b', { category: 'Read', resultMark: 'failed' }),
      tool('c', { category: 'Search' }),
    ]
    const s = summarizeGroup(calls, new Set())
    expect(line(s)).toBe('검색 2 · 읽기 1 · 오류 1 · 거부 1')
    expect(summaryParts(s).map(p => p.tone)).toEqual(['default', 'default', 'error', 'declined'])
  })

  it('거부만 있으면 「오류」 칸이 없다', () => {
    const s = summarizeGroup([tool('a', { resultMark: 'refused' }), tool('b', { resultMark: 'declined' })], new Set())
    expect(line(s)).toBe('읽기 2 · 거부 2')
  })

  it("DECLINED_MARK='error' 면 거부가 오류 칸으로 — 「오류 2」 · 거부 칸 없음", () => {
    const s = summarizeGroup([tool('a', { resultMark: 'refused' }), tool('b', { resultMark: 'declined' })], new Set(), 'error')
    expect(line(s)).toBe('읽기 2 · 오류 2')
  })

  it('종류 아홉 전부 문구 키가 있다', () => {
    const all = (['Search', 'Read', 'List', 'Edit', 'Command', 'Web', 'Agent', 'Mcp', 'Other'] as const).map(category =>
      tool(null, { category }),
    )
    const texts = summaryParts(summarizeGroup(all, new Set())).map(p => t(p.key, { count: String(p.count) }))
    expect(texts).toEqual(['검색 1', '읽기 1', '목록 1', '편집 1', '명령 1', '웹 1', '에이전트 1', 'MCP 1', '기타 1'])
  })
})
