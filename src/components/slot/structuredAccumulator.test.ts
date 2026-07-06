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
/** user Structured 이벤트(합성 에코·replay 공통 shape: {"type":"text","text":…,"uuid":"X"}). */
function userEcho(text: string, uuid: string | null): StructuredEvent {
  const block: Record<string, unknown> = { type: 'text', text }
  if (uuid !== null) block['uuid'] = uuid
  return { type: 'Structured', kind: 'user', json: JSON.stringify(block) }
}
/** user-role tool_result 블록(decoder 가 line-level uuid 를 실어 통과 — 도구 결과 데이터).
 *  실측 fixture(tool.jsonl)의 tool_result user 라인은 top-level uuid 를 가지므로 여기서도 uuid 부착. */
function userToolResult(toolUseId: string, content: string, uuid: string | null): StructuredEvent {
  const block: Record<string, unknown> = { type: 'tool_result', tool_use_id: toolUseId, content }
  if (uuid !== null) block['uuid'] = uuid
  return { type: 'Structured', kind: 'user', json: JSON.stringify(block) }
}
/** item.label 시퀀스(structured item 판별용). */
const labels = (items: StructuredItem[]): (string | undefined)[] =>
  items.map((it) => (it.kind === 'structured' ? it.label : undefined))
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

  // ── ★user uuid dedup(blunt-suppress → uuid dedup 교체)★ ──
  it('(a) 같은 uuid 의 user 에코(합성) + replay → 정확히 한 개만 남는다', () => {
    const acc = new StructuredEventAccumulator()
    // 입력-시점 합성 에코(uuid=U) → 이후 claude replay 가 같은 uuid=U 로 되울림.
    acc.feed(encode(userEcho('내 메시지', 'U')))
    acc.feed(encode(userEcho('내 메시지', 'U')))
    expect(labels(acc.snapshot())).toEqual(['user'])
    expect(acc.snapshot().length).toBe(1)
  })

  it('(b) uuid 가 다른 user text 블록(과거/비매칭)은 전부 보존된다 — vanish 회귀 가드', () => {
    const acc = new StructuredEventAccumulator()
    // resume 로 되살아난 과거 대화: 서로 다른 uuid 를 가진 여러 user 턴.
    acc.feed(encode(userEcho('과거 1', 'A')))
    acc.feed(encode(userEcho('과거 2', 'B')))
    acc.feed(encode(userEcho('현재', 'C')))
    expect(labels(acc.snapshot())).toEqual(['user', 'user', 'user'])
    expect(acc.snapshot().length).toBe(3)
  })

  it('(c) uuid 없는 user text item(과거 비-replay)은 dedup 하지 않고 보존', () => {
    const acc = new StructuredEventAccumulator()
    // uuid 없는 동일 내용 user item 두 개 → dedup 대상 아님(둘 다 남는다).
    acc.feed(encode(userEcho('uuid 없음', null)))
    acc.feed(encode(userEcho('uuid 없음', null)))
    expect(labels(acc.snapshot())).toEqual(['user', 'user'])
  })

  // ── ★HIGH FIX: multi-block user 라인에서 tool_result 가 dedup 으로 소실되지 않는다★ ──
  it('(c-real) uuid 를 가진 tool_result 도 dedup 하지 않고 보존(실측 fixture 정합)', () => {
    const acc = new StructuredEventAccumulator()
    // 실측 fixture(tool.jsonl)의 tool_result user 라인은 top-level uuid 를 갖는다 → 블록 json 에 uuid 실림.
    // 그래도 tool_result 는 dedup 대상이 아니라(type!=="text") 항상 보존 — 같은 uuid 두 번 와도 둘 다 남는다.
    acc.feed(encode(userToolResult('toolu_1', '파일 내용', 'RESULT-UUID')))
    acc.feed(encode(userToolResult('toolu_1', '파일 내용', 'RESULT-UUID')))
    expect(labels(acc.snapshot())).toEqual(['user', 'user'])
  })

  it('(c-multi) 같은 uuid 의 text 에코 + tool_result → 둘 다 보존(tool_result 소실 금지)', () => {
    const acc = new StructuredEventAccumulator()
    // 한 user replay 라인의 두 블록(text 에코 + tool_result)이 같은 line-level uuid=U 로 온다.
    // 예전 결함: text(uuid=U)를 seenUserUuids 에 넣고 tool_result(같은 uuid U)를 "이미 본 uuid" 로
    //   스킵 → tool_result 소실. 이제 dedup 은 text 블록에만 적용되므로 tool_result 는 항상 보존.
    acc.feed(encode(userEcho('echo', 'U')))
    acc.feed(encode(userToolResult('t1', 'r', 'U')))
    // 둘 다 남는다(text 1 + tool_result 1).
    expect(labels(acc.snapshot())).toEqual(['user', 'user'])
    expect(acc.snapshot().length).toBe(2)
    // 두 번째 item 이 tool_result 인지 json 으로 확인(소실 안 됨).
    const second = acc.snapshot()[1]
    expect(second.kind).toBe('structured')
    if (second.kind === 'structured') {
      const parsed = JSON.parse(second.json) as { type: string; tool_use_id: string }
      expect(parsed.type).toBe('tool_result')
      expect(parsed.tool_use_id).toBe('t1')
    }
  })

  it('(c-multi) tool_result 보존 중에도 text 에코 자체는 여전히 uuid dedup 된다', () => {
    const acc = new StructuredEventAccumulator()
    // 합성 에코(text, uuid=U) → replay 라인의 text(uuid=U) + tool_result(uuid=U).
    // text 에코는 합성분과 합쳐져 1개, tool_result 는 보존 → 총 2개.
    acc.feed(encode(userEcho('echo', 'U'))) // 입력-시점 합성 에코
    acc.feed(encode(userEcho('echo', 'U'))) // replay text — dedup 되어 스킵
    acc.feed(encode(userToolResult('t1', 'r', 'U'))) // replay tool_result — 보존
    expect(labels(acc.snapshot())).toEqual(['user', 'user'])
    expect(acc.snapshot().length).toBe(2)
    // 첫 item = text(dedup 후 1개), 둘째 = tool_result.
    const first = acc.snapshot()[0]
    if (first.kind === 'structured') {
      expect((JSON.parse(first.json) as { type: string }).type).toBe('text')
    }
  })

  it('user uuid dedup 은 kind!=="user" 탈출구 이벤트에는 영향 없다', () => {
    const acc = new StructuredEventAccumulator()
    // user 아닌 Structured 는 uuid 개념 없음 — 같은 json 이어도 dedup 안 됨.
    acc.feed(encode({ type: 'Structured', kind: 'thinking', json: '{"thinking":"t"}' }))
    acc.feed(encode({ type: 'Structured', kind: 'thinking', json: '{"thinking":"t"}' }))
    expect(labels(acc.snapshot())).toEqual(['thinking', 'thinking'])
  })

  it('reset → uuid dedup 상태도 초기화(같은 uuid 를 다시 볼 수 있다 · replay idempotence)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(userEcho('m', 'U')))
    acc.feed(encode(userEcho('m', 'U')))
    expect(acc.snapshot().length).toBe(1)

    // 웹뷰 리로드 replay: reset 후 히스토리 전체가 다시 흐른다 → 동일 스냅샷으로 재수렴.
    acc.reset()
    acc.feed(encode(userEcho('m', 'U')))
    acc.feed(encode(userEcho('m', 'U')))
    expect(acc.snapshot().length).toBe(1)
    expect(labels(acc.snapshot())).toEqual(['user'])
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
