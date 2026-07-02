// 라이브 누산기 단위테스트 — 바이트 청크(임의 분할) → RichMessage[] 병합 검증.
// 순수 로직이라 jsdom 불필요. 병합 시맨틱(같은 message.id disjoint 블록 concat)은 실측 fixture 로 핀.

import { describe, it, expect, vi } from 'vitest'
import { StreamAccumulator } from './streamParse'
import { parseStreamJson } from './parse'
import type { ContentBlock } from './types'
import toolFixture from './fixtures/tool.jsonl?raw'

const types = (blocks: ContentBlock[]): string[] => blocks.map((b) => b.type)

describe('StreamAccumulator', () => {
  // ── ★병합 시맨틱 핀(ADR-0044)★: 같은 assistant message.id 의 여러 라인은 disjoint 블록 배치라
  //    한 메시지로 concat 돼야 한다. tool.jsonl 실측: 9번줄=[thinking],10번줄=[tool_use] 같은 id →
  //    한 메시지 [thinking, tool_use]. 16/17번줄=[thinking],[text] 같은 id → [thinking, text]. ──
  it('실측 tool fixture: 같은 message.id assistant 라인들을 disjoint 블록으로 concat', () => {
    const acc = new StreamAccumulator()
    acc.feed(toolFixture)
    const msgs = acc.snapshot()

    // 병합 결과 = assistant(2턴) + 사이 user(tool_result) = 3 메시지.
    expect(msgs).toHaveLength(3)
    expect(msgs[0].role).toBe('assistant')
    expect(types(msgs[0].blocks)).toEqual(['thinking', 'tool_use'])
    expect(msgs[1].role).toBe('user')
    expect(types(msgs[1].blocks)).toEqual(['tool_result'])
    expect(msgs[2].role).toBe('assistant')
    expect(types(msgs[2].blocks)).toEqual(['thinking', 'text'])
  })

  it('병합이 blind append(parseStreamJson)와 다르다는 증거 — 같은 입력에 메시지 수가 준다', () => {
    // parseStreamJson 은 라인마다 새 메시지(id 병합 X) → assistant 4 + user 1 = 5.
    expect(parseStreamJson(toolFixture)).toHaveLength(5)
    // 누산기는 같은 id 를 한 메시지로 접어 3.
    const acc = new StreamAccumulator()
    acc.feed(toolFixture)
    expect(acc.snapshot()).toHaveLength(3)
  })

  // ── 청크 경계: NDJSON 라인을 아무 데서나 잘라도(멀티바이트 UTF-8 포함) 동일 결과 ──
  it('임의 바이트 청크(5바이트 단위, 멀티바이트 경계 포함)로 먹여도 통짜와 동일', () => {
    const whole = new StreamAccumulator()
    whole.feed(toolFixture)

    const bytes = new TextEncoder().encode(toolFixture)
    const chunked = new StreamAccumulator()
    for (let i = 0; i < bytes.length; i += 5) chunked.feed(bytes.subarray(i, i + 5))

    expect(chunked.snapshot()).toEqual(whole.snapshot())
  })

  it('라인을 1바이트씩 흘려도(최악 멀티바이트 분할) 텍스트가 온전히 복원된다', () => {
    const line =
      '{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"안녕하세요 😀 world"}]}}\n'
    const bytes = new TextEncoder().encode(line)
    const acc = new StreamAccumulator()
    for (let i = 0; i < bytes.length; i++) acc.feed(bytes.subarray(i, i + 1))
    const msgs = acc.snapshot()
    expect(msgs).toHaveLength(1)
    expect(msgs[0].blocks[0]).toEqual({ type: 'text', text: '안녕하세요 😀 world' })
  })

  it('개행 없는 미완 라인은 도착 전엔 메시지가 안 되고, 개행이 와야 확정된다(라인 재조립)', () => {
    const acc = new StreamAccumulator()
    acc.feed('{"type":"assistant","message":{"id":"m","content":[{"type":"text",')
    expect(acc.snapshot()).toHaveLength(0) // 아직 라인 미완
    acc.feed('"text":"hi"}]}}\n')
    expect(acc.snapshot()).toHaveLength(1)
    expect(acc.snapshot()[0].blocks[0]).toEqual({ type: 'text', text: 'hi' })
  })

  // ── replay: 재구독 시 히스토리 전체가 다시 흐른다 → reset 후 재파싱이 동일 상태로 재구성 ──
  it('reset 후 같은 히스토리를 다른 청크로 재생하면 동일 스냅샷(replay from zero)', () => {
    const acc = new StreamAccumulator()
    acc.feed(toolFixture)
    const first = acc.snapshot()

    acc.reset()
    const bytes = new TextEncoder().encode(toolFixture)
    for (let i = 0; i < bytes.length; i += 13) acc.feed(bytes.subarray(i, i + 13))
    expect(acc.snapshot()).toEqual(first)
  })

  // ── user echo(--replay-user-messages): 유저 턴이 출력 스트림에 되울림 → user 메시지로 렌더 ──
  it('user 텍스트 라인(유저 턴 되울림)을 user role 메시지로 수집', () => {
    const acc = new StreamAccumulator()
    acc.feed('{"type":"user","message":{"role":"user","content":[{"type":"text","text":"do it"}]}}\n')
    const msgs = acc.snapshot()
    expect(msgs).toHaveLength(1)
    expect(msgs[0].role).toBe('user')
    expect(msgs[0].blocks[0]).toEqual({ type: 'text', text: 'do it' })
  })

  it('비-JSON/메타(system·rate_limit) 라인은 조용히 스킵', () => {
    const acc = new StreamAccumulator()
    acc.feed('Warning: no stdin data received\n')
    acc.feed('{"type":"system","subtype":"init"}\n')
    acc.feed('{"type":"rate_limit_event"}\n')
    acc.feed('{"type":"assistant","message":{"id":"m","content":[{"type":"text","text":"hi"}]}}\n')
    expect(acc.snapshot()).toHaveLength(1)
  })

  // ── result 라인 = 턴 종료 신호(렌더 대상 아님, 입력 UX 힌트) ──
  it('result 라인은 메시지가 아니라 turnDone 플래그를 세운다', () => {
    const acc = new StreamAccumulator()
    expect(acc.isTurnDone()).toBe(false)
    acc.feed('{"type":"assistant","message":{"id":"m","content":[{"type":"text","text":"hi"}]}}\n')
    expect(acc.isTurnDone()).toBe(false) // 어시 응답 중
    acc.feed('{"type":"result","subtype":"success"}\n')
    expect(acc.isTurnDone()).toBe(true) // 턴 종료
    expect(acc.snapshot()).toHaveLength(1) // result 는 메시지로 안 잡힘
  })

  it('두 개의 서로 다른 message.id assistant 는 별도 메시지로 남는다(턴 분리)', () => {
    const acc = new StreamAccumulator()
    acc.feed('{"type":"assistant","message":{"id":"a","content":[{"type":"text","text":"first"}]}}\n')
    acc.feed('{"type":"assistant","message":{"id":"b","content":[{"type":"text","text":"second"}]}}\n')
    const msgs = acc.snapshot()
    expect(msgs).toHaveLength(2)
    expect(msgs[0].id).toBe('a')
    expect(msgs[1].id).toBe('b')
  })

  // ── CRLF 라인 종결(FIX 6): \r\n 스트림에서 뒤따르는 \r 이 JSON.parse 를 깨면 안 된다(trim 처리) ──
  it('CRLF(\\r\\n) 종결 라인 — 남는 \\r 이 JSON.parse 를 깨지 않는다', () => {
    const acc = new StreamAccumulator()
    acc.feed('{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"hi"}]}}\r\n')
    acc.feed('{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"bye"}]}}\r\n')
    const msgs = acc.snapshot()
    expect(msgs).toHaveLength(2)
    expect(msgs[0].blocks[0]).toEqual({ type: 'text', text: 'hi' })
    expect(msgs[1].blocks[0]).toEqual({ type: 'text', text: 'bye' })
  })

  it('CRLF 가 청크 경계에서 갈라져도(\\r 끝 / \\n 시작) 온전히 처리', () => {
    const acc = new StreamAccumulator()
    acc.feed('{"type":"assistant","message":{"id":"m","content":[{"type":"text","text":"x"}]}}\r')
    expect(acc.snapshot()).toHaveLength(0) // \n 아직 안 옴 → 라인 미확정
    acc.feed('\n')
    expect(acc.snapshot()).toHaveLength(1)
    expect(acc.snapshot()[0].blocks[0]).toEqual({ type: 'text', text: 'x' })
  })

  // ── reset(FIX 6): 미완 멀티바이트 partial 을 남긴 뒤 reset → 재사용 시 잔여가 새 입력을 오염 안 함 ──
  it('reset() 은 미완 멀티바이트 partial 을 버리고 재사용 시 모지바케 없이 새 라인을 디코드', () => {
    const acc = new StreamAccumulator()
    // '안' = UTF-8 3바이트. 첫 바이트만 흘려 디코더에 미완 상태를 남긴다(아직 라인/문자 미완).
    const partial = new TextEncoder().encode('안')
    acc.feed(partial.subarray(0, 1))
    expect(acc.snapshot()).toHaveLength(0)
    acc.reset() // 새 TextDecoder + 빈 buffer → 이전 미완 바이트 폐기(재사용 시 오염 방지)
    acc.feed('{"type":"assistant","message":{"id":"m","content":[{"type":"text","text":"재사용 😀"}]}}\n')
    const msgs = acc.snapshot()
    expect(msgs).toHaveLength(1)
    expect(msgs[0].blocks[0]).toEqual({ type: 'text', text: '재사용 😀' })
  })

  // ── FIX 3: 같은 id 재방출/중복 라인은 블록을 중복 append 하지 않는다(멱등) ──
  it('같은 message.id assistant 라인을 두 번 먹여도 블록이 중복되지 않는다(멱등 재방출 방어)', () => {
    const acc = new StreamAccumulator()
    const line = '{"type":"assistant","message":{"id":"dup","content":[{"type":"text","text":"once"}]}}\n'
    acc.feed(line)
    acc.feed(line) // 정확히 같은 라인 재방출
    const msgs = acc.snapshot()
    expect(msgs).toHaveLength(1)
    expect(msgs[0].blocks).toHaveLength(1) // 중복 append 안 됨
    expect(msgs[0].blocks[0]).toEqual({ type: 'text', text: 'once' })
  })

  it('같은 id 의 새 블록은 여전히 concat 되고 중복 블록만 걸러진다(dedup 이 disjoint concat 을 막지 않음)', () => {
    const acc = new StreamAccumulator()
    acc.feed('{"type":"assistant","message":{"id":"m","content":[{"type":"thinking","thinking":"t"}]}}\n')
    // 같은 id — 앞 블록(thinking) 재출현 + 새 블록(text). thinking 은 skip, text 만 이어붙는다.
    acc.feed(
      '{"type":"assistant","message":{"id":"m","content":[{"type":"thinking","thinking":"t"},{"type":"text","text":"hi"}]}}\n',
    )
    const msgs = acc.snapshot()
    expect(msgs).toHaveLength(1)
    expect(types(msgs[0].blocks)).toEqual(['thinking', 'text'])
  })

  // ── FIX 4: 개행 없는 초대형 입력은 버퍼 상한 초과 시 드롭 → OOM 없이 다음 정상 라인에서 복구 ──
  it('개행 없는 초대형 입력은 버퍼 상한 초과 시 버려지고 다음 정상 라인에서 복구된다', () => {
    const acc = new StreamAccumulator()
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    // 상한(4MB) 초과 no-newline 입력 → OOM 없이 버퍼 드롭.
    acc.feed('x'.repeat(4 * 1024 * 1024 + 10))
    expect(acc.snapshot()).toHaveLength(0)
    expect(warn).toHaveBeenCalled() // overflow 경고 발화
    // 드롭 후 정상 라인이 오면 복구(부분 라인 1개 손실은 감수).
    acc.feed('{"type":"assistant","message":{"id":"m","content":[{"type":"text","text":"recovered"}]}}\n')
    expect(acc.snapshot()).toHaveLength(1)
    expect(acc.snapshot()[0].blocks[0]).toEqual({ type: 'text', text: 'recovered' })
    warn.mockRestore()
  })
})
