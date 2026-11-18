//! wire messages — client→daemon [`AgentCommand`], daemon→client [`AgentEvent`].
//! Both are externally-tagged JSON (serde default).

use serde_json::value::RawValue;
use ts_rs::TS;

use crate::domain::{
    AgentBackendKind, AgentInfo, AgentOutputFormat, AgentProfile, AgentStatus, Capabilities,
    EnvelopeFormat, Preset, RestoreReport, SnapshotChunk, UsageLimitSnapshot,
};
use crate::ids::{AgentId, OwnerToken, PresetId, ProfileId, RequestId};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
// ★There is no `Auth` variant here (ADR-0129 0-4)★: the auth frame used only as the first frame after connecting is a separate type,
//   `handshake::AuthFrame` — token auth is the decision "should this socket live", so the network layer that makes that decision
//   must be able to decode that frame without knowing the agent vocabulary (this enum) as a type (ADR-0129 decision 1). The frame is
//   `{"Auth":{"token":"…","protocol_version":N}}`, and the canonical test of that shape is the golden JSON of that module.
//   ★Do not put it back here★: if two definitions coexist they silently diverge. That is also why the frontend binding (`bindings/AgentCommand.ts`)
//   has no `Auth` (the frontend `wsTransport` builds it as an object literal without a type in the first place).
//
// ★Name collision★ — agent `profile::AgentCommand` means something different (the program a profile launches).
//   Its wire mirror is not this enum but `AgentSpawnCommand`. Calling it
//   "AgentCommand" without the crate leaves the meaning undetermined.
// ★★The **call contract** of the activation set (`Spawn`·`SpawnByCwd`·`SpawnProfile`) — what the client must keep★★
//   The daemon **takes these three off** that connection's arrival-order lane and runs them **as separate tasks** (activation is a branch
//   that waits until its outcome, so keeping it in the lane delays every other command of that connection meanwhile — the daemon crate's
//   `connection_core::dispatch_order`). This contract is the price:
//   ① **Do not send anything addressed to that agent id before receiving the activation's answer (`Spawned`/`Error`).**
//      If `WriteStdin(X)` is sent right after `SpawnProfile(X)` on the same socket, the latter runs first and
//      fails with 「agent not found」. The answer carries the request's `request_id` as is, so the client can
//      establish the correlation.
//   ② **Do not send the activation of the same id again before receiving the answer.** The two requests enter the daemon's check/register window
//      together. That window was not created by this batch but pre-existed (the daemon's `SpawnReservation` guard narrows it but
//      does not close it); what this batch changed is only that **a single connection alone can now reach that window**.
//   ★This repository's frontend already runs per this contract★ — `src/api/protocolClient.ts` holds the activation Promise
//   with a `request_id` correlation table, and the call site uses the returned `agent.id` after `await`ing it.
//   ★Why it was not reverted to ordering★: the branch 「queue commands addressed to the same id behind that id's activation」 turns
//   that wait into a stall of the whole connection (activation is typically 2s · backstop 15s), so it would conditionally revive
//   the symptom this batch set out to remove.
pub enum AgentCommand {
    /// Activate by profile id. ★Contract of waiting for the answer = 「the call contract of the activation set」 at the head of this enum★.
    Spawn {
        #[ts(type = "string")]
        profile_id: ProfileId,
        request_id: RequestId,
    },
    Kill {
        #[ts(type = "string")]
        agent_id: AgentId,
        request_id: RequestId,
    },
    /// Interrupts only the in-progress work. The process survives. Terminal mode (PTY) is refused — the terminal receives keys directly (ADR-0245).
    Interrupt {
        #[ts(type = "string")]
        agent_id: AgentId,
        request_id: RequestId,
    },
    WriteStdin {
        #[ts(type = "string")]
        agent_id: AgentId,
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        data: Vec<u8>,
        request_id: RequestId,
    },
    /// viewport_id = which view among the multi-views made the request (for the ControlLease decision).
    Resize {
        #[ts(type = "string")]
        agent_id: AgentId,
        cols: u16,
        rows: u16,
        viewport_id: Option<String>,
    },
    /// Reconnect resume with epoch/after_seq (design §1-3).
    /// Both None = a new subscription that wants to receive from the start (from the oldest).
    Subscribe {
        #[ts(type = "string")]
        agent_id: AgentId,
        epoch: Option<u32>,
        #[ts(type = "number | null")]
        after_seq: Option<u64>,
    },
    Unsubscribe {
        #[ts(type = "string")]
        agent_id: AgentId,
    },
    /// Input lease acquire request (prevents input collisions between multiple viewers, Zellij explicit lease model). If the lease is empty,
    /// this connection takes the input right. If another connection already holds it, Error. §5: the LLM also takes the right with this command.
    AcquireInput {
        #[ts(type = "string")]
        agent_id: AgentId,
        request_id: RequestId,
    },
    /// Only the holder can release it (Error if not the holder). After release anyone can acquire again.
    ReleaseInput {
        #[ts(type = "string")]
        agent_id: AgentId,
        request_id: RequestId,
    },
    /// Query the whole agent list (the daemon also auto-pushes right after connecting, but explicit queries are allowed too).
    /// The response is [`AgentEvent::AgentList`] with request_id attached (dedicated reply).
    ListAgents { request_id: RequestId },
    /// Daemon shutdown (§5 LLM control). force=true shuts down even with active agents, kill_agents=true cleans them up together.
    StopDaemon {
        force: bool,
        kill_agents: bool,
        request_id: RequestId,
    },

    // ── Profile CRUD + ad-hoc spawn (phase4 step 1) ───────────────────────────────────
    /// ad-hoc spawn with only cwd and backend — no profile made in advance by the caller is needed. It creates a profile on the spot
    /// (auto_restore=false on creation) and does a Fresh spawn, and the spawn path registers · persists that profile in the
    /// registry.
    ///
    /// `backend` is `#[serde(default)]`, so **packets from old clients still deserialize** (PROTOCOL_VERSION
    /// kept — it added a field to an existing variant, not a variant to the envelope). Instead, **the handler refuses
    /// its absence** — this is the actual form of the decision not to have a silent default backend (see `AgentBackendKind`).
    ///
    /// ★Contract of waiting for the answer = 「the call contract of the activation set」 at the head of this enum★.
    SpawnByCwd {
        cwd: String,
        #[serde(default)]
        backend: Option<AgentBackendKind>,
        request_id: RequestId,
    },

    /// The response is [`AgentEvent::ProfileList`] with request_id attached (dedicated reply).
    ListProfiles { request_id: RequestId },

    /// Create a profile (does not spawn — register · persist only). ※No credentials in env (persisted in plain text).
    CreateProfile {
        name: String,
        cwd: String,
        extra_args: Vec<String>,
        env: Vec<(String, String)>,
        auto_restore: bool,
        /// `#[serde(default)]`, so an old frontend/wire without this field is absorbed as Terminal (existing behavior unchanged,
        /// PROTOCOL_VERSION kept — the same additive·tolerant approach as sibling OutputCaps.structured).
        /// ★Both backends read it★ — claude splits PTY/headless NDJSON, codex splits interactive TUI/resident JSON
        /// server (`codex app-server`). codex coming to read this value is
        /// the reason for [`crate::PROTOCOL_VERSION`] v5 — a daemon that discarded the value would launch an interactive TUI behind the 「Codex JSON」 label
        /// while still deserializing successfully, so the version gate is the only thing that catches that mismatch.
        #[serde(default)]
        output_format: AgentOutputFormat,
        /// Absent = error (the same contract as sibling `SpawnByCwd.backend` — see `AgentBackendKind`).
        #[serde(default)]
        backend: Option<AgentBackendKind>,
        request_id: RequestId,
    },

    DeleteProfile {
        #[ts(type = "string")]
        profile_id: ProfileId,
        request_id: RequestId,
    },

    /// resume=true resumes the existing session (claude `--resume`).
    ///
    /// ★Contract of waiting for the answer = 「the call contract of the activation set」 at the head of this enum★.
    SpawnProfile {
        #[ts(type = "string")]
        profile_id: ProfileId,
        resume: bool,
        request_id: RequestId,
    },

    SetProfileAutoRestore {
        #[ts(type = "string")]
        profile_id: ProfileId,
        auto_restore: bool,
        request_id: RequestId,
    },

    /// Set/clear the profile display name override (ADR-0061 enrichment — tree rename). `name=Some` → store override,
    /// `None` → clear (back to the cwd basename derivation). ★Normalization is the responsibility of the daemon's store gate (`AgentManager::try_rename_agent`)★
    /// — trimming both ends and deciding "only whitespace left means no override" are finished there **before** the name-uniqueness
    /// decision. So even if `" bob "` arrives as is, it is settled as a `bob` request, and the gate handles resubmission of the same request idempotently
    /// (no suffix number consumed). The frontend may trim before sending, but that is UX convenience, not the contract.
    /// Error for an unknown id (same shape as SetProfileAutoRestore). After success, [`AgentEvent::ProfileListUpdated`]
    /// broadcast (no optimistic update — every window synced).
    RenameProfile {
        #[ts(type = "string")]
        profile_id: ProfileId,
        #[ts(type = "string | null")]
        name: Option<String>,
        request_id: RequestId,
    },

    /// Set/clear the tree parent (ADR-0072 hierarchy reparent). `parent_id=Some(pid)` → child becomes a child of pid
    /// (one level of nesting), `None` → promoted to root. Validation (self-parent·nonexistent parent·one-level cap·no two levels) is done by the daemon
    /// in one critical section with `ProfileRegistry::try_reparent` — Error on violation, Ack +
    /// [`AgentEvent::ProfileListUpdated`] broadcast on success (same shape as RenameProfile — every window synced, no optimistic update).
    /// Per §5, the LLM/user builds the tree with the same command (human dragging is an auxiliary input).
    ReparentProfile {
        #[ts(type = "string")]
        child_id: ProfileId,
        #[ts(type = "string | null")]
        parent_id: Option<ProfileId>,
        request_id: RequestId,
    },

    /// The response is [`AgentEvent::Snapshot`].
    /// (A one-time query separate from Subscribe replay.)
    GetSnapshot {
        #[ts(type = "string")]
        agent_id: AgentId,
        request_id: RequestId,
    },

    // ── Preset CRUD (ADR-0061) ──────────────────────────────────────────────────────
    // Preset = a pre-spawn "cwd bookmark" (not an instance). The daemon solely owns presets.json and
    // does CRUD only over the wire.
    /// The response is the dedicated reply [`AgentEvent::PresetList`] with request_id attached (to the requesting connection only).
    ListPresets { request_id: RequestId },

    /// Create a preset (register · persist only — does not spawn). The daemon normalizes the cwd (dunce::canonicalize) before storing.
    /// The name is not stored (derived from the cwd basename — ADR-0061). After success, [`AgentEvent::PresetListUpdated`] broadcast.
    CreatePreset { cwd: String, request_id: RequestId },

    /// Delete a preset (unregister · persist). ★Deleting a preset ≠ ending an agent★ (ADR-0061) — agents already
    /// spawned from that preset live on regardless. No-op for an unknown id. After success, [`AgentEvent::PresetListUpdated`] broadcast.
    DeletePreset {
        #[ts(type = "string")]
        preset_id: PresetId,
        request_id: RequestId,
    },

    /// Set/clear the preset display name override (ADR-0061 enrichment). `name=Some` → store override, `None` → clear
    /// (back to the cwd basename derivation). trim · empty-string refusal · skip-if-unchanged are handled by the frontend right before confirming — only a valid value
    /// or an explicit None arrives here. No-op for an unknown id (Ack, same shape as DeletePreset). After success, [`AgentEvent::PresetListUpdated`]
    /// broadcast (no optimistic update — every window synced, ADR-0061 invariant).
    RenamePreset {
        #[ts(type = "string")]
        preset_id: PresetId,
        #[ts(type = "string | null")]
        name: Option<String>,
        request_id: RequestId,
    },

    /// Global envelope format switch (ADR-0096) — switches the A→B message envelope to colon/xml. The daemon holds **one global
    /// state**, and every later `wrap_message` assembly reads that value (ADR-0086 single wrap point
    /// invariant kept — the state is only an input, assembly is still in one place). ★Steering surface only★: this command arrives only via the path where the src-tauri Tauri
    /// command `set_envelope_format` forwards it to the daemon — it is not exposed on the worker MCP channel
    /// (a managed agent must not change the team-wide format, ADR-0096 decision 3 · ADR-0094 least privilege).
    /// No persistence (reset on daemon restart — backlog).
    SetEnvelopeFormat {
        format: EnvelopeFormat,
        request_id: RequestId,
    },

    // ── Command bus registration wire (ADR-0155/0156 · TRD §3-7) ───────────────────────────────
    // The daemon dispatch reflects these three into the owner registry and answers with [`AgentEvent::CommandList`]
    //   (`connection_core.rs`). The sender of the two registrations is the shell — the full `RegisterCommands` per connection
    //   (`daemon_client/connection.rs`), `UpdateCommands` when the webview screen list changes (`commands/view_bus.rs`).
    //
    // ★Do not put `#[serde(default)]` on `request_id`★ — this crate's [`RequestId::default`]
    //   **mints a new v4** (`ids.rs`). If a default were allowed, a packet missing that field would get, instead of refusal, **a new correlation key
    //   that pairs with nothing**, and the reply would never attach (the same-named type on the tool crate side has no `Default`
    //   at all, the exact opposite — the tool crate's `envelope.rs` records why).
    /// The owner puts its full set of declarations **at the moment it attaches**, in one shot (TRD §3-7 clause 1). No round trip per name;
    /// the full set is resent on every reconnect.
    ///
    /// ★Do not expect a handover through `owner` — this field is **an advertisement**★: the daemon derives the registry owner from **the connection the packet
    /// came on** and does not use this field (the contract that identity is the connection, not the envelope — TRD §4-⑧).
    /// So even if the sender prepares a fixed `owner` string, nothing is taken over through that value.
    /// The handover happens **last-wins per name** — when the new reconnected connection puts the same name again,
    /// that name's owner moves to the new connection (TRD §3-7 clauses 1·4).
    ///
    /// ★`decls` is a raw JSON field (ADR-0272 decision 3)★ — the bytes are the command bus tool crate's registration item list
    /// (an array of `CommandDecl` — shape in the TS field below) serialized as is, and this crate does not know that structure. The whole
    /// list is one field — the registry accepts or bounces the packet as a whole, so there is nothing to split per item. Wrapping is at the shell edge,
    /// unwrapping at the daemon edge (putting the agent-reference field `agent_args` into the registry is part of that unwrapping too — ADR-0308).
    /// Like the envelope field of [`AgentCommand::Command`], **a list with the wrong shape still decodes as a packet** — handling that branch is
    /// the edge's job (ADR-0272 decision 6). The correlation key (`request_id`) is outside the field, so it is read even if the list is broken.
    /// ★The compiler does not check the TS field★ — the guard that compares it with the tool crate's item shape is at the shell edge that wraps the field
    /// (`daemon_client::protocol_state`'s `command_decl_hand_written_ts_matches_rust_shape`).
    ///
    /// An item's `help` is **an opaque string** — it is a violation if the receiver (daemon) parses · validates it or branches on its content
    /// (TRD §3-7 hard constraint). That field being `String` in the tool crate is itself that gate.
    /// `catalog_version` is the sending crate's generation number and is **for diagnostics** — the receiver is wrong if it compares it with its own number
    /// and refuses (TRD §4-①).
    RegisterCommands {
        #[ts(type = "string")]
        owner: OwnerToken,
        #[ts(type = "Array<{ name: string, help: string, agent_args?: Array<string> }>")]
        decls: Box<RawValue>,
        catalog_version: u32,
        request_id: RequestId,
    },

    /// The **diff** while attached — a late-starting feature adds names and a feature turned off removes names
    /// (TRD §3-7 clause 3). The full resend is only [`AgentCommand::RegisterCommands`].
    ///
    /// `added` is the same raw field as `decls` of [`AgentCommand::RegisterCommands`] (ADR-0272 decision 3).
    /// It is intentional that `removed` carries only names — no shape is needed when removing. A removed name is **erased along with its spot**
    /// from the registry (ADR-0150 decision 3) and from then on answers with `UNKNOWN_COMMAND` — if it were left as a trace, an attached owner
    /// could keep changing names and fill its own share cap permanently.
    UpdateCommands {
        #[ts(type = "string")]
        owner: OwnerToken,
        #[ts(type = "Array<{ name: string, help: string, agent_args?: Array<string> }>")]
        added: Box<RawValue>,
        removed: Vec<String>,
        request_id: RequestId,
    },

    /// Query the whole registry. The response is [`AgentEvent::CommandList`] with request_id attached (dedicated reply).
    ListCommands { request_id: RequestId },

    /// A **command request** this client issues — the answer comes as [`AgentEvent::CommandReply`] (ADR-0155 decision 3).
    ///
    /// ★The mirror image of [`AgentEvent::CommandRequest`], and the envelope field is **the same**★ — the same vocabulary flows in two directions
    /// and **which connection it was written to is the direction** (TRD §3-2). So there is no direction field and no per-direction envelope.
    ///
    /// ★`envelope` is a raw JSON field (ADR-0272 decision 3)★ — the bytes are the command bus tool crate's envelope
    /// (`CommandEnvelope` — shape in the TS field below) serialized as is, and this crate does not know that structure. Unwrapping it into an envelope
    /// is the shell · daemon edge. When decoding the packet, the only thing checked in this field is 「is it valid JSON」, so **an envelope with the wrong shape
    /// still decodes as a packet** — handling that branch is the edge's job (ADR-0272 decision 6).
    ///
    /// ★The correlation key is inside the envelope, so this variant has no `request_id` field★ — it looks different from its siblings, but it is a pending-match
    /// target: the shell's pairing (`daemon_client::protocol_state`'s `command_request_id`) unwraps the envelope and reads the number
    /// (ADR-0272 decision 4). Its pair is the `CommandReply` branch of `event_reply_request_id` in the same module — **if only one of the two
    /// is `Some`, the round trip does not close.**
    ///
    /// The envelope's `owner` is the **destination** token, not the sender — the final targeting is done by the daemon with its own registry
    /// (ADR-0154). The envelope's `args` are passed through without the daemon interpreting them (ADR-0081 「keep the daemon opaque」) — except that fields the owner
    /// marked as agent references at registration are rewritten to ids by the daemon before relaying (ADR-0308).
    Command {
        #[ts(
            type = "{ name: string, request_id: string, owner: string, proto_ver: number, args: unknown }"
        )]
        envelope: Box<RawValue>,
    },

    /// The **outcome** of a command the daemon delivered ([`AgentEvent::CommandRequest`]).
    ///
    /// ★This is a reply, not a request★ — so it has no `request_id` field of its own, and the correlation key is inside `reply`
    /// (the very key the envelope received — not minted anew per hop, ADR-0081 「preserve request_id round trip」).
    /// The shell's pairing (`daemon_client::protocol_state`'s `command_request_id`) giving `None` to this variant
    /// is the expression of that fact: putting it into pending matching would leave nothing to wake, so it would stay pending forever.
    /// ★It pairs with the absence of a direction field★ — the same envelope vocabulary flows in two directions, and which connection it was written to is the direction
    /// (TRD §3-2 · tool crate `CommandEnvelope::owner` comment).
    ///
    /// ★`reply` is a raw JSON field (ADR-0272 decision 3)★ — the bytes are the command bus tool crate's reply (`CommandReply` —
    /// shape in the TS field below) serialized as is, and this crate does not know that structure. Unwrapping it into a reply is the shell · daemon
    /// edge. Like the envelope field of [`AgentCommand::Command`], **a reply with the wrong shape still decodes as a packet** — handling that branch
    /// is the edge's job (ADR-0272 decision 6).
    ///
    /// ★The TS field records **what the edge accepts when it unwraps a reply** — recording only what it emits would be false★: all three error
    /// fields allow omission · `null` (`CommandError`'s `RawError` is three `Option<String>`) and fields outside the contract
    /// pass through as is (`#[serde(flatten)] extra` — `deny_unknown_fields` is not attached because it breaks additive evolution). Writing them as required
    /// strings would make **valid input inexpressible in TS.** That tolerance is real in the sending-back direction too —
    /// when a relay hop reserializes the raw text it received, it goes out with some of the three fields missing (its `Serialize` impl). The test measuring that tolerance
    /// is at the daemon edge that unwraps the reply (`connection_core`'s
    /// `the_outcome_slot_accepts_everything_its_ts_type_advertises`).
    CommandOutcome {
        #[ts(
            type = "{ request_id: string, outcome: { Ok: unknown } | { Err: { code?: string | null, message?: string | null, retry?: string | null, [key: string]: unknown } } }"
        )]
        reply: Box<RawValue>,
    },

    // ── Queued input list (ADR-0231) ─────────────────────────────────────────────────────
    // The WS pair of bus `agent.listQueuedInputs`·`agent.cancelQueuedInput` — the daemon carries the same manager values.
    /// Query the queued input list of the live incarnation. The response is the dedicated reply [`AgentEvent::QueuedInputs`]. ★It does not look at the input lease★
    /// (it is a read). `Error` (`NOT_FOUND: …`) if there is no live session.
    ListQueuedInputs {
        #[ts(type = "string")]
        agent_id: AgentId,
        request_id: RequestId,
    },
    /// Cancels one queued input (the same command as the on-screen ✕). The response is the dedicated reply [`AgentEvent::QueuedInputCancelReply`].
    /// ★The input lease goes through the same check as `WriteStdin`★ — a non-holder is refused with an `Error` of the same wording.
    /// An id not in the list (unknown · already settled · sleeping agent) is `Error` (`NOT_FOUND: …`).
    /// `input_id` is required — no default like 「if absent, the most recent」.
    CancelQueuedInput {
        #[ts(type = "string")]
        agent_id: AgentId,
        input_id: String,
        request_id: RequestId,
    },

    // ── Usage limits (TRD S21 usage-limit-slot §1-6) ──────────────────────────────────────
    /// Replaces this connection's usage subscription set **as a whole** — empty array = unsubscribe; when the connection drops it is the empty set. No answer
    /// (no `request_id`). For each newly added vendor, one [`AgentEvent::UsageLimitsUpdated`] comes to this connection only. Vendor words the daemon
    /// keeps no usage for are discarded.
    /// ★No subscribe/unsubscribe pair★ — if one unsubscribe is lost, the query keeps running. With full replacement, the last one is the
    ///   state. So one connection's replacements are applied in arrival order.
    UsageSubscribe { vendors: Vec<AgentBackendKind> },
    /// ⟳ — query that vendor's usage anew. Answer = [`AgentEvent::Ack`] and it **carries no value** — the value goes to connections that subscribed
    /// to that vendor as [`AgentEvent::UsageLimitsUpdated`] (the sending connection receives it only if it subscribed too).
    /// If a query was started (or joined while in progress), `Ack` comes after waiting for its end up to the daemon's cap (a few seconds). Within the refusal
    /// deadline it comes right away without a query. If the daemon keeps no usage for that vendor, `Error` (this `request_id`).
    /// ★The daemon runs this command outside that connection's arrival-order lane★ — the answer to a later command of the same connection
    ///   can come before this answer.
    RefreshUsageLimits {
        vendor: AgentBackendKind,
        request_id: RequestId,
    },
}

/// One row of the daemon registry's **client projection** (an element of [`AgentEvent::CommandList`]).
///
/// ★It lives here because it is a wire contract, not a bus contract★ — its fields overlap with the tool crate's `CommandDecl` (registration unit),
/// but the direction and owner differ: `CommandDecl` is what the owner puts to the daemon, and this is what the daemon sweeps out of the registry
/// and hands down, so it carries one more field only the registry knows (`available`).
///
/// ★`available` is **always `true` now** and must not be used as a branching basis★ — when the owner disconnects, that name disappears from the registry
/// and is not listed at all (ADR-0150 decision 3), so every listed item is a live registration. `false` is
/// unreachable, and code that decides availability by this field is **a decision with nothing to decide**. The reason the field was not removed is
/// that removing it is itself a wire contract change with nothing gained (TRD §3-7).
/// `help` is the string the owner put, **as is** (the daemon does not open it, so there is no processing either).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct CommandListEntry {
    pub name: String,
    pub help: String,
    pub available: bool,
}

/// One row of the queued input list (an element of [`AgentEvent::QueuedInputs`]) — the same shape · words as the bus `agent.listQueuedInputs` row.
///
/// `state` = `queued` | `sent` | `unconfirmed` | `cancelling`. `sent` = a queued row the link handed to the vendor (reduced state — the reattach
///   reconciliation reads it as a row with the handover mark set · the mark of a cancel-pending row is buried in `cancelling`).
///   ★`unconfirmed` is not a reduced state of the registry★ — it is a mark patched onto an item the link held as acceptance-unknown
///   at query time, so it is not one atom with `as_of_seq`, and the reattach reconciliation reads it as `queued`.
/// ★The word field is not raised to an enum★ — when words grow, an old shell receives just that row with an unknown word
///   instead of failing to read the whole response (the bus row is a string too).
/// `cancel` = carried only on `cancelling` rows (`null` otherwise).
// ADR-0231
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct QueuedInputRow {
    /// Used as is as the `input_id` of [`AgentCommand::CancelQueuedInput`].
    pub id: String,
    pub text: String,
    pub state: String,
    pub cancel: Option<QueuedInputCancel>,
}

/// The two fields of a cancel-pending row — the registry's reduced state as is (both are needed to re-reduce later events on top of the snapshot).
/// `answer` = `none` (no response to our cancel request yet) | `not_removed` (answered that it did not remove it, or the request failed).
/// `vendor_closed` = the vendor close of that id was already seen.
// ADR-0231
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct QueuedInputCancel {
    pub answer: String,
    pub vendor_closed: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum AgentEvent {
    /// Handshake right after connecting.
    Hello {
        protocol_version: u32,
        daemon_version: String,
        /// Daemon-wide capability (per-agent capability is in AgentInfo).
        capabilities: Option<Capabilities>,
    },
    /// Acknowledgement of receiving/handling a side-effect command.
    Ack {
        request_id: RequestId,
    },
    SubscribeAck {
        #[ts(type = "string")]
        agent_id: AgentId,
        action: SubscribeAction,
        current_epoch: u32,
        #[ts(type = "number")]
        oldest_seq: u64,
        #[ts(type = "number")]
        latest_seq: u64,
        /// The head of this replay — the seq actually sent first. If nothing was sent, the next seq to issue for an empty ring,
        /// or `after_seq+1` if there is no tail to resume. ★It is not the dedup basis (「the last seen seq」)★ — the view
        /// uses this value only for the flush starting point `max(last+1, replay_from)` (ADR-0231).
        #[ts(type = "number")]
        replay_from: u64,
        /// Partial loss from being pushed out of the ring (clear+tail).
        truncated: bool,
        /// This replay's incarnation started to resume a stored conversation (the spawn carried a handle to resume) —
        /// ★it does not mean the resume **succeeded**★. It comes from the same subscribe response as `current_epoch`.
        /// An ack from an old daemon without the field reads as `false` (`serde(default)`).
        /// ★[`crate::PROTOCOL_VERSION`] is not bumped for this addition★ — measured against that criterion (a silent **harmful** malfunction),
        ///   both directions fall back to today's behavior:
        ///   - **new daemon + old shell**: the old shell drops the unknown field (no `deny_unknown_fields`) — a resuming slot
        ///     draws the first screen as today.
        ///   - **old daemon + new shell**: no field, so `false` — it also draws the first screen as today.
        ///   No combination runs something different; the feature is just absent. The shell↔webview marker bit has no version axis because the shell and the webview
        ///   ship in one build. (ADR-0226)
        #[serde(default)]
        continues_conversation: bool,
    },
    /// [`AgentCommand::Subscribe`] was **refused** — the failure pair of [`AgentEvent::SubscribeAck`].
    ///
    /// ★Why not send it as `Error` (load-bearing — the reason this variant exists)★: `Subscribe` is a command without
    /// `request_id` (the shell's `protocol_state::command_request_id` is `None`), so its refusal went out as `Error{request_id: None}`,
    /// and that envelope has **no field identifying the owner**. The receiver (src-tauri) could not tell which agent's subscription
    /// broke, so it could not release its single-flight slot; that slot stayed a zombie and **that agent's Subscribe could never go out
    /// again** — after a daemon restart, that agent's output was permanently cut off in every window (measured 2026-08-19).
    /// So the refusal carries `agent_id` to make it correlatable.
    ///
    /// ★Contract★: when this event goes out, **neither `SubscribeAck` nor `ReplayComplete` for that Subscribe
    /// comes** (both are emitted only after the refusal point). The receiver can rely on this and release the slot immediately
    /// safely — no response exists that could arrive later and be misattributed.
    ///
    /// ★`reason` is a human-readable diagnostic string★ — no machine branching (the wording changes without notice). If branching is needed,
    /// add a new code field.
    SubscribeFailed {
        #[ts(type = "string")]
        agent_id: AgentId,
        reason: String,
    },
    /// Low-frequency structured output (TextDelta/Usage/ToolCall etc.).
    Output {
        #[ts(type = "string")]
        agent_id: AgentId,
        epoch: u32,
        #[ts(type = "number")]
        seq: u64,
        chunk: OutputChunk,
    },
    /// End of the replay section — live from here on (the client-side signal of the C4 atomic switch).
    ReplayComplete {
        #[ts(type = "string")]
        agent_id: AgentId,
        epoch: u32,
    },
    /// epoch attached (defends against stale notices from an old session).
    StatusChanged {
        #[ts(type = "string")]
        agent_id: AgentId,
        status: AgentStatus,
        epoch: u32,
    },
    /// Whole-list update (broadcast). The terminal decision is made with this (not status_changed — design invariant).
    AgentListUpdated {
        agents: Vec<AgentInfo>,
    },
    /// ListAgents query response (dedicated reply) — request_id echo to match "the result of my request" exactly.
    /// Same payload as the broadcast AgentListUpdated, but request_id is attached to remove
    /// piggyback matching (pairing with the next arriving message) (same shape as Spawned/Created).
    AgentList {
        request_id: RequestId,
        agents: Vec<AgentInfo>,
    },
    RestoreResult {
        report: RestoreReport,
    },
    /// Input lease state change notice (lets multiple viewers know "it is locked now"). held=true means someone holds it,
    /// false means it is empty (anyone can acquire). The holder conn identifier is not exposed for security (only whether it is locked).
    InputLeaseChanged {
        #[ts(type = "string")]
        agent_id: AgentId,
        held: bool,
    },
    /// Profile list update (broadcast). Auto-pushed after CRUD (create/delete/toggle).
    ProfileListUpdated {
        profiles: Vec<AgentProfile>,
    },
    /// ListProfiles query response (dedicated reply) — request_id echo. Same payload as the broadcast ProfileListUpdated,
    /// but request_id is attached to remove piggyback matching (same shape as Spawned/Created).
    ProfileList {
        request_id: RequestId,
        profiles: Vec<AgentProfile>,
    },

    /// Preset list update (broadcast, ADR-0061). Auto-pushed after CRUD (create/delete).
    PresetListUpdated {
        presets: Vec<Preset>,
    },
    /// ListPresets query response (dedicated reply, ADR-0061) — request_id echo. Same payload as the broadcast PresetListUpdated,
    /// but request_id is attached to remove piggyback matching (same shape as ProfileList).
    PresetList {
        request_id: RequestId,
        presets: Vec<Preset>,
    },

    /// GetSnapshot response (dedicated reply) — the replay buffer snapshot at that moment.
    /// request_id echo to match concurrent queries of the same agent exactly (removes the old agent_id piggyback matching).
    /// Not a broadcast (responds only to the specific request).
    Snapshot {
        request_id: RequestId,
        #[ts(type = "string")]
        agent_id: AgentId,
        chunks: Vec<SnapshotChunk>,
    },

    /// CreateProfile response — the created profile with request_id attached (for DaemonClient's "mine" matching).
    /// Separate from the existing ProfileListUpdated broadcast (that one is for updating every connection's mirror, no request_id).
    Created {
        request_id: RequestId,
        profile: AgentProfile,
    },
    /// SpawnByCwd/SpawnProfile response — the spawned AgentInfo with request_id attached.
    /// Separate from the existing AgentListUpdated broadcast (StatusSink pushes it to every connection, no request_id).
    Spawned {
        request_id: RequestId,
        agent: AgentInfo,
    },

    /// [`AgentCommand::ListCommands`] response (dedicated reply, ADR-0155/0156) — request_id echo.
    /// Not a broadcast (goes only to the requesting connection). The daemon dispatch sweeps the registry and emits this.
    ///
    /// ★Old-shell safety relies only on the fact that it is a "dedicated reply"★ — an old shell does not send `ListCommands`,
    /// so it never receives this reply either, and even if it did, it silently drops an unknown externally-tagged variant key (no error,
    /// no log). **The moment this variant is changed to a broadcast (push without a request)**, every old shell
    /// loses that event without any signal — changing it to a broadcast requires redesigning this safety.
    ///
    /// Names whose owner disconnected are not listed — because they disappeared from the registry (ADR-0150 decision 3). So a name missing
    /// from the list is an answer that merges 「no such name」 and 「the owner is away」 (an accepted loss).
    CommandList {
        request_id: RequestId,
        entries: Vec<CommandListEntry>,
    },

    /// A **command** the daemon delivers addressed to this client (step 2 of ADR-0155 decision 3).
    ///
    /// This variant makes the client a 「daemon command **receiving** peer」 — what ADR-0081 recorded as a 「new capability」,
    /// and that ADR's 3-variant opaque relay envelope was replaced by ADR-0155 with this unified envelope.
    ///
    /// ★The only 「request」 variant of this enum★ — all the rest are notices or replies to commands I sent. So
    /// the receiver splits only this one out of the [`AgentEvent`] consumption flow and hands it to the inbound receiver, and the answer
    /// goes back as [`AgentCommand::CommandOutcome`]. The contract is that the shell's pairing (`daemon_client::protocol_state`'s
    /// `event_reply_request_id`) gives `None` here — with `Some`, the receiver's pending matching would read this
    /// request as 「the reply I was waiting for」 and swallow it (the envelope vanishes without being executed, and the sender hangs
    /// until the deadline).
    ///
    /// `envelope` is the same raw JSON field as [`AgentCommand::Command`] (shape · what the edge unwraps · handling of broken
    /// envelopes — see that variant's comment, ADR-0272). The envelope's `args` are values the daemon passed through without interpreting (ADR-0081 「keep the daemon opaque」) — except that fields the owner marked as agent references at registration are values the daemon rewrote to ids before relaying
    /// (ADR-0308). The envelope's `owner` is the **destination** token, not the sender.
    CommandRequest {
        #[ts(
            type = "{ name: string, request_id: string, owner: string, proto_ver: number, args: unknown }"
        )]
        envelope: Box<RawValue>,
    },

    /// The **reply** to [`AgentCommand::Command`] (dedicated reply, ADR-0155 decision 3) — the correlation key is the `request_id`
    /// inside the `reply` field.
    ///
    /// ★Same fields as [`AgentCommand::CommandOutcome`], only the direction differs★ — this one is the answer to a request I made, so
    /// the shell's `event_reply_request_id` gives `Some`, and that one is an answer I send, so `None`. They have the same shape, so
    /// this is an easy place to get confused, and the only basis for the split is **whether I own that round trip**.
    /// ★Do not change it to a broadcast★ — for the same reason as [`AgentEvent::CommandList`], old-shell safety stands on the
    /// fact that it is a 「dedicated reply」.
    ///
    /// `reply` is the same raw JSON field as [`AgentCommand::CommandOutcome`], and the basis for the tolerance the TS field advertises (three error fields
    /// may be omitted · `null` allowed, fields outside the contract pass through) is written there too — same field, same tolerance (ADR-0272).
    ///
    /// ★A producer exists now★ — the daemon finds the owner in its own registry, delivers, and returns the outcome with this reply
    /// (`engram_dashboard_daemon::command_delivery` · ADR-0154). ★Still, **it does not reach the frontend**★ —
    /// the two pieces below are still empty, and the two must be added **in the same change** (last paragraph).
    ///
    /// The shell correlates: the shell's pairing (`daemon_client::protocol_state`'s `command_request_id`)
    /// gives `Some` to [`AgentCommand::Command`], so `forward_daemon_command` (`src-tauri/src/commands/agent.rs`)
    /// takes the **wait for reply** branch (that function has no variant allowlist — it blocks only `Subscribe`/`Unsubscribe`, and the rest
    /// split on this one `Some`/`None`). When the reply comes, `src-tauri/src/daemon_client/connection.rs` wakes the pending with the same module's
    /// `event_reply_request_id` and returns that event **as is**.
    /// The carrier passes it too: `TauriTransport.send` (`src/api/tauriTransport.ts`) lifts that invoke return into a
    /// control message and puts it into `ProtocolClient.handleEvent`. ★The filter that rebuilds only a few named broadcasts
    /// exists **only in the push direction** — the reply direction passes unfiltered★.
    ///
    /// ★But `handleEvent` (`src/api/protocolClient.ts`) has **no** branch for this variant★ — it passes the last `if`
    /// and ends with nothing done (a silent discard with no else, no warn, no throw).
    /// ★Then that call's promise **never resolves**★: the frontend `sendCommand` sets no timer, and
    /// `invoke` ended in **success**, so `send().catch` is not called either. The only escape hatches left are a connection state transition or
    /// the bulk reject of `close()`. The shell's 30-second reply cap is a device against **the case where no reply comes**, so
    /// it does not reach here (the case where the reply came and was discarded).
    ///
    /// ★So the change that adds **the screen-side producer** (the spot where the webview emits [`AgentCommand::Command`] — 0 cases in `src/` today)
    /// must add the `handleEvent` branch **in the same change** too★ — the same requirement as the shell's two correlation functions holding
    /// only as a pair repeats one layer down. While that producer does not exist, this hole is **unreachable**:
    /// envelopes the daemon delivers are executed by the shell from its own table (`daemon_client::inbound`) and the replies are also
    /// received by the shell, so they do not pass through the webview's pending.
    CommandReply {
        #[ts(
            type = "{ request_id: string, outcome: { Ok: unknown } | { Err: { code?: string | null, message?: string | null, retry?: string | null, [key: string]: unknown } } }"
        )]
        reply: Box<RawValue>,
    },

    /// [`AgentCommand::ListQueuedInputs`] response (dedicated reply) — the same value as the answer of the bus `agent.listQueuedInputs`.
    ///
    /// `inputs` = non-final items only (in list order). `as_of_seq` = the seq of the last list event the row reduced (`null` = this
    /// incarnation has no list event yet) — ★compare only within `epoch`★ (the reattach reconciliation aligns the snapshot and live events by
    /// this seq). `stopped_after_error` = mail is stopped after a turn that ended in error, until the user's next turn succeeds.
    /// ★Do not change it to a broadcast★ — for the same reason as [`AgentEvent::CommandList`], old-shell safety stands on 「dedicated reply」.
    // ADR-0231
    QueuedInputs {
        request_id: RequestId,
        #[ts(type = "string")]
        agent_id: AgentId,
        inputs: Vec<QueuedInputRow>,
        #[ts(type = "number | null")]
        as_of_seq: Option<u64>,
        epoch: u32,
        stopped_after_error: bool,
    },
    /// [`AgentCommand::CancelQueuedInput`] response (dedicated reply). ★It is the answer to the cancel request, not 「it was cancelled」★ —
    /// `outcome` = `cancelled` (withdrawn right away) | `requested` (cancellation requested · it was already cancel-pending — the outcome is decided by the
    /// following list event). That is why the name is not `…Cancelled`. Failures (lease refusal · `NOT_FOUND` · failure to write the cancel line)
    /// come as `Error`, not this variant.
    // ADR-0231
    QueuedInputCancelReply {
        request_id: RequestId,
        #[ts(type = "string")]
        agent_id: AgentId,
        input_id: String,
        outcome: String,
    },

    // ADR-0253
    /// One usage sheet — goes only to connections that subscribed to that vendor ([`AgentCommand::UsageSubscribe`]). Not an answer to a request.
    /// `subscribed` = the receiving connection's subscription set — as of the moment the daemon built this sheet, so it can be one step behind.
    /// ★A sheet for the same area can arrive out of order or twice★ — keep only the largest `snapshot.revision` per area.
    UsageLimitsUpdated {
        snapshot: UsageLimitSnapshot,
        subscribed: Vec<AgentBackendKind>,
    },

    /// With request_id, a specific command failed.
    Error {
        request_id: Option<RequestId>,
        message: String,
    },
}

/// Subscribe result branches (design §1-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum SubscribeAction {
    /// epoch mismatch → full reset, then from the oldest.
    Reset,
    /// epoch match & after_seq<oldest → from the oldest (front part lost, clear+tail).
    TruncatedReplay,
    /// epoch match & after_seq>=oldest → lossless resume from after_seq+1.
    Resume,
}

/// Structured output event wire mirror (ADR-0045 tag1 StructuredEvent) — a **faithful mirror** of agent `OutputEvent`.
///
/// ★Why a new type (not extending OutputChunk)★: the existing wire `OutputChunk` (below) is an S14 leftover, so it has no `turn_id`/
/// `id`/`message_id` and no `MessageDone`/`Error` variant. Besides, it is tied to the `AgentEvent::Output`·
/// `export_all_to` use sites, so extending it risks breaking that contract. Satisfying ADR-0045 "self-describing +
/// swappability (preserve optional turn_id/message_id)" requires mirroring agent `OutputEvent` with zero field loss,
/// so a clean **new wire type** is introduced (OutputChunk stays as is, only for the GetSnapshot snapshot).
///
/// ★The agent↔wire conversion is the daemon adapter★ (ADR-0003 isolation): agent `OutputEvent` (domain type, no Serialize)
/// → this wire type is mapped explicitly by daemon `connection_core::output_event_to_wire`. protocol owns only the wire
/// type (no agent dependency).
///
/// ★TerminalBytes is excluded★: console raw bytes flow only in the tag0 terminal frame (payload=raw bytes) and
/// are not carried in the tag1 payload. So this mirror has no TerminalBytes variant — if agent
/// `OutputEvent::TerminalBytes` comes to this conversion, the adapter absorbs it defensively (the rationale comment is in output_event_to_wire).
///
/// ★self-describing serde★: internally-tagged (`#[serde(tag="type")]`) — the `"type"` discriminator is embedded in the payload JSON
/// so the frontend splits the variant after JSON.parse.
/// wire serialization format = JSON (serde_json) — the daemon adapter builds the tag1 payload with `serde_json::to_vec`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type")]
#[ts(export)]
pub enum StructuredEvent {
    /// Assistant text increment (streaming delta).
    TextDelta {
        text: String,
        turn_id: Option<String>,
        message_id: Option<String>,
    },
    /// Serialized arguments (the per-backend schema as is).
    ToolCall {
        name: String,
        args_json: String,
        /// Call identifier (for permission UX · result matching). claude tool_use id etc.
        id: Option<String>,
        turn_id: Option<String>,
        message_id: Option<String>,
        /// The tool's neutral category. ★If the field is absent (old daemon), the frontend consumer reads it as `Other`★ — a new daemon always carries it.
        // ADR-0239: `PROTOCOL_VERSION` is not bumped for this field — the same judgment as `TurnEnd` (one direction daemon→shell ·
        //   nothing to misread on the old side).
        // ts-rs cannot read this serde attribute, so it warns ("failed to parse") and ignores it — the TS field shape is decided by
        //   `ts(optional)` below.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        category: Option<ToolCategory>,
    },
    /// The end result of one tool call (mirror of agent `OutputEvent::ToolResult`) — points to the preceding `ToolCall` by `id`.
    /// ★Not a new row★ — if there is no call to point to, there is nowhere to attach.
    ///
    /// ★No such event ≠ success★ — absence is 「unknown」 (old daemon · pushed out of the ring · the end never came).
    /// ★It also comes **after** the turn end★ — it attaches to a row of the previous turn. Do not read it as 「new content arrived」 and release the waiting indicator.
    // ADR-0241: `PROTOCOL_VERSION` is not bumped for this variant — the same judgment as `TurnEnd` (one direction daemon→shell ·
    //   nothing to misread on an old shell · an old daemon just does not send this event).
    ToolResult { id: String, outcome: ToolOutcome },
    Usage {
        #[ts(type = "number")]
        input_tokens: u64,
        #[ts(type = "number")]
        output_tokens: u64,
        turn_id: Option<String>,
    },
    /// End signal of one message (turn response).
    MessageDone {
        turn_id: Option<String>,
        message_id: Option<String>,
    },
    /// Turn boundary + **how it ended** (mirror of agent `OutputEvent::TurnEnd`).
    ///
    /// ★Being a separate type from `MessageDone` is the contract★ — that one is "one message closed", this one is "one turn closed with this
    ///   outcome". In backends with several completed items per turn, the two are not the same event.
    /// ★`PROTOCOL_VERSION` is not bumped for this addition★ — the bump criterion [`crate::PROTOCOL_VERSION`] records
    ///   is not shape but **silent malfunction**. Measuring each of the two directions against that criterion gives this:
    ///   - **new daemon + old shell**: the event enum lets an unknown variant pass silently on the Rust side, so that combination
    ///     does not break. ★But the frontend accumulator switch has no `default:` branch, so **it is not silent on the
    ///     screen**★ (an unknown `type` adds no item and does not move the turn indicator) — adding that branch is the job of
    ///     the side that consumes this vocabulary.
    ///   - **old daemon + new shell**: ★this is the everyday combination of local development★ — the daemon outlives shell rebuilds (that
    ///     fact is recorded in that constant's v4 entry). The old daemon **never sends** this event. So the new shell's
    ///     waiting indicator never releases and no notification shows — a person **cannot tell apart** 「my fix does not work」 and
    ///     「your daemon is stale」.
    /// ★Why it is still not bumped★ — the cases that criterion lists are all cases where **the two sides interpret the same data
    ///   differently**. This change is **one direction only**: events flow only daemon→shell, and the new shell sends nothing new
    ///   to the daemon. So the old daemon has nothing to misread at all and **merely lacks the feature**. Counting 「an old build lacks a new
    ///   fix」 as a silent malfunction would force a bump **for every daemon-side fix**.
    /// ★And it is unreleased, so the price is cheap★ — there are no outside users yet, so only developers meet this combination, and
    ///   reversing this judgment later is cheap too. (user decision 2026-09-14)
    /// ★The way to recover from the symptom★: **if the waiting indicator never releases, check whether the daemon is stale before digging for a bug.**
    TurnEnd {
        turn_id: Option<String>,
        outcome: TurnOutcome,
    },
    /// An error the backend reported — ★**not a turn boundary**★.
    ///
    /// ★What each of the two words means — consumers split here★:
    ///   - `Error` = **an incident within the turn**. The same turn continues after it (a retried stream error is
    ///     of this kind). Reading it as turn end splits one turn into as many pieces as incidents, and the real boundary that follows
    ///     attaches to an already-ended turn.
    ///   - [`StructuredEvent::TurnEnd`] = **the turn ended**. A turn that ended in failure also comes in this word — the reason
    ///     is carried by its outcome field.
    /// ★So 「is it retried」 is not emitted as a separate field★ — that distinction is already carried by the **event type**,
    ///   and making one more field would have the same fact live in two places until one goes stale.
    Error { message: String },
    /// The escape hatch for per-backend events not caught by the regular variants above (forward-compat).
    /// kind=category tag, json=original serialized payload (the frontend branches · interprets by kind).
    Structured { kind: String, json: String },
    /// Queued input registry event (mirror of agent `OutputEvent::QueuedInput`). Shape = `{"type":"QueuedInput","op":{"kind":…}}`.
    ///
    /// ★The frontend accumulator reads this event sequence with **the same reduction rules** as the agent registry★ — the shared golden of the two sides is
    ///   `crates/engram-dashboard-agent/src/queued_input_golden.json`, and its event object is exactly the shape of `op`.
    /// ★`PROTOCOL_VERSION` is not bumped for this variant★ — the same judgment as [`StructuredEvent::TurnEnd`]
    ///   (one direction daemon→shell · an old daemon just does not send this event, nothing to misread).
    // ADR-0231
    QueuedInput { op: QueuedInputEvent },
}

/// Queued input registry event — mirror of agent `QueuedInputEvent`. Discriminator = `"kind"` (so it does not overlap with the outer envelope's `"type"`
/// — the same reason as [`TurnOutcome`]).
///
/// ★`Delivered` does not carry the body★ — the bubble body is the preceding `Queued`'s `text` (our local copy).
/// ★`AckUnavailable.delivered` is always filled when written to the ring (the agent core fills it)★ — one (id, body) pair for each item that event closes
///   as received. Even a ring window that lost `Queued` draws that bubble from this copy.
// ADR-0231
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum QueuedInputEvent {
    Queued {
        id: String,
        text: String,
    },
    CancelRequested {
        id: String,
    },
    /// Only `removed: true` is an outcome (cancelled) — `false` is material for telling the cause of the following vendor close.
    CancelAnswered {
        id: String,
        removed: bool,
    },
    CancelFailed {
        id: String,
    },
    /// Handover mark — `sent: true` = the link handed it to the vendor · `false` = put back to the held spot. Not an outcome (the row stays —
    ///   a handed-over row hides ✕). No-op for an id not in the list · the last one wins.
    HandedOver {
        id: String,
        sent: bool,
    },
    Delivered {
        id: String,
    },
    /// `Rejected` also erases the bubble drawn with that id.
    Dropped {
        id: String,
        cause: DropCause,
    },
    AckUnavailable {
        delivered: Vec<DeliveredCopy>,
    },
}

/// The cause of [`QueuedInputEvent::Dropped`] — one string on the wire (`"Withdrawn"` etc.).
/// ★Only `Unknown` leaves a revivable tombstone★ (a late `Delivered` wins).
// ADR-0231
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum DropCause {
    Withdrawn,
    Interrupted,
    AgentEnded,
    Rejected,
    Unknown,
}

/// The bubble copy [`QueuedInputEvent::AckUnavailable`] carries.
// ADR-0231
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct DeliveredCopy {
    pub id: String,
    pub text: String,
}

/// **How** a turn ended — the neutral vocabulary [`StructuredEvent::TurnEnd`] carries (mirror of agent `TurnOutcome`).
///
/// ★What is neutral is the **discriminator** — `detail` is not★ (ADR-0004): the counterpart protocol's state vocabulary
///   (values like `completed`·`failed`·`inProgress`) ends inside each backend decoder, and only these four
///   branches go out. So **the axis consumers branch on** has no backend name.
/// ★Instead, `detail` carries the sentence the counterpart made as is — that is this field's contract★: the failure reason is
///   the only information the counterpart gave (codex puts its own label like `serverOverloaded` and 「the sentence to put in
///   the next turn as is」 there), and translating it into neutral vocabulary would mean inventing meaning. So this field's grade is
///   **the same** as `message` of [`StructuredEvent::Error`] — a human-readable opaque string, and only length and
///   credential masking are the backend decoder's responsibility.
/// ★Consumer duty = only draw it★: parsing `detail` or branching on its content makes the frontend know
///   backend vocabulary at that moment, and the isolation really leaks. Branch only on the discriminator.
/// ★Do not fold `Unknown` into one of the three known ones★ — folding requires guessing the meaning of an unknown value, and if wrong
///   the screen draws a false outcome. The fact that it is unknown is itself carried.
/// ★self-describing serde★: internally-tagged (`#[serde(tag="kind")]`) — the outer envelope uses `"type"`, so
///   this one uses `"kind"` (so two discriminators do not overlap in the same object).
// ADR-0045
// ADR-0004
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum TurnOutcome {
    /// Normal end.
    Completed,
    /// Failure. `detail` = **the human-readable reason the counterpart made** (may be absent — when only the failure is known, without a reason).
    ///   ★Not neutral vocabulary — do not branch on it, only draw it★ (type doc).
    Failed { detail: Option<String> },
    /// The user interrupted — ★not a failure★.
    Interrupted,
    /// The outcome cannot be known (an unknown value, or none at all). The turn still counts as ended.
    Unknown,
}

/// The neutral category of a tool call — `category` of [`StructuredEvent::ToolCall`] (mirror of agent `ToolCategory`). On the wire,
/// one string (`"Read"` etc.).
///
/// ★Vendor tool names · item types do not come here★ (ADR-0004) — the decision ends inside each backend translator.
/// ★The frontend consumer reads an unknown word as `Other`★ — the screen holds up even if a newer daemon adds variants.
// ADR-0239
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum ToolCategory {
    Read,
    Search,
    List,
    Edit,
    Command,
    Web,
    Agent,
    Mcp,
    Other,
}

/// The neutral outcome of a tool call's end result — `outcome` of [`StructuredEvent::ToolResult`] (mirror of agent `ToolOutcome`). On the wire,
/// one string (`"Failed"` etc.).
///
/// - `Completed` = completed normally. ★No daemon carries it now★ — only failure · refusal come, and this word exists only as laid-down vocabulary.
/// - `Failed` = it ran and failed.
/// - `Declined` = it was not executed — a refusal not attributed to our rejection. The reason is unknown.
/// - `Refused` = it was not executed because the dashboard rejected the approval request.
///
/// ★Vendor end-state strings do not come here★ (ADR-0004). ★The frontend consumer reads an unknown word as no mark★ —
/// the screen holds up even if a newer daemon adds words.
// ADR-0241
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum ToolOutcome {
    Completed,
    Failed,
    Declined,
    Refused,
}

/// Output chunk — kind-agnostic (design §2).
/// (Structured per-turn output gets actually filled in when the TUI↔structured switching mode is designed — for now only the shape is opened.)
///
/// ※S15/ADR-0045: tag1 structured events flow not through this type but through [`StructuredEvent`] above (zero field loss
/// mirror). This `OutputChunk` remains only for the GetSnapshot snapshot (AgentEvent::Output/Snapshot) contract.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum OutputChunk {
    /// Console raw bytes (the only real use now). Not carried on the JSON path — codec binary frame only.
    TerminalBytes(
        #[serde(with = "serde_bytes")]
        #[ts(type = "number[]")]
        Vec<u8>,
    ),
    TextDelta(String),
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    ToolCall {
        name: String,
        args_json: String,
    },
    /// Arbitrary structured payload (forward-compat escape hatch).
    Structured {
        kind: String,
        json: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn structured_event_roundtrip_all_variants() {
        let cases = vec![
            StructuredEvent::TextDelta {
                text: "hello".into(),
                turn_id: Some("t1".into()),
                message_id: None,
            },
            StructuredEvent::ToolCall {
                name: "read".into(),
                args_json: r#"{"path":"/x"}"#.into(),
                id: Some("call_1".into()),
                turn_id: None,
                message_id: Some("m1".into()),
                category: Some(ToolCategory::Read),
            },
            StructuredEvent::ToolCall {
                name: "x".into(),
                args_json: "{}".into(),
                id: None,
                turn_id: None,
                message_id: None,
                category: None,
            },
            StructuredEvent::ToolResult {
                id: "call_1".into(),
                outcome: ToolOutcome::Declined,
            },
            StructuredEvent::Usage {
                input_tokens: 123,
                output_tokens: 456,
                turn_id: Some("t2".into()),
            },
            StructuredEvent::MessageDone {
                turn_id: Some("t3".into()),
                message_id: Some("m2".into()),
            },
            StructuredEvent::TurnEnd {
                turn_id: Some("t4".into()),
                outcome: TurnOutcome::Completed,
            },
            StructuredEvent::TurnEnd {
                turn_id: None,
                outcome: TurnOutcome::Failed {
                    detail: Some("model refused".into()),
                },
            },
            StructuredEvent::TurnEnd {
                turn_id: None,
                outcome: TurnOutcome::Failed { detail: None },
            },
            StructuredEvent::TurnEnd {
                turn_id: Some("t5".into()),
                outcome: TurnOutcome::Interrupted,
            },
            StructuredEvent::TurnEnd {
                turn_id: Some("t6".into()),
                outcome: TurnOutcome::Unknown,
            },
            StructuredEvent::Error {
                message: "stream error".into(),
            },
            StructuredEvent::Structured {
                kind: "custom".into(),
                json: r#"{"k":1}"#.into(),
            },
            StructuredEvent::QueuedInput {
                op: QueuedInputEvent::Dropped {
                    id: "u1".into(),
                    cause: DropCause::Unknown,
                },
            },
            StructuredEvent::QueuedInput {
                op: QueuedInputEvent::AckUnavailable {
                    delivered: vec![DeliveredCopy {
                        id: "u2".into(),
                        text: "hi".into(),
                    }],
                },
            },
        ];
        for ev in cases {
            let json = serde_json::to_string(&ev).expect("serialization succeeds");
            assert!(
                json.contains("\"type\""),
                "internally-tagged discriminator missing: {json}"
            );
            let back: StructuredEvent =
                serde_json::from_str(&json).expect("deserialization succeeds");
            assert_eq!(ev, back, "lossless round trip (zero field loss)");
        }
    }

    /// Whether optional turn_id/message_id restore exactly as None when they are None (swappability — fields codex/gemini
    /// cannot fill must not be filled with arbitrary values).
    #[test]
    fn structured_event_optional_fields_preserve_none() {
        let ev = StructuredEvent::TextDelta {
            text: "x".into(),
            turn_id: None,
            message_id: None,
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: StructuredEvent = serde_json::from_str(&json).unwrap();
        match back {
            StructuredEvent::TextDelta {
                turn_id,
                message_id,
                ..
            } => {
                assert!(turn_id.is_none() && message_id.is_none(), "None preserved");
            }
            _ => panic!("variant mismatch"),
        }
    }

    /// ★The frontend reads these characters as is — pin them with a golden★: the outer envelope discriminator is `"type"`,
    /// the outcome discriminator is `"kind"`. Giving both the same name would make them overlap in one object and the outcome would vanish.
    ///
    /// ★The neutrality this item pins is one thing, the **discriminator set**★ — those four are exactly the axis the frontend branches on,
    /// and if backend vocabulary comes in there, this turns red. ★`detail` is not that axis — that field carries the sentence the counterpart
    /// made as is★ (the last assertion below pins that fact explicitly). Do not read it as 「no backend
    /// vocabulary at all reaches the wire」 — that is false, and once this spot and the type doc were
    /// written that way.
    #[test]
    fn turn_end_wire_shape_is_pinned_and_the_outcome_discriminant_is_neutral() {
        assert_eq!(
            serde_json::to_string(&StructuredEvent::TurnEnd {
                turn_id: Some("t1".into()),
                outcome: TurnOutcome::Interrupted,
            })
            .unwrap(),
            r#"{"type":"TurnEnd","turn_id":"t1","outcome":{"kind":"Interrupted"}}"#
        );
        assert_eq!(
            serde_json::to_string(&StructuredEvent::TurnEnd {
                turn_id: None,
                outcome: TurnOutcome::Failed {
                    detail: Some("boom".into())
                },
            })
            .unwrap(),
            r#"{"type":"TurnEnd","turn_id":null,"outcome":{"kind":"Failed","detail":"boom"}}"#
        );
        assert_eq!(
            serde_json::to_string(&StructuredEvent::TurnEnd {
                turn_id: None,
                outcome: TurnOutcome::Unknown,
            })
            .unwrap(),
            r#"{"type":"TurnEnd","turn_id":null,"outcome":{"kind":"Unknown"}}"#
        );

        // ★The discriminator set is the neutrality★ — if any value other than the four goes out, backend vocabulary has been put on
        //   the frontend's branching axis. The values are read as `serde_json` emits them (names are not copied by hand).
        let discriminants: Vec<String> = [
            TurnOutcome::Completed,
            TurnOutcome::Failed { detail: None },
            TurnOutcome::Interrupted,
            TurnOutcome::Unknown,
        ]
        .iter()
        .map(|o| {
            serde_json::to_value(o).unwrap()["kind"]
                .as_str()
                .expect("kind")
                .to_string()
        })
        .collect();
        assert_eq!(
            discriminants,
            ["Completed", "Failed", "Interrupted", "Unknown"]
        );

        // ★And `detail` goes **as is** — this is the contract, not a defect★: the failure reason given by the counterpart is
        //   the only information, so translating it into neutral vocabulary would mean inventing meaning. Consumers only draw this field
        //   and do not parse · branch on it (type doc). Its grade is the same as `Error{message}`.
        let raw = "stream error (serverOverloaded) [willRetry]";
        assert_eq!(
            serde_json::to_string(&StructuredEvent::TurnEnd {
                turn_id: None,
                outcome: TurnOutcome::Failed {
                    detail: Some(raw.into())
                },
            })
            .unwrap(),
            format!(
                r#"{{"type":"TurnEnd","turn_id":null,"outcome":{{"kind":"Failed","detail":"{raw}"}}}}"#
            )
        );
    }

    // ── Tool category (ADR-0239) ──────────────────────────────────────────────────────────

    /// ★The frontend reads these characters as is — pin them with a golden★: the category is one string, a line from an old daemon without the field
    /// is still read (→ `None`), and `None` does not carry the field at all.
    // ADR-0239
    #[test]
    fn tool_call_category_wire_shape_is_pinned_and_absent_means_none() {
        let with = StructuredEvent::ToolCall {
            name: "Read".into(),
            args_json: "{}".into(),
            id: Some("c1".into()),
            turn_id: None,
            message_id: None,
            category: Some(ToolCategory::Search),
        };
        let json = serde_json::to_string(&with).unwrap();
        assert_eq!(
            json,
            r#"{"type":"ToolCall","name":"Read","args_json":"{}","id":"c1","turn_id":null,"message_id":null,"category":"Search"}"#
        );
        assert_eq!(
            serde_json::from_str::<StructuredEvent>(&json).unwrap(),
            with
        );

        let old = r#"{"type":"ToolCall","name":"Read","args_json":"{}","id":"c1","turn_id":null,"message_id":null}"#;
        let back: StructuredEvent =
            serde_json::from_str(old).expect("an old line without the field does not read");
        assert_eq!(
            back,
            StructuredEvent::ToolCall {
                name: "Read".into(),
                args_json: "{}".into(),
                id: Some("c1".into()),
                turn_id: None,
                message_id: None,
                category: None,
            }
        );
        assert_eq!(
            serde_json::to_string(&back).unwrap(),
            old,
            "None carried the field"
        );
    }

    /// ★The nine words travel as strings as is★ — the frontend reads these words through the generated `ToolCategory.ts`
    /// (`TOOL_CATEGORIES` in `structuredAccumulator.ts`).
    // ADR-0239
    #[test]
    fn every_tool_category_round_trips_as_its_own_name() {
        let all = [
            (ToolCategory::Read, "Read"),
            (ToolCategory::Search, "Search"),
            (ToolCategory::List, "List"),
            (ToolCategory::Edit, "Edit"),
            (ToolCategory::Command, "Command"),
            (ToolCategory::Web, "Web"),
            (ToolCategory::Agent, "Agent"),
            (ToolCategory::Mcp, "Mcp"),
            (ToolCategory::Other, "Other"),
        ];
        for (category, name) in all {
            let json = serde_json::to_string(&category).unwrap();
            assert_eq!(json, format!("\"{name}\""));
            assert_eq!(
                serde_json::from_str::<ToolCategory>(&json).unwrap(),
                category
            );
        }
    }

    // ── Tool end result (ADR-0241) ──────────────────────────────────────────────────────────

    /// ★The frontend accumulator reads these characters as is — pin them with a golden★: discriminator `"type"`, the outcome is one string.
    // ADR-0241
    #[test]
    fn tool_result_wire_shape_is_pinned() {
        for (outcome, want) in [
            (
                ToolOutcome::Failed,
                r#"{"type":"ToolResult","id":"i-1","outcome":"Failed"}"#,
            ),
            (
                ToolOutcome::Refused,
                r#"{"type":"ToolResult","id":"i-1","outcome":"Refused"}"#,
            ),
        ] {
            let ev = StructuredEvent::ToolResult {
                id: "i-1".into(),
                outcome,
            };
            let json = serde_json::to_string(&ev).unwrap();
            assert_eq!(json, want);
            assert_eq!(serde_json::from_str::<StructuredEvent>(&json).unwrap(), ev);
        }
    }

    /// ★The four words travel as strings as is★ — the frontend reads these words through the generated `ToolOutcome.ts`
    /// (`toolResultMark` in `structuredAccumulator.ts`).
    // ADR-0241
    #[test]
    fn every_tool_outcome_round_trips_as_its_own_name() {
        let all = [
            (ToolOutcome::Completed, "Completed"),
            (ToolOutcome::Failed, "Failed"),
            (ToolOutcome::Declined, "Declined"),
            (ToolOutcome::Refused, "Refused"),
        ];
        for (outcome, name) in all {
            let json = serde_json::to_string(&outcome).unwrap();
            assert_eq!(json, format!("\"{name}\""));
            assert_eq!(serde_json::from_str::<ToolOutcome>(&json).unwrap(), outcome);
        }
    }

    // ── Queued input registry events (ADR-0231) ────────────────────────────────────────────────

    /// ★The frontend accumulator reads these characters as is — pin them with a golden★: outer discriminator `"type"`, event discriminator
    /// `"kind"`, the cause is one string, the copy is an `{id,text}` array.
    #[test]
    fn queued_input_wire_shape_is_pinned() {
        let json = |op| serde_json::to_string(&StructuredEvent::QueuedInput { op }).unwrap();
        assert_eq!(
            json(QueuedInputEvent::Queued {
                id: "u1".into(),
                text: "hi".into()
            }),
            r#"{"type":"QueuedInput","op":{"kind":"Queued","id":"u1","text":"hi"}}"#
        );
        assert_eq!(
            json(QueuedInputEvent::CancelAnswered {
                id: "u1".into(),
                removed: false
            }),
            r#"{"type":"QueuedInput","op":{"kind":"CancelAnswered","id":"u1","removed":false}}"#
        );
        assert_eq!(
            json(QueuedInputEvent::HandedOver {
                id: "u1".into(),
                sent: true
            }),
            r#"{"type":"QueuedInput","op":{"kind":"HandedOver","id":"u1","sent":true}}"#
        );
        assert_eq!(
            json(QueuedInputEvent::Dropped {
                id: "u1".into(),
                cause: DropCause::AgentEnded
            }),
            r#"{"type":"QueuedInput","op":{"kind":"Dropped","id":"u1","cause":"AgentEnded"}}"#
        );
        assert_eq!(
            json(QueuedInputEvent::AckUnavailable {
                delivered: vec![DeliveredCopy {
                    id: "u2".into(),
                    text: "yo".into()
                }]
            }),
            r#"{"type":"QueuedInput","op":{"kind":"AckUnavailable","delivered":[{"id":"u2","text":"yo"}]}}"#
        );
        let causes: Vec<String> = [
            DropCause::Withdrawn,
            DropCause::Interrupted,
            DropCause::AgentEnded,
            DropCause::Rejected,
            DropCause::Unknown,
        ]
        .iter()
        .map(|c| {
            serde_json::to_value(c)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
        assert_eq!(
            causes,
            [
                "Withdrawn",
                "Interrupted",
                "AgentEnded",
                "Rejected",
                "Unknown"
            ]
        );
    }

    /// The event objects of the shared golden are exactly the wire `op` — the accumulator eats that object as `{type:"QueuedInput", op}`. If the golden
    /// is changed and a shape the wire cannot read comes in, this turns red (the file is in the agent crate — its home is next to the registry).
    #[test]
    fn the_shared_queued_input_golden_speaks_the_wire_shape() {
        let golden: serde_json::Value = serde_json::from_str(include_str!(
            "../../engram-dashboard-agent/src/queued_input_golden.json"
        ))
        .expect("golden JSON");
        let mut fed = 0;
        for case in golden["cases"].as_array().expect("cases") {
            for ev in case["events"].as_array().expect("events") {
                let op: QueuedInputEvent = serde_json::from_value(ev.clone())
                    .unwrap_or_else(|e| panic!("golden event the wire cannot read {ev}: {e}"));
                assert_eq!(
                    &serde_json::to_value(&op).unwrap(),
                    ev,
                    "lossless round trip"
                );
                fed += 1;
            }
        }
        assert!(fed > 0);
    }

    // ── Resume marker of the subscribe response (ADR-0226) ──────────────────────────────────────────
    #[test]
    fn subscribe_ack_json_golden_carries_continues_conversation() {
        let agent_id = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let json = serde_json::to_string(&AgentEvent::SubscribeAck {
            agent_id,
            action: SubscribeAction::Reset,
            current_epoch: 7,
            oldest_seq: 1,
            latest_seq: 2,
            replay_from: 0,
            truncated: false,
            continues_conversation: true,
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"SubscribeAck":{"agent_id":"11111111-1111-1111-1111-111111111111","action":"Reset","current_epoch":7,"oldest_seq":1,"latest_seq":2,"replay_from":0,"truncated":false,"continues_conversation":true}}"#
        );
    }

    /// An old daemon does not know this field — an ack missing the field must not be refused but read as `false` for a new shell to attach to an old daemon.
    #[test]
    fn a_subscribe_ack_from_an_older_daemon_reads_as_not_continuing() {
        let old = r#"{"SubscribeAck":{"agent_id":"11111111-1111-1111-1111-111111111111","action":"Resume","current_epoch":7,"oldest_seq":1,"latest_seq":2,"replay_from":1,"truncated":true}}"#;
        match serde_json::from_str::<AgentEvent>(old)
            .expect("an old ack without the field reads too")
        {
            AgentEvent::SubscribeAck {
                continues_conversation,
                truncated,
                current_epoch,
                ..
            } => {
                assert!(!continues_conversation, "missing field = not resuming");
                assert!(truncated, "the remaining fields read as is");
                assert_eq!(current_epoch, 7);
            }
            other => panic!("must be SubscribeAck: {other:?}"),
        }
    }

    // ── Preset wire contract (ADR-0061) — JSON envelope golden + round-trip ─────────────
    //
    // Pins that preset CRUD serializes in the same shape as profiles (variant name tag + fields) — if the wire
    // format changes silently (field rename/omission), the frontend mirror breaks, so a golden string guards against regression.

    #[test]
    fn create_preset_command_json_golden() {
        let request_id = RequestId(Uuid::nil());
        let cmd = AgentCommand::CreatePreset {
            cwd: "C:/proj".into(),
            request_id,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"CreatePreset":{"cwd":"C:/proj","request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "CreatePreset wire shape does not match the golden"
        );
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            AgentCommand::CreatePreset { cwd, .. } if cwd == "C:/proj"
        ));
    }

    #[test]
    fn list_delete_preset_commands_roundtrip() {
        let cases = vec![
            AgentCommand::ListPresets {
                request_id: RequestId(Uuid::nil()),
            },
            AgentCommand::DeletePreset {
                preset_id: Uuid::nil(),
                request_id: RequestId(Uuid::nil()),
            },
        ];
        for cmd in cases {
            let json = serde_json::to_string(&cmd).unwrap();
            let back: AgentCommand = serde_json::from_str(&json).unwrap();
            assert_eq!(json, serde_json::to_string(&back).unwrap());
        }
    }

    #[test]
    fn preset_list_events_json_golden_and_roundtrip() {
        let preset = Preset {
            id: Uuid::nil(),
            cwd: "C:/proj".into(),
            name: None,
        };
        let list = AgentEvent::PresetList {
            request_id: RequestId(Uuid::nil()),
            presets: vec![preset.clone()],
        };
        let list_json = serde_json::to_string(&list).unwrap();
        assert_eq!(
            list_json,
            r#"{"PresetList":{"request_id":"00000000-0000-0000-0000-000000000000","presets":[{"id":"00000000-0000-0000-0000-000000000000","cwd":"C:/proj","name":null}]}}"#,
            "PresetList wire shape does not match the golden"
        );

        let updated = AgentEvent::PresetListUpdated {
            presets: vec![preset],
        };
        let updated_json = serde_json::to_string(&updated).unwrap();
        assert_eq!(
            updated_json,
            r#"{"PresetListUpdated":{"presets":[{"id":"00000000-0000-0000-0000-000000000000","cwd":"C:/proj","name":null}]}}"#,
            "PresetListUpdated wire shape does not match the golden"
        );

        for json in [list_json, updated_json] {
            let back: AgentEvent = serde_json::from_str(&json).unwrap();
            assert_eq!(json, serde_json::to_string(&back).unwrap());
        }
    }

    #[test]
    fn rename_preset_command_json_golden_and_roundtrip() {
        let cmd = AgentCommand::RenamePreset {
            preset_id: Uuid::nil(),
            name: Some("내 프리셋".into()), // hangul-ok
            request_id: RequestId(Uuid::nil()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"RenamePreset":{"preset_id":"00000000-0000-0000-0000-000000000000","name":"내 프리셋","request_id":"00000000-0000-0000-0000-000000000000"}}"#, // hangul-ok
            "RenamePreset wire shape does not match the golden"
        );
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            AgentCommand::RenamePreset { name: Some(ref n), .. } if n == "내 프리셋" // hangul-ok
        ));

        let clear = AgentCommand::RenamePreset {
            preset_id: Uuid::nil(),
            name: None,
            request_id: RequestId(Uuid::nil()),
        };
        let clear_json = serde_json::to_string(&clear).unwrap();
        let clear_back: AgentCommand = serde_json::from_str(&clear_json).unwrap();
        assert_eq!(clear_json, serde_json::to_string(&clear_back).unwrap());
    }

    #[test]
    fn rename_profile_command_json_golden_and_roundtrip() {
        let cmd = AgentCommand::RenameProfile {
            profile_id: Uuid::nil(),
            name: Some("내 에이전트".into()), // hangul-ok
            request_id: RequestId(Uuid::nil()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"RenameProfile":{"profile_id":"00000000-0000-0000-0000-000000000000","name":"내 에이전트","request_id":"00000000-0000-0000-0000-000000000000"}}"#, // hangul-ok
            "RenameProfile wire shape does not match the golden"
        );
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            AgentCommand::RenameProfile { name: Some(ref n), .. } if n == "내 에이전트" // hangul-ok
        ));
    }

    #[test]
    fn set_envelope_format_command_json_golden_and_roundtrip() {
        let cmd = AgentCommand::SetEnvelopeFormat {
            format: EnvelopeFormat::Xml,
            request_id: RequestId(Uuid::nil()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"SetEnvelopeFormat":{"format":"xml","request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "SetEnvelopeFormat wire shape does not match the golden (format must be lowercase xml — invoke JSON contract)"
        );
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            AgentCommand::SetEnvelopeFormat {
                format: EnvelopeFormat::Xml,
                ..
            }
        ));

        let colon = AgentCommand::SetEnvelopeFormat {
            format: EnvelopeFormat::Colon,
            request_id: RequestId(Uuid::nil()),
        };
        let colon_json = serde_json::to_string(&colon).unwrap();
        assert_eq!(
            colon_json,
            r#"{"SetEnvelopeFormat":{"format":"colon","request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "the colon variant must be lowercase too"
        );
        let colon_back: AgentCommand = serde_json::from_str(&colon_json).unwrap();
        assert_eq!(colon_json, serde_json::to_string(&colon_back).unwrap());
    }

    #[test]
    fn envelope_format_default_is_xml_and_lowercase_deserializes() {
        assert_eq!(
            EnvelopeFormat::default(),
            EnvelopeFormat::Xml,
            "default envelope format = xml (ADR-0103 default flip — consistent with the daemon's operating default)"
        );
        let xml: EnvelopeFormat = serde_json::from_str(r#""xml""#).unwrap();
        assert_eq!(xml, EnvelopeFormat::Xml);
        let colon: EnvelopeFormat = serde_json::from_str(r#""colon""#).unwrap();
        assert_eq!(colon, EnvelopeFormat::Colon);
    }

    #[test]
    fn reparent_profile_command_json_golden_and_roundtrip() {
        let child = Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap();
        let parent = Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap();

        let cmd = AgentCommand::ReparentProfile {
            child_id: child,
            parent_id: Some(parent),
            request_id: RequestId(Uuid::nil()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"ReparentProfile":{"child_id":"11111111-1111-1111-1111-111111111111","parent_id":"22222222-2222-2222-2222-222222222222","request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "ReparentProfile(Some) wire shape does not match the golden"
        );
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            AgentCommand::ReparentProfile { parent_id: Some(p), .. } if p == parent
        ));

        let clear = AgentCommand::ReparentProfile {
            child_id: child,
            parent_id: None,
            request_id: RequestId(Uuid::nil()),
        };
        let clear_json = serde_json::to_string(&clear).unwrap();
        assert_eq!(
            clear_json,
            r#"{"ReparentProfile":{"child_id":"11111111-1111-1111-1111-111111111111","parent_id":null,"request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "ReparentProfile(None) wire shape does not match the golden"
        );
        let clear_back: AgentCommand = serde_json::from_str(&clear_json).unwrap();
        assert!(matches!(
            clear_back,
            AgentCommand::ReparentProfile {
                parent_id: None,
                ..
            }
        ));
    }

    // ── Command bus registration wire (ADR-0155/0156) ─────────────────────────────────────────
    //
    // ★The registration item list field is raw JSON (ADR-0272 decision 3)★ — this measures only the packet's outer shape and 「the field bytes cross
    //   as is」. The item → bytes golden (with · without `agent_args`) and the guard of the hand-written TS field are owned by the shell edge that wraps
    //   the field (`daemon_client::protocol_state` tests), and unwrapping an old item missing the field into empty `agent_args` is owned by the daemon edge
    //   that unwraps the field (`connection_core` tests), with the same literals.

    const SPAWN_DECLS: &str = r#"[{"name":"agent.spawn","help":"{\"name\":\"agent.spawn\",\"effect\":\"Write\",\"since\":1}"}]"#;
    const MIXED_DECLS: &str = r#"[{"name":"slot.focus","help":"{}","agent_args":["agent","after_tab"]},{"name":"tab.focus","help":"{}"}]"#;

    /// A registration packet round-trips carrying the item list field — `owner` is a bare string, the field crosses **byte for byte** (the item's
    /// `help` does not change by a single character — the actual form of the contract that relays do not open it).
    #[test]
    fn register_commands_round_trips_declarations() {
        let cmd = AgentCommand::RegisterCommands {
            owner: OwnerToken::new("shell"),
            decls: raw_slot(SPAWN_DECLS),
            catalog_version: 1,
            request_id: RequestId(Uuid::nil()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"RegisterCommands":{"owner":"shell","decls":[{"name":"agent.spawn","help":"{\"name\":\"agent.spawn\",\"effect\":\"Write\",\"since\":1}"}],"catalog_version":1,"request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "RegisterCommands wire shape does not match the golden (owner is a bare string, help is a whole string)"
        );

        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        match back {
            AgentCommand::RegisterCommands { owner, decls, .. } => {
                assert_eq!(owner.as_str(), "shell");
                assert_eq!(
                    decls.get(),
                    SPAWN_DECLS,
                    "the raw field crosses byte for byte"
                );
            }
            other => panic!("variant mismatch: {other:?}"),
        }
    }

    /// The diff splits into `added` (with shape) + `removed` (names only) — merging the two into one shape would require inventing
    /// a nonexistent `help` when removing.
    #[test]
    fn update_commands_carries_added_shapes_and_removed_names() {
        let cmd = AgentCommand::UpdateCommands {
            owner: OwnerToken::new("shell"),
            added: raw_slot(r#"[{"name":"chat.get","help":"{}"}]"#),
            removed: vec!["chat.reset".into()],
            request_id: RequestId(Uuid::nil()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"UpdateCommands":{"owner":"shell","added":[{"name":"chat.get","help":"{}"}],"removed":["chat.reset"],"request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "UpdateCommands wire shape does not match the golden"
        );
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(json, serde_json::to_string(&back).unwrap());
    }

    /// ★Old items without the agent-reference field (ADR-0308) and new items with it cross through the same raw field★ — when the field is empty it is
    /// left out on the wire, so the two goldens above (registration without marks) stand as they are. Reading the missing field as an empty list is the daemon edge's
    /// job (`connection_core`'s `an_old_register_slot_unpacks_to_no_agent_args`).
    #[test]
    fn decl_agent_args_are_optional_on_the_wire() {
        let old_decls = r#"[{"name":"tab.focus","help":"{}"}]"#;
        let old = format!(
            r#"{{"RegisterCommands":{{"owner":"shell","decls":{old_decls},"catalog_version":1,"request_id":"00000000-0000-0000-0000-000000000000"}}}}"#
        );
        match serde_json::from_str::<AgentCommand>(&old).expect("old packet") {
            AgentCommand::RegisterCommands { decls, .. } => assert_eq!(decls.get(), old_decls),
            other => panic!("variant mismatch: {other:?}"),
        }

        let cmd = AgentCommand::UpdateCommands {
            owner: OwnerToken::new("shell"),
            added: raw_slot(
                r#"[{"name":"slot.focus","help":"{}","agent_args":["agent","after_tab"]}]"#,
            ),
            removed: vec![],
            request_id: RequestId(Uuid::nil()),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"UpdateCommands":{"owner":"shell","added":[{"name":"slot.focus","help":"{}","agent_args":["agent","after_tab"]}],"removed":[],"request_id":"00000000-0000-0000-0000-000000000000"}}"#,
            "with marks, they are carried in declaration order"
        );
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(json, serde_json::to_string(&back).unwrap());
    }

    #[test]
    fn register_commands_json_golden_carries_agent_args() {
        let cmd = AgentCommand::RegisterCommands {
            owner: OwnerToken::new("shell"),
            decls: raw_slot(MIXED_DECLS),
            catalog_version: 1,
            request_id: RequestId(Uuid::nil()),
        };
        let golden = r#"{"RegisterCommands":{"owner":"shell","decls":[{"name":"slot.focus","help":"{}","agent_args":["agent","after_tab"]},{"name":"tab.focus","help":"{}"}],"catalog_version":1,"request_id":"00000000-0000-0000-0000-000000000000"}}"#;
        assert_eq!(
            serde_json::to_string(&cmd).unwrap(),
            golden,
            "RegisterCommands packet outer shape does not match the golden — the item shape is guarded by the shell's \
             `command_decl_json_golden_with_and_without_agent_args`"
        );
        match serde_json::from_str::<AgentCommand>(golden).unwrap() {
            AgentCommand::RegisterCommands {
                owner,
                decls,
                catalog_version,
                request_id,
            } => {
                assert_eq!(owner.as_str(), "shell");
                assert_eq!(
                    decls.get(),
                    MIXED_DECLS,
                    "the raw field crosses byte for byte"
                );
                assert_eq!(catalog_version, 1);
                assert_eq!(request_id, RequestId(Uuid::nil()));
            }
            other => panic!("variant mismatch: {other:?}"),
        }
    }

    /// ★The item list field also checks only 「is it valid JSON」★ (ADR-0272 decisions 3 · 6) — a list with the wrong shape must still decode as a packet
    /// for the edge to handle that branch (daemon = refuse with that packet's `request_id` · webview path = invoke failure).
    #[test]
    fn the_decl_slots_take_any_json_but_reject_non_json() {
        let nil = "00000000-0000-0000-0000-000000000000";
        for decls in [r#"[{"name":1}]"#, r#"{"name":"tab.focus"}"#, "null"] {
            let registered: AgentCommand = serde_json::from_str(&format!(
                r#"{{"RegisterCommands":{{"owner":"shell","decls":{decls},"catalog_version":1,"request_id":"{nil}"}}}}"#
            ))
            .expect("a list with the wrong shape must still decode as a RegisterCommands packet");
            let AgentCommand::RegisterCommands { decls: carried, .. } = registered else {
                panic!("must be RegisterCommands");
            };
            assert_eq!(carried.get(), decls);

            let updated: AgentCommand = serde_json::from_str(&format!(
                r#"{{"UpdateCommands":{{"owner":"shell","added":{decls},"removed":[],"request_id":"{nil}"}}}}"#
            ))
            .expect("a list with the wrong shape must still decode as an UpdateCommands packet");
            let AgentCommand::UpdateCommands { added: carried, .. } = updated else {
                panic!("must be UpdateCommands");
            };
            assert_eq!(carried.get(), decls);
        }
        assert!(serde_json::from_str::<AgentCommand>(&format!(
            r#"{{"RegisterCommands":{{"owner":"shell","decls":,"catalog_version":1,"request_id":"{nil}"}}}}"#
        ))
        .is_err());
        assert!(
            serde_json::from_str::<AgentCommand>(&format!(
                r#"{{"UpdateCommands":{{"owner":"shell","removed":[],"request_id":"{nil}"}}}}"#
            ))
            .is_err(),
            "a packet without the list field is refused"
        );
    }

    /// Query round trip. Baking `available=false` in too is intended for now — the daemon does not emit that value
    /// (ADR-0150 decision 3), but the contract carries both values, and this golden holds that field from silently disappearing.
    #[test]
    fn list_commands_and_command_list_round_trip() {
        let req = AgentCommand::ListCommands {
            request_id: RequestId(Uuid::nil()),
        };
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            r#"{"ListCommands":{"request_id":"00000000-0000-0000-0000-000000000000"}}"#
        );

        let reply = AgentEvent::CommandList {
            request_id: RequestId(Uuid::nil()),
            entries: vec![
                CommandListEntry {
                    name: "agent.spawn".into(),
                    help: "{}".into(),
                    available: true,
                },
                CommandListEntry {
                    name: "tab.create".into(),
                    help: "{}".into(),
                    available: false,
                },
            ],
        };
        let json = serde_json::to_string(&reply).unwrap();
        assert_eq!(
            json,
            r#"{"CommandList":{"request_id":"00000000-0000-0000-0000-000000000000","entries":[{"name":"agent.spawn","help":"{}","available":true},{"name":"tab.create","help":"{}","available":false}]}}"#,
            "CommandList wire shape does not match the golden"
        );
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(json, serde_json::to_string(&back).unwrap());
    }

    // ── Queued input list wire (ADR-0231) — the frontend · shell read these characters as is ─────────────
    const NIL: &str = "00000000-0000-0000-0000-000000000000";

    #[test]
    fn queued_input_commands_json_golden_and_roundtrip() {
        let list = AgentCommand::ListQueuedInputs {
            agent_id: Uuid::nil(),
            request_id: RequestId(Uuid::nil()),
        };
        let cancel = AgentCommand::CancelQueuedInput {
            agent_id: Uuid::nil(),
            input_id: "q1".into(),
            request_id: RequestId(Uuid::nil()),
        };
        for (cmd, golden) in [
            (
                list,
                format!(r#"{{"ListQueuedInputs":{{"agent_id":"{NIL}","request_id":"{NIL}"}}}}"#),
            ),
            (
                cancel,
                format!(
                    r#"{{"CancelQueuedInput":{{"agent_id":"{NIL}","input_id":"q1","request_id":"{NIL}"}}}}"#
                ),
            ),
        ] {
            let json = serde_json::to_string(&cmd).unwrap();
            assert_eq!(
                json, golden,
                "queued input command wire shape does not match the golden"
            );
            let back: AgentCommand = serde_json::from_str(&json).unwrap();
            assert_eq!(json, serde_json::to_string(&back).unwrap());
        }
    }

    /// The row shape · words are the same as the bus `agent.listQueuedInputs` row — only cancel-pending rows carry the two fields (`answer`·`vendor_closed`),
    /// the rest `cancel: null`. `as_of_seq` is a JSON number (the frontend binding is `number | null` too).
    #[test]
    fn queued_inputs_reply_json_golden_and_roundtrip() {
        let reply = AgentEvent::QueuedInputs {
            request_id: RequestId(Uuid::nil()),
            agent_id: Uuid::nil(),
            inputs: vec![
                QueuedInputRow {
                    id: "q1".into(),
                    text: "first".into(),
                    state: "queued".into(),
                    cancel: None,
                },
                QueuedInputRow {
                    id: "q2".into(),
                    text: "second".into(),
                    state: "cancelling".into(),
                    cancel: Some(QueuedInputCancel {
                        answer: "not_removed".into(),
                        vendor_closed: true,
                    }),
                },
            ],
            as_of_seq: Some(7),
            epoch: 3,
            stopped_after_error: true,
        };
        let json = serde_json::to_string(&reply).unwrap();
        assert_eq!(
            json,
            format!(
                r#"{{"QueuedInputs":{{"request_id":"{NIL}","agent_id":"{NIL}","inputs":[{{"id":"q1","text":"first","state":"queued","cancel":null}},{{"id":"q2","text":"second","state":"cancelling","cancel":{{"answer":"not_removed","vendor_closed":true}}}}],"as_of_seq":7,"epoch":3,"stopped_after_error":true}}}}"#
            ),
            "QueuedInputs wire shape does not match the golden"
        );
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(json, serde_json::to_string(&back).unwrap());

        let empty = AgentEvent::QueuedInputs {
            request_id: RequestId(Uuid::nil()),
            agent_id: Uuid::nil(),
            inputs: vec![],
            as_of_seq: None,
            epoch: 0,
            stopped_after_error: false,
        };
        assert_eq!(
            serde_json::to_string(&empty).unwrap(),
            format!(
                r#"{{"QueuedInputs":{{"request_id":"{NIL}","agent_id":"{NIL}","inputs":[],"as_of_seq":null,"epoch":0,"stopped_after_error":false}}}}"#
            ),
            "an incarnation with no list event yet = as_of_seq null"
        );
    }

    #[test]
    fn queued_input_cancel_reply_json_golden_and_roundtrip() {
        let reply = AgentEvent::QueuedInputCancelReply {
            request_id: RequestId(Uuid::nil()),
            agent_id: Uuid::nil(),
            input_id: "q1".into(),
            outcome: "requested".into(),
        };
        let json = serde_json::to_string(&reply).unwrap();
        assert_eq!(
            json,
            format!(
                r#"{{"QueuedInputCancelReply":{{"request_id":"{NIL}","agent_id":"{NIL}","input_id":"q1","outcome":"requested"}}}}"#
            ),
            "QueuedInputCancelReply wire shape does not match the golden"
        );
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(json, serde_json::to_string(&back).unwrap());
    }

    /// `input_id` has no default — a missing cancel is not absorbed into something like 「the most recent」 but bounced.
    #[test]
    fn queued_input_commands_reject_missing_fields() {
        for json in [
            format!(r#"{{"CancelQueuedInput":{{"agent_id":"{NIL}","request_id":"{NIL}"}}}}"#),
            format!(r#"{{"CancelQueuedInput":{{"agent_id":"{NIL}","input_id":"q1"}}}}"#),
            format!(r#"{{"ListQueuedInputs":{{"agent_id":"{NIL}"}}}}"#),
        ] {
            assert!(
                serde_json::from_str::<AgentCommand>(&json).is_err(),
                "a missing field must not be absorbed into a default: {json}"
            );
        }
    }

    /// A packet missing `request_id` **must be refused** — this crate's `RequestId::default()` mints a new v4,
    /// so with `#[serde(default)]` attached, an unpaired correlation key would silently appear (the reply would never attach).
    /// If that attribute creeps in by mistake, this assertion breaks first.
    #[test]
    fn registration_variants_reject_missing_request_id() {
        for json in [
            r#"{"RegisterCommands":{"owner":"shell","decls":[],"catalog_version":1}}"#,
            r#"{"UpdateCommands":{"owner":"shell","added":[],"removed":[]}}"#,
            r#"{"ListCommands":{}}"#,
        ] {
            assert!(
                serde_json::from_str::<AgentCommand>(json).is_err(),
                "a missing request_id must not be absorbed into a default: {json}"
            );
        }
    }

    #[test]
    fn agent_profile_wire_roundtrip_includes_parent_id() {
        use crate::domain::{AgentProfile as WireProfile, AgentSpawnCommand, RestartPolicy};

        let parent = Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap();
        let mk = |parent_id: Option<Uuid>| WireProfile {
            id: Uuid::nil(),
            name: "p".into(),
            display_name: None,
            parent_id,
            command: AgentSpawnCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
            cwd: "C:/proj".into(),
            env: vec![],
            backend_session_id: None,
            old_session_ids: vec![],
            epoch: 0,
            auto_restore: true,
            restart_policy: RestartPolicy::Always,
            restart_count: 0,
            failed_reason: None,
            last_failure: None,
            created_at: 1,
            last_active: 1,
            last_start_at: None,
        };

        for parent_id in [Some(parent), None] {
            let p = mk(parent_id);
            let json = serde_json::to_string(&p).unwrap();
            let back: WireProfile = serde_json::from_str(&json).unwrap();
            assert_eq!(
                back.parent_id, parent_id,
                "parent_id preserved in round trip"
            );
            assert_eq!(json, serde_json::to_string(&back).unwrap());
        }

        let legacy = r#"{
            "id": "00000000-0000-0000-0000-000000000000",
            "name": "legacy",
            "command": { "kind": "Shell", "program": "cmd.exe", "args": [] },
            "cwd": "C:/proj",
            "env": [],
            "backend_session_id": null,
            "old_session_ids": [],
            "epoch": 0,
            "auto_restore": true,
            "restart_policy": "Always",
            "restart_count": 0,
            "failed_reason": null,
            "created_at": 1,
            "last_active": 1,
            "last_start_at": null
        }"#;
        let p: WireProfile =
            serde_json::from_str(legacy).expect("deserialize wire without parent_id");
        assert_eq!(p.parent_id, None, "parent_id absent → None (root)");
        assert_eq!(
            p.last_failure, None,
            "last_failure absent → None (ADR-0172 additive — old wire must pass as is so there is no version bump)"
        );
    }

    /// ★The half that protects old peers when the vocabulary grows★: if one unknown kind string breaks decoding of the **whole message**
    /// of `AgentProfile`, that profile disappears from the screen entirely (not just the field going empty). It pairs with the frontend table's
    /// `table[kind] ?? Other`.
    // ADR-0172
    #[test]
    fn an_unknown_failure_kind_is_absorbed_instead_of_failing_the_whole_profile() {
        use crate::domain::{AgentFailureKind, AgentProfile as WireProfile};

        let with_future_kind = r#"{
            "id": "00000000-0000-0000-0000-000000000000",
            "name": "future",
            "command": { "kind": "Shell", "program": "cmd.exe", "args": [] },
            "cwd": "C:/proj",
            "env": [],
            "backend_session_id": null,
            "old_session_ids": [],
            "epoch": 0,
            "auto_restore": true,
            "restart_policy": "Always",
            "restart_count": 0,
            "failed_reason": null,
            "last_failure": "SomeKindFromANewerDaemon",
            "created_at": 1,
            "last_active": 1,
            "last_start_at": null
        }"#;
        let p: WireProfile = serde_json::from_str(with_future_kind)
            .expect("an unknown kind must not break the message");
        assert_eq!(
            p.name, "future",
            "the rest of the profile's fields remain intact"
        );
        assert_eq!(
            p.last_failure,
            Some(AgentFailureKind::Other),
            "an unknown kind is absorbed into `Other` (retriable = fail-open)"
        );

        // Known vocabulary passes as is (the absorption does not squash everything).
        let known = with_future_kind.replace("SomeKindFromANewerDaemon", "NoConversationToResume");
        let p: WireProfile = serde_json::from_str(&known).expect("known kind");
        assert_eq!(
            p.last_failure,
            Some(AgentFailureKind::NoConversationToResume)
        );

        // ★Absorption is open only to **unknown strings**★: if it swallowed format violations too, a broken value would turn into `Other` and
        //   a failure message would show on an item with no failure (fabrication, not absorption).
        for bad in ["42", "[]", "{}", "true"] {
            let broken = with_future_kind.replace("\"SomeKindFromANewerDaemon\"", bad);
            assert!(
                serde_json::from_str::<WireProfile>(&broken).is_err(),
                "non-string {bad} is not subject to absorption"
            );
        }
    }

    /// ★The wire vocabulary and the **generated TS union** must be the same★: the type the frontend receives is made by ts-rs, and the actual
    /// JSON is made by `as_wire_str`. If the two diverge, the frontend table absorbs every value into `Other` and the messages
    /// silently squash into one.
    ///
    /// ★Do not write the vocabulary again inside the test (then it becomes a test that measures nothing)★: the old form compared hand-written
    ///   strings in a tuple, and when the variant was changed `Other`→`Unknown` while the arm kept emitting `"Other"`,
    ///   it still **passed** — exactly the accident this doc says it prevents. Now one side comes from the generated file,
    ///   the other from the list the macro builds with `stringify!`.
    // ADR-0172
    #[test]
    fn failure_kind_vocabulary_matches_the_generated_ts_union() {
        use crate::domain::AgentFailureKind;

        // Extract the union members from the generated file (the side ts-rs made from variant names).
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/bindings/AgentFailureKind.ts");
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("must read the generated binding ({path}): {e}"));
        let decl = src
            .lines()
            .find(|l| l.starts_with("export type AgentFailureKind"))
            .expect("union declaration line");
        // `export type X = "A" | "B";` → the odd pieces are the members.
        let ts_members: Vec<&str> = decl.split('"').skip(1).step_by(2).collect();

        let wire_names: Vec<&str> = AgentFailureKind::wire_names().collect();
        assert_eq!(
            ts_members, wire_names,
            "the TS union and the wire vocabulary diverged — values the frontend receives are not in the table, so all fold into `Other`"
        );

        // And that vocabulary actually round-trips (serialized string = that name, deserialized = that variant).
        for (kind, name) in AgentFailureKind::all() {
            assert_eq!(serde_json::to_string(kind).unwrap(), format!("\"{name}\""));
            assert_eq!(
                &serde_json::from_str::<AgentFailureKind>(&format!("\"{name}\"")).unwrap(),
                kind,
                "the round trip must hold"
            );
        }
    }

    /// ★Values containing escapes must decode too★: back when it was received as a borrowed `&str`, a single `"Other"`
    /// broke the **whole message** of `AgentProfile`, and the src-tauri receiver silently discarded that failure, so the only symptom
    /// was "the tree never changes".
    // ADR-0172
    #[test]
    fn an_escaped_failure_kind_string_still_decodes() {
        use crate::domain::AgentFailureKind;

        // `O` = 'O' — same value, only the encoding is an escape.
        let escaped = r#""Other""#;
        assert_eq!(
            serde_json::from_str::<AgentFailureKind>(escaped).expect("escapes decode too"),
            AgentFailureKind::Other
        );

        // An unknown word + escape combination also falls into absorption (not an error).
        let unknown_escaped = r#""SomeKindFromANewerDaemon""#;
        assert_eq!(
            serde_json::from_str::<AgentFailureKind>(unknown_escaped).expect("absorbed"),
            AgentFailureKind::Other
        );

        // The owned value path (`from_value`) is the same — the borrowed `&str` failed here too.
        let owned = serde_json::Value::String("SpawnFailed".to_string());
        assert_eq!(
            serde_json::from_value::<AgentFailureKind>(owned).expect("from_value path"),
            AgentFailureKind::SpawnFailed
        );
    }

    // ── Command bus wire contract (step 2 of ADR-0155 decision 3) ─────────────────────────────────
    //
    // ★What these goldens guard is the hand-written `#[ts(type = …)]`★ — the envelope types are owned by the tool crate, so
    //   they have no ts-rs derive (that crate does not take ts-rs), and so this file writes the TS field by hand. Even if the Rust side's
    //   shape changes, that string silently stays as is, so the actual JSON is pinned here to expose divergence.
    //
    // ★The envelope · reply fields are raw JSON (ADR-0272 decision 3 — the registration item list field follows the same rule, see the registration section above)★ — so the golden of a packet carrying that field is built
    //   from byte literals, and inside the raw field only 「it crosses byte for byte」 is measured. The golden of envelope · reply struct → bytes (the side that
    //   stands against the hand-written TS field) is owned by the edge that unwraps that field, with the same literals (shell `daemon_client::protocol_state` · daemon
    //   `connection_core` tests).
    //
    // ★One literal being used **together** by the relay leg and the sender leg is intended★ — that the envelope · reply of the two legs have the same fields · same
    //   shape is the contract (TRD §3-2 symmetry), and having a different literal per leg would hide that divergence.

    const NIL_ENVELOPE: &str = r#"{"name":"tab.create","request_id":"00000000-0000-0000-0000-000000000000","owner":"shell","proto_ver":1,"args":{"window":"main"}}"#;
    const NIL_FAILED_REPLY: &str = r#"{"request_id":"00000000-0000-0000-0000-000000000000","outcome":{"Err":{"code":"NOT_FOUND","message":"no view 'x'","retry":"never"}}}"#;
    const NIL_OK_REPLY: &str = r#"{"request_id":"00000000-0000-0000-0000-000000000000","outcome":{"Ok":{"view_id":"v1"}}}"#;

    fn raw_slot(json: &str) -> Box<RawValue> {
        RawValue::from_string(json.to_string()).expect("the field literal is valid JSON")
    }

    fn nil_envelope() -> Box<RawValue> {
        raw_slot(NIL_ENVELOPE)
    }

    #[test]
    fn command_request_json_golden_and_roundtrip() {
        let ev = AgentEvent::CommandRequest {
            envelope: nil_envelope(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert_eq!(
            json,
            r#"{"CommandRequest":{"envelope":{"name":"tab.create","request_id":"00000000-0000-0000-0000-000000000000","owner":"shell","proto_ver":1,"args":{"window":"main"}}}}"#,
            "CommandRequest packet outer shape does not match the golden — the envelope shape is guarded by the shell's \
             `the_envelope_slot_unpacks_to_the_bus_envelope_byte_for_byte` · the daemon's \
             `the_relay_repacks_the_envelope_into_the_golden_bytes`"
        );
        let back: AgentEvent = serde_json::from_str(&json).unwrap();
        let AgentEvent::CommandRequest { envelope } = &back else {
            panic!("must decode as CommandRequest");
        };
        assert_eq!(
            envelope.get(),
            NIL_ENVELOPE,
            "the raw field crosses byte for byte"
        );
    }

    /// ★The packet checks only 「is it valid JSON」 and not the envelope shape★ (ADR-0272 decisions 3 · 6) — an envelope with the wrong shape must still
    /// decode as a packet for the edge to handle that branch (shell = no request number · daemon = `Error{request_id: None}`).
    /// Conversely, a field that is not JSON is refused with the whole packet — then it never reaches the edge.
    #[test]
    fn the_envelope_slot_takes_any_json_but_rejects_non_json() {
        for envelope in [r#"{"name":1}"#, "[]", "null", r#""tab.create""#] {
            let sent: AgentCommand =
                serde_json::from_str(&format!(r#"{{"Command":{{"envelope":{envelope}}}}}"#))
                    .expect(
                        "an envelope with the wrong shape must still decode as a Command packet",
                    );
            let AgentCommand::Command { envelope: carried } = sent else {
                panic!("must be Command");
            };
            assert_eq!(carried.get(), envelope);

            let delivered: AgentEvent = serde_json::from_str(&format!(
                r#"{{"CommandRequest":{{"envelope":{envelope}}}}}"#
            ))
            .expect(
                "an envelope with the wrong shape must still decode as a CommandRequest packet",
            );
            let AgentEvent::CommandRequest { envelope: carried } = delivered else {
                panic!("must be CommandRequest");
            };
            assert_eq!(carried.get(), envelope);
        }
        assert!(serde_json::from_str::<AgentCommand>(r#"{"Command":{"envelope":}}"#).is_err());
        assert!(
            serde_json::from_str::<AgentCommand>(r#"{"Command":{}}"#).is_err(),
            "a packet without the envelope field is refused"
        );
    }

    #[test]
    fn command_outcome_json_golden_and_roundtrip() {
        for (reply, golden) in [
            (
                NIL_FAILED_REPLY,
                r#"{"CommandOutcome":{"reply":{"request_id":"00000000-0000-0000-0000-000000000000","outcome":{"Err":{"code":"NOT_FOUND","message":"no view 'x'","retry":"never"}}}}}"#,
            ),
            (
                NIL_OK_REPLY,
                r#"{"CommandOutcome":{"reply":{"request_id":"00000000-0000-0000-0000-000000000000","outcome":{"Ok":{"view_id":"v1"}}}}}"#,
            ),
        ] {
            let json = serde_json::to_string(&AgentCommand::CommandOutcome {
                reply: raw_slot(reply),
            })
            .unwrap();
            assert_eq!(
                json, golden,
                "CommandOutcome packet outer shape does not match the golden — the reply shape is guarded by the shell's \
                 `the_reply_slot_unpacks_to_the_bus_reply_byte_for_byte` · the daemon's \
                 `the_relay_repacks_the_reply_into_the_golden_bytes`"
            );
            let back: AgentCommand = serde_json::from_str(&json).unwrap();
            let AgentCommand::CommandOutcome { reply: carried } = &back else {
                panic!("must decode as CommandOutcome");
            };
            assert_eq!(carried.get(), reply, "the raw field crosses byte for byte");
        }
    }

    /// ★The reply field also checks only 「is it valid JSON」★ (ADR-0272 decisions 3 · 6) — a reply with the wrong shape must still decode as a packet for the edge
    /// to handle that branch (shell = no request number · daemon = `Error{request_id: None}`). The tolerance the TS field advertises (three fields
    /// absent · `null` · fields outside the contract) is measured not by this field but by the edge that unwraps the reply (daemon
    /// `the_outcome_slot_accepts_everything_its_ts_type_advertises`).
    #[test]
    fn the_reply_slot_takes_any_json_but_rejects_non_json() {
        for reply in [
            r#"{"request_id":"00000000-0000-0000-0000-000000000000","outcome":{"Err":{"detail":{"n":1}}}}"#,
            r#"{"request_id":"not-a-uuid"}"#,
            "[]",
            "null",
        ] {
            let sent: AgentCommand =
                serde_json::from_str(&format!(r#"{{"CommandOutcome":{{"reply":{reply}}}}}"#))
                    .expect(
                        "a reply with the wrong shape must still decode as a CommandOutcome packet",
                    );
            let AgentCommand::CommandOutcome { reply: carried } = sent else {
                panic!("must be CommandOutcome");
            };
            assert_eq!(carried.get(), reply);

            let answered: AgentEvent = serde_json::from_str(&format!(
                r#"{{"CommandReply":{{"reply":{reply}}}}}"#
            ))
            .expect("a reply with the wrong shape must still decode as a CommandReply packet");
            let AgentEvent::CommandReply { reply: carried } = answered else {
                panic!("must be CommandReply");
            };
            assert_eq!(carried.get(), reply);
        }
        assert!(serde_json::from_str::<AgentEvent>(r#"{"CommandReply":{"reply":}}"#).is_err());
        assert!(
            serde_json::from_str::<AgentCommand>(r#"{"CommandOutcome":{}}"#).is_err(),
            "a packet without the reply field is refused"
        );
    }

    #[test]
    fn command_json_golden_and_roundtrip() {
        let cmd = AgentCommand::Command {
            envelope: nil_envelope(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            json,
            r#"{"Command":{"envelope":{"name":"tab.create","request_id":"00000000-0000-0000-0000-000000000000","owner":"shell","proto_ver":1,"args":{"window":"main"}}}}"#,
            "Command packet outer shape does not match the golden — the envelope shape is guarded by the shell's \
             `the_envelope_slot_unpacks_to_the_bus_envelope_byte_for_byte` · the daemon's \
             `the_relay_repacks_the_envelope_into_the_golden_bytes`"
        );
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        let AgentCommand::Command { envelope } = &back else {
            panic!("must decode as Command");
        };
        assert_eq!(
            envelope.get(),
            NIL_ENVELOPE,
            "the raw field crosses byte for byte"
        );
    }

    #[test]
    fn command_reply_json_golden_and_roundtrip() {
        for (reply, golden) in [
            (
                NIL_FAILED_REPLY,
                r#"{"CommandReply":{"reply":{"request_id":"00000000-0000-0000-0000-000000000000","outcome":{"Err":{"code":"NOT_FOUND","message":"no view 'x'","retry":"never"}}}}}"#,
            ),
            (
                NIL_OK_REPLY,
                r#"{"CommandReply":{"reply":{"request_id":"00000000-0000-0000-0000-000000000000","outcome":{"Ok":{"view_id":"v1"}}}}}"#,
            ),
        ] {
            let json = serde_json::to_string(&AgentEvent::CommandReply {
                reply: raw_slot(reply),
            })
            .unwrap();
            assert_eq!(
                json, golden,
                "CommandReply packet outer shape does not match the golden — the reply shape is guarded by the shell's \
                 `the_reply_slot_unpacks_to_the_bus_reply_byte_for_byte` · the daemon's \
                 `the_relay_repacks_the_reply_into_the_golden_bytes`"
            );
            let back: AgentEvent = serde_json::from_str(&json).unwrap();
            let AgentEvent::CommandReply { reply: carried } = &back else {
                panic!("must decode as CommandReply");
            };
            assert_eq!(carried.get(), reply, "the raw field crosses byte for byte");
        }
    }

    // ── Usage limits ──

    #[test]
    fn usage_commands_json_golden_and_roundtrip() {
        for (cmd, golden) in [
            (
                AgentCommand::UsageSubscribe {
                    vendors: vec![AgentBackendKind::Claude, AgentBackendKind::Codex],
                },
                r#"{"UsageSubscribe":{"vendors":["claude","codex"]}}"#.to_owned(),
            ),
            (
                AgentCommand::UsageSubscribe { vendors: vec![] },
                r#"{"UsageSubscribe":{"vendors":[]}}"#.to_owned(),
            ),
            (
                AgentCommand::RefreshUsageLimits {
                    vendor: AgentBackendKind::Codex,
                    request_id: RequestId(Uuid::nil()),
                },
                format!(r#"{{"RefreshUsageLimits":{{"vendor":"codex","request_id":"{NIL}"}}}}"#),
            ),
        ] {
            let json = serde_json::to_string(&cmd).unwrap();
            assert_eq!(
                json, golden,
                "usage command wire shape does not match the golden"
            );
            let back: AgentCommand = serde_json::from_str(&json).unwrap();
            assert_eq!(json, serde_json::to_string(&back).unwrap());
        }
        // If absorbed into a default, ⟳ waits for an unpaired answer.
        assert!(serde_json::from_str::<AgentCommand>(
            r#"{"RefreshUsageLimits":{"vendor":"claude"}}"#
        )
        .is_err());
    }

    /// The canonical full snapshot shape is the golden in `domain` — this measures the event envelope and `subscribed`.
    #[test]
    fn usage_limits_updated_json_golden_and_roundtrip() {
        let ev = AgentEvent::UsageLimitsUpdated {
            snapshot: UsageLimitSnapshot {
                vendor: AgentBackendKind::Claude,
                account_key: "default".into(),
                five_hour: None,
                weekly: None,
                model_scoped: vec![],
                plan: None,
                in_flight: false,
                state: crate::domain::UsageVendorState::Ready,
                revision: 3,
            },
            subscribed: vec![AgentBackendKind::Claude, AgentBackendKind::Codex],
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "UsageLimitsUpdated": {
                    "snapshot": {
                        "vendor": "claude",
                        "account_key": "default",
                        "five_hour": null,
                        "weekly": null,
                        "model_scoped": [],
                        "plan": null,
                        "in_flight": false,
                        "state": { "kind": "Ready" },
                        "revision": 3
                    },
                    "subscribed": ["claude", "codex"]
                }
            })
        );
        let back: AgentEvent = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap(), json);
    }
}
