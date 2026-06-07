# Step 7 QA — xterm.js 더미 출력 검증

## 할 일

1. `npm run tauri dev` 실행 (I:\Engram\apps\engram-dashboard\)
2. chrome-devtools MCP로 스크린샷 + 확인

## 체크리스트

| 항목 | 기대값 |
|------|--------|
| 터미널 렌더링 | 슬롯 내 xterm.js 터미널 표시 |
| ANSI 색상 | ✓ 초록 / ⚠ 노랑 / ✗ 빨강 / > 회색 텍스트 |
| 폰트 | --font-terminal(Cascadia Code) 적용 |
| 배경/전경 | --bg/#0a0a0a 배경, --text/#e0e0e0 텍스트 |
| 리사이즈 | allotment 드래그 시 터미널 cols/rows 깨지지 않음 |

## 완료 후

`orch 4 "⟁dq30 step7 QA 결과: (통과/실패 + 이슈)"` 로 보고.
