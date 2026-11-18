// Transport — carrier abstraction (ADR-0020 decision 3): a seam so ProtocolClient does not know carrier details (socket/Channel/bytes). The carrier unpacks inbound frames into a normalized InboundMessage (onMessage) and takes commands as AgentCommand wire objects (send).
// The carrier also owns connection state — when it leaves connected, ProtocolClient drops every view to detached (ADR-0046 amend: the re-attach trigger is the authoritative registry, not the socket), so shaking the state would flicker slots to the absent display.

import type { Band } from '../../src-tauri/bindings/Band'
import type { ConnectionState, UsageSnapshotPull } from './agentClient'

/**
 * The **normalized inbound message** the carrier passes up to ProtocolClient; the per-carrier encoding (WS binary frame / TauriOutbound) is already unpacked. Control event = externally-tagged AgentEvent JSON. Auth/Hello is consumed inside the transport and does not come up here.
 */
export type InboundMessage =
  | { kind: 'control'; event: Record<string, unknown> }
  // tag = frame kind (0 = terminal bytes / 1 = StructuredEvent JSON) — the source of truth for the constants is wsFrame.ts.
  | { kind: 'output'; tag: number; agentId: string; epoch: number; seq: number; bytes: Uint8Array }
  // ★Replay boundary marker (ADR-0046)★: normalization of the tag=255 frame src-tauri sends down the same output Channel at the end of each replay; not exposed on the public agentClient surface — only for the frontend's internal state machine. failed=true = this replay ended without completing (deadline/disconnect).
  //   continuesConversation = this incarnation came up to resume a saved conversation (ADR-0226 — not whether it succeeded); always false at a failure boundary.
  //   replayFrom = head of this replay — the daemon SubscribeAck's `replay_from` (the seq actually sent first · the next seq to issue if the ring is empty). ★Do not substitute the buffer's minimum seq★ — wrong when the replay is empty (ADR-0231). Meaningless at a failure boundary (no flush).
  | {
      kind: 'replayBoundary'
      agentId: string
      epoch: number
      gen: bigint
      truncated: boolean
      failed: boolean
      continuesConversation: boolean
      replayFrom: number
    }

/** The carrier abstraction — the only transport surface ProtocolClient depends on (daemon connection only, ADR-0029). */
export interface Transport {
  readonly connectionState: ConnectionState

  /**
   * The shell's connection band (TRD S21 trd-3-2-u5 §4) — `null` = none (a carrier without a shell never has one). A band change is notified through [`onConnectionStateChange`] with the state word, even if the word is unchanged.
   */
  readonly connectionBand: Band | null

  /** Notifies the current state once immediately on registration, then is called on change. Returns a release function. */
  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void

  /** Only 1 callback is kept — re-registering replaces the earlier one (ProtocolClient is the sole router). Returns a release function. */
  onMessage(cb: (msg: InboundMessage) => void): () => void

  /** Send a command (AgentCommand wire object). Ensuring the connection before the call is the caller's job — ProtocolClient awaits ensureReady(). */
  send(payload: unknown): void | Promise<void>

  /**
   * Ensures transport readiness = **attach-only** (ADR-0021 invariant). The command/subscribe path awaits it before every call and **never spawns the daemon** (a key press or resize after turning the daemon off must not respawn it). After an explicit close it rejects immediately — recovery is start() only.
   */
  ensureReady(): Promise<void>

  /**
   * **The explicit spawn entry point** (ADR-0021 §1) — the only path that can bring up the daemon (boot connect / user daemon_start); separate from ensureReady to block "respawn as a side effect of a command".
   */
  start(): Promise<void>

  /** Explicit stop (stop reconnecting + clean up the carrier). Afterwards connectionState='down'. */
  close(): void

  /**
   * View-driven replay request (ADR-0046 F2): requests a **full re-replay of that agent's daemon ring** (not incremental) and returns the assigned gen — a u64 monotonic counter carried as 8-byte BE in the marker frame and compared by ProtocolClient with the fence `request gen ≤ marker gen`, hence BigInt.
   * ★Contract (FIX-6)★: every requestReplay ends with **at least 1 replayBoundary event** or the connection drops (a dropped one emits no marker — a connected re-transition restarts it). Not "exactly 1": a failure marker (deadline) followed by a late success marker of the same gen is a normal path.
   */
  requestReplay(agentId: string): Promise<bigint>

  /**
   * Shell usage cache pull (TRD S21 usage-limit-slot §1-8) — a shell command, so it skips the command path (ensureReady) and the daemon. Contract = exactly [`AgentClient.getUsageSnapshot`] (a carrier with no shell = `{socketEpoch: 0, snapshots: []}`).
   */
  getUsageSnapshot(): Promise<UsageSnapshotPull>
}
