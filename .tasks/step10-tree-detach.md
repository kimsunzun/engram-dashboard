# Step 10 — 에이전트 트리 분리

## 할 일

### 1. `src-tauri/tauri.conf.json` 수정
- `windows` 배열에 트리 창 설정 추가:
  ```json
  {
    "label": "agent-tree",
    "url": "index.html#/tree",
    "visible": false,
    "width": 280,
    "height": 600,
    "decorations": true,
    "title": "Agent Tree"
  }
  ```

### 2. `src/pages/TreePage.tsx` 생성
- route `/tree` 에 마운트
- `<AgentTree />` 단독 렌더링
- 배경: `var(--bg-secondary)`

### 3. `src/App.tsx` 라우터에 추가
- `/tree` → TreePage

### 4. `src/components/layout/Sidebar.tsx` 수정
- 상단에 "트리 분리" 버튼 추가
- 클릭 시 `window.open('index.html#/tree', '_blank')`
- 분리 후 사이드바는 접힘 처리 (setCollapsed(true))

## 완료 기준
- `npm run build` 에러 없음
- 완료 후 `orch 4 "⟁dc29 step10 완료"` 로 보고
