//! `agent.*` commands — live next to the code that works with **declaration and body as one set** (`AgentManager`) (ADR-0155).
//!
//! ★This table is the real entry of the control verbs — there are **two** surfaces★: the CLI's `/control/agent` route (it finds `{verb}`
//!   as `agent.<verb>`) and the envelope the client issues on the command bus (step 1 of the 3-step delivery). Both pass through the daemon's
//!   one common entry (daemon `control/commands.rs`). That is, this file's return shapes, error codes and notification counts are
//!   exactly the response of `engram agent …` and the answer the LLM receives on the bus.
//! ★Hosted by the daemon, declared by the core★ — the only place where these two split (ADR-0029 + TRD §2-3).
//!
//! ★`usage.*` (the two usage-limit verbs) are also declared here — their bodies are not here★: the producer of the vocabulary (the backend word `AgentBackend` · row
//!   types) is this crate, so the declaration lives here (ADR-0155 decision 1), and the real thing is plugged in by the daemon's usage service through the port
//!   [`UsageCommandHost`] (TRD S21 usage-limit-slot §3 #18).
//!
//! Entry points: [`make_table`](fn.make_table.html) (assembly) · [`AgentCommandHost`](trait.AgentCommandHost.html) (injection seam) ·
//! [`UsageCommandHost`](trait.UsageCommandHost.html) (usage port).

use std::path::PathBuf;
use std::sync::Arc;

use engram_dashboard_base::path::normalize_spelling;
use engram_dashboard_command::{
    blocking_handler, declare_commands, CommandError, CommandTable, ErrorCode, RetryMode,
};

use crate::manager::{AgentManager, RenameOutcome};
use crate::persistence::AGENTS_FILE;
use crate::preset::PresetId;
use crate::queued_input::{ListedState, QueuedListing};
// An alias telling the core enum apart from the declaration vocabulary of the same name below.
use crate::profile::AgentOutputFormat as CoreAgentOutputFormat;
use crate::profile::{AgentCommand, AgentProfile, Refusal, SpawnMode, StoreError, StoreStatus};
use crate::types::{
    AgentId, AgentStatus, CancelError, CancelOutcome, PtyError, AGENT_STATE_LIVE,
    AGENT_STATE_SLEEPING, RENAME_OUTCOME_RENAMED, RENAME_OUTCOME_UNCHANGED,
};
use crate::usage::UsageVendorKey;

// ★Success responses are flat (user decision 2026-08-13)★: each command declares its return, so there is no reason to wrap one more
//   layer `{"agent":{…}}`.
declare_commands! {
    // v8 (2026-10-26): one name was added — `agent.resolve` (ADR-0307 · TRD S28 §6).
    // v7 (2026-10-10): the answer of `agent.list` gained the field `store` — the state of the two registry stores (agents · presets) (ADR-0291
    //   R17 · D6). The names are unchanged and the shape of the answer changed.
    // v6 (2026-09-28·29): three names were added — `agent.interrupt` (ADR-0237) · `usage.get`·`usage.refresh` (TRD S21
    //   usage-limit-slot §1-6). Two branches each raised it to 6 separately, and v6 was never shipped in any release (tag), so they were merged into one generation.
    // v5 (2026-09-26): two names were added — `agent.listQueuedInputs`·`agent.cancelQueuedInput` (ADR-0231).
    // v4 (2026-09-22): the `backend` vocabulary of `agent.new` went from **`Claude` alone → the two `Claude`·`Codex`**
    //   (ADR-0219). Neither names nor fields grew, but **the set of words that field accepts** changed, and that is the declaration
    //   callers see — the generated output carries that difference as is (`"backend":{"enum":[…]}` of
    //   `bindings/commands.schema.json`). Without raising it, a build that creates codex and a build that refuses it would report the same
    //   generation. ★The generation is not a number raised **only when names grow** — it is raised when the declaration changes.★
    // v3 (2026-09-08): the `backend` of `agent.new` went from **optional → required**. It is a breaking change removing the silent claude default,
    //   so the generation is raised (reason = that field's doc). This number is diagnostic, and the receiving side does not use it for refusal
    //   (the RegisterCommands branch of `connection_core`).
    catalog_version: 8;

    /// One row of the roster.
    struct AgentRow {
        id: String,
        name: String,
        state: String,
        cwd: String,
        parent: Option<String>,
    }

    /// One line of the `agent.resolve` answer — the agent one requested word pointed at.
    struct ResolvedAgentRow {
        /// The word carried in the request, as is.
        target: String,
        id: String,
        /// The current roster name.
        name: String,
        /// The same decision as `state` being `live` in `agent.list`.
        live: bool,
    }

    // ── Roster store state (`store` of `agent.list` — ADR-0291 R17 · D6) ─────────────────────────────────────
    // ★A macro enum is only a string enumeration, so the store state (the tag enum `profile::StoreStatus` carrying a reason) is spread into a word + optional
    //   fields★ — the same shape as the usage rows.

    /// The state of one store file — `Writable` (writable) | `ReadOnly` (the daemon did not accept that file when it started — the roster is
    /// empty, and for this whole run requests changing the roster are `CONFLICT` (`NOT_FOUND` if there is no entry to point at). It clears only by fixing
    /// or replacing the file and then restarting the daemon) |
    /// `Refusing` (the last save was refused — the next change decides again and it clears if accepted. If the file was fixed in the meantime,
    /// this value is stale).
    enum StoreStateWord {
        Writable,
        ReadOnly,
        Refusing,
    }

    /// The reason the store does not accept — `Newer` (a file written by a newer edition than this daemon) | `Unreadable` (the file could not be read — lock ·
    /// permission etc.).
    enum StoreRefusalWord {
        Newer,
        Unreadable,
    }

    /// The state row of one store file.
    struct StoreStatusRow {
        state: StoreStateWord,
        /// The reason for `ReadOnly` · `Refusing` — `null` if `Writable`.
        reason: Option<StoreRefusalWord>,
        /// For `Newer`, the edition number that file wrote — otherwise `null`.
        file_version: Option<u64>,
    }

    /// The two roster stores — `agents` (agents.json · agent roster) · `presets` (presets.json · path bookmarks).
    struct RosterStoreRow {
        agents: StoreStatusRow,
        presets: StoreStatusRow,
    }

    /// The backend to run the new agent on — ★the values accepted today are the two `Claude` and `Codex`★ (codex came in
    /// on 2026-09-22).
    ///
    /// The core has other backends too (gemini · shell of `agent::backend`), but **it has not been confirmed that the profile creation path
    /// runs with them** — so the field is left open but the values are narrowed to the confirmed two (so the contract's **shape** does not
    /// change later).
    /// ★The worth of adding a variant is decided by **whether its pair already exists**★: the run command (`AgentCommand`)
    /// **is written as is into `agents.json`** with `#[serde(tag = "kind")]`, and an old build meeting an unknown kind treats that
    /// file **as one lump** as damaged and **comes up with an empty roster** — the first save copies that file aside to `.corrupt` and overwrites it
    /// (persistence `FileProfileStore::load` · builds older than that move it to `.corrupt-<ms>` at load). So
    /// adding here a backend **not yet** in `AgentCommand` is a migration that breaks disk compatibility, and adding a backend whose pair **already
    /// exists** is not a migration.
    /// ★`Codex` was the latter★ — `AgentCommand::Codex { extra_args, output_format }` existed before that, and
    /// the human menu (`agentlist.createCodex`·`createCodexJson`) was already writing **exactly that shape** into `agents.json`.
    /// What opened this door was neither a new kind, a new field nor a schema bump — ★at one point this place said 「opening codex is a migration that breaks disk compatibility」,
    /// but the human door was already writing the same shape, so
    /// it was not true even then.★
    /// ★The real case of the former is `Gemini`★ — `agent::backend::gemini` stands as a stub, but `AgentCommand` has
    /// no pair for it. An edit adding that word here **puts a new kind onto disk**, so the migration above must be designed
    /// along with it. Do not read the two cases with the same weight.
    /// ★**Why** it is open and **how the next word is opened** are carried by [`LLM_BACKEND_POLICY`]★ — the other
    /// creation doors look at that table too (that table's header writes the doors out separately).
    enum AgentBackend {
        Claude,
        Codex,
    }

    /// The output format (= render mode) of the new agent — fixed at creation and immutable afterwards (ADR-0044/0078).
    ///
    /// `Terminal` = PTY interactive (xterm render) · `StreamJson` = headless NDJSON stream.
    /// The vocabulary must be the same as the core `profile::AgentOutputFormat` **in both variants and wire notation** — ★since the rename (ADR-0194)
    /// this declaration vocabulary and the core enum **use the same name**★. When pointing at it, write it with the path (the bare name
    /// points at this declaration within this file — that is why the import above puts an alias on the core side). If they diverge, the frontend allowlist
    /// (`coerceOutputFormat`) and this entry read the same word differently. The place where the compiler, not prose, keeps that match
    /// is `tests::the_declared_vocabularies_are_pinned_to_the_core_ones`.
    enum AgentOutputFormat {
        Terminal,
        StreamJson,
    }

    /// The two fields of a pending-cancel entry — the registry's reduced state as is (both are needed to re-reduce later events on top of the snapshot).
    struct QueuedInputCancel {
        /// The response to our cancel request — `none` (none yet) | `not_removed` (it answered it did not remove it, or the request failed).
        answer: String,
        /// The vendor close of that id has already been seen.
        vendor_closed: bool,
    }

    /// One line of the waiting list.
    struct QueuedInputRow {
        /// The input id — used as is as the `input_id` of `agent.cancelQueuedInput`.
        id: String,
        text: String,
        /// `queued` | `sent` (the link handed it over and waits for the echo) | `unconfirmed` (the link handed it over but it is unknown whether it was accepted —
        /// mail waits for that outcome · ★no backend emits it today★ — ADR-0235) | `cancelling` (pending cancel — stays on the list until
        /// the outcome happens).
        state: String,
        /// Carried only on `cancelling` rows — otherwise `null`.
        cancel: Option<QueuedInputCancel>,
    }

    // ── Usage limits (`usage.*` — TRD S21 usage-limit-slot §1-6) ──────────────────────────────────────
    // ★A macro enum is only a string enumeration, so the wire state (a tag enum carrying fields) is spread into a word + optional fields★ — the words map 1:1 to the six wire
    //   states and the fields are carried only in that state (`UsageVendorRow`).

    /// The kind of usage window — `FiveHour` (5 hours) · `Weekly` (weekly) · `ModelWeekly` (per-model weekly — `label` is its name).
    enum UsageWindowWord {
        FiveHour,
        Weekly,
        ModelWeekly,
    }

    /// One usage limit window.
    struct UsageWindowRow {
        window: UsageWindowWord,
        /// Carried only for `ModelWeekly` (the display name the vendor gave) — otherwise `null`.
        label: Option<String>,
        /// Percentage 0–100 of the amount **used** (not the amount left). `null` = unknown — not 0.
        used_pct: Option<f64>,
        /// The amount left — the same `floor(clamp(100 − used_pct, 0, 100))` as the screen. `null` if `used_pct` is `null`.
        left_pct: Option<u64>,
        /// Reset time, epoch seconds — the only absolute time of this row.
        resets_at: Option<u64>,
        /// Seconds elapsed since this value was observed (as of the moment of answering).
        age_secs: u64,
        /// The daemon confirmed the reset has passed — the value is that of the previous window.
        expired: bool,
    }

    /// Whether the answer came from the query this request waited for — a word moving daemon `usage_service::UsageServed` as is.
    enum UsageServedWord {
        /// A value fetched by the successful query this request waited for (or a query after it).
        Fresh,
        /// No new value was received during this request (no query · failure · wait cap exceeded) — it is the value held.
        Cached,
    }

    /// The state of the vendor query — 1:1 with the six wire `UsageVendorState` (the fields are spread by [`UsageVendorRow`]).
    enum UsageStateWord {
        Ready,
        NotInstalled,
        NeedsLogin,
        Unavailable,
        Failed,
        Rejected,
    }

    // errors lists only codes **specific to this command** — argument rejection (INVALID_ARGUMENT) and internal failure (INTERNAL)
    //   are emitted by the table, so `CommandSpec::advertised_errors` adds them automatically.

    /// The whole roster — name · liveness · working folder · parent — and the state of the two roster stores (`store`).
    #[effect(Read)]
    #[since(1)]
    "agent.list" => args AgentListArgs {}
                 -> ok   AgentListOk { agents: Vec<AgentRow>, store: RosterStoreRow }
                 errors [];

    // ADR-0307
    // ★The head `targets[i]: ` of the failure text is a contract★ — the shell (TRD S28 §5-1) traces a failure back to its own argument name by this head.
    //   The shell does not depend on this crate, so compilation does not stop if it changes. The place building the head = `target_failure` · the pinning test =
    //   `tests::the_resolve_failure_prefix_is_a_stable_contract`.
    /// Resolves 1–8 words pointing at agents (name or id) at once — `agents` is in the same order as the request. The resolving rules are the same as other verbs
    /// taking `target` (exact id match first · then exact name match — case-sensitive · asleep agents included).
    /// `live` = `state` of `agent.list` is `live`. If even one cannot be resolved, it returns no line and fails. The text of a failure caused by one word
    /// always starts with `targets[i]: ` (i = that word's position counting from 0) — INVALID_ARGUMENT = an empty word (only whitespace) ·
    /// NOT_FOUND = no such agent (the trailing `candidates` of the text = names worth choosing, in name order · `none` if the roster is empty) ·
    /// CONFLICT = that name exists two or more times (the trailing `candidates` of the text = those ids — call again by id). If the count is outside 1–8,
    /// it is INVALID_ARGUMENT without that head.
    #[effect(Read)]
    #[since(8)]
    "agent.resolve" => args AgentResolveArgs {
        targets: Vec<String>,
    } -> ok AgentResolveOk {
        agents: Vec<ResolvedAgentRow>,
    } errors [NOT_FOUND, CONFLICT];

    /// Launches an agent (including waking an asleep one).
    #[effect(Write)]
    #[since(1)]
    "agent.spawn" => args AgentSpawnArgs {
        /// The target to wake (name or id). Mutually exclusive with cwd.
        target: Option<String>,
        /// The working folder to create and launch anew. Mutually exclusive with target.
        cwd: Option<String>,
        /// Used only when creating anew — does not apply to waking.
        name: Option<String>,
    } -> ok AgentSpawnOk {
        agent_id: String,
        name: String,
        state: String,
        created: bool,
    } errors [NOT_FOUND, CONFLICT];

    /// Registers a new agent in the roster (does not launch it — it stays asleep).
    #[effect(Write)]
    #[since(1)]
    "agent.new" => args AgentNewArgs {
        /// Working folder. ★Mutually exclusive with `preset`, and **exactly one of the two** must be present★.
        cwd: Option<String>,
        /// The id of a registered path bookmark (preset). Mutually exclusive with cwd.
        preset: Option<String>,
        name: Option<String>,
        /// Unspecified = StreamJson.
        output_format: Option<AgentOutputFormat>,
        /// Which backend to run — ★**required. Unspecified is rejected**★ (2026-09-08).
        ///
        /// Unspecified used to mean `Claude`. Since there came to be two backends, that silent default folds **not choosing**
        /// and **choosing claude** into the same value — it is the shape already removed from the spawn packet for the same reason
        /// (the `AgentBackendKind` doc of `protocol` · the daemon's `MISSING_BACKEND`), and leaving only this door
        /// would split that decision per entry.
        backend: AgentBackend,
    } -> ok AgentNewOk {
        agent_id: String,
        name: String,
        state: String,
    } errors [NOT_FOUND, CONFLICT];

    /// Changes the display name.
    #[effect(Write)]
    #[since(1)]
    "agent.rename" => args AgentRenameArgs {
        target: String,
        name: String,
    } -> ok AgentRenameOk {
        agent_id: String,
        /// The settled name — may differ from the requested name (roster uniqueness suffix).
        name: String,
        outcome: String,
    } errors [NOT_FOUND, CONFLICT];

    /// Changes the parent in the tree.
    #[effect(Write)]
    #[since(1)]
    "agent.move" => args AgentMoveArgs {
        target: String,
        /// The new parent (name or id). ★`null` = detach to top level · **an absent field is rejected**★ — so that a single typo field
        /// does not silently run as a hierarchy release, absence and `null` are told apart (the outer Option = whether it was carried).
        parent: Option<Option<String>>,
    } -> ok AgentMoveOk {
        agent_id: String,
        name: String,
        parent: Option<String>,
    } errors [NOT_FOUND, CONFLICT];

    /// The list of inputs a live agent received mid-turn without confirmation yet that they were accepted (non-terminal only). `state` = `queued` (before confirmation —
    /// we hold it or handed it to the agent and wait for the outcome. codex hands it over by steer when the running turn's tool ends, or to the next turn if the turn
    /// ends without a tool · claude hands it to the CLI queue right away) | `sent` (codex handed it to the agent and waits for the echo saying it was received
    /// — cancelling does not remove it) | `unconfirmed` (handed over but unknown whether it was accepted —
    /// it stays on the list and mail waits for that outcome, and cancelling removes it right away · ★no backend emits it today★ — that stage of codex
    /// was removed, ADR-0235) | `cancelling` (a cancel was requested — stays until the agent produces an outcome).
    /// `cancel.answer` = `none` (no answer yet) | `not_removed` (the agent answered it could not remove it, or the request failed) ·
    /// `cancel.vendor_closed` = the agent already closed that entry.
    /// `stopped_after_error` = the previous turn ended with a real error and no turn has ended successfully since — meanwhile mail
    /// does not go in automatically (kernel `last_end_failed`). User text codex holds may go out again the moment new user text comes in
    /// (ADR-0235 decision 4). An asleep agent holds no list (NOT_FOUND).
    #[effect(Read)]
    #[since(5)]
    "agent.listQueuedInputs" => args AgentListQueuedInputsArgs {
        target: String,
    } -> ok AgentListQueuedInputsOk {
        inputs: Vec<QueuedInputRow>,
        /// The seq of the last list event the rows reduced (`null` = none yet). Compared only within `epoch` — for reattach comparison.
        as_of_seq: Option<u64>,
        /// Incarnation marker — compared only for match/mismatch.
        epoch: u32,
        stopped_after_error: bool,
    } errors [NOT_FOUND, CONFLICT];

    /// Cancels one entry of the waiting list (the same command as the ✕ on screen) — `outcome` = `cancelled` (withdrawn right away) |
    /// `requested` (a cancel was requested — the list shows the outcome). An id not on the list is NOT_FOUND. CONFLICT = the name is
    /// ambiguous (two or more with the same name — point at it by id) or a connected viewer holds this agent's input (try again after it lets go).
    #[effect(Write)]
    #[since(5)]
    "agent.cancelQueuedInput" => args AgentCancelQueuedInputArgs {
        target: String,
        /// ★Required★ — no default like 「the most recent if absent」 (TRD §5-6).
        input_id: String,
    } -> ok AgentCancelQueuedInputOk {
        outcome: String,
    } errors [NOT_FOUND, CONFLICT];

    /// Interrupts the running turn (≠ kill — the process lives). `outcome` = `requested` (the interrupt was sent — whether the turn actually stopped is announced by the turn-end event).
    /// NOT_FOUND = no such agent or it is asleep. CONFLICT = the name is ambiguous · there is no turn to interrupt · this agent
    /// does not support interrupt · a connected viewer holds this agent's input.
    #[effect(Write)]
    #[since(6)]
    "agent.interrupt" => args AgentInterruptArgs {
        target: String,
    } -> ok AgentInterruptOk {
        outcome: String,
    } errors [NOT_FOUND, CONFLICT];

    /// The usage limits of one backend (5-hour · weekly · per-model weekly) — the values the daemon holds. If the next automatic query time has passed,
    /// it launches a query here (or attaches to one in progress) and waits up to 5 seconds for it to end — past that it answers with the held value with
    /// `in_flight` = true and the query continues. `served` = `Fresh` (a value fetched by the query this request waited for) |
    /// `Cached`. `state` = `Ready` | `NotInstalled` | `NeedsLogin` | `Unavailable` (this account has no limit info — no windows
    /// are carried) | `Failed` (`next_attempt_in_secs` = until the next automatic query) | `Rejected` (upstream refused —
    /// until `retry_in_secs` passes, even `usage.refresh` does not query). Even on failure, the windows are the last known values.
    /// The cause of an abnormal state = `detail_kind` (category word) · `detail_code` (the number upstream gave) · `upstream` (upstream
    /// original text — secrets masked · 200 chars). A window's `left_pct` = the amount left (`used_pct` is the amount used), and `null` means unknown (not
    /// 0). Time fields are relative seconds as of the moment of answering, and only `resets_at` is epoch seconds. A backend whose field this daemon does not hold is
    /// NOT_FOUND.
    #[effect(Read)]
    #[since(6)]
    "usage.get" => args UsageGetArgs {
        /// Whose backend's limits — the same word as `agent.new`.
        backend: AgentBackend,
    } -> ok UsageVendorRow {
        backend: AgentBackend,
        /// The account within the same backend — always `"default"` for now (the daemon's default login).
        account_key: String,
        plan: Option<String>,
        /// Only the windows the vendor gave — in the order 5-hour · weekly · per-model weekly. Windows with unknown utilization (`used_pct: null`) are carried too.
        windows: Vec<UsageWindowRow>,
        /// The query is still in progress — the values of this row are from before that query.
        in_flight: bool,
        served: UsageServedWord,
        state: UsageStateWord,
        /// Carried only for `Failed` — seconds left until the next automatic query (0 if already passed).
        next_attempt_in_secs: Option<u64>,
        /// Carried only for `Rejected` — until these seconds pass, even a forced refresh sends no query.
        retry_in_secs: Option<u64>,
        /// The category word of an abnormal state (e.g. `timeout`·`rpc_error`) — an untranslated string. `null` = the cause is unknown.
        detail_kind: Option<String>,
        /// The number upstream gave (Codex JSON-RPC `error.code`).
        detail_code: Option<i64>,
        /// The upstream original text (secrets masked · whitespace-class controls become spaces · 200 chars) — it goes only to the UI and this row, not to logs.
        ///
        /// ★The `Debug` of this type prints the original as is — do not put it into logs with `{:?}` or a tracing `?` field, whether the whole row or this
        ///   field★. The declaration macro always derives `Debug` and there is no door to turn it off per type, so it cannot be hand-written like the masked `Debug` (agent
        ///   `usage::UpstreamText` · protocol `UsageStateDetail`). The way to the log is the one result log after the deadline,
        ///   and there the key of this name is stripped from the JSON copy (daemon `command_delivery::log_late_local`).
        upstream: Option<String>,
    } errors [NOT_FOUND];

    /// Forces a new query of the usage limits (the same command as ⟳ on screen) — launches a query (or attaches to one in progress) and waits up to 5 seconds for
    /// it to end, answering with the same row as `usage.get`. There is no call interval limit — it queries again even if the previous query just ended,
    /// and if frequent, upstream may refuse (that refusal may show only as `Failed`, not `Rejected`). Answering with the held value (`Cached`)
    /// without querying = the upstream refusal period remains (even if a value came in during the refusal and `state` is
    /// `Ready`, the period remains — `retry_in_secs` is not carried then). Even if the query fails or does not end within 5 seconds, it is
    /// `Cached` (the query did go out then). The meaning of the fields is the same as `usage.get`. A backend whose field this daemon does not hold is NOT_FOUND.
    #[effect(Write)]
    #[since(6)]
    "usage.refresh" => args UsageRefreshArgs {
        /// Whose backend's limits — the same word as `agent.new`.
        backend: AgentBackend,
    } -> ok UsageRefreshOk {
        // ★The fields must not differ by a single character from `UsageVendorRow`★ — the macro builds a new return struct per command,
        //   so two commands cannot share one type. The pairing is carried by `From<UsageVendorRow>` (destructuring — compilation
        //   stops if the fields diverge) and the test comparing the bytes of the two return schemas. The meaning of the fields is in the `UsageVendorRow` doc.
        backend: AgentBackend,
        account_key: String,
        plan: Option<String>,
        windows: Vec<UsageWindowRow>,
        in_flight: bool,
        served: UsageServedWord,
        state: UsageStateWord,
        next_attempt_in_secs: Option<u64>,
        retry_in_secs: Option<u64>,
        detail_kind: Option<String>,
        detail_code: Option<i64>,
        /// Upstream original text — ★`Debug` prints the original; do not put it into logs with `{:?}`★ (the doc of `UsageVendorRow.upstream`).
        upstream: Option<String>,
    } errors [NOT_FOUND];
}

/// Commands that **move the input** of a live agent — the common entry looks at the input lease (`check_input`) first.
///
/// ★Kept here rather than adding a field to the declaration macro★ — putting a marker on `CommandSpec` would make the command crate know the domain of
///   input leases. Every name of this list must be a `Write` declaration of this block (a test measures it).
/// The query (`agent.listQueuedInputs`) does not look at the lease — it is a read.
/// Interrupt (`agent.interrupt`) is included too — the same as WS `Interrupt` looking at the lease.
// ADR-0231
// ADR-0237
pub const INPUT_AFFECTING: &[&str] = &["agent.cancelQueuedInput", "agent.interrupt"];

/// One roster row — only what this table sees from the manager.
///
/// `live` = if `Some(status)`, a session is attached (the liveness decision is [`AgentStatus::is_live`] alone, ADR-0116).
pub struct AgentRosterRow {
    pub id: AgentId,
    pub canonical_name: String,
    pub cwd: String,
    pub parent: Option<AgentId>,
    pub live: Option<AgentStatus>,
}

/// The current state of the two roster stores — `agent.list` spreads it into [`RosterStoreRow`].
// ADR-0291 R17 (D6)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RosterStoreStatus {
    pub agents: StoreStatus,
    pub presets: StoreStatus,
}

/// The launch result — the three fields this table sees.
pub struct StartedAgent {
    pub id: AgentId,
    pub name: String,
    pub status: AgentStatus,
}

/// **Everything** the `agent.*` handlers have the manager do.
///
/// ★Why a trait★: taking `AgentManager` as is would drag in a real PTY process just to assert one handler —
///   a direct violation of ADR-0012 「cut external dependencies with a seam」, the form rule T-1 forbids.
///   With this seam, the whole table is verified without a process through a fake manager.
/// ★Implementations only translate★: the verb rules (target resolution · mode derivation · outcome translation) are held **by the handlers**.
///   If rules went into implementations, the real and the fake would run different rules and the harness would guard nothing.
// ADR-0155
// ADR-0012
pub trait AgentCommandHost: Send + Sync {
    fn roster(&self) -> Vec<AgentRosterRow>;
    /// The current state of the two roster stores (agents · presets).
    // ADR-0291 R17 (D6)
    fn store_status(&self) -> RosterStoreStatus;
    fn agent_snapshot(&self, id: AgentId) -> Option<AgentProfile>;
    /// ★The three that change the roster (`create_agent` · `rename_agent` · `reparent_agent`) are changes with a caller★ — if the store
    ///   does not accept the change, the roster is left as is and it is [`PtyError::Store`] (the manager's `try_` versions · ADR-0291 R17).
    fn create_agent(&self, profile: AgentProfile) -> Result<AgentProfile, PtyError>;
    fn activate_profile(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<StartedAgent, PtyError>;
    fn rename_agent(
        &self,
        id: AgentId,
        display_name: Option<String>,
    ) -> Result<RenameOutcome, PtyError>;
    /// ★`Ok(false)` does not tell the reason★ — the manager gives only one bool (`ProfileRegistry::try_reparent`).
    /// Absence and a structural conflict are told apart by the handler **by re-querying the roster** (`verb_move`).
    fn reparent_agent(&self, child: AgentId, parent: Option<AgentId>) -> Result<bool, PtyError>;
    /// The working folder of one registered path bookmark (preset, ADR-0061).
    ///
    /// ★Pointing is **exact id match** only★ — it cannot be found by name (preset names are not unique). A missing id and
    /// a string not in id format are both `None`: because the caller has the same thing to do (pick an id from the list again).
    fn preset_cwd(&self, id: &str) -> Option<String>;
    /// The waiting list of the live incarnation. `None` = no live session.
    // ADR-0231
    fn list_queued_inputs(&self, id: AgentId) -> Option<QueuedListing>;
    /// Cancels one queued input of the live incarnation. `CancelError::NotFound` if there is no live session.
    // ADR-0231
    fn cancel_queued_input(
        &self,
        id: AgentId,
        input_id: &str,
    ) -> Result<CancelOutcome, CancelError>;
    /// Interrupts the running turn of the live incarnation. `PtyError::NotFound` if there is no live session · `PtyError::Unsupported` if there is no turn to interrupt or the link
    /// cannot interrupt.
    // ADR-0237
    fn interrupt_agent(&self, id: AgentId) -> Result<(), PtyError>;
}

/// The exit (port) telling attached clients the roster changed.
///
/// ★The symptom when this is missing★: even after creating or renaming an agent, the dashboard tree **shows the old roster until an unrelated event
///   comes** (a silent stale). A verb that changed the **composition** of the roster must call this port —
///   that decision differs per verb (waking does not change the composition), and the handler carries it.
/// ★Why a port and not a dependency of the table★: the real notification is an all-connection fan-out, owned by the daemon. The consumer here holds only a narrow
///   trait, and the assembly provides the real adapter (the same shape as daemon `control::RosterBroadcast`).
/// Must be non-blocking — the fan-out only does try_send to per-connection queues.
// ADR-0155
pub trait RosterChanged: Send + Sync {
    fn roster_changed(&self);
}

/// **Everything** the `usage.*` handlers have the daemon's usage service do (a port) — the real thing = daemon `UsageService`.
///
/// ★Synchronous — called on a blocking pool thread (TRD S21 usage-limit-slot §3 #17)★: the daemon's common entry drives only handlers that finish at the first poll
///   (`call_daemon_command`), and an async handler that waits becomes `OUTCOME_UNKNOWN`. So the two
///   methods wait for the query to end **on the calling thread** (cap 5 seconds). The real thing waits on a std channel — this crate stays
///   unaware of an async runtime.
/// ★The field is found by [`usage_vendor_of`], not by the incoming spelling★ — the bus word (`Claude`) and the wire word (`claude`) must hit one
///   field (§3 #43).
/// `None` = this daemon does not hold the field of that backend — the handler answers `NOT_FOUND`.
// ADR-0155
// ADR-0012
pub trait UsageCommandHost: Send + Sync {
    /// The `Get` of the request table — before the next automatic query time, it answers with the current value without querying.
    fn get(&self, backend: &AgentBackend) -> Option<UsageVendorRow>;
    /// The `Refresh` of the request table — within the refusal period, it answers with the current value without querying.
    fn refresh(&self, backend: &AgentBackend) -> Option<UsageVendorRow>;
}

/// Bus word → the vendor key of the usage field. `None` = there is no prober for that word.
///
/// ★The same path as the wire entry★ — both entries find the prober with [`usage_probe_for`](crate::backend::usage_probe_for) and
///   take that prober's key as the field (the wire-side pair in daemon `connection_core`). Making the key from the incoming spelling would have the two entries
///   hit different fields.
// ADR-0004
pub fn usage_vendor_of(backend: &AgentBackend) -> Option<UsageVendorKey> {
    crate::backend::usage_probe_for(&backend_word(backend)).map(|probe| probe.key())
}

/// The return of `usage.refresh` — its fields are the same as `UsageVendorRow` (the declaration's comment). ★Moved by destructuring (without `..`)★ — if either
/// side gains a field, compilation stops here.
impl From<UsageVendorRow> for UsageRefreshOk {
    fn from(row: UsageVendorRow) -> Self {
        let UsageVendorRow {
            backend,
            account_key,
            plan,
            windows,
            in_flight,
            served,
            state,
            next_attempt_in_secs,
            retry_in_secs,
            detail_kind,
            detail_code,
            upstream,
        } = row;
        Self {
            backend,
            account_key,
            plan,
            windows,
            in_flight,
            served,
            state,
            next_attempt_in_secs,
            retry_in_secs,
            detail_kind,
            detail_code,
            upstream,
        }
    }
}

impl AgentCommandHost for AgentManager {
    fn roster(&self) -> Vec<AgentRosterRow> {
        AgentManager::roster(self)
            .into_iter()
            .map(|entry| AgentRosterRow {
                id: entry.id,
                canonical_name: entry.canonical_name,
                cwd: entry.cwd,
                parent: entry.parent,
                live: entry.live.map(|info| info.status),
            })
            .collect()
    }

    fn store_status(&self) -> RosterStoreStatus {
        RosterStoreStatus {
            agents: self.agents_store_status(),
            presets: self.presets().store_status(),
        }
    }

    fn agent_snapshot(&self, id: AgentId) -> Option<AgentProfile> {
        AgentManager::agent_snapshot(self, id)
    }

    // ADR-0291 R17
    fn create_agent(&self, profile: AgentProfile) -> Result<AgentProfile, PtyError> {
        AgentManager::try_create_agent(self, profile)
    }

    fn activate_profile(
        &self,
        profile: &AgentProfile,
        mode: SpawnMode,
    ) -> Result<StartedAgent, PtyError> {
        AgentManager::activate_profile(self, profile, mode).map(|info| StartedAgent {
            id: info.id,
            name: info.name,
            status: info.status,
        })
    }

    fn rename_agent(
        &self,
        id: AgentId,
        display_name: Option<String>,
    ) -> Result<RenameOutcome, PtyError> {
        AgentManager::try_rename_agent(self, id, display_name)
    }

    fn reparent_agent(&self, child: AgentId, parent: Option<AgentId>) -> Result<bool, PtyError> {
        AgentManager::try_reparent_agent(self, child, parent)
    }

    fn preset_cwd(&self, id: &str) -> Option<String> {
        let id: PresetId = id.parse().ok()?;
        AgentManager::presets(self)
            .list()
            .into_iter()
            .find(|preset| preset.id == id)
            .map(|preset| preset.cwd.to_string_lossy().into_owned())
    }

    fn list_queued_inputs(&self, id: AgentId) -> Option<QueuedListing> {
        AgentManager::list_queued_inputs(self, id).ok()
    }

    fn cancel_queued_input(
        &self,
        id: AgentId,
        input_id: &str,
    ) -> Result<CancelOutcome, CancelError> {
        AgentManager::cancel_queued_input(self, id, input_id)
    }

    fn interrupt_agent(&self, id: AgentId) -> Result<(), PtyError> {
        AgentManager::interrupt(self, id)
    }
}

/// The backend output format of a newly created agent.
///
/// ★Why StreamJson★: the caller of this family is the LLM, and the default of the creation command the LLM calls on the frontend
///   (`agentlist.createAgent`) is already StreamJson (ADR-0078). If two entries gave different defaults to the same verb,
///   it would become "created it, but the screen is different".
/// ★The home of the value is here alone★ — even as entries opening the create verb grow, they reference this instead of keeping their own constant.
/// ★**There is only one place pinning the value itself**★ — `tests::the_new_agent_default_output_format_is_pinned_to_a_literal`.
/// All other assertions are **relative comparisons** with this constant, so they move along if the value is flipped (so without that one,
/// an edit changing this line would pass with the whole suite green).
pub const NEW_AGENT_OUTPUT_FORMAT: CoreAgentOutputFormat = CoreAgentOutputFormat::StreamJson;

// ── Backend creation policy of the LLM control surface (ADR-0219 — 2026-09-22 · ADR-0279) ───────────────────────────
//
// ★The grounds switched★ — what set up this table was 「do not open」 (user decision 2026-09-07 · TRD S21 §6-G), and that
// decision stood on the premise that **the LLM cannot get past the folder trust modal**. That premise was overturned by measurement on 2026-09-22
// (the app-server link has no such modal) and the refusal was removed — the canonical record of that reversal is ADR-0219. Do not close this table again
// on the grounds of the old coordinates: that place was already reconsidered as (b) in the 「Rejected alternatives」 of ADR-0219.
//
// ★The home of the policy is here alone★ — there are two doors asking this decision, and they live in different crates:
//   ① `agent.new` (this file) — the declaration vocabulary `AgentBackend` stands narrowed by the vocabulary itself, and this table stands one more layer
//      on top (it stops at runtime an edit that widens the vocabulary without widening the policy — that arm does not reach today).
//   ② the daemon `SpawnByCwd` handler (`connection_core.rs` of `engram-dashboard-daemon`) — asks this table about the wire
//      word of the spawn packet and refuses before an instant profile goes onto the roster and disk (ADR-0279). The shell
//      `agent.spawnInto` and the frontend `slot.createAgentHere` · `agent.spawn` all send this packet.
//      ★This door does not distinguish callers — though named 「LLM control surface」, human paths see this table too★
//      (the daemon does not know who sent the packet · ADR-0279 「Accepted cost」 · impact 0 today — the table closes
//      nothing).
// ★The shell is not a door★ — the shell `agent.spawnInto` carries only a typo net (`parse_backend` — asks the wire enum) and
// does not look at this table. The shell does not depend on this crate (ADR-0270 decision 1).
// ★There are creation paths that do not look at the table — do not miss them when counting doors★ (an uncounted door stays open without ever
// looking at the table): the agent bus `agent.spawn` (fixed to claude — that verb's comment) · the daemon `CreateProfile` (the frontend's reserved-
// node creation doors — `agentlist.createCodex` etc. — go there). The frontend command registry (`humanOnly`) is not
// a door looking at this table — the `humanOnly` of the codex door was removed on 2026-09-22.
// If each door held its own list, 「a backend one door creates but another cannot」 would appear. The places measuring the mismatch
// = ① `tests::new_creates_exactly_what_the_llm_backend_policy_opens` ·
// `tests::new_advertises_exactly_the_backends_the_llm_policy_opens` (this file) · ② the daemon's
// `every_wire_backend_declares_an_llm_policy` in `connection_core.rs` ·
// `the_production_core_asks_the_agent_backend_policy` · `spawn_by_cwd_refuses_a_backend_the_policy_closes`.
//
// ★Why it asks by word and not by type★ — the type ② brings is the wire enum `protocol::AgentBackendKind`, but this
// crate does not depend on protocol (the canonical source of that prohibition = the `[dependencies]` comment of this crate's `Cargo.toml`).
// The only thing the two crates can share is the word, and a spelling divergence is caught by ②'s first test.

/// One line of the policy table.
pub struct LlmBackendPolicy {
    /// The backend word. Compared **case-insensitively** — this crate's declaration vocabulary writes `Claude` and the wire vocabulary
    /// `claude` (the reason for that difference is already carried by the docs of the two enums), and there is no reason to write the same backend
    /// on two lines.
    pub word: &'static str,
    /// `None` = this surface creates it. `Some` = it does not — carries **the reason and when it opens** together.
    pub refusal: Option<&'static str>,
}

/// ★Which backends the LLM control surface can create — the only place of that policy★. Despite its name, the daemon `SpawnByCwd`
/// door applies this table without distinguishing callers and blocks human paths too (② of the section header above · ADR-0279).
///
/// ★Today this table refuses nothing★ (codex's refusal was removed on 2026-09-22 — the reason is on that line).
/// Still, there are two reasons not to remove the table: when the next backend comes, 「no declaration = closed」 ([`NO_POLICY_DECLARED`]) must
/// stand as is, and this is the place that keeps creation doors from each holding their own list.
///
/// ★Reopen ADR-0279 before changing any line's `refusal` to `Some` or bringing in an undeclared word★ —
/// the daemon `SpawnByCwd` door blocks human paths too. The trigger for reopening that ADR is exactly that edit (then a caller axis is added
/// to the spawn packet).
///
/// ★Opening one line alone does not finish it★ — `agent.new` actually creates only after adding that variant to the `AgentBackend` vocabulary and filling
/// the pair in [`backend_command`]. Opening only this table without doing those two turns the tests above red — that is the only guidance
/// left for whoever opens this table.
///
/// ★What this table splits is **only the backend word — but the output mode axis already has an LLM expression**★:
/// it is the `output_format` field of `agent.new` (`Terminal`|`StreamJson`, unspecified = [`NEW_AGENT_OUTPUT_FORMAT`]), and
/// [`verb_new`] passes that value as is to [`backend_command`] **regardless of the backend**. So
/// opening a word opens both modes together. ★At one point this place said 「the output mode axis has no LLM expression anywhere」,
/// but that field existed before that sentence — it was false from the moment it was written.★ Do not build a second entry on the mode axis
/// on the grounds of that sentence (「do not make a second surface」 of CLAUDE.md 「LLM-first control」).
pub const LLM_BACKEND_POLICY: &[LlmBackendPolicy] = &[
    LlmBackendPolicy {
        word: "claude",
        refusal: None,
    },
    // ★codex's refusal was removed on 2026-09-22, and the reason it was removed is that the premise that refusal was built on was overturned
    //   by measurement★. Old reason = 「in a never-seen folder codex shows its own trust confirmation modal and a non-human caller cannot get past it」.
    //   Measured (codex 0.155.1, 2026-09-22) = the `app-server` path returns `thread/start`
    //   as **success** for a never-seen folder (about 338ms · no error · no inbound trust
    //   request). And it **writes that folder itself** into its own `config.toml` as `trust_level = "trusted"`.
    //   That is, the folder trust modal was **a TUI-only device**.
    // ★So no code checking trust is kept★ — this path has no gate to check in the first place, and building a check that pretends there is one
    //   would only add a dependency reading codex's settings file (TOML) while guarding nothing.
    LlmBackendPolicy {
        word: "codex",
        refusal: None,
    },
];

/// The reason for a word not in the table — ★absence is not open but closed★. If a new backend silently opened without a policy declaration,
/// this gate would have guarded nothing.
const NO_POLICY_DECLARED: &str = "no LLM control surface policy has been declared for this backend yet — it is closed until declared (add one line to `commands::LLM_BACKEND_POLICY` of engram-dashboard-agent).";

/// Whether **the door asking this policy can create** the backend of this word now — `None` = creates, `Some(reason)` = does not.
///
/// ★A different axis from 「an unknown word」★ — a value giving `Some` here is also a legitimate backend this repository knows, so lumping the two
/// refusals into one text would make the caller try to fix a typo that does not exist. The caller must emit the two axes each with a different
/// text.
/// ★Do not read it as blocking only non-human callers★ — the daemon `SpawnByCwd` door applies this decision without distinguishing callers
/// and blocks human paths too (frontend `slot.createAgentHere` · `agent.spawn`) (ADR-0279). Only on paths that do not ask this decision
/// (the daemon `CreateProfile` etc. — the section header above) is it created as is.
///
/// A word not in the table is also `Some` (fail-closed).
pub fn llm_creation_refusal(backend_word: &str) -> Option<&'static str> {
    LLM_BACKEND_POLICY
        .iter()
        .find(|policy| policy.word.eq_ignore_ascii_case(backend_word))
        .map_or(Some(NO_POLICY_DECLARED), |policy| policy.refusal)
}

/// The word of the declaration vocabulary — asking the policy table needs a word. `AgentBackend` has only unit variants, so serde
/// serialization is the word itself, and writing it by hand would silently diverge as variants grow. If serialization fails, it becomes an empty word
/// and the policy closes (fail-closed).
fn backend_word(backend: &AgentBackend) -> String {
    serde_json::to_value(backend)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Declaration vocabulary → run command.
///
/// ★This `match` sweeps **the declaration enum** — even if the core gains backends, compilation does not break here★.
/// The net catching that direction (the core grew while the declaration stays narrow) is the reverse `match` sweeping `AgentCommand`,
/// and its only place is `tests::the_declared_vocabularies_are_pinned_to_the_core_ones`.
/// ★`output_format` is carried as is **regardless of the backend**★ — so opening one word opens both modes
/// together (the place splitting codex's two modes is `backend::codex::is_app_server`, and that decision also looks at this field).
fn backend_command(backend: AgentBackend, output_format: CoreAgentOutputFormat) -> AgentCommand {
    match backend {
        AgentBackend::Claude => AgentCommand::Claude {
            extra_args: vec![],
            output_format,
        },
        // ★`extra_args` being empty is a decision★ — this entry does not grow the argument surface (no field to choose the model, reasoning effort
        //   and the like). The arguments codex actually receives are all assembled from this one command by
        //   `backend::codex::build_spec`.
        AgentBackend::Codex => AgentCommand::Codex {
            extra_args: vec![],
            output_format,
        },
    }
}

/// Declaration vocabulary → core vocabulary. **Unspecified is [`NEW_AGENT_OUTPUT_FORMAT`]**, and values outside the vocabulary are rejected
/// by deserialization as `INVALID_ARGUMENT` before reaching here (no silent fallback — the same discipline as the frontend's `coerceOutputFormat`).
fn output_format(given: Option<AgentOutputFormat>) -> CoreAgentOutputFormat {
    match given {
        None => NEW_AGENT_OUTPUT_FORMAT,
        Some(AgentOutputFormat::Terminal) => CoreAgentOutputFormat::Terminal,
        Some(AgentOutputFormat::StreamJson) => CoreAgentOutputFormat::StreamJson,
    }
}

/// ★It says **what the text must carry**★ — a missing id and a non-id string get the same answer
/// (`AgentCommandHost::preset_cwd`), so it must be guidance that can fix both at once.
fn preset_not_found(id: &str) -> CommandError {
    CommandError::not_found(format!(
        "no registered path with id '{id}' — preset takes the id of a path bookmark, not its name or the folder path itself"
    ))
}

/// Assembles the `agent.*` table — ★the only place real handlers come in★ (rule T-1).
///
/// ★blocking contract★: handler bodies write to disk while holding the profile lock and poll for resume early exit
///   (`AgentManager::activate_profile`). Polling on an async runtime thread would block that thread, so the assembly
///   must call it behind `spawn_blocking` — the two surfaces calling this table (the daemon's `/control/agent` route and
///   command bus delivery) come in through that one common entry, and that place sits on the blocking pool.
/// ★Why `notify` is taken as an argument★: a verb that changed the roster must notify (see the port doc), but the real thing is
///   owned by the daemon. Having the assembly pass it makes it **impossible to omit** — as a trait default implementation it would silently not be called.
/// ★Even as commands grow, the assembly (the executable) does not change★ — what grows is the declaration block and one line of this function.
/// ★`usage` is an argument for the same reason★: the real `usage.*` (the daemon usage service) is owned by the daemon, so the assembly passes it. Assemblies outside
///   production (smoke bins · tests) pass a port without fields — in those assemblies `usage.*` answers `NOT_FOUND`.
// ADR-0155
pub fn make_table(
    host: Arc<dyn AgentCommandHost>,
    notify: Arc<dyn RosterChanged>,
    usage: Arc<dyn UsageCommandHost>,
) -> CommandTable {
    let mut table = CommandTable::new(COMMAND_SPECS);

    let list = Arc::clone(&host);
    let resolve = Arc::clone(&host);
    let spawn = (Arc::clone(&host), Arc::clone(&notify));
    let new = (Arc::clone(&host), Arc::clone(&notify));
    let rename = (Arc::clone(&host), Arc::clone(&notify));
    let move_ = (Arc::clone(&host), Arc::clone(&notify));
    let list_queued = Arc::clone(&host);
    let cancel_queued = Arc::clone(&host);
    let interrupt = Arc::clone(&host);
    let usage_get = Arc::clone(&usage);
    let usage_refresh = usage;

    // ★Blow up at assembly★: the three insert rejects (a name not in the declared set · duplicate insertion · declared schema text
    //   not JSON) are all **values the build decides**, so they do not differ at runtime. Which one it is is told by the `TableError`
    //   carried with the panic, so the messages below do not assert the reason.
    table
        .insert(
            "agent.list",
            blocking_handler(move |_: AgentListArgs| verb_list(list.as_ref())),
        )
        .expect("could not plug agent.list into the table");
    table
        .insert(
            "agent.resolve",
            blocking_handler(move |args: AgentResolveArgs| verb_resolve(resolve.as_ref(), args)),
        )
        .expect("could not plug agent.resolve into the table");
    table
        .insert(
            "agent.spawn",
            blocking_handler(move |args: AgentSpawnArgs| {
                verb_spawn(spawn.0.as_ref(), spawn.1.as_ref(), args)
            }),
        )
        .expect("could not plug agent.spawn into the table");
    table
        .insert(
            "agent.new",
            blocking_handler(move |args: AgentNewArgs| {
                verb_new(new.0.as_ref(), new.1.as_ref(), args)
            }),
        )
        .expect("could not plug agent.new into the table");
    table
        .insert(
            "agent.rename",
            blocking_handler(move |args: AgentRenameArgs| {
                verb_rename(rename.0.as_ref(), rename.1.as_ref(), args)
            }),
        )
        .expect("could not plug agent.rename into the table");
    table
        .insert(
            "agent.move",
            blocking_handler(move |args: AgentMoveArgs| {
                verb_move(move_.0.as_ref(), move_.1.as_ref(), args)
            }),
        )
        .expect("could not plug agent.move into the table");
    table
        .insert(
            "agent.listQueuedInputs",
            blocking_handler(move |args: AgentListQueuedInputsArgs| {
                verb_list_queued_inputs(list_queued.as_ref(), args)
            }),
        )
        .expect("could not plug agent.listQueuedInputs into the table");
    table
        .insert(
            "agent.cancelQueuedInput",
            blocking_handler(move |args: AgentCancelQueuedInputArgs| {
                verb_cancel_queued_input(cancel_queued.as_ref(), args)
            }),
        )
        .expect("could not plug agent.cancelQueuedInput into the table");
    table
        .insert(
            "agent.interrupt",
            blocking_handler(move |args: AgentInterruptArgs| {
                verb_interrupt(interrupt.as_ref(), args)
            }),
        )
        .expect("could not plug agent.interrupt into the table");
    // ★Both are `blocking_handler` (TRD §3 #17)★ — the bodies wait for the query to end, but as async the daemon entry would cut them off at the first
    //   poll with `OUTCOME_UNKNOWN`. The waiting ends inside the port (a std channel).
    table
        .insert(
            "usage.get",
            blocking_handler(move |args: UsageGetArgs| verb_usage_get(usage_get.as_ref(), args)),
        )
        .expect("could not plug usage.get into the table");
    table
        .insert(
            "usage.refresh",
            blocking_handler(move |args: UsageRefreshArgs| {
                verb_usage_refresh(usage_refresh.as_ref(), args)
            }),
        )
        .expect("could not plug usage.refresh into the table");

    table
}

fn verb_usage_get(
    usage: &dyn UsageCommandHost,
    args: UsageGetArgs,
) -> Result<UsageVendorRow, CommandError> {
    usage
        .get(&args.backend)
        .ok_or_else(|| no_usage_limits(&args.backend))
}

fn verb_usage_refresh(
    usage: &dyn UsageCommandHost,
    args: UsageRefreshArgs,
) -> Result<UsageRefreshOk, CommandError> {
    usage
        .refresh(&args.backend)
        .map(UsageRefreshOk::from)
        .ok_or_else(|| no_usage_limits(&args.backend))
}

/// A backend without a field — **the same code and the same sentence** as the same absence of wire ⟳ (`no_usage_text` of daemon
/// `connection_core`). Only the word is carried in this entry's spelling (`Claude`).
fn no_usage_limits(backend: &AgentBackend) -> CommandError {
    CommandError::not_found(format!(
        "this daemon keeps no usage limits for backend '{}'",
        backend_word(backend)
    ))
}

fn verb_list(host: &dyn AgentCommandHost) -> Result<AgentListOk, CommandError> {
    // ★Everything is built from one roster lookup★: name · liveness · cwd · hierarchy must come from the same snapshot for the rows within one response
    //   to be consistent.
    let agents = host
        .roster()
        .into_iter()
        .map(|row| AgentRow {
            id: row.id.to_string(),
            name: row.canonical_name,
            state: state_of(row.live.as_ref()).to_string(),
            cwd: row.cwd,
            parent: row.parent.map(|p| p.to_string()),
        })
        .collect();
    // Not a value at the same moment as the roster sheet — the store state is as of the last save attempt, so it only matches that well to begin with.
    let store = host.store_status();
    Ok(AgentListOk {
        agents,
        store: RosterStoreRow {
            agents: store_status_row(store.agents),
            presets: store_status_row(store.presets),
        },
    })
}

// The cap of TRD S28 §6.
const RESOLVE_TARGETS_MAX: usize = 8;

// ADR-0307
fn verb_resolve(
    host: &dyn AgentCommandHost,
    args: AgentResolveArgs,
) -> Result<AgentResolveOk, CommandError> {
    let count = args.targets.len();
    if !(1..=RESOLVE_TARGETS_MAX).contains(&count) {
        return Err(CommandError::invalid_argument(format!(
            "targets takes 1 to {RESOLVE_TARGETS_MAX} agent names or ids, got {count}"
        )));
    }
    // Blank words are all gathered and emitted at once — the head is only the first position, so the rest are appended at the end of the text.
    let blanks: Vec<usize> = args
        .targets
        .iter()
        .enumerate()
        .filter(|(_, target)| is_blank(Some(target)))
        .map(|(index, _)| index)
        .collect();
    if let Some((&first, rest)) = blanks.split_first() {
        let mut message = "blank value — an empty argument is usually an unset shell variable; every target needs an agent name or id".to_string();
        if !rest.is_empty() {
            let others: Vec<String> = rest
                .iter()
                .map(|index| format!("targets[{index}]"))
                .collect();
            message.push_str(&format!(" (also blank: {})", others.join(", ")));
        }
        return Err(target_failure(
            first,
            CommandError::invalid_argument(message),
        ));
    }
    // ★Resolve everything with one roster sheet★ — reading again per word would have the lines within one answer see the roster at different moments.
    let roster = host.roster();
    let mut agents = Vec::with_capacity(count);
    for (index, target) in args.targets.into_iter().enumerate() {
        let resolved =
            resolve_in(&roster, &target).map_err(|error| target_failure(index, error))?;
        let live = roster
            .iter()
            .find(|row| row.id == resolved.id)
            .is_some_and(|row| state_of(row.live.as_ref()) == AGENT_STATE_LIVE);
        agents.push(ResolvedAgentRow {
            target,
            id: resolved.id.to_string(),
            name: resolved.name,
            live,
        });
    }
    Ok(AgentResolveOk { agents })
}

/// Puts the head `targets[i]: ` of that position on a failure caused by one word — the code and retry are left as is.
// ADR-0307
fn target_failure(index: usize, mut error: CommandError) -> CommandError {
    let message = format!("targets[{index}]: {}", error.message());
    error.set_message(message);
    error
}

/// Spreads the store state (a tag enum carrying a reason) into a word + optional fields.
// ADR-0291 R17 (D6)
fn store_status_row(status: StoreStatus) -> StoreStatusRow {
    let (state, refusal) = match status {
        StoreStatus::Writable => (StoreStateWord::Writable, None),
        StoreStatus::ReadOnly(refusal) => (StoreStateWord::ReadOnly, Some(refusal)),
        StoreStatus::Refusing(refusal) => (StoreStateWord::Refusing, Some(refusal)),
    };
    StoreStatusRow {
        state,
        reason: refusal.map(|refusal| match refusal {
            Refusal::Newer { .. } => StoreRefusalWord::Newer,
            Refusal::Unreadable => StoreRefusalWord::Unreadable,
        }),
        file_version: match refusal {
            Some(Refusal::Newer { found }) => Some(found),
            _ => None,
        },
    }
}

fn verb_spawn(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    args: AgentSpawnArgs,
) -> Result<AgentSpawnOk, CommandError> {
    let (target, cwd, name) = (
        args.target.as_deref(),
        args.cwd.as_deref(),
        args.name.as_deref(),
    );
    // ★The constraint of this verb is not one field but **a combination**★: by declaration all three are `Option`, but a call must have either `target` or
    //   `cwd`. So guiding with 「may be omitted」 would make a retry that omits it as told **rejected again** with 「one of the two must be given」
    //   — a text looking at one field cannot state this condition, so it is written directly here.
    let blanks = blank_fields(&[("target", target), ("cwd", cwd), ("name", name)]);
    if !blanks.is_empty() {
        return Err(CommandError::invalid_argument(format!(
            "blank value(s) for: {} — an empty argument is usually an unset shell variable. spawn needs exactly one of target (an existing agent to wake) or cwd (a folder to create one in); name only applies when creating",
            blanks.join(", ")
        )));
    }

    // ★Two verbs use one name (wake / create and launch)★: which one is decided by the arguments. If both are given
    //   there is no ground to choose, and silently choosing one would wake someone else instead of the agent meant to be created.
    match (target, cwd) {
        (Some(_), Some(_)) => Err(CommandError::invalid_argument(
            "spawn takes either an existing agent (target) or cwd for a new one, not both",
        )),
        (None, None) => Err(CommandError::invalid_argument(
            "spawn needs either an existing agent to wake (target) or cwd to create a new one",
        )),
        // Waking does not change the name, so name can do nothing — silently ignoring it would make the caller think the name
        //   changed.
        // ★No command identifier is written in the text★: `agent.rename` is a catalog name, not a command that can be typed,
        //   so writing it as is would make the LLM type it into the shell. Per-surface notation is attached by the adapter.
        (Some(token), None) if name.is_some() => Err(CommandError::invalid_argument(format!(
            "name does not apply when waking an existing agent ({token}) — changing a name is a separate verb, so drop this field"
        ))),
        (Some(token), None) => wake_existing(host, notify, token),
        (None, Some(cwd)) => create_and_start(host, notify, cwd, name.map(str::to_string)),
    }
}

/// ★Waking also emits a roster notification — even if the entry count, names and hierarchy are unchanged★: the activation outcome changes that entry's
/// 「last failure」 (success = clear / failure = record, ADR-0172), and that axis flows **only through the profile list**.
/// The liveness transitions the manager emits (`agent_list_updated`) are the live session list and do not carry this value —
/// if not announced here, the result of an LLM wake-up would never appear on screen (the human click path does = the two handles diverge,
/// CLAUDE.md 「LLM-first control」).
// ADR-0172
fn wake_existing(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    token: &str,
) -> Result<AgentSpawnOk, CommandError> {
    let id = resolve(host, token)?.id;
    let Some(profile) = host.agent_snapshot(id) else {
        return Err(not_found(token));
    };
    // ★The mode derivation rule is the same as the WS path (ADR-0076)★: resume if there is a stored session, otherwise
    //   new. Using a different rule here would make the same agent lose its conversation history depending on which entry wakes it.
    //   ★Not split by sid presence alone (ADR-0185)★: it first asks the backend whether the stored sid is **resumable with that command**
    //     — that decision is owned by the same dispatch as boot restore and the WS entry. Without asking,
    //     a shape that cannot resume would launch as Resume, opening a new conversation reported as 「resumed」.
    let mode = if crate::backend::can_resume_profile(&profile) {
        SpawnMode::Resume
    } else {
        SpawnMode::Fresh
    };
    let started = host.activate_profile(&profile, mode);
    // ★It announces on failure too — even on a save refusal★: an activation failure writes that entry's 「last failure」 (ADR-0172 ·
    //   ADR-0291 E2 — this notification was excluded from 「side effects after a change only on `Ok`」).
    notify.roster_changed();
    let started = started.map_err(|e| match e {
        PtyError::Store(error) => store_error(
            "start",
            &error,
            store_command_error(
                &error,
                format!(
                    "could not start agent '{token}': {}",
                    store_error_reason(AGENTS_FILE, &error)
                ),
            ),
        ),
        other => CommandError::internal(format!("could not start agent '{token}': {other}")),
    })?;
    Ok(started_payload(started, false))
}

fn create_and_start(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    cwd: &str,
    name: Option<String>,
) -> Result<AgentSpawnOk, CommandError> {
    // ★This verb has no format or backend field — so the two values below are now **chosen**★ (reconfirmed 2026-09-22).
    //   Back when the vocabulary was claude alone, `Claude` was 「the only value」 so there was nothing to choose, but as `agent.new`
    //   opened codex, this place became **an intended claude pin**.
    // ★Still, no field is added here★ — the entry choosing backend and mode is `agent.new` alone, and adding a field to this verb
    //   would split the same choice across two doors (bringing along a catalog generation bump and binding regeneration).
    //   A caller wanting codex registers with `agent.new` and then wakes it with `agent.spawn {target}`.
    // ★The frontend door of the same name (`agent.spawn` of `src/commands/agentCommands.ts`) is also pinned to claude
    //   for the same reason — the two share the name, so they move together.★ But the policy table ([`LLM_BACKEND_POLICY`]) sees only the frontend door
    //   — that one passes through the daemon `SpawnByCwd`, and this verb does not ask (ADR-0279).
    let stored = register(
        host,
        cwd,
        name,
        backend_command(AgentBackend::Claude, NEW_AGENT_OUTPUT_FORMAT),
    )?;
    notify.roster_changed();
    let started = host.activate_profile(&stored, SpawnMode::Fresh);
    // ★It announces **after** activation too — the notification above is a pre-activation snapshot★: in between 「last failure」 is recorded or
    //   cleared (ADR-0172), and that axis flows only through the profile list. It matters especially on the failure branch — the Err below
    //   goes only to the caller and nothing remains on screen.
    // ADR-0172
    notify.roster_changed();
    let started = started.map_err(|e| {
            // ★The registration is not reverted★: the created agent stays in the roster asleep. Rewinding would create a second
            //   deletion path, and the semantics of deletion itself are undecided (ADR-0122).
            // ★The text carries the recovery path★: the created agent remains in the roster, so what the caller should do is
            //   not 「create again」 but **launching again by that name**. Without writing it, they would create again with the same cwd
            //   and names would grow one by one.
            let created_but = |reason: String| {
                format!(
                    "agent '{}' ({}) was created but did not start: {reason} — it is registered and asleep, so start it again by that name (in a moment) instead of creating another",
                    stored.canonical_name_when_live(),
                    stored.id
                )
            };
            match e {
                // ★The reason carries only the cause ([`store_error_cause`]) — no instruction of what to do is attached★: here 「again a little later」
                //   would read as creating again with the same `--cwd`, producing duplicates, and 「retrying is useless」 contradicts the later 「launch again by that name」.
                //   What to do is given by the one trailing tail.
                // ★Even on a save refusal it is not carried as `CONFLICT` — this branch already created the agent★: the daemon
                //   reads `CONFLICT` as 「a rejection with nothing applied」 and lets go of that request number (daemon
                //   `command_delivery::retains_the_id`). Letting go would make a retry with the same number create again, leaving an undeletable
                //   duplicate (ADR-0122) — only `INTERNAL` holds on to the number.
                // ADR-0291 R17
                PtyError::Store(error) => store_error(
                    "start",
                    &error,
                    CommandError::internal(created_but(store_error_cause(AGENTS_FILE, &error))),
                ),
                other => CommandError::internal(created_but(other.to_string())),
            }
        })?;
    Ok(started_payload(started, true))
}

fn verb_new(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    args: AgentNewArgs,
) -> Result<AgentNewOk, CommandError> {
    let (cwd, preset, name) = (
        args.cwd.as_deref(),
        args.preset.as_deref(),
        args.name.as_deref(),
    );
    // ★Neither field is 「may be omitted」★ — one must be present, so the blank-value guidance points at the **sibling field** too.
    //   Otherwise a retry that omitted it as told would be rejected again with 「one of the two must be given」.
    reject_blanks(&[
        (
            "cwd",
            cwd,
            Blank::OrElse("a folder path, or give preset instead"),
        ),
        (
            "preset",
            preset,
            Blank::OrElse("the id of a registered path, or give cwd instead"),
        ),
        ("name", name, Blank::OrOmit),
    ])?;
    // ★Two fields can say which folder — do not silently choose one★ (the same discipline as target/cwd of
    //   `agent.spawn`): if both are given there is no ground to choose, and if neither, there is no place to create.
    let cwd = match (cwd, preset) {
        (Some(_), Some(_)) => {
            return Err(CommandError::invalid_argument(
                "new takes either a folder (cwd) or a registered path (preset), not both",
            ))
        }
        (None, None) => return Err(CommandError::invalid_argument(
            "new needs either cwd (a folder to work in) or preset (the id of a registered path)",
        )),
        (Some(cwd), None) => cwd.to_string(),
        (None, Some(id)) => {
            let stored = host.preset_cwd(id).ok_or_else(|| preset_not_found(id))?;
            // ★A value that came in a preset gets **the same check as a value given directly**★: the `reject_blanks` above looks only at the fields the caller
            //   typed, so this branch passes that net entirely. Preset registration has no value validation
            //   (`CreatePreset` stores the received string as is), and `register` below does not filter an empty folder either
            //   (there is no code in the core producing `PtyError::CwdDenied`), so without looking here **an agent with an empty working
            //   folder** would sit in the roster, and there is no verb to delete it (ADR-0122).
            if is_blank(Some(&stored)) {
                return Err(CommandError::invalid_argument(format!(
                    "the registered path '{id}' has a blank folder, so there is nothing to work in — register that path again with a real folder, or pass cwd directly"
                )));
            }
            stored
        }
    };
    // ★There is no default★ — unspecified is rejected by deserialization (the doc of `AgentNewArgs::backend`). Do not bring back
    //   `unwrap_or(Claude)` here: with two backends now, it is a path that silently folds 「did not choose」
    //   into claude, and it was already removed from the spawn packet for the same reason.
    let backend = args.backend.clone();
    let word = backend_word(&backend);
    // ★Every door asking the policy looks at this one table★ — this `if` runs on every call but **the refusal arm does not reach today either**.
    //   ★But the **reason** it does not reach changed on 2026-09-22★: before, it was 「because the declaration vocabulary was narrower than the policy table」,
    //   and now it is because **the sets the vocabulary and the table open are exactly the same** (both claude · codex)
    //   and **the table closes no word**. Words outside the table are still rejected by deserialization before that.
    //   ★Still, do not delete it★ — the edit this arm blocks is 「widening the vocabulary without widening [`LLM_BACKEND_POLICY`]」,
    //   and the day that edit comes, the arm comes alive. A test measures that fact =
    //   `tests::new_creates_exactly_what_the_llm_backend_policy_opens` (it turns red if the vocabulary is widened without opening the
    //   table). The sibling door's place = the daemon `SpawnByCwd` handler (`connection_core.rs` — does not distinguish callers ·
    //   ADR-0279). ★**Today that door blocks nothing by policy either**★ (2026-09-22 · ADR-0219 — the wire vocabulary
    //   is only the two claude · codex and the table opened both). The shell `agent.spawnInto` is not a door — the refusal remaining there is
    //   not policy but the typo net of `parse_backend` (ADR-0270). The frontend `commands/registry.ts` is not a sibling door
    //   — the `humanOnly` reason string of the codex door was deleted that day. ★At one point this place said 「the shell's runtime refusal still reaches today」,
    //   which is false since the table opened.★
    if let Some(reason) = llm_creation_refusal(&word) {
        return Err(CommandError::invalid_argument(format!(
            "backend '{word}' is a known word, but this surface does not create it now — {reason}"
        )));
    }
    let stored = register(
        host,
        &cwd,
        name.map(str::to_string),
        backend_command(backend, output_format(args.output_format)),
    )?;
    notify.roster_changed();
    Ok(AgentNewOk {
        agent_id: stored.id.to_string(),
        name: stored.canonical_name_when_live(),
        state: AGENT_STATE_SLEEPING.to_string(),
    })
}

/// The common part of roster registration — `agent.new` and `agent.spawn --cwd` use the same place.
///
/// ★The settled name can differ from the requested name★: a suffix is attached for global roster uniqueness (ADR-0120/0123).
/// So the response reads the name from **the registered value** — returning the requested name would diverge from the screen and the address.
fn register(
    host: &dyn AgentCommandHost,
    cwd: &str,
    name: Option<String>,
    command: AgentCommand,
) -> Result<AgentProfile, CommandError> {
    // ★The typed spelling is tidied here★: this field is filled by people and LLMs, and Windows paths are typed as `C:\work\thing`.
    //   A value from the shell sometimes arrives still wrapped in `"…"` because of spaces. Without absorbing both here,
    //   an agent 「whose folder name is weird as a whole」 would sit in the roster, and there is no verb to delete it (ADR-0122).
    //   Only the spelling is tidied; the existence check is still later (spawn's `dunce::canonicalize`). The rule of not stripping a pair of quotes
    //   with nothing inside guards the earlier blank-value check (`reject_blanks`).
    let cwd = &normalize_spelling(cwd);
    let mut profile =
        AgentProfile::new(cwd.to_string(), command, PathBuf::from(cwd), vec![], false);
    profile.display_name = name;
    // ★The caller has different things to do per failure reason★ — free a slot (CONFLICT) · fix the arguments
    //   (INVALID_ARGUMENT) · retrying is useless (INTERNAL). Mashing them into one code would lose that split.
    host.create_agent(profile).map_err(|e| match e {
        // ★The text reveals the **nature** of the cap★: without this line, the first reaction of a caller (person · LLM) hitting the cap would be
        //   "raise the number" — that would be removing by oneself the backstop placed to stop runaway creation loops.
        PtyError::RosterFull { current, limit } => CommandError::of(
            ErrorCode::Conflict,
            format!(
                "the team already has {current} agents, which is the safety ceiling ({limit}) that stops a runaway create loop — it is not a product limit and raising it is not the fix. Remove agents you no longer need, then try again"
            ),
        ),
        PtyError::CwdDenied => CommandError::invalid_argument(format!(
            "cwd is outside the allowed workspace: {cwd}"
        )),
        PtyError::Unsupported(reason) => CommandError::of(
            ErrorCode::Conflict,
            format!("could not register a new agent: {reason}"),
        ),
        PtyError::Store(error) => store_error(
            "register",
            &error,
            store_command_error(
                &error,
                format!(
                    "could not register a new agent: {} — nothing was registered",
                    store_error_reason(AGENTS_FILE, &error)
                ),
            ),
        ),
        other => CommandError::internal(format!("could not register a new agent: {other}")),
    })
}

fn verb_rename(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    args: AgentRenameArgs,
) -> Result<AgentRenameOk, CommandError> {
    let (token, name) = (args.target.as_str(), args.name.as_str());
    reject_blanks(&[
        ("target", Some(token), Blank::NeedsValue),
        ("name", Some(name), Blank::NeedsValue),
    ])?;
    let id = resolve(host, token)?.id;
    // ★A rename the store did not accept is an error without notification★ — the name is unchanged, so there is no change to announce (ADR-0291 D3).
    let outcome = host
        .rename_agent(id, Some(name.to_string()))
        .map_err(|e| match e {
            PtyError::Store(error) => store_error(
                "rename",
                &error,
                store_command_error(
                    &error,
                    format!(
                        "could not rename '{token}': {} — the name was left as it is",
                        store_error_reason(AGENTS_FILE, &error)
                    ),
                ),
            ),
            other => CommandError::internal(format!("could not rename '{token}': {other}")),
        })?;
    // ★The four outcomes are not mashed★: the settled name · no change since it already holds that family · absent · name space exhausted.
    //   The first two are successes but different facts, so they are carried split by outcome.
    match outcome {
        RenameOutcome::Renamed(committed) => {
            notify.roster_changed();
            Ok(AgentRenameOk {
                agent_id: id.to_string(),
                name: committed,
                outcome: RENAME_OUTCOME_RENAMED.to_string(),
            })
        }
        RenameOutcome::Unchanged(kept) => {
            notify.roster_changed();
            Ok(AgentRenameOk {
                agent_id: id.to_string(),
                name: kept,
                outcome: RENAME_OUTCOME_UNCHANGED.to_string(),
            })
        }
        RenameOutcome::NotFound => Err(not_found(token)),
        RenameOutcome::Exhausted => Err(CommandError::of(
            ErrorCode::Conflict,
            format!("every numbered variant of that name is taken, so '{token}' was left as it is"),
        )),
    }
}

fn verb_move(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    args: AgentMoveArgs,
) -> Result<AgentMoveOk, CommandError> {
    let token = args.target.as_str();
    // ★Absence is not 「no parent given, so detach」★ — the instruction to detach to root is `null`. Folding absence would make one typo
    //   field run as a hierarchy release.
    // ★Absence coming in over the wire cannot get this far★ — the declaration macro does not put `#[serde(default)]` on this field,
    //   so deserialization rejects it with `missing field`. This branch is for paths where code builds the arguments directly and calls.
    let Some(parent_token) = args.parent.as_ref() else {
        return Err(CommandError::invalid_argument(
            "move needs parent: a name/id to move under, or null to move it back to the top level",
        ));
    };
    let parent_token = parent_token.as_deref();
    // ★The recovery path of `parent` differs from the other fields★ — rejecting a blank value only with 「fill in a value」 would hide, in this branch alone, the
    //   `null` branch (detach to top level) the absence text above tells about. But rejecting only this field first would **hide the sibling
    //   field**: a request with both blank would be told only about `parent`, and hear about `target` for the first time on the next trip after fixing it.
    //   The two demands stand in the same check — different guidance per field, at once.
    reject_blanks(&[
        ("target", Some(token), Blank::NeedsValue),
        (
            "parent",
            parent_token,
            Blank::OrElse("a name/id to move under, or null to move it back to the top level"),
        ),
    ])?;
    let child = resolve(host, token)?;
    let parent = match parent_token {
        None => None,
        Some(p) => Some(resolve(host, p)?.id),
    };
    // ★A move the store did not accept is an error without notification★ — the hierarchy is unchanged, so there is no change to announce (ADR-0291 D3).
    let moved = host.reparent_agent(child.id, parent).map_err(|e| match e {
        PtyError::Store(error) => store_error(
            "move",
            &error,
            store_command_error(
                &error,
                format!(
                    "could not move '{token}': {} — it was left where it was",
                    store_error_reason(AGENTS_FILE, &error)
                ),
            ),
        ),
        other => CommandError::internal(format!("could not move '{token}': {other}")),
    })?;
    if !moved {
        // ★`false` alone does not tell the reason★: the target just resolved may have vanished in between (NOT_FOUND),
        //   or the tree structure may have refused it (CONFLICT). The two have different things for the caller to do, so the roster is looked at again
        //   to tell them apart — the reason list corresponds line by line to the refusal conditions of `ProfileRegistry::try_reparent`.
        // ★Name the side that vanished★: if the child's name were given whichever of the two vanished, then when only the parent vanished
        //   the caller would hear that an agent that **is perfectly there** is missing and search in the wrong place.
        let roster = host.roster();
        let gone = |id: AgentId| !roster.iter().any(|row| row.id == id);
        if gone(child.id) {
            return Err(not_found(token));
        }
        // If parent is Some, parent_token is Some too (the match above makes the two together).
        if let Some((parent_token, parent_id)) = parent_token.zip(parent) {
            if gone(parent_id) {
                return Err(not_found(parent_token));
            }
        }
        return Err(CommandError::of(
            ErrorCode::Conflict,
            format!(
                "'{token}' cannot go there. The tree is one level deep, so: an agent cannot be its own parent; the new parent must itself be top-level; and the agent being moved must not already have children."
            ),
        ));
    }
    notify.roster_changed();
    // ★This response is **not** a snapshot of one moment (a scope left knowingly)★: the name is read from the roster after applying, and `parent` is
    //   **the value this call instructed**. So if another call moves the same agent again right after, the response becomes 「the current name + the parent this call attached」,
    //   a combination that may not be true at the same time at any moment. Still, this
    //   report **is not false** — the move this call requested was actually applied, and that is all this answer states.
    //   Wanting an atomic snapshot would need an API where the manager also returns 「the post-apply state」, and that is not a decision
    //   for here. For the same reason, **a target that vanished after applying is also `Ok`** (the requested move happened).
    Ok(AgentMoveOk {
        agent_id: child.id.to_string(),
        // ★The name is read again from the roster **after** applying★: carrying the value at resolve time as is would, because of a rename committed in between,
        //   make the response 「old name + new parent」, **a combination that never existed** (the caller types
        //   the next command with that name). The cost is one roster lookup on the success path.
        // ★If it cannot be read, fall back to the resolve-time name — do not fold it into an empty string★: if it vanished in between,
        //   emitting a nameless row as a success response would make the caller's decider read it as "an unreadable response".
        name: current_name(host, child.id).unwrap_or(child.name),
        parent: parent.map(|p| p.to_string()),
    })
}

// ★The two verbs do not change the roster — they emit no roster notification★ (the waiting list flows through the agent output stream).
// ADR-0231
fn verb_list_queued_inputs(
    host: &dyn AgentCommandHost,
    args: AgentListQueuedInputsArgs,
) -> Result<AgentListQueuedInputsOk, CommandError> {
    let token = args.target.as_str();
    reject_blanks(&[("target", Some(token), Blank::NeedsValue)])?;
    let id = resolve(host, token)?.id;
    let listing = host
        .list_queued_inputs(id)
        .ok_or_else(|| not_running(token))?;
    Ok(AgentListQueuedInputsOk {
        inputs: listing
            .rows
            .into_iter()
            .map(|row| QueuedInputRow {
                id: row.id,
                text: row.text,
                state: row.state.as_str().to_string(),
                cancel: match row.state {
                    ListedState::Cancelling {
                        answer,
                        vendor_closed,
                    } => Some(QueuedInputCancel {
                        answer: answer.as_str().to_string(),
                        vendor_closed,
                    }),
                    ListedState::Queued | ListedState::Sent | ListedState::Unconfirmed => None,
                },
            })
            .collect(),
        as_of_seq: listing.as_of_seq,
        epoch: listing.epoch,
        stopped_after_error: listing.stopped_after_error,
    })
}

// ADR-0231
fn verb_cancel_queued_input(
    host: &dyn AgentCommandHost,
    args: AgentCancelQueuedInputArgs,
) -> Result<AgentCancelQueuedInputOk, CommandError> {
    let (token, input_id) = (args.target.as_str(), args.input_id.as_str());
    reject_blanks(&[
        ("target", Some(token), Blank::NeedsValue),
        ("input_id", Some(input_id), Blank::NeedsValue),
    ])?;
    // ★The text carries the roster name, not the pointing token★ — the common entry rewrites `target` as the resolved id
    //   (daemon `admit_input`), so the token arriving here is usually a UUID.
    let agent = resolve(host, token)?;
    let name = agent.name.as_str();
    match host.cancel_queued_input(agent.id, input_id) {
        Ok(outcome) => Ok(AgentCancelQueuedInputOk {
            outcome: outcome.as_str().to_string(),
        }),
        // ★「already has an outcome」 and 「asleep」 are not told apart★ — the caller has the same thing to do (look at the list again).
        Err(CancelError::NotFound) => Err(CommandError::not_found(format!(
            "'{name}' has no waiting input '{input_id}' — it was already delivered, cancelled or dropped, or the agent is not running; list its queued inputs to see what is still waiting"
        ))),
        // ★It does not guide to call again★: the entry stays pending cancel, so calling again writes nothing and returns `requested`
        //   (idempotent). The outcome is decided by the agent's own lifecycle.
        Err(CancelError::Write(e)) => Err(CommandError::internal(format!(
            "the cancel request for '{input_id}' could not be written to '{name}': {e} — the item stays in the cancelling state and calling again will not resend it; its fate now follows the agent's own lifecycle, so watch it in the queued-input list"
        ))),
    }
}

// ADR-0237
fn verb_interrupt(
    host: &dyn AgentCommandHost,
    args: AgentInterruptArgs,
) -> Result<AgentInterruptOk, CommandError> {
    let token = args.target.as_str();
    reject_blanks(&[("target", Some(token), Blank::NeedsValue)])?;
    // The text carries the roster name — the common entry rewrites `target` as the resolved id (same as `verb_cancel_queued_input`).
    let agent = resolve(host, token)?;
    let name = agent.name.as_str();
    match host.interrupt_agent(agent.id) {
        // ★`requested` is not 「stopped」★ — it only sent the interrupt, and the stop is announced by the turn-end event.
        Ok(()) => Ok(AgentInterruptOk {
            outcome: "requested".to_string(),
        }),
        Err(PtyError::NotFound(_)) => Err(CommandError::not_found(format!(
            "'{name}' is not running, so it has no turn to interrupt"
        ))),
        // The two reasons (no turn to interrupt · a link that cannot interrupt) are not told apart — the link answers with one variant and the caller
        //   has the same thing to do.
        Err(PtyError::Unsupported(reason)) => Err(CommandError::of(
            ErrorCode::Conflict,
            format!(
                "'{name}' has no turn to interrupt, or this agent does not support interrupting: {reason}"
            ),
        )),
        Err(other) => Err(CommandError::internal(format!(
            "could not interrupt '{name}': {other}"
        ))),
    }
}

/// A change with a caller that the store (profiles · presets) did not accept → a caller error (code + retry instruction). The bus and WS (daemon
/// `connection_core`) use this one, and the shell settings (`settings.set` · `settings.reset`) follow the same split (user decision
/// 2026-10-11):
/// - A store refused at load (read-only for this whole run — regardless of cause) = `CONFLICT` · `never` — clears only by fixing or replacing the file and
///   restarting the daemon.
/// - A refusal by the re-decision right before writing — a file written by a newer edition = `CONFLICT` · `never` (retrying gives the same while that file stays) · unreadable =
///   `CONFLICT` · `after-condition` (usually a briefly held lock, so retrying a little later works).
/// - Write failure = `INTERNAL`.
///
/// ★In all three the roster is unchanged★ — a change with a caller commits only if the save succeeds (the `try_` versions of the registry · manager). So
///   `CONFLICT` means exactly what the daemon reads as 「a rejection with nothing applied」 (daemon
///   `command_delivery::retains_the_id`).
/// ★Today that retry instruction does not reach the caller★ — the socket bus exit lowers it to `never` (daemon
///   `command_delivery::send_reply` · ADR-0159) and the control route's answer does not carry that field. So 「again a little later」 is
///   carried by the text ([`store_error_reason`]) and the help together.
// ADR-0291 R17
pub fn store_command_error(error: &StoreError, message: String) -> CommandError {
    match error {
        StoreError::Refused(Refusal::Unreadable) => {
            CommandError::with_retry(ErrorCode::Conflict, message, RetryMode::AfterCondition)
        }
        StoreError::ReadOnly(_) | StoreError::Refused(Refusal::Newer { .. }) => {
            CommandError::of(ErrorCode::Conflict, message)
        }
        StoreError::Io(_) => CommandError::internal(message),
    }
}

/// The reason carried in the caller text — which file (`file`) did not accept and why, and what the caller should do. ★It is English because it is a surface the LLM reads★ —
/// the `Display` of the store errors themselves is for logs.
// ADR-0291 R17
pub fn store_error_reason(file: &str, error: &StoreError) -> String {
    let cause = store_error_cause(file, error);
    match error {
        StoreError::ReadOnly(Refusal::Newer { .. }) => format!(
            "{cause}; retrying will not help (use the newer build or replace the file, then restart the daemon)"
        ),
        StoreError::ReadOnly(Refusal::Unreadable) => format!(
            "{cause}; retrying will not help (fix or replace the file, then restart the daemon)"
        ),
        StoreError::Refused(Refusal::Newer { .. }) => {
            format!("{cause}; retrying will not help while that file is there")
        }
        StoreError::Refused(Refusal::Unreadable) => {
            format!("{cause}; this is usually a brief lock, so retry in a moment")
        }
        StoreError::Io(_) => cause,
    }
}

/// From the same reason, only the cause — which file (`file`) did not accept and why. Used where an instruction of what to do does not fit that request (a launch failure
/// after already creating — what to do is given by that text's tail).
// ADR-0291 R17
fn store_error_cause(file: &str, error: &StoreError) -> String {
    match error {
        StoreError::ReadOnly(Refusal::Newer { found }) => format!(
            "{file} was written by a newer build (version {found}) and is read-only for this daemon run"
        ),
        StoreError::ReadOnly(Refusal::Unreadable) => format!(
            "{file} could not be read when the daemon started and is read-only for this run"
        ),
        StoreError::Refused(Refusal::Newer { found }) => format!(
            "{file} was written by a newer build (version {found}) and was not overwritten"
        ),
        StoreError::Refused(Refusal::Unreadable) => {
            format!("{file} could not be read just now and was not overwritten")
        }
        StoreError::Io(io) => format!("writing {file} failed: {io}"),
    }
}

/// Returns as is the caller error (`failure`) of a change the store did not accept, while recording it — ★the one line recording the refusal is
/// here alone★ (the place where that request becomes an error on this surface). The store's log is the file-side fact (what blocked it) and this line is the request-side
/// fact (which request was blocked). `action` = what it was trying to do when blocked. Values the caller typed (names · paths) are not put in the log.
// ADR-0291 R17
fn store_error(action: &'static str, error: &StoreError, failure: CommandError) -> CommandError {
    tracing::warn!(
        action,
        file = AGENTS_FILE,
        %error,
        "the store did not accept it, so this request is returned as an error (command bus)"
    );
    failure
}

/// It is in the roster but there is no live session — only a live incarnation holds a waiting list.
fn not_running(token: &str) -> CommandError {
    CommandError::not_found(format!(
        "'{token}' is not running, so it holds no queued inputs — an asleep agent has nothing waiting"
    ))
}

fn current_name(host: &dyn AgentCommandHost, id: AgentId) -> Option<String> {
    host.roster()
        .into_iter()
        .find(|row| row.id == id)
        .map(|row| row.canonical_name)
}

fn started_payload(started: StartedAgent, created: bool) -> AgentSpawnOk {
    AgentSpawnOk {
        agent_id: started.id.to_string(),
        name: started.name,
        state: state_of(Some(&started.status)).to_string(),
        created,
    }
}

/// **The single mapping** of state → wire vocabulary. The predicate is [`AgentStatus::is_live`] alone (ADR-0116).
///
/// ★Why no literal is baked in (a place that actually diverged)★: if a change verb baked in `"live"`, an agent that dies right after waking
/// (ADR-0082 resume early exit) would be reported as alive by `spawn` and as asleep by the `list` right after —
/// and the caller would write mail to a corpse.
fn state_of(status: Option<&AgentStatus>) -> &'static str {
    match status {
        Some(s) if s.is_live() => AGENT_STATE_LIVE,
        _ => AGENT_STATE_SLEEPING,
    }
}

/// Pointing token → one agent.
///
/// ★Rules (must be **the same** as the mail entry)★: ① an agent id match is looked at first — so a *name* that looks like a UUID cannot
/// hijack id pointing ② then **exact** name match (case-sensitive · no prefix matching · no whitespace correction) ③ if the same name
/// exists two or more times, nobody is chosen and it is refused.
/// ★Do not bring in a lenient resolver★: people and LLMs read the mail recipient token and the control pointing token as pointing at the same X.
/// If the two entries' resolutions diverged, the agent that got the mail and the agent that was renamed would differ —
/// that match is guarded by the daemon crate's cross-check test, which runs [`resolve_in`].
// ADR-0132
fn resolve(host: &dyn AgentCommandHost, token: &str) -> Result<ResolvedAgent, CommandError> {
    resolve_in(&host.roster(), token)
}

/// The pure kernel of [`resolve`] — **the very resolution rule the control entry actually uses**.
///
/// ★Two reasons it is `pub`★: ① for the daemon crate's cross-check test to measure whether it is **the same rule** as the mail entry, it must run the resolver the real entry
/// uses. Taking the roster as an argument means that test does not have to imitate the whole `AgentCommandHost`,
/// and since no separate copy is kept, what is measured and what runs cannot diverge. ② the daemon's input lease check
/// (`control::commands::admit_input` — the production path) resolves the `target` of [`INPUT_AFFECTING`] commands once **by the same
/// rule** as the verb body and looks at the lease with that id. With two resolvers, the agent checked and the agent executed would diverge.
/// ★The outcome is read by code★: absent = `NOT_FOUND` · two or more of the same name = `CONFLICT` (the two codes this function emits). At the end of both texts
/// ` — candidates: a, b` is appended (at most 5 · sorted) — absent = names worth choosing (`none` if the roster is empty) · same name = those
/// ids. The text is prose to read, not a contract (the CLI · cross-check look only at the code).
// ADR-0132
// ADR-0155
// ADR-0231
// ADR-0307
pub fn resolve_in(roster: &[AgentRosterRow], token: &str) -> Result<ResolvedAgent, CommandError> {
    let found = |row: &AgentRosterRow| ResolvedAgent {
        id: row.id,
        name: row.canonical_name.clone(),
    };
    // ★Not compared in canonical spelling★: the daemon converts the agent fields of shell commands (including the old `agent_id`) with this function (ADR-0308),
    //   so matching only the canonical spelling would make old callers that sent ids in other spellings such as uppercase get `NOT_FOUND`. An id-shaped value not in the roster
    //   falls through to the name.
    if let Ok(id) = AgentId::parse_str(token) {
        if let Some(row) = roster.iter().find(|r| r.id == id) {
            return Ok(found(row));
        }
    }
    let same_name: Vec<&AgentRosterRow> = roster
        .iter()
        .filter(|r| r.canonical_name == token)
        .collect();
    match same_name.as_slice() {
        [row] => Ok(found(row)),
        [] => Err(CommandError::not_found(with_candidates(
            not_found_message(token),
            name_candidates(roster, token),
        ))),
        rows => {
            let mut ids: Vec<String> = rows.iter().map(|row| row.id.to_string()).collect();
            ids.sort_unstable();
            ids.truncate(CANDIDATES_MAX);
            Err(CommandError::of(
                ErrorCode::Conflict,
                with_candidates(
                    format!(
                        "more than one agent is called '{token}', so this command would have to guess — use the agent id instead; the roster listing shows both"
                    ),
                    ids,
                ),
            ))
        }
    }
}

// The cap of TRD S28 §5-3.
const CANDIDATES_MAX: usize = 5;

/// Name candidates to give for a missing word — names containing that word (case-insensitive) if any, otherwise the first ones of all in name order.
/// The same name is carried only once.
// ★Sorted★: the row order of the roster is not fixed (asleep rows come from a new HashMap each time — `AgentManager::roster`).
//   Cutting in that order would give the same request different candidates each call.
fn name_candidates(roster: &[AgentRosterRow], token: &str) -> Vec<String> {
    let mut names: Vec<&str> = roster
        .iter()
        .map(|row| row.canonical_name.as_str())
        .collect();
    names.sort_unstable();
    names.dedup();
    let folded = token.to_lowercase();
    let containing: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| name.to_lowercase().contains(&folded))
        .collect();
    let picked = if containing.is_empty() {
        names
    } else {
        containing
    };
    picked
        .into_iter()
        .take(CANDIDATES_MAX)
        .map(str::to_string)
        .collect()
}

/// `none` if there are no candidates (an empty roster) — dropping the tail would make 「no candidates」 indistinguishable from 「a text that carries no candidates」.
fn with_candidates(message: String, candidates: Vec<String>) -> String {
    if candidates.is_empty() {
        return format!("{message} — candidates: none");
    }
    format!("{message} — candidates: {}", candidates.join(", "))
}

/// The agent the pointing pointed at — the id and **the name the roster said at that time**.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAgent {
    pub id: AgentId,
    pub name: String,
}

fn not_found(token: &str) -> CommandError {
    CommandError::not_found(not_found_message(token))
}

fn not_found_message(token: &str) -> String {
    format!(
        "no agent called '{token}' — names are matched exactly, with no case-folding, prefixes or trimming; the roster listing shows the names in use"
    )
}

/// A whitespace-only value is **rejected rather than folded into absence**.
///
/// ★The shape where an unset shell variable expands into an empty argument (`--parent "$UNSET"`) realistically comes in★ — folding it
/// as "not given" would silently turn a single typo into a different action (hierarchy release etc.).
/// ★Emitted **all gathered** rather than cut per field★: cutting at the first field would make a caller who sent three blank values repeat rejection and fix
/// three times (for the same reason the entry's unknown-field check also counts all — `CommandTable::check_args`).
/// ★The guidance differs **per field** (load-bearing)★: if a caller (especially an LLM) gets rejected again after doing as told, it circles the same
/// place. Telling a required field "may be omitted" would get the omitting retry rejected as 「missing」, and telling a field where `null` is an active instruction
/// only "fill in a value" would hide that branch entirely. So each field writes **the way that actually works for that field**
/// ([`Blank`]).
fn reject_blanks(fields: &[(&str, Option<&str>, Blank)]) -> Result<(), CommandError> {
    let advice: Vec<String> = fields
        .iter()
        .filter(|(_, given, _)| is_blank(*given))
        .map(|(field, _, what)| match what {
            Blank::NeedsValue => format!("{field} needs a real value"),
            Blank::OrOmit => format!("{field} must either carry a value or be left out entirely"),
            Blank::OrElse(alternative) => format!("{field} needs {alternative}"),
        })
        .collect();
    if advice.is_empty() {
        return Ok(());
    }
    Err(CommandError::invalid_argument(format!(
        "blank value(s) — an empty argument is usually an unset shell variable: {}",
        advice.join("; ")
    )))
}

/// **What actually works** for a field that came blank.
enum Blank<'a> {
    /// A value is required (omitting it is rejected as 「missing」).
    NeedsValue,
    /// Putting a value in or omitting the field entirely works.
    OrOmit,
    /// There is a branch other than a value — that branch is carried in the text as is (e.g. a field where `null` is an active instruction).
    OrElse(&'a str),
}

fn is_blank(given: Option<&str>) -> bool {
    given.is_some_and(|v| v.trim().is_empty())
}

/// All names of fields that came blank — used when **the verb writes** the rejection text directly (places where the constraint is a combination, not one field).
fn blank_fields<'a>(fields: &[(&'a str, Option<&str>)]) -> Vec<&'a str> {
    fields
        .iter()
        .filter(|(_, given)| is_blank(*given))
        .map(|(field, _)| *field)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use engram_dashboard_command::testing::block_on;
    use serde_json::json;

    use super::*;

    /// Fake manager — ★neither a process nor a PTY★. If this harness does not stand, T-1 is broken.
    #[derive(Default)]
    struct FakeHost {
        rows: Mutex<Vec<AgentRosterRow>>,
        profiles: Mutex<HashMap<AgentId, AgentProfile>>,
        started: Mutex<Vec<(AgentId, bool)>>,
        rename_result: Mutex<Option<RenameOutcome>>,
        reparent_ok: Mutex<bool>,
        start_fails: Mutex<bool>,
        create_fails: Mutex<Option<PtyError>>,
        /// When on, the three that change the roster and activation come back with a save refusal (a file written by a newer edition) — the roster is unchanged.
        // ADR-0291 R17
        store_refuses: Mutex<bool>,
        /// The store state `agent.list` carries — by default both writable.
        // ADR-0291 R17 (D6)
        store_status: Mutex<RosterStoreStatus>,
        /// The target to delete from the roster **inside** the reparent call — reproduces 「vanished in the meantime」 (child or parent).
        /// ★Deleted regardless of apply success★: vanishing after a failure exercises the reason branch, vanishing after success the response name's
        /// fallback — putting it on only one side leaves the other side's code unreached by any test.
        vanish_on_reparent: Mutex<Option<AgentId>>,
        /// The state `activate_profile` returns. ★It must be possible to put a terminal one in★ — dying right after launching
        ///   (ADR-0082 resume early exit) is the only way to measure whether the response state is made up, and if this were fixed at `Running`,
        ///   baking `"live"` into `started_payload` would keep the whole suite green.
        started_status: Mutex<Option<AgentStatus>>,
        /// The new name the child gets right after a reparent succeeds — reproduces 「a rename slipped in after applying」.
        rename_on_reparent: Mutex<Option<String>>,
        /// Registered path bookmarks (id → cwd) — the material measuring that preset pointing **actually fetches a folder**.
        presets: Mutex<HashMap<String, String>>,
        /// The answer of the waiting-list query. `None` = no live session.
        queued_listing: Mutex<Option<QueuedListing>>,
        /// The answer of the queued input cancellation (used once and emptied). `NotFound` if empty.
        cancel_answer: Mutex<Option<Result<CancelOutcome, CancelError>>>,
        /// The pointing the two verbs handed to the manager — measures whether the resolved id goes as is.
        queued_calls: Mutex<Vec<(AgentId, Option<String>)>>,
        /// The answer of the interrupt (used once and emptied). `Ok(())` if empty.
        interrupt_answer: Mutex<Option<Result<(), PtyError>>>,
        /// The pointing the interrupt handed to the manager.
        interrupt_calls: Mutex<Vec<AgentId>>,
    }

    /// Roster notification counter — the watcher of 「renamed but the tree shows the old roster」.
    #[derive(Default)]
    struct FakeNotify {
        calls: Mutex<usize>,
    }

    impl RosterChanged for FakeNotify {
        fn roster_changed(&self) {
            *self.calls.lock().expect("notify poisoned") += 1;
        }
    }

    /// Fake usage port — records the received backend and returns the planted row (`None` = no field).
    #[derive(Default)]
    struct FakeUsage {
        row: Mutex<Option<UsageVendorRow>>,
        calls: Mutex<Vec<(&'static str, AgentBackend)>>,
    }

    impl UsageCommandHost for FakeUsage {
        fn get(&self, backend: &AgentBackend) -> Option<UsageVendorRow> {
            self.calls.lock().unwrap().push(("get", backend.clone()));
            self.row.lock().unwrap().clone()
        }

        fn refresh(&self, backend: &AgentBackend) -> Option<UsageVendorRow> {
            self.calls
                .lock()
                .unwrap()
                .push(("refresh", backend.clone()));
            self.row.lock().unwrap().clone()
        }
    }

    fn wiring(host: &Arc<FakeHost>) -> (CommandTable, Arc<FakeNotify>) {
        let notify = Arc::new(FakeNotify::default());
        let table = make_table(
            Arc::clone(host) as Arc<dyn AgentCommandHost>,
            Arc::clone(&notify) as Arc<dyn RosterChanged>,
            Arc::new(FakeUsage::default()) as Arc<dyn UsageCommandHost>,
        );
        (table, notify)
    }

    impl FakeHost {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                reparent_ok: Mutex::new(true),
                ..Self::default()
            })
        }

        /// If save refusal is on, it returns before touching the roster, like the manager's `try_` versions.
        fn refuse_if_set(&self) -> Result<(), PtyError> {
            if *self.store_refuses.lock().unwrap() {
                return Err(PtyError::Store(StoreError::ReadOnly(
                    crate::profile::Refusal::Newer { found: 2 },
                )));
            }
            Ok(())
        }

        fn with_agent(self: &Arc<Self>, name: &str, live: bool, resumable: bool) -> AgentId {
            self.with_status(name, live.then_some(AgentStatus::Running), resumable)
        }

        fn with_preset(self: &Arc<Self>, id: &str, cwd: &str) {
            self.presets
                .lock()
                .unwrap()
                .insert(id.to_string(), cwd.to_string());
        }

        /// Plants the row's state **as is** — to measure what word a terminal-state row comes out as in `list`,
        /// something other than `Running` must be insertable too.
        fn with_status(
            self: &Arc<Self>,
            name: &str,
            live: Option<AgentStatus>,
            resumable: bool,
        ) -> AgentId {
            self.with_status_command(
                name,
                live,
                resumable,
                AgentCommand::Claude {
                    extra_args: vec![],
                    output_format: NEW_AGENT_OUTPUT_FORMAT,
                },
            )
        }

        /// The variant deciding even the row's **command** — to measure whether activation mode derivation looks at the backend axis, commands other than claude
        /// must be plantable too (planting only claude makes that decision always give the same answer, splitting nothing).
        fn with_status_command(
            self: &Arc<Self>,
            name: &str,
            live: Option<AgentStatus>,
            resumable: bool,
            command: AgentCommand,
        ) -> AgentId {
            let profile = AgentProfile::new(
                format!("C:/work/{name}"),
                command,
                PathBuf::from(format!("C:/work/{name}")),
                vec![],
                false,
            );
            let id = profile.id;
            let mut profile = profile;
            profile.display_name = Some(name.to_string());
            if resumable {
                profile.backend_session_id = Some(uuid::Uuid::new_v4());
            }
            self.profiles.lock().unwrap().insert(id, profile);
            self.rows.lock().unwrap().push(AgentRosterRow {
                id,
                canonical_name: name.to_string(),
                cwd: format!("C:/work/{name}"),
                parent: None,
                live,
            });
            id
        }
    }

    impl AgentCommandHost for FakeHost {
        fn roster(&self) -> Vec<AgentRosterRow> {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .map(|r| AgentRosterRow {
                    id: r.id,
                    canonical_name: r.canonical_name.clone(),
                    cwd: r.cwd.clone(),
                    parent: r.parent,
                    live: r.live.clone(),
                })
                .collect()
        }

        fn store_status(&self) -> RosterStoreStatus {
            *self.store_status.lock().unwrap()
        }

        fn agent_snapshot(&self, id: AgentId) -> Option<AgentProfile> {
            self.profiles.lock().unwrap().get(&id).cloned()
        }

        fn create_agent(&self, profile: AgentProfile) -> Result<AgentProfile, PtyError> {
            if let Some(failure) = self.create_fails.lock().unwrap().take() {
                return Err(failure);
            }
            self.refuse_if_set()?;
            let id = profile.id;
            let name = profile.canonical_name_when_live();
            self.profiles.lock().unwrap().insert(id, profile.clone());
            self.rows.lock().unwrap().push(AgentRosterRow {
                id,
                canonical_name: name,
                cwd: profile.cwd.to_string_lossy().to_string(),
                parent: None,
                live: None,
            });
            Ok(profile)
        }

        fn activate_profile(
            &self,
            profile: &AgentProfile,
            mode: SpawnMode,
        ) -> Result<StartedAgent, PtyError> {
            if *self.start_fails.lock().unwrap() {
                return Err(PtyError::SpawnFailed("fake".to_string()));
            }
            self.refuse_if_set()?;
            let resumed = matches!(mode, SpawnMode::Resume);
            self.started.lock().unwrap().push((profile.id, resumed));
            let status = self
                .started_status
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(AgentStatus::Running);
            Ok(StartedAgent {
                id: profile.id,
                name: profile.canonical_name_when_live(),
                status,
            })
        }

        fn rename_agent(
            &self,
            _id: AgentId,
            display_name: Option<String>,
        ) -> Result<RenameOutcome, PtyError> {
            self.refuse_if_set()?;
            Ok(self
                .rename_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| RenameOutcome::Renamed(display_name.unwrap_or_default())))
        }

        fn reparent_agent(
            &self,
            child: AgentId,
            _parent: Option<AgentId>,
        ) -> Result<bool, PtyError> {
            self.refuse_if_set()?;
            let ok = *self.reparent_ok.lock().unwrap();
            // ★It vanishes regardless of apply success★: the window vanishing **after** success is the only path that exercises the response name's fallback.
            //   Putting it only on the failure side leaves that fallback unreached by any test.
            if let Some(vanished) = *self.vanish_on_reparent.lock().unwrap() {
                self.rows.lock().unwrap().retain(|row| row.id != vanished);
            }
            // The window where a rename is committed between applying and responding — in reality another caller issues it.
            if let (true, Some(renamed)) = (ok, self.rename_on_reparent.lock().unwrap().take()) {
                for row in self.rows.lock().unwrap().iter_mut() {
                    if row.id == child {
                        row.canonical_name = renamed.clone();
                    }
                }
            }
            Ok(ok)
        }

        fn preset_cwd(&self, id: &str) -> Option<String> {
            self.presets.lock().unwrap().get(id).cloned()
        }

        fn list_queued_inputs(&self, id: AgentId) -> Option<QueuedListing> {
            self.queued_calls.lock().unwrap().push((id, None));
            self.queued_listing.lock().unwrap().clone()
        }

        fn cancel_queued_input(
            &self,
            id: AgentId,
            input_id: &str,
        ) -> Result<CancelOutcome, CancelError> {
            self.queued_calls
                .lock()
                .unwrap()
                .push((id, Some(input_id.to_string())));
            self.cancel_answer
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Err(CancelError::NotFound))
        }

        fn interrupt_agent(&self, id: AgentId) -> Result<(), PtyError> {
            self.interrupt_calls.lock().unwrap().push(id);
            self.interrupt_answer
                .lock()
                .unwrap()
                .take()
                .unwrap_or(Ok(()))
        }
    }

    fn call(
        table: &CommandTable,
        name: &str,
        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, CommandError> {
        // Called through the table — the only place fitting the arguments to the declared schema (`CommandTable::call`).
        let future = table.call(name, &mut args).expect("a name in the table");
        block_on(future)
    }

    #[test]
    fn table_holds_exactly_the_declared_verbs() {
        let (table, _notify) = wiring(&FakeHost::new());
        let names: Vec<&str> = table.specs().map(|s| s.name).collect();
        assert_eq!(
            names,
            vec![
                "agent.cancelQueuedInput",
                "agent.interrupt",
                "agent.list",
                "agent.listQueuedInputs",
                "agent.move",
                "agent.new",
                "agent.rename",
                "agent.resolve",
                "agent.spawn",
                "usage.get",
                "usage.refresh"
            ]
        );
        assert_eq!(names.len(), COMMAND_SPECS.len());
    }

    #[test]
    fn decls_carry_a_non_empty_json_shape_for_every_verb() {
        let (table, _notify) = wiring(&FakeHost::new());
        for decl in table.decls() {
            let item: serde_json::Value =
                serde_json::from_str(&decl.help).expect("help is one JSON item");
            assert_eq!(item["name"], decl.name.as_str());
            assert!(item["args"]["type"] == "object");
            assert!(item["ok"]["type"] == "object");
            assert!(item["errors"].as_array().is_some_and(|e| !e.is_empty()));
        }
    }

    #[test]
    fn list_reports_live_and_sleeping_without_a_process() {
        let host = FakeHost::new();
        host.with_agent("alpha", true, false);
        host.with_agent("beta", false, false);
        let (table, _notify) = wiring(&host);

        let out = call(&table, "agent.list", json!({})).expect("query succeeds");
        let agents = out["agents"].as_array().expect("array");
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0]["name"], "alpha");
        assert_eq!(agents[0]["state"], "live");
        assert_eq!(agents[1]["state"], "sleeping");
        assert_eq!(agents[0]["parent"], serde_json::Value::Null);
    }

    /// ★The three store states are carried in `agent.list` with their reasons (D6)★ — without `Refusing`, a refusal during the run would look `Writable`
    ///   while change requests come back `CONFLICT`, and the caller would not see the state explaining that error.
    // ADR-0291 R17 (D6)
    #[test]
    fn list_reports_each_store_status_with_its_reason() {
        let host = FakeHost::new();
        let (table, _notify) = wiring(&host);
        let writable = json!({ "state": "Writable", "reason": null, "file_version": null });

        let out = call(&table, "agent.list", json!({})).expect("query succeeds");
        assert_eq!(
            out["store"],
            json!({ "agents": writable, "presets": writable })
        );

        for (status, expected_agents, expected_presets) in [
            (
                RosterStoreStatus {
                    agents: StoreStatus::ReadOnly(Refusal::Newer { found: 3 }),
                    presets: StoreStatus::Refusing(Refusal::Unreadable),
                },
                json!({ "state": "ReadOnly", "reason": "Newer", "file_version": 3 }),
                json!({ "state": "Refusing", "reason": "Unreadable", "file_version": null }),
            ),
            (
                RosterStoreStatus {
                    agents: StoreStatus::Refusing(Refusal::Newer { found: 4 }),
                    presets: StoreStatus::ReadOnly(Refusal::Unreadable),
                },
                json!({ "state": "Refusing", "reason": "Newer", "file_version": 4 }),
                json!({ "state": "ReadOnly", "reason": "Unreadable", "file_version": null }),
            ),
        ] {
            *host.store_status.lock().unwrap() = status;
            let out = call(&table, "agent.list", json!({})).expect("query succeeds");
            assert_eq!(out["store"]["agents"], expected_agents, "{status:?}");
            assert_eq!(out["store"]["presets"], expected_presets, "{status:?}");
        }
    }

    /// ★The real manager carries the state of the two stores each in its own place (D6)★ — the fake manager only returns the planted fields, so a manager-side
    ///   implementation swapping the two or always leaving one `Writable` is caught only with the real thing.
    // ADR-0291 R17 (D6)
    #[test]
    fn a_real_manager_reports_each_store_in_its_own_slot() {
        use crate::preset::{Preset, PresetRegistry, PresetStore};
        use crate::profile::{ProfileRegistry, ProfileStore};
        use crate::session_tracker::{SessionTracker, TrackerConfig};
        use crate::types::{AgentInfo, StatusSink};

        struct NoStatus;
        impl StatusSink for NoStatus {
            fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }
        struct NewerAgents;
        impl ProfileStore for NewerAgents {
            fn save(&self, _profiles: &[AgentProfile]) -> Result<(), StoreError> {
                Err(StoreError::ReadOnly(Refusal::Newer { found: 9 }))
            }
            fn load(&self) -> Vec<AgentProfile> {
                vec![]
            }
            fn status(&self) -> StoreStatus {
                StoreStatus::ReadOnly(Refusal::Newer { found: 9 })
            }
        }
        struct RefusingPresets;
        impl PresetStore for RefusingPresets {
            fn save(&self, _presets: &[Preset]) -> Result<(), StoreError> {
                Err(StoreError::Refused(Refusal::Unreadable))
            }
            fn load(&self) -> Vec<Preset> {
                vec![]
            }
            fn status(&self) -> StoreStatus {
                StoreStatus::Refusing(Refusal::Unreadable)
            }
        }

        let manager = Arc::new(AgentManager::new(
            Arc::new(NoStatus),
            Arc::new(ProfileRegistry::new(Arc::new(NewerAgents))),
            Arc::new(PresetRegistry::new(Arc::new(RefusingPresets))),
            Arc::new(SessionTracker::new(
                TrackerConfig {
                    enabled: false,
                    poll_interval: std::time::Duration::from_secs(1),
                },
                Arc::new(|_, _| {}),
            )),
        ));
        let table = make_table(
            manager as Arc<dyn AgentCommandHost>,
            Arc::new(FakeNotify::default()) as Arc<dyn RosterChanged>,
            Arc::new(FakeUsage::default()) as Arc<dyn UsageCommandHost>,
        );

        let out = call(&table, "agent.list", json!({})).expect("query succeeds");
        assert_eq!(
            out["store"],
            json!({
                "agents": { "state": "ReadOnly", "reason": "Newer", "file_version": 9 },
                "presets": { "state": "Refusing", "reason": "Unreadable", "file_version": null },
            })
        );
    }

    #[test]
    fn spawn_refuses_both_target_and_cwd() {
        let (table, _notify) = wiring(&FakeHost::new());
        let err = call(
            &table,
            "agent.spawn",
            json!({ "target": "alpha", "cwd": "C:/x" }),
        )
        .expect_err("giving both is rejected");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn spawn_refuses_neither_target_nor_cwd() {
        let (table, _notify) = wiring(&FakeHost::new());
        let err = call(&table, "agent.spawn", json!({})).expect_err("giving nothing is rejected");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn spawn_refuses_name_when_waking() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.spawn",
            json!({ "target": "alpha", "name": "beta" }),
        )
        .expect_err("name can do nothing for waking");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn waking_a_stored_session_resumes_it() {
        let host = FakeHost::new();
        let id = host.with_agent("alpha", false, true);
        let (table, notify) = wiring(&host);

        let out = call(&table, "agent.spawn", json!({ "target": "alpha" })).expect("wake");
        assert_eq!(out["created"], false);
        assert_eq!(out["state"], "live");
        assert_eq!(host.started.lock().unwrap().as_slice(), &[(id, true)]);
        assert_eq!(
            *notify.calls.lock().unwrap(),
            1,
            "even if the composition is unchanged, activation changes the last failure (ADR-0172) — that axis flows only through the profile list"
        );
    }

    /// ★A failure from an LLM wake-up must reach the screen★: without this notification, a human click (WS `SpawnProfile`) shows
    /// the failure marker but the command bus does not, so the two handles would not move the same thing.
    // ADR-0172
    #[test]
    fn waking_that_fails_still_announces_the_roster() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, true);
        *host.start_fails.lock().unwrap() = true;
        let (table, notify) = wiring(&host);

        call(&table, "agent.spawn", json!({ "target": "alpha" }))
            .expect_err("premise: activation fails");
        assert_eq!(*notify.calls.lock().unwrap(), 1);
    }

    /// ★Resume is not derived from a stored sid alone (ADR-0185)★ — a state where a sid remains on a command that cannot resume
    /// is made even today (a hand-edited `agents.json` · a profile that switched backends). Launching it as Resume then
    /// **opens a new conversation but reports the result as 「resumed」** — the wire has no marker saying it is a new conversation,
    /// so neither the screen nor the LLM can tell.
    /// ★The WS entry has the same pair★ — the two handles must move the same thing (CLAUDE.md 「LLM-first control」).
    // ADR-0185
    /// ★The sample being shell is intended — do not bring the codex terminal mode back here★: by ADR-0208 that mode
    /// came to use `codex resume <id>`, and its resume axis is now **true**. The only sample with the axis off is shell, and
    /// replacing shell thinking it is a wrong sample would remove what this entry measures.
    #[test]
    fn waking_does_not_resume_a_profile_whose_backend_cannot_resume() {
        let host = FakeHost::new();
        let id = host.with_status_command(
            "shell-with-stale-sid",
            None,
            true,
            AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
        );
        let (table, _notify) = wiring(&host);

        call(
            &table,
            "agent.spawn",
            json!({ "target": "shell-with-stale-sid" }),
        )
        .expect("wake");
        assert_eq!(
            host.started.lock().unwrap().as_slice(),
            &[(id, false)],
            "even with a sid, a command that cannot resume must launch Fresh"
        );
    }

    /// ★The pair of the entry above — the codex terminal mode now stands **on the opposite side** (ADR-0208)★: if the handle is in the roster,
    /// that mode must also launch as Resume. Without this, only the entry above would remain, green even if the axis were turned off entirely.
    // ADR-0208
    #[test]
    fn waking_a_codex_terminal_profile_with_a_stored_handle_resumes() {
        let host = FakeHost::new();
        let id = host.with_status_command(
            "codex-term",
            None,
            true,
            AgentCommand::Codex {
                extra_args: vec![],
                output_format: CoreAgentOutputFormat::Terminal,
            },
        );
        let (table, _notify) = wiring(&host);

        call(&table, "agent.spawn", json!({ "target": "codex-term" })).expect("wake");
        assert_eq!(
            host.started.lock().unwrap().as_slice(),
            &[(id, true)],
            "if it launched Fresh while a handle the hook recorded exists, that id would never be used"
        );
    }

    #[test]
    fn waking_without_a_stored_session_starts_fresh() {
        let host = FakeHost::new();
        let id = host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        call(&table, "agent.spawn", json!({ "target": "alpha" })).expect("wake");
        assert_eq!(host.started.lock().unwrap().as_slice(), &[(id, false)]);
    }

    #[test]
    fn spawn_by_cwd_creates_then_starts() {
        let host = FakeHost::new();
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.spawn",
            json!({ "cwd": "C:/work/gamma", "name": "gamma" }),
        )
        .expect("create + launch");
        assert_eq!(out["created"], true);
        assert_eq!(out["name"], "gamma");
        assert_eq!(host.started.lock().unwrap().len(), 1);
        assert_eq!(
            *notify.calls.lock().unwrap(),
            2,
            "once right after registration (new entry) + once right after activation (the last failure outcome, ADR-0172)"
        );
    }

    #[test]
    fn a_created_agent_that_fails_to_start_is_not_rolled_back() {
        let host = FakeHost::new();
        *host.start_fails.lock().unwrap() = true;
        let (table, _notify) = wiring(&host);

        let err = call(&table, "agent.spawn", json!({ "cwd": "C:/work/gamma" }))
            .expect_err("start fails");
        assert_eq!(err.code(), ErrorCode::Internal);
        assert_eq!(
            host.rows.lock().unwrap().len(),
            1,
            "it remains in the roster"
        );
    }

    #[test]
    fn new_registers_a_sleeping_agent() {
        let host = FakeHost::new();
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/work/delta", "backend": "Claude" }),
        )
        .expect("register");
        assert_eq!(out["state"], "sleeping");
        assert!(host.started.lock().unwrap().is_empty(), "does not launch");
        assert_eq!(
            *notify.calls.lock().unwrap(),
            1,
            "announces the roster change"
        );
    }

    // ── Spelling of hand-typed words ───────────────────────────────────────────────────────────────

    /// Backend words are case-insensitive — this helper calling through the table actually runs the place where that adjustment happens
    /// (`CommandTable::call`).
    #[test]
    fn new_accepts_the_backend_word_in_any_casing() {
        for word in ["claude", "Claude", "CLAUDE", "codex", "Codex", "CODEX"] {
            let host = FakeHost::new();
            let (table, _notify) = wiring(&host);

            let out = call(
                &table,
                "agent.new",
                json!({ "cwd": "C:/work/delta", "backend": word }),
            )
            .unwrap_or_else(|e| panic!("backend '{word}' must be accepted: {e:?}"));
            assert_eq!(out["state"], "sleeping", "{word}");
        }
    }

    /// Leniency applies only to **spelling** — an unknown word is rejected as is.
    #[test]
    fn new_still_refuses_a_backend_word_it_does_not_know() {
        let host = FakeHost::new();
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/work/delta", "backend": "codx" }),
        )
        .expect_err("an unknown word is rejected");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
    }

    /// The emitted spelling does not change — the generated output (the `"enum"` of `bindings/commands.schema.json`) carries this value.
    #[test]
    fn the_declared_backend_words_still_serialize_in_pascal_case() {
        assert_eq!(
            serde_json::to_value(AgentBackend::Claude).expect("serialize"),
            json!("Claude")
        );
        assert_eq!(
            serde_json::to_value(AgentBackend::Codex).expect("serialize"),
            json!("Codex")
        );
    }

    /// The two doors passing through the common registration part (`agent.new` · `agent.spawn --cwd`) seat the same spelling in the roster.
    #[test]
    fn a_windows_path_in_quotes_is_registered_with_the_normalized_spelling() {
        for verb in ["agent.new", "agent.spawn"] {
            let host = FakeHost::new();
            let (table, _notify) = wiring(&host);

            call(
                &table,
                verb,
                json!({ "cwd": r#""C:\work\delta""#, "backend": "Claude" }),
            )
            .unwrap_or_else(|e| panic!("{verb}: {e:?}"));

            let rows = host.rows.lock().unwrap();
            assert_eq!(rows.len(), 1, "{verb}");
            assert_eq!(rows[0].cwd, "C:/work/delta", "{verb}");
        }
    }

    /// Registration failures go out with a different code and retry instruction per reason — because what the caller should do differs.
    #[test]
    fn registration_failures_map_to_distinct_codes() {
        use crate::profile::Refusal;
        let cases = [
            (
                PtyError::RosterFull {
                    current: 40,
                    limit: 40,
                },
                ErrorCode::Conflict,
                RetryMode::Never,
            ),
            (
                PtyError::CwdDenied,
                ErrorCode::InvalidArgument,
                RetryMode::Never,
            ),
            (
                PtyError::SpawnFailed("disk on fire".to_string()),
                ErrorCode::Internal,
                RetryMode::Never,
            ),
            // ADR-0291 R17 — a refusal is `CONFLICT`, and the retry instruction is decided by where it came from and why (user decision
            //   2026-10-11): load refusal = `never` (clears only by restarting the daemon) · re-decision newer edition = `never` · re-decision unreadable
            //   = `after-condition` (usually a briefly held lock). Write failure is `INTERNAL`.
            (
                PtyError::Store(StoreError::ReadOnly(Refusal::Newer { found: 2 })),
                ErrorCode::Conflict,
                RetryMode::Never,
            ),
            (
                PtyError::Store(StoreError::ReadOnly(Refusal::Unreadable)),
                ErrorCode::Conflict,
                RetryMode::Never,
            ),
            (
                PtyError::Store(StoreError::Refused(Refusal::Newer { found: 2 })),
                ErrorCode::Conflict,
                RetryMode::Never,
            ),
            (
                PtyError::Store(StoreError::Refused(Refusal::Unreadable)),
                ErrorCode::Conflict,
                RetryMode::AfterCondition,
            ),
            (
                PtyError::Store(StoreError::Io(std::io::Error::other("disk full"))),
                ErrorCode::Internal,
                RetryMode::Never,
            ),
        ];

        for (failure, expected, retry) in cases {
            let label = format!("{failure:?}");
            let from_store = matches!(failure, PtyError::Store(_));
            let host = FakeHost::new();
            *host.create_fails.lock().unwrap() = Some(failure);
            let (table, notify) = wiring(&host);

            let err = call(
                &table,
                "agent.new",
                json!({ "cwd": "C:/work/delta", "backend": "Claude" }),
            )
            .expect_err("registration fails");
            assert_eq!(err.code(), expected, "{label}");
            assert_eq!(err.retry(), retry, "{label}");
            assert_eq!(
                *notify.calls.lock().unwrap(),
                0,
                "if it failed, the roster did not change"
            );
            if from_store {
                // The text names which file did not accept and why — so the caller finds the place to fix.
                assert!(
                    err.message().contains("agents.json")
                        && err.message().contains("nothing was registered"),
                    "{}",
                    err.message()
                );
            }
        }
    }

    /// ★The reason of a store refusal is in English, and names what to do★ — it is a surface the LLM reads. A load refusal says restart, and a re-decision's
    ///   unreadable says again a little later (the retry instruction is lowered at the daemon exit, so the text carries that role).
    // ADR-0291 R17
    #[test]
    fn store_refusal_reasons_are_english_and_name_the_way_out() {
        use crate::profile::Refusal;
        let at_load = store_error_reason("agents.json", &StoreError::ReadOnly(Refusal::Unreadable));
        assert!(
            at_load.starts_with("agents.json could not be read")
                && at_load.contains("restart the daemon"),
            "{at_load}"
        );
        let newer = store_error_reason(
            "presets.json",
            &StoreError::Refused(Refusal::Newer { found: 3 }),
        );
        assert!(
            newer.starts_with("presets.json was written by a newer build (version 3)")
                && newer.contains("retrying will not help"),
            "{newer}"
        );
        let transient =
            store_error_reason("agents.json", &StoreError::Refused(Refusal::Unreadable));
        assert!(transient.contains("retry in a moment"), "{transient}");
        assert_eq!(
            store_error_reason(
                "agents.json",
                &StoreError::Io(std::io::Error::other("disk full"))
            ),
            "writing agents.json failed: disk full"
        );
    }

    /// ★A way to register without a dialog★ — one preset id decides the working folder (the path bookmarks of ADR-0061).
    #[test]
    fn new_resolves_a_preset_id_to_its_folder() {
        let host = FakeHost::new();
        host.with_preset("bookmark-1", "C:/work/from-preset");
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.new",
            json!({ "preset": "bookmark-1", "backend": "Claude" }),
        )
        .expect("register");

        assert_eq!(out["state"], "sleeping");
        assert_eq!(
            host.rows.lock().unwrap()[0].cwd,
            "C:/work/from-preset",
            "it must be registered with the folder the preset pointed at"
        );
        assert_eq!(*notify.calls.lock().unwrap(), 1);
    }

    /// ★Giving both or nothing **does not silently choose one**★ — choosing would create the agent in a folder other than the one the caller intended
    /// (and there is no way to delete it — ADR-0122).
    #[test]
    fn new_refuses_both_a_folder_and_a_preset_and_refuses_neither() {
        let host = FakeHost::new();
        host.with_preset("bookmark-1", "C:/work/from-preset");
        let (table, notify) = wiring(&host);

        for args in [
            json!({ "cwd": "C:/work/x", "preset": "bookmark-1", "backend": "Claude" }),
            json!({ "backend": "Claude" }),
        ] {
            let err = call(&table, "agent.new", args.clone()).expect_err("no ground to choose");
            assert_eq!(err.code(), ErrorCode::InvalidArgument, "{args}");
            assert!(
                err.message().contains("cwd") && err.message().contains("preset"),
                "must say what to give for it to work: {}",
                err.message()
            );
        }
        assert!(
            host.rows.lock().unwrap().is_empty(),
            "a rejection does not register"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "a rejection does not notify"
        );
    }

    /// ★The preset branch does not **bypass** the empty-folder check★ — that registration has no value validation, so a bookmark holding only whitespace
    /// can actually exist, and letting it through would leave an agent with an empty working folder with no means of deleting it.
    #[test]
    fn a_preset_holding_a_blank_folder_is_refused_like_a_blank_cwd() {
        let host = FakeHost::new();
        host.with_preset("blank-bookmark", "   ");
        let (table, notify) = wiring(&host);

        let err = call(
            &table,
            "agent.new",
            json!({ "preset": "blank-bookmark", "backend": "Claude" }),
        )
        .expect_err("an empty folder is not a registration target");

        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("blank-bookmark"),
            "must point out which bookmark: {}",
            err.message()
        );
        assert!(
            host.rows.lock().unwrap().is_empty(),
            "a rejection does not register"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "a rejection does not notify"
        );
    }

    #[test]
    fn an_unknown_preset_is_not_found() {
        let host = FakeHost::new();
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.new",
            json!({ "preset": "no-such-bookmark", "backend": "Claude" }),
        )
        .expect_err("a missing bookmark");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(
            host.rows.lock().unwrap().is_empty(),
            "a rejection does not register"
        );
    }

    /// ★Unspecified is StreamJson, values outside the vocabulary are rejected★ — with a silent fallback it would become "created it, but the screen is different"
    /// (ADR-0078) and the caller would never learn of their own typo.
    #[test]
    fn new_defaults_the_output_format_and_refuses_a_word_outside_the_vocabulary() {
        let host = FakeHost::new();
        let (table, _notify) = wiring(&host);

        let format_of = |name: &str| {
            let id = host
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|row| row.canonical_name == name)
                .expect("registered row")
                .id;
            match host.profiles.lock().unwrap()[&id].command.clone() {
                AgentCommand::Claude { output_format, .. } => output_format,
                other => panic!("must be a claude profile: {other:?}"),
            }
        };

        call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/x", "name": "silent", "backend": "Claude" }),
        )
        .expect("unspecified");
        assert_eq!(format_of("silent"), NEW_AGENT_OUTPUT_FORMAT);

        call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/x", "name": "typed", "output_format": "Terminal", "backend": "Claude" }),
        )
        .expect("explicit");
        assert_eq!(format_of("typed"), CoreAgentOutputFormat::Terminal);

        // A spelling differing only in case is not outside the vocabulary — it is moved to the declared spelling and works.
        call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/x", "name": "folded", "output_format": "terminal", "backend": "Claude" }),
        )
        .expect("a value differing only in spelling");
        assert_eq!(format_of("folded"), CoreAgentOutputFormat::Terminal);

        let err = call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/x", "output_format": "Ndjson", "backend": "Claude" }),
        )
        .expect_err("a value outside the vocabulary");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("Terminal") && err.message().contains("StreamJson"),
            "must say what works: {}",
            err.message()
        );
    }

    /// ★The fact that a rejection **touches nothing in the roster** is decision material for another crate★
    ///
    /// The daemon uses the premise 「failure answer = nothing remained」 when deciding whether to block re-queries after the deadline
    /// (`command_delivery::retains_the_id`). If that premise breaks, that side is **silently** wrong — holding on to a failed number
    /// blocks a normal retry, or letting go of an applied number makes it apply twice.
    /// ★`agent.spawn` is **absent** from this assertion — because only that verb is different★: if it cannot launch after creating, it
    /// answers `INTERNAL` while leaving the agent in the roster (the neighboring test `a_created_agent_that_fails_to_start_is_not_rolled_back`
    /// pins that fact). So the daemon-side decision also splits off that one code as an exception.
    #[test]
    fn a_rejection_from_the_mutating_verbs_leaves_the_roster_untouched() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        host.with_agent("lead", false, false);
        host.with_preset("blank-bookmark", "   ");
        let (table, notify) = wiring(&host);
        let snapshot = || -> Vec<(AgentId, String, Option<AgentId>)> {
            host.roster()
                .into_iter()
                .map(|row| (row.id, row.canonical_name, row.parent))
                .collect()
        };
        let before = snapshot();

        // Sweeps the branches where rejections happen, per verb — the codes differ but all must be 「before touching anything」.
        let rejections = [
            ("agent.new", json!({ "cwd": "   ", "backend": "Claude" })),
            (
                "agent.new",
                json!({ "preset": "no-such-bookmark", "backend": "Claude" }),
            ),
            (
                "agent.new",
                json!({ "preset": "blank-bookmark", "backend": "Claude" }),
            ),
            (
                "agent.new",
                json!({ "cwd": "C:/x", "preset": "b", "backend": "Claude" }),
            ),
            ("agent.rename", json!({ "target": "ghost", "name": "beta" })),
            ("agent.rename", json!({ "target": "alpha", "name": "  " })),
            ("agent.move", json!({ "target": "ghost", "parent": null })),
            ("agent.move", json!({ "target": "alpha" })),
        ];
        for (name, args) in rejections {
            let err =
                call(&table, name, args.clone()).expect_err("must be a rejection: {name} {args}");
            assert_ne!(
                err.code(),
                ErrorCode::Internal,
                "{name}: if this branch is INTERNAL, the daemon reads it as may-have-been-applied — {args}"
            );
            assert_eq!(
                snapshot(),
                before,
                "{name}: the rejection changed the roster — {args}"
            );
        }
        // ★The same holds for changes the store did not accept★ — the manager's `try_` versions commit only if the save succeeds, so the daemon's premise that `CONFLICT` is
        //   「before touching anything」 stands as is.
        // ADR-0291 R17
        *host.store_refuses.lock().unwrap() = true;
        for (name, args) in [
            (
                "agent.new",
                json!({ "cwd": "C:/work/delta", "backend": "Claude" }),
            ),
            ("agent.rename", json!({ "target": "alpha", "name": "beta" })),
            ("agent.move", json!({ "target": "alpha", "parent": "lead" })),
        ] {
            let err = call(&table, name, args.clone())
                .expect_err("must be a save refusal: {name} {args}");
            assert_eq!(err.code(), ErrorCode::Conflict, "{name}: {args}");
            assert_eq!(
                snapshot(),
                before,
                "{name}: the save refusal changed the roster — {args}"
            );
        }
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "a rejection does not notify"
        );
    }

    /// ★A rename or move the store did not accept is `CONFLICT` with no notification★ — the name and hierarchy are unchanged, so there is no change to announce.
    ///   If a rename answered `Renamed`, the caller would type the next command with the changed name.
    // ADR-0291 R17 (D3)
    #[test]
    fn a_rename_or_move_the_store_refuses_is_a_conflict_that_announces_nothing() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        host.with_agent("lead", false, false);
        *host.store_refuses.lock().unwrap() = true;
        let (table, notify) = wiring(&host);

        let err = call(
            &table,
            "agent.rename",
            json!({ "target": "alpha", "name": "beta" }),
        )
        .expect_err("the rename comes back as a save refusal — not Renamed");
        assert_eq!(err.code(), ErrorCode::Conflict);
        assert!(
            err.message().contains("agents.json")
                && err.message().contains("the name was left as it is"),
            "{}",
            err.message()
        );

        let err = call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": "lead" }),
        )
        .expect_err("the move comes back as a save refusal");
        assert_eq!(err.code(), ErrorCode::Conflict);
        assert!(err.message().contains("agents.json"), "{}", err.message());

        assert_eq!(*notify.calls.lock().unwrap(), 0, "nothing changed");
    }

    /// ★Waking announces the roster even on a save refusal (E2)★ — the activation failure writes that entry's 「last failure」, and that axis
    ///   reaches the screen only through the roster notification (ADR-0172). The code is `CONFLICT`.
    // ADR-0291 R17 (E2)
    #[test]
    fn a_wake_the_store_refuses_is_a_conflict_that_still_announces_the_roster() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        *host.store_refuses.lock().unwrap() = true;
        let (table, notify) = wiring(&host);

        let err = call(&table, "agent.spawn", json!({ "target": "alpha" }))
            .expect_err("waking comes back as a save refusal");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert!(
            err.message().contains("could not start agent 'alpha'")
                && err.message().contains("agents.json"),
            "{}",
            err.message()
        );
        assert!(host.started.lock().unwrap().is_empty(), "nothing launched");
        assert_eq!(*notify.calls.lock().unwrap(), 1);
    }

    /// ★If the create of create-and-launch is refused, it does not reach the launch (§3-9 ⓐ)★ — roster unchanged · 0 launch attempts · 0 notifications.
    ///   Retrying creates no duplicate (the create is refused too).
    // ADR-0291 R17
    #[test]
    fn a_create_and_start_whose_create_the_store_refuses_starts_nothing_and_announces_nothing() {
        let host = FakeHost::new();
        *host.store_refuses.lock().unwrap() = true;
        let (table, notify) = wiring(&host);

        let err = call(
            &table,
            "agent.spawn",
            json!({ "cwd": "C:/work/gamma", "name": "gamma" }),
        )
        .expect_err("the create comes back as a save refusal");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert!(
            err.message().contains("nothing was registered"),
            "{}",
            err.message()
        );
        assert!(host.rows.lock().unwrap().is_empty(), "roster unchanged");
        assert!(host.started.lock().unwrap().is_empty(), "0 launch attempts");
        assert_eq!(*notify.calls.lock().unwrap(), 0);
    }

    /// ★Even if the store refuses the launch after creating, it is `INTERNAL` (§3-9 ⓑ)★ — the already-created agent remains, so if the daemon
    ///   read it as 「rejection before touching」 (`CONFLICT`) and let go of the request number, a retry with the same number would create again. The text carries the recovery path
    ///   (launch again by that name) as is, and the store's reason carries only the cause — if instructions like 「again a little later」 · 「retrying is useless」
    ///   mixed in, the caller would create again with the same `--cwd` or contradict the tail. Measured per refusal cause.
    // ADR-0291 R17
    #[test]
    fn a_created_agent_whose_start_the_store_refuses_stays_internal_with_the_recovery_hint() {
        // The create is saved and the launch's registration is refused — the shape where the file changed in between. The second field makes the refusal cause.
        struct RefuseOnStart(Arc<FakeHost>, fn() -> StoreError);
        impl AgentCommandHost for RefuseOnStart {
            fn roster(&self) -> Vec<AgentRosterRow> {
                self.0.roster()
            }
            fn store_status(&self) -> RosterStoreStatus {
                self.0.store_status()
            }
            fn agent_snapshot(&self, id: AgentId) -> Option<AgentProfile> {
                self.0.agent_snapshot(id)
            }
            fn create_agent(&self, profile: AgentProfile) -> Result<AgentProfile, PtyError> {
                let stored = self.0.create_agent(profile)?;
                *self.0.store_refuses.lock().unwrap() = true;
                Ok(stored)
            }
            fn activate_profile(
                &self,
                profile: &AgentProfile,
                mode: SpawnMode,
            ) -> Result<StartedAgent, PtyError> {
                if *self.0.store_refuses.lock().unwrap() {
                    return Err(PtyError::Store((self.1)()));
                }
                self.0.activate_profile(profile, mode)
            }
            fn rename_agent(
                &self,
                id: AgentId,
                display_name: Option<String>,
            ) -> Result<RenameOutcome, PtyError> {
                self.0.rename_agent(id, display_name)
            }
            fn reparent_agent(
                &self,
                child: AgentId,
                parent: Option<AgentId>,
            ) -> Result<bool, PtyError> {
                self.0.reparent_agent(child, parent)
            }
            fn preset_cwd(&self, id: &str) -> Option<String> {
                self.0.preset_cwd(id)
            }
            fn list_queued_inputs(&self, id: AgentId) -> Option<QueuedListing> {
                self.0.list_queued_inputs(id)
            }
            fn cancel_queued_input(
                &self,
                id: AgentId,
                input_id: &str,
            ) -> Result<CancelOutcome, CancelError> {
                self.0.cancel_queued_input(id, input_id)
            }
            fn interrupt_agent(&self, id: AgentId) -> Result<(), PtyError> {
                self.0.interrupt_agent(id)
            }
        }
        use crate::profile::Refusal;
        let causes: [(fn() -> StoreError, &str); 3] = [
            (
                || StoreError::Refused(Refusal::Unreadable),
                "agents.json could not be read just now",
            ),
            (
                || StoreError::Refused(Refusal::Newer { found: 2 }),
                "agents.json was written by a newer build (version 2)",
            ),
            (
                || StoreError::Io(std::io::Error::other("disk full")),
                "writing agents.json failed: disk full",
            ),
        ];

        for (refusal, cause) in causes {
            let host = FakeHost::new();
            let notify = Arc::new(FakeNotify::default());
            let table = make_table(
                Arc::new(RefuseOnStart(Arc::clone(&host), refusal)) as Arc<dyn AgentCommandHost>,
                Arc::clone(&notify) as Arc<dyn RosterChanged>,
                Arc::new(FakeUsage::default()) as Arc<dyn UsageCommandHost>,
            );

            let err = call(
                &table,
                "agent.spawn",
                json!({ "cwd": "C:/work/gamma", "name": "gamma" }),
            )
            .expect_err("the launch comes back as a save refusal");

            let message = err.message();
            assert_eq!(err.code(), ErrorCode::Internal, "{message}");
            assert!(
                message.contains("was created but did not start")
                    && message.contains(cause)
                    && message.contains(
                        "start it again by that name (in a moment) instead of creating another"
                    ),
                "{message}"
            );
            // Only the cause — the instruction of what to do is the one tail.
            assert!(
                !message.contains("retry in a moment")
                    && !message.contains("retrying will not help")
                    && !message.contains("restart the daemon"),
                "{message}"
            );
            assert_eq!(
                host.rows.lock().unwrap().len(),
                1,
                "the created agent remains"
            );
            assert!(host.started.lock().unwrap().is_empty());
            assert_eq!(
                *notify.calls.lock().unwrap(),
                2,
                "after creating · after activation (E2) — the current contract as is"
            );
        }
    }

    /// ★Through the bus with the real manager and a store that does not accept★ — the fake manager only imitates the `try_` versions, so whether the table really calls the manager's
    ///   caller entry (an old version would stand in memory and be `Ok`) is measured only with the real thing. Decision refusal = `CONFLICT` · write
    ///   failure = `INTERNAL`, and either way the roster is unchanged · nothing launched · the only notification is the one after waking's activation (E2).
    /// ★It does not launch a process★ — the launch stops at registration (①). Even if it regressed and passed through, it would be a shell calling a nonexistent program
    ///   and end in failure.
    // ADR-0291 R17 (D3 · E2)
    #[test]
    fn a_real_manager_behind_the_bus_changes_nothing_while_its_store_does_not_save() {
        use crate::preset::{Preset, PresetRegistry, PresetStore};
        use crate::profile::scripted_store::{Outcome, ScriptedStore, FAILURES};
        use crate::profile::ProfileRegistry;
        use crate::session_tracker::{SessionTracker, TrackerConfig};
        use crate::types::{AgentInfo, StatusSink};

        struct NoStatus;
        impl StatusSink for NoStatus {
            fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }
        struct NoPresets;
        impl PresetStore for NoPresets {
            fn save(&self, _presets: &[Preset]) -> Result<(), StoreError> {
                Ok(())
            }
            fn load(&self) -> Vec<Preset> {
                vec![]
            }
        }

        for failure in FAILURES {
            let expected = match failure {
                Outcome::FailsIo => ErrorCode::Internal,
                _ => ErrorCode::Conflict,
            };
            let store = ScriptedStore::new();
            let manager = Arc::new(AgentManager::new(
                Arc::new(NoStatus),
                Arc::new(ProfileRegistry::new(store.clone())),
                Arc::new(PresetRegistry::new(Arc::new(NoPresets))),
                Arc::new(SessionTracker::new(
                    TrackerConfig {
                        enabled: false,
                        poll_interval: std::time::Duration::from_secs(1),
                    },
                    Arc::new(|_, _| {}),
                )),
            ));
            let program = format!("engram-not-a-real-program-{}.exe", uuid::Uuid::new_v4());
            let shell = |name: &str| {
                let cwd = format!("C:/bus-store/{name}");
                let mut profile = AgentProfile::new(
                    cwd.clone(),
                    AgentCommand::Shell {
                        program: program.clone(),
                        args: vec![],
                    },
                    PathBuf::from(cwd),
                    vec![],
                    false,
                );
                profile.display_name = Some(name.to_string());
                profile
            };
            let alpha = manager
                .try_create_agent(shell("alpha"))
                .expect("register when saved");
            manager
                .try_create_agent(shell("lead"))
                .expect("register when saved");
            let roster = |m: &AgentManager| -> Vec<(AgentId, Option<String>, Option<AgentId>)> {
                let mut rows: Vec<_> = m
                    .agent_snapshots()
                    .into_iter()
                    .map(|p| (p.id, p.display_name, p.parent_id))
                    .collect();
                rows.sort();
                rows
            };
            let before = roster(&manager);
            store.set(failure);
            let notify = Arc::new(FakeNotify::default());
            let table = make_table(
                manager.clone() as Arc<dyn AgentCommandHost>,
                Arc::clone(&notify) as Arc<dyn RosterChanged>,
                Arc::new(FakeUsage::default()) as Arc<dyn UsageCommandHost>,
            );

            for (name, args) in [
                (
                    "agent.new",
                    json!({ "cwd": "C:/bus-store/delta", "backend": "Claude" }),
                ),
                ("agent.rename", json!({ "target": "alpha", "name": "beta" })),
                ("agent.move", json!({ "target": "alpha", "parent": "lead" })),
                ("agent.spawn", json!({ "cwd": "C:/bus-store/gamma" })),
                ("agent.spawn", json!({ "target": "alpha" })),
            ] {
                let err = call(&table, name, args.clone())
                    .expect_err("an error if the store does not accept: {name} {args}");
                assert_eq!(err.code(), expected, "{failure:?} · {name} {args}");
                assert!(
                    err.message().contains("agents.json"),
                    "{failure:?} · {name}: {}",
                    err.message()
                );
            }

            assert_eq!(roster(&manager), before, "{failure:?}: roster unchanged");
            assert!(
                manager.list_agents().is_empty(),
                "{failure:?}: nothing launched"
            );
            assert_eq!(
                *notify.calls.lock().unwrap(),
                1,
                "{failure:?}: only the notification after waking's activation (E2)"
            );
            assert_eq!(
                manager
                    .agent_snapshot(alpha.id)
                    .and_then(|p| p.last_failure),
                Some(crate::failure::AgentFailureKind::Other),
                "{failure:?}: recorded as 'other', not as a launch failure"
            );
        }
    }

    /// ★Pins **the value itself** of [`NEW_AGENT_OUTPUT_FORMAT`] as a literal★ — every place reading this constant is a
    /// relative comparison (`assert_eq!(…, NEW_AGENT_OUTPUT_FORMAT)`), so flipping the constant to `Terminal` makes those assertions
    /// move along and **the whole workspace stays green** (measured 2026-09-22 — there was not a single assertion in the repository measuring that value
    /// as a literal). A relative comparison measures only 「the two branches look at the same home」, not 「what is in that home」
    /// — the latter is the value the user sees (whether the screen right after creation is a chat or a terminal), so it is
    /// measured separately here. Changing the value is a decision, so an edit fixing this line must go through that decision (ADR-0078).
    #[test]
    fn the_new_agent_default_output_format_is_pinned_to_a_literal() {
        assert_eq!(NEW_AGENT_OUTPUT_FORMAT, CoreAgentOutputFormat::StreamJson);
    }

    /// The value list of that field the `agent.new` declaration advertised — **the list callers actually see**. If the test wrote this by hand,
    /// it would become a third list and create the very divergence it meant to measure.
    fn advertised(field: &str) -> Vec<String> {
        let schema: serde_json::Value =
            serde_json::from_str(AgentNewArgs::SPEC.args_schema).expect("args schema");
        // ★Both shapes are read★ — a required field spreads directly as `{"enum":[…]}`, and an omittable field is
        //   wrapped one layer as `{"anyOf":[{"enum":[…]},…]}`. Reading only one shape would make the sweep an **empty list** when that field
        //   switches between required and optional, silently measuring nothing (the caller's `is_empty` assertion is
        //   the last defense, but without folding here that assertion would blow up every time).
        let property = &schema["properties"][field];
        let branches = match property.get("anyOf") {
            Some(any_of) => any_of.as_array().expect("anyOf is an array").clone(),
            None => vec![property.clone()],
        };
        branches
            .iter()
            .filter_map(|branch| branch.get("enum"))
            .flat_map(|values| values.as_array().expect("enum array").clone())
            .map(|value| value.as_str().expect("string").to_string())
            .collect()
    }

    /// ★The place where **the compiler** binds the declaration vocabulary and the core vocabulary★ — binding them only in prose would leave the declaration side silently
    /// narrow when the core gains a variant, and that word would be rejected at the entry with `INVALID_ARGUMENT` (a missing feature
    /// with no error or log, so nobody notices).
    ///
    /// ★Half of what this test measures is **compilation**★ — the two `match`es below have no wildcard, so when the core enum
    /// gains a variant this file **does not compile**. So do not put `_ =>` when filling the branches: the moment you do,
    /// that half guards nothing.
    /// ★The other half comes from **the value list the declaration advertised**★ — writing words by hand would let an edit that adds a variant without putting it
    /// in that list silently bypass the spelling check. Drawing from the advertisement makes that impossible: when the core gains a variant
    /// the `match` breaks → a variant is added to the declaration → that variant enters the advertisement → this sweep covers it.
    #[test]
    fn the_declared_vocabularies_are_pinned_to_the_core_ones() {
        // ── Output format: declaration variants ↔ core variants are one-to-one, down to the wire notation ──
        fn declared_output_format(core: CoreAgentOutputFormat) -> AgentOutputFormat {
            match core {
                CoreAgentOutputFormat::Terminal => AgentOutputFormat::Terminal,
                CoreAgentOutputFormat::StreamJson => AgentOutputFormat::StreamJson,
            }
        }
        let formats = advertised("output_format");
        assert!(
            !formats.is_empty(),
            "if the advertisement is empty, this sweep is disarmed"
        );
        for word in &formats {
            let spelled = serde_json::Value::String(word.clone());
            // The same word must be read **on both sides** — if only one side reads it, that place is exactly the point of divergence.
            let declared: AgentOutputFormat = serde_json::from_value(spelled.clone())
                .expect("the declaration vocabulary reads the advertisement");
            let core: CoreAgentOutputFormat =
                serde_json::from_value(spelled).expect("the core vocabulary reads the same word");
            assert_eq!(
                declared_output_format(core),
                declared,
                "{word}: the two vocabularies read the same word as different variants"
            );
            assert_eq!(
                output_format(Some(declared)),
                core,
                "{word}: the declaration → core mapping must be one-to-one"
            );
        }

        // ── Backends: for each run command variant of the core, decide here **whether this entry can create it** ──
        fn declared_backend(command: &AgentCommand) -> Option<AgentBackend> {
            match command {
                AgentCommand::Claude { .. } => Some(AgentBackend::Claude),
                // ★`Shell` being `None` is a decision, not a blank★ — this entry cannot create it
                //   (it has not been confirmed that the profile creation path runs with it — the `AgentBackend` declaration).
                AgentCommand::Shell { .. } => None,
                // ★`Codex` opened on 2026-09-22★ — the old `None` reason (「the LLM cannot get past the trust confirmation modal of a never-seen folder」)
                //   was overturned by measurement (that table = the
                //   codex line of [`LLM_BACKEND_POLICY`] carries the grounds). What this line measures now is not that decision but **the vocabulary round trip**:
                //   whether the run command the declaration `Codex` builds reads back as `Codex`, and whether the stored `kind` tag has
                //   the same spelling.
                AgentCommand::Codex { .. } => Some(AgentBackend::Codex),
            }
        }
        let backends = advertised("backend");
        assert!(
            !backends.is_empty(),
            "if the advertisement is empty, this sweep is disarmed"
        );
        for word in &backends {
            let declared: AgentBackend =
                serde_json::from_value(serde_json::Value::String(word.clone()))
                    .expect("the declaration vocabulary reads the advertisement");
            let built = backend_command(declared.clone(), NEW_AGENT_OUTPUT_FORMAT);
            assert_eq!(
                declared_backend(&built),
                Some(declared),
                "{word}: the run command the declaration vocabulary builds does not read back into that vocabulary"
            );
            // ★Even the core-side wire spelling is bound★: `AgentCommand` is written to disk as is with `#[serde(tag = "kind")]`
            //   (`agents.json`), so changing that tag value must be caught here — if not, the declaration vocabulary and
            //   the stored vocabulary silently diverge, and that divergence shows up as old builds unable to read the file.
            assert_eq!(
                serde_json::to_value(&built).expect("core serialization")["kind"],
                serde_json::Value::String(word.clone()),
                "{word}: the declaration vocabulary and the stored kind tag diverged"
            );
        }
    }

    /// ★The values passing today are the two `Claude`·`Codex`, and **each of them works in both modes**★ (2026-09-22).
    ///
    /// The core also has `Shell`, but it has not been confirmed that the creation path runs with it, so it is outside the vocabulary (reason =
    /// the `AgentBackend` declaration).
    ///
    /// ★Why the mode axis is measured here too★ — opening codex is not one word but **word × two modes**, and
    /// whether the mode was actually carried is visible only by looking into **the stored run command** (`AgentNewOk` has no such field).
    /// Without measuring here, the whole suite would be green even if [`backend_command`] pinned codex to one mode.
    #[test]
    fn new_accepts_both_declared_backends_and_refuses_the_rest() {
        for word in ["Claude", "Codex"] {
            for (spelled, expected) in [
                ("Terminal", CoreAgentOutputFormat::Terminal),
                ("StreamJson", CoreAgentOutputFormat::StreamJson),
            ] {
                let host = FakeHost::new();
                let (table, _notify) = wiring(&host);

                call(
                    &table,
                    "agent.new",
                    json!({ "cwd": "C:/x", "backend": word, "output_format": spelled }),
                )
                .unwrap_or_else(|e| {
                    panic!(
                        "{word}/{spelled}: the declaration vocabulary was rejected — {}",
                        e.message()
                    )
                });

                let profiles = host.profiles.lock().unwrap();
                let stored = profiles.values().next().expect("one registered profile");
                let (kind, format) = match &stored.command {
                    AgentCommand::Claude {
                        extra_args,
                        output_format,
                    } => ("Claude", (extra_args.clone(), *output_format)),
                    AgentCommand::Codex {
                        extra_args,
                        output_format,
                    } => ("Codex", (extra_args.clone(), *output_format)),
                    other => panic!("{word}/{spelled}: the wrong run command — {other:?}"),
                };
                assert_eq!(
                    kind, word,
                    "{word}/{spelled}: a different backend was stored"
                );
                // ★This entry does not grow the argument surface★ — if fields like model or reasoning effort leaked in here, the assertion that it is empty
                //   breaks first.
                assert!(
                    format.0.is_empty(),
                    "{word}/{spelled}: this entry carries no extra_args — {:?}",
                    format.0
                );
                assert_eq!(
                    format.1, expected,
                    "{word}/{spelled}: the mode was not carried"
                );
            }
        }

        // ★Outside the vocabulary is rejected before registration — but **spelling is not the outside-vocabulary axis**★: a word differing only in case
        //   reads as the same word (`new_accepts_the_backend_word_in_any_casing`).
        let host = FakeHost::new();
        let (table, notify) = wiring(&host);
        for outside in ["Gemini", "Shell", "codx"] {
            let err = call(
                &table,
                "agent.new",
                json!({ "cwd": "C:/x", "backend": outside }),
            )
            .expect_err("a backend outside the vocabulary");
            assert_eq!(err.code(), ErrorCode::InvalidArgument, "{outside}");
            // It must say **everything** that works — saying only one would make the guidance fall behind the day the vocabulary widens.
            for accepted in ["Claude", "Codex"] {
                assert!(
                    err.message().contains(accepted),
                    "{outside}: does not say the working word '{accepted}' — {}",
                    err.message()
                );
            }
        }
        assert!(
            host.rows.lock().unwrap().is_empty(),
            "a rejection does not register"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "a rejection does not notify"
        );
    }

    /// ★Not giving the field at all is a rejection — it does not silently become claude★ (2026-09-08).
    ///
    /// Unspecified used to mean `Claude`. Since there came to be two backends, that default folds 「did not choose」 and 「chose claude」
    /// into the same value, and it was already removed from the spawn packet for the same reason (the daemon's `MISSING_BACKEND`).
    /// Leaving only this door would split that decision per entry.
    #[test]
    fn new_refuses_an_unspecified_backend_instead_of_defaulting() {
        let host = FakeHost::new();
        let (table, notify) = wiring(&host);

        let err = call(&table, "agent.new", json!({ "cwd": "C:/x" }))
            .expect_err("not choosing is not choosing claude");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("backend"),
            "must say which field is empty: {}",
            err.message()
        );
        assert!(
            host.rows.lock().unwrap().is_empty(),
            "a rejection does not register"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "a rejection does not notify"
        );
    }

    /// ★This door opens **the same set** as [`LLM_BACKEND_POLICY`] — measured by **calls**, not by comparing lists★.
    ///
    /// Matching only the lists stays green even if the gate is removed entirely (it actually happened at a sibling door — 2026-09-08 review).
    /// So for each word of the table, `agent.new` is **called** and the outcome is checked: words the table opened go onto the roster, and words the table closed
    /// do not go on **in any spelling**.
    ///
    /// ★The spellings this door accepts are decided by the declaration vocabulary★ — they differ from the table's words only in case (the `word` doc of
    /// [`LlmBackendPolicy`]). So for one word, **every spelling this door could accept** is tried. Trying only one spelling
    /// would make that attempt hit the wrong spelling and silently pass the day the vocabulary widens.
    ///
    /// ★What it catches — written precisely (updated 2026-09-22: with the table closing nothing, the weight of the two directions split)★
    ///   - **the table closed but this door still creates** → turns red. An edit that widens the vocabulary ([`AgentBackend`]) while
    ///     deleting only the [`llm_creation_refusal`] arm of `verb_new` is caught here. This is the only regression net of that
    ///     arm.
    ///   - **the table opened but this door does not create** → turns red. An edit that adds only one line to the table without filling the vocabulary and [`backend_command`]
    ///     is caught here (the [`LLM_BACKEND_POLICY`] doc gives that guidance).
    ///
    /// ★**Deleting only the arm** cannot be caught by this test even today★ — but the reason changed: before, it was 「because the vocabulary was narrower than the table」,
    /// and now it is 「because the table closes nothing, no input reaches the arm at all」. Do not read that fact as 「so the arm is unnecessary」
    /// — the day the table closes a line again, that arm becomes the only runtime defense.
    #[test]
    fn new_creates_exactly_what_the_llm_backend_policy_opens() {
        /// Spelling candidates this door can accept for this word — the table is lowercase, the declaration vocabulary PascalCase.
        fn spellings(word: &str) -> Vec<String> {
            let mut chars = word.chars();
            let pascal = match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            };
            if pascal == word {
                vec![word.to_string()]
            } else {
                vec![word.to_string(), pascal]
            }
        }

        assert!(
            !LLM_BACKEND_POLICY.is_empty(),
            "if the table is empty, this sweep is disarmed"
        );
        for policy in LLM_BACKEND_POLICY {
            let mut opened_by_some_spelling = false;
            for spelling in spellings(policy.word) {
                let host = FakeHost::new();
                let (table, notify) = wiring(&host);
                let outcome = call(
                    &table,
                    "agent.new",
                    json!({ "cwd": "C:/x", "backend": spelling }),
                );
                match outcome {
                    Ok(_) => {
                        opened_by_some_spelling = true;
                        assert_eq!(
                            host.rows.lock().unwrap().len(),
                            1,
                            "{spelling}: passed but did not go onto the roster"
                        );
                    }
                    Err(err) => {
                        assert_eq!(err.code(), ErrorCode::InvalidArgument, "{spelling}");
                        assert!(
                            host.rows.lock().unwrap().is_empty(),
                            "{spelling}: ★refused before registration★"
                        );
                        assert_eq!(*notify.calls.lock().unwrap(), 0, "{spelling}");
                    }
                }
            }
            assert_eq!(
                opened_by_some_spelling,
                policy.refusal.is_none(),
                "'{}': the set this door opens and the policy table diverged — the table {}, this door {}",
                policy.word,
                if policy.refusal.is_none() {
                    "opens"
                } else {
                    "closes"
                },
                if opened_by_some_spelling {
                    "creates"
                } else {
                    "does not create"
                }
            );
        }
    }

    /// ★The backend words `agent.new` **advertises** == the set the policy table opens★ — spellings differ only in case
    /// (declaration vocabulary `Claude` · table `claude`).
    ///
    /// `new_creates_exactly_what_the_llm_backend_policy_opens` sweeps **the table's lines**, so it cannot see a word only in the advertisement
    /// — if a variant is added to the vocabulary without adding a line to the table, that word is advertised but
    /// closed by [`NO_POLICY_DECLARED`], and the caller gets rejected for a value it was told it could choose. This test catches that
    /// direction.
    #[test]
    fn new_advertises_exactly_the_backends_the_llm_policy_opens() {
        let mut opened: Vec<String> = LLM_BACKEND_POLICY
            .iter()
            .filter(|policy| policy.refusal.is_none())
            .map(|policy| policy.word.to_ascii_lowercase())
            .collect();
        let mut advertised: Vec<String> = advertised("backend")
            .iter()
            .map(|word| word.to_ascii_lowercase())
            .collect();
        assert!(
            !advertised.is_empty(),
            "if the advertisement is empty, this test is disarmed"
        );
        opened.sort();
        advertised.sort();
        assert_eq!(
            advertised, opened,
            "the `AgentBackend` vocabulary of `agent.new` and the set `LLM_BACKEND_POLICY` opens diverged"
        );
    }

    #[test]
    fn rename_splits_renamed_from_unchanged() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        *host.rename_result.lock().unwrap() = Some(RenameOutcome::Unchanged("alpha".to_string()));
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.rename",
            json!({ "target": "alpha", "name": "alpha" }),
        )
        .expect("rename");
        assert_eq!(out["outcome"], "unchanged");

        let out = call(
            &table,
            "agent.rename",
            json!({ "target": "alpha", "name": "beta" }),
        )
        .expect("rename");
        assert_eq!(out["outcome"], "renamed");
        assert_eq!(out["name"], "beta");
        assert_eq!(
            *notify.calls.lock().unwrap(),
            2,
            "both outcomes change the roster display, so they announce"
        );
    }

    #[test]
    fn rename_exhaustion_is_a_conflict() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        *host.rename_result.lock().unwrap() = Some(RenameOutcome::Exhausted);
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.rename",
            json!({ "target": "alpha", "name": "beta" }),
        )
        .expect_err("name space exhausted");
        assert_eq!(err.code(), ErrorCode::Conflict);
    }

    /// ★`null` is an active instruction to 「detach to root」, and **absence is rejected**★ — folding them would make one typo field
    /// silently run a hierarchy release.
    #[test]
    fn move_splits_an_absent_parent_from_an_explicit_null() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": null }),
        )
        .expect("null = to top level");
        assert_eq!(out["parent"], serde_json::Value::Null);
        assert_eq!(
            out["name"], "alpha",
            "the success response carries the name"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            1,
            "announces the roster change"
        );

        let err = call(&table, "agent.move", json!({ "target": "alpha" }))
            .expect_err("absence is rejected");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("parent"),
            "says which field is missing: {}",
            err.message()
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            1,
            "a rejection does not notify"
        );
    }

    #[test]
    fn a_structural_rejection_is_conflict() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        host.with_agent("beta", false, false);
        *host.reparent_ok.lock().unwrap() = false;
        let (table, notify) = wiring(&host);

        let err = call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": "beta" }),
        )
        .expect_err("the tree structure refuses");
        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(*notify.calls.lock().unwrap(), 0);
    }

    /// ★A single `false` is not read as a reason★ — vanished in the meantime and a structural conflict have different things for the caller to do.
    #[test]
    fn a_target_that_vanished_mid_move_is_not_found() {
        let host = FakeHost::new();
        let alpha = host.with_agent("alpha", false, false);
        *host.reparent_ok.lock().unwrap() = false;
        *host.vanish_on_reparent.lock().unwrap() = Some(alpha);
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": null }),
        )
        .expect_err("the target vanished");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(err.message().contains("alpha"), "{}", err.message());
    }

    /// ★Name the side that vanished★ — if only the parent disappeared but the child's name were given, the caller would hear that an agent that is perfectly there
    /// is missing and search in the wrong place.
    #[test]
    fn a_parent_that_vanished_mid_move_names_the_parent_not_the_child() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let beta = host.with_agent("beta", false, false);
        *host.reparent_ok.lock().unwrap() = false;
        *host.vanish_on_reparent.lock().unwrap() = Some(beta);
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": "beta" }),
        )
        .expect_err("the new parent vanished");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(
            err.message().contains("beta") && !err.message().contains("alpha"),
            "must point at the side that vanished: {}",
            err.message()
        );
    }

    #[test]
    fn unknown_target_is_not_found_and_ambiguous_target_is_conflict() {
        let host = FakeHost::new();
        host.with_agent("twin", false, false);
        host.with_agent("twin", false, false);
        let (table, _notify) = wiring(&host);

        let missing =
            call(&table, "agent.spawn", json!({ "target": "nope" })).expect_err("a missing name");
        assert_eq!(missing.code(), ErrorCode::NotFound);

        let ambiguous = call(&table, "agent.spawn", json!({ "target": "twin" }))
            .expect_err("two with the same name");
        assert_eq!(ambiguous.code(), ErrorCode::Conflict);
    }

    #[test]
    fn an_id_is_matched_before_a_name() {
        let host = FakeHost::new();
        let id = host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        call(&table, "agent.spawn", json!({ "target": id.to_string() })).expect("id pointing");
        assert_eq!(host.started.lock().unwrap()[0].0, id);
    }

    #[test]
    fn a_blank_value_is_refused_not_treated_as_absent() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": "   " }),
        )
        .expect_err("a whitespace-only value");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        // ★For the same field, whichever mistake was made, the same branch is told★: if the blank-value text hid the `null` (detach to top level)
        //   the absence text tells about, the caller would read that putting a value into that field is the only way.
        assert!(
            err.message().contains("null"),
            "must tell the null branch for a blank value too: {}",
            err.message()
        );
    }

    /// ★Doing as the rejection text says must work★: telling a caller who sent a required field as a blank value "may be omitted"
    /// would make the retry that omits it as is **rejected again** (this time as 「missing」) — the LLM follows the text as is,
    /// so it circles the same place. For a required field it says only to fill in a value.
    #[test]
    fn the_advice_for_a_blank_required_field_does_not_tell_the_caller_to_drop_it() {
        let host = FakeHost::new();
        let (table, notify) = wiring(&host);

        let required = call(
            &table,
            "agent.new",
            json!({ "cwd": "   ", "backend": "Claude" }),
        )
        .expect_err("the field deciding where to create is blank")
        .message()
        .to_string();
        assert!(
            required.contains("cwd needs a folder path") && required.contains("preset"),
            "must say to fill in a value or use the sibling field: {required}"
        );
        assert!(
            !required.contains("left out"),
            "saying only to omit it gets that retry rejected again as one-of-the-two-must-be-given: {required}"
        );
        // ★A rejection creates nothing★ — if the check were pushed after registration, a call with blank arguments would create an agent and then
        //   answer with a rejection (the caller reads it as a failure and calls again — only the names grow one by one).
        assert!(
            host.rows.lock().unwrap().is_empty(),
            "a rejection does not register"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "a rejection does not notify"
        );

        // An optional field is the opposite — omitting it is the way that actually works, so that way is told too.
        let optional = call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/x", "name": " ", "backend": "Claude" }),
        )
        .expect_err("an optional field is blank")
        .message()
        .to_string();
        assert!(
            optional.contains("name") && optional.contains("left out"),
            "saying an optional field may be omitted is true: {optional}"
        );
        assert!(
            host.rows.lock().unwrap().is_empty(),
            "a rejection does not register"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "a rejection does not notify"
        );

        // The retry that omitted it as told actually works.
        call(
            &table,
            "agent.new",
            json!({ "cwd": "C:/x", "backend": "Claude" }),
        )
        .expect("following the omit guidance works");
    }

    /// ★Both fields of `rename` are **required**★ — telling either 「may be omitted」 would make the retry that omits it as is rejected again at the entry
    /// check with 「requires …」, and the caller would circle the same place.
    ///
    /// ★A property to be looked at **per call site**, not per field★: whether blank-value guidance is right or wrong depends on what that field is
    /// required to be in that verb, so checking the shared function once guards no verb.
    #[test]
    fn a_blank_rename_argument_is_told_to_fill_it_not_to_drop_it() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, notify) = wiring(&host);

        for (args, blank) in [
            (json!({ "target": "alpha", "name": "  " }), "name"),
            (json!({ "target": " ", "name": "beta" }), "target"),
        ] {
            let refusal = call(&table, "agent.rename", args.clone())
                .expect_err("blank value")
                .message()
                .to_string();

            assert!(
                refusal.contains(&format!("{blank} needs a real value")),
                "must say to fill in {blank}: {refusal}"
            );
            assert!(
                !refusal.contains("left out"),
                "saying to omit {blank} gets that retry rejected again with requires: {refusal}"
            );
            assert_eq!(
                *notify.calls.lock().unwrap(),
                0,
                "a rejection does not notify"
            );
        }

        // The retry that filled both fields as the text said works.
        let out = call(
            &table,
            "agent.rename",
            json!({ "target": "alpha", "name": "beta" }),
        )
        .expect("filling in as guided works");
        assert_eq!(out["name"], "beta", "{out}");
    }

    /// ★The constraint of `spawn` is not a field but **a combination**★: `cwd` is `Option` by declaration, so 「may be omitted」 is true by the declaration,
    /// but the retry that omits it as is gets rejected again with 「either target or cwd must be given」.
    /// The text must state the combination to get out in one go.
    #[test]
    fn a_blank_spawn_argument_is_told_what_actually_satisfies_the_verb() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, notify) = wiring(&host);
        let before = host.rows.lock().unwrap().len();

        let refusal = call(&table, "agent.spawn", json!({ "cwd": "  " }))
            .expect_err("blank cwd")
            .message()
            .to_string();

        assert!(
            refusal.contains("cwd"),
            "must point out which field: {refusal}"
        );
        assert!(
            !refusal.contains("left out"),
            "saying to omit it gets that retry rejected again: {refusal}"
        );
        assert!(
            refusal.contains("target") && refusal.contains("cwd"),
            "must say what to give for it to work (one of the two): {refusal}"
        );
        // ★In this verb the rejection must happen **before doing anything irreversible**★: if the blank-value check were pushed after the combination branch,
        //   a call with a blank `cwd` would **create an agent and even notify** and then answer with a rejection. The caller
        //   reads it as a failure and calls again, so only the names grow one by one, and what was created has no way to be deleted (ADR-0122).
        assert_eq!(
            host.rows.lock().unwrap().len(),
            before,
            "the rejection created an agent"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "a rejection does not notify"
        );

        // **Both** ways the text points at actually work.
        call(&table, "agent.spawn", json!({ "target": "alpha" })).expect("the wake way");
        call(&table, "agent.spawn", json!({ "cwd": "C:/work/new" })).expect("the create way");
    }

    /// ★`move` points out the two blank fields **at once**★ — rejecting only `parent` first would make the caller hear about `target` only on the next trip after fixing it
    /// (two round trips for one mistyped call). And the `null` branch of `parent` still remains.
    #[test]
    fn a_move_with_two_blank_fields_names_both_in_one_trip() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        let refusal = call(
            &table,
            "agent.move",
            json!({ "target": " ", "parent": "  " }),
        )
        .expect_err("both blank")
        .message()
        .to_string();

        assert!(
            refusal.contains("target"),
            "the sibling field was hidden: {refusal}"
        );
        assert!(refusal.contains("parent"), "{refusal}");
        assert!(
            refusal.contains("null"),
            "points out at once without losing the null branch: {refusal}"
        );

        // The retry fixed as the text said works.
        call(
            &table,
            "agent.move",
            json!({ "target": "alpha", "parent": null }),
        )
        .expect("fixing as guided works");
    }

    /// ★With several blank fields, all of them come out★ — pointing one at a time would make a caller who sent three unset shell variables repeat rejection and fix
    /// three times (the same discipline as the entry's unknown-field check).
    #[test]
    fn every_blank_field_is_named_not_just_the_first() {
        let (table, _notify) = wiring(&FakeHost::new());

        let err = call(
            &table,
            "agent.spawn",
            json!({ "target": " ", "cwd": "  ", "name": "\t" }),
        )
        .expect_err("all blank");

        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        for field in ["target", "cwd", "name"] {
            assert!(
                err.message().contains(field),
                "{field} is missing: {}",
                err.message()
            );
        }
    }

    // ── State honesty ──────────────────────────────────────────────────────────────

    /// ★The change verbs and `list` must pass through the same function so the two answers cannot diverge★ — every state is pinned here.
    ///
    /// ★Known blind spot (not fixed)★: the list below is **written by hand**, and `state_of` emits asleep through a catch-all branch (`_ =>`).
    /// So even when a new variant appears this test is green, and if that variant is 「of a live nature」, it silently
    /// goes out as asleep. Filling the list automatically would require changing the shape on the `AgentStatus` side, so it is not done here.
    #[test]
    fn every_status_maps_to_one_of_the_two_agent_state_words() {
        assert_eq!(
            state_of(None),
            AGENT_STATE_SLEEPING,
            "no session means asleep"
        );
        assert_eq!(state_of(Some(&AgentStatus::Running)), AGENT_STATE_LIVE);
        assert_eq!(
            state_of(Some(&AgentStatus::Exiting)),
            AGENT_STATE_LIVE,
            "going down is still alive"
        );
        for dead in [
            AgentStatus::Exited { code: Some(0) },
            AgentStatus::Exited { code: None },
            AgentStatus::Killed,
            AgentStatus::Failed {
                message: "boom".to_string(),
            },
        ] {
            assert_eq!(
                state_of(Some(&dead)),
                AGENT_STATE_SLEEPING,
                "a corpse is not alive: {dead:?}"
            );
        }
        // Not mixed with the message outcome vocabulary (mixing the two axes once tangled decisions — ADR-0116).
        for message_word in ["delivered", "pending", "failed"] {
            assert_ne!(AGENT_STATE_LIVE, message_word);
            assert_ne!(AGENT_STATE_SLEEPING, message_word);
        }
    }

    /// ★The state of the response is **derived**, not a literal★: if `spawn` answered that an agent that died right after launching (ADR-0082 resume early exit —
    /// there is no fresh fallback) is alive, the `list` right after would answer that it is asleep. The caller
    /// would write mail to a corpse in between. Baking `"live"` into `started_payload` dies here.
    #[test]
    fn a_spawn_whose_activation_lands_terminal_reports_sleeping() {
        for terminal in [
            AgentStatus::Exited { code: Some(1) },
            AgentStatus::Killed,
            AgentStatus::Failed {
                message: "resume died".to_string(),
            },
        ] {
            let host = FakeHost::new();
            host.with_agent("alpha", false, true);
            *host.started_status.lock().unwrap() = Some(terminal.clone());
            let (table, _notify) = wiring(&host);

            let woken = call(&table, "agent.spawn", json!({ "target": "alpha" })).expect("wake");
            assert_eq!(
                woken["state"], AGENT_STATE_SLEEPING,
                "something that died right after waking must not be reported as alive: {terminal:?}"
            );
            assert_eq!(woken["created"], false);

            let born = call(&table, "agent.spawn", json!({ "cwd": "C:/work/beta" }))
                .expect("create and launch");
            assert_eq!(
                born["state"], AGENT_STATE_SLEEPING,
                "the create path uses the same derivation: {terminal:?}"
            );
            assert_eq!(born["created"], true);
        }
    }

    /// Terminal states held in the roster come out as the same words — the other half of the axis that change verbs and queries pass through one function.
    #[test]
    fn the_roster_reports_a_terminal_status_as_sleeping() {
        let host = FakeHost::new();
        host.with_status(
            "zombie",
            Some(AgentStatus::Failed {
                message: "died".to_string(),
            }),
            false,
        );
        host.with_status("alive", Some(AgentStatus::Running), false);
        let (table, _notify) = wiring(&host);

        let out = call(&table, "agent.list", json!({})).expect("query");
        let agents = out["agents"].as_array().expect("array");
        assert_eq!(agents[0]["state"], AGENT_STATE_SLEEPING, "{out}");
        assert_eq!(agents[1]["state"], AGENT_STATE_LIVE, "{out}");
    }

    /// ★The name of the move response is the value **after** applying★ — carrying the resolve-time value would, because of a rename committed in between, make the response
    /// 「old name + new parent」, a combination that never existed, and the caller would type the next command with that old name.
    #[test]
    fn move_reports_the_name_the_agent_has_after_the_move() {
        let host = FakeHost::new();
        let helper = host.with_agent("helper", false, false);
        let lead = host.with_agent("lead", false, false);
        // ★The old parent is planted★: it is the only way to tell whether the response's `parent` is **the parent this call instructed** or the old value
        //   left in the roster (if neither existed, either would carry the same value and they could not be told apart).
        //   ★The fake manager does not fix the parent field★ — so this old value stays in the roster as is.
        let stale = AgentId::new_v4();
        for row in host.rows.lock().unwrap().iter_mut() {
            if row.id == helper {
                row.parent = Some(stale);
            }
        }
        *host.rename_on_reparent.lock().unwrap() = Some("renamed".to_string());
        let (table, _notify) = wiring(&host);

        let out = call(
            &table,
            "agent.move",
            json!({ "target": "helper", "parent": "lead" }),
        )
        .expect("move");
        assert_eq!(out["name"], "renamed", "{out}");
        assert_eq!(
            out["parent"],
            lead.to_string(),
            "carries the parent this call attached (not the old parent left in the roster): {out}"
        );
    }

    /// ★A target that was applied but vanished in the meantime★ — the response must not become **a nameless success row**. Carrying an empty name would
    /// make the caller's decider read the success response as "an unreadable response", and a move that actually happened would be recorded as a failure.
    ///
    /// ★Resolution must succeed to reach this branch★: emptying the roster beforehand would have `resolve` emit `NOT_FOUND` first and this code
    /// would not even run — so the vanishing is caused **at apply time** (`vanish_on_reparent`).
    #[test]
    fn move_falls_back_to_the_resolved_name_when_the_agent_vanishes_after_the_move() {
        let host = FakeHost::new();
        let helper = host.with_agent("helper", false, false);
        // The apply succeeds (reparent_ok = true) but within that call it vanishes from the roster.
        *host.vanish_on_reparent.lock().unwrap() = Some(helper);
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.move",
            json!({ "target": "helper", "parent": null }),
        )
        .expect("the apply itself succeeded");

        // ★Without this assertion the test does not exercise the fallback★: if the row remained, `current_name` would give a value and
        //   the name assertion below would pass as is — that is, green even if the seam did not fire.
        assert!(
            host.roster().iter().all(|row| row.id != helper),
            "premise: it must have vanished from the roster during the apply for the fallback to run"
        );
        assert_eq!(
            out["name"], "helper",
            "if it cannot be read after applying, fall back to the resolve-time name (no empty name): {out}"
        );
        assert_eq!(out["agent_id"], helper.to_string(), "{out}");
        assert!(
            out["name"].as_str().is_some_and(|n| !n.is_empty()),
            "a nameless row must not be carried in a success response: {out}"
        );
        // Even if the target vanished, **the parent axis carries the value this call instructed** — if a request told to detach succeeded but
        //   the `parent` field were empty or gone, the caller could not read what was applied.
        //   (The defect of carrying the old parent splits only while the roster remains, so the neighboring test below looks at it.)
        assert!(
            out["parent"].is_null(),
            "it was told to detach, so null: {out}"
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            1,
            "it was applied, so it announces"
        );
    }

    // ── Pointing resolution (the function the cross-check runs) ────────────────────────────────────────

    /// ★This is the entry point the daemon's cross-check test runs★ — that this is the same function as the resolver the table's verbs use is
    /// the premise of that check (`resolve` delegates to this function).
    #[test]
    fn resolve_in_matches_ids_first_then_exact_names_and_refuses_duplicates() {
        let host = FakeHost::new();
        let alpha = host.with_agent("alpha", false, false);
        let twin_a = host.with_agent("twin", false, false);
        let _twin_b = host.with_agent("twin", false, false);
        let roster = host.roster();

        assert_eq!(resolve_in(&roster, "alpha").expect("name match").id, alpha);
        assert_eq!(
            resolve_in(&roster, &twin_a.to_string())
                .expect("id match")
                .id,
            twin_a,
            "id pointing works even with the same name"
        );
        assert_eq!(
            resolve_in(&roster, "twin")
                .expect_err("two with the same name")
                .code(),
            ErrorCode::Conflict
        );
        for miss in ["ALPHA", "alph", " alpha", "alpha "] {
            assert_eq!(
                resolve_in(&roster, miss)
                    .expect_err("exact match only")
                    .code(),
                ErrorCode::NotFound,
                "{miss}"
            );
        }
    }

    /// ★Ids are accepted in every spelling `Uuid` reads — before names★: even if another agent carries the uppercase spelling as its name, the id
    /// wins. An id-shaped value not in the roster falls through to the name and is absent.
    #[test]
    fn resolve_in_reads_every_uuid_spelling_as_the_id_before_names() {
        let alpha = AgentId::new_v4();
        let upper = alpha.to_string().to_uppercase();
        let row = |id: AgentId, name: &str| AgentRosterRow {
            id,
            canonical_name: name.to_string(),
            cwd: String::new(),
            parent: None,
            live: None,
        };
        let roster = vec![row(alpha, "alpha"), row(AgentId::new_v4(), &upper)];

        for spelling in [
            upper.clone(),
            format!("{{{alpha}}}"),
            format!("{{{upper}}}"),
            alpha.simple().to_string(),
            alpha.simple().to_string().to_uppercase(),
            alpha.urn().to_string(),
        ] {
            assert_eq!(
                resolve_in(&roster, &spelling).expect("id spelling").id,
                alpha,
                "{spelling}"
            );
        }
        let ghost = AgentId::new_v4().to_string().to_uppercase();
        assert_eq!(
            resolve_in(&roster, &ghost)
                .expect_err("an id not in the roster")
                .code(),
            ErrorCode::NotFound
        );
        assert_eq!(
            resolve_in(&roster, &format!(" {alpha}"))
                .expect_err("no whitespace correction")
                .code(),
            ErrorCode::NotFound
        );
    }

    // ── Candidates of pointing failures (TRD S28 §6) ───────────────────────────────────────────────

    /// Plants name lines into the roster as is — candidates look only at roster rows, so no profile is needed.
    fn roster_of(names: &[&str]) -> Vec<AgentRosterRow> {
        names
            .iter()
            .map(|name| AgentRosterRow {
                id: AgentId::new_v4(),
                canonical_name: name.to_string(),
                cwd: format!("C:/work/{name}"),
                parent: None,
                live: None,
            })
            .collect()
    }

    // ADR-0307
    #[test]
    fn a_missing_name_lists_the_names_that_contain_it_ignoring_case() {
        let roster = roster_of(&["alpha-main", "beta", "ALPHA-two", "gamma"]);

        let err = resolve_in(&roster, "alpha").expect_err("there is no exact match");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(
            err.message().starts_with(&not_found_message("alpha")),
            "the old text stays in front as is: {}",
            err.message()
        );
        // In name order (byte order — uppercase first).
        assert!(
            err.message()
                .ends_with(" — candidates: ALPHA-two, alpha-main"),
            "{}",
            err.message()
        );
    }

    /// Without containing names, the first five of all in name order — the same name once.
    // ADR-0307
    #[test]
    fn with_no_containing_name_the_first_five_distinct_names_by_name_are_listed() {
        let roster = roster_of(&["f", "b", "d", "b", "a", "e", "c"]);

        let err = resolve_in(&roster, "zzz").expect_err("absent");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(
            err.message().ends_with(" — candidates: a, b, c, d, e"),
            "{}",
            err.message()
        );
    }

    /// ★The order of roster rows does not change the candidates★ — asleep rows of the production roster come in HashMap order.
    // ADR-0307
    #[test]
    fn candidates_do_not_depend_on_roster_row_order() {
        let forward = roster_of(&["g", "f", "e", "d", "c", "b", "a", "twin", "twin"]);
        let mut backward: Vec<AgentRosterRow> = forward
            .iter()
            .map(|row| AgentRosterRow {
                id: row.id,
                canonical_name: row.canonical_name.clone(),
                cwd: row.cwd.clone(),
                parent: None,
                live: None,
            })
            .collect();
        backward.reverse();

        for token in ["zzz", "twin"] {
            assert_eq!(
                resolve_in(&forward, token).expect_err("failure").message(),
                resolve_in(&backward, token).expect_err("failure").message(),
                "{token}"
            );
        }
    }

    /// An empty roster gives `none` — without the tail, 「no candidates」 and 「a text that carries no candidates」 look the same.
    #[test]
    fn an_empty_roster_says_there_are_no_candidates() {
        let err = resolve_in(&[], "alpha").expect_err("absent");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert_eq!(
            err.message(),
            format!("{} — candidates: none", not_found_message("alpha"))
        );
    }

    /// Candidates for the same name are ids, not names — giving the name again would be the same CONFLICT again.
    // ADR-0307
    #[test]
    fn a_duplicate_name_lists_the_clashing_ids_up_to_five() {
        let roster = roster_of(&["other", "twin", "twin"]);
        let err = resolve_in(&roster, "twin").expect_err("two with the same name");
        assert_eq!(err.code(), ErrorCode::Conflict);
        assert!(
            err.message()
                .starts_with("more than one agent is called 'twin'"),
            "{}",
            err.message()
        );
        let mut pair = [roster[1].id.to_string(), roster[2].id.to_string()];
        pair.sort();
        assert!(
            err.message()
                .ends_with(&format!(" — candidates: {}, {}", pair[0], pair[1])),
            "{}",
            err.message()
        );

        let crowd = roster_of(&["twin"; 7]);
        let err = resolve_in(&crowd, "twin").expect_err("seven with the same name");
        let mut ids: Vec<String> = crowd.iter().map(|row| row.id.to_string()).collect();
        ids.sort();
        ids.truncate(5);
        assert!(
            err.message()
                .ends_with(&format!(" — candidates: {}", ids.join(", "))),
            "{}",
            err.message()
        );
    }

    /// Candidates come from the one resolver, so every verb taking `target` receives them together.
    #[test]
    fn a_target_verb_carries_the_candidates_too() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        let err = call(
            &table,
            "agent.rename",
            json!({ "target": "alp", "name": "x" }),
        )
        .expect_err("a missing name");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert!(
            err.message().ends_with(" — candidates: alpha"),
            "{}",
            err.message()
        );
    }

    // ── agent.resolve ────────────────────────────────────────────────────────────

    // ADR-0307
    #[test]
    fn resolve_answers_every_target_in_order_with_name_id_and_liveness() {
        let host = FakeHost::new();
        let alpha = host.with_agent("alpha", true, false);
        let beta = host.with_agent("beta", false, false);
        let corpse = host.with_status("gamma", Some(AgentStatus::Killed), false);
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.resolve",
            json!({ "targets": ["beta", alpha.to_string(), "gamma", "alpha"] }),
        )
        .expect("all resolve");
        assert_eq!(
            out,
            json!({ "agents": [
                { "target": "beta", "id": beta.to_string(), "name": "beta", "live": false },
                { "target": alpha.to_string(), "id": alpha.to_string(), "name": "alpha", "live": true },
                { "target": "gamma", "id": corpse.to_string(), "name": "gamma", "live": false },
                { "target": "alpha", "id": alpha.to_string(), "name": "alpha", "live": true },
            ]})
        );
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "a query does not change the roster"
        );
    }

    /// ★`live` is one decision with the `state` of `agent.list`★ — even if the session remains, an ended incarnation (not yet collected by the reaper)
    /// is not alive (ADR-0116).
    #[test]
    fn resolve_and_list_agree_on_liveness() {
        let host = FakeHost::new();
        host.with_status("exiting", Some(AgentStatus::Exiting), false);
        host.with_status("exited", Some(AgentStatus::Exited { code: Some(0) }), false);
        host.with_agent("asleep", false, false);
        let (table, _notify) = wiring(&host);

        let listed = call(&table, "agent.list", json!({})).expect("query");
        let resolved = call(
            &table,
            "agent.resolve",
            json!({ "targets": ["exiting", "exited", "asleep"] }),
        )
        .expect("query");
        for (row, answer) in listed["agents"]
            .as_array()
            .expect("array")
            .iter()
            .zip(resolved["agents"].as_array().expect("array"))
        {
            assert_eq!(
                answer["live"].as_bool(),
                Some(row["state"] == AGENT_STATE_LIVE),
                "{row} / {answer}"
            );
        }
    }

    /// Exactly the form the shell reads — `targets[` + decimal position + `]: `. `None` if the form does not match.
    fn failing_position(message: &str) -> Option<usize> {
        let rest = message.strip_prefix("targets[")?;
        let (digits, after) = rest.split_once(']')?;
        after.strip_prefix(": ")?;
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok()
    }

    /// ★A failure caused by one word keeps its code and its text starts with `targets[i]: ` — the shell reads this head★
    /// (the declaration header comment). With several blank words, the head is the first position.
    // ADR-0307
    #[test]
    fn the_resolve_failure_prefix_is_a_stable_contract() {
        let host = FakeHost::new();
        host.with_agent("alpha", true, false);
        host.with_agent("twin", false, false);
        host.with_agent("twin", false, false);
        let (table, _notify) = wiring(&host);
        let fail = |targets: serde_json::Value| {
            call(&table, "agent.resolve", json!({ "targets": targets })).expect_err("failure")
        };

        let missing = fail(json!(["alpha", "ghost"]));
        assert_eq!(missing.code(), ErrorCode::NotFound);
        assert_eq!(
            failing_position(missing.message()),
            Some(1),
            "{}",
            missing.message()
        );
        assert!(
            missing
                .message()
                .starts_with("targets[1]: no agent called 'ghost'"),
            "{}",
            missing.message()
        );
        assert!(
            missing.message().ends_with(" — candidates: alpha, twin"),
            "{}",
            missing.message()
        );

        let ambiguous = fail(json!(["alpha", "alpha", "twin"]));
        assert_eq!(ambiguous.code(), ErrorCode::Conflict);
        assert_eq!(
            failing_position(ambiguous.message()),
            Some(2),
            "{}",
            ambiguous.message()
        );
        assert!(
            ambiguous
                .message()
                .starts_with("targets[2]: more than one agent is called 'twin'"),
            "{}",
            ambiguous.message()
        );

        // Blank words are checked before reaching the resolver — a later blank word is rejected before a missing name at an earlier position (0).
        let blank = fail(json!(["ghost", "alpha", "  ", ""]));
        assert_eq!(blank.code(), ErrorCode::InvalidArgument);
        assert_eq!(
            failing_position(blank.message()),
            Some(2),
            "{}",
            blank.message()
        );
        assert!(
            blank.message().starts_with("targets[2]: blank value"),
            "{}",
            blank.message()
        );
        assert!(
            blank.message().ends_with("(also blank: targets[3])"),
            "the remaining blank positions are named at once too: {}",
            blank.message()
        );
    }

    #[test]
    fn resolve_takes_one_to_eight_targets() {
        let host = FakeHost::new();
        host.with_agent("alpha", false, false);
        let (table, _notify) = wiring(&host);

        for count in [0, RESOLVE_TARGETS_MAX + 1] {
            let err = call(
                &table,
                "agent.resolve",
                json!({ "targets": vec!["alpha"; count] }),
            )
            .expect_err("count out of range");
            assert_eq!(err.code(), ErrorCode::InvalidArgument, "{count}");
            assert_eq!(
                failing_position(err.message()),
                None,
                "a count rejection is not one word's failure, so there is no head: {}",
                err.message()
            );
        }
        let out = call(
            &table,
            "agent.resolve",
            json!({ "targets": vec!["alpha"; RESOLVE_TARGETS_MAX] }),
        )
        .expect("up to the cap is accepted");
        assert_eq!(
            out["agents"].as_array().map(Vec::len),
            Some(RESOLVE_TARGETS_MAX)
        );
    }

    /// ★The answer shape is pinned to a file★ — the shell does not depend on this crate and reads the answer as untyped JSON, and the shell tests'
    /// fake daemon builds its answers from the same file (TRD S28 §6). If a field name changes, either here or the shell side turns red.
    // ADR-0307
    #[test]
    fn the_resolve_reply_matches_the_golden_fixture() {
        let golden: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/agent_resolve_ok.json"))
                .expect("the golden is JSON");
        let host = FakeHost::new();
        for (id, name, live) in [
            ("11111111-1111-4111-8111-111111111111", "alpha", true),
            ("22222222-2222-4222-8222-222222222222", "beta", false),
        ] {
            host.rows.lock().unwrap().push(AgentRosterRow {
                id: id.parse().expect("fixed id"),
                canonical_name: name.to_string(),
                cwd: format!("C:/work/{name}"),
                parent: None,
                live: live.then_some(AgentStatus::Running),
            });
        }
        let (table, _notify) = wiring(&host);

        let out = call(
            &table,
            "agent.resolve",
            json!({ "targets": ["alpha", "beta"] }),
        )
        .expect("resolves");
        assert_eq!(out, golden);
    }

    #[test]
    fn a_wrong_argument_type_is_invalid_argument() {
        let (table, _notify) = wiring(&FakeHost::new());
        let err = call(&table, "agent.rename", json!({ "target": 5 })).expect_err("type mismatch");
        assert_eq!(err.code(), ErrorCode::InvalidArgument);
    }

    #[test]
    fn the_list_schema_inlines_the_nested_row_type() {
        let spec = AgentListArgs::SPEC;
        let ok: serde_json::Value =
            serde_json::from_str(spec.ok_schema).expect("the ok schema is JSON");
        assert_eq!(ok["properties"]["agents"]["type"], "array");
        assert_eq!(
            ok["properties"]["agents"]["items"]["properties"]["state"]["type"], "string",
            "a struct declared inside the block is spread inline"
        );
    }

    // ── Waiting-list query and cancel (ADR-0231 · TRD §5-6) ──

    fn listed(id: &str, state: ListedState) -> crate::queued_input::ListedRow {
        crate::queued_input::ListedRow {
            id: id.into(),
            text: format!("text of {id}"),
            state,
        }
    }

    #[test]
    fn the_queued_listing_carries_every_row_state_the_seq_the_incarnation_and_the_halt() {
        let host = FakeHost::new();
        let id = host.with_agent("alpha", true, false);
        *host.queued_listing.lock().unwrap() = Some(QueuedListing {
            rows: vec![
                listed("q1", ListedState::Queued),
                listed("q2", ListedState::Unconfirmed),
                listed("q5", ListedState::Sent),
                listed(
                    "q3",
                    ListedState::Cancelling {
                        answer: crate::queued_input::CancelAnswer::NotRemoved,
                        vendor_closed: true,
                    },
                ),
                listed(
                    "q4",
                    ListedState::Cancelling {
                        answer: crate::queued_input::CancelAnswer::Unanswered,
                        vendor_closed: false,
                    },
                ),
            ],
            as_of_seq: Some(7),
            epoch: 4_000_000_000,
            stopped_after_error: true,
        });
        let (table, notify) = wiring(&host);

        let out = call(
            &table,
            "agent.listQueuedInputs",
            json!({ "target": "alpha" }),
        )
        .expect("query succeeds");
        assert_eq!(
            out,
            json!({
                "inputs": [
                    { "id": "q1", "text": "text of q1", "state": "queued", "cancel": null },
                    { "id": "q2", "text": "text of q2", "state": "unconfirmed", "cancel": null },
                    { "id": "q5", "text": "text of q5", "state": "sent", "cancel": null },
                    { "id": "q3", "text": "text of q3", "state": "cancelling",
                      "cancel": { "answer": "not_removed", "vendor_closed": true } },
                    { "id": "q4", "text": "text of q4", "state": "cancelling",
                      "cancel": { "answer": "none", "vendor_closed": false } },
                ],
                "as_of_seq": 7,
                "epoch": 4_000_000_000u32,
                "stopped_after_error": true,
            })
        );
        assert_eq!(*host.queued_calls.lock().unwrap(), vec![(id, None)]);
        assert_eq!(
            *notify.calls.lock().unwrap(),
            0,
            "does not change the roster"
        );
    }

    #[test]
    fn an_empty_queue_lists_no_rows_and_a_null_seq() {
        let host = FakeHost::new();
        host.with_agent("alpha", true, false);
        *host.queued_listing.lock().unwrap() = Some(QueuedListing {
            rows: vec![],
            as_of_seq: None,
            epoch: 0,
            stopped_after_error: false,
        });
        let (table, _notify) = wiring(&host);
        let out = call(
            &table,
            "agent.listQueuedInputs",
            json!({ "target": "alpha" }),
        )
        .expect("query succeeds");
        assert_eq!(
            out,
            json!({ "inputs": [], "as_of_seq": null, "epoch": 0, "stopped_after_error": false })
        );
    }

    #[test]
    fn listing_the_queue_of_an_asleep_unknown_or_ambiguous_agent_is_refused() {
        let host = FakeHost::new();
        host.with_agent("asleep", false, false);
        host.with_agent("twin", true, false);
        host.with_agent("twin", true, false);
        let (table, _notify) = wiring(&host);

        let asleep = call(
            &table,
            "agent.listQueuedInputs",
            json!({ "target": "asleep" }),
        )
        .expect_err("there is no live session");
        assert_eq!(asleep.code(), ErrorCode::NotFound);
        assert!(
            asleep.message().contains("not running"),
            "does not say that an agent in the roster is missing: {}",
            asleep.message()
        );
        for (target, code) in [
            ("nobody", ErrorCode::NotFound),
            ("twin", ErrorCode::Conflict),
        ] {
            let err = call(
                &table,
                "agent.listQueuedInputs",
                json!({ "target": target }),
            )
            .expect_err("pointing fails");
            assert_eq!(err.code(), code, "{target}");
        }
        let blank = call(&table, "agent.listQueuedInputs", json!({ "target": " " }))
            .expect_err("blank value");
        assert_eq!(blank.code(), ErrorCode::InvalidArgument);
        assert_eq!(
            host.queued_calls.lock().unwrap().len(),
            1,
            "if the pointing does not stand, it does not reach the manager (asked only once, for the asleep agent)"
        );
    }

    /// The 「cancel result」 of the daemon row of TRD §7-1 — the session's answer is translated line by line into the verb response.
    #[test]
    fn cancel_translates_every_session_answer() {
        let answers: Vec<(Result<CancelOutcome, CancelError>, Result<&str, ErrorCode>)> = vec![
            (Ok(CancelOutcome::Cancelled), Ok("cancelled")),
            (Ok(CancelOutcome::Requested), Ok("requested")),
            (Err(CancelError::NotFound), Err(ErrorCode::NotFound)),
            (
                Err(CancelError::Write(PtyError::WriteFailed(
                    "pipe closed".into(),
                ))),
                Err(ErrorCode::Internal),
            ),
        ];
        for (answer, expected) in answers {
            let host = FakeHost::new();
            let id = host.with_agent("alpha", true, false);
            let label = format!("{answer:?}");
            *host.cancel_answer.lock().unwrap() = Some(answer);
            let (table, notify) = wiring(&host);

            let got = call(
                &table,
                "agent.cancelQueuedInput",
                json!({ "target": "alpha", "input_id": "q1" }),
            );
            match expected {
                Ok(word) => assert_eq!(got.expect(&label), json!({ "outcome": word }), "{label}"),
                Err(code) => {
                    let err = got.expect_err(&label);
                    assert_eq!(err.code(), code, "{label}");
                    if code == ErrorCode::Internal {
                        assert!(
                            err.message().contains("stays in the cancelling state"),
                            "a write failure says the entry stays pending cancel: {}",
                            err.message()
                        );
                    }
                }
            }
            assert_eq!(
                *host.queued_calls.lock().unwrap(),
                vec![(id, Some("q1".to_string()))],
                "{label}: the resolved id and input_id are handed over as is"
            );
            assert_eq!(
                *notify.calls.lock().unwrap(),
                0,
                "{label}: does not change the roster"
            );
        }
    }

    /// ★The failure text calls it by the roster name★ — the daemon's common entry rewrites `target` as the resolved id, so carrying the token as is
    /// would make the caller read a UUID it did not type.
    #[test]
    fn cancel_failures_name_the_agent_by_its_roster_name_even_when_targeted_by_id() {
        for answer in [
            CancelError::NotFound,
            CancelError::Write(PtyError::WriteFailed("pipe closed".into())),
        ] {
            let host = FakeHost::new();
            let id = host.with_agent("alpha", true, false);
            let label = format!("{answer:?}");
            *host.cancel_answer.lock().unwrap() = Some(Err(answer));
            let (table, _notify) = wiring(&host);

            let err = call(
                &table,
                "agent.cancelQueuedInput",
                json!({ "target": id.to_string(), "input_id": "q1" }),
            )
            .expect_err(&label);
            assert!(
                err.message().contains("'alpha'"),
                "{label}: {}",
                err.message()
            );
            assert!(
                !err.message().contains(&id.to_string()),
                "{label}: does not carry the id in the name position: {}",
                err.message()
            );
        }
    }

    #[test]
    fn cancel_requires_an_input_id_and_refuses_a_blank_one_before_touching_the_agent() {
        let host = FakeHost::new();
        host.with_agent("alpha", true, false);
        let (table, _notify) = wiring(&host);

        for args in [
            json!({ "target": "alpha" }),
            json!({ "target": "alpha", "input_id": "" }),
            json!({ "target": "alpha", "input_id": "   " }),
            json!({ "target": " ", "input_id": "q1" }),
        ] {
            let err = call(&table, "agent.cancelQueuedInput", args.clone()).expect_err("rejection");
            assert_eq!(err.code(), ErrorCode::InvalidArgument, "{args}");
        }
        let unknown = call(
            &table,
            "agent.cancelQueuedInput",
            json!({ "target": "nobody", "input_id": "q1" }),
        )
        .expect_err("pointing fails");
        assert_eq!(unknown.code(), ErrorCode::NotFound);
        assert!(
            host.queued_calls.lock().unwrap().is_empty(),
            "if the arguments or pointing do not stand, it does not reach the manager"
        );
    }

    /// The link's answer is translated line by line into the verb outcome (TRD S21-chat-ux §3-3).
    // ADR-0237
    #[test]
    fn interrupt_translates_every_transport_answer() {
        let answers: Vec<(Result<(), PtyError>, Result<&str, ErrorCode>)> = vec![
            (Ok(()), Ok("requested")),
            (
                Err(PtyError::NotFound(AgentId::new_v4())),
                Err(ErrorCode::NotFound),
            ),
            (
                Err(PtyError::Unsupported("no turn in flight".into())),
                Err(ErrorCode::Conflict),
            ),
            (
                Err(PtyError::WriteFailed("pipe closed".into())),
                Err(ErrorCode::Internal),
            ),
        ];
        for (answer, expected) in answers {
            let host = FakeHost::new();
            let id = host.with_agent("alpha", true, false);
            let label = format!("{answer:?}");
            *host.interrupt_answer.lock().unwrap() = Some(answer);
            let (table, notify) = wiring(&host);

            let got = call(
                &table,
                "agent.interrupt",
                json!({ "target": id.to_string() }),
            );
            match expected {
                Ok(word) => assert_eq!(got.expect(&label), json!({ "outcome": word }), "{label}"),
                Err(code) => {
                    let err = got.expect_err(&label);
                    assert_eq!(err.code(), code, "{label}");
                    assert!(
                        err.message().contains("'alpha'"),
                        "{label}: calls it by the roster name: {}",
                        err.message()
                    );
                }
            }
            assert_eq!(*host.interrupt_calls.lock().unwrap(), vec![id], "{label}");
            assert_eq!(
                *notify.calls.lock().unwrap(),
                0,
                "{label}: does not change the roster"
            );
        }
    }

    #[test]
    fn interrupt_refuses_a_blank_or_unknown_target_before_touching_the_agent() {
        let host = FakeHost::new();
        host.with_agent("alpha", true, false);
        let (table, _notify) = wiring(&host);

        let blank =
            call(&table, "agent.interrupt", json!({ "target": " " })).expect_err("blank value");
        assert_eq!(blank.code(), ErrorCode::InvalidArgument);
        let unknown = call(&table, "agent.interrupt", json!({ "target": "nobody" }))
            .expect_err("pointing fails");
        assert_eq!(unknown.code(), ErrorCode::NotFound);
        assert!(
            host.interrupt_calls.lock().unwrap().is_empty(),
            "if the arguments or pointing do not stand, it does not reach the manager"
        );
    }

    #[test]
    fn the_queue_list_schema_inlines_the_row_and_the_cancel_pair() {
        let ok: serde_json::Value =
            serde_json::from_str(AgentListQueuedInputsArgs::SPEC.ok_schema).expect("ok schema");
        let row = &ok["properties"]["inputs"]["items"]["properties"];
        assert_eq!(row["state"]["type"], "string");
        assert_eq!(
            row["cancel"]["anyOf"][0]["properties"]["vendor_closed"]["type"],
            "boolean"
        );
        let required: Vec<&str> = ok["required"]
            .as_array()
            .expect("required")
            .iter()
            .map(|v| v.as_str().expect("string"))
            .collect();
        assert_eq!(required, vec!["inputs", "epoch", "stopped_after_error"]);
    }

    // ── Usage (`usage.*` — TRD S21 usage-limit-slot §1-6) ───────────────────────────────────────────

    fn usage_table(usage: &Arc<FakeUsage>) -> CommandTable {
        make_table(
            FakeHost::new() as Arc<dyn AgentCommandHost>,
            Arc::new(FakeNotify::default()) as Arc<dyn RosterChanged>,
            Arc::clone(usage) as Arc<dyn UsageCommandHost>,
        )
    }

    fn usage_row() -> UsageVendorRow {
        UsageVendorRow {
            backend: AgentBackend::Claude,
            account_key: "default".to_owned(),
            plan: Some("max".to_owned()),
            windows: vec![UsageWindowRow {
                window: UsageWindowWord::FiveHour,
                label: None,
                used_pct: Some(40.0),
                left_pct: Some(60),
                resets_at: Some(1_900_000_000),
                age_secs: 3,
                expired: false,
            }],
            in_flight: false,
            served: UsageServedWord::Fresh,
            state: UsageStateWord::Failed,
            next_attempt_in_secs: Some(70),
            retry_in_secs: None,
            detail_kind: Some("rpc_error".to_owned()),
            detail_code: Some(-32_603),
            upstream: Some("upstream text".to_owned()),
        }
    }

    /// The two verbs go to their own methods of the same port (with the received word as is), the answer is a flat row without a layer, and the answer shapes of the two verbs
    /// are the same — even though the return struct of `usage.refresh` is built separately, the JSON is one row.
    #[test]
    fn the_usage_verbs_answer_the_ports_row_flat_and_alike() {
        let usage = Arc::new(FakeUsage::default());
        *usage.row.lock().unwrap() = Some(usage_row());
        let table = usage_table(&usage);

        let got = call(&table, "usage.get", json!({ "backend": "Claude" })).expect("success");
        let refreshed =
            call(&table, "usage.refresh", json!({ "backend": "Codex" })).expect("success");

        assert_eq!(
            got,
            serde_json::to_value(usage_row()).expect("row serialization")
        );
        assert_eq!(got["state"], "Failed");
        assert_eq!(got["served"], "Fresh");
        assert_eq!(got["windows"][0]["window"], "FiveHour");
        assert_eq!(got["windows"][0]["left_pct"], 60);
        assert_eq!(got["detail_code"], -32_603);
        assert_eq!(
            refreshed, got,
            "the answers of the two verbs have the same row shape"
        );
        assert_eq!(
            *usage.calls.lock().unwrap(),
            vec![
                ("get", AgentBackend::Claude),
                ("refresh", AgentBackend::Codex)
            ]
        );
    }

    /// A backend without a field fails **with a value** (no panic) — the same code and the same sentence as the same absence of wire ⟳.
    #[test]
    fn a_backend_without_a_usage_slot_fails_by_value_with_not_found() {
        let usage = Arc::new(FakeUsage::default());
        let table = usage_table(&usage);
        for name in ["usage.get", "usage.refresh"] {
            let err = call(&table, name, json!({ "backend": "Codex" })).expect_err("no field");
            assert_eq!(err.code(), ErrorCode::NotFound, "{name}");
            assert!(
                err.message()
                    .contains("this daemon keeps no usage limits for backend 'Codex'"),
                "{name}: {err}"
            );
        }
    }

    /// ★The bus word (capitalized) reaches the same field key as the wire word (lowercase)★ — all declared words. Building the key from the incoming spelling
    /// would have the two entries hit different fields (TRD §3 #43).
    #[test]
    fn every_declared_backend_reaches_the_usage_key_its_wire_word_reaches() {
        let every = |backend: &AgentBackend| match backend {
            // If the declared words grow, compilation stops here — add to the list below.
            AgentBackend::Claude | AgentBackend::Codex => (),
        };
        for backend in [AgentBackend::Claude, AgentBackend::Codex] {
            every(&backend);
            let bus =
                usage_vendor_of(&backend).unwrap_or_else(|| panic!("the prober of {backend:?}"));
            let wire_word = backend_word(&backend).to_ascii_lowercase();
            let wire = crate::backend::usage_probe_for(&wire_word)
                .unwrap_or_else(|| panic!("the prober of {wire_word}"))
                .key();
            assert_eq!(bus, wire, "{backend:?}");
        }
    }

    /// The return schemas of the two verbs are the same down to the bytes — with two return structs, a field could grow on only one side (`From` blocks it at compile
    /// time, but the advertisement is carried by the schema).
    #[test]
    fn the_two_usage_verbs_advertise_one_row_schema() {
        assert_eq!(
            UsageGetArgs::SPEC.ok_schema,
            UsageRefreshArgs::SPEC.ok_schema
        );
        let ok: serde_json::Value =
            serde_json::from_str(UsageGetArgs::SPEC.ok_schema).expect("ok schema");
        let window = &ok["properties"]["windows"]["items"]["properties"];
        assert_eq!(
            window["window"]["enum"],
            json!(["FiveHour", "Weekly", "ModelWeekly"])
        );
        assert_eq!(
            ok["properties"]["state"]["enum"],
            json!([
                "Ready",
                "NotInstalled",
                "NeedsLogin",
                "Unavailable",
                "Failed",
                "Rejected"
            ])
        );
        assert_eq!(
            ok["properties"]["served"]["enum"],
            json!(["Fresh", "Cached"])
        );
        assert_eq!(
            ok["properties"]["detail_code"]["anyOf"][0]["type"], "integer",
            "upstream numbers are carried as JSON numbers"
        );
    }
}
