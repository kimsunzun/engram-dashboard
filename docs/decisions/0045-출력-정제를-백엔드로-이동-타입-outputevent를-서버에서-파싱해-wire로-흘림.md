# ADR-0045: Move output refinement to the backend — parse the typed OutputEvent on the server and carry it on the wire

- 상태: 확정 (2026-07-03, basis: §5 hands/brain separation + ADR-0002 no assumption about output kinds + the virtual terminal requirement + measurement of the output pipeline seam code (3 re-reviews) + user decision) · 부분 폐기 by ADR-0231 (adding a waiting list to the render model and narrowing the scope of uuid dedup)
- 관련: Amends ADR-0044 (the unrefined channel · frontend parsing → switched to backend server-side refinement (typed OutputEvent)) · ADR-0002 (no assumption about output kinds · no forcing a terminal — this design is its realization) · ADR-0003 (OutputSink/OutputFrame contract · core isolation — the target of the payload extension) · ADR-0004 (claude knowledge isolation) · ADR-0006 (lock order) · ADR-0030 · ADR-0040 (server-authoritative bounded buffer) · ADR-0043 (mount-replay) · Amended by ADR-0231 (adding a waiting list to the render model and narrowing the scope of uuid dedup)

## Terms (self-contained)
- **`OutputEvent`** — an extensible enum at the pump/core boundary. `TerminalBytes(Vec<u8>)` + new structured variants (`TextDelta`/`ToolCall`/`Usage`/`MessageDone`/`Error`, optional `turn_id`/`message_id`, + the `Structured{kind,json}` escape hatch).
- **decoder** — a converter owned by the backend (not a buffer). bytes → line reassembly → claude JSON parsing → `OutputEvent`. Knowledge of the claude schema lives only here (ADR-0004).
- **`Ring<StoredOutput>`** — a single history buffer that holds seq · bounded (byte budget) · cursor replay. `StoredOutput{ seq, payload, cost_bytes }`. The pump input event and the stored type are separated.
- **`OutputPayload<'a>{ Bytes(&[u8]) | Event(&OutputEvent) }`** — the core domain payload (borrowed). **`WirePayload`** — the name of the sink/daemon encoding stage (no wire concept leaks into the core).
- **frame tag** — the physical wire representation. `tag0` = TerminalBytes, `tag1` = StructuredEvent (payload = a self-describing serialized event). The codec does not know the event schema (payload opaque).

## 맥락
ADR-0044 wired JSON mode as "an unrefined channel (a dumb pipe) + frontend parsing". NDJSON flows as `OutputEvent::TerminalBytes`, and the frontend `RichSlot` is the only parser.

Problems (confirmed by code measurement · 3 re-reviews):
- **§5 violation** — the understanding of structure exists only in the webview, and the backend-side LLM (the brain) cannot see its own output.
- **Byte flattening** — polymorphism exists only in `OutputEvent` (pump/core), and below the sink it is bytes only (`OutputFrame{data:&[u8]}` · `OutputSink` · replay bytes).
- **Virtual terminal** — a VT byte stream (terminal) and discrete events (structured LLM) are payloads of essentially different natures. Unifying them into a single structured buffer distorts by forcing a terminal (violating ADR-0002).
- **The client relay validates and rejects the tag** — `connection.rs` calls `decode_frame` before storing into the buffer, and `codec.rs` rejects `tag≠0` as `UnknownTag`. So if a new tag is simply passed through, it is not even stored in the client buffer and a reload breaks → the old B recurs. The wire/codec choice hinges on this (↓ decision).

## 결정
**Keep payload polymorphism, but have the backend refine with a decoder and carry the typed event as a binary frame (a single `tag1`), reusing the append/replay logic of the client shared buffer.**

1. **Backend refinement = a decoder layer (the implementation fork = B, user decision)** — the `transport` (pump) **emits bytes as they are** (keeping a pure pipe, uniform with `PtyTransport`). Between the pump and the core, a **backend-owned decoder** does line reassembly + claude parsing → `OutputEvent`. The transport and the core have no knowledge of claude/lines (ADR-0003/0004). (Rejected: the transport pump owning parsing too = non-uniform transport responsibility. Basis = only the decoder is swapped per backend, the transport is shared as one.)
2. **A single `Ring<StoredOutput>` buffer (user decision)** — one history buffer per agent. Terminal = a bytes payload, JSON = an event payload. **No dual storage of original/converted.** The stored type (`StoredOutput`) is separated from the pump event (`OutputEvent`). Eviction = **byte budget (encoded payload size) + an event count cap** (so that a large `args_json` does not slip through as a count of 1).
3. **Wire = frame `tag0`/`tag1` (user decision)** — typed events go not per variant but as **a single `tag1 StructuredEvent`** (payload self-describing). If `codec.decode_frame` is widened so that **a known tag parses only the header (payload opaque)** → the **client shared buffer's append/replay logic is reused** (that logic unchanged). The same seam for codex/gemini and the API.
4. **core ↔ wire conversion = the daemon adapter (ADR-0003)** — the conversion is placed not in the protocol crate but in the daemon (the core has only `OutputEvent`, the protocol has only wire types, the daemon couples them). `WsOutputSink` encodes `WirePayload` into a frame.
5. **Frontend** — `wsFrame.decodeOutputFrame` branches by tag (`tag0` → `TerminalSlot` xterm / `tag1` → `RichSlot` typed render). The `RichSlot` NDJSON parser is removed.

## 거부한 대안
- **Keep frontend parsing (the ADR-0044 MVP · current)** — the structure is trapped in the webview and the brain cannot see it (§5). → *The target of Amends.*
- **A typed JSON `AgentEvent::Output` (Text arm, without a frame tag)** — the client shared buffer is attached only to the frame path (Binary arm), so the Text arm would need seq/cursor/replay newly created (more expensive + a risk of a reload break). The frame tag is cheaper through append/replay reuse.
- **A frame tag per variant (`tag1` = TextDelta, `tag2` = ToolCall…)** — the codec would change with every event schema change. A single `tag1 StructuredEvent` + a self-describing payload keeps the codec schema-ignorant (swappability ↑).
- **Unify into a single structured (JSON) buffer** — forcing the virtual terminal's raw VT into discrete events = loss + violating ADR-0002.
- **Two buffer classes (`TerminalBuffer`/`StructuredBuffer` + trait)** — there is only one buffer per agent, so dyn/trait only increases the risk of hiding the lock discipline (ADR-0006). A single `Ring<StoredOutput>` + `cost()` absorbs the eviction difference.
- **Bytes + a typed sidecar (a parallel channel)** / **a backend-only typed view** — respectively a double replay burden / frontend re-parsing remaining (falls short of §5 "frontend = pure rendering").
- **Daemon re-request/re-hydration (the part ADR-0040 rejected)** — unrelated, still rejected.

## 근거
- **§5 hands/brain separation (invariant)** · **ADR-0002** (realizing polymorphism · no forcing a terminal) · **the virtual terminal requirement** (forces keeping the bytes payload).
- **The client relay validates the tag before storing (code-verified in the re-review)** — so "no change at all on the client" is false. Correction: if the `codec.decode_frame` tag gate is widened, **only the append/replay logic is reused** (unchanged), while the codec gate and the frontend decode are the change surface. Still cheaper than the JSON path (JSON would create replay anew).
- **User decision (2026-07-03)** — server-side refinement + abstracting each payload (the pipe shared, only the decoder per backend) + ① B / ② single ring / ③ tag1 settled. §0 (low-risk seam, long term → do it properly now).

## 영향 / 불변식
- **Change surface (honest):** ① the `OutputEvent` structured variants + `Serialize` ② the decoder (backend/claude.rs, line reassembly + parsing) ③ generalizing the `Ring<StoredOutput>` buffer (session/output_core) ④ `codec` `tag1` + widening the decode_frame header-only gate + golden ⑤ the daemon adapter (core → wire conversion) + `WsOutputSink` frame encoding ⑥ `OutputSink`/`OutputFrame` → `OutputPayload` (ADR-0003, Copy/borrowed) ⑦ the frontend `wsFrame` (tag branching) + `protocolClient`/`agentClient` (union) + `RichSlot` (parser removed). **Reused (unchanged): the client shared buffer's append/replay · mount-replay logic** (only the decode gate is widened).
- **ADR-0006 lock** — the buffer generalization keeps the emit discipline "keep the replay lock short → release → send outside the lock" (a single ring keeps the existing structure as is, zero new locks).
- **ADR-0003 isolation** — core `OutputEvent` ≠ protocol wire type, and the conversion is the daemon adapter. `OutputPayload` is a core domain type (borrowed), and the wire encoding is the sink. Core tauri-import stays 0.
- **ADR-0004** — claude schema knowledge = only the decoder (backend/claude.rs).
- **Output kind = immutable within a session lifetime** — fixed by capability at spawn, switching = an epoch swap. No runtime switching.
- **Swappability** — the `Structured{kind,json}` escape hatch + optional `turn_id`/`message_id` absorb the leak of the codex/gemini/API event models.
- **MVP** — pipeline axis (decoder · tag1 · buffer · decode = passing TextDelta through) = MUST / render axis (TextDelta render MUST; ToolCall · Usage · MessageDone · Error render · permission UX DEFER).
- **Out of scope** — the original "B" (the terminal byte path's reload break) is a pre-existing bug, a separate follow-up. The typed path reuses append/replay so it does not reintroduce it.
- **ADR-0044 partial-discard boundary** — only 0044's "the unrefined channel / frontend parsing / typed rejected" is amended. "The unrefined channel" is redefined as limited to the transport layer (OutputCore/buffer/codec/daemon are payload-polymorphic). StdioTransport · the persistent process · input wrapping isolation remain valid. (Inline pointers placed at 0044 §31/§34.)
