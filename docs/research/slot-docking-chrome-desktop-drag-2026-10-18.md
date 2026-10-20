# 조사 — 슬롯 도킹을 데스크톱 Chrome 탭 끌기 그대로 (탭바 안 = 진짜 탭 · 밖 = 진짜 창이 커서를 따라옴 · 다른 창 = 놓기 전에 붙음)

- **상태:** 적대 리뷰 1 회(codex · level 4 · BLOCK = 「P 를 확정된 구현 경로로 쓰는 것」 — Chrome 모델 자체는 지지) 반영 · **결정(사용자 2026-10-18): Chrome 방식(P 계열) — 스파이크 S1–S6 먼저 · **Chrome 소스 그대로** — 메커니즘 · 상수(A 표)를 그대로 옮긴다 · P2 는 Chrome 과 달라 기각 · 스파이크에서 Chrome 방식이 이 스택에서 안 되는 곳이 나오면 대안을 지어내지 않고 멈춰 사용자에게(사용자 2026-10-18 「내가 크롬 소스보고 똑같이 하라고 했잖아」).**
- **날짜:** 2026-10-18
- **계기:** 9c 마우스 실측(사용자 · 2026-10-18)에서 결함 묶음 — 탭바 안 끌기에 OS 끌기 그림이 따라옴 · 커서 옆 점선 네모 · Esc 가 원래 자리로 안 돌아감 · 다른 창 위 게이지(붙기 전 기다림) · 탭 하나 창의 끌기 · 연달아 닫기에서 X 가 움직임 · 놓을 때 전환 · 활성 탭 구별 약함 · 도킹 영역이 넓어 새 창으로 빼려면 가운데까지 가야 함. 사용자: 「크롬 보고 좀 참고하라니깐 드럽게 다르게 구현하네」 · 「나중에 탑바 없애는것도 고려해서 설계해야된다」.
- **앞선 조사의 잘못:** [slot-docking-chrome-tab-drag-2026-10-17.md](slot-docking-chrome-tab-drag-2026-10-17.md) 는 Chrome 의 **터치 모드 WebUI 탭바**(`chrome/browser/resources/tab_strip/` · main 에서 지워짐)를 모델로 삼아 HTML5 끌어 놓기를 유지(선택지 A)했다. **데스크톱 Chrome(Views 탭바)은 OS 끌어 놓기를 쓰지 않는다** — 이 문서가 그 모델을 바로잡는다. 그 문서의 선택지 B(포인터 순서 이동 + 셸이 OS 끌기 시작)도 이 문서의 모델이 아니다(이쪽은 OS 끌기 자체가 없다).
- **방법:** 수집자 3(Chromium 소스 직접 읽기 · WebView2/Tauri 실현성 · 도킹 영역 관행) + 지금 코드 구조 점검 1(→ [../refactoring/slot-docking-drag-code-audit-2026-10-18.md](../refactoring/slot-docking-drag-code-audit-2026-10-18.md)) → 메인 grounding(Chromium 원본 `?format=TEXT` 로 받아 상수 · 호출 줄 대조 — 아래 「grounding」) → cross-family 적대 리뷰.
- **확신도 범례:** 확실 = 1차 출처 + 독립 교차확증 · 가능성 높음 = 1차 출처 하나(grounding 지지) · 불확실 = 미확인 · 추론.
- **출처 기준:** Chromium `main` 2026-10-18 · 경로 접두 `https://chromium.googlesource.com/chromium/src/+/main/` · 줄 번호는 그날 스냅숏. 끌기 코드는 지금 `chrome/browser/ui/views/tabs/dragging/` 아래다.

## 결론

### A. 데스크톱 Chrome 이 하는 일 (소스)

| # | 동작 | 내용 | 출처 | 확신도 |
|---|---|---|---|---|
| A1 | 입력 수단 | views 마우스 캡처(`TabDragController::SetCapture` → 위젯 `SetCapture`). OS 끌어 놓기는 창 이동 루프가 없는 플랫폼(Wayland 등)에서만(`ShouldDragWindowUsingSystemDnD`) | `tabs/dragging/tab_drag_controller.cc` #2729 · #1269 | 가능성 높음 |
| A2 | 끌기 시작 | 누른 점에서 유클리드 거리 > 10 px(`kMinimumDragDistance = 10` · 가로세로 같음) | 같은 파일 #1068-1072 | 가능성 높음(grounding 지지) |
| A3 | 탭바 안 | 진짜 탭 뷰가 커서를 계속 따라간다(`LayoutDraggedViewsAt` 매 이동) · 모델 순서는 마지막 이동점에서 `16 × 탭 폭 ÷ 표준 폭` 넘게 움직일 때만 바뀐다(떨림 방지) · 옆 탭은 애니메이션(200 ms) · 커서는 탭 안의 상대 위치(폭 비율)를 유지 | `dragging/dragging_tabs_session.cc` #66-165 · #89-104 · `tab_container_impl.cc` #1158 | 가능성 높음 |
| A4 | 전환 시점 | 비선택 탭을 왼쪽 **누르는 순간** 활성(`Tab::OnMousePressed` → `SelectTab`) · 뗄 때 다시 부르는 것은 다중 선택 접기용 · 가운데 클릭 닫기는 뗄 때(커서가 그 탭 위일 때만) | `tabs/tab.cc` #736-779(#768) · #787-826 | 가능성 높음(grounding 지지) |
| A5 | 떼기 조건 | 탭바 끌기 영역 가로 밖 또는 탭바 위아래 ±15 px 밖(`kVerticalDetachMagnetism = 15` · 터치 50) | `tab_drag_controller.cc` #398-401 · #1461-1477 · #233-240 | 가능성 높음(grounding 지지) |
| A6 | 떼면 | **그 즉시 진짜 새 브라우저 창**을 만들어 커서 밑에 두고(`CreateBrowserForDrag` · `SetBounds(점 − 끌기 오프셋)` · `Show`) Windows 창 이동 루프로 따라가게 한다 — `ReleaseCapture` 뒤 `SendMessage(WM_SYSCOMMAND, SC_MOVE\|0x0002, GetMessagePos())` · 끝내기 = `WM_CANCELMODE` · 루프가 취소를 알려 주지 않아 스레드 지역 마우스 · 키보드 훅으로 뗌 · Esc 를 가린다. **끌기 그림 없음**(그림은 OS 끌기 경로 전용) | `tab_drag_controller.cc` #1787-1955 · #1958-2049 · `ui/views/win/hwnd_message_handler.cc` #942-962 · #91-200 | 가능성 높음(grounding 지지 — `SC_MOVE \| 0x0002` 줄 #951) |
| A7 | 다른 창에 붙기 | 창이 움직일 때마다(`OnWidgetBoundsChanged` → `Drag(GetCursorScreenPoint())`) 커서 밑의 맨 위 Chrome 창(끄는 창 제외)을 찾고, 그 탭바(±15 px) 안이면 **타이머 없이 즉시** 붙는다 — 끄는 창을 숨기고 루프를 끝낸 뒤(Windows 는 루프가 끝난 다음) 탭을 그 탭바에 넣고 그 탭바에 캡처를 준다(`AttachImpl` → `SetCapture`) → 놓기 전에 그 창에서 순서 이동이 이어진다. 비워진 창은 숨겼다가 끌기가 끝날 때 닫는다. 타이머는 커서 밑 창을 앞으로 올리는 750 ms 하나뿐 | `tab_drag_controller.cc` #966-984 · #3002-3041 · #1148-1266 · #2023-2037 · #1651 · #1135 · #2161-2170 | 가능성 높음 |
| A8 | Esc · 취소 | 앱 전체 키 감시가 Esc = 되돌리기(`EndDrag(kCancel)` → `RevertDrag`: 원래 창 · 원래 자리 · 선택 · 최대화를 되돌린다) · 다른 키 = 놓은 것으로 끝 · 이동 루프 중 Esc 는 새 창을 숨기고 되돌린다 · 캡처 잃음 = 취소(창 이동 중엔 무시) · 세 번째 창에 붙은 뒤 Esc 가 첫 창으로 가는지는 코드가 `source_context_` 를 겨눈다는 데서 추론 | `tab_drag_controller.cc` #320-336 · #856-918 · #2113 · #2173-2275 · #1993-1996 | 가능성 높음(마지막 항목 = 불확실) |
| A9 | 탭 하나뿐 · 전부 선택 | 떼지 않고 **창 자체를 바로** 이동 루프로 옮긴다(최대화면 먼저 복원) | `tab_drag_controller.cc` #819-850 · #1789-1813 | 가능성 높음(grounding 지지 #819) |
| A10 | 연달아 닫기 | 마우스로 닫으면 「닫기 모드」 — 탭바 폭을 `지금 폭 − 닫힌 탭 폭 + 겹침` 으로 고정해 다음 탭이 커서 밑으로 미끄러져 온다 · **마우스가 탭바를 떠나야** 다시 배치(`MouseWatcher` · 아래 40 px · 새 탭 버튼 쪽 60 px 여유) · 마우스는 타이머 없음 · 터치는 2 초 · 맨 끝 보이는 탭을 닫으면 모드를 나간다 | `tabs/tab_container_impl.cc` #716-741 · #1425-1474 · #1549-1556 · #1572-1594 · `horizontal/horizontal_tab_closing_helper.cc` | 가능성 높음(grounding 지지 #1551 · #1576 · #1580) |
| A11 | 활성 탭 모양 | 위 모서리 반지름 10 · 아래 바깥 휘어짐 12 · 구분선 2×16(여백 2) · 겹침 18 · 표준 폭 232+24(DIP) · 배경이 보이는 탭(활성 · 선택 · 호버) 옆에는 구분선을 숨긴다(서서히) | `chrome/browser/ui/tabs/tab_style.cc` #22-45 · #97-126 · `tabs/common/horizontal_tab_style_views.cc` #65-280 · #510+ | 가능성 높음 |
| A12 | 탭바 = 제목 표시줄 | 탭바의 빈 곳 = `HTCAPTION`(창 이동 · OS 처리) · 탭 = `HTCLIENT`(탭 끌기) · 최대화가 아닐 때 비활성 탭 위 6 px 도 `HTCAPTION`(`kDragHandleExtension = 6` · 활성 탭은 아님) · 두 번 눌러 최대화는 Chrome 전용 코드가 없어 OS 기본 동작으로 추정 | `frame/browser_view.cc` #4633 · #4725-4735 · `tabs/tab_strip.cc` #1286-1292 · `tab_container_impl.cc` #47 | 가능성 높음(두 번 누름 = 불확실) |

> ★5.8판에서 맞춘 탭 폭 상수(`tabStripLayout.ts` 의 표준 160 · 활성 최소 42 · 비활성 최소 32)는 Chromium 규칙을 우리 크기로 옮긴 것이고, A11 의 232+24 는 Chrome 자신의 크기다 — 서로 어긋난 것이 아니다.

### B. 우리 앱(Tauri v2 · wry · WebView2 · Windows)에서 되나

| # | 질문 | 결론 | 출처 | 확신도 |
|---|---|---|---|---|
| B1 | 웹뷰가 쥔 포인터 캡처가 창 밖에서도 이어지나 | 된다 — Win32 호스팅 WebView2(wry 가 이것)에서 누름은 Chromium 위젯 HWND 의 `::SetCapture` 가 되어 바탕화면 · 다른 창 위에서도 `pointermove`(screenX/Y) 가 온다. 실패 보고는 WinUI3 호스팅뿐 | `content/browser/renderer_host/render_widget_host_view_event_handler.cc` #412-417 · `ui/aura/window_event_dispatcher.cc` #495 · `hwnd_message_handler.cc` #998-1005 · WebView2Feedback #1321 · #5701 | 불확실(리뷰 1 — S1 이 닫는다) |
| B2 | 웹이 캡처를 쥔 채 셸이 창을 매 이동마다 옮길 수 있나 | 된다고 보인다 — #5701 이 바로 그 꼴(Tauri 2.10 · 테두리 없는 창 크기를 `setPointerCapture` + 매 `pointermove` `SetWindowPos`)이고 기본 설정에서 동작한다(`AllowHostInputProcessing=true` 일 때만 깨지고 wry 는 그것을 켜지 않는다 — wry `src/webview2/mod.rs` #391-408). 우리 2026-10-16 스파이크의 invoke 왕복 p50 2.2 ms · 최대 약 50 ms 는 IPC 흔들림이 지연으로 보일 수 있다는 뜻 | 위 · `slot-docking-drag-mechanism-2026-10-16.md` | 가능성 높음 |
| B3 | 끌기 도중 출처 창을 숨기면 | **캡처가 죽을 수 있다(미검)** — Tauri `hide()` → wry `controller.SetIsVisible(false)` → Chromium 이 뷰를 숨기며 캡처를 놓는다. 우리 10-16 스파이크의 「출처를 숨겨도 끌기가 이어진다」는 **OLE 끌기**(캡처를 `DoDragDrop` 이 자기 창에 쥔다)에서 잰 것이라 옮겨 오지 못한다. Chrome 은 캡처를 먼저 옮긴 뒤에만 숨긴다 | wry · Chromium 소스 읽기 · `tab_drag_controller.cc` #1183-1199 · #1620-1632 | 불확실(리뷰 2 — S2 가 닫는다) |
| B4 | 붙은 뒤 다른 창 웹뷰가 끌기를 이어받을 수 있나 | **웹뷰끼리는 못 넘긴다고 봐야 한다** — 웹뷰 HWND 는 `msedgewebview2` 프로세스 것이라 셸이 거기에 `SetCapture` 할 수 없고, 누름을 보지 않은 웹뷰의 `setPointerCapture` 는 미검. 그래서 **가장 그럴 법한 갈래는 떼는 순간부터 끌기의 주인이 셸인 것**(커서 · 버튼 읽기 · 마우스 훅 · OS 이동 루프)이고 대상 웹뷰에는 그리기용 위치만 밀어 준다 — Chrome 에서도 끌기의 주인은 브라우저 프로세스다 | 추론 | 불확실(스파이크 필요) |
| B5 | 새 창에 OS 이동 루프를 끌기 도중 넘길 수 있나 | 그럴 법하다 — Tauri `start_dragging` = tao `handle_os_dragging`(`GetCursorPos` · `ReleaseCapture` · `PostMessage(WM_NCLBUTTONDOWN, HTCAPTION)`)로 Chrome 꼴에서 `SetBounds` 먼저만 빠졌고, 웹뷰 누름에서 이미 쓰인다(`data-tauri-drag-region`). Aero Snap 이 따라온다. 대가: tauri#10767(뗌을 먹고 포커스 깜빡임) · 루프를 끝내는 Tauri API 없음(`WM_CANCELMODE` = `platform` 함수 신설 · ADR-0266) · **캡처를 쥔 창과 다른 창에** 거는 것은 미검 · 루프 동안 invoke · emit 지연 미측정 | tao `src/platform_impl/windows/window.rs` #528-557 · tauri#10767 | 가능성 높음(다른 창 넘김 = 불확실) |
| B6 | 제목 표시줄 없앤 뒤 | wry 0.55.1 이 `ICoreWebView2Settings9` 가 있으면 `SetIsNonClientRegionSupportEnabled(true)` 를 이미 켠다 → 빈 탭바에 CSS `app-region: drag` = 창 이동 · 두 번 눌러 최대화 · 시스템 메뉴 · Snap. 탭은 `app-region: no-drag`(drag 영역은 포인터 사건을 먹는다). v152+ 의 `app-region` 상속 변경 주의(#5696). 셸 최상위의 `WM_NCHITTEST` 가로채기는 웹뷰가 덮은 곳에선 안 먹는다(자식 HWND 가 다른 프로세스) | MS Learn `icorewebview2settings9` · wry `mod.rs` #600-602 · WebView2Feedback #5696 · #3605 · [custom-titlebar-tauri-2026-10-18.md](custom-titlebar-tauri-2026-10-18.md) | 가능성 높음 |
| B7 | 웹 기반 선례 | Chrome 처럼 창 사이 놓기 전 붙기를 하는 Electron · Tauri · 웹 앱은 못 찾았다 — Firefox = OS 끌기(끝에 새 창) · Tabby = 창 안 순서만(떼기는 요청 #581) · VS Code · Chromium WebUI 탭바 · Windows Terminal = OS/HTML5 끌기 | Tabby `appRoot.component.pug` #34-37 · Eugeny/tabby#581 · 앞선 조사 | 불확실(미발견 ≠ 없음) |

### C. 도킹 영역 크기 관행

| 제품 | 붙는 영역 | 미리보기 | 표식 | 새 창 | 확신도 |
|---|---|---|---|---|---|
| VS Code 편집기 그룹 | 가장자리 10%(그룹 끌기 한 축 30%) · 나머지 = 합치기 | 분할 50% · 합치기 100% — 붙는 영역보다 크다 | 없음(암묵) | 이 파일엔 없음 | 가능성 높음(요약 읽기 — 상수는 원본 재확인 필요) |
| dockview | 가장자리 `activationSize` 기본 20% · 나머지 = 가운데 | 덮개 크기 `size` 는 따로 | 없음 | 떠 있는 그룹은 별 기능 | 20% = 가능성 높음 · `size` 기본값 = 불확실 |
| FlexLayout | 대각 4 삼각형 + 가운데 0.25–0.75 | 절반 | 없음 | 팝아웃 별 기능 | 가능성 높음 |
| Lumino / JupyterLab | 위젯 = 3 등분 · 뿌리 가장자리 px(위 12 · 나머지 40) | 절반 · 뿌리 0.618 | 없음 | 없음 | 상수 = 가능성 높음 |
| Qt ADS | 십자 아이콘(40 px 또는 글꼴 3 배) | 1/3 · 1/2 | 있음 | 표적 밖에 놓으면 뜬다(미확인) | 크기 = 가능성 높음 |
| Visual Studio | 마름모 안내의 화살표만 | 화살표 위에서만 음영 | 있음 | Ctrl 끌기 · 표적 밖 | 가능성 높음(MS Learn) |

- **「붙는 영역 < 미리보기」는 흔한 관행이다** — 사용자 요구와 맞는다. 붙는 영역을 좁게(10–20% 또는 고정 px), 미리보기는 칸 절반.
- **「칸 가운데 = 새 창」 선례는 못 찾았다** — 다들 가운데 = 그 칸에 합치기다. 지금 우리 규칙(`hit.rs` `choose_band` 가 어느 띠에도 없으면 새 창 · 바깥 띠 32 px · 띠 사각형 = 미리보기 사각형 `landing_rect`)이 「새 창으로 빼려면 가운데까지」 불편의 원인이다.
- **A6 모델이면 이 문제는 자연히 풀린다** — 떼는 순간 진짜 창이 이미 있으므로, 도킹 띠 위에서 놓으면 그 칸에 붙이고(띄운 창은 닫는다) 그 밖 어디서 놓든 창이 그대로 남는다. 「새 창」 표적이 따로 필요 없다.


### D. 끌던 진짜 창이 밑 창의 안내를 가리는 문제 (ADR-0295 미결 1 · 2026-10-18 추가 수집)

수집자 1(소스 얕은 클론 — Qt ADS `4f4f602` · AvalonDock `10d1cc6` · DockPanelSuite `577d5b8` · KDDockWidgets `88adab0` · IntelliJ master) · 메인 grounding 안 함 · 적대 리뷰 안 거침(추가 수집).

- **진짜 창을 옮기는 도킹 시스템은 모두 안내를 따로 된 최상위 창에 그린다** — 테두리 없음 · 배경 투명 · 활성화 안 함 · 끌던 창보다 위. AvalonDock 이 가장 분명하다: 대상이 바뀔 때마다 z 순서를 「대상 창 → 끌던 창 → 안내 창」으로 다시 세운다(`DragService.cs:93-110` · `239-246` · 안내 창 `OverlayWindow.cs:65-68` — `AllowsTransparency` · `WindowStyle=None` · `ShowActivated=false`). Qt ADS · KDDockWidgets = `Qt::Tool | Frameless` + `WA_TranslucentBackground`(ADS `DockOverlay.cpp:442-448` · KDDW `ClassicIndicatorsWindow.cpp:159-185`). — 가능성 높음.
- **끌던 창을 반투명하게 하는 것은 드물고 선택 사항이다** — KDDW `setDraggedWindowOpacity`(기본 1.0 = 불투명) · ADS · AvalonDock 은 바꾸지 않는다. 그림을 옮기는 쪽(IntelliJ · DockPanelSuite 윤곽 0.5)만 그림을 반투명하게. — 가능성 높음.
- **판정은 늘 커서 화면 점 · 끌던 창은 뺀다** — 자기 컨테이너 목록 기하(ADS `FloatingDockContainer.cpp:566-588` · AvalonDock `DockingManager.cs:2539-2560`) · 끌던 HWND 아래로 z 순서 걷기(KDDW `DragController.cpp:953-985`) · `HTTRANSPARENT` 로 안내 창을 `WindowFromPoint` 에서 숨김(DockPanelSuite `DragForm.cs:8-60` — `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` · `TopMost`). 끌던 창 자리 = 커서 − 누를 때 잡은 오프셋. — 가능성 높음.
- **함의:** 우리 판정은 이미 셸이 화면 점으로 한다(`hit.rs` · `regions.rs` — 끌던 창만 빼면 된다). 안내 · 미리보기는 **투명 · 클릭 통과 · 비활성 최상위 창 하나**에 그리는 것이 피어 관행이다. 그 창을 Tauri/WebView2 로 만들 수 있는지(투명 · `ignore_cursor_events` · 항상 위)는 스파이크 S7 이 잰다.
- **공백:** Visual Studio 내부(소스 없음 — 문서상 안내 마름모 · 음영만) · Godot · GoldenLayout 미도달 · ADS 의 Windows z 순서는 읽기만.

### E. TRD 로 넘긴 Chrome 규칙 — Chromium 소스 읽기 (2026-10-18 추가 수집 · PRD 4.8 R12 · R15 · R7 · R24 · R26 · R13b)

수집자 1(앞선 부분 수집 1 을 이어 끝냄) · Chromium main 2026-10-18 로컬 사본(`tab_drag_controller.cc` = tdc · `hwnd_message_handler.cc` = hmh · `desktop_window_tree_host_win.cc` = dwth · `tab_container_impl.cc` = tci · `chrome/browser/ui/tabs/tab_strip_model.cc` = tsm · `ui/views/mouse_watcher*.cc`) · 메인 grounding 안 함 · 적대 리뷰 안 거침.

| # | 질문 | 결론 | 출처 | 확신도 |
|---|---|---|---|---|
| E1 | 끄는 중 키 | `EventTracker` = 앱 전체 `EventMonitor` 의 `ui::EventObserver`(`kKeyPressed` · `kMouseReleased`) — Esc = 되돌리기 · 그 밖 눌린 키 = 그 자리 놓기. 관찰자라 키를 먹지 못한다(`OnEvent(const ui::Event&)`). 단 문턱을 넘으면 `SaveFocus` 가 출처 위젯의 포커스를 지워, 끝낸 키는 **어느 뷰 · 페이지에도 닿지 않을 가능성이 높다**(문턱 전이면 포커스된 뷰로 간다). 수정 키를 거르는 코드 없음 → Ctrl · Shift · Alt 눌림도 놓기. 이동 루프의 키보드 훅은 Esc 때 DWM 전환을 끄고(새 창이면) 숨기기만 하고 늘 `CallNextHookEx` — 아무 키도 먹지 않는다 | tdc #303-341 · #571-576 · #807-810 · #1018-1024 · hmh #201-213 | 가능성 높음(포커스 경로 · Win32 이동 루프가 키를 먼저 먹는지 미검) |
| E2 | 떼고 나서 출처 창이 보일 탭 | **누르기 전에 비활성이던 탭을 끌면 누르기 전의 활성 탭이 다시 활성**(`initial_selection_model_` 를 떼기 전에 선택에 더함). 이미 활성인 탭을 끌면 `DetermineNewSelectedIndex` 순서(그 탭이 연 탭 → 같은 opener → opener → 같은 그룹 · split → 다음 펼친 탭 → 오른쪽 이웃, 마지막이면 왼쪽). 되돌리기는 처음 선택을 복원 | tab.cc #745-775 · tab_strip.cc #319-321 · tdc #1732-1745 · #1756-1763 · tsm #5059-5064 · #5945-5975 · #6033-6039 | 확실(a) · 가능성 높음(b — 떼기 경로가 그 함수를 지나는 것은 grep 연결) |
| E3 | 떼어 낸 창 크기 · 자리 | 크기 = 출처 창의 **복원(비최대화) 크기**(Init 에서 한 번 잡음) · 작업 영역을 넘는 쪽은 작업 영역 − 2×10 DIP · 붙어 있던 창이 최대화면 각 변 ≥ 최대화 크기의 절반 · **다시 떼어도 처음 크기** · 새 창은 최대화로 만들지 않는다 · 자리 = 커서 − 끌기 오프셋(탭 안 상대 위치 유지) · 탭 전부를 끌면 새 창 없이 출처를 복원하고 이동 루프 | tdc #609-610 · #2760-2788 · #130 · #2913-2934 · #1892-1898 · #1789-1814 · #2749-2757 | 확실 |
| E4 | 넘친 탭바에 붙기 | 스크롤로 끄는 탭을 보이게 하는 장치 없음 — 끄는 점을 `[0, 끌기 영역 폭 − 끄는 탭 폭]` 으로 자르고 「그냥 넘친다」(주석 · crbug 40791711) · 끄는 탭은 오른쪽 끝이 탭바 오른쪽을 넘지 않으면 보임(`ShouldTabBeVisible` — 끝을 넘으면 숨김이 먼저) | dragging_tabs_session.cc #336-344 · tci #1677-1695 · tab_strip.cc #530-543 | 가능성 높음(따로 된 자동 스크롤 미발견 ≠ 없음) |
| E5 | 닫기 모드 경계 | 기준 = 탭바 뷰 사각형을 아래 40 · 끝(새 탭) 쪽 60 만 넓힌 것(위 · 시작 쪽 0) · 반열린 `[x, x+w)` 포함 판정 · 밖에서 누름 = 즉시 해제 · 밖으로 움직임 · 창 밖으로 나감 = **300 ms 뒤** 해제(그 안에 돌아오면 취소) · 마우스엔 시간만으로 끝나는 길 없음(터치 2 s) | tci #1570-1590 · `mouse_watcher_view_host.cc` · `mouse_watcher.cc` #26 · #34-35 · #67-84 · tci #1118-1119 | 확실 |
| E6 | 750 ms 앞으로 올리기 | 창 이동 중 `Drag()` 마다 다시 시작하는 750 ms 타이머 · 커서 밑 **자기 프로세스 창 중 탭을 받을 수 있는 것**만 · `StackAtTop`(`SWP_NOACTIVATE` — 활성화 안 함) 뒤 끄는 창을 다시 맨 위로 | tdc #1133-1136 · #2666-2731 · hmh #763-768 | 확실 |
| E7 | 이동 루프가 뗌 없이 끝나면(밖의 `WM_CANCELMODE` · 보안 데스크톱 · Esc) | 훅이 `WM_LBUTTONUP` 을 못 봤으면 `kCanceled` → 되돌리기(훅 설치 실패면 놓기로 침) · Chrome 자신이 다른 탭바로 옮기려 끝낸 루프는 상태를 먼저 바꿔 이 길을 타지 않는다 | hmh #942-956 · #190-197 · #156-160 · dwth #677-678 · tdc #2041-2045 · #915-917 · #2113-2114 · #2026-2040 | 확실 |
| E8 | 캡처 잃음 | 탭바 안 끌기 = 취소 → 되돌리기 · 창 이동 중 · 시스템 끌기 중 = 무시(결과는 E7 이 정함) | tab_strip.cc #204 · tab.cc #835-836 · tdc #885-898 · #1981-1990 | 확실 |
| E9 | 출처 탭바 위로 다시 오면 | 특별 처리 없음 — 다른 탭바와 같은 붙기 경로 · 새 창이거나 탭 전부면 루프를 끝내기 전에 숨긴다 · 자리는 커서로 | tdc #1102-1106 · #1185-1194 · #2026-2037 · #1521-1524 | 확실 |

- **PRD 에 주는 뜻:** E1 → PRD Q32(끝낸 키가 Chrome 에선 페이지로 가는가)는 「가지 않을 가능성이 높다」 — 우리 규칙(어느 슬롯에도 안 감)과 같은 쪽이라 확인 대기를 닫을 수 있다(Win32 이동 루프 몫은 스파이크에서 본다). E2 → PRD AC41 의 「같은 창 3 개 이상에서 다른 탭 옆 도킹 불가」 한계는 **대부분 없어진다** — 「작업」을 보던 중 「검토」를 눌러 끌어 떼면 메인은 다시 「작업」을 보인다. E3 → PRD R7 의 「첫 떼기 크기 고정」과 맞는다. E5 → PRD R26 에 300 ms 지연 · 누름 즉시를 더한다. E7 · E8 → PRD R13b 그대로.
## 설계 함의 (선택지 재료 — 결정 아님)

| | 방식 | 맞추는 사용자 항목 | 비용 · 위험 | 기존 코드 |
|---|---|---|---|---|
| **P (스파이크 통과 조건부 추천)** | **Chrome 그대로** — 탭바 안 = 웹 포인터 캡처 · 진짜 탭 이동(TS 만) · 떼기(±15 px) = 셸이 진짜 새 창을 커서 밑에 만들고 OS 이동 루프로 따라가게(`platform` 함수 — 루프 시작 · 끝내기 · 훅) · 그 동안 셸이 커서 밑 창의 탭바 · 도킹 띠를 판정(지금 `hit.rs` · `regions.rs` · 크롬 보고 재사용) · 탭바에 닿으면 즉시 붙이고 셸이 끌기를 이어 몰며 대상 웹뷰엔 위치만 밀어 줌 · 도킹 띠 위 놓기 = 그 칸에 붙임 · 그 밖 = 창 남음 · Esc = 원래 창 · 자리로 · 탭 하나 창 = 창 자체 이동 루프 · 제목 표시줄 없앤 뒤 = 빈 탭바 `app-region: drag` | 전부(1–9) | 큼 · 스파이크 먼저(아래 S1–S6) · `platform` 새 목적 함수(ADR-0266 절차) · macOS 는 따로(`performWindowDragWithEvent` 미조사) | 기하 · 적용 · 크롬 보고 · 탭 폭 = 유지 · 셸 끌기 장부 · 웹 끌기 사건 처리 = 새로(코드 점검 문서) |
| ~~P2~~ (리뷰 6 · **기각** — Chrome 과 다르다 · 사용자 2026-10-18) | P 와 같되 떼어진 뒤 창을 **OS 이동 루프 없이** 셸이 끌기를 쥐고(숨은 셸 창 캡처 또는 `WH_MOUSE_LL` · 키보드 훅) 테두리 없는 진짜 창을 `SetWindowPos` 로 옮긴다 | 전부(1–9) | P 와 비슷 · 이동 루프의 post/send 차이 · 루프 중 사건 문제를 피한다 · 대신 Aero Snap 을 끌기 도중엔 못 받는다(놓은 뒤는 보통 창) · 훅은 `platform` 함수 | P 와 같다 |
| Q | 탭바 안만 Chrome 식(포인터 · 진짜 탭) · 떼면 지금처럼 OS 끌기 | 1 · 2 · 4 일부 | 중간 · 웹 쪽만으로 제스처 도중 OS 끌기를 시작할 길이 없다(앞선 조사 결론 2) → 성립 어려움 | — |
| R | 지금 HTML5 유지 + 고칠 수 있는 것만(누름 전환 · 닫기 모드 · 활성 탭 모양 · Esc 되돌리기 · 붙는 영역 좁힘) | 3 · 6 · 7 · 8 · 9 | 작음 | 거의 그대로 — 그림 · 점선 · 게이지 · 탭 하나 창은 남는다 |

- **방식과 무관한 것(어느 쪽이든 한다):** A4 누름 전환 · A10 닫기 모드 · A11 활성 탭 모양 · A8 Esc 되돌리기 · C 붙는 영역 < 미리보기 · 파일 드롭 막기(AC22 · 늘 필요).
- **P 의 스파이크(TRD 전에 실측):** S1 캡처를 쥔 채 같은 프로세스의 다른 WebView2 창 · 바탕화면 위로 — `pointermove` 빈도 · 좌표 · 거기서 뗀 `pointerup` 이 오나 · S2 캡처 중 출처 창 `hide()` · 날 `ShowWindow(SW_HIDE)` · 화면 밖 이동 · 알파 0 — 각각 `lostpointercapture` 가 나나 · S3 끌기 도중 새 창을 만들어 이동 루프 — 버튼 누른 채 시작되나 · 따라오나 · Snap · 루프 중 이동 사건 · IPC 지연 · `WM_CANCELMODE` 로 끝내기 · S4 셸이 쥐는 끌기(숨은 셸 HWND 캡처 또는 `WH_MOUSE_LL` · 키보드 훅으로 Esc) — 대상 웹뷰로 위치 밀기 지연 · S5 누름을 안 본 웹뷰의 `setPointerCapture` · S6 `app-region: drag` 위 호버 · 놓기가 웹에 닿나.

## grounding (메인 · Chromium 원본 대조)

`?format=TEXT` 로 받은 원본에서 직접 확인: `tab_drag_controller.cc` #401 `kVerticalDetachMagnetism = 15` · #1068 `kMinimumDragDistance = 10` · #819 `is_dragging_all_tabs` · #323 `VKEY_ESCAPE` · #1135 `bring_to_front_timer_.Start` · #1198 `browser_widget->Hide()` · `hwnd_message_handler.cc` #951 `SendMessage(hwnd(), WM_SYSCOMMAND, SC_MOVE | 0x0002` · `tab_container_impl.cc` #47 `kDragHandleExtension = 6` · #1551 `kTouchResizeLayoutTime = base::Seconds(2)` · #1576 `kTabStripAnimationVSlop = 40` · #1580 `kTabStripAnimationHSlop = 60` · `tab.cc` #768 `controller_->SelectTab(this, event)` — 전부 지지. B 표 · C 표의 웹 출처(WebView2Feedback · tao · wry · VS Code · dockview 등)는 메인이 대조하지 않았다 — 리뷰 몫.

## 적대 리뷰 결과

codex(GPT · effort high · web_search · level 4) — **판정 BLOCK**: 「P 를 확정된 구현 경로로 쓰는 것」에 대해서다. 데스크톱 Chrome 모델(A 표)은 지지했고(진짜 창 떼기 · 놓기 전 다시 붙기 · 되돌리기 · Windows 이동 루프), 막힌 것은 **이 스택에서 WebView2 → 진짜 창 → 대상 웹뷰로 넘기는 길이 증명되지 않았다**는 점이다 — 보고서가 이미 스파이크를 먼저로 둔 것과 같은 결론이라, 이 문서는 P 를 「스파이크 통과 조건부 추천」으로 낮춘다.

| # | 지적 | 반영 |
|---|---|---|
| 1 | B1 — #5701 은 창 크기 바꾸기 제스처만 보였고 다른 창으로 넘어가 거기서 떼는 흐름은 아니다 · #1321 은 WinUI3 라 Win32 를 증명 못 함(의심 · 높음) | B1 확신도 → **불확실**(S1 이 닫는다) |
| 2 | B3 — wry 의 `SetIsVisible` 은 웹뷰 초기화 때 부르고 Tauri 는 **창** 보이기를 따로 다룬다 · MS 문서의 `IsVisible` 은 그리기 · 보임이지 캡처 해제를 말하지 않는다 — 인과 화살표 둘 다 근거 없음(의심 · 높음) | B3 확신도 → **불확실** · 「캡처가 죽는다」를 「죽을 수 있다 — 미검」으로(S2 가 닫는다) |
| 3 | B4 — `SetCapture` 가 호출 스레드의 창만 받는다는 좁은 반론은 맞다(MS Learn) · 그러나 대상 웹뷰의 포인터 처리나 셸 소유 캡처가 불가능하다는 데까지는 못 간다(의심 · 중간) | 「그래서 셸이 주인」을 결론이 아니라 **가장 그럴 법한 갈래**로 낮춤 |
| 4 | tao 는 `WM_NCLBUTTONDOWN` 을 **post** 하고 Chromium 은 `WM_SYSCOMMAND SC_MOVE\|0x0002` 를 **send** 해 중첩 루프를 쥔다 — 루프 동안 경계 사건 · 취소 · 루프 끝 뒤 다시 붙기를 Tauri 경로에서 보인 적 없음(높음) | B5 에 차이 명시 · S3 에 「루프 동안 경계 사건이 오나 · `WM_CANCELMODE` 뒤 다시 붙기」 추가 · 이동 루프는 `platform` 함수로 Chrome 처럼 직접 send 하는 안을 함께 잰다 |
| 5 | 포커스 · 입력 영향 과소평가 — tauri#10767(포커스 잃음 · 뗌 유실) · Esc · IME 조합 · 단축키 · Snap · 혼합 DPI 를 스파이크에(의심) | S3 · S4 에 추가 |
| 6 | Q · R 이 더 싼 대안을 다 덮지 않는다 — **혼합안**(탭바 안 = 웹 포인터 · 떼면 셸이 끌기를 쥐고 테두리 없는 진짜 창을 `SetWindowPos` 로 옮김 · OS 이동 루프 없음)을 비용 · 동작으로 견줄 것(중간) | 아래 선택지에 **P2** 추가 |
| 7 | 새 제스처에 LLM 호출 경로가 없다(CLAUDE.md 「LLM-우선 제어」) — 창 사이 이동 · 순서 · 분할 · 팝아웃 · 취소의 명령(중간) | 이미 T-65(`tab.dock` · `layout.get` — 슬롯 도킹 LLM 명령 · 의식적 예외로 등록)가 그 자리다 — P 의 TRD 가 T-65 와 같은 적용 경로(`layout::apply`)를 쓰도록 적는다 |
| 8 | 도킹 관행 — VS Code 10%(편집기) · 10/30%(그룹) · dockview 20% 는 확인되나 「보편 미리보기 비율」까지는 아니다 · Lumino · FlexLayout 수치는 인용 전 재확인(낮음) | C 표 문구 유지(이미 요약 읽기 · 재확인 표시) |

## 공백 · 한계

- macOS(이동 루프 · `performWindowDragWithEvent`) · 혼합 DPI · 다중 모니터 미조사.
- `CalculateDraggedWindowSize` · `GetTabOffsetForDetachedWindow`(새 창 크기 · 자리) · `CompleteDrag` 세부 미독.
- 도킹 영역 쪽 JetBrains · Eclipse · Zed · rc-dock · Golden Layout 미도달 · VS Code `0.1/0.3` · Lumino `DEFAULT_EDGES` 는 요약 읽기라 ADR 인용 전 원본 재확인.
- B3 · B4 · B5 의 핵심은 소스 읽기 · 추론이다 — P 를 고르면 스파이크가 먼저다.
