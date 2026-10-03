pub mod commands;
pub mod daemon_client;
mod fsutil;
pub mod layout;
pub mod output_channel;
pub mod output_router;
pub mod settings;
mod state;
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
use tauri::Manager;

// ── run() ────────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // ADR-0029: 부팅 기동(autostart 등록 인자에 --hidden 포함)은 창 없이 트레이만 상주시킨다.
    let hidden = std::env::args().any(|a| a == "--hidden");

    // ── 화면 상태(TRD S21-storage §6) — 부팅 단계 플러그인 · 사용자 setup · 종료가 같은 인스턴스를 본다 ──
    let layout = crate::layout::LayoutState::new();
    let tree_attrs = std::sync::Arc::new(crate::state::tree_attrs::TreeAttrs::default());
    let labels = std::sync::Arc::new(crate::commands::popout::PopupCounter::default());
    let state_session = std::sync::Arc::new(crate::state::boot_plugin::StateSession::default());

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
    // ★단일 인스턴스 바로 뒤에 둔다★ — 로그 초기화 · 셸 실행 잠금 · 상태 파일 판정 · 모델 채우기를 관문 뒤 · 어느
    //   창보다 앞에서 한다(TRD S21-storage §6-5 · 정본 = 그 모듈 머리).
    builder = builder.plugin(crate::state::boot_plugin::init(
        crate::state::boot_plugin::Boot {
            layout: layout.clone(),
            tree: tree_attrs.clone(),
            labels: labels.clone(),
            session: state_session.clone(),
        },
    ));
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
    //   그 안의 모델은 부팅 단계 플러그인이 창보다 먼저 판정한 모델로 갈아끼운다(TRD S21-storage §6-5 ⑥).
    let setup_layout = layout.clone();
    let setup_tree = tree_attrs.clone();
    builder = builder.manage(layout);
    builder = builder.manage(tree_attrs).manage(labels.clone());
    builder = builder.manage(crate::state::placement::DeferredMaximize::default());

    // ── 셸 설정 + 유효 테마(TRD S21-storage §5-3 · §5-6) ─────────────────────────────
    // ★위 LayoutState 와 같은 이유로 빌더에서 manage 한다(ADR-0102)★ — 웹뷰의 첫 `get_ui_settings` ·
    //   `settings_get` 이 setup 보다 먼저 올 수 있고, 그때 상태가 없으면 그 창은 기본값으로 굳는다.
    // ★여기서는 읽기만 한다★ — 적재는 파일·폴더를 만들지도 고치지도 않고, 쓰기는 setup 의 `enable_writes`
    //   뒤에만 열린다. 단일 인스턴스 관문(위 플러그인)은 build 안에서 판정되므로, 여기서 디스크를 바꾸면 곧
    //   종료될 두 번째 인스턴스도 그것을 바꾼다.
    let settings = std::sync::Arc::new(crate::settings::SettingsService::load_from_dir(
        &crate::discovery::DataLayout::resolve().shell_config_dir(),
    ));
    // ★셸에 하나★ — 밀기 순서를 지키는 락이 이 안에 있다(사람 경로·LLM 경로가 같은 인스턴스를 본다).
    let themes = std::sync::Arc::new(crate::ui_settings::EffectiveThemes::new(
        settings.clone(),
        Box::new(crate::ui_settings::FileSource::in_data_dir()),
    ));
    builder = builder.manage(settings.clone()).manage(themes.clone());

    let exit_session = state_session.clone();
    builder
        .setup(move |app| {
            // 로그는 부팅 단계 플러그인이 이미 열었다(TRD S21-storage §6-5 ⓪).

            // ── 부팅 단계 ⑦: 기록기 시작(가드면 띄우지 않는다 — §6-5 ③) ─────────────────────
            state_session.start_saver();

            // ── 죽은 창의 테마 항목 쓸기 ─────────────────────────────────────────────────
            // ★부팅에서만 돈다 — `ui.refresh` 로 옮기지 말 것★. **이 순간 팝아웃 창이 하나도 없고**(복원한
            //   팝아웃도 아직 창이 없다) 그 label 은 이 실행에서 새로 받은 것이라(§6-3), 지금 파일에 있는
            //   비-선언 label 은 생사를 물을 것도 없이 정의상 전부 죽은 것이다. 여기에 생존 확인을 덧대면
            //   아직 만들어지는 중인 창의 항목을 지우는 경합이 되살아난다(사유·불변식 전문 =
            //   `ui_settings::sweep_dead_windows`). 로그 자리를 잡은 뒤에 부른다 — 무엇을 지웠는지가 이 앱
            //   로그에만 남는다.
            // ADR-0167
            crate::commands::settings::sweep_dead_window_entries(app.handle());

            // 단일 인스턴스 관문을 지난 뒤라 디스크를 바꿔도 된다(빌더 쪽 적재 주석). 로거가 선 뒤라 적재가
            //   모아 둔 로그(못 쓰는 `settings.json` · 접힌 값)도 여기서 나간다.
            settings.enable_writes();

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
                                    settings.clone(),
                                    themes.clone(),
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

            // ── 부팅 단계 ⑧ ⑨: main · 트리 창 자리 · 복원한 팝아웃 창 · 테마 한 번(TRD S21-storage §6-5) ──
            // 설정 창은 이미 있다(사용자 setup). 팝아웃 창은 위 쓸기 뒤에 연다 — 그 쓸기의 전제가 「이 순간 팝아웃
            //   창이 하나도 없다」다. `--hidden` 이면 아래 숨기기가 이 창들도 숨긴다(사용자 결정 F13).
            crate::state::placement::restore_windows(
                app.handle(),
                &setup_layout,
                &setup_tree,
                &themes,
            );

            // ── 부팅 단계 ⑩: 파생 표(라우터 · 사용량 관심)를 마지막 창 묶음으로 한 번 다시 계산한다 ──
            // 부팅 단계 플러그인은 이 클라이언트보다 먼저 돌아 부를 수 없었고, ⑨ 가 못 연 팝아웃을 모델에서 지운
            //   뒤여야 한다. 안 하면 복원한 슬롯의 에이전트 출력이 다음 레이아웃 변경까지 어느 창에도 안 간다.
            if let Some(client) = app.try_state::<std::sync::Arc<crate::daemon_client::DaemonClient>>() {
                match setup_layout.0.lock() {
                    Ok(mgr) => crate::layout::SubscriptionSync::resync(
                        &crate::commands::layout::RouterSubs {
                            router: &router,
                            client: &client,
                        },
                        &mgr,
                    ),
                    Err(_) => {
                        tracing::error!("레이아웃 락에 독이 들어 부팅 뒤 구독 재계산을 못 했다")
                    }
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
                crate::tray::actions::hide_main_ui(app.handle());
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
                    // ADR-0229: 숨기기 경로는 하나다 — 여기서 창을 따로 숨기면 입구마다 숨기는 창이 갈린다.
                    if window.label() == crate::layout::MAIN_WINDOW_LABEL {
                        api.prevent_close();
                        crate::tray::actions::hide_main_ui(window.app_handle());
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
                // 창 자리 기록(TRD S21-storage §6-3) — 사건 값 대신 게터를 다시 읽는다(최소화 · 최대화 여부가 같이
                //   필요하다).
                tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                    crate::state::placement::record(window);
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
            commands::get_usage_snapshot,
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
            commands::set_split_ratio,
            commands::close_slot,
            commands::focus_slot,
            commands::rename_tab,
            commands::assign_agent,
            commands::set_slot_content,
            commands::set_usage_slot,
            commands::spawn_into,
            commands::get_view,
            commands::list_tabs,
            commands::list_windows,
            commands::resolve_spatial,
            // 측정 보고(웹뷰 → 셸) — 버스 명령이 아니다(ADR-0227).
            commands::report_window_canvas,
            commands::report_ui_metrics,
            // 부팅 조회 — 미는 쪽(`ui.refresh` · `theme.default` 쓰기)은 따로 있다(`commands/settings.rs`
            //   「창별 테마를 읽는 자리가 둘인 이유」).
            commands::get_ui_settings,
            // 셸 설정 — 버스 `settings.*` 와 같은 서비스(ADR-0081 결정 3).
            commands::settings_get,
            commands::settings_set,
            commands::settings_reset,
            commands::settings_schema,
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
        // ADR-0029: 앱은 in-proc 에이전트를 호스팅하지 않으므로 종료 때 거둘 manager 가 없다(데몬이 자기 에이전트
        // graceful 을 담당). 여기서 하는 일은 화면 상태의 정상 종료 쓰기와 셸 실행 잠금 놓기뿐이다(TRD
        // S21-storage §6-6) — 트레이 「완전 종료」(`app.exit(0)`)도 이 사건을 낸다.
        .run(move |_handle, event| {
            if let tauri::RunEvent::Exit = event {
                exit_session.shutdown();
            }
        });
}
