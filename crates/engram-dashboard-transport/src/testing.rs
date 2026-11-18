//! Harness only — behind feature `test-support` (ADR-0012). ★The real thing behind 「never **actually waits** for backoff, keepalive or write deadlines」★ (ADR-0177 decision 8):
//! [`ManualClock`] stands in for time, [`MemoryNetwork`] and [`MemoryListener`] for the transport. ★Their far ends differ★ — [`MemoryNetwork`]'s is a script the test
//! drives ([`MemoryEndpoint`]); [`MemoryListener`]'s is a real [`LinkTx`]/[`LinkRx`] pair ([`Accepted`]), so it joins two supervisors.
//!
//! ★Only [`settle`] and [`settle_until`] need the current_thread runtime★ — they sync by cooperative yielding, so under `multi_thread` they would be silently flaky; both check the
//! runtime kind and panic (helpers built on them, like `Near::arrived` in `registry.rs`, share the premise). The in-memory parts and [`PermitPool`] also work under multi_thread
//! with real-time-bounded waits.
//!
//! ★Production code must not reference this module★ (absent from the default build). ★Consumers enable `test-support` only in `[dev-dependencies]`★ — CI checks the consumer
//! (`.github/workflows/ci.yml` `shell gate 5`; the daemon line comes with TRD 3 §6-3 U4).

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

// ── Time ──────────────────────────────────────────────────────────────────────

struct ManualState {
    now: Instant,
    waiters: Vec<(Instant, oneshot::Sender<()>)>,
}

impl ManualState {
    /// Drops the sleeper whose receiving side is gone. ★Without this `waiters` grows without bound★ — each supervisor turn creates a `sleep` and drops all but the winner.
    fn prune(&mut self) {
        self.waiters.retain(|(_, tx)| !tx.is_closed());
    }
}

/// A clock moved by hand — ★it never flows on its own★; only [`ManualClock::advance`] moves time.
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

    pub fn handle(&self) -> Arc<dyn Clock> {
        Arc::new(self.clone())
    }

    /// Moves time and wakes sleepers whose deadline passed **in deadline order** — ★the order is the contract★ (registration order across two deadlines would produce an order real time cannot, passing only under it).
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

const SETTLE_YIELDS: usize = 512;

fn assert_single_threaded() {
    // ★Yielding is not synchronization on a multithreaded runtime★ — a loud failure beats a silent flake.
    let flavor = tokio::runtime::Handle::current().runtime_flavor();
    assert_eq!(
        flavor,
        tokio::runtime::RuntimeFlavor::CurrentThread,
        "engram-dashboard-transport's harness is for the current_thread runtime only \
          (the #[tokio::test] default). Under multi_thread, settle() guarantees nothing"
    );
}

/// Lets the supervisor and reader tasks run as far as they can under the manual clock. ★No real sleep — it only repeats `yield_now`★ and does not notice unfinished work;
/// prefer [`settle_until`] when the condition is known.
pub async fn settle() {
    assert_single_threaded();
    for _ in 0..SETTLE_YIELDS {
        tokio::task::yield_now().await;
    }
}

/// Yields until the condition holds; ★past the limit it panics on the spot★ so 「not finished」 is not disguised as 「the assertion is wrong」.
pub async fn settle_until(what: &str, mut done: impl FnMut() -> bool) {
    assert_single_threaded();
    for _ in 0..SETTLE_YIELDS {
        if done() {
            return;
        }
        tokio::task::yield_now().await;
    }
    if !done() {
        panic!("did not settle even after yielding {SETTLE_YIELDS} times: {what}");
    }
}


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

pub struct MemoryEndpoint {
    to_client: mpsc::UnboundedSender<ServerMsg>,
    /// The number pushed in but not yet read by the client — held together with `MemRx`.
    unread: Arc<AtomicUsize>,
    from_client: Mutex<mpsc::UnboundedReceiver<ClientMsg>>,
    stall_tx: watch::Sender<bool>,
    ping_stall_tx: watch::Sender<bool>,
}

impl MemoryEndpoint {
    pub fn take(&self) -> Option<ClientMsg> {
        self.from_client.lock().unwrap().try_recv().ok()
    }

    pub fn drain(&self) -> Vec<ClientMsg> {
        let mut rx = self.from_client.lock().unwrap();
        let mut out = Vec::new();
        while let Ok(m) = rx.try_recv() {
            out.push(m);
        }
        out
    }

    pub fn frames(&self) -> Vec<Frame> {
        self.drain()
            .into_iter()
            .filter_map(|m| match m {
                ClientMsg::Frame(f) => Some(f),
                _ => None,
            })
            .collect()
    }

    pub fn push(&self, frame: Frame) {
        self.send(ServerMsg::Frame(frame));
    }

    /// Items (frames, closes, failures) pushed but not yet read by the client. ★Shows whether backpressure reached the link★ — an undrained read-ahead queue or bytes over the read share stop the read task.
    pub fn unread(&self) -> usize {
        self.unread.load(Ordering::Acquire)
    }

    /// ★Count before sending★ — counted after, the reader may take it in between and the count wraps below 0.
    fn send(&self, msg: ServerMsg) {
        self.unread.fetch_add(1, Ordering::AcqRel);
        if self.to_client.send(msg).is_err() {
            self.unread.fetch_sub(1, Ordering::AcqRel);
        }
    }

    /// ★Closes **without** a code★ — `LinkRead::Closed(None)`, which by contract still means 「the peer said it closed」 (vanishing silently is [`MemoryEndpoint::fail`] = `Err`).
    /// ★Dropping the endpoint is not this branch★ — it is a silent vanish, a read error as on a real socket; use this or [`MemoryEndpoint::reject`] for 「said it closed」.
    pub fn close(&self) {
        self.send(ServerMsg::Closed(None));
    }

    /// ★Closes with a code and message★ — the only way the client can see 「connected but rejected」.
    pub fn reject(&self, close: Close) {
        self.send(ServerMsg::Closed(Some(close)));
    }

    /// Cuts with an error — ★reading side only★ (the client's next read errors; writes still go out).
    pub fn fail(&self, why: &str) {
        self.send(ServerMsg::Fail(LinkError::new(why)));
    }

    /// ★Cuts the writing side only★ — the client's next `send`/`ping` errors (after release if stalled); reads stay alive. Counterpart of [`MemoryEndpoint::fail`]; dropping the endpoint
    /// cuts both, so which is reported first would depend on scheduling. Already-sent data stays available to [`MemoryEndpoint::take`].
    pub fn fail_writes(&self) {
        self.from_client.lock().unwrap().close();
    }

    /// While on, the client's writes stall — **all three**: `send`, `ping`, `close` (one link on a real socket). ★Turning off **really wakes** them★ — an unwakeable `pending()` would make
    /// the stall decoration. ★A test dropping the link while blocked must account for the pending close★ (`drop_link`'s close stalls and later steps never start): turn this off or advance the
    /// clock `write_deadline`. ★The 「peer does not read」 kernel-buffer case cannot be measured here★ — a real socket does; this measures our deadline and queue logic.
    pub fn stall_writes(&self, on: bool) {
        let _ = self.stall_tx.send(on);
    }

    pub fn writes_stalled(&self) -> bool {
        *self.stall_tx.borrow()
    }

    /// While on, **only** the client's `ping` stalls (stacks with [`MemoryEndpoint::stall_writes`], really wakes when off). ★Does not exist on a real socket★ — it builds 「an earlier write
    /// finished, then it stops at the next timer turn's ping」, where the supervisor moves on without yielding at release.
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

    /// ★Close shares the link with `send`/`ping`, so it blocks with them★ — a real close frame queues behind a truncated frame, which is where `drop_link` spends `write_deadline`.
    /// ★Do not delete this `wait_writable`★ — without it the harness could not express 「a close queued behind a blocked write」.
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
            // ★Same contract as the real adapter★ — a `Closed(None)` after a failure would make code branching on it green here and dead on a real socket ([`crate::LinkRead::Closed`]).
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
                // ★Channel exhausted = vanished without a word★ — a read error, as on a real socket (not `Ok(Closed(None))`, which made the [`crate::LinkRead::Closed`] contract false only in the harness).
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

    pub fn fail_next(&self, n: usize) {
        self.inner.lock().unwrap().fail_next = n;
    }

    pub fn fail_forever(&self) {
        self.inner.lock().unwrap().fail_forever = true;
    }

    /// From now on every dial ends in [`DialFailure::Refused`] with no link (a known terminal refusal such as a local version check); beats [`MemoryNetwork::fail_forever`] · [`MemoryNetwork::fail_next`].
    /// `close.code` must be a terminal refusal code.
    pub fn refuse(&self, close: Close) {
        self.inner.lock().unwrap().refuse = Some(close);
    }

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

    /// The far side of the oldest link that has 「not yet been taken」.
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

    pub fn open_endpoints(&self) -> usize {
        self.inner.lock().unwrap().opened.len()
    }
}

impl Dialer for MemoryNetwork {
    fn dial(
        &self,
        _addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>> {
        // ★The link is created **inside** the future★ — outside, a cancelled dial leaves a link the test would `accept()` and assert on though the supervisor never adopted it.
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

struct EndState {
    stall: watch::Sender<bool>,
    ping_stall: watch::Sender<bool>,
    writes_failed: AtomicBool,
    unread: AtomicUsize,
    /// The number of live halves (2 at the start). 0 = this end has been dropped entirely.
    halves: AtomicUsize,
    /// This end's incoming queue (where the handle puts failures). ★Weak★ — a strong handle held by a test would stop the queue from draining after the peer vanishes and hide the read error.
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

/// Puts one item into the `dest` end's queue ★counting before putting in★ (see `MemoryEndpoint::send`).
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

    /// Returns when both stalls are released, re-checking the other on each wake. ★Do not turn it back into passing them one after another★ — a write stall set while stopped at the ping stall would let the ping out.
    async fn wait_pingable(&mut self) {
        loop {
            self.wait_writable().await;
            let _ = self.ping_stall.wait_for(|stalled| !*stalled).await;
            if !*self.stall.borrow() {
                return;
            }
        }
    }

    /// ★If only the peer end's reading half is dropped, writes succeed★ — a real socket closes only when both halves are dropped ([`LinkTx`] rustdoc); what was put in vanishes.
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
    /// Where the reply to a received Ping goes — on a real socket too it comes from the reading side (`ws.rs`); ★so an end that does not read gets no reply★.
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
                    // The reply does not take the stall — on a real socket it goes out late behind a congested buffer.
                    if !self.me.writes_failed.load(Ordering::Acquire) {
                        deliver(&self.peer, &self.to_peer, PipeMsg::Pong);
                    }
                    Ok(LinkRead::Frame(Frame::Keepalive))
                }
                Some(PipeMsg::Pong) => Ok(LinkRead::Frame(Frame::Keepalive)),
                Some(PipeMsg::Closed(close)) => Ok(LinkRead::Closed(close)),
                // ★Queue exhausted = peer end fully dropped = vanished without a word★ — a read error, not a close (same arm in `MemRx`).
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

pub struct MemoryLinkEnd {
    pub tx: Box<dyn LinkTx>,
    pub rx: Box<dyn LinkRx>,
    pub control: MemoryLinkControl,
}

/// A handle operating one end of a two-ended link — ★acts on its own end only★. [`MemoryEndpoint`]'s handle is what the far end applies to the client end, so names differ (its `fail` =
/// [`MemoryLinkControl::fail_reads`] here). Holding it does not keep the end alive — when both halves drop the peer sees a read error.
#[derive(Clone)]
pub struct MemoryLinkControl {
    state: Arc<EndState>,
}

impl MemoryLinkControl {
    /// While on, this end's `send`, `ping` and `close` stall; off really wakes them (as [`MemoryEndpoint::stall_writes`]).
    pub fn stall_writes(&self, on: bool) {
        self.state.stall.send_replace(on);
    }

    pub fn writes_stalled(&self) -> bool {
        *self.state.stall.borrow()
    }

    pub fn stall_pings(&self, on: bool) {
        self.state.ping_stall.send_replace(on);
    }

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

    pub fn unread(&self) -> usize {
        self.state.unread.load(Ordering::Acquire)
    }

    pub fn dropped(&self) -> bool {
        self.state.halves.load(Ordering::Acquire) == 0
    }
}

/// Builds two ends joined to each other. ★The queue is unbounded★ — a write stopping in front of a non-reading peer (kernel buffer) belongs to a real socket.
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

#[derive(Clone)]
pub struct MemoryLinkControls {
    pub dialer: MemoryLinkControl,
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

/// In-memory listener — hands out links dialed through [`MemoryConnector`] as [`Accepted`]. ★Finishing the dial is the end of the upgrade★ (there is none): the link enters the ready
/// queue and [`Accepted::upgraded_at`] is that time; the permit ([`MemoryListener::permits`]) is taken then and goes with [`Accepted`] — all used makes that dial fail (TRD 3 §5 14).
/// ★The terminal `Err` of [`Listener::accept`] = every [`MemoryConnector`] gone and the ready queue empty★. Dropping the listener closes queued links (the dialer reads an error) and later dials fail.
pub struct MemoryListener {
    ready: mpsc::UnboundedReceiver<Accepted>,
    permits: PermitPool,
}

impl MemoryListener {
    /// `clock` = the registry's clock. `max_pending_upgrades` = upgrade permits (ready queue + taken ones, TRD 3 §5 14) — ★not the TCP listen backlog★.
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

/// Takes out both ends of the accepted link for a real-socket test driving the receiving side without a supervisor (`tests/ws_*.rs`). ★Not a production surface★ — [`Accepted`]'s ends never leave
/// the crate; this exists behind `test-support`, after the listener's accept check, and releases the upgrade permit.
pub fn accepted_ends(accepted: Accepted) -> (Box<dyn LinkTx>, Box<dyn LinkRx>) {
    (accepted.tx, accepted.rx)
}

/// A [`Dialer`] dialing through a [`MemoryListener`]; clones dial the same listener; the address is ignored.
#[derive(Clone)]
pub struct MemoryConnector {
    ready: mpsc::UnboundedSender<Accepted>,
    shared: Arc<ListenShared>,
}

impl MemoryConnector {
    pub fn dialer(&self) -> Arc<dyn Dialer> {
        Arc::new(self.clone())
    }

    pub fn dials(&self) -> usize {
        self.shared.log.lock().unwrap().dials
    }

    pub fn links(&self) -> usize {
        self.shared.log.lock().unwrap().links.len()
    }

    /// The handle of the `n`th (from 1) link; its [`LinkInfo::remote`] is `mem:<n>`.
    pub fn link(&self, n: usize) -> Option<MemoryLinkControls> {
        let log = self.shared.log.lock().unwrap();
        n.checked_sub(1).and_then(|i| log.links.get(i).cloned())
    }

    pub fn latest(&self) -> Option<MemoryLinkControls> {
        self.shared.log.lock().unwrap().links.last().cloned()
    }
}

impl Dialer for MemoryConnector {
    fn dial(
        &self,
        _addr: &Address,
    ) -> BoxFuture<'_, Result<(Box<dyn LinkTx>, Box<dyn LinkRx>), DialFailure>> {
        // ★The link is created **inside** the future★ ([`Dialer`] contract).
        let ready = self.ready.clone();
        let shared = self.shared.clone();
        Box::pin(async move {
            // ★Read the injected clock outside the ledger lock★ — a clock reading this connector's ledger would retake the lock and deadlock.
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

/// Rejects right after connecting — [`RejectingHandshake::new`] = terminal (★does not spend budget★), [`RejectingHandshake::retryable`] = redial (spends budget).
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

/// Sends nothing and waits for the peer's first frame — measures the path where the peer speaks by **closing**.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestOut {
    Request { tag: u64, body: String },
    Notice(String),
    Resume { key: u32, after: Option<u64> },
    Blob(Vec<u8>),
}

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

pub struct TestWire;

impl TestWire {
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

    pub fn chunk(key: u32, generation: u32, seq: u64) -> Frame {
        Self::encode_in(&TestIn::Chunk {
            key,
            generation,
            seq,
            body: format!("s{seq}"),
        })
    }

    pub fn reply(tag: u64) -> Frame {
        Self::encode_in(&TestIn::Reply {
            tag,
            body: "ok".into(),
        })
    }
}

/// A decode failure of [`TestWire`]. ★Value-carrying branches quote the received text verbatim★ — to measure whether it leaks into the event ([`DecodeClass`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestDecodeError {
    Keepalive,
    BadNumber(String),
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
    // ★Put the stalled work on **another task**★ — polled by hand on the same task, we re-poll after release and **an implementation that registers no waker passes too**; only a wake-up completes it on a task.
    #[tokio::test]
    async fn clearing_the_stall_actually_wakes_the_parked_write() {
        let net = MemoryNetwork::new();
        let (mut tx, _rx) = net.dial(&Address::new("mem://x")).await.unwrap();
        let endpoint = net.accept().unwrap();
        endpoint.stall_writes(true);

        let parked = tokio::spawn(async move { tx.send(Frame::Text("payload".into())).await });
        settle().await;
        assert!(!parked.is_finished(), "must be blocked");
        assert!(endpoint.frames().is_empty());

        // ★It is this side, not the sleeping task, that releases the stall★ — the only means of waking the other side is the waker.
        endpoint.stall_writes(false);
        // ★Bounded wait★ — if it falls back to an unwakeable stall it must fail cleanly instead of hanging.
        tokio::time::timeout(Duration::from_secs(5), parked)
            .await
            .expect("the write did not wake even after the stall was released")
            .expect("the write task panicked")
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
            "★the close must be blocked too★ — otherwise the harness cannot express the shape of a real socket"
        );
        assert!(endpoint.drain().is_empty());

        endpoint.stall_writes(false);
        // ★Bounded wait★ — if it falls back to an unwakeable stall it must fail cleanly instead of hanging.
        tokio::time::timeout(Duration::from_secs(5), parked)
            .await
            .expect("the close did not wake even after the stall was released")
            .expect("the close task panicked");
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
    #[should_panic(expected = "did not settle")]
    async fn settle_until_says_so_instead_of_failing_somewhere_else() {
        settle_until("a condition that is never true", || false).await;
    }

    #[tokio::test]
    async fn settle_until_returns_as_soon_as_the_condition_holds() {
        let flag = Arc::new(Mutex::new(false));
        let setter = flag.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            *setter.lock().unwrap() = true;
        });
        settle_until("flag", || *flag.lock().unwrap()).await;
        assert!(*flag.lock().unwrap());
    }

    // ── H4: a cancelled dial leaves no link ──
    #[tokio::test]
    async fn a_dial_that_is_never_polled_opens_nothing() {
        let net = MemoryNetwork::new();
        let never_polled = net.dial(&Address::new("mem://x"));
        drop(never_polled);
        assert_eq!(net.open_endpoints(), 0);
        assert_eq!(
            net.dials(),
            0,
            "a dial that was not even polled is not an attempt"
        );
    }

    #[tokio::test]
    async fn a_completed_dial_registers_exactly_one_endpoint() {
        let net = MemoryNetwork::new();
        let _link = net.dial(&Address::new("mem://x")).await.unwrap();
        assert_eq!(net.open_endpoints(), 1);
        assert_eq!(net.dials(), 1);
    }

    // ── H5: the harness also carries the 「no close is emitted after a failure」 latch ──
    //
    // ★Queue a close after a failure★ — without the latch the second read raises it as 「the peer said so」, making the [`crate::LinkRead::Closed`] contract false in the harness
    //   (real-adapter counterpart: `tests/ws_reject.rs` `a_read_after_a_failed_read_is_still_an_error_never_a_spoken_close`).
    #[tokio::test]
    async fn a_read_after_a_failed_read_is_still_an_error_in_the_harness_too() {
        let net = MemoryNetwork::new();
        let (_tx, mut rx) = net.dial(&Address::new("mem://x")).await.unwrap();
        let endpoint = net.accept().unwrap();
        endpoint.fail("boom");
        endpoint.close();
        assert!(rx.recv().await.is_err(), "the first read is a failure");
        match rx.recv().await {
            Err(_) => {}
            Ok(other) => panic!("a close came out after the failure: {other:?}"),
        }
    }

    // ── H6: dropping the endpoint is a read error, not a close ──
    //
    // ★This test alone guards the channel-exhausted arm of `MemRx::recv`★ (H5 covers only the `Fail` arm).
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
            Ok(other) => panic!("a link that vanished silently came up as a close: {other:?}"),
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

    // As a real socket — an end closes only when both halves drop (`LinkTx` rustdoc); to the peer a fully dropped end is a read error, not a close (two-ended H6).
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
            .expect("if the writing half of the peer's end is alive, the link is open");
        assert!(
            poll_immediate(a.rx.recv()).await.is_none(),
            "not closed yet"
        );
        assert!(!b_control.dropped());
        drop(b_tx);
        assert!(b_control.dropped());
        assert!(
            a.rx.recv().await.is_err(),
            "an end that vanished silently is a read error"
        );
        assert!(
            a.rx.recv().await.is_err(),
            "still an error after the failure"
        );
        assert!(a.tx.send(Frame::Text("x".into())).await.is_err());
    }

    #[tokio::test]
    async fn a_ping_is_answered_only_once_the_far_end_reads_it() {
        let (mut a, mut b) = memory_link();
        a.tx.ping().await.unwrap();
        assert!(
            poll_immediate(a.rx.recv()).await.is_none(),
            "no answer before the peer reads"
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
        assert!(!parked.is_finished(), "must be blocked");
        assert_eq!(b.control.unread(), 0);
        a.control.stall_writes(false);
        tokio::time::timeout(Duration::from_secs(5), parked)
            .await
            .expect("the write did not wake even after the stall was released")
            .expect("the write task panicked")
            .unwrap();
        assert!(
            matches!(b.rx.recv().await.unwrap(), LinkRead::Frame(Frame::Text(t)) if t == "late")
        );
    }

    // A write stall set while stopped at the ping stall: releasing only the ping stall must not send it; both must be released.
    #[tokio::test]
    async fn a_ping_parked_on_its_own_stall_still_waits_for_a_write_stall_set_meanwhile() {
        let (a, mut b) = memory_link();
        a.control.stall_pings(true);
        let mut tx = a.tx;
        let parked = tokio::spawn(async move { tx.ping().await });
        settle().await;
        assert!(!parked.is_finished(), "must be stuck at the ping stall");
        a.control.stall_writes(true);
        a.control.stall_pings(false);
        settle().await;
        assert!(
            !parked.is_finished(),
            "a ping went out while a write stall was in place"
        );
        assert_eq!(b.control.unread(), 0);
        a.control.stall_writes(false);
        tokio::time::timeout(Duration::from_secs(5), parked)
            .await
            .expect("the ping did not wake even after both stalls were released")
            .expect("the ping task panicked")
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
            "what already came in comes first"
        );
        assert_eq!(a.rx.recv().await.unwrap_err().message, "boom");
        assert!(a.rx.recv().await.is_err());
        a.tx.send(Frame::Text("writes still go".into()))
            .await
            .expect("a read failure does not cut writing");
        assert!(matches!(b.rx.recv().await.unwrap(), LinkRead::Frame(_)));

        b.control.fail_writes();
        assert!(b.tx.send(Frame::Text("x".into())).await.is_err());
        assert!(b.tx.ping().await.is_err());
        b.tx.close(Close::going_away()).await;
        assert_eq!(
            a.control.unread(),
            0,
            "closing an end whose writing was cut sends nothing"
        );
        a.tx.send(Frame::Text("still readable".into()))
            .await
            .unwrap();
        assert!(
            matches!(b.rx.recv().await.unwrap(), LinkRead::Frame(_)),
            "a write failure does not cut reading"
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

    // Drives the accepted end by hand without a supervisor (the `Registry::attach` path is tested in `registry.rs`): does it join the dialing supervisor through a real link.
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
        settle_until("the dial reached the ready queue", || {
            listener.queued() == 1
        })
        .await;
        let mut accepted = listener.accept().await.expect("accepted link");
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
        settle_until("operation entered", || peer.peer_state() == PeerState::Live).await;
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
            .expect("the reply did not come")
            .expect("the request task panicked")
            .expect("the request failed");
        assert_eq!(
            got,
            TestIn::Reply {
                tag: 7,
                body: "ok".into()
            }
        );
        assert_eq!((connector.dials(), connector.links()), (1, 1));
    }

    // A future dropped after waking — an accept woken by an arriving link leaves it in place even if dropped unpolled.
    #[tokio::test]
    async fn accept_is_cancel_safe() {
        let (_clock, mut listener, connector) = listener(4);
        let mut waiting = listener.accept();
        assert!(
            poll_immediate(&mut waiting).await.is_none(),
            "nobody has dialed yet"
        );
        let (mut tx, _rx) = connector.dial(&mem_addr()).await.unwrap();
        tx.send(Frame::Text("first words".into())).await.unwrap();
        drop(waiting);
        assert_eq!(
            listener.queued(),
            1,
            "the abandoned accept did not take the link"
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
        assert_eq!(
            permits.in_use(),
            1,
            "a link sitting in the ready queue holds a permit"
        );
        drop(listener);
        assert_eq!(permits.in_use(), 0);
        assert!(
            connector.link(1).unwrap().acceptor.dropped(),
            "the far end that was not taken was closed"
        );
        assert!(
            rx.recv().await.is_err(),
            "the dialing side sees it as link loss (read error)"
        );
        assert!(tx.send(Frame::Text("x".into())).await.is_err());
        assert!(
            matches!(
                connector.dial(&mem_addr()).await,
                Err(DialFailure::Unreachable(_))
            ),
            "cannot attach without a listener"
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
        settle_until("handshake entered", || {
            peer.peer_state() == PeerState::Handshaking
        })
        .await;
        assert_eq!(listener.queued(), 1);
        drop(listener);
        settle_until("backed off due to link loss", || {
            peer.peer_state() == PeerState::Backoff
        })
        .await;
        clock.advance(Duration::from_secs(11));
        settle_until("redialed", || connector.dials() == 2).await;
        assert_eq!(connector.links(), 1, "only the first link arrived");
    }

    // The auth deadline starts at the end of the upgrade (TRD 3 §5 3) — stamped at take time, ready-queue time would drop out.
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
        assert_eq!(
            permits.in_use(),
            1,
            "even when taken, the permit goes with that link"
        );
        assert!(
            connector.dial(&mem_addr()).await.is_err(),
            "the permit has not returned yet"
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
            "what had already arrived is handed out first"
        );
        assert!(listener.accept().await.is_err());
        assert!(listener.accept().await.is_err(), "terminal stays terminal");
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

    /// A clock that reads the connector's ledger inside `now` (they hold each other and leak, for one test).
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
            .expect("the dial deadlocked — the clock was read inside the ledger lock");
        assert_eq!(landed, Some(true));
        assert_eq!(clock.reads.load(Ordering::Acquire), 1);
        assert_eq!(listener.queued(), 1);
    }
}
