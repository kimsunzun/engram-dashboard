//! Test doubles and builders shared by the daemon client's suites — the frontend-event recorder, the layout command
//! table on a client, the usage-slot layout helpers.

use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use engram_dashboard_command::{CommandEnvelope, CommandTable, OwnerToken};
use engram_dashboard_protocol::{
    AgentBackendKind, AgentEvent, AgentId, AgentInfo, AgentProfile, AgentStatus, Preset,
    RestoreReport, UsageLimitSnapshot, UsageVendorState,
};

use super::events::{ConnectionStateEvent, DaemonEvents};
use super::usage_interest::InterestAction;
use super::{DaemonClient, SharedUsageInterest};
use crate::commands::layout::OwnedSubs;
use crate::commands::popout::PopupCounter;
use crate::layout::commands::{make_table, LayoutPorts, CATALOG_VERSION};
use crate::layout::window_ctl::{Activation, Applied, OsWindowState, WindowAction, WindowControl};
use crate::layout::{
    tree, AgentSpawner, LayoutEvents, LayoutState, MoveFailed, MoveStep, SlotContent, SpareClaim,
    ViewManager, ViewSnapshot, WindowHost, WindowPlace, WindowTabsPayload, MAIN_WINDOW_LABEL,
};
use crate::settings::{SettingsEvents, SettingsService, SettingsSnapshot};
use crate::theme::{EffectiveThemes, ThemeControl, ThemeWindows, UiSettingsPayload};

// 발화를 **온 순서대로** 한 줄씩 적는 가짜. ★Vec 이지 집합이 아니다★ — 이 하네스가 사는 것의 절반이
// 순서다(연결 상태가 broadcast 보다 먼저 나가야 프론트가 출력 Channel 을 등록한 뒤 목록을 받는다).
#[derive(Default)]
pub(crate) struct RecordingEvents {
    seen: std::sync::Mutex<Vec<String>>,
    // Hooks run one per `Connected` notice, front first (none left = nothing runs). ★They run synchronously on the
    //   connection driver's actor★, at the moment `Connected` is shown — the one handle on that moment without a wall
    //   clock. ★Do not panic inside a hook★ — the actor dies and the test ends on a timeout instead of an assertion;
    //   hand what you observed out and assert in the test body.
    on_connected: std::sync::Mutex<std::collections::VecDeque<Box<dyn FnOnce() + Send>>>,
}

impl RecordingEvents {
    fn record(&self, line: String) {
        self.seen
            .lock()
            .expect("recording events poisoned")
            .push(line);
    }

    pub(crate) fn seen(&self) -> Vec<String> {
        self.seen.lock().expect("recording events poisoned").clone()
    }

    pub(crate) fn then_on_connected(&self, hook: impl FnOnce() + Send + 'static) {
        self.on_connected
            .lock()
            .expect("recording events poisoned")
            .push_back(Box::new(hook));
    }
}

impl DaemonEvents for RecordingEvents {
    fn connection_state(&self, state: ConnectionStateEvent) {
        // ★`as_str()` 을 거쳐 적는다★ — 재려는 것은 프론트가 실제로 받는 **그 문자열**이지 enum 이름이 아니다.
        self.record(format!("state:{}", state.as_str()));
        if state == ConnectionStateEvent::Connected {
            let hook = self
                .on_connected
                .lock()
                .expect("recording events poisoned")
                .pop_front();
            if let Some(hook) = hook {
                hook();
            }
        }
    }

    fn agent_list_updated(&self, agents: &[AgentInfo]) {
        self.record(format!("agent_list:{}", agents.len()));
    }

    fn status_changed(&self, agent_id: &AgentId, status: &AgentStatus, epoch: u32) {
        self.record(format!("status:{agent_id}:{status:?}:{epoch}"));
    }

    fn restore_result(&self, _report: &RestoreReport) {
        self.record("restore".to_string());
    }

    fn profile_list_updated(&self, profiles: &[AgentProfile]) {
        self.record(format!("profile_list:{}", profiles.len()));
    }

    fn preset_list_updated(&self, presets: &[Preset]) {
        self.record(format!("preset_list:{}", presets.len()));
    }

    fn usage_limits_updated(
        &self,
        snapshot: &UsageLimitSnapshot,
        labels: &[String],
        socket_epoch: u64,
    ) {
        self.record(format!(
            "usage:{:?}:{}:{}:{socket_epoch}",
            snapshot.vendor,
            snapshot.revision,
            labels.join(",")
        ));
    }
}

// 이름 장부 포트도 같은 한 줄 기록에 적는다 — 프론트 알림과의 차례를 한 목록으로 잰다.
impl super::AgentNamesSink for RecordingEvents {
    fn agents(&self, agents: &[AgentInfo]) {
        let names: Vec<String> = agents
            .iter()
            .map(|agent| format!("{}={}", agent.id, agent.name))
            .collect();
        self.record(format!("names:{}", names.join(",")));
    }
}

pub(crate) fn wire_agent(id: AgentId, name: &str) -> AgentInfo {
    use engram_dashboard_protocol::{
        Capabilities, ControlCaps, InputCaps, ModelCaps, OutputCaps, SessionCaps,
    };
    AgentInfo {
        id,
        name: name.to_string(),
        cwd: "C:\\work".to_string(),
        status: AgentStatus::Running,
        cols: 80,
        rows: 24,
        epoch: 1,
        capabilities: Capabilities {
            input: InputCaps {
                raw: true,
                message: false,
                attachment: false,
            },
            output: OutputCaps {
                terminal_bytes: true,
                structured: false,
                markdown: false,
                tool_events: false,
                usage: false,
            },
            control: ControlCaps {
                resize: true,
                interrupt: false,
                cancel: false,
                graceful_shutdown: false,
            },
            session: SessionCaps {
                resume: false,
                snapshot: false,
                cwd_env: false,
            },
            model: ModelCaps {
                select: false,
                temperature: false,
                max_tokens: false,
            },
        },
    }
}

// 관심을 「main 의 활성 탭 = 사용량 슬롯 하나」 레이아웃으로 다시 계산한다. 넛지는 부르는 쪽이 정한다 — 결과를 안 쓰는
// 자리는 `let _ =` 로 버린다(연결 전이라 넛지가 할 일이 없거나, 시험이 넛지를 직접 넣는다).
pub(crate) fn relayout(
    usage: &SharedUsageInterest,
    show_claude: bool,
    show_codex: bool,
) -> InterestAction {
    let mut mgr = ViewManager::new();
    let view = mgr.windows[MAIN_WINDOW_LABEL].active;
    let slot = tree::first_slot_id(&mgr.views[&view].layout);
    mgr.set_slot_content(
        view,
        slot,
        SlotContent::Usage {
            show_claude,
            show_codex,
        },
    )
    .unwrap();
    usage.lock().recompute(&mgr, Instant::now())
}

pub(crate) fn usage_update(
    vendor: AgentBackendKind,
    revision: u64,
    subscribed: &[AgentBackendKind],
) -> AgentEvent {
    AgentEvent::UsageLimitsUpdated {
        snapshot: UsageLimitSnapshot {
            vendor,
            account_key: "default".into(),
            five_hour: None,
            weekly: None,
            model_scoped: Vec::new(),
            plan: None,
            in_flight: false,
            state: UsageVendorState::Ready,
            revision,
        },
        subscribed: subscribed.to_vec(),
    }
}

pub(crate) const POPUP: &str = "slot-popup-7";

pub(crate) struct NoLayoutEvents;

impl LayoutEvents for NoLayoutEvents {
    fn layout_updated(&self, _snapshot: &ViewSnapshot) {}
    fn window_tabs_updated(&self, _tabs: &WindowTabsPayload) {}
}

struct NoWindows;

const NO_OS_WINDOW: &str = "이 시험에는 OS 창이 없다";

impl WindowHost for NoWindows {
    fn open(&self, _label: &str) -> Result<(), String> {
        Err(NO_OS_WINDOW.to_string())
    }
    fn open_at(&self, _label: &str, _at: &WindowPlace, _activate: bool) -> Result<(), String> {
        Err(NO_OS_WINDOW.to_string())
    }
    fn close(&self, _label: &str) {}
    fn is_open(&self, _label: &str) -> bool {
        false
    }
    fn move_window(&self, _label: &str, _at: &WindowPlace) -> Result<(), MoveFailed> {
        Err(MoveFailed {
            step: MoveStep::Position,
            error: NO_OS_WINDOW.to_string(),
        })
    }
    fn claim_spare(&self) -> Option<SpareClaim> {
        None
    }
    fn committed(&self, _claim: &SpareClaim) {}
    fn release_spare(&self, _claim: &SpareClaim) {}
}

struct NoWindowControl;

impl WindowControl for NoWindowControl {
    fn apply(&self, _label: &str, _action: WindowAction) -> Result<Applied, String> {
        Err(NO_OS_WINDOW.to_string())
    }
    fn show_all(&self, _how: Activation) -> bool {
        false
    }
    fn hide_all(&self) {}
    fn state(&self, _label: &str) -> Option<OsWindowState> {
        None
    }
}

struct NoSpawner;

impl AgentSpawner for NoSpawner {
    fn spawn_by_cwd<'a>(
        &'a self,
        _cwd: String,
        _backend: Option<AgentBackendKind>,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>> {
        Box::pin(async { Err("이 시험은 스폰하지 않는다".to_string()) })
    }
}

struct NoThemeWindows;

impl ThemeWindows for NoThemeWindows {
    fn labels(&self) -> Vec<String> {
        Vec::new()
    }
    fn send(&self, _label: &str, _payload: UiSettingsPayload) -> Result<(), String> {
        Err("이 시험에는 웹뷰 창이 없다".to_string())
    }
}

struct NoSettingsEvents;

impl SettingsEvents for NoSettingsEvents {
    fn changed(&self, _change: &SettingsSnapshot) {}
    fn theme_default_changed(&self) {}
}

// 운영 버스 표와 같은 구독 재동기 어댑터(`OwnedSubs` — 사람 경로의 `RouterSubs` 에 넘긴다)를 끼운 레이아웃 명령 표.
pub(crate) fn bus_table(state: &LayoutState, client: &Arc<DaemonClient>) -> CommandTable {
    // 쓰기를 안 연다 — 이 시험은 설정을 건드리지 않고, 적재는 없는 폴더를 만들지 않는다.
    let settings = Arc::new(SettingsService::load_from_dir(
        &std::env::temp_dir().join("engram-dashboard-no-settings"),
    ));
    let themes = ThemeControl::new(
        Arc::new(EffectiveThemes::new(Arc::clone(&settings), state.clone())),
        Arc::new(NoThemeWindows),
    );
    make_table(LayoutPorts {
        state: state.clone(),
        subs: Arc::new(OwnedSubs {
            router: client.router.clone(),
            client: Arc::clone(client),
        }),
        events: Arc::new(NoLayoutEvents),
        windows: Arc::new(NoWindows),
        labels: Arc::new(PopupCounter::default()),
        spawner: Arc::new(NoSpawner),
        control: Arc::new(NoWindowControl),
        themes,
        settings,
        settings_events: Arc::new(NoSettingsEvents),
        // 포트를 꽂지 않은 조율자 — 이 시험은 복원에 답하지 않는다.
        restore: Arc::new(crate::state::restore::RestoreCoordinator::new(
            Arc::new(crate::state::restore::RestoreService::new()),
            state.clone(),
            Arc::default(),
            Arc::new(PopupCounter::default()),
        )),
    })
}

// 데몬이 배달하는 셸 명령 한 장(LLM 의 `engram layout.setSlotContent …` 가 셸에 닿는 모양).
pub(crate) fn bus_request(name: &str, args: serde_json::Value) -> AgentEvent {
    let envelope = CommandEnvelope {
        name: name.to_string(),
        request_id: engram_dashboard_command::RequestId::new(),
        owner: OwnerToken::new("shell"),
        proto_ver: CATALOG_VERSION,
        args,
    };
    AgentEvent::CommandRequest {
        envelope: serde_json::value::to_raw_value(&envelope).expect("봉투 직렬화"),
    }
}

// `window` 활성 탭의 첫 슬롯.
pub(crate) fn active_slot(state: &LayoutState, window: &str) -> (uuid::Uuid, uuid::Uuid) {
    let mgr = state.0.lock().unwrap();
    let view = mgr.windows[window].active;
    (view, tree::first_slot_id(&mgr.views[&view].layout))
}

// 창 빌드가 늘 성공하는 호스트 — 슬롯 옮기기가 phase B 를 지나 phase C 까지 간다.
pub(crate) struct OpenWindows;

impl WindowHost for OpenWindows {
    fn open(&self, _label: &str) -> Result<(), String> {
        Ok(())
    }
    fn open_at(&self, _label: &str, _at: &WindowPlace, _activate: bool) -> Result<(), String> {
        Ok(())
    }
    fn close(&self, _label: &str) {}
    fn is_open(&self, _label: &str) -> bool {
        true
    }
    fn move_window(&self, _label: &str, _at: &WindowPlace) -> Result<(), MoveFailed> {
        Ok(())
    }
    fn claim_spare(&self) -> Option<SpareClaim> {
        None
    }
    fn committed(&self, _claim: &SpareClaim) {}
    fn release_spare(&self, _claim: &SpareClaim) {}
}
