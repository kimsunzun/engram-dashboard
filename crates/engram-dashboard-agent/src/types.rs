pub type AgentId = uuid::Uuid;

pub type SinkId = uuid::Uuid;

/// internally-tagged serialization — the frontend receives it as a discriminated union.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum AgentStatus {
    Running,
    Exiting,
    Exited { code: Option<i32> },
    Failed { message: String },
    Killed,
}

impl AgentStatus {
    /// ★Different from "present in the session map" (load-bearing)★: a session stays in the map until the reaper collects it, so
    ///   deciding by mere presence mixes in corpses. The roster (`AgentManager::roster`) and the daemon adapter
    ///   (`messaging_host::is_live` — it only calls this predicate) must look at **the same condition** so the sending side and
    ///   the flush side do not see different worlds. This predicate is canonical — do not make a duplicate.
    // ADR-0116
    pub fn is_live(&self) -> bool {
        matches!(self, AgentStatus::Running | AgentStatus::Exiting)
    }
}

/// Internal pump→core output event. An extensible enum. core is variant-agnostic (_ => ignore).
///
/// ★ADR-0045 (output refining moved to the backend)★: the console stays `TerminalBytes` (VT byte stream) as is,
/// and for structured backends (claude stream-json etc.) the backend decoder parses and emits the structured variants below.
/// This type is **an agent domain type**, not a protocol wire type — agent↔wire conversion is done by the daemon
/// adapter (ADR-0003 isolation: core does not know the wire). No tauri import in core (serde is allowed).
///
/// `turn_id`/`message_id` are optional fields for conversation tracking — claude need not fill them, and they are left open to absorb
/// the turn/message model leaks of codex/gemini (swappability). None if the backend cannot fill them.
#[derive(Debug, Clone)]
pub enum OutputEvent {
    /// Console raw bytes (VT stream). The only payload of PtyTransport and terminal mode.
    TerminalBytes(Vec<u8>),
    TextDelta {
        text: String,
        turn_id: Option<String>,
        message_id: Option<String>,
    },
    /// `args_json` = serialized arguments exactly in the backend schema (unrefined by agent).
    ToolCall {
        name: String,
        args_json: String,
        /// Call identifier (for permission UX and result matching). claude tool_use id etc.
        id: Option<String>,
        turn_id: Option<String>,
        message_id: Option<String>,
        /// ★Not an `Option`★ — the translator always picks one ([`ToolCategory::Other`] if unknown).
        category: ToolCategory,
    },
    /// The end result of one tool call — points at the earlier [`ToolCall`](Self::ToolCall) by `id` (not a new row).
    ///
    /// ★`id` is not an `Option`★ — if there is no call to point at, the translator does not emit this event.
    /// ★The absence of this event ≠ success★ — absence is 「unknown」 (the end did not come · pushed out of the ring · an old daemon · a backend that does not
    ///   emit ends).
    /// ★It also comes **after** the turn end (codex measured — an interrupted command keeps running and closes late)★ — so neither turn classifier counts it as progress
    ///   or as an error.
    // ADR-0241
    ToolResult { id: String, outcome: ToolOutcome },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        turn_id: Option<String>,
    },
    /// End signal of one message (turn response).
    MessageDone {
        turn_id: Option<String>,
        message_id: Option<String>,
    },
    /// The turn boundary + **how it ended**. Being a different type from [`MessageDone`](Self::MessageDone) is intended —
    /// that one is "one message closed" and this one is "one turn closed with this outcome".
    ///
    /// ★Some backends have several completed items in one turn (measured)★ — there **turn end ≠ message end**, so
    ///   folding the outcome into `MessageDone` would split one turn into several boundaries. And folding the outcome into
    ///   [`Error`](Self::Error) would **stamp a turn the user normally aborted as a failure.**
    /// ★`MessageDone` is not migrated to this★ — claude emits ends as `MessageDone` as is, and emits only the interrupted-turn
    ///   branch as this (`Interrupted`) (ADR-0238). The two vocabularies coexist, and which one is emitted is decided by each backend's
    ///   decoder.
    /// ★The outcome is **a neutral enum** — the backend's raw state string is not carried as is★ (ADR-0004).
    TurnEnd {
        turn_id: Option<String>,
        outcome: TurnOutcome,
    },
    /// An error the backend reported — ★**not a turn boundary**★ (separate from `TerminalReason` too).
    ///
    /// ★What each of the two vocabularies means — consumers split here★:
    ///   - `Error` = **an incident within the turn**. The same turn continues after it (a stream error that gets retried is
    ///     of this kind). Reading it as an end would split one turn into as many pieces as there were incidents.
    ///   - [`TurnEnd`](Self::TurnEnd) = **the turn ended**. A turn that ended in failure also comes in this vocabulary.
    /// ★So 「is it retried」 is not sent out as a separate field★ — the event type already carries that distinction.
    /// ★A failed claude turn comes as `Error` followed by [`MessageDone`](Self::MessageDone)★ — that backend, which does not put failures in `TurnEnd`
    ///   (only interrupted turns are `TurnEnd`), emits the two in this order from one failed `result` line, and the turn
    ///   classifier recognizes that `Error` by its prefix and folds the following end into an error end (`RESULT_FAILURE_DETAIL` of
    ///   `backend/claude`). ★An `Error` without that
    ///   prefix is not a turn error★ — a line-buffer overflow (`partial-line buffer overflow`) is an example.
    Error(String),
    /// Escape hatch (forward-compat) for per-backend structured events not caught by the formal variants above.
    /// kind = event kind tag, json = original serialized payload. core does not interpret the content.
    Structured { kind: String, json: String },
    /// Queued input registry event — backend-neutral. The canonical interpretation (reduction rules) is [`crate::queued_input`].
    ///
    /// ★Not carried in the [`Structured`](Self::Structured) escape hatch★ — the claude turn classifier counts every `Structured`
    ///   except the interrupted marker (`kind:"interrupted"` — ADR-0243) as a progress signal, so carried there, a registry event coming after the turn end
    ///   would turn 「in a turn」 back on.
    // ADR-0231
    QueuedInput(QueuedInputEvent),
}

/// The event [`OutputEvent::QueuedInput`] carries — the registry (`crate::queued_input`) and the frontend accumulator read it with the same reduction
/// rules. `id` = that input's identifier (claude = input uuid · codex = `clientId`).
///
/// ★Producer obligations — what the reduction rules rely on★:
///   - `Queued` is emitted **once** per id (no re-emission — the reducer drops a second one).
///   - `CancelRequested` is also once per id (the idempotency decision is on the producer side · the reducer is a second defense).
///   - `AckUnavailable.delivered` **is emitted empty by the decoder** — the body copies of the entries that event closes as accepted are
///     filled by the core at the moment it writes to the ring (so the bubbles can be redrawn even if the ring cap pushes out `Queued`).
/// ★`Delivered` carries no body★ — the bubble body is our local copy (the `text` of `Queued`) (the vendor may
///   normalize it — ADR-0198).
// ADR-0231
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueuedInputEvent {
    /// It goes onto the list.
    Queued { id: String, text: String },
    /// The cancel request was accepted — the outcome is decided by the event that follows.
    CancelRequested { id: String },
    /// Success response of the cancel request. `removed` = whether the vendor removed that text from its own queue.
    ///   ★Only `true` is an outcome (cancelled right away)★ — `false` is material for telling whether the vendor close that follows was our cancel.
    CancelAnswered { id: String, removed: bool },
    /// The cancel request itself failed (error response · cancel line write failure). ★It is not returned to the list★ — it stays pending cancel
    ///   as 「could not remove」, and the outcome is decided by the vendor lifecycle.
    CancelFailed { id: String },
    /// The link handed that text to the vendor (`sent: true` — wrote codex `turn/steer`·`turn/start`) · put it back where it held it
    ///   (`false` — the turn ended without an echo · steer refused · write failure). ★Not an outcome★ — the row stays on the list (a handed-over row hides its ✕)
    ///   and what removes it is still `Delivered`·`Dropped`·the cancel response. A no-op for an id not on the list.
    ///   ★The value is the link's current stage★ — it can come several times for the same id and the last one wins. claude does not emit it.
    HandedOver { id: String, sent: bool },
    /// The vendor put it into a turn → it leaves the list and becomes a bubble at the end of the conversation.
    Delivered { id: String },
    /// Not delivered. `Rejected` also erases the bubble drawn for that id (the accumulator's drawing rule).
    Dropped { id: String, cause: DropCause },
    /// This incarnation cannot give per-entry acks — every open entry is closed as a bubble at that ring position.
    AckUnavailable { delivered: Vec<DeliveredCopy> },
}

/// The cause of [`QueuedInputEvent::Dropped`]. ★Only `Unknown` leaves a revivable tombstone★ — a late vendor
/// ack beats 「unknown」 (the tombstone rule of `crate::queued_input`).
// ADR-0231
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropCause {
    /// The user withdrew it (before it reached the vendor).
    Withdrawn,
    /// A turn interrupt closed it.
    Interrupted,
    /// The agent ended (core exit synthesis · seal).
    AgentEnded,
    /// The vendor or link refused it (including write failures).
    Rejected,
    /// Unknown whether the vendor received it.
    Unknown,
}

/// The bubble copy [`QueuedInputEvent::AckUnavailable`] carries — our local body as is.
// ADR-0231
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveredCopy {
    pub id: String,
    pub text: String,
}

/// **How** the turn ended — the neutral vocabulary [`OutputEvent::TurnEnd`] carries.
///
/// ★What is neutral is **the discriminant** — not `detail`★ (ADR-0004): each backend decoder moves the other protocol's state strings
///   into these four branches, and that is the only **axis** the outside (core · wire · frontend) **branches on**.
/// ★`detail` carries the human-readable sentence the other side made as is★ — the failure reason the other side gave is the only
///   information, so moving it into a neutral vocabulary would require inventing meaning. The grade of that field is **the same** as the string of
///   [`OutputEvent::Error`] (opaque · for drawing only). Length and credential masking are carried by the backend decoder that made it.
/// ★Do not fold [`Unknown`](Self::Unknown) into one of the three known ones★ — folding would require guessing the meaning of an unknown value,
///   and if that guess is wrong the screen draws **a false outcome**. The fact of not knowing itself is carried.
// ADR-0004
// ADR-0045
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    /// Normal end.
    Completed,
    /// Failure. `detail` = **the human-readable reason the other side made** (may be absent — when only the failure is known without a reason).
    ///   ★Not a neutral vocabulary — do not branch on it, only draw it★ (type doc).
    Failed { detail: Option<String> },
    /// The user interrupted it. ★Not a failure★ — in this repository, aborting is a first-class normal path
    ///   (the same reason `TerminalReason::Interrupted` exists separately).
    Interrupted,
    /// The outcome cannot be known — the other side gave a value we do not know, or gave none at all.
    /// ★The turn is still counted as ended★: dropping the event because the outcome is unknown would keep that conversation's waiting indicator spinning forever.
    Unknown,
}

/// Neutral category of a tool call — carried by [`OutputEvent::ToolCall`]. Decided by each backend translator.
///
/// ★Vendor tool names and item types do not come here★ (ADR-0004) — the name tables and decisions are inside `backend/claude` · `backend/codex`,
///   and the outside (core · daemon · wire · frontend) knows only these nine branches.
// ADR-0239
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCategory {
    Read,
    Search,
    List,
    Edit,
    Command,
    Web,
    Agent,
    Mcp,
    /// A tool that belongs to none of the categories above or that the translator does not know — if the vendor renames or adds tools, they fall here.
    Other,
}

/// Neutral outcome of a tool call's end result — carried by [`OutputEvent::ToolResult`].
///
/// ★Vendor end-state strings do not come here★ (ADR-0004) — the decision ends inside each backend.
// ADR-0241
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOutcome {
    /// Completed normally. ★No translator emits it now★ — only failures and refusals are carried, and this word is kept only as wire vocabulary.
    Completed,
    Failed,
    /// Not run — a refusal not attributed to our refusal (the vendor's own refusal · vendor preparation failure · our refusal whose
    ///   attribution memory was lost). The reason is unknown.
    Declined,
    /// Not run because we (the host) refused the approval request. ★The translator does not emit it★ — attribution is the job of the codex link,
    ///   which remembers the refused item (ADR-0241 decision 8 — after translation it rewrites `Failed`·`Declined` as this).
    Refused,
}

/// session→transport input event. An extensible enum.
#[derive(Debug, Clone)]
pub enum InputEvent {
    Raw(Vec<u8>), // PTY keystroke bytes
}

/// **From whom** the input came — received by the session write verbs.
///
/// ★`Mail` never goes onto the queued list in any backend★ — non-human callers (mail delivery · pilot) always take today's
///   path. Caller table: WS `WriteStdin` = `User` · all `*_observed` verbs = `Mail`.
// ADR-0231
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputOrigin {
    User,
    Mail,
}

/// **Who classifies and who releases** input typed mid-turn — the backend reports it at spawn (ADR-0004 shape).
///
/// ★The session decides only by this value — it does not branch on the encoder tag or backend name★. Branching that way would leak backend knowledge
///   into the session.
// ADR-0231
#[derive(Debug, Clone, Copy)]
pub enum MidTurnPolicy {
    /// Today's path as is (terminal · shell) — no list and nothing to cancel.
    None,
    /// The session classifies inside the input lock and the vendor releases (claude JSON). `cancel_line` = one stdin line
    /// requesting the cancellation of that id (newline included — the session `send_input`s it as is).
    SessionClassified { cancel_line: fn(&str) -> Vec<u8> },
    /// The link carries classification, release and cancellation (codex JSON) — the session hands over [`TurnInput`] and only hands cancellation
    /// to `AgentTransport::withdraw`. It does not take the input lock (order is carried by the link state lock).
    /// ★The session does not count the submissions of the session id first-submission latch either★ — a backend reporting this policy must plug the first-turn port
    ///   into its link (the pairing discipline of `backend::FirstTurnSink` · ADR-0226 amendment).
    TransportOwned,
}

/// Whether this incarnation **can give per-entry acks** — three incarnation-shared values. The backend fills it and the session and link read it.
///
/// ★The transition happens only from `Unknown` — once decided it is not reverted★. If `Available` were reverted, the decisions of the entries
///   listed before then would diverge (the ack-unavailable verdict applies once to 「all entries listed until then」).
/// ★The ring order of the ack-unavailable verdict is set not by this atomic value but by the `AckUnavailable` in the ring★ — the core does not read
///   this value (the list event door of `OutputCore`).
// ADR-0231
#[derive(Debug, Default)]
pub struct DeliveryAck(std::sync::atomic::AtomicU8);

/// The value of [`DeliveryAck`].
// ADR-0231
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DeliveryAckState {
    /// Not known yet — before the incarnation's first decision.
    Unknown = 0,
    Available = 1,
    /// An old version — goes with today's behavior as is.
    Unavailable = 2,
}

impl DeliveryAck {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> DeliveryAckState {
        match self.0.load(std::sync::atomic::Ordering::Acquire) {
            1 => DeliveryAckState::Available,
            2 => DeliveryAckState::Unavailable,
            _ => DeliveryAckState::Unknown,
        }
    }

    /// `Unknown → Available`. `true` = this call made the transition. If already decided (either way), a no-op.
    pub fn set_available(&self) -> bool {
        self.leave_unknown(DeliveryAckState::Available)
    }

    /// `Unknown → Unavailable`. ★Exactly one call receives `true`★ — that winner emits `AckUnavailable` once.
    /// After `Available` it is always `false` (not reverted).
    pub fn try_set_unavailable(&self) -> bool {
        self.leave_unknown(DeliveryAckState::Unavailable)
    }

    fn leave_unknown(&self, to: DeliveryAckState) -> bool {
        self.0
            .compare_exchange(
                DeliveryAckState::Unknown as u8,
                to as u8,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .is_ok()
    }
}

/// One turn a `TransportOwned` session hands to the link (`AgentTransport::send_turn`).
///
/// `id` = this input's identifier — the `WriteOutcome::msg_uuid` string of this write (the same value as the id of the list event) · `body` =
/// the bytes after the encoder.
// ADR-0231
#[derive(Debug, Clone)]
pub struct TurnInput {
    pub id: String,
    pub body: Vec<u8>,
    pub origin: InputOrigin,
}

/// The answer of `AgentTransport::withdraw`.
// ADR-0231
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Withdraw {
    /// The link erased the entry it held — the vendor never saw that text.
    Withdrawn,
    /// Already handed over — the link cannot withdraw it and the row stays on the list (the outcome is the vendor's — accepted, or withdrawn when it turns out not accepted).
    TooLate,
    /// An id the link does not know (already has an outcome or is off the list).
    NotHeld,
}

/// The shape of an **accepted** queued input cancellation (`AgentSession::cancel_queued_input`).
// ADR-0231
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// Cancellation requested (or it was already pending cancel) — the outcome is decided by the list event that follows.
    Requested,
    /// Cancelled right away (the link withdrew the entry it held).
    Cancelled,
}

impl CancelOutcome {
    /// The `outcome` word of the cancel response — the bus and WS carry the same word.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Failure of a queued input cancellation.
// ADR-0231
#[derive(Debug, thiserror::Error)]
pub enum CancelError {
    /// Not on the list — an unknown id · an id that already has an outcome · a session that does not use the list.
    #[error("queued input not found")]
    NotFound,
    /// Could not write the cancel line. ★The entry stays pending cancel★ — it is not returned to the list.
    #[error("cancel write failed: {0}")]
    Write(PtyError),
}

/// The exit reason the transport produces (flat). core maps it to AgentStatus (finalize once).
/// ※ Raw lib errors (reqwest/nix) must not be exposed directly — use domain strings.
#[derive(Debug, Clone)]
pub enum TerminalReason {
    Exited { code: Option<i32> },
    Killed,
    Interrupted,
    StreamClosed,
    Cancelled,
    Error(String),
}

/// User intent — filled by the kill handler (ADR-0019). It is **separated** from the PTY-observed fact (TerminalReason):
/// inferring the intent by observing the exit would misclassify a daemon-shutdown Job-kill as a user kill, so the intent is
/// tagged explicitly at "the action point that caused the exit" (the kill command handler).
/// `#[repr(u8)]` — kept and snapshotted per session as `Arc<AtomicU8>`. DaemonShutdown is separated into the global
/// `shutting_down` flag (not kept here).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminationIntent {
    None = 0,
    UserKill = 1,
}

impl TerminationIntent {
    /// Unknown values are conservatively None (= the crash-treatment path).
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => TerminationIntent::UserKill,
            _ => TerminationIntent::None,
        }
    }
}

/// The exit event the pump emits once when it is the finish winner (consumed solely by the ADR-0019 reaper).
///
/// ★Key to race prevention★: `intent_at_finish`/`shutting_down_at_finish` are frozen values snapshotted at **the very moment of finish**.
/// If the reaper read them live at reap time, a race would arise: "after dying by crash and before the reaper handles it, the user
/// kills → the crash is misclassified as a user kill → profile deleted (data loss)" (found in a GPT consult).
#[derive(Debug, Clone)]
pub struct ReapMsg {
    pub id: AgentId,
    /// Checks the epoch matches before reaping so a stale done cannot wrongly delete a respawned new session (ADR-0007).
    pub epoch: u32,
    pub reason: TerminalReason,
    pub intent_at_finish: TerminationIntent,
    pub shutting_down_at_finish: bool,
}

/// Exit classification result (ADR-0019 §decide). reap_one applies it to the ProfileRegistry outside the lock.
/// ★Having no delete disposition is intended (ADR-0083)★ — the reaper does not auto-delete a profile on any exit.
/// Profile deletion is done only by an explicit user command (AgentCommand::DeleteProfile / Tauri delete_profile)
/// calling ProfileRegistry::try_remove directly, without going through this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Every runtime exit (user kill · normal exit · crash · EOF · signal) → keep the profile + auto_restore=false
    /// (keep the corpse — resumed with --resume on reactivation).
    KeepDisableAutoRestore,
    /// Daemon shutdown → left untouched (auto_restore=true remains → boot restore).
    KeepAsIs,
}

/// Neutral execution spec injected into the transport. Produced by the backend. PtyTransport does not know claude/codex.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: std::path::PathBuf,
}

// ── ADR-0086: control channel entry (MCP) — agent seam ──────────────────────────────────────
//
// ★Why agent has an abstract descriptor + seam★: for a spawned agent to attach to the daemon's control channel (MCP entry),
//   (a) the daemon issues a token per (AgentId, epoch) and (b) that token + endpoint must be injected into the backend command line.
//   But **token issuing, the MCP server and the mcp-config file** are all daemon concerns (rmcp/axum/HTTP), so
//   bringing them into agent would break the same spirit as the tauri-import-0 isolation (no dependency on transport or infrastructure). So with
//   **the same idiom (ADR-0003)** as OutputSink/StatusSink, agent knows only a pure trait (`ControlChannel`) + an abstract
//   descriptor (`ControlEndpoint`), and the real implementation is provided by the daemon (`DaemonControlChannel`).

/// The **bare executable name** the spawned agent **actually types** in the shell (no path or extension). The stem of the deployed executable,
/// the exe of `ToolGrant::Cli`, the name the backend makes resolvable by adding to PATH, and the command the priming teaches —
/// the four must be **literally the same** (ADR-0094 alignment invariant).
///
/// ★Do not retype it anywhere★: if the name diverges by even one place, the agent calls a command not on PATH or not covered by the grant,
///   and mail **silently stops** without an error (ADR-0099 measured: 6 of 7 not sent).
/// ★What CI catches now / does not catch★: the grant string and the PATH resolution name are derived from this constant and cannot diverge separately,
///   and **shipped file name ↔ constant** (`engram-dashboard-cli/tests/engram_cli.rs`) and **priming ↔ constant** (daemon
///   `control/priming.rs` pin) are tests that run without claude, so CI sees them. The axis left outside CI is **whether the agent actually
///   resolves and runs that name** — because the real claude spawn tests are excluded since the runner has no claude, and
///   that is confirmed only by local measurement.
// ADR-0094
pub const CLI_EXE_NAME: &str = "engram";

/// The **logical name** of the daemon MCP server (the `mcpServers` key · the server slot of `mcp__{server}__{tool}`).
///
/// ★The definition is here because there are consumers outside the daemon too★: the daemon authors the mcp-config JSON with this name
///   and fills `ToolGrant::Mcp`, and the backend folder also assembles **its program's MCP
///   attachment syntax** with the same name (codex's `-c mcp_servers.<name>=…`). The backend crate cannot depend on the daemon
///   (dependency direction), so if the name stayed there, a **second copy** would appear on the backend side.
/// ★If they diverge the symptom is quiet★: if the server name mismatches, the grant points at a nonexistent server, and on the codex side
///   the tools come up under a wrong name matching neither the priming nor the grant — neither branch raises an error.
// ADR-0094
// ADR-0209
pub const MCP_SERVER_NAME: &str = "engram";

/// CLI mail group name — the middle token of `engram mail <verb>` (ADR-0132 group structure).
///
/// ★Why agent knows this★: the deciders guarding the **education↔wiring equality** of the mail channel (ADR-0128/0099) must
///   **find the mail CLI education in the priming body**, and if that decision token diverges from the notation the CLI actually accepts, the decision
///   silently flips. So the CLI's dispatch and those deciders look at the same value.
/// ★That decision cannot be made from the bare executable name alone★: the MCP priming legitimately uses the same word as the MCP **server name**
///   (`on the engram server`) — so the decision is made by the **adjacency** of `CLI_EXE_NAME` + this token.
// ADR-0132
pub const CLI_GROUP_MAIL: &str = "mail";

/// The **silence limit** (seconds) the CLI endures on the control route — the upper bound of the daemon deadline.
///
/// ★Not 「a total budget」★: consumers put this into `set_read_timeout` (+`set_write_timeout`), so the firing
/// condition is 「after the request is sent, **one read** receives not a single byte within this time」 — it measures not the duration of the whole
/// round trip but **a continuous no-response interval**. Today the control route streams no bytes before the answer, so the two
/// values are effectively the same, but do not write sentences relying on that coincidence (if intermediate bytes appear, the total duration can exceed this value
/// without the client cutting off).
/// ★Why both sides must look at the same value★: the daemon **answers with `TIMEOUT`** on a round trip past its deadline, but if
/// the CLI cuts the socket before that answer goes out, nobody sees it — to the user it looks like 「could not reach the daemon」
/// (when in fact it did reach and the command may have been applied), and the daemon side loses the place to send the reply. It actually
/// was inverted that way: both sides held 10 seconds each, but the CLI's clock started **first** (connect · parse · pool scheduling come
/// before the daemon deadline) and the daemon's answer went out only after the collection period (1 second), so the client deterministically cut off first.
/// ★So the relation is an inequality★: `daemon deadline + collection period + margin ≤ this value`. The one place enforcing that inequality is the daemon
/// side (`fits_caller_silence_window` of daemon `command_delivery` — the default by a `const` assertion,
/// injected values decided in the constructor). Shrinking this value breaks that assertion and stops the build. **Do not write that arithmetic
/// again here** — the day the two copies diverge, neither can be trusted.
/// ★Why an integer in seconds★: `Duration` has a different grain from the other CLI vocabulary in this agent list, and letting the two consumers
/// each wrap it in their own type keeps this constant a pure **number**.
pub const CLI_CONTROL_READ_TIMEOUT_SECS: u64 = 10;

/// The variable name used when carrying the **absolute path** of the CLI executable in the spawn env.
///
/// The backend puts the value under this name (claude), and the priming deciders look for this name in the body to see "whether the CLI surface
/// is written" — the two places must look at the same value so the decision does not diverge from the actual wiring.
/// ★The name merely appearing is not education★: what counts as send education is only **the call form** (the form where the group token follows
///   this variable). The canonical decision rule is daemon `control/priming.rs`.
// ADR-0132
pub const CLI_EXE_ENV: &str = "ENGRAM_CLI_EXE";

/// The variable name used when carrying the control channel bearer token in the spawn env.
///
/// ★There is a place where the name itself became a contract — hence a constant★: codex receives, in its MCP server settings, **this variable name**
///   instead of the token **value** (`bearer_token_env_var`) and reads it from env itself. That is, the side planting it
///   ([`crate::backend::inject_cli_entrance`]) and the side writing that name into the settings (backend/codex) must look at one
///   value.
/// ★If they diverge it is quiet★: if the name mismatches, codex attaches without an Authorization header and the daemon returns 401, and
///   that 401 shows up not on the agent screen but only as an MCP server startup failure.
// ADR-0086
// ADR-0209
pub const TOKEN_ENV: &str = "ENGRAM_TOKEN";

/// All verbs of the mail group — `engram mail <verb>`.
///
/// Two consumers must look at the same value: the CLI parser (what it accepts) · the priming decider (whether the body taught an **executable**
/// call). ★The third, 「the user guidance text」, went away with ADR-0220 decision 5 — the help screen no longer teaches the CLI mail
/// verbs.★ If the decider looked only up to the group token, it would count **a verb-less fragment** like `engram mail` as
/// education too, but that is a command that does not run, so "taught" would be false.
// ADR-0132
pub const CLI_MAIL_VERBS: [&str; 3] = ["send", "status", "pending"];

/// All mail-CLI-only flag notations (kebab).
///
/// The set the parser recognizes, and the vocabulary the priming decider uses to see "whether the CLI surface is written". The MCP entry receives the same
/// concept as a snake_case JSON field (`reply_to`), so **the notation axis tells the two entries apart**.
/// ★If the parser's match arms and this list diverge★, the value-slot defense (the accident of swallowing a flag as a value) and the priming decision
///   miss the new flag — the drift test of `engram-dashboard-cli/src/bin/engram.rs` catches that mismatch.
// ADR-0132
pub const CLI_MAIL_FLAGS: [&str; 6] = [
    "--to",
    "--body",
    "--body-stdin",
    "--request",
    "--reply-by",
    "--reply-to",
];

/// CLI control group name — the middle token of `engram agent <verb>`.
///
/// ★Unlike the mail group (`CLI_GROUP_MAIL`), it is not used for the priming decision★: for mail, the decider must keep the "taught channel = laid wiring"
///   equality (ADR-0128), so agent had to know that token, but control has no such equality
///   (ADR-0132 decision 5 — permission is open to everyone). The only reason it is here is to make **the CLI parser, help and drift test look at one
///   string**.
// ADR-0132
pub const CLI_GROUP_AGENT: &str = "agent";

/// All verbs of the control group — `engram agent <verb>`.
///
/// ★The absence of `kill`·`rm` is not a missing implementation but **a deferred decision**★: whether removing from the tree ends the agent's life
///   (ADR-0122) is still out of line with the code (the current `try_delete_agent` deletes only the profile and leaves the process),
///   so putting those two here would expose that inconsistency through a new entry. Do not add them to this list before that decision stands.
// ADR-0132
// ADR-0122
pub const CLI_AGENT_VERBS: [&str; 5] = ["list", "spawn", "new", "rename", "move"];

/// All control-group-only flag notations (kebab) — the set the parser recognizes and the vocabulary of the value-slot defense
/// (the same discipline as `CLI_MAIL_FLAGS`, kept separately per group — because one group's flag must not steal another group's
/// value slot).
// ADR-0132
pub const CLI_AGENT_FLAGS: [&str; 3] = ["--cwd", "--name", "--parent"];

/// The help screen route — the CLI POSTs without a session and the daemon produces the screen.
///
/// ★Body = `{"topic": null | "<word>"}` · success response = `{"screen": "<screen>"}` (key names = the two constants below)★.
///   `topic` is required (null = table of contents screen), and if there are other keys the daemon rejects with `INVALID_ARGUMENT` — so that a CLI
///   whose key spelling diverged does not silently get the table of contents. The rejection is the same `{"status":"error",…}` envelope as the sibling
///   control routes.
/// ★Both sides look at path and keys through this one copy★ — retyping the literals would let an edit fixing only one side show up only as a 404 or a key
///   rejection. The five sibling control routes (`/control/send` etc.) are still matched by hand between the CLI and the daemon
///   (the `CONTROL_HELP_PATH` doc of `mcp_server.rs` · moving them to shared constants is a follow-up — TRD S21 2-3 §8 O15).
// ADR-0285
pub const CLI_HELP_ROUTE: &str = "/control/help";
/// The screen word key of the [`CLI_HELP_ROUTE`] request body.
pub const CLI_HELP_TOPIC_KEY: &str = "topic";
/// The screen body key of the [`CLI_HELP_ROUTE`] success response.
pub const CLI_HELP_SCREEN_KEY: &str = "screen";

/// The wire notation of the **agent state axis** (alive · asleep · absent) carried in control responses.
///
/// ★Why it is in agent★: the producer of this string (the table of `agent::commands`) and the consumer (the CLI's response decider) are **different
///   crates**, so if each wrote the literal, nobody would notice when only one side changed — the symptom is a false alarm where a normal response bounces as
///   "an unreadable shape" (exit 2). Both sides look at one value.
/// ★Having no value corresponding to "absent" is intended★ — absence is expressed not by a state value but by a rejection code
///   (`NOT_FOUND`). It is not mixed with the message outcome vocabulary (`delivered`/`pending`/`failed`) (ADR-0116).
// ADR-0132
pub const AGENT_STATE_LIVE: &str = "live";
pub const AGENT_STATE_SLEEPING: &str = "sleeping";

/// The `outcome` vocabulary of the rename response — owned by agent for the same reason as above. The two failure branches (absent · name space exhausted)
/// go out as rejection codes, so they are not here.
// ADR-0132
pub const RENAME_OUTCOME_RENAMED: &str = "renamed";
pub const RENAME_OUTCOME_UNCHANGED: &str = "unchanged";

/// ADR-0094: the abstract spec (backend-agnostic) of one tool to **pre-approve** for a spawned agent. The daemon control channel
/// fills it next to its own entry definition, and the backend translates it into its program's syntax (claude = `--allowedTools`).
///
/// ★Why an abstract enum (single source · isolation)★: only the control channel knows the **identity** of the send entry (which tool of which MCP server, which CLI exe)
///   — the tool name (`eg_send`), server name (`engram`) and CLI path are canonical in its definitions.
///   agent carries that identity only as data (server/tool/exe strings) and does not know the concepts of "permission" or "allowlist".
///   backend/claude/ only translates this data into claude syntax (`mcp__{server}__{tool}` / `Bash({exe}:*)` +
///   `PowerShell({exe}:*)`) — it does not retype names (ADR-0004 isolation + ADR-0094 single-source invariant).
/// ★Least privilege (ADR-0094)★: this list holds only send-entry tools — widening this *list* requires an explicit decision (ADR-0094 amendment).
///   Note: by the user decision of 2026-07-22, the spawn itself runs under `--permission-mode bypassPermissions` (auto) —
///   this grant now remains not as a runtime gate but as **a policy surface + documentation for a future shared constraint layer**
///   (see backend/claude/, step-log backlog "constraint layer shared by all LLMs").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolGrant {
    /// One tool of an MCP server. The backend translates it into `mcp__{server}__{tool}` (claude).
    Mcp { server: String, tool: String },
    /// One CLI executable (allows commands starting with that exe — bare name, resolved through the PATH the backend injects).
    /// The backend translates it into two patterns, `Bash({exe}:*)` **and** `PowerShell({exe}:*)` (claude — two shell tool shapes).
    Cli { exe: String },
}

/// The control channel endpoint the daemon issues (an abstract descriptor). The backend receives it and injects it into the command line/env
/// its program's way (claude = `--mcp-config <path>` — that knowledge is backend/claude/ alone,
/// ADR-0004). agent/transport carry only url/token/path strings and know neither "MCP" nor claude flags.
///
/// ★Each field has a different consumer — do not read 「the backend translates」 as 「one backend writes all fields」★:
/// `token`·`send_exe`·`mail_allowed` below are written by **the backend-shared injection** (`backend::inject_cli_entrance`),
/// and `config_path`·`priming_file`·`settings_file`·`grants` are translated only into their own flags by the folder of the program
/// that accepts them. The grounds for that division is the sentence the `config_path` doc right below already wrote.
#[derive(Debug, Clone)]
pub struct ControlEndpoint {
    /// The agent that was issued this endpoint and its incarnation — the field holding as a value the lifetime unit (`(AgentId,epoch)`)
    /// of the `token` below. Incarnation markers are compared only for match/mismatch (ADR-0163).
    ///
    /// ★Carrying it in the descriptor is the decision — do not revert to receiving it separately as a backend assembly argument★:
    ///   that way this pair would reach spawns **without** a control channel too, and 「carried only together with a control channel」
    ///   would have to be rewritten as a conditional in every backend (the condition ADR-0217 decision 5 guards). Here,
    ///   one `Option<ControlEndpoint>` answers that condition with a type.
    // ADR-0217
    pub agent_id: AgentId,
    /// Paired with `agent_id` above — that doc is canonical.
    pub epoch: u32,
    /// The daemon MCP Streamable HTTP endpoint URL (e.g. `http://127.0.0.1:<port>/mcp`).
    pub url: String,
    /// The bearer token dedicated to this (AgentId,epoch) (carried in the HTTP Authorization header).
    /// ★Security★: this value is not logged (recorded only in the mcp-config file — the file is deleted on revoke).
    pub token: String,
    /// Per-agent mcp-config file path (the daemon creates it and deletes it on revoke). backend/claude/ writes
    /// url+token into this file and injects it with `--mcp-config`.
    /// ★Option = absence encoded in the type (ADR-0099)★: for an MCP-capable backend (claude) `Some(path)` (the mcp-config
    ///   physically exists), for a non-MCP backend (codex/gemini stub) `None` — the mcp-config is **not written at all** (the MCP
    ///   entry physically deleted). The backend injects `--mcp-config` only when `Some` (no empty-path defense code needed —
    ///   the type enforces it).
    /// ★This Option means only **whether there is an MCP entry** (ADR-0133)★ — not whether there is CLI wiring. The CLI is laid down
    ///   for everyone in both branches, and mail availability is decided by the daemon's credential refusal. The daemon writes the config only when
    ///   MCP-capable and cuts provision with Err on a failure of that write, so a half MCP endpoint does not exist.
    pub config_path: Option<std::path::PathBuf>,
    /// ADR-0086 step 2 (CLI entry): the absolute path of the `engram` CLI binary the daemon located (if any).
    /// It is a sibling of the daemon exe and lives with it when deployed, but may be missing in a partial build etc. → if `None`, the CLI entry injection
    /// skips that env (`CLI_EXE_ENV`) and the PATH prepend (`backend::inject_cli_entrance` — shared by backends).
    /// ★Consumption condition = every spawn with a control endpoint (ADR-0133 · ADR-0132 decision 5)★: the control verbs are open to everyone,
    /// so both branches use this value. The knowledge of finding the sibling exe is owned by the daemon (lib.rs).
    pub send_exe: Option<std::path::PathBuf>,
    /// Whether this spawn **can use the CLI/HTTP mail entry (`engram mail …`)** (ADR-0133 decision 2) — a **fact** the daemon
    /// decides and carries, which this crate neither derives nor re-derives.
    ///
    /// ★Narrower than its name — it is not "can it use mail"★: a spawn with `false` (= an MCP-capable backend) **uses mail normally**
    ///   through MCP tools. The channel is decided only by backend capability with no runtime fallback (ADR-0128
    ///   decision 1) — this value points only at the entry (CLI/HTTP) that must be closed in that design.
    ///
    /// ★No production code reads this field today — but it was not deleted★: the same decision is baked into the credentials
    ///   and the daemon refuses (`control/mcp_server.rs`), and this field remains as **the record** of that credential axis.
    ///   At one point the backend translated this value into a spawn env marker to hide mail from the CLI usage, but that marker,
    ///   being the agent's own env, enforced nothing while leaving open an entry for the outside to choose the screen
    ///   (removal = user decision 2026-09-23). ★Do not bring that translation back★ — Rust does not warn when there is no longer a reader,
    ///   so this sentence is the only record of "why it remained".
    // ADR-0133
    pub mail_allowed: bool,
    /// ADR-0092 (receive-contract priming): **the absolute path of the priming MD file** to inject into the system prompt at spawn
    /// (if any). The daemon's `PrimingProvider` seam resolves it and carries it — `None` if the file is missing/unconfigured.
    /// backend/claude/ injects this path as `--append-system-prompt-file <abs-path>` (claude
    /// **reads the file directly** — the daemon/agent do not read the content). It is broker-injected data orthogonal to MCP, but
    /// it reuses this descriptor the daemon already fills for every claude spawn instead of making a separate threading
    /// path.
    pub priming_file: Option<std::path::PathBuf>,
    /// The list of tools to pre-approve (ADR-0094 — the contract is `ToolGrant`). The daemon control channel fills the send entries (MCP
    /// `eg_send` / `engram` CLI). If the Vec is empty, the backend injects nothing
    /// (no permission flags = existing gate kept).
    pub grants: Vec<ToolGrant>,
    /// S18 D (spec §6 allowedMcpServers measure): **the absolute path of a settings fragment file** to put only on the spawned session (if any).
    /// The daemon writes it to `<data_dir>/mcp-config/<id>-<epoch>.settings.json` at provision and deletes it at revoke.
    /// backend/claude/ translates it to `--settings <abs-path>` (that flag knowledge is there alone — ADR-0004).
    ///
    /// ★Why it is needed (measured 2026-07-24)★: the user-global setting `allowedMcpServers: []` (= block everything) **applied
    ///   to spawned agents as is**, so the engram MCP server did not appear in the tool list. This fragment allows the engram server only for that
    ///   session — **the global settings file is never touched** (allowed scope = only agents engram
    ///   spawned). It has the same lifetime as config_path (created and discarded per epoch), so it rides the same descriptor.
    pub settings_file: Option<std::path::PathBuf>,
}

impl ControlEndpoint {
    /// Whether the daemon **authorized the send entry of our MCP server** for this spawn.
    ///
    /// ★This is **the only real thing** of 「MCP mail availability」 the backend can see★: the only place making that decision is
    ///   the daemon's `DaemonControlChannel::build_grants` (the operator knob `ENGRAM_DISALLOW_MCP_SEND`, and
    ///   the harness knob `ENGRAM_FORCE_CLI_ONLY_SEND` flipping `accepts_mcp_config`, all pass through that one
    ///   place and appear in this list), and its result comes in as `grants`. ★Do not read those environment variables
    ///   again in the backend★ — the owner of the decision is the daemon, and if the backend re-derived it there would be two authorities (ADR-0004).
    /// ★Do not read it through `mail_allowed` instead★ — that field means **CLI mail** authorization (ADR-0133), and in a spawn
    ///   using MCP mail it is always `false`, so it cannot tell MCP turned off from turned on.
    /// ★Tool names are not looked at — only the server name★: the canonical source of that name is the daemon side (`SEND_MESSAGE_TOOL`), and
    ///   this crate carries it only as data (the isolation discipline of the [`ToolGrant`] doc above). The server name is canonical here,
    ///   [`MCP_SERVER_NAME`], so it is not a retyping.
    // ADR-0094
    // ADR-0133
    // ADR-0209
    pub fn grants_mcp_send(&self) -> bool {
        self.grants
            .iter()
            .any(|g| matches!(g, ToolGrant::Mcp { server, .. } if server == MCP_SERVER_NAME))
    }
}

/// The reason a control channel provision failed (ADR-0086 fail-closed). The case where there "**was an intent to attach** the control channel
/// **but it failed**", such as a file write or CSPRNG failure — when spawn meets this Err it aborts the spawn fail-closed (no
/// agent running without a control channel is made). ★Distinct from absence★: Ok(None) = legitimate absence of not using a control channel
/// (Noop · shell), Err = failed while trying to use it (fatal). agent carries only strings (prevents rmcp/io type leaks, ADR-0003).
#[derive(Debug)]
pub struct ProvisionError(pub String);

impl std::fmt::Display for ProvisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "control channel provision failed: {}", self.0)
    }
}

impl std::error::Error for ProvisionError {}

/// **What this spawn receives** from the control channel — all the axes provision splits on.
///
/// ★Three fields, not one — do not merge them★: once, a single `accepts_mcp_config` drove the MCP entry, the priming
///   variant, the grants and **mail availability** all together. Because of that folding, a backend 「using the control channel but outside the mail plane」
///   **opened send authorization along with it** the moment it turned control on (with receiving closed). That asymmetry was written
///   in no declaration — because there were two axes but one field.
/// ★The third field split off in the same shape★: while `accepts_mcp_config` doubled as 「does it use mail through MCP」 and
///   「does it read the file we wrote」, a backend turning MCP on by another mechanism (codex's
///   `-c mcp_servers.engram=…`) got **a plaintext token file nobody reads** on every spawn just by turning that field on to match the mail axis
///   (default ACL · a failure of that write, being fail-closed, cut even the spawn).
/// ★Still, **the derivation point of `mail_allowed` is still one** (ADR-0133 decision 2)★: the daemon makes one value from these two fields,
///   and that one value produces **together** the enforcement baked into the credentials and the marker carried on the endpoint. What changed is
///   only the number of ingredients of that formula. Deciding separately in two places would split education and enforcement.
/// ★The `engram` CLI wiring (env · PATH) is laid down in every combination★ — because the control verbs are open to everyone
///   (ADR-0132 decision 5). What this type splits is only **mail and MCP**.
// ADR-0099
// ADR-0126
// ADR-0133
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlChannelNeeds {
    /// Whether this backend **can consume the mcp-config file we make** (ADR-0099).
    ///
    /// true → mcp-config + session settings fragment recorded + MCP endpoint bits. false → none of the three is made.
    /// ★Not "can it use MCP"★ — a program turning MCP on by another mechanism also has this field false
    ///   (the canonical decision is in each backend folder).
    pub accepts_mcp_config: bool,
    /// Whether the daemon **will write the mcp-config file** for this spawn (`AgentBackend::writes_mcp_config_file`).
    ///
    /// true → records `<data_dir>/mcp-config/<id>-<epoch>.json` (plaintext Bearer token) and the session settings fragment,
    /// and a failure of that write is **fail-closed** (spawn aborted). false → makes neither, and `config_path`·
    /// `settings_file` go out as `None`.
    /// ★Do not **fold it into one field** with the field above★: the one above is the axis deriving the mail channel decision (`mail_allowed`), and
    ///   this field asks only **whether to write a plaintext token to disk**. 「uses MCP but does not read our file」
    ///   (codex) is the normal value of that combination — the third-field item of the type doc is the canonical reasoning.
    // ADR-0209
    pub writes_mcp_config_file: bool,
    /// Whether this spawn **is assigned a mail channel** (`AgentBackend::uses_mail`).
    ///
    /// false → neither the mail education (priming) nor the send grant is carried, and the daemon refuses mail requests with these credentials.
    /// ★It still receives the control verbs and the CLI entry★ — the last item of the type doc above.
    // ADR-0209
    pub uses_mail: bool,
}

/// Control channel provisioning seam (ADR-0086). AgentManager default = `NoopControlChannel`.
pub trait ControlChannel: Send + Sync + 'static {
    /// Issues a token for (AgentId,epoch), creates the mcp-config file (if MCP-capable) and returns the endpoint.
    /// Called on the spawn path right before spec assembly. Three return values (fail-closed contract, ADR-0086):
    ///   - `Ok(Some(ep))` — control channel issued (the backend injects it).
    ///   - `Ok(None)`     — **legitimate absence** of not using a control channel (Noop · shell-only · unconfigured). Spawn continues.
    ///   - `Err(_)`       — **failed** while trying to use the control channel (CSPRNG/file write error). ★Fatal★ — when spawn
    ///     meets this Err it aborts fail-closed (no agent secretly running without a control channel, no health disguise).
    ///
    /// `needs` = what this spawn receives from the control channel ([`ControlChannelNeeds`] — that type doc is the canonical source of the meaning
    ///   of the fields and how they split). The manager fills it through backend dispatch and passes it.
    // ADR-0126
    // ADR-0133
    fn provision(
        &self,
        id: AgentId,
        epoch: u32,
        needs: ControlChannelNeeds,
    ) -> Result<Option<ControlEndpoint>, ProvisionError>;

    /// Discards the token of (AgentId,epoch) and deletes the mcp-config file. The reaper calls it on any terminal (kill · crash · EOF · normal
    /// exit), so nothing is missed. kill_agent and the spawn failure guard also call it pre-emptively, so duplicate calls come for the same
    /// (id,epoch) (absorbed by remove-if-present). It also receives the epoch so a stale terminal cannot erase a live token newly attached
    /// by reactivation (a new incarnation = a different marker) (the spirit of the ADR-0007/0084 epoch-guard
    /// — the decision is **match/mismatch**. Markers have no order).
    fn revoke(&self, id: AgentId, epoch: u32);
}

/// No-op implementation for paths that do not use a control channel (headless tests · shell-only). provision always returns Ok(None)
/// (legitimate absence — not a failure), revoke is a no-op. The AgentManager default — only the daemon injects the real
/// `DaemonControlChannel`.
pub struct NoopControlChannel;

impl ControlChannel for NoopControlChannel {
    fn provision(
        &self,
        _id: AgentId,
        _epoch: u32,
        _needs: ControlChannelNeeds,
    ) -> Result<Option<ControlEndpoint>, ProvisionError> {
        Ok(None)
    }
    fn revoke(&self, _id: AgentId, _epoch: u32) {}
}

/// Per-area capabilities (no bool explosion). Serialized (shared with the frontend, snake_case).
///
/// ★Source separation (load-bearing)★: the 5 areas of this composite come from **two sources** — input/output/control are decided
/// by the physical channel (transport), and session/model by the program (backend). Previously the transport
/// hardcoded even session.resume (resume=true regardless of claude/shell), so the shell backend was inaccurate.
/// Now it is built only through `Capabilities::compose(TransportCaps, BackendCaps)`, enforcing the source by type
/// (ADR-0030 capability matrix).
// ADR-0030
#[derive(Debug, Clone, serde::Serialize)]
pub struct Capabilities {
    pub input: InputCaps,
    pub output: OutputCaps,
    pub control: ControlCaps,
    pub session: SessionCaps,
    pub model: ModelCaps,
}

/// session/model being absent here is intended — the transport cannot make them (ownership enforced by type).
#[derive(Debug, Clone, serde::Serialize)]
pub struct TransportCaps {
    pub input: InputCaps,
    pub output: OutputCaps,
    pub control: ControlCaps,
}

/// Capabilities of the target program (claude/shell/codex…). input/output/control being absent here is intended —
/// the backend cannot make them (ownership enforced by type).
#[derive(Debug, Clone, serde::Serialize)]
pub struct BackendCaps {
    pub session: SessionCaps,
    pub model: ModelCaps,
}

impl Capabilities {
    /// **The only normal construction path** of Capabilities — pinned by type so the sources do not mix.
    pub fn compose(t: TransportCaps, b: BackendCaps) -> Capabilities {
        Capabilities {
            input: t.input,
            output: t.output,
            control: t.control,
            session: b.session,
            model: b.model,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct InputCaps {
    pub raw: bool,
    pub message: bool,
    pub attachment: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct OutputCaps {
    pub terminal_bytes: bool,
    /// Reports that the output is a structured stream (NDJSON etc.) and needs parsed rendering (RichSlot) instead of terminal rendering (ADR-0044).
    /// The frontend's `defaultRenderMode` picks the renderer by this one value (true=RichSlot / false=xterm, ADR-0002).
    /// Not content interpretation (the link-unrefined invariant) — just a render hint saying "this byte stream is not a terminal".
    pub structured: bool,
    pub markdown: bool,
    pub tool_events: bool,
    pub usage: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ControlCaps {
    pub resize: bool,
    pub interrupt: bool,
    pub cancel: bool,
    pub graceful_shutdown: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionCaps {
    pub resume: bool,
    pub snapshot: bool,
    pub cwd_env: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelCaps {
    pub select: bool,
    pub temperature: bool,
    pub max_tokens: bool,
}

/// Raw PTY output chunk for internal drain delivery — binary as is (prevents UTF-8 splitting)
#[derive(Debug, Clone, serde::Serialize)]
pub struct OutputChunk {
    pub seq: u64,
    pub data: Vec<u8>,
}

/// The PTY output wire format going to the frontend — base64-encoded for JSON compatibility.
/// ※S12: this is a representation **only for Embedded (Tauri JSON Channel)**. base64 is a JSON Channel constraint and
/// not a core concern — ChannelOutputSink receives OutputFrame (raw) and encodes it into this.
///
/// ★epoch★: like the WS binary frame header ([tag][agentId][epoch][seq]), it carries the session epoch on every output frame
/// (OutputFrame.epoch as is). If frame.epoch were **dropped** when encoding,
/// embedded would flow with epoch fixed at 0, mismatching SubscribeAck.current_epoch≥1 (resume-fallback), and
/// the ProtocolClient epoch guard (f.epoch !== st.epoch) would wipe out the output (Stage 3
/// BLOCKER 1). So frame.epoch must always be included, matching the WS path.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PtyEvent {
    pub agent_id: AgentId,
    pub seq: u64,
    pub epoch: u32,
    pub data_b64: String,
}

/// core→sink output payload (S15 B5 payload-generic). Passed **borrowed** — core does not know the wire (ADR-0003).
/// ★ADR-0002 (no assumption about output kind)★: output is not forced into terminal bytes — split into two branches Bytes/Event
/// so the sink handles each kind (Bytes→tag0 terminal frame, Event→tag1 structured frame, B7).
/// Holds only references to stay Copy (preserving the OutputFrame Copy contract) — no Serialize attached (an agent domain type, ADR-0003).
#[derive(Debug, Clone, Copy)]
pub enum OutputPayload<'a> {
    /// Console raw bytes (terminal · tag0 path). The payload of PtyTransport and terminal mode.
    Bytes(&'a [u8]),
    /// Structured event (tag1 path — encoded by B7). An OutputEvent the backend decoder parsed.
    Event(&'a OutputEvent),
}

/// core→sink output boundary (S15 B5 payload-generic). Passes **the payload borrowed** — Copy (references only), so zero
/// copies on fanout. agent_id/epoch carry as is the immutable values OutputCore holds (for the daemon frame header).
#[derive(Debug, Clone, Copy)]
pub struct OutputFrame<'a> {
    pub agent_id: AgentId,
    pub epoch: u32,
    pub seq: u64,
    pub payload: OutputPayload<'a>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentInfo {
    pub id: AgentId,
    /// canonical display name = display_name(override) ?? basename(cwd) — the same string as the frontend tree and routing
    /// (ADR-0101). Degrades to the first 8 characters of the id only when there is no cwd segment to derive.
    pub name: String,
    pub cwd: String,
    pub status: AgentStatus,
    pub cols: u16,
    pub rows: u16,
    /// A random marker drawn anew per incarnation — not an ordered increment (ADR-0163). Not put into the frontend resubscription deps
    /// (ADR-0164 decision 8; the old statement S9 §18-a is stale).
    pub epoch: u32,
    pub capabilities: Capabilities,
    /// Whether this agent is **a reader of mail** (= eligibility for the mail recipient list). The value the session received from the backend
    /// at spawn is **carried as is in this snapshot**.
    ///
    /// ★Why it is carried in the snapshot (load-bearing — do not revert to re-querying)★: if a consumer received the list and then asked the manager back
    ///   per entry, ① between the eligibility check and the injection the same id could split into a different session (TOCTOU),
    ///   ② if the reaper collected the session between the two queries, that entry would drop out of both the live list and the asleep one, and mail
    ///   that would have been parked and delivered on reappearance would be **lost** as "no such recipient", and ③ N per-session lock acquisitions would come back
    ///   (`AgentManager::roster` doc — the mail-sending critical path). Coming out of one snapshot, none of the three holds.
    /// ★Not a capability★: `capabilities` is "what it can do", this is "what the input is interpreted as".
    ///   So it is a separate field, not inside `Capabilities`, and it does not go out on the wire either (a daemon-internal decision axis).
    pub reads_messages: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("agent not found: {0}")]
    NotFound(AgentId),
    #[error("spawn failed: {0}")]
    SpawnFailed(String),
    #[error("write failed: {0}")]
    WriteFailed(String),
    #[error("cwd outside workspace")]
    CwdDenied,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// The transport does not support that action (an ApiTransport stub etc.). A per-verb unsupported signal.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// ★It hit the total roster cap and **refused to register a new agent** (runaway backstop — `AgentManager::MAX_ROSTER_SIZE`)★.
    ///
    /// ★Why a dedicated variant★: callers must tell this apart from name space exhaustion (`Unsupported`) — what the user should do
    ///   is opposite (retry with a different name vs delete agents to free a slot). Telling them apart by string matching
    ///   silently breaks the moment the text changes.
    /// ★It does not apply to existing agents★: the cap looks only at **new registrations**. Restore and respawn of a roster already over the cap
    ///   (replacing the incarnation of the same id) pass through as is — if the backstop took the existing team hostage, recovery would be impossible.
    #[error("roster is full: {current} agents (ceiling {limit}) — refusing to register another")]
    RosterFull { current: usize, limit: usize },
    /// ★The profile store refused to save this change (`ReadOnly`) or could not write it (`Io`) — the in-memory registry is unchanged★.
    ///
    /// Only the registry's entries with a caller (`try_` verbs) emit this — internal changes without a caller apply to memory on any
    ///   save `Err` and set dirty, so they do not become errors.
    /// ★The launch path has partial success★: if the save of clearing the session id fails after the roster registration was saved, the registration stays
    ///   (it is not reverted) and only the launch stops — canonical = the `spawn_agent_watching_link` doc of `manager`.
    /// ★Why a dedicated variant★: the bus must carry a save refusal with a code different from other failures (the caller has different things to do) —
    ///   for the same reason as `RosterFull`, it is split by variant, not by string.
    // ADR-0291 R17
    #[error("agent store: {0}")]
    Store(crate::profile::StoreError),
    // ★Do not turn duplicate spawn requests back into an error here★: "already up / already launching" is not a failure but
    //   **a request with nothing to do**, so it is answered with `Ok(manager::SpawnOutcome::Moot)`. When it was an error, every consumer
    //   had to carry a list of "this error is not a real failure", and over the three times that list was fixed, one item was missed each time
    //   and a failure marker was stamped on a live agent (ADR-0172).
}

/// Subscription replay branch result (core-neutral — the daemon maps it to protocol::SubscribeAction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayKind {
    /// Full replay from the beginning (oldest) — a new subscription or an epoch mismatch.
    FromOldest,
    /// after_seq is older than the ring's oldest → from oldest (front part lost).
    Truncated,
    /// Lossless continuation from after_seq+1 (tail only).
    Resumed,
}

/// subscribe_from result metadata (the daemon uses it to build the SubscribeAck).
#[derive(Debug, Clone, Copy)]
pub struct SubscribeOutcome {
    pub kind: ReplayKind,
    pub sink_id: SinkId,
    pub oldest_seq: u64,
    pub latest_seq: u64,
    /// The seq of the chunk actually sent first. If nothing was sent — for an empty ring, the next seq to issue (the value read under the replay lock
    /// · ADR-0231); if there is no tail to continue, `after_seq+1`.
    pub replay_from: u64,
    /// Number of chunks actually sent (can be 0).
    pub replayed: usize,
}

/// The **incarnation facts** a subscription response carries — a value type. Not a session field.
///
/// ★Do not put a field of this type in the session★: the home of the incarnation marker is `AgentSession::epoch` alone. Bringing this value in as a session
///   field would make two markers, and if the two diverged nobody would know which is the incarnation. It is assembled on the spot when the response
///   is built.
// ADR-0226
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Incarnation {
    pub epoch: u32,
    /// This incarnation launched to resume a stored conversation (the handle to resume was carried at spawn).
    /// ★It does not mean the resume succeeded★.
    pub continues_conversation: bool,
}

/// The result of `subscribe_from` — the replay result and the facts of **the incarnation that did that replay**.
///
/// ★The two fields come from one session lookup★: looked up separately, the incarnation could change in between, and then the marker would
///   speak of a different incarnation from the one that replayed.
// ADR-0226
#[derive(Debug, Clone, Copy)]
pub struct SubscribeReply {
    pub outcome: SubscribeOutcome,
    pub incarnation: Incarnation,
}

/// Delivery-boundary measurement output of an input write (ADR-0088 Stage 0).
///
/// ★Why it exists★: if `write_input`/`write_stdin` returned only `Ok(())`, there would be no evidence to tell "not inserted due to a send failure" from
///   "fully inserted but ignored by the model". For the delivery accuracy harness (ADR-0088) to tell these two
///   apart, the **completeness signal** (full acceptance vs failure) and this user turn's replay-dedup key
///   (`msg_uuid`) must be made observable at the write boundary. This value is that output (returned only on the success path).
///
/// ★★What `Ok` means **depends on which verb it was obtained from** — this paragraph is that split, and the old statement is false★★:
///   at one point this said 「the transport's `send_input` writes with `write_all` (+`flush`)」, but now that all three links
///   (PTY · stdio · codex) put input into **a bounded input queue** that a dedicated writer pops and writes, that is not true.
///   The split is this:
///   - The `Ok` of `AgentSession::submit_input_observed` (= the verb mail delivery uses, `AgentManager::submit_stdin_observed`)
///     = ★**it actually went out to the OS**★. That verb issues the receipt only after confirming the landing with `AgentTransport::flush_input`
///     — the "send failure" vs "ignored by the model" ADR-0088 wants to tell apart stands on that confirmation.
///   - The `Ok` of `AgentSession::write_input_observed` (the keystroke streaming version) = **the whole thing was accepted with its order settled**.
///     That path does not wait (if it waited, the per-connection dispatch consumer would stand still entirely on one stuck agent).
///   ★A link without a means of confirmation stays at the old level★ — for the codex app-server and the test-rig seam, `flush_input` is
///     `Unsupported`, so even the delivery verb knows only up to 「accepted」. **For those links this record does not assert landing**
///     (pre-existing — those links' queues existed before this discipline).
///
/// ★The completeness signal = Ok-vs-Err, not a byte comparison (important)★: on either side of the split above, partial acceptance is not hidden as `Ok`
///   — either the whole thing is accepted (or the whole thing goes out) or it is `Err`. So the only evidence of that decision is
///   `Ok(WriteOutcome)` **itself** (vs `Err`) — not a comparison of the two byte fields below. The real written
///   byte count is not threaded out of the transport (unnecessary under the write_all contract), so `bytes_written` is
///   not an independent measurement but a value **copied as is from `bytes_requested` by construction** (short writes cannot be detected —
///   comparing them always gives equal, a tautology). This field exists only for the self-description of the observation record (in logs and forensics,
///   the by-construction identity "this much was requested to write and write_all was Ok"), not as a completeness decision
///   lever. Completeness must look at `Ok`.
///
/// ★Byte units · layer★: the two values here are **the byte length of the logical message the caller handed to the session boundary**
///   (`bytes.len()` — **before** encoder wrapping, not a char count). When the encoder wraps text (json mode),
///   the actual wire bytes are larger, but that encoded wire count is not measured here (not this layer's logical
///   unit). The daemon layer's `DeliveryObservation` uses the same "logical message bytes" meaning
///   (there that logical message = the bytes of the envelope string built by `wrap_message`).
#[derive(Debug, Clone, Copy)]
pub struct WriteOutcome {
    pub bytes_requested: usize,
    pub bytes_written: usize,
    /// The message uuid of this user turn (replay-dedup key, generated by session.write_input — LOAD-BEARING).
    /// The delivery harness correlates the ingress's logical msg_id with this value to decide "did claude replay this turn" (=
    /// did it actually parse it) (ADR-0088). The value and meaning are not changed here — only exposed.
    pub msg_uuid: uuid::Uuid,
    /// ★The epoch of the incarnation where the write actually landed (ADR-0088 Stage 1)★. A value carrying by construction the
    /// `self.epoch` of the session that performed this write (the same nature as bytes_written — not an independent measurement, but
    /// the session that executed the write fills in its own epoch as is). ★Why it is needed★: for the control channel delivery observation record
    /// to be **record-self-sufficient**, "which incarnation was this message inserted into" must be answerable directly within the record
    /// — only by holding the epoch of the session **where the write actually landed**, not the epoch snapshotted at resolve time,
    /// can the mid-flight epoch race (a restart between resolve↔write) be settled from the record alone (that race is marked design-accepted by
    /// ADR-0086 §F5 — mail is aimed at the logical agent, so landing on a new
    /// incarnation is the correct behavior). The value is filled only by the session that executed the write (success path only),
    /// so it can diverge from resolve-time, and it exists precisely to observe that divergence.
    pub epoch: u32,
}

/// OutputSink send failure signal — triggers removal of that subscriber when drain detects it
#[derive(Debug)]
pub struct SinkError;

/// PTY output delivery abstraction — isolated so headless tests are possible without a Tauri dependency.
/// ※S12: wire encoding is owned by the implementation (ChannelOutputSink = base64 PtyEvent / the daemon frame sink = binary
/// frame) → core is transport-agnostic.
pub trait OutputSink: Send + Sync + 'static {
    /// ★Contract = never blocks — if it cannot send, do not wait; `SinkError`★. There are two places where this call is made while holding
    /// an order lock (both exceptions to ADR-0006 「send without holding a lock」) — the session's input lock (`input_order` —
    /// the `SessionClassified` path) and the codex link's emit lane (`Announcer.order` — `announce`). An implementation waiting here stops that incarnation's input and cancellation (for codex,
    /// the input and the writer's emits) together.
    // ADR-0231
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError>;
    fn sink_id(&self) -> SinkId;
}

/// Agent status change notification abstraction — pty/ notifies the upper layer without an AppHandle
pub trait StatusSink: Send + Sync + 'static {
    /// Carries the epoch (S9 §18-d): lets the frontend discard a delayed terminal notification of an old session after respawn
    /// by the epoch mismatch (stale Killed defense, fable C-1/Mn-1).
    fn status_changed(&self, id: AgentId, status: AgentStatus, epoch: u32);
    fn agent_list_updated(&self, agents: Vec<AgentInfo>);
    /// Restore attempt result notification (S9 §18-d). Default no-op — sinks that do not use restore need not implement it.
    fn restore_result(&self, _report: crate::profile::RestoreReport) {}
    /// The backend observed a usage limit (meaning of missing fields = [`crate::usage::UsageObservation`]). Default no-op.
    ///
    /// ★The contract is the same as [`Self::turn_ended`] (non-blocking · non-reentrant)★ — the caller is the output pump, so stopping here stops
    ///   that incarnation's output along with it.
    fn usage_observed(&self, _obs: crate::usage::UsageObservation) {}
    /// This incarnation just ended a turn (ADR-0113 — the push exit of turn observation). Default no-op.
    ///
    /// ★Contract = non-blocking · non-reentrant (ADR-0006 callback discipline — never violate)★: what an implementation can do is at most a non-blocking
    ///   notification (channel send). Doing IO, injection or reentrant calls makes the thread that called this callback stop
    ///   until that work finishes.
    /// ★That thread is the output pump **today** — but the structure does not guarantee it (load-bearing)★: `emit` has
    ///   two callers (the output pump · the injection thread that emitted the input echo — turn.rs header), and this hook is called from whichever of them
    ///   emitted the end signal. The only reason it is fixed to the pump now is **that the input echo is classified as a progress signal**
    ///   (the backend classifier). If a new non-pump emit path emitting end signals appears, the weight of this contract moves to that thread
    ///   — then this must still be non-blocking so delivery work is not loaded onto the sending thread.
    /// ★It goes out on every end signal regardless of whether there was a transition★: a consumer that wants to narrow it to "only when it was in a turn just before" must
    ///   make that decision on its own side. Narrowing it here would drop the notification for a turn that ends right away without assistant events, and
    ///   while surplus notifications are usually harmless, a missing one makes the consumer wait forever.
    // ADR-0113
    fn turn_ended(&self, _id: AgentId, _epoch: u32) {}
    /// This incarnation's user waiting list just became empty — the doorbell that keeps parked mail from being tied up until the next turn end
    /// even when only the list empties without the turn ending. Default no-op.
    ///
    /// ★The contract is the same as [`Self::turn_ended`] (non-blocking · non-reentrant)★. ★It rings **after** 「empty」 is written
    ///   to the waiting-list table★ — the receiver asks about busy again right away. It goes out every time it empties (except when emptied by the exit synthesis —
    ///   `OutputCore::finish`), and surplus is absorbed by the consumer.
    /// ★Why a separate verb instead of widening `turn_ended`★: if the name 「the turn ended」 came to mean even the case of emptying without a turn,
    ///   consumers relying on that name would misjudge.
    // ADR-0231
    fn inputs_drained(&self, _id: AgentId, _epoch: u32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Capabilities::compose — whether the sources merge into the right areas (ownership composition check) ──
    #[test]
    fn compose_merges_each_source_into_its_region() {
        let t = TransportCaps {
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
                resize: true,
                interrupt: true,
                cancel: false,
                graceful_shutdown: false,
            },
        };
        let b = BackendCaps {
            session: SessionCaps {
                resume: true,
                snapshot: false,
                cwd_env: true,
            },
            model: ModelCaps {
                select: false,
                temperature: false,
                max_tokens: false,
            },
        };

        let caps = Capabilities::compose(t, b);

        assert!(
            caps.control.resize,
            "resize is decided by the transport → preserved in the composition"
        );
        assert!(
            caps.session.resume,
            "resume is decided by the backend → preserved in the composition"
        );
        assert!(caps.input.raw);
        assert!(caps.output.terminal_bytes);
        assert!(caps.session.cwd_env);
        assert!(!caps.model.select);
    }

    // ── DeliveryAck — leaves Unknown only once and does not go back ──
    #[test]
    fn delivery_ack_leaves_unknown_once_and_never_goes_back() {
        let ack = DeliveryAck::new();
        assert_eq!(ack.state(), DeliveryAckState::Unknown);
        assert!(ack.set_available(), "the first decision transitions");
        assert!(
            !ack.set_available(),
            "does not transition again with the same value"
        );
        assert!(
            !ack.try_set_unavailable(),
            "after Available it does not flip to ack-unavailable"
        );
        assert_eq!(ack.state(), DeliveryAckState::Available);

        let old_cli = DeliveryAck::new();
        assert!(
            old_cli.try_set_unavailable(),
            "the winner gets true — it emits AckUnavailable once"
        );
        assert!(!old_cli.try_set_unavailable(), "there is no second winner");
        assert!(
            !old_cli.set_available(),
            "after Unavailable it does not go to Available"
        );
        assert_eq!(old_cli.state(), DeliveryAckState::Unavailable);
    }

    #[test]
    fn exactly_one_racer_wins_the_unavailable_verdict() {
        let ack = std::sync::Arc::new(DeliveryAck::new());
        let winners: usize = (0..8)
            .map(|_| {
                let ack = ack.clone();
                std::thread::spawn(move || ack.try_set_unavailable())
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| usize::from(h.join().unwrap()))
            .sum();
        assert_eq!(
            winners, 1,
            "the ack-unavailable verdict happens only once per incarnation"
        );
        assert_eq!(ack.state(), DeliveryAckState::Unavailable);
    }
}
