//! The stop client — the wire half of the shell's one stop path (TRD S21 trd-3-2-u5 §3-2). The path itself (single flight · deadlines · the kill fallback) is `daemon_client::shutdown`.
//! - [`resolve_stop_target`] reads `daemon.json` once: a live daemon not newer than this app is the target — an older one too (ADR-0343 d2 · ADR-0344 d3). A record busy with another process answers [`Resolved::Busy`] (the caller retries). The kill fallback uses the identity resolved here, never a re-read. The token is read after the record: this account's token file for a daemon of protocol 8 or later, the record's own `token` field for an older one (it kept the token inside — ADR-0395).
//! - The stop dial goes through `account::AccountCheckedDialer` in production ([`crate::daemon_client::shutdown::StopPorts::local`]): a listener of another OS account gets nothing (ADR-0395).
//! - [`StopHandshake`] authenticates with the daemon's own protocol version (ADR-0343 d2 — the one exception to "never echo the daemon's version", `handshake.rs` Fix C) and ends on any text frame whose top-level JSON key is `"Hello"` (older daemons' `Hello` bodies differ). An `"Error"` frame is a terminal refusal.
//! - [`send_stop`] makes one dial on a one-shot registry (`max_attempts = 0`), bounded by the caller's time. Connected → `StopDaemon{force, kill_agents}` and nothing else, ever (the echoed version authenticates only this command). It answers [`StopSend::Sent`] or [`StopSend::NothingSent`] from what the transport reported; no decision is read from failure texts.
//! - No close right behind `StopDaemon`: after `Sent` it keeps reading until the daemon ends the link or the caller's bound. Closing at once made an older daemon's write fail and its `net` layer abort the reader before `StopDaemon` was dispatched — the daemon survived (measured QA regression of the old sync client).
// ADR-0343 · ADR-0344 · ADR-0395

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use engram_dashboard_protocol::handshake::AuthFrame;
use engram_dashboard_protocol::{AgentCommand, DaemonInfo, RequestId, Secret, PROTOCOL_VERSION};
use engram_dashboard_transport::{
    with_deadline, Address, Clock, Dialer, Frame, Handshake, HandshakeStep, Incoming, PeerId,
    Policy, Registry, RejectKind, TransportEvent,
};

use super::handshake::loggable;
use super::wire::{json_class, ShellWire};
use crate::daemon_client::shutdown::StopReason;
use crate::discovery::layout::DAEMON_FILE;
use crate::discovery::{check_acceptable, AcceptCheck, DataLayout, PidLiveness};

/// The daemon one stop flight aims at — its dial target, credential, protocol version and process identity, all from one `daemon.json` read (and the token read after it). `Debug` shows no token ([`Secret`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopTarget {
    pub address: Address,
    /// `None` = no token could be read — the handshake then sends nothing and refuses itself (nothing sent; a `User` stop goes on to the kill).
    pub token: Option<Secret>,
    pub protocol_version: u32,
    pub pid: u32,
    pub start_time: u64,
}

/// One read of the endpoint record, classified by the reader — the sharing-violation verdict is OS knowledge (`platform::fs`), so it stays on the real reader's side of the seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordRead {
    Bytes(Vec<u8>),
    Absent,
    Busy,
    Failed(String),
}

pub trait RecordReader: Send + Sync {
    fn read(&self) -> RecordRead;
    /// This account's token file (ADR-0395) — called only after [`Self::read`]; `None` = it cannot be read.
    fn read_token(&self) -> Option<Secret>;
}

pub struct DaemonFileReader {
    path: PathBuf,
    data_dir: PathBuf,
}

impl DaemonFileReader {
    pub fn new(data_dir: &std::path::Path) -> Self {
        Self {
            path: DataLayout::new(data_dir).daemon_file(),
            data_dir: data_dir.to_path_buf(),
        }
    }
}

impl RecordReader for DaemonFileReader {
    fn read(&self) -> RecordRead {
        match std::fs::read(&self.path) {
            Ok(bytes) => RecordRead::Bytes(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => RecordRead::Absent,
            Err(e) if engram_dashboard_platform::fs::is_sharing_violation(&e) => RecordRead::Busy,
            Err(e) => RecordRead::Failed(format!("{DAEMON_FILE} read failed: {e}")),
        }
    }

    fn read_token(&self) -> Option<Secret> {
        use super::account::Accounts;
        let read = super::account::OsAccounts
            .current()
            .and_then(|account| crate::discovery::read_token(&self.data_dir, &account));
        match read {
            Ok(token) => Some(token),
            Err(e) => {
                tracing::info!("stop: this account's daemon token file could not be read: {e}");
                None
            }
        }
    }
}

/// The first protocol whose daemon keeps the shell token out of `daemon.json`, in the per-account token file (ADR-0395).
const TOKEN_FILE_SINCE: u32 = 8;

/// The `token` field an older daemon's record still carries (protocol before [`TOKEN_FILE_SINCE`]) — the stop of an older daemon
///   authenticates with it (ADR-0343 d2), since such a daemon wrote no token file.
fn legacy_token(bytes: &[u8]) -> Option<Secret> {
    #[derive(serde::Deserialize)]
    struct Legacy {
        token: Option<Secret>,
    }
    serde_json::from_slice::<Legacy>(bytes).ok()?.token
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    Target(StopTarget),
    /// Nothing to stop — no record · a broken one · a dead pid · a daemon newer than this app. The text is a local diagnostic for the log.
    NoTarget(StopReason, String),
    Busy,
}

/// One `daemon.json` read → the stop target. Liveness comes before the version (ADR-0196 — `check_acceptable`), and only a daemon newer than this app is refused: an older one is exactly what the version band stops (ADR-0344 d3).
pub fn resolve_stop_target(reader: &dyn RecordReader, liveness: &dyn PidLiveness) -> Resolved {
    let bytes = match reader.read() {
        RecordRead::Bytes(bytes) => bytes,
        RecordRead::Absent => {
            return Resolved::NoTarget(StopReason::NoRecord, format!("no {DAEMON_FILE}"))
        }
        RecordRead::Busy => return Resolved::Busy,
        RecordRead::Failed(why) => return Resolved::NoTarget(StopReason::RecordUnreadable, why),
    };
    let info = match DaemonInfo::parse(&bytes) {
        Ok(info) => info,
        Err(e) => {
            return Resolved::NoTarget(
                StopReason::RecordBroken,
                format!(
                    "{DAEMON_FILE} does not parse ({} at line {} column {})",
                    json_class(&e),
                    e.line(),
                    e.column()
                ),
            )
        }
    };
    match check_acceptable(&info, liveness) {
        AcceptCheck::DeadPid => {
            return Resolved::NoTarget(
                StopReason::DeadPid,
                format!("{DAEMON_FILE} names pid {} which is not alive", info.pid),
            )
        }
        AcceptCheck::VersionMismatch { daemon } if daemon > PROTOCOL_VERSION => {
            return Resolved::NoTarget(
                StopReason::NewerDaemon,
                format!(
                    "the daemon speaks protocol {daemon}, newer than this app's {PROTOCOL_VERSION}"
                ),
            )
        }
        AcceptCheck::Accept | AcceptCheck::VersionMismatch { .. } => {}
    }
    let token = if info.protocol_version < TOKEN_FILE_SINCE {
        legacy_token(&bytes)
    } else {
        reader.read_token()
    };
    Resolved::Target(StopTarget {
        address: Address::new(format!("ws://{}:{}", info.host, info.port)),
        token,
        protocol_version: info.protocol_version,
        pid: info.pid,
        start_time: info.start_time,
    })
}

#[derive(Debug)]
pub struct StopHandshake {
    token: Option<Secret>,
    protocol_version: u32,
}

impl StopHandshake {
    pub fn new(target: &StopTarget) -> Self {
        Self {
            token: target.token.clone(),
            protocol_version: target.protocol_version,
        }
    }
}

/// The top-level key of a JSON object frame — the externally tagged variant name. `None` for anything else.
fn top_level_key(text: &str) -> Option<(String, serde_json::Value)> {
    match serde_json::from_str::<serde_json::Value>(text).ok()? {
        serde_json::Value::Object(map) if map.len() == 1 => map.into_iter().next(),
        _ => None,
    }
}

impl Handshake for StopHandshake {
    fn start(&self) -> HandshakeStep {
        let Some(token) = self.token.clone() else {
            return HandshakeStep::Reject {
                kind: RejectKind::Terminal,
                last: None,
                reason: "no token for this daemon".into(),
            };
        };
        // ADR-0343 d2: the daemon's own version, so a daemon of another version accepts this one command.
        let auth = AuthFrame::Auth {
            token,
            protocol_version: self.protocol_version,
        };
        // Serializing a two-field struct of a string and an integer cannot fail.
        let text = serde_json::to_string(&auth).expect("AuthFrame serializes");
        HandshakeStep::Send(Frame::Text(text))
    }

    fn on_frame(&self, frame: &Frame) -> HandshakeStep {
        let Frame::Text(text) = frame else {
            return HandshakeStep::Await;
        };
        match top_level_key(text) {
            Some((key, _)) if key == "Hello" => HandshakeStep::Done,
            Some((key, body)) if key == "Error" => {
                let message = body
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                match loggable(self.token.as_ref(), message) {
                    Some(detail) => tracing::warn!(%detail, "the daemon refused the stop handshake"),
                    None => tracing::warn!(
                        detail_bytes = message.len(),
                        "the daemon refused the stop handshake (its text carries the token and is not logged)"
                    ),
                }
                HandshakeStep::Reject {
                    kind: RejectKind::Terminal,
                    last: None,
                    reason: "the daemon refused the stop handshake".into(),
                }
            }
            _ => HandshakeStep::Await,
        }
    }
}

/// The one command the stop link writes — `force` and `kill_agents` set: an agent at work does not keep the daemon up (user decision). The daemon echoes the request id in its Ack, which nobody reads.
pub fn stop_command() -> AgentCommand {
    AgentCommand::StopDaemon {
        force: true,
        kill_agents: true,
        request_id: RequestId::new(),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StopDial {
    pub dial_bound: Duration,
    pub read_until: Instant,
    pub notify_before: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopSend {
    Sent {
        disconnected: bool,
    },
    /// Nothing reached the daemon: the dial or the handshake ended without `Hello` (refused · reset · closed · an `Error` frame · timed out), or the command could not be queued.
    NothingSent,
}

/// Label of the one-shot peer — its own registry, so it never meets the shell's link.
const STOP_PEER: &str = "daemon-stop";

/// One dial to `target` and, once authenticated, `StopDaemon` — see the module doc. Drains its inbox on this task.
pub async fn send_stop(
    dialer: Arc<dyn Dialer>,
    clock: Arc<dyn Clock>,
    target: &StopTarget,
    dial: StopDial,
) -> StopSend {
    let mut policy = Policy::default();
    // One dial: the first failure gives up (`Policy::check` accepts 0).
    policy.reconnect.max_attempts = 0;
    policy.connect_timeout = dial.dial_bound;
    let registry = Registry::new(Arc::new(ShellWire), dialer, clock.clone(), policy);
    let (peer, mut inbox) = registry.add(
        PeerId::new(STOP_PEER),
        target.address.clone(),
        Arc::new(StopHandshake::new(target)),
    );

    let left = |until: Instant| until.saturating_duration_since(clock.now());
    let connected = loop {
        match with_deadline(clock.as_ref(), left(dial.read_until), inbox.recv()).await {
            Ok(Some(Incoming::Event(TransportEvent::Connected { .. }))) => break true,
            Ok(Some(Incoming::Event(
                TransportEvent::ConnectFailed { .. }
                | TransportEvent::Rejected { .. }
                | TransportEvent::GaveUp { .. }
                | TransportEvent::Closed { .. },
            )))
            | Ok(None)
            | Err(_) => break false,
            Ok(Some(_)) => {}
        }
    };
    if !connected {
        peer.close();
        tracing::info!(
            pid = target.pid,
            "stop: the daemon was not reached — nothing sent"
        );
        return StopSend::NothingSent;
    }
    if clock.now() >= dial.notify_before {
        peer.close();
        tracing::warn!(
            pid = target.pid,
            "stop: the flight deadline passed before StopDaemon — not sent"
        );
        return StopSend::NothingSent;
    }
    if let Err(e) = peer.notify(stop_command()) {
        peer.close();
        tracing::warn!(pid = target.pid, error = %e, "stop: StopDaemon could not be queued — nothing sent");
        return StopSend::NothingSent;
    }
    tracing::info!(
        pid = target.pid,
        daemon_protocol = target.protocol_version,
        "stop: StopDaemon sent"
    );

    let disconnected = loop {
        match with_deadline(clock.as_ref(), left(dial.read_until), inbox.recv()).await {
            Ok(Some(Incoming::Event(
                TransportEvent::Disconnected { .. }
                | TransportEvent::GaveUp { .. }
                | TransportEvent::Closed { .. },
            )))
            | Ok(None) => break true,
            Err(_) => break false,
            Ok(Some(_)) => {}
        }
    };
    peer.close();
    StopSend::Sent { disconnected }
}

#[cfg(test)]
mod tests;
