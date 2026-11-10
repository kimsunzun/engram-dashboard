//! Tests of the daemon's transport adapter.
//!
//! Two harnesses: synthetic inbox items drive the read · dispatch · bypass tasks without a socket
//! (ADR-0129), and a real loopback listener drives the whole connection life (`Server::serve`) with a tokio-tungstenite
//! client.

use super::*;

use std::sync::atomic::AtomicBool;

use engram_dashboard_protocol::handshake::AuthFrame;
use engram_dashboard_protocol::Secret;
use engram_dashboard_transport::Arrival;
use futures_util::SinkExt;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

// ── fakes ─────────────────────────────────────────────────────────────────────────────

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
    /// On receiving this text it puts `Frame::Close` in the queue and then waits on [`Self::slow_gate`] — the link ends on the
    /// **write side** while dispatch is stuck inside the handler.
    close_then_hang_on: Option<String>,
    /// On receiving this text it puts `Text("hi")` · `Binary([1, 2, 3])` · `Close` in that order and returns Continue.
    emit_on: Option<String>,
    /// On receiving this text it puts `Text("ack")` and returns `ConnFlow::Close`.
    ack_close_on: Option<String>,
    /// [`backlog_frame`]s that `emit_on` · `ack_close_on` put **before** their own frames.
    backlog: usize,
    /// Notified once `emit_on` has put all its frames, the close included.
    emitted: Arc<Notify>,
    /// On receiving this text it `try_send`s this many frames without yielding; the index of the first refusal is recorded.
    flood_on: Option<(String, usize)>,
    flood_refused_at: Mutex<Option<usize>>,
    /// After `on_connect` puts in the greeting frame, it waits here. During that window the test feeds commands and checks that
    /// none is processed (= the read task is not started yet).
    connect_gate: Option<Arc<Notify>>,
    /// Signal that 1 `on_text` has started — so the test can proceed regardless of the client close timing.
    text_seen: Arc<Notify>,
    /// On receiving this text it waits **inside** `on_text` until [`Self::slow_gate`] opens.
    /// ★A device to avoid relying on timing assumptions★: imitating it with sleep closes the window on a slow runner and gives
    ///   a false positive. The gate stays closed **forever** until the test opens it.
    slow_on: Option<String>,
    slow_gate: Arc<Notify>,
    /// The **priority lane version** of [`Self::slow_on`]. Only by locking it **separately** from the arrival-order lane can the state 「도착순은 끝났는데 우선 줄은 아직
    /// 한 건에 붙들려 있다」 (arrival order is finished but the priority lane is still held by one item) be created.
    bypass_gate_on: Option<String>,
    bypass_gate: Arc<Notify>,
    /// A weak reference to the frame outlet that `on_connect` received. ★This fake **must never keep a strong reference to the
    /// frame outlet**★ — if even one `Arc` is left in a field, upgrade always succeeds and the abort tests always fail on correct code.
    frames_weak: Mutex<Option<std::sync::Weak<dyn FrameSink>>>,
    /// Whether this connection was still in the fan-out registry at the time of `on_disconnect` (observing the cleanup order).
    registry: Option<ConnRegistry>,
    registered_at_disconnect: Mutex<Option<bool>>,
    /// How many times `on_inbound_saturated` was asked.
    saturated_calls: AtomicU64,
}

impl FakeHandler {
    fn new(close_on: Option<&str>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            close_on: close_on.map(|s| s.to_string()),
            close_then_hang_on: None,
            emit_on: None,
            ack_close_on: None,
            backlog: 0,
            emitted: Arc::new(Notify::new()),
            flood_on: None,
            flood_refused_at: Mutex::new(None),
            connect_gate: None,
            text_seen: Arc::new(Notify::new()),
            slow_on: None,
            slow_gate: Arc::new(Notify::new()),
            bypass_gate_on: None,
            bypass_gate: Arc::new(Notify::new()),
            frames_weak: Mutex::new(None),
            registry: None,
            registered_at_disconnect: Mutex::new(None),
            saturated_calls: AtomicU64::new(0),
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

    /// For order verification of the connection task — peeks into the registry and catches on_connect with a gate.
    fn probing(registry: ConnRegistry, connect_gate: Arc<Notify>) -> Self {
        Self {
            registry: Some(registry),
            connect_gate: Some(connect_gate),
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

    /// In correct code, no frame of this connection can be processed before `on_connect` finishes (its read task does not exist yet).
    fn assert_nothing_processed_yet(&self, conn_id: ConnId, at: &str) {
        let calls = self.calls.lock().unwrap();
        let mine = calls.iter().any(|c| match c {
            HandlerCall::Connect(id)
            | HandlerCall::Text(id, _)
            | HandlerCall::Binary(id, _)
            | HandlerCall::Disconnect(id) => *id == conn_id,
        });
        assert!(
            !mine,
            "on_connect({at}) 보다 먼저 처리된 프레임이 있다 — read_task 가 앞서 스폰됐다: {calls:?}"
        );
    }

    fn put_backlog(&self, frames: &Arc<dyn FrameSink>) {
        for i in 0..self.backlog {
            frames
                .try_send(Frame::Text(backlog_frame(i)))
                .expect("the backlog fits the outbound queue");
        }
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
            self.assert_nothing_processed_yet(conn_id, "게이트 진입 전");
            let _ = frames.send(Frame::Text("greeting".into())).await;
            if let Some(gate) = &self.connect_gate {
                gate.notified().await;
            }
            // ★Assertion **after** the gate★: during the window until the gate opens (in which the test has fed commands in advance), it catches whether
            //   a wrongly spawned read task processed that command.
            self.assert_nothing_processed_yet(conn_id, "게이트 통과 후");
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
            self.calls
                .lock()
                .unwrap()
                .push(HandlerCall::Text(conn_id, text.to_string()));
            self.text_seen.notify_one();
            if let Some((flood, count)) = &self.flood_on {
                if flood == text {
                    for i in 0..*count {
                        if frames.try_send(Frame::Text(format!("flood-{i}"))).is_err() {
                            *self.flood_refused_at.lock().unwrap() = Some(i);
                            break;
                        }
                    }
                }
            }
            if self.emit_on.as_deref() == Some(text) {
                self.put_backlog(frames);
                let _ = frames.try_send(Frame::Text("hi".into()));
                let _ = frames.try_send(Frame::Binary(vec![1, 2, 3]));
                let _ = frames.try_send(Frame::Close("bye".into()));
                self.emitted.notify_one();
            }
            if self.ack_close_on.as_deref() == Some(text) {
                self.put_backlog(frames);
                let _ = frames.try_send(Frame::Text("ack".into()));
                return ConnFlow::Close;
            }
            if self.close_then_hang_on.as_deref() == Some(text) {
                let _ = frames.try_send(Frame::Close("테스트: 쓰기 쪽이 먼저 끝난다".into()));
                self.slow_gate.notified().await;
            }
            if self.slow_on.as_deref() == Some(text) {
                self.slow_gate.notified().await;
            }
            if self.bypass_gate_on.as_deref() == Some(text) {
                self.bypass_gate.notified().await;
            }
            if self.close_on.as_deref() == Some(text) {
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
    /// the refusal too to tell it apart from silent loss — and splits into [`Saturated::Unanswered`] by the reply result,
    /// the same shape as the production handler.
    fn on_inbound_saturated(
        &self,
        _conn_id: ConnId,
        text: &str,
        frames: &Arc<dyn FrameSink>,
    ) -> Saturated {
        self.saturated_calls.fetch_add(1, Ordering::AcqRel);
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

// ── synthetic inbox harness ───────────────────────────────────────────────────────────

fn arrival(msg: RawIn) -> Incoming<DaemonWire> {
    Incoming::Message(Arrival {
        peer: PeerId::new("test"),
        generation: Generation(1),
        msg,
    })
}

fn text_item(s: &str) -> Incoming<DaemonWire> {
    arrival(RawIn::Text(s.to_string()))
}

fn binary_item(bytes: Vec<u8>) -> Incoming<DaemonWire> {
    arrival(RawIn::Binary(bytes))
}

fn closed_item(cause: DisconnectCause) -> Incoming<DaemonWire> {
    Incoming::Event(TransportEvent::Closed {
        peer: PeerId::new("test"),
        generation: Generation(1),
        cause,
    })
}

fn new_rx_bytes() -> RxBytes {
    Arc::new(AtomicUsize::new(0))
}

/// Runs the read layer and processing layer together **in the actual arrangement** (chained through the inbound queue) — a minimal
/// reproduction of the wiring the connection task does, and returns after both finish.
async fn run_read_and_dispatch<S>(
    incoming: S,
    frames: Arc<dyn FrameSink>,
    handler: Arc<dyn ConnectionHandler>,
    conn_id: ConnId,
) where
    S: Stream<Item = Incoming<DaemonWire>> + Unpin + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
    let rx_bytes = new_rx_bytes();
    let (bypass_tx, bypass_rx) = mpsc::channel::<Inbound>(CONN_BYPASS_CAP);
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let read = tokio::spawn(read_task(
        incoming,
        tx,
        bypass_tx,
        rx_bytes.clone(),
        frames.clone(),
        handler.clone(),
        conn_id,
        async move {
            let _ = stop_rx.await;
        },
    ));
    let bypass = tokio::spawn(bypass_task(
        bypass_rx,
        frames.clone(),
        handler.clone(),
        conn_id,
    ));
    dispatch_task(rx, rx_bytes, frames, handler, conn_id).await;
    // The same disposition as the connection task's "dispatch finished first" branch.
    let _ = stop_tx.send(());
    let _ = read.await;
    finish_bypass_lane(bypass, conn_id).await;
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
            text_item("cmd"),
            binary_item(vec![1, 2, 3]),
            closed_item(DisconnectCause::PeerClosed),
            text_item("after-close"),
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
            "disconnect:7", // Items after the end are not consumed
        ]
    );
    assert_eq!(
        fake_sink.frames(),
        vec!["text:greeting"],
        "on_connect 가 넣은 프레임이 단일 출구로 나간다"
    );
}

/// When the processing layer receives `ConnFlow::Close`, it stops on the spot, and frames that were **already in the queue**
/// behind it are not processed either.
#[tokio::test]
async fn close_flow_from_on_text_stops_the_dispatch_row() {
    let frames: Arc<dyn FrameSink> = Arc::new(FakeFrameSink::default());
    let fake = Arc::new(FakeHandler::new(Some("stop")));
    let handler: Arc<dyn ConnectionHandler> = fake.clone();

    run_read_and_dispatch(
        futures_util::stream::iter(vec![
            text_item("go"),
            text_item("stop"),
            text_item("unreachable"),
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

/// ★★Regression net for socket head-of-line blocking★★ — while one handler call runs, **input · kill · another activation
/// bound for other agents** sent by the same client must still be taken out.
///
/// ★It measures two things at once★: ① while the handler is held by one command, the rest of the items are **all** read
/// ② and yet processing is still **one at a time in arrival order**. Measuring only ① misses a regression that "got faster by dropping order", and measuring
/// only ② stays green even if the original defect is still there.
#[tokio::test]
async fn the_read_loop_runs_ahead_while_a_handler_call_is_still_running() {
    let frames: Arc<dyn FrameSink> = Arc::new(FakeFrameSink::default());
    let fake = Arc::new(FakeHandler::blocking_on("slow"));
    let handler: Arc<dyn ConnectionHandler> = fake.clone();

    let yielded = Arc::new(AtomicU64::new(0));
    let counter = yielded.clone();
    let incoming = futures_util::stream::iter(vec![
        text_item("slow"),
        text_item("other-agent-input"),
        text_item("kill"),
        binary_item(vec![9, 9]),
        text_item("tail"),
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
        std::future::pending::<()>(),
    ));
    let dispatch = tokio::spawn(dispatch_task(rx, rx_bytes, frames, handler, 5));

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
        "핸들러가 첫 명령에 붙들린 동안 나머지 프레임이 꺼내지지 않았다 — head-of-line blocking 회귀"
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

/// ★★If the inbound lane **has no room for cancellation**, the moment the lane fills, a `Kill` starves behind it★★.
///
/// ★The point is not 「우선 줄이 있다」 (there is a priority lane) but 「**읽기가 안 선다**」 (reading does not stop)★: this test places `Kill` as **a frame
/// that arrives after the lane is full**, and slips in one more ordinary command before it to also check 「그 한 장을 처분하고 계속 읽었는가」
/// (did it dispose of that one and keep reading).
#[tokio::test]
async fn a_cancel_still_gets_through_a_saturated_inbound_queue() {
    let sink = Arc::new(FakeFrameSink::default());
    let frames: Arc<dyn FrameSink> = sink.clone();
    let fake = Arc::new(FakeHandler::blocking_on("slow"));
    let handler: Arc<dyn ConnectionHandler> = fake.clone();

    // Fill the lane completely: 1 frame held at the gate + CONN_RX_CAP frames to fill the lane.
    let mut items = vec![text_item("slow")];
    for i in 0..CONN_RX_CAP {
        items.push(text_item(&format!("filler-{i}")));
    }
    // ★From here on is the main point of this test★ — two frames arriving while the lane has no room.
    items.push(text_item("overflow-ordered"));
    items.push(text_item("bypass:kill"));

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
        std::future::pending::<()>(),
    ));
    let bypass = tokio::spawn(bypass_task(bypass_rx, frames.clone(), handler.clone(), 11));
    let dispatch = tokio::spawn(dispatch_task(rx, rx_bytes, frames, handler, 11));

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
    // ★What is measured here is **the set, not the order**★ — the two lanes are different tasks.
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
/// ★Why **two** large frames★: the decision is 「더하기 전 물높이」 (the fill level before adding), so one frame arriving at an empty lane
///   always goes in regardless of size. What exceeds the budget is from the next frame on.
#[tokio::test]
async fn the_inbound_row_is_bounded_by_bytes_not_only_by_slots() {
    let sink = Arc::new(FakeFrameSink::default());
    let frames: Arc<dyn FrameSink> = sink.clone();
    let fake = Arc::new(FakeHandler::blocking_on("slow"));
    let handler: Arc<dyn ConnectionHandler> = fake.clone();

    // Two frames each one byte more than half the budget — two exceed the budget, and only three of the 64 entries are used.
    let big = "x".repeat(CONN_RX_MAX_BYTES / 2 + 1);
    let items = vec![
        text_item("slow"),
        text_item(&big),
        text_item(&big),
        text_item("after-budget"),
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
        std::future::pending::<()>(),
    ));
    let dispatch = tokio::spawn(dispatch_task(rx, rx_bytes, frames, handler, 21));

    // The read loop finishes after scanning all the items (it does not stop even on exceeding the budget).
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

/// ★★If the refusal **cannot be sent as a reply**, feeding ends — and the inbox is still drained to its end★★ — the
/// regression net for [`Saturated::Unanswered`].
///
/// ★The regression it catches★: the shape where the read layer treated `Unanswered` like `Refused` and went on asking for
///   dispositions. Observation = 「위층에 처분을 몇 번 물었나」 (how many times the upper layer was asked): once if feeding stopped, three
///   times (the trigger + the two tails) if not.
/// ★And the inbox is not left undrained★ — every item is still taken out (and discarded), so the transport's cleanup is not
///   held by this connection (transport receiver invariant 5).
/// ★Do not start the consumer at all★: with only the receiving end kept alive, the lane fills at exactly `CONN_RX_CAP` frames,
///   so the decision is deterministic.
#[tokio::test]
async fn a_refusal_that_could_not_be_sent_stops_feeding_but_drains_the_inbox() {
    // An outlet that accepts not a single frame = a state where the outbound queue is saturated too.
    let frames: Arc<dyn FrameSink> = Arc::new(FakeFrameSink::refusing());
    let fake = Arc::new(FakeHandler::new(None));
    let handler: Arc<dyn ConnectionHandler> = fake.clone();

    let mut items = Vec::new();
    for i in 0..CONN_RX_CAP {
        items.push(text_item(&format!("filler-{i}")));
    }
    items.push(text_item("unanswerable")); // That one frame where the lane is full and no reply can be sent either
    items.push(text_item("tail-1"));
    items.push(text_item("tail-2"));
    let total = items.len();

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
        std::future::pending::<()>(),
    )
    .await;

    assert_eq!(
        fake.saturated_calls.load(Ordering::Acquire),
        1,
        "답장 못 한 거절을 '거절했다'로 읽고 계속 줄에 넣었다"
    );
    assert_eq!(
        pulled.load(Ordering::Acquire) as usize,
        total,
        "줄에 넣기를 멈춘 뒤에도 수신단은 끝까지 비워야 한다"
    );
}

/// The read task reports how the link ended — the connection task splits its teardown on it (read end = drain ·
/// write end = abort). Items after the end are not consumed.
#[tokio::test]
async fn the_link_end_tells_the_read_side_from_the_write_side() {
    async fn end_of(items: Vec<Incoming<DaemonWire>>) -> (LinkEnd, Vec<String>) {
        let fake = Arc::new(FakeHandler::new(None));
        let handler: Arc<dyn ConnectionHandler> = fake.clone();
        let frames: Arc<dyn FrameSink> = Arc::new(FakeFrameSink::default());
        let (tx, mut rx) = mpsc::channel::<Inbound>(CONN_RX_CAP);
        let (bypass_tx, _bypass_rx) = mpsc::channel::<Inbound>(CONN_BYPASS_CAP);
        let end = read_task(
            futures_util::stream::iter(items),
            tx,
            bypass_tx,
            new_rx_bytes(),
            frames,
            handler,
            1,
            std::future::pending::<()>(),
        )
        .await;
        let mut fed = Vec::new();
        while let Ok(item) = rx.try_recv() {
            if let Inbound::Text(t) = item {
                fed.push(t);
            }
        }
        (end, fed)
    }

    for cause in [DisconnectCause::PeerClosed, DisconnectCause::ReadFailed] {
        let (end, fed) = end_of(vec![text_item("a"), closed_item(cause), text_item("after")]).await;
        assert_eq!(end.cause, Some(cause));
        assert!(end.read_side(), "{cause:?}");
        assert_eq!(fed, vec!["a"], "{cause:?}");
    }
    for cause in [
        DisconnectCause::WriteFailed,
        DisconnectCause::WriteDeadline,
        DisconnectCause::Idle,
        DisconnectCause::Explicit,
        DisconnectCause::QueueFull,
    ] {
        let (end, _) = end_of(vec![closed_item(cause)]).await;
        assert!(!end.read_side(), "{cause:?}");
    }
    // An inbox that ends without `Closed` = an abnormal supervisor end — handled as a write end (abort).
    let (end, _) = end_of(vec![text_item("a")]).await;
    assert_eq!(end.cause, None);
    assert!(!end.read_side());
}

// ── real-socket harness ────────────────────────────────────────────────────────────────

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// An arbitrary protocol version. ★Deliberately not the production version★ — if the server read the constant back,
/// the test would break (proof that the injection path exists).
const TEST_WIRE_VERSION: u32 = 4242;

fn auth_message(version: u32) -> Message {
    let auth = serde_json::to_string(&AuthFrame::Auth {
        token: Secret::new("tok"),
        protocol_version: version,
    })
    .unwrap();
    Message::Text(auth.into())
}

fn test_server(
    fake: Arc<FakeHandler>,
    version: u32,
    keepalive: KeepaliveConfig,
) -> (Listening, Arc<Server>, ConnRegistry) {
    let listening = Listening::bind_loopback().expect("bind");
    let conns = ConnRegistry::new();
    let factory: Arc<dyn ConnectionHandlerFactory> = Arc::new(FakeFactory { handler: fake });
    let server = Server::new(
        listening.clock(),
        conns.clone(),
        factory,
        "tok",
        version,
        keepalive,
    );
    (listening, server, conns)
}

async fn connect(port: u16) -> Client {
    let (client, _) = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/")),
    )
    .await
    .expect("connect timeout")
    .expect("connect");
    client
}

/// Serves exactly one connection and returns a client that has sent its auth frame. The join handle ends when that
/// connection's cleanup is done.
async fn serve_one_with_versions(
    fake: Arc<FakeHandler>,
    keepalive: KeepaliveConfig,
    server_expects: u32,
    client_sends: u32,
) -> (Client, tokio::task::JoinHandle<()>, ConnRegistry) {
    let (listening, server, conns) = test_server(fake, server_expects, keepalive);
    let port = listening.port();
    let Listening {
        mut listener,
        refusals,
        clock: _,
    } = listening;
    let task = tokio::spawn(async move {
        let _refusals = refusals;
        let accepted = listener.accept().await.expect("accept");
        server.serve(accepted).await;
    });
    let mut client = connect(port).await;
    client.send(auth_message(client_sends)).await.unwrap();
    (client, task, conns)
}

async fn serve_one(
    fake: Arc<FakeHandler>,
    keepalive: KeepaliveConfig,
) -> (Client, tokio::task::JoinHandle<()>, ConnRegistry) {
    serve_one_with_versions(fake, keepalive, TEST_WIRE_VERSION, TEST_WIRE_VERSION).await
}

/// The next data message, skipping pings — `None` = the connection closed.
async fn next_data(client: &mut Client, within: Duration) -> Option<Message> {
    loop {
        match tokio::time::timeout(within, client.next())
            .await
            .expect("no message in time")
        {
            Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return None,
            Some(Ok(other)) => return Some(other),
        }
    }
}

fn text_of(msg: Option<Message>) -> String {
    match msg {
        Some(Message::Text(t)) => t.to_string(),
        other => panic!("expected a text frame, got {other:?}"),
    }
}

async fn wait_released(fake: &FakeHandler) -> bool {
    // An abort is a request, so the future drop is not synchronous with the return — poll generously.
    for _ in 0..200 {
        if !fake.frames_still_held() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

/// Frames that stay queued while the client does not read: 32 × 256 KiB = 8 MiB, more than the loopback socket buffers
/// hold, so the writer stalls with frames still in the outbound queue. ★A close that does not wait for the queue
/// (`Peer::close` — the control channel overtakes the data queue) drops them★ — that is what the two close-order tests
/// below catch.
const BACKLOG_FRAMES: usize = 32;
const BACKLOG_FRAME_BYTES: usize = 256 * 1024;

fn backlog_frame(i: usize) -> String {
    let mut text = format!("backlog-{i}:");
    text.extend(std::iter::repeat_n('x', BACKLOG_FRAME_BYTES));
    text
}

/// Reads the backlog in order — each frame must arrive whole.
async fn read_backlog(client: &mut Client, within: Duration) {
    for i in 0..BACKLOG_FRAMES {
        let text = text_of(next_data(client, within).await);
        assert!(
            text == backlog_frame(i),
            "backlog frame {i} arrived wrong ({} bytes, starts {:?})",
            text.len(),
            &text[..text.len().min(16)]
        );
    }
}

/// Waits until the connection's cleanup ran — every close of the connection task is issued before `on_disconnect`.
async fn wait_disconnected(fake: &FakeHandler) {
    for _ in 0..1000 {
        if fake.calls().iter().any(|c| c.starts_with("disconnect:")) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("on_disconnect did not run: {:?}", fake.calls());
}

#[tokio::test]
async fn auth_rejects_version_mismatch_using_the_injected_expectation() {
    let fake = Arc::new(FakeHandler::new(None));
    let (mut client, server, conns) = serve_one_with_versions(
        fake.clone(),
        KeepaliveConfig::default(),
        TEST_WIRE_VERSION,
        TEST_WIRE_VERSION + 1,
    )
    .await;

    assert_eq!(
        text_of(next_data(&mut client, Duration::from_secs(5)).await),
        format!(
            "protocol_version mismatch: client {} != server {}",
            TEST_WIRE_VERSION + 1,
            TEST_WIRE_VERSION
        ),
        "두 숫자 모두 주입값 기준이어야(상수 하드코딩이면 어긋난다)"
    );
    assert!(
        next_data(&mut client, Duration::from_secs(5))
            .await
            .is_none(),
        "거부 통보 뒤에는 닫혀야"
    );

    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("버전 불일치면 연결 수명이 끝나야")
        .unwrap();
    assert!(!conns.contains(1), "인증 실패 연결은 등록되지 않는다");
    assert_eq!(
        fake.calls(),
        Vec::<String>::new(),
        "인증 실패면 위층 핸들러가 아예 붙지 않는다"
    );
}

/// Pins down the orders the connection task keeps:
///   ① `on_connect` comes before **starting the read task** (so that a command cannot overtake the greeting)
///   ② `on_disconnect` comes before registry removal
///
/// ★The honest range of detection power★ — ① is **probabilistic** (a wrongly early task leaves a trace only if it is polled inside
///   the gate window — the assertions sit before and after the gate to widen it); ② looks directly at the registry inside
///   `on_disconnect`, so it is deterministic. False positives 0.
/// ★There is no third order (no writer during `on_connect`)★ — the transport's writer runs from the moment the
///   link is live, so the greeting may go out while `on_connect` is still running.
#[tokio::test]
async fn connect_comes_before_the_tasks_and_cleanup_before_unregister() {
    let gate = Arc::new(Notify::new());
    let listening = Listening::bind_loopback().expect("bind");
    // The probe looks at the server's own registry.
    let conns = ConnRegistry::new();
    let fake = Arc::new(FakeHandler::probing(conns.clone(), gate.clone()));
    let factory: Arc<dyn ConnectionHandlerFactory> = Arc::new(FakeFactory {
        handler: fake.clone(),
    });
    let server = Server::new(
        listening.clock(),
        conns.clone(),
        factory,
        "tok",
        TEST_WIRE_VERSION,
        KeepaliveConfig::default(),
    );
    let port = listening.port();
    let Listening {
        mut listener,
        refusals,
        clock: _,
    } = listening;
    let server_task = tokio::spawn(async move {
        let _refusals = refusals;
        let accepted = listener.accept().await.expect("accept");
        server.serve(accepted).await;
    });
    let mut client = connect(port).await;
    client.send(auth_message(TEST_WIRE_VERSION)).await.unwrap();

    // Feed a command in advance so it waits while the gate is closed.
    client
        .send(Message::Text("cmd".to_string().into()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    gate.notify_one();

    tokio::time::timeout(Duration::from_secs(5), fake.text_seen.notified())
        .await
        .expect("명령이 처리돼야");
    client.close(None).await.unwrap();

    tokio::time::timeout(Duration::from_secs(10), server_task)
        .await
        .expect("연결 수명이 끝나야")
        .unwrap();

    assert_eq!(
        fake.calls(),
        vec!["connect:1", "text:1:cmd", "disconnect:1"],
        "connect → 프레임 → disconnect 순서"
    );
    assert_eq!(
        fake.registered_at_disconnect(),
        Some(true),
        "on_disconnect 시점엔 아직 fanout 레지스트리에 있다"
    );
    assert!(!conns.contains(1), "연결 수명이 끝날 땐 등록 해제돼 있다");
}

/// ★★Teardown does **not discard** an accepted priority frame★★ — the regression net for [`finish_bypass_lane`].
///
/// ★Setting the stage★: lock the two lanes **each**. The arrival-order lane is held at a gate to fill the inbound queue (so that
///   what follows goes to the priority lane), and the priority lane is separately locked **at the first item** so the second stays in the queue. In that state,
///   releasing the arrival-order lane and closing the socket starts teardown, and at that moment the priority lane has **one frame accepted and not yet run**.
/// ★The honest range of detection★: the 300ms window looks only at **whether the buggy version finishes cleanup within it** (correct code waits for
///   the priority lane — false positives impossible). The assertion that opens the gate afterwards and looks at the result is deterministic in both branches.
#[tokio::test]
async fn an_accepted_bypass_frame_survives_the_ordered_row_finishing_first() {
    let fake = Arc::new(FakeHandler::blocking_on_both("slow", "bypass:first"));
    let (mut client, mut server, _conns) =
        serve_one(fake.clone(), KeepaliveConfig::default()).await;

    client
        .send(Message::Text("slow".to_string().into()))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), fake.text_seen.notified())
        .await
        .expect("첫 명령이 핸들러에 닿아야");

    for i in 0..CONN_RX_CAP {
        client
            .send(Message::Text(format!("filler-{i}").into()))
            .await
            .unwrap();
    }
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

    fake.slow_gate.notify_one();
    client.close(None).await.unwrap();

    let finished_before_the_gate = tokio::time::timeout(Duration::from_millis(300), &mut server)
        .await
        .is_ok();

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

/// ★A write end aborts a dispatch stuck inside a handler★ — not detached. The handler puts `Frame::Close` and then hangs:
/// the link ends on the write side while dispatch holds the frame outlet. Only an abort releases it.
///
/// ★Observation method★: the fake holds the outlet weakly; the tasks hold it strongly. If the stuck dispatch were detached
/// rather than aborted, the weak reference would stay upgradable forever.
#[tokio::test]
async fn a_write_end_aborts_a_dispatch_stuck_in_a_handler() {
    let fake = Arc::new(FakeHandler {
        close_then_hang_on: Some("die".into()),
        ..FakeHandler::new(None)
    });
    let (mut client, server, _conns) = serve_one(fake.clone(), KeepaliveConfig::default()).await;

    client
        .send(Message::Text("die".to_string().into()))
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("쓰기 쪽이 끝나면 연결 수명도 끝나야")
        .unwrap();
    assert!(
        wait_released(&fake).await,
        "멈춘 dispatch 가 abort 되지 않았다(detach) — 프레임 출구를 쥔 채 살아남는다"
    );
    drop(client);
}

/// ★A read end does not wait forever on a handler that never returns★ — dispatch is aborted after one ping interval.
#[tokio::test]
async fn a_read_end_aborts_a_stuck_dispatch_after_one_ping_interval() {
    let keepalive = KeepaliveConfig {
        ping_interval: Duration::from_millis(500),
        idle_timeout: Duration::from_secs(5),
    };
    let fake = Arc::new(FakeHandler::blocking_on("slow"));
    let (mut client, server, _conns) = serve_one(fake.clone(), keepalive).await;

    client
        .send(Message::Text("slow".to_string().into()))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), fake.text_seen.notified())
        .await
        .expect("명령이 핸들러에 닿아야");
    client.close(None).await.unwrap();

    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("읽기 끝 뒤 한 ping 주기 안에 정리가 끝나야")
        .unwrap();
    assert_eq!(
        fake.calls().last().map(String::as_str),
        Some("disconnect:1")
    );
    assert!(
        wait_released(&fake).await,
        "멈춘 dispatch 가 abort 되지 않았다"
    );
}

/// The three frame kinds ride the single writer queue in FIFO as they are — the close goes out after the frames put in before it.
///
/// ★The close is issued while frames are still queued★: the client reads nothing until the handler has put the close, and
///   the backlog before it is more than the socket buffers hold — a `Frame::Close` that closed at once would drop the tail.
#[tokio::test]
async fn frames_go_out_in_order_and_the_close_comes_last() {
    let fake = Arc::new(FakeHandler {
        emit_on: Some("emit".into()),
        backlog: BACKLOG_FRAMES,
        ..FakeHandler::new(None)
    });
    let (mut client, server, _conns) = serve_one(fake.clone(), KeepaliveConfig::default()).await;
    client
        .send(Message::Text("emit".to_string().into()))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), fake.emitted.notified())
        .await
        .expect("the handler did not emit");

    let within = Duration::from_secs(5);
    assert_eq!(text_of(next_data(&mut client, within).await), "greeting");
    read_backlog(&mut client, within).await;
    assert_eq!(text_of(next_data(&mut client, within).await), "hi");
    match next_data(&mut client, within).await {
        Some(Message::Binary(b)) => assert_eq!(b.as_ref(), &[1, 2, 3]),
        other => panic!("binary 여야: {other:?}"),
    }
    assert!(
        next_data(&mut client, within).await.is_none(),
        "close 가 마지막이어야"
    );
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("닫힌 연결의 수명이 끝나야")
        .unwrap();
}

/// ★Per-connection saturation closes that connection★ (`conn_port::FrameSink::try_send`) — on the current-thread runtime the
/// writer cannot drain while the handler pushes, so the outbound queue fills deterministically.
#[tokio::test]
async fn outbound_saturation_closes_the_connection() {
    let fake = Arc::new(FakeHandler {
        flood_on: Some(("flood".into(), CONN_TX_CAP + 100)),
        ..FakeHandler::new(None)
    });
    let (mut client, server, _conns) = serve_one(fake.clone(), KeepaliveConfig::default()).await;
    client
        .send(Message::Text("flood".to_string().into()))
        .await
        .unwrap();

    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("포화한 연결의 수명이 끝나야")
        .unwrap();
    assert!(
        fake.flood_refused_at.lock().unwrap().is_some(),
        "송신 큐가 포화하지 않았다 — 이 시험의 무대가 안 섰다"
    );
    let mut closed = false;
    for _ in 0..(CONN_TX_CAP + 200) {
        if next_data(&mut client, Duration::from_secs(5))
            .await
            .is_none()
        {
            closed = true;
            break;
        }
    }
    assert!(closed, "포화한 연결은 닫혀야");
}

/// ★The all-connection fan-out skips a saturated connection and neither stops at it nor closes it★ (`conn_port::FrameFanout`
/// duties 2 · 3 and the asymmetry with `try_send`).
///
/// ★Deterministic★: the registry visits connections in id order, and the saturated one is id 1 — a fan-out that stopped at the
///   first failure would starve the other two in every run. The outbound queue is filled on the current-thread runtime with no await in
///   between, so no writer drains it before the broadcast.
#[tokio::test]
async fn the_fanout_skips_a_saturated_connection_without_closing_it() {
    const PAYLOAD: &str = "opaque-fanout-payload";
    let fake = Arc::new(FakeHandler::new(None));
    let (listening, server, conns) =
        test_server(fake, TEST_WIRE_VERSION, KeepaliveConfig::default());
    let port = listening.port();
    let run = tokio::spawn(server.run(listening, std::future::pending::<()>()));

    let mut clients = Vec::new();
    for _ in 0..3 {
        let mut client = connect(port).await;
        client.send(auth_message(TEST_WIRE_VERSION)).await.unwrap();
        assert_eq!(
            text_of(next_data(&mut client, Duration::from_secs(5)).await),
            "greeting"
        );
        clients.push(client);
    }
    for id in 1..=3 {
        assert!(conns.contains(id), "conn {id} 이 등록돼야");
    }

    let saturated = conns.outlet(1).expect("conn 1");
    let mut filled = 0;
    while saturated
        .peer
        .notify_in(saturated.generation, RawOut::Text("filler".into()))
        .is_ok()
    {
        filled += 1;
    }
    conns.broadcast_text(PAYLOAD.to_string());

    for (n, client) in clients.iter_mut().enumerate().skip(1) {
        assert_eq!(
            text_of(next_data(client, Duration::from_secs(5)).await),
            PAYLOAD,
            "ok{n}: 멀쩡한 연결이 프레임을 못 받았다(fanout 이 첫 실패에서 멈춘 회귀)"
        );
    }
    let slow = &mut clients[0];
    for _ in 0..filled {
        assert_eq!(
            text_of(next_data(slow, Duration::from_secs(5)).await),
            "filler"
        );
    }
    match tokio::time::timeout(Duration::from_millis(300), slow.next()).await {
        Err(_) => {}
        Ok(other) => panic!("포화 연결은 이번 건을 못 받고 닫히지도 않아야: {other:?}"),
    }
    run.abort();
}

/// A handler's `ConnFlow::Close` closes the connection **after** what it queued went out — the stop `Ack` reaches the client.
///
/// ★The close is issued while frames are still queued★: the client reads nothing until the connection's cleanup ran (the
///   close comes before it), and the backlog before the `ack` is more than the socket buffers hold — a close that did not
///   wait for the queue would drop the tail and the `ack` with it.
#[tokio::test]
async fn a_close_from_dispatch_flushes_the_last_reply_first() {
    let fake = Arc::new(FakeHandler {
        ack_close_on: Some("stop".into()),
        backlog: BACKLOG_FRAMES,
        ..FakeHandler::new(None)
    });
    let (mut client, server, _conns) = serve_one(fake.clone(), KeepaliveConfig::default()).await;
    client
        .send(Message::Text("stop".to_string().into()))
        .await
        .unwrap();
    wait_disconnected(&fake).await;

    let within = Duration::from_secs(5);
    assert_eq!(text_of(next_data(&mut client, within).await), "greeting");
    read_backlog(&mut client, within).await;
    assert_eq!(text_of(next_data(&mut client, within).await), "ack");
    assert!(
        next_data(&mut client, within).await.is_none(),
        "답 뒤에 닫혀야"
    );
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("연결 수명이 끝나야")
        .unwrap();
}

/// The listener applies the Origin allowlist ([`ALLOWED_ORIGINS`]): a listed Origin and no Origin pass, any other Origin gets 403.
#[tokio::test]
async fn the_listener_applies_the_net_origin_list() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue;

    let listening = Listening::bind_loopback().expect("bind");
    let port = listening.port();
    let Listening {
        mut listener,
        refusals,
        clock: _,
    } = listening;
    let accepting = tokio::spawn(async move {
        let _refusals = refusals;
        let mut kept = Vec::new();
        while let Ok(accepted) = listener.accept().await {
            kept.push(accepted);
        }
    });

    let upgrade = |origin: Option<&'static str>| async move {
        let mut request = format!("ws://127.0.0.1:{port}/")
            .into_client_request()
            .unwrap();
        if let Some(origin) = origin {
            request
                .headers_mut()
                .insert("origin", HeaderValue::from_static(origin));
        }
        tokio_tungstenite::connect_async(request).await.map(|_| ())
    };
    for origin in ALLOWED_ORIGINS {
        assert!(upgrade(Some(origin)).await.is_ok(), "{origin}");
    }
    assert!(upgrade(None).await.is_ok(), "Origin 없음은 허용");
    assert!(
        upgrade(Some("http://evil.example.com")).await.is_err(),
        "목록 밖 Origin 은 거부"
    );
    accepting.abort();
}
