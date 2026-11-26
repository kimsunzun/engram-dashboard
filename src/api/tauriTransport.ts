// TauriTransport — the Tauri (app.emit/invoke/Channel) carrier implementation of Transport (ADR-0036).
// (Fix-C / Fix-D labels in the body and tests = S14 review fix anchors pointing at the corresponding guard comments.)
//
// The frontend talks to the Rust DaemonClient (src-tauri) through this transport; 3 communication planes:
//  ① control broadcast (list/status/profile): Rust `app.emit(event)` → `listen` → control InboundMessage.
//  ② command upstream: `invoke('forward_daemon_command', { cmd })` → Rust → daemon WS.
//  ③-a reply downstream (answer to ②): forward_daemon_command return (reply AgentEvent) → control InboundMessage.
//  ③-b output downstream: per-window Tauri Channel (`subscribe_output` invoke) → raw bytes → decodeOutputFrame → output InboundMessage.
//
// Same shape as WsTransport: reply/output are raised as the same InboundMessage shape, so ProtocolClient (unmodified) does not know whether the carrier is WS or Tauri.
//
// Connection state = Rust emit is the single source of truth (Fix-C ①): the `daemon-connection-state` event (u5) from the Rust DaemonClient.
// The frontend (doConnect) never makes it connected on its own — Rust knows Hello receipt/reconnect/disconnect authoritatively, and a guess from the invoke resolve
// would diverge (stale discard · reconnecting). doConnect only attempts the connection (invoke + output Channel registration); u5 drives the state.

import { Channel, invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import type { UsageLimitSnapshot } from '../../crates/engram-dashboard-protocol/bindings/UsageLimitSnapshot'
import type { Band } from '../../src-tauri/bindings/Band'
import type { ConnectionState, UsageSnapshotPull } from './agentClient'
import type { InboundMessage, Transport } from './transport'
import { decodeOutputFrame, decodeReplayMarker } from './wsFrame'

// Field names of the two usage payloads the shell gives the webview — no ts bindings (elements are protocol types), so matched by hand. Canonical =
//   `src-tauri/src/commands/usage.rs` `UsageSnapshotReply` · `src-tauri/src/daemon_client/events.rs` `UsageLimitsUpdatedPayload`. A renamed field silently becomes `undefined` here.
interface ShellUsageSnapshotReply {
  socket_epoch: number
  snapshots: UsageLimitSnapshot[]
}
interface ShellUsageLimitsUpdated {
  labels: string[]
  socket_epoch: number
  snapshot: UsageLimitSnapshot
}

/**
 * One `daemon-connection-state` payload or `daemon_connection_state` reply (`ConnectionStatus`), read defensively: an unknown `state` reads as `down`
 * and an unknown `band.kind` as band ① without a cause, each with a warning (TRD S21 trd-3-2-u5 §4-2) — a word the shell adds later is never misread as `connected` or as no band.
 */
function parseConnectionStatus(raw: unknown): { state: ConnectionState; band: Band | null } {
  const status = isRecord(raw) ? raw : {}
  let state: ConnectionState
  if (status.state === 'connected' || status.state === 'reconnecting' || status.state === 'down') {
    state = status.state
  } else {
    console.warn(`[TauriTransport] unknown connection state '${String(status.state)}' — treated as down`)
    state = 'down'
  }
  return { state, band: parseBand(status.band) }
}

function parseBand(raw: unknown): Band | null {
  if (raw === null || raw === undefined) return null
  const band = isRecord(raw) ? raw : {}
  if (band.kind === 'not_connected') return { kind: 'not_connected', refused: band.refused === true }
  if (band.kind === 'version') return { kind: 'version', can_stop: band.can_stop === true }
  console.warn(`[TauriTransport] unknown connection band '${String(band.kind)}' — treated as not connected`)
  return { kind: 'not_connected', refused: false }
}

function isRecord(raw: unknown): raw is Record<string, unknown> {
  return typeof raw === 'object' && raw !== null
}

function sameBand(a: Band | null, b: Band | null): boolean {
  if (a === null || b === null) return a === b
  if (a.kind === 'not_connected' && b.kind === 'not_connected') return a.refused === b.refused
  if (a.kind === 'version' && b.kind === 'version') return a.can_stop === b.can_stop
  return false
}

export class TauriTransport implements Transport {
  private _state: ConnectionState = 'down'
  private _band: Band | null = null
  private stateListeners = new Set<(s: ConnectionState) => void>()
  private messageCb: ((msg: InboundMessage) => void) | null = null

  private unlisten: Array<() => void> = []

  // Output Channel (③-b): the per-window Channel carrying all agent output of this window (agent_id tag embedded in the frame), registered once in the Rust registry
  //   with `subscribe_output` on connected. Receives raw bytes, raises them via decodeOutputFrame. #13133: cleanup is `delete channel.onmessage`, not a null assignment.
  private outputChannel: Channel<ArrayBuffer> | null = null

  // ★Output Channel registration single-flight (FIX 6)★: overlapping registrations each make a Channel and delete the old onmessage; if an older invoke reaches Rust *later*,
  //   Rust holds the Channel whose onmessage was deleted and output is silently cut off (the arrival order is uncontrollable). So registration is serialized: in progress the promise is shared,
  //   a request meanwhile only sets the pending flag and re-registers exactly once after completion.
  private outputChannelInflight: Promise<void> | null = null
  private outputChannelRerun = false
  // Whether this window's output Channel has finished registering in the current generation (`subscribe_output` resolved). Lowered at registration start and by close(); replay requests check it (`awaitOutputChannel`).
  private outputChannelRegistered = false

  private connectPromise: Promise<void> | null = null

  private closedByUser = false

  // ★Connection generation (Fix-C ①)★: +1 on every close(). An in-flight doConnect captures it (myGen) and re-checks after the invoke resolves, so a stale doConnect that completed late
  //   cannot register a zombie output Channel. State revival is already blocked structurally (Rust emit is the single source; doConnect does not setStatus).
  private generation = 0

  // State application version (selfHeal pull-vs-event race guard, FIX 5): +1 each time applyConnectionStatus applies a state. selfHeal captures it before the invoke and
  //   discards the pull result if it changed — the pull snapshot is older than any event that cut in (event→pull would otherwise overwrite the new state).
  private stateVersion = 0

  get connectionState(): ConnectionState {
    return this._state
  }

  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void {
    this.stateListeners.add(cb)
    // Notify the current state once right at registration (same shape as WsTransport — ProtocolClient learns the initial state).
    cb(this._state)
    return () => {
      this.stateListeners.delete(cb)
    }
  }

  onMessage(cb: (msg: InboundMessage) => void): () => void {
    this.messageCb = cb
    return () => {
      if (this.messageCb === cb) this.messageCb = null
    }
  }

  /** The shell's connection band (TRD S21 trd-3-2-u5 §4) — `null` = none. Listeners hear a band change as a call with the state word, even when the word did not change. */
  get connectionBand(): Band | null {
    return this._band
  }

  private setStatus(s: ConnectionState, band: Band | null): void {
    if (this._state === s && sameBand(this._band, band)) return
    this._state = s
    this._band = band
    for (const cb of this.stateListeners) cb(s)
  }

  // ── Applying the shell's connection status (the single source of truth) ────────────────────────
// The one path the `daemon-connection-state` event handler and the self-heal pull share (both carry the shell's `ConnectionStatus` {state, band}).
//
// Idempotent (FIX 5): when already connected (wasConnected) the output Channel is not registered again — an overlapping self-heal pull and real event register once per transition.
//   The self-heal feeds its reply here at once; an event landing between its query and reply makes the stateVersion compare drop the pull result (the event wins; see selfHeal).
  private applyConnectionStatus(raw: unknown): void {
    const { state, band } = parseConnectionStatus(raw)
    // Version bump (FIX 5): +1 on every applied call even when setStatus is a no-op (same state) — the fact that an event came must be marked for selfHeal.
    this.stateVersion += 1
    const wasConnected = this._state === 'connected'
    this.setStatus(state, band)
    if (state === 'connected' && !wasConnected) {
      this.registerOutputChannel().catch((err: unknown) => {
        console.warn('[TauriTransport] output Channel registration failed:', err)
      })
    }
  }

  // ── reload self-recovery (Fix-D — one pull query) ───────────────────────────────────
// Why needed: the Rust DaemonClient emits `daemon-connection-state` only on a state *transition* — connect()/ensure() short-circuit without an emit when already Connected
//   (mod.rs connect/ensure entry shortcut; connection.rs emits only on transitions). A reloaded webview recreates TauriTransport with _state='down' while the daemon is already
//   Connected, so no event comes and the window never registers the output Channel — all replay/live output of that window is cut off.
//
// The push model's blind spot is filled by one pull: after listener registration, ask the current state, and if connected while _state is not, pass it through the same path
//   as the event handler (applyConnectionStatus).
//
// ★Race guard (FIX 5)★: a real event can cut in between query and response. (1) event first: the stateVersion compare discards the stale pull snapshot (the event wins).
//   (2) pull first: a later event is applied normally through the same path. applyConnectionStatus's wasConnected guard prevents double Channel registration.
//   A non-connected answer does nothing (later transition events handle it).
  private async selfHeal(): Promise<void> {
    const versionBefore = this.stateVersion
    const genBefore = this.generation
    let raw: unknown
    try {
      raw = await invoke<unknown>('daemon_connection_state')
    } catch (e) {
      // A query failure (init race etc.) is not fatal — later real transition events fill in the state.
      console.warn('[TauriTransport] connection state self-heal query failed:', e)
      return
    }
    if (this.stateVersion !== versionBefore) {
      return
    }
    // If close() cut in while the query waited, discard — close does not raise the version, so a closed transport would otherwise come back connected with a zombie Channel.
    if (this.generation !== genBefore) return
    this.applyConnectionStatus(raw)
  }

  // ── Tauri event subscription registration ──────────────────────────────────────────────────
  // Receives the events the Rust DaemonClient pushes with `app.emit(event, payload)`, normalizes them into control InboundMessage and raises them to ProtocolClient.
  //
  // ★Idempotent (MED-1)★: no-op if already registered (unlisten not empty); re-registers only after close()→cleanupListeners emptied it (prevents double listeners).
  //
  // ★Partial registration cleanup (Fix-C ①)★: if a listen() fails midway, the listeners registered before it are released immediately and it throws (otherwise they leaked with no release path).
  //
  // ★In-flight close guard (Fix-C ①)★: listen() is async; a close() cutting in while awaiting would see the still-empty `this.unlisten`, then registration completes and a zombie listener survives.
  //   So at completion the entry generation (myGen) is re-checked; if stale, what was just registered is released and not stored.
  private async registerListeners(): Promise<void> {
    if (this.unlisten.length > 0) return
    const myGen = this.generation
    const registered: Array<() => void> = []
    try {
      registered.push(
        await listen<unknown>('agent-list-updated', (e) => {
          this.messageCb?.({
            kind: 'control',
            event: { AgentListUpdated: { agents: e.payload } },
          })
        }),
      )
      registered.push(
        await listen<{ agentId: string; status: unknown; epoch: number }>('status-changed', (e) => {
          const { agentId, status, epoch } = e.payload
          this.messageCb?.({
            kind: 'control',
            event: {
              StatusChanged: {
                agent_id: agentId,
                status,
                epoch,
              },
            },
          })
        }),
      )
      registered.push(
        await listen<{ result: unknown }>('restore-result', (e) => {
          this.messageCb?.({
            kind: 'control',
            event: {
              RestoreResult: { report: e.payload.result },
            },
          })
        }),
      )
      registered.push(
        await listen<unknown>('profile-list-updated', (e) => {
          this.messageCb?.({
            kind: 'control',
            event: { ProfileListUpdated: { profiles: e.payload } },
          })
        }),
      )
      // The shell emits to every webview; the target is picked by the payload's `labels` (windows that have a usage slot) — dropped if its own label is not in it.
      //   The label is given by Tauri (inferred from the hash route, every window other than `#/popup?window=` would read as `main`).
      //   `socketEpoch` is a shell-added field, camelCase so it does not mix with the daemon wire (`subscribed` is used by the shell only for the check, not carried).
      registered.push(
        await listen<ShellUsageLimitsUpdated>('usage-limits-updated', (e) => {
          const { labels, socket_epoch, snapshot } = e.payload
          if (!labels.includes(getCurrentWindow().label)) return
          this.messageCb?.({
            kind: 'control',
            event: { UsageLimitsUpdated: { snapshot, socketEpoch: socket_epoch } },
          })
        }),
      )
      // ★Connection state sync (single source of truth)★: the Rust connection task emits this on a transition (connected/reconnecting/down); the frontend changes state only through it.
      //
      // ★Output Channel registration = this single transition path (Fix-C ④)★: on a non-connected → connected transition it calls registerOutputChannel — the first connection and
      //   Rust-internal reconnects alike. Why here, not in doConnect: on connect Ok Rust emits `connected` *before* the invoke resolves (connection.rs: app.emit("connected") → ready_tx.send → invoke resolve),
      //   so this handler always runs first, closing the gap where output is lost before registration; one path also removes duplicate registration with doConnect, and covers
      //   Rust-driven reconnects that never go through doConnect. Idempotent (same window_label, old onmessage deleted).
      registered.push(
        await listen<unknown>('daemon-connection-state', (e) => {
          this.applyConnectionStatus(e.payload)
        }),
      )
      if (myGen !== this.generation) {
        for (const off of registered) off()
        return
      }
      this.unlisten = registered
    } catch (e) {
      for (const off of registered) off()
      throw e
    }
  }

  private cleanupListeners(): void {
    for (const off of this.unlisten) off()
    this.unlisten = []
  }

  // ── output Channel registration (③-b, HIGH-1) ───────────────────────────────────────────
  // Makes this window's per-window output Channel and registers it in the Rust registry with `subscribe_output`; the Rust connection task fans out all agent output of the window
  // as raw bytes through it, decoded with decodeOutputFrame and raised as an output InboundMessage (same shape as the WsTransport binary arm).
  //
  // ★Idempotent (Fix-C ①·④)★: may be called on every connected transition (first connection · Rust-internal reconnect · reload self-heal); re-registering overwrites the same window_label
  //   in Rust (agent.rs subscribe_output drops the old WindowEntry). The previous outputChannel onmessage is cleaned up first with delete (#13133 — not a null assignment),
  //   so two concurrent doConnects cannot leave zombie callbacks.
  //
  // ★Single-flight (FIX 6)★: a gate serializing registration — in progress it reuses that promise and only records a rerun request, so two subscribe_output invokes are never in flight to Rust at once
  //   (which reaches last cannot be controlled). The work itself is doRegisterOutputChannel.
  private registerOutputChannel(): Promise<void> {
    if (this.outputChannelInflight) {
      // The registration in progress may already reflect the latest state; re-register once anyway so the reconnect boundary converges on a Channel that is surely alive.
      this.outputChannelRerun = true
      return this.outputChannelInflight
    }
    const run = async (): Promise<void> => {
      try {
        do {
          this.outputChannelRerun = false
          await this.doRegisterOutputChannel()
        } while (this.outputChannelRerun)
      } finally {
        this.outputChannelInflight = null
      }
    }
    const p = run()
    this.outputChannelInflight = p
    return p
  }

  private async doRegisterOutputChannel(): Promise<void> {
    const myGen = this.generation
    this.outputChannelRegistered = false
    // Clean up the old Channel (#13133: delete, not a null assignment) — removes zombie onmessage.
    // ★On a real `@tauri-apps/api` Channel this delete does nothing★ (onmessage is a prototype accessor — `node_modules/@tauri-apps/api/core.js`); the old Channel keeps flowing until Rust swaps in the new one.
    //   ADR-0231: not losing the frames in that gap relies on this — ★do not make the detach 「real」★ (frames before the swap would be lost).
    if (this.outputChannel) {
      delete (this.outputChannel as { onmessage?: unknown }).onmessage
      this.outputChannel = null
    }
    const channel = new Channel<ArrayBuffer>()
    channel.onmessage = (raw: ArrayBuffer) => {
      // raw = Rust Response::new(frame bytes). Two kinds share the Channel (ADR-0046 — markers take the same path, so order after the replay tail is preserved):
      //   - daemon output frame: [tag0/1][agentId:16][epoch:4][seq:8][payload]
      //   - replay boundary marker: [tag255][agentId:16][epoch:4][gen:8][flags:1][replay_from:8]
      // Check for the marker first; a marker (tag=255) is normalized into a replayBoundary (not exposed on the public surface).
      const marker = decodeReplayMarker(raw)
      if (marker) {
        this.messageCb?.({
          kind: 'replayBoundary',
          agentId: marker.agentId,
          epoch: marker.epoch,
          gen: marker.gen,
          truncated: marker.truncated,
          failed: marker.failed,
          continuesConversation: marker.continuesConversation,
          replayFrom: marker.replayFrom,
        })
        return
      }
      const f = decodeOutputFrame(raw)
      if (!f) return
      this.messageCb?.({
        kind: 'output',
        tag: f.tag, // frame kind (0 terminal / 1 structured) — same shape as the WsTransport binary arm.
        agentId: f.agentId,
        epoch: f.epoch,
        seq: f.seq,
        bytes: f.payload,
      })
    }
    this.outputChannel = channel
    // window_label is injected automatically by Rust from the calling webview (agent.rs subscribe_output: tauri::Window).
    await invoke('subscribe_output', { channel })
    if (myGen === this.generation) this.outputChannelRegistered = true
  }

  /**
   * Waits until this window's output Channel registration finishes — the one in progress if any, otherwise registers anew if never done. reject = closed · registration failed.
   *
   * ★Replay requests go through this (load-bearing)★: the shell silently skips frames for an unregistered window (`output_channel.rs` `send_to_windows`), and `subscribe_output` (sync command)
   *   vs `request_replay` (async command) have no ordering guarantee — a replay flowing before registration loses its head in this window, the view holds everything after the first gap,
   *   and an idle agent's screen stays empty forever.
   */
  // ADR-0231
  private async awaitOutputChannel(): Promise<void> {
    if (this.outputChannelInflight) await this.outputChannelInflight
    if (this.outputChannelRegistered) return
    if (this.closedByUser) throw new Error('transport closed — output Channel not registered')
    await this.registerOutputChannel()
    if (!this.outputChannelRegistered) throw new Error('output Channel registration superseded')
  }

  // ── ensure transport readiness = attach-only (ADR-0021) ─────────────────────────────────
// Counterpart of WsTransport.ensureReady. The Rust DaemonClient already unifies connection, so no own reconnect loop here.
  ensureReady(): Promise<void> {
    if (this._state === 'connected') return Promise.resolve()
    if (this.connectPromise) return this.connectPromise
    if (this.closedByUser) {
      return Promise.reject(
        new Error('daemon down — explicit start with daemon_start required (ADR-0021: commands do not respawn)'),
      )
    }
    this.connectPromise = this.doConnect(false)
    return this.connectPromise
  }

  // ── explicit spawn entry point (ADR-0021 §1) ──────────────────────────────────────────
// Calls Rust DaemonClient.connect() (may spawn) through invoke.
//
// ★Re-entry guard (Fix-C ①)★: if a doConnect is in progress (connectPromise), reuse its promise — overwriting it would send a duplicate `daemon_connect` invoke and make Rust raise the generation twice.
//   The meaning of start (explicit spawn intent = closedByUser reset) is kept either way: after the user closed it, start again must allow reconnecting.
  start(): Promise<void> {
    if (this._state === 'connected') return Promise.resolve()
    this.closedByUser = false
    // Even if the one in progress was ensure (no-spawn), start's spawn intent is already reflected by the closedByUser reset — if that ensure fails and retries, the next call takes the spawn path.
    if (this.connectPromise) return this.connectPromise
    this.connectPromise = this.doConnect(true)
    return this.connectPromise
  }

  // Connection attempt only (MED-1 + Fix-C ①·④): doConnect does (1) idempotent registerListeners (so reconnecting after close()→cleanupListeners does not lose the control event listeners)
  //   (2) the Rust connect/ensure invoke. State transition and output Channel registration both belong to u5 (the transition emit; selfHeal's query when no emit came) — see registerListeners.
  //   Setting `connected` after the invoke resolves could diverge from Rust's real state (stale discard · reconnect).
  // ★Close generation guard (Fix-C ①)★: if close() cuts in during the invoke await (myGen != generation), the guard keeps a stale doConnect from wrongly clearing connectPromise (finally).
  // Failures propagate as is (the ensureReady/start caller catches).
  private async doConnect(allowSpawn: boolean): Promise<void> {
    const myGen = this.generation
    try {
      // ★Register listeners idempotently *before* the invoke (order load-bearing)★: on connect success Rust emits `daemon-connection-state="connected"` and *then* resolves the invoke;
      // attaching after the resolve can miss that event and leave _state stuck at 'down'.
      await this.registerListeners()
      await invoke(allowSpawn ? 'daemon_connect' : 'daemon_ensure')
      // ★Generation guard★: if close() cut in during the invoke await, this connection is stale. State and Channel revival are already blocked (u5 is the single source; close() removed its listeners),
      //   so there is no extra side effect — it just drops out quietly.
      if (myGen !== this.generation || this.closedByUser) {
        return
      }
      // ADR-0231: when the shell is already Connected, connect/ensure short-circuit with Ok without a transition emit — reloaded and popout windows can get here before init's query finishes.
      //   Get the state from the query and wait for the output Channel registration that transition started (registration before the replay request about to go out — `awaitOutputChannel`).
      //   The path where an emit came does not wait (that path's replay request waits on `awaitOutputChannel`).
      if (this._state !== 'connected') {
        await this.selfHeal()
        await this.outputChannelInflight?.catch((err: unknown) => {
          console.warn('[TauriTransport] output Channel registration failed:', err)
        })
      }
    } finally {
      // Clear connectPromise only for the current generation — close() may already have made a different one (=null); a stale doConnect must not erase it.
      if (myGen === this.generation) this.connectPromise = null
    }
  }

  // ── command send + reply downstream (③-a, HIGH-2) ──────────────────────────────────────
// ProtocolClient passes an AgentCommand wire object; it is delivered to the Rust DaemonClient by invoke. Rust forward_daemon_command returns the daemon reply (AgentEvent) of a command with a request_id
// as is, and that return is raised as a control InboundMessage so ProtocolClient.handleEvent breaks the pending. Commands without a request_id (Resize/Subscribe/Unsubscribe) return null — fire-and-forget.
//
// ★Returns a Promise★: ProtocolClient.sendCommand looks only at .catch (a successful return is ignored), so the reply is raised directly to onMessage and an invoke failure
//   (disconnect/Rust Err/reply timeout) rejects, rejecting that pending (blocks a permanent hang).
  send(payload: unknown): void | Promise<void> {
    return invoke<unknown>('forward_daemon_command', { cmd: payload }).then((reply) => {
      if (reply != null && typeof reply === 'object') {
        this.messageCb?.({ kind: 'control', event: reply as Record<string, unknown> })
      }
    })
  }

  // ── view-driven replay request (ADR-0046 F2) ──────────────────────────────────────────
// A view (slot) calls it through ProtocolClient on (re)mount · reconnect · watchdog · the failure ladder. Asks the src-tauri single-flight command `request_replay` for a full re-replay of that
// agent's daemon ring (wire Subscribe {after_seq:None}) and collects the assigned gen (u64). window_label is injected by Rust from the calling webview (unforgeable).
// The closing marker (success/failure) arrives separately as a tag=255 frame through the output Channel.
//
// gen = BigInt (§F2): invoke serializes u64 as a JSON number; BigInt() (absorbing number or string) matches the marker frame's getBigUint64 width so the gen fence comparison is exact.
  async requestReplay(agentId: string): Promise<bigint> {
    // ADR-0231: it does not go out before this window's output Channel registration (`awaitOutputChannel`).
    await this.awaitOutputChannel()
    const gen = await invoke<number | string>('request_replay', { agentId })
    // ★Safe integer guard (FIX-5)★: a u64 gen over 2^53 serialized as a JSON number already lost precision (BigInt() cannot restore it). Unreachable in practice (monotonic +1 per replay request); a warning beats a silent gen-fence misread.
    if (typeof gen === 'number' && !Number.isSafeInteger(gen)) {
      console.warn(`[TauriTransport] request_replay gen exceeds the safe integer range (${gen}) — precision may be lost (unreachable in practice)`)
    }
    return BigInt(gen)
  }

  async getUsageSnapshot(): Promise<UsageSnapshotPull> {
    const r = await invoke<ShellUsageSnapshotReply>('get_usage_snapshot')
    return { socketEpoch: r.socket_epoch, snapshots: r.snapshots }
  }

  // Generation guard (Fix-C ①): generation++ makes an in-flight doConnect stale. connectPromise=null lets the next ensureReady/start connect anew (closedByUser blocks it, but start resets it).
  close(): void {
    this.closedByUser = true
    this.generation += 1
    this.connectPromise = null
    this.cleanupListeners()
    // Clean up the output Channel (#13133: delete onmessage). On reconnect u5's connected handler registers anew.
    if (this.outputChannel) {
      delete (this.outputChannel as { onmessage?: unknown }).onmessage
      this.outputChannel = null
    }
    // Cancel the single-flight re-registration (FIX 6): clears the "re-register after completion" flag so no zombie Channel is attached after close; the in-flight promise completes and nulls itself.
    this.outputChannelRerun = false
    this.outputChannelRegistered = false
    invoke('daemon_close').catch((e: unknown) => {
      console.warn('[TauriTransport] daemon_close failed:', e)
    })
    // An explicit close shows no band (TRD S21 trd-3-2-u5 R7), and its listener is gone, so the shell's status never arrives here.
    this.setStatus('down', null)
  }

  // ProtocolClient takes the transport in its constructor and listener registration is async, so init is called separately (or awaited in clientFactory).
  // doConnect also calls the idempotent registerListeners, so init→connect or connect alone registers exactly one set.
  async init(): Promise<void> {
    await this.registerListeners()
    // Reload self-recovery (Fix-D): pull the current connection state once after listener registration — see selfHeal.
    await this.selfHeal()
  }
}
