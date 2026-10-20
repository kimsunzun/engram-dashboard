# 여러 항목이 실행에 실패할 때 문제 표시 · 닫은 뒤 다시 열기 (2026-10-21)

| 항목 | 값 |
|---|---|
| **상태** | 조사 완료 · **다음 주제(에이전트 시스템)로 넘김 2026-10-21**(사용자 — 막힘 창이 여럿일 때 · 닫았다 다시 열 때 · 다시 실행은 그때 정한다) |
| **방법** | `/research` light — 수집자 1(검색 5 · 원문 1) + 메인 grounding(JetBrains 원문은 수집자가 읽음 · 나머지 스니펫) |
| **질문** | 실행이 여러 항목에서 막히면 ① 항목마다 창인가 · 공용 패널 하나에 묶나 ② 닫은 뒤 무엇으로 다시 여나 ③ 고치는 동안 바로 갱신되나 · 그 자리에 다시 실행이 있나 |

## 결론

1. **항목마다 창이 아니라 공용 패널 하나에 묶는 것이 관행이다**(가능성 높음) — VS Code Problems(파일별) · JetBrains Problems(탭 · 묶음) · Unreal Message Log(분류별) · GitHub Actions run 페이지(job 별). 실패마다 창을 띄우는 사례는 찾지 못했다.
2. **다시 여는 길 = 늘 있는 진입점**(가능성 높음) — 메뉴 · 단축키 · 상태 표시줄의 오류 수 · 항목 옆 아이콘. 문제가 없어도 진입점은 남는다.
3. **바로 갱신 · 다시 실행은 그 패널 안이나 곁**(JetBrains 갱신 = 확실 · 원문 「IDE continuously checks … updated as you switch files」 / GitHub Actions 「Re-run failed jobs」 = 확실 · 공식 문서 스니펫 / JetBrains Run 창 rerun = 가능성 높음).

## 사례

| 사례 | 여러 실패 | 다시 열기 | 갱신 · 다시 실행 | 출처 | 확신도 |
|---|---|---|---|---|---|
| JetBrains Problems | 탭(Current File · Project · Inspection Results) · 묶음 | 도구 창 바 · View 메뉴(추정) | 계속 갱신 · 퀵픽스 | https://jetbrains.com/help/clion/problems-tool-window.html | 확실(원문) |
| JetBrains Run 창 | 실행 구성마다 탭 | — | 툴바 rerun · rerun failed | https://www.jetbrains.com/help/idea/2018.2/rerunning-tests.html | 가능성 높음 |
| VS Code Problems | 패널 하나 · 파일별 | View > Problems · Ctrl+Shift+M · 상태 표시줄 오류 수 | 재실행 개념 없음 | 2차 글 스니펫 | 가능성 높음 |
| Unreal Message Log | 창 하나 · 분류별 로그 | Window > Developer Tools > Message Log | 미확인 | https://unrealcommunity.wiki/message-log-4wzqj97j (커뮤니티) | 가능성 높음 |
| GitHub Actions | run 페이지 하나 · job 별 | — | Re-run failed jobs · 전체 · 특정 job | https://docs.github.com/en/actions/how-tos/manage-workflow-runs/re-run-workflows-and-jobs | 확실(공식 스니펫) |

## 공백

- Visual Studio Error List · Xcode · Unity Console · Docker Desktop · Task Scheduler 는 보지 않았다.
- 「항목 배지를 눌러 그 항목의 문제를 다시 연다」 꼴은 확인 못 함(진입점 존재까지만).
