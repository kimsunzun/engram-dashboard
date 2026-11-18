//! Peer-N registry — ★driving layer★. Sets up one supervisor per peer and returns that handle's receiver end ([`Inbox`]) to the side that raised it.
//!
//! ★N peers from the start★ (ADR-0177 decision 3 · ADR-0179) — no state presumes a world of one. **Peers share no state**: if daemon A gives up, B operates.
//!
//! ★The upward line is separate per handle★ (ADR-0310 decision 17 · TRD 3 §3-0 ① D24 ②) — with one line a noisy peer would hold a quiet peer's
//! events · packets behind it and 「even if A dies, B and C must keep working」 collapses (ADR-0180 decision 8). Each [`Registry::add`] sets up a line (length =
//! [`Policy::inbound_queue`]), returns its receiving end, and only that one supervisor holds the enqueue end. When full, the supervisor holds in its own
//! ordering area and stops that peer's reading — it neither drops nor cuts the connection (`peer.rs` header). An undrained receiver end stalls only that peer.
//!
//! ★Re-raising the same name tag does not continue the line★ — the old supervisor puts into the old receiver end until teardown finishes, the new one into
//! the new; one receiver end's end is that supervisor's end, and there is no ordering between receiver ends (a consumer adapter merges them — filtering rules TRD 3 §3-3).
//!
//! ★Two ways to raise a peer★ — [`Registry::add`] dials by address and redials when cut; [`Registry::attach`] puts on one received channel ([`Accepted`]),
//! never redials, and removes itself when it ends (TRD 3 §3-1 ①). The consumer runs the accept loop; the registry holds no listener.
//!
//! ★The registry does not decide 「is the whole app disconnected」★ — that is a screen concept; this crate only counts peers. It gives two ingredients
//! ([`Registry::peers`] · [`crate::Peer::reconnect_now`]) and the shell folds them (closing ADR-0180's open question).

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::budget::Charged;
use crate::clock::{with_deadline, Clock, Elapsed};
use crate::event::{Incoming, PeerId};
use crate::frame::CloseCode;
use crate::link::{Accepted, Address, Dialer, Handshake};
use crate::machine::PeerState;
use crate::peer::{prepare_peer, AcceptedLink, LeaveRoster, Origin, Peer, PeerConfig, BYTE_BUDGET};
use crate::policy::Policy;
use crate::wire::Wire;

/// One handle's receiver end — only what the one supervisor raised by the [`Registry::add`] · [`Registry::attach`] that returned it emitted.
///
/// ★The end = `None` after [`TransportEvent::Closed`](crate::TransportEvent::Closed)★ — the supervisor loads `Closed` last in teardown and releases the
/// enqueue end. ★A `None` without `Closed`, unless the receiver called [`Inbox::close`]★ means the supervisor ended without normal teardown: teardown
/// exceeded `CLOSE_DRAIN_LIMIT` (`peer.rs`) and dropped the rest up to `Closed` (neither counted nor reported), or the task ended abnormally (panic ·
/// runtime shutdown). A `None` after [`Inbox::close`] is not the supervisor's end — it may still run. ★Drain to the end★ — crate header 「Receiver-side invariants」 5.
///
/// Each item carries the supervisor's byte share, released when taken out (`budget.rs`); dropping the receiver end releases the share of what it held.
pub struct Inbox<W: Wire> {
    rx: mpsc::Receiver<Charged<Incoming<W>>>,
}

impl<W: Wire> Inbox<W> {
    /// `None` = the end — the supervisor released the enqueue end or [`Inbox::close`] was called, and everything held has been taken out.
    /// ★Cancel-safe★ — on a `select!` arm no item is lost when another arm wins.
    pub async fn recv(&mut self) -> Option<Incoming<W>> {
        self.rx.recv().await.map(Charged::into_item)
    }

    /// Without waiting. `None` = nothing held now or ended — use [`Inbox::recv`] to tell them apart.
    pub fn try_recv(&mut self) -> Option<Incoming<W>> {
        self.rx.try_recv().ok().map(Charged::into_item)
    }

    /// Blocks only new items. What is already held (including an already loaded `Closed`) can still be taken out with [`Inbox::recv`] · [`Inbox::try_recv`];
    /// when all is taken out it is `None` even if the supervisor still runs. ★What the supervisor holds in the ordering area unloaded is silently dropped★
    /// (a closed line cannot take it; neither counted nor reported) — the same if the [`Inbox`] is dropped.
    ///
    /// ★The supervisor folds immediately★ — even with nothing to put in it goes to closing on the spot and does not re-attach (crate header 「Receiver-side
    /// invariants」 5); the same if dropped. ★Accepted outbound work is dropped too★ — queued commands are rejected and the frame being written is dropped
    /// (counted, nowhere to report). To send and leave, drain until `Closed` · `None`, then release.
    pub fn close(&mut self) {
        self.rx.close();
    }
}

struct RegistryInner<W: Wire> {
    wire: Arc<W>,
    dialer: Arc<dyn Dialer>,
    clock: Arc<dyn Clock>,
    policy: Policy,
    /// ★Do not wait inside the lock★ — starting supervisors · closing · dropping handles are outside it.
    roster: Mutex<Roster<W>>,
}

/// Everything one registry lock guards. ★The generation table is inside it too★ — taking out the handle to replace · choosing the counter · raising the
/// new handle must be one critical section (`Registry::launch`).
struct Roster<W: Wire> {
    peers: HashMap<PeerId, Peer<W>>,
    /// The per-name-tag generation counter. ★Do not erase with `remove`★ — re-raising the same name tag would rewind generations to 0 and the consumer's
    /// ordinary defense (「ignore anything below the max generation」) would discard the new connection entirely.
    ///
    /// It grows by **the number of name tags the consumer wrote with [`Registry::add`]** (not peer-decided). ★Accepted peers are not put in★ — their name
    /// tag is new per connection and it would grow forever ([`Registry::attach_with_policy`]).
    generations: HashMap<PeerId, Arc<AtomicU64>>,
    /// [`Registry::close_all`] was called — a peer raised afterwards does not get onto the registry and is closed at once. The setter (`close_all`'s draining)
    /// and reader (`launch`) share the lock, so a raise crossing the fold does not remain in the emptied registry.
    folded: bool,
}

impl<W: Wire> RegistryInner<W> {
    /// Reuse a poisoned lock as is — leaving the poison would make the registry unusable forever. No half-changed table remains: the only thing that can panic
    /// inside the lock is building the supervisor (`prepare_peer` of `Registry::launch` — e.g. the injected clock), which ends before the table changes, and the
    /// table change is one field at a time. ★A panic while building does not retake this lock while unwinding★ (`new_peer` in `peer.rs`) — it would deadlock on the same thread.
    fn roster(&self) -> MutexGuard<'_, Roster<W>> {
        self.roster.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<W: Wire> Roster<W> {
    /// The generation counter this name tag will use — the table's if present, else the raised (to be replaced) handle's, else a new one. Does not change the
    /// table; `Registry::launch` puts it there after building.
    fn counter(&self, id: &PeerId) -> Arc<AtomicU64> {
        if let Some(counter) = self.generations.get(id) {
            return counter.clone();
        }
        // ★Continue the counter itself, not the old supervisor's last generation★ (race = `PeerConfig::generations`). Replacing an accepted peer not in the table
        //   with `add` with a new counter would repeat numbers the accepted peer already drew.
        self.peers
            .get(id)
            .map(Peer::generations)
            .unwrap_or_default()
    }
}

/// Peer registry. A clone points to the same registry. ★When the last clone disappears the registry merely releases the handles it holds★ — a supervisor hears
/// it as a close once all of that peer's [`Peer`] handles are gone and tears down (its receiver end ends with `Closed` → `None`); a consumer-held handle keeps it running.
pub struct Registry<W: Wire> {
    inner: Arc<RegistryInner<W>>,
}

impl<W: Wire> Clone for Registry<W> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<W: Wire> Registry<W> {
    /// Sets up one registry. ★What is received comes out only through the [`Inbox`] that [`Registry::add`] · [`Registry::attach`] return per handle★. `dialer` is used only by `add`.
    pub fn new(
        wire: Arc<W>,
        dialer: Arc<dyn Dialer>,
        clock: Arc<dyn Clock>,
        policy: Policy,
    ) -> Self {
        debug_assert_eq!(
            policy.check(),
            Ok(()),
            "the policy value violates its own invariants"
        );
        Self {
            inner: Arc::new(RegistryInner {
                wire,
                dialer,
                clock,
                policy,
                roster: Mutex::new(Roster {
                    peers: HashMap::new(),
                    generations: HashMap::new(),
                    folded: false,
                }),
            }),
        }
    }

    /// Raises a peer and starts attaching at once — returns that handle and receiver end. An existing same name tag is **closed and replaced**.
    ///
    /// ★Must be called inside a tokio runtime★ — it starts a supervisor task; outside, it panics before the registry is touched (usable afterwards, a peer
    /// raised under the same name tag stays raised).
    ///
    /// After [`Registry::close_all`] it does not raise onto the registry and closes before dialing — the receiver end ends `Closed` → `None` and the
    /// [`Dialer`] is not called.
    pub fn add(
        &self,
        id: PeerId,
        addr: Address,
        handshake: Arc<dyn Handshake>,
    ) -> (Peer<W>, Inbox<W>) {
        let policy = self.inner.policy.clone();
        self.add_with_policy(id, addr, handshake, policy)
    }

    /// Applies a different policy to this peer only. Runtime conditions · a folded registry as [`Registry::add`].
    ///
    /// ★Close the old supervisor first, and pass its last generation as the new supervisor's floor★ — reversed, two sockets attach to the same peer at once;
    /// without the floor, generations rewind to 0 and the consumer's ordinary defense (「ignore anything below the max generation」) **discards the new
    /// connection entirely**.
    ///
    /// ★The overlap still does not become 0★ — [`Peer::close`] has no completion signal, so the old supervisor may still be closing and may enter the
    /// operating phase and issue one more generation. The **shared counter** makes that harmless — the generation drawn on attach (`Connected` · `Disconnected`
    /// · the number on messages) never comes out twice. ★Exception: a supervisor that never attached★ carries the floor (the old supervisor's last
    /// generation) as is, so its [`TransportEvent::Closed`](crate::TransportEvent::Closed) and [`Peer::generation`] show the old number; the receiver end tells which supervisor it is.
    ///
    /// ★Generations start again from 1 only for a name tag never raised with `add` and not on the registry now★ — `add` on a name tag put on as an accepted peer
    /// that then removed itself or was removed with [`Registry::remove`] falls here too. Otherwise it continues this name tag's counter (table's, else the
    /// replaced handle's) and puts it on the table, also among simultaneous raises — no call makes a new counter without seeing a handle another took out.
    ///
    /// ★The receiver end is new even under the same name tag★ — the old supervisor puts into the old one until teardown finishes and its end is `None` after
    /// `Closed`. If the old receiver end is released, the old teardown ends without waiting; held undrained, it remains up to `CLOSE_DRAIN_LIMIT` in `peer.rs`.
    pub fn add_with_policy(
        &self,
        id: PeerId,
        addr: Address,
        handshake: Arc<dyn Handshake>,
        policy: Policy,
    ) -> (Peer<W>, Inbox<W>) {
        debug_assert_eq!(
            policy.check(),
            Ok(()),
            "the policy value violates its own invariants"
        );
        let (inbound, rx) = mpsc::channel(policy.inbound_queue.max(1));
        let origin = Origin::Dial {
            addr,
            dialer: self.inner.dialer.clone(),
        };
        let peer = self.launch(id.clone(), true, |generations| PeerConfig {
            id,
            origin,
            wire: self.inner.wire.clone(),
            clock: self.inner.clock.clone(),
            handshake,
            policy,
            inbound,
            byte_budget: BYTE_BUDGET,
            generations,
        });
        (peer, Inbox { rx })
    }

    /// Puts a received channel on as a peer — returns that handle and receiver end. The policy is the registry's ([`Registry::attach_with_policy`] overrides it; contract there).
    pub fn attach(
        &self,
        id: PeerId,
        accepted: Accepted,
        handshake: Arc<dyn Handshake>,
    ) -> (Peer<W>, Inbox<W>) {
        let policy = self.inner.policy.clone();
        self.attach_with_policy(id, accepted, handshake, policy)
    }

    /// Puts a received channel ([`Accepted`]) on as a peer with this policy (TRD 3 §3-1 ①). Runtime conditions · replacing the same name tag (closes the old
    /// one · old receiver end ends `Closed` → `None` · receiver end is new) as [`Registry::add_with_policy`].
    ///
    /// ★Starts from the handshake and never redials★ — ending without entering the operating phase (reject · deadline · channel loss) or cut during operation,
    /// it folds at once: the receiver end ends with `None` after [`TransportEvent::Closed`](crate::TransportEvent::Closed) whose `cause` carries the real reason
    /// (for a disconnect during operation, the preceding `Disconnected`'s cause, read-end/write-end distinction included). Dialing-side events (`ConnectFailed` ·
    /// `Rejected` · `GaveUp`) do not come and [`Peer::state`] skips `Dialing` · `Backoff` · `GaveUp`. [`Peer::reconnect_now`] is not listened to. Other handle
    /// contracts (sending · closing · drain and close) as a dialed peer.
    ///
    /// ★Auth deadline = [`Accepted::upgraded_at`] + [`Policy::connect_timeout`]★ (TRD 3 §5 3) — listener ready-queue time counts. If already passed on putting on,
    /// the handshake does not run and it closes (`cause` = `Idle`). So the listener must stamp `upgraded_at` with this registry's [`Clock`] (a debug build asserts a future time).
    ///
    /// ★When it ends it removes itself from the registry★ — after dropping the channel and before teardown (which waits for the receiver end to drain); a
    /// same-name replacement in the meantime is left alone. It removes itself even if the supervisor ends abnormally, and a peer ending at its first poll (deadline
    /// already passed · peer already cut) does not remain either — the supervisor starts after raising.
    ///
    /// ★Not in the generation table★ — the receiving name tag is new per connection (TRD 3 §3-1 ① `conn-<ConnId>`), so it would grow with connections. The counter
    /// is the table's if the name tag was raised with `add`, else the replaced handle's, else new — re-putting a name tag already on the registry does not rewind
    /// generations even when simultaneous. ★Generations start again from 1 only for a name tag never raised with `add` and not on the registry now★ — first-time, or re-put after it
    /// removed itself or was removed with [`Registry::remove`] (on the premise that receiving name tags are not reused).
    ///
    /// ★The upgrade permit ([`crate::ConnPermit`] of [`Accepted`]) is released on entering the operating phase★ — if it ends without entering, where the channel is dropped
    /// (before the receiver end drains — TRD 3 §5 14 「a peer that failed auth releases its permit as soon as it closes」). The auth-wait permit does not exist yet (3-3 U4),
    /// so this one covers that role; when it arrives, this holds it and releases the upgrade permit.
    ///
    /// After [`Registry::close_all`] it does not raise onto the registry and closes before the handshake — loads a close ([`CloseCode::GOING_AWAY`]) and drops it
    /// without reading or writing the channel.
    pub fn attach_with_policy(
        &self,
        id: PeerId,
        accepted: Accepted,
        handshake: Arc<dyn Handshake>,
        policy: Policy,
    ) -> (Peer<W>, Inbox<W>) {
        debug_assert_eq!(
            policy.check(),
            Ok(()),
            "the policy value violates its own invariants"
        );
        let Accepted {
            tx,
            rx,
            info: _,
            upgraded_at,
            permit,
        } = accepted;
        debug_assert!(
            upgraded_at <= self.inner.clock.now(),
            "listener/registry clock mismatch — the listener stamped upgraded_at with a clock different from the registry's"
        );
        let (inbound, inbox) = mpsc::channel(policy.inbound_queue.max(1));
        let origin = Origin::Accepted(AcceptedLink {
            tx,
            rx,
            upgraded_at,
            permit,
            leave: self.leave_roster(id.clone()),
        });
        let peer = self.launch(id.clone(), false, |generations| PeerConfig {
            id,
            origin,
            wire: self.inner.wire.clone(),
            clock: self.inner.clock.clone(),
            handshake,
            policy,
            inbound,
            byte_budget: BYTE_BUDGET,
            generations,
        });
        (peer, Inbox { rx: inbox })
    }

    /// The common tail of `add` · `attach` — builds the supervisor and handle and starts the supervisor **after** raising it on the registry. `record` = if the
    /// name tag was not in the table, put the chosen counter there (`add`).
    ///
    /// ★Raising comes before starting★ — reversed, an accepted peer ending at its first poll (deadline passed · peer already cut) could finish even removing itself
    /// on another worker and then be raised, leaving the ended handle forever (the name tag is new per connection, so nobody removes it again). A folded registry is
    /// also told apart before starting and the close put in first, so the supervisor sees it at its first poll and neither dials nor handshakes (first line of `dial` ·
    /// `shake_hands` in `peer.rs`); reversed, a supervisor running first on another worker would dial once or start a handshake.
    ///
    /// ★Choosing the counter · taking out the handle to replace · raising is one critical section★ — split, one of two simultaneous raises of the same name tag could
    /// make a new counter without seeing the other's taken handle or the table field and draw an already issued generation again.
    ///
    /// ★The table changes only after building (`prepare_peer`) finishes★ — a panic while building (e.g. the injected clock) leaves the registry untouched, so the
    /// handle to be replaced stays raised and running. The table change afterwards has no place to panic (`roster`).
    fn launch(
        &self,
        id: PeerId,
        record: bool,
        config: impl FnOnce(Arc<AtomicU64>) -> PeerConfig<W>,
    ) -> Peer<W> {
        let runtime = tokio::runtime::Handle::current();
        let (peer, supervisor, displaced, folded) = {
            let mut roster = self.inner.roster();
            let generations = roster.counter(&id);
            let (peer, supervisor) = prepare_peer(config(generations.clone()));
            if record {
                roster.generations.entry(id.clone()).or_insert(generations);
            }
            let displaced = roster.peers.remove(&id);
            let folded = roster.folded;
            if !folded {
                roster.peers.insert(id, peer.clone());
            }
            (peer, supervisor, displaced, folded)
        };
        if let Some(old) = displaced {
            old.close();
        }
        if folded {
            peer.close();
        }
        supervisor.start(&runtime);
        peer
    }

    /// How an accepted peer removes itself when it ends — only if that name tag still points to that handle. ★Weak★: it does not hold the registry (if the registry vanished first, nothing to remove).
    fn leave_roster(&self, id: PeerId) -> LeaveRoster<W> {
        let registry = Arc::downgrade(&self.inner);
        Box::new(move |me: &Peer<W>| {
            let Some(registry) = registry.upgrade() else {
                return;
            };
            let _left = {
                let mut roster = registry.roster();
                match roster.peers.get(&id) {
                    Some(listed) if listed.same(me) => roster.peers.remove(&id),
                    _ => None,
                }
            };
        })
    }

    /// Removes it from the registry and closes that peer; its receiver end ends `Closed` → `None` when the supervisor finishes teardown.
    pub fn remove(&self, id: &PeerId) {
        let removed = self.inner.roster().peers.remove(id);
        if let Some(peer) = removed {
            peer.close();
        }
    }

    /// Removes all peers from the registry and, after each drains the outbound work it had accepted, closes ([`Peer::close_after_flush`] ·
    /// [`CloseCode::GOING_AWAY`]) — waits for all those supervisors until `deadline` (the registry's clock) (TRD 3 §3-0 ④ — the daemon shutdown waits on this).
    /// `Err` = not all finished by the deadline; the rest keep folding and finish at their own deadlines ([`Policy::flush_timeout`] · the teardown limit).
    ///
    /// ★It drains rather than closing at once★ — so the last answer put in just before closing (e.g. a stop Ack) goes out; to close at once use [`Registry::remove`].
    ///
    /// ★Waits only for peers on the registry at the call★ — a supervisor still folding because `add` replaced it or `remove` took it out earlier is not known.
    ///
    /// ★It folds the registry — it does not reopen★ — [`Registry::add`] · [`Registry::attach`] afterwards do not get onto the registry and are closed before
    /// dialing or handshaking (receiver end ends `Closed` → `None`; an accepted channel loads a close and is dropped unread). This function does not wait for that.
    /// It keeps a peer the accept loop put on across the fold from remaining unclosed on the emptied registry; still stand up the accept loop first.
    ///
    /// ★The close starts at the call★ — the returned future only waits; dropping it, the close continues (as [`Peer::close_after_flush`]). The deadline runs from its first poll.
    ///
    /// ★Do not wait on the task that drains those receiver ends★ — a supervisor's end depends on the receiver draining (crate header 「known limits」), so that
    /// supervisor lingers to the teardown limit and this wait goes to `deadline`. Keep draining to the end (`None`); if released, that peer's remaining outbound work is
    /// dropped (crate header 「Receiver-side invariants」 5).
    pub fn close_all(
        &self,
        deadline: Duration,
    ) -> impl Future<Output = Result<(), Elapsed>> + Send + 'static {
        let peers: Vec<Peer<W>> = {
            let mut roster = self.inner.roster();
            roster.folded = true;
            roster.peers.drain().map(|(_, peer)| peer).collect()
        };
        let ends: Vec<_> = peers
            .iter()
            .map(|peer| peer.close_after_flush(CloseCode::GOING_AWAY, "going away"))
            .collect();
        let clock = self.inner.clock.clone();
        async move {
            with_deadline(&*clock, deadline, futures_util::future::join_all(ends))
                .await
                .map(|_| ())
        }
    }

    pub fn get(&self, id: &PeerId) -> Option<Peer<W>> {
        self.inner.roster().peers.get(id).cloned()
    }

    /// A snapshot of now, sorted by name tag — ★the ingredient the shell uses to fold 「the whole app」★.
    pub fn peers(&self) -> Vec<(PeerId, PeerState)> {
        let mut out: Vec<(PeerId, PeerState)> = self
            .inner
            .roster()
            .peers
            .iter()
            .map(|(id, peer)| (id.clone(), peer.peer_state()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn len(&self) -> usize {
        self.inner.roster().peers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Pushes the same thing to all peers. ★Encoded once and the frame handed out★ — no `Clone` on `W::Out`; in exchange, the fanout has **fixed notification
    /// semantics** (no reply wait).
    ///
    /// A partial failure comes up as that peer's [`TransportEvent::Dropped`](crate::TransportEvent::Dropped) **on that peer's receiver end** — ★not creating
    /// something that disappears silently is the one tricky part★ (today's `net` fanout is silent there). That peer's supervisor counts and reports, through
    /// the same ordering area as its other output, so the report is not dropped even when the line is full (`Peer::send_frame`).
    ///
    /// ★What could not be put in because a peer the app called [`Peer::close`] on is not operating is not reported★, even if it remains on the registry (user
    /// decision 2026-09-26 — `lib.rs` 「known limits」); what was in the queue earlier is counted by the supervisor.
    pub fn broadcast(&self, out: W::Out) {
        let frame = self.inner.wire.encode(&out);
        let targets: Vec<Peer<W>> = self.inner.roster().peers.values().cloned().collect();
        for peer in targets {
            let _ = peer.send_frame(frame.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Arrival, Direction, TransportEvent};
    use crate::frame::{Close, Frame};
    use crate::link::{
        HandshakeStep, LinkInfo, LinkRead, LinkRx, LinkTx, Listener, PermitPool, RejectKind,
    };
    use crate::machine::{GaveUpReason, Generation};
    use crate::peer::CLOSE_DRAIN_LIMIT;
    use crate::stream::StreamMark;
    use crate::testing::{
        memory_link, settle, settle_until, ClientMsg, HelloHandshake, ImmediateHandshake,
        ManualClock, MemoryConnector, MemoryLinkControl, MemoryListener, MemoryNetwork,
        RejectingHandshake, TestDecodeError, TestIn, TestOut, TestWire,
    };
    use futures_util::future::poll_immediate;
    use std::time::{Duration, Instant};

    struct Fixture {
        registry: Registry<TestWire>,
        net: MemoryNetwork,
        clock: ManualClock,
    }

    impl Fixture {
        fn new(policy: Policy) -> Self {
            let net = MemoryNetwork::new();
            let clock = ManualClock::new();
            let registry = Registry::new(Arc::new(TestWire), net.dialer(), clock.handle(), policy);
            Self {
                registry,
                net,
                clock,
            }
        }

        fn add(
            &self,
            name: &str,
            handshake: Arc<dyn Handshake>,
        ) -> (Peer<TestWire>, Inbox<TestWire>) {
            self.registry.add(
                PeerId::new(name),
                Address::new(format!("mem://{name}")),
                handshake,
            )
        }
    }

    fn immediate() -> Arc<dyn Handshake> {
        Arc::new(ImmediateHandshake)
    }

    struct PanickingHandshake;

    impl Handshake for PanickingHandshake {
        fn start(&self) -> HandshakeStep {
            panic!("test: panics on purpose at the handshake start");
        }

        fn on_frame(&self, _frame: &Frame) -> HandshakeStep {
            HandshakeStep::Done
        }
    }

    fn notice(body: &str) -> Frame {
        TestWire::encode_in(&TestIn::Notice(body.into()))
    }

    fn drain(inbox: &mut Inbox<TestWire>) -> Vec<Incoming<TestWire>> {
        let mut out = Vec::new();
        while let Some(item) = inbox.try_recv() {
            out.push(item);
        }
        out
    }

    /// Runs the supervisors and drains — letting them move what they hold in the ordering area into the line — until nothing comes up in one turn.
    async fn drain_until_quiet(inbox: &mut Inbox<TestWire>) -> Vec<Incoming<TestWire>> {
        let mut out = Vec::new();
        loop {
            settle().await;
            let batch = drain(inbox);
            if batch.is_empty() {
                return out;
            }
            out.extend(batch);
        }
    }

    /// Has it ended now — nothing held and the enqueue end released (or closed). ★Something held is taken out and discarded★ — call it after taking everything out.
    async fn ended(inbox: &mut Inbox<TestWire>) -> bool {
        matches!(poll_immediate(inbox.recv()).await, Some(None))
    }

    fn label(item: &Incoming<TestWire>) -> String {
        match item {
            Incoming::Message(arrival) => match &arrival.msg {
                TestIn::Notice(body) => format!("msg:{body}"),
                other => format!("msg:{other:?}"),
            },
            Incoming::Event(TransportEvent::Connected { generation, .. }) => {
                format!("Connected:{}", generation.0)
            }
            Incoming::Event(TransportEvent::Disconnected {
                generation, cause, ..
            }) => format!("Disconnected:{}:{cause:?}", generation.0),
            Incoming::Event(TransportEvent::Closed {
                generation, cause, ..
            }) => format!("Closed:{}:{cause:?}", generation.0),
            Incoming::Event(other) => other.kind().to_string(),
        }
    }

    fn labels(items: &[Incoming<TestWire>]) -> Vec<String> {
        items.iter().map(label).collect()
    }

    fn kinds(items: &[Incoming<TestWire>]) -> Vec<&'static str> {
        items
            .iter()
            .filter_map(|i| i.as_event().map(|e| e.kind()))
            .collect()
    }

    fn chunk_seqs(items: &[Incoming<TestWire>]) -> Vec<u64> {
        items
            .iter()
            .filter_map(|i| match i.as_message().map(|a| &a.msg) {
                Some(TestIn::Chunk { seq, .. }) => Some(*seq),
                _ => None,
            })
            .collect()
    }

    fn outbound_losses(items: &[Incoming<TestWire>]) -> Vec<u64> {
        items
            .iter()
            .filter_map(|i| match i.as_event() {
                Some(TransportEvent::Dropped {
                    direction: Direction::Outbound,
                    count,
                    ..
                }) => Some(*count),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn every_arrival_carries_the_peer_it_came_from() {
        let f = Fixture::new(Policy::default());
        let (_a, mut inbox_a) = f.add("a", immediate());
        settle().await;
        let ea = f.net.accept().unwrap();
        let (_b, mut inbox_b) = f.add("b", immediate());
        settle().await;
        let eb = f.net.accept().unwrap();

        ea.push(notice("from-a"));
        eb.push(notice("from-b"));
        settle().await;

        for (inbox, name) in [(&mut inbox_a, "a"), (&mut inbox_b, "b")] {
            let items = drain(inbox);
            assert!(
                items.iter().all(|i| i.peer() == &PeerId::new(name)),
                "★only what that handle's supervisor emitted comes to the inbox★"
            );
            assert_eq!(
                labels(&items),
                vec!["Connected:1".to_string(), format!("msg:from-{name}")]
            );
        }
    }

    #[tokio::test]
    async fn messages_and_events_share_one_line() {
        let f = Fixture::new(Policy::default());
        let (_peer, mut inbox) = f.add("a", immediate());
        settle().await;
        let endpoint = f.net.accept().unwrap();
        endpoint.push(notice("x"));
        settle().await;

        let items = drain(&mut inbox);
        assert!(items.iter().any(|i| i.as_event().is_some()));
        assert!(items.iter().any(|i| i.as_message().is_some()));
    }

    #[tokio::test]
    async fn one_peer_giving_up_leaves_the_others_alone() {
        let f = Fixture::new(Policy::default());
        let (good, _good_inbox) = f.add("good", immediate());
        settle().await;
        let _endpoint = f.net.accept().unwrap();

        let (bad, mut bad_inbox) = f.add("bad", RejectingHandshake::new("nope"));
        settle().await;

        assert_eq!(bad.peer_state(), PeerState::GaveUp(GaveUpReason::Rejected));
        assert_eq!(good.peer_state(), PeerState::Live);
        assert_eq!(
            f.registry.peers(),
            vec![
                (
                    PeerId::new("bad"),
                    PeerState::GaveUp(GaveUpReason::Rejected)
                ),
                (PeerId::new("good"), PeerState::Live),
            ]
        );
        assert_eq!(kinds(&drain(&mut bad_inbox)), vec!["Rejected", "GaveUp"]);
    }

    #[tokio::test]
    async fn broadcast_reaches_everyone_with_a_single_encode() {
        let f = Fixture::new(Policy::default());
        let (_a, _inbox_a) = f.add("a", immediate());
        settle().await;
        let ea = f.net.accept().unwrap();
        let (_b, _inbox_b) = f.add("b", immediate());
        settle().await;
        let eb = f.net.accept().unwrap();

        f.registry.broadcast(TestOut::Notice("all".into()));
        settle().await;
        assert_eq!(ea.frames(), vec![Frame::Text("not|all".into())]);
        assert_eq!(eb.frames(), vec![Frame::Text("not|all".into())]);
    }

    #[tokio::test]
    async fn a_partial_broadcast_failure_shows_up_as_that_peers_event() {
        let f = Fixture::new(Policy::default());
        let (_up, mut up_inbox) = f.add("up", immediate());
        settle().await;
        let ea = f.net.accept().unwrap();
        let (down, mut down_inbox) = f.add("down", RejectingHandshake::new("nope"));
        settle().await;
        assert!(matches!(down.peer_state(), PeerState::GaveUp(_)));
        drain(&mut up_inbox);
        drain(&mut down_inbox);

        f.registry.broadcast(TestOut::Notice("all".into()));
        settle().await;
        assert_eq!(ea.frames(), vec![Frame::Text("not|all".into())]);
        assert_eq!(outbound_losses(&drain(&mut down_inbox)), vec![1]);
        assert!(
            outbound_losses(&drain(&mut up_inbox)).is_empty(),
            "the inbox of an accepted peer has no report"
        );
    }

    #[tokio::test]
    async fn remove_closes_the_peer_and_drops_it_from_the_roster() {
        let f = Fixture::new(Policy::default());
        let (peer, mut inbox) = f.add("a", immediate());
        settle().await;
        assert_eq!(f.registry.len(), 1);
        assert!(f.registry.get(&PeerId::new("a")).is_some());

        f.registry.remove(&PeerId::new("a"));
        settle().await;
        assert!(f.registry.is_empty());
        assert_eq!(peer.peer_state(), PeerState::Closed);
        assert_eq!(
            labels(&drain(&mut inbox)),
            vec![
                "Connected:1",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit"
            ]
        );
        assert!(ended(&mut inbox).await, "the inbox of a removed peer ends");
    }

    #[tokio::test]
    async fn re_adding_the_same_name_replaces_the_old_supervisor() {
        let f = Fixture::new(Policy::default());
        let (first, _first_inbox) = f.add("a", immediate());
        settle().await;
        let (second, _second_inbox) =
            f.registry
                .add(PeerId::new("a"), Address::new("mem://a2"), immediate());
        settle().await;
        assert_eq!(first.peer_state(), PeerState::Closed);
        assert_eq!(second.peer_state(), PeerState::Live);
        assert_eq!(f.registry.len(), 1);
    }

    // ── N7: raising the same name tag again does not rewind the generation ──
    #[tokio::test]
    async fn re_adding_the_same_name_never_rewinds_the_generation() {
        let f = Fixture::new(Policy::default());
        let (first, _first_inbox) = f.add("a", immediate());
        settle().await;
        assert_eq!(first.generation(), Generation(1));

        let (second, _second_inbox) =
            f.registry
                .add(PeerId::new("a"), Address::new("mem://a2"), immediate());
        settle().await;
        assert_eq!(
            second.generation(),
            Generation(2),
            "★rewinding to 0 makes the 「ignore below the max generation」 defense drop the new connection entirely★"
        );
    }

    #[tokio::test]
    async fn the_old_supervisor_is_told_to_close_before_the_new_one_dials() {
        let f = Fixture::new(Policy::default());
        let (first, _first_inbox) = f.add("a", immediate());
        settle().await;
        let (_second, _second_inbox) =
            f.registry
                .add(PeerId::new("a"), Address::new("mem://a2"), immediate());
        settle().await;
        assert_eq!(first.peer_state(), PeerState::Closed);
    }

    #[tokio::test]
    async fn per_peer_policy_overrides_the_registry_default() {
        let f = Fixture::new(Policy::default());
        let strict = Policy {
            reconnect: crate::policy::ReconnectPolicy {
                max_attempts: 1,
                jitter: 0.0,
                ..Default::default()
            },
            ..Policy::default()
        };
        f.net.fail_forever();
        let (_peer, _inbox) = f.registry.add_with_policy(
            PeerId::new("a"),
            Address::new("mem://a"),
            immediate(),
            strict,
        );
        settle().await;
        f.clock.advance(Duration::from_millis(500));
        settle().await;
        assert_eq!(
            f.registry.peers()[0].1,
            PeerState::GaveUp(GaveUpReason::BudgetExhausted),
            "the per-peer budget 1 beats the registry default 3"
        );
        assert_eq!(f.net.dials(), 2);
    }

    // ── N1: a full upward line neither cuts nor drops the connection ──
    #[tokio::test]
    async fn a_saturated_line_neither_cuts_the_link_nor_loses_anything() {
        let f = Fixture::new(Policy {
            inbound_queue: 2,
            ..Policy::default()
        });
        let (peer, mut inbox) = f.add("a", immediate());
        settle().await;
        let endpoint = f.net.accept().unwrap();
        for seq in 1..=200 {
            endpoint.push(TestWire::chunk(1, 1, seq));
        }
        settle().await;
        assert_eq!(
            peer.peer_state(),
            PeerState::Live,
            "having no room to go up is not this connection's fault"
        );
        endpoint.close();
        settle().await;

        let items = drain_until_quiet(&mut inbox).await;
        assert_eq!(
            chunk_seqs(&items),
            (1..=200).collect::<Vec<_>>(),
            "★even what came while the queue was full is not dropped and comes out in arrival order★"
        );
        assert_eq!(
            kinds(&items),
            vec!["Connected", "Disconnected"],
            "★the disconnect comes after what came before it, and there is no loss report★"
        );
        assert!(
            matches!(
                items.last().and_then(|i| i.as_event()),
                Some(TransportEvent::Disconnected { .. })
            ),
            "the disconnect is last"
        );
    }

    #[tokio::test]
    async fn a_replay_sized_burst_arrives_whole_while_the_consumer_lags() {
        let f = Fixture::new(Policy::default());
        let (_peer, mut inbox) = f.add("a", immediate());
        settle().await;
        let endpoint = f.net.accept().unwrap();
        for seq in 1..=4096 {
            endpoint.push(TestWire::chunk(1, 1, seq));
        }
        settle().await;
        assert!(
            endpoint.unread() > 0,
            "★while the receiver lags, the supervisor reads only up to the queue · read-ahead queue amount and leaves the rest on the link★ (backpressure)"
        );

        let items = drain_until_quiet(&mut inbox).await;
        assert_eq!(chunk_seqs(&items), (1..=4096).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn a_flooding_peer_does_not_sever_a_healthy_peer() {
        let f = Fixture::new(Policy {
            inbound_queue: 2,
            ..Policy::default()
        });
        let (noisy, _noisy_inbox) = f.add("noisy", immediate());
        settle().await;
        let noisy_link = f.net.accept().unwrap();
        for seq in 1..=500 {
            noisy_link.push(TestWire::chunk(1, 1, seq));
        }
        settle().await;

        let (quiet, mut quiet_inbox) = f.add("quiet", immediate());
        settle().await;
        assert_eq!(
            quiet.peer_state(),
            PeerState::Live,
            "★if someone else's flood cut my connection, ADR-0180 decision 8 would break★"
        );
        assert_eq!(noisy.peer_state(), PeerState::Live);
        assert_eq!(f.net.dials(), 2, "the reconnect did not run");
        assert_eq!(
            kinds(&drain(&mut quiet_inbox)),
            vec!["Connected"],
            "someone else's flood cannot make my Connected be dropped"
        );
    }

    // ── 3-0 U1d: a receiver end that is not drained stalls only that peer ──
    #[tokio::test]
    async fn an_undrained_inbox_stalls_only_its_own_peer() {
        let f = Fixture::new(Policy {
            inbound_queue: 64,
            frame_buffer: 8,
            ..Policy::default()
        });
        let (noisy, mut noisy_inbox) = f.add("noisy", immediate());
        settle().await;
        let noisy_link = f.net.accept().unwrap();
        for seq in 1..=500 {
            noisy_link.push(TestWire::chunk(1, 1, seq));
        }
        settle().await;

        let (quiet, mut quiet_inbox) = f.add("quiet", immediate());
        settle().await;
        let quiet_link = f.net.accept().unwrap();
        for n in 0..3 {
            quiet_link.push(notice(&format!("q{n}")));
        }
        settle().await;
        assert!(
            noisy_link.unread() > 0,
            "★the peer of an undrained inbox gets backpressure on its own link★ — once the queue is full, that link is not read"
        );
        assert_eq!(
            quiet_link.unread(),
            0,
            "★the quiet side's link is read completely regardless of others' backpressure★"
        );
        assert_eq!(
            labels(&drain(&mut quiet_inbox)),
            vec!["Connected:1", "msg:q0", "msg:q1", "msg:q2"],
            "★nothing of the quiet peer is dropped, and even with the noisy side undrained, everything is there within one settle★"
        );
        assert_eq!(noisy.peer_state(), PeerState::Live);
        assert_eq!(quiet.peer_state(), PeerState::Live);

        let noisy_items = drain_until_quiet(&mut noisy_inbox).await;
        assert_eq!(chunk_seqs(&noisy_items), (1..=500).collect::<Vec<_>>());
    }

    // ── 3-0 U1c: the registry hands the byte budget to the supervisor ──
    //
    // ★Measure with the real budget value★ — supervisor tests pass a small budget directly, so a wrong value from the registry (0 · no limit) would not show there.
    //   For a non-stream peer the whole budget (16 MiB) is the read share, so two frames over half of it close the read gate at the second (same shape as the net
    //   `CONN_RX_MAX_BYTES` test — same value).
    #[tokio::test]
    async fn the_registry_stops_reading_a_peer_once_it_holds_the_byte_budget() {
        let f = Fixture::new(Policy::default());
        let (peer, mut inbox) = f.add("a", immediate());
        settle().await;
        let endpoint = f.net.accept().unwrap();
        drain(&mut inbox);
        let big = BYTE_BUDGET / 2 + 1;
        for tag in 0..3u8 {
            endpoint.push(Frame::Binary(vec![tag; big]));
        }
        settle().await;
        assert_eq!(
            endpoint.unread(),
            1,
            "★the second item exceeds the read share and the third stays on the link★"
        );
        assert_eq!(peer.held_bytes(), 2 * big);

        let tags: Vec<u8> = drain_until_quiet(&mut inbox)
            .await
            .iter()
            .filter_map(|i| match i.as_message().map(|a| &a.msg) {
                Some(TestIn::Blob(bytes)) => Some(bytes[0]),
                _ => None,
            })
            .collect();
        assert_eq!(tags, vec![0, 1, 2], "★0 loss · order as is★");
        assert_eq!(peer.held_bytes(), 0);
    }

    #[tokio::test]
    async fn each_peer_line_takes_its_length_from_that_peers_policy() {
        let f = Fixture::new(Policy {
            inbound_queue: 2,
            ..Policy::default()
        });
        let (_peer, mut inbox) = f.registry.add_with_policy(
            PeerId::new("roomy"),
            Address::new("mem://r"),
            immediate(),
            Policy {
                inbound_queue: 8,
                ..Policy::default()
            },
        );
        settle().await;
        let link = f.net.accept().unwrap();
        for n in 0..6 {
            link.push(notice(&format!("n{n}")));
        }
        settle().await;
        let messages = drain(&mut inbox)
            .iter()
            .filter(|i| i.as_message().is_some())
            .count();
        assert_eq!(
            messages, 6,
            "the per-peer 8, not the registry default 2, is the length of that queue"
        );
    }

    #[tokio::test]
    async fn a_broadcast_loss_report_is_not_crowded_out_by_another_peers_flood() {
        let f = Fixture::new(Policy {
            inbound_queue: 4,
            ..Policy::default()
        });
        let (_noisy, _noisy_inbox) = f.add("noisy", immediate());
        settle().await;
        let noisy_link = f.net.accept().unwrap();
        for seq in 1..=500 {
            noisy_link.push(TestWire::chunk(1, 1, seq));
        }
        settle().await;
        let (_down, mut down_inbox) = f.add("down", RejectingHandshake::new("nope"));
        settle().await;
        drain(&mut down_inbox);

        f.registry.broadcast(TestOut::Notice("all".into()));
        settle().await;
        assert_eq!(outbound_losses(&drain(&mut down_inbox)), vec![1]);
    }

    // ── 3-0 U1d: fanout loss is counted by that peer's supervisor and emitted to its own receiver end ──
    //
    // ★Do not advance the clock★ — a backoff wait woken by the clock leaves the report stuck at the turn start, and we could not tell whether the supervisor woke by
    //   itself. Line length 1 = a line filled by one `ConnectFailed`.
    #[tokio::test]
    async fn a_broadcast_loss_wakes_a_backing_off_supervisor_and_survives_a_full_inbox() {
        let f = Fixture::new(Policy {
            inbound_queue: 1,
            ..Policy::default()
        });
        f.net.fail_forever();
        let (down, mut inbox) = f.add("down", immediate());
        settle_until("backoff", || down.peer_state() == PeerState::Backoff).await;

        f.registry.broadcast(TestOut::Notice("all".into()));
        settle().await;
        let items = drain_until_quiet(&mut inbox).await;
        assert_eq!(
            kinds(&items),
            vec!["ConnectFailed", "Dropped"],
            "★even with a full inbox, reports are not dropped and come in the order the supervisor emitted them★"
        );
        assert_eq!(outbound_losses(&items), vec![1]);
        assert_eq!(
            f.net.dials(),
            1,
            "it is not a redial because the backoff ended"
        );
    }

    #[tokio::test]
    async fn a_waiting_recv_wakes_for_its_first_event_and_for_later_messages() {
        let f = Fixture::new(Policy::default());
        let (_peer, mut inbox) = f.add("late", immediate());
        let waiting = tokio::spawn(async move {
            let first = inbox.recv().await;
            (first, inbox)
        });
        settle().await;
        let (first, mut inbox) = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("the waiting recv did not see the first event — a permanent wait regression")
            .expect("the receiving task panicked");
        assert!(matches!(
            first.as_ref().and_then(|i| i.as_event()),
            Some(TransportEvent::Connected { peer, .. }) if peer == &PeerId::new("late")
        ));

        let link = f.net.accept().unwrap();
        let waiting = tokio::spawn(async move { inbox.recv().await });
        settle().await;
        assert!(!waiting.is_finished());
        link.push(notice("wake"));
        let next = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("the waiting recv did not see the new item")
            .expect("the receiving task panicked");
        assert!(matches!(
            next.as_ref().and_then(|i| i.as_message()).map(|a| &a.msg),
            Some(TestIn::Notice(body)) if body == "wake"
        ));
    }

    #[tokio::test]
    async fn closing_an_inbox_folds_its_peer_and_keeps_what_was_queued() {
        let f = Fixture::new(Policy::default());
        let (early, mut inbox) = f.add("a", immediate());
        settle().await;
        let link = f.net.accept().unwrap();
        let (other, _other_inbox) = f.add("b", immediate());
        settle().await;
        link.push(notice("before"));
        settle().await;

        inbox.close();
        link.push(notice("after"));
        settle_until("a folded", || early.peer_state() == PeerState::Closed).await;
        assert_eq!(
            other.peer_state(),
            PeerState::Live,
            "★closing one inbox leaves the other peers as they are★"
        );
        assert_eq!(
            f.net.dials(),
            2,
            "a missing consumer is not a reason to reconnect"
        );

        let items = drain(&mut inbox);
        assert_eq!(
            labels(&items),
            vec!["Connected:1", "msg:before"],
            "what went in before the close can be taken out"
        );
        assert!(
            ended(&mut inbox).await,
            "a closed inbox ends once everything is taken out"
        );
    }

    #[tokio::test]
    async fn dropping_an_inbox_folds_its_peer_and_a_re_added_name_starts_afresh() {
        let f = Fixture::new(Policy::default());
        let (early, inbox) = f.add("a", immediate());
        settle().await;
        let link = f.net.accept().unwrap();
        drop(inbox);
        link.push(notice("nobody listens"));
        settle_until("a folded", || early.peer_state() == PeerState::Closed).await;
        assert_eq!(
            f.net.dials(),
            1,
            "a peer whose receiver vanished does not redial"
        );

        let (again, mut fresh) = f.add("a", immediate());
        settle_until("re-added a operating", || {
            again.peer_state() == PeerState::Live
        })
        .await;
        assert_eq!(
            f.net.dials(),
            2,
            "★releasing the old inbox does not spread to the new handle with the same label★"
        );
        assert_eq!(labels(&drain(&mut fresh)), vec!["Connected:2"]);
    }

    // ── 3-0 U1d: even with a quiet peer, closing or releasing the receiver end folds without a next frame ──
    #[tokio::test]
    async fn closing_an_inbox_on_a_silent_peer_folds_it_and_still_drains_what_was_queued() {
        let f = Fixture::new(Policy::default());
        let (peer, mut inbox) = f.add("a", immediate());
        settle_until("connection", || peer.peer_state() == PeerState::Live).await;
        let link = f.net.accept().unwrap();
        link.push(notice("before"));
        settle().await;

        inbox.close();
        settle().await;
        assert!(
            peer.state().has_changed().is_err(),
            "★the supervisor folded and finished cleanup even with nothing to put in★ — it waited for neither the next frame nor the cap"
        );
        assert_eq!(peer.peer_state(), PeerState::Closed);
        assert_eq!(f.net.dials(), 1, "did not redial");
        assert_eq!(
            labels(&drain(&mut inbox)),
            vec!["Connected:1", "msg:before"],
            "everything that went in before the close can be taken out"
        );
        assert!(
            ended(&mut inbox).await,
            "after that is the end — `Closed` had nowhere to be delivered"
        );
    }

    // Why 30 times: without the check before polling the dial phase, the dial future and the closed arm become ready together and which is polled first is random
    //   (`select!`) — one run could miss that the check is gone.
    #[tokio::test]
    async fn an_inbox_dropped_before_its_supervisor_runs_never_dials() {
        let f = Fixture::new(Policy::default());
        for n in 0..30 {
            let (peer, inbox) = f.add(&format!("a{n}"), immediate());
            drop(inbox);
            settle().await;
            assert_eq!(peer.peer_state(), PeerState::Closed, "{n}");
            assert!(
                peer.state().has_changed().is_err(),
                "{n}: the supervisor ended"
            );
        }
        assert_eq!(
            f.net.dials(),
            0,
            "★if the inbox is released before even the first event is put in, the link is never dialed★"
        );
    }

    // ── 3-0 U1d: if the same name tag is raised again, the old receiver end ends at the old supervisor's end and the new receiver end receives only the new supervisor's ──
    #[tokio::test]
    async fn a_re_added_name_ends_the_old_inbox_and_starts_a_new_one() {
        let f = Fixture::new(Policy::default());
        let (_old, mut old_inbox) = f.add("a", RejectingHandshake::new("nope"));
        settle().await;
        let (_new, mut new_inbox) =
            f.registry
                .add(PeerId::new("a"), Address::new("mem://a2"), immediate());
        settle().await;

        assert_eq!(
            kinds(&drain(&mut old_inbox)),
            vec!["Rejected", "GaveUp", "Closed"]
        );
        assert!(
            ended(&mut old_inbox).await,
            "★the old inbox ends after the old supervisor's Closed★"
        );
        assert_eq!(
            labels(&drain(&mut new_inbox)),
            vec!["Connected:1"],
            "★only the new supervisor's items come to the new inbox★"
        );
        assert!(!ended(&mut new_inbox).await);
    }

    // ★The old supervisor emits `Closed` only after fully closing the channel, and meanwhile the new one attaches★ — with one line the two would mix and an
    //   item's supervisor would have to be told by generation.
    #[tokio::test]
    async fn a_name_re_added_while_its_old_supervisor_is_still_closing_splits_by_inbox() {
        let f = Fixture::new(Policy::default());
        let (_old, mut old_inbox) = f.add("a", immediate());
        settle().await;
        drain(&mut old_inbox);
        f.registry.remove(&PeerId::new("a"));
        let (_new, mut new_inbox) =
            f.registry
                .add(PeerId::new("a"), Address::new("mem://a2"), immediate());
        settle().await;

        assert_eq!(
            labels(&drain(&mut old_inbox)),
            vec!["Disconnected:1:Explicit", "Closed:1:Explicit"],
            "inside the old supervisor the order is disconnect → end"
        );
        assert!(ended(&mut old_inbox).await);
        assert_eq!(
            labels(&drain(&mut new_inbox)),
            vec!["Connected:2"],
            "★the old supervisor's tail does not mix into the new inbox★"
        );
    }

    #[tokio::test]
    async fn every_inbox_ends_with_closed_then_none_even_when_read_after_later_re_adds() {
        let f = Fixture::new(Policy {
            inbound_queue: 4,
            ..Policy::default()
        });
        let name = PeerId::new("a");
        let mut inboxes = Vec::new();
        for _ in 0..3 {
            let (_peer, inbox) = f
                .registry
                .add(name.clone(), Address::new("mem://a"), immediate());
            inboxes.push(inbox);
            settle().await;
            f.registry.remove(&name);
            settle().await;
        }
        for (at, inbox) in inboxes.iter_mut().enumerate() {
            let g = at + 1;
            assert_eq!(
                labels(&drain(inbox)),
                vec![
                    format!("Connected:{g}"),
                    format!("Disconnected:{g}:Explicit"),
                    format!("Closed:{g}:Explicit"),
                ],
                "★Closed is the last item★ — items of a supervisor added later do not mix in"
            );
            assert!(
                ended(inbox).await,
                "★after Closed the supervisor releases the enqueue end, so it is None★"
            );
        }
    }

    #[tokio::test]
    async fn churning_a_name_before_its_supervisor_runs_never_dials_and_ends_every_inbox() {
        let f = Fixture::new(Policy::default());
        let name = PeerId::new("a");
        let mut inboxes = Vec::new();
        for _ in 0..30 {
            let (_peer, inbox) = f
                .registry
                .add(name.clone(), Address::new("mem://a"), immediate());
            f.registry.remove(&name);
            inboxes.push(inbox);
            settle().await;
        }
        assert_eq!(f.net.dials(), 0);
        for inbox in &mut inboxes {
            assert_eq!(kinds(&drain(inbox)), vec!["Closed"]);
            assert!(ended(inbox).await);
        }
    }

    // ── 3-0 U1d: if teardown exceeds the limit, the receiver end ends without `Closed` ──
    //
    // Line length 1 = a line filled by one `Connected` — the notifications · disconnect · `Closed` after it wait in the supervisor's ordering area.
    #[tokio::test]
    async fn an_inbox_nobody_drains_ends_without_closed_once_the_teardown_gives_up() {
        for (waited, expected) in [
            (
                CLOSE_DRAIN_LIMIT - Duration::from_secs(1),
                vec![
                    "Connected:1",
                    "msg:held",
                    "Disconnected:1:Explicit",
                    "Closed:1:Explicit",
                ],
            ),
            (CLOSE_DRAIN_LIMIT, vec!["Connected:1"]),
        ] {
            let f = Fixture::new(Policy {
                inbound_queue: 1,
                ..Policy::default()
            });
            let (peer, mut inbox) = f.add("a", immediate());
            settle_until("connection", || peer.peer_state() == PeerState::Live).await;
            let link = f.net.accept().unwrap();
            link.push(notice("held"));
            settle().await;
            peer.close();
            settle_until("folded", || peer.peer_state() == PeerState::Closed).await;
            f.clock.advance(waited);
            settle().await;

            let items = drain_until_quiet(&mut inbox).await;
            assert_eq!(labels(&items), expected, "after waiting {waited:?}");
            assert!(
                ended(&mut inbox).await,
                "{waited:?}: ★the supervisor released the enqueue end — if it passed the cap, `None` without `Closed` is that drop★"
            );
        }
    }

    #[tokio::test]
    async fn dropping_an_old_inbox_ends_its_supervisors_teardown_without_waiting_for_the_limit() {
        let f = Fixture::new(Policy {
            inbound_queue: 1,
            ..Policy::default()
        });
        let (old, old_inbox) = f.add("a", immediate());
        settle_until("connection", || old.peer_state() == PeerState::Live).await;
        let link = f.net.accept().unwrap();
        link.push(notice("held"));
        settle().await;
        let (_new, _new_inbox) =
            f.registry
                .add(PeerId::new("a"), Address::new("mem://a2"), immediate());
        settle().await;
        assert_eq!(old.peer_state(), PeerState::Closed);
        assert!(
            old.state().has_changed().is_ok(),
            "the old supervisor is cleaning up, waiting on the old inbox that has not been drained yet"
        );

        drop(old_inbox);
        settle().await;
        assert!(
            old.state().has_changed().is_err(),
            "★releasing the old inbox ends the old supervisor's cleanup without waiting for the cap★"
        );
    }

    // ── 3-0 U1a: fanout loss is reported to an open peer · an ended peer receives nothing even if it remains on the registry ──
    //
    // That the rejection after the app called close is not counted (user decision 2026-09-26) cannot be measured here — the supervisor and receiver end have
    //   ended, so a count would not show. The test right below measures that gap.
    #[tokio::test]
    async fn a_broadcast_loss_is_reported_for_an_open_peer_and_never_reaches_an_ended_one() {
        let f = Fixture::new(Policy::default());
        let (_down, mut down_inbox) = f.add("down", RejectingHandshake::new("nope"));
        let (shut, mut shut_inbox) = f.add("shut", immediate());
        settle().await;
        shut.close();
        settle_until("shut folded", || shut.peer_state() == PeerState::Closed).await;
        assert_eq!(
            f.registry.len(),
            2,
            "stays in the registry even though the app closed it"
        );
        drain(&mut down_inbox);
        drain(&mut shut_inbox);

        f.registry.broadcast(TestOut::Notice("all".into()));
        settle().await;
        assert_eq!(outbound_losses(&drain(&mut down_inbox)), vec![1]);
        assert!(drain(&mut shut_inbox).is_empty());
        assert!(ended(&mut shut_inbox).await);
    }

    // ★Measures the gap after close and before the supervisor handles it★ — a supervisor asleep in the give-up wait that gets control and a wake together has no
    //   fixed look order, so a handle-side count in that gap may load `Dropped` ahead of `Closed`. So close without yielding, push right away, and look at the
    //   handle-side count first.
    #[tokio::test]
    async fn a_broadcast_right_after_close_is_not_counted_before_the_supervisor_sees_the_close() {
        let f = Fixture::new(Policy::default());
        let (closing, mut inbox) = f.add("closing", RejectingHandshake::new("nope"));
        settle().await;
        assert!(matches!(closing.peer_state(), PeerState::GaveUp(_)));
        drain(&mut inbox);

        closing.close();
        f.registry.broadcast(TestOut::Notice("all".into()));
        assert_eq!(
            closing.refused_count(),
            0,
            "★refusals after the app called close are not counted★ (user decision 2026-09-26)"
        );
        settle().await;
        assert_eq!(kinds(&drain(&mut inbox)), vec!["Closed"]);
        assert!(ended(&mut inbox).await);
    }

    // ── 3-0 U1d: if the supervisor ends without a normal teardown, the receiver end ends without `Closed` ──
    #[tokio::test]
    async fn a_supervisor_that_ends_abnormally_ends_its_inbox_without_closed() {
        let f = Fixture::new(Policy::default());
        let (peer, mut inbox) = f.add("a", Arc::new(PanickingHandshake));
        settle_until("the supervisor task ended", || {
            peer.state().has_changed().is_err()
        })
        .await;
        assert!(
            drain(&mut inbox).is_empty(),
            "★it did not go through cleanup, so there is no `Closed`★"
        );
        assert!(
            ended(&mut inbox).await,
            "★the enqueue end dropped with the task, so it is `None` — it looks the same as exceeding the cap★"
        );
    }

    #[test]
    fn an_add_that_panics_outside_a_runtime_leaves_the_registry_usable() {
        let net = MemoryNetwork::new();
        let clock = ManualClock::new();
        let registry = Registry::new(
            Arc::new(TestWire),
            net.dialer(),
            clock.handle(),
            Policy::default(),
        );
        let spawned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            registry.add(PeerId::new("a"), Address::new("mem://a"), immediate())
        }));
        assert!(
            spawned.is_err(),
            "a supervisor cannot be started outside a runtime"
        );
        assert_eq!(
            registry.len(),
            0,
            "★a single panic does not make the registry unusable★"
        );
        registry.remove(&PeerId::new("a"));
    }

    #[tokio::test]
    async fn a_dropped_registry_leaves_held_peers_running_until_their_handles_go() {
        let Fixture {
            registry,
            net: _net,
            clock: _clock,
        } = Fixture::new(Policy::default());
        let (peer, mut inbox) =
            registry.add(PeerId::new("a"), Address::new("mem://a"), immediate());
        settle().await;
        drop(registry);
        settle().await;
        assert_eq!(
            peer.peer_state(),
            PeerState::Live,
            "★even if the registry vanishes, a peer holding a handle keeps running★"
        );

        drop(peer);
        settle().await;
        assert_eq!(
            labels(&drain(&mut inbox)),
            vec![
                "Connected:1",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit"
            ]
        );
        assert!(
            ended(&mut inbox).await,
            "when all handles are gone the inbox ends"
        );
    }

    // ── 3-0 U3: close_all ──

    #[tokio::test]
    async fn close_all_flushes_every_peer_then_closes_it_and_empties_the_roster() {
        let fx = Fixture::new(Policy::default());
        let (a, mut inbox_a) = fx.add("a", immediate());
        let (b, mut inbox_b) = fx.add("b", immediate());
        settle_until("both operating", || {
            a.peer_state().is_live() && b.peer_state().is_live()
        })
        .await;
        let endpoints = [fx.net.accept().unwrap(), fx.net.accept().unwrap()];
        // ★Block the write★ — the close must arrive while 「last」 is being written, to tell it from a regression that closes at once.
        for endpoint in &endpoints {
            endpoint.stall_writes(true);
        }
        a.notify(TestOut::Notice("last".into())).unwrap();
        b.notify(TestOut::Notice("last".into())).unwrap();

        let closing = tokio::spawn(fx.registry.close_all(Duration::from_secs(3)));
        settle().await;
        assert!(!closing.is_finished(), "draining the blocked write");
        for endpoint in &endpoints {
            endpoint.stall_writes(false);
        }
        settle_until("ended without advancing the clock", || {
            closing.is_finished()
        })
        .await;
        assert_eq!(closing.await.expect("close_all task"), Ok(()));
        assert!(fx.registry.is_empty(), "removed from the registry");
        for endpoint in &endpoints {
            assert_eq!(
                endpoint.drain(),
                vec![
                    ClientMsg::Frame(notice("last")),
                    ClientMsg::Close(Close::new(CloseCode::GOING_AWAY, "going away")),
                ],
                "★does not close right away but writes what was accepted earlier first★"
            );
        }
        for inbox in [&mut inbox_a, &mut inbox_b] {
            assert_eq!(
                labels(&drain(inbox)),
                vec![
                    "Connected:1",
                    "Disconnected:1:Explicit",
                    "Closed:1:Explicit"
                ]
            );
            assert!(ended(inbox).await);
        }
    }

    // ★Closes even if the future is not awaited★ — the close starts at the call; the future only waits.
    #[tokio::test]
    async fn close_all_closes_even_when_nobody_waits_for_it() {
        let fx = Fixture::new(Policy::default());
        let (peer, mut inbox) = fx.add("a", immediate());
        settle_until("operating", || peer.peer_state().is_live()).await;
        drop(fx.registry.close_all(Duration::ZERO));
        let items = drain_until_quiet(&mut inbox).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit"
            ]
        );
        assert!(ended(&mut inbox).await);
    }

    // ★Put close_all's deadline (1 s · 3 s) on both sides of the drain's deadline (2 s)★ — unfinished within it is `Err`, and the supervisor ends at its own deadline regardless.
    #[tokio::test]
    async fn close_all_waits_only_up_to_its_deadline() {
        assert_eq!(Policy::default().flush_timeout, Duration::from_secs(2));
        for (deadline, in_time) in [
            (Duration::from_secs(1), false),
            (Duration::from_secs(3), true),
        ] {
            let fx = Fixture::new(Policy::default());
            let (peer, mut inbox) = fx.add("a", immediate());
            settle_until("operating", || peer.peer_state().is_live()).await;
            let endpoint = fx.net.accept().unwrap();
            endpoint.stall_writes(true);
            peer.notify(TestOut::Notice("stuck".into())).unwrap();
            settle().await;

            let closing = tokio::spawn(fx.registry.close_all(deadline));
            settle().await;
            fx.clock.advance(Duration::from_secs(1));
            settle().await;
            assert_eq!(closing.is_finished(), !in_time, "{deadline:?}: 1 s later");
            fx.clock.advance(Duration::from_secs(1));
            settle_until("the supervisor ended at the drain's deadline", || {
                closing.is_finished()
            })
            .await;
            let expected = if in_time { Ok(()) } else { Err(Elapsed) };
            assert_eq!(
                closing.await.expect("close_all task"),
                expected,
                "{deadline:?}"
            );
            let items = drain_until_quiet(&mut inbox).await;
            assert_eq!(kinds(&items).last(), Some(&"Closed"), "{deadline:?}");
            assert!(ended(&mut inbox).await, "{deadline:?}");
        }
    }

    // ── 3-0 U6b: an accepted peer (`attach`) ──

    /// An auth window for these tests (`ManualClock`, so the value only sets the step size; the daemon's real window is 15 s, TRD 3 revision 9 ⑧).
    const AUTH: Duration = Duration::from_secs(1);
    const GOOD: &str = "auth|good";

    fn auth_policy() -> Policy {
        Policy {
            connect_timeout: AUTH,
            ..Policy::default()
        }
    }

    /// Receiving-side auth — the first frame must be [`GOOD`] to enter the operating phase; otherwise a reject that makes it redial (stale credentials — TRD 3 §3-0 ⑤).
    struct TokenGate;

    impl Handshake for TokenGate {
        fn start(&self) -> HandshakeStep {
            HandshakeStep::Await
        }

        fn on_frame(&self, frame: &Frame) -> HandshakeStep {
            match frame {
                Frame::Text(text) if text == GOOD => HandshakeStep::Done,
                _ => HandshakeStep::Reject {
                    kind: RejectKind::Retryable,
                    last: None,
                    reason: "bad token".into(),
                },
            }
        }
    }

    /// The receiving-side registry and its listener. ★The registry's Dialer is a net that only counts★ — it checks whether an accepted peer redials.
    struct Accepting {
        registry: Registry<TestWire>,
        redials: MemoryNetwork,
        clock: ManualClock,
        listener: MemoryListener,
        connector: MemoryConnector,
    }

    struct Near {
        tx: Box<dyn LinkTx>,
        rx: Box<dyn LinkRx>,
    }

    #[derive(Debug, PartialEq)]
    enum Seen {
        Frame(Frame),
        Closed(Option<Close>),
        Failed,
    }

    impl Accepting {
        fn new(policy: Policy) -> Self {
            let clock = ManualClock::new();
            let redials = MemoryNetwork::new();
            let registry =
                Registry::new(Arc::new(TestWire), redials.dialer(), clock.handle(), policy);
            let (listener, connector) = MemoryListener::new(clock.handle(), 8);
            Self {
                registry,
                redials,
                clock,
                listener,
                connector,
            }
        }

        async fn connect(&mut self) -> (Near, Accepted) {
            let (tx, rx) = self
                .connector
                .dial(&Address::new("mem://acceptor"))
                .await
                .expect("dial");
            let accepted = self.listener.accept().await.expect("receive");
            (Near { tx, rx }, accepted)
        }

        fn far(&self) -> MemoryLinkControl {
            self.connector.latest().expect("link").acceptor
        }

        fn attach(
            &self,
            name: &str,
            accepted: Accepted,
            handshake: Arc<dyn Handshake>,
        ) -> (Peer<TestWire>, Inbox<TestWire>) {
            self.registry.attach(PeerId::new(name), accepted, handshake)
        }

        fn generation_names(&self) -> usize {
            self.registry.inner.roster().generations.len()
        }
    }

    impl Near {
        async fn say(&mut self, text: &str) {
            self.tx
                .send(Frame::Text(text.into()))
                .await
                .expect("dialing-side write");
        }

        async fn arrived(&mut self) -> Vec<Seen> {
            let mut out = Vec::new();
            while let Some(read) = poll_immediate(self.rx.recv()).await {
                let seen = match read {
                    Ok(LinkRead::Frame(frame)) => Seen::Frame(frame),
                    Ok(LinkRead::Closed(close)) => Seen::Closed(close),
                    Err(_) => Seen::Failed,
                };
                let end = !matches!(seen, Seen::Frame(_));
                out.push(seen);
                if end {
                    break;
                }
            }
            out
        }
    }

    fn closing() -> Seen {
        Seen::Closed(Some(Close::new(CloseCode::GOING_AWAY, "closing")))
    }

    // ★See the end without advancing the clock★ — if it went to reconnect it would stand in the backoff wait and `Closed` would not come.
    #[tokio::test]
    async fn an_accepted_peer_that_loses_its_link_ends_with_that_cause_and_never_redials() {
        let mut a = Accepting::new(auth_policy());
        let (near, accepted) = a.connect().await;
        let (peer, mut inbox) = a.attach("conn-1", accepted, immediate());
        settle_until("operating", || peer.peer_state().is_live()).await;
        drop(near);
        let items = drain_until_quiet(&mut inbox).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "Disconnected:1:ReadFailed",
                "Closed:1:ReadFailed"
            ],
            "ends without dialing-side events (ConnectFailed · GaveUp)"
        );
        assert!(ended(&mut inbox).await);
        assert_eq!(peer.peer_state(), PeerState::Closed);
        assert!(
            a.registry.is_empty(),
            "removes itself from the registry when it ends"
        );
        a.clock.advance(Duration::from_secs(60));
        settle().await;
        assert_eq!(a.redials.dials(), 0, "an accepted peer does not redial");
    }

    // The daemon adapter tells the read end (finish remaining commands) from the write end (cut at once) by this cause (TRD 3 §3-0 ①).
    #[tokio::test]
    async fn read_end_and_write_end_causes_reach_closed_on_the_accepted_side() {
        let write_deadline = Policy::default().write_deadline;
        for case in [
            "the peer closed",
            "read error",
            "vanished silently",
            "write error",
            "write deadline",
            "silence",
        ] {
            let mut a = Accepting::new(auth_policy());
            let (mut near, accepted) = a.connect().await;
            let far = a.far();
            let (peer, mut inbox) = a.attach("conn-1", accepted, immediate());
            settle_until("operating", || peer.peer_state().is_live()).await;
            let expected = match case {
                "the peer closed" => {
                    near.tx.close(Close::going_away()).await;
                    "PeerClosed"
                }
                "read error" => {
                    far.fail_reads("boom");
                    "ReadFailed"
                }
                "vanished silently" => {
                    drop(near);
                    "ReadFailed"
                }
                "write error" => {
                    far.fail_writes();
                    peer.notify(TestOut::Notice("x".into())).unwrap();
                    "WriteFailed"
                }
                "write deadline" => {
                    far.stall_writes(true);
                    peer.notify(TestOut::Notice("x".into())).unwrap();
                    settle().await;
                    a.clock.advance(write_deadline);
                    settle().await;
                    a.clock.advance(write_deadline);
                    "WriteDeadline"
                }
                _ => {
                    for _ in 0..5 {
                        settle().await;
                        a.clock.advance(Duration::from_secs(10));
                    }
                    "Idle"
                }
            };
            let ends: Vec<String> = labels(&drain_until_quiet(&mut inbox).await)
                .into_iter()
                .filter(|label| label != "Dropped")
                .collect();
            assert_eq!(
                ends,
                vec![
                    "Connected:1".to_string(),
                    format!("Disconnected:1:{expected}"),
                    format!("Closed:1:{expected}"),
                ],
                "{case}"
            );
            assert!(ended(&mut inbox).await, "{case}");
            assert!(a.registry.is_empty(), "{case}");
        }
    }

    #[tokio::test]
    async fn a_handshake_that_ends_on_the_link_reports_which_end_failed() {
        for case in [
            "the peer closed",
            "vanished silently",
            "write error",
            "write deadline",
        ] {
            let mut a = Accepting::new(auth_policy());
            let (mut near, accepted) = a.connect().await;
            let far = a.far();
            let handshake: Arc<dyn Handshake> = match case {
                "write error" | "write deadline" => Arc::new(HelloHandshake),
                _ => Arc::new(TokenGate),
            };
            let expected = match case {
                "the peer closed" => "PeerClosed",
                "vanished silently" => "ReadFailed",
                "write error" => {
                    far.fail_writes();
                    "WriteFailed"
                }
                _ => {
                    far.stall_writes(true);
                    "WriteDeadline"
                }
            };
            let (_peer, mut inbox) = a.attach("conn-1", accepted, handshake);
            settle().await;
            match case {
                "the peer closed" => near.tx.close(Close::going_away()).await,
                "vanished silently" => drop(near),
                "write deadline" => a.clock.advance(AUTH),
                _ => {}
            }
            assert_eq!(
                labels(&drain_until_quiet(&mut inbox).await),
                vec![format!("Closed:0:{expected}")],
                "{case}"
            );
            assert!(ended(&mut inbox).await, "{case}");
            assert!(a.registry.is_empty(), "{case}");
        }
    }

    #[tokio::test]
    async fn auth_failures_and_silent_links_do_not_stop_a_later_real_connection() {
        let mut a = Accepting::new(auth_policy());
        let permits = a.listener.permits();
        for n in 1..=5 {
            let (mut near, accepted) = a.connect().await;
            let (_peer, mut inbox) = a.attach(&format!("conn-{n}"), accepted, Arc::new(TokenGate));
            near.say("auth|stale").await;
            let items = drain_until_quiet(&mut inbox).await;
            assert_eq!(
                labels(&items),
                vec!["Closed:0:Explicit"],
                "{n}: only the end, without dialing-side events (Rejected · ConnectFailed · GaveUp) — we did the rejecting"
            );
            assert!(ended(&mut inbox).await, "{n}");
            assert_eq!(
                near.arrived().await,
                vec![Seen::Closed(Some(Close::new(
                    RejectKind::Retryable.close_code(),
                    "bad token"
                )))],
                "{n}: the reject code goes to the peer"
            );
        }
        let (mut near, accepted) = a.connect().await;
        let (_peer, mut inbox) = a.attach(
            "conn-terminal",
            accepted,
            RejectingHandshake::new("version mismatch"),
        );
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec!["Closed:0:Explicit"]
        );
        assert_eq!(
            near.arrived().await,
            vec![Seen::Closed(Some(Close::new(
                RejectKind::Terminal.close_code(),
                "version mismatch"
            )))]
        );
        let (mut silent, accepted) = a.connect().await;
        let (_peer, mut inbox) = a.attach("conn-6", accepted, Arc::new(TokenGate));
        settle().await;
        a.clock.advance(AUTH);
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec!["Closed:0:Idle"]
        );
        assert_eq!(silent.arrived().await, vec![Seen::Failed]);

        let (mut near, accepted) = a.connect().await;
        let (peer, mut inbox) = a.attach("conn-7", accepted, Arc::new(TokenGate));
        near.say(GOOD).await;
        settle_until("operating", || peer.peer_state().is_live()).await;
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec!["Connected:1"]
        );
        assert_eq!(
            a.registry.len(),
            1,
            "a failed peer does not stay in the registry"
        );
        assert_eq!(a.generation_names(), 0);
        assert_eq!(permits.in_use(), 0);
        assert_eq!(a.redials.dials(), 0);
    }

    // ★The auth frame has already arrived★ — if the path reads once even on a passed deadline (0 of `with_deadline`), it passes.
    #[tokio::test]
    async fn a_link_that_sat_in_the_ready_queue_past_the_auth_deadline_closes_as_soon_as_it_is_attached(
    ) {
        for (sat, live) in [
            (AUTH - Duration::from_millis(1), true),
            (AUTH, false),
            (AUTH * 3, false),
        ] {
            let mut a = Accepting::new(auth_policy());
            let (mut near, accepted) = a.connect().await;
            near.say(GOOD).await;
            a.clock.advance(sat);
            let (_peer, mut inbox) = a.attach("conn-1", accepted, Arc::new(TokenGate));
            let items = drain_until_quiet(&mut inbox).await;
            if live {
                assert_eq!(labels(&items), vec!["Connected:1"], "{sat:?}");
            } else {
                assert_eq!(labels(&items), vec!["Closed:0:Idle"], "{sat:?}");
                assert!(ended(&mut inbox).await, "{sat:?}");
                assert!(a.registry.is_empty(), "{sat:?}");
                assert_eq!(near.arrived().await, vec![Seen::Failed], "{sat:?}");
            }
        }
    }

    #[tokio::test]
    async fn the_auth_deadline_counts_from_the_upgrade_not_from_attach() {
        let mut a = Accepting::new(auth_policy());
        let (_near, accepted) = a.connect().await;
        a.clock.advance(Duration::from_millis(600));
        let (_peer, mut inbox) = a.attach("conn-1", accepted, Arc::new(TokenGate));
        settle().await;
        a.clock.advance(Duration::from_millis(399));
        assert!(
            drain_until_quiet(&mut inbox).await.is_empty(),
            "999 ms after the upgrade — still before the deadline"
        );
        a.clock.advance(Duration::from_millis(1));
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec!["Closed:0:Idle"],
            "only 400 ms after attaching"
        );
    }

    #[tokio::test]
    async fn roster_generation_table_and_permits_stay_flat_across_many_accept_close_cycles() {
        let mut a = Accepting::new(auth_policy());
        let permits = a.listener.permits();
        for n in 0..30 {
            let (mut near, accepted) = a.connect().await;
            let (peer, mut inbox) = a.attach(&format!("conn-{n}"), accepted, Arc::new(TokenGate));
            match n % 3 {
                0 => {
                    near.say(GOOD).await;
                    settle_until("operating", || peer.peer_state().is_live()).await;
                    drop(near);
                }
                1 => near.say("auth|stale").await,
                _ => {
                    near.say(GOOD).await;
                    settle_until("operating", || peer.peer_state().is_live()).await;
                    peer.close();
                }
            }
            let items = drain_until_quiet(&mut inbox).await;
            assert_eq!(kinds(&items).last(), Some(&"Closed"), "{n}");
            assert!(ended(&mut inbox).await, "{n}");
            assert!(a.registry.is_empty(), "{n}");
            assert_eq!(
                a.generation_names(),
                0,
                "{n}: an accepted peer is not in the generation table"
            );
            assert_eq!(permits.in_use(), 0, "{n}");
        }
        assert_eq!(a.listener.queued(), 0);
        assert_eq!(a.redials.dials(), 0);
    }

    /// A minimal packet where both sides share one vocabulary (ask and answer). `TestWire` is one side's (client's) vocabulary and cannot connect two registries face to face.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Talk {
        Ask { tag: u64, body: String },
        Answer { tag: u64, body: String },
    }

    struct TalkWire;

    impl Wire for TalkWire {
        type Out = Talk;
        type In = Talk;
        type Tag = u64;
        type StreamKey = u32;
        type DecodeError = TestDecodeError;

        fn encode(&self, out: &Talk) -> Frame {
            Frame::Text(match out {
                Talk::Ask { tag, body } => format!("ask|{tag}|{body}"),
                Talk::Answer { tag, body } => format!("ans|{tag}|{body}"),
            })
        }

        fn decode(&self, frame: Frame) -> Result<Talk, TestDecodeError> {
            let Frame::Text(text) = frame else {
                return Err(TestDecodeError::Unknown("not text".into()));
            };
            let tag = |s: &str| {
                s.parse::<u64>()
                    .map_err(|e| TestDecodeError::BadNumber(e.to_string()))
            };
            match text.splitn(3, '|').collect::<Vec<_>>().as_slice() {
                ["ask", t, body] => Ok(Talk::Ask {
                    tag: tag(t)?,
                    body: (*body).to_string(),
                }),
                ["ans", t, body] => Ok(Talk::Answer {
                    tag: tag(t)?,
                    body: (*body).to_string(),
                }),
                _ => Err(TestDecodeError::Unknown(text)),
            }
        }

        fn request_tag(&self, out: &Talk) -> Option<u64> {
            match out {
                Talk::Ask { tag, .. } => Some(*tag),
                Talk::Answer { .. } => None,
            }
        }

        fn reply_tag(&self, inn: &Talk) -> Option<u64> {
            match inn {
                Talk::Answer { tag, .. } => Some(*tag),
                Talk::Ask { .. } => None,
            }
        }

        fn stream_mark(&self, _inn: &Talk) -> Option<StreamMark<u32>> {
            None
        }

        fn resume_request(&self, _key: &u32, _after: Option<u64>) -> Talk {
            unreachable!("does not use streams")
        }
    }

    async fn answer_asks(peer: &Peer<TalkWire>, inbox: &mut Inbox<TalkWire>) -> usize {
        settle().await;
        let mut answered = 0;
        while let Some(item) = inbox.try_recv() {
            if let Some(Arrival {
                generation,
                msg: Talk::Ask { tag, body },
                ..
            }) = item.as_message()
            {
                let answer = Talk::Answer {
                    tag: *tag,
                    body: format!("re:{body}"),
                };
                peer.notify_in(*generation, answer).unwrap();
                answered += 1;
            }
        }
        answered
    }

    // A transport-level round trip — measures whether the receiving side carries bidirectional requests, not a daemon-to-daemon vocabulary (TRD 3 §6-0 U6).
    #[tokio::test]
    async fn two_registries_ask_each_other_over_a_dialed_peer_and_an_attached_one() {
        let clock = ManualClock::new();
        let (mut listener, connector) = MemoryListener::new(clock.handle(), 4);
        let client = Registry::new(
            Arc::new(TalkWire),
            connector.dialer(),
            clock.handle(),
            Policy::default(),
        );
        let server = Registry::new(
            Arc::new(TalkWire),
            MemoryNetwork::new().dialer(),
            clock.handle(),
            Policy::default(),
        );
        let (to_server, mut client_inbox) = client.add(
            PeerId::new("server"),
            Address::new("mem://server"),
            immediate(),
        );
        let accepted = listener.accept().await.expect("receive");
        let (to_client, mut server_inbox) =
            server.attach(PeerId::new("conn-1"), accepted, immediate());
        settle_until("both sides operating", || {
            to_server.peer_state().is_live() && to_client.peer_state().is_live()
        })
        .await;

        let ask = |peer: &Peer<TalkWire>, tag: u64, body: &str| {
            let peer = peer.clone();
            let out = Talk::Ask {
                tag,
                body: body.into(),
            };
            tokio::spawn(async move { peer.request(out).await })
        };
        let client_asked = ask(&to_server, 1, "ping");
        let server_asked = ask(&to_client, 2, "who");
        assert_eq!(answer_asks(&to_client, &mut server_inbox).await, 1);
        assert_eq!(answer_asks(&to_server, &mut client_inbox).await, 1);
        for (asked, expected) in [
            (
                client_asked,
                Talk::Answer {
                    tag: 1,
                    body: "re:ping".into(),
                },
            ),
            (
                server_asked,
                Talk::Answer {
                    tag: 2,
                    body: "re:who".into(),
                },
            ),
        ] {
            let got = tokio::time::timeout(Duration::from_secs(5), asked)
                .await
                .expect("the answer did not come")
                .expect("the request task panicked")
                .expect("the request failed");
            assert_eq!(got, expected);
        }
    }

    #[tokio::test]
    async fn an_accepted_peer_flushes_what_it_holds_before_closing() {
        let mut a = Accepting::new(auth_policy());
        let (mut near, accepted) = a.connect().await;
        let far = a.far();
        let (peer, mut inbox) = a.attach("conn-1", accepted, immediate());
        settle_until("operating", || peer.peer_state().is_live()).await;
        far.stall_writes(true);
        peer.notify(TestOut::Notice("ack".into())).unwrap();
        peer.notify(TestOut::Notice("last".into())).unwrap();
        let closing = tokio::spawn(peer.close_after_flush(CloseCode::GOING_AWAY, "bye"));
        settle().await;
        assert!(!closing.is_finished(), "draining the blocked write");
        far.stall_writes(false);
        settle_until("drained and closed", || closing.is_finished()).await;
        assert_eq!(
            near.arrived().await,
            vec![
                Seen::Frame(Frame::Text("not|ack".into())),
                Seen::Frame(Frame::Text("not|last".into())),
                Seen::Closed(Some(Close::new(CloseCode::GOING_AWAY, "bye"))),
            ]
        );
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec![
                "Connected:1",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit"
            ]
        );
        assert!(ended(&mut inbox).await);
        assert!(a.registry.is_empty());
    }

    // ★The old supervisor ends after the new one is raised★ — its registry removal must not remove the new one.
    #[tokio::test]
    async fn re_attaching_a_name_closes_the_old_peer_and_its_end_leaves_the_new_one_listed() {
        let mut a = Accepting::new(auth_policy());
        let (mut first, accepted) = a.connect().await;
        let (old, mut old_inbox) = a.attach("conn", accepted, immediate());
        settle_until("old one operating", || old.peer_state().is_live()).await;
        let (second, accepted) = a.connect().await;
        let (new, mut new_inbox) = a.attach("conn", accepted, immediate());
        settle_until("new one operating", || new.peer_state().is_live()).await;
        assert_eq!(
            labels(&drain_until_quiet(&mut old_inbox).await),
            vec![
                "Connected:1",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit"
            ]
        );
        assert!(ended(&mut old_inbox).await);
        assert_eq!(first.arrived().await, vec![closing()]);
        assert_eq!(
            labels(&drain_until_quiet(&mut new_inbox).await),
            vec!["Connected:2"],
            "inherits the counter of the replaced handle — the generation does not rewind"
        );
        assert!(
            a.registry
                .get(&PeerId::new("conn"))
                .is_some_and(|listed| listed.same(&new)),
            "the old supervisor's end does not remove the new one"
        );
        assert_eq!(a.generation_names(), 0);
        drop(second);
        drain_until_quiet(&mut new_inbox).await;
        assert!(a.registry.is_empty());
    }

    // A table name tag (`add`) replaced by an accepted channel continues the table's counter (no table growth); an accepted peer replaced with `add` puts that counter on the table — either way the same number is never drawn twice.
    #[tokio::test]
    async fn a_name_passed_between_add_and_attach_never_mints_the_same_generation_twice() {
        let mut a = Accepting::new(auth_policy());
        let x = PeerId::new("x");
        let (dialed, _dialed_inbox) =
            a.registry
                .add(x.clone(), Address::new("mem://x"), immediate());
        settle_until("add operating", || dialed.peer_state().is_live()).await;
        assert_eq!(dialed.generation(), Generation(1));
        let (_near, accepted) = a.connect().await;
        let (attached, _attached_inbox) = a.attach("x", accepted, immediate());
        settle_until("attach operating", || attached.peer_state().is_live()).await;
        assert_eq!(attached.generation(), Generation(2));
        assert_eq!(a.generation_names(), 1, "only read");

        let (_near, accepted) = a.connect().await;
        let (attached, _attached_inbox) = a.attach("y", accepted, immediate());
        settle_until("attach operating", || attached.peer_state().is_live()).await;
        assert_eq!(attached.generation(), Generation(1));
        let (dialed, _dialed_inbox) =
            a.registry
                .add(PeerId::new("y"), Address::new("mem://y"), immediate());
        settle_until("add operating", || dialed.peer_state().is_live()).await;
        assert_eq!(dialed.generation(), Generation(2));
        assert_eq!(a.generation_names(), 2);
    }

    #[tokio::test]
    async fn after_close_all_an_attach_or_an_add_is_closed_at_once_and_never_listed() {
        let mut a = Accepting::new(auth_policy());
        let (_listed_near, accepted) = a.connect().await;
        let (listed, mut listed_inbox) = a.attach("conn-0", accepted, immediate());
        settle_until("operating", || listed.peer_state().is_live()).await;
        let folding = tokio::spawn(a.registry.close_all(Duration::from_secs(3)));

        let (mut near, accepted) = a.connect().await;
        let (_late, mut inbox) = a.attach("conn-1", accepted, immediate());
        assert!(a.registry.get(&PeerId::new("conn-1")).is_none());
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec!["Closed:0:Explicit"]
        );
        assert!(ended(&mut inbox).await);
        assert_eq!(near.arrived().await, vec![closing()]);

        let (_dialed, mut dialed_inbox) =
            a.registry
                .add(PeerId::new("d"), Address::new("mem://d"), immediate());
        assert_eq!(
            labels(&drain_until_quiet(&mut dialed_inbox).await),
            vec!["Closed:0:Explicit"]
        );
        assert!(ended(&mut dialed_inbox).await);
        assert_eq!(
            a.redials.dials(),
            0,
            "a peer added to a folded registry does not even dial"
        );

        assert_eq!(
            labels(&drain_until_quiet(&mut listed_inbox).await),
            vec![
                "Connected:1",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit"
            ]
        );
        settle_until("fold end", || folding.is_finished()).await;
        assert_eq!(folding.await.expect("close_all task"), Ok(()));
        assert!(a.registry.is_empty());
    }

    #[tokio::test]
    async fn the_upgrade_permit_is_held_through_the_handshake_and_given_back_at_live_or_at_failure()
    {
        let mut a = Accepting::new(auth_policy());
        let permits = a.listener.permits();
        let (mut near, accepted) = a.connect().await;
        assert_eq!(permits.in_use(), 1, "ready queue");
        let (peer, _inbox) = a.attach("conn-1", accepted, Arc::new(TokenGate));
        settle().await;
        assert_eq!(
            permits.in_use(),
            1,
            "held during the handshake until the auth-wait permit exists"
        );
        near.say(GOOD).await;
        settle_until("operating", || peer.peer_state().is_live()).await;
        assert_eq!(
            permits.in_use(),
            0,
            "released on entering the operating phase"
        );

        let (mut stale, accepted) = a.connect().await;
        assert_eq!(permits.in_use(), 1);
        let (_failed, _failed_inbox) = a.attach("conn-2", accepted, Arc::new(TokenGate));
        stale.say("auth|stale").await;
        settle().await;
        assert_eq!(
            permits.in_use(),
            0,
            "a peer that failed auth releases the permit even without draining the inbox"
        );
    }

    #[tokio::test]
    async fn an_accepted_peer_ignores_reconnect_now_and_folds_once_its_inbox_is_dropped() {
        let mut a = Accepting::new(auth_policy());
        let (mut near, accepted) = a.connect().await;
        let (peer, mut inbox) = a.attach("conn-1", accepted, immediate());
        settle_until("operating", || peer.peer_state().is_live()).await;
        peer.reconnect_now();
        settle().await;
        assert!(
            peer.peer_state().is_live(),
            "no address to dial — the operating link is not abandoned"
        );
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec!["Connected:1"]
        );
        assert!(near.arrived().await.is_empty());

        drop(inbox);
        settle().await;
        assert_eq!(near.arrived().await, vec![closing()]);
        assert_eq!(peer.peer_state(), PeerState::Closed);
        assert!(a.registry.is_empty());
    }

    #[tokio::test]
    async fn an_accepted_peer_leaves_the_roster_before_its_teardown_waits_on_the_inbox() {
        let mut a = Accepting::new(Policy {
            inbound_queue: 1,
            ..auth_policy()
        });
        let (_near, accepted) = a.connect().await;
        let (peer, mut inbox) = a.attach("conn-1", accepted, immediate());
        settle_until("operating", || peer.peer_state().is_live()).await;
        peer.close();
        settle().await;
        assert!(
            a.registry.is_empty(),
            "even if the cleanup is stuck because the inbox is full, it is not in the registry"
        );
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec![
                "Connected:1",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit"
            ]
        );
        assert!(ended(&mut inbox).await);
    }

    #[tokio::test]
    async fn an_accepted_peer_whose_supervisor_panics_still_leaves_the_roster() {
        let mut a = Accepting::new(auth_policy());
        let (_near, accepted) = a.connect().await;
        let (_peer, mut inbox) = a.attach("conn-1", accepted, Arc::new(PanickingHandshake));
        settle_until("the supervisor ended", || a.registry.is_empty()).await;
        assert!(drain_until_quiet(&mut inbox).await.is_empty());
        assert!(ended(&mut inbox).await, "`None` without `Closed`");
    }

    // ── 3-0 U6b: raising on a runtime with several worker threads ──
    //
    // ★`settle` cannot be used★ (current_thread only — `testing.rs`) — the end is seen by a real-time-deadline wait. `RACE_THREADS` OS threads start together at a
    //   barrier and hammer the same registry while the supervisors run on worker threads.

    const RACE_THREADS: usize = 4;
    const REAL_WAIT: Duration = Duration::from_secs(10);

    async fn drain_to_end(inbox: &mut Inbox<TestWire>) -> Vec<Incoming<TestWire>> {
        tokio::time::timeout(REAL_WAIT, async {
            let mut out = Vec::new();
            while let Some(item) = inbox.recv().await {
                out.push(item);
            }
            out
        })
        .await
        .expect("the inbox did not end within the real-time deadline")
    }

    async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        tokio::time::timeout(REAL_WAIT, async {
            while !done() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("did not happen within the real-time deadline: {what}"));
    }

    fn accepted_link(pool: &PermitPool, upgraded_at: Instant) -> (Near, Accepted) {
        let (near, far) = memory_link();
        let permit = pool.try_take().expect("upgrade permit");
        let accepted = Accepted::new(far.tx, far.rx, LinkInfo::default(), upgraded_at, permit);
        (
            Near {
                tx: near.tx,
                rx: near.rx,
            },
            accepted,
        )
    }

    /// [`RACE_THREADS`] OS threads cross a barrier together inside the runtime context and run `call(thread number)`. ★This wait blocks the test-body thread★ —
    /// a multi_thread test body is the `block_on` thread, not a worker, so the supervisors keep running.
    fn race<T: Send>(call: impl Fn(usize) -> T + Sync) -> Vec<T> {
        let runtime = tokio::runtime::Handle::current();
        let start = std::sync::Barrier::new(RACE_THREADS);
        std::thread::scope(|scope| {
            let threads: Vec<_> = (0..RACE_THREADS)
                .map(|t| {
                    let (runtime, start, call) = (&runtime, &start, &call);
                    scope.spawn(move || {
                        let _entered = runtime.enter();
                        start.wait();
                        call(t)
                    })
                })
                .collect();
            threads
                .into_iter()
                .map(|thread| thread.join().expect("a racing thread panicked"))
                .collect()
        })
    }

    /// Reads the dialing-side end to the end (close · error) — up to the real-time [`REAL_WAIT`]. ★Do not use [`Near::arrived`]★ — it polls once without waiting, so on a
    /// task with an exhausted tokio cooperative budget it cannot read even what has arrived.
    async fn read_to_end(near: &mut Near) -> Vec<Seen> {
        tokio::time::timeout(REAL_WAIT, async {
            let mut out = Vec::new();
            loop {
                let seen = match near.rx.recv().await {
                    Ok(LinkRead::Frame(frame)) => Seen::Frame(frame),
                    Ok(LinkRead::Closed(close)) => Seen::Closed(close),
                    Err(_) => Seen::Failed,
                };
                let end = !matches!(seen, Seen::Frame(_));
                out.push(seen);
                if end {
                    return out;
                }
            }
        })
        .await
        .expect("the dialing side's end did not finish within the real-time deadline")
    }

    fn connected_generations(items: &[Incoming<TestWire>]) -> Vec<u64> {
        items
            .iter()
            .filter_map(|item| match item {
                Incoming::Event(TransportEvent::Connected { generation, .. }) => Some(generation.0),
                _ => None,
            })
            .collect()
    }

    // An accepted peer ending at its first poll — deadline already passed (`Idle`) or the dialing side already cut. If raising came after starting, the handle of a
    //   supervisor that finished even removing itself on another worker would be raised afterwards and remain forever (`Registry::launch`).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn accepted_peers_that_end_on_their_first_poll_never_stay_listed() {
        const PER_THREAD: usize = 50;
        let clock = ManualClock::new();
        let registry = Registry::new(
            Arc::new(TestWire),
            MemoryNetwork::new().dialer(),
            clock.handle(),
            auth_policy(),
        );
        let upgraded_at = clock.handle().now();
        clock.advance(AUTH * 2);
        let pool = PermitPool::new(RACE_THREADS * PER_THREAD);
        let attached = race(|t| {
            (0..PER_THREAD)
                .map(|n| {
                    let past_deadline = n % 2 == 0;
                    let at = if past_deadline {
                        upgraded_at
                    } else {
                        clock.handle().now()
                    };
                    let (near, accepted) = accepted_link(&pool, at);
                    if !past_deadline {
                        drop(near);
                    }
                    let (_peer, inbox) = registry.attach(
                        PeerId::new(format!("conn-{t}-{n}")),
                        accepted,
                        Arc::new(TokenGate),
                    );
                    inbox
                })
                .collect::<Vec<_>>()
        });
        for mut inbox in attached.into_iter().flatten() {
            assert_eq!(kinds(&drain_to_end(&mut inbox).await), vec!["Closed"]);
        }
        assert_eq!(
            registry.len(),
            0,
            "an ended accepted peer does not stay in the registry"
        );
        assert_eq!(pool.in_use(), 0);
    }

    // A peer raised on a folded registry has the close in before starting, so on whichever worker it first runs it neither dials nor handshakes.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn after_close_all_no_attach_shakes_hands_and_no_add_dials_on_a_multi_thread_runtime() {
        const PER_THREAD: usize = 25;
        let clock = ManualClock::new();
        let net = MemoryNetwork::new();
        let registry = Registry::new(
            Arc::new(TestWire),
            net.dialer(),
            clock.handle(),
            auth_policy(),
        );
        assert_eq!(registry.close_all(Duration::from_secs(1)).await, Ok(()));
        let pool = PermitPool::new(RACE_THREADS * PER_THREAD);
        let late = race(|t| {
            (0..PER_THREAD)
                .map(|n| {
                    let (near, accepted) = accepted_link(&pool, clock.handle().now());
                    let (_attached, attached_inbox) = registry.attach(
                        PeerId::new(format!("conn-{t}-{n}")),
                        accepted,
                        Arc::new(HelloHandshake),
                    );
                    let (_dialed, dialed_inbox) = registry.add(
                        PeerId::new(format!("d-{t}-{n}")),
                        Address::new("mem://d"),
                        immediate(),
                    );
                    (near, attached_inbox, dialed_inbox)
                })
                .collect::<Vec<_>>()
        });
        for (mut near, mut attached_inbox, mut dialed_inbox) in late.into_iter().flatten() {
            for inbox in [&mut attached_inbox, &mut dialed_inbox] {
                assert_eq!(
                    labels(&drain_to_end(inbox).await),
                    vec!["Closed:0:Explicit"]
                );
            }
            assert_eq!(
                read_to_end(&mut near).await,
                vec![closing()],
                "only a close comes, without the handshake frame (hello)"
            );
        }
        assert_eq!(
            net.dials(),
            0,
            "a peer added to a folded registry does not dial"
        );
        assert!(registry.is_empty());
        assert_eq!(pool.in_use(), 0);
    }

    // Several calls replace the same name tag at once with `add` · `attach`. ★The race is raising an accepted peer with no table field★ — if taking out that handle and
    //   choosing the counter were split, another call would see neither, make a new counter and draw an already issued generation (`Registry::launch`); so each name tag starts in that state.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn racing_replacements_of_one_name_never_mint_the_same_generation_twice() {
        const NAMES: usize = 60;
        const PER_THREAD: usize = 4;
        let clock = ManualClock::new();
        let net = MemoryNetwork::new();
        let registry = Registry::new(
            Arc::new(TestWire),
            net.dialer(),
            clock.handle(),
            Policy::default(),
        );
        let pool = PermitPool::new(NAMES * (1 + RACE_THREADS * PER_THREAD));
        for round in 0..NAMES {
            let name = PeerId::new(format!("x{round}"));
            let (first_near, accepted) = accepted_link(&pool, clock.handle().now());
            let (first, first_inbox) = registry.attach(name.clone(), accepted, immediate());
            wait_until("accepted peer operating", || first.peer_state().is_live()).await;
            let raced = race(|t| {
                (0..PER_THREAD)
                    .map(|n| {
                        if (t + n) % 2 == 0 {
                            let (_peer, inbox) =
                                registry.add(name.clone(), Address::new("mem://x"), immediate());
                            (None, inbox)
                        } else {
                            let (near, accepted) = accepted_link(&pool, clock.handle().now());
                            let (_peer, inbox) =
                                registry.attach(name.clone(), accepted, immediate());
                            (Some(near), inbox)
                        }
                    })
                    .collect::<Vec<_>>()
            });
            let (last, last_inbox) =
                registry.add(name.clone(), Address::new("mem://x"), immediate());
            wait_until("last peer operating", || last.peer_state().is_live()).await;
            registry.remove(&name);

            let mut nears = vec![first_near];
            let mut inboxes = vec![first_inbox, last_inbox];
            for (near, inbox) in raced.into_iter().flatten() {
                nears.extend(near);
                inboxes.push(inbox);
            }
            let mut minted = Vec::new();
            for inbox in &mut inboxes {
                minted.extend(connected_generations(&drain_to_end(inbox).await));
            }
            let mut unique = minted.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(
                unique.len(),
                minted.len(),
                "{name:?}: drew the same generation twice — {minted:?}"
            );
            drop(nears);
        }
    }

    struct Tripped;

    /// A clock that panics once at the n-th `now()` of a designated thread — otherwise [`ManualClock`]. ★Filtered by thread★ so other supervisors' reads on worker threads do not count.
    #[derive(Default)]
    struct TrippingClock {
        clock: ManualClock,
        trip: Mutex<Option<(std::thread::ThreadId, usize)>>,
    }

    impl TrippingClock {
        fn arm(&self, nth: usize) {
            *self.trip.lock().unwrap() = Some((std::thread::current().id(), nth));
        }
    }

    impl Clock for TrippingClock {
        fn now(&self) -> Instant {
            let fire = {
                let mut trip = self.trip.lock().unwrap();
                match trip.as_mut() {
                    Some((thread, left)) if *thread == std::thread::current().id() => {
                        *left -= 1;
                        let fire = *left == 0;
                        if fire {
                            *trip = None;
                        }
                        fire
                    }
                    _ => false,
                }
            };
            // Fire after releasing the lock — blowing up while holding it would poison it and blow up other supervisors' reads too.
            if fire {
                std::panic::panic_any(Tripped);
            }
            self.clock.now()
        }

        fn sleep(&self, d: Duration) -> futures_util::future::BoxFuture<'static, ()> {
            self.clock.sleep(d)
        }
    }

    /// Runs `call` on an OS thread in the runtime context catching panics — not finishing within the real-time [`REAL_WAIT`] is treated as a deadlock and panics.
    /// ★Detach that thread★ — waiting on a deadlocked thread would hang the test instead of failing it.
    fn off_thread<T: Send + 'static>(
        call: impl FnOnce() -> T + Send + 'static,
    ) -> std::thread::Result<T> {
        let runtime = tokio::runtime::Handle::current();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _entered = runtime.enter();
            let _ = done_tx.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)));
        });
        done_rx
            .recv_timeout(REAL_WAIT)
            .expect("did not finish within the real-time deadline — registry lock deadlock")
    }

    // The replacing `attach` · `add` panics while building the supervisor inside the registry lock (the injected clock). ★If an accepted peer panics after being
    //   built, the registry removal dropped while unwinding retakes the lock on the same thread★ (`new_peer` in `peer.rs`) — called on a thread with a real-time
    //   deadline so that deadlock does not hang the test. If the table changed before building, the handle to be replaced would vanish (`Registry::launch`).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_replacement_that_panics_while_preparing_leaves_the_roster_untouched_and_usable() {
        let clock = Arc::new(TrippingClock::default());
        let registry = Registry::new(
            Arc::new(TestWire),
            MemoryNetwork::new().dialer(),
            clock.clone(),
            Policy::default(),
        );
        let pool = PermitPool::new(2);
        let name = PeerId::new("a");
        let (first, mut first_inbox) =
            registry.add(name.clone(), Address::new("mem://a"), immediate());
        let still_first = |registry: &Registry<TestWire>| {
            registry.len() == 1 && registry.get(&name).is_some_and(|peer| peer.same(&first))
        };

        let nth = if cfg!(debug_assertions) { 2 } else { 1 };
        let (_near, accepted) = accepted_link(&pool, clock.now());
        let attached = off_thread({
            let (registry, clock, name) = (registry.clone(), clock.clone(), name.clone());
            move || {
                clock.arm(nth);
                registry.attach(name, accepted, immediate())
            }
        });
        assert!(attached.is_err_and(|panic| panic.is::<Tripped>()));
        assert!(
            still_first(&registry),
            "even if building an accepted peer panics, the table stays as is"
        );
        assert_eq!(
            pool.in_use(),
            0,
            "the upgrade permit returns along with the panic"
        );

        let added = off_thread({
            let (registry, clock, name) = (registry.clone(), clock.clone(), name.clone());
            move || {
                clock.arm(1);
                registry.add(name, Address::new("mem://a"), immediate())
            }
        });
        assert!(added.is_err_and(|panic| panic.is::<Tripped>()));
        assert!(
            still_first(&registry),
            "even if building a dialed peer panics, the table stays as is"
        );
        wait_until("the peer about to be replaced is operating", || {
            first.peer_state().is_live()
        })
        .await;

        let (_near, accepted) = accepted_link(&pool, clock.now());
        let (replaced, _replaced_inbox) = off_thread({
            let (registry, name) = (registry.clone(), name.clone());
            move || registry.attach(name, accepted, immediate())
        })
        .expect("uses the registry even after the panic");
        assert!(registry.get(&name).is_some_and(|peer| peer.same(&replaced)));
        assert_eq!(
            kinds(&drain_to_end(&mut first_inbox).await).last(),
            Some(&"Closed"),
            "the replaced handle is closed"
        );
    }
}
