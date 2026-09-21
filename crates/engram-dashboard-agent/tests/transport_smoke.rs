//! ② 격리 통합테스트 — PtyTransport + OutputCore 신경로를 manager 없이 직접 단언 검증.
//!
//! 실 PTY(default shell)를 spawn 한다. 가볍고 전역 경합 없어 default(자동 실행).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_agent::manager::default_shell;
use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
use engram_dashboard_agent::transport::pty::PtyTransport;
use engram_dashboard_agent::transport::AgentTransport;
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, InputEvent, OutputFrame, OutputPayload,
    OutputSink, SinkError, SinkId, StatusSink,
};

// ── RecordingSink ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct RecordingSink {
    id: SinkId,
    output: Arc<Mutex<Vec<u8>>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            output: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn output_len(&self) -> usize {
        self.output.lock().unwrap().len()
    }

    fn output_contains(&self, needle: &str) -> bool {
        let buf = self.output.lock().unwrap();
        String::from_utf8_lossy(&buf).contains(needle)
    }
}

impl OutputSink for RecordingSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        if let OutputPayload::Bytes(b) = frame.payload {
            self.output.lock().unwrap().extend_from_slice(b);
        }
        Ok(())
    }

    fn sink_id(&self) -> SinkId {
        self.id
    }
}

struct NoopStatusSink;
impl StatusSink for NoopStatusSink {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

fn wait_until<F: Fn() -> bool>(timeout: Duration, cond: F) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    cond()
}

#[test]
fn transport_open_input_resize_shutdown() {
    let started = Instant::now();
    let id = Uuid::new_v4();

    let spec = CommandSpec {
        program: default_shell().to_string(),
        args: vec![],
        env: vec![],
        cwd: PathBuf::from("."),
    };
    let (transport, _child_pid) = PtyTransport::open(&spec, 80, 24).expect("open failed");

    let status_sink: Arc<dyn StatusSink> = Arc::new(NoopStatusSink);
    let core = Arc::new(OutputCore::new(id, 0, status_sink, TurnWiring::detached()));
    assert!(
        matches!(core.status(), AgentStatus::Running),
        "open 직후 status 가 Running 이어야 함"
    );

    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());
    let out_sink = RecordingSink::new();
    let _sid = core.subscribe(Arc::new(out_sink.clone()));

    assert!(
        wait_until(Duration::from_secs(2), || out_sink.output_len() > 0),
        "2s 내 PTY 초기 출력 미수신"
    );

    transport
        .send_input(InputEvent::Raw(b"echo smoke-test\r\n".to_vec()))
        .expect("send_input failed");
    assert!(
        wait_until(Duration::from_secs(3), || out_sink
            .output_contains("smoke-test")),
        "echo 입력이 PTY 출력에 반영되지 않음(smoke-test 미수신)"
    );

    transport.resize(100, 30).expect("resize failed");

    transport.shutdown();
    let join_started = Instant::now();
    core.join_pump(Duration::from_secs(5));
    let join_elapsed = join_started.elapsed();
    assert!(
        join_elapsed < Duration::from_secs(5),
        "join_pump 가 5s 안에 끝나지 않음(hang 의심): {join_elapsed:?}"
    );

    assert!(
        matches!(core.status(), AgentStatus::Killed),
        "shutdown 후 status 가 Killed 가 아님: {:?}",
        core.status()
    );

    // 20s = 개별 단계(5s)보다 느슨한 여유 상한 — 전체 hang 회귀만 잡는다.
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "전체 흐름이 비정상적으로 오래 걸림: {:?}",
        started.elapsed()
    );
}

// ═══════════════════════════════════════════════════════════════════════════════════════════
// 입력 큐(ADR 미부여 — `transport::input_queue`) — PTY 쪽 회귀망
// ═══════════════════════════════════════════════════════════════════════════════════════════

/// ★증명한다★: 큐를 거쳐도 **FIFO 가 보존된다** — 따로따로 넣은 세 줄이 자식에게 그 순서로 도착해
///   그 순서로 echo 된다. 라이터가 큐를 뒤섞거나 덩이를 합치는 회귀는 아래 부분열 단언에 걸린다.
/// ★단언을 "부분열"로 잡은 이유★: PTY 는 프롬프트·ANSI 를 섞어 뱉으므로 정확 일치는 셸 의존이 된다.
///   세 표식이 **이 순서로** 나타나는지만 본다 — 그것이 재려는 성질(순서)의 전부다.
#[test]
fn pty_input_queue_preserves_fifo_order() {
    let spec = CommandSpec {
        program: default_shell().to_string(),
        args: vec![],
        env: vec![],
        cwd: PathBuf::from("."),
    };
    let (transport, _pid) = PtyTransport::open(&spec, 80, 24).expect("open failed");
    let core = Arc::new(OutputCore::new(
        Uuid::new_v4(),
        0,
        Arc::new(NoopStatusSink) as Arc<dyn StatusSink>,
        TurnWiring::detached(),
    ));
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());
    let sink = RecordingSink::new();
    let _sid = core.subscribe(Arc::new(sink.clone()));

    assert!(
        wait_until(Duration::from_secs(5), || sink.output_len() > 0),
        "5s 내 PTY 초기 출력 미수신"
    );

    // 세 번 나눠 넣는다 — 큐가 순서를 지키는지 재는 것이므로 한 덩이로 합쳐 보내면 의미가 없다.
    for marker in ["fifo-alpha", "fifo-bravo", "fifo-charlie"] {
        transport
            .send_input(InputEvent::Raw(format!("echo {marker}\r\n").into_bytes()))
            .expect("send_input 실패");
    }

    assert!(
        wait_until(Duration::from_secs(20), || sink
            .output_contains("fifo-charlie")),
        "20s 내 마지막 표식 미도달 — 큐가 흘러 나가지 않는다"
    );

    let text = {
        let buf = sink.output.lock().unwrap();
        String::from_utf8_lossy(&buf).into_owned()
    };
    let a = text.find("fifo-alpha").expect("alpha 미도달");
    let b = text.find("fifo-bravo").expect("bravo 미도달");
    let c = text.find("fifo-charlie").expect("charlie 미도달");
    assert!(
        a < b && b < c,
        "FIFO 위반 — 도착 위치 alpha={a} bravo={b} charlie={c}"
    );

    transport.shutdown();
    core.join_pump(Duration::from_secs(5));
}

/// ★증명한다★: 상한 초과는 `Err` 로 **거절**되고(버리지 않는다), `shutdown` 뒤의 입력도 거절된다 —
///   즉 라이터가 사라진 뒤에 바이트가 조용히 쌓이는 자리가 없다.
/// ★`shutdown` 뒤 거절이 라이터 종료의 관측 창이다★ — 그 스레드는 아무도 join 하지 않으므로(그것이
///   kill 인과를 지연시키지 않는 근거다) 직접 죽음을 볼 수단이 없다. 대신 그 스레드를 끝내는 바로 그
///   신호(큐 닫힘)를 밖에서 관측한다. 루프 자체가 그 신호로 끝나는 것은
///   `transport::input_queue` 의 `close_terminates_the_drain_loop` 가 잰다.
#[test]
fn pty_input_queue_refuses_overflow_and_writes_after_shutdown() {
    use engram_dashboard_agent::transport::input_queue::INPUT_QUEUE_MAX_BYTES;
    use engram_dashboard_agent::types::PtyError;

    let spec = CommandSpec {
        program: default_shell().to_string(),
        args: vec![],
        env: vec![],
        cwd: PathBuf::from("."),
    };
    let (transport, _pid) = PtyTransport::open(&spec, 80, 24).expect("open failed");
    let core = Arc::new(OutputCore::new(
        Uuid::new_v4(),
        0,
        Arc::new(NoopStatusSink) as Arc<dyn StatusSink>,
        TurnWiring::detached(),
    ));
    let transport: Box<dyn AgentTransport> = Box::new(transport);

    // ★`start()` 전에 잰다★ — 라이터가 없으면 큐가 드레인되지 않아 상한 판정이 결정적이다(라이터가
    //   돌면 얼마나 빠졌는지에 따라 경계가 흔들린다). 큐는 `start()` 전에도 받는다는 사실도 함께 잰다.
    let err = transport
        .send_input(InputEvent::Raw(vec![b'z'; INPUT_QUEUE_MAX_BYTES + 1]))
        .expect_err("상한 초과는 거절돼야 한다");
    assert!(matches!(err, PtyError::WriteFailed(_)), "{err:?}");

    transport.start(core.clone());
    transport.shutdown();

    let late = transport.send_input(InputEvent::Raw(b"late\r\n".to_vec()));
    assert!(
        matches!(late, Err(PtyError::WriteFailed(_))),
        "shutdown 뒤의 입력은 거절돼야 한다(조용히 쌓이면 유실): {late:?}"
    );

    core.join_pump(Duration::from_secs(5));
}
