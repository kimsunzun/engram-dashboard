// 테스트 전용 — 레이아웃 트리에서 평평한 렌더러(ADR-0227)에 넘길 칸·분할 사각형 픽스처를 만든다.
// 분할마다 자기 상자를 비율대로 둘로 가르는 단순 재귀다. 겹치지 않는 사각형만 보장하고, 값이 셸
//   `src-tauri/src/layout/geometry.rs` 와 같다고 주장하지 않는다 — 기하 정확성은 그 Rust 테스트의 몫이다(TRD §5).
// 방향: `left_right` = x 축을 가르고 a 가 왼쪽, `top_bottom` = y 축을 가르고 a 가 위(ADR-0140).

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

/** 스냅샷 `ratio_min`/`ratio_max` 자리 — 렌더러 `ratioBounds` 에 그대로 넘긴다. */
export const TEST_RATIO_BOUNDS = { min: 0.1, max: 0.9 } as const

/** `node` 를 [0,1]×[0,1] 에 채운 사각형. 배열 순서는 트리 전위 순이다(렌더러가 id 순으로 다시 정렬한다). */
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
