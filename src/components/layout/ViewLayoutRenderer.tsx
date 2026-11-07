// ★The only layout renderer★ (Brick 1): the old frontend-only slotStore/LayoutRenderer (number id + content union)
// was removed. This renderer draws only the wire LayoutNode (string UUID id + content: SlotContent, ADR-0060, src-tauri/bindings) —
// whether by a human click (SlotContextMenu — right-click only, ADR-0144) or the LLM (window.__engramCmd), it is updated through the same invoke→emit
// authority loop.
// ADR-0227: flat root — the tree is not unfolded recursively; areas (leaves) and dividers are placed absolutely under one root using
//   rectangles computed by the shell. The screen does not compute tree geometry itself (the direction mapping ADR-0140 is also in the shell `layout/geometry.rs`).

import { useEffect, useMemo, useRef } from 'react'

import type { LayoutNode, SlotRect, SplitRect } from '../../api/layoutTypes'
import { useCurrentViewId } from '../../store/viewStore'
import LayoutLeaf from './LayoutLeaf'
import Splitter from './Splitter'
import { useSplitDrag } from './useSplitDrag'

type SlotNode = Extract<LayoutNode, { type: 'slot' }>

const NO_SLOT_RECTS: SlotRect[] = []
const NO_SPLIT_RECTS: SplitRect[] = []
const NO_FOREIGN_SLOTS: readonly string[] = []

// Code unit comparison — deterministic regardless of locale.
const byId = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0)

interface TreeIds {
  slots: Map<string, SlotNode>
  splits: Set<string>
}

function collectTree(node: LayoutNode, out: TreeIds): TreeIds {
  if (node.type === 'slot') {
    out.slots.set(node.id, node)
  } else {
    out.splits.add(node.id)
    collectTree(node.a, out)
    collectTree(node.b, out)
  }
  return out
}

/** `ids` is exactly the same set as `expected` — nothing missing, nothing left over, no duplicates. */
function sameIds(ids: readonly string[], expected: ReadonlySet<string> | ReadonlyMap<string, unknown>): boolean {
  return ids.length === expected.size && new Set(ids).size === ids.length && ids.every(id => expected.has(id))
}

// The error left when the rectangles cannot be used — it also states how it draws.
const RECT_ERRORS = {
  missing: '[ViewLayoutRenderer] 분할 트리인데 사각형이 없어 아무것도 그리지 않는다:',
  mismatchBlank: '[ViewLayoutRenderer] 사각형의 칸·분할 id 가 트리와 어긋나 아무것도 그리지 않는다:',
  mismatchFullBox: '[ViewLayoutRenderer] 사각형의 칸·분할 id 가 트리와 어긋나 루트 칸을 전체 상자로 그린다:',
} as const

export default function ViewLayoutRenderer({
  node,
  focusedSlotId,
  viewIdOverride,
  slotRects,
  splitRects,
  ratioBounds,
  version,
  foreignSlots = NO_FOREIGN_SLOTS,
}: {
  node: LayoutNode
  focusedSlotId: string | null
  // ★View id override that this renderer draws (optional).★ WindowLayout (main · popup) passes each tab canvas that tab's view
  //   (ADR-0057), pinning the action coordinates of the inner SlotContextMenu to that tab view. Without it the menu falls back to
  //   useCurrentViewId (the active tab of this webview window).
  viewIdOverride?: string | null
  /**
   * Area · split rectangles computed by the shell — ★pass the cache array by reference as is (no copy · sort)★. The divider preview recognizes
   *   a new snapshot by array reference, and returning unchanged rectangles as the same object keeps leaves from redrawing.
   * Used only when the area · split ids match the tree exactly. If absent or mismatched, draw the full box only when the root is a single area, and for a split
   *   tree leave an error and draw nothing.
   */
  slotRects?: SlotRect[]
  splitRects?: SplitRect[]
  /** The shell's split ratio limits (snapshot `ratio_min`/`ratio_max`). Without them dividers are locked — the screen does not copy shell constants. */
  ratioBounds?: { min: number; max: number }
  /** Cache version — the basis on which the divider preview recognizes a confirmed snapshot. */
  version?: number
  /** Cache `foreignSlots` — slots holding content this version does not know. If absent, there are no such slots. */
  foreignSlots?: readonly string[]
}) {
  // The same View coordinate as the leaf's menu · focus (same formula as LayoutLeaf's targetViewId).
  const currentViewId = useCurrentViewId()
  const viewId = viewIdOverride ?? currentViewId
  const rootRef = useRef<HTMLDivElement>(null)

  const tree = useMemo(() => collectTree(node, { slots: new Map(), splits: new Set() }), [node])
  const foreign = useMemo(() => new Set(foreignSlots), [foreignSlots])
  const slotNodes = tree.slots
  const rootSlotId = node.type === 'slot' ? node.id : null
  const fullBox = useMemo<SlotRect[]>(
    () => (rootSlotId === null ? NO_SLOT_RECTS : [{ slot_id: rootSlotId, x0: 0, y0: 0, x1: 1, y1: 1 }]),
    [rootSlotId],
  )
  // Drawing with mismatched rectangles drops areas and makes dividers drag a split that is not in the tree. The screen does not compute tree geometry itself, so
  //   a split tree is not drawn. Not on the production path (a snapshot always carries rectangles equal to the tree).
  const rectsMatch = useMemo(
    () =>
      slotRects !== undefined &&
      sameIds(slotRects.map(r => r.slot_id), tree.slots) &&
      sameIds((splitRects ?? NO_SPLIT_RECTS).map(s => s.split_id), tree.splits),
    [slotRects, splitRects, tree],
  )
  const drawNothing = !rectsMatch && rootSlotId === null
  // A single area without rectangles is an allowed input (full box) — no notice.
  let rectError: keyof typeof RECT_ERRORS | null = null
  if (slotRects === undefined) {
    if (drawNothing) rectError = 'missing'
  } else if (!rectsMatch) {
    rectError = drawNothing ? 'mismatchBlank' : 'mismatchFullBox'
  }
  useEffect(() => {
    if (rectError !== null) console.error(RECT_ERRORS[rectError], node.id)
  }, [rectError, node.id])

  const drag = useSplitDrag({
    viewId: viewId ?? '',
    slotRects: rectsMatch && slotRects !== undefined ? slotRects : fullBox,
    splitRects: rectsMatch ? (splitRects ?? NO_SPLIT_RECTS) : NO_SPLIT_RECTS,
    ratioMin: ratioBounds?.min ?? NaN,
    ratioMax: ratioBounds?.max ?? NaN,
    version: version ?? 0,
    rootRef,
  })
  // Leaves in slot id order, dividers in split id order (TRD §2e · D6) — following tree order would move DOM nodes on every rebuild
  //   (losing focus · scroll, losing the pointer capture of a divider being dragged). Surviving leaves keep their parent · key · relative order.
  const leafRects = useMemo(
    () => [...drag.slotRects].sort((a, b) => byId(a.slot_id, b.slot_id)),
    [drag.slotRects],
  )
  const dividerRects = useMemo(
    () => [...drag.splitRects].sort((a, b) => byId(a.split_id, b.split_id)),
    [drag.splitRects],
  )

  if (drawNothing) return null
  // If the view coordinate or ratio limits are unknown, lock the dividers — there is neither a place nor a range to commit to (leaf focus is also a no-op when the view is unconfirmed).
  const locked = viewId === null || ratioBounds === undefined
  return (
    // ★Do not apply transform · contain★ — SlotContextMenu inside a leaf is position:fixed, so an ancestor's transform breaks the coordinate system.
    <div ref={rootRef} style={{ position: 'absolute', inset: 0, overflow: 'hidden' }}>
      {leafRects.map(r => {
        const leaf = slotNodes.get(r.slot_id)
        return leaf === undefined ? null : (
          <LayoutLeaf
            key={r.slot_id}
            node={leaf}
            rect={r}
            focusedSlotId={focusedSlotId}
            isForeign={foreign.has(r.slot_id)}
            viewIdOverride={viewIdOverride}
          />
        )
      })}
      {dividerRects.map(s => {
        const props = drag.splitterProps(s)
        return <Splitter key={s.split_id} {...props} range={locked ? null : props.range} />
      })}
    </div>
  )
}
