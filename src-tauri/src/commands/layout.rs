//! Layout invoke shell + the Tauri adapters of its ports — §5 LLM control surface (ADR-0035/0057).
//!
//! ★There is no logic in this file★: all of `#[tauri::command]` are thin shells that call the transport-neutral apply service (`crate::layout::apply`),
//! and lock discipline · order · validation are all there (ADR-0081 decision 3 — a human click and a relayed LLM
//! call land on the same function). What remains here is only 4 kinds of translation into the Tauri world (notification emit (+ OS window title — `crate::chrome`) · subscription resync ·
//! OS window · daemon spawn), the window label of measurement reports (taken from the `Window` that Tauri injects), and unpacking the window argument (the same
//! resolver as the bus — [`resolve_window`]).
//!
//! ## ★Event: per-window `window:tabs-updated` (ADR-0057)★
//! `view:closed` is retired end to end (no longer emitted — §5-2/G2). `layout:updated` (view snapshot) is kept.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State, Window};
use uuid::Uuid;

use engram_dashboard_protocol::{AgentBackendKind, AgentCommand, AgentEvent, RequestId};

use crate::commands::popout::{PopupCounter, TauriWindowHost};
use crate::commands::settings::TauriSettingsEvents;
use crate::commands::window_ctl::TauriWindowControl;
use crate::daemon_client::protocol_state::CommandFailure;
use crate::daemon_client::DaemonClient;
use crate::layout::apply;
use crate::layout::window_arg;
use crate::layout::{
    AgentSpawner, LayoutEvents, LayoutState, MoveFailed, NewWindow, SlotContent, SpareClaim,
    SpawnRefusal, SpawnedAgent, SplitDir, SplitRatioApplied, SubscriptionSync, UiMetrics,
    ViewManager, ViewSnapshot, WindowHost, WindowPlace, WindowTabsPayload,
};
use crate::output_router::{OutputRouter, SubscriptionDelta};
use crate::settings::SettingsService;
use crate::spare::SpareState;
use crate::state::restore::RestoreCoordinator;
use crate::theme::ThemeControl;

const EVT_LAYOUT_UPDATED: &str = "layout:updated";
// The frontend reacts only when `label` matches its own window (§7-1).
const EVT_WINDOW_TABS_UPDATED: &str = "window:tabs-updated";

// ★Called holding the ViewManager lock — queues the subscription cleanup delta on the DaemonClient (fire-and-forget)★.
// Called **inside the same critical section** as the rebuild so the queue order is the rebuild order (no interleaving
// of concurrent invokes). The method only sends to the connection driver's actor (no await, no network, no other
// lock), so calling it under the lock keeps ADR-0006. With no open connection the client does nothing.
pub(crate) fn send_subscription_delta(client: &DaemonClient, delta: SubscriptionDelta) {
    for agent_id in delta.to_unsubscribe {
        client.unsubscribe(agent_id);
    }
}

/// Resolve the window argument into the label of a window currently in the model — the same resolver as the bus verbs (TRD S26-window-chrome §4-5). The web passes
/// its own window label, so it passes through as is. The layout lock is held only while resolving.
pub(crate) fn resolve_window(state: &LayoutState, window: &str) -> Result<String, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    window_arg::resolve(&mgr, window).map_err(|e| e.to_string())
}

// ★Emitted without holding the lock★.
pub(crate) fn emit_window_tabs(app: &AppHandle, tabs: &WindowTabsPayload) {
    if let Err(e) = app.emit(EVT_WINDOW_TABS_UPDATED, tabs) {
        tracing::warn!("[layout] {EVT_WINDOW_TABS_UPDATED} emit 실패: {e}");
    }
}

// ── Tauri adapters of the apply service's ports ──────────────────────────────────────────

// The popout shell borrows the same adapter too — if emit logic is written twice, the human path and the LLM path diverge.
pub(crate) struct TauriEvents<'a> {
    pub app: &'a AppHandle,
}

impl LayoutEvents for TauriEvents<'_> {
    fn layout_updated(&self, snapshot: &ViewSnapshot) {
        if let Err(e) = self.app.emit(EVT_LAYOUT_UPDATED, snapshot) {
            tracing::warn!("[layout] {EVT_LAYOUT_UPDATED} emit 실패: {e}");
        }
    }

    // The title in the payload is also applied to the OS window — the web does not read that field (TRD S26-window-chrome §4-4).
    fn window_tabs_updated(&self, tabs: &WindowTabsPayload) {
        emit_window_tabs(self.app, tabs);
        crate::chrome::push_title(self.app, tabs);
    }
}

pub(crate) struct RouterSubs<'a> {
    pub router: &'a OutputRouter,
    pub client: &'a DaemonClient,
}

impl SubscriptionSync for RouterSubs<'_> {
    fn resync(&self, mgr: &ViewManager) {
        send_subscription_delta(self.client, self.router.rebuild(mgr));
        self.client.usage_layout_changed(mgr);
    }
}

// Interpreting the spawn response (which frame is success) is a transport contract, so it is the adapter's job.
//
// backend puts the chosen word on the wire as is — if empty, the daemon rejects it (there is no default on either side).
// `deliver_notices` = this caller shows the reply's notices to a reader (`SpawnByCwd.deliver_notices`): only the bus
//   spawner does; the webview does not read them yet (TRD S25 step 20 §7-1 — step 11).
// ADR-0348 (M13)
pub(crate) struct DaemonSpawner<'a> {
    pub(crate) client: &'a DaemonClient,
    pub(crate) deliver_notices: bool,
}

impl AgentSpawner for DaemonSpawner<'_> {
    fn spawn_by_cwd<'a>(
        &'a self,
        cwd: String,
        backend: Option<AgentBackendKind>,
    ) -> Pin<Box<dyn Future<Output = Result<SpawnedAgent, SpawnRefusal>> + Send + 'a>> {
        Box::pin(async move {
            let reply = self
                .client
                .send_command_with_origin(AgentCommand::SpawnByCwd {
                    cwd,
                    backend,
                    request_id: RequestId::new(),
                    deliver_notices: self.deliver_notices,
                })
                .await;
            match reply {
                Ok(AgentEvent::Spawned { agent, notices, .. }) => Ok(SpawnedAgent {
                    id: agent.id.to_string(),
                    notices: notices.unwrap_or_default(),
                }),
                // ADR-0279: daemon rejection messages (including policy rejections) go through without a prefix.
                // ADR-0348: the block reasons and the created-asleep id travel next to the text, as received.
                Err(CommandFailure::Daemon {
                    message,
                    launch_blocked,
                    created_agent_id,
                }) => Err(SpawnRefusal {
                    message,
                    launch_blocked,
                    created_agent_id,
                    live_agent_id: None,
                    notices: Vec::new(),
                }),
                // A reply timeout (the link's round-trip cap) arrives here too.
                Err(CommandFailure::Local(message)) => Err(message.into()),
                Ok(other) => Err(format!("spawn 응답 예상 밖(Spawned 기대): {other:?}").into()),
            }
        })
    }
}

// ── Owning adapters held by the command table ──────────────────────────────────────────────
//
// ★**Reuses the same implementation** as the four adapters above★: the table's handlers must be `'static`, so they cannot
// hold borrowed references, and yet if the emit · window build · spawn logic is written twice the human path and the LLM path diverge again (the very 2 code paths
// that ADR-0081 decision 3 meant to eliminate). So the owning type lends its own to the implementation above and passes it through — the logic is one copy.

pub(crate) struct OwnedEvents {
    pub(crate) app: AppHandle,
}

impl LayoutEvents for OwnedEvents {
    fn layout_updated(&self, snapshot: &ViewSnapshot) {
        TauriEvents { app: &self.app }.layout_updated(snapshot);
    }

    fn window_tabs_updated(&self, tabs: &WindowTabsPayload) {
        TauriEvents { app: &self.app }.window_tabs_updated(tabs);
    }
}

// Opened inside the crate so that shell unit tests can stand up the bus path (table → apply service → this adapter → real socket) without a window.
pub(crate) struct OwnedSubs {
    pub(crate) router: Arc<OutputRouter>,
    pub(crate) client: Arc<DaemonClient>,
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

// The subscription source of the restore coordinator — each time it is used it finds the router · daemon client from the `AppHandle` (same as window-destroy cleanup
//   finding them on every event). The coordinator port is plugged in once at shell setup, so finding and holding them then would miss a client registered later.
pub(crate) struct AppSubscriptions {
    pub(crate) app: AppHandle,
}

impl crate::state::restore::SubscriptionSource for AppSubscriptions {
    fn current(&self) -> Option<Arc<dyn SubscriptionSync>> {
        let router = self.app.try_state::<Arc<OutputRouter>>()?;
        let client = self.app.try_state::<Arc<DaemonClient>>()?;
        Some(Arc::new(OwnedSubs {
            router: router.inner().clone(),
            client: client.inner().clone(),
        }))
    }
}

struct OwnedWindowHost {
    app: AppHandle,
}

impl WindowHost for OwnedWindowHost {
    fn open(&self, label: &str) -> Result<(), String> {
        TauriWindowHost { app: &self.app }.open(label)
    }

    fn open_at(&self, label: &str, at: &WindowPlace, activate: bool) -> Result<(), String> {
        TauriWindowHost { app: &self.app }.open_at(label, at, activate)
    }

    fn close(&self, label: &str) {
        TauriWindowHost { app: &self.app }.close(label);
    }

    fn is_open(&self, label: &str) -> bool {
        TauriWindowHost { app: &self.app }.is_open(label)
    }

    fn move_window(&self, label: &str, at: &WindowPlace) -> Result<(), MoveFailed> {
        TauriWindowHost { app: &self.app }.move_window(label, at)
    }

    fn record_placement(&self, label: &str) {
        TauriWindowHost { app: &self.app }.record_placement(label);
    }

    // ADR-0299
    fn claim_spare(&self) -> Option<SpareClaim> {
        TauriWindowHost { app: &self.app }.claim_spare()
    }

    fn committed(&self, claim: &SpareClaim) {
        TauriWindowHost { app: &self.app }.committed(claim);
    }

    fn release_spare(&self, claim: &SpareClaim) {
        TauriWindowHost { app: &self.app }.release_spare(claim);
    }
}

/// The bus table's spawner (`agent.spawnInto`) — its answer carries the notices to the caller, so it asks for delivery.
pub(crate) struct OwnedSpawner {
    pub(crate) client: Arc<DaemonClient>,
}

impl AgentSpawner for OwnedSpawner {
    fn spawn_by_cwd<'a>(
        &'a self,
        cwd: String,
        backend: Option<AgentBackendKind>,
    ) -> Pin<Box<dyn Future<Output = Result<SpawnedAgent, SpawnRefusal>> + Send + 'a>> {
        Box::pin(async move {
            // ADR-0348 (M13)
            DaemonSpawner {
                client: &self.client,
                deliver_notices: true,
            }
            .spawn_by_cwd(cwd, backend)
            .await
        })
    }
}

/// The bundle of ports the layout command table holds — the assembly part (`lib.rs` setup) makes it once.
///
/// ★It receives **the same state · the same router · the same issuer** as the human click path★: if a different instance is given, a window made by the LLM
/// is not in the list the human sees and labels collide (the concrete form of there being a single ADR-0035 layout authority). The settings service and
/// the effective theme · restore coordinator are the same — they take the very instance managed in the builder (otherwise the two paths see different values · different push
/// orders · different in-progress markers).
#[allow(clippy::too_many_arguments)]
pub fn command_ports(
    app: AppHandle,
    state: LayoutState,
    router: Arc<OutputRouter>,
    labels: Arc<PopupCounter>,
    client: Arc<DaemonClient>,
    settings: Arc<SettingsService>,
    themes: ThemeControl,
    restore: Arc<RestoreCoordinator>,
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
        control: Arc::new(TauriWindowControl::new(app.clone())),
        // The four below are not layout ports — they are carried here together because there is one table (`layout::commands` header).
        themes: themes.clone(),
        settings,
        settings_events: Arc::new(TauriSettingsEvents::new(app, themes)),
        restore,
    }
}

// ── tab command (per window — ADR-0057) ─────────────────────────────────────────────

// Add and activate a new empty-slot tab in window `window`. Returns the new View id. (Tab bar `+`.)
#[tauri::command]
pub fn create_tab(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    window: String,
    name: Option<String>,
) -> Result<Uuid, String> {
    let window = resolve_window(&state, &window)?;
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

// Create an empty new window (1 empty tab) + build the webview (D-6). On success the new window's label · persistent id · number ([`NewWindow`]).
// ★async fn required★: avoids a WebviewWindowBuilder deadlock (build outside the lock).
#[tauri::command]
pub async fn create_window(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    counter: State<'_, Arc<PopupCounter>>,
    client: State<'_, Arc<DaemonClient>>,
) -> Result<NewWindow, String> {
    apply::create_window(
        &state,
        &RouterSubs {
            router: &router,
            client: &client,
        },
        &TauriEvents { app: &app },
        &TauriWindowHost { app: &app },
        &**counter,
    )
}

// Replace the active tab of window `window` with `view` (that window only, other windows unchanged).
#[tauri::command]
pub fn switch_tab(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    window: String,
    view: Uuid,
) -> Result<(), String> {
    let window = resolve_window(&state, &window)?;
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

// Only for restoring the original selection after a tab drag (web → shell · TRD S21-slot-docking §4-6) — not a bus command (the LLM uses `tab.switch` · §3-6).
#[tauri::command]
pub fn switch_tab_if(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    window: String,
    view: Uuid,
    expect: Uuid,
) -> Result<bool, String> {
    let window = resolve_window(&state, &window)?;
    apply::switch_tab_if(
        &state,
        &RouterSubs {
            router: &router,
            client: &client,
        },
        &TauriEvents { app: &app },
        &window,
        view,
        expect,
    )
}

// Close tab `view` of window `window` (§5-2 state machine).
#[tauri::command]
pub async fn close_tab(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    window: String,
    view: Uuid,
) -> Result<(), String> {
    let window = resolve_window(&state, &window)?;
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

// Returns the new slot id.
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

// JS: `invoke('set_split_ratio', { viewId, splitId, ratio })` — ratio = the share of the a-side (left/top) area. The outcome · meaning of the returned
// version is in the `SplitRatioApplied` docs (carried in the bindings).
// ADR-0227
#[tauri::command]
pub fn set_split_ratio(
    app: AppHandle,
    state: State<'_, LayoutState>,
    view_id: Uuid,
    split_id: Uuid,
    ratio: f64,
) -> Result<SplitRatioApplied, String> {
    apply::set_split_ratio(&state, &TauriEvents { app: &app }, view_id, split_id, ratio)
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

// It does not take router/client State — focus movement does not change output routing (ADR-0066, the reason is in the apply service).
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

/// Write the display fields of a usage slot field by field — only the fields given change, and a `None` field keeps that slot's current value. A slot that was not usage
/// becomes usage and its `None` fields are `true` (the same apply function as Usage in bus `layout.setSlotContent`).
///
/// ★Do not substitute by sending a whole content through `set_slot_content`★ — the content the webview holds is stale until the broadcast arrives,
/// so it would revert other fields to old values (a Claude off right after Codex on turns Codex back off). Reading and writing being in one lock is
/// the apply service's responsibility. A nonexistent view·slot is `Err`.
// ADR-0258
#[tauri::command]
pub fn set_usage_slot(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    view_id: Uuid,
    slot_id: Uuid,
    show_claude: Option<bool>,
    show_codex: Option<bool>,
) -> Result<(), String> {
    apply::set_usage_slot(
        &state,
        &RouterSubs {
            router: &router,
            client: &client,
        },
        &TauriEvents { app: &app },
        view_id,
        slot_id,
        show_claude,
        show_codex,
    )
}

// ── composite command: spawn_into (D-7 placement-directed spawn) ──────────────────────────────

// Spawn an agent on the daemon and assign that agent to a tab slot of `window`. On success returns the new AgentId (String).
// Order · slot policy · failure visibility are owned by the apply service.
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
    let window = resolve_window(&state, &window)?;
    apply::spawn_into(
        &state,
        &RouterSubs {
            router: &router,
            client: &client,
        },
        &TauriEvents { app: &app },
        // ADR-0348 (M13): the webview does not show `Spawned.notices` yet (step 11) — do not let the daemon record them.
        &DaemonSpawner {
            client: &client,
            deliver_notices: false,
        },
        &window,
        tab,
        slot,
        backend,
        cwd,
    )
    .await
    .map_err(|refusal| refusal.message)
}

// ── measurement reports (webview → shell) ─────────────────────────────────────────────────────
//
// Neither is a bus command — on the bus the LLM could forge the canvas · metrics. They neither raise nor announce the version.
// ADR-0227
//
// For a window of the spare window pool (outside the model), both are recorded in the pool instead of the model (TRD S21-slot-docking §3-9 「풀」) — the model rejects an unknown
//   label, and if not recorded there would be no measurement to move over when it is taken in.

/// The window webview tells the size of its own tab content area (integer CSS px) — `invoke('report_window_canvas', { w, h })`.
///
/// ★It does not take the window label as an argument★ — one wrongly written label would overwrite another window's canvas. The [`Window`] that
/// Tauri injects is the sole authority for that value (the same rule as `report_view_commands`).
/// If `w` or `h` is 0 it is ignored and the previous value is kept (`Ok`). If the window is in neither the tab model nor the spare window pool, `Err`.
#[tauri::command]
pub fn report_window_canvas(
    window: Window,
    state: State<'_, LayoutState>,
    spare: State<'_, Arc<SpareState>>,
    w: u32,
    h: u32,
) -> Result<(), String> {
    spare.report_canvas(&state, window.label(), w, h)
}

/// The window webview tells the basic metrics of the area frame — `invoke('report_ui_metrics', { metrics })`. The contract (units · range) is
/// canonical in the [`UiMetrics`] docs. If out of range it is `Err` and the previous value is kept. The window label is taken, like
/// `report_window_canvas`, from the [`Window`] that Tauri injects.
#[tauri::command]
pub fn report_ui_metrics(
    window: Window,
    state: State<'_, LayoutState>,
    spare: State<'_, Arc<SpareState>>,
    metrics: UiMetrics,
) -> Result<(), String> {
    spare.report_metrics(&state, window.label(), metrics)
}

// ── read-only queries ───────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_view(state: State<'_, LayoutState>, view_id: Uuid) -> Result<ViewSnapshot, String> {
    apply::get_view(&state, view_id)
}

// Query window `window`'s tab list + active + version (= isomorphic to the window:tabs-updated payload).
#[tauri::command]
pub fn list_tabs(
    state: State<'_, LayoutState>,
    window: String,
) -> Result<WindowTabsPayload, String> {
    apply::list_tabs(&state, &resolve_window(&state, &window)?)
}

// Query the window label list (for boot · diagnostics).
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
    let window = window
        .map(|window| resolve_window(&state, &window))
        .transpose()?;
    apply::resolve_spatial(&state, &token, window.as_deref(), view_id)
}

#[cfg(test)]
mod tests {
    // The production part of this file, whitespace removed — the test module (which spells both values) cut off.
    fn squashed_item(head: &str) -> String {
        let source = include_str!("layout.rs");
        let production = &source[..source.find("#[cfg(test)]").expect("the test module")];
        let start = production
            .find(head)
            .unwrap_or_else(|| panic!("{head} not found"));
        let item = &production[start..];
        let end = item.find("\n}").expect("the item ends at column 0");
        item[..end].split_whitespace().collect()
    }

    /// ★Only the bus spawner asks the daemon to record the notices as shown★ — the Tauri `spawn_into` answers the
    /// webview, which does not show `Spawned.notices` yet (TRD S25 step 20 §7-1 — step 11). Flip it and the set is
    /// recorded as shown with nobody having seen it.
    // ADR-0348 (M13)
    #[test]
    fn only_the_bus_spawner_asks_for_notice_delivery() {
        let tauri = squashed_item("pub async fn spawn_into(");
        assert!(tauri.contains("deliver_notices:false"), "{tauri}");
        assert!(!tauri.contains("deliver_notices:true"), "{tauri}");

        let bus = squashed_item("impl AgentSpawner for OwnedSpawner");
        assert!(bus.contains("deliver_notices:true"), "{bus}");
        assert!(!bus.contains("deliver_notices:false"), "{bus}");
    }
}
