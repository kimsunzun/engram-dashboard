# 맨 위 「전체 추가」 와 칸별 `+` 를 함께 두나 (2026-10-19)

| 항목 | 값 |
|---|---|
| **상태** | 조사 완료 · 지금 설계(PRD R23 — 편집 칸 머리 「+ 추가」 = 전체 검색 · 칸 제목 `+` = 그 종류만) 그대로 둠 |
| **방법** | `/research` light — 수집자 1(검색 + 공식 문서 읽음) + 메인 grounding(Epic 문서 원문 확인) |
| **질문** | 고정 종류 칸(지침 · 스킬 · 설정 · 프리셋)으로 나뉜 편집 화면에서 맨 위 전체 추가와 칸별 `+` 를 같이 두는 선례가 있나 |

## 결론

- **같이 두는 선례가 있다 — 언리얼 My Blueprint 패널**(확실 — 메인이 Epic 문서 확인): 패널 맨 위 「Add New」 단추 + 각 칸 제목(Graphs · Functions · Macros · Variables · Event Dispatchers) 끝의 `+` 단축 단추. 고정 종류 칸이라는 점까지 우리와 같다. 차이 = 언리얼 「Add New」 는 새로 만드는 메뉴이고(우클릭 메뉴와 같은 항목이라고만 적혀 있다 — 한 메뉴에 종류가 다 나오는지는 문서에 없다), 우리 것은 있는 항목을 검색해 붙인다.
- **하나만 두는 곳** — 전체 추가 하나: Unity Inspector 「Add Component」(가능성 높음 · 스니펫) · JetBrains Run/Debug Configurations 툴바 `+`(확실) — 둘 다 종류가 열린 목록. 칸별 `+` 만: Figma 속성 칸 Fill · Stroke · Effects(가능성 높음 · 스니펫 · 전체 추가가 없는지는 미확인).
- 약한 패턴(해석): 고정 종류 칸은 칸별 `+`, 열린 단일 목록은 전체 추가 하나. 디자인 시스템 지침은 찾지 않았다.

## 피어

| 피어 | 전체 추가 | 칸별 `+` | 칸 | 출처 | 확신도 |
|---|---|---|---|---|---|
| Unreal My Blueprint | 예 · 맨 위 「Add New」 | 예 · 각 칸 제목 끝 | 고정 | https://dev.epicgames.com/documentation/unreal-engine/my-blueprint-panel-in-the-blueprints-visual-scripting-editor-for-unreal-engine | 확실(메인 원문 확인) |
| Unity Inspector | 예 · 「Add Component」 | 없음 | 열림 | https://docs.unity.com/en-us/unity-studio/develop/ui-refs/inspector-ref | 가능성 높음(스니펫) |
| JetBrains Run/Debug Configurations | 예 · 툴바 `+` / Alt+Insert → 템플릿 목록 | 없음 | 열림(폴더로 묶음) | https://www.jetbrains.com/help/idea/run-debug-configuration.html | 확실(수집자 읽음) |
| Figma 속성 칸 | 미확인 | 예 · Fill · Stroke · Effects | 고정 종류 | https://help.figma.com/hc/en-us/articles/360049283914 | 가능성 높음(스니펫) |

## 한계

- light — 적대 리뷰 없음. Notion · VS Code · Postman · Blender 는 보지 않았다.
