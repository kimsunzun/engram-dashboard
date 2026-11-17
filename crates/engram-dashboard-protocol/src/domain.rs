//! Domain types (wire representation). Mirrors the serialized form of the current `agent::types` / `agent::profile`.
//! ★Do not make `agent` depend on this crate to merge this duplication★ — that crate being protocol-independent is
//! an invariant, so the mirror is the price (canonical = the `[dependencies]` comment of `crates/engram-dashboard-agent/Cargo.toml`).
//! ★But the usage limit types (`Usage*`) are not mirrors★ — they are the shape of the daemon's answer, so their fields deliberately differ from
//!   agent `usage`'s observation types (sign · age · expiry). Do not try to match them.

use ts_rs::TS;

use crate::ids::{AgentId, PresetId, ProfileId};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum AgentStatus {
    Running,
    Exiting,
    Exited { code: Option<i32> },
    Failed { message: String },
    Killed,
}

/// Per-area capability (prevents a bool explosion).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct Capabilities {
    pub input: InputCaps,
    pub output: OutputCaps,
    pub control: ControlCaps,
    pub session: SessionCaps,
    pub model: ModelCaps,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct InputCaps {
    pub raw: bool,
    pub message: bool,
    pub attachment: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct OutputCaps {
    pub terminal_bytes: bool,
    /// Whether it is a structured stream (NDJSON) (ADR-0044).
    /// `#[serde(default)]` (FIX 3): a field newly added in M1, so even receiving old wire without this field (old daemon/frontend)
    /// it tolerantly deserializes as false (the same additive·tolerant approach as sibling `output_format` —
    /// PROTOCOL_VERSION kept). ts-rs does not mark serde(default) as optional, so the TS is still
    /// `structured: boolean` (non-optional) — the frontend needs no change.
    #[serde(default)]
    pub structured: bool,
    pub markdown: bool,
    pub tool_events: bool,
    pub usage: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct ControlCaps {
    pub resize: bool,
    pub interrupt: bool,
    pub cancel: bool,
    pub graceful_shutdown: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct SessionCaps {
    pub resume: bool,
    pub snapshot: bool,
    pub cwd_env: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct ModelCaps {
    pub select: bool,
    pub temperature: bool,
    pub max_tokens: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct AgentInfo {
    #[ts(type = "string")]
    pub id: AgentId,
    /// Display name (single source of truth is ProfileRegistry; if absent, the first 8 chars of the id).
    pub name: String,
    pub cwd: String,
    pub status: AgentStatus,
    pub cols: u16,
    pub rows: u16,
    /// ★An **opaque marker** pointing at one incarnation★ — a random number drawn anew per incarnation, so **its order has no
    /// meaning**. Comparison uses only equal/unequal (do not derive "newer" from magnitude, ADR-0163). The receiver uses this
    /// value to decide "is the output stream I am reading now the same stream as before" — the resubscribe trigger·deps are not this
    /// field but observation of the authoritative registry (ADR-0164 decision 8).
    /// It does not outlive the daemon process — after a restart, even the same agent comes back with a different marker.
    pub epoch: u32,
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum RestoreOutcome {
    Resumed,
    Started,
    FreshFallback {
        old_sid: Option<String>,
        new_sid: String,
        reason: String,
    },
    Blocked {
        reason: String,
    },
    Failed {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct RestoreReport {
    #[ts(type = "string")]
    pub agent_id: AgentId,
    pub epoch: u32,
    pub outcome: RestoreOutcome,
}

// ── Profile wire mirror (phase4 step 1) ──────────────────────────────────────────────
//
// agent is protocol-independent (§1 invariant), so agent types cannot be used here — so an independent type of the same JSON shape
// is kept, and the explicit agent↔wire conversion is done by the daemon (no reflection round trip — the agent_info_to_wire pattern).
// Matches the frontend `src/api/types.ts` AgentProfile/AgentCommand/RestartPolicy character for character.

/// Terminal=PTY interactive, StreamJson=headless NDJSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum AgentOutputFormat {
    #[default]
    Terminal,
    StreamJson,
}

/// Envelope format wire type (ADR-0096/0103) — the value of the format switch the daemon uses when wrapping A→B messages.
/// The render rules (exact strings·attributes) are owned solely by the daemon's `control::ingress` — this wire type carries only the switch
/// value (design·assembly is the daemon's, this crate is a pure wire contract). Same name as the actual render enum (daemon side) but a separate type.
///
/// ★serde lowercase (load-bearing)★: with `#[serde(rename_all="lowercase")]` the wire JSON serializes as `"colon"`/`"xml"`
/// (lowercase variant names) — the contract that lets the `set_envelope_format({format:"xml"})` invoke JSON deserialize
/// as is (a surface operators/LLMs call by hand, so lowercase is natural). Other wire
/// enums (AgentOutputFormat etc.) are PascalCase, but this type is exposed directly on the invoke surface, so it stays lowercase.
/// ★Default = Xml★: `#[default]` — consistent with the daemon's global state initial value (ADR-0103 default flip). The wire default itself
/// is not used in the wiring because SetEnvelopeFormat.format is not `#[serde(default)]` (always explicit), but if it diverged from the operating default,
/// future code calling `EnvelopeFormat::default()` would be misled, so it is kept identical to the daemon default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum EnvelopeFormat {
    /// `<message from="{sender}" ...>{body}</message>` — structured envelope, operating default (ADR-0103).
    #[default]
    Xml,
    /// `{sender}: {body}` — human chat convention, a remaining switch (ADR-0103 — not deleted).
    Colon,
}

/// Wire mirror of agent `profile::AgentCommand`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum AgentSpawnCommand {
    /// extra_args are the user's additional arguments excluding session arguments.
    /// output_format is `#[serde(default)]`, so an old packet without this field is absorbed as Terminal.
    Claude {
        extra_args: Vec<String>,
        #[serde(default)]
        output_format: AgentOutputFormat,
    },
    Shell {
        program: String,
        args: Vec<String>,
    },
    /// extra_args are the user's additional arguments excluding the interactive arguments (`--cd`·`-s`·`-a`) — that assembly is done by the core's
    /// codex backend, and this wire carries the list as is.
    /// output_format has the same contract as sibling `Claude` (`#[serde(default)]` → Terminal if absent), but
    /// **what it splits is different**: in codex this value splits the interactive TUI and the resident JSON server (`codex app-server`)
    /// (core `backend::codex::is_app_server`).
    Codex {
        extra_args: Vec<String>,
        #[serde(default)]
        output_format: AgentOutputFormat,
    },
}

/// The field with which a spawn packet picks **which backend to launch**. If `AgentSpawnCommand` is 「what, with which arguments」,
/// this is a narrow vocabulary that picks only 「which program」 — used by an entry point that cannot decide the arguments yet (`SpawnByCwd`).
///
/// ★Absence means error (user decision 2026-09-07)★: the daemon refuses a packet that leaves this field empty. With a default,
/// every new spawn entry point would fail to tell apart **not choosing** and **choosing claude**, and that silent
/// default would become 「an agent different from the requested one started」.
/// ★Why the spelling is lowercase★: this value must be the same word as the string coming from the invoke surface (the `backend` argument of `spawn_into`),
/// and that spot was already written as `"claude"` (the same reason as sibling `EnvelopeFormat`).
/// ★The accepted spelling is case-insensitive · **the emitted spelling is lowercase only**★: what fills this field
/// is a word typed by hand by a person·LLM (`backend` of `agent.spawnInto`, the frontend's `SpawnByCwd`), and there
/// `Claude` is not a typo but the same meaning. Conversely, widening the emitting side would make `agents.json` and the ts-rs union
/// diverge, so [`Serialize`](serde::Serialize) stays as the derive.
// `Ord` = declaration order — no meaning, used only as the key of an ordered set (the shell's usage interest `BTreeSet`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum AgentBackendKind {
    Claude,
    Codex,
}

impl AgentBackendKind {
    /// All wire words — ★this is the only place the spelling is written★. The expected value in the refusal message is this array as is
    /// (`parse_backend` carries that message), and `ALL` below also takes the words from here.
    /// Must equal the spelling `#[serde(rename_all = "lowercase")]` emits, and the place that measures that match =
    /// `tests::backend_kind_serializes_exactly_the_declared_words`.
    const WORDS: &'static [&'static str] = &["claude", "codex"];

    /// (variant, wire notation) pairs — the only table deserialization uses to go back from a word to a variant.
    const ALL: &'static [(AgentBackendKind, &'static str)] = &[
        (AgentBackendKind::Claude, Self::WORDS[0]),
        (AgentBackendKind::Codex, Self::WORDS[1]),
    ];
}

/// ★Received as `String` (do not narrow it to `&str`)★ — the same reason as sibling [`AgentFailureKind`]: if there is even one
/// escape, serde_json goes down the scratch path, the borrowed `&str` fails, and **the whole message** breaks.
/// ★Unknown words are refused, not absorbed★ — this is where it parts ways with that sibling: folding here would silently launch a backend
/// different from the requested one (the same decision as 「absence means error」 in this type's doc).
impl<'de> serde::Deserialize<'de> for AgentBackendKind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = <String as serde::Deserialize>::deserialize(d)?;
        let folded = raw.to_ascii_lowercase();
        AgentBackendKind::ALL
            .iter()
            .find(|(_, word)| *word == folded)
            .map(|(kind, _)| *kind)
            .ok_or_else(|| serde::de::Error::unknown_variant(&raw, AgentBackendKind::WORDS))
    }
}

/// **Reserved — not a dead field.** Behavior not implemented, but ADR-0016 "revisit later" is in effect (decision 2026-06-18).
/// Removing it drags agent·ts-rs bindings·frontend along + causes a PROTOCOL_VERSION bump → do not remove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum RestartPolicy {
    Never,
    OnCrash,
    Always,
}

/// Declares the 「last failure」 vocabulary **in one list** — the enum · wire notation · full list · two-way serialization
/// all come from a single expansion of this macro.
///
/// ★Reason to exist = keep half-done edits from compiling★: with two hand-written tables (the old form: one exhaustive match
///   + one fixed-length array), fixing only one side when adding a variant **still builds**. Then two peers of the same version
///   have serialize emitting the new string while deserialize cannot find it and folds it into `Other` — the round trip breaks and
///   the screen shows the generic message instead of that kind's message forever. With a single list, that half-done edit is impossible.
/// ★The wire strings are **derived from the variant names** with `stringify!`★: the TS union ts-rs emits also comes from the variant names,
///   so unless strings are written by hand the two are structurally the same (the golden in `messages.rs` re-checks that fact by actually reading
///   the generated `.ts`).
// ADR-0172
macro_rules! declare_failure_kinds {
    (
        $( $(#[$vmeta:meta])* $variant:ident ),+ $(,)?
        ; absorbing = $absorbing:ident
    ) => {
        /// Same as agent `failure::AgentFailureKind`. The kind vocabulary of the 「last failure」 — **the screen messages are not
        /// here**: the kind → {worth retrying · message · suggested action} table is owned by the frontend
        /// (ADR-0172 decision 5 · ADR-0173).
        ///
        /// ★A separate axis from the status (`AgentStatus`), so they are not merged★ — the combination "running but holding a last failure" must be
        ///   expressible for the screen's 「running wins」 rule to hold.
        /// ★To add variants, fix only the one `declare_failure_kinds!` call★ — you cannot write them here directly.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, TS)]
        #[ts(export)]
        pub enum AgentFailureKind {
            $( $(#[$vmeta])* $variant, )+
        }

        impl AgentFailureKind {
            /// Full vocabulary = (variant, wire notation). The macro builds it from **the same list** as the enum.
            const ALL: &'static [(AgentFailureKind, &'static str)] =
                &[ $( (AgentFailureKind::$variant, stringify!($variant)), )+ ];

            /// Full vocabulary — the axis the golden compares with the generated TS union (so the test does not write the vocabulary again).
            pub fn all() -> impl Iterator<Item = &'static (AgentFailureKind, &'static str)> {
                AgentFailureKind::ALL.iter()
            }

            /// Only the names of the same list as above.
            pub fn wire_names() -> impl Iterator<Item = &'static str> {
                AgentFailureKind::ALL.iter().map(|(_, name)| *name)
            }

            /// Wire notation — the variant name as is (`stringify!`).
            fn as_wire_str(self) -> &'static str {
                match self {
                    $( AgentFailureKind::$variant => stringify!($variant), )+
                }
            }
        }

        impl serde::Serialize for AgentFailureKind {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(self.as_wire_str())
            }
        }

        /// Folds an unknown kind into the absorbing variant — **the only device protecting old peers when the vocabulary grows**.
        ///
        /// ★The accident it prevents★: this type is **inside** the profile struct, so with a plain derive, one new variant string
        ///   fails decoding of the **whole message** of `AgentProfile` — not just the field going empty, but that profile
        ///   disappearing entirely. An old-build src-tauri shell attaching to a new daemon is a real path, and that side's
        ///   receiver **silently discards** decode failures (the only symptom is "the tree does not change").
        /// ★The Rust-side half of the same contract as the frontend★: `src/components/agent/failureKinds.ts` already does
        ///   the same fail-open with `table[kind] ?? Other`.
        /// ★The absorption covers only **unknown strings**★: non-strings like `42`·`[]`·`{}` stay errors.
        ///   Swallowing everything would turn a broken value into the absorbing variant and **show a failure message on an item with no failure** —
        ///   that is fabrication, not absorption.
        /// ★Received as `String` (do not narrow it to `&str`)★: a borrowed `&str` succeeds only on an in-memory buffer without escapes
        ///   — with even a single escape like `"Other"`, serde_json goes down the scratch
        ///   path, emits `invalid type: string, expected a borrowed string`, and **the whole message**
        ///   breaks. The `from_value`/`from_reader` paths are the same.
        impl<'de> serde::Deserialize<'de> for AgentFailureKind {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let raw = <String as serde::Deserialize>::deserialize(d)?;
                Ok(AgentFailureKind::ALL
                    .iter()
                    .find(|(_, name)| *name == raw)
                    .map(|(kind, _)| *kind)
                    .unwrap_or(AgentFailureKind::$absorbing))
            }
        }
    };
}

declare_failure_kinds! {
    /// There is no real conversation to resume — an item that died without exchanging a single word.
    NoConversationToResume,
    /// Could not launch the process.
    SpawnFailed,
    /// Started to resume but exited within the observation window.
    EarlyExitAfterResume,
    /// The producer could not classify it — **and** the spot where the consumer absorbs unknown vocabulary.
    Other,
    ; absorbing = Other
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct AgentProfile {
    #[ts(type = "string")]
    pub id: ProfileId,
    pub name: String,
    /// User-specified display name override (ADR-0061 enrichment — tree rename). `Some` → shown as is, `None` → derived from the cwd
    /// basename (existing behavior unchanged). The frontend tree uses this value as the preferred display name
    /// instead of `name` (`name` is the CreateProfile name/ad-hoc cwd string, so it is unsuitable as a display name).
    #[serde(default)]
    #[ts(type = "string | null")]
    pub display_name: Option<String>,
    /// Tree hierarchy parent profile id (ADR-0072). `Some` → this profile is a child of that parent (tree indentation), `None` →
    /// top level (root). One-level nesting·parent deletion=promotion to root rules are the daemon's (reparent).
    /// `#[serde(default)]`, so old wire without this field → None (root, PROTOCOL_VERSION kept — same shape as display_name).
    #[serde(default)]
    #[ts(type = "string | null")]
    pub parent_id: Option<ProfileId>,
    pub command: AgentSpawnCommand,
    /// Normalized cwd.
    pub cwd: String,
    /// ※No credentials (persisted in plain text).
    pub env: Vec<(String, String)>,
    #[ts(type = "string | null")]
    pub backend_session_id: Option<String>,
    #[ts(type = "string[]")]
    pub old_session_ids: Vec<String>,
    pub epoch: u32,
    pub auto_restore: bool,
    pub restart_policy: RestartPolicy,
    /// Crash guard counter (reset to 0 on manual restart). **Reserved** — behavior not implemented, ADR-0016 in effect.
    pub restart_count: u32,
    /// Failed (auto-restore suspend) reason — persists across cold boot, excluded from auto-restore until a manual wake-up (ADR-0016).
    /// **Reserved** — behavior not implemented but in effect per ADR-0016, do not remove (causes a version bump).
    #[ts(type = "string | null")]
    pub failed_reason: Option<String>,
    /// The kind of this item's last activation failure (ADR-0172). `null` = no failure record.
    ///
    /// ★Lives only in daemon memory★ — the agent-side original is `#[serde(skip)]`, so it is not in `agents.json`, and it disappears
    ///   when the daemon restarts (it survives an app window restart).
    /// `#[serde(default)]`, so old wire without this field → None (PROTOCOL_VERSION kept — the same additive shape as display_name·
    /// parent_id). An unknown **value** is absorbed into `Other` (the `Deserialize` impl of `AgentFailureKind` — that doc is
    /// canonical for why it is needed).
    #[serde(default)]
    pub last_failure: Option<AgentFailureKind>,
    pub created_at: i64,
    pub last_active: i64,
    /// Last process start time (for records·debugging, not used for the reset decision).
    #[ts(type = "number | null")]
    pub last_start_at: Option<i64>,
}

// ── Preset wire mirror (ADR-0061) ──────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct Preset {
    #[ts(type = "string")]
    pub id: PresetId,
    /// Normalized cwd.
    pub cwd: String,
    /// User-specified display name override (ADR-0061 enrichment). `Some` → shown as is, `None` → derived from the cwd basename
    /// (existing behavior unchanged).
    #[serde(default)]
    #[ts(type = "string | null")]
    pub name: Option<String>,
}

/// Matches agent `types::OutputChunk`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct SnapshotChunk {
    #[ts(type = "number")]
    pub seq: u64,
    #[serde(with = "serde_bytes")]
    #[ts(type = "number[]")]
    pub data: Vec<u8>,
}

// ── Usage limits wire (subscription-based — the client subscribes to a vendor and the daemon sends whenever it changes) ──────────────

/// The usage state of one vendor·account — one sheet the daemon sends to connections that subscribed to that vendor
/// ([`AgentEvent::UsageLimitsUpdated`](crate::AgentEvent::UsageLimitsUpdated)).
///
/// ★Time field rule★: the only absolute time is [`UsageWindow::resets_at`] (epoch seconds the server gave). The other time
///   fields (`age_secs`·`next_attempt_in_secs`·`retry_in_secs`) are **relative seconds as of the moment this sheet was taken**, so the receiver
///   reads them by adding the time elapsed since receipt — so they do not wobble even if the two wall clocks disagree or rewind.
/// ★The status text is built by the receiver with translation keys★ — status travels as code + numbers. The exception = `detail` (the classification word `kind`·
///   the upstream original `upstream` are shown as is without translation).
/// ★A `null` window·`null` number means 「unknown」, not 0★ — must not be drawn as 0%.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct UsageLimitSnapshot {
    pub vendor: AgentBackendKind,
    /// The account within the same vendor — always `"default"` for now (the default login of the daemon env).
    pub account_key: String,
    pub five_hour: Option<UsageWindow>,
    pub weekly: Option<UsageWindow>,
    /// Per-model weekly windows. Empty array = none.
    pub model_scoped: Vec<UsageScopedWindow>,
    pub plan: Option<String>,
    /// The query is still in progress — this sheet's values are from before that query, and the result goes to subscribed connections as a new
    /// sheet when the query ends.
    pub in_flight: bool,
    /// The values above are valid even when not `Ready` — a failure does not erase the last known values.
    /// The exception is `Unavailable` — it carries no values.
    pub state: UsageVendorState,
    /// A monotonic number for this area (vendor·account) — +1 whenever anything changes. The receiver keeps only the maximum per area within the same connection (socket)
    /// and drops smaller ones. When a new socket comes up it forgets that memory — when the daemon restarts it counts again from 0.
    #[ts(type = "number")]
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct UsageWindow {
    /// Percentage 0–100 of the amount **used** (not remaining). The producer rounds it to the fourth decimal place before carrying it (agent
    /// `PCT_SCALE` in `usage/normalize.rs`) — the receiver shows the remaining amount as `floor(100 − used_pct)` without correction, so
    /// keeping the binary floating-point tail (0.55 × 100 = 55.00000000000001) from crossing an integer boundary relies on that rounding.
    pub used_pct: Option<f64>,
    /// Reset time, epoch seconds.
    #[ts(type = "number | null")]
    pub resets_at: Option<u64>,
    /// Seconds elapsed since this value was observed.
    #[ts(type = "number")]
    pub age_secs: u64,
    /// The daemon confirmed the reset elapsed — once set, it stays until a new value comes for this window (even if the wall clock rewinds).
    pub expired: bool,
}

/// One per-model weekly window. `label` = the display name the vendor gave.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct UsageScopedWindow {
    pub label: String,
    pub window: UsageWindow,
}

/// The status code of a vendor query. `detail` of the five abnormal ones = why it is in that state (`null` if absent — the cause is unknown).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum UsageVendorState {
    Ready,
    /// That vendor's CLI is missing.
    NotInstalled {
        detail: Option<UsageStateDetail>,
    },
    /// Auth error — login is needed.
    NeedsLogin {
        detail: Option<UsageStateDetail>,
    },
    /// The query worked but this account has no limit info (logged out · API key account · a token without profile permission, etc.)
    /// — the snapshot has no window values.
    Unavailable {
        detail: Option<UsageStateDetail>,
    },
    /// Other failures. `next_attempt_in_secs` = seconds left until the next automatic query — always defined (if there is no reference point, the end of the refusal
    /// or now), 0 if already past.
    Failed {
        #[ts(type = "number")]
        next_attempt_in_secs: u64,
        detail: Option<UsageStateDetail>,
    },
    /// The upstream refused the query. Until these seconds pass, even a forced refresh does not send a query.
    Rejected {
        #[ts(type = "number")]
        retry_in_secs: u64,
        detail: Option<UsageStateDetail>,
    },
}

/// The cause of an abnormal state.
///
/// `kind` = classification word (e.g. `rate_limits_null`·`claude_error`·`rpc_error`·`limits_unavailable`·`timeout`).
/// ★A string, not an enum★ — so an old shell never fails to read the whole snapshot even as classifications grow. The receiver
///   shows it as is without translating.
/// `code` = the number the upstream gave (Codex JSON-RPC `error.code`). `upstream` = the upstream original — the sender
///   trimmed it in this order: mask secrets → whitespace-like controls (tab·newline·CR·VT·FF·NEL·U+2028/2029) become spaces, other controls·
///   invisible format characters become U+FFFD → cut to 200 chars. The receiver does not need to sanitize again.
/// ★`Debug` prints `upstream` only as a char count★ — so the original never goes to a log wherever it is printed with `{:?}`.
///   The original is seen only by the wire (`Serialize`) and comparison (`PartialEq`).
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct UsageStateDetail {
    pub kind: String,
    #[ts(type = "number | null")]
    pub code: Option<i64>,
    pub upstream: Option<String>,
}

impl std::fmt::Debug for UsageStateDetail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        struct CharCount(usize);
        impl std::fmt::Debug for CharCount {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "<{} chars>", self.0)
            }
        }
        f.debug_struct("UsageStateDetail")
            .field("kind", &self.kind)
            .field("code", &self.code)
            .field(
                "upstream",
                &self.upstream.as_ref().map(|s| CharCount(s.chars().count())),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An old packet without this field — builds whose daemon did not carry `output_format` when sending the registry have this shape.
    /// ★`#[serde(default)]` covers only one direction, **old daemon → new shell**★: without it, that daemon's
    /// `ProfileList` collapses with a deserialization failure of **the whole response, not one row** (the canonical source for that failure unit is
    /// the v4 entry of [`crate::PROTOCOL_VERSION`]). What supports the opposite direction (new daemon → old shell) is not this
    /// attribute but serde's **unknown field tolerance**, so it is not measured here.
    #[test]
    fn codex_spawn_command_without_output_format_defaults_to_terminal() {
        let legacy = r#"{ "kind": "Codex", "extra_args": ["--foo"] }"#;
        let cmd: AgentSpawnCommand = serde_json::from_str(legacy).expect("deserialize old packet");
        assert_eq!(
            cmd,
            AgentSpawnCommand::Codex {
                extra_args: vec!["--foo".to_string()],
                output_format: AgentOutputFormat::Terminal,
            }
        );
    }

    /// Carried values come back as is — with the item above alone, it would be green even if this type **pinned** `Terminal`.
    #[test]
    fn codex_spawn_command_roundtrips_both_modes() {
        for mode in [AgentOutputFormat::Terminal, AgentOutputFormat::StreamJson] {
            let cmd = AgentSpawnCommand::Codex {
                extra_args: vec![],
                output_format: mode,
            };
            let json = serde_json::to_string(&cmd).expect("serialize");
            assert_eq!(
                serde_json::from_str::<AgentSpawnCommand>(&json).expect("deserialize"),
                cmd,
                "{json}"
            );
        }
    }

    /// Hand-typed words are case-insensitive — `backend` of `agent.spawnInto` and the frontend's
    /// `SpawnByCwd` carry strings typed by a person·LLM as is.
    #[test]
    fn backend_kind_accepts_any_casing() {
        for (raw, want) in [
            ("claude", AgentBackendKind::Claude),
            ("Claude", AgentBackendKind::Claude),
            ("CLAUDE", AgentBackendKind::Claude),
            ("codex", AgentBackendKind::Codex),
            ("Codex", AgentBackendKind::Codex),
            ("CODEX", AgentBackendKind::Codex),
        ] {
            let got: AgentBackendKind =
                serde_json::from_value(serde_json::Value::String(raw.to_string()))
                    .unwrap_or_else(|e| panic!("'{raw}' must be accepted: {e}"));
            assert_eq!(got, want, "{raw}");
        }
    }

    /// The tolerance covers only **spelling** — unknown words are refused as is, and the message lists the expected words.
    #[test]
    fn backend_kind_still_rejects_a_word_it_does_not_know() {
        let err = serde_json::from_value::<AgentBackendKind>(serde_json::Value::String(
            "codx".to_string(),
        ))
        .expect_err("unknown word is refused");
        let text = err.to_string();
        assert!(text.contains("codx"), "{text}");
        assert!(text.contains("claude") && text.contains("codex"), "{text}");
    }

    /// The emitted spelling does not change — `agents.json` and the ts-rs union read this value.
    #[test]
    fn backend_kind_serializes_exactly_the_declared_words() {
        for (kind, word) in AgentBackendKind::ALL {
            assert_eq!(
                serde_json::to_value(kind).expect("serialize"),
                serde_json::Value::String((*word).to_string())
            );
        }
        assert_eq!(
            AgentBackendKind::ALL.len(),
            AgentBackendKind::WORDS.len(),
            "the word table and the pair table diverged"
        );
    }

    // ── Usage limits wire ──

    fn usage_window(used_pct: Option<f64>) -> UsageWindow {
        UsageWindow {
            used_pct,
            resets_at: Some(1_900_000_000),
            age_secs: 42,
            expired: false,
        }
    }

    fn usage_snapshot(state: UsageVendorState) -> UsageLimitSnapshot {
        UsageLimitSnapshot {
            vendor: AgentBackendKind::Codex,
            account_key: "default".to_string(),
            five_hour: Some(usage_window(Some(37.5))),
            weekly: None,
            model_scoped: vec![UsageScopedWindow {
                label: "opus".to_string(),
                window: usage_window(None),
            }],
            plan: Some("pro".to_string()),
            in_flight: true,
            state,
            revision: 7,
        }
    }

    fn detail(code: Option<i64>, upstream: Option<&str>) -> UsageStateDetail {
        UsageStateDetail {
            kind: "rpc_error".to_string(),
            code,
            upstream: upstream.map(str::to_string),
        }
    }

    /// Measures the whole shape at once — partial assertions let renames·notation changes of unchecked fields pass.
    #[test]
    fn usage_snapshot_serializes_to_the_exact_wire_shape() {
        let snap = usage_snapshot(UsageVendorState::Failed {
            next_attempt_in_secs: 840,
            detail: Some(detail(Some(-32603), Some("internal error"))),
        });
        assert_eq!(
            serde_json::to_value(&snap).expect("serialize"),
            serde_json::json!({
                "vendor": "codex",
                "account_key": "default",
                "five_hour": {
                    "used_pct": 37.5, "resets_at": 1_900_000_000u64, "age_secs": 42,
                    "expired": false
                },
                "weekly": null,
                "model_scoped": [{
                    "label": "opus",
                    "window": {
                        "used_pct": null, "resets_at": 1_900_000_000u64, "age_secs": 42,
                        "expired": false
                    }
                }],
                "plan": "pro",
                "in_flight": true,
                "state": {
                    "kind": "Failed",
                    "next_attempt_in_secs": 840,
                    "detail": { "kind": "rpc_error", "code": -32603, "upstream": "internal error" }
                },
                "revision": 7
            })
        );
    }

    /// The six states round-trip with only code + relative seconds + cause — the five abnormal ones run both with and without `detail`, the two with
    /// fields run both a value and `null`. The tag `kind` (state) and `detail.kind` (classification) live together in one JSON.
    #[test]
    fn every_usage_vendor_state_roundtrips_inside_a_snapshot() {
        let full = || Some(detail(Some(429), Some("slow down")));
        let bare = || {
            Some(UsageStateDetail {
                kind: "timeout".to_string(),
                code: None,
                upstream: None,
            })
        };
        let full_json =
            serde_json::json!({ "kind": "rpc_error", "code": 429, "upstream": "slow down" });
        let bare_json = serde_json::json!({ "kind": "timeout", "code": null, "upstream": null });
        let cases = [
            (
                UsageVendorState::Ready,
                serde_json::json!({ "kind": "Ready" }),
            ),
            (
                UsageVendorState::NotInstalled { detail: None },
                serde_json::json!({ "kind": "NotInstalled", "detail": null }),
            ),
            (
                UsageVendorState::NotInstalled { detail: bare() },
                serde_json::json!({ "kind": "NotInstalled", "detail": bare_json }),
            ),
            (
                UsageVendorState::NeedsLogin { detail: None },
                serde_json::json!({ "kind": "NeedsLogin", "detail": null }),
            ),
            (
                UsageVendorState::NeedsLogin { detail: full() },
                serde_json::json!({ "kind": "NeedsLogin", "detail": full_json }),
            ),
            (
                UsageVendorState::Unavailable { detail: None },
                serde_json::json!({ "kind": "Unavailable", "detail": null }),
            ),
            (
                UsageVendorState::Unavailable { detail: bare() },
                serde_json::json!({ "kind": "Unavailable", "detail": bare_json }),
            ),
            (
                UsageVendorState::Failed {
                    next_attempt_in_secs: 0,
                    detail: None,
                },
                serde_json::json!({ "kind": "Failed", "next_attempt_in_secs": 0, "detail": null }),
            ),
            (
                UsageVendorState::Failed {
                    next_attempt_in_secs: 90,
                    detail: full(),
                },
                serde_json::json!({
                    "kind": "Failed", "next_attempt_in_secs": 90, "detail": full_json
                }),
            ),
            (
                UsageVendorState::Rejected {
                    retry_in_secs: 600,
                    detail: None,
                },
                serde_json::json!({ "kind": "Rejected", "retry_in_secs": 600, "detail": null }),
            ),
            (
                UsageVendorState::Rejected {
                    retry_in_secs: 600,
                    detail: full(),
                },
                serde_json::json!({ "kind": "Rejected", "retry_in_secs": 600, "detail": full_json }),
            ),
        ];
        for (state, want) in cases {
            let snap = usage_snapshot(state);
            let json = serde_json::to_value(&snap).expect("serialize");
            assert_eq!(json["state"], want);
            assert_eq!(
                serde_json::from_value::<UsageLimitSnapshot>(json).expect("deserialize"),
                snap
            );
        }
    }

    /// A packet without the `detail` key (a daemon from before this field) reads as `None` — so a single state does not collapse the whole snapshot
    /// with a deserialization failure.
    #[test]
    fn usage_vendor_state_without_detail_key_reads_as_none() {
        let cases = [
            (
                r#"{ "kind": "NotInstalled" }"#,
                UsageVendorState::NotInstalled { detail: None },
            ),
            (
                r#"{ "kind": "NeedsLogin" }"#,
                UsageVendorState::NeedsLogin { detail: None },
            ),
            (
                r#"{ "kind": "Unavailable" }"#,
                UsageVendorState::Unavailable { detail: None },
            ),
            (
                r#"{ "kind": "Failed", "next_attempt_in_secs": 5 }"#,
                UsageVendorState::Failed {
                    next_attempt_in_secs: 5,
                    detail: None,
                },
            ),
            (
                r#"{ "kind": "Rejected", "retry_in_secs": 7 }"#,
                UsageVendorState::Rejected {
                    retry_in_secs: 7,
                    detail: None,
                },
            ),
        ];
        for (raw, want) in cases {
            assert_eq!(
                serde_json::from_str::<UsageVendorState>(raw).expect(raw),
                want
            );
        }
    }

    /// `{:?}` does not emit the original — it emits only the char count (not bytes). Wire·comparison are the original as is.
    #[test]
    fn usage_state_detail_debug_hides_upstream_text() {
        let secret = "token=sk-비밀값"; // hangul-ok
        let d = detail(Some(1), Some(secret));
        let dbg = format!("{d:?}");
        assert!(!dbg.contains("sk-") && !dbg.contains("비밀"), "{dbg}"); // hangul-ok
        assert!(
            dbg.contains(&format!("<{} chars>", secret.chars().count())),
            "{dbg}"
        );
        assert!(
            dbg.contains("rpc_error") && dbg.contains("Some(1)"),
            "{dbg}"
        );
        let nested = format!(
            "{:#?}",
            UsageVendorState::NeedsLogin {
                detail: Some(d.clone())
            }
        );
        assert!(!nested.contains("sk-"), "{nested}");
        assert_eq!(
            serde_json::to_value(&d).expect("serialize")["upstream"],
            secret
        );
        assert_eq!(
            format!("{:?}", detail(None, None)),
            r#"UsageStateDetail { kind: "rpc_error", code: None, upstream: None }"#
        );
    }

    /// The TS shape the frontend receives — if u64·i64 fields leak as `bigint`, the type diverges from code receiving a JSON number.
    #[test]
    fn usage_ts_shapes_use_number_for_every_time_field() {
        let decls = [
            UsageLimitSnapshot::decl(),
            UsageWindow::decl(),
            UsageScopedWindow::decl(),
            UsageVendorState::decl(),
            UsageStateDetail::decl(),
        ];
        for decl in &decls {
            assert!(!decl.contains("bigint"), "{decl}");
        }
        let snap = UsageLimitSnapshot::decl();
        assert!(snap.contains("revision: number,"), "{snap}");
        // `served` lives only inside the daemon — the wire shape must have neither the field nor a type reference.
        assert!(
            !snap.contains("served") && !snap.contains("UsageServed"),
            "{snap}"
        );
        let window = UsageWindow::decl();
        // The source of a value (pickup·active query) is not visible to the user — the wire window must have no field for it.
        assert!(!window.contains("source"), "{window}");
        assert!(window.contains("resets_at: number | null"), "{window}");
        assert!(window.contains("age_secs: number,"), "{window}");
        assert_eq!(
            UsageStateDetail::inline(),
            "{ kind: string, code: number | null, upstream: string | null, }"
        );
        // The states are measured in full — a text (`string`) field is allowed only as `UsageStateDetail` and appears here only by name.
        // If a bare `string` field sneaks into a state variant, this breaks.
        assert_eq!(
            UsageVendorState::inline(),
            concat!(
                r#"{ "kind": "Ready" } | "#,
                r#"{ "kind": "NotInstalled", detail: UsageStateDetail | null, } | "#,
                r#"{ "kind": "NeedsLogin", detail: UsageStateDetail | null, } | "#,
                r#"{ "kind": "Unavailable", detail: UsageStateDetail | null, } | "#,
                r#"{ "kind": "Failed", next_attempt_in_secs: number, "#,
                r#"detail: UsageStateDetail | null, } | "#,
                r#"{ "kind": "Rejected", retry_in_secs: number, detail: UsageStateDetail | null, }"#,
            )
        );
    }
}
