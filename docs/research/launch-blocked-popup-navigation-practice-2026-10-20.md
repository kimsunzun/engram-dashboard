# 실행이 막혔을 때 무엇이 뜨나 · 그 안 항목을 누르면 어디로 가나 (2026-10-20)

| 항목 | 값 |
|---|---|
| **상태** | 조사 완료 · **결정됨 2026-10-20** — 사람이 낸 실행이 막히면 그 에이전트의 프리셋 편집 칸을 트리 줄 옆 따로 된 떠 있는 창으로 · 바깥 누름에 안 닫히고 × · Esc 로만(남는 패널 관행 — Unreal Compiler Results) · 고치기는 그 안에서 · 라이브러리는 더블클릭 · 돋보기로 갈 때만(사용자 「ㅇㅋㅇㅋ … 최종 적용」 — PRD E4) |
| **방법** | `/research` light — 수집자 1(검색 4회 · 원문 직접 읽음 없음) + 메인 grounding(Unreal 문서 원문 확인 · VS Code 이슈 원문 확인 → 미지지) |
| **질문** | 실행 · 플레이 · 컴파일이 설정 문제로 막힐 때 ① 무엇이 뜨나(대상 옆 팝오버 · 알림 · 모달 · 패널) ② 항목을 누르면 출처로 가나 · 대상 편집기가 닫혀 있으면 여나 ③ 이동한 뒤 그 표시는 어떻게 되나 |

## 결론

1. **실행한 대상 옆에 붙는 팝오버로 문제를 보여 주는 성숙한 도구는 찾지 못했다**(불확실 — 없음을 다 훑은 것은 아니다). 주류 = 저절로 열리는 **남는 패널**(Unreal Compiler Results — 확실) 또는 모달 · 일시 알림(VS Code · Visual Studio · JetBrains — 기억 · 스니펫, 미검증).
2. **항목을 누르면 문제 자리로 간다**(확실 — Unreal 원문 「Click on a message to jump the graph view directly to the problematic node」). 대상 편집기가 닫혀 있을 때 열어 주는지는 Unity(콘솔 더블클릭 → 외부 스크립트 편집기를 띄움 · 스니펫)뿐이고, 그것을 적은 공식 문서는 못 찾았다.
3. **표시는 남는다**(가능성 높음): Unreal 패널 · Unity 콘솔은 이동한 뒤에도 남는다. 모달은 닫히고 풍선은 사라진다는 것은 기억 기반 추정이다.

## 사례

| 사례 | 실행 때 표시 | 누르면 | 이동 뒤 | 출처 | 확신도 |
|---|---|---|---|---|---|
| Unreal Blueprint | Compiler Results 패널이 오류 · 경고 때 **저절로 열림**(보통 Graph 패널 아래) | 메시지 · 링크 → 그 노드로 | 패널 남음 | https://dev.epicgames.com/documentation/en-us/unreal-engine/compiler-results-in-the-blueprints-visual-scripting-editor-for-unreal-engine | 확실(메인 원문 확인) |
| Unity | Play 때 콘솔에 「All compiler errors have to be fixed before you can enter playmode!」 | 콘솔 더블클릭 → 외부 스크립트 편집기가 열리고 그 줄로 | 콘솔 남음 | https://support.unity.com/hc/en-us/articles/205930539 · https://discussions.unity.com/t/all-compilers-errors-have-to-be-fixed-before-you-can-enter-playmode/503684 | 가능성 높음(스니펫) |
| VS Code | preLaunchTask 실패 → 모달(Debug Anyway · Show Errors · Abort) | Show Errors → Problems 패널 · 항목 → 파일 탭 | — | 수집자가 든 https://github.com/microsoft/vscode/issues/54397 는 그 대화창을 다루지 않는다(메인 확인) | 불확실(기억) |
| JetBrains | 「Error running '<config>'」 풍선 + 설정 편집 링크 | 링크 → Run/Debug Configurations 창 | 풍선 사라짐 · 알림 기록에 남음 | 없음 | 불확실(기억) |
| Visual Studio | 「There were build errors…」 모달 + Error List | — | — | 없음 | 불확실(기억) |

## 공백

- 대상 옆 팝오버 꼴(GitHub Actions · Docker Desktop 등)은 보지 않았다. Xcode 도 보지 않았다.
- 「편집기가 닫혀 있을 때 누르면 새 창을 연다」 를 적은 공식 문서는 없다.
