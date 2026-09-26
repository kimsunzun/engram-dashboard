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
