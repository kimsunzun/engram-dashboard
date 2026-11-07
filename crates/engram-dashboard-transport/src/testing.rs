//! Harness only — lives behind the feature flag `test-support` (ADR-0012, precedent = `engram-dashboard-command`).
//!
//! ★This is the real thing behind 「백오프·keepalive·쓰기 시한을 **실제로 기다리지 않는다**」 (never actually waiting for backoff, keepalive or write deadlines)★
//! (ADR-0177 decision 8): [`ManualClock`] stands in for time, [`MemoryNetwork`] and [`MemoryListener`] for the transport.
//! ★Their far ends differ★ — the far end of [`MemoryNetwork`] is a script the test drives by hand ([`MemoryEndpoint`]),
//! and the far end of [`MemoryListener`] is a real [`LinkTx`]/[`LinkRx`] pair ([`Accepted`]), so it joins two supervisors to each other.
//!
//! ★**Only [`settle`] and [`settle_until`] are for the current_thread runtime**★ — they synchronize by cooperative yielding alone,
//! so under `#[tokio::test(flavor = "multi_thread")]` they guarantee nothing. Breaking that gives a silently
//! flaky test, so both **look at the runtime kind themselves and panic.** Test helpers built on them (like `Near::arrived`
//! in the `registry.rs` tests, which polls once without waiting) share the same premise. The in-memory parts ([`MemoryNetwork`],
//! [`memory_link`], [`ManualClock`] ...) and [`PermitPool`] also work under multi_thread — the ends are
//! observed through waits bounded by a real-time deadline (the 3-0 U6b test in `registry.rs`).
//!
//! ★Production code must not reference this module★ — it is absent from the default build, so compilation breaks first.
//!
//! ★Consumers turn `test-support` on in `[dev-dependencies]` only★ — in `[dependencies]` this module would land in their
//! release binary. CI checks it on the consumer: `.github/workflows/ci.yml` `shell gate 5` (shell; the daemon line comes
//! with TRD 3 §6-3 U4).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use tokio::sync::{mpsc, oneshot, watch};

use crate::clock::Clock;
use crate::frame::{Close, Frame};
use crate::link::{
    Accepted, Address, DialFailure, Dialer, Handshake, HandshakeStep, LinkError, LinkInfo,
    LinkRead, LinkRx, LinkTx, Listener, PermitPool, RejectKind,
};
use crate::stream::StreamMark;
use crate::wire::{DecodeClass, Wire};

// ── 시간 ──────────────────────────────────────────────────────────────────────

struct ManualState {
    now: Instant,
    waiters: Vec<(Instant, oneshot::Sender<()>)>,
}

impl ManualState {
    /// Drops the sleeper whose receiving side is gone.
    ///
    /// ★Without this `waiters` grows without bound★ — each turn of the supervisor's production loop creates a new `sleep`
    /// and all but the winner are simply dropped (one piles up per traffic event).
    fn prune(&mut self) {
        self.waiters.retain(|(_, tx)| !tx.is_closed());
    }
}

/// A clock pushed by hand. ★It does not flow on its own★ — only [`ManualClock::advance`] moves the time.
#[derive(Clone)]
pub struct ManualClock {
    inner: Arc<Mutex<ManualState>>,
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl ManualClock {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ManualState {
                now: Instant::now(),
                waiters: Vec::new(),
            })),
        }
    }

    /// A handle to plug in where an `Arc<dyn Clock>` goes. It sees the same time as the original.
    pub fn handle(&self) -> Arc<dyn Clock> {
        Arc::new(self.clone())
    }

    /// Moves the time and wakes the sleepers whose deadline has passed **in deadline order**.
    ///
    /// ★The order is the contract★ — if one `advance` crosses two deadlines and wakes them in registration order, the result is an order
    /// real time can never produce, and some test passes only under that order.
    pub fn advance(&self, d: Duration) {
        let due = {
            let mut st = self.inner.lock().unwrap();
            st.prune();
            st.now += d;
            let now = st.now;
            let (mut due, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut st.waiters)
                .into_iter()
                .partition(|(deadline, _)| *deadline <= now);
            st.waiters = rest;
            due.sort_by_key(|(deadline, _)| *deadline);
            due
        };
        for (_, tx) in due {
            let _ = tx.send(());
        }
    }

    /// The number currently asleep **and alive**. Dropped timers are not counted.
    pub fn sleepers(&self) -> usize {
        let mut st = self.inner.lock().unwrap();
        st.prune();
        st.waiters.len()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        self.inner.lock().unwrap().now
    }

    fn sleep(&self, d: Duration) -> BoxFuture<'static, ()> {
        if d.is_zero() {
            return Box::pin(async {});
        }
        let (tx, rx) = oneshot::channel();
        {
            let mut st = self.inner.lock().unwrap();
            st.prune();
            let deadline = st.now + d;
            st.waiters.push((deadline, tx));
        }
        Box::pin(async move {
            let _ = rx.await;
        })
    }
}

/// The number of yields [`settle`] runs in one call.
const SETTLE_YIELDS: usize = 512;

fn assert_single_threaded() {
    // ★On a multithreaded runtime yielding is not synchronization★ — every assertion pattern in this module
    //   silently turns flaky there. A loud failure beats a silent flake.
    let flavor = tokio::runtime::Handle::current().runtime_flavor();
    assert_eq!(
        flavor,
        tokio::runtime::RuntimeFlavor::CurrentThread,
        "engram-dashboard-transport 의 하네스는 current_thread 런타임 전용이다 \
         (#[tokio::test] 기본값). multi_thread 아래에서는 settle() 이 아무것도 보장하지 않는다"
    );
}

/// Lets the supervisor and reader tasks run as far as they can under the manual clock.
///
/// ★It does not sleep in real time★ — it only repeats `yield_now`. If you know the condition, [`settle_until`] is better:
/// this function **does not notice that things are not finished**, and the failure surfaces at some unrelated assertion.
pub async fn settle() {
    assert_single_threaded();
    for _ in 0..SETTLE_YIELDS {
        tokio::task::yield_now().await;
    }
}

/// Yields until the condition holds. ★Past the limit it panics on the spot★ — the reason this function exists is to avoid
/// disguising 「안 끝났다」 (not finished yet) as 「단언이 틀렸다」 (the assertion is wrong).
pub async fn settle_until(what: &str, mut done: impl FnMut() -> bool) {
    assert_single_threaded();
    for _ in 0..SETTLE_YIELDS {
        if done() {
            return;
        }
        tokio::task::yield_now().await;
    }
    if !done() {
        panic!("{SETTLE_YIELDS}번 양보하고도 정착하지 않았다: {what}");
    }
}

// ── 전송 ──────────────────────────────────────────────────────────────────────

/// What the client sent down the link. Besides frames, **keepalive and close** are also observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMsg {
    Frame(Frame),
    Ping,
    Close(Close),
}

#[derive(Debug)]
enum ServerMsg {
    Frame(Frame),
    Closed(Option<Close>),
    Fail(LinkError),
}

/// The other side of the link. The test pushes frames in through it and sees what the client sent.
pub struct MemoryEndpoint {
    to_client: mpsc::UnboundedSender<ServerMsg>,
    /// The number pushed in but not yet read by the client — held together with `MemRx`.
    unread: Arc<AtomicUsize>,
    from_client: Mutex<mpsc::UnboundedReceiver<ClientMsg>>,
    stall_tx: watch::Sender<bool>,
    ping_stall_tx: watch::Sender<bool>,
}

impl MemoryEndpoint {
    /// One item the client sent. `None` if there is none.
    pub fn take(&self) -> Option<ClientMsg> {
        self.from_client.lock().unwrap().try_recv().ok()
    }

    /// Everything the client sent.
    pub fn drain(&self) -> Vec<ClientMsg> {
        let mut rx = self.from_client.lock().unwrap();
        let mut out = Vec::new();
        while let Ok(m) = rx.try_recv() {
            out.push(m);
        }
        out
    }

    /// Only the **frames** the client sent.
    pub fn frames(&self) -> Vec<Frame> {
        self.drain()
            .into_iter()
            .filter_map(|m| match m {
                ClientMsg::Frame(f) => Some(f),
                _ => None,
            })
            .collect()
    }

    /// Sends the client one chunk.
    pub fn push(&self, frame: Frame) {
        self.send(ServerMsg::Frame(frame));
    }

    /// The number of items (frames, closes, failures) pushed in but not yet read by the client. ★This is how to see whether
    /// backpressure reached the link★ — if the supervisor does not drain the front queue or the bytes it holds exceed the read share, the read task does not read the link and
    /// the items stay here.
    pub fn unread(&self) -> usize {
        self.unread.load(Ordering::Acquire)
    }

    /// ★Count before sending★ — if counted after, the reader may take it out in between and the count wraps below 0.
    fn send(&self, msg: ServerMsg) {
        self.unread.fetch_add(1, Ordering::AcqRel);
        if self.to_client.send(msg).is_err() {
            self.unread.fetch_sub(1, Ordering::AcqRel);
        }
    }

    /// ★Says it closes **without** a code★ — this is `LinkRead::Closed(None)`, and by contract it too means 「상대가 닫았다고
    /// 말했다」 (the peer said it closed). (Vanishing without a word is the [`MemoryEndpoint::fail`] side = `Err`.) The client learns of the close but
    /// gets no reason.
    ///
    /// ★**Simply dropping this link** (dropping the endpoint) is not this branch★ — that is vanishing
    /// without a word, so `MemRx` reports it as a read error (same shape as a real socket). To make 「닫았다고 말했다」 (said it closed), call this or
    /// [`MemoryEndpoint::reject`].
    pub fn close(&self) {
        self.send(ServerMsg::Closed(None));
    }

    /// ★Closes with a code and a message★ — the only path where the client can recognize 「붙었는데 거절당했다」 (connected but rejected).
    pub fn reject(&self, close: Close) {
        self.send(ServerMsg::Closed(Some(close)));
    }

    /// Cuts with an error. ★It reaches the reading side only★ — the client's next read is an error (writes still go out).
    pub fn fail(&self, why: &str) {
        self.send(ServerMsg::Fail(LinkError::new(why)));
    }

    /// ★Cuts the writing side only★ — the client's next `send` or `ping` ends in an error (after release, if a stall is set) and reads
    /// stay alive. The counterpart of [`MemoryEndpoint::fail`] — dropping the endpoint cuts both sides together, so which one
    /// is reported first depends on scheduling. What was already sent can still be taken out with [`MemoryEndpoint::take`].
    pub fn fail_writes(&self) {
        self.from_client.lock().unwrap().close();
    }

    /// When on, the client's writes stall — **all three**: `send`, `ping` and `close` (on a real socket those three
    /// use one link). ★When off, it **really wakes up**★ — if it blocked with an unwakeable `pending()`, the stall would be decoration,
    /// and a test that passed on it measured nothing.
    ///
    /// ★A test where the link is dropped while blocked must account for the pending close★ — if the close in `drop_link`
    /// stalls here, the later steps (backoff wait, loss report) never start. There are two ways to release it: turn this
    /// off, or push the clock `write_deadline` further so the deadline cuts that close.
    ///
    /// ★The 「상대가 안 읽을 때」 (when the peer does not read) case, which needs a kernel buffer to measure, cannot be measured with this★ — that belongs to a real socket,
    /// and what is measured here is **our side's deadline and queue logic**.
    pub fn stall_writes(&self, on: bool) {
        let _ = self.stall_tx.send(on);
    }

    /// Whether writes are blocked right now.
    pub fn writes_stalled(&self) -> bool {
        *self.stall_tx.borrow()
    }

    /// When on, **only** the client's `ping` stalls — it stacks with [`MemoryEndpoint::stall_writes`] and really wakes up when off.
    /// ★This shape does not exist on a real socket★ (ping goes out the same link) — it is here only to build 「앞선 쓰기는 끝났고 그 직후 타이머 바퀴의 ping 에서 선다」 (the earlier write
    /// finished, then it stops at the ping of the timer turn right after). One handle cannot tell the two apart: the moment the stall is released the supervisor
    /// pushes on to the next write without yielding.
    pub fn stall_pings(&self, on: bool) {
        let _ = self.ping_stall_tx.send(on);
    }
}

struct MemTx {
    to_server: mpsc::UnboundedSender<ClientMsg>,
    stall_rx: watch::Receiver<bool>,
    ping_stall_rx: watch::Receiver<bool>,
}

impl MemTx {
    fn put(&self, msg: ClientMsg) -> Result<(), LinkError> {
        self.to_server
            .send(msg)
            .map_err(|_| LinkError::new("peer endpoint gone"))
    }

    async fn wait_writable(&mut self) {
        let _ = self.stall_rx.wait_for(|stalled| !*stalled).await;
    }
}

impl LinkTx for MemTx {
    fn send(&mut self, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(async move {
            self.wait_writable().await;
            self.put(ClientMsg::Frame(frame))
        })
    }

    fn ping(&mut self) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(async move {
            self.wait_writable().await;
            let _ = self.ping_stall_rx.wait_for(|stalled| !*stalled).await;
            self.put(ClientMsg::Ping)
        })
    }

    /// ★Close goes out over the **same link** as `send`/`ping`, so it is blocked together with them★ — on a real socket the close frame
    /// queues behind the rest of the truncated frame, and that is where the supervisor's `drop_link` spends
    /// `write_deadline`. ★Do not delete this `wait_writable`★ — while it was missing the harness could not express 「이미 막힌 쓰기 뒤에 닫기가 걸린다」
    /// (a close queued behind an already blocked write), so the defect that `drop_link` cannot hear control meanwhile
    /// stayed unexamined (2026-09-07).
    fn close(&mut self, close: Close) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.wait_writable().await;
            let _ = self.put(ClientMsg::Close(close));
        })
    }
}

struct MemRx {
    from_server: mpsc::UnboundedReceiver<ServerMsg>,
    unread: Arc<AtomicUsize>,
    /// Reading has already failed — the real thing behind the [`LinkRx::recv`] contract (no close is emitted after a failure).
    failed: bool,
}

impl LinkRx for MemRx {
    fn recv(&mut self) -> BoxFuture<'_, Result<LinkRead, LinkError>> {
        Box::pin(async move {
            // ★It carries the same contract as the real adapter★ — if the harness emitted `Closed(None)` after a failure, code
            //   that branches on that value would be green here and never taken on a real socket (the canonical source of that trap = the
            //   [`crate::LinkRead::Closed`] rustdoc in `link.rs`).
            if self.failed {
                return Err(LinkError::new("link already failed"));
            }
            let next = self.from_server.recv().await;
            if next.is_some() {
                self.unread.fetch_sub(1, Ordering::AcqRel);
            }
            match next {
                Some(ServerMsg::Frame(f)) => Ok(LinkRead::Frame(f)),
                Some(ServerMsg::Closed(close)) => Ok(LinkRead::Closed(close)),
                // ★Channel exhausted = vanished without a word★ — on a real socket the same event (a socket dropped without a close handshake)
                //   is a read error, so it is an error here too. The old version that raised it as `Ok(Closed(None))`
                //   made the [`crate::LinkRead::Closed`] contract (「상대가 **말했다**」 (the peer said so)) false in the harness alone,
                //   so a test that simulated death by dropping the endpoint was green as `PeerClosed` while on a real socket it
                //   went to a link read error (now `ReadFailed`).
                None => {
                    self.failed = true;
                    Err(LinkError::new("peer endpoint gone"))
                }
                Some(ServerMsg::Fail(e)) => {
                    self.failed = true;
                    Err(e)
                }
            }
        })
    }
}

struct NetState {
    dials: usize,
    fail_next: usize,
    fail_forever: bool,
    refuse: Option<Close>,
    opened: VecDeque<Arc<MemoryEndpoint>>,
}

/// In-memory transport. Opens links behind one [`Dialer`] and hands the far side of an opened link to the test.
#[derive(Clone)]
pub struct MemoryNetwork {
    inner: Arc<Mutex<NetState>>,
}

impl Default for MemoryNetwork {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryNetwork {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(NetState {
                dials: 0,
                fail_next: 0,
                fail_forever: false,
                refuse: None,
                opened: VecDeque::new(),
            })),
        }
    }

    pub fn dialer(&self) -> Arc<dyn Dialer> {
        Arc::new(self.clone())
    }

    /// The next `n` dials fail.
    pub fn fail_next(&self, n: usize) {
        self.inner.lock().unwrap().fail_next = n;
    }

    /// From now on every dial fails.
    pub fn fail_forever(&self) {
        self.inner.lock().unwrap().fail_forever = true;
    }

    /// From now on every dial ends in [`DialFailure::Refused`] with no link — imitates a terminal refusal the caller already knows (local version
    /// check). It takes precedence over [`MemoryNetwork::fail_forever`] and [`MemoryNetwork::fail_next`]. `close.code` must be
    /// a terminal refusal code (the contract of that branch).
    pub fn refuse(&self, close: Close) {
        self.inner.lock().unwrap().refuse = Some(close);
    }

    /// Lets it connect again.
    pub fn allow(&self) {
        let mut st = self.inner.lock().unwrap();
        st.fail_forever = false;
        st.fail_next = 0;
        st.refuse = None;
    }

    /// The number of dials **actually polled** so far. A dial that was cancelled and never started is not counted.
    pub fn dials(&self) -> usize {
        self.inner.lock().unwrap().dials
    }

    /// The far side of the oldest link that has 「아직 안 가져간」 (not yet been taken).
    pub fn accept(&self) -> Option<Arc<MemoryEndpoint>> {
        self.inner.lock().unwrap().opened.pop_front()
    }

    /// The far side of the last link opened (earlier ones are discarded).
    pub fn latest(&self) -> Option<Arc<MemoryEndpoint>> {
        let mut st = self.inner.lock().unwrap();
        let last = st.opened.pop_back();
        st.opened.clear();
        last
    }

    /// The number of links nobody has taken yet.
    pub fn open_endpoints(&self) -> usize {
        self.inner.lock().unwrap().opened.len()
    }
}

impl Dialer for MemoryNetwork {
    fn dial(
        &self,
        _addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>> {
        // ★The link is created **inside** the future★ — if created outside, a cancelled dial leaves a link nobody uses in
        //   the registry, and the test picks it up with `accept()` and asserts on a channel the supervisor never adopted.
        let inner = self.inner.clone();
        Box::pin(async move {
            let mut st = inner.lock().unwrap();
            st.dials += 1;
            if let Some(close) = &st.refuse {
                return Err(DialFailure::Refused {
                    code: close.code,
                    reason: close.reason.clone(),
                });
            }
            if st.fail_forever || st.fail_next > 0 {
                st.fail_next = st.fail_next.saturating_sub(1);
                return Err(LinkError::new("connection refused").into());
            }
            let (to_server, from_client) = mpsc::unbounded_channel();
            let (to_client, from_server) = mpsc::unbounded_channel();
            let (stall_tx, stall_rx) = watch::channel(false);
            let (ping_stall_tx, ping_stall_rx) = watch::channel(false);
            let unread = Arc::new(AtomicUsize::new(0));
            st.opened.push_back(Arc::new(MemoryEndpoint {
                to_client,
                unread: unread.clone(),
                from_client: Mutex::new(from_client),
                stall_tx,
                ping_stall_tx,
            }));
            let tx: Box<dyn LinkTx> = Box::new(MemTx {
                to_server,
                stall_rx,
                ping_stall_rx,
            });
            let rx: Box<dyn LinkRx> = Box::new(MemRx {
                from_server,
                unread,
                failed: false,
            });
            Ok((tx, rx))
        })
    }
}

// ── Two-ended link · receiving side ───────────────────────────────────────────

/// What crosses over a two-ended link.
#[derive(Debug)]
enum PipeMsg {
    Frame(Frame),
    /// The receiving end raises it as [`Frame::Keepalive`] and sends the reply (`Pong`) back.
    Ping,
    Pong,
    Closed(Option<Close>),
    /// Put in by [`MemoryLinkControl::fail_reads`].
    Fail(LinkError),
}

/// The state of one end of a two-ended link — held together by that end's two halves and that end's handle.
struct EndState {
    stall: watch::Sender<bool>,
    ping_stall: watch::Sender<bool>,
    writes_failed: AtomicBool,
    /// The number that arrived at this end but has not been read yet.
    unread: AtomicUsize,
    /// The number of live halves (2 at the start). 0 = this end has been dropped entirely.
    halves: AtomicUsize,
    /// This end's incoming queue — where the handle puts in a failure. ★A weak handle★ — if strong, a test holding the handle would keep the queue from draining even after the peer
    /// vanishes, so this end could not see 「말 없이 사라졌다」 (vanished without a word) (a read error).
    inbound: mpsc::WeakUnboundedSender<PipeMsg>,
}

impl EndState {
    fn new(inbound: mpsc::WeakUnboundedSender<PipeMsg>) -> Self {
        Self {
            stall: watch::channel(false).0,
            ping_stall: watch::channel(false).0,
            writes_failed: AtomicBool::new(false),
            unread: AtomicUsize::new(0),
            halves: AtomicUsize::new(2),
            inbound,
        }
    }
}

/// Puts one item into the `dest` end's queue. ★Count before putting in★ — the reason is in `MemoryEndpoint::send`.
fn deliver(dest: &EndState, line: &mpsc::UnboundedSender<PipeMsg>, msg: PipeMsg) -> bool {
    dest.unread.fetch_add(1, Ordering::AcqRel);
    if line.send(msg).is_err() {
        dest.unread.fetch_sub(1, Ordering::AcqRel);
        return false;
    }
    true
}

struct MemLinkTx {
    me: Arc<EndState>,
    peer: Arc<EndState>,
    to_peer: mpsc::UnboundedSender<PipeMsg>,
    stall: watch::Receiver<bool>,
    ping_stall: watch::Receiver<bool>,
}

impl MemLinkTx {
    async fn wait_writable(&mut self) {
        let _ = self.stall.wait_for(|stalled| !*stalled).await;
    }

    /// Returns when the two stalls are **both** released — it re-checks the other each time it wakes. ★Do not turn this back into passing each one
    /// in turn★ — while stopped at the ping stall after passing the write stall, if the write stall gets set, releasing only the ping stall let the ping out (ping also goes out
    /// the write link — [`MemoryLinkControl::stall_writes`]).
    async fn wait_pingable(&mut self) {
        loop {
            self.wait_writable().await;
            let _ = self.ping_stall.wait_for(|stalled| !*stalled).await;
            if !*self.stall.borrow() {
                return;
            }
        }
    }

    /// ★If only the peer end's reading half is dropped, writes succeed★ — a real socket also closes only when both halves of an end are dropped
    /// ([`LinkTx`] rustdoc). What was put in then vanishes because no one can read it.
    fn put(&self, msg: PipeMsg) -> Result<(), LinkError> {
        if self.me.writes_failed.load(Ordering::Acquire) {
            return Err(LinkError::new("writes failed"));
        }
        if deliver(&self.peer, &self.to_peer, msg) || self.peer.halves.load(Ordering::Acquire) > 0 {
            Ok(())
        } else {
            Err(LinkError::new("peer end gone"))
        }
    }
}

impl LinkTx for MemLinkTx {
    /// [`Frame::Keepalive`] goes out like `ping` — matches the WS adapter translating it into a Ping.
    fn send(&mut self, frame: Frame) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(async move {
            self.wait_writable().await;
            self.put(match frame {
                Frame::Keepalive => PipeMsg::Ping,
                other => PipeMsg::Frame(other),
            })
        })
    }

    fn ping(&mut self) -> BoxFuture<'_, Result<(), LinkError>> {
        Box::pin(async move {
            self.wait_pingable().await;
            self.put(PipeMsg::Ping)
        })
    }

    /// Takes the stall like `send` and `ping` — the reason is in `MemTx::close`.
    fn close(&mut self, close: Close) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.wait_writable().await;
            let _ = self.put(PipeMsg::Closed(Some(close)));
        })
    }
}

impl Drop for MemLinkTx {
    fn drop(&mut self) {
        self.me.halves.fetch_sub(1, Ordering::AcqRel);
    }
}

struct MemLinkRx {
    me: Arc<EndState>,
    peer: Arc<EndState>,
    from_peer: mpsc::UnboundedReceiver<PipeMsg>,
    /// Where the reply to a received Ping is sent back — on a real socket too the reply comes from the reading side (`ws.rs` 「들어온 Ping 에 대한 자동
    /// Pong」 (automatic Pong for an incoming Ping)). ★So if this end does not read, the peer's Ping gets no reply★.
    to_peer: mpsc::UnboundedSender<PipeMsg>,
    /// The real thing behind the [`LinkRx::recv`] contract (no close is emitted after a failure) — same as `MemRx`.
    failed: bool,
}

impl LinkRx for MemLinkRx {
    fn recv(&mut self) -> BoxFuture<'_, Result<LinkRead, LinkError>> {
        Box::pin(async move {
            if self.failed {
                return Err(LinkError::new("link already failed"));
            }
            let next = self.from_peer.recv().await;
            if next.is_some() {
                self.me.unread.fetch_sub(1, Ordering::AcqRel);
            }
            match next {
                Some(PipeMsg::Frame(f)) => Ok(LinkRead::Frame(f)),
                Some(PipeMsg::Ping) => {
                    // The reply does not take the stall — on a real socket it goes out late behind a congested send buffer (that shape belongs to a real socket).
                    if !self.me.writes_failed.load(Ordering::Acquire) {
                        deliver(&self.peer, &self.to_peer, PipeMsg::Pong);
                    }
                    Ok(LinkRead::Frame(Frame::Keepalive))
                }
                Some(PipeMsg::Pong) => Ok(LinkRead::Frame(Frame::Keepalive)),
                Some(PipeMsg::Closed(close)) => Ok(LinkRead::Closed(close)),
                // ★Queue exhausted = both halves of the peer end are dropped = vanished without a word★ — a read error, not a close
                //   (the same arm in `MemRx` · the [`crate::LinkRead::Closed`] contract).
                None => {
                    self.failed = true;
                    Err(LinkError::new("peer end gone"))
                }
                Some(PipeMsg::Fail(e)) => {
                    self.failed = true;
                    Err(e)
                }
            }
        })
    }
}

impl Drop for MemLinkRx {
    fn drop(&mut self) {
        self.me.halves.fetch_sub(1, Ordering::AcqRel);
    }
}

/// One end of a two-ended link — two halves and that end's handle.
pub struct MemoryLinkEnd {
    pub tx: Box<dyn LinkTx>,
    pub rx: Box<dyn LinkRx>,
    pub control: MemoryLinkControl,
}

/// A handle that operates one end of a two-ended link — ★it acts on its own end only★. The handle of [`MemoryEndpoint`] is what the far end applies to the client end,
/// so the names differ: its `fail` = [`MemoryLinkControl::fail_reads`] here. The rest have the same names.
///
/// Holding the handle does not keep that end alive — once both halves are dropped the peer sees a read error.
#[derive(Clone)]
pub struct MemoryLinkControl {
    state: Arc<EndState>,
}

impl MemoryLinkControl {
    /// When on, this end's `send`, `ping` and `close` stall, and when off it really wakes up (same contract as [`MemoryEndpoint::stall_writes`]
    /// — the cautions in its rustdoc apply too).
    pub fn stall_writes(&self, on: bool) {
        self.state.stall.send_replace(on);
    }

    pub fn writes_stalled(&self) -> bool {
        *self.state.stall.borrow()
    }

    /// When on, **only** this end's `ping` stalls (same as [`MemoryEndpoint::stall_pings`] — a shape that does not exist on a real socket).
    pub fn stall_pings(&self, on: bool) {
        self.state.ping_stall.send_replace(on);
    }

    /// Cuts this end's reading with an error — it queues behind what has already arrived. Writes still go out.
    pub fn fail_reads(&self, why: &str) {
        if let Some(line) = self.state.inbound.upgrade() {
            deliver(&self.state, &line, PipeMsg::Fail(LinkError::new(why)));
        }
    }

    /// This end's next `send` or `ping` ends in an error (after release, if a stall is set). `close` sends nothing.
    /// Reads stay alive.
    pub fn fail_writes(&self) {
        self.state.writes_failed.store(true, Ordering::Release);
    }

    /// The number of items (frames, replies, closes, failures) that arrived at this end but have not been read yet.
    pub fn unread(&self) -> usize {
        self.state.unread.load(Ordering::Acquire)
    }

    /// Whether both halves of this end have been dropped.
    pub fn dropped(&self) -> bool {
        self.state.halves.load(Ordering::Acquire) == 0
    }
}

/// Builds two ends joined to each other — what one end sends, the other reads.
///
/// ★The queue has no upper bound★ — the shape where a write stops in front of a peer that does not read (kernel buffer) does not exist. That belongs to a real socket.
pub fn memory_link() -> (MemoryLinkEnd, MemoryLinkEnd) {
    let (into_a, a_from) = mpsc::unbounded_channel();
    let (into_b, b_from) = mpsc::unbounded_channel();
    let a = Arc::new(EndState::new(into_a.downgrade()));
    let b = Arc::new(EndState::new(into_b.downgrade()));
    let end = |me: &Arc<EndState>,
               peer: &Arc<EndState>,
               from_peer: mpsc::UnboundedReceiver<PipeMsg>,
               to_peer: mpsc::UnboundedSender<PipeMsg>| MemoryLinkEnd {
        tx: Box::new(MemLinkTx {
            me: me.clone(),
            peer: peer.clone(),
            to_peer: to_peer.clone(),
            stall: me.stall.subscribe(),
            ping_stall: me.ping_stall.subscribe(),
        }),
        rx: Box::new(MemLinkRx {
            me: me.clone(),
            peer: peer.clone(),
            from_peer,
            to_peer,
            failed: false,
        }),
        control: MemoryLinkControl { state: me.clone() },
    };
    (end(&a, &b, a_from, into_b), end(&b, &a, b_from, into_a))
}

/// The two handles of one link opened by [`MemoryConnector`].
#[derive(Clone)]
pub struct MemoryLinkControls {
    /// The dialing side (the end the supervisor holds).
    pub dialer: MemoryLinkControl,
    /// The receiving side (the end carried in [`Accepted`]).
    pub acceptor: MemoryLinkControl,
}

struct ListenLog {
    dials: usize,
    links: Vec<MemoryLinkControls>,
}

struct ListenShared {
    clock: Arc<dyn Clock>,
    permits: PermitPool,
    log: Mutex<ListenLog>,
}

/// In-memory listener — hands out links dialed through [`MemoryConnector`] as [`Accepted`].
///
/// ★The moment of dialing is the end of the upgrade★ — an in-memory link has no upgrade, so the moment dialing finishes the link
/// sits in the ready queue and [`Accepted::upgraded_at`] is that time. The permit ([`MemoryListener::permits`]) is taken at that moment and goes
/// along with [`Accepted`] — if the permits are all used, that dial fails (the overflowing new one is closed — TRD 3 §5 14).
///
/// ★The terminal `Err` of [`Listener::accept`] = every [`MemoryConnector`] is gone and the ready queue is empty★ — there is no way left to dial.
/// Dropping the listener closes the links sitting in the ready queue (the dialing side gets a read error) and later dials fail.
pub struct MemoryListener {
    ready: mpsc::UnboundedReceiver<Accepted>,
    permits: PermitPool,
}

impl MemoryListener {
    /// `clock` = the same clock as the registry the accepted peers are put on. `max_pending_upgrades` = the number of upgrade permits (ready queue + those taken away but
    /// still holding a permit) — the upgrade-wait cap of TRD 3 §5 14. ★It is not the TCP listen backlog★ — it is a permit
    /// count this crate keeps, not the kernel accept queue.
    pub fn new(clock: Arc<dyn Clock>, max_pending_upgrades: usize) -> (Self, MemoryConnector) {
        let (ready_tx, ready) = mpsc::unbounded_channel();
        let permits = PermitPool::new(max_pending_upgrades);
        let connector = MemoryConnector {
            ready: ready_tx,
            shared: Arc::new(ListenShared {
                clock,
                permits: permits.clone(),
                log: Mutex::new(ListenLog {
                    dials: 0,
                    links: Vec::new(),
                }),
            }),
        };
        (Self { ready, permits }, connector)
    }

    /// The upgrade permit pool — the count can still be read after the listener is dropped.
    pub fn permits(&self) -> PermitPool {
        self.permits.clone()
    }

    /// The number sitting in the ready queue not yet taken.
    pub fn queued(&self) -> usize {
        self.ready.len()
    }
}

impl Listener for MemoryListener {
    fn accept(&mut self) -> BoxFuture<'_, Result<Accepted, LinkError>> {
        Box::pin(async move {
            self.ready
                .recv()
                .await
                .ok_or_else(|| LinkError::new("every connector is gone"))
        })
    }
}

/// Takes out both ends of the accepted link — used when a real-socket test drives the receiving side by hand without a supervisor (`tests/ws_*.rs`).
///
/// ★Not a production surface★ — the contract of [`Accepted`] is that both ends never leave the crate, and this path exists only behind `test-support`.
/// The ends taken out have already passed the listener's accept check, so this is not a way to skip that check. The upgrade permit is released here.
pub fn accepted_ends(accepted: Accepted) -> (Box<dyn LinkTx>, Box<dyn LinkRx>) {
    (accepted.tx, accepted.rx)
}

/// A [`Dialer`] that dials through a [`MemoryListener`]. A clone dials the same listener. It does not look at the address.
#[derive(Clone)]
pub struct MemoryConnector {
    ready: mpsc::UnboundedSender<Accepted>,
    shared: Arc<ListenShared>,
}

impl MemoryConnector {
    pub fn dialer(&self) -> Arc<dyn Dialer> {
        Arc::new(self.clone())
    }

    /// The number of dials **actually polled** so far (failures included).
    pub fn dials(&self) -> usize {
        self.shared.log.lock().unwrap().dials
    }

    /// The number of links that reached the ready queue.
    pub fn links(&self) -> usize {
        self.shared.log.lock().unwrap().links.len()
    }

    /// The handle of the `n`th (from 1) link — the [`LinkInfo::remote`] of its [`Accepted`] is `mem:<n>`.
    pub fn link(&self, n: usize) -> Option<MemoryLinkControls> {
        let log = self.shared.log.lock().unwrap();
        n.checked_sub(1).and_then(|i| log.links.get(i).cloned())
    }

    /// The handle of the last link.
    pub fn latest(&self) -> Option<MemoryLinkControls> {
        self.shared.log.lock().unwrap().links.last().cloned()
    }
}

impl Dialer for MemoryConnector {
    fn dial(
        &self,
        _addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>> {
        // ★The link is created **inside** the future★ — the [`Dialer`] contract (a cancelled dial leaves nothing behind).
        let ready = self.ready.clone();
        let shared = self.shared.clone();
        Box::pin(async move {
            // ★Read the injected clock outside the ledger lock★ — if the clock's `now` reads this connector's ledger (`links`, `dials`),
            //   it takes the held lock again and deadlocks. Do not call anything injected while inside a lock.
            let now = shared.clock.now();
            let mut log = shared.log.lock().unwrap();
            log.dials += 1;
            if ready.is_closed() {
                return Err(LinkError::new("listener gone").into());
            }
            let Some(permit) = shared.permits.try_take() else {
                return Err(LinkError::new("upgrade permits full").into());
            };
            let n = log.links.len() + 1;
            let (near, far) = memory_link();
            let info = LinkInfo {
                remote: Some(format!("mem:{n}")),
                ..LinkInfo::default()
            };
            let accepted = Accepted::new(far.tx, far.rx, info, now, permit);
            // On failure the returned `Accepted` is dropped here, and the permit and the far end go back together.
            if ready.send(accepted).is_err() {
                return Err(LinkError::new("listener gone").into());
            }
            log.links.push(MemoryLinkControls {
                dialer: near.control,
                acceptor: far.control,
            });
            Ok((near.tx, near.rx))
        })
    }
}

// ── Handshake ─────────────────────────────────────────────────────────────────

/// Straight to the operating phase with no round trip.
pub struct ImmediateHandshake;

impl Handshake for ImmediateHandshake {
    fn start(&self) -> HandshakeStep {
        HandshakeStep::Done
    }

    fn on_frame(&self, _frame: &Frame) -> HandshakeStep {
        HandshakeStep::Done
    }
}

/// Two steps — send `hello` and on receiving `welcome` go to the operating phase, otherwise reject.
pub struct HelloHandshake;

impl HelloHandshake {
    pub const HELLO: &'static str = "hello";
    pub const WELCOME: &'static str = "welcome";
}

impl Handshake for HelloHandshake {
    fn start(&self) -> HandshakeStep {
        HandshakeStep::Send(Frame::Text(Self::HELLO.into()))
    }

    fn on_frame(&self, frame: &Frame) -> HandshakeStep {
        match frame {
            Frame::Text(t) if t == Self::WELCOME => HandshakeStep::Done,
            other => HandshakeStep::Reject {
                kind: RejectKind::Terminal,
                last: None,
                reason: format!("unexpected handshake frame: {other:?}"),
            },
        }
    }
}

/// Rejects right after connecting — [`RejectingHandshake::new`] is for measuring a terminal one (★the path that does not spend the budget★), [`RejectingHandshake::retryable`] the
/// redial one (the path that spends the budget).
pub struct RejectingHandshake {
    pub kind: RejectKind,
    pub reason: String,
}

impl RejectingHandshake {
    pub fn new(reason: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            kind: RejectKind::Terminal,
            reason: reason.into(),
        })
    }

    pub fn retryable(reason: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            kind: RejectKind::Retryable,
            reason: reason.into(),
        })
    }

    fn reject(&self) -> HandshakeStep {
        HandshakeStep::Reject {
            kind: self.kind,
            last: None,
            reason: self.reason.clone(),
        }
    }
}

impl Handshake for RejectingHandshake {
    fn start(&self) -> HandshakeStep {
        self.reject()
    }

    fn on_frame(&self, _frame: &Frame) -> HandshakeStep {
        self.reject()
    }
}

/// Sends nothing and waits for the peer's first frame — used to measure the path where the peer speaks by **closing**.
pub struct ListeningHandshake;

impl Handshake for ListeningHandshake {
    fn start(&self) -> HandshakeStep {
        HandshakeStep::Await
    }

    fn on_frame(&self, _frame: &Frame) -> HandshakeStep {
        HandshakeStep::Done
    }
}

// ── Fake packet vocabulary ────────────────────────────────────────────────────

/// What the harness sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestOut {
    Request { tag: u64, body: String },
    Notice(String),
    Resume { key: u32, after: Option<u64> },
    Blob(Vec<u8>),
}

/// What the harness receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestIn {
    Reply {
        tag: u64,
        body: String,
    },
    Chunk {
        key: u32,
        generation: u32,
        seq: u64,
        body: String,
    },
    Notice(String),
    /// Reports itself as both a reply and a stream chunk — for observing a [`Wire`] contract violation.
    Ambiguous {
        tag: u64,
        key: u32,
        generation: u32,
        seq: u64,
    },
    Blob(Vec<u8>),
}

/// A minimal packet vocabulary. Text split into fields by `|`, so it round-trips without serde.
pub struct TestWire;

impl TestWire {
    /// Builds a frame for the other side to push to the client.
    pub fn encode_in(inn: &TestIn) -> Frame {
        match inn {
            TestIn::Reply { tag, body } => Frame::Text(format!("rep|{tag}|{body}")),
            TestIn::Chunk {
                key,
                generation,
                seq,
                body,
            } => Frame::Text(format!("chk|{key}|{generation}|{seq}|{body}")),
            TestIn::Notice(body) => Frame::Text(format!("not|{body}")),
            TestIn::Ambiguous {
                tag,
                key,
                generation,
                seq,
            } => Frame::Text(format!("amb|{tag}|{key}|{generation}|{seq}")),
            TestIn::Blob(bytes) => Frame::Binary(bytes.clone()),
        }
    }

    /// A shortcut to build one stream chunk.
    pub fn chunk(key: u32, generation: u32, seq: u64) -> Frame {
        Self::encode_in(&TestIn::Chunk {
            key,
            generation,
            seq,
            body: format!("s{seq}"),
        })
    }

    /// A shortcut to build one reply.
    pub fn reply(tag: u64) -> Frame {
        Self::encode_in(&TestIn::Reply {
            tag,
            body: "ok".into(),
        })
    }
}

/// A decode failure of [`TestWire`]. ★The branches that carry a value quote the received text verbatim★ — material for measuring whether that text leaks
/// into the event ([`DecodeClass`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestDecodeError {
    /// A [`Frame::Keepalive`] reached the decoder — a crate contract violation.
    Keepalive,
    /// Could not read a numeric field.
    BadNumber(String),
    /// Unknown shape.
    Unknown(String),
}

impl DecodeClass for TestDecodeError {
    fn class(&self) -> &'static str {
        match self {
            TestDecodeError::Keepalive => "keepalive",
            TestDecodeError::BadNumber(_) => "bad number",
            TestDecodeError::Unknown(_) => "unknown frame",
        }
    }
}

impl Wire for TestWire {
    type Out = TestOut;
    type In = TestIn;
    type Tag = u64;
    type StreamKey = u32;
    type DecodeError = TestDecodeError;

    fn encode(&self, out: &Self::Out) -> Frame {
        match out {
            TestOut::Request { tag, body } => Frame::Text(format!("req|{tag}|{body}")),
            TestOut::Notice(body) => Frame::Text(format!("not|{body}")),
            TestOut::Resume { key, after } => {
                let after = after.map(|n| n.to_string()).unwrap_or_else(|| "-".into());
                Frame::Text(format!("res|{key}|{after}"))
            }
            TestOut::Blob(bytes) => Frame::Binary(bytes.clone()),
        }
    }

    fn decode(&self, frame: Frame) -> Result<Self::In, Self::DecodeError> {
        let text = match frame {
            Frame::Binary(bytes) => return Ok(TestIn::Blob(bytes)),
            Frame::Keepalive => return Err(TestDecodeError::Keepalive),
            Frame::Text(t) => t,
        };
        let parts: Vec<&str> = text.splitn(5, '|').collect();
        let num = |s: &str| -> Result<u64, TestDecodeError> {
            s.parse::<u64>()
                .map_err(|e| TestDecodeError::BadNumber(format!("{s:?}: {e}")))
        };
        match parts.as_slice() {
            ["rep", tag, body] => Ok(TestIn::Reply {
                tag: num(tag)?,
                body: (*body).to_string(),
            }),
            ["chk", key, generation, seq, body] => Ok(TestIn::Chunk {
                key: num(key)? as u32,
                generation: num(generation)? as u32,
                seq: num(seq)?,
                body: (*body).to_string(),
            }),
            ["not", body] => Ok(TestIn::Notice((*body).to_string())),
            ["amb", tag, key, generation, seq] => Ok(TestIn::Ambiguous {
                tag: num(tag)?,
                key: num(key)? as u32,
                generation: num(generation)? as u32,
                seq: num(seq)?,
            }),
            _ => Err(TestDecodeError::Unknown(text)),
        }
    }

    fn request_tag(&self, out: &Self::Out) -> Option<Self::Tag> {
        match out {
            TestOut::Request { tag, .. } => Some(*tag),
            _ => None,
        }
    }

    fn reply_tag(&self, inn: &Self::In) -> Option<Self::Tag> {
        match inn {
            TestIn::Reply { tag, .. } | TestIn::Ambiguous { tag, .. } => Some(*tag),
            _ => None,
        }
    }

    fn stream_mark(&self, inn: &Self::In) -> Option<StreamMark<Self::StreamKey>> {
        match inn {
            TestIn::Chunk {
                key,
                generation,
                seq,
                ..
            }
            | TestIn::Ambiguous {
                key,
                generation,
                seq,
                ..
            } => Some(StreamMark {
                key: *key,
                generation: *generation,
                seq: *seq,
            }),
            _ => None,
        }
    }

    fn resume_request(&self, key: &Self::StreamKey, after: Option<u64>) -> Self::Out {
        TestOut::Resume { key: *key, after }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Incoming, PeerId, TransportEvent};
    use crate::frame::CloseCode;
    use crate::machine::PeerState;
    use crate::policy::Policy;
    use crate::registry::Registry;
    use futures_util::future::poll_immediate;

    // ── H1: a stalled write must really wake up ──
    //
    // ★Put the stalled work on **another task**★ — if polled by hand on the same task, after releasing the stall we
    //   poll it again ourselves, and then **an implementation that does not register a waker passes too**. The old version written that way
    //   caught none of the counterexample mutants (a `poll_fn` that returns `Pending` during the stall and does not register a waker)
    //   (measured 2026-09-07). With the work on a task, nothing but a wake-up leads to completion.
    #[tokio::test]
    async fn clearing_the_stall_actually_wakes_the_parked_write() {
        let net = MemoryNetwork::new();
        let (mut tx, _rx) = net.dial(&Address::new("mem://x")).await.unwrap();
        let endpoint = net.accept().unwrap();
        endpoint.stall_writes(true);

        let parked = tokio::spawn(async move { tx.send(Frame::Text("payload".into())).await });
        settle().await;
        assert!(!parked.is_finished(), "막혀 있어야 한다");
        assert!(endpoint.frames().is_empty());

        // ★It is this side, not the sleeping task, that releases the stall★ — the only means of waking the other side is the waker.
        endpoint.stall_writes(false);
        // ★Bounded wait★ — if it falls back to an unwakeable stall it must fail cleanly instead of hanging.
        tokio::time::timeout(Duration::from_secs(5), parked)
            .await
            .expect("멈춤을 풀었는데도 쓰기가 안 깨어났다")
            .expect("쓰기 태스크가 패닉했다")
            .unwrap();
        assert_eq!(endpoint.frames(), vec![Frame::Text("payload".into())]);
    }

    // ── H1b: a stalled **close** must really wake up too ──
    #[tokio::test]
    async fn clearing_the_stall_actually_wakes_the_parked_close() {
        let net = MemoryNetwork::new();
        let (mut tx, _rx) = net.dial(&Address::new("mem://x")).await.unwrap();
        let endpoint = net.accept().unwrap();
        endpoint.stall_writes(true);

        let parked = tokio::spawn(async move { tx.close(Close::going_away()).await });
        settle().await;
        assert!(
            !parked.is_finished(),
            "★닫기도 막혀야 한다★ — 안 막히면 하네스가 실소켓의 모양을 표현하지 못한다"
        );
        assert!(endpoint.drain().is_empty());

        endpoint.stall_writes(false);
        // ★Bounded wait★ — if it falls back to an unwakeable stall it must fail cleanly instead of hanging.
        tokio::time::timeout(Duration::from_secs(5), parked)
            .await
            .expect("멈춤을 풀었는데도 닫기가 안 깨어났다")
            .expect("닫기 태스크가 패닉했다");
        assert!(matches!(
            endpoint.drain().as_slice(),
            [ClientMsg::Close(c)] if c.code == crate::frame::CloseCode::GOING_AWAY
        ));
    }

    // ── H2: dropped timers do not pile up, and wake-up order is deadline order ──
    #[tokio::test]
    async fn abandoned_timers_do_not_pile_up() {
        let clock = ManualClock::new();
        for _ in 0..1000 {
            drop(clock.sleep(Duration::from_secs(60)));
        }
        assert_eq!(clock.sleepers(), 0);

        let kept = clock.sleep(Duration::from_secs(60));
        assert_eq!(clock.sleepers(), 1);
        drop(kept);
        assert_eq!(clock.sleepers(), 0);
    }

    #[tokio::test]
    async fn one_advance_wakes_in_deadline_order_not_registration_order() {
        let clock = ManualClock::new();
        let order = Arc::new(Mutex::new(Vec::new()));
        // Register the later deadline first — waking in registration order would reverse them.
        for (label, secs) in [("late", 9u64), ("early", 1), ("middle", 5)] {
            let sleep = clock.sleep(Duration::from_secs(secs));
            let order = order.clone();
            tokio::spawn(async move {
                sleep.await;
                order.lock().unwrap().push(label);
            });
        }
        settle().await;
        clock.advance(Duration::from_secs(10));
        settle().await;
        assert_eq!(*order.lock().unwrap(), vec!["early", "middle", "late"]);
    }

    // ── H3: if it does not settle, it fails loudly ──
    #[tokio::test]
    #[should_panic(expected = "정착하지 않았다")]
    async fn settle_until_says_so_instead_of_failing_somewhere_else() {
        settle_until("절대 참이 안 되는 조건", || false).await;
    }

    #[tokio::test]
    async fn settle_until_returns_as_soon_as_the_condition_holds() {
        let flag = Arc::new(Mutex::new(false));
        let setter = flag.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            *setter.lock().unwrap() = true;
        });
        settle_until("깃발", || *flag.lock().unwrap()).await;
        assert!(*flag.lock().unwrap());
    }

    // ── H4: a cancelled dial leaves no link ──
    #[tokio::test]
    async fn a_dial_that_is_never_polled_opens_nothing() {
        let net = MemoryNetwork::new();
        let never_polled = net.dial(&Address::new("mem://x"));
        drop(never_polled);
        assert_eq!(net.open_endpoints(), 0);
        assert_eq!(net.dials(), 0, "폴링조차 안 된 dial 은 시도가 아니다");
    }

    #[tokio::test]
    async fn a_completed_dial_registers_exactly_one_endpoint() {
        let net = MemoryNetwork::new();
        let _link = net.dial(&Address::new("mem://x")).await.unwrap();
        assert_eq!(net.open_endpoints(), 1);
        assert_eq!(net.dials(), 1);
    }

    // ── H5: the harness also carries the 「실패 뒤에는 닫기를 내지 않는다」 (no close is emitted after a failure) latch ──
    //
    // ★The point is **queueing a close after a failure**★ — without the latch the second read raises the close left in the queue
    //   as 「상대가 말했다」 (the peer said so), and the [`crate::LinkRead::Closed`] contract becomes false in the harness alone.
    //   Counterpart on the real adapter side = `tests/ws_reject.rs`
    //   `a_read_after_a_failed_read_is_still_an_error_never_a_spoken_close`.
    #[tokio::test]
    async fn a_read_after_a_failed_read_is_still_an_error_in_the_harness_too() {
        let net = MemoryNetwork::new();
        let (_tx, mut rx) = net.dial(&Address::new("mem://x")).await.unwrap();
        let endpoint = net.accept().unwrap();
        endpoint.fail("boom");
        endpoint.close();
        assert!(rx.recv().await.is_err(), "첫 읽기는 실패다");
        match rx.recv().await {
            Err(_) => {}
            Ok(other) => panic!("실패 뒤에 닫기가 나왔다: {other:?}"),
        }
    }

    // ── H6: dropping the endpoint is a read error, not a close ──
    //
    // ★This test alone guards the channel-exhausted arm of `MemRx::recv`★ — H5 takes only the `Fail` arm through `fail`, and
    //   even with the channel-exhausted arm turned back into `Ok(Closed(None))` the rest of the suite was all green (measured 2026-09-25).
    #[tokio::test]
    async fn dropping_the_endpoint_is_a_read_error_not_a_close() {
        let net = MemoryNetwork::new();
        let (_tx, mut rx) = net.dial(&Address::new("mem://x")).await.unwrap();
        let endpoint = net.accept().unwrap();
        // `accept` takes it out of the registry, so this `Arc` is the sole owner of the sender.
        assert_eq!(Arc::strong_count(&endpoint), 1);
        drop(endpoint);
        match rx.recv().await {
            Err(_) => {}
            Ok(other) => panic!("말 없이 사라진 통로가 닫기로 올라왔다: {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_peer_can_close_with_a_code_and_the_reader_sees_it() {
        let net = MemoryNetwork::new();
        let (_tx, mut rx) = net.dial(&Address::new("mem://x")).await.unwrap();
        let endpoint = net.accept().unwrap();
        endpoint.reject(Close::new(
            crate::frame::CloseCode::HANDSHAKE_REJECTED,
            "no",
        ));
        match rx.recv().await.unwrap() {
            LinkRead::Closed(Some(close)) => {
                assert_eq!(close.code, crate::frame::CloseCode::HANDSHAKE_REJECTED);
                assert_eq!(close.reason, "no");
            }
            other => panic!("{other:?}"),
        }
    }

    // ── H7: two-ended link — both ends are real links ──
    #[tokio::test]
    async fn a_two_ended_link_carries_frames_both_ways_and_a_spoken_close() {
        let (mut a, mut b) = memory_link();
        a.tx.send(Frame::Text("a to b".into())).await.unwrap();
        b.tx.send(Frame::Binary(vec![1, 2])).await.unwrap();
        assert_eq!((a.control.unread(), b.control.unread()), (1, 1));
        assert!(
            matches!(b.rx.recv().await.unwrap(), LinkRead::Frame(Frame::Text(t)) if t == "a to b")
        );
        assert!(
            matches!(a.rx.recv().await.unwrap(), LinkRead::Frame(Frame::Binary(v)) if v == [1, 2])
        );
        assert_eq!((a.control.unread(), b.control.unread()), (0, 0));
        a.tx.close(Close::new(CloseCode::HANDSHAKE_REJECTED, "no"))
            .await;
        match b.rx.recv().await.unwrap() {
            LinkRead::Closed(Some(close)) => {
                assert_eq!(close.code, CloseCode::HANDSHAKE_REJECTED);
                assert_eq!(close.reason, "no");
            }
            other => panic!("{other:?}"),
        }
    }

    // Same as a real socket — an end closes only when both of its halves are dropped (`LinkTx` rustdoc). To the peer, a fully dropped end is a read error,
    // not a close (the two-ended version of H6).
    #[tokio::test]
    async fn one_end_closes_only_once_both_of_its_halves_are_gone() {
        let (mut a, b) = memory_link();
        let MemoryLinkEnd {
            tx: b_tx,
            rx: b_rx,
            control: b_control,
        } = b;
        drop(b_rx);
        a.tx.send(Frame::Text("into the void".into()))
            .await
            .expect("상대 끝의 쓰는 반쪽이 살아 있으면 통로는 열려 있다");
        assert!(
            poll_immediate(a.rx.recv()).await.is_none(),
            "아직 안 닫혔다"
        );
        assert!(!b_control.dropped());
        drop(b_tx);
        assert!(b_control.dropped());
        assert!(
            a.rx.recv().await.is_err(),
            "말 없이 사라진 끝은 읽기 오류다"
        );
        assert!(a.rx.recv().await.is_err(), "실패 뒤에도 오류다");
        assert!(a.tx.send(Frame::Text("x".into())).await.is_err());
    }

    #[tokio::test]
    async fn a_ping_is_answered_only_once_the_far_end_reads_it() {
        let (mut a, mut b) = memory_link();
        a.tx.ping().await.unwrap();
        assert!(
            poll_immediate(a.rx.recv()).await.is_none(),
            "상대가 읽기 전에는 대답이 없다"
        );
        assert!(matches!(
            b.rx.recv().await.unwrap(),
            LinkRead::Frame(Frame::Keepalive)
        ));
        assert!(matches!(
            a.rx.recv().await.unwrap(),
            LinkRead::Frame(Frame::Keepalive)
        ));
        assert_eq!((a.control.unread(), b.control.unread()), (0, 0));
    }

    // The two-ended version of H1 — the reason for putting the stalled write on another task is the same as in H1.
    #[tokio::test]
    async fn clearing_a_two_ended_stall_actually_wakes_the_parked_write() {
        let (a, mut b) = memory_link();
        a.control.stall_writes(true);
        assert!(a.control.writes_stalled());
        let mut tx = a.tx;
        let parked = tokio::spawn(async move { tx.send(Frame::Text("late".into())).await });
        settle().await;
        assert!(!parked.is_finished(), "막혀 있어야 한다");
        assert_eq!(b.control.unread(), 0);
        a.control.stall_writes(false);
        tokio::time::timeout(Duration::from_secs(5), parked)
            .await
            .expect("멈춤을 풀었는데도 쓰기가 안 깨어났다")
            .expect("쓰기 태스크가 패닉했다")
            .unwrap();
        assert!(
            matches!(b.rx.recv().await.unwrap(), LinkRead::Frame(Frame::Text(t)) if t == "late")
        );
    }

    // If the write stall gets set while stopped at the ping stall, releasing only the ping stall does not send it out; both stalls must be released.
    #[tokio::test]
    async fn a_ping_parked_on_its_own_stall_still_waits_for_a_write_stall_set_meanwhile() {
        let (a, mut b) = memory_link();
        a.control.stall_pings(true);
        let mut tx = a.tx;
        let parked = tokio::spawn(async move { tx.ping().await });
        settle().await;
        assert!(!parked.is_finished(), "ping 멈춤에서 서 있어야 한다");
        a.control.stall_writes(true);
        a.control.stall_pings(false);
        settle().await;
        assert!(
            !parked.is_finished(),
            "쓰기 멈춤이 걸려 있는데 ping 이 나갔다"
        );
        assert_eq!(b.control.unread(), 0);
        a.control.stall_writes(false);
        tokio::time::timeout(Duration::from_secs(5), parked)
            .await
            .expect("두 멈춤을 다 풀었는데도 ping 이 안 깨어났다")
            .expect("ping 태스크가 패닉했다")
            .unwrap();
        assert!(matches!(
            b.rx.recv().await.unwrap(),
            LinkRead::Frame(Frame::Keepalive)
        ));
    }

    #[tokio::test]
    async fn the_fail_knobs_act_on_their_own_end_and_one_direction_only() {
        let (mut a, mut b) = memory_link();
        b.tx.send(Frame::Text("before".into())).await.unwrap();
        a.control.fail_reads("boom");
        assert!(
            matches!(a.rx.recv().await.unwrap(), LinkRead::Frame(_)),
            "이미 들어온 것이 먼저다"
        );
        assert_eq!(a.rx.recv().await.unwrap_err().message, "boom");
        assert!(a.rx.recv().await.is_err());
        a.tx.send(Frame::Text("writes still go".into()))
            .await
            .expect("읽기 실패는 쓰기를 안 끊는다");
        assert!(matches!(b.rx.recv().await.unwrap(), LinkRead::Frame(_)));

        b.control.fail_writes();
        assert!(b.tx.send(Frame::Text("x".into())).await.is_err());
        assert!(b.tx.ping().await.is_err());
        b.tx.close(Close::going_away()).await;
        assert_eq!(
            a.control.unread(),
            0,
            "쓰기가 끊긴 끝의 닫기는 아무것도 안 보낸다"
        );
        a.tx.send(Frame::Text("still readable".into()))
            .await
            .unwrap();
        assert!(
            matches!(b.rx.recv().await.unwrap(), LinkRead::Frame(_)),
            "쓰기 실패는 읽기를 안 끊는다"
        );
    }

    // ── H8: MemoryListener — the receiving side ──
    fn listener(max_pending_upgrades: usize) -> (ManualClock, MemoryListener, MemoryConnector) {
        let clock = ManualClock::new();
        let (listener, connector) = MemoryListener::new(clock.handle(), max_pending_upgrades);
        (clock, listener, connector)
    }

    fn mem_addr() -> Address {
        Address::new("mem://listener")
    }

    // Drives the accepted end by hand without a supervisor (the path that puts it on the registry is `Registry::attach` — that test is in `registry.rs`) — the point is whether it joins
    //   the dialing supervisor through a real link.
    #[tokio::test]
    async fn a_dialer_peer_handshakes_and_round_trips_a_request_with_a_raw_accepted_end() {
        let (clock, mut listener, connector) = listener(4);
        let registry = Registry::new(
            Arc::new(TestWire),
            connector.dialer(),
            clock.handle(),
            Policy::default(),
        );
        let (peer, mut inbox) =
            registry.add(PeerId::new("d"), mem_addr(), Arc::new(HelloHandshake));
        settle_until("걸기가 준비 큐에 닿았다", || {
            listener.queued() == 1
        })
        .await;
        let mut accepted = listener.accept().await.expect("받은 통로");
        assert_eq!(accepted.info().remote.as_deref(), Some("mem:1"));
        assert!(matches!(
            accepted.rx.recv().await.unwrap(),
            LinkRead::Frame(Frame::Text(t)) if t == HelloHandshake::HELLO
        ));
        accepted
            .tx
            .send(Frame::Text(HelloHandshake::WELCOME.into()))
            .await
            .unwrap();
        settle_until("운영 진입", || peer.peer_state() == PeerState::Live).await;
        assert!(matches!(
            inbox.recv().await,
            Some(Incoming::Event(TransportEvent::Connected { .. }))
        ));

        let asking = peer.clone();
        let reply = tokio::spawn(async move {
            asking
                .request(TestOut::Request {
                    tag: 7,
                    body: "q".into(),
                })
                .await
        });
        match accepted.rx.recv().await.unwrap() {
            LinkRead::Frame(frame) => assert_eq!(frame, Frame::Text("req|7|q".into())),
            other => panic!("{other:?}"),
        }
        accepted.tx.send(TestWire::reply(7)).await.unwrap();
        let got = tokio::time::timeout(Duration::from_secs(5), reply)
            .await
            .expect("답장이 안 왔다")
            .expect("요청 태스크가 패닉했다")
            .expect("요청이 실패했다");
        assert_eq!(
            got,
            TestIn::Reply {
                tag: 7,
                body: "ok".into()
            }
        );
        assert_eq!((connector.dials(), connector.links()), (1, 1));
    }

    // The point is a future dropped after waking — an accept that woke because a link arrived while waiting leaves the link in place even if dropped without polling.
    #[tokio::test]
    async fn accept_is_cancel_safe() {
        let (_clock, mut listener, connector) = listener(4);
        let mut waiting = listener.accept();
        assert!(
            poll_immediate(&mut waiting).await.is_none(),
            "아직 아무도 안 걸었다"
        );
        let (mut tx, _rx) = connector.dial(&mem_addr()).await.unwrap();
        tx.send(Frame::Text("first words".into())).await.unwrap();
        drop(waiting);
        assert_eq!(
            listener.queued(),
            1,
            "버린 accept 가 통로를 가져가지 않았다"
        );
        let mut accepted = listener.accept().await.unwrap();
        assert_eq!(accepted.info().remote.as_deref(), Some("mem:1"));
        assert!(matches!(
            accepted.rx.recv().await.unwrap(),
            LinkRead::Frame(Frame::Text(t)) if t == "first words"
        ));
    }

    #[tokio::test]
    async fn dropping_the_listener_closes_what_it_has_not_handed_out() {
        let (_clock, listener, connector) = listener(4);
        let permits = listener.permits();
        let (mut tx, mut rx) = connector.dial(&mem_addr()).await.unwrap();
        assert_eq!(permits.in_use(), 1, "준비 큐에 앉은 통로가 칸을 쥔다");
        drop(listener);
        assert_eq!(permits.in_use(), 0);
        assert!(
            connector.link(1).unwrap().acceptor.dropped(),
            "안 받아 간 먼 끝이 닫혔다"
        );
        assert!(
            rx.recv().await.is_err(),
            "거는 쪽은 통로 손실(읽기 오류)로 본다"
        );
        assert!(tx.send(Frame::Text("x".into())).await.is_err());
        assert!(
            matches!(
                connector.dial(&mem_addr()).await,
                Err(DialFailure::Unreachable(_))
            ),
            "청취기가 없으면 못 붙는다"
        );
        assert_eq!((connector.dials(), connector.links()), (2, 1));
    }

    #[tokio::test]
    async fn a_dialer_peer_sees_a_dropped_listener_as_link_loss_and_cannot_land_again() {
        let (clock, listener, connector) = listener(4);
        let registry = Registry::new(
            Arc::new(TestWire),
            connector.dialer(),
            clock.handle(),
            Policy::default(),
        );
        let (peer, _inbox) = registry.add(PeerId::new("d"), mem_addr(), Arc::new(HelloHandshake));
        settle_until("핸드셰이크 진입", || {
            peer.peer_state() == PeerState::Handshaking
        })
        .await;
        assert_eq!(listener.queued(), 1);
        drop(listener);
        settle_until("통로 손실로 물러남", || {
            peer.peer_state() == PeerState::Backoff
        })
        .await;
        clock.advance(Duration::from_secs(11));
        settle_until("다시 걸었다", || connector.dials() == 2).await;
        assert_eq!(connector.links(), 1, "닿은 통로는 처음 하나뿐이다");
    }

    // The auth deadline is measured from the end of the upgrade (TRD 3 §5 3) — if stamped at the moment it was taken, the time sitting in the ready queue drops out of the deadline.
    #[tokio::test]
    async fn upgraded_at_is_when_the_link_landed_not_when_it_was_accepted() {
        let (clock, mut listener, connector) = listener(4);
        let landed = clock.now();
        let _near = connector.dial(&mem_addr()).await.unwrap();
        clock.advance(Duration::from_secs(3));
        let accepted = listener.accept().await.unwrap();
        assert_eq!(accepted.upgraded_at(), landed);
        assert_eq!(clock.now() - accepted.upgraded_at(), Duration::from_secs(3));
    }

    #[tokio::test]
    async fn the_upgrade_permit_rides_in_the_accepted_link_and_full_permits_refuse_the_dial() {
        let (_clock, mut listener, connector) = listener(1);
        let permits = listener.permits();
        let _first = connector.dial(&mem_addr()).await.unwrap();
        assert!(matches!(
            connector.dial(&mem_addr()).await,
            Err(DialFailure::Unreachable(e)) if e.message == "upgrade permits full"
        ));
        let accepted = listener.accept().await.unwrap();
        assert_eq!(permits.in_use(), 1, "받아 가도 칸은 그 통로와 함께 간다");
        assert!(
            connector.dial(&mem_addr()).await.is_err(),
            "아직 칸이 안 돌아왔다"
        );
        drop(accepted);
        assert_eq!(permits.in_use(), 0);
        assert!(connector.dial(&mem_addr()).await.is_ok());
        assert_eq!((connector.dials(), connector.links()), (4, 2));
    }

    #[tokio::test]
    async fn accept_ends_for_good_once_every_connector_is_gone_but_hands_out_what_landed_first() {
        let (_clock, mut listener, connector) = listener(4);
        let _near = connector.dial(&mem_addr()).await.unwrap();
        let other = connector.clone();
        drop((connector, other));
        assert!(
            listener.accept().await.is_ok(),
            "닿아 있던 것은 먼저 내준다"
        );
        assert!(listener.accept().await.is_err());
        assert!(listener.accept().await.is_err(), "종결은 계속 종결이다");
    }

    #[tokio::test]
    async fn a_connector_dial_that_is_never_polled_lands_nothing() {
        let (_clock, listener, connector) = listener(1);
        drop(connector.dial(&mem_addr()));
        assert_eq!(
            (
                connector.dials(),
                listener.queued(),
                listener.permits().in_use()
            ),
            (0, 0, 0)
        );
    }

    /// A clock that reads the connector's ledger inside `now`. Clock and connector hold each other and leak, but only for the length of one test.
    struct LogReadingClock {
        connector: std::sync::OnceLock<MemoryConnector>,
        reads: AtomicUsize,
    }

    impl Clock for LogReadingClock {
        fn now(&self) -> Instant {
            if let Some(connector) = self.connector.get() {
                let _ = (connector.links(), connector.dials());
                self.reads.fetch_add(1, Ordering::AcqRel);
            }
            Instant::now()
        }

        fn sleep(&self, _d: Duration) -> BoxFuture<'static, ()> {
            Box::pin(std::future::pending())
        }
    }

    // Runs the dialing on another thread — so a deadlock shows up as a deadline failure instead of hanging the test.
    #[test]
    fn a_clock_that_reads_the_connector_log_does_not_deadlock_the_dial() {
        use futures_util::FutureExt;

        let clock = Arc::new(LogReadingClock {
            connector: std::sync::OnceLock::new(),
            reads: AtomicUsize::new(0),
        });
        let (listener, connector) = MemoryListener::new(clock.clone(), 1);
        assert!(clock.connector.set(connector.clone()).is_ok());
        let (done_tx, done) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let landed = connector
                .dial(&mem_addr())
                .now_or_never()
                .map(|r| r.is_ok());
            let _ = done_tx.send(landed);
        });
        let landed = done
            .recv_timeout(Duration::from_secs(5))
            .expect("걸기가 교착했다 — 시계를 장부 자물쇠 안에서 읽었다");
        assert_eq!(landed, Some(true));
        assert_eq!(clock.reads.load(Ordering::Acquire), 1);
        assert_eq!(listener.queued(), 1);
    }
}
