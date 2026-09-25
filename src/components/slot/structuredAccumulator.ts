// RichSlot 라이브 누산기(tag1 StructuredEvent) — 프레임 1건 = StructuredEvent JSON 1건 (순수 TS, React 무관).
//
// ★S14 NDJSON 누산기(lab/richslot/streamParse.ts)와 다른 이유★: S15(ADR-0045)부터 라이브 구조화 출력은
//   백엔드가 정제해 **binary frame tag1** 로 흘린다 — payload 는 self-describing StructuredEvent JSON 1건
//   (internally-tagged `"type"` 판별자)이지 NDJSON 라인 스트림이 아니다. 그래서 라인 재조립(개행 분할)이
//   필요 없고, feed 한 번 = 이벤트 하나다. NDJSON 통로는 fixture 스파이크(lab/richslot/fixtureParse.ts)가
//   계속 쓴다(그건 통짜 파서, 이 누산기는 라이브 tag1 전용).
//
// ★렌더 모델 = 순서 보존 item 스트림(ADR-0045 §52 렌더, 사용자 결정 2026-07-05)★: 이벤트가 도착한
//   순서 그대로 item 배열에 쌓는다.
//
// ★idempotent 재구성 불변식(replay 안전 — 왜 중요한가)★: 웹뷰 리로드/슬롯 재배정 시 클라 공유버퍼가
//   히스토리 전체를 seq 순서로 다시 흘린다(replay, ADR-0043). 구독 effect 가 reset() 후 재구독하므로,
//   같은 이벤트열을 다시 먹여도 동일 스냅샷이 나와야 한다(그래야 리로드가 화면을 그대로 복원). 이 누산기는
//   상태를 오직 feed 순서로만 세우고(순서 보존) reset 이 전부 비우므로, reset→같은 순서 refeed = 동일 결과다.
//   상류(ProtocolClient)가 seq dedup·순서 보장을 하므로 이 누산기는 중복/역전 방어를 따로 하지 않는다.

import type { QueuedInputEvent } from '../../../crates/engram-dashboard-protocol/bindings/QueuedInputEvent'
import type { StructuredEvent } from '../../../crates/engram-dashboard-protocol/bindings/StructuredEvent'
import type { TurnOutcome } from '../../../crates/engram-dashboard-protocol/bindings/TurnOutcome'
import { QueuedInputRegistry, type QueuedEntry } from './queuedInputReducer'

/**
 * 턴이 **정상 완료가 아닌** 결말로 닫혔을 때 화면에 남기는 표식 — 중립 어휘(백엔드 이름이 없다).
 * ★'interrupted' 를 실패로 그리지 말 것★ — 사용자가 끊은 것은 이 저장소에서 1급 정상 경로다.
 */
export type TurnOutcomeMark = 'failed' | 'interrupted' | 'unknown'

/** `itemId` 는 누산기 인스턴스 내 단조 증가 id(reset 시 0 복귀, React key 로 사용). */
export type StructuredItem =
  | { kind: 'text'; text: string; itemId: number }
  // id 는 백엔드 tool-use id.
  | { kind: 'tool'; name: string; argsJson: string; id: string | null; itemId: number }
  | { kind: 'usage'; inputTokens: number; outputTokens: number; itemId: number }
  | { kind: 'error'; message: string; itemId: number }
  // 탈출구 이벤트(codex/gemini·API 모델 누수 흡수).
  | { kind: 'structured'; label: string; json: string; itemId: number }
  | { kind: 'separator'; itemId: number }
  // 턴 결말 표식 — `detail` = 실패 사유(상대가 줬을 때만). 정상 완료는 이 item 을 만들지 않는다(구분선만).
  | { kind: 'outcome'; outcome: TurnOutcomeMark; detail: string | null; itemId: number }
  // 이 셸이 모르는 이벤트가 왔다는 표식. `count` = 연속 누적분. ★원본 payload 는 싣지 않는다★(아래 default arm).
  | { kind: 'unsupported'; count: number; itemId: number }

export class StructuredEventAccumulator {
  private items: StructuredItem[] = []
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
  // ADR-0231: 대기 입력의 말풍선도 이 집합으로 「한 id 에 한 번」을 지킨다 — `Delivered` 배치·판명 사본이
  //   여기 넣고, 여기 든 uuid 는 목록(`snapshotQueued`)에도 안 그린다.
  private seenUserUuids = new Set<string>()
  // ADR-0231: 대기 입력 명부의 환원 상태(agent 명부와 같은 규칙 · 같은 골든). 그리기 거름은 이 밖에서 한다.
  private readonly queued = new QueuedInputRegistry()

  /**
   * 라이브 경로는 항상 Uint8Array, 문자열은 테스트/편의용.
   * tag1 은 프레임 1개 = 이벤트 1개라 라인 재조립·버퍼링이 필요 없다.
   *
   * @returns 이 프레임을 **이해했는가**. `false` = 파싱에 실패했거나 이 셸이 모르는 종류라, 돌아온
   *   상태(`snapshot`·`isTurnDone`)에 이 프레임의 뜻이 하나도 반영되지 않았다는 뜻이다.
   *   ★호출자는 이 값을 보고 자기 대기 상태를 누산기에 넘길지 정한다★ — 프레임이 왔다는 사실만으로
   *   넘기면, 못 알아들은 프레임이 「응답이 왔다」로 둔갑해 대기 표시가 꺼진다(RichSlot 의 `awaiting`).
   */
  feed(payload: Uint8Array | string): boolean {
    const json = typeof payload === 'string' ? payload : new TextDecoder('utf-8').decode(payload)
    if (!json) return false
    let ev: StructuredEvent
    try {
      ev = JSON.parse(json) as StructuredEvent
    } catch (err) {
      // 통로는 바보 파이프(무정제) — malformed JSON 은 프로토콜 수준 데이터 유실 신호이므로 경고 후 스킵.
      console.warn('[structuredAccumulator] tag1 JSON 파싱 실패 — 이벤트 스킵:', err)
      return false
    }
    return this.consume(ev)
  }

  /** @returns 위 `feed` 와 같은 뜻 — 아는 종류였으면 true. */
  private consume(ev: StructuredEvent): boolean {
    switch (ev.type) {
      case 'TextDelta': {
        // 빈 델타("")는 phantom item(빈 Markdown 블록·의미 없는 구분선 유발)을 만들지 않도록 스킵.
        if (!ev.text) break
        // copy-on-write: 이전에 반환된 snapshot() 참조가 이 객체를 가리키므로 제자리 변경 금지.
        const last = this.items[this.items.length - 1]
        if (last && last.kind === 'text') {
          this.items[this.items.length - 1] = { ...last, text: last.text + ev.text }
        } else {
          this.items.push({ kind: 'text', text: ev.text, itemId: this.nextId++ })
        }
        this.turnDone = false
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
        this.turnDone = false
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
        // 텍스트로 누적하지 않고 별도 item 으로 표시한다.
        // ★턴을 닫지 않는다 — 되살리지 말 것★: 이 어휘는 양쪽 백엔드에서 「턴 경계 아님」이다. codex 는
        //   재시도 가능한 스트림 오류(과부하·rate limit)를 이걸로 내고 그 줄 뒤에도 같은 턴이 이어지며
        //   (`backend/codex/decoder.rs` 의 `error` 알림), claude 는 실패한 턴조차 `MessageDone` 으로 닫는다
        //   (`backend/claude/mod.rs` 의 result 라인이 Error 뒤에 MessageDone 을 반드시 붙인다). 두 백엔드의
        //   턴 분류자도 이 어휘엔 신호를 주지 않는다 — 여기서만 닫으면 두 축이 어긋난다.
        //   닫았을 때의 증상 = 재시도 중에 대기 표시가 꺼졌다가 다음 델타에 되살아나는 깜빡임.
        // ★알려진 구멍(프론트가 못 메운다)★: wire `StructuredEvent::Error` 에는 재시도 가능 여부 칸이
        //   없다. 그래서 「정말 턴을 끝낸 오류인데 경계가 안 오는」 경로에서는 대기 표시가 남는다.
        //   문자열을 뒤져 가르지 말 것 — 그 구별을 문자열에 적지 않는 것이 decoder 쪽 결정이다.
        this.items.push({ kind: 'error', message: ev.message, itemId: this.nextId++ })
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
            // ADR-0231: ★대기 중인 uuid 의 에코는 그리지도 「본 것」에 넣지도 않는다★ — 그 말풍선은 그 id 의
            //   `Delivered` 가 받음 자리에 그린다. 실측이 두 순서를 다 보였다: 에코가 `Delivered` 바로 앞에 오면
            //   여기서, 뒤에 오면 아래 uuid dedup 이 받는다. 그래서 「첫 항목이 이긴다」는 대기 중이 아닌
            //   uuid 에만 선다.
            if (this.queued.row(uuid) !== undefined) break
            if (this.seenUserUuids.has(uuid)) break
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
        // 탈출구 이벤트 — 알 수 없는 종류(kind)도 흘려 유실 방지.
        this.items.push({ kind: 'structured', label: ev.kind, json: ev.json, itemId: this.nextId++ })
        break
      }
      case 'MessageDone':
        // ADR-0045: claude decoder 는 결과 한 줄·한 턴마다 MessageDone 을 정확히 1회 발행하고, 그
        //   `turn_id` 는 지금도 항상 None 이다(`backend/claude/mod.rs` 의 발행 지점 둘 다 literal None).
        //   ★그래서 여기서 turn_id 로 경계를 유도하지 말 것★ — always-None 이라 경계가 사라진다.
        // ★단 「유일한 턴 경계 트리거」는 아니다★ — codex decoder 는 MessageDone 을 아예 내지 않고 아래
        //   `TurnEnd` 로 턴을 닫는다(그쪽 turn_id 는 채워져 온다). 두 어휘가 공존하며 렌더 경로는 하나다.
        this.closeTurn()
        break
      case 'TurnEnd':
        // ★결말 넷이 **전부** 턴을 닫는다★ — 결말 칸이 가르는 것은 화면 표시이지 「닫을지 말지」가 아니다.
        //   하나라도 안 닫으면 그 대화의 대기 인디케이터가 영영 돈다.
        {
          const mark = outcomeMark(ev.outcome)
          if (mark !== null) this.items.push({ kind: 'outcome', ...mark, itemId: this.nextId++ })
        }
        this.closeTurn()
        break
      case 'QueuedInput':
        return this.consumeQueued(ev.op)
      default: {
        // ★모르는 이벤트를 조용히 삼키지 않는다★: 아무것도 안 하면 화면은 한 픽셀도 안 바뀌는데 호출자는
        //   프레임이 온 것으로 행동한다. 릴리스 WebView2 에는 devtools 가 없어 console 이 사용자에게 도달
        //   하지 않으므로(ConnectionNotice 가 존재하는 사유와 같다) 화면에 남는 표식을 만든다.
        // ★원본 payload·이벤트 이름을 싣지 않는다★ — 프로토콜 낱말을 사용자 화면에 올리지 않는다는 결정
        //   (trd-phase2a §6-2). 진단용 이름은 console 로만 나간다.
        // ★이 item 이 대기 표시를 떠받친다고 기대하지 말 것(옛 오해 — 되살리지 말 것)★: 소비자의 파생은
        //   `!turnDone && items.length>0` 이라, 직전 턴이 이미 닫힌 뒤(= 둘째 턴부터)에는 item 을 더해도
        //   turnDone 이 true 로 남아 대기 표시가 그대로 꺼진다. 대기 표시를 지키는 것은 **아래 false 반환**
        //   이고(호출자가 자기 대기 상태를 안 푼다), 이 item 은 「무슨 일이 있었나」를 남기는 쪽만 맡는다.
        // ★그래도 `turnDone` 을 내리지는 않는다★ — 모르는 이벤트가 턴을 끝냈는지 우리는 모르고, 여기서
        //   false 로 내리면 **복원된 이력의 마지막 프레임이 모르는 종류일 때** 대기 표시가 영영 도는 쪽으로
        //   바뀐다(더 새 데몬이 턴 끝에 새 어휘를 붙이면 복원되는 모든 세션이 그 모양이 된다). 라이브 갭은
        //   다음 프레임이 스스로 고치지만 이력의 끝은 고칠 다음 프레임이 없다 — 그래서 축을 갈랐다.
        // ★알려진 구멍★: 이력의 마지막 프레임이 모르는 종류이면서 그 앞이 미완결 턴이면 대기 표시가 남는다.
        //   프론트 정보만으로는 못 가른다(턴 상태를 말해 주는 신호가 wire 에 따로 없다).
        // ★`never` 대입은 컴파일 시점 방어다★ — **이 셸의 bindings** 에 변형이 늘면 여기서 타입 에러가
        //   난다(그때 할 일은 이 arm 에 기대는 것이 아니라 제대로 된 arm 을 더하는 것). 런타임 방어는 그
        //   아래 — 더 새 데몬이 보내는, 이 셸의 bindings 에 **아예 없는** 종류다. 둘이 겨냥하는 창이 다르다.
        const exhaustive: never = ev
        console.warn(
          '[structuredAccumulator] 모르는 StructuredEvent type — 표식만 남기고 버린다:',
          (exhaustive as { type?: unknown }).type,
        )
        const last = this.items[this.items.length - 1]
        if (last && last.kind === 'unsupported') {
          // copy-on-write — 이전에 반환된 snapshot() 참조가 이 객체를 가리킨다(TextDelta arm 과 같은 규율).
          this.items[this.items.length - 1] = { ...last, count: last.count + 1 }
        } else {
          this.items.push({ kind: 'unsupported', count: 1, itemId: this.nextId++ })
        }
        return false
      }
    }
    return true
  }

  /**
   * 턴 경계 닫기 — 구분선 1개 + `turnDone`. 연속 종료는 구분선을 겹쳐 쌓지 않고, 선행 item 이 없으면
   * 구분선을 만들지 않는다(맨 앞 빈 경계 방지).
   */
  private closeTurn(): void {
    if (this.items.length > 0 && this.items[this.items.length - 1].kind !== 'separator') {
      this.items.push({ kind: 'separator', itemId: this.nextId++ })
    }
    this.turnDone = true
  }

  /**
   * 대기 입력 명부 사건 — 환원 뒤 그리기 규칙 셋만 대화 줄을 바꾼다(`Delivered` 배치 · 판명 사본 배치 ·
   * `Rejected` 말풍선 지우기). ★`turnDone` 을 건드리지 않는다★ — 목록 사건은 출력이 아니다(말풍선을 그린
   * 때만 사용자 arm 처럼 내린다). 종결은 목록에서 빠질 뿐 알림 행을 그리지 않는다.
   */
  // ADR-0231
  private consumeQueued(op: QueuedInputEvent | null | undefined): boolean {
    // `feed` 의 try/catch 는 JSON.parse 만 감싼다 — 모양이 깨진 프레임에서 던지지 않는다(outcomeMark 와 같은 규율).
    if (op === null || typeof op !== 'object') return false
    // 배치 본문은 앞선 `Queued` 의 사본이다 — 환원이 그 항목을 지우기 전에 잡는다.
    const pending = op.kind === 'Delivered' ? this.queued.row(op.id) : undefined
    if (op.kind === 'AckUnavailable' && !isCopyList(op.delivered)) return false
    if (this.queued.reduce(op) === null) {
      console.warn(
        '[structuredAccumulator] 모르는 QueuedInput kind — 버린다:',
        (op as { kind?: unknown }).kind,
      )
      return false
    }
    switch (op.kind) {
      case 'Delivered':
        // 대기 중이 아니던 id(되살림 · 모르는 id · 둘째 `Delivered`)는 그리지 않는다 — 되살림의 말풍선은
        //   그 id 가 이제 대기 중이 아니라 억제되지 않는 벤더 에코가 그린다.
        if (pending !== undefined) this.placeUserBubble(pending.id, pending.text)
        break
      case 'AckUnavailable':
        // 사본마다(목록 순) 그 자리에 — 이 창이 그 `Queued` 를 링에서 잃었어도 사본 본문으로 그린다.
        for (const copy of op.delivered) this.placeUserBubble(copy.id, copy.text)
        break
      case 'Dropped':
        // 환원 상태와 무관한 그리기 규칙 — 거절만 지운다(끊기·종료·실패 턴의 말풍선은 남는다).
        if (op.cause === 'Rejected') this.removeUserBubbles(op.id)
        break
      default:
        break
    }
    return true
  }

  /** 받음 자리에 사용자 말풍선 — 에코·합성 에코와 같은 모양이라 렌더러가 가르지 않는다. 한 uuid 에 한 번. */
  private placeUserBubble(id: string, text: string): void {
    if (this.seenUserUuids.has(id)) return
    this.seenUserUuids.add(id)
    this.items.push({
      kind: 'structured',
      label: 'user',
      json: JSON.stringify({ type: 'text', text, uuid: id }),
      itemId: this.nextId++,
    })
    this.turnDone = false
  }

  /**
   * 그 uuid 로 그린 사용자 말풍선(text 블록)을 걷는다. ★uuid 는 「본 것」에 남긴다★ — 거절은 되살림 불가라
   * 뒤늦은 같은 uuid 를 다시 그리지 않는다. 같은 줄의 비-text 블록(tool_result)은 dedup 대상이 아니듯 여기서도
   * 남는다.
   */
  private removeUserBubbles(id: string): void {
    for (let i = this.items.length - 1; i >= 0; i--) {
      const item = this.items[i]
      if (item.kind === 'structured' && item.label === 'user' && extractUserUuid(item.json) === id) {
        this.items.splice(i, 1)
      }
    }
  }

  /**
   * `QueuedInputList` 가 그릴 항목 — 대기(`queued`)이고 아직 말풍선으로 그리지 않은 uuid 만, 든 순서(가장
   * 오래된 것이 앞). 취소 대기는 모든 창에서 안 그린다. 매번 새 배열이다.
   * ★거름은 그리기에만 선다★ — 명부와 같은 환원 상태는 `queuedRows()`.
   */
  // ADR-0231
  snapshotQueued(): QueuedEntry[] {
    return this.queued
      .rows()
      .filter((entry) => entry.phase.state === 'queued' && !this.seenUserUuids.has(entry.id))
  }

  /** 환원 상태 그대로(취소 대기 · 이미 그린 uuid 포함) — agent 명부·골든과 같은 값이다. */
  queuedRows(): readonly QueuedEntry[] {
    return this.queued.rows()
  }

  /** 내부 배열 참조를 그대로 돌려준다 — React 소비자는 [...snapshot()] 로 새 참조를 떠서 set. */
  snapshot(): StructuredItem[] {
    return this.items
  }

  /**
   * 마지막 신호가 턴 종료(MessageDone · TurnEnd)였는가 — streaming/idle 표시 힌트(옵션).
   * ★`Error` 는 이 목록에 없다★ — 양쪽 백엔드에서 턴 경계가 아니다(그 arm 주석).
   */
  isTurnDone(): boolean {
    return this.turnDone
  }

  /** 재구독(replay) 전 초기화 — 히스토리 전체가 다시 흘러 동일 상태로 재구성되게 한다(위 idempotent 불변식). */
  reset(): void {
    this.items = []
    this.turnDone = false
    this.nextId = 0
    this.seenUserUuids.clear()
    this.queued.clear()
  }
}

/**
 * 결말 → 화면 표식. ★정상 완료는 표식을 남기지 않는다(null)★ — 구분선만으로 끝맺어 평범한 완료가
 * 경고처럼 보이지 않게 한다.
 * ★`default` 가 받는 것이 둘이다★ — ① 더 새 데몬이 더한 다섯째 결말 ② 결말 칸 자체가 없거나 모양이
 * 깨진 프레임. 둘 다 뜻을 모르므로 「모름」으로 적는다 — 아는 셋 중 하나로 접으면 화면이 거짓 결말을
 * 그린다(wire `TurnOutcome` 주석과 같은 규율).
 * ★타입이 `TurnOutcome` 인데도 nullish 를 받는 것은 의도★ — `feed` 의 try/catch 는 JSON.parse 만 감싸므로
 * 여기서 던지면 예외가 구독 콜백(RichSlot)까지 그대로 올라간다. 모양 가정을 하지 않는다.
 */
function outcomeMark(
  outcome: TurnOutcome | null | undefined,
): { outcome: TurnOutcomeMark; detail: string | null } | null {
  switch (outcome?.kind) {
    case 'Completed':
      return null
    case 'Failed':
      // `detail` 이 아예 없는 프레임도 표식은 남긴다 — 사유를 모를 뿐 실패는 실패다.
      return { outcome: 'failed', detail: outcome.detail ?? null }
    case 'Interrupted':
      return { outcome: 'interrupted', detail: null }
    case 'Unknown':
      return { outcome: 'unknown', detail: null }
    default:
      return { outcome: 'unknown', detail: null }
  }
}

function isCopyList(value: unknown): boolean {
  return (
    Array.isArray(value) &&
    value.every((copy) => copy !== null && typeof copy === 'object' && typeof copy.id === 'string')
  )
}

/**
 * user Structured item 의 json 에서 dedup 키 `uuid` 를 뽑는다(합성 에코 · replay 공통 부착).
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
      if (obj['type'] !== 'text') return null
      const uuid = obj['uuid']
      if (typeof uuid === 'string' && uuid.length > 0) return uuid
    }
    return null
  } catch {
    return null
  }
}
