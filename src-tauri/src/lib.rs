pub mod commands;
pub mod daemon_client;
pub mod layout;
pub mod output_channel;
pub mod output_router;
pub mod ui_settings;
// ADR-0155: 웹뷰가 주인인 명령의 셸쪽 다리(등록 대리 + 2단 배달의 마지막 홉).
pub mod view_commands;
// 순수 discovery 로직은 engram-dashboard-discovery crate (tray-host 와 공유).
// 호출부(commands/discovery.rs)가 crate::discovery 경로를 그대로 쓰도록 re-export 만 남긴다.
pub use engram_dashboard_discovery as discovery;
mod tray;

// ADR-0029: embedded(in-process 호스팅) 제거 → daemon-only. 앱(src-tauri)은 데몬의 상주 클라이언트
// 셸이다(창/트레이/로컬 제어 command + 데몬 discovery). 에이전트는 데몬이 호스팅한다.
// 그래서 옛 in-proc 배선(AgentManager/ConnectionCore/embedded
// carrier/AppState/TauriStatusSink/모드 시스템)은 전부 제거됐다.
use engram_dashboard_base::logging;

use tauri::Manager;

// ── run() ────────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // ADR-0029: 부팅 기동(autostart 등록 인자에 --hidden 포함)은 창 없이 트레이만 상주시킨다.
    let hidden = std::env::args().any(|a| a == "--hidden");

    let mut builder = tauri::Builder::default();
    // single-instance 플러그인은 가장 먼저 등록(플러그인 규약). ADR-0029: 앱은 데몬 클라 전역 단일 —
    // 무조건 등록.
    // ★"전역 단일"의 범위 = 번들 identifier★: 플러그인은 Windows 뮤텍스 이름을 `{identifier}-sim` 으로
    //   만든다(tauri-plugin-single-instance 2.4.2 `platform_impl/windows.rs:67`). 그래서 identifier 를
    //   공유하는 두 빌드는 서로를 죽인다 — dev 빌드는 `src-tauri/tauri.dev.conf.json` 오버레이로
    //   identifier 를 갈라 릴리즈와 공존한다. ★두 identifier 를 통일하지 말 것★(사유 = 그 파일 주석).
    builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
        crate::tray::actions::show_main_ui(app);
    }));
    builder = builder.plugin(tauri_plugin_opener::init());
    // 네이티브 폴더 선택 다이얼로그(프리셋 경로 추가) — 프론트 PresetPalette 우클릭 "추가"가
    //   open({directory:true}) 로 호출한다. 권한은 default.json 의 dialog:allow-open 으로 최소 부여.
    builder = builder.plugin(tauri_plugin_dialog::init());

    // ★플러그인 등록 ≠ 활성화★: 기본 OFF, set_autostart command/트레이 토글로만 enable(레지스트리 Run 기록).
    // LaunchAgent 는 macOS 전용 인자라 Windows 무관(Windows 는 레지스트리 Run 키 사용).
    builder = builder.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        Some(vec!["--hidden"]),
    ));

    // ADR-0102: ★LayoutState 는 반드시 pre-build(빌더)에서 manage 한다★ — setup() 이 아니라 여기서.
    //   부팅 레이스: 웹뷰는 builder.build() *도중* JS 를 로드해 setup() 실행 전에 invoke('list_tabs',
    //   {window:"main"}) 를 쏠 수 있다. 그 상태 등록이 setup() 안에 있으면(과거 배치) command 가 미등록
    //   managed state 를 만나 Err 로 떨어지고, main 은 이벤트 복구 경로가 없어(window:tabs-updated 는 탭
    //   변형 시에만 발화) 로딩 플레이스홀더에 영구 고착된다. LayoutState::new() 는 결정적(app handle·런타임
    //   불필요 — ViewManager::new() 가 기본 View 1개를 동기 생성)이라 빌더에서 등록 가능 → 웹뷰 첫 invoke
    //   전에 상태가 반드시 존재해 레이스가 구조적으로 불가능. ★setup 으로 되돌리지 말 것★(레이스 재발).
    //   대조: DaemonClient 는 tokio 런타임이 필요해 setup 에 남는다(그쪽 조기 invoke 는 프론트 retry 가 커버).
    builder = builder.manage(crate::layout::LayoutState::new());

    builder
        .setup(move |app| {
            // 데몬과 **다른 파일**(`app-*.log`)에 쓴다 — 한 파일을 두 프로세스가 나눠 쓰면 줄이
            //   섞인다. 폴더는 데몬과 같은 `default_data_dir()` 이라 기동 실패를 쫓을 때 두 로그가
            //   한자리에 모인다.
            let data_dir = crate::discovery::default_data_dir();
            // 데몬과 같은 이유로 자기 로그 위치를 남긴다(daemon `run()` 의 "데이터 폴더 결정"): 1차
            //   폴더를 못 쓰면 이 경로가 `%TEMP%` 아래로 갈릴 수 있어, 반환값 말고는 어디에 쓰고
            //   있는지 아는 수단이 없다.
            let log_file = logging::init_logging_with_file(&data_dir, logging::LogKind::App);
            tracing::info!(
                data_dir = %data_dir.display(),
                log_file = ?log_file,
                "앱 로그 파일 결정"
            );

            // ── 죽은 창의 테마 항목 쓸기 ─────────────────────────────────────────────────
            // ★부팅에서만 돈다 — `ui.refresh` 로 옮기지 말 것★. 레이아웃은 디스크에 영속되지 않아
            //   **이 순간 팝아웃이 하나도 없고**, 그래서 지금 파일에 있는 비-선언 label 은 생사를 물을
            //   것도 없이 정의상 전부 죽은 것이다. 여기에 생존 확인을 덧대면 아직 만들어지는 중인 창의
            //   항목을 지우는 경합이 되살아난다(사유·불변식 전문 = `ui_settings::sweep_dead_windows`).
            //   로그 자리를 잡은 뒤에 부른다 — 무엇을 지웠는지가 이 앱 로그에만 남는다.
            // ADR-0167
            crate::commands::settings::sweep_dead_window_entries(app.handle());

            // ── ADR-0026 2단계: 네이티브 트레이 배선 ─────────────────────────────────────
            // ADR-0029: 앱은 항상 트레이를 갖는 daemon 클라이언트라 무조건 호출(모드 게이트 없음).
            // ADR-0028: 데몬 생사 push 의 단일 소유 상태. build_tray 의 초기 refresh 가 publish 를
            // 타려면(중복차단·억제창 판정) state 가 먼저 manage 되어 있어야 한다 → build_tray 전에 등록.
            app.manage(tray::actions::LivenessState::default());

            // ── 출력 평면(ADR-0046 — 무상태 통과): OutputRouter + window Channel registry ──
            // ★단일 공유 Arc 2벌★: router·registry — 동일 인스턴스를 본다.
            // ★미러 버퍼(buffer_store) 제거★ — remount/새 창은 데몬 ring 전량 재replay(뷰 주도, ADR-0046).
            let router = std::sync::Arc::new(crate::output_router::OutputRouter::new());
            let registry: crate::output_channel::WindowChannelRegistry = Default::default();
            app.manage(router.clone());
            app.manage(registry.clone());

            let labels = std::sync::Arc::new(crate::commands::popout::PopupCounter::default());
            app.manage(labels.clone());

            // ── 웹뷰 몫 명령의 다리(ADR-0155, TRD §6 Step 4) ─────────────────────────────
            // ★DaemonClient 보다 먼저 만든다★ — 표를 꽂을 때 함께 넘겨야 하고(등록 패킷이 두 층을 한 방에
            //   싣는다), 부팅 보고를 받는 invoke 핸들러도 같은 실물을 봐야 한다. 같은 Arc 를 양쪽에 준다.
            let view_commands = std::sync::Arc::new(crate::view_commands::ViewCommandBridge::new(
                std::sync::Arc::new(crate::view_commands::TauriViewDispatch(app.handle().clone())),
                // 설정이 숨긴 창(오늘 = agent-tree)은 마지막 수단 목적지에서 뺀다 — 사유는 그 함수 doc.
                crate::view_commands::hidden_window_labels(app.handle()),
            ));
            app.manage(view_commands.clone());

            // ── DaemonClient(데몬 WS 연결 단일 권위) 등록 ──────────
            // 전용 멀티스레드 런타임을 소유하는 클라이언트(setup 은 tokio 컨텍스트 밖이라
            // Handle::current() 대신 전용 런타임 — DaemonClient::new_real_with_owned_runtime).
            // ★app-startup connect 는 T6/connect 로 이연★ — 여기선 cmd 평면만
            // 배선하고, 실제 연결 수립(connect/ensure)은 프론트/부팅 시퀀스가 부른다.
            match crate::daemon_client::DaemonClient::new_real_with_owned_runtime(
                router.clone(),
                registry.clone(),
                app.handle().clone(),
            ) {
                Ok(client) => {
                    let client = std::sync::Arc::new(client);
                    // ── 명령 표를 그 클라이언트에 꽂는다(ADR-0155 결정 4·5) ──
                    // ★순서가 계약이다★: 표의 스폰 포트가 이 클라이언트를 쥐므로 클라이언트가 먼저 서야 하고,
                    //   연결은 아직 안 섰으므로(위 주석 — connect 는 프론트/부팅 시퀀스가 부른다) 첫 봉투보다
                    //   표가 먼저 꽂힌다. 여기서 빠뜨리면 데몬이 배달한 명령이 「표 없음」 오류로 되돌아간다.
                    // ★사람 클릭과 **같은** 상태·라우터·발급기를 넘긴다★ — 다른 인스턴스면 LLM 이 만든 창이
                    //   사람이 보는 목록에 없다(ADR-0035 레이아웃 권위는 하나).
                    match app.try_state::<crate::layout::LayoutState>() {
                        Some(state) => client.install_command_table(
                            crate::layout::commands::make_table(
                                crate::commands::layout::command_ports(
                                    app.handle().clone(),
                                    state.inner().clone(),
                                    router.clone(),
                                    labels.clone(),
                                    client.clone(),
                                ),
                            ),
                            crate::layout::commands::CATALOG_VERSION,
                            view_commands.clone(),
                        ),
                        // LayoutState 는 빌더에서 manage 되므로(위 ADR-0102) 여기 닿지 않는다 — 닿았다면 그
                        //   pre-build 등록이 사라진 것이라 조용히 넘기지 않는다.
                        None => tracing::error!(
                            "LayoutState 미등록 — 레이아웃 명령 표를 꽂지 못했다(데몬이 배달한 명령이 실패한다)"
                        ),
                    }
                    app.manage(client);
                }
                Err(e) => {
                    tracing::warn!("DaemonClient 런타임 생성 실패(데몬 명령 불가, 앱 계속): {e}")
                }
            }
            // TODO(T6/connect): 부팅 시 DaemonClient.ensure()/connect() 호출로 자동 연결 수립.
            if let Err(e) = tray::build_tray(app) {
                tracing::warn!("트레이 생성 실패(앱은 계속): {e}");
            }
            // ADR-0028: 데몬 생사 주기(회색 고착 해소 — 외부 변화도 트레이/emit 에 반영).
            // build_tray 가 초기 아이콘을 확정한 뒤 변화만 push 한다(첫 관측은 push 안 함).
            tray::spawn_daemon_observer(&app.handle().clone());

            // ★한계(주석 명시)★: main 창 conf 기본 visible=true 라 창이 잠깐 떴다 숨어 깜빡일 수 있다.
            // 일단 수용 — 깜빡임 제거(conf visible:false + 비-hidden 시 show)는 후속으로 이연.
            if hidden {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.hide();
                }
            }
            Ok(())
        })
        // ADR-0026 2단계: main X(WM_CLOSE)=hide(창만 숨기고 트레이 상주) — 진짜 종료는 트레이
        // "완전 종료"(app.exit(0))뿐.
        // tauri.conf.json 이 첫 창 label 을 "main" 으로 명시한다.
        // 주의: CloseRequested 는 Rust 측 이벤트 관찰이라 JS capability(core:window:allow-close) 불필요.
        .on_window_event(move |window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    if window.label() == "main" {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
                // ★팝업 창 Destroyed 정리(수명/누수 임계)★: 팝업이 실제로 소멸하면(정상 close 또는 프로그램
                //   destroy), main/agent-tree 는 대상 아님(main 은 위에서 hide 만 하니 애초에 Destroyed 안
                //   남, agent-tree 도 팝업 prefix 아님). 강제 프로세스 kill 은 모든 state 를 통째로 죽여
                //   이 경로가 안 타지만(수용) 정상 close·프로그램 destroy 는 여기서 확실히 정리한다.
                //   (ADR-0046: 일반 라우팅 메커니즘 정리.)
                tauri::WindowEvent::Destroyed => {
                    let label = window.label().to_string();
                    if crate::commands::popout::is_popup_label(&label) {
                        let app = window.app_handle();
                        // 하나라도 없으면(초기화 실패 극단 케이스) 조용히 스킵(정리 불가여도 앱은 계속).
                        if let (Some(state), Some(router), Some(registry), Some(client)) = (
                            app.try_state::<crate::layout::LayoutState>(),
                            app.try_state::<std::sync::Arc<crate::output_router::OutputRouter>>(),
                            app.try_state::<crate::output_channel::WindowChannelRegistry>(),
                            app.try_state::<std::sync::Arc<crate::daemon_client::DaemonClient>>(),
                        ) {
                            crate::commands::popout::cleanup_popup_window(
                                &app, &label, &state, &router, &registry, &client,
                            );
                        }
                    }
                }
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::discover_daemon,
            commands::daemon_start,
            commands::daemon_stop,
            commands::daemon_status,
            commands::read_daemon_info,
            commands::daemon_connect,
            commands::daemon_ensure,
            commands::daemon_close,
            commands::daemon_connection_state,
            commands::forward_daemon_command,
            commands::show_main_ui,
            commands::hide_main_ui,
            commands::quit_app,
            commands::set_autostart,
            commands::get_autostart,
            commands::create_tab,
            commands::create_window,
            commands::switch_tab,
            commands::close_tab,
            commands::close_window,
            commands::split_slot,
            commands::close_slot,
            commands::focus_slot,
            commands::rename_tab,
            commands::assign_agent,
            commands::set_slot_content,
            commands::spawn_into,
            commands::get_view,
            commands::list_tabs,
            commands::list_windows,
            commands::resolve_spatial,
            // 부팅 조회 — 미는 쪽(`ui.refresh`)은 명령 표에 있다(`commands/settings.rs` 「읽는 자리가 둘인 이유」).
            commands::get_ui_settings,
            // 웹뷰 몫 명령(ADR-0155) — 부팅 보고와 결말 회수 한 쌍(`commands/view_bus.rs`).
            commands::report_view_commands,
            commands::report_command_outcome,
            commands::agent_spawn,
            commands::agent_kill,
            commands::agent_interrupt,
            commands::agent_write_stdin,
            commands::agent_resize,
            commands::set_envelope_format,
            commands::subscribe_output,
            commands::request_replay,
            commands::move_slot_to_window,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        // ADR-0029: 앱은 in-proc 에이전트를 호스팅하지 않으므로 ExitRequested 에서 정리할 manager 가
        // 없다(데몬이 자기 에이전트 graceful 을 담당).
        .run(|_handle, _event| {});
}
