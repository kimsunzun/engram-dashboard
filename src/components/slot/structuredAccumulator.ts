// RichSlot live accumulator (tag1 StructuredEvent) — 1 frame = 1 StructuredEvent JSON (pure TS, React-independent).
//
// Live structured output is a binary frame tag1 carrying one self-describing StructuredEvent JSON (ADR-0045), not an NDJSON
// line stream: no line reassembly, one feed = one event. The NDJSON parser (lab/richslot/fixtureParse.ts) is whole-file, fixture spike only.
//
// Render model = order-preserving item stream (ADR-0045): events pile up in arrival order.
//
// ★Idempotent reconstruction (replay safety)★: on webview reload / slot reassignment the client buffer replays the whole history in seq order (ADR-0043)
// after reset(), so feeding the same sequence again must yield the same snapshot. State derives only from feed order and reset clears everything.
// Upstream (ProtocolClient) guarantees seq dedup and ordering, so no duplicate/inversion defense here.

import type { QueuedInputEvent } from '../../../crates/engram-dashboard-protocol/bindings/QueuedInputEvent'
import type { QueuedInputRow } from '../../../crates/engram-dashboard-protocol/bindings/QueuedInputRow'
import type { StructuredEvent } from '../../../crates/engram-dashboard-protocol/bindings/StructuredEvent'
import type { ToolCategory } from '../../../crates/engram-dashboard-protocol/bindings/ToolCategory'
import type { ToolOutcome } from '../../../crates/engram-dashboard-protocol/bindings/ToolOutcome'
import type { TurnOutcome } from '../../../crates/engram-dashboard-protocol/bindings/TurnOutcome'
import { entryOfListedRow, QueuedInputRegistry, type QueuedEntry } from './queuedInputReducer'

/** Non-normal turn outcome marker (neutral vocabulary, no backend name). Do not draw 'interrupted' as a failure — a user interrupt is a normal path. */
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
      /** `null` is not success: it also covers a call whose end has not come (running · interrupted · old daemon · pushed out of the ring) and an unknown outcome. A later result overwrites an earlier marker. */
      resultMark: ToolResultMark | null
      itemId: number
    }
  | { kind: 'usage'; inputTokens: number; outputTokens: number; itemId: number }
  | { kind: 'error'; message: string; itemId: number }
  // Escape-hatch event (absorbs codex/gemini · API model leaks).
  | { kind: 'structured'; label: string; json: string; itemId: number }
  | { kind: 'separator'; itemId: number }
  // Turn outcome marker; `detail` = failure reason when given. A normal completion makes no item (divider only).
  | { kind: 'outcome'; outcome: TurnOutcomeMark; detail: string | null; itemId: number }
  // Interrupt notice the backend writes in an interrupted turn, `text` verbatim; a turn containing it draws no interrupt outcome row (ADR-0243).
  | { kind: 'interruptNote'; text: string; itemId: number }
  // Unknown event arrived; `count` = consecutive accumulated. The original payload is not carried.
  | { kind: 'unsupported'; count: number; itemId: number }

export type ToolItem = Extract<StructuredItem, { kind: 'tool' }>

/** Answer of `AgentClient.listQueuedInputs`. `as_of_seq` = last list event seq the daemon registry reduced (`null` = all list events of that incarnation are after the snapshot). */
export interface QueuedSnapshot {
  readonly inputs: readonly QueuedInputRow[]
  readonly as_of_seq: number | null
  readonly epoch: number
}

/** Outcome of `offerQueuedSnapshot`: `held` = view not yet delivered up to the snapshot seq (applied when it arrives via `feed`·`observeSeq`); `stale` = answer of another generation or incarnation. */
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
  // ADR-0244: turn boundaries consumed; reset does not roll it back (callers compare before/after one `feed`).
  private turnBoundaries = 0
  // Monotonic item id; 0 after reset(), so a refeed reproduces the same ids.
  private nextId = 0
  // User uuid dedup (text blocks only): after write_input the synthetic user echo arrives first, then claude echoes the same uuid via
  //   --replay-user-messages; a user item with an already seen uuid is skipped so exactly one remains.
  //   ★Only `type==="text"` blocks dedup★ (extractUserUuid returns null for non-text): tool_result blocks of the same replay line share the uuid and must be kept.
  //   User items without a uuid are kept. reset() clears the set (replay idempotence).
  // ADR-0231: a queued input's bubble uses this set too (`Delivered` placement · ascertained copy put the uuid here); a uuid in it is not drawn in `snapshotQueued` either.
  private seenUserUuids = new Set<string>()
  // ADR-0231: reduced queued-input registry (same rule · golden as the agent registry); the draw filter applies outside.
  private readonly queued = new QueuedInputRegistry()
  // ADR-0231 reattach reconciliation: largest seq delivered (any tag — one seq space); reset returns it to -1.
  private deliveredSeq = -1
  // Reconciliation generation. ★reset does not return it to 0★: an answer to a pre-clear query would otherwise match a new query with the same number.
  private reconcileGen = 0
  private reconcile: QueuedReconcile | null = null

  /**
   * Live path is Uint8Array; string is for tests. tag1 = 1 frame = 1 event, no buffering.
   * @param seq this frame's seq. Reattach reconciliation stands on it alone — if omitted, the frame's list events are not recorded and a held answer does not count it as delivered (tests only).
   * @returns whether the caller may release its waiting state. `false` when ① parse failed or unknown kind (nothing reflected in `snapshot`·`isTurnDone`)
   *   ② a tool end result (`ToolResult`): an earlier row's marker may change but no response arrived (ADR-0241).
   *   Callers must use this, not "a frame arrived", or an un-understood frame turns off the waiting indicator (`awaiting` of RichSlot).
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
      // Dumb pipe (unrefined): malformed JSON is protocol-level data loss — warn, then skip.
      console.warn('[structuredAccumulator] tag1 JSON parse failed — skipping the event:', err)
      return false
    }
    // Filter null/primitive: `consume` reading `ev.type` would throw into the subscription callback.
    if (parsed === null || typeof parsed !== 'object') {
      console.warn('[structuredAccumulator] tag1 payload is not an object — skipping the event:', parsed)
      return false
    }
    return this.consume(parsed as StructuredEvent, seq)
  }

  /** @returns same meaning as `feed` above. */
  private consume(ev: StructuredEvent, seq: number | undefined): boolean {
    switch (ev.type) {
      case 'TextDelta': {
        // Skip an empty delta so no phantom item (empty Markdown block · meaningless divider).
        if (!ev.text) break
        // copy-on-write: a returned snapshot() reference points at this object.
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
          // ADR-0239: the wire value may be an old daemon (no field) or a newer daemon (unknown word).
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
        // ★Does not close the turn — do not revive that★: not a boundary on either backend. codex emits retryable stream errors (overload · rate limit)
        //   with this and the turn continues (`error` in `backend/codex/decoder.rs`); claude always follows Error with MessageDone (`backend/claude/mod.rs`).
        //   Closing here would flicker the waiting indicator off during a retry.
        // ★Known hole★: wire `StructuredEvent::Error` has no retryable field, so an error that truly ended the turn with no boundary leaves the waiting indicator on.
        //   Do not tell them apart by string search — that is the decoder side's decision.
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
        // User uuid dedup (text blocks only; see `seenUserUuids`): the synthetic echo and the claude replay deliver the same uuid; skip an already seen one.
        //   Non-text blocks (tool_result) get null from extractUserUuid and are always kept. No uuid → kept.
        if (ev.kind === 'user') {
          const uuid = extractUserUuid(ev.json)
          if (uuid !== null) {
            // ADR-0231: the echo of a queued uuid is neither drawn nor marked seen — that bubble is drawn by `Delivered` of that id.
            //   The echo may arrive before `Delivered` (received here) or after it (the dedup below receives it).
            if (this.queued.row(uuid) !== undefined) break
            if (this.seenUserUuids.has(uuid)) break
            this.seenUserUuids.add(uuid)
          }
          // New user turn → turnDone=false (not idle): otherwise a follow-up send's synthetic echo releases awaiting and the waiting indicator (WaitRow)
          //   flickers off until the first assistant token. Replay idempotence holds: the final MessageDone sets it true again.
          this.turnDone = false
        }
        // Escape-hatch event — flow even an unknown kind (kind) through to prevent loss.
        this.items.push({ kind: 'structured', label: ev.kind, json: ev.json, itemId: this.nextId++ })
        break
      }
      case 'MessageDone':
        // ADR-0045: claude emits MessageDone once per turn with `turn_id` always None, so do not derive the boundary from turn_id.
        // Not the only boundary trigger: codex emits no MessageDone and closes with `TurnEnd` below (turn_id filled).
        this.closeTurn()
        break
      case 'TurnEnd':
        // All four outcomes close the turn; the outcome only decides the marker. Not closing leaves the waiting indicator spinning forever.
        {
          const mark = outcomeMark(ev.outcome)
          // ADR-0243: if an interrupt-notice row already told of the interrupt, the outcome row is not drawn on top.
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
        // ★Do not swallow unknown events silently★: Release WebView2 has no devtools, so leave an on-screen marker (same reason ConnectionNotice exists).
        // ★Do not carry the original payload · event name★ (protocol words stay off the user's screen — trd-phase2a §6-2); the name goes to the console only.
        // ★This item does not prop up the waiting indicator (do not revive that misunderstanding)★: the consumer derives `!turnDone && items.length>0`, so from the
        //   second turn on adding an item changes nothing. What guards it is the `false` return below.
        // ★Do not lower `turnDone`★: we do not know whether the unknown event ended the turn, and lowering it makes the waiting indicator spin forever
        //   when the last frame of a restored history is an unknown kind (a newer daemon's new turn-end vocabulary).
        // ★Known hole★: an unknown last frame after an unfinished turn leaves the waiting indicator on; the wire has no turn-state signal.
        // ★The `never` assignment is a compile-time defense★: new variants in this shell's bindings give a type error — add a proper arm. The runtime defense is below
        //   (a kind absent from the bindings entirely).
        const exhaustive: never = ev
        console.warn(
          '[structuredAccumulator] unknown StructuredEvent type — dropped, leaving only a marker:',
          (exhaustive as { type?: unknown }).type,
        )
        const last = this.items[this.items.length - 1]
        if (last && last.kind === 'unsupported') {
          // copy-on-write (same as the TextDelta arm).
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

  /** Close a turn boundary — 1 divider + `turnDone`. Consecutive ends do not stack dividers; no divider if no preceding item. */
  private closeTurn(): void {
    if (this.items.length > 0 && this.items[this.items.length - 1].kind !== 'separator') {
      this.items.push({ kind: 'separator', itemId: this.nextId++ })
    }
    this.turnDone = true
    this.turnBoundaries++
  }

  /**
   * Attach the tool end result as a marker to the earlier tool row of the same id; no new item. No row (pushed out of the ring) → discard.
   * ★Does not touch `turnDone` and always returns `false`★: an interrupted turn's tool ends after the turn end (maybe mid next turn);
   * releasing waiting here would turn off the new turn's waiting indicator without an answer.
   */
  // ADR-0241
  private consumeToolResult(ev: ToolResultEvent): boolean {
    // Filter so a `null` id of a malformed frame does not attach to a tool row with no id.
    if (typeof ev.id !== 'string') return false
    const mark = toolResultMark(ev.outcome)
    for (let i = this.items.length - 1; i >= 0; i--) {
      const item = this.items[i]
      if (item.kind !== 'tool' || item.id !== ev.id) continue
      // copy-on-write (same as the TextDelta arm).
      if (item.resultMark !== mark) this.items[i] = { ...item, resultMark: mark }
      return false
    }
    return false
  }

  /**
   * Queued-input registry event — after reduction only three draw rules change the conversation lines (`Delivered` placement · ascertained copy placement ·
   * `Rejected` bubble erase). Does not touch `turnDone` (list events are not output). A termination draws no notice row.
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
        '[structuredAccumulator] unknown QueuedInput kind — dropped:',
        (op as { kind?: unknown }).kind,
      )
      return false
    }
    // Reconciliation record: only events the reduction accepted (re-run through the same reducer at apply time).
    if (seq !== undefined && this.reconcile !== null) this.reconcile.log.push({ seq, op })
    switch (op.kind) {
      case 'Delivered':
        // An id not queued (revival · unknown id · second `Delivered`) is not drawn; a revival's bubble is drawn by the vendor echo.
        if (pending !== undefined) this.placeUserBubble(pending.id, pending.text)
        break
      case 'AckUnavailable':
        // Per copy (list order) — draw with the copy's body even if this window lost that `Queued` from the ring.
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

  /** Remove the user bubble (text block) drawn with that uuid. The uuid stays in 「seen」 (a refusal cannot be revived); a non-text block of the same line stays too. */
  private removeUserBubbles(id: string): void {
    for (let i = this.items.length - 1; i >= 0; i--) {
      const item = this.items[i]
      if (item.kind === 'structured' && item.label === 'user' && extractUserUuid(item.json) === id) {
        this.items.splice(i, 1)
      }
    }
  }

  /**
   * Items `QueuedInputList` draws — open items whose uuid is not yet drawn as a bubble, oldest first; a new array each time.
   * ★Cancel-pending rows are drawn too★ (ADR-0235): a ✕ row stays until an outcome event is reduced — hiding it early opens a window where
   *   text that could not be withdrawn is in neither list nor conversation.
   * The filter is drawing only; the registry's reduced state is `queuedRows()`.
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
   * Open a reattach reconciliation (TRD §5-7) — call **right before** sending the list query; it records list events received from now on
   * (events after the snapshot are all issued after the query read the registry). A reconciliation in progress is discarded.
   * @param epoch incarnation this view reads — discard if the answer's marker differs.
   * @returns generation of this query — pass it to `offerQueuedSnapshot`.
   */
  // ADR-0231
  beginQueuedReconcile(epoch: number): number {
    this.reconcileGen += 1
    this.reconcile = { gen: this.reconcileGen, epoch, log: [], held: null }
    return this.reconcileGen
  }

  /**
   * Hand over the list query answer. If generation and incarnation match, apply **after** the view has been delivered up to the snapshot seq: ids in the snapshot
   * restart from that row and re-reduce only recorded events with a larger seq; ids not in it close as current state (if after the snapshot) or with cause unknown
   * (`QueuedInputRegistry.adoptSnapshot`). Fixes only the list (draws no bubble). A row's unknown word drops only that row.
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
   * Discard the reconciliation in progress — when the view leaves `'live'` for a new replay cycle (`reset` calls this too). Raises the generation, so an old answer is `stale`.
   * @param gen if given, discard only when it is the current generation (on query failure, do not touch a newer reconciliation).
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

  /** Was the last signal a turn end (MessageDone · TurnEnd) — streaming/idle display hint. `Error` is not included (not a turn boundary on either backend). */
  isTurnDone(): boolean {
    return this.turnDone
  }

  /**
   * Count of turn boundaries consumed (MessageDone · TurnEnd, any outcome); `reset` does not roll it back — the difference before/after one `feed` means "that frame was a boundary".
   * ★Do not substitute `isTurnDone` going false → true★: a boundary received while already ended has no such transition.
   */
  turnBoundaryCount(): number {
    return this.turnBoundaries
  }

  /** Initialize before resubscribing (replay) so the whole history reconstructs to the same state. */
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
 * Outcome → on-screen marker. A normal completion leaves no marker (null): the divider alone.
 * `default` receives ① a fifth outcome a newer daemon added ② a missing/malformed outcome; both are recorded as 'unknown' — folding into a known one would draw a false outcome.
 * ★Nullish is accepted despite the `TurnOutcome` type★: the try/catch in `feed` wraps only JSON.parse, so a throw here rises to the subscription callback (RichSlot).
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

/** An unknown word (newer daemon) or broken shape means no marker — folding into a known one would draw a false outcome (same as `outcomeMark`). */
// ADR-0241: the argument is the generated `ToolOutcome`; changed words turn tsc red at the `never` in `default` (drift alarm). A newer daemon's unknown value falls into `default` at runtime.
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
      console.warn('[structuredAccumulator] unknown ToolResult outcome — left without a marker:', unknownWord)
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
 * Extract the dedup key `uuid` from the json of a user Structured item (synthetic echo · replay): `{"type":"text","text":…,"uuid":"X"}`.
 * null if no uuid or not a string (the caller keeps it without dedup).
 *
 * ★Only `type==="text"` blocks★: the backend decoder puts the same line-level uuid on **all content blocks** of one user replay line (claude.rs consume_block),
 * while the synthetic echo (user_text_echo_json) is a single text block. Returning the uuid for tool_result would skip it as "already seen" and lose the tool OUT body.
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
