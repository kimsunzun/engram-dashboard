// RichSlot 라이브 누산기 — stream-json 바이트 청크(라인 임의 분할) → RichMessage[] (순수 TS, React 무관).
//
// TerminalSlot 이 PTY 바이트를 xterm 에 흘리듯, RichSlot 라이브 모드는 StdioTransport 의 NDJSON
// 바이트를 이 누산기에 흘려 RichMessage[] 로 누적한다. 데몬 통로는 바보 파이프(ADR-0044 통로 무정제
// 불변)라 청크가 NDJSON 라인 경계를 아무 데서나 자른다 — 여기서 재조립·파싱·병합을 소유한다.
//
// ★두 층 분리★: 라인 1개 해석은 parse.ts(parseStreamLine)와 공유하고, 이 파일은 "바이트 → 라인
//   재조립"과 "라인 → RichMessage[] 병합"만 책임진다.
//
// ★병합 시맨틱(ADR-0044, 실측 fixture 로 확정)★: `fixtures/tool.jsonl` 관측 — 같은 assistant 턴이
//   여러 라인으로 나뉘어 오는데, 각 라인은 **같은 message.id** 를 달고 **서로 다른(disjoint) 블록**을
//   싣는다(예: 9번줄=[thinking], 10번줄=[tool_use], 둘 다 msg_01DXX...). 즉 누적 스냅샷(cumulative)이
//   아니라 disjoint 배치다 → 같은 id 재출현 시 블록을 **이어붙인다(concat)**. (누적 스냅샷이었다면
//   replace/dedup 이 맞겠지만 fixture 는 명확히 disjoint 라 concat 이 옳다. 만약 claude 가 이 모드에서
//   누적 재발행으로 바뀌면 여기 병합을 블록-레벨 dedup 으로 바꿔야 한다 — 그땐 fixture 갱신 + 테스트로 잡힌다.)
//   ※concat 은 블록-레벨 멱등 가드를 낀다(FIX 3): 실측은 disjoint 지만 재방출/중복 라인이 와도 이미 있는
//     블록과 구조적으로 동일하면 skip 해 중복 append 를 막는다(방어적 견고성).

import { parseStreamLine } from './parse'
import type { RichMessage } from './types'

// ★미종결 라인 버퍼 상한(FIX 4)★: 개행이 영영 오지 않는 malformed/거대 출력이면 buffer 가 무한 증가해
//   OOM 을 낸다. 통로는 바보 파이프(ADR-0044 무정제 불변)라 상류가 라인을 보장하지 않으므로 소비자(누산기)가
//   방어한다 — 4MB(문자) 넘으면 부분 라인을 버리고 다음 개행부터 복구한다. NDJSON 한 라인이 4MB 를 넘는
//   정상 케이스는 없다(thinking/text 블록도 그보다 훨씬 작다) → 상한 초과 = 비정상으로 간주.
const MAX_BUFFER_CHARS = 4 * 1024 * 1024

/**
 * 라이브 stream-json 누산기. `feed` 로 바이트/문자 청크를 밀어 넣으면 완성된 라인마다 파싱해
 * `snapshot()` 이 반환하는 RichMessage[] 를 갱신한다. 재구독(replay) 전엔 `reset()` 으로 초기화한다
 * (terminal.reset() 규율의 RichSlot 판 — 히스토리 전체가 다시 흘러 동일 상태로 재구성).
 */
export class StreamAccumulator {
  // stream:true — 멀티바이트 UTF-8 문자가 청크 경계에서 잘려도 미완 바이트를 디코더가 내부 버퍼링해
  // 다음 청크와 합쳐 온전히 디코드한다(모지바케 방지). 그래서 Uint8Array 는 반드시 이 인스턴스로만 디코드.
  private decoder = new TextDecoder('utf-8')
  // 마지막 개행 뒤 미완성 라인(라인-레벨 분할 재조립).
  private buffer = ''
  private messages: RichMessage[] = []
  // assistant message.id → messages[] 인덱스. 같은 id 재출현 시 disjoint 블록을 이어붙일 대상 위치.
  private assistantIndexById = new Map<string, number>()
  // 마지막으로 result 라인을 봤는가(턴 종료). 라이브 입력 UX 의 streaming/idle 힌트(옵션).
  private turnDone = false
  // 버퍼 상한 초과로 미종결 라인을 버린 횟수(FIX 4 — 진단용 카운터). overflow 마다 1회 경고.
  private overflowCount = 0

  /**
   * 바이트(Uint8Array) 또는 이미 디코드된 문자열 청크를 밀어 넣는다.
   * 라이브 경로는 항상 Uint8Array(OutputChunk.bytes) — 문자열 입력은 테스트/편의용.
   * ※한 누산기에서 Uint8Array 와 string 을 섞어 먹이지 말 것(디코더 stream 상태와 어긋날 수 있음).
   */
  feed(chunk: Uint8Array | string): void {
    const text = typeof chunk === 'string' ? chunk : this.decoder.decode(chunk, { stream: true })
    if (!text) return
    this.buffer += text
    let nl: number
    while ((nl = this.buffer.indexOf('\n')) !== -1) {
      const line = this.buffer.slice(0, nl)
      this.buffer = this.buffer.slice(nl + 1)
      this.consumeLine(line)
    }
    // 완성 라인을 모두 소비한 뒤 남은 미종결 tail 이 상한을 넘으면 버린다(무한 성장·OOM 방지, FIX 4).
    // 다음 feed 에 정상 개행이 오면 빈 버퍼 + 새 텍스트로 복구된다(부분 라인 1개 손실은 감수).
    if (this.buffer.length > MAX_BUFFER_CHARS) {
      this.overflowCount++
      console.warn(
        `[StreamAccumulator] partial-line buffer overflow (#${this.overflowCount}) — ` +
          `dropping ${this.buffer.length} chars (no line terminator)`,
      )
      this.buffer = ''
    }
  }

  private consumeLine(line: string): void {
    const parsed = parseStreamLine(line)
    if (!parsed) return // 메타·비-JSON·빈 줄 — 조용히 스킵
    if (parsed.kind === 'result') {
      this.turnDone = true // 턴 종료 신호(렌더 대상 아님)
      return
    }
    // assistant 는 message.id 로 병합(disjoint 블록 배치 이어붙임). user(및 id 없는 라인)는 항상 새 메시지.
    if (parsed.role === 'assistant' && parsed.id !== undefined) {
      const idx = this.assistantIndexById.get(parsed.id)
      if (idx !== undefined) {
        // 같은 턴의 다음 블록 배치 — 기존 메시지에 블록 concat(불변 갱신: 새 객체/새 배열).
        // ★멱등 가드(FIX 3)★: 실측은 disjoint 배치지만, 같은 id 라인이 재방출/중복돼도 이미 있는 블록과
        //   구조적으로 동일한 것은 skip 한다(중복 append 방어). 지문 = 블록 JSON.stringify(메시지당 블록
        //   수는 유한 → 비용 bounded). 새 블록이 하나도 없으면 배열을 그대로 두어 불필요한 리렌더도 아낀다.
        const prev = this.messages[idx]
        const seen = new Set(prev.blocks.map((b) => JSON.stringify(b)))
        const fresh = parsed.blocks.filter((b) => !seen.has(JSON.stringify(b)))
        if (fresh.length > 0) {
          this.messages[idx] = { ...prev, blocks: [...prev.blocks, ...fresh] }
        }
        this.turnDone = false
        return
      }
      this.assistantIndexById.set(parsed.id, this.messages.length)
    }
    // 새 메시지 등장 = 에이전트가 작업 중(유저 턴 되울림 포함) → idle 해제.
    this.turnDone = false
    this.messages.push({ role: parsed.role, blocks: parsed.blocks, id: parsed.id })
  }

  /** 현재까지 누적된 메시지 목록(내부 배열 참조). React 소비자는 [...snapshot()] 로 새 참조를 떠서 set 한다. */
  snapshot(): RichMessage[] {
    return this.messages
  }

  /** 마지막 신호가 result(턴 종료)였는가 — streaming/idle 표시 힌트(옵션). */
  isTurnDone(): boolean {
    return this.turnDone
  }

  /** 재구독(replay) 전 초기화 — 히스토리 전체가 다시 흘러 동일 상태로 재구성되게 한다. */
  reset(): void {
    this.decoder = new TextDecoder('utf-8')
    this.buffer = ''
    this.messages = []
    this.assistantIndexById = new Map()
    this.turnDone = false
    this.overflowCount = 0
  }
}
