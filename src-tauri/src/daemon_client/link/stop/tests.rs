//! The stop client on a memory network and a manual clock — the test side plays the daemon.

use super::*;

use std::sync::Mutex;
use std::time::Instant;

use engram_dashboard_protocol::AgentEvent;
use engram_dashboard_transport::testing::{ClientMsg, ManualClock, MemoryEndpoint, MemoryNetwork};
use tokio::task::JoinHandle;

use crate::discovery::tests::{info, FakeLiveness};

const TOKEN: &str = "dddd3333dddd3333dddd3333dddd3333dddd3333dddd3333dddd3333dddd3333";

/// Answers one scripted read.
struct OneRead(Mutex<Option<RecordRead>>);

impl OneRead {
    fn new(read: RecordRead) -> Self {
        Self(Mutex::new(Some(read)))
    }
}

impl RecordReader for OneRead {
    fn read(&self) -> RecordRead {
        self.0.lock().unwrap().take().expect("read once")
    }
}

fn record(pid: u32, version: u32) -> RecordRead {
    let mut daemon = info(pid, version);
    daemon.start_time = 77;
    RecordRead::Bytes(daemon.to_json_pretty().unwrap())
}

fn alive() -> FakeLiveness {
    FakeLiveness { dead: vec![] }
}

fn target(version: u32) -> StopTarget {
    StopTarget {
        address: Address::new("ws://127.0.0.1:5999"),
        token: Secret::new(TOKEN),
        protocol_version: version,
        pid: 4242,
        start_time: 77,
    }
}

fn text(ev: &AgentEvent) -> Frame {
    Frame::Text(serde_json::to_string(ev).unwrap())
}

fn hello() -> Frame {
    text(&AgentEvent::Hello {
        protocol_version: PROTOCOL_VERSION,
        daemon_version: "0.0.0".into(),
        capabilities: None,
    })
}

// ── target ──────────────────────────────────────────────────────────────────────────────

#[test]
fn a_live_daemon_of_this_version_is_the_target_with_its_identity() {
    let got = resolve_stop_target(&OneRead::new(record(1001, PROTOCOL_VERSION)), &alive());
    let Resolved::Target(t) = got else {
        panic!("expected a target, got {got:?}")
    };
    assert_eq!(
        (t.pid, t.start_time, t.protocol_version),
        (1001, 77, PROTOCOL_VERSION)
    );
    assert_eq!(t.address, Address::new("ws://127.0.0.1:12345"));
    assert_eq!(t.token.expose(), "t".repeat(64));
}

#[test]
fn an_older_daemon_is_a_target() {
    let got = resolve_stop_target(&OneRead::new(record(1002, PROTOCOL_VERSION - 1)), &alive());
    assert!(
        matches!(&got, Resolved::Target(t) if t.protocol_version == PROTOCOL_VERSION - 1),
        "{got:?}"
    );
}

#[test]
fn a_newer_a_dead_an_absent_a_broken_or_an_unreadable_record_is_no_target() {
    use crate::daemon_client::shutdown::StopReason;
    let cases = [
        (
            record(1003, PROTOCOL_VERSION + 1),
            alive(),
            StopReason::NewerDaemon,
        ),
        (
            record(1004, PROTOCOL_VERSION),
            FakeLiveness { dead: vec![1004] },
            StopReason::DeadPid,
        ),
        (RecordRead::Absent, alive(), StopReason::NoRecord),
        (
            RecordRead::Bytes(b"{ not json".to_vec()),
            alive(),
            StopReason::RecordBroken,
        ),
        (
            RecordRead::Failed("access denied".into()),
            alive(),
            StopReason::RecordUnreadable,
        ),
    ];
    for (read, liveness, reason) in cases {
        let shown = format!("{read:?}");
        let got = resolve_stop_target(&OneRead::new(read), &liveness);
        assert!(
            matches!(got, Resolved::NoTarget(r, _) if r == reason),
            "{shown} → {got:?}"
        );
    }
}

#[test]
fn a_dead_pid_wins_over_a_newer_version() {
    // ADR-0196 order: liveness first — the text names the dead pid, not the version.
    let got = resolve_stop_target(
        &OneRead::new(record(1005, PROTOCOL_VERSION + 1)),
        &FakeLiveness { dead: vec![1005] },
    );
    assert!(
        matches!(&got, Resolved::NoTarget(_, why) if why.contains("not alive")),
        "{got:?}"
    );
}

#[test]
fn a_busy_record_asks_for_a_retry() {
    assert_eq!(
        resolve_stop_target(&OneRead::new(RecordRead::Busy), &alive()),
        Resolved::Busy
    );
}

// ── handshake ───────────────────────────────────────────────────────────────────────────

#[test]
fn the_auth_frame_carries_the_daemons_own_version() {
    let old = PROTOCOL_VERSION - 1;
    match StopHandshake::new(&target(old)).start() {
        HandshakeStep::Send(Frame::Text(sent)) => assert_eq!(
            sent,
            format!(r#"{{"Auth":{{"token":"{TOKEN}","protocol_version":{old}}}}}"#)
        ),
        other => panic!("expected the Auth text frame, got {other:?}"),
    }
}

#[test]
fn any_frame_keyed_hello_ends_the_handshake_whatever_its_body() {
    let handshake = StopHandshake::new(&target(PROTOCOL_VERSION));
    for frame in [
        hello(),
        Frame::Text(r#"{"Hello":{}}"#.into()),
        Frame::Text(r#"{"Hello":{"protocol_version":6}}"#.into()),
    ] {
        assert!(
            matches!(handshake.on_frame(&frame), HandshakeStep::Done),
            "{frame:?}"
        );
    }
}

#[test]
fn an_error_frame_is_a_terminal_refusal_and_its_token_echo_is_not_logged() {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let echo = text(&AgentEvent::Error {
        request_id: None,
        message: format!("bad token {TOKEN}"),
    });
    let step = super::super::test_log::capture(lines.clone(), || {
        StopHandshake::new(&target(PROTOCOL_VERSION)).on_frame(&echo)
    });
    assert!(
        matches!(
            step,
            HandshakeStep::Reject {
                kind: RejectKind::Terminal,
                ..
            }
        ),
        "{step:?}"
    );
    let lines = lines.lock().unwrap();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(!lines[0].contains(TOKEN), "{lines:?}");
}

#[test]
fn other_frames_before_hello_are_dropped() {
    let handshake = StopHandshake::new(&target(PROTOCOL_VERSION));
    for frame in [
        Frame::Text(r#""Hello""#.into()),
        Frame::Text(r#"{"Hello":{},"x":1}"#.into()),
        Frame::Text("not json".into()),
        text(&AgentEvent::AgentListUpdated { agents: vec![] }),
        Frame::Binary(vec![1]),
    ] {
        assert!(
            matches!(handshake.on_frame(&frame), HandshakeStep::Await),
            "{frame:?}"
        );
    }
}

#[test]
fn the_stop_command_is_a_forced_stop_daemon() {
    let json = serde_json::to_string(&stop_command()).unwrap();
    assert!(json.starts_with(r#"{"StopDaemon":{"#), "{json}");
    assert!(json.contains(r#""force":true"#), "{json}");
    assert!(json.contains(r#""kill_agents":true"#), "{json}");
}

// ── send_stop ───────────────────────────────────────────────────────────────────────────

struct Wire {
    net: MemoryNetwork,
    clock: ManualClock,
    start: Instant,
}

impl Wire {
    fn new() -> Self {
        let clock = ManualClock::new();
        Self {
            net: MemoryNetwork::new(),
            start: clock.now(),
            clock,
        }
    }

    /// `send_stop` with a 9.5 s dial bound, reading until 10 s and writing until 13 s.
    fn send(&self, version: u32) -> JoinHandle<StopSend> {
        let dialer = self.net.dialer();
        let clock = self.clock.handle();
        let dial = StopDial {
            dial_bound: Duration::from_millis(9_500),
            read_until: self.start + Duration::from_secs(10),
            notify_before: self.start + Duration::from_secs(13),
        };
        tokio::spawn(async move { send_stop(dialer, clock, &target(version), dial).await })
    }

    async fn daemon(&self) -> Arc<MemoryEndpoint> {
        wait_for("a link is opened", || self.net.open_endpoints() > 0).await;
        self.net.accept().unwrap()
    }
}

async fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "not reached within 10 s: {what}");
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

async fn joined<T>(what: &str, task: JoinHandle<T>) -> T {
    wait_for(what, || task.is_finished()).await;
    task.await.expect("the task ran to its end")
}

/// The texts the client wrote so far — pings dropped.
fn written(endpoint: &MemoryEndpoint, into: &mut Vec<ClientMsg>) {
    into.extend(
        endpoint
            .drain()
            .into_iter()
            .filter(|m| !matches!(m, ClientMsg::Ping)),
    );
}

#[tokio::test]
async fn after_hello_only_stop_daemon_is_written_and_the_end_of_the_link_is_awaited() {
    let w = Wire::new();
    let old = PROTOCOL_VERSION - 1;
    let task = w.send(old);
    let daemon = w.daemon().await;
    let mut seen = Vec::new();
    wait_for("Auth", || {
        written(&daemon, &mut seen);
        !seen.is_empty()
    })
    .await;
    daemon.push(Frame::Text(r#"{"Hello":{"protocol_version":6}}"#.into()));
    wait_for("StopDaemon", || {
        written(&daemon, &mut seen);
        seen.len() >= 2
    })
    .await;
    // The client keeps the link open until the daemon ends it.
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    written(&daemon, &mut seen);
    assert!(
        !task.is_finished(),
        "no early return before the daemon ends the link"
    );
    daemon.close();
    assert_eq!(
        joined("send_stop", task).await,
        StopSend::Sent { disconnected: true }
    );

    let texts: Vec<String> = seen
        .iter()
        .map(|m| match m {
            ClientMsg::Frame(Frame::Text(t)) => t.clone(),
            other => panic!("only text frames before the daemon closes, got {other:?}"),
        })
        .collect();
    assert_eq!(texts.len(), 2, "{texts:?}");
    assert!(
        texts[0].contains(&format!(r#""protocol_version":{old}"#)),
        "{texts:?}"
    );
    assert!(texts[1].starts_with(r#"{"StopDaemon":"#), "{texts:?}");
}

#[tokio::test]
async fn a_daemon_that_never_ends_the_link_is_read_until_the_bound() {
    let w = Wire::new();
    let task = w.send(PROTOCOL_VERSION);
    let daemon = w.daemon().await;
    daemon.push(hello());
    let mut seen = Vec::new();
    wait_for("StopDaemon", || {
        written(&daemon, &mut seen);
        seen.len() >= 2
    })
    .await;
    w.clock.advance(Duration::from_secs(10));
    assert_eq!(
        joined("send_stop", task).await,
        StopSend::Sent {
            disconnected: false
        }
    );
}

#[tokio::test]
async fn the_dial_bound_is_the_handshake_bound() {
    let w = Wire::new();
    let task = w.send(PROTOCOL_VERSION);
    let _daemon = w.daemon().await;
    // A slow daemon still inside the bound keeps the attempt alive …
    w.clock.advance(Duration::from_millis(9_400));
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert!(!task.is_finished(), "still within the 9.5 s dial bound");
    // … and the transport gives up once it passes, before the 10 s read bound.
    w.clock.advance(Duration::from_millis(200));
    assert_eq!(joined("send_stop", task).await, StopSend::NothingSent);
    assert_eq!(w.net.dials(), 1, "one dial");
}

#[tokio::test]
async fn a_refused_dial_a_reset_or_an_error_frame_sends_nothing() {
    // Refused at the dial.
    let w = Wire::new();
    w.net.fail_next(1);
    assert_eq!(
        joined("refused", w.send(PROTOCOL_VERSION)).await,
        StopSend::NothingSent
    );
    assert_eq!(w.net.dials(), 1, "one dial — no retry");

    // Reset during the handshake.
    let w = Wire::new();
    let task = w.send(PROTOCOL_VERSION);
    w.daemon().await.fail("connection reset");
    assert_eq!(joined("reset", task).await, StopSend::NothingSent);

    // The daemon refuses with an `Error` frame and closes without a code.
    let w = Wire::new();
    let task = w.send(PROTOCOL_VERSION);
    let daemon = w.daemon().await;
    daemon.push(text(&AgentEvent::Error {
        request_id: None,
        message: "protocol_version mismatch".into(),
    }));
    daemon.close();
    assert_eq!(joined("error frame", task).await, StopSend::NothingSent);
    let mut seen = Vec::new();
    written(&daemon, &mut seen);
    assert!(
        !seen
            .iter()
            .any(|m| matches!(m, ClientMsg::Frame(Frame::Text(t)) if t.contains("StopDaemon"))),
        "{seen:?}"
    );
    assert_eq!(w.net.dials(), 1);
}

#[tokio::test]
async fn nothing_is_written_once_the_flight_deadline_has_passed() {
    let w = Wire::new();
    let dialer = w.net.dialer();
    let clock = w.clock.handle();
    let dial = StopDial {
        dial_bound: Duration::from_secs(9),
        read_until: w.start + Duration::from_secs(10),
        // Already due.
        notify_before: w.start,
    };
    let task =
        tokio::spawn(
            async move { send_stop(dialer, clock, &target(PROTOCOL_VERSION), dial).await },
        );
    let daemon = w.daemon().await;
    daemon.push(hello());
    assert_eq!(joined("send_stop", task).await, StopSend::NothingSent);
    let mut seen = Vec::new();
    written(&daemon, &mut seen);
    assert!(
        !seen
            .iter()
            .any(|m| matches!(m, ClientMsg::Frame(Frame::Text(t)) if t.contains("StopDaemon"))),
        "{seen:?}"
    );
}
