pub mod commands;
pub mod logging;
pub mod persistence;
pub mod pty;

use std::sync::Arc;

use tauri::{Emitter, Manager};
use uuid::Uuid;

use persistence::FileProfileStore;
use pty::manager::AgentManager;
use pty::profile::{ProfileRegistry, RestoreReport};
use pty::session_tracker::{SessionTracker, TrackerConfig};
use pty::types::{AgentId, AgentInfo, AgentStatus, OutputSink, SinkError, SinkId};

// ── AppState ─────────────────────────────────────────────────────────────────

/// Tauri 관리 상태 — AgentManager 접근점. 외부 Mutex 없음(M1).
pub struct AppState {
    pub manager: Arc<AgentManager>,
}

// ── ChannelOutputSink ─────────────────────────────────────────────────────────

use pty::types::PtyEvent;

/// OutputSink의 Tauri 구현 — Tauri IPC Channel을 OutputSink trait으로 래핑
pub struct ChannelOutputSink {
    id: SinkId,
    channel: tauri::ipc::Channel<PtyEvent>,
}

impl ChannelOutputSink {
    pub fn new(channel: tauri::ipc::Channel<PtyEvent>) -> Self {
        Self {
            id: Uuid::new_v4(),
            channel,
        }
    }
}

impl OutputSink for ChannelOutputSink {
    fn send(&self, event: PtyEvent) -> Result<(), SinkError> {
        // send 실패 = 창이 닫힘 → drain이 dead sink로 감지해 구독자 목록에서 제거.
        self.channel.send(event).map_err(|_| SinkError)
    }

    fn sink_id(&self) -> SinkId {
        self.id
    }
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

impl pty::types::StatusSink for TauriStatusSink {
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

            app.manage(AppState { manager });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::spawn_agent,
            commands::kill_agent,
            commands::get_agents,
            commands::subscribe_agent_output,
            commands::unsubscribe_agent_output,
            commands::write_stdin,
            commands::resize_pty,
            commands::get_agent_snapshot,
            // S9-6: 프로필 CRUD + 프로필 기반 spawn
            commands::list_profiles,
            commands::create_claude_profile,
            commands::delete_profile,
            commands::spawn_profile,
            commands::set_profile_auto_restore,
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
