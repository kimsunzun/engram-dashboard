//! ② 격리 통합테스트 — StdioTransport(파이프 자식) + OutputCore 신경로를 manager 없이 직접 단언.
//!
//! json 모드(ADR-0044)가 쓰는 transport. 실 claude 없이(격리 — ADR-0012) cmd.exe/ping 같은
//! 가짜 자식으로 pump·EOF·shutdown·stdin·stderr 격리를 검증한다. 실 프로세스를 spawn 하지만
//! 가볍고(echo/ping) 전역 경합이 없다.
//!
//! ★Windows 전용★: 픽스처가 cmd.exe/ping 기반이라 이 파일은 Windows 에서만 컴파일·실행한다
//!   (프로젝트는 Windows/WebView2 전제). 비Windows 는 빈 파일.
#![cfg(windows)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_core::agent::output_core::OutputCore;
use engram_dashboard_core::agent::transport::stdio::StdioTransport;
use engram_dashboard_core::agent::transport::AgentTransport;
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo, AgentStatus, CommandSpec, InputEvent, OutputFrame, OutputSink, SinkError,
    SinkId, StatusSink,
};

// ── RecordingSink: (seq, bytes) 누적 ────────────────────────────────────────────
#[derive(Clone)]
struct RecordingSink {
    id: SinkId,
    events: Arc<Mutex<Vec<(u64, Vec<u8>)>>>,
}
impl RecordingSink {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }
    fn concat(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (_, b) in self.events.lock().unwrap().iter() {
            out.extend_from_slice(b);
        }
        out
    }
    fn contains(&self, needle: &str) -> bool {
        String::from_utf8_lossy(&self.concat()).contains(needle)
    }
    fn seqs(&self) -> Vec<u64> {
        self.events.lock().unwrap().iter().map(|e| e.0).collect()
    }
}
impl OutputSink for RecordingSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        self.events
            .lock()
            .unwrap()
            .push((frame.seq, frame.data.to_vec()));
        Ok(())
    }
    fn sink_id(&self) -> SinkId {
        self.id
    }
}

// ── RecordingStatusSink: terminal 전이 횟수(finalize 1회 검증) ─────────────────────
#[derive(Clone)]
struct RecordingStatusSink {
    statuses: Arc<Mutex<Vec<AgentStatus>>>,
}
impl RecordingStatusSink {
    fn new() -> Self {
        Self {
            statuses: Arc::new(Mutex::new(Vec::new())),
        }
    }
    fn terminal_count(&self) -> usize {
        self.statuses
            .lock()
            .unwrap()
            .iter()
            .filter(|s| {
                matches!(
                    s,
                    AgentStatus::Exited { .. } | AgentStatus::Killed | AgentStatus::Failed { .. }
                )
            })
            .count()
    }
    fn statuses(&self) -> Vec<AgentStatus> {
        self.statuses.lock().unwrap().clone()
    }
}
impl StatusSink for RecordingStatusSink {
    fn status_changed(&self, _id: AgentId, status: AgentStatus, _epoch: u32) {
        self.statuses.lock().unwrap().push(status);
    }
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

fn spec(program: &str, args: &[&str]) -> CommandSpec {
    CommandSpec {
        program: program.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        env: vec![],
        cwd: PathBuf::from("."),
    }
}

/// 자연 종료: cmd /c echo → stdout 에 마커 전달(seq 증가) → EOF → pump finish(Exited) 정확히 1회.
#[test]
fn stdio_stdout_pump_and_finalize_once() {
    let id = Uuid::new_v4();
    // structured 인자는 이 pump 테스트와 무관 — json 경로(true)로 open(caps 만 영향).
    let (transport, _pid) =
        StdioTransport::open(&spec("cmd.exe", &["/c", "echo OUT-MARKER"]), true).expect("open");

    let status_sink = RecordingStatusSink::new();
    let core = Arc::new(OutputCore::new(id, 0, Arc::new(status_sink.clone())));
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());

    let out = RecordingSink::new();
    core.subscribe(Arc::new(out.clone()));

    // stdout 마커 수신(pump 가 바이트를 core 로 흘림).
    assert!(
        wait_until(Duration::from_secs(5), || out.contains("OUT-MARKER")),
        "5s 내 stdout 마커 미수신"
    );

    // 자연 종료 → EOF → pump finish. 종점 Exited 도달.
    assert!(
        wait_until(Duration::from_secs(5), || matches!(
            core.status(),
            AgentStatus::Exited { .. }
        )),
        "자연 종료가 Exited 로 finish 되지 않음: {:?}",
        core.status()
    );
    core.join_pump(Duration::from_secs(5));

    // seq 는 0 부터 단조 증가(중복/역전 없음).
    let seqs = out.seqs();
    assert!(!seqs.is_empty(), "출력 청크가 없음");
    assert_eq!(seqs[0], 0, "첫 seq 는 0");
    assert!(
        seqs.windows(2).all(|w| w[1] > w[0]),
        "seq 가 단조 증가여야 함: {seqs:?}"
    );

    // finalize 정확히 1회 — terminal status 는 딱 1건.
    assert_eq!(
        status_sink.terminal_count(),
        1,
        "finalize 1회 위반 — terminal 전이 {}건: {:?}",
        status_sink.terminal_count(),
        status_sink.statuses()
    );
}

/// stderr 격리: stdout=OUT-MARKER / stderr=ERR-MARKER. 출력 스트림엔 stdout 만, stderr 는 안 섞임
/// (NDJSON 오염 방지 — ADR-0044). stderr 는 tracing::debug 로만 흐른다(출력 sink 도달 금지, FIX 4).
#[test]
fn stdio_stderr_not_in_output_stream() {
    let id = Uuid::new_v4();
    // 단일 인자 복합 명령: cmd 가 바깥 따옴표를 벗기고 `echo OUT& echo ERR 1>&2` 로 실행한다
    // (echo 의 stdout 을 stderr 로 리다이렉트). 마커는 ASCII 라 로케일 무관.
    let (transport, _pid) = StdioTransport::open(
        &spec("cmd.exe", &["/c", "echo OUT-MARKER& echo ERR-MARKER 1>&2"]),
        true,
    )
    .expect("open");

    let status_sink = RecordingStatusSink::new();
    let core = Arc::new(OutputCore::new(id, 0, Arc::new(status_sink)));
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());
    let out = RecordingSink::new();
    core.subscribe(Arc::new(out.clone()));

    assert!(
        wait_until(Duration::from_secs(5), || out.contains("OUT-MARKER")),
        "5s 내 stdout 마커 미수신"
    );
    // 종료까지 대기해 stderr 도 다 흘렀을 시점 확보.
    assert!(
        wait_until(Duration::from_secs(5), || matches!(
            core.status(),
            AgentStatus::Exited { .. }
        )),
        "종료 미도달"
    );
    core.join_pump(Duration::from_secs(5));
    // 여유 대기(stderr drain 스레드가 끝날 시간).
    std::thread::sleep(Duration::from_millis(200));

    assert!(out.contains("OUT-MARKER"), "stdout 마커는 출력에 있어야 함");
    assert!(
        !out.contains("ERR-MARKER"),
        "stderr 가 출력 스트림에 새어들어옴(NDJSON 오염): {}",
        String::from_utf8_lossy(&out.concat())
    );
}

/// stdin write 가 자식에 도달(echo-back) + 장수 프로세스 shutdown 이 timeout 내 kill.
/// cmd.exe(무인자, 파이프 stdin)는 stdin 을 안 닫으면 명령 대기로 계속 살아 있다 → 장수 자식.
#[test]
fn stdio_stdin_reaches_child_then_shutdown_kills() {
    let id = Uuid::new_v4();
    let (transport, _pid) = StdioTransport::open(&spec("cmd.exe", &[]), true).expect("open");

    let status_sink = RecordingStatusSink::new();
    let core = Arc::new(OutputCore::new(id, 0, Arc::new(status_sink)));
    let transport: Box<dyn AgentTransport> = Box::new(transport);
    transport.start(core.clone());
    let out = RecordingSink::new();
    core.subscribe(Arc::new(out.clone()));

    // stdin 으로 명령 주입 → cmd 가 실행 → 출력에 마커. (stdin 이 자식에 도달함을 증명)
    transport
        .send_input(InputEvent::Raw(b"echo engram-echo-back\r\n".to_vec()))
        .expect("send_input");
    assert!(
        wait_until(Duration::from_secs(5), || out.contains("engram-echo-back")),
        "5s 내 echo-back 미수신(stdin 이 자식에 도달 못 함): {}",
        String::from_utf8_lossy(&out.concat())
    );

    // 아직 살아 있음(장수 자식) — 종점 미도달.
    assert!(
        !matches!(
            core.status(),
            AgentStatus::Exited { .. } | AgentStatus::Killed
        ),
        "echo-back 후에도 살아 있어야 함(장수 자식)"
    );

    // shutdown → kill 트리 → stdout write 핸들 close → EOF → pump Killed. join 은 timeout 내 반환.
    let kill_started = Instant::now();
    transport.shutdown();
    core.join_pump(Duration::from_secs(5));
    let elapsed = kill_started.elapsed();
    assert!(
        elapsed < Duration::from_secs(5),
        "shutdown+join 이 5s 내 안 끝남(hang 의심): {elapsed:?}"
    );
    assert!(
        matches!(core.status(), AgentStatus::Killed),
        "shutdown 후 status 가 Killed 가 아님: {:?}",
        core.status()
    );
}
