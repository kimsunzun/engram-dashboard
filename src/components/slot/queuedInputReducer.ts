// 대기 입력 명부의 TS 판 — 링의 `QueuedInput` 사건을 링 순서대로 환원해 목록(항목)과 묘비를 쥔다(순수 TS).
//
// ★agent `queued_input.rs` 환원기와 한 벌이다★: 규칙을 한 줄도 따로 짓지 않는다. 둘 다 골든
//   `crates/engram-dashboard-agent/src/queued_input_golden.json` 을 먹으므로 갈라지면 한쪽이 빨개진다 —
//   규칙을 고치면 Rust 판·골든·이 파일을 같은 변경에서 고친다.
// ★환원 상태만 쥔다 — 말풍선은 모른다★: 「이미 그린 uuid」 거름·말풍선 배치는 누산기의 그리기 규칙이다.
//   여기서 그 거름을 하면 명부·골든과 갈린다.
// ★규칙은 이력이 아니라 지금 상태로 선다★ — 같은 사건열은 늘 같은 상태로 떨어진다(replay 멱등).
// ADR-0231

import type { DeliveredCopy } from '../../../crates/engram-dashboard-protocol/bindings/DeliveredCopy'
import type { DropCause } from '../../../crates/engram-dashboard-protocol/bindings/DropCause'
import type { QueuedInputEvent } from '../../../crates/engram-dashboard-protocol/bindings/QueuedInputEvent'
import type { QueuedInputRow } from '../../../crates/engram-dashboard-protocol/bindings/QueuedInputRow'

/** 묘비 상한 — 넘치면 가장 먼저 든 묘비부터 버린다(FIFO). ★골든 머리의 `tombstone_cap` 과 같아야 한다★. */
// ADR-0231
export const TOMBSTONE_CAP = 1024

/**
 * 취소 대기 항목의 「응답」 칸. `'none'` = 아직 응답이 없다 · `'not_removed'` = `removed:false` 를 받았거나
 * 요청 자체가 실패했다(`CancelFailed`). 「뺐다」는 칸에 남지 않는다 — 그 응답이 곧바로 취소됨으로 닫는다.
 * 낱말은 골든·목록 조회 행의 `cancel.answer` 와 같다.
 */
export type CancelAnswer = 'none' | 'not_removed'

/**
 * 둘 다 비종결이다. `vendorClosed` = 그 id 의 벤더 닫힘(`Dropped{Unknown}`)을 이미 봤다 — 그것이 우리
 * 취소였는지는 `answer` 가 가른다.
 */
export type QueuedPhase =
  | { readonly state: 'queued' }
  | { readonly state: 'cancelling'; readonly answer: CancelAnswer; readonly vendorClosed: boolean }

/** 목록 한 줄. 객체는 바꾸지 않는다 — 상태가 바뀌면 새 객체로 갈아 끼운다(이전 스냅숏이 그대로 남는다). */
export interface QueuedEntry {
  readonly id: string
  readonly text: string
  readonly phase: QueuedPhase
}

/** 한 id 가 종결에 닿은 결말. `Discarded` 의 원인은 `Withdrawn` 이 아니다(그 원인은 늘 `Cancelled`). */
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

/** ★되살림 정의 — 한 벌★: 원인 `Unknown` 으로 닫힌 것만 되살림 가능 묘비다(어느 사건이 그 모름을 만들었든). */
function resurrectable(verdict: QueuedVerdict): boolean {
  return verdict.kind === 'Discarded' && verdict.cause === 'Unknown'
}

function verdictOfDrop(cause: DropCause): QueuedVerdict {
  return cause === 'Withdrawn' ? CANCELLED : discarded(cause)
}

/**
 * 목록 조회 행(wire `QueuedInputRow`) → 환원 상태 한 줄. `unconfirmed` 는 조회 순간 덧댄 표지라 환원으로는
 * `queued` 다. 모르는 낱말·깨진 모양(더 새 데몬)이면 `null` — 그 행은 대조에서 빠지고 그 id 는 누산기의 지금
 * 상태 그대로 남는다(던지지 않는다).
 */
// ADR-0231
export function entryOfListedRow(row: QueuedInputRow): QueuedEntry | null {
  if (row === null || typeof row !== 'object') return null
  const { id, text, state, cancel } = row
  if (typeof id !== 'string' || typeof text !== 'string') return null
  switch (state) {
    case 'queued':
    case 'unconfirmed':
      return { id, text, phase: { state: 'queued' } }
    case 'cancelling':
      if (cancel === null || typeof cancel !== 'object') return null
      if (cancel.answer !== 'none' && cancel.answer !== 'not_removed') return null
      if (typeof cancel.vendor_closed !== 'boolean') return null
      return {
        id,
        text,
        phase: { state: 'cancelling', answer: cancel.answer, vendorClosed: cancel.vendor_closed },
      }
    default:
      return null
  }
}

/**
 * 명부 본체. id 하나의 자리는 셋 중 하나다 — **항목**(`rows`) · **묘비**(종결 — 본문 없이 「되살림 가능」
 * 표시 하나) · **모름**.
 */
export class QueuedInputRegistry {
  private items: QueuedEntry[] = []
  // 묘비 = id → 되살림 가능. ★Map 의 삽입 순서가 곧 FIFO 순서다★ — 이미 있는 키에 set 해도 자리가 안 옮겨진다.
  private readonly tombstones = new Map<string, boolean>()
  // 「받음 불가 판명」 표식. ★환원 규칙은 이 칸을 읽지 않는다★(골든이 이 칸도 잰다).
  private ackUnavailable = false

  /**
   * 사건 하나를 환원한다. 반환 = 이 사건으로 결말이 선 id 와 그 결말(환원 순서).
   * ★항목이 닫힌 것만 돌려주지 않는다★ — 모르는 id 에 묘비만 남긴 종결, 되살림(`Delivered`), 받음 뒤
   * 거절의 고쳐 읽기(`Discarded`·`Rejected`)도 돌려준다. 항목이 있었는지는 호출자가 환원 전 상태로 가른다.
   * @returns `null` = 모르는 `kind`(더 새 데몬) — 상태를 건드리지 않았다.
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
        // `never` 대입 = 이 셸의 bindings 에 변형이 늘면 컴파일이 여기서 깨진다(누산기 default arm 과 같은 규율).
        const exhaustive: never = op
        void exhaustive
        return null
      }
    }
    return closed
  }

  /** 열린 항목(든 순서 — 가장 오래된 것이 앞). 취소 대기 항목도 든다. */
  rows(): readonly QueuedEntry[] {
    return this.items
  }

  row(id: string): QueuedEntry | undefined {
    return this.items.find((entry) => entry.id === id)
  }

  ackUnavailableSeen(): boolean {
    return this.ackUnavailable
  }

  /** 그 id 의 묘비. `true` = 되살림 가능 · `undefined` = 묘비가 없다(항목이거나 모름). */
  tombstone(id: string): boolean | undefined {
    return this.tombstones.get(id)
  }

  tombstoneCount(): number {
    return this.tombstones.size
  }

  /** 묘비까지 전부 비운다(누산기 `reset()` — 링 전량 재생 전). */
  clear(): void {
    this.items = []
    this.tombstones.clear()
    this.ackUnavailable = false
  }

  /**
   * 재부착 대조 — 스냅숏 행마다 그 상태에서 출발해 `later`(스냅숏 seq **뒤**의 사건, 링 순서) 중 그 id 의 것과
   * `AckUnavailable` 만 다시 환원하고, 그 결과로 이 명부의 그 id 자리를 갈아 끼운다(열림 = 항목 · 닫힘 = 묘비).
   * 스냅숏에 없는 열린 항목은 `later` 에 그 id 의 `Queued` 가 있으면(스냅숏 뒤에 섰다) 그대로 두고, 없으면
   * 원인 모름으로 닫는다(되살림 가능 묘비 — 아래 본문).
   * ★seq ≤ 스냅숏 seq 인 사건을 `later` 에 넣지 말 것★ — 스냅숏에 이미 들어 있어 두 번 환원된다.
   * 순서 = 남은 스냅숏 행(데몬 명부 순) 다음에 나머지(스냅숏 뒤에 선 항목). 말풍선은 모른다(목록만 고친다).
   * @param listed 답에 실린 **모든** 행의 id — `rows` 가 거른 행(모르는 낱말)의 id 도 든다. 그 id 는 「있다」로
   *   세어 닫지 않는다(모르는 낱말이지 결말이 아니다).
   */
  // ADR-0231
  adoptSnapshot(
    rows: readonly QueuedEntry[],
    later: readonly QueuedInputEvent[],
    listed: ReadonlySet<string>,
  ): void {
    // ADR-0231: TRD L627 은 「스냅숏에 없는 id 는 누산기가 이미 봤다」라 두지만 머리 점프가 그 전제를 깬다 —
    //   flush 가 커서를 뒤의 replay 머리로 건너뛰면(같은 화신 재부착 · 붙듦 넘침 재버퍼) 누산기는 비우지 않은 채
    //   닫힘 사건만 링에서 밀려나 잃는다. 스냅숏 seq 에 열려 있던 항목은 스냅숏에 든다는 것이 이 판정의 근거라,
    //   점프가 없으면 오늘과 같은 결과다.
    const queuedLater = new Set<string>()
    for (const op of later) if (op.kind === 'Queued') queuedLater.add(op.id)
    this.items = this.items.filter((entry) => {
      if (listed.has(entry.id) || queuedLater.has(entry.id)) return true
      this.entomb(entry.id, true)
      return false
    })
    const adopted: QueuedEntry[] = []
    for (const row of rows) {
      // 환원 규칙은 id 마다 따로 선다(모두에 걸리는 것은 `AckUnavailable` 하나) — 한 행짜리 명부로 다시 돌린다.
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

  /** 항목을 지우고 묘비로 접는다 — 본문은 쥐지 않는다. */
  private close(at: number, verdict: QueuedVerdict, closed: QueuedClosure[]): void {
    const [entry] = this.items.splice(at, 1)
    this.bury(entry.id, resurrectable(verdict))
    closed.push({ id: entry.id, verdict })
  }

  /** ★집합이다★ — 이미 있는 id 는 무동작이고 자리도 안 옮긴다. */
  private bury(id: string, canRevive: boolean): void {
    if (this.tombstones.has(id)) return
    if (this.tombstones.size >= TOMBSTONE_CAP) {
      const oldest = this.tombstones.keys().next()
      if (!oldest.done) this.tombstones.delete(oldest.value)
    }
    this.tombstones.set(id, canRevive)
  }

  /** 결말을 아는 묘비 — 이미 있으면 되살림 표시만 고친다(자리는 그대로 — FIFO 순서가 안 흔들린다). */
  private entomb(id: string, canRevive: boolean): void {
    if (this.tombstones.has(id)) this.tombstones.set(id, canRevive)
    else this.bury(id, canRevive)
  }

  private clearResurrectable(id: string): void {
    if (this.tombstones.has(id)) this.tombstones.set(id, false)
  }

  private onQueued(id: string, text: string): void {
    // 이미 항목이면 무동작(재방출 없음) · 묘비면 버린다(늦은 `Queued` 가 영구 항목을 만들지 않게).
    if (this.position(id) !== -1 || this.tombstones.has(id)) return
    this.items.push({ id, text, phase: { state: 'queued' } })
  }

  private onCancelRequested(id: string): void {
    const at = this.position(id)
    if (at !== -1 && this.items[at].phase.state === 'queued') {
      this.setPhase(at, { state: 'cancelling', answer: 'none', vendorClosed: false })
    }
  }

  /** `removed:true` — 수명주기 줄을 기다리지 않고 닫는다(그 줄이 끝내 안 오면 항목이 목록을 쥔 채 남는다). */
  private onRemoved(id: string, closed: QueuedClosure[]): void {
    const at = this.position(id)
    if (at !== -1 && this.items[at].phase.state === 'cancelling') this.close(at, CANCELLED, closed)
  }

  /** `removed:false` · 요청 실패. ★목록으로 되돌리지 않는다★ — 되돌리면 ✕ 로 모든 창에서 빠진 항목이 다시 그려진다. */
  private onNotRemoved(id: string, closed: QueuedClosure[]): void {
    const at = this.position(id)
    if (at === -1) return
    const phase = this.items[at].phase
    if (phase.state === 'queued') return
    if (phase.vendorClosed) {
      // 벤더가 이미 닫았는데 우리 취소가 아니었다.
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
      // 「모름」으로 버린 항목에 벤더 받음이 뒤늦게 왔다(되살림).
      this.clearResurrectable(id)
      closed.push({ id, verdict: DELIVERED })
    }
    // 되살림 불가 묘비 — 같은 id 의 둘째 `Delivered`(codex `item/started`·`item/completed`)도 여기서 끝난다.
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
        // 벤더 닫힘을 이미 봤다 — 둘째 닫힘은 새 사실이 없다.
      } else if (phase.answer === 'not_removed') {
        // 실패·끊긴 턴이 닫았다 — 우리 취소는 이미 「못 뺐다」로 답했다.
        this.close(at, discarded('Unknown'), closed)
      } else {
        // 우리 취소가 뺐다면 그 응답이 뒤따른다 — 취소 대기 그대로 두고 기다린다.
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
      // 받음 뒤 거절 — 결말을 고쳐 읽고 되살림을 막는다.
      this.clearResurrectable(id)
      closed.push({ id, verdict: discarded('Rejected') })
    }
  }

  /**
   * 열린 항목 전부를 이 링 자리의 받음으로 닫는다(취소 대기도 응답을 기다리지 않는다). 사본의 id 는 그 뒤
   * `Delivered` 와 같게 환원한다 — 방금 닫힌 항목은 무동작 · 모르는 id 는 묘비만.
   */
  private onAckUnavailable(delivered: readonly DeliveredCopy[], closed: QueuedClosure[]): void {
    this.ackUnavailable = true
    while (this.items.length > 0) this.close(0, DELIVERED, closed)
    for (const copy of delivered) this.onDelivered(copy.id, closed)
  }
}
