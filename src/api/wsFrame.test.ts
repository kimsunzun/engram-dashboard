// 옛 daemonClient.test 의 decodeOutputFrame describe 를 이관(Stage 4a — daemonClient 삭제).
// wsTransport.test 의 "binary frame → output" 한 케이스로는 못 잡는 엣지(tag!=0/헤더미만/대문자
// 정규화/빈 payload)를 순수 함수 단위로 보존한다.

import { describe, expect, it } from 'vitest'

import placeholderSource from '../../crates/engram-dashboard-protocol/src/placeholder.rs?raw'
import {
  decodeOutputFrame,
  decodeReplayMarker,
  peekFrameHeader,
  PLACEHOLDER_ERROR_MESSAGE,
  placeholderErrorPayload,
} from './wsFrame'

// ── binary frame 빌더(codec.rs 와 동일 포맷: [tag:1][agentId:16][epoch:4 BE][seq:8 BE][payload]) ──
const FRAME_HEADER_LEN = 29
function uuidToBytes(uuid: string): Uint8Array {
  const hex = uuid.replace(/-/g, '')
  const out = new Uint8Array(16)
  for (let i = 0; i < 16; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16)
  return out
}
function buildFrame(opts: {
  tag?: number
  agentId: string
  epoch: number
  seq: number
  payload?: Uint8Array
  truncateTo?: number
}): ArrayBuffer {
  const payload = opts.payload ?? new Uint8Array(0)
  const buf = new ArrayBuffer(FRAME_HEADER_LEN + payload.length)
  const view = new DataView(buf)
  view.setUint8(0, opts.tag ?? 0)
  const idBytes = uuidToBytes(opts.agentId)
  for (let i = 0; i < 16; i++) view.setUint8(1 + i, idBytes[i])
  view.setUint32(17, opts.epoch, false) // BE
  view.setBigUint64(21, BigInt(opts.seq), false) // BE
  new Uint8Array(buf, FRAME_HEADER_LEN).set(payload)
  if (opts.truncateTo !== undefined) return buf.slice(0, opts.truncateTo)
  return buf
}

const AGENT = '12345678-9abc-def0-1234-56789abcdef0'

describe('decodeOutputFrame', () => {
  it('codec.rs 포맷대로 디코드: tag/epoch/seq/payload + agentId UUID 왕복', () => {
    const payload = new Uint8Array([0x68, 0x69]) // "hi"
    const buf = buildFrame({ agentId: AGENT, epoch: 7, seq: 42, payload })
    const f = decodeOutputFrame(buf)
    expect(f).not.toBeNull()
    expect(f!.tag).toBe(0)
    expect(f!.epoch).toBe(7)
    expect(f!.seq).toBe(42)
    expect(f!.agentId).toBe(AGENT)
    expect(Array.from(f!.payload)).toEqual([0x68, 0x69])
  })

  it('대문자 입력도 소문자 UUID 로 정규화한다(byte→hex 는 항상 소문자)', () => {
    const upper = 'ABCDEF01-2345-6789-ABCD-EF0123456789'
    const buf = buildFrame({ agentId: upper, epoch: 0, seq: 0 })
    const f = decodeOutputFrame(buf)
    expect(f!.agentId).toBe(upper.toLowerCase())
  })

  it('헤더 길이 미만이면 null', () => {
    const buf = buildFrame({ agentId: AGENT, epoch: 1, seq: 1, truncateTo: 28 })
    expect(decodeOutputFrame(buf)).toBeNull()
  })

  it('tag1(StructuredEvent) 프레임 디코드: tag=1 + payload(JSON 바이트) 그대로 추출', () => {
    // S15/ADR-0045: tag1 = 구조화 이벤트. codec 은 payload 를 opaque 로 넘긴다(JSON 해석은 소비자 몫).
    const json = '{"type":"TextDelta","text":"hi","turn_id":null,"message_id":null}'
    const payload = new TextEncoder().encode(json)
    const buf = buildFrame({ tag: 1, agentId: AGENT, epoch: 2, seq: 5, payload })
    const f = decodeOutputFrame(buf)
    expect(f).not.toBeNull()
    expect(f!.tag).toBe(1)
    expect(f!.epoch).toBe(2)
    expect(f!.seq).toBe(5)
    expect(f!.agentId).toBe(AGENT)
    expect(new TextDecoder().decode(f!.payload)).toBe(json)
  })

  it('tag >= 2(미지원 variant)면 null(tag0/tag1 만 유효)', () => {
    const buf = buildFrame({ tag: 2, agentId: AGENT, epoch: 1, seq: 1 })
    expect(decodeOutputFrame(buf)).toBeNull()
  })

  it('tag=255(ADR-0046 replay 경계 마커)는 조용히 skip(null) — 던지지 않음, 전방 호환(M0)', () => {
    // src-tauri 가 흘리는 마커 프레임 규격: [tag=255][agentId:16][epoch:4][gen:8 BE][flags:1][replay_from:8 BE]
    //   = 38바이트. decodeOutputFrame 은 마커를 소비하지 않으므로(M2) 미지 tag 를 예외 없이 null 로 버려야
    //   한다. 길이가 헤더(29) 이상이어도 tag 게이트에서 걸러진다(payload 로 오해 금지).
    const marker = new ArrayBuffer(1 + 16 + 4 + 8 + 1 + 8)
    const view = new DataView(marker)
    view.setUint8(0, 255)
    const idBytes = uuidToBytes(AGENT)
    for (let i = 0; i < 16; i++) view.setUint8(1 + i, idBytes[i])
    view.setUint32(17, 3, false) // epoch
    // gen · flags · replay_from 은 decodeOutputFrame 이 안 읽는다 — tag 게이트에서 이미 null.
    expect(() => decodeOutputFrame(marker)).not.toThrow()
    expect(decodeOutputFrame(marker)).toBeNull()
  })

  it('빈 payload(헤더만)도 디코드 성공(payload 길이 0)', () => {
    const buf = buildFrame({ agentId: AGENT, epoch: 3, seq: 9 })
    const f = decodeOutputFrame(buf)
    expect(f).not.toBeNull()
    expect(f!.payload.length).toBe(0)
  })
})

// ── ADR-0231: 자리채움 — 모르는 tag 프레임의 seq 를 비우지 않는다 ──────────────────────────
describe('peekFrameHeader · 자리채움', () => {
  it('머리는 tag 를 가리지 않고 읽는다 · 머리보다 짧으면 null', () => {
    const buf = buildFrame({ tag: 7, agentId: AGENT, epoch: 3, seq: 42, payload: new Uint8Array([1, 2]) })
    expect(peekFrameHeader(buf)).toEqual({ tag: 7, agentId: AGENT, epoch: 3, seq: 42 })
    expect(peekFrameHeader(buildFrame({ agentId: AGENT, epoch: 3, seq: 42, truncateTo: 28 }))).toBeNull()
  })

  it('문구가 프로토콜 상수와 바이트 같고 페이로드는 그 Error 사건의 JSON 이다', () => {
    const literal = /macro_rules! placeholder_message \{\s*\(\) => \{\s*"([^"]*)"/.exec(placeholderSource)
    expect(literal?.[1]).toBe(PLACEHOLDER_ERROR_MESSAGE)
    expect(placeholderSource).toContain('r#"{"type":"Error","message":""#')
    expect(new TextDecoder().decode(placeholderErrorPayload())).toBe(
      `{"type":"Error","message":"${PLACEHOLDER_ERROR_MESSAGE}"}`,
    )
  })
})

// ── replay 경계 마커(src-tauri replay_flight 가 합성하는 38바이트 — ADR-0046 · ADR-0226 bit2 · ADR-0231 머리) ──
const MARKER_LEN = 38
function buildMarker(opts: {
  epoch: number
  gen: bigint
  flags: number
  replayFrom?: bigint
  length?: number
}): ArrayBuffer {
  const buf = new ArrayBuffer(MARKER_LEN)
  const view = new DataView(buf)
  view.setUint8(0, 255)
  const idBytes = uuidToBytes(AGENT)
  for (let i = 0; i < 16; i++) view.setUint8(1 + i, idBytes[i])
  view.setUint32(17, opts.epoch, false)
  view.setBigUint64(21, opts.gen, false)
  view.setUint8(29, opts.flags)
  view.setBigUint64(30, opts.replayFrom ?? 0n, false)
  return opts.length === undefined ? buf : buf.slice(0, opts.length)
}

describe('decodeReplayMarker', () => {
  it('bit2(0x04) = 이어받기 화신 — 다른 두 비트와 독립으로 읽힌다', () => {
    const m = decodeReplayMarker(buildMarker({ epoch: 7, gen: 42n, flags: 0x04 }))
    expect(m).toEqual({
      agentId: AGENT,
      epoch: 7,
      gen: 42n,
      truncated: false,
      failed: false,
      continuesConversation: true,
      replayFrom: 0,
    })
    expect(decodeReplayMarker(buildMarker({ epoch: 7, gen: 42n, flags: 0x05 }))).toMatchObject({
      truncated: true,
      failed: false,
      continuesConversation: true,
    })
  })

  // 표식을 안 싣는 옛 셸·데몬 경로 = bit2 없음. 그때는 표식 도입 전 동작(첫 화면)이 나와야 한다.
  it.each([0x00, 0x01, 0x02, 0x03])('bit2 가 없으면(flags=%i) 이어받기 아님', (flags) => {
    expect(decodeReplayMarker(buildMarker({ epoch: 1, gen: 1n, flags }))?.continuesConversation).toBe(
      false,
    )
  })

  it('길이는 38 — 37바이트와 옛 30바이트 마커는 마커가 아니다', () => {
    expect(decodeReplayMarker(buildMarker({ epoch: 1, gen: 1n, flags: 0x04 }))).not.toBeNull()
    expect(decodeReplayMarker(buildMarker({ epoch: 1, gen: 1n, flags: 0x04, length: 37 }))).toBeNull()
    expect(decodeReplayMarker(buildMarker({ epoch: 1, gen: 1n, flags: 0x04, length: 30 }))).toBeNull()
  })

  // ADR-0231: flush 시작점 = max(마지막+1, replay 머리). 칸 자리(30..38 BE)가 어긋나면 머리가 엉뚱한 값이 돼
  //   뷰가 이력을 건너뛰거나 영영 오지 않을 seq 를 기다린다.
  it('replay 머리(30..38 BE)를 seq 와 같은 number 로 읽고 flags 와 섞이지 않는다', () => {
    expect(
      decodeReplayMarker(buildMarker({ epoch: 9, gen: 3n, flags: 0x07, replayFrom: 40n })),
    ).toMatchObject({ gen: 3n, truncated: true, failed: true, continuesConversation: true, replayFrom: 40 })
    // 상위 바이트까지 BE 로 — 2^32 를 넘는 seq 도 그대로.
    expect(
      decodeReplayMarker(buildMarker({ epoch: 9, gen: 3n, flags: 0, replayFrom: 0x1_0000_0002n }))
        ?.replayFrom,
    ).toBe(0x1_0000_0002)
  })
})
