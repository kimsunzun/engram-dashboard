//! AgentSession — one agent = a composite of OutputCore (output side) + Box<dyn AgentTransport> (channel/resource side).
//!
//! It exposes a shared surface independent of the transport kind (PTY/API) and delegates internally to core/transport.
//!
//! Ownership split (impl-spec table): master/child/shutdown/job/reader/writer → inside the transport (PtyTransport),
//!   subscribers/replay/seq/status/finalized → inside core (OutputCore).
//!
//! So every method only touches its own fields (cols/rows atomics) or delegates to core/transport.
//!
//! tauri import 0.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use engram_dashboard_base::sync;

use crate::backend::InputEncoder;
use crate::output_core::{CancelRequest, OutputCore};
use crate::queued_input::{overlay_unconfirmed, ListedRow, QueuedInputs, RowPhase};
use crate::session_id_latch::SessionIdLatch;
use crate::transport::AgentTransport;
use crate::types::{
    AgentId, AgentStatus, BackendCaps, CancelError, CancelOutcome, Capabilities, DeliveryAck,
    DeliveryAckState, DropCause, Incarnation, InputEvent, InputOrigin, MidTurnPolicy, OutputChunk,
    OutputEvent, OutputSink, PtyError, QueuedInputEvent, SinkId, SubscribeReply, TerminationIntent,
    TurnInput, Withdraw, WriteOutcome,
};

pub struct AgentSession {
    pub id: AgentId,
    pub cwd: PathBuf,
    pub epoch: u32,
    pub cols: AtomicU16,
    pub rows: AtomicU16,
    /// User exit intent. Why `Arc`: the finalize hook closure captures the same value shared.
    intent: Arc<AtomicU8>,
    /// The caps (session/model) the backend (program) decided. manager.spawn computes them from profile.command and
    /// injects them — the transport does not know this value.
    backend_caps: BackendCaps,
    /// The input encoding applied **right before** write_input is passed to the transport (ADR-0044/0004).
    /// manager.spawn computes and injects it.
    encoder: InputEncoder,
    /// Whether this session is **a reader of mail** (= eligibility for the mail recipient list). It holds as is the value the backend computed
    /// at spawn — a backend-derived fact of the same kind as the encoder.
    ///
    /// ★Why the **session** holds it, not the profile (load-bearing)★: `DeleteProfile` does not kill a live session.
    ///   Deciding by the profile would make a live shell whose profile was deleted "unknown", putting it back on the list, and envelopes would be
    ///   executed as commands. The session knows what it was spawned as, and that fact does not change with profile deletion.
    reads_messages: bool,
    /// The wait between the body write and the submit write — grounds and value source in `backend::SUBMIT_PACING`.
    ///
    /// ★The default is set by `new` (it is not a constructor argument)★: letting the caller pick the value would make this defect silently recur
    ///   the moment some assembly path passes 0. The only way to lower it is one test-only seam.
    submit_pacing: Duration,
    /// The function that actually sleeps the wait above. In production it is a blocking sleep, and tests plug in one that **does not sleep but only records the call**
    /// to assert "the wait was issued" without time measurement (= non-flaky).
    sleeper: fn(Duration),
    /// This incarnation launched to resume a stored conversation — invariant per incarnation. The subscription response carries it with `epoch`.
    /// ★The incarnation marker is not this field but `epoch` alone★ — putting a marker here too would make two markers.
    continues_conversation: bool,
    /// This incarnation's session id first-submission latch. `None` = the submission counting of the two input verbs is a no-op.
    session_id_latch: Option<Arc<SessionIdLatch>>,
    /// Mid-turn input policy — the backend's reported value. Default (`with_mid_turn` not called) = `None` (today's path).
    // ADR-0231
    mid_turn: MidTurnPolicy,
    /// Ack availability the backend fills — the very Arc `SpawnParts` carried.
    delivery_ack: Arc<DeliveryAck>,
    /// The session input lock — ★takes only `SessionClassified` user input and cancellation★. The output pump does not take it: this
    ///   lock contains the first-submission latch commit (a disk write), so if the pump waited, output would stand behind disk I/O.
    ///   Lock order = `input_order` → replay → registry → waiting-list table (emit does not take this, so there is no reverse order).
    // ADR-0231
    input_order: Mutex<()>,
    core: Arc<OutputCore>,
    transport: Box<dyn AgentTransport>,
}

/// The time limit at which to give up confirming whether a delivery write **actually went out** ([`AgentTransport::flush_input`]).
///
/// ★The axis this value measures★: the place that tells 「not out yet」 from 「never going out」. Too short, and a briefly busy
///   receiver (a TUI reading input late because it is rendering a turn) is reported as a delivery failure and **the redelivery creates a duplicate**;
///   too long, and one stuck agent holds the delivery loop that long. ★Of the two, the worse is the duplicate★ —
///   a failure is recovered by retry, but a duplicate makes the receiver read the same mail twice. So it leans to the generous side.
/// ★Source of the value = a number this repository already uses for 「it should have finished by now」★: the `join_pump` cap of the kill causality is
///   5 seconds (ADR-0001). Instead of inventing a new number, the same place is used — this path's decision is also the same ("if it has not gone out
///   after 5 seconds, it is stuck").
/// ★The only path taking this wait is mail delivery★ — that path is a verb that already holds the caller 0.5 seconds because of
///   [`crate::backend::SUBMIT_PACING`], so waiting here is **not a new kind of blocking.** The keystroke
///   path (`write_input`) does not take this wait.
const INPUT_FLUSH_BUDGET: Duration = Duration::from_secs(5);

/// Production sleeper — this layer is a synchronous path, so it just sleeps blocking (the delivery loop being held that long is an accepted property:
/// terminal reception is for demos, and async separation is a structural change and a separate decision).
fn blocking_sleep(d: Duration) {
    std::thread::sleep(d);
}

impl AgentSession {
    /// Composite session creation. **start is not called here** — the manager calls
    /// `transport.start(core.clone())` directly before new (impl-spec: test visibility · explicit spawn flow).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: AgentId,
        cwd: PathBuf,
        epoch: u32,
        cols: u16,
        rows: u16,
        intent: Arc<AtomicU8>,
        backend_caps: BackendCaps,
        encoder: InputEncoder,
        reads_messages: bool,
        core: Arc<OutputCore>,
        transport: Box<dyn AgentTransport>,
    ) -> Self {
        Self {
            id,
            cwd,
            epoch,
            cols: AtomicU16::new(cols),
            rows: AtomicU16::new(rows),
            intent,
            backend_caps,
            encoder,
            reads_messages,
            submit_pacing: crate::backend::SUBMIT_PACING,
            sleeper: blocking_sleep,
            continues_conversation: false,
            session_id_latch: None,
            mid_turn: MidTurnPolicy::None,
            delivery_ack: Arc::new(DeliveryAck::new()),
            input_order: Mutex::new(()),
            core,
            transport,
        }
    }

    /// Carries incarnation facts — default (not called) = not a resume.
    // ADR-0226
    pub(crate) fn with_incarnation(mut self, continues_conversation: bool) -> Self {
        self.continues_conversation = continues_conversation;
        self
    }

    /// Carries the session id first-submission latch — default (not called) = no latch.
    ///
    /// ★If omitted at the production assembly point, every resume silently disappears★ — no submission is ever counted, so no
    ///   incarnation persists its id, and every next activation becomes a new conversation (with no error).
    /// ★A `TransportOwned` session does not count this latch's submissions★ — the first-turn port of the same latch must be plugged into the link
    ///   ([`SessionIdLatch::first_turn_sink`] · assembly point = `open_spawn`).
    // ADR-0226
    pub(crate) fn with_session_id_latch(mut self, latch: Arc<SessionIdLatch>) -> Self {
        self.session_id_latch = Some(latch);
        self
    }

    /// Carries the mid-turn input policy and ack availability — default (not called) = `None` · `Unknown`.
    ///
    /// ★If omitted at the production assembly point, the list silently disappears★ — all input goes to today's path, with no error.
    // ADR-0231
    pub(crate) fn with_mid_turn(
        mut self,
        mid_turn: MidTurnPolicy,
        delivery_ack: Arc<DeliveryAck>,
    ) -> Self {
        self.mid_turn = mid_turn;
        self.delivery_ack = delivery_ack;
        self
    }

    /// ★Test-only seam★ — lowers the submit wait (so the harness does not sleep 0.5 seconds each time) or observes the wait issuance itself.
    /// **The production default is always `backend::SUBMIT_PACING`** (field doc above) — this function does not change that default;
    /// a test lowers it explicitly only for its own instance.
    #[cfg(test)]
    pub(crate) fn with_submit_pacing(mut self, pacing: Duration, sleeper: fn(Duration)) -> Self {
        self.submit_pacing = pacing;
        self.sleeper = sleeper;
        self
    }

    /// Whether this session is a reader of mail — the decision grounds are in the field doc.
    pub fn reads_messages(&self) -> bool {
        self.reads_messages
    }

    /// This incarnation's queued input registry — **the very** Arc the core feeds in emit.
    /// ★Taken from the core rather than held separately by the session★: plugging it into two places separately could split the registry the session reads
    ///   from the one the core feeds, and that mismatch shows only as an empty list, with no compile error or runtime error.
    /// ★Do not emit while holding the guard★ — the lock order is replay → registry (`OutputCore::queued_inputs`).
    // ADR-0231
    pub fn queued_inputs(&self) -> &Arc<QueuedInputs> {
        self.core.queued_inputs()
    }

    /// User exit intent tagging (ADR-0019) — kill_agent calls it **before** transport.shutdown.
    /// The finish hook snapshots this value at the moment of finish, so it must be set before shutdown so that when the pump
    /// wakes up and finishes, UserKill is observed (the order is the key to race prevention).
    pub fn set_intent(&self, intent: TerminationIntent) {
        self.intent.store(intent as u8, Ordering::SeqCst);
    }

    /// The exit intent observed so far — ★a value meant to be read as a monotonic latch★.
    ///
    /// ★Why a reading verb was needed★: the activation decision must know **the user's cancellation before the terminal state**.
    ///   `kill_agent` sets this value even before `enter_exiting` ([`AgentManager::kill_agent`]),
    ///   and only after that brings the link down. So this latch is **always seen first** before any observation caused by the kill (the link's delivery stop · delivery channel closed ·
    ///   `Exiting` state) — the terminal transition (`Killed`) does not stand until the pump wakes up,
    ///   so a decision looking only at it misreads the interval as 「connection failure」.
    /// ★Read with `Ordering::SeqCst`★ — the same ordering as the writing side (`set_intent`), so they pair.
    /// ★A new value per incarnation★ — `spawn_session` makes a new `Arc<AtomicU8>` per incarnation. So the kill intent of the previous
    ///   incarnation is not visible in this incarnation.
    // ADR-0019
    pub fn termination_intent(&self) -> TerminationIntent {
        TerminationIntent::from_u8(self.intent.load(Ordering::SeqCst))
    }

    /// Delegates the pump startup (transport.start). ★ADR-0019 reaper order★: the manager starts this session **after inserting** it into the sessions
    /// map. Even if the pump immediately sends EOF→finish→ReapMsg, it is already in the map by then,
    /// so the reaper reaps it normally (if start came before insert, the hook send would point at an id not in the map and
    /// become a zombie). attach_pump completes synchronously inside start, so it is independent of the insert order (no effect on join_pump).
    pub fn start_pump(&self) {
        self.transport.start(self.core.clone());
    }

    /// Passes input bytes → (encoder applied) → transport.
    ///
    /// ★Wiring point (ADR-0044)★: applies the encoder to wrap a text turn per the backend convention, then
    ///   passes it to the transport **always as Raw bytes**. The transport is a dumb pipe and does not know the shape.
    ///   - Raw (terminal · shell): `encode` copies the bytes as is → **byte-identical** to the existing path.
    ///   - ClaudeStreamJson (json mode): wraps the text into a claude user JSON line (escaping and schema are
    ///     backend/claude/ alone — the session holds only the tag and does not know the shape, ADR-0004 isolation).
    ///
    /// ★Call contract (FIX 6a) — in json mode `1 write_input call == 1 complete user turn`★:
    ///   the ClaudeStreamJson encoder wraps every call into **one** `{"type":"user",…}\n` line. That is, per call
    ///   claude receives one whole user turn. Calling **one keystroke at a time** like the terminal path
    ///   would create a wrong one-character turn per character and break the conversation. So json-mode callers (RichSlot · M2)
    ///   must send **the whole completed message at once** (accumulating partial input is the frontend input box's job). The terminal path
    ///   is Raw, so streaming byte calls are normal as before (this contract is json mode only).
    /// Origin = `Mail` (same as [`AgentSession::write_input_observed`]) — human input carries its origin through
    /// [`AgentSession::write_input_from`].
    pub fn write_input(&self, bytes: &[u8]) -> Result<(), PtyError> {
        self.write_input_observed(bytes).map(|_| ())
    }

    /// A write carrying its origin — behavior, return and byte contract are the same as [`AgentSession::write_input_observed`], and the only difference is
    /// that `origin` reaches the policy branch.
    ///
    /// ★With policy `None`, `origin` changes nothing★ — both origins take today's path (byte-identical · no list
    ///   events). `TransportOwned` hands the body and origin to the link turn verb — the link classifies.
    ///   `SessionClassified` classifies only `User` (below, [`Self::write_user_classified`]) and `Mail` takes today's path.
    // ADR-0231
    pub fn write_input_from(
        &self,
        bytes: &[u8],
        origin: InputOrigin,
    ) -> Result<WriteOutcome, PtyError> {
        match (self.mid_turn, origin) {
            (MidTurnPolicy::None, _) => self.write_now(bytes, None),
            (MidTurnPolicy::SessionClassified { .. }, InputOrigin::User) => {
                self.write_user_classified(bytes)
            }
            (MidTurnPolicy::SessionClassified { .. }, InputOrigin::Mail) => {
                self.write_now(bytes, None)
            }
            (MidTurnPolicy::TransportOwned, _) => self.write_now(bytes, Some(origin)),
        }
    }

    /// User input under `SessionClassified` — classification → list event → write run as one lump while holding the input lock.
    ///
    /// ★If idle or ack-unavailable is settled (`Unavailable`), it is today's path (write + synthetic echo)★. Idle = waiting-list table empty ∧
    ///   turn observation not in a turn ([`OutputCore::classified_input_busy`]) — the stop after error is not read (text typed during the stop
    ///   also takes today's path if idle, and that turn clears the stop).
    /// Otherwise it is the list branch and emits no synthetic echo (the bubble stands at the vendor's acceptance position):
    ///   - `Available` — emits `Queued` **before** the write: the lifecycle comes within 1 ms after writing, so emitting after would have the acceptance dropped as an
    ///     unknown id. Write failure = emit `Dropped{Rejected}` and `Err`.
    ///   - `Unknown` — emits `Queued` **only after** the write succeeds. Emitting before would, if the pump settled ack-unavailable while the write runs,
    ///     close that entry as accepted, and even if the write then failed, `Dropped{Rejected}` would be swallowed by the terminal tombstone, leaving
    ///     unsent text as a bubble. Write failure = emit nothing and `Err`.
    /// ★What the lock guards★: the classification and emit order of two inputs do not diverge, and the cancel line for the same id goes out only after this write returns
    ///   ([`Self::cancel_queued_input`]). Emits under this lock run while holding the subscriber fanout —
    ///   the contract that `OutputSink::send` does not block backs that (an exception to ADR-0006).
    // ADR-0231
    fn write_user_classified(&self, bytes: &[u8]) -> Result<WriteOutcome, PtyError> {
        let _order = sync::lock(&self.input_order);
        let ack = self.delivery_ack.state();
        if ack == DeliveryAckState::Unavailable || !self.core.classified_input_busy() {
            return self.write_now(bytes, None);
        }
        // ADR-0226: the same place as today's path — counted before sending.
        self.count_turn_submission(bytes)?;
        let msg_uuid = uuid::Uuid::new_v4();
        let id = msg_uuid.to_string();
        let encoded = self.encoder.encode(bytes, msg_uuid);
        let queued = OutputEvent::QueuedInput(QueuedInputEvent::Queued {
            id: id.clone(),
            text: String::from_utf8_lossy(bytes).into_owned(),
        });
        if ack == DeliveryAckState::Unknown {
            self.transport.send_input(InputEvent::Raw(encoded))?;
            self.core.emit(queued);
        } else {
            self.core.emit(queued);
            if let Err(e) = self.transport.send_input(InputEvent::Raw(encoded)) {
                self.core
                    .emit(OutputEvent::QueuedInput(QueuedInputEvent::Dropped {
                        id,
                        cause: DropCause::Rejected,
                    }));
                return Err(e);
            }
        }
        let n = bytes.len();
        Ok(WriteOutcome {
            bytes_requested: n,
            bytes_written: n,
            msg_uuid,
            epoch: self.epoch,
        })
    }

    /// The delivery-boundary instrumented version of `write_input` (ADR-0088 Stage 0) — on success returns `WriteOutcome` (logical message bytes +
    ///   this turn's `msg_uuid`). Behavior and bytes are **exactly the same** as `write_input` (same body),
    ///   and the only difference is that it **returns the observation output instead of swallowing it**. The control channel relay (ingress::handle_send)
    ///   builds the delivery observation record from this output (the premise of telling "send failure" vs "ignored by the model" — ADR-0088).
    ///
    /// ★Completeness = Ok-vs-Err★: when `send_input` returns `Ok(())`, the requested bytes were **fully accepted**
    ///   (partial acceptance is not hidden as `Ok`). Non-full acceptance is returned by this function as `Err`, not shrunk into
    ///   `Ok`. The byte fields of `WriteOutcome` are not the completeness decision lever (reasons in the
    ///   `WriteOutcome` comment) — completeness is seen through this function's `Ok`/`Err`.
    ///
    /// ★★The meaning of that 「acceptance」 changed — the old sentence's 「inner `write_all`」 and 「std write_all contract」 are now
    ///   false★★: all three links (PTY · stdio · codex) put input into **a bounded input queue** that a dedicated writer thread pops and
    ///   writes. So `Ok` is not 「it went to the child」 but **「the whole thing was accepted with its order settled」**, and
    ///   ★a write that fails after being accepted does not come back in this return value★ — where it appears is **the `Err` of the next
    ///   call** and the log, and usually the terminal transition that follows soon after.
    /// ★The effect on ADR-0088's delivery observation (precisely)★: of 「send failure vs ignored by the model」, which that record told apart,
    ///   **the range 「send failure」 catches shrank** — now that place catches 「not even accepted」 (the link closed or the cap
    ///   was exceeded), and 「accepted but could not write」 is reported as `Err` by **the next** delivery. The canonical contract is the
    ///   module header of [`crate::transport::input_queue`].
    /// Origin = `Mail` (mail delivery · non-human callers) — it does not go onto the waiting list even mid-turn.
    // ADR-0088
    // ADR-0231
    pub fn write_input_observed(&self, bytes: &[u8]) -> Result<WriteOutcome, PtyError> {
        self.write_input_from(bytes, InputOrigin::Mail)
    }

    /// The write that sends now — the body of today's path. `turn_origin` = if `Some`, hands it to the link turn verb with the origin
    /// (`TransportOwned`); if `None`, it is `send_input(Raw)`. The bytes the two branches hand over are the same.
    /// ★The `Some` branch emits no synthetic echo — even if the encoder declares an echo★: the bubbles and list events of that branch are
    ///   classified and emitted by the link (idle = synthetic bubble · otherwise = `Queued`). If the session emitted one more copy, text standing on the list
    ///   would also be drawn as a bubble.
    fn write_now(
        &self,
        bytes: &[u8],
        turn_origin: Option<InputOrigin>,
    ) -> Result<WriteOutcome, PtyError> {
        // ADR-0226: a write that opens a turn is counted **before** sending — the session id is persisted before the first turn reaches the other side.
        //   The `Some` branch (`TransportOwned`) takes only the user-exit refusal and does not count (that function's doc).
        self.count_turn_submission(bytes)?;
        // ★The message uuid of this user turn (replay dedup key)★: one is generated per write_input and passed as **the same value to both**
        //   (a) the stdin user line (encode) and (b) the input-time synthetic echo (input_echo_event).
        //   In json mode claude echoes this uuid back as is on replay (measured). The session knows only an opaque Uuid
        //   token and does not know the json shape or where the uuid is attached (ADR-0004 isolation — schema knowledge is
        //   backend/claude/ alone). The Raw (terminal) encoder ignores this uuid.
        let msg_uuid = uuid::Uuid::new_v4();
        let encoded = self.encoder.encode(bytes, msg_uuid);
        match turn_origin {
            None => self.transport.send_input(InputEvent::Raw(encoded))?,
            // ADR-0231: the id the link uses for list events = this write's `msg_uuid` — the receipt and the list point at the same value.
            Some(origin) => self.transport.send_turn(TurnInput {
                id: msg_uuid.to_string(),
                body: encoded,
                origin,
            })?,
        }

        // ★ADR-0044/0045 · why: the input-time user echo★: for the terminal (Raw) the PTY echoes input locally right away, but
        //   in json (stream-json) mode the user message does not appear on screen until claude echoes it back with `--replay-user-messages`
        //   (round-trip delay). So **after** send_input succeeds, if the encoder is json mode, the same
        //   user event is core.emitted right away to imitate the terminal's immediate echo (perceived responsiveness). The replay duplicate claude
        //   echoes back later is deduped by uuid in the frontend accumulator (same msg_uuid) — the decoder does not
        //   suppress it but passes it through carrying the uuid (backend/claude/). Past/non-matching-uuid user
        //   text (resumed portions) is not deduped and is all preserved (vanish regression removed).
        //   ★Lock discipline (ADR-0006)★: reuses core.emit without a new lock — it rides the discipline where emit takes the replay/subscribers locks only briefly
        //   and sends outside the lock. It emits after send_input succeeds, so the order is natural too.
        // ADR-0231: the bubble of text handed to the link turn verb is emitted by the link (this function's doc).
        if turn_origin.is_none() {
            if let Some(event) = self.encoder.input_echo_event(bytes, msg_uuid) {
                self.core.emit(event);
            }
        }
        let n = bytes.len();
        Ok(WriteOutcome {
            bytes_requested: n,
            bytes_written: n,
            msg_uuid,
            epoch: self.epoch,
        })
    }

    /// `write_input_observed` + **submit**: after writing the body, if the encoder requires submit bytes, it emits them once more
    /// **as a separate write**. The return value, user echo and byte accounting are the same as `write_input_observed`
    /// (the submit bytes are not a logical message, so they are not counted in `WriteOutcome`).
    ///
    /// ★Why it writes twice · why the encoder cannot merge them★ = the `InputEncoder::submit_sequence` comment (measured grounds).
    /// ★It sleeps `backend::SUBMIT_PACING` between the two writes★: splitting alone is not enough; without a gap
    ///   they are bundled into one chunk in the PTY and the receiver gets them in one read (that constant's doc — the old observation "it works with 0ms"
    ///   was a measurement error). **So this verb holds the calling thread that long.**
    /// ★★That gap is measured from **when the body actually went out** — not from when it was queued★★: since `send_input`
    ///   became queueing, measuring from the queueing time shrinks the actual gap to `wait - time for the body to go out`,
    ///   and if that time is longer than the wait it **becomes 0** (the two writes are bundled into one chunk and the submit fails). So the body
    ///   first confirms the body's OS landing with `confirm_written` and then sleeps.
    /// ★At one point this said 「that condition means the child does not read stdin, so the submit would not work anyway」 —
    ///   **that excuse was wrong**★: a body larger than the pipe/ConPTY buffer (4–64KiB) takes several write cycles even against
    ///   **a child reading perfectly well**, and mail delivery sending large envelopes is exactly that path. That is, the gap collapses not on a stuck agent
    ///   but on **normal delivery**. And the gap was **always** shrunk by the `body write
    ///   time` even before crossing that threshold — the direction was wrong regardless of size.
    /// ★Why this layer★: this is the lowest layer that owns the transport and can issue `send_input` twice. The upper layers
    ///   (manager · daemon adapter · messaging kernel) do not know the transport, and the lower layer (encoder) has a contract returning one byte sequence,
    ///   so it cannot make a write boundary.
    /// ★Why it is split from `write_input`★: for terminal keystrokes a person types Enter directly — if that path took this verb,
    ///   a turn would be submitted on every keystroke. This verb is only for callers that "put one completed message in as a turn"
    ///   (mail delivery).
    /// ★Error contract★: if the body went but the submit write failed, it is `Err` — a delivery whose turn did not start is not a delivery,
    ///   so the upper layer (parking · retry) must treat it as a failure.
    pub fn submit_input_observed(&self, bytes: &[u8]) -> Result<WriteOutcome, PtyError> {
        let outcome = self.write_input_observed(bytes)?;
        if let Some(submit) = self.encoder.submit_sequence() {
            // ★★The wait is measured from **when the body actually went out** — this one line makes the wait below meaningful★★:
            //   `send_input` now queues and returns immediately, so measuring from the queueing time shrinks the actual gap
            //   to `wait - time for the body to go out`, and if that time is longer than the wait it **becomes 0.**
            //   Then the two writes are bundled into one chunk, the receiver gets them in one read and **the turn is not submitted**
            //   (the defect `backend::SUBMIT_PACING` recorded by measurement). What must be widened is not our call gap
            //   but **the receiver's read boundary**, so the reference point must be the time it went out to the OS.
            self.confirm_written().map_err(|e| {
                tracing::warn!(
                    agent = %self.id,
                    epoch = self.epoch,
                    bytes = bytes.len(),
                    "body write confirmation failed — not attempting the submit: {e}"
                );
                e
            })?;
            // ★This wait is part of the submit (without it the submit fails — measured)★: the grounds, value source and why the old observation
            //   "it works with 0ms" was wrong are in the `backend::SUBMIT_PACING` doc.
            (self.sleeper)(self.submit_pacing);
            // ADR-0226: ★counted right before the submit CR — do not move it to the very front★. What opens the turn is this CR,
            //   and it goes out without passing through `write_input_observed`. Counting at the very front would put the body write, the landing confirmation and
            //   the wait between persistence and the turn, and a kill or failure in that window would leave **persistence without a turn**.
            self.count_turn_submission(submit)?;
            //
            // ★The two failures are told apart in the log (even the body did not go vs the body went and only the submit failed)★: the latter leaves an unsubmitted
            //   envelope in the receiver's input box, and the upper layer's lossless reparking overwrites the same envelope on top of it at the next flush
            //   (two copies in one turn). A verb that clears the input box is outside this layer's contract, so it cannot
            //   erase it, and leaving it so a person can recognize that leftover is all that can be done here.
            if let Err(e) = self.transport.send_input(InputEvent::Raw(submit.to_vec())) {
                tracing::warn!(
                    agent = %self.id,
                    epoch = self.epoch,
                    bytes = bytes.len(),
                    "the body was written but the submit write failed — an unsubmitted envelope remains in the receiver's input box and a retry will overwrite it: {e}"
                );
                return Err(e);
            }
        }
        // ★★The place that turns the meaning of the receipt from 「accepted」 back to 「went out」★★ — without this confirmation, a body that was only
        //   queued would be recorded as **delivery success**, and the two cases ADR-0088 wants to tell apart ("send failure" vs "ignored by the
        //   model") would be exactly inverted. Whether there is a submit path (terminal) or not (json), the end gathers here.
        // ★The confirmation includes even the submit bytes★: a delivery whose submit did not go out is a delivery whose turn did not start, and that is
        //   the state the `★Error contract★` above already set as `Err`.
        self.confirm_written().map_err(|e| {
            tracing::warn!(
                agent = %self.id,
                epoch = self.epoch,
                bytes = bytes.len(),
                "delivery write confirmation failed — not issuing a receipt: {e}"
            );
            e
        })?;
        Ok(outcome)
    }

    /// If this write opens a turn ([`InputEncoder::submits_turn`]), counts the submission in the session id latch — the caller calls it **before sending**
    /// that write, and does not send on `Err`. A no-op without a latch.
    ///
    /// ★`TransportOwned` is not counted — it takes only the user-exit refusal below★: the first input of that mode can be withdrawn with ✕,
    ///   so counting at send time would persist an id without a conversation. Submissions of that mode are counted by the link at the other side's echo of the first user message
    ///   through the latch's first-turn port ([`SessionIdLatch::first_turn_sink`]) — 「persist before the first turn」 does not exist for that mode.
    ///
    /// ★While the user is exiting, it neither counts nor sends (`Err`)★: counting and sending would persist an id without a conversation, and sending without
    ///   counting could hand a turn accepted before the input queue closed to a dying child, creating **a conversation of an unpersisted
    ///   id**. The outcome is the same `WriteFailed` as a `send_input` after the queue closed — it only moves that error
    ///   forward to the moment the exit intent stood.
    /// ★The check is one atomic read★ — no lock is taken. It checks even after the latch already finished persisting.
    // ADR-0226
    // ADR-0233
    fn count_turn_submission(&self, bytes: &[u8]) -> Result<(), PtyError> {
        let Some(latch) = &self.session_id_latch else {
            return Ok(());
        };
        if !self.encoder.submits_turn(bytes) {
            return Ok(());
        }
        if self.termination_intent() == TerminationIntent::UserKill {
            tracing::info!(
                agent = %self.id,
                epoch = self.epoch,
                bytes = bytes.len(),
                "a turn submission that came while the user is exiting, so it is not sent — the session id is not persisted either"
            );
            return Err(PtyError::WriteFailed(
                "the user is terminating this agent, so the turn was not sent".into(),
            ));
        }
        // ADR-0226 · ADR-0231: user decision — the id of this mode is counted by the link when it becomes real (the first echo).
        if matches!(self.mid_turn, MidTurnPolicy::TransportOwned) {
            return Ok(());
        }
        latch.note_submission();
        Ok(())
    }

    /// Confirms with the link whether accepted input actually went out. ★A link that **says it has no means** of confirming is
    /// passed through as is★ — those links (codex app-server · test-rig seam) had only that level of knowledge **since before** this change,
    /// and turning that into `Err` here would newly fail deliveries unrelated to what this change fixes.
    ///
    /// ★So this function treats two things **differently**★: 「checked and it did not go out」 (`WriteFailed`) is
    ///   a failure, and 「no means to check」 (`Unsupported`) is not. Folding the latter into a failure would look honest
    ///   but actually kills a healthy delivery.
    /// ★Known residual — exceeding the time limit is not 「did not go out」★: even if it returns `Err` past [`INPUT_FLUSH_BUDGET`],
    ///   the queued bytes **are not cancelled** and may go out late. Then the upper layer's reparking delivers the same
    ///   envelope once more (two copies in one turn). Removing that residual would require a cancel verb that takes that chunk out of the queue on timeout,
    ///   which is the separate question 「how far a partial delivery can be rolled back」.
    fn confirm_written(&self) -> Result<(), PtyError> {
        match self.transport.flush_input(INPUT_FLUSH_BUDGET) {
            Err(PtyError::Unsupported(_)) => Ok(()),
            other => other,
        }
    }

    /// Cancels one queued input of the list (the session side of the command `agent.cancelQueuedInput`).
    ///
    /// ★The registry check comes first★: an id not on the list (unknown · already has an outcome) = `NotFound` · already pending cancel = no event,
    ///   `Requested` (idempotent). A session that does not use the list (`None`) is always `NotFound` — it does not reach the link.
    /// Per-policy branches:
    ///   - `SessionClassified` — inside the input lock: check → `CancelRequested` → cancel line `send_input` →
    ///     `Requested`. User input is also written inside the same lock, so the cancel line for that id goes out **only after the text write returns**
    ///     (if the cancel overtook the text, no reservation would be made mid-turn and the text would be delivered while the screen believes the cancel).
    ///     ★The check and `CancelRequested` are one replay section★ (`OutputCore::emit_cancel_request`) — if the pump's outcome slips in
    ///     after the check, that entry answers `NotFound` and the cancel line is not written either. An outcome arriving after the record (a late acceptance etc.)
    ///     stays `Requested`, and the registry decides the outcome.
    ///     ★Cancel line write failure = emit `CancelFailed` and `Err(Write)`★ — the entry stays pending cancel as 「could not remove」 (the reason it is not
    ///     returned to waiting = the `QueuedInputEvent::CancelFailed` doc).
    ///   - `TransportOwned` — check without the lock → `withdraw`: `Withdrawn` = `Cancelled` · `TooLate` = `Requested`
    ///     · `NotHeld` = `NotFound` (an outcome happened between the check and the withdrawal — that outcome event soon stands in the registry). List
    ///     events are emitted by the link.
    /// ★Do not emit while holding the registry guard★ — the lock order is replay → registry (the `TransportOwned` check copies and releases right away,
    ///   and the `SessionClassified` check is done by the core under the replay lock).
    // ADR-0231
    pub fn cancel_queued_input(&self, id: &str) -> Result<CancelOutcome, CancelError> {
        match self.mid_turn {
            MidTurnPolicy::None => Err(CancelError::NotFound),
            MidTurnPolicy::SessionClassified { cancel_line } => {
                let _order = sync::lock(&self.input_order);
                match self.core.emit_cancel_request(id) {
                    CancelRequest::NotListed => return Err(CancelError::NotFound),
                    CancelRequest::AlreadyCancelling => return Ok(CancelOutcome::Requested),
                    CancelRequest::Recorded => {}
                }
                if let Err(e) = self.transport.send_input(InputEvent::Raw(cancel_line(id))) {
                    self.core
                        .emit(OutputEvent::QueuedInput(QueuedInputEvent::CancelFailed {
                            id: id.to_owned(),
                        }));
                    return Err(CancelError::Write(e));
                }
                Ok(CancelOutcome::Requested)
            }
            MidTurnPolicy::TransportOwned => {
                if let Some(answer) = self.answer_from_the_registry(id) {
                    return answer;
                }
                match self.transport.withdraw(id) {
                    Withdraw::Withdrawn => Ok(CancelOutcome::Cancelled),
                    Withdraw::TooLate => Ok(CancelOutcome::Requested),
                    Withdraw::NotHeld => Err(CancelError::NotFound),
                }
            }
        }
    }

    /// A cancellation answered by the registry alone — `None` = it stands on the list as `Queued` (goes to the policy branch).
    fn answer_from_the_registry(&self, id: &str) -> Option<Result<CancelOutcome, CancelError>> {
        let phase = self
            .core
            .queued_inputs()
            .lock()
            .rows()
            .iter()
            .find(|row| row.id == id)
            .map(|row| row.phase);
        match phase {
            None => Some(Err(CancelError::NotFound)),
            Some(RowPhase::Cancelling { .. }) => Some(Ok(CancelOutcome::Requested)),
            Some(RowPhase::Queued) => None,
        }
    }

    /// The session side of the list query — reads the registry rows and `as_of_seq` under one lock and, **after** releasing that lock, asks the link
    /// about acceptance-unknown and adds it to `Queued` rows. The marker is the link state at query time, so it is not one atom with the row snapshot (the next query fixes it).
    /// ★No link fills that marker today★ — codex is the default implementation too (empty list) (`AgentTransport::unconfirmed_inputs`).
    /// It does not take the input lock — it is a read.
    // ADR-0006
    // ADR-0231
    pub fn list_queued_inputs(&self) -> (Vec<ListedRow>, Option<u64>) {
        let (rows, as_of_seq) = self.core.queued_inputs().snapshot();
        let unconfirmed = self.transport.unconfirmed_inputs();
        (overlay_unconfirmed(rows, &unconfirmed), as_of_seq)
    }

    /// Updates the cols/rows atomics only after transport.resize succeeds — on failure the old values are kept.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        self.transport.resize(cols, rows)?;
        self.cols.store(cols, Ordering::Relaxed);
        self.rows.store(rows, Ordering::Relaxed);
        Ok(())
    }

    /// Interrupts only the work in progress (≠ kill — the process stays alive). The link decides how to interrupt — PTY is `Unsupported`
    /// (in terminal mode the terminal takes keys directly · ADR-0245).
    pub fn interrupt(&self) -> Result<(), PtyError> {
        self.transport.interrupt()
    }

    /// Notifies the link of the retreat ([`AgentTransport::begin_retire`]) — does not collect resources (that is [`Self::kill`]'s job).
    pub(crate) fn begin_retire(&self) {
        self.transport.begin_retire();
    }

    /// Forced resource teardown, then waiting for the pump. **This two-verb order (shutdown THEN
    /// join_pump) is the core of the kill causality**: shutdown destroys the transport's resources so the
    /// pump's read wakes and ends (→ `core.finish(Killed)`), and join_pump waits for that end. Reversed, it
    /// hangs (waiting on a pump that is still alive).
    ///
    /// Only the join is bounded (`timeout`); `shutdown` has no bound of its own. For a PTY transport on
    /// Windows it returns because a running pump keeps draining after shutdown until the master is
    /// closed — otherwise `ClosePseudoConsole` can block (rule = `transport::pty::run_pump`; with no live
    /// pump, e.g. after a contained pump panic, the close can still block — S34 TRD §3B residual).
    // ADR-0001
    pub fn kill(&self, timeout: Duration) {
        self.transport.shutdown();
        self.core.join_pump(timeout);
    }

    /// Transitional Exiting transition — the manager calls it first right before kill (stage 6). enter_exiting and kill are
    /// separate verbs. false if terminal (already exited).
    pub fn enter_exiting(&self) -> bool {
        self.core.enter_exiting()
    }

    /// Final capabilities — the composition of transport (physical: input/output/control) and backend (program:
    /// session/model). The sources are separated by type, so the accident of the transport filling resume or the backend filling resize
    /// is structurally impossible.
    pub fn capabilities(&self) -> Capabilities {
        Capabilities::compose(self.transport.capabilities(), self.backend_caps.clone())
    }

    pub fn subscribe(&self, sink: Arc<dyn OutputSink>) -> SinkId {
        self.core.subscribe(sink)
    }

    /// `on_ready`: called once right before the replay is sent (while holding the subscribers lock) — delegated to core (invariant 2/TOCTOU).
    ///
    /// Compares `requested_epoch` with **this session's** marker to decide whether to continue by seq. The response `on_ready` receives and
    /// the response returned are the same value — the marker, the resume marker and the replay all come from this one incarnation.
    // ADR-0226
    pub fn subscribe_from(
        &self,
        sink: Arc<dyn OutputSink>,
        after_seq: Option<u64>,
        requested_epoch: Option<u32>,
        on_ready: impl FnOnce(&SubscribeReply),
    ) -> SubscribeReply {
        let incarnation = Incarnation {
            epoch: self.epoch,
            continues_conversation: self.continues_conversation,
        };
        let epoch_matches = requested_epoch == Some(self.epoch);
        let outcome = self
            .core
            .subscribe_from(sink, after_seq, epoch_matches, |outcome| {
                on_ready(&SubscribeReply {
                    outcome: *outcome,
                    incarnation,
                })
            });
        SubscribeReply {
            outcome,
            incarnation,
        }
    }

    pub fn unsubscribe(&self, sink_id: SinkId) {
        self.core.unsubscribe(sink_id);
    }

    pub fn snapshot(&self) -> Vec<OutputChunk> {
        self.core.snapshot()
    }

    /// The last console bytes, at most `max_bytes` (contract and cost in `OutputCore::terminal_tail`).
    // ADR-0172
    pub fn terminal_tail(&self, max_bytes: usize) -> Vec<u8> {
        self.core.terminal_tail(max_bytes)
    }

    /// The tail of the **diagnostic (stderr) text** this incarnation emitted (contract and cap in `OutputCore::diagnostic_tail`).
    ///
    /// ★Not a substitute for `terminal_tail` but its pair★: depending on the transport the two streams fill exclusively
    ///   — for a pipe (structured) session this one fills and the ring is empty, for a PTY (terminal) session it is the opposite. So the classification
    ///   caller does not pick one of the two but passes them **merged**.
    // ADR-0172
    pub fn diagnostic_tail(&self) -> String {
        self.core.diagnostic_tail()
    }

    pub fn status(&self) -> AgentStatus {
        self.core.status()
    }

    pub fn cols(&self) -> u16 {
        self.cols.load(Ordering::Relaxed)
    }

    pub fn rows(&self) -> u16 {
        self.rows.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{AgentBackend, ClaudeBackend, ShellBackend};
    use crate::queued_input::{CancelAnswer, ListedState};
    use crate::transport::stdio::StdioTransport;
    use crate::types::{ControlCaps, InputCaps, OutputCaps, TransportCaps};
    use std::sync::Mutex;

    /// An isolation harness for asserting the encoding wiring without a real process (ADR-0012).
    /// `fail_from`: makes writes fail from this ordinal (0-based) on — it is for making the branch where the body succeeds and only the submit fails
    ///   (`None` = all succeed).
    struct CapturingTransport {
        captured: Arc<Mutex<Vec<Vec<u8>>>>,
        fail_from: Option<usize>,
    }
    impl AgentTransport for CapturingTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            let InputEvent::Raw(bytes) = input;
            let mut captured = self.captured.lock().unwrap();
            if self.fail_from.is_some_and(|n| captured.len() >= n) {
                return Err(PtyError::WriteFailed("harness: write refused".into()));
            }
            captured.push(bytes);
            Ok(())
        }
        fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            harness_caps()
        }
    }

    fn harness_caps() -> TransportCaps {
        TransportCaps {
            input: InputCaps {
                raw: true,
                message: false,
                attachment: false,
            },
            output: OutputCaps {
                terminal_bytes: true,
                structured: false,
                markdown: false,
                tool_events: false,
                usage: false,
            },
            control: ControlCaps {
                resize: false,
                interrupt: false,
                cancel: false,
                graceful_shutdown: false,
            },
        }
    }

    struct NoopStatusSink;
    impl crate::types::StatusSink for NoopStatusSink {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
        fn agent_list_updated(&self, _agents: Vec<crate::types::AgentInfo>) {}
    }

    /// A harness for asserting the input-time user echo emit (ADR-0044/0045) without a real process.
    /// Collected tags: `Structured` as `"structured:<kind>"`, others as the variant name.
    struct EmitCapturingSink {
        id: SinkId,
        seen: Arc<Mutex<Vec<String>>>,
    }
    impl OutputSink for EmitCapturingSink {
        fn send(
            &self,
            frame: crate::types::OutputFrame<'_>,
        ) -> Result<(), crate::types::SinkError> {
            use crate::types::{OutputEvent, OutputPayload};
            if let OutputPayload::Event(e) = frame.payload {
                let tag = match e {
                    OutputEvent::Structured { kind, .. } => format!("structured:{kind}"),
                    other => format!("{other:?}"),
                };
                self.seen.lock().unwrap().push(tag);
            }
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    fn session_with(encoder: InputEncoder) -> (AgentSession, Arc<Mutex<Vec<Vec<u8>>>>) {
        session_harness(encoder, None)
    }

    fn session_failing_after(
        encoder: InputEncoder,
        ok_writes: usize,
    ) -> (AgentSession, Arc<Mutex<Vec<Vec<u8>>>>) {
        session_harness(encoder, Some(ok_writes))
    }

    fn session_harness(
        encoder: InputEncoder,
        fail_from: Option<usize>,
    ) -> (AgentSession, Arc<Mutex<Vec<Vec<u8>>>>) {
        let id = uuid::Uuid::new_v4();
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatusSink),
            crate::output_core::TurnWiring::detached(),
        ));
        let captured = Arc::new(Mutex::new(Vec::new()));
        let transport = Box::new(CapturingTransport {
            captured: captured.clone(),
            fail_from,
        });
        let shell_cmd = crate::profile::AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let session = AgentSession::new(
            id,
            PathBuf::from("."),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            ShellBackend.capabilities(&shell_cmd),
            encoder,
            true,
            core,
            transport,
        )
        // The harness does not sleep — **whether the wait was issued** is asserted through sleeper calls by the dedicated test below (without time
        //   measurement). Actually sleeping here would hold the unit tests 0.5 seconds per call.
        .with_submit_pacing(Duration::ZERO, |_| {});
        (session, captured)
    }

    // ── Raw encoder (terminal path regression invariant) ──
    #[test]
    fn write_input_raw_is_byte_identical() {
        let (session, captured) = session_with(InputEncoder::Raw);
        let input = b"echo hi\r\n\x1b[A\x03";
        session.write_input(input).unwrap();
        let got = captured.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], input.to_vec(), "Raw must be byte-identical");
    }

    // ── ADR-0088: delivery-boundary measurement ──
    #[test]
    fn write_input_observed_surfaces_bytes_and_msg_uuid() {
        let (session, captured) = session_with(InputEncoder::Raw);
        let input = b"hello-observed"; // 14 bytes ASCII.
        let outcome = session
            .write_input_observed(input)
            .expect("write_input_observed ok");
        // ★FIX-5★: exact count assertions that filter off-by-one and layer regressions.
        assert_eq!(
            outcome.bytes_requested, 14,
            "requested bytes = the exact byte count of the passed input"
        );
        assert_eq!(
            outcome.bytes_requested,
            input.len(),
            "requested = input len"
        );
        assert_eq!(
            outcome.bytes_written, outcome.bytes_requested,
            "by-construction identity (bytes_written = copy of bytes_requested) — not short-write detection"
        );
        assert!(
            !outcome.msg_uuid.is_nil(),
            "must expose this user turn's msg_uuid (correlation key)"
        );
        assert_eq!(
            outcome.epoch, 0,
            "WriteOutcome.epoch = the epoch of the session that performed the write (by-construction)"
        );
        // The instrumented version does not break Raw byte identity.
        assert_eq!(captured.lock().unwrap()[0], input.to_vec());
    }

    // ── ADR-0088 (FIX-5): multibyte (UTF-8) body ──
    #[test]
    fn write_input_observed_counts_bytes_not_chars_multibyte() {
        // "안녕" = 2 Hangul characters, 3 bytes each in UTF-8 = 6 bytes. Counting chars (2) breaks here. // hangul-ok
        let (session, _captured) = session_with(InputEncoder::Raw);
        let input = "안녕".as_bytes(); // hangul-ok
        assert_eq!(input.len(), 6, "must be 6 bytes in UTF-8 (test premise)");
        let outcome = session
            .write_input_observed(input)
            .expect("write_input_observed ok");
        assert_eq!(
            outcome.bytes_requested, 6,
            "a multibyte request must be the byte count (6), not the char count (2)"
        );
        assert_eq!(
            outcome.bytes_written, 6,
            "the by-construction copy is also the byte count (6)"
        );
    }

    // ── Submit split (submit_input_observed) ──
    //
    // ★The regression axis is not bytes but **the write boundary**★: the envelope bytes reached the PTY before too, but the turn did not
    //   start. Combining `body+CR` into one write makes the claude TUI not submit, and splitting it into two writes submits
    //   (measured 2026-08-17). So the assertions below look at the **number and boundaries** of writes — not the concatenated bytes.

    #[test]
    fn submit_input_terminal_writes_the_body_then_the_submit_byte_separately() {
        let (session, captured) = session_with(InputEncoder::Raw);
        let body = b"<message from=\"bob\">hi</message>";

        let outcome = session
            .submit_input_observed(body)
            .expect("submit_input_observed ok");

        let got = captured.lock().unwrap();
        assert_eq!(
            got.len(),
            2,
            "the body and the submit must be separate writes (combined into one write the TUI does not submit): {got:?}"
        );
        assert_eq!(
            got[0],
            body.to_vec(),
            "the body write is the envelope bytes as is — no submit bytes mixed in"
        );
        assert_eq!(
            got[1],
            b"\r".to_vec(),
            "second write = the submit (CR) alone"
        );
        assert_eq!(
            outcome.bytes_requested,
            body.len(),
            "accounting counts only logical message bytes — submit bytes are not counted"
        );
        assert_eq!(outcome.bytes_written, body.len());
    }

    #[test]
    fn submit_input_json_mode_writes_exactly_the_encoded_line_and_nothing_else() {
        let (session, captured) = session_with(InputEncoder::ClaudeStreamJson);
        let body = b"hello";

        let outcome = session
            .submit_input_observed(body)
            .expect("submit_input_observed ok");

        // ★exact comparison★: rebuilds the expected line with the returned msg_uuid and checks **an exact byte match**. Every regression that
        //   adds a submit write or appends characters to the envelope is caught here (unlike weak assertions such as "there is no CR",
        //   it catches whatever is added). Why it does not compare identity with `write_input` directly = that one draws its own
        //   new msg_uuid, so the two outputs differ by the uuid.
        let expected = InputEncoder::ClaudeStreamJson.encode(body, outcome.msg_uuid);
        let got = captured.lock().unwrap();
        assert_eq!(
            got.len(),
            1,
            "the json path is one write as before — adding a submit write creates one more empty turn holding only a CR: {got:?}"
        );
        assert_eq!(
            got[0],
            expected,
            "must match the encoder output byte for byte: {}",
            String::from_utf8_lossy(&got[0])
        );
    }

    /// ★Pins down both whether the wait **is actually issued** and what the production default is★.
    ///
    /// The body of this defect was not the number of writes but the receiver's read boundary — even when split, without a gap they are bundled
    /// into one chunk and not submitted (`backend::SUBMIT_PACING` doc). So both "was there a wait before the submit write" and
    /// "is that value the production default" are regression axes. It asserts by recording sleeper calls without measuring time, so it is
    /// not flaky, and the test does not actually sleep 0.5 seconds.
    ///
    /// ★★It **states clearly what this entry cannot catch** — that is why there is a paired entry★★: the oracle here is only
    ///   「was the wait called」, so **it lets through a defect that measures the wait from the wrong point.** After the queue came in, that defect
    ///   actually happened — measuring the wait from the queueing time eats the gap while the body goes out and makes it 0. What catches that
    ///   branch is the entry looking at **the physical read boundary**, which needs a real pipe and is in the integration test rig:
    ///   in `tests/submit_delivery_boundary.rs`
    ///   `submit_byte_lands_in_its_own_read_even_when_the_body_takes_many_write_cycles`.
    ///   ★Do not keep only one of the two★ — this entry carries the constant and the call, that entry the actual gap the call makes.
    #[test]
    fn submit_input_waits_between_the_body_and_the_submit_write() {
        use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
        // The record board for this test only — other tests' harnesses use the no-op sleeper, so they do not reach here.
        static SLEPT_MICROS: AtomicU64 = AtomicU64::new(0);
        static CALLS: AtomicU64 = AtomicU64::new(0);
        fn recording_sleep(d: Duration) {
            SLEPT_MICROS.store(d.as_micros() as u64, AtomicOrdering::SeqCst);
            CALLS.fetch_add(1, AtomicOrdering::SeqCst);
        }

        let (session, captured) = session_harness(InputEncoder::Raw, None);
        // ★Keep the production default as is and change only the sleeping function★: if the test also decided the value, it could not see a default regression.
        let session = session.with_submit_pacing(crate::backend::SUBMIT_PACING, recording_sleep);

        session
            .submit_input_observed(b"envelope")
            .expect("submit ok");

        assert_eq!(
            CALLS.load(AtomicOrdering::SeqCst),
            1,
            "a wait must be issued exactly once between the body and the submit (without it, the two writes are bundled into one read)"
        );
        assert_eq!(
            SLEPT_MICROS.load(AtomicOrdering::SeqCst),
            crate::backend::SUBMIT_PACING.as_micros() as u64,
            "wait value = the production default (shrinking it brings this defect back — read the grounds in the constant's doc first)"
        );
        assert!(
            crate::backend::SUBMIT_PACING > Duration::ZERO,
            "★a shape where the production default becomes 0 is forbidden★ — at 0 the point of splitting the writes is lost"
        );
        assert_eq!(
            captured.lock().unwrap().len(),
            2,
            "even with the wait, the writes are still two: body + submit"
        );
    }

    #[test]
    fn submit_input_reports_a_failure_that_only_hit_the_submit_write() {
        // The branch where the body has already gone into the receiver's input box and only the submit fails — for the upper layer to handle it by reparking,
        // it must not be swallowed as success (the meaning of that leftover is in the submit_input_observed doc).
        let (session, captured) = session_failing_after(InputEncoder::Raw, 1);

        let err = session.submit_input_observed(b"envelope");

        assert!(
            matches!(err, Err(PtyError::WriteFailed(_))),
            "a submit write failure must surface as Err: {err:?}"
        );
        assert_eq!(
            captured.lock().unwrap().as_slice(),
            &[b"envelope".to_vec()],
            "the body has already gone out — so a leftover remains that a retry will overwrite with the same envelope"
        );
    }

    #[test]
    fn write_input_does_not_submit() {
        // The keystroke path where a person types Enter directly — if a submit slipped in here, a turn would go out on every keystroke.
        let (session, captured) = session_with(InputEncoder::Raw);
        session.write_input(b"partial").unwrap();
        let got = captured.lock().unwrap();
        assert_eq!(got.len(), 1, "write_input stays one write: {got:?}");
        assert_eq!(got[0], b"partial".to_vec());
    }

    // ── ADR-0088: surfacing failures ──
    #[test]
    fn write_input_observed_surfaces_transport_error() {
        struct FailingTransport;
        impl AgentTransport for FailingTransport {
            fn start(&self, _core: Arc<OutputCore>) {}
            fn send_input(&self, _input: InputEvent) -> Result<(), PtyError> {
                Err(PtyError::WriteFailed("stdin closed".into()))
            }
            fn resize(&self, _c: u16, _r: u16) -> Result<(), PtyError> {
                Ok(())
            }
            fn interrupt(&self) -> Result<(), PtyError> {
                Ok(())
            }
            fn shutdown(&self) {}
            fn capabilities(&self) -> crate::types::TransportCaps {
                use crate::types::{ControlCaps, InputCaps, OutputCaps, TransportCaps};
                TransportCaps {
                    input: InputCaps {
                        raw: true,
                        message: false,
                        attachment: false,
                    },
                    output: OutputCaps {
                        terminal_bytes: true,
                        structured: false,
                        markdown: false,
                        tool_events: false,
                        usage: false,
                    },
                    control: ControlCaps {
                        resize: false,
                        interrupt: false,
                        cancel: false,
                        graceful_shutdown: false,
                    },
                }
            }
        }
        let id = uuid::Uuid::new_v4();
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatusSink),
            crate::output_core::TurnWiring::detached(),
        ));
        let shell_cmd = crate::profile::AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let session = AgentSession::new(
            id,
            PathBuf::from("."),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            ShellBackend.capabilities(&shell_cmd),
            InputEncoder::Raw,
            true,
            core,
            Box::new(FailingTransport),
        );
        let err = session.write_input_observed(b"x");
        assert!(
            matches!(err, Err(PtyError::WriteFailed(_))),
            "a send_input failure must surface as Err (not swallowed as success): {err:?}"
        );
    }

    // ── ClaudeStreamJson encoder (ADR-0044) ──
    #[test]
    fn write_input_json_mode_wraps_as_stream_json_line() {
        let (session, captured) = session_with(InputEncoder::ClaudeStreamJson);
        session.write_input(b"hello").unwrap();
        let got = captured.lock().unwrap();
        assert_eq!(got.len(), 1);
        let line = &got[0];
        assert_eq!(*line.last().unwrap(), b'\n', "line terminator \\n");
        let s = String::from_utf8(line.clone()).unwrap();
        assert!(s.contains("\"type\":\"user\""), "user turn schema: {s}");
        assert!(s.contains("\"text\":\"hello\""), "text preserved: {s}");
    }

    // ── ADR-0044/0045: input-time user echo ──────────
    #[test]
    fn write_input_json_mode_emits_input_time_user_echo() {
        let (session, _captured) = session_with(InputEncoder::ClaudeStreamJson);
        let seen = Arc::new(Mutex::new(Vec::new()));
        session.subscribe(Arc::new(EmitCapturingSink {
            id: uuid::Uuid::new_v4(),
            seen: seen.clone(),
        }));

        session.write_input("hello claude".as_bytes()).unwrap();

        let got = seen.lock().unwrap();
        assert_eq!(
            *got,
            vec!["structured:user".to_string()],
            "json-mode write_input must emit 1 input-time user echo"
        );
    }

    #[test]
    fn write_input_terminal_mode_does_not_emit_user_echo() {
        // Raw (terminal · shell) already has the PTY local echo, so emitting a synthetic echo would duplicate → emits nothing.
        let (session, _captured) = session_with(InputEncoder::Raw);
        let seen = Arc::new(Mutex::new(Vec::new()));
        session.subscribe(Arc::new(EmitCapturingSink {
            id: uuid::Uuid::new_v4(),
            seen: seen.clone(),
        }));

        session.write_input(b"echo hi\r\n").unwrap();

        assert!(
            seen.lock().unwrap().is_empty(),
            "the terminal (Raw) path must not emit the input-time user echo (prevents duplicating the PTY echo)"
        );
    }

    // ── json-mode session caps: StdioTransport ⊕ ClaudeBackend composition ──
    #[cfg(windows)]
    #[test]
    fn json_mode_session_caps_are_structured() {
        let id = uuid::Uuid::new_v4();
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatusSink),
            crate::output_core::TurnWiring::detached(),
        ));
        let spec = crate::types::CommandSpec {
            program: "cmd.exe".into(),
            args: vec!["/c".into(), "echo probe".into()],
            env: vec![],
            cwd: PathBuf::from("."),
        };
        // json mode = structured carrier → structured=true injected into StdioTransport (in production the backend building that link
        //   puts in the same value — ADR-0191).
        let (transport, _pid) = StdioTransport::open(&spec, true, None).expect("open");
        // json-mode command — the backend looks at it to compute caps.
        let json_cmd = crate::profile::AgentCommand::Claude {
            extra_args: vec![],
            output_format: crate::profile::AgentOutputFormat::StreamJson,
        };
        let session = AgentSession::new(
            id,
            PathBuf::from("."),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            // In json mode the backend is still ClaudeBackend (resume/model are the program's concern, ADR-0030).
            ClaudeBackend.capabilities(&json_cmd),
            InputEncoder::ClaudeStreamJson,
            true,
            core,
            Box::new(transport),
        );
        let caps = session.capabilities();
        assert!(caps.output.structured, "json session → structured output");
        assert!(!caps.output.terminal_bytes, "not terminal bytes");
        assert!(!caps.control.resize, "cannot resize");
        assert!(
            !caps.control.interrupt,
            "a link with no interrupt line injected = no interrupt capability (in production, claude JSON gets it injected by the backend — ADR-0238)"
        );
        // ★ADR-0044 follow-up done★: json mode also supports --resume (spike-verified, claude 2.1.170) → resume=true.
        //   build_spec emits --resume in SpawnMode::Resume and reuses the controlled sid (ADR-0008), so there is no sid
        //   collision.
        assert!(
            caps.session.resume,
            "json-mode session → resume=true (--resume supported, spike-verified)"
        );
        session.kill(Duration::from_secs(5));
    }

    // ── Session id first-submission latch — where the two input verbs count (ADR-0226) ──
    //
    // ★The axis measured is order★: the link's writes and the latch commit port calls are put into **one event record**, so 「commit before the write opening the turn」
    //   is seen by event order, not by bytes.

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Ev {
        Commit(String),
        Send(Vec<u8>),
    }

    /// `fail_from`: refuses writes from this ordinal (0-based) on. `flush_fails`: the landing confirmation fails.
    struct EventTransport {
        events: Arc<Mutex<Vec<Ev>>>,
        fail_from: Option<usize>,
        flush_fails: bool,
    }
    impl AgentTransport for EventTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            let InputEvent::Raw(bytes) = input;
            let mut events = self.events.lock().unwrap();
            let sent = events.iter().filter(|e| matches!(e, Ev::Send(_))).count();
            if self.fail_from.is_some_and(|n| sent >= n) {
                return Err(PtyError::WriteFailed("harness: write refused".into()));
            }
            events.push(Ev::Send(bytes));
            Ok(())
        }
        fn flush_input(&self, _timeout: Duration) -> Result<(), PtyError> {
            if self.flush_fails {
                Err(PtyError::WriteFailed("harness: never landed".into()))
            } else {
                Ok(())
            }
        }
        fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            harness_caps()
        }
    }

    fn recording_port(events: &Arc<Mutex<Vec<Ev>>>) -> crate::backend::SessionIdSink {
        let events = events.clone();
        Arc::new(move |raw: &str| events.lock().unwrap().push(Ev::Commit(raw.to_owned())))
    }

    /// A session carrying a latch. The latch **already** holds the id (offered where the latch is made, like claude) —
    /// so the first submission commits on the spot, and the order of submission and commit is visible in the event record.
    fn latched_session_with(
        encoder: InputEncoder,
        fail_from: Option<usize>,
        flush_fails: bool,
        events: &Arc<Mutex<Vec<Ev>>>,
        port: crate::backend::SessionIdSink,
    ) -> AgentSession {
        let id = uuid::Uuid::new_v4();
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatusSink),
            crate::output_core::TurnWiring::detached(),
        ));
        let shell_cmd = crate::profile::AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let latch = SessionIdLatch::new(id, 0, port);
        latch.offer("sid-1");
        AgentSession::new(
            id,
            PathBuf::from("."),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            ShellBackend.capabilities(&shell_cmd),
            encoder,
            true,
            core,
            Box::new(EventTransport {
                events: events.clone(),
                fail_from,
                flush_fails,
            }),
        )
        .with_submit_pacing(Duration::ZERO, |_| {})
        .with_session_id_latch(latch)
    }

    fn latched(encoder: InputEncoder) -> (AgentSession, Arc<Mutex<Vec<Ev>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let session = latched_session_with(encoder, None, false, &events, recording_port(&events));
        (session, events)
    }

    fn commits(events: &Arc<Mutex<Vec<Ev>>>) -> usize {
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, Ev::Commit(_)))
            .count()
    }

    #[test]
    fn a_key_fragment_without_cr_is_not_counted() {
        let (session, events) = latched(InputEncoder::Raw);
        session.write_input(b"hel").expect("write");
        assert_eq!(*events.lock().unwrap(), vec![Ev::Send(b"hel".to_vec())]);
    }

    #[test]
    fn a_key_fragment_with_cr_is_counted_before_it_is_sent() {
        let (session, events) = latched(InputEncoder::Raw);
        session.write_input(b"hi\r").expect("write");
        assert_eq!(
            *events.lock().unwrap(),
            vec![Ev::Commit("sid-1".into()), Ev::Send(b"hi\r".to_vec())],
            "persistence must come before the turn"
        );

        session.write_input(b"again\r").expect("write");
        assert_eq!(
            commits(&events),
            1,
            "the second submission does not commit again"
        );
    }

    #[test]
    fn a_json_turn_is_counted_before_it_is_sent() {
        let (session, events) = latched(InputEncoder::ClaudeStreamJson);
        session.write_input(b"hello").expect("write");
        let got = events.lock().unwrap();
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(
            got[0],
            Ev::Commit("sid-1".into()),
            "persistence before the turn: {got:?}"
        );
        assert!(matches!(got[1], Ev::Send(_)), "{got:?}");
    }

    /// ★`Raw` mail counts after the body, after the wait, right before the submit CR★ — counting at the very front would put the body write, the landing confirmation and the wait
    ///   between persistence and the turn, and a failure or kill in that window would leave persistence without a turn.
    #[test]
    fn raw_mail_is_counted_after_the_body_and_the_pacing_right_before_the_submit_cr() {
        use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
        // The record board for this test only — at the moment of commit it reads whether the wait was issued before the commit.
        static SLEPT: AtomicBool = AtomicBool::new(false);
        fn mark_slept(_d: Duration) {
            SLEPT.store(true, AtomicOrdering::SeqCst);
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let port_events = events.clone();
        let port: crate::backend::SessionIdSink = Arc::new(move |raw: &str| {
            let slept = SLEPT.load(AtomicOrdering::SeqCst);
            port_events
                .lock()
                .unwrap()
                .push(Ev::Commit(format!("{raw} slept={slept}")));
        });
        let session = latched_session_with(InputEncoder::Raw, None, false, &events, port)
            .with_submit_pacing(Duration::ZERO, mark_slept);

        session.submit_input_observed(b"envelope").expect("submit");

        assert_eq!(
            *events.lock().unwrap(),
            vec![
                Ev::Send(b"envelope".to_vec()),
                Ev::Commit("sid-1 slept=true".into()),
                Ev::Send(b"\r".to_vec()),
            ]
        );
    }

    /// The JSON encoder has no submit CR — the body is itself the turn, so it counts **before** the body.
    #[test]
    fn json_mail_is_counted_before_the_body() {
        let (session, events) = latched(InputEncoder::ClaudeStreamJson);
        session.submit_input_observed(b"hello").expect("submit");
        let got = events.lock().unwrap();
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0], Ev::Commit("sid-1".into()), "{got:?}");
        assert!(matches!(got[1], Ev::Send(_)), "{got:?}");
    }

    #[test]
    fn raw_mail_whose_body_write_fails_is_not_counted() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let session = latched_session_with(
            InputEncoder::Raw,
            Some(0),
            false,
            &events,
            recording_port(&events),
        );
        assert!(session.submit_input_observed(b"envelope").is_err());
        assert_eq!(
            commits(&events),
            0,
            "a delivery where even the body did not go left a persistence"
        );
    }

    #[test]
    fn raw_mail_whose_body_never_lands_is_not_counted() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let session = latched_session_with(
            InputEncoder::Raw,
            None,
            true,
            &events,
            recording_port(&events),
        );
        assert!(session.submit_input_observed(b"envelope").is_err());
        assert_eq!(
            *events.lock().unwrap(),
            vec![Ev::Send(b"envelope".to_vec())],
            "if the landing confirmation fails, there is neither a submit CR nor persistence"
        );
    }

    /// The default session without a latch is a no-op for both counting and the user-exit check — behavior unchanged until production carries a latch.
    #[test]
    fn a_session_without_a_latch_is_untouched_even_during_a_user_kill() {
        let (session, captured) = session_with(InputEncoder::Raw);
        session.set_intent(TerminationIntent::UserKill);

        session
            .write_input(b"hi\r")
            .expect("without a latch it sends as today");
        session
            .submit_input_observed(b"envelope")
            .expect("without a latch it delivers as today");

        assert_eq!(
            *captured.lock().unwrap(),
            vec![b"hi\r".to_vec(), b"envelope".to_vec(), b"\r".to_vec()]
        );
    }

    fn assert_user_kill_refusal(result: Result<WriteOutcome, PtyError>) {
        match result {
            Err(PtyError::WriteFailed(msg)) => assert!(
                msg.contains("terminating") && !msg.contains("overwrite"),
                "must be its own text stating the user exit (no reuse of the submit write failure text): {msg}"
            ),
            other => panic!("a turn submission during user exit must be WriteFailed: {other:?}"),
        }
    }

    /// ★While the user is exiting, it neither counts nor sends★ — counting and sending would persist an id without a conversation, and sending without
    ///   counting could create a conversation of an unpersisted id.
    #[test]
    fn a_user_kill_refuses_turn_submissions_without_counting_them() {
        // Terminal keystrokes: a fragment containing CR does not go out at all, and a fragment without CR goes out as is.
        let (session, events) = latched(InputEncoder::Raw);
        session.set_intent(TerminationIntent::UserKill);
        assert_user_kill_refusal(session.write_input_observed(b"hi\r"));
        session
            .write_input(b"typing")
            .expect("a keystroke that does not open a turn is not a place to count");
        assert_eq!(*events.lock().unwrap(), vec![Ev::Send(b"typing".to_vec())]);

        // JSON turn: does not go out, body and all.
        let (session, events) = latched(InputEncoder::ClaudeStreamJson);
        session.set_intent(TerminationIntent::UserKill);
        assert_user_kill_refusal(session.write_input_observed(b"hello"));
        assert_user_kill_refusal(session.submit_input_observed(b"hello"));
        assert!(
            events.lock().unwrap().is_empty(),
            "{:?}",
            events.lock().unwrap()
        );

        // `Raw` mail: the body may have gone out, but the CR opening the turn does not.
        let (session, events) = latched(InputEncoder::Raw);
        session.set_intent(TerminationIntent::UserKill);
        assert_user_kill_refusal(session.submit_input_observed(b"envelope"));
        assert_eq!(
            *events.lock().unwrap(),
            vec![Ev::Send(b"envelope".to_vec())]
        );
    }

    /// It checks even after persistence finished (the fast path) — the second turn also does not go out while the user is exiting.
    #[test]
    fn a_user_kill_is_checked_even_after_the_latch_settled() {
        let (session, events) = latched(InputEncoder::Raw);
        session.write_input(b"a\r").expect("first turn");
        session.set_intent(TerminationIntent::UserKill);
        assert_user_kill_refusal(session.write_input_observed(b"b\r"));
        assert_eq!(
            *events.lock().unwrap(),
            vec![Ev::Commit("sid-1".into()), Ev::Send(b"a\r".to_vec())]
        );
    }

    // ── Submissions in the mode where the link owns the turn (ADR-0226 amendment · user decision 2026-09-26) ──
    //
    // ★The session of that mode does not count submissions — it takes only the user-exit refusal★. A turn happening in the conversation is the other side's first
    //   echo, and the link counts it through the latch's first-turn port (the echo side = the `backend/codex/transport.rs` tests).

    /// A `TransportOwned` session carrying a latch — the latch is empty (in production the link's handshake offers).
    fn transport_owned_latched() -> (
        AgentSession,
        Arc<Probe>,
        Arc<SessionIdLatch>,
        Arc<Mutex<Vec<Ev>>>,
    ) {
        let (session, probe) = probed(InputEncoder::TransportFramed, MidTurnPolicy::TransportOwned);
        let events = Arc::new(Mutex::new(Vec::new()));
        let latch = SessionIdLatch::new(session.id, 0, recording_port(&events));
        (
            session.with_session_id_latch(latch.clone()),
            probe,
            latch,
            events,
        )
    }

    /// (b) — sending is not counted (neither human input nor mail). The commit happens at the first-turn port (the first echo the link sees).
    #[test]
    fn a_transport_owned_write_leaves_the_commit_to_the_first_echo() {
        let (session, probe, latch, events) = transport_owned_latched();
        latch.offer("sid-1");

        session
            .write_input_from(b"hi", InputOrigin::User)
            .expect("hands over");
        session.write_input_observed(b"letter").expect("hands over");
        assert_eq!(probe.turns.lock().unwrap().len(), 2);
        assert_eq!(
            commits(&events),
            0,
            "persisted at the moment of sending — even a first input withdrawn with ✕ leaves a 0-turn id"
        );

        (latch.first_turn_sink())();
        assert_eq!(*events.lock().unwrap(), vec![Ev::Commit("sid-1".into())]);
    }

    /// (c) — withdrawing with ✕ the first input typed before the handshake persists nothing. Counting at send time would commit the thread
    ///   id that came after on the spot, leaving a 0-turn id — do not bring it back.
    #[test]
    fn a_first_input_withdrawn_before_any_echo_persists_nothing() {
        let (session, probe, latch, events) = transport_owned_latched();
        let typed = session
            .write_input_from(b"hi", InputOrigin::User)
            .expect("hands over");
        latch.offer("sid-1");
        let id = typed.msg_uuid.to_string();
        session.core.emit(queued(&id));
        *probe.withdraw_answer.lock().unwrap() = Some(Withdraw::Withdrawn);

        assert_eq!(
            session.cancel_queued_input(&id).unwrap(),
            CancelOutcome::Cancelled
        );
        assert_eq!(commits(&events), 0);
    }

    /// (g) — while the user is exiting, that mode does not hand over either — not counting does not remove the refusal too.
    #[test]
    fn a_user_kill_refuses_a_transport_owned_turn_and_persists_nothing() {
        let (session, probe, latch, events) = transport_owned_latched();
        latch.offer("sid-1");
        session.set_intent(TerminationIntent::UserKill);

        assert_user_kill_refusal(session.write_input_from(b"hi", InputOrigin::User));
        assert_user_kill_refusal(session.write_input_observed(b"letter"));

        assert!(
            probe.turns.lock().unwrap().is_empty(),
            "a turn was handed over during user exit"
        );
        assert_eq!(commits(&events), 0);
    }

    // ── Incarnation facts of the subscription response (ADR-0226) ──

    #[test]
    fn subscribe_from_matches_the_requested_epoch_against_its_own_incarnation() {
        let (session, _captured) = session_with(InputEncoder::Raw);
        let sink = || -> Arc<dyn OutputSink> {
            Arc::new(EmitCapturingSink {
                id: uuid::Uuid::new_v4(),
                seen: Arc::new(Mutex::new(Vec::new())),
            })
        };
        use crate::types::ReplayKind;

        let same = session.subscribe_from(sink(), Some(0), Some(0), |_| {});
        assert_eq!(same.outcome.kind, ReplayKind::Resumed);
        let other = session.subscribe_from(sink(), Some(0), Some(1), |_| {});
        assert_eq!(other.outcome.kind, ReplayKind::FromOldest);
        let unknown = session.subscribe_from(sink(), Some(0), None, |_| {});
        assert_eq!(unknown.outcome.kind, ReplayKind::FromOldest);
    }

    #[test]
    fn subscribe_from_hands_the_same_incarnation_to_on_ready_and_the_caller() {
        let sink = || -> Arc<dyn OutputSink> {
            Arc::new(EmitCapturingSink {
                id: uuid::Uuid::new_v4(),
                seen: Arc::new(Mutex::new(Vec::new())),
            })
        };
        for continues in [false, true] {
            let (session, _captured) = session_with(InputEncoder::Raw);
            let session = session.with_incarnation(continues);
            let mut at_ready = None;
            let reply =
                session.subscribe_from(sink(), None, None, |r| at_ready = Some(r.incarnation));
            let expected = Incarnation {
                epoch: session.epoch,
                continues_conversation: continues,
            };
            assert_eq!(reply.incarnation, expected);
            assert_eq!(at_ready, Some(expected));
        }
    }

    // ── ADR-0231: input path skeleton — origin · policy · cancel ──

    /// A sink collecting whole events — it looks even at the shape of list events.
    struct EventSink {
        id: SinkId,
        seen: Arc<Mutex<Vec<OutputEvent>>>,
    }
    impl OutputSink for EventSink {
        fn send(
            &self,
            frame: crate::types::OutputFrame<'_>,
        ) -> Result<(), crate::types::SinkError> {
            if let crate::types::OutputPayload::Event(e) = frame.payload {
                self.seen.lock().unwrap().push(e.clone());
            }
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    /// Collects only events **after** subscribing — the ring the subscription rewound synchronously is discarded.
    fn watch(session: &AgentSession) -> Arc<Mutex<Vec<OutputEvent>>> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        session.subscribe(Arc::new(EventSink {
            id: uuid::Uuid::new_v4(),
            seen: seen.clone(),
        }));
        seen.lock().unwrap().clear();
        seen
    }

    fn list_events(seen: &Arc<Mutex<Vec<OutputEvent>>>) -> Vec<QueuedInputEvent> {
        seen.lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                OutputEvent::QueuedInput(op) => Some(op.clone()),
                _ => None,
            })
            .collect()
    }

    /// A policy-`None` session standing with the same wiring as the manager (shared turn observation table + waiting-list table) — returned in a state where
    /// **the core answers that it is in a turn**.
    fn in_turn_policy_none_session(
        encoder: InputEncoder,
    ) -> (
        AgentSession,
        Arc<Mutex<Vec<Vec<u8>>>>,
        Arc<crate::turn::TurnObservations>,
        Arc<crate::inputs_pending::InputsPendingTable>,
    ) {
        let id = uuid::Uuid::new_v4();
        let turns = Arc::new(crate::turn::TurnObservations::new());
        let pending = Arc::new(crate::inputs_pending::InputsPendingTable::new());
        turns.register(id, 0);
        pending.register(id, 0);
        turns.observe(id, 0, 1, crate::turn::TurnSignal::Progress);
        assert!(
            turns.is_in_turn(id, 0),
            "premise: the core answers that it is in a turn"
        );
        let core = Arc::new(
            OutputCore::new(
                id,
                0,
                Arc::new(NoopStatusSink),
                crate::output_core::TurnWiring::new(turns.clone(), crate::backend::no_turn_signals),
            )
            .with_queued(crate::output_core::QueuedWiring {
                registry: Arc::new(QueuedInputs::new()),
                pending: pending.clone(),
            }),
        );
        let captured = Arc::new(Mutex::new(Vec::new()));
        let shell_cmd = crate::profile::AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let session = AgentSession::new(
            id,
            PathBuf::from("."),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            ShellBackend.capabilities(&shell_cmd),
            encoder,
            true,
            core,
            Box::new(CapturingTransport {
                captured: captured.clone(),
                fail_from: None,
            }),
        )
        .with_submit_pacing(Duration::ZERO, |_| {});
        (session, captured, turns, pending)
    }

    // TRD §7-1 new regression.
    #[test]
    fn a_policy_none_session_never_emits_queued_input_even_while_the_core_says_in_turn() {
        for encoder in [InputEncoder::Raw, InputEncoder::ClaudeStreamJson] {
            let (session, captured, _turns, _pending) = in_turn_policy_none_session(encoder);
            let seen = watch(&session);
            let mut expected = Vec::new();
            for origin in [InputOrigin::User, InputOrigin::Mail] {
                let body = b"echo hi\r\n";
                let outcome = session.write_input_from(body, origin).unwrap();
                expected.push(encoder.encode(body, outcome.msg_uuid));
            }
            let mail = b"<message from=\"bob\">hi</message>";
            let outcome = session.submit_input_observed(mail).unwrap();
            expected.push(encoder.encode(mail, outcome.msg_uuid));
            if let Some(submit) = encoder.submit_sequence() {
                expected.push(submit.to_vec());
            }

            assert_eq!(
                *captured.lock().unwrap(),
                expected,
                "{encoder:?}: policy None gives today's bytes as is for both origins"
            );
            assert!(
                list_events(&seen).is_empty(),
                "{encoder:?}: even in a turn, a policy-None session emits not a single list event"
            );
            let echoes = seen
                .lock()
                .unwrap()
                .iter()
                .filter(|e| matches!(e, OutputEvent::Structured { .. }))
                .count();
            assert_eq!(
                echoes,
                if encoder == InputEncoder::Raw { 0 } else { 3 },
                "{encoder:?}: the synthetic echo is also the same as today (json one per write · terminal none)"
            );
            assert!(session.queued_inputs().lock().is_empty());
        }
    }

    // TRD §7-1 new regression.
    #[test]
    fn a_policy_none_agent_never_reports_inputs_pending() {
        let (session, _captured, turns, pending) = in_turn_policy_none_session(InputEncoder::Raw);
        let id = session.id;
        for origin in [InputOrigin::User, InputOrigin::Mail] {
            session.write_input_from(b"typed\r", origin).unwrap();
        }
        session.submit_input_observed(b"mail").unwrap();
        assert!(matches!(
            session.cancel_queued_input("anything"),
            Err(CancelError::NotFound)
        ));

        assert_eq!(
            pending.get(id, 0),
            Some(false),
            "the waiting-list fact of a terminal incarnation is always empty — busy stands on turn observation alone"
        );
        assert!(
            turns.is_in_turn(id, 0),
            "the input path does not touch turn observation"
        );
    }

    #[test]
    fn cancel_under_policy_none_is_not_found_and_touches_nothing() {
        let (session, captured) = session_with(InputEncoder::Raw);
        // Defense: there is no way for a list row to stand under policy None, but even if one stood, it does not reach the link.
        session.core.emit(queued("q1"));
        let seen = watch(&session);
        assert!(matches!(
            session.cancel_queued_input("q1"),
            Err(CancelError::NotFound)
        ));
        assert!(captured.lock().unwrap().is_empty());
        assert!(list_events(&seen).is_empty());
    }

    fn queued(id: &str) -> OutputEvent {
        OutputEvent::QueuedInput(QueuedInputEvent::Queued {
            id: id.into(),
            text: format!("text of {id}"),
        })
    }

    fn test_cancel_line(id: &str) -> Vec<u8> {
        format!("cancel:{id}\n").into_bytes()
    }

    /// A link observing the policy branches — records raw write · turn hand-over · withdraw calls, and gives withdraw answers per the script.
    #[derive(Default)]
    struct Probe {
        raw: Mutex<Vec<Vec<u8>>>,
        turns: Mutex<Vec<TurnInput>>,
        withdraws: Mutex<Vec<String>>,
        withdraw_answer: Mutex<Option<Withdraw>>,
        fail_raw: std::sync::atomic::AtomicBool,
        unconfirmed: Mutex<Vec<String>>,
    }
    struct ProbeTransport(Arc<Probe>);
    impl AgentTransport for ProbeTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            let InputEvent::Raw(bytes) = input;
            if self.0.fail_raw.load(Ordering::SeqCst) {
                return Err(PtyError::WriteFailed("probe: write refused".into()));
            }
            self.0.raw.lock().unwrap().push(bytes);
            Ok(())
        }
        fn send_turn(&self, turn: TurnInput) -> Result<(), PtyError> {
            self.0.turns.lock().unwrap().push(turn);
            Ok(())
        }
        fn withdraw(&self, id: &str) -> Withdraw {
            self.0.withdraws.lock().unwrap().push(id.to_owned());
            self.0
                .withdraw_answer
                .lock()
                .unwrap()
                .unwrap_or(Withdraw::NotHeld)
        }
        fn unconfirmed_inputs(&self) -> Vec<String> {
            self.0.unconfirmed.lock().unwrap().clone()
        }
        fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            harness_caps()
        }
    }

    fn probed(encoder: InputEncoder, mid_turn: MidTurnPolicy) -> (AgentSession, Arc<Probe>) {
        let id = uuid::Uuid::new_v4();
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatusSink),
            crate::output_core::TurnWiring::detached(),
        ));
        let probe = Arc::new(Probe::default());
        let shell_cmd = crate::profile::AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![],
        };
        let session = AgentSession::new(
            id,
            PathBuf::from("."),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            ShellBackend.capabilities(&shell_cmd),
            encoder,
            true,
            core,
            Box::new(ProbeTransport(probe.clone())),
        )
        .with_submit_pacing(Duration::ZERO, |_| {})
        .with_mid_turn(mid_turn, Arc::new(DeliveryAck::new()));
        (session, probe)
    }

    fn classified() -> MidTurnPolicy {
        MidTurnPolicy::SessionClassified {
            cancel_line: test_cancel_line,
        }
    }

    fn phase_of(session: &AgentSession, id: &str) -> Option<RowPhase> {
        session
            .queued_inputs()
            .lock()
            .rows()
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.phase)
    }

    #[test]
    fn a_session_classified_cancel_announces_the_request_then_writes_the_cancel_line() {
        let (session, probe) = probed(InputEncoder::ClaudeStreamJson, classified());
        session.core.emit(queued("q1"));
        let seen = watch(&session);

        assert_eq!(
            session.cancel_queued_input("q1").unwrap(),
            CancelOutcome::Requested
        );
        assert_eq!(
            list_events(&seen),
            vec![QueuedInputEvent::CancelRequested { id: "q1".into() }]
        );
        assert_eq!(
            *probe.raw.lock().unwrap(),
            vec![test_cancel_line("q1")],
            "the cancel line goes once as the backend fragment as is — it does not pass through the encoder"
        );
        assert!(matches!(
            phase_of(&session, "q1"),
            Some(RowPhase::Cancelling { .. })
        ));
    }

    #[test]
    fn a_second_cancel_of_a_cancelling_item_is_requested_without_events_or_vendor_calls() {
        for mid_turn in [classified(), MidTurnPolicy::TransportOwned] {
            let (session, probe) = probed(InputEncoder::ClaudeStreamJson, mid_turn);
            session.core.emit(queued("q1"));
            session.core.emit(OutputEvent::QueuedInput(
                QueuedInputEvent::CancelRequested { id: "q1".into() },
            ));
            let seen = watch(&session);

            assert_eq!(
                session.cancel_queued_input("q1").unwrap(),
                CancelOutcome::Requested,
                "{mid_turn:?}: idempotent if already pending cancel"
            );
            assert!(list_events(&seen).is_empty(), "{mid_turn:?}");
            assert!(probe.raw.lock().unwrap().is_empty(), "{mid_turn:?}");
            assert!(probe.withdraws.lock().unwrap().is_empty(), "{mid_turn:?}");
        }
    }

    #[test]
    fn a_failed_cancel_line_write_reports_cancel_failed_and_keeps_the_item_cancelling() {
        let (session, probe) = probed(InputEncoder::ClaudeStreamJson, classified());
        session.core.emit(queued("q1"));
        probe.fail_raw.store(true, Ordering::SeqCst);
        let seen = watch(&session);

        assert!(matches!(
            session.cancel_queued_input("q1"),
            Err(CancelError::Write(PtyError::WriteFailed(_)))
        ));
        assert_eq!(
            list_events(&seen),
            vec![
                QueuedInputEvent::CancelRequested { id: "q1".into() },
                QueuedInputEvent::CancelFailed { id: "q1".into() },
            ]
        );
        assert!(
            matches!(phase_of(&session, "q1"), Some(RowPhase::Cancelling { .. })),
            "even if the cancel request fails, it is not returned to the list"
        );
    }

    #[test]
    fn cancel_of_an_unknown_or_settled_id_is_not_found_under_every_listing_policy() {
        for mid_turn in [classified(), MidTurnPolicy::TransportOwned] {
            let (session, probe) = probed(InputEncoder::ClaudeStreamJson, mid_turn);
            session.core.emit(queued("done"));
            session
                .core
                .emit(OutputEvent::QueuedInput(QueuedInputEvent::Delivered {
                    id: "done".into(),
                }));
            let seen = watch(&session);

            for id in ["never-listed", "done"] {
                assert!(
                    matches!(session.cancel_queued_input(id), Err(CancelError::NotFound)),
                    "{mid_turn:?}: {id}"
                );
            }
            assert!(list_events(&seen).is_empty(), "{mid_turn:?}");
            assert!(probe.raw.lock().unwrap().is_empty(), "{mid_turn:?}");
            assert!(probe.withdraws.lock().unwrap().is_empty(), "{mid_turn:?}");
        }
    }

    #[test]
    fn a_transport_owned_cancel_maps_the_three_withdraw_answers() {
        for (answer, expected) in [
            (Withdraw::Withdrawn, Ok(CancelOutcome::Cancelled)),
            (Withdraw::TooLate, Ok(CancelOutcome::Requested)),
            (Withdraw::NotHeld, Err(())),
        ] {
            let (session, probe) =
                probed(InputEncoder::TransportFramed, MidTurnPolicy::TransportOwned);
            session.core.emit(queued("q1"));
            *probe.withdraw_answer.lock().unwrap() = Some(answer);
            let seen = watch(&session);

            let got = session.cancel_queued_input("q1");
            match expected {
                Ok(outcome) => assert_eq!(got.unwrap(), outcome, "{answer:?}"),
                Err(()) => assert!(matches!(got, Err(CancelError::NotFound)), "{answer:?}"),
            }
            assert_eq!(*probe.withdraws.lock().unwrap(), vec!["q1".to_string()]);
            assert!(
                list_events(&seen).is_empty(),
                "{answer:?}: list events are emitted by the link — the session does not emit them"
            );
            assert!(probe.raw.lock().unwrap().is_empty(), "{answer:?}");
        }
    }

    #[test]
    fn the_listing_marks_what_the_transport_holds_unconfirmed_on_queued_rows_only() {
        let (session, probe) = probed(InputEncoder::TransportFramed, MidTurnPolicy::TransportOwned);
        for id in ["q1", "q2", "q3"] {
            session.core.emit(queued(id));
        }
        session.core.emit(OutputEvent::QueuedInput(
            QueuedInputEvent::CancelRequested { id: "q3".into() },
        ));
        *probe.unconfirmed.lock().unwrap() = vec!["q2".into(), "q3".into(), "gone".into()];

        let (rows, as_of_seq) = session.list_queued_inputs();
        let states: Vec<(&str, ListedState)> =
            rows.iter().map(|r| (r.id.as_str(), r.state)).collect();
        assert_eq!(
            states,
            vec![
                ("q1", ListedState::Queued),
                ("q2", ListedState::Unconfirmed),
                (
                    "q3",
                    ListedState::Cancelling {
                        answer: CancelAnswer::Unanswered,
                        vendor_closed: false
                    }
                ),
            ],
            "pending cancel beats the marker, and an id not in the registry does not become a row"
        );
        assert_eq!(rows[0].text, "text of q1");
        assert!(as_of_seq.is_some());
        assert_eq!(
            as_of_seq,
            session.queued_inputs().snapshot().1,
            "as_of_seq is that of the registry snapshot as is"
        );
    }

    #[test]
    fn a_transport_that_holds_nothing_unconfirmed_lists_every_row_as_registered() {
        let (session, _captured) = session_with(InputEncoder::Raw);
        assert_eq!(
            session.list_queued_inputs(),
            (vec![], None),
            "an incarnation without list events — empty list · no as_of_seq"
        );
        session.core.emit(queued("q1"));
        let (rows, as_of_seq) = session.list_queued_inputs();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, ListedState::Queued);
        assert!(as_of_seq.is_some());
    }

    #[test]
    fn a_transport_owned_write_hands_the_turn_over_with_its_origin_and_receipt_id() {
        let (session, probe) = probed(InputEncoder::TransportFramed, MidTurnPolicy::TransportOwned);
        let seen = watch(&session);

        let typed = session.write_input_from(b"hi", InputOrigin::User).unwrap();
        let mail = session.write_input_observed(b"letter").unwrap();

        let turns = probe.turns.lock().unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].id, typed.msg_uuid.to_string());
        assert_eq!(turns[0].body, b"hi".to_vec());
        assert_eq!(turns[0].origin, InputOrigin::User);
        assert_eq!(turns[1].id, mail.msg_uuid.to_string());
        assert_eq!(turns[1].body, b"letter".to_vec());
        assert_eq!(
            turns[1].origin,
            InputOrigin::Mail,
            "`*_observed` verbs are mail"
        );
        assert!(
            probe.raw.lock().unwrap().is_empty(),
            "turns go only through the link turn verb"
        );
        assert!(list_events(&seen).is_empty(), "the link classifies");
    }

    /// The bubbles of the link turn verb branch are emitted by the link — even if the encoder declares a synthetic echo, the session emits not one copy.
    /// Policy `None` with the same encoder (control group) emits the echo as today.
    #[test]
    fn a_transport_owned_write_makes_no_session_echo_even_with_an_echoing_encoder() {
        let (session, probe) = probed(
            InputEncoder::ClaudeStreamJson,
            MidTurnPolicy::TransportOwned,
        );
        let seen = watch(&session);
        session.write_input_from(b"hi", InputOrigin::User).unwrap();
        session.write_input_observed(b"letter").unwrap();
        assert_eq!(probe.turns.lock().unwrap().len(), 2);
        assert!(
            seen.lock().unwrap().is_empty(),
            "the session emitted an event"
        );

        let (control, _probe) = probed(InputEncoder::ClaudeStreamJson, MidTurnPolicy::None);
        let seen = watch(&control);
        control.write_input_from(b"hi", InputOrigin::User).unwrap();
        assert!(
            matches!(&seen.lock().unwrap()[..], [OutputEvent::Structured { kind, .. }] if kind == "user"),
            "the control group did not emit an echo — this test measures nothing"
        );
    }

    // ── ADR-0231: session classification (SessionClassified) — user input · cancel (TRD §7-1 claude session row) ──

    use crate::inputs_pending::InputsPendingTable;
    use crate::turn::{TurnEndKind, TurnObservations, TurnSignal};
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc;

    /// A link observing writes — records per write (the bytes, the ids standing in the registry **at that moment**), and fails or is held per the script.
    struct Gate {
        core: Arc<OutputCore>,
        writes: Mutex<Vec<(Vec<u8>, Vec<String>)>>,
        fail: AtomicBool,
        /// If `Some`, as soon as the next write comes in it rings the former and stops until it receives the latter (only once).
        hold: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
        /// An event record putting write attempts, latch commits and `Queued` emits on one line (for latch order assertions).
        trace: Arc<Mutex<Vec<String>>>,
    }
    struct GateTransport(Arc<Gate>);
    impl AgentTransport for GateTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            let InputEvent::Raw(bytes) = input;
            let listed: Vec<String> = self
                .0
                .core
                .queued_inputs()
                .lock()
                .rows()
                .iter()
                .map(|r| r.id.clone())
                .collect();
            self.0.trace.lock().unwrap().push("write".into());
            let hold = self.0.hold.lock().unwrap().take();
            if let Some((entered, release)) = hold {
                entered.send(()).unwrap();
                release.recv().unwrap();
            }
            if self.0.fail.load(Ordering::SeqCst) {
                return Err(PtyError::WriteFailed("gate: write refused".into()));
            }
            self.0.writes.lock().unwrap().push((bytes, listed));
            Ok(())
        }
        fn resize(&self, _cols: u16, _rows: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            harness_caps()
        }
    }

    /// A sink collecting with the output seq — asserts event order by ring order (seq) (fanout arrival order is not ring order).
    struct SeqSink {
        id: SinkId,
        seen: Arc<Mutex<Vec<(u64, OutputEvent)>>>,
    }
    impl OutputSink for SeqSink {
        fn send(
            &self,
            frame: crate::types::OutputFrame<'_>,
        ) -> Result<(), crate::types::SinkError> {
            if let crate::types::OutputPayload::Event(e) = frame.payload {
                self.seen.lock().unwrap().push((frame.seq, e.clone()));
            }
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    /// A sink writing `Queued` emits into the event record — fanout runs synchronously inside emit.
    struct TraceSink {
        id: SinkId,
        trace: Arc<Mutex<Vec<String>>>,
    }
    impl OutputSink for TraceSink {
        fn send(
            &self,
            frame: crate::types::OutputFrame<'_>,
        ) -> Result<(), crate::types::SinkError> {
            if let crate::types::OutputPayload::Event(OutputEvent::QueuedInput(
                QueuedInputEvent::Queued { .. },
            )) = frame.payload
            {
                self.trace.lock().unwrap().push("queued".into());
            }
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    /// One claude JSON session standing with the same wiring as the manager (shared turn observation table + waiting-list table + classifier).
    struct Classified {
        session: Arc<AgentSession>,
        gate: Arc<Gate>,
        turns: Arc<TurnObservations>,
        pending: Arc<InputsPendingTable>,
        ack: Arc<DeliveryAck>,
        trace: Arc<Mutex<Vec<String>>>,
    }

    impl Classified {
        fn new(classify: crate::backend::TurnClassifier) -> Self {
            Self::build(classify, false)
        }

        /// A claude session carrying a session id latch — the latch already holds the id (same as `latched_session_with`), so the first submission
        /// commits on the spot, and that commit stands in the same event record (`trace`) as the writes and `Queued`.
        fn with_session_id_latch() -> Self {
            let fx = Self::build(crate::backend::claude::classify_turn, true);
            fx.session.subscribe(Arc::new(TraceSink {
                id: uuid::Uuid::new_v4(),
                trace: fx.trace.clone(),
            }));
            fx.trace.lock().unwrap().clear();
            fx
        }

        fn build(classify: crate::backend::TurnClassifier, latched: bool) -> Self {
            let id = uuid::Uuid::new_v4();
            let turns = Arc::new(TurnObservations::new());
            let pending = Arc::new(InputsPendingTable::new());
            turns.register(id, 0);
            pending.register(id, 0);
            let core = Arc::new(
                OutputCore::new(
                    id,
                    0,
                    Arc::new(NoopStatusSink),
                    crate::output_core::TurnWiring::new(turns.clone(), classify),
                )
                .with_queued(crate::output_core::QueuedWiring {
                    registry: Arc::new(QueuedInputs::new()),
                    pending: pending.clone(),
                }),
            );
            let trace = Arc::new(Mutex::new(Vec::new()));
            let gate = Arc::new(Gate {
                core: core.clone(),
                writes: Mutex::new(Vec::new()),
                fail: AtomicBool::new(false),
                hold: Mutex::new(None),
                trace: trace.clone(),
            });
            let ack = Arc::new(DeliveryAck::new());
            let shell_cmd = crate::profile::AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            };
            let session = AgentSession::new(
                id,
                PathBuf::from("."),
                0,
                80,
                24,
                Arc::new(AtomicU8::new(0)),
                ShellBackend.capabilities(&shell_cmd),
                InputEncoder::ClaudeStreamJson,
                true,
                core,
                Box::new(GateTransport(gate.clone())),
            )
            .with_submit_pacing(Duration::ZERO, |_| {})
            .with_mid_turn(classified(), ack.clone());
            let session = if latched {
                let port_trace = trace.clone();
                let latch = SessionIdLatch::new(
                    id,
                    0,
                    Arc::new(move |raw: &str| {
                        port_trace.lock().unwrap().push(format!("commit:{raw}"))
                    }),
                );
                latch.offer("sid-1");
                session.with_session_id_latch(latch)
            } else {
                session
            };
            Self {
                session: Arc::new(session),
                gate,
                turns,
                pending,
                ack,
                trace,
            }
        }

        fn trace(&self) -> Vec<String> {
            self.trace.lock().unwrap().clone()
        }

        fn claude() -> Self {
            Self::new(crate::backend::claude::classify_turn)
        }

        fn id(&self) -> AgentId {
            self.session.id
        }

        /// Puts the core in a turn with one vendor output line (the classifier counts it as progress).
        fn start_turn(&self) {
            self.session.core.emit(OutputEvent::TextDelta {
                text: "thinking".into(),
                turn_id: None,
                message_id: None,
            });
            assert!(self.turns.is_in_turn(self.id(), 0), "premise: in a turn");
        }

        /// Collects only events **after** subscribing, as (seq, event).
        fn watch(&self) -> Arc<Mutex<Vec<(u64, OutputEvent)>>> {
            let seen = Arc::new(Mutex::new(Vec::new()));
            self.session.subscribe(Arc::new(SeqSink {
                id: uuid::Uuid::new_v4(),
                seen: seen.clone(),
            }));
            seen.lock().unwrap().clear();
            seen
        }

        fn written(&self) -> Vec<Vec<u8>> {
            self.gate
                .writes
                .lock()
                .unwrap()
                .iter()
                .map(|(bytes, _)| bytes.clone())
                .collect()
        }

        fn listed(&self) -> Vec<String> {
            self.session
                .queued_inputs()
                .lock()
                .rows()
                .iter()
                .map(|r| r.id.clone())
                .collect()
        }

        fn hold_next_write(&self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
            let (entered_tx, entered_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            *self.gate.hold.lock().unwrap() = Some((entered_tx, release_rx));
            (entered_rx, release_tx)
        }
    }

    fn seq_list_events(seen: &Arc<Mutex<Vec<(u64, OutputEvent)>>>) -> Vec<QueuedInputEvent> {
        let mut got = seen.lock().unwrap().clone();
        got.sort_by_key(|(seq, _)| *seq);
        got.into_iter()
            .filter_map(|(_, e)| match e {
                OutputEvent::QueuedInput(op) => Some(op),
                _ => None,
            })
            .collect()
    }

    fn echoes(seen: &Arc<Mutex<Vec<(u64, OutputEvent)>>>) -> usize {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|(_, e)| matches!(e, OutputEvent::Structured { .. }))
            .count()
    }

    fn queued_op(id: &str, text: &str) -> QueuedInputEvent {
        QueuedInputEvent::Queued {
            id: id.into(),
            text: text.into(),
        }
    }

    // TRD §7-1: idle ∧ registry empty → echo, no event.
    #[test]
    fn an_idle_user_input_is_written_and_echoed_without_a_list_event() {
        let fx = Classified::claude();
        fx.ack.set_available();
        let seen = fx.watch();

        let outcome = fx
            .session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();

        assert_eq!(
            fx.written(),
            vec![InputEncoder::ClaudeStreamJson.encode(b"typed", outcome.msg_uuid)]
        );
        assert_eq!(echoes(&seen), 1, "today's path — one synthetic echo");
        assert!(seq_list_events(&seen).is_empty());
    }

    // TRD §7-1: in a turn (`Available`) → `Queued` is in the ring before `send_input` · no synthetic echo.
    #[test]
    fn a_user_input_during_a_turn_is_queued_before_its_write_and_not_echoed() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.start_turn();
        let seen = fx.watch();

        let outcome = fx
            .session
            .write_input_from("hello".as_bytes(), InputOrigin::User)
            .unwrap();
        let id = outcome.msg_uuid.to_string();

        let writes = fx.gate.writes.lock().unwrap().clone();
        assert_eq!(writes.len(), 1);
        assert_eq!(
            writes[0].0,
            InputEncoder::ClaudeStreamJson.encode("hello".as_bytes(), outcome.msg_uuid),
            "the written line is the same as today (a user line with the uuid)"
        );
        assert_eq!(
            writes[0].1,
            vec![id.clone()],
            "already standing in the registry at the moment of the write"
        );
        assert_eq!(seq_list_events(&seen), vec![queued_op(&id, "hello")]);
        assert_eq!(echoes(&seen), 0, "the list branch emits no synthetic echo");
    }

    // TRD §7-1: no turn ∧ registry present → Queued (PRD §3-5).
    #[test]
    fn a_user_input_while_items_wait_between_turns_is_queued() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.session.core.emit(queued("q0"));
        assert!(!fx.turns.is_in_turn(fx.id(), 0));
        assert_eq!(fx.pending.get(fx.id(), 0), Some(true));

        let outcome = fx
            .session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();

        assert_eq!(
            fx.listed(),
            vec!["q0".to_string(), outcome.msg_uuid.to_string()]
        );
    }

    // TRD §7-1: after the `Delivered` of a drain `started` is written (core in a turn · before the first output) → Queued.
    #[test]
    fn a_user_input_after_a_drain_started_before_any_output_is_queued() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.session.core.emit(queued("q0"));
        fx.session
            .core
            .emit(OutputEvent::QueuedInput(QueuedInputEvent::Delivered {
                id: "q0".into(),
            }));
        assert!(fx.listed().is_empty());
        assert_eq!(fx.pending.get(fx.id(), 0), Some(false));
        assert!(fx.turns.is_in_turn(fx.id(), 0), "accepted = progress");
        let seen = fx.watch();

        let outcome = fx
            .session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();

        assert_eq!(fx.listed(), vec![outcome.msg_uuid.to_string()]);
        assert_eq!(echoes(&seen), 0);
    }

    /// A classifier that holds the emit of the drain `Delivered` emptying the list **after** the registry reduction and **before** turn observation — it applies only to the one id
    /// [`DRAIN_GATE_ID`], so it does not mix with other tests running in parallel.
    static DRAIN_GATE: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>> = Mutex::new(None);
    const DRAIN_GATE_ID: &str = "drain-gate";
    fn drain_gated_classifier(event: &OutputEvent) -> Option<TurnSignal> {
        if let OutputEvent::QueuedInput(QueuedInputEvent::Delivered { id }) = event {
            if id == DRAIN_GATE_ID {
                let gate = DRAIN_GATE.lock().unwrap().take();
                if let Some((entered, release)) = gate {
                    entered.send(()).unwrap();
                    release.recv().unwrap();
                }
            }
        }
        crate::backend::claude::classify_turn(event)
    }

    // TRD §7-1: input that came inside the emit of the drain `Delivered` emptying the list (registry empty, progress not yet) → Queued —
    //   the classification does not read the registry but reads the waiting-list table first.
    #[test]
    fn a_user_input_inside_the_draining_delivered_emit_is_queued() {
        let fx = Classified::new(drain_gated_classifier);
        fx.ack.set_available();
        fx.session.core.emit(queued(DRAIN_GATE_ID));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        *DRAIN_GATE.lock().unwrap() = Some((entered_tx, release_rx));

        let core = fx.session.core.clone();
        let pump = std::thread::spawn(move || {
            core.emit(OutputEvent::QueuedInput(QueuedInputEvent::Delivered {
                id: DRAIN_GATE_ID.into(),
            }))
        });
        entered_rx.recv().unwrap();
        assert!(
            fx.listed().is_empty(),
            "premise: the registry is already empty"
        );
        assert!(
            !fx.turns.is_in_turn(fx.id(), 0),
            "premise: progress not yet"
        );
        assert_eq!(
            fx.pending.get(fx.id(), 0),
            Some(true),
            "premise: empty not yet either"
        );

        let outcome = fx
            .session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();
        release_tx.send(()).unwrap();
        pump.join().unwrap();

        assert_eq!(fx.listed(), vec![outcome.msg_uuid.to_string()]);
        assert_eq!(
            fx.pending.get(fx.id(), 0),
            Some(true),
            "the empty deferred outside the lock does not overwrite the filled that stood after it"
        );
    }

    // TRD §7-1: the classification does not read the registry — it looks only at the waiting-list table (a state where the table and the registry diverge is made by hand).
    #[test]
    fn the_classification_reads_the_pending_table_not_the_registry() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.session.core.emit(queued("q0"));
        fx.pending.set(fx.id(), 0, 1_000, false);
        let seen = fx.watch();
        fx.session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();
        assert!(
            seq_list_events(&seen).is_empty(),
            "even if the registry is filled, if the table is empty it is idle"
        );
        assert_eq!(echoes(&seen), 1);

        let fx = Classified::claude();
        fx.ack.set_available();
        fx.pending.set(fx.id(), 0, 1_000, true);
        let outcome = fx
            .session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();
        assert_eq!(
            fx.listed(),
            vec![outcome.msg_uuid.to_string()],
            "even if the registry is empty, if the table is filled it is the list branch"
        );
    }

    // TRD §7-1: the Queued branch writes nothing to the turn table (the input path does not write progress).
    #[test]
    fn the_queued_branch_writes_nothing_to_the_turn_table() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.pending.set(fx.id(), 0, 1_000, true);
        let before = fx.turns.get(fx.id(), 0).unwrap();

        fx.session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();

        let after = fx.turns.get(fx.id(), 0).unwrap();
        assert!(!after.in_turn);
        assert_eq!(after.last_signal, before.last_signal);
    }

    // TRD §7-1: `Unknown` → Queued (assumed) · `Queued` after the write (not in the ring at the moment of the write).
    #[test]
    fn an_unknown_ack_announces_queued_only_after_a_successful_write() {
        let fx = Classified::claude();
        fx.start_turn();
        let seen = fx.watch();

        let outcome = fx
            .session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();
        let id = outcome.msg_uuid.to_string();

        let writes = fx.gate.writes.lock().unwrap().clone();
        assert_eq!(writes.len(), 1);
        assert!(
            writes[0].1.is_empty(),
            "not in the registry at the moment of the write"
        );
        assert_eq!(seq_list_events(&seen), vec![queued_op(&id, "typed")]);
        assert_eq!(echoes(&seen), 0);
    }

    // TRD §7-1: `Unknown` write failure → 0 events · `Err`.
    #[test]
    fn an_unknown_ack_write_failure_emits_nothing() {
        let fx = Classified::claude();
        fx.start_turn();
        fx.gate.fail.store(true, Ordering::SeqCst);
        let seen = fx.watch();

        assert!(matches!(
            fx.session.write_input_from(b"typed", InputOrigin::User),
            Err(PtyError::WriteFailed(_))
        ));
        assert!(seen.lock().unwrap().is_empty());
        assert!(fx.listed().is_empty());
    }

    // TRD §7-1: if, while the write is held, the pump side sets `AckUnavailable` and the write is made to fail, that text does not become a bubble
    //   (`Delivered`) · the pump-side emit does not take the input lock.
    #[test]
    fn a_failed_unknown_ack_write_is_not_turned_into_a_bubble_by_ack_unavailable() {
        let fx = Classified::claude();
        fx.start_turn();
        let seen = fx.watch();
        let (entered, release) = fx.hold_next_write();

        let session = fx.session.clone();
        let writer =
            std::thread::spawn(move || session.write_input_from(b"typed", InputOrigin::User));
        entered.recv().unwrap();
        assert!(
            fx.session.input_order.try_lock().is_err(),
            "premise: the write holds the lock"
        );

        let (done_tx, done_rx) = mpsc::channel();
        let (core, ack) = (fx.session.core.clone(), fx.ack.clone());
        std::thread::spawn(move || {
            assert!(ack.try_set_unavailable());
            core.emit(OutputEvent::QueuedInput(QueuedInputEvent::AckUnavailable {
                delivered: vec![],
            }));
            done_tx.send(()).unwrap();
        });
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the pump-side emit waited for the input lock");

        fx.gate.fail.store(true, Ordering::SeqCst);
        release.send(()).unwrap();
        assert!(writer.join().unwrap().is_err());

        assert_eq!(
            seq_list_events(&seen),
            vec![QueuedInputEvent::AckUnavailable { delivered: vec![] }],
            "text that could not be written goes onto neither the list nor a bubble"
        );
        assert!(fx.listed().is_empty());
        assert_eq!(echoes(&seen), 0);
    }

    // TRD §7-1: `Unavailable` → today's echo even in a turn (N1 (a)).
    #[test]
    fn an_unavailable_ack_takes_todays_path_even_during_a_turn() {
        let fx = Classified::claude();
        assert!(fx.ack.try_set_unavailable());
        fx.start_turn();
        let seen = fx.watch();

        fx.session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();

        assert_eq!(fx.written().len(), 1);
        assert_eq!(echoes(&seen), 1);
        assert!(seq_list_events(&seen).is_empty());
    }

    // TRD §7-1: `origin=Mail` → no `Queued` even in a turn, today's echo as is (AC22) · mail does not take the input lock.
    #[test]
    fn a_mail_input_takes_todays_path_even_during_a_turn() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.start_turn();
        let seen = fx.watch();

        let _order = fx.session.input_order.lock().unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        let session = fx.session.clone();
        std::thread::spawn(move || {
            session.write_input_from(b"a", InputOrigin::Mail).unwrap();
            session.write_input_observed(b"b").unwrap();
            session.submit_input_observed(b"c").unwrap();
            done_tx.send(()).unwrap();
        });
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("mail waited for the input lock");

        assert_eq!(fx.written().len(), 3);
        assert_eq!(echoes(&seen), 3);
        assert!(seq_list_events(&seen).is_empty());
    }

    // TRD §7-1: write failure → `Dropped{Rejected}` + `Err`.
    #[test]
    fn a_failed_queued_write_is_dropped_as_rejected_and_errors() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.start_turn();
        fx.gate.fail.store(true, Ordering::SeqCst);
        let seen = fx.watch();

        assert!(matches!(
            fx.session.write_input_from(b"typed", InputOrigin::User),
            Err(PtyError::WriteFailed(_))
        ));

        let ops = seq_list_events(&seen);
        assert_eq!(ops.len(), 2, "{ops:?}");
        let QueuedInputEvent::Queued { id, .. } = &ops[0] else {
            panic!("the first event is Queued: {ops:?}")
        };
        assert_eq!(
            ops[1],
            QueuedInputEvent::Dropped {
                id: id.clone(),
                cause: DropCause::Rejected
            }
        );
        assert!(fx.listed().is_empty());
        assert_eq!(echoes(&seen), 0);
        assert!(
            matches!(
                fx.session.cancel_queued_input(id),
                Err(CancelError::NotFound)
            ),
            "cancelling an entry closed because it could not be written is NOT_FOUND"
        );
    }

    // ── ADR-0226 × ADR-0231: even in the session classification branch, the first-submission latch counts once before sending ──

    /// The first user input in a turn (`Available`) — one commit comes before the `Queued` emit and before the write.
    // ADR-0226
    #[test]
    fn a_first_queued_user_input_commits_the_latch_once_before_queued_and_the_write() {
        let fx = Classified::with_session_id_latch();
        fx.ack.set_available();
        fx.start_turn();

        let first = fx
            .session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();
        let second = fx
            .session
            .write_input_from(b"again", InputOrigin::User)
            .unwrap();

        assert_eq!(
            fx.trace(),
            vec!["commit:sid-1", "queued", "write", "queued", "write"],
            "persistence before the first list event and the first write · the second submission does not commit again"
        );
        assert_ne!(first.msg_uuid, second.msg_uuid);
    }

    /// The first user input in a turn (`Unknown`) — even in the after-write `Queued` branch, the commit comes before the write.
    // ADR-0226
    #[test]
    fn a_first_unknown_ack_user_input_commits_the_latch_before_the_write() {
        let fx = Classified::with_session_id_latch();
        fx.start_turn();

        fx.session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();

        assert_eq!(fx.trace(), vec!["commit:sid-1", "write", "queued"]);
    }

    /// While the user is exiting, the latch refuses the list branch too — `Err` · 0 list events · 0 writes · 0 commits.
    // ADR-0226
    #[test]
    fn a_user_kill_refuses_a_queued_user_input_without_listing_writing_or_committing() {
        let fx = Classified::with_session_id_latch();
        fx.ack.set_available();
        fx.start_turn();
        let seen = fx.watch();
        fx.session.set_intent(TerminationIntent::UserKill);

        assert_user_kill_refusal(fx.session.write_input_from(b"typed", InputOrigin::User));

        assert!(seq_list_events(&seen).is_empty());
        assert!(fx.listed().is_empty());
        assert!(fx.written().is_empty());
        assert!(fx.trace().is_empty(), "{:?}", fx.trace());
    }

    // TRD §7-1: input lock contention — even if another thread cancels the same id while the text write is held, the order written to the link is
    //   text line → cancel line.
    #[test]
    fn a_cancel_line_is_never_written_before_its_input_line() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.start_turn();
        let (entered, release) = fx.hold_next_write();

        let session = fx.session.clone();
        let writer =
            std::thread::spawn(move || session.write_input_from(b"typed", InputOrigin::User));
        entered.recv().unwrap();
        let listed = fx.listed();
        assert_eq!(listed.len(), 1, "Queued stood before the write");
        let id = listed[0].clone();

        // ★So it does not spin idly★: a barrier confirms the cancel thread got right up to the cancel call, and confirms at that moment the text write holds the input
        //   lock — so during the window below, the cancel is actually contending for that lock. The length of the window is
        //   not the grounds of the order but a grace period for a regression skipping the lock to show itself.
        let started = Arc::new(std::sync::Barrier::new(2));
        let (done_tx, done_rx) = mpsc::channel();
        let session = fx.session.clone();
        let cancel_id = id.clone();
        let canceller_started = started.clone();
        let canceller = std::thread::spawn(move || {
            canceller_started.wait();
            let answer = session.cancel_queued_input(&cancel_id);
            done_tx.send(()).unwrap();
            answer
        });
        started.wait();
        assert!(
            fx.session.input_order.try_lock().is_err(),
            "premise: when the cancel started, the text write holds the lock"
        );
        assert!(
            done_rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "the cancel does not finish before the text write releases the lock"
        );
        assert!(
            fx.written().is_empty(),
            "nothing was written before the held text"
        );
        assert!(
            matches!(phase_of(&fx.session, &id), Some(RowPhase::Queued)),
            "the cancel request was not recorded while waiting for the lock either"
        );
        release.send(()).unwrap();

        let outcome = writer.join().unwrap().unwrap();
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("once the text write finishes, the cancel gets the lock");
        assert_eq!(canceller.join().unwrap().unwrap(), CancelOutcome::Requested);
        assert_eq!(
            fx.written(),
            vec![
                InputEncoder::ClaudeStreamJson.encode(b"typed", outcome.msg_uuid),
                test_cancel_line(&id),
            ]
        );
        assert!(matches!(
            phase_of(&fx.session, &id),
            Some(RowPhase::Cancelling { .. })
        ));
    }

    // TRD §7-1: the classification and emit order of two concurrent inputs match — of two overlapping while idle, exactly one takes today's path (echo),
    //   and it is the one written first.
    #[test]
    fn concurrent_user_inputs_are_classified_in_the_order_they_are_written() {
        for _ in 0..50 {
            let fx = Classified::claude();
            fx.ack.set_available();
            let seen = fx.watch();
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let writers: Vec<_> = ["one", "two"]
                .into_iter()
                .map(|text| {
                    let (session, barrier) = (fx.session.clone(), barrier.clone());
                    std::thread::spawn(move || {
                        barrier.wait();
                        session
                            .write_input_from(text.as_bytes(), InputOrigin::User)
                            .unwrap()
                            .msg_uuid
                            .to_string()
                    })
                })
                .collect();
            let ids: Vec<String> = writers.into_iter().map(|w| w.join().unwrap()).collect();

            let written = fx.written();
            assert_eq!(written.len(), 2);
            let first = ids
                .iter()
                .find(|id| String::from_utf8_lossy(&written[0]).contains(id.as_str()))
                .unwrap()
                .clone();
            let second = ids.iter().find(|id| **id != first).unwrap().clone();
            assert_eq!(echoes(&seen), 1, "only one was classified as idle");
            let echoed = seen
                .lock()
                .unwrap()
                .iter()
                .find_map(|(_, e)| match e {
                    OutputEvent::Structured { json, .. } => Some(json.clone()),
                    _ => None,
                })
                .unwrap();
            assert!(echoed.contains(&first), "the one written first was echoed");
            assert_eq!(
                fx.listed(),
                vec![second],
                "the one written later stood on the list"
            );
        }
    }

    // Cancel vs the pump's acceptance — the check and `CancelRequested` are one replay section, so if it answered `Requested`,
    //   `CancelRequested` is before `Delivered` in the ring, and if `NOT_FOUND`, there is neither an event nor a cancel line.
    #[test]
    fn a_cancel_racing_the_delivered_answers_consistently_with_the_ring() {
        for _ in 0..200 {
            let fx = Classified::claude();
            fx.ack.set_available();
            fx.session.core.emit(queued("q1"));
            let seen = fx.watch();
            let barrier = Arc::new(std::sync::Barrier::new(2));

            let (core, b) = (fx.session.core.clone(), barrier.clone());
            let pump = std::thread::spawn(move || {
                b.wait();
                core.emit(OutputEvent::QueuedInput(QueuedInputEvent::Delivered {
                    id: "q1".into(),
                }));
            });
            barrier.wait();
            let got = fx.session.cancel_queued_input("q1");
            pump.join().unwrap();

            let ops = seq_list_events(&seen);
            let requested = ops
                .iter()
                .position(|op| *op == QueuedInputEvent::CancelRequested { id: "q1".into() });
            let delivered = ops
                .iter()
                .position(|op| *op == QueuedInputEvent::Delivered { id: "q1".into() })
                .expect("acceptance always stands in the ring");
            match got {
                Ok(CancelOutcome::Requested) => {
                    assert!(requested.is_some_and(|r| r < delivered), "{ops:?}");
                    assert_eq!(fx.written(), vec![test_cancel_line("q1")]);
                }
                Err(CancelError::NotFound) => {
                    assert!(requested.is_none(), "{ops:?}");
                    assert!(fx.written().is_empty());
                }
                other => panic!("{other:?}"),
            }
            assert!(fx.listed().is_empty(), "the outcome is acceptance");
        }
    }

    // TRD §7-1 (the session column of the 「stop after error for mail」 row): stop after error ∧ idle ∧ list empty → Direct (write + echo — the session does not read that column).
    #[test]
    fn a_halted_idle_session_still_sends_a_user_input_directly() {
        let fx = Classified::claude();
        fx.ack.set_available();
        fx.turns.observe(fx.id(), 0, 1, TurnSignal::Failed);
        fx.turns
            .observe(fx.id(), 0, 2, TurnSignal::Ended(TurnEndKind::Clean));
        let fact = fx.turns.get(fx.id(), 0).unwrap();
        assert!(
            fact.last_end_failed && !fact.in_turn,
            "premise: stop after error"
        );
        let seen = fx.watch();

        fx.session
            .write_input_from(b"typed", InputOrigin::User)
            .unwrap();

        assert_eq!(fx.written().len(), 1);
        assert_eq!(echoes(&seen), 1);
        assert!(seq_list_events(&seen).is_empty());
    }
}
