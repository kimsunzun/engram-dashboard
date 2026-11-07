//! Peer-N registry — ★driving layer★. It sets up one supervisor per peer and returns that handle's receiver end ([`Inbox`]) to
//! the side that raised it.
//!
//! ★There are N peers from the start★ (ADR-0177 decision 3 · ADR-0179) — there is no state anywhere that presumes a world of just one.
//! **Peers do not share state with each other**: even if daemon A gives up, B is operating.
//!
//! ★The upward line is not shared either — it is separate per handle★ (ADR-0310 decision 17 · TRD 3 §3-0 ① D24 ②) — with one line,
//! the moment a noisy peer fills it, a quiet peer's events · packets are held behind it, and 「A 하나 죽어도
//! B·C 는 잘 동작해야 한다」 (even if A dies, B and C must keep working) collapses (ADR-0180 decision 8). So each time [`Registry::add`] raises one it sets up a line (length =
//! [`Policy::inbound_queue`]), returns its receiving end, and only that one supervisor holds the enqueue end. When the line is full, that
//! supervisor holds in its own ordering area and stops that peer's reading — it neither drops nor cuts the connection (`peer.rs` header).
//! So if the receiving side does not drain one receiver end, only that peer stalls.
//!
//! ★Even raising the same name tag again does not continue the line★ — the old supervisor puts into the old receiver end until it finishes teardown, and the new supervisor into the new
//! receiver end. So only one supervisor's output comes to one receiver end and that receiver end's end is that supervisor's end. There is no ordering between
//! receiver ends — if a merged view is needed, the consumer adapter merges the receiver ends (the filtering rules are TRD 3 §3-3).
//!
//! ★There are two ways to raise a peer★ — [`Registry::add`] dials by address and redials when cut. [`Registry::attach`] puts on one received
//! channel ([`Accepted`]) and does not redial, and when it ends it removes itself from the registry (TRD 3 §3-1 ①). The consumer runs the accept loop
//! — the registry does not hold the listener.
//!
//! ★The registry does not decide 「앱 전체가 끊겼나」 (is the whole app disconnected)★ — that is a screen concept and this crate only counts peers and
//! does not know the app. It gives two ingredients ([`Registry::peers`] and [`crate::Peer::reconnect_now`]) and the shell folds them
//! (that is how ADR-0180's open question gets closed).

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

/// One handle's receiver end — only what was emitted by the one supervisor raised by the [`Registry::add`] · [`Registry::attach`] that returned it comes.
///
/// ★The end = `None` after [`TransportEvent::Closed`](crate::TransportEvent::Closed)★ — the supervisor loads `Closed` last
/// as it finishes teardown and releases the enqueue end. ★If `None` comes without `Closed` — unless the receiving side called [`Inbox::close`]
/// itself — the supervisor ended without a normal teardown★: either teardown exceeded the limit (`CLOSE_DRAIN_LIMIT` in `peer.rs`) and dropped the rest
/// up to `Closed` (neither counted nor reported), or the task ended abnormally (panic · runtime shutdown — the enqueue
/// end is dropped with the task). Do not read it only as exceeding the limit. A `None` after calling [`Inbox::close`] is not the
/// supervisor's end — the supervisor may still be running. ★Drain to the end★ — crate header 「받는 쪽 불변식」 (receiver invariants) 5.
///
/// Each item carries the supervisor's byte share and it is released when taken out (`budget.rs`). If the receiver end is dropped, the share of what it held
/// is released with it.
pub struct Inbox<W: Wire> {
    rx: mpsc::Receiver<Charged<Incoming<W>>>,
}

impl<W: Wire> Inbox<W> {
    /// `None` = the end — the supervisor released the enqueue end or [`Inbox::close`] was called, and everything it held has been taken out.
    /// ★Cancel-safe★ — put on a `select!` arm, no item is lost even if another arm wins.
    pub async fn recv(&mut self) -> Option<Incoming<W>> {
        self.rx.recv().await.map(Charged::into_item)
    }

    /// One without waiting. `None` = nothing held now or it ended — to tell the two apart use [`Inbox::recv`].
    pub fn try_recv(&mut self) -> Option<Incoming<W>> {
        self.rx.try_recv().ok().map(Charged::into_item)
    }

    /// It blocks only new items. What is already held (including an already loaded `Closed`) can still be taken out with [`Inbox::recv`] · [`Inbox::try_recv`],
    /// and when all are taken out it is `None` even if the supervisor is still running. ★What the supervisor holds in the ordering area and has not yet loaded
    /// is silently dropped★ — it cannot be loaded into a closed line, and it is neither counted nor reported. The same if the [`Inbox`] is dropped (what it held
    /// is dropped along with it).
    ///
    /// ★The supervisor folds immediately★ — even with nothing to put in (even if the peer is quiet) it goes to closing on the spot and does not re-attach
    /// (crate header 「받는 쪽 불변식」 (receiver invariants) 5). The same if dropped. ★The outbound work it had accepted is dropped too★ — commands in the queue are
    /// rejected and the frame being written is dropped (counted but nowhere to report). To send and leave, drain until `Closed` · `None`
    /// and then release.
    pub fn close(&mut self) {
        self.rx.close();
    }
}

struct RegistryInner<W: Wire> {
    wire: Arc<W>,
    dialer: Arc<dyn Dialer>,
    clock: Arc<dyn Clock>,
    policy: Policy,
    /// ★Do not wait inside the lock★ — starting supervisors · closing · dropping handles are outside the lock.
    roster: Mutex<Roster<W>>,
}

/// Everything one registry lock guards. ★The generation table is inside this lock too★ — because taking out the handle to replace · choosing the counter · raising the new
/// handle must be one critical section (`Registry::launch`).
struct Roster<W: Wire> {
    /// The handle of a name tag that is raised.
    peers: HashMap<PeerId, Peer<W>>,
    /// The per-name-tag generation counter. ★Do not erase with `remove`★ — if erased, the generation is rewound to
    /// 0 when the same name tag is raised again, and the consumer's ordinary defense (「최대 세대 미만은 무시」 — ignore anything below the max generation) discards the new connection entirely.
    ///
    /// This map grows by as many as **the number of name tags the consumer wrote with [`Registry::add`]** (it is not a value the peer decides).
    /// ★Accepted peers are not put in★ — the name tag is new for each connection so putting them in would grow it forever by the number of connections
    /// ([`Registry::attach_with_policy`]).
    generations: HashMap<PeerId, Arc<AtomicU64>>,
    /// [`Registry::close_all`] was called — a peer raised after that does not get onto the registry and is closed at once. The side that sets it (`close_all`'s
    /// draining) and the side that reads it (`launch`) are inside the same lock, so a raise that crosses the fold does not remain in the emptied registry.
    folded: bool,
}

impl<W: Wire> RegistryInner<W> {
    /// Reuse a poisoned lock as is — if the poison were left, one panic would make the registry unusable forever.
    ///
    /// Even so, no half-changed table remains: the only thing that can panic inside the lock is building the supervisor (`prepare_peer` of `Registry::launch`
    /// — things like the injected clock), and it ends before the table is changed, and the table change is one field at a time with no place to panic. ★Even if it panics while building,
    /// what is dropped as it unwinds does not retake this lock★ (`new_peer` in `peer.rs`) — if it did,
    /// it would be a deadlock on the same thread.
    fn roster(&self) -> MutexGuard<'_, Roster<W>> {
        self.roster.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl<W: Wire> Roster<W> {
    /// The generation counter this name tag will use — the one in the table if there, else that of the raised (to be replaced) handle, and if neither, a new one.
    /// It does not change the table — putting it on the table is done after `Registry::launch` finishes building.
    fn counter(&self, id: &PeerId) -> Arc<AtomicU64> {
        if let Some(counter) = self.generations.get(id) {
            return counter.clone();
        }
        // ★It continues the counter itself without reading the old supervisor's last generation★ (that race = `PeerConfig::generations`). When replacing an accepted
        //   peer not in the table with `add`, a new counter would repeat numbers the accepted peer had already drawn.
        self.peers
            .get(id)
            .map(Peer::generations)
            .unwrap_or_default()
    }
}

/// Peer registry. A clone points to the same registry.
///
/// ★When the last clone disappears the registry merely releases the handles it holds★ — the supervisor hears it as a close when all of that peer's handles ([`Peer`]) are
/// gone and tears down, and that receiver end ends with `Closed` → `None`. If the consumer holds a handle,
/// that supervisor keeps running.
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
    /// Sets up one registry. ★What is received comes out only through the [`Inbox`] that [`Registry::add`] · [`Registry::attach`] return per handle★.
    /// `dialer` is used only by `add`.
    pub fn new(
        wire: Arc<W>,
        dialer: Arc<dyn Dialer>,
        clock: Arc<dyn Clock>,
        policy: Policy,
    ) -> Self {
        debug_assert_eq!(policy.check(), Ok(()), "정책 값이 자기 불변식을 어긴다");
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

    /// Raises a peer and starts attaching at once — it returns that handle and receiver end. If the same name tag already exists, it **closes the old one
    /// and replaces it**.
    ///
    /// ★It must be called inside a tokio runtime★ — it starts a supervisor task, so calling from outside panics. That panic is before the registry is touched
    /// — the registry can still be used afterwards, and a peer that was raised under the same name tag stays raised.
    ///
    /// If called after [`Registry::close_all`], it does not raise onto the registry and closes before dialing — the receiver end ends with `Closed` → `None`
    /// and the [`Dialer`] is not called.
    pub fn add(
        &self,
        id: PeerId,
        addr: Address,
        handshake: Arc<dyn Handshake>,
    ) -> (Peer<W>, Inbox<W>) {
        let policy = self.inner.policy.clone();
        self.add_with_policy(id, addr, handshake, policy)
    }

    /// Applies a different policy to this peer only. Runtime conditions · a folded registry are the same as [`Registry::add`].
    ///
    /// ★Close the old supervisor first, and pass its last generation as the new supervisor's floor value★ — if the order is reversed, two sockets
    /// attach to the same peer at once, and if the floor value is not passed, the generation is rewound to 0 and the consumer's ordinary defense
    /// (「최대 세대 미만은 무시」 — ignore anything below the max generation) **discards the new connection entirely**.
    ///
    /// ★Even so, the overlap does not become 0★ — [`Peer::close`] has no completion signal so the old supervisor may be still
    /// closing the channel for a while, and in that time it may enter the operating phase and issue one more generation.
    /// The **shared counter** is the device that makes that overlap harmless — the connection generation drawn on attach (`Connected` ·
    /// `Disconnected` · the number carried on messages) never comes out twice. ★The generation of a supervisor that never attached is an exception★
    /// — it carries the floor value (the old supervisor's last generation) as is, so that supervisor's
    /// [`TransportEvent::Closed`](crate::TransportEvent::Closed) and [`Peer::generation`] show the same
    /// number as the old supervisor's. Which supervisor it belongs to is told by the receiver end.
    ///
    /// ★Generations start again from 1 only for a name tag that was never raised with `add` and is not on the registry now★ — calling `add` on a name tag that was put on as an accepted peer and then removed itself
    /// or was removed with [`Registry::remove`], falls here too. Otherwise it continues this name tag's counter (the one in the generation
    /// table · if none, the replaced handle's) and puts it on the table. The same holds among calls that raise the same name tag at the same time
    /// — no call makes a new counter without seeing a handle another call took out.
    ///
    /// ★Even under the same name tag the receiver end is new★ — the old supervisor puts into the old receiver end until it finishes teardown, and its end is `None`
    /// after `Closed`. If the old receiver end is released, the old supervisor's teardown ends without waiting, and if held without draining,
    /// it remains up to `CLOSE_DRAIN_LIMIT` in `peer.rs`.
    pub fn add_with_policy(
        &self,
        id: PeerId,
        addr: Address,
        handshake: Arc<dyn Handshake>,
        policy: Policy,
    ) -> (Peer<W>, Inbox<W>) {
        debug_assert_eq!(policy.check(), Ok(()), "정책 값이 자기 불변식을 어긴다");
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

    /// Puts a received channel on as a peer — it returns that handle and receiver end. The policy is the registry's
    /// ([`Registry::attach_with_policy`] overrides it). The contract is there.
    pub fn attach(
        &self,
        id: PeerId,
        accepted: Accepted,
        handshake: Arc<dyn Handshake>,
    ) -> (Peer<W>, Inbox<W>) {
        let policy = self.inner.policy.clone();
        self.attach_with_policy(id, accepted, handshake, policy)
    }

    /// Puts a received channel ([`Accepted`]) on as a peer with this policy (TRD 3 §3-1 ①). Runtime conditions · replacing the same name tag (closes the old one ·
    /// the old receiver end ends with `Closed` → `None` · the receiver end is new) are the same as [`Registry::add_with_policy`].
    ///
    /// ★It starts from the handshake and does not redial★ — if it ends without entering the operating phase (reject · deadline · channel loss) or is cut during operation, it
    /// folds at once: the receiver end ends with `None` after [`TransportEvent::Closed`](crate::TransportEvent::Closed) and
    /// its `cause` carries the real reason (for a disconnect during operation, the same cause as the preceding `Disconnected` — including the read-end/write-end distinction).
    /// Dialing-side events (`ConnectFailed` · `Rejected` · `GaveUp`) do not come and [`Peer::state`] does not pass through `Dialing` ·
    /// `Backoff` · `GaveUp`. [`Peer::reconnect_now`] is not listened to. The other handle contracts (sending ·
    /// closing · drain and close) are the same as a dialed peer.
    ///
    /// ★Auth deadline = [`Accepted::upgraded_at`] + [`Policy::connect_timeout`]★ (TRD 3 §5 3) — the time sitting in the listener's
    /// ready queue counts too. If it has already passed at the moment of putting on, it does not run the handshake and closes (`cause` = `Idle`). So the listener must stamp `upgraded_at` with the same
    /// [`Clock`] as this registry (a debug build catches a future time with an assertion).
    ///
    /// ★When it ends it removes itself from the registry★ — after dropping the channel and before teardown (which waits for the receiving side to drain the receiver end). If in the meantime
    /// it was replaced under the same name tag, the new one is left alone. It removes itself even if the supervisor ends abnormally. A peer that ends at its first poll
    /// (the deadline had already passed · the peer had already cut) also does not remain on the registry — the supervisor is started after raising.
    ///
    /// ★It does not go into the generation table★ — the receiving side's name tag is new for each connection (TRD 3 §3-1 ① `conn-<ConnId>`) so putting it in would grow the table by the
    /// number of connections. The generation counter uses the one in the table if that name tag is there (it was raised with `add`), the replaced handle's if not,
    /// and a new one if neither — if you put on a name tag already on the registry again, the generation is not rewound even when put on at the same time. ★Generations start
    /// again from 1 only for a name tag that was never raised with `add` and is not on the registry now★ — a name tag put on for the first time, and a name tag put on again after it
    /// removed itself or was removed with [`Registry::remove`] (on the premise that receiving-side name tags are not reused).
    ///
    /// ★The upgrade field ([`crate::ConnPermit`] of [`Accepted`]) is released on entering the operating phase★ — if it ends without entering, it is released at the place
    /// where the channel is dropped (before the receiving side drains the receiver end — TRD 3 §5 14 「인증에 실패한 상대는 닫히는 즉시 칸을 놓는다」 (a peer that failed auth releases its field as soon as it closes)). The auth-waiting field does not exist yet (3-3 U4)
    /// so this field covers that role too — when that field arrives, this holds it here
    /// and releases the upgrade field.
    ///
    /// If called after [`Registry::close_all`], it does not raise onto the registry and closes before the handshake — it loads a close ([`CloseCode::GOING_AWAY`])
    /// and drops it without reading or writing anything on the channel.
    pub fn attach_with_policy(
        &self,
        id: PeerId,
        accepted: Accepted,
        handshake: Arc<dyn Handshake>,
        policy: Policy,
    ) -> (Peer<W>, Inbox<W>) {
        debug_assert_eq!(policy.check(), Ok(()), "정책 값이 자기 불변식을 어긴다");
        let Accepted {
            tx,
            rx,
            info: _,
            upgraded_at,
            permit,
        } = accepted;
        debug_assert!(
            upgraded_at <= self.inner.clock.now(),
            "listener/registry clock mismatch — 청취기가 명부와 다른 시계로 upgraded_at 을 찍었다"
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

    /// The common tail of `add` · `attach` — builds the supervisor and handle, and starts the supervisor **after** raising it on the registry. `record` = if the name tag was not in the table,
    /// puts the chosen generation counter on the table (`add`).
    ///
    /// ★Raising comes before starting★ — if reversed, an accepted peer that ends at its first poll (the deadline had already passed · the peer had already
    /// cut) finishes even removing itself from the registry on another worker thread and is then raised, so the ended handle remains forever (the receiving side's name tag is
    /// new for each connection so no one removes it again — the registry grows by the number of quickly cut connections). A folded registry is also
    /// told apart before starting and the close is put in first — the supervisor sees that at its first poll and neither dials nor handshakes (the first line of `dial` ·
    /// `shake_hands` in `peer.rs`). If reversed, a supervisor that ran first on another worker would dial once or start a handshake.
    ///
    /// ★Choosing the counter · taking out the handle to replace · raising is one critical section★ — if split, one of two
    /// calls raising the same name tag at the same time would make a new counter without seeing either the handle the other took out or the table field, and draw a generation that already went out again.
    ///
    /// ★The table is changed only after building (`prepare_peer`) finishes★ — if it panics while building (like the injected clock), the registry is left untouched
    /// as it was, so the handle that was to be replaced stays raised and keeps running. The table change after that has no place to panic (`roster`).
    fn launch(
        &self,
        id: PeerId,
        record: bool,
        config: impl FnOnce(Arc<AtomicU64>) -> PeerConfig<W>,
    ) -> Peer<W> {
        // Outside a runtime it panics here — before the registry is touched (`Registry::add`).
        let runtime = tokio::runtime::Handle::current();
        let (peer, supervisor, displaced, folded) = {
            let mut roster = self.inner.roster();
            let generations = roster.counter(&id);
            let (peer, supervisor) = prepare_peer(config(generations.clone()));
            // Only `add` puts the chosen counter on the table — an accepted peer only reads the table (`Roster::generations`).
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

    /// The hand by which an accepted peer removes itself from the registry when it ends — it removes only when that name tag still points to that handle. ★It is a weak
    /// handle★ — it does not hold the registry (if the registry vanished first, there is nothing to remove).
    fn leave_roster(&self, id: PeerId) -> LeaveRoster<W> {
        let registry = Arc::downgrade(&self.inner);
        Box::new(move |me: &Peer<W>| {
            let Some(registry) = registry.upgrade() else {
                return;
            };
            // A removed handle is dropped outside the lock.
            let _left = {
                let mut roster = registry.roster();
                match roster.peers.get(&id) {
                    Some(listed) if listed.same(me) => roster.peers.remove(&id),
                    _ => None,
                }
            };
        })
    }

    /// Removes it from the registry and closes that peer. That handle's receiver end ends with `Closed` → `None` when the supervisor finishes
    /// teardown.
    pub fn remove(&self, id: &PeerId) {
        let removed = self.inner.roster().peers.remove(id);
        if let Some(peer) = removed {
            peer.close();
        }
    }

    /// Removes all peers from the registry, and after each drains the outbound work it had accepted, closes ([`Peer::close_after_flush`] ·
    /// [`CloseCode::GOING_AWAY`]) — waits for those supervisors to all finish until `deadline` (the registry's clock) (TRD 3 §3-0 ④ — the daemon
    /// shutdown waits on this). `Err` = not all finished by the deadline — the remaining supervisors keep folding and finish at their own deadlines
    /// ([`Policy::flush_timeout`] · the teardown limit).
    ///
    /// ★It drains rather than closing at once★ — the reason for this function is to let the last answer
    /// put in just before closing (something like a stop Ack) go out. To close at once, use [`Registry::remove`].
    ///
    /// ★It waits only for peers that were on the registry at the moment of the call★ — it does not know
    /// a supervisor still folding because `add` replaced it or `remove` took it out before that.
    ///
    /// ★It folds the registry — it does not reopen★ — [`Registry::add`] · [`Registry::attach`] after the call do not get onto the registry
    /// and are closed before dialing or handshaking (their receiver end ends with `Closed` → `None`,
    /// and an accepted channel loads a close and is dropped
    /// without reading or writing anything). This function does not wait for that. It is meant so that a peer the accept loop put on crossing the fold does not
    /// remain unclosed on the emptied registry — even so, stand up the accept loop first (an accepted channel put on after the fold is closed at once anyway).
    ///
    /// ★The close starts at the place of the call★ — the returned future only waits. Dropping it, the close continues
    /// (same as [`Peer::close_after_flush`]). The deadline is measured from the first poll of that future.
    ///
    /// ★Do not wait on the task that drains those peers' receiver ends★ — a supervisor's end depends on the receiving side draining
    /// (crate header 「알려진 한계」 (known limits)), so waiting there leaves that supervisor until the teardown limit and this wait goes to `deadline`.
    /// Keep draining the receiver end to the end (`None`) — if released, that peer's remaining outbound work is dropped (crate header 「받는 쪽
    /// 불변식」 (receiver invariants) 5).
    pub fn close_all(
        &self,
        deadline: Duration,
    ) -> impl Future<Output = Result<(), Elapsed>> + Send + 'static {
        let peers: Vec<Peer<W>> = {
            let mut roster = self.inner.roster();
            roster.folded = true;
            roster.peers.drain().map(|(_, peer)| peer).collect()
        };
        // The handle may be released here — the marker holds the control channel and the drain continues (`Peer::close_after_flush`).
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

    /// A snapshot of now, sorted by name tag. ★This is the ingredient the shell uses to fold 「앱 전체」 (the whole app)★.
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

    /// Pushes the same thing to all peers.
    ///
    /// ★Encode once and hand out the frame★ — so it does not require `Clone` on `W::Out`. In exchange,
    /// the fanout has **fixed notification semantics**: it does not wait for a reply.
    ///
    /// A partial failure comes up as that peer's [`TransportEvent::Dropped`](crate::TransportEvent::Dropped) **on that peer's
    /// receiver end** — ★not creating something that disappears silently is the only tricky part of this function★ (today's
    /// `net` fanout is silent in exactly that place). What counts and reports is that peer's supervisor, so the report passes the same ordering area as other things that
    /// supervisor emits and is not dropped even when the line is full (`Peer::send_frame`).
    ///
    /// ★What could not be put in because a peer the app called [`Peer::close`] on is not in the operating phase is not reported★ — the same even if it remains
    /// on the registry. The count of losses after the app said it would close has no reader and a clean shutdown comes first (user decision
    /// 2026-09-26 — `lib.rs` 「알려진 한계」 (known limits)). What was in the queue before that is counted by the supervisor.
    pub fn broadcast(&self, out: W::Out) {
        let frame = self.inner.wire.encode(&out);
        let targets: Vec<Peer<W>> = self.inner.roster().peers.values().cloned().collect();
        for peer in targets {
            // Failure is counted by that peer's supervisor (`Peer::send_frame`).
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

        /// ★You must hold the returned receiver end★ — if dropped, the supervisor folds at once.
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

    /// Ends the supervisor task abnormally — if the channel opens, it panics in `start`.
    struct PanickingHandshake;

    impl Handshake for PanickingHandshake {
        fn start(&self) -> HandshakeStep {
            panic!("시험: 핸드셰이크 시작에서 일부러 패닉한다");
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

    /// Runs the supervisors and drains — while giving the supervisor room to move what it holds in the ordering area into the line, until nothing
    /// comes up in one turn.
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

    /// Has it ended now — nothing held and the enqueue end released (or closed). ★If there is something held, it takes one out and discards it★ —
    /// call it after taking everything out.
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
                "★수신단에는 그 손잡이의 감독이 낸 것만 온다★"
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

    // A peer that gave up is asleep waiting only for control — that supervisor must wake by itself and report.
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
            "받은 상대의 수신단에는 신고가 없다"
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
        assert!(ended(&mut inbox).await, "내린 상대의 수신단은 끝난다");
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
            "★0 으로 되감기면 「최대 세대 미만은 무시」 방어가 새 연결을 통째로 버린다★"
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
        // A point where no task has run yet — the old handle should already have received the shutdown signal.
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
            "상대별 예산 1이 명부 기본값 3을 이긴다"
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
        // Push in with nobody reading, and the peer closes at once.
        for seq in 1..=200 {
            endpoint.push(TestWire::chunk(1, 1, seq));
        }
        settle().await;
        assert_eq!(
            peer.peer_state(),
            PeerState::Live,
            "위로 올라갈 자리가 없는 것은 이 연결의 잘못이 아니다"
        );
        endpoint.close();
        settle().await;

        let items = drain_until_quiet(&mut inbox).await;
        assert_eq!(
            chunk_seqs(&items),
            (1..=200).collect::<Vec<_>>(),
            "★줄이 찬 동안 온 것도 하나도 안 버려지고 온 순서대로 나온다★"
        );
        assert_eq!(
            kinds(&items),
            vec!["Connected", "Disconnected"],
            "★끊김은 그 앞에 온 것들 뒤에 오고, 유실 신고는 없다★"
        );
        assert!(
            matches!(
                items.last().and_then(|i| i.as_event()),
                Some(TransportEvent::Disconnected { .. })
            ),
            "끊김이 마지막이다"
        );
    }

    // The shape of one daemon rewind ring (4096) arriving at once — eight times the line length (default 512).
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
            "★받는 쪽이 밀린 동안 감독은 줄 · 앞잡이 줄 분량까지만 읽고 나머지는 통로에 둔다★(배압)"
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
        // Fill to the brim a receiver end that nobody reads.
        for seq in 1..=500 {
            noisy_link.push(TestWire::chunk(1, 1, seq));
        }
        settle().await;

        let (quiet, mut quiet_inbox) = f.add("quiet", immediate());
        settle().await;
        assert_eq!(
            quiet.peer_state(),
            PeerState::Live,
            "★남의 홍수로 내 연결이 끊기면 ADR-0180 결정 8 이 깨진다★"
        );
        assert_eq!(noisy.peer_state(), PeerState::Live);
        assert_eq!(f.net.dials(), 2, "재연결이 돌지 않았다");
        assert_eq!(
            kinds(&drain(&mut quiet_inbox)),
            vec!["Connected"],
            "남의 홍수가 내 Connected 를 버리게 하지 못한다"
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
        // With nobody reading, the noisy side overfills its own receiver end.
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
            "★안 비운 수신단의 상대는 자기 통로에서 배압을 받는다★ — 줄이 찬 뒤로는 그 통로를 안 읽는다"
        );
        assert_eq!(
            quiet_link.unread(),
            0,
            "★조용한 쪽의 통로는 남의 배압과 무관하게 다 읽힌다★"
        );
        assert_eq!(
            labels(&drain(&mut quiet_inbox)),
            vec!["Connected:1", "msg:q0", "msg:q1", "msg:q2"],
            "★조용한 상대의 것은 하나도 안 버려지고, 시끄러운 쪽이 안 비워진 채여도 한 번의 정착 안에 다 와 있다★"
        );
        assert_eq!(noisy.peer_state(), PeerState::Live);
        assert_eq!(quiet.peer_state(), PeerState::Live);

        // The overflowed noisy side's items too are held by that supervisor and emitted in order.
        let noisy_items = drain_until_quiet(&mut noisy_inbox).await;
        assert_eq!(chunk_seqs(&noisy_items), (1..=500).collect::<Vec<_>>());
    }

    // ── 3-0 U1c: the registry hands the byte budget to the supervisor ──
    //
    // ★Measure with the real budget value★ — supervisor tests put in a small budget directly, so even if the registry handed over a wrong value (0 · no limit)
    //   it would not show there. For a peer that does not use streams, the whole budget (16 MiB) is the read share, so with two frames larger than half of it
    //   the read gate closes at the second (the same shape as the net `CONN_RX_MAX_BYTES` test — the same value).
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
            "★둘째 장에서 읽기 몫을 넘기고 셋째는 통로에 남는다★"
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
        assert_eq!(tags, vec![0, 1, 2], "★손실 0 · 순서 그대로★");
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
            "명부 기본값 2 가 아니라 상대별 8 이 그 줄의 길이다"
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
    // ★Do not push the clock★ — if the backoff wait is woken by the clock, the report comes out stuck on the start of that turn, and we cannot tell whether the supervisor
    //   woke by itself. Line length 1 = a line filled by one `ConnectFailed`.
    #[tokio::test]
    async fn a_broadcast_loss_wakes_a_backing_off_supervisor_and_survives_a_full_inbox() {
        let f = Fixture::new(Policy {
            inbound_queue: 1,
            ..Policy::default()
        });
        f.net.fail_forever();
        let (down, mut inbox) = f.add("down", immediate());
        settle_until("백오프", || down.peer_state() == PeerState::Backoff).await;

        f.registry.broadcast(TestOut::Notice("all".into()));
        settle().await;
        let items = drain_until_quiet(&mut inbox).await;
        assert_eq!(
            kinds(&items),
            vec!["ConnectFailed", "Dropped"],
            "★찬 수신단이어도 신고가 버려지지 않고 감독이 낸 순서대로 온다★"
        );
        assert_eq!(outbound_losses(&items), vec![1]);
        assert_eq!(f.net.dials(), 1, "백오프가 끝나 다시 건 것이 아니다");
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
            .expect("기다리던 recv 가 첫 사건을 못 봤다 — 영구 대기 회귀다")
            .expect("받는 태스크가 패닉했다");
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
            .expect("기다리던 recv 가 새 항목을 못 봤다")
            .expect("받는 태스크가 패닉했다");
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
        settle_until("a 접힘", || early.peer_state() == PeerState::Closed).await;
        assert_eq!(
            other.peer_state(),
            PeerState::Live,
            "★한 수신단을 닫아도 다른 상대는 그대로다★"
        );
        assert_eq!(f.net.dials(), 2, "소비자 부재는 재연결할 일이 아니다");

        let items = drain(&mut inbox);
        assert_eq!(
            labels(&items),
            vec!["Connected:1", "msg:before"],
            "닫기 전에 든 것은 꺼낼 수 있다"
        );
        assert!(ended(&mut inbox).await, "닫힌 수신단은 다 꺼내면 끝난다");
    }

    #[tokio::test]
    async fn dropping_an_inbox_folds_its_peer_and_a_re_added_name_starts_afresh() {
        let f = Fixture::new(Policy::default());
        let (early, inbox) = f.add("a", immediate());
        settle().await;
        let link = f.net.accept().unwrap();
        drop(inbox);
        link.push(notice("nobody listens"));
        settle_until("a 접힘", || early.peer_state() == PeerState::Closed).await;
        assert_eq!(f.net.dials(), 1, "받는 쪽이 사라진 상대는 다시 걸지 않는다");

        let (again, mut fresh) = f.add("a", immediate());
        settle_until("다시 올린 a 운영", || {
            again.peer_state() == PeerState::Live
        })
        .await;
        assert_eq!(
            f.net.dials(),
            2,
            "★옛 수신단을 놓은 것이 같은 이름표의 새 손잡이에 번지지 않는다★"
        );
        assert_eq!(labels(&drain(&mut fresh)), vec!["Connected:2"]);
    }

    // ── 3-0 U1d: even with a quiet peer, closing or releasing the receiver end folds without a next frame ──
    #[tokio::test]
    async fn closing_an_inbox_on_a_silent_peer_folds_it_and_still_drains_what_was_queued() {
        let f = Fixture::new(Policy::default());
        let (peer, mut inbox) = f.add("a", immediate());
        settle_until("연결", || peer.peer_state() == PeerState::Live).await;
        let link = f.net.accept().unwrap();
        link.push(notice("before"));
        settle().await;

        inbox.close();
        settle().await;
        assert!(
            peer.state().has_changed().is_err(),
            "★넣을 것이 없어도 감독이 접고 정리를 마쳤다★ — 다음 프레임도 상한도 기다리지 않았다"
        );
        assert_eq!(peer.peer_state(), PeerState::Closed);
        assert_eq!(f.net.dials(), 1, "다시 걸지 않았다");
        assert_eq!(
            labels(&drain(&mut inbox)),
            vec!["Connected:1", "msg:before"],
            "닫기 전에 든 것은 다 꺼낼 수 있다"
        );
        assert!(
            ended(&mut inbox).await,
            "그 뒤는 끝이다 — `Closed` 는 실을 데가 없었다"
        );
    }

    // Why 30 times = without the check before polling the dial phase, the dial future and the closed arm become ready together and which
    //   is polled first is random (`select!`) — a single run could miss that the check is gone.
    #[tokio::test]
    async fn an_inbox_dropped_before_its_supervisor_runs_never_dials() {
        let f = Fixture::new(Policy::default());
        for n in 0..30 {
            let (peer, inbox) = f.add(&format!("a{n}"), immediate());
            drop(inbox);
            settle().await;
            assert_eq!(peer.peer_state(), PeerState::Closed, "{n}");
            assert!(peer.state().has_changed().is_err(), "{n}: 감독이 끝났다");
        }
        assert_eq!(
            f.net.dials(),
            0,
            "★첫 사건을 넣기도 전에 놓인 수신단이면 통로를 아예 안 건다★"
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
            "★옛 수신단은 옛 감독의 Closed 뒤에 끝난다★"
        );
        assert_eq!(
            labels(&drain(&mut new_inbox)),
            vec!["Connected:1"],
            "★새 수신단에는 새 감독의 것만 온다★"
        );
        assert!(!ended(&mut new_inbox).await);
    }

    // ★The old supervisor emits `Closed` only after fully closing the channel, and in the meantime the new supervisor attaches★ — with one line the two would be mixed and
    //   which supervisor an item belongs to would have to be told by generation.
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
            "옛 감독 안에서는 끊김 → 끝 순서다"
        );
        assert!(ended(&mut old_inbox).await);
        assert_eq!(
            labels(&drain(&mut new_inbox)),
            vec!["Connected:2"],
            "★옛 감독의 꼬리는 새 수신단에 섞이지 않는다★"
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
                "★Closed 가 마지막 항목이다★ — 뒤에 올린 감독의 것이 끼지 않는다"
            );
            assert!(
                ended(inbox).await,
                "★Closed 뒤에는 감독이 넣는 끝을 놓아 None 이다★"
            );
        }
    }

    #[tokio::test]
    async fn churning_a_name_before_its_supervisor_runs_never_dials_and_ends_every_inbox() {
        let f = Fixture::new(Policy::default());
        let name = PeerId::new("a");
        let mut inboxes = Vec::new();
        for _ in 0..30 {
            // Removed before the supervisor runs — the supervisor sees the close before dialing and ends having emitted only a `Closed`.
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
            settle_until("연결", || peer.peer_state() == PeerState::Live).await;
            let link = f.net.accept().unwrap();
            link.push(notice("held"));
            settle().await;
            peer.close();
            settle_until("접힘", || peer.peer_state() == PeerState::Closed).await;
            f.clock.advance(waited);
            settle().await;

            let items = drain_until_quiet(&mut inbox).await;
            assert_eq!(labels(&items), expected, "{waited:?} 기다린 뒤");
            assert!(
                ended(&mut inbox).await,
                "{waited:?}: ★감독이 넣는 끝을 놓았다 — 상한을 넘겼으면 `Closed` 없는 `None` 이 그 버림이다★"
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
        settle_until("연결", || old.peer_state() == PeerState::Live).await;
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
            "옛 감독이 아직 안 비운 옛 수신단을 기다리며 정리 중이다"
        );

        drop(old_inbox);
        settle().await;
        assert!(
            old.state().has_changed().is_err(),
            "★옛 수신단을 놓으면 옛 감독의 정리가 상한을 기다리지 않고 끝난다★"
        );
    }

    // ── 3-0 U1a: fanout loss is reported to an open peer · an ended peer receives nothing even if it remains on the registry ──
    //
    // That the rejection after the app called close is not counted (user decision 2026-09-26) is something this test cannot measure — the supervisor has already ended
    //   and the receiver end has ended, so even if there were a count it would not show. That gap is measured by the test right below.
    #[tokio::test]
    async fn a_broadcast_loss_is_reported_for_an_open_peer_and_never_reaches_an_ended_one() {
        let f = Fixture::new(Policy::default());
        let (_down, mut down_inbox) = f.add("down", RejectingHandshake::new("nope"));
        let (shut, mut shut_inbox) = f.add("shut", immediate());
        settle().await;
        shut.close();
        settle_until("shut 접힘", || shut.peer_state() == PeerState::Closed).await;
        assert_eq!(f.registry.len(), 2, "앱이 닫았어도 명부에는 남아 있다");
        drain(&mut down_inbox);
        drain(&mut shut_inbox);

        f.registry.broadcast(TestOut::Notice("all".into()));
        settle().await;
        assert_eq!(outbound_losses(&drain(&mut down_inbox)), vec![1]);
        assert!(drain(&mut shut_inbox).is_empty());
        assert!(ended(&mut shut_inbox).await);
    }

    // ★Measures the gap after calling close and before the supervisor handles it★ — a supervisor asleep in the give-up wait, if it receives control and a wake together,
    //   has no fixed order for which it looks at first, so if the handle counts in that gap, `Dropped` may be loaded ahead of `Closed`. So we
    //   close without yielding, push right away, and then look at the handle-side count first.
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
            "★앱이 닫기를 부른 뒤의 거절은 세지 않는다★(사용자 결정 2026-09-26)"
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
        settle_until("감독 태스크가 끝남", || {
            peer.state().has_changed().is_err()
        })
        .await;
        assert!(
            drain(&mut inbox).is_empty(),
            "★정리를 거치지 않았으니 `Closed` 가 없다★"
        );
        assert!(
            ended(&mut inbox).await,
            "★넣는 끝이 태스크와 함께 떨어져 `None` 이다 — 상한 초과와 겉모양이 같다★"
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
        assert!(spawned.is_err(), "런타임 밖에서는 감독을 못 띄운다");
        assert_eq!(
            registry.len(),
            0,
            "★패닉 한 번이 명부를 못 쓰게 만들지 않는다★"
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
            "★명부가 사라져도 손잡이를 쥔 상대는 계속 돈다★"
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
            "손잡이가 다 사라지면 수신단이 끝난다"
        );
    }

    // ── 3-0 U3: close_all ──

    #[tokio::test]
    async fn close_all_flushes_every_peer_then_closes_it_and_empties_the_roster() {
        let fx = Fixture::new(Policy::default());
        let (a, mut inbox_a) = fx.add("a", immediate());
        let (b, mut inbox_b) = fx.add("b", immediate());
        settle_until("둘 다 운영", || {
            a.peer_state().is_live() && b.peer_state().is_live()
        })
        .await;
        let endpoints = [fx.net.accept().unwrap(), fx.net.accept().unwrap()];
        // ★Block the write★ — the close must come in while 「last」 is being written, to tell it from a regression that closes at once.
        for endpoint in &endpoints {
            endpoint.stall_writes(true);
        }
        a.notify(TestOut::Notice("last".into())).unwrap();
        b.notify(TestOut::Notice("last".into())).unwrap();

        let closing = tokio::spawn(fx.registry.close_all(Duration::from_secs(3)));
        settle().await;
        assert!(!closing.is_finished(), "막힌 쓰기를 비우는 중이다");
        for endpoint in &endpoints {
            endpoint.stall_writes(false);
        }
        settle_until("시계를 밀지 않고 끝났다", || {
            closing.is_finished()
        })
        .await;
        assert_eq!(closing.await.expect("close_all 태스크"), Ok(()));
        assert!(fx.registry.is_empty(), "명부에서 내렸다");
        for endpoint in &endpoints {
            assert_eq!(
                endpoint.drain(),
                vec![
                    ClientMsg::Frame(notice("last")),
                    ClientMsg::Close(Close::new(CloseCode::GOING_AWAY, "going away")),
                ],
                "★곧바로 닫지 않고 앞서 받아 둔 것을 먼저 쓴다★"
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

    // ★It closes even if you do not await the future★ — the close starts at the place of the call and the future only waits.
    #[tokio::test]
    async fn close_all_closes_even_when_nobody_waits_for_it() {
        let fx = Fixture::new(Policy::default());
        let (peer, mut inbox) = fx.add("a", immediate());
        settle_until("운영", || peer.peer_state().is_live()).await;
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

    // ★Put close_all's deadline (1 s · 3 s) on both sides of the drain's deadline (2 s)★ — if it does not finish within, it is `Err`, and the supervisor
    //   ends at its own deadline regardless of the waiting.
    #[tokio::test]
    async fn close_all_waits_only_up_to_its_deadline() {
        assert_eq!(Policy::default().flush_timeout, Duration::from_secs(2));
        for (deadline, in_time) in [
            (Duration::from_secs(1), false),
            (Duration::from_secs(3), true),
        ] {
            let fx = Fixture::new(Policy::default());
            let (peer, mut inbox) = fx.add("a", immediate());
            settle_until("운영", || peer.peer_state().is_live()).await;
            let endpoint = fx.net.accept().unwrap();
            endpoint.stall_writes(true);
            peer.notify(TestOut::Notice("stuck".into())).unwrap();
            settle().await;

            let closing = tokio::spawn(fx.registry.close_all(deadline));
            settle().await;
            fx.clock.advance(Duration::from_secs(1));
            settle().await;
            assert_eq!(closing.is_finished(), !in_time, "{deadline:?}: 1 s 뒤");
            fx.clock.advance(Duration::from_secs(1));
            settle_until("비우기의 시한에 감독이 끝났다", || {
                closing.is_finished()
            })
            .await;
            let expected = if in_time { Ok(()) } else { Err(Elapsed) };
            assert_eq!(
                closing.await.expect("close_all 태스크"),
                expected,
                "{deadline:?}"
            );
            let items = drain_until_quiet(&mut inbox).await;
            assert_eq!(kinds(&items).last(), Some(&"Closed"), "{deadline:?}");
            assert!(ended(&mut inbox).await, "{deadline:?}");
        }
    }

    // ── 3-0 U6b: an accepted peer (`attach`) ──

    /// An auth window for these tests (they run on `ManualClock`, so the value only sets the step size — the
    /// daemon's real window is 15 s, TRD 3 revision 9 ⑧).
    const AUTH: Duration = Duration::from_secs(1);
    const GOOD: &str = "auth|good";

    fn auth_policy() -> Policy {
        Policy {
            connect_timeout: AUTH,
            ..Policy::default()
        }
    }

    /// Receiving-side auth — the first frame must be [`GOOD`] to be the operating phase. Otherwise it is a reject that makes it redial (stale credentials — TRD 3 §3-0 ⑤).
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

    /// The dialing-side end — driven by hand without a supervisor.
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

        /// Opens one dialing-side end and takes that channel from the listener — `upgraded_at` = now.
        async fn connect(&mut self) -> (Near, Accepted) {
            let (tx, rx) = self
                .connector
                .dial(&Address::new("mem://acceptor"))
                .await
                .expect("걸기");
            let accepted = self.listener.accept().await.expect("받기");
            (Near { tx, rx }, accepted)
        }

        /// The receiving-side end of the last channel.
        fn far(&self) -> MemoryLinkControl {
            self.connector.latest().expect("통로").acceptor
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
                .expect("거는 쪽 쓰기");
        }

        /// Reads everything that has arrived now — it does not wait, and does not read past the end (close · error).
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

    // ★See the end without pushing the clock★ — if it goes to reconnect, it stands in the backoff wait and `Closed` does not come.
    #[tokio::test]
    async fn an_accepted_peer_that_loses_its_link_ends_with_that_cause_and_never_redials() {
        let mut a = Accepting::new(auth_policy());
        let (near, accepted) = a.connect().await;
        let (peer, mut inbox) = a.attach("conn-1", accepted, immediate());
        settle_until("운영", || peer.peer_state().is_live()).await;
        drop(near);
        let items = drain_until_quiet(&mut inbox).await;
        assert_eq!(
            labels(&items),
            vec![
                "Connected:1",
                "Disconnected:1:ReadFailed",
                "Closed:1:ReadFailed"
            ],
            "거는 쪽 사건(ConnectFailed · GaveUp) 없이 끝난다"
        );
        assert!(ended(&mut inbox).await);
        assert_eq!(peer.peer_state(), PeerState::Closed);
        assert!(a.registry.is_empty(), "끝나면 명부에서 스스로 내린다");
        a.clock.advance(Duration::from_secs(60));
        settle().await;
        assert_eq!(a.redials.dials(), 0, "받은 상대는 다시 걸지 않는다");
    }

    // The daemon adapter tells the read end (finish running the remaining commands) from the write end (cut at once) by this cause (TRD 3 §3-0 ①).
    #[tokio::test]
    async fn read_end_and_write_end_causes_reach_closed_on_the_accepted_side() {
        let write_deadline = Policy::default().write_deadline;
        for case in [
            "상대가 닫음",
            "읽기 오류",
            "말 없이 사라짐",
            "쓰기 오류",
            "쓰기 시한",
            "침묵",
        ] {
            let mut a = Accepting::new(auth_policy());
            let (mut near, accepted) = a.connect().await;
            let far = a.far();
            let (peer, mut inbox) = a.attach("conn-1", accepted, immediate());
            settle_until("운영", || peer.peer_state().is_live()).await;
            let expected = match case {
                "상대가 닫음" => {
                    near.tx.close(Close::going_away()).await;
                    "PeerClosed"
                }
                "읽기 오류" => {
                    far.fail_reads("boom");
                    "ReadFailed"
                }
                "말 없이 사라짐" => {
                    drop(near);
                    "ReadFailed"
                }
                "쓰기 오류" => {
                    far.fail_writes();
                    peer.notify(TestOut::Notice("x".into())).unwrap();
                    "WriteFailed"
                }
                "쓰기 시한" => {
                    far.stall_writes(true);
                    peer.notify(TestOut::Notice("x".into())).unwrap();
                    settle().await;
                    a.clock.advance(write_deadline);
                    settle().await;
                    // The close that drops the channel is also blocked — that wait too is cut by the write deadline.
                    a.clock.advance(write_deadline);
                    "WriteDeadline"
                }
                // The dialing side does not read, so there is no reply to our ping either.
                _ => {
                    for _ in 0..5 {
                        settle().await;
                        a.clock.advance(Duration::from_secs(10));
                    }
                    "Idle"
                }
            };
            // The write-side end also counts that half-written notification as `Dropped` — what is measured here is the end cause.
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

    // An end that did not enter the operating phase also tells the read end from the write end (at each failure place of `handshake_once`).
    #[tokio::test]
    async fn a_handshake_that_ends_on_the_link_reports_which_end_failed() {
        for case in ["상대가 닫음", "말 없이 사라짐", "쓰기 오류", "쓰기 시한"] {
            let mut a = Accepting::new(auth_policy());
            let (mut near, accepted) = a.connect().await;
            let far = a.far();
            // The write-side branch is measured with a handshake that speaks first.
            let handshake: Arc<dyn Handshake> = match case {
                "쓰기 오류" | "쓰기 시한" => Arc::new(HelloHandshake),
                _ => Arc::new(TokenGate),
            };
            let expected = match case {
                "상대가 닫음" => "PeerClosed",
                "말 없이 사라짐" => "ReadFailed",
                "쓰기 오류" => {
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
                "상대가 닫음" => near.tx.close(Close::going_away()).await,
                "말 없이 사라짐" => drop(near),
                "쓰기 시한" => a.clock.advance(AUTH),
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
                "{n}: 거는 쪽 사건(Rejected · ConnectFailed · GaveUp) 없이 끝만 — 거절은 우리가 했다"
            );
            assert!(ended(&mut inbox).await, "{n}");
            assert_eq!(
                near.arrived().await,
                vec![Seen::Closed(Some(Close::new(
                    RejectKind::Retryable.close_code(),
                    "bad token"
                )))],
                "{n}: 거절 코드는 상대에게 간다"
            );
        }
        // A terminal reject also comes with only the end — `Rejected` is a dialing-side event that promises a `GaveUp` right after it.
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
        // A channel that says nothing — dropped without a close at the auth deadline.
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
        settle_until("운영", || peer.peer_state().is_live()).await;
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec!["Connected:1"]
        );
        assert_eq!(a.registry.len(), 1, "실패한 상대는 명부에 남지 않는다");
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
            "업그레이드 뒤 999 ms — 아직 시한 전"
        );
        a.clock.advance(Duration::from_millis(1));
        assert_eq!(
            labels(&drain_until_quiet(&mut inbox).await),
            vec!["Closed:0:Idle"],
            "붙인 뒤로는 400 ms 뿐이다"
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
                    settle_until("운영", || peer.peer_state().is_live()).await;
                    drop(near);
                }
                1 => near.say("auth|stale").await,
                _ => {
                    near.say(GOOD).await;
                    settle_until("운영", || peer.peer_state().is_live()).await;
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
                "{n}: 받은 상대는 세대 표에 안 든다"
            );
            assert_eq!(permits.in_use(), 0, "{n}");
        }
        assert_eq!(a.listener.queued(), 0);
        assert_eq!(a.redials.dials(), 0);
    }

    /// A minimal packet where both sides use the same vocabulary — only asking and answering. `TestWire` is one side's (client's) vocabulary so it cannot connect two registries
    /// face to face.
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
            unreachable!("스트림을 안 쓴다")
        }
    }

    /// For every question that comes to the receiver end, it answers on the connection the question came from. It returns the number answered.
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

    // It is a transport-level round trip — it measures not a daemon-to-daemon vocabulary but whether the receiving side's place carries bidirectional requests (TRD 3 §6-0 U6).
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
        let accepted = listener.accept().await.expect("받기");
        let (to_client, mut server_inbox) =
            server.attach(PeerId::new("conn-1"), accepted, immediate());
        settle_until("양쪽 운영", || {
            to_server.peer_state().is_live() && to_client.peer_state().is_live()
        })
        .await;

        // The request is awaited outside the task that drains the receiver end (crate header 「받는 쪽 불변식」 (receiver invariants) 1).
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
                .expect("답이 안 왔다")
                .expect("요청 태스크가 패닉했다")
                .expect("요청이 실패했다");
            assert_eq!(got, expected);
        }
    }

    #[tokio::test]
    async fn an_accepted_peer_flushes_what_it_holds_before_closing() {
        let mut a = Accepting::new(auth_policy());
        let (mut near, accepted) = a.connect().await;
        let far = a.far();
        let (peer, mut inbox) = a.attach("conn-1", accepted, immediate());
        settle_until("운영", || peer.peer_state().is_live()).await;
        far.stall_writes(true);
        peer.notify(TestOut::Notice("ack".into())).unwrap();
        peer.notify(TestOut::Notice("last".into())).unwrap();
        let closing = tokio::spawn(peer.close_after_flush(CloseCode::GOING_AWAY, "bye"));
        settle().await;
        assert!(!closing.is_finished(), "막힌 쓰기를 비우는 중이다");
        far.stall_writes(false);
        settle_until("비우고 닫았다", || closing.is_finished()).await;
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

    // ★The old supervisor ends after the new one is raised★ — the registry removal at its end must not remove the new one.
    #[tokio::test]
    async fn re_attaching_a_name_closes_the_old_peer_and_its_end_leaves_the_new_one_listed() {
        let mut a = Accepting::new(auth_policy());
        let (mut first, accepted) = a.connect().await;
        let (old, mut old_inbox) = a.attach("conn", accepted, immediate());
        settle_until("옛것 운영", || old.peer_state().is_live()).await;
        let (second, accepted) = a.connect().await;
        let (new, mut new_inbox) = a.attach("conn", accepted, immediate());
        settle_until("새것 운영", || new.peer_state().is_live()).await;
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
            "갈아치운 손잡이의 계수기를 물려받는다 — 세대가 되감기지 않는다"
        );
        assert!(
            a.registry
                .get(&PeerId::new("conn"))
                .is_some_and(|listed| listed.same(&new)),
            "옛 감독의 끝이 새것을 내리지 않는다"
        );
        assert_eq!(a.generation_names(), 0);
        drop(second);
        drain_until_quiet(&mut new_inbox).await;
        assert!(a.registry.is_empty());
    }

    // If a name tag in the table (`add`) is replaced by an accepted channel, it continues the table's counter (the table does not grow), and if an accepted peer is replaced
    //   with `add`, that counter is put on the table — either way the same number is never drawn twice.
    #[tokio::test]
    async fn a_name_passed_between_add_and_attach_never_mints_the_same_generation_twice() {
        let mut a = Accepting::new(auth_policy());
        let x = PeerId::new("x");
        let (dialed, _dialed_inbox) =
            a.registry
                .add(x.clone(), Address::new("mem://x"), immediate());
        settle_until("add 운영", || dialed.peer_state().is_live()).await;
        assert_eq!(dialed.generation(), Generation(1));
        let (_near, accepted) = a.connect().await;
        let (attached, _attached_inbox) = a.attach("x", accepted, immediate());
        settle_until("attach 운영", || attached.peer_state().is_live()).await;
        assert_eq!(attached.generation(), Generation(2));
        assert_eq!(a.generation_names(), 1, "읽기만 했다");

        let (_near, accepted) = a.connect().await;
        let (attached, _attached_inbox) = a.attach("y", accepted, immediate());
        settle_until("attach 운영", || attached.peer_state().is_live()).await;
        assert_eq!(attached.generation(), Generation(1));
        let (dialed, _dialed_inbox) =
            a.registry
                .add(PeerId::new("y"), Address::new("mem://y"), immediate());
        settle_until("add 운영", || dialed.peer_state().is_live()).await;
        assert_eq!(dialed.generation(), Generation(2));
        assert_eq!(a.generation_names(), 2);
    }

    #[tokio::test]
    async fn after_close_all_an_attach_or_an_add_is_closed_at_once_and_never_listed() {
        let mut a = Accepting::new(auth_policy());
        let (_listed_near, accepted) = a.connect().await;
        let (listed, mut listed_inbox) = a.attach("conn-0", accepted, immediate());
        settle_until("운영", || listed.peer_state().is_live()).await;
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
            "접은 명부에 올린 상대는 걸지도 않는다"
        );

        assert_eq!(
            labels(&drain_until_quiet(&mut listed_inbox).await),
            vec![
                "Connected:1",
                "Disconnected:1:Explicit",
                "Closed:1:Explicit"
            ]
        );
        settle_until("접기 끝", || folding.is_finished()).await;
        assert_eq!(folding.await.expect("close_all 태스크"), Ok(()));
        assert!(a.registry.is_empty());
    }

    #[tokio::test]
    async fn the_upgrade_permit_is_held_through_the_handshake_and_given_back_at_live_or_at_failure()
    {
        let mut a = Accepting::new(auth_policy());
        let permits = a.listener.permits();
        let (mut near, accepted) = a.connect().await;
        assert_eq!(permits.in_use(), 1, "준비 큐");
        let (peer, _inbox) = a.attach("conn-1", accepted, Arc::new(TokenGate));
        settle().await;
        assert_eq!(
            permits.in_use(),
            1,
            "인증 대기 칸이 생기기 전까지 핸드셰이크 동안 쥔다"
        );
        near.say(GOOD).await;
        settle_until("운영", || peer.peer_state().is_live()).await;
        assert_eq!(permits.in_use(), 0, "운영 단계에 들면 놓는다");

        let (mut stale, accepted) = a.connect().await;
        assert_eq!(permits.in_use(), 1);
        let (_failed, _failed_inbox) = a.attach("conn-2", accepted, Arc::new(TokenGate));
        stale.say("auth|stale").await;
        settle().await;
        assert_eq!(
            permits.in_use(),
            0,
            "인증에 실패한 상대는 수신단을 안 비워도 칸을 놓는다"
        );
    }

    #[tokio::test]
    async fn an_accepted_peer_ignores_reconnect_now_and_folds_once_its_inbox_is_dropped() {
        let mut a = Accepting::new(auth_policy());
        let (mut near, accepted) = a.connect().await;
        let (peer, mut inbox) = a.attach("conn-1", accepted, immediate());
        settle_until("운영", || peer.peer_state().is_live()).await;
        peer.reconnect_now();
        settle().await;
        assert!(
            peer.peer_state().is_live(),
            "걸 주소가 없다 — 운영 중인 통로를 안 버린다"
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

    // Teardown waits for the receiving side to drain the receiver end — in the meantime an ended handle must not be seen on the registry.
    #[tokio::test]
    async fn an_accepted_peer_leaves_the_roster_before_its_teardown_waits_on_the_inbox() {
        let mut a = Accepting::new(Policy {
            inbound_queue: 1,
            ..auth_policy()
        });
        let (_near, accepted) = a.connect().await;
        let (peer, mut inbox) = a.attach("conn-1", accepted, immediate());
        settle_until("운영", || peer.peer_state().is_live()).await;
        peer.close();
        settle().await;
        assert!(
            a.registry.is_empty(),
            "수신단이 차 정리가 서 있어도 명부에는 없다"
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
        settle_until("감독이 끝났다", || a.registry.is_empty()).await;
        assert!(drain_until_quiet(&mut inbox).await.is_empty());
        assert!(ended(&mut inbox).await, "`Closed` 없는 `None`");
    }

    // ── 3-0 U6b: raising on a runtime with several worker threads ──
    //
    // ★`settle` cannot be used★ (current_thread only — `testing.rs`) — the end is seen by a wait with a real-time deadline. `RACE_THREADS` OS threads
    //   start together at a barrier and hammer the same registry — the supervisors meanwhile run on worker threads.

    const RACE_THREADS: usize = 4;
    const REAL_WAIT: Duration = Duration::from_secs(10);

    /// Drains the receiver end to the end (`None`) — panics if it does not finish within the real-time [`REAL_WAIT`].
    async fn drain_to_end(inbox: &mut Inbox<TestWire>) -> Vec<Incoming<TestWire>> {
        tokio::time::timeout(REAL_WAIT, async {
            let mut out = Vec::new();
            while let Some(item) = inbox.recv().await {
                out.push(item);
            }
            out
        })
        .await
        .expect("수신단이 실시간 시한 안에 안 끝났다")
    }

    /// Waits for a condition to hold up to the real-time [`REAL_WAIT`].
    async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
        tokio::time::timeout(REAL_WAIT, async {
            while !done() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("실시간 시한 안에 안 섰다: {what}"));
    }

    /// One accepted channel and its dialing-side end — upgrade end = `upgraded_at`.
    fn accepted_link(pool: &PermitPool, upgraded_at: Instant) -> (Near, Accepted) {
        let (near, far) = memory_link();
        let permit = pool.try_take().expect("업그레이드 칸");
        let accepted = Accepted::new(far.tx, far.rx, LinkInfo::default(), upgraded_at, permit);
        (
            Near {
                tx: near.tx,
                rx: near.rx,
            },
            accepted,
        )
    }

    /// [`RACE_THREADS`] OS threads cross a barrier together inside the runtime context and run `call(스레드 번호)`. ★This wait
    /// blocks the test-body thread★ — the body of a multi_thread test is not a worker but the `block_on` thread, so the supervisors keep running.
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
                .map(|thread| thread.join().expect("경합 스레드가 패닉했다"))
                .collect()
        })
    }

    /// Reads the dialing-side end to the end (close · error) — up to the real-time [`REAL_WAIT`]. ★Do not use [`Near::arrived`]★ — it
    /// does not wait and polls only once, so on a task whose tokio cooperative budget is exhausted it cannot read even what has arrived.
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
        .expect("거는 쪽 끝이 실시간 시한 안에 안 끝났다")
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

    // An accepted peer that ends at its first poll — the deadline has already passed (`Idle`) or the dialing side has already cut. If raising comes after
    //   starting, the handle of a supervisor that finished even removing itself on another worker is raised afterwards and remains forever (`Registry::launch`).
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
        assert_eq!(registry.len(), 0, "끝난 받은 상대는 명부에 남지 않는다");
        assert_eq!(pool.in_use(), 0);
    }

    // A peer raised on a folded registry has the close in before starting, so whichever worker it runs on first, it neither dials nor handshakes.
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
                "핸드셰이크 프레임(hello) 없이 닫기만 온다"
            );
        }
        assert_eq!(net.dials(), 0, "접은 명부에 올린 상대는 걸지 않는다");
        assert!(registry.is_empty());
        assert_eq!(pool.in_use(), 0);
    }

    // Several calls replace the same name tag at once with `add` · `attach`. ★The race place is when an accepted peer with no field in the table is raised★
    //   — if the place that takes out that handle and the place that chooses the counter split, another call sees neither the handle nor the table field, makes a new
    //   counter, and draws a generation that already went out again (`Registry::launch`). So each name tag starts in that state.
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
            wait_until("받은 상대 운영", || first.peer_state().is_live()).await;
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
            // Attach once more with the counter left in the table after the race — if the race put a new counter on the table, 1 is drawn again here.
            let (last, last_inbox) =
                registry.add(name.clone(), Address::new("mem://x"), immediate());
            wait_until("마지막 상대 운영", || last.peer_state().is_live()).await;
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
                "{name:?}: 같은 세대를 두 번 뽑았다 — {minted:?}"
            );
            drop(nears);
        }
    }

    /// The value of the panic that [`TrippingClock`] sets off.
    struct Tripped;

    /// A clock that panics once at the n-th `now()` of a designated thread — otherwise it is [`ManualClock`] as is. ★Filter by
    /// thread★ — so that reads other supervisors make on worker threads do not mix into the count.
    #[derive(Default)]
    struct TrippingClock {
        clock: ManualClock,
        trip: Mutex<Option<(std::thread::ThreadId, usize)>>,
    }

    impl TrippingClock {
        /// Makes the calling thread panic on its `nth` read.
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
            // Sets it off after releasing the lock — if it blew up while holding it, that poison would blow up other supervisors' reads too.
            if fire {
                std::panic::panic_any(Tripped);
            }
            self.clock.now()
        }

        fn sleep(&self, d: Duration) -> futures_util::future::BoxFuture<'static, ()> {
            self.clock.sleep(d)
        }
    }

    /// Runs `call` on an OS thread in the runtime context while catching panics — if it does not finish within the real-time [`REAL_WAIT`], it is treated as a deadlock
    /// and panics. ★Detach that thread★ — if we waited on a deadlocked thread, the test would hang instead of failing.
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
            .expect("실시간 시한 안에 안 끝났다 — 명부 자물쇠 교착")
    }

    // The replacing `attach` · `add` panics while building the supervisor inside the registry lock (the injected clock). ★If an accepted peer
    //   panics after being built, the registry removal that is dropped as it unwinds retakes the lock on the same thread★ (`new_peer` in `peer.rs`) — we call it
    //   on a thread with a real-time deadline so that deadlock does not hang the test. If the table were changed before building, the handle that was to be replaced
    //   would vanish from the registry (`Registry::launch`).
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

        // In a debug build, the clock assertion of `attach_with_policy` stands ahead of one read of the build.
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
            "받은 상대 짓기가 패닉해도 표는 그대로다"
        );
        assert_eq!(pool.in_use(), 0, "업그레이드 칸은 패닉과 함께 돌아간다");

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
            "건 상대 짓기가 패닉해도 표는 그대로다"
        );
        wait_until("갈아치우려던 상대 운영", || {
            first.peer_state().is_live()
        })
        .await;

        let (_near, accepted) = accepted_link(&pool, clock.now());
        let (replaced, _replaced_inbox) = off_thread({
            let (registry, name) = (registry.clone(), name.clone());
            move || registry.attach(name, accepted, immediate())
        })
        .expect("패닉 뒤에도 명부를 쓴다");
        assert!(registry.get(&name).is_some_and(|peer| peer.same(&replaced)));
        assert_eq!(
            kinds(&drain_to_end(&mut first_inbox).await).last(),
            Some(&"Closed"),
            "갈아치운 손잡이는 닫힌다"
        );
    }
}
