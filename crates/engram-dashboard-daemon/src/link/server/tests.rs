//! Tests of the daemon's transport adapter.
//!
//! Two harnesses: synthetic inbox items drive the read · dispatch · bypass tasks without a socket (ADR-0129), and a real loopback listener drives the whole connection life (`Server::serve`) with a tokio-tungstenite client.

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
    /// When on, every `try_send`/`send` fails — makes 「the outbound queue is saturated too」 without a time limit.
    refuse: AtomicBool,
}

impl FakeFrameSink {
    /// An outlet that accepts no frame (a connection whose outbound queue is saturated or closed).
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
    /// Only the payload length — content is not this seam's concern.
    Binary(ConnId, usize),
    Disconnect(ConnId),
}

struct FakeHandler {
    calls: Mutex<Vec<HandlerCall>>,
    /// On receiving this text it returns `ConnFlow::Close` (for verifying exit from the receive loop).
    close_on: Option<String>,
    /// On receiving this text it puts `Frame::Close` in the queue and then waits on [`Self::slow_gate`] — the link ends on the write side while dispatch is stuck inside the handler.
    close_then_hang_on: Option<String>,
    /// On receiving this text it puts `Text("hi")` · `Binary([1, 2, 3])` · `Close` in that order and returns Continue.
    emit_on: Option<String>,
    /// On receiving this text it puts `Text("ack")` and returns `ConnFlow::Close`.
    ack_close_on: Option<String>,
    /// [`backlog_frame`]s that `emit_on` · `ack_close_on` put before their own frames.
    backlog: usize,
    /// Notified once `emit_on` has put all its frames, the close included.
    emitted: Arc<Notify>,
    /// On receiving this text it `try_send`s this many frames without yielding; records the index of the first refusal.
    flood_on: Option<(String, usize)>,
    flood_refused_at: Mutex<Option<usize>>,
    /// After `on_connect` puts in the greeting frame, it waits here; meanwhile the test feeds commands and checks none is processed (the read task is not started yet).
    connect_gate: Option<Arc<Notify>>,
    /// Signals that 1 `on_text` has started, so the test proceeds regardless of client close timing.
    text_seen: Arc<Notify>,
    /// On receiving this text it waits inside `on_text` until [`Self::slow_gate`] opens.
    /// ★Avoids timing assumptions★: a sleep would close the window on a slow runner and give a false positive. The gate stays closed until the test opens it.
    slow_on: Option<String>,
    slow_gate: Arc<Notify>,
    /// The priority-lane version of [`Self::slow_on`]. Only by locking it separately from the arrival-order lane can 「arrival order is finished but the priority lane is still held by one item」 be created.
    bypass_gate_on: Option<String>,
    bypass_gate: Arc<Notify>,
    /// Weak reference to the frame outlet `on_connect` received. ★This fake must never keep a strong reference★ — one stray `Arc` makes upgrade always succeed and the abort tests always fail on correct code.
    frames_weak: Mutex<Option<std::sync::Weak<dyn FrameSink>>>,
    /// Whether this connection was still in the fan-out registry at `on_disconnect` (cleanup order).
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

    /// A variant where `on_text` does not return until `slow_gate` is opened.
    fn blocking_on(slow_on: &str) -> Self {
        Self {
            slow_on: Some(slow_on.to_string()),
            ..Self::new(None)
        }
    }

    /// A variant locking the two lanes each — the arrival-order lane with `slow_on`, the priority lane with `bypass_gate_on`.
    fn blocking_on_both(slow_on: &str, bypass_gate_on: &str) -> Self {
        Self {
            bypass_gate_on: Some(bypass_gate_on.to_string()),
            ..Self::blocking_on(slow_on)
        }
    }

    /// For connection-task order verification — peeks into the registry and catches on_connect with a gate.
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

    /// In correct code no frame of this connection is processed before `on_connect` finishes (its read task does not exist yet).
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
            "a frame was processed before on_connect({at}) — read_task was spawned first: {calls:?}"
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
            self.assert_nothing_processed_yet(conn_id, "before entering the gate");
            let _ = frames.send(Frame::Text("greeting".into())).await;
            if let Some(gate) = &self.connect_gate {
                gate.notified().await;
            }
            // ★Assertion after the gate★: during the window until the gate opens (commands fed in advance), it catches a wrongly spawned read task processing a command.
            self.assert_nothing_processed_yet(conn_id, "after passing the gate");
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
                let _ = frames.try_send(Frame::Close("test: the write side ends first".into()));
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

    /// This fake skips the lane only for text starting with "bypass:". It refuses the rest and records the refusal (to tell it from silent loss), splitting into [`Saturated::Unanswered`] by reply result like the production handler.
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

/// Runs the read layer and processing layer together in the actual arrangement (chained through the inbound queue) — a minimal reproduction of the connection task's wiring; returns after both finish.
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
    // Same disposition as the connection task's "dispatch finished first" branch.
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
        "the frame on_connect pushed goes out through the single outlet"
    );
}

/// When the processing layer receives `ConnFlow::Close` it stops on the spot; frames already queued behind it are not processed.
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
        "on ConnFlow::Close processing stops on the spot (later ones do not run even if queued)"
    );
}

/// ★Regression net for socket head-of-line blocking★ — while one handler call runs, input · kill · another activation for other agents sent by the same client must still be taken out.
///
/// ★Measures two things at once★: ① while the handler is held by one command, the rest of the items are all read ② processing is still one at a time in arrival order. ① alone misses a regression that "got faster by dropping order"; ② alone stays green with the original defect.
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
        .expect("the first command must reach the handler");

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
        "the remaining frames were not taken out while the handler was held by the first command — head-of-line blocking regression"
    );
    assert_eq!(
        fake.calls(),
        vec!["text:5:slow"],
        "even if reading runs ahead, processing is one at a time — the next must not start before the previous call returns"
    );

    fake.slow_gate.notify_one();
    tokio::time::timeout(Duration::from_secs(10), dispatch)
        .await
        .expect("once the gate opens, the processing row must end")
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
        "even through the queue, the processing order stays the arrival order"
    );
}

/// ★If the inbound lane has no room for cancellation, a `Kill` starves behind it the moment the lane fills★.
///
/// ★The point is not 「there is a priority lane」 but 「reading does not stop」★: `Kill` is placed as a frame arriving after the lane is full, with one more ordinary command before it to check that the loop disposed of that one and kept reading.
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
    // ★Main point★ — two frames arriving while the lane has no room.
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
        "the cancel did not pass while the lane was full — no-cancel-room regression: {:?}",
        fake.calls()
    );
    // ★Measures the set, not the order★ — the two lanes are different tasks.
    let ran: Vec<String> = fake.calls();
    assert!(
        ran.iter()
            .all(|c| c == "text:11:slow" || c == "text:11:bypass:kill"),
        "something other than the cancel skipped the lane (or the blocked lane ran): {ran:?}"
    );
    // ★The overflowed ordinary command does not vanish silently★ — this fake's refusal reply is the evidence.
    assert!(
        sink.frames()
            .iter()
            .any(|f| f == "text:refused:overflow-ordered"),
        "the overflowing command was discarded without a reply: {:?}",
        sink.frames()
    );

    fake.slow_gate.notify_one();
    let _ = tokio::time::timeout(Duration::from_secs(10), dispatch).await;
    read.abort();
    bypass.abort();
}

/// ★Entries remain but the bytes fill first★ — regression net for [`CONN_RX_MAX_BYTES`].
///
/// ★Why two large frames★: the decision uses the fill level before adding, so one frame arriving at an empty lane always goes in. What exceeds the budget is from the next frame on.
#[tokio::test]
async fn the_inbound_row_is_bounded_by_bytes_not_only_by_slots() {
    let sink = Arc::new(FakeFrameSink::default());
    let frames: Arc<dyn FrameSink> = sink.clone();
    let fake = Arc::new(FakeHandler::blocking_on("slow"));
    let handler: Arc<dyn ConnectionHandler> = fake.clone();

    // Two frames each one byte over half the budget — two exceed it, using only three of the 64 entries.
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

    // The read loop finishes after scanning all items (it does not stop on exceeding the budget).
    tokio::time::timeout(Duration::from_secs(10), read)
        .await
        .expect("the read loop must not stop at a budget overrun")
        .unwrap();

    assert!(
        sink.frames()
            .iter()
            .any(|f| f == "text:refused:after-budget"),
        "let it through because entries were left — the byte budget did not apply: {:?}",
        sink.frames()
    );
    tokio::time::timeout(Duration::from_secs(5), fake.text_seen.notified())
        .await
        .expect("the consumer must pick up the first frame");
    assert_eq!(
        fake.calls(),
        vec!["text:21:slow"],
        "the consumer must still be held by the first frame (if the lane empties, this test's premise collapses)"
    );

    fake.slow_gate.notify_one();
    let _ = tokio::time::timeout(Duration::from_secs(10), dispatch).await;
}

/// ★If the refusal cannot be sent as a reply, feeding ends — and the inbox is still drained to its end★ — regression net for [`Saturated::Unanswered`].
///
/// ★Catches★: the read layer treating `Unanswered` like `Refused` and continuing to ask for dispositions. Observation = how many times the upper layer was asked: once if feeding stopped, three times (trigger + two tails) if not.
/// ★The inbox is not left undrained★ — every item is still taken out (and discarded), so the transport's cleanup is not held by this connection (transport receiver invariant 5).
/// ★Do not start the consumer★: with only the receiving end kept alive, the lane fills at exactly `CONN_RX_CAP` frames, so the decision is deterministic.
#[tokio::test]
async fn a_refusal_that_could_not_be_sent_stops_feeding_but_drains_the_inbox() {
    // An outlet accepting no frame = the outbound queue is saturated too.
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

    // ★Keep the receiving end alive★ — dropped, the first `try_send` is cut by Closed and the test would measure "no consumer" rather than saturation.
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
        "read a refusal it could not reply to as 'refused' and kept pushing to the queue"
    );
    assert_eq!(
        pulled.load(Ordering::Acquire) as usize,
        total,
        "even after it stops pushing to the queue, the inbox must be drained to the end"
    );
}

/// The read task reports how the link ended — the connection task splits its teardown on it (read end = drain · write end = abort). Items after the end are not consumed.
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
    // An inbox ending without `Closed` = abnormal supervisor end — handled as a write end (abort).
    let (end, _) = end_of(vec![text_item("a")]).await;
    assert_eq!(end.cause, None);
    assert!(!end.read_side());
}

// ── real-socket harness ────────────────────────────────────────────────────────────────

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// An arbitrary protocol version. ★Deliberately not the production version★ — if the server read the constant back, the test would break (proof the injection path exists).
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

/// Serves exactly one connection and returns a client that has sent its auth frame. The join handle ends when that connection's cleanup is done.
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

/// Frames that stay queued while the client does not read: 32 × 256 KiB = 8 MiB, more than the loopback socket buffers hold, so the writer stalls with frames still queued. ★A close that does not wait for the queue (`Peer::close` — the control channel overtakes the data queue) drops them★ — what the two close-order tests below catch.
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
        "both numbers must be based on the injected value (a hardcoded constant would diverge)"
    );
    assert!(
        next_data(&mut client, Duration::from_secs(5))
            .await
            .is_none(),
        "must close after the refusal notice"
    );

    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("on a version mismatch the connection lifetime must end")
        .unwrap();
    assert!(
        !conns.contains(1),
        "a connection that failed auth is not registered"
    );
    assert_eq!(
        fake.calls(),
        Vec::<String>::new(),
        "on auth failure the upper-layer handler is never attached"
    );
}

/// Pins down the orders the connection task keeps:
///   ① `on_connect` comes before starting the read task (a command cannot overtake the greeting)
///   ② `on_disconnect` comes before registry removal
///
/// ★Detection power★: ① is probabilistic (a wrongly early task leaves a trace only if polled inside the gate window — assertions sit before and after the gate to widen it); ② looks directly at the registry inside `on_disconnect`, so it is deterministic. False positives 0.
/// ★No third order (no writer during `on_connect`)★ — the transport's writer runs from the moment the link is live, so the greeting may go out while `on_connect` still runs.
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
        .expect("the command must be processed");
    client.close(None).await.unwrap();

    tokio::time::timeout(Duration::from_secs(10), server_task)
        .await
        .expect("the connection lifetime must end")
        .unwrap();

    assert_eq!(
        fake.calls(),
        vec!["connect:1", "text:1:cmd", "disconnect:1"],
        "connect → frame → disconnect order"
    );
    assert_eq!(
        fake.registered_at_disconnect(),
        Some(true),
        "at on_disconnect it is still in the fanout registry"
    );
    assert!(
        !conns.contains(1),
        "when the connection lifetime ends it is unregistered"
    );
}

/// ★Teardown does not discard an accepted priority frame★ — regression net for [`finish_bypass_lane`].
///
/// ★Stage★: lock the two lanes separately. The arrival-order lane is held at a gate to fill the inbound queue (so later frames go to the priority lane), and the priority lane is locked at the first item so the second stays queued. Releasing the arrival-order lane and closing the socket starts teardown with one priority frame accepted and not yet run.
/// ★Detection range★: the 300ms window only checks whether the buggy version finishes cleanup within it (correct code waits for the priority lane — no false positives). The assertion after opening the gate is deterministic in both branches.
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
        .expect("the first command must reach the handler");

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
        "the priority lane did not pick up the first frame — this test's stage was not set: {:?}",
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
            .expect("once the priority lane is empty, cleanup must end")
            .unwrap();
    }

    assert!(
        fake.calls().iter().any(|c| c == "text:1:bypass:second"),
        "a held priority frame vanished with teardown's abort (ended inside the window = {finished_before_the_gate}): {:?}",
        fake.calls()
    );
    drop(client);
}

/// ★A write end aborts a dispatch stuck inside a handler★ — not detached. The handler puts `Frame::Close` and then hangs: the link ends on the write side while dispatch holds the frame outlet. Only an abort releases it.
///
/// ★Observation★: the fake holds the outlet weakly and the tasks strongly; a detached (not aborted) stuck dispatch would leave the weak reference upgradable forever.
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
        .expect("once the write side ends, the connection lifetime must end too")
        .unwrap();
    assert!(
        wait_released(&fake).await,
        "the stalled dispatch was not aborted (detach) — it survives holding the frame outlet"
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
        .expect("the command must reach the handler");
    client.close(None).await.unwrap();

    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("cleanup must end within one ping period after the read end")
        .unwrap();
    assert_eq!(
        fake.calls().last().map(String::as_str),
        Some("disconnect:1")
    );
    assert!(
        wait_released(&fake).await,
        "the stalled dispatch was not aborted"
    );
}

/// The three frame kinds ride the single writer queue in FIFO — the close goes out after earlier frames.
///
/// ★The close is issued while frames are still queued★: the client reads nothing until the handler has put the close, and the backlog before it exceeds the socket buffers — a `Frame::Close` that closed at once would drop the tail.
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
        other => panic!("must be binary: {other:?}"),
    }
    assert!(
        next_data(&mut client, within).await.is_none(),
        "close must be last"
    );
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("the closed connection's lifetime must end")
        .unwrap();
}

/// ★Per-connection saturation closes that connection★ (`conn_port::FrameSink::try_send`) — on the current-thread runtime the writer cannot drain while the handler pushes, so the outbound queue fills deterministically.
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
        .expect("the saturated connection's lifetime must end")
        .unwrap();
    assert!(
        fake.flood_refused_at.lock().unwrap().is_some(),
        "the send queue did not saturate — this test's stage was not set"
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
    assert!(closed, "a saturated connection must close");
}

/// ★The all-connection fan-out skips a saturated connection and neither stops at it nor closes it★ (`conn_port::FrameFanout` duties 2 · 3 and the asymmetry with `try_send`).
///
/// ★Deterministic★: the registry visits connections in id order and the saturated one is id 1 — a fan-out stopping at the first failure would starve the other two every run. The queue is filled on the current-thread runtime with no await in between, so no writer drains it before the broadcast.
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
        assert!(conns.contains(id), "conn {id} must be registered");
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
            "ok{n}: a healthy connection did not get the frame (regression where fanout stops at the first failure)"
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
        Ok(other) => {
            panic!("the saturated connection must neither get this one nor be closed: {other:?}")
        }
    }
    run.abort();
}

/// A handler's `ConnFlow::Close` closes the connection after what it queued went out — the stop `Ack` reaches the client.
///
/// ★The close is issued while frames are still queued★: the client reads nothing until cleanup ran (the close comes before it), and the backlog before the `ack` exceeds the socket buffers — a close that did not wait for the queue would drop the tail and the `ack`.
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
        "must close after the reply"
    );
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("the connection lifetime must end")
        .unwrap();
}

/// The listener applies the Origin allowlist ([`ALLOWED_ORIGINS`]): a listed Origin and no Origin pass, any other gets 403.
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
    assert!(upgrade(None).await.is_ok(), "no Origin is allowed");
    assert!(
        upgrade(Some("http://evil.example.com")).await.is_err(),
        "an Origin outside the list is refused"
    );
    accepting.abort();
}
