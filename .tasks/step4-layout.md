# Step 4 — 레이아웃 셸

## 패키지 설치
```bash
npm install allotment
```

## 목표 레이아웃
```
┌─────────────┬──────────────────────────────────┐
│ Agent Tree  │  Slot 1        │  Slot 2         │
│ (사이드바)  ├────────────────┼─────────────────│
│             │  Slot 3        │  Slot 4         │
├─────────────┴──────────────────────────────────┤
│ Status Bar                                      │
└─────────────────────────────────────────────────┘
```

## 할 일

### 1. `src/components/layout/AppLayout.tsx` 생성
- allotment `Allotment` (수평 분할): 좌=사이드바(200px 기본, 최소 120px) / 우=메인
- 메인 영역: allotment `Allotment` (수직) → 상단 슬롯존 / 하단 StatusBar(고정 24px)
- 슬롯존: allotment `Allotment` (수평) → Slot1 / Slot2 (일단 2분할 고정)
- 사이드바 접기/펼치기: 버튼 클릭 시 사이드바 width 0으로 토글

### 2. `src/components/layout/Sidebar.tsx` 생성
- 더미 텍스트 "Agent Tree" 표시 (실제 트리는 Step 5)
- 배경: `var(--bg-secondary)`, 테두리: `1px solid var(--border)`

### 3. `src/components/layout/SlotPane.tsx` 생성
- props: `slotId: number`
- 배경: `var(--bg)`, 테두리: `1px solid var(--border)`
- 중앙에 `Slot {slotId}` 텍스트 표시

### 4. `src/components/layout/StatusBar.tsx` 생성
- 높이 24px 고정
- 배경: `var(--bg-secondary)`, 테두리 상단: `1px solid var(--border)`
- 더미 텍스트: "Ready"

### 5. `src/App.tsx` 수정
- 기존 버튼들 + 폰트 미리보기 유지 (지우지 말 것)
- AppLayout을 최상단 컨테이너로 감싸기
- App 전체 height: `100vh`

## 완료 기준
- `npm run build` 에러 없음
- 완료 후 `orch 4 "⟁dc29 step4 완료"` 로 보고
