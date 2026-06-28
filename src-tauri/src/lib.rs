pub mod commands;
// S14 모듈①(ADR-0036): 데몬 WS 연결의 src-tauri측 단일 권위(DaemonClient). 창마다 N개 직결하던
// 전송을 여기로 끌어올린다 — 연결 1개. T2 = 연결 수립 + Auth/Hello 핸드셰이크 + connect/ensure 분리.
pub mod daemon_client;
// ADR-0035: 레이아웃 권위 = src-tauri(데몬 UI 불가지론). ViewManager 상태 + 순수 트리 연산 + 타입
// (ts-rs 미러). protocol/daemon crate 에 넣지 않는다 — 레이아웃은 신규 클라(src-tauri) 관심사.
pub mod layout;
// S14 모듈①(ADR-0036) T5: OutputRouter — agent_id → window-label 라우팅(lock-free arc-swap 핫패스)
// + 구독 union diff(F-B, layout 파생). 순수 로직(Tauri 의존 0, headless 테스트). T6 가 배선한다:
//   - rebuild 트리거 = layout command 의 ViewManager 락 보유 critical section 안(layout mutation 직후,
//     같은 락으로 router.rebuild(&mgr) → table+delta 산출). load→delta→store RMW 직렬화 + 현재 mgr
//     일관성(ABA 방지) — emit_after_unlock 이 아니다(락 밖 동시 호출 시 델타 어긋남, FIX-1).
//   - 델타 송신은 락 해제 후 = rebuild 반환 SubscriptionDelta 를 DaemonClient cmd_tx 로
//     Subscribe/Unsubscribe enqueue(락 안에서 송신 금지).
//   - targets 사용 = connection.rs:668 Message::Binary 자리(decode_frame → decide_output → route)
// app-level 공유(재연결 task 수명 초월) → Arc<OutputRouter> 로 manage(T6).
pub mod output_router;
// S13 sub-step 2: 순수 discovery 로직은 engram-dashboard-discovery crate 로 이동(tray-host 와 공유).
// 호출부(commands/discovery.rs)가 crate::discovery 경로를 그대로 쓰도록 re-export 만 남긴다(중복 코드 0).
pub use engram_dashboard_discovery as discovery;
// ADR-0026 2단계: 트레이를 앱에 통합(네이티브 TrayIconBuilder 배선). core=순수, actions=공유 부수효과.
mod tray;

// ADR-0029: embedded(in-process 호스팅) 제거 → daemon-only. 앱(src-tauri)은 데몬의 상주 클라이언트
// 셸이다(창/트레이/로컬 제어 command + 데몬 discovery). 에이전트는 데몬이 호스팅하고 프론트가 WS 로
// 직접 붙는다(앱 Rust 경유 안 함). 그래서 옛 in-proc 배선(AgentManager/ConnectionCore/embedded
// carrier/AppState/TauriStatusSink/모드 시스템)은 전부 제거됐다. logging::init_logging 만 코어에서 쓴다.
use engram_dashboard_core::logging;

use tauri::Manager;

// ── run() ────────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // ADR-0029: 부팅 기동(autostart 등록 인자에 --hidden 포함)은 창 없이 트레이만 상주시킨다.
    // 사용자 직접 실행(인자 없음)은 창 표시. setup 에서 이 플래그로 main 창 hide 여부를 가른다.
    // 앱은 이제 항상 트레이를 갖는 daemon 클라이언트라 모드 게이트 없이 단순 스캔만 한다.
    let hidden = std::env::args().any(|a| a == "--hidden");

    let mut builder = tauri::Builder::default();
    // single-instance 플러그인은 가장 먼저 등록(플러그인 규약). ADR-0029: 앱은 데몬 클라 전역 단일 —
    // 무조건 등록. 2nd 인스턴스 실행 → 기존 main 창 raise(show→unminimize→set_focus).
    builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        crate::tray::actions::show_main_ui(app);
    }));
    builder = builder.plugin(tauri_plugin_opener::init());

    // ADR-0029 §55: autostart. 등록 인자=--hidden(부팅 시 창 미표시·트레이 상주). 모드 인자 없음.
    // ★플러그인 등록 ≠ 활성화★: 기본 OFF, set_autostart command/트레이 토글로만 enable(레지스트리 Run 기록).
    // LaunchAgent 는 macOS 전용 인자라 Windows 무관(Windows 는 레지스트리 Run 키 사용).
    builder = builder.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        Some(vec!["--hidden"]),
    ));

    builder
        .setup(move |app| {
            // 기본 warn(OFF) — RUST_LOG 환경변수로 재정의 가능
            logging::init_logging();

            // ── ADR-0026 2단계: 네이티브 트레이 배선 ─────────────────────────────────────
            // 아이콘 두 벌 생성·메뉴·핸들러 + setup 직후 데몬 상태로 아이콘 확정. Windows 전용.
            // ADR-0029: 앱은 항상 트레이를 갖는 daemon 클라이언트라 무조건 호출(모드 게이트 없음).
            // ADR-0028: 데몬 생사 push 의 단일 소유 상태. build_tray 의 초기 refresh 가 publish 를
            // 타려면(중복차단·억제창 판정) state 가 먼저 manage 되어 있어야 한다 → build_tray 전에 등록.
            app.manage(tray::actions::LivenessState::default());

            // ADR-0035: 레이아웃 권위 상태(ViewManager). invoke 스레드풀 동시접근 → Arc<Mutex>.
            // 락 해제 후 emit(ADR-0006) 은 command 레이어가 보장. 초기엔 기본 View 1개.
            app.manage(crate::layout::LayoutState::new());
            if let Err(e) = tray::build_tray(app) {
                tracing::warn!("트레이 생성 실패(앱은 계속): {e}");
            }
            // ADR-0028: 데몬 생사 주기 옵저버 spawn(회색 고착 해소 — 외부 변화도 트레이/emit 에 반영).
            // build_tray 가 초기 아이콘을 확정한 뒤 변화만 push 한다(첫 관측은 push 안 함).
            tray::spawn_daemon_observer(&app.handle().clone());

            // ADR-0029 §55: --hidden 기동(autostart)은 main 창을 숨겨 트레이만 상주시킨다.
            // ★한계(주석 명시)★: main 창 conf 기본 visible=true 라 창이 잠깐 떴다 숨어 깜빡일 수 있다.
            // 일단 수용 — 깜빡임 제거(conf visible:false + 비-hidden 시 show)는 후속으로 이연.
            // 앱은 항상 트레이가 있어 hide 해도 트레이로 회수 가능(daemon-only, 모드 게이트 없음).
            if hidden {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            Ok(())
        })
        // ADR-0026 2단계: main X(WM_CLOSE)=hide(창만 숨기고 트레이 상주) — 진짜 종료는 트레이
        // "완전 종료"(app.exit(0))뿐. ADR-0029: 앱이 항상 트레이를 갖는 daemon 클라이언트라 모드 분기
        // 없이 무조건 prevent_close + hide.
        // ★main 만 대상★: agent-tree/slot-popup(hidden 창)은 기존대로 단독 close 처리. main 라벨만
        //  분기 — conf 첫 창은 label 미지정이라 Tauri 기본 라벨 "main".
        // 주의: CloseRequested 는 Rust 측 이벤트 관찰이라 JS capability(core:window:allow-close) 불필요.
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    // X=hide(트레이 상주). prevent_close 후 hide.
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            // Step 5: 데몬 발견(없으면 WMI spawn) — §5 LLM 제어 표면. 부팅 자동 호출은 phase4.
            commands::discover_daemon,
            // ADR-0021: 데몬 lifecycle 명시 제어 표면(§5). start=ensure(spawn 허용), stop=fallback kill,
            //   status=alive/pid/port. 재연결 루프는 이걸 안 부른다(attach-only, wsTransport).
            commands::daemon_start,
            commands::daemon_stop,
            commands::daemon_status,
            // ADR-0021: 재연결이 옮겨간(hot-swap·크래시 재spawn) 데몬을 따라가게 daemon.json 을
            //   재조회(token 포함, no-spawn). ★재연결 attach-only 의 spawn-금지 유지★(read-only).
            commands::read_daemon_info,
            // ADR-0026 2단계 §5: 트레이 동작의 LLM/cdp 제어 표면(트레이 핸들러와 같은 actions 함수).
            //   데몬 켜기/끄기는 위 daemon_start/daemon_stop 재사용 → 여기엔 창/종료만.
            commands::show_main_ui,
            commands::hide_main_ui,
            commands::quit_app,
            // ADR-0027 §53~55: 부팅 자동 시작 토글/조회 — §5 LLM 제어 표면.
            commands::set_autostart,
            commands::get_autostart,
            // ADR-0035: 레이아웃 권위(ViewManager) 상태변경 — §5 LLM 제어 표면(window.__engramLayout).
            //   락→변형→해제→emit(ADR-0006). assign_agent 는 참조 문자열만(데몬 검증 호출 0).
            commands::create_view,
            commands::close_view,
            commands::switch_view,
            commands::split_slot,
            commands::close_slot,
            commands::assign_agent,
            commands::get_view,
            // ADR-0035: read-only 조회 — 부팅 시 webview 가 active view id 를 발견하는 유일 경로
            //   (변경 핸들러는 변경 직후에만 emit → 부팅 직후엔 닿지 않음). 상태변경·emit 없음.
            commands::list_views,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        // ADR-0029: 앱은 in-proc 에이전트를 호스팅하지 않으므로 ExitRequested 에서 정리할 manager 가
        // 없다(데몬이 자기 에이전트 graceful 을 담당). RunEvent 콜백은 비어 있다.
        .run(|_handle, _event| {});
}
