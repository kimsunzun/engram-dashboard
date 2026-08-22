//! 셸 명령 표(`layout::commands`) + 인바운드 수신기(`daemon_client::inbound`) 통합 테스트 —
//! 데몬·창·소켓 0 (ADR-0012 seam 격리).
//!
//! ★이 파일이 `tests/`(통합 타깃)에 있는 이유★: 이 패키지의 lib 테스트 타깃은 실행 자체가 안 된다
//! (`0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND`, 실측 2026-08-17) — `#[cfg(test)]` 안의 단언은 한 번도 돌지
//! 않는다. 실행: `cargo test -p engram-dashboard --test layout_commands`(자식 프로세스를 하나도 안 띄우므로
//! `-- --test-threads=4` 를 붙이지 않는다 — 판정 규칙 정본 = CLAUDE.md 「빌드·검증 명령」).
//!
//! ★`layout_apply.rs` 와 나누는 기준★: 저쪽은 **적용 서비스의 락 위치**(어느 포트가 락 안/밖에서 불리나)를
//! 재고, 여기는 **봉투가 그 서비스까지 가는 길**(이름 배달 · 인자 검문 · 답장 상관 · 연결 태스크 밖 실행)을
//! 잰다. 그래서 이 파일의 가짜 포트에는 락 프로브가 없다 — 같은 것을 두 번 재면 한쪽이 낡는다.
//!
//! ★`ui_settings` 의 순수 단위 단언도 여기 있다(§F)★ — 그 모듈은 레이아웃이 아니지만, 이 패키지에서
//! **실제로 도는 테스트 타깃**은 `tests/` 의 네 개뿐이고(lib 은 `0xc0000139`) 그중 셸 명령 표를 재는 것이
//! 이 파일이다. 새 타깃을 파면 CI 가 타깃을 이름으로 열거하므로(`.github/workflows/ci.yml`) 아무도 안 도는
//! 초록 파일이 하나 는다.
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
    OwnerToken, ReplySink, RequestId, TableError,
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
    make_table, LayoutPorts, SlotPopoutArgs, UiRefreshArgs, WindowListArgs, CATALOG_VERSION,
    COMMAND_SPECS,
};
use engram_dashboard_lib::layout::{
    tree, AgentSpawner, LayoutEvents, LayoutState, SlotContent, SubscriptionSync, ViewManager,
    ViewSnapshot, WindowHost, WindowTabsPayload, MAIN_WINDOW_LABEL,
};
use engram_dashboard_lib::ui_settings::{
    load_theme, parse_theme, read_capped, LoadedTheme, SettingsSource, ThemeSource,
    UiSettingsPayload, UiSettingsRefresh, UiTheme, DEFAULT_THEME,
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

/// 스폰 요청 한 건 — cwd 와 그 답을 넣을 자리.
type SpawnRequest = (String, oneshot::Sender<Result<String, String>>);

/// ★답을 **다른 태스크가** 넣어 줘야 끝난다★ — 데몬 왕복의 성질을 그대로 흉내낸다. 이게 있어야
/// 「적용이 호출자 태스크를 붙들고 있나」를 잴 수 있다(가짜가 즉답하면 그 질문 자체가 사라진다).
struct DaemonSpawner {
    requests: mpsc::UnboundedSender<SpawnRequest>,
}

impl AgentSpawner for DaemonSpawner {
    fn spawn_by_cwd<'a>(
        &'a self,
        cwd: String,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'a>>
    {
        Box::pin(async move {
            let (answer, wait) = oneshot::channel();
            self.requests
                .send((cwd, answer))
                .map_err(|_| "테스트 스폰 채널이 닫혔다".to_string())?;
            wait.await
                .map_err(|_| "테스트 스폰 응답이 없다".to_string())?
        })
    }
}

/// UI 설정 포트 대역 — 디스크도 창도 없이 「몇 번 다시 읽었나 · 무엇을 돌려줬나」만 남긴다.
///
/// 값을 바꿔 끼울 수 있어야 한다: 「같은 명령을 두 번 불러도 **그때의 파일 값**이 돌아온다」를 재려면
/// 두 호출 사이에 답이 달라져야 한다.
struct FakeUiSettings {
    /// `Err` = 알림을 못 보낸 것으로 친다(값은 정해졌으나 어느 창에도 안 닿았다).
    loaded: Mutex<Result<LoadedTheme, String>>,
    calls: Mutex<usize>,
}

impl Default for FakeUiSettings {
    fn default() -> Self {
        FakeUiSettings {
            loaded: Mutex::new(Ok(LoadedTheme {
                theme: DEFAULT_THEME,
                source: ThemeSource::File,
            })),
            calls: Mutex::new(0),
        }
    }
}

impl UiSettingsRefresh for FakeUiSettings {
    fn refresh(&self) -> Result<LoadedTheme, String> {
        *self.calls.lock().unwrap() += 1;
        self.loaded.lock().unwrap().clone()
    }
}

impl FakeUiSettings {
    /// 파일에 그 값이 적혀 있었던 것으로 친다.
    fn set(&self, theme: UiTheme) {
        *self.loaded.lock().unwrap() = Ok(LoadedTheme {
            theme,
            source: ThemeSource::File,
        });
    }

    /// 파일을 못 써서 기본값으로 접힌 것으로 친다 — ★값은 dark 인데 출처가 다르다★.
    fn fold(&self) {
        *self.loaded.lock().unwrap() = Ok(LoadedTheme {
            theme: DEFAULT_THEME,
            source: ThemeSource::Fallback,
        });
    }

    /// 값은 정해졌는데 **알림을 못 보낸** 것으로 친다.
    fn fail_broadcast(&self) {
        *self.loaded.lock().unwrap() = Err("창이 하나도 안 받았다".to_string());
    }

    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
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

// ── 패닉 훅 잠깐 끄기(RAII) ─────────────────────────────────────────────────

/// 패닉 메시지를 잠깐 삼킨다 — ★반드시 RAII 로 되돌린다★.
///
/// 훅은 **프로세스 전역**이고 이 바이너리의 테스트들은 한 프로세스에서 **병렬**로 돈다. 손으로
/// `set_hook`/`set_hook(previous)` 를 쓰면 그 사이에서 무엇이든 패닉하면(=테스트 실패) 복원 줄에 닿지 못해
/// **그 뒤 모든 패닉 메시지가 조용해진다** — 실패 원인이 순서에 따라 사라지는 최악의 진단 파괴다. Drop 은
/// unwind 중에도 돌므로 그 경로가 없다.
struct QuietPanics(Option<Box<dyn Fn(&std::panic::PanicHookInfo<'_>) + Sync + Send + 'static>>);

impl QuietPanics {
    fn install() -> Self {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        QuietPanics(Some(previous))
    }
}

impl Drop for QuietPanics {
    fn drop(&mut self) {
        if let Some(previous) = self.0.take() {
            std::panic::set_hook(previous);
        }
    }
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
    ui: Arc<FakeUiSettings>,
    mail: Mailbox,
    spawn_requests: mpsc::UnboundedReceiver<SpawnRequest>,
}

impl World {
    fn build() -> (World, LayoutPorts) {
        let state = LayoutState::new();
        let windows = Arc::new(Windows::default());
        let ui = Arc::new(FakeUiSettings::default());
        let (tx, spawn_requests) = mpsc::unbounded_channel();
        let ports = LayoutPorts {
            state: state.clone(),
            subs: Arc::new(Subs),
            events: Arc::new(Events),
            windows: Arc::clone(&windows) as Arc<dyn WindowHost>,
            // 실물 발급기 — label 단조성은 닫힌 label 재-build 를 막는 계약이라 가짜로 대체하지 않는다.
            labels: Arc::new(PopupCounter::default()),
            spawner: Arc::new(DaemonSpawner { requests: tx }),
            ui_settings: Arc::clone(&ui) as Arc<dyn UiSettingsRefresh>,
        };
        (
            World {
                state,
                windows,
                ui,
                mail: Mailbox::default(),
                spawn_requests,
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

    // main 첫 탭은 트리 슬롯 + 빈 작업 슬롯으로 뜬다(ADR-0063) — 그 빈 칸.
    fn empty_slot(&self, view: uuid::Uuid) -> uuid::Uuid {
        tree::first_empty_slot_id(&apply::get_view(&self.state, view).expect("view").layout)
            .expect("빈 슬롯")
    }

    /// pop-out 의 전제 — 빈 슬롯은 옮길 수 없으므로 한 칸을 채운다. 반환 = (탭, 그 슬롯).
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
            "slot.assignAgent",
            "slot.close",
            "slot.focus",
            "slot.popout",
            "slot.resolveSpatial",
            "slot.split",
            "tab.close",
            "tab.create",
            "tab.list",
            "tab.rename",
            "tab.switch",
            "ui.refresh",
            "window.close",
            "window.create",
            "window.list",
        ]
    );
    assert_eq!(names.len(), COMMAND_SPECS.len(), "선언은 있는데 안 꽂힌 것");
}

/// ★세대 번호를 손으로 못 박는다★ — 매크로 계약이 「선언이 바뀌면 손으로 올린다」이고, 안 올리면 어휘가
/// 다른 두 셸이 같은 세대를 보고해 진단이 거짓말을 한다. 위 골든 목록은 **이름만** 보므로 이것을 못 잡는다.
/// 세 줄이 함께 움직여야 한다: 세대 · 선언 수 · 새 명령이 주장하는 `since`(코어 `command_alphabet.rs` 와 같은 형태).
#[test]
fn the_catalog_generation_is_pinned_to_the_declaration_set() {
    // ★이름 수가 안 늘어도 올라간다★ — 세대 4 는 `ui.refresh` 의 **답 모양**이 바뀐 세대다(선언이 바뀌면
    //   올린다). 아래 선언 수가 그대로인 것이 그 구분의 실물이다.
    assert_eq!(CATALOG_VERSION, 4);
    assert_eq!(COMMAND_SPECS.len(), 17);
    assert_eq!(
        SlotPopoutArgs::SPEC.since,
        2,
        "세대 2에 들어온 명령이 1부터 있었다고 광고하면 안 된다"
    );
    assert_eq!(
        UiRefreshArgs::SPEC.since,
        3,
        "세대 3에 들어온 명령이 그 앞부터 있었다고 광고하면 안 된다"
    );
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

/// ★이 바이너리에 링크된 **전 crate** 를 본다★ — 셸이 `agent.*` 계열에 이름을 더하므로(`agent.spawnInto`)
/// 코어 선언과 겹치는 순간 어느 주인이 이기는지 알 수 없어진다.
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

/// 옮길 것이 없는 슬롯은 `CONFLICT` + 서비스의 사유 문구로 반려된다(창도 안 연다).
#[tokio::test]
async fn popout_of_an_empty_slot_is_refused_without_opening_a_window() {
    let (world, queue, receiver) = queued();
    let view = world.main_tabs().active;
    let slot = world.empty_slot(view);

    let err = error_of(
        call(
            &receiver,
            &queue,
            &world.mail,
            "slot.popout",
            json!({ "view_id": view.to_string(), "slot_id": slot.to_string() }),
        )
        .await,
    );

    assert_eq!(err.code(), ErrorCode::Conflict);
    assert!(err.message().contains("빈 슬롯"), "{}", err.message());
    assert!(
        world.windows.opened.lock().unwrap().is_empty(),
        "거절됐으면 창도 안 연다"
    );
    assert!(world.slots(view).contains(&slot), "부분변경 금지");
}

/// `ui.refresh` 는 레이아웃을 안 건드리는 유일한 이름이다 — 그래서 「선언만 있고 자기 포트에 안 닿는다」가
/// 조용히 성립할 수 있다. 답의 값이 **그 포트가 그때 돌려준 것**임을 본다.
#[tokio::test]
async fn ui_refresh_rereads_through_its_own_port() {
    let (world, queue, receiver) = queued();
    world.ui.set(UiTheme::Light);

    let reply = call(&receiver, &queue, &world.mail, "ui.refresh", json!({})).await;

    let ok = reply.outcome.expect("성공 답장");
    assert_eq!(ok["theme"], "light");
    assert_eq!(ok["source"], "File", "파일 값을 그대로 썼으면 File 이다");
    assert_eq!(world.ui.calls(), 1, "명령 한 번 = 다시 읽기 한 번");
}

/// ★답의 존재 이유가 이 한 케이스다★: 접혔을 때 `theme` 은 `dark` 인데, 그것만 보면 「파일에 dark 라고
/// 적혀 있다」와 구별이 안 된다. 호출자는 「내가 적은 값이 반려됐다」를 알아야 파일을 다시 볼 수 있다.
///
/// 접힌 **사유**는 여기 없다(없음·못 읽음·깨짐·모르는 이름·상한 초과 — 전부 앱 로그). 그 다섯을 답에
/// 실으면 호출자가 사유별 분기를 짜기 시작하고 그 순간 다섯 갈래가 계약이 된다.
#[tokio::test]
async fn a_folded_refresh_says_so_even_though_the_theme_is_dark() {
    let (world, queue, receiver) = queued();
    world.ui.fold();

    let reply = call(&receiver, &queue, &world.mail, "ui.refresh", json!({})).await;

    let ok = reply.outcome.expect("성공 답장(접힘은 오류가 아니다)");
    assert_eq!(ok["theme"], "dark", "접히면 기본값이 적용된다");
    assert_eq!(
        ok["source"], "Fallback",
        "값이 dark 인 것만으로는 반려됐는지 알 수 없다 — 그걸 가르는 칸이 source 다"
    );
    assert!(
        ok.get("message").is_none() && ok.get("reason").is_none(),
        "사유는 로그가 진다 — 답에 실으면 그 갈래들이 계약이 된다: {ok}"
    );
}

/// ★알림을 못 보내면 성공이 아니다★ — 이 명령이 하는 일은 그 알림뿐이라, 못 보냈으면 아무 창도 안 바뀌었다.
///
/// `source` 로는 이걸 못 가른다: 그 칸은 **값이 어디서 왔나**를 말하지 **화면이 바뀌었나**를 말하지 않는다.
/// 그래서 enum 에 세 번째 값을 더하는 대신 성공/실패로 가른다.
#[tokio::test]
async fn a_broadcast_that_never_left_is_not_a_success() {
    let (world, queue, receiver) = queued();
    world.ui.fail_broadcast();

    let reply = call(&receiver, &queue, &world.mail, "ui.refresh", json!({})).await;

    let err = error_of(reply);
    assert_eq!(err.code(), ErrorCode::Internal);
    assert_eq!(world.ui.calls(), 1, "실패해도 시도는 한 번이다");
}

/// ★같은 어휘가 두 표면에 산다 — 철자가 갈리면 여기서 걸린다★.
///
/// `ui.refresh` 의 답은 `ThemeOrigin`(선언 매크로), 부팅 조회·푸시 페이로드는 `ThemeSource`(셸 내부)로
/// 직렬화된다. 둘 다 손으로 적은 리터럴은 없지만(각자 variant 이름을 serde 가 낸다) **이름이 갈릴 수는
/// 있다** — 한쪽 variant 를 고치거나 serde rename 을 달면 두 표면이 같은 뜻을 다른 철자로 말한다.
/// `layout::commands` 의 exhaustive `match` 는 **빠진 갈래**만 잡지 철자는 못 잡는다.
///
/// 이 테스트가 `UiSettingsPayload` 를 읽는 유일한 자리이기도 하다.
#[test]
fn both_surfaces_spell_the_outcome_the_same_way() {
    let spec = spec_of("ui.refresh").expect("선언돼 있다");
    let shape: serde_json::Value = serde_json::from_str(spec.ok_schema).expect("스키마는 JSON");
    let advertised = shape["properties"]["source"]["enum"].clone();

    for (source, expected) in [
        (ThemeSource::File, "File"),
        (ThemeSource::Fallback, "Fallback"),
    ] {
        let payload: UiSettingsPayload = LoadedTheme {
            theme: UiTheme::Dark,
            source,
        }
        .into();
        let json = serde_json::to_value(&payload).expect("직렬화");
        assert_eq!(
            json["source"], expected,
            "Tauri 페이로드 쪽 철자가 갈렸다: {json}"
        );
        assert!(
            advertised
                .as_array()
                .expect("enum 목록")
                .contains(&json["source"]),
            "명령 답이 광고하는 값에 {expected} 가 없다: {advertised}"
        );
    }
}

/// 어휘 발견 표면(카탈로그 JSON)에 두 값이 **이름으로** 실린다 — 호출자는 스키마를 읽고 분기한다.
#[test]
fn the_reply_advertises_both_outcomes_by_name() {
    let spec = spec_of("ui.refresh").expect("선언돼 있다");
    let shape: serde_json::Value = serde_json::from_str(spec.ok_schema).expect("스키마는 JSON");
    let source = &shape["properties"]["source"];
    assert_eq!(
        source["enum"],
        json!(["File", "Fallback"]),
        "두 값이 이름으로 안 실리면 호출자가 문구를 읽고 추측한다: {shape}"
    );
}

/// ★파일이 부팅 뒤에 바뀌는 것이 이 명령의 존재 이유다★ — 두 번째 호출이 첫 값을 캐시해서 돌려주면
/// 그 이유가 사라진다(그리고 화면은 안 바뀌는데 답은 성공이라 진단이 막힌다).
#[tokio::test]
async fn a_second_refresh_answers_with_the_new_value() {
    let (world, queue, receiver) = queued();

    world.ui.set(UiTheme::Light);
    let first = call(&receiver, &queue, &world.mail, "ui.refresh", json!({})).await;
    assert_eq!(first.outcome.expect("성공 답장")["theme"], "light");

    world.mail.clear();
    world.ui.set(UiTheme::EInk);
    let second = call(&receiver, &queue, &world.mail, "ui.refresh", json!({})).await;
    assert_eq!(second.outcome.expect("성공 답장")["theme"], "e-ink");
    assert_eq!(world.ui.calls(), 2);
}

/// 레이아웃을 건드리지 않는다 — 슬롯·탭이 그대로여야 프론트가 다시 마운트할 이유가 없다(ADR-0149).
#[tokio::test]
async fn ui_refresh_leaves_the_layout_untouched() {
    let (world, queue, receiver) = queued();
    let before = world.main_tabs();
    let slots_before = world.slots(before.active);

    call(&receiver, &queue, &world.mail, "ui.refresh", json!({})).await;

    let after = world.main_tabs();
    assert_eq!(
        after.version, before.version,
        "레이아웃 version 이 움직였다"
    );
    assert_eq!(after.active, before.active);
    assert_eq!(world.slots(after.active), slots_before);
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

    let (cwd, answer) = tokio::time::timeout(Duration::from_secs(5), world.spawn_requests.recv())
        .await
        .expect(
            "호출자 태스크가 스폰 요청을 서비스하지 못했다 — 적용이 인라인으로 돈다(self-deadlock)",
        )
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

    let reply = {
        let _quiet = QuietPanics::install();
        call(&receiver, &queue, &mail, "window.list", json!({})).await
    };

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
// - **select arm 의 갈래 선택**(`connection.rs` 의 `else if let AgentEvent::CommandRequest`) — 그 루프는 실
//   `WebSocketStream` 과 실 `AppHandle` 을 요구한다. 게다가 `connection.rs` 에는 `#[cfg(test)]` 블록이
//   **하나도 없고** 이 패키지의 lib 테스트는 실행 자체가 안 되므로(`0xc0000139`), 그 파일의 코드 커버는 전부
//   이 파일이 진다.
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
        // 설정이 숨긴 창 — 운영에서는 `hidden_window_labels` 가 `tauri.conf.json` 에서 뽑는다(오늘 이 하나).
        [TREE_WINDOW_LABEL.to_string()],
    ));
    (bridge, rx, dispatch)
}

/// 설정이 `visible: false` 로 선언한 창(`src-tauri/tauri.conf.json`) — ★사전순으로 `slot-popup-N` 보다
/// **앞선다**★. 마지막 수단이 그냥 첫 생존자를 고르면 이 창이 목적지가 된다.
const TREE_WINDOW_LABEL: &str = "agent-tree";

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

/// ★데몬이 답하는 이름이 하나라도 실리면 **패킷 전체**가 반려된다★ — 그러면 셸의 17개 이름이 그 하나
/// 때문에 함께 명부에 못 올라 LLM 이 창·탭·슬롯을 통째로 못 만진다(데몬 `refuse_names_i_answer` — 겹친
/// 이름만 빼 주지 않는다).
///
/// 웹뷰 레지스트리에는 실제로 `agent.spawn`·`agent.rename` 이 있다(`src/commands/agentCommands.ts`) —
/// 그래서 이 필터가 없으면 그 사고가 **오늘 바로** 난다. ★이 테스트가 데몬의 반려 계약을 안 건드리는
/// 근거다★: 셸이 스스로 안 싣는다.
/// ★기대값을 손으로 적지 않는다★ — 코어 선언 전량을 훑으므로 데몬 어휘가 늘면 이 그물도 함께 자란다.
#[tokio::test]
async fn the_registration_packet_never_carries_a_name_the_daemon_answers_itself() {
    let (bridge, _seen) = recording_bridge(Duration::from_secs(1));
    let daemon_answers: Vec<&str> = engram_dashboard_core::agent::commands::COMMAND_SPECS
        .iter()
        .map(|spec| spec.name)
        .collect();
    assert!(
        daemon_answers.contains(&"agent.spawn"),
        "이 테스트의 전제 — 데몬이 agent.spawn 을 답한다"
    );

    // 웹뷰가 자기 id 를 통째로 보고한 상황(오늘 프론트 레지스트리에 실재하는 이름들이다).
    let reported: Vec<ViewCommandDecl> = daemon_answers
        .iter()
        .map(|name| view_decl(name))
        .chain([view_decl("theme.set")])
        .collect();
    let outcome = bridge.report(MAIN_WINDOW_LABEL, reported);
    for name in &daemon_answers {
        assert!(
            outcome.refused.iter().any(|r| r == name),
            "{name} 은 빠졌다고 말해야 한다"
        );
    }

    let (_world, receiver) = with_view(Arc::clone(&bridge));
    let Some(AgentCommand::RegisterCommands { decls, .. }) = registration_command(&receiver) else {
        panic!("얹을 이름이 있으면 등록 패킷이 나온다");
    };
    let names: BTreeSet<&str> = decls.iter().map(|d| d.name.as_str()).collect();
    for name in &daemon_answers {
        assert!(
            !names.contains(name),
            "데몬이 답하는 '{name}' 가 실렸다 — 이 패킷은 통째로 반려된다"
        );
    }
    assert!(names.contains("theme.set"), "웹뷰 몫은 실린다");
    assert!(names.contains("tab.create"), "셸 몫도 그대로 실린다");
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
    bridge.report("popup-1", vec![view_decl("theme.set")]);
    bridge.report("popup-2", vec![view_decl("theme.set")]);
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
        envelope("theme.set", json!({ "theme": "light" }), request_id),
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

/// ★★사람이 못 보는 창은 마지막 수단 목적지가 될 수 없다★★
///
/// 사전순 첫 생존자를 그냥 고르면 `agent-tree` 가 모든 `slot-popup-N` 보다 앞선다 — 그 창은 설정이
/// `visible: false` 라, `theme.set` 이 **성공을 답하면서** 아무도 안 보는 창을 칠한다. 호출자에게는
/// 「적용됐다」인데 화면은 그대로다.
///
/// ★이 규칙이 지키는 것은 「올바른 목적지」가 아니라 **최악의 모양**이다★: main 우선 규칙이 기대는 전제
/// (숨은 main 이 웹뷰 표에 남는다)는 GUI 로 확인하지 못했다(권한 설정이 `hide()`·`close()` 를 막는다 —
/// 2026-08-23). 그 전제가 틀려도 여기서 나오는 답은 「host 없음 = 지금 부를 수 없음」이지 「안 보이는 곳에
/// 조용히 적용됨」이 아니다.
#[tokio::test]
async fn the_last_resort_host_is_never_a_window_the_user_cannot_see() {
    let (bridge, mut seen, windows) = recording_bridge_with_windows(Duration::from_secs(5));

    // ★위 상수가 진짜 설정과 같은지 먼저 본다★ — 운영은 `hidden_window_labels` 로 설정에서 뽑으므로,
    //   설정이 바뀌면 이 하네스가 재는 것이 실물과 갈린다(그때 이 줄이 먼저 걸린다).
    let conf: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string("tauri.conf.json").expect("셸 설정"))
            .expect("설정은 JSON");
    let declared_hidden: Vec<&str> = conf["app"]["windows"]
        .as_array()
        .expect("창 목록")
        .iter()
        .filter(|window| window["visible"] == serde_json::Value::Bool(false))
        .map(|window| window["label"].as_str().expect("label"))
        .collect();
    assert_eq!(
        declared_hidden,
        vec![TREE_WINDOW_LABEL],
        "설정이 숨긴 창 목록이 바뀌었다 — 하네스 상수를 함께 고칠 것"
    );

    // main 은 보고에 실패했다고 친다 — 남은 후보는 숨은 트리 창과 팝아웃뿐이다.
    bridge.report(TREE_WINDOW_LABEL, vec![view_decl("theme.set")]);
    assert!(
        TREE_WINDOW_LABEL < "slot-popup-1",
        "이 테스트의 전제 — 트리 label 이 팝아웃보다 사전순 앞이다"
    );
    assert_eq!(
        bridge.host(),
        None,
        "보이는 후보가 없으면 목적지가 없다 — 숨은 창을 고르지 않는다"
    );
    assert!(
        bridge.declarations().is_empty(),
        "목적지가 없으면 광고도 없다 — 못 부를 이름을 명부에 올리지 않는다"
    );

    // 팝아웃이 뜨면 그쪽이 목적지다(사전순으로는 뒤지만 사람이 볼 수 있다).
    bridge.report("slot-popup-1", vec![view_decl("theme.set")]);
    assert_eq!(bridge.host().as_deref(), Some("slot-popup-1"));

    let (_world, receiver) = with_view(Arc::clone(&bridge));
    let mail = Mailbox::default();
    let request_id = RequestId::new();
    receiver.accept(
        envelope("theme.set", json!({ "theme": "light" }), request_id),
        mail.deliver(),
    );
    let (target, _request) = seen.recv().await.expect("보이는 창으로 내려간다");
    assert_eq!(target, "slot-popup-1", "숨은 창에는 안 보낸다");

    // 그 팝아웃이 닫히면 다시 목적지가 없다 — 숨은 창으로 **떨어지지 않는다**.
    windows.close("slot-popup-1");
    assert_eq!(bridge.host(), None);

    // main 은 이 규칙 밖이다 — `--hidden` 부팅에서 숨어 있어도 사용자가 트레이로 여는 그 창이다.
    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("theme.set")]);
    assert_eq!(bridge.host().as_deref(), Some(MAIN_WINDOW_LABEL));
}

/// ★광고하는 명단과 봉투를 받는 창은 **같은 창에서 나온다**★ — 갈리면 데몬이 B 를 광고하는 동안 A 가
/// 실행돼, 광고된 이름이 `UNKNOWN_COMMAND` 로 나가거나 실행되는 이름이 광고에 없다.
///
/// 오늘은 창마다 같은 정적 `contributions` 를 올려 두 집합이 바이트 동일하지만, 그 우연 위에 계약을 얹지
/// 않는다 — 그래서 여기서는 **일부러 다른 목록**을 보고시킨다.
#[tokio::test]
async fn a_non_host_report_does_not_change_what_is_advertised() {
    let (bridge, mut seen, windows) = recording_bridge_with_windows(Duration::from_secs(5));
    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("theme.set")]);
    assert_eq!(bridge.host().as_deref(), Some(MAIN_WINDOW_LABEL));

    let popup = bridge.report("popup-1", vec![view_decl("theme.toggle")]);

    assert!(
        !popup.changed(),
        "host 가 아닌 창의 보고는 차분을 만들지 않는다"
    );
    let advertised: Vec<String> = bridge.declarations().into_iter().map(|d| d.name).collect();
    assert_eq!(
        advertised,
        vec!["theme.set".to_string()],
        "광고는 host 것뿐"
    );

    // 그 팝아웃의 이름은 배달도 안 받는다 — 광고에 없으니 명부에도 없다.
    let (_world, receiver) = with_view(Arc::clone(&bridge));
    let mail = Mailbox::default();
    let request_id = RequestId::new();
    receiver.accept(
        envelope("theme.toggle", json!({}), request_id),
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
        vec!["theme.toggle".to_string()]
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
                    vec![view_decl("theme.set")],
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
                    vec![view_decl("theme.toggle")],
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
/// 아래 문자열은 `src/commands/viewCommandBridge.ts` 의 `offeredCommands()` 가 내는 모양 그대로다 —
/// 짝 단언은 `src/commands/viewCommandBridge.test.ts`(invoke 인자)와 `busCommands.test.ts`(effect 어휘)다.
#[test]
fn the_frontend_spelling_of_the_report_payload_deserializes_here() {
    const FROM_WEBVIEW: &str = r#"[
      {"name":"theme.set","help":{
        "summary":"창 하나의 테마를 바꾼다",
        "effect":"write",
        "args":{"theme":{"type":"string","enum":["dark","light","e-ink"],"description":"적용할 테마 이름"}},
        "required":["theme"]}},
      {"name":"theme.toggle","help":{"summary":"테마 순환","effect":"read"}}
    ]"#;

    let decls: Vec<ViewCommandDecl> =
        serde_json::from_str(FROM_WEBVIEW).expect("프론트가 찍는 모양 그대로 읽힌다");

    assert_eq!(decls.len(), 2);
    assert_eq!(decls[0].help.effect, Some(ViewEffect::Write));
    assert_eq!(decls[1].help.effect, Some(ViewEffect::Read));
    let theme = decls[0].help.args.get("theme").expect("인자 칸");
    assert_eq!(theme.ty.as_deref(), Some("string"));
    assert_eq!(theme.allowed.as_deref().map(<[String]>::len), Some(3));
    assert_eq!(decls[0].help.required, vec!["theme".to_string()]);
    // `args`·`required` 를 안 실은 항목도 읽힌다(둘 다 `#[serde(default)]`).
    assert!(decls[1].help.args.is_empty());

    // ★대문자 철자는 **안** 읽힌다 — rename 방향을 못 박는다★. 한 항목이 깨지면 벡터 전체가 실패하므로
    //   이 반려의 대가가 「그 창은 0개 등록」이라는 것도 함께 남긴다.
    let wrong = r#"[{"name":"theme.set","help":{"summary":"s","effect":"Write"}}]"#;
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
    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("theme.set")]);

    let (_world, receiver) = with_view(Arc::clone(&bridge));
    let mail = Mailbox::default();
    let request_id = RequestId::new();
    receiver.accept(
        envelope("theme.set", json!({ "theme": "light" }), request_id),
        mail.deliver(),
    );
    let (_target, first) = seen.recv().await.expect("첫 봉투가 내려간다");

    // 같은 번호로 다시 — 아직 첫 왕복이 돌고 있다.
    let second = Mailbox::default();
    receiver.accept(
        envelope("theme.set", json!({ "theme": "dark" }), request_id),
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
    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("theme.set")]);

    let (_world, receiver) = with_view(Arc::clone(&bridge));
    let mail = Mailbox::default();
    let request_id = RequestId::new();
    receiver.accept(
        envelope("theme.set", json!({ "theme": "light" }), request_id),
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
        !decls.iter().any(|d| d.name == "theme.set"),
        "아직 아무 창도 보고하지 않았다"
    );

    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("theme.set")]);

    let after = registration_command(&receiver).expect("패킷");
    let AgentCommand::RegisterCommands { decls, .. } = after else {
        panic!("RegisterCommands");
    };
    assert!(
        decls.iter().any(|d| d.name == "theme.set"),
        "다음 (재)핸드셰이크의 패킷에는 웹뷰 몫이 합쳐져 있다"
    );
}

/// ★버려진 왕복은 자리를 남기지 않는다★ — 답장 자리를 지우는 경로가 마감 하나뿐이면, 런타임이 접히거나
/// 배달 future 가 취소될 때마다 자리가 쌓여 다리가 영구히 불어난다.
#[tokio::test]
async fn a_cancelled_delivery_gives_its_answer_slot_back() {
    let (bridge, mut seen) = recording_bridge(Duration::from_secs(60));
    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("theme.toggle")]);

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
        envelope("theme.toggle", json!({}), request_id),
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
    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("theme.set")]);

    let (_world, receiver) = with_view(Arc::clone(&bridge));
    let mail = Mailbox::default();
    let request_id = RequestId::new();
    receiver.accept(
        envelope("theme.set", json!({ "theme": "light" }), request_id),
        mail.deliver(),
    );

    let (target, request) = seen.recv().await.expect("웹뷰로 내려간다");
    assert_eq!(target, MAIN_WINDOW_LABEL, "보고한 창으로 간다");
    assert_eq!(request.name, "theme.set");
    assert_eq!(request.args, json!({ "theme": "light" }));
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
    bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("theme.toggle")]);

    let (_world, receiver) = with_view(Arc::clone(&bridge));
    let mail = Mailbox::default();
    let request_id = RequestId::new();
    receiver.accept(
        envelope("theme.toggle", json!({}), request_id),
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
    receiver.accept(envelope("theme.set", json!({}), request_id), mail.deliver());
    mail.settle(1).await;

    let err = error_of(mail.only());
    assert_eq!(err.code(), ErrorCode::UnknownCommand);
    assert!(
        seen.try_recv().is_err(),
        "보낼 곳이 없으니 아무것도 안 나갔다"
    );
}

/// ★같은 목록을 다시 보고하면 차분이 없다★ — 창마다 이 App 이 떠서 전부 보고하므로(main·트리·팝아웃)
/// 보고마다 차분을 내면 뜻 없는 왕복이 창 수만큼 는다.
/// ★목적지는 main 이 이긴다★ — 팝아웃이 목적지를 가져가면 그 창이 닫히는 순간 웹뷰 명령 전체가 죽는다.
#[tokio::test]
async fn repeating_the_same_report_asks_for_no_delta_and_main_keeps_the_destination() {
    let (bridge, _seen) = recording_bridge(Duration::from_secs(1));

    let first = bridge.report("popup-1", vec![view_decl("theme.set")]);
    assert!(first.changed(), "첫 보고는 명단을 채운다");
    assert_eq!(
        bridge.host().as_deref(),
        Some("popup-1"),
        "먼저 온 창을 받아 둔다"
    );

    let again = bridge.report(MAIN_WINDOW_LABEL, vec![view_decl("theme.set")]);
    assert!(!again.changed(), "같은 목록이면 보낼 차분이 없다");
    assert_eq!(
        bridge.host().as_deref(),
        Some(MAIN_WINDOW_LABEL),
        "main 이 덮는다"
    );

    let stolen = bridge.report("popup-2", vec![view_decl("theme.set")]);
    assert!(!stolen.changed());
    assert_eq!(
        bridge.host().as_deref(),
        Some(MAIN_WINDOW_LABEL),
        "main 은 뺏기지 않는다"
    );

    let shrunk = bridge.report(MAIN_WINDOW_LABEL, vec![]);
    assert_eq!(shrunk.removed, vec!["theme.set".to_string()]);
    assert!(shrunk.accepted.is_empty());
}

/// 등록 패킷의 `help` 는 **Rust 선언이 내는 것과 같은 칸**을 쓴다 — 갈리면 명부 하나에 모양이 두 방언으로
/// 섞여, 그것을 읽는 LLM 이 명령마다 다른 독법을 써야 한다.
#[tokio::test]
async fn a_reported_shape_becomes_a_catalog_item_in_the_same_dialect() {
    let (bridge, _seen) = recording_bridge(Duration::from_secs(1));
    let mut args = BTreeMap::new();
    args.insert(
        "theme".to_string(),
        ViewArgSchema {
            ty: Some("string".to_string()),
            allowed: Some(vec!["dark".to_string(), "light".to_string()]),
            description: Some("적용할 테마".to_string()),
        },
    );
    let outcome = bridge.report(
        MAIN_WINDOW_LABEL,
        vec![ViewCommandDecl {
            name: "theme.set".to_string(),
            help: ViewCommandHelp {
                summary: "  이 창의 테마를 바꾼다  ".to_string(),
                effect: Some(ViewEffect::Write),
                args,
                // ★선언에 없는 칸은 required 에서 빠진다★ — 남겨 두면 그 스키마는 어떤 인자로도 만족되지
                //   않아 호출자가 영영 못 부른다.
                required: vec!["theme".to_string(), "ghost".to_string()],
            },
        }],
    );

    let item: serde_json::Value =
        serde_json::from_str(&outcome.accepted[0].help).expect("help 는 JSON 이다");
    assert_eq!(item["name"], "theme.set");
    assert_eq!(item["summary"], "이 창의 테마를 바꾼다");
    assert_eq!(item["args"]["properties"]["theme"]["type"], "string");
    assert_eq!(item["args"]["properties"]["theme"]["enum"][1], "light");
    assert_eq!(item["args"]["required"], json!(["theme"]));
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
            name: "theme.set".to_string(),
            help: ViewCommandHelp {
                summary: "   ".to_string(),
                effect: Some(ViewEffect::Write),
                args: BTreeMap::new(),
                required: Vec::new(),
            },
        }],
    );

    assert_eq!(outcome.refused, vec!["theme.set".to_string()]);
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

// ── (F) UI 설정 읽기 — 파일 시스템도 Tauri 도 없이 ──────────────────────────
//
// ★여기 있는 이유는 헤더 마지막 절★(이 패키지에서 실제로 도는 타깃이 `tests/` 뿐이다). 재는 것은 두 층이다:
// 순수 변환(`parse_theme`)과 그 위의 기본값 접기(`load_theme` + 주입 seam).
//
// ## ★안 재는 것 — 로그 레벨(알려진 갭)★
// `NotFound`=debug · 그 밖의 읽기 실패=warn · 파싱 실패=error · 성공=debug 가 **실제로 그 레벨로 나가는지**는
// 여기서 안 잰다(tracing subscriber 하네스가 없어 반환값만 덮인다). 넷 중 하나를 한 낱말 고쳐 뒤바꿔도
// 이 스위트는 초록이다. 근거·의도는 `ui_settings::load_theme` 의 doc 이 진다.

/// 파일 대신 미리 정한 답을 내는 원문 출처.
struct Canned(std::io::Result<String>);

impl Canned {
    fn text(raw: &str) -> Self {
        Canned(Ok(raw.to_string()))
    }

    fn missing() -> Self {
        Canned(Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "없는 파일",
        )))
    }

    fn unreadable() -> Self {
        Canned(Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "권한 없음",
        )))
    }
}

impl SettingsSource for Canned {
    fn read(&self) -> std::io::Result<String> {
        match &self.0 {
            Ok(text) => Ok(text.clone()),
            Err(e) => Err(std::io::Error::new(e.kind(), e.to_string())),
        }
    }

    fn origin(&self) -> String {
        "<canned>".to_string()
    }
}

/// 세 값이 다 살아 있어야 한다 — e-ink 를 dark/light 로 접으면 그 테마의 의도(색 무력화)가 사라진다(ADR-0062).
#[test]
fn every_theme_name_round_trips() {
    for (raw, expected) in [
        ("dark", UiTheme::Dark),
        ("light", UiTheme::Light),
        ("e-ink", UiTheme::EInk),
    ] {
        let text = format!("{{\"theme\":\"{raw}\"}}");
        assert_eq!(parse_theme(&text), Ok(expected), "{raw}");
        assert_eq!(
            load_theme(&Canned::text(&text)),
            LoadedTheme {
                theme: expected,
                source: ThemeSource::File
            },
            "{raw}"
        );
        // 프론트가 `data-theme` 에 박는 철자 = `src/styles/theme.css` 의 셀렉터.
        assert_eq!(expected.as_wire(), raw);
    }
}

/// 못 읽는 네 모양이 **전부 같은 값**으로 접힌다 — 종류를 가르지 않는 것이 계약이다.
#[test]
fn an_unusable_settings_file_falls_back_to_dark() {
    // ★값도 출처도 같아야 한다★ — 접힌 것은 전부 `Fallback` 이다(호출자가 「내 편집이 먹었나」를 이걸로 안다).
    let folded = LoadedTheme {
        theme: DEFAULT_THEME,
        source: ThemeSource::Fallback,
    };
    assert_eq!(load_theme(&Canned::missing()), folded);
    assert_eq!(load_theme(&Canned::unreadable()), folded);
    assert_eq!(load_theme(&Canned::text("{ this is not json")), folded);
    assert_eq!(
        load_theme(&Canned::text(r#"{"theme":"solarized"}"#)),
        folded
    );
    assert_eq!(DEFAULT_THEME, UiTheme::Dark);
}

/// 모양은 JSON 인데 값이 못 쓸 때도 같은 자리로 간다 — 키 부재 · 문자열 아님 · 대소문자 다름.
#[test]
fn a_theme_field_that_is_not_a_known_name_is_refused_not_guessed() {
    for text in [
        "{}",
        r#"{"theme":7}"#,
        r#"{"theme":null}"#,
        r#"{"theme":"Dark"}"#,
        r#"{"theme":"e_ink"}"#,
        r#"{"theme":" dark "}"#,
    ] {
        assert!(parse_theme(text).is_err(), "{text} 를 통과시켰다");
        assert_eq!(
            load_theme(&Canned::text(text)),
            LoadedTheme {
                theme: DEFAULT_THEME,
                source: ThemeSource::Fallback
            },
            "{text}"
        );
    }
}

/// 같은 원문을 두 번 읽으면 두 번 다 같은 답 — 「부팅과 refresh 가 다른 값을 본다」가 여기서 나오면 안 된다.
#[test]
fn reading_the_same_text_twice_gives_the_same_answer() {
    let broken = Canned::text("{oops");
    assert_eq!(load_theme(&broken), load_theme(&broken));

    let good = Canned::text(r#"{"theme":"e-ink"}"#);
    let from_file = LoadedTheme {
        theme: UiTheme::EInk,
        source: ThemeSource::File,
    };
    assert_eq!(load_theme(&good), from_file);
    assert_eq!(load_theme(&good), from_file);
}

/// ★모르는 칸을 무시하는 것은 의도다 — 반려로 바꾸지 말 것★(사용자 결정).
///
/// 이 파일럿은 **여러 칸짜리 설정 파일의 첫 칸**이다. 모르는 키를 반려하면 칸을 하나 더할 때마다 옛 셸이
/// 파일 전체를 거부한다 — 앞날을 위한 호환이지 검증을 빠뜨린 것이 아니다.
#[test]
fn unknown_keys_do_not_break_the_one_key_we_read() {
    assert_eq!(
        parse_theme(r#"{"theme":"light","fontSize":13,"whatever":{"a":1}}"#),
        Ok(UiTheme::Light)
    );
}

/// ★상한을 넘는 원문은 **읽고 나서** 재는 것이 아니라 읽는 양 자체가 끊긴다★ — 밖에서 쓰는 파일이라
/// 크기가 우리 손에 없고, 통째로 읽으면 기본값 접기·경고가 돌기 전에 프로세스가 죽는다.
#[test]
fn an_oversized_settings_file_is_refused_instead_of_swallowed() {
    let cap = 32u64;

    let exact = vec![b'x'; cap as usize];
    assert!(
        read_capped(std::io::Cursor::new(exact), cap).is_ok(),
        "상한 자체는 통과다"
    );

    let over = vec![b'x'; cap as usize + 1];
    let refused = read_capped(std::io::Cursor::new(over), cap).expect_err("상한 초과는 반려다");
    assert_eq!(refused.kind(), std::io::ErrorKind::InvalidData);

    // 그 반려는 못 읽은 것과 같은 자리로 간다(기본값 + 로그).
    assert_eq!(
        load_theme(&Canned(Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "상한 초과"
        )))),
        LoadedTheme {
            theme: DEFAULT_THEME,
            source: ThemeSource::Fallback
        }
    );
}

/// 내보낸 바이트를 세는 리더 — ★`take` 가 **읽기 자체를** 끊는지 재는 유일한 수단★.
///
/// 결과만 보면 「다 읽고 나서 반려」와 「끊어 읽고 반려」가 똑같이 `InvalidData` 라 구분이 안 된다.
/// 그래서 리더가 실제로 몇 바이트를 내보냈는지를 센다.
struct Counting<R> {
    inner: R,
    produced: Arc<Mutex<u64>>,
}

impl<R: std::io::Read> std::io::Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        *self.produced.lock().unwrap() += n as u64;
        Ok(n)
    }
}

/// ★상한이 결과가 아니라 **읽는 양**을 끊는다★ — 이게 [`read_capped`] 가 존재하는 이유 그 자체다.
/// 다 읽고 나서 길이를 재는 구현은 반려 코드가 같아서 통과해 버리므로, 여기서는 리더가 내보낸 바이트를 센다.
#[test]
fn the_cap_stops_the_read_rather_than_the_result() {
    let cap = 16u64;
    let produced = Arc::new(Mutex::new(0u64));
    let reader = Counting {
        inner: std::io::Cursor::new(vec![b'x'; 1024 * 1024]),
        produced: Arc::clone(&produced),
    };

    let refused = read_capped(reader, cap).expect_err("상한 초과는 반려다");
    assert_eq!(refused.kind(), std::io::ErrorKind::InvalidData);

    let read = *produced.lock().unwrap();
    assert!(
        read <= cap + 1,
        "상한을 넘겨 {read} 바이트를 읽었다(허용 {}) — 끊지 않으면 원문 전체가 메모리에 올라와 \
         기본값 접기도 로그도 못 돌고 프로세스가 죽는다",
        cap + 1
    );
}

/// ★못 쓰는 값을 로그 문구에 그대로 옮기지 않는다★ — 이 문구는 곧장 로그로 나가고, 파일을 쓰는 것은 밖의
/// 에이전트라 내용물이 우리 손에 없다. 새면 안 되는 것(자격증명)과 커지면 안 되는 것(상한까지의 덩치,
/// 창을 열 때마다·refresh 때마다 증폭) 둘 다 막는다.
#[test]
fn an_unusable_theme_value_is_not_echoed_into_the_message() {
    // ① 덩치 — 원문이 통째로 실리지 않고 길이만 남는다.
    let blob = "A".repeat(4096);
    let big = parse_theme(&format!(r#"{{"theme":"{blob}"}}"#)).expect_err("반려");
    assert!(!big.contains(&blob), "원문이 그대로 실렸다");
    assert!(big.len() < 200, "문구가 {} 바이트로 불었다", big.len());
    assert!(
        big.contains("4096자"),
        "길이도 안 남으면 진단이 죽는다: {big}"
    );

    // ② 문자열이 아닌 값 — **종류만** 싣는다(객체·배열은 통째로 상한 크기다).
    let obj = parse_theme(r#"{"theme":{"token":"sk-proj-AAAAAAAAAAAAAAAAAAAAAAAAAAAA"}}"#)
        .expect_err("반려");
    assert!(!obj.contains("sk-proj"), "값이 실렸다: {obj}");
    assert!(obj.contains("object"), "종류가 안 실렸다: {obj}");

    // ③ ★모양 게이트★ — 테마 이름처럼 안 생긴 것은 charset 이나 길이에서 걸려 길이만 남는다.
    //    마스킹만으로는 이것들을 못 잡는다(그 헬퍼는 **알려진 키 모양**만 안다 — 이메일·URL·사내 토큰은
    //    그 목록에 없고, 그게 이 게이트를 둔 이유다).
    for (label, value, fragment) in [
        ("이메일", "customer-email@example.com", "example.com"),
        ("URL", "https://example.com/theme", "example.com"),
        ("밑줄 섞인 이름", "internal_build_token_9", "internal"),
        ("공백 섞인 문장", "please use dark", "please"),
    ] {
        let err = parse_theme(&format!(r#"{{"theme":"{value}"}}"#)).expect_err("반려");
        assert!(
            !err.contains(fragment),
            "{label} 가 로그 문구로 샜다: {err}"
        );
    }

    // ④ ★게이트를 통과하는 값에도 마스킹이 남아 있다★ — 이 조합(20자 영숫자 = 길이·charset 둘 다 통과,
    //    그런데 키 모양)이 그 겹이 죽어 있지 않다는 증거다.
    let akia = parse_theme(r#"{"theme":"AKIAIOSFODNN7EXAMPLE"}"#).expect_err("반려");
    assert!(!akia.contains("AKIA"), "키가 그대로 실렸다: {akia}");

    // ⑤ 그래도 오타 진단은 산다 — 게이트를 통과하는 값은 그대로 보인다.
    //    `Dark` 가 가장 흔한 오타다(`from_wire` 가 대소문자를 가린다) — 게이트에서 대문자를 뺐다면
    //    정작 제일 자주 나는 실수를 못 보여준다.
    for typo in ["Dark", "darkk", "e-inkk", "light2"] {
        let err = parse_theme(&format!(r#"{{"theme":"{typo}"}}"#)).expect_err("반려");
        assert!(err.contains(typo), "오타 {typo} 를 못 보여준다: {err}");
    }
}

/// ★마스킹만으로는 못 막는 모양 — 그래서 게이트가 **앞**에 선다★.
///
/// 키 패턴 앞에 다른 것이 붙어 있으면 마스킹은 **그 패턴만** 지우고 앞머리는 그대로 남긴다. 게이트가 없으면
/// 여기 `x` 스무 자가 로그로 나간다(패턴이 잘려 정규식을 비켜 가는 경우도 같은 구멍이다). 게이트는 그 값을
/// 아예 「모양이 아님 + 길이」로 접어서 그 구멍을 닫는다.
#[test]
fn a_value_wrapped_around_a_key_pattern_is_gated_not_just_masked() {
    let prefixed = format!("{}sk-proj-{}", "x".repeat(20), "A".repeat(30));
    let err = parse_theme(&format!(r#"{{"theme":"{prefixed}"}}"#)).expect_err("반려");

    assert!(
        !err.contains(&"x".repeat(20)),
        "패턴 앞머리가 그대로 실렸다: {err}"
    );
    assert!(!err.contains("sk-proj"), "키 조각이 실렸다: {err}");
    assert!(err.contains("58자"), "길이가 안 남았다: {err}");
}

/// UTF-8 이 아닌 원문도 「원문을 못 가져왔다」로 접힌다 — seam 의 계약이 그 하나다.
#[test]
fn a_non_utf8_settings_file_is_a_read_failure() {
    let refused = read_capped(std::io::Cursor::new(vec![0xff, 0xfe, 0x00]), 64).expect_err("반려");
    assert_eq!(refused.kind(), std::io::ErrorKind::InvalidData);
}
