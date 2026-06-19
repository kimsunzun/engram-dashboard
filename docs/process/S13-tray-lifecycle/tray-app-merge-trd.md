# TRD — 트레이를 Tauri 앱에 통합 (ADR-0026, 2프로세스)

작성 2026-06-19 dashboard9. ADR-0026 구현 설계. 이전 `tray-topology-trd.md`(3프로세스)는 ADR-0023과 함께 폐기 — 본 문서가 대체.

## 0. 범위 / 비범위
**범위(이번):** 별도 트레이 프로세스(`engram-tray-host`) 제거 → 트레이를 Tauri 앱 안으로. 로컬 제어 평면만:
- Tauri v2 네이티브 트레이(아이콘·메뉴·상태색).
- 트레이 메뉴 → 데몬 ensure/stop(기존 discovery), 창 show/hide(프로세스 내부), 완전 종료.
- X = hide(prevent_close). autostart(`--hidden`). single-instance(hidden raise).

**비범위(다음):** 데이터 평면 flip(UI↔데몬 WS attach), 원격·다수 데몬 profile/context, updater. 에이전트는 이번에도 embedded 유지(데몬은 lifecycle 대상일 뿐 아직 에이전트 호스트 아님 — flip 전).

## 1. 목표 구조 (2프로세스)
```
로컬 앱 (engram-dashboard.exe — Tauri v2, 1 프로세스, autostart 상주)
├─ 트레이(네이티브)  ─┐
├─ main 창(WebView) ─┤  같은 프로세스 → 창 제어 = window.show()/hide() (IPC 없음)
├─ agent-tree/slot-popup 창(hidden) ─┘
├─ Rust: tray 모듈(의도→command) · commands(daemon_*) · embedded_carrier(에이전트)
└─ discovery(ensure/send_stop/status, WMI/WS) ── 로컬 제어 평면

데몬 (engram-dashboard-daemon.exe — detached, WMI spawn, 앱과 독립 생존)
  └─ 향후 에이전트 호스트(flip). 지금은 lifecycle 대상.
```

## 2. 모듈 배치 (src-tauri)
신규 `src-tauri/src/tray/` (또는 `tray.rs`):
- `mod.rs` — TrayIcon 생성/메뉴 배선/이벤트 핸들러(Tauri `tray::TrayIconBuilder`, `menu::{Menu,MenuItem,PredefinedMenuItem}`). setup()에서 1회 생성.
- `core.rs` — **engram-tray-host의 core.rs에서 살리는 순수 로직 이관**: `MenuAction`(의도 enum)·`menu_id`/`label`·`action_for_menu_id`·`IconState`·`icon_state_for`·`to_grayscale_rgba`. 단위테스트 함께 이관. (Launcher/DaemonProbe trait·dispatch는 앱에선 불필요 — 트레이 핸들러가 직접 command 호출. 단순화.)
- 아이콘 두 벌(컬러/회색)은 `include_bytes!`로 박아 앱 빌드에 포함(기존 방식 유지).

## 3. 핵심 동작 설계
- **메뉴 6→단순화:** 데몬 켜기 / 데몬 끄기 / UI 보이기 / UI 숨기기 / ── / 완전 종료. ("트레이 종료"는 통합으로 무의미 — 트레이=앱이라 트레이만 끄기 불가. "완전 종료"만.)
- **창 보이기/숨기기:** `app.get_webview_window("main")` → `show()`+`unminimize()`+`set_focus()` / `hide()`. **프로세스 내부 호출, IPC 없음.**
- **X = hide:** `on_window_event`의 main `CloseRequested`에서 `api.prevent_close()` + `window.hide()`. (현 `app.exit(0)` 교체 — ADR-0026.) 진짜 종료는 트레이 "완전 종료"의 `app.exit(0)`만.
- **데몬 켜기/끄기:** 기존 `discovery::ensure_daemon`(WMI)·`send_stop`(WS) 호출. **WMI spawn이라 데몬은 WmiPrvSE 자식 = 앱 Job 미상속 = detached/breakaway 자동충족(ADR-0024 C1 유지).** 절대 `std::process::Command`로 데몬 직접 spawn 금지.
- **blocking 회피:** ensure/send_stop은 수초 blocking(WMI 폴링/WS). 트레이 이벤트 핸들러(메인 스레드)에서 직접 호출하면 트레이·창 멈춤 → `tauri::async_runtime::spawn`(또는 spawn_blocking)으로 보내고, 완료 후 `tray.set_icon()`으로 상태색 갱신. set_icon은 메인 스레드 제약 주의(AppHandle로 메인에 post). (tray-host의 워커+회수 패턴을 Tauri async로 대체 — 더 단순.)
- **상태색:** 데몬 alive=컬러/dead=회색. 액션 직후 `discovery::daemon_status().alive`로 갱신. 외부/크래시 죽음 주기 감지는 비범위(flip 때 상시연결로 흡수).

## 4. 플러그인 (신규 의존)
- `tauri-plugin-single-instance` v2 — **가장 먼저 등록**. 콜백 `|app, argv, cwd|`: argv에 `--hide` 있으면(=다른 트리거가 숨기려는 경우) hide, 아니면 main 창 `show()+unminimize()+set_focus()`. **hidden 창 raise 알려진 함정:** set_focus만으론 Windows focus-stealing에 막혀 작업표시줄 깜빡임만 날 수 있음 → show→unminimize→set_focus 순서 필수, 부족하면 일시 `always_on_top` 토글.
- `tauri-plugin-autostart` v2 — 로그인 시작. 부팅 기동 시 `--hidden` 인자 → setup()에서 감지해 main 창 미표시(트레이만). 사용자 직접 실행은 창 표시.
- `tauri = { features=["tray-icon", "image-ico"] }` — 네이티브 트레이.

## 5. 롤백(제거) 절차
1. `core.rs` 순수 로직 → `src-tauri/src/tray/core.rs` 이관(테스트 포함).
2. `crates/engram-tray-host/` 삭제.
3. 루트 `Cargo.toml` members에서 `"crates/engram-tray-host"` 제거.
4. `cargo build`(루트) → Cargo.lock 재생성, green 확인.
5. `rg engram[-_]tray[-_]host` → docs/handoff 외 코드 참조 0 확인.

## 6. 불변식 (구현 시 지킴)
- 데몬 spawn = discovery(WMI)만. 앱 Job에 데몬 안 넣음(detached/breakaway, ADR-0024 C1).
- 트레이 액션 = Tauri command 경로(§5 — LLM이 같은 command surface 호출). 네이티브 트레이 팝업은 cdp DOM 아님 → cdp 검증은 command/상태로(직클릭 기대 금지).
- 로컬 제어(창/트레이/데몬 lifecycle)를 원격 데이터 채널로 라우팅 금지(ADR-0026 평면 분리).
- core.rs 순수성 유지(OS/Tauri 무의존 — 단위테스트 대상).

## 7. 테스트 / 검증
- **단위(이관):** MenuAction id 매핑·icon 상태·grayscale(기존 tray-host 테스트 그대로).
- **build/test:** `cargo build` green, `cargo test` green, `rg "use tauri" crates/engram-dashboard-core/src/`=0.
- **GUI 실측(cdp):** 창 show/hide invoke, X=hide 후 창 사라지고 프로세스 생존, "UI 보이기"로 복귀. 트레이 메뉴 클릭은 수동(네이티브 팝업) — 대신 트레이 command를 invoke로 직접 호출해 검증.
- **spike(ADR-0026, 구현 전/중 실측):** ① WebView2 hidden 메모리, ② hidden raise focus-stealing, ③ updater 재시작 중 데몬 reconnect(비범위지만 메모), ④ 앱 크래시 후 데몬 재발견.

## 8. 구현 실행 (CLAUDE.md 규약)
코더(opus) → reviewer-deep(또는 fable) → QA(build/test + cdp 실측) 서브에이전트 분리. 롤백 제거 + 재구축을 단계 커밋(① 롤백 제거 green, ② 트레이+X=hide, ③ single-instance+autostart).
