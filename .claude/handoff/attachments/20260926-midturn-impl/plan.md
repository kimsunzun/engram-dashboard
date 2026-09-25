# Mid-turn input queue — implementation plan (TRD 8판 → coder chunks)

Sources: `docs/process/S21-codex-backend/trd-mid-turn-input-queue.md` (cited `TRD Lnnn`) · PRD 6판 (AC#) · CLAUDE.md.
Baseline HEAD `6787c83`: since the TRD's read commit `2f456f8` nothing changed in `crates/engram-dashboard-{agent,daemon,protocol,messaging}`, `src-tauri/src/daemon_client`, `src/api`, `RichSlot.tsx`, `structuredAccumulator.ts` (master merge = layout + unrelated `engram-dashboard-transport` crate) → TRD `file:line` anchors hold (~30 spot-checked). Code anchor: `// ADR-0231` (file exists; stamps on older ADRs are being written concurrently).

## 0. Rules for every coder brief

- Build stands after every chunk (`cargo build`, the crate's tests, `cargo fmt --check`); types before callers; never a half-migrated enum. Commit per chunk (`S21: feat(midturn): …`) = revert point.
- **Keep `OutputCore::new` (53 call sites incl. `transport/pty.rs:607,642,648,823`, `stdio.rs:551`) and `AgentSession::new` (18 sites) signatures** — add builders (`with_queued`, `with_mid_turn`) like `with_incarnation`. TRD L717: `transport/{pty,stdio,input_queue}.rs`, `TerminalSlot.tsx`, `src/lab/terminal/` diff 0 vs master (`git diff --stat master... -- <paths>` empty at each phase end). `InputEvent` gains no variant; new `AgentTransport` verbs have defaults.
- CLAUDE.md commands: `cargo test -p engram-dashboard-agent -- --test-threads=4`, `-p engram-dashboard-daemon -- --test-threads=4`, `-p engram-dashboard-messaging`, `-p engram-dashboard-protocol`, `-p engram-dashboard --test lib_unit`, `npm test`, `npx tsc --noEmit`; phase end = `cargo test --workspace -- --test-threads=4` + gates (core `use tauri` rg · messaging rg + cargo-tree · `replay_flight` purity regex · ts-rs bindings regenerated and committed).
- Unit tests use fixtures only; GUI only via `/qa full` isolated instance. Read cited ranges only (`manager.rs` 6.2k lines, codex `transport.rs` 5.5k, TRD 255k chars).

## 1. Phases

### P0 — fixtures (1 chunk · simple · worker-scout)
- Goal: raw logs `.claude/handoff/attachments/20260925-midturn-phase0/logs/*.jsonl` (`{t,dir,line}` wrappers) → vendor-line fixtures.
- New files: `agent/src/backend/claude/fixtures/{lifecycle_m1,cancel_m3,drain_m7,slash_m13}.jsonl` (precedent `include_str!` at `claude/mod.rs:2581`); new dir `backend/codex/fixtures/{steer_m6,record_only_m10,empty_turn_m9,tool_end_m7}.jsonl`. Hand-build and mark what was never captured: claude `is_error:true` result (TRD L744), transcript `attachment{queued_command}` (M5 shape from the Phase 0 report).
- Scrub: `line` payloads only, no `meta`; replace absolute paths / OS user / cwd (codex thread responses echo cwd). Done: `rg -i "users\\\\|kimsunzun" fixtures` = 0; agent tests unchanged. No GUI.

### P1 — vocabulary + skeleton, zero behavior change (5 chunks)
No worse = nobody emits `QueuedInput`; all backends report `MidTurnPolicy::None`; `inputs_pending`/`last_end_failed` always false; accumulator arm never fed.

- **P1a vocab + reducer + wire (critical: protocol).** `agent/src/types.rs` (`OutputEvent` :37; `OutputSink` :872 doc = non-blocking contract, TRD L237) · **new** `agent/src/queued_input.rs` + `queued_input_golden.json` · `output_core.rs` `estimate_cost_bytes` :834 (text + copies) · `classify_turn` in `backend/claude/mod.rs` and `codex/mod.rs:1163` · `protocol/src/messages.rs` `StructuredEvent` :628 · `daemon/src/connection_core.rs` `output_event_to_wire` :739 (+ all-variants test :4577) · every other exhaustive `OutputEvent` match (`daemon/src/bin/saturation_pilot.rs:936`, `drain_latency.rs`) · bindings. Types verbatim TRD L302-314 + `OutputEvent::QueuedInput(QueuedInputEvent)`. Pure reducer = TRD §5-2 table L324-353 + tombstones L357-362 (FIFO 1024, set, `resurrectable = cause==Unknown`); `Registry{items, tombstones, ack_unavailable_seen, as_of_seq}`, `QueuedInputs` = `Mutex<Registry>` + `snapshot() -> (Vec<QueuedRow>, Option<u64>)`. Classifiers: claude `QueuedInput(Delivered)`→`Progress`, other `QueuedInput`→`None`; codex all `None`. Tests: TRD L738 (reducer), L739 (golden), classifier rows of L742/L747, wire row of L748.
- **P1b TS reducer + accumulator arm (standard; parallel with P1c–P1e).** New `src/components/slot/queuedInputReducer.ts`, `structuredAccumulator.ts` (:42). Test reads the golden by path and checks 1024. Draw rules TRD L600-611: bubble at ring position on `Delivered`, pending-uuid echo suppression, "already drawn uuid", `AckUnavailable` copies, `Cancelling` not drawn, `Dropped{Rejected}` removes bubble, `turnDone` untouched; `snapshotQueued()`. Tests: accumulator subset of L750.
- **P1c turn table + kernel (critical).** `agent/src/turn.rs` (`TurnSignal`, `observe_at` rebuilds the entry — carry new fields) · `types.rs` `StatusSink` :878 · `output_core.rs:~331` (`signal == TurnSignal::Ended` → `matches!`) · both classifiers · `messaging/src/busy.rs` (`TurnFact`, `BusyPolicy::is_busy`, `ScriptedTurnFacts` setters) · `daemon/src/messaging_host.rs` (`ManagerTurnFacts` :246, `MessagingFlushSink` :879) · `manager.rs` (:645/:778/:1970). `TurnSignal::{Progress, Failed, Ended(TurnEndKind)}`, `TurnEndKind::{Clean, Failed, Other}`*; entry `error_seen` + `last_end_failed`, fold at `Ended` (Failed or error_seen → true · Clean → false · Other → keep; smaller seq dropped; `forget` clears — TRD L251). New leaf `InputsPendingTable`* (`register/set(id,epoch,seq,bool)/get/forget`, smaller seq dropped) owned by manager. `TurnFact += inputs_pending, last_end_failed`; `busy = inputs_pending ∨ last_end_failed ∨ (in_turn ∧ ¬stale)`, 30-min valve on `in_turn` only. Adapter reads pending table **then** turn table (L247 ④). `StatusSink::inputs_drained(id, epoch)`* default no-op; flush sink → `idle.enqueue` + forward. Mapping now: claude `MessageDone`→`Ended(Clean)`, `Error`→`None` (P3a flips); codex `Completed`→`Clean`, `Interrupted|Unknown`→`Other`, **`Failed`→`Other` until P2f**. Tests: L741, table part of L744.
- **P1d core wiring (critical: lock order).** `output_core.rs`, `session.rs`, `manager.rs` (spawn :1884-1975, `wired_test_core` :2771). `OutputCore::with_queued(QueuedWiring{registry, pending})`* (detached default). In `emit_inner` **inside the replay lock**: seal (`sealed ∧ Queued` → ring `Dropped{AgentEnded}`) → ack rewrite (`ack_unavailable_seen ∧ Queued` → `Queued` + `AckUnavailable{[copy]}`, two seqs) → fill `AckUnavailable.delivered` → reduce → "filled"; after turn observation → "empty" + `inputs_drained` (TRD L243-247). `finish` :406: after `finalized.swap`, one replay-lock batch (scan → `Dropped{AgentEnded}` + seq + push + reduce + seal) → fanout, shape of `emit_batch_without_turn_observation` :209; forget pending table. Session holds the same `Arc<QueuedInputs>`. Tests: L740 (all but empty-ring `replay_from`), incl. two-thread delivery.
- **P1e input-path skeleton (critical: public API).** `types.rs` · `transport/mod.rs` · `backend/mod.rs` `SpawnParts` :635 · three `open_spawn` (default, `claude/mod.rs:392`, `codex/mod.rs:1051`) · `session.rs` `write_input_observed` :220 · `manager.rs` `write_stdin` :2671 family · daemon `connection_core.rs:1087`, `messaging_host.rs:135`, `bin/saturation_pilot.rs:1122,1488`, smokes. `InputOrigin{User,Mail}`; `MidTurnPolicy{None, SessionClassified{cancel_line: fn(&str)->Vec<u8>}*, TransportOwned}`; `DeliveryAck` (AtomicU8, `set_available` CAS never back, `try_set_unavailable()->bool`); `TurnInput{id,body,origin}`; `Withdraw{Withdrawn,TooLate,NotHeld}` (TRD L222-229, L428-429). Defaults `send_turn`→`send_input(Raw(body))`, `withdraw`→`NotHeld`. `SpawnParts += mid_turn, delivery_ack` (None/Unknown). Session `input_order: Mutex<()>`, `write_input_from(bytes, origin)`; `cancel_queued_input(id) -> Result<CancelOutcome{Requested,Cancelled}, CancelError{NotFound,Write}>`* skeleton. Manager `write_stdin(id,data,origin)`; `*_observed` = Mail; WS = User; pilot/smokes = Mail. Tests: TRD L726-727 regressions + existing byte-identical tests.
- Done (phase): workspace + gates + bindings + PTY diff-0. GUI: smoke only (json chat unchanged).

### P4 — commands (3 chunks; P4a standard, P4b/P4c critical)
No worse = list returns empty, cancel `NOT_FOUND`, no UI change.
- **P4a agent side.** `agent/src/commands.rs`: `agent.listQueuedInputs` (Read, `{target}`) and `agent.cancelQueuedInput` (Write, `{target, input_id}` both required, errors `[NOT_FOUND]`) per TRD L555-563; `catalog_version 4→5` (:39); `AgentCommandHost` (:195) += `list_queued_inputs`, `cancel_queued_input`, impl for `AgentManager` (:228). Response `{inputs:[{id,text,state,cancel}], as_of_seq, epoch, stopped_after_error}`, `state ∈ queued|unconfirmed|cancelling`, `cancel = {answer:"none"|"not_removed", vendor_closed}`|null*. Assembly: snapshot under `queued_inputs` → release → overlay `AgentTransport::unconfirmed_inputs()` (new, default empty) on `queued` rows → `stopped_after_error` from turn table. Not in `CLI_AGENT_VERBS` (types.rs:320). Tests: `tests/command_declarations.rs`, outcome mapping (L748).
- **P4b wire.** `AgentCommand::{ListQueuedInputs, CancelQueuedInput}` (TRD L566) + replies (`AgentEvent::QueuedInputs` / `QueuedInputCancelled`*) + `request_id()` matchers (messages.rs :774, :850); `PROTOCOL_VERSION 5→6` (`lib.rs:104`, v3-style doc); `connection_core.rs` arms (list: no lease; cancel: `check_input` like `WriteStdin` :1080). Shell relays generically (`forward_daemon_command`). Keep `version_mismatch_live_daemon_errors_without_spawn` (discovery/lib.rs:1991) green.
- **P4c bus lease.** `call_daemon_command` (`control/commands.rs:73`) + `LocalCommands::run` (`command_delivery.rs:1159`) gain `caller: Option<ConnId>`; `/control/call` (`catalog.rs:347`) and `/control/agent` (`control/agent.rs:156-168`) pass `None`. Input-affecting commands: resolve `target` once (`resolve_in`, commands.rs:931) → lease port over `MultiViewState::check_input` (connection_core.rs:452; `None` = non-holder) → rewrite `args.target`. Fix stale comment `control/commands.rs:154-156`. Tests: L748 (three entrances reject alike; resolved id reaches body).
- Order P4a → P4b → P4c. GUI none; CLI `engram call agent.listQueuedInputs` smoke.

### P5 — front + relay (3 chunks)
No worse = list never populated; seq continuity is a no-op without holes; placeholders only on bug paths; `historyPending` untouched.
- **P5a Rust relay (critical).** protocol: `peek_frame_header()` + `placeholder_error_frame(agent,epoch,seq)` (tag1 `Error`, fixed text) shared by daemon + shell (TRD L587, L591-593). `daemon/src/agent_conn.rs:84-110` two `return Ok(())` → placeholder (keep `debug_assert`). `src-tauri/src/daemon_client/connection.rs:1048` `Err(_)=>{}` → `UnknownTag` = placeholder via normal path, `TooShort` = `LoopExit::Disconnected`; SubscribeAck `replay_from` (dropped at :1444-1450) → subscription state → marker. `replay_flight.rs`: `Marker += replay_from`, layout `[tag255][agent16][epoch4 BE][gen8 BE][flags1][replay_from8 BE]`, `MARKER_FRAME_LEN 30→38` (tests :871/:913/:928). Poison → `into_inner` at `output_channel.rs:45`, `commands/agent.rs:164`, `commands/popout.rs:209`. `output_core.rs:579/604` empty ring `replay_from` = next seq. Tests: L748 placeholder, L749 (`lib_unit`), L740 last item.
- **P5b TS ordering (critical).** `src/api/protocolClient.ts`: live holds `seq > last+1` (today's drop :270), releases contiguous; flush (:397-403) from `max(last+1, replay_from)` contiguous only, rest held; overflow (4 MiB/8192) → `startBuffering` :454, never `ladderRerequest` in live. `tauriTransport.ts:326` marker 38 bytes → `replayFrom`; WS path reads SubscribeAck. Tests: L750 seq items.
- **P5c UI (standard).** New `QueuedInputList.tsx` (no header, ✕ on every item, `phase==queued`, oldest top, `--text-muted`, 3 + 「외 N개」 click-to-expand as prop defaults, full-text tooltip, ✕ tooltip 「목록에서 빼기」, Tab, dispatch `agent.cancelQueuedInput`). `RichSlot.tsx`: list-above-input wrapper, label on wrapper, `{hasQueued && …}` fixed child index (:385-387), `showEmpty` :304 `&& queued.length===0`, re-attach reconciliation (TRD L623-631: `as_of_seq`, apply after S delivered, replay seq > S, epoch filter, generation bumped on reset/`startBuffering`). `agentClient.ts` + `protocolClient.ts` methods; `agentCommands.ts` registration (no `help`); `ko.ts`.
- Order P5a ∥ (P5b → P5c) (shared `protocolClient.ts`). GUI (meaningful): json chat and replay/reconnect unchanged both backends (trial 13 re-attach part), terminal unchanged (trial 12).

### P3 — claude producer (2 chunks · critical)
Visible (producer's change): claude mid-turn list + ✕, drain-turn input queues, mail waits for empty user list, rejected idle bubble removed, mail stops after error `result` until next user turn succeeds (AC29).
- **P3a decoder/classifier** (`backend/claude/mod.rs`): lifecycle (key `command_uuid`) `started→Delivered`, `cancelled→Dropped{Unknown}`, `discarded→Dropped{AgentEnded}`, `refused→Dropped{Rejected}` (TRD L406); `control_response` `cancel:<uuid>` → `CancelAnswered{removed: response.response.cancelled}` / error → `CancelFailed`; ack detection (init `msg_lifecycle_v1` or first lifecycle line → Available; init without → Unavailable + one `AckUnavailable{[]}`) with the `Arc<DeliveryAck>` handed to the decoder in `open_spawn` (:411); classifier `Error`→`TurnSignal::Failed` (result :806-829, line overflow :714); transcript `attachment{queued_command}` → user bubble (uuid `source_uuid`), skip `queue-operation`; `cancel_line` (L407 JSON). `mid_turn` still `None`. Tests: L742, claude part of L744.
- **P3b session** (`session.rs` + claude `open_spawn` flip): classify under `input_order` (pending table then turn table; `Queued` before `send_input`, after a successful send when ack `Unknown`; failure → `Dropped{Rejected}`+Err; no echo in Queued branch; `Unavailable`/`Mail` → today) per TRD L400-405; cancel L407-411 (`CancelRequested` → `cancel_line` → write failure `CancelFailed`+Err; idempotent). Tests: L743, session part of L744.
- GUI (meaningful): trials 1–4, 8–11, 13–16 claude parts (TRD L754-769).

### P2 — codex `Immediate` skeleton (7 sequential chunks on `backend/codex/transport.rs` · critical)
Visible at P2 end: codex mid-turn list steered immediately, idle bubble synthesized, hydration input listed, loading panel cleared by delivered bubble, nothing auto-sent after abnormal end until next user turn succeeds, ✕ everywhere.

| Chunk | Content | State at end |
|---|---|---|
| **P2a** pending model | `State.input` (:368) → `pending: VecDeque<PendingItem{id, body, origin, stage: Stage{Held, InFlight{gen, turn_id, awaiting_reply}, Unconfirmed}, listed, announced, arrival, cancel_asked, death}>` (TRD L458); `send_turn`; legacy `send_input` = Mail-like; `take_turn_locked` :1158 on head | identical to today |
| **P2b** announce + turn/start | floor verdict (`cliVersion`/`userAgent`, L468, L550) → ack; announce only after verdict; Direct echo via `emit_without_turn_observation`, Queued event, two-phase announce, writer waits for announced head, death marks (L460, L548); `TurnStartParams.clientUserMessageId` (protocol.rs:537); decoder `user_message` :642 `clientId` → uuid + `Delivered` before bubble; reader drops delivered id (gen check); `withdraw(Held)`→`Dropped{Withdrawn}`; turn-end disposition v1 at every end incl. early `resolve` :2086 (`EarlyCompletion` :376 keeps status): interrupted→`Dropped{Interrupted}`, no echo→`Unconfirmed`, write fail/error reply→`Dropped{Rejected}`, deadline/decode/long id→`Unconfirmed` (Direct: silent removal except Rejected); below floor → today | unreachable in prod (policy None); Harness-tested |
| **P2c** flip | `session.rs` TransportOwned branch (`send_turn` + origin; cancel→`withdraw`: Withdrawn→`cancelled`, TooLate→`requested`, NotHeld→`NOT_FOUND`); codex `open_spawn` `TransportOwned` + shared ack; `RichSlot.tsx:308` drop `!hasSent` + test (L621); rename test `neither_mode_makes_a_synthetic_input_echo` (mod.rs:1972) → `the_terminal_mode_makes_no_synthetic_input_echo` (L724) | list visible, delivery at turn end (today's timing), ✕ withdraws |
| **P2d** tracking + policy + trace | `HandOverPolicy{Immediate, AtEarliestBoundary}` const Immediate, seam at transport build; `decoder.rs` `pub(super) fn item_class()->{Tool,Output,Other}` from `TOOL_ITEM_TYPES` :270; segment state on current turn id; signals tool end / answer end / `tokenUsage`; writer wake; `hand_over()->{Now,Hold}` (L433-442); M15 trace (target `engram::codex_handover`*: signal time, reader lag, writer wake) | unchanged |
| **P2e1** steer (gated) | `Waiter::Steer{id,gen}` + arms in `sweep_deadlines` :1263 and stream close :1839; `turn/steer{threadId, expectedTurnId, clientUserMessageId, input}`; success/timeout→`awaiting_reply=false`; error→`Held` + "steer rejected" turn mark (`cancel_asked`→`Dropped{Withdrawn}`); gen masking; `withdraw(InFlight)`→`TooLate` + `CancelRequested` + `CancelAnswered{false}`; trace leg ② end; `const STEER_ENABLED=false` | unchanged |
| **P2e2** settlement + debt + enable | per-turn settlement (`settling{gen, deadline=REQUEST_DEADLINE :194, awaited}`, no turn opened meanwhile, L516-526); `Unconfirmed` exits (late echo→`Delivered` + wake + warn counter, no debt; ✕→3 events + wake); `cancel_asked` turning unknown→`Dropped{Unknown}`; `follow_up_owed` + empty `turn/start{input:[]}` never while Active, cleared at issue (L540-547); Idle order user → debt → mail (mail blocked by any `Unconfirmed`); enable steer, delete const | immediate steer live |
| **P2f** halted + error-end | `halted: Option<u64>` outside `TurnState` (L509-517): set on `failed` (incl. early) and five exits, any turn kind; Idle opens only a user turn with a post-halt user item (state-evaluated); cleared by `completed`; not by `interrupted`; never below floor; `failed` settlement = no debt; codex `classify_turn` `Failed`→`Ended(Failed)`; `unconfirmed_inputs()` body | final P2 |

- Tests: TRD L745, L746, L747, codex parts of L744; existing `Harness` (:2823) green every chunk.
- GUI (meaningful after P2c and at end): trials 5 (Immediate part), 6, 7, 8–11, 13, 14, 16 codex parts.

### P7 — measurements → §5. P8 — `AtEarliestBoundary` (1 chunk · critical, only if M15 green)
`backend/codex/{transport,mod}.rs`: tool-segment cell (hold → release on tool end), answer cell per M16 (`plan` = Other if not reproduced), const → `AtEarliestBoundary`. Tests: L746 `AtEarliestBoundary` rows on the default. GUI trial 5 (✕ during tool withdraws).

### P6 — docs + final QA (2 chunks · standard)
Finish/verify ADR-0231 + stamps (0044/0045/0190/0198/0193/0006/0226/0127/0110/0104 — partly being written now by another worker); CLAUDE.md invariants (ownership split, lock order + pending leaf, seq continuity, sink/shell never skip); `docs/reference/backend-capabilities.md`, `architecture-overview.md`; `/review doc`; `/qa full` trials 1–16.

## 2. Chunk count and ordering
P0 1 · P1 5 · P4 3 · P5 3 · P3 2 · P2 7 · P7 2 runs · P8 1 · P6 2 = **26**.
P0 → P1a → {P1b ∥ P1c → P1d → P1e} → P4a → P4b → P4c → {P5a ∥ P5b → P5c} → P3a → P3b → P2a…P2f → P7 (M16 can start after P0; M15 needs P2e2) → P8 → P6. Largest overrun risk: P2b, P2e2, P1d — run P1a as the pilot and size the rest from its token/tool counts.

## 3. Contact points (* = proposed name, main pins before the consumer starts)

| Contact | From | To | Shape |
|---|---|---|---|
| `QueuedInputEvent`/`DropCause`/`DeliveredCopy`, `OutputEvent::QueuedInput` | P1a | P1b P1d P3a P2b | TRD L302-314 |
| wire `StructuredEvent::QueuedInput{op}` | P1a | P1b P5c | `{"type":"QueuedInput","op":{"kind":"Queued","id","text"}}`*, cause string, `delivered:[{id,text}]` |
| golden file | P1a | P1b | `agent/src/queued_input_golden.json`, `{"tombstone_cap":1024,"cases":[{events,expect}]}`* |
| `QueuedInputs::snapshot()->(Vec<QueuedRow>,Option<u64>)` | P1a/P1d | P4a | row `{id,text,state,cancel}` |
| `TurnSignal::{Progress,Failed,Ended(TurnEndKind{Clean,Failed,Other})}`* | P1c | P3a, P2f | fold TRD L251 |
| `TurnObservation.last_end_failed`, `TurnFact{…,inputs_pending,last_end_failed}` | P1c | P4a, adapter | TRD L240 |
| `InputsPendingTable`*, `StatusSink::inputs_drained`* | P1c | P1d | leaf, smaller-seq drop |
| `OutputCore::with_queued(QueuedWiring)`* | P1d | manager, tests | builder |
| `InputOrigin`, `MidTurnPolicy`, `DeliveryAck`, `TurnInput`, `Withdraw` | P1e | P2 P3 P4a | TRD L222-229, L428-429 |
| `AgentSession::cancel_queued_input()`* | P1e | P4a; bodies P3b/P2c | outcome `requested`/`cancelled`, `NOT_FOUND` |
| `AgentTransport::unconfirmed_inputs()->Vec<String>` | P4a | P2f | default empty |
| bus names/args/outcomes, `catalog_version 5` | P4a | P5c, CLI | TRD L555-563 |
| WS commands + replies*, `PROTOCOL_VERSION 6` | P4b | P5c | TRD L566 |
| `call_daemon_command(…, caller, lease)` + input-affecting list* | P4c | — | TRD L569-575 |
| placeholder frame helper; 38-byte marker | P5a | daemon, shell, P5b | P5a row |
| `HandOverPolicy`, `ItemClass`, trace names* | P2d | P2e1 P8 P7 | TRD L431-442 |

## 4. Unknowns / risks the TRD does not settle (orchestrator to decide)

1. **`SpawnedSession` does not exist** — it is `SpawnParts` (`backend/mod.rs:635`, destructured `manager.rs:1884`). Simplest: add the fields there.
2. **Classifier is a stateless `fn` pointer** (`backend/mod.rs:651`); claude "error then end" must live in the turn-table entry, and `observe_at` rebuilds entries. Names in §3 are proposals.
3. **`write_stdin` serves WS (User) and `saturation_pilot` (Mail).** Simplest: `origin` param on `AgentManager::write_stdin` only; every `*_observed` = Mail.
4. **`INPUT_QUEUE_LIMIT = 32`** (`transport.rs:266`) will count items that can stay `Unconfirmed` indefinitely. Simplest: count all pending (visible, ✕-able).
5. **"Input-affecting" mark** has no field in `CommandSpec` (`command/src/spec.rs:29`); a macro change touches the command crate. Simplest: `pub const INPUT_AFFECTING: &[&str]` beside the declaration in `agent/src/commands.rs`; lease-denial code = reuse `CONFLICT` with the WS text.
6. **Marker layout (TRD L584 "unconfirmed")** — found at `replay_flight.rs:69-72,384` (30 B) and `tauriTransport.ts:326`; 38 B is safe (shell + webview ship together).
7. **Fixture privacy** — logs carry absolute paths with the OS user name and cwd; scrub in P0 (org rule).
8. **Halt before flip would be worse than today** (user input still Mail-like, nothing releases it) — order P2c before P2f; `STEER_ENABLED` in P2e1 is scaffolding (alternative: merge P2e1+P2e2 into one bigger coder).
9. **M15 red handling conflicts:** TRD §9 L854 = stop and ask the user; orchestrator brief = skip P8, keep `Immediate`. Only tool/answer-segment holding is lost (AC10/18/24/28 intact). Simplest: keep `Immediate`, state the trade-off in the final report.
10. **PROTOCOL_VERSION 6** — a v6 shell refuses a live v5 release daemon on the dev PC; GUI QA must use the isolated instance.
11. **M11/M12/M14/M17** have no harness here; non-blocking — note opportunistically in `/qa full`.

## 5. P7 — M16 / M15

- **M16** (no product code; any time after P0): `m7codex.js` → `m16codex.js` in the Phase 0 attachment folder; real codex 0.156.1; N ≥ 10 each: tool-less long answer, final answer after a tool, `plan` turn if reproducible. Steer at `agentMessage`/`plan` `item/completed` with no running tool; record hit (echo in the same turn's next sampling) and window. Green → P8 holds the answer segment; red → answer segment stays "now" (no stop).
- **M15** (needs P2e2): isolated instance via `scripts/launch-detached.ps1`, output to file, `RUST_LOG=engram::codex_handover=debug`; ≥ 20 tool ends incl. thousand-line-output commands and parallel tools (leg ① + reader lag); ≥ 20 `turn/start`-reply → held-item steer hops and ≥ 20 steers during an output flood (leg ②). Node script parses the trace. Green iff `max(①) + max(reader lag) + max(②) < 6 ms` (TRD L161); also report answer-end reaction for M16.
- **Stop:** M15 red → no P8, keep `HandOverPolicy::Immediate` (§4.9). M16 red alone → P8 with answer segment = now.
- Output: `docs/research/mid-turn-m15-m16-measurements-<date>.md`; scrubbed raw logs under `.claude/handoff/attachments/`.

## 6. Orchestrator decisions (2026-09-26 — pinned; coders follow these)

- §4.1 add the new spawn fields to `SpawnParts` (no `SpawnedSession` type). §4.2 the §3 proposed names (`*`) are adopted as written.
- §4.3 `origin: InputOrigin` param on `AgentManager::write_stdin` only; every `*_observed` path = `Mail`; WS = `User`; pilot/smokes = `Mail`.
- §4.4 `INPUT_QUEUE_LIMIT` counts every pending item (incl. `Unconfirmed` — visible and ✕-able).
- §4.5 `pub const INPUT_AFFECTING: &[&str]` beside the declarations in `agent/src/commands.rs`; lease denial reuses `CONFLICT` with the WS text.
- §4.6 marker 38 bytes as planned. §4.7 scrub fixtures in P0. §4.10 GUI QA only on the isolated instance.
- §4.8 order P2c before P2f; P2e1 and P2e2 are reviewed together (the `STEER_ENABLED` const never reaches a gated commit).
- §4.9 M15 red → no P8, keep `HandOverPolicy::Immediate`; report the trade-off (user delegated: 「복잡한것보다 약간 양보하면서 단순한 방향으로」).
- Wire shape `{"type":"QueuedInput","op":{"kind":"Queued","id","text"}}` and golden `{"tombstone_cap":1024,"cases":[{events,expect}]}` adopted. Code anchor = `// ADR-0231`.
- Commit policy: each coder chunk gets a local WIP commit by the orchestrator (revert point); after the phase's `/review code` + `/qa` pass, the phase is squashed (`git reset --soft <phase-base>`) into one `S21: feat(midturn): P<n> …` commit (CLAUDE.md: commit after gates).
