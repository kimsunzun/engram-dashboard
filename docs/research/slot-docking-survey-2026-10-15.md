# 슬롯 도킹 — 창을 끌어 다른 창 레이아웃에 붙이기 (피어 · 기술 타당성 · 크기 규칙)

- 상태: 적대 리뷰 반영 · **크기 규칙 확정(사용자 2026-10-15)** — 슬롯 옆이든 바깥 테두리든 **새것 = 1/3, 기존 = 2/3** 하나로 통일한다. px 최소 · 상한 · 옆 칸 양보 같은 예외 규칙은 두지 않는다(순수 비율 — Unreal 도 px 규칙 없이 비율만 쓴다 · 최소 0). 슬롯 옆 = 대상 슬롯만 나뉘고 다른 슬롯은 그대로 · 바깥 테두리 = 기존 레이아웃 전체가 안쪽 비율을 유지한 채 2/3 로 준다. 작은 대상에서는 결과도 작아지는데 이는 지금 분할 규칙(ADR-0227 결정 9 — 작은 칸 허용 · 폭 0 만 막음)과 같은 태도다. 거친 길: 30% → 퍼센트 + 기준 px(Unity 식 A) → 200px 고정 띠가 작은 창에서 0 이 되는 문제 → 순수 1/3 로 정리. §8 의 A · B · C 는 이 결정으로 닫혔다(공통 항목은 유효).
- 방법: `/research` medium · 설계-결정 모드. 수집자 3(피어 UI 관례 · Tauri 기술 타당성 · 우리 코드 실태) → 메인 grounding(원문 소스 직접 대조) → cross-family 적대 리뷰 1회
- 날짜: 2026-10-15
- 트래킹: T-63(슬롯 도킹 — `v0.3.3/chore/misc` 브랜치의 `docs/tracking.md`, master 미반영)
- 확신도 범례: 확실(독립 교차확증) · 가능성 높음(근거 지지 · 단일 출처) · 불확실 · contested

## 사용자가 정한 모양 (2026-10-15 대화)

- 도킹 단위 = **창 전체**. 슬롯 하나를 옮기려면 기존 팝아웃으로 먼저 따로 된 창으로 꺼낸 뒤 그 창을 붙인다.
- 창을 끌어 대상 슬롯 위에 올리면 그 슬롯에 **상 · 하 · 좌 · 우 4칸** 가이드가 뜨고, 놓은 쪽으로 그 슬롯이 나뉘며 끼워진다.
- **창 바깥 테두리**에도 칸이 있어 거기 놓으면 레이아웃 전체의 그 변에 길게 붙는다(예: 아래 = 전체 폭 아래 띠).
- 사용자가 꼽은 핵심 쟁점 = **도킹할 때 크기를 어떻게 정하나**.

## 1. 크기 규칙 — 피어 (핵심)

| 피어 | 슬롯 옆(안쪽) 도킹 | 바깥 테두리 도킹 | 미리보기 | 근거 · 확신도 |
|---|---|---|---|---|
| **KDDockWidgets** | 떠 있는 창을 통째로 놓으면(`addMultiSplitter`) `DefaultSizeMode::FairButFloor` — **균등 몫을 주되, 끌고 온 것이 그보다 작으면 그 작은 크기를 쓴다** | 같은 drop 경로(같은 규칙) | 미확인 | `src/core/DropArea.cpp:461,466-467` · `src/KDDockWidgets.h:147-149` 원문 대조 — **가능성 높음** · ★떠 있는 창을 통째로 붙이는 우리 모양에 가장 가까운 피어★ |
| Qt Advanced Docking System | 대상 영역을 **반반** — `Size = (TargetAreaSize - handleWidth) / 2`(같은 방향) · `TargetAreaSize / 2`(다른 방향). ★조건부★ — 여러 영역을 품은 떠 있는 컨테이너면 반반 조정을 건너뛰고 내부 배치를 보존할 수 있다(리뷰 지적 · 메인 미대조) | 크기 지정 없이 splitter 기본값(실값 미확인) | 영역 모드 = 대상의 절반 · 컨테이너 모드 = 1/3 비율 | `src/DockContainerWidget.cpp:705,738,833,844` 원문 대조 — **가능성 높음** |
| FlexLayout (caplin) | **반반** — `moveNode.setWeight(this.getWeight() / 2); this.setWeight(this.getWeight() / 2)` | 미확인 | 대상의 절반 영역 | `src/model/TabSetNode.ts:539-540` 원문 대조 — **가능성 높음** |
| rc-dock | 같은 방향이면 **새 패널 30%**(`newPanel.size = box.size * 0.3; box.size *= 0.7`) · 다른 방향이면 120 : 280(= 30%) | 루트(dockbox)도 같은 30% | 미확인 | `src/Algorithm.ts:283-284,296-298,310-311,325-326` 원문 대조 — **가능성 높음** |
| Lumino (JupyterLab) | 대상의 절반(미리보기 기준) | **새 패널 ≈ 38%** — 기존 루트를 합 1 로 정규화한 뒤 새 패널 sizer `GOLDEN_RATIO = 0.618` 을 넣고 다시 정규화 → 0.618 / 1.618 ≈ 0.382 | 루트 칸 미리보기 = 패널 높이 × 0.618(실제 삽입 비율과 다를 수 있음 — 미확인) | `packages/widgets/src/docklayout.ts:926-928,1489` 원문 대조 — **가능성 높음** |
| VS Code(편집기 그룹) | 미리보기 50% | 바깥 칸 없음(요약 기반 — 불확실) | 결과 절반 영역 | `editorDropTarget.ts` 요약 — 불확실 |
| Dockview | `Sizing.Distribute` 로 넣음(동작 미확인) | 바깥 칸 있음(`edges`) · 크기 미확인 | `size` 옵션으로 조정 · 작으면 얇은 테두리 | 문서 — 불확실 |

- **안쪽 도킹 크기는 관례가 하나로 모이지 않는다** — 반반(Qt ADS 영역 하나짜리 · FlexLayout · VS Code · Lumino 미리보기) · 새 패널 30%(rc-dock) · 균등 몫이되 끌고 온 크기가 작으면 그 크기(KDDockWidgets)로 갈린다.
- **끌고 온 창 크기를 반영하는 피어가 있다** — KDDockWidgets. 초안의 「찾지 못했다」는 리뷰가 반증했다.
- **크기를 기억하는 피어** — AvalonDock 은 창마다 `DockWidth`/`DockHeight` 와 `FloatingWidth`/`FloatingHeight` 를 따로 저장하고, JetBrains 는 「도구 창마다 크기 기억」 설정이 있다. 단 이것이 「다시 붙일 때 그 크기를 쓴다」는 뜻인지는 **확인하지 않았다**(저장과 재사용은 다른 말 — 리뷰 지적).
- **바깥 테두리 도킹도 갈린다** — Lumino 새 패널 약 38% · rc-dock 30% · KDDockWidgets 균등 몫(작으면 끌고 온 크기) · Qt ADS 는 미리보기 1/3, 실제 크기는 splitter 기본값(미확인). 반반을 쓰는 피어는 확인하지 못했다.
- **최소 크기 처리** — 확인한 코드에서 드롭 시점의 명시적 처리를 찾지 못했다(Dockview = 격자 잎의 `minimumWidth`/`minimumHeight` 에 맡김 · FlexLayout = weight 하한 1).
- **옆 슬롯** — Qt ADS 는 바깥 splitter 의 기존 크기를 되돌려 다른 영역에 영향이 없다. Lumino 는 비율대로 다시 정규화한다.

### 우리 쪽 현황

- 분할 비율 상수 `SPLIT_RATIO = 0.5`(`src-tauri/src/layout/tree.rs:23`) — 지금 `slot.split` 은 늘 반반이고, 분할 노드가 `ratio` 칸을 가져 반반 외 비율도 표현된다(`src-tauri/src/layout/types.rs:80-100`). 비율 조정 명령 `split.setRatio` 도 이미 있다.

## 2. 가이드 칸 배치 — 피어

- **안쪽 4방향 + 가운데** 가 공통이다. 가운데 = 그 영역에 **탭으로 끼우기**(Qt ADS · FlexLayout · Lumino · VS Code · Dockview). 우리 슬롯은 탭을 갖지 않으므로(탭은 창 단위 — ADR-0057) 가운데 칸은 그대로 옮길 수 없다.
- **바깥 테두리 칸**: Lumino(`root-top/left/right/bottom`) · Qt ADS(컨테이너 compass) · Dockview(`edges`)에 있다. VS Code 는 확인 못 함. 사용자가 원한 모양과 같다.
- **미리보기**: 넷 이상이 놓기 전에 결과 영역을 반투명으로 칠한다.
- **명령 API**: Lumino `addWidget(widget, {mode: 'split-left', ref})` · FlexLayout `MOVE_NODE`(대상 + 위치) · Dockview `addPanel`(기준 + 방향) — 「기준 + 방향」 꼴이 공통이라 LLM 명령(CLAUDE.md 「LLM-우선 제어」)에 그대로 맞는다.

## 3. 「OS 창을 끌어 붙이기」 선례

- **KDDockWidgets** — 떠 있는 창을 통째로 대상 레이아웃에 놓고 떠 있던 창을 지운다(`addMultiSplitter` 뒤 `scheduleDeleteLater` — `DropArea.cpp:466-469`). 우리가 정한 모양과 같다 — 가능성 높음.
- **AvalonDock(WPF)** — 떠 있는 OS 창을 끌면 `WM_MOVING` 마다 커서 위치로 가이드를 갱신하고 `WM_EXITSIZEMOVE` 에서 놓는다. 가이드는 별도 최상위 overlay 창. — 소스 `LayoutFloatingWindowControl.cs` · 수집자 보고 — 가능성 높음
- **Chrome 탭 떼기 · 합치기** — 떼어 낸 창이 네이티브 이동 루프(`SC_MOVE`)를 돌고 마우스 훅으로 놓음을 감지, 끌려가는 창을 빼고 커서 아래 자기 창을 찾는다. — chromium 소스 · 가능성 높음
- **Qt ADS 의 떠 있는 위젯** — 실제 최상위 창이고 통째로 놓을 수 있다.
- **Visual Studio** — 창을 끌면 가이드 다이아몬드가 뜬다(문서). 크기 규칙 미확인.
- **VS Code 보조 창** — 창을 끌어 다시 붙이는 길은 없다(탭을 끌거나 창을 닫으면 돌아온다 — 2차 출처).
- Tauri 앱 · 라이브러리 중 네이티브 창 재도킹 선례는 **찾지 못했다**.

## 4. 기술 타당성 — Tauri v2 + WebView2 (Windows)

판정: **되지만 조건이 있다** — 네이티브 이동 루프를 그대로 두고 백엔드가 도킹을 몬다(AvalonDock · Chrome 과 같은 꼴).

- **「창 내용을 옮긴다」 = 모델 이동이다** — 슬롯 내용은 백엔드 레이아웃 모델의 데이터(`SlotContent`)이고, 기존 팝아웃이 이미 모델을 옮겨 새 창을 그린다(`apply::move_slot_to_window`). 그래서 도킹도 웹뷰를 옮기는 것(Tauri `Webview::reparent`)이 아니라 떠 있는 창의 슬롯 트리를 대상 뷰 트리에 끼우고 떠 있는 창을 닫는 일이다(리뷰가 이 선택을 명시하라고 지적).
- **제목줄 끌기와 `start_dragging` 은 같은 OS 이동 루프다** — tao `drag_window` 가 `ReleaseCapture` 뒤 `WM_NCLBUTTONDOWN`(HTCAPTION)을 보낸다(`tao-0.35.3/src/platform_impl/windows/window.rs:539,546` 대조) — 확실(소스).
- **Tauri 에 이동 시작 · 끝 이벤트가 없다** — `WindowEvent` = Resized · Moved · CloseRequested · Destroyed · Focused · ScaleFactorChanged · DragDrop(파일 드롭) · ThemeChanged(`tauri-runtime-2.11.3/src/window.rs` 대조). tao 는 `WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE` 를 내부 표시로만 쓴다(`event_loop.rs:1028,1036`). 그래서 끌기 끝을 알려면 HWND 서브클래스(`SetWindowSubclass`)로 `WM_MOVING`/`WM_EXITSIZEMOVE` 를 받아야 한다 — 이 저장소 규칙상 OS 층 crate `platform` 안(ADR-0266). Electron 은 `moved`(끝) 이벤트를 제공한다.
- **커서 위치** — `AppHandle::cursor_position()`(Windows = `GetCursorPos`)이 있고, 서브클래스 안에서 `GetCursorPos` 를 직접 부르는 편이 왕복이 없다.
- **끌리는 동안 대상 창은 포인터 이벤트를 못 받는다(가능성 높음)** → 가이드는 백엔드가 「이 창 · 이 칸에 가이드」를 대상 웹뷰에 알려 그 DOM 에 그린다.
- **커서 아래 판정** — `WindowFromPoint` 는 끌리는 창을 돌려주므로 자기 창 목록을 z-order 로 훑어 끌리는 창을 뺀다(Chrome 도 같다). 우리 모델이 창별 캔버스 크기와 슬롯 픽셀 사각형을 이미 갖는다(`ViewManager` 의 `set_window_canvas` · `slot_px`) — 판정 재료가 있다.
- **Esc 취소** — 네이티브 루프는 Esc 로 원위치하지만 `WM_EXITSIZEMOVE` 는 그대로 온다 → 취소와 놓기를 가려야 한다(Chrome = 키보드 훅).
- **가이드가 끌리는 창에 가려진다** — 대상 웹뷰 안에 그리면 끌리는 창이 그 위를 덮는다. 끌리는 창을 반투명으로 하거나(VS 식 — 미확인) 별도 overlay 창(투명 · 클릭 통과 — WebView2 투명 창 이슈 있음)을 쓴다.

### 스파이크로만 풀리는 것

1. 모달 이동 루프 동안 대상 웹뷰로의 emit 이 매끄럽게 흐르나(가이드 지연)
2. tao 가 이미 건 서브클래스 위에 우리 서브클래스를 얹어도 충돌이 없나 · `start_dragging` 뒤 tao 가 합성하는 `WM_LBUTTONUP` 이 엉뚱한 클릭이 되지 않나
3. Esc 취소 판별
4. 모니터마다 DPI 가 다를 때 슬롯 사각형 판정
5. 가이드 가시성(반투명 창 vs overlay 창)
6. 「끄는 동안 창 내용 표시」 OS 설정이 꺼져 있을 때
7. 서브클래스는 그 창을 만든 UI 스레드에서만 걸고 풀 수 있다 — 스레드를 건너 서브클래싱하는 것은 금지다(`SetWindowSubclass` 문서 · 리뷰 지적). 창이 사라지기 전 해제 순서도 정해야 한다.

## 5. 우리 코드 실태 (현재 무엇이 있고 없나)

- **있다**: 뷰마다 이진 분할 트리(`Slot | Split{dir, ratio, a, b}`) · `slot.split` · `slot.close` · `split.setRatio` · 슬롯 하나를 다른 창의 **새 탭**으로 옮기는 `slot.popout{to_window}`(버스 · LLM 도달) · 팝아웃 창 = 네이티브 제목줄 · 창 이동/크기 이벤트 연결(배치 기록용).
- **없다**: 「대상 슬롯 옆 특정 방향에 내용(서브트리)을 끼워 넣기」 연산(`split_in_tree` 는 원래 슬롯을 늘 `a`, 빈 새 슬롯을 `b` 에 둔다 — 방향 · 내용 인자 없음) · 「루트 전체를 감싸 바깥 변에 붙이기」 연산 · 뷰 / 여러 슬롯 서브트리를 창 사이로 옮기는 연산 · 끌기 · 가이드 코드 · HWND 사용.
- 원래 구상 문서도 VS 식 도킹을 적어 두었다(`docs/process/S0-view-phase/requirements.md:98-104`). 창 간 탭 이동은 ADR-0057 에서 「후속」으로 남았고 아직 없다.
- 렌더러 TRD 가 dockview · golden-layout · Lumino · FlexLayout 을 「자기 모델을 소유해서」 거부했다(`docs/process/S21-flat-split-renderer/trd.md:422`) — 라이브러리 채택이 아니라 패턴 차용 대상이다.

## 5b. 최소 크기 — 붙였더니 너무 작아질 때 (2026-10-15 추가 조사)

수집자 1(소스 직접 읽기) → 메인 원문 대조(tmux · dockview · KDDockWidgets · VS Code) → cross-family 리뷰.

| 꼴 | 피어 | 근거 · 확신도 |
|---|---|---|
| **(b)+(c) 받아들이고 이웃에서 빌린 뒤 최소에서 멈춤** — 모자라면 넘친 내용을 자르거나(overflow hidden · 스크롤) 창을 키운다 | KDDockWidgets(최소 80×90 px · 루트가 모자라면 창을 키움) · dockview(그룹 최소 100×100 px · 넘치면 잘림) · VS Code 편집기 그룹(최소 220×70 px · SplitView 는 넘치면 스크롤) · Lumino(「모자라면 전부 최소로」) · Qt ADS · AvalonDock(최소 25 px — 추정) | KDDW `src/core/layouting/Item.cpp:59` · dockview `dockviewGroupPanel.ts:21-22` · VS Code `browser/parts/editor/editor.ts:29` 원문 대조 — 가능성 높음 · Qt ADS · AvalonDock 는 불확실 |
| **(d) 거절** — 두 쪽이 최소에 못 미치면 나누지 않고 알린다 | tmux(「no space for a new pane」 · 최소 1칸) · Zellij(2×5 칸 미만이면 「CAN'T SPLIT!」) · WezTerm(0 칸이 될 때만 「No space for split!」) | tmux `layout.c:1694` 원문 대조 — 가능성 높음 · Zellij · WezTerm 은 수집자 소스 읽기 |
| **(b) 조건부** — 이웃에 여유가 있으면 빌리고, 최소 합이 안 들어가면 그대로 둔다(최소 미만 남음) | golden-layout(최소 폭 10 px · `respectMinItemSize`) | `src/ts/items/row-or-column.ts` — 수집자 · 리뷰 둘 다 같은 동작 서술 — 가능성 높음 |
| **(e) 처리 없음** — 늘 나뉘고 아주 작아질 수 있다 | sway(최소값은 손 크기조절만 막음) · i3 · Hyprland(추정) · FlexLayout 기본값(최소 1 px) | 수집자 소스 읽기 — 가능성 높음 ~ 불확실 |
| **(a) 가이드에서 그 칸을 숨김** — 놓으면 두 쪽이 안 들어가는 칸에는 그 방향 칸을 내놓지 않는다 | deepseek-harness 의 `ui-dockkit`(「An edge zone is only offered where the split it would make is allowed: within the pane budget and with room for two halves」) · 훅만 주는 쪽 = dockview `canDisplayOverlay` · KDDockWidgets 지시자 숨김 콜백(기본은 크기 검사 없음) | `packages/client/ui-dockkit/src/components/DockSurface.tsx` 원문 대조(리뷰가 찾음 · 메인 확인) — 가능성 높음 · 단일 출처 |
| **(d) GUI 쪽 거절** | Dock(Avalonia) `PreventSizeConflicts` — 고정 크기 도구끼리 크기가 충돌하면 도킹을 거절(리뷰 보고 · 메인 미확인 — 문서 페이지에서 문구를 못 찾음) | 불확실 |

- GUI 도킹 라이브러리는 대부분 **받아들이고 이웃에서 빌린다**. 거절은 터미널 멀티플렉서 쪽이 주류이고, GUI 에서 놓기 전에 칸을 숨겨 막는 선례는 하나(deepseek-harness `ui-dockkit`)를 확인했다.
- **우리 쪽 현재 규칙** — 30px 미만 칸을 만드는 **분할은 허용**하고, 막는 것은 한쪽 폭이 0 이 되는 극단뿐이다(ADR-0227 결정 9 · R11 — 사용자 「0이되는 극단적인 상황만 막으면 될것같아 일단은」). 칸 최소 30px 은 구분선 끌기에만 걸린다(ADR-0260).

## 6. 공백

- 미확인 피어: golden-layout · Unreal 의 크기 · 칸 규칙.
- AvalonDock · JetBrains · Visual Studio 가 저장해 둔 크기를 재도킹 때 실제로 쓰는지.
- Dockview `Sizing.Distribute` 의 실제 동작 · Qt ADS 컨테이너 도킹의 실제 크기.

## 적대 리뷰 결과

codex(cross-family) 1회 · effort high · 웹 검색 켬 — 판정 **FIX**. 반증 출처는 메인이 원문으로 다시 대조했다(KDDockWidgets · rc-dock).

| 지적 | 처리 |
|---|---|
| 「안쪽 도킹은 반반이 관례」 과장 — rc-dock 30% | 반영(원문 대조) — 「관례가 하나로 모이지 않는다」로 고침 |
| 「끌고 온 크기를 쓰는 피어 없음」 반증 — KDDockWidgets `FairButFloor` | 반영(원문 대조) — 표에 추가, 가장 가까운 피어로 표시 |
| Qt ADS 반반은 조건부(여러 영역 컨테이너면 내부 배치 보존) | 반영 — 메인 원문 미대조로 표시 |
| 크기 기억 대안 누락(AvalonDock · JetBrains · VS) | 반영 — 저장 사실만 적고 재사용 여부는 공백으로 |
| 「창 내용을 옮긴다」의 뜻(웹뷰 reparent vs 모델 이동) 불명 | 반영 — 모델 이동임을 명시(기존 팝아웃과 같은 길) |
| `SetWindowSubclass` 스레드 제약 누락 | 반영 — 스파이크 목록 7 |

### 5b 리뷰(codex 1회 · 판정 FIX — 반영)

| 지적 | 처리 |
|---|---|
| (a) 꼴 선례 누락 — deepseek-harness `ui-dockkit` | 반영 — 원문 대조로 확인 |
| golden-layout 을 (e) 로 분류한 것 — 조건부 처리다 | 반영 — 별도 줄로 |
| GUI 쪽 크기 거절 누락 — Dock(Avalonia) `PreventSizeConflicts` | 반영 — 메인 미확인으로 표시 |

확인된 쪽(리뷰가 못 때림): FlexLayout 반반 · Lumino 루트 약 38% · tao `drag_window` → `WM_NCLBUTTONDOWN` · Tauri `WindowEvent` 에 이동 시작/끝 없음.

## 7. 전반 관례 조사 (2026-10-15 셋째 라운드 — 게임 엔진 · 제작 도구 · IDE · 전문 툴킷)

수집자 2(엔진 · 제작 도구 / IDE · 툴킷 — 소스 직접 읽기) → 메인 원문 대조(Unity `SplitView.cs` · Eclipse `SplitDropAgent2.java` · JetBrains `WindowInfoImpl.kt`) → cross-family 리뷰. 사용자 지시: 「특정 것만 보고 다시 묻지 말고 성숙한 앱 전반을 조합해 최종안 몇 개로」.

| 피어 | 슬롯 옆 크기 | 바깥 테두리 크기 | 작은 대상 | 가이드 · 가운데 | 미리보기 | 단위 · 취소 | 근거 · 확신도 |
|---|---|---|---|---|---|---|---|
| **Unity 에디터** | **대상의 1/3, 단 최소 100px** | **고정 200px 띠**(띠 판정 70px) | 옆 칸이 각자 최소까지 양보 | 대상 가장자리 띠(1/3 · 최대 300px) · 가운데 = 탭 줄에 끼우기 | 정확한 결과 사각형 | 창(탭) 하나 · Esc 취소 | `Editor/Mono/GUI/SplitView.cs:14-18,315-316,371-372` 원문 대조 — 가능성 높음 |
| **Unreal(Slate)** | 대상과 같은 가중치 → 대개 반반 | 가중치 1.0(기존 자식들과 균등) | 최소 0(처리 없음) | 띠 = 대상의 0.3(5~150px) · 십자 + 바깥 칸 · 가운데 = 탭 | 대상의 절반으로 변형(실제와 다를 수 있음) | 탭 하나 · Esc 취소(가능성 높음) | 수집자가 엔진 소스 열람(`SDockingTabStack.cpp` · `SDockingArea.cpp` · `SDockingCross.cpp`) — 메인 미대조 · 가능성 높음 |
| Blender | 커서 위치로 정함(0.5 에 붙음 · Ctrl = 1/8 단위) | — | 들어갈 수 없는 방향 칸을 빼 버림(약 58px 미만 = 옆 칸 없음) | 가운데 = 내용 바꾸기 | 적용 비율 그대로(가능성 높음) | 영역 하나 · Esc · 우클릭 취소 | `screen_ops.cc` · 매뉴얼 — 수집자 |
| Adobe(Photoshop · AE) | 미확인 | 미확인 | 미확인 | 가장자리 = 옆에 · 그룹 칸 = 탭 | 미확인 | 그룹 손잡이로 통째 · **Ctrl 누르면 붙지 않고 이동** | 도움말 검색 조각 — 불확실 |
| Visual Studio | 미확인 | 미확인 | 미확인 | 다이아몬드 가이드 + 음영 | 음영(정확한지 미확인) | **Ctrl 누르고 끌면 붙지 않고 이동** · Ctrl+더블클릭 = 마지막 도킹 자리로 | MS 문서 — 확실(문서 문구) |
| **JetBrains** | 사이드 절반 | **기본 가중치 0.33** · 「도구 창마다 크기 기억」 켜면 다시 붙일 때 그 크기 | 미확인 | 사이드 줄무늬 | — | 도구 창 하나 · 취소 = 끌기 중단 | `WindowInfoImpl.kt:35,87,89` 원문 대조 · `ToolWindowManagerImpl.kt` — 가능성 높음 |
| **Eclipse 4** | 0.5 | **0.33**(바깥 띠 35px) | 미확인 | 나침반 없음 · 가장자리 띠 | 비율 표시 | — | `SplitDropAgent2.java:37,237,240` 원문 대조 — 가능성 높음 |
| AvalonDock | 미리보기 절반 | 미리보기 = `min(선호 크기, 절반)` | 최소 25px | 나침반 + 바깥 | 일반 절반 | 떠 있는 창 통째(내부 분할 유지) · **다시 붙일 때 마지막 도킹 크기 재사용**(가능성 높음) | 소스 — 수집자 |
| Qt ADS · KDDockWidgets | 반반 · 균등(작으면 끌고 온 크기) | 미리보기 1/3 · 균등 | 쪼그라듦 · 최소 80×90 + 창 키움 | 나침반 + 바깥 | 일반 · **정확(KDDW)** | 떠 있는 창 통째(내부 분할 유지) · Esc(ADS) | §1 · §5b |

**모이는 것**
- 바깥 테두리는 **새것을 작게** — 1/3 안팎(Eclipse 0.33 · JetBrains 0.33 · ADS 미리보기 1/3 · Lumino 38% · rc-dock 30%) 또는 고정 px(Unity 200px).
- 슬롯 옆은 갈린다 — 반반(Unreal · Qt ADS · Eclipse · FlexLayout) vs 작게(Unity 1/3 · 최소 100px · rc-dock 30%).
- **「퍼센트, 기준보다 작으면 기준 px」 은 Unity 가 그대로 한다**(대상의 1/3, 최소 100px).
- 떠 있는 창을 통째로 붙이는 툴킷(Qt ADS · AvalonDock · KDDW)은 **창 안의 분할 배치를 그대로 끼운다**. 엔진(Unreal · Unity)은 탭 하나씩만 옮긴다.
- 취소 = **Esc**(Unreal · Unity · Blender · Qt ADS). **Ctrl 누르고 끌면 붙지 않고 그냥 이동**(Visual Studio · Adobe).
- 명령 API = 「기준 + 방향 (+ 비율)」 꼴이 공통(Eclipse `insert(el, relTo, where, ratio)` · KDDW `addDockWidget(location, relativeTo)` · Unreal 레이아웃 빌더).
- 작은 대상 — 처리 없음(Unreal) · 옆 칸 양보(Unity) · 칸 빼기(Blender) · 창 키움(KDDW). 거절하는 GUI 피어는 없다.

## 8. 최종안 (조합 — 사용자 선택 대기)

**모든 안에 공통**(피어가 모이는 것 · 이미 정한 것)
- 단위 = 떠 있는 창 통째. 창 안의 분할 배치는 그대로 끼운다(Qt ADS · AvalonDock · KDDW). 탭이 여럿이면 보이는 탭의 배치를 끼우고 나머지 탭은 떠 있는 창에 남는다(우리 구조 사정 — 피어 선례 없음).
- 가이드 = 대상 슬롯의 상 · 하 · 좌 · 우 가장자리 띠 + 창 바깥 테두리 띠. 가운데 칸은 없다(우리 슬롯은 탭을 갖지 않는다).
- 미리보기 = 놓으면 생길 **정확한 사각형**(Unity · KDDW).
- 취소 = Esc. **Ctrl 누르고 끌면 붙지 않고 그냥 창 이동**(Visual Studio · Adobe).
- 남는 공간은 늘 기존 것을 밀어서 만든다 — 창을 꽉 채우는 분할이라 넘침은 없다(사용자 결정).
- LLM 명령 = 「창 · 기준 슬롯(또는 바깥 변) · 방향」(Eclipse · KDDW · Unreal 의 공통 꼴).
- 선택 추가 — **마지막 도킹 자리로 되돌리기**(Visual Studio Ctrl+더블클릭 — 떠 있음 ↔ 마지막 도킹 자리 전환). 팝아웃한 슬롯을 원래 자리로 돌리는 데 쓸 수 있다.
- 참고: Eclipse 는 바깥 위쪽 띠를 꺼 두었다(왼 · 오 · 아래만) — 바깥 네 변을 다 둘지는 구현 때 정한다.

| | **A. Unity 식** | **B. 반반 + 바깥 1/3** — 슬롯 옆 반반 = Unreal · Qt ADS · Eclipse 부분 스택 · 바깥 1/3 = Eclipse(`0.33`) · JetBrains 기본 가중치 | **C. 크기 기억** — JetBrains(선택 설정) · AvalonDock(소스상 가능성 높음) |
|---|---|---|---|
| 슬롯 옆 | 목표 = 대상의 1/3, 단 최소 100px(Unity 의 드롭 사각형 — 실제 크기는 옆 칸 최소 때문에 달라질 수 있다) | 대상의 절반 | 그 창이 마지막으로 붙어 있던 크기 · 처음이면 A 규칙 |
| 바깥 테두리 | 고정 200px 띠 | 창의 1/3 | 마지막 크기 · 처음이면 A 규칙 |
| 작은 대상 | 대상이 최소를 못 채우면 같은 분할의 옆 칸이 각자 최소까지 양보(Unity) | 처리 없음(지금 분할 규칙 그대로 — 폭 0 만 막음 · ADR-0227) | A 와 같음 |
| 장점 | 사용자가 말한 「퍼센트 + 기준 px」 와 같다 · 기존 레이아웃이 가장 덜 흔들린다 | 가장 단순 · 지금 `slot.split` 과 같은 반반 | 팝아웃했다 되돌릴 때 원래 크기로 돌아온다 |
| 단점 | 숫자 셋(1/3 · 100px · 200px)을 정해야 한다 | 슬롯 옆에 붙이면 대상이 크게 줄어든다(사용자가 고른 「새것을 작게」 와 어긋남) | 창마다 크기를 저장해야 한다 · 팝아웃 창은 그때그때 생겨 「그 창의 마지막 크기」 가 무엇인지 정의가 필요 |

### 7 · 8 리뷰(codex 1회 · 판정 FIX — 반영)

| 지적 | 처리 |
|---|---|
| Unity 100px 은 드롭 사각형이지 최종 크기 보장이 아니다 | 반영 — A 를 「목표」로 고침 |
| C 의 근거 과장 — JetBrains 는 선택 설정, AvalonDock 은 저장 상태 문서뿐 | 반영 — C 근거 표시를 낮춤(AvalonDock 재사용은 수집자 소스 읽기 · 메인 미대조) |
| B 의 바깥 1/3 을 Unreal 에 귀속 | 반영 — Eclipse · JetBrains 로 한정 |
| Eclipse 바깥 위쪽 띠 꺼짐 | 반영 — 참고 줄 |
| 어느 이웃이 양보하나 미정 | 반영 — A 에 「같은 분할의 옆 칸」 |
| 마지막 도킹 자리로 되돌리기 누락 | 반영 — 공통 선택 추가 |

## 9. 쓸 수 있는 라이브러리 (2026-10-15 넷째 라운드)

수집자 1 → 메인 대조(AgentCat PR #36 · drag-rs 저장소 사실) → cross-family 리뷰.

**판정: 핵심을 해 주는 라이브러리는 없다 — 패턴만 빌리고 `windows` crate 를 `platform` 안에서 직접 쓴다.** 후보는 전부 자기 레이아웃 모델을 갖거나, 끌기를 DOM 에서 시작한다(네이티브 제목줄 이동이 아니다).

| 후보 | 주는 것 | 맞음 | 판정 |
|---|---|---|---|
| `windows` crate(이미 `platform` 의존) | `SetWindowSubclass` · `DefSubclassProc` · `GetCursorPos` | 그대로 맞음(OS 코드는 `platform` 안 — ADR-0266) | **쓴다** |
| tao(Tauri 창 층 · Apache-2.0) | `WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE` 를 안에서만 쓰고 이벤트로 안 낸다 · 끝에 합성 `WM_LBUTTONUP` | 우리 서브클래스는 메시지를 보기만 하고 `DefSubclassProc` 로 넘겨야 tao 표시가 안 깨진다 | 참고만 |
| AgentCat PR #36(Tauri 앱 · 머지 2026-08-01) | 창 서브클래싱으로 `WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE` 를 받아 네이티브 이동 루프를 끌기 수명의 정본으로 쓴다 · Windows 밖은 웹뷰 pointerup 으로 물러남 | 같은 꼴의 실제 Tauri 선례(별 4 · 라이선스 표기 없음) | 패턴 차용 |
| crabnebula `drag-rs` · `tauri-plugin-drag-as-window`(MIT/Apache-2.0 · 활발) | DOM 요소에서 OLE 끌기를 시작해 대상 웹뷰가 `DragDropEvent::Over`(위치 포함)를 받는다 | 끌기 시작이 DOM — 제목줄 끌기가 아니다. 탭 손잡이로 끌기를 시작하는 설계라면 대안이 된다 | 지금 설계엔 해당 없음 |
| Tauri `startDragging()` · `WindowEvent::Moved` · 커서 위치 API | 같은 네이티브 이동 루프 · 이동 중 위치 | 끝 이벤트 없음 | 배관으로 쓴다 |
| Atlassian pragmatic-drag-and-drop(Apache-2.0 · 활발) | 가장 가까운 변 계산(약 10줄) · 드롭 표시 부품 | 포인터 이벤트로만 움직인다 — 네이티브 이동 중 웹뷰는 포인터 이벤트를 못 받는다 | 패턴만 |
| dnd-kit · react-dnd | 문서 안 끌기 | 위와 같음 | 해당 없음 |
| dockview · FlexLayout · golden-layout | 여러 창(`window.open` 팝아웃) · 도킹 가이드 | 자기 모델 소유(렌더러 TRD 가 이미 거부) · 가이드가 모델과 엉켜 있다 · Tauri/WebView2 에서 `window.open` 팝아웃이 도는지 미확인 | 해당 없음 |

- Tauri 도킹 플러그인은 찾지 못했다.
- **공백** — macOS 의 이동 끝 감지 · dockview 팝아웃의 WebView2 동작 · AgentCat 코드 본문(설명만 읽음) · Tauri issue #10767(창 끌기가 포커스를 뒤집고 마우스 이벤트를 삼킨다 — 가이드와 엮일 수 있음 · 미조사).

### 9 리뷰(codex 1회 · 판정 FIX — 결론 유지, 보강 반영)

| 지적 | 처리 |
|---|---|
| Tauri 자체 런타임에 `SetWindowSubclass`/`DefSubclassProc` + `WM_NCDESTROY` 정리 구현이 있다 — 가장 가까운 참고 | 반영 — `tauri-runtime-wry/src/undecorated_resizing.rs:88-89,189,252,260` 로컬 대조 확인. **구현 참고 1순위** |
| `drag-rs` 에 `dragBack()`(창으로 되돌리기)도 있다 | 반영 — DOM 끌기 경로라 판정은 그대로 |
| dockview 는 팝아웃 되붙이기 · 위치 이벤트 · Tauri 전용 처리를 문서화했다 | 반영 — 모델 소유라 판정 그대로, WebView2 동작은 여전히 미검증 |
| winit 0.31 이 창 간 끌기 원천 · 드롭/취소 이벤트를 더했다(재도킹 요구 issue #3229 계기) | 반영 — 데이터 전달이지 제목줄 이동 추적이 아니고 Tauri 는 tao 위라 직접 해당 없음 |
| 서브클래스 래퍼 `michiu`(자기가 만든 창만) · `winsafe`(unsafe 노출) | 반영 — 대체 불가, 직접 `windows` crate |
