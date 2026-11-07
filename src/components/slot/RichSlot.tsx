// RichSlot — structured (JSON mode) live render slot (ADR-0044/0045).
//
//  Live mode (<RichSlot viewId agentId />) — receives the backend-refined **structured output tag1** frames with the same subscription discipline as TerminalSlot,
//  accumulates them in StructuredEventAccumulator and draws them + a text input box (Enter=send,
//  Shift+Enter=newline). There are two input box layouts — pinned to the bottom when there is a conversation, centered on screen when empty (ADR-0145 below).
//
// ★M0 fixture spike removed (Brick 1)★: FixtureRichSlot (<RichSlot />, no agentId), which parsed a stream-json sample whole
//   and drew it without a live agent/daemon, and its lab/richslot dependency were removed along with the legacy frontend
//   layout cleanup. Style tuning is done in a lab entry (separate), and this component owns only the live path.
//
// ★S15 source switch (ADR-0045)★: the live accumulation source changed from the S14 NDJSON byte parser to tag1
//   StructuredEvent (StructuredEventAccumulator). The backend refines the output and streams self-describing
//   events, so the frontend does no line reassembly and consumes one event at a time. If tag0 (terminal bytes) arrives at
//   this slot it is ignored (a structured slot, so not a render target — tag gate).
//
// ★Empty state (ADR-0145)★: only when restore-complete signal ('live') + 0 items + nothing sent does it draw the marker, product name and centered
//   input box. It touches only the decision and leaves the subscription, accumulation and send paths alone. The two layouts of the input box are
//   the same element (keep send · IME · focus guard as one set — do not split them).
//   ★A resumed incarnation draws loading instead of the empty state (ADR-0226)★: if 'live' announces "an incarnation that came up to resume a saved
//   conversation", a loading panel goes on the conversation area until history that draws rows arrives · the user sends · or the agent becomes absent.
//   The input box is active in the bottom layout — it is the way out for input if history never comes.
//
// ★Backend notation★: title · marker · color tone, three of them, tell claude and codex apart (defined in richBranding.ts). The notation is
//   a display value derived from the profile, so it has neither a setting nor a command — observe it via the `data-rich-brand` token.
//   ★Do not pick the renderer with it★: which slot to show is still decided by capabilities (api/types.ts).
//
// ★Layer separation★: parsing/accumulation is owned by pure TS (structuredAccumulator.ts), and rendering by a dedicated component (StructuredTextView).
//   This component does only the pure I/O wiring of "subscribe → feed the accumulator → render the result + capture input"
//   (§5 hands/brain separation: frontend = I/O, control is a handle on the backend side).

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
  /** Subscription key (ADR-0046) = slot id. Two slots with the same agentId have independent progress — bug B resolved. */
  viewId?: string
  agentId: string
}

export default function RichSlot({ viewId, agentId }: RichSlotProps) {
  const vid = viewId ?? agentId
  // ★When identity (viewId·agentId) changes, mount anew — key★
  //   If the list, restore indicator and send indicator are taken down in the subscription effect, that one commit is drawn with **new props + old state**
  //   (passive effects run after paint). Assigning a different agent to the same slot flashes the previous
  //   agent's conversation in that frame. Replacing the instance with key means a commit inheriting old state does not exist at all.
  //   Side effect = loss of the draft being typed. It is accepted because it is a switch to a different agent.
  //   ★The incarnation (epoch) is not put in this key★: a re-spawn of the same agent **does not replace the instance** —
  //   replacing it would erase the conversation before the replay arrives, and the daemon has no history to revive either. What takes that place is the
  //   clearing signal (onReset), and that callback in the subscription below does the same job while keeping the instance.
  return <LiveRichSlot key={`${vid}|${agentId}`} viewId={vid} agentId={agentId} />
}

// ══════════════════════════════════════════════════════════════════════════════════
// ② Live mode — live stream subscription + accumulation + input box
// ══════════════════════════════════════════════════════════════════════════════════

function LiveRichSlot({ viewId, agentId }: { viewId: string; agentId: string }) {
  // Order-preserving render item stream (text/chip/divider) — holds the accumulator snapshot as it is (ADR-0045 §52).
  const [items, setItems] = useState<StructuredItem[]>([])
  // ADR-0231: queued-input list above the input box — a copy of the accumulator's `snapshotQueued()` (after the draw filter too).
  const [queued, setQueued] = useState<readonly QueuedEntry[]>([])
  const [turnDone, setTurnDone] = useState(false)
  // ★Local awaiting flag (FIX 5b)★: fills the gap between right after a send and arrival of the first response byte. turnDone is
  //   set by the accumulator only from turn-end signals (MessageDone · TurnEnd), so a new send while the previous turn is idle
  //   looks 'idle' until the first byte. This flag is set right on send to flip to 'streaming', and when a frame ★that the accumulator
  //   **answered may release the wait**★ arrives it is cleared and later display is handed to turnDone (subscription callback below).
  const [awaiting, setAwaiting] = useState(false)
  const [input, setInput] = useState('')
  const inputRef = useRef<HTMLTextAreaElement>(null)
  // ADR-0145: did we receive the signal ('live') that history restore is finished? The gate of the empty-state display; lowered on every resubscribe.
  const [replayDone, setReplayDone] = useState(false)
  // ADR-0226: did the last 'live' announce "this incarnation came up to resume a saved conversation" (not whether it succeeded).
  //   Set only on 'live', and lowered in the clearing callback and the subscription effect reset.
  const [continuesConversation, setContinuesConversation] = useState(false)
  // The phase the subscription last announced — the third basis of the absent curtain below (ADR-0148).
  const [phase, setPhase] = useState<ViewPhase | null>(null)
  // ★ADR-0145: "already sent on this instance"★ — the fact that a send actually went out. If the send itself fails,
  //   the catch in send below reverts it (a slot where nothing went out must return to the first-run screen).
  //   Why awaiting is not used instead = awaiting is also cleared by tag1 frames that create no item (empty delta · user uuid
  //   dedup skip · empty MessageDone), so in a span where a response is in progress but items is still 0, the mascot
  //   comes back. The empty state only needs to see "did a conversation start", so that fact is held separately (the streaming hint
  //   role of awaiting is unchanged — ADR-0044/0045).
  const [hasSent, setHasSent] = useState(false)
  // The resubscribe effect resets it (replay discipline).
  const accRef = useRef<StructuredEventAccumulator>(null as unknown as StructuredEventAccumulator)
  if (accRef.current === null) accRef.current = new StructuredEventAccumulator()
  const follow = useScrollFollow(viewId)
  // Send sequence — the basis on which an outcome callback (success·failure) decides "am I still the latest send?". Prevents a belated
  //   outcome of an earlier send from erasing the indicator of the latest send in progress or fabricating a send fact for the new incarnation. The clearing
  //   callback (onReset) also bumps it here to invalidate a send in flight (a ref, not state: it is not used for display).
  const sendSeqRef = useRef(0)
  // Has any send gone out successfully from this instance? If so, even if a later send fails it does not
  //   revert to the first-run screen — the response to an earlier send may be on its way.
  const sendOkRef = useRef(false)

  const agents = useAgentStore((s) => s.agents)
  const agent = agents.find((a) => a.id === agentId) ?? null
  // ADR-0148: did we receive the authoritative registry? — a guard so the empty registry before receipt (right after boot/reconnect) is not mistaken for "gone".
  const agentsLoaded = useAgentStore((s) => s.agentsLoaded)
  // ★Absent = found in terminal state ∪ unresolved even after the registry is received (ADR-0148)★. The latter is the real outcome of a kill
  //   — the reaper collects the session and removes it from the registry, so status does not catch it. If it were not seen as absent, the dead
  //   agent's input box would stay active and could attempt a send.
  const agentGone =
    (agentsLoaded && agent == null) ||
    (agent != null &&
      (agent.status.type === 'Exited' || agent.status.type === 'Killed' || agent.status.type === 'Failed'))

  // ADR-0148 decision 1 / ADR-0149 decision 4: a lost connection is also seen the same as "the targeted agent is not there now" (user
  //   decision). It only **reads** the existing connection state surface (agentClient) — it creates no new global handle or store
  //   (same shape as ConnectionNotice). The subscription callback fires once with the current state on registration, so no initial value sync is needed.
  const [connected, setConnected] = useState(() => agentClient.connectionState === 'connected')
  useEffect(() => agentClient.onConnectionStateChange((s) => setConnected(s === 'connected')), [])
  // Also looks at the subscription phase — the span where the socket is up but the registry has no such agent ('detached') and the span where the re-request ladder is
  //   exhausted ('error') fall in here (looking only at connection state reads both as "normal", so a dead slot pretends to be alive).
  const subscriptionDown = phase === 'detached' || phase === 'error'
  const agentUnavailable = agentGone || !connected || subscriptionDown

  // ★Identity label (§ user request)★: json mode has no "which agent is this" signal like the terminal's claude welcome banner.
  //   Only the name is shown as a small label on the right of the wait strip (WaitStrip) above the input box (render below). The display name follows the same rule as the
  //   tree (displayNameOf): display_name override ?? agent.name ?? cwd basename (duplicate names allowed).
  const profiles = useAgentStore((s) => s.profiles)
  // profiles is always an array ([]) in the real store, but some unit test mocks do not fill s.profiles → defensive optional.
  const profile = profiles?.find((p) => p.id === agentId) ?? null
  const cwd = agent?.cwd ?? profile?.cwd ?? ''
  // ★basename only★: agent.name is the profile's name (= the full cwd we passed to createClaudeProfile), so the full path
  //   shows up → as in the tree (displayNameOf), without a display_name override only basename(cwd) is used ("Filter Library").
  const headerName = profile?.display_name ?? basename(cwd)
  // ★Backend notation (title·marker·color tone)★: rich mode drew claude and codex identically, so the screen had no
  //   signal of who you are talking to. The basis of the decision is one thing — the kind of the profile's run command — and with no profile it stays the claude notation
  //   (a regression-free default — reason in richBranding.ts). ★Renderer selection is still done by capabilities★.
  //   ★`command` is also carried as optional★ — it is required by type, but for the same reason as profiles above, mocks that do not fill that field
  //   really exist (if missing, not the notation but the whole slot dies). Falling to the default when it cannot be read is right.
  const branding = richBranding(profile?.command?.kind)

  // Output subscription — mirrors the TerminalSlot discipline: seq dedup (component defense — the client also dedups internally),
  // exact unsubscribe (the stale guard token is owned by the client).
  useEffect(() => {
    const acc = accRef.current
    acc.reset() // History replay reconstructs the same state (also prevents StrictMode duplicates)
    setItems([])
    setQueued([])
    setTurnDone(false)
    setAwaiting(false) // Prevents a stale streaming hint
    // ADR-0145: the restore-complete indicator is lowered here too — if not, it inherits the previous session's complete state and the flicker where the empty state shows
    //   in the restore span with an empty list and then flips to the conversation comes back.
    // ★Identity change (viewId·agentId) is handled by key remount★ — the only case where the reset in this place really works is the path where
    //   **the effect runs again on the same instance**, like StrictMode double mount (the accumulator ref survives in between). Still kept —
    //   if key goes away this is the only line of defense.
    //   ★A re-spawn of the same agent does not come here★ — that is received by the clearing callback (onReset) below.
    setReplayDone(false)
    setContinuesConversation(false)
    setHasSent(false)
    setPhase(null) // The phase of a new subscription is set again by that subscription's notification.
    follow.pin() // ADR-0242: the history that arrives after clearing lands at the bottom.
    // ★The tool bundle's chosen expansion is not cleared here★ (ADR-0239) — the replay of the same incarnation is rebuilt with the same bundle keys, so
    //   the chosen value must attach again as is. The only place that clears is the clearing callback (onReset) below.

    let sub: OutputSubscription | null = null
    let cancelled = false
    const lastSeq = { current: -1 }
    // ADR-0244: have we passed this incarnation's first 'live'? — frames before it are history this view builds for the first time (the turn-end measurement below filters them).
    let caughtUp = false

    // While the list stays empty it keeps the same reference — almost every frame is this case, so re-renders are not increased.
    const refreshQueued = (): void => {
      const next = acc.snapshotQueued()
      setQueued((prev) => (prev.length === 0 && next.length === 0 ? prev : next))
    }

    // ★Re-attach check (ADR-0231 · TRD §5-7)★: when reattaching in a long turn past the ring cap, `Queued` is pushed out of the ring and
    //   items still waiting vanish from this window's list — on every `'live'` the daemon registry is asked once to reconcile.
    //   Even if the reply is late it is matched with live events by the snapshot seq (accumulator), and on leaving this replay cycle it is dropped by generation.
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
          // ★tag gate (S15/ADR-0045)★: this slot renders only structured (tag1). When tag0 (terminal raw bytes) arrives it is
          //   ignored — even a structured agent may have the backend stream tag0 (transitional), and parsing the bytes here instead of xterm
          //   breaks them. seq was already advanced above (one seq space regardless of tag), so dedup stays consistent even
          //   when tag0 is skipped. The seq is also told to the accumulator — a held check reply may be waiting for that seq.
          if (chunk.tag !== FRAME_TAG_STRUCTURED_EVENT) {
            if (acc.observeSeq(chunk.seq)) refreshQueued()
            return
          }
          // tag1 payload = one StructuredEvent JSON.
          const boundariesBefore = acc.turnBoundaryCount()
          const releasesAwaiting = acc.feed(chunk.bytes, chunk.seq)
          // ADR-0244: this frame is a turn boundary (MessageDone · TurnEnd — whatever the outcome) — lift 「중단하는 중」 (interrupting). ★The reason it is measured on the frame, not on
          //   the render (`streaming`), is that if an end event and the next turn's first frame are batched into one render, `streaming` is never drawn false★
          //   — then the indicator stays until the next turn and that turn's Esc is ignored. ★Do not measure it by the accumulator's not-end → end transition★
          //   — the end of a turn interrupted right after a send (the accumulator still reading the previous turn's end) has no such transition.
          // ★Frames before the first 'live' are not counted★ — when a new view of the same agent attaches, an empty accumulator rebuilds the history and goes to end at
          //   every old turn end, lifting early the indicator of a turn being interrupted from another view (then an Esc pressed again sends a second interrupt).
          //   Frames that a later same-incarnation re-buffering streams may be counted — what passed the seq filter above is a frame this view sees for the first time.
          if (caughtUp && acc.turnBoundaryCount() !== boundariesBefore) useInterruptStore.getState().end(agentId)
          // Set with a new reference (the accumulator updates its internal array in place, so a new outer array reference is taken to guarantee a re-render).
          //   Drawn first regardless of the return value — a frame that does not release the wait may also have changed an earlier row (② below).
          setItems([...acc.snapshot()])
          refreshQueued()
          setTurnDone(acc.isTurnDone())
          // ★Display control is handed to turnDone only for frames `feed` answered may release★. `false` has two cases —
          //   ① a frame that was not understood (unknown kind · malformed JSON) cannot update turnDone, so releasing awaiting then
          //   decides the display by the **previous turn's** stale turnDone — from the second turn that value is true, so the wait indicator goes off
          //   while the response is running (a defect that a test seeing only the first turn could not see).
          //   ② A tool end result (`ToolResult`) is understood but is not a response — codex does not kill a running command even when interrupted, so
          //   the end of a previous turn's tool arrives late, and if it lands right after a send · before the first reply, the new turn's wait indicator goes off with no reply (ADR-0241).
          if (releasesAwaiting) setAwaiting(false)
        },
        // ADR-0145: replay phase callback — 'live' is the point where the client consumed the marker the daemon put at the end of restore and emptied the buffer
        //   (protocolClient.flushToLive). A new agent with 0 history items also gets the same signal, so
        //   "restore end + 0 items" is told apart exactly. It is lowered on 'buffering' (restoring) and 'error' (replay re-request exhausted)
        //   — both are spans where the empty state must not be drawn (flicker during restore / on failure the current blank screen is kept).
        (state, info) => {
          if (cancelled) return
          setReplayDone(state === 'live')
          setPhase(state)
          if (state === 'live') {
            // ADR-0244: caught up for the first time and there is no open turn = the turn we meant to interrupt already ended — if that end is only in history before the catch-up
            //   (a view that attached after the interrupting view vanished · a command called during the first buffering), neither the turn-end measurement above nor the wait indicator going off can see it.
            //   ★If it ends with an open turn, do not lift★ — the turn being interrupted from another view is exactly that open turn.
            if (!caughtUp && acc.isTurnDone()) useInterruptStore.getState().end(agentId)
            caughtUp = true
            setContinuesConversation(info?.continuesConversation ?? false)
            // With no marker we cannot tell which incarnation the reply belongs to, so no check is made (the list as the ring gave it — same as today).
            if (info?.epoch !== undefined) reconcileQueued(info.epoch)
          } else {
            // Left live — this is a new replay cycle. A check in progress belongs to the old cycle and is dropped (check generation).
            //   ★`startBuffering` from hold overflow also comes here ('buffering')★ — it is the same incarnation, so clearing (onReset) does not come.
            acc.abandonQueuedReconcile()
          }
        },
        // The reason for the clearing duty and the mandatory onReset pass-through is the same as TerminalSlot (here it rolls back the accumulator too).
        // ★Send traces are taken down too★: this callback means history of **another incarnation** is coming from now on, so a
        //   send that went out earlier was received by an incarnation that is already gone and its reply never comes. If left, the wait indicator (awaiting)
        //   sticks, and the new incarnation with 0 history items does not show the first-run screen (mascot), looking like "empty slot but in conversation".
        //   ★This callback does not come on resuming the same incarnation★ — the wait indicator in that span stays as it is.
        // ★replayDone is not lowered★: this callback is the place where the new incarnation's replay has **arrived**, so
        //   'live' follows in the same tick right after. What opens the restoring span is the 'buffering' notification before it (ADR-0145).
        () => {
          if (cancelled) return
          acc.reset()
          setItems([])
          setQueued([])
          setTurnDone(false)
          setAwaiting(false)
          setHasSent(false)
          follow.pin() // ADR-0242: the new incarnation's history lands at the bottom.
          // ADR-0239: the chosen expansion is cleared too — the new incarnation has a different event sequence, so the bundle key of an item number the accumulator numbers from 0 again
          //   (`item:<itemId>`) points to a different bundle than the old selection.
          useToolGroupStore.getState().clear(viewId)
          // ADR-0244: the turn we meant to interrupt belongs to the vanished incarnation. If new history in the same tick leaves a turn open, `streaming` below is never
          //   drawn false, so it is lifted here.
          useInterruptStore.getState().end(agentId)
          caughtUp = false // The new incarnation's history flows from now on — 'live' in the same tick right after sets it again.
          // ADR-0226: whether the new incarnation is a resumed incarnation is announced again by 'live' in the same tick right after.
          setContinuesConversation(false)
          sendOkRef.current = false
          // ★A send in flight is invalidated by the sequence★: for a send made to the previous incarnation, even if the traces are taken down here the promise
          //   itself is alive and its outcome comes later. If the sequence is not bumped, that belated success sets the new incarnation's
          //   sendOkRef ("a send went out from this instance"), and even if a later real send fails, it cannot return to the first-run
          //   screen — the new incarnation that received nothing stays as in-conversation.
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
    // viewId included — it is the subscription key (ADR-0046, two slots with the same agentId are independent). ★The incarnation is not put in — reason in the key comment above.★
    // ★`follow` is not put in either★ — its `pin` is the same function for the whole mount lifetime (ADR-0242 · the head of `useScrollFollow`).
  }, [viewId, agentId])

  // ADR-0239: tells the expansion store that this slot's conversation view is mounted now — neither the head toggle nor
  //   `chat.toolGroup.setExpanded` can write to an unbound slot. Calls the release returned from cleanup (the chosen expansion stays).
  //   Calling via `getState` keeps this component from subscribing to the store — what reads the expansion is `ToolGroupRow`.
  useEffect(() => useToolGroupStore.getState().bind(viewId, agentId), [viewId, agentId])

  // ADR-0244: tells that this window has a conversation view of that agent — for an agent that has none the command does not set 「중단하는 중」 (interrupting), and when the last
  //   view is released it is lifted (head of `store/interruptStore.ts`).
  //   ★It is not counted while this view's subscription is stopped (`detached` · `error`)★ — it is a view that cannot see the turn end. If the shared state were
  //   lifted here directly it would also erase the interrupting indicator of another healthy view of the same agent, and a second interrupt would go out before the turn ends. Just
  //   dropping out lets the last-view rule lift it only when no healthy view remains.
  useEffect(
    () => (subscriptionDown ? undefined : useInterruptStore.getState().watch(agentId)),
    [agentId, subscriptionDown],
  )
  const interrupting = useInterruptStore((s) => pendingInterrupt(s, agentId) !== undefined)

  // ADR-0242 · TRD S21-chat-ux §4-5: when a human expands a non-last bundle, follow is released — if kept pinned, it would go back down to the bottom by the expanded height
  //   and push the pressed head up off the screen. The last bundle leaves the pin as it is.
  const onGroupToggle = (isLast: boolean): void => {
    if (!isLast) follow.unpin()
  }

  const send = (): void => {
    // ★1 send == 1 completed user turn (ADR-0044/0004)★: the whole text is sent at once. The backend encoder wraps it
    //   in a claude user JSON line, so no newline appending or wrapping here (raw text bytes only).
    // ★The send gate does not check turnDone — intended behavior (ADR-0044 mechanism A: allow mid-turn
    //   guidance injection while streaming)★. Do not put a turn lock (blocking sends while streaming) here.
    // ★Blocking condition = curtain-raising condition (agentUnavailable)★: the curtain is pointer-events-none so it does not intercept
    //   operation, and disabled cannot stop programmatic key events either — looking at exit alone lets a send go out from a disconnected slot and
    //   fail, and that failure only shakes the wait indicator while the user sees no response.
    if (!input.trim() || agentUnavailable) return
    // FIX 5a: the guard also decides by trim, so the actual send is trim-consistent too.
    const text = input.trim()
    setInput('')
    if (REPIN_ON_SEND) follow.pin() // ADR-0242 · U4: keeps the reply to sent text from flowing off screen.
    setAwaiting(true) // FIX 5b: switch to the streaming hint at once without waiting for the first byte
    setHasSent(true) // ADR-0145: a conversation has started, so the first-run screen is taken down (reverted below on failure)
    const token = ++sendSeqRef.current
    // ★Revert on write failure★: if the writeStdin promise rejects (the send itself fails), the response event never
    //   comes → awaiting stays set so the 'streaming'/Thinking indicator sticks forever, and hasSent remains so a slot that sent
    //   nothing cannot return to the first-run screen. The failure path reverts both (only the derived display value is corrected —
    //   WIRE unchanged, ADR-0044/45/46).
    // ★But only the failure of the latest send reverts★: this catch does not know by itself which send failed. After sending A→B,
    //   if A rejects belatedly, without a guard it erases the wait indicator of B in progress. A failure that arrives after the incarnation changes
    //   is absorbed by the same sequence guard — the clearing callback bumps the sequence, so any send before it is out of step whether its outcome is
    //   success or failure (a re-spawn does not replace the instance, so that outcome lands on the same instance).
    void agentClient
      .writeStdin(agentId, new TextEncoder().encode(text))
      .then(() => {
        // Success passes the same sequence guard as failure — a success arriving after clearing passed belongs to the vanished incarnation, and
        //   counting it as "a send that went out from this incarnation" is wrong (guarding only one of the two leaves a window between them).
        if (sendSeqRef.current !== token) return
        sendOkRef.current = true
      })
      .catch((err) => {
        console.warn('[RichSlot] writeStdin failed — clearing awaiting:', err)
        if (sendSeqRef.current !== token) return
        setAwaiting(false)
        // If an earlier send succeeded, its response may be on its way, so it does not revert to the first-run screen.
        if (!sendOkRef.current) setHasSent(false)
      })
  }

  // Has at least one history row arrived? — the first term that ends the resume loading (ADR-0226).
  //   ★Do not replace it with items.length★: the first live frame of a resume is usage, which draws no row, and reading it
  //   as "history arrived" would show an empty list before history arrives.
  //   ★Turn dividers (separator) are not counted either★: even when the preceding item is only usage, the turn boundary attaches a divider (accumulator
  //   closeTurn), but it is an empty 12px spacer, not history. Counting it lifts the loading and leaves an empty panel with no first screen.
  //   ★Do not drop all of 'boundary'★ — the user bubble is also 'boundary' (StructuredTextView rowKindOf).
  const hasHistoryRow = items.some((item) => isRenderedItem(item) && item.kind !== 'separator')

  // ADR-0145: empty state = restore-complete signal + 0 items + nothing sent yet on this subscription. Drawing on 0 items alone makes even a session with history
  //   have 0 items before restore ends, so the intro shows and then flips to the conversation (flicker). Why hasSent is looked at too =
  //   the span right after the first send before items fill is already "conversation started", so it is not the empty state.
  //   ADR-0226: a resumed incarnation may have history following even at restore end + 0 items, so it draws the loading below instead of the empty state.
  //   ADR-0231: queued input from another window or an LLM standing there is not the first screen either.
  const hasQueued = queued.length > 0
  const showEmpty =
    replayDone && !continuesConversation && !hasSent && items.length === 0 && !hasQueued

  // ADR-0226: a resumed incarnation has not yet received history — the part independent of phase (restoring · absent). It ends in
  //   two ways: the first row (hasHistoryRow — history or a delivered bubble) · restart (the new incarnation's clearing + 'live' false).
  // ADR-0231: ★Do not revive input (hasSent) as a way to end it★ — text typed while connecting stands in the queued list, not as a row, and
  //   if history comes the history lifts the loading, and if it never comes the bubble delivered after connecting does. Lifting it by input shows an
  //   empty panel before history arrives.
  const historyPending = continuesConversation && !hasHistoryRow
  // The condition to **draw** that wait as a loading panel — only when restore has ended and there is no curtain (if absent, the curtain wins).
  //   ★Do not end it with a timer★ (ADR-0038) — if history never comes, this is the way out for input.
  const awaitingHistory = replayDone && historyPending && !agentUnavailable

  // ★FIX 5★: with initial turnDone=false and empty items (fresh/idle slot), !turnDone alone made the shimmer·streaming
  //   badge show wrongly. Narrowed with the items.length>0 condition to display idle as idle, while (a) the real streaming
  //   signal and (b) 'waiting for the first token right after a send (awaiting)' are kept as they are.
  //   ADR-0226: the wait indicator is taken down during history wait — it would show together with the loading panel in a window that received only usage. In other
  //   windows the rule above holds as is. ★It is taken down by historyPending, not awaitingHistory★: if the wait indicator (with elapsed seconds) gets in a window where only the panel
  //   briefly went down during 'buffering' of a same-incarnation reattach or the absent curtain, it flickers panel → indicator → panel.
  //   ★The awaiting term is kept outside this gate★: text typed while connecting does not lift the loading, so right after a send the two
  //   overlap — when the queued-list event comes awaiting is released and the wait indicator goes down. On a path without that event (a
  //   path that does not use the list · before the decision) this indicator is the only sign of 「보낸 글이 걸려 있다」 (sent text is pending), so it is not put inside the gate.
  //   (Derived display value — the subscribe/accumulate/send data flow is untouched. ADR-0044/0045/0046.)
  const streaming = awaiting || (!turnDone && items.length > 0 && !historyPending)

  // ADR-0244: lift 「중단하는 중」 (interrupting) even when the wait indicator goes off with no turn boundary — the sent text stood in the queued list without opening a turn (there is
  //   no turn to interrupt yet). The next wait indicator to stand belongs to the new turn that text opens. Lift only on on → off — something false at mount
  //   is not an end.
  const wasStreamingRef = useRef(streaming)
  useEffect(() => {
    if (wasStreamingRef.current && !streaming) useInterruptStore.getState().end(agentId)
    wasStreamingRef.current = streaming
  }, [streaming, agentId])
  // ADR-0244: while the agent is gone or the connection is lost, no view in this window has a way to see the turn end — if left, this window's interrupt
  //   command sends nothing and only answers 「이미 보냈다」 (already sent). ★It lifts even on a brief disconnect★ — an Esc after reattaching interrupts again as today.
  //   ★This view's own subscription stop (`subscriptionDown`) is not put in here★ — it is handled by dropping out of `watch` above.
  const agentOrConnectionDown = agentGone || !connected
  useEffect(() => {
    if (agentOrConnectionDown && interrupting) useInterruptStore.getState().end(agentId)
  }, [agentOrConnectionDown, interrupting, agentId])

  // ADR-0237: a bare Esc anywhere in the area (U6) = interrupt the running turn. ★Why capture★: the input box onKeyDown cuts propagation first,
  //   so with bubble the input box's Esc would not arrive here. ★It does not change the conversation's optimistic state (awaiting · turnDone)★ — that it was interrupted
  //   is announced by the turn-end event. 「중단하는 중」 (interrupting) is set by the command (ADR-0244 — the human key and the LLM take the same path). It does not touch the text in the input box
  //   either. An Esc before a turn opens (right after a send) is refused by the path (the command lifts 「중단하는 중」) and fireAndForget swallows it
  //   as a warning — just press it again.
  // ★The capability is carried as optional★ — if it cannot be read, not interrupting is right (same reason as `command` above).
  const canInterrupt = agent?.capabilities?.control?.interrupt === true
  const onRootKeyDownCapture = (e: KeyboardEvent<HTMLDivElement>): void => {
    // Filters only Esc before measuring the overlay — so the document is not scanned on every keystroke.
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
      // In the empty state the root is the alignment container itself (justify-center) — the [mascot·text·input box] group is placed
      //   at the vertical center as a whole. Splitting with top and bottom spacers would push the group up by the height of the mascot·text in the top part.
      // relative = the anchor of the absent overlay below (absolute inset-0).
      // The tone comes in as a single class (defined in richBranding.css) — claude is '' so it looks exactly as now.
      // tabIndex -1 = the area root receives focus only by click · code (outside tab order) — pressing the body margin also makes Esc
      //   reach this area (ADR-0237 U6). Pressing conversation text makes the scroll viewport receive it first, but keydown still bubbles up here.
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
      // ★The same token is on the marker too, but do not remove it here★ — when a conversation starts the title and marker fold away together
      //   (ADR-0145 empty state), and then this attribute is the only DOM that tells the backend.
      data-rich-brand={branding.brand}
      data-rich-awaiting-history={awaitingHistory ? '1' : undefined}
    >
      {/* Conversation render (scroll) — the ScrollArea seam (ADR-0053: the app-wide Radix overlay scrollbar). Order-preserving item stream.
          ★The ref is forwarded by this seam to the real scroll node (Radix Viewport) — scroll-follow (ADR-0242) measures and writes that
          node (aiming at the Root does not scroll). The CC-look render is StructuredTextView's concern. */}
      {!showEmpty && (
        <ScrollArea ref={follow.viewportRef} className="min-h-0 flex-1">
          <StructuredTextView
            items={items}
            streaming={streaming}
            slotId={viewId}
            onGroupToggle={onGroupToggle}
          />
          {/* ADR-0226 history wait — an icon in the center of the conversation area + a pale curtain (user decision 2026-09-24).
              ★Why it is placed here★: the absolute reference is the ScrollArea root (the seam's relative), so it covers exactly
              the conversation area only and does not scroll, and does not reach the input box, its sibling below. This leans on the Radix Viewport and its inner
              wrapper having no position (checked in the @radix-ui/react-scroll-area dist source) — if the inner wrapper becomes
              positioned, the reference moves there and the curtain shrinks to the content height (while waiting, one line of bottom margin).
              Placing it as a child of the slot root would cover the input box too.
              pointer-events-none — scroll, selection and clicks pass straight through.
              Why the curtain color is a pale foreground tint, not background = the area this curtain covers is always empty (waiting ends
              when a history row arrives). With a background-family color it would be the same color on the same color and invisible. */}
          {awaitingHistory && (
            <LoadingPanel className="pointer-events-none absolute inset-0 bg-foreground/8" />
          )}
          <JumpToBottom follow={follow} />
        </ScrollArea>
      )}
      {/* Wait indicator row — outside the scroll list, so turning on and off does not shake conversation height or bottom-follow (WaitRow header).
          ★`{조건 && …}` pins one child position★ — the position of the input group below does not shift (textarea comment).
          ★The identity label (§ user request) is this row's right-hand area★ — like the claude-code terminal, only the name of which
          agent it is shows at the upper right just above the input box (duplicate names allowed · status glyph is the tree's job). It shares the line with wait text, and when narrow the label is
          cut off. If it floated over the input group (absolute), in a narrow area it would cover 「중단하는 중…」 (interrupting…) and the elapsed seconds. The empty state has no
          row so no label either — just above the input box is where the product name text goes (ADR-0145 §2 layout). */}
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

      {/* ADR-0145 empty-state upper part — marker + product name. Vertical centering is done by the root (justify-center),
          and this takes only its own height. The text is a product name so it is not a translation target (ADR-0145) — codex's side also does not use i18n for the same
          reason. Which of the two to draw is decided by the single richBranding decision.
          ★When space shrinks, only the mascot shrinks★: only the mascot area is shrink+overflow-hidden, so when the slot gets
          lower it is naturally cut off and disappears, while the text and input box stay whole with flex-none
          (the way of hiding everything at a threshold height was discarded on the user's judgment that it vanishes abruptly and feels unnatural). */}
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

      {/* Input group = [queued-input list · input box] (ADR-0231). The list stands **above** the input box.
          ★The textarea is the same element in both layouts (ADR-0145)★: the position (parent children index) is fixed
          and only className·rows are swapped so React does not remount — if split, send · IME · focus guard become
          two sets, and the focus being typed in breaks at the moment of transition. The list occupying **always the same child position** with `{조건 && …}` is
          for the same reason (the input box row does not shift when the list stands and leaves). */}
      <div className={showEmpty ? 'flex flex-none flex-col' : 'flex flex-none flex-col border-t border-border'}>
        {hasQueued && <QueuedInputList agentId={agentId} entries={queued} />}
        {/* Input box — Enter sends / Shift+Enter newline (no separate send button). ★Focus guard★: stopPropagation
            keeps key input from leaking to parent/global keybindings (same isolation as the terminal slot's onData capture). */}
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
              // ★Preventing misfire of the Enter that confirms Korean IME composition (the main user is Korean)★: in WebView2 the Enter that
              //   confirms Hangul composition comes as keydown with isComposing=true (keyCode 229) — handling this as a send makes
              //   an Enter that only meant to end composition send unfinished input early. An Enter during composition is let through before the send branch.
              if (e.nativeEvent.isComposing || e.keyCode === 229) return
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault()
                send()
              }
            }}
            // Priority = ended > empty state > bottom. The ended indicator comes first (in either layout, if ended, that fact wins).
            // ★Disabled is wider than this and the text speaks only of exit★: writing "종료됨" (ended) for
            //   disconnect/subscription stop would leave an assertion in a span where the decision wavers, and that conflicts with the reason the curtain left out text
            //   (SlotUnavailableVeil header).
            placeholder={
              agentGone
                ? t('agent.terminatedPlaceholder')
                : showEmpty
                  ? t('agent.emptyInputPlaceholder')
                  : t('agent.inputPlaceholder')
            }
            disabled={agentUnavailable}
            rows={showEmpty ? 3 : 2}
            // Only the empty state gets rounded corners and a slightly larger size (ADR-0145 §5) — the bottom layout is the existing high-contrast bar as is.
            // The bottom side margins are set at a line that does not shift indentation much from the conversation body (ChatRow px-4) and the vertical
            // margin is left alone to keep the bar's height and character. The empty state is a wide box so it gets more side margin and the top is lifted a step too.
            className={
              showEmpty
                ? 'w-full max-w-[560px] resize-none rounded-xl border border-border bg-surface px-5 py-4 text-[14px] text-foreground outline-none placeholder:text-muted focus:border-accent disabled:opacity-50'
                : 'flex-1 resize-none rounded border border-border bg-surface px-3 py-1.5 text-[13px] text-foreground outline-none placeholder:text-muted focus:border-accent disabled:opacity-50'
            }
          />
        </div>
      </div>

      {/* ADR-0148: when the targeted agent is not there now (process exit · connection lost · the subscription cannot produce output —
          the three are seen as the same) it kills the slot. The look is shared by the three slots (SlotUnavailableVeil). Why it is placed as the last child =
          the indexes of earlier children must not shift so the input box is not remounted (textarea comment above). */}
      {agentUnavailable && <SlotUnavailableVeil phase={phase} />}
    </div>
  )
}
