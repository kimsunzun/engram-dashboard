# 분할 레이아웃 렌더러 — allotment 유지 vs 교체 피어 서베이

- 상태: 확정 조사 · **결정 = 지금은 A(비율 `defaultSizes`)로 쭈그러짐만 막고 머지 · 칸 재마운트를 없애는 구조 교체는 별도 주제로 설계부터 · 그 구조 교체 = D(평평한 잎 + 퍼센트 배치를 직접 구현) — 사용자 결정 2026-09-24** · 날짜: 2026-09-24 · 강도: medium · 설계-결정 모드
- 방법: 주계열(Claude) 수집자 3 갈래(react-resizable-panels 심층 / allotment 결함 이력 / 피어 앱·후보 라이브러리) + 메인 grounding(react-resizable-panels 4.13.3 배포본 소스 · allotment 1.20.5 설치본 · React 19 `react-dom-client.development.js` · react-mosaic `MosaicRoot.tsx`·`BoundingBox.ts`·`package.json` 원문 · 로컬 클론 orca·paseo·vibe-kanban 코드 · GitHub API·npm 으로 이슈 상태·저장소 수치) + cross-family(codex, effort high, 웹 검색) 적대 리뷰 1 회 — 판정 REFUTE-PARTIAL, 지적 11 건 전부 반영(과장 2 · 도출 누락 1 · 근거 없는 인과 1 · 출처 맥락 누락 1 · 누락 4 · 날짜 오류 1 · 통합 공백 1)
- 확신도 범례: 확실 = 독립 출처 2개 이상 또는 코드 직접 확인 / 가능성 높음 = 단일 출처로 지지 / 불확실 = 미지지·추론

## 질문

1. 칸을 닫아 분할이 합쳐질 때 잠깐 쭈그러진 화면이 보인다(사용자 실측 2026-09-24). 원인과 근본 해법은.
2. 분할 라이브러리를 react-resizable-panels 로 바꾸면 이런 부류가 원천 차단되나. 대신 무엇이 새로 생기나.
3. 같은 문제(외부가 소유한 분할 트리 · 칸 안에 터미널)를 피어는 어떻게 풀었나.

## 우리 현황 (코드 확인)

- 레이아웃 트리(분할 `id`·`dir`·`ratio`·두 자식)는 셸 백엔드가 소유하고 프론트는 스냅샷을 그리기만 한다(CLAUDE.md 「LLM-우선 제어」 · ADR-0035).
- 렌더러는 트리를 **재귀 컴포넌트**로 그린다 — 분할마다 `Allotment` 하나, 그 안에 자식 렌더러 둘(`src/components/layout/ViewLayoutRenderer.tsx:300-310`). allotment 를 쓰는 파일은 이것 하나고, CSS 는 `src/main.tsx:8` 이 import 한다.
- 분할 인스턴스 key = `${id}:${dir}`(ADR-0223). 조사 시점엔 초기 비율을 첫 칸의 `preferredSize` 퍼센트 문자열로 줬고 `defaultSizes` 는 쓰지 않았다(같은 파일 주석: 비율을 주면 ~1px 로 붕괴했다는 옛 실측). 이 조사의 선택지 A 로 `defaultSizes` 를 넣었다(같은 날 — step-log).
- 드래그한 크기는 백엔드로 되쓰지 않는다(렌더러 주석이 ADR-0063 범위 밖으로 적지만 ADR-0063 본문에 그 조항은 없다 — ADR-0227 에서 바로잡음).
- **탭은 이미 재마운트가 없다** — 탭 하나 = 뷰 하나(자기 분할 트리·포커스를 가진다)이고, 창의 모든 탭 화면을 절대배치로 겹쳐 두고 활성 탭만 `display` 로 보인다(`src/components/layout/WindowLayout.tsx:182-197` · ADR-0056 — 코드 확인). 숨은 탭은 WebGL 좌석만 반납하고 터미널 인스턴스는 살아 있다. 그러니 평평한 구조는 **뷰 하나 안에만** 적용하면 된다. 칸을 다른 창으로 떼는 팝아웃은 새 칸 id·새 웹뷰라 어느 구조로도 재마운트를 피할 수 없다(`src-tauri/src/layout/apply.rs:561-600` — 조사 서브에이전트 보고).
- **칸을 닫으면 살아남은 형제 서브트리가 통째로 재마운트된다** — 형제가 분할이면 새 `Allotment`·슬롯·xterm·재구독·replay 까지, 형제가 슬롯이어도 트리 깊이가 바뀌어 그 슬롯이 재마운트된다(원인 조사 서브에이전트 보고 · 후자는 ADR-0223 이전부터). ADR-0223 「영향」의 「승격된 서브트리 안 죽은 에이전트 보존 화면 소실」이 이 재마운트의 귀결이다.

## 발견

### F1. 쭈그러짐의 원인 — 새로 마운트된 `Allotment` 가 칸 크기를 정하기 전에 한 프레임을 그린다 — 가능성 높음

- `defaultSizes` 가 없으면 allotment 는 마운트 시 뷰 0개로 SplitView 를 짓고, 뷰는 ResizeObserver 콜백이 상태를 바꾼 **뒤의 React 재렌더**에서야 추가된다(`node_modules/allotment/dist/modern.mjs:1004-1060`, `:1132` — 코드 확인). 그 재렌더는 기본 레인이라 페인트 뒤에 돈다 — 그 사이 칸 컨테이너엔 left/width 가 없어 주축 방향으로 몰린다.
- **상류가 아는 미해결 결함이다** — 이슈 #820 「Content not full-size on first paint」(열림, 2024-09). 첫 페인트 전 크기 초기화를 넣었던 PR #526(v1.18.0)은 PR #805(v1.20.2)로 되돌려졌다(GitHub API 로 상태 확인 — 확실). #805 는 `preferredSize` 가 안 먹는 결함 #583 을 링크한다 — 되돌림이 그 회귀 때문이었을 가능성을 시사하지만 메인테이너의 사유 전문은 없다.
- 이 깜빡임이 닫기에서 새로 보이게 된 것은 ADR-0223 의 key 때문이다 — 그 전엔 같은 인스턴스를 재사용해 깜빡임 대신 크기가 틀렸다. 분할을 새로 만들 때는 원래부터 있었을 것이다(추론 — 불확실).
- 한 번 닫을 때 갱신이 여러 번 와서 중간 상태가 그려지는 것은 **아니다** — 백엔드는 `layout_updated` 를 한 번 보내고 프론트 스토어도 한 번 갱신한다(`src-tauri/src/layout/apply.rs:325-342` · `src/store/viewStore.ts:235-251` — 원인 조사 보고, 확실).
- 미검: 실제 WebView2 에서 칠해진 프레임을 찍지 않았다.

### F2. allotment 안에서 고치기(비율 `defaultSizes`) — 정적 분석상 틈이 닫힌다(조건부) · 재마운트는 남는다

- `defaultSizes` 는 픽셀이 아니라 **비율로 정규화된다** — 합이 전체 크기가 되고 `layout(e)` 가 칸마다 `round(비율 × e)` 로 배치한다(`modern.mjs:575-583, 655-666` — 코드 확인). 그러니 옛 주석의 설명(「`defaultSizes` 는 픽셀이라 0.2px 로 먹는다」)은 틀렸다.
- **옛 「~1px 붕괴」 관측의 원인은 모른다 — 불확실.** 수집 단계에선 「당시 CSS import 누락과 겹친 오염」으로 봤으나, 리뷰가 그 인과를 반박했다: `defaultSizes` 경로는 `addView(…, true)` 로 `relayout()` 을 건너뛰고(`:575-581`, `:634`) 배치는 `layoutViews()` 를 기다리므로(`:839-847`) 0.5px 칸이 칠해진다는 이야기가 코드에서 안 나온다. 지금은 CSS 가 import 돼 있다. **재현 여부는 실측으로만 가린다.**
- `defaultSizes` 를 주면 뷰가 마운트 layout effect 에서 동기로 만들어져 **재렌더 한 번이 빠진다**(확실).
- **남는 한 프레임이 있나 — 두 계열의 정적 도출이 「없다」로 맞는다(조건부 · 가능성 높음):** zustand 의 `useSyncExternalStore` 갱신은 SyncLane(레인 2)이고, React 19 는 레인 `& 3` 커밋의 passive effect 를 커밋 끝에 동기로 비운다(`react-dom-client.development.js:18323` — 코드 확인). 그래서 allotment 가 `useEffect` 에서 거는 ResizeObserver(`modern.mjs:230-249`)도 첫 페인트 전에 등록되고, ResizeObserver 규격상 첫 관측은 페인트 전에 배달되며 그 콜백이 곧장 `layout` 을 부른다(`:1125-1133`). 원인 조사(Claude)와 적대 리뷰(GPT)가 독립적으로 같은 결론을 냈다. 반대 의견(allotment 수집자 — 「RO 가 `useEffect` 라 한 프레임은 가능」)은 이 레인 동작을 보지 않았다.
  - **조건:** 컨테이너 크기가 0 이 아닐 것 · 갱신이 SyncLane 경로로 올 것 · WebView2 의 RO 타이밍이 규격대로일 것. 셋 중 하나가 깨지면(숨은 칸, 다른 스케줄 경로) 한 프레임이 남을 수 있다. **정적 결과이지 화면 증거가 아니다 → 실측으로 확정한다.**
- 이 길로는 **닫을 때 형제 서브트리 재마운트(현황 마지막 항목)가 그대로 남는다** — 쭈그러짐은 사라져도 재마운트된 터미널이 replay 가 올 때까지 비어 보인다.
- allotment 는 저활동 유지보수 상태다 — 최신 1.20.5(2025-12-19), 그 뒤로 의존성 갱신 커밋만, 이슈·PR 100 여 개 열림, 방향 전환 지원 요청(#341·#457)은 2022 년부터 열림이고 상류의 권고 우회책이 `key={vertical}` 재마운트다(GitHub API 확인 — 확실).

### F3. react-resizable-panels(v4) — 첫 페인트·방향 부류는 막는다 · 재마운트는 렌더러 모양에 달렸다 — 가능성 높음

- **첫 페인트:** 칸은 첫 렌더부터 CSS(`flexBasis` = `defaultSize`, 없으면 `flexGrow:1`)로 그려지고, 실제 측정·배치는 **useLayoutEffect** 안에서 페인트 전에 끝난다(배포본 `dist/react-resizable-panels.js:3`(import `useLayoutEffect as et`) · `:1703`(`V = window ? et : Se`) · `:2011`(그룹 등록이 `V(() => …)`) · `:2338-2372`(칸 스타일 폴백) — 코드 확인). 단 그룹 크기가 0 으로 측정되면(숨은 서브트리) RO 를 기다린다(`:1661` `defaultLayoutDeferred`). 화면 실측은 없다.
- **방향:** `orientation` 이 그룹 등록 effect 의 의존성이라(`:1911` 기본값 · `:2073-2080` 의존 배열 — 코드 확인) 바뀌면 React 자식 재마운트 없이 그룹만 다시 등록된다. 크기가 유지되는지는 실행하지 않았다(가능성 높음).
- **재마운트 — 백엔드 트리를 그대로 비추는 재귀 렌더러라면 못 막는다(가능성 높음):** 형제가 승격되면 살아남은 서브트리의 React 조상 경로가 바뀌고, React 는 조상 경로가 바뀐 요소의 상태를 보존하지 않는다(React 문서 「Preserving and Resetting State」 위치 규칙). 도착지의 type·key 를 고정해도 조상 경로 전체가 같아야 하므로 소용없다.
  - **단 「중첩 Group 으로는 불가능」은 과장이다(리뷰 지적 반영):** 옛 조상 `Group`/`Panel` 을 통과 노드로 남겨 조상 경로를 유지하면 보존할 수 있다. 대가 = 화면의 React 트리가 백엔드 트리와 어긋나고, 통과 노드 정리·크기 계산·백엔드 매핑을 렌더러가 따로 관리해야 한다(미구현 — 가능성 높음). 이 변형은 allotment 에도 같은 이론으로 적용된다.
- **백엔드 정본과의 동기화는 공짜가 아니다:** `defaultSize` 는 초기값일 뿐이라 백엔드 비율이 바뀌면 `groupRef.setLayout()` 을 명시로 불러야 하고, 안정된 칸 id 와 「프로그램 변경 vs 사용자 드래그」 콜백 정책(`onLayoutChanged(layout, {isUserInteraction})`)을 렌더러가 짜야 한다(API 는 있다 — 수집자 보고 · 리뷰 지적).
- 성숙도: 4.13.3(2026-09-23) · MIT · 단일 메인테이너(Brian Vaughn) · 5.4k★ · 열린 이슈 6 · 주간 다운로드 약 2,680만(allotment 약 19만) · peer `react ^18 || ^19`(npm·GitHub API — 확실). v4(2025-12)는 큰 파괴적 변경(`PanelGroup`→`Group`, `direction`→`orientation` 등)이었고 한동안 회귀가 이어졌다(성능 #531·#572, 조건부 칸 #565·#691·#717, 멈춘 리사이즈 #729 — 대부분 닫힘 · 수집자 보고).
- 단위: px·%·em·rem·vh·vw 를 받는다(`.d.ts` — 수집자 보고).
- shadcn/ui 의 현재 Resizable 은 react-resizable-panels **^4** 를 감싸고 Tailwind v4 로 스타일한다(shadcn 저장소 `new-york-v4/ui/resizable.tsx` — 수집자 보고).

### F4. 피어 — 터미널 분할 트리를 다루는 앱은 대부분 자체 엔진이고, 라이브러리는 평평한 앱 셸에 쓴다 — 확실(무엇을 쓰나) / 불확실(왜)

| 앱 | 방식 | 출처 |
|---|---|---|
| orca (Electron) | 자체 명령형 칸 관리자 · 비율 = `style.flex = "<비율> 1 0%"` | 로컬 클론 `orca/src/renderer/src/lib/pane-manager/pane-divider-drag.ts:88-89`(확인) |
| paseo | 자체 `split-container.tsx` · 비율 → `flexGrow` + `flexBasis: 0` | 로컬 클론 `paseo/packages/app/src/components/split-container.tsx:852-860`(확인) |
| t3code | 자체 flex 레이아웃 · 중첩 트리가 아니라 평평한 터미널 그룹 + 그룹당 방향 하나 | 수집자 보고 |
| vibe-kanban | react-resizable-panels ^4.12.4 — 작업공간 셸 | 로컬 클론 `packages/web-core/package.json:83`(확인) |
| Wave Terminal | 블록은 자체 TileLayout, 작업공간 셸만 react-resizable-panels | 수집자 보고 |
| Hyper · Tabby · VS Code · Zed | 전부 자체 구현(Hyper `split-pane.tsx` · Tabby `splitTab.component.ts` 퍼센트 배치 · VS Code gridview/splitview · Zed GPUI `pane_group.rs`) | 수집자 보고 |
| Theia · JupyterLab | Lumino(명령형 위젯 툴킷) | 수집자 보고 |

- 어느 피어도 선택 사유를 적어 두지 않았다 — 「터미널 트리엔 라이브러리가 안 맞아서」는 해석이다.

### F5. 재마운트 없는 모양이 있다 — **평평한 잎 목록 + 트리에서 계산한 퍼센트 절대 배치** — 확실(react-mosaic) / 가능성 높음(Tabby)

- react-mosaic 은 트리를 걸으며 잎마다 경계 상자를 계산하고, 잎을 **잎 id 로 key 를 단 한 줄 목록**으로 렌더한다. 각 잎은 `top/right/bottom/left: X%` 절대 배치다. 소스 주석이 그대로 적는다: 「트리 어디로 옮겨도 재마운트도 DOM 이동도 없다(iframe 이 다시 로드되지 않는다)」(`libs/react-mosaic-component/src/lib/MosaicRoot.tsx:55-63, 83-90` · `util/BoundingBox.ts:90-98` — 원문 확인).
- 이 모양이면 세 부류가 **구조적으로** 사라진다: ① 첫 페인트 — 측정 없이 CSS 퍼센트만으로 그린다 ② 방향 전환 — 방향은 경계 상자 계산의 입력일 뿐이다 ③ 닫기·분할·승격 때 살아남은 칸의 재마운트 — 잎의 key 와 부모(루트)가 바뀌지 않는다. ③이 곧 ADR-0223 「영향」의 죽은 에이전트 보존 화면 소실까지 푼다(추론 — 가능성 높음).
- **대가:**
  - DOM 순서가 레이아웃 순서가 아니게 된다(탭 포커스·스크린리더 순서 — react-mosaic 주석이 스스로 적는 트레이드오프).
  - **구분선 드래그와 최소 크기는 알고리즘이다(리뷰 지적 반영):** 잎의 CSS 최소값은 트리 구분선을 막지 못한다. 분할마다 자손 최소값을 위로 올려 합치고, 드래그 가능 범위·구분선 두께를 계산하고, 포인터·키보드 조작과 비율 동기화를 짜야 한다 — 지금 allotment 가 대신 해 주는 일이다(`modern.mjs:713-777, 784-875`). 구분선 상호작용만 떼어 주는 헤드리스 부품(예: Zag splitter)도 있으나 트리 전체 제약은 여전히 우리 몫이다.
  - **터미널 크기 재맞춤은 여전히 일어난다(리뷰 지적 반영):** 퍼센트 경계는 소수 픽셀이 되고, FitAddon 은 픽셀을 버림해 행·열을 정한다(`node_modules/@xterm/addon-fit/lib/addon-fit.js` · `src/components/slot/TerminalSlot.tsx:93-119`). 잎 정체가 안정돼도 리사이즈 때 셀 경계를 넘나드는 재배치·PTY 리사이즈 트래픽은 막지 못한다 — 이건 어느 방식에서나 생기는 일이고 재마운트와는 다른 축이다.
- react-mosaic 자체: 7.1.0(2026-09-10) · Apache-2.0(LICENSE 원문 확인 — GitHub 자동 식별은 `NOASSERTION`) · 4.8k★ · 트리를 제어 값(`value` + `onChange`)으로 받는다(수집자 보고) · peer `react 16 - 19`(`package.json` 원문 확인).
  - **의존성이 무겁다:** `react-dnd` 계열 다섯(`react-dnd`·`dnd-core` 16.0.1 고정·html5/touch/multi 백엔드) + `lodash-es`·`uuid`·`immutability-helper`(`package.json` 원문 확인). **`react-dnd` 는 16.0.1(2022-06) 이후 새 배포가 없다**(npm 확인). 드래그앤드롭을 안 써도 설치 그래프에 딸려 온다.
  - 창 장식(`MosaicWindow`)은 선택이다 — `MosaicRoot` 는 넘긴 잎을 그대로 그린다(`MosaicRoot.tsx:83-90`). Blueprint 는 선택이지만 패키지 스타일시트는 레이아웃 조작부를 보이려면 필요하다(README·테마 가이드 — 리뷰 보고).

### F6. 그 밖의 후보

- 외부 소유 트리에 안 맞는다(가능성 높음 — 수집자 보고): dockview(VS Code gridview 이식 · 자기 모델 소유 · 명령형) · golden-layout(명령형 · 마지막 릴리스 2022-09) · Lumino(비 React 명령형 툴킷) · FlexLayout(자기 JSON 모델이 정본).
- 정체: Split.js 1.6.5 · react-split 2.0.14 — 둘 다 약 5년 전 배포가 마지막이다(npm — 리뷰 보고. 초안의 「2018 이후」는 틀렸다).
- **react-split-pane v3(3.2.0) 은 평가하지 않았다** — 제어 크기·중첩·사용자 구분선·키보드 리사이즈를 문서가 적는다(리뷰 보고). 중첩 모양이라 F3 의 재마운트 문제를 공유할 가능성이 높지만 확인하지 않았다. rc-dock 도 내부 미확인.

## 선택지 (제약 적합도)

제약: ⓐ 백엔드가 트리 정본, 프론트는 그리기만 ⓑ 첫 페인트가 맞다 ⓒ 제자리 방향 전환 ⓓ 닫기·분할 때 살아남은 칸(터미널)을 재마운트하지 않는다 ⓔ 드래그 크기 되쓰기로 갈 길 ⓕ 스택(React 19·Tailwind v4·shadcn)·의존성 ⓖ 바꾸는 규모

| 선택지 | ⓐ | ⓑ | ⓒ | ⓓ | ⓔ | ⓕ | ⓖ |
|---|---|---|---|---|---|---|---|
| A. allotment + 비율 `defaultSizes` | ○ | ○ 정적 분석(실측 필요) | ✕ key 재마운트 유지 | ✕ | △ 명령형 resize 필요 | ○ | 한 파일·속성 하나 |
| B. react-resizable-panels v4 중첩 | ○ | ○ | ○(가능성 높음) | ✕ 그대로 비추면 / △ 옛 조상을 통과 노드로 남기면 | △ `setLayout` 동기화를 짜야 함 | ○ shadcn 이 감쌈 | 렌더러 한 파일 + allotment 하네스 12건 재작성 |
| C. react-mosaic | ○ 제어 값 | ○ | ○ | ○ | ○ `onChange` | △ React 19 peer ○ · 정체된 `react-dnd` 계열 딸림 | 트리 변환 + 하네스 재작성 |
| D. 자체 구현(평평한 잎 + 퍼센트, orca·Tabby·mosaic 모양) | ○ | ○ | ○ | ○ | 직접 짠다 | ○ 의존성 0 | 렌더러 재작성 + 구분선 드래그·트리 최소 크기 알고리즘 직접 |

- **A 는 이번 증상만 줄이는 국소 수정**이다. B 는 쭈그러짐·방향 부류를 막지만 재마운트는 렌더러를 백엔드 트리와 어긋나게 짜야 막는다. **재마운트를 모양 자체로 막는 것은 C·D 다.**
- 거부 후보(ADR 거부 대안 후보): dockview·golden-layout·Lumino·FlexLayout — 자기 모델을 소유하거나 명령형이라 ⓐ 와 충돌 · Split.js·react-split — 정체.

## 쟁점·한계

- 어느 선택지도 WebView2 에서 칠해진 프레임을 찍어 보지 않았다 — A 의 「틈이 닫힌다」는 조건부 정적 결과다.
- 옛 「~1px 붕괴」의 원인은 모른다(F2).
- react-resizable-panels 의 방향 전환 뒤 크기 보존, B 의 통과 노드 변형, react-split-pane v3·rc-dock 내부는 미확인이다.
- 피어의 선택 사유는 어디에도 적혀 있지 않다.
