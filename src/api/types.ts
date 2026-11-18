// Rust backend type mirror — LLD §3 / frontend-integration-lld.md §1; discriminated unions match the backend #[serde(tag="type")].

export type AgentStatus =
  | { type: 'Running' }
  | { type: 'Exiting' }
  | { type: 'Exited'; code: number | null }
  | { type: 'Failed'; message: string }
  | { type: 'Killed' }

export interface PtyEvent {
  agent_id: string
  seq: number
  /** Session epoch — same as the WS binary frame header. */
  epoch: number
  data_b64: string
}

// ── Capabilities (Rust Capabilities mirror, snake_case) ──────────────────────────

export interface InputCaps {
  raw: boolean
  message: boolean
  attachment: boolean
}

export interface OutputCaps {
  terminal_bytes: boolean
  /** Whether it is a structured stream (NDJSON) — the basis for the renderer split (xterm vs RichSlot) (ADR-0044) */
  structured: boolean
  markdown: boolean
  tool_events: boolean
  usage: boolean
}

export interface ControlCaps {
  resize: boolean
  interrupt: boolean
  cancel: boolean
  graceful_shutdown: boolean
}

export interface SessionCaps {
  resume: boolean
  snapshot: boolean
  cwd_env: boolean
}

export interface ModelCaps {
  select: boolean
  temperature: boolean
  max_tokens: boolean
}

export interface Capabilities {
  input: InputCaps
  output: OutputCaps
  control: ControlCaps
  session: SessionCaps
  model: ModelCaps
}

export interface AgentInfo {
  id: string
  /** Canonical display name — the backend fills it with `display_name ?? cwd basename` (ADR-0101). Not profile.name. */
  name: string
  cwd: string
  status: AgentStatus
  cols: number
  rows: number
  /**
   * ★**Opaque marker** of one incarnation★ — a random number drawn per incarnation, so **its order means nothing**: compare only match/mismatch (never derive "newer" from magnitude). The frontend uses it only to decide whether the output stream being read is the same stream as before — ★never as a resubscribe/remount trigger★: exit means leaving the roster, so passing it down as a prop drops the value at exit and erases the screen **before the replay arrives** (the daemon ring is gone, unrecoverable) (ADR-0007).
   */
  epoch: number
  capabilities: Capabilities
}

export interface AgentStatusChanged {
  id: string
  status: AgentStatus
  /** The marker of the incarnation that issued this notification — used to drop delayed notifications of an old session (match/mismatch only, AgentInfo.epoch). */
  epoch: number
}

// ── S9: profiles + restore ──────────────────────────────────────────────────────────

/** Agent output mode — Terminal=PTY, StreamJson=JSON-based non-terminal. The exchange shape is decided by the backend. (ADR-0044) */
export type AgentOutputFormat = 'Terminal' | 'StreamJson'

/**
 * Agent launch command — matches the backend #[serde(tag="kind")].
 * ★Name collision★: the Rust `protocol` has a same-named type meaning something else (the wire command to the daemon). The counterpart of this type is `AgentSpawnCommand` — import `AgentSpawnCommand.ts`, not `AgentCommand.ts`, when wiring generated bindings (tsc catches it only if a test fixture builds this value).
 */
export type AgentCommand =
  | { kind: 'Claude'; extra_args: string[]; output_format: AgentOutputFormat }
  | { kind: 'Shell'; program: string; args: string[] }
  | { kind: 'Codex'; extra_args: string[]; output_format: AgentOutputFormat }

/**
 * The backend a spawn packet picks — mirror of wire `AgentBackendKind`. ★Absence is an error, not a default★: the daemon refuses a spawn packet without this word (not picking is told apart from picking claude).
 * ★Not for renderer selection★ — `capabilities.output.structured` alone decides which slot launches (terminal · rich · DOM); branching here would make two places decide the same capability. Backend labelling **within** one renderer (title · mark · tint) is not banned, but it uses the profile's `command.kind` (`components/slot/richBranding.ts`), not this type — this is the value the spawn packet carries, not what the screen reads.
 */
export type AgentBackendKind = 'claude' | 'codex'

export type RestartPolicy = 'Never' | 'OnCrash' | 'Always'

/**
 * Kind of "the last failure" — mirror of wire `AgentFailureKind` (ADR-0172). **A separate axis** from AgentStatus: a running agent may hold a last failure, where the screen's "running wins" rule (ADR-0173) applies.
 */
export type AgentFailureKind =
  | 'NoConversationToResume'
  | 'SpawnFailed'
  | 'EarlyExitAfterResume'
  | 'Other'

/** Persistent profile — the agents.json unit. No credentials in env (stored in plain text) */
export interface AgentProfile {
  id: string
  name: string
  /** User display-name override (ADR-0061, tree rename): Some → shown as is, null → cwd basename. Preferred over `name` (the CreateProfile name / ad-hoc cwd string). Mirror of wire `AgentProfile.display_name` (#[serde(default)] — old agents.json is null). */
  display_name: string | null
  /** Parent profile id of the tree hierarchy (ADR-0072): Some(pid) → child of pid, null → root. One level only (the parent must be a root) — validated by backend ProfileRegistry::try_reparent + write boundary normalization. Mirror of wire `AgentProfile.parent_id` (#[serde(default)]). */
  parent_id: string | null
  command: AgentCommand
  cwd: string
  env: [string, string][]
  backend_session_id: string | null
  old_session_ids: string[]
  epoch: number
  auto_restore: boolean
  restart_policy: RestartPolicy
  /** Crash guard counter (reset to 0 on a manual restart — behaviour TODO) */
  restart_count: number
  /** Reason for Failed (auto-restore suspend) — persists across cold boots (ADR-0016). Behaviour TODO */
  failed_reason: string | null
  /** Kind of this entry's last activation failure (ADR-0172); null = none. Lives only in daemon memory — not in agents.json, lost on daemon restart (survives an app window restart). Mirror of wire `AgentProfile.last_failure` (#[serde(default)]). Kind→{retryable · text · action} table = `src/components/agent/failureKinds.ts`. */
  last_failure: AgentFailureKind | null
  created_at: number
  last_active: number
  /** Last process launch time (for records/debugging, not used for reset decisions) */
  last_start_at: number | null
}

// ── presets (ADR-0061) ────────────────────────────────────────────────────────────

/** Persistent preset — presets.json unit (daemon-owned, ADR-0061), mirror of wire `Preset`. A separate axis from profiles: preset = frequently used cwd bookmark (no spawn); profile = the real agent storage unit. */
export interface Preset {
  id: string
  /** Normalized cwd (the JSON representation of PathBuf = a string). Without a name override, derived from this value's basename. */
  cwd: string
  /** Display-name override (ADR-0061): Some → shown as is, null → cwd basename. Mirror of wire `Preset.name` (#[serde(default)]). The rename command sets/clears it. */
  name: string | null
}

/** Restore outcome — restore-result event, #[serde(tag="type")] */
export type RestoreOutcome =
  | { type: 'Resumed' }
  | { type: 'Started' }
  | { type: 'FreshFallback'; old_sid: string | null; new_sid: string; reason: string }
  | { type: 'Blocked'; reason: string }
  | { type: 'Failed'; reason: string }

/** The `result` field of the restore-result Tauri event payload */
export interface RestoreReport {
  agent_id: string
  epoch: number
  outcome: RestoreOutcome
}
