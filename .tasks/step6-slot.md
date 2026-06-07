# Step 6 — 슬롯 컴포넌트

## 할 일

### 1. `src/store/slotStore.ts` 생성
```ts
// Zustand store
interface Slot {
  id: number
  agentId: string | null  // 연결된 에이전트 id
}
// state: slots: Slot[], focusedSlotId: number
// actions: setFocusedSlot(id), assignAgent(slotId, agentId)
```

### 2. `src/components/slot/SlotPane.tsx` 수정
- 포커스 슬롯: 테두리 `2px solid var(--accent)`
- 비포커스: `1px solid var(--border)`
- 클릭 시 `setFocusedSlot(id)` 호출
- 우하단 에이전트 이름 오버레이 (absolute, font-size 11px, color: var(--text-muted))
  - agentId 없으면 "—" 표시

### 3. `src/components/slot/SlotContextMenu.tsx` 생성
- 우클릭 컨텍스트 메뉴 (position: fixed)
- 메뉴 항목: 분할(더미) / 에이전트 전환(더미) / 닫기(더미)
- 외부 클릭 시 닫힘

### 4. 에이전트 트리 → 슬롯 연동
- AgentTree 노드 클릭 시 현재 포커스 슬롯에 해당 agentId 할당
- SlotPane 오버레이에 에이전트 이름 표시

## 완료 기준
- `npm run build` 에러 없음
- 완료 후 `orch 4 "⟁dc29 step6 완료"` 로 보고
