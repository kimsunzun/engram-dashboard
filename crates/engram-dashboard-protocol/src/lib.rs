//! # engram-dashboard-protocol — boundary contract (linchpin)
//!
//! The wire protocol between UI ↔ daemon (daemon-only single path — ADR-0029; ADR-0036 src-tauri relays as the single daemon client). Two
//!   segments carry these types:
//!   - frontend ↔ src-tauri (local IPC): Tauri invoke (commands) · events (broadcast) · Channel (output frames, raw bytes).
//!   - src-tauri(DaemonClient) ↔ daemon: 127.0.0.1 WS.
//!
//! ## Design (daemon-design.md)
//! - §1-1 single WS connection·single receive loop (no lane split).
//! - §1-2 **output hot path = custom fixed-header binary frame** (`codec`), control = JSON.
//!
//! ## No Tauri imports, no domain logic (pure types·serde·codec only); no workspace crate dependencies (ADR-0272 「Impact」; gate = `ci.yml`
//!   step `protocol gate 1`).
//!
//! ## ★ Name collision note
//! This crate's [`AgentCommand`] = **client→daemon request envelope** (design §3). The agent crate's `profile.rs::AgentCommand` = **spawn
//!   kind** (Claude/Shell);
//! its wire mirror is [`AgentSpawnCommand`].
//!
//! ## TS mapping of seq
//! u64 seq is pinned to `#[ts(type="number")]` (ts-rs default is `bigint`) for consistency with the frontend (`PtyEvent.seq: number`) and
//!   safety within the JSON number limit (2^53).
//! The binary frame's seq is read with JS `DataView.getBigUint64`, so only the JSON path is number.

mod codec;
mod discovery;
mod domain;
pub mod handshake;
mod ids;
mod messages;
// ADR-0394: the peer state the app's PC list slot shows.
mod peer;
// ADR-0231: placeholder for the seq of an output event that could not be carried — the daemon sink and the shell relay share one set.
mod placeholder;
mod secret;

pub use codec::{
    decode_frame, encode_structured_frame, encode_terminal_frame, peek_frame_header, CodecError,
    DecodedFrame, FrameHeader, FRAME_HEADER_LEN, FRAME_TAG_STRUCTURED_EVENT,
    FRAME_TAG_TERMINAL_BYTES,
};
pub use discovery::{token_file_name, DaemonInfo};
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
pub use peer::{
    PeerState, PeerStateAgent, PeerStateKnock, PeerStateNetwork, PeerStatePc, PeerStateRequest,
};
pub use placeholder::{placeholder_error_frame, PLACEHOLDER_ERROR_MESSAGE};
pub use secret::Secret;

/// +1 only on breaking changes (field meaning change·removal) (design decision #6: version handling deferred; for now only the constant exists, carried in Hello).
///
/// ★Bump criterion = silent malfunction★, not shape. Adding a command or spawn-kind variant is **non-tolerant additive**: an old daemon cannot decode the unknown
/// externally-tagged key / `kind`, and the failure is the whole response or an uncorrelated `Error{request_id: None}` the shell cannot match (it then waits forever).
/// The daemon outlives shell rebuilds, so old-daemon + new-shell arises routinely in development. The bump is enforced by auth's version check (daemon `link/auth.rs`)
/// + discovery's `check_acceptable` (shell `discovery`), which **refuse/restart instead of reusing** a mismatched daemon; measured by the shell `discovery` test
/// `version_mismatch_live_daemon_errors_without_spawn`.
///
/// - v2: ListAgents/ListProfiles replies became dedicated (AgentList/ProfileList) with request_id; request_id added to Snapshot and to the ListAgents/ListProfiles commands.
///   An old v1 daemon answers only the old shape, so a new client could wait forever.
/// - v3: `AgentCommand::SetEnvelopeFormat` (ADR-0096) — non-tolerant additive; the Ack would never come.
/// - v4: codex wiring — [`AgentSpawnCommand::Codex`] variant + `backend` ([`AgentBackendKind`]) in `SpawnByCwd`·`CreateProfile`. A variant add under `#[serde(tag = "kind")]`
///   is a deserialization failure, so "a `#[serde(default)]` field in an existing variant keeps the version" does not apply. New daemon + old shell: one codex profile fails
///   `AgentProfile.command` decoding of `ProfileList` — the **whole registry** is lost. Old daemon + new shell: `backend` is discarded and claude·StreamJson launches
///   behind a node labeled codex (why [`AgentBackendKind`] has no default).
/// - v5: `output_format` in [`AgentSpawnCommand::Codex`], carried by `CreateProfile.output_format` in the codex branch. Both sides `#[serde(default)]`, yet it bumps under the
///   criterion: old daemon + new shell launches `Terminal` instead of codex JSON with nothing loud. (New daemon + old shell only shows less — silent but harmless; not a reason.)
/// - v6: queued input — `AgentCommand::ListQueuedInputs`·`CancelQueuedInput` + replies `AgentEvent::QueuedInputs`·`QueuedInputCancelReply` (ADR-0231). Command axis: an old daemon
///   answers `Error{request_id: None}` and the new shell's request hangs until its reply cap. Not a reason: [`StructuredEvent::QueuedInput`] (daemon→shell only) and `stopped_after_error`.
/// - v7: usage limit slot (TRD S21 usage-limit-slot §1-6) — `AgentCommand::UsageSubscribe` (replaces the whole subscription set, no answer)·`RefreshUsageLimits` (answer = `Ack`)
///   + `AgentEvent::UsageLimitsUpdated`. Command axis: an old daemon's uncorrelated parse-failure `Error` leaves a subscribe failure invisible and ⟳ hanging.
///   Not a reason: the snapshot shape (`UsageLimitSnapshot`'s `revision` · `detail`) and `subscribed`.
/// - v8: the shell token left `daemon.json` for the owner-only per-account file ([`token_file_name`]) and the record gained the
///   owner account (ADR-0395). Discovery axis: a new shell reading an old daemon's record finds no token file — the bump makes it take
///   the version-mismatch path instead (the old record still parses: the new type ignores its `token`). The other direction is not
///   covered by the bump: an old shell's `DaemonInfo` requires `token` (no `#[serde(default)]`), so a new record does not parse for it
///   and it takes its "not ready yet" path until its discovery times out.
/// - v9: the app's PC list slot (ADR-0394) — `AgentCommand::PeerSubscribe` + `AgentEvent::PeerStateUpdated`. Development axis:
///   a daemon built before them meets a rebuilt shell routinely, answers the subscribe with an uncorrelated `Error` the shell only
///   logs, and never broadcasts — the slot would stay silently stale after its one `peer.list` read on mount. The bump sends that pair
///   down the version-mismatch path instead. Not a reason: the additive question fields of `PeerStateRequest` and the directory fields of `PeerStatePc` (`#[serde(default)]`).
///   Pulled in: the pre-S38 Korean placeholder marker the frontend still accepted (`LEGACY_PLACEHOLDER_ERROR_MESSAGE`) is gone — a
///   daemon old enough to send it is refused by this version (ADR-0383 consequence).
///
/// Measured against this criterion and **not bumped**: [`StructuredEvent::TurnEnd`] — the analysis lives in that variant's doc.
/// [`AgentCommand::SetUiLanguage`] — fire-and-forget, so nothing waits on it: an old daemon cannot decode it and answers an uncorrelated
/// `Error` the shell only logs, and an old daemon has no dialogs for the language to reach; a new daemon behind an old shell is never told
/// and shows its dialogs in English, the documented default.
pub const PROTOCOL_VERSION: u32 = 9;
