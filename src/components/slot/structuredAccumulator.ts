// RichSlot 라이브 누산기(tag1 StructuredEvent) — 프레임 1건 = StructuredEvent JSON 1건 (순수 TS, React 무관).
//
// ★S14 NDJSON 누산기(lab/richslot/streamParse.ts)와 다른 이유★: S15(ADR-0045)부터 라이브 구조화 출력은
//   백엔드가 정제해 **binary frame tag1** 로 흘린다 — payload 는 self-describing StructuredEvent JSON 1건
//   (internally-tagged `"type"` 판별자)이지 NDJSON 라인 스트림이 아니다. 그래서 라인 재조립(개행 분할)이
//   필요 없고, feed 한 번 = 이벤트 하나다. NDJSON 통로는 fixture 스파이크(lab/richslot/fixtureParse.ts)가
//   계속 쓴다(그건 통짜 파서, 이 누산기는 라이브 tag1 전용).
//
// ★렌더 모델 = 순서 보존 item 스트림(ADR-0045 §52 렌더, 사용자 결정 2026-07-05)★: 이벤트가 도착한
//   순서 그대로 item 배열에 쌓는다. text 는 인접분끼리 이어붙이고(assistant 응답 조각), ToolCall/Usage/
//   Error 는 각각 칩 item 1개로, MessageDone(턴 종료)은 구분선(separator) item 으로 삽입한다. RichSlot 이
//   이 순서 그대로 렌더한다(text=Markdown, 칩=클릭 펼침 한줄, separator=수평선).
//
// ★idempotent 재구성 불변식(replay 안전 — 왜 중요한가)★: 웹뷰 리로드/슬롯 재배정 시 클라 공유버퍼가
//   히스토리 전체를 seq 순서로 다시 흘린다(replay, ADR-0043). 구독 effect 가 reset() 후 재구독하므로,
//   같은 이벤트열을 다시 먹여도 동일 스냅샷이 나와야 한다(그래야 리로드가 화면을 그대로 복원). 이 누산기는
//   상태를 오직 feed 순서로만 세우고(순서 보존) reset 이 전부 비우므로, reset→같은 순서 refeed = 동일 결과다.
//   상류(ProtocolClient)가 seq dedup·순서 보장을 하므로 이 누산기는 중복/역전 방어를 따로 하지 않는다.

import type { StructuredEvent } from '../../../crates/engram-dashboard-protocol/bindings/StructuredEvent'

/** 순서 보존 렌더 item — RichSlot 이 배열 순서 그대로 그린다.
 *  `itemId` 는 누산기 인스턴스 내 단조 증가 id(reset 시 0 복귀, React key 로 사용). */
export type StructuredItem =
  // assistant 텍스트 조각(인접 TextDelta 를 이어붙인 누적 세그먼트). Markdown 으로 렌더.
  | { kind: 'text'; text: string; itemId: number }
  // 도구 호출 칩 — 접힌 한 줄(name), 펼치면 args_json. id 는 백엔드 tool-use id.
  | { kind: 'tool'; name: string; argsJson: string; id: string | null; itemId: number }
  // 토큰 사용량 칩 — 접힌 한 줄(in/out 토큰), 펼치면 상세.
  | { kind: 'usage'; inputTokens: number; outputTokens: number; itemId: number }
  // 에러 칩 — 접힌 한 줄(요약), 펼치면 전체 메시지.
  | { kind: 'error'; message: string; itemId: number }
  // 탈출구 이벤트 칩(codex/gemini·API 모델 누수 흡수) — kind 라벨 + json 상세.
  | { kind: 'structured'; label: string; json: string; itemId: number }
  // 턴 경계(구분선) — MessageDone 로 삽입(ADR-0045 turn 경계 semantics).
  | { kind: 'separator'; itemId: number }

/**
 * 라이브 tag1 누산기. `feed` 로 tag1 payload 바이트(StructuredEvent JSON 1건)를 밀어 넣으면 파싱해
 * 순서 보존 item 스트림에 누적한다. `snapshot()` 이 반환하는 StructuredItem[] 를 RichSlot 이 그린다.
 * 재구독(replay) 전 `reset()` 으로 초기화(terminal.reset() 규율의 RichSlot 판 — 위 idempotent 불변식).
 */
export class StructuredEventAccumulator {
  // 도착 순서 그대로의 렌더 item 스트림.
  private items: StructuredItem[] = []
  // 마지막 이벤트가 MessageDone/Error 였는가(턴 종료) — 라이브 입력 UX 의 streaming/idle 힌트(옵션).
  private turnDone = false
  // 단조 증가 item id — reset() 시 0 복귀. 같은 이벤트열을 refeed 하면 동일 id 를 재현(replay idempotence).
  private nextId = 0
  // ★user 메시지 uuid dedup(blunt-suppress → uuid dedup 교체, text 블록 한정)★: json 모드는 write_input
  //   직후 세션이 합성 유저 에코(Structured{kind:"user", json 에 uuid 부착})를 먼저 흘리고, 이후 claude 가
  //   --replay-user-messages 로 **같은 uuid** 를 그대로 되울린다(백엔드 decoder 가 line-level uuid 를
  //   블록 json 에 실어 통과). 여기서 이미 본 uuid 의 user item 은 스킵해 정확히 한 개만 남긴다.
  //   ★dedup 대상은 `type==="text"` user 블록뿐★: 합성 에코가 만드는 블록이 text 하나뿐이라(dedup 짝도
  //   text 에서만 발생), 같은 replay 라인의 tool_result 등 비-text 블록은 같은 uuid 를 공유해도 dedup 하지
  //   않고 보존한다(extractUserUuid 가 non-text 에 null 반환 — multi-block tool_result 소실 방지 HIGH FIX).
  //   uuid 없는 user item(과거/비-replay)도 dedup 하지 않고 전부 보존한다(vanish 방지).
  //   reset() 이 비우므로 replay idempotence 유지(refeed 시 같은 uuid 를 같은 순서로 다시 보고 재수렴).
  private seenUserUuids = new Set<string>()

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
    } catch (err) {
      // 통로는 바보 파이프(무정제) — malformed JSON 은 프로토콜 수준 데이터 유실 신호이므로 경고 후 스킵.
      console.warn('[structuredAccumulator] tag1 JSON 파싱 실패 — 이벤트 스킵:', err)
      return
    }
    this.consume(ev)
  }

  private consume(ev: StructuredEvent): void {
    switch (ev.type) {
      case 'TextDelta': {
        // 빈 델타("")는 phantom item(빈 Markdown 블록·의미 없는 구분선 유발)을 만들지 않도록 스킵.
        if (!ev.text) break
        // 텍스트 조각 이어붙임 — 직전 item 이 text 면 copy-on-write concat, 아니면 새 text item 을 연다.
        //   copy-on-write: 이전에 반환된 snapshot() 참조가 이 객체를 가리키므로 제자리 변경 금지.
        //   (칩/구분선 뒤에 다시 텍스트가 오면 별도 세그먼트로 뜬다 — 순서 보존.)
        const last = this.items[this.items.length - 1]
        if (last && last.kind === 'text') {
          // 새 객체로 교체(copy-on-write) — 기존 snapshot 참조가 가리키던 객체는 불변.
          this.items[this.items.length - 1] = { ...last, text: last.text + ev.text }
        } else {
          this.items.push({ kind: 'text', text: ev.text, itemId: this.nextId++ })
        }
        this.turnDone = false // 새 델타 = 에이전트 작업 중 → idle 해제.
        break
      }
      case 'ToolCall':
        this.items.push({
          kind: 'tool',
          name: ev.name,
          argsJson: ev.args_json,
          id: ev.id,
          itemId: this.nextId++,
        })
        this.turnDone = false // 도구 호출 = 응답 진행 중.
        break
      case 'Usage':
        this.items.push({
          kind: 'usage',
          inputTokens: ev.input_tokens,
          outputTokens: ev.output_tokens,
          itemId: this.nextId++,
        })
        break
      case 'Error':
        // 에러 칩 삽입 + 턴 종료 신호. (텍스트로 누적하지 않는다 — 칩으로 별도 표시.)
        this.items.push({ kind: 'error', message: ev.message, itemId: this.nextId++ })
        this.turnDone = true
        break
      case 'Structured': {
        // ★user uuid dedup(text 블록 한정)★: user 항목은 합성 입력-시점 에코와 claude replay 가
        //   **같은 uuid** 로 두 번 온다(백엔드 uuid dedup 계약). 이미 본 uuid 면 스킵해 한 개만 남긴다.
        //   단 dedup 대상은 `type==="text"` user 블록뿐이다 — 합성 에코가 만드는 블록이 text 하나뿐이라
        //   dedup 짝도 text 에서만 생긴다. 한 replay 라인의 tool_result 등 비-text 블록은 같은 line-level
        //   uuid 를 공유하더라도 extractUserUuid 가 null 을 돌려주므로 dedup 대상이 아니라 항상 보존된다
        //   (multi-block 에서 tool_result 소실 방지 — HIGH FIX). uuid 가 없으면(과거/비-replay) 그대로 보존.
        //   (kind!=='user' 탈출구 이벤트는 dedup 대상 아님 — uuid 개념이 없다.)
        if (ev.kind === 'user') {
          const uuid = extractUserUuid(ev.json)
          if (uuid !== null) {
            if (this.seenUserUuids.has(uuid)) break // 중복(합성 에코 ↔ replay) → 스킵
            this.seenUserUuids.add(uuid)
          }
          // ★새 유저 턴 시작 → turnDone(=idle) 해제★: 직전 MessageDone 이 turnDone=true 로 뒀는데, 새 유저
          //   메시지가 오면 어시스턴트 응답을 기다리는 중이다(더는 idle 아님). 이걸 안 내리면 후속 전송 시
          //   합성 user 에코가 awaiting 을 해제하는 순간(RichSlot 구독 콜백 setAwaiting(false)) 파생
          //   streaming 이 false 로 떨어져, 첫 assistant 토큰 전까지 대기 인디케이터(WaitRow)가 깜빡 꺼진다
          //   (후속 전송 flicker). replay 멱등 유지: 완결 히스토리의 최종 MessageDone 이 다시 turnDone=true 로
          //   세우므로 refeed 후 최종 상태 불변 — 중간 전이만 정확해진다. (dedup 스킵분은 위 break 로 여기 못 옴.)
          this.turnDone = false
        }
        // 탈출구 이벤트 — 알 수 없는 종류(kind)를 칩으로 흘려 유실 방지. (user 는 위에서 turnDone 해제,
        //   그 외 kind 의 turnDone 은 건드리지 않는다.)
        this.items.push({ kind: 'structured', label: ev.kind, json: ev.json, itemId: this.nextId++ })
        break
      }
      case 'MessageDone':
        // ADR-0045: decoder(backend claude.rs)가 claude 결과 한 줄·한 턴마다 MessageDone 을 정확히 1회
        // 발행하며, turn_id 는 현재 항상 None 이다. 따라서 MessageDone 이 유일하게 신뢰할 수 있는 턴
        // 경계 트리거다(turn_id 로 교체하면 현재 always-None 이라 경계가 사라짐 — 재론 방지).
        // 연속 MessageDone(빈 턴)으로 구분선이 겹쳐 쌓이지 않게 가드한다.
        // 선행 item 이 없는 경우(빈 스냅샷)도 구분선 생략 — leading-separator 방지.
        if (this.items.length > 0 && this.items[this.items.length - 1].kind !== 'separator') {
          this.items.push({ kind: 'separator', itemId: this.nextId++ })
        }
        this.turnDone = true
        break
    }
  }

  /** 현재까지 누적된 렌더 item 스트림(내부 배열 참조). React 소비자는 [...snapshot()] 로 새 참조를 떠서 set. */
  snapshot(): StructuredItem[] {
    return this.items
  }

  /** 마지막 신호가 MessageDone/Error(턴 종료)였는가 — streaming/idle 표시 힌트(옵션). */
  isTurnDone(): boolean {
    return this.turnDone
  }

  /** 재구독(replay) 전 초기화 — 히스토리 전체가 다시 흘러 동일 상태로 재구성되게 한다(위 idempotent 불변식). */
  reset(): void {
    this.items = []
    this.turnDone = false
    this.nextId = 0
    this.seenUserUuids.clear()
  }
}

/**
 * user Structured item 의 json 에서 dedup 키 `uuid` 를 뽑는다(합성 에코 · replay 공통 부착). 절대 throw 안 함.
 * 백엔드가 심는 shape: `{"type":"text","text":…,"uuid":"X"}`(합성 에코) / replay 블록도 같은 위치에 uuid.
 * uuid 가 없거나 문자열이 아니면 null(→ 호출자가 dedup 하지 않고 보존).
 *
 * ★dedup 은 `type==="text"` 블록에만★(multi-block 소실 방지 — HIGH FIX): 백엔드 decoder 는
 *   한 user replay 라인의 **모든 content 블록**(text·tool_result 등)에 같은 line-level uuid 를
 *   실어 통과시킨다(claude.rs consume_block). 합성 에코가 만들 수 있는 블록은 오직 단일
 *   `{"type":"text"}` 뿐이므로(user_text_echo_json), dedup 짝이 생기는 것도 text 블록뿐이다.
 *   여기서 type 을 안 보고 uuid 만 뽑으면, 한 라인의 text(에코) + tool_result 가 **같은 uuid** 를
 *   공유해 tool_result 가 "이미 본 uuid" 로 스킵돼 소실된다(도구 OUT 본문 사라짐). 그래서
 *   `type==="text"` 인 user 블록만 uuid 를 반환하고, tool_result 등 비-text 블록은 null →
 *   dedup 제외 → 항상 보존한다(uuid 유무 무관). (권장 seam — 프론트 국소 수정, dedup 의도와 일치.)
 */
function extractUserUuid(json: string): string | null {
  try {
    const parsed: unknown = JSON.parse(json)
    if (parsed !== null && typeof parsed === 'object') {
      const obj = parsed as Record<string, unknown>
      // 합성 에코와 dedup 짝이 되는 건 text 블록뿐 — 비-text(tool_result 등)는 dedup 제외(항상 보존).
      if (obj['type'] !== 'text') return null
      const uuid = obj['uuid']
      if (typeof uuid === 'string' && uuid.length > 0) return uuid
    }
    return null
  } catch {
    return null
  }
}
