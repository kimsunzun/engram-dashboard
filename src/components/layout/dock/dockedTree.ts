// TRD S21-slot-docking §4-5: a mirror of the shell `layout::dock::{docked_tree, incoming_rect}` · `layout::geometry::{compute, boundary, px_edge,
//   frame_px}` — during a drag the drag does not align its decision with the shell (ADR-0296 · ADR-0297), so the web counts band validity (R8) · preview (R4)
//   alone. Pure.
// ★Do the same f64 operations as the shell in the same order★ — boundaries copy the parent value and the only newly computed value is one split boundary (shell `geometry.rs`
//   head invariant). That way the preview is bit-for-bit the same as the area the shell draws after the drop, and the zero-area decision of a deep chain stands at the same area as the shell.
//   The two versions are tied by the golden `src-tauri/src/layout/bands_golden.json` (`bands.test.ts` · `judge.test.ts`).
// Ratio limits are received as snapshot `ratio_min` · `ratio_max` (ADR-0227 — the screen keeps no constants of its own).

import type { LayoutNode, SlotRect } from '../../../api/layoutTypes'
import type { DockSide } from '../../../../src-tauri/bindings/DockSide'
import type { DropAnchor } from '../../../../src-tauri/bindings/DropAnchor'

/** Split ratio limits — snapshot `ratio_min` · `ratio_max` (shell `tree::RATIO_MIN` · `RATIO_MAX`). */
export interface RatioLimits {
  min: number
  max: number
}

/** Canvas size — integer CSS px. */
export interface CanvasPx {
  w: number
  h: number
}

/** Rectangle in integer CSS px boundary form (canvas origin) — half-open. Shell `geometry::PxRect`. */
export interface PxRect {
  x0: number
  y0: number
  x1: number
  y1: number
}

/** Rectangle in normalized boundary form — shell `geometry::RectF64`. */
export interface RectF64 {
  x0: number
  y0: number
  x1: number
  y1: number
}

/** Shell `DockTreeError` — the order the shell reports (the first one hit in this order). */
export type DockTreeError = 'slot-not-found' | 'slot-id-overlap' | 'zero-area'

export type DockedTree = { ok: true; tree: LayoutNode } | { ok: false; error: DockTreeError }

const U32_MAX = 4294967295

/** Shell `f64 as u32` — fraction dropped, NaN · negative become 0, overflow gives `u32::MAX`. */
export function toU32(v: number): number {
  if (!(v > 0)) return 0
  return v > U32_MAX ? U32_MAX : Math.trunc(v)
}

/** Shell `tree::clamp_ratio` — NaN stays NaN (filtering is done by `boundary`). */
function clampRatio(r: number, limits: RatioLimits): number {
  return Math.min(Math.max(r, limits.min), limits.max)
}

/** Shell `geometry::boundary` — the boundary that splits the axis range `[lo, hi]` by `ratio` (a's share). A NaN ratio is read as a half split. */
export function boundary(lo: number, hi: number, ratio: number, limits: RatioLimits): number {
  const t = Number.isNaN(ratio) ? 0.5 : clampRatio(ratio, limits)
  return lo + (hi - lo) * t
}

/** The area share of shell `geometry::compute` — area rectangles in tree pre-order (root = (0,0,1,1)). The same value as snapshot `slot_rects`. */
export function computeSlots(node: LayoutNode, limits: RatioLimits): SlotRect[] {
  const out: SlotRect[] = []
  const walk = (n: LayoutNode, r: RectF64): void => {
    if (n.type === 'slot') {
      out.push({ slot_id: n.id, x0: r.x0, y0: r.y0, x1: r.x1, y1: r.y1 })
      return
    }
    // ADR-0140: left_right = x-axis split (a on the left) · top_bottom = y-axis split (a on top).
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

/** Shell `geometry::has_area` — both axes strictly `x0 < x1` · `y0 < y1`. */
export function hasArea(r: SlotRect): boolean {
  return r.x0 < r.x1 && r.y0 < r.y1
}

/** Shell `geometry::px_edge` — `round(canvas × e)` (ties go to the side away from 0 = `Math.round` on non-negative input). Out of range saturates to `u32`. */
export function pxEdge(canvas: number, e: number): number {
  return toU32(Math.round(canvas * e))
}

/** Shell `geometry::frame_px` — area frame (canvas CSS px). Neighboring areas share the same integer boundary. */
export function framePx(canvas: CanvasPx, r: RectF64): PxRect {
  return { x0: pxEdge(canvas.w, r.x0), y0: pxEdge(canvas.h, r.y0), x1: pxEdge(canvas.w, r.x1), y1: pxEdge(canvas.h, r.y1) }
}

/** Shell `tree::slot_ids` — area ids in pre-order. */
export function slotIds(node: LayoutNode): string[] {
  return node.type === 'slot' ? [node.id] : [...slotIds(node.a), ...slotIds(node.b)]
}

// Id of the enclosing split — this resulting tree is used only for geometry and not sent to the shell (the shell picks new ids with its own `docked_tree` at drop ·
//   ADR-0223). Geometry does not read split ids.
const WRAP_SPLIT_ID = 'docked-wrap'

/**
 * Shell `layout::dock::docked_tree` — the result of wrapping `anchor` of `target` in a new split and inserting `incoming` at the `side` with share `share`.
 * The input is not touched. Error order = reference area missing → area ids of the two trees overlap → a **new** zero-area area appears in the result (original position — the target area is
 * the area that had area in the `target` geometry · the inserted area in the `incoming`-alone geometry) (R8).
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
  // Ratio = a's share (ADR-0140) — if the new one is b, `1 − share`.
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

  // ADR-0227 — an area that was zero-area from the start is not a reason (if not subtracted, one such area blocks all docking into that tab).
  const hadArea = new Set(
    [target, incoming].flatMap(n => computeSlots(n, limits).filter(hasArea).map(r => r.slot_id)),
  )
  const collapses = computeSlots(result, limits).some(r => !hasArea(r) && hadArea.has(r.slot_id))
  return collapses ? { ok: false, error: 'zero-area' } : { ok: true, tree: result }
}

/**
 * Shell `layout::dock::incoming_rect` — the bounding normalized rectangle of the `ids` areas in result geometry `result` (it only picks boundary values — bit-for-bit the same as the inserted
 * subtree box). `null` if `ids` is empty or any id is not in `result`.
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
