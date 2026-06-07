# Step 5 QA — 에이전트 트리 검증

## 할 일

1. `npm run tauri dev` 실행 (I:\Engram\apps\engram-dashboard\)
2. chrome-devtools MCP로 스크린샷 촬영

## 체크리스트

| 항목 | 기대값 |
|------|--------|
| 트리 표시 | 사이드바에 비서/코더/리뷰어 3개 노드 |
| 그룹 노드 | '코딩룰' 그룹, 멤버 수 뱃지 표시 |
| status 색상 | running=accent색 / idle=muted / error=빨강 |
| 비용 표시 | 각 노드에 $0.12 / $0.21 / $0.08 |
| 클릭 | 노드 클릭 시 선택 상태 변경 |

## 완료 후

`orch 4 "⟁dq30 step5 QA 결과: (통과/실패 + 이슈)"` 로 보고.
