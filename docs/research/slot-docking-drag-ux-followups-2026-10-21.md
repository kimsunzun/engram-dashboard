# 슬롯 도킹 OS 끌기 뒤 체감 문제 넷 — 관행 · 실측 (2026-10-21)

> **상태:** 조사 끝 · 결정(사용자 2026-10-21 — 부록 끝) — ① 예열 창 = 채택(「예비창을 띄워줘」 · 처음부터 하나를 늘 예열) · ② 금지 커서 = 미결(부록 2 판단 대기 — 막히면 그대로 둔다) · ③ 가림 = 그대로(가려진 자리엔 못 놓는다) · ④ 탭 색 = ADR-0298. 계기 = U3 뒤 사용자 실측(2026-10-21): ① 바탕화면 놓기의 새 창이 늦다 ② 우리 창 밖에서 금지 커서 ③ 끄는 출처 창 · 다른 앱 창이 놓을 창을 가린다 ④ 활성 탭이 안 보인다.
> **강도:** medium — 수집자 4(Claude · 문서형) + 메인 합성 · grounding + codex 적대 리뷰 1(FIX 7 — 반영). 실측 = 마우스 버튼 상태 읽기(입력 주입 없음) · 앱 로그 · 새 창 페이지의 CDP Navigation/Resource timing(읽기만).
> **링크:** TRD `docs/process/S21-slot-docking/trd.md` §9 · ADR-0296 · ADR-0297 · 옛 예열 조사 `.claude/handoff/attachments/slot-docking-popout-prewarm-research-2026-10-19.md`(미추적).

## 결론

1. **새 창 지연(개발 빌드 ≈ 0.6 s) 의 대부분은 창 짓기 + 페이지 적재다 — OS 몫은 작다.** 떼기 → 셸 적용 로그 0.16–0.19 s(3 회)의 대부분은 **WebView2 창 · 컨트롤러 짓기** 로 보인다 — 적용 로그가 창을 다 지은 뒤에 찍힌다. 그 구간에는 입력 · 출처 기하 · 모니터 읽기와 자리 놓기도 섞여 있어 짓기만 따로 잰 값은 아니다(탐색 시작 시각 기준 ≈ 0.16 s · codex 리뷰 지적)(`layout/apply.rs:1242` `open_at` → `:1276` `record_placement` 의 게터가 메인 스레드에서 짓기 끝을 기다림). 적용 → 보임 ≈ 0.40 s = 개발 서버 모듈 적재 · 실행 ≈ 0.27 s + 마운트 · 직렬 IPC ≈ 0.10 s + 첫 화면 문 ≈ 0.03 s. 떼기 → dragend 는 ≤ ~0.03 s 로 추정(미검). **확신: 가능성 높음**(구간 경계 = 표본 1 실측 + 3 회 로그 일치 · 릴리즈 수치는 추정).
2. **줄이는 주된 길 = 숨긴 예열 창 채택**(Electron `show:false` · Chrome spare renderer · 옛 조사 2026-10-19 와 같은 결론). 옛 스파이크 S8 「pool」 = 보임 p50 35.8 ms · 깜빡임 0 — 단 이미 그려 둔 화면 밖 창을 옮겨 온 다른 흐름의 값이고, **숨긴 창의 첫 보임은 잰 적이 없다**(`.claude/handoff/attachments/spike-chrome-drag-2026-10-18.md:78-96`). 채택 뒤 실제 수치는 구현해서 잰다. 지금 새 끌기 경로는 부팅 때 지어 둔 예비 창(`slot-popup-1`)을 **쓰지 않고** 매번 새로 짓는다(`reused=false`). **확신: 확실**(로그 · 코드).
3. **금지 커서는 업계도 그대로 둔다.** Chromium 은 `GiveFeedback` 에서 `DRAGDROP_S_USEDEFAULTCURSORS` 를 돌려 커서를 OS 에 맡기고, OS 는 대상이 `DROPEFFECT_NONE` 이면 금지 커서를 띄운다. OLE 자체는 끌기 출처가 커서를 직접 줄 수 있게 허용하지만(`IDropSource::GiveFeedback` 계약), WebView2 · Chromium 이 그 손잡이를 페이지에 내주는 길은 못 찾았다 — 「불가능」이 아니라 「우리가 닿는 길 없음」이다. HTML5 끌기로 창 밖 새 창을 만드는 피어(Infragistics Dock Manager Electron 예제 · `zcbenz/dnd-drag-hint`)는 dragend 의 `dropEffect === 'none'` 을 「새 창」 신호로 쓴다(그 문서들이 사용자에게 보이는 커서를 적지는 않는다). 바탕화면이 받는 형식(파일 등)을 실으면 커서는 바뀌지만 놓으면 바탕화면에 실제 파일 · 바로가기가 생겨 쓸 수 없다(추론 — 미검). **확신: 확실**(Chromium 소스) / 피어 관행 = 가능성 높음.
4. **가림 — 끄는 동안 커서 밑 창을 올리는 것은 OS 셸 수준 관행이다**(Windows 작업 표시줄: 끄는 중 단추 위에 머물면 그 창이 「잠시 뒤」 앞으로 · KDE Plasma 5.24 「1–2 초 머물면 올림」 · macOS 스프링 로딩은 폴더). 탭 도킹 앱이 스스로 하는 사례는 못 찾았다(VS Code · JetBrains · Visual Studio — 모름). (옛 판이 여기 적은 Metacity 「뗄 때 올림 기각」은 **출처 창의 클릭 올림을 미루는 이야기**라 이 결론의 근거가 아니다 — codex 리뷰 지적 · 대상 창을 끄는 중 올리는 것은 GNOME 버그 112308 이 따로 다룬다.) 자체 끌기 피어(Chrome · KDDockWidgets · AvalonDock)는 끄는 창이 커서를 따라가고 대상 찾기에서 끄는 창을 뺀다(z 순회) — OS 끌기인 우리가 같은 길을 쓸 수 있는지는 OLE 가 `WS_EX_TRANSPARENT` 층 창을 건너뛰는지에 달렸고 그것은 미검이다(문서는 층 창 + 투명이 마우스를 아래로 넘긴다고만 한다). 반투명 · 클릭 통과 · 숨김으로 푼 사례는 0 건. **확신: 가능성 높음**(대부분 이슈 · 검색 요약 — 지연 ms 원문 미확인).
5. **활성 탭 구분 — 「활성 = 내용과 같은 면」 피어들은 탭바를 내용과 다른 밝기로 둔다.** Zed Ayu Dark: 활성 = 내용 `#0D1016` · 탭바 = 비활성 `#1F2127`(활성이 가장 어둡고 나머지가 밝다 — ΔL* ≈ 9, 추정치) · Zed One Dark 같은 방향(ΔL* ≈ 3) · VS Code Dark Modern 은 반대 방향(탭바 · 비활성 `#181818` < 활성 · 내용 `#1F1F1F`) + 활성 위 1 px 강조선 `#0078D4` · 글자 `#FFFFFF` 대 `#9D9D9D` · JetBrains Darcula 밑줄 3 px `#4A88C7`. 우리는 탭바 `#000000` · 활성 = 내용 `#0a0a0a` 로 ΔL* ≈ 2.7(셈 — 처음 적은 「≈ 2」는 메인 추정) 라 안 보인다. **확신: 확실**(테마 원본 값 — 도구 요약 경유) / ΔL* = 메인 추정.

## 1. 새 창 지연 — 구간 표 (표본 `slot-popup-9` · 개발 빌드 · vite 1420)

| 구간 | ms | 근거 | 상태 |
|---|---|---|---|
| 떼기 → dragend · IPC 보냄 | ≤ ~30 | 버튼 놓음 → 적용 164–187(3 회) 과 dragend → 적용 197 의 차 | 추정 |
| 셸 짓기 전(입력 · 출처 기하 · 모니터) | < 5 | `commands/tab_drop.rs:51-99` | 추정 |
| WebView2 창 · 컨트롤러 짓기 → Navigate | ≈ 160 | `performance.timeOrigin` − IPC 보냄 = 166 · 옛 스파이크 S8 fresh 163–222 | 실측(1) |
| 짓기 뒤 셸(자리 · dock · 게터) → 적용 로그 | 31 | 탐색 시작 → 적용 로그 | 실측 |
| HTML | 7 | Navigation timing | 실측 |
| 개발 모듈 폭포(180 요청 · 9.4 MB · 정적 import 11 단) | ≈ 185 | Resource timing — 팝업 경로가 지연 적재 아님(`App.tsx:4` · `:20`) | 실측 |
| 모듈 실행 → DOMContentLoaded | ≈ 80 | | 실측 |
| React 첫 커밋(개발 · StrictMode) | ≈ 23 | | 실측 |
| 직렬 데이터 사슬(listen 셋 → `window_role` → `list_tabs` → `get_view` → 첫 화면) | ≈ 77 | 끝점 실측 · 사슬은 코드(`dockSession.ts:151` · `WindowLayout.tsx:97` · `:142` · `:179` · `:199`) | 실측 + 코드 |
| 첫 화면 문(double rAF → `webview_painted` → show) | ≈ 33 | `firstPaint.ts:34` · 100 ms 상한은 발동 안 함 | 실측 |

- 다른 회차: 적용 → 보임 375–410 ms(한 번 476). 메뉴 「팝업으로 분리」 = 375 ms — 같은 길.
- **릴리즈 추정(미측):** 짓기 ≈ 160 그대로 · 모듈 적재 · 실행 → 100–150(번들 1.68 MB · V8 컴파일 대용 측정 43 ms · 사용자 지정 스킴의 코드 캐시 여부 미검) · 사슬 40–60 · 문 그대로 → **dragend → 보임 ≈ 330–420 ms**.

### 줄이는 길

| 안 | 이득 | 대가 |
|---|---|---|
| A 예열 창 채택(새 창 = 숨긴 예비 창에 탭을 넣고 보임 · 곧 다음 예비 창을 다시 짓는다) | 짓기 · 적재 · 마운트가 빠짐 — 수치는 가설(S8 은 다른 흐름 · 위 결론 2) | 늘 숨은 WebView2 하나의 메모리 · ADR-0296 결정 5 · TRD U4a(예비 창 걷기)와 충돌 → 결정 필요 · 0.5 s 안의 연속 끌기는 새로 짓기로 떨어짐 · SW_HIDE 로 숨긴 창의 첫 보임은 미측 |
| B 끌기가 출처 창을 떠날 때 미리 짓기 | 끌기가 0.5 s 이상이면 A 와 같음 · 쉬는 창 없음 | 우리 창에 놓으면 헛짓기 · 짓기가 메인 스레드를 ~160 ms 묶어 다른 창의 동기 명령이 끄는 동안 선다 |
| C 첫 화면 재료를 짓을 때 실어 보냄(역할 · 탭 · 스냅숏) | 30–60 ms(개발) · 15–30(릴리즈 추정) | 두 번째 데이터 길 |
| D 경로별 코드 분할 | 50–120 ms(개발) · 10–25(릴리즈 추정) | Suspense 경계 · 내용이 보인 뒤에 차오름 |
| E 첫 화면 문 rAF 하나로 | ≈ 16 ms | 아직 안 그린 프레임을 보일 위험 |

### 함께 본 것

- 적용 로그 이름이 단계를 오해하게 한다(짓기 뒤에 찍힘) — 짓기 시간 줄(`build_ms`)을 따로 남기면 이 표를 로그만으로 읽는다.
- 예비 창 · 안내 창이 아직 부팅 때 지어져 쓰이지 않은 채 남고(ADR-0296 이 걷기로 정함 — U4a), vite 전체 새로고침마다 함께 다시 적재된다.
- 팝아웃을 도로 붙여 창이 사라질 때 `PostMessage failed … 0x80070578`(wry 개발 빌드 출력 — 닫힌 창에 보냄) — 성능 무관.

## 2. 금지 커서

- Chromium `ui/base/dragdrop/drag_source_win.cc` — `GiveFeedback` 이 `DRAGDROP_S_USEDEFAULTCURSORS` 를 돌려준다(확실 · 원문). MS `IDropSource::GiveFeedback` 문서 — 대상을 벗어나면 `DROPEFFECT_NONE` 으로 불린다(확실).
- VS Code #10121(창 밖 끌기에 금지 커서 · 탐색기에 놓으면 파일 복사) → 1.85(2023-11)가 바탕화면 놓기로 떠 있는 창을 만들었다. 그 뒤 커서 모양은 모름.
- Firefox 1598915 — 다른 앱(VS Code · Discord 등)이 `DROPEFFECT_COPY` 로 받기만 하고 아무것도 안 해 탭이 새 창이 안 되던 것을 132 에서 「자기 탭 형식은 바깥 앱의 받음 응답을 무시」로 고쳤다(130 의 수정은 조건이 뒤집혀 130 · 131 은 영향받음). 우리 Q39(다른 앱이 받아도 새 창)와 같은 방향(확실 · 버그 트래커 요약).

## 3. 가림

| 피어 | 끄는 동안 | 가린 대상에 닿는 법 | 확신 |
|---|---|---|---|
| Windows 작업 표시줄 | 끄는 중 단추 위에 머물면 그 창이 앞으로(Windows 11 22557 공지는 **파일** 끌기 기준) | 단추 경유 — 우리 탭 끌기에서도 되는지는 미검 | 동작 확실 · 지연 ms 모름(400 ms 는 일반 hover 값 — 미확인) |
| KDE Plasma 5.24 | 1–2 초 머물면 올림(버그 176582 FIXED) | 머물기 | 가능성 높음 |
| GNOME / Metacity | 끄는 중 대상 올림 없음 · 대상 올림 논의 = 버그 112308(미결) | — | 가능성 높음 |
| Chromium | 끄는 창이 커서를 따라감 · 대상 찾기에서 끄는 창 제외(`GetLocalProcessWindowAtPoint(…, ignore)`) | 자체 z 판정 | 가능성 높음(Windows 구현 미독) |
| KDDockWidgets · AvalonDock | 떠 있는 창이 커서를 따라감 · 끄는 창 빼고 z 순회 · 안내 창은 마우스 통과 | 자체 z 판정 | 확실(소스) |
| ChromeOS | `tab_drag_drop_windows_hider` 가 있다(이름으로 미루어 다른 창을 숨김) | — | 불확실(미독) |
| VS Code · JetBrains · Visual Studio | 근거 없음 | — | 모름 |

- OLE 의 대상 창 고르기가 `WS_EX_TRANSPARENT` · 알파 0 · 숨긴 창을 건너뛰는지 — MS 문서에 없다(모름). Wine 은 `WindowFromPoint` 대신 `ChildWindowFromPointEx` 로 따로 짰다(검색 요약 — 미독).
- 바깥 스크립트로 끄는 동안 출처 팝아웃을 반투명 + 통과로 바꾸는 스파이크를 했으나 결과 판정 전에 멈췄다(사용자 관찰: 제목 표시줄 끌기에도 반투명이 걸림 — 스크립트가 팝아웃 안의 모든 끌기에 반응). **결과 없음.**

## 4. 활성 탭

| 피어 | 탭바 | 활성 | 비활성 | 내용 | 표시 | 확신 |
|---|---|---|---|---|---|---|
| VS Code Dark Modern | `#181818` | `#1F1F1F` | `#181818` | `#1F1F1F` | 위 1 px `#0078D4` · 글자 `#FFF`/`#9D9D9D` | 확실 |
| Zed Ayu Dark | `#1F2127` | `#0D1016` | `#1F2127` | `#0D1016` | — | 확실(도구 요약) |
| Zed One Dark | `#2F343E` | `#282C33` | `#2F343E` | `#282C33` | — (`#464B57` 은 일반 테두리 값) | 확실(도구 요약) |
| JetBrains Darcula | `#3C3F41` | `#4E5254` | `#3C3F41` | — | 밑줄 3 px `#4A88C7`(편집기 탭 적용은 추론) | 가능성 높음 |
| Firefox | — | — | — | — | 그림자 · 고대비에서 윤곽 | 표시 확실 · 값 모름 |
| Chrome · Windows Terminal · JetBrains New UI | 모름 | | | | | |

- 활성 · 비활성 면의 밝기 차 지침(ΔL · 대비비)은 못 찾았다. WCAG 1.4.11(비텍스트 3:1)은 기억 — 미확인.

## 한계

- 수집 도구가 일부 원문(Chromium `window_finder` Windows 구현 · Raymond Chen 원문 · Windows Terminal 스펙 · WinUI 리소스)을 못 읽었다 — 해당 행은 커밋 메시지 · 검색 요약 근거다.
- 지연 구간은 표본 1 의 경계 실측 + 3 회 로그다. 릴리즈 수치는 전부 추정이다.
- 반투명 · 통과 스파이크는 판정 전 중단 — 결론 없음.

## 부록 — 가려진 대상 2차 조사(소스 수준 · 2026-10-21) · 결정

사용자 질문 「A 창 왼쪽에 붙이고 싶은데 다른 창이 가리면?」 · 「다 가이드 방식이야?」 에 답하려고 수집자 둘이 소스를 직접 읽었다(Claude 문서형 · 이 부록은 codex 리뷰를 거치지 않았다).

| 대상 | 가려진 대상 처리 | 근거 |
|---|---|---|
| KDDockWidgets | **맨 위 가이드** — 가려졌을 법하면(`DockRegistry::isProbablyObscured`) 바깥 가이드를 다시 띄운다. 주석: 「there might be another window obscuring our target, so it's useful to show the outer indicators」(`src/core/DropIndicatorOverlay.cpp` 183–191) · Win32 z 순회(`DragController.cpp` `qtTopLevelUnderCursor`) | 소스 |
| Qt-ADS · AvalonDock · DockPanel Suite | 맨 위 가이드(별도 최상위 창) + 끄는 창을 판정에서 뺌 | 소스 |
| Dear ImGui(docking) | 끄는 viewport 를 입력 없음으로 · 가이드는 호스트 창 안 · 「non-imgui windows … in-between」은 다루지 않는다고 주석으로 인정(`imgui.cpp` 17192) | 소스 |
| Eclipse e4 | 끄는 셸에 커서 자리 구멍(Region)을 뚫어 밑을 판정 | 소스 |
| NetBeans | 가려진 창은 대상에서 뺀다(못 놓음) | 소스 |
| wxAUI · Qt QDockWidget | 기하만 본다(가림 무시) | 소스 |
| Chromium | 끄는 창을 빼고 우리 프로세스 창을 z 순으로 · **가려진 창에는 놓지 않는다** — 「tab drag drop shouldn't drop on an occluded browser window」(`ui/display/win/local_process_window_finder_win.cc`) | 소스 |
| IntelliJ | 화면 사각형만 보고 가림을 안 본다(안 보이는 창에도 놓아짐) · 빈 떠 있는 창은 끄는 동안 알파 0.5(`DockManagerImpl.kt`) | 소스 |
| Unreal Slate | 커서 장식 창(0.45 불투명) · OS 가 정한 창 안에서 판정 · 다른 앱 창 가림은 못 봄 | 소스 |
| VS Code(우리와 같은 HTML5 끌기) | 처리 없음 — 가려진 곳엔 사건이 안 가서 못 놓음 | 코드 요약 |
| 대상 창 올리기 | **0 곳** | — |

- 셈: 맨 위 가이드 4(+ImGui 부분) · 끄는 창 빼기 7(프레임워크) · 대상 올리기 0 · 가려진 곳 못 놓음 = Chromium · VS Code · NetBeans.
- **결정(사용자 2026-10-21): 그대로 둔다 — 가려진 자리엔 못 놓는다(VS Code · Chromium 과 같다).** 맨 위 가이드는 하지 않는다 · 끄는 중 창 올리기(Q36)도 하지 않는다.
- **결정(사용자 2026-10-21): 예열 창 — 앱이 뜰 때 숨긴 창 하나를 늘 유지하고, 바탕화면 놓기의 새 창은 그 창을 쓴 뒤 곧바로 다시 하나 짓는다.**
- **결정(사용자 2026-10-21): 금지 커서 — 더 조사한다**(셸 쪽 수단 · 결과 = 다음 세션).
- **결정(사용자 2026-10-21): 탭 색 — 다크 탭바 `#242424` · 활성 = 내용 `#0a0a0a`(Zed Ayu 꼴) · 구분선 1×13(Chrome 16/34 비율) · 끝 구분선 · × · + = 22 px(사용자 결정) · 아이콘 13 = Chrome 비율 · 다크 비활성 글자 `#999`(「업계 기준으로 추천하는대로」 · ADR-0298).**

## 부록 2 — 금지 커서를 바꿀 길(2026-10-21 · 수집자 1 · 리뷰 안 거침)

- **WebView2 · Chromium 설정 · 플래그 · DataTransfer 로는 못 바꾼다** — `DragSourceWin::GiveFeedback` 이 `DRAGDROP_S_USEDEFAULTCURSORS` 고정(`ui/base/dragdrop/drag_source_win.cc` · 확실). 바탕화면이 받는 형식을 실으면 커서는 바뀌지만 놓을 때 `.url` · 내려받기 파일이 생기고 dragend 의 `none` 판정도 깨진다(가능성 높음). 합성 컨트롤러의 끌기 API 는 받는 쪽 전용(가능성 높음 · wry 는 visual hosting 을 안 쓴다).
- **남는 길(전부 꼼수 · 선례 없음):**
  1. **받이 창** — 끄는 동안만 모니터마다 투명 · `WS_EX_NOACTIVATE` 최상위 창을 `RegisterDragDrop` 으로 깔고 `DROPEFFECT_MOVE` 를 답한다. 커서가 이동 모양이 된다(가능성 높음). 대가 = 우리 창들 **바로 밑 · 다른 앱 위** z 자리를 지켜야 한다(위면 우리 놓기를 가린다) · 다른 앱 위 놓기를 가로챈다(Q39 와 같은 결과라 문제 아님) · dragend 효과가 `none` 이 아니라 `move` 가 되어 「새 창」 신호를 받이 창의 Drop 으로 옮겨야 한다 · 끌기 시작을 셸이 알아야 한다(ADR-0296/0297 의 「끄는 동안 셸 호출 0」 예외).
  2. **`SetSystemCursor(OCR_NO)` 바꿔치기** — 시스템 전체 · 앱이 죽으면 되돌리기 전까지 남는다 · ole32 가 그 시스템 커서를 쓰는지조차 불확실. 권하지 않음.
  3. **OS 끌기를 버리고 자체 끌기 고리**(Chrome · Windows Terminal 꼴 — 우리 옛 엔진 · ADR-0295) — 깨끗하지만 ADR-0296 을 되돌리는 일.
- VS Code 1.85 는 macOS · Wayland 에서 Chromium 을 고쳐서 풀었다(WebView2 는 그 길이 없다). Windows 에서 VS Code 가 바탕화면 위에 어떤 커서를 보이는지는 모름.
- **판단 대기(사용자):** 받이 창(1)을 스파이크로 재 볼지 · 그대로 둘지.
