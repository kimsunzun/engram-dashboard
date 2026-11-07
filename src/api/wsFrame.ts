// WS binary output frame decode — the inverse of codec.rs `encode_terminal_frame`/`decode_frame`.
//
// Format (big-endian): [tag:1][agentId:16][epoch:4 BE][seq:8 BE][raw payload...].

// ── codec.rs binary frame constants (must match codec.rs) ─────────────────────────────
// tag0 = terminal raw bytes (xterm write), tag1 = StructuredEvent JSON (payload=serde_json, ADR-0045).
// The header format is the same regardless of tag — only the payload interpretation depends on the tag.
export const FRAME_TAG_TERMINAL_BYTES = 0
export const FRAME_TAG_STRUCTURED_EVENT = 1
const FRAME_HEADER_LEN = 1 + 16 + 4 + 8

// ★Replay boundary marker tag (ADR-0046 — Channel-internal contract)★: absent from the daemon codec. src-tauri synthesizes it at each replay
//   end and sends it down the same output Channel, and only the transport (decodeReplayMarker) interprets it. Its seq space differs from daemon binary
//   frames (tag0/1), so decodeOutputFrame still skips this tag (forward compatibility).
export const FRAME_TAG_REPLAY_MARKER = 255
// Marker format (big-endian): [tag=255:1][agentId:16][epoch:4 BE][gen:8 BE][flags:1][replay_from:8 BE] — length 38.
//   flags: bit0=truncated · bit1=failed · bit2=continues_conversation (matches src-tauri replay_flight).
//   replay_from = the head of that replay (the same field in SubscribeAck — ADR-0231). Since the shell and webview ship in one build,
//   a marker shorter than this (the old 30 bytes) is not accepted — it falls to non-marker and the view takes the watchdog · ladder.
const MARKER_LEN = 1 + 16 + 4 + 8 + 1 + 8
const MARKER_FLAG_TRUNCATED = 0x01
const MARKER_FLAG_FAILED = 0x02
// ADR-0226: this incarnation came up to resume a saved conversation — not whether the resume succeeded. A failure marker is always 0.
const MARKER_FLAG_CONTINUES = 0x04

/**
 * The text of the placeholder `Error` — **byte-identical** to `PLACEHOLDER_ERROR_MESSAGE` in the protocol `placeholder.rs` (a test
 * measures it). It is an opaque human-readable string — do not branch on this value.
 */
// ADR-0231
export const PLACEHOLDER_ERROR_MESSAGE = '이 출력 사건을 싣지 못했습니다'

/**
 * The placeholder tag1 payload — the JSON of `StructuredEvent::Error { message: PLACEHOLDER_ERROR_MESSAGE }`, byte-identical to the payload the daemon
 * sink · shell relay carries. A new array on every call.
 */
// ADR-0231
export function placeholderErrorPayload(): Uint8Array {
  return new TextEncoder().encode(JSON.stringify({ type: 'Error', message: PLACEHOLDER_ERROR_MESSAGE }))
}

/**
 * Reads only the binary frame head — does not tell tags apart (the inverse of codec.rs `peek_frame_header`). Null if shorter than the head
 * — a frame whose seq cannot be read.
 */
export function peekFrameHeader(
  buf: ArrayBuffer,
): { tag: number; agentId: string; epoch: number; seq: number } | null {
  if (buf.byteLength < FRAME_HEADER_LEN) return null
  const view = new DataView(buf)
  const tag = view.getUint8(0)
  // agentId: byte[1..17] = AgentId(Uuid).as_bytes() — RFC4122 network order (the standard bytes as they are).
  // Hex of the 16 bytes then insert hyphens 8-4-4-4-12 = the same representation as the lowercase hyphenated UUID sent at subscribe.
  const agentId = bytesToUuid(new Uint8Array(buf, 1, 16))
  // epoch/seq: codec.rs uses to_be_bytes — read as BE (false=big-endian).
  const epoch = view.getUint32(17, false)
  const seq = Number(view.getBigUint64(21, false)) // Keep seq as a number (design decision)
  return { tag, agentId, epoch, seq }
}

/**
 * Decodes a binary output frame. Null (ignored) for a length shorter than the header or an unsupported tag (≥2).
 * The payload is passed as raw — per-tag interpretation (bytes vs JSON) is the consumer's job.
 */
export function decodeOutputFrame(
  buf: ArrayBuffer,
): { tag: number; agentId: string; epoch: number; seq: number; payload: Uint8Array } | null {
  const header = peekFrameHeader(buf)
  if (header === null) return null
  // F1 regression: the old code also null-dropped tag1, so structured output was silently lost — now tag1 passes too.
  // ★ADR-0046 forward compatibility (M0)★: the replay boundary marker (tag=255) also comes down the same output Channel but is not interpreted
  //   here (consumption is decodeReplayMarker, M2). So an unknown tag is not mistaken for a payload, everything except tag0/tag1 is dropped as null
  //   regardless of length.
  if (header.tag !== FRAME_TAG_TERMINAL_BYTES && header.tag !== FRAME_TAG_STRUCTURED_EVENT) return null
  return { ...header, payload: new Uint8Array(buf, FRAME_HEADER_LEN) }
}

/**
 * Decodes the replay boundary marker (tag=255) — the inverse of the src-tauri synthesized frame (ADR-0046). Null (non-marker) for a length shorter than the header or another tag. gen is a u64 so it is read
 * as a BigInt (getBigUint64) — matching the width of the requestReplay return value so the gen fence
 * comparison is exact without precision loss.
 */
export function decodeReplayMarker(
  buf: ArrayBuffer,
): {
  agentId: string
  epoch: number
  gen: bigint
  truncated: boolean
  failed: boolean
  continuesConversation: boolean
  replayFrom: number
} | null {
  if (buf.byteLength < MARKER_LEN) return null
  const view = new DataView(buf)
  if (view.getUint8(0) !== FRAME_TAG_REPLAY_MARKER) return null
  const agentId = bytesToUuid(new Uint8Array(buf, 1, 16))
  const epoch = view.getUint32(17, false)
  const gen = view.getBigUint64(21, false)
  const flags = view.getUint8(29)
  return {
    agentId,
    epoch,
    gen,
    truncated: (flags & MARKER_FLAG_TRUNCATED) !== 0,
    failed: (flags & MARKER_FLAG_FAILED) !== 0,
    continuesConversation: (flags & MARKER_FLAG_CONTINUES) !== 0,
    // Read at the same width as seq (the seq of decodeOutputFrame is also a number) — flush compares the two.
    replayFrom: Number(view.getBigUint64(30, false)),
  }
}

const HEX: string[] = Array.from({ length: 256 }, (_, i) => i.toString(16).padStart(2, '0'))

/** 16-byte UUID → lowercase hyphenated string (8-4-4-4-12). The standard uuid byte order as is. */
function bytesToUuid(b: Uint8Array): string {
  return (
    HEX[b[0]] + HEX[b[1]] + HEX[b[2]] + HEX[b[3]] + '-' +
    HEX[b[4]] + HEX[b[5]] + '-' +
    HEX[b[6]] + HEX[b[7]] + '-' +
    HEX[b[8]] + HEX[b[9]] + '-' +
    HEX[b[10]] + HEX[b[11]] + HEX[b[12]] + HEX[b[13]] + HEX[b[14]] + HEX[b[15]]
  )
}
