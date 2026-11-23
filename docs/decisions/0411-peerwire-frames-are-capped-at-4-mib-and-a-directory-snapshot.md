# ADR-0411: PeerWire frames are capped at 4 MiB and a directory snapshot goes as one frame; no multi-part split or assembly

- 상태: 확정 (2026-11-23, 근거: user decision 2026-11-23 (「iroh로 보낼때는 그쪽룰로 하면 되는거 아님?」 · 「어 그렇게하면 되긴하는데」) + Reason Network precedent)
- 관련: Amends ADR-0391 (decision 4 multi-part snapshots) · ADR-0390 (survive a broken peer) · TRD T30 (frame cap) · `docs/reference/structure/peer-link.md`

## 맥락
ADR-0391 decision 4 sends a directory snapshot in parts above the PeerWire frame cap (512 KiB, TRD T30 — sized from the worst-case JSON escaping of a 64 KiB mail body) and the receiver buffers parts until the `last` one arrives. Review found the in-progress buffer unbounded: a trusted but broken PC could keep sending parts. While discussing a cap the user pointed out that QUIC (iroh) already splits and reassembles anything; our own split exists only because of our own frame cap. One frame of 512 KiB already holds ~5000 rows of ~100 bytes.

## 결정
1. **The PeerWire frame cap is 4 MiB**, both directions (`MAX_FRAME_BYTES`; the link adapter's inbound limit follows it). It stays above the mail-escaping bound of TRD T30 and holds a whole directory of ~40k rows.
2. **A directory snapshot is one frame** — no `part` / `last`, no receiver-side assembly. An oversized coalesced delta goes as one snapshot frame. The `run` / `rev` / `DirResync` rules of ADR-0391 decision 4 are unchanged.
3. **A directory over the cap** is sent with the rows past the cap left out (a deterministic order) and one warning per publish; the link is never closed and the snapshot is never split. The copy is then incomplete, which ADR-0391 R16 otherwise forbids; at ~40k agents per PC this is accepted.
4. Mail is unaffected: the 64 KiB body check and the per-frame row limits stay.

## 거부한 대안
- **Keep 512 KiB parts and cap only the in-progress buffer** (the reviewer's fix) — keeps a second split on top of QUIC's for a case (> ~5000 agents) that does not occur; user rejected directly (「iroh로 보낼때는 그쪽룰로 하면 되는거 아님?」).
- **Keep parts with no cap** (ADR-0391 as written) — a broken trusted PC can grow the receiver's memory without bound; HashiCorp memberlist bounds push/pull state at 20 MiB even between keyed peers (`net.go` `maxPushStateBytes`).
- **Each snapshot on its own QUIC stream, read with a size limit** — removes the split too, but changes the link adapter's one-stream model for no extra gain over one larger frame (not measured — 약함).
- **A larger cap such as 16 MiB** — 4 MiB follows Reason Network (`src/net.rs` `MAX_FRAME = 4 << 20`, whole length-prefixed JSON frames over iroh) and already covers ~40k rows.
- **Close the link or split when the directory exceeds the cap** — closing loses mail for an unrealistic case; splitting brings back the assembly this ADR removes.

## 근거
- Reason Network sends length-prefixed JSON frames whole over one iroh stream with a single 4 MiB cap and no multi-part (local clone `I:\Engram_Workspace\opensource\reason-network` @ `5442c8b5`, `src/net.rs` `write_frame` / `read_frame`).
- Size: a row (UUID id + name + state) is ~100 bytes; 1000 agents ≈ 100 KB.
- PeerWire v1 is unreleased, so the wire change needs no compatibility path.

## 영향 / 불변식
- `DirSnapshot` has no part fields; the copies keep no in-progress state.
- Any new PeerWire message that could outgrow 4 MiB must be bounded by its producer, not split by the link.
