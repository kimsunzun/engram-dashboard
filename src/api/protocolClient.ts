// ProtocolClient — the carrier-independent implementation of AgentClient (ADR-0020, TRD Stage 3, ADR-0046 view-direct replay).
//
// View-direct replay (ADR-0046): progress state lives **per webview view (slot)**, so subs are keyed by viewId (slot id), not agentId — N views on the same agent keep independent progress.
// The replay boundary is the replayBoundary control event the transport raises (the normalized tag=255 marker); a view sort+dedup flushes only on a success marker at or above the myGen its own requestReplay returned.
//
// ★The attach trigger is the roster, not the socket (ADR-0046 amend)★: on disconnect a view only settles into `detached` (screen and cursor kept, no requests). The only trigger to request again is
// observing the agent in the authoritative roster (observeRoster). A restarted daemon's roster is empty; a socket re-transition trigger would fire a full Subscribe at agents that are not alive (refusal storm).
//
// "Same session" is checked against the incarnation marker the roster states, never inferred. The marker is a random number per incarnation, so its magnitude means nothing —
// ★never compare two markers by size here★ (it would invent "newer"). Frame side: mismatch = not mine (drop); only the authoritative roster decides that the incarnation changed.
// A dropped frame is not lost — the attach claims a full replay again.
//
// ★The next seq delivered to a view is always `last+1` (ADR-0231)★: the daemon issues seq and pushes the ring inside the replay lock but fans out outside it (ADR-0006), so N+1 can reach a view before N.
// Passing it and raising the cursor would make dedup permanently drop the late N (a pending list event would leave that window's list wrong). So frames after a gap are held and passed in order
// once the gap fills — in live and at the buffer→live boundary (flush); without a gap they pass at once.
// ★There is no time limit on waiting for a gap★ (a time limit is a magic number deciding the right answer — ADR-0038): the daemon sink and shell relay do not skip seqs (placeholder · interrupt),
// and the held amount is bounded only by the hold cap — on overflow it returns to the buffer phase and claims a full replay again (`holdLive`).

import type { Band } from '../../src-tauri/bindings/Band'
import type {
  AgentClient,
  ConnectionState,
  OutputChunk,
  OutputSubscription,
  QueuedInputListing,
  ReplayLiveInfo,
  UsageSnapshotPull,
  ViewOutputState,
  ViewPhase,
  ViewResetFn,
} from './agentClient'
import type { QueuedInputRow } from '../../crates/engram-dashboard-protocol/bindings/QueuedInputRow'
import type { UsageLimitSnapshot } from '../../crates/engram-dashboard-protocol/bindings/UsageLimitSnapshot'
import type { InboundMessage, Transport } from './transport'
import type {
  AgentBackendKind,
  AgentInfo,
  AgentProfile,
  AgentStatus,
  AgentOutputFormat,
  RestoreReport,
} from './types'

// ── replay state machine constants (ADR-0046 §2·§4) ────────────────────────────────────────────
/** Maximum attempts of the re-request ladder (bounded). When exhausted, the view transitions to the error state (no infinite runaway). */
const LADDER_MAX_ATTEMPTS = 3
const LADDER_BACKOFF_MS = [1000, 2000, 4000]
/** watchdog expiry (ms) — if no success marker arrives within this time while buffering, re-request (not flush, §2). */
const WATCHDOG_MS = 10_000
/**
 * Cap of a view's buffering buffer — twice the ring cap (§4). The buffer must be able to hold "the tail of the previous replay + its own whole replay".
 * On overflow, no partial keep (drop-oldest) → discard the buffer + re-request.
 */
const VIEW_BUFFER_MAX_BYTES = 4 * 1024 * 1024
const VIEW_BUFFER_MAX_FRAMES = 8192
/**
 * Cap of the live hold list (`SubState.heldFrames`) — kept smaller than the daemon ring cap (`output_core.rs` `REPLAY_MAX_EVENTS` 4096 · `REPLAY_MAX_BYTES` 2 MiB):
 * overflow recovery fills the gap with a full replay, and a larger hold would already be pushed out of the ring, so recovery would always truncate.
 */
// ADR-0231
const HELD_MAX_BYTES = 1024 * 1024
const HELD_MAX_FRAMES = 2048

interface BufferedFrame {
  tag: number
  seq: number
  bytes: Uint8Array
  /** Marker of the incarnation that emitted this frame. While buffering it may not be known which incarnation's replay is awaited, so two incarnations can mix in one buffer; flush keeps only the marker the success marker stated. */
  epoch: number
}

/** Minimal replayBoundary marker: while buffering with myGen unsettled, only the 1 highest gen is kept (§2). epoch is needed to re-evaluate the gen fence (old-epoch markers are invalid). */
interface HeldMarker {
  epoch: number
  gen: bigint
  truncated: boolean
  failed: boolean
  continuesConversation: boolean
  /** The head of this replay (the same field of the `InboundMessage` replayBoundary) — used as the flush starting point. */
  replayFrom: number
}

// ── internal subscription state (per view, ADR-0046 F1) ──────────────────────────────────────────────
interface SubState {
  agentId: string
  onChunk: (chunk: OutputChunk) => void
  onState?: (state: ViewPhase, info?: ReplayLiveInfo) => void
  onReset?: ViewResetFn
  phase: ViewPhase
  buffer: BufferedFrame[]
  bufferBytes: number
  /** **Unsettled (undefined)**: before the invoke response (the Channel may come first). gen fence: flush only on a success marker at or above its own myGen (blocks an early flush on someone else's/a previous replay). */
  myGen: bigint | undefined
  /** A marker that arrived while myGen was unsettled is kept (the 1 highest gen) and re-evaluated when myGen is settled (§2). */
  heldMarker: HeldMarker | undefined
  lastDeliveredSeq: number
  /** Frames that came first after a gap in live (`lastDeliveredSeq+1` not arrived), seq → frame; connected ones are passed in order when it arrives (`releaseHeld`). Always empty outside live: `startBuffering` and detached empty it, flush fills it anew. */
  heldFrames: Map<number, BufferedFrame>
  heldBytes: number
  /** Marker of the incarnation this view is reading (opaque — no magnitude comparison); undefined = not known yet. Only the success marker (flushToLive) adopts it — not frames (see handleOutput). */
  epoch: number | undefined
  /**
   * Whether the next flush should **drop the progress cursor and tell the consumer to clear the screen**. A new-incarnation attach sets it; the flush clears it.
   *
   * ★Clearing happens at result time, not request time (load-bearing)★: clearing at request time leaves the slot **permanently empty** when the replay is refused or times out (right after a daemon restart).
   * At result time a failed replay keeps the previous screen, and on success clearing and redrawing land on the same tick (no empty screen between).
   * ★Once set it does not come down before the flush★ — even a same-incarnation re-attach (newSession=false) cutting in must not cancel the cursor drop, or frames coming again from 0 would be buried under the old high-water.
   */
  restartPending: boolean
  token: number
  attempts: number
  backoffTimer: ReturnType<typeof setTimeout> | null
  watchdogTimer: ReturnType<typeof setTimeout> | null
}

interface Pending {
  resolve: (v: unknown) => void
  reject: (e: unknown) => void
}

type WireEvent = Record<string, unknown>

/** Band ② or ① `refused` — the bands a start attempt's cause never shows under (TRD S21 trd-3-2-u5 R21). */
function refusesCause(band: Band | null): boolean {
  return band !== null && (band.kind === 'version' || band.refused)
}

// ViewOutputState (§5 LLM control surface — ADR-0046): the type lives in agentClient.ts (the LLM discovers it by type).

export class ProtocolClient implements AgentClient {
  private readonly transport: Transport

  private pending = new Map<string, Pending>()
  private subs = new Map<string, SubState>()
  private subSeq = 0

  /** The last observed authoritative roster (agentId → incarnation marker). `null` = **not known yet**, not "empty": merging them would read the boot/reload/query-failure window as "agent does not exist" and block output. Back to `null` on disconnect (a dead daemon's roster is not a fact about a live one). */
  private roster: Map<string, number> | null = null

  private agentListCbs = new Set<(agents: AgentInfo[]) => void>()
  private statusCbs = new Set<(id: string, status: AgentStatus, epoch: number) => void>()
  private restoreCbs = new Set<(report: RestoreReport) => void>()
  private profileListCbs = new Set<(profiles: AgentProfile[]) => void>()
  private usageCbs = new Set<(snapshot: UsageLimitSnapshot, socketEpoch: number) => void>()

  private offMessage: (() => void) | null = null
  private offState: (() => void) | null = null

  private lastState: ConnectionState

  // ADR-0134: failure reason on the same surface as the connection state; held here because subscribers must be notified even when the state string does not change (the transport knows only state transitions).
  private connError: string | null = null
  private stateCbs = new Set<(state: ConnectionState) => void>()

  constructor(transport: Transport) {
    this.transport = transport
    this.lastState = transport.connectionState
    this.offMessage = transport.onMessage((msg) => this.route(msg))
    this.offState = transport.onConnectionStateChange((s) => {
      const prev = this.lastState
      this.lastState = s
      if (s === 'connected' && prev !== 'connected') {
        this.connError = null
        // Views are not revived here: socket up ≠ agent exists. Re-requests come from the roster observation (observeRoster) — pushed by broadcast or pulled by getAgents of eventBus.resyncAfterReconnect.
      } else if (s !== 'connected' && prev === 'connected') {
        const lost = new Error('connection lost')
        for (const p of this.pending.values()) p.reject(lost)
        this.pending.clear()
        this.detachAllViews()
      }
      // TRD S21 trd-3-2-u5 R21: a cause never shows under band ② or ① `refused` — drop it when one arrives.
      if (refusesCause(transport.connectionBand)) this.connError = null
      this.notifyState()
    })
  }

  // ── connection state (delegated to the transport) ───────────────────────────────────────────────────
  get connectionState(): ConnectionState {
    return this.transport.connectionState
  }

  get connectionBand(): Band | null {
    return this.transport.connectionBand
  }

  get connectionError(): string | null {
    return this.connError
  }

  reportConnectionError(reason: string | null): void {
    // ★Not recorded while connected, under ② or under ① `refused` (load-bearing)★: the clears are transitions into those states, so a cause written while one holds would wait for a clear that never comes
    //   (contract on `AgentClient.reportConnectionError`).
    const blocked = this.transport.connectionState === 'connected' || refusesCause(this.transport.connectionBand)
    const next = blocked ? null : reason
    if (this.connError === next) return
    this.connError = next
    this.notifyState()
  }

  onConnectionStateChange(cb: (state: ConnectionState) => void): () => void {
    this.stateCbs.add(cb)
    cb(this.transport.connectionState)
    return () => {
      this.stateCbs.delete(cb)
    }
  }

  private notifyState(): void {
    const s = this.transport.connectionState
    for (const cb of this.stateCbs) cb(s)
  }

  // ── explicit connect/disconnect (ADR-0021 §1·note3, delegated to the transport) ─────────────────────────────
  connect(): Promise<void> {
    return this.transport.start()
  }
  disconnect(): void {
    this.transport.close()
  }

  // ── inbound routing (normalized messages) ───────────────────────────────────────────────────
  private route(msg: InboundMessage): void {
    if (msg.kind === 'output') {
      this.handleOutput(msg)
      return
    }
    if (msg.kind === 'replayBoundary') {
      this.handleReplayBoundary(msg)
      return
    }
    this.handleEvent(msg.event)
  }

  private *viewsForAgent(agentId: string): Generator<SubState> {
    for (const st of this.subs.values()) {
      if (st.agentId === agentId) yield st
    }
  }

  /**
   * Normalized output frame — fan-out to every view watching the agent (ADR-0046 §2).
   * Tag-independent: epoch guard, seq dedup and hold do not look at the tag (0 terminal/1 structured) — both share the same seq space of OutputCore; the tag is only carried to onChunk.
   * So a tag1 placeholder sent in place of an event the daemon or shell could not carry also fills the gap of a terminal view.
   */
  private handleOutput(f: {
    tag: number
    agentId: string
    epoch: number
    seq: number
    bytes: Uint8Array
  }): void {
    for (const st of this.viewsForAgent(f.agentId)) {
      // detached = this view requested nothing. Another view's replay of the same agent arrives via fan-out, but taking it before the attach decision would shake the cursor without knowing the frame's incarnation.
      if (st.phase === 'error' || st.phase === 'detached') continue
      // Marker mismatch = not mine (live and buffering alike): the marker is random, so magnitude does not mean "newer"; incarnation rotation is declared only by the authoritative roster (observeRoster),
      //   whose attach claims a full replay again, so this frame is not lost.
      if (st.epoch !== undefined && f.epoch !== st.epoch) continue
      if (st.phase === 'live') {
        if (f.seq <= st.lastDeliveredSeq) continue
        // ADR-0231: frames after a gap are held, not passed (file head 「the next seq is always last+1」).
        if (f.seq > st.lastDeliveredSeq + 1) {
          this.holdLive(st, { tag: f.tag, seq: f.seq, bytes: f.bytes, epoch: f.epoch })
          continue
        }
        st.lastDeliveredSeq = f.seq
        st.onChunk({ tag: f.tag, seq: f.seq, bytes: f.bytes })
        this.releaseHeld(st)
        continue
      }
      // ★Markers are not adopted from frames (load-bearing)★: settling the incarnation from the first frame to arrive would let one late frame of the previous incarnation win, the mismatch guard above would drop
      //   every frame of the real new incarnation, and the view would exhaust the ladder and settle in error. Only the authority (the success marker, flushToLive) sets it, filtering other incarnations out of this buffer.
      this.pushBuffer(st, f)
    }
  }

  private pushBuffer(
    st: SubState,
    f: { tag: number; epoch: number; seq: number; bytes: Uint8Array },
  ): void {
    st.buffer.push({ tag: f.tag, seq: f.seq, bytes: f.bytes, epoch: f.epoch })
    st.bufferBytes += f.bytes.length
    if (st.bufferBytes > VIEW_BUFFER_MAX_BYTES || st.buffer.length > VIEW_BUFFER_MAX_FRAMES) {
      // ★No partial flush (§4)★: discard the whole buffer *then* re-request (pathological, normally unreachable). Discard comes first: otherwise the pre-overflow gen success marker flushes stale incomplete frames
      //   and the re-requested replay's complete content is trapped behind the dedup high-water (FIX-2).
      console.warn(`[ProtocolClient] view buffer cap exceeded (agent=${st.agentId}) — discarded, re-requesting`)
      st.buffer = []
      st.bufferBytes = 0
      // ★FIX-A: overflow also invalidates the gen fence★ (unlike the failure/watchdog paths, which keep the buffer so a late old-gen success marker may flush it — zombie recovery).
      //   Overflow discarded the whole buffer: an old-gen success marker arriving before the re-request backoff fires would flush an *empty* buffer to live (content lost) and cancel the scheduled re-request.
      //   So no marker may flush until the re-requested replay's new gen is settled.
      st.myGen = undefined
      st.heldMarker = undefined
      this.ladderRerequest(st)
    }
  }

  /**
   * Holds one frame after a gap in live. The same seq already held is dropped (same incarnation + seq = same event).
   *
   * ★On overflow it is `startBuffering`, not the re-request ladder (load-bearing)★: `ladderRerequest` drops the scheduled re-claim when `phase !== 'buffering'`, so in live the re-claim would never go out.
   * Same incarnation → `newSession=false`: the cursor is kept so dedup eats the overlapping front part and the new replay fills the gap by the flush rule (a gap pushed out of the ring skips from the replay head — truncation).
   */
  // ADR-0231
  private holdLive(st: SubState, f: BufferedFrame): void {
    if (st.heldFrames.has(f.seq)) return
    st.heldFrames.set(f.seq, f)
    st.heldBytes += f.bytes.length
    if (st.heldBytes > HELD_MAX_BYTES || st.heldFrames.size > HELD_MAX_FRAMES) {
      console.warn(`[ProtocolClient] live hold cap exceeded (agent=${st.agentId}) — back to the buffer phase, re-requesting`)
      this.startBuffering(st, st.epoch, /*resetLadder*/ false, /*newSession*/ false)
    }
  }

  /** Passes the held frames connected from `lastDeliveredSeq+1` in order. */
  private releaseHeld(st: SubState): void {
    let next = st.heldFrames.get(st.lastDeliveredSeq + 1)
    while (next) {
      st.heldFrames.delete(next.seq)
      st.heldBytes -= next.bytes.length
      st.lastDeliveredSeq = next.seq
      st.onChunk({ tag: next.tag, seq: next.seq, bytes: next.bytes })
      next = st.heldFrames.get(st.lastDeliveredSeq + 1)
    }
  }

  private dropHeld(st: SubState): void {
    st.heldFrames = new Map()
    st.heldBytes = 0
  }

  /** Replay boundary marker (ADR-0046 §2) — the control event the transport raises after normalizing the tag=255 marker. */
  private handleReplayBoundary(m: {
    agentId: string
    epoch: number
    gen: bigint
    truncated: boolean
    failed: boolean
    continuesConversation: boolean
    replayFrom: number
  }): void {
    for (const st of this.viewsForAgent(m.agentId)) {
      this.evalMarker(st, m)
    }
  }

  /** Marker decision for one view (§2 state transition table, every marker row), evaluated at marker arrival against the SubState of that moment (not registration time). */
  private evalMarker(st: SubState, m: HeldMarker): void {
    // live·error views: ignore markers (any gen) — someone else's replay boundary reaching via fan-out. For live, dedup and hold suffice (§2).
    if (st.phase !== 'buffering') return
    // myGen unsettled: the marker is not dropped; the 1 highest gen is kept and re-evaluated when myGen is settled (resolveHeldMarker).
    //   Replacement (FIX-3): replace if the gen is higher, or the gen is the same, the kept one failed and the new one succeeded. Without the second rule, when a gen's failure marker (deadline) precedes that gen's
    //   late success marker (zombie late-Complete recovery), the success is dropped, cannot flush, and falls to the ladder. Success wins.
    if (st.myGen === undefined) {
      const held = st.heldMarker
      const replace =
        held === undefined || m.gen > held.gen || (m.gen === held.gen && held.failed && !m.failed)
      if (replace) {
        st.heldMarker = {
          epoch: m.epoch,
          gen: m.gen,
          truncated: m.truncated,
          failed: m.failed,
          continuesConversation: m.continuesConversation,
          replayFrom: m.replayFrom,
        }
      }
      return
    }
    // ★Gen fence (ADR-0046)★: a marker below its own myGen = someone else's/a previous replay → ignored; an epoch mismatch is ignored too. Flush only on gen≥myGen: a later replay of the same agent
    //   is a cumulative superset of the previous (full-from-oldest), so sort+dedup of a late-mounted view's buffer on its own gen marker is complete, while an early flush on a previous gen's marker loses its own replay head to dedup (bug B).
    if (m.gen < st.myGen) return
    if (m.failed) {
      // ★Failure marker (§2)★: no flush — keep the buffer (sort+dedup absorbs duplicates; the next replay is full-from-oldest, so discarding would only lose frames to be received again) and run the re-request ladder.
      // ★It comes before the epoch fence (load-bearing)★: a failure marker's epoch is **not authoritative** — a failure has no SubscribeAck, so the sender carries the last known value (0 when none) as best effort
      //   (src-tauri plan_subscribe_refusal · deadline sweep). Fencing on it would drop the marker silently and the view would wait for the 10-second watchdog, defeating its purpose (push the ladder immediately).
      //   Misdelivery is already covered by the gen fence above.
      this.ladderRerequest(st)
      return
    }
    // A success marker adopts the epoch (flushToLive), so filter here — taking an old-generation/old-epoch replay's boundary as its own would flush an incomplete buffer.
    if (st.epoch !== undefined && m.epoch !== st.epoch) return
    this.flushToLive(st, m.epoch, m.truncated, m.continuesConversation, m.replayFrom)
  }

  private flushToLive(
    st: SubState,
    epoch: number,
    truncated: boolean,
    continuesConversation: boolean,
    replayFrom: number,
  ): void {
    // Epoch adoption: settled with the success marker's epoch (passed the src-tauri decide_epoch filter — ADR-0046 amends ADR-0007 "epoch authority = SubscribeAck alone").
    st.epoch = epoch
    // Cleared now that the new incarnation's replay has actually arrived (not at request time — see `restartPending`): the cursor drop and consumer signal sit right before the delivery below, so clearing and redrawing land on the same tick.
    if (st.restartPending) {
      st.restartPending = false
      st.lastDeliveredSeq = -1
      st.onReset?.()
    }
    // Frames of other incarnations are dropped here: a buffer of a view attached with an unknown marker may hold late frames of the previous incarnation whose seq space overlaps (both from 0), so dedup cannot tell them apart.
    // Sort by seq ascending before flush (out-of-order defence): array order delivering a large seq first would raise the high-water and dedup would drop a small one.
    const ordered = st.buffer.filter((f) => f.epoch === epoch).sort((a, b) => a.seq - b.seq)
    st.buffer = []
    st.bufferBytes = 0
    this.dropHeld(st)
    // ADR-0231: sorts, but does not pass everything — passes only what is connected from `max(last+1, replay head)`; the first gap after it moves into the hold. The daemon puts `ReplayComplete` into the connection queue
    //   outside the subscribers lock, so inverted live frames split around the marker (`[…L, L+2, marker, L+1]`) — passing what follows the gap first would make live dedup drop the late L+1.
    //   Only the replay head may skip `last+1` (a replay is the whole ring, contiguous, so the empty space before the head is ring eviction). ★Do not estimate the head as the buffer's minimum seq★:
    //   an empty replay (new incarnation, seq 1 before the marker · 0 after) would pass from 1 and drop 0.
    const start = Math.max(st.lastDeliveredSeq + 1, replayFrom)
    st.lastDeliveredSeq = start - 1
    for (const frame of ordered) {
      if (frame.seq <= st.lastDeliveredSeq) continue
      if (frame.seq > st.lastDeliveredSeq + 1) {
        if (!st.heldFrames.has(frame.seq)) {
          st.heldFrames.set(frame.seq, frame)
          st.heldBytes += frame.bytes.length
        }
        continue
      }
      st.lastDeliveredSeq = frame.seq
      st.onChunk({ tag: frame.tag, seq: frame.seq, bytes: frame.bytes })
    }
    if (truncated) console.warn('[ProtocolClient] output truncated for', st.agentId)
    st.phase = 'live'
    st.attempts = 0
    this.clearTimers(st)
    // ADR-0226: whether this is a resume incarnation is passed on exactly as the success marker carried it, not stored in the view state. `epoch` feeds the queued input re-attach check's incarnation filter (ADR-0231).
    st.onState?.('live', { continuesConversation, epoch })
  }

  /** Re-request ladder (bounded — §2·§4). When exhausted, transition to phase='error' and surface it through onState. A remount or connected transition resets the ladder. */
  private ladderRerequest(st: SubState): void {
    // If a backoff is in progress, do not schedule a duplicate (one ladder step is one timer).
    if (st.backoffTimer) return
    if (st.attempts >= LADDER_MAX_ATTEMPTS) {
      st.phase = 'error'
      st.buffer = []
      st.bufferBytes = 0
      this.clearTimers(st)
      st.onState?.('error')
      return
    }
    st.attempts += 1
    const delay = LADDER_BACKOFF_MS[Math.min(st.attempts - 1, LADDER_BACKOFF_MS.length - 1)]
    // The watchdog is meaningless during the backoff wait (a re-request is scheduled) → cleared, then re-armed after the re-request.
    this.clearWatchdog(st)
    st.backoffTimer = setTimeout(() => {
      st.backoffTimer = null
      // At re-request time, check this view is still buffering (it may have gone live meanwhile through a success marker).
      if (st.phase !== 'buffering') return
      this.issueReplay(st)
    }, delay)
  }

  /**
   * Turns the view back to buffering and reissues a full-from-oldest replay.
   *
   * `newSession` = whether this replay belongs to an **incarnation that numbers again from 0**. If true the progress cursor must be dropped — otherwise the new incarnation's frames are all buried under
   * the old high-water and dedup'd, and the view cannot draw a byte until relaunch. But the drop is only scheduled here; the actual drop and screen clearing happen at flush time (`restartPending` has the rationale).
   * If false (same incarnation), the cursor stays, dedup absorbs the overlapping front part, and only what was missed while disconnected is delivered.
   *
   * `epoch` = the marker this view accepts from now on. For a new incarnation `undefined` is passed so it is adopted from the new replay (self-corrects even if the roster stated another marker a beat late).
   */
  private startBuffering(
    st: SubState,
    epoch: number | undefined,
    resetLadder: boolean,
    newSession: boolean,
  ): void {
    st.phase = 'buffering'
    st.buffer = []
    st.bufferBytes = 0
    // ADR-0231: the held frames are dropped too — the full replay coming now fills their place again.
    this.dropHeld(st)
    if (newSession) st.restartPending = true
    st.epoch = epoch
    st.myGen = undefined
    st.heldMarker = undefined
    if (resetLadder) st.attempts = 0
    this.clearTimers(st)
    // ADR-0145: notify the buffering restart — otherwise the consumer's 'restore done' stays open and re-flushing the accumulated history flips the screen (empty state → conversation flicker). After the phase assignment, so a callback reading getViewOutputState is consistent.
    st.onState?.('buffering')
    // A roster attach re-requests immediately (no backoff).
    this.issueReplay(st)
  }

  private issueReplay(st: SubState): void {
    st.myGen = undefined
    st.heldMarker = undefined
    this.armWatchdog(st)
    const token = st.token
    const viewId = this.findViewId(st)
    this.transport
      .requestReplay(st.agentId)
      .then((gen) => {
        // Token/liveness guard: if this SubState was replaced by unsubscribe/resubscribe between round trips, plant nothing.
        if (viewId === null || this.subs.get(viewId)?.token !== token) return
        if (st.phase !== 'buffering') return // already transitioned to live/error (late return) — ignored.
        st.myGen = gen
        // Re-evaluate the held marker now that myGen is settled (a marker may have come before the invoke response).
        this.resolveHeldMarker(st)
      })
      .catch(() => {
        // requestReplay reject (not connected etc.): no marker comes; the watchdog drives the re-request. If really disconnected, the following detach clears this view's timers and settles it as detached; reviving is the roster attach's job.
      })
  }

  private resolveHeldMarker(st: SubState): void {
    const held = st.heldMarker
    if (!held || st.myGen === undefined) return
    st.heldMarker = undefined
    // The gen fence · failed · epoch rules apply to held as is (same order as evalMarker after myGen is settled — failure comes before the epoch fence).
    if (held.gen < st.myGen) return
    if (held.failed) {
      this.ladderRerequest(st)
      return
    }
    if (st.epoch !== undefined && held.epoch !== st.epoch) return
    this.flushToLive(st, held.epoch, held.truncated, held.continuesConversation, held.replayFrom)
  }

  private armWatchdog(st: SubState): void {
    this.clearWatchdog(st)
    st.watchdogTimer = setTimeout(() => {
      st.watchdogTimer = null
      if (st.phase !== 'buffering') return
      // No flush: the watchdog re-requests (§2), through the ladder (new myGen).
      this.ladderRerequest(st)
    }, WATCHDOG_MS)
  }

  private clearWatchdog(st: SubState): void {
    if (st.watchdogTimer) {
      clearTimeout(st.watchdogTimer)
      st.watchdogTimer = null
    }
  }

  private clearTimers(st: SubState): void {
    this.clearWatchdog(st)
    if (st.backoffTimer) {
      clearTimeout(st.backoffTimer)
      st.backoffTimer = null
    }
  }

  /**
   * Whether a request may be sent to this agent now. ★An unknown roster (`roster === null`) is **not absence** — send★: boot, webview reload and roster query failure land here; treating it as absence would block output
   * on one failed query and delay the first picture by a roster round trip. A wrong send costs at worst one refusal, already absorbed (refusal → failure marker → ladder; a daemon refusal does not leave the single-flight slot as a zombie).
   */
  private canAttach(agentId: string): boolean {
    return this.roster === null || this.roster.has(agentId)
  }

  /** subs is small, so a linear search is harmless. */
  private findViewId(target: SubState): string | null {
    for (const [viewId, st] of this.subs) {
      if (st === target) return viewId
    }
    return null
  }

  /**
   * When the connection drops, every view settles into detached — **neither the screen nor the progress cursor is touched** (only the pipe is gone; the intent "this slot watches this agent" stays).
   * Only the buffer is dropped: it is an incomplete replay fragment of the disconnected generation and would mix in among new frames if the next attach is a new incarnation. epoch is kept — it is "the incarnation I was reading" for the next roster check.
   * The roster is dropped too: it belongs to the daemon that just disconnected, and using it for a live daemon would read agents that vanished after a restart as alive.
   */
  private detachAllViews(): void {
    this.roster = null
    for (const st of this.subs.values()) this.detachView(st)
  }

  /** Settles one view into detached (idempotent). Screen, progress cursor and marker stay as they are — the detachAllViews comment above. */
  private detachView(st: SubState): void {
    if (st.phase === 'detached') return
    st.phase = 'detached'
    st.buffer = []
    st.bufferBytes = 0
    this.dropHeld(st)
    st.myGen = undefined
    st.heldMarker = undefined
    this.clearTimers(st)
    st.onState?.('detached')
  }

  /**
   * One observation of the authoritative roster — **the only trigger of attach and incarnation rotation** (ADR-0046 amend).
   *
   * Both the broadcast (AgentListUpdated) and the query (getAgents) must converge here: the first roster after a reconnect comes through the query (eventBus.resyncAfterReconnect), so omitting it means the main recovery path never attaches.
   *
   * An agent not in the roster settles the view into `detached` — nothing sent (cuts the refusal storm of a full Subscribe at a restarted daemon's empty roster). An attached view is lowered too:
   * otherwise a slot watching a reaped agent stays `live` in `getViewOutputState` while the screen has stopped (a §5 control contract).
   *
   * The rotation of an attached view is issued here too: the frame side only drops a marker mismatch (handleOutput), so nothing else recognizes a re-spawn — removing this branch leaves a live slot blank after a re-spawn.
   */
  private observeRoster(agents: AgentInfo[]): void {
    const roster = new Map(agents.map((a) => [a.id, a.epoch]))
    this.roster = roster
    for (const st of this.subs.values()) {
      // Existence is decided by `has`, the marker by `get`: `get() !== undefined` would read an entry that could not carry a marker as "agent does not exist" and the slot would never attach.
      if (!roster.has(st.agentId)) {
        this.detachView(st)
        continue
      }
      const tag = roster.get(st.agentId)
      // A changed marker = a different incarnation (magnitude is not looked at). A view that does not yet know its incarnation (undefined) has no progress to keep, so it is "continuing the same incarnation".
      // ★A roster entry with no marker is **not assumed the same incarnation**★: that assumption was the original defect (the kept cursor dedups away the whole new incarnation from 0 and the slot stays empty forever).
      //   A wrong rotation costs one unneeded full replay with clearing and redrawing on the same tick — a noisy re-replay is chosen over a silent permanent stop.
      const newSession = st.epoch !== undefined && tag !== st.epoch
      // error attaches like detached: the only recovery trigger for a view that exhausted the ladder (error can be reached without a disconnect, so checking only detached would never revive it).
      if (st.phase === 'detached' || st.phase === 'error' || newSession) {
        this.startBuffering(st, newSession ? undefined : st.epoch, /*resetLadder*/ true, newSession)
      }
    }
  }

  // ── JSON control event handling ────────────────────────────────────────────────────
  private handleEvent(msg: WireEvent): void {
    if ('Ack' in msg) {
      this.resolvePending((msg.Ack as { request_id: string }).request_id, undefined)
      return
    }
    if ('Created' in msg) {
      const c = msg.Created as { request_id: string; profile: AgentProfile }
      this.resolvePending(c.request_id, c.profile)
      return
    }
    if ('Spawned' in msg) {
      const s = msg.Spawned as { request_id: string; agent: AgentInfo }
      this.resolvePending(s.request_id, s.agent)
      return
    }
    if ('Error' in msg) {
      const e = msg.Error as { request_id?: string | null; message: string }
      if (e.request_id) this.rejectPending(e.request_id, new Error(e.message))
      else console.warn('[ProtocolClient] backend error:', e.message)
      return
    }
    if ('SubscribeFailed' in msg) {
      // ★Only a diagnostic is left — the state machine is not touched★: the carrier normalizes this refusal into a replay boundary (failure marker; Tauri = src-tauri single-flight, direct WS = wsTransport.observeReplayWire),
      //   so touching the view here would bring one event in through two paths and push the ladder twice. ★Do not remove the log★: once the refusal leaves `Error`, the backend error warn below cannot catch it.
      const s = msg.SubscribeFailed as { agent_id: string; reason: string }
      console.warn('[ProtocolClient] subscribe refused:', s.agent_id, s.reason)
      return
    }
    if ('SubscribeAck' in msg) {
      // ADR-0046: epoch authority moved to the src-tauri decide_epoch filter + success marker epoch adoption. SubscribeAck is not an input to the state machine (the marker carries boundary and epoch); `truncated` arrives via marker flags.
      return
    }
    if ('ReplayComplete' in msg) {
      // ADR-0046: the boundary decision is replayBoundary (marker) alone. This control is ignored (the carrier normalizes it into a marker).
      return
    }
    if ('AgentList' in msg) {
      const a = msg.AgentList as { request_id: string; agents: AgentInfo[] }
      this.resolvePending(a.request_id, a.agents)
      return
    }
    if ('AgentListUpdated' in msg) {
      const agents = (msg.AgentListUpdated as { agents: AgentInfo[] }).agents
      for (const cb of this.agentListCbs) cb(agents)
      // The roster door that pushes; the door that pulls is getAgents (not through this routing).
      this.observeRoster(agents)
      return
    }
    if ('ProfileList' in msg) {
      const p = msg.ProfileList as { request_id: string; profiles: AgentProfile[] }
      this.resolvePending(p.request_id, p.profiles)
      return
    }
    if ('ProfileListUpdated' in msg) {
      const profiles = (msg.ProfileListUpdated as { profiles: AgentProfile[] }).profiles
      for (const cb of this.profileListCbs) cb(profiles)
      return
    }
    if ('UsageLimitsUpdated' in msg) {
      // `socketEpoch` is added by the shell carrier — daemon frames raised by the direct carrier do not have it, so 0 (= marker unknown) is passed.
      const u = msg.UsageLimitsUpdated as { snapshot: UsageLimitSnapshot; socketEpoch?: unknown }
      const socketEpoch = typeof u.socketEpoch === 'number' ? u.socketEpoch : 0
      for (const cb of this.usageCbs) cb(u.snapshot, socketEpoch)
      return
    }
    if ('Snapshot' in msg) {
      const s = msg.Snapshot as { request_id: string; agent_id: string; chunks: unknown[] }
      this.resolvePending(s.request_id, s.chunks)
      return
    }
    // ADR-0231: the dedicated reply of the two queued input commands, paired with the sending method (without this branch the promise never resolves). A refusal is rejected by the `Error` branch above with the same request_id.
    if ('QueuedInputs' in msg) {
      const q = msg.QueuedInputs as {
        request_id: string
        inputs?: QueuedInputRow[]
        as_of_seq?: number | null
        epoch: number
        stopped_after_error?: boolean
      }
      const listing: QueuedInputListing = {
        inputs: Array.isArray(q.inputs) ? q.inputs : [],
        as_of_seq: typeof q.as_of_seq === 'number' ? q.as_of_seq : null,
        epoch: q.epoch,
        stopped_after_error: q.stopped_after_error === true,
      }
      this.resolvePending(q.request_id, listing)
      return
    }
    if ('QueuedInputCancelReply' in msg) {
      const c = msg.QueuedInputCancelReply as { request_id: string; outcome: string }
      this.resolvePending(c.request_id, c.outcome)
      return
    }
    if ('StatusChanged' in msg) {
      const s = msg.StatusChanged as { agent_id: string; status: AgentStatus; epoch: number }
      for (const cb of this.statusCbs) cb(s.agent_id, s.status, s.epoch)
      return
    }
    if ('RestoreResult' in msg) {
      const r = (msg.RestoreResult as { report: RestoreReport }).report
      for (const cb of this.restoreCbs) cb(r)
      return
    }
    // Hello/InputLeaseChanged etc. are not consumed here. Ignored.
  }

  // ── request_id pending helpers ──────────────────────────────────────────────────────
  private resolvePending(requestId: string, value: unknown): void {
    const p = this.pending.get(requestId)
    if (p) {
      this.pending.delete(requestId)
      p.resolve(value)
    }
  }
  private rejectPending(requestId: string, err: unknown): void {
    const p = this.pending.get(requestId)
    if (p) {
      this.pending.delete(requestId)
      p.reject(err)
    }
  }

  private async sendCommand<T>(build: (requestId: string) => unknown): Promise<T> {
    await this.transport.ensureReady()
    const requestId = crypto.randomUUID()
    return new Promise<T>((resolve, reject) => {
      this.pending.set(requestId, { resolve: resolve as (v: unknown) => void, reject })
      try {
        const r = this.transport.send(build(requestId))
        if (r && typeof (r as Promise<void>).catch === 'function') {
          ;(r as Promise<void>).catch((e) => {
            this.pending.delete(requestId)
            reject(e)
          })
        }
      } catch (e) {
        this.pending.delete(requestId)
        reject(e)
      }
    })
  }

  // ── output subscription (per view, ADR-0046 F1) ─────────────────────────────────────────────────
  async subscribeOutput(
    viewId: string,
    agentId: string,
    onChunk: (chunk: OutputChunk) => void,
    onState?: (state: ViewPhase, info?: ReplayLiveInfo) => void,
    onReset?: ViewResetFn,
  ): Promise<OutputSubscription> {
    const token = ++this.subSeq
    const st: SubState = {
      agentId,
      onChunk,
      onState,
      onReset,
      phase: 'buffering',
      buffer: [],
      bufferBytes: 0,
      myGen: undefined,
      heldMarker: undefined,
      lastDeliveredSeq: -1,
      heldFrames: new Map(),
      heldBytes: 0,
      epoch: undefined,
      restartPending: false,
      token,
      attempts: 0,
      backoffTimer: null,
      watchdogTimer: null,
    }
    // ★Clean up the old SubState's timers (FIX-1)★: a resubscription of the same viewId replaces it in the map, but its watchdog/backoff timers survive unless cleared and on expiry issue a stray requestReplay
    //   through ladderRerequest (issueReplay's token guard does not stop an already scheduled callback) — a re-request storm.
    const prev = this.subs.get(viewId)
    if (prev) this.clearTimers(prev)
    // ★subs.set runs synchronously *before* the await (blocks the StrictMode double-subscribe race)★: this function is async and yields at `await ensureReady()`; StrictMode runs the subscribe effect twice in quick succession —
    //   different viewIds are different SubStates (normal), the same viewId (resubscribe) needs the set hoisted so the final surviving SubState is settled. Premise of the replay issue guard below.
    this.subs.set(viewId, st)
    // ★Zombie subscription rollback when ensureReady fails★: the set moved before the await, so on reject/hang st would stay in subs and frames leak into a dead subscription. Only its own registration is rolled back (the token guard protects a normal resubscribe).
    try {
      await this.transport.ensureReady()
    } catch (e) {
      if (this.subs.get(viewId)?.token === token) {
        this.clearTimers(st)
        this.subs.delete(viewId)
      }
      throw e
    }
    // ★Only the surviving subscriber issues the replay (suppresses StrictMode duplicate invokes)★: a replaced old st is skipped (no duplicate re-request storm); single-flight merges overlap with the assignment-triggered replay of a normal mount.
    if (this.subs.get(viewId)?.token === token) {
      // Mount uses the same rule as re-attach (all in `canAttach`, so one question has one answer): the old unconditional fire at a missing agent hardened into refusal → ladder → error.
      if (this.canAttach(st.agentId)) {
        this.issueReplay(st)
      } else {
        // While the roster says "absent", nothing is sent. Reviving is the roster observation's job (observeRoster).
        st.phase = 'detached'
        st.onState?.('detached')
      }
    } else {
      // A replaced old st drops out quietly without timers (the surviving subscriber issues); its buffer does not fill either (fan-out hits only live sts and viewsForAgent does not catch it).
      this.clearTimers(st)
    }
    return {
      unsubscribe: () => {
        // ★Delete only when the current subs entry is my token (stale-unsubscribe guard)★: a late old unsubscribe must not delete the live subscription of a resubscribe.
        if (this.subs.get(viewId)?.token === token) {
          this.clearTimers(st)
          this.subs.delete(viewId)
        }
        // BLOCK-1 (ADR-0046): wire Subscribe/Unsubscribe is not sent by any path; daemon subscription cleanup is the router Unsubscribe (prune) alone. Here only the JS callback is detached.
      },
    }
  }

  /** LLM control surface (§5) — per-view replay state query, to observe error exhaustion (3 failed re-requests) etc. and decide on restarting. */
  getViewOutputState(viewId: string): ViewOutputState | null {
    const st = this.subs.get(viewId)
    if (!st) return null
    return {
      agentId: st.agentId,
      phase: st.phase,
      buffered: st.buffer.length,
      held: st.heldFrames.size,
      attempts: st.attempts,
    }
  }

  // ── commands (interface → wire) ───────────────────────────────────────────────────────
  /**
   * ★Activation (the spawn family) uses that agent id only after awaiting this Promise★ — same for `spawnProfile`. The daemon takes activation commands off that connection's arrival-order lane and runs them as separate tasks
   * (activation alone waits for the outcome; keeping it in the lane would delay every other command), so a follow-up sent without waiting **runs first** and fails with 「agent not found」.
   * Resending an activation of the same id before the answer arrives is forbidden too (two requests land together in the daemon's check/register window).
   * Canonical contract: 「the call contract of the activation set」 at the head of `AgentCommand` in `crates/engram-dashboard-protocol/src/messages.rs`.
   */
  spawnAgent(cwd: string, backend: AgentBackendKind): Promise<AgentInfo> {
    return this.sendCommand<AgentInfo>((request_id) => ({ SpawnByCwd: { cwd, backend, request_id } }))
  }
  killAgent(agentId: string): Promise<void> {
    return this.sendCommand<void>((request_id) => ({ Kill: { agent_id: agentId, request_id } }))
  }
  interruptAgent(agentId: string): Promise<void> {
    return this.sendCommand<void>((request_id) => ({
      Interrupt: { agent_id: agentId, request_id },
    }))
  }
  writeStdin(agentId: string, data: Uint8Array): Promise<void> {
    return this.sendCommand<void>((request_id) => ({
      WriteStdin: { agent_id: agentId, data: Array.from(data), request_id },
    }))
  }
  async resizePty(agentId: string, cols: number, rows: number): Promise<void> {
    await this.transport.ensureReady()
    this.transport.send({ Resize: { agent_id: agentId, cols, rows, viewport_id: null } })
  }
  async getAgents(): Promise<AgentInfo[]> {
    const agents = await this.sendCommand<AgentInfo[]>((request_id) => ({ ListAgents: { request_id } }))
    // The roster door that pulls: the reply is collected by the request_id pending and does not pass handleEvent's broadcast branch; the first roster after a reconnect comes this way, so feed it here (see observeRoster).
    this.observeRoster(agents)
    return agents
  }
  getSnapshot(agentId: string): Promise<unknown[]> {
    return this.sendCommand<unknown[]>((request_id) => ({
      GetSnapshot: { agent_id: agentId, request_id },
    }))
  }
  listQueuedInputs(agentId: string): Promise<QueuedInputListing> {
    return this.sendCommand<QueuedInputListing>((request_id) => ({
      ListQueuedInputs: { agent_id: agentId, request_id },
    }))
  }
  cancelQueuedInput(agentId: string, inputId: string): Promise<string> {
    return this.sendCommand<string>((request_id) => ({
      CancelQueuedInput: { agent_id: agentId, input_id: inputId, request_id },
    }))
  }

  // ── profile CRUD ────────────────────────────────────────────────────────────────
  listProfiles(): Promise<AgentProfile[]> {
    return this.sendCommand<AgentProfile[]>((request_id) => ({ ListProfiles: { request_id } }))
  }
  createClaudeProfile(
    name: string,
    cwd: string,
    extraArgs: string[],
    env: [string, string][],
    autoRestore: boolean,
    outputFormat: AgentOutputFormat = 'Terminal',
  ): Promise<AgentProfile> {
    return this.sendCommand<AgentProfile>((request_id) => ({
      CreateProfile: {
        name,
        cwd,
        extra_args: extraArgs,
        env,
        auto_restore: autoRestore,
        output_format: outputFormat,
        backend: 'claude',
        request_id,
      },
    }))
  }
  createCodexProfile(
    name: string,
    cwd: string,
    extraArgs: string[],
    env: [string, string][],
    autoRestore: boolean,
    outputFormat: AgentOutputFormat = 'Terminal',
  ): Promise<AgentProfile> {
    return this.sendCommand<AgentProfile>((request_id) => ({
      CreateProfile: {
        name,
        cwd,
        extra_args: extraArgs,
        env,
        auto_restore: autoRestore,
        output_format: outputFormat,
        backend: 'codex',
        request_id,
      },
    }))
  }
  deleteProfile(agentId: string): Promise<void> {
    return this.sendCommand<void>((request_id) => ({
      DeleteProfile: { profile_id: agentId, request_id },
    }))
  }
  /** Use that agent id only after awaiting the answer — contract on `spawnAgent`'s doc. */
  spawnProfile(agentId: string, resume: boolean): Promise<AgentInfo> {
    return this.sendCommand<AgentInfo>((request_id) => ({
      SpawnProfile: { profile_id: agentId, resume, request_id },
    }))
  }
  setProfileAutoRestore(agentId: string, autoRestore: boolean): Promise<void> {
    return this.sendCommand<void>((request_id) => ({
      SetProfileAutoRestore: { profile_id: agentId, auto_restore: autoRestore, request_id },
    }))
  }
  renameProfile(agentId: string, name: string | null): Promise<void> {
    // Reply=Ack(void). The display name update comes with the next ProfileListUpdated broadcast (no optimistic update, ADR-0061).
    return this.sendCommand<void>((request_id) => ({
      RenameProfile: { profile_id: agentId, name, request_id },
    }))
  }
  reparentProfile(childId: string, parentId: string | null): Promise<void> {
    // Reply=Ack(void); the hierarchy update comes with the next ProfileListUpdated broadcast (no optimistic update, ADR-0072). An invalid move (cycle/self/2 levels/non-existent parent) is a backend Error → reject.
    return this.sendCommand<void>((request_id) => ({
      ReparentProfile: { child_id: childId, parent_id: parentId, request_id },
    }))
  }

  // ── usage (TRD S21 usage-limit-slot §1-8) ──────────────────────────────────────
  refreshUsageLimits(vendor: AgentBackendKind): Promise<void> {
    return this.sendCommand<void>((request_id) => ({ RefreshUsageLimits: { vendor, request_id } }))
  }
  getUsageSnapshot(): Promise<UsageSnapshotPull> {
    return this.transport.getUsageSnapshot()
  }

  // ── status/list/restore/profile events — registry registration + remove disposer ──────────────────
  onAgentListUpdated(cb: (agents: AgentInfo[]) => void): () => void {
    this.agentListCbs.add(cb)
    return () => {
      this.agentListCbs.delete(cb)
    }
  }
  onStatusChanged(cb: (id: string, status: AgentStatus, epoch: number) => void): () => void {
    this.statusCbs.add(cb)
    return () => {
      this.statusCbs.delete(cb)
    }
  }
  onRestoreResult(cb: (report: RestoreReport) => void): () => void {
    this.restoreCbs.add(cb)
    return () => {
      this.restoreCbs.delete(cb)
    }
  }
  onProfileListUpdated(cb: (profiles: AgentProfile[]) => void): () => void {
    this.profileListCbs.add(cb)
    return () => {
      this.profileListCbs.delete(cb)
    }
  }
  onUsageLimitsUpdated(cb: (snapshot: UsageLimitSnapshot, socketEpoch: number) => void): () => void {
    this.usageCbs.add(cb)
    return () => {
      this.usageCbs.delete(cb)
    }
  }

  // ── explicit shutdown ───────────────────────────────────────────────────────────────────
  close(): void {
    const closed = new Error('client closed')
    for (const p of this.pending.values()) p.reject(closed)
    this.pending.clear()
    for (const st of this.subs.values()) this.clearTimers(st)
    this.subs.clear()
    if (this.offMessage) {
      this.offMessage()
      this.offMessage = null
    }
    if (this.offState) {
      this.offState()
      this.offState = null
    }
    this.transport.close()
  }
}
