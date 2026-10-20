// TRD S21-slot-docking §4-5: 셸 `layout::dock::{docked_tree, incoming_rect}` · `layout::geometry::{compute, boundary, px_edge,
//   frame_px}` 의 거울 — 끄는 동안 끌기는 판정을 셸과 맞추지 않아(ADR-0296 · ADR-0297) 띠 유효성(R8) · 미리보기(R4)를 웹이 혼자
//   센다. 순수하다.
// ★셸과 같은 f64 연산을 같은 순서로 한다★ — 경계는 부모 값을 복사하고 새로 셈하는 값은 분할 경계 하나뿐이다(셸 `geometry.rs`
//   머리 불변식). 그래야 미리보기가 놓은 뒤 셸이 그릴 칸과 비트 단위로 같고, 깊은 사슬의 면적 0 판정이 셸과 같은 칸에서 선다.
//   두 판은 골든 `src-tauri/src/layout/bands_golden.json` 이 묶는다(`bands.test.ts` · `judge.test.ts`).
// 비율 한계는 스냅숏 `ratio_min` · `ratio_max` 로 받는다(ADR-0227 — 화면이 상수를 따로 두지 않는다).

import type { LayoutNode, SlotRect } from '../../../api/layoutTypes'
import type { DockSide } from '../../../../src-tauri/bindings/DockSide'
import type { DropAnchor } from '../../../../src-tauri/bindings/DropAnchor'

/** 분할 비율 한계 — 스냅숏 `ratio_min` · `ratio_max`(셸 `tree::RATIO_MIN` · `RATIO_MAX`). */
export interface RatioLimits {
  min: number
  max: number
}

/** 캔버스 크기 — 정수 CSS px. */
export interface CanvasPx {
  w: number
  h: number
}

/** 정수 CSS px 경계 꼴 사각형(캔버스 원점) — 반열림. 셸 `geometry::PxRect`. */
export interface PxRect {
  x0: number
  y0: number
  x1: number
  y1: number
}

/** 정규화 경계 꼴 사각형 — 셸 `geometry::RectF64`. */
export interface RectF64 {
  x0: number
  y0: number
  x1: number
  y1: number
}

/** 셸 `DockTreeError` — 셸이 보고하는 차례(이 차례로 처음 걸린 것). */
export type DockTreeError = 'slot-not-found' | 'slot-id-overlap' | 'zero-area'

export type DockedTree = { ok: true; tree: LayoutNode } | { ok: false; error: DockTreeError }

const U32_MAX = 4294967295

/** 셸 `f64 as u32` — 소수는 버리고 NaN · 음수는 0, 넘치면 `u32::MAX`. */
export function toU32(v: number): number {
  if (!(v > 0)) return 0
  return v > U32_MAX ? U32_MAX : Math.trunc(v)
}

/** 셸 `tree::clamp_ratio` — NaN 은 그대로 NaN(거르는 것은 `boundary`). */
function clampRatio(r: number, limits: RatioLimits): number {
  return Math.min(Math.max(r, limits.min), limits.max)
}

/** 셸 `geometry::boundary` — 축 구간 `[lo, hi]` 를 `ratio`(a 몫)로 나누는 경계. NaN 비율은 반분으로 읽는다. */
export function boundary(lo: number, hi: number, ratio: number, limits: RatioLimits): number {
  const t = Number.isNaN(ratio) ? 0.5 : clampRatio(ratio, limits)
  return lo + (hi - lo) * t
}

/** 셸 `geometry::compute` 의 칸 몫 — 트리 전위 순 칸 사각형(루트 = (0,0,1,1)). 스냅숏 `slot_rects` 와 같은 값이다. */
export function computeSlots(node: LayoutNode, limits: RatioLimits): SlotRect[] {
  const out: SlotRect[] = []
  const walk = (n: LayoutNode, r: RectF64): void => {
    if (n.type === 'slot') {
      out.push({ slot_id: n.id, x0: r.x0, y0: r.y0, x1: r.x1, y1: r.y1 })
      return
    }
    // ADR-0140: left_right = x 축 분할(a 왼쪽) · top_bottom = y 축 분할(a 위).
    if (n.dir === 'left_right') {
      const at = boundary(r.x0, r.x1, n.ratio, limits)
      walk(n.a, { ...r, x1: at })
      walk(n.b, { ...r, x0: at })
    } else {
      const at = boundary(r.y0, r.y1, n.ratio, limits)
      walk(n.a, { ...r, y1: at })
      walk(n.b, { ...r, y0: at })
    }
  }
  walk(node, { x0: 0, y0: 0, x1: 1, y1: 1 })
  return out
}

/** 셸 `geometry::has_area` — 두 축 모두 엄격히 `x0 < x1` · `y0 < y1`. */
export function hasArea(r: SlotRect): boolean {
  return r.x0 < r.x1 && r.y0 < r.y1
}

/** 셸 `geometry::px_edge` — `round(canvas × e)`(동률은 0 에서 먼 쪽 = 음이 아닌 입력에서 `Math.round`). 범위 밖은 `u32` 포화. */
export function pxEdge(canvas: number, e: number): number {
  return toU32(Math.round(canvas * e))
}

/** 셸 `geometry::frame_px` — 칸 틀(캔버스 CSS px). 이웃 칸이 같은 정수 경계를 나눈다. */
export function framePx(canvas: CanvasPx, r: RectF64): PxRect {
  return { x0: pxEdge(canvas.w, r.x0), y0: pxEdge(canvas.h, r.y0), x1: pxEdge(canvas.w, r.x1), y1: pxEdge(canvas.h, r.y1) }
}

/** 셸 `tree::slot_ids` — 전위 순 칸 id. */
export function slotIds(node: LayoutNode): string[] {
  return node.type === 'slot' ? [node.id] : [...slotIds(node.a), ...slotIds(node.b)]
}

// 감싸는 분할의 id — 이 결과 나무는 기하에만 쓰고 셸로 보내지 않는다(셸은 놓을 때 자기 `docked_tree` 로 새 id 를 뽑는다 ·
//   ADR-0223). 기하는 분할 id 를 읽지 않는다.
const WRAP_SPLIT_ID = 'docked-wrap'

/**
 * 셸 `layout::dock::docked_tree` — `target` 의 `anchor` 를 새 분할로 감싸 `side` 쪽에 `incoming` 을 몫 `share` 로 끼운 결과.
 * 입력은 건드리지 않는다. 오류 차례 = 기준 칸 없음 → 두 나무의 칸 id 겹침 → 결과에 **새로** 면적 0 칸(원래 자리 — 대상 칸은
 * `target` 기하 · 끼운 칸은 `incoming` 단독 기하 — 에서 면적이 있던 칸)이 생김(R8).
 */
export function dockedTree(
  target: LayoutNode,
  anchor: DropAnchor,
  side: DockSide,
  incoming: LayoutNode,
  share: number,
  limits: RatioLimits,
): DockedTree {
  const dir = side === 'Left' || side === 'Right' ? 'left_right' : 'top_bottom'
  const newIsA = side === 'Left' || side === 'Top'
  let found = anchor.type === 'outer'
  // 비율 = a 몫(ADR-0140) — 새것이 b 면 `1 − share`.
  const wrap = (original: LayoutNode): LayoutNode =>
    newIsA
      ? { type: 'split', id: WRAP_SPLIT_ID, dir, ratio: share, a: incoming, b: original }
      : { type: 'split', id: WRAP_SPLIT_ID, dir, ratio: 1 - share, a: original, b: incoming }
  const rebuild = (n: LayoutNode): LayoutNode => {
    if (n.type === 'slot') {
      if (!found && anchor.type === 'slot' && n.id === anchor.id) {
        found = true
        return wrap(n)
      }
      return n
    }
    const a = rebuild(n.a)
    const b = rebuild(n.b)
    return a === n.a && b === n.b ? n : { ...n, a, b }
  }
  const result = anchor.type === 'outer' ? wrap(target) : rebuild(target)
  if (!found) return { ok: false, error: 'slot-not-found' }

  const targetSlots = new Set(slotIds(target))
  if (slotIds(incoming).some(s => targetSlots.has(s))) return { ok: false, error: 'slot-id-overlap' }

  // ADR-0227 — 원래부터 면적 0 이던 칸은 사유가 아니다(안 빼면 그런 칸 하나가 그 탭으로의 도킹을 전부 막는다).
  const hadArea = new Set(
    [target, incoming].flatMap(n => computeSlots(n, limits).filter(hasArea).map(r => r.slot_id)),
  )
  const collapses = computeSlots(result, limits).some(r => !hasArea(r) && hadArea.has(r.slot_id))
  return collapses ? { ok: false, error: 'zero-area' } : { ok: true, tree: result }
}

/**
 * 셸 `layout::dock::incoming_rect` — 결과 기하 `result` 에서 `ids` 칸들의 외접 정규화 사각형(경계 값을 고르기만 한다 — 끼운
 * 서브트리 상자와 비트 단위로 같다). `ids` 가 비었거나 `result` 에 없는 id 가 하나라도 있으면 `null`.
 */
export function incomingRect(result: readonly SlotRect[], ids: readonly string[]): RectF64 | null {
  let out: RectF64 | null = null
  for (const id of ids) {
    const r = result.find(s => s.slot_id === id)
    if (r === undefined) return null
    out =
      out === null
        ? { x0: r.x0, y0: r.y0, x1: r.x1, y1: r.y1 }
        : { x0: Math.min(out.x0, r.x0), y0: Math.min(out.y0, r.y0), x1: Math.max(out.x1, r.x1), y1: Math.max(out.y1, r.y1) }
  }
  return out
}
