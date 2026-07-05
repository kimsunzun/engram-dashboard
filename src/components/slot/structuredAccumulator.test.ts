// StructuredEventAccumulator 단위테스트 — tag1 StructuredEvent 누적을 순서 보존 item 스트림으로.
//
// 라이브 RichSlot 이 tag1 payload(StructuredEvent JSON 1건)를 이 누산기에 흘려 item 을 누적한다.
// 프레임 1건 = 이벤트 1건(NDJSON 아님)이라 라인 재조립은 없다. 렌더 모델(ADR-0045 §52, 사용자 결정):
// text=텍스트 세그먼트 · ToolCall/Usage/Error/Structured=칩 item · MessageDone=구분선(turn 경계).

import { describe, expect, it, vi } from 'vitest'

import { StructuredEventAccumulator, type StructuredItem } from './structuredAccumulator'
import type { StructuredEvent } from '../../../crates/engram-dashboard-protocol/bindings/StructuredEvent'

/** StructuredEvent → tag1 payload 바이트(라이브 경로가 받는 형태). */
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
/** item.kind 시퀀스 — 순서 단언용. */
const kinds = (items: StructuredItem[]): string[] => items.map((it) => it.kind)
/** item.itemId 시퀀스 — id 단언용. */
const ids = (items: StructuredItem[]): number[] => items.map((it) => it.itemId)

describe('StructuredEventAccumulator', () => {
  it('TextDelta 1건 → text item 1개', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('hello')))
    expect(acc.snapshot()).toEqual([{ kind: 'text', text: 'hello', itemId: 0 }])
  })

  it('연속 TextDelta 는 한 text item 으로 이어붙인다(concat)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('Hel')))
    acc.feed(encode(textDelta('lo, ')))
    acc.feed(encode(textDelta('world')))
    expect(acc.snapshot()).toEqual([{ kind: 'text', text: 'Hello, world', itemId: 0 }])
  })

  it('ToolCall → tool 칩 item(name/args/id 보존)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(toolCall('Read', '{"path":"a.ts"}', 'tu_1')))
    expect(acc.snapshot()).toEqual([
      { kind: 'tool', name: 'Read', argsJson: '{"path":"a.ts"}', id: 'tu_1', itemId: 0 },
    ])
  })

  it('Usage → usage 칩 item(토큰 수 보존)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode({ type: 'Usage', input_tokens: 10, output_tokens: 5, turn_id: null }))
    expect(acc.snapshot()).toEqual([{ kind: 'usage', inputTokens: 10, outputTokens: 5, itemId: 0 }])
  })

  it('Error → error 칩 item + turnDone', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('partial')))
    acc.feed(encode({ type: 'Error', message: 'boom' }))
    expect(acc.snapshot()).toEqual([
      { kind: 'text', text: 'partial', itemId: 0 },
      { kind: 'error', message: 'boom', itemId: 1 },
    ])
    expect(acc.isTurnDone()).toBe(true)
  })

  it('Structured(탈출구) → structured 칩 item(유실 방지)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode({ type: 'Structured', kind: 'CustomEvent', json: '{"x":1}' }))
    expect(acc.snapshot()).toEqual([
      { kind: 'structured', label: 'CustomEvent', json: '{"x":1}', itemId: 0 },
    ])
  })

  // ── ★순서 보존★: 이벤트 도착 순서 그대로 item 이 쌓인다(text↔칩 인터리브) ──
  it('text→tool→text 인터리브 순서를 그대로 보존(중간 칩이 text 세그먼트를 가른다)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('before ')))
    acc.feed(encode(toolCall('Bash', '{}')))
    acc.feed(encode(textDelta('after')))
    expect(kinds(acc.snapshot())).toEqual(['text', 'tool', 'text'])
    expect(acc.snapshot()[0]).toEqual({ kind: 'text', text: 'before ', itemId: 0 })
    expect(acc.snapshot()[2]).toEqual({ kind: 'text', text: 'after', itemId: 2 })
  })

  // ── ★turn 경계(ADR-0045)★: MessageDone → 구분선 item 삽입 ──
  it('MessageDone → separator item 삽입(turn 경계) + turnDone', () => {
    const acc = new StructuredEventAccumulator()
    expect(acc.isTurnDone()).toBe(false)
    acc.feed(encode(textDelta('turn one')))
    acc.feed(encode(messageDone))
    expect(kinds(acc.snapshot())).toEqual(['text', 'separator'])
    expect(acc.isTurnDone()).toBe(true)
    // 다음 턴 텍스트는 구분선 뒤 새 세그먼트(직전이 separator 라 concat 안 됨).
    acc.feed(encode(textDelta('turn two')))
    expect(kinds(acc.snapshot())).toEqual(['text', 'separator', 'text'])
    expect(acc.isTurnDone()).toBe(false)
  })

  it('연속 MessageDone(빈 턴)은 구분선을 겹쳐 쌓지 않는다', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('x')))
    acc.feed(encode(messageDone))
    acc.feed(encode(messageDone))
    expect(kinds(acc.snapshot())).toEqual(['text', 'separator'])
  })

  it('맨 앞 MessageDone(선행 item 없음)은 구분선을 만들지 않는다', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(messageDone))
    expect(acc.snapshot()).toEqual([])
    expect(acc.isTurnDone()).toBe(true)
  })

  it('TextDelta 없이 칩만 와도 그 칩 item 은 그대로 노출(빈 스냅샷 아님)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(toolCall('Read')))
    acc.feed(encode({ type: 'Usage', input_tokens: 1, output_tokens: 2, turn_id: null }))
    expect(kinds(acc.snapshot())).toEqual(['tool', 'usage'])
  })

  it('문자열 입력(테스트/편의)도 동일 처리', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(JSON.stringify(textDelta('str-path')))
    expect(acc.snapshot()).toEqual([{ kind: 'text', text: 'str-path', itemId: 0 }])
  })

  it('malformed JSON → 조용히 스킵(누산기 안 죽음, 방어)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(new TextEncoder().encode('{not json'))
    acc.feed(encode(textDelta('after')))
    expect(acc.snapshot()).toEqual([{ kind: 'text', text: 'after', itemId: 0 }])
  })

  // ── ★replay idempotence★: reset 후 같은 이벤트열 refeed → 동일 스냅샷(웹뷰 리로드 복원 규율) ──
  it('reset → 초기화 + 같은 이벤트열 refeed 시 동일 스냅샷(replay idempotence)', () => {
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

    // 히스토리 전체가 다시 흐름(리로드 replay) → 동일 상태로 재구성.
    for (const ev of stream) acc.feed(encode(ev))
    expect(acc.snapshot()).toEqual(first)
    // FIX-4: itemId 도 동일(단조 id 가 reset→refeed 시 동일 시퀀스를 재현).
    expect(ids(acc.snapshot())).toEqual(ids(first))
  })

  // ── FIX-1: snapshot immutability(copy-on-write coalescing) ──
  it('FIX-1: TextDelta 합산이 이전 snapshot 객체를 변경하지 않는다(copy-on-write)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('hello')))
    // 스냅샷 참조를 캡처(React state 에 set 한 직후를 시뮬레이션).
    const snap1 = acc.snapshot()
    const item1 = snap1[0]
    // 추가 TextDelta 를 먹인다 — coalescing 발생.
    acc.feed(encode(textDelta(' world')))
    // 이전 스냅샷 객체(item1)가 여전히 원래 값이어야 한다.
    expect(item1).toEqual({ kind: 'text', text: 'hello', itemId: 0 })
    // 새 스냅샷은 이어붙인 값을 반영.
    expect(acc.snapshot()[0]).toEqual({ kind: 'text', text: 'hello world', itemId: 0 })
    // 두 item 이 별개 객체여야 한다(copy-on-write 확인).
    expect(acc.snapshot()[0]).not.toBe(item1)
  })

  // ── FIX-2: 빈 TextDelta("") 스킵 ──
  it('FIX-2: 빈 TextDelta("") 는 phantom item 을 만들지 않는다', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('')))
    acc.feed(encode(messageDone))
    // text item 이 없으므로 leading-separator 가드 발동 → separator 도 없어야 한다.
    expect(acc.snapshot()).toEqual([])
    expect(acc.isTurnDone()).toBe(true)
  })

  it('FIX-2: 빈 TextDelta("") 는 coalesce 도 건드리지 않는다(no-op)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('real')))
    const snap = acc.snapshot()
    const before = snap[0]
    acc.feed(encode(textDelta('')))
    // 빈 delta 이후 스냅샷이 변해선 안 된다.
    expect(acc.snapshot()[0]).toEqual(before)
    expect(acc.snapshot().length).toBe(1)
  })

  // ── FIX-3: malformed tag1 JSON → console.warn ──
  it('FIX-3: malformed JSON → console.warn 호출(프로토콜 데이터 유실 신호)', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const acc = new StructuredEventAccumulator()
      acc.feed(new TextEncoder().encode('{not valid json'))
      expect(warnSpy).toHaveBeenCalledOnce()
      // item 은 추가되지 않아야 한다.
      expect(acc.snapshot()).toEqual([])
    } finally {
      warnSpy.mockRestore()
    }
  })

  // ── FIX-4: stable itemIds — 단조 id, reset 후 동일 시퀀스 재현 ──
  it('FIX-4: itemId 는 item 삽입 순서대로 단조 증가한다', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('a')))
    acc.feed(encode(toolCall('Read')))
    acc.feed(encode(messageDone))
    acc.feed(encode({ type: 'Usage', input_tokens: 1, output_tokens: 2, turn_id: null }))
    // text(id=0), tool(id=1), separator(id=2), usage(id=3)
    expect(ids(acc.snapshot())).toEqual([0, 1, 2, 3])
  })

  it('FIX-4: 연속 TextDelta concat 은 새 itemId 를 소비하지 않는다(같은 item 확장)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('a')))
    acc.feed(encode(textDelta('b')))
    acc.feed(encode(textDelta('c')))
    // 세 델타가 하나의 text item 으로 합산 — itemId 는 0 하나만 소비.
    expect(acc.snapshot()).toEqual([{ kind: 'text', text: 'abc', itemId: 0 }])
    // 이후 tool item 은 itemId=1.
    acc.feed(encode(toolCall('Bash')))
    expect(ids(acc.snapshot())).toEqual([0, 1])
  })

  it('FIX-4: reset 후 itemId 는 0 부터 재시작(replay 동일 id 보장)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('x')))
    acc.feed(encode(toolCall('A')))
    const firstIds = ids(acc.snapshot())

    acc.reset()
    acc.feed(encode(textDelta('x')))
    acc.feed(encode(toolCall('A')))
    expect(ids(acc.snapshot())).toEqual(firstIds)
  })
})
