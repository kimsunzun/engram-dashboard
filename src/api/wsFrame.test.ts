// Migrated the decodeOutputFrame describe of the old daemonClient.test (Stage 4a — daemonClient deleted).
// Preserves, as pure-function units, the edges that the one case "binary frame → output" in wsTransport.test cannot catch (tag!=0 / shorter than the header / uppercase
// normalization / empty payload).

import { describe, expect, it } from 'vitest'

import placeholderSource from '../../crates/engram-dashboard-protocol/src/placeholder.rs?raw'
import {
  decodeOutputFrame,
  decodeReplayMarker,
  peekFrameHeader,
  PLACEHOLDER_ERROR_MESSAGE,
  placeholderErrorPayload,
} from './wsFrame'

// ── binary frame builder (same format as codec.rs: [tag:1][agentId:16][epoch:4 BE][seq:8 BE][payload]) ──
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
  it('decodes per the codec.rs format: tag/epoch/seq/payload + agentId UUID round trip', () => {
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

  it('normalizes uppercase input to a lowercase UUID too (byte→hex is always lowercase)', () => {
    const upper = 'ABCDEF01-2345-6789-ABCD-EF0123456789'
    const buf = buildFrame({ agentId: upper, epoch: 0, seq: 0 })
    const f = decodeOutputFrame(buf)
    expect(f!.agentId).toBe(upper.toLowerCase())
  })

  it('null when shorter than the header length', () => {
    const buf = buildFrame({ agentId: AGENT, epoch: 1, seq: 1, truncateTo: 28 })
    expect(decodeOutputFrame(buf)).toBeNull()
  })

  it('decodes a tag1 (StructuredEvent) frame: tag=1 + extracts the payload (JSON bytes) as is', () => {
    // S15/ADR-0045: tag1 = structured event. The codec passes the payload through as opaque (JSON interpretation is the consumer's job).
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

  it('null when tag >= 2 (unsupported variant) (only tag0/tag1 are valid)', () => {
    const buf = buildFrame({ tag: 2, agentId: AGENT, epoch: 1, seq: 1 })
    expect(decodeOutputFrame(buf)).toBeNull()
  })

  it('tag=255 (ADR-0046 replay boundary marker) is silently skipped (null) — does not throw, forward compatible (M0)', () => {
    // Marker frame spec src-tauri sends: [tag=255][agentId:16][epoch:4][gen:8 BE][flags:1][replay_from:8 BE]
    //   = 38 bytes. decodeOutputFrame does not consume markers (M2), so it must drop an unknown tag as null without an exception.
    //   Even if the length is at least the header (29), it is filtered at the tag gate (do not mistake it for a payload).
    const marker = new ArrayBuffer(1 + 16 + 4 + 8 + 1 + 8)
    const view = new DataView(marker)
    view.setUint8(0, 255)
    const idBytes = uuidToBytes(AGENT)
    for (let i = 0; i < 16; i++) view.setUint8(1 + i, idBytes[i])
    view.setUint32(17, 3, false) // epoch
    // decodeOutputFrame does not read gen · flags · replay_from — already null at the tag gate.
    expect(() => decodeOutputFrame(marker)).not.toThrow()
    expect(decodeOutputFrame(marker)).toBeNull()
  })

  it('an empty payload (header only) also decodes successfully (payload length 0)', () => {
    const buf = buildFrame({ agentId: AGENT, epoch: 3, seq: 9 })
    const f = decodeOutputFrame(buf)
    expect(f).not.toBeNull()
    expect(f!.payload.length).toBe(0)
  })
})

// ── ADR-0231: placeholder — do not leave the seq of an unknown-tag frame empty ──────────────────────────
describe('peekFrameHeader · placeholder', () => {
  it('reads the header regardless of the tag · null when shorter than the header', () => {
    const buf = buildFrame({ tag: 7, agentId: AGENT, epoch: 3, seq: 42, payload: new Uint8Array([1, 2]) })
    expect(peekFrameHeader(buf)).toEqual({ tag: 7, agentId: AGENT, epoch: 3, seq: 42 })
    expect(peekFrameHeader(buildFrame({ agentId: AGENT, epoch: 3, seq: 42, truncateTo: 28 }))).toBeNull()
  })

  it('the wording is byte-equal to the protocol constant and the payload is the JSON of that Error event', () => {
    const literal = /macro_rules! placeholder_message \{\s*\(\) => \{\s*"([^"]*)"/.exec(placeholderSource)
    expect(literal?.[1]).toBe(PLACEHOLDER_ERROR_MESSAGE)
    expect(placeholderSource).toContain('r#"{"type":"Error","message":""#')
    expect(new TextDecoder().decode(placeholderErrorPayload())).toBe(
      `{"type":"Error","message":"${PLACEHOLDER_ERROR_MESSAGE}"}`,
    )
  })
})

// ── replay boundary marker (the 38 bytes src-tauri replay_flight synthesizes — ADR-0046 · ADR-0226 bit2 · ADR-0231 head) ──
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
  it('bit2 (0x04) = resume incarnation — read independently of the other two bits', () => {
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

  // An old shell/daemon path that carries no marker = no bit2. Then the pre-marker behavior (first screen) must come out.
  it.each([0x00, 0x01, 0x02, 0x03])('without bit2 (flags=%i) it is not a resume', (flags) => {
    expect(decodeReplayMarker(buildMarker({ epoch: 1, gen: 1n, flags }))?.continuesConversation).toBe(
      false,
    )
  })

  it('the length is 38 — 37-byte and old 30-byte markers are not markers', () => {
    expect(decodeReplayMarker(buildMarker({ epoch: 1, gen: 1n, flags: 0x04 }))).not.toBeNull()
    expect(decodeReplayMarker(buildMarker({ epoch: 1, gen: 1n, flags: 0x04, length: 37 }))).toBeNull()
    expect(decodeReplayMarker(buildMarker({ epoch: 1, gen: 1n, flags: 0x04, length: 30 }))).toBeNull()
  })

  // ADR-0231: flush start point = max(last+1, replay head). If the field position (30..38 BE) is off, the head becomes a wrong value and
  //   the view skips history or waits for a seq that will never come.
  it('reads the replay head (30..38 BE) as a number like seq and does not mix it with flags', () => {
    expect(
      decodeReplayMarker(buildMarker({ epoch: 9, gen: 3n, flags: 0x07, replayFrom: 40n })),
    ).toMatchObject({ gen: 3n, truncated: true, failed: true, continuesConversation: true, replayFrom: 40 })
    // Up to the high bytes in BE — a seq beyond 2^32 comes through as is.
    expect(
      decodeReplayMarker(buildMarker({ epoch: 9, gen: 3n, flags: 0, replayFrom: 0x1_0000_0002n }))
        ?.replayFrom,
    ).toBe(0x1_0000_0002)
  })
})
