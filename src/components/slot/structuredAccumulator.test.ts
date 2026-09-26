import { describe, expect, it, vi } from 'vitest'

import { StructuredEventAccumulator, type StructuredItem } from './structuredAccumulator'
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
/** codex 경로의 턴 종료 — claude 의 MessageDone 과 다른 타입이고 turn_id 가 채워져 온다. */
function turnEnd(outcome: TurnOutcome, turnId: string | null = 't1'): StructuredEvent {
  return { type: 'TurnEnd', turn_id: turnId, outcome }
}
/** 이 셸의 bindings 에 없는 종류(더 새 데몬). 타입 게이트를 우회해야만 만들 수 있다. */
function unknownEvent(type: string): StructuredEvent {
  return { type, whatever: 1 } as unknown as StructuredEvent
}
/** RichSlot 의 파생 표현값(그 파일의 streaming) — 화면 대기 표시가 서는지 재는 축. */
function streamingOf(acc: StructuredEventAccumulator, awaiting: boolean): boolean {
  return awaiting || (!acc.isTurnDone() && acc.snapshot().length > 0)
}
/**
 * RichSlot 구독 콜백의 최소 모형 — 전송(awaiting on) → 프레임 도착(`feed` 가 알아들었을 때만 awaiting off)
 * → 파생 streaming. ★여러 턴에 걸친 판정을 한 축에서 재려고 둔다★: 갓 만든 누산기만 먹이는 테스트는
 * turnDone 이 아직 false 라 「대기 표시가 산다」가 공짜로 통과한다(둘째 턴부터 깨지는 것을 못 본다).
 * 실물 회귀는 RichSlot.test.tsx 가 진짜 컴포넌트로 잰다 — 여기는 누산기가 내주는 신호만으로 그 판정이
 * 서는지를 잰다.
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
/** user Structured 이벤트 — 합성 에코·replay 공통 shape. */
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
const labels = (items: StructuredItem[]): (string | undefined)[] =>
  items.map((it) => (it.kind === 'structured' ? it.label : undefined))
const kinds = (items: StructuredItem[]): string[] => items.map((it) => it.kind)
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

  it('Error → error 칩 item(턴은 닫지 않는다 — 양쪽 백엔드에서 경계가 아니다)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('partial')))
    acc.feed(encode({ type: 'Error', message: 'boom' }))
    expect(acc.snapshot()).toEqual([
      { kind: 'text', text: 'partial', itemId: 0 },
      { kind: 'error', message: 'boom', itemId: 1 },
    ])
    expect(acc.isTurnDone()).toBe(false)
    // 구분선도 만들지 않는다 — 턴이 안 끝났으니 경계도 없다.
    expect(kinds(acc.snapshot())).not.toContain('separator')
  })

  // ── 재시도 가능한 스트림 오류가 턴 한복판에 온다(codex 과부하·rate limit) ────────────────
  //
  // ★재는 것★: 「오류 한 줄에 대기 표시가 꺼졌다가 다음 델타에 되살아나는 깜빡임」이 없을 것. 그 어휘가
  //   턴 경계가 아니라는 것은 decoder 쪽 사실이고(codex `error` 알림 주석), claude 도 실패 턴을
  //   MessageDone 으로 닫는다 — 그래서 이 자리에서 턴을 닫으면 두 축이 어긋난다.
  it('Error 는 턴 한복판에서 대기 표시를 끄지 않고, 이어지는 델타·경계가 그대로 흐른다', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('thinking')))
    expect(streamingOf(acc, false)).toBe(true)
    acc.feed(encode({ type: 'Error', message: 'server overloaded (serverOverloaded) [willRetry]' }))
    expect(streamingOf(acc, false)).toBe(true) // ★깜빡임 없음★
    acc.feed(encode(textDelta(' — resumed')))
    expect(streamingOf(acc, false)).toBe(true)
    acc.feed(encode(messageDone))
    expect(streamingOf(acc, false)).toBe(false) // 경계는 여전히 경계가 닫는다
    expect(kinds(acc.snapshot())).toEqual(['text', 'error', 'text', 'separator'])
  })

  it('둘째 턴 한복판의 재시도 오류도 대기 표시를 끄지 않는다(첫 턴만 보는 테스트가 놓치던 축)', () => {
    const m = slotModel()
    m.frame(textDelta('turn 1'))
    m.frame(messageDone)
    expect(m.streaming).toBe(false)

    m.send()
    m.frame(userEcho('follow-up', 'U2')) // 합성 에코가 새 턴을 연다(turnDone=false)
    m.frame({ type: 'Error', message: 'rate limit (rateLimitExceeded) [willRetry]' })
    expect(m.streaming).toBe(true) // fix 전: Error 가 턴을 닫아 여기서 꺼졌다
    m.frame(textDelta('turn 2 answer'))
    expect(m.streaming).toBe(true)
    m.frame(messageDone)
    expect(m.streaming).toBe(false)
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
    acc.feed(encode(userEcho('과거 1', 'A')))
    acc.feed(encode(userEcho('과거 2', 'B')))
    acc.feed(encode(userEcho('현재', 'C')))
    expect(labels(acc.snapshot())).toEqual(['user', 'user', 'user'])
    expect(acc.snapshot().length).toBe(3)
  })

  it('(c) uuid 없는 user text item(과거 비-replay)은 dedup 하지 않고 보존', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(userEcho('uuid 없음', null)))
    acc.feed(encode(userEcho('uuid 없음', null)))
    expect(labels(acc.snapshot())).toEqual(['user', 'user'])
  })

  // ── ★HIGH FIX: multi-block user 라인에서 tool_result 가 dedup 으로 소실되지 않는다★ ──
  it('(c-real) uuid 를 가진 tool_result 도 dedup 하지 않고 보존(실측 fixture 정합)', () => {
    const acc = new StructuredEventAccumulator()
    // 실측 fixture(tool.jsonl)의 tool_result user 라인은 top-level uuid 를 갖는다 → 블록 json 에 uuid 실림.
    acc.feed(encode(userToolResult('toolu_1', '파일 내용', 'RESULT-UUID')))
    acc.feed(encode(userToolResult('toolu_1', '파일 내용', 'RESULT-UUID')))
    expect(labels(acc.snapshot())).toEqual(['user', 'user'])
  })

  it('(c-multi) 같은 uuid 의 text 에코 + tool_result → 둘 다 보존(tool_result 소실 금지)', () => {
    const acc = new StructuredEventAccumulator()
    // 한 user replay 라인의 두 블록(text 에코 + tool_result)이 같은 line-level uuid=U 로 온다.
    // 예전 결함: text(uuid=U)를 seenUserUuids 에 넣고 tool_result(같은 uuid U)를 "이미 본 uuid" 로
    //   스킵 → tool_result 소실.
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

  it('(c-multi) tool_result 보존 중에도 text 에코 자체는 여전히 uuid dedup 된다', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(userEcho('echo', 'U'))) // 입력-시점 합성 에코
    acc.feed(encode(userEcho('echo', 'U'))) // replay text — dedup 되어 스킵
    acc.feed(encode(userToolResult('t1', 'r', 'U'))) // replay tool_result — 보존
    expect(labels(acc.snapshot())).toEqual(['user', 'user'])
    expect(acc.snapshot().length).toBe(2)
    const first = acc.snapshot()[0]
    if (first.kind === 'structured') {
      expect((JSON.parse(first.json) as { type: string }).type).toBe('text')
    }
  })

  it('user uuid dedup 은 kind!=="user" 탈출구 이벤트에는 영향 없다', () => {
    const acc = new StructuredEventAccumulator()
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
    acc.feed(encode(textDelta('turn two')))
    expect(kinds(acc.snapshot())).toEqual(['text', 'separator', 'text'])
    expect(acc.isTurnDone()).toBe(false)
  })

  // ── ★후속 전송 flicker FIX★: 새 유저 메시지(합성 에코 포함)는 turnDone(=idle) 을 해제한다 ──
  it('MessageDone(turnDone=true) 뒤 user Structured 가 오면 turnDone 이 다시 false 로 내려간다', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('prev turn')))
    acc.feed(encode(messageDone))
    expect(acc.isTurnDone()).toBe(true)
    // 이걸 안 내리면 RichSlot 파생 streaming 이 이 순간 false 로 떨어져 WaitRow 가 깜빡 꺼진다.
    acc.feed(encode(userEcho('follow-up', 'U')))
    expect(acc.isTurnDone()).toBe(false)
    // 멱등 종점 불변 — 응답이 끝나면 MessageDone 이 다시 turnDone=true 로 세운다.
    acc.feed(encode(textDelta('assistant reply')))
    acc.feed(encode(messageDone))
    expect(acc.isTurnDone()).toBe(true)
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
    expect(ids(acc.snapshot())).toEqual(ids(first))
  })

  // ── FIX-1: snapshot immutability(copy-on-write coalescing) ──
  it('FIX-1: TextDelta 합산이 이전 snapshot 객체를 변경하지 않는다(copy-on-write)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('hello')))
    // 스냅샷 참조를 캡처(React state 에 set 한 직후를 시뮬레이션).
    const snap1 = acc.snapshot()
    const item1 = snap1[0]
    acc.feed(encode(textDelta(' world')))
    expect(item1).toEqual({ kind: 'text', text: 'hello', itemId: 0 })
    expect(acc.snapshot()[0]).toEqual({ kind: 'text', text: 'hello world', itemId: 0 })
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
    expect(acc.snapshot()).toEqual([{ kind: 'text', text: 'abc', itemId: 0 }])
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

  // ══ TurnEnd — 결말 넷이 전부 턴을 닫고, 결말은 화면 표시만 가른다 ═════════════════════
  //
  // ★이 블록이 지키는 계약★: 결말 칸이 가르는 것은 표시이지 「턴을 닫을지 말지」가 아니다. 하나라도
  //   안 닫으면 그 대화의 대기 인디케이터가 영영 돈다.

  it.each([
    ['Completed', { kind: 'Completed' } as TurnOutcome],
    ['Failed', { kind: 'Failed', detail: null } as TurnOutcome],
    ['Interrupted', { kind: 'Interrupted' } as TurnOutcome],
    ['Unknown', { kind: 'Unknown' } as TurnOutcome],
  ])('TurnEnd(%s) → 결말 무관하게 턴이 닫힌다(대기 표시 정지)', (_name, outcome) => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('working')))
    expect(streamingOf(acc, false)).toBe(true)
    acc.feed(encode(turnEnd(outcome)))
    expect(acc.isTurnDone()).toBe(true)
    expect(streamingOf(acc, false)).toBe(false)
  })

  it('TurnEnd(Completed) 는 결말 표식을 남기지 않는다 — 구분선만(평범한 완료가 경고로 보이면 안 된다)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('done')))
    acc.feed(encode(turnEnd({ kind: 'Completed' })))
    expect(kinds(acc.snapshot())).toEqual(['text', 'separator'])
  })

  it('TurnEnd(Failed) → outcome=failed 표식 + 사유(detail) 보존', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('partial')))
    acc.feed(encode(turnEnd({ kind: 'Failed', detail: 'model refused' })))
    expect(acc.snapshot()).toEqual([
      { kind: 'text', text: 'partial', itemId: 0 },
      { kind: 'outcome', outcome: 'failed', detail: 'model refused', itemId: 1 },
      { kind: 'separator', itemId: 2 },
    ])
  })

  it('TurnEnd(Failed, 사유 없음) → 표식은 남고 detail 만 null(사유를 지어내지 않는다)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(turnEnd({ kind: 'Failed', detail: null })))
    expect(acc.snapshot()).toEqual([
      { kind: 'outcome', outcome: 'failed', detail: null, itemId: 0 },
      // 선행 item(표식)이 있으므로 구분선이 뒤따른다.
      { kind: 'separator', itemId: 1 },
    ])
  })

  it('TurnEnd(Interrupted) 는 실패로 찍히지 않는다 — error item 도 outcome=failed 도 아니다', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('half a sentence')))
    acc.feed(encode(turnEnd({ kind: 'Interrupted' })))
    const marks = acc.snapshot().filter((it) => it.kind === 'outcome')
    expect(marks).toEqual([{ kind: 'outcome', outcome: 'interrupted', detail: null, itemId: 1 }])
    expect(kinds(acc.snapshot())).not.toContain('error')
  })

  it('TurnEnd(Unknown) 은 실패가 아니라 모름으로 남는다', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(turnEnd({ kind: 'Unknown' })))
    expect(acc.snapshot()).toEqual([
      { kind: 'outcome', outcome: 'unknown', detail: null, itemId: 0 },
      { kind: 'separator', itemId: 1 },
    ])
  })

  it('모르는 결말(더 새 데몬의 다섯째 값)도 모름으로 적고 턴은 닫는다 — 추측 금지', () => {
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

  it('결말 칸이 아예 없는 TurnEnd 도 던지지 않고 모름으로 닫는다', () => {
    const acc = new StructuredEventAccumulator()
    // feed 의 try/catch 는 JSON.parse 만 감싼다 — 여기서 던지면 예외가 구독 콜백까지 올라간다.
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

  it('MessageDone 뒤 TurnEnd(두 어휘가 겹쳐 와도) 구분선을 겹쳐 쌓지 않는다', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('x')))
    acc.feed(encode(messageDone))
    acc.feed(encode(turnEnd({ kind: 'Completed' })))
    expect(kinds(acc.snapshot())).toEqual(['text', 'separator'])
  })

  it('맨 앞 TurnEnd(Completed, 선행 item 없음)는 구분선을 만들지 않는다(MessageDone 과 동형)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(turnEnd({ kind: 'Completed' })))
    expect(acc.snapshot()).toEqual([])
    expect(acc.isTurnDone()).toBe(true)
  })

  it('TurnEnd 를 포함한 이벤트열도 refeed 시 동일 스냅샷(replay idempotence)', () => {
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

  // ══ 모르는 이벤트 종류 — 조용히 삼키면 스피너만 사라진다 ═══════════════════════════════
  //
  // ★재는 것★: 「화면은 한 픽셀도 안 바뀌었는데 대기 표시만 사라졌다」가 다시 일어나지 않는 것.
  //   릴리스 WebView2 에는 devtools 가 없어 console 은 이 축의 증거가 못 된다.

  // ★이 블록의 규율 — 갓 만든 누산기 하나로 판정하지 않는다★: 첫 턴은 turnDone 이 아직 false 라 어떤
  //   arm 을 쓰든 대기 표시가 선다. 결함은 **직전 턴이 닫힌 뒤**에만 나타나므로 회귀는 여러 턴으로 잰다.

  it('둘째 턴의 첫 프레임이 모르는 종류여도 대기 표시가 살아남는다(조용한 소등 회귀 가드)', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const m = slotModel()
      m.frame(textDelta('turn 1'))
      m.frame(messageDone)
      expect(m.acc.isTurnDone()).toBe(true)
      expect(m.streaming).toBe(false)

      // 둘째 턴 전송 → 그 턴의 **첫** 프레임이 모르는 종류로 온다.
      m.send()
      m.frame(unknownEvent('SomethingNewer'))
      // ★fix 전엔 여기서 꺼졌다★: 표식 item 을 더해도 turnDone 이 직전 턴의 true 로 남아 파생이 죽었고,
      //   호출자는 프레임 도착만 보고 대기 상태를 풀었다. 지금은 `feed` 가 false 를 돌려 대기가 유지된다.
      expect(m.streaming).toBe(true)
      expect(m.acc.snapshot()[m.acc.snapshot().length - 1]).toEqual({
        kind: 'unsupported',
        count: 1,
        itemId: 2,
      })

      // 아는 프레임이 오면 그때 판정을 turnDone 에 넘긴다 — 그 뒤 경계가 정상적으로 닫는다.
      m.frame(textDelta('turn 2 answer'))
      expect(m.streaming).toBe(true)
      m.frame(messageDone)
      expect(m.streaming).toBe(false)
    } finally {
      warnSpy.mockRestore()
    }
  })

  it('feed 는 못 알아들은 프레임에 false 를 돌려준다 — 대기 표시를 지키는 것이 이 반환값이다', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const acc = new StructuredEventAccumulator()
      expect(acc.feed(encode(textDelta('x')))).toBe(true)
      expect(acc.feed(encode(messageDone))).toBe(true)
      expect(acc.feed(encode({ type: 'Error', message: 'boom' }))).toBe(true)
      expect(acc.feed(encode(turnEnd({ kind: 'Completed' })))).toBe(true)
      // 빈 델타는 item 을 안 만들지만 「아는 종류」다 — 뜻을 알고 흘려보낸 것이다.
      expect(acc.feed(encode(textDelta('')))).toBe(true)
      expect(acc.feed(encode(unknownEvent('Newer')))).toBe(false)
      // malformed JSON·빈 payload 도 같은 축 — 이 프레임에서 배운 것이 없다.
      expect(acc.feed('{not json')).toBe(false)
      expect(acc.feed('')).toBe(false)
    } finally {
      warnSpy.mockRestore()
    }
  })

  it('첫 턴(갓 만든 누산기)에서도 표식 item 자체는 생긴다 — 화면에 남는 고지', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const acc = new StructuredEventAccumulator()
      acc.feed(encode(unknownEvent('SomethingNewer')))
      expect(acc.snapshot()).toEqual([{ kind: 'unsupported', count: 1, itemId: 0 }])
    } finally {
      warnSpy.mockRestore()
    }
  })

  it('모르는 type 의 표식에는 이벤트 이름·payload 가 실리지 않는다(프로토콜 낱말 비노출)', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const acc = new StructuredEventAccumulator()
      acc.feed(encode(unknownEvent('item/reasoning/summaryTextDelta')))
      expect(JSON.stringify(acc.snapshot())).not.toContain('reasoning')
      expect(JSON.stringify(acc.snapshot())).not.toContain('whatever')
      // 진단용 이름은 console 로만 나간다.
      expect(warnSpy).toHaveBeenCalledOnce()
    } finally {
      warnSpy.mockRestore()
    }
  })

  it('연속된 모르는 type 은 행을 쌓지 않고 한 표식에 합산된다', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const acc = new StructuredEventAccumulator()
      acc.feed(encode(unknownEvent('A')))
      acc.feed(encode(unknownEvent('B')))
      acc.feed(encode(unknownEvent('C')))
      expect(acc.snapshot()).toEqual([{ kind: 'unsupported', count: 3, itemId: 0 }])
      // copy-on-write — 앞서 반환한 snapshot 객체를 제자리 변경하지 않는다.
      const before = acc.snapshot()[0]
      acc.feed(encode(unknownEvent('D')))
      expect(acc.snapshot()[0]).not.toBe(before)
      expect(before).toEqual({ kind: 'unsupported', count: 3, itemId: 0 })
    } finally {
      warnSpy.mockRestore()
    }
  })

  // ★위 회귀의 짝(반대쪽 낭떠러지)★: 같은 결함을 `turnDone=false` 로 고치면 여기가 깨진다 — 복원된
  //   이력의 마지막 프레임이 모르는 종류이면 고칠 다음 프레임이 없어 대기 표시가 영영 돈다. 더 새 데몬이
  //   턴 끝에 새 어휘를 붙이면 복원되는 **모든** 세션이 그 모양이 되므로, 라이브 갭보다 비싸다.
  it('복원된 이력의 끝이 모르는 종류여도 대기 표시가 고착되지 않는다(turnDone 미변경의 값어치)', () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})
    try {
      const m = slotModel() // 복원 경로 = 전송 없이 이력만 다시 흐른다(awaiting 없음)
      m.frame(textDelta('x'))
      m.frame(messageDone)
      m.frame(unknownEvent('Newer')) // 더 새 데몬이 턴 끝에 붙인 새 어휘
      expect(m.acc.isTurnDone()).toBe(true)
      expect(m.streaming).toBe(false)
    } finally {
      warnSpy.mockRestore()
    }
  })

  it('모르는 type 을 포함한 이벤트열도 refeed 시 동일 스냅샷(replay idempotence)', () => {
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

// ── ADR-0231: 대기 입력(QueuedInput) arm ──────────────────────────────────────────

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

/** 대화 줄의 사용자 text 말풍선 — [uuid, 본문] 순서대로. */
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

describe('StructuredEventAccumulator — 대기 입력(ADR-0231)', () => {
  it('공유 골든: 모든 사례에서 누산기의 환원 상태가 골든 목록과 같다(agent 명부와 같은 파일)', () => {
    expect(queuedInputGolden.cases.length).toBeGreaterThan(0)
    for (const c of queuedInputGolden.cases) {
      const acc = new StructuredEventAccumulator()
      feedAll(acc, c.events.map(queuedInput))
      expect(acc.queuedRows().map(goldenRowOf), `[${c.name}] 목록`).toEqual(c.expect.items)
      // 골든엔 되울림이 없다 — 「이미 그린 uuid」 거름이 비어 있으니 그릴 목록 = 대기 칸 그대로.
      expect(listed(acc), `[${c.name}] 그릴 목록`).toEqual(
        c.expect.items.filter((r) => r.state === 'queued').map((r) => r.id),
      )
    }
  })

  it('목록 사건은 알아들은 프레임이다 — 모르는 op kind·깨진 모양은 false 이고 던지지 않는다', () => {
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

  it('배치: Delivered 는 대기 중인 id 를 목록에서 빼고 그 자리에 Queued 본문으로 말풍선을 더한다', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [textDelta('앞'), queued('X', '나중 글'), textDelta('뒤')])
    expect(listed(acc)).toEqual(['X'])
    expect(bubbles(acc)).toEqual([])
    feedAll(acc, [delivered('X'), textDelta('끝')])
    expect(kinds(acc.snapshot())).toEqual(['text', 'structured', 'text'])
    expect(bubbles(acc)).toEqual([['X', '나중 글']])
    // 합성 에코와 같은 모양 — 렌더러가 가르지 않는다.
    const bubble = acc.snapshot()[1]
    expect(bubble.kind === 'structured' && bubble.json).toBe('{"type":"text","text":"나중 글","uuid":"X"}')
    expect(listed(acc)).toEqual([])
    expect(rowIds(acc)).toEqual([])
  })

  it('turnDone: 목록 사건은 건드리지 않고, 말풍선을 그린 배치만 사용자 arm 처럼 내린다', () => {
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

  it('대기 uuid 억제 — 되울림이 Delivered 앞에 와도(접기 경로) 뒤에 와도(새 턴 경로) 말풍선은 받음 자리에 하나', () => {
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

  it('되살림: 누산기는 말풍선을 따로 안 그리고 뒤따르는 벤더 에코가 그린다', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [queued('X', '글'), dropped('X', 'Unknown')])
    expect(listed(acc)).toEqual([])
    feedAll(acc, [delivered('X')])
    expect(bubbles(acc)).toEqual([])
    feedAll(acc, [userEcho('글', 'X')])
    expect(bubbles(acc)).toEqual([['X', '글']])
  })

  it('CancelAnswered{true} 두 순서 — 둘 다 목록에서 빠지고 아무것도 안 그린다', () => {
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

  it('종결은 목록에서 빼고 알림 행을 그리지 않는다(원인 무관 · 받음 전 거절 포함)', () => {
    for (const cause of ['Withdrawn', 'Interrupted', 'AgentEnded', 'Unknown', 'Rejected'] as const) {
      const acc = new StructuredEventAccumulator()
      feedAll(acc, [textDelta('본문'), queued('X'), dropped('X', cause)])
      expect(kinds(acc.snapshot()), cause).toEqual(['text'])
      expect(listed(acc), cause).toEqual([])
    }
  })

  it('취소 대기 감춤: CancelRequested 를 환원한 순간 그릴 목록에서 빠지고 CancelFailed 뒤에도 안 돌아온다', () => {
    const ring = [queued('X'), queued('Y'), cancelRequested('X')]
    const a = new StructuredEventAccumulator()
    const b = new StructuredEventAccumulator()
    feedAll(a, ring)
    feedAll(b, ring)
    expect(listed(a)).toEqual(['Y'])
    expect(listed(b)).toEqual(listed(a))
    // 환원 상태에는 남는다(결말 대기).
    expect(a.queuedRows().map((e) => [e.id, e.phase.state])).toEqual([
      ['X', 'cancelling'],
      ['Y', 'queued'],
    ])
    feedAll(a, [cancelFailed('X')])
    expect(listed(a)).toEqual(['Y'])
    feedAll(a, [cancelAnswered('X', false)])
    expect(listed(a)).toEqual(['Y'])
    // reset 뒤 전량 재생도 같게 감춘다.
    a.reset()
    feedAll(a, [...ring, cancelFailed('X')])
    expect(listed(a)).toEqual(['Y'])
  })

  it('취소 대기의 결말: (나) Delivered = 그 자리 말풍선 · (가)·(다) = 아무것도', () => {
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

  it('넘긴 codex 항목의 ✕: 응답 false 로 곧바로 빠지고 Delivered 가 말풍선 한 벌(둘째 Delivered 는 무동작)', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [queued('X', '넘긴 글'), cancelRequested('X'), cancelAnswered('X', false)])
    expect(listed(acc)).toEqual([])
    feedAll(acc, [delivered('X'), delivered('X')])
    expect(bubbles(acc)).toEqual([['X', '넘긴 글']])
  })

  it('거절된 말풍선 지우기: Direct 말풍선 뒤 Dropped{Rejected} 는 그 행을 걷고 uuid 는 「본 것」에 남긴다', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [userEcho('보낸 글', 'X'), textDelta('답'), dropped('X', 'Rejected')])
    expect(kinds(acc.snapshot())).toEqual(['text'])
    feedAll(acc, [userEcho('보낸 글', 'X')])
    expect(bubbles(acc)).toEqual([])
  })

  it('거절만 지운다 — 끊기·에이전트 종료·모름은 Direct 말풍선을 안 건드린다', () => {
    for (const cause of ['Interrupted', 'AgentEnded', 'Unknown', 'Withdrawn'] as const) {
      const acc = new StructuredEventAccumulator()
      feedAll(acc, [userEcho('보낸 글', 'X'), dropped('X', cause)])
      expect(bubbles(acc), cause).toEqual([['X', '보낸 글']])
    }
  })

  it('받음 뒤 거절 Queued→Delivered→Dropped{Rejected}(골든과 같은 사건열): Delivered 가 그린 말풍선이 걷힌다', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [queued('X'), delivered('X'), dropped('X', 'Rejected')])
    expect(acc.snapshot()).toEqual([])
    expect(rowIds(acc)).toEqual([])
  })

  it('거절 지우기는 같은 uuid 를 공유한 tool_result 블록을 남긴다(text 말풍선만 걷는다)', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [userEcho('글', 'X'), userToolResult('tu1', 'OUT', 'X'), dropped('X', 'Rejected')])
    expect(bubbles(acc)).toEqual([])
    expect(labels(acc.snapshot())).toEqual(['user'])
    const left = acc.snapshot()[0]
    expect(left.kind === 'structured' && JSON.parse(left.json).type).toBe('tool_result')
  })

  it('판명의 사본: 링 창에 Queued 가 없어도 사본 본문으로 그 자리에 말풍선 하나', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [textDelta('답'), ackUnavailable([['X', '사본 본문']]), textDelta('더')])
    expect(kinds(acc.snapshot())).toEqual(['text', 'structured', 'text'])
    expect(bubbles(acc)).toEqual([['X', '사본 본문']])
  })

  it('판명의 사본: 아는 항목도 사본마다(목록 순) 하나씩 — 목록은 비고 이미 그린 uuid 는 안 그린다', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [queued('X', 'A'), queued('Y', 'B'), cancelRequested('Y')])
    feedAll(acc, [ackUnavailable([['X', 'A'], ['Y', 'B']])])
    expect(bubbles(acc)).toEqual([
      ['X', 'A'],
      ['Y', 'B'],
    ])
    expect(rowIds(acc)).toEqual([])
    // 코어가 판정 뒤 바꿔 적은 쌍(`Queued` + 그 사본 하나의 `AckUnavailable`)도 평범한 두 사건이다.
    feedAll(acc, [queued('Z', 'C'), ackUnavailable([['Z', 'C']]), ackUnavailable([['X', 'A']])])
    expect(bubbles(acc)).toEqual([
      ['X', 'A'],
      ['Y', 'B'],
      ['Z', 'C'],
    ])
  })

  it('이미 그린 uuid: 되울림 → Queued → Delivered = 말풍선 하나(되울림 자리) · 그 사이 그릴 목록에 없다', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [userEcho('글', 'X'), textDelta('답'), queued('X', '글')])
    expect(listed(acc)).toEqual([])
    // 환원 상태는 골든과 같다 — 목록에 올랐다가 Delivered 로 빠진다.
    expect(rowIds(acc)).toEqual(['X'])
    feedAll(acc, [delivered('X')])
    expect(rowIds(acc)).toEqual([])
    expect(bubbles(acc)).toEqual([['X', '글']])
    expect(kinds(acc.snapshot())).toEqual(['structured', 'text'])
  })

  it('이미 그린 uuid: 되울림 → Queued → AckUnavailable = 말풍선 하나', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [userEcho('글', 'X'), queued('X', '글'), ackUnavailable([['X', '글']])])
    expect(bubbles(acc)).toEqual([['X', '글']])
    expect(rowIds(acc)).toEqual([])
  })

  it('이미 그린 uuid: 되울림 → Delivered → Queued = 말풍선 하나(묘비가 늦은 Queued 를 버린다)', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [userEcho('글', 'X'), delivered('X'), queued('X', '글')])
    expect(bubbles(acc)).toEqual([['X', '글']])
    expect(rowIds(acc)).toEqual([])
    expect(listed(acc)).toEqual([])
  })

  it('멱등: reset → 같은 순서 refeed 가 대화 줄(itemId 포함)·목록·환원 상태를 그대로 재현한다', () => {
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
    expect(list).toEqual(['Z'])
  })

  it('reset 은 묘비까지 비운다 — 재생 전 링 밖의 종결이 새 링의 Queued 를 버리지 않는다', () => {
    const acc = new StructuredEventAccumulator()
    feedAll(acc, [queued('X'), delivered('X')])
    acc.reset()
    feedAll(acc, [queued('X')])
    expect(listed(acc)).toEqual(['X'])
  })
})

// ── ADR-0231: 재부착 대조(TRD §5-7) — 스냅숏 위에 스냅숏 seq 뒤의 사건을 seq 순으로 ──────────

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
/** seq 를 실어 먹인다(RichSlot 구독 콜백과 같다) — 대조는 seq 로만 선다. */
function feedAt(acc: StructuredEventAccumulator, seq: number, ev: StructuredEvent): void {
  acc.feed(encode(ev), seq)
}
const phaseOf = (acc: StructuredEventAccumulator, id: string) =>
  acc.queuedRows().find((e) => e.id === id)?.phase

describe('StructuredEventAccumulator — 재부착 대조(ADR-0231)', () => {
  it('스냅숏 seq 가 아직 안 왔으면 쥐고 기다렸다가 그 seq 가 배달되면 적용한다', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 5, textDelta('a'))
    const gen = acc.beginQueuedReconcile(7)
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('X')], 10))).toBe('held')
    expect(listed(acc)).toEqual([])
    feedAt(acc, 6, textDelta('b'))
    feedAt(acc, 9, textDelta('c'))
    expect(listed(acc), 'S 전에는 적용하지 않는다').toEqual([])
    feedAt(acc, 10, textDelta('d'))
    expect(listed(acc)).toEqual(['X'])
  })

  it('tag0 처럼 feed 를 안 지나는 프레임의 seq 도 배달로 센다', () => {
    const acc = new StructuredEventAccumulator()
    const gen = acc.beginQueuedReconcile(7)
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('X')], 3))).toBe('held')
    expect(acc.observeSeq(2)).toBe(false)
    expect(acc.observeSeq(3)).toBe(true)
    expect(listed(acc)).toEqual(['X'])
  })

  it('이미 S 까지 배달했으면 곧바로 적용한다 · as_of_seq null 이면 기다리지 않는다', () => {
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

  // TRD §5-7 의 경합: 질의가 Queued{X} 를 읽은 뒤 다른 창의 ✕ 가 낸 CancelRequested{X} 가 답보다 먼저 온다.
  it('질의 뒤 라이브 CancelRequested{X}(seq > S) 가 답보다 먼저 와도 X 가 다시 그려지지 않는다', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 10, queued('X'))
    const gen = acc.beginQueuedReconcile(7)
    feedAt(acc, 11, cancelRequested('X'))
    expect(listed(acc)).toEqual([])
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('X')], 10))).toBe('applied')
    expect(listed(acc)).toEqual([])
    expect(phaseOf(acc, 'X')).toEqual({ state: 'cancelling', answer: 'none', vendorClosed: false })
  })

  it('링에서 Queued{X} 가 밀려난 창: 대조 창의 CancelRequested{X} 가 이기고, 뒤이은 Delivered{X} 는 행 본문으로 말풍선을 그린다', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 50, textDelta('long turn'))
    const gen = acc.beginQueuedReconcile(7)
    // 누산기는 X 를 모른다 — 이 사건을 「모름 → 버린다」로 흘린다.
    feedAt(acc, 51, cancelRequested('X'))
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('X', 'held body')], 40))).toBe('applied')
    expect(listed(acc), '취소 대기는 그리지 않는다').toEqual([])
    expect(phaseOf(acc, 'X')?.state).toBe('cancelling')
    feedAt(acc, 52, delivered('X'))
    expect(bubbles(acc)).toEqual([['X', 'held body']])
    expect(rowIds(acc)).toEqual([])
  })

  it('대조 창의 Delivered{X}(누산기가 모르던 항목) → 목록에서 빠지고 말풍선은 더하지 않는다', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 50, textDelta('long turn'))
    const gen = acc.beginQueuedReconcile(7)
    feedAt(acc, 51, delivered('X'))
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('X')], 40))).toBe('applied')
    expect(rowIds(acc)).toEqual([])
    expect(bubbles(acc)).toEqual([])
  })

  it('seq ≤ S 인 사건은 다시 환원하지 않는다(스냅숏에 이미 들었다)', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 9, textDelta('a'))
    const gen = acc.beginQueuedReconcile(7)
    // S 전에 배달된 받음 불가 판명 — 스냅숏(S=11)은 그 뒤에 선 Y 를 대기로 싣는다.
    feedAt(acc, 10, ackUnavailable([]))
    feedAt(acc, 11, queued('Y'))
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('Y')], 11))).toBe('applied')
    expect(listed(acc), 'seq 10 의 AckUnavailable 을 Y 위에 다시 돌리면 Y 가 닫힌다').toEqual(['Y'])
  })

  it('AckUnavailable(seq > S) 은 모든 스냅숏 행에 걸린다', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 30, textDelta('a'))
    const gen = acc.beginQueuedReconcile(7)
    feedAt(acc, 31, ackUnavailable([]))
    expect(
      acc.offerQueuedSnapshot(gen, snapshot([queuedRow('A'), cancellingRow('B')], 20)),
    ).toBe('applied')
    expect(rowIds(acc)).toEqual([])
    expect(bubbles(acc), '대조는 말풍선을 그리지 않는다').toEqual([])
  })

  it('스냅숏 뒤에 선 항목(Queued seq > S)은 누산기 상태 그대로 · 순서 = 스냅숏 행 다음에 나머지', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 10, queued('B'))
    const gen = acc.beginQueuedReconcile(7)
    feedAt(acc, 11, queued('C'))
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('A'), queuedRow('B')], 10))).toBe('applied')
    expect(listed(acc)).toEqual(['A', 'B', 'C'])
  })

  // 머리 점프: X 를 본 뒤 flush 가 커서를 뒤의 replay 머리(20)로 건너뛰어 X 의 닫힘 사건(링에서 밀려났다)을 잃었다.
  it('스냅숏 seq 에 열려 있던 항목이 스냅숏에 없고 그 뒤에 선 적도 없으면 원인 모름으로 닫는다 — 말풍선 없이', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 10, queued('X'))
    feedAt(acc, 20, textDelta('after jump'))
    const gen = acc.beginQueuedReconcile(7)
    expect(acc.offerQueuedSnapshot(gen, snapshot([], 20))).toBe('applied')
    expect(rowIds(acc)).toEqual([])
    expect(bubbles(acc)).toEqual([])
    // 묘비라 늦게 온 같은 id 의 Queued 가 다시 세우지 못한다.
    feedAt(acc, 21, queued('X'))
    expect(rowIds(acc)).toEqual([])
  })

  it('모르는 낱말의 행도 그 id 는 「있다」로 센다 — 닫지 않고 누산기 상태 그대로 둔다', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 10, queued('X'))
    feedAt(acc, 20, textDelta('a'))
    const gen = acc.beginQueuedReconcile(7)
    const rows: SnapshotRow[] = [{ id: 'X', text: 'text X', state: 'parked', cancel: null }]
    expect(acc.offerQueuedSnapshot(gen, snapshot(rows, 20))).toBe('applied')
    expect(listed(acc)).toEqual(['X'])
    expect(phaseOf(acc, 'X')).toEqual({ state: 'queued' })
  })

  it('적어 둔 Queued{X} 가 seq > S 면 남기고, seq ≤ S 면 그 기록으로는 남기지 않는다', () => {
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
    expect(listed(atS), 'S 에 선 항목은 스냅숏에 들었어야 한다 — 없으면 S 까지 닫혔다').toEqual([])
  })

  it('누산기가 링으로 아는 항목은 대조 뒤에도 같은 상태다(같은 환원 규칙)', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 1, queued('X'))
    feedAt(acc, 2, cancelRequested('X'))
    feedAt(acc, 3, cancelFailed('X'))
    const before = acc.queuedRows().map((e) => ({ ...e }))
    const gen = acc.beginQueuedReconcile(7)
    expect(acc.offerQueuedSnapshot(gen, snapshot([cancellingRow('X', 'not_removed')], 3))).toBe('applied')
    expect(acc.queuedRows()).toEqual(before)
  })

  it('unconfirmed 는 queued 로 읽는다(여느 회색 항목)', () => {
    const acc = new StructuredEventAccumulator()
    const gen = acc.beginQueuedReconcile(7)
    acc.offerQueuedSnapshot(gen, snapshot([{ id: 'U', text: 'u', state: 'unconfirmed', cancel: null }], null))
    expect(listed(acc)).toEqual(['U'])
  })

  it('모르는 낱말의 행은 그 행만 빠진다 — 던지지 않는다', () => {
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

  it('다른 화신 표식의 답은 버린다', () => {
    const acc = new StructuredEventAccumulator()
    const gen = acc.beginQueuedReconcile(7)
    expect(acc.offerQueuedSnapshot(gen, snapshot([queuedRow('X')], null, 8))).toBe('stale')
    expect(listed(acc)).toEqual([])
  })

  // TRD §5-7 대조 세대: 질의 뒤 붙듦 넘침(startBuffering) → 기록이 비워진 채 옛 답이 오면 ✕ 한 항목이 되살아난다.
  it('세대: 답이 오기 전에 대조를 버렸으면(버퍼 국면) 늦게 온 옛 답은 버리고 다음 질의의 답만 적용한다', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 50, textDelta('long turn'))
    const oldGen = acc.beginQueuedReconcile(7)
    acc.abandonQueuedReconcile() // 'buffering'
    feedAt(acc, 51, cancelRequested('X')) // 다른 창의 ✕ — 기록할 대조가 없다
    expect(acc.offerQueuedSnapshot(oldGen, snapshot([queuedRow('X')], 40))).toBe('stale')
    expect(listed(acc), '옛 스냅숏의 Queued{X} 가 되살아나지 않는다').toEqual([])

    const gen = acc.beginQueuedReconcile(7)
    expect(acc.offerQueuedSnapshot(gen, snapshot([cancellingRow('X')], 51))).toBe('applied')
    expect(listed(acc)).toEqual([])
    expect(phaseOf(acc, 'X')?.state).toBe('cancelling')
  })

  it('세대: reset() 도 진행 중인 대조와 쥔 답을 버린다(세대는 되돌리지 않는다)', () => {
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

  it('질의 실패는 그 세대만 버린다 — 그 사이 새로 연 대조는 건드리지 않는다', () => {
    const acc = new StructuredEventAccumulator()
    const g1 = acc.beginQueuedReconcile(7)
    const g2 = acc.beginQueuedReconcile(7)
    acc.abandonQueuedReconcile(g1)
    expect(acc.offerQueuedSnapshot(g2, snapshot([queuedRow('X')], null))).toBe('applied')
    expect(listed(acc)).toEqual(['X'])
  })

  it('seq 없이 먹인 프레임은 대조 기록에 남지 않는다(seq 를 빼는 것은 시험 편의뿐)', () => {
    const acc = new StructuredEventAccumulator()
    feedAt(acc, 20, textDelta('a'))
    const gen = acc.beginQueuedReconcile(7)
    feedAll(acc, [cancelRequested('X')])
    acc.offerQueuedSnapshot(gen, snapshot([queuedRow('X')], 10))
    expect(listed(acc)).toEqual(['X'])
  })
})
