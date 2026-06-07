# Step 10 QA — 에이전트 트리 분리 검증

## 할 일

1. `npm run tauri dev` 실행 (I:\Engram\apps\engram-dashboard\)
2. chrome-devtools MCP로 확인

## 체크리스트

| 항목 | 기대값 |
|------|--------|
| 트리 분리 버튼 | 사이드바 상단 "트리 분리" 버튼 표시 |
| 새 창 열림 | 클릭 시 AgentTree 단독 창 (#/tree) 열림 |
| 트리 창 내용 | 에이전트 목록 정상 표시 |
| 사이드바 접힘 | 분리 후 사이드바 자동 접힘 |
| 라우팅 | `/tree` 경로 정상 동작 |

## 완료 후

`orch 4 "⟁dq30 step10 QA 결과: (통과/실패 + 이슈)"` 로 보고.
