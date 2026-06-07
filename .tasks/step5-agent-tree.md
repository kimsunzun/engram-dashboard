# Step 5 — 에이전트 트리 (더미)

## 패키지 설치
```bash
npm install react-arborist
```

## 더미 데이터
```ts
// src/store/agentStore.ts
export const dummyAgents = [
  { id: '1', name: '비서', status: 'running', cost: '$0.12' },
  { id: '2', name: '코더', status: 'idle',    cost: '$0.21' },
  { id: '3', name: '리뷰어', status: 'error', cost: '$0.08' },
]
export const dummyGroups = [
  { id: 'g1', name: '코딩룰', members: ['1', '2', '3'] },
]
```

## 할 일

### 1. `src/store/agentStore.ts` 생성
- Zustand store, dummyAgents / dummyGroups 상태
- `selectedAgentId: string | null`, `setSelectedAgent(id)` action

### 2. `src/components/agent/AgentTree.tsx` 생성
- react-arborist `Tree` 컴포넌트 사용
- NodeRenderer:
  - status별 아이콘 색상: running=`var(--accent)` / idle=`var(--text-muted)` / error=`#ff4444`
  - 이름 + 비용 표시
  - 클릭 시 `setSelectedAgent(id)` 호출
- 그룹 노드: 멤버 수 뱃지 표시

### 3. `src/components/layout/Sidebar.tsx` 수정
- 기존 더미 텍스트 대신 `<AgentTree />` 렌더링

## 완료 기준
- `npm run build` 에러 없음
- 완료 후 `orch 4 "⟁dc29 step5 완료"` 로 보고
