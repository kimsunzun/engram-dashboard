import { describe, expect, it, vi } from 'vitest'

import { StructuredEventAccumulator, type StructuredItem } from './structuredAccumulator'
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
