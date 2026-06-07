# Step 4 QA — 레이아웃 셸 검증

## 할 일

1. `npm run tauri dev` 실행 (I:\Engram\apps\engram-dashboard\)
2. chrome-devtools MCP로 스크린샷 촬영
3. 아래 항목 확인

## 체크리스트

| 항목 | 기대값 |
|------|--------|
| 사이드바 표시 | 좌측 "Agent Tree" 텍스트, bg-secondary 배경 |
| 슬롯 분할 | Slot 1 / Slot 2 수평 분할 |
| 상태바 | 하단 고정 "Ready" 텍스트, 24px |
| 테마 색상 | CSS 변수 정상 적용 (dark 기본) |
| 사이드바 토글 | 버튼 클릭 시 접기/펼치기 동작 |

## 완료 후

`orch 4 "⟁dq30 step4 QA 결과: (통과/실패 + 이슈)"` 로 보고.
