// AgentClient — the single control surface the frontend depends on (daemon-design §3-a).
//
// Components and stores do not call invoke/Channel/WS directly. Single implementation: ProtocolClient (protocol semantics) + a carrier (TauriTransport, ADR-0020); daemon-only (ADR-0029).
// Transport and base64/decoding stay inside the transport — the interface exposes only decoded byte chunks.

import type { QueuedInputRow } from '../../crates/engram-dashboard-protocol/bindings/QueuedInputRow'
import type { UsageLimitSnapshot } from '../../crates/engram-dashboard-protocol/bindings/UsageLimitSnapshot'
import type { Band } from '../../src-tauri/bindings/Band'
import type {
  AgentBackendKind,
  AgentInfo,
  AgentProfile,
  AgentStatus,
  AgentOutputFormat,
  Preset,
  RestoreReport,
} from './types'

/**
 * Answer of the queued input list query (ADR-0231) — same shape and words as the bus `agent.listQueuedInputs`. `inputs` = entries before their outcome, oldest first.
 * Word fields (`state` · `cancel.answer`) are strings — a newer daemon's words may come; an unknown value skips only that row.
 * `as_of_seq` = seq of the last list event the row reduced (`null` = none), matched against live events by the re-attach check and compared only within the `epoch` incarnation.
 * `stopped_after_error` = that incarnation is stopped after an error (mail is stopped).
 */
export interface QueuedInputListing {
  inputs: QueuedInputRow[]
  as_of_seq: number | null
  epoch: number
  stopped_after_error: boolean
}

/**
 * Input lease refusal text — **byte-identical** to `INPUT_LOCKED_REFUSAL` in the daemon `connection_core.rs` (a test measures it).
 * No `CODE:` prefix (same text as the `WriteStdin` refusal), so it is recognized by the text itself; the bus-side code of the same refusal is `CONFLICT`.
 */
// ADR-0231
export const INPUT_LOCKED_REFUSAL = 'input locked by another viewer; acquire first'

export type ConnectionState = 'connected' | 'reconnecting' | 'down'

/**
 * One copy of the shell usage cache (TRD S21 usage-limit-slot §1-8) — one highest revision per vendor received on the current socket, relative time fields moved by the time elapsed since receipt.
 * `socketEpoch` = that socket's marker (ADR-0195 — does not rewind within the shell process, so magnitude is socket order; unlike the incarnation marker, ADR-0163).
 * `0` = the shell has no socket: `snapshots` is empty and the receiver keeps the marker it holds.
 */
export interface UsageSnapshotPull {
  socketEpoch: number
  snapshots: UsageLimitSnapshot[]
}

/** A decoded output chunk — transport-independent (base64/binary frames are already unpacked inside the client). */
export interface OutputChunk {
  /** seq issued by the agent OutputCore (monotonically increasing). */
  seq: number
  /**
   * frame kind (wsFrame.ts): 0=terminal raw bytes (xterm write) / 1=StructuredEvent JSON (ADR-0045 tag1).
   * A tag field, not a separate callback: seq dedup, the epoch guard and the pre-subscribe buffer then need not know the tag (one seq space, one delivery path).
   */
  tag: number
  /** raw payload — terminal bytes for tag0, StructuredEvent JSON UTF-8 bytes for tag1. */
  bytes: Uint8Array
}

/** Unsubscribe handle — must be called (on unmount/resubscribe). Internally it also cleans up the transport. */
export interface OutputSubscription {
  unsubscribe: () => void
}

/**
 * View output phase — the `onState` notification value and the value set of [`ViewOutputState.phase`].
 *
 * - `detached` — **no agent to attach to**: the connection dropped, or the known roster does not have that id. Waits without sending requests; drawn content is **not erased**
 *   (the intent "this slot watches this agent" outlives the connection). "The roster itself is not known yet" (boot · webview reload · roster query failure) is not this value —
 *   there it just sends requests without a notification (unknown ≠ absent; merging them blocks output on one failed query).
 *   So receiving this while connected means the roster said that agent does not exist.
 * - `buffering` — accumulating (waiting for the replay boundary). Must not clear: it is not yet known what history will come. The same incarnation's overlap is absorbed by dedup;
 *   for a different incarnation the clear is issued separately by `onReset`.
 * - `live` — direct delivery.
 * - `error` — the re-request ladder is exhausted. Like `detached`, **nothing comes**, so the consumer must show it on screen — otherwise the slot stays a blank board with no signal.
 *   The screen content is not erased here either (clearing is `onReset` alone).
 */
export type ViewPhase = 'detached' | 'buffering' | 'live' | 'error'

/** An extra fact that comes only with the `'live'` notification (ADR-0226); when absent it reads as `continuesConversation: false`. */
export interface ReplayLiveInfo {
  /**
   * This incarnation was launched to resume a saved conversation — **not** that the resume succeeded (history may be 0 entries). Old daemons and shells do not carry it, so it is false.
   */
  continuesConversation: boolean
  /**
   * Marker of the incarnation that adopted this replay (opaque — match/mismatch only). ProtocolClient always carries it; without it the view skips the queued input re-attach check (ADR-0231).
   */
  epoch?: number
}

/**
 * View clear signal — **another incarnation's history is delivered from now on**.
 *
 * Receiver's duty: erase what is drawn and reset **its own seq guard too** — only the guard reset draws over, only the screen erased drops every new frame (a new incarnation numbers from 0 again).
 *
 * It comes at **arrival** time, not request time: the history follows on the same tick, so no empty screen shows. When the replay is refused or times out it does **not** come and the
 * previous incarnation's screen stays (intended — right after a daemon restart).
 */
export type ViewResetFn = () => void

/**
 * Per-view replay state snapshot (§5 LLM control surface — ADR-0046): lets the LLM/automation find error (ladder exhausted), detached etc. by type and decide on restarting. Minimal exposure.
 */
export interface ViewOutputState {
  agentId: string
  phase: ViewPhase
  /** Number of frames accumulated while buffering (debug · observation). */
  buffered: number
  /** Frames held after a gap in live (ADR-0231). Non-zero = the view waits for a seq that has not come and draws nothing after it; the wait has no time limit, so this is the only window onto the stall. */
  held: number
  /** Number of re-request ladder attempts (0 = no re-request yet). */
  attempts: number
}

/** The single surface for agent control/subscription: human UI clicks and (future) LLM calls share one entry point (§5). The implementation owns idempotency and retry policy of every side-effect method. */
export interface AgentClient {
  // ── connection state ──────────────────────────────────────────────────────────────
  readonly connectionState: ConnectionState
  /** The shell's connection band (TRD S21 trd-3-2-u5 §4-1) — `null` = none. The shell decides it; the frontend only renders it. A carrier without a shell never has one. */
  readonly connectionBand: Band | null
  /**
   * Cause text of one failed start attempt (human-readable, e.g. the unwritable data folder of ADR-0134), or null. Only a cause shown under band ① (TRD S21 trd-3-2-u5 R14): it never raises a band.
   * It rides the connection state surface because a daemon that cannot start (data folder not writable) is otherwise indistinguishable from a causeless timeout (ADR-0134; §5 one control surface).
   * Cleared on `connected`, when band ② (`version`) or band ① `refused` arrives, and when a new start attempt begins.
   */
  readonly connectionError: string | null
  /**
   * Subscribes to state changes; returns the release function. Also notified when [`connectionBand`] or [`connectionError`] changes with the state word unchanged, so the subscriber can redraw.
   */
  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void
  /**
   * Records the cause of a failed start (null = clear). Ignored while connected, under band ② and under band ① `refused` (TRD S21 trd-3-2-u5 R21): those arrivals are the automatic clears, so a cause written
   * while one holds would never be cleared and would surface later as a stale cause of an unrelated ① (e.g. after a tray stop). Public on `window.__ENGRAM_AGENT__`, so the LLM and cdp reach it too.
   *
   * Exactly one writer — `DaemonControl.start()` (boot bootstrap, `window.__ENGRAM_DAEMON__.start()`, the band's 「Reconnect」 and the LLM `connection.reconnect` all go through it).
   * Known gaps — paths not covered:
   * - `AgentClient.connect()` — goes as far as a spawn but not through `DaemonControl`; a direct LLM/cdp call does not surface the failure reason.
   * - The native tray's "start daemon" — Rust calls `discovery::ensure_daemon` directly (the tray shows it with its own icon state).
   */
  reportConnectionError(reason: string | null): void

  /**
   * **Explicit connection (spawn allowed)** — ADR-0021 §1. Called once at boot / by the user's daemon_start (DaemonControl.start). The only place the daemon is spawned when absent
   * (the command path `ensureReady` is attach-only). Resets the state stopped by reconnecting (closedByUser/attempt).
   */
  connect(): Promise<void>
  /**
   * **Explicit disconnect (stops reconnecting, ADR-0021 note3)** — closedByUser=true settles on 'down' at once so the transport does not keep retrying a gone daemon (the shell link retries
   * without limit — ADR-0346). ProtocolClient itself (subscription router, callback registry) stays, so a later connect revives it (unlike close).
   */
  disconnect(): void

  // ── output subscription ──────────────────────────────────────────────────────────────
  /**
   * Per-view (slot) output subscription (ADR-0046). viewId = slot id — N views on the same agentId each have independent progress. Decoded bytes go to onChunk.
   * onState (optional) = phase notification ([`ViewPhase`]), with [`ReplayLiveInfo`] as the second argument only for `'live'`. onReset (optional) = clear signal ([`ViewResetFn`]).
   * Released with the returned handle's unsubscribe.
   */
  subscribeOutput(
    viewId: string,
    agentId: string,
    onChunk: (chunk: OutputChunk) => void,
    onState?: (state: ViewPhase, info?: ReplayLiveInfo) => void,
    onReset?: ViewResetFn,
  ): Promise<OutputSubscription>

  /** Per-view replay state query (§5 LLM control surface — ADR-0046), to observe error exhaustion (3 failed re-requests), stuck buffering etc. null for an unknown viewId. */
  getViewOutputState(viewId: string): ViewOutputState | null

  // ── status/list/restore events ─────────────────────────────────────────────────────
  /** Replaces the authoritative agent list (the basis for existence/removal decisions). */
  onAgentListUpdated(cb: (agents: AgentInfo[]) => void): () => void
  /** Individual status update (does not remove from the list). */
  onStatusChanged(cb: (id: string, status: AgentStatus, epoch: number) => void): () => void
  /** Boot restore result (S9). */
  onRestoreResult(cb: (report: RestoreReport) => void): () => void
  /** Live update of the profile list (create/delete/activate broadcast, AgentEvent::ProfileListUpdated routing); the store mirror is updated. */
  onProfileListUpdated(cb: (profiles: AgentProfile[]) => void): () => void
  /** Live update of the preset list (ADR-0061 — same shape as the profile version); the store mirror is updated (AgentEvent::PresetListUpdated routing). */
  onPresetListUpdated(cb: (presets: Preset[]) => void): () => void
  /**
   * Usage snapshot broadcast (TRD S21 usage-limit-slot §1-8). Only the ones the shell picked for this window come. `socketEpoch` = that of [`UsageSnapshotPull`]; daemon frames of the direct carrier (no shell) carry none and come as `0`.
   * An old copy of the same vendor can come late (no ordering guarantee vs the pull) — telling them apart is the receiver's job.
   */
  onUsageLimitsUpdated(cb: (snapshot: UsageLimitSnapshot, socketEpoch: number) => void): () => void

  // ── usage ────────────────────────────────────────────────────────────────
  /**
   * ⟳ — makes that vendor's usage be queried again now. resolve = the daemon received it (`Ack`), not that a value came (it comes via [`onUsageLimitsUpdated`]).
   * The daemon merges overlapping requests into one and does not query within a refusal (429) deadline. Rejects when the connection drops.
   */
  refreshUsageLimits(vendor: AgentBackendKind): Promise<void>
  /**
   * Pulls the shell usage cache (no daemon round trip). Call it **after** setting up [`onUsageLimitsUpdated`] — calling first misses the broadcasts in between.
   * On a carrier without a shell, always `{socketEpoch: 0, snapshots: []}`.
   */
  getUsageSnapshot(): Promise<UsageSnapshotPull>

  // ── commands ──────────────────────────────────────────────────────────────────
  /** ad-hoc spawn — only cwd and backend, no profile beforehand (the daemon makes a profile on the spot and puts it on the roster). `backend` is **required** — without it the daemon refuses (no default). */
  spawnAgent(cwd: string, backend: AgentBackendKind): Promise<AgentInfo>
  killAgent(agentId: string): Promise<void>
  interruptAgent(agentId: string): Promise<void>
  writeStdin(agentId: string, data: Uint8Array): Promise<void>
  resizePty(agentId: string, cols: number, rows: number): Promise<void>
  getAgents(): Promise<AgentInfo[]>
  getSnapshot(agentId: string): Promise<unknown[]>
  /** Queued input list query (ADR-0231 — WS `ListQueuedInputs`). Does not look at the input lease. A sleeping agent rejects with `NOT_FOUND: …`. */
  listQueuedInputs(agentId: string): Promise<QueuedInputListing>
  /**
   * Cancels one queued input — same command as the on-screen ✕ (WS `CancelQueuedInput`). resolve = outcome word (`requested` = passed on, the outcome comes as a list event ·
   * `cancelled` = cancelled right away · a newer daemon may send an unknown word). reject = `NOT_FOUND: …` (unknown or already settled id) · `INTERNAL: …` ·
   * the input lease refusal is the [`INPUT_LOCKED_REFUSAL`] text as is, with no code prefix (same as `WriteStdin`).
   */
  cancelQueuedInput(agentId: string, inputId: string): Promise<string>

  // ── profile CRUD ────────────────────────────────────────────────────────────
  listProfiles(): Promise<AgentProfile[]>
  /**
   * claude profile creation. outputFormat picks the render mode (ADR-0044): 'Terminal'=PTY (xterm), 'StreamJson'=headless NDJSON (RichSlot). Default 'Terminal' (wire is `#[serde(default)]`).
   * The §5 control surface (cdp/console) spawns json agents with this argument.
   */
  createClaudeProfile(
    name: string,
    cwd: string,
    extraArgs: string[],
    env: [string, string][],
    autoRestore: boolean,
    outputFormat?: AgentOutputFormat,
  ): Promise<AgentProfile>
  /**
   * codex profile creation — like `createClaudeProfile`, only registers, does not spawn. outputFormat: 'Terminal'=interactive TUI, 'StreamJson'=resident JSON server (codex app-server). Default 'Terminal'.
   */
  createCodexProfile(
    name: string,
    cwd: string,
    extraArgs: string[],
    env: [string, string][],
    autoRestore: boolean,
    outputFormat?: AgentOutputFormat,
  ): Promise<AgentProfile>
  deleteProfile(agentId: string): Promise<void>
  spawnProfile(agentId: string, resume: boolean): Promise<AgentInfo>
  setProfileAutoRestore(agentId: string, autoRestore: boolean): Promise<void>
  /**
   * Set/clear the profile display-name override (ADR-0061, §5): string → store, null → clear (back to the cwd basename). trim, empty-string refusal and unchanged-value skipping are the caller's (UI)
   * job — only a valid value or explicit null comes here. Reply=Ack(void); the display name update comes with the next ProfileListUpdated broadcast (no optimistic update). Error for an unknown id.
   */
  renameProfile(agentId: string, name: string | null): Promise<void>
  /**
   * Re-parent a profile (ADR-0072, §5). Moves childId under parentId (null → root). One level only — the backend refuses cycle/self/2 levels/non-existent parent (Error).
   * Reply=Ack(void); the hierarchy update comes with the next ProfileListUpdated broadcast (no optimistic update). Human drag and LLM calls share the entry point.
   */
  reparentProfile(childId: string, parentId: string | null): Promise<void>

  // ── preset CRUD (ADR-0061 — cwd bookmarks, no spawn) ─────────────────────────────
  /** Queries all saved presets. Collected through the PresetList-only reply (request_id echo). */
  listPresets(): Promise<Preset[]>
  /**
   * Preset creation (cwd bookmark). The name is not passed — the backend stores only {id,cwd}; the frontend derives the display name from the cwd basename (ADR-0061).
   * Reply=Ack(void); the created Preset reaches the store only through the PresetListUpdated broadcast (unlike CreateProfile returning Created{profile}).
   */
  createPreset(cwd: string): Promise<void>
  /** Preset deletion (agents live on regardless — ADR-0061). no-op (Ack) for an unknown id. */
  deletePreset(id: string): Promise<void>
  /**
   * Set/clear the preset display-name override (ADR-0061, §5): string → store, null → clear. trim, empty-string refusal and unchanged-value skipping are the caller's (UI) job.
   * Reply=Ack(void); the update comes with the next PresetListUpdated broadcast (no optimistic update). no-op (Ack) for an unknown id.
   */
  renamePreset(id: string, name: string | null): Promise<void>
}
