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
