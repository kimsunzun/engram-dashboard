# 슬롯 동적 분할

## 목표
현재 2분할 고정 → 런타임에 슬롯 추가/삭제 가능한 재귀 allotment 구조.

## 데이터 구조 변경

### `src/store/slotStore.ts` 전면 교체
```ts
import { create } from 'zustand'

export type SplitDir = 'horizontal' | 'vertical'

export interface SlotNode {
  type: 'slot'
  id: number
  agentId: string | null
}

export interface SplitNode {
  type: 'split'
  dir: SplitDir
  children: LayoutNode[]
}

export type LayoutNode = SlotNode | SplitNode

let nextId = 3 // 초기 슬롯 1, 2 이후

interface SlotState {
  layout: LayoutNode          // 루트 레이아웃 트리
  focusedSlotId: number
  setFocusedSlot: (id: number) => void
  assignAgent: (slotId: number, agentId: string) => void
  splitSlot: (slotId: number, dir: SplitDir) => void
  closeSlot: (slotId: number) => void
}

// 초기값: horizontal split → Slot1 | Slot2
const initialLayout: LayoutNode = {
  type: 'split',
  dir: 'horizontal',
  children: [
    { type: 'slot', id: 1, agentId: null },
    { type: 'slot', id: 2, agentId: null },
  ],
}
```

구현 요령:
- `splitSlot(slotId, dir)`: 트리 순회 → slotId 찾으면 해당 SlotNode를 SplitNode로 교체 (원본 슬롯 + 새 슬롯 children)
- `closeSlot(slotId)`: 부모 SplitNode에서 해당 child 제거. children 1개 남으면 부모 자체를 그 child로 교체 (flatten)
- 루트가 SlotNode 단독이 되는 경우도 처리

## 렌더링

### `src/components/layout/LayoutRenderer.tsx` 신규 생성
```tsx
// LayoutNode를 재귀적으로 allotment로 렌더링
function LayoutRenderer({ node }: { node: LayoutNode }) {
  if (node.type === 'slot') {
    return <SlotPane slotId={node.id}><TerminalSlot /></SlotPane>
  }
  return (
    <Allotment vertical={node.dir === 'vertical'}>
      {node.children.map((child, i) => (
        <Allotment.Pane key={i}>
          <LayoutRenderer node={child} />
        </Allotment.Pane>
      ))}
    </Allotment>
  )
}
```

### `src/components/layout/AppLayout.tsx` 수정
- 기존 하드코딩된 Slot1/Slot2 allotment → `<LayoutRenderer node={layout} />` 로 교체

### `src/components/slot/SlotContextMenu.tsx` 수정
- "분할" 메뉴: 서브메뉴 or 두 항목으로 분리
  - "가로 분할" → `splitSlot(slotId, 'horizontal')`
  - "세로 분할" → `splitSlot(slotId, 'vertical')`
- "닫기" → `closeSlot(slotId)`

## 완료 기준
- `npm run build` 에러 없음
- 완료 후 `orch 4 "⟁dc29 dynamic-split 완료"` 로 보고
