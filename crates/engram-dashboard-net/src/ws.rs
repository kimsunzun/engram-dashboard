//! WebSocket server body — a network layer that does only socket lifecycle management (ADR-0129).
//!
//! Duty: upgrade an accepted TCP stream to WS (Origin allowlist) → token auth on the first frame within
//! [`AUTH_TIMEOUT`] → connection lifetime · single writer · keepalive · registry. **It does not know the vocabulary of frame content** — incoming text/binary
//! is raised to `ConnectionHandler` (frame_port), and outgoing frames are received through `FrameSink` (per connection) · `FrameFanout`
//! (all connections).
//!
//! ★Concurrency model (risk points)★
//! - **Single writer per connection**: SplitSink cannot write concurrently. So all output frames are put into a single per-connection
//!   `mpsc::Sender<Frame>` (conn_tx), and only write_task writes to the SinkHalf.
//!   The FIFO order of SubscribeAck→replay→live is guaranteed by this single queue.
//! - **Per-connection inbound queue (reading ≠ processing)**: the read loop decodes frames, puts them in the `Inbound` queue, and immediately reads the next
//!   frame. Handler calls are made in arrival order by the **single** consumer of that queue (`dispatch_task`).
//!   Tying the two into one line makes the processing time of one command the stall time of that whole connection (socket
//!   head-of-line blocking). Commands whose order carries meaning keep it through the property that there is one consumer —
//!   the decision to peel a long-running command off that line is the upper layer's job (the `frame_port::ConnectionHandler` contract).
//! - **try_send vs await boundary**: `FrameSink::try_send`, which the upper layer's sink calls from a pump thread, must never
//!   block. `FrameSink::send` in an async context may await (.send().await).
//! - **Out-of-band close signal (close_signal)**: when conn_tx is full, even the in-queue marker (`Frame::Close`)
//!   fails to try_send and the connection becomes a zombie. So there is an `Arc<Notify>` close_signal **outside** the queue.
//!   When `ConnFrameSink` meets full in try_send, it signals with `close_signal.notify_one()` (sync-safe),
//!   and write_task waits on conn_rx.recv() and close_signal.notified() simultaneously with `tokio::select!`,
//!   so even if the queue is blocked it wakes, does sink_half.close(), then break → cleanup.
//! - **Registry**: for all-connection fan-out. It keeps every connection's conn_tx in a ConnId→Sender map, and gives the upper layer
//!   only `FrameFanout` (try_send one opaque text to all connections). Registration · release · id issuing are connection lifetime held by this file,
//!   so they are not in that port — what that port can express is only one thing: "this text to everyone".

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::auth::AuthFrame;

use crate::frame_port::{
    ConnFlow, ConnId, ConnectionHandler, ConnectionHandlerFactory, Frame, FrameError, FrameFanout,
    FrameSink, Saturated,
};

use futures_util::future::BoxFuture;
use futures_util::{SinkExt, Stream, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Notify};
use tokio_tungstenite::tungstenite::handshake::server::{
    Callback, ErrorResponse, Request, Response,
};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::{Bytes, Message, Utf8Bytes};

/// Per-connection outbound queue capacity. ReplayBuffer.max_events(4096) + control_slack(512) = 4608.
/// Leaves control headroom even when the whole replay fits (consistent with the output_core.rs invariant).
const CONN_TX_CAP: usize = 4608;

/// **Number of entries** in the per-connection **inbound** arrival-order lane — headroom that lets the read loop read the next frame without waiting for the handler's work.
/// The memory axis is carried not by this value but by [`CONN_RX_MAX_BYTES`] (that doc is the source of truth for the accounting).
///
/// ★This queue is a **burst absorber**, not a backlog store★: what the consumer (`dispatch_task`) holds at a time
///   is only **one** frame whose arrival order carries meaning, and long work is peeled off this lane by the upper layer
///   (the 「도착 순서」 (arrival order) contract of `frame_port::ConnectionHandler`). So unlike [`CONN_TX_CAP`], which must hold
///   the entire replay, no large value is needed — do not compare the two values on the same axis.
/// ★★Even when full, **the read loop does not stop** — it asks the upper layer how to dispose of that one frame and keeps reading★★
///   ([`ConnectionHandler::on_inbound_saturated`]). ★The 「가득 차면 읽기 루프가
///   기다린다」 (when full, the read loop waits) that was once written here **was a wrong design**★: TCP is a stream, so while waiting **the frames behind it
///   are not even taken out of the socket**. Then a frame the upper layer chose as 「굶으면 안 된다」 (must not starve) ([`Saturated::Bypass`])
///   starves behind the blocked lane, and the very symptom this arrangement was meant to eliminate comes back by another path.
///   ㆍ**Even so it does not drop silently**: asking the upper layer for the disposition and getting [`Saturated::Refused`] means
///     **the upper layer has already sent a reply** (obligation 2 of the port contract). The disposition for when even that reply could not be sent is
///     held separately by [`Saturated::Unanswered`] — only there is the connection ended.
///   ㆍ**Otherwise it does not disconnect either**: the outbound queue ([`ConnFrameSink::try_send`]) disconnects on saturation
///     because its caller is a synchronous path that **cannot produce a reply** (the source of truth for the asymmetry =
///     `frame_port::FrameSink`). Here the upper layer can state the refusal in words, so there is no reason to kill the connection.
/// ★So what this value measures★ = 「소비자가 막힌 동안 **손실 없이** 받아 둘 프레임 **개수**」 (the **number** of frames to hold **without loss** while the consumer is blocked). Whichever of the two
///   is hit first is the saturation — the entries all fill, or the byte budget all fills.
/// ★When the consumer is healthy this value is not even seen★: what remains in the lane is only short frames with no await point
///   (long ones are peeled off by the upper layer), so in normal operation the 64 entries never fill. Being full is a signal that **the consumer
///   is blocked**, and the right answer in that state is not to wait but to let through what should be skipped.
// ADR-0206
const CONN_RX_CAP: usize = 64;

/// Budget of **payload bytes** the arrival-order lane may hold — decides saturation **together with** [`CONN_RX_CAP`].
///
/// ★Why the entry count alone is not enough★: what sits in an entry is the payload handle tungstenite handed over, so **each entry
///   differs in size**. Counting only entries makes this lane's real ceiling `CONN_RX_CAP × (한 프레임의 상한)`, and that
///   limit is **tungstenite's default `max_message_size` = 64 MiB**, which this crate never set
///   (no `WebSocketConfig` is passed to the upgrade — `accept_hdr_async` in `handle_connection`).
///   That is, 64 × 64 MiB = **4 GiB**. ★The 「프레임 하나의 상한은 이 상수의 관심사가
///   아니다」 (the limit of one frame is not this constant's concern) once written here is a sentence that made nobody count that product★ — one of the two must bear the accounting.
/// ★The decision uses **the fill level before adding**★: if the bytes already held are at or above this value, it counts as saturated. So a frame arriving at an empty lane
///   **is always accepted, whatever its size** — a legitimate command is never refused merely for being large.
///   The price is that the ceiling becomes 「예산 + 프레임 하나」 (budget + one frame) (= 16 MiB + max_message_size).
/// ★The remaining term — written honestly★: to narrow the second term of the product above (the limit of one frame), the upgrade would have to be passed
///   a `WebSocketConfig`, but that is a **behavior change that alters the size of frames accepted**, so it is a separate matter.
///   Here only the count axis is closed.
/// ★Basis of the value★: what stands in this lane is short control frames, and a large one is an occasional single frame — 16 MiB leaves room even if that
///   「어쩌다 한 장」 (an occasional single frame) overlaps several times, and a normal burst never sees this value.
const CONN_RX_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Capacity of the lane where a frame that skipped the arrival-order lane ([`Saturated::Bypass`]) stands.
///
/// ★Why it is small★: what enters this lane is only 「막힌 줄을 풀려는 것」 (things that try to unblock the blocked lane), so in normal operation it **uses not a single entry**.
///   8 entries being full means eight were sent and not one finished — that path itself is blocked,
///   and accepting more then only lines up the same thing eight more times.
/// ★Here it waits (unlike the lane above)★: if reading stops when this lane is full, what starves is **the frames bound for the arrival-order lane**,
///   and those were about to be refused anyway. That is, waiting here does not starve anything that must be protected.
/// ★The byte budget is **not applied** here★: refusing by budget a frame the upper layer chose as 「굶으면 안 된다」 (must not starve)
///   removes the reason this lane exists. So this lane's ceiling is the entry count as is, and its value is
///   8 × max_message_size (the same term as the second term written in [`CONN_RX_MAX_BYTES`]). The property that this lane is
///   empty in normal operation makes that ceiling effectively 0 — it is an observation, not a guarantee, so it is written as is.
const CONN_BYPASS_CAP: usize = 8;

/// Grace period in connection cleanup, waiting until the priority lane **empties what it has accepted** ([`finish_bypass_lane`]).
///
/// ★This value does **not** bear correctness★ — correctness is borne by 「기다린다」 (waiting) itself, and this value is only the upper bound that keeps that wait
///   from becoming infinite. Past the grace period it falls back to the old behavior (abort), so even if this constant
///   is wrong it **cannot be worse than before**.
/// ★Basis of the size★: one item in this lane is an action in the upper layer that 「막힌 것을 끝내는」 (finishes something blocked), and such actions have a section
///   that synchronously waits for the target to die (today the upper bound of that section is the upper layer's 5-second join). With 10 seconds the one in progress
///   + the next one finish, and if more is backed up than that, that path itself is blocked, and waiting longer only
///   lines up the same thing (the same judgment as 「8칸이 차 있다는 것은」 (8 entries being full means) in [`CONN_BYPASS_CAP`]).
/// ★What is held during this grace period★ = only **the cleanup of that one connection**. The socket is already finished, and neither the accept loop nor
///   other connections are caught by this wait.
const BYPASS_DRAIN_GRACE: Duration = Duration::from_secs(10);

/// How long a new connection may stay open without its first (auth) frame before it is closed with an
/// "auth timeout" error. 15 s — the open-source defaults read are 3–60 s, mostly 5–15 s (SignalR
/// `HandshakeTimeout` 15 s, EMQX `idle_timeout` 15 s — `docs/research/first-auth-frame-timeout-2026-11-06.md`).
/// A short window turns a briefly stalled shell into a terminal refusal: the shell reads any handshake `Error`
/// as terminal (TRD trd-3 revision 9 ⑧). There is still no cap on pre-auth connections on this path (TRD §5 14
/// · R11) — the transport daemon's caps land in 3-3 U4.
// ADR-0347
const AUTH_TIMEOUT: Duration = Duration::from_secs(15);

const DEFAULT_PING_INTERVAL: Duration = Duration::from_secs(20);
/// 2.5x ping_interval (the connection drops only after several Pings are missed — prevents false positives from a transient delay).
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(50);

/// WS application-level keepalive settings (A).
///
/// ★Half-open detection★: tungstenite only auto-Pongs incoming Pings and does not send active Pings.
/// For connections that drop without FIN (sleep/wake · NAT timeout · mobile tunnel), TCP keepalive (default 2 hours)
/// is meaningless, so write_task sends a Ping every ping_interval and read_task
/// records the last receive time. If idle_timeout is exceeded, that connection is dropped (prevents zombie subscription/broadcast leaks).
///
/// ★Test injection★: hard-coding constants would make tests take tens of seconds, so short values (e.g. 200ms/600ms)
/// are made configurable for injection. The production path stays `default()` (20s/50s) as is.
#[derive(Clone, Copy, Debug)]
pub struct KeepaliveConfig {
    pub ping_interval: Duration,
    pub idle_timeout: Duration,
}

impl Default for KeepaliveConfig {
    fn default() -> Self {
        Self {
            ping_interval: DEFAULT_PING_INTERVAL,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
        }
    }
}

/// Allowed Origin allowlist. No Origin (native/harness) is allowed — the token is the main defense.
/// ★Unmeasured — known unverified★: the four strings below are **design values**. The Origins actually sent by Tauri WebView2 · mobile
///   have never been measured and confirmed. A mismatch = 403. When growing the client surface (packaging variants · mobile tunnels, etc.), do not treat this list
///   as verified — a 403 may be this list's fault, not a client bug.
const ALLOWED_ORIGINS: &[&str] = &[
    "http://localhost:1420",
    "http://127.0.0.1:1420",
    "tauri://localhost",
    "https://tauri.localhost",
];

#[derive(Clone)]
pub struct ConnRegistry {
    inner: Arc<Mutex<HashMap<ConnId, mpsc::Sender<Frame>>>>,
    next_id: Arc<AtomicU64>,
}

impl ConnRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    fn alloc_id(&self) -> ConnId {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn register(&self, id: ConnId, tx: mpsc::Sender<Frame>) {
        self.inner
            .lock()
            .expect("conn registry poisoned")
            .insert(id, tx);
    }

    fn unregister(&self, id: ConnId) {
        self.inner
            .lock()
            .expect("conn registry poisoned")
            .remove(&id);
    }

    /// Whether this conn is still a fan-out target — the test observes the cleanup order (release registration after cleanup).
    #[cfg(test)]
    pub(crate) fn contains(&self, id: ConnId) -> bool {
        self.inner
            .lock()
            .expect("conn registry poisoned")
            .contains_key(&id)
    }
}

impl FrameFanout for ConnRegistry {
    /// ★Do not send while holding the map locked (ADR-0006 lock order)★: take a snapshot, release the lock, and send with that copy
    ///   — so as not to make a call that goes outside (a channel) while holding the lock.
    fn broadcast_text(&self, text: String) {
        let conns: Vec<(ConnId, mpsc::Sender<Frame>)> = {
            let guard = self.inner.lock().expect("conn registry poisoned");
            guard.iter().map(|(id, tx)| (*id, tx.clone())).collect()
        };
        for (id, tx) in conns {
            if let Err(e) = tx.try_send(Frame::Text(text.clone())) {
                tracing::warn!(conn = id, "전-연결 팬아웃 try_send 실패(느린 소비자): {e}");
            }
        }
    }
}

impl Default for ConnRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── ConnFrameSink (per-connection frame outlet — the WS implementation of the frame port) ──────────────────────

pub(crate) struct ConnFrameSink {
    conn_tx: mpsc::Sender<Frame>,
    close_signal: Arc<Notify>,
}

impl ConnFrameSink {
    pub(crate) fn new(conn_tx: mpsc::Sender<Frame>, close_signal: Arc<Notify>) -> Self {
        Self {
            conn_tx,
            close_signal,
        }
    }
}

impl FrameSink for ConnFrameSink {
    fn try_send(&self, frame: Frame) -> Result<(), FrameError> {
        match self.conn_tx.try_send(frame) {
            Ok(()) => Ok(()),
            Err(_) => {
                self.close_signal.notify_one();
                Err(FrameError)
            }
        }
    }

    fn send(&self, frame: Frame) -> BoxFuture<'_, Result<(), FrameError>> {
        Box::pin(async move { self.conn_tx.send(frame).await.map_err(|_| FrameError) })
    }
}

// ── Origin allowlist 콜백 ─────────────────────────────────────────────────────────

/// Upgrade callback — checks the Origin header. Allowed if absent (native/harness); rejected if present and outside the allowlist.
struct OriginCheck;

impl Callback for OriginCheck {
    fn on_request(self, request: &Request, response: Response) -> Result<Response, ErrorResponse> {
        match request.headers().get("origin") {
            None => {
                tracing::debug!("WS upgrade: Origin 없음 — 허용(토큰 검증으로 방어)");
                Ok(response)
            }
            Some(value) => {
                let origin = value.to_str().unwrap_or("");
                if ALLOWED_ORIGINS.contains(&origin) {
                    tracing::debug!(origin, "WS upgrade: Origin 허용");
                    Ok(response)
                } else {
                    tracing::warn!(origin, "WS upgrade: Origin 불일치 — 거부");
                    let mut resp = ErrorResponse::new(Some("origin not allowed".into()));
                    *resp.status_mut() = StatusCode::FORBIDDEN;
                    Err(resp)
                }
            }
        }
    }
}

// ── Constant-time token comparison ──────────────────────────────────────────────────────────

/// Exposing the length is harmless since the token length is fixed (64 hex chars).
fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ── Connection handler ────────────────────────────────────────────────────────────────

/// `expected_token` is the token from daemon.json. `handlers` is the factory of upper-layer handlers to attach to this connection —
/// the meaning of frames (command interpretation · event encoding · connection cleanup) is all owned by that side, so this function does not know
/// agent vocabulary (ADR-0129).
///
/// ★`expected_protocol_version` is also **injected** (0-4)★: in the same vein as the token, the assembly part puts the value in.
/// So this crate does not know "what the current version is" and decides only **"is the number the client stated equal to the number I received"** —
/// reading the version constant here would, at that moment, bring protocol vocabulary back into the network layer.
pub async fn handle_connection(
    stream: TcpStream,
    peer: std::net::SocketAddr,
    registry: ConnRegistry,
    handlers: Arc<dyn ConnectionHandlerFactory>,
    expected_token: Arc<String>,
    expected_protocol_version: u32,
    keepalive: KeepaliveConfig,
) {
    let mut ws = match tokio_tungstenite::accept_hdr_async(stream, OriginCheck).await {
        Ok(ws) => ws,
        Err(e) => {
            tracing::warn!(%peer, "WS 업그레이드 실패(또는 Origin 거부): {e}");
            return;
        }
    };

    match tokio::time::timeout(AUTH_TIMEOUT, ws.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => {
            match serde_json::from_str::<AuthFrame>(&text) {
                Ok(AuthFrame::Auth {
                    token,
                    protocol_version,
                }) => {
                    // Security: no logging of the token value.
                    if !constant_time_eq(token.expose(), expected_token.as_str()) {
                        tracing::warn!(%peer, "auth 실패: 토큰 불일치 — close");
                        let _ = send_error_and_close(
                            &mut ws,
                            handlers.handshake_error_frame("auth failed"),
                        )
                        .await;
                        return;
                    }
                    if protocol_version != expected_protocol_version {
                        tracing::warn!(
                            %peer,
                            client = protocol_version,
                            server = expected_protocol_version,
                            "auth 실패: protocol_version 불일치 — close"
                        );
                        let _ = send_error_and_close(
                            &mut ws,
                            handlers.handshake_error_frame(&format!(
                                "protocol_version mismatch: client {protocol_version} != server {expected_protocol_version}"
                            )),
                        )
                        .await;
                        return;
                    }
                    tracing::info!(%peer, "auth 성공");
                }
                // ★The boundary of the two failure messages★: this crate does not know upper-layer commands, so it cannot decide "parses as an upper-layer command
                //   but is not auth" —
                //   instead it uses the error classification serde already computed (zero re-parsing):
                //     · data error  = JSON is valid but not this shape → "expected auth"
                //     · anything else (syntax/eof/io) = not JSON at all → "the frame itself is wrong"
                //   No test asserts this wording (handshake failure is observed through close) and none of the senders
                //   branches on the wording — it is diagnostic text for humans.
                // ★Do not carry the error as `{e}`★: the Display of `{"Auth":"<토큰>"}` emits that string
                //   as is, and the token leaks into the log (`docs/reference/logging-conventions.md`). Leave only the classification and position
                //   — the regression defense is `auth_parse_failure_log_fields_do_not_carry_the_payload` below.
                Err(e) if e.is_data() => {
                    tracing::warn!(
                        %peer,
                        kind = ?e.classify(),
                        line = e.line(),
                        column = e.column(),
                        "첫 frame 이 Auth 가 아님 — close"
                    );
                    let _ = send_error_and_close(
                        &mut ws,
                        handlers.handshake_error_frame("expected Auth as first frame"),
                    )
                    .await;
                    return;
                }
                // For the same reason as the branch above, do not carry Display (the token can arrive on this branch too).
                Err(e) => {
                    tracing::warn!(
                        %peer,
                        kind = ?e.classify(),
                        line = e.line(),
                        column = e.column(),
                        "첫 frame 파싱 실패 — close"
                    );
                    let _ = send_error_and_close(
                        &mut ws,
                        handlers.handshake_error_frame("invalid first frame"),
                    )
                    .await;
                    return;
                }
            }
        }
        Ok(Some(Ok(_))) => {
            tracing::warn!(%peer, "첫 frame 이 Text 가 아님 — close");
            let _ = send_error_and_close(
                &mut ws,
                handlers.handshake_error_frame("expected Auth text frame"),
            )
            .await;
            return;
        }
        Ok(Some(Err(e))) => {
            tracing::warn!(%peer, "첫 frame 수신 오류: {e} — close");
            return;
        }
        Ok(None) => {
            tracing::warn!(%peer, "auth 전에 연결 종료");
            return;
        }
        Err(_) => {
            tracing::warn!(%peer, timeout_ms = AUTH_TIMEOUT.as_millis() as u64, "no auth frame in time — closing");
            let _ =
                send_error_and_close(&mut ws, handlers.handshake_error_frame("auth timeout")).await;
            return;
        }
    }

    let (conn_tx, conn_rx) = mpsc::channel::<Frame>(CONN_TX_CAP);
    let close_signal = Arc::new(Notify::new());
    let conn_id = registry.alloc_id();
    registry.register(conn_id, conn_tx.clone());
    tracing::info!(%peer, conn = conn_id, "연결 인증 완료 — 등록");

    let (sink_half, stream_half) = ws.split();

    let frames: Arc<dyn FrameSink> =
        Arc::new(ConnFrameSink::new(conn_tx.clone(), close_signal.clone()));
    let handler = handlers.handler_for(conn_id);

    // In order, this must come before spawning the two tasks so that command dispatch cannot get ahead of the greeting.
    handler.on_connect(conn_id, &frames).await;
    // ★Observe only and do not change the flow★: the reason not to use a panic is that in the production daemon it is a process exit
    //   (the hook — ADR-0292), and in a hookless build, dying here would skip the cleanup hook and registry release below wholesale and leave a dead
    //   queue as a fan-out target (an exit path that was not in HEAD).
    // ★The caught range = only "the one boundary where on_connect **returned** exactly full"★: the truly dangerous side
    //   (putting in over capacity so `send` waits forever) **cannot even reach this line** — that hang leaves
    //   no log anywhere (a known unlogged hole — described in the `ConnectionHandler::on_connect` contract).
    // ★It does not mean "blocked right now"★: right below, write_task starts and drains the queue, so progress continues.
    if conn_tx.capacity() == 0 {
        tracing::warn!(
            conn = conn_id,
            "on_connect 반환 시점에 연결 큐가 가득 찼다 — 한 프레임만 더 넣었으면 writer 기동 전에 영구 대기했을 것(핸들러 푸시 + 등록 후 전-연결 팬아웃 합계)"
        );
    }

    // ── keepalive shared clock (A) ──────────────────────────────────────────────────────
    // base = connection start time (tokio Instant). last_recv = elapsed ms relative to base (AtomicU64).
    // read_task updates it whenever it receives anything (Pong included) from the client, and write_task's ping arm computes
    // the idle elapsed time with base.elapsed() - last_recv and fires close_signal if idle_timeout is exceeded.
    let keepalive_base = tokio::time::Instant::now();
    let last_recv = Arc::new(AtomicU64::new(0));

    // ★Inbound queue = the boundary between the read layer and the processing layer★: reading only takes from the socket and puts here, and handler calls
    //   are borne by [`dispatch_task`] in arrival order. The basis of capacity and saturation disposition is [`CONN_RX_CAP`].
    let (inbound_tx, inbound_rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
    // ★The fill level of the byte budget that decides saturation **together with** the entry count★ — source of truth for the basis = [`CONN_RX_MAX_BYTES`].
    let rx_bytes: RxBytes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    // ★The second lane for frames that must not starve behind a blocked lane★ — why it must have separate capacity · a separate consumer
    //   is held by [`CONN_BYPASS_CAP`] and [`bypass_task`] respectively.
    let (bypass_tx, bypass_rx) = mpsc::channel::<Inbound>(CONN_BYPASS_CAP);

    let read_handle = tokio::spawn(read_task(
        stream_half,
        inbound_tx,
        bypass_tx,
        rx_bytes.clone(),
        frames.clone(),
        handler.clone(),
        conn_id,
        keepalive_base,
        last_recv.clone(),
    ));

    let bypass_handle = tokio::spawn(bypass_task(
        bypass_rx,
        frames.clone(),
        handler.clone(),
        conn_id,
    ));

    // ★The strong reference to the frame outlet moved from read_task to this task★ — the observation of `the_losing_task_is_aborted_not_detached`
    //   stands on that fact (that test's comment).
    let mut dispatch_handle = tokio::spawn(dispatch_task(
        inbound_rx,
        rx_bytes,
        frames,
        handler.clone(),
        conn_id,
    ));

    let mut write_handle = tokio::spawn(write_task(
        sink_half,
        conn_rx,
        conn_id,
        close_signal,
        keepalive,
        keepalive_base,
        last_recv,
    ));

    // ★Explicitly abort the survivor★ — if a JoinHandle is simply dropped,
    //    the task is detached and keeps running (a zombie holding the WS half). So select on &mut so the handle
    //    is not consumed, and abort the losing side (so the connection's read/write end together).
    //    Regression defense = `the_losing_task_is_aborted_not_detached` (only the branch that aborts read).
    //    ★Do not remove the abort on the branch that aborts write either★: usually all senders are dropped and write_task
    //    ends by itself, but that is conditional on "all copies of Sender<Frame> die together" — if a copy survives because of a missed
    //    subscription record (the on_disconnect race below), self-termination does not hold. The full enumeration and the measured
    //    range are in that test's comment.
    // ★★There are three tasks but **two** select targets — this is intended; read_task is not among them★★:
    //    The read loop ending (client close · EOF · receive error) means 「입력이 끝났다」 (the input has ended), not 「이 연결을 그
    //    자리에서 끊으라」 (cut this connection on the spot). When it ends, the inbound queue's **only sender** is dropped, so dispatch_task
    //    ends by itself **after** processing what is left, and the first branch below catches that end. Putting read here and aborting
    //    immediately would cut off the last command of a client that 「명령 한 장 보내고 곧바로 소켓을 닫는」 (sends one command and closes the socket right away) before it is processed —
    //    in HEAD, where reading and processing were one line, that command was **always** finished before the close frame was read,
    //    so that would be a regression.
    //    ★The price = if the handler never returns, this connection is not cleaned up★. HEAD was the same (then the read
    //    loop stayed parked inside `on_text`) — this is not a risk this arrangement newly created.
    //    ★The priority lane (bypass_task) is not a select target either★ — its ending only means read has ended
    //    (its own sender is in read), and is not a reason to cut the connection. The intent to end that it raises
    //    goes through the write lane as `Frame::Close` and comes back to the second branch below (that task's comment).
    //    ★★The priority lane is **not aborted — it is drained**★★: the frames in that lane are ones the upper layer chose as 「굶으면 안 된다」 (must not starve) and
    //    **accepted**. Aborting here would be accepting and then not doing, and the reason this lane was made
    //    collapses entirely (the arrival-order lane has no such hole — when its sender is dropped it drains what is left and ends).
    //    So cut read first to release **that lane's only sender**, and [`finish_bypass_lane`] waits for
    //    that draining.
    tokio::select! {
        _ = &mut dispatch_handle => {
            tracing::debug!(conn = conn_id, "dispatch_task 종료 → read abort · bypass 비움 · write abort + cleanup");
            read_handle.abort();
            finish_bypass_lane(bypass_handle, conn_id).await;
            write_handle.abort();
        }
        _ = &mut write_handle => {
            tracing::debug!(conn = conn_id, "write_task 종료 → read abort · bypass 비움 · dispatch abort + cleanup");
            read_handle.abort();
            finish_bypass_lane(bypass_handle, conn_id).await;
            dispatch_handle.abort();
        }
    }

    // ── cleanup (leak prevention — review required) ──────────────────────────────────────────
    handler.on_disconnect(conn_id);

    registry.unregister(conn_id);
    tracing::info!(%peer, conn = conn_id, "연결 종료 — cleanup 완료");
}

/// Writes the handshake-failure notice + close directly to the socket (no single-writer queue yet since it is before registry registration).
async fn send_error_and_close(
    ws: &mut tokio_tungstenite::WebSocketStream<TcpStream>,
    frame: Option<String>,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    if let Some(text) = frame {
        ws.send(Message::Text(text.into())).await?;
    }
    ws.close(None).await
}

// ── write_task(단일 writer) ───────────────────────────────────────────────────

type SinkHalf =
    futures_util::stream::SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, Message>;

/// ★Known hole — an in-progress socket write is not cut by this signal (same in HEAD, outside this slice's scope)★:
/// the recv arm, **after** taking a frame, waits on `sink_half.send(msg).await` **outside select!** (in the arm body).
/// If an authenticated peer stops reading, that send can be pending indefinitely, and during that time this task does not poll select!, so
/// neither `close_signal` nor the keepalive tick can save that connection — they are in the same select!, so both stop together.
/// That is, the "slow consumer cleanup" the code advertises **does not cover this case**.
/// `Notify` keeps a permit when there is no waiter, so the wake-up is **a delay, not a loss** (it fires immediately once that send is eventually released).
/// To fix it, a timeout/cancel would have to be put on the write itself, but that is a behavior change.
#[allow(clippy::too_many_arguments)]
async fn write_task(
    mut sink_half: SinkHalf,
    mut conn_rx: mpsc::Receiver<Frame>,
    conn_id: ConnId,
    close_signal: Arc<Notify>,
    keepalive: KeepaliveConfig,
    keepalive_base: tokio::time::Instant,
    last_recv: Arc<AtomicU64>,
) {
    let mut ping_tick = tokio::time::interval(keepalive.ping_interval);
    // Prevents the first tick firing immediately (so a Ping is not shot right after connecting) — start from the normal first period.
    ping_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = close_signal.notified() => {
                tracing::info!(conn = conn_id, "write_task: close_signal(슬로우 소비자) — 종료");
                let _ = sink_half.close().await;
                break;
            }
            _ = ping_tick.tick() => {
                let now_ms = keepalive_base.elapsed().as_millis() as u64;
                let last_ms = last_recv.load(Ordering::Acquire);
                let idle = Duration::from_millis(now_ms.saturating_sub(last_ms));
                if idle >= keepalive.idle_timeout {
                    tracing::info!(
                        conn = conn_id,
                        idle_ms = idle.as_millis() as u64,
                        "write_task: keepalive idle_timeout 초과(half-open 추정) — 종료"
                    );
                    let _ = sink_half.close().await;
                    break;
                }
                if let Err(e) = sink_half.send(Message::Ping(Vec::new().into())).await {
                    tracing::debug!(conn = conn_id, "write_task keepalive Ping 송신 실패 — 종료: {e}");
                    break;
                }
            }
            recv = conn_rx.recv() => {
                let Some(out) = recv else {
                    break;
                };
                let msg = match out {
                    Frame::Text(s) => Message::Text(s.into()),
                    Frame::Binary(b) => Message::Binary(b.into()),
                    Frame::Close(reason) => {
                        tracing::info!(conn = conn_id, %reason, "write_task: close 신호 — 종료");
                        let _ = sink_half.close().await;
                        break;
                    }
                };
                if let Err(e) = sink_half.send(msg).await {
                    tracing::debug!(conn = conn_id, "write_task send 실패 — 종료: {e}");
                    break;
                }
            }
        }
    }
    tracing::debug!(conn = conn_id, "write_task 루프 종료");
}

// ── read_task + dispatch_task (two layers split by the inbound queue) ─────────────────────────────

/// The unit the read loop decodes and hands to the inbound queue.
///
/// ★Unlike the outgoing [`Frame`], there is no Close★: an outgoing Close is a marker the upper layer puts in the queue, but on the incoming
///   side the close decision is made not by frame **content** but by the handler's answer (`ConnFlow::Close`) — so this
///   vocabulary has no such shape.
/// ★The payload is **not copied**★: the payload type tungstenite gave is moved as is (both are refcount
///   handles, so moving is cheap). Converting to `String`/`Vec<u8>` would create a copy per frame of **the size the client chose** —
///   the same axis of reason as the old `on_binary` lending the payload.
enum Inbound {
    Text(Utf8Bytes),
    Binary(Bytes),
}

impl Inbound {
    /// The payload bytes this frame holds in the lane — the unit of [`CONN_RX_MAX_BYTES`] accounting.
    ///
    /// ★It is the **payload** length, not the size of the handle itself★: both are refcount handles, so `size_of` is the same few tens of
    ///   bytes, and what actually bites memory is the buffer behind it.
    fn payload_len(&self) -> usize {
        match self {
            Inbound::Text(t) => t.len(),
            Inbound::Binary(b) => b.len(),
        }
    }
}

/// The payload bytes the arrival-order lane currently holds — the producer (`read_task`) adds and the consumer
/// (`dispatch_task`) subtracts. One per connection.
///
/// ★That the subtraction happens **after the handler call** is intended★: subtracting right on taking it out would count a frame that is 「꺼냈지만 아직 처리 중인」 (taken out but still being processed) as
///   0, so the budget would actually double (the same-axis mistake = the in-flight accounting of `input_queue`).
type RxBytes = Arc<std::sync::atomic::AtomicUsize>;

/// ★Why stream is generic★: to have an isolation harness that runs this loop on a synthetic frame sequence without a socket
/// (ADR-0129). The production path is monomorphized only to the WS stream half.
///
/// ★It does **not wait for the handler's work**★: it hands the decoded frame to the inbound queue and immediately
/// reads the next frame. In the old shape (awaiting `on_text` to completion here), the processing time of one command was the stall time of that connection's
/// **whole** — one agent activation kept even input · kill bound for other agents from being taken out of the socket.
/// Processing is borne by [`dispatch_task`] in arrival order.
// ADR-0206
#[allow(clippy::too_many_arguments)]
async fn read_task<S>(
    mut incoming: S,
    inbound_tx: mpsc::Sender<Inbound>,
    bypass_tx: mpsc::Sender<Inbound>,
    rx_bytes: RxBytes,
    frames: Arc<dyn FrameSink>,
    handler: Arc<dyn ConnectionHandler>,
    conn_id: ConnId,
    keepalive_base: tokio::time::Instant,
    last_recv: Arc<AtomicU64>,
) where
    S: Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin + Send,
{
    while let Some(item) = incoming.next().await {
        let msg = match item {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!(conn = conn_id, "read_task 수신 오류 — 종료: {e}");
                break;
            }
        };
        // ★That the keepalive (A) update is **before dispatch** stays as it was★: this line was before the handler
        //   call in the old shape too, so moving processing to another task does not change the keepalive decision.
        //   tungstenite raises a Pong as Message::Pong, so the reply to an active Ping is also caught here.
        last_recv.store(
            keepalive_base.elapsed().as_millis() as u64,
            Ordering::Release,
        );
        let queued = match msg {
            Message::Text(text) => Inbound::Text(text),
            Message::Binary(payload) => Inbound::Binary(payload),
            // Ping/Pong are auto-answered by tungstenite (internally, not by write_task). Ignored here.
            Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(_) => {
                tracing::debug!(conn = conn_id, "Close frame 수신 — 종료");
                break;
            }
            Message::Frame(_) => continue,
        };
        // ★The **byte budget can be hit before the entries**★ — the decision uses the fill level before adding (an empty lane
        //   accepts one frame regardless of size). Source of truth for the basis = [`CONN_RX_MAX_BYTES`].
        let over_budget = rx_bytes.load(Ordering::Acquire) >= CONN_RX_MAX_BYTES;
        // ★It is `try_send` — waiting here would mean **not even taking the frames behind it out of the socket**★ (source of truth for the basis =
        //   [`CONN_RX_CAP`]). If there is room, this is all (normal path cost 0), and only when there is none does it go below.
        let full = if over_budget {
            queued
        } else {
            let len = queued.payload_len();
            match inbound_tx.try_send(queued) {
                Ok(()) => {
                    rx_bytes.fetch_add(len, Ordering::AcqRel);
                    continue;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    tracing::debug!(conn = conn_id, "수신 큐 소비자 없음 — read_task 종료");
                    break;
                }
                // **Gets back** the frame that could not be put in — only with this can a disposition be chosen without silent loss.
                Err(mpsc::error::TrySendError::Full(item)) => item,
            }
        };

        // ★The lane is full — the disposition of this one frame is decided by the upper layer★ (only the upper layer knows the vocabulary).
        //   Binary is not asked: in this protocol a client→daemon binary is itself a violation and the upper layer closes
        //   the connection anyway, and lane order is not at stake in that decision.
        let disposition = match &full {
            Inbound::Text(text) => handler.on_inbound_saturated(conn_id, text.as_str(), &frames),
            Inbound::Binary(_) => Saturated::Bypass,
        };
        match disposition {
            Saturated::Refused => {
                // The upper layer sent a reply — discard it and keep reading. 「계속 읽는다」 (keep reading) is the reason this branch exists.
                tracing::debug!(
                    conn = conn_id,
                    "수신 줄 포화 — 위층이 거절했다(계속 읽는다)"
                );
            }
            // ★It could not even send a refusal — the basis of 「버리고 계속 읽는다」 (discard and keep reading) is gone, so end★ (source of truth for the basis =
            //   [`Saturated::Unanswered`]). If only reading ends, the arrival-order lane finishes processing what it already accepted and
            //   ends by itself, and `handle_connection` catches that end and cleans up.
            Saturated::Unanswered => {
                tracing::warn!(
                    conn = conn_id,
                    "수신 줄 포화 — 거절 답장조차 큐에 못 넣었다(송신 큐도 포화) — 읽기를 끝낸다"
                );
                break;
            }
            // ★Waiting is fine here★ — the basis is [`CONN_BYPASS_CAP`].
            Saturated::Bypass => {
                if bypass_tx.send(full).await.is_err() {
                    tracing::debug!(conn = conn_id, "우선 줄 소비자 없음 — read_task 종료");
                    break;
                }
            }
        }
    }
    tracing::debug!(conn = conn_id, "read_task 루프 종료");
}

/// The **single** consumer of the inbound queue — raises one connection's frames to the handler in arrival order, without overlapping.
///
/// ★This serialization is the **real thing** behind the upper layer's order invariants★: 「같은 에이전트로 가는 입력끼리」 (inputs bound for the same agent) · 「입력 lease
/// 획득과 그 뒤 입력」 (input lease acquisition and the input after it) · 「같은 연결의 구독/해지」 (subscribe/unsubscribe on the same connection) · 「명령 명부 등록과 그 차분」 (command registry registration and its diff) all
/// hang on arrival order, and what keeps it is only the property that this task is one and awaits only one at a time
/// (source of truth for the contract = `frame_port::ConnectionHandler`). **Starting a second consumer or scattering here with spawn
/// breaks all four at once** — the decision to peel a long-running command off the lane is the upper layer's job.
async fn dispatch_task(
    mut inbound_rx: mpsc::Receiver<Inbound>,
    rx_bytes: RxBytes,
    frames: Arc<dyn FrameSink>,
    handler: Arc<dyn ConnectionHandler>,
    conn_id: ConnId,
) {
    while let Some(item) = inbound_rx.recv().await {
        let len = item.payload_len();
        let flow = match item {
            Inbound::Text(text) => handler.on_text(conn_id, text.as_str(), &frames).await,
            // The payload is **lent** — the rejection path is the only consumer, so no copy is made (letting it allocate the size
            //   the client chose would be wasted allocation up to the maximum frame size after authentication).
            Inbound::Binary(payload) => handler.on_binary(conn_id, payload.as_ref(), &frames).await,
        };
        // ★Subtract **after** the handler returns★ — counting the one being processed as 0 doubles the budget
        //   ([`RxBytes`]). If the handler panics it never reaches this line, but the production daemon ends the whole process in the hook
        //   (ADR-0292), and in a hookless test build this task itself dies and the connection is cleaned up wholesale, so there is nowhere
        //   for a leaked budget to remain.
        rx_bytes.fetch_sub(len, Ordering::AcqRel);
        if flow == ConnFlow::Close {
            tracing::debug!(conn = conn_id, "핸들러가 Close — dispatch_task 종료");
            break;
        }
    }
    tracing::debug!(conn = conn_id, "dispatch_task 루프 종료");
}

/// The consumer of frames that skipped the arrival-order lane ([`Saturated::Bypass`]).
///
/// ★★It must be a **separate task from** [`dispatch_task`] — it cannot be merged by `select!` in the same task★★: the situation in which this lane
/// exists is precisely one where **that side's consumer is blocked**. A blocked task cannot poll anything, so
/// with a priority arrangement inside one task, this frame would never run.
///
/// ★So the two lanes **run overlapped**★ — the only exception to the port contract's 「겹치지 않는다」 (they do not overlap), and the one who
/// chose that exception is the upper layer itself (`on_inbound_saturated`). It means that what runs here and what is blocked there may
/// touch the same target, so the responsibility of choosing only frames that are allowed to do so also lies with the upper layer.
///
/// ★Route `Close` as an in-queue marker★: this task is **not** a select target of `handle_connection` (if it were,
/// the moment reading ends and this lane closes, teardown would start, and the arrival-order lane would lose the chance to drain what is left).
/// So the intent to end is sent on the write lane as `Frame::Close` — it closes after the frames put in earlier go out first.
async fn bypass_task(
    mut bypass_rx: mpsc::Receiver<Inbound>,
    frames: Arc<dyn FrameSink>,
    handler: Arc<dyn ConnectionHandler>,
    conn_id: ConnId,
) {
    while let Some(item) = bypass_rx.recv().await {
        let flow = match item {
            Inbound::Text(text) => handler.on_text(conn_id, text.as_str(), &frames).await,
            Inbound::Binary(payload) => handler.on_binary(conn_id, payload.as_ref(), &frames).await,
        };
        if flow == ConnFlow::Close {
            tracing::debug!(
                conn = conn_id,
                "우선 줄에서 Close — write 줄로 종료를 넘긴다"
            );
            let _ = frames.try_send(Frame::Close("bypass lane requested close".into()));
            break;
        }
    }
    tracing::debug!(conn = conn_id, "bypass_task 루프 종료");
}

/// **Reap** the priority lane — not with `abort()`, but by draining what is left and waiting for it to end by itself.
///
/// ★Why abort was a defect★: a frame that entered the priority lane is one the upper layer decided 「이건 굶으면 안 된다」 (this must not starve) and
///   **accepted** (`on_inbound_saturated` → [`Saturated::Bypass`]). But when the arrival-order lane ended first
///   (the common shape where a peer sends one command and closes right away), this task was aborted on the spot, and
///   **a frame that was accepted and never polled simply vanished.** This is the branch where the reason this lane was made collapses entirely —
///   the arrival-order lane does not have the same hole (when its sender is dropped it drains what is left and ends).
///
/// ★The caller's obligation = **cut `read_task` first**★: this lane's only sender is there, and if it is
///   not dropped, `recv()` never gets `None` and this wait uses up the whole grace period.
/// ★Past the grace period it falls back to the old disposition★ — so this function has **no downside**: if it goes well it finishes
///   what was accepted, and if not it aborts exactly as before. ★Lending the handle to `timeout` **instead of passing it
///   is intended**★: passing it would drop the handle on timeout, so the task would be **detached** rather than aborted, and that is
///   exactly the leak that `handle_connection` blocks with the explicit abort.
async fn finish_bypass_lane(mut handle: tokio::task::JoinHandle<()>, conn_id: ConnId) {
    match tokio::time::timeout(BYPASS_DRAIN_GRACE, &mut handle).await {
        Ok(_) => {}
        Err(_) => {
            tracing::warn!(
                conn = conn_id,
                grace_ms = BYPASS_DRAIN_GRACE.as_millis() as u64,
                "우선 줄이 유예 안에 안 비워졌다 — abort(받아 둔 것이 남아 있으면 유실된다)"
            );
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    // ── 2. Token constant-time comparison correctness ──────────────────────────────────────────
    #[test]
    fn constant_time_eq_correctness() {
        let a = "a".repeat(64);
        assert!(constant_time_eq(&a, &"a".repeat(64)), "동일 토큰은 true");
        assert!(!constant_time_eq(&a, &"b".repeat(64)), "다른 토큰은 false");
        assert!(!constant_time_eq(&a, &"a".repeat(63)));
        assert!(!constant_time_eq(&a, &"a".repeat(65)));
        let mut almost = "a".repeat(64);
        almost.replace_range(63..64, "b");
        assert!(!constant_time_eq(&a, &almost));
        assert!(constant_time_eq("", ""));
    }

    // ── 2b. The auth parse-failure log does not carry the payload ─────────────────────
    #[test]
    fn auth_parse_failure_log_fields_do_not_carry_the_payload() {
        let token = "deadbeef".repeat(8); // Same 64 chars as production
        let e = serde_json::from_str::<AuthFrame>(&format!(r#"{{"Auth":"{token}"}}"#)).unwrap_err();
        assert!(
            e.to_string().contains(&token),
            "Display 가 토큰을 싣지 않으면 이 테스트가 지키는 게 없다: {e}"
        );
        let logged = format!("{:?} {} {}", e.classify(), e.line(), e.column());
        assert!(
            !logged.contains(&token),
            "로깅 필드에 토큰이 실렸다: {logged}"
        );
    }

    // ── 3. Frame mapping (Text/Binary/Close → Message) ───────────────────────────
    // Directly verifies the same mapping as write_task's conversion logic (without a real WS).
    #[test]
    fn frame_maps_to_message() {
        let t = Frame::Text("hi".into());
        let b = Frame::Binary(vec![1, 2, 3]);
        let c = Frame::Close("bye".into());

        let to_msg = |o: Frame| -> Message {
            match o {
                Frame::Text(s) => Message::Text(s.into()),
                Frame::Binary(b) => Message::Binary(b.into()),
                Frame::Close(_) => Message::Close(None),
            }
        };
        assert!(matches!(to_msg(t), Message::Text(_)));
        assert!(matches!(to_msg(b), Message::Binary(_)));
        assert!(matches!(to_msg(c), Message::Close(_)));
    }

    // ── 4. ConnFrameSink: try_send rings the out-of-band close_signal on saturation ──
    #[tokio::test]
    async fn conn_frame_sink_notifies_close_signal_when_full() {
        let (tx, mut rx) = mpsc::channel::<Frame>(1);
        let close_signal = Arc::new(Notify::new());
        let sink = ConnFrameSink::new(tx, close_signal.clone());

        sink.try_send(Frame::Text("first".into()))
            .expect("빈 큐엔 들어간다");
        assert!(
            sink.try_send(Frame::Text("second".into())).is_err(),
            "full 이면 FrameError"
        );

        tokio::time::timeout(Duration::from_millis(200), close_signal.notified())
            .await
            .expect("close_signal 이 full 에서도 발동해야 함");

        assert!(matches!(rx.recv().await.unwrap(), Frame::Text(_)));
    }

    // ── 4b. ConnFrameSink: send (backpressure) does not ring close_signal ──
    #[tokio::test]
    async fn conn_frame_sink_send_does_not_signal_close() {
        let (tx, mut rx) = mpsc::channel::<Frame>(1);
        let close_signal = Arc::new(Notify::new());
        let sink = ConnFrameSink::new(tx, close_signal.clone());

        sink.try_send(Frame::Text("first".into())).expect("한 칸");

        // ★**Positive observation** that the saturation path was taken★: poll the future once directly to confirm Pending. Without
        //   this, in a `spawn` + immediate `recv()` combination the send could be polled for the first time **only after room opens**
        //   (spawn does not guarantee polling, and recv on a non-empty channel is Ready without yielding), so it could pass without ever taking
        //   the saturation branch that is the very thing under verification (measured: that form missed the regression).
        let mut pending = Box::pin(sink.send(Frame::Text("second".into())));
        assert!(
            futures_util::poll!(pending.as_mut()).is_pending(),
            "가득 찬 큐에서 send 는 반드시 park 해야(포화 경로 미실행이면 이 테스트가 무의미하다)"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), close_signal.notified())
                .await
                .is_err(),
            "send 는 포화를 기다릴 뿐 종료 신호를 울리지 않는다"
        );

        assert!(matches!(rx.recv().await.unwrap(), Frame::Text(_)));
        pending.await.expect("자리가 나면 backpressure 가 풀린다");
    }

    // ── 4c. ConnFrameSink: the three frame kinds ride the single writer queue in FIFO as they are ──
    #[tokio::test]
    async fn conn_frame_sink_maps_frames_to_queue_in_order() {
        let (tx, mut rx) = mpsc::channel::<Frame>(8);
        let sink = ConnFrameSink::new(tx, Arc::new(Notify::new()));
        sink.send(Frame::Text("hi".into())).await.expect("text ok");
        sink.try_send(Frame::Binary(vec![1, 2, 3]))
            .expect("binary ok");
        sink.send(Frame::Close("bye".into()))
            .await
            .expect("close ok");

        assert!(matches!(rx.recv().await.unwrap(), Frame::Text(_)));
        assert!(matches!(rx.recv().await.unwrap(), Frame::Binary(_)));
        match rx.recv().await.unwrap() {
            Frame::Close(r) => assert_eq!(r, "bye"),
            other => panic!("Close 여야 함: {other:?}"),
        }
    }

    // ── 4d. ConnRegistry: all-connection fan-out — one saturated connection does not block the rest (ADR-0129) ──
    #[test]
    fn broadcast_text_copies_the_text_to_every_connection_that_can_take_it() {
        // ★The reason for repeating = to redraw the HashMap iteration order every round (not padding)★: `broadcast_text` iterates the
        //   registry map and takes a Vec, so the visiting order is the delivery order.
        //   - **Correct code passes regardless of order** → this loop cannot create false positives (flakes). What repetition
        //     changes is only the **detection power**.
        //   - The regression to catch = "abort the fan-out at the first try_send failure". That regression survives a round in which the saturated connection is visited **last**,
        //     because the healthy connections have already all received (the same family as "passing without ever taking the saturation path" recorded in S18.17).
        //     So each round redraws the order with a **new registry**.
        //     ※ Reusing the same registry fixes the order and makes repetition meaningless (so it is created **inside** the loop).
        // ★Detection power is empirical, not a proof (stated honestly)★: std says only that `RandomState` uses a different seed per instance, and
        //   **guarantees neither order independence between instances nor any particular distribution**. So there is no basis to write a calculation like "with K rounds the miss
        //   probability is 2^-K" here — that would promote an observation to a guarantee.
        //   ★Measured (2026-08-04 · this form = 1 saturated + 2 healthy)★: planting the regression above, **all 10 attempts** caught it.
        //   The round that caught it and the starved connection (ok0/ok1) differed every run — evidence that the order is actually
        //   redrawn every round. **A measurement, not a guarantee.**
        // ★If you want deterministic detection, you must control the iteration order★ = swap the `ConnRegistry` map type (an ordered map, etc.).
        //   It is outside the scope of the move slice (ADR-0129), so it is not done — the day this needs to be a **hard guarantee**, that
        //   swap is the proper way, and until then K is only a detection-power knob (not a threshold-tuning target — ADR-0038).
        // ★Why two healthy connections★: it covers the arrangement where the saturated one comes in the **middle** too. For the regression to survive
        //   a round the saturated connection must come **last**, and with 1 healthy connection "last" is one of two
        //   positions, with 2 it is one of three — so the room for survival per round narrows (with no distribution guarantee,
        //   this too is a **position-count argument**, not a probability calculation).
        const K: usize = 20;
        const PAYLOAD: &str = "opaque-fanout-payload";
        for round in 0..K {
            let registry = ConnRegistry::new();
            // Uses the production registration path as is (alloc_id + register) — it is the same module, so no test-only seam is needed.
            let register = |tx: mpsc::Sender<Frame>| {
                let id = registry.alloc_id();
                registry.register(id, tx);
            };
            let (full_tx, mut full_rx) = mpsc::channel::<Frame>(1);
            full_tx
                .try_send(Frame::Text("선점".into()))
                .expect("cap 1 을 미리 채운다");
            register(full_tx);
            let mut oks: Vec<mpsc::Receiver<Frame>> = (0..2)
                .map(|_| {
                    let (ok_tx, ok_rx) = mpsc::channel::<Frame>(8);
                    register(ok_tx);
                    ok_rx
                })
                .collect();

            registry.broadcast_text(PAYLOAD.to_string());

            assert!(
                matches!(full_rx.try_recv(), Ok(Frame::Text(s)) if s == "선점"),
                "round {round}: 포화 연결엔 선점 프레임만 있어야"
            );
            assert!(
                full_rx.try_recv().is_err(),
                "round {round}: 포화 연결은 이번 건을 못 받는다"
            );
            // ★Put the round · connection number in the message at each branch★: the regression we catch ("abort at the first saturation") shows up as "received
            //   nothing", so a generic message does not show which round was hit.
            for (n, ok_rx) in oks.iter_mut().enumerate() {
                match ok_rx.try_recv() {
                    Ok(Frame::Text(s)) => assert_eq!(
                        s, PAYLOAD,
                        "round {round}/ok{n}: 넘겨받은 text 를 그대로 복제해야"
                    ),
                    other => panic!(
                        "round {round}/ok{n}: 멀쩡한 연결이 프레임을 못 받았다(fanout 이 첫 실패에서 멈춘 회귀): {other:?}"
                    ),
                }
                assert!(
                    ok_rx.try_recv().is_err(),
                    "round {round}/ok{n}: 팬아웃 1회는 연결당 정확히 1프레임"
                );
            }
        }
    }

    // ── 10. (apply 4-1) OriginCheck::on_request branches — verifying the reject/allow branches that were unprotected ──────
    //    A pure header check, so no in-process server is needed. Build the Request directly and call the callback.
    fn run_origin_check(origin: Option<&str>) -> Result<(), ()> {
        use tokio_tungstenite::tungstenite::http::Request as HttpRequest;
        let mut builder = HttpRequest::builder().uri("/");
        if let Some(o) = origin {
            builder = builder.header("origin", o);
        }
        let request = builder.body(()).unwrap();
        // Response is a dummy that the callback passes through.
        let response = tokio_tungstenite::tungstenite::http::Response::builder()
            .body(())
            .unwrap();
        OriginCheck
            .on_request(&request, response)
            .map(|_| ())
            .map_err(|_| ())
    }

    #[test]
    fn origin_check_allows_listed_origin() {
        assert!(run_origin_check(Some("tauri://localhost")).is_ok());
        assert!(run_origin_check(Some("http://localhost:1420")).is_ok());
    }

    #[test]
    fn origin_check_rejects_unlisted_origin() {
        assert!(run_origin_check(Some("http://evil.example.com")).is_err());
    }

    #[test]
    fn origin_check_allows_missing_origin() {
        assert!(run_origin_check(None).is_ok());
    }

    // ── 11. Frame port seam — an isolation harness that runs without a socket (ADR-0129) ──────────────────
    //    Reproduces connection lifetime with a fake ConnectionHandler + a fake FrameSink. Without a TcpStream,
    //    this verification stays alive even if the network layer is split off into a separate crate.

    #[derive(Debug, PartialEq, Eq)]
    enum SeenFrame {
        Text(String),
        Binary(Vec<u8>),
        Close(String),
    }

    #[derive(Default)]
    struct FakeFrameSink {
        frames: Mutex<Vec<SeenFrame>>,
        /// When on, every `try_send`/`send` fails — a knob that makes 「송신 큐도 포화」 (the outbound queue is saturated too) without a time limit.
        refuse: AtomicBool,
    }

    impl FakeFrameSink {
        /// An outlet that accepts no frame at all (= a stand-in for a connection whose outbound queue is saturated or closed).
        fn refusing() -> Self {
            Self {
                refuse: AtomicBool::new(true),
                ..Self::default()
            }
        }

        fn frames(&self) -> Vec<String> {
            self.frames
                .lock()
                .unwrap()
                .iter()
                .map(|f| match f {
                    SeenFrame::Text(s) => format!("text:{s}"),
                    SeenFrame::Binary(b) => format!("bin:{}", b.len()),
                    SeenFrame::Close(r) => format!("close:{r}"),
                })
                .collect()
        }
    }

    impl FrameSink for FakeFrameSink {
        fn try_send(&self, frame: Frame) -> Result<(), FrameError> {
            if self.refuse.load(Ordering::Acquire) {
                return Err(FrameError);
            }
            let seen = match frame {
                Frame::Text(s) => SeenFrame::Text(s),
                Frame::Binary(b) => SeenFrame::Binary(b),
                Frame::Close(r) => SeenFrame::Close(r),
            };
            self.frames.lock().unwrap().push(seen);
            Ok(())
        }
        fn send(&self, frame: Frame) -> BoxFuture<'_, Result<(), FrameError>> {
            Box::pin(async move { self.try_send(frame) })
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum HandlerCall {
        Connect(ConnId),
        Text(ConnId, String),
        /// Only the payload length — the content is not this seam's concern.
        Binary(ConnId, usize),
        Disconnect(ConnId),
    }

    struct FakeHandler {
        calls: Mutex<Vec<HandlerCall>>,
        /// On receiving this text it returns `ConnFlow::Close` (for verifying exit from the receive loop).
        close_on: Option<String>,
        /// On receiving this text it puts `Frame::Close` in the queue and returns **Continue** — it ends the connection
        /// on the **write_task side** rather than the read side, taking the select! "abort read" branch.
        close_queue_on: Option<String>,
        /// After `on_connect` puts in the greeting frame, it waits here. During that window the test observes "nothing has gone out to the
        /// socket yet" (= writer not started) and "no frame has been processed yet" (= reader not
        /// started).
        connect_gate: Option<Arc<Notify>>,
        /// Signal that 1 `on_text` has finished processing — so the test can proceed regardless of the client close timing.
        text_seen: Arc<Notify>,
        /// On receiving this text it waits **inside** `on_text` until [`Self::slow_gate`] opens —
        /// it creates a window that measures 「핸들러가 한 명령에 붙들려 있는 동안 읽기 루프가 앞서 나가는가」 (does the read loop get ahead while the handler is held by one command).
        /// ★A device to avoid relying on timing assumptions★: imitating it with sleep closes the window on a slow runner and gives
        ///   a false positive. The gate stays closed **forever** until the test opens it.
        slow_on: Option<String>,
        slow_gate: Arc<Notify>,
        /// The **priority lane version** of [`Self::slow_on`]: on receiving this text it waits inside `on_text`
        /// until [`Self::bypass_gate`] opens. Only by locking it **separately** from the arrival-order lane can the state 「도착순은 끝났는데 우선 줄은 아직
        /// 한 건에 붙들려 있다」 (arrival order is finished but the priority lane is still held by one item) be created (that state is the stage of the abort defect).
        bypass_gate_on: Option<String>,
        bypass_gate: Arc<Notify>,
        /// A weak reference to the frame outlet that `on_connect` received. Only read_task holds a strong reference, so if it can still be
        /// upgraded after the connection has ended, that task was not aborted but **detached**.
        /// ★This fake **must never keep a strong reference to the frame outlet**★ — if even one `Arc` is left in a field, upgrade
        /// always succeeds and the polling loop of `the_losing_task_is_aborted_not_detached` runs to the end with
        /// `released == false`, so **that test always fails on correct code** (not silent inability to detect but a loud false positive — so
        /// it is hard to trace the cause back to this field).
        frames_weak: Mutex<Option<std::sync::Weak<dyn FrameSink>>>,
        /// Whether this connection was still in the fan-out registry at the time of `on_disconnect` (observing the cleanup order).
        registry: Option<ConnRegistry>,
        registered_at_disconnect: Mutex<Option<bool>>,
    }

    impl FakeHandler {
        fn new(close_on: Option<&str>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                close_on: close_on.map(|s| s.to_string()),
                close_queue_on: None,
                connect_gate: None,
                text_seen: Arc::new(Notify::new()),
                slow_on: None,
                slow_gate: Arc::new(Notify::new()),
                bypass_gate_on: None,
                bypass_gate: Arc::new(Notify::new()),
                frames_weak: Mutex::new(None),
                registry: None,
                registered_at_disconnect: Mutex::new(None),
            }
        }

        /// A variant held by one command — that `on_text` does not return until `slow_gate` is opened.
        fn blocking_on(slow_on: &str) -> Self {
            Self {
                slow_on: Some(slow_on.to_string()),
                ..Self::new(None)
            }
        }

        /// A variant that locks the two lanes **each** — the arrival-order lane with `slow_on`, the priority lane with `bypass_gate_on`.
        fn blocking_on_both(slow_on: &str, bypass_gate_on: &str) -> Self {
            Self {
                bypass_gate_on: Some(bypass_gate_on.to_string()),
                ..Self::blocking_on(slow_on)
            }
        }

        /// For order verification of `handle_connection` — peeks into the registry and catches on_connect with a gate.
        fn probing(registry: ConnRegistry, connect_gate: Arc<Notify>) -> Self {
            Self {
                registry: Some(registry),
                connect_gate: Some(connect_gate),
                ..Self::new(None)
            }
        }

        /// A variant that ends the connection on the write_task side (for verifying the loser abort).
        fn closing_via_writer(close_queue_on: &str) -> Self {
            Self {
                close_queue_on: Some(close_queue_on.to_string()),
                ..Self::new(None)
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|c| match c {
                    HandlerCall::Connect(id) => format!("connect:{id}"),
                    HandlerCall::Text(id, t) => format!("text:{id}:{t}"),
                    HandlerCall::Binary(id, n) => format!("binary:{id}:{n}"),
                    HandlerCall::Disconnect(id) => format!("disconnect:{id}"),
                })
                .collect()
        }

        fn registered_at_disconnect(&self) -> Option<bool> {
            *self.registered_at_disconnect.lock().unwrap()
        }

        /// Assertions `on_connect` uses at two points — in correct code, no frame can be processed
        /// before `on_connect` finishes (read_task does not exist yet).
        fn assert_nothing_processed_yet(&self, at: &str) {
            let calls = self.calls.lock().unwrap();
            assert!(
                calls.is_empty(),
                "on_connect({at}) 보다 먼저 처리된 프레임이 있다 — read_task 가 앞서 스폰됐다: {calls:?}"
            );
        }

        fn frames_still_held(&self) -> bool {
            self.frames_weak
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|w| w.upgrade())
                .is_some()
        }
    }

    impl ConnectionHandler for FakeHandler {
        fn on_connect<'a>(
            &'a self,
            conn_id: ConnId,
            frames: &'a Arc<dyn FrameSink>,
        ) -> BoxFuture<'a, ()> {
            Box::pin(async move {
                *self.frames_weak.lock().unwrap() = Some(Arc::downgrade(frames));
                // ★Assertion **before** the gate (decided by program order)★: in correct code read_task has not even been spawned yet,
                //   so there can be no processed frame. True regardless of the scheduler.
                self.assert_nothing_processed_yet("게이트 진입 전");
                // Put the greeting **before the gate** — if the writer were already up, this frame would go out to the socket
                //   during the gate wait, and the test would catch it.
                let _ = frames.send(Frame::Text("greeting".into())).await;
                if let Some(gate) = &self.connect_gate {
                    gate.notified().await;
                }
                // ★Assertion **after** the gate★: during the window until the gate opens (in which the test has fed commands in advance), it catches whether
                //   a wrongly spawned read_task processed that command.
                self.assert_nothing_processed_yet("게이트 통과 후");
                self.calls
                    .lock()
                    .unwrap()
                    .push(HandlerCall::Connect(conn_id));
            })
        }

        fn on_text<'a>(
            &'a self,
            conn_id: ConnId,
            text: &'a str,
            frames: &'a Arc<dyn FrameSink>,
        ) -> BoxFuture<'a, ConnFlow> {
            Box::pin(async move {
                let close = self.close_on.as_deref() == Some(text);
                let close_via_writer = self.close_queue_on.as_deref() == Some(text);
                self.calls
                    .lock()
                    .unwrap()
                    .push(HandlerCall::Text(conn_id, text.to_string()));
                self.text_seen.notify_one();
                if self.slow_on.as_deref() == Some(text) {
                    self.slow_gate.notified().await;
                }
                if self.bypass_gate_on.as_deref() == Some(text) {
                    self.bypass_gate.notified().await;
                }
                if close_via_writer {
                    let _ = frames.try_send(Frame::Close("테스트: writer 가 먼저 끝난다".into()));
                    return ConnFlow::Continue;
                }
                if close {
                    ConnFlow::Close
                } else {
                    ConnFlow::Continue
                }
            })
        }

        fn on_binary<'a>(
            &'a self,
            conn_id: ConnId,
            payload: &'a [u8],
            _frames: &'a Arc<dyn FrameSink>,
        ) -> BoxFuture<'a, ConnFlow> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .unwrap()
                    .push(HandlerCall::Binary(conn_id, payload.len()));
                ConnFlow::Continue
            })
        }

        /// This fake makes only text starting with "bypass:" skip the lane. It refuses the rest, and **records**
        /// the refusal too to tell it apart from silent loss (this fake's reply, corresponding to obligation 2 of the port contract).
        ///
        /// ★It is the same shape as the production handler down to splitting into [`Saturated::Unanswered`] by the reply result★ —
        ///   if it discarded the result and always returned `Refused`, this fake could not even **express** that branch.
        fn on_inbound_saturated(
            &self,
            _conn_id: ConnId,
            text: &str,
            frames: &Arc<dyn FrameSink>,
        ) -> Saturated {
            if text.starts_with("bypass:") {
                return Saturated::Bypass;
            }
            match frames.try_send(Frame::Text(format!("refused:{text}"))) {
                Ok(()) => Saturated::Refused,
                Err(_) => Saturated::Unanswered,
            }
        }

        fn on_disconnect(&self, conn_id: ConnId) {
            if let Some(registry) = &self.registry {
                *self.registered_at_disconnect.lock().unwrap() = Some(registry.contains(conn_id));
            }
            self.calls
                .lock()
                .unwrap()
                .push(HandlerCall::Disconnect(conn_id));
        }
    }

    struct FakeFactory {
        handler: Arc<FakeHandler>,
    }

    impl ConnectionHandlerFactory for FakeFactory {
        fn handler_for(&self, _conn_id: ConnId) -> Arc<dyn ConnectionHandler> {
            self.handler.clone()
        }
        fn handshake_error_frame(&self, message: &str) -> Option<String> {
            Some(message.to_string())
        }
    }

    fn text_frame(s: &str) -> Result<Message, tokio_tungstenite::tungstenite::Error> {
        Ok(Message::Text(s.to_string().into()))
    }

    /// Runs the read layer and processing layer together **in the actual arrangement** (chained through the inbound queue) — a minimal
    /// reproduction of the wiring that `handle_connection` does, and returns after both finish.
    ///
    /// ★The property of an isolation harness that runs without a socket stays as it is★ (ADR-0129) — the only change is that
    ///   taking this seam now takes two tasks instead of one.
    async fn run_read_and_dispatch<S>(
        incoming: S,
        frames: Arc<dyn FrameSink>,
        handler: Arc<dyn ConnectionHandler>,
        conn_id: ConnId,
    ) where
        S: Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
            + Unpin
            + Send
            + 'static,
    {
        let (tx, rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
        let rx_bytes = new_rx_bytes();
        let (bypass_tx, bypass_rx) = mpsc::channel::<Inbound>(CONN_BYPASS_CAP);
        let read = tokio::spawn(read_task(
            incoming,
            tx,
            bypass_tx,
            rx_bytes.clone(),
            frames.clone(),
            handler.clone(),
            conn_id,
            tokio::time::Instant::now(),
            Arc::new(AtomicU64::new(0)),
        ));
        let bypass = tokio::spawn(bypass_task(
            bypass_rx,
            frames.clone(),
            handler.clone(),
            conn_id,
        ));
        dispatch_task(rx, rx_bytes, frames, handler, conn_id).await;
        // The same disposition as `handle_connection`'s "dispatch finished first" branch.
        read.abort();
        let _ = read.await;
        finish_bypass_lane(bypass, conn_id).await;
    }

    fn new_rx_bytes() -> RxBytes {
        Arc::new(std::sync::atomic::AtomicUsize::new(0))
    }

    #[tokio::test]
    async fn handler_sees_connect_then_frames_then_disconnect() {
        let fake_sink = Arc::new(FakeFrameSink::default());
        let frames: Arc<dyn FrameSink> = fake_sink.clone();
        let fake = Arc::new(FakeHandler::new(None));
        let handler: Arc<dyn ConnectionHandler> = fake.clone();

        handler.on_connect(7, &frames).await;
        run_read_and_dispatch(
            futures_util::stream::iter(vec![
                text_frame("cmd"),
                Ok(Message::Binary(vec![1, 2, 3].into())),
                Ok(Message::Close(None)),
                text_frame("after-close"),
            ]),
            frames.clone(),
            handler.clone(),
            7,
        )
        .await;
        handler.on_disconnect(7);

        assert_eq!(
            fake.calls(),
            vec![
                "connect:7",
                "text:7:cmd",
                "binary:7:3",
                "disconnect:7", // Frames after a Close frame are not consumed
            ]
        );
        assert_eq!(
            fake_sink.frames(),
            vec!["text:greeting"],
            "on_connect 가 넣은 프레임이 단일 출구로 나간다"
        );
    }

    /// ★The meaning is the same even after the close decision left the read loop★: when the processing layer receives `ConnFlow::Close`,
    /// it stops on the spot, and frames that were **already in the queue** behind it are not processed either. What was
    /// "no more reading from the socket" in the old shape is now "no more processing", and what ties it to connection end is
    /// the select branch of `handle_connection`.
    #[tokio::test]
    async fn close_flow_from_on_text_stops_the_dispatch_row() {
        let frames: Arc<dyn FrameSink> = Arc::new(FakeFrameSink::default());
        let fake = Arc::new(FakeHandler::new(Some("stop")));
        let handler: Arc<dyn ConnectionHandler> = fake.clone();

        run_read_and_dispatch(
            futures_util::stream::iter(vec![
                text_frame("go"),
                text_frame("stop"),
                text_frame("unreachable"),
            ]),
            frames,
            handler,
            3,
        )
        .await;

        assert_eq!(
            fake.calls(),
            vec!["text:3:go", "text:3:stop"],
            "ConnFlow::Close 면 그 자리에서 처리를 멈춘다(뒤엣것은 큐에 있어도 안 돈다)"
        );
    }

    /// ★★Regression net for the defect this change fixed★★ — the old read loop **awaited the handler call to completion** before
    /// reading the next frame. So while one agent activation (typically 2s · backstop 15s) ran, **input · kill · another
    /// activation bound for other agents** sent by the same client were not even taken out of the socket
    /// (socket head-of-line blocking).
    ///
    /// ★It measures two things at once★: ① while the handler is held by one command, the rest of the frames are **all** read
    /// ② and yet processing is still **one at a time in arrival order** (commands whose order carries meaning hang on this single property — the
    /// `frame_port::ConnectionHandler` contract). Measuring only ① misses a regression that "got faster by dropping order", and measuring
    /// only ② stays green even if the original defect is still there.
    ///
    /// ★Measure with a gate, not with sleep★: it removes the false positives that occur when the window closes on a slow runner.
    #[tokio::test]
    async fn the_read_loop_runs_ahead_while_a_handler_call_is_still_running() {
        let frames: Arc<dyn FrameSink> = Arc::new(FakeFrameSink::default());
        let fake = Arc::new(FakeHandler::blocking_on("slow"));
        let handler: Arc<dyn ConnectionHandler> = fake.clone();

        let yielded = Arc::new(AtomicU64::new(0));
        let counter = yielded.clone();
        let incoming = futures_util::stream::iter(vec![
            text_frame("slow"),
            text_frame("other-agent-input"),
            text_frame("kill"),
            Ok(Message::Binary(vec![9, 9].into())),
            text_frame("tail"),
        ])
        .inspect(move |_| {
            counter.fetch_add(1, Ordering::Release);
        });

        let (tx, rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
        let rx_bytes = new_rx_bytes();
        let (bypass_tx, _bypass_rx) = mpsc::channel::<Inbound>(CONN_BYPASS_CAP);
        let read = tokio::spawn(read_task(
            incoming,
            tx,
            bypass_tx,
            rx_bytes.clone(),
            frames.clone(),
            handler.clone(),
            5,
            tokio::time::Instant::now(),
            Arc::new(AtomicU64::new(0)),
        ));
        let dispatch = tokio::spawn(dispatch_task(rx, rx_bytes, frames, handler, 5));

        // Observe after confirming that the first command is held **inside** the handler.
        tokio::time::timeout(Duration::from_secs(5), fake.text_seen.notified())
            .await
            .expect("첫 명령이 핸들러에 닿아야");

        let mut all_read = false;
        for _ in 0..500 {
            if yielded.load(Ordering::Acquire) == 5 {
                all_read = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            all_read,
            "핸들러가 첫 명령에 붙들린 동안 나머지 프레임이 소켓에서 꺼내지지 않았다 — head-of-line blocking 회귀"
        );
        assert_eq!(
            fake.calls(),
            vec!["text:5:slow"],
            "읽기가 앞서 나가도 처리는 한 번에 하나다 — 앞 호출이 반환하기 전에 다음이 시작되면 안 된다"
        );

        fake.slow_gate.notify_one();
        tokio::time::timeout(Duration::from_secs(10), dispatch)
            .await
            .expect("게이트가 열리면 처리 행이 끝나야")
            .unwrap();
        let _ = read.await;

        assert_eq!(
            fake.calls(),
            vec![
                "text:5:slow",
                "text:5:other-agent-input",
                "text:5:kill",
                "binary:5:2",
                "text:5:tail",
            ],
            "큐를 거쳐도 처리 순서는 도착 순서 그대로다"
        );
    }

    /// ★★Regression net for the second hole the adversarial review found★★ — if the inbound lane **has no room for cancellation**, the moment the lane
    /// fills, a `Kill` starves behind it and the symptom this change meant to eliminate (「취소가 안 먹는다」 (cancel has no effect)) comes back by another
    /// path.
    ///
    /// ★The point is not 「우선 줄이 있다」 (there is a priority lane) but 「**읽기가 안 선다**」 (reading does not stop)★: if the read loop waits on a saturated lane,
    /// the `Kill` that comes after it is **not even taken out of the socket** (TCP is a stream). So this
    /// test places `Kill` as **a frame that arrives after the lane is full**, and slips in one more ordinary command before it
    /// to also check 「그 한 장을 처분하고 계속 읽었는가」 (did it dispose of that one and keep reading).
    ///
    /// ★It creates the state where the consumer is blocked★: the first frame is held at the gate, so the arrival-order lane is never empty —
    /// the same shape as the real incident (key input piles up behind a blocked PTY write).
    #[tokio::test]
    async fn a_cancel_still_gets_through_a_saturated_inbound_queue() {
        let sink = Arc::new(FakeFrameSink::default());
        let frames: Arc<dyn FrameSink> = sink.clone();
        let fake = Arc::new(FakeHandler::blocking_on("slow"));
        let handler: Arc<dyn ConnectionHandler> = fake.clone();

        // Fill the lane completely: 1 frame held at the gate + CONN_RX_CAP frames to fill the lane.
        let mut items = vec![text_frame("slow")];
        for i in 0..CONN_RX_CAP {
            items.push(text_frame(&format!("filler-{i}")));
        }
        // ★From here on is the main point of this test★ — two frames arriving while the lane has no room.
        items.push(text_frame("overflow-ordered"));
        items.push(text_frame("bypass:kill"));

        let (tx, rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
        let rx_bytes = new_rx_bytes();
        let (bypass_tx, bypass_rx) = mpsc::channel::<Inbound>(CONN_BYPASS_CAP);
        let read = tokio::spawn(read_task(
            futures_util::stream::iter(items),
            tx,
            bypass_tx,
            rx_bytes.clone(),
            frames.clone(),
            handler.clone(),
            11,
            tokio::time::Instant::now(),
            Arc::new(AtomicU64::new(0)),
        ));
        let bypass = tokio::spawn(bypass_task(bypass_rx, frames.clone(), handler.clone(), 11));
        let dispatch = tokio::spawn(dispatch_task(rx, rx_bytes, frames, handler, 11));

        // Check that the cancel is actually processed **while the arrival-order consumer is still held by the first command**.
        let mut cancelled = false;
        for _ in 0..500 {
            if fake.calls().iter().any(|c| c == "text:11:bypass:kill") {
                cancelled = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            cancelled,
            "줄이 가득 찬 동안 취소가 통과하지 못했다 — 취소 자리 없음 회귀: {:?}",
            fake.calls()
        );
        // ★What is measured here is **the set, not the order**★: the two lanes are different tasks, so which of "slow" and the cancel
        //   is recorded first is up to the scheduler, and asserting it gives false positives. What must hold is
        //   「줄에 든 것들은 아직 하나도 안 돌았다」 (none of the things in the lane has run yet) — that is, the only thing skipped is the cancel.
        let ran: Vec<String> = fake.calls();
        assert!(
            ran.iter()
                .all(|c| c == "text:11:slow" || c == "text:11:bypass:kill"),
            "취소 말고 다른 것이 줄을 건너뛰었다(또는 막힌 줄이 돌았다): {ran:?}"
        );
        // ★The overflowed ordinary command **does not vanish silently**★ — this fake's refusal reply is the evidence.
        assert!(
            sink.frames()
                .iter()
                .any(|f| f == "text:refused:overflow-ordered"),
            "넘친 명령이 답장 없이 버려졌다: {:?}",
            sink.frames()
        );

        fake.slow_gate.notify_one();
        let _ = tokio::time::timeout(Duration::from_secs(10), dispatch).await;
        read.abort();
        bypass.abort();
    }

    /// ★★Entries remain but **the bytes fill first**★★ — the regression net for [`CONN_RX_MAX_BYTES`].
    ///
    /// ★The regression it catches★: the shape that measured saturation by entry count only. Then this lane's real ceiling becomes
    ///   `CONN_RX_CAP × max_message_size` (= 64 × 64 MiB), and nobody had counted that product.
    /// ★Deterministic★: no sleep and no scheduler assumption — if the consumer is held at a gate the lane is never empty,
    ///   so the budget decision is fixed by the order in which frames are put in alone.
    /// ★Why **two** large frames★: the decision is 「더하기 전 물높이」 (the fill level before adding), so one frame arriving at an empty lane
    ///   always goes in regardless of size (that property is nailed down here too). What exceeds the budget is from the next frame on.
    #[tokio::test]
    async fn the_inbound_row_is_bounded_by_bytes_not_only_by_slots() {
        let sink = Arc::new(FakeFrameSink::default());
        let frames: Arc<dyn FrameSink> = sink.clone();
        let fake = Arc::new(FakeHandler::blocking_on("slow"));
        let handler: Arc<dyn ConnectionHandler> = fake.clone();

        // Two frames each one byte more than half the budget — two exceed the budget, and only three of the 64 entries are used.
        let big = "x".repeat(CONN_RX_MAX_BYTES / 2 + 1);
        let items = vec![
            text_frame("slow"),
            text_frame(&big),
            text_frame(&big),
            text_frame("after-budget"),
        ];

        let (tx, rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
        let rx_bytes = new_rx_bytes();
        let (bypass_tx, _bypass_rx) = mpsc::channel::<Inbound>(CONN_BYPASS_CAP);
        let read = tokio::spawn(read_task(
            futures_util::stream::iter(items),
            tx,
            bypass_tx,
            rx_bytes.clone(),
            frames.clone(),
            handler.clone(),
            21,
            tokio::time::Instant::now(),
            Arc::new(AtomicU64::new(0)),
        ));
        let dispatch = tokio::spawn(dispatch_task(rx, rx_bytes, frames, handler, 21));

        // The read loop finishes after scanning all the frames (it does not stop even on exceeding the budget — that is this arrangement's premise).
        tokio::time::timeout(Duration::from_secs(10), read)
            .await
            .expect("읽기 루프가 예산 초과에서 서면 안 된다")
            .unwrap();

        assert!(
            sink.frames()
                .iter()
                .any(|f| f == "text:refused:after-budget"),
            "칸이 남았다고 통과시켰다 — 바이트 예산이 안 걸렸다: {:?}",
            sink.frames()
        );
        // Confirm **by signal** that the consumer actually picked up the first frame, then assert below (no scheduler assumptions).
        tokio::time::timeout(Duration::from_secs(5), fake.text_seen.notified())
            .await
            .expect("소비자가 첫 장을 집어야");
        assert_eq!(
            fake.calls(),
            vec!["text:21:slow"],
            "소비자는 여전히 첫 장에 붙들려 있어야(줄이 비면 이 시험의 전제가 무너진다)"
        );

        fake.slow_gate.notify_one();
        let _ = tokio::time::timeout(Duration::from_secs(10), dispatch).await;
    }

    /// ★★If the refusal **cannot be sent as a reply**, that connection ends★★ — the regression net for [`Saturated::Unanswered`].
    ///
    /// ★The regression it catches★: the shape where the upper layer discarded the result of enqueuing the `Error` reply and always returned `Refused`. Then
    ///   the network layer trusts a reply that does not exist and discards the original command, and the sender gets **neither the command nor the refusal** and
    ///   waits until its own deadline.
    /// ★Observation = 「그 뒤 프레임을 소켓에서 꺼냈는가」 (did it take the frames after that out of the socket)★: measuring whether it ended by join does not
    ///   distinguish the two branches (both end soon). Counting **how many frames were pulled** from the stream tells them apart — if it ended, the later ones
    ///   are not pulled.
    /// ★Do not start the consumer at all★: if started, 「소비자가 첫 장을 꺼내 갔는가」 (did the consumer take the first frame) depends on the scheduler and the saturation moment
    ///   wobbles by one frame (we saw that wobble in measurement). With only the receiving end kept alive, the lane fills at exactly `CONN_RX_CAP` frames,
    ///   so the decision is deterministic — what this test measures is not consumer behavior but **the read layer's disposition**.
    #[tokio::test]
    async fn a_refusal_that_could_not_be_sent_ends_the_read_row() {
        // An outlet that accepts not a single frame = a state where the outbound queue is saturated too.
        let frames: Arc<dyn FrameSink> = Arc::new(FakeFrameSink::refusing());
        let handler: Arc<dyn ConnectionHandler> = Arc::new(FakeHandler::new(None));

        let mut items = Vec::new();
        for i in 0..CONN_RX_CAP {
            items.push(text_frame(&format!("filler-{i}")));
        }
        items.push(text_frame("unanswerable")); // That one frame where the lane is full and no reply can be sent either
        items.push(text_frame("tail-1"));
        items.push(text_frame("tail-2"));
        let total = items.len();
        let trigger_index = CONN_RX_CAP + 1; // fillers + that one frame

        let pulled = Arc::new(AtomicU64::new(0));
        let counter = pulled.clone();
        let incoming = futures_util::stream::iter(items).inspect(move |_| {
            counter.fetch_add(1, Ordering::Release);
        });

        // ★Keep the receiving end alive★ — if dropped, the first `try_send` is cut by Closed and this test would measure
        //   "no consumer" rather than saturation.
        let (tx, _rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
        let (bypass_tx, _bypass_rx) = mpsc::channel::<Inbound>(CONN_BYPASS_CAP);
        read_task(
            incoming,
            tx,
            bypass_tx,
            new_rx_bytes(),
            frames,
            handler,
            23,
            tokio::time::Instant::now(),
            Arc::new(AtomicU64::new(0)),
        )
        .await;

        assert_eq!(
            pulled.load(Ordering::Acquire) as usize,
            trigger_index,
            "답장 못 한 거절을 '거절했다'로 읽고 계속 읽었다 — 전체 {total} 장 중 {trigger_index} 장에서 멈췄어야",
        );
    }

    /// Set the initial value to an unreachable sentinel so "it was updated" can be decided without timing.
    async fn clock_updated_by(msg: Message) -> bool {
        let last_recv = Arc::new(AtomicU64::new(u64::MAX));

        // ★Keep the receiving end alive★: if dropped, `send` fails and the loop is cut at the first frame, so this decision
        //   would measure "is the queue alive" rather than "was it updated".
        let (tx, _rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
        let (bypass_tx, _bypass_rx) = mpsc::channel::<Inbound>(CONN_BYPASS_CAP);
        read_task(
            futures_util::stream::iter(vec![Ok(msg)]),
            tx,
            bypass_tx,
            new_rx_bytes(),
            Arc::new(FakeFrameSink::default()),
            Arc::new(FakeHandler::new(None)),
            1,
            tokio::time::Instant::now(),
            last_recv.clone(),
        )
        .await;

        last_recv.load(Ordering::Acquire) < u64::MAX
    }

    #[tokio::test]
    async fn every_received_message_updates_the_keepalive_clock() {
        // ★"Received anything = alive"★: the update does not distinguish message **kinds** (the reason production code updates
        //   **before** the match). In particular, if Pong is missing, a healthy connection that answers only active Pings is misjudged as idle and
        //   cut (half-open false positive). To catch a regression that moves the update into some arms, all 6 kinds must be
        //   exercised, so every variant of `Message` is fed in.
        assert!(clock_updated_by(Message::Text("cmd".to_string().into())).await);
        assert!(clock_updated_by(Message::Binary(vec![1].into())).await);
        assert!(clock_updated_by(Message::Ping(Vec::new().into())).await);
        assert!(clock_updated_by(Message::Pong(Vec::new().into())).await);
        assert!(clock_updated_by(Message::Close(None)).await);
        // Message::Frame does not come up from real socket receives (tungstenite docs), but read_task has an arm for it,
        //   so it is synthesized and exercised.
        assert!(
            clock_updated_by(Message::Frame(
                tokio_tungstenite::tungstenite::protocol::frame::Frame::pong(Vec::new())
            ))
            .await
        );
    }

    // ── 12. handle_connection order contract (ADR-0129) ─────────────────────────────────

    /// An arbitrary protocol version used by this crate's tests. ★Deliberately a different value from the real production version★ — if a regression arises where the server
    /// reads the constant back, it diverges from this value and the test breaks (proof that the injection path exists).
    const TEST_WIRE_VERSION: u32 = 4242;

    /// Starts one test server and returns a client that has completed authentication (common skeleton).
    async fn serve_one(
        registry: ConnRegistry,
        fake: Arc<FakeHandler>,
        keepalive: KeepaliveConfig,
    ) -> (
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
        tokio::task::JoinHandle<()>,
    ) {
        serve_one_with_versions(
            registry,
            fake,
            keepalive,
            TEST_WIRE_VERSION,
            TEST_WIRE_VERSION,
        )
        .await
    }

    async fn serve_one_with_versions(
        registry: ConnRegistry,
        fake: Arc<FakeHandler>,
        keepalive: KeepaliveConfig,
        server_expects: u32,
        client_sends: u32,
    ) -> (
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let factory: Arc<dyn ConnectionHandlerFactory> = Arc::new(FakeFactory { handler: fake });
        let server = tokio::spawn(async move {
            let (stream, peer) = listener.accept().await.unwrap();
            handle_connection(
                stream,
                peer,
                registry,
                factory,
                Arc::new("tok".to_string()),
                server_expects,
                keepalive,
            )
            .await;
        });

        let (mut client, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/"))
            .await
            .unwrap();
        let auth = serde_json::to_string(&AuthFrame::Auth {
            token: engram_dashboard_protocol::Secret::new("tok"),
            protocol_version: client_sends,
        })
        .unwrap();
        client.send(Message::Text(auth.into())).await.unwrap();
        (client, server)
    }

    #[tokio::test]
    async fn auth_rejects_version_mismatch_using_the_injected_expectation() {
        let registry = ConnRegistry::new();
        let fake = Arc::new(FakeHandler::new(None));
        let (mut client, server) = serve_one_with_versions(
            registry.clone(),
            fake.clone(),
            KeepaliveConfig::default(),
            TEST_WIRE_VERSION,
            TEST_WIRE_VERSION + 1,
        )
        .await;

        let msg = tokio::time::timeout(Duration::from_secs(5), client.next())
            .await
            .expect("거부 통보가 와야")
            .expect("스트림이 살아 있어야")
            .expect("프레임 수신");
        match msg {
            Message::Text(t) => assert_eq!(
                t.as_str(),
                format!(
                    "protocol_version mismatch: client {} != server {}",
                    TEST_WIRE_VERSION + 1,
                    TEST_WIRE_VERSION
                ),
                "두 숫자 모두 주입값 기준이어야(상수 하드코딩이면 어긋난다)"
            ),
            other => panic!("Text 거부 통보여야: {other:?}"),
        }

        tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .expect("버전 불일치면 handle_connection 이 반환해야")
            .unwrap();
        assert!(!registry.contains(1), "인증 실패 연결은 등록되지 않는다");
        assert_eq!(
            fake.calls(),
            Vec::<String>::new(),
            "인증 실패면 위층 핸들러가 아예 붙지 않는다"
        );
    }

    /// Pins down the three orders the real `handle_connection` keeps — things the read_task harness cannot catch:
    ///   ① `on_connect` comes before **spawning write_task** (so that the `on_connect` contract's premise "there is no
    ///      consumer yet" holds)
    ///   ② `on_connect` comes before **spawning read_task** (so that a command cannot overtake the greeting)
    ///   ③ `on_disconnect` comes before registry removal
    ///
    /// ★The honest range of detection power★ — none of the three assertions **can fail correct code** (all negative: "nothing
    ///   has been processed/reached yet"). But only ③ catches a regression deterministically:
    ///   - ①·② are both **probabilistic**. Spawning by itself runs nothing, so a wrongly
    ///     early-spawned task leaves a trace only if it **is polled inside this window**. In the case of ②, even if a regressed implementation spawns read_task first,
    ///     the executor may poll `on_connect` up to the assertion before the gate before that, and if the command is still not processed by the time the gate
    ///     opens, it all passes through the assertion after the gate and the last call-order check. So putting assertions at **two places**, before and after the gate, widens
    ///     the window (it is free), but it is not a guarantee.
    ///     ★There is no deterministic discriminator★ — it cannot be made without a hook directly on scheduling or task creation.
    ///   - ③ looks directly at the registry inside `on_disconnect`, so it is effectively deterministic.
    ///   Summary: false positives 0 · ③ deterministic · ①② probabilistic (sampling with a widened window).
    #[tokio::test]
    async fn handle_connection_orders_connect_before_both_tasks_and_cleanup_before_unregister() {
        let registry = ConnRegistry::new();
        let gate = Arc::new(Notify::new());
        let fake = Arc::new(FakeHandler::probing(registry.clone(), gate.clone()));
        let (mut client, server) =
            serve_one(registry.clone(), fake.clone(), KeepaliveConfig::default()).await;

        // Feed commands in advance so they wait on the server socket while the gate is closed.
        client
            .send(Message::Text("cmd".to_string().into()))
            .await
            .unwrap();

        // ★①★ If anything reaches the client in this window, the writer is already up. In correct code
        //   there is no writer, so **nothing** ever arrives — that is, a false positive (flake) is impossible, and under
        //   high load it errs only toward false negatives (misses). It is **probabilistic detection, not a deterministic guarantee** (which is also why it is not a
        //   threshold-tuning target — raising the value does not make it a guarantee).
        assert!(
            tokio::time::timeout(Duration::from_millis(200), client.next())
                .await
                .is_err(),
            "on_connect 진행 중엔 writer 가 없어 소켓으로 나가는 게 없어야 한다"
        );

        gate.notify_one();

        // ★Does not depend on client close timing★: close **after** confirming the command was actually processed.
        //   (If closing came first, the race of the writer's greeting write failing → reader abort could decide the result.)
        tokio::time::timeout(Duration::from_secs(5), fake.text_seen.notified())
            .await
            .expect("명령이 처리돼야");
        client.close(None).await.unwrap();

        tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .expect("handle_connection 이 반환해야")
            .unwrap();

        let calls = fake.calls();
        assert_eq!(
            calls,
            vec!["connect:1", "text:1:cmd", "disconnect:1"],
            "connect → 프레임 → disconnect 순서"
        );
        // ★③★
        assert_eq!(
            fake.registered_at_disconnect(),
            Some(true),
            "on_disconnect 시점엔 아직 fanout 레지스트리에 있다"
        );
        assert!(
            !registry.contains(1),
            "handle_connection 이 반환할 땐 등록 해제돼 있다"
        );
    }

    /// ★★Teardown does **not discard** an accepted priority frame★★ — the regression net for [`finish_bypass_lane`].
    ///
    /// ★The defect it catches★: in the branch where the arrival-order lane ends first (the common shape where a peer sends a command and closes right away),
    ///   `handle_connection` simply `abort()`ed the priority-lane task. A frame the upper layer decided 「이건 굶으면 안 된다」 (this must not starve) and
    ///   **accepted** vanished without ever running — the reason that lane was made collapses entirely.
    ///
    /// ★Setting the stage★: lock the two lanes **each**. The arrival-order lane is held at a gate to fill the inbound queue (so that
    ///   what follows goes to the priority lane), and the priority lane is separately locked **at the first item** so the second stays in the queue. In that state,
    ///   releasing the arrival-order lane and closing the socket starts teardown, and at that moment the priority lane has **one frame accepted and not yet run** — if the defect exists,
    ///   that frame vanishes together with the abort.
    /// ★The honest range of detection★: the 300ms window looks only at **whether the buggy version finishes cleanup within it** (correct code waits for
    ///   the priority lane and never finishes in that window — false positives impossible). If the window is short and the buggy version has not finished yet, it
    ///   errs only toward false negatives. The assertion that opens the gate afterwards and looks at the result is deterministic in both branches.
    #[tokio::test]
    async fn an_accepted_bypass_frame_survives_the_ordered_row_finishing_first() {
        let registry = ConnRegistry::new();
        let fake = Arc::new(FakeHandler::blocking_on_both("slow", "bypass:first"));
        let (mut client, mut server) =
            serve_one(registry.clone(), fake.clone(), KeepaliveConfig::default()).await;

        // ① Hold the arrival-order consumer — after this that lane is never empty.
        client
            .send(Message::Text("slow".to_string().into()))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), fake.text_seen.notified())
            .await
            .expect("첫 명령이 핸들러에 닿아야");

        // ② Fill the arrival-order lane completely (the consumer took one, so exactly CONN_RX_CAP frames).
        for i in 0..CONN_RX_CAP {
            client
                .send(Message::Text(format!("filler-{i}").into()))
                .await
                .unwrap();
        }

        // ③ Two frames to the priority lane in the saturated state — the first is held at that lane's gate, and the second stays in the queue.
        client
            .send(Message::Text("bypass:first".to_string().into()))
            .await
            .unwrap();
        client
            .send(Message::Text("bypass:second".to_string().into()))
            .await
            .unwrap();

        let mut first_running = false;
        for _ in 0..500 {
            if fake.calls().iter().any(|c| c == "text:1:bypass:first") {
                first_running = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            first_running,
            "우선 줄이 첫 장을 집지 못했다 — 이 시험의 무대가 안 섰다: {:?}",
            fake.calls()
        );

        // ④ Release the arrival-order lane and close the socket → read ends → dispatch drains what is left and ends → teardown.
        fake.slow_gate.notify_one();
        client.close(None).await.unwrap();

        // ⑤ Correct code waits for the priority lane, so it does not finish within this window. If it finished, it threw the frame away.
        let finished_before_the_gate =
            tokio::time::timeout(Duration::from_millis(300), &mut server)
                .await
                .is_ok();

        // ⑥ Now release the priority lane — the accepted second frame must run here.
        fake.bypass_gate.notify_one();
        if !finished_before_the_gate {
            tokio::time::timeout(Duration::from_secs(15), &mut server)
                .await
                .expect("우선 줄이 비면 정리가 끝나야")
                .unwrap();
        }

        assert!(
            fake.calls().iter().any(|c| c == "text:1:bypass:second"),
            "받아 둔 우선 프레임이 teardown 의 abort 로 사라졌다(창 안 종료 = {finished_before_the_gate}): {:?}",
            fake.calls()
        );
        drop(client);
    }

    /// ★Explicit abort of the loser task★ (the select! in `handle_connection`) — if a JoinHandle is simply dropped,
    /// it is detached and the task holding the WS half survives. That leak is not visible in e2e (everything ends normally, so
    /// the remaining half does not show up in the table).
    ///
    /// ★Observation method★: **only dispatch_task** holds the **strong reference** to the frame outlet `Arc` (`handle_connection` moves
    /// it into dispatch_task, and `on_connect` only borrows). So if the weak reference can still be upgraded after the connection ends = that task is
    /// alive = it was detached instead of aborted.
    /// ★That the observation also bites **the abort of read_task** is due to the wiring★: for dispatch_task to end by itself, the inbound queue's
    /// only sender (held by read_task) must be dropped, but in this branch read_task is parked on `next()`
    /// and does not end by itself — that is, even if read's abort is missing, this polling runs to the end with
    /// `released == false`. The old comment stated that this strong reference was in read_task.
    /// ★This observation holds **only with this test's fake handler**★ — that fake keeps no strong reference at all
    /// (the prohibition clause in the `FakeHandler` comment below). The production handler copies in `on_connect` and puts the copy in the
    /// **command registry** and the **usage registry**, holding it for the whole connection lifetime, so if this test were moved there the observation would not
    /// hold. That is, items ⑤·⑥ of the 「사본 전수」 (copy enumeration) below are **not protected by this test**.
    /// To take this direction (the branch where write ends first and aborts read), the handler puts `Frame::Close` in the queue and
    /// returns Continue — read stays parked on `next()`.
    /// ★The symmetric branch (read ends first and aborts write) is **left unverified**★ — it is only that the conditional observation below
    /// kept a negative test from being made, and **it does not mean that `abort()` is unnecessary.**
    ///
    /// ★Observed (2026-08-04, only with the current `AgentConnection` and only when the race below does not occur)★: even when write_task is
    /// detached, on `handle_connection` return the last `Sender<Frame>` is dropped, `conn_rx.recv()` returns
    /// None, and write_task **ended by itself** and released sink_half (looking at "do more Pings come after the end" with a short `ping_interval`
    /// gave an immediate EOF — the socket was closed = the writer had already ended).
    ///
    /// ★That self-termination is **conditional** on "all copies of `Sender<Frame>` die together with `handle_connection`"★.
    /// Enumeration of copies: ① this function's local `conn_tx` ② the registry entry (vanishes on deregistration) ③ the
    /// `ConnFrameSink` owned by read_task ④ the `FrameOutputSink` copies registered in the core subscribers — ④ is reclaimed only
    /// through the `session.subs` record (`on_disconnect`) ⑤ one copy held by **the command bus's owner registry** from `on_connect` to `on_disconnect`
    /// — the upper layer holds it to designate which connection to send a command envelope through. The body of ⑤ has one place to put it in and one to take it out, both inside one lock of that registry,
    /// so it has no missed-record race like ④.
    /// ⑥ one copy held by **the usage registry's** per-connection outlet (the daemon's `ConnUsageOutlet`) — put in at `on_connect` and
    /// collected by the usage-registry detach in `on_disconnect`. As with ⑤, there is one place to put it in and one to collect it.
    /// ★Short-lived copies that can outlive `on_disconnect` are not in this enumeration★ — the copy that the ⑥ outlet briefly holds when sending one frame
    /// (a single `try_send` to a dying queue is all), and the sink copies held by command tasks peeled off the lane
    /// (the activation command — which existed originally — and the usage ⟳ query, which holds it up to the daemon's `REPLY_WAIT_MAX`).
    /// Both are harmless because `handle_connection` aborts the write side — deleting that abort delays their
    /// self-termination by that much.
    /// ★However, the copy that ⑤ **hands out** is not in this enumeration either★ — the registry takes a copy at designation time and hands it to the delivering side,
    /// and after handing it out there is no means of reclaiming it. If a delivery puts it in a wait ticket and waits for the reply, that
    /// copy outlives `on_disconnect`. So that side is blocked only by the **caller contract** (receive it, use it immediately, and discard it),
    /// and neither the compiler nor this crate enforces it.
    /// ★So if ④ leaks, the condition breaks, and a writer detached because of a surviving sender **does not
    /// end by itself**★. There is a path where it really leaks: if read_task **panics** after `handle_subscribe` registers a sink in the core and **before**
    /// `subs.insert` (no `.await` lies in between so cancellation cannot intrude, but a panic can), that sink stays in the core
    /// unrecorded anywhere and holds a conn_tx copy.
    /// The production daemon ends that panic in the hook before it unwinds, but it is reachable in a hookless test build (ADR-0292).
    /// **Conclusion: the `abort()` on that branch must not be removed** — the measurement above is "that is how it was when that race did not occur", not
    /// an invariant.
    /// On the opposite branch (this test), read_task is parked on `next()` and does not end by itself, so abort is the only
    /// means of reclamation — that is why only this one is verified.
    #[tokio::test]
    async fn the_losing_task_is_aborted_not_detached() {
        let registry = ConnRegistry::new();
        let fake = Arc::new(FakeHandler::closing_via_writer("die"));
        let (mut client, server) =
            serve_one(registry.clone(), fake.clone(), KeepaliveConfig::default()).await;

        client
            .send(Message::Text("die".to_string().into()))
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(10), server)
            .await
            .expect("write_task 가 Frame::Close 로 끝나면 handle_connection 도 반환해야")
            .unwrap();

        // abort is a request, so the future drop is not synchronous with the return — poll generously (correct code releases
        //   soon, and the detach regression is parked on `next()` forever and never releases).
        let mut released = false;
        for _ in 0..200 {
            if !fake.frames_still_held() {
                released = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            released,
            "패자 task 가 abort 되지 않았다(detach) — WS half 를 쥔 채 살아남는다"
        );
        drop(client);
    }
}
