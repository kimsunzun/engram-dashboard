// RichSlot live accumulator (tag1 StructuredEvent) — 1 frame = 1 StructuredEvent JSON (pure TS, React-independent).
//
// ★Why this differs from the S14 NDJSON accumulator (lab/richslot/streamParse.ts)★: since S15 (ADR-0045) live structured output is
//   refined by the backend and flows as a **binary frame tag1** — the payload is one self-describing StructuredEvent JSON
//   (internally-tagged `"type"` discriminator), not an NDJSON line stream. So no line reassembly (newline splitting)
//   is needed, and one feed = one event. The NDJSON path stays in use by the fixture spike (lab/richslot/fixtureParse.ts)
//   (that is a whole-file parser, this accumulator is for live tag1 only).
//
// ★Render model = order-preserving item stream (ADR-0045 §52 render, user decision 2026-07-05)★: events pile up in the item array
//   in the order they arrived.
//
// ★idempotent reconstruction invariant (replay safety — why it matters)★: on a webview reload / slot reassignment the client shared buffer
//   replays the whole history in seq order (replay, ADR-0043). The subscription effect resubscribes after reset(),
//   so feeding the same event sequence again must yield the same snapshot (so a reload restores the screen as it was). This accumulator
//   builds state only from feed order (order preserved) and reset clears everything, so reset→refeed in the same order = same result.
//   Upstream (ProtocolClient) guarantees seq dedup and ordering, so this accumulator does no duplicate/inversion defense of its own.

import type { QueuedInputEvent } from '../../../crates/engram-dashboard-protocol/bindings/QueuedInputEvent'
import type { QueuedInputRow } from '../../../crates/engram-dashboard-protocol/bindings/QueuedInputRow'
import type { StructuredEvent } from '../../../crates/engram-dashboard-protocol/bindings/StructuredEvent'
import type { ToolCategory } from '../../../crates/engram-dashboard-protocol/bindings/ToolCategory'
import type { ToolOutcome } from '../../../crates/engram-dashboard-protocol/bindings/ToolOutcome'
import type { TurnOutcome } from '../../../crates/engram-dashboard-protocol/bindings/TurnOutcome'
import { entryOfListedRow, QueuedInputRegistry, type QueuedEntry } from './queuedInputReducer'

/**
 * Marker left on screen when a turn closed with an outcome that is **not a normal completion** — neutral vocabulary (no backend name).
 * ★Do not draw 'interrupted' as a failure★ — a user interrupt is a first-class normal path in this repository.
 */
export type TurnOutcomeMark = 'failed' | 'interrupted' | 'unknown'

// ADR-0239 · ADR-0241: re-export the generated types here so consumers get them from this one module.
export type { ToolCategory, ToolOutcome }

/** End marker of a tool row — `ToolOutcome` minus normal completion. */
export type ToolResultMark = 'failed' | 'declined' | 'refused'

/** `itemId` is a monotonic id within an accumulator instance (back to 0 on reset, used as the React key). */
export type StructuredItem =
  | { kind: 'text'; text: string; itemId: number }
  // id is the backend tool-use id.
  | {
      kind: 'tool'
      name: string
      argsJson: string
      id: string | null
      category: ToolCategory
      /**
       * Do not read no marker (`null`) as success — it is the same value for normal completion and also for a call whose end has not come (interrupted and still running · old daemon ·
       * pushed out of the ring) and for an unknown outcome. A later result overwrites an earlier marker (clears it for completion · an unknown outcome).
       */
      resultMark: ToolResultMark | null
      itemId: number
    }
  | { kind: 'usage'; inputTokens: number; outputTokens: number; itemId: number }
  | { kind: 'error'; message: string; itemId: number }
  // Escape-hatch event (absorbs codex/gemini · API model leaks).
  | { kind: 'structured'; label: string; json: string; itemId: number }
  | { kind: 'separator'; itemId: number }
  // Turn outcome marker — `detail` = failure reason (only when the other side gave one). A normal completion makes no such item (divider only).
  | { kind: 'outcome'; outcome: TurnOutcomeMark; detail: string | null; itemId: number }
  // Interrupt notice the backend writes in an interrupted turn (where it is emitted today = claude synthetic line) — `text` = verbatim. A turn containing this item
  //   does not draw the interrupt outcome row (ADR-0243).
  | { kind: 'interruptNote'; text: string; itemId: number }
  // Marker that an event this shell does not know arrived. `count` = consecutive accumulated amount. ★The original payload is not carried★ (default arm below).
  | { kind: 'unsupported'; count: number; itemId: number }

export type ToolItem = Extract<StructuredItem, { kind: 'tool' }>

/**
 * The list query answer handed to reattach reconciliation — the answer of `AgentClient.listQueuedInputs` fits as is.
 * `as_of_seq` = seq of the last list event the daemon registry reduced (`null` = all list events of that incarnation are after the snapshot).
 */
export interface QueuedSnapshot {
  readonly inputs: readonly QueuedInputRow[]
  readonly as_of_seq: number | null
  readonly epoch: number
}

/**
 * Outcome of `offerQueuedSnapshot`. `held` = the view has not yet been delivered up to the snapshot seq so it holds and waits (it is applied when that seq
 * arrives via `feed`·`observeSeq`) · `stale` = discarded because it was an answer of another generation or incarnation.
 */
export type QueuedReconcileOutcome = 'applied' | 'held' | 'stale'

/** One reattach reconciliation in progress — after sending the query it records the list events received, and holds the answer that came first. */
interface QueuedReconcile {
  readonly gen: number
  readonly epoch: number
  readonly log: Array<{ readonly seq: number; readonly op: QueuedInputEvent }>
  held: { readonly rows: QueuedEntry[]; readonly listed: ReadonlySet<string>; readonly asOfSeq: number } | null
}

export class StructuredEventAccumulator {
  private items: StructuredItem[] = []
  private turnDone = false
  // ADR-0244: count of turn boundaries consumed — ★reset does not roll it back either★ (the caller compares only before and after one `feed`).
  private turnBoundaries = 0
  // Monotonic item id — back to 0 on reset(). Refeeding the same event sequence reproduces the same ids (replay idempotence).
  private nextId = 0
  // ★user message uuid dedup (blunt-suppress → uuid dedup swap, text blocks only)★: in json mode, right after write_input the
  //   session first flows the synthetic user echo (Structured{kind:"user", uuid attached to json}), and later claude echoes back the **same uuid**
  //   as is via --replay-user-messages (the backend decoder carries the line-level uuid
  //   into the block json). A user item with an already seen uuid is skipped here so exactly one remains.
  //   ★Dedup applies only to `type==="text"` user blocks★: the synthetic echo makes only a single text block (the dedup partner
  //   also arises only from text), so non-text blocks such as tool_result in the same replay line are kept without dedup
  //   even if they share the same uuid (extractUserUuid returns null for non-text — HIGH FIX against multi-block tool_result loss).
  //   User items without a uuid (past/non-replay) are also all kept without dedup (vanish prevention).
  //   reset() clears it so replay idempotence holds (on refeed it sees the same uuids in the same order again and reconverges).
  // ADR-0231: the bubble of a queued input also keeps 「한 id 에 한 번」 (once per id) with this set — the `Delivered` placement · ascertained copy
  //   put into it here, and a uuid in it is not drawn in the list (`snapshotQueued`) either.
  private seenUserUuids = new Set<string>()
  // ADR-0231: reduced state of the queued-input registry (same rule · same golden as the agent registry). The draw filter is applied outside this.
  private readonly queued = new QueuedInputRegistry()
  // ADR-0231 reattach reconciliation: the largest seq this instance was delivered (regardless of tag — one seq space). reset returns it to -1.
  private deliveredSeq = -1
  // Reconciliation generation. ★reset does not return it to 0★ — if it did, the answer to a query sent before clearing would be read as the answer to a new
  //   query carrying the same number and applied (the record was cleared meanwhile, so there is no material to reduce the after-snapshot events again).
  private reconcileGen = 0
  private reconcile: QueuedReconcile | null = null

  /**
   * The live path is always Uint8Array; string is for tests/convenience.
   * tag1 is 1 frame = 1 event so no line reassembly or buffering is needed.
   *
   * @param seq the seq of this frame. ★Reattach reconciliation stands on this value alone★ — if omitted, this frame's list events are not left in the reconciliation
   *   record and a held answer does not count this frame as delivered either (omitted only for test convenience).
   * @returns whether the caller may release its own waiting state because of this frame. `false` is two cases — ① parsing failed or
   *   it is a kind this shell does not know, so none of this frame's meaning is reflected in the returned state (`snapshot`·`isTurnDone`)
   *   ② it is a tool end result (`ToolResult`) — the marker of an earlier tool row may have changed (if there is no row pointed at or the marker is
   *   the same, `snapshot` stays as it is) but a response did not arrive (ADR-0241).
   *   ★The caller looks at this value to decide whether to hand its waiting state to the accumulator★ — if it hands over
   *   just because a frame arrived, a frame that was not understood is disguised as 「응답이 왔다」 (a response came) and the waiting indicator goes off (`awaiting` of RichSlot).
   */
  feed(payload: Uint8Array | string, seq?: number): boolean {
    const understood = this.parseAndConsume(payload, seq)
    // A frame that was not understood still had its seq delivered — what a held reconciliation answer waits for is the seq, not the meaning.
    if (seq !== undefined) this.observeSeq(seq)
    return understood
  }

  private parseAndConsume(payload: Uint8Array | string, seq: number | undefined): boolean {
    const json = typeof payload === 'string' ? payload : new TextDecoder('utf-8').decode(payload)
    if (!json) return false
    let parsed: unknown
    try {
      parsed = JSON.parse(json)
    } catch (err) {
      // The path is a dumb pipe (unrefined) — malformed JSON is a protocol-level data loss signal, so warn then skip.
      console.warn('[structuredAccumulator] tag1 JSON 파싱 실패 — 이벤트 스킵:', err)
      return false
    }
    // If `null` is not filtered here, `consume`'s read of `ev.type` throws up to the subscription callback (a primitive is not an event either).
    if (parsed === null || typeof parsed !== 'object') {
      console.warn('[structuredAccumulator] tag1 payload 가 객체가 아니다 — 이벤트 스킵:', parsed)
      return false
    }
    return this.consume(parsed as StructuredEvent, seq)
  }

  /** @returns same meaning as `feed` above. */
  private consume(ev: StructuredEvent, seq: number | undefined): boolean {
    switch (ev.type) {
      case 'TextDelta': {
        // Skip an empty delta ("") so it does not make a phantom item (which would cause an empty Markdown block · a meaningless divider).
        if (!ev.text) break
        // copy-on-write: a previously returned snapshot() reference points at this object, so no in-place mutation.
        const last = this.items[this.items.length - 1]
        if (last && last.kind === 'text') {
          this.items[this.items.length - 1] = { ...last, text: last.text + ev.text }
        } else {
          this.items.push({ kind: 'text', text: ev.text, itemId: this.nextId++ })
        }
        this.turnDone = false
        break
      }
      case 'ToolCall':
        this.items.push({
          kind: 'tool',
          name: ev.name,
          argsJson: ev.args_json,
          id: ev.id,
          // ADR-0239: the type is narrow but the wire value carries an old daemon (no field) · a newer daemon (unknown word).
          category: normalizeToolCategory(ev.category),
          resultMark: null,
          itemId: this.nextId++,
        })
        this.turnDone = false
        break
      case 'Usage':
        this.items.push({
          kind: 'usage',
          inputTokens: ev.input_tokens,
          outputTokens: ev.output_tokens,
          itemId: this.nextId++,
        })
        break
      case 'Error':
        // Do not accumulate as text; show as a separate item.
        // ★Does not close the turn — do not revive that★: this vocabulary is 「턴 경계 아님」 (not a turn boundary) on both backends. codex emits
        //   a retryable stream error (overload · rate limit) with this and the same turn continues after that line
        //   (`error` notification in `backend/codex/decoder.rs`), and claude closes even a failed turn with `MessageDone`
        //   (the result line in `backend/claude/mod.rs` always attaches MessageDone after Error). The turn classifiers
        //   of both backends also give no signal to this vocabulary — closing only here puts the two axes out of step.
        //   Symptom when closed = during a retry the waiting indicator goes off and revives on the next delta, flickering.
        // ★Known hole (the frontend cannot plug it)★: the wire `StructuredEvent::Error` has no retryable field.
        //   So on a path of 「정말 턴을 끝낸 오류인데 경계가 안 오는」 (an error that truly ended the turn but no boundary comes) the waiting indicator stays.
        //   Do not tell them apart by searching the string — not writing that distinction in the string is the decoder side's decision.
        this.items.push({ kind: 'error', message: ev.message, itemId: this.nextId++ })
        break
      case 'Structured': {
        // ADR-0243: does not take the user branch (uuid dedup · clearing `turnDone`) — it is not text the user typed.
        if (ev.kind === 'interrupted') {
          const text = extractInterruptText(ev.json)
          if (text !== null) {
            this.items.push({ kind: 'interruptNote', text, itemId: this.nextId++ })
            break
          }
        }
        // ★user uuid dedup (text blocks only)★: a user item arrives twice with the **same uuid** from the synthetic input-time echo and the claude replay
        //   (backend uuid dedup contract). If the uuid was already seen, skip so only one remains.
        //   But dedup applies only to `type==="text"` user blocks — the synthetic echo makes only a single text block, so
        //   the dedup partner also arises only from text. Non-text blocks such as tool_result in one replay line, even if they share the same line-level
        //   uuid, get null from extractUserUuid, so they are not dedup targets and are always kept
        //   (prevents tool_result loss in multi-block — HIGH FIX). If there is no uuid (past/non-replay) keep as is.
        //   (kind!=='user' escape-hatch events are not dedup targets — they have no uuid concept.)
        if (ev.kind === 'user') {
          const uuid = extractUserUuid(ev.json)
          if (uuid !== null) {
            // ADR-0231: ★the echo of a queued uuid is neither drawn nor put into 「본 것」 (seen)★ — that bubble is drawn by the `Delivered` of that id
            //   at the receipt spot. Measurement showed both orders: if the echo comes right before `Delivered` it is received
            //   here, if after, the uuid dedup below receives it. So 「첫 항목이 이긴다」 (the first item wins) stands only for
            //   uuids that are not queued.
            if (this.queued.row(uuid) !== undefined) break
            if (this.seenUserUuids.has(uuid)) break
            this.seenUserUuids.add(uuid)
          }
          // ★New user turn start → clear turnDone (=idle)★: the previous MessageDone left turnDone=true, but when a new user
          //   message arrives we are waiting for the assistant response (no longer idle). If this is not cleared, at the moment a follow-up send's
          //   synthetic user echo releases awaiting (RichSlot subscription callback setAwaiting(false)) derived
          //   streaming drops to false, and the waiting indicator (WaitRow) flickers off until the first assistant token
          //   (follow-up send flicker). Replay idempotence kept: the final MessageDone of a complete history sets turnDone=true again,
          //   so the final state after refeed is unchanged — only the intermediate transition becomes correct. (Dedup-skipped ones cannot get here due to the break above.)
          this.turnDone = false
        }
        // Escape-hatch event — flow even an unknown kind (kind) through to prevent loss.
        this.items.push({ kind: 'structured', label: ev.kind, json: ev.json, itemId: this.nextId++ })
        break
      }
      case 'MessageDone':
        // ADR-0045: the claude decoder emits MessageDone exactly once per result line · per turn, and its
        //   `turn_id` is still always None (both emit points in `backend/claude/mod.rs` are literal None).
        //   ★So do not derive the boundary from turn_id here★ — it is always-None, so the boundary would vanish.
        // ★But it is not the 「유일한 턴 경계 트리거」 (only turn-boundary trigger)★ — the codex decoder emits no MessageDone at all and closes the turn with
        //   `TurnEnd` below (its turn_id arrives filled in). The two vocabularies coexist and the render path is one.
        this.closeTurn()
        break
      case 'TurnEnd':
        // ★All four outcomes close the turn★ — what the outcome field decides is the on-screen marker, not 「닫을지 말지」 (whether to close).
        //   If even one does not close it, that conversation's waiting indicator spins forever.
        {
          const mark = outcomeMark(ev.outcome)
          // ADR-0243: if an interrupt-notice row already told of that turn's interrupt, the outcome row is not drawn on top — decided not by the backend but
          //   by the presence of the notice row (a turn where no notice row arrived keeps its outcome row as is).
          const alreadyNoted = mark?.outcome === 'interrupted' && this.currentTurnHasInterruptNote()
          if (mark !== null && !alreadyNoted) this.items.push({ kind: 'outcome', ...mark, itemId: this.nextId++ })
        }
        this.closeTurn()
        break
      case 'QueuedInput':
        return this.consumeQueued(ev.op, seq)
      case 'ToolResult':
        // ADR-0241
        return this.consumeToolResult(ev)
      default: {
        // ★Do not swallow unknown events silently★: if nothing is done the screen does not change by one pixel while the caller
        //   acts as if a frame came. Release WebView2 has no devtools, so the console does not reach the user
        //   (same reason ConnectionNotice exists), so make a marker that stays on screen.
        // ★Do not carry the original payload · event name★ — the decision that protocol words do not go on the user's screen
        //   (trd-phase2a §6-2). The diagnostic name goes out to the console only.
        // ★Do not expect this item to prop up the waiting indicator (an old misunderstanding — do not revive it)★: the consumer's derivation is
        //   `!turnDone && items.length>0`, so after the previous turn has already closed (= from the second turn on), adding an item
        //   leaves turnDone true and the waiting indicator goes off just the same. What guards the waiting indicator is **the false return below**
        //   (the caller does not release its own waiting state), and this item takes only the side of leaving 「무슨 일이 있었나」 (what happened).
        // ★Even so, do not lower `turnDone`★ — we do not know whether the unknown event ended the turn, and lowering it to
        //   false here changes things toward the waiting indicator spinning forever **when the last frame of a restored history is an unknown kind**
        //   (if a newer daemon attaches a new vocabulary at turn end, every restored session takes that shape). A live gap is
        //   fixed by the next frame itself, but the end of a history has no next frame to fix it — so the axes were split.
        // ★Known hole★: if the last frame of a history is an unknown kind and what precedes it is an unfinished turn, the waiting indicator stays.
        //   It cannot be told apart from frontend information alone (no separate signal on the wire tells the turn state).
        // ★The `never` assignment is a compile-time defense★ — if variants grow in **this shell's bindings** a type error arises
        //   here (the thing to do then is not lean on this arm but add a proper arm). The runtime defense is below
        //   that — a kind a newer daemon sends that is **not at all** in this shell's bindings. The two target different windows.
        const exhaustive: never = ev
        console.warn(
          '[structuredAccumulator] 모르는 StructuredEvent type — 표식만 남기고 버린다:',
          (exhaustive as { type?: unknown }).type,
        )
        const last = this.items[this.items.length - 1]
        if (last && last.kind === 'unsupported') {
          // copy-on-write — a previously returned snapshot() reference points at this object (same discipline as the TextDelta arm).
          this.items[this.items.length - 1] = { ...last, count: last.count + 1 }
        } else {
          this.items.push({ kind: 'unsupported', count: 1, itemId: this.nextId++ })
        }
        return false
      }
    }
    return true
  }

  /** Is there an interrupt notice row after the last divider (= the current turn). */
  private currentTurnHasInterruptNote(): boolean {
    for (let i = this.items.length - 1; i >= 0; i--) {
      const kind = this.items[i].kind
      if (kind === 'separator') return false
      if (kind === 'interruptNote') return true
    }
    return false
  }

  /**
   * Close a turn boundary — 1 divider + `turnDone`. Consecutive ends do not pile up dividers, and if there is no preceding item
   * no divider is made (prevents an empty boundary at the very front).
   */
  private closeTurn(): void {
    if (this.items.length > 0 && this.items[this.items.length - 1].kind !== 'separator') {
      this.items.push({ kind: 'separator', itemId: this.nextId++ })
    }
    this.turnDone = true
    this.turnBoundaries++
  }

  /**
   * Attach the tool end result as a marker to the earlier tool row of the same id — no new item is made. If there is no row pointed at
   * (pushed out of the ring) discard it.
   * ★Does not touch `turnDone` and always returns `false` — whether attached or not found★: the tool of an interrupted turn keeps running and ends after the turn end
   * (possibly in the middle of the next turn). Releasing waiting with this frame would turn off the new turn's waiting indicator without an answer.
   */
  // ADR-0241
  private consumeToolResult(ev: ToolResultEvent): boolean {
    // Filter so a `null` id of a malformed frame does not attach to a tool row with no id.
    if (typeof ev.id !== 'string') return false
    const mark = toolResultMark(ev.outcome)
    for (let i = this.items.length - 1; i >= 0; i--) {
      const item = this.items[i]
      if (item.kind !== 'tool' || item.id !== ev.id) continue
      // copy-on-write — a previously returned snapshot() reference points at this object (same discipline as the TextDelta arm).
      if (item.resultMark !== mark) this.items[i] = { ...item, resultMark: mark }
      return false
    }
    return false
  }

  /**
   * Queued-input registry event — after reduction only three draw rules change the conversation lines (`Delivered` placement · ascertained copy placement ·
   * `Rejected` bubble erase). ★Does not touch `turnDone`★ — list events are not output (only when it draws a bubble
   * does it lower it like the user arm). A termination just leaves the list and draws no notice row.
   */
  // ADR-0231
  private consumeQueued(op: QueuedInputEvent | null | undefined, seq: number | undefined): boolean {
    // The try/catch in `feed` wraps only JSON.parse — it does not throw on a malformed frame (same discipline as outcomeMark).
    if (op === null || typeof op !== 'object') return false
    // The placement body is a copy of the earlier `Queued` — grab it before the reduction deletes that item.
    const pending = op.kind === 'Delivered' ? this.queued.row(op.id) : undefined
    if (op.kind === 'AckUnavailable' && !isCopyList(op.delivered)) return false
    if (op.kind === 'HandedOver' && typeof op.sent !== 'boolean') return false
    if (this.queued.reduce(op) === null) {
      console.warn(
        '[structuredAccumulator] 모르는 QueuedInput kind — 버린다:',
        (op as { kind?: unknown }).kind,
      )
      return false
    }
    // Reconciliation record — write only events the reduction accepted (they are run again through the same reducer at apply time, so it must be the filtered shape).
    if (seq !== undefined && this.reconcile !== null) this.reconcile.log.push({ seq, op })
    switch (op.kind) {
      case 'Delivered':
        // An id that was not queued (revival · unknown id · a second `Delivered`) is not drawn — the bubble of a revival is
        //   drawn by the vendor echo, which is not suppressed because that id is no longer queued.
        if (pending !== undefined) this.placeUserBubble(pending.id, pending.text)
        break
      case 'AckUnavailable':
        // For each copy (list order) in that spot — even if this window lost that `Queued` from the ring, draw it with the copy's body.
        for (const copy of op.delivered) this.placeUserBubble(copy.id, copy.text)
        break
      case 'Dropped':
        // Draw rule independent of reduced state — only a refusal erases (bubbles of interrupted · exited · failed turns stay).
        if (op.cause === 'Rejected') this.removeUserBubbles(op.id)
        break
      default:
        break
    }
    return true
  }

  /** User bubble at the receipt spot — same shape as the echo · synthetic echo, so the renderer does not tell them apart. Once per uuid. */
  private placeUserBubble(id: string, text: string): void {
    if (this.seenUserUuids.has(id)) return
    this.seenUserUuids.add(id)
    this.items.push({
      kind: 'structured',
      label: 'user',
      json: JSON.stringify({ type: 'text', text, uuid: id }),
      itemId: this.nextId++,
    })
    this.turnDone = false
  }

  /**
   * Remove the user bubble (text block) drawn with that uuid. ★The uuid stays in 「본 것」 (seen)★ — a refusal cannot be revived so
   * a later same uuid is not drawn again. A non-text block (tool_result) of the same line, as it is not a dedup target, also
   * stays here.
   */
  private removeUserBubbles(id: string): void {
    for (let i = this.items.length - 1; i >= 0; i--) {
      const item = this.items[i]
      if (item.kind === 'structured' && item.label === 'user' && extractUserUuid(item.json) === id) {
        this.items.splice(i, 1)
      }
    }
  }

  /**
   * Items `QueuedInputList` will draw — among open items only uuids not yet drawn as a bubble, in held order (oldest
   * first). A new array each time.
   * ★Cancel-pending is drawn too★ (ADR-0235 — ✕ removes the row only after the registry confirms it removed it): a row where ✕ was pressed
   *   stays until an outcome event (`CancelAnswered{removed:true}` · `Dropped{Withdrawn}` etc.) is reduced — hiding it in advance
   *   creates a window where text that could not be withdrawn is seen neither in the list nor in the conversation.
   * ★The filter stands on drawing only★ — the reduced state same as the registry is `queuedRows()`.
   */
  // ADR-0231
  snapshotQueued(): QueuedEntry[] {
    return this.queued.rows().filter((entry) => !this.seenUserUuids.has(entry.id))
  }

  /** Reduced state as is (including cancel-pending · already drawn uuids) — same values as the agent registry · golden. */
  queuedRows(): readonly QueuedEntry[] {
    return this.queued.rows()
  }

  /**
   * Open a reattach reconciliation (TRD §5-7) — call it **right before** sending the list query. From this moment it records list events received
   * (events after the snapshot are all issued after the query read the registry, so they fall inside this record). A reconciliation
   * in progress is discarded.
   * @param epoch marker of the incarnation this view currently reads — discard if the answer's marker differs.
   * @returns reconciliation generation of this query — hand it along when passing the answer to `offerQueuedSnapshot`.
   */
  // ADR-0231
  beginQueuedReconcile(epoch: number): number {
    this.reconcileGen += 1
    this.reconcile = { gen: this.reconcileGen, epoch, log: [], held: null }
    return this.reconcileGen
  }

  /**
   * Hand over the list query answer. If generation and incarnation match, apply **after** the view has been delivered up to the snapshot seq — for an
   * id in the snapshot, start from that row and re-reduce only recorded events with a larger seq; an id not in it is closed, if it stood after the snapshot, as the current
   * state as is · otherwise with cause unknown (`QueuedInputRegistry.adoptSnapshot`). Applying fixes only the list
   * (draws no bubble). A row's unknown word drops only that row — that id is left as its current state.
   */
  // ADR-0231
  offerQueuedSnapshot(gen: number, snapshot: QueuedSnapshot): QueuedReconcileOutcome {
    const r = this.reconcile
    if (r === null || r.gen !== gen) return 'stale'
    if (snapshot.epoch !== r.epoch) {
      // It is another incarnation's registry — this reconciliation has no further use (a new incarnation opens a new reconciliation after the clear).
      this.reconcile = null
      return 'stale'
    }
    const rows: QueuedEntry[] = []
    const listed = new Set<string>()
    for (const row of Array.isArray(snapshot.inputs) ? snapshot.inputs : []) {
      const id = (row as { id?: unknown } | null)?.id
      if (typeof id === 'string') listed.add(id)
      const entry = entryOfListedRow(row)
      if (entry !== null) rows.push(entry)
    }
    // `null` = all list events of that incarnation are after the snapshot — there is no seq to wait for.
    const asOfSeq = typeof snapshot.as_of_seq === 'number' ? snapshot.as_of_seq : -1
    r.held = { rows, listed, asOfSeq }
    return this.applyHeldSnapshot() ? 'applied' : 'held'
  }

  /**
   * Discard the reconciliation in progress — when the view leaves `'live'` and enters a new replay cycle (`reset` calls this too). It
   * raises the generation, so an old answer arriving after that is `stale`.
   * @param gen if given, discard only when that generation is the current one (query failure — if a new reconciliation opened meanwhile, do not touch it).
   */
  // ADR-0231
  abandonQueuedReconcile(gen?: number): void {
    if (gen !== undefined && this.reconcile?.gen !== gen) return
    this.reconcileGen += 1
    this.reconcile = null
  }

  /**
   * Count the seq of a frame that does not pass through `feed` (tag0, which the structured slot does not draw) as delivered too.
   * @returns whether the held reconciliation answer was applied by this delivery (if so, re-read the list).
   */
  // ADR-0231
  observeSeq(seq: number): boolean {
    if (seq > this.deliveredSeq) this.deliveredSeq = seq
    return this.applyHeldSnapshot()
  }

  private applyHeldSnapshot(): boolean {
    const r = this.reconcile
    if (r === null || r.held === null || this.deliveredSeq < r.held.asOfSeq) return false
    const asOfSeq = r.held.asOfSeq
    const later = r.log
      .filter((entry) => entry.seq > asOfSeq)
      .sort((a, b) => a.seq - b.seq)
      .map((entry) => entry.op)
    this.queued.adoptSnapshot(r.held.rows, later, r.held.listed)
    this.reconcile = null
    return true
  }

  /** Returns the internal array reference as is — a React consumer takes a new reference with [...snapshot()] and sets it. */
  snapshot(): StructuredItem[] {
    return this.items
  }

  /**
   * Was the last signal a turn end (MessageDone · TurnEnd) — streaming/idle display hint (optional).
   * ★`Error` is not in this list★ — it is not a turn boundary on either backend (comment on that arm).
   */
  isTurnDone(): boolean {
    return this.turnDone
  }

  /**
   * Count of turn boundaries consumed so far (MessageDone · TurnEnd — regardless of outcome). Monotonic and `reset` does not roll it back — the difference before and after one `feed`
   * is itself 「그 프레임이 턴 경계였다」 (that frame was a turn boundary). ★Do not substitute `isTurnDone` going false → true★ — a boundary received while already reading the end
   * (right after sending · before a new turn's first frame) has no such transition.
   */
  turnBoundaryCount(): number {
    return this.turnBoundaries
  }

  /** Initialize before resubscribing (replay) — so the whole history flows again and is reconstructed to the same state (idempotent invariant above). */
  reset(): void {
    this.items = []
    this.turnDone = false
    this.nextId = 0
    this.seenUserUuids.clear()
    this.queued.clear()
    this.deliveredSeq = -1
    // Discard a reconciliation in progress too — the recorded events are no material to reduce again on a cleared registry.
    this.abandonQueuedReconcile()
  }
}

/**
 * Outcome → on-screen marker. ★A normal completion leaves no marker (null)★ — end with the divider alone so an ordinary completion
 * does not look like a warning.
 * ★`default` receives two things★ — ① a fifth outcome a newer daemon added ② a frame with no outcome field or a malformed shape. Both
 * have unknown meaning so record them as 「모름」 (unknown) — folding into one of the three known would make the screen draw a false outcome
 * (same discipline as the wire `TurnOutcome` comment).
 * ★Accepting nullish even though the type is `TurnOutcome` is intentional★ — the try/catch in `feed` wraps only JSON.parse, so
 * if it throws here the exception rises as is up to the subscription callback (RichSlot). Make no shape assumptions.
 */
function outcomeMark(
  outcome: TurnOutcome | null | undefined,
): { outcome: TurnOutcomeMark; detail: string | null } | null {
  switch (outcome?.kind) {
    case 'Completed':
      return null
    case 'Failed':
      // A frame with no `detail` at all also leaves the marker — only the reason is unknown, a failure is still a failure.
      return { outcome: 'failed', detail: outcome.detail ?? null }
    case 'Interrupted':
      return { outcome: 'interrupted', detail: null }
    case 'Unknown':
      return { outcome: 'unknown', detail: null }
    default:
      return { outcome: 'unknown', detail: null }
  }
}

// ADR-0241
type ToolResultEvent = Extract<StructuredEvent, { type: 'ToolResult' }>

/**
 * ★An unknown word (newer daemon) · a broken shape means no marker★ — folding into one of the three known would make the screen draw a false outcome
 * (same discipline as `outcomeMark`).
 */
// ADR-0241: the argument is the generated `ToolOutcome`, so if words grow or change tsc goes red at the `never` assignment in `default`
//   (word drift alarm). At runtime a newer daemon's unknown value falls straight into `default`.
function toolResultMark(outcome: ToolOutcome): ToolResultMark | null {
  switch (outcome) {
    case 'Failed':
      return 'failed'
    case 'Declined':
      return 'declined'
    case 'Refused':
      return 'refused'
    case 'Completed':
      return null
    default: {
      const unknownWord: never = outcome
      console.warn('[structuredAccumulator] 모르는 ToolResult outcome — 표식 없이 둔다:', unknownWord)
      return null
    }
  }
}

// ADR-0239: being a `Record`, if words in `ToolCategory` grow or shrink tsc goes red here.
const TOOL_CATEGORIES: Record<ToolCategory, true> = {
  Read: true,
  Search: true,
  List: true,
  Edit: true,
  Command: true,
  Web: true,
  Agent: true,
  Mcp: true,
  Other: true,
}

export function normalizeToolCategory(value: unknown): ToolCategory {
  // Not using `in` is so a prototype name like `'toString'` is not accepted as a kind.
  return typeof value === 'string' && Object.prototype.hasOwnProperty.call(TOOL_CATEGORIES, value)
    ? (value as ToolCategory)
    : 'Other'
}

function isCopyList(value: unknown): boolean {
  return (
    Array.isArray(value) &&
    value.every((copy) => copy !== null && typeof copy === 'object' && typeof copy.id === 'string')
  )
}

/** Extract the original text from the `json` (`{"text": …}`) of `Structured{kind:"interrupted"}`. null if the shape differs (→ escape-hatch item). */
// ADR-0243
function extractInterruptText(json: string): string | null {
  try {
    const parsed: unknown = JSON.parse(json)
    if (parsed === null || typeof parsed !== 'object') return null
    const text = (parsed as Record<string, unknown>)['text']
    // An empty text becomes a notice row with nothing to draw and erases only the outcome row — send it to the escape hatch.
    return typeof text === 'string' && text !== '' ? text : null
  } catch {
    return null
  }
}

/**
 * Extract the dedup key `uuid` from the json of a user Structured item (attached in common by the synthetic echo · replay).
 * Shape the backend plants: `{"type":"text","text":…,"uuid":"X"}` (synthetic echo) / a replay block has the uuid in the same place.
 * null if there is no uuid or it is not a string (→ the caller keeps it without dedup).
 *
 * ★dedup only on `type==="text"` blocks★ (prevents multi-block loss — HIGH FIX): the backend decoder
 *   passes the same line-level uuid into **all content blocks** (text · tool_result etc.) of one user replay line
 *   (claude.rs consume_block). The only block the synthetic echo can make is a single
 *   `{"type":"text"}` (user_text_echo_json), so a dedup partner arises only for text blocks too.
 *   If we extract only the uuid here without looking at type, the text (echo) + tool_result of one line share **the same uuid**,
 *   and the tool_result is skipped as an "already seen uuid" and lost (tool OUT body vanishes). So
 *   only a user block with `type==="text"` returns a uuid, and non-text blocks such as tool_result get null →
 *   excluded from dedup → always kept (regardless of uuid presence). (Recommended seam — a local frontend fix, matches the dedup intent.)
 */
function extractUserUuid(json: string): string | null {
  try {
    const parsed: unknown = JSON.parse(json)
    if (parsed !== null && typeof parsed === 'object') {
      const obj = parsed as Record<string, unknown>
      if (obj['type'] !== 'text') return null
      const uuid = obj['uuid']
      if (typeof uuid === 'string' && uuid.length > 0) return uuid
    }
    return null
  } catch {
    return null
  }
}
