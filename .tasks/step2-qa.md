# Step 2 QA — 테마 시스템 검증

## 할 일

1. `npm run tauri dev` 실행 (I:\Engram\apps\engram-dashboard\)
2. chrome-devtools MCP로 WebView 접속 (http://localhost:1420)
3. 스크린샷 촬영 — dark 기본 상태
4. [dark] [light] [e-ink] 버튼 각각 클릭 후 스크린샷 촬영
5. 각 테마에서 배경/텍스트 색상이 CSS 변수대로 바뀌는지 확인

## 기대값

| 테마 | --bg | --text |
|---|---|---|
| dark | #0a0a0a | #e0e0e0 |
| light | #f5f5f5 | #1a1a1a |
| e-ink | #ffffff | #000000 |

## 완료 후

완료 후 `orch 2 "⟁dq30 결과내용"` 으로 매니저에게 보고.
