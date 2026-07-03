// StructuredEventAccumulator 단위테스트 — tag1 StructuredEvent 누적(TextDelta → 텍스트, 나머지 DEFER).
//
// 라이브 RichSlot 이 tag1 payload(StructuredEvent JSON 1건)를 이 누산기에 흘려 텍스트를 누적한다.
// 프레임 1건 = 이벤트 1건(NDJSON 아님)이라 라인 재조립은 없다. MVP 스코프(ADR-0045 §52): TextDelta 만
// 렌더, ToolCall/Usage/MessageDone/Error/Structured 는 파싱만·무시(렌더 DEFER).

import { describe, expect, it } from 'vitest'

import { StructuredEventAccumulator } from './structuredAccumulator'
import type { StructuredEvent } from '../../../crates/engram-dashboard-protocol/bindings/StructuredEvent'

/** StructuredEvent → tag1 payload 바이트(라이브 경로가 받는 형태). */
function encode(ev: StructuredEvent): Uint8Array {
  return new TextEncoder().encode(JSON.stringify(ev))
}
function textDelta(text: string): StructuredEvent {
  return { type: 'TextDelta', text, turn_id: null, message_id: null }
}

describe('StructuredEventAccumulator', () => {
  it('TextDelta 1건 → assistant text 메시지로 노출', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('hello')))
    expect(acc.snapshot()).toEqual([{ role: 'assistant', blocks: [{ type: 'text', text: 'hello' }] }])
  })

  it('여러 TextDelta 는 한 메시지로 이어붙인다(concat)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('Hel')))
    acc.feed(encode(textDelta('lo, ')))
    acc.feed(encode(textDelta('world')))
    const snap = acc.snapshot()
    expect(snap.length).toBe(1)
    expect(snap[0].blocks).toEqual([{ type: 'text', text: 'Hello, world' }])
  })

  it('TextDelta 없으면 빈 배열(빈 assistant 메시지 렌더 방지)', () => {
    const acc = new StructuredEventAccumulator()
    // ToolCall/Usage 만 와도 텍스트 0 → 빈 스냅샷(렌더 DEFER).
    acc.feed(encode({ type: 'ToolCall', name: 'Read', args_json: '{}', id: null, turn_id: null, message_id: null }))
    acc.feed(encode({ type: 'Usage', input_tokens: 10, output_tokens: 5, turn_id: null }))
    expect(acc.snapshot()).toEqual([])
  })

  it('문자열 입력(테스트/편의)도 동일 처리', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(JSON.stringify(textDelta('str-path')))
    expect(acc.snapshot()[0].blocks).toEqual([{ type: 'text', text: 'str-path' }])
  })

  it('MessageDone → turnDone=true(TextDelta 는 progress → false)', () => {
    const acc = new StructuredEventAccumulator()
    expect(acc.isTurnDone()).toBe(false)
    acc.feed(encode(textDelta('working')))
    expect(acc.isTurnDone()).toBe(false)
    acc.feed(encode({ type: 'MessageDone', turn_id: null, message_id: null }))
    expect(acc.isTurnDone()).toBe(true)
    // 새 TextDelta 도착 → 다시 진행 중.
    acc.feed(encode(textDelta(' more')))
    expect(acc.isTurnDone()).toBe(false)
  })

  it('Error variant → turnDone=true(표시는 DEFER, 텍스트 누적 안 함)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('partial')))
    acc.feed(encode({ type: 'Error', message: 'boom' }))
    expect(acc.isTurnDone()).toBe(true)
    // Error 는 텍스트로 누적하지 않는다(렌더 DEFER) — 기존 텍스트만 남는다.
    expect(acc.snapshot()[0].blocks).toEqual([{ type: 'text', text: 'partial' }])
  })

  it('malformed JSON → 조용히 스킵(누산기 안 죽음, 방어)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(new TextEncoder().encode('{not json'))
    acc.feed(encode(textDelta('after')))
    expect(acc.snapshot()[0].blocks).toEqual([{ type: 'text', text: 'after' }])
  })

  it('reset → 누적/turnDone 초기화(재구독 replay 규율)', () => {
    const acc = new StructuredEventAccumulator()
    acc.feed(encode(textDelta('old')))
    acc.feed(encode({ type: 'MessageDone', turn_id: null, message_id: null }))
    acc.reset()
    expect(acc.snapshot()).toEqual([])
    expect(acc.isTurnDone()).toBe(false)
    // reset 후 새 스트림이 동일 상태로 재구성.
    acc.feed(encode(textDelta('new')))
    expect(acc.snapshot()[0].blocks).toEqual([{ type: 'text', text: 'new' }])
  })
})
