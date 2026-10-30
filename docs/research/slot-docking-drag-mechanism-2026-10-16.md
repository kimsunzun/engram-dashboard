# 슬롯 도킹 — 창 사이 끌기 수단 조사 (2026-10-16)

- **상태:** 결론 = **HTML5 DnD(브라우저 내장)를 바탕으로 쓴다** — 사용자 결정 2026-10-16(「내장 기능을 최대한 · 기존 것 교체」 · 끌기 그림 = Windows 기본 모양).
- **방법:** /research medium — 수집 3 갈래(crabnebula drag 플러그인 소스 대조 · 웹 기반 피어 · Tauri 한정 실현성) + 메인 grounding + **실측 스파이크**. ★cross-family 적대 리뷰는 돌리지 않았다 — 결론을 가르는 주장(셸 메인 스레드가 막히나)을 문헌 대신 실측으로 판정해 대체했다(열화 라벨: medium — 적대 리뷰 생략)★.
- **확신도:** 확실 = 소스 · 실측으로 확인 · 가능성 높음 = 단일 출처 · 불확실 · 모름.
- 앞선 조사 = `docs/research/slot-docking-survey-2026-10-15.md` §9(그때 설계는 제목줄 끌기 — 이 조사가 탭 끌기 기준으로 다시 봤다).

## 1. crabnebula `tauri-plugin-drag-as-window` / `drag-rs` — 쓰지 않는다 (확실)

- `DoDragDrop` 을 Tauri **메인 스레드**에서 돌린다(`crates/tauri-plugin-drag-as-window/src/commands.rs` 의 `run_on_main_thread`). wry 의 드롭 대상은 위치 사건을 `proxy.send_event` 로 그 메인 루프에 실어(`tauri-runtime-wry-2.11.3/src/lib.rs:4888-4895`) — 끄는 동안 막혔다가 놓은 뒤 몰려온다(열린 이슈 #77). 실시간 가이드 불가.
- Esc = 취소 고정(`crates/drag/src/platform_impl/windows/mod.rs:72-80` `fescapepressed → DRAGDROP_S_CANCEL`) · 끄는 동안 위치 없음(끝에 `GetCursorPos` 한 번) · 실은 것이 실제 임시 파일(탐색기에 놓으면 복사) · 끌기 그림 크기 조절 없음 · 끝나지 않아 앱이 멈추는 열린 버그 #94. 메인이 소스로 위 세 줄을 대조했다.
- 버전 2.1.1(2026-05-01) · Apache-2.0 OR MIT · 121★.

## 2. 피어 — 브라우저 내장 HTML5 DnD 가 주류

| 피어 | 수단 | 근거 · 확신도 |
|---|---|---|
| VS Code(Electron) — 편집기 탭을 다른 창으로 · 새 창으로 떼기 | HTML5 DnD · 창 밖 판정 = `dragend` 에서 「어느 창도 마지막에 dragover 를 못 봤다」(`isWindowDraggedOver` · `BroadcastDataChannel` 공유) → 호스트에 커서 위치를 물어(`getCursorScreenPoint`) 새 창 | `src/vs/workbench/browser/parts/editor/editorTabsControl.ts:474-505` · `src/vs/workbench/browser/dnd.ts:700,757` — **메인이 직접 받아 확인(확실)** |
| Windows Terminal 1.18+ 탭 떼기 | OLE 끌어 놓기 | 스펙 커밋 · 가능성 높음 |
| Chromium 탭 떼기 | 창이 따라오는 네이티브 이동 루프 | 커밋 269f665 · 확실 |
| dockview(3.4k★) Tauri 데모 | HTML5 DnD · 모든 창 `disable_drag_drop_handler()` | 리눅스(WebKitGTK)에서만 실측 · 확실(데모 README · 소스) |
| tauri-explorer · block/buzz-app PR #102 · base-studio-code | 포인터 직접 추적(buzz = Rust 가 화면 좌표로 창 판정 + 항상 위 클릭 통과 고스트 창 — 이전 설계와 같은 꼴) | 셋 다 「HTML5 는 창 밖으로 못 나간다」고 했으나 각각 wry 핸들러를 켠 채 · macOS · 미측정 — 근거로 약함 |
| 언리얼 에디터 | 자체 UI 프레임워크(Slate)가 끌기 · 창 · 도킹을 직접 | 이전 설계의 모델 |

## 3. Tauri v2 + WebView2 실현성

- `dragDropEnabled=false`(설정) 또는 `disable_drag_drop_handler()`(빌더)이면 wry 가 자기 `IDropTarget` 을 달지 않아 WebView2 기본 드롭이 남는다(`wry-0.55.1/src/webview2/drag_drop.rs:45-75` · `mod.rs:150-157` · Tauri 문서 「Windows 에서 HTML5 DnD 를 쓰려면 꺼야 한다」 `tauri-2.11.3/src/webview/mod.rs:971-976`). 확실.
- ★우리 코드: main 은 이미 꺼져 있다(`src-tauri/tauri.conf.json:21`) · 런타임 팝아웃(`src-tauri/src/commands/popout.rs:150` `WebviewWindowBuilder::new`)은 끄지 않는다 — 저장소에 `disable_drag_drop_handler` 호출 0★(메인 확인). 팝아웃에서 HTML5 DnD 가 막혀 있다 → 한 줄 추가로 풀린다.
- 끄면 잃는 것 = Tauri `DragDropEvent`(우리 코드는 쓰지 않음 — grep 0) · wry 의 `AllowExternalDrop(false)` → OS 파일을 끄는 페이지에 놓으면 탐색할 수 있어 전역 `preventDefault` 가 필요할 수 있다(미검).
- Chromium 끌기 그림 상한: 요소 그림 8192px/변 · `<img>` 200×200 DIP(비 Mac) · 기본 알파 0.75 — Blink 소스(가능성 높음).

## 4. 실측 스파이크 (확실 — 측정 · 상세 = `.claude/handoff/attachments/spike-html5dnd-2026-10-16.md`)

환경: tauri 2.11.3 · WebView2 Edg/155 · RDP 세션 · DPR 1.25 · 22 끌기.

- **셸 메인 스레드는 막히지 않는다** — `DoDragDrop` 은 WebView2 브라우저 프로세스 UI 스레드(캡처 · 끌기 그림 창 소유로 확인). `run_on_main_thread` 지연 max 4.1ms(평소 1.4) · emit 도착 p99 11 · max 33ms · invoke 왕복 p50 2.2 · max ~50ms.
- 대상 창 dragover 60/s(움직일 때) · ~15/s(멈춤) · 좌표 정확(`inner_position + client×scale` = 물리 커서) · 커스텀 MIME 12/12 · dragend 모든 경우 발생(놓음 move · 창 밖 none · Esc none) · dragend 직후 셸 키 상태 읽기로 Esc 와 창 밖 놓기 구분 · 원본 창을 끄는 중 숨겨도 계속.
- 끌기 그림 = Windows 셸 모양(반투명 · 가장자리 흐려짐) · 다른 앱(드롭 대상 아님) 위에선 사라짐.
- 어느 웹뷰도 아닌 자리(틈 · 바탕화면)에선 JS 사건 없음 — 필요하면 셸이 커서를 읽는다(메인 스레드가 자유).

## 5. 결론과 영향

- HTML5 DnD 를 바탕으로 한다 — 커서 추적 · 창 판정 · 따라오는 그림 · 버튼 읽기를 엔진이 맡는다. 셸 폴링 루프 · 맨 위 창 판정 · 딱지 창 · OS 함수 대부분이 빠진다(TRD 5판).
- 이미 만든 1 · 2 · 3 · 5 단계(트리 연산 · 도킹 모델 · 닫히는 창 · 판정)는 놓기 · 적용 쪽이라 남는다. 6 단계 상태 기계는 교체 대상.
- 플랫폼: 방식은 웹 표준이라 플랫폼별 OS 코드가 줄어든다. macOS(WKWebView) 창 사이 동작은 모름 — 이식 때 따로 잰다.

## 6. 한계 · 공백

- 실제 탐색기 바탕화면 놓기 · 파일 드롭(핸들러 끔) · 혼합 DPI · 다중 모니터 · 다른 WebView2 환경 · 콘솔(비 RDP) 세션 · 표본 작음(숨김 2 · 창 밖 Esc 3) — 미검.
- Esc 와 창 밖 놓기의 구분은 dragend 직후 키 상태에 기댄다(타이밍 주의).
- 웹 기반 피어 넓은 서베이(Theia · Obsidian · 터미널 앱들)는 사용자 지시로 Tauri 한정으로 좁혀 돌리지 않았다.
