# ★ 접고 펴기 기억 — 브라우저(창)마다 따로인가 · 무엇을 기억하나 — light 조사

| 항목 | 값 |
|---|---|
| 상태 | 조사 완료 · 결정 대기(사용자) |
| 날짜 | 2026-10-17 |
| 방법 | `/research` light — 수집자 1명(검색 · 가져오기 약 8회) · 메인 grounding 스팟체크 1건(언리얼 `EExpansionPersistenceMethod` API 문서 WebFetch) · 적대 리뷰 없음 |
| 질문 | 라이브러리 1~4(언리얼 콘텐츠 브라우저 1~4 꼴)에서 접고 펴기 상태를 번호마다 따로 기억하나, 공통 하나인가 · 접히는 것 가운데 무엇을 기억하고 무엇을 안 하나 |
| 앞 조사 | `docs/research/collapse-state-persistence-scope-survey-2026-10-17.md`(보기마다 · 항목마다) |

## 결론

- **언리얼 Details 의 기본은 「종류마다 전역 공유」 — 창마다 따로가 아니다.** 원문: Default = 「Expansion will use default persistence (Global shared per struct type)」 · 창(뷰 id)마다 따로는 선택지로만 있다 — TransientPerViewIdentifier(편집기 세션 동안) · PersistentPerViewIdentifier(디스크). (확실에 가까움 — 메인 원문 확인 · 단일 출처) → 사용자 짐작(「보통은 브라우저 별로 저장할 것 같지는 않다」)과 맞다.
- **콘텐츠 브라우저 1~4 가 창마다 무엇을 따로 저장하는지는 확인 못 했다** — 인스턴스 이름 · 「저장된 설정」 개념은 있고(가능성 높음), `UContentBrowserSettings` 는 「global not per-project」 전역이다(확실 — API 문서). ini 구획 이름 · 창마다 들어가는 칸은 미확인(문서 403 · 소스 비공개).
- **기억 vs 안 함 — 확인된 것은 상세 · 인스펙터 묶음(기억)뿐.** 트리 펼침(VS Code = 다시 열면 복원 — 출처 약함) · 칸 머리 · 줄 펼침 · 메뉴/팝오버는 출처를 못 찾았다(기권).
- **수집자 경험칙(출처 없는 추론):** 오래 가는 구조(트리 노드 · 칸 · 묶음)는 (보기 id + 고정 요소 id)로 기억 · 내용에 딸린 줄 펼침과 메뉴 · 팝오버는 기억 안 함.

## 발견

| 무엇 | 창마다? | 저장 | 출처 | 확신 |
|---|---|---|---|---|
| 콘텐츠 브라우저 인스턴스 — 「global name of a content browser instance」 · 만들 때 기본값 대신 「saved settings」 사용 가능 | 예(이름 붙은 인스턴스 · 저장된 설정) | 미확인 | https://dev.epicgames.com/documentation/unreal-engine/API/Editor/ContentBrowser/IContentBrowserSingleton/CreateContentBrowser | 가능성 높음 |
| `UContentBrowserSettings`(엔진 폴더 보이기 · 컬렉션 도킹 …) | 아니요 — 전역 | 편집기 설정 | https://dev.epicgames.com/documentation/unreal-engine/API/Editor/UnrealEd/UContentBrowserSettings | 확실 |
| Details 묶음 펼침 | 기본 = 종류마다 전역 공유 · 선택 = 뷰 id 마다(세션 · 디스크) | 세션 / 디스크 | https://dev.epicgames.com/documentation/unreal-engine/API/Editor/PropertyEditor/FDetailsViewArgs/EExpansionPersistenceMethod (메인 확인) | 확실에 가까움 |
| 폴더 트리 펼침(VS Code) | 작업 공간마다로 추정 | 미확인 | — | 불확실 |
| 칸 머리 · 줄 펼침 · 메뉴/팝오버 | — | — | — | 기권 |

## 우리 설계에 주는 것

- 접고 펴기 기억 = **1~4 공통 하나 · 요소의 고정 id 를 키로**(언리얼 Details 기본 꼴). 창마다 따로는 하지 않는다.
- 공용 장치 하나(요소 id → 펼침)에 오래 가는 구조만 싣는다 — 왼쪽 칸 머리(라이브러리 · 즐겨찾기) · 상세 카드 「참조」 묶음(프리셋 · 에이전트) · 편집 칸 종류 칸 머리. 편집 칸 속 프리셋 줄 펼침 · 메뉴 · 👁 카드는 싣지 않는다. 폴더 트리 펼침은 근거가 약해 결정 대기.

## 공백

- 언리얼 ini 구획 · 창마다 저장되는 칸 · VS Code 탐색기 · 칸 접힘 저장(소스 `explorerView.ts` 를 읽으면 확인 가능) · Obsidian 줄 펼침 · 메뉴 비기억의 출처.
