# Contacts landed (actual shapes — authoritative over plan §3 proposals)

## P1a (WIP 2026-09-26) — `engram_dashboard_agent::queued_input`
- `OutputEvent::QueuedInput(QueuedInputEvent)`; `QueuedInputEvent`/`DropCause`/`DeliveredCopy` verbatim TRD L302-314 (no serde on domain types).
- protocol `StructuredEvent::QueuedInput { op: QueuedInputEvent }` (wire mirrors, `#[serde(tag="kind")]`, `DropCause` plain string). `PROTOCOL_VERSION` not bumped yet (P4b → 6). Bindings: protocol `StructuredEvent.ts`, `QueuedInputEvent.ts`, `DropCause.ts`, `DeliveredCopy.ts`.
```rust
pub const TOMBSTONE_CAP: usize = 1024;
pub enum CancelAnswer { Unanswered, NotRemoved }            // wire/golden "none" | "not_removed"
pub enum RowPhase { Queued, Cancelling { answer: CancelAnswer, vendor_closed: bool } }
pub struct QueuedRow { pub id: String, pub text: String, pub phase: RowPhase }
pub enum Verdict { Delivered, Cancelled, Discarded(DropCause) }   // Withdrawn always -> Cancelled
impl Registry {
  pub fn new() -> Self;
  pub(crate) fn reduce(&mut self, seq: u64, event: &QueuedInputEvent) -> Vec<(String, Verdict)>; // debug_assert seq strictly increasing; as_of_seq advances even for ignored events
  pub fn rows(&self) -> &[QueuedRow]; pub fn is_empty(&self) -> bool;
  pub fn ack_unavailable_seen(&self) -> bool; pub fn as_of_seq(&self) -> Option<u64>;
  pub fn open_copies(&self) -> Vec<DeliveredCopy>; // P1d: fill AckUnavailable.delivered BEFORE reducing it
  pub fn tombstone(&self, id: &str) -> Option<bool>; pub fn tombstone_count(&self) -> usize;
}
impl QueuedInputs { pub fn new() -> Self; pub(crate) fn lock(&self) -> MutexGuard<'_, Registry> /* poison -> into_inner; pub(crate) since dbe092e */; pub fn snapshot(&self) -> (Vec<QueuedRow>, Option<u64>); }
```
- `reduce` returns every verdict reached incl. tombstone-only ones (unknown id terminal, revival `Delivered`, reread `Discarded(Rejected)`); callers derive "existed before" from state before the call.
- Golden `crates/engram-dashboard-agent/src/queued_input_golden.json`: `{"tombstone_cap":1024,"_format":"…","cases":[{"name","events":[<wire op objects>],"expect":{items,tombstones,tombstone_count,absent,ack_unavailable,outcomes}}]}`; events fed with seq = index+1; `outcomes` = `{id: ["Delivered"|"Cancelled"|"Discarded:<cause>", …]}` (only listed ids checked). `items` rows = `{id,text,state:"queued"|"cancelling",cancel:null|{answer,vendor_closed}}`.
- Table gaps P1a filled (golden pins them): `Queued` + `CancelAnswered`/`CancelFailed` without request → ignored · `Cancelling{Unanswered,vendor_closed:true}` + second `Dropped{Unknown}` → no-op · unknown-id `Dropped{Withdrawn}` → tombstone, verdict `Cancelled` · tombstone + `Dropped{Rejected}` → always `Discarded:Rejected` · `AckUnavailable` copies applied with `Delivered` semantics after closing open items (a resurrectable tombstone among copies is revived).
- Poison policy: `QueuedInputs::lock` uses `into_inner` (differs from the replay lock's `expect` — deliberate).
- `structuredAccumulator.ts` has a placeholder `case 'QueuedInput': return false` — P1b replaces it.

## P1b (WIP) — TS
```ts
// src/components/slot/queuedInputReducer.ts
export const TOMBSTONE_CAP = 1024
export type CancelAnswer = 'none' | 'not_removed'
export type QueuedPhase = { readonly state: 'queued' } | { readonly state: 'cancelling'; readonly answer: CancelAnswer; readonly vendorClosed: boolean }
export interface QueuedEntry { readonly id: string; readonly text: string; readonly phase: QueuedPhase }
export type QueuedVerdict = { kind: 'Delivered' } | { kind: 'Cancelled' } | { kind: 'Discarded'; cause: DropCause }
export class QueuedInputRegistry { reduce(op): QueuedClosure[] | null; rows(); row(id); ackUnavailableSeen(); tombstone(id); tombstoneCount(); clear() }
// StructuredEventAccumulator: snapshotQueued(): QueuedEntry[] (phase queued AND uuid not drawn) · queuedRows() (raw state)
```
- Bubble = `{kind:'structured', label:'user', json: JSON.stringify({type:'text',text,uuid:id})}` (byte-identical to the synthetic echo). `turnDone` drops only when a bubble is drawn. `reset()` clears the registry incl. tombstones. Golden read via `src/components/slot/testing/queuedInputGolden.ts` (`?raw`).
- TS reducer has NO seq / `as_of_seq` — **P5c must add seq tracking** for reattach reconciliation (TRD L627).
- Orchestrator rulings on P1b findings: (1) `feed` returns true for `QueuedInput` → RichSlot clears `awaiting`; keep it (spinner off while an item waits grey during the error stop is correct) — P5c wires accordingly. (2) A late codex `Delivered` after turn end sets `turnDone=false` → **P2/P5 must not leave the wait indicator on**: when the bubble is placed while the channel is Idle, P2 should not re-open "turn"; check in P2 QA. (3) **P2b must put the codex `clientId` into the user echo's `uuid`** (today `codex/decoder.rs` `user_message_event` uses codex's item id) — else double bubbles. (4) Two adjacent separators after a Rejected removal — accepted (cosmetic, rare). (5) AckUnavailable draws only carried copies — fine.

## P1c (WIP) — turn table + kernel
```rust
// engram_dashboard_agent::turn
pub enum TurnSignal { Progress, Failed, Ended(TurnEndKind) }        // Copy, Eq
pub enum TurnEndKind { Clean, Failed, Other }                       // Copy, Eq
pub struct TurnObservation { pub in_turn: bool, pub last_signal: Instant, pub last_end_failed: bool }
// fold (after P1 review fix dbe092e): halt has its OWN cursor — Progress applied only if seq >= last_seq; Failed dropped only if seq < last_end_seq, else error_seq = min(existing, seq); Ended(k) dropped only if seq < last_end_seq, else held = error_seq < seq → last_end_failed = true if k==Failed||held, false if Clean, keep if Other; held error cleared; last_end_seq = seq; in_turn=false only if newest
// engram_dashboard_agent::inputs_pending
pub struct InputsPendingTable; // Default
impl InputsPendingTable { pub fn new(); pub fn register(&self, id: AgentId, epoch: u32); pub fn set(&self, id, epoch, seq: u64, pending: bool); pub fn get(&self, id, epoch) -> Option<bool>; pub fn forget(&self, id, epoch); }
// set drops: no entry | epoch mismatch | seq < last_seq (equal accepted) — core tests must register first
// AgentManager: pub fn inputs_pending(&self) -> Arc<InputsPendingTable>;  (registered next to turns.register in spawn_session; forgotten in finish since P1d)
// StatusSink: fn inputs_drained(&self, _id: AgentId, _epoch: u32) {} — call AFTER writing "empty"; MessagingFlushSink = idle.enqueue + forward
// engram_dashboard_messaging::busy
pub struct TurnFact { pub in_turn: bool, pub last_signal: Instant, pub inputs_pending: bool, pub last_end_failed: bool }
// is_busy: inputs_pending || last_end_failed → busy before valve; 30-min valve on in_turn only
impl ScriptedTurnFacts { pub fn set_inputs_pending(&self, id, epoch, pending, at); pub fn set_last_end_failed(&self, id, epoch, failed, at); }
```
- Classifier mapping now (behaviour-neutral): claude `MessageDone`→`Ended(Clean)`, `Error`→None (P3a flips to `Failed`); codex `TurnEnd{Completed}`→`Clean`, `Failed|Interrupted|Unknown`→`Other` (P2f flips `Failed`→`Ended(Failed)`), `MessageDone`→`Other`. `TODO(ADR-0231)` marks the arms to flip. Stale doc lines to fix then: codex classifier doc (「Ended with no Progress writes a fresh-registration state」), claude 「Error is not a turn boundary」.
- Adapter `ManagerTurnFacts`: pending table read THEN turn table; pending-only → fact with `last_signal: Instant::now()` placeholder.
- Line endings: check with `git ls-files --eol <path>` before scripted edits (busy.rs was CRLF in the tree).

## P1d (WIP) — core wiring
```rust
pub struct QueuedWiring { pub registry: Arc<QueuedInputs>, pub pending: Arc<InputsPendingTable> }
impl OutputCore { pub fn with_queued(self, wiring: QueuedWiring) -> Self; pub fn queued_inputs(&self) -> &Arc<QueuedInputs>; }
impl AgentSession { pub fn queued_inputs(&self) -> &Arc<QueuedInputs>; } // delegates to the core — NO session builder (one Arc by construction)
```
- Emit path: `QueuedInput` events go through `emit_list_event`/`record_list_event` inside the replay lock (seal → ack rewrite → copy-fill → reduce → "filled"); "empty" + `inputs_drained` written after turn observation, no lock held. `QueuedInput` must NOT enter via `seed` or `emit_batch_without_turn_observation` (debug_assert).
- Lock order: replay → registry → pending table (leaf); StatusSink calls with no core lock.
- `finish`: synthesize `Dropped{AgentEnded}` for open rows + seal in one replay section → fanout → terminal status. Since dbe092e finish does NOT write "empty" nor ring `inputs_drained` (entry forgotten right after; parked mail flushes on the next incarnation). Pending entry forgotten with the turn entry.
- Without `with_queued`: private registry, same ring shaping, no pending writes / no `inputs_drained`.
- Docs owed (P6): CLAUDE.md 「소유권 분할」 (queued registry, seal, rewrite — TRD L386) · ADR-0006 one line (replay → registry order — TRD L380) · TRD L378 wording (core owns the Arc; session reaches it via the core).

## P1 review (deep, 2026-09-26) — carried items (MUST be honoured by later chunks)
- **P5b**: live delivery can reorder between emitters (incl. finish synthesis) → the client MUST hold `seq > last+1` and release contiguously instead of dropping (codex P1 finding — the design's seq-continuity rule; verify with a test that a later-seq frame arriving first does not cause the earlier one to be discarded).
- **P3a**: a claude line-overflow `Error` outside a turn must NOT be emitted as `TurnSignal::Failed` (it would sit in `error_seq` (else the next clean turn folds as failed → halt with no ceiling). Map only the failed-`result` `Error` (the one emitted just before `MessageDone`) to `TurnSignal::Failed`.
- **P3b**: take `input_order` around classify → emit → send on the SessionClassified write path (rule ② cancel line only after the write returns); optionally have the emit path report "applied" so a cancel racing the pump answers `NotFound` instead of `Requested`.
- **P6 docs owed**: TRD §5-0 「같은 락 · 같은 seq 규칙」 for `last_end_failed` → the halt has its own cursor (`last_end_seq`/`error_seq`); finish does not ring `inputs_drained` (TRD L245); CLAUDE.md lock order `input_order → replay → registry → pending`; ownership split (mid_turn, delivery_ack, input_order, queued registry via core).

## P4a (WIP cd03b14) — agent commands
```rust
// engram_dashboard_agent::queued_input
pub enum ListedState { Queued, Unconfirmed, Cancelling { answer: CancelAnswer, vendor_closed: bool } } // as_str: "queued"|"unconfirmed"|"cancelling"
pub struct ListedRow { pub id: String, pub text: String, pub state: ListedState }
pub struct QueuedListing { pub rows: Vec<ListedRow>, pub as_of_seq: Option<u64>, pub epoch: u32, pub stopped_after_error: bool }
// CancelAnswer::as_str "none"|"not_removed" · CancelOutcome::as_str "requested"|"cancelled"
// AgentTransport: fn unconfirmed_inputs(&self) -> Vec<String> { Vec::new() }
// AgentSession::list_queued_inputs(&self) -> (Vec<ListedRow>, Option<u64>)
// AgentManager — USE THESE for the WS arms in P4b:
pub fn list_queued_inputs(&self, agent_id: AgentId) -> Result<QueuedListing, PtyError>;   // Err(NotFound) = no live session
pub fn cancel_queued_input(&self, agent_id: AgentId, input_id: &str) -> Result<CancelOutcome, CancelError>;
// commands.rs: pub const INPUT_AFFECTING: &[&str] = &["agent.cancelQueuedInput"]; catalog_version 5
// declared types: QueuedInputCancel{answer,vendor_closed}, QueuedInputRow{id,text,state,cancel:Option<..>}, AgentListQueuedInputsArgs{target}, AgentListQueuedInputsOk{inputs,as_of_seq:Option<u64>,epoch:u32,stopped_after_error:bool}, AgentCancelQueuedInputArgs{target,input_id}, AgentCancelQueuedInputOk{outcome:String}
```
- Errors advertised = `[NOT_FOUND, CONFLICT]` for both verbs (resolver ambiguity → CONFLICT; P4c lease denial reuses CONFLICT) — orchestrator accepts; TRD L560/L748 owe an amendment (P6). Cancel `Write(e)` → `INTERNAL` by value. List on a sleeping agent → `NOT_FOUND`.
- Agent TS binding `as_of_seq` is `bigint | null` (ts-rs u64) — P5c must treat it as bigint or read the WS reply type instead.
- Daemon tests pinning agent command names: `control/commands.rs` (table = CLI verbs + named bus-only list), `connection_core.rs` (CommandList names), `tests/control_agent.rs`.

## P4b (WIP) — WS wire, PROTOCOL_VERSION 6
```rust
AgentCommand::ListQueuedInputs { agent_id, request_id }
AgentCommand::CancelQueuedInput { agent_id, input_id: String, request_id }
AgentEvent::QueuedInputs { request_id, agent_id, inputs: Vec<QueuedInputRow>, as_of_seq: Option<u64> /* TS number|null */, epoch: u32, stopped_after_error: bool }
AgentEvent::QueuedInputCancelReply { request_id, agent_id, input_id: String, outcome: String /* requested|cancelled */ }
pub struct QueuedInputRow { id, text, state: String /* queued|unconfirmed|cancelling */, cancel: Option<QueuedInputCancel> }
pub struct QueuedInputCancel { answer: String /* none|not_removed */, vendor_closed: bool }
// errors: Error{request_id: Some} with message "NOT_FOUND: …" / "INTERNAL: …"; lease refusal = INPUT_LOCKED_REFUSAL (same text as WriteStdin)
```
- **P5c MUST add `handleEvent` branches in `src/api/protocolClient.ts` for `QueuedInputs` and `QueuedInputCancelReply`** in the same change that sends these commands (else promises never resolve).
- **P3b**: the WS cancel arm runs on the async worker without `spawn_blocking` (like WriteStdin); once claude cancel writes a line under `input_order`, check the transport write cannot block the worker.
- Blank `input_id`: WS → NOT_FOUND vs bus → INVALID_ARGUMENT (accepted asymmetry, minor).

## P4 review (deep, 2026-09-26) — carried to P5c
- WS lease refusal is the bare `INPUT_LOCKED_REFUSAL` text (no `CODE:` prefix, parity with WriteStdin); NOT_FOUND/INTERNAL carry `CODE: `. P5c must map the bare refusal to CONFLICT when a front command result reaches the LLM, and must not rely on the prefix for it.
- The front `agent.cancelQueuedInput` registration must stay WITHOUT `help` — with help the shell's `RegisterCommands` would include a daemon-answered name and the clash check refuses the WHOLE packet (every front command drops off the bus).
- Words are strings on both surfaces — P5c must tolerate unknown words.
- WS reply renamed (P4 fix): `AgentEvent::QueuedInputCancelled` → `AgentEvent::QueuedInputCancelReply` (outcome may be "requested").

## P5b (uncommitted — MUST land with P5a in one phase commit)
- `InboundMessage.replayBoundary` gains required `replayFrom: number`. `decodeReplayMarker(buf)` → `{agentId, epoch, gen: bigint, truncated, failed, continuesConversation, replayFrom: number} | null`; marker 38 bytes `[255][agent16][epoch u32 BE][gen u64 BE][flags1][replay_from u64 BE]` — shorter markers (incl. old 30-byte) are REJECTED. `ViewOutputState` gains `held: number` (frames held behind a gap — LLM-visible diagnostic).
- Live holds `seq > last+1` (no timeout), releases contiguously; flush from `max(last+1, replayFrom)`; held overflow (4 MiB/8192) → `startBuffering` (fires `onState('buffering')`), never ladder in live.
- **P5a hard requirements**: shell must emit the 38-byte marker; `output_core.rs` empty-ring `replay_from` (~:579/:604) must be the next seq (today it reads latest+1 = 1 and would drop a new agent's seq 0 under P5b).
- For P5c: reconciliation generation bump can key off `onState('buffering')`.
- Docs owed (P6): CLAUDE.md 「통합 micro-rules」 add hold/contiguity; TRD §5-7 marker layout now pinned (38 B); line anchors in TRD for protocolClient.ts shifted.

## P4 re-review (API lens PASS) — owed
- **P6**: catalog text for `queued` overclaims 「넘겼고 대기로 확인됐다」 → 「넘겼고 아직 결말 전(받혔는지 모르게 된 것은 `unconfirmed`)」 (`agent/src/commands.rs` ~:189-190 + regenerate `commands.schema.json`). Optional: connectionless lease tail wording.

## P5a (WIP 232724b) — Rust relay (tree consistent with P5b again)
```rust
// engram_dashboard_protocol
pub struct FrameHeader { pub tag: u8, pub agent_id: AgentId, pub epoch: u32, pub seq: u64 }
pub fn peek_frame_header(buf: &[u8]) -> Result<FrameHeader, CodecError>; // only TooShort
pub const PLACEHOLDER_ERROR_MESSAGE: &str; // "이 출력 사건을 싣지 못했습니다"
pub fn placeholder_error_frame(agent_id: AgentId, epoch: u32, seq: u64) -> Vec<u8>;
// shell: Marker { generation, truncated, failed, continues_conversation, replay_from: u64 }; MARKER_FRAME_LEN = 38
// ReplayFlightSet::on_ack(agent, truncated, continues_conversation, replay_from, now)
// daemon_client::frame_relay::relay_binary_frame(...) -> FrameRelay { Continue, Disconnect }
```
- Daemon drop paths now send a placeholder at the same seq; shell `UnknownTag` → placeholder, `TooShort` → disconnect. Empty ring `replay_from` = next seq. Poisoned registry lock → `into_inner` (3 sites).
- Stale TRD anchors for P6: `connection.rs:1036-1049` → `daemon_client/frame_relay.rs`; `output_core.rs:579/604` → ~:830-870; `agent_conn.rs:96/109` → `structured_payload()` + `send_placeholder()`; TRD L584 marker now pinned 38 B.
- Tooling trap: never wrap `scripts/run-detached.ps1` in a script that discards its `PID=`/`LOG=` stdout — the log may never be created and the `__EXIT` wait loops forever; call the ps1 directly with `-Command "<literal>"`.

## P4 local QA (96ead26) — result
- Real-claude tests 6/6 PASS · ADR-0130 trigger: 2 NEW production lines (`use crate::connection_core::INPUT_LOCKED_REFUSAL` in `control/agent.rs` and `control/commands.rs` — existing control→connection_core module edge, new files) → alert for the user; simplest removal = move the const to a neutral module · GUI smoke PASS (claude/codex/terminal unchanged) · CLI PARTIAL (`engram.exe` needs an agent token by design; bus verified via the WS `Command` entrance instead).

## P5c (WIP d03ba0c) — front list UI + reattach reconciliation
```ts
// queuedInputReducer.ts
export function entryOfListedRow(row: QueuedInputRow): QueuedEntry | null   // queued|unconfirmed→queued; cancelling needs cancel{…}; unknown words → null
QueuedInputRegistry.adoptSnapshot(rows: readonly QueuedEntry[], later: readonly QueuedInputEvent[], listed: ReadonlySet<string>): void
// structuredAccumulator.ts
feed(payload, seq?) · beginQueuedReconcile(epoch): number · offerQueuedSnapshot(gen, {inputs, as_of_seq, epoch}): 'applied'|'held'|'stale' · abandonQueuedReconcile(gen?) · observeSeq(seq): boolean
// agentClient.ts
AgentClient.listQueuedInputs(agentId): Promise<QueuedInputListing> · cancelQueuedInput(agentId, inputId): Promise<string> · ReplayLiveInfo.epoch?: number
export const INPUT_LOCKED_REFUSAL  // moved here from protocolClient (review fix); byte-parity test vs connection_core.rs
// front command: run('agent.cancelQueuedInput', { agentId, inputId }) → { outcome } — NO help; bare refusal → 'CONFLICT: …'
// QueuedInputList props: { agentId; entries; collapsedCount?=3; collapsedSide?='oldest'; defaultExpanded?=false } · list max-h-40 scroll
```
- Seq tracking lives in the accumulator (`deliveredSeq` over all frames), not the reducer — golden untouched. RichSlot: on `'live'`+epoch → begin + query; any non-live state / reset → abandon.
- Every json RichSlot sends one `ListQueuedInputs` per `'live'` (NOT_FOUND on a sleeping agent swallowed).

## P5 review (deep, 2026-09-26) — FIX → fixed (59337cb shell, b8ed2a4 front, + 2 inline lines) → re-verify PASS ×3
- **Ghost rows (all three reviewers):** `adoptSnapshot` now closes open rows absent from ALL raw snapshot ids with no logged `Queued{id}` seq > S → resurrectable tombstone, no bubble. ★Deviates from TRD L627 (「스냅숏에 없는 id = 누산기 상태 그대로」 — premise false after a head jump)★ → P6: amend TRD L627 + L750 test list.
- **Replay before the window's output Channel is registered (concurrency F1, pre-existing race that P5b turned into a permanent blank view):** `TauriTransport.requestReplay` awaits `awaitOutputChannel()`; `doConnect` runs `selfHeal()` + awaits registration when `daemon_ensure` returns already-Connected without an emit; `selfHeal` generation guard (close() during pull no longer resurrects). Defence (b) (replay end in the marker → ladder on a missing seq) NOT done — reviewer: acceptable, cheap insurance if a drop path reappears.
- **Held caps** 2048 frames / 1 MiB (below ring 4096 / 2 MiB); buffer caps unchanged. ★TRD L597 (held bounded by the 4 MiB · 8192 buffer caps) now stale★ → P6. Flush may still move up to the buffer cap into held (not cap-checked until the next hold) — accepted.
- WS path: SubscribeAck `replay_from` guarded; unknown tag → placeholder at same seq (`PLACEHOLDER_ERROR_MESSAGE` mirrored in `wsFrame.ts`, parity test); short frame → warn + close.
- Shell: poison recovery warns once + `clear_poison()` (`lock_registry` helper); unknown tag warns once per (agent, tag) per connection (`UnknownTagLog`, owned by `connection.rs::main_loop`; `relay_binary_frame` gained `unknown_tags: &mut UnknownTagLog`).
- ★`delete channel.onmessage` is a no-op on the real `@tauri-apps/api` Channel (prototype accessor)★ — old Channel keeps delivering until Rust replaces it, and frames across re-registration rely on that; comment added at `tauriTransport.ts` `doRegisterOutputChannel`. P6: CLAUDE.md 「통합 micro-rules」 `delete channel.onmessage` rationale is overstated (FakeChannel in tests hides it).
- Advisories not acted on: flush-time held-cap check (A2); pre-existing concurrent `registerListeners` from `init()` + first `doConnect` can leak a listener set (A3 — a leaked 'connected' listener could resurrect a closed transport); N1 flush discards contiguous frames below a later-generation head (data loss only, rare).
- P6 docs owed (add): TRD L597/L627/L750; CLAUDE.md 「통합 micro-rules」 (hold/contiguity, reconciliation: live → list query → apply after S → generation; replay waits for Channel registration); contacts P5b "held overflow (4 MiB/8192)" line is stale.

## P3a (WIP a4e17af) — claude decoder/classifier
```rust
// backend/claude/mod.rs
fn cancel_line(id: &str) -> Vec<u8>   // private, ~:686, `#[cfg_attr(not(test), allow(dead_code))]` — P3b sets MidTurnPolicy::SessionClassified { cancel_line } in the JSON branch and deletes the attr
// bytes = {"type":"control_request","request_id":"cancel:<id>","request":{"subtype":"cancel_async_message","message_uuid":"<id>"}}\n
pub fn ClaudeStreamDecoder::with_delivery_ack(ack: Arc<DeliveryAck>) -> Self   // open_spawn makes ONE Arc: decoder clone + SpawnParts.delivery_ack; terminal branch stays Unknown
const RESULT_FAILURE_DETAIL = "claude stream-json result reported failure"      // only an Error starting with this → TurnSignal::Failed (line-overflow Error → None)
```
- consume_line(line, events, LineSource::{Live(&DeliveryAck), Transcript}) — lifecycle `started→Delivered`, `cancelled→Dropped{Unknown}`, `discarded→Dropped{AgentEnded}`, `refused→Dropped{Rejected}`, `queued/completed`→nothing; `control_response` `cancel:*` → `CancelAnswered{removed}` / `CancelFailed`; init with `msg_lifecycle_v1` → Available, without → CAS winner emits one `AckUnavailable{[]}`; Transcript: `attachment{queued_command, commandMode:"prompt"}` → user bubble (uuid `source_uuid`), `queue-operation` skipped.
- ★CLI 2.1.280 emits `command_lifecycle` for EVERY uuid-bearing input (idle Direct and mail too)★ → from P3a on, production emits `Delivered`/`Dropped` for ordinary inputs (registry: unknown id → tombstone; front: already-drawn echo → no-op; `refused` → drawn bubble removed). Old CLI (no capability) → one `AckUnavailable{[]}` per incarnation.
- Classifier: `TurnEnd{Failed}` → `Ended(Failed)`; `interrupted` result still → MessageDone → Clean (clears halt — TRD L413 leaves it to the interrupt feature).
- P6 owed: TRD L744 (overflow Error ≠ turn error — carried item wins); TRD L414 (`commandMode:"prompt"` only); `types.rs` `OutputEvent::Error` doc (claude failed turns = Error + MessageDone, not TurnEnd).

## P5 gate — DONE (af9ebe3): review deep PASS after fixes · CI green (run 36208419586) · local QA full PASS
- Real-claude 6/6 (with `ENGRAM_TEST_REQUIRE_CLAUDE=1`, no skips) · ADR-0130: no new matches (P4's 2 lines remain) · GUI isolated release instance: claude/codex JSON chat, terminal, reload ×3, popout ×3 (+ popout reloads) → live, held 0, no queued list. Forced shell↔daemon reconnect NOT done (no non-destructive procedure) — reloads cover re-attach.
- Suspicious, likely pre-existing (not checked against 96ead26) → for the user: (1) terminal replay garbled when its tab is hidden at reload (0-width hidden xterm?); (2) codex chat shows claude branding right after `agent.spawnInto` until reload.
- QA tooling: `getViewOutputState` only via `window.__ENGRAM_AGENT__` (not in `__engramCmd`); isolated worktree teardown needs robocopy `/MIR` (long paths). A debug shell would load wt2's vite on 1420 → QA used a release shell.

## P2a (WIP) — codex transport pending model (all private to `backend/codex/transport.rs`)
```rust
struct State { …, pending: VecDeque<PendingItem>, next_arrival: u64, … }
impl State { fn push_pending(&mut self, id: Option<String>, body: Vec<u8>, origin: InputOrigin); } // pushes Held, listed=false, announced=true (★P2b: parameterise for classified items★), cancel_asked=false, death=None
struct PendingItem { id: Option<String> /* None = legacy send_input: no withdraw, no echo match, never listed */, body: Vec<u8>, origin: InputOrigin, stage: Stage, listed: bool, announced: bool, arrival: u64 /* monotonic over ALL items */, cancel_asked: bool, death: Option<DropCause> }
enum Stage { Held, InFlight { gen: u64, turn_id: Option<String> /* None = turn/start reply not yet */, awaiting_reply: bool }, Unconfirmed }
fn accept_input(state: &SharedState, id: Option<String>, body: Vec<u8>, origin: InputOrigin) -> Result<(), PtyError>; // closed → Down → limit checks, then push + notify_all
fn send_turn(&self, turn: TurnInput) -> Result<(), PtyError>; // codex override = accept_input(Some(id), …)
```
- `take_turn_locked` still pops the head at take (P2a holds only `Held`). `#[allow(dead_code)]` on `PendingItem`/`Stage` — remove when read. No per-state `gen` counter yet (P2b adds it with its first reader).
- ★P2b must keep id-less (`None`) items out of settlement★ (pop at take, no `clientUserMessageId`) — else they sit `Unconfirmed` forever and block mail; legacy `turn/start` bytes stay identical.

## P3b (WIP 62d6081) — claude session classification + cancel
```rust
// output_core.rs (crate-internal)
pub(crate) enum CancelRequest { Recorded, AlreadyCancelling, NotListed }
impl OutputCore { pub(crate) fn emit_cancel_request(&self, id: &str) -> CancelRequest; /* registry check + CancelRequested in ONE replay section */ pub(crate) fn classified_input_busy(&self) -> bool; /* pending table then turn table, never the registry */ }
// session.rs: fn write_user_classified(&self, bytes) — input_order held through classify → emit → send; Available: Queued→send (fail → Dropped{Rejected}); Unknown: send→Queued (fail → nothing); Unavailable/idle → write_now (today); Mail → no lock
// claude: fn mid_turn_policy(command) -> MidTurnPolicy  (JSON → SessionClassified{cancel_line}, terminal → None)
```
- Lock order live: `input_order → replay → registry → pending`; pump/emit/finish never take `input_order`. ★Fanout (sink sends) now runs while holding `input_order` on the session path★ — an ADR-0006 exception to name in P6 (CLAUDE.md 「emit은 … lock 미보유 send」 refers to the subscribers lock).
- Cancel vs pump `Delivered`: `Delivered` first → NotFound, no line; after → Requested, line written, row closes as Delivered.

## P3 review (deep, 2026-09-26) — codex PASS · concurrency lens PASS · doc-aware FIX (minor) → fixes pending
- Adopted (orchestrator, delegated; TRD deviation → final report): ack Available only on a lifecycle line with a non-empty `command_uuid` AND a known state word (vendor drift → stays Unknown → init without v1 → Unavailable → today's path). TRD §5-4 「command_lifecycle 줄을 처음 보면 Available」 → P6 amend.
- Fix items: latch regression tests on the SessionClassified path (Available/Unknown/UserKill); `info!` on the first pump `AckUnavailable` reduction (agent, epoch); named reader fns for the busy read order + non-vacuous cancel-order test.
- Residual accepted → P6 TRD §5-9: kernel check-then-write gap lets mail released just before a user `Queued` hit stdin before that user item (pre-existing class). WS cancel arm parks a tokio worker on `input_order` (bounded; accepted). `Unknown` has no fallback if a CLI sends neither init nor lifecycle (TRD-accepted, M4).
- P6 owed (add): TRD L255 (overflow Error ≠ turn error; the prefix rule), L230 (cancel check+record atomic), L234/CLAUDE.md lock order incl. `input_order`, ADR-0006 exception above.
- Re-verify: codex PASS · doc-aware PASS. Advisories → P6: TRD L742 test list (「첫 command_lifecycle → Available」) also amend; §5-9 residual — the recognisable-line gate only helps when drift comes with an init lacking `msg_lifecycle_v1` (capability kept + renamed keys → rows never close, no ceiling); optional test pinning `LIFECYCLE_STATES` ≡ `lifecycle_event` words.

## P2b1 (WIP b8dd06b) — codex floor verdict, announce, clientUserMessageId, echo matching, withdraw(Held)
```rust
// protocol.rs
pub(crate) struct TurnStartParams { thread_id, input, #[serde(skip_serializing_if="Option::is_none")] client_user_message_id: Option<String> }
// decoder.rs
pub(super) fn user_bubble(text: &str, uuid: Option<&str>) -> OutputEvent
// transport.rs (private)
const CLIENT_MESSAGE_ID_FLOOR = (0,140,0); enum Floor { Pending, Above, Below } /* State.floor */
struct Announcer { order: Mutex<()>, ack: Arc<DeliveryAck> }   // lock order: order → state
fn announce(core, state, &Announcer); fn settle_floor(core, state, &Announcer, Floor); fn floor_verdict(cli_version, user_agent) -> Floor;
fn withdraw_item(state, core: Option<&OutputCore>, id) -> Withdraw;
impl State { fn push_pending(..) /* classifies: Mail|no id → none; User+Pending → undecided; User+Ready∧Idle∧empty → Direct(listed=false); else Queued(listed=true) */; fn take_delivered(&mut self, id) -> Option<PendingItem>; fn turn_start_carrier(&mut self, gen) -> Option<&mut PendingItem>; fn drop_listed(&mut self, pos, cause) -> Option<QueuedInputEvent>; }
impl CodexAppServerTransport { core: OnceLock<Arc<OutputCore>>; #[allow(dead_code)] pub(crate) fn delivery_ack(&self) -> Arc<DeliveryAck> /* P2c: put into SpawnParts.delivery_ack in codex open_spawn */ }
```
- Verdict source: `thread/start|resume` `cliVersion` first, then `initialize.userAgent`; applied in `writer_loop` after `record_session_id`, before `hydrate_history`/`open_gate`. Below → every item loses its id (today's path, no events, no `AckUnavailable`).
- `gen` = the turn seq (`TurnState::Active.seq`) — no separate counter (deviates from TRD L459 wording). Carrier = `InFlight{gen==seq, turn_id: None, awaiting_reply: true}`; on reply → `turn_id=Some, awaiting_reply=false`.
- Echo: `clientId` (≤128 B) → bubble uuid + `Delivered{clientId}` right before it (item/started|completed only; history door uses clientId as uuid, no Delivered). Reader `note_delivered` removes the id whatever the stage (TRD L494 "Delivered wins"). Delivered once per item (TRD L356 says twice → P6).
- withdraw: listed Held announced → remove + `Dropped{Withdrawn}`; listed Held unannounced → death mark (announcer emits Queued then Dropped); Direct/Mail/pre-verdict/unknown → NotHeld; InFlight/Unconfirmed → NotHeld + TODO (P2e).
- ★Until P2b2★: an id-bearing InFlight item without an echo never leaves pending (blocks Direct, counts to the limit). Mail items with ids (above floor) also become InFlight (TRD L481).
- P6 owed: TRD L356, L459; `codex/mod.rs` `output_decoder` dedup-key comment stale after P2c.

## P2b2 (WIP) — codex turn-end disposition v1
```rust
enum TurnClose { Completed, Failed, Interrupted, Rejected, Unanswered }   // of_completion(params) reads turn.status: interrupted | failed | else Completed
struct EarlyCompletion { turn_id, boundary: Option<OutputEvent>, close: TurnClose }
impl State { fn close_turn_items(&mut self, gen: u64, close: TurnClose) -> Vec<QueuedInputEvent> }  // ★guards + `_` arms — a new TurnClose variant compiles silently; P2e2/P2f must revisit★
fn end_turn_if(state, core, seq, detail, close: TurnClose) -> bool; fn turn_write_failed(state, pending, core, id, seq, e)
```
- Table: listed-in-turn: Interrupted → Dropped{Interrupted} · Rejected → Dropped{Rejected} · Completed/Failed/Unanswered → Unconfirmed (no event). Direct-in-turn: Rejected → Dropped{Rejected}, else silent removal. Mail-in-turn: silent removal. Listed Held: Interrupted → Dropped{Interrupted}, else kept. Mail Held / id-less / older Unconfirmed / death-marked: kept.
- Disposition events emitted outside the lock and BEFORE the turn boundary. Stream close → core `finish` synthesis only.
- TRD reading: L486 = steer error replies only (turn/start error → Rejected, L504); Mail Held on interrupt kept (L497/L688 wording vs L481 — dropping would lose mail).
- ★Window after P2c and before P2e2★: an Unconfirmed item has no ✕ exit (withdraw → NotHeld) and keeps `inputs_pending` true → mail blocked until late echo / agent end (anomaly-only). Not gated mid-phase, so acceptable; P2e2 closes it.

## P3 gate (64fe0bd): review deep PASS after fixes · CI green (run 36212104534) · local QA full = PARTIAL (every run check PASS)
- GUI claude trials 1, 2, 4, 8, 10, 11, 13 (reload/popout/shell restart/forced daemon disconnect), 14 (LLM surface incl. real `engram agent.*Queued*` from another agent), 15 (themes), 16 (mail waits for the list), AC29 halt (max-turns failure → `stopped_after_error` true → mail parked → user turn RECOVER → mail flows) — all PASS; held 0; no flicker/dup/stuck rows.
- NOT RUN: trial 3 (✕ losing race — window inside the daemon; §7-1 fixtures stand in) · trial 9 part 2 (not reproducible on claude per TRD) · trial 16 terminal-slot mail.
- Real-claude tests: 6/6 on a clean rerun; ★`c2_live_mid_turn_send_parks_and_delivers_after_turn_end` failed 1/3 under build load★ (first send to an idle recipient → `pending` with the overlapping-drain hint; guess: collides with the first-appearance flush) — flaky, cause unverified → final report / P6 QA re-check. `c1_park_then_spawn_auto_delivers` self-SKIPs its main axis every run (pre-existing, P5 too).
- Defects found → follow-up chunk (in progress): front `agent.cancelQueuedInput` CONFLICT mapping needs `instanceof Error` but the production carrier rejects with a string (`agentCommands.ts:149`); mail hint says "mid-turn" for a halted-after-error / list-waiting recipient (`messaging/src/service.rs:3180`).
- For the user (pre-existing/likely): daemon WARN "structured event dropped from wire snapshot (B7 미배선)" now also for `kind="QueuedInput"` (per event?); codex branding after spawnInto; wait timer resets on reattach / ticks after kill (→ next-task memo). Unexpected Korean inputs "아이우에오"/"ㅇ" appeared in the visible QA window — most likely a person at the window; behaviour correct.

## QA fix (pushed 160ba1e on top of 64fe0bd) — review light PASS after wording fix
- Remote tip = `160ba1e`. Local branch = `64fe0bd` → WIP P2a `aa9441b` → P2b1 `b8dd06b` → P2b2 `e09d549` → QA-fix WIPs `783dede`,`03600c0` (same content as 160ba1e) → (P2c…). ★P2 gate squash = `git reset --soft 160ba1e`★ (index keeps the local tree; diff vs 160ba1e = P2 only — verify with `git diff --stat 160ba1e HEAD` first). Never force-push.
- Local QA of the fix itself (CONFLICT mapping under a real lease conflict; halted-recipient mail hint text) → fold into the P2 QA run.

## P2c (WIP) — codex flip to TransportOwned
- codex app-server `open_spawn`: `mid_turn: TransportOwned`, `delivery_ack: t.delivery_ack()`; session `write_now` emits the echo only when `turn_origin.is_none()` (no session echo on TransportOwned). TurnInput id = the write's `Uuid::new_v4()` (= `WriteOutcome.msg_uuid`). No `input_order` on this path (TRD L226/L238). Mail → `send_turn(origin: Mail)`.
- RichSlot: `historyPending = continuesConversation && !hasHistoryRow` (TRD L621).
- Ruling (2): front NOT changed — `placeUserBubble` sets `turnDone=false` (TRD L602); a codex `Delivered` is the head of the next turn, whose boundary follows. Residual: turn/start `Unanswered` → vendor starts late → unattributable `turn/completed` → "Wait" stays (pre-existing class). ★P2e2/P2f owe: a late echo placed while settling with no debt / halted must be followed by a boundary or not placed★.
- ★Open concern for P2 review (ADR-0226)★: `count_turn_submission` runs in `write_now` before `send_turn`; on a fresh codex agent the first input typed before `Link::Ready` becomes Queued → if ✕-withdrawn, the latch already counted → a zero-turn thread id may be persisted (can codex resume a zero-turn thread? unverified). Before the flip nothing was withdrawable.
- Below-floor codex continuing agent: loading panel now stays after send until the vendor echo (no producer events below floor) — accepted, report.

## ★User decision 2026-09-26 — codex session id persists at the FIRST USER-MESSAGE ECHO (ADR-0226 amendment)★
- Quotes: 「세션 id가 저장되는건 우리가 보내는 시점이 아니고 오는 시점이잖아」 → 「제출하는게 아니고 코덱스가 뱉는거 기준으로 저장하라고 최초 뱉을때가 완전한 타이밍인거잖아. 미세하게 중간에 종료되서 저장안되는건 그냥 초기화 하라고하고」 → (asked A = first echo vs B = thread/start reply) → 「일단 실제 아이디가 생성된 기준이어야지 당연히. 그게 사용자 글을 에코할때잖아.」
- Decision: codex JSON (TransportOwned) persists the thread id when codex first echoes a user message in the live stream (the conversation really has a turn), NOT when the session counts our submission. Dies between send and echo → nothing persisted → next activation starts fresh (accepted: 「그냥 초기화」).
- Rejected: counting at session receive (today — a pre-Ready first input that is ✕-withdrawn persists a 0-turn id); persisting at `thread/start` reply (B — 0-turn ids for agents that never send / withdraw).
- Scope: codex JSON only. claude unchanged (its first Queued input implies a prior counted turn). codex terminal unchanged (ADR-0226 decision 11).
- Implement as chunk "P2c-fix" right after P2d (same files). P6: record via `/adr` (amendment of ADR-0226 — new number if it reverses a decision; stamp the old one).

## P2d (WIP) — codex segment tracking, policy seam, M15 trace
- decoder: `ItemClass{Tool,Output,Other}`, `item_class()`, `ends_answer()` (agentMessage|plan; reasoning excluded).
- transport: `HandOverPolicy{Immediate,AtEarliestBoundary}` (mod.rs `const HAND_OVER_POLICY = Immediate`, seam `with_hand_over`), `HandOver{Now,Hold}`, `Zone{Blocked,BoundaryOpen,Tool,Answer,Other}`, `verdict(zone, answer_holds)`, `const ANSWER_SEGMENT_HOLDS = true`; `State.{policy, segment, unseen_signal, next_signal_seq}`, `zone_of(pos)`, `hand_over(pos)` (dead until P2e1); `Signal{TurnId,ToolEnd,AnswerEnd,TokenUsage}` → `raise()` + notify; `Job::Woke(Wake)` (writer only traces it — ★P2e1 folds the steer decision into this wake; the "steer during output flood" hop is woken by announce, P2e1 takes its own wake Instant★). Reader entry points `handle_line_at` / `resolve_at`.
- `zone_of` TODOs for settlement / halt / steer-rejected (P2e1/P2e2/P2f).
- M15 trace: target `engram::codex_handover` debug; `phase="signal"` {signal, seq, lag_us, handle_us, recv_ms, emitted_ms} and `phase="wake"` {signal, seq, coalesced, lag_us, wake_us}; pair by seq; leg ② `phase="steer"` = P2e1 TODO. Contract doc on `HANDOVER_TRACE`.

## P2c-fix (WIP) — codex JSON thread id persists at the first user-message echo
- `pub type FirstTurnSink = Arc<dyn Fn() + Send + Sync>` (`crate::backend`); `SessionIdLatch::first_turn_sink()` = port calling the existing `note_submission()` (latch state machine unchanged); `AgentBackend::open_spawn(.., sid_sink, first_turn_sink, resume_session_id, ..)` gained the param (trait, 3 impls, dispatch fn, manager wiring next to `offer_sink`); codex app-server branch `.with_first_turn(sink)` paired with `TransportOwned`; terminal drops it.
- codex `Reader::witness_first_turn` fires once (`take()`) on the first live `item/started|completed` with type `userMessage` (clientId or not; mail and below-floor turns count; foreign threadId skipped; history never). Called with no lock held, after the line's events + trace → the one profile save per incarnation now runs on the codex reader thread (bounded by the agents.json rewrite, ADR-0071).
- Session: `count_turn_submission` keeps the UserKill refusal, returns early for TransportOwned without `note_submission`.
- Resolves the P2c "open concern" (withdrawn pre-Ready first input no longer persists a 0-turn id).
- Residuals → P2 review / final report: vendor drift (no userMessage echo) → never persists, only the latch "Held" info log (optional warn when an attributed turn completes with the port unfired); commit-port panic on the reader thread (unwind → pump Failed; release aborts); kill-vs-late-commit window (same as record_session_id's).
- ★P6 owed★: CLAUDE.md 「핵심 불변식」 last bullet (codex JSON exception — suggested text in the coder report: 「★codex JSON(`TransportOwned`)은 세션이 세지 않고 통로가 상대의 첫 유저 메시지 되울림에서 래치의 첫 턴 포트(`FirstTurnSink`)를 부른다 — 첫 턴 뒤 영속이고, 보내고 되울림 전에 죽으면 영속이 없다(사용자 결정 2026-09-26 · ADR-0226 개정)★」); ADR-0226 decisions 2 and 11 + 「영향/불변식」 → `/adr` amendment (new ADR number, stamp 0226); check `docs/reference/backend-capabilities.md` for codex persistence timing.

## P2e1 (WIP d959322) — codex steer, gated off (474k tokens · 105 tools · 25 min — upper bound; P2e2 split into P2e2a/P2e2b)
```rust
// protocol.rs
pub(crate) const method::TURN_STEER = "turn/steer";
pub(crate) struct TurnSteerParams { thread_id, expected_turn_id, client_user_message_id: String /* steer only carries id-bearing items */, input }
// transport.rs (private)
const STEER_ENABLED: bool = false;   // sets State::steer in State::new(); P2e2b deletes const + field
struct State { …, steer: bool /*test seam; helper steering(h)*/, steer_refused: Option<u64> /*turn seq; zone_of → Blocked for that turn*/, woken_by: Option<(SignalMark, Instant)> }
enum Waiter { …, Steer { id: String, gen: u64 } }
impl State { fn steer_replied(&mut self, id, gen, reply: SteerReply) -> SteerMove }  // SteerReply{Accepted,Refused,Unanswered} · SteerMove{Stale, Moved(Option<QueuedInputEvent>)}
fn take_steer_locked(s: &mut State, next_id: &AtomicI64) -> Option<Steer>;  // candidate = oldest Held User item; Mail skipped (neither steers nor blocks); unannounced / Direct / Hold → wait (no skip); needs Active{turn_id: Some} ∧ Floor::Above ∧ steer
struct Steer { request, item, gen, line, hop: Hop }  enum Hop { Signal{mark, seen}, Announce{seen} }
struct Wake { mark, seen, steer: Option<Steer> }  enum Job { …, Steer(Steer) }  enum Issued { Written(Instant), Failed, Closed }
fn issue_steer(state, pending, core, steer, write) -> Issued;  fn steer_write_failed(..) /* → Dropped{Rejected}, turn continues */
enum Traced { Wake(Wake), Steer(Hop, Instant) }   // trace lines held back until the wake's steers are written
```
- Replies: Accepted/Unanswered → `awaiting_reply=false` only. Refused → item back to Held in place (no event) + `steer_refused=Some(gen)`; with `cancel_asked` → `Dropped{Withdrawn}`. Stale = any reply whose item is not `InFlight{gen, Some(_), awaiting_reply: true}`.
- `withdraw(InFlight)` → TooLate + `CancelRequested` + `CancelAnswered{removed:false}` (first ✕ only; outside lock). ★Live in production too★ (turn/start carrier window before echo) — no-echo case leaves a `Cancelling` row that blocks mail until P2e2a's `Dropped{Unknown}`.
- M15 leg ②: `phase="steer"` {hop: signal|announce, signal, seq, steer_us, hop_us}; pairs with `wake` by seq. `woken_by` cleared only when the writer sleeps (skew documented on `Hop`).
- TODOs: :528 (P2e2 settlement → Blocked; P2f halted) · :549 (P2e2b enable) · :1073 (close_turn_items: cancel_asked → Dropped{Unknown}; awaiting steer reply → wait for settlement) · :1112 (steer reply after turn end is Stale today) · :4001 (withdraw(Unconfirmed) → TooLate + 3 events + wake) · :1186 (P2f arrival).
- ★Gate must stay off until settlement★: turn completes between steer take and reply → item Unconfirmed, later −32600 Stale → stays grey though rejected.
- Not unit-tested: `writer_loop` wiring (needs ChildStdin). Stale anchors: `sweep_deadlines` ≈ :2505, `ReaderExit` drop ≈ :3100.

## P2e2a (WIP fe93077) — codex settlement, Unconfirmed exits, Dropped{Unknown} (471k tokens · 115 tools · 30 min)
```rust
struct Settling { gen: u64, deadline: Instant }   // State.settling: Option<Settling>; "awaited" derived: State::awaiting(gen) over PendingItem::awaits_reply_in(gen) = InFlight{gen, awaiting_reply: true}
impl State {
  fn settle_if_resolved(&mut self) -> bool;                       // single close point — debt is set here (P2e2b)
  fn expire_settlement(&mut self, now) -> Option<Disposal>;       // + free fn expire_settlement(state, core, now), end of sweep_deadlines
  fn acceptance_unknown(&mut self, pos) -> Option<QueuedInputEvent>; // listed → Unconfirmed · listed+cancel_asked → Dropped{Unknown} · unlisted → silent removal
  fn close_turn_items(&mut self, gen, close: TurnClose) -> Disposal; // exhaustive, no guard/_ arm
}
struct Disposal { events: Vec<QueuedInputEvent>, unechoed: Option<(u64,u64)> }  fn emit(self, core) // outside lock
enum SteerMove { Stale, Moved(Disposal) }
struct Anomalies { unechoed: u64, late_echoes: u64 }   // State.anomalies, per incarnation
Reader::note_delivered(&self, events) -> bool          // true → display-only TurnEnd{turn_id: None, Completed} via emit_without_turn_observation
```
- Settlement opens only when a steer still awaits its reply at `completed`/`failed` → never in production with steer off. While open: `take_turn_locked` → None, `zone_of` → Blocked. Closes on echo / reply / steer deadline / write failure / own deadline; `ReaderExit` drops it.
- Replies in settlement: Accepted (no echo) / Unanswered → acceptance unknown (+ unechoed counter); Refused → Held in place (cancel_asked → Dropped{Withdrawn}).
- ✕ on Unconfirmed → TooLate + CancelRequested + CancelAnswered{removed:false} + Dropped{Unknown}, removed, writer woken. Late echo → Delivered + removal + notify_all + warn(count,total); if Idle and not `ours` → synthetic display boundary (P2c ruling (2) resolved: place + boundary; also covers halted Idle).
- Writer wake = `cv.notify_all()` in the same state-lock section (no new hook).
- ★Orchestrator decision 2026-09-26 (delegated; TRD deviation → final report + P6)★: **debt (`follow_up_owed`) is judged only for items that entered via `turn/steer`** — TRD L540's literal rule would also fire for `turn/start` carriers (turn completes with no output item after the echo) and send an empty `turn/start` in production with steer off (M9: model repeats its previous answer). A carrier's turn always samples its input first; M10 "recorded only" is a steer phenomenon; (a) is no worse than today.
- Residuals: `debug_assert!(settling.is_none())` on open (reader-thread panic in debug if ever violated) · race: `note_delivered` reads Idle before emit — a Direct send in between → boundary right after that Direct bubble hides "Wait" until first output · `unechoed` also counts steers whose own deadline expired before turn end.
- P6 owed: TRD L458 (awaited derived, not stored) · TRD §5-5/§5-7 synthetic display boundary after a late echo · `output_core.rs` doc of `emit_without_turn_observation` (「호출자는 오늘 둘」, 「종료 신호를 낼 이벤트는 이 문으로 보내지 않는다」) stale → P2e2b fixes the comment.
- TODOs: P2e2b L610 (const) · L1184 (debt, immediate close) · L1233 (debt at settle_if_resolved) · L2368 (Idle order) · P2f L589 (halted Idle → Blocked) · L1354 (arrival) · `TODO(ADR-0231)` on `TurnClose::Failed` and codex `classify_turn`.

## P2e2b (WIP 79ffcc2) — codex debt issuance, Idle order, steer ENABLED (444k tokens · 126 tools · 24 min)
```rust
struct State { …, unanswered: Option<u64> /*gen; set in note_delivered for InFlight{steered:true} of the open turn; cleared by an Output item/started of the current turn id; taken at every close*/, follow_up_owed: Option<u64>, debt_paid_by: Option<u64> /*only for the abnormal-end warn*/ }
struct Settling { gen, deadline, owes: bool }   struct Anomalies { unechoed, late_echoes, unpaid }   // Disposal gains unpaid: Option<u64>
enum Opening { Item(usize), FollowUp }   impl State { fn idle_opening(&self) -> Option<Opening> }  // Above: user Held (Direct incl.; waits if unannounced) → FollowUp (empty turn/start, no clientUserMessageId) → mail only if no Unconfirmed; Below/Pending: today's FIFO
enum Stage { Held, InFlight { gen, turn_id, awaiting_reply, steered: bool }, Unconfirmed }   // steered only via take_steer_locked
```
- Debt = `unanswered == gen ∧ close == Completed`, set in `close_turn_items` (no settlement) or via `Settling.owes` → `settle_if_resolved`. Carrier (incl. re-sent after refusal) never owes. Debt cleared at issue (user or empty turn; mail never clears). Paying turn ends abnormally → no reissue, `unpaid += 1` + warn.
- `STEER_ENABLED` + `State::steer` deleted; steer gated only on `Floor::Above`. `zone_of` Idle derives from `idle_opening()`.
- Intended above-floor behaviour change: a Held user item opens before older Held mail; mail (incl. legacy id-less `send_input`) waits while any Unconfirmed exists.
- ★P2f plug points★: halted → top of the Above branch of `idle_opening` (while halted: `Item(first user Held)` only if some user item has `arrival` > mark, else None — no FollowUp, no mail); `zone_of` follows automatically. Set halted in `close_turn_items`' `match close` (Failed/Rejected/Unanswered split already there).
- Residuals now live in production (steer on): P2e2a `debug_assert!(settling.is_none())`, the `note_delivered` Idle race. Unverified: the empty follow-up turn streams assistant output with no preceding user bubble — front rendering / "Wait" needs P2 GUI QA.
- P6 owed: TRD L540 / L520 / L656 contradict the steered-only debt decision (amend); TRD L458 field list (+ `unanswered`, `Settling.owes`, `debt_paid_by`, `Anomalies.unpaid`, `InFlight.steered`); TRD tension L542 vs L670/L746 on `debt_paid_by`. P2e1 anchor list in this file is stale.
- TODOs: :559 `TODO(ADR-0231)` Failed → halted (P2f) · :599 AtEarliestBoundary (P8) · :619 / :1055 / :1447 `TODO(P2f)` · codex/mod.rs:1210 `TODO(ADR-0231)` classify_turn Failed.

## M15 phase A (2026-09-26) — setup works, measurement defect found
- Driving surface = direct WS client to the daemon (`scratchpad/m15drive.mjs`: daemon.json → `Auth` → `CreateProfile{backend:'codex', output_format:'StreamJson'}` → `SpawnProfile` → `Subscribe` → `WriteStdin`). No GUI / client shell / vite needed. `engram` CLI unusable (per-agent token, no input verb). Daemon launched directly via `launch-detached.ps1 -EnvVars 'RUST_LOG=warn,engram::codex_handover=debug','ENGRAM_DATA_DIR=<isolated>'` works. Flood helper `flood.js N sleepMs batch` (model-written inline loops break on cmd quoting).
- Observed: all three phases emitted; format matches the parser; turn-id hop and announce steers work end to end.
- ★Defect (P2d trace)★: `lag_us` measures from the previous short read, so time the reader spends BLOCKED in `read()` waiting for codex (model thinking, idle between turns — up to 101 s) counts as lag; contradicts the `HANDOVER_TRACE` doc. codex emit→pick = 0–3 ms vs lag_us 21–178 ms. Fix: count only busy time (timestamp before each read; exclude the blocked interval). → P2 fix round.
- Debug build: `handle_us` 2.1 ms parsing a 4000-line `item/completed` → phase B must use a `--release` daemon.
- Command-output deltas are not forwarded to subscribers (driver keys off ToolCall). Parallel tools not yet reproduced (codex ran two echoes sequentially).
- README header stale (says steer off). Phase B estimate ≈ 35 codex turns, 20–40 min.

## P2 review (deep, 2026-09-26, snapshot df1ab8d) — FIX → fix round "P2g"
- codex blind **FIX** (61k tokens, 2 tool calls): echo before the steer reply → `note_delivered` removes the item → `close_turn_items` sees nothing awaited → next `turn/start` may open before the reply; later reply Stale (transport.rs ~:3992). Fix: keep the reply obligation after the item leaves.
- doc-aware **FIX**: F1 halt / settlement / debt transitions unlogged (logging-conventions 「계측 의무」); F2 unknown/missing `turn.status` → `TurnClose::Completed` lifts the transport halt and can set debt while the kernel (`Ended(Other)`) keeps `last_end_failed` → add an Unknown close that neither raises nor lifts the halt and owes no debt. Advisories: A1 empty follow-up measured only on 0.156.1 (floor 0.140) → residual §5-9; A2 RichSlot loading gate also reaches claude (continuing claude with a Queued first input keeps the loading panel until Delivered) → report.
- concurrency lens **PASS** (no lost wake, no lock cycle, `debug_assert` unreachable, reducer correct under every interleaving). Advisories → residuals: F1 previous `TurnEnd` can land after the next Direct bubble (cosmetic "Wait" hidden until first output); F2 list-drained doorbell rings before `TurnEnd{Failed}` is observed → one mail can slip into the halted transport (same class as §5-9 check-then-write). Out-of-lens: kernel halt below floor = by design (TRD L250, confirmed by doc-aware).
- Doc debts (P6, from doc-aware): CLAUDE.md 「핵심 불변식」 codex JSON persistence exception + `order → state` lock / fanout-under-`order` ADR-0006 exception; ADR-0226 amendment; `docs/reference/backend-capabilities.md` L38/L43/L121; TRD L540/L520/L656 (steered-only debt), L458 field list (+ `unanswered`, `Settling.owes`, `debt_paid_by`, `Anomalies.unpaid`, `InFlight.steered`, `halted`, `floor`), L356, L459, §5-5/§5-7 display boundary, L245, §5-9 (A1, unknown status); `decoder.rs:710-714` comment (fixed in P2g).

## P2g (WIP 4ba00b5) — review fixes (407k tokens · 96 tools · 18 min; one rate-limit death before any edit → resumed)
- `State.steers_owed: Vec<SteerOwed{gen, item, echoed}>` (reply obligation outlives the item; created in `take_steer_locked`, released by reply / steer deadline / write failure / settlement expiry / ReaderExit); `awaiting(gen)` = owed entry ∨ pending item awaiting; refused-after-echo → release + `Anomalies.refused_after_echo` + warn, no resurrection; `push_pending` Direct also requires `settling.is_none()`.
- `enum Transition{Halted, HaltLifted, SettlementOpened, SettlementClosed, DebtSet, Discarded}` logged after unlock (halt info, rest debug); `Job::Turn{follow_up, paid}` → writer logs empty follow-up written (info) / debt paid (debug).
- `TurnClose::Unknown` (missing / `inProgress` / unknown status): disposition as Completed, halt unchanged, no debt, not unpaid.
- `DrainMark{drained, blocked}`: reader lag = busy time only (blocked `read()` excluded); known upper-side error after an idle wait documented. M15 README updated (pre-P2g logs invalid for verdicts).
- Re-verify: codex PASS (thread check OK) · doc-aware PASS (F1/F2 closed; nit: `refused_after_echo` warn lacks `agent` field) · concurrency PASS. ★P2 review gate = PASS after fixes★.
- Residuals (→ P6 §5-9 + final report): concurrency F1 (previous `TurnEnd` after next Direct bubble — cosmetic) · F2 (drained doorbell before `TurnEnd{Failed}` → one mail into the halted transport; new bounded variant: during an owed-reply-only Completed settlement the kernel sees an empty list → mail Held in the transport ≤ 30 s; kill in that window drops it) · A1 empty follow-up measured only on 0.156.1 · A2 RichSlot loading gate reaches claude · owed-only settlement moves the mail wait from kernel to transport (bounded).
- Doc debts added (P6, TRD): L458 (+ `steers_owed`, `refused_after_echo`; awaited now stored), L460 + L522 (settlement can stand with an empty list; Queued because Direct requires no settlement), L516 + L518 step 1 ("unanswered steer requests of T, echoed or not"), L525 (echo delivers, settlement keeps waiting for the reply), §5-5 L495–502 add an unknown/missing-status row (disposition as completed, settlement yes, halt unchanged, no debt). L251 needs no change.

## P8 (WIP 939d9c7) — codex default `AtEarliestBoundary` (341k tokens · 112 tools · 18 min)
- `HAND_OVER_POLICY = AtEarliestBoundary` (codex/mod.rs), `State::new()` defaults from the const (Harness = production policy); `Immediate` kept via `with_hand_over` (fallback: revert if the vendor narrows the window). Zone table unchanged; gap fixed: Answer → Other on `item/started` now wakes the writer (no Signal). `answered_turn` helper ends its answer at a boundary (16 debt/settlement/halt tests depended on mid-answer steering). Gates: agent 1124 · daemon 746.
- Review (code deep): codex blind FIX — "held text before a failed turn stalls until the user types" · doc-aware PASS ("halt design AC24, not a leak") · concurrency PASS (N1 trace mislabel for the new wake; N2 = same halt amplification, "confirm intended"). ★Resolved = by design★: TRD §10-1 item 9 (Q8, user decision) — codex never auto-sends after an error end; held items wait for the next user message (older first), not even after limits reset; claude = vendor behaviour. Orchestrator first recommended auto-sending (B) without checking the record → corrected to the user (2026-09-26). N1 → P8-fix.
- User (2026-09-26): after everything, discuss simplification / refactoring candidates together (「다끝난뒤에 단순화 한번 나하고 얘기해보자」). Also asked: does queued input vanish on a claude usage-limit error? → unknown (not measured; claude = vendor); offered an error-turn check in the final QA (answer pending).
- TRD 9판 committed locally (`5cc8ce0`, 410k tokens · 157 tools · 31 min) → `/review doc full` running. Other docs owed (from the TRD worker): CLAUDE.md (ownership split, lock order four locks + `order → state`, two fanout-under-lock exceptions, codex JSON persistence exception, replay→live seq, micro-rules hold/caps/reconcile/Channel wait, `last_end_failed` cursor) · ADR-0006 · ADR-0226 amendment (new ADR, stamp) · ADR-0231 check · `docs/reference/backend-capabilities.md` L38/L43/L121 · `docs/reference/architecture-overview.md` input path.
- P8-fix (`7d0bf7d`, `1ec0486`): trace `hop="zone"` + `State::zone_opened` (cleared on a new Hold zone and on any boundary signal) · `refused_after_echo` warn gets `agent` · comments (`output_decoder`, `OutputEvent::Error`) · `agent.listQueuedInputs` summary + `commands.schema.json` regenerated + `prompts/engram-help.md` · M15 parser accepts `hop=zone`. codex re-verify r1 FIX (stale zone marker) → r2 pending.
- TRD 9판 review (doc full): codex cut-advocate FIX ×7 (9판 notes vs struck 8판 text in the same line: L383 leaf, L157/L164/L873 "remaining measurement / stop and ask"; M15/P8 repeated in ~10 places → keep numbers at L164, decision at L873, index at L1109; L26 old order + master-merge instruction; L497 "second Delivered"; L644–647; L25 list vs L32 vs §10-6) · doc-aware FIX ×10 (F1 ★orchestrator plan "M15 red → don't ask" contradicted PRD R8 / PRD §10 / ADR-0231 decision 2 / §10-1 23 — never triggered (M15 green); record it as such★ · F2 §10-6 decider labels (review fixes are orchestrator-adopted → final report; L1106 → orchestrator) · F3 add `INPUT_QUEUE_LIMIT` row · F4 strike L335/L766/L521/L527, L513 "셋"→넷 · F5 M16 before P0 (`6290c5b`), M15 after P2 (`9ebebe8`) · F6 master merge done (`6787c83`) · F7 L227 first_turn_sink wording · F8 code comments cite 8판 TRD lines (`queuedInputReducer.ts:194`, `session.rs:3048`) → P6 · F9 lock order `order → {state, replay → registry → table}` for ADR-0006 · F10 L603 「실측」→「판독」). All user attributions verified word for word; all spot-checked behaviour claims match code.

## P2 local QA full (commit 3014247, isolated portable release) — PARTIAL, no P2 defect (442k tokens · 170 tools · 52 min)
- PASS: trial 5 Immediate (steered at once; ✕ on a handed-over row → requested + CancelAnswered{removed:false}, text still delivered, no notice) · 6 resume/hydration (caveat: Held window during hydration 5–30 ms — ✕ only via a WS auto-cancel client) · 7 interrupt (Dropped{Interrupted}×2) · 8 kill (Dropped{AgentEnded}) · 9 part 1 · 10 (3 + "외 2개", expand, popout ellipsis/tooltip, one bubble each) · 11 (no header element) · 13 (popout/main identical, reload, daemon close/connect, shell restart) · 14 LLM surface (engram agent.listQueuedInputs / cancelQueuedInput from another codex agent) · 16 codex mail waits for the list · QA-fix (a) real lease conflict → `Error("CONFLICT: …")`, UI ✕ keeps the row · (b) halted recipient hint "stopped after a failed turn …", recovery by user turn, `멈춤 풀림`, session id saved at first echo (P2c-fix confirmed).
- NOT RUN / NOT PROVOKED (Immediate makes Held short-lived): AC27 · UI ✕ on a Held row · empty follow-up turn/start (5 timing attempts) · late echo / Unconfirmed · trial 9 part 2 second input in the −32600 gap (gap itself reproduced once) · terminal-slot mail. → final QA on the P8 build (AtEarliestBoundary holds during tools).
- Observations for the final report: handed-over row can stay grey up to ~30 s under Immediate (codex records steer at its next sampling); ✕ then removes the row while the text still goes (TooLate, no notice) · trace lines are flushed at the writer's next non-steer job (≤ 500 ms) — only `steer_us` is reliable, not log timestamps · Wait timer ticks after kill / resets on reattach (pre-existing, P3 gate) · mail recipient names are cwd basenames, not the profile `name` (likely pre-existing) · a rejected Direct input's bubble stays, followed by the error row.
- Leftover: 6 launch logs + 3 .bat under %TEMP% (`detached-engram-dashboard-*`, `detached-cmd-*`) — the worker's delete was denied; left for the user.
- QA worker suggestion: TRD §7-2 trials that need a long-lived Held row should be marked fixture-only under Immediate, or get a fault-injection seam.

## P8 gate + TRD 9판 — DONE (2026-09-26)
- P8 squash `37ecc5e` pushed on top of `3014247` (code only, built with a temp index; local docs rebased onto it as `fbea43f`); CI green run 36229690656. Only GUI trial 5 under the new default is left for the final `/qa full`.
- ★Local history rewrite made these WIP hashes unreachable (never pushed): `9ebebe8`, `939d9c7`, `7d0bf7d`, `1ec0486`, `5cc8ce0` — older contacts sections cite them (and earlier P2 WIP hashes); code of `1ec0486` ≡ `37ecc5e`.★
- TRD 9판: `fbea43f` (content of the old `5cc8ce0`) → r1 fixes `12c9ec4` → r2 fixes `555993e`. `/review doc full`: r1 codex FIX ×7 + doc-aware FIX ×10 → r2 codex FIX ×3 + doc-aware FIX ×2 (R1 stale hashes / P8 status, R2 8판 original = `6290c5b`) → r3 codex PASS (verified its own 3 + doc-aware R1/R2 incl. hash ancestry; doc-aware did not re-run r3). TRD worker cost 410k + 471k + 488k tokens (cumulative).
- Still owed (P6): CLAUDE.md (ownership split · lock order four locks + `order → {state, replay → registry → table}` · fanout-under-lock exceptions · codex JSON persistence exception · replay→live seq · micro-rules hold/caps/reconcile/Channel wait · `last_end_failed` cursor) · ADR-0226 amendment (new ADR via /adr, stamp 0226) · ADR-0006 · ADR-0231 check (steered-only debt, Unknown close, recognisable lifecycle, P8 default) · `docs/reference/backend-capabilities.md` L38/L43/L121 · `docs/reference/architecture-overview.md` · F8 code comments citing 8판 TRD lines (`src/components/slot/queuedInputReducer.ts:194`, `crates/engram-dashboard-agent/src/session.rs:3048`) · `/review doc` · final `/qa full` on the P8 build (trial 5 AtEarliestBoundary, UI ✕ on Held, AC27, 13 with Held rows, empty follow-up if provokable; optional: claude queued input across an error turn — user asked, answer pending) · visual final report (user: image-centric) · then the simplification/refactoring discussion with the user.
- Push policy: only code commits are pushed; docs commits stay local until the user decides the public-repo privacy question (handoff docs contain an internal plugin name).
