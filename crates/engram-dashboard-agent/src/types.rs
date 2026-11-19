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
    /// ★Not "present in the session map" (load-bearing)★: a session stays in the map until the reaper collects it, so presence mixes in
    ///   corpses. The roster (`AgentManager::roster`) and the daemon adapter (`messaging_host::is_live` — only calls this) must see **the
    ///   same condition**. Canonical — no duplicates.
    // ADR-0116
    pub fn is_live(&self) -> bool {
        matches!(self, AgentStatus::Running | AgentStatus::Exiting)
    }
}

/// Internal pump→core output event. An extensible enum; core is variant-agnostic (_ => ignore).
///
/// ★ADR-0045 (output refining moved to the backend)★: the console stays `TerminalBytes` (VT byte stream); structured backends (claude
///   stream-json etc.) have their decoder emit the structured variants below.
/// **An agent domain type**, not a protocol wire type — agent↔wire conversion is the daemon adapter's (ADR-0003: core does not know the
///   wire). No tauri import in core (serde allowed).
///
/// `turn_id`/`message_id` are optional conversation-tracking fields — claude need not fill them; open to absorb codex/gemini turn/message
///   model leaks. None if the backend cannot fill them.
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
    /// ★`id` is not an `Option`★ — with no call to point at the translator does not emit this event.
    /// ★Absence ≠ success★ — absence is 「unknown」 (end did not come · pushed out of the ring · an old daemon · a backend that does not
    ///   emit ends).
    /// ★It also comes **after** the turn end (codex: an interrupted command keeps running and closes late)★ — so neither turn classifier
    ///   counts it as progress or error.
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
    /// The turn boundary + **how it ended**. Intentionally not [`MessageDone`](Self::MessageDone): that is "one message closed", this "one
    ///   turn closed with this outcome".
    ///
    /// ★Some backends have several completed items per turn (measured)★ — **turn end ≠ message end**, so folding the outcome into
    ///   `MessageDone` would split one turn into several boundaries; folding it into [`Error`](Self::Error) would **stamp a user-aborted
    ///   turn a failure.**
    /// ★`MessageDone` is not migrated to this★ — claude emits ends as `MessageDone` and only the interrupted-turn branch as this
    ///   (`Interrupted`, ADR-0238); the two coexist, each backend's decoder decides.
    /// ★The outcome is **a neutral enum** — the backend's raw state string is not carried★ (ADR-0004).
    TurnEnd {
        turn_id: Option<String>,
        outcome: TurnOutcome,
    },
    /// An error the backend reported — ★**not a turn boundary**★ (separate from `TerminalReason` too).
    ///
    /// ★Two vocabularies★:
    ///   - `Error` = **an incident within the turn**; the same turn continues (a retried stream error is this). Read as an end it would
    ///   split a turn into as many pieces as incidents.
    ///   - [`TurnEnd`](Self::TurnEnd) = **the turn ended**; a failed turn comes in this vocabulary too.
    /// ★「Is it retried」 is not a separate field★ — the event type carries it.
    /// ★A failed claude turn comes as `Error` followed by [`MessageDone`](Self::MessageDone)★ — claude puts failures not in `TurnEnd`
    ///   (only interrupted turns) but emits the two in this order from one failed `result` line; the turn classifier recognizes that
    ///   `Error` by prefix and folds the following
    ///   end into an error end (`RESULT_FAILURE_DETAIL` of `backend/claude`). ★An `Error` without that prefix is not a turn error★ — e.g.
    ///   a line-buffer overflow (`partial-line buffer overflow`).
    Error(String),
    /// Escape hatch (forward-compat) for per-backend structured events not caught by the formal variants above.
    /// kind = event kind tag, json = original serialized payload. core does not interpret the content.
    Structured { kind: String, json: String },
    /// Queued input registry event — backend-neutral. Canonical interpretation (reduction rules): [`crate::queued_input`].
    ///
    /// ★Not carried in the [`Structured`](Self::Structured) escape hatch★ — the claude turn classifier counts every `Structured` except
    ///   the interrupted marker (`kind:"interrupted"` — ADR-0243) as progress, so a registry event coming after the turn end would turn
    ///   「in a turn」 back on.
    // ADR-0231
    QueuedInput(QueuedInputEvent),
}

/// The event [`OutputEvent::QueuedInput`] carries — the registry (`crate::queued_input`) and the frontend accumulator read it with the
///   same reduction rules. `id` = that input's identifier (claude = input uuid · codex = `clientId`).
///
/// ★Producer obligations — what the reduction rules rely on★:
///   - `Queued` is emitted **once** per id (the reducer drops a second).
///   - `CancelRequested` is once per id too (idempotency on the producer side · the reducer is a second defense).
///   - `AckUnavailable.delivered` **is emitted empty by the decoder** — the core fills the body copies of the entries that event closes as
///   accepted when it writes to the ring (so bubbles redraw even if the ring cap pushes out `Queued`).
/// ★`Delivered` carries no body★ — the bubble body is our local copy (`Queued`'s `text`); the vendor may normalize it (ADR-0198).
// ADR-0231
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueuedInputEvent {
    /// It goes onto the list.
    Queued { id: String, text: String },
    /// The cancel request was accepted — the outcome is decided by the event that follows.
    CancelRequested { id: String },
    /// Success response of the cancel request. `removed` = whether the vendor removed that text from its own queue.
    ///   ★Only `true` is an outcome (cancelled right away)★ — `false` is material for telling whether the vendor close that follows was
    ///   our cancel.
    CancelAnswered { id: String, removed: bool },
    /// The cancel request itself failed (error response · cancel line write failure). ★It is not returned to the list★ — it stays pending cancel
    ///   as 「could not remove」, and the outcome is decided by the vendor lifecycle.
    CancelFailed { id: String },
    /// The link handed that text to the vendor (`sent: true` — wrote codex `turn/steer`·`turn/start`) · put it back where it held it
    ///   (`false` — turn ended without an echo · steer refused · write failure). ★Not an outcome★ — the row stays on the list (a
    ///   handed-over row hides its ✕) and
    ///   `Delivered`·`Dropped`·the cancel response still remove it. A no-op for an id not on the list. ★The value is the link's current
    ///   stage★ — it can come several times for one id, last wins. claude does not emit it.
    HandedOver { id: String, sent: bool },
    /// The vendor put it into a turn → it leaves the list and becomes a bubble at the end of the conversation.
    Delivered { id: String },
    /// Not delivered. `Rejected` also erases the bubble drawn for that id (the accumulator's drawing rule).
    Dropped { id: String, cause: DropCause },
    /// This incarnation cannot give per-entry acks — every open entry is closed as a bubble at that ring position.
    AckUnavailable { delivered: Vec<DeliveredCopy> },
}

/// The cause of [`QueuedInputEvent::Dropped`]. ★Only `Unknown` leaves a revivable tombstone★ — a late vendor ack beats 「unknown」
///   (tombstone rule of `crate::queued_input`).
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
/// ★What is neutral is **the discriminant**, not `detail`★ (ADR-0004): each backend decoder maps the other protocol's state strings into
///   these four branches, the only **axis** the outside (core · wire · frontend) **branches on**.
/// ★`detail` carries the other side's human-readable sentence as is★ — the failure reason it gave is the only information, and a neutral
///   vocabulary would need invented meaning. Same grade as the string of [`OutputEvent::Error`] (opaque · for drawing only); length and
///   credential masking belong to the
/// decoder that made it.
/// ★Do not fold [`Unknown`](Self::Unknown) into one of the three known ones★ — that would guess an unknown value's meaning and a wrong
///   guess draws **a false outcome**; the fact of not knowing is carried.
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
    /// ★The turn is still counted as ended★: dropping the event because the outcome is unknown would keep that conversation's waiting
    ///   indicator spinning forever.
    Unknown,
}

/// Neutral category of a tool call — carried by [`OutputEvent::ToolCall`], decided by each backend translator.
///
/// ★Vendor tool names and item types do not come here★ (ADR-0004) — name tables and decisions stay in `backend/claude` · `backend/codex`;
///   the outside (core · daemon · wire · frontend) knows only these nine branches.
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
    /// A tool that belongs to none of the categories above or that the translator does not know — if the vendor renames or adds tools,
    ///   they fall here.
    Other,
}

/// Neutral outcome of a tool call's end result — carried by [`OutputEvent::ToolResult`]. ★Vendor end-state strings do not come here★
///   (ADR-0004) — the decision ends inside each backend.
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
/// ★`Mail` never goes onto the queued list in any backend★ — non-human callers (mail delivery · pilot) take the plain path. Callers: WS
///   `WriteStdin` = `User` · all `*_observed` verbs = `Mail`.
// ADR-0231
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputOrigin {
    User,
    Mail,
}

/// **Who classifies and who releases** input typed mid-turn — the backend reports it at spawn (ADR-0004 shape).
///
/// ★The session decides only by this value, not by encoder tag or backend name★ — branching that way would leak backend knowledge into the
///   session.
// ADR-0231
#[derive(Debug, Clone, Copy)]
pub enum MidTurnPolicy {
    /// Today's path as is (terminal · shell) — no list and nothing to cancel.
    None,
    /// The session classifies inside the input lock and the vendor releases (claude JSON). `cancel_line` = one stdin line
    /// requesting the cancellation of that id (newline included — the session `send_input`s it as is).
    SessionClassified { cancel_line: fn(&str) -> Vec<u8> },
    /// The link carries classification, release and cancellation (codex JSON) — the session hands over [`TurnInput`] and only passes
    ///   cancellation to `AgentTransport::withdraw`; it does not take the input lock (order is carried by the link state lock).
    /// ★The session does not count the session id first-submission latch's submissions either★ — a backend reporting this policy must plug
    ///   the first-turn port into its link (pairing: `backend::FirstTurnSink` · ADR-0226 amendment).
    TransportOwned,
}

/// Whether this incarnation **can give per-entry acks** — three incarnation-shared values; the backend fills it, the session and link read it.
///
/// ★The transition happens only from `Unknown` — once decided it is not reverted★: reverting `Available` would diverge the decisions of
///   entries listed before (the ack-unavailable verdict applies once to 「all entries listed until then」).
/// ★The ack-unavailable verdict's ring order is set by the `AckUnavailable` in the ring, not by this atomic★ — the core does not read it
///   (the list event door of `OutputCore`).
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

/// One turn a `TransportOwned` session hands to the link (`AgentTransport::send_turn`). `id` = this input's identifier — the
///   `WriteOutcome::msg_uuid` string of this write (same as the list event id) · `body` = the bytes after the encoder.
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
    /// Already handed over — the link cannot withdraw it and the row stays on the list (the outcome is the vendor's — accepted, or
    ///   withdrawn when it turns out not accepted).
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

/// User intent — filled by the kill handler (ADR-0019), **separated** from the PTY-observed fact (TerminalReason): inferring intent from
///   the exit would misclassify a daemon-shutdown Job-kill as a user kill, so the intent is tagged at "the action point that caused the
///   exit" (the kill command handler).
/// `#[repr(u8)]` — kept per session as `Arc<AtomicU8>`. DaemonShutdown is the global `shutting_down` flag (not kept here).
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
/// ★Race prevention★: `intent_at_finish`/`shutting_down_at_finish` are frozen at **the very moment of finish**; read live at reap time,
///   "crash → user kills before the reaper handles it → crash misclassified as a user kill → profile deleted (data loss)" would arise.
#[derive(Debug, Clone)]
pub struct ReapMsg {
    pub id: AgentId,
    /// Checks the epoch matches before reaping so a stale done cannot wrongly delete a respawned new session (ADR-0007).
    pub epoch: u32,
    pub reason: TerminalReason,
    pub intent_at_finish: TerminationIntent,
    pub shutting_down_at_finish: bool,
}

/// Exit classification result (ADR-0019 §decide); reap_one applies it to the ProfileRegistry outside the lock.
/// ★No delete disposition is intended (ADR-0083)★ — the reaper never auto-deletes a profile on any exit; deletion is only an explicit user
///   command (AgentCommand::DeleteProfile / Tauri delete_profile) calling ProfileRegistry::try_remove directly, not through this enum.
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
// ★Why an abstract descriptor + seam★: for a spawned agent to attach to the control channel (MCP entry), (a) the daemon issues a token per
//   (AgentId, epoch) and (b) token + endpoint are injected into the backend command line; but **token issuing, the MCP server and the
//   mcp-config file** are daemon
//   concerns (rmcp/axum/HTTP), and bringing them into agent would break the tauri-import-0 isolation. So, in the same idiom as
//   OutputSink/StatusSink (ADR-0003), agent knows only a pure trait (`ControlChannel`) + an abstract descriptor (`ControlEndpoint`); the
//   daemon provides the real
//   implementation (`DaemonControlChannel`).

/// The **bare executable name** the spawned agent **actually types** in the shell (no path or extension). The stem of the deployed
///   executable, the exe of `ToolGrant::Cli`, the name the backend makes resolvable via PATH, and the command the priming teaches must be
///   **literally the same** (ADR-0094 alignment invariant).
///
/// ★Do not retype it anywhere★: one diverging place makes the agent call a command not on PATH or not covered by the grant, and mail
///   **silently stops** (ADR-0099 measured: 6 of 7 not sent).
/// ★What CI catches / does not★: the grant string and PATH resolution name derive from this constant; **shipped file name ↔ constant**
///   (`engram-dashboard-cli/tests/engram_cli.rs`) and **priming ↔ constant** (daemon `control/priming.rs` pin) run without claude. Outside
///   CI: **whether the agent actually
/// resolves and runs that name** — real claude spawn tests are excluded (no claude on the runner); local measurement only.
// ADR-0094
pub const CLI_EXE_NAME: &str = "engram";

/// The **logical name** of the daemon MCP server (the `mcpServers` key · the server slot of `mcp__{server}__{tool}`).
///
/// ★Defined here because consumers exist outside the daemon too★: the daemon authors the mcp-config JSON and `ToolGrant::Mcp` with it, and
///   the backend folder assembles **its program's MCP attachment syntax** with the same name (codex's `-c mcp_servers.<name>=…`). The
///   backend crate cannot depend on the
/// daemon, so a name kept there would need a **second copy**.
/// ★A divergence is quiet★: a mismatching server name points the grant at a nonexistent server, and on codex the tools appear under a
///   wrong name matching neither priming nor grant — no error either way.
// ADR-0094
// ADR-0209
pub const MCP_SERVER_NAME: &str = "engram";

/// CLI mail group name — the middle token of `engram mail <verb>` (ADR-0132 group structure).
///
/// ★Why agent knows this★: the deciders guarding the mail channel's **education↔wiring equality** (ADR-0128/0099) must **find the mail CLI
///   education in the priming body**, and a decision token diverging from the CLI's accepted notation silently flips the decision; so the
///   CLI dispatch and the deciders look at one value.
/// ★Not decidable from the bare executable name alone★: the MCP priming legitimately uses the same word as the MCP **server name** (`on
///   the engram server`), so the decision uses the **adjacency** of `CLI_EXE_NAME` + this token.
// ADR-0132
pub const CLI_GROUP_MAIL: &str = "mail";

/// The CLI's **silence window** on a control route (seconds) for every call that is not a launch — the upper bound of the
/// daemon's ordinary seat deadline. A launch waits [`LAUNCH_RELAY_CALLER_WAIT_SECS`] instead (ADR-0360, ADR-0369).
///
/// ★Not 「a total budget」★: consumers put it into `set_read_timeout` (+`set_write_timeout`), so it fires when, after the request is sent,
///   **one read** receives not a byte within this time — a **continuous no-response interval**, not the whole round trip. Today the
///   control route streams no bytes
/// before the answer, so the two coincide; do not rely on it (with intermediate bytes the total can exceed this without the client cutting off).
/// ★Both sides must look at the same value★: the daemon **answers `TIMEOUT`** on a round trip past its deadline, but a CLI cutting the
///   socket first hides it — to the user 「could not reach the daemon」 though the command may have applied. It happened: both held 10 s,
///   but the CLI's clock started **first** (connect · parse ·
/// pool scheduling precede the daemon deadline) and the daemon's answer left only after the 1 s collection period, so the client cut off
///   first deterministically.
/// ★So the relation is an inequality★: `daemon deadline + collection period + margin ≤ this value`, enforced in one place on the daemon
///   side (`fits_caller_silence_window` of daemon `command_delivery` — `const` assertion for the default, constructor for injected
///   values); shrinking this breaks the build. **Do not write that
/// arithmetic here again** — diverged copies cannot be trusted.
/// ★Integer seconds★: `Duration` has a different grain from the other CLI vocabulary here, and wrapping per consumer keeps this a pure
///   **number**.
pub const CLI_CONTROL_READ_TIMEOUT_SECS: u64 = 10;

// The launch time values (ADR-0360 decision 6). Whole seconds as `u64`, like `CLI_CONTROL_READ_TIMEOUT_SECS` — each
// consumer wraps them in its own type. Internal constants: they become settings only once file-based settings loading
// exists (tracking T-105). The two folder waits are the inputs; the reply bound and every launch wait are derived as
// `const` sums of one base and one margin so the chain cannot drift (ADR-0369 — a literal launch wait is a defect).

/// How long one launch waits, in total, for the folder IO whose failure blocks it — the library-root stat, the
/// folder-entry stats, listings and file reads, `prepare`'s material reads. What has not answered when it is spent
/// counts as unreadable and blocks the launch. 60 s = where the Windows SMB client gives up on a request by itself.
// ADR-0360
pub const LAUNCH_REQUIRED_FOLDER_WAIT_SECS: u64 = 60;

/// The separate allowance, per launch, for folder IO that only feeds a notice or a best-effort value (working-folder
/// canonicalize, the backend's own skill roots). Never taken from [`LAUNCH_REQUIRED_FOLDER_WAIT_SECS`]; what does not
/// answer in it is skipped, never a block.
// ADR-0360
pub const LAUNCH_ADVISORY_FOLDER_WAIT_SECS: u64 = 2;

/// The longest one launch may take to reply: the two folder waits + the resume early-exit window + the link
/// backstop (both in `manager.rs`, after the folder checks — summed as an upper bound; a launch takes at most
/// one of the two), plus a 5 s slack for parsing, locks, spawning and the reply (a failed link's 5 s teardown join
/// falls inside it) = 85.
// ADR-0360
pub const LAUNCH_REPLY_BOUND_SECS: u64 = LAUNCH_REQUIRED_FOLDER_WAIT_SECS
    + LAUNCH_ADVISORY_FOLDER_WAIT_SECS
    + crate::manager::EARLY_EXIT_WINDOW.as_secs()
    + crate::manager::LINK_RESOLUTION_BACKSTOP.as_secs()
    + LAUNCH_REPLY_SLACK_SECS;

/// The one margin of the launch-wait chain: the daemon's 1 s expiry sweep + its 2 s caller margin = 3 — the same
/// quantity as two daemon↔shell hops. Every step of the chain adds exactly this; where waits nest, outer = inner + this
/// (ADR-0360 decision 1 (b)).
///
/// ★A deliberate exception to the `CLI_CONTROL_READ_TIMEOUT_SECS` doc's clause 「do not write that arithmetic here again」★:
/// the two terms restate the daemon's private `SWEEP_INTERVAL` · `CALLER_MARGIN` (agent cannot see the daemon),
/// because the CLI and the shell need the derived values as constants. The pin is the daemon's
/// `fits_caller_silence_window`, which checks (bound, caller wait) and (relay seat, relay caller wait) at compile time.
// ADR-0369
pub const LAUNCH_MARGIN_SECS: u64 = DAEMON_SWEEP_SECS + DAEMON_CALLER_MARGIN_SECS;

/// How long a caller that waits **directly** on one launch waits: [`LAUNCH_REPLY_BOUND_SECS`] + [`LAUNCH_MARGIN_SECS`]
/// = 88 — the shell's wait for `Spawn` · `SpawnByCwd` · `SpawnProfile`, also inside the `agent.spawnInto` relay.
// ADR-0360
// ADR-0369
pub const LAUNCH_CALLER_WAIT_SECS: u64 = LAUNCH_REPLY_BOUND_SECS + LAUNCH_MARGIN_SECS;

/// The daemon's seat for a relayed launch-class name (`agent.spawnInto`): the owner's inner wait
/// [`LAUNCH_CALLER_WAIT_SECS`] + [`LAUNCH_MARGIN_SECS`] = 91.
// ADR-0369
pub const LAUNCH_RELAY_SEAT_SECS: u64 = LAUNCH_CALLER_WAIT_SECS + LAUNCH_MARGIN_SECS;

/// The CLI's wait for every launch verb: [`LAUNCH_RELAY_SEAT_SECS`] + [`LAUNCH_MARGIN_SECS`] = 94 — one value whether
/// the daemon answers the launch itself or relays it.
// ADR-0369
pub const LAUNCH_RELAY_CALLER_WAIT_SECS: u64 = LAUNCH_RELAY_SEAT_SECS + LAUNCH_MARGIN_SECS;

/// Parsing, locks, spawning and the reply itself inside [`LAUNCH_REPLY_BOUND_SECS`].
const LAUNCH_REPLY_SLACK_SECS: u64 = 5;
/// Restates daemon `CommandDeliveries::SWEEP_INTERVAL` (see [`LAUNCH_MARGIN_SECS`]).
const DAEMON_SWEEP_SECS: u64 = 1;
/// Restates daemon `CommandDeliveries::CALLER_MARGIN` (see [`LAUNCH_MARGIN_SECS`]).
const DAEMON_CALLER_MARGIN_SECS: u64 = 2;

// `as_secs` above drops any fraction, so both activation waits must be whole seconds.
const _: () = assert!(
    crate::manager::EARLY_EXIT_WINDOW.subsec_nanos() == 0
        && crate::manager::LINK_RESOLUTION_BACKSTOP.subsec_nanos() == 0,
    "an activation wait has a fraction of a second — LAUNCH_REPLY_BOUND_SECS would drop it"
);

/// The variable name used when carrying the **absolute path** of the CLI executable in the spawn env.
///
/// The backend puts the value under this name (claude), and the priming deciders look for this name in the body to see "whether the CLI
///   surface is written" — the two must look at one value.
/// ★The name merely appearing is not education★: only **the call form** (group token following this variable) counts as send education.
///   Canonical rule: daemon `control/priming.rs`.
// ADR-0132
pub const CLI_EXE_ENV: &str = "ENGRAM_CLI_EXE";

/// The variable name used when carrying the control channel bearer token in the spawn env.
///
/// ★The name itself became a contract — hence a constant★: codex receives **this variable name** in its MCP server settings instead of the
///   token **value** (`bearer_token_env_var`) and reads it from env itself; the planting side ([`crate::backend::inject_cli_entrance`])
///   and the settings side (backend/codex) must look at one value.
/// ★A divergence is quiet★: a mismatching name makes codex attach without an Authorization header and the daemon returns 401, visible not
///   on the agent screen but only as an MCP server startup failure.
// ADR-0086
// ADR-0209
pub const TOKEN_ENV: &str = "ENGRAM_TOKEN";

/// All verbs of the mail group — `engram mail <verb>`.
///
/// Two consumers must look at one value: the CLI parser (what it accepts) · the priming decider (whether the body taught an **executable**
///   call). A decider looking only up to the group token would count **a verb-less fragment** like `engram mail` as education, but that
///   command does not run. (The third,
/// the user guidance text, went with ADR-0220 decision 5 — help no longer teaches the CLI mail verbs.)
// ADR-0132
pub const CLI_MAIL_VERBS: [&str; 3] = ["send", "status", "pending"];

/// All mail-CLI-only flag notations (kebab).
///
/// The set the parser recognizes and the vocabulary the priming decider uses for "whether the CLI surface is written". The MCP entry
///   receives the same concept as a snake_case JSON field (`reply_to`), so **the notation axis tells the two entries apart**.
/// ★If the parser's match arms and this list diverge★, the value-slot defense (a flag swallowed as a value) and the priming decision miss
///   the new flag — the drift test of `engram-dashboard-cli/src/bin/engram.rs` catches it.
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
/// ★Unlike the mail group (`CLI_GROUP_MAIL`), not used for the priming decision★: mail's decider must keep the "taught channel = laid
///   wiring" equality (ADR-0128), but control has none (ADR-0132 decision 5 — permission open to everyone). It is here only so **the CLI
///   parser, help and drift test look at one string**.
// ADR-0132
pub const CLI_GROUP_AGENT: &str = "agent";

/// All verbs of the control group — `engram agent <verb>`.
///
/// ★The absence of `kill`·`rm` is **a deferred decision**, not a missing implementation★: whether removing from the tree ends the agent's
///   life (ADR-0122) is still out of line with the code (`try_delete_agent` deletes only the profile and leaves the process), and these
///   two would expose that inconsistency through a new entry.
/// Do not add them before that decision stands.
// ADR-0132
// ADR-0122
pub const CLI_AGENT_VERBS: [&str; 5] = ["list", "spawn", "new", "rename", "move"];

/// All control-group-only flag notations (kebab) — the set the parser recognizes and the value-slot defense's vocabulary (same discipline
///   as `CLI_MAIL_FLAGS`, kept per group so one group's flag cannot steal another's value slot).
// ADR-0132
pub const CLI_AGENT_FLAGS: [&str; 3] = ["--cwd", "--name", "--parent"];

/// The help screen route — the CLI POSTs without a session and the daemon produces the screen.
///
/// ★Body = `{"topic": null | "<word>"}` · success = `{"screen": "<screen>"}` (key names = the two constants below)★. `topic` is required
///   (null = table of contents), and other keys are rejected by the daemon with `INVALID_ARGUMENT` — so a CLI with diverged key spelling
///   does not silently get the table of contents.
///   The rejection is the same `{"status":"error",…}` envelope as the sibling control routes.
/// ★Both sides look at path and keys through this one copy★ — retyped literals would let a one-sided fix show up only as a 404 or key
///   rejection. The five sibling control routes (`/control/send` etc.) are still hand-matched between CLI and daemon (`CONTROL_HELP_PATH`
///   doc of `mcp_server.rs` · shared
/// constants are a follow-up — TRD S21 2-3 §8 O15).
// ADR-0285
pub const CLI_HELP_ROUTE: &str = "/control/help";
/// The screen word key of the [`CLI_HELP_ROUTE`] request body.
pub const CLI_HELP_TOPIC_KEY: &str = "topic";
/// The screen body key of the [`CLI_HELP_ROUTE`] success response.
pub const CLI_HELP_SCREEN_KEY: &str = "screen";

/// The wire notation of the **agent state axis** (alive · asleep · absent) carried in control responses.
///
/// ★In agent because producer (the `agent::commands` table) and consumer (the CLI's response decider) are **different crates**★ — separate
///   literals would let a one-sided change go unnoticed, showing as a false alarm where a normal response bounces as "an unreadable shape"
///   (exit 2).
/// ★No value for "absent" is intended★ — absence is a rejection code (`NOT_FOUND`), not mixed with the message outcome vocabulary
///   (`delivered`/`pending`/`failed`) (ADR-0116).
// ADR-0132
pub const AGENT_STATE_LIVE: &str = "live";
pub const AGENT_STATE_SLEEPING: &str = "sleeping";

/// The `outcome` vocabulary of the rename response — owned by agent for the same reason. The two failure branches (absent · name space
///   exhausted) go out as rejection codes, so they are not here.
// ADR-0132
pub const RENAME_OUTCOME_RENAMED: &str = "renamed";
pub const RENAME_OUTCOME_UNCHANGED: &str = "unchanged";

/// ADR-0094: the abstract spec (backend-agnostic) of one tool to **pre-approve** for a spawned agent. The daemon control channel fills it
///   next to its own entry definition and the backend translates it into its program's syntax (claude = `--allowedTools`).
///
/// ★Why an abstract enum (single source · isolation)★: only the control channel knows the send entry's **identity** (which tool of which
///   MCP server, which CLI exe) — tool name (`eg_send`), server name (`engram`) and CLI path are canonical in its definitions. agent
///   carries it only as data (server/tool/exe strings) and
///   does not know "permission" or "allowlist"; backend/claude/ translates it into claude syntax (`mcp__{server}__{tool}` /
///   `Bash({exe}:*)` + `PowerShell({exe}:*)`) without retyping names (ADR-0004 + ADR-0094 single-source).
/// ★Least privilege (ADR-0094)★: this list holds only send-entry tools — widening the *list* needs an explicit decision (ADR-0094
///   amendment). The spawn itself runs under `--permission-mode bypassPermissions` (auto), so this grant remains **a policy surface +
///   documentation for a future shared constraint layer**, not a
///   runtime gate (see backend/claude/).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolGrant {
    /// One tool of an MCP server. The backend translates it into `mcp__{server}__{tool}` (claude).
    Mcp { server: String, tool: String },
    /// One CLI executable (allows commands starting with that exe — bare name, resolved through the PATH the backend injects).
    /// The backend translates it into two patterns, `Bash({exe}:*)` **and** `PowerShell({exe}:*)` (claude — two shell tool shapes).
    Cli { exe: String },
}

/// The control channel endpoint the daemon issues (an abstract descriptor). The backend receives it and injects it into the command
///   line/env its program's way (claude = `--mcp-config <path>` — that knowledge is backend/claude/ alone, ADR-0004). agent/transport
///   carry only url/token/path strings and know neither "MCP" nor claude flags.
///
/// ★Each field has a different consumer — 「the backend translates」 does not mean 「one backend writes all fields」★:
///   `token`·`send_exe`·`mail_allowed` are written by **the backend-shared injection** (`backend::inject_cli_entrance`);
///   `config_path`·`grants` are translated into their own flags only by the
/// folder of the program that accepts them (grounds: the `config_path` doc below).
///
/// ★No priming file and no settings fragment ride here — do not add them back★: the launch materials carry the
/// instructions and the one `--settings`, and a launch without materials carries neither.
// ADR-0370
#[derive(Debug, Clone)]
pub struct ControlEndpoint {
    /// The agent that was issued this endpoint and its incarnation — the lifetime unit (`(AgentId,epoch)`) of the `token` below as a
    ///   value. Incarnation markers are compared only for match/mismatch (ADR-0163).
    ///
    /// ★Carrying it in the descriptor is the decision — do not revert to a separate backend assembly argument★: that pair would reach
    ///   spawns **without** a control channel too, and 「only together with a control channel」 would become a conditional in every backend
    ///   (the condition ADR-0217 decision 5 guards); one
    ///   `Option<ControlEndpoint>` answers it with a type.
    // ADR-0217
    pub agent_id: AgentId,
    /// Paired with `agent_id` above — that doc is canonical.
    pub epoch: u32,
    /// The daemon MCP Streamable HTTP endpoint URL (e.g. `http://127.0.0.1:<port>/mcp`).
    pub url: String,
    /// The bearer token dedicated to this (AgentId,epoch) (carried in the HTTP Authorization header).
    /// ★Security★: this value is not logged (recorded only in the mcp-config file — the file is deleted on revoke).
    pub token: String,
    /// Per-agent mcp-config file path (the daemon creates it, deletes it on revoke). backend/claude/ writes url+token into it and injects
    ///   it with `--mcp-config`.
    /// ★Option = absence encoded in the type (ADR-0099)★: `Some(path)` for an MCP-capable backend (claude), `None` for non-MCP
    ///   (codex/gemini stub) — the mcp-config is **not written at all**. The backend injects `--mcp-config` only when `Some` (no
    ///   empty-path defense — the type enforces it).
    /// ★Means only **whether there is an MCP entry** (ADR-0133)★, not whether there is CLI wiring: the CLI is laid down for everyone in
    ///   both branches and mail availability is the daemon's credential refusal. The daemon writes the config only when MCP-capable and
    ///   cuts provision with Err on a write failure,
    ///   so a half MCP endpoint does not exist.
    pub config_path: Option<std::path::PathBuf>,
    /// ADR-0086 step 2 (CLI entry): the absolute path of the `engram` CLI binary the daemon located (if any). A sibling of the daemon exe
    ///   deployed together, but may be missing in a partial build → `None` makes the CLI entry injection skip that env (`CLI_EXE_ENV`) and
    ///   the PATH prepend
    /// (`backend::inject_cli_entrance` — shared by backends).
    /// ★Consumed by every spawn with a control endpoint (ADR-0133 · ADR-0132 decision 5)★: control verbs are open to everyone, so both
    ///   branches use it. Finding the sibling exe is the daemon's knowledge (lib.rs).
    pub send_exe: Option<std::path::PathBuf>,
    /// Whether this spawn **can use the CLI/HTTP mail entry (`engram mail …`)** (ADR-0133 decision 2) — a **fact** the daemon decides and
    ///   carries; this crate neither derives nor re-derives it.
    ///
    /// ★Narrower than its name★: a spawn with `false` (= an MCP-capable backend) **uses mail normally** through MCP tools. The channel is
    ///   decided only by backend capability with no runtime fallback (ADR-0128 decision 1); this points only at the entry (CLI/HTTP) that
    ///   must be closed.
    ///
    /// ★No production code reads this field today — not deleted★: the same decision is baked into the credentials and the daemon refuses
    ///   (`control/mcp_server.rs`); this field remains **the record** of that credential axis. A backend once translated it into a spawn
    ///   env marker hiding mail from the CLI usage,
    ///   but that marker, being the agent's own env, enforced nothing and left an entry for the outside to choose the screen. ★Do not
    ///   bring that translation back★ — Rust does not warn when a reader is gone, so this sentence is the only record of why it remained.
    // ADR-0133
    pub mail_allowed: bool,
    /// The list of tools to pre-approve (ADR-0094 — the contract is `ToolGrant`). The daemon control channel fills the send entries (MCP
    ///   `eg_send` / `engram` CLI). An empty Vec makes the backend inject nothing (no permission flags = existing gate kept).
    pub grants: Vec<ToolGrant>,
}

impl ControlEndpoint {
    /// Whether the daemon **authorized the send entry of our MCP server** for this spawn.
    ///
    /// ★The only real thing of 「MCP mail availability」 the backend can see★: the decision is made only in the daemon's
    ///   `DaemonControlChannel::build_grants` (operator knob `ENGRAM_DISALLOW_MCP_SEND` and harness knob `ENGRAM_FORCE_CLI_ONLY_SEND`
    ///   flipping `accepts_mcp_config` both pass there and appear in this list) and arrives as
    ///   `grants`. ★Do not read those environment variables again in the backend★ — the owner is the daemon; a backend re-derivation would
    ///   make two authorities (ADR-0004).
    /// ★Do not read it through `mail_allowed` instead★ — that is **CLI mail** authorization (ADR-0133) and always `false` in an MCP-mail
    ///   spawn, so it cannot tell MCP off from on.
    /// ★Only the server name is looked at, not tool names★: tool names are canonical on the daemon side (`SEND_MESSAGE_TOOL`) and carried
    ///   here only as data ([`ToolGrant`] doc); the server name is canonical here, [`MCP_SERVER_NAME`] — not a retyping.
    // ADR-0094
    // ADR-0133
    // ADR-0209
    pub fn grants_mcp_send(&self) -> bool {
        self.grants
            .iter()
            .any(|g| matches!(g, ToolGrant::Mcp { server, .. } if server == MCP_SERVER_NAME))
    }
}

/// The reason a control channel provision failed (ADR-0086 fail-closed): there **was an intent to attach** the control channel **but it
///   failed** (file write · CSPRNG failure) — spawn aborts fail-closed (no agent running without a control channel is made). ★Distinct
///   from absence★: Ok(None) = legitimate
/// absence of not using a control channel (Noop · shell), Err = failed while trying (fatal). agent carries only strings (no rmcp/io type
///   leaks, ADR-0003).
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
/// ★Three fields, not one — do not merge them★: a single `accepts_mcp_config` once drove the MCP entry, priming variant, grants and **mail
///   availability** together, so a backend 「using the control channel but outside the mail plane」 **opened send authorization the moment
///   it turned control on** (receiving closed) —
///   an asymmetry declared nowhere, because two axes had one field.
/// ★The third field split off the same way★: while `accepts_mcp_config` doubled as 「mail through MCP」 and 「reads the file we wrote」, a
///   backend turning MCP on by another mechanism (codex's `-c mcp_servers.engram=…`) got **a plaintext token file nobody reads** on every
///   spawn by turning that field on for the mail axis (default
///   ACL · the fail-closed write could even cut the spawn).
/// ★The derivation point of `mail_allowed` is still one (ADR-0133 decision 2)★: the daemon makes one value from these two fields and that
///   value produces **together** the enforcement baked into the credentials and the marker on the endpoint; two separate decisions would
///   split education and enforcement.
/// ★The `engram` CLI wiring (env · PATH) is laid down in every combination★ — control verbs are open to everyone (ADR-0132 decision 5);
///   this type splits only **mail and MCP**.
// ADR-0099
// ADR-0126
// ADR-0133
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlChannelNeeds {
    /// Whether this backend **can consume the mcp-config file we make** (ADR-0099).
    ///
    /// true → the MCP endpoint bits (the MCP send grant; `mail_allowed` false). false → none of them. ★Not "can it use
    ///   MCP"★ — a program turning MCP on by another mechanism also has this false (canonical decision: each backend folder).
    pub accepts_mcp_config: bool,
    /// Whether the daemon **will write the mcp-config file** for this spawn (`AgentBackend::writes_mcp_config_file`).
    ///
    /// true → the daemon writes `<data_dir>/mcp-config/<id>-<epoch>.json` (a plaintext Bearer token), and a failed
    /// write is **fail-closed** (the spawn is aborted). false → no file, and `config_path` goes out as `None`.
    /// ★Do not **fold it into one field** with the field above★: that is the axis deriving the mail channel decision (`mail_allowed`);
    ///   this asks only **whether to write a plaintext token to disk**. 「MCP but does not read our file」 (codex) is the normal value of
    ///   that combination (canonical: the third-field item of the type doc).
    // ADR-0209
    pub writes_mcp_config_file: bool,
    /// Whether this spawn **is assigned a mail channel** (`AgentBackend::uses_mail`). false → no send grant goes out, and the daemon
    ///   refuses this credential's mail requests. Mail teaching is not decided here — the priming comes only from the launch materials
    ///   (ADR-0370). ★It still receives the control verbs and the CLI entry★ (last item of the type doc).
    // ADR-0209
    pub uses_mail: bool,
}

/// Control channel provisioning seam (ADR-0086). AgentManager default = `NoopControlChannel`.
pub trait ControlChannel: Send + Sync + 'static {
    /// Issues a token for (AgentId,epoch), creates the mcp-config file (if MCP-capable) and returns the endpoint. Called on the spawn path
    ///   right before spec assembly. Three returns (fail-closed contract, ADR-0086):
    ///   - `Ok(Some(ep))` — control channel issued (the backend injects it).
    ///   - `Ok(None)`     — **legitimate absence** of not using a control channel (Noop · shell-only · unconfigured). Spawn continues.
    ///   - `Err(_)`       — **failed** while trying to use it (CSPRNG/file write error). ★Fatal★ — spawn aborts fail-closed (no agent
    ///   secretly running without a control channel).
    ///
    /// `needs` = what this spawn receives from the control channel ([`ControlChannelNeeds`] — canonical for the fields' meaning and
    ///   split); the manager fills it through backend dispatch.
    // ADR-0126
    // ADR-0133
    fn provision(
        &self,
        id: AgentId,
        epoch: u32,
        needs: ControlChannelNeeds,
    ) -> Result<Option<ControlEndpoint>, ProvisionError>;

    /// Discards the token of (AgentId,epoch) and deletes the mcp-config file. The reaper calls it on any terminal (kill · crash · EOF ·
    ///   normal exit); kill_agent and the spawn failure guard call it pre-emptively too, so duplicate calls for one (id,epoch) come
    ///   (absorbed by remove-if-present). It receives the epoch so a stale
    /// terminal cannot erase a live token newly attached by reactivation (a new incarnation = a different marker; ADR-0007/0084
    ///   epoch-guard — the decision is **match/mismatch**, markers have no order).
    fn revoke(&self, id: AgentId, epoch: u32);
}

/// No-op implementation for paths not using a control channel (headless tests · shell-only): provision always returns Ok(None) (legitimate
///   absence), revoke is a no-op. The AgentManager default — only the daemon injects the real `DaemonControlChannel`.
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
/// ★Source separation (load-bearing)★: the 5 areas come from **two sources** — input/output/control from the physical channel (transport),
///   session/model from the program (backend). The transport once hardcoded even session.resume (true regardless of claude/shell), making
///   the shell backend inaccurate; now it is built only through
/// `Capabilities::compose(TransportCaps, BackendCaps)`, enforcing the source by type (ADR-0030 capability matrix).
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
    /// Reports that the output is a structured stream (NDJSON etc.) needing parsed rendering (RichSlot) instead of terminal rendering
    ///   (ADR-0044). The frontend's `defaultRenderMode` picks the renderer by this one value (true=RichSlot / false=xterm, ADR-0002). Not
    ///   content interpretation (link-unrefined invariant) — just a hint that "this byte stream is not a terminal".
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

/// The PTY output wire format going to the frontend — base64-encoded for JSON compatibility. ※S12: a representation **only for Embedded
///   (Tauri JSON Channel)**; base64 is a JSON Channel constraint, not a core concern — ChannelOutputSink receives OutputFrame (raw) and
///   encodes it into this.
///
/// ★epoch★: like the WS binary frame header ([tag][agentId][epoch][seq]), every output frame carries the session epoch (OutputFrame.epoch
///   as is). **Dropping** frame.epoch in encoding would flow embedded with epoch fixed at 0, mismatching SubscribeAck.current_epoch≥1
///   (resume-fallback), and the ProtocolClient epoch
/// guard (f.epoch !== st.epoch) would wipe out the output (Stage 3 BLOCKER 1). So frame.epoch must always be included, matching the WS path.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PtyEvent {
    pub agent_id: AgentId,
    pub seq: u64,
    pub epoch: u32,
    pub data_b64: String,
}

/// core→sink output payload (S15 B5 payload-generic). Passed **borrowed** — core does not know the wire (ADR-0003).
/// ★ADR-0002 (no assumption about output kind)★: output is not forced into terminal bytes — two branches Bytes/Event let the sink handle
///   each kind (Bytes→tag0 terminal frame, Event→tag1 structured frame, B7).
/// Holds only references to stay Copy (the OutputFrame Copy contract) — no Serialize (an agent domain type, ADR-0003).
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
    /// Whether this agent is **a reader of mail** (= eligibility for the mail recipient list). The value the session received from the
    ///   backend at spawn is **carried as is in this snapshot**.
    ///
    /// ★Carried in the snapshot (load-bearing — do not revert to re-querying)★: a consumer receiving the list and asking the manager back
    ///   per entry would face ① TOCTOU — the same id could split into a different session between the check and the injection, ② the
    ///   reaper collecting the session between the two queries,
    ///   dropping that entry from both live and asleep lists so mail that would have been parked and delivered on reappearance is **lost**
    ///   as "no such recipient", ③ N per-session lock acquisitions again (`AgentManager::roster` doc — the mail-sending critical path).
    ///   One snapshot avoids all three.
    /// ★Not a capability★: `capabilities` is "what it can do", this "what the input is interpreted as" — a separate field, not on the wire
    ///   (a daemon-internal decision axis).
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
    /// ★Dedicated variant★: callers must tell it from name space exhaustion (`Unsupported`) — the user's remedy is opposite (retry with
    ///   another name vs delete agents); string matching breaks when the text changes.
    /// ★Does not apply to existing agents★: the cap looks only at **new registrations**; restore and respawn of a roster already over the
    ///   cap (replacing the same id's incarnation) pass — a backstop taking the existing team hostage would make recovery impossible.
    #[error("roster is full: {current} agents (ceiling {limit}) — refusing to register another")]
    RosterFull { current: usize, limit: usize },
    /// ★The profile store refused to save this change (`ReadOnly`) or could not write it (`Io`) — the in-memory registry is unchanged★.
    ///
    /// Only the registry's entries with a caller (`try_` verbs) emit this — internal changes without a caller apply to memory on any save
    ///   `Err` and set dirty, so they do not become errors.
    /// ★The launch path has partial success★: if clearing the session id fails to save after the roster registration was saved, the
    ///   registration stays (not reverted) and only the launch stops — canonical: the `spawn_agent_watching_link` doc of `manager`.
    /// ★Dedicated variant★: the bus must carry a save refusal with its own code (the caller's remedy differs); split by variant, not
    ///   string, as `RosterFull`.
    // ADR-0291 R17
    #[error("agent store: {0}")]
    Store(crate::profile::StoreError),
    /// The launch-material check blocked this launch — every reason hit. No process and no control channel were made.
    // ADR-0334
    #[error("{0}")]
    LaunchBlocked(crate::launch::LaunchBlocked),
    /// The checked launch input does not match what would be launched — the attachments · command · env changed twice during launch
    /// preparation, the command · env of the copy handed to the spawn differ from the checked roster values, or the source of the
    /// checked plan (a library file) disappeared or changed before the materials were prepared ([`crate::launch::PrepareError::Stale`]).
    /// No process and no control channel were made; retrying with a freshly read copy succeeds. A source race happens after the
    /// incarnation marker was drawn, so one marker is spent (the stored session id stays).
    // ADR-0319
    #[error(
        "launch raced: the agent's launch inputs (presets or their library files, command or env) changed while it was being prepared — try again"
    )]
    LaunchRaced,
    /// The agent's reservation is held by a working-folder change (`agent.setWorkspace`) — this launch started nothing (no process, no
    /// control channel) and records no last failure. Retrying after that change starts it. A launch losing to another launch is not
    /// this (it keeps the double-start answers below).
    // ADR-0386
    #[error(
        "the agent's working folder is being changed (agent.setWorkspace) — nothing was started; start it again once that change is done"
    )]
    WorkingFolderChanging,
    // ★Do not turn duplicate spawn requests back into an error★: "already up / already launching" is **a request with nothing to do**,
    //   answered `Ok(manager::SpawnOutcome::Moot)`. As an error, every consumer carried a list of "this error is not a real failure" and
    //   each fix of that list missed one item and stamped a failure marker on a
    //   live agent (ADR-0172).
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
    /// The seq of the chunk actually sent first. If nothing was sent — for an empty ring, the next seq to issue (the value read under the
    ///   replay lock
    /// · ADR-0231); if there is no tail to continue, `after_seq+1`.
    pub replay_from: u64,
    /// Number of chunks actually sent (can be 0).
    pub replayed: usize,
}

/// The **incarnation facts** a subscription response carries — a value type, not a session field.
///
/// ★Do not put a field of this type in the session★: the incarnation marker's home is `AgentSession::epoch` alone; a second marker
///   diverging would leave unknown which is the incarnation. It is assembled on the spot when the response is built.
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
/// ★The two fields come from one session lookup★: separate lookups could change the incarnation in between and the marker would speak of a
///   different incarnation than the one that replayed.
// ADR-0226
#[derive(Debug, Clone, Copy)]
pub struct SubscribeReply {
    pub outcome: SubscribeOutcome,
    pub incarnation: Incarnation,
}

/// Delivery-boundary measurement output of an input write (ADR-0088 Stage 0).
///
/// ★Why it exists★: `write_input`/`write_stdin` returning only `Ok(())` gives no evidence to tell "not inserted due to a send failure"
///   from "fully inserted but ignored by the model"; for the delivery accuracy harness (ADR-0088) the **completeness signal** (full
///   acceptance vs failure) and this turn's replay-dedup key (`msg_uuid`)
/// must be observable at the write boundary. Returned only on the success path.
///
/// ★★What `Ok` means **depends on which verb it came from**★★ (all three links — PTY · stdio · codex — put input into **a bounded input
///   queue** that a dedicated writer pops and writes):
///   - The `Ok` of `AgentSession::submit_input_observed` (= the mail delivery verb, `AgentManager::submit_stdin_observed`) = ★**it
///   actually went out to the OS**★: that verb issues the receipt only after `AgentTransport::flush_input` confirms the landing — the
///   "send failure" vs "ignored by the model" split of ADR-0088 stands on it.
///   - The `Ok` of `AgentSession::write_input_observed` (keystroke streaming) = **the whole thing was accepted with its order settled**;
///   that path does not wait (waiting would stall the per-connection dispatch consumer on one stuck agent).
///   ★A link without a confirmation means stays at the old level★: for the codex app-server and the test-rig seam `flush_input` is
///   `Unsupported`, so even the delivery verb knows only 「accepted」 — **for those links this record does not assert landing**.
///
/// ★The completeness signal = Ok-vs-Err, not a byte comparison★: partial acceptance is never hidden as `Ok` — the whole thing is accepted
///   (or goes out) or `Err`. So the only evidence is `Ok(WriteOutcome)` **itself**. The real written byte count is not threaded out of the
///   transport, so `bytes_written` is
/// **copied from `bytes_requested` by construction** (short writes undetectable — always equal, a tautology); it exists for the
///   observation record's self-description (logs and forensics), not as a completeness lever. Completeness must look at `Ok`.
///
/// ★Byte units · layer★: both values are **the byte length of the logical message the caller handed to the session boundary**
///   (`bytes.len()` — **before** encoder wrapping, not a char count). With json mode wrapping the wire bytes are larger, but that is not
///   measured here. The daemon's `DeliveryObservation` uses the same
/// "logical message bytes" meaning (there = the envelope string built by `wrap_message`).
#[derive(Debug, Clone, Copy)]
pub struct WriteOutcome {
    pub bytes_requested: usize,
    pub bytes_written: usize,
    /// The message uuid of this user turn (replay-dedup key, generated by session.write_input — LOAD-BEARING). The delivery harness
    ///   correlates the ingress's logical msg_id with it to decide "did claude replay this turn" (= did it actually parse it) (ADR-0088).
    ///   Only exposed here.
    pub msg_uuid: uuid::Uuid,
    /// ★The epoch of the incarnation where the write actually landed (ADR-0088 Stage 1)★ — the executing session's own `self.epoch`,
    ///   filled by construction (as bytes_written: not an independent measurement). ★Needed★: for the delivery observation record to be
    ///   **self-sufficient** ("which incarnation was this message inserted
    /// into"); only the epoch of the session **where the write actually landed**, not the one snapshotted at resolve time, settles the
    ///   mid-flight epoch race (a restart between resolve↔write) from the record alone (the race is design-accepted — ADR-0086 §F5: mail
    ///   targets the logical agent, so landing on a new incarnation is
    /// correct). Success path only; it can diverge from resolve-time, and exists to observe that divergence.
    pub epoch: u32,
}

/// OutputSink send failure signal — triggers removal of that subscriber when drain detects it
#[derive(Debug)]
pub struct SinkError;

/// PTY output delivery abstraction — isolated so headless tests work without a Tauri dependency. ※S12: wire encoding is the
///   implementation's (ChannelOutputSink = base64 PtyEvent / the daemon frame sink = binary frame), so core is transport-agnostic.
pub trait OutputSink: Send + Sync + 'static {
    /// ★Contract = never blocks — if it cannot send, do not wait; `SinkError`★. Two callers hold an order lock (both exceptions to
    ///   ADR-0006 「send without holding a lock」): the session's input lock (`input_order` — `SessionClassified` path) and the codex link's
    ///   emit lane (`Announcer.order` — `announce`);
    /// an implementation waiting here stops that incarnation's input and cancellation (codex: input and the writer's emits) together.
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
    /// The backend observed a usage limit (missing-field meaning = [`crate::usage::UsageObservation`]). Default no-op.
    ///
    /// ★Same contract as [`Self::turn_ended`] (non-blocking · non-reentrant)★ — the caller is the output pump, so stopping here stops that
    ///   incarnation's output.
    fn usage_observed(&self, _obs: crate::usage::UsageObservation) {}
    /// This incarnation just ended a turn (ADR-0113 — the push exit of turn observation). Default no-op.
    ///
    /// ★Contract = non-blocking · non-reentrant (ADR-0006 callback discipline — never violate)★: at most a non-blocking notification
    ///   (channel send); IO, injection or reentrant calls stop the thread that called this callback until done.
    /// ★That thread is the output pump **today**, not by structural guarantee (load-bearing)★: `emit` has two callers (the output pump ·
    ///   the injection thread emitting the input echo — turn.rs header) and this hook runs on whichever emitted the end signal. It is
    ///   pump-only now **because the input echo is classified as a
    ///   progress signal** (backend classifier); a new non-pump emit path emitting end signals moves this contract's weight there — it
    ///   must stay non-blocking so delivery work is not loaded onto the sending thread.
    /// ★It goes out on every end signal regardless of transition★: a consumer wanting "only when in a turn just before" decides on its own
    ///   side; narrowing here would drop the notification for a turn ending right away without assistant events, and a missing
    ///   notification makes the consumer wait forever.
    // ADR-0113
    fn turn_ended(&self, _id: AgentId, _epoch: u32) {}
    /// This incarnation's user waiting list just became empty — the doorbell keeping parked mail from being tied up until the next turn
    ///   end when only the list empties. Default no-op.
    ///
    /// ★Same contract as [`Self::turn_ended`] (non-blocking · non-reentrant)★. ★It rings **after** 「empty」 is written to the waiting-list
    ///   table★ — the receiver asks about busy again at once. It goes out every time it empties (except emptying by the exit synthesis —
    ///   `OutputCore::finish`); surplus is absorbed by the consumer.
    /// ★A separate verb, not a widened `turn_ended`★: 「the turn ended」 meaning even emptying without a turn would mislead consumers
    ///   relying on the name.
    // ADR-0231
    fn inputs_drained(&self, _id: AgentId, _epoch: u32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── launch time constants (ADR-0360) ──
    #[test]
    fn launch_time_constants_hold_their_values() {
        assert_eq!(LAUNCH_REQUIRED_FOLDER_WAIT_SECS, 60);
        assert_eq!(LAUNCH_ADVISORY_FOLDER_WAIT_SECS, 2);
        assert_eq!(LAUNCH_REPLY_BOUND_SECS, 85);
        assert_eq!(LAUNCH_CALLER_WAIT_SECS, 88);
    }

    /// ADR-0369: every launch wait is the previous step + the one margin, and every nested pair holds
    /// outer ≥ inner + margin — so moving the base or the margin moves every wait and keeps the order.
    #[test]
    fn launch_waits_are_derived_from_one_base_and_one_margin() {
        assert_eq!(
            LAUNCH_MARGIN_SECS,
            DAEMON_SWEEP_SECS + DAEMON_CALLER_MARGIN_SECS
        );
        assert_eq!(
            LAUNCH_CALLER_WAIT_SECS,
            LAUNCH_REPLY_BOUND_SECS + LAUNCH_MARGIN_SECS
        );
        assert_eq!(
            LAUNCH_RELAY_SEAT_SECS,
            LAUNCH_CALLER_WAIT_SECS + LAUNCH_MARGIN_SECS
        );
        assert_eq!(
            LAUNCH_RELAY_CALLER_WAIT_SECS,
            LAUNCH_RELAY_SEAT_SECS + LAUNCH_MARGIN_SECS
        );
        assert!(
            LAUNCH_MARGIN_SECS > 0,
            "a zero margin makes the order prose"
        );
        for (inner, outer) in [
            (LAUNCH_REPLY_BOUND_SECS, LAUNCH_CALLER_WAIT_SECS),
            (LAUNCH_CALLER_WAIT_SECS, LAUNCH_RELAY_SEAT_SECS),
            (LAUNCH_RELAY_SEAT_SECS, LAUNCH_RELAY_CALLER_WAIT_SECS),
        ] {
            assert!(outer >= inner + LAUNCH_MARGIN_SECS, "({inner}, {outer})");
        }
        assert_eq!(
            (
                LAUNCH_MARGIN_SECS,
                LAUNCH_RELAY_SEAT_SECS,
                LAUNCH_RELAY_CALLER_WAIT_SECS
            ),
            (3, 91, 94)
        );
    }

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
