// TS edition of the queued-input registry: reduces the ring's `QueuedInput` events in ring order, holding the list (items) and tombstones. Pure.
// ★One set with the agent `queued_input.rs` reducer★: both eat the golden `crates/engram-dashboard-agent/src/queued_input_golden.json` — change a rule in the Rust edition · golden · this file together.
// ★Reduction state only — no bubbles★: the "already drawn uuid" filter and bubble placement are the accumulator's; doing the filter here would diverge from the registry and golden.
// ★Rules stand on current state, not history★ — the same event sequence always lands in the same state (replay idempotent).
// ADR-0231

import type { DeliveredCopy } from '../../../crates/engram-dashboard-protocol/bindings/DeliveredCopy'
import type { DropCause } from '../../../crates/engram-dashboard-protocol/bindings/DropCause'
import type { QueuedInputEvent } from '../../../crates/engram-dashboard-protocol/bindings/QueuedInputEvent'
import type { QueuedInputRow } from '../../../crates/engram-dashboard-protocol/bindings/QueuedInputRow'

/** Tombstone cap — on overflow the tombstone that went in first is dropped first (FIFO). ★Must equal `tombstone_cap` in the golden head★. */
// ADR-0231
export const TOMBSTONE_CAP = 1024

/**
 * Answer field of a cancel-pending item: `'none'` = no answer yet; `'not_removed'` = `removed:false` received or the request failed (`CancelFailed`). "Removed" never stays in the field — it closes the item as cancelled. Same words as `cancel.answer` of the golden and the list query row.
 */
export type CancelAnswer = 'none' | 'not_removed'

/** Both non-terminal. `vendorClosed` = the vendor's close (`Dropped{Unknown}`) of that id was already seen; whether it was our cancel is told by `answer`. */
export type QueuedPhase =
  | { readonly state: 'queued' }
  | { readonly state: 'cancelling'; readonly answer: CancelAnswer; readonly vendorClosed: boolean }

/** One list line. The object is not mutated — when state changes it is replaced with a new object (the previous snapshot stays as it was). */
export interface QueuedEntry {
  readonly id: string
  readonly text: string
  readonly phase: QueuedPhase
  /** Handed to the vendor (last `HandedOver` value), independent of stage. Not an outcome — the row stays until receipt or withdrawal; drawing hides the handed-over row's ✕. */
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
 * List query row (wire `QueuedInputRow`) → one reduction line. `unconfirmed` is a query-time overlay and reduces as `queued`; `sent` = `queued` with the handover marker (a cancel-pending row carries none).
 * Unknown word or broken shape (newer daemon) → `null`: the row drops out and that id stays as in the accumulator's state (no throw).
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

/** The registry body. One id's place is an **item** (`rows`) · a **tombstone** (terminal; one "revivable" mark, no body) · **unknown**. */
export class QueuedInputRegistry {
  private items: QueuedEntry[] = []
  // Tombstone = id → revivable. ★The Map's insertion order is the FIFO order★ — setting an existing key does not move its place.
  private readonly tombstones = new Map<string, boolean>()
  // "Found unable to receive" marker; ★the reduction rule does not read it★ (the golden measures it).
  private ackUnavailable = false

  /**
   * Reduces one event. Returns the ids whose outcome stood from this event, with that outcome (reduction order). ★Not only item closes★: also a tombstone left for an unknown id, a revival (`Delivered`), and re-reading of a rejection after receipt (`Discarded`·`Rejected`) — the caller tells whether an item existed from the pre-reduction state.
   * @returns `null` = unknown `kind` (newer daemon) — state untouched.
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
        // `never` assignment: compilation breaks here if variants grow in the bindings.
        const exhaustive: never = op
        void exhaustive
        return null
      }
    }
    return closed
  }

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
   * Re-attach check: per snapshot row, re-reduce from its state only the events of that id among `later` (events **after** the snapshot seq, ring order) plus `AckUnavailable`, and replace that id's place with the result (open = item, closed = tombstone). An open item not in the snapshot stays if `later` has its `Queued`, otherwise it closes with unknown cause (revivable tombstone).
   * ★Do not put events with seq ≤ snapshot seq in `later`★ — they would be reduced twice. Order = remaining snapshot rows (daemon registry order), then items that stood after the snapshot. Bubbles are not touched.
   * @param listed ids of **all** rows in the answer, including rows `rows` filtered out (unknown words) — counted as present, not closed.
   */
  // ADR-0231
  adoptSnapshot(
    rows: readonly QueuedEntry[],
    later: readonly QueuedInputEvent[],
    listed: ReadonlySet<string>,
  ): void {
    // ADR-0231: an id missing from the snapshot is NOT assumed already seen by the accumulator (TRD §5-7 edition 8 said so; edition 9 dropped it) — a head jump (flush skips the cursor to a later replay head: same-incarnation reattach · re-buffer after hold overflow) leaves the accumulator uncleared with only the close event pushed out of the ring. Items open at the snapshot seq are in the snapshot, so without a jump the result is unchanged.
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

  /** `removed:false` · request failure. ★Not put back to waiting★ — the answer field (`not_removed`) tells a following vendor close's cause (drawn like a handed-over row without ✕ — ADR-0235). */
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
      } else if (phase.answer === 'not_removed') {
        // A failed or interrupted turn closed it — our cancel already answered 「could not remove」.
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
      this.clearResurrectable(id)
      closed.push({ id, verdict: discarded('Rejected') })
    }
  }

  /** Closes all open items with this ring place's receipt (cancel-pending too, without waiting for an answer); the copy's ids are then reduced like `Delivered` — an item just closed is a no-op, an unknown id gets a tombstone. */
  private onAckUnavailable(delivered: readonly DeliveredCopy[], closed: QueuedClosure[]): void {
    this.ackUnavailable = true
    while (this.items.length > 0) this.close(0, DELIVERED, closed)
    for (const copy of delivered) this.onDelivered(copy.id, closed)
  }
}
