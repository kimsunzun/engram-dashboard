//! 레이아웃 invoke 껍데기 + 그 포트의 Tauri 어댑터 — §5 LLM 제어 표면(ADR-0035/0057).
//!
//! ★이 파일에 로직이 없다★: 16개 `#[tauri::command]` 는 전송 중립 적용 서비스(`crate::layout::apply`)를
//! 부르는 얇은 껍데기이고, 락 규율·순서·검증은 전부 거기 있다(ADR-0081 결정 3 — 사람 클릭과 중계된 LLM
//! 호출이 같은 함수에 떨어진다). 여기 남는 것은 Tauri 세계로의 번역 4종(알림 emit · 구독 재동기 ·
//! OS 창 · 데몬 스폰)뿐이다.
//!
//! ## ★이벤트: 창별 `window:tabs-updated`(ADR-0057)★
//! `view:closed` 는 엔드투엔드 은퇴(더는 emit 안 함 — §5-2/G2). `layout:updated`(뷰 스냅샷)는 유지.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use engram_dashboard_protocol::{AgentCommand, AgentEvent, RequestId};

use crate::commands::popout::{PopupCounter, TauriWindowHost};
use crate::daemon_client::DaemonClient;
use crate::layout::apply;
use crate::layout::{
    AgentSpawner, LayoutEvents, LayoutState, SlotContent, SplitDir, SubscriptionSync, ViewManager,
    ViewSnapshot, WindowHost, WindowTabsPayload,
};
use crate::output_router::{OutputRouter, SubscriptionDelta};

const EVT_LAYOUT_UPDATED: &str = "layout:updated";
// 프론트는 `label` 이 자기 창과 일치할 때만 반응한다(§7-1).
const EVT_WINDOW_TABS_UPDATED: &str = "window:tabs-updated";

// ★ViewManager 락 보유 중 호출 — 구독 정리 델타를 DaemonClient 로 enqueue(fire-and-forget)★. rebuild 와
// **같은 critical section 안**에서 불러 "enqueue 순서 = rebuild 순서"를 세운다(동시 invoke 인터리브 방지).
// 부르는 메서드는 동기 `try_send`(await/network 0)라 락 안에서 ADR-0006 위반 아님(lifecycle 락도 독립 —
// 데드락 0). 비연결이면 DaemonClient 가 조용히 no-op.
pub(crate) fn send_subscription_delta(client: &DaemonClient, delta: SubscriptionDelta) {
    for agent_id in delta.to_unsubscribe {
        client.unsubscribe(agent_id);
    }
}

// ★락 미보유 상태에서 발행★.
pub(crate) fn emit_window_tabs(app: &AppHandle, tabs: &WindowTabsPayload) {
    if let Err(e) = app.emit(EVT_WINDOW_TABS_UPDATED, tabs) {
        tracing::warn!("[layout] {EVT_WINDOW_TABS_UPDATED} emit 실패: {e}");
    }
}

// ── 적용 서비스 포트의 Tauri 어댑터 ──────────────────────────────────────────

// popout 껍데기도 같은 어댑터를 빌려 쓴다 — emit 로직을 두 번 적으면 사람 경로와 LLM 경로가 갈린다.
pub(crate) struct TauriEvents<'a> {
    pub app: &'a AppHandle,
}

impl LayoutEvents for TauriEvents<'_> {
    fn layout_updated(&self, snapshot: &ViewSnapshot) {
        if let Err(e) = self.app.emit(EVT_LAYOUT_UPDATED, snapshot) {
            tracing::warn!("[layout] {EVT_LAYOUT_UPDATED} emit 실패: {e}");
        }
    }

    fn window_tabs_updated(&self, tabs: &WindowTabsPayload) {
        emit_window_tabs(self.app, tabs);
    }
}

pub(crate) struct RouterSubs<'a> {
    pub router: &'a OutputRouter,
    pub client: &'a DaemonClient,
}

impl SubscriptionSync for RouterSubs<'_> {
    fn resync(&self, mgr: &ViewManager) {
        send_subscription_delta(self.client, self.router.rebuild(mgr));
    }
}

// 스폰 응답 해석(어떤 프레임이 성공인가)은 전송 계약이라 어댑터 몫이다.
//
// ★backend fail-loud 근거(ADR-0058)★: 이 wire(`SpawnByCwd{cwd}`)에는 backend 선택 인자가 없고 데몬
// 핸들러는 무조건 자기 고정 기본 백엔드를 스폰한다 — ★오늘 그 값은 claude(`StreamJson` 출력)다★
// (`connection_core.rs` 의 `SpawnByCwd` arm). 그래서 적용 서비스가 명시 backend 를 스폰 전에 거부한다
// — 거부의 지속적 근거는 「무엇이 뜨나」가 아니라 **고를 칸이 wire 에 없다**는 것이다.
struct DaemonSpawner<'a> {
    client: &'a DaemonClient,
}

impl AgentSpawner for DaemonSpawner<'_> {
    fn spawn_by_cwd<'a>(
        &'a self,
        cwd: String,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>> {
        Box::pin(async move {
            let reply = self
                .client
                .send_command(AgentCommand::SpawnByCwd {
                    cwd,
                    request_id: RequestId::new(),
                })
                .await?;
            match reply {
                AgentEvent::Spawned { agent, .. } => Ok(agent.id.to_string()),
                AgentEvent::Error { message, .. } => Err(format!("spawn 실패: {message}")),
                other => Err(format!("spawn 응답 예상 밖(Spawned 기대): {other:?}")),
            }
        })
    }
}

// ── 명령 표가 쥐는 소유형 어댑터 ──────────────────────────────────────────────
//
// ★위 어댑터 넷과 **같은 구현을 재사용한다**★: 표의 핸들러는 `'static` 이라야 하므로 빌린 참조를 담을 수
// 없고, 그렇다고 emit·창 빌드·스폰 로직을 두 번 적으면 사람 경로와 LLM 경로가 다시 갈린다(ADR-0081 결정 3
// 이 없애려던 그 2 코드 경로). 그래서 소유형은 자기 것을 빌려 위 구현에 그대로 넘긴다 — 로직은 한 벌이다.

struct OwnedEvents {
    app: AppHandle,
}

impl LayoutEvents for OwnedEvents {
    fn layout_updated(&self, snapshot: &ViewSnapshot) {
        TauriEvents { app: &self.app }.layout_updated(snapshot);
    }

    fn window_tabs_updated(&self, tabs: &WindowTabsPayload) {
        TauriEvents { app: &self.app }.window_tabs_updated(tabs);
    }
}

struct OwnedSubs {
    router: Arc<OutputRouter>,
    client: Arc<DaemonClient>,
}

impl SubscriptionSync for OwnedSubs {
    fn resync(&self, mgr: &ViewManager) {
        RouterSubs {
            router: &self.router,
            client: &self.client,
        }
        .resync(mgr);
    }
}

struct OwnedWindowHost {
    app: AppHandle,
}

impl WindowHost for OwnedWindowHost {
    fn open(&self, label: &str) -> Result<(), String> {
        TauriWindowHost { app: &self.app }.open(label)
    }

    fn close(&self, label: &str) {
        TauriWindowHost { app: &self.app }.close(label);
    }

    fn is_open(&self, label: &str) -> bool {
        TauriWindowHost { app: &self.app }.is_open(label)
    }
}

struct OwnedSpawner {
    client: Arc<DaemonClient>,
}

impl AgentSpawner for OwnedSpawner {
    fn spawn_by_cwd<'a>(
        &'a self,
        cwd: String,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>> {
        Box::pin(async move {
            DaemonSpawner {
                client: &self.client,
            }
            .spawn_by_cwd(cwd)
            .await
        })
    }
}

/// 레이아웃 명령 표가 쥘 포트 묶음 — 조립부(`lib.rs` setup)가 한 번 만든다.
///
/// ★사람 클릭 경로와 **같은 상태·같은 라우터·같은 발급기**를 받는다★: 다른 인스턴스를 주면 LLM 이 만든 창이
/// 사람이 보는 목록에 없고 label 이 충돌한다(ADR-0035 레이아웃 권위가 하나라는 것의 실물).
pub fn command_ports(
    app: AppHandle,
    state: LayoutState,
    router: Arc<OutputRouter>,
    labels: Arc<PopupCounter>,
    client: Arc<DaemonClient>,
) -> crate::layout::commands::LayoutPorts {
    crate::layout::commands::LayoutPorts {
        state,
        subs: Arc::new(OwnedSubs {
            router: Arc::clone(&router),
            client: Arc::clone(&client),
        }),
        events: Arc::new(OwnedEvents { app: app.clone() }),
        windows: Arc::new(OwnedWindowHost { app: app.clone() }),
        labels,
        spawner: Arc::new(OwnedSpawner { client }),
        // 레이아웃 포트가 아니다 — 표가 하나라 여기 함께 실린다(`layout::commands` 헤더).
        ui_settings: Arc::new(crate::commands::settings::TauriUiSettings::new(app)),
    }
}

// ── 탭 command (창별 — ADR-0057) ─────────────────────────────────────────────

// 창 `window` 에 새 빈-슬롯 탭 추가·활성화. 새 View id 반환. (탭바 `+`.)
#[tauri::command]
pub fn create_tab(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    window: String,
    name: Option<String>,
) -> Result<Uuid, String> {
    apply::create_tab(
        &state,
        &RouterSubs {
            router: &router,
            client: &client,
        },
        &TauriEvents { app: &app },
        &window,
        name,
    )
}

// 빈 새 창(빈 탭 1개) 생성 + 웹뷰 빌드(D-6). 성공 시 새 창 label 반환.
// ★async fn 필수★: WebviewWindowBuilder 데드락 회피(락 밖 빌드).
#[tauri::command]
pub async fn create_window(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    counter: State<'_, Arc<PopupCounter>>,
    client: State<'_, Arc<DaemonClient>>,
) -> Result<String, String> {
    apply::create_window(
        &state,
        &RouterSubs {
            router: &router,
            client: &client,
        },
        &TauriWindowHost { app: &app },
        &**counter,
    )
}

// 창 `window` 의 활성 탭을 `view` 로 교체(그 창만, 타 창 불변).
#[tauri::command]
pub fn switch_tab(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    window: String,
    view: Uuid,
) -> Result<(), String> {
    apply::switch_tab(
        &state,
        &RouterSubs {
            router: &router,
            client: &client,
        },
        &TauriEvents { app: &app },
        &window,
        view,
    )
}

// 창 `window` 의 탭 `view` 닫기(§5-2 상태기계).
#[tauri::command]
pub async fn close_tab(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    window: String,
    view: Uuid,
) -> Result<(), String> {
    apply::close_tab(
        &state,
        &RouterSubs {
            router: &router,
            client: &client,
        },
        &TauriEvents { app: &app },
        &TauriWindowHost { app: &app },
        &window,
        view,
    )
}

// 창 `window` 통째 닫기(모든 탭). 모델에서 창을 지운 뒤 OS 창을 destroy 한다.
#[tauri::command]
pub async fn close_window(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    window: String,
) -> Result<(), String> {
    apply::close_window(
        &state,
        &RouterSubs {
            router: &router,
            client: &client,
        },
        &TauriWindowHost { app: &app },
        &window,
    )
}

// 새 슬롯 id 반환.
#[tauri::command]
pub fn split_slot(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    view_id: Uuid,
    slot_id: Uuid,
    dir: SplitDir,
) -> Result<Uuid, String> {
    apply::split_slot(
        &state,
        &RouterSubs {
            router: &router,
            client: &client,
        },
        &TauriEvents { app: &app },
        view_id,
        slot_id,
        dir,
    )
}

#[tauri::command]
pub fn close_slot(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    view_id: Uuid,
    slot_id: Uuid,
) -> Result<(), String> {
    apply::close_slot(
        &state,
        &RouterSubs {
            router: &router,
            client: &client,
        },
        &TauriEvents { app: &app },
        view_id,
        slot_id,
    )
}

// router/client State 를 안 받는다 — 포커스 이동은 출력 라우팅을 안 바꾼다(ADR-0066, 사유는 적용 서비스).
#[tauri::command]
pub fn focus_slot(
    app: AppHandle,
    state: State<'_, LayoutState>,
    view_id: Uuid,
    slot_id: Uuid,
) -> Result<(), String> {
    apply::focus_slot(&state, &TauriEvents { app: &app }, view_id, slot_id)
}

#[tauri::command]
pub fn rename_tab(
    app: AppHandle,
    state: State<'_, LayoutState>,
    view_id: Uuid,
    name: String,
) -> Result<(), String> {
    apply::rename_tab(&state, &TauriEvents { app: &app }, view_id, name)
}

#[tauri::command]
pub fn assign_agent(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    view_id: Uuid,
    slot_id: Uuid,
    agent_id: String,
) -> Result<(), String> {
    apply::assign_agent(
        &state,
        &RouterSubs {
            router: &router,
            client: &client,
        },
        &TauriEvents { app: &app },
        view_id,
        slot_id,
        agent_id,
    )
}

#[tauri::command]
pub fn set_slot_content(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    view_id: Uuid,
    slot_id: Uuid,
    content: SlotContent,
) -> Result<(), String> {
    apply::set_slot_content(
        &state,
        &RouterSubs {
            router: &router,
            client: &client,
        },
        &TauriEvents { app: &app },
        view_id,
        slot_id,
        content,
    )
}

// ── 합성 command: spawn_into(D-7 배치 지정 스폰) ──────────────────────────────

// 데몬에 에이전트를 스폰하고 그 agent 를 `window` 의 탭 슬롯에 배정한다. 성공 시 새 AgentId(String) 반환.
// 순서·슬롯 정책·실패 가시성은 적용 서비스가 소유한다.
#[tauri::command]
pub async fn spawn_into(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    window: String,
    tab: Option<Uuid>,
    slot: Option<Uuid>,
    backend: Option<String>,
    cwd: String,
) -> Result<String, String> {
    apply::spawn_into(
        &state,
        &RouterSubs {
            router: &router,
            client: &client,
        },
        &TauriEvents { app: &app },
        &DaemonSpawner { client: &client },
        &window,
        tab,
        slot,
        backend,
        cwd,
    )
    .await
}

// ── read-only 조회 ───────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_view(state: State<'_, LayoutState>, view_id: Uuid) -> Result<ViewSnapshot, String> {
    apply::get_view(&state, view_id)
}

// 창 `window` 의 탭 목록 + 활성 + version 조회(= window:tabs-updated 페이로드와 동형).
#[tauri::command]
pub fn list_tabs(
    state: State<'_, LayoutState>,
    window: String,
) -> Result<WindowTabsPayload, String> {
    apply::list_tabs(&state, &window)
}

// 창 label 목록 조회(부팅·진단용).
#[tauri::command]
pub fn list_windows(state: State<'_, LayoutState>) -> Result<Vec<String>, String> {
    apply::list_windows(&state)
}

#[tauri::command]
pub fn resolve_spatial(
    state: State<'_, LayoutState>,
    token: String,
    window: Option<String>,
    view_id: Option<Uuid>,
) -> Result<Option<Uuid>, String> {
    apply::resolve_spatial(&state, &token, window.as_deref(), view_id)
}
