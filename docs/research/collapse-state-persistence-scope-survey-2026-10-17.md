# ★ 접고 펴기 상태를 기억하나 — 보기마다 · 항목마다 — light 조사

| 항목 | 값 |
|---|---|
| 상태 | 조사 완료 · 결정 대기(사용자) |
| 날짜 | 2026-10-17 |
| 방법 | `/research` light — 수집자 1명(검색 · 가져오기 약 10회 · VS Code 소스 1개 읽음) · 메인 grounding 스팟체크 1건(GOV.UK 아코디언 WebFetch) · 적대 리뷰 없음 |
| 질문 | 상세 카드 「참조」 안의 묶음(「▾ 프리셋 3」 · 「▸ 에이전트 17」)의 펼침 상태를 기억하나 — 안 함 · 보기마다 하나(항목끼리 공유) · 항목마다 — 그리고 어디에 저장하나 |

## 결론

- **보기(칸)마다 · 또는 종류마다 기억한다 — 항목마다는 거의 없다.** 보이는 항목을 바꿔도 묶음은 사용자가 둔 대로 남는다. VS Code = 칸 id 마다(파일마다 아님) · 언리얼 Details = 클래스마다 · Unity UI Toolkit = 요소 키마다. (가능성 높음)
- **기억하는 것이 기본이고, 안 하는 곳은 불평을 받는다** — Obsidian 백링크 「결과 접기」 는 새 탭 · 새 파일에서 풀려 포럼 요청과 그걸 고치는 플러그인이 있다. (가능성 높음)
- **저장 범위:** IDE 칸 = 창 · 작업 공간마다 오래 · 웹 아코디언 = 세션 동안 — GOV.UK: 「the accordion component will use session storage to remember which sections were open」(기본 켜짐 · `rememberExpanded: false` 로 끔). (GOV.UK = 확실에 가까움 — 메인 원문 확인)
- **묶음마다 고정 id 가 있어야 기억할 수 있다** — 자리 순서가 아니라 묶음 종류(「프리셋」 · 「에이전트」)로 키를 잡는다.

## 발견

| 도구 | 접히는 것 | 기억 | 범위 | 저장 | 출처 | 확신 |
|---|---|---|---|---|---|---|
| VS Code | 컨테이너 안 칸(탐색기 · 개요 · 타임라인) | 예 | 칸 id 마다(파일마다 아님) | 작업 공간 | https://raw.githubusercontent.com/microsoft/vscode/main/src/vs/workbench/browser/parts/views/viewPaneContainer.ts | 가능성 높음(접힘 깃발 저장 파일은 안 읽음) |
| Obsidian | 백링크 「Collapse results」 | 아니요 — 새 탭 · 파일에서 풀림 · 「Backlink Settings」 플러그인이 고침 | 칸 인스턴스 | 없음 | https://forum.obsidian.md/t/maintain-show-backlinks-in-document-settings-like-collapse-results-across-tabs/51506 · https://community.obsidian.md/plugins/backlink-settings | 가능성 높음 |
| JetBrains | 프로젝트 트리 | 도구 창이 열려 있는 동안만(포럼) | 도구 창 | 미확인 | https://intellij-support.jetbrains.com/hc/en-us/community/posts/207073755 | 불확실 |
| Unity | UI Toolkit Foldout | `viewDataKey` 를 주면 예 | 요소 키마다 | 미확인 | https://docs.unity3d.com/2023.2/Documentation/Manual/UIE-ViewData.html | 가능성 높음 |
| Unity | IMGUI Foldout | 아니요 — 직접 저장 | — | — | https://discussions.unity.com/t/editorguilayout-foldout-no-way-to-remember-state/36422 | 가능성 높음 |
| 언리얼 | Details 패널 분류 | 예 — 세션 사이에도(RestoreExpansionState) | 클래스마다 | 미확인 | https://dev.epicgames.com/documentation/unreal-engine/API/Editor/PropertyEditor/IDetailCategoryBuilder · https://issues.unrealengine.com/issue/UE-96324 | 가능성 높음 |
| macOS Finder | 정보 가져오기 구획 | 예 | 미확인 | 미확인 | https://macmost.com/the-three-different-types-of-get-info-window-in-the-mac-finder.html | 불확실 |
| GOV.UK | 아코디언 | 예(기본) | 아코디언 id · 페이지마다 | 세션 저장소 | https://design-system.service.gov.uk/components/accordion/ (메인 확인) | 확실에 가까움 |
| DNB Eufemia | 아코디언 | 선택(rememberState · 고유 id 필요) | id 마다 | 미명시 | https://app.unpkg.com/@dnb/eufemia@11.8.3/files/components/accordion/AccordionDocs.js | 가능성 높음 |

## 우리 설계에 주는 것

- 「참조」 묶음 펼침 = **라이브러리 번호(1~4)마다 · 묶음 종류(프리셋 · 에이전트)를 키로** 기억 — 항목을 바꿔도 그대로. 셸 화면 상태(칸 폭 · 정렬 · 마지막 폴더와 같은 자리)라 다시 켜도 남는다(IDE 칸 꼴).

## 공백

- VS Code 설정 편집기 묶음 · 참조 보기 · 접힘 깃발의 정확한 저장 키 · JetBrains Find Usages · Unity · 언리얼 저장 위치 · Finder 가 항목마다인지 · 탐색기 세부 정보 칸 미확인.
