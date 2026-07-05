// WS binary output frame 디코드 — codec.rs `encode_terminal_frame`/`decode_frame` 의 역.
// WsTransport 가 사용. 순수 함수 — 테스트 용이(wsFrame.test). (Stage 4a: DaemonClient re-export 제거됨.)
//
// 포맷(big-endian): [tag:1][agentId:16][epoch:4 BE][seq:8 BE][raw payload...].

// ── codec.rs binary frame 상수(반드시 codec.rs 와 일치) ─────────────────────────────
// tag0 = 터미널 raw 바이트(xterm write), tag1 = StructuredEvent JSON(payload=serde_json, ADR-0045).
// 헤더 포맷은 tag 무관 동일 — payload 해석만 tag 로 갈린다(소비자=ProtocolClient.handleOutput).
export const FRAME_TAG_TERMINAL_BYTES = 0
export const FRAME_TAG_STRUCTURED_EVENT = 1
const FRAME_HEADER_LEN = 1 + 16 + 4 + 8 // 29

/**
 * binary output frame 디코드. 헤더 미만 길이·미지원 tag(≥2) 시 null(무시).
 * tag0/tag1 은 둘 다 통과시키고 payload 는 raw 로 넘긴다 — tag 별 해석(바이트 vs JSON)은 소비자 몫.
 */
export function decodeOutputFrame(
  buf: ArrayBuffer,
): { tag: number; agentId: string; epoch: number; seq: number; payload: Uint8Array } | null {
  if (buf.byteLength < FRAME_HEADER_LEN) return null
  const view = new DataView(buf)
  const tag = view.getUint8(0)
  // codec.rs: tag0=TerminalBytes / tag1=StructuredEvent 둘만 유효, 그 밖은 UnknownTag → 조용히 skip(null).
  // (F1 회귀: 옛 코드는 tag1 도 null-drop 해 구조화 출력이 무음 유실됐다 — tag1 도 통과시킨다.)
  // ★ADR-0046 전방 호환(M0)★: src-tauri 가 앞으로 replay 경계 마커(tag=255, Channel 내부 계약)를 같은
  //   출력 Channel 로 흘린다 — 현 프론트는 그 미지 tag 를 여기서 조용히 skip 해야 한다(마커 소비는 M2).
  //   미지 tag 를 던지거나 payload 로 오해하지 않게, tag0/tag1 외는 전부 null 로 버린다(길이 무관).
  if (tag !== FRAME_TAG_TERMINAL_BYTES && tag !== FRAME_TAG_STRUCTURED_EVENT) return null

  // agentId: byte[1..17] = AgentId(Uuid).as_bytes() — RFC4122 network order(표준 바이트 그대로).
  // 16바이트 hex 후 8-4-4-4-12 하이픈 삽입 = 구독 시 보낸 소문자 하이픈 UUID 와 동일 표현.
  const bytes = new Uint8Array(buf, 1, 16)
  const agentId = bytesToUuid(bytes)

  // epoch/seq: codec.rs 가 to_be_bytes — BE 로 읽는다(false=big-endian).
  const epoch = view.getUint32(17, false)
  const seq = Number(view.getBigUint64(21, false)) // seq 는 number 로 유지(설계 결정)

  const payload = new Uint8Array(buf, FRAME_HEADER_LEN)
  return { tag, agentId, epoch, seq, payload }
}

const HEX: string[] = Array.from({ length: 256 }, (_, i) => i.toString(16).padStart(2, '0'))

/** 16바이트 UUID → 소문자 하이픈 문자열(8-4-4-4-12). uuid 표준 바이트 순서 그대로. */
function bytesToUuid(b: Uint8Array): string {
  return (
    HEX[b[0]] + HEX[b[1]] + HEX[b[2]] + HEX[b[3]] + '-' +
    HEX[b[4]] + HEX[b[5]] + '-' +
    HEX[b[6]] + HEX[b[7]] + '-' +
    HEX[b[8]] + HEX[b[9]] + '-' +
    HEX[b[10]] + HEX[b[11]] + HEX[b[12]] + HEX[b[13]] + HEX[b[14]] + HEX[b[15]]
  )
}
