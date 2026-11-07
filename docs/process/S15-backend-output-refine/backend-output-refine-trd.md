# S15 TRD — moving output refinement to the backend (payload-generic pipeline + tag1 frame)

> **Why/what = ADR-0045**. This document = **how** (seam · signature · volume · tests · order). Invariants = ADR-0002/0003/0004/0006/0040/0043.
> **Confirmed decisions (user):** ① parser = decoder layer (the transport carries only bytes, a backend-owned decoder reassembles + parses) · ② a single `Ring<StoredOutput>` · ③ wire = `tag0` bytes / `tag1` StructuredEvent (self-describing) · ④ the original "B" reload outage is out of scope.

## 1. Purpose · scope
Move the claude `stream-json` (NDJSON) refinement from the frontend to a backend decoder, flow typed events as `tag1` frames, and reuse the client shared-buffer append/replay (ADR-0040/0043) logic. The frontend discards the parser and branches on the tag.
**Out of scope:** the original "B" (reload outage on the terminal-bytes path) = a pre-existing bug, separate. ToolCall/Usage/MessageDone/Error rendering · permission UX = DEFER.

## 2. Data flow
```
claude stdout(NDJSON bytes)
   ▼  StdioTransport.pump: read → emit(bytes)         ← 순수 파이프(변경 최소)
   ▼  decoder(backend 소유): 라인재조립 + claude 파싱 → OutputEvent   ← claude 지식 여기(ADR-0004)
   ▼  OutputCore: Ring<StoredOutput>(seq·byte budget·cursor) 저장, emit lock 밖 send(ADR-0006)
   ▼  OutputPayload{Bytes|Event}(코어 도메인, 빌림)
   ▼  daemon adapter: core OutputEvent → wire 타입 변환(ADR-0003)
   ▼  WsOutputSink: WirePayload → frame(tag0=bytes / tag1=StructuredEvent 직렬화)
   ▼  codec.decode_frame: known tag 헤더-only 파싱(payload opaque)
   ▼  클라 공유버퍼 append/replay ── ★로직 재사용(무변경)★, decode 게이트만 넓힘
   ▼  reload resubscribe replay(ADR-0043) ── 재사용
   ▼  프론트 wsFrame decode(tag 분기): tag0→TerminalSlot / tag1→RichSlot(파서 제거)
```

## 3. Design per seam

### Backend (Rust)
| # | File | Change | Content |
|---|---|---|---|
| B1 | `core/agent/types.rs` | `OutputEvent` extension (✅module 1) | `TextDelta`/`ToolCall{name,args_json,optional id}`/`Usage`/`MessageDone`/`Error`/`Structured{kind,json}` (escape hatch) + optional `turn_id`/`message_id`. Keep `#[derive(Debug,Clone)]` — **`Serialize` is attached to the wire type in the B7 daemon adapter** (core does not know the wire format = ADR-0003, avoids prematurely fixing the serialization format = §0). |
| B2 | `core/agent/backend/claude.rs` | **decoder** new | `LineDecoder` (line-reassembly state + claude JSON→OutputEvent). Owned by the backend. transport · core see this type only as dyn |
| B3 | `core/agent/` (assembly point: session/manager) | decoder injection | Plug the decoder between the pump and the core. The transport keeps emitting bytes. In json mode there is a decoder, in terminal mode there is none (bytes pass straight through). **★Must not be done before B4** — before B4 (payload-generic emit) structured events are silently dropped in the `output_core` `_` arm (module 1 attached a `debug_assert!`+`warn` dormant guard to that arm so that an order violation is loud). |
| B4 | `core/agent/session.rs` (ReplayBuffer) · `output_core.rs` (emit) | generalize to `Ring<StoredOutput>` | Storage unit `StoredOutput{seq,payload,cost_bytes}`. `cost()` = the byte len / the encoded size of the event. **emit lock discipline unchanged (ADR-0006)** |
| B5 | `core/agent/types.rs` | `OutputPayload{Bytes\|Event}` | `OutputFrame.data:&[u8]`→`OutputPayload` (Copy/borrowing kept). Update the `OutputSink::send` signature |
| B6 | `protocol/codec.rs` | `tag1` + header-only gate | `tag1` encode/decode (payload = the serialized event, the codec does not know the content). `decode_frame` parses only the header for a known tag with the payload opaque. golden |
| B7 | `daemon/ws.rs` (+adapter) | core→wire conversion + frame encoding | The daemon adapter turns `OutputEvent`→a wire type. `WsOutputSink` turns `OutputPayload::Event`→a `tag1` frame |
| B8 | `src-tauri/daemon_client/connection.rs` | relay (confirm mostly unchanged) | If decode_frame succeeds on tag1, the existing Ok path stores the frame and replays it (logic reused). Only confirm tag1 passes |

### Frontend (TS)
| # | File | Change | LOC |
|---|---|---|---|
| F1 | `api/wsFrame.ts` | tag branch (tag1→header + opaque payload→StructEvent deserialization) | +~20 |
| F2 | `api/transport.ts` · `wsTransport.ts` | `InboundMessage` union + onmessage branch | +~15 |
| F3 | `api/protocolClient.ts` · `agentClient.ts` | `handleOutput` tag branch + `OutputChunk` union (seq dedup · epoch guard kept) | +~35 |
| F4 | `components/slot/RichSlot.tsx` | consumes typed events directly | +~15 |
| F5 | `lab/richslot/streamParse.ts` · `parse.ts` | remove the NDJSON parser | -189 (+replacement ~40) |
**Confirmed unchanged:** `TerminalSlot` · `renderMode` · `ViewLayoutRenderer` · the pre-subscribe buffer · the reload path (tag branch only). ts-rs bindings auto-generated.

## 4. Volume
- **Backend ~500–650 LOC** — decoder + line reassembly (B2, ~150 max) · buffer generalization (B4, ~90) · codec tag1 + gate + golden (B6, ~90) · the rest (B1/B3/B5/B7/B8) small + module tests.
- **Frontend net ≈ -100 LOC** — tag decode · union · RichSlot (+~85), parser removal (-189+40).
- **Scope:** core · protocol · daemon + backend (claude decoder) + src-tauri relay confirmation + 5 frontend files. **High-risk seams (ADR-0003 contract · ADR-0006 lock)** → coder high · `/review code` deep · `/qa` full.
- **Feel:** a medium-to-large single feature, several days. Commit per module.

## 5. Tests (TDD + module isolation, ADR-0012)
- **codec (B6):** golden — tag0/tag1 encode↔decode round-trip, header-only parsing, unknown tag rejection. `cargo test -p ...-protocol`.
- **decoder (B2):** claude stream-json fixtures (measured captures) → OutputEvent assertions + a **partial-line (split at a chunk boundary) reassembly** case. Pure-function isolation.
- **buffer (B4):** `Ring<StoredOutput>` push/replay/eviction (byte budget + count boundary) · ReplayKind. headless.
- **emit lock (B4/B5):** existing output_core regression + `OutputPayload` fanout.
- **★client opaque replay (new):** receive a `tag1` frame → shared-buffer append → the resubscribe replay returns **the same original frame bytes**. (Do not rely only on cdp measurement — catch regressions early.)
- **Frontend (F):** vitest — wsFrame tag branch · protocolClient seq dedup/epoch guard regression.
- **Integration/measurement:** `cargo test` (workspace) + `scripts/cdp.mjs` — JSON agent output→RichSlot render + **typed-output replay restored after a webview reload**.

## 6. Implementation order (dependency order)
1. **B1 OutputEvent + B6 codec tag1 contract + golden** — fix the types · wire contract first.
2. **B2 decoder (+ line-reassembly tests, fixtures).**
3. **B4 Ring<StoredOutput> generalization** (mind the lock-discipline regression) + **B5 OutputPayload/OutputSink.**
4. **B7 daemon adapter + WsOutputSink frame encoding** + **B8 confirm the client relay passes tag1** + the client opaque replay test.
5. **B3 decoder injection** (assembly point) — complete the E2E wiring.
6. **F1~F4 frontend tag branch** → **F5 parser removal.**
7. Integration · cdp measurement (reload replay consistency).

## 7. Invariants · risks
- **ADR-0006** — send outside the emit lock. The single ring keeps the existing structure (0 new locks).
- **ADR-0003** — core `OutputEvent` ≠ wire type, conversion in the daemon adapter, core tauri-import 0.
- **ADR-0004** — the claude schema = the decoder (backend/claude.rs) only. transport · core see only the dyn decoder.
- **decoder injection point (B3)** — the transport keeps emitting bytes, the decoder consumes the pump output and passes it to the core. The injection location (session vs the manager assembly point) is settled during coding, with keeping the transport/core pure as the constraint.
- **codec gate ripple (B6)** — `decode_frame` is shared by daemon and client. Allowing tag1 passes on both sides (intended). Unknown (≥2) tags are still rejected.
- **epoch · seq dedup (ADR-0007)** — the frontend tag branch does not bypass them (F3).
- **The original "B" bug** — out of scope. The typed path reuses append/replay so it does not reintroduce it.
