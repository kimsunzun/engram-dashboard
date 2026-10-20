# 조사 — 프로세스 경계를 사이에 둔 탭 끌기: 끌기 상태 · 기하 · 판정의 주인을 어디에 두나 + 프론트 베이스 후보

- **상태:** 수집 4 갈래 · 메인 grounding · cross-family 적대 리뷰 대기 · 결정 대기(사용자).
- **날짜:** 2026-10-19
- **계기:** 슬롯 도킹 Chrome 식 끌기 구현(TRD 6.3 U8 까지) 실측에서 결함이 셸 ↔ 웹뷰 경계에서 반복됨 — ① 재부착 때 모델 쓰기가 웹 알림보다 먼저 그려져 0 번 자리 한 프레임 ② 웹이 낡은 · 0 폭 탭바 기하를 보고해 셸이 오판(B1) ③ 붙었다 11 ms 만에 떨어짐. 사용자: 「자꾸 버그가 튀어나오는 원인이 뭐야」 · 「우리랑 비슷한 다른 곳은 어떻게 했는지」 · 「베이스 삼아야 될 것도 하나 선정 — 프론트 단 · 성숙한 것 · rust 와 리엑트 분리되는 구조」.
- **방법:** 수집자 4(직접 피어 · 순서 어긋남 방지 기법 · 경계 결함 시험법 · 프론트 베이스 후보) → 메인 grounding(받아 둔 원문 사본 · 클론에서 줄 대조 — 아래 「grounding」) → cross-family 적대 리뷰(codex).
- **확신도 범례:** 확실 = 1차 출처 + 독립 교차확증 · 가능성 높음 = 1차 출처 하나(grounding 지지) · 불확실 = 미확인 · 추론.
- **짝 문서:** [slot-docking-chrome-desktop-drag-2026-10-18.md](slot-docking-chrome-desktop-drag-2026-10-18.md)(Chrome Views 메커니즘 — 여기서 되풀지 않는다) · [../refactoring/slot-docking-drag-code-audit-2026-10-18.md](../refactoring/slot-docking-drag-code-audit-2026-10-18.md).

## 결론

### A. 피어 — 끌기 도중 모델을 프로세스 경계 너머에서 고치는 곳은 없다

| # | 발견 | 출처 | 확신도 |
|---|---|---|---|
| A1 | **렌더러와 모델 주인이 갈린 피어는 끌기 내내 모델을 얼려 두고 놓을 때 한 번만 쓴다.** Chromium WebUI 탭바(렌더러 프로세스의 웹 페이지 ↔ 브라우저 프로세스 `TabStripModel`)는 끌기 동안 자기 DOM 만 옮기고 `finish()` 에서 `tabsProxy_.moveTab(id, dstIndex)` 를 한 번 부른다 · 취소는 DOM 을 원래 자리로. 모델 쪽 메아리(`onTabMoved_`)는 요소를 그 자리에 다시 놓을 뿐이라 이미 거기 있으면 아무 일도 없다 | `chrome/browser/resources/tab_strip/drag_manager.ts`(태그 120.0.6099.1) #224-240 · `tab_list.ts` #575-578 | 가능성 높음 |
| A2 | 같은 프로세스여도 성숙한 구현은 같은 규칙을 따른다 — Firefox 는 끌기 중 `translateX` 로 보이기만 옮기고 모델 이동(`moveTabTo` · `moveTabsBefore/After` · 창 사이 `adoptTab`)은 `handle_drop` 안에서만 · Windows Terminal 은 놓을 때 대상이 자리를 셈해 `RequestReceiveContent(src, dst, index)` 한 번 | Firefox `browser/components/tabbrowser/content/drag-and-drop.js` #105-273(모델 호출 0) · #274-555 · Windows Terminal `TerminalPage.cpp` #6050-6062 | 가능성 높음 |
| A3 | **탭바 기하는 그리는 쪽만 갖는다 — 주인에게 기하를 보내는 피어가 없다.** 자리 판정은 렌더러가 살아 있는 DOM · XAML 로 하고(`getBoundingClientRect` · `e.GetPosition(item)` + `ActualWidth`) 주인은 「탭 X → 자리 N」 또는 「창 밖 화면 점 P 에 놓음」만 받는다. 우리처럼 웹이 재서 디바운스해 보고하고 주인이 그것으로 판정하는 꼴은 피어 7 곳에 없었다 | 위 출처들 | 가능성 높음(없음의 증거 — 7 곳 표본) |
| A4 | **창 사이 붙기는 피어 전부가 OS 끌어 놓기 세션에 맡긴다** — 대상 창의 렌더러가 dragover · drop 을 스스로 받아 자리를 센다(WebUI 는 대상 탭바에 자리채움 탭 `PLACEHOLDER_TAB_ID = -1` 을 만들어 끈다). 커서 추종은 OS 끌기 그림(`setDragImage` · Vivaldi 는 탭 영역을 비트맵으로 떠 `tabsPrivate.startDrag` 에 넘김). 대가 = 「커서를 따라오는 진짜 창」 · 놓기 전 미리보기 · 끌기 중 스냅이 없다 — Windows Terminal 이 바로 이것을 미해결로 적는다(#16129 · #14900) | `drag_manager.ts` #15 · #120 · #302 · Vivaldi `tabs_private_api.cc` #902-949 · `tabs_private.json` #1101-1121(`onDragEnd` 의 `targetWindowId`) · microsoft/terminal#16129 | 가능성 높음 |
| A5 | **「커서를 따라오는 진짜 창 + 놓기 전 붙기」(Chrome Views 꼴)는 한 프로세스 네이티브 구현에만 있다** — Chrome Views · WinUI 3 TabView 뜯어내기 API(같은 프로세스). 네이티브 주인 + 웹 렌더러 앱에서 이것을 하는 곳은 0 곳 찾았다(VS Code 는 한 렌더러 안의 `window.open` 보조 창 · Wave 는 창 안 순서만 · Warp 는 요청 #9030). dockview 의 Tauri 실험이 우리 경계를 그대로 재고 「메인 창과 스크립트 못 하는 창(별 JS 맥락) 사이 끌기는 없고 앞으로도 없다」고 적는다 — **단 그것은 라이브러리가 JS 힙 안에서 끌기를 넘기는 길에 대한 말이다.** 우리처럼 셸(네이티브)이 끌기를 쥐는 길은 그 문서의 범위 밖이다 | dockview `packages/dockview-tauri-demo/docs/unscriptable-windows.md` #155-170(@`1eb9c69` 2026-10-17) · 베이스 갈래 수집 | 가능성 높음(0 곳 = 미발견 ≠ 없음 · WinUI 3 API 는 기억 기반 = 불확실) |
| A6 | Vivaldi 는 **창 UI 전체가 React 앱이고 창 · 탭 모델은 Chromium C++** 인 — 우리와 가장 닮은 분리 구조다. 브라우저 쪽은 끌기를 「탭 끌기 중」 깃발 하나로만 알고 끝을 JS 에 `onDragEnd(canceled, x, y, targetWindowId)` 로 돌려준다 · JS 가 새 창 / 이동을 정해 한 번 청하는 것으로 보인다(JS 묶음은 닫힌 · 축소 코드라 직접 못 읽음) | `tabs_private_api.cc` #946-949 · `tabs_private.json` #1101-1125 | C++ 쪽 = 가능성 높음 · JS 쪽 = 불확실 |
| A7 | 끌기 끝 사건이 안 오는 경우를 따로 지킨다 — Firefox 는 끌기 세션이 아직 있다고 우겨도 주 버튼 누름을 「끌기는 끝났다」의 증거로 보고 복구하며, **그때 낡은 좌표로 떼어 내는 길을 일부러 건너뛴다** | `drag-and-drop.js` #905-930 | 가능성 높음 |

### B. 경계를 둔 채 주인을 한쪽에 둘 때 쓰는 기법

| # | 기법 | 무엇을 막나 | 출처 | 우리 결함과 | 확신도 |
|---|---|---|---|---|---|
| B1 | **Wayland `configure(serial)` → `ack_configure(serial)` → commit** — 주인이 상태에 번호를 붙여 보내고, 그리는 쪽은 그 번호의 상태를 다 그린 뒤 번호를 돌려준다 · 여럿 밀리면 마지막만 응답해도 되고 낡은 번호 응답은 오류 | 주인이 다른 상태의 그림 · 기하로 판단하는 것 | `wayland-protocols` `stable/xdg-shell/xdg-shell.xml` #572-615 | ① · ② · ③ 전부 — 셸 판정은 「지금 모델 번호」로 그려진 뒤 잰 기하만 쓴다 | 가능성 높음(grounding = 원문 문구 대조) |
| B2 | Chromium surface 동기화(`LocalSurfaceId`) — 맞는 번호의 프레임이 올 때까지 옛 프레임을 계속 보인다 | 크기 바꿈 깜빡임 · 어긋난 프레임 | 구현자 노트 hackmd.io/@elkurin/HJ1yK-6jh · chromium.org surfaces 설계 문서 | ① — 같은 번호의 모델 + 끌기 상태가 다 올 때까지 그리지 않음 | 불확실(원 설계 문서 미발견 · 구현자 노트) |
| B3 | Chromium Viz — 적중 판정 기하를 **그 기하를 만든 프레임과 함께** 보낸다 · 믿을 수 없으면 「물어보기」(`kHitTestAsk`)로 빠진다 | 화면과 다른 기하로 판정 | `components/viz/common/hit_test/hit_test_query.cc` · `hit_test_data_provider.h` | ② — 0 폭 · 낡은 기하는 「모름」으로 | 불확실(헤더 · 검색 요약) |
| B4 | 반례 — Electron 끌기 영역: 렌더러가 보낸 사각형으로 주 프로세스가 판정 → 메시지가 밀리면 낡은 사각형(macOS 실시간 크기 바꿈) · Electron 은 프로토콜이 아니라 진단 플래그로 대응 | — | releases.electronjs.org/pr/53598 | ② 와 같은 부류의 알려진 결함 | 불확실(릴리스 노트) |
| B5 | Figma — 아직 응답 못 받은 자기 변경이 있는 속성은 서버 갱신을 버린다 · Replicache — 낙관 적용 뒤 서버 상태로 되감고 재적용해 **한 번에** 드러냄 · 게임 클라 예측 — 입력에 번호, 판정이 어느 입력 번호에 기댔는지 기록 | 상호작용 중 되튐 · 낡은 응답 | figma.com/blog/how-figmas-multiplayer-technology-works · doc.replicache.dev/concepts/how-it-works · gabrielgambetta.com/client-side-prediction-server-reconciliation.html | ③ 진단(어느 입력 번호의 판정인가) · 끌기 중 모델 메아리 무시 | 가능성 높음(구현자 글) |

- **메인 판단(출처 아님):** A 와 B 는 같은 결론의 두 얼굴이다. 피어는 「끌기 도중 경계 너머 모델 쓰기」를 아예 없애서 순서 문제를 지웠고(A1 · A2), 그것을 없앨 수 없는 시스템(창 관리자)은 번호 · 응답으로 묶었다(B1 · B2).

### C. 경계 결함을 시험으로 잡는 법

| # | 방법 | 잡는 것 | 출처 | 비용 | 확신도 |
|---|---|---|---|---|---|
| C1 | **fast-check `fc.scheduler()` + `scheduledModelRun`** — 비동기 해소 순서를 스케줄러가 고르고, 실패한 순서는 seed 로 줄여 재현. vitest 와 같이 돈다 | 「관련 사건보다 먼저 그림」 · 낡은 응답 | fast-check.dev/docs/advanced/race-conditions | 낮음 | 가능성 높음(API) · 끌기 적용 선례 없음 |
| C2 | Rust `proptest-state-machine` — 참조 상태 기계 대 실제 세션, 사건 열 무작위 생성 · 줄이기. 「메시지 k 를 늦게 배달」을 전이로 넣어야 지연을 모델링 | 셸 세션의 순서 의존 전이 | docs.rs/proptest-state-machine | 낮음–중간 | 가능성 높음 |
| C3 | 실제 세션의 IPC 메시지 기록(양쪽 · 번호 · 시각) → 가짜 전송에 재생해 회귀 시험 · 디버그 빌드에서 매 프레임 불변식(「그린 자리 == 모델 자리」) 단언 | 사람이 먼저 본 결함의 재발 | 선례 없음(설계 착상) | 낮음–중간(추정) | 불확실 |
| C4 | 결정적 시뮬레이션(turmoil · madsim · Antithesis) | 분산 · 동시성 | s2.dev/blog/dst | 높음 — 이 결함 부류엔 과함 | 가능성 높음 |
| C5 | 실 입력 OS 시험(Chromium `interactive_ui_tests` · Firefox mochitest) — 비싸고 바로 이 부류에서 흔들린다(Chromium 은 다중 프로세스 구성에서 탭 끌기 시험을 꺼 둔 적이 있다) | 끝단 배선 | codereview.chromium.org/2851333002 | 높음 | 불확실(검색 요약) |
| C6 | Tauri `mockIPC`(`@tauri-apps/api/mocks`) — invoke 가로채기 · 사건 부분 흉내. 순서 뒤섞기는 없어 C1 로 감싸야 한다 | 웹 쪽 응답별 동작 | v2.tauri.app/develop/tests/mocking | 낮음 | 가능성 높음 |

### D. 프론트 베이스 후보 — 「Rust 가 모델 주인 · 창마다 별 웹뷰」에서

| 후보 | 성숙도(2026-10-19) | 모델 주인을 밖에 둘 수 있나 | 창 사이 탭 끌기 | 판정 |
|---|---|---|---|---|
| dockview | ★3.5k · v8.6.0(2026-10-16) · MIT(엔터프라이즈 판 별도 상용) · React 16.8–19 | ✕ — 내부 상태 · `toJSON`/`fromJSON` 통째 교체 · 변경 알림에 내용 없음 | `window.open` 같은 프로세스 팝아웃만 · 별 웹뷰 사이 불가(자기 문서) | 베이스 부적합 — 단 Tauri 실험 문서가 우리 경계의 선행 연구 |
| FlexLayout | ★1.4k · v0.11.1(2026-09-26) · MIT · React 18/19 | △ — `onAction` 으로 모든 GUI 동작을 적용 전에 가로채 · 취소 가능 · `Model.fromJson` 재구성 | `window.open` + 포털(같은 JS 맥락)만 | 창 하나의 렌더러로는 가장 맞음 — 창 사이 끌기는 여전히 우리가 짠다 |
| rc-dock | ★0.8k · Apache-2.0 · 릴리스 없음(npm) · React ≥18(19 미검) | ○ — `layout` + `onLayoutChange` 제어 모드 | `rc-new-window`(같은 프로세스) | 제어는 맞으나 공동체가 작다 |
| Golden Layout | ★6.7k · 마지막 릴리스 2022-09 · React 바인딩 뺌 | — | — | 정체 — 제외 |
| react-mosaic | ★4.8k · v7.2.1 · 제어 컴포넌트 | ○ | 탭 · 팝아웃 없음 | 분할 트리만 |
| dnd-kit · pragmatic-drag-and-drop | 활발 | — | 포인터 기반 = 문서 하나 · HTML5 기반 = OS 끌기에 실리나 JS 객체는 못 넘김 | 원시 도구 |
| **Chromium WebUI 탭바**(참조 구현 · 라이브러리 아님) | Chromium 소스(main 에선 지워짐 · 120 태그) · BSD | ○ — 모델은 브라우저 프로세스, 웹은 그리기 + 한 번 청하기 | OS 끌기 + 대상 자리채움 탭 | **같은 분리 구조의 성숙한 실물** — 탭바 안 웹 쪽 설계의 본보기 |
| Vivaldi(참조) | 상용 · C++ 쪽만 소스 공개 | ○ — React UI ↔ Chromium 모델 | OS 끌기 | 구조는 가장 닮았으나 JS 를 못 읽는다 |
| VS Code(참조) | Electron | ✕ — 한 렌더러가 모델 주인 · 보조 창은 같은 렌더러의 `window.open` | ○(한 렌더러 안) | 「웹뷰를 하나로 모으는」 길의 선례 |

- **베이스 갈래 결론:** 그대로 가져다 쓰면 Rust 모델 주인 원칙(ADR-0035) · 창마다 별 웹뷰 구조(ADR-0057)를 지키면서 창 사이 끌기까지 해 주는 React 라이브러리는 없다. 라이브러리가 창 사이 끌기를 하려면 실시간 모델이 JS 맥락 하나에 있어야 한다(VS Code 꼴 — 우리 원칙과 정면). 그래서 「베이스」는 둘로 갈린다 — **창 하나 안의 렌더러**(FlexLayout 이 가장 맞음 · 지금 우리 직접 구현과 바꿀 값어치는 따로 재야 함)와 **경계 설계의 본보기**(Chromium WebUI 탭바 — 같은 분리 구조의 성숙한 실물).

### E. OS 끌기의 Windows 모양 — 커서 옆 점선 네모의 정체 (2026-10-20 추가 수집 · 메인 grounding = E1 · E2)

| # | 발견 | 출처 | 확신도 |
|---|---|---|---|
| E1 | **커서 옆 점선 네모 = OLE 기본 「이동」 커서**(화살표 + 점선 상자). 끌기 출처의 `IDropSource::GiveFeedback` 이 `DRAGDROP_S_USEDEFAULTCURSORS` 를 돌려주면 OLE 가 그린다 · 복사 = 상자 + 「+」 · 없음 = 금지 원. 끌기 그림을 달아도 커서는 따로라 안 없어진다 | MS Learn `IDropSource::GiveFeedback` · 수집자가 이 PC `ole32.dll` 커서 자원 1–7 을 그려 확인 | 가능성 높음 |
| E2 | **Chromium(= WebView2 · Electron/VS Code)은 늘 기본 커서를 쓴다** — `GiveFeedback` 이 무조건 `DRAGDROP_S_USEDEFAULTCURSORS`. 웹 페이지는 바꿀 수 없다. **Firefox 만 자기 엔진의 사설 훅으로 없앤다** — 탭 끌기에 `dt.mozCursor = "default"` → Windows 끌기 출처가 `IDC_ARROW` 를 `SetCursor` 하고 `S_OK` | `ui/base/dragdrop/drag_source_win.cc` #29-31 · Firefox `widget/windows/nsNativeDragSource.cpp` #95-114 · `drag-and-drop.js` #1248(main 2026-10-20 받아 grep) | 확실(소스 · 메인 대조) · WebView2 가 같은 길 = 가능성 높음 |
| E3 | 끌기 그림은 Windows 셸 도우미(`IDragSourceHelper::InitializeFromBitmap`)가 그린다 — 반투명 · 가장자리 흐림은 셸 모양이고 Firefox 도 같은 도우미를 쓴다. `setDragImage(요소)` 는 불투명 1.0 · 변당 8192 px 까지 · **10-16 조사의 「알파 0.75 · `<img>` 200×200」은 기본 그림 · 선택 끌기 몫이라 `setDragImage` 에는 틀렸다** | `os_exchange_data_provider_win.cc` #968-997 · Blink `data_transfer.cc` #123-128 · #160-161 · `drag_controller.cc` #110-117 | 가능성 높음 |
| E4 | 피어 모양 — Firefox = 160×90 페이지 축소판(불투명) · 화살표 커서 · 탭바 안에서도 그림 유지 · VS Code = 탭 요소 그대로(`setDragImage(tab, 0, 0)`) · 기본 OLE 커서(점선 상자 — 화면 실측 아님) · Windows Terminal = XAML 기본(미관측) | `drag-and-drop.js` #1254-1332 · vscode `multiEditorTabsControl.ts` #1309-1324 · terminal `TerminalPage.cpp` #5942-6000 | 가능성 높음 |
| E5 | 끌기 도중 그림 바꾸기 — 네이티브는 된다(Firefox 가 다시 `InitializeFromBitmap` + 대상 도우미 Leave/Enter 흉내) · 웹 콘텐츠(Chromium/WebView2)는 `dragstart` 에서만 | Firefox `nsNativeDragTarget.cpp` · Blink | 가능성 높음 |
| E6 | Esc 와 「창 밖 놓기」가 HTML 에선 같은 신호(`dragend` · `dropEffect "none"`) — Firefox 는 사설 `mozUserCancelled` 로 가른다 · 창 밖 놓기 때 Windows 셸이 실패(되튐) 애니메이션을 그린다 — Firefox 는 사설 `mozShowFailAnimation`, VS Code 는 Chromium 패치로 막았다는 진술(미확인) | `drag-and-drop.js` #672-690 · `drag_source_win.cc` #17-20 · github.com/zcbenz/dnd-drag-hint | 가능성 높음(VS Code 패치 = 불확실) |

- **뜻:** WebView2 안의 HTML5 끌기로 낼 수 있는 상한은 「VS Code 수준」(점선 상자 커서 · Esc 와 창 밖 놓기 구분 불가 · 창 밖 놓기의 되튐 애니메이션)이다. Firefox 수준(화살표 커서 · 사설 구분)은 끌기 출처를 네이티브로 쥐어야 나온다 — WebView2 의 끌기 출처는 WebView2 브라우저 프로세스 안이라 우리 셸이 훅할 수 없다.

### F. VS Code 탭 끌기 벤치마크 — 소스 (2026-10-20 · ADR-0296 「VS Code 꼴」의 기준)

출처 = microsoft/vscode main `6c8316e`(2026-10-19) · 경로는 `src/vs/` 기준 · ETC = `workbench/browser/parts/editor/editorTabsControl.ts` · METC = `…/multiEditorTabsControl.ts` · EDT = `…/editorDropTarget.ts`. 메인 grounding = ★ 표시 줄(클론에서 줄 대조).

| 동작 | VS Code | 출처 |
|---|---|---|
| 끌기 시작 | 탭마다 `draggable` · 문턱 없음(브라우저 기본) · 빈 탭바 영역도 `draggable` = **그룹(칸) 통째 끌기** | METC #1009-1012 · #265-269 · ETC #437-439 |
| 끌기 그림 | 탭 폭 `fit`(기본) = 탭 요소 그대로 `setDragImage(tab, 0, 0)` · `shrink` · 다중 선택 · 그룹 = 글자 라벨 `applyDragImage` → `setDragImage(el, -10, -10)` ★ · 라벨 = `padding 1px 7px · 둥글기 10px · 12px · 최대 폭 120px · 말줄임` | METC #1310-1323 · `base/browser/ui/dnd/dnd.ts` #9-31 · `dnd.css` #6-28 |
| 탭바 위 끼울 자리 | 탭 가운데 기준 왼/오른쪽 · 2px 세로선(왼쪽 이웃 `::after` + 오른쪽 이웃 `::before`) · 빈 영역 = 맨 끝 | METC #1481-1508 · #1449 |
| 탭 위에 머물면 | **1500 ms** 뒤 그 탭을 연다(`DRAG_OVER_OPEN_TAB_THRESHOLD`) ★ · 창을 앞으로 올리는 동작은 없다 | METC #105 · #1360-1366 |
| 편집 영역 나누기 | 가장자리 **10%**(탭 끌기 · 그룹 끌기는 선호 축 30%) ★ · 띠 안 방향 = 가로 1/3 · 2/3 경계로 좌/우, 가운데 세로 1/2 로 위/아래 · 미리보기 = 나누기 50% · 합치기 100% · 덮개 150 ms 페이드 · 이동 70 ms · 덮개 `pointer-events:none` · 덮개에 `mouseover` 가 오면 300 ms 뒤 치움(멈춘 덮개 방지) | EDT #402-503 · #77 · #213-224 · `editordroptarget.css` |
| 자기 칸 위 | 이동이고 출처 칸 편집기가 1 개뿐이면 덮개 없음(칸이 비게 되므로) | EDT #172-180 |
| 놓기 | 놓은 쪽 `drop` 이 `moveEditor(…, {index})` 한 번 · 빈 칸은 닫힘(`closeEmptyGroups` 기본 true) · **메인 창은 마지막 칸을 지우지 않음** ★ · 보조 창은 마지막 칸이 빠지면 창이 닫힘 | METC #2726-2794 · `editorPart.ts` #822-826 · `auxiliaryEditorPart.ts` #399-434 |
| 창 밖에 놓기 | 출처의 `dragend` 가 처리 · 조건 = 「어느 창도 dragover 중이 아님」(`isWindowDraggedOver` — 모든 창의 capture dragover/dragleave 를 `BroadcastChannel` 로 공유하는 불리언 하나) · 커서 = 네이티브 `getCursorScreenPoint` · **활성 · 포커스 창의 바깥 사각형 안에서 놓으면 새 창을 만들지 않음** ★ · 새 창 자리 = 커서 − (요소 폭/2, 30 + 요소 높이/2) · 크기 = 1024×768 기본 | ETC #480-529 ★(#505-511) · `workbench/browser/dnd.ts` #724-787 |
| Esc | 따로 다루지 않음 — `dragend` 가 `dropEffect` 를 안 보므로 창 밖에서 Esc 해도 새 창이 열릴 수 있음(추론 · 미검) | METC #1371-1383 |
| 제목 표시줄 | 창 끌기 영역(`-webkit-app-region: drag`)은 **제목 표시줄에만** · 편집기 탭바엔 `app-region` 0 곳 · 빈 탭바 두 번 누름 = 새 문서 | `titlebarpart.css` #57-65 · METC #428-456 |
| 창 사이 자료 | `LocalSelectionTransfer`(프로세스 전역 싱글턴)에 기대는데, 보조 창이 `window.open` 으로 **같은 JS 맥락**이라 된다 — 우리(창마다 별 웹뷰)는 못 쓴다. dragover 중엔 `getData` 가 막혀 MIME 이름만 볼 수 있다 | `platform/dnd/browser/dnd.ts` #408-448 |

- **우리와 다른 점(결정에 걸리는 것):** ① VS Code 는 메인 창 마지막 탭도 끌 수 있다(빈 칸 하나가 남음) — 우리는 메인 ≥ 1 탭 규칙(ADR-0057)으로 끌기를 막기로 했다(사용자 2026-10-20). ② 빈 탭바 = 칸 통째 끌기 손잡이이지 창 이동 영역이 아니다 — 제목 표시줄을 걷을 때(T-66) 「빈 탭바로 창 끌기」와 겹친다. ③ 창 사이 「끄는 중」 불리언을 `BroadcastChannel`(웹 ↔ 웹)으로 나눈다 — 우리 「끌기 중 셸 호출 0」 불변식과 맞는 길이다.

## 설계 함의 (선택지 재료 — 결정 아님)

| | 방식 | 우리 결함 ①②③ | 비용 · 위험 |
|---|---|---|---|
| **F (피어 합의)** | **끌기 중 모델을 얼린다.** 셸은 「끌기 덧층」(끄는 탭 · 지금 있는 창 · 자리) 하나를 번호와 함께 쥐고 모든 창이 「모델 + 덧층」 한 스냅숏으로 그린다 · 모델 쓰기는 놓을 때 한 번 · 취소 = 덧층 버림. 탭바 안 자리 판정은 그 창 웹뷰가 자기 DOM 으로(A3 — 기하를 셸로 안 보냄). 창 사이 판정(어느 창 탭바 위인가)에만 셸이 기하가 필요 → 그건 B1 번호를 단 보고만 믿는다 | ① 사라짐(끌기 중 모델 쓰기 없음) · ② 범위가 창 사이 판정 하나로 좁아지고 번호로 막힘 · ③ 덧층 번호로 진단 · 막음 | 떼어 낸 창에 탭 내용을 띄우는 것도 덧층으로(모델에선 아직 원래 창) — 터미널 출력 구독이 덧층을 따라야 함(미검) · 끌기 중 LLM 버스 · 저장은 끌기 전 상태를 봄(원자성은 좋아짐) · Chrome Views 는 끌기 중 모델을 고친다 — 그것과 다르다(사용자 결정 「크롬 소스 그대로」와의 관계 = 판단 필요) |
| **G (주인 한쪽 + 번호)** | 끌기 중 모델 쓰기는 지금처럼 두고, 셸 → 웹 모든 밀기에 번호 · 웹은 그 번호를 다 그린 뒤 기하와 함께 응답(B1) · 셸은 지금 번호의 기하로만 판정 · 웹은 같은 번호의 모델 + 끌기 상태가 다 올 때까지 옛 프레임(B2) | ① · ② · ③ 막힘 | 피어 선례 없음(창 관리자 선례만) · 모든 경로에 번호 배관 · 웹이 「그리지 않고 기다리는」 시한 정책 필요 |
| H (결함별 땜질) | 지금 구조 + 확정 결함 셋만 | ① · ③ 일부 | 작음 · 같은 부류가 다시 나온다 |
| ~~OS 끌어 놓기로 돌아감~~ | 피어 다수 방식 | 원천 제거 | 진짜 창 추종 · 놓기 전 붙기를 잃는다 — 사용자가 2026-10-18 에 Chrome Views 방식으로 정함 · 되돌림 |
| ~~웹뷰 하나 + `window.open`~~ | VS Code 꼴 · dockview/FlexLayout 이 창 사이 끌기를 해 줌 | 원천 제거 | 모델 주인이 JS 로 → ADR-0035 · LLM 우선 제어 원칙 위반 · 창마다 별 웹뷰 구조 갈아엎기 |

- **어느 쪽이든 함께:** C1(fast-check 스케줄러로 셸 ↔ 웹 사건 순서 뒤섞기) + C3(실측 세션 메시지 기록 → 재생 회귀) — 사람보다 먼저 잡는 그물.

## grounding (메인 · 원문 대조)

- Chromium WebUI 탭바(받아 둔 `drag_manager.ts` @120.0.6099.1 사본): `finish()` #225 안 `moveTab` #238 · `PLACEHOLDER_TAB_ID = -1` #15 · 자리채움 생성 #120 · `setDragImage` #302 · `TODO(crbug.com/843556)` #477 · `tab_list.ts` `onTabMoved_` #575-578 — 지지.
- Firefox `drag-and-drop.js`: `handle_dragover` #105-273 범위에 모델 이동 호출 0 · `handle_drop` #274 이후 `moveTabTo` #446 · `moveTabsBefore/After` #456-460 · `adoptTab` #543-553 · `replaceTabsWithWindow` #831 · 낡은 끌기 복구 #905-930(주석 「낡은 좌표로 새 창 떼기를 건너뛴다」) — 지지.
- Windows Terminal `TerminalPage.cpp`: `GetPosition(item).X` · `ActualWidth` #6050-6051 · `RequestReceiveContentArgs(src, WindowId, index)` #6062 — 지지.
- Vivaldi: `tabs_private.json` `startDrag` #648 · `onDragEnd` #1101 · `targetWindowId` #1121 · `tabs_private_api.cc` `CaptureVisible` #902 · `SetTabDragInProgress(true)` #948 · `StartDragging` #949 — 지지(C++ 쪽만).
- Wayland `xdg-shell.xml`(main 받아 grep): 「only has to ack the last configure event」 #572 · 「free to discard all but the last event」 #615 · `invalid_serial` #468 · #573 · #593 — 지지.
- dockview `unscriptable-windows.md` #163-166 문구 지지 — 범위(JS 힙 안 끌기)는 #155-161 문맥으로 메인이 좁혔다(A5).
- FlexLayout README #440 `onAction` · rc-dock README #100 `onLayoutChange` · Wave `layoutModel.ts` `pendingbackendactions` — 지지.
- B2 · B3 · B4 · C5 · 성숙도 수치 · WinUI 3 API 는 메인이 대조하지 않았다.

## 적대 리뷰 결과

(대기)

## 공백 · 한계

- Zed/GPUI · Tabby · Hyper · KDDockWidgets · Arc/Edge/Opera 미도달(대부분 한 프로세스라 분리 피어가 아님).
- Vivaldi JS 쪽(닫힌 축소 묶음) · Chromium `TabStripService` mojom 의 끌기 프로토콜 · `webui_browser/tab_strip.ts`(gitiles 503) 미독 — 마지막 것이 가장 가까운 선례일 수 있다.
- 피어의 경계 경주 버그 추적(crbug · Bugzilla)은 843556 외 미수집.
- 「끌기 중 모델 쓰기 + 번호 펜스」(G)를 실제로 하는 웹 렌더러 앱 선례는 못 찾았다.
- F 의 「떼어 낸 창 내용 = 덧층」이 우리 터미널 구독 · 출력 연속(ADR-0231)과 맞는지 미검.
