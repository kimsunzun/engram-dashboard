# Step 9 QA — 슬롯 팝업 분리 검증

## 할 일

1. `npm run tauri dev` 실행 (I:\Engram\apps\engram-dashboard\)
2. chrome-devtools MCP로 확인

## 체크리스트

| 항목 | 기대값 |
|------|--------|
| 컨텍스트 메뉴 | "팝업으로 분리" 항목 표시 |
| 팝업 열기 | 클릭 시 새 창(PopupPage) 열림 |
| 팝업 내용 | 슬롯 터미널 더미 출력 표시 |
| 라우팅 | `/popup` 경로 정상 동작 |

## 완료 후

`orch 4 "⟁dq30 step9 QA 결과: (통과/실패 + 이슈)"` 로 보고.
