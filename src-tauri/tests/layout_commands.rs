//! 셸 명령 표(`layout::commands`) + 인바운드 수신기(`daemon_client::inbound`) 통합 테스트 —
//! 데몬·창·소켓 0 (ADR-0012 seam 격리).
//!
//! ★이 파일이 `tests/`(통합 타깃)에 있는 이유★: 재는 대상이 **실 소켓·실 `AppHandle` 없이는 못 세우는
//! 배선**이라, 그 자리를 포트로 끊어 세우는 통합 하네스가 제자리다(그래서 하네스 자신은 데몬·창·소켓을
//! 하나도 안 쓴다). 순수 단위 단언은 모듈 옆 `#[cfg(test)]` 로 가고 그쪽은 `--test lib_unit` 으로 돈다
//! (그 타깃을 세운 결정 = ADR-0174 · 현황 = CLAUDE.md 「빌드·검증 명령」).
//! 실행: `cargo test -p engram-dashboard --test layout_commands`(자식 프로세스를 하나도 안 띄우므로
//! `-- --test-threads=4` 를 붙이지 않는다 — 판정 규칙 정본 = CLAUDE.md 「빌드·검증 명령」).
//!
//! ★`layout_apply.rs` 와 나누는 기준★: 저쪽은 **적용 서비스의 락 위치**(어느 포트가 락 안/밖에서 불리나)를
//! 재고, 여기는 **봉투가 그 서비스까지 가는 길**(이름 배달 · 인자 검문 · 답장 상관 · 연결 태스크 밖 실행)을
//! 잰다. 그래서 이 파일의 가짜 포트에는 락 프로브가 없다 — 같은 것을 두 번 재면 한쪽이 낡는다.
//!
//! ## ★연결 태스크를 안 막는다는 것을 어떻게 재나★
//! 두 하네스가 서로 다른 실패 모드를 잡는다.
//! - [`Queued`] — 태스크를 **쥐고만** 있는 spawner. `on_command` 가 반환한 시점에 적용이 **아직 안 일어났음**을
//!   본다(인라인 실행이면 이 단언이 깨진다).
//! - `RuntimeSpawner` + 실 런타임 — 합성 명령(`agent.spawnInto`)이 데몬 답장을 기다리는 동안 **호출자
//!   태스크가 계속 돈다**는 것을 본다. 인라인이면 호출자가 스폰 요청을 서비스하지 못해 그 자리에서 교착이다
//!   (ADR-0081 「relay 적용은 액터 밖」이 막는 self-deadlock 그대로).
//!
//! ## ★무엇이 실코드로 덮이고 무엇이 안 덮이나★
//! 잔여 목록의 정본은 아래 「연결 arm 이 걷는 바이트 경로」 절 머리다 — 이 헤더에 베끼지 않는다(두 곳에 적으면
//! 한쪽이 낡는다).

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
use engram_dashboard_protocol::{
    command_request_id, event_reply_request_id, AgentCommand, AgentEvent,
};

use engram_dashboard_lib::commands::popout::PopupCounter;
use engram_dashboard_lib::daemon_client::connection::{
    accept_inbound, outcome_sink, registration_command, ConnectionCommand, OutcomeSender,
};
use engram_dashboard_lib::daemon_client::inbound::{
    BoxedTask, InboundReceiver, InboundSlot, RuntimeSpawner, TaskSpawner, ViewCommandPort,
};
use engram_dashboard_lib::layout::apply;
use engram_dashboard_lib::layout::commands::{
    make_table, LayoutPorts, RestoreAnswerArgs, RestoreStatusArgs, SettingsGetArgs,
    SettingsResetArgs, SettingsSchemaArgs, SettingsSetArgs, SlotPopoutArgs, SplitListArgs,
    SplitSetRatioArgs, WindowGetThemeArgs, WindowListArgs, WindowSetThemeArgs, CATALOG_VERSION,
    COMMAND_SPECS,
};
use engram_dashboard_lib::layout::geometry::Insets;
use engram_dashboard_lib::layout::{
    tree, AgentSpawner, LayoutEvents, LayoutState, SlotContent, SplitDir, SplitRatioApplied,
    SplitRatioOutcome, SubscriptionSync, UiMetrics, ViewManager, ViewSnapshot, WindowBounds,
    WindowHost, WindowTabsPayload, MAIN_WINDOW_LABEL,
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
use engram_dashboard_lib::view_commands::{
    reserved_names, ViewArgSchema, ViewCommandBridge, ViewCommandDecl, ViewCommandHelp,
    ViewCommandRequest, ViewDispatch, ViewEffect, VIEW_HOP_MARGIN, VIEW_REPLY_DEADLINE,
};

// ── 가짜 포트 ────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Events;

impl LayoutEvents for Events {
    fn layout_updated(&self, _snapshot: &ViewSnapshot) {}
    fn window_tabs_updated(&self, _tabs: &WindowTabsPayload) {}
}

#[derive(Default)]
struct Subs;

impl SubscriptionSync for Subs {
    fn resync(&self, _mgr: &ViewManager) {}
}

#[derive(Default)]
struct Windows {
    opened: Mutex<Vec<String>>,
    closed: Mutex<Vec<String>>,
}

impl WindowHost for Windows {
    fn open(&self, label: &str) -> Result<(), String> {
        self.opened.lock().unwrap().push(label.to_string());
        Ok(())
    }

    fn close(&self, label: &str) {
        self.closed.lock().unwrap().push(label.to_string());
    }

    // ★main 을 특례로 참이라 답한다★ — 정적 config 창이라 이 가짜가 연 적이 없는데 실 앱에서는 항상 떠 있다.
    fn is_open(&self, label: &str) -> bool {
        label == MAIN_WINDOW_LABEL
            || (self.opened.lock().unwrap().iter().any(|l| l == label)
                && !self.closed.lock().unwrap().iter().any(|l| l == label))
    }
}

/// 스폰 요청 한 건 — cwd · 고른 백엔드 · 그 답을 넣을 자리.
type SpawnRequest = (
    String,
    Option<engram_dashboard_protocol::AgentBackendKind>,
    oneshot::Sender<Result<String, String>>,
);

/// ★답을 **다른 태스크가** 넣어 줘야 끝난다★ — 데몬 왕복의 성질을 그대로 흉내낸다. 이게 있어야
/// 「적용이 호출자 태스크를 붙들고 있나」를 잴 수 있다(가짜가 즉답하면 그 질문 자체가 사라진다).
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
                .map_err(|_| "테스트 스폰 채널이 닫혔다".to_string())?;
            wait.await
                .map_err(|_| "테스트 스폰 응답이 없다".to_string())?
        })
    }
}

/// 테마 밀기의 창 쪽 대역 — 창 명단을 정해 두고 보낸 것을 순서대로 남긴다. `refuse` = 받지 못하는 창(시도는
/// 남긴다).
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

    /// 지금까지 보낸 것을 비우며 돌려준다 — `(창, 테마)`.
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

/// [`RecordingWindows`] 가 받지 못하는 창에 주는 사유.
const WINDOW_GONE: &str = "창이 이미 닫혔다";

/// 웹뷰는 떠 있지만 레이아웃 모델에는 아직 없는 팝아웃 — 밀기는 이 창에 전역 값을 보낸다. 하네스의 발급기가 닿지
/// 않는 번호라 `window.create` 가 만든 창과 겹치지 않는다.
const UNMODELED_POPOUT_LABEL: &str = "slot-popup-99";

fn sent(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(label, theme)| (label.to_string(), theme.to_string()))
        .collect()
}

/// 설정 알림 포트 대역 — 창도 Tauri 도 없이 「무엇을 언제 알렸나」만 남긴다.
#[derive(Default)]
struct FakeSettingsEvents {
    changed: Mutex<Vec<SettingsSnapshot>>,
    theme_pushes: Mutex<usize>,
    /// `changed` 가 불린 스레드 — 쓰기 본문이 어디서 돌았나.
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

/// 시험 하나 몫의 셸 config 폴더 — 적재는 폴더를 만들지 않고 첫 쓰기가 만든다. 끝나면 치운다.
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
                .expect("시계")
                .as_nanos()
        )))
    }

    /// 셸 `setup` 이 하는 그대로 — 적재 뒤 쓰기를 연다.
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

// ── 태스크 spawner 하네스 ────────────────────────────────────────────────────

/// 태스크를 받아 **쥐고만** 있는다 — 테스트가 [`Queued::drain`] 으로 직접 돌린다.
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

// ── 패닉 훅 잠깐 끄기 ───────────────────────────────────────────────────────

/// 패닉 메시지를 삼킨 채 `body` 를 끝까지 돌리고 원래 훅을 되돌린다 — 같은 모양의 동기 판
/// `engram_dashboard_command::testing::with_quiet_panic_hook` 의 async 판이다.
///
/// ★구간을 의도된 패닉 하나로 좁게 잡고, 되돌리기는 unwind 를 잡은 **뒤에** 한다 — RAII(`Drop`)로 옮기지
/// 말 것★. 두 사유의 정본은 그 헬퍼의 doc 과 본문 주석이다.
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
            .unwrap_or("<문자열이 아닌 패닉 페이로드>");
        eprintln!("조용한 패닉 훅 구간 안에서 본문이 패닉했다(원래 출력은 삼켜졌다): {message}");
        std::panic::resume_unwind(payload)
    })
}

// ── 답장 수거 ────────────────────────────────────────────────────────────────

#[derive(Default, Clone)]
struct Mailbox(Arc<Mutex<Vec<CommandReply>>>);

impl Mailbox {
    fn sink(&self, request_id: RequestId) -> ReplySink {
        let seen = Arc::clone(&self.0);
        ReplySink::new(request_id, move |reply| {
            seen.lock().unwrap().push(reply);
        })
    }

    /// `InboundReceiver::accept` 이 받는 배달 콜백 — 연결 루프가 소켓에 되쓰는 자리에 해당한다.
    fn deliver(&self) -> impl FnOnce(CommandReply) + Send + 'static {
        let seen = Arc::clone(&self.0);
        move |reply| {
            seen.lock().unwrap().push(reply);
        }
    }

    fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    /// 같은 하네스로 명령을 두 번 부르는 테스트용 — [`Mailbox::only`] 가 「정확히 하나」를 요구한다.
    fn clear(&self) {
        self.0.lock().unwrap().clear();
    }

    fn only(&self) -> CommandReply {
        let seen = self.0.lock().unwrap();
        assert_eq!(seen.len(), 1, "한 request_id 에 답장은 정확히 하나다");
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

    /// 답장 `n` 개가 다 올 때까지 이 태스크를 양보한다 — 안 오면 실패로 끝낸다(hang 금지).
    async fn settle(&self, n: usize) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while self.len() < n {
            assert!(
                tokio::time::Instant::now() < deadline,
                "답장 {n} 개를 기다렸는데 {} 개만 왔다",
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
        // 목적지 토큰 — 이 홉 앞으로 온 것이다(앞 홉이 자기 명부의 답으로 적어 넣는 칸).
        owner: OwnerToken::new("shell"),
        proto_ver: CATALOG_VERSION,
        args,
    }
}

// ── 하네스 ───────────────────────────────────────────────────────────────────

struct World {
    state: LayoutState,
    windows: Arc<Windows>,
    /// 표가 쥔 것과 같은 테마 손잡이 — 창 쪽은 `theme_windows`(main · 모델 밖 팝아웃).
    themes: ThemeControl,
    theme_windows: Arc<RecordingWindows>,
    /// 표가 쥔 것과 같은 서비스 — 시험이 명령 밖에서 값을 확인한다.
    settings: Arc<SettingsService>,
    settings_events: Arc<FakeSettingsEvents>,
    /// 쥐고만 있다 — 시험이 끝나면 폴더를 치운다.
    _config: ConfigDir,
    mail: Mailbox,
    spawn_requests: mpsc::UnboundedReceiver<SpawnRequest>,
    /// 표가 쥔 조율자의 복원 서비스 — 시험이 부팅 단계 ⑥ 을 흉내 낸다(`set_boot`).
    restore_service: Arc<RestoreService>,
    /// 표가 쥔 것과 같은 조율자 — 시험이 셸 setup 끝처럼 포트를 꽂는다.
    restore: Arc<RestoreCoordinator>,
}

impl World {
    fn build() -> (World, LayoutPorts) {
        let state = LayoutState::new();
        let windows = Arc::new(Windows::default());
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
        // 실물 발급기 — label 단조성은 닫힌 label 재-build 를 막는 계약이라 가짜로 대체하지 않는다. 복원 조율자도
        //   같은 것을 쓴다(운영 조립과 같다 — 새 팝아웃 label 이 떠 있는 창과 겹치지 않는다).
        let labels = Arc::new(PopupCounter::default());
        let restore_service = Arc::new(RestoreService::new());
        let restore = Arc::new(RestoreCoordinator::new(
            Arc::clone(&restore_service),
            state.clone(),
            // 기록기가 없다 — 답은 곧바로 `durable:false` 다(가드 아래 셸과 같다).
            Arc::new(StateSession::default()),
            labels.clone(),
        ));
        let ports = LayoutPorts {
            state: state.clone(),
            subs: Arc::new(Subs),
            events: Arc::new(Events),
            windows: Arc::clone(&windows) as Arc<dyn WindowHost>,
            labels,
            spawner: Arc::new(DaemonSpawner { requests: tx }),
            themes: themes.clone(),
            settings: Arc::clone(&settings),
            settings_events: Arc::clone(&settings_events) as Arc<dyn SettingsEvents>,
            restore: Arc::clone(&restore),
        };
        (
            World {
                state,
                windows,
                themes,
                theme_windows,
                settings,
                settings_events,
                _config: config,
                mail: Mailbox::default(),
                spawn_requests,
                restore_service,
                restore,
            },
            ports,
        )
    }

    fn main_tabs(&self) -> WindowTabsPayload {
        apply::list_tabs(&self.state, MAIN_WINDOW_LABEL).expect("main 창은 항상 있다")
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
            .expect("빈 슬롯")
    }

    /// agent 가 든 한 칸 — 반환 = (탭, 그 슬롯).
    ///
    /// agent 콘텐츠를 쓰는 이유는 운영 형태를 그대로 태우려는 것뿐이다 — ★구독 마이그레이션은 여기서 안
    /// 잰다★(이 파일의 `Subs` 는 no-op 이고, 실 `OutputRouter` 로 재는 자리는 `layout_apply.rs` 다).
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
        .expect("콘텐츠 배치");
        (view, slot)
    }
}

/// 표를 쥐고만 있는 수신기 — 적용 시점을 테스트가 고른다.
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

/// 큐에 쌓인 태스크를 전부 끝까지 돌린다.
async fn run(queue: &Queued) {
    for task in queue.drain() {
        task.await;
    }
}

/// 봉투 하나를 넣고 끝까지 돌린 답장.
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
        "답장은 요청과 같은 상관 키를 달고 온다"
    );
    reply
}

fn error_of(reply: CommandReply) -> CommandError {
    reply.outcome.expect_err("오류 답장")
}

// ── (A) 선언과 표 ────────────────────────────────────────────────────────────

/// 표에 실제로 꽂힌 이름 전량 — ★손 목록이 아니라 골든이다★: 선언을 늘리면서 조립을 빠뜨리면
/// (또는 그 반대) 여기서 걸린다.
#[test]
fn the_table_holds_exactly_the_declared_commands() {
    let (_world, ports) = World::build();
    let table = make_table(ports);
    let names: Vec<&str> = table.specs().map(|s| s.name).collect();
    assert_eq!(
        names,
        vec![
            "agent.spawnInto",
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
            "tab.list",
            "tab.rename",
            "tab.switch",
            "window.close",
            "window.create",
            "window.getTheme",
            "window.list",
            "window.setTheme",
        ]
    );
    assert_eq!(names.len(), COMMAND_SPECS.len(), "선언은 있는데 안 꽂힌 것");
}

/// ★세대 번호를 손으로 못 박는다★ — 매크로 계약이 「선언이 바뀌면 손으로 올린다」이고, 안 올리면 어휘가
/// 다른 두 셸이 같은 세대를 보고해 진단이 거짓말을 한다. 위 골든 목록은 **이름만** 보므로 이것을 못 잡는다.
/// 세 줄이 함께 움직여야 한다: 세대 · 선언 수 · 새 명령이 주장하는 `since`(코어 `command_alphabet.rs` 와 같은 형태).
#[test]
fn the_catalog_generation_is_pinned_to_the_declaration_set() {
    // ★이름 수가 안 늘어도 올라간다★ — 세대 4 는 `ui.refresh` 의 **답 모양**이, 세대 5 는
    //   `agent.spawnInto` 의 `backend` 가 받는 **어휘**가, 세대 6 은 그 칸의 **정책**(아는 낱말 하나를 이
    //   표면이 안 만든다)이, 세대 7 은 그 정책이 **뒤집힌 것**(그 낱말을 이 표면이 실제로 만든다 —
    //   2026-09-22 · ADR-0219)이 바뀐 세대다(넷 다 선언이라 올린다). 세대 8 은 이름이 는 세대다
    //   (`split.setRatio`·`split.list` — ADR-0227). 세대 9 는 `layout.setSlotContent` 의 **어휘와 칸**이
    //   는 세대다(`content=Usage` + `show_claude`·`show_codex` — TRD S21 usage-limit-slot §1-7). 세대 10 은
    //   이름이 넷 늘고(`settings.*`) `ui.refresh` 답의 `theme` 출처가 설정으로 바뀐 세대다(TRD S21-storage §5-4).
    //   세대 11 은 이름이 둘 는 세대다(`restore.*` — TRD S21-storage §6-7). 세대 12 는 `restore.status` 의
    //   **답 모양**이 바뀐 세대다(`state_file` — TRD S21-storage §6-5). 세대 13 도 그 답 모양이다(`saves` — 같은
    //   절 · 가드 ⅱ 안내(ADR-0276)를 나르는 칸 — 칸 자체는 구현 · 세션 판단). 세대 14 는 이름이 둘 늘고
    //   (`window.setTheme`·`window.getTheme`) 하나가 빠진 세대다(`ui.refresh` — TRD S21-storage §5-6 · §5-7).
    //   세대 15 는 그 둘의 `window` 칸이 받는 **어휘**가 준 세대다(`agent-tree` — ADR-0225 · summary 는
    //   `window.getTheme` · `restore.status` 것만 바뀌었다).
    assert_eq!(CATALOG_VERSION, 15);
    assert_eq!(COMMAND_SPECS.len(), 26);
    assert_eq!(
        SlotPopoutArgs::SPEC.since,
        2,
        "세대 2에 들어온 명령이 1부터 있었다고 광고하면 안 된다"
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

/// 창별 테마는 상태가 됐다 — 파일을 다시 읽는 명령은 표에서도 링커 수집에서도 사라졌다(TRD S21-storage §5-7).
#[test]
fn ui_refresh_is_gone_from_the_catalog() {
    assert!(spec_of("ui.refresh").is_none());
}

/// ★등록 패킷의 `help` 하나가 명부 상한을 넘으면 데몬이 패킷을 통째로 거절한다★(`Roster::register` — 한 이름도
/// 넣지 않는다) — 그러면 셸의 명령 전부가 `UNKNOWN_COMMAND` 다. 요약(선언의 doc 주석)이 자라는 쪽은 셸이고 상한은
/// 데몬에만 있어, 여기서 재지 않으면 실제로 붙어 봐야 드러난다. 재는 것은 등록이 싣는 문자열 그대로다(`decls`).
#[test]
fn every_registered_declaration_fits_the_roster_help_cap() {
    let (_world, ports) = World::build();
    for decl in make_table(ports).decls() {
        assert!(
            decl.help.len() <= Roster::MAX_HELP_BYTES,
            "{}: help {} 바이트 > 상한 {}",
            decl.name,
            decl.help.len(),
            Roster::MAX_HELP_BYTES
        );
    }
}

#[test]
fn every_declaration_carries_a_usable_json_shape() {
    for spec in COMMAND_SPECS {
        lint_spec(spec).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            !spec.summary.trim().is_empty(),
            "{}: 요약이 비었다",
            spec.name
        );
        for (label, text) in [("args", spec.args_schema), ("ok", spec.ok_schema)] {
            let shape: serde_json::Value = serde_json::from_str(text).expect("스키마는 JSON");
            assert_eq!(
                shape["type"], "object",
                "{}: {label} 는 객체여야 한다",
                spec.name
            );
        }
        assert_eq!(
            spec_of(spec.name).map(|s| s.name),
            Some(spec.name),
            "{}: 링커 수집에서 안 보인다",
            spec.name
        );
    }
}

/// ★재는 범위 = 이 바이너리에 **링크된** crate 의 선언끼리다★ — 링커 수집(`duplicate_command_names`)은
/// 링크된 선언 객체만 본다. 셸이 `agent.*` 계열에 이름을 더하므로(`agent.spawnInto`) agent 선언과 겹치면
/// 어느 주인이 이기는지 알 수 없어지는데, 셸 lib 은 agent 를 의존하지 않는다. agent 선언이 여기 보이는
/// 것은 데몬 dev 의존을 타고 온 결과다(이 파일이 데몬 상수를 읽는다 · 실측 2026-10-06 — `spec_of("agent.new")`
/// 가 `Some`). 그 경로가 끊기면 이 시험은 그 겹침을 말없이 못 본다.
/// 셸 · 데몬 이름 겹침의 벽은 데몬의 등록 반려다(ADR-0270 결정 2).
#[test]
fn no_two_declarations_claim_the_same_name() {
    assert_eq!(duplicate_command_names(), Vec::<&str>::new());
}

// ── (B) 배달 ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_write_command_reaches_the_apply_service() {
    let (world, queue, receiver) = queued();
    let before = world.main_tabs().tabs.len();

    let reply = call(
        &receiver,
        &queue,
        &world.mail,
        "tab.create",
        json!({ "window": MAIN_WINDOW_LABEL, "name": "새 탭" }),
    )
    .await;

    let ok = reply.outcome.expect("생성 성공");
    let after = world.main_tabs();
    assert_eq!(after.tabs.len(), before + 1);
    assert_eq!(
        ok["view_id"].as_str(),
        Some(after.active.to_string().as_str()),
        "새로 만든 탭이 활성이고 그 id 가 답장에 실린다"
    );
}

#[tokio::test]
async fn a_read_command_answers_from_the_same_authority() {
    let (world, queue, receiver) = queued();
    let created = call(&receiver, &queue, &world.mail, "window.create", json!({}))
        .await
        .outcome
        .expect("창 생성");
    let label = created["window"].as_str().expect("label").to_string();
    assert_eq!(
        world.windows.opened.lock().unwrap().as_slice(),
        &[label.clone()]
    );

    let mail = Mailbox::default();
    let listed = call(&receiver, &queue, &mail, "window.list", json!({}))
        .await
        .outcome
        .expect("창 목록");
    let windows: Vec<&str> = listed["windows"]
        .as_array()
        .expect("배열")
        .iter()
        .map(|v| v.as_str().expect("label"))
        .collect();
    assert!(
        windows.contains(&label.as_str()),
        "방금 만든 창이 조회에 보인다: {windows:?}"
    );
}

#[tokio::test]
async fn an_unknown_command_name_is_answered_not_dropped() {
    let (world, queue, receiver) = queued();
    let err = error_of(call(&receiver, &queue, &world.mail, "tab.teleport", json!({})).await);
    assert_eq!(err.code(), ErrorCode::UnknownCommand);
    assert!(err.message().contains("tab.teleport"));
}

/// 형식이 깨진 id 는 적용 **전에** 반려된다 — 없는 id 와 같은 답을 받으면 호출자가 멀쩡한 목록을 뒤진다.
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

/// 적용 서비스가 거절한 것은 `CONFLICT` + **그 사유 문구**로 나간다(코드 하나 · 사유는 문구).
#[tokio::test]
async fn an_unapplicable_request_keeps_the_services_reason() {
    let (world, queue, receiver) = queued();
    let err = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "window.close",
            json!({ "window": MAIN_WINDOW_LABEL }),
        )
        .await,
    );
    assert_eq!(err.code(), ErrorCode::Conflict);
    assert!(!err.message().is_empty(), "사유가 비어 나가면 고칠 수 없다");
    assert!(
        world.windows.closed.lock().unwrap().is_empty(),
        "거절됐으면 OS 창도 안 닫는다"
    );
}

/// 태그와 곁칸이 어긋나면 조용히 고치지 않는다.
#[tokio::test]
async fn slot_content_refuses_a_contradictory_pair() {
    let (world, queue, receiver) = queued();
    let tabs = world.main_tabs();
    let view = tabs.active;
    let slot = apply::get_view(&world.state, view)
        .expect("view")
        .slot_spatial
        .first()
        .expect("슬롯 하나")
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

/// `show_*` 는 `content=Usage` 에만 — 다른 종류에 붙으면 호출자는 그 회사를 켠 줄 안다.
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
        "반려는 아무것도 안 바꾼다"
    );
}

/// 버스의 사용량 슬롯 — 뺀 칸은 처음엔 `true`, 그 뒤로는 지금 값이다(한 칸 토글이 다른 칸을 안 건드린다).
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
    placed.outcome.expect("사용량 슬롯 배치");
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
    .expect("codex 끔");
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
    .expect("claude 끔");
    assert_eq!(
        usage_at(&world),
        Some(SlotContent::Usage {
            show_claude: false,
            show_codex: false
        }),
        "앞서 끈 codex 가 되살아나지 않는다"
    );
}

/// ★버스에서 창을 떼어낸다★ — 클릭 없이도 슬롯이 자기 창으로 나가고, 답장이 그 창과 새 탭을 함께 준다
/// (둘 다 없으면 호출자가 방금 만든 창을 다시 찾아 헤맨다).
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
    .expect("분리 성공");

    let window = ok["window"].as_str().expect("창 label").to_string();
    assert_eq!(
        world.windows.opened.lock().unwrap().as_slice(),
        &[window.clone()],
        "to_window 를 빼면 새 창이 열린다"
    );
    let tabs = apply::list_tabs(&world.state, &window).expect("새 창 탭");
    assert_eq!(
        ok["new_view_id"].as_str(),
        Some(tabs.active.to_string().as_str()),
        "답장의 new_view_id 가 그 창의 활성 탭이다"
    );
    // ★인자의 `view_id`(원본)와 답의 `new_view_id`(도착지)는 다른 것을 가리킨다★ — 답을 그대로 되먹여
    //   두 번 부르는 호출자가 원본 대신 방금 만든 탭을 집는 사고를 이름으로 막는다.
    assert!(
        ok.get("view_id").is_none(),
        "답에 `view_id` 를 두면 인자와 같은 이름이 반대쪽을 뜻하게 된다: {ok}"
    );
    assert!(
        !world.slots(view).contains(&slot),
        "MOVE 다 — 원본 슬롯은 남지 않는다"
    );
}

/// `to_window` 를 주면 그 창의 새 탭이 된다 — 창이 늘지 않는다.
#[tokio::test]
async fn popout_into_a_named_window_adds_a_tab_there() {
    let (world, queue, receiver) = queued();
    let target = call(&receiver, &queue, &world.mail, "window.create", json!({}))
        .await
        .outcome
        .expect("창 생성")["window"]
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
    .expect("분리 성공");

    assert_eq!(ok["window"].as_str(), Some(target.as_str()));
    assert_eq!(
        world.windows.opened.lock().unwrap().len(),
        opened_before,
        "기존 창 타깃은 창을 새로 열지 않는다"
    );
    let tabs = apply::list_tabs(&world.state, &target).expect("대상 창 탭");
    assert_eq!(tabs.tabs.len(), 2, "빈 탭 + 옮겨온 탭");
    assert_eq!(
        ok["new_view_id"].as_str(),
        Some(tabs.active.to_string().as_str())
    );
}

/// 빈 슬롯도 버스로 떼어낸다 — 새 창이 열리고 원본 슬롯은 닫힌다. // ADR-0228
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
    .expect("분리 성공");

    let window = ok["window"].as_str().expect("창 label").to_string();
    assert_eq!(
        world.windows.opened.lock().unwrap().as_slice(),
        &[window.clone()]
    );
    let tabs = apply::list_tabs(&world.state, &window).expect("새 창 탭");
    assert_eq!(
        ok["new_view_id"].as_str(),
        Some(tabs.active.to_string().as_str())
    );
    assert!(
        !world.slots(view).contains(&slot),
        "MOVE 다 — 원본 슬롯은 남지 않는다"
    );
}

/// 없는 슬롯은 `CONFLICT` + 서비스의 사유 문구로 반려된다(창도 안 연다).
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
    assert!(err.message().contains("slot 없음"), "{}", err.message());
    assert!(
        world.windows.opened.lock().unwrap().is_empty(),
        "거절됐으면 창도 안 연다"
    );
}

// ── (B) 창 테마 — window.getTheme · window.setTheme (TRD S21-storage §5-6) ──────────────
//
// 유효 테마 · 설정 · 모델은 실물이고(설정 = 임시 폴더) 창 쪽만 기록하는 가짜다. 재는 것 = 봉투가 표가 쥔 **그
// 손잡이**에 닿나 · 답 모양 · null 해제 · 오류 코드 · 쓰기 뒤 밀기. 유효 값 계산은 `theme` 옆 단위 시험
// (`--test lib_unit`)이 잰다 — 여기서 다시 재지 않는다.

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
    .expect("성공 답장");

    assert_eq!(
        ok,
        json!({ "theme": null, "effective": DEFAULT_THEME.as_wire() }),
        "자기 테마가 없는 창 = theme null · effective 는 전역 값"
    );
}

/// 창 쓰기가 그 창의 모델 항목에 들고, 쓴 뒤 모든 창을 다시 민다 — 자기 테마가 없는 창은 그때의 `theme.default`.
#[tokio::test]
async fn window_set_theme_writes_the_window_and_pushes_every_window() {
    let (world, queue, receiver) = queued();
    world.settings.set(THEME_DEFAULT, "light").expect("쓰기");

    let ok = call(
        &receiver,
        &queue,
        &world.mail,
        "window.setTheme",
        json!({ "window": MAIN_WINDOW_LABEL, "theme": "e-ink" }),
    )
    .await
    .outcome
    .expect("성공 답장");

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
    .expect("성공 답장");
    assert_eq!(read, ok, "읽기가 쓴 값을 그대로 돌려준다");
}

/// ★`null` 은 지우기다★ — 그 창은 다시 `theme.default` 를 따른다. 레이아웃 번호는 안 움직인다(창 속성 번호만).
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
    .expect("성공 답장");
    assert_eq!(
        set,
        json!({ "theme": "light", "effective": "light" }),
        "대소문자를 가리지 않고 정규 철자로 답한다"
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
    .expect("성공 답장");

    assert_eq!(cleared, json!({ "theme": null, "effective": "dark" }));
    {
        let mgr = world.state.0.lock().unwrap();
        assert_eq!(mgr.window_attrs(MAIN_WINDOW_LABEL).unwrap().theme, None);
        assert_eq!(mgr.attrs_rev(), attrs_rev + 1);
    }
    assert_eq!(
        world.main_tabs().version,
        before.version,
        "창 테마는 레이아웃 번호를 안 올린다"
    );
    assert_eq!(
        world.theme_windows.take(),
        sent(&[
            (MAIN_WINDOW_LABEL, "dark"),
            (UNMODELED_POPOUT_LABEL, "dark")
        ])
    );
}

/// ★`theme` 칸 부재는 지우기가 아니다★ — 오타 필드 하나가 조용히 창 테마를 지우면 안 된다.
#[tokio::test]
async fn window_set_theme_without_the_theme_field_is_refused() {
    let (world, queue, receiver) = queued();
    world
        .themes
        .set(MAIN_WINDOW_LABEL, Some(UiTheme::Light))
        .expect("창 테마");
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
        "거절이면 밀지 않는다"
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
            "받는 값을 알려 준다: {}",
            err.message()
        );
    }
    assert_eq!(world.themes.get(MAIN_WINDOW_LABEL).unwrap().own, None);
    assert!(world.theme_windows.take().is_empty());
}

/// 없는 창은 이웃 `window.*` 명령과 같은 `CONFLICT` 다.
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

/// ADR-0225: 트리 전용 창(`agent-tree`)은 걷혔다 — 그 label 도 다른 모르는 창과 같은 `CONFLICT` 이고, 쓰기는 아무
/// 창도 만들지 않고 밀지도 않는다.
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

/// ★그 창이 못 받았으면 성공이 아니다★(ADR-0166 결정 6) — `INTERNAL` 이고 문구가 그 창과 사유를 말한다. 쓴 값은
/// 남아 `window.getTheme` 이 새 값을 답한다.
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
    assert_eq!(world.theme_windows.take().len(), 2, "모든 창에 시도한다");

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
    .expect("성공 답장");
    assert_eq!(read, json!({ "theme": "light", "effective": "light" }));
}

/// 다른 창만 못 받았으면 그 쓰기는 성공이다.
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
    .expect("성공 답장");

    assert_eq!(ok, json!({ "theme": "light", "effective": "light" }));
    assert_eq!(world.theme_windows.take().len(), 2);
}

/// 팝아웃의 테마는 그 창의 모델 항목에 든다 — 창을 닫으면 함께 사라진다.
#[tokio::test]
async fn a_popout_theme_lives_and_dies_with_its_window() {
    let (world, queue, receiver) = queued();
    let created = call(&receiver, &queue, &world.mail, "window.create", json!({}))
        .await
        .outcome
        .expect("성공 답장");
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
    .expect("성공 답장");
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
    .expect("성공 답장");
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

// ── (B) 셸 설정 — settings.get · set · reset · schema (TRD S21-storage §5-4) ──────────
//
// 서비스는 실물이고(임시 폴더 위 — 셸 setup 처럼 쓰기를 연 것) 알림만 가짜다. 재는 것 = 봉투가 표가 쥔 **그
// 서비스**에 닿나 · 서비스의 오류 종류가 같은 이름의 코드로 나가나 · **실제로 바뀐 쓰기에만** 알리나. 값
// 정규화·파일 관용은 서비스 옆 단위 시험(`--test lib_unit`)이 잰다 — 여기서 다시 재지 않는다.

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
        .expect("성공 답장");
    assert_eq!(all["rev"], 0);
    let items = all["items"].as_array().expect("items");
    assert_eq!(items.len(), 12);
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
    .expect("성공 답장");
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

/// ★표가 쥔 서비스가 사람 경로와 **같은 인스턴스**다★ — 다른 인스턴스면 LLM 이 바꾼 값을 화면이 못 본다.
/// 그리고 디스크에 남아 다음 적재(= 재시작)에 읽힌다.
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
        reply.outcome.expect("성공 답장"),
        json!({"rev": 1, "key": "theme.default", "value": "e-ink", "changed": true}),
        "답의 value 는 정규형이다"
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
        "전역 테마가 바뀌면 창마다 다시 민다"
    );
    assert_eq!(
        SettingsService::load_from_dir(&world._config.0)
            .effective(THEME_DEFAULT)
            .as_deref(),
        Some("e-ink"),
        "재시작을 넘긴다"
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
        reply.outcome.expect("성공 답장"),
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

    assert_eq!(reply.outcome.expect("성공 답장")["value"], "15px");
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
        "어느 키의 형식인지가 문구에 있어야 한다: {}",
        err.message()
    );

    assert_eq!(world.settings.get(None).expect("읽기").rev, 0);
    assert!(world.settings_events.changes().is_empty());
}

/// 셸 `setup` 이 쓰기를 열기 전에 온 쓰기 — 서비스의 `Internal` 이 `INTERNAL` 로 나가고 디스크는 그대로다.
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
    assert!(!closed.0.exists(), "쓰기 전에 폴더를 만들었다");
}

/// ★쓰기 본문은 적용 태스크를 폴링하는 런타임 스레드가 아니라 블로킹 풀에서 돈다★ — `sync_all` 을 기다리는
/// 동안 그 워커에 얹힌 다른 태스크(연결 소켓 포함)가 서지 않게.
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
            .expect("성공 답장");
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
        .expect("성공 답장");
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
    .expect("성공 답장");

    assert_eq!(ok["rev"], 3);
    let reset = ok["reset"].as_array().expect("reset");
    assert_eq!(reset.len(), 11, "선택자가 덮은 키 전부");
    assert!(
        ok.get("changed").is_none(),
        "버스 답은 {{rev, reset}} 이다: {ok}"
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
        "알림은 바뀐 키만 싣는다"
    );
    assert_eq!(world.settings_events.theme_pushes(), 0);

    // 이미 다 기본값 — 쓰지도 알리지도 않는다(rev 그대로).
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
    .expect("성공 답장");
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
    .expect("성공 답장");

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
    .expect("성공 답장");

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
    .expect("성공 답장");
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
    .expect("성공 답장");
    assert_eq!(size["items"][0]["kind"], "css-length");
    assert_eq!(size["items"][0]["min"], "8px, 0.5rem, 0.5em");
    assert!(size["items"][0]["choices"].is_null());

    world.mail.clear();
    let all = call(&receiver, &queue, &world.mail, "settings.schema", json!({}))
        .await
        .outcome
        .expect("성공 답장");
    assert_eq!(all["items"].as_array().expect("items").len(), 12);
}

/// ★버스 쓰기 → 알림 → 유효 테마 밀기를 한 줄로★ — 운영 어댑터(`TauriSettingsEvents`)와 같은 배선을 창 없이
/// 세운다: 알림 포트가 표가 쥔 것과 같은 [`ThemeControl`] 로 창마다 민다.
#[tokio::test]
async fn a_bus_theme_default_write_reaches_every_window_without_its_own_theme() {
    struct Pushing(ThemeControl);

    impl SettingsEvents for Pushing {
        fn changed(&self, _change: &SettingsSnapshot) {}

        fn theme_default_changed(&self) {
            self.0.push();
        }
    }

    let (world, mut ports) = World::build();
    world
        .themes
        .set(MAIN_WINDOW_LABEL, Some(UiTheme::EInk))
        .expect("창 테마");
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
    .expect("성공 답장");

    assert_eq!(
        world.theme_windows.take(),
        sent(&[
            (MAIN_WINDOW_LABEL, "e-ink"),
            (UNMODELED_POPOUT_LABEL, "light")
        ])
    );
}

// ── (B) 크래시 사본 — restore.status · restore.answer (TRD S21-storage §6-7) ──────────
//
// 조율자는 실물이고(포트만 가짜) 표가 쥔 **그 인스턴스**를 시험이 함께 쥔다. 재는 것 = 봉투가 그 조율자에 닿나 ·
// 답 모양 · 조율자의 오류 종류가 `CONFLICT` · `INTERNAL` 로 나가나 · 막는 수락 본문이 런타임 스레드 밖에서 도나.
// 수락의 화면 교체 순서 · 되돌림은 조율자 옆 단위 시험(`--test lib_unit`)이 잰다 — 여기서 다시 재지 않는다.

/// 복원 수락의 창 포트 대역 — 모든 창이 보이고 만들기는 늘 성공한다. 숨긴 채 만든 창과 본문이 돈 스레드만 남긴다.
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

/// 데몬 클라이언트가 없는 셸 — 구독 재계산을 건너뛴다.
struct NoSubscriptions;

impl SubscriptionSource for NoSubscriptions {
    fn current(&self) -> Option<Arc<dyn SubscriptionSync>> {
        None
    }
}

impl World {
    /// 셸 setup 끝처럼 조율자에 포트를 꽂는다.
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

    /// 부팅 단계 ⑥ — 앞 실행의 화면(main 탭 「지난 탭」 + 빈 팝아웃 `popouts` 개)을 답하지 않은 사본으로 세운다.
    fn crash_copy_awaits(&self, popouts: usize) {
        self.crash_copy_awaits_with(popouts, false, StateFileStatus::Ok);
    }

    fn crash_copy_awaits_with(&self, popouts: usize, saves: bool, state_file: StateFileStatus) {
        let mut previous = ViewManager::new();
        let main_view = previous.windows[MAIN_WINDOW_LABEL].active;
        previous
            .rename_tab(main_view, "지난 탭".to_string())
            .expect("탭 이름");
        for n in 0..popouts {
            previous
                .create_window(&format!("slot-popup-{}", 90 + n))
                .expect("팝아웃");
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
        mgr.views[&mgr.windows[MAIN_WINDOW_LABEL].active]
            .name
            .clone()
    }
}

#[tokio::test]
async fn restore_status_reports_the_crash_copy_shape() {
    let (world, queue, receiver) = queued();

    let none = call(&receiver, &queue, &world.mail, "restore.status", json!({}))
        .await
        .outcome
        .expect("성공 답장");
    assert_eq!(
        none,
        json!({"crash_copy": "none", "saved_at_ms": null, "windows": null, "tabs": null, "durable": null, "saves": true, "state_file": "ok"}),
        "물을 사본이 없으면 사본의 셋은 null"
    );

    world.crash_copy_awaits(2);
    world.mail.clear();
    let awaiting = call(&receiver, &queue, &world.mail, "restore.status", json!({}))
        .await
        .outcome
        .expect("성공 답장");
    assert_eq!(
        awaiting,
        json!({"crash_copy": "awaiting", "saved_at_ms": 1_700_000_000_000_u64, "windows": 3, "tabs": 3, "durable": false, "saves": false, "state_file": "ok"}),
        "창 수 = main + 팝아웃"
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
        .expect("성공 답장");
    assert_eq!(
        unreadable,
        json!({"crash_copy": "none", "saved_at_ms": null, "windows": null, "tabs": null, "durable": null, "saves": false, "state_file": "unreadable"}),
        "사본이 없어도 state_file 은 값이다"
    );

    for (state_file, wire) in [
        (StateFileStatus::CorruptCopiedAside, "corrupt_copied_aside"),
        (StateFileStatus::CorruptNotCopied, "corrupt_not_copied"),
    ] {
        world.crash_copy_awaits_with(0, true, state_file);
        world.mail.clear();
        let reply = call(&receiver, &queue, &world.mail, "restore.status", json!({}))
            .await
            .outcome
            .expect("성공 답장");
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
            .expect("성공 답장");
        assert_eq!(none["saves"], json!(saves), "사본이 없어도 값 — {saves}");
        assert_eq!(none["durable"], json!(null));

        world.crash_copy_awaits_with(0, saves, StateFileStatus::Ok);
        world.mail.clear();
        let awaiting = call(&receiver, &queue, &world.mail, "restore.status", json!({}))
            .await
            .outcome
            .expect("성공 답장");
        assert_eq!(awaiting["crash_copy"], json!("awaiting"));
        assert_eq!(awaiting["saves"], json!(saves));
        assert_eq!(
            awaiting["durable"], awaiting["saves"],
            "묻는 동안 둘은 같다 — {saves}"
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
        .expect("거절 답장");
        world.mail.clear();
        let answered = call(&receiver, &queue, &world.mail, "restore.status", json!({}))
            .await
            .outcome
            .expect("성공 답장");
        assert_eq!(answered["crash_copy"], json!("answered"));
        assert_eq!(answered["durable"], json!(null), "awaiting 밖은 null");
        assert_eq!(
            answered["saves"],
            json!(saves),
            "답한 뒤에도 그 실행의 값 — {saves}"
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
    assert_eq!(no_copy.code(), ErrorCode::Conflict, "사본이 없다");

    world.crash_copy_awaits(0);
    let held = world.restore_service.begin_answer().expect("답할 수 있다");
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
        "다른 답이 처리 중이다"
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
    .expect("거절은 답이다");
    assert_eq!(
        rejected,
        json!({"restored_windows": 0, "durable": false}),
        "거절은 화면을 안 바꾸고 · 기록기가 없으면 디스크에 안 붙는다"
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
    assert_eq!(again.code(), ErrorCode::Conflict, "이미 답했다");
    assert_ne!(
        again.message(),
        no_copy.message(),
        "사본 없음과 이미 답함을 문구가 가른다(사람이 읽는다)"
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
        "지시는 싣지 않는다 — 데몬이 중계하며 전부 never 로 내린다(ADR-0159)"
    );
    assert!(
        early.message().contains("answer again"),
        "다시 답하라는 안내는 문구가 나른다: {}",
        early.message()
    );
    assert_eq!(
        world.restore.status().crash_copy,
        CrashCopyStatus::Awaiting,
        "아무것도 안 바뀌었다 — 다시 답할 수 있다"
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
    .expect("포트가 선 뒤 다시 답하면 된다");
    assert_eq!(retried, json!({"restored_windows": 1, "durable": false}));
}

/// ★표가 쥔 조율자가 사람 경로와 **같은 인스턴스**다★ — 수락은 모델을 사본 화면으로 바꾸고, 막는 본문은 적용
/// 태스크를 폴링하는 런타임 스레드가 아니라 블로킹 풀에서 돈다.
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
    .expect("수락");

    assert_eq!(accepted, json!({"restored_windows": 2, "durable": false}));
    assert_eq!(world.main_tab_name(), "지난 탭");
    let opened = screen.opened.lock().unwrap().clone();
    assert_eq!(opened, ["slot-popup-1"], "새 팝아웃은 공유 발급기의 label");
    let mut windows = world.state.0.lock().unwrap().list_windows();
    windows.sort();
    assert_eq!(windows, [MAIN_WINDOW_LABEL, "slot-popup-1"]);
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
        "인자 검문에서 멈춘다"
    );
}

// ── (B) 분할 비율 — split.setRatio · split.list (ADR-0227) ────────────────────

impl World {
    /// 명령을 한 번 더 부른다 — 같은 우편함을 비우고 쓴다([`Mailbox::only`] 가 「정확히 하나」를 요구한다).
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

    /// 주 탭을 `x | (y / z)` 로 나눈 세계 — 반환 = (탭, x, y, z). 분할은 전위 순으로 바깥(좌우) · 안쪽(위아래).
    fn three_slots(&self) -> (uuid::Uuid, uuid::Uuid, uuid::Uuid, uuid::Uuid) {
        let view = self.main_tabs().active;
        let x = self.empty_slot(view);
        let y = apply::split_slot(&self.state, &Subs, &Events, view, x, SplitDir::LeftRight)
            .expect("좌우 분할");
        let z = apply::split_slot(&self.state, &Subs, &Events, view, y, SplitDir::TopBottom)
            .expect("위아래 분할");
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

const A_SIDE: &str = "a 쪽(왼쪽/위) 칸의 몫";

#[test]
fn both_ratio_commands_define_the_ratio_as_the_a_side_share() {
    for name in ["split.setRatio", "split.list"] {
        let spec = spec_of(name).expect("선언돼 있다");
        assert!(
            spec.summary.contains(A_SIDE),
            "{name}: 요약에 ratio 의 뜻({A_SIDE})이 없다: {}",
            spec.summary
        );
    }
    let list = spec_of("split.list").expect("선언돼 있다");
    assert!(list.summary.contains("a = 왼쪽/위"), "{}", list.summary);
    // 요약은 손으로 쓴 글이라 상수가 바뀌면 조용히 LLM 에게 거짓을 말한다 — 상수에서 만든 문자열로 잰다.
    let bounds = format!("{}~{}", tree::RATIO_MIN, tree::RATIO_MAX);
    let set = spec_of("split.setRatio").expect("선언돼 있다");
    assert!(
        set.summary.contains(&bounds),
        "split.setRatio 요약이 클램프 범위 {bounds} 를 말해야 한다: {}",
        set.summary
    );
}

#[test]
fn set_ratio_advertises_its_three_outcomes_by_name() {
    let spec = spec_of("split.setRatio").expect("선언돼 있다");
    let shape: serde_json::Value = serde_json::from_str(spec.ok_schema).expect("스키마는 JSON");
    assert_eq!(
        shape["properties"]["outcome"]["enum"],
        json!(["Applied", "Unchanged", "TooSmall"]),
        "{shape}"
    );
    assert_eq!(shape["properties"]["ratio"]["type"], "number");
}

/// ★같은 결말을 두 표면이 같은 철자로 말한다★(`both_surfaces_spell_the_outcome_the_same_way` 와 같은 규칙).
///
/// `split.setRatio` 의 답은 선언 매크로의 `RatioOutcome`, Tauri `set_split_ratio` 의 답은 셸 내부
/// `SplitRatioApplied.outcome`(ts-rs 로 화면에 간다)으로 직렬화된다. 둘 다 variant 이름을 serde 가 그대로
/// 내지만, 셸 쪽에 serde rename 을 달면(이 crate 의 다른 레이아웃 enum 은 snake_case 다) 두 표면이 갈린다.
/// 핸들러의 exhaustive `match` 는 **빠진 갈래**만 잡지 철자는 못 잡는다.
#[test]
fn both_surfaces_spell_the_split_ratio_outcome_the_same_way() {
    let spec = spec_of("split.setRatio").expect("선언돼 있다");
    let shape: serde_json::Value = serde_json::from_str(spec.ok_schema).expect("스키마는 JSON");
    let advertised = shape["properties"]["outcome"]["enum"]
        .as_array()
        .expect("enum 목록")
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
        let json = serde_json::to_value(payload).expect("직렬화");
        assert_eq!(
            json["outcome"], expected,
            "Tauri 답 쪽 철자가 갈렸다: {json}"
        );
        assert!(
            advertised.contains(&json["outcome"]),
            "명령 답이 광고하는 값에 {expected} 가 없다: {advertised:?}"
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
        .expect("적용");
    assert_eq!(ok, json!({ "ratio": 0.3, "outcome": "Applied" }));
    let layout = apply::get_view(&world.state, view).expect("view");
    let left = layout
        .slot_rects
        .iter()
        .find(|r| r.slot_id == x)
        .expect("x");
    assert_eq!(left.x1, 0.3, "0.3 이 그대로 a(왼쪽) 칸의 몫이 된다");

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
            .expect("범위 밖은 오류가 아니다");
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
        .expect("이미 하한");
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
        // JSON 은 NaN·±∞ 를 못 싣는다 — `json!` 이 비유한 수를 null 로 적는다.
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

    // 반려 문구가 id 를 얻는 곳으로 안내한다 — 분할 id 를 주는 명령은 split.list 하나뿐이다.
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
    assert_eq!(world.main_tabs().version, version, "반려는 무변경");
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
        assert!(err.message().contains("split 없음"), "{}", err.message());
    }
}

/// 사람의 드래그와 같은 px 최소가 LLM 값에도 걸린다 — 두 표면이 같은 적용 서비스에 떨어진다.
#[tokio::test]
async fn set_ratio_is_clamped_by_the_px_minimum_once_the_window_reported_its_size() {
    let (world, queue, receiver) = queued();
    let (view, _x, _y, _z) = world.three_slots();
    let inner = world.split_ids(view)[1];
    // 안쪽(위아래) 분할의 상자 = 오른쪽 절반 × 캔버스 높이 400px → m = 100 이면 허용 [0.25, 0.75].
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
        .expect("클램프해 적용");
    assert_eq!(ok, json!({ "ratio": 0.25, "outcome": "Applied" }));

    // 창이 두 쪽 최소를 못 줄 만큼 작으면 손대지 않고 TooSmall 을 싣는다(지금 값 그대로).
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
        .expect("TooSmall 은 오류가 아니다");
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
        .expect("목록");
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

    // 칸 하나뿐인 탭은 빈 목록 · 형식이 깨진 id 는 반려 · 없는 탭은 적용 서비스의 사유.
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
        .expect("목록");
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
    assert!(err.message().contains("view 없음"), "{}", err.message());
}

/// 표현할 수 없을 만큼 깊은 분할은 `CONFLICT` + 사유로 나간다(패닉·무응답 없이).
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
        err.message().contains("더 나눌 수 없음"),
        "{}",
        err.message()
    );
    assert_eq!(world.slots(view), slots_before, "트리 불변");
}

// ── (B) 연결 태스크를 안 막는다 ──────────────────────────────────────────────

/// ★`on_command` 은 큐 push 하나만 하고 돌아온다★ — 반환 시점에 적용이 일어났으면 그만큼 연결 읽기 루프가
/// 멈춰 있던 것이다.
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

    assert_eq!(queue.pending(), 1, "적용은 태스크로 나갔다");
    assert_eq!(
        world.main_tabs().version,
        before,
        "on_command 안에서는 아무것도 적용되지 않는다"
    );
    assert_eq!(world.mail.len(), 0, "답장도 아직 없다");

    run(&queue).await;
    assert!(world.main_tabs().version > before);
    assert!(world.mail.only().outcome.is_ok());
}

/// ★self-deadlock 회귀★(ADR-0081 「relay 적용은 액터 밖(비블로킹)」 · ADR-0155 결정 4).
///
/// 합성 명령의 스폰은 **호출자 태스크가 서비스해야** 끝난다 — 여기서는 그 호출자가 스폰 요청 채널을
/// 읽어 답을 넣는다(실제로는 데몬 답장을 읽는 연결 루프). `on_command` 이 인라인으로 기다렸다면 호출자는
/// 그 `recv().await` 에 닿지 못하고 양쪽이 서로를 기다린다 — 아래 timeout 이 그것을 실패로 바꾼다
/// (안 그러면 회귀가 hang 으로 나타나 원인이 안 보인다).
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
    .expect("호출자 태스크가 스폰 요청을 서비스하지 못했다 — 적용이 인라인으로 돈다(self-deadlock)")
    .expect("스폰 요청이 온다");
    assert_eq!(cwd, "C:/work/engram");
    answer
        .send(Ok("agent-1".to_string()))
        .expect("스폰 답을 넣는다");

    // 답장은 적용이 끝난 뒤 다른 태스크에서 온다 — 도착할 때까지 이 태스크가 양보한다.
    world.mail.settle(1).await;
    let ok = world.mail.only().outcome.expect("배치 성공");
    assert_eq!(ok["agent_id"], "agent-1");
}

/// ★이 문은 `agent.new` 의 등록 공통부를 안 지난다★ — 그래서 같은 철자 규칙을 여기서 따로 잰다.
/// 백엔드 낱말은 wire enum 의 역직렬화가(`AgentBackendKind`), cwd 는 base `normalize_spelling` 이 흡수한다.
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
                .unwrap_or_else(|_| panic!("{word}: 스폰까지 못 갔다"))
                .expect("스폰 요청이 온다");
        assert_eq!(cwd, "C:/work/engram", "{word}: cwd 철자가 안 옮겨졌다");
        assert_eq!(backend, Some(want), "{word}: 다른 백엔드로 읽혔다");
        answer
            .send(Ok("agent-1".to_string()))
            .expect("스폰 답을 넣는다");

        world.mail.settle(1).await;
        world.mail.only().outcome.expect("배치 성공");
    }
}

/// 관용은 **철자**에만 든다 — 오탈자는 스폰 전에 반려된다(그 그물의 근거 = `apply::parse_backend`).
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
        "반려된 낱말로 스폰이 나갔다"
    );
}

// ── (B) 터져서 죽지 않는다 ───────────────────────────────────────────────────

/// ★명령 핸들러는 터져서 죽지 않는다 — 오류를 값으로 돌려준다★(TRD §4-⑨). 이 그물은 개발·테스트
/// 빌드에서만 실효가 있고(릴리즈는 `panic = "abort"`), 그래서 규약이 그물보다 앞이다.
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
        .expect("선언된 이름");
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

// ── (B) 연결 루프가 실제로 부를 진입점 ──────────────────────────────────────

/// `accept` 은 **봉투에서** 상관 키를 꺼내 답장 자리를 만든다 — 부르는 쪽이 남의 키를 실을 방법이 없다.
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
    let ok = reply.outcome.expect("조회 성공");
    assert_eq!(ok["window"], MAIN_WINDOW_LABEL);
}

/// 아직 안 끝난 명령 — 답을 낼 준비가 될 때까지 매달려 있는 핸들러.
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

/// ★읽기 루프는 느린 핸들러 뒤에 줄 서지 않는다★ — 앞 명령이 아직 매달려 있는 동안에도 다음 프레임을
/// 계속 받아 넘긴다. 인라인 실행이면 첫 `accept` 에서 멈춰 둘째 프레임이 들어오지 못한다(연결 하나가
/// 통째로 head-of-line 블록 — 출력·상태 이벤트까지 함께 선다).
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
        .expect("선언된 이름");
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
        "다섯 프레임을 다 받는 동안 아무것도 끝나지 않았다 — 루프가 적용을 기다리지 않았다는 뜻"
    );

    gate.add_permits(5);
    mail.settle(5).await;
    assert_eq!(
        mail.request_ids(),
        sent.iter()
            .map(|id| id.to_string())
            .collect::<BTreeSet<_>>(),
        "다섯 왕복의 상관 키가 섞이지 않는다"
    );
}

// ── (B) 연결 arm 이 걷는 바이트 경로 ────────────────────────────────────────
//
// ★실코드를 태운다 — 손으로 다시 지은 경로가 아니다★: 디코드한 봉투를 `accept_inbound` 에 그대로 넘기므로
// 슬롯 조회 · 결말 조립 · 소켓 세대 각인 · 채널 배달 · 표 부재 갈래가 모두 실제 함수로 덮인다. 그 함수를
// 지우면 이 테스트들은 컴파일되지 않는다.
//
// ## ★그래도 안 덮이는 것(잔여 목록 — 이 자리가 정본)★
// - **select arm 의 갈래 선택**(`connection.rs` 의 `else if let AgentEvent::CommandRequest`) — 그 루프는
//   **실 소켓을 요구한다**. ★그것은 불가능이 아니라 비용이다★ — 같은 패키지의 `src/daemon_client/tests.rs`
//   가 루프백 WS 를 세워 `run_connection`→`main_loop` 를 실제로 돌린다(`recording_events_capture_connected_then_broadcasts`).
//   그러니 이 갈래가 무커버인 것은 「태울 수 없어서」가 아니라, 갈래 하나를 재려고 매번 소켓·핸드셰이크를
//   세우는 값이 비싸서다. 여기서 사는 것은 소켓 없이 얻는 결정론과 속도다.
//   ★2026-08-24 전에는 이 자리에 「실 `AppHandle` 도 함께 요구한다」·「`connection.rs` 의 코드 커버는 전부
//   이 파일이 진다」고 적혀 있었다★ — 앞은 emit 이 포트로 끊기면서(`src/daemon_client/events.rs`) 그 파일이
//   `AppHandle` 을 이름으로도 모르게 돼 사라졌고, 뒤는 그 단위 스위트가 같은 루프를 태우기 시작하면서 낡았다.
// - **sink 로의 실제 소켓 쓰기**(`send_fire` 의 직렬화·전송 실패 갈래 포함).
// - **`register_own_commands` 의 전송부**(pending 슬롯 · 결말 로그) — 실리는 내용물은 `registration_command`
//   테스트가 덮는다.
// - **끊김 drain 과 소켓 세대 대조 arm 자체** — 결말에 그 세대가 실린다는 것까지만 아래가 잰다.

/// 인바운드 봉투가 도착한 소켓의 세대 — 0이 아니어야 한다(기본값 0이 우연히 통과하는 것을 막는다).
const SOCKET: u64 = 7;

struct WireTrip {
    world: World,
    /// 결말 프레임(`AgentCommand::CommandOutcome`)의 `reply` 노드.
    reply: serde_json::Value,
    request_id: RequestId,
}

async fn wire_round_trip(name: &str, args: serde_json::Value) -> WireTrip {
    let (world, ports) = World::build();
    let queue = Arc::new(Queued::default());
    let slot = Arc::new(InboundSlot::new());
    slot.set(Arc::new(InboundReceiver::new(
        make_table(ports),
        Arc::clone(&queue) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    )));
    // 연결 태스크의 명령 채널 — 결말은 소켓이 아니라 **이 채널**로 돌아온다(단일 writer 규약).
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<ConnectionCommand>(8);

    let request_id = RequestId::new();
    let sent = serde_json::to_string(&AgentEvent::CommandRequest {
        envelope: envelope(name, args, request_id),
    })
    .expect("데몬이 보낼 Text 프레임");

    // 연결 루프의 디코드 — ★이 프레임은 「내가 기다린 답장」이 아니다★(그러면 pending 이 삼킨다).
    let ev: AgentEvent = serde_json::from_str(&sent).expect("AgentEvent 로 디코드된다");
    assert_eq!(
        event_reply_request_id(&ev),
        None,
        "인바운드 요청이 reply 로 읽히면 그 명령은 실행되지 않고 사라진다"
    );
    let AgentEvent::CommandRequest { envelope } = ev else {
        panic!("CommandRequest arm 으로 갈라져야 한다");
    };

    accept_inbound(&slot, &cmd_tx.downgrade(), SOCKET, envelope);
    run(&queue).await;

    let Some(ConnectionCommand::CommandOutcome { reply, socket }) = cmd_rx.recv().await else {
        panic!("결말이 연결 태스크의 명령 채널로 돌아온다");
    };
    assert_eq!(
        socket, SOCKET,
        "결말에 소켓 세대가 실려야 옛 소켓 몫을 폐기할 수 있다"
    );

    // 연결 태스크가 그 소켓으로 내보내는 프레임.
    let outcome = AgentCommand::CommandOutcome { reply };
    assert_eq!(
        command_request_id(&outcome),
        None,
        "답장을 보내며 pending 슬롯을 만들면 깨울 짝이 없다"
    );
    let text = serde_json::to_string(&outcome).expect("결말 프레임");
    let back: serde_json::Value = serde_json::from_str(&text).expect("결말은 JSON");
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
        .expect("성공 결말에 새 탭 id 가 실린다");
    let tabs = world.main_tabs();
    assert_eq!(tabs.tabs.len(), 2, "적용 서비스가 실제로 탭을 만들었다");
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
        "코드가 wire 를 건너야 호출자가 문구 대신 코드로 분기한다"
    );
    assert_eq!(reply["outcome"]["Err"]["retry"], "never");
}

// ── (B) 등록 패킷 · 늦게 채워지는 슬롯 ──────────────────────────────────────

/// ★실제로 나가는 그 패킷을 잰다★ — `register_own_commands` 가 보내는 것이 `registration_command` 의 반환값
/// 그대로다(그 함수를 지우면 이 테스트가 컴파일되지 않는다). 손으로 다시 지은 패킷을 재면 둘이 갈려도 초록이다.
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
        panic!("얹을 이름이 있으면 등록 패킷이 나온다");
    };
    assert_eq!(
        decls.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
        plugged
    );
    for decl in &decls {
        // `help` 는 데몬이 열어보지 않는 불투명 문자열이지만 **비어 있으면** 발견이 죽는다.
        assert!(
            !decl.help.trim().is_empty(),
            "{}: help 가 비었다",
            decl.name
        );
    }
    assert_eq!(catalog_version, CATALOG_VERSION);
    // 데몬 주인 토큰의 접두를 흉내내면 데몬이 「남의 토큰을 적은 등록」으로 보고 경고를 남긴다.
    assert!(!owner.as_str().is_empty());
    assert!(
        !owner.as_str().starts_with("conn-"),
        "광고 토큰이 데몬 파생 토큰 형식을 흉내내면 안 된다: {owner}"
    );

    // ★얹을 것이 없으면 패킷도 없다★ — 빈 등록은 데몬 명부에 아무 뜻 없는 왕복을 하나 늘린다.
    let empty = InboundReceiver::new(
        CommandTable::new(COMMAND_SPECS),
        Arc::new(Queued::default()) as Arc<dyn TaskSpawner>,
        CATALOG_VERSION,
    );
    assert!(registration_command(&empty).is_none());
}

/// ★광고는 **선언이 아니라 꽂힌 것**이다 — 그 차이를 실제로 가른다★. 같은 선언 집합으로 만든 표에 하나만
/// 꽂으면 광고도 하나여야 하고, 선언에 없는 이름은 표가 애초에 거절한다(그래서 광고 ⊆ 선언이 성립한다).
/// 이 구분이 없으면 못 부를 이름이 명부에 올라 데몬이 배달한 봉투가 `UNKNOWN_COMMAND` 로 되돌아간다.
#[test]
fn advertising_follows_what_is_plugged_not_what_is_declared() {
    assert!(
        COMMAND_SPECS.len() > 1,
        "선언이 하나뿐이면 이 구분을 잴 수 없다"
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
        .expect("선언된 이름");
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
        "꽂힌 하나만 광고한다(선언은 {}개)",
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
        .expect_err("선언 집합 밖 이름은 꽂히지 않는다");
    assert_eq!(refused, TableError::NotDeclared("tab.teleport"));
}

/// 슬롯은 비어 있는 상태가 보여야 한다(그 창에 온 봉투는 호출부가 오류 답장으로 답한다) — 그리고 첫 설치만
/// 이긴다(표를 갈아 끼우면 어느 표가 도는지 알 수 없어진다).
#[test]
fn an_empty_slot_is_visible_and_the_first_install_wins() {
    let slot = InboundSlot::new();
    assert!(slot.get().is_none(), "안 꽂힌 상태가 보인다");

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

    assert!(Arc::ptr_eq(slot.get().expect("꽂혀 있다"), &first));
}

// ── (V) 웹뷰 몫 — 대리 등록과 마지막 홉 ─────────────────────────────────────
//
// ★창 0으로 잰다★: 배달 실물(`AppHandle::emit_to`)만 가짜로 끊고(`RecordingDispatch`) 나머지는 실코드다 —
// 예약 이름 필터 · 등록 패킷 합류 · 3단계 배달의 2단계 · 답장 상관 · 마감. 그 함수들을 지우면 이 절이
// 컴파일되지 않는다.
//
// ## ★그래도 안 덮이는 것★
// - **`TauriViewDispatch::emit_to` 자체**(실 창이 필요하다) 와 웹뷰 쪽 리스너(`src/commands/
//   viewCommandBridge.ts` — vitest 가 투영만 잰다).
// - **`report_view_commands`·`report_command_outcome` invoke 껍데기** — Tauri 의 `State`/`WebviewWindow`
//   주입이 필요하다. 그 안의 판정은 전부 아래가 태우는 `ViewCommandBridge` 메서드에 있다.

/// 봉투를 받아 기록만 하는 배달 — 실물은 `emit_to` 라 창 없이 못 세운다.
///
/// 창 생사도 여기서 흉내낸다: `dead` 에 든 label 은 닫힌 창이다. ★`deliver` 는 그래도 성공한다★ —
/// 운영의 `emit_to` 가 없는 label 에도 `Ok` 를 주기 때문이고, 그 성질이 바로 생사 조회를 따로 둔 이유다.
struct RecordingDispatch {
    seen: mpsc::UnboundedSender<(String, ViewCommandRequest)>,
    dead: Mutex<BTreeSet<String>>,
}

impl ViewDispatch for RecordingDispatch {
    fn deliver(&self, target: &str, request: &ViewCommandRequest) -> Result<(), String> {
        self.seen
            .send((target.to_string(), request.clone()))
            .map_err(|_| "테스트 기록 채널이 닫혔다".to_string())
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

/// 창을 닫아 볼 수 있는 판 — 가짜 배달을 함께 돌려준다.
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
        // ★실 예약 집합을 쓴다★ — 손으로 이름을 적으면 이 테스트가 재는 것이 「내가 적은 목록」이 되고,
        //   어휘가 늘어도 아무 신호가 안 난다.
        reserved_names(),
    ));
    (bridge, rx, dispatch)
}

impl RecordingDispatch {
    fn close(&self, label: &str) {
        self.dead.lock().unwrap().insert(label.to_string());
    }
}

/// 웹뷰가 보고하는 항목 — 인자 없는 최소형.
fn view_decl(name: &str) -> ViewCommandDecl {
    view_decl_with_effect(name, Some(ViewEffect::Write))
}

fn view_decl_with_effect(name: &str, effect: Option<ViewEffect>) -> ViewCommandDecl {
    ViewCommandDecl {
        name: name.to_string(),
        help: ViewCommandHelp {
            summary: format!("{name} 이 하는 일"),
            effect,
            args: BTreeMap::new(),
            required: Vec::new(),
        },
    }
}

/// 웹뷰 몫을 진 수신기 — 적용은 **실 런타임**에서 돈다(답장이 다른 태스크로 들어와야 끝나는 왕복이라
/// 태스크를 쥐고 있는 `Queued` 로는 잴 수 없다).
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

/// ★셸 표 이름은 표에서 온 한 번만 실리고, 데몬이 답하는 이름은 셸이 거르지 않는다★(ADR-0270 결정 2).
///
/// 웹뷰가 셸 표 이름 전량 · 자기 몫(`tab.next`) · 데몬이 답하는 꼴의 이름(`agent.spawn`)을 통째로 보고한
/// 가정 상황이다(오늘 웹뷰는 `help` 를 단 것만 보고한다 — 그래서 이 상황은 누가 그 이름에 `help` 를 단 날이다).
/// - 셸 표 이름은 예약이라 웹뷰 몫에서 빠지고 패킷에는 표에서 온 **한 번**만 실린다 — 명부는 한 패킷 안 같은
///   이름을 오류 없이 접으므로, 두 번 실리면 어느 주인이 이기는지가 말없이 갈린다.
/// - 데몬이 답하는 이름은 실린다 — 그 벽은 데몬의 등록 반려다(셸은 agent crate 를 의존하지 않아 그 어휘를
///   모른다). 반려 쪽 단언 = 데몬 `connection_core.rs` 의
///   `registering_a_name_this_daemon_answers_is_refused_as_a_conflict`.
///
/// ★셸 표 쪽 기대값을 손으로 적지 않는다★ — 셸 선언 전량을 훑으므로 셸 어휘가 늘면 이 그물도 함께 자란다.
#[tokio::test]
async fn the_registration_packet_carries_shell_names_once_and_leaves_daemon_names_to_the_daemon() {
    // 데몬이 답하는 꼴의 이름 — 셸은 그 어휘를 모르므로 문자열로만 댄다.
    const DAEMON_NAME: &str = "agent.spawn";
    let (bridge, _seen) = recording_bridge(Duration::from_secs(1));
    let shell_names: BTreeSet<&str> = COMMAND_SPECS.iter().map(|spec| spec.name).collect();
    assert!(
        !shell_names.contains(DAEMON_NAME),
        "이 시험의 전제 — 셸 표는 {DAEMON_NAME} 를 선언하지 않는다"
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
        "웹뷰 몫에서 빠지는 것은 셸 표 이름 전부이고 그것뿐이다"
    );

    let (_world, receiver) = with_view(Arc::clone(&bridge));
    let Some(AgentCommand::RegisterCommands { decls, .. }) = registration_command(&receiver) else {
        panic!("얹을 이름이 있으면 등록 패킷이 나온다");
    };
    for name in &shell_names {
        let carried: Vec<_> = decls.iter().filter(|d| d.name == *name).collect();
        assert_eq!(carried.len(), 1, "{name}: 셸 표 이름은 한 번만 실린다");
        assert!(
            !carried[0].help.contains(&view_decl(name).help.summary),
            "{name}: 실린 것은 셸 표의 선언이어야 한다 — 웹뷰가 보고한 설명이 실렸다"
        );
    }
    let names: BTreeSet<&str> = decls.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains("tab.next"), "웹뷰 몫은 실린다");
    assert!(
        names.contains(DAEMON_NAME),
        "데몬이 답하는 이름은 셸이 거르지 않는다 — 벽은 데몬의 등록 반려다"
    );
    for decl in &decls {
        assert!(
            !decl.help.trim().is_empty(),
            "{}: help 가 비었다",
            decl.name
        );
    }
}

/// ★★목적지 창이 닫히면 살아 있는 보고자가 이어받는다★★ — 회복 경로가 없으면 그 뒤 모든 배달이 죽은
/// label 로 나가 마감까지 기다렸다가 `TIMEOUT` 이 되고, 이름은 명부에 남아 「있는데 영영 안 되는」 상태가
/// 연결 내내 굳는다.
///
/// 재현하는 경로는 리뷰가 짚은 그것이다: main 의 **단발 보고가 실패**해(그 invoke 에 재시도가 없다) 팝아웃이
/// host 가 된 뒤 그 팝아웃이 닫힌다.
/// ★`deliver` 실패로는 못 잡는다는 것도 함께 박는다★ — 가짜도 운영처럼 죽은 label 에 `Ok` 를 준다.
#[tokio::test]
async fn a_closed_host_hands_the_destination_to_a_live_reporter() {
    let (bridge, mut seen, windows) = recording_bridge_with_windows(Duration::from_secs(5));
    // main 은 보고에 실패했다고 친다 — 아예 안 나타난다.
    bridge.report("popup-1", vec![view_decl("tab.next")]);
    bridge.report("popup-2", vec![view_decl("tab.next")]);
    assert_eq!(
        bridge.host().as_deref(),
        Some("popup-1"),
        "먼저 온 창이 목적지"
    );

    windows.close("popup-1");

    assert_eq!(
        bridge.host().as_deref(),
        Some("popup-2"),
        "죽은 host 를 살아 있는 보고자가 이어받는다"
    );
    let (_world, receiver) = with_view(Arc::clone(&bridge));
    let mail = Mailbox::default();
    let request_id = RequestId::new();
    receiver.accept(
        envelope("tab.next", json!({ "window": "main" }), request_id),
        mail.deliver(),
    );
    let (target, request) = seen.recv().await.expect("살아 있는 창으로 내려간다");
    assert_eq!(target, "popup-2");
    bridge
        .settle("popup-2", &request.request_id, Ok(json!({ "ok": true })))
        .expect("이어받은 창이 답한다");
    mail.settle(1).await;
    assert_eq!(mail.only().outcome, Ok(json!({ "ok": true })));

    // 마지막 창까지 닫히면 광고도 함께 내려간다 — 못 부를 이름을 명부에 남기지 않는다.
    windows.close("popup-2");
    assert_eq!(bridge.host(), None);
    assert!(bridge.declarations().is_empty());
}

/// ★광고하는 명단과 봉투를 받는 창은 **같은 창에서 나온다**★ — 갈리면 데몬이 B 를 광고하는 동안 A 가
/// 실행돼, 광고된 이름이 `UNKNOWN_COMMAND` 로 나가거나 실행되는 이름이 광고에 없다.
///
/// 오늘은 창마다 같은 정적 `contributions` 를 올려 두 집합이 바이트 동일하지만, 그 우연 위에 계약을 얹지
/// 않는다 — 그래서 여기서는 **일부러 다른 목록**을 보고시킨다.
#[tokio::test]
async fn a_non_host_report_does_not_change_what_is_advertised() {
    let (bridge, mut seen, windows) = recording_bridge_with_windows(Duration::from_secs(5));
    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("tab.next")]);
    assert_eq!(bridge.host().as_deref(), Some(MAIN_WINDOW_LABEL));

    let popup = bridge.report("popup-1", vec![view_decl("slot.empty")]);

    assert!(
        !popup.changed(),
        "host 가 아닌 창의 보고는 차분을 만들지 않는다"
    );
    let advertised: Vec<String> = bridge.declarations().into_iter().map(|d| d.name).collect();
    assert_eq!(advertised, vec!["tab.next".to_string()], "광고는 host 것뿐");

    // 그 팝아웃의 이름은 배달도 안 받는다 — 광고에 없으니 명부에도 없다.
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

    // main 이 죽으면 광고도 팝아웃 것으로 **함께** 옮겨간다(둘이 갈리지 않는다).
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

/// ★보고의 **상태 변경과 그 차분 송신이 한 덩이**여야 한다★ — 갈라지면 나중 보고의 차분이 먼저 나가고,
/// 데몬은 두 보고의 합집합을 쥔 채 남는다(다리는 나중 것만 안다 → 옛 이름이 배달되면 웹뷰가 모른다고 답한다).
///
/// 재는 법: 첫 송신을 문 안에서 붙잡아 둔 채 둘째 보고를 넣는다. 문이 없으면 둘째가 먼저 끝나 순서가
/// 뒤집히고, 문이 있으면 둘째는 첫 송신이 풀릴 때까지 **시작조차 못 한다**.
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
                        let _ = gate.acquire().await.expect("게이트");
                        order.lock().unwrap().push("first-send-end");
                    },
                )
                .await;
        })
    };
    // 첫 송신이 문 안에서 멈출 때까지 기다린다.
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

    // 둘째가 문 앞에서 막혀 있다 — 막히지 않으면 여기서 이미 순서가 뒤집힌다.
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        *order.lock().unwrap(),
        vec!["first-send-begin"],
        "둘째 보고가 첫 송신을 앞질렀다 — 데몬이 합집합을 쥔 채 남는다"
    );
    // ★「아직 시작도 안 했다」와 「막혀 있다」를 가른다★ — 앞 단언만이면 둘째 태스크가 굼떠도 통과한다.
    assert!(!second.is_finished(), "둘째 보고가 문을 안 거치고 끝났다");

    gate.add_permits(1);
    first.await.expect("첫 보고");
    second.await.expect("둘째 보고");
    assert_eq!(
        *order.lock().unwrap(),
        vec!["first-send-begin", "first-send-end", "second-send"]
    );
}

/// ★★TypeScript 가 **실제로 찍는 글자**를 Rust 가 읽는지 여기서만 잰다★★
///
/// 다른 어느 테스트도 이 경계를 안 건넌다 — Rust 쪽은 enum 을 직접 만들고 vitest 는 invoke 를 mock 한다.
/// 철자가 갈리면 serde 가 **벡터 전체**를 실패시켜 `report_view_commands` 가 payload 를 통째로 반려하고,
/// 그 창은 명령을 **0개** 등록한다. 유일한 신호는 웹뷰 콘솔 경고 한 줄이다.
/// 아래 첫 문자열은 `src/commands/viewCommandBridge.ts` 의 `offeredCommands()` 가 **오늘 내는 모양
/// 그대로**다 — 짝 단언은 `src/commands/viewCommandBridge.test.ts`(invoke 인자)와
/// `busCommands.test.ts`(명단·effect 어휘)다.
///
/// ★뒤이은 둘째 문자열은 오늘 아무 명령도 안 내는 모양이다★ — `enum` 과 `read` 를 싣던 유일한 발신자가
/// 테마 명령이었고 ADR-0167 이 그것을 내렸다. 그래도 그 두 칸은 **계약**이라 여기서 계속 잰다: 안 재면
/// 다음에 그 모양을 내는 명령이 생길 때 이 경계가 무검증으로 그것을 만난다.
#[test]
fn the_frontend_spelling_of_the_report_payload_deserializes_here() {
    const FROM_WEBVIEW: &str = r#"[
      {"name":"slot.empty","help":{
        "summary":"그 슬롯을 빈 칸으로 되돌린다(슬롯 자체는 남는다 — 없애려면 slot.close).",
        "effect":"write",
        "args":{"viewId":{"type":"string","description":"탭 id(UUID) — tab.list 가 준다."},
                "slotId":{"type":"string","description":"슬롯 id(UUID) — slot.resolveSpatial 이 준다."}},
        "required":["viewId","slotId"]}},
      {"name":"tab.next","help":{"summary":"그 창의 활성 탭을 다음 탭으로 옮긴다.","effect":"write"}}
    ]"#;

    let decls: Vec<ViewCommandDecl> =
        serde_json::from_str(FROM_WEBVIEW).expect("프론트가 찍는 모양 그대로 읽힌다");

    assert_eq!(decls.len(), 2);
    assert_eq!(decls[0].help.effect, Some(ViewEffect::Write));
    let slot = decls[0].help.args.get("slotId").expect("인자 칸");
    assert_eq!(slot.ty.as_deref(), Some("string"));
    assert_eq!(
        decls[0].help.required,
        vec!["viewId".to_string(), "slotId".to_string()]
    );
    // `args`·`required` 를 안 실은 항목도 읽힌다(둘 다 `#[serde(default)]`).
    assert!(decls[1].help.args.is_empty());

    // 오늘 발신자가 없는 두 칸 — `read` 철자와 `enum` 목록(위 doc).
    const NOT_EMITTED_TODAY: &str = r#"[{"name":"probe.shape","help":{"summary":"s","effect":"read","args":{"pick":{"type":"string","enum":["a","b","c"]}}}}]"#;
    let shapes: Vec<ViewCommandDecl> =
        serde_json::from_str(NOT_EMITTED_TODAY).expect("계약에 있는 모양은 읽힌다");
    assert_eq!(shapes[0].help.effect, Some(ViewEffect::Read));
    assert_eq!(
        shapes[0].help.args["pick"]
            .allowed
            .as_deref()
            .map(<[String]>::len),
        Some(3)
    );

    // ★대문자 철자는 **안** 읽힌다 — rename 방향을 못 박는다★. 한 항목이 깨지면 벡터 전체가 실패하므로
    //   이 반려의 대가가 「그 창은 0개 등록」이라는 것도 함께 남긴다.
    let wrong = r#"[{"name":"tab.next","help":{"summary":"s","effect":"Write"}}]"#;
    assert!(
        serde_json::from_str::<Vec<ViewCommandDecl>>(wrong).is_err(),
        "철자가 갈리면 payload 가 통째로 반려된다 — 그 창은 명령을 하나도 등록하지 못한다"
    );
}

/// ★★웹뷰 마감은 **데몬 자리 마감 안쪽**이어야 한다 — 뒤집히면 두 가지가 동시에 깨진다★★
///
/// ① **이쪽 마감이 도달 불가가 된다**: 데몬이 먼저 자리를 거둬 호출자에게 `TIMEOUT`/`retry: never` 로
///    답하고, 뒤늦은 셸의 결말은 자리 없는 답장(`NoSeat`)으로 버려진다 — 아래 timeout 테스트가 단언하는
///    `retry: same-request-id` 는 운영에서 **한 번도** 나올 수 없는 성질이 된다.
/// ② **같은 명령이 두 번 돈다**: 데몬 마감 뒤 호출자가 같은 id 로 다시 부르면 아직 도는 첫 왕복 위로
///    두 번째 봉투가 내려간다.
///
/// ★이 파일이 그 관계를 물 수 있는 유일한 자리다★ — 데몬 crate 는 이 패키지의 **dev 의존**이라 운영
/// 코드가 그 상수를 못 본다. 그래서 값은 셸에 박고 부등식은 여기서 잰다. 어느 쪽 상수를 고쳐 순서를
/// 뒤집으면 여기가 빨개진다.
#[test]
fn the_webview_deadline_fits_inside_the_daemon_seat() {
    let seat = engram_dashboard_daemon::command_delivery::CommandDeliveries::DEFAULT_DEADLINE;

    assert!(
        VIEW_REPLY_DEADLINE + VIEW_HOP_MARGIN <= seat,
        "웹뷰 마감({VIEW_REPLY_DEADLINE:?}) + 홉 여유({VIEW_HOP_MARGIN:?}) 가 데몬 자리 마감({seat:?}) 을 넘는다 \
         — 이 순서가 뒤집히면 셸의 TIMEOUT 은 도달 불가가 되고 같은 명령이 두 번 돌 수 있다"
    );
    assert!(
        !VIEW_HOP_MARGIN.is_zero(),
        "여유 항이 0이면 부등식이 산문이 된다 — 데몬↔셸 두 홉이 공짜라는 주장이다"
    );
}

/// ★같은 번호가 도는 중이면 **안 보낸다**★ — 조용히 덮으면 부수효과가 두 번 일어나고, 먼저 온 옛 결말이
/// 새 시도의 답으로 붙는다. 위 부등식이 이 상황을 막지만 그물은 둘이어야 한다.
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
    let (_target, first) = seen.recv().await.expect("첫 봉투가 내려간다");

    // 같은 번호로 다시 — 아직 첫 왕복이 돌고 있다.
    let second = Mailbox::default();
    receiver.accept(
        envelope("tab.next", json!({ "window": "slot-popup-1" }), request_id),
        second.deliver(),
    );
    second.settle(1).await;

    assert_eq!(
        error_of(second.only()).code(),
        ErrorCode::RequestIdConflict,
        "두 번째 시도는 거절된다"
    );
    assert!(
        seen.try_recv().is_err(),
        "웹뷰로 두 번째 봉투가 내려가지 않았다 — 내려갔다면 명령이 두 번 돈다"
    );

    // 첫 대기자는 그대로 살아 있다 — 밀려나지 않았다.
    bridge
        .settle(
            MAIN_WINDOW_LABEL,
            &first.request_id,
            Ok(json!({ "first": true })),
        )
        .expect("첫 자리가 남아 있다");
    mail.settle(1).await;
    assert_eq!(mail.only().outcome, Ok(json!({ "first": true })));
}

/// ★봉투를 받지 않은 창은 그 왕복을 끝낼 수 없다★ — 상관 키 하나로만 열면 남의 창이 위조 결말을 낼 수
/// 있고, 호출자는 그것을 받는 동안 진짜 창의 부수효과는 그대로 일어난다.
/// ★대조에 실패해도 자리는 남는다★ — 빼 버리면 위조 한 번이 진짜 답의 자리를 지운다.
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
    let (_target, request) = seen.recv().await.expect("봉투가 내려간다");

    let forged = bridge
        .settle(
            "popup-9",
            &request.request_id,
            Ok(json!({ "forged": true })),
        )
        .expect_err("남의 창은 못 끝낸다");
    assert!(forged.contains("popup-9"), "누가 답했는지 말한다: {forged}");
    assert_eq!(mail.len(), 0, "위조는 아무 답장도 못 낸다");

    bridge
        .settle(
            MAIN_WINDOW_LABEL,
            &request.request_id,
            Ok(json!({ "real": true })),
        )
        .expect("진짜 창의 자리는 위조에 지워지지 않았다");
    mail.settle(1).await;
    assert_eq!(mail.only().outcome, Ok(json!({ "real": true })));
}

/// ★표식은 웹뷰가 실은 값을 그대로 광고한다★ — 상수로 박으면 첫 조회 명령이 붙는 날 명부가 거짓 표식을
/// 광고하고, 그 값은 데몬의 쓰기 보존 회계에 그대로 먹인다. 안 실은 항목은 아예 등록하지 않는다(기본값을
/// 고르는 것이 곧 그 거짓말이다).
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
        "표식 없는 항목은 등록에서 빠진다"
    );
    let effect_of = |name: &str| -> String {
        let decl = outcome
            .accepted
            .iter()
            .find(|d| d.name == name)
            .unwrap_or_else(|| panic!("{name} 이 등록됐어야 한다"));
        let item: serde_json::Value = serde_json::from_str(&decl.help).expect("help 는 JSON");
        item["effect"].as_str().expect("effect 칸").to_string()
    };
    assert_eq!(effect_of("view.peek"), "Read");
    assert_eq!(effect_of("view.poke"), "Write");
}

/// ★등록 패킷은 **매번 다시 읽는다**★ — 웹뷰 보고는 소켓과 아무 순서 관계가 없어서, 표를 꽂을 때
/// 한 번 만들어 캐시하면 재연결이 옛 목록(대개 빈 목록)을 다시 보낸다. 그러면 창은 떠 있는데 화면
/// 명령이 명부에 없는 상태가 재연결마다 되살아난다.
#[tokio::test]
async fn a_report_that_arrives_after_the_table_still_rides_the_next_registration() {
    let (bridge, _seen) = recording_bridge(Duration::from_secs(1));
    let (_world, receiver) = with_view(Arc::clone(&bridge));

    let before = registration_command(&receiver).expect("셸 몫만으로도 패킷은 나온다");
    let AgentCommand::RegisterCommands { decls, .. } = before else {
        panic!("RegisterCommands");
    };
    assert!(
        !decls.iter().any(|d| d.name == "tab.next"),
        "아직 아무 창도 보고하지 않았다"
    );

    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("tab.next")]);

    let after = registration_command(&receiver).expect("패킷");
    let AgentCommand::RegisterCommands { decls, .. } = after else {
        panic!("RegisterCommands");
    };
    assert!(
        decls.iter().any(|d| d.name == "tab.next"),
        "다음 (재)핸드셰이크의 패킷에는 웹뷰 몫이 합쳐져 있다"
    );
}

/// ★버려진 왕복은 자리를 남기지 않는다★ — 답장 자리를 지우는 경로가 마감 하나뿐이면, 런타임이 접히거나
/// 배달 future 가 취소될 때마다 자리가 쌓여 다리가 영구히 불어난다.
#[tokio::test]
async fn a_cancelled_delivery_gives_its_answer_slot_back() {
    let (bridge, mut seen) = recording_bridge(Duration::from_secs(60));
    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("slot.empty")]);

    // 태스크를 **쥐고만** 있다가 버린다 — 취소를 그대로 흉내낸다(`ReplySink` 의 Drop 이 답장을 낸다).
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
    let mut task = Box::pin(queue.drain().pop().expect("적용 태스크 하나"));

    // 첫 poll 에서 배달이 나가고 답장 자리가 선다 — 그 뒤 future 를 버린다.
    let waker = futures_util::task::noop_waker();
    let mut cx = std::task::Context::from_waker(&waker);
    assert!(std::future::Future::poll(task.as_mut(), &mut cx).is_pending());
    let (_target, request) = seen.try_recv().expect("웹뷰로 내려갔다");
    drop(task);

    // 자리가 남아 있으면 늦게 온 답이 그것을 붙잡아 성공한다 — 비었어야 한다.
    bridge
        .settle(MAIN_WINDOW_LABEL, &request.request_id, Ok(json!(null)))
        .expect_err("버려진 왕복의 자리는 남지 않는다");
    // 답장은 `ReplySink` 의 Drop 이 낸다(태스크가 답 없이 사라져도 부르는 쪽은 매달리지 않는다).
    assert_eq!(mail.len(), 1);
}

/// ★셸 표가 먼저 답하는 이름은 등록에서 빠진다★ — 실어도 배달은 안 갈리지만(`route` 가 표를 먼저 본다)
/// 명부에 **닿을 수 없는 항목**이 하나 늘고, 그것을 발견한 호출자는 웹뷰 계약을 보고 셸 계약대로 답을 받는다.
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

    assert!(mail.only().outcome.is_ok(), "셸 적용 서비스가 답했다");
    assert_eq!(world.main_tabs().tabs.len(), 2, "실제로 탭이 늘었다");
    assert!(seen.try_recv().is_err(), "웹뷰로는 아무것도 안 내려갔다");
}

/// ★셸 표에 없는 이름은 웹뷰로 내려가고 그 답이 같은 상관 키로 돌아온다★ — 3단계 배달의 2단계다.
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

    let (target, request) = seen.recv().await.expect("웹뷰로 내려간다");
    assert_eq!(target, MAIN_WINDOW_LABEL, "보고한 창으로 간다");
    assert_eq!(request.name, "tab.next");
    assert_eq!(request.args, json!({ "window": "main" }));
    assert_eq!(
        request.request_id,
        request_id.to_string(),
        "상관 키는 전 구간 동일하다"
    );
    assert!(mail.len() == 0, "웹뷰가 답하기 전에는 결말이 없다");

    bridge
        .settle(
            MAIN_WINDOW_LABEL,
            &request.request_id,
            Ok(json!({ "applied": true })),
        )
        .expect("기다리는 자리가 있다");
    mail.settle(1).await;

    let reply = mail.only();
    assert_eq!(reply.request_id, request_id);
    assert_eq!(reply.outcome, Ok(json!({ "applied": true })));

    // ★두 번째 답은 붙일 자리가 없다★ — 창 둘이 같은 봉투를 받았다는 신호라 조용히 먹지 않는다.
    bridge
        .settle(MAIN_WINDOW_LABEL, &request.request_id, Ok(json!(null)))
        .expect_err("한 request_id 에 답장은 하나다");
}

/// ★창이 사라져도 왕복은 값으로 끝난다★ — 마감이 없으면 그 봉투는 영영 안 끝나고 호출자가 매달린다
/// (`route` 는 마감을 안 건다 — 그것이 조립부인 이 다리의 몫이다).
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
    seen.recv().await.expect("웹뷰로 내려간다");

    mail.settle(1).await;
    let err = error_of(mail.only());
    assert_eq!(err.code(), ErrorCode::Timeout);
    assert_eq!(
        err.retry(),
        engram_dashboard_command::RetryMode::SameRequestId,
        "적용 여부가 불명이라 새 id 로 다시 부르면 두 번 적용될 수 있다"
    );
}

/// ★아무 창도 보고하지 않았으면 「모르는 이름」이다★ — 배달할 곳이 없는데 주인이 있다고 답하면 호출자는
/// 「그런 명령 없음」과 「보낼 곳 없음」을 구분할 수 없다.
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
        "보낼 곳이 없으니 아무것도 안 나갔다"
    );
}

/// ★같은 목록을 다시 보고하면 차분이 없다★ — 창마다 이 App 이 떠서 전부 보고하므로(main·팝아웃)
/// 보고마다 차분을 내면 뜻 없는 왕복이 창 수만큼 는다.
/// ★목적지는 main 이 이긴다★ — 팝아웃이 목적지를 가져가면 그 창이 닫히는 순간 웹뷰 명령 전체가 죽는다.
#[tokio::test]
async fn repeating_the_same_report_asks_for_no_delta_and_main_keeps_the_destination() {
    let (bridge, _seen) = recording_bridge(Duration::from_secs(1));

    let first = bridge.report("popup-1", vec![view_decl("tab.next")]);
    assert!(first.changed(), "첫 보고는 명단을 채운다");
    assert_eq!(
        bridge.host().as_deref(),
        Some("popup-1"),
        "먼저 온 창을 받아 둔다"
    );

    let again = bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("tab.next")]);
    assert!(!again.changed(), "같은 목록이면 보낼 차분이 없다");
    assert_eq!(
        bridge.host().as_deref(),
        Some(MAIN_WINDOW_LABEL),
        "main 이 덮는다"
    );

    let stolen = bridge.report("popup-2", vec![view_decl("tab.next")]);
    assert!(!stolen.changed());
    assert_eq!(
        bridge.host().as_deref(),
        Some(MAIN_WINDOW_LABEL),
        "main 은 뺏기지 않는다"
    );

    let shrunk = bridge.report(MAIN_WINDOW_LABEL, vec![]);
    assert_eq!(shrunk.removed, vec!["tab.next".to_string()]);
    assert!(shrunk.accepted.is_empty());
}

/// 등록 패킷의 `help` 는 **Rust 선언이 내는 것과 같은 칸**을 쓴다 — 갈리면 명부 하나에 모양이 두 방언으로
/// 섞여, 그것을 읽는 LLM 이 명령마다 다른 독법을 써야 한다.
#[tokio::test]
async fn a_reported_shape_becomes_a_catalog_item_in_the_same_dialect() {
    let (bridge, _seen) = recording_bridge(Duration::from_secs(1));
    let mut args = BTreeMap::new();
    // ★`enum` 을 싣는 화면 명령은 오늘 하나도 없다★ — 마지막 발신자였던 테마 명령을 ADR-0167 이 내렸다.
    //   그래도 투영이 그 칸을 흘려보내는지는 계약이라, 여기서 값을 지어내 잰다.
    args.insert(
        "window".to_string(),
        ViewArgSchema {
            ty: Some("string".to_string()),
            allowed: Some(vec!["main".to_string(), "slot-popup-1".to_string()]),
            description: Some("대상 창 label".to_string()),
        },
    );
    let outcome = bridge.report(
        MAIN_WINDOW_LABEL,
        vec![ViewCommandDecl {
            name: "tab.next".to_string(),
            help: ViewCommandHelp {
                summary: "  그 창의 활성 탭을 다음 탭으로 옮긴다  ".to_string(),
                effect: Some(ViewEffect::Write),
                args,
                // ★선언에 없는 칸은 required 에서 빠진다★ — 남겨 두면 그 스키마는 어떤 인자로도 만족되지
                //   않아 호출자가 영영 못 부른다.
                required: vec!["window".to_string(), "ghost".to_string()],
            },
        }],
    );

    let item: serde_json::Value =
        serde_json::from_str(&outcome.accepted[0].help).expect("help 는 JSON 이다");
    assert_eq!(item["name"], "tab.next");
    assert_eq!(item["summary"], "그 창의 활성 탭을 다음 탭으로 옮긴다");
    assert_eq!(item["args"]["properties"]["window"]["type"], "string");
    assert_eq!(
        item["args"]["properties"]["window"]["enum"][1],
        "slot-popup-1"
    );
    assert_eq!(item["args"]["required"], json!(["window"]));
    // 카탈로그 항목의 칸 이름은 Rust 쪽과 같아야 한다 — 그 목록을 여기서 못 박는다.
    for key in ["name", "effect", "since", "summary", "args", "ok", "errors"] {
        assert!(item.get(key).is_some(), "{key} 칸이 없다: {item}");
    }
    // ★광고하는 오류 = 이 다리가 낼 수 있는 것과 **정확히 같은 집합**이다★ — 부분집합으로 재면 안 되는
    //   쪽이 열린다: 낼 수 없는 코드를 광고하면 호출자가 **한 번도 안 도는 분기**를 짜고, 그 코드가 안
    //   온다는 사실은 어디서도 안 드러난다. 빠지는 쪽은 반대로 호출자가 기본 갈래로 떨어진다.
    //   넷의 출처 —
    //   - `INTERNAL`  : 웹뷰가 던진 실패(종류를 못 가른다) · 배달 실패 · 답장 채널 조기 종료.
    //   - `TIMEOUT`   : 마감 초과(`a_webview_that_never_answers_ends_as_a_timeout_not_a_hang`).
    //   - `REQUEST_ID_CONFLICT` : 같은 번호가 이미 돈다
    //     (`a_duplicate_request_id_is_refused_instead_of_displacing_the_live_waiter`).
    //   - `UNSUPPORTED` : ★보고한 창이 없다 — 다만 **평소엔 여기까지 안 온다**★. 조회가 먼저 「모르는
    //     이름」으로 답해 배달이 `UNKNOWN_COMMAND` 로 끝나기 때문이다
    //     (`a_webview_name_is_unknown_until_a_window_reports_it` 이 그것을 잰다). 이 갈래는 조회와 배달
    //     **사이에** host 가 죽은 경우에만 열리고, 그 인터리브를 만드는 테스트는 없다(무커버 잔여).
    let advertised: BTreeSet<&str> = item["errors"]
        .as_array()
        .expect("errors 는 배열")
        .iter()
        .map(|code| code.as_str().expect("코드는 문자열"))
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
        "광고와 실제가 갈렸다 — 남으면 안 도는 분기가 생기고, 빠지면 답이 광고 밖으로 나간다"
    );
}

/// ★설명 없는 항목은 등록하지 않는다★ — 이름만 오른 명령은 발견해도 인자를 채울 재료가 없어 부를 수 없다
/// (등록에 모양을 동봉한 이유가 그 왕복을 없애는 것이다 — ADR-0156).
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

// ── (B) 결말 출구가 연결 수명을 붙들지 않는다 ────────────────────────────────

/// ★연결 태스크가 쥐는 송신단은 **weak** 이라는 것을 타입으로 못박는다★.
///
/// 아래 EOF 테스트는 `outcome_sink` 만 태우므로, `run_connection`·`main_loop` 의 칸이 강한 `mpsc::Sender` 로
/// 되돌아가도 침묵한다. 이 한 줄은 그 칸들이 쓰는 **별칭의 뜻**이 바뀌는 것을 컴파일 에러로 만든다
/// (`run_connection` 은 `pub(crate)` 라 여기서 부를 수 없어 이것이 붙잡을 수 있는 전부다).
const _: fn(mpsc::WeakSender<ConnectionCommand>) -> OutcomeSender = |sender| sender;

/// ★이 채널의 **강한 송신단 수가 연결 태스크의 수명**이다★ — 결말 출구가 강한 clone 을 쥐면 그 수명 신호가
/// 영원히 오지 않는다.
///
/// 무엇이 깨지나: `DaemonClient::close()` 는 소켓을 닫지도 abort 핸들을 쥐지도 않고 **송신단을 놓을 뿐**이고,
/// `main_loop` 의 select 에는 취소 arm 이 없다 → EOF 가 유일한 즉시 종료 경로다. 같은 EOF 가 stale 연결 억제도
/// 진다(세대가 밀린 연결은 저장을 거부당하고 그 송신단이 drop 된다). 강한 clone 하나가 그 둘을 동시에 없애고,
/// 그러면 닫힌 줄 아는 연결이 계속 프레임을 읽어 같은 `router`/`registry` 로 팬아웃한다.
///
/// ★고치기 전에는 이 테스트가 timeout 으로 죽는다★(`recv()` 가 영원히 안 돌아온다) — 그래서 상한을 걸어
/// hang 대신 이름 붙은 실패로 만든다.
#[tokio::test]
async fn the_outcome_sink_does_not_keep_the_command_channel_alive() {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<ConnectionCommand>(4);
    let request_id = RequestId::new();
    let sink = outcome_sink(cmd_tx.downgrade(), request_id, SOCKET);

    // lifecycle 이 `close()` 로 자기 송신단을 놓은 상태 — 이제 강한 송신단은 0이어야 한다.
    drop(cmd_tx);

    let eof = tokio::time::timeout(Duration::from_secs(5), cmd_rx.recv())
        .await
        .expect(
            "cmd 채널이 EOF 에 닿지 못했다 — 누군가 강한 송신단을 쥐고 있다(close() 와 stale 억제가 함께 죽는다)",
        );
    assert!(eof.is_none(), "EOF 는 곧 연결 태스크의 종료 신호다");

    // 보낼 곳이 사라졌어도 출구는 값으로 끝난다(패닉 없음) — 답장은 유실된다.
    sink(CommandReply::ok(request_id, json!({})));
}

/// 결말에는 **받은 소켓의 세대**가 실린다 — 그 값이 없으면 끊김 직전에 큐에 든 답장이 다음 소켓으로 나간다.
#[tokio::test]
async fn the_outcome_sink_stamps_the_socket_it_belongs_to() {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<ConnectionCommand>(4);
    let request_id = RequestId::new();
    outcome_sink(cmd_tx.downgrade(), request_id, SOCKET)(CommandReply::ok(request_id, json!({})));

    let Some(ConnectionCommand::CommandOutcome { reply, socket }) = cmd_rx.recv().await else {
        panic!("결말은 자기 variant 로 큐에 든다(Fire 로 보내면 소켓 대조를 할 수 없다)");
    };
    assert_eq!(socket, SOCKET);
    assert_eq!(reply.request_id, request_id);
}

/// 표가 안 꽂힌 채 봉투가 오면 **오류 답장이 나간다** — 조용히 버리면 보낸 쪽이 마감시각까지 매달린다.
#[tokio::test]
async fn an_envelope_arriving_before_the_table_still_gets_an_answer() {
    let slot = Arc::new(InboundSlot::new());
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<ConnectionCommand>(4);
    let request_id = RequestId::new();

    accept_inbound(
        &slot,
        &cmd_tx.downgrade(),
        SOCKET,
        envelope(
            "tab.create",
            json!({ "window": MAIN_WINDOW_LABEL }),
            request_id,
        ),
    );

    let Some(ConnectionCommand::CommandOutcome { reply, .. }) = cmd_rx.recv().await else {
        panic!("표가 없어도 답장은 나간다");
    };
    assert_eq!(reply.request_id, request_id);
    assert_eq!(
        reply.outcome.expect_err("표 부재는 실패다").code(),
        ErrorCode::Internal,
        "조립 누락은 재시도로 낫지 않는다 — retry: never"
    );
}

/// 답을 못 내고 태스크가 사라져도 호출자는 마감시각까지 매달리지 않는다 — [`ReplySink`] 의 `Drop` 이 낸다.
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
