// WsTransport — the WS (daemon) carrier implementation of Transport (ADR-0020 decision 3, TRD Stage 3).
//
// Extracts the WS-specific parts from DaemonClient (580 lines): discover_daemon → WebSocket creation → Auth →
// wait for Hello → exponential-backoff reconnect → ws.send / ws.onmessage. Binary frames and JSON AgentEvents are
// **normalized into InboundMessage** and passed up to ProtocolClient (ProtocolClient does not know
// carrier decoding). Auth/Hello are consumed here (handshake) — only control/output go up.
//
// Protocol semantics (request_id matching · dedup · epoch guard · resubscribe) are owned by ProtocolClient.
// This transport is responsible only for the carrier: connect · reconnect · frame encoding/decoding.

import { invoke } from '@tauri-apps/api/core'

import type { ConnectionState, UsageSnapshotPull } from './agentClient'
import type { InboundMessage, Transport } from './transport'
import {
  decodeOutputFrame,
  FRAME_TAG_STRUCTURED_EVENT,
  peekFrameHeader,
  placeholderErrorPayload,
} from './wsFrame'

// ── discover_daemon DTO (mirror of discovery.rs DaemonInfoDto) ──────────────────────────
interface DaemonInfoDto {
  pid: number
  host: string
  port: number
  token: string
  protocol_version: number
}

type WireEvent = Record<string, unknown>

// The direct-path single-flight state of one agent (the field meanings are canonical in the comment above the `wsReplay` declaration below).
interface WsReplayEntry {
  inflight?: {
    gen: bigint
    truncated: boolean
    epoch: number | undefined
    continuesConversation: boolean
    replayFrom: number
  }
  pending?: {
    gen: bigint
    waiters: Array<{ resolve: (gen: bigint) => void; reject: (e: Error) => void }>
  }
}

export class WsTransport implements Transport {
  private ws: WebSocket | null = null

  private _state: ConnectionState = 'down'
  private stateListeners = new Set<(s: ConnectionState) => void>()

  // The connection attempt in progress (prevents duplicate connections). Resolves on receiving Hello (= auth success).
  private connectPromise: Promise<void> | null = null

  // The reject handle of the openSocket promise in progress (cross-socket). settled only prevents a double-settle within one socket —
  // once the socket is detached (cleanupSocket), the resolve/reject of that promise is trapped inside a closed closure and
  // is never called. This field lets us wake it from outside so the await does not hang. Always cleared to null on settle.
  private pendingReject: ((e: Error) => void) | null = null

  private closedByUser = false
  private reconnectAttempt = 0
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null

  // ADR-0021: a cache for separating ensure (explicit spawn) from reconnect (attach-only).
  // Holds the host/port/token obtained from discover_daemon at the first explicit connection. The reconnect loop
  // uses this cache as the starting point, but before every retry it re-queries the current daemon.json with read_daemon_info (no-spawn)
  // to **follow the new address of a daemon that moved (hot-swap · crash respawn)** (see the openSocket comment below).
  // ★Invariant kept★: reconnect never calls discover/spawn — read_daemon_info is
  // read-only, so it does not wake the daemon. If there is no live daemon: backoff/exhausted → 'down' (recovered by the user's explicit start).
  private cachedInfo: DaemonInfoDto | null = null

  // ★Generation token (Blocker-1 zombie socket guard)★: incremented on every openSocket entry to identify "the currently valid attempt".
  // The openSocket body yields at awaits (discover_daemon · read_daemon_info), so if another
  // start()/close() (= cleanupSocket) slips in meanwhile, this attempt is superseded. pendingReject only
  // settles the outer promise and cannot stop the run() body that resumed past the await from creating a new WebSocket and
  // planting it in this.ws (a zombie) — so right after the await and just before socket creation, a stale resume is discarded with myGen!==openGen.
  // cleanupSocket() also increments it, so the close() path that only cleans up without a new openSocket invalidates the in-flight attempt.
  private openGen = 0

  // The single receive callback ProtocolClient registers (normalized control/output messages).
  private messageCb: ((msg: InboundMessage) => void) | null = null

  // ★legacy requestReplay single-flight (ADR-0046 F2, FIX-4)★: WsTransport connects straight to the daemon, so it lacks the real
  //   single-flight of src-tauri (gen numbering · acked gate). It is a test/trace path (the production carrier =
  //   TauriTransport only, ADR-0029), but it enforces ★per-agent sole-outstanding★ — at no point is more than 1 wire
  //   Subscribe in-flight per agent. A request arriving meanwhile is merged into the next single gen (all waiters share the same
  //   promise/gen), and after the current request closes at its boundary, it sends the Subscribe exactly once.
  //   The boundary (replayBoundary) is stamped with the gen of that unfinished request — this cuts off at the root last-value stamping (concurrent
  //   requests send two Subscribes and stamp the last gen on the boundary, causing an early flush) (FIX-4).
  //   ★There are two close triggers★ — `ReplayComplete` (success) · `SubscribeFailed` (rejection = failure boundary). Both
  //   converge on `settleReplay`.
  //
  //   State (agentId →):
  //    - inflight: the current unfinished (sent, awaiting boundary) request {gen, truncated(Ack), epoch(Ack),
  //      continuesConversation(Ack)}. undefined if none.
  //    - pending: the next request merged during inflight {gen, waiters[]}. Promoted at boundary close to send the Subscribe.
  //      A waiter is a resolve/reject pair — if the socket closes before ReplayComplete (handleClose) it is woken with a reject (FIX-B).
  private wsReplay = new Map<string, WsReplayEntry>()
  private wsGenCounter = 0n

  private wsReplayEntry(agentId: string): WsReplayEntry {
    let e = this.wsReplay.get(agentId)
    if (!e) {
      e = {}
      this.wsReplay.set(agentId, e)
    }
    return e
  }

  /** If not connected, send throws — the caller rejects per the contract. */
  private sendSubscribeFromOldest(agentId: string): void {
    this.send({ Subscribe: { agent_id: agentId, epoch: null, after_seq: null } })
  }

  // ── Connection state ──────────────────────────────────────────────────────────────────
  get connectionState(): ConnectionState {
    return this._state
  }

  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void {
    this.stateListeners.add(cb)
    // Notify the current state once immediately on registration (so ProtocolClient learns the initial state).
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

  private setState(s: ConnectionState): void {
    if (this._state === s) return
    this._state = s
    for (const cb of this.stateListeners) cb(s)
  }

  // ── Transport readiness guarantee = attach-only (ADR-0021 B-1) ────────────────────────────────────
  // The command/subscribe path (ProtocolClient.sendCommand/subscribeOutput/resizePty) calls it before every call.
  // ★Invariant★: this path never spawns — only openSocket(false) (reopening from the cache). That way the bug (B-1) where one key press/window
  // resize after the user turns the daemon off revives it does not occur. A spawn happens only in an explicit
  // start(). Resetting closedByUser/attempt is also not done here (that is the explicit start's responsibility).
  ensureReady(): Promise<void> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN && this._state === 'connected') {
      return Promise.resolve()
    }
    if (this.connectPromise) return this.connectPromise
    // If the user explicitly closed it, even when a command comes in it does not even try to attach and rejects immediately — a command
    // must not wake the daemon (it stays off). Recovery only through an explicit start.
    if (this.closedByUser) {
      return Promise.reject(
        new Error('daemon down — daemon_start 로 명시 시작 필요 (ADR-0021: 명령은 respawn 안 함)'),
      )
    }
    if (!this.cachedInfo) {
      return Promise.reject(
        new Error('daemon down — daemon_start 로 명시 시작 필요 (no cached daemon, ADR-0021)'),
      )
    }
    this.connectPromise = this.openSocket(false)
    return this.connectPromise
  }

  // ── Explicit spawn entry point (ADR-0021 §1) ───────────────────────────────────────────────
  // Only boot connect / the user's daemon_start (DaemonControl.start → client.connect → here) calls it.
  // Spawns the daemon (if absent) with discover_daemon and fills the cache. Resets the reconnect state stopped by an earlier close()/exhaustion
  // (clear closedByUser + attempt 0 + clean up the timer in progress) so it can come alive again.
  start(): Promise<void> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN && this._state === 'connected') {
      return Promise.resolve()
    }
    // ★race guard★: if the onclose of an abandoned in-flight socket (this.ws) fires later, a race occurs where handleClose
    // nulls the new socket's this.ws and schedules a reconnect. Before the new openSocket(true), use
    // cleanupSocket to delete (#13133) and close the old socket's handlers, detaching that onclose.
    this.cleanupSocket()
    this.closedByUser = false
    this.reconnectAttempt = 0
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
    this.connectPromise = this.openSocket(true)
    return this.connectPromise
  }

  /**
   * Open the socket once + send Auth + wait for Hello. Resolves on success, rejects on failure (handled by the caller).
   *
   * ADR-0021 ensure/reconnect separation:
   *  - allowDiscover=true (explicit connect): call discover_daemon (spawn if absent) → keep the result in cachedInfo.
   *  - allowDiscover=false (reconnect loop): **discover/spawn strictly forbidden.** However, re-query the current daemon.json with read_daemon_info (no-spawn)
   *    to **follow a daemon that moved** (hot-swap · crash respawn — new port/token).
   *    This is tracking the attach target, not a spawn (read-only, does not wake the daemon). If there is no live daemon,
   *    make one last try with the cached host/port, and if there is none either, reject (exhausted → down). Reconnect does not create a new daemon.
   */
  private openSocket(allowDiscover: boolean): Promise<void> {
    // ★Blocker-1 zombie guard★: capture this attempt's generation token synchronously before every await. If another start()/close() raises
    // openGen while the awaits below yield (supersede), the resumed run() body uses this value to tell that it is stale
    // and discards itself before creating a new WebSocket (blocks the zombie socket).
    const myGen = ++this.openGen
    return new Promise<void>((resolve, reject) => {
      // Keep this promise's reject reachable from outside (cleanupSocket) too. Cleared to null on every settle path.
      this.pendingReject = reject
      const settleResolve = (): void => {
        this.pendingReject = null
        resolve()
      }
      const settleReject = (e: Error): void => {
        this.pendingReject = null
        reject(e)
      }
      // Was this attempt superseded by another start()/close() (cleanupSocket)? If true, the caller must already
      // settleReject and then return (before creating a WebSocket · assigning this.ws = no zombie appears).
      const isStale = (): boolean => myGen !== this.openGen
      const run = async () => {
        let info: DaemonInfoDto
        if (allowDiscover) {
          info = await invoke<DaemonInfoDto>('discover_daemon')
          // ★Stale check right after the await resumes★: if cleanupSocket (openGen++) slipped in while discover yielded,
          // this attempt is invalid — only settleReject and bail out (no socket creation, this.ws not polluted).
          if (isStale()) {
            settleReject(new Error('openSocket superseded — stale attempt discarded'))
            return
          }
          this.cachedInfo = info
        } else {
          // Reconnect = attach-only. ★Tracking a moved daemon (ADR-0021)★: instead of spawning, re-query the
          // current daemon.json with read_daemon_info (no-spawn). Even if the daemon comes up on a new
          // port/token through a hot-swap (stop→start) or crash respawn, follow that address and attach (read-only — does not create or wake the daemon).
          const fresh = await invoke<DaemonInfoDto | null>('read_daemon_info').catch(() => null)
          // ★Stale check right after the await resumes (Blocker-1)★: if the cleanupSocket (openGen++) of close()/start() slipped in while
          // read_daemon_info yielded, it is invalid — discard. pendingReject alone settles only the outer promise and
          // the body proceeds to new WebSocket, making a zombie or hijacking this.ws, so cut it off here.
          if (isStale()) {
            settleReject(new Error('openSocket superseded — stale attempt discarded'))
            return
          }
          if (fresh) {
            this.cachedInfo = fresh
            info = fresh
          } else if (this.cachedInfo) {
            // read_daemon_info is None (no live daemon) but there is a cache — it may be a transient disconnect, so
            // make one last try with the old address (still attach-only, not a spawn). If it fails: onclose→backoff→exhausted.
            info = this.cachedInfo
          } else {
            throw new Error('no live/cached daemon — reconnect cannot discover/spawn (ADR-0021)')
          }
        }
        // ★Final stale check just before socket creation (Blocker-1)★: synchronous time may have passed in the branch after the await, and
        // this is the key point for blocking zombies — new WebSocket only if it is "the current attempt". Passing here
        // is what justifies assigning this.ws.
        if (isStale()) {
          settleReject(new Error('openSocket superseded — stale attempt discarded'))
          return
        }
        // Assume the host is fixed at 127.0.0.1 for now (local IPC).
        const ws = new WebSocket('ws://' + info.host + ':' + info.port)
        ws.binaryType = 'arraybuffer'
        this.ws = ws

        let settled = false

        ws.onopen = () => {
          // The first frame = Auth (JSON text). protocol_version echoes the value discover gave.
          ws.send(
            JSON.stringify({
              Auth: { token: info.token, protocol_version: info.protocol_version },
            }),
          )
        }

        ws.onmessage = (event: MessageEvent) => {
          if (typeof event.data === 'string') {
            const msg = JSON.parse(event.data) as WireEvent
            // Receiving Hello = auth success. Connect complete. (Consumed inside the handshake — not passed up.)
            if ('Hello' in msg && !settled) {
              settled = true
              this.reconnectAttempt = 0
              // The connected transition itself does not call replay — the trigger for attaching a view is the authoritative registry (ADR-0046 amend,
              //   protocolClient.observeRoster). Here we only set the state.
              this.setState('connected')
              settleResolve()
              return
            }
            // An Error before auth = Auth failure → reject (the daemon closes the connection).
            if ('Error' in msg && !settled) {
              settled = true
              const m = (msg.Error as { message?: string })?.message ?? 'auth failed'
              settleReject(new Error('daemon auth failed: ' + m))
              return
            }
            // ★legacy replay boundary synthesis (ADR-0046 F2)★: it is a direct daemon connection, so there is no marker frame (tag=255). We observe the wire events
            //   the daemon sends, SubscribeAck (remember truncated/epoch) → ReplayComplete (boundary), to approximate
            //   the requestReplay contract (at least 1 boundary per request, sole-outstanding). The control message is
            //   also passed up as is below.
            this.observeReplayWire(msg)
            this.messageCb?.({ kind: 'control', event: msg })
          } else if (event.data instanceof ArrayBuffer) {
            const f = decodeOutputFrame(event.data)
            if (f) {
              this.messageCb?.({
                kind: 'output',
                tag: f.tag, // Frame kind (0 terminal / 1 structured) — ProtocolClient branches the consumption path on it.
                agentId: f.agentId,
                epoch: f.epoch,
                seq: f.seq,
                bytes: f.payload,
              })
              return
            }
            // ADR-0231: dropping it leaves a hole at that seq and the view stalls holding the tail — like the shell relay (`frame_relay.rs`),
            //   raise an unknown tag as a placeholder with the same seq, and cut off a short frame whose seq cannot be read (the re-attached view
            //   asks for a full replay). The browser does not deliver messages after close (readyState is not OPEN).
            const header = peekFrameHeader(event.data)
            if (header === null) {
              console.warn(`[wsTransport] frame too short for a header (${event.data.byteLength} B) — closing for a full replay`)
              ws.close()
              return
            }
            this.messageCb?.({
              kind: 'output',
              tag: FRAME_TAG_STRUCTURED_EVENT,
              agentId: header.agentId,
              epoch: header.epoch,
              seq: header.seq,
              bytes: placeholderErrorPayload(),
            })
          }
        }

        ws.onerror = () => {
          if (!settled) {
            settled = true
            settleReject(new Error('daemon websocket error'))
          }
        }

        ws.onclose = () => {
          this.handleClose(settled)
          if (!settled) {
            settled = true
            settleReject(new Error('daemon websocket closed before handshake'))
          }
        }
      }
      run().catch((e) => settleReject(e))
    })
  }

  /** The pending reject is handled by ProtocolClient through the connectionState transition (connected→reconnecting)
   * (promoted to a carrier-independent place). */
  private handleClose(wasHandshakeSettled: boolean): void {
    this.connectPromise = null
    this.ws = null
    // ★FIX-B: tie the replay single-flight state to the socket's lifetime★. If the socket closes before ReplayComplete (boundary), the
    //   unfinished inflight never receives a closing marker, and merge waiters (pending) have no socket to be promoted on — if wsReplay is
    //   not emptied, after reconnect requestReplay sees entry.inflight and does not send a new Subscribe, so all
    //   waiters are stuck for good. Here we empty the map and reject the merge waiters (the catch of ProtocolClient.issueReplay
    //   absorbs it → the reconnect transition drives the full re-request again). onclose fires after onerror per the spec, so
    //   this point covers both the close and error paths.
    this.rejectAllReplays(new Error('daemon websocket closed before replay complete'))

    if (this.closedByUser) {
      this.setState('down')
      return
    }
    // A disconnect during the handshake is also a reconnect target (waiting for the daemon to restart). Exponential backoff.
    void wasHandshakeSettled
    this.setState('reconnecting')
    this.scheduleReconnect()
  }

  // The maximum number of attach-only reconnect attempts. ADR-0021: if the daemon dies (graceful stop · kill · crash), every reconnect to the cached
  // host:port fails — rather than hang on an endless reconnecting, try this many times
  // and then settle at 'down' (stays off). A transient network drop recovers within this. Recovery is only by
  // the user's explicit daemon_start (= start(), discover allowed) — reconnect and commands (ensureReady) must not spawn.
  private static readonly MAX_RECONNECT_ATTEMPTS = 5

  private scheduleReconnect(): void {
    if (this.reconnectTimer) return
    if (this.closedByUser) return
    if (this.reconnectAttempt >= WsTransport.MAX_RECONNECT_ATTEMPTS) {
      this.setState('down')
      return
    }
    const delay = Math.min(500 * 2 ** this.reconnectAttempt, 10000)
    this.reconnectAttempt += 1
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null
      if (this.closedByUser) return
      // ★attach-only★: discover/spawn forbidden — only reopen the socket with the cached host:port (openSocket(false)).
      // On success connectPromise resolves with Hello and reconnectAttempt is reset to 0.
      // On failure (daemon death → connection refused) onclose comes and scheduleReconnect runs again → 'down' when exhausted.
      this.connectPromise = this.openSocket(false)
      this.connectPromise.catch(() => {
        // openSocket reject (no cache / socket onclose did not fire) — reschedule directly ('down' when exhausted).
        if (!this.reconnectTimer && !this.closedByUser) {
          this.scheduleReconnect()
        }
      })
    }, delay)
  }

  // ── Sending commands ───────────────────────────────────────────────────────────────────
  send(payload: unknown): void {
    const ws = this.ws
    if (!ws || ws.readyState !== WebSocket.OPEN) {
      throw new Error('daemon not connected')
    }
    ws.send(JSON.stringify(payload))
  }

  // ── View-driven replay request (ADR-0046 F2 — legacy direct single-flight, FIX-4) ────────────────
  // Instead of the src-tauri single-flight (gen numbering · acked gate), send its own wire Subscribe{after_seq:null} and
  // assign the gen with a per-agent gen counter. ★sole-outstanding★: with no in-flight, number it and send immediately, and return that
  // gen. With an in-flight, do not overlap wire Subscribes but merge into the next single gen — all
  // waiters share the same gen/promise, and after the current ReplayComplete that merged request is sent exactly once.
  // The closing marker is stamped by observeReplayWire with the in-flight's gen (boundary gen = matches the request that closes that replay).
  requestReplay(agentId: string): Promise<bigint> {
    const entry = this.wsReplayEntry(agentId)
    if (entry.inflight) {
      if (!entry.pending) entry.pending = { gen: ++this.wsGenCounter, waiters: [] }
      const pending = entry.pending
      // Keep a resolve/reject pair — resolve on ReplayComplete promotion, reject if the socket closes before that (FIX-B).
      return new Promise<bigint>((resolve, reject) => pending.waiters.push({ resolve, reject }))
    }
    // No in-flight → number and send immediately. If not connected, send throws — reject per the contract (without a boundary).
    const gen = ++this.wsGenCounter
    try {
      this.sendSubscribeFromOldest(agentId)
    } catch (e) {
      if (!entry.pending) this.wsReplay.delete(agentId)
      return Promise.reject(e instanceof Error ? e : new Error(String(e)))
    }
    entry.inflight = { gen, truncated: false, epoch: undefined, continuesConversation: false, replayFrom: 0 }
    return Promise.resolve(gen)
  }

  // A carrier without a shell cache — it is a direct daemon connection, so nothing sends the usage subscription (`UsageSubscribe`) either. Production is fixed to Tauri (ADR-0036).
  getUsageSnapshot(): Promise<UsageSnapshotPull> {
    return Promise.resolve({ socketEpoch: 0, snapshots: [] })
  }

  // Observe the daemon wire events and synthesize the replay boundary (replayBoundary) (legacy direct single-flight). SubscribeAck
  // remembers the epoch/truncated of the unfinished request, and ReplayComplete closes that request with a success boundary, then if there is
  // a merged next request (pending) promotes it and sends the Subscribe exactly once (1 boundary ↔ 1 Subscribe).
  private observeReplayWire(msg: WireEvent): void {
    if ('SubscribeAck' in msg) {
      // ADR-0226: an old daemon does not carry continues_conversation — false if absent (today's behavior).
      const a = msg.SubscribeAck as {
        agent_id: string
        current_epoch: number
        replay_from?: number
        truncated: boolean
        continues_conversation?: boolean
      }
      const entry = this.wsReplay.get(a.agent_id)
      if (entry?.inflight) {
        entry.inflight.epoch = a.current_epoch
        entry.inflight.truncated = a.truncated
        entry.inflight.continuesConversation = a.continues_conversation === true
        // ADR-0231: the replay head of the direct path — on the Tauri path the shell carries it on the marker. 0 if the field is absent —
        //   left as is, the `Math.max` in flush would become NaN.
        entry.inflight.replayFrom = typeof a.replay_from === 'number' ? a.replay_from : 0
      }
      return
    }
    if ('SubscribeFailed' in msg) {
      // ★A rejection also closes an unfinished request (failure boundary)★: otherwise this entry.inflight stays forever and
      //   every requestReplay of that agent is merged into pending and the wire Subscribe never goes out again
      //   — output is cut off for good after a daemon restart (measured 2026-08-19). Give this carrier the same
      //   semantics as on_refused of the src-tauri single-flight.
      const f = msg.SubscribeFailed as { agent_id: string }
      const entry = this.wsReplay.get(f.agent_id)
      // ★A request that received the Ack (= accepted by the daemon) is not released★ — the same defense as src-tauri on_refused. If released,
      //   the ReplayComplete that follows meets an empty slot and is ignored, and the view waiting for that replay never
      //   gets a success marker. Here whether the Ack was observed is exactly `inflight.epoch !== undefined`.
      if (!entry?.inflight || entry.inflight.epoch !== undefined) return
      // The epoch is unknown because there is no Ack → 0 (approximation). The view puts no epoch fence on a failure marker (protocolClient
      //   evalMarker) — if it did, this marker would be dropped and the view would wait for the watchdog.
      this.settleReplay(f.agent_id, entry, {
        epoch: 0,
        gen: entry.inflight.gen,
        truncated: entry.inflight.truncated,
        failed: true,
        continuesConversation: false,
        replayFrom: entry.inflight.replayFrom,
      })
      return
    }
    if ('ReplayComplete' in msg) {
      const c = msg.ReplayComplete as { agent_id: string; epoch?: number }
      const entry = this.wsReplay.get(c.agent_id)
      if (!entry?.inflight) return
      // Close the unfinished request with a success boundary — boundary gen = the gen of the request that closes this replay (prevents last-value
      //   mis-stamping). The epoch is the value observed from the Ack, else the Complete's epoch, else 0 (direct approximation). The replay head
      //   is also the value observed from the Ack and 0 if absent (direct approximation) — if that value is smaller than the real head and the replay was trimmed, the view can never fill the
      //   hole before the head and stalls until the hold limit. The Ack always comes before the replay, so that branch does not exist in production.
      const done = entry.inflight
      this.settleReplay(c.agent_id, entry, {
        epoch: done.epoch ?? c.epoch ?? 0,
        gen: done.gen,
        truncated: done.truncated,
        failed: false,
        continuesConversation: done.continuesConversation,
        replayFrom: done.replayFrom,
      })
    }
  }

  // Close one unfinished request with a boundary marker, and if there is a merged next request, promote it and send the Subscribe exactly once
  // (1 boundary ↔ 1 Subscribe). A success close (ReplayComplete) and a failure close (SubscribeFailed)
  // share this cleanup — split apart, only one side advances the queue and the remaining waiters are stuck for good.
  private settleReplay(
    agentId: string,
    entry: WsReplayEntry,
    boundary: {
      epoch: number
      gen: bigint
      truncated: boolean
      failed: boolean
      continuesConversation: boolean
      replayFrom: number
    },
  ): void {
    entry.inflight = undefined
    this.messageCb?.({ kind: 'replayBoundary', agentId, ...boundary })
    const pending = entry.pending
    if (pending) {
      entry.pending = undefined
      try {
        this.sendSubscribeFromOldest(agentId)
        entry.inflight = {
          gen: pending.gen,
          truncated: false,
          epoch: undefined,
          continuesConversation: false,
          replayFrom: 0,
        }
      } catch {
        // Send failure (disconnect) — cannot set up an in-flight. Waiters get the gen (the contract returns a gen) but the marker comes from the re-request
        //   that the reconnect transition drives. The entry is reused on the next request.
      }
      for (const w of pending.waiters) w.resolve(pending.gen)
    } else if (!entry.inflight) {
      // If there is neither an unfinished nor a merged request, clean up the entry (prevents a map leak).
      this.wsReplay.delete(agentId)
    }
  }

  /**
   * ★FIX-B: fully reset the replay single-flight state on socket close★. Empty wsReplay so that requestReplay after reconnect
   * sends a new wire Subscribe again (resolving the stuck state), and wake the merge waiters (pending.waiters) with a reject —
   * since this replay died without a closing marker. The inflight request's promise has already resolved returning its gen, so
   * it is not a separate settle target here (even if the ProtocolClient view gets no marker, the reconnect transition drives the re-request).
   */
  private rejectAllReplays(err: Error): void {
    const entries = [...this.wsReplay.values()]
    this.wsReplay.clear()
    for (const e of entries) {
      if (e.pending) {
        for (const w of e.pending.waiters) w.reject(err)
      }
    }
  }

  // ── Explicit close (stop reconnecting) ──────────────────────────────────────────────────────
  close(): void {
    this.closedByUser = true
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
    this.cleanupSocket()
    this.setState('down')
  }

  /** #13133: clean up the handlers with delete, not a null assignment, then close. */
  private cleanupSocket(): void {
    // When a new start()/close() replaces an in-flight start(), the old socket's resolve/reject live only inside the
    // onclose/onmessage/onerror closures that are about to be deleted and never settle → the caller that was awaiting that promise
    // (DaemonControl.start → client.connect) hangs. Wake it with an explicit reject (on a normal handshake
    // completion pendingReject is already null, so a healthy close does not wake it).
    //
    // ★This also includes the window before ws creation (discover in-flight)★: pendingReject is set synchronously in the openSocket executor, but
    // this.ws is assigned only after await invoke('discover_daemon'). Move this settle and the connectPromise cleanup
    // above the `if (!ws) return` below so they always run even in the discover window where ws is still null —
    // otherwise the second start() overwrites pendingReject, the first promise never settles, and the caller hangs.
    // ★Blocker-1 zombie guard★: invalidate the generation of the in-flight openSocket. The path that only cleans up
    // without a new openSocket, like close() (or the cleanup step of start()), supersedes the attempt yielding at an await, so that on resume it
    // is discarded by myGen!==openGen (pendingReject wakes only the outer promise and cannot stop the body from creating a zombie).
    this.openGen++
    if (this.pendingReject) {
      const rej = this.pendingReject
      this.pendingReject = null
      rej(new Error('connection superseded (start/close) — ADR-0021'))
    }
    this.connectPromise = null
    // ★FIX-B: cleanupSocket also resets the replay state★. The supersede path of close()/start() deletes the old socket's onclose
    //   so handleClose does not run, so it must be emptied here too for a new Subscribe to go out after reconnect and the merge waiters
    //   to be rejected. Even if called twice with handleClose it is harmless (idempotent) as it is clear + empty waiters.
    this.rejectAllReplays(new Error('connection superseded (start/close) — ADR-0021'))
    const ws = this.ws
    if (!ws) return
    delete (ws as { onmessage?: unknown }).onmessage
    delete (ws as { onopen?: unknown }).onopen
    delete (ws as { onerror?: unknown }).onerror
    delete (ws as { onclose?: unknown }).onclose
    try {
      ws.close()
    } catch {
      // Already closed — ignored.
    }
    this.ws = null
  }
}
