# Step 8 QA — Monaco DiffEditor 검증

## 할 일

1. `npm run tauri dev` 실행 (I:\Engram\apps\engram-dashboard\)
2. chrome-devtools MCP로 스크린샷 + 확인

## 체크리스트

| 항목 | 기대값 |
|------|--------|
| Diff 토글 | "Diff ▼" 클릭 시 DiffPanel 표시, "Diff ▲" 클릭 시 숨김 |
| Diff 렌더링 | original/modified 코드 좌우 분할 표시 |
| 변경 하이라이트 | 추가/삭제 라인 색상 구분 |
| Accept/Revert | 버튼 표시 확인 (동작은 더미) |
| 높이 | DiffPanel 약 300px |

## 완료 후

`orch 4 "⟁dq30 step8 QA 결과: (통과/실패 + 이슈)"` 로 보고.
