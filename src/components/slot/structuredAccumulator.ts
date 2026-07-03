// RichSlot 라이브 누산기(tag1 StructuredEvent) — 프레임 1건 = StructuredEvent JSON 1건 (순수 TS, React 무관).
//
// ★S14 NDJSON 누산기(lab/richslot/streamParse.ts)와 다른 이유★: S15(ADR-0045)부터 라이브 구조화 출력은
//   백엔드가 정제해 **binary frame tag1** 로 흘린다 — payload 는 self-describing StructuredEvent JSON 1건
//   (internally-tagged `"type"` 판별자)이지 NDJSON 라인 스트림이 아니다. 그래서 라인 재조립(개행 분할)이
//   필요 없고, feed 한 번 = 이벤트 하나다. NDJSON 통로(StreamAccumulator)는 fixture 스파이크(FixtureRichSlot)
//   가 계속 쓰므로 남겨둔다(이 누산기는 라이브 tag1 전용).
//
// ★MVP 스코프(ADR-0045 §52 렌더 DEFER)★: 이번엔 TextDelta 만 텍스트로 누적해 렌더한다. ToolCall/Usage/
//   MessageDone/Error/Structured 는 파싱만 하고 화면 표시는 후속(여기선 무시). 그래서 렌더 산출은 최소 —
//   누적 텍스트를 assistant 메시지 1개(text 블록 하나)로 노출해 기존 ChatLayout(RichMessage[] 소비)에
//   그대로 태운다(types.ts RichMessage 수정 없이 재사용).

import type { StructuredEvent } from '../../../crates/engram-dashboard-protocol/bindings/StructuredEvent'
import type { RichMessage } from '../../lab/richslot/types'

/**
 * 라이브 tag1 누산기. `feed` 로 tag1 payload 바이트(StructuredEvent JSON 1건)를 밀어 넣으면 파싱해
 * TextDelta 텍스트를 이어붙인다. `snapshot()` 이 반환하는 RichMessage[](assistant 1개·text 블록 1개)를
 * ChatLayout 이 렌더한다. 재구독(replay) 전 `reset()` 으로 초기화(terminal.reset() 규율의 RichSlot 판).
 */
export class StructuredEventAccumulator {
  // 누적 텍스트(TextDelta.text 이어붙임). 여러 TextDelta 가 한 assistant 응답을 조각내 오므로 concat.
  private text = ''
  // 마지막 이벤트가 MessageDone/Error 였는가(턴 종료) — 라이브 입력 UX 의 streaming/idle 힌트(옵션).
  private turnDone = false

  /**
   * tag1 payload(StructuredEvent JSON UTF-8 바이트) 1건을 먹인다. 라이브 경로는 항상 Uint8Array,
   * 문자열은 테스트/편의용. tag1 은 프레임 1개 = 이벤트 1개라 라인 재조립·버퍼링이 필요 없다.
   */
  feed(payload: Uint8Array | string): void {
    const json = typeof payload === 'string' ? payload : new TextDecoder('utf-8').decode(payload)
    if (!json) return
    let ev: StructuredEvent
    try {
      ev = JSON.parse(json) as StructuredEvent
    } catch {
      // 통로는 바보 파이프(무정제) — malformed JSON 이 와도 누산기가 죽지 않게 조용히 스킵(방어).
      return
    }
    this.consume(ev)
  }

  private consume(ev: StructuredEvent): void {
    switch (ev.type) {
      case 'TextDelta':
        // 텍스트 조각 이어붙임(누적). 새 델타 도착 = 에이전트 작업 중 → idle 해제.
        this.text += ev.text
        this.turnDone = false
        break
      case 'MessageDone':
      case 'Error':
        // 턴 종료 신호(렌더 대상 아님 — Error 표시는 렌더 DEFER). streaming/idle 힌트만 내린다.
        this.turnDone = true
        break
      case 'ToolCall':
      case 'Usage':
      case 'Structured':
        // ★렌더 DEFER(ADR-0045 §52)★: MVP 에선 파싱만 하고 표시하지 않는다(후속 모듈에서 블록화).
        //   무시하되 turnDone 은 건드리지 않는다(응답 진행 중일 수 있음).
        break
    }
  }

  /**
   * 현재까지 누적된 렌더 메시지. TextDelta 가 하나도 없으면 빈 배열(빈 assistant 메시지 렌더 방지).
   * 있으면 assistant 메시지 1개(text 블록 1개)로 노출 — ChatLayout 이 Markdown 으로 그린다.
   */
  snapshot(): RichMessage[] {
    if (this.text === '') return []
    return [{ role: 'assistant', blocks: [{ type: 'text', text: this.text }] }]
  }

  /** 마지막 신호가 MessageDone/Error(턴 종료)였는가 — streaming/idle 표시 힌트(옵션). */
  isTurnDone(): boolean {
    return this.turnDone
  }

  /** 재구독(replay) 전 초기화 — 히스토리 전체가 다시 흘러 동일 상태로 재구성되게 한다. */
  reset(): void {
    this.text = ''
    this.turnDone = false
  }
}
