// WsTransport — the WS (daemon) carrier implementation of Transport (ADR-0020, TRD Stage 3).
//
// WS-specific parts of the old DaemonClient: discover_daemon → WebSocket creation → Auth → wait for Hello → exponential-backoff reconnect → ws.send / ws.onmessage. Binary frames and JSON AgentEvents are **normalized into InboundMessage**
// for ProtocolClient (which does not know carrier decoding); Auth/Hello are consumed here — only control/output go up.
// Protocol semantics (request_id matching · dedup · epoch guard · resubscribe) belong to ProtocolClient; this transport owns connection · reconnection · frame encoding/decoding.
//
// ADR-0310 D23 ①: the WebView never receives the daemon token (the three discovery commands drop it), so the Auth token is the constructor's `authToken`, fixed for the transport's life; without it every connect attempt rejects with
// `WS_NO_AUTH_TOKEN_ERROR` before any invoke or socket. This direct path is not production (production = TauriTransport through the shell, ADR-0029); a daemon replaced with a new token is reached at its new port but refuses the old token.

import { invoke } from '@tauri-apps/api/core'

import type { Band } from '../../src-tauri/bindings/Band'
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
  protocol_version: number
}

/** Rejection text of every connect attempt (`start` · `ensureReady` · reconnect) on a transport built without `authToken`. */
export const WS_NO_AUTH_TOKEN_ERROR =
  'direct daemon connection has no auth token — the WebView does not receive the daemon token; connect through TauriTransport'

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
  private readonly authToken: string | undefined

  /**
   * `authToken` = the daemon token sent in Auth on every socket this transport opens; omitted = cannot connect. Dev/test only — production has no source of this token on purpose (ADR-0310 D23 ①):
   * do not add a shell command that hands it to the WebView to revive this path. (TS `private` is not private at runtime.)
   */
  constructor(opts?: { authToken?: string }) {
    this.authToken = opts?.authToken
  }

  private ws: WebSocket | null = null

  private _state: ConnectionState = 'down'
  private stateListeners = new Set<(s: ConnectionState) => void>()

  // The connection attempt in progress (prevents duplicate connections). Resolves on receiving Hello (= auth success).
  private connectPromise: Promise<void> | null = null

  // Reject handle of the openSocket promise in progress (cross-socket). `settled` only prevents a double-settle within one socket; once the socket is detached (cleanupSocket) its resolve/reject is trapped in a closed closure,
  //   so this field wakes the await from outside to avoid a hang. Always cleared to null on settle.
  private pendingReject: ((e: Error) => void) | null = null

  private closedByUser = false
  private reconnectAttempt = 0
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null

  // ADR-0021: cache keeping ensure (explicit spawn) apart from reconnect (attach-only): the host/port the first explicit connect got from discover_daemon. The reconnect loop starts from it but re-reads the current daemon.json
  //   through read_daemon_info (no-spawn) before every retry and **follows a daemon that moved (hot-swap · crash respawn)** (see openSocket).
  // ★Invariant★: reconnect never calls discover/spawn — read_daemon_info is read-only and never wakes a daemon. With no live daemon it backs off until exhausted → 'down' (recovery = the user's explicit start).
  private cachedInfo: DaemonInfoDto | null = null

  // ★Generation token (zombie socket guard)★: incremented on every openSocket entry to identify the currently valid attempt. openSocket yields at awaits (discover_daemon · read_daemon_info), so another start()/close() (= cleanupSocket)
  //   may supersede it; pendingReject only settles the outer promise and cannot stop the resumed body from creating a WebSocket and planting it in this.ws (a zombie) — so after the await and just before socket creation a stale resume is discarded with myGen!==openGen.
  //   cleanupSocket() also increments it, so a close() that only cleans up invalidates the in-flight attempt.
  private openGen = 0

  // The single receive callback ProtocolClient registers (normalized control/output messages).
  private messageCb: ((msg: InboundMessage) => void) | null = null

  // ★legacy requestReplay single-flight (ADR-0046 F2, FIX-4)★: WsTransport connects straight to the daemon, so it lacks the src-tauri single-flight (gen numbering · acked gate). It is a test/trace path (production = TauriTransport only, ADR-0029)
  //   but enforces ★per-agent sole-outstanding★ — never more than 1 wire Subscribe in flight per agent. A request arriving meanwhile merges into the next single gen (all waiters share the promise/gen) and sends the Subscribe once after the current request closes at its boundary.
  //   The boundary (replayBoundary) is stamped with the gen of that unfinished request — this cuts off last-value stamping (two concurrent Subscribes stamping the last gen cause an early flush) (FIX-4).
  //   ★There are two close triggers★ — `ReplayComplete` (success) · `SubscribeFailed` (failure boundary); both converge on `settleReplay`.
  //
  //   State (agentId →):
  //    - inflight: the current unfinished (sent, awaiting boundary) request {gen, truncated(Ack), epoch(Ack), continuesConversation(Ack)}; undefined if none.
  //    - pending: the next request merged during inflight {gen, waiters[]}, promoted at boundary close to send the Subscribe. A waiter is a resolve/reject pair — woken with a reject if the socket closes before ReplayComplete (handleClose, FIX-B).
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

  /** Always `null` — the shell decides the band, and this carrier talks to the daemon without one. */
  get connectionBand(): Band | null {
    return null
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

  // The command/subscribe path (ProtocolClient.sendCommand/subscribeOutput/resizePty) calls it before every call.
  // ★Invariant★: this path never spawns — only openSocket(false) (reopen from the cache), so a key press/window resize after the user turns the daemon off cannot revive it (B-1). Spawn happens only in an explicit start();
  //   resetting closedByUser/attempt is also start's job, not here.
  ensureReady(): Promise<void> {
    // Ahead of the closed / no-cache rejections: their "daemon_start" recovery cannot work without a token.
    if (this.authToken === undefined) return Promise.reject(new Error(WS_NO_AUTH_TOKEN_ERROR))
    if (this.ws && this.ws.readyState === WebSocket.OPEN && this._state === 'connected') {
      return Promise.resolve()
    }
    if (this.connectPromise) return this.connectPromise
    // If the user explicitly closed it, a command does not even try to attach and rejects immediately — a command must not wake the daemon. Recovery only through an explicit start.
    if (this.closedByUser) {
      return Promise.reject(
        new Error('daemon down — explicit start with daemon_start required (ADR-0021: commands do not respawn)'),
      )
    }
    if (!this.cachedInfo) {
      return Promise.reject(
        new Error('daemon down — explicit start with daemon_start required (no cached daemon, ADR-0021)'),
      )
    }
    this.connectPromise = this.openSocket(false)
    return this.connectPromise
  }

  // ── Explicit spawn entry point (ADR-0021 §1) ───────────────────────────────────────────────
// Only boot connect / the user's daemon_start (DaemonControl.start → client.connect → here) calls it. Spawns the daemon (if absent) with discover_daemon and fills the cache. Resets the reconnect state stopped by an earlier close()/exhaustion
// (clear closedByUser + attempt 0 + clean up the timer in progress) so it can come alive again.
  start(): Promise<void> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN && this._state === 'connected') {
      return Promise.resolve()
    }
    // ★race guard★: the onclose of an abandoned in-flight socket firing later would make handleClose null the new socket's this.ws and schedule a reconnect; before the new openSocket(true), cleanupSocket deletes (#13133) and closes the old socket's handlers.
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
   * Opens one socket, sends Auth, waits for Hello. Resolves on success, rejects on failure (the caller handles it).
   *
   * ADR-0021 ensure / reconnect split:
   *  - allowDiscover=true (explicit connect): calls discover_daemon (spawns if none) → keeps the result in cachedInfo.
   *  - allowDiscover=false (reconnect loop): **never discovers or spawns.** Re-reads the current daemon.json through read_daemon_info (no spawn, never wakes a daemon) and **follows a moved daemon** (hot swap · crash respawn — new port);
   *    with no live daemon it makes a last try on the cached host/port, and rejects if there is none (exhausted → down).
   */
  private openSocket(allowDiscover: boolean): Promise<void> {
    // ★Zombie guard★: capture this attempt's generation synchronously before every await; if another start()/close() raises openGen while the awaits yield, the resumed body sees it is stale and discards itself before creating a WebSocket.
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
      // Was this attempt superseded by another start()/close() (cleanupSocket)? If so, the caller must settleReject and return before creating a WebSocket / assigning this.ws (no zombie).
      const isStale = (): boolean => myGen !== this.openGen
      const run = async () => {
        const authToken = this.authToken
        if (authToken === undefined) throw new Error(WS_NO_AUTH_TOKEN_ERROR)
        let info: DaemonInfoDto
        if (allowDiscover) {
          info = await invoke<DaemonInfoDto>('discover_daemon')
          // ★Stale check right after the await resumes★: if cleanupSocket (openGen++) slipped in while discover yielded, only settleReject and bail out (no socket creation, this.ws not polluted).
          if (isStale()) {
            settleReject(new Error('openSocket superseded — stale attempt discarded'))
            return
          }
          this.cachedInfo = info
        } else {
          // Reconnect = attach-only. ★Follow a daemon that moved (ADR-0021)★: instead of spawning, re-read daemon.json through read_daemon_info (no-spawn); after a hot-swap or crash respawn on a new port, attach there (read-only — never creates or wakes a daemon).
          const fresh = await invoke<DaemonInfoDto | null>('read_daemon_info').catch(() => null)
          // ★Stale check after the await (zombie guard)★: if close()/start() cleanupSocket (openGen++) slipped in while read_daemon_info yielded, discard — pendingReject settles only the outer promise and the body would create a zombie WebSocket or hijack this.ws.
          if (isStale()) {
            settleReject(new Error('openSocket superseded — stale attempt discarded'))
            return
          }
          if (fresh) {
            this.cachedInfo = fresh
            info = fresh
          } else if (this.cachedInfo) {
            // read_daemon_info is None (no live daemon) but a cache exists — possibly a transient disconnect, so make one last try at the old address (still attach-only). If it fails: onclose→backoff→exhausted.
            info = this.cachedInfo
          } else {
            throw new Error('no live/cached daemon — reconnect cannot discover/spawn (ADR-0021)')
          }
        }
        // ★Final stale check just before socket creation★: synchronous time may have passed after the await; new WebSocket only if this is the current attempt — this is what justifies assigning this.ws.
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
              Auth: { token: authToken, protocol_version: info.protocol_version },
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
              // The connected transition does not call replay — the trigger for attaching a view is the authoritative registry (ADR-0046 amend, protocolClient.observeRoster); here only set the state.
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
            // ★legacy replay boundary synthesis (ADR-0046 F2)★: a direct daemon connection has no marker frame (tag=255), so observe the wire events SubscribeAck (remember truncated/epoch) → ReplayComplete (boundary) to approximate the requestReplay contract (≥1 boundary per request, sole-outstanding). The control message is also passed up as is.
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
            // ADR-0231: dropping it leaves a hole at that seq and the view stalls holding the tail — like the shell relay (`frame_relay.rs`), raise an unknown tag as a placeholder with the same seq, and cut off a short frame whose seq cannot be read (the re-attached view asks for a full replay). The browser delivers no messages after close (readyState not OPEN).
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

  /** The pending reject is handled by ProtocolClient through the connectionState transition (connected→reconnecting). */
  private handleClose(wasHandshakeSettled: boolean): void {
    this.connectPromise = null
    this.ws = null
    // ★FIX-B: tie the replay single-flight state to the socket's lifetime★: if the socket closes before ReplayComplete, the unfinished inflight never gets a closing marker and merge waiters have nothing to be promoted on — unless wsReplay is emptied,
    //   after reconnect requestReplay sees entry.inflight and sends no new Subscribe, leaving all waiters stuck. So empty the map and reject the merge waiters (ProtocolClient.issueReplay's catch absorbs it → the reconnect transition drives the full re-request).
    //   onclose fires after onerror per the spec, so this covers both paths.
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

  // Maximum attach-only reconnect attempts. ADR-0021: if the daemon dies (graceful stop · kill · crash) every reconnect to the cached host:port fails — rather than hang on endless reconnecting, try this many times then settle at 'down'.
  //   A transient network drop recovers within this. Recovery is only by the user's explicit daemon_start (= start(), discover allowed) — reconnect and commands (ensureReady) must not spawn.
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
      // ★attach-only★: discover/spawn forbidden — reopen the socket with the cached host:port only (openSocket(false)). On success connectPromise resolves with Hello and reconnectAttempt resets to 0;
      //   on failure onclose comes and scheduleReconnect runs again → 'down' when exhausted.
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
  // Instead of the src-tauri single-flight (gen numbering · acked gate), send its own wire Subscribe{after_seq:null} and assign the gen with a per-agent gen counter. ★sole-outstanding★: with no in-flight, number it, send immediately and return that gen;
  // with an in-flight, do not overlap wire Subscribes but merge into the next single gen (all waiters share gen/promise), sent exactly once after the current ReplayComplete.
  // The closing marker is stamped by observeReplayWire with the in-flight's gen (boundary gen = the request that closes that replay).
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

  // Observe the daemon wire events and synthesize the replay boundary (replayBoundary) (legacy direct single-flight). SubscribeAck remembers the unfinished request's epoch/truncated; ReplayComplete closes it with a success boundary and promotes a merged next request (pending),
  // sending its Subscribe exactly once (1 boundary ↔ 1 Subscribe).
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
        // ADR-0231: the replay head of the direct path (on the Tauri path the shell carries it on the marker). 0 if the field is absent — left undefined, the `Math.max` in flush would become NaN.
        entry.inflight.replayFrom = typeof a.replay_from === 'number' ? a.replay_from : 0
      }
      return
    }
    if ('SubscribeFailed' in msg) {
      // ★A rejection also closes an unfinished request (failure boundary)★: otherwise entry.inflight stays forever, every requestReplay of that agent merges into pending and the wire Subscribe never goes out again — output is cut off for good after a daemon restart.
      //   Same semantics as on_refused of the src-tauri single-flight.
      const f = msg.SubscribeFailed as { agent_id: string }
      const entry = this.wsReplay.get(f.agent_id)
      // ★A request that received the Ack (= accepted by the daemon) is not released★ (same as src-tauri on_refused): releasing it would make the following ReplayComplete meet an empty slot and be ignored, so the view never gets a success marker.
      //   "Ack observed" is exactly `inflight.epoch !== undefined`.
      if (!entry?.inflight || entry.inflight.epoch !== undefined) return
      // No Ack → epoch unknown → 0 (approximation). The view puts no epoch fence on a failure marker (protocolClient evalMarker) — otherwise this marker would be dropped and the view would wait for the watchdog.
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
      // Close the unfinished request with a success boundary — boundary gen = the gen of the request that closes this replay (prevents last-value mis-stamping). Epoch = from the Ack, else the Complete's, else 0; replay head = from the Ack, else 0 (direct approximation).
      //   A head smaller than the real one after a trimmed replay leaves a hole the view can never fill, stalling until the hold limit; the Ack always precedes the replay, so that branch does not exist in production.
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

  // Close one unfinished request with a boundary marker and promote a merged next request, sending its Subscribe exactly once (1 boundary ↔ 1 Subscribe). Success (ReplayComplete) and failure (SubscribeFailed) closes share this cleanup — split apart, only one side advances the queue and the remaining waiters are stuck for good.
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
        // Send failure (disconnect) — cannot set up an in-flight. Waiters get the gen (the contract returns one) but the marker comes from the re-request the reconnect transition drives. The entry is reused on the next request.
      }
      for (const w of pending.waiters) w.resolve(pending.gen)
    } else if (!entry.inflight) {
      // If there is neither an unfinished nor a merged request, clean up the entry (prevents a map leak).
      this.wsReplay.delete(agentId)
    }
  }

  /**
   * ★FIX-B: fully reset the replay single-flight state on socket close★: empty wsReplay so requestReplay after reconnect sends a new wire Subscribe (resolving the stuck state), and reject the merge waiters (pending.waiters) — this replay died without a closing marker.
   * The inflight request's promise already resolved with its gen, so it is not a settle target (a view with no marker is re-driven by the reconnect transition).
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
    // When a new start()/close() replaces an in-flight start(), the old socket's resolve/reject live only in closures about to be deleted and never settle → the caller awaiting that promise (DaemonControl.start → client.connect) hangs. Wake it with an explicit reject
    //   (on a normal handshake completion pendingReject is already null, so a healthy close does not wake it).
    //
    // ★Includes the window before ws creation (discover in flight)★: pendingReject is set synchronously in the openSocket executor but this.ws is assigned only after await invoke('discover_daemon'); this settle and the connectPromise cleanup sit above the `if (!ws) return`
    //   so they run even while ws is null — otherwise the second start() overwrites pendingReject, the first promise never settles, and the caller hangs.
    // ★Zombie guard★: invalidate the generation of the in-flight openSocket — a path that only cleans up (close(), start()'s cleanup step) supersedes an attempt yielding at an await so it is discarded on resume by myGen!==openGen.
    this.openGen++
    if (this.pendingReject) {
      const rej = this.pendingReject
      this.pendingReject = null
      rej(new Error('connection superseded (start/close) — ADR-0021'))
    }
    this.connectPromise = null
    // ★FIX-B: cleanupSocket also resets the replay state★: close()/start()'s supersede path deletes the old socket's onclose so handleClose does not run; empty it here so a new Subscribe goes out after reconnect and merge waiters are rejected. Calling both is harmless (idempotent).
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
