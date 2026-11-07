# ADR-0052: Duplicate removal for the user echo in json mode = uuid/isReplay-based dedup (the blunt suppress is discarded)

- 상태: 확정 (2026-07-06, basis: uuid round-trip measurement + OSS survey + /review code deep PASS)
- 관련: CLAUDE.md §5 (LLM-first control) · S10 backend abstraction · ADR-0044/0045 (claude stream-json decoder + input-time echo) · ADR-0004 (backend isolation) · 0006 (lock) · 0045/0046 (accumulator idempotence, replay→live) · code: `crates/engram-dashboard-core/src/agent/backend/claude.rs` (wrap_user_turn, consume_block) · `backend/mod.rs` (input_echo_event) · `agent/session.rs` (write_input) · `src/components/slot/structuredAccumulator.ts` (extractUserUuid, seenUserUuids) · step-log 2026-07-07

## 맥락
json (StreamJson) mode has no local echo like a terminal PTY — a message the user types does not appear on screen until claude's round-trip replay. For perceived responsiveness, we emit a synthetic user event (an optimistic echo) as soon as `write_input` succeeds, but when claude echoes the same message back with `--replay-user-messages`, it becomes a duplicate. How to remove this duplicate was the problem. The initial implementation (follow-up to ADR-0044/0045) deduplicated by having the decoder **suppress every user-role `type=="text"` block unconditionally**.

## 결정
**Match the optimistic echo and the replay by a client-generated uuid and dedup only that pair**:
- `write_input` generates one `Uuid::new_v4()` per call and attaches **the same value** to both (a) the user message going out over stdin (`wrap_user_turn`) and (b) the synthetic echo event.
- The decoder (`consume_block`) **does not suppress** the replayed user block and passes it through carrying the line-level uuid.
- The frontend `structuredAccumulator` dedups by uuid **only the `type=="text"` user blocks** (synthetic uuid == replay uuid → 1 item). tool_result, non-text, a different uuid and past history are always kept.
- Seam = **S1 (frontend dedup)**: zero shared mutable backend state, wire unchanged (the uuid rides inside `Structured.json`).

## 거부한 대안
- **A. blunt suppress (suppress user text unconditionally) — discarded.** It works now, but it is correct only under the premise that json-mode resume is disabled (fixed to a fresh session). The moment resume/history replay is turned on, **even the past user messages** that claude echoes back are all deleted, and the conversation history disappears from the screen. The cross-family reviewer (Codex) raised it as BLOCK (HIGH) — correctness is tied to an implicit invariant (resume off), and a test pins that suppression as "right", hiding a future regression: a latent trap.
- **B. Backend pending-set (S2) — rejected.** The session keeps the set of uuids it sent, and the decoder suppresses only when `isReplay && uuid∈집합`. Correct, but it needs shared mutable state between session and decoder, which makes the lock and isolation (ADR-0006/0004) burden larger. S1 achieves the same correctness without shared state.
- **C. Remove the optimistic echo and render with replay as the only source of truth — rejected.** The dedup logic disappears altogether, which is the simplest, but the round-trip delay between sending and display becomes visible on screen (undoing the original purpose of adding the echo). To keep the perceived responsiveness, we adopt echo + uuid dedup.

## 근거
- **Measured (confirmed):** if claude is launched with `--replay-user-messages` and a top-level `uuid` is planted in the stdin user message, the replayed stdout user line **preserves that uuid as it is** and carries `isReplay:true` (observed directly in this session: sent uuid `X` → got back `X` + `isReplay:true`). It is confirmed as fact that precise id correlation is possible.
- **OSS survey:** the standard of mature chat systems (Discord `nonce`, Matrix `txnId`, XMPP `origin-id`) is a client-generated correlation-id + pending matching, and "always discarding the user echo" is a documented anti-pattern that breaks with resume/history/multi-send. **The official VS Code Claude extension (measured from the installed code) also dedups by the message `uuid` + stores the transcript** — the adopted approach is the same as the official tool.
- **Verification:** `/review code deep` with three adversarial reviewers — all three found the defect that the initial implementation lost items in multi-block lines (text + tool_result with the same uuid) → after the FIX limiting dedup to `type=="text"` blocks, two reviewers re-reviewed and passed. Full build · core 144 + integration · protocol 42 · vitest 277 · tsc · fmt · isolation gate PASS. uuid round-trip measured.

## 영향 / 불변식
- **The dedup key = the client uuid of a `type=="text"` user block.** Non-text blocks such as tool_result are not dedup targets (always kept) — `extractUserUuid` returns `null` for non-text. If this boundary is broken (going back to the uuid alone as the key), tool_result is lost in multi-block lines.
- **The synthetic echo and the stdin message must carry the same uuid** — `write_input` injects one value into both. If they diverge, dedup fails (double rendering).
- **ADR-0004 isolation kept** — the claude json schema (where uuid/isReplay sit) lives only in `backend/claude.rs`. session/mod pass only an opaque `Uuid` token.
- **ADR-0006 lock** — zero new locks, `core.emit` reused. **ADR-0045 accumulator idempotence** — `reset()` also clears `seenUserUuids` → it reconverges in the replay→live rebuild (ADR-0046).
- **Unconfirmed (behavior):** GUI measurement (dedup on the real screen) had not been done at the time of this ADR — the logic was verified by review + unit tests + a uuid measurement probe, but end-to-end screen confirmation is pending. Commit `dbced5a` is local only (push held).
- Observation (non-blocking): `seenUserUuids` grows without bound over the life of a single epoch (cleared at every reset, low risk in real use).
