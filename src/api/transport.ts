// Transport — carrier (transport) abstraction (ADR-0020 decision 3).
//
// A seam so ProtocolClient does not know carrier details (socket/Channel/bytes): the carrier unpacks inbound frames into
// a normalized InboundMessage and passes it up (onMessage), and takes commands as AgentCommand wire objects (send).
// Connection state is also owned by the carrier — when it leaves connected, ProtocolClient drops every view to detached
// (ADR-0046 amend: the trigger to re-attach is the authoritative registry, not the socket), so shaking the state makes
// slots flicker to the absent display each time.

import type { Band } from '../../src-tauri/bindings/Band'
import type { ConnectionState, UsageSnapshotPull } from './agentClient'

/**
 * The **normalized inbound message** the carrier passes up to ProtocolClient. The per-carrier encoding (WS binary frame /
 * TauriOutbound) was already unpacked by the transport. The control event = externally-tagged AgentEvent JSON.
 *
 * Auth/Hello is consumed inside the transport (handshake) and does not come up here.
 */
export type InboundMessage =
  | { kind: 'control'; event: Record<string, unknown> }
  // tag = frame kind (0 = terminal bytes / 1 = StructuredEvent JSON) — the source of truth for the constants is wsFrame.ts.
  | { kind: 'output'; tag: number; agentId: string; epoch: number; seq: number; bytes: Uint8Array }
  // ★Replay boundary marker (ADR-0046)★: the normalization of the tag=255 frame that src-tauri sends down the same output Channel
  //   at the end of each replay. It is not exposed on the public agentClient surface — the marker is only for the frontend's internal state machine
  //   (Designer review requirement). failed=true means this replay ended without completing (deadline/disconnect).
  //   continuesConversation = this incarnation came up to resume a saved conversation (ADR-0226 — not whether it succeeded).
  //   A failure boundary is always false.
  //   replayFrom = the head of this replay — the daemon SubscribeAck's `replay_from` (the seq actually sent first · the next seq to be issued
  //   if the ring is empty). ★Do not substitute the buffer's minimum seq★ — it is wrong when the replay is empty (ADR-0231).
  //   Meaningless at a failure boundary (no flush).
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
   * The shell's connection band (TRD S21 trd-3-2-u5 §4) — `null` = none. A carrier without a shell never has one.
   * A band change is notified through [`onConnectionStateChange`] with the state word, even when the word is unchanged.
   */
  readonly connectionBand: Band | null

  /** Notifies the current state once immediately on registration, then is called on change. Returns a release function. */
  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void

  /** Only 1 callback is kept — re-registering replaces the earlier one (ProtocolClient is the sole router). Returns a release function. */
  onMessage(cb: (msg: InboundMessage) => void): () => void

  /** Send a command (AgentCommand wire object). Ensuring the connection before the call is the caller's job — ProtocolClient awaits ensureReady(). */
  send(payload: unknown): void | Promise<void>

  /**
   * Ensures transport readiness = **attach-only** (ADR-0021 invariant). The command/subscribe path awaits it before every call — this
   * path **never spawns the daemon** (a key press or resize after turning the daemon off must not respawn it).
   * After an explicit close it rejects immediately — recovery is start() only.
   */
  ensureReady(): Promise<void>

  /**
   * **The explicit spawn entry point** (ADR-0021 §1) — the only path that can bring up the daemon (boot connect / user
   * daemon_start). Separated from the command path (ensureReady) to block "respawn as a side effect of a command".
   */
  start(): Promise<void>

  /** Explicit stop (stop reconnecting + clean up the carrier). Afterwards connectionState='down'. */
  close(): void

  /**
   * View-driven replay request (ADR-0046 F2) — requests a **full re-replay of that agent's daemon ring** (not incremental)
   * and retrieves the assigned gen. gen is a u64 monotonic counter — the marker frame carries it as 8-byte BE and ProtocolClient compares
   * it with the fence `요청 gen ≤ 마커 gen`, so we match the width and unify on BigInt.
   *
   * ★Contract (FIX-6 correction)★: every requestReplay ends with **at least 1 replayBoundary event** or the connection
   * drops (when dropped no marker is emitted — a connected re-transition restarts it). It is not "exactly 1": the failed→success pair, where a late success marker
   * of the same gen follows a failure marker (deadline), is a normal path.
   */
  requestReplay(agentId: string): Promise<bigint>

  /**
   * Shell usage cache pull (TRD S21 usage-limit-slot §1-8) — it is a shell command, so it does not take the command path (ensureReady) and does not
   * go through the daemon. The contract is exactly [`AgentClient.getUsageSnapshot`] (a carrier with no shell = `{socketEpoch: 0, snapshots: []}`).
   */
  getUsageSnapshot(): Promise<UsageSnapshotPull>
}
