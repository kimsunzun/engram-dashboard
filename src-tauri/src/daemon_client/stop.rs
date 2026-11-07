//! Graceful daemon stop — fires `StopDaemon` one-way at the daemon over WS (tray 「데몬 끄기」 (stop daemon) · app exit).
//!
//! ★Temporary home★ — 3-2 (attaching transport to the shell) rewrites the stop-command client (ADR-0271 decision 5).
//!
//! The [`crate::discovery`] internals this file uses are `pub(crate)`: `FileReader` (and its field `path`) for this
//! file only · `check_acceptable` · `AcceptCheck` · `RealLiveness` also for `daemon_client::link::endpoint`, which
//! reads `daemon.json` itself.
//!
//! ★Division of labor (difference from daemon_stop)★: send_stop is the **graceful** path of the layer above — it sends
//! StopDaemon{force} to the daemon over WS so that the daemon itself does shutdown_all (cleaning up child PTYs) + self-exit
//! (handled by the connection_core StopDaemon handler).
//!
//! ★One-way fire (fire-and-forget) — user decision★: the ack/response is not read. If there is no response or the daemon is cleaning up,
//! the daemon stays alive as is (the probe reports alive), and if the user presses again it fires again. Only that the message actually goes out on the socket
//! through the flush before close is guaranteed.
// ADR-0271
// ADR-0282

use std::path::Path;
use std::time::Duration;

use engram_dashboard_protocol::handshake::AuthFrame;
use engram_dashboard_protocol::{AgentCommand, DaemonInfo, RequestId, Secret, PROTOCOL_VERSION};

use crate::discovery::{
    check_acceptable, AcceptCheck, DaemonReader, DataLayout, DiscoveryError, FileReader,
    PidLiveness, RealLiveness,
};

/// ★Why it is lifted into an enum (load-bearing)★: if right after the stop the tray decided the icon via
/// `daemon_status` (PID probe), for a few ms just before the daemon died it looked "still alive" and there was a race where **the icon
/// stuck on color** (QA measured). The fix = do not ask for the PID again, and use the **"daemon closed the connection"** observed in the drain
/// read loop as the "confirmed off" signal. send_stop raises that signal to the caller (the tray) as this enum, and the tray
/// confirms the icon gray by bypassing the probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    /// The daemon closed **this WS connection** gracefully. The tray confirms gray without a probe.
    /// ★Scope limit (do not over-trust)★: precisely, this is the signal that "the daemon processed StopDaemon, **entered the exit path**, and
    /// closed this connection". The actual process exit comes right after (usually ms) — connection close and
    /// process disappearance are not the same instant. On the normal path the gap is ms so confirming gray is right, but if the daemon's graceful
    /// shutdown itself hangs, the process may live a little longer even though the connection closed (that is a separate bug —
    /// the one-way fire re-fire model recovers on the next click). Do not trust this signal further as "process death confirmed" and
    /// remove the probe fallback additionally.
    DaemonClosed,
    /// The daemon neither closed nor responded within STOP_WS_TIMEOUT(3s).
    /// = uncertain (the daemon may still be cleaning up) → the tray uses the existing probe fallback.
    Timeout,
    /// There was no daemon to stop (no daemon.json / dead / broken / version mismatch — send itself is not done).
    /// = the tray uses the existing probe fallback (it should already be gray).
    NoTarget,
}

pub trait StopSender {
    /// Sends a graceful StopDaemon to the live daemon `info` (Auth → StopDaemon → flush → drain → close).
    /// Being one-way fire it does not interpret the ack **content**, but by the **termination reason** of the drain read it tells apart
    /// [`StopOutcome::DaemonClosed`] (connection closed = confirmed off) / [`StopOutcome::Timeout`] (3s no response) and
    /// returns it. Only send/connect failures are Err.
    fn send_stop(&self, info: &DaemonInfo) -> Result<StopOutcome, DiscoveryError>;
}

/// Sends a graceful StopDaemon to the daemon over WS (real entry point, S13 sub-step 2).
///
/// ★A place to attach later (load-bearing)★: this send_stop path has no taskkill (daemon_stop) forced fallback
/// (user decision: no response means the daemon stays active, pressing again fires again). To grow it later into graceful-with-fallback,
/// add "call daemon_stop(data_dir) on Timeout" **inside this function (or TungsteniteStopSender::send_stop)** — the
/// call site sees only the send_stop signature, so the fallback itself is absorbed
/// here.
pub fn send_stop(data_dir: &Path) -> Result<StopOutcome, DiscoveryError> {
    stop_with_sender(
        &FileReader {
            path: DataLayout::new(data_dir).daemon_file(),
        },
        &RealLiveness,
        &TungsteniteStopSender,
    )
}

/// ★Target decision = check_acceptable(Accept)★: a version-mismatched daemon's Auth rejects with the
/// protocol_version check anyway, so a one-way fire is pointless, and stopping such a daemon is the job of the taskkill
/// fallback (daemon_stop) (to be wired in later).
fn stop_with_sender(
    reader: &dyn DaemonReader,
    liveness: &dyn PidLiveness,
    sender: &dyn StopSender,
) -> Result<StopOutcome, DiscoveryError> {
    match reader.read() {
        Ok(Some(info)) => match check_acceptable(&info, liveness) {
            AcceptCheck::Accept => sender.send_stop(&info),
            AcceptCheck::DeadPid | AcceptCheck::VersionMismatch { .. } => Ok(StopOutcome::NoTarget),
        },
        _ => Ok(StopOutcome::NoTarget),
    }
}

/// force=true · kill_agents=true fixed (even with an agent at work the daemon cleans up and stops — user decision).
/// request_id is a new Uuid (the daemon echoes it but we do not read the ack so it is not used for matching — it is filled
/// only because it is a required protocol field).
fn build_stop_command() -> AgentCommand {
    AgentCommand::StopDaemon {
        force: true,
        kill_agents: true,
        request_id: RequestId::new(),
    }
}

/// The daemon expects this as the first frame of a connection, within the net lib's `AUTH_TIMEOUT`.
///
/// ★Type origin (ADR-0129 0-4)★: this is **not a command but a separate handshake frame** (protocol
/// `handshake::AuthFrame`). The daemon unpacks it with the same type, so instead of the sender hand-building the JSON
/// it uses the same type — that way a drift where only one side changes becomes a compile error.
fn build_auth_command(token: &Secret) -> AuthFrame {
    AuthFrame::Auth {
        token: token.clone(),
        protocol_version: PROTOCOL_VERSION,
    }
}

/// ★1:1 with the daemon handshake (ws.rs)★: the daemon read_task parses only Message::Text as AgentCommand and
/// rejects Binary, so it is sent as Text.
struct TungsteniteStopSender;

/// Upper bound (seconds) on send_stop's connect/handshake/read/write. Going past it falls cleanly into an error.
///
/// ★Why the timeout is load-bearing★: the default `tungstenite::connect(url)` calls the inner `TcpStream::connect`
/// **without a timeout**, and the handshake read has no bound either. In the rare case where the pid/port in daemon.json is stale
/// yet that PID was reused (M2) and slipped past the liveness decision, a connect attempt to a closed or firewall-blocked port
/// can block up to the Windows default ~21 seconds — the tray stop worker thread is tied up meanwhile, so
/// icon/state updates are delayed (close to a worker leak). connect_timeout + set_read/write_timeout put a bound on
/// every blocking stretch (TCP connect → WS handshake read → send/flush/close) and prevent an infinite block.
const STOP_WS_TIMEOUT: Duration = Duration::from_secs(3);

impl StopSender for TungsteniteStopSender {
    fn send_stop(&self, info: &DaemonInfo) -> Result<StopOutcome, DiscoveryError> {
        use std::net::{SocketAddr, TcpStream};
        use tungstenite::{Error as WsError, Message};

        // ws://host:port — the daemon is local plaintext WS (no TLS, ws:// fixed). host is always the 127.0.0.1 loopback.
        let url = format!("ws://{}:{}", info.host, info.port);

        let addr: SocketAddr = format!("{}:{}", info.host, info.port)
            .parse()
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon 주소 파싱 실패({url}): {e}")))?;
        let stream = TcpStream::connect_timeout(&addr, STOP_WS_TIMEOUT)
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon WS 접속 실패({url}): {e}")))?;
        stream
            .set_read_timeout(Some(STOP_WS_TIMEOUT))
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon read timeout 설정 실패: {e}")))?;
        stream
            .set_write_timeout(Some(STOP_WS_TIMEOUT))
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon write timeout 설정 실패: {e}")))?;
        // The request is only the URL, so the token does not enter the HandshakeError Display either.
        let (mut ws, _resp) = tungstenite::client(&url, stream)
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon WS handshake 실패({url}): {e}")))?;

        let auth = serde_json::to_string(&build_auth_command(&info.token))
            .map_err(|e| DiscoveryError::Io(format!("Auth 직렬화 실패: {e}")))?;
        ws.send(Message::Text(auth.into()))
            .map_err(|e| DiscoveryError::Io(format!("Auth 전송 실패: {e}")))?;

        let stop = serde_json::to_string(&build_stop_command())
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon 직렬화 실패: {e}")))?;
        ws.send(Message::Text(stop.into()))
            .map_err(|e| DiscoveryError::Io(format!("StopDaemon 전송 실패: {e}")))?;

        // ★The flush actually pushes the two frames onto the socket★ — tungstenite send buffers internally, so closing right away without a
        // flush may leave them unsent. This flush is the "delivery guarantee" of the one-way fire.
        ws.flush()
            .map_err(|e| DiscoveryError::Io(format!("WS flush 실패: {e}")))?;

        // ★drain read — no immediate close (QA measured regression fix)★:
        //    if ws.close() is called right after the flush, the daemon write_task writes outbound (Hello etc.) to the closed socket and
        //    fails with os error 10053 → write_task ends → by the daemon's "when one side ends, abort the other" (ws.rs)
        //    read_task aborts before it reads StopDaemon → StopDaemon is not dispatched → the daemon
        //    cannot graceful self-exit (it survives). The defect was that an immediate close took away from the daemon the "time to read and process StopDaemon".
        //    So read is run on the socket until the daemon closes the connection by self-exit (or read_timeout
        //    3s), giving it processing time.
        //    The read loop is for "giving the daemon time", not for "receiving". With the 3s bound, even if the daemon does not die
        //    send_stop returns after at most 3s (the same worker-block bound as the connect timeout).
        let outcome = loop {
            match ws.read() {
                Ok(Message::Close(_)) => break StopOutcome::DaemonClosed,
                Ok(_) => {}
                Err(WsError::ConnectionClosed) | Err(WsError::AlreadyClosed) => {
                    break StopOutcome::DaemonClosed
                }
                Err(WsError::Io(e)) => {
                    use std::io::ErrorKind;
                    match e.kind() {
                        ErrorKind::WouldBlock | ErrorKind::TimedOut => break StopOutcome::Timeout,
                        // EOF / connection dropped — the daemon process vanished and the socket dropped → confirmed off.
                        _ => break StopOutcome::DaemonClosed,
                    }
                }
                // Other WS errors (protocol/Utf8 etc.) — nothing more to receive so end, but since we cannot assert the daemon closed it,
                // treat as Timeout (uncertain) (safely recovered by the probe fallback).
                Err(_) => break StopOutcome::Timeout,
            }
        };

        // If the daemon already closed it this is harmless, and even if not it is closed by drop — the explicit close is best-effort.
        let _ = ws.close(None);
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    use crate::discovery::tests::{info, FakeLiveness, FakeReader};

    // ── send_stop (graceful StopDaemon WS one-way fire) — pure decision/assembly ──────────────────
    //
    // The real WS round trip is QA (real daemon) territory. Here we verify with a StopSender fake (1) the target decision (which daemons get a send and which do not),
    // (2) the assembly of the message to send (Auth/StopDaemon serialization shape).

    struct CountingStopSender {
        sent: RefCell<Vec<u32>>,
    }
    impl CountingStopSender {
        fn new() -> Self {
            Self {
                sent: RefCell::new(Vec::new()),
            }
        }
    }
    impl StopSender for CountingStopSender {
        fn send_stop(&self, info: &DaemonInfo) -> Result<StopOutcome, DiscoveryError> {
            self.sent.borrow_mut().push(info.pid);
            Ok(StopOutcome::DaemonClosed)
        }
    }

    #[test]
    fn send_stop_live_daemon_sends() {
        let reader = FakeReader::new(vec![Ok(Some(info(1001, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let sender = CountingStopSender::new();
        let outcome = stop_with_sender(&reader, &liveness, &sender).unwrap();
        assert_eq!(sender.sent.borrow().as_slice(), &[1001], "live 면 1회 발사");
        assert_eq!(
            outcome,
            StopOutcome::DaemonClosed,
            "sender 결과를 그대로 전파"
        );
    }

    #[test]
    fn send_stop_dead_daemon_is_noop() {
        let reader = FakeReader::new(vec![Ok(Some(info(1002, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![1002] };
        let sender = CountingStopSender::new();
        let outcome = stop_with_sender(&reader, &liveness, &sender).unwrap();
        assert_eq!(outcome, StopOutcome::NoTarget, "죽은 데몬 → NoTarget");
        assert!(
            sender.sent.borrow().is_empty(),
            "죽은 데몬엔 graceful stop 안 보냄"
        );
    }

    #[test]
    fn send_stop_missing_file_is_noop() {
        let reader = FakeReader::new(vec![Ok(None)]);
        let liveness = FakeLiveness { dead: vec![] };
        let sender = CountingStopSender::new();
        let outcome = stop_with_sender(&reader, &liveness, &sender).unwrap();
        assert_eq!(outcome, StopOutcome::NoTarget);
        assert!(sender.sent.borrow().is_empty());
    }

    #[test]
    fn send_stop_corrupt_file_is_noop() {
        let reader = FakeReader::new(vec![Err(DiscoveryError::Parse("bad".into()))]);
        let liveness = FakeLiveness { dead: vec![] };
        let sender = CountingStopSender::new();
        let outcome = stop_with_sender(&reader, &liveness, &sender).expect("깨진 파일은 no-op Ok");
        assert_eq!(outcome, StopOutcome::NoTarget);
        assert!(sender.sent.borrow().is_empty());
    }

    #[test]
    fn send_stop_version_mismatch_is_noop() {
        let reader = FakeReader::new(vec![Ok(Some(info(1003, PROTOCOL_VERSION + 1)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let sender = CountingStopSender::new();
        let outcome = stop_with_sender(&reader, &liveness, &sender).unwrap();
        assert_eq!(outcome, StopOutcome::NoTarget, "버전 불일치 → NoTarget");
        assert!(
            sender.sent.borrow().is_empty(),
            "버전 불일치는 graceful 대상 아님"
        );
    }

    #[test]
    fn send_stop_propagates_sender_outcome_timeout() {
        struct TimeoutSender;
        impl StopSender for TimeoutSender {
            fn send_stop(&self, _info: &DaemonInfo) -> Result<StopOutcome, DiscoveryError> {
                Ok(StopOutcome::Timeout)
            }
        }
        let reader = FakeReader::new(vec![Ok(Some(info(1005, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let outcome = stop_with_sender(&reader, &liveness, &TimeoutSender).unwrap();
        assert_eq!(
            outcome,
            StopOutcome::Timeout,
            "live 데몬 + sender Timeout → Timeout 전파"
        );
    }

    #[test]
    fn send_stop_propagates_sender_error() {
        struct FailingSender;
        impl StopSender for FailingSender {
            fn send_stop(&self, _info: &DaemonInfo) -> Result<StopOutcome, DiscoveryError> {
                Err(DiscoveryError::Io("send boom".into()))
            }
        }
        let reader = FakeReader::new(vec![Ok(Some(info(1004, PROTOCOL_VERSION)))]);
        let liveness = FakeLiveness { dead: vec![] };
        let err = stop_with_sender(&reader, &liveness, &FailingSender).unwrap_err();
        assert!(matches!(err, DiscoveryError::Io(_)), "{err:?}");
    }

    #[test]
    fn build_stop_command_is_force_kill_stopdaemon() {
        // Pins the shape that the daemon read_task's serde_json::from_str will parse.
        match build_stop_command() {
            AgentCommand::StopDaemon {
                force, kill_agents, ..
            } => {
                assert!(force, "force=true(작업 중 에이전트 있어도 끔)");
                assert!(kill_agents, "kill_agents=true");
            }
            other => panic!("StopDaemon 이 아님: {other:?}"),
        }
        let json = serde_json::to_string(&build_stop_command()).unwrap();
        assert!(
            json.contains("StopDaemon"),
            "externally-tagged 태그: {json}"
        );
        assert!(json.contains("\"force\":true"));
        assert!(json.contains("\"kill_agents\":true"));
    }

    #[test]
    fn build_auth_command_carries_token_and_version() {
        // ★A single variant makes this a non-falsifiable pattern (ADR-0129 0-4)★ — the old `other => panic!` branch is now an impossible
        //   state so it was deleted (the assertion did not get weaker; the compiler guarantees it instead).
        let token = "f".repeat(64);
        let AuthFrame::Auth {
            token: t,
            protocol_version,
        } = build_auth_command(&Secret::new(token.clone()));
        assert_eq!(t.expose(), token);
        assert_eq!(protocol_version, PROTOCOL_VERSION);

        // Wire shape — pins not only the tag's presence but the **whole bytes**. The protocol `handshake` holds the same string as a
        // golden, and whether this path actually emits those bytes is measured only here (if tray stop quietly fails
        // auth the daemon does not shut down).
        let json = serde_json::to_string(&build_auth_command(&Secret::new(token.clone()))).unwrap();
        assert_eq!(
            json,
            format!(r#"{{"Auth":{{"token":"{token}","protocol_version":{PROTOCOL_VERSION}}}}}"#),
            "핸드셰이크 wire 형태(externally-tagged)"
        );
    }
}
