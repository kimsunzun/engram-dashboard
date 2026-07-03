// wsFrame 단위테스트 — decodeOutputFrame 순수 함수(codec.rs binary frame 디코드).
//
// 옛 daemonClient.test 의 decodeOutputFrame describe 를 이관(Stage 4a — daemonClient 삭제).
// wsTransport.test 의 "binary frame → output" 한 케이스로는 못 잡는 엣지(tag!=0/헤더미만/대문자
// 정규화/빈 payload)를 순수 함수 단위로 보존한다.

import { describe, expect, it } from 'vitest'

import { decodeOutputFrame } from './wsFrame'

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
  /** 헤더 미만 길이 테스트용 — 헤더 일부만 생성. */
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
    // 16바이트 → 8-4-4-4-12 소문자 UUID 정확 복원(알려진 uuid ↔ 바이트 왕복).
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
    // payload 왕복 — 소비자가 JSON.parse 로 StructuredEvent 를 복원한다.
    expect(new TextDecoder().decode(f!.payload)).toBe(json)
  })

  it('tag >= 2(미지원 variant)면 null(tag0/tag1 만 유효)', () => {
    const buf = buildFrame({ tag: 2, agentId: AGENT, epoch: 1, seq: 1 })
    expect(decodeOutputFrame(buf)).toBeNull()
  })

  it('빈 payload(헤더만)도 디코드 성공(payload 길이 0)', () => {
    const buf = buildFrame({ agentId: AGENT, epoch: 3, seq: 9 })
    const f = decodeOutputFrame(buf)
    expect(f).not.toBeNull()
    expect(f!.payload.length).toBe(0)
  })
})
