pub mod commands;
// S13 sub-step 2: 순수 discovery 로직은 engram-dashboard-discovery crate 로 이동(tray-host 와 공유).
// 호출부(commands/discovery.rs)가 crate::discovery 경로를 그대로 쓰도록 re-export 만 남긴다(중복 코드 0).
pub use engram_dashboard_discovery as discovery;
pub mod embedded_carrier;
// ADR-0026 2단계: 트레이를 앱에 통합(네이티브 TrayIconBuilder 배선). core=순수, actions=공유 부수효과.
mod tray;

// S12 phase 1: agent(구 pty)/persistence/logging 은 engram-dashboard-core 로 이동. 여기선 re-import 만.
use engram_dashboard_core::{agent, logging, persistence};

use std::sync::Arc;

use tauri::{Emitter, Manager};

use agent::manager::AgentManager;
use agent::profile::{ProfileRegistry, RestoreReport};
use agent::session_tracker::{SessionTracker, TrackerConfig};
use agent::types::{AgentId, AgentInfo, AgentStatus};
use persistence::FileProfileStore;

// ── AppState ─────────────────────────────────────────────────────────────────

/// Tauri 관리 상태 — AgentManager 접근점. 외부 Mutex 없음(M1).
pub struct AppState {
    pub manager: Arc<AgentManager>,
    /// ADR-0020 Stage 2: embedded 단일 in-proc 연결(ConnectionCore + inbound mpsc + command loop).
    /// agent_connect 가 Channel 을 등록하고, agent_command 가 inbound 에 명령을 넣는다. 기존 invoke
    /// 경로와 ★공존★(Stage 4 에서 옛 경로 제거).
    pub embedded: embedded_carrier::EmbeddedConnection,
}

// ── TauriStatusSink ───────────────────────────────────────────────────────────

/// agent-status-changed 이벤트 페이로드 — 프론트 타입과 일치 필수
#[derive(serde::Serialize, Clone)]
struct AgentStatusChanged {
    id: AgentId,
    status: AgentStatus,
    /// S9 §18-d: 재spawn 트리거 epoch. 프론트가 옛 세션의 지연 알림을 버릴 수 있게 한다.
    epoch: u32,
}

/// StatusSink의 Tauri 구현 — AppHandle로 저빈도 상태 이벤트 emit
pub struct TauriStatusSink {
    app: tauri::AppHandle,
}

impl agent::types::StatusSink for TauriStatusSink {
    fn status_changed(&self, id: AgentId, status: AgentStatus, epoch: u32) {
        let payload = AgentStatusChanged { id, status, epoch };
        // emit 실패는 무시(로그만) — 창이 닫히는 중일 수 있음. 패닉 금지.
        if let Err(e) = self.app.emit("agent-status-changed", payload) {
            tracing::warn!("emit agent-status-changed failed: {e}");
        }
    }

    fn agent_list_updated(&self, agents: Vec<AgentInfo>) {
        if let Err(e) = self.app.emit("agent-list-updated", agents) {
            tracing::warn!("emit agent-list-updated failed: {e}");
        }
    }

    fn restore_result(&self, report: RestoreReport) {
        // 복원 결과를 프론트에 통지(S9 §18-d). 실패는 로그만.
        if let Err(e) = self.app.emit("agent-restore-result", report) {
            tracing::warn!("emit agent-restore-result failed: {e}");
        }
    }
}

// ── run() ────────────────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // ADR-0027 보강 §63: 모드는 run() 최상단 1회 확정 → 트레이 게이트·X=hide 게이트(이 커밋),
    // single-instance·data_dir·주입(후속 커밋)이 전부 이 단일 값에 의존. 전환=self-relaunch(새 프로세스).
    let mode = discovery::resolve_mode();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            // 기본 warn(OFF) — RUST_LOG 환경변수로 재정의 가능
            logging::init_logging();
            let status_sink = Arc::new(TauriStatusSink {
                app: app.handle().clone(),
            });

            // 프로필 저장 위치 = data_dir/agents.json. 단일 출처(ADR-0024) — daemon 과 같은
            // `.engram-data/` 를 보게 discovery::default_data_dir() 에 위임(옛 app_data_dir 대체).
            // ADR-0027 보강: run() 최상단에서 확정한 실 mode 를 적용(setup 클로저가 캡처). debug 에선 모드 무관.
            let data_dir = discovery::default_data_dir(mode);
            let store = Arc::new(FileProfileStore::new(data_dir));
            let profiles = Arc::new(ProfileRegistry::new(store));

            // 세션 추적: sid 변경(/clear 등) 관측 시 레지스트리에 반영(즉시 persist).
            let profiles_cb = profiles.clone();
            let tracker = Arc::new(SessionTracker::new(
                TrackerConfig::default(),
                Arc::new(move |agent_id, new_sid| {
                    profiles_cb.observe_session_id(agent_id, new_sid);
                }),
            ));
            tracker.start();

            let manager = Arc::new(AgentManager::new(status_sink, profiles, tracker));

            // 복원은 백그라운드 — 앱 창 블로킹 방지(H-1.8). stagger·조기종료 윈도 대기 포함.
            let mgr = manager.clone();
            std::thread::spawn(move || {
                mgr.restore_all();
            });

            // ── ADR-0020 Stage 2: embedded 단일 in-proc 연결 기동 ───────────────────────
            // WS 의 handle_connection 1회에 대응(앱당 1개 영속). ConnectionCore 를 만들어
            // command loop(inbound mpsc 직렬화)를 띄운다.
            //
            // ★ConnRegistry/shutdown_tx 는 embedded 에선 무력★:
            //  - registry: WS conn_tx fanout 용(ProfileListUpdated/InputLeaseChanged broadcast). embedded
            //    Channel 은 여기 등록 불가(타입 상이) → 빈 registry 라 broadcast 는 no-op. 상태/목록은
            //    TauriStatusSink(status_changed/agent_list_updated/restore_result)가 별도로 emit 하므로
            //    트리 갱신은 유지된다. ProfileListUpdated 갱신은 프론트가 응답(Created/Ack) 후 ListProfiles
            //    재호출로 흡수(Stage 3) — 기존 invoke 동작과 동일. (carrier-중립 fanout 은 후속 과제.)
            //  - shutdown_tx: StopDaemon → main 종료 트리거용. embedded 는 in-proc 라 무의미 → dummy watch.
            let registry = engram_dashboard_daemon::ws::ConnRegistry::new();
            let multiview = engram_dashboard_daemon::connection_core::MultiViewState::new();
            let (shutdown_tx, _shutdown_rx) = tokio::sync::watch::channel(false);
            let core = Arc::new(
                engram_dashboard_daemon::connection_core::ConnectionCore::new(
                    manager.clone(),
                    multiview,
                    registry,
                    shutdown_tx,
                ),
            );
            let embedded = embedded_carrier::spawn_embedded_connection(core);

            app.manage(AppState { manager, embedded });

            // ── ADR-0026 2단계: 네이티브 트레이 배선 ─────────────────────────────────────
            // 아이콘 두 벌 생성·메뉴·핸들러 + setup 직후 데몬 상태로 아이콘 확정. Windows 전용.
            // ADR-0027 B안: 트레이는 daemon 모드 전용 — embedded 는 평범한 창 앱(트레이 미생성).
            if mode == discovery::AppMode::Daemon {
                if let Err(e) = tray::build_tray(app) {
                    tracing::warn!("트레이 생성 실패(앱은 계속): {e}");
                }
            }
            Ok(())
        })
        // ADR-0026 2단계 + ADR-0027 B안/§20: main X(WM_CLOSE) 동작은 ★모드별로 갈린다★.
        // - daemon: 트레이=앱 통합이라 X=hide(창만 숨기고 트레이 상주) — 진짜 종료는 트레이 "완전 종료"(app.exit(0))뿐.
        // - embedded: X=종료(재오픈 cold restore, §20). app.exit(0) 명시 호출이 ★필수★다.
        //   이유: tauri.conf.json 에 보조 hidden 창(agent-tree/slot-popup, visible:false)이 부팅 시 함께 생성돼
        //   (WindowConfig.create 기본 true) main 만 닫혀도 window store 가 비지 않는다(is_empty()==false).
        //   → RunEvent::ExitRequested 가 emit 되지 않아 프로세스가 좀비로 남는다(embedded 는 트레이도 없어 회수 불가).
        //   app.exit(0) → RunEvent::ExitRequested → 아래 .run() 의 shutdown_all(graceful PTY 정리)을 탄다.
        // (구 동작 = 무조건 X=hide. ADR-0027 B안으로 daemon-gate 됨. 더 구: app.exit(0) → ADR-0026 폐기.)
        // ★main 만 대상★: agent-tree/slot-popup(hidden 창)은 기존대로 단독 close 처리. main 라벨만
        //  분기 — conf 첫 창은 label 미지정이라 Tauri 기본 라벨 "main".
        // 주의: CloseRequested 는 Rust 측 이벤트 관찰이라 JS capability(core:window:allow-close) 불필요.
        .on_window_event(move |window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    match mode {
                        discovery::AppMode::Daemon => {
                            // ADR-0027 B안: daemon=X=hide(트레이 상주). prevent_close 후 hide.
                            api.prevent_close();
                            let _ = window.hide();
                        }
                        discovery::AppMode::Embedded => {
                            // ADR-0027 §20: embedded=X=종료(재오픈 cold restore). 보조 hidden 창
                            // (agent-tree/slot-popup)이 상주해 main close 만으론 이벤트루프가 안 끝난다(좀비).
                            // → app.exit(0) 명시 → RunEvent::ExitRequested → shutdown_all(graceful PTY 정리).
                            window.app_handle().exit(0);
                        }
                    }
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
            // ADR-0026 2단계 §5: 트레이 동작의 LLM/cdp 제어 표면(트레이 핸들러와 같은 actions 함수).
            //   데몬 켜기/끄기는 위 daemon_start/daemon_stop 재사용 → 여기엔 창/종료만.
            commands::show_main_ui,
            commands::hide_main_ui,
            commands::quit_app,
            // ADR-0020 Stage 4a: 옛 개별 invoke(spawn/kill/profile/pty 14개)는 삭제 — 아래 generic
            //   agent_command 1개가 AgentCommand 전 variant 를 처리(embedded carrier → ConnectionCore).
            //   agent_connect 가 단일 outbound Channel 을 등록한다.
            embedded_carrier::agent_connect,
            embedded_carrier::agent_command,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|handle, event| {
            // §12(b): 앱 종료 시 PTY graceful 정리.
            // Windows KILL_ON_JOB_CLOSE 안전망 있지만 명세(§12(b)) 준수.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                handle.state::<AppState>().manager.shutdown_all();
            }
        });
}
