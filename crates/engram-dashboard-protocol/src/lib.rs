//! # engram-dashboard-protocol — boundary contract (linchpin)
//!
//! The wire protocol between the UI (frontend) ↔ daemon (daemon-only single path — ADR-0029 removed embedded mode,
//! ADR-0036 src-tauri relays as the single daemon client without a direct window↔daemon link).
//! Two segments carry these types:
//!   - frontend ↔ src-tauri (local IPC): Tauri invoke (commands) · events (broadcast) · Channel (output frames, raw bytes).
//!   - src-tauri(DaemonClient) ↔ daemon: 127.0.0.1 WS.
//!
//! ## Design rationale (daemon-design.md)
//! - §1-1 single WS connection·single receive loop (no lane split) — control and output on the same connection.
//! - §1-2 wire codec: **output hot path = custom fixed-header binary frame** (`codec`), control = JSON.
//!
//! ## No Tauri imports. No domain logic (pure types·serde·codec only).
//!
//! ## No workspace crate dependencies (ADR-0272 「Impact」)
//! Gate = the `ci.yml` step `protocol gate 1` (direct workspace deps = exactly self).
//!
//! ## ★ Name collision note:
//! This crate's [`AgentCommand`] = **client→daemon request envelope** (design §3 naming).
//! The existing `agent(profile.rs)::AgentCommand` = **spawn kind** (Claude/Shell) — its wire mirror is not this enum
//! but [`AgentSpawnCommand`]. Calling it "AgentCommand" without the crate leaves the meaning undetermined.
//!
//! ## TS mapping of seq
//! ts-rs maps u64 seq to `bigint` by default, but for consistency with the existing frontend (`PtyEvent.seq: number`)
//! + practical safety within the JSON number limit (2^53) (thousands of years even at tens of thousands of chunks per second), it is pinned to `#[ts(type="number")]`.
//! The binary frame's seq is received with JS `DataView.getBigUint64`, so it is unaffected (only the JSON path is number).

mod codec;
mod discovery;
mod domain;
pub mod handshake;
mod ids;
mod messages;
// ADR-0231: the placeholder that does not leave empty the seq of an output event that could not be carried — the daemon sink and the shell relay share one set.
mod placeholder;
mod secret;

pub use codec::{
    decode_frame, encode_structured_frame, encode_terminal_frame, peek_frame_header, CodecError,
    DecodedFrame, FrameHeader, FRAME_HEADER_LEN, FRAME_TAG_STRUCTURED_EVENT,
    FRAME_TAG_TERMINAL_BYTES,
};
pub use discovery::DaemonInfo;
pub use domain::{
    AgentBackendKind, AgentFailureKind, AgentInfo, AgentOutputFormat, AgentProfile,
    AgentSpawnCommand, AgentStatus, Capabilities, ControlCaps, EnvelopeFormat, InputCaps,
    ModelCaps, OutputCaps, Preset, RestartPolicy, RestoreOutcome, RestoreReport, SessionCaps,
    SnapshotChunk, UsageLimitSnapshot, UsageScopedWindow, UsageStateDetail, UsageVendorState,
    UsageWindow,
};
pub use ids::{AgentId, OwnerToken, PresetId, ProfileId, RequestId};
pub use messages::{
    AgentCommand, AgentEvent, CommandListEntry, DeliveredCopy, DropCause, OutputChunk,
    QueuedInputCancel, QueuedInputEvent, QueuedInputRow, StructuredEvent, SubscribeAction,
    ToolCategory, ToolOutcome, TurnOutcome,
};
pub use placeholder::{placeholder_error_frame, PLACEHOLDER_ERROR_MESSAGE};
pub use secret::Secret;

/// +1 only on breaking changes (field meaning change·removal) (design decision #6: version handling deferred,
/// for now only the constant exists and is carried in Hello — the mismatch popup guide comes later).
///
/// v2: ListAgents/ListProfiles query responses switched from piggyback matching on the broadcast (AgentListUpdated/ProfileListUpdated)
/// to a dedicated reply (AgentList/ProfileList) with request_id attached + request_id added to Snapshot.
/// The ListAgents/ListProfiles commands also changed from unit→with request_id attached (reply contract change). An old daemon (v1)
/// sends only the old response, so a new client could wait forever, hence it is refused by version mismatch (the auto-restart policy is a separate matter).
///
/// v3: added `AgentCommand::SetEnvelopeFormat` (UI→daemon envelope format switch, ADR-0096). This command is
/// **non-tolerant additive** — even the same v2 daemon cannot decode this variant (unknown externally-tagged
/// key) and it is blocked at deserialize. Then a new client could wait forever for the Ack (the same scenario as the v2 bump
/// reason), so auth's version check (ws.rs) + discovery's version-mismatch refusal force the old daemon to be
/// **refused/restarted instead of reused**.
///
/// v4: codex wiring — **variant added** [`AgentSpawnCommand::Codex`] + new `backend` ([`AgentBackendKind`]) field in `SpawnByCwd`·`CreateProfile`.
/// ★It adds a variant, so it is on the same **non-tolerant** axis as v3★ — `AgentSpawnCommand` is `#[serde(tag = "kind")]`,
/// so an unknown `kind` is not a tolerated unknown **field** but a deserialization **failure**. 「Adding a
/// `#[serde(default)]` field inside an existing variant keeps the version」 (passed that way several times between v2·v3) does not apply
/// to this change. Each of the two directions breaks silently:
///   - **new daemon + old shell**: if even one codex profile is in the registry, decoding fails at `AgentProfile.command`
///     of `ProfileList`, and the failure unit is **the whole response**, so the old shell loses not that one row
///     but **the whole registry**.
///   - **old daemon + new shell**: the new `backend` field is `#[serde(default)]`, so the old daemon **discards** it and launches the old
///     hard-coding (claude·StreamJson) — a person picked the codex terminal create item (`agent.createCodex`), but claude runs behind
///     a node labeled codex. It is exactly the failure the decision of [`AgentBackendKind`] to have no default meant to prevent,
///     but that decision does not reach a daemon that does not know it.
/// ★It comes first in local development, not after release★ — by design the daemon outlives shell rebuilds (even if the shell is rebuilt,
/// it attaches to the daemon that was running as is). So the two combinations above arise routinely during development. This is where the judgment
/// that tried to skip this bump with 「it is pre-release, so there is no counterpart to protect」 was wrong.
/// The bump makes both loud: auth's version check (daemon `link/auth.rs`) + discovery's
/// version-mismatch refusal (`check_acceptable` in the shell's `discovery` module) **refuse/restart instead of reusing** a daemon that does not
/// match. The place that measures that enforcement = the shell `discovery` test's
/// `version_mismatch_live_daemon_errors_without_spawn`.
///
/// v5: the codex output mode crosses the wire — new `output_format` field in [`AgentSpawnCommand::Codex`] +
/// the daemon boundary carries `CreateProfile.output_format` in the codex branch **instead of discarding it**.
/// ★By shape alone it is a change that need not bump, and that judgment is wrong★ — the new field is `#[serde(default)]` on both sides, so
/// no peer dies at deserialization. So it looks caught by the rule 「adding a `#[serde(default)]` field to an existing variant keeps
/// the version」. ★But the criterion for bumping v4 was not shape but **silent malfunction**★ — the
/// 「old daemon + new shell」 entry of v4 just above is that criterion, and this change reproduces it exactly with only the words changed:
///   - **old daemon + new shell**: a daemon that discarded the mode field in the codex branch launches `Terminal` even though it received that field —
///     a person picked the codex JSON create item (`agent.createCodexJson`), but an interactive TUI runs behind the node with that label. Deserialization
///     succeeds, so **nothing is loud**. It is the same kind of failure v4 prevented, and not bumping here would make the basis for bumping v4
///     an exception at this spot alone.
///   - **new daemon + old shell**: the old shell's `AgentSpawnCommand` tolerates unknown fields, so the registry survives, and only the mode
///     is not visible. This side is silent but **not harmful** (the screen shows less, nothing different runs) — i.e. what forces this
///     bump is the previous entry alone.
/// ★The difference the bump makes★: discovery's `check_acceptable` looks at the version in `daemon.json` and **does not reuse a live daemon
/// that does not match** — the 「old daemon + new shell」 combination above is cut before it gets past the handshake.
/// The place that measures that enforcement is the same as the v4 entry (`version_mismatch_live_daemon_errors_without_spawn` of the shell `discovery` test).
/// (user decision 2026-09-13)
///
/// v6: queued input list — added `AgentCommand::ListQueuedInputs`·`AgentCommand::CancelQueuedInput` + dedicated
/// replies `AgentEvent::QueuedInputs` (rows · `as_of_seq` · incarnation marker · `stopped_after_error`)·
/// `AgentEvent::QueuedInputCancelReply` (ADR-0231). It adds command variants, so it is **non-tolerant additive** like v3
/// — an old daemon cannot decode the unknown externally-tagged key, goes to the command parse failure branch, and there
/// answers with **`Error{request_id: None}`**. With no correlation key, the new shell's waiting request finds no pair and hangs until the shell's reply
/// cap (the silent malfunction v3 recorded as 「the new client waits forever for the Ack」). So auth's
/// version check + discovery's version-mismatch refusal force the old daemon to be **refused/restarted instead of reused**
/// (the place that measures it = `version_mismatch_live_daemon_errors_without_spawn`, the same as the v4 entry).
/// ★Shipped in the same edition but not a bump reason★: [`StructuredEvent::QueuedInput`] (one direction daemon→shell — see that
/// variant's doc) and the `stopped_after_error` field of the list response. What forces the bump is the command axis alone. (user approval —
/// ADR-0231)
///
/// v7: usage limit slot (TRD S21 usage-limit-slot §1-6) — commands `AgentCommand::UsageSubscribe` (replace the whole subscription set
/// · no answer)·`AgentCommand::RefreshUsageLimits` (⟳ · answer = existing `Ack`) + `AgentEvent::UsageLimitsUpdated` going to subscribed
/// connections. It adds command variants, so it is **non-tolerant additive** like v3·v6 — with old daemon +
/// new shell, the old daemon cannot decode the two commands and answers with the parse failure `Error{request_id: None}`. That error has no
/// correlation key, so the shell does not know which command failed — a subscribe failure leaves the slot empty with no way to tell, and ⟳ hangs until
/// the shell's reply cap. Both fall under v4's criterion (silent malfunction). So auth's version check + discovery's
/// version-mismatch refusal force the old daemon to be **refused/restarted instead of reused** (the place that measures it = same as the v4 entry).
/// ★Shipped in the same edition but not a bump reason★: the usage snapshot shape (`UsageLimitSnapshot`'s `revision` ·
/// `detail` of abnormal states) and the event's `subscribed` field. Before this branch no variant carried the usage wire, so
/// there is no old peer reading that shape. What forces the bump is the command axis alone.
///
/// ★There is also a change that was measured against this criterion and **not bumped**★ — adding [`StructuredEvent::TurnEnd`],
/// which carries the turn boundary + outcome. The two-direction analysis and its conclusion live in that variant's own doc —
/// not repeated here. **The criterion's home is this spot, that judgment's home is over there.**
pub const PROTOCOL_VERSION: u32 = 7;
