// ★유일한 레이아웃 렌더러★(Brick 1): 옛 프론트 전용 slotStore/LayoutRenderer(number id + content union)는
// 제거됐다. 이 렌더러는 wire LayoutNode(string UUID id + content: SlotContent, ADR-0060, src-tauri/bindings)만 그린다 —
// 사람 클릭(SlotContextMenu — 우클릭 전용, ADR-0144)이든 LLM(window.__engramCmd)이든 같은 invoke→emit
// 권위 루프로 갱신된다.
// ADR-0227: 평평한 루트 — 트리를 재귀로 풀지 않고, 셸이 계산한 사각형으로 칸(잎)과 구분선을 루트 하나 아래에
//   절대 배치한다. 화면은 트리 기하를 따로 계산하지 않는다(방향 매핑 ADR-0140 도 셸 `layout/geometry.rs` 에 있다).

import { useEffect, useMemo, useRef } from 'react'

import type { LayoutNode, SlotRect, SplitRect } from '../../api/layoutTypes'
import { useCurrentViewId } from '../../store/viewStore'
import LayoutLeaf from './LayoutLeaf'
import Splitter from './Splitter'
import { useSplitDrag } from './useSplitDrag'

type SlotNode = Extract<LayoutNode, { type: 'slot' }>

const NO_SLOT_RECTS: SlotRect[] = []
const NO_SPLIT_RECTS: SplitRect[] = []

// 코드 단위 비교 — 로캘과 무관하게 결정적이다.
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

/** `ids` 가 `expected` 와 정확히 같은 집합이다 — 빠짐·남음·중복이 없다. */
function sameIds(ids: readonly string[], expected: ReadonlySet<string> | ReadonlyMap<string, unknown>): boolean {
  return ids.length === expected.size && new Set(ids).size === ids.length && ids.every(id => expected.has(id))
}

// 사각형을 쓰지 못할 때 남기는 오류 — 어떻게 그리는지까지 적는다.
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
}: {
  node: LayoutNode
  focusedSlotId: string | null
  // ★이 렌더러가 그리는 View id 오버라이드(선택).★ WindowLayout(main·팝업)이 각 탭 캔버스에 그 탭 view 를
  //   넘겨(ADR-0057) 내부 SlotContextMenu 의 액션 좌표를 그 탭 view 로 고정한다. 없으면 메뉴가
  //   useCurrentViewId(이 웹뷰 창의 active 탭) 폴백.
  viewIdOverride?: string | null
  /**
   * 셸이 계산한 칸·분할 사각형 — ★캐시 배열을 참조 그대로 넘긴다(복사·정렬 금지)★. 구분선 미리보기가 배열 참조로
   *   새 스냅샷을 알아보고, 안 바뀐 사각형을 같은 객체로 돌려줘 잎이 다시 그리지 않는다.
   * 칸·분할 id 가 트리와 정확히 맞을 때만 쓴다. 없거나 어긋나면 루트가 칸 하나일 때만 전체 상자로 그리고, 분할
   *   트리는 오류를 남기고 아무것도 그리지 않는다.
   */
  slotRects?: SlotRect[]
  splitRects?: SplitRect[]
  /** 셸의 분할 비율 한계(스냅샷 `ratio_min`/`ratio_max`). 없으면 구분선은 잠긴다 — 화면이 셸 상수를 베끼지 않는다. */
  ratioBounds?: { min: number; max: number }
  /** 캐시 version — 구분선 미리보기가 확정 스냅샷을 알아보는 기준. */
  version?: number
}) {
  // 잎의 메뉴·포커스와 같은 View 좌표다(LayoutLeaf 의 targetViewId 와 같은 식).
  const currentViewId = useCurrentViewId()
  const viewId = viewIdOverride ?? currentViewId
  const rootRef = useRef<HTMLDivElement>(null)

  const tree = useMemo(() => collectTree(node, { slots: new Map(), splits: new Set() }), [node])
  const slotNodes = tree.slots
  const rootSlotId = node.type === 'slot' ? node.id : null
  const fullBox = useMemo<SlotRect[]>(
    () => (rootSlotId === null ? NO_SLOT_RECTS : [{ slot_id: rootSlotId, x0: 0, y0: 0, x1: 1, y1: 1 }]),
    [rootSlotId],
  )
  // 어긋난 사각형으로 그리면 칸이 빠지고 구분선이 트리에 없는 분할을 끈다. 화면은 트리 기하를 따로 계산하지 않으므로
  //   분할 트리는 그리지 않는다. 운영 경로엔 없다(스냅샷이 늘 트리와 같은 사각형을 싣는다).
  const rectsMatch = useMemo(
    () =>
      slotRects !== undefined &&
      sameIds(slotRects.map(r => r.slot_id), tree.slots) &&
      sameIds((splitRects ?? NO_SPLIT_RECTS).map(s => s.split_id), tree.splits),
    [slotRects, splitRects, tree],
  )
  const drawNothing = !rectsMatch && rootSlotId === null
  // 사각형 없는 단일 칸은 허용된 입력이다(전체 상자) — 알리지 않는다.
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
  // 잎은 slot id 순, 구분선은 split id 순(TRD §2e · D6) — 트리 순서를 따르면 재구성마다 DOM 노드가 옮겨진다
  //   (포커스·스크롤 소실, 끌던 구분선의 포인터 캡처 소실). 생존 잎은 부모·key·상대 순서가 그대로다.
  const leafRects = useMemo(
    () => [...drag.slotRects].sort((a, b) => byId(a.slot_id, b.slot_id)),
    [drag.slotRects],
  )
  const dividerRects = useMemo(
    () => [...drag.splitRects].sort((a, b) => byId(a.split_id, b.split_id)),
    [drag.splitRects],
  )

  if (drawNothing) return null
  // 뷰 좌표나 비율 한계를 모르면 구분선을 잠근다 — 확정할 곳도 범위도 없다(잎의 포커스도 뷰 미확정이면 no-op).
  const locked = viewId === null || ratioBounds === undefined
  return (
    // ★transform·contain 을 걸지 말 것★ — 잎 안 SlotContextMenu 가 position:fixed 라 조상의 transform 이 좌표계를 깬다.
    <div ref={rootRef} style={{ position: 'absolute', inset: 0, overflow: 'hidden' }}>
      {leafRects.map(r => {
        const leaf = slotNodes.get(r.slot_id)
        return leaf === undefined ? null : (
          <LayoutLeaf
            key={r.slot_id}
            node={leaf}
            rect={r}
            focusedSlotId={focusedSlotId}
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
