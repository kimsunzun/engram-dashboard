// Test only — builds the area · split rectangle fixtures to pass from a layout tree to the flat renderer (ADR-0227).
// A simple recursion that splits each split's own box in two by ratio. It only guarantees non-overlapping rectangles and does not claim the values equal the shell
//   `src-tauri/src/layout/geometry.rs` — geometry correctness is the job of that Rust test (TRD §5).
// Direction: `left_right` = splits the x axis with a on the left, `top_bottom` = splits the y axis with a on top (ADR-0140).

import type { LayoutNode, SlotRect, SplitRect } from '../../../api/layoutTypes'

export interface LayoutRects {
  slotRects: SlotRect[]
  splitRects: SplitRect[]
}

interface Box {
  x0: number
  y0: number
  x1: number
  y1: number
}

/** Snapshot `ratio_min`/`ratio_max` position — passed as is to the renderer's `ratioBounds`. */
export const TEST_RATIO_BOUNDS = { min: 0.1, max: 0.9 } as const

/** A rectangle filling `node` into [0,1]×[0,1]. The array order is tree pre-order (the renderer re-sorts by id). */
export function rectsFor(node: LayoutNode): LayoutRects {
  const out: LayoutRects = { slotRects: [], splitRects: [] }
  walk(node, { x0: 0, y0: 0, x1: 1, y1: 1 }, out)
  return out
}

function walk(node: LayoutNode, box: Box, out: LayoutRects): void {
  if (node.type === 'slot') {
    out.slotRects.push({ slot_id: node.id, ...box })
    return
  }
  if (node.dir === 'left_right') {
    const at = box.x0 + (box.x1 - box.x0) * node.ratio
    out.splitRects.push({ split_id: node.id, dir: node.dir, ...box, at })
    walk(node.a, { ...box, x1: at }, out)
    walk(node.b, { ...box, x0: at }, out)
  } else {
    const at = box.y0 + (box.y1 - box.y0) * node.ratio
    out.splitRects.push({ split_id: node.id, dir: node.dir, ...box, at })
    walk(node.a, { ...box, y1: at }, out)
    walk(node.b, { ...box, y0: at }, out)
  }
}
