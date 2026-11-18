import { describe, expect, it, vi } from 'vitest'

import {
  StructuredEventAccumulator,
  type StructuredItem,
  type ToolCategory,
  type ToolResultMark,
} from './structuredAccumulator'
import { goldenRowOf, queuedInputGolden } from './testing/queuedInputGolden'
import type { QueuedInputEvent } from '../../../crates/engram-dashboard-protocol/bindings/QueuedInputEvent'
import type { StructuredEvent } from '../../../crates/engram-dashboard-protocol/bindings/StructuredEvent'
import type { TurnOutcome } from '../../../crates/engram-dashboard-protocol/bindings/TurnOutcome'

function encode(ev: StructuredEvent): Uint8Array {
  return new TextEncoder().encode(JSON.stringify(ev))
}
function textDelta(text: string): StructuredEvent {
  return { type: 'TextDelta', text, turn_id: null, message_id: null }
}
function toolCall(name: string, argsJson = '{}', id: string | null = null): StructuredEvent {
  return { type: 'ToolCall', name, args_json: argsJson, id, turn_id: null, message_id: null }
}
const messageDone: StructuredEvent = { type: 'MessageDone', turn_id: null, message_id: null }
/** Turn end on the codex path — a different type from claude's MessageDone, and turn_id arrives filled in. */
function turnEnd(outcome: TurnOutcome, turnId: string | null = 't1'): StructuredEvent {
  return { type: 'TurnEnd', turn_id: turnId, outcome }
}
/** A kind not in this shell's bindings (a newer daemon). It can be made only by bypassing the type gate. */
function unknownEvent(type: string): StructuredEvent {
  return { type, whatever: 1 } as unknown as StructuredEvent
}
/** RichSlot's derived display value (streaming in that file) — the axis that measures whether the screen's waiting indicator stands. */
function streamingOf(acc: StructuredEventAccumulator, awaiting: boolean): boolean {
  return awaiting || (!acc.isTurnDone() && acc.snapshot().length > 0)
}
/**
 * Minimal model of RichSlot's subscription callback: send (awaiting on) → frame arrival (awaiting off only when `feed` understood it) → derived streaming.
 * ★Needed to measure decisions spanning several turns★: a freshly made accumulator has turnDone false, so "the waiting indicator stays alive" passes for free. The real-component regression is in RichSlot.test.tsx.
 */
function slotModel(): {
  acc: StructuredEventAccumulator
  send: () => void
  frame: (ev: StructuredEvent) => void
  readonly streaming: boolean
} {
  const acc = new StructuredEventAccumulator()
  let awaiting = false
  return {
    acc,
    send: () => {
      awaiting = true
    },
    frame: (ev: StructuredEvent) => {
      if (acc.feed(encode(ev))) awaiting = false
    },
    get streaming() {
      return streamingOf(acc, awaiting)
    },
  }
}
/** user Structured event — shape common to synthetic echo · replay. */
function userEcho(text: string, uuid: string | null): StructuredEvent {
  const block: Record<string, unknown> = { type: 'text', text }
  if (uuid !== null) block['uuid'] = uuid
  return { type: 'Structured', kind: 'user', json: JSON.stringify(block) }
}
/** user-role tool_result block; carries the line-level uuid like the measured fixture (tool.jsonl). */
function userToolResult(toolUseId: string, content: string, uuid: string | null): StructuredEvent {
  const block: Record<string, unknown> = { type: 'tool_result', tool_use_id: toolUseId, content }
  if (uuid !== null) block['uuid'] = uuid
  return { type: 'Structured', kind: 'user', json: JSON.stringify(block) }
}
const labels = (items: StructuredItem[]): (string | undefined)[] =>
  items.map((it) => (it.kind === 'structured' ? it.label : undefined))
const kinds = (items: StructuredItem[]): string[] => items.map((it) => it.kind)
const ids = (items: StructuredItem[]): number[] => items.map((it) => it.itemId)

describe('StructuredEventAccumulator', () => {
  it('one TextDelta → one text item', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('hello')))
    expect(acc.snapshot()).toEqual([{ kind: 'text', text: 'hello', itemId: 0 }])
  })

  it('consecutive TextDelta join into one text item (concat)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('Hel')))
    acc.feed(encode(textDelta('lo, ')))
    acc.feed(encode(textDelta('world')))
    expect(acc.snapshot()).toEqual([{ kind: 'text', text: 'Hello, world', itemId: 0 }])
  })

  it('ToolCall → tool chip item (name/args/id preserved · Other if there is no kind field · no marker)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(toolCall('Read', '{"path":"a.ts"}', 'tu_1')))
    expect(acc.snapshot()).toEqual([
      {
        kind: 'tool',
        name: 'Read',
        argsJson: '{"path":"a.ts"}',
        id: 'tu_1',
        category: 'Other',
        resultMark: null,
        itemId: 0,
      },
    ])
  })

  it('Usage → usage chip item (token counts preserved)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode({ type: 'Usage', input_tokens: 10, output_tokens: 5, turn_id: null }))
    expect(acc.snapshot()).toEqual([{ kind: 'usage', inputTokens: 10, outputTokens: 5, itemId: 0 }])
  })

  it('Error → error chip item (does not close the turn — not a boundary on either backend)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('partial')))
    acc.feed(encode({ type: 'Error', message: 'boom' }))
    expect(acc.snapshot()).toEqual([
      { kind: 'text', text: 'partial', itemId: 0 },
      { kind: 'error', message: 'boom', itemId: 1 },
    ])
    expect(acc.isTurnDone()).toBe(false)
    // No divider is made either — the turn has not ended, so there is no boundary.
    expect(kinds(acc.snapshot())).not.toContain('separator')
  })

  // ── Retryable stream error mid-turn (codex overload · rate limit) ──
  // ★Measured★: no flicker where one error line turns the waiting indicator off and the next delta revives it. The codex `error` notification is not a turn boundary (decoder side) and claude also closes a failed turn with MessageDone — closing here would put the two axes out of step.
  it('Error does not switch off the waiting indicator mid-turn, and the following deltas and boundary flow as is', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('thinking')))
    expect(streamingOf(acc, false)).toBe(true)
    acc.feed(encode({ type: 'Error', message: 'server overloaded (serverOverloaded) [willRetry]' }))
    expect(streamingOf(acc, false)).toBe(true) // ★No flicker★
    acc.feed(encode(textDelta(' — resumed')))
    expect(streamingOf(acc, false)).toBe(true)
    acc.feed(encode(messageDone))
    expect(streamingOf(acc, false)).toBe(false) // A boundary is still closed by a boundary
    expect(kinds(acc.snapshot())).toEqual(['text', 'error', 'text', 'separator'])
  })

  it('a retry error mid second turn also does not switch off the waiting indicator (the axis a first-turn-only test missed)', () => {
    const m = slotModel()
    m.frame(textDelta('turn 1'))
    m.frame(messageDone)
    expect(m.streaming).toBe(false)

    m.send()
    m.frame(userEcho('follow-up', 'U2')) // The synthetic echo opens a new turn (turnDone=false)
    m.frame({ type: 'Error', message: 'rate limit (rateLimitExceeded) [willRetry]' })
    expect(m.streaming).toBe(true)
    m.frame(textDelta('turn 2 answer'))
    expect(m.streaming).toBe(true)
    m.frame(messageDone)
    expect(m.streaming).toBe(false)
  })

  it('Structured (escape hatch) → structured chip item (prevents loss)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode({ type: 'Structured', kind: 'CustomEvent', json: '{"x":1}' }))
    expect(acc.snapshot()).toEqual([
      { kind: 'structured', label: 'CustomEvent', json: '{"x":1}', itemId: 0 },
    ])
  })

  // ── user uuid dedup ──
  it('(a) a user echo (synthetic) with the same uuid + replay → exactly one remains', () => {
    const acc = new StructuredEventAccumulator()
    // Input-time synthetic echo (uuid=U) → later the claude replay echoes back the same uuid=U.
    acc.feed(encode(userEcho('내 메시지', 'U')))
    acc.feed(encode(userEcho('내 메시지', 'U')))
    expect(labels(acc.snapshot())).toEqual(['user'])
    expect(acc.snapshot().length).toBe(1)
  })

  it('(b) user text blocks with different uuids (past/non-matching) are all preserved — vanish regression guard', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(userEcho('과거 1', 'A')))
    acc.feed(encode(userEcho('과거 2', 'B')))
    acc.feed(encode(userEcho('현재', 'C')))
    expect(labels(acc.snapshot())).toEqual(['user', 'user', 'user'])
    expect(acc.snapshot().length).toBe(3)
  })

  it('(c) a user text item without uuid (past non-replay) is preserved without dedup', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(userEcho('uuid 없음', null)))
    acc.feed(encode(userEcho('uuid 없음', null)))
    expect(labels(acc.snapshot())).toEqual(['user', 'user'])
  })

  // ── tool_result in a multi-block user line is not lost to dedup ──
  it('(c-real) a tool_result with a uuid is also preserved without dedup (matches the measured fixture)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(userToolResult('toolu_1', '파일 내용', 'RESULT-UUID')))
    acc.feed(encode(userToolResult('toolu_1', '파일 내용', 'RESULT-UUID')))
    expect(labels(acc.snapshot())).toEqual(['user', 'user'])
  })

  it('(c-multi) a text echo + tool_result with the same uuid → both preserved (no tool_result loss)', () => {
    const acc = new StructuredEventAccumulator()
    // Two blocks of one user replay line (text echo + tool_result) share the line-level uuid=U; the tool_result must not be skipped as an "already seen uuid".
    acc.feed(encode(userEcho('echo', 'U')))
    acc.feed(encode(userToolResult('t1', 'r', 'U')))
    expect(labels(acc.snapshot())).toEqual(['user', 'user'])
    expect(acc.snapshot().length).toBe(2)
    const second = acc.snapshot()[1]
    expect(second.kind).toBe('structured')
    if (second.kind === 'structured') {
      const parsed = JSON.parse(second.json) as { type: string; tool_use_id: string }
      expect(parsed.type).toBe('tool_result')
      expect(parsed.tool_use_id).toBe('t1')
    }
  })

  it('(c-multi) even while tool_result is preserved the text echo itself is still uuid-deduped', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(userEcho('echo', 'U'))) // Input-time synthetic echo
    acc.feed(encode(userEcho('echo', 'U'))) // replay text — deduped and skipped
    acc.feed(encode(userToolResult('t1', 'r', 'U'))) // replay tool_result — kept
    expect(labels(acc.snapshot())).toEqual(['user', 'user'])
    expect(acc.snapshot().length).toBe(2)
    const first = acc.snapshot()[0]
    if (first.kind === 'structured') {
      expect((JSON.parse(first.json) as { type: string }).type).toBe('text')
    }
  })

  it('user uuid dedup does not affect kind!=="user" escape-hatch events', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode({ type: 'Structured', kind: 'thinking', json: '{"thinking":"t"}' }))
    acc.feed(encode({ type: 'Structured', kind: 'thinking', json: '{"thinking":"t"}' }))
    expect(labels(acc.snapshot())).toEqual(['thinking', 'thinking'])
  })

  it('reset → uuid dedup state is also reset (the same uuid can be seen again · replay idempotence)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(userEcho('m', 'U')))
    acc.feed(encode(userEcho('m', 'U')))
    expect(acc.snapshot().length).toBe(1)

    // Webview reload replay: after reset the whole history flows again → reconverges to the same snapshot.
    acc.reset()
    acc.feed(encode(userEcho('m', 'U')))
    acc.feed(encode(userEcho('m', 'U')))
    expect(acc.snapshot().length).toBe(1)
    expect(labels(acc.snapshot())).toEqual(['user'])
  })

  // ── ★Order preserved★: items pile up in event arrival order (text↔chip interleave) ──
  it('text→tool→text interleave order is preserved as is (the middle chip splits text segments)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('before ')))
    acc.feed(encode(toolCall('Bash', '{}')))
    acc.feed(encode(textDelta('after')))
    expect(kinds(acc.snapshot())).toEqual(['text', 'tool', 'text'])
    expect(acc.snapshot()[0]).toEqual({ kind: 'text', text: 'before ', itemId: 0 })
    expect(acc.snapshot()[2]).toEqual({ kind: 'text', text: 'after', itemId: 2 })
  })

  // ── ★turn boundary (ADR-0045)★: MessageDone → insert a divider item ──
  it('MessageDone → separator item inserted (turn boundary) + turnDone', () => {
    const acc = new StructuredEventAccumulator()
    expect(acc.isTurnDone()).toBe(false)
    acc.feed(encode(textDelta('turn one')))
    acc.feed(encode(messageDone))
    expect(kinds(acc.snapshot())).toEqual(['text', 'separator'])
    expect(acc.isTurnDone()).toBe(true)
    acc.feed(encode(textDelta('turn two')))
    expect(kinds(acc.snapshot())).toEqual(['text', 'separator', 'text'])
    expect(acc.isTurnDone()).toBe(false)
  })

  // ── a new user message (including the synthetic echo) clears turnDone ──
  it('after MessageDone (turnDone=true), a user Structured makes turnDone go back down to false', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('prev turn')))
    acc.feed(encode(messageDone))
    expect(acc.isTurnDone()).toBe(true)
    // If this is not cleared, RichSlot's derived streaming drops to false at this moment and WaitRow flickers off.
    acc.feed(encode(userEcho('follow-up', 'U')))
    expect(acc.isTurnDone()).toBe(false)
    acc.feed(encode(textDelta('assistant reply')))
    acc.feed(encode(messageDone))
    expect(acc.isTurnDone()).toBe(true)
  })

  it('consecutive MessageDone (empty turn) do not stack dividers', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('x')))
    acc.feed(encode(messageDone))
    acc.feed(encode(messageDone))
    expect(kinds(acc.snapshot())).toEqual(['text', 'separator'])
  })

  it('a leading MessageDone (no preceding item) makes no divider', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(messageDone))
    expect(acc.snapshot()).toEqual([])
    expect(acc.isTurnDone()).toBe(true)
  })

  it('even chips with no TextDelta are exposed as their chip items (not an empty snapshot)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(toolCall('Read')))
    acc.feed(encode({ type: 'Usage', input_tokens: 1, output_tokens: 2, turn_id: null }))
    expect(kinds(acc.snapshot())).toEqual(['tool', 'usage'])
  })

  it('string input (test/convenience) is handled the same', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(JSON.stringify(textDelta('str-path')))
    expect(acc.snapshot()).toEqual([{ kind: 'text', text: 'str-path', itemId: 0 }])
  })

  it('malformed JSON → silently skipped (the accumulator does not die, defensive)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(new TextEncoder().encode('{not json'))
    acc.feed(encode(textDelta('after')))
    expect(acc.snapshot()).toEqual([{ kind: 'text', text: 'after', itemId: 0 }])
  })

  it('a non-object payload (null · primitive) returns false without throwing — leaves only a warning and does not change state', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const acc = new StructuredEventAccumulator()
      for (const payload of ['null', '42', '"TextDelta"', 'true']) {
        expect(() => acc.feed(payload)).not.toThrow()
        expect(acc.feed(payload)).toBe(false)
      }
      expect(acc.snapshot()).toEqual([])
      expect(warnSpy).toHaveBeenCalled()
    } finally {
      warnSpy.mockRestore()
    }
  })

  // ── replay idempotence: after reset, refeeding the same events gives the same snapshot ──
  it('reset → cleared + refeeding the same event sequence gives the same snapshot (replay idempotence)', () => {
    const acc = new StructuredEventAccumulator()
    const stream: StructuredEvent[] = [
      textDelta('one'),
      toolCall('Read', '{"p":1}', 'tu_1'),
      textDelta('two'),
      messageDone,
      { type: 'Usage', input_tokens: 3, output_tokens: 4, turn_id: null },
      textDelta('three'),
    ]
    for (const ev of stream) acc.feed(encode(ev))
    const first = acc.snapshot().map((it) => ({ ...it }))
    expect(acc.isTurnDone()).toBe(false)

    acc.reset()
    expect(acc.snapshot()).toEqual([])
    expect(acc.isTurnDone()).toBe(false)

    for (const ev of stream) acc.feed(encode(ev))
    expect(acc.snapshot()).toEqual(first)
    expect(ids(acc.snapshot())).toEqual(ids(first))
  })

  // ── FIX-1: snapshot immutability(copy-on-write coalescing) ──
  it('FIX-1: summing TextDelta does not mutate the previous snapshot object (copy-on-write)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('hello')))
    const snap1 = acc.snapshot()
    const item1 = snap1[0]
    acc.feed(encode(textDelta(' world')))
    expect(item1).toEqual({ kind: 'text', text: 'hello', itemId: 0 })
    expect(acc.snapshot()[0]).toEqual({ kind: 'text', text: 'hello world', itemId: 0 })
    expect(acc.snapshot()[0]).not.toBe(item1)
  })

  // ── FIX-2: skip an empty TextDelta("") ──
  it('FIX-2: an empty TextDelta("") makes no phantom item', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('')))
    acc.feed(encode(messageDone))
    // There is no text item, so the leading-separator guard fires → there must be no separator either.
    expect(acc.snapshot()).toEqual([])
    expect(acc.isTurnDone()).toBe(true)
  })

  it('FIX-2: an empty TextDelta("") does not touch coalescing either (no-op)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('real')))
    const snap = acc.snapshot()
    const before = snap[0]
    acc.feed(encode(textDelta('')))
    expect(acc.snapshot()[0]).toEqual(before)
    expect(acc.snapshot().length).toBe(1)
  })

  // ── FIX-3: malformed tag1 JSON → console.warn ──
  it('FIX-3: malformed JSON → console.warn is called (signal of protocol data loss)', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const acc = new StructuredEventAccumulator()
      acc.feed(new TextEncoder().encode('{not valid json'))
      expect(warnSpy).toHaveBeenCalledOnce()
      expect(acc.snapshot()).toEqual([])
    } finally {
      warnSpy.mockRestore()
    }
  })

  // ── FIX-4: stable itemIds — monotonic id, same sequence reproduced after reset ──
  it('FIX-4: itemId increases monotonically in item insertion order', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('a')))
    acc.feed(encode(toolCall('Read')))
    acc.feed(encode(messageDone))
    acc.feed(encode({ type: 'Usage', input_tokens: 1, output_tokens: 2, turn_id: null }))
    // text(id=0), tool(id=1), separator(id=2), usage(id=3)
    expect(ids(acc.snapshot())).toEqual([0, 1, 2, 3])
  })

  it('FIX-4: consecutive TextDelta concat does not consume a new itemId (extends the same item)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('a')))
    acc.feed(encode(textDelta('b')))
    acc.feed(encode(textDelta('c')))
    expect(acc.snapshot()).toEqual([{ kind: 'text', text: 'abc', itemId: 0 }])
    acc.feed(encode(toolCall('Bash')))
    expect(ids(acc.snapshot())).toEqual([0, 1])
  })

  it('FIX-4: after reset itemId restarts from 0 (guarantees the same id on replay)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('x')))
    acc.feed(encode(toolCall('A')))
    const firstIds = ids(acc.snapshot())

    acc.reset()
    acc.feed(encode(textDelta('x')))
    acc.feed(encode(toolCall('A')))
    expect(ids(acc.snapshot())).toEqual(firstIds)
  })

  // ══ TurnEnd — all four outcomes close the turn; the outcome only decides the on-screen marker ══
  // ★Contract★: the outcome decides the display, not whether the turn closes — if one does not close it, the waiting indicator spins forever.

  it.each([
    ['Completed', { kind: 'Completed' } as TurnOutcome],
    ['Failed', { kind: 'Failed', detail: null } as TurnOutcome],
    ['Interrupted', { kind: 'Interrupted' } as TurnOutcome],
    ['Unknown', { kind: 'Unknown' } as TurnOutcome],
  ])('TurnEnd(%s) → the turn closes regardless of the outcome (waiting indicator stops)', (_name, outcome) => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('working')))
    expect(streamingOf(acc, false)).toBe(true)
    acc.feed(encode(turnEnd(outcome)))
    expect(acc.isTurnDone()).toBe(true)
    expect(streamingOf(acc, false)).toBe(false)
  })

  it('TurnEnd(Completed) leaves no outcome marker — only the divider (an ordinary completion must not look like a warning)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('done')))
    acc.feed(encode(turnEnd({ kind: 'Completed' })))
    expect(kinds(acc.snapshot())).toEqual(['text', 'separator'])
  })

  it('TurnEnd(Failed) → outcome=failed marker + reason (detail) preserved', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('partial')))
    acc.feed(encode(turnEnd({ kind: 'Failed', detail: 'model refused' })))
    expect(acc.snapshot()).toEqual([
      { kind: 'text', text: 'partial', itemId: 0 },
      { kind: 'outcome', outcome: 'failed', detail: 'model refused', itemId: 1 },
      { kind: 'separator', itemId: 2 },
    ])
  })

  it('TurnEnd(Failed, no reason) → the marker stays and only detail is null (a reason is not invented)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(turnEnd({ kind: 'Failed', detail: null })))
    expect(acc.snapshot()).toEqual([
      { kind: 'outcome', outcome: 'failed', detail: null, itemId: 0 },
      // A preceding item (marker) exists, so a divider follows.
      { kind: 'separator', itemId: 1 },
    ])
  })

  it('TurnEnd(Interrupted) is not stamped as a failure — neither an error item nor outcome=failed', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('half a sentence')))
    acc.feed(encode(turnEnd({ kind: 'Interrupted' })))
    const marks = acc.snapshot().filter((it) => it.kind === 'outcome')
    expect(marks).toEqual([{ kind: 'outcome', outcome: 'interrupted', detail: null, itemId: 1 }])
    expect(kinds(acc.snapshot())).not.toContain('error')
  })

  it('TurnEnd(Unknown) is left as unknown, not a failure', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(turnEnd({ kind: 'Unknown' })))
    expect(acc.snapshot()).toEqual([
      { kind: 'outcome', outcome: 'unknown', detail: null, itemId: 0 },
      { kind: 'separator', itemId: 1 },
    ])
  })

  it('an unknown outcome (a fifth value from a newer daemon) is also recorded as unknown and the turn closes — no guessing', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(turnEnd({ kind: 'Abandoned' } as unknown as TurnOutcome)))
    expect(acc.snapshot()[0]).toEqual({
      kind: 'outcome',
      outcome: 'unknown',
      detail: null,
      itemId: 0,
    })
    expect(acc.isTurnDone()).toBe(true)
  })

  it('a TurnEnd with no outcome field at all also closes as unknown without throwing', () => {
    const acc = new StructuredEventAccumulator()
    // The try/catch in feed wraps only JSON.parse — if it throws here the exception rises up to the subscription callback.
    expect(() =>
      acc.feed(encode({ type: 'TurnEnd', turn_id: null } as unknown as StructuredEvent)),
    ).not.toThrow()
    expect(acc.snapshot()[0]).toEqual({
      kind: 'outcome',
      outcome: 'unknown',
      detail: null,
      itemId: 0,
    })
    expect(acc.isTurnDone()).toBe(true)
  })

  it('a TurnEnd after MessageDone (even when the two vocabularies overlap) does not stack dividers', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('x')))
    acc.feed(encode(messageDone))
    acc.feed(encode(turnEnd({ kind: 'Completed' })))
    expect(kinds(acc.snapshot())).toEqual(['text', 'separator'])
  })

  it('a leading TurnEnd(Completed, no preceding item) makes no divider (same as MessageDone)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(turnEnd({ kind: 'Completed' })))
    expect(acc.snapshot()).toEqual([])
    expect(acc.isTurnDone()).toBe(true)
  })

  it('an event sequence including TurnEnd also gives the same snapshot on refeed (replay idempotence)', () => {
    const acc = new StructuredEventAccumulator()
    const stream: StructuredEvent[] = [
      textDelta('one'),
      turnEnd({ kind: 'Failed', detail: 'boom' }),
      textDelta('two'),
      turnEnd({ kind: 'Interrupted' }, 't2'),
    ]
    for (const ev of stream) acc.feed(encode(ev))
    const first = acc.snapshot().map((it) => ({ ...it }))
    acc.reset()
    for (const ev of stream) acc.feed(encode(ev))
    expect(acc.snapshot()).toEqual(first)
    expect(ids(acc.snapshot())).toEqual(ids(first))
  })

  // ══ Unknown event kind — swallowing it silently makes only the spinner vanish ══
  // ★Measured★: the screen must not stay unchanged while only the waiting indicator vanishes (release WebView2 has no devtools, so the console is no evidence).
  // ★Do not use one freshly made accumulator★: turnDone is false in the first turn so the indicator stands regardless; the defect appears only after the previous turn closed — measure over several turns.

  it('even if the first frame of the second turn is an unknown kind the waiting indicator survives (quiet switch-off regression guard)', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const m = slotModel()
      m.frame(textDelta('turn 1'))
      m.frame(messageDone)
      expect(m.acc.isTurnDone()).toBe(true)
      expect(m.streaming).toBe(false)

      // Send the second turn → the **first** frame of that turn arrives as an unknown kind.
      m.send()
      m.frame(unknownEvent('SomethingNewer'))
      // ★Must stay on★: `feed` returns false for the unknown kind, so waiting is kept (deriving from turnDone would die — it stays at the previous turn's true).
      expect(m.streaming).toBe(true)
      expect(m.acc.snapshot()[m.acc.snapshot().length - 1]).toEqual({
        kind: 'unsupported',
        count: 1,
        itemId: 2,
      })

      // When a known frame arrives, the decision passes to turnDone then — after that the boundary closes it normally.
      m.frame(textDelta('turn 2 answer'))
      expect(m.streaming).toBe(true)
      m.frame(messageDone)
      expect(m.streaming).toBe(false)
    } finally {
      warnSpy.mockRestore()
    }
  })

  it('feed returns false for a frame it did not understand — this return value is what protects the waiting indicator', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const acc = new StructuredEventAccumulator()
      expect(acc.feed(encode(textDelta('x')))).toBe(true)
      expect(acc.feed(encode(messageDone))).toBe(true)
      expect(acc.feed(encode({ type: 'Error', message: 'boom' }))).toBe(true)
      expect(acc.feed(encode(turnEnd({ kind: 'Completed' })))).toBe(true)
      // An empty delta makes no item but is a 「known kind」 — it was understood and let through.
      expect(acc.feed(encode(textDelta('')))).toBe(true)
      expect(acc.feed(encode(unknownEvent('Newer')))).toBe(false)
      // Malformed JSON · an empty payload are the same axis — nothing was learned from this frame.
      expect(acc.feed('{not json')).toBe(false)
      expect(acc.feed('')).toBe(false)
    } finally {
      warnSpy.mockRestore()
    }
  })

  it('even on the first turn (a freshly made accumulator) the marker item itself is made — a notice left on screen', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const acc = new StructuredEventAccumulator()
      acc.feed(encode(unknownEvent('SomethingNewer')))
      expect(acc.snapshot()).toEqual([{ kind: 'unsupported', count: 1, itemId: 0 }])
    } finally {
      warnSpy.mockRestore()
    }
  })

  it('the marker of an unknown type carries no event name or payload (protocol words not exposed)', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const acc = new StructuredEventAccumulator()
      acc.feed(encode(unknownEvent('item/reasoning/summaryTextDelta')))
      expect(JSON.stringify(acc.snapshot())).not.toContain('reasoning')
      expect(JSON.stringify(acc.snapshot())).not.toContain('whatever')
      // The diagnostic name goes out to the console only.
      expect(warnSpy).toHaveBeenCalledOnce()
    } finally {
      warnSpy.mockRestore()
    }
  })

  it('consecutive unknown types do not stack rows and are summed into one marker', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const acc = new StructuredEventAccumulator()
      acc.feed(encode(unknownEvent('A')))
      acc.feed(encode(unknownEvent('B')))
      acc.feed(encode(unknownEvent('C')))
      expect(acc.snapshot()).toEqual([{ kind: 'unsupported', count: 3, itemId: 0 }])
      const before = acc.snapshot()[0]
      acc.feed(encode(unknownEvent('D')))
      expect(acc.snapshot()[0]).not.toBe(before)
      expect(before).toEqual({ kind: 'unsupported', count: 3, itemId: 0 })
    } finally {
      warnSpy.mockRestore()
    }
  })

  // ★Counterpart of the regression above★: fixing it with `turnDone=false` breaks here — if the last frame of a restored history is an unknown kind nothing fixes it afterwards and the indicator spins forever (every restored session takes that shape when a newer daemon adds a turn-end vocabulary).
  it('even if the end of a restored history is an unknown kind the waiting indicator does not stick (the worth of leaving turnDone unchanged)', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const m = slotModel() // Restore path = only history flows again, without a send (no awaiting)
      m.frame(textDelta('x'))
      m.frame(messageDone)
      m.frame(unknownEvent('Newer')) // A new vocabulary a newer daemon attached at turn end
      expect(m.acc.isTurnDone()).toBe(true)
      expect(m.streaming).toBe(false)
    } finally {
      warnSpy.mockRestore()
    }
  })

  it('an event sequence including an unknown type also gives the same snapshot on refeed (replay idempotence)', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const acc = new StructuredEventAccumulator()
      const stream: StructuredEvent[] = [
        textDelta('one'),
        unknownEvent('X'),
        unknownEvent('Y'),
        textDelta('two'),
        turnEnd({ kind: 'Completed' }),
      ]
      for (const ev of stream) acc.feed(encode(ev))
      const first = acc.snapshot().map((it) => ({ ...it }))
      acc.reset()
      for (const ev of stream) acc.feed(encode(ev))
      expect(acc.snapshot()).toEqual(first)
      expect(ids(acc.snapshot())).toEqual(ids(first))
    } finally {
      warnSpy.mockRestore()
    }
  })
})

// ── ADR-0231: queued input (QueuedInput) arm ──────────────────────────────────────────

function queuedInput(op: QueuedInputEvent): StructuredEvent {
  return { type: 'QueuedInput', op }
}
const queued = (id: string, text = `text ${id}`) => queuedInput({ kind: 'Queued', id, text })
const cancelRequested = (id: string) => queuedInput({ kind: 'CancelRequested', id })
const cancelAnswered = (id: string, removed: boolean) => queuedInput({ kind: 'CancelAnswered', id, removed })
const cancelFailed = (id: string) => queuedInput({ kind: 'CancelFailed', id })
const delivered = (id: string) => queuedInput({ kind: 'Delivered', id })
const dropped = (id: string, cause: 'Withdrawn' | 'Interrupted' | 'AgentEnded' | 'Rejected' | 'Unknown') =>
  queuedInput({ kind: 'Dropped', id, cause })
const ackUnavailable = (copies: [string, string][]) =>
  queuedInput({ kind: 'AckUnavailable', delivered: copies.map(([id, text]) => ({ id, text })) })

/** User text bubbles of the conversation lines — in [uuid, body] order. */
function bubbles(acc: StructuredEventAccumulator): [string | null, string][] {
  return acc.snapshot().flatMap((it): [string | null, string][] => {
    if (it.kind !== 'structured' || it.label !== 'user') return []
    const block = JSON.parse(it.json) as { type?: string; text?: string; uuid?: string }
    return block.type === 'text' ? [[block.uuid ?? null, block.text ?? '']] : []
  })
}
const listed = (acc: StructuredEventAccumulator): string[] => acc.snapshotQueued().map((e) => e.id)
const rowIds = (acc: StructuredEventAccumulator): string[] => acc.queuedRows().map((e) => e.id)
function feedAll(acc: StructuredEventAccumulator, events: StructuredEvent[]): void {
  for (const ev of events) expect(acc.feed(encode(ev))).toBe(true)
}

describe('StructuredEventAccumulator — queued input (ADR-0231)', () => {
  it('shared golden: in every case the accumulator reduction state equals the golden list (the same file as the agent roster)', () => {
    expect(queuedInputGolden.cases.length).toBeGreaterThan(0)
    for (const c of queuedInputGolden.cases) {
      const acc = new StructuredEventAccumulator()
      feedAll(acc, c.events.map(queuedInput))
      expect(acc.queuedRows().map(goldenRowOf), `[${c.name}] list`).toEqual(c.expect.items)
      // The golden has no echo back — the 「already drawn uuid」 filter is empty, so the list to draw = the open items as they are (including cancel-pending).
      expect(listed(acc), `[${c.name}] list to draw`).toEqual(c.expect.items.map((r) => r.id))
    }
  })

  it('list events are understood frames — an unknown op kind or broken shape returns false and does not throw', () => {
    const acc = new StructuredEventAccumulator()
    expect(acc.feed(encode(queued('a')))).toBe(true)
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      expect(acc.feed(JSON.stringify({ type: 'QueuedInput', op: { kind: 'Reordered', id: 'a' } }))).toBe(false)
      expect(acc.feed(JSON.stringify({ type: 'QueuedInput' }))).toBe(false)
      expect(acc.feed(JSON.stringify({ type: 'QueuedInput', op: null }))).toBe(false)
      expect(acc.feed(JSON.stringify({ type: 'QueuedInput', op: { kind: 'AckUnavailable' } }))).toBe(false)
      const nullCopy = { type: 'QueuedInput', op: { kind: 'AckUnavailable', delivered: [null] } }
      expect(acc.feed(JSON.stringify(nullCopy))).toBe(false)
    } finally {
      warnSpy.mockRestore()
    }
    expect(rowIds(acc)).toEqual(['a'])
    expect(acc.snapshot()).toEqual([])
  })

  it('placement: Delivered removes the waiting id from the list and adds a bubble in that place with the Queued body', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [textDelta('앞'), queued('X', '나중 글'), textDelta('뒤')])
    expect(listed(acc)).toEqual(['X'])
    expect(bubbles(acc)).toEqual([])
    feedAll(acc, [delivered('X'), textDelta('끝')])
    expect(kinds(acc.snapshot())).toEqual(['text', 'structured', 'text'])
    expect(bubbles(acc)).toEqual([['X', '나중 글']])
    // Same shape as the synthetic echo — the renderer does not tell them apart.
    const bubble = acc.snapshot()[1]
    expect(bubble.kind === 'structured' && bubble.json).toBe('{"type":"text","text":"나중 글","uuid":"X"}')
    expect(listed(acc)).toEqual([])
    expect(rowIds(acc)).toEqual([])
  })

  it('turnDone: does not touch list events, and only a placement that drew a bubble lowers it like a user arm', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [textDelta('답'), messageDone])
    expect(acc.isTurnDone()).toBe(true)
    feedAll(acc, [
      queued('X'),
      queued('Y'),
      cancelRequested('Y'),
      cancelAnswered('Y', false),
      dropped('Y', 'Interrupted'),
      delivered('ghost'),
      dropped('X', 'Withdrawn'),
      ackUnavailable([]),
    ])
    expect(acc.isTurnDone()).toBe(true)
    expect(bubbles(acc)).toEqual([])
    const acc2 = new StructuredEventAccumulator()
    feedAll(acc2, [textDelta('답'), messageDone, queued('X'), delivered('X')])
    expect(acc2.isTurnDone()).toBe(false)
  })

  it('queued uuid suppression — whether the echo comes before Delivered (fold path) or after (new-turn path) the bubble is one at the received place', () => {
    const before = new StructuredEventAccumulator()
    feedAll(before, [queued('X', '글'), userEcho('글', 'X')])
    expect(bubbles(before)).toEqual([])
    expect(listed(before)).toEqual(['X'])
    feedAll(before, [delivered('X'), userEcho('글', 'X')])
    expect(bubbles(before)).toEqual([['X', '글']])

    const after = new StructuredEventAccumulator()
    feedAll(after, [queued('X', '글'), textDelta('답'), delivered('X'), textDelta('더'), userEcho('글', 'X')])
    expect(bubbles(after)).toEqual([['X', '글']])
    expect(kinds(after.snapshot())).toEqual(['text', 'structured', 'text'])
  })

  it('revival: the accumulator does not draw the bubble itself and the following vendor echo draws it', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [queued('X', '글'), dropped('X', 'Unknown')])
    expect(listed(acc)).toEqual([])
    feedAll(acc, [delivered('X')])
    expect(bubbles(acc)).toEqual([])
    feedAll(acc, [userEcho('글', 'X')])
    expect(bubbles(acc)).toEqual([['X', '글']])
  })

  it('CancelAnswered{true} in both orders — both drop from the list and draw nothing', () => {
    for (const tail of [
      [dropped('X', 'Unknown'), cancelAnswered('X', true)],
      [cancelAnswered('X', true), dropped('X', 'Unknown')],
    ]) {
      const acc = new StructuredEventAccumulator()
      feedAll(acc, [queued('X'), cancelRequested('X'), ...tail])
      expect(acc.snapshot()).toEqual([])
      expect(rowIds(acc)).toEqual([])
      expect(listed(acc)).toEqual([])
    }
  })

  it('a terminal outcome drops from the list and draws no notice row (regardless of cause · including a refusal before receipt)', () => {
    for (const cause of ['Withdrawn', 'Interrupted', 'AgentEnded', 'Unknown', 'Rejected'] as const) {
      const acc = new StructuredEventAccumulator()
      feedAll(acc, [textDelta('본문'), queued('X'), dropped('X', cause)])
      expect(kinds(acc.snapshot()), cause).toEqual(['text'])
      expect(listed(acc), cause).toEqual([])
    }
  })

  // ADR-0231 · ADR-0235 — ✕ is not hidden in advance: the row stays until the registry-removed event is reduced (claude and codex alike); a `not_removed` answer keeps it as cancel-pending (drawn like a handed-off row, without ✕).
  it('✕ keeps the row until the removal is confirmed — also draws cancel-pending, and drops only on removed:true · Dropped{Withdrawn}', () => {
    const ring = [queued('X'), queued('Y'), cancelRequested('X')]
    const a = new StructuredEventAccumulator()
    const b = new StructuredEventAccumulator()
    feedAll(a, ring)
    feedAll(b, ring)
    expect(listed(a)).toEqual(['X', 'Y'])
    expect(listed(b)).toEqual(listed(a))
    expect(a.snapshotQueued().map((e) => [e.id, e.phase.state])).toEqual([
      ['X', 'cancelling'],
      ['Y', 'queued'],
    ])
    feedAll(a, [cancelAnswered('X', true)])
    expect(listed(a)).toEqual(['Y'])
    expect(bubbles(a)).toEqual([])
    feedAll(b, [dropped('X', 'Withdrawn')])
    expect(listed(b)).toEqual(['Y'])
    // A full replay after reset is the same.
    a.reset()
    feedAll(a, ring)
    expect(listed(a)).toEqual(['X', 'Y'])
  })

  it('an answer that it could not be removed (removed:false · CancelFailed) leaves the row as cancel-pending (not_removed) and it drops from receipt as a bubble', () => {
    for (const answer of [cancelAnswered('X', false), cancelFailed('X')]) {
      const acc = new StructuredEventAccumulator()
      feedAll(acc, [queued('X', '못 뺀 글'), cancelRequested('X'), answer])
      expect(listed(acc)).toEqual(['X'])
      expect(acc.snapshotQueued()[0].phase).toEqual({
        state: 'cancelling',
        answer: 'not_removed',
        vendorClosed: false,
      })
      feedAll(acc, [delivered('X')])
      expect(listed(acc)).toEqual([])
      expect(bubbles(acc)).toEqual([['X', '못 뺀 글']])
    }
  })

  it('handover marker: HandedOver makes the row 「sent」 · a revert releases it · only receipt removes the row (a late marker is a no-op)', () => {
    const handedOver = (id: string, sentMark: boolean) => queuedInput({ kind: 'HandedOver', id, sent: sentMark })
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [queued('X', '넘긴 글'), queued('Y'), handedOver('X', true)])
    expect(acc.snapshotQueued().map((e) => [e.id, e.sent])).toEqual([
      ['X', true],
      ['Y', false],
    ])
    feedAll(acc, [handedOver('X', false)])
    expect(acc.snapshotQueued()[0].sent).toBe(false)
    feedAll(acc, [handedOver('X', true), delivered('X'), handedOver('X', true)])
    expect(listed(acc)).toEqual(['Y'])
    expect(bubbles(acc)).toEqual([['X', '넘긴 글']])
    // ✕ overlapped with the handoff — even if the marker stands after the could-not-remove answer, the row stays and is 「sent」.
    const race = new StructuredEventAccumulator()
    feedAll(race, [queued('Z'), cancelRequested('Z'), cancelAnswered('Z', false), handedOver('Z', true)])
    expect(race.snapshotQueued().map((e) => [e.id, e.sent])).toEqual([['Z', true]])
  })

  it('outcome of cancel-pending: (b) Delivered = the bubble in that place · (a)·(c) = nothing', () => {
    const late = new StructuredEventAccumulator()
    feedAll(late, [queued('X', '늦은 취소'), cancelRequested('X'), delivered('X')])
    expect(bubbles(late)).toEqual([['X', '늦은 취소']])

    for (const verdict of [cancelAnswered('X', true), dropped('X', 'AgentEnded'), dropped('X', 'Rejected')]) {
      const acc = new StructuredEventAccumulator()
      feedAll(acc, [queued('X'), cancelRequested('X'), verdict])
      expect(acc.snapshot()).toEqual([])
      expect(rowIds(acc)).toEqual([])
    }
  })

  it('✕ of a handed-over codex item: the row stays even after response false and Delivered removes the row with one set of bubble (a second Delivered is a no-op)', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [queued('X', '넘긴 글'), cancelRequested('X'), cancelAnswered('X', false)])
    expect(listed(acc)).toEqual(['X'])
    feedAll(acc, [delivered('X'), delivered('X')])
    expect(listed(acc)).toEqual([])
    expect(bubbles(acc)).toEqual([['X', '넘긴 글']])
  })

  it('erasing a refused bubble: Dropped{Rejected} after a Direct bubble takes that row away and leaves the uuid in 「seen」', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [userEcho('보낸 글', 'X'), textDelta('답'), dropped('X', 'Rejected')])
    expect(kinds(acc.snapshot())).toEqual(['text'])
    feedAll(acc, [userEcho('보낸 글', 'X')])
    expect(bubbles(acc)).toEqual([])
  })

  it('only a refusal erases — interrupt · agent exit · unknown do not touch the Direct bubble', () => {
    for (const cause of ['Interrupted', 'AgentEnded', 'Unknown', 'Withdrawn'] as const) {
      const acc = new StructuredEventAccumulator()
      feedAll(acc, [userEcho('보낸 글', 'X'), dropped('X', cause)])
      expect(bubbles(acc), cause).toEqual([['X', '보낸 글']])
    }
  })

  it('refusal after receipt Queued→Delivered→Dropped{Rejected} (the same event sequence as the golden): the bubble Delivered drew is taken away', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [queued('X'), delivered('X'), dropped('X', 'Rejected')])
    expect(acc.snapshot()).toEqual([])
    expect(rowIds(acc)).toEqual([])
  })

  it('refusal erasing leaves the tool_result block sharing the same uuid (only the text bubble is taken away)', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [userEcho('글', 'X'), userToolResult('tu1', 'OUT', 'X'), dropped('X', 'Rejected')])
    expect(bubbles(acc)).toEqual([])
    expect(labels(acc.snapshot())).toEqual(['user'])
    const left = acc.snapshot()[0]
    expect(left.kind === 'structured' && JSON.parse(left.json).type).toBe('tool_result')
  })

  it('copy of a finding: even with no Queued in the ring window one bubble in that place with the copy body', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [textDelta('답'), ackUnavailable([['X', '사본 본문']]), textDelta('더')])
    expect(kinds(acc.snapshot())).toEqual(['text', 'structured', 'text'])
    expect(bubbles(acc)).toEqual([['X', '사본 본문']])
  })

  it('copy of a finding: known items also one per copy (list order) — the list is empty and an already drawn uuid is not drawn', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [queued('X', 'A'), queued('Y', 'B'), cancelRequested('Y')])
    feedAll(acc, [ackUnavailable([['X', 'A'], ['Y', 'B']])])
    expect(bubbles(acc)).toEqual([
      ['X', 'A'],
      ['Y', 'B'],
    ])
    expect(rowIds(acc)).toEqual([])
    // A pair the core rewrote after deciding (`Queued` + one copy's `AckUnavailable`) is also two ordinary events.
    feedAll(acc, [queued('Z', 'C'), ackUnavailable([['Z', 'C']]), ackUnavailable([['X', 'A']])])
    expect(bubbles(acc)).toEqual([
      ['X', 'A'],
      ['Y', 'B'],
      ['Z', 'C'],
    ])
  })

  it('already-drawn uuid: echo → Queued → Delivered = one bubble (at the echo place) · not in the list to draw in between', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [userEcho('글', 'X'), textDelta('답'), queued('X', '글')])
    expect(listed(acc)).toEqual([])
    // The reduced state matches the golden — it goes onto the list and leaves by Delivered.
    expect(rowIds(acc)).toEqual(['X'])
    feedAll(acc, [delivered('X')])
    expect(rowIds(acc)).toEqual([])
    expect(bubbles(acc)).toEqual([['X', '글']])
    expect(kinds(acc.snapshot())).toEqual(['structured', 'text'])
  })

  it('already-drawn uuid: echo → Queued → AckUnavailable = one bubble', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [userEcho('글', 'X'), queued('X', '글'), ackUnavailable([['X', '글']])])
    expect(bubbles(acc)).toEqual([['X', '글']])
    expect(rowIds(acc)).toEqual([])
  })

  it('already-drawn uuid: echo → Delivered → Queued = one bubble (the tombstone drops a late Queued)', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [userEcho('글', 'X'), delivered('X'), queued('X', '글')])
    expect(bubbles(acc)).toEqual([['X', '글']])
    expect(rowIds(acc)).toEqual([])
    expect(listed(acc)).toEqual([])
  })

  it('idempotent: reset → refeed in the same order reproduces the conversation lines (including itemId), the list and the reduction state as is', () => {
    const ring: StructuredEvent[] = [
      userEcho('첫', 'D'),
      textDelta('답1'),
      queued('X', '둘'),
      queued('Y', '셋'),
      userEcho('둘', 'X'),
      delivered('X'),
      cancelRequested('Y'),
      queued('Z', '넷'),
      dropped('D', 'Rejected'),
      textDelta('답2'),
      messageDone,
    ]
    const acc = new StructuredEventAccumulator()
    feedAll(acc, ring)
    const items = JSON.parse(JSON.stringify(acc.snapshot())) as StructuredItem[]
    const list = listed(acc)
    const rows = acc.queuedRows().map(goldenRowOf)
    const done = acc.isTurnDone()
    acc.reset()
    expect(acc.snapshot()).toEqual([])
    expect(acc.queuedRows()).toEqual([])
    feedAll(acc, ring)
    expect(acc.snapshot()).toEqual(items)
    expect(ids(acc.snapshot())).toEqual(ids(items))
    expect(listed(acc)).toEqual(list)
    expect(acc.queuedRows().map(goldenRowOf)).toEqual(rows)
    expect(acc.isTurnDone()).toBe(done)
    expect(list).toEqual(['Y', 'Z'])
  })

  it('reset also clears tombstones — a terminal outcome outside the ring before replay does not drop the new ring\'s Queued', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [queued('X'), delivered('X')])
    acc.reset()
    feedAll(acc, [queued('X')])
    expect(listed(acc)).toEqual(['X'])
  })
})

// ── ADR-0231: reattach reconciliation (TRD §5-7) — on top of the snapshot, the events after the snapshot seq in seq order ──────────

type SnapshotRow = {
  id: string
  text: string
  state: string
  cancel: { answer: string; vendor_closed: boolean } | null
}
const queuedRow = (id: string, text = `text ${id}`): SnapshotRow => ({ id, text, state: 'queued', cancel: null })
const cancellingRow = (id: string, answer = 'none', vendorClosed = false): SnapshotRow => ({
  id,
  text: `text ${id}`,
  state: 'cancelling',
  cancel: { answer, vendor_closed: vendorClosed },
})
function snapshot(rows: SnapshotRow[], asOfSeq: number | null, epoch = 7) {
  return { inputs: rows, as_of_seq: asOfSeq, epoch }
}
/** Feed it carrying seq (same as the RichSlot subscription callback) — reconciliation stands on seq alone. */
function feedAt(acc: StructuredEventAccumulator, seq: number, ev: StructuredEvent): void {
  acc.feed(encode(ev), seq)
}
const phaseOf = (acc: StructuredEventAccumulator, id: string) =>
  acc.queuedRows().find((e) => e.id === id)?.phase

describe('StructuredEventAccumulator — reattach reconciliation (ADR-0231)', () => {
  it('if the snapshot seq has not arrived yet it holds and waits, then applies when that seq is delivered', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 5, textDelta('a'))
    const gen = acc.beginQueuedReconcile(7)
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('X')], 10))).toBe('held')
    expect(listed(acc)).toEqual([])
    feedAt(acc, 6, textDelta('b'))
    feedAt(acc, 9, textDelta('c'))
    expect(listed(acc), 'not applied before S').toEqual([])
    feedAt(acc, 10, textDelta('d'))
    expect(listed(acc)).toEqual(['X'])
  })

  // ADR-0231 · ADR-0235 — a reattached window also draws a handed-off row as 「sent」 (no ✕).
  it('reattach: the `sent` row of a list query becomes a row with the handover marker, and marker events after the snapshot stand on it again', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 20, textDelta('a'))
    const gen = acc.beginQueuedReconcile(7)
    feedAt(acc, 21, queuedInput({ kind: 'HandedOver', id: 'Y', sent: false }))
    const sentRow = (id: string): SnapshotRow => ({ id, text: `text ${id}`, state: 'sent', cancel: null })
    expect(acc.offerQueuedSnapshot(gen, snapshot([sentRow('X'), sentRow('Y'), queuedRow('Z')], 20))).toBe('applied')
    expect(acc.snapshotQueued().map((e) => [e.id, e.phase.state, e.sent])).toEqual([
      ['X', 'queued', true],
      ['Y', 'queued', false],
      ['Z', 'queued', false],
    ])
  })

  it('the seq of a frame that does not pass through feed, like tag0, also counts as delivered', () => {
    const acc = new StructuredEventAccumulator()
    const gen = acc.beginQueuedReconcile(7)
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('X')], 3))).toBe('held')
    expect(acc.observeSeq(2)).toBe(false)
    expect(acc.observeSeq(3)).toBe(true)
    expect(listed(acc)).toEqual(['X'])
  })

  it('if S has already been delivered it applies at once · with as_of_seq null it does not wait', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 20, textDelta('a'))
    const gen = acc.beginQueuedReconcile(7)
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('X')], 12))).toBe('applied')
    expect(listed(acc)).toEqual(['X'])

    const fresh = new StructuredEventAccumulator()
    const g = fresh.beginQueuedReconcile(7)
    expect(fresh.offerQueuedSnapshot(g, snapshot([], null))).toBe('applied')
    expect(listed(fresh)).toEqual([])
  })

  // The race in TRD §5-7: after the query read Queued{X}, CancelRequested{X} emitted by another window's ✕ arrives before the answer.
  it('even if a live CancelRequested{X} (seq > S) after the query comes before the answer, X does not go back to waiting (cancel-pending stays)', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 10, queued('X'))
    const gen = acc.beginQueuedReconcile(7)
    feedAt(acc, 11, cancelRequested('X'))
    expect(listed(acc)).toEqual(['X'])
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('X')], 10))).toBe('applied')
    expect(listed(acc)).toEqual(['X'])
    expect(phaseOf(acc, 'X')).toEqual({ state: 'cancelling', answer: 'none', vendorClosed: false })
  })

  it('a window where Queued{X} was pushed out of the ring: the CancelRequested{X} of the reconciliation window wins, and the following Delivered{X} draws a bubble with the row body', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 50, textDelta('long turn'))
    const gen = acc.beginQueuedReconcile(7)
    // The accumulator does not know X — it lets this event through as 「unknown → discard」.
    feedAt(acc, 51, cancelRequested('X'))
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('X', 'held body')], 40))).toBe('applied')
    expect(listed(acc), 'a pending cancel is drawn too (✕ locked)').toEqual(['X'])
    expect(phaseOf(acc, 'X')?.state).toBe('cancelling')
    feedAt(acc, 52, delivered('X'))
    expect(bubbles(acc)).toEqual([['X', 'held body']])
    expect(rowIds(acc)).toEqual([])
  })

  it('Delivered{X} in the reconciliation window (an item the accumulator did not know) → drops from the list and adds no bubble', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 50, textDelta('long turn'))
    const gen = acc.beginQueuedReconcile(7)
    feedAt(acc, 51, delivered('X'))
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('X')], 40))).toBe('applied')
    expect(rowIds(acc)).toEqual([])
    expect(bubbles(acc)).toEqual([])
  })

  it('an event with seq ≤ S is not reduced again (it is already in the snapshot)', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 9, textDelta('a'))
    const gen = acc.beginQueuedReconcile(7)
    // A receipt-unavailable finding delivered before S — the snapshot (S=11) carries Y, which stood after it, as queued.
    feedAt(acc, 10, ackUnavailable([]))
    feedAt(acc, 11, queued('Y'))
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('Y')], 11))).toBe('applied')
    expect(listed(acc), 'replaying the seq 10 AckUnavailable over Y closes Y').toEqual(['Y'])
  })

  it('AckUnavailable (seq > S) applies to every snapshot row', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 30, textDelta('a'))
    const gen = acc.beginQueuedReconcile(7)
    feedAt(acc, 31, ackUnavailable([]))
    expect(
      acc.offerQueuedSnapshot(gen, snapshot([queuedRow('A'), cancellingRow('B')], 20)),
    ).toBe('applied')
    expect(rowIds(acc)).toEqual([])
    expect(bubbles(acc), 'the reconcile draws no bubbles').toEqual([])
  })

  it('an item that stands after the snapshot (Queued seq > S) stays as the accumulator state · order = snapshot rows then the rest', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 10, queued('B'))
    const gen = acc.beginQueuedReconcile(7)
    feedAt(acc, 11, queued('C'))
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('A'), queuedRow('B')], 10))).toBe('applied')
    expect(listed(acc)).toEqual(['A', 'B', 'C'])
  })

  // Head jump: after seeing X, flush jumped the cursor to the later replay head (20) and lost X's close event (it was pushed out of the ring).
  it('an item open at the snapshot seq that is not in the snapshot and never stood after it closes as cause unknown — without a bubble', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 10, queued('X'))
    feedAt(acc, 20, textDelta('after jump'))
    const gen = acc.beginQueuedReconcile(7)
    expect(acc.offerQueuedSnapshot(gen, snapshot([], 20))).toBe('applied')
    expect(rowIds(acc)).toEqual([])
    expect(bubbles(acc)).toEqual([])
    // As a tombstone, a late-arriving Queued of the same id cannot stand it up again.
    feedAt(acc, 21, queued('X'))
    expect(rowIds(acc)).toEqual([])
  })

  it('a row of an unknown word also counts its id as 「present」 — not closed, left as the accumulator state', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 10, queued('X'))
    feedAt(acc, 20, textDelta('a'))
    const gen = acc.beginQueuedReconcile(7)
    const rows: SnapshotRow[] = [{ id: 'X', text: 'text X', state: 'parked', cancel: null }]
    expect(acc.offerQueuedSnapshot(gen, snapshot(rows, 20))).toBe('applied')
    expect(listed(acc)).toEqual(['X'])
    expect(phaseOf(acc, 'X')).toEqual({ state: 'queued' })
  })

  it('a recorded Queued{X} stays if seq > S, and does not stay on that record if seq ≤ S', () => {
    const later = new StructuredEventAccumulator()
    feedAt(later, 20, textDelta('a'))
    const g1 = later.beginQueuedReconcile(7)
    feedAt(later, 21, queued('X'))
    expect(later.offerQueuedSnapshot(g1, snapshot([], 20))).toBe('applied')
    expect(listed(later)).toEqual(['X'])

    const atS = new StructuredEventAccumulator()
    const g2 = atS.beginQueuedReconcile(7)
    feedAt(atS, 20, queued('X'))
    expect(atS.offerQueuedSnapshot(g2, snapshot([], 20))).toBe('applied')
    expect(listed(atS), 'an entry queued at S must be in the snapshot — if absent, it was closed by S').toEqual([])
  })

  it('an item the accumulator knows from the ring is in the same state after reconciliation (the same reduction rule)', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 1, queued('X'))
    feedAt(acc, 2, cancelRequested('X'))
    feedAt(acc, 3, cancelFailed('X'))
    const before = acc.queuedRows().map((e) => ({ ...e }))
    const gen = acc.beginQueuedReconcile(7)
    expect(acc.offerQueuedSnapshot(gen, snapshot([cancellingRow('X', 'not_removed')], 3))).toBe('applied')
    expect(acc.queuedRows()).toEqual(before)
  })

  it('unconfirmed reads as queued (an ordinary gray item)', () => {
    const acc = new StructuredEventAccumulator()
    const gen = acc.beginQueuedReconcile(7)
    acc.offerQueuedSnapshot(gen, snapshot([{ id: 'U', text: 'u', state: 'unconfirmed', cancel: null }], null))
    expect(listed(acc)).toEqual(['U'])
  })

  it('a row of an unknown word drops only that row — does not throw', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 5, textDelta('a'))
    const gen = acc.beginQueuedReconcile(7)
    const rows: SnapshotRow[] = [
      { id: 'W', text: 'w', state: 'parked', cancel: null },
      cancellingRow('V', 'maybe'),
      { id: 'N', text: 'n', state: 'cancelling', cancel: null },
      queuedRow('OK'),
    ]
    expect(() => acc.offerQueuedSnapshot(gen, snapshot(rows, 5))).not.toThrow()
    expect(rowIds(acc)).toEqual(['OK'])
  })

  it('an answer carrying another incarnation marker is dropped', () => {
    const acc = new StructuredEventAccumulator()
    const gen = acc.beginQueuedReconcile(7)
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('X')], null, 8))).toBe('stale')
    expect(listed(acc)).toEqual([])
  })

  // TRD §5-7 reconciliation generation: buffering overflow after the query (startBuffering) → if an old answer arrives with the record cleared, an item that had ✕ revives.
  it('generation: if the reconciliation was dropped before the answer came (buffer phase) a late old answer is dropped and only the next query\'s answer applies', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 50, textDelta('long turn'))
    const oldGen = acc.beginQueuedReconcile(7)
    acc.abandonQueuedReconcile() // 'buffering'
    feedAt(acc, 51, cancelRequested('X')) // ✕ from another window — there is no reconciliation to record into
    expect(acc.offerQueuedSnapshot(oldGen, snapshot([queuedRow('X')], 40))).toBe('stale')
    expect(listed(acc), 'Queued{X} of the old snapshot does not revive').toEqual([])

    const gen = acc.beginQueuedReconcile(7)
    expect(acc.offerQueuedSnapshot(gen, snapshot([cancellingRow('X')], 51))).toBe('applied')
    expect(listed(acc)).toEqual(['X'])
    expect(phaseOf(acc, 'X')?.state).toBe('cancelling')
  })

  it('generation: reset() also drops the in-progress reconciliation and the held answer (the generation is not rolled back)', () => {
    const acc = new StructuredEventAccumulator()
    const g1 = acc.beginQueuedReconcile(7)
    expect(acc.offerQueuedSnapshot(g1, snapshot([queuedRow('X')], 10))).toBe('held')
    acc.reset()
    expect(acc.observeSeq(10)).toBe(false)
    expect(listed(acc)).toEqual([])
    const g2 = acc.beginQueuedReconcile(7)
    expect(g2).not.toBe(g1)
    expect(acc.offerQueuedSnapshot(g1, snapshot([queuedRow('X')], null))).toBe('stale')
  })

  it('a query failure drops only that generation — a reconciliation opened in between is untouched', () => {
    const acc = new StructuredEventAccumulator()
    const g1 = acc.beginQueuedReconcile(7)
    const g2 = acc.beginQueuedReconcile(7)
    acc.abandonQueuedReconcile(g1)
    expect(acc.offerQueuedSnapshot(g2, snapshot([queuedRow('X')], null))).toBe('applied')
    expect(listed(acc)).toEqual(['X'])
  })

  it('a frame fed without seq does not stay in the reconciliation record (omitting seq is only a test convenience)', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 20, textDelta('a'))
    const gen = acc.beginQueuedReconcile(7)
    feedAll(acc, [cancelRequested('X')])
    acc.offerQueuedSnapshot(gen, snapshot([queuedRow('X')], 10))
    expect(listed(acc)).toEqual(['X'])
  })
})

// ── ADR-0239 · ADR-0241: tool kind · end result ──────────────────────────────────────

/** Bypass the type gate to carry an old daemon (no field) · a newer daemon (unknown word) · a broken value. */
function toolCallOf(name: string, id: string | null, category: unknown): StructuredEvent {
  return {
    type: 'ToolCall',
    name,
    args_json: '{}',
    id,
    turn_id: null,
    message_id: null,
    category,
  } as unknown as StructuredEvent
}
/** Bypass the type gate to carry a newer daemon's outcome word · a broken `id`. */
function toolResult(id: unknown, outcome: unknown): StructuredEvent {
  return { type: 'ToolResult', id, outcome } as unknown as StructuredEvent
}
/** Per tool row [backend id, marker]. */
function toolMarks(acc: StructuredEventAccumulator): [string | null, ToolResultMark | null | undefined][] {
  return acc
    .snapshot()
    .flatMap((it): [string | null, ToolResultMark | null | undefined][] =>
      it.kind === 'tool' ? [[it.id, it.resultMark]] : [],
    )
}
const categories = (acc: StructuredEventAccumulator): (ToolCategory | undefined)[] =>
  acc.snapshot().flatMap((it) => (it.kind === 'tool' ? [it.category] : []))

describe('StructuredEventAccumulator — tool kind · final result (ADR-0239 · ADR-0241)', () => {
  const known: ToolCategory[] = ['Read', 'Search', 'List', 'Edit', 'Command', 'Web', 'Agent', 'Mcp', 'Other']

  it.each(known)('category %s is carried as is', (category) => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(toolCallOf('T', 'c1', category)))
    expect(categories(acc)).toEqual([category])
  })

  it('if the category field is missing or an unknown value then Other — case and prototype names are also unknown values', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(toolCall('Read', '{}', 'c0')))
    const odd: unknown[] = ['Teleport', 'read', 'toString', 'constructor', 42, null, { kind: 'Read' }]
    for (const value of odd) acc.feed(encode(toolCallOf('T', 'x', value)))
    expect(categories(acc)).toEqual(Array(1 + odd.length).fill('Other'))
  })

  it.each<[string, ToolResultMark]>([
    ['Failed', 'failed'],
    ['Declined', 'declined'],
    ['Refused', 'refused'],
  ])('ToolResult(%s) → marker %s on the same-id row · no new item · feed is false', (outcome, mark) => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(toolCall('Bash', '{}', 'c1')))
    acc.feed(encode(toolCall('Read', '{}', 'c2')))
    const before = kinds(acc.snapshot())
    expect(acc.feed(encode(toolResult('c1', outcome)))).toBe(false)
    expect(kinds(acc.snapshot())).toEqual(before)
    expect(toolMarks(acc)).toEqual([
      ['c1', mark],
      ['c2', null],
    ])
  })

  it('Completed · unknown word · no outcome field → no marker and does not throw (only the unknown side warns)', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const acc = new StructuredEventAccumulator()
      for (const id of ['c1', 'c2', 'c3']) acc.feed(encode(toolCall('Bash', '{}', id)))
      expect(acc.feed(encode(toolResult('c1', 'Completed')))).toBe(false)
      expect(warnSpy).not.toHaveBeenCalled()
      expect(acc.feed(encode(toolResult('c2', 'TimedOut')))).toBe(false)
      expect(acc.feed(encode(toolResult('c3', undefined)))).toBe(false)
      expect(warnSpy).toHaveBeenCalledTimes(2)
      expect(toolMarks(acc)).toEqual([
        ['c1', null],
        ['c2', null],
        ['c3', null],
      ])
      expect(kinds(acc.snapshot())).toEqual(['tool', 'tool', 'tool'])
    } finally {
      warnSpy.mockRestore()
    }
  })

  it('if there is no row to point at (pushed out of the ring) it is dropped — snapshot as is · feed is false', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(toolCall('Bash', '{}', 'c1')))
    acc.feed(encode(messageDone))
    const before = acc.snapshot().map((it) => ({ ...it }))
    expect(acc.feed(encode(toolResult('gone', 'Failed')))).toBe(false)
    expect(acc.snapshot()).toEqual(before)
  })

  it('the null id of a broken frame does not attach to a tool row without an id — the outcome of a dropped frame does not warn', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const acc = new StructuredEventAccumulator()
      acc.feed(encode(toolCall('Glob', '{}', null)))
      expect(acc.feed(encode(toolResult(null, 'Failed')))).toBe(false)
      expect(acc.feed(encode(toolResult(null, 'Newer')))).toBe(false)
      expect(toolMarks(acc)).toEqual([[null, null]])
      expect(warnSpy).not.toHaveBeenCalled()
    } finally {
      warnSpy.mockRestore()
    }
  })

  it('a later result overwrites the earlier marker — Completed after Failed = no marker · unknown word after Failed = no marker + warning', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const acc = new StructuredEventAccumulator()
      acc.feed(encode(toolCall('Bash', '{}', 'c1')))
      acc.feed(encode(toolCall('Bash', '{}', 'c2')))
      acc.feed(encode(toolResult('c1', 'Failed')))
      acc.feed(encode(toolResult('c2', 'Failed')))
      expect(toolMarks(acc)).toEqual([
        ['c1', 'failed'],
        ['c2', 'failed'],
      ])
      expect(acc.feed(encode(toolResult('c1', 'Completed')))).toBe(false)
      expect(warnSpy).not.toHaveBeenCalled()
      expect(acc.feed(encode(toolResult('c2', 'Newer')))).toBe(false)
      expect(warnSpy).toHaveBeenCalledTimes(1)
      expect(toolMarks(acc)).toEqual([
        ['c1', null],
        ['c2', null],
      ])
    } finally {
      warnSpy.mockRestore()
    }
  })

  it('if two rows share the same id it attaches to the later row', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(toolCall('Bash', '{}', 'dup')))
    acc.feed(encode(toolCall('Bash', '{}', 'dup')))
    acc.feed(encode(toolResult('dup', 'Failed')))
    expect(toolMarks(acc)).toEqual([
      ['dup', null],
      ['dup', 'failed'],
    ])
  })

  it('copy-on-write — the row object of a snapshot received before attaching is not mutated', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(toolCall('Bash', '{}', 'c1')))
    const row = acc.snapshot()[0]
    acc.feed(encode(toolResult('c1', 'Failed')))
    expect(row).toMatchObject({ resultMark: null })
    expect(acc.snapshot()[0]).not.toBe(row)
    expect(acc.snapshot()[0]).toMatchObject({ resultMark: 'failed' })
  })

  it('does not touch turnDone — whether it comes mid-turn or after the turn end', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(toolCall('Bash', '{}', 'c1')))
    acc.feed(encode(toolCall('Bash', '{}', 'c2')))
    acc.feed(encode(toolResult('c1', 'Failed')))
    expect(acc.isTurnDone()).toBe(false)
    acc.feed(encode(turnEnd({ kind: 'Completed' })))
    acc.feed(encode(toolResult('c2', 'Failed')))
    expect(acc.isTurnDone()).toBe(true)
    expect(toolMarks(acc)).toEqual([
      ['c1', 'failed'],
      ['c2', 'failed'],
    ])
  })

  // ★Measured (TRD S21-chat-ux §4-7 ⑧)★: a late tool end of an interrupted turn landing right after a new send must not switch off the new turn's waiting indicator; measured over two turns since the first passes for free.
  it('a late end does not switch off the new turn\'s waiting indicator and only attaches a marker to the row — the next real item takes over the wait', () => {
    const m = slotModel()
    m.frame(toolCall('Bash', '{}', 'c1'))
    m.frame(turnEnd({ kind: 'Interrupted' }))
    expect(m.streaming).toBe(false)

    m.send()
    m.frame(toolResult('c1', 'Failed'))
    expect(m.streaming).toBe(true)
    expect(toolMarks(m.acc)).toEqual([['c1', 'failed']])

    m.frame(textDelta('turn 2'))
    m.frame(turnEnd({ kind: 'Completed' }, 't2'))
    expect(m.streaming).toBe(false)
  })

  // ★ⓐ pinned (TRD S21-chat-ux §11 ⑩)★: a tool of an interrupted turn that ends late with a failure gets the ordinary failure marker on its row; that turn's "interrupted" outcome row stays — no separate rule.
  it('(a) a late Failed of an interrupted turn row (arriving after the next turn end) → that row marker failed · the interrupted-outcome row as is', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(toolCall('Bash', '{}', 'c1')))
    acc.feed(encode(toolCall('Bash', '{}', 'c2')))
    acc.feed(encode(turnEnd({ kind: 'Interrupted' }, 't1')))
    acc.feed(encode(textDelta('turn 2')))
    acc.feed(encode(turnEnd({ kind: 'Completed' }, 't2')))
    const outcomeRow = acc.snapshot().find((it) => it.kind === 'outcome')
    const before = kinds(acc.snapshot())

    expect(acc.feed(encode(toolResult('c1', 'Failed')))).toBe(false)

    expect(toolMarks(acc)).toEqual([
      ['c1', 'failed'],
      ['c2', null],
    ])
    expect(kinds(acc.snapshot())).toEqual(before)
    expect(outcomeRow).toEqual({ kind: 'outcome', outcome: 'interrupted', detail: null, itemId: 2 })
    expect(acc.snapshot().filter((it) => it.kind === 'outcome')).toEqual([outcomeRow])
    expect(acc.isTurnDone()).toBe(true)
  })

  it('feeding the same event sequence twice gives the same snapshot (including marker · kind · itemId · feed return · replay)', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const acc = new StructuredEventAccumulator()
      const stream: StructuredEvent[] = [
        toolCallOf('Grep', 'c1', 'Search'),
        toolCallOf('Bash', 'c2', 'Command'),
        toolCallOf('Edit', 'c3', 'Edit'),
        toolCall('Web', '{}', 'c4'),
        toolResult('c2', 'Failed'),
        toolResult('c3', 'Refused'),
        turnEnd({ kind: 'Interrupted' }),
        toolResult('c1', 'Declined'),
        toolResult('gone', 'Failed'),
        toolResult('c4', 'Newer'),
      ]
      const returns = stream.map((ev) => acc.feed(encode(ev)))
      const first = acc.snapshot().map((it) => ({ ...it }))
      acc.reset()
      expect(stream.map((ev) => acc.feed(encode(ev)))).toEqual(returns)
      expect(acc.snapshot()).toEqual(first)
      expect(ids(acc.snapshot())).toEqual(ids(first))
      expect(toolMarks(acc)).toEqual([
        ['c1', 'declined'],
        ['c2', 'failed'],
        ['c3', 'refused'],
        ['c4', null],
      ])
      expect(categories(acc)).toEqual(['Search', 'Command', 'Edit', 'Other'])
    } finally {
      warnSpy.mockRestore()
    }
  })

  it('the seq of a ToolResult frame also counts as delivered — even if it returns false the held reconciliation answer stands at that seq', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 1, toolCall('Bash', '{}', 'c1'))
    const gen = acc.beginQueuedReconcile(7)
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('X')], 2))).toBe('held')
    feedAt(acc, 2, toolResult('c1', 'Failed'))
    expect(listed(acc)).toEqual(['X'])
    expect(toolMarks(acc)).toEqual([['c1', 'failed']])
  })
})

describe('StructuredEventAccumulator — claude interruption display row (ADR-0243)', () => {
  const SHORT = '[Request interrupted by user]'
  const FOR_TOOL = '[Request interrupted by user for tool use]'
  function interrupted(json: string): StructuredEvent {
    return { type: 'Structured', kind: 'interrupted', json }
  }
  const note = (text: string): StructuredEvent => interrupted(JSON.stringify({ text }))
  const interruptedEnd = turnEnd({ kind: 'Interrupted' })

  it.each([SHORT, FOR_TOOL])('%s → an interruptNote item carrying the original text', (text) => {
    const acc = new StructuredEventAccumulator()
    expect(acc.feed(encode(note(text)))).toBe(true)
    expect(acc.snapshot()).toEqual([{ kind: 'interruptNote', text, itemId: 0 }])
  })

  it.each([
    ['not JSON', 'not json'],
    ['no text field', JSON.stringify({ other: 1 })],
    ['text is not a string', JSON.stringify({ text: 3 })],
    ['not an object', JSON.stringify('x')],
    ['text is empty', JSON.stringify({ text: '' })],
  ])('broken-shape json (%s) → an escape-hatch structured item', (_name, json) => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(interrupted(json)))
    expect(acc.snapshot()).toEqual([{ kind: 'structured', label: 'interrupted', json, itemId: 0 }])
  })

  it('even the same text, kind=user is a user bubble as today', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(userEcho(SHORT, 'u1')))
    expect(labels(acc.snapshot())).toEqual(['user'])
    expect(kinds(acc.snapshot())).not.toContain('interruptNote')
  })

  it('does not take the side effects of the user arm — even after a closed turn it does not lower turnDone and does not eat the uuid', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('a')))
    acc.feed(encode(messageDone))
    acc.feed(encode(interrupted(JSON.stringify({ text: SHORT, uuid: 'u1' }))))
    expect(acc.isTurnDone()).toBe(true)
    // If the uuid had been put into 「seen」, this echo would vanish by dedup.
    acc.feed(encode(userEcho('hello', 'u1')))
    expect(labels(acc.snapshot()).filter((l) => l === 'user')).toHaveLength(1)
  })

  it('TurnEnd(Interrupted) after a display row → no outcome row · the divider stands · the turn closes', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(toolCall('Bash', '{}', 'c1')))
    acc.feed(encode(note(FOR_TOOL)))
    acc.feed(encode(interruptedEnd))
    expect(kinds(acc.snapshot())).toEqual(['tool', 'interruptNote', 'separator'])
    expect(acc.isTurnDone()).toBe(true)
  })

  it('TurnEnd(Interrupted) without a display row → the interrupted-outcome row stands as is (claude · codex where no synthetic line came)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('half')))
    acc.feed(encode(interruptedEnd))
    expect(kinds(acc.snapshot())).toEqual(['text', 'outcome', 'separator'])
  })

  it('the display row of the previous turn does not erase the next turn\'s interrupted-outcome row', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(note(SHORT)))
    acc.feed(encode(interruptedEnd))
    acc.feed(encode(userEcho('again', 'u2')))
    acc.feed(encode(textDelta('b')))
    acc.feed(encode(interruptedEnd))
    expect(kinds(acc.snapshot())).toEqual([
      'interruptNote',
      'separator',
      'structured',
      'text',
      'outcome',
      'separator',
    ])
  })

  it('even with a display row, the outcome rows of other outcomes (Failed · Unknown) stand', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(note(SHORT)))
    acc.feed(encode(turnEnd({ kind: 'Failed', detail: 'boom' })))
    expect(kinds(acc.snapshot())).toEqual(['interruptNote', 'outcome', 'separator'])
  })

  it('a codex-shaped event sequence without a display row is the same as today', () => {
    const acc = new StructuredEventAccumulator()
    for (const ev of [
      userEcho('go', 'u1'),
      toolCall('shell', '{}', 'c1'),
      textDelta('part'),
      interruptedEnd,
    ])
      acc.feed(encode(ev))
    expect(acc.snapshot()).toEqual([
      { kind: 'structured', label: 'user', json: JSON.stringify({ type: 'text', text: 'go', uuid: 'u1' }), itemId: 0 },
      { kind: 'tool', name: 'shell', argsJson: '{}', id: 'c1', category: 'Other', resultMark: null, itemId: 1 },
      { kind: 'text', text: 'part', itemId: 2 },
      { kind: 'outcome', outcome: 'interrupted', detail: null, itemId: 3 },
      { kind: 'separator', itemId: 4 },
    ])
  })

  it('on refeed the same snapshot (replay idempotence)', () => {
    const acc = new StructuredEventAccumulator()
    const stream = [textDelta('a'), note(SHORT), interruptedEnd, textDelta('b'), interruptedEnd]
    stream.forEach((ev) => acc.feed(encode(ev)))
    const first = acc.snapshot().map((it) => ({ ...it }))
    acc.reset()
    stream.forEach((ev) => acc.feed(encode(ev)))
    expect(acc.snapshot()).toEqual(first)
  })
})
