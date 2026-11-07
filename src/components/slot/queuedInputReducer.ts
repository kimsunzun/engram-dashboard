// TS edition of the queued-input registry — reduces the ring's `QueuedInput` events in ring order and holds the list (items) and tombstones (pure TS).
//
// ★This is one set with the agent `queued_input.rs` reducer★: not a single rule is written separately. Both eat the golden
//   `crates/engram-dashboard-agent/src/queued_input_golden.json`, so if they diverge one side goes red —
//   when a rule is changed, fix the Rust edition · the golden · this file in the same change.
// ★It holds only reduction state — it knows nothing of bubbles★: the 「이미 그린 uuid」 (already drawn uuid) filter and bubble placement are the accumulator's drawing rules.
//   Doing that filter here would diverge from the registry and the golden.
// ★Rules stand on current state, not on history★ — the same event sequence always lands in the same state (replay idempotent).
// ADR-0231

import type { DeliveredCopy } from '../../../crates/engram-dashboard-protocol/bindings/DeliveredCopy'
import type { DropCause } from '../../../crates/engram-dashboard-protocol/bindings/DropCause'
import type { QueuedInputEvent } from '../../../crates/engram-dashboard-protocol/bindings/QueuedInputEvent'
import type { QueuedInputRow } from '../../../crates/engram-dashboard-protocol/bindings/QueuedInputRow'

/** Tombstone cap — on overflow the tombstone that went in first is dropped first (FIFO). ★Must equal `tombstone_cap` in the golden head★. */
// ADR-0231
export const TOMBSTONE_CAP = 1024

/**
 * The 「응답」 (answer) field of a cancel-pending item. `'none'` = no answer yet · `'not_removed'` = `removed:false` was received or
 * the request itself failed (`CancelFailed`). 「뺐다」 (it was removed) does not remain in the field — that answer closes it at once as cancelled.
 * The words are the same as `cancel.answer` of the golden and the list query row.
 */
export type CancelAnswer = 'none' | 'not_removed'

/**
 * Both are non-terminal. `vendorClosed` = the vendor's close (`Dropped{Unknown}`) of that id was already seen — whether it was our
 * cancel is told apart by `answer`.
 */
export type QueuedPhase =
  | { readonly state: 'queued' }
  | { readonly state: 'cancelling'; readonly answer: CancelAnswer; readonly vendorClosed: boolean }

/** One list line. The object is not mutated — when state changes it is replaced with a new object (the previous snapshot stays as it was). */
export interface QueuedEntry {
  readonly id: string
  readonly text: string
  readonly phase: QueuedPhase
  /**
   * The path handed it to the vendor (the value of the last `HandedOver`) — stands apart from the stage (✕ after the handover is a cancel-pending and handed-over row).
   * It is not an outcome — the row stays until receipt or withdrawal, and drawing hides the handed-over row's ✕.
   */
  readonly sent: boolean
}

/** The outcome where an id reached a terminal. The cause of `Discarded` is not `Withdrawn` (that cause is always `Cancelled`). */
export type QueuedVerdict =
  | { readonly kind: 'Delivered' }
  | { readonly kind: 'Cancelled' }
  | { readonly kind: 'Discarded'; readonly cause: DropCause }

export interface QueuedClosure {
  readonly id: string
  readonly verdict: QueuedVerdict
}

const DELIVERED: QueuedVerdict = { kind: 'Delivered' }
const CANCELLED: QueuedVerdict = { kind: 'Cancelled' }

function discarded(cause: DropCause): QueuedVerdict {
  return { kind: 'Discarded', cause }
}

/** ★Revival definition — one set★: only what closed with cause `Unknown` is a revivable tombstone (whichever event made that unknown). */
function resurrectable(verdict: QueuedVerdict): boolean {
  return verdict.kind === 'Discarded' && verdict.cause === 'Unknown'
}

function verdictOfDrop(cause: DropCause): QueuedVerdict {
  return cause === 'Withdrawn' ? CANCELLED : discarded(cause)
}

/**
 * List query row (wire `QueuedInputRow`) → one line of reduction state. `unconfirmed` is a marker overlaid at query time so in reduction it is
 * `queued` · `sent` is a `queued` with the handover marker standing (a cancel-pending row carries no marker — read as not handed over).
 * With an unknown word or a broken shape (a newer daemon) it is `null` — that row drops out of the check and that id stays in the accumulator's current
 * state as it is (it does not throw).
 */
// ADR-0231
export function entryOfListedRow(row: QueuedInputRow): QueuedEntry | null {
  if (row === null || typeof row !== 'object') return null
  const { id, text, state, cancel } = row
  if (typeof id !== 'string' || typeof text !== 'string') return null
  switch (state) {
    case 'queued':
    case 'unconfirmed':
      return { id, text, phase: { state: 'queued' }, sent: false }
    case 'sent':
      return { id, text, phase: { state: 'queued' }, sent: true }
    case 'cancelling':
      if (cancel === null || typeof cancel !== 'object') return null
      if (cancel.answer !== 'none' && cancel.answer !== 'not_removed') return null
      if (typeof cancel.vendor_closed !== 'boolean') return null
      return {
        id,
        text,
        phase: { state: 'cancelling', answer: cancel.answer, vendorClosed: cancel.vendor_closed },
        sent: false,
      }
    default:
      return null
  }
}

/**
 * The registry body. The place of one id is one of three — an **item** (`rows`) · a **tombstone** (terminal — a single 「되살림 가능」 (revivable)
 * mark with no body) · **unknown**.
 */
export class QueuedInputRegistry {
  private items: QueuedEntry[] = []
  // Tombstone = id → revivable. ★The Map's insertion order is the FIFO order★ — setting an existing key does not move its place.
  private readonly tombstones = new Map<string, boolean>()
  // The 「받음 불가 판명」 (found unable to receive) marker. ★The reduction rule does not read this field★ (the golden measures this field too).
  private ackUnavailable = false

  /**
   * Reduces one event. Return = the ids whose outcome stood from this event and that outcome (in reduction order).
   * ★It does not return only item closes★ — it also returns a terminal that left only a tombstone for an unknown id, a revival (`Delivered`), and the re-reading of
   * a rejection after receipt (`Discarded`·`Rejected`). Whether an item existed is told by the caller from the state before reduction.
   * @returns `null` = an unknown `kind` (a newer daemon) — the state was not touched.
   */
  reduce(op: QueuedInputEvent): QueuedClosure[] | null {
    const closed: QueuedClosure[] = []
    switch (op.kind) {
      case 'Queued':
        this.onQueued(op.id, op.text)
        break
      case 'CancelRequested':
        this.onCancelRequested(op.id)
        break
      case 'CancelAnswered':
        if (op.removed) this.onRemoved(op.id, closed)
        else this.onNotRemoved(op.id, closed)
        break
      case 'CancelFailed':
        this.onNotRemoved(op.id, closed)
        break
      case 'HandedOver':
        this.onHandedOver(op.id, op.sent)
        break
      case 'Delivered':
        this.onDelivered(op.id, closed)
        break
      case 'Dropped':
        this.onDropped(op.id, op.cause, closed)
        break
      case 'AckUnavailable':
        this.onAckUnavailable(op.delivered, closed)
        break
      default: {
        // `never` assignment = if variants grow in this shell's bindings, compilation breaks here (the same discipline as the accumulator's default arm).
        const exhaustive: never = op
        void exhaustive
        return null
      }
    }
    return closed
  }

  /** Open items (in the order held — oldest first). Cancel-pending items are included. */
  rows(): readonly QueuedEntry[] {
    return this.items
  }

  row(id: string): QueuedEntry | undefined {
    return this.items.find((entry) => entry.id === id)
  }

  ackUnavailableSeen(): boolean {
    return this.ackUnavailable
  }

  /** The tombstone of that id. `true` = revivable · `undefined` = no tombstone (an item or unknown). */
  tombstone(id: string): boolean | undefined {
    return this.tombstones.get(id)
  }

  tombstoneCount(): number {
    return this.tombstones.size
  }

  /** Clears everything including tombstones (accumulator `reset()` — before the full ring replay). */
  clear(): void {
    this.items = []
    this.tombstones.clear()
    this.ackUnavailable = false
  }

  /**
   * Re-attach check — for each snapshot row, starting from that state, re-reduces only the events of that id among `later` (events **after** the snapshot seq, in ring order) and
   * `AckUnavailable`, and replaces that id's place in this registry with the result (open = item · closed = tombstone).
   * An open item not in the snapshot is left as is if `later` has a `Queued` for that id (it stood after the snapshot), and otherwise is closed
   * with unknown cause (a revivable tombstone — body below).
   * ★Do not put events with seq ≤ snapshot seq in `later`★ — they are already in the snapshot and would be reduced twice.
   * Order = the remaining snapshot rows (daemon registry order), then the rest (items that stood after the snapshot). It knows nothing of bubbles (it fixes only the list).
   * @param listed ids of **all** rows carried in the answer — including the ids of rows `rows` filtered out (unknown words). Those ids are
   *   counted as 「있다」 (present) and not closed (an unknown word is not an outcome).
   */
  // ADR-0231
  adoptSnapshot(
    rows: readonly QueuedEntry[],
    later: readonly QueuedInputEvent[],
    listed: ReadonlySet<string>,
  ): void {
    // ADR-0231: TRD edition 8 §5-7 put it as 「스냅숏에 없는 id 는 누산기가 이미 봤다」 (an id not in the snapshot was already seen by the accumulator) (edition 9 erased it and changed to this rule — the TRD §5-7 「9판」 line), but a head jump breaks that premise —
    //   if flush skips the cursor to the head of a later replay (same-incarnation reattach · re-buffer after hold overflow), the accumulator is not cleared and
    //   only the close event is pushed out of the ring and lost. The basis of this decision is that items open at the snapshot seq are in the snapshot,
    //   so with no jump the result is the same as today.
    const queuedLater = new Set<string>()
    for (const op of later) if (op.kind === 'Queued') queuedLater.add(op.id)
    this.items = this.items.filter((entry) => {
      if (listed.has(entry.id) || queuedLater.has(entry.id)) return true
      this.entomb(entry.id, true)
      return false
    })
    const adopted: QueuedEntry[] = []
    for (const row of rows) {
      // Reduction rules stand per id (only `AckUnavailable` applies to all) — re-run with a one-row registry.
      const scratch = new QueuedInputRegistry()
      scratch.items = [row]
      for (const op of later) {
        if (op.kind === 'AckUnavailable' || op.id === row.id) scratch.reduce(op)
      }
      const at = this.position(row.id)
      if (at !== -1) this.items.splice(at, 1)
      const left = scratch.row(row.id)
      if (left !== undefined) {
        this.tombstones.delete(row.id)
        adopted.push(left)
      } else {
        const mark = scratch.tombstone(row.id)
        if (mark !== undefined) this.entomb(row.id, mark)
      }
    }
    this.items = [...adopted, ...this.items]
  }

  private position(id: string): number {
    return this.items.findIndex((entry) => entry.id === id)
  }

  private setPhase(at: number, phase: QueuedPhase): void {
    this.items[at] = { ...this.items[at], phase }
  }

  /** Deletes the item and folds it into a tombstone — the body is not kept. */
  private close(at: number, verdict: QueuedVerdict, closed: QueuedClosure[]): void {
    const [entry] = this.items.splice(at, 1)
    this.bury(entry.id, resurrectable(verdict))
    closed.push({ id: entry.id, verdict })
  }

  /** ★It is a set★ — an id already there is a no-op and does not move its place. */
  private bury(id: string, canRevive: boolean): void {
    if (this.tombstones.has(id)) return
    if (this.tombstones.size >= TOMBSTONE_CAP) {
      const oldest = this.tombstones.keys().next()
      if (!oldest.done) this.tombstones.delete(oldest.value)
    }
    this.tombstones.set(id, canRevive)
  }

  /** A tombstone with a known outcome — if already there only the revival mark is fixed (the place stays — so the FIFO order does not shake). */
  private entomb(id: string, canRevive: boolean): void {
    if (this.tombstones.has(id)) this.tombstones.set(id, canRevive)
    else this.bury(id, canRevive)
  }

  private clearResurrectable(id: string): void {
    if (this.tombstones.has(id)) this.tombstones.set(id, false)
  }

  private onQueued(id: string, text: string): void {
    // If already an item, a no-op (no re-emit) · if a tombstone, dropped (so a late `Queued` does not create a permanent item).
    if (this.position(id) !== -1 || this.tombstones.has(id)) return
    this.items.push({ id, text, phase: { state: 'queued' }, sent: false })
  }

  /** Handover marker — if an item, only the marker is swapped regardless of stage (no outcome). An unknown id · tombstone is a no-op (no row is invented). */
  private onHandedOver(id: string, sent: boolean): void {
    const at = this.position(id)
    if (at !== -1) this.items[at] = { ...this.items[at], sent }
  }

  private onCancelRequested(id: string): void {
    const at = this.position(id)
    if (at !== -1 && this.items[at].phase.state === 'queued') {
      this.setPhase(at, { state: 'cancelling', answer: 'none', vendorClosed: false })
    }
  }

  /** `removed:true` — closes without waiting for the lifecycle line (if that line never comes the item stays in the list). */
  private onRemoved(id: string, closed: QueuedClosure[]): void {
    const at = this.position(id)
    if (at !== -1 && this.items[at].phase.state === 'cancelling') this.close(at, CANCELLED, closed)
  }

  /**
   * `removed:false` · request failure. ★It is not put back to waiting★ — the answer field (`not_removed`) tells the cause of a following vendor close apart
   * (drawing draws this row like a handed-over row with no ✕ — ADR-0235).
   */
  private onNotRemoved(id: string, closed: QueuedClosure[]): void {
    const at = this.position(id)
    if (at === -1) return
    const phase = this.items[at].phase
    if (phase.state === 'queued') return
    if (phase.vendorClosed) {
      // The vendor had already closed it but it was not our cancel.
      this.close(at, discarded('Unknown'), closed)
    } else {
      this.setPhase(at, { state: 'cancelling', answer: 'not_removed', vendorClosed: false })
    }
  }

  private onDelivered(id: string, closed: QueuedClosure[]): void {
    const at = this.position(id)
    if (at !== -1) {
      this.close(at, DELIVERED, closed)
      return
    }
    const mark = this.tombstones.get(id)
    if (mark === undefined) {
      this.bury(id, false)
      closed.push({ id, verdict: DELIVERED })
    } else if (mark) {
      // A vendor receipt came belatedly for an item dropped as 「모름」 (unknown) (revival).
      this.clearResurrectable(id)
      closed.push({ id, verdict: DELIVERED })
    }
    // A non-revivable tombstone — a second `Delivered` of the same id (codex `item/started`·`item/completed`) also ends here.
  }

  private onDropped(id: string, cause: DropCause, closed: QueuedClosure[]): void {
    const at = this.position(id)
    if (at !== -1) {
      const phase = this.items[at].phase
      if (cause === 'Withdrawn') {
        this.close(at, CANCELLED, closed)
      } else if (phase.state === 'queued') {
        this.close(at, discarded(cause), closed)
      } else if (cause !== 'Unknown') {
        this.close(at, discarded(cause), closed)
      } else if (phase.vendorClosed) {
        // The vendor close was already seen — a second close has no new fact.
      } else if (phase.answer === 'not_removed') {
        // A failed or interrupted turn closed it — our cancel already answered 「못 뺐다」 (could not remove).
        this.close(at, discarded('Unknown'), closed)
      } else {
        // If our cancel removed it, that answer follows — leave it as cancel-pending and wait.
        this.setPhase(at, { state: 'cancelling', answer: 'none', vendorClosed: true })
      }
      return
    }
    const mark = this.tombstones.get(id)
    if (mark === undefined) {
      const verdict = verdictOfDrop(cause)
      this.bury(id, resurrectable(verdict))
      closed.push({ id, verdict })
    } else if (cause === 'Rejected') {
      // Rejection after receipt — re-read the outcome and block revival.
      this.clearResurrectable(id)
      closed.push({ id, verdict: discarded('Rejected') })
    }
  }

  /**
   * Closes all open items with this ring place's receipt (cancel-pending ones too do not wait for an answer). The copy's ids are then reduced
   * the same as `Delivered` — an item just closed is a no-op · an unknown id gets only a tombstone.
   */
  private onAckUnavailable(delivered: readonly DeliveredCopy[], closed: QueuedClosure[]): void {
    this.ackUnavailable = true
    while (this.items.length > 0) this.close(0, DELIVERED, closed)
    for (const copy of delivered) this.onDelivered(copy.id, closed)
  }
}
