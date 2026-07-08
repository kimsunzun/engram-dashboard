# ADR-0054: 런타임 WebviewWindow는 config 창과 동일한 WebView2 additionalBrowserArgs를 써야 한다 (환경 옵션 parity 불변식)

- 상태: 확정 (2026-07-08, 근거: GUI 실측 — 스파이크로 단일 변수 반증/실증, EnumWindows 확인)
- 관련: CLAUDE.md §아키텍처 원칙 §5 · `src-tauri/src/commands/popout.rs:35-43,155-163`(상수 정의 + 적용) · `src-tauri/tauri.conf.json:20,30`(config 창 args) · ADR-0038(비자명 결함 = OSS/교차조사 우선) · ADR-0046(라우팅) · ADR-0035(레이아웃 권위) · step-log 2026-07-08

## 맥락
슬롯 팝업 분리(`pop_out_slot`)는 런타임에 `WebviewWindowBuilder::build()` 로 새 OS 창을 만든다. 그런데 Windows(WebView2)에서 **런타임 생성 창이 "유령"** 이었다: `build()` 는 `Ok`, Tauri 레지스트리(`getAllWindows()`)엔 등록되는데 **실제 OS 창(HWND)이 생기지 않았다**(Win32 `EnumWindows` — 숨김 포함 열거 — 로 확정: 창 수가 안 늘어남). `tauri.conf.json` 에 선언한 config 창(`main`·`agent-tree`)은 정상 표시되고, **런타임 생성 창만** 유령이었다. 순수 `about:blank` 창(팝업/뷰/라우팅 전부 배제한 스파이크)도 동일하게 유령이라, 원인은 팝업 로직·URL·dev 서버가 아니라 **런타임 창 생성 자체**로 좁혀졌다.

## 결정
**런타임에 WebView 창을 만드는 모든 코드는 config 창과 문자-단위로 동일한 `additionalBrowserArgs` 를 줘야 한다.** 그 값을 SSOT 상수 `WEBVIEW2_BROWSER_ARGS`(`popout.rs`)로 두고 런타임 빌더가 `.additional_browser_args(WEBVIEW2_BROWSER_ARGS)` 로 참조한다. 값 = `--disable-features=msWebOOUI,msPdfOOUI --autoplay-policy=no-user-gesture-required`(현재 `tauri.conf.json` 의 `additionalBrowserArgs` 두 항목과 동일). config JSON 은 strict 파싱(`deny_unknown_fields`)이라 동기 주석을 못 달아 상수 쪽 주석에 "tauri.conf.json 과 반드시 동기" 를 박는다.

## 거부한 대안
- **async command 를 `run_on_main_thread` 로 감싸 build 하기 (스레드/STA 가설)** — "async command 는 tokio 워커 스레드라 WebView2 STA UI 스레드 요건을 못 맞춰 유령" 가설. **실측으로 반증**: build 를 `run_on_main_thread` 로 메인 이벤트 루프 스레드에서 실행(mpsc 로 완료 대기 확인)해도 유령이 그대로였다. 스레드 문제가 아니다.
- **wry 의 더 넓은 기본 args 채택(추가로 `msSmartScreenProtection` 비활성)** — 언뜻 "표준값" 같지만, 그러면 config 창과 **새 불일치**가 생겨 같은 버그가 재발한다. 불변식은 "표준값" 이 아니라 "같은 user-data 폴더 안에서 서로 동일" 이다 → config 값에 정확히 맞춘다.
- **단일 소스 생성(build.rs 로 config↔상수 자동 동기 또는 build-time assert)** — drift 를 원천 차단하는 이상적 안이지만 이번 수정 범위 밖. JSON→Rust 방향 silent drift 리스크는 **후속 하드닝**(상수 vs 파싱된 config 대조 build-time assert)으로 남긴다(리뷰 지적 반영).

## 근거
- WebView2(COM/STA)는 같은 user-data 폴더를 공유하는 WebView 들이 **동일한 `CoreWebView2EnvironmentOptions`**(= `additionalBrowserArgs`)로 환경을 만들어야 한다. config 창은 args 를 주고 런타임 창은 안 줘서 옵션이 어긋나면, 같은 폴더에서의 런타임 WebView2 환경 생성이 **조용히 실패**(build 은 Ok 를 반환하고 tao 레지스트리엔 등록되나 HWND 는 안 생김 = 유령).
- **실측(단일 변수)**: 원래 유령 상태에서 `.additional_browser_args(config 문자열)` **하나만** 추가(다른 조건 원복)하니 `EnumWindows` 창 수 7→10, `HELLO`/`slot-popup-1` 창이 실제 표시. 실제 `pop_out_slot` 으로도 `Engram — slot-popup-1` OS 창(visible, 유효 HWND) 확인.
- 근본원인 도달 = ADR-0038(비자명 결함은 솔로 추측 금지·OSS/교차조사 우선). 스레드 가설이 반증된 뒤 cross-family(Codex) 교차 확인이 config↔런타임 args 불일치를 지목 → repo config 로 grounding → 스파이크로 실증.

## 영향 / 불변식
- **불변식(parity):** 같은 user-data 폴더의 **모든** WebView 창(config·런타임 불문)은 **동일한** `additionalBrowserArgs` 를 쓴다. 어기면 런타임 창이 유령이 된다(HWND 미생성). 새 런타임 창 생성부는 반드시 `WEBVIEW2_BROWSER_ARGS` 를 준다.
- **동기 지점(3중 중복):** `tauri.conf.json`(main·agent-tree 2곳) + `WEBVIEW2_BROWSER_ARGS`(Rust 1곳). 어느 한 쪽을 바꾸면 나머지도 같이 바꾼다. 현재는 상수 주석으로만 강제(one-way) — **후속: build-time assert 로 양방향 강제**(위 거부 대안 3).
- **묶이는 코드:** `popout.rs`(상수 + `pop_out_slot` 적용). 향후 다른 런타임 창(예: codex·API 백엔드 창)도 이 상수를 참조한다.
