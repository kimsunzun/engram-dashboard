# 편집 칸 위쪽 문제 요약 관행 — 전부 나열 · 개수 · 배너 (2026-10-18)

| 항목 | 값 |
|---|---|
| **상태** | 조사 완료 · 결정 대기(프리셋 편집 칸 위쪽에 문제 요약을 둘지 · 꼴) |
| **방법** | `/research` light — 수집자 1(검색 + 공식 페이지 4개 읽음) + 메인 grounding(GOV.UK 원문 확인) |
| **질문** | 문제가 여럿일 때 위쪽에 ① 하나씩 전부 나열하나 개수만 보이나 ② 몇 개부터 접나 ③ 줄마다 있는 표시를 함께 두나 ④ 누르면 그 자리로 가나 |

## 결론

1. **가장 강한 꼴 = 위쪽에 문제마다 한 줄씩 전부 나열**(확실) — GOV.UK 오류 요약은 제목 「There is a problem」 + 오류마다 링크 한 줄을 본문 맨 위에 두고, 오류가 하나여도 늘 보인다. NN/g 도 요약이 「모든 오류를 한눈에」 준다고 한다. 언리얼 컴파일 결과 · VS Code Problems 창도 항목별 목록이다. **개수만 보이는 꼴**(VS Code 상태 표시줄 · Fluent MessageBar 묶음의 남은 수 배지)은 목록으로 들어가는 입구이지 목록을 대신하지 않는다(가능성 높음).
2. **접는 기준 수는 어디에도 없다**(확실 — GOV.UK 원문에 개수 제한 없음). Fluent 는 메시지가 여럿이면 심각한 것을 위로 · 선택으로 접고 남은 수 배지(검색 요약만 — 원문 미확인).
3. **줄마다 있는 표시는 그대로 둔다**(확실) — NN/g 「요약을 유일한 오류 표시로 쓰지 말라」 · GOV.UK 「요약 문구를 입력 옆 문구와 같게」.
4. **누르면 그 자리로**(확실 — GOV.UK 각 오류 = 그 칸 링크 · 언리얼 컴파일 결과 = 그 노드로 이동, 가능성 높음).

## 피어

| 피어 | 꼴 | 접기 | 줄 표시 함께 | 누르면 이동 | 출처 | 확신도 |
|---|---|---|---|---|---|---|
| GOV.UK Error summary | 맨 위 · 제목 + 오류마다 링크 한 줄 · 하나여도 표시 | 기준 없음 | 예 · 문구 같게 | 예 · 그 칸(여러 칸이면 첫 칸) | https://design-system.service.gov.uk/components/error-summary/ | 확실(메인 원문 확인) |
| NN/g 폼 오류 지침 | 요약 = 전체 그림 | 없음 | 예 · 요약만 쓰지 말 것 | 언급 없음 | https://www.nngroup.com/articles/errors-forms-design-guidelines/ | 확실(수집자 읽음) |
| Fluent 2 MessageBar | 쌓기 · 심각한 것 위 · 선택으로 접고 남은 수 배지 | 정한 수 없음 | 언급 없음 | 언급 없음 | https://fluent2.microsoft.design/components/web/react/messagebar/usage | 가능성 높음(검색 요약만) |
| Carbon notification | 여러 오류 모으기 지침 없음 · 콜아웃 쌓기 피하라 | — | — | — | https://carbondesignsystem.com/components/notification/usage/ | 확실(없음을 확인) |
| Atlassian section message | 여러 메시지 지침 없음 | — | — | — | https://atlassian.design/components/section-message/usage | 확실(없음을 확인) |
| Unreal 컴파일 결과 | 메시지 목록 창 | 없음 | 미확인 | 예 · 그 노드로 | https://dev.epicgames.com/documentation/unreal-engine/compiler-results-in-the-blueprints-visual-scripting-editor-for-unreal-engine | 가능성 높음(검색 요약만) |
| VS Code | 상태 표시줄 = 개수 → 누르면 Problems 창(전체 목록) | 없음 | 예 · 물결 밑줄 | 개수 → 창 · Alt+F8 로 차례 이동 | https://code.visualstudio.com/docs/editor/editingevolved | 확실(수집자 읽음) |

## 우리에게

편집 칸 머리 밑에 문제마다 한 줄(문구 = 그 줄 표시와 같게) · 누르면 그 줄로(접힌 칸 · 안에 든 프리셋은 펼쳐서) · 줄 표시는 그대로 · 문제가 없으면 안 보임. 접는 수는 관행 근거가 없어 정하지 않는다(길면 스크롤). 사용자 결정.

## 한계 · 공백

- light — 적대 리뷰 없음. JetBrains 검사 위젯 · Polaris 는 보지 않았다. 읽은 페이지도 요약 도구를 거친 것이라 인용은 바이트 대조가 아니다.
