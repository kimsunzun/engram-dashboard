pub mod commands;
pub mod discovery;
pub mod embedded_carrier;

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
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // 기본 warn(OFF) — RUST_LOG 환경변수로 재정의 가능
            logging::init_logging();
            let status_sink = Arc::new(TauriStatusSink {
                app: app.handle().clone(),
            });

            // 프로필 저장 위치 = 앱 데이터 디렉토리(agents.json).
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Step 5: 데몬 발견(없으면 WMI spawn) — §5 LLM 제어 표면. 부팅 자동 호출은 phase4.
            commands::discover_daemon,
            // ADR-0021: 데몬 lifecycle 명시 제어 표면(§5). start=ensure(spawn 허용), stop=fallback kill,
            //   status=alive/pid/port. 재연결 루프는 이걸 안 부른다(attach-only, wsTransport).
            commands::daemon_start,
            commands::daemon_stop,
            commands::daemon_status,
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
