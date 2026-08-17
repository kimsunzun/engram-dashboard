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

use std::collections::BTreeSet;
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
    BoxedTask, InboundReceiver, InboundSlot, RuntimeSpawner, TaskSpawner,
};
use engram_dashboard_lib::layout::apply;
use engram_dashboard_lib::layout::commands::{
    make_table, LayoutPorts, WindowListArgs, CATALOG_VERSION, COMMAND_SPECS,
};
use engram_dashboard_lib::layout::{
    AgentSpawner, LayoutEvents, LayoutState, SubscriptionSync, ViewManager, ViewSnapshot,
    WindowHost, WindowTabsPayload, MAIN_WINDOW_LABEL,
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
    mail: Mailbox,
    spawn_requests: mpsc::UnboundedReceiver<SpawnRequest>,
}

impl World {
    fn build() -> (World, LayoutPorts) {
        let state = LayoutState::new();
        let windows = Arc::new(Windows::default());
        let (tx, spawn_requests) = mpsc::unbounded_channel();
        let ports = LayoutPorts {
            state: state.clone(),
            subs: Arc::new(Subs),
            events: Arc::new(Events),
            windows: Arc::clone(&windows) as Arc<dyn WindowHost>,
            // 실물 발급기 — label 단조성은 닫힌 label 재-build 를 막는 계약이라 가짜로 대체하지 않는다.
            labels: Arc::new(PopupCounter::default()),
            spawner: Arc::new(DaemonSpawner { requests: tx }),
        };
        (
            World {
                state,
                windows,
                mail: Mailbox::default(),
                spawn_requests,
            },
            ports,
        )
    }

    fn main_tabs(&self) -> WindowTabsPayload {
        apply::list_tabs(&self.state, MAIN_WINDOW_LABEL).expect("main 창은 항상 있다")
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
            "slot.resolveSpatial",
            "slot.split",
            "tab.close",
            "tab.create",
            "tab.list",
            "tab.rename",
            "tab.switch",
            "window.close",
            "window.create",
            "window.list",
        ]
    );
    assert_eq!(names.len(), COMMAND_SPECS.len(), "선언은 있는데 안 꽂힌 것");
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

/// ★self-deadlock 회귀★(ADR-0081 「relay 적용은 액터 밖(비블로킹)」 · ADR-0140 결정 4).
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
