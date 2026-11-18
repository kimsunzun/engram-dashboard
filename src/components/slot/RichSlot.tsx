// RichSlot — structured (JSON mode) live render slot (ADR-0044/0045).
//
// Subscribes to backend-refined structured output tag1 frames with the same discipline as TerminalSlot, accumulates them in StructuredEventAccumulator,
// and draws them plus a text input box (Enter=send, Shift+Enter=newline). Input layouts: pinned to the bottom with a conversation, centered when empty (ADR-0145).
// Live path only (no fixture mode). tag0 (terminal bytes) is ignored — tag gate.
//
// Empty state (ADR-0145): drawn only on restore-complete ('live') + 0 items + nothing sent; it touches only that decision, not subscription/accumulation/send.
//   Both input layouts are the same element (send · IME · focus guard stay one set — do not split).
//   A resumed incarnation draws loading instead (ADR-0226): when 'live' announces an incarnation that came up to resume a saved conversation, a loading panel covers the
//   conversation area until history that draws rows arrives · the user sends · or the agent becomes absent. The bottom-layout input stays active — the way out if history never comes.
//
// Backend notation (title · marker · color tone; richBranding.ts) is a display value derived from the profile — no setting or command; observe it via `data-rich-brand`.
//   Do not pick the renderer with it: capabilities (api/types.ts) decide which slot to show.
//
// Layer separation: parsing/accumulation = structuredAccumulator.ts (pure TS), rendering = StructuredTextView; this component is only the I/O wiring
// (subscribe → feed → render + capture input; §5 hands/brain separation).

import { useEffect, useRef, useState, type KeyboardEvent } from 'react'

import { agentClient } from '../../api/clientFactory'
import { FRAME_TAG_STRUCTURED_EVENT } from '../../api/wsFrame'
import type { OutputSubscription, ViewPhase } from '../../api/agentClient'
import { fireAndForget } from '../../commands/dispatch'
import { useAgentStore } from '../../store/agentStore'
import { useToolGroupStore } from '../../store/toolGroupStore'
import { pendingInterrupt, useInterruptStore } from '../../store/interruptStore'
import { StructuredEventAccumulator, type StructuredItem } from './structuredAccumulator'
import type { QueuedEntry } from './queuedInputReducer'
import { QueuedInputList } from './QueuedInputList'
import { isRenderedItem, StructuredTextView } from './StructuredTextView'
import { WaitStrip } from './chat/WaitRow'
import { richBranding } from './richBranding'
import { SlotUnavailableVeil } from './SlotUnavailableVeil'
import { ESC_SCOPE, isInterruptEscape, OVERLAY_SELECTOR } from './interruptKey'
import './richBranding.css' // Where the tone class is defined (the convention that a component imports the css next to it).
import { ScrollArea } from '../ui/scroll-area' // ADR-0053: the app-wide Radix overlay scrollbar seam
import { REPIN_ON_SEND, useScrollFollow } from './scrollFollow/useScrollFollow'
import { JumpToBottom } from './scrollFollow/JumpToBottom'
import { LoadingPanel } from '../ui/LoadingPanel'
import { basename } from '../../util/basename'
import { t } from '../../i18n'

interface RichSlotProps {
  /** Subscription key (ADR-0046) = slot id; two slots with the same agentId progress independently. */
  viewId?: string
  agentId: string
}

export default function RichSlot({ viewId, agentId }: RichSlotProps) {
  const vid = viewId ?? agentId
  // ★key remount on identity (viewId·agentId) change★: resetting state in the subscription effect would draw one commit with new props + old state
  //   (passive effects run after paint) — a different agent in the same slot would flash the previous agent's conversation. Cost: the typed draft is lost (accepted).
  // ★The incarnation (epoch) is not in the key★: a re-spawn of the same agent must not replace the instance — it would erase the conversation before replay
  //   arrives and the daemon has no history to revive. The clearing callback (onReset) below does that job while keeping the instance.
  return <LiveRichSlot key={`${vid}|${agentId}`} viewId={vid} agentId={agentId} />
}

// ══════════════════════════════════════════════════════════════════════════════════
// ② Live mode — live stream subscription + accumulation + input box
// ══════════════════════════════════════════════════════════════════════════════════

function LiveRichSlot({ viewId, agentId }: { viewId: string; agentId: string }) {
  // Order-preserving render item stream — the accumulator snapshot as is (ADR-0045).
  const [items, setItems] = useState<StructuredItem[]>([])
  // ADR-0231: queued-input list above the input box — a copy of the accumulator's `snapshotQueued()` (after the draw filter too).
  const [queued, setQueued] = useState<readonly QueuedEntry[]>([])
  const [turnDone, setTurnDone] = useState(false)
  // Local awaiting flag (FIX 5b): fills the gap between a send and the first response byte (turnDone only changes on turn-end signals, so a send during an idle turn would
  //   look 'idle'). Cleared when a frame the accumulator says may release the wait arrives; display then follows turnDone.
  const [awaiting, setAwaiting] = useState(false)
  const [input, setInput] = useState('')
  const inputRef = useRef<HTMLTextAreaElement>(null)
  // ADR-0145: did we receive the signal ('live') that history restore is finished? The gate of the empty-state display; lowered on every resubscribe.
  const [replayDone, setReplayDone] = useState(false)
  // ADR-0226: did the last 'live' announce that this incarnation came up to resume a saved conversation (not whether it succeeded). Lowered in onReset and the effect reset.
  const [continuesConversation, setContinuesConversation] = useState(false)
  // The phase the subscription last announced — the third basis of the absent curtain below (ADR-0148).
  const [phase, setPhase] = useState<ViewPhase | null>(null)
  // ADR-0145: a send actually went out on this instance; reverted in send's catch if the send fails (a slot where nothing went out returns to the first-run screen).
  //   ★Not derived from awaiting★: awaiting is also cleared by frames that create no item (empty delta · uuid dedup skip · empty MessageDone), which would bring the mascot back mid-response.
  const [hasSent, setHasSent] = useState(false)
  // The resubscribe effect resets it (replay discipline).
  const accRef = useRef<StructuredEventAccumulator>(null as unknown as StructuredEventAccumulator)
  if (accRef.current === null) accRef.current = new StructuredEventAccumulator()
  const follow = useScrollFollow(viewId)
  // Send sequence: an outcome callback acts only if it belongs to the latest send, so a belated outcome neither erases the latest send's indicator nor fabricates a send fact
  //   for a new incarnation. onReset bumps it to invalidate sends in flight (a ref: not used for display).
  const sendSeqRef = useRef(0)
  // Has any send gone out successfully from this instance? Then a later failed send does not revert to the first-run screen.
  const sendOkRef = useRef(false)

  const agents = useAgentStore((s) => s.agents)
  const agent = agents.find((a) => a.id === agentId) ?? null
  // ADR-0148: did we receive the authoritative registry? — a guard so the empty registry before receipt (right after boot/reconnect) is not mistaken for "gone".
  const agentsLoaded = useAgentStore((s) => s.agentsLoaded)
  // Absent = terminal state ∪ unresolved after the registry is received (ADR-0148). The latter is the real outcome of a kill (the reaper removes the session from the registry,
  //   so status does not show it); without it a dead agent's input box would stay active and could send.
  const agentGone =
    (agentsLoaded && agent == null) ||
    (agent != null &&
      (agent.status.type === 'Exited' || agent.status.type === 'Killed' || agent.status.type === 'Failed'))

  // ADR-0148 / ADR-0149: a lost connection is treated like an absent agent. Only reads the existing connection state (agentClient), creating no new handle (same as ConnectionNotice);
  //   the subscription callback fires once with the current state, so no initial sync.
  const [connected, setConnected] = useState(() => agentClient.connectionState === 'connected')
  useEffect(() => agentClient.onConnectionStateChange((s) => setConnected(s === 'connected')), [])
  // Subscription phase too: socket up but agent not in the registry ('detached') or the re-request ladder exhausted ('error') — connection state alone would call these normal.
  const subscriptionDown = phase === 'detached' || phase === 'error'
  const agentUnavailable = agentGone || !connected || subscriptionDown

  // Identity label: json mode has no "which agent" signal (the terminal has the welcome banner). The name is shown on the right of WaitStrip, same rule as the tree (displayNameOf).
  const profiles = useAgentStore((s) => s.profiles)
  // profiles is always an array in the real store; some test mocks omit it → optional.
  const profile = profiles?.find((p) => p.id === agentId) ?? null
  const cwd = agent?.cwd ?? profile?.cwd ?? ''
  // basename only: agent.name is the full cwd passed to createClaudeProfile; like the tree (displayNameOf), use basename(cwd) without a display_name override.
  const headerName = profile?.display_name ?? basename(cwd)
  // Backend notation (title·marker·color tone) from the profile's run-command kind; no profile → the claude notation (regression-free default, see richBranding.ts).
  //   Renderer selection is still by capabilities. `command` is read optionally: some mocks omit it, and a missing field must not kill the slot.
  const branding = richBranding(profile?.command?.kind)

  // Output subscription — same discipline as TerminalSlot: seq dedup (component defense; the client also dedups), exact unsubscribe.
  useEffect(() => {
    const acc = accRef.current
    acc.reset() // History replay reconstructs the same state (also prevents StrictMode duplicates)
    setItems([])
    setQueued([])
    setTurnDone(false)
    setAwaiting(false) // Prevents a stale streaming hint
    // ADR-0145: lower the restore-complete indicator too — otherwise the previous session's state brings back the flicker (empty state → conversation).
    // Identity change is handled by key remount; this reset matters only when the effect re-runs on the same instance (StrictMode double mount; the accumulator ref survives) —
    //   the last line of defense if key goes away. A re-spawn of the same agent does not come here — the clearing callback (onReset) receives it.
    setReplayDone(false)
    setContinuesConversation(false)
    setHasSent(false)
    setPhase(null) // The phase of a new subscription is set again by that subscription's notification.
    follow.pin() // ADR-0242: the history that arrives after clearing lands at the bottom.
    // ★The tool bundle's chosen expansion is not cleared here★ (ADR-0239): same-incarnation replay rebuilds the same bundle keys. Only onReset clears it.

    let sub: OutputSubscription | null = null
    let cancelled = false
    const lastSeq = { current: -1 }
    // ADR-0244: past this incarnation's first 'live'? Earlier frames are history this view builds for the first time (filtered from the turn-end measurement below).
    let caughtUp = false

    // While the list stays empty it keeps the same reference — almost every frame is this case, so re-renders are not increased.
    const refreshQueued = (): void => {
      const next = acc.snapshotQueued()
      setQueued((prev) => (prev.length === 0 && next.length === 0 ? prev : next))
    }

    // Re-attach check (ADR-0231 · TRD §5-7): after attaching mid-long-turn past the ring cap, `Queued` may be pushed out of the ring and waiting items vanish from the list — each `'live'` asks the daemon registry once.
    //   A late reply is matched with live events by snapshot seq (accumulator) and dropped by generation on leaving this replay cycle.
    const reconcileQueued = (epoch: number): void => {
      const gen = acc.beginQueuedReconcile(epoch)
      agentClient.listQueuedInputs(agentId).then(
        (listing) => {
          if (cancelled) return
          if (acc.offerQueuedSnapshot(gen, listing) === 'applied') refreshQueued()
        },
        // Sleeping agent (NOT_FOUND) · disconnect — the list is left as the ring gave it. Only the record stops.
        () => {
          if (!cancelled) acc.abandonQueuedReconcile(gen)
        },
      )
    }

    agentClient
      .subscribeOutput(
        viewId,
        agentId,
        (chunk) => {
          if (cancelled) return
          if (chunk.seq <= lastSeq.current) return
          lastSeq.current = chunk.seq
          // ★Tag gate (ADR-0045)★: this slot renders only structured (tag1); tag0 (terminal raw bytes, possibly streamed transitionally) is ignored.
          //   seq was already advanced above (one seq space) and is also told to the accumulator — a held check reply may wait for it.
          if (chunk.tag !== FRAME_TAG_STRUCTURED_EVENT) {
            if (acc.observeSeq(chunk.seq)) refreshQueued()
            return
          }
          // tag1 payload = one StructuredEvent JSON.
          const boundariesBefore = acc.turnBoundaryCount()
          const releasesAwaiting = acc.feed(chunk.bytes, chunk.seq)
          // ADR-0244: a turn boundary frame (MessageDone · TurnEnd, any outcome) lifts 「interrupting」. Measured on the frame, not on the render (`streaming`):
          //   if an end event and the next turn's first frame batch into one render, `streaming` never draws false and the next turn's Esc is ignored.
          //   ★Do not measure by the accumulator's not-end → end transition★: a turn interrupted right after a send (accumulator still reading the previous end) has none.
          // ★Frames before the first 'live' are not counted★: a new view's empty accumulator rebuilds history through every old turn end, lifting early the indicator
          //   of a turn being interrupted from another view (a second Esc would send a second interrupt). Later same-incarnation re-buffering frames may count — they passed the seq filter, so are new to this view.
          if (caughtUp && acc.turnBoundaryCount() !== boundariesBefore) useInterruptStore.getState().end(agentId)
          // New reference (the accumulator mutates its array in place) to guarantee a re-render. Drawn regardless of the return value — a frame that does not release the wait may still have changed an earlier row.
          setItems([...acc.snapshot()])
          refreshQueued()
          setTurnDone(acc.isTurnDone())
          // Hand display to turnDone only for frames `feed` says may release. `false` = ① not understood (unknown kind · malformed JSON): turnDone is not updated,
          //   so releasing awaiting would use the previous turn's stale turnDone (true from the second turn) and the wait indicator would go off mid-response.
          //   ② a tool end result (`ToolResult`): understood but not a response — codex keeps a running command alive after interrupt, so a late end landing right after a send
          //   would turn off the new turn's wait indicator with no reply (ADR-0241).
          if (releasesAwaiting) setAwaiting(false)
        },
        // ADR-0145: replay phase callback — 'live' = the client consumed the daemon's end-of-restore marker and emptied the buffer (protocolClient.flushToLive); a new agent
        //   with 0 history gets it too, so "restore end + 0 items" is exact. Lowered on 'buffering' and 'error' — spans where the empty state must not draw.
        (state, info) => {
          if (cancelled) return
          setReplayDone(state === 'live')
          setPhase(state)
          if (state === 'live') {
            // ADR-0244: first catch-up with no open turn = the turn we meant to interrupt already ended only in pre-catch-up history (view attached after the interrupting view vanished ·
            //   command during first buffering), invisible to the measurement above and the wait indicator. ★With an open turn, do not lift★ — that is the turn being interrupted from another view.
            if (!caughtUp && acc.isTurnDone()) useInterruptStore.getState().end(agentId)
            caughtUp = true
            setContinuesConversation(info?.continuesConversation ?? false)
            // No marker → cannot tell which incarnation the reply belongs to, so no check (list as the ring gave it).
            if (info?.epoch !== undefined) reconcileQueued(info.epoch)
          } else {
            // Left live — a new replay cycle; a check in progress belongs to the old cycle and is dropped (generation).
            //   `startBuffering` from hold overflow also comes here ('buffering') — same incarnation, so onReset does not.
            acc.abandonQueuedReconcile()
          }
        },
        // Clearing duty and mandatory onReset pass-through: same as TerminalSlot (here the accumulator is rolled back too).
        // ★Send traces are cleared too★: history of another incarnation follows, so an earlier send was received by a gone incarnation and its reply never comes —
        //   awaiting would stick and a 0-item new incarnation would not show the first-run screen. Not called when resuming the same incarnation.
        // ★replayDone is not lowered★: this callback is where the new replay has arrived, 'live' follows in the same tick; the 'buffering' notification before it opens the restoring span (ADR-0145).
        () => {
          if (cancelled) return
          acc.reset()
          setItems([])
          setQueued([])
          setTurnDone(false)
          setAwaiting(false)
          setHasSent(false)
          follow.pin() // ADR-0242: the new incarnation's history lands at the bottom.
          // ADR-0239: clear the chosen expansion too — the new incarnation's `item:<itemId>` bundle keys (numbered from 0 again) would point to different bundles.
          useToolGroupStore.getState().clear(viewId)
          // ADR-0244: the turn we meant to interrupt belonged to the gone incarnation; new history left open in the same tick would keep `streaming` true, so lift here.
          useInterruptStore.getState().end(agentId)
          caughtUp = false // New incarnation's history flows from now; 'live' in the same tick sets it again.
          // ADR-0226: 'live' in the same tick announces again whether the new incarnation is a resume.
          setContinuesConversation(false)
          sendOkRef.current = false
          // ★Invalidate sends in flight by sequence★: the promise of a send to the previous incarnation is alive and its outcome comes later; without the bump a belated success would set
          //   sendOkRef for the new incarnation, and a later real failure could not return to the first-run screen.
          sendSeqRef.current++
          lastSeq.current = -1
        },
      )
      .then((handle) => {
        if (cancelled) {
          handle.unsubscribe()
          return
        }
        sub = handle
      })
      // A subscription failure (just-before kill etc.) is absorbed to prevent an unhandled rejection.
      .catch(() => {})

    return () => {
      cancelled = true
      sub?.unsubscribe()
    }
    // viewId is the subscription key (ADR-0046). The incarnation is not in it (see the key comment above); `follow.pin` is stable for the whole mount (ADR-0242, `useScrollFollow`).
  }, [viewId, agentId])

  // ADR-0239: tells the expansion store this slot's conversation view is mounted — neither the head toggle nor `chat.toolGroup.setExpanded` can write to an unbound slot.
  //   Cleanup releases it (the chosen expansion stays). `getState` avoids subscribing; `ToolGroupRow` reads the expansion.
  useEffect(() => useToolGroupStore.getState().bind(viewId, agentId), [viewId, agentId])

  // ADR-0244: tells that this window has a conversation view of the agent — without one the command does not set 「interrupting」; releasing the last view lifts it (`store/interruptStore.ts`).
  //   Not counted while this view's subscription is stopped (`detached` · `error`): it cannot see the turn end. Lifting shared state directly would erase another healthy view's
  //   indicator and allow a second interrupt before the turn ends; dropping out lets the last-view rule lift it only when no healthy view remains.
  useEffect(
    () => (subscriptionDown ? undefined : useInterruptStore.getState().watch(agentId)),
    [agentId, subscriptionDown],
  )
  const interrupting = useInterruptStore((s) => pendingInterrupt(s, agentId) !== undefined)

  // ADR-0242 · TRD S21-chat-ux §4-5: expanding a non-last bundle releases follow — staying pinned would scroll down by the expanded height and push the pressed head off screen.
  const onGroupToggle = (isLast: boolean): void => {
    if (!isLast) follow.unpin()
  }

  const send = (): void => {
    // 1 send == 1 completed user turn (ADR-0044): the whole text is sent at once; the backend encoder wraps it in a claude user JSON line, so raw text bytes only.
    // ★The send gate does not check turnDone — intended★ (ADR-0044 mechanism A: mid-turn guidance injection while streaming). Do not add a turn lock.
    // Blocking condition = the curtain's (agentUnavailable): the curtain is pointer-events-none and `disabled` cannot stop programmatic key events, so exit alone would let a
    //   send from a disconnected slot fail, shaking only the wait indicator while the user sees no response.
    if (!input.trim() || agentUnavailable) return
    // FIX 5a: the guard decides by trim, so the actual send is trim-consistent.
    const text = input.trim()
    setInput('')
    if (REPIN_ON_SEND) follow.pin() // ADR-0242 · U4: keeps the reply to sent text from flowing off screen.
    setAwaiting(true) // FIX 5b: switch to the streaming hint at once without waiting for the first byte
    setHasSent(true) // ADR-0145: a conversation has started, so the first-run screen is taken down (reverted below on failure)
    const token = ++sendSeqRef.current
    // ★Revert on write failure★: if writeStdin rejects no response event comes — awaiting would stick the 'streaming' indicator and hasSent would keep a nothing-sent slot off the first-run screen.
    // ★Only the latest send's failure reverts★: after sending A→B a belated A rejection would erase B's wait indicator. An outcome after the incarnation changes is absorbed by the same
    //   sequence guard (onReset bumps it; a re-spawn keeps the instance, so that outcome lands on this instance).
    void agentClient
      .writeStdin(agentId, new TextEncoder().encode(text))
      .then(() => {
        // Success passes the same sequence guard as failure: a success after clearing belongs to the vanished incarnation (guarding only one of the two leaves a window).
        if (sendSeqRef.current !== token) return
        sendOkRef.current = true
      })
      .catch((err) => {
        console.warn('[RichSlot] writeStdin failed — clearing awaiting:', err)
        if (sendSeqRef.current !== token) return
        setAwaiting(false)
        // An earlier successful send's response may be on its way — do not revert to the first-run screen.
        if (!sendOkRef.current) setHasSent(false)
      })
  }

  // Has at least one history row arrived? — first term ending the resume loading (ADR-0226).
  //   ★Not items.length★: a resume's first live frame is usage, which draws no row. Turn dividers (separator) do not count either: closeTurn attaches an empty 12px spacer even
  //   when the preceding item is only usage. ★Do not drop all of 'boundary'★ — the user bubble is also 'boundary' (StructuredTextView rowKindOf).
  const hasHistoryRow = items.some((item) => isRenderedItem(item) && item.kind !== 'separator')

  // ADR-0145: empty state = restore-complete + 0 items + nothing sent on this subscription (0 items alone would flash the intro for sessions with history; hasSent covers the span after the first send before items fill).
  //   ADR-0226: a resumed incarnation may still receive history at restore end + 0 items, so it draws the loading panel instead.
  //   ADR-0231: queued input from another window or an LLM is not the first screen either.
  const hasQueued = queued.length > 0
  const showEmpty =
    replayDone && !continuesConversation && !hasSent && items.length === 0 && !hasQueued

  // ADR-0226: a resumed incarnation has not yet received history (independent of phase). Ends on the first row (hasHistoryRow — history or a delivered bubble) or on restart (new incarnation's clearing + 'live' false).
  // ADR-0231: ★do not use input (hasSent) to end it★ — text typed while connecting stands in the queued list, not as a row; lifting by input would show an empty panel before history.
  const historyPending = continuesConversation && !hasHistoryRow
  // Draw the wait as a loading panel only when restore ended and there is no curtain (the absent curtain wins). ★No timer (ADR-0038)★ — if history never comes, this is the way out for input.
  const awaitingHistory = replayDone && historyPending && !agentUnavailable

  // FIX 5: initial turnDone=false with empty items would show the streaming badge on a fresh slot, so require items.length>0; the real streaming signal and `awaiting` are kept.
  //   ADR-0226: no wait indicator during history wait (it would overlap the loading panel). ★Gated by historyPending, not awaitingHistory★ — the panel briefly drops during 'buffering'
  //   of a same-incarnation reattach or the absent curtain, which would flicker panel → indicator → panel.
  //   ★The awaiting term stays outside the gate★: text typed while connecting does not lift the loading, and where no queued-list event comes (before the decision / unused path)
  //   this indicator is the only sign that sent text is pending.
  const streaming = awaiting || (!turnDone && items.length > 0 && !historyPending)

  // ADR-0244: lift 「interrupting」 when the wait indicator goes off with no turn boundary — sent text stood in the queued list without opening a turn. Only on on → off (false at mount is not an end).
  const wasStreamingRef = useRef(streaming)
  useEffect(() => {
    if (wasStreamingRef.current && !streaming) useInterruptStore.getState().end(agentId)
    wasStreamingRef.current = streaming
  }, [streaming, agentId])
  // ADR-0244: agent gone or connection lost — no view can see the turn end, so this window's interrupt command would send nothing and answer 「already sent」. Lifts even on a brief disconnect
  //   (an Esc after reattaching interrupts again). This view's own subscription stop is handled by dropping out of `watch` above.
  const agentOrConnectionDown = agentGone || !connected
  useEffect(() => {
    if (agentOrConnectionDown && interrupting) useInterruptStore.getState().end(agentId)
  }, [agentOrConnectionDown, interrupting, agentId])

  // ADR-0237: a bare Esc anywhere in the area = interrupt the running turn. Captured because the input box onKeyDown cuts propagation first. It does not change the optimistic conversation state
  //   (awaiting · turnDone; the turn-end event announces it) or the input text. 「interrupting」 is set by the command (ADR-0244 — same path for the human key and the LLM).
  //   An Esc before a turn opens is refused by the path and fireAndForget swallows it as a warning — press again.
  // The capability is read optionally — unreadable means do not interrupt (same reason as `command` above).
  const canInterrupt = agent?.capabilities?.control?.interrupt === true
  const onRootKeyDownCapture = (e: KeyboardEvent<HTMLDivElement>): void => {
    // Filter Esc before measuring the overlay so the document is not scanned on every keystroke.
    if (e.key !== 'Escape') return
    const fire = isInterruptEscape(e, {
      streaming,
      interrupting,
      agentUnavailable,
      canInterrupt,
      overlayOpen: document.querySelector(OVERLAY_SELECTOR) !== null,
      scope: ESC_SCOPE,
      textarea: inputRef.current,
    })
    if (!fire) return
    e.preventDefault()
    fireAndForget('agent.interrupt', { agentId })
  }

  return (
    <div
      // Empty state: the root is the alignment container (justify-center), centering the [mascot·text·input box] group as a whole.
      // relative = anchor of the absent overlay (absolute inset-0). The tone comes as one class (richBranding.css; claude is '').
      // tabIndex -1: the root receives focus by click · code only (outside tab order), so Esc reaches this area from a body-margin press (ADR-0237); conversation text focuses the scroll viewport first, but keydown still bubbles here.
      tabIndex={-1}
      onKeyDownCapture={onRootKeyDownCapture}
      className={[
        'relative flex h-full w-full flex-col bg-background outline-none',
        branding.tintClass,
        showEmpty ? 'justify-center' : '',
      ]
        .filter(Boolean)
        .join(' ')}
      data-rich-live="1" // For checking in cdp eval whether the live RichSlot is mounted
      data-agent-id={agentId}
      // Same token is on the marker too — keep it here: when a conversation starts the marker folds away and this is the only DOM telling the backend (ADR-0145).
      data-rich-brand={branding.brand}
      data-rich-awaiting-history={awaitingHistory ? '1' : undefined}
    >
      {/* Conversation render (scroll) — ScrollArea seam (ADR-0053). The seam forwards the ref to the real scroll node (Radix Viewport); scroll-follow (ADR-0242) measures and writes it. */}
      {!showEmpty && (
        <ScrollArea ref={follow.viewportRef} className="min-h-0 flex-1">
          <StructuredTextView
            items={items}
            streaming={streaming}
            slotId={viewId}
            onGroupToggle={onGroupToggle}
          />
          {/* ADR-0226 history wait — centered icon + pale curtain. Placed here so the absolute reference is the ScrollArea root: covers only the conversation area, does not scroll, does not reach the input box.
              Relies on the Radix Viewport and its inner wrapper having no position (checked in the @radix-ui/react-scroll-area dist) — if the wrapper becomes positioned the curtain shrinks to content height.
              pointer-events-none: scroll, selection and clicks pass through. Pale foreground tint, not background: the covered area is always empty, so a background-family color would be invisible. */}
          {awaitingHistory && (
            <LoadingPanel className="pointer-events-none absolute inset-0 bg-foreground/8" />
          )}
          <JumpToBottom follow={follow} />
        </ScrollArea>
      )}
      {/* Wait indicator row — outside the scroll list so toggling does not shake conversation height or bottom-follow (WaitRow header).
          `{cond && …}` pins one child position so the input group below does not shift (textarea comment).
          The identity label is this row's right-hand area (name only, duplicate names allowed; status glyph is the tree's job). It shares the line with the wait text and is cut off when narrow —
          floating it over the input group (absolute) would cover 「interrupting…」 and the elapsed seconds. The empty state has no row, so no label (the product name goes just above the input box, ADR-0145). */}
      {!showEmpty && (
        <WaitStrip
          streaming={streaming}
          interrupting={interrupting}
          label={
            <div
              data-rich-label="1"
              title={headerName}
              className="pointer-events-none min-w-0 truncate rounded border border-border bg-surface px-1.5 py-0.5 text-[11px] text-muted"
            >
              {headerName}
            </div>
          }
        />
      )}

      {/* ADR-0145 empty-state upper part — marker + product name; the root's justify-center centers it, this takes only its own height. Product name text is not a translation target (ADR-0145).
          Only the mascot shrinks when space shrinks (shrink + overflow-hidden), so it is cut off naturally while the text and input box stay whole (flex-none); a threshold-height hide-all was rejected as abrupt. */}
      {showEmpty && (
        <div data-rich-empty="1" className="flex min-h-0 flex-col items-center px-4 pb-8">
          <div className="min-h-0 shrink overflow-hidden">
            <branding.Mascot />
          </div>
          <div className="mt-3 flex-none text-[20px] font-semibold tracking-tight text-foreground">
            {branding.title}
          </div>
        </div>
      )}

      {/* Input group = [queued-input list · input box] (ADR-0231), the list above the input box.
          The textarea is the same element in both layouts (ADR-0145): fixed position (parent children index), only className·rows swap, so React does not remount — splitting would make
          send · IME · focus guard two sets and break the focus being typed in at the transition. The list likewise holds one child position via `{cond && …}`. */}
      <div className={showEmpty ? 'flex flex-none flex-col' : 'flex flex-none flex-col border-t border-border'}>
        {hasQueued && <QueuedInputList agentId={agentId} entries={queued} />}
        {/* Input box — Enter sends / Shift+Enter newline. Focus guard: stopPropagation keeps keys from leaking to parent/global keybindings (same as the terminal slot's onData capture). */}
        <div
          className={
            showEmpty
              ? 'flex flex-none items-stretch justify-center px-4'
              : 'flex flex-none items-stretch px-2 py-1.5'
          }
        >
          <textarea
            ref={inputRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              e.stopPropagation()
              // Enter that confirms Korean IME composition arrives as keydown with isComposing=true (keyCode 229) in WebView2; handling it as a send would send unfinished input —
              //   let it through before the send branch.
              if (e.nativeEvent.isComposing || e.keyCode === 229) return
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault()
                send()
              }
            }}
            // Priority = ended > empty state > bottom. Disabled is wider than this; the text speaks only of exit — "ended" for disconnect/subscription stop would assert something in a span
            //   where the decision wavers (same reason the curtain has no text, SlotUnavailableVeil header).
            placeholder={
              agentGone
                ? t('agent.terminatedPlaceholder')
                : showEmpty
                  ? t('agent.emptyInputPlaceholder')
                  : t('agent.inputPlaceholder')
            }
            disabled={agentUnavailable}
            rows={showEmpty ? 3 : 2}
            // Only the empty state is rounded and larger (ADR-0145); the bottom layout is the existing high-contrast bar, side margins aligned with ChatRow px-4.
            className={
              showEmpty
                ? 'w-full max-w-[560px] resize-none rounded-xl border border-border bg-surface px-5 py-4 text-[14px] text-foreground outline-none placeholder:text-muted focus:border-accent disabled:opacity-50'
                : 'flex-1 resize-none rounded border border-border bg-surface px-3 py-1.5 text-[13px] text-foreground outline-none placeholder:text-muted focus:border-accent disabled:opacity-50'
            }
          />
        </div>
      </div>

      {/* ADR-0148: target agent not there (exit · connection lost · subscription cannot produce output) kills the slot; look shared by the three slots (SlotUnavailableVeil).
          Last child so earlier children's indexes do not shift and the input box is not remounted (textarea comment). */}
      {agentUnavailable && <SlotUnavailableVeil phase={phase} />}
    </div>
  )
}
