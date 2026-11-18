//! Integration test of the shell command table (`layout::commands`) + the inbound receiver (`daemon_client::inbound`) — zero daemon, window or socket (ADR-0012).
//! Lives in `tests/` because it needs a harness that cuts the socket and `AppHandle` with ports; pure unit assertions go beside the module (`--test lib_unit`, ADR-0174).
// hangul-ok-begin
//! Run: `cargo test -p engram-dashboard --test layout_commands`; it starts no child process, so no `-- --test-threads=4` (`.claude/skill-bindings/qa.md` 「모듈별 명령」).
// hangul-ok-end
//! Split from `layout_apply.rs`: that one measures the apply service's lock position; this one measures the envelope's path to it (name delivery, argument screening, reply correlation, running off the connection task). The fake ports here have no lock probe.
//! Two harnesses show the connection task is not blocked: [`Queued`] holds the task, proving `on_command` returned before the application happened; `RuntimeSpawner` + a real runtime proves the caller task keeps running while `agent.spawnInto` waits for the daemon (inline would self-deadlock — ADR-0081).
//! What real code does not cover is listed at the section 「byte path walked by the connection drive」.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tokio::sync::{mpsc, oneshot, Semaphore};

use engram_dashboard_command::{
    blocking_handler, duplicate_command_names, lint_spec, spec_of, CommandEnvelope, CommandError,
    CommandFuture, CommandHandler, CommandReply, CommandTable, ErrorCode, InboundCommands,
    OwnerToken, ReplySink, RequestId, Roster, TableError,
};
use engram_dashboard_protocol::{AgentCommand, AgentEvent};
use engram_dashboard_transport::testing::{
    settle_until, ImmediateHandshake, ManualClock, MemoryEndpoint, MemoryNetwork,
};
use engram_dashboard_transport::{Address, Frame, Peer, PeerId, PeerState, Policy, Registry};

use engram_dashboard_lib::commands::popout::{popup_label, PopupCounter};
use engram_dashboard_lib::daemon_client::inbound::{
    registration_command, BoxedTask, InboundReceiver, InboundSlot, RuntimeSpawner, TaskSpawner,
    ViewCommandPort,
};
use engram_dashboard_lib::daemon_client::link::route::send_command_outcome;
use engram_dashboard_lib::daemon_client::link::wire::ShellWire;
use engram_dashboard_lib::daemon_client::protocol_state::{
    command_request_id, decls_from_wire, envelope_from_wire, event_reply_request_id,
};
use engram_dashboard_lib::layout::apply;
use engram_dashboard_lib::layout::commands::{
    make_table, LayoutGetArgs, LayoutPorts, RestoreAnswerArgs, RestoreStatusArgs, SettingsGetArgs,
    SettingsResetArgs, SettingsSchemaArgs, SettingsSetArgs, SlotPopoutArgs, SplitListArgs,
    SplitSetRatioArgs, TabDetachArgs, TabDockArgs, TabMoveArgs, WindowCloseArgs,
    WindowGetThemeArgs, WindowHideAllArgs, WindowListArgs, WindowMaximizeArgs, WindowMinimizeArgs,
    WindowRaiseArgs, WindowSetThemeArgs, WindowShowAllArgs, WindowUnmaximizeArgs, CATALOG_VERSION,
    COMMAND_SPECS,
};
use engram_dashboard_lib::layout::geometry::Insets;
use engram_dashboard_lib::layout::tab_name::effective_tab_name;
use engram_dashboard_lib::layout::window_ctl::{
    Activation, Applied, OsWindowState, WindowAction, WindowControl,
};
use engram_dashboard_lib::layout::{
    tree, AgentSpawner, LayoutEvents, LayoutState, MoveFailed, SlotContent, SpareClaim, SplitDir,
    SplitRatioApplied, SplitRatioOutcome, SubscriptionSync, UiMetrics, ViewManager, ViewSnapshot,
    WindowBounds, WindowHost, WindowPlace, WindowTabsPayload, MAIN_WINDOW_LABEL,
};
use engram_dashboard_lib::settings::{
    SettingItem, SettingsEvents, SettingsService, SettingsSnapshot, THEME_DEFAULT,
};
use engram_dashboard_lib::state::boot_plugin::StateSession;
use engram_dashboard_lib::state::convert::to_persisted;
use engram_dashboard_lib::state::placement::{Landing, MonitorArea};
use engram_dashboard_lib::state::restore::{
    CrashCopy, CrashCopyStatus, RestoreCoordinator, RestorePorts, RestoreService, RestoreWindows,
    StateFileStatus, SubscriptionSource,
};
use engram_dashboard_lib::state::schema::{StateFile, STATE_VERSION};
use engram_dashboard_lib::theme::{
    global_theme, EffectiveThemes, ThemeControl, ThemeWindows, UiSettingsPayload, UiTheme,
    DEFAULT_THEME,
};
use engram_dashboard_lib::ui_text::Lang;
use engram_dashboard_lib::view_commands::{
    reserved_names, ViewArgSchema, ViewCommandBridge, ViewCommandDecl, ViewCommandHelp,
    ViewCommandRequest, ViewDispatch, ViewEffect, VIEW_HOP_MARGIN, VIEW_REPLY_DEADLINE,
};

// ── Fake ports ────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Events;

impl LayoutEvents for Events {
    fn layout_updated(&self, _snapshot: &ViewSnapshot) {}
    fn window_tabs_updated(&self, _tabs: &WindowTabsPayload) {}
}

/// The notification port the table holds — runs `hook` once before the next notification, imitating a write that slips in between the outcome's lock and the command answer.
#[derive(Default)]
struct HookedEvents {
    hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl HookedEvents {
    fn fire(&self) {
        let hook = self.hook.lock().unwrap().take();
        if let Some(hook) = hook {
            hook();
        }
    }
}

impl LayoutEvents for HookedEvents {
    fn layout_updated(&self, _snapshot: &ViewSnapshot) {
        self.fire();
    }

    fn window_tabs_updated(&self, _tabs: &WindowTabsPayload) {
        self.fire();
    }
}

#[derive(Default)]
struct Subs;

impl SubscriptionSync for Subs {
    fn resync(&self, _mgr: &ViewManager) {}
}

struct Windows {
    state: LayoutState,
    opened: Mutex<Vec<String>>,
    closed: Mutex<Vec<String>>,
    /// Hidden prewarmed window to hand over once (TRD S21-slot-docking §8-1 ⑬).
    spare: Mutex<Option<String>>,
    /// Order of the three prewarmed-window calls: `"claim:<label>"` · `"committed:<label>"` · `"release:<label>"`.
    spare_calls: Mutex<Vec<String>>,
    placed: Mutex<Vec<(String, WindowPlace, bool)>>,
    /// Runs once before the next `open_at` — a write slipping between the two locks of new-window docking.
    on_open: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl Windows {
    fn new(state: LayoutState) -> Self {
        Windows {
            state,
            opened: Mutex::new(Vec::new()),
            closed: Mutex::new(Vec::new()),
            spare: Mutex::new(None),
            spare_calls: Mutex::new(Vec::new()),
            placed: Mutex::new(Vec::new()),
            on_open: Mutex::new(None),
        }
    }
}

impl WindowHost for Windows {
    // Must come before entering the model: the bus `window.create` · `slot.popout` also open the window first (TRD S21-slot-docking §3-9 · ADR-0299). It takes no lock — the lock position is measured by `layout_apply.rs`, and waiting here would turn a defect into a hang.
    fn open(&self, label: &str) -> Result<(), String> {
        if let Ok(mgr) = self.state.0.try_lock() {
            assert!(
                !mgr.windows.contains_key(label),
                "open({label}) must be called before the window enters the model"
            );
        }
        self.opened.lock().unwrap().push(label.to_string());
        Ok(())
    }

    fn open_at(&self, label: &str, at: &WindowPlace, activate: bool) -> Result<(), String> {
        let meddle = self.on_open.lock().unwrap().take();
        if let Some(meddle) = meddle {
            meddle();
        }
        self.placed
            .lock()
            .unwrap()
            .push((label.to_string(), *at, activate));
        self.open(label)
    }

    fn close(&self, label: &str) {
        self.closed.lock().unwrap().push(label.to_string());
    }

    // True for main: a static config window this fake never opened but that is always up in the real app.
    fn is_open(&self, label: &str) -> bool {
        label == MAIN_WINDOW_LABEL
            || (self.opened.lock().unwrap().iter().any(|l| l == label)
                && !self.closed.lock().unwrap().iter().any(|l| l == label))
    }

    fn move_window(&self, _label: &str, _at: &WindowPlace) -> Result<(), MoveFailed> {
        Ok(())
    }

    fn claim_spare(&self) -> Option<SpareClaim> {
        let label = self.spare.lock().unwrap().take()?;
        self.spare_calls
            .lock()
            .unwrap()
            .push(format!("claim:{label}"));
        Some(SpareClaim {
            label,
            generation: 1,
        })
    }

    fn committed(&self, claim: &SpareClaim) {
        self.spare_calls
            .lock()
            .unwrap()
            .push(format!("committed:{}", claim.label));
    }

    fn release_spare(&self, claim: &SpareClaim) {
        self.spare_calls
            .lock()
            .unwrap()
            .push(format!("release:{}", claim.label));
    }
}

/// Window state port stand-in — records calls (`"apply:<label>:<action>"` · `"apply_unmodeled:<label>:<action>"` · `"show_all:<branch>"` · `"hide_all"` · `"state:<label>"`) and the calling thread. A label not in `states` has no OS window; `unmodeled` labels answer as windows outside the model.
#[derive(Default)]
struct Control {
    calls: Mutex<Vec<String>>,
    threads: Mutex<Vec<std::thread::ThreadId>>,
    answer: Mutex<Option<Result<Applied, String>>>,
    focused: Mutex<bool>,
    states: Mutex<BTreeMap<String, OsWindowState>>,
    unmodeled: Mutex<BTreeSet<String>>,
}

impl Control {
    fn note(&self, call: String) {
        self.calls.lock().unwrap().push(call);
        self.threads
            .lock()
            .unwrap()
            .push(std::thread::current().id());
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn answer_with(&self, answer: Result<Applied, String>) {
        *self.answer.lock().unwrap() = Some(answer);
    }
}

impl WindowControl for Control {
    fn apply(&self, label: &str, action: WindowAction) -> Result<Applied, String> {
        self.note(format!("apply:{label}:{action:?}"));
        self.answer
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(Ok(Applied::default()))
    }

    fn show_all(&self, how: Activation) -> bool {
        self.note(format!("show_all:{how:?}"));
        *self.focused.lock().unwrap()
    }

    fn hide_all(&self) {
        self.note("hide_all".to_string());
    }

    fn state(&self, label: &str) -> Option<OsWindowState> {
        self.note(format!("state:{label}"));
        self.states.lock().unwrap().get(label).copied()
    }

    fn is_unmodeled(&self, label: &str) -> bool {
        self.unmodeled.lock().unwrap().contains(label)
    }

    fn apply_unmodeled(&self, label: &str, action: WindowAction) -> Result<Applied, String> {
        self.note(format!("apply_unmodeled:{label}:{action:?}"));
        self.answer
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(Ok(Applied::default()))
    }
}

type SpawnRequest = (
    String,
    Option<engram_dashboard_protocol::AgentBackendKind>,
    oneshot::Sender<Result<String, String>>,
);

/// Finishes only when another task supplies the answer, like a daemon round trip — otherwise "does the application hold the caller task" could not be measured.
struct DaemonSpawner {
    requests: mpsc::UnboundedSender<SpawnRequest>,
}

impl AgentSpawner for DaemonSpawner {
    fn spawn_by_cwd<'a>(
        &'a self,
        cwd: String,
        backend: Option<engram_dashboard_protocol::AgentBackendKind>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>>
    {
        Box::pin(async move {
            let (answer, wait) = oneshot::channel();
            self.requests
                .send((cwd, backend, answer))
                .map_err(|_| "the test spawn channel is closed".to_string())?;
            wait.await
                .map_err(|_| "there is no test spawn response".to_string())?
        })
    }
}

/// Test agent id in the post-daemon shape: `#[agent_ref]` fields take ids only (ADR-0308); display names (`alpha`, `beta`) live only in the shell name cache (`World::show` · `World::name`).
const ALPHA: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const BETA: &str = "bbbbbbbb-0000-4000-8000-000000000002";

fn agent_id(id: &str) -> uuid::Uuid {
    uuid::Uuid::parse_str(id).expect("UUID")
}

/// Window-side stand-in of the theme push — fixed window list, records what was sent; `refuse` = windows that cannot receive.
struct RecordingWindows {
    labels: Vec<String>,
    refuse: Mutex<Option<String>>,
    sent: Mutex<Vec<(String, String)>>,
}

impl RecordingWindows {
    fn new(labels: &[&str]) -> Self {
        RecordingWindows {
            labels: labels.iter().map(|label| label.to_string()).collect(),
            refuse: Mutex::new(None),
            sent: Mutex::new(Vec::new()),
        }
    }

    fn take(&self) -> Vec<(String, String)> {
        std::mem::take(&mut *self.sent.lock().unwrap())
    }

    fn refuse(&self, label: &str) {
        *self.refuse.lock().unwrap() = Some(label.to_string());
    }
}

impl ThemeWindows for RecordingWindows {
    fn labels(&self) -> Vec<String> {
        self.labels.clone()
    }

    fn send(&self, label: &str, payload: UiSettingsPayload) -> Result<(), String> {
        self.sent
            .lock()
            .unwrap()
            .push((label.to_string(), payload.theme));
        if self.refuse.lock().unwrap().as_deref() == Some(label) {
            return Err(WINDOW_GONE.to_string());
        }
        Ok(())
    }
}

const WINDOW_GONE: &str = "the window is already closed";

/// A pop-out whose webview is up but not yet in the model — the push sends it the global value. Outside the issuer's number range, so no collision with `window.create`.
const UNMODELED_POPOUT_LABEL: &str = "slot-popup-99";

fn sent(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(label, theme)| (label.to_string(), theme.to_string()))
        .collect()
}

/// Settings notification port stand-in — with no window or Tauri, leaves only 「what was announced when」.
#[derive(Default)]
struct FakeSettingsEvents {
    changed: Mutex<Vec<SettingsSnapshot>>,
    theme_pushes: Mutex<usize>,
    threads: Mutex<Vec<std::thread::ThreadId>>,
}

impl SettingsEvents for FakeSettingsEvents {
    fn changed(&self, change: &SettingsSnapshot) {
        self.changed.lock().unwrap().push(change.clone());
        self.threads
            .lock()
            .unwrap()
            .push(std::thread::current().id());
    }

    fn theme_default_changed(&self) {
        *self.theme_pushes.lock().unwrap() += 1;
    }

    fn ui_language_changed(&self) {}
}

impl FakeSettingsEvents {
    fn changes(&self) -> Vec<SettingsSnapshot> {
        self.changed.lock().unwrap().clone()
    }

    fn theme_pushes(&self) -> usize {
        *self.theme_pushes.lock().unwrap()
    }

    fn threads(&self) -> Vec<std::thread::ThreadId> {
        self.threads.lock().unwrap().clone()
    }
}

/// Shell config folder for one test — created on first write, cleaned up at the end.
struct ConfigDir(std::path::PathBuf);

impl ConfigDir {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        ConfigDir(std::env::temp_dir().join(format!(
            "engram-layout-commands-settings-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        )))
    }

    fn service(&self) -> Arc<SettingsService> {
        let service = SettingsService::load_from_dir(&self.0);
        service.enable_writes();
        Arc::new(service)
    }
}

impl Drop for ConfigDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

// ── Task spawner harness ────────────────────────────────────────────────────

#[derive(Default)]
struct Queued {
    tasks: Mutex<Vec<BoxedTask>>,
}

impl TaskSpawner for Queued {
    fn spawn(&self, task: BoxedTask) {
        self.tasks.lock().unwrap().push(task);
    }
}

impl Queued {
    fn pending(&self) -> usize {
        self.tasks.lock().unwrap().len()
    }

    fn drain(&self) -> Vec<BoxedTask> {
        std::mem::take(&mut *self.tasks.lock().unwrap())
    }
}

// ── Briefly turning the panic hook off ───────────────────────────────────────────────────

/// Runs `body` while swallowing the panic message and restores the hook afterwards — async twin of `engram_dashboard_command::testing::with_quiet_panic_hook`. The section covers one intended panic and restores after catching the unwind; do not move the restore to `Drop` (reasons: that helper's doc).
async fn quiet_panics<T>(body: impl std::future::Future<Output = T>) -> T {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = futures_util::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(body)).await;
    std::panic::set_hook(previous);
    outcome.unwrap_or_else(|payload| {
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("<non-string panic payload>");
        eprintln!("the body panicked inside the quiet panic hook section (the original output was swallowed): {message}");
        std::panic::resume_unwind(payload)
    })
}

// ── Collecting replies ────────────────────────────────────────────────────────────────

#[derive(Default, Clone)]
struct Mailbox(Arc<Mutex<Vec<CommandReply>>>);

impl Mailbox {
    fn sink(&self, request_id: RequestId) -> ReplySink {
        let seen = Arc::clone(&self.0);
        ReplySink::new(request_id, move |reply| {
            seen.lock().unwrap().push(reply);
        })
    }

    fn deliver(&self) -> impl FnOnce(CommandReply) + Send + 'static {
        let seen = Arc::clone(&self.0);
        move |reply| {
            seen.lock().unwrap().push(reply);
        }
    }

    fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    /// Reset between two calls with one harness — [`Mailbox::only`] demands exactly one reply.
    fn clear(&self) {
        self.0.lock().unwrap().clear();
    }

    fn only(&self) -> CommandReply {
        let seen = self.0.lock().unwrap();
        assert_eq!(seen.len(), 1, "exactly one reply per request_id");
        seen[0].clone()
    }

    fn request_ids(&self) -> BTreeSet<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|reply| reply.request_id.to_string())
            .collect()
    }

    /// Yields this task until all `n` replies arrive — if they do not come, it ends as a failure (no hang).
    async fn settle(&self, n: usize) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while self.len() < n {
            assert!(
                tokio::time::Instant::now() < deadline,
                "waited for {n} replies but only {} came",
                self.len()
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

fn envelope(name: &str, args: serde_json::Value, request_id: RequestId) -> CommandEnvelope {
    CommandEnvelope {
        name: name.to_string(),
        request_id,
        // Destination token: addressed to this hop.
        owner: OwnerToken::new("shell"),
        proto_ver: CATALOG_VERSION,
        args,
    }
}

// ── Harness ─────────────────────────────────────────────────────────────────

struct World {
    state: LayoutState,
    windows: Arc<Windows>,
    control: Arc<Control>,
    /// Same handle as the table's; the window side is `theme_windows`.
    themes: ThemeControl,
    theme_windows: Arc<RecordingWindows>,
    settings: Arc<SettingsService>,
    settings_events: Arc<FakeSettingsEvents>,
    _config: ConfigDir,
    mail: Mailbox,
    spawn_requests: mpsc::UnboundedReceiver<SpawnRequest>,
    /// The restore service of the coordinator the table holds — the test imitates boot step ⑥ (`set_boot`).
    restore_service: Arc<RestoreService>,
    restore: Arc<RestoreCoordinator>,
    labels: Arc<PopupCounter>,
    events: Arc<HookedEvents>,
}

impl World {
    fn build() -> (World, LayoutPorts) {
        let state = LayoutState::new();
        let windows = Arc::new(Windows::new(state.clone()));
        let control = Arc::new(Control::default());
        let config = ConfigDir::new();
        let settings = config.service();
        let theme_windows = Arc::new(RecordingWindows::new(&[
            MAIN_WINDOW_LABEL,
            UNMODELED_POPOUT_LABEL,
        ]));
        let themes = ThemeControl::new(
            Arc::new(EffectiveThemes::new(Arc::clone(&settings), state.clone())),
            Arc::clone(&theme_windows) as Arc<dyn ThemeWindows>,
        );
        let settings_events = Arc::new(FakeSettingsEvents::default());
        let (tx, spawn_requests) = mpsc::unbounded_channel();
        // Real issuer: label monotonicity blocks re-building a closed label, so it is not faked; the restore coordinator shares it (as in production).
        let labels = Arc::new(PopupCounter::default());
        let restore_service = Arc::new(RestoreService::new());
        let restore = Arc::new(RestoreCoordinator::new(
            Arc::clone(&restore_service),
            state.clone(),
            // There is no recorder — the answer is `durable:false` at once (same as the shell under the guard).
            Arc::new(StateSession::default()),
            labels.clone(),
        ));
        let events = Arc::new(HookedEvents::default());
        let ports = LayoutPorts {
            state: state.clone(),
            subs: Arc::new(Subs),
            events: Arc::clone(&events) as Arc<dyn LayoutEvents>,
            windows: Arc::clone(&windows) as Arc<dyn WindowHost>,
            labels: labels.clone(),
            spawner: Arc::new(DaemonSpawner { requests: tx }),
            control: Arc::clone(&control) as Arc<dyn WindowControl>,
            themes: themes.clone(),
            settings: Arc::clone(&settings),
            settings_events: Arc::clone(&settings_events) as Arc<dyn SettingsEvents>,
            restore: Arc::clone(&restore),
        };
        (
            World {
                state,
                windows,
                control,
                themes,
                theme_windows,
                settings,
                settings_events,
                _config: config,
                mail: Mailbox::default(),
                spawn_requests,
                restore_service,
                restore,
                labels,
                events,
            },
            ports,
        )
    }

    fn main_tabs(&self) -> WindowTabsPayload {
        apply::list_tabs(&self.state, MAIN_WINDOW_LABEL).expect("the main window always exists")
    }

    fn slots(&self, view: uuid::Uuid) -> Vec<uuid::Uuid> {
        apply::get_view(&self.state, view)
            .expect("view")
            .slot_spatial
            .iter()
            .map(|s| s.slot_id)
            .collect()
    }

    fn empty_slot(&self, view: uuid::Uuid) -> uuid::Uuid {
        tree::first_empty_slot_id(&apply::get_view(&self.state, view).expect("view").layout)
            .expect("an empty slot")
    }

    /// One area holding an agent — returns (tab, that slot). Subscription migration is not measured here: `Subs` is a no-op, and the real `OutputRouter` measurement is `layout_apply.rs`.
    fn filled_slot(&self) -> (uuid::Uuid, uuid::Uuid) {
        let view = self.main_tabs().active;
        let slot = self.empty_slot(view);
        apply::set_slot_content(
            &self.state,
            &Subs,
            &Events,
            view,
            slot,
            SlotContent::Agent {
                agent_id: uuid::Uuid::new_v4().to_string(),
            },
        )
        .expect("content placement");
        (view, slot)
    }
}

fn queued() -> (World, Arc<Queued>, InboundReceiver) {
    let (world, ports) = World::build();
    let queue = Arc::new(Queued::default());
    let receiver = InboundReceiver::new(
        make_table(ports),
        Arc::clone(&queue) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    );
    (world, queue, receiver)
}

async fn run(queue: &Queued) {
    for task in queue.drain() {
        task.await;
    }
}

async fn call(
    receiver: &InboundReceiver,
    queue: &Queued,
    mail: &Mailbox,
    name: &str,
    args: serde_json::Value,
) -> CommandReply {
    let request_id = RequestId::new();
    receiver.on_command(envelope(name, args, request_id), mail.sink(request_id));
    run(queue).await;
    let reply = mail.only();
    assert_eq!(
        reply.request_id, request_id,
        "the reply comes with the same correlation key as the request"
    );
    reply
}

fn error_of(reply: CommandReply) -> CommandError {
    reply.outcome.expect_err("error reply")
}

// ── (A) Declarations and the table ────────────────────────────────────────────────────────────

/// All names actually plugged into the table — a golden, not a hand list: declarations growing without assembly (or the reverse) is caught here.
#[test]
fn the_table_holds_exactly_the_declared_commands() {
    let (_world, ports) = World::build();
    let table = make_table(ports);
    let names: Vec<&str> = table.specs().map(|s| s.name).collect();
    assert_eq!(
        names,
        vec![
            "agent.spawnInto",
            "layout.get",
            "layout.setSlotContent",
            "restore.answer",
            "restore.status",
            "settings.get",
            "settings.reset",
            "settings.schema",
            "settings.set",
            "slot.assignAgent",
            "slot.close",
            "slot.focus",
            "slot.popout",
            "slot.resolveSpatial",
            "slot.split",
            "split.list",
            "split.setRatio",
            "tab.close",
            "tab.create",
            "tab.detach",
            "tab.dock",
            "tab.list",
            "tab.move",
            "tab.rename",
            "tab.switch",
            "window.close",
            "window.create",
            "window.getTheme",
            "window.hideAll",
            "window.list",
            "window.maximize",
            "window.minimize",
            "window.raise",
            "window.setTheme",
            "window.showAll",
            "window.unmaximize",
        ]
    );
    assert_eq!(
        names.len(),
        COMMAND_SPECS.len(),
        "declared but not plugged in"
    );
}

/// The generation number is pinned by hand (macro contract: raise it when a declaration changes), otherwise two shells with different vocabularies report the same generation. The names-only golden above cannot catch this.
/// Generation · declaration count · the new command's `since` must move together (same shape as core `command_alphabet.rs`).
#[test]
fn the_catalog_generation_is_pinned_to_the_declaration_set() {
    // Rises with any declaration change (answer shape, vocabulary, error declarations), not only when the name count grows.
    // ADR-0219 · ADR-0227 · ADR-0276 · ADR-0225 · ADR-0291 · ADR-0308
    // TRD S21 usage-limit-slot §1-7 · TRD S21-storage §5-4 · §5-6 · §5-7 · §6-5 · §6-7 · TRD S26-window-chrome §3-2 · §4-5 · §5-3 · §6-1 · §6-6 · TRD S28 §4 · §7
    assert_eq!(CATALOG_VERSION, 21);
    assert_eq!(COMMAND_SPECS.len(), 36);
    for since in [
        TabMoveArgs::SPEC.since,
        TabDockArgs::SPEC.since,
        TabDetachArgs::SPEC.since,
        LayoutGetArgs::SPEC.since,
    ] {
        assert_eq!(since, 21);
    }
    for since in [
        WindowMinimizeArgs::SPEC.since,
        WindowMaximizeArgs::SPEC.since,
        WindowUnmaximizeArgs::SPEC.since,
        WindowRaiseArgs::SPEC.since,
        WindowHideAllArgs::SPEC.since,
        WindowShowAllArgs::SPEC.since,
    ] {
        assert_eq!(since, 20);
    }
    assert_eq!(
        WindowCloseArgs::SPEC.since,
        1,
        "a command whose shape changed advertises the generation it first entered, as is"
    );
    assert_eq!(
        SlotPopoutArgs::SPEC.since,
        2,
        "a command that entered at generation 2 must not advertise that it existed from 1"
    );
    assert_eq!(SplitSetRatioArgs::SPEC.since, 8);
    assert_eq!(SplitListArgs::SPEC.since, 8);
    for since in [
        SettingsGetArgs::SPEC.since,
        SettingsSetArgs::SPEC.since,
        SettingsResetArgs::SPEC.since,
        SettingsSchemaArgs::SPEC.since,
    ] {
        assert_eq!(since, 10);
    }
    assert_eq!(RestoreStatusArgs::SPEC.since, 11);
    assert_eq!(RestoreAnswerArgs::SPEC.since, 11);
    assert_eq!(WindowGetThemeArgs::SPEC.since, 14);
    assert_eq!(WindowSetThemeArgs::SPEC.since, 14);
}

/// Per-window theme became state: the command that re-reads the file left both the table and the linker collection (TRD S21-storage §5-7).
#[test]
fn ui_refresh_is_gone_from_the_catalog() {
    assert!(spec_of("ui.refresh").is_none());
}

/// If one registration `help` exceeds the roster limit the daemon rejects the whole packet (`Roster::register`), making every shell command `UNKNOWN_COMMAND`. Summaries (declaration docs) grow here while the limit lives only in the daemon, so the carried string (`decls`) is measured; agent reference field count and name length trigger the same rejection and are measured with it.
#[test]
fn every_registered_declaration_fits_the_roster_help_cap() {
    let (_world, ports) = World::build();
    for decl in make_table(ports).decls() {
        assert!(
            decl.help.len() <= Roster::MAX_HELP_BYTES,
            "{}: help {} bytes > limit {}",
            decl.name,
            decl.help.len(),
            Roster::MAX_HELP_BYTES
        );
        assert!(
            decl.agent_args.len() <= Roster::MAX_AGENT_ARGS,
            "{}: {} agent reference fields > limit {}",
            decl.name,
            decl.agent_args.len(),
            Roster::MAX_AGENT_ARGS
        );
        for arg in &decl.agent_args {
            assert!(
                arg.len() <= Roster::MAX_NAME_BYTES,
                "{}: agent reference field name {arg} is {} bytes > limit {}",
                decl.name,
                arg.len(),
                Roster::MAX_NAME_BYTES
            );
        }
    }
}

#[test]
fn every_declaration_carries_a_usable_json_shape() {
    for spec in COMMAND_SPECS {
        lint_spec(spec).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            !spec.summary.trim().is_empty(),
            "{}: the summary is empty",
            spec.name
        );
        for (label, text) in [("args", spec.args_schema), ("ok", spec.ok_schema)] {
            let shape: serde_json::Value = serde_json::from_str(text).expect("the schema is JSON");
            assert_eq!(
                shape["type"], "object",
                "{}: {label} must be an object",
                spec.name
            );
        }
        assert_eq!(
            spec_of(spec.name).map(|s| s.name),
            Some(spec.name),
            "{}: not seen in the linker collection",
            spec.name
        );
    }
}

/// Measured range = declarations of crates linked into this binary (`duplicate_command_names` sees only linked objects). The shell adds `agent.spawnInto` to the `agent.*` family; agent declarations are visible here only via the daemon dev dependency (`spec_of("agent.new")` is `Some`) — if that path is cut, this test silently stops seeing the overlap.
/// The wall against shell · daemon overlap is the daemon's registration refusal (ADR-0270 decision 2).
#[test]
fn no_two_declarations_claim_the_same_name() {
    assert_eq!(duplicate_command_names(), Vec::<&str>::new());
}

// ── (B) Delivery ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_write_command_reaches_the_apply_service() {
    let (world, queue, receiver) = queued();
    let before = world.main_tabs().tabs.len();

    let reply = call(
        &receiver,
        &queue,
        &world.mail,
        "tab.create",
        json!({ "window": MAIN_WINDOW_LABEL, "name": "new tab" }),
    )
    .await;

    let ok = reply.outcome.expect("creation succeeds");
    let after = world.main_tabs();
    assert_eq!(after.tabs.len(), before + 1);
    assert_eq!(
        ok["view_id"].as_str(),
        Some(after.active.to_string().as_str()),
        "the newly made tab is active and its id is carried in the reply"
    );
}

#[tokio::test]
async fn a_read_command_answers_from_the_same_authority() {
    let (world, queue, receiver) = queued();
    let created = call(&receiver, &queue, &world.mail, "window.create", json!({}))
        .await
        .outcome
        .expect("window creation");
    let label = created["window"].as_str().expect("label").to_string();
    assert_eq!(
        world.windows.opened.lock().unwrap().as_slice(),
        &[label.clone()]
    );

    let mail = Mailbox::default();
    let listed = call(&receiver, &queue, &mail, "window.list", json!({}))
        .await
        .outcome
        .expect("window list");
    let windows: Vec<&str> = listed["windows"]
        .as_array()
        .expect("array")
        .iter()
        .map(|v| v.as_str().expect("label"))
        .collect();
    assert!(
        windows.contains(&label.as_str()),
        "the window just made shows in the query: {windows:?}"
    );
}

/// `tab.list` rows carry whether the name was set: an unnamed tab is auto-named, and `tab.rename` with that same text makes it a set name (TRD S26-window-chrome §3-1 · §3-2).
#[tokio::test]
async fn tab_list_says_whether_each_tab_name_was_set() {
    let (world, queue, receiver) = queued();
    let auto = call(
        &receiver,
        &queue,
        &world.mail,
        "tab.create",
        json!({ "window": MAIN_WINDOW_LABEL }),
    )
    .await
    .outcome
    .expect("auto-named tab");
    let named = call(
        &receiver,
        &queue,
        &Mailbox::default(),
        "tab.create",
        json!({ "window": MAIN_WINDOW_LABEL, "name": "work" }),
    )
    .await
    .outcome
    .expect("tab with a set name");
    async fn list(receiver: &InboundReceiver, queue: &Queued) -> serde_json::Value {
        call(
            receiver,
            queue,
            &Mailbox::default(),
            "tab.list",
            json!({ "window": MAIN_WINDOW_LABEL }),
        )
        .await
        .outcome
        .expect("tab list")
    }
    let row = |listed: &serde_json::Value, id: &serde_json::Value| {
        listed["tabs"]
            .as_array()
            .expect("array")
            .iter()
            .find(|t| t["id"] == *id)
            .cloned()
            .expect("that tab")
    };

    let listed = list(&receiver, &queue).await;
    assert_eq!(
        row(&listed, &auto["view_id"]),
        json!({ "id": auto["view_id"], "name": "Empty slot", "named": false })
    );
    assert_eq!(
        row(&listed, &named["view_id"]),
        json!({ "id": named["view_id"], "name": "work", "named": true })
    );

    call(
        &receiver,
        &queue,
        &Mailbox::default(),
        "tab.rename",
        json!({ "view_id": auto["view_id"], "name": "Empty slot" }),
    )
    .await
    .outcome
    .expect("rename with the same text");
    assert_eq!(
        row(&list(&receiver, &queue).await, &auto["view_id"]),
        json!({ "id": auto["view_id"], "name": "Empty slot", "named": true })
    );
}

/// `tab.create` · `tab.rename` trim whitespace around `name`, same as the web registry and tab bar editing.
#[tokio::test]
async fn tab_create_and_rename_trim_the_name() {
    let (world, queue, receiver) = queued();
    let created = call(
        &receiver,
        &queue,
        &world.mail,
        "tab.create",
        json!({ "window": MAIN_WINDOW_LABEL, "name": "  work \t" }),
    )
    .await
    .outcome
    .expect("tab with a set name");
    async fn name_of(
        receiver: &InboundReceiver,
        queue: &Queued,
        id: &serde_json::Value,
    ) -> serde_json::Value {
        let listed = call(
            receiver,
            queue,
            &Mailbox::default(),
            "tab.list",
            json!({ "window": MAIN_WINDOW_LABEL }),
        )
        .await
        .outcome
        .expect("tab list");
        listed["tabs"]
            .as_array()
            .expect("array")
            .iter()
            .find(|t| t["id"] == *id)
            .expect("that tab")["name"]
            .clone()
    }
    assert_eq!(
        name_of(&receiver, &queue, &created["view_id"]).await,
        json!("work")
    );

    call(
        &receiver,
        &queue,
        &Mailbox::default(),
        "tab.rename",
        json!({ "view_id": created["view_id"], "name": " new name  " }),
    )
    .await
    .outcome
    .expect("rename");
    assert_eq!(
        name_of(&receiver, &queue, &created["view_id"]).await,
        json!("new name")
    );
}

#[tokio::test]
async fn an_unknown_command_name_is_answered_not_dropped() {
    let (world, queue, receiver) = queued();
    let err = error_of(call(&receiver, &queue, &world.mail, "tab.teleport", json!({})).await);
    assert_eq!(err.code(), ErrorCode::UnknownCommand);
    assert!(err.message().contains("tab.teleport"));
}

/// A malformed id is rejected **before** application — receiving the same answer as for a nonexistent id would make the caller dig through a healthy list.
#[tokio::test]
async fn a_malformed_id_is_an_invalid_argument() {
    let (world, queue, receiver) = queued();
    let err = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "tab.switch",
            json!({ "window": MAIN_WINDOW_LABEL, "view_id": "not-a-uuid" }),
        )
        .await,
    );
    assert_eq!(err.code(), ErrorCode::InvalidArgument);
    assert!(err.message().contains("view_id"), "{}", err.message());
}

/// What the apply service refused goes out as `CONFLICT` + **that reason text** (one code · the reason is the text).
#[tokio::test]
async fn an_unapplicable_request_keeps_the_services_reason() {
    let (world, queue, receiver) = queued();
    let before = world.main_tabs();
    let err = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "tab.close",
            json!({ "window": MAIN_WINDOW_LABEL, "view_id": uuid::Uuid::new_v4().to_string() }),
        )
        .await,
    );
    assert_eq!(err.code(), ErrorCode::Conflict);
    assert!(
        !err.message().is_empty(),
        "if the reason goes out empty it cannot be fixed"
    );
    assert_eq!(world.main_tabs(), before, "if refused, nothing changes");
    assert!(
        world.windows.closed.lock().unwrap().is_empty(),
        "if refused, the OS window is not closed either"
    );
}

// ── Window state commands (TRD S26-window-chrome §6-1 · §10-2) ──

/// A window state verb goes to the window state port with the resolved label, from the blocking pool (the real port waits for a main-thread job, §5-2).
#[tokio::test]
async fn window_state_verbs_reach_the_port_with_the_resolved_label() {
    let (world, queue, receiver) = queued();
    let created = call(&receiver, &queue, &world.mail, "window.create", json!({}))
        .await
        .outcome
        .expect("window creation");
    let popout = created["window"].as_str().expect("label").to_string();
    world.control.answer_with(Ok(Applied {
        deferred: true,
        pending: true,
        shown_all: true,
        focused: true,
    }));

    let mut answers = Vec::new();
    for name in [
        "window.minimize",
        "window.maximize",
        "window.unmaximize",
        "window.raise",
    ] {
        answers.push(
            call(
                &receiver,
                &queue,
                &Mailbox::default(),
                name,
                json!({ "window": created["window_id"] }),
            )
            .await
            .outcome
            .expect(name),
        );
    }
    assert_eq!(
        answers,
        [
            json!({}),
            json!({ "deferred": true }),
            json!({ "deferred": true }),
            json!({ "shown_all": true, "focused": true, "pending": true }),
        ]
    );
    assert_eq!(
        world.control.calls(),
        [
            format!("apply:{popout}:Minimize"),
            format!("apply:{popout}:Maximize"),
            format!("apply:{popout}:Unmaximize"),
            format!("apply:{popout}:Raise"),
        ]
    );
    let here = std::thread::current().id();
    assert!(
        world
            .control
            .threads
            .lock()
            .unwrap()
            .iter()
            .all(|thread| *thread != here),
        "the window state port is called from the blocking pool, not from the apply task's worker"
    );
}

#[tokio::test]
async fn a_window_state_refusal_is_a_conflict_with_the_ports_reason() {
    let (world, queue, receiver) = queued();
    world.control.answer_with(Err(
        "a hidden window — first window.raise or window.showAll".to_string(),
    ));
    let err = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "window.minimize",
            json!({ "window": MAIN_WINDOW_LABEL }),
        )
        .await,
    );
    assert_eq!(err.code(), ErrorCode::Conflict);
    assert!(err.message().contains("window.raise"), "{}", err.message());
}

#[tokio::test]
async fn hide_all_and_show_all_reach_the_port() {
    let (world, queue, receiver) = queued();
    *world.control.focused.lock().unwrap() = true;
    let hidden = call(&receiver, &queue, &world.mail, "window.hideAll", json!({}))
        .await
        .outcome
        .expect("hide all");
    let shown = call(
        &receiver,
        &queue,
        &Mailbox::default(),
        "window.showAll",
        json!({}),
    )
    .await
    .outcome
    .expect("show all");
    assert_eq!((hidden, shown), (json!({}), json!({ "focused": true })));
    assert_eq!(world.control.calls(), ["hide_all", "show_all:NoSteal"]);
}

/// Closing main hides everything instead of refusing; closing a pop-out closes it (TRD §5-4).
#[tokio::test]
async fn window_close_hides_everything_for_main_and_closes_a_popout() {
    let (world, queue, receiver) = queued();
    let main = call(
        &receiver,
        &queue,
        &world.mail,
        "window.close",
        json!({ "window": MAIN_WINDOW_LABEL }),
    )
    .await
    .outcome
    .expect("close main");
    assert_eq!(main, json!({ "outcome": "HiddenAll" }));
    assert_eq!(world.control.calls(), ["hide_all"]);
    assert!(
        world.windows.closed.lock().unwrap().is_empty(),
        "main is not closed"
    );

    let popout = call(
        &receiver,
        &queue,
        &Mailbox::default(),
        "window.create",
        json!({}),
    )
    .await
    .outcome
    .expect("window creation")["window"]
        .as_str()
        .expect("label")
        .to_string();
    let closed = call(
        &receiver,
        &queue,
        &Mailbox::default(),
        "window.close",
        json!({ "window": popout }),
    )
    .await
    .outcome
    .expect("pop-out close");
    assert_eq!(closed, json!({ "outcome": "Closed" }));
    assert_eq!(world.windows.closed.lock().unwrap().as_slice(), &[popout]);
}

/// Window buttons of a window outside the model (a stale pop-out of the boot failure screen whose OS window is open): minimize · maximize · restore act on it directly, close reaps it (`Closed`), raise stays `CONFLICT` (TRD S26-window-chrome §6-2). The model is unchanged.
#[tokio::test]
async fn window_buttons_of_an_unmodeled_window_reach_its_os_window() {
    let (world, queue, receiver) = queued();
    world
        .control
        .unmodeled
        .lock()
        .unwrap()
        .insert(UNMODELED_POPOUT_LABEL.to_string());
    let before = apply::list_windows(&world.state).unwrap();

    let mut answers = Vec::new();
    for name in ["window.minimize", "window.maximize", "window.unmaximize"] {
        answers.push(
            call(
                &receiver,
                &queue,
                &Mailbox::default(),
                name,
                json!({ "window": UNMODELED_POPOUT_LABEL }),
            )
            .await
            .outcome
            .expect(name),
        );
    }
    assert_eq!(
        answers,
        [
            json!({}),
            json!({ "deferred": false }),
            json!({ "deferred": false })
        ]
    );
    let err = error_of(
        call(
            &receiver,
            &queue,
            &Mailbox::default(),
            "window.raise",
            json!({ "window": UNMODELED_POPOUT_LABEL }),
        )
        .await,
    );
    assert_eq!(err.code(), ErrorCode::Conflict);
    let closed = call(
        &receiver,
        &queue,
        &world.mail,
        "window.close",
        json!({ "window": UNMODELED_POPOUT_LABEL }),
    )
    .await
    .outcome
    .expect("close a window outside the model");
    assert_eq!(closed, json!({ "outcome": "Closed" }));

    assert_eq!(
        world.control.calls(),
        [
            format!("apply_unmodeled:{UNMODELED_POPOUT_LABEL}:Minimize"),
            format!("apply_unmodeled:{UNMODELED_POPOUT_LABEL}:Maximize"),
            format!("apply_unmodeled:{UNMODELED_POPOUT_LABEL}:Unmaximize"),
        ]
    );
    assert_eq!(
        world.windows.closed.lock().unwrap().as_slice(),
        &[UNMODELED_POPOUT_LABEL.to_string()]
    );
    assert_eq!(apply::list_windows(&world.state).unwrap(), before);
}

/// `if_empty` is decided before the resolver (no CONFLICT for a window outside the model or one with tabs); a digits-only argument is `INVALID_ARGUMENT` even earlier (TRD §4-5).
#[tokio::test]
async fn window_close_if_empty_never_conflicts_but_still_refuses_a_number() {
    let (world, queue, receiver) = queued();
    let created = call(&receiver, &queue, &world.mail, "window.create", json!({}))
        .await
        .outcome
        .expect("window creation");
    for window in [
        created["window_id"].clone(),
        json!(uuid::Uuid::new_v4().to_string()),
        json!("slot-popup-1"),
    ] {
        let kept = call(
            &receiver,
            &queue,
            &Mailbox::default(),
            "window.close",
            json!({ "window": window, "if_empty": true }),
        )
        .await
        .outcome
        .expect("close if empty");
        assert_eq!(kept, json!({ "outcome": "Kept" }), "{window}");
    }
    assert!(world.windows.closed.lock().unwrap().is_empty());

    let err = error_of(
        call(
            &receiver,
            &queue,
            &Mailbox::default(),
            "window.close",
            json!({ "window": "2", "if_empty": true }),
        )
        .await,
    );
    assert_eq!(err.code(), ErrorCode::InvalidArgument);
    assert!(err.message().contains("window_id"), "{}", err.message());
}

/// `window.list` carries `windows` as is and adds `details` in number order — a row whose OS window could not be read has all states false.
#[tokio::test]
async fn window_list_keeps_windows_and_adds_details_in_number_order() {
    let (world, queue, receiver) = queued();
    let mut created = Vec::new();
    for _ in 0..2 {
        created.push(
            call(
                &receiver,
                &queue,
                &Mailbox::default(),
                "window.create",
                json!({}),
            )
            .await
            .outcome
            .expect("window creation"),
        );
    }
    let second = created[1]["window"].as_str().expect("label").to_string();
    world.control.states.lock().unwrap().insert(
        second.clone(),
        OsWindowState {
            visible: true,
            maximized: true,
            focused: true,
            ..OsWindowState::default()
        },
    );

    let listed = call(&receiver, &queue, &world.mail, "window.list", json!({}))
        .await
        .outcome
        .expect("window list");
    let row = |window: &serde_json::Value, number: u32, main: bool, visible: bool| {
        json!({
            "window_id": window["window_id"],
            "window": window["window"],
            "number": number,
            "main": main,
            "visible": visible,
            "minimized": false,
            "maximized": visible,
            "focused": visible,
        })
    };
    let main = json!({ "window_id": "main", "window": MAIN_WINDOW_LABEL });
    assert_eq!(
        listed["details"],
        json!([
            row(&main, 1, true, false),
            row(&created[0], 2, false, false),
            row(&created[1], 3, false, true),
        ])
    );
    let mut windows: Vec<&str> = listed["windows"]
        .as_array()
        .expect("array")
        .iter()
        .map(|v| v.as_str().expect("label"))
        .collect();
    windows.sort_unstable();
    let mut want = vec![
        MAIN_WINDOW_LABEL,
        created[0]["window"].as_str().unwrap(),
        second.as_str(),
    ];
    want.sort_unstable();
    assert_eq!(windows, want);
}

// ── Window argument (TRD S26-window-chrome §4-5 · §10-2) ──

fn window_arg_calls(
    window: &str,
    view: uuid::Uuid,
    slot: uuid::Uuid,
) -> Vec<(&'static str, serde_json::Value)> {
    let (view, slot) = (view.to_string(), slot.to_string());
    vec![
        ("tab.create", json!({ "window": window })),
        ("tab.switch", json!({ "window": window, "view_id": view })),
        ("tab.close", json!({ "window": window, "view_id": view })),
        ("tab.list", json!({ "window": window })),
        ("window.close", json!({ "window": window })),
        ("window.minimize", json!({ "window": window })),
        ("window.maximize", json!({ "window": window })),
        ("window.unmaximize", json!({ "window": window })),
        ("window.raise", json!({ "window": window })),
        ("window.getTheme", json!({ "window": window })),
        (
            "window.setTheme",
            json!({ "window": window, "theme": "light" }),
        ),
        (
            "slot.popout",
            json!({ "view_id": view, "slot_id": slot, "to_window": window }),
        ),
        (
            "agent.spawnInto",
            json!({ "window": window, "cwd": "C:/work" }),
        ),
        (
            "slot.resolveSpatial",
            json!({ "token": "top-left", "window": window }),
        ),
        ("tab.move", json!({ "tab": view, "to_window": window })),
        (
            "tab.dock",
            json!({ "tab": view, "beside_at": "top-left", "window": window, "side": "Left" }),
        ),
        ("layout.get", json!({ "window": window })),
        // `window` of an area verb — the window to read `at` in (TRD S28 §7).
        ("slot.focus", json!({ "at": "top-left", "window": window })),
        ("slot.close", json!({ "at": "top-left", "window": window })),
        (
            "slot.split",
            json!({ "at": "top-left", "window": window, "dir": "LeftRight" }),
        ),
        ("slot.popout", json!({ "at": "top-left", "window": window })),
        (
            "slot.assignAgent",
            json!({ "at": "top-left", "window": window, "assign": ALPHA }),
        ),
        (
            "layout.setSlotContent",
            json!({ "at": "top-left", "window": window, "content": "Empty" }),
        ),
    ]
}

/// A digits-only window argument is a window number and `INVALID_ARGUMENT` — numbers are reused, so even an existing number does not pick; the text guides to window_id. Nothing changes.
#[tokio::test]
async fn a_window_number_is_an_invalid_argument_on_every_window_verb() {
    let (mut world, queue, receiver) = queued();
    let popout = call(&receiver, &queue, &world.mail, "window.create", json!({}))
        .await
        .outcome
        .expect("window creation")["window"]
        .as_str()
        .expect("label")
        .to_string();
    assert_eq!(apply::list_tabs(&world.state, &popout).unwrap().number, 2);
    let (view, slot) = world.filled_slot();
    let before = (
        apply::list_windows(&world.state).unwrap(),
        world.main_tabs(),
        world.windows.opened.lock().unwrap().len(),
    );

    for (name, args) in window_arg_calls("2", view, slot) {
        let err = error_of(call(&receiver, &queue, &Mailbox::default(), name, args).await);
        assert_eq!(err.code(), ErrorCode::InvalidArgument, "{name}");
        assert!(
            err.message().contains("window_id") && err.message().contains("layout.get"),
            "{name}: {}",
            err.message()
        );
    }
    assert_eq!(
        (
            apply::list_windows(&world.state).unwrap(),
            world.main_tabs(),
            world.windows.opened.lock().unwrap().len(),
        ),
        before,
        "a number argument changes nothing"
    );
    assert!(world.windows.closed.lock().unwrap().is_empty());
    assert!(
        world.control.calls().is_empty(),
        "does not reach the window state port"
    );
    assert!(
        world.spawn_requests.try_recv().is_err(),
        "does not reach the spawn"
    );
    assert!(world.theme_windows.take().is_empty());
}

/// A persistent id (window_id) reaches the same window as that window's label — the `window` of the answer is the label.
#[tokio::test]
async fn a_window_id_reaches_the_same_window_as_its_label() {
    let (world, queue, receiver) = queued();
    let created = call(&receiver, &queue, &world.mail, "window.create", json!({}))
        .await
        .outcome
        .expect("window creation");
    let popout = created["window"].as_str().expect("label").to_string();
    let window_id = created["window_id"]
        .as_str()
        .expect("persistent id")
        .to_string();
    assert_ne!(window_id, popout);

    let created = call(
        &receiver,
        &queue,
        &Mailbox::default(),
        "tab.create",
        json!({ "window": window_id.to_uppercase() }),
    )
    .await
    .outcome
    .expect("make a tab by persistent id");
    let listed = call(
        &receiver,
        &queue,
        &Mailbox::default(),
        "tab.list",
        json!({ "window": window_id }),
    )
    .await
    .outcome
    .expect("tab list by persistent id");
    assert_eq!(listed["window"].as_str(), Some(popout.as_str()));
    assert_eq!(listed["active"], created["view_id"]);
    assert_eq!(listed["tabs"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        call(
            &receiver,
            &queue,
            &Mailbox::default(),
            "tab.list",
            json!({ "window": "main" }),
        )
        .await
        .outcome
        .expect("main")["window"],
        json!(MAIN_WINDOW_LABEL)
    );
}

/// A label of an earlier run (old shape without run marker, or another run marker) and the persistent id of a closed window do not point to a current window: `CONFLICT` with that argument in the text.
#[tokio::test]
async fn a_label_from_a_previous_run_or_a_closed_window_id_is_a_conflict() {
    let (world, queue, receiver) = queued();
    let popout = call(&receiver, &queue, &world.mail, "window.create", json!({}))
        .await
        .outcome
        .expect("window creation")["window"]
        .as_str()
        .expect("label")
        .to_string();
    assert_eq!(popout, popup_label(1));
    let (run, _) = popout
        .strip_prefix("slot-popup-")
        .and_then(|rest| rest.split_once('-'))
        .expect("run marker");
    let other_run = if run == "00000000" {
        "11111111"
    } else {
        "00000000"
    };
    let closed = call(
        &receiver,
        &queue,
        &Mailbox::default(),
        "window.create",
        json!({}),
    )
    .await
    .outcome
    .expect("window creation")["window"]
        .as_str()
        .expect("label")
        .to_string();
    let closed_id = apply::list_tabs(&world.state, &closed).unwrap().window_id;
    call(
        &receiver,
        &queue,
        &Mailbox::default(),
        "window.close",
        json!({ "window": closed }),
    )
    .await
    .outcome
    .expect("close");
    let (view, slot) = world.filled_slot();

    for stale in [
        "slot-popup-1".to_string(),
        format!("slot-popup-{other_run}-1"),
        closed_id,
    ] {
        for (name, args) in window_arg_calls(&stale, view, slot) {
            let err = error_of(call(&receiver, &queue, &Mailbox::default(), name, args).await);
            assert_eq!(err.code(), ErrorCode::Conflict, "{name} {stale}");
            assert!(err.message().contains(&stale), "{name}: {}", err.message());
        }
    }
    assert!(
        world.slots(view).contains(&slot),
        "the slot being moved is still in its place"
    );
}

/// A bus verb that opens a new window answers with that window's persistent id · number (same as the tab list); a `slot.popout` into an existing window answers that window's (TRD S26-window-chrome §5-3 · §10-2).
#[tokio::test]
async fn new_window_answers_carry_the_windows_id_and_number() {
    let (world, queue, receiver) = queued();
    let identity = |label: &str| {
        let tabs = apply::list_tabs(&world.state, label).expect("that window");
        (tabs.window_id, tabs.number)
    };
    let check = |answer: &serde_json::Value, want_number: u32| {
        let label = answer["window"].as_str().expect("label");
        let (window_id, number) = identity(label);
        assert_eq!(
            answer["window_id"].as_str(),
            Some(window_id.as_str()),
            "{answer}"
        );
        assert_eq!(answer["number"], json!(number), "{answer}");
        assert_eq!(number, want_number, "{answer}");
        assert!(uuid::Uuid::parse_str(&window_id).is_ok(), "{answer}");
    };

    let created = call(&receiver, &queue, &world.mail, "window.create", json!({}))
        .await
        .outcome
        .expect("window creation");
    check(&created, 2);

    let (view, slot) = world.filled_slot();
    let popped = call(
        &receiver,
        &queue,
        &Mailbox::default(),
        "slot.popout",
        json!({ "view_id": view.to_string(), "slot_id": slot.to_string() }),
    )
    .await
    .outcome
    .expect("detach to a new window");
    check(&popped, 3);

    let (view, slot) = world.filled_slot();
    let into = call(
        &receiver,
        &queue,
        &Mailbox::default(),
        "slot.popout",
        json!({
            "view_id": view.to_string(),
            "slot_id": slot.to_string(),
            "to_window": created["window_id"],
        }),
    )
    .await
    .outcome
    .expect("detach to an existing window by persistent id");
    assert_eq!(into["window"], created["window"]);
    check(&into, 2);

    *world.windows.spare.lock().unwrap() = Some(SPARE.to_string());
    let prewarmed = call(
        &receiver,
        &queue,
        &Mailbox::default(),
        "window.create",
        json!({}),
    )
    .await
    .outcome
    .expect("create with a prewarmed window");
    assert_eq!(prewarmed["window"].as_str(), Some(SPARE));
    check(&prewarmed, 4);
}

/// A verb taking a window argument states the accepted shapes (window_id · a same-run label) and the number refusal in its summary; `window.list` states the number reuse rule (TRD S26-window-chrome §4-5).
#[test]
fn every_window_verb_summary_says_what_the_window_argument_takes() {
    let any = uuid::Uuid::nil();
    for (name, _) in window_arg_calls("main", any, any) {
        let spec = spec_of(name).expect("declaration");
        for want in ["window_id", "label", "not accepted"] {
            assert!(
                spec.summary.contains(want),
                "{name}: {want} missing — {}",
                spec.summary
            );
        }
    }
    assert!(spec_of("window.list")
        .expect("declaration")
        .summary
        .contains("reuse the smallest free number"));
}

#[tokio::test]
async fn slot_content_refuses_a_contradictory_pair() {
    let (world, queue, receiver) = queued();
    let tabs = world.main_tabs();
    let view = tabs.active;
    let slot = apply::get_view(&world.state, view)
        .expect("view")
        .slot_spatial
        .first()
        .expect("one slot")
        .slot_id;

    let missing = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "layout.setSlotContent",
            json!({ "view_id": view.to_string(), "slot_id": slot.to_string(), "content": "Agent" }),
        )
        .await,
    );
    assert_eq!(missing.code(), ErrorCode::InvalidArgument);

    let mail = Mailbox::default();
    let extra = error_of(
        call(
            &receiver,
            &queue,
            &mail,
            "layout.setSlotContent",
            json!({
                "view_id": view.to_string(),
                "slot_id": slot.to_string(),
                "content": "Empty",
                "agent_id": "a1",
            }),
        )
        .await,
    );
    assert_eq!(extra.code(), ErrorCode::InvalidArgument);
}

#[tokio::test]
async fn slot_content_refuses_usage_flags_on_other_content() {
    let (world, queue, receiver) = queued();
    let view = world.main_tabs().active;
    let slot = world.empty_slot(view);
    for (content, extra) in [
        ("Empty", json!({ "show_claude": true })),
        ("AgentList", json!({ "show_codex": false })),
        ("Agent", json!({ "agent_id": "a1", "show_codex": true })),
    ] {
        let mut args =
            json!({ "view_id": view.to_string(), "slot_id": slot.to_string(), "content": content });
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let mail = Mailbox::default();
        let refused = error_of(call(&receiver, &queue, &mail, "layout.setSlotContent", args).await);
        assert_eq!(refused.code(), ErrorCode::InvalidArgument, "{content}");
    }
    assert_eq!(
        tree::find_slot(&apply::get_view(&world.state, view).unwrap().layout, slot),
        Some(&SlotContent::Empty),
        "a rejection changes nothing"
    );
}

/// Usage slot via the bus: a removed field is `true` at first, then the current value (a one-field toggle leaves the other alone).
#[tokio::test]
async fn slot_content_usage_merges_missing_flags() {
    let (world, queue, receiver) = queued();
    let view = world.main_tabs().active;
    let slot = world.empty_slot(view);
    let usage_at = |world: &World| {
        tree::find_slot(&apply::get_view(&world.state, view).unwrap().layout, slot).cloned()
    };

    let placed = call(
        &receiver,
        &queue,
        &world.mail,
        "layout.setSlotContent",
        json!({ "view_id": view.to_string(), "slot_id": slot.to_string(), "content": "Usage" }),
    )
    .await;
    placed.outcome.expect("usage slot placement");
    assert_eq!(
        usage_at(&world),
        Some(SlotContent::Usage {
            show_claude: true,
            show_codex: true
        })
    );

    let mail = Mailbox::default();
    call(
        &receiver,
        &queue,
        &mail,
        "layout.setSlotContent",
        json!({
            "view_id": view.to_string(),
            "slot_id": slot.to_string(),
            "content": "Usage",
            "show_codex": false,
        }),
    )
    .await
    .outcome
    .expect("codex off");
    assert_eq!(
        usage_at(&world),
        Some(SlotContent::Usage {
            show_claude: true,
            show_codex: false
        })
    );

    let mail = Mailbox::default();
    call(
        &receiver,
        &queue,
        &mail,
        "layout.setSlotContent",
        json!({
            "view_id": view.to_string(),
            "slot_id": slot.to_string(),
            "content": "Usage",
            "show_claude": false,
        }),
    )
    .await
    .outcome
    .expect("claude off");
    assert_eq!(
        usage_at(&world),
        Some(SlotContent::Usage {
            show_claude: false,
            show_codex: false
        }),
        "the earlier switched-off codex does not come back to life"
    );
}

/// Detach from the bus: without a click the slot goes to its own window, and the reply gives that window and the new tab together.
#[tokio::test]
async fn popout_detaches_a_slot_into_a_new_window() {
    let (world, queue, receiver) = queued();
    let (view, slot) = world.filled_slot();

    let ok = call(
        &receiver,
        &queue,
        &world.mail,
        "slot.popout",
        json!({ "view_id": view.to_string(), "slot_id": slot.to_string() }),
    )
    .await
    .outcome
    .expect("detach succeeds");

    let window = ok["window"].as_str().expect("window label").to_string();
    assert_eq!(
        world.windows.opened.lock().unwrap().as_slice(),
        &[window.clone()],
        "omitting to_window opens a new window"
    );
    let tabs = apply::list_tabs(&world.state, &window).expect("new window tab");
    assert_eq!(
        ok["new_view_id"].as_str(),
        Some(tabs.active.to_string().as_str()),
        "the reply's new_view_id is that window's active tab"
    );
    // `view_id` (argument, the original) and `new_view_id` (answer, the destination) are different tabs — feeding the answer back must not pick the original.
    assert!(
        ok.get("view_id").is_none(),
        "if the answer carried `view_id`, the same name as the argument would mean the opposite side: {ok}"
    );
    assert!(
        !world.slots(view).contains(&slot),
        "it is a MOVE — the original slot does not remain"
    );
}

#[tokio::test]
async fn popout_into_a_named_window_adds_a_tab_there() {
    let (world, queue, receiver) = queued();
    let target = call(&receiver, &queue, &world.mail, "window.create", json!({}))
        .await
        .outcome
        .expect("window creation")["window"]
        .as_str()
        .expect("label")
        .to_string();
    let (view, slot) = world.filled_slot();
    let opened_before = world.windows.opened.lock().unwrap().len();

    let mail = Mailbox::default();
    let ok = call(
        &receiver,
        &queue,
        &mail,
        "slot.popout",
        json!({
            "view_id": view.to_string(),
            "slot_id": slot.to_string(),
            "to_window": target,
        }),
    )
    .await
    .outcome
    .expect("detach succeeds");

    assert_eq!(ok["window"].as_str(), Some(target.as_str()));
    assert_eq!(
        world.windows.opened.lock().unwrap().len(),
        opened_before,
        "an existing-window target does not open a new window"
    );
    let tabs = apply::list_tabs(&world.state, &target).expect("target window tabs");
    assert_eq!(tabs.tabs.len(), 2, "empty tab + the moved-in tab");
    assert_eq!(
        ok["new_view_id"].as_str(),
        Some(tabs.active.to_string().as_str())
    );
}

/// An empty slot detaches from the bus too — a new window opens and the original slot closes (ADR-0228).
#[tokio::test]
async fn popout_of_an_empty_slot_opens_a_window() {
    let (world, queue, receiver) = queued();
    let view = world.main_tabs().active;
    let slot = world.empty_slot(view);

    let ok = call(
        &receiver,
        &queue,
        &world.mail,
        "slot.popout",
        json!({ "view_id": view.to_string(), "slot_id": slot.to_string() }),
    )
    .await
    .outcome
    .expect("detach succeeds");

    let window = ok["window"].as_str().expect("window label").to_string();
    assert_eq!(
        world.windows.opened.lock().unwrap().as_slice(),
        &[window.clone()]
    );
    let tabs = apply::list_tabs(&world.state, &window).expect("new window tab");
    assert_eq!(
        ok["new_view_id"].as_str(),
        Some(tabs.active.to_string().as_str())
    );
    assert!(
        !world.slots(view).contains(&slot),
        "it is a MOVE — the original slot does not remain"
    );
}

#[tokio::test]
async fn popout_of_a_missing_slot_is_refused_without_opening_a_window() {
    let (world, queue, receiver) = queued();
    let view = world.main_tabs().active;
    let ghost = uuid::Uuid::new_v4();

    let err = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "slot.popout",
            json!({ "view_id": view.to_string(), "slot_id": ghost.to_string() }),
        )
        .await,
    );

    assert_eq!(err.code(), ErrorCode::Conflict);
    assert!(
        err.message().contains("slot not found"),
        "{}",
        err.message()
    );
    assert!(
        world.windows.opened.lock().unwrap().is_empty(),
        "if refused, no window is opened either"
    );
}

// ── Prewarmed window — the two new windows of the bus also ask the window port first (TRD S21-slot-docking §3-9 · §8-1 ⑬ · ADR-0299) ──────────

const SPARE: &str = "slot-popup-9";

#[tokio::test]
async fn window_create_opens_the_prewarmed_window_under_its_label() {
    let (world, queue, receiver) = queued();
    *world.windows.spare.lock().unwrap() = Some(SPARE.to_string());

    let created = call(&receiver, &queue, &world.mail, "window.create", json!({}))
        .await
        .outcome
        .expect("window creation");

    assert_eq!(created["window"].as_str(), Some(SPARE));
    assert_eq!(world.windows.opened.lock().unwrap().as_slice(), [SPARE]);
    assert_eq!(
        world.windows.spare_calls.lock().unwrap().as_slice(),
        [format!("claim:{SPARE}"), format!("committed:{SPARE}")]
    );
    assert!(apply::list_windows(&world.state)
        .unwrap()
        .iter()
        .any(|w| w == SPARE));
}

#[tokio::test]
async fn popout_into_a_new_window_opens_the_prewarmed_window() {
    let (world, queue, receiver) = queued();
    let (view, slot) = world.filled_slot();
    *world.windows.spare.lock().unwrap() = Some(SPARE.to_string());

    let ok = call(
        &receiver,
        &queue,
        &world.mail,
        "slot.popout",
        json!({ "view_id": view.to_string(), "slot_id": slot.to_string() }),
    )
    .await
    .outcome
    .expect("detach succeeds");

    assert_eq!(ok["window"].as_str(), Some(SPARE));
    assert_eq!(world.windows.opened.lock().unwrap().as_slice(), [SPARE]);
    assert_eq!(
        world.windows.spare_calls.lock().unwrap().as_slice(),
        [format!("claim:{SPARE}"), format!("committed:{SPARE}")]
    );
}

// An early rejection (phase A — a nonexistent slot) hands the handed-over window back — it was never opened, so it is not closed.
#[tokio::test]
async fn a_refused_popout_gives_the_prewarmed_window_back() {
    let (world, queue, receiver) = queued();
    let view = world.main_tabs().active;
    *world.windows.spare.lock().unwrap() = Some(SPARE.to_string());

    let err = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "slot.popout",
            json!({ "view_id": view.to_string(), "slot_id": uuid::Uuid::new_v4().to_string() }),
        )
        .await,
    );

    assert_eq!(err.code(), ErrorCode::Conflict);
    assert_eq!(
        world.windows.spare_calls.lock().unwrap().as_slice(),
        [format!("claim:{SPARE}"), format!("release:{SPARE}")]
    );
    assert!(world.windows.opened.lock().unwrap().is_empty());
    assert!(world.windows.closed.lock().unwrap().is_empty());
}

// ── (B) Window theme — window.getTheme · window.setTheme (TRD S21-storage §5-6) ──────────────
//
// Valid themes · settings · model are real (settings in a temporary folder); only the window side is a recording fake. Measured: the envelope reaches the handle the table holds, answer shape, null clearing, error codes, push after write. Valid-value computation is measured beside `theme` (`--test lib_unit`).

#[tokio::test]
async fn window_get_theme_answers_the_own_and_the_effective_theme() {
    let (world, queue, receiver) = queued();

    let ok = call(
        &receiver,
        &queue,
        &world.mail,
        "window.getTheme",
        json!({ "window": MAIN_WINDOW_LABEL }),
    )
    .await
    .outcome
    .expect("success reply");

    assert_eq!(
        ok,
        json!({ "theme": null, "effective": DEFAULT_THEME.as_wire() }),
        "a window with no theme of its own = theme null · effective is the global value"
    );
}

#[tokio::test]
async fn window_set_theme_writes_the_window_and_pushes_every_window() {
    let (world, queue, receiver) = queued();
    world.settings.set(THEME_DEFAULT, "light").expect("write");

    let ok = call(
        &receiver,
        &queue,
        &world.mail,
        "window.setTheme",
        json!({ "window": MAIN_WINDOW_LABEL, "theme": "e-ink" }),
    )
    .await
    .outcome
    .expect("success reply");

    assert_eq!(ok, json!({ "theme": "e-ink", "effective": "e-ink" }));
    assert_eq!(
        world
            .state
            .0
            .lock()
            .unwrap()
            .window_attrs(MAIN_WINDOW_LABEL)
            .unwrap()
            .theme,
        Some(UiTheme::EInk)
    );
    assert_eq!(
        world.theme_windows.take(),
        sent(&[
            (MAIN_WINDOW_LABEL, "e-ink"),
            (UNMODELED_POPOUT_LABEL, "light")
        ])
    );

    world.mail.clear();
    let read = call(
        &receiver,
        &queue,
        &world.mail,
        "window.getTheme",
        json!({ "window": MAIN_WINDOW_LABEL }),
    )
    .await
    .outcome
    .expect("success reply");
    assert_eq!(read, ok, "the read returns the written value as is");
}

/// `null` clears — that window follows `theme.default` again. The layout version does not move (only the window property version).
#[tokio::test]
async fn window_set_theme_null_clears_the_windows_own_theme() {
    let (world, queue, receiver) = queued();
    let before = world.main_tabs();

    let set = call(
        &receiver,
        &queue,
        &world.mail,
        "window.setTheme",
        json!({ "window": MAIN_WINDOW_LABEL, "theme": "Light" }),
    )
    .await
    .outcome
    .expect("success reply");
    assert_eq!(
        set,
        json!({ "theme": "light", "effective": "light" }),
        "answers in the canonical spelling regardless of case"
    );
    let attrs_rev = world.state.0.lock().unwrap().attrs_rev();

    world.mail.clear();
    world.theme_windows.take();
    let cleared = call(
        &receiver,
        &queue,
        &world.mail,
        "window.setTheme",
        json!({ "window": MAIN_WINDOW_LABEL, "theme": null }),
    )
    .await
    .outcome
    .expect("success reply");

    assert_eq!(cleared, json!({ "theme": null, "effective": "dark" }));
    {
        let mgr = world.state.0.lock().unwrap();
        assert_eq!(mgr.window_attrs(MAIN_WINDOW_LABEL).unwrap().theme, None);
        assert_eq!(mgr.attrs_rev(), attrs_rev + 1);
    }
    assert_eq!(
        world.main_tabs().version,
        before.version,
        "a window theme does not bump the layout version"
    );
    assert_eq!(
        world.theme_windows.take(),
        sent(&[
            (MAIN_WINDOW_LABEL, "dark"),
            (UNMODELED_POPOUT_LABEL, "dark")
        ])
    );
}

/// An absent `theme` field does not clear — one typo field must not quietly erase the window theme.
#[tokio::test]
async fn window_set_theme_without_the_theme_field_is_refused() {
    let (world, queue, receiver) = queued();
    world
        .themes
        .set(MAIN_WINDOW_LABEL, Some(UiTheme::Light))
        .expect("window theme");
    world.theme_windows.take();

    let err = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "window.setTheme",
            json!({ "window": MAIN_WINDOW_LABEL, "them": null }),
        )
        .await,
    );

    assert_eq!(err.code(), ErrorCode::InvalidArgument);
    assert_eq!(
        world.themes.get(MAIN_WINDOW_LABEL).unwrap().own,
        Some(UiTheme::Light)
    );
    assert!(
        world.theme_windows.take().is_empty(),
        "on refusal there is no push"
    );
}

#[tokio::test]
async fn an_unknown_theme_is_an_invalid_argument() {
    let (world, queue, receiver) = queued();

    for given in ["solarized", ""] {
        world.mail.clear();
        let err = error_of(
            call(
                &receiver,
                &queue,
                &world.mail,
                "window.setTheme",
                json!({ "window": MAIN_WINDOW_LABEL, "theme": given }),
            )
            .await,
        );
        assert_eq!(err.code(), ErrorCode::InvalidArgument, "{given:?}");
        assert!(
            err.message().contains("dark, light or e-ink"),
            "tells the accepted values: {}",
            err.message()
        );
    }
    assert_eq!(world.themes.get(MAIN_WINDOW_LABEL).unwrap().own, None);
    assert!(world.theme_windows.take().is_empty());
}

#[tokio::test]
async fn an_unknown_window_is_a_conflict_for_both_theme_commands() {
    let (world, queue, receiver) = queued();

    for (name, args) in [
        ("window.getTheme", json!({ "window": "slot-popup-9" })),
        (
            "window.setTheme",
            json!({ "window": "slot-popup-9", "theme": "light" }),
        ),
    ] {
        world.mail.clear();
        let err = error_of(call(&receiver, &queue, &world.mail, name, args).await);
        assert_eq!(err.code(), ErrorCode::Conflict, "{name}");
        assert!(
            err.message().contains("slot-popup-9"),
            "{name}: {}",
            err.message()
        );
    }
    assert!(world.theme_windows.take().is_empty());
}

/// ADR-0225: the tree-only window (`agent-tree`) is retired — that label is the same `CONFLICT` as any unknown window; the write makes no window and pushes nothing.
#[tokio::test]
async fn the_retired_tree_window_label_is_an_unknown_window_for_both_theme_commands() {
    let (world, queue, receiver) = queued();
    let retired = "agent-tree";

    for (name, args) in [
        ("window.getTheme", json!({ "window": retired })),
        (
            "window.setTheme",
            json!({ "window": retired, "theme": "light" }),
        ),
    ] {
        world.mail.clear();
        let err = error_of(call(&receiver, &queue, &world.mail, name, args).await);
        assert_eq!(err.code(), ErrorCode::Conflict, "{name}");
        assert!(err.message().contains(retired), "{name}: {}", err.message());
    }
    assert!(world.state.0.lock().unwrap().window_attrs(retired).is_err());
    assert!(world.theme_windows.take().is_empty());
}

/// A window that could not receive makes the write an `INTERNAL` failure naming that window and the reason (ADR-0166 decision 6); the written value stays and `window.getTheme` answers it.
#[tokio::test]
async fn a_theme_the_target_window_did_not_receive_is_internal_and_stays_written() {
    let (world, queue, receiver) = queued();
    world.theme_windows.refuse(MAIN_WINDOW_LABEL);

    let err = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "window.setTheme",
            json!({ "window": MAIN_WINDOW_LABEL, "theme": "light" }),
        )
        .await,
    );

    assert_eq!(err.code(), ErrorCode::Internal);
    assert!(
        err.message().contains(MAIN_WINDOW_LABEL) && err.message().contains(WINDOW_GONE),
        "{}",
        err.message()
    );
    assert_eq!(world.theme_windows.take().len(), 2, "tries every window");

    world.mail.clear();
    let read = call(
        &receiver,
        &queue,
        &world.mail,
        "window.getTheme",
        json!({ "window": MAIN_WINDOW_LABEL }),
    )
    .await
    .outcome
    .expect("success reply");
    assert_eq!(read, json!({ "theme": "light", "effective": "light" }));
}

#[tokio::test]
async fn a_theme_only_another_window_did_not_receive_is_a_success() {
    let (world, queue, receiver) = queued();
    world.theme_windows.refuse(UNMODELED_POPOUT_LABEL);

    let ok = call(
        &receiver,
        &queue,
        &world.mail,
        "window.setTheme",
        json!({ "window": MAIN_WINDOW_LABEL, "theme": "light" }),
    )
    .await
    .outcome
    .expect("success reply");

    assert_eq!(ok, json!({ "theme": "light", "effective": "light" }));
    assert_eq!(world.theme_windows.take().len(), 2);
}

#[tokio::test]
async fn a_popout_theme_lives_and_dies_with_its_window() {
    let (world, queue, receiver) = queued();
    let created = call(&receiver, &queue, &world.mail, "window.create", json!({}))
        .await
        .outcome
        .expect("success reply");
    let popout = created["window"].as_str().expect("label").to_string();

    world.mail.clear();
    call(
        &receiver,
        &queue,
        &world.mail,
        "window.setTheme",
        json!({ "window": popout, "theme": "e-ink" }),
    )
    .await
    .outcome
    .expect("success reply");
    assert_eq!(
        world
            .state
            .0
            .lock()
            .unwrap()
            .window_attrs(&popout)
            .unwrap()
            .theme,
        Some(UiTheme::EInk)
    );

    world.mail.clear();
    call(
        &receiver,
        &queue,
        &world.mail,
        "window.close",
        json!({ "window": popout }),
    )
    .await
    .outcome
    .expect("success reply");
    world.mail.clear();
    let err = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "window.getTheme",
            json!({ "window": popout }),
        )
        .await,
    );
    assert_eq!(err.code(), ErrorCode::Conflict);
}

// ── (B) Shell settings — settings.get · set · reset · schema (TRD S21-storage §5-4) ──────────
//
// The service is real (temporary folder, writes opened as in shell setup); only notifications are fake. Measured: the envelope reaches the service the table holds, its error kinds go out as same-named codes, and only actually changed writes are announced. Value normalization · file tolerance are measured beside the service (`--test lib_unit`).

fn setting(key: &str, value: &str, is_default: bool) -> SettingItem {
    SettingItem {
        key: key.to_string(),
        value: value.to_string(),
        is_default,
    }
}

#[tokio::test]
async fn settings_get_answers_from_the_shared_service() {
    let (world, queue, receiver) = queued();

    let all = call(&receiver, &queue, &world.mail, "settings.get", json!({}))
        .await
        .outcome
        .expect("success reply");
    assert_eq!(all["rev"], 0);
    let items = all["items"].as_array().expect("items");
    assert_eq!(items.len(), 13);
    assert_eq!(
        items[0],
        json!({"key": "theme.default", "value": "dark", "is_default": true})
    );

    world.mail.clear();
    let chat = call(
        &receiver,
        &queue,
        &world.mail,
        "settings.get",
        json!({"key": "chat.style."}),
    )
    .await
    .outcome
    .expect("success reply");
    let keys: Vec<&str> = chat["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|item| item["key"].as_str().expect("key"))
        .collect();
    assert_eq!(keys.len(), 11);
    assert!(
        keys.iter().all(|key| key.starts_with("chat.style.")),
        "{keys:?}"
    );
}

/// The service the table holds is the same instance as the human path (else the screen would not see a value the LLM changed), and the value stays on disk for the next load.
#[tokio::test]
async fn settings_set_writes_through_the_shared_service_and_announces_once() {
    let (world, queue, receiver) = queued();

    let reply = call(
        &receiver,
        &queue,
        &world.mail,
        "settings.set",
        json!({"key": "theme.default", "value": "E-INK"}),
    )
    .await;

    assert_eq!(
        reply.outcome.expect("success reply"),
        json!({"rev": 1, "key": "theme.default", "value": "e-ink", "changed": true}),
        "the answer's value is the canonical form"
    );
    assert_eq!(
        world.settings.effective(THEME_DEFAULT).as_deref(),
        Some("e-ink")
    );
    assert_eq!(
        world.settings_events.changes(),
        vec![SettingsSnapshot {
            rev: 1,
            items: vec![setting("theme.default", "e-ink", false)],
        }]
    );
    assert_eq!(
        world.settings_events.theme_pushes(),
        1,
        "when the global theme changes, it is pushed again for every window"
    );
    assert_eq!(
        SettingsService::load_from_dir(&world._config.0)
            .effective(THEME_DEFAULT)
            .as_deref(),
        Some("e-ink"),
        "survives a restart"
    );
}

#[tokio::test]
async fn a_set_that_changes_nothing_is_answered_but_never_announced() {
    let (world, queue, receiver) = queued();

    let reply = call(
        &receiver,
        &queue,
        &world.mail,
        "settings.set",
        json!({"key": "theme.default", "value": "dark"}),
    )
    .await;

    assert_eq!(
        reply.outcome.expect("success reply"),
        json!({"rev": 0, "key": "theme.default", "value": "dark", "changed": false})
    );
    assert!(world.settings_events.changes().is_empty());
    assert_eq!(world.settings_events.theme_pushes(), 0);
}

#[tokio::test]
async fn a_change_to_another_key_is_announced_without_a_theme_push() {
    let (world, queue, receiver) = queued();

    let reply = call(
        &receiver,
        &queue,
        &world.mail,
        "settings.set",
        json!({"key": "chat.style.fontSize", "value": "15.0PX"}),
    )
    .await;

    assert_eq!(reply.outcome.expect("success reply")["value"], "15px");
    assert_eq!(
        world.settings_events.changes(),
        vec![SettingsSnapshot {
            rev: 1,
            items: vec![setting("chat.style.fontSize", "15px", false)],
        }]
    );
    assert_eq!(world.settings_events.theme_pushes(), 0);
}

#[tokio::test]
async fn an_unknown_setting_is_not_found_on_every_settings_command() {
    let (world, queue, receiver) = queued();

    for (name, args) in [
        ("settings.get", json!({"key": "no.such.key"})),
        ("settings.get", json!({"key": "nothing."})),
        ("settings.set", json!({"key": "no.such.key", "value": "x"})),
        ("settings.reset", json!({"key": "nothing."})),
        ("settings.schema", json!({"key": "no.such.key"})),
    ] {
        world.mail.clear();
        let err = error_of(call(&receiver, &queue, &world.mail, name, args.clone()).await);
        assert_eq!(err.code(), ErrorCode::NotFound, "{name} {args}");
    }
    assert!(world.settings_events.changes().is_empty());
}

#[tokio::test]
async fn a_malformed_setting_is_an_invalid_argument_and_changes_nothing() {
    let (world, queue, receiver) = queued();

    for args in [
        json!({"key": "theme.default", "value": "purple"}),
        json!({"key": "chat.style.fontSize", "value": "48rem"}),
        json!({"key": "theme.", "value": "light"}),
        json!({"key": "", "value": "light"}),
        json!({"key": "theme.default", "value": "  "}),
    ] {
        world.mail.clear();
        let err =
            error_of(call(&receiver, &queue, &world.mail, "settings.set", args.clone()).await);
        assert_eq!(err.code(), ErrorCode::InvalidArgument, "{args}");
    }
    world.mail.clear();
    let err = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "settings.set",
            json!({"key": "theme.default", "value": "purple"}),
        )
        .await,
    );
    assert!(
        err.message().contains("theme.default"),
        "the text must say which key's format it is: {}",
        err.message()
    );

    assert_eq!(world.settings.get(None).expect("read").rev, 0);
    assert!(world.settings_events.changes().is_empty());
}

#[tokio::test]
async fn a_write_before_the_shell_opens_writes_is_internal() {
    let (world, mut ports) = World::build();
    let closed = ConfigDir::new();
    ports.settings = Arc::new(SettingsService::load_from_dir(&closed.0));
    let queue = Arc::new(Queued::default());
    let receiver = InboundReceiver::new(
        make_table(ports),
        Arc::clone(&queue) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    );

    let err = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "settings.set",
            json!({"key": "theme.default", "value": "light"}),
        )
        .await,
    );

    assert_eq!(err.code(), ErrorCode::Internal);
    assert!(world.settings_events.changes().is_empty());
    assert!(!closed.0.exists(), "the folder was made before writing");
}

/// ADR-0291 R2: a settings file written by a newer version makes the write `CONFLICT`, not `INTERNAL` (retrying is the same until the file changes). File bytes · notifications unchanged.
#[tokio::test]
async fn a_write_over_a_settings_file_from_a_newer_version_is_conflict() {
    let (world, mut ports) = World::build();
    let newer = ConfigDir::new();
    std::fs::create_dir_all(&newer.0).expect("folder");
    let text = r#"{"$version":2,"theme.default":"light"}"#;
    std::fs::write(newer.0.join("settings.json"), text).expect("newer-version file");
    ports.settings = newer.service();
    let queue = Arc::new(Queued::default());
    let receiver = InboundReceiver::new(
        make_table(ports),
        Arc::clone(&queue) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    );

    let err = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "settings.set",
            json!({"key": "theme.default", "value": "e-ink"}),
        )
        .await,
    );

    assert_eq!(err.code(), ErrorCode::Conflict);
    assert!(world.settings_events.changes().is_empty());
    assert_eq!(
        std::fs::read_to_string(newer.0.join("settings.json")).expect("file"),
        text,
        "not overwritten"
    );
    assert!(
        !newer.0.join("settings.json.corrupt").exists(),
        "not copied aside"
    );
}

/// ADR-0291 R17: a settings file that cannot be re-read just before writing is `CONFLICT` · `after-condition` (usually a brief lock; distinct from `never` for a newer-version file). Value · notifications unchanged; the unreadable place is a folder at the file name.
#[tokio::test]
async fn a_write_over_an_unreadable_settings_file_is_conflict_to_retry_later() {
    let (world, mut ports) = World::build();
    let blocked = ConfigDir::new();
    std::fs::create_dir_all(blocked.0.join("settings.json")).expect("a folder at the file place");
    ports.settings = blocked.service();
    let queue = Arc::new(Queued::default());
    let receiver = InboundReceiver::new(
        make_table(ports),
        Arc::clone(&queue) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    );

    let err = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "settings.set",
            json!({"key": "theme.default", "value": "e-ink"}),
        )
        .await,
    );

    assert_eq!(err.code(), ErrorCode::Conflict, "{}", err.message());
    assert_eq!(
        err.retry(),
        engram_dashboard_command::RetryMode::AfterCondition
    );
    assert!(world.settings_events.changes().is_empty());
    assert!(
        blocked.0.join("settings.json").is_dir(),
        "that place is not overwritten"
    );
}

/// The write body runs on the blocking pool, not on a runtime thread polling the apply task, so tasks sharing that worker (including the connection socket) do not stall on `sync_all`.
#[tokio::test]
async fn settings_writes_run_off_the_runtime_thread() {
    let (world, queue, receiver) = queued();
    let here = std::thread::current().id();

    for (name, args) in [
        (
            "settings.set",
            json!({"key": "theme.default", "value": "light"}),
        ),
        ("settings.reset", json!({"key": "theme."})),
    ] {
        world.mail.clear();
        call(&receiver, &queue, &world.mail, name, args)
            .await
            .outcome
            .expect("success reply");
    }

    let threads = world.settings_events.threads();
    assert_eq!(threads.len(), 2);
    assert!(threads.iter().all(|id| *id != here), "{threads:?}");
}

#[tokio::test]
async fn settings_reset_names_every_covered_key_and_announces_only_the_changed_ones() {
    let (world, queue, receiver) = queued();
    for (key, value) in [
        ("chat.style.fontSize", "15px"),
        ("chat.style.userPx", "4px"),
    ] {
        world.mail.clear();
        call(
            &receiver,
            &queue,
            &world.mail,
            "settings.set",
            json!({"key": key, "value": value}),
        )
        .await
        .outcome
        .expect("success reply");
    }

    world.mail.clear();
    let ok = call(
        &receiver,
        &queue,
        &world.mail,
        "settings.reset",
        json!({"key": "chat.style."}),
    )
    .await
    .outcome
    .expect("success reply");

    assert_eq!(ok["rev"], 3);
    let reset = ok["reset"].as_array().expect("reset");
    assert_eq!(reset.len(), 11, "every key the selector covered");
    assert!(
        ok.get("changed").is_none(),
        "the bus answer is {{rev, reset}}: {ok}"
    );
    assert_eq!(
        world.settings_events.changes().last(),
        Some(&SettingsSnapshot {
            rev: 3,
            items: vec![
                setting("chat.style.userPx", "0.9rem", true),
                setting("chat.style.fontSize", "13px", true),
            ],
        }),
        "the notification carries only changed keys"
    );
    assert_eq!(world.settings_events.theme_pushes(), 0);

    world.mail.clear();
    let again = call(
        &receiver,
        &queue,
        &world.mail,
        "settings.reset",
        json!({"key": "chat.style."}),
    )
    .await
    .outcome
    .expect("success reply");
    assert_eq!(again["rev"], 3);
    assert_eq!(world.settings_events.changes().len(), 3);
}

#[tokio::test]
async fn resetting_theme_default_pushes_themes_again() {
    let (world, queue, receiver) = queued();
    call(
        &receiver,
        &queue,
        &world.mail,
        "settings.set",
        json!({"key": "theme.default", "value": "light"}),
    )
    .await
    .outcome
    .expect("success reply");

    world.mail.clear();
    let ok = call(
        &receiver,
        &queue,
        &world.mail,
        "settings.reset",
        json!({"key": "theme.default"}),
    )
    .await
    .outcome
    .expect("success reply");

    assert_eq!(ok, json!({"rev": 2, "reset": ["theme.default"]}));
    assert_eq!(world.settings_events.theme_pushes(), 2);
    assert_eq!(global_theme(&world.settings), UiTheme::Dark);
}

#[tokio::test]
async fn settings_schema_describes_the_keys() {
    let (world, queue, receiver) = queued();

    let theme = call(
        &receiver,
        &queue,
        &world.mail,
        "settings.schema",
        json!({"key": "theme.default"}),
    )
    .await
    .outcome
    .expect("success reply");
    let row = &theme["items"][0];
    assert_eq!(row["key"], "theme.default");
    assert_eq!(row["kind"], "choice");
    assert_eq!(row["default"], "dark");
    assert_eq!(row["choices"], json!(["dark", "light", "e-ink"]));
    assert!(row["min"].is_null() && row["max"].is_null(), "{row}");
    assert!(row["description"].as_str().is_some_and(|d| !d.is_empty()));

    world.mail.clear();
    let size = call(
        &receiver,
        &queue,
        &world.mail,
        "settings.schema",
        json!({"key": "chat.style.fontSize"}),
    )
    .await
    .outcome
    .expect("success reply");
    assert_eq!(size["items"][0]["kind"], "css-length");
    assert_eq!(size["items"][0]["min"], "8px, 0.5rem, 0.5em");
    assert!(size["items"][0]["choices"].is_null());

    world.mail.clear();
    let all = call(&receiver, &queue, &world.mail, "settings.schema", json!({}))
        .await
        .outcome
        .expect("success reply");
    assert_eq!(all["items"].as_array().expect("items").len(), 13);
}

/// ADR-0373: the LLM path of a `ui.language` write reaches the language hook, which here does what production does to the layout (`apply::language_changed`), so `tab.list` answers in the new language.
#[tokio::test]
async fn a_bus_ui_language_write_reaches_the_language_hook_and_renames_auto_tabs() {
    struct Applying {
        settings: Arc<SettingsService>,
        state: LayoutState,
        events: Arc<dyn LayoutEvents>,
        heard: Mutex<usize>,
    }

    impl SettingsEvents for Applying {
        fn changed(&self, _change: &SettingsSnapshot) {}

        fn theme_default_changed(&self) {}

        fn ui_language_changed(&self) {
            *self.heard.lock().unwrap() += 1;
            apply::language_changed(
                &self.state,
                self.events.as_ref(),
                Lang::from_settings(&self.settings),
            )
            .unwrap();
        }
    }

    let (world, mut ports) = World::build();
    let hook = Arc::new(Applying {
        settings: Arc::clone(&world.settings),
        state: world.state.clone(),
        events: Arc::clone(&world.events) as Arc<dyn LayoutEvents>,
        heard: Mutex::default(),
    });
    ports.settings_events = Arc::clone(&hook) as Arc<dyn SettingsEvents>;
    let queue = Arc::new(Queued::default());
    let receiver = InboundReceiver::new(
        make_table(ports),
        Arc::clone(&queue) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    );
    let main_tab_name = |listed: serde_json::Value| listed["tabs"][0]["name"].clone();
    async fn verb(
        receiver: &InboundReceiver,
        queue: &Queued,
        name: &str,
        args: serde_json::Value,
    ) -> serde_json::Value {
        call(receiver, queue, &Mailbox::default(), name, args)
            .await
            .outcome
            .expect("success reply")
    }
    let list = json!({ "window": MAIN_WINDOW_LABEL });

    assert_eq!(
        main_tab_name(verb(&receiver, &queue, "tab.list", list.clone()).await),
        "Empty slot"
    );
    verb(
        &receiver,
        &queue,
        "settings.set",
        json!({"key": "ui.language", "value": "ko"}),
    )
    .await;
    assert_eq!(*hook.heard.lock().unwrap(), 1);
    assert_eq!(
        main_tab_name(verb(&receiver, &queue, "tab.list", list.clone()).await),
        "빈 칸" // hangul-ok: the Korean UI table
    );

    verb(
        &receiver,
        &queue,
        "settings.set",
        json!({"key": "theme.default", "value": "light"}),
    )
    .await;
    assert_eq!(
        *hook.heard.lock().unwrap(),
        1,
        "another key does not reach it"
    );

    verb(&receiver, &queue, "settings.reset", json!({"key": "ui."})).await;
    assert_eq!(*hook.heard.lock().unwrap(), 2);
    assert_eq!(
        main_tab_name(verb(&receiver, &queue, "tab.list", list).await),
        "Empty slot"
    );
}

/// Bus write → notification → effective theme push in one line, wired like the production adapter (`TauriSettingsEvents`) but without a window.
#[tokio::test]
async fn a_bus_theme_default_write_reaches_every_window_without_its_own_theme() {
    struct Pushing(ThemeControl);

    impl SettingsEvents for Pushing {
        fn changed(&self, _change: &SettingsSnapshot) {}

        fn theme_default_changed(&self) {
            self.0.push();
        }

        fn ui_language_changed(&self) {}
    }

    let (world, mut ports) = World::build();
    world
        .themes
        .set(MAIN_WINDOW_LABEL, Some(UiTheme::EInk))
        .expect("window theme");
    world.theme_windows.take();
    ports.settings_events = Arc::new(Pushing(world.themes.clone()));
    let queue = Arc::new(Queued::default());
    let receiver = InboundReceiver::new(
        make_table(ports),
        Arc::clone(&queue) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    );

    call(
        &receiver,
        &queue,
        &world.mail,
        "settings.set",
        json!({"key": "theme.default", "value": "light"}),
    )
    .await
    .outcome
    .expect("success reply");

    assert_eq!(
        world.theme_windows.take(),
        sent(&[
            (MAIN_WINDOW_LABEL, "e-ink"),
            (UNMODELED_POPOUT_LABEL, "light")
        ])
    );
}

// ── (B) Crash copy — restore.status · restore.answer (TRD S21-storage §6-7) ──────────
//
// The coordinator is real (only ports are fake) and the test holds the same instance as the table. Measured: the envelope reaches it, answer shape, error kinds → `CONFLICT` · `INTERNAL`, and the blocking acceptance body runs off the runtime thread. Screen-swap order · rollback are measured beside the coordinator (`--test lib_unit`).

/// Window port stand-in of the restore acceptance — every window is visible and creation always succeeds; records hidden windows and the body's thread.
#[derive(Default)]
struct RestoreScreen {
    opened: Mutex<Vec<String>>,
    threads: Mutex<Vec<std::thread::ThreadId>>,
}

impl RestoreWindows for RestoreScreen {
    fn app_has_focus(&self) -> bool {
        false
    }

    fn monitors(&self) -> Vec<MonitorArea> {
        self.threads
            .lock()
            .unwrap()
            .push(std::thread::current().id());
        vec![MonitorArea {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
            scale: 1.0,
        }]
    }

    fn open_hidden(
        &self,
        label: &str,
        _at: Option<(WindowBounds, Landing)>,
        _maximized: bool,
    ) -> Result<(), String> {
        self.opened.lock().unwrap().push(label.to_string());
        Ok(())
    }

    fn visibility(&self, _label: &str) -> Option<bool> {
        Some(true)
    }

    fn place_main(&self, _at: Option<(WindowBounds, Landing)>, _maximized: bool) {}

    fn set_shown(&self, _label: &str, _shown: bool) {}

    fn record_placement(&self, _label: &str) {}

    fn focus(&self, _label: &str) {}

    fn destroy(&self, _label: &str) -> Result<(), String> {
        Ok(())
    }
}

struct NoSubscriptions;

impl SubscriptionSource for NoSubscriptions {
    fn current(&self) -> Option<Arc<dyn SubscriptionSync>> {
        None
    }
}

impl World {
    fn attach_restore_ports(&self) -> Arc<RestoreScreen> {
        let screen = Arc::new(RestoreScreen::default());
        self.restore.attach(RestorePorts {
            windows: Arc::clone(&screen) as Arc<dyn RestoreWindows>,
            events: Arc::new(Events),
            subs: Arc::new(NoSubscriptions),
            themes: self.themes.clone(),
        });
        screen
    }

    /// Boot step ⑥ — sets the previous run's screen (a main tab 「last tab」 + `popouts` empty pop-outs) as an unanswered copy.
    fn crash_copy_awaits(&self, popouts: usize) {
        self.crash_copy_awaits_with(popouts, false, StateFileStatus::Ok);
    }

    fn crash_copy_awaits_with(&self, popouts: usize, saves: bool, state_file: StateFileStatus) {
        let mut previous = ViewManager::new();
        let main_view = previous.windows[MAIN_WINDOW_LABEL].active;
        previous
            .rename_tab(main_view, "last tab".to_string())
            .expect("tab name");
        for n in 0..popouts {
            previous
                .create_window(&format!("slot-popup-{}", 90 + n))
                .expect("pop-out");
        }
        self.restore_service.set_boot(
            Some(CrashCopy {
                text: "{}".to_string(),
                hash: "h".to_string(),
                file: StateFile {
                    version: STATE_VERSION,
                    saved_at_ms: 1_700_000_000_000,
                    clean_exit: false,
                    resolved_crash_copy: None,
                    windows: to_persisted(&previous),
                },
            }),
            state_file,
            saves,
        );
    }

    fn main_tab_name(&self) -> String {
        let mgr = self.state.0.lock().unwrap();
        effective_tab_name(&mgr, &mgr.views[&mgr.windows[MAIN_WINDOW_LABEL].active])
    }
}

#[tokio::test]
async fn restore_status_reports_the_crash_copy_shape() {
    let (world, queue, receiver) = queued();

    let none = call(&receiver, &queue, &world.mail, "restore.status", json!({}))
        .await
        .outcome
        .expect("success reply");
    assert_eq!(
        none,
        json!({"crash_copy": "none", "saved_at_ms": null, "windows": null, "tabs": null, "durable": null, "saves": true, "state_file": "ok"}),
        "with no copy to ask about, the copy's three are null"
    );

    world.crash_copy_awaits(2);
    world.mail.clear();
    let awaiting = call(&receiver, &queue, &world.mail, "restore.status", json!({}))
        .await
        .outcome
        .expect("success reply");
    assert_eq!(
        awaiting,
        json!({"crash_copy": "awaiting", "saved_at_ms": 1_700_000_000_000_u64, "windows": 3, "tabs": 3, "durable": false, "saves": false, "state_file": "ok"}),
        "window count = main + pop-outs"
    );
}

#[tokio::test]
async fn restore_status_carries_the_state_file_status_in_its_wire_spelling() {
    let (world, queue, receiver) = queued();

    world
        .restore_service
        .set_boot(None, StateFileStatus::Unreadable, false);
    let unreadable = call(&receiver, &queue, &world.mail, "restore.status", json!({}))
        .await
        .outcome
        .expect("success reply");
    assert_eq!(
        unreadable,
        json!({"crash_copy": "none", "saved_at_ms": null, "windows": null, "tabs": null, "durable": null, "saves": false, "state_file": "unreadable"}),
        "state_file is a value even with no copy"
    );

    for (state_file, wire) in [
        (StateFileStatus::Newer, "newer"),
        (StateFileStatus::CorruptCopiedAside, "corrupt_copied_aside"),
        (StateFileStatus::CorruptNotCopied, "corrupt_not_copied"),
    ] {
        world.crash_copy_awaits_with(0, true, state_file);
        world.mail.clear();
        let reply = call(&receiver, &queue, &world.mail, "restore.status", json!({}))
            .await
            .outcome
            .expect("success reply");
        assert_eq!(reply["crash_copy"], json!("awaiting"));
        assert_eq!(reply["state_file"], json!(wire));
    }
}

#[tokio::test]
async fn restore_status_carries_saves_always_and_durable_only_while_awaiting() {
    for saves in [true, false] {
        let (world, queue, receiver) = queued();

        world
            .restore_service
            .set_boot(None, StateFileStatus::Ok, saves);
        let none = call(&receiver, &queue, &world.mail, "restore.status", json!({}))
            .await
            .outcome
            .expect("success reply");
        assert_eq!(
            none["saves"],
            json!(saves),
            "a value even with no copy — {saves}"
        );
        assert_eq!(none["durable"], json!(null));

        world.crash_copy_awaits_with(0, saves, StateFileStatus::Ok);
        world.mail.clear();
        let awaiting = call(&receiver, &queue, &world.mail, "restore.status", json!({}))
            .await
            .outcome
            .expect("success reply");
        assert_eq!(awaiting["crash_copy"], json!("awaiting"));
        assert_eq!(awaiting["saves"], json!(saves));
        assert_eq!(
            awaiting["durable"], awaiting["saves"],
            "while asking, the two are the same — {saves}"
        );

        world.mail.clear();
        call(
            &receiver,
            &queue,
            &world.mail,
            "restore.answer",
            json!({"accept": false}),
        )
        .await
        .outcome
        .expect("refusal reply");
        world.mail.clear();
        let answered = call(&receiver, &queue, &world.mail, "restore.status", json!({}))
            .await
            .outcome
            .expect("success reply");
        assert_eq!(answered["crash_copy"], json!("answered"));
        assert_eq!(
            answered["durable"],
            json!(null),
            "outside awaiting it is null"
        );
        assert_eq!(
            answered["saves"],
            json!(saves),
            "the value of that run even after answering — {saves}"
        );
    }
}

#[tokio::test]
async fn restore_answer_conflicts_while_not_awaiting_or_while_another_answer_runs() {
    let (world, queue, receiver) = queued();
    world.attach_restore_ports();

    let no_copy = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "restore.answer",
            json!({"accept": true}),
        )
        .await,
    );
    assert_eq!(no_copy.code(), ErrorCode::Conflict, "there is no copy");

    world.crash_copy_awaits(0);
    let held = world.restore_service.begin_answer().expect("can answer");
    world.mail.clear();
    let in_flight = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "restore.answer",
            json!({"accept": false}),
        )
        .await,
    );
    assert_eq!(
        in_flight.code(),
        ErrorCode::Conflict,
        "another answer is being processed"
    );
    drop(held);

    world.mail.clear();
    let rejected = call(
        &receiver,
        &queue,
        &world.mail,
        "restore.answer",
        json!({"accept": false}),
    )
    .await
    .outcome
    .expect("a refusal is an answer");
    assert_eq!(
        rejected,
        json!({"restored_windows": 0, "durable": false}),
        "a refusal does not change the screen · and with no recorder it does not touch the disk"
    );

    world.mail.clear();
    let again = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "restore.answer",
            json!({"accept": true}),
        )
        .await,
    );
    assert_eq!(again.code(), ErrorCode::Conflict, "already answered");
    assert_ne!(
        again.message(),
        no_copy.message(),
        "the text tells apart no copy and already answered (a person reads it)"
    );
    assert_eq!(world.restore.status().crash_copy, CrashCopyStatus::Answered);
}

#[tokio::test]
async fn an_accept_before_the_shell_attaches_its_ports_is_internal_and_can_be_retried() {
    let (world, queue, receiver) = queued();
    world.crash_copy_awaits(0);

    let early = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "restore.answer",
            json!({"accept": true}),
        )
        .await,
    );
    assert_eq!(early.code(), ErrorCode::Internal);
    assert_eq!(
        early.retry(),
        engram_dashboard_command::RetryMode::Never,
        "instructions are not carried — the daemon relays and lowers everything to never (ADR-0159)"
    );
    assert!(
        early.message().contains("answer again"),
        "the text carries the guidance to answer again: {}",
        early.message()
    );
    assert_eq!(
        world.restore.status().crash_copy,
        CrashCopyStatus::Awaiting,
        "nothing changed — can answer again"
    );

    world.attach_restore_ports();
    world.mail.clear();
    let retried = call(
        &receiver,
        &queue,
        &world.mail,
        "restore.answer",
        json!({"accept": true}),
    )
    .await
    .outcome
    .expect("answer again after the port stands");
    assert_eq!(retried, json!({"restored_windows": 1, "durable": false}));
}

/// The coordinator the table holds is the same instance as the human path — acceptance turns the model into the copy's screen, on the blocking pool.
#[tokio::test]
async fn restore_accept_swaps_the_screen_through_the_shared_coordinator_off_the_runtime_thread() {
    let (world, queue, receiver) = queued();
    let screen = world.attach_restore_ports();
    world.crash_copy_awaits(1);
    let here = std::thread::current().id();

    let accepted = call(
        &receiver,
        &queue,
        &world.mail,
        "restore.answer",
        json!({"accept": true}),
    )
    .await
    .outcome
    .expect("acceptance");

    assert_eq!(accepted, json!({"restored_windows": 2, "durable": false}));
    assert_eq!(world.main_tab_name(), "last tab");
    let opened = screen.opened.lock().unwrap().clone();
    assert_eq!(
        opened,
        [popup_label(1)],
        "a new pop-out gets the label of the shared issuer"
    );
    let mut windows = world.state.0.lock().unwrap().list_windows();
    windows.sort();
    assert_eq!(windows, [MAIN_WINDOW_LABEL.to_string(), popup_label(1)]);
    assert_eq!(world.restore.status().crash_copy, CrashCopyStatus::Answered);
    let threads = screen.threads.lock().unwrap().clone();
    assert_eq!(threads.len(), 1);
    assert!(threads.iter().all(|id| *id != here), "{threads:?}");
}

#[tokio::test]
async fn restore_answer_needs_the_accept_flag() {
    let (world, queue, receiver) = queued();
    world.crash_copy_awaits(0);

    let missing = error_of(call(&receiver, &queue, &world.mail, "restore.answer", json!({})).await);

    assert_eq!(missing.code(), ErrorCode::InvalidArgument);
    assert_eq!(
        world.restore.status().crash_copy,
        CrashCopyStatus::Awaiting,
        "stops at argument screening"
    );
}

// ── (B) Split ratio — split.setRatio · split.list (ADR-0227) ────────────────────

impl World {
    async fn ask(
        &self,
        receiver: &InboundReceiver,
        queue: &Queued,
        name: &str,
        args: serde_json::Value,
    ) -> CommandReply {
        self.mail.clear();
        call(receiver, queue, &self.mail, name, args).await
    }

    fn split_ids(&self, view: uuid::Uuid) -> Vec<uuid::Uuid> {
        apply::list_splits(&self.state, view)
            .expect("view")
            .into_iter()
            .map(|s| s.id)
            .collect()
    }

    /// Main tab split as `x | (y / z)` — returns (tab, x, y, z); preorder splits are the outer (left-right) then the inner (top-bottom).
    fn three_slots(&self) -> (uuid::Uuid, uuid::Uuid, uuid::Uuid, uuid::Uuid) {
        let view = self.main_tabs().active;
        let x = self.empty_slot(view);
        let y = apply::split_slot(&self.state, &Subs, &Events, view, x, SplitDir::LeftRight)
            .expect("left-right split");
        let z = apply::split_slot(&self.state, &Subs, &Events, view, y, SplitDir::TopBottom)
            .expect("top-bottom split");
        (view, x, y, z)
    }
}

fn metrics_with_min(min_pane_px: u32) -> UiMetrics {
    UiMetrics {
        frame_insets: Insets {
            t: 1.0,
            r: 1.0,
            b: 1.0,
            l: 1.0,
        },
        min_pane_px,
    }
}

const A_SIDE: &str = "the share of side a's (left/top) area";

#[test]
fn both_ratio_commands_define_the_ratio_as_the_a_side_share() {
    for name in ["split.setRatio", "split.list"] {
        let spec = spec_of(name).expect("is declared");
        assert!(
            spec.summary.contains(A_SIDE),
            "{name}: the summary lacks the meaning of ratio ({A_SIDE}): {}",
            spec.summary
        );
    }
    let list = spec_of("split.list").expect("is declared");
    assert!(list.summary.contains("a = left/top"), "{}", list.summary);
    // The summary is hand-written, so a changed constant would quietly lie to the LLM — measure with a string made from the constant.
    let bounds = format!("{}~{}", tree::RATIO_MIN, tree::RATIO_MAX);
    let set = spec_of("split.setRatio").expect("is declared");
    assert!(
        set.summary.contains(&bounds),
        "the split.setRatio summary must state the clamp range {bounds}: {}",
        set.summary
    );
}

#[test]
fn set_ratio_advertises_its_three_outcomes_by_name() {
    let spec = spec_of("split.setRatio").expect("is declared");
    let shape: serde_json::Value =
        serde_json::from_str(spec.ok_schema).expect("the schema is JSON");
    assert_eq!(
        shape["properties"]["outcome"]["enum"],
        json!(["Applied", "Unchanged", "TooSmall"]),
        "{shape}"
    );
    assert_eq!(shape["properties"]["ratio"]["type"], "number");
}

/// Both surfaces spell the outcome the same (same rule as `both_surfaces_spell_the_outcome_the_same_way`): `split.setRatio` serializes the macro's `RatioOutcome`, Tauri `set_split_ratio` the shell-internal `SplitRatioApplied.outcome` (to the screen via ts-rs). A serde rename on the shell side would diverge them, and the handler's exhaustive `match` catches only a missing branch, not the spelling.
#[test]
fn both_surfaces_spell_the_split_ratio_outcome_the_same_way() {
    let spec = spec_of("split.setRatio").expect("is declared");
    let shape: serde_json::Value =
        serde_json::from_str(spec.ok_schema).expect("the schema is JSON");
    let advertised = shape["properties"]["outcome"]["enum"]
        .as_array()
        .expect("enum list")
        .clone();

    let cases = [
        (SplitRatioOutcome::Applied, "Applied"),
        (SplitRatioOutcome::Unchanged, "Unchanged"),
        (SplitRatioOutcome::TooSmall, "TooSmall"),
    ];
    assert_eq!(advertised.len(), cases.len(), "{advertised:?}");
    for (outcome, expected) in cases {
        let payload = SplitRatioApplied {
            ratio: 0.5,
            outcome,
            version: 1,
        };
        let json = serde_json::to_value(payload).expect("serialization");
        assert_eq!(
            json["outcome"], expected,
            "the spelling on the Tauri answer side diverged: {json}"
        );
        assert!(
            advertised.contains(&json["outcome"]),
            "the values the command answer advertises lack {expected}: {advertised:?}"
        );
    }
}

#[tokio::test]
async fn set_ratio_writes_through_the_apply_service_and_clamps_out_of_range_values() {
    let (world, queue, receiver) = queued();
    let (view, x, _y, _z) = world.three_slots();
    let outer = world.split_ids(view)[0];

    let ok = world
        .ask(
            &receiver,
            &queue,
            "split.setRatio",
            json!({ "view_id": view.to_string(), "split_id": outer.to_string(), "ratio": 0.3 }),
        )
        .await
        .outcome
        .expect("apply");
    assert_eq!(ok, json!({ "ratio": 0.3, "outcome": "Applied" }));
    let layout = apply::get_view(&world.state, view).expect("view");
    let left = layout
        .slot_rects
        .iter()
        .find(|r| r.slot_id == x)
        .expect("x");
    assert_eq!(
        left.x1, 0.3,
        "0.3 becomes the share of the a (left) area as is"
    );

    for (asked, clamped) in [(1.5, tree::RATIO_MAX), (-1.0, tree::RATIO_MIN)] {
        let ok = world
            .ask(
                &receiver,
                &queue,
                "split.setRatio",
                json!({ "view_id": view.to_string(), "split_id": outer.to_string(), "ratio": asked }),
            )
            .await
            .outcome
            .expect("out of range is not an error");
        assert_eq!(
            ok,
            json!({ "ratio": clamped, "outcome": "Applied" }),
            "{asked}"
        );
    }

    let ok = world
        .ask(
            &receiver,
            &queue,
            "split.setRatio",
            json!({ "view_id": view.to_string(), "split_id": outer.to_string(), "ratio": 0.005 }),
        )
        .await
        .outcome
        .expect("already at the lower bound");
    assert_eq!(
        ok,
        json!({ "ratio": tree::RATIO_MIN, "outcome": "Unchanged" })
    );
}

#[tokio::test]
async fn set_ratio_refuses_malformed_ids_and_non_numbers_before_applying() {
    let (world, queue, receiver) = queued();
    let (view, _x, _y, _z) = world.three_slots();
    let split = world.split_ids(view)[0];
    let version = world.main_tabs().version;

    let cases = [
        (
            json!({ "view_id": "nope", "split_id": split.to_string(), "ratio": 0.3 }),
            Some("view_id"),
        ),
        (
            json!({ "view_id": view.to_string(), "split_id": "nope", "ratio": 0.3 }),
            Some("split_id"),
        ),
        // JSON cannot carry NaN · ±∞ — `json!` writes a non-finite number as null.
        (
            json!({ "view_id": view.to_string(), "split_id": split.to_string(), "ratio": f64::NAN }),
            None,
        ),
        (
            json!({ "view_id": view.to_string(), "split_id": split.to_string(), "ratio": f64::INFINITY }),
            None,
        ),
        (
            json!({ "view_id": view.to_string(), "split_id": split.to_string(), "ratio": "NaN" }),
            None,
        ),
        (
            json!({ "view_id": view.to_string(), "split_id": split.to_string(), "ratio": "Infinity" }),
            None,
        ),
    ];
    for (args, field) in cases {
        let err = error_of(
            world
                .ask(&receiver, &queue, "split.setRatio", args.clone())
                .await,
        );
        assert_eq!(err.code(), ErrorCode::InvalidArgument, "{args}");
        if let Some(field) = field {
            assert!(err.message().contains(field), "{args}: {}", err.message());
        }
    }

    let err = error_of(
        world
            .ask(
                &receiver,
                &queue,
                "split.setRatio",
                json!({ "view_id": view.to_string(), "split_id": "nope", "ratio": 0.3 }),
            )
            .await,
    );
    assert!(err.message().contains("split.list"), "{}", err.message());
    assert!(!err.message().contains("tab.list"), "{}", err.message());
    assert_eq!(
        world.main_tabs().version,
        version,
        "a rejection changes nothing"
    );
}

#[tokio::test]
async fn set_ratio_on_a_missing_split_keeps_the_services_reason() {
    let (world, queue, receiver) = queued();
    let (view, x, _y, _z) = world.three_slots();
    for missing in [uuid::Uuid::new_v4(), x] {
        let err = error_of(
            world
                .ask(
                    &receiver,
                    &queue,
                    "split.setRatio",
                    json!({ "view_id": view.to_string(), "split_id": missing.to_string(), "ratio": 0.3 }),
                )
                .await,
        );
        assert_eq!(err.code(), ErrorCode::Conflict);
        assert!(
            err.message().contains("split not found"),
            "{}",
            err.message()
        );
    }
}

#[tokio::test]
async fn set_ratio_is_clamped_by_the_px_minimum_once_the_window_reported_its_size() {
    let (world, queue, receiver) = queued();
    let (view, _x, _y, _z) = world.three_slots();
    let inner = world.split_ids(view)[1];
    // The box of the inner (top-bottom) split = the right half × canvas height 400px → with m = 100 the allowed range is [0.25, 0.75].
    apply::report_window_canvas(&world.state, MAIN_WINDOW_LABEL, 1000, 400).unwrap();
    apply::report_ui_metrics(&world.state, MAIN_WINDOW_LABEL, metrics_with_min(100)).unwrap();

    let ok = world
        .ask(
            &receiver,
            &queue,
            "split.setRatio",
            json!({ "view_id": view.to_string(), "split_id": inner.to_string(), "ratio": 0.1 }),
        )
        .await
        .outcome
        .expect("applied with clamping");
    assert_eq!(ok, json!({ "ratio": 0.25, "outcome": "Applied" }));

    // If the window is too small to give both sides the minimum, nothing is touched and TooSmall is carried (the current value as is).
    apply::report_window_canvas(&world.state, MAIN_WINDOW_LABEL, 1000, 150).unwrap();
    let ok = world
        .ask(
            &receiver,
            &queue,
            "split.setRatio",
            json!({ "view_id": view.to_string(), "split_id": inner.to_string(), "ratio": 0.5 }),
        )
        .await
        .outcome
        .expect("TooSmall is not an error");
    assert_eq!(ok, json!({ "ratio": 0.25, "outcome": "TooSmall" }));
}

#[tokio::test]
async fn split_list_names_the_slots_on_each_side_in_preorder() {
    let (world, queue, receiver) = queued();
    let (view, x, y, z) = world.three_slots();
    let ids = world.split_ids(view);
    apply::set_split_ratio(&world.state, &Events, view, ids[1], 0.3).unwrap();

    let ok = world
        .ask(
            &receiver,
            &queue,
            "split.list",
            json!({ "view_id": view.to_string() }),
        )
        .await
        .outcome
        .expect("list");
    assert_eq!(
        ok,
        json!({ "splits": [
            {
                "split_id": ids[0].to_string(),
                "dir": "LeftRight",
                "ratio": 0.5,
                "a_slots": [x.to_string()],
                "b_slots": [y.to_string(), z.to_string()],
            },
            {
                "split_id": ids[1].to_string(),
                "dir": "TopBottom",
                "ratio": 0.3,
                "a_slots": [y.to_string()],
                "b_slots": [z.to_string()],
            },
        ]})
    );

    // A tab with a single area gives an empty list · a malformed id is rejected · a nonexistent tab gets the apply service's reason.
    let fresh = apply::create_tab(&world.state, &Subs, &Events, MAIN_WINDOW_LABEL, None).unwrap();
    let ok = world
        .ask(
            &receiver,
            &queue,
            "split.list",
            json!({ "view_id": fresh.to_string() }),
        )
        .await
        .outcome
        .expect("list");
    assert_eq!(ok, json!({ "splits": [] }));
    let err = error_of(
        world
            .ask(
                &receiver,
                &queue,
                "split.list",
                json!({ "view_id": "nope" }),
            )
            .await,
    );
    assert_eq!(err.code(), ErrorCode::InvalidArgument);
    let err = error_of(
        world
            .ask(
                &receiver,
                &queue,
                "split.list",
                json!({ "view_id": uuid::Uuid::new_v4().to_string() }),
            )
            .await,
    );
    assert_eq!(err.code(), ErrorCode::Conflict);
    assert!(
        err.message().contains("view not found"),
        "{}",
        err.message()
    );
}

/// A split too deep to be representable goes out as `CONFLICT` + reason (without a panic or no response).
#[tokio::test]
async fn a_split_too_deep_to_represent_is_a_conflict() {
    let (world, queue, receiver) = queued();
    let view = world.main_tabs().active;
    let mut target = world.empty_slot(view);
    while let Ok(new) = apply::split_slot(
        &world.state,
        &Subs,
        &Events,
        view,
        target,
        SplitDir::LeftRight,
    ) {
        target = new;
    }
    let slots_before = world.slots(view);

    let err = error_of(
        world
            .ask(
                &receiver,
                &queue,
                "slot.split",
                json!({ "view_id": view.to_string(), "slot_id": target.to_string(), "dir": "LeftRight" }),
            )
            .await,
    );
    assert_eq!(err.code(), ErrorCode::Conflict);
    assert!(
        err.message().contains("too small to split further"),
        "{}",
        err.message()
    );
    assert_eq!(world.slots(view), slots_before, "tree unchanged");
}

// ── (B) Does not block the connection task ──────────────────────────────────────────────

/// `on_command` only queues and returns — if the application happened by then, the connection read loop was stalled that long.
#[tokio::test]
async fn on_command_returns_before_the_handler_runs() {
    let (world, queue, receiver) = queued();
    let before = world.main_tabs().version;

    let request_id = RequestId::new();
    receiver.on_command(
        envelope(
            "tab.create",
            json!({ "window": MAIN_WINDOW_LABEL }),
            request_id,
        ),
        world.mail.sink(request_id),
    );

    assert_eq!(queue.pending(), 1, "the application went out as a task");
    assert_eq!(
        world.main_tabs().version,
        before,
        "nothing is applied inside on_command"
    );
    assert_eq!(world.mail.len(), 0, "no reply yet either");

    run(&queue).await;
    assert!(world.main_tabs().version > before);
    assert!(world.mail.only().outcome.is_ok());
}

/// Self-deadlock regression (ADR-0081 · ADR-0155 decision 4). A synthetic command's spawn finishes only when the caller task services it — here the caller reads the spawn request channel and answers (in reality the connection loop reading the daemon reply). If `on_command` waited inline, both would wait for each other; the timeout below turns that into a failure instead of a hang.
#[tokio::test]
async fn a_composite_command_does_not_wait_on_its_caller() {
    let (mut world, ports) = World::build();
    let receiver = InboundReceiver::new(
        make_table(ports),
        Arc::new(RuntimeSpawner(tokio::runtime::Handle::current())) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    );

    let request_id = RequestId::new();
    receiver.on_command(
        envelope(
            "agent.spawnInto",
            json!({ "window": MAIN_WINDOW_LABEL, "cwd": "C:/work/engram" }),
            request_id,
        ),
        world.mail.sink(request_id),
    );

    let (cwd, _backend, answer) = tokio::time::timeout(
        Duration::from_secs(5),
        world.spawn_requests.recv(),
    )
    .await
    .expect("the caller task could not service the spawn request — the application runs inline (self-deadlock)")
    .expect("the spawn request comes");
    assert_eq!(cwd, "C:/work/engram");
    answer
        .send(Ok("agent-1".to_string()))
        .expect("puts in the spawn answer");

    world.mail.settle(1).await;
    let ok = world.mail.only().outcome.expect("placement succeeds");
    assert_eq!(ok["agent_id"], "agent-1");
}

/// A spawn failure the daemon answered with a code goes out with that code, not layered under `CONFLICT` (ADR-0291 R17) — a storage write failure (`INTERNAL:`) wrapped in `CONFLICT` would read as "nothing was applied". Text without a code prefix is `CONFLICT`.
#[tokio::test]
async fn spawn_into_keeps_the_code_the_daemon_answered_with() {
    for (answered, code, message) in [
        (
            "CONFLICT: the agent was not created or started: agents.json could not be read just now",
            ErrorCode::Conflict,
            "the agent was not created or started: agents.json could not be read just now",
        ),
        (
            "INTERNAL: the agent was not created or started: writing agents.json failed: disk full",
            ErrorCode::Internal,
            "the agent was not created or started: writing agents.json failed: disk full",
        ),
        (
            "the backend field is empty",
            ErrorCode::Conflict,
            "the backend field is empty",
        ),
    ] {
        let (mut world, ports) = World::build();
        let receiver = InboundReceiver::new(
            make_table(ports),
            Arc::new(RuntimeSpawner(tokio::runtime::Handle::current())) as Arc<dyn TaskSpawner>,
            CATALOG_VERSION,
        );
        let request_id = RequestId::new();
        receiver.on_command(
            envelope(
                "agent.spawnInto",
                json!({ "window": MAIN_WINDOW_LABEL, "cwd": "C:/work/engram", "backend": "claude" }),
                request_id,
            ),
            world.mail.sink(request_id),
        );
        let (_cwd, _backend, answer) =
            tokio::time::timeout(Duration::from_secs(5), world.spawn_requests.recv())
                .await
                .expect("the spawn request comes")
                .expect("the spawn request comes");
        answer
            .send(Err(answered.to_string()))
            .expect("puts in the spawn answer");

        world.mail.settle(1).await;
        let err = world
            .mail
            .only()
            .outcome
            .expect_err("the spawn failed");
        assert_eq!(err.code(), code, "{answered}");
        assert_eq!(err.message(), message, "{answered}");
    }
}

/// This door skips the common registration part of `agent.new`, so the same spelling rules are measured here: backend words by the wire enum (`AgentBackendKind`), cwd by `normalize_spelling`.
#[tokio::test]
async fn spawn_into_absorbs_the_spelling_a_person_would_type() {
    for (word, want) in [
        ("codex", engram_dashboard_protocol::AgentBackendKind::Codex),
        ("Codex", engram_dashboard_protocol::AgentBackendKind::Codex),
        ("CODEX", engram_dashboard_protocol::AgentBackendKind::Codex),
        (
            "Claude",
            engram_dashboard_protocol::AgentBackendKind::Claude,
        ),
        (
            "CLAUDE",
            engram_dashboard_protocol::AgentBackendKind::Claude,
        ),
    ] {
        let (mut world, ports) = World::build();
        let receiver = InboundReceiver::new(
            make_table(ports),
            Arc::new(RuntimeSpawner(tokio::runtime::Handle::current())) as Arc<dyn TaskSpawner>,
            CATALOG_VERSION,
        );

        let request_id = RequestId::new();
        receiver.on_command(
            envelope(
                "agent.spawnInto",
                json!({
                    "window": MAIN_WINDOW_LABEL,
                    "cwd": r#""C:\work\engram""#,
                    "backend": word,
                }),
                request_id,
            ),
            world.mail.sink(request_id),
        );

        let (cwd, backend, answer) =
            tokio::time::timeout(Duration::from_secs(5), world.spawn_requests.recv())
                .await
                .unwrap_or_else(|_| panic!("{word}: did not get as far as the spawn"))
                .expect("the spawn request comes");
        assert_eq!(
            cwd, "C:/work/engram",
            "{word}: the cwd spelling was not carried over"
        );
        assert_eq!(backend, Some(want), "{word}: read as a different backend");
        answer
            .send(Ok("agent-1".to_string()))
            .expect("puts in the spawn answer");

        world.mail.settle(1).await;
        world.mail.only().outcome.expect("placement succeeds");
    }
}

#[tokio::test]
async fn spawn_into_still_refuses_a_backend_word_it_does_not_know() {
    let (mut world, ports) = World::build();
    let receiver = InboundReceiver::new(
        make_table(ports),
        Arc::new(RuntimeSpawner(tokio::runtime::Handle::current())) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    );

    let request_id = RequestId::new();
    receiver.on_command(
        envelope(
            "agent.spawnInto",
            json!({ "window": MAIN_WINDOW_LABEL, "cwd": "C:/work/engram", "backend": "codx" }),
            request_id,
        ),
        world.mail.sink(request_id),
    );

    world.mail.settle(1).await;
    error_of(world.mail.only());
    assert!(
        world.spawn_requests.try_recv().is_err(),
        "a spawn went out with a rejected word"
    );
}

// ── (B) Does not die by blowing up ───────────────────────────────────────────────────

/// A command handler returns an error as a value instead of dying by panic (TRD §4-⑨). The net works only in a test build without the hook (production confines nothing; the panic hook ends it — ADR-0292).
#[tokio::test]
async fn a_panicking_handler_answers_instead_of_taking_the_process_down() {
    let mut table = CommandTable::new(COMMAND_SPECS);
    table
        .insert(
            "window.list",
            blocking_handler(
                |_: WindowListArgs| -> Result<serde_json::Value, CommandError> {
                    panic!("handler blew up")
                },
            ),
        )
        .expect("a declared name");
    let queue = Arc::new(Queued::default());
    let receiver = InboundReceiver::new(
        table,
        Arc::clone(&queue) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    );
    let mail = Mailbox::default();

    let reply = quiet_panics(call(&receiver, &queue, &mail, "window.list", json!({}))).await;

    assert_eq!(error_of(reply).code(), ErrorCode::Internal);
}

// ── (B) The entry point the connection loop actually calls ──────────────────────────────────────

#[tokio::test]
async fn accept_correlates_the_reply_to_the_envelope_it_was_given() {
    let (world, queue, receiver) = queued();
    let request_id = RequestId::new();

    receiver.accept(
        envelope(
            "tab.list",
            json!({ "window": MAIN_WINDOW_LABEL }),
            request_id,
        ),
        world.mail.deliver(),
    );
    run(&queue).await;

    let reply = world.mail.only();
    assert_eq!(reply.request_id, request_id);
    let ok = reply.outcome.expect("query succeeds");
    assert_eq!(ok["window"], MAIN_WINDOW_LABEL);
}

struct Gated {
    gate: Arc<Semaphore>,
}

impl CommandHandler for Gated {
    fn call(&self, _args: serde_json::Value) -> CommandFuture {
        let gate = Arc::clone(&self.gate);
        Box::pin(async move {
            let _permit = gate
                .acquire()
                .await
                .map_err(|e| CommandError::internal(e.to_string()))?;
            Ok(json!({ "windows": [] }))
        })
    }
}

/// The read loop does not queue behind a slow handler: while the earlier command hangs, it keeps receiving and passing on the next frames (inline would stop at the first `accept`, head-of-line blocking output · status events).
#[tokio::test]
async fn a_slow_handler_does_not_stall_the_read_loop() {
    let gate = Arc::new(Semaphore::new(0));
    let mut table = CommandTable::new(COMMAND_SPECS);
    table
        .insert(
            "window.list",
            Arc::new(Gated {
                gate: Arc::clone(&gate),
            }),
        )
        .expect("a declared name");
    let receiver = InboundReceiver::new(
        table,
        Arc::new(RuntimeSpawner(tokio::runtime::Handle::current())) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    );
    let mail = Mailbox::default();

    let sent: Vec<RequestId> = (0..5).map(|_| RequestId::new()).collect();
    for request_id in &sent {
        receiver.accept(
            envelope("window.list", json!({}), *request_id),
            mail.deliver(),
        );
    }
    assert_eq!(
        mail.len(),
        0,
        "nothing finished while all five frames were received — meaning the loop did not wait for the application"
    );

    gate.add_permits(5);
    mail.settle(5).await;
    assert_eq!(
        mail.request_ids(),
        sent.iter()
            .map(|id| id.to_string())
            .collect::<BTreeSet<_>>(),
        "the correlation keys of the five round trips are not mixed"
    );
}

// ── (B) The byte path the connection drive walks ────────────────────────────────────────
//
// The driver's daemon-command arm rebuilt from its real parts: the daemon frame decodes as `AgentEvent`, the envelope is unwrapped by the same edge function, the receiver gets the outcome writer the driver hands it (`link::route::send_command_outcome`), and the outcome is read back as the frame written on a memory link. Delete any of those functions and these tests do not compile.
//
// Uncovered (canonical here):
// - The driver choosing this arm, its skip rules and no-table answer (`connection.rs` `on_driver_event` · `accept_inbound`) — driven on a memory network in `daemon_client::connection::tests`.
// - The registration's send (`register_commands`) — what it carries is covered by the `registration_command` tests.

struct WireTrip {
    world: World,
    reply: serde_json::Value,
    request_id: RequestId,
}

/// A live connection on the shell wire over a memory network — `endpoint` is the daemon's side; the other fields keep the link alive (a dropped inbox ends the peer).
struct LiveLink {
    peer: Peer<ShellWire>,
    endpoint: Arc<MemoryEndpoint>,
    _inbox: engram_dashboard_transport::Inbox<ShellWire>,
    _registry: Registry<ShellWire>,
    _net: MemoryNetwork,
    _clock: ManualClock,
}

async fn live_link() -> LiveLink {
    let net = MemoryNetwork::new();
    let clock = ManualClock::new();
    let registry = Registry::new(
        Arc::new(ShellWire),
        net.dialer(),
        clock.handle(),
        Policy::default(),
    );
    let (peer, inbox) = registry.add(
        PeerId::new("daemon"),
        Address::new("mem"),
        Arc::new(ImmediateHandshake),
    );
    settle_until("live", || peer.peer_state() == PeerState::Live).await;
    let endpoint = net.accept().expect("a link was opened");
    LiveLink {
        peer,
        endpoint,
        _inbox: inbox,
        _registry: registry,
        _net: net,
        _clock: clock,
    }
}

async fn wire_round_trip(name: &str, args: serde_json::Value) -> WireTrip {
    let (world, ports) = World::build();
    let queue = Arc::new(Queued::default());
    let receiver = InboundReceiver::new(
        make_table(ports),
        Arc::clone(&queue) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    );
    let link = live_link().await;

    let request_id = RequestId::new();
    let sent = serde_json::to_string(&AgentEvent::CommandRequest {
        envelope: serde_json::value::to_raw_value(&envelope(name, args, request_id))
            .expect("envelope serialization"),
    })
    .expect("a Text frame the daemon would send");

    // This frame is not a reply the shell waits for (a pending slot would swallow it).
    let ev: AgentEvent = serde_json::from_str(&sent).expect("decodes as AgentEvent");
    assert_eq!(
        event_reply_request_id(&ev),
        None,
        "if the inbound request is read as a reply, that command is not executed and vanishes"
    );
    let AgentEvent::CommandRequest { envelope } = ev else {
        panic!("must branch to the CommandRequest arm");
    };
    // ADR-0272 decision 4: the driver unwraps the envelope with this same edge function.
    let envelope = envelope_from_wire(&envelope).expect("an envelope the daemon built unpacks");

    let peer = link.peer.clone();
    let generation = peer.generation();
    receiver.accept(envelope, move |reply| {
        send_command_outcome(&peer, generation, &reply)
    });
    run(&queue).await;

    let mut frames = Vec::new();
    settle_until("the outcome is written", || {
        frames.extend(link.endpoint.frames());
        !frames.is_empty()
    })
    .await;
    assert_eq!(frames.len(), 1, "{frames:?}");
    let Frame::Text(text) = frames.remove(0) else {
        panic!("the outcome is a Text frame");
    };
    let outcome: AgentCommand =
        serde_json::from_str(&text).expect("the outcome frame is an AgentCommand");
    assert_eq!(
        command_request_id(&outcome),
        None,
        "if sending a reply creates a pending slot, there is no counterpart to wake"
    );
    let back: serde_json::Value = serde_json::from_str(&text).expect("the outcome is JSON");
    WireTrip {
        world,
        reply: back["CommandOutcome"]["reply"].clone(),
        request_id,
    }
}

#[tokio::test]
async fn a_daemon_frame_reaches_the_service_and_the_answer_goes_back_correlated() {
    let WireTrip {
        world,
        reply,
        request_id,
    } = wire_round_trip("tab.create", json!({ "window": MAIN_WINDOW_LABEL })).await;

    assert_eq!(reply["request_id"], request_id.to_string());
    let view_id = reply["outcome"]["Ok"]["view_id"]
        .as_str()
        .expect("the success outcome carries the new tab id");
    let tabs = world.main_tabs();
    assert_eq!(
        tabs.tabs.len(),
        2,
        "the apply service actually made the tab"
    );
    assert_eq!(view_id, tabs.active.to_string());
}

#[tokio::test]
async fn an_unknown_name_from_the_daemon_comes_back_as_a_typed_error() {
    let WireTrip {
        reply, request_id, ..
    } = wire_round_trip("tab.teleport", json!({})).await;

    assert_eq!(reply["request_id"], request_id.to_string());
    assert_eq!(
        reply["outcome"]["Err"]["code"], "UNKNOWN_COMMAND",
        "the code must cross the wire so the caller branches on the code, not the text"
    );
    assert_eq!(reply["outcome"]["Err"]["retry"], "never");
}

// ── (B) Registration packet · late-filled slot ──────────────────────────────────────

/// Measures the packet that actually goes out: the driver (`register_commands`) sends exactly what `registration_command` returns (delete it and this does not compile); a hand-rebuilt packet would stay green when the two drift.
#[test]
fn the_registration_packet_is_the_one_the_connection_sends() {
    let (_world, ports) = World::build();
    let table = make_table(ports);
    let plugged: Vec<&str> = table.specs().map(|s| s.name).collect();
    let receiver = InboundReceiver::new(
        table,
        Arc::new(Queued::default()) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    );

    let Some(AgentCommand::RegisterCommands {
        owner,
        decls,
        catalog_version,
        ..
    }) = registration_command(&receiver)
    else {
        panic!("with names to put on, a registration packet comes out");
    };
    let decls = decls_from_wire(&decls).expect("the registration field the shell wrapped unpacks");
    assert_eq!(
        decls.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
        plugged
    );
    for decl in &decls {
        assert!(!decl.help.trim().is_empty(), "{}: help is empty", decl.name);
    }
    assert_eq!(catalog_version, CATALOG_VERSION);
    assert!(!owner.as_str().is_empty());
    assert!(
        !owner.as_str().starts_with("conn-"),
        "the advertised token must not imitate the daemon-derived token form: {owner}"
    );

    // With nothing to put on there is no packet — an empty registration costs the daemon roster a meaningless round trip.
    let empty = InboundReceiver::new(
        CommandTable::new(COMMAND_SPECS),
        Arc::new(Queued::default()) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    );
    assert!(registration_command(&empty).is_none());
}

/// The advertisement is what is plugged in, not the declaration — for a table made from the same declaration set with only one plugged in, the advertisement must be one; a name not in the declaration is refused by the table (advertisement ⊆ declaration). Otherwise an uncallable name lands on the roster and the daemon's envelope returns `UNKNOWN_COMMAND`.
#[test]
fn advertising_follows_what_is_plugged_not_what_is_declared() {
    assert!(
        COMMAND_SPECS.len() > 1,
        "with only one declaration this distinction cannot be measured"
    );
    let mut table = CommandTable::new(COMMAND_SPECS);
    table
        .insert(
            "window.list",
            blocking_handler(
                |_: WindowListArgs| -> Result<serde_json::Value, CommandError> {
                    Ok(json!({ "windows": [] }))
                },
            ),
        )
        .expect("a declared name");
    let receiver = InboundReceiver::new(
        table,
        Arc::new(Queued::default()) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    );
    assert_eq!(
        receiver
            .declarations()
            .iter()
            .map(|d| d.name.as_str())
            .collect::<Vec<_>>(),
        vec!["window.list"],
        "only the one plugged in is advertised ({} declarations)",
        COMMAND_SPECS.len()
    );

    let mut table = CommandTable::new(COMMAND_SPECS);
    let refused = table
        .insert(
            "tab.teleport",
            blocking_handler(
                |_: WindowListArgs| -> Result<serde_json::Value, CommandError> { Ok(json!({})) },
            ),
        )
        .expect_err("a name outside the declaration set is not plugged in");
    assert_eq!(refused, TableError::NotDeclared("tab.teleport"));
}

/// The slot must show the empty state (an envelope to that window gets an error reply from the call site), and only the first install wins (else it is unknowable which table runs).
#[test]
fn an_empty_slot_is_visible_and_the_first_install_wins() {
    let slot = InboundSlot::new();
    assert!(slot.get().is_none(), "the unplugged state is visible");

    let first = {
        let (_world, ports) = World::build();
        Arc::new(InboundReceiver::new(
            make_table(ports),
            Arc::new(Queued::default()) as Arc<dyn TaskSpawner>,
            CATALOG_VERSION,
        ))
    };
    let second = {
        let (_world, ports) = World::build();
        Arc::new(InboundReceiver::new(
            make_table(ports),
            Arc::new(Queued::default()) as Arc<dyn TaskSpawner>,
            CATALOG_VERSION,
        ))
    };
    slot.set(Arc::clone(&first));
    slot.set(Arc::clone(&second));

    assert!(Arc::ptr_eq(slot.get().expect("is plugged in"), &first));
}

// ── (V) Webview part — proxy registration and the last hop ─────────────────────────────────────
//
// Measured with zero windows: only the real delivery (`AppHandle::emit_to`) is a fake (`RecordingDispatch`); the reserved-name filter, registration merge, stage 2 of the 3-stage delivery, reply correlation and deadline are real code. Delete those functions and this section does not compile.
//
// Uncovered:
// - `TauriViewDispatch::emit_to` itself (needs a real window) and the webview-side listener (`src/commands/viewCommandBridge.ts` — vitest measures only the projection).
// - The `report_view_commands` · `report_command_outcome` invoke wrappers (need Tauri `State`/`WebviewWindow`); their decisions are all in the `ViewCommandBridge` methods the section below rides.

/// A delivery that only records the envelope (the real `emit_to` needs a window). A label in `dead` is a closed window, yet `deliver` still succeeds — production's `emit_to` gives `Ok` for a nonexistent label, which is why a separate liveness lookup exists.
struct RecordingDispatch {
    seen: mpsc::UnboundedSender<(String, ViewCommandRequest)>,
    dead: Mutex<BTreeSet<String>>,
}

impl ViewDispatch for RecordingDispatch {
    fn deliver(&self, target: &str, request: &ViewCommandRequest) -> Result<(), String> {
        self.seen
            .send((target.to_string(), request.clone()))
            .map_err(|_| "the test record channel is closed".to_string())
    }

    fn is_alive(&self, label: &str) -> bool {
        !self.dead.lock().unwrap().contains(label)
    }
}

fn recording_bridge(
    deadline: Duration,
) -> (
    Arc<ViewCommandBridge>,
    mpsc::UnboundedReceiver<(String, ViewCommandRequest)>,
) {
    let (bridge, rx, _dispatch) = recording_bridge_with_windows(deadline);
    (bridge, rx)
}

fn recording_bridge_with_windows(
    deadline: Duration,
) -> (
    Arc<ViewCommandBridge>,
    mpsc::UnboundedReceiver<(String, ViewCommandRequest)>,
    Arc<RecordingDispatch>,
) {
    let (seen, rx) = mpsc::unbounded_channel();
    let dispatch = Arc::new(RecordingDispatch {
        seen,
        dead: Mutex::new(BTreeSet::new()),
    });
    let bridge = Arc::new(ViewCommandBridge::with_reserved(
        Arc::clone(&dispatch) as Arc<dyn ViewDispatch>,
        deadline,
        // Use the real reserved set — a hand-written list would make this measure "the list I wrote" and never signal when the vocabulary grows.
        reserved_names(),
    ));
    (bridge, rx, dispatch)
}

impl RecordingDispatch {
    fn close(&self, label: &str) {
        self.dead.lock().unwrap().insert(label.to_string());
    }
}

fn view_decl(name: &str) -> ViewCommandDecl {
    view_decl_with_effect(name, Some(ViewEffect::Write))
}

fn view_decl_with_effect(name: &str, effect: Option<ViewEffect>) -> ViewCommandDecl {
    ViewCommandDecl {
        name: name.to_string(),
        help: ViewCommandHelp {
            summary: format!("what {name} does"),
            effect,
            args: BTreeMap::new(),
            required: Vec::new(),
        },
    }
}

/// A receiver bearing the webview part on a real runtime (the round trip ends only when the reply comes into another task, so `Queued` cannot measure it).
fn with_view(bridge: Arc<ViewCommandBridge>) -> (World, Arc<InboundReceiver>) {
    let (world, ports) = World::build();
    let receiver = Arc::new(InboundReceiver::with_view(
        make_table(ports),
        Arc::new(RuntimeSpawner(tokio::runtime::Handle::current())) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
        bridge,
    ));
    (world, receiver)
}

/// A shell table name is carried once, from the table, and names the daemon answers are not filtered by the shell (ADR-0270 decision 2). Hypothetical: the webview reports every shell table name, its own part (`tab.next`) and a daemon-answered name (`agent.spawn`) (today it reports only names with `help`).
/// - Shell table names are reserved and drop out of the webview part; the packet carries them once from the table (the roster folds the same name in one packet without error, so carrying twice would silently diverge which owner wins).
/// - Daemon-answered names are carried — that wall is the daemon's registration refusal (the shell does not know that vocabulary): `registering_a_name_this_daemon_answers_is_refused_as_a_conflict` in the daemon's `connection_core.rs`.
/// The expected shell-table side is computed from every shell declaration, so it grows with the vocabulary.
#[tokio::test]
async fn the_registration_packet_carries_shell_names_once_and_leaves_daemon_names_to_the_daemon() {
    const DAEMON_NAME: &str = "agent.spawn";
    let (bridge, _seen) = recording_bridge(Duration::from_secs(1));
    let shell_names: BTreeSet<&str> = COMMAND_SPECS.iter().map(|spec| spec.name).collect();
    assert!(
        !shell_names.contains(DAEMON_NAME),
        "premise of this test — the shell table does not declare {DAEMON_NAME}"
    );

    let reported: Vec<ViewCommandDecl> = shell_names
        .iter()
        .map(|name| view_decl(name))
        .chain([view_decl("tab.next"), view_decl(DAEMON_NAME)])
        .collect();
    let outcome = bridge.report(MAIN_WINDOW_LABEL, reported);
    let refused: BTreeSet<&str> = outcome.refused.iter().map(String::as_str).collect();
    assert_eq!(
        refused, shell_names,
        "what drops out of the webview part is all the shell table names and nothing else"
    );

    let (_world, receiver) = with_view(Arc::clone(&bridge));
    let Some(AgentCommand::RegisterCommands { decls, .. }) = registration_command(&receiver) else {
        panic!("with names to put on, a registration packet comes out");
    };
    let decls = decls_from_wire(&decls).expect("the registration field the shell wrapped unpacks");
    for name in &shell_names {
        let carried: Vec<_> = decls.iter().filter(|d| d.name == *name).collect();
        assert_eq!(
            carried.len(),
            1,
            "{name}: a shell table name is carried only once"
        );
        assert!(
            !carried[0].help.contains(&view_decl(name).help.summary),
            "{name}: what is carried must be the shell table's declaration — the description the webview reported was carried"
        );
    }
    let names: BTreeSet<&str> = decls.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains("tab.next"), "the webview part is carried");
    assert!(
        names.contains(DAEMON_NAME),
        "names the daemon answers are not filtered by the shell — the wall is the daemon's registration refusal"
    );
    for decl in &decls {
        assert!(!decl.help.trim().is_empty(), "{}: help is empty", decl.name);
    }
}

/// When the destination window closes, a living reporter takes over — without recovery every later delivery goes to the dead label, waits for the deadline (`TIMEOUT`) and the name stays on the roster "present but never working" for the whole connection.
/// Path: main's single-shot report fails (no retry on that invoke), a pop-out becomes host, then closes. A `deliver` failure cannot catch this — the fake gives `Ok` to a dead label like production.
#[tokio::test]
async fn a_closed_host_hands_the_destination_to_a_live_reporter() {
    let (bridge, mut seen, windows) = recording_bridge_with_windows(Duration::from_secs(5));
    bridge.report("popup-1", vec![view_decl("tab.next")]);
    bridge.report("popup-2", vec![view_decl("tab.next")]);
    assert_eq!(
        bridge.host().as_deref(),
        Some("popup-1"),
        "the window that came first is the destination"
    );

    windows.close("popup-1");

    assert_eq!(
        bridge.host().as_deref(),
        Some("popup-2"),
        "a living reporter takes over the dead host"
    );
    let (_world, receiver) = with_view(Arc::clone(&bridge));
    let mail = Mailbox::default();
    let request_id = RequestId::new();
    receiver.accept(
        envelope("tab.next", json!({ "window": "main" }), request_id),
        mail.deliver(),
    );
    let (target, request) = seen.recv().await.expect("goes down to a living window");
    assert_eq!(target, "popup-2");
    bridge
        .settle("popup-2", &request.request_id, Ok(json!({ "ok": true })))
        .expect("the window that took over answers");
    mail.settle(1).await;
    assert_eq!(mail.only().outcome, Ok(json!({ "ok": true })));

    windows.close("popup-2");
    assert_eq!(bridge.host(), None);
    assert!(bridge.declarations().is_empty());
}

/// The advertised list and the window receiving the envelope come from the same window — if they diverge, A runs while the daemon advertises B (`UNKNOWN_COMMAND`, or an executed name not advertised). Every window uploads the same static `contributions` today, so a deliberately different list is reported here.
#[tokio::test]
async fn a_non_host_report_does_not_change_what_is_advertised() {
    let (bridge, mut seen, windows) = recording_bridge_with_windows(Duration::from_secs(5));
    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("tab.next")]);
    assert_eq!(bridge.host().as_deref(), Some(MAIN_WINDOW_LABEL));

    let popup = bridge.report("popup-1", vec![view_decl("slot.empty")]);

    assert!(
        !popup.changed(),
        "a report from a non-host window makes no diff"
    );
    let advertised: Vec<String> = bridge.declarations().into_iter().map(|d| d.name).collect();
    assert_eq!(
        advertised,
        vec!["tab.next".to_string()],
        "the advertisement is the host's only"
    );

    let (_world, receiver) = with_view(Arc::clone(&bridge));
    let mail = Mailbox::default();
    let request_id = RequestId::new();
    receiver.accept(
        envelope("slot.empty", json!({}), request_id),
        mail.deliver(),
    );
    mail.settle(1).await;
    assert_eq!(error_of(mail.only()).code(), ErrorCode::UnknownCommand);
    assert!(seen.try_recv().is_err());

    windows.close(MAIN_WINDOW_LABEL);
    assert_eq!(bridge.host().as_deref(), Some("popup-1"));
    assert_eq!(
        bridge
            .declarations()
            .into_iter()
            .map(|d| d.name)
            .collect::<Vec<_>>(),
        vec!["slot.empty".to_string()]
    );
}

/// A report's state change and the sending of its diff are one unit — if split, a later report's diff goes first and the daemon keeps the union of both (the bridge knows only the later, so an old name's delivery gets "unknown" from the webview).
/// Measured: hold the first send inside the gate and put in the second report; without the gate the order flips, with it the second cannot start until the first send is released.
#[tokio::test]
async fn a_report_and_its_delta_go_out_as_one_unit() {
    let (bridge, _seen) = recording_bridge(Duration::from_secs(5));
    let order = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let gate = Arc::new(Semaphore::new(0));

    let first = {
        let (bridge, order, gate) = (Arc::clone(&bridge), Arc::clone(&order), Arc::clone(&gate));
        tokio::spawn(async move {
            bridge
                .report_and_push(
                    MAIN_WINDOW_LABEL,
                    vec![view_decl("tab.next")],
                    |_, _| async move {
                        order.lock().unwrap().push("first-send-begin");
                        let _ = gate.acquire().await.expect("gate");
                        order.lock().unwrap().push("first-send-end");
                    },
                )
                .await;
        })
    };
    while order.lock().unwrap().is_empty() {
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    let second = {
        let (bridge, order) = (Arc::clone(&bridge), Arc::clone(&order));
        tokio::spawn(async move {
            bridge
                .report_and_push(
                    MAIN_WINDOW_LABEL,
                    vec![view_decl("slot.empty")],
                    |_, _| async move {
                        order.lock().unwrap().push("second-send");
                    },
                )
                .await;
        })
    };

    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        *order.lock().unwrap(),
        vec!["first-send-begin"],
        "the second report overtook the first send — the daemon is left holding the union"
    );
    // Tells "has not started" from "is blocked" — the earlier assertion alone would pass for a merely sluggish task.
    assert!(
        !second.is_finished(),
        "the second report finished without passing the gate"
    );

    gate.add_permits(1);
    first.await.expect("first report");
    second.await.expect("second report");
    assert_eq!(
        *order.lock().unwrap(),
        vec!["first-send-begin", "first-send-end", "second-send"]
    );
}

/// Measures whether Rust reads the text TypeScript actually prints — no other test crosses this boundary (Rust builds the enum directly, vitest mocks invoke). A spelling divergence makes serde fail the whole vector, `report_view_commands` rejects the payload wholesale, and the window registers zero commands with only a console warning.
/// The first string is exactly what `offeredCommands()` of `src/commands/viewCommandBridge.ts` emits (paired: `viewCommandBridge.test.ts` invoke arguments, `busCommands.test.ts` list · effect vocabulary).
/// The second is a shape no command emits today (the theme command, the last sender of `enum` and `read`, was removed — ADR-0167), but those fields are a contract, so keep measuring them.
#[test]
fn the_frontend_spelling_of_the_report_payload_deserializes_here() {
    const FROM_WEBVIEW: &str = r#"[
      {"name":"slot.empty","help":{
        "summary":"Returns that slot to an empty area (the slot itself stays — to remove it, slot.close).",
        "effect":"write",
        "args":{"viewId":{"type":"string","description":"Tab id (UUID) — given by tab.list."},
                "slotId":{"type":"string","description":"Slot id (UUID) — given by slot.resolveSpatial."}},
        "required":["viewId","slotId"]}},
      {"name":"tab.next","help":{"summary":"moves that window's active tab to the next tab.","effect":"write"}}
    ]"#;

    let decls: Vec<ViewCommandDecl> =
        serde_json::from_str(FROM_WEBVIEW).expect("reads exactly the shape the frontend prints");

    assert_eq!(decls.len(), 2);
    assert_eq!(decls[0].help.effect, Some(ViewEffect::Write));
    let slot = decls[0].help.args.get("slotId").expect("argument field");
    assert_eq!(slot.ty.as_deref(), Some("string"));
    assert_eq!(
        decls[0].help.required,
        vec!["viewId".to_string(), "slotId".to_string()]
    );
    assert!(decls[1].help.args.is_empty());

    const NOT_EMITTED_TODAY: &str = r#"[{"name":"probe.shape","help":{"summary":"s","effect":"read","args":{"pick":{"type":"string","enum":["a","b","c"]}}}}]"#;
    let shapes: Vec<ViewCommandDecl> =
        serde_json::from_str(NOT_EMITTED_TODAY).expect("a shape in the contract is read");
    assert_eq!(shapes[0].help.effect, Some(ViewEffect::Read));
    assert_eq!(
        shapes[0].help.args["pick"]
            .allowed
            .as_deref()
            .map(<[String]>::len),
        Some(3)
    );

    // The uppercase spelling is not read — pins the rename direction. One broken item fails the whole vector, i.e. that window registers zero.
    let wrong = r#"[{"name":"tab.next","help":{"summary":"s","effect":"Write"}}]"#;
    assert!(
        serde_json::from_str::<Vec<ViewCommandDecl>>(wrong).is_err(),
        "if the spelling diverges the payload is rejected wholesale — that window cannot register a single command"
    );
}

/// The webview deadline must be inside the daemon seat deadline — if reversed: (1) this side's deadline becomes unreachable (the daemon withdraws the seat first and answers `TIMEOUT`/`retry: never`; the late outcome is discarded as `NoSeat`, so `retry: same-request-id` asserted by the timeout test below can never occur), (2) the same command runs twice (a re-call with the same id after the daemon deadline sends a second envelope over the running first one).
/// Only this file can bite the relation: the daemon crate is a dev dependency, so production code cannot see that constant — the value is pinned in the shell and the inequality measured here.
#[test]
fn the_webview_deadline_fits_inside_the_daemon_seat() {
    let seat = engram_dashboard_daemon::command_delivery::CommandDeliveries::DEFAULT_DEADLINE;

    assert!(
        VIEW_REPLY_DEADLINE + VIEW_HOP_MARGIN <= seat,
        "webview deadline ({VIEW_REPLY_DEADLINE:?}) + hop margin ({VIEW_HOP_MARGIN:?}) exceeds the daemon seat deadline ({seat:?}) \
         — if this order is reversed the shell's TIMEOUT becomes unreachable and the same command can run twice"
    );
    assert!(
        !VIEW_HOP_MARGIN.is_zero(),
        "if the margin term is 0 the inequality becomes prose — a claim that the two daemon↔shell hops are free"
    );
}

/// A number already running is not sent — covering it silently would run the side effect twice and attach the old outcome to the new attempt. The inequality above prevents this, but there must be two nets.
#[tokio::test]
async fn a_duplicate_request_id_is_refused_instead_of_displacing_the_live_waiter() {
    let (bridge, mut seen) = recording_bridge(Duration::from_secs(60));
    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("tab.next")]);

    let (_world, receiver) = with_view(Arc::clone(&bridge));
    let mail = Mailbox::default();
    let request_id = RequestId::new();
    receiver.accept(
        envelope("tab.next", json!({ "window": "main" }), request_id),
        mail.deliver(),
    );
    let (_target, first) = seen.recv().await.expect("the first envelope goes down");

    let second = Mailbox::default();
    receiver.accept(
        envelope("tab.next", json!({ "window": "slot-popup-1" }), request_id),
        second.deliver(),
    );
    second.settle(1).await;

    assert_eq!(
        error_of(second.only()).code(),
        ErrorCode::RequestIdConflict,
        "the second attempt is refused"
    );
    assert!(
        seen.try_recv().is_err(),
        "a second envelope did not go down to the webview — if it had, the command would run twice"
    );

    bridge
        .settle(
            MAIN_WINDOW_LABEL,
            &first.request_id,
            Ok(json!({ "first": true })),
        )
        .expect("the first seat remains");
    mail.settle(1).await;
    assert_eq!(mail.only().outcome, Ok(json!({ "first": true })));
}

/// A window that did not receive the envelope cannot end that round trip — with only one correlation key someone else's window could forge an outcome the caller accepts while the real side effect still happens. The seat also stays on a failed match (removing it would let one forgery erase the real answer's seat).
#[tokio::test]
async fn only_the_window_that_received_the_envelope_may_settle_it() {
    let (bridge, mut seen) = recording_bridge(Duration::from_secs(60));
    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("tab.next")]);

    let (_world, receiver) = with_view(Arc::clone(&bridge));
    let mail = Mailbox::default();
    let request_id = RequestId::new();
    receiver.accept(
        envelope("tab.next", json!({ "window": "main" }), request_id),
        mail.deliver(),
    );
    let (_target, request) = seen.recv().await.expect("the envelope goes down");

    let forged = bridge
        .settle(
            "popup-9",
            &request.request_id,
            Ok(json!({ "forged": true })),
        )
        .expect_err("someone else's window cannot end it");
    assert!(forged.contains("popup-9"), "says who answered: {forged}");
    assert_eq!(mail.len(), 0, "a forgery cannot produce any reply");

    bridge
        .settle(
            MAIN_WINDOW_LABEL,
            &request.request_id,
            Ok(json!({ "real": true })),
        )
        .expect("the real window's seat was not erased by the forgery");
    mail.settle(1).await;
    assert_eq!(mail.only().outcome, Ok(json!({ "real": true })));
}

/// The mark is advertised as the value the webview carried — pinned, since the day a query command is attached the roster would advertise a false mark feeding the daemon's write-retention accounting. An item with no mark is not registered (choosing a default is itself the lie).
#[tokio::test]
async fn the_advertised_effect_comes_from_the_webview_not_from_a_constant() {
    let (bridge, _seen) = recording_bridge(Duration::from_secs(1));
    let outcome = bridge.report(
        MAIN_WINDOW_LABEL,
        vec![
            view_decl_with_effect("view.peek", Some(ViewEffect::Read)),
            view_decl_with_effect("view.poke", Some(ViewEffect::Write)),
            view_decl_with_effect("view.mute", None),
        ],
    );

    assert_eq!(
        outcome.refused,
        vec!["view.mute".to_string()],
        "an item with no mark is dropped from registration"
    );
    let effect_of = |name: &str| -> String {
        let decl = outcome
            .accepted
            .iter()
            .find(|d| d.name == name)
            .unwrap_or_else(|| panic!("{name} should have been registered"));
        let item: serde_json::Value = serde_json::from_str(&decl.help).expect("help is JSON");
        item["effect"].as_str().expect("effect field").to_string()
    };
    assert_eq!(effect_of("view.peek"), "Read");
    assert_eq!(effect_of("view.poke"), "Write");
}

/// The registration packet is re-read every time — webview reports have no ordering with the socket, so a packet cached at plug-in time would resend the old (usually empty) list on reconnect, reviving "window up but screen commands not on the roster".
#[tokio::test]
async fn a_report_that_arrives_after_the_table_still_rides_the_next_registration() {
    let (bridge, _seen) = recording_bridge(Duration::from_secs(1));
    let (_world, receiver) = with_view(Arc::clone(&bridge));

    let before =
        registration_command(&receiver).expect("a packet comes out even with the shell part alone");
    let AgentCommand::RegisterCommands { decls, .. } = before else {
        panic!("RegisterCommands");
    };
    let decls = decls_from_wire(&decls).expect("the registration field the shell wrapped unpacks");
    assert!(
        !decls.iter().any(|d| d.name == "tab.next"),
        "no window has reported yet"
    );

    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("tab.next")]);

    let after = registration_command(&receiver).expect("packet");
    let AgentCommand::RegisterCommands { decls, .. } = after else {
        panic!("RegisterCommands");
    };
    let decls = decls_from_wire(&decls).expect("the registration field the shell wrapped unpacks");
    assert!(
        decls.iter().any(|d| d.name == "tab.next"),
        "the packet of the next (re)handshake has the webview part merged in"
    );
}

/// An abandoned round trip leaves no seat — if only the deadline erased the reply seat, seats would pile up whenever the runtime folds or a delivery future is cancelled.
#[tokio::test]
async fn a_cancelled_delivery_gives_its_answer_slot_back() {
    let (bridge, mut seen) = recording_bridge(Duration::from_secs(60));
    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("slot.empty")]);

    // Holds the task and then drops it — imitating cancellation (the Drop of `ReplySink` emits the reply).
    let (_world, ports) = World::build();
    let queue = Arc::new(Queued::default());
    let receiver = InboundReceiver::with_view(
        make_table(ports),
        Arc::clone(&queue) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
        Arc::clone(&bridge) as Arc<dyn ViewCommandPort>,
    );
    let mail = Mailbox::default();
    let request_id = RequestId::new();
    receiver.on_command(
        envelope("slot.empty", json!({}), request_id),
        mail.sink(request_id),
    );
    let mut task = Box::pin(queue.drain().pop().expect("one apply task"));

    let waker = futures_util::task::noop_waker();
    let mut cx = std::task::Context::from_waker(&waker);
    assert!(std::future::Future::poll(task.as_mut(), &mut cx).is_pending());
    let (_target, request) = seen.try_recv().expect("went down to the webview");
    drop(task);

    bridge
        .settle(MAIN_WINDOW_LABEL, &request.request_id, Ok(json!(null)))
        .expect_err("the seat of an abandoned round trip does not remain");
    assert_eq!(mail.len(), 1);
}

/// A name the shell table answers first is dropped from registration — carried delivery would not diverge (`route` looks at the table first), but the roster would gain an unreachable item whose discovered webview contract differs from the shell contract.
#[tokio::test]
async fn a_name_the_shell_table_answers_is_left_out_and_still_runs_in_the_shell() {
    let (bridge, mut seen) = recording_bridge(Duration::from_secs(1));
    let outcome = bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("tab.create")]);
    assert_eq!(outcome.refused, vec!["tab.create".to_string()]);
    assert!(outcome.accepted.is_empty());

    let (world, receiver) = with_view(Arc::clone(&bridge));
    let mail = Mailbox::default();
    let request_id = RequestId::new();
    receiver.accept(
        envelope(
            "tab.create",
            json!({ "window": MAIN_WINDOW_LABEL }),
            request_id,
        ),
        mail.deliver(),
    );
    mail.settle(1).await;

    assert!(
        mail.only().outcome.is_ok(),
        "the shell apply service answered"
    );
    assert_eq!(world.main_tabs().tabs.len(), 2, "a tab was actually added");
    assert!(seen.try_recv().is_err(), "nothing went down to the webview");
}

/// A name not in the shell table goes down to the webview and its answer returns with the same correlation key (stage 2 of the 3-stage delivery).
#[tokio::test]
async fn a_name_only_the_webview_owns_is_delivered_there_and_answered() {
    let (bridge, mut seen) = recording_bridge(Duration::from_secs(5));
    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("tab.next")]);

    let (_world, receiver) = with_view(Arc::clone(&bridge));
    let mail = Mailbox::default();
    let request_id = RequestId::new();
    receiver.accept(
        envelope("tab.next", json!({ "window": "main" }), request_id),
        mail.deliver(),
    );

    let (target, request) = seen.recv().await.expect("goes down to the webview");
    assert_eq!(
        target, MAIN_WINDOW_LABEL,
        "goes to the window that reported"
    );
    assert_eq!(request.name, "tab.next");
    assert_eq!(request.args, json!({ "window": "main" }));
    assert_eq!(
        request.request_id,
        request_id.to_string(),
        "the correlation key is the same across the whole span"
    );
    assert!(
        mail.len() == 0,
        "there is no outcome before the webview answers"
    );

    bridge
        .settle(
            MAIN_WINDOW_LABEL,
            &request.request_id,
            Ok(json!({ "applied": true })),
        )
        .expect("there is a waiting seat");
    mail.settle(1).await;

    let reply = mail.only();
    assert_eq!(reply.request_id, request_id);
    assert_eq!(reply.outcome, Ok(json!({ "applied": true })));

    // The second answer has no seat — a sign that two windows received the same envelope, so it is not swallowed quietly.
    bridge
        .settle(MAIN_WINDOW_LABEL, &request.request_id, Ok(json!(null)))
        .expect_err("one reply per request_id");
}

/// Even if the window vanishes the round trip ends as a value — without a deadline the envelope never ends and the caller hangs (`route` sets none; that is this bridge's job).
#[tokio::test]
async fn a_webview_that_never_answers_ends_as_a_timeout_not_a_hang() {
    let (bridge, mut seen) = recording_bridge(Duration::from_millis(50));
    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("slot.empty")]);

    let (_world, receiver) = with_view(Arc::clone(&bridge));
    let mail = Mailbox::default();
    let request_id = RequestId::new();
    receiver.accept(
        envelope("slot.empty", json!({}), request_id),
        mail.deliver(),
    );
    seen.recv().await.expect("goes down to the webview");

    mail.settle(1).await;
    let err = error_of(mail.only());
    assert_eq!(err.code(), ErrorCode::Timeout);
    assert_eq!(
        err.retry(),
        engram_dashboard_command::RetryMode::SameRequestId,
        "whether it was applied is unknown, so calling again with a new id may apply it twice"
    );
}

/// With no window reported the name is "unknown" — answering that an owner exists with nowhere to deliver would blur "no such command" and "nowhere to send".
#[tokio::test]
async fn a_webview_name_is_unknown_until_a_window_reports_it() {
    let (bridge, mut seen) = recording_bridge(Duration::from_secs(1));
    let (_world, receiver) = with_view(Arc::clone(&bridge));
    assert!(bridge.host().is_none());

    let mail = Mailbox::default();
    let request_id = RequestId::new();
    receiver.accept(envelope("tab.next", json!({}), request_id), mail.deliver());
    mail.settle(1).await;

    let err = error_of(mail.only());
    assert_eq!(err.code(), ErrorCode::UnknownCommand);
    assert!(
        seen.try_recv().is_err(),
        "there is nowhere to send, so nothing went out"
    );
}

/// Reporting the same list again gives no diff (every window runs this App and reports everything; a diff per report would multiply meaningless round trips by the window count). For the destination main wins — a pop-out as destination would kill all webview commands when it closes.
#[tokio::test]
async fn repeating_the_same_report_asks_for_no_delta_and_main_keeps_the_destination() {
    let (bridge, _seen) = recording_bridge(Duration::from_secs(1));

    let first = bridge.report("popup-1", vec![view_decl("tab.next")]);
    assert!(first.changed(), "the first report fills the list");
    assert_eq!(
        bridge.host().as_deref(),
        Some("popup-1"),
        "keeps the window that came first"
    );

    let again = bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("tab.next")]);
    assert!(
        !again.changed(),
        "with the same list there is no diff to send"
    );
    assert_eq!(
        bridge.host().as_deref(),
        Some(MAIN_WINDOW_LABEL),
        "main overrides"
    );

    let stolen = bridge.report("popup-2", vec![view_decl("tab.next")]);
    assert!(!stolen.changed());
    assert_eq!(
        bridge.host().as_deref(),
        Some(MAIN_WINDOW_LABEL),
        "main is not taken over"
    );

    let shrunk = bridge.report(MAIN_WINDOW_LABEL, vec![]);
    assert_eq!(shrunk.removed, vec!["tab.next".to_string()]);
    assert!(shrunk.accepted.is_empty());
}

/// A registration packet's `help` uses the same fields a Rust declaration emits — otherwise one roster mixes two dialects and the LLM must read each command differently.
#[tokio::test]
async fn a_reported_shape_becomes_a_catalog_item_in_the_same_dialect() {
    let (bridge, _seen) = recording_bridge(Duration::from_secs(1));
    let mut args = BTreeMap::new();
    // No screen command carries `enum` today (ADR-0167 removed the last sender), but whether the projection lets the field through is a contract — a value is invented to measure it.
    args.insert(
        "window".to_string(),
        ViewArgSchema {
            ty: Some("string".to_string()),
            allowed: Some(vec!["main".to_string(), "slot-popup-1".to_string()]),
            description: Some("target window label".to_string()),
        },
    );
    let outcome = bridge.report(
        MAIN_WINDOW_LABEL,
        vec![ViewCommandDecl {
            name: "tab.next".to_string(),
            help: ViewCommandHelp {
                summary: "  moves that window's active tab to the next tab  ".to_string(),
                effect: Some(ViewEffect::Write),
                args,
                // A field not in the declaration drops out of `required` — left in, no argument could satisfy the schema.
                required: vec!["window".to_string(), "ghost".to_string()],
            },
        }],
    );

    let item: serde_json::Value =
        serde_json::from_str(&outcome.accepted[0].help).expect("help is JSON");
    assert_eq!(item["name"], "tab.next");
    assert_eq!(
        item["summary"],
        "moves that window's active tab to the next tab"
    );
    assert_eq!(item["args"]["properties"]["window"]["type"], "string");
    assert_eq!(
        item["args"]["properties"]["window"]["enum"][1],
        "slot-popup-1"
    );
    assert_eq!(item["args"]["required"], json!(["window"]));
    for key in ["name", "effect", "since", "summary", "args", "ok", "errors"] {
        assert!(item.get(key).is_some(), "field {key} is missing: {item}");
    }
    // The advertised errors must equal exactly the set this bridge can emit — a subset check would allow advertising a code that never comes (the caller writes a branch that never runs); a missing one drops the caller to the default branch. The four sources:
    // - `INTERNAL`: a failure the webview threw (kind not distinguishable) · delivery failure · early close of the reply channel.
    // - `TIMEOUT`: deadline exceeded (`a_webview_that_never_answers_ends_as_a_timeout_not_a_hang`).
    // - `REQUEST_ID_CONFLICT`: the same number is already running (`a_duplicate_request_id_is_refused_instead_of_displacing_the_live_waiter`).
    // - `UNSUPPORTED`: no window reported — normally unreachable, since the lookup first answers "unknown name" and delivery ends as `UNKNOWN_COMMAND` (`a_webview_name_is_unknown_until_a_window_reports_it`); it opens only when the host died between lookup and delivery, which no test interleaves (uncovered).
    let advertised: BTreeSet<&str> = item["errors"]
        .as_array()
        .expect("errors is an array")
        .iter()
        .map(|code| code.as_str().expect("the code is a string"))
        .collect();
    let produced: BTreeSet<&str> = [
        ErrorCode::Internal,
        ErrorCode::Timeout,
        ErrorCode::Unsupported,
        ErrorCode::RequestIdConflict,
    ]
    .iter()
    .map(|code| code.as_str())
    .collect();
    assert_eq!(
        advertised, produced,
        "the advertisement and reality diverged — a surplus makes a branch that never runs, a missing one makes an answer go out of the advertisement"
    );
}

/// An item with no description is not registered — a name alone cannot be called, since discovery gives no material for arguments (the registration bundles the shape to remove that round trip — ADR-0156).
#[tokio::test]
async fn a_reported_command_without_a_summary_is_left_out() {
    let (bridge, _seen) = recording_bridge(Duration::from_secs(1));
    let outcome = bridge.report(
        MAIN_WINDOW_LABEL,
        vec![ViewCommandDecl {
            name: "tab.next".to_string(),
            help: ViewCommandHelp {
                summary: "   ".to_string(),
                effect: Some(ViewEffect::Write),
                args: BTreeMap::new(),
                required: Vec::new(),
            },
        }],
    );

    assert_eq!(outcome.refused, vec!["tab.next".to_string()]);
    assert!(outcome.accepted.is_empty());
}

#[tokio::test]
async fn a_dropped_task_still_answers() {
    let (world, queue, receiver) = queued();
    let request_id = RequestId::new();
    receiver.on_command(
        envelope("window.list", json!({}), request_id),
        world.mail.sink(request_id),
    );

    drop(queue.drain());

    let reply = world.mail.only();
    assert_eq!(reply.request_id, request_id);
    assert_eq!(error_of(reply).code(), ErrorCode::Internal);
}

// ── (S28) Commands that take targets — choose by agent (id the daemon swapped in) · spatial word, answer the changed layout (TRD S28 §4 · §5) ───────────

impl World {
    fn show(&self, view: uuid::Uuid, slot: uuid::Uuid, agent: uuid::Uuid, name: &str) {
        apply::assign_agent(&self.state, &Subs, &Events, view, slot, agent.to_string())
            .expect("agent placement");
        apply::agent_names_updated(
            &self.state,
            &Events,
            &[(agent.to_string(), name.to_string())],
        )
        .expect("name ledger");
    }

    fn popout(&self) -> (String, uuid::Uuid) {
        let created = apply::create_window(
            &self.state,
            &Subs,
            &Events,
            self.windows.as_ref(),
            self.labels.as_ref(),
        )
        .expect("pop-out");
        let tab = self.tabs(&created.window).active;
        (created.window, tab)
    }

    fn add_tab(&self, window: &str, shown: uuid::Uuid) -> uuid::Uuid {
        let tab = apply::create_tab(&self.state, &Subs, &Events, window, None).expect("tab");
        apply::switch_tab(&self.state, &Subs, &Events, window, shown).expect("visible tab");
        tab
    }

    fn tabs(&self, window: &str) -> WindowTabsPayload {
        apply::list_tabs(&self.state, window).expect("window")
    }

    fn tab_ids(&self, window: &str) -> Vec<uuid::Uuid> {
        self.tabs(window).tabs.iter().map(|meta| meta.id).collect()
    }

    fn version(&self) -> u64 {
        self.state.0.lock().unwrap().version
    }

    fn focused(&self, view: uuid::Uuid) -> Option<uuid::Uuid> {
        apply::get_view(&self.state, view)
            .expect("view")
            .focused_slot_id
    }
}

/// Whether the errors this command advertises include `code` — the daemon · CLI filter codes by that declaration (ADR-0156).
fn assert_declared(name: &str, code: ErrorCode) {
    let spec = COMMAND_SPECS
        .iter()
        .find(|spec| spec.name == name)
        .unwrap_or_else(|| panic!("{name} declaration"));
    assert!(
        spec.advertised_errors().contains(&code),
        "{name} does not advertise {code}"
    );
}

async fn ok_of(
    receiver: &InboundReceiver,
    queue: &Queued,
    name: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    call(receiver, queue, &Mailbox::default(), name, args)
        .await
        .outcome
        .unwrap_or_else(|e| panic!("{name}: {e:?}"))
}

async fn refused(
    receiver: &InboundReceiver,
    queue: &Queued,
    name: &str,
    args: serde_json::Value,
) -> CommandError {
    let error = error_of(call(receiver, queue, &Mailbox::default(), name, args).await);
    assert_declared(name, error.code());
    error
}

fn rows(answer: &serde_json::Value, field: &str) -> Vec<serde_json::Value> {
    answer[field].as_array().expect("row list").clone()
}

/// The rows of a default answer carry no id fields at all — not even `null` (rule §4 — size).
fn assert_no_ids(rows: &[serde_json::Value]) {
    for row in rows {
        for key in ["window_id", "tab", "slot_id", "agent_id"] {
            assert!(row.get(key).is_none(), "{key} was carried: {row}");
        }
    }
}

#[test]
fn the_new_names_are_kept_off_the_webview_registry() {
    let reserved = reserved_names();
    for name in ["tab.move", "tab.dock", "tab.detach", "layout.get"] {
        assert!(reserved.contains(name), "{name}");
    }
}

/// Fields that choose an agent — arguments marked per command with `#[agent_ref]` (declaration order). The daemon turns names into ids by this list in the registration packet (ADR-0308): a missing field's name reaches the shell as INVALID_ARGUMENT, a wrong one makes the daemon resolve an id field as a name and refuse.
fn marked_agent_args() -> BTreeMap<&'static str, Vec<&'static str>> {
    BTreeMap::from([
        ("tab.switch", vec!["tab_of"]),
        ("tab.close", vec!["tab_of"]),
        ("tab.rename", vec!["tab_of"]),
        ("slot.split", vec!["slot_of"]),
        ("slot.close", vec!["slot_of"]),
        ("slot.popout", vec!["slot_of"]),
        ("slot.focus", vec!["slot_of"]),
        ("slot.assignAgent", vec!["agent_id", "slot_of", "assign"]),
        (
            "layout.setSlotContent",
            vec!["agent_id", "slot_of", "assign"],
        ),
        ("tab.move", vec!["tab_of", "after_tab_of"]),
        ("tab.dock", vec!["tab_of", "beside"]),
        ("tab.detach", vec!["tab_of"]),
        ("layout.get", vec!["agent"]),
    ])
}

/// The registration packet (`decls`) carries the mark outside `help`; an unmarked command has an empty list. A marked command advertises the daemon's two refusal codes (`NOT_FOUND` · `CONFLICT`, ADR-0308), and every argument of a name-shaped agent form (`*_of` · `beside` · `assign` · `agent` · `agent_id`) is marked — a forgotten mark lets the name reach the shell as INVALID_ARGUMENT.
#[test]
fn the_registration_carries_each_commands_agent_reference_args() {
    let (_world, ports) = World::build();
    let want = marked_agent_args();
    let decls = make_table(ports).decls();
    assert_eq!(decls.len(), COMMAND_SPECS.len());
    let names_an_agent = |arg: &str| {
        arg.ends_with("_of") || matches!(arg, "beside" | "assign" | "agent" | "agent_id")
    };
    for decl in &decls {
        let expected = want.get(decl.name.as_str()).cloned().unwrap_or_default();
        assert_eq!(decl.agent_args, expected, "{}", decl.name);
        assert!(!decl.help.contains("agent_ref"), "{}", decl.name);

        let help: serde_json::Value = serde_json::from_str(&decl.help).expect("help is JSON");
        if !decl.agent_args.is_empty() {
            let errors = help["errors"].as_array().expect("errors array");
            for code in ["NOT_FOUND", "CONFLICT"] {
                assert!(
                    errors.iter().any(|e| e == code),
                    "{}: has a marked field but does not advertise {code} — {errors:?}",
                    decl.name
                );
            }
        }
        let args = help["args"]["properties"]
            .as_object()
            .expect("args is an object schema");
        for arg in args.keys().filter(|arg| names_an_agent(arg)) {
            assert!(
                decl.agent_args.iter().any(|marked| marked == arg),
                "{}: the agent-taking argument {arg} is not marked",
                decl.name
            );
        }
    }
    for name in want.keys() {
        assert!(
            decls.iter().any(|decl| decl.name == *name),
            "{name} is not in the table"
        );
    }
}

/// A non-UUID in a marked field (a call that bypassed the daemon, so the name arrived as is) is not resolved by the shell and gets INVALID_ARGUMENT — nothing changes.
#[tokio::test]
async fn a_name_in_an_agent_reference_arg_is_an_invalid_argument_here() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    let (tab, slot) = (a.to_string(), world.empty_slot(a).to_string());
    world.add_tab(MAIN_WINDOW_LABEL, a);
    let version = world.version();

    // (command, an argument that makes it pass the earlier screening other than the marked field)
    let others = |name: &str, arg: &str| match (name, arg) {
        ("tab.move", "tab_of") => json!({ "index": 0 }),
        ("tab.move", "after_tab_of") => json!({ "tab": tab }),
        ("tab.dock", "tab_of") => json!({ "beside_at": "top-left", "side": "Left" }),
        ("tab.dock", "beside") => json!({ "tab": tab, "side": "Left" }),
        ("tab.rename", _) => json!({ "name": "work" }),
        ("slot.split", _) => json!({ "dir": "LeftRight" }),
        ("slot.assignAgent", "slot_of") => json!({ "assign": BETA }),
        ("slot.assignAgent", _) => json!({ "slot_id": slot }),
        ("layout.setSlotContent", "slot_of") => json!({ "content": "Empty" }),
        ("layout.setSlotContent", _) => json!({ "slot_id": slot, "content": "Agent" }),
        _ => json!({}),
    };
    let mut checked = 0;
    for (name, args) in marked_agent_args() {
        for arg in args {
            let call_args = with(&others(name, arg), json!({ arg: "alpha" }));
            let error = refused(&receiver, &queue, name, call_args.clone()).await;
            assert_eq!(
                error.code(),
                ErrorCode::InvalidArgument,
                "{name} {call_args}"
            );
            assert!(
                error
                    .message()
                    .starts_with(&format!("{arg} must be an agent id (a UUID)")),
                "{name} {call_args}: {}",
                error.message()
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 19);
    assert_eq!(world.version(), version, "nothing changes");
}

// ── tab.move ──

#[tokio::test]
async fn tab_move_by_agent_carries_its_tab_into_another_window_keeping_what_is_shown() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    world.show(a, world.empty_slot(a), agent_id(ALPHA), "alpha");
    let b = world.add_tab(MAIN_WINDOW_LABEL, a);
    let (pop, p0) = world.popout();
    let p1 = world.add_tab(&pop, p0);

    let answer = ok_of(
        &receiver,
        &queue,
        "tab.move",
        json!({ "tab_of": ALPHA, "to_window": pop, "index": 1 }),
    )
    .await;

    assert_eq!(world.tab_ids(&pop), [p0, a, p1]);
    assert_eq!(world.tab_ids(MAIN_WINDOW_LABEL), [b]);
    assert_eq!(
        world.tabs(&pop).active,
        p0,
        "the destination window's visible tab stays"
    );
    assert_eq!(
        world.main_tabs().active,
        b,
        "a window whose visible tab left shows the neighbouring tab"
    );
    assert_eq!(answer["changed"], true);
    assert_eq!(answer["window"], json!(pop));
    assert_eq!(answer["window_id"], json!(world.tabs(&pop).window_id));
    assert_eq!(answer["number"], 2);
    assert_eq!(answer["tab"], json!(a.to_string()));
    assert_eq!(answer["tab_index"], 1, "the placed tab's row position");
    let layout = rows(&answer, "layout");
    assert_no_ids(&layout);
    assert_eq!(
        layout
            .iter()
            .map(|row| (
                row["window"].clone(),
                row["tab_index"].clone(),
                row["tab_active"].clone()
            ))
            .collect::<Vec<_>>(),
        [
            (json!(MAIN_WINDOW_LABEL), json!(0), json!(true)),
            (json!(pop), json!(1), json!(false)),
        ]
    );
    assert_eq!(layout[1]["agent"], "alpha");
    assert_eq!(layout[1]["content"], "Agent");
    assert!(
        layout[0].get("agent").is_none(),
        "an empty area has no agent field"
    );
}

#[tokio::test]
async fn tab_move_by_tab_id_reorders_its_own_window() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    let b = world.add_tab(MAIN_WINDOW_LABEL, a);
    let c = world.add_tab(MAIN_WINDOW_LABEL, a);

    let answer = ok_of(
        &receiver,
        &queue,
        "tab.move",
        json!({ "tab": c.to_string(), "index": 0 }),
    )
    .await;

    assert_eq!(world.tab_ids(MAIN_WINDOW_LABEL), [c, a, b]);
    assert_eq!(world.main_tabs().active, a, "the visible tab stays");
    assert_eq!(answer["changed"], true);
    assert_eq!(answer["window"], MAIN_WINDOW_LABEL);
    assert_eq!(answer["window_id"], MAIN_WINDOW_LABEL);
    assert_eq!(answer["number"], 1);
    assert_eq!(answer["tab"], json!(c.to_string()));

    let version = world.version();
    let again = ok_of(
        &receiver,
        &queue,
        "tab.move",
        json!({ "tab": c.to_string(), "index": 0 }),
    )
    .await;
    assert_eq!(again["changed"], false, "the same place");
    assert_eq!(world.version(), version);
    assert_eq!(rows(&again, "layout").len(), 1, "that single tab's row");
}

#[tokio::test]
async fn tab_move_after_another_agents_tab_goes_right_after_it() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    world.show(a, world.empty_slot(a), agent_id(ALPHA), "alpha");
    let b = world.add_tab(MAIN_WINDOW_LABEL, a);
    let (pop, p0) = world.popout();
    world.show(p0, world.empty_slot(p0), agent_id(BETA), "beta");
    let p1 = world.add_tab(&pop, p0);

    let answer = ok_of(
        &receiver,
        &queue,
        "tab.move",
        json!({ "tab_of": ALPHA, "after_tab_of": BETA }),
    )
    .await;

    assert_eq!(world.tab_ids(&pop), [p0, a, p1]);
    assert_eq!(world.tab_ids(MAIN_WINDOW_LABEL), [b]);
    assert_eq!(answer["window"], json!(pop));
    assert_eq!(answer["tab"], json!(a.to_string()));
}

/// After an agent leaves its tab for another, `tab_of` moves the tab that holds it now, not the one it left (target found at call time). The daemon-conversion/shell-application interleaving cannot be reproduced here; `layout_apply.rs` measures it by resolving inside the apply lock (TRD S28 §5-2).
#[tokio::test]
async fn tab_move_by_agent_moves_the_tab_it_is_in_now_not_the_one_it_left() {
    let (world, queue, receiver) = queued();
    let alpha = agent_id(ALPHA);
    let a = world.main_tabs().active;
    let a_slot = world.empty_slot(a);
    world.show(a, a_slot, alpha, "alpha");
    let b = world.add_tab(MAIN_WINDOW_LABEL, a);
    let b_slot = world.empty_slot(b);
    let (pop, p0) = world.popout();

    apply::set_slot_content(&world.state, &Subs, &Events, a, a_slot, SlotContent::Empty)
        .expect("left");
    apply::assign_agent(&world.state, &Subs, &Events, b, b_slot, alpha.to_string()).expect("moved");

    ok_of(
        &receiver,
        &queue,
        "tab.move",
        json!({ "tab_of": ALPHA, "to_window": pop }),
    )
    .await;

    assert_eq!(
        world.tab_ids(&pop),
        [p0, b],
        "the tab that agent holds now is the one that goes"
    );
    assert_eq!(
        world.tab_ids(MAIN_WINDOW_LABEL),
        [a],
        "the tab it left stays"
    );
}

#[tokio::test]
async fn tab_move_refuses_argument_combinations_before_applying() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    world.show(a, world.empty_slot(a), agent_id(ALPHA), "alpha");
    let version = world.version();

    let a_id = a.to_string();
    for args in [
        json!({ "tab_of": ALPHA, "after_tab_of": BETA, "index": 0 }),
        json!({ "tab_of": ALPHA, "after_tab": a_id, "index": 0 }),
        json!({ "tab_of": ALPHA, "to_window": "main", "after_tab_of": BETA }),
        json!({ "tab_of": ALPHA, "to_window": "main", "after_tab": a_id }),
        json!({ "tab_of": ALPHA, "after_tab": a_id, "after_tab_of": BETA }),
        json!({ "tab_of": ALPHA }),
        json!({ "tab": a_id, "tab_of": ALPHA, "index": 0 }),
        json!({ "index": 0 }),
        json!({ "tab": "not-a-uuid", "index": 0 }),
        json!({ "tab_of": ALPHA, "after_tab": "beta" }),
        json!({ "tab_of": "   ", "index": 0 }),
        json!({ "tab_of": ALPHA, "to_window": "2" }),
        json!({ "tab_of": ALPHA, "after_tab_of": "" }),
        json!({ "tab_of": ALPHA, "window": "main", "index": 0 }),
    ] {
        let error = refused(&receiver, &queue, "tab.move", args.clone()).await;
        assert_eq!(error.code(), ErrorCode::InvalidArgument, "{args}");
    }
    assert_eq!(world.version(), version, "nothing changes");
}

#[tokio::test]
async fn tab_move_refuses_a_window_argument_and_names_to_window() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    world.add_tab(MAIN_WINDOW_LABEL, a);
    let version = world.version();

    let error = refused(
        &receiver,
        &queue,
        "tab.move",
        json!({ "tab": a.to_string(), "window": "main", "index": 1 }),
    )
    .await;

    assert_eq!(error.code(), ErrorCode::InvalidArgument);
    assert!(
        error
            .message()
            .starts_with("tab.move's destination is to_window"),
        "{}",
        error.message()
    );
    assert_eq!(world.version(), version);
}

#[tokio::test]
async fn a_non_uuid_in_an_id_argument_names_the_argument_that_takes_a_name() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active.to_string();
    for (name, args, alternative) in [
        (
            "tab.move",
            json!({ "tab": "alpha", "index": 0 }),
            "use tab_of (",
        ),
        (
            "tab.move",
            json!({ "tab": a, "after_tab": "alpha" }),
            "use after_tab_of (",
        ),
        (
            "tab.dock",
            json!({ "tab": a, "beside_slot": "alpha", "side": "Left" }),
            "use beside (",
        ),
        (
            "tab.dock",
            json!({ "tab": a, "beside_slot": "alpha", "side": "Left" }),
            "or beside_at (",
        ),
        ("tab.detach", json!({ "tab": "alpha" }), "use tab_of ("),
    ] {
        let error = refused(&receiver, &queue, name, args.clone()).await;
        assert_eq!(error.code(), ErrorCode::InvalidArgument, "{args}");
        assert!(
            error.message().contains("to name it another way")
                && error.message().contains(alternative),
            "{name} {args}: {}",
            error.message()
        );
    }
}

#[tokio::test]
async fn tab_move_after_a_tab_id_goes_right_after_it() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    let b = world.add_tab(MAIN_WINDOW_LABEL, a);
    let (pop, p0) = world.popout();
    let p1 = world.add_tab(&pop, p0);

    let answer = ok_of(
        &receiver,
        &queue,
        "tab.move",
        json!({ "tab": b.to_string(), "after_tab": p0.to_string() }),
    )
    .await;

    assert_eq!(world.tab_ids(&pop), [p0, b, p1]);
    assert_eq!(answer["window"], json!(pop));
    assert_eq!(answer["tab_index"], 1);

    let error = refused(
        &receiver,
        &queue,
        "tab.move",
        json!({ "tab": b.to_string(), "after_tab": b.to_string() }),
    )
    .await;
    assert_eq!(error.code(), ErrorCode::InvalidArgument);
    assert!(
        error.message().starts_with(&format!("after_tab '{b}': ")),
        "{}",
        error.message()
    );
}

#[tokio::test]
async fn tab_move_after_an_agent_inside_the_moving_tab_is_an_invalid_argument() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    world.show(a, world.empty_slot(a), agent_id(ALPHA), "alpha");
    world.add_tab(MAIN_WINDOW_LABEL, a);

    let error = refused(
        &receiver,
        &queue,
        "tab.move",
        json!({ "tab_of": ALPHA, "after_tab_of": ALPHA }),
    )
    .await;

    assert_eq!(error.code(), ErrorCode::InvalidArgument);
    assert!(
        error
            .message()
            .starts_with(&format!("after_tab_of 'alpha' ({ALPHA}): ")),
        "{}",
        error.message()
    );
}

/// A name the daemon knows but no area shows: `NOT_FOUND` + the names now shown; shown in several tabs: `CONFLICT` + those areas' ids (a clue for the next call, rule §1-3).
#[tokio::test]
async fn tab_move_names_an_agent_not_shown_or_shown_in_two_tabs_with_candidates() {
    let (world, queue, receiver) = queued();
    let alpha = agent_id(ALPHA);
    let a = world.main_tabs().active;
    let a_slot = world.empty_slot(a);
    world.show(a, a_slot, alpha, "alpha");

    let error = refused(
        &receiver,
        &queue,
        "tab.move",
        json!({ "tab_of": BETA, "index": 0 }),
    )
    .await;
    assert_eq!(error.code(), ErrorCode::NotFound);
    assert!(
        error.message().starts_with(&format!("tab_of '{BETA}': ")),
        "{}",
        error.message()
    );
    assert!(
        error.message().ends_with(" — candidates: alpha"),
        "{}",
        error.message()
    );

    let (pop, p0) = world.popout();
    let p0_slot = world.empty_slot(p0);
    world.show(p0, p0_slot, alpha, "alpha");
    let version = world.version();
    let error = refused(
        &receiver,
        &queue,
        "tab.move",
        json!({ "tab_of": ALPHA, "index": 0 }),
    )
    .await;
    assert_eq!(error.code(), ErrorCode::Conflict);
    assert!(
        error
            .message()
            .contains("; pass one of these as tab — candidates: "),
        "{}",
        error.message()
    );
    for (tab, window) in [(a, MAIN_WINDOW_LABEL), (p0, pop.as_str())] {
        assert!(
            error
                .message()
                .contains(&format!("tab={tab} (window={window}, tab_index=0)")),
            "{}",
            error.message()
        );
    }
    assert!(
        !error.message().contains("slot_id="),
        "a tab row is a per-tab candidate"
    );
    assert_eq!(world.version(), version);
}

#[tokio::test]
async fn tab_level_ambiguity_lists_each_tab_once_and_names_the_id_argument_of_its_row() {
    let (world, queue, receiver) = queued();
    let alpha = agent_id(ALPHA);
    let a = world.main_tabs().active;
    let first = world.empty_slot(a);
    let second = apply::split_slot(&world.state, &Subs, &Events, a, first, SplitDir::LeftRight)
        .expect("split");
    world.show(a, first, alpha, "alpha");
    world.show(a, second, alpha, "alpha");
    let b = world.add_tab(MAIN_WINDOW_LABEL, a);
    let b_slot = world.empty_slot(b);
    world.show(b, b_slot, alpha, "alpha");
    let c = world.add_tab(MAIN_WINDOW_LABEL, a);

    let error = refused(
        &receiver,
        &queue,
        "tab.move",
        json!({ "tab": c.to_string(), "after_tab_of": ALPHA }),
    )
    .await;

    assert_eq!(error.code(), ErrorCode::Conflict);
    assert!(
        error
            .message()
            .starts_with(&format!("after_tab_of 'alpha' ({ALPHA}): ")),
        "{}",
        error.message()
    );
    assert!(
        error.message().ends_with(&format!(
            "; pass one of these as after_tab — candidates: after_tab={a} (window={MAIN_WINDOW_LABEL}, tab_index=0), after_tab={b} (window={MAIN_WINDOW_LABEL}, tab_index=1)"
        )),
        "{}",
        error.message()
    );
}

#[tokio::test]
async fn tab_move_of_a_tab_id_that_does_not_exist_is_not_found() {
    let (_world, queue, receiver) = queued();
    let ghost = uuid::Uuid::new_v4();
    let error = refused(
        &receiver,
        &queue,
        "tab.move",
        json!({ "tab": ghost.to_string(), "index": 0 }),
    )
    .await;
    assert_eq!(error.code(), ErrorCode::NotFound);
    assert!(
        error.message().starts_with(&format!("tab '{ghost}': ")),
        "{}",
        error.message()
    );
}

// ── tab.dock ──

#[tokio::test]
async fn tab_dock_by_agents_attaches_beside_the_agents_slot_and_answers_the_surviving_tab() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    let beta_slot = world.empty_slot(a);
    world.show(a, beta_slot, agent_id(BETA), "beta");
    let b = world.add_tab(MAIN_WINDOW_LABEL, a);
    world.show(b, world.empty_slot(b), agent_id(ALPHA), "alpha");

    let answer = ok_of(
        &receiver,
        &queue,
        "tab.dock",
        json!({ "tab_of": ALPHA, "beside": BETA, "side": "Left", "share": 0.25 }),
    )
    .await;

    assert_eq!(
        world.tab_ids(MAIN_WINDOW_LABEL),
        [a],
        "the attached tab disappears"
    );
    assert_eq!(world.main_tabs().active, a);
    assert_eq!(world.focused(a), Some(beta_slot), "focus stays");
    assert_eq!(answer["changed"], true);
    assert_eq!(
        answer["tab"],
        json!(a.to_string()),
        "the surviving destination tab"
    );
    assert_eq!(answer["tab_index"], 0);
    assert_eq!(answer["window"], MAIN_WINDOW_LABEL);
    let layout = rows(&answer, "layout");
    assert_no_ids(&layout);
    assert_eq!(
        layout
            .iter()
            .map(|row| (
                row["agent"].clone(),
                row["x"].clone(),
                row["w"].clone(),
                row["focused"].clone()
            ))
            .collect::<Vec<_>>(),
        [
            (json!("alpha"), json!(0.0), json!(0.25), json!(false)),
            (json!("beta"), json!(0.25), json!(0.75), json!(true)),
        ]
    );
}

#[tokio::test]
async fn tab_dock_by_a_spatial_word_reads_it_in_the_named_windows_shown_tab() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    let b = world.add_tab(MAIN_WINDOW_LABEL, a);
    let (pop, p0) = world.popout();
    let left = world.empty_slot(p0);
    apply::split_slot(&world.state, &Subs, &Events, p0, left, SplitDir::LeftRight).expect("split");

    let answer = ok_of(
        &receiver,
        &queue,
        "tab.dock",
        json!({ "tab": b.to_string(), "beside_at": "top-right", "window": pop, "side": "Bottom" }),
    )
    .await;

    assert_eq!(answer["window"], json!(pop));
    assert_eq!(answer["tab"], json!(p0.to_string()));
    assert_eq!(world.tab_ids(MAIN_WINDOW_LABEL), [a]);
    let layout = rows(&answer, "layout");
    assert_eq!(layout.len(), 3);
    let right_column = layout.iter().filter(|row| row["x"] == json!(0.5)).count();
    assert_eq!(
        right_column, 2,
        "the right area is split top-bottom: {layout:?}"
    );
}

#[tokio::test]
async fn tab_dock_on_a_whole_tab_puts_the_tab_along_its_outer_edge() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    let a_slot = world.empty_slot(a);
    apply::split_slot(&world.state, &Subs, &Events, a, a_slot, SplitDir::LeftRight).expect("split");
    let b = world.add_tab(MAIN_WINDOW_LABEL, a);

    let answer = ok_of(
        &receiver,
        &queue,
        "tab.dock",
        json!({
            "tab": b.to_string(),
            "beside_slot": a_slot.to_string(),
            "whole": true,
            "side": "Top",
            "share": 0.5,
        }),
    )
    .await;

    let layout = rows(&answer, "layout");
    assert_eq!(layout.len(), 3);
    assert_eq!(
        (
            layout[0]["y"].clone(),
            layout[0]["w"].clone(),
            layout[0]["h"].clone()
        ),
        (json!(0.0), json!(1.0), json!(0.5)),
        "the attached tab is the whole top half"
    );
}

#[tokio::test]
async fn tab_dock_beside_an_agent_in_two_slots_is_ambiguous_unless_it_takes_the_whole_tab() {
    let (world, queue, receiver) = queued();
    let alpha = agent_id(ALPHA);
    let a = world.main_tabs().active;
    let first = world.empty_slot(a);
    let second = apply::split_slot(&world.state, &Subs, &Events, a, first, SplitDir::LeftRight)
        .expect("split");
    world.show(a, first, alpha, "alpha");
    world.show(a, second, alpha, "alpha");
    let b = world.add_tab(MAIN_WINDOW_LABEL, a);

    let error = refused(
        &receiver,
        &queue,
        "tab.dock",
        json!({ "tab": b.to_string(), "beside": ALPHA, "side": "Right" }),
    )
    .await;
    assert_eq!(error.code(), ErrorCode::Conflict);
    assert!(
        error
            .message()
            .starts_with(&format!("beside 'alpha' ({ALPHA}): ")),
        "{}",
        error.message()
    );
    for slot in [first, second] {
        assert!(
            error.message().contains(&format!("beside_slot={slot}")),
            "{}",
            error.message()
        );
    }

    let answer = ok_of(
        &receiver,
        &queue,
        "tab.dock",
        json!({ "tab": b.to_string(), "beside": ALPHA, "whole": true, "side": "Right" }),
    )
    .await;
    assert_eq!(answer["tab"], json!(a.to_string()));
}

#[tokio::test]
async fn tab_dock_maps_each_refusal_to_its_code() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    world.show(a, world.empty_slot(a), agent_id(ALPHA), "alpha");
    let b = world.add_tab(MAIN_WINDOW_LABEL, a);
    let (b_id, a_id) = (b.to_string(), a.to_string());
    let version = world.version();

    const SAME_TAB: &str =
        "both are in one tab; tab.dock moves whole tabs — slot.popout that slot first, then tab.dock";
    const CORNERS: &str =
        "try a corner word (top-left, top-right, bottom-left, bottom-right) or beside";
    // (argument, code, text head, what the text must contain)
    let (alpha_head, beta_head) = (
        format!("beside 'alpha' ({ALPHA}): "),
        format!("beside '{BETA}': "),
    );
    for (args, code, head, says) in [
        (
            json!({ "tab_of": ALPHA, "beside": ALPHA, "side": "Left" }),
            ErrorCode::InvalidArgument,
            alpha_head.as_str(),
            SAME_TAB,
        ),
        (
            json!({ "tab": b_id, "beside_at": "left", "side": "Left" }),
            ErrorCode::NotFound,
            "beside_at 'left': ",
            CORNERS,
        ),
        (
            json!({ "tab": b_id, "beside_slot": uuid::Uuid::nil().to_string(), "side": "Left" }),
            ErrorCode::NotFound,
            "beside_slot '",
            "(SlotNotFound)",
        ),
        (
            json!({ "tab": b_id, "beside": BETA, "side": "Left" }),
            ErrorCode::NotFound,
            beta_head.as_str(),
            " — candidates: alpha",
        ),
        (
            json!({ "tab": a_id, "beside_at": "top-left", "side": "Left" }),
            ErrorCode::InvalidArgument,
            "beside_at 'top-left': ",
            SAME_TAB,
        ),
    ] {
        let error = refused(&receiver, &queue, "tab.dock", args.clone()).await;
        assert_eq!(error.code(), code, "{args}: {}", error.message());
        assert!(
            error.message().starts_with(head) && error.message().contains(says),
            "{args}: {}",
            error.message()
        );
    }
    assert_eq!(world.version(), version);
}

#[tokio::test]
async fn tab_dock_refuses_argument_combinations_before_applying() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    let slot = world.empty_slot(a).to_string();
    let b = world.add_tab(MAIN_WINDOW_LABEL, a).to_string();

    for args in [
        json!({ "tab": b, "beside": ALPHA, "window": "main", "side": "Left" }),
        json!({ "tab": b, "beside": ALPHA, "beside_slot": slot, "side": "Left" }),
        json!({ "tab": b, "side": "Left" }),
        json!({ "tab": b, "beside_at": "middle", "side": "Left" }),
        json!({ "tab": b, "beside": ALPHA, "side": "Left", "share": 0.9 }),
        json!({ "tab": b, "beside": ALPHA, "side": "Left", "share": 0.0 }),
        json!({ "tab": b, "beside": ALPHA }),
        json!({ "tab": b, "beside": ALPHA, "side": "Diagonal" }),
        json!({ "tab_of": ALPHA, "beside_at": "left", "window": "2", "side": "Left" }),
    ] {
        let error = refused(&receiver, &queue, "tab.dock", args.clone()).await;
        assert_eq!(error.code(), ErrorCode::InvalidArgument, "{args}");
    }
}

// ── tab.detach ──

#[tokio::test]
async fn tab_detach_opens_a_new_window_without_changing_what_is_shown() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    let b = world.add_tab(MAIN_WINDOW_LABEL, a);
    world.show(b, world.empty_slot(b), agent_id(ALPHA), "alpha");

    let answer = ok_of(&receiver, &queue, "tab.detach", json!({ "tab_of": ALPHA })).await;

    let window = answer["window"].as_str().expect("new window").to_string();
    assert_ne!(window, MAIN_WINDOW_LABEL);
    assert_eq!(
        world.windows.opened.lock().unwrap().as_slice(),
        [window.clone()]
    );
    assert_eq!(answer["changed"], true);
    assert_eq!(answer["number"], 2);
    assert_eq!(answer["window_id"], json!(world.tabs(&window).window_id));
    assert_eq!(answer["tab"], json!(b.to_string()));
    assert_eq!(answer["tab_index"], 0);
    assert_eq!(world.tab_ids(&window), [b]);
    assert_eq!(world.tab_ids(MAIN_WINDOW_LABEL), [a]);
    assert_eq!(world.main_tabs().active, a, "the visible tab stays");
    assert_eq!(
        world.windows.placed.lock().unwrap().as_slice(),
        [(
            window.clone(),
            WindowPlace {
                pos: None,
                size: None
            },
            false
        )],
        "default place · does not take focus"
    );
    let layout = rows(&answer, "layout");
    assert_eq!(layout.len(), 1);
    assert_eq!(layout[0]["window"], json!(window));
    assert_eq!(layout[0]["agent"], "alpha");
}

/// If the agent moves to another tab while the new window opens (between two locks), the opened window is reaped and the result is `CONFLICT` — nothing moved (TRD S28 §4-3).
#[tokio::test]
async fn tab_detach_whose_agent_moves_while_the_window_opens_withdraws_it_as_a_conflict() {
    let (world, queue, receiver) = queued();
    let alpha = agent_id(ALPHA);
    let a = world.main_tabs().active;
    let b = world.add_tab(MAIN_WINDOW_LABEL, a);
    let b_slot = world.empty_slot(b);
    world.show(b, b_slot, alpha, "alpha");
    let c = world.add_tab(MAIN_WINDOW_LABEL, a);
    let c_slot = world.empty_slot(c);
    let state = world.state.clone();
    *world.windows.on_open.lock().unwrap() = Some(Box::new(move || {
        apply::set_slot_content(&state, &Subs, &Events, b, b_slot, SlotContent::Empty)
            .expect("left");
        apply::assign_agent(&state, &Subs, &Events, c, c_slot, alpha.to_string()).expect("moved");
    }));

    let error = refused(&receiver, &queue, "tab.detach", json!({ "tab_of": ALPHA })).await;

    assert_eq!(error.code(), ErrorCode::Conflict);
    assert!(error.message().contains("(Moved)"), "{}", error.message());
    let opened = world.windows.opened.lock().unwrap().clone();
    assert_eq!(opened.len(), 1, "the window was opened");
    assert_eq!(
        world.windows.closed.lock().unwrap().as_slice(),
        opened.as_slice(),
        "the opened window was reaped"
    );
    assert_eq!(
        world.tab_ids(MAIN_WINDOW_LABEL),
        [a, b, c],
        "nothing was moved"
    );
    assert_eq!(
        apply::list_windows(&world.state).unwrap(),
        [MAIN_WINDOW_LABEL]
    );
}

#[tokio::test]
async fn tab_detach_of_a_lone_popout_tab_changes_nothing() {
    let (world, queue, receiver) = queued();
    let (pop, p0) = world.popout();
    world.show(p0, world.empty_slot(p0), agent_id(BETA), "beta");
    let opened = world.windows.opened.lock().unwrap().len();
    let version = world.version();

    let answer = ok_of(&receiver, &queue, "tab.detach", json!({ "tab_of": BETA })).await;

    assert_eq!(answer["changed"], false);
    assert_eq!(answer["window"], json!(pop));
    assert_eq!(answer["tab"], json!(p0.to_string()));
    assert_eq!(
        world.windows.opened.lock().unwrap().len(),
        opened,
        "no new window"
    );
    assert_eq!(world.version(), version);
}

#[tokio::test]
async fn tab_detach_needs_exactly_one_tab() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active.to_string();
    for args in [json!({}), json!({ "tab": a, "tab_of": ALPHA })] {
        let error = refused(&receiver, &queue, "tab.detach", args.clone()).await;
        assert_eq!(error.code(), ErrorCode::InvalidArgument, "{args}");
    }
}

// ── layout.get ──

#[tokio::test]
async fn layout_get_lists_every_slot_in_window_tab_tree_order_without_ids() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    let left = world.empty_slot(a);
    apply::split_slot(&world.state, &Subs, &Events, a, left, SplitDir::LeftRight).expect("split");
    world.show(a, left, agent_id(ALPHA), "alpha");
    apply::focus_slot(&world.state, &Events, a, left).expect("focus");
    let (pop, _p0) = world.popout();

    let answer = ok_of(&receiver, &queue, "layout.get", json!({})).await;

    let rows = rows(&answer, "rows");
    assert_no_ids(&rows);
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0],
        json!({
            "window": MAIN_WINDOW_LABEL,
            "window_number": 1,
            "tab_index": 0,
            "tab_name": rows[0]["tab_name"],
            "tab_active": true,
            "content": "Agent",
            "agent": "alpha",
            "focused": true,
            "x": 0.0, "y": 0.0, "w": 0.5, "h": 1.0,
        })
    );
    assert_eq!(rows[1]["content"], "Empty");
    assert!(rows[1].get("agent").is_none());
    assert_eq!(rows[1]["x"], json!(0.5));
    assert_eq!(rows[1]["focused"], false);
    assert_eq!(rows[2]["window"], json!(pop));
    assert_eq!(rows[2]["window_number"], 2);
}

#[tokio::test]
async fn layout_get_with_ids_adds_the_four_ids() {
    let (world, queue, receiver) = queued();
    let alpha = agent_id(ALPHA);
    let a = world.main_tabs().active;
    let left = world.empty_slot(a);
    let right = apply::split_slot(&world.state, &Subs, &Events, a, left, SplitDir::LeftRight)
        .expect("split");
    world.show(a, left, alpha, "alpha");

    let answer = ok_of(&receiver, &queue, "layout.get", json!({ "ids": true })).await;

    let rows = rows(&answer, "rows");
    assert_eq!(rows[0]["window_id"], MAIN_WINDOW_LABEL);
    assert_eq!(rows[0]["tab"], json!(a.to_string()));
    assert_eq!(rows[0]["slot_id"], json!(left.to_string()));
    assert_eq!(rows[0]["agent_id"], json!(alpha.to_string()));
    assert_eq!(rows[1]["slot_id"], json!(right.to_string()));
    assert!(
        rows[1].get("agent_id").is_none(),
        "an empty area has no agent id"
    );
}

#[tokio::test]
async fn layout_get_filters_by_window_and_by_agent() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    let left = world.empty_slot(a);
    apply::split_slot(&world.state, &Subs, &Events, a, left, SplitDir::LeftRight).expect("split");
    world.show(a, left, agent_id(ALPHA), "alpha");
    let (pop, _p0) = world.popout();
    let pop_id = world.tabs(&pop).window_id;

    for window in [pop.clone(), pop_id] {
        let answer = ok_of(&receiver, &queue, "layout.get", json!({ "window": window })).await;
        let rows = rows(&answer, "rows");
        assert_eq!(rows.len(), 1, "{window}");
        assert_eq!(rows[0]["window"], json!(pop));
    }

    let answer = ok_of(&receiver, &queue, "layout.get", json!({ "agent": ALPHA })).await;
    let alpha_rows = rows(&answer, "rows");
    assert_eq!(alpha_rows.len(), 1);
    assert_eq!(alpha_rows[0]["agent"], "alpha");

    let answer = ok_of(&receiver, &queue, "layout.get", json!({ "agent": BETA })).await;
    assert!(
        rows(&answer, "rows").is_empty(),
        "an agent in no area = an empty list"
    );
}

#[tokio::test]
async fn a_dock_reply_is_the_snapshot_of_the_lock_that_committed_it() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    let b = world.add_tab(MAIN_WINDOW_LABEL, a);
    let b_slot = world.empty_slot(b);
    let (pop, _p0) = world.popout();
    let state = world.state.clone();
    *world.events.hook.lock().unwrap() = Some(Box::new(move || {
        apply::split_slot(&state, &Subs, &Events, b, b_slot, SplitDir::LeftRight)
            .expect("interleaved split");
    }));

    let answer = ok_of(
        &receiver,
        &queue,
        "tab.move",
        json!({ "tab": b.to_string(), "to_window": pop }),
    )
    .await;

    assert_eq!(
        world.slots(b).len(),
        2,
        "the interleaved write entered the model"
    );
    let moved: Vec<_> = rows(&answer, "layout")
        .into_iter()
        .filter(|row| row["window"] == json!(pop))
        .collect();
    assert_eq!(
        moved.len(),
        1,
        "the answer is the single area at the moment of the move: {moved:?}"
    );
    assert_eq!(moved[0]["w"], json!(1.0));
}

// ── (S28) Widened area · tab verbs — choose by agent · spatial word, answer the changed tab's layout (TRD S28 §7) ────────────────────

impl World {
    fn name(&self, agent: uuid::Uuid, name: &str) {
        apply::agent_names_updated(
            &self.state,
            &Events,
            &[(agent.to_string(), name.to_string())],
        )
        .expect("name ledger");
    }

    fn content(&self, view: uuid::Uuid, slot: uuid::Uuid) -> Option<SlotContent> {
        tree::find_slot(
            &apply::get_view(&self.state, view).expect("view").layout,
            slot,
        )
        .cloned()
    }
}

fn slot_verbs() -> Vec<(&'static str, serde_json::Value)> {
    vec![
        ("slot.focus", json!({})),
        ("slot.close", json!({})),
        ("slot.split", json!({ "dir": "LeftRight" })),
        ("slot.popout", json!({})),
        ("slot.assignAgent", json!({ "assign": BETA })),
        ("layout.setSlotContent", json!({ "content": "Empty" })),
    ]
}

fn with(base: &serde_json::Value, extra: serde_json::Value) -> serde_json::Value {
    let mut args = base.clone();
    args.as_object_mut()
        .expect("object")
        .extend(extra.as_object().expect("object").clone());
    args
}

fn row_agents(layout: &[serde_json::Value]) -> Vec<(serde_json::Value, serde_json::Value)> {
    layout
        .iter()
        .map(|row| {
            (
                row["window"].clone(),
                row.get("agent").cloned().unwrap_or(json!(null)),
            )
        })
        .collect()
}

#[tokio::test]
async fn slot_verbs_take_exactly_one_way_to_name_the_slot_before_applying() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    let (view, slot) = (a.to_string(), world.empty_slot(a).to_string());
    let version = world.version();

    for (name, base) in slot_verbs() {
        for line in [
            json!({}),
            json!({ "slot_id": slot, "slot_of": ALPHA }),
            json!({ "slot_of": ALPHA, "at": "top-left" }),
            json!({ "view_id": view, "slot_of": ALPHA }),
            json!({ "view_id": view, "at": "top-left" }),
            json!({ "view_id": view }),
            json!({ "slot_id": slot, "window": "main" }),
            json!({ "slot_of": ALPHA, "window": "main" }),
            json!({ "at": "middle" }),
            json!({ "slot_of": "  " }),
            json!({ "at": "top-left", "window": "2" }),
        ] {
            let args = with(&base, line);
            let error = refused(&receiver, &queue, name, args.clone()).await;
            assert_eq!(error.code(), ErrorCode::InvalidArgument, "{name} {args}");
        }
        let error = refused(
            &receiver,
            &queue,
            name,
            with(&base, json!({ "view_id": view, "slot_id": "alpha" })),
        )
        .await;
        assert!(
            error.message().starts_with(
                "slot_id must be a UUID (the id shown by layout.get with ids), got \"alpha\" — to name it another way, use slot_of ("
            ),
            "{name}: {}",
            error.message()
        );
    }
    assert_eq!(world.version(), version, "nothing changes");
}

#[tokio::test]
async fn slot_verbs_by_agent_or_spatial_word_change_that_slot_and_answer_its_tab() {
    let (world, queue, receiver) = queued();
    let (alpha, beta) = (agent_id(ALPHA), agent_id(BETA));
    let a = world.main_tabs().active;
    let left = world.empty_slot(a);
    let right = apply::split_slot(&world.state, &Subs, &Events, a, left, SplitDir::LeftRight)
        .expect("split");
    world.show(a, left, alpha, "alpha");
    world.name(beta, "beta");
    assert_eq!(
        world.focused(a),
        Some(right),
        "a split focuses the new area"
    );

    let answer = ok_of(&receiver, &queue, "slot.focus", json!({ "slot_of": ALPHA })).await;
    assert_eq!(world.focused(a), Some(left));
    let layout = rows(&answer, "layout");
    assert_no_ids(&layout);
    assert_eq!(
        layout
            .iter()
            .map(|row| row["focused"].clone())
            .collect::<Vec<_>>(),
        [json!(true), json!(false)],
        "that tab's rows — the picture with the focus area changed"
    );

    let answer = ok_of(
        &receiver,
        &queue,
        "slot.assignAgent",
        json!({ "at": "top-right", "assign": BETA }),
    )
    .await;
    assert_eq!(
        world.content(a, right),
        Some(SlotContent::Agent {
            agent_id: beta.to_string()
        })
    );
    let layout = rows(&answer, "layout");
    assert_no_ids(&layout);
    assert_eq!(
        row_agents(&layout),
        [
            (json!(MAIN_WINDOW_LABEL), json!("alpha")),
            (json!(MAIN_WINDOW_LABEL), json!("beta"))
        ]
    );
    assert_eq!(layout[0]["focused"], true);

    let answer = ok_of(
        &receiver,
        &queue,
        "slot.split",
        json!({ "slot_of": BETA, "dir": "TopBottom" }),
    )
    .await;
    let below = uuid::Uuid::parse_str(answer["slot_id"].as_str().expect("new area")).expect("UUID");
    assert_eq!(
        world.slots(a).into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([left, right, below])
    );
    let layout = rows(&answer, "layout");
    assert_eq!(layout.len(), 3);
    assert_eq!(
        (layout[2]["x"].clone(), layout[2]["y"].clone()),
        (json!(0.5), json!(0.5)),
        "the new area is bottom-right"
    );

    let answer = ok_of(
        &receiver,
        &queue,
        "layout.setSlotContent",
        json!({ "slot_id": below.to_string(), "content": "PresetPalette" }),
    )
    .await;
    assert_eq!(world.content(a, below), Some(SlotContent::PresetPalette));
    assert_eq!(rows(&answer, "layout")[2]["content"], "PresetPalette");

    let answer = ok_of(
        &receiver,
        &queue,
        "slot.close",
        json!({ "at": "bottom-right" }),
    )
    .await;
    assert_eq!(
        world.slots(a).into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([left, right])
    );
    assert_eq!(rows(&answer, "layout").len(), 2);

    let answer = ok_of(
        &receiver,
        &queue,
        "slot.popout",
        json!({ "slot_of": ALPHA }),
    )
    .await;
    let window = answer["window"].as_str().expect("new window").to_string();
    assert_ne!(window, MAIN_WINDOW_LABEL);
    assert_eq!(world.slots(a), [right]);
    let layout = rows(&answer, "layout");
    assert_no_ids(&layout);
    assert_eq!(
        row_agents(&layout),
        [
            (json!(MAIN_WINDOW_LABEL), json!("beta")),
            (json!(window), json!("alpha"))
        ],
        "rows of the original tab + the new tab (window number order)"
    );
}

/// After an agent leaves its area for another, `slot_of` writes to the area that holds it now (the interleaving is `layout_apply.rs`'s part — as in `tab_move_by_agent_moves_the_tab_it_is_in_now_not_the_one_it_left`).
#[tokio::test]
async fn slot_of_writes_to_the_slot_the_agent_is_in_now_not_the_one_it_left() {
    let (world, queue, receiver) = queued();
    let alpha = agent_id(ALPHA);
    let a = world.main_tabs().active;
    let left = world.empty_slot(a);
    let right = apply::split_slot(&world.state, &Subs, &Events, a, left, SplitDir::LeftRight)
        .expect("split");
    world.show(a, left, alpha, "alpha");
    apply::set_slot_content(&world.state, &Subs, &Events, a, left, SlotContent::Empty)
        .expect("left");
    apply::assign_agent(&world.state, &Subs, &Events, a, right, alpha.to_string()).expect("moved");

    ok_of(
        &receiver,
        &queue,
        "layout.setSlotContent",
        json!({ "slot_of": ALPHA, "content": "AgentList" }),
    )
    .await;

    assert_eq!(world.content(a, right), Some(SlotContent::AgentList));
    assert_eq!(
        world.content(a, left),
        Some(SlotContent::Empty),
        "the area it left stays"
    );
}

#[tokio::test]
async fn old_id_calls_refuse_exactly_as_the_id_service_did() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    let slot = world.empty_slot(a);
    let (_pop, p0) = world.popout();
    let pop_slot = world.empty_slot(p0);
    let ghost = uuid::Uuid::new_v4();
    let alpha = agent_id(ALPHA);
    let (state, windows, labels) = (
        world.state.clone(),
        Arc::clone(&world.windows),
        Arc::clone(&world.labels),
    );
    let old = |result: Result<(), String>| result.expect_err("the old id edition refuses too");
    let id = |id: uuid::Uuid| id.to_string();
    let cases = [
        (
            "slot.focus",
            json!({ "view_id": id(ghost), "slot_id": id(slot) }),
            old(apply::focus_slot(&state, &Events, ghost, slot)),
        ),
        (
            "slot.close",
            json!({ "view_id": id(a), "slot_id": id(pop_slot) }),
            old(apply::close_slot(&state, &Subs, &Events, a, pop_slot)),
        ),
        (
            "slot.split",
            json!({ "view_id": id(a), "slot_id": id(ghost), "dir": "LeftRight" }),
            old(
                apply::split_slot(&state, &Subs, &Events, a, ghost, SplitDir::LeftRight)
                    .map(|_| ()),
            ),
        ),
        (
            "slot.popout",
            json!({ "view_id": id(a), "slot_id": id(ghost) }),
            old(apply::move_slot_to_window(
                &state,
                &Subs,
                &Events,
                windows.as_ref(),
                labels.as_ref(),
                a,
                ghost,
                None,
            )
            .map(|_| ())),
        ),
        (
            "slot.assignAgent",
            json!({ "view_id": id(ghost), "slot_id": id(slot), "agent_id": id(alpha) }),
            old(apply::assign_agent(
                &state,
                &Subs,
                &Events,
                ghost,
                slot,
                id(alpha),
            )),
        ),
        (
            "layout.setSlotContent",
            json!({ "view_id": id(a), "slot_id": id(ghost), "content": "Empty" }),
            old(apply::set_slot_content(
                &state,
                &Subs,
                &Events,
                a,
                ghost,
                SlotContent::Empty,
            )),
        ),
        (
            "layout.setSlotContent",
            json!({ "view_id": id(ghost), "slot_id": id(slot), "content": "Usage" }),
            old(apply::set_usage_slot(
                &state, &Subs, &Events, ghost, slot, None, None,
            )),
        ),
        (
            "tab.switch",
            json!({ "window": MAIN_WINDOW_LABEL, "view_id": id(ghost) }),
            old(apply::switch_tab(
                &state,
                &Subs,
                &Events,
                MAIN_WINDOW_LABEL,
                ghost,
            )),
        ),
        (
            "tab.switch",
            json!({ "window": MAIN_WINDOW_LABEL, "view_id": id(p0) }),
            old(apply::switch_tab(
                &state,
                &Subs,
                &Events,
                MAIN_WINDOW_LABEL,
                p0,
            )),
        ),
        (
            "tab.close",
            json!({ "window": MAIN_WINDOW_LABEL, "view_id": id(ghost) }),
            old(apply::close_tab(
                &state,
                &Subs,
                &Events,
                windows.as_ref(),
                MAIN_WINDOW_LABEL,
                ghost,
            )),
        ),
        (
            "tab.rename",
            json!({ "view_id": id(ghost), "name": "work" }),
            old(apply::rename_tab(
                &state,
                &Events,
                ghost,
                "work".to_string(),
            )),
        ),
    ];
    let version = world.version();

    for (name, args, want) in cases {
        let error = refused(&receiver, &queue, name, args.clone()).await;
        assert_eq!(error.code(), ErrorCode::Conflict, "{name} {args}");
        assert_eq!(error.message(), want, "{name} {args}");
    }
    assert_eq!(world.version(), version, "nothing changes");
    assert!(
        world.windows.opened.lock().unwrap().len() <= 1,
        "no window is opened other than one pop-out"
    );
}

/// A pre-widening call still succeeds and gains that tab's rows (the daemon checks the id against the roster before relaying — ADR-0308).
#[tokio::test]
async fn old_id_calls_still_apply_and_gain_the_layout_field() {
    let (world, queue, receiver) = queued();
    let alpha = agent_id(ALPHA);
    let a = world.main_tabs().active;
    let slot = world.empty_slot(a);
    world.name(alpha, "alpha");

    let answer = ok_of(
        &receiver,
        &queue,
        "slot.assignAgent",
        json!({ "view_id": a.to_string(), "slot_id": slot.to_string(), "agent_id": alpha.to_string() }),
    )
    .await;

    assert_eq!(
        world.content(a, slot),
        Some(SlotContent::Agent {
            agent_id: alpha.to_string()
        })
    );
    assert_eq!(
        row_agents(&rows(&answer, "layout")),
        [(json!(MAIN_WINDOW_LABEL), json!("alpha"))]
    );

    let b = world.add_tab(MAIN_WINDOW_LABEL, a);
    let answer = ok_of(
        &receiver,
        &queue,
        "tab.close",
        json!({ "window": MAIN_WINDOW_LABEL, "view_id": b.to_string() }),
    )
    .await;
    assert_eq!(world.tab_ids(MAIN_WINDOW_LABEL), [a]);
    let layout = rows(&answer, "layout");
    assert_eq!(
        layout.len(),
        1,
        "the row of the tab now visible in that window"
    );
    assert_eq!(layout[0]["tab_active"], true);
}

/// `slot_of` shown in several areas: CONFLICT + those `slot_id`s; in none: NOT_FOUND + the shown names; a word picking no area: NOT_FOUND + the corner words · `slot_of` guidance.
#[tokio::test]
async fn slot_verbs_name_ambiguous_or_missing_agents_and_unpickable_words_with_candidates() {
    let (world, queue, receiver) = queued();
    let alpha = agent_id(ALPHA);
    let a = world.main_tabs().active;
    let left = world.empty_slot(a);
    let right = apply::split_slot(&world.state, &Subs, &Events, a, left, SplitDir::LeftRight)
        .expect("split");
    world.show(a, left, alpha, "alpha");
    world.show(a, right, alpha, "alpha");
    let version = world.version();

    let error = refused(&receiver, &queue, "slot.focus", json!({ "slot_of": ALPHA })).await;
    assert_eq!(error.code(), ErrorCode::Conflict);
    assert!(
        error
            .message()
            .starts_with(&format!("slot_of 'alpha' ({ALPHA}): "))
            && error
                .message()
                .contains("; pass one of these as slot_id — candidates: "),
        "{}",
        error.message()
    );
    for slot in [left, right] {
        assert!(
            error
                .message()
                .contains(&format!("slot_id={slot} (in_tab={a}")),
            "{}",
            error.message()
        );
    }
    assert_eq!(world.version(), version);
    ok_of(
        &receiver,
        &queue,
        "slot.focus",
        json!({ "slot_id": left.to_string() }),
    )
    .await;
    assert_eq!(
        world.focused(a),
        Some(left),
        "calling again with the candidate's slot_id works"
    );

    let error = refused(&receiver, &queue, "slot.close", json!({ "slot_of": BETA })).await;
    assert_eq!(error.code(), ErrorCode::NotFound);
    assert!(
        error.message().starts_with(&format!("slot_of '{BETA}': "))
            && error.message().ends_with(" — candidates: alpha"),
        "{}",
        error.message()
    );

    let error = refused(
        &receiver,
        &queue,
        "slot.split",
        json!({ "at": "left", "dir": "LeftRight" }),
    )
    .await;
    assert_eq!(error.code(), ErrorCode::NotFound);
    assert!(
        error.message().starts_with("at 'left': ")
            && error.message().ends_with(
                "try a corner word (top-left, top-right, bottom-left, bottom-right) or slot_of"
            ),
        "{}",
        error.message()
    );
    assert_eq!(
        world.slots(a).into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([left, right]),
        "nothing changes"
    );
}

/// Screening of the agent field to put in: only one of `assign` · `agent_id`, not for non-Agent content, both agent ids (the daemon turns names into ids before relaying; off-roster it refuses and the area stays — ADR-0308 · TRD S28 §11-3).
#[tokio::test]
async fn the_agent_to_put_in_a_slot_is_one_agent_id_and_only_for_agent_content() {
    let (world, queue, receiver) = queued();
    let (alpha, beta) = (agent_id(ALPHA), agent_id(BETA));
    let a = world.main_tabs().active;
    let slot = world.empty_slot(a);
    let slot_id = slot.to_string();
    let version = world.version();

    for (name, args, says) in [
        (
            "slot.assignAgent",
            json!({ "slot_id": slot_id, "agent_id": "alpha" }),
            "agent_id must be an agent id (a UUID)",
        ),
        (
            "slot.assignAgent",
            json!({ "slot_id": slot_id, "agent_id": alpha.to_string(), "assign": ALPHA }),
            "give only one of assign, agent_id",
        ),
        (
            "slot.assignAgent",
            json!({ "slot_id": slot_id }),
            "the agent to put in the slot is missing",
        ),
        (
            "layout.setSlotContent",
            json!({ "slot_id": slot_id, "content": "Empty", "assign": ALPHA }),
            "assign only applies to content=Agent",
        ),
        (
            "layout.setSlotContent",
            json!({ "slot_id": slot_id, "content": "Agent" }),
            "content=Agent needs assign",
        ),
    ] {
        let error = refused(&receiver, &queue, name, args.clone()).await;
        assert_eq!(error.code(), ErrorCode::InvalidArgument, "{name} {args}");
        assert!(
            error.message().contains(says),
            "{name} {args}: {}",
            error.message()
        );
    }
    assert_eq!(world.version(), version, "the area stays");

    ok_of(
        &receiver,
        &queue,
        "layout.setSlotContent",
        json!({ "slot_id": slot_id, "content": "Agent", "assign": BETA }),
    )
    .await;
    assert_eq!(
        world.content(a, slot),
        Some(SlotContent::Agent {
            agent_id: beta.to_string()
        }),
        "goes in with the received id as is"
    );
}

/// A tab verb chooses a tab by `tab_of` and the window follows from that tab; a given `window` must be that tab's; `tab.close` answers the row now visible in that window (empty if the window closed with it).
#[tokio::test]
async fn tab_verbs_pick_their_tab_by_agent_and_take_the_window_from_it() {
    let (world, queue, receiver) = queued();
    let (alpha, beta) = (agent_id(ALPHA), agent_id(BETA));
    let a = world.main_tabs().active;
    world.show(a, world.empty_slot(a), alpha, "alpha");
    let b = world.add_tab(MAIN_WINDOW_LABEL, a);
    let (pop, p0) = world.popout();
    let p1 = world.add_tab(&pop, p0);
    world.show(p1, world.empty_slot(p1), beta, "beta");

    assert_eq!(
        ok_of(&receiver, &queue, "tab.switch", json!({ "tab_of": BETA })).await,
        json!({})
    );
    assert_eq!(world.tabs(&pop).active, p1);
    assert_eq!(world.main_tabs().active, a, "the other window stays");

    ok_of(
        &receiver,
        &queue,
        "tab.rename",
        json!({ "tab_of": ALPHA, "name": " work " }),
    )
    .await;
    assert_eq!(world.main_tabs().tabs[0].name, "work");

    ok_of(
        &receiver,
        &queue,
        "tab.switch",
        json!({ "view_id": b.to_string() }),
    )
    .await;
    assert_eq!(world.main_tabs().active, b, "by id without window");

    let version = world.version();
    let error = refused(
        &receiver,
        &queue,
        "tab.close",
        json!({ "view_id": p1.to_string(), "window": MAIN_WINDOW_LABEL }),
    )
    .await;
    assert_eq!(error.code(), ErrorCode::Conflict);
    assert_eq!(
        error.message(),
        format!("view not found: {p1}"),
        "not that window's tab"
    );
    assert_eq!(world.version(), version);

    let answer = ok_of(&receiver, &queue, "tab.close", json!({ "tab_of": BETA })).await;
    assert_eq!(world.tab_ids(&pop), [p0]);
    let layout = rows(&answer, "layout");
    assert_no_ids(&layout);
    assert_eq!(layout.len(), 1);
    assert_eq!(
        (layout[0]["window"].clone(), layout[0]["tab_active"].clone()),
        (json!(pop), json!(true))
    );

    let answer = ok_of(
        &receiver,
        &queue,
        "tab.close",
        json!({ "view_id": p0.to_string() }),
    )
    .await;
    assert!(
        rows(&answer, "layout").is_empty(),
        "the window closed with it"
    );
    assert_eq!(world.windows.closed.lock().unwrap().as_slice(), [pop]);
}

#[tokio::test]
async fn tab_verbs_take_exactly_one_tab_and_name_ambiguous_or_missing_agents() {
    let (world, queue, receiver) = queued();
    let alpha = agent_id(ALPHA);
    let a = world.main_tabs().active;
    world.show(a, world.empty_slot(a), alpha, "alpha");
    let b = world.add_tab(MAIN_WINDOW_LABEL, a);
    world.show(b, world.empty_slot(b), alpha, "alpha");
    let version = world.version();

    for (name, base) in [
        ("tab.switch", json!({})),
        ("tab.close", json!({})),
        ("tab.rename", json!({ "name": "work" })),
    ] {
        for line in [
            json!({}),
            json!({ "view_id": a.to_string(), "tab_of": ALPHA }),
            json!({ "tab_of": "" }),
            json!({ "view_id": "alpha" }),
        ] {
            let args = with(&base, line);
            let error = refused(&receiver, &queue, name, args.clone()).await;
            assert_eq!(error.code(), ErrorCode::InvalidArgument, "{name} {args}");
        }
    }
    for name in ["tab.switch", "tab.close"] {
        let error = refused(
            &receiver,
            &queue,
            name,
            json!({ "tab_of": ALPHA, "window": "2" }),
        )
        .await;
        assert_eq!(error.code(), ErrorCode::InvalidArgument, "{name}");
    }

    let error = refused(&receiver, &queue, "tab.switch", json!({ "tab_of": ALPHA })).await;
    assert_eq!(error.code(), ErrorCode::Conflict);
    assert!(
        error.message().starts_with(&format!("tab_of 'alpha' ({ALPHA}): "))
            && error.message().ends_with(&format!(
                "; pass one of these as view_id — candidates: view_id={a} (window={MAIN_WINDOW_LABEL}, tab_index=0), view_id={b} (window={MAIN_WINDOW_LABEL}, tab_index=1)"
            )),
        "{}",
        error.message()
    );
    let error = refused(
        &receiver,
        &queue,
        "tab.rename",
        json!({ "tab_of": BETA, "name": "work" }),
    )
    .await;
    assert_eq!(error.code(), ErrorCode::NotFound);
    assert!(
        error.message().starts_with(&format!("tab_of '{BETA}': "))
            && error.message().ends_with(" — candidates: alpha"),
        "{}",
        error.message()
    );
    assert_eq!(world.version(), version);
}

async fn spawn_reply(
    world: &mut World,
    receiver: &InboundReceiver,
    args: serde_json::Value,
    spawned: Option<String>,
) -> CommandReply {
    world.mail.clear();
    let request_id = RequestId::new();
    receiver.on_command(
        envelope("agent.spawnInto", args, request_id),
        world.mail.sink(request_id),
    );
    if let Some(agent) = spawned {
        let (_cwd, _backend, answer) =
            tokio::time::timeout(Duration::from_secs(5), world.spawn_requests.recv())
                .await
                .expect("the spawn request comes")
                .expect("the spawn request comes");
        answer.send(Ok(agent)).expect("puts in the spawn answer");
    }
    world.mail.settle(1).await;
    world.mail.only()
}

/// `at` of `agent.spawnInto` brings the agent up in the empty area that word picks in the window's visible tab; unpickable · occupied · or combined with a tab · area id is rejected before the spawn. The placed tab's row is attached (id edition too).
#[tokio::test]
async fn spawn_into_at_places_the_new_agent_in_the_slot_the_word_picks() {
    let (mut world, ports) = World::build();
    let receiver = InboundReceiver::new(
        make_table(ports),
        Arc::new(RuntimeSpawner(tokio::runtime::Handle::current())) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    );
    let a = world.main_tabs().active;
    let left = world.empty_slot(a);
    let right = apply::split_slot(&world.state, &Subs, &Events, a, left, SplitDir::LeftRight)
        .expect("split");
    apply::focus_slot(&world.state, &Events, a, left).expect("focus");
    let base = json!({ "window": MAIN_WINDOW_LABEL, "cwd": "C:/work", "backend": "claude" });

    for (extra, code, says) in [
        (
            json!({ "at": "top-left", "view_id": a.to_string() }),
            ErrorCode::InvalidArgument,
            "without view_id and slot_id",
        ),
        (
            json!({ "at": "middle" }),
            ErrorCode::InvalidArgument,
            "must be one of",
        ),
        (
            json!({ "at": "left" }),
            ErrorCode::NotFound,
            "nothing was spawned",
        ),
    ] {
        let reply = spawn_reply(&mut world, &receiver, with(&base, extra.clone()), None).await;
        let error = error_of(reply);
        assert_eq!(error.code(), code, "{extra}: {}", error.message());
        assert_declared("agent.spawnInto", code);
        assert!(
            error.message().contains(says),
            "{extra}: {}",
            error.message()
        );
    }
    apply::set_slot_content(
        &world.state,
        &Subs,
        &Events,
        a,
        left,
        SlotContent::AgentList,
    )
    .expect("fill");
    let error = error_of(
        spawn_reply(
            &mut world,
            &receiver,
            with(&base, json!({ "at": "top-left" })),
            None,
        )
        .await,
    );
    assert_eq!(error.code(), ErrorCode::Conflict, "{}", error.message());
    assert!(
        error.message().contains("Not spawned") && error.message().contains("with slot.split"),
        "{}",
        error.message()
    );
    assert!(
        world.spawn_requests.try_recv().is_err(),
        "none of them spawned"
    );

    let agent = uuid::Uuid::new_v4();
    let ok = spawn_reply(
        &mut world,
        &receiver,
        with(&base, json!({ "at": "top-right" })),
        Some(agent.to_string()),
    )
    .await
    .outcome
    .expect("placement succeeds");
    assert_eq!(ok["agent_id"], json!(agent.to_string()));
    assert_eq!(
        world.content(a, right),
        Some(SlotContent::Agent {
            agent_id: agent.to_string()
        })
    );
    let layout = rows(&ok, "layout");
    assert_no_ids(&layout);
    assert_eq!(
        layout
            .iter()
            .map(|row| row["content"].clone())
            .collect::<Vec<_>>(),
        [json!("AgentList"), json!("Agent")]
    );

    let ok = spawn_reply(
        &mut world,
        &receiver,
        json!({ "cwd": "C:/work", "backend": "claude" }),
        Some(uuid::Uuid::new_v4().to_string()),
    )
    .await
    .outcome
    .expect("placed in a new tab");
    let layout = rows(&ok, "layout");
    assert_eq!(
        layout.len(),
        1,
        "the id edition (new tab) carries the placed tab's rows too"
    );
    assert_eq!(
        (layout[0]["window"].clone(), layout[0]["tab_index"].clone()),
        (json!(MAIN_WINDOW_LABEL), json!(1))
    );
    assert_eq!(world.tab_ids(MAIN_WINDOW_LABEL).len(), 2);
}

#[tokio::test]
async fn a_slot_write_reply_is_the_snapshot_of_the_lock_that_wrote_it() {
    let (world, queue, receiver) = queued();
    let a = world.main_tabs().active;
    let first = world.empty_slot(a);
    let state = world.state.clone();
    *world.events.hook.lock().unwrap() = Some(Box::new(move || {
        apply::split_slot(&state, &Subs, &Events, a, first, SplitDir::TopBottom)
            .expect("interleaved split");
    }));

    let answer = ok_of(
        &receiver,
        &queue,
        "slot.split",
        json!({ "at": "top-left", "dir": "LeftRight" }),
    )
    .await;

    assert_eq!(
        world.slots(a).len(),
        3,
        "the interleaved write entered the model"
    );
    assert_eq!(
        rows(&answer, "layout").len(),
        2,
        "the answer is the two areas at the moment of the split"
    );
}

#[tokio::test]
async fn an_agent_id_in_another_uuid_spelling_is_written_in_its_canonical_spelling() {
    let (world, queue, receiver) = queued();
    let alpha = agent_id(ALPHA);
    let a = world.main_tabs().active;
    let slot = world.empty_slot(a);

    ok_of(
        &receiver,
        &queue,
        "slot.assignAgent",
        json!({ "slot_id": slot.to_string(), "agent_id": alpha.to_string().to_uppercase() }),
    )
    .await;

    assert_eq!(
        world.content(a, slot),
        Some(SlotContent::Agent {
            agent_id: alpha.to_string()
        })
    );
}

/// With only `view_id`, `agent.spawnInto` places in that tab's window; a `window` that is not that tab's is rejected before the spawn — an agent with nowhere to be placed must not survive.
#[tokio::test]
async fn spawn_into_a_tab_takes_its_window_and_refuses_another_before_spawning() {
    let (mut world, ports) = World::build();
    let receiver = InboundReceiver::new(
        make_table(ports),
        Arc::new(RuntimeSpawner(tokio::runtime::Handle::current())) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    );
    let (pop, p0) = world.popout();

    let error = error_of(
        spawn_reply(
            &mut world,
            &receiver,
            json!({ "window": MAIN_WINDOW_LABEL, "view_id": p0.to_string(), "cwd": "C:/work", "backend": "claude" }),
            None,
        )
        .await,
    );
    assert_eq!(error.code(), ErrorCode::Conflict, "{}", error.message());
    assert!(
        error.message().contains("Not spawned"),
        "{}",
        error.message()
    );
    assert!(world.spawn_requests.try_recv().is_err(), "did not spawn");

    let agent = uuid::Uuid::new_v4();
    let ok = spawn_reply(
        &mut world,
        &receiver,
        json!({ "view_id": p0.to_string(), "cwd": "C:/work", "backend": "claude" }),
        Some(agent.to_string()),
    )
    .await
    .outcome
    .expect("placed in that tab's window");
    assert_eq!(
        world.content(p0, world.slots(p0)[0]),
        Some(SlotContent::Agent {
            agent_id: agent.to_string()
        })
    );
    let layout = rows(&ok, "layout");
    assert_eq!(layout.len(), 1);
    assert_eq!(layout[0]["window"], json!(pop));
}
