//! Shared handle to the command owner registry — every connection sees the same one copy (ADR-0155/0156 · TRD §3-7).
//!
//! The registry's rules (full registration last-wins · diff · removal on disconnect · cap) are owned by the tool crate's [`Roster`]. This file adds connection lifetime and the path to the owner — the
//! registry knows only names and owners, not which connections are attached or how to send to an owner (ADR-0154).
//!
//! The net stopping late packets from a dropped connection is only here (`Shared::refuse_if_detached`): [`Roster`] erases a dropped owner's registrations (ADR-0150 decision 3) and cannot tell a late registration or diff from a real handover.
//! Without this net a dead connection's packets pass, and the name points at a nonexistent owner answering `Available` for the daemon's lifetime. Do not build the same net on the [`Roster`] side — per-owner state would grow without bound like the trace (why ADR-0150 dropped it).
//!
//! The liveness check and the registry change happen in the same critical section: split, cleanup can slip between check and change and a dropped connection's registration lands afterwards. `on_disconnect` can overlap `on_text`
//! (`conn_port::ConnectionHandler` contract — the network layer only calls abort and does not wait), so the overlap is real.
//!
//! Not to be confused with this crate's `RosterBroadcast`, `RosterChanged`, `RosterDiff` (the agent/profile list notification port, ADR-0132); this registry maps command name → owner.
//!
//! The lock is taken and released only inside this file (ADR-0006): methods lock, then call [`Roster`] once or copy a handle and release — the caller sends outside the lock (a guard passed out would let one delivery waiting for a reply stall
//! registration, cleanup and query; rationale: the [`engram_dashboard_command::OwnerLookupSource`] comment).
//! Known cost: [`CommandRoster::entries`] clones the whole registry while locked (long at the cap; reducing it means changing the shape [`Roster`] lends out, outside this slice); [`CommandRoster::sink_of`] scans the table linearly while locked (connections have no cap; once per delivery).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::conn_port::{ConnId, FrameSink};
use engram_dashboard_base::sync;
use engram_dashboard_command::{
    CommandDecl, CommandError, ErrorCode, OwnerLookup, OwnerLookupSource, OwnerToken, Roster,
    RosterEntry,
};

use crate::connection_core::sanitize_for_log;

/// How many names to write on one disconnect line.
///
/// Without a cap the peer decides one log line's size: an owner can put up 512 names ([`Roster::MAX_NAMES_PER_OWNER`]) of 128 B each (`Roster::MAX_NAME_BYTES`) — 64 KiB, all client-chosen strings.
/// Truncation does not lose the count (the `names` field carries it); it loses the tail of "which names", and diagnosis uses the first few.
const MAX_LOGGED_NAMES: usize = 8;

/// Turns the names a client registered into one log field — shape bounded by the sanitizer, count by [`MAX_LOGGED_NAMES`].
///
/// One value through one door is not written in two shapes: the registration bounce uses the same form (`connection_core::ConnectionCore::refuse_names_i_answer`, `'name'` joined by commas), so a log scanner sees the same thing as the same.
fn logged_names(names: &[String]) -> String {
    let mut out = names
        .iter()
        .take(MAX_LOGGED_NAMES)
        .map(|name| format!("'{}'", sanitize_for_log(name)))
        .collect::<Vec<_>>()
        .join(", ");
    let elided = names.len().saturating_sub(MAX_LOGGED_NAMES);
    if elided > 0 {
        out.push_str(&format!(" (+{elided} more)"));
    }
    out
}

/// The wording for bouncing the registration or diff of a connection that is not attached.
///
/// A constant for tests: [`Roster`]'s check on others' names raises the same `CONFLICT`, so without looking at the wording "the connection-lifetime net caught it" and "the roster check caught it" cannot be told apart, and a test would stay green with this file's net deleted.
pub(crate) const DETACHED_REFUSAL: &str =
    "this connection is not attached — a registration lands only while its connection is up";

#[derive(Clone, Default)]
pub struct CommandRoster {
    inner: Arc<Mutex<Shared>>,
}

#[derive(Default)]
struct Shared {
    roster: Roster,
    /// The connections attached now and the path to each — under the same lock as the registry so no gap opens between check and change.
    // ADR-0154
    live: BTreeMap<ConnId, LiveConn>,
}

/// One attached connection — whose it is and how to reach it.
///
/// The owner token is computed at attach and stored, not traced back at query time: it derives from the connection id ([`CommandRoster::owner_of`]) by a provisional rule that slice B replaces with a client-made identifier (ADR-0150); a query tracing the connection id back
/// from the token would then quietly come up empty, indistinguishable from "no owner". Stored, the query holds when the rule changes.
// ADR-0154
struct LiveConn {
    owner: OwnerToken,
    sink: Arc<dyn FrameSink>,
}

impl CommandRoster {
    /// Prefix of the owner token — this format appears only inside the daemon (`CommandRoster::owner_of`).
    pub const OWNER_TOKEN_PREFIX: &'static str = "conn-";

    pub fn new() -> Self {
        Self::default()
    }

    /// The owner token of one connection — derived from the connection id. The only derivation rule; the only caller is [`CommandRoster::attach`] (policy and rationale: `connection_core::ConnectionSession::owner_token`).
    ///
    /// Registration, cleanup and query use the value attach stored, not this function, so the three do not diverge when the derivation changes (ADR-0154); ask the stored value from outside via [`CommandRoster::attached_owner`].
    /// This is today's provisional rule: slice B makes the owner key a client-made identifier carried in the first greeting (ADR-0150 decisions 1 and 2), keeping the connection-id derivation only as the fail-open branch (that decision's impact section). It is not in the code yet.
    // ADR-0150
    pub fn owner_of(conn_id: ConnId) -> OwnerToken {
        OwnerToken::new(format!("{}{conn_id}", Self::OWNER_TOKEN_PREFIX))
    }

    /// The connection is up — from here its registrations are accepted, and envelopes addressed to its owner go out through `frames`.
    ///
    /// Hold a clone of `frames`: the lifetime hook hands a borrowed reference that cannot outlive the connection (`crate::conn_port::ConnectionHandler::on_connect` contract); the table needs a `'static` shared handle.
    /// The path comes before the registry entry: names reach the registry through a registration packet accepted only after this call ([`Shared::refuse_if_detached`]), so a registry name is always reachable; reversed, a "in the roster but unreachable" window opens with no runtime signal (ADR-0154 insertion order).
    ///
    /// Precondition — do not call twice with the same [`ConnId`]: the second overwrites the first's outlet and an envelope for the earlier connection's names goes to the later one. No refusal logic is added (the overwrite stays), but one `error!` is emitted.
    /// Its unreachability depends on two callers' sequences holding together: socket connections come from the network layer's monotonically increasing, never-reused counter (`ConnRegistry::alloc_id` in `link/server.rs`), and the control route's fake caller connection counts down from `u64::MAX`
    /// (`command_delivery::CommandBus::open_caller` — the 2^63-allocation argument is in its doc). If either issuer changes, this precondition breaks.
    /// Do not panic here (do not revive): this runs inside `on_connect`; a death there ends the production process (outside the containment mark — hook · ADR-0292), and in a hook-less test build skips cleanup hooks and registry release, leaving a dead queue as a fan-out target
    /// (the network layer registers the connection before `on_connect` and unregisters it only after `on_disconnect` — `link::server::Server::run_connection`) and an unerased entry so [`CommandRoster::sink_of`] answers `Some` for a dead owner. `debug_assert` is out too: it vanishes in release while only debug produces the accident.
    ///
    /// Derivation happens only once, here — [`CommandRoster::detach`], [`register`](CommandRoster::register) and [`update`](CommandRoster::update) read the stored value, so registry entry, removal and query see the same token if the rule changes (ADR-0154 reopen trigger 2).
    // ADR-0154
    pub fn attach(&self, conn_id: ConnId, frames: &Arc<dyn FrameSink>) {
        // Strong references are dropped outside the lock — this table does not know what a destructor does (header rationale).
        let replaced = {
            let mut shared = self.lock();
            shared.live.insert(
                conn_id,
                LiveConn {
                    owner: Self::owner_of(conn_id),
                    sink: frames.clone(),
                },
            )
        };
        // The overwritten entry itself proves the conflict, so logging after the lock is released still describes the event (same reason as [`CommandRoster::detach`]).
        // Debt: this token is logged unsanitized — today the daemon-made `conn-<id>` has our length and shape, but once slice B stores a client-sent identifier here this logs an unverified peer string verbatim (megabyte lines, control-character forged entries).
        //   The commit that changes the value `attach` stores must fix this line too — the sanitizer is in this crate (`connection_core::sanitize_for_log`, already used below in [`logged_names`]).
        if let Some(previous) = &replaced {
            tracing::error!(
                conn = conn_id,
                previous_owner = %previous.owner,
                "attached twice with the same connection id — overwrote the outlet of the owner attached first (connection ids are not reused)"
            );
        }
        drop(replaced);
    }

    /// The owner token stored when that connection attached. Absent = not attached now (so no name can stand for this connection — [`Shared::refuse_if_detached`]).
    ///
    /// The place to ask who the registry's owner is: re-calling [`CommandRoster::owner_of`] would diverge from the stored value when the derivation changes (ADR-0154).
    /// The answer goes stale the moment the lock is released — do not put it on a decision path: that connection's `detach` can slip in (overlap is real — header). "Attached so the registration will pass" / "attached so the envelope will go out" must not be decided from the value read;
    /// those decisions are atomic with the lock inside [`CommandRoster::register`] and [`CommandRoster::sink_of`]. That is why today's only consumer (`note_claimed_owner` in `connection_core`) is safe: it picks only a log branch, and registration success is decided by [`Shared::refuse_if_detached`] in one critical section.
    // ADR-0154
    pub fn attached_owner(&self, conn_id: ConnId) -> Option<OwnerToken> {
        self.lock()
            .live
            .get(&conn_id)
            .map(|conn| conn.owner.clone())
    }

    /// The frame outlet of the connection the owner token is currently attached to. Absent (`None`) = no currently attached connection holds that token — it cannot tell "never attached" from "dropped mid round trip". When the query answered `Available` and then absence comes out here,
    /// the caller answers `OUTCOME_UNKNOWN` (ADR-0154); do not fold it into owner-absent (false certainty; precedent: the tool crate's `route.rs` `forwarding_unknown` and the correlation-key mismatch branch).
    ///
    /// Only a handle copy goes out and the lock ends here — the caller sends outside the lock (ADR-0006 · header).
    /// The query is an equality comparison with the stored token, not a trace-back of the connection id (rationale: [`LiveConn`]).
    /// Use the received handle at once and drop it — never keep it in a wait ticket or correlation table: the registry cannot take back a handed-out copy, so a kept one makes that connection's send queue outlive the registry (what a surviving copy keeps alive: the `on_disconnect` duty of `conn_port::ConnectionHandler`).
    /// With several connections for one owner, which one comes out is not guaranteed: today [`CommandRoster::owner_of`] is 1:1 so it is unreachable, and it becomes reachable when slice B makes the owner key client-made; the selection rule belongs to that slice, and today's table-iteration order is no promise
    /// (same root as the "known mismatch" of [`CommandRoster::detach`]).
    /// Known cost: it scans the table linearly while locked; connections have no cap (the accept loop has no concurrent limit) and delivery queries once per command, so growth contends with registration and cleanup for the lock. A reverse index is outside this slice.
    // ADR-0154
    pub fn sink_of(&self, owner: &OwnerToken) -> Option<Arc<dyn FrameSink>> {
        self.lock()
            .live
            .values()
            .find(|conn| conn.owner == *owner)
            .map(|conn| conn.sink.clone())
    }

    /// The frame outlet of one connection — used when a delivery sends the reply back (sibling [`CommandRoster::sink_of`] looks up by owner token).
    ///
    /// Not merged with `sink_of`: an outgoing envelope's destination is the owner, the returning reply's is the connection that asked, so the keys differ. Today 1:1 they read the same table; when slice B makes the owner key client-made, one owner has several connections and the reply could go to the wrong one.
    /// The received handle is used at once and dropped (rationale as `sink_of`). Absent = that connection already dropped (nowhere to send the reply, not an error).
    /// Pointing at a connection by [`ConnId`] alone is safe only because the allocator does not reuse the value — there is no generation number, so once reuse appears a lingering correlation-table entry would hand someone else's reply to a new peer with the same number (measurement and remedy: `Pending::origin` in `command_delivery`).
    // ADR-0154
    pub fn sink_for_conn(&self, conn_id: ConnId) -> Option<Arc<dyn FrameSink>> {
        self.lock().live.get(&conn_id).map(|conn| conn.sink.clone())
    }

    /// Is that connection still attached — a query that makes no copy of the outlet.
    ///
    /// A place needing only liveness (checking the caller before side effects) that used [`CommandRoster::sink_for_conn`] would keep a strong reference for its lifetime; in a blocking body of several seconds the dropped connection's writer could not finish itself
    /// (the leak ADR-0154's "do not hold a handed-out copy across a round trip" blocks). `ConnId` is not reused, so the predicate also reads "is it the same peer" (measurement: `Pending::origin` of `command_delivery`).
    // ADR-0154
    pub fn is_attached(&self, conn_id: ConnId) -> bool {
        self.lock().live.contains_key(&conn_id)
    }

    /// Test only — swaps only the stored owner token to deterministically make a torn window (the registry query says `Available` but no path exists); in production it arises only from a `detach` slipping between query and delivery.
    #[cfg(test)]
    pub(crate) fn overwrite_stored_owner(&self, conn_id: ConnId, owner: OwnerToken) {
        self.lock()
            .live
            .get_mut(&conn_id)
            .expect("must be an attached connection")
            .owner = owner;
    }

    /// The connection dropped — erase that owner's names from the registry and withdraw the path to that connection, both inside one lock (a concurrent registration cannot get between).
    ///
    /// Uses the removal order ADR-0154 fixed (registry first, path later): inside one lock nobody sees "in the roster but unreachable", but the order must stay in code so the window does not open if the table is split outside the lock.
    ///
    /// ## Two call sites withdraw different kinds of connection
    ///
    /// 1. Drop of a real connection — only `AgentConnection::on_disconnect`; a second removal point of this kind would split the causality (ADR-0150).
    /// 2. End of a fake caller connection's round trip — the destructor of `command_delivery::CallerSeat` (ADR-0160): the connection one control-route call sets up to receive a reply, which never reaches `on_disconnect` (no socket).
    ///
    /// Do not write 1's invariant back as "one for the whole" — now false, and deleting 2 would look justified by a "single removal point"; without 2 that entry stays for the process lifetime and permanently lengthens the linear scan in [`CommandRoster::sink_of`].
    /// The carve-out holds because the two kinds cannot touch each other's entries: fake connections count down from `u64::MAX`, real ones up from 1 (that issuer's doc), and fake ones register no names, so 2 only removes its own entry from `live`.
    /// Log what was erased — this line is the event's only diagnostic surface: the registry keeps no trace of a dropped owner (ADR-0150 decision 3), so without it nothing answers "which command vanished and why" (the bounce log tells only refused packets). `info!` as a connection-lifetime event
    /// (`docs/reference/logging-conventions.md` level table · "Instrumentation duty"). A drop with no names to remove is `debug!`: nothing erased, the connection's end is logged by the network layer, and registering clients are still zero, so that branch is the usual case in full.
    /// The log is called after the lock is released: the file sink writes synchronously (logging convention "Infrastructure"), so inside the critical section it would stall registration, query and others' cleanup for the IO. `help` is not included (client-sent; cap 32× a name — `Roster::MAX_HELP_BYTES`).
    ///
    /// Known mismatch — the removal unit is the owner token but `Shared::live` is keyed by [`ConnId`]: harmless while [`CommandRoster::owner_of`] is 1:1; if slice B lets several connections share one owner, `detach(conn1)` erases the still-alive `conn2`'s registrations while it keeps passing
    /// `Shared::refuse_if_detached` and does not know its names are gone (registration is once at attach). Slice B must close this; fixing it now means narrowing the removal unit to the connection, but [`ConnId`] is unknown to the tool crate and fixing would pre-decide that slice's owner model
    /// (user decision 2026-08-17 — `docs/process/step-log.md` ㉲).
    ///
    /// The owner to erase is read from the table, not derived again: a re-derivation diverging from the token [`CommandRoster::attach`] stored would let `disconnect` erase nothing and those names stay `Available` toward a dead owner for the daemon's lifetime (ADR-0154). A connection not in the table has nothing to erase — registration stands only by passing [`Shared::refuse_if_detached`].
    // ADR-0150
    // ADR-0154
    pub fn detach(&self, conn_id: ConnId) {
        // Drop the withdrawn entry outside the lock: it may be the last strong reference to the frame outlet (a client registered but subscribed to no agent), and channel closing and waker waking would run inside the critical section — outside the header's "lock, call once, release at once".
        let (removed, reclaimed) = {
            // Do not panic on a lock failure — both callers are cleanup paths (1 and 2 above). 2 is a destructor: a panic during unwinding is an immediate abort, and otherwise the destructor's next line (correlation-table cleanup) would not run and both tables would leak. This function only removes, so continuing on a poisoned table
            //   is also less risky than for the sibling verbs (registration, query), which carry the panic of [`CommandRoster::lock`]. Paired discipline: `command_delivery::CommandDeliveries::lock_for_cleanup`.
            let mut shared = self.lock_for_cleanup();
            let owner = shared.live.get(&conn_id).map(|conn| conn.owner.clone());
            let removed = match owner {
                Some(owner) => shared.roster.disconnect(&owner),
                None => Vec::new(),
            };
            let reclaimed = shared.live.remove(&conn_id);
            (removed, reclaimed)
        };
        drop(reclaimed);
        if removed.is_empty() {
            tracing::debug!(
                conn = conn_id,
                "connection dropped — no names to take down from the roster"
            );
        } else {
            tracing::info!(
                conn = conn_id,
                names = removed.len(),
                removed = %logged_names(&removed),
                "connection dropped — removed this owner's names from the command roster"
            );
        }
    }

    /// Full registration at attach time. A dropped connection's late packet is bounced with `CONFLICT`. The owner a name stands for is the token [`CommandRoster::attach`] stored — not derived again (ADR-0154).
    pub fn register(&self, conn_id: ConnId, decls: Vec<CommandDecl>) -> Result<(), CommandError> {
        let mut shared = self.lock();
        let owner = shared.refuse_if_detached(conn_id)?;
        shared.roster.register(&owner, decls)
    }

    /// Diff while attached. Bounce rule and owner-token source as [`CommandRoster::register`].
    pub fn update(
        &self,
        conn_id: ConnId,
        added: Vec<CommandDecl>,
        removed: Vec<String>,
    ) -> Result<(), CommandError> {
        let mut shared = self.lock();
        let owner = shared.refuse_if_detached(conn_id)?;
        shared.roster.update(&owner, added, removed)
    }

    /// Snapshot of the whole registry — drained here, since handing out [`Roster::entries`]'s iterator would hold the lock while the caller iterates.
    pub fn entries(&self) -> Vec<RosterEntry> {
        self.lock().roster.entries().collect()
    }

    /// A copy of the agent-reference slots (registration order) that this name's owner marked — delivery reads it before relaying ([`Roster::agent_args`]). The lock ends inside this call.
    // ADR-0308
    pub fn agent_args_of(&self, name: &str) -> Vec<String> {
        self.lock().roster.agent_args(name).to_vec()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.inner.lock().expect("command roster poisoned")
    }

    /// Lock acquisition that does not panic on failure — for the cleanup path only (sole caller [`CommandRoster::detach`]; reason in its inner comment).
    ///
    /// A production process cannot reach this branch (known scope): a panic holding this table is outside the containment mark so the hook ends the process before unwinding and nothing is poisoned (ADR-0292). It stays because in a hook-less test build the branch is real and a leaking table ruins regression observation.
    fn lock_for_cleanup(&self) -> std::sync::MutexGuard<'_, Shared> {
        sync::lock(&self.inner)
    }
}

/// The one thing stage 2 of the 3-stage delivery asks the registry (`engram_dashboard_command::route`).
///
/// The lock ends inside this call — the reason this trait exists: handed the reference whole, the registry would stay locked while delivery waits for a reply, stalling registration, cleanup and other deliveries (rationale: that trait's comment).
// ADR-0154
impl OwnerLookupSource for CommandRoster {
    fn lookup(&self, name: &str) -> OwnerLookup {
        self.lock().roster.lookup(name)
    }
}

impl Shared {
    /// The only net stopping late packets from a dropped connection — rationale and "do not build a second one" are in this file's header.
    ///
    /// On pass it returns the owner token stored at attach, so the caller does not derive it again (single derivation point [`CommandRoster::attach`] — ADR-0154); the refusal branch has no token, which is this net's meaning.
    ///
    /// Code `CONFLICT`: the packet is fine (not `INVALID_ARGUMENT`) and not the caller's fault — the state is refused (that connection is not on the list). Its retry instruction is `never`: resending over a gone connection is pointless, and re-attaching gets a new token, so it is re-registration, not retry.
    /// The wording is not narrowed to "dropped": the branch also stands for a connection that never attached — not on the production path today (`on_connect` precedes dispatch) but the type does not prevent it (`register`/`update` take a bare [`ConnId`]).
    /// No separate log: the caller (`reply_roster` in `connection_core`) already leaves every registry refusal as a `warn!` (conn · verb · code · wording); another line would duplicate it and be emitted while holding the lock (as [`CommandRoster::detach`] moved its log out).
    /// Known mismatch — this decision's unit is [`ConnId`] but the removal unit is the owner token: harmless while 1:1, but if slice B makes several connections share an owner, connections whose registrations were erased by someone else's `detach` still pass this net, not knowing their names are gone.
    /// Rationale and "slice B closes it": the [`CommandRoster::detach`] comment.
    // ADR-0150
    fn refuse_if_detached(&self, conn_id: ConnId) -> Result<OwnerToken, CommandError> {
        match self.live.get(&conn_id) {
            Some(conn) => Ok(conn.owner.clone()),
            None => Err(CommandError::of(ErrorCode::Conflict, DETACHED_REFUSAL)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conn_port::{Frame, FrameError};
    use crate::test_doubles::FakeFrameSink;
    use engram_dashboard_command::OwnerLookup;
    use futures_util::future::BoxFuture;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::mpsc;

    fn decl(name: &str) -> CommandDecl {
        CommandDecl {
            name: name.to_string(),
            help: "{}".to_string(),
            agent_args: Vec::new(),
        }
    }

    /// One outlet to go in the table and where it reaches. Handles are not compared by `Arc` pointer because the table guards "sending through that handle goes to that connection", not "the same object came out".
    fn sink_with_inbox() -> (Arc<dyn FrameSink>, mpsc::Receiver<Frame>) {
        let (tx, rx) = mpsc::channel::<Frame>(4);
        (Arc::new(FakeFrameSink::new(tx)), rx)
    }

    /// A placeholder for tests that do not look at the path.
    fn some_sink() -> Arc<dyn FrameSink> {
        sink_with_inbox().0
    }

    /// An outlet that records whether the registry lock was released when it was destroyed. It drops frames — it watches when it itself dies, not delivery.
    struct LockProbeSink {
        roster: CommandRoster,
        unlocked_at_drop: Arc<AtomicBool>,
    }

    impl FrameSink for LockProbeSink {
        fn try_send(&self, _frame: Frame) -> Result<(), FrameError> {
            Ok(())
        }

        fn send(&self, _frame: Frame) -> BoxFuture<'_, Result<(), FrameError>> {
            Box::pin(async { Ok(()) })
        }
    }

    impl Drop for LockProbeSink {
        fn drop(&mut self) {
            let unlocked = self.roster.inner.try_lock().is_ok();
            self.unlocked_at_drop.store(unlocked, Ordering::Release);
        }
    }

    /// Marks the handle so we can tell which receiver it went out to.
    fn ping(sink: &Arc<dyn FrameSink>, mark: &str) {
        sink.try_send(Frame::Text(mark.to_string()))
            .expect("the fake outlet is empty");
    }

    fn delivered(rx: &mut mpsc::Receiver<Frame>) -> Option<String> {
        match rx.try_recv() {
            Ok(Frame::Text(text)) => Some(text),
            Ok(other) => panic!("must be Text: {other:?}"),
            Err(_) => None,
        }
    }

    /// Asks the registry directly — what delivery sees is this answer (not the projection of `entries`).
    fn lookup(roster: &CommandRoster, name: &str) -> OwnerLookup {
        let shared = roster.lock();
        shared.roster.lookup(name)
    }

    // ── Registration running concurrently with connection cleanup (the remaining race of the conn_port::ConnectionHandler contract) ──────
    //
    // Four branches: {registration, diff} × {a name nobody holds, a name a live connection holds}. The registry keeps no trace of a dropped owner (ADR-0150), so this net stands only on connection lifetime.
    // Assert the wording too, not only the code: `Roster::check_added_are_not_taken` raises the same `CONFLICT`, so a code-only assertion would stay green for another reason with this file's net deleted.

    /// Also check whether the name comes back to life — a late registration putting back a name cleanup erased makes it `Available` toward a nonexistent owner for the daemon's lifetime.
    #[test]
    fn a_registration_that_lands_after_its_disconnect_is_refused() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());
        roster
            .register(1, vec![decl("tab.create")])
            .expect("register");
        roster.detach(1);

        let err = roster
            .register(1, vec![decl("tab.create")])
            .expect_err("a registration that landed after cleanup");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(
            err.message(),
            DETACHED_REFUSAL,
            "must be what the connection-lifetime net caught"
        );
        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Unknown,
            "a name the cleanup removed must not come back"
        );
        assert!(roster.entries().is_empty(), "nothing remains in the roster");
    }

    /// A branch the registry alone cannot stop — a dropped owner's registrations are erased, so `Roster` cannot remember that death and a late registration passes as a handover of a live connection's names; connection lifetime stops it here.
    #[test]
    fn a_late_registration_cannot_take_a_live_connections_name() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());
        roster
            .register(1, vec![decl("tab.create")])
            .expect("register");
        roster.detach(1);
        roster.attach(2, &some_sink());
        roster
            .register(2, vec![decl("tab.create")])
            .expect("the live connection takes over the name");

        let err = roster
            .register(1, vec![decl("tab.create")])
            .expect_err("a late registration from a dead connection");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(
            err.message(),
            DETACHED_REFUSAL,
            "must be what the connection-lifetime net caught"
        );
        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Available(CommandRoster::owner_of(2)),
            "the live connection stays the owner"
        );
    }

    #[test]
    fn a_delta_that_lands_after_its_disconnect_is_refused() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());
        roster
            .register(1, vec![decl("tab.create")])
            .expect("register");
        roster.detach(1);

        let err = roster
            .update(1, vec![decl("tab.split")], vec![])
            .expect_err("a diff that landed after cleanup");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(
            err.message(),
            DETACHED_REFUSAL,
            "must be what the connection-lifetime net caught"
        );
        assert_eq!(lookup(&roster, "tab.split"), OwnerLookup::Unknown);
    }

    #[test]
    fn a_late_delta_cannot_take_a_live_connections_name() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());
        roster
            .register(1, vec![decl("tab.create")])
            .expect("register");
        roster.detach(1);
        roster.attach(2, &some_sink());
        roster
            .register(2, vec![decl("tab.create")])
            .expect("the live connection takes over the name");

        let err = roster
            .update(1, vec![decl("tab.create")], vec![])
            .expect_err("a late diff from a dead connection");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(
            err.message(),
            DETACHED_REFUSAL,
            "must be what the connection-lifetime net caught"
        );
        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Available(CommandRoster::owner_of(2))
        );
    }

    /// A spot the registry check bypasses entirely — connection lifetime is the only net: a diff adding a name nobody holds passes `Roster::check_added_are_not_taken`, which has no check telling a dropped owner apart (ADR-0150). Without the net `tab.split` becomes `Available` toward a nonexistent owner for the daemon's lifetime.
    #[test]
    fn a_late_delta_adding_an_unclaimed_name_is_refused() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());
        roster
            .register(1, vec![decl("tab.create")])
            .expect("register");
        roster.detach(1);
        roster.attach(2, &some_sink());
        roster
            .register(2, vec![decl("tab.create")])
            .expect("the live connection newly puts up the same name");

        let err = roster
            .update(1, vec![decl("tab.split")], vec![])
            .expect_err("a late diff from a dead connection");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(
            err.message(),
            DETACHED_REFUSAL,
            "must be what the connection-lifetime net caught"
        );
        assert_eq!(
            lookup(&roster, "tab.split"),
            OwnerLookup::Unknown,
            "a new name must not stand addressed to a dead connection"
        );
    }

    /// The log must say what the disconnect erased.
    ///
    /// No trace is kept (ADR-0150 decision 3), so this line is the only surface answering "which command vanished and why" (the bounce log in `reply_roster` tells only refused packets).
    /// It checks the level and fields, not the wording: `info!` as a connection-lifetime event (`docs/reference/logging-conventions.md` level table), and names are carried since the count cannot say which command vanished.
    #[test]
    fn a_detach_logs_the_names_it_removed() {
        let roster = CommandRoster::new();
        roster.attach(7, &some_sink());
        roster
            .register(7, vec![decl("tab.create"), decl("tab.close")])
            .expect("register");

        let logged = capture_info(|| roster.detach(7));

        assert!(logged.contains("conn=7"), "which connection: {logged:?}");
        assert!(
            logged.contains("tab.create") && logged.contains("tab.close"),
            "the removed names must be carried: {logged:?}"
        );
    }

    /// The peer cannot decide this line's shape — one newline would split the log line and forge an entry.
    ///
    /// The registry checks only a name's byte length, not its character set (the tool crate `Roster` registration check), so a newline-bearing name really registers — the test's premise.
    /// Put up only one name; do not merge this axis with the count axis: when merged, once the fill exceeded [`MAX_LOGGED_NAMES`] the forged name fell on the truncated side of the name-order sort and stayed green without the sanitizer. The count axis is the sibling below.
    #[test]
    fn a_control_character_in_a_removed_name_never_reaches_the_log_field_verbatim() {
        let roster = CommandRoster::new();
        roster.attach(9, &some_sink());
        let forged = "tab.forged\n2026-08-19T00:00:00Z  WARN forged entry";
        roster.register(9, vec![decl(forged)]).expect("register");

        let logged = capture_info(|| roster.detach(9));

        assert!(
            logged.contains("tab.forged"),
            "premise: that name was carried on this line: {logged:?}"
        );
        assert!(
            !logged.contains('\n'),
            "a newline was carried raw — the log line splits: {logged:?}"
        );
    }

    /// The peer cannot decide this line's size — one owner can put up 512 names (`Roster::MAX_NAMES_PER_OWNER`), 64 KiB untruncated.
    ///
    /// `names` is not truncated (the exact count must remain, so "how many vanished" does not blur); only the tail of "which names" is cut, and the line says so.
    #[test]
    fn the_removed_names_line_says_how_many_it_elided() {
        let roster = CommandRoster::new();
        roster.attach(9, &some_sink());
        let total = MAX_LOGGED_NAMES + 3;
        let decls: Vec<CommandDecl> = (0..total).map(|i| decl(&format!("tab.n{i:02}"))).collect();
        roster.register(9, decls).expect("register");

        let logged = capture_info(|| roster.detach(9));

        assert!(
            logged.contains(&format!("names={total}")),
            "the count must be exact: {logged:?}"
        );
        assert!(
            logged.contains(&format!("(+{} more)", total - MAX_LOGGED_NAMES)),
            "it must say it cut: {logged:?}"
        );
        assert!(
            !logged.contains(&format!("tab.n{:02}", total - 1)),
            "the last name must be cut off: {logged:?}"
        );
        // With a generous cap, pin only "the peer cannot decide the size" — an exact number would break with every added field.
        assert!(
            logged.len() < 1024,
            "the peer decides the line size: {} bytes",
            logged.len()
        );
    }

    /// A disconnect with no names to remove does not emit `info!`: nothing erased, the connection's end is logged by the network layer, and registering clients are still zero (TRD §3-7), so this branch is the usual case in full and would bury the default path under pointless lines.
    #[test]
    fn a_detach_with_nothing_to_remove_stays_quiet_at_info() {
        let roster = CommandRoster::new();
        roster.attach(7, &some_sink());

        let logged = capture_info(|| roster.detach(7));

        assert!(
            logged.is_empty(),
            "with nothing removed, info must be empty: {logged:?}"
        );
    }

    /// Cleanup of a connection that never attached — no table entry, so no owner to read and no name to erase (this branch arose when `detach` began to read the owner token from the table). It must be quiet and above all must not touch a live connection's names.
    #[test]
    fn a_detach_for_a_connection_that_never_attached_is_quiet() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());
        roster
            .register(1, vec![decl("tab.create")])
            .expect("register");

        let logged = capture_info(|| roster.detach(2));

        assert!(
            logged.is_empty(),
            "with nothing to remove, info must be empty: {logged:?}"
        );
        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Available(CommandRoster::owner_of(1)),
            "another's cleanup must not touch the live connection's names"
        );
    }

    /// A minimal collector gathering only the fields of INFO events — this test looks only at "did an INFO event carrying that field occur" (same form as `control::tests`). `with_default` is installed only on this thread, so it does not mix with parallel tests.
    fn capture_info(body: impl FnOnce()) -> String {
        use tracing::subscriber;

        struct InfoCollector {
            lines: Arc<Mutex<Vec<String>>>,
        }
        struct Visit<'a>(&'a mut String);
        impl tracing::field::Visit for Visit<'_> {
            fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
                self.0.push_str(&format!("{}={:?} ", f.name(), v));
            }
        }
        impl subscriber::Subscriber for InfoCollector {
            fn enabled(&self, m: &tracing::Metadata<'_>) -> bool {
                *m.level() == tracing::Level::INFO
            }
            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::Id {
                tracing::Id::from_u64(1)
            }
            fn record(&self, _: &tracing::Id, _: &tracing::span::Record<'_>) {}
            fn record_follows_from(&self, _: &tracing::Id, _: &tracing::Id) {}
            fn event(&self, event: &tracing::Event<'_>) {
                let mut buf = String::new();
                event.record(&mut Visit(&mut buf));
                self.lines.lock().expect("lines poisoned").push(buf);
            }
            fn enter(&self, _: &tracing::Id) {}
            fn exit(&self, _: &tracing::Id) {}
        }

        let lines: Arc<Mutex<Vec<String>>> = Arc::default();
        subscriber::with_default(
            InfoCollector {
                lines: lines.clone(),
            },
            body,
        );
        let captured = lines.lock().expect("lines poisoned");
        captured.join("\n")
    }

    /// A reconnect comes with a new connection id so it is not blocked — the bounce above does not lock the normal path.
    #[test]
    fn reconnecting_under_a_new_connection_id_registers_normally() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());
        roster
            .register(1, vec![decl("tab.create")])
            .expect("register");
        roster.detach(1);

        roster.attach(2, &some_sink());
        roster
            .register(2, vec![decl("tab.create")])
            .expect("a reconnect comes with a new token");

        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Available(CommandRoster::owner_of(2))
        );
    }

    // ── Owner token → the path to that owner (ADR-0154) ──────────────────────────────────
    //
    // Measured in delivery's order: ask the registry by name for the owner (`lookup`), then the outlet with that owner (`sink_of`) — so the lookup key is a value the registry really holds, not a hand-made string.

    #[test]
    fn an_attached_owner_resolves_to_that_connections_sink() {
        let roster = CommandRoster::new();
        let (sink, mut inbox) = sink_with_inbox();
        roster.attach(1, &sink);
        roster
            .register(1, vec![decl("tab.create")])
            .expect("register");

        let owner = match lookup(&roster, "tab.create") {
            OwnerLookup::Available(owner) => owner,
            other => panic!("there must be an owner: {other:?}"),
        };
        let found = roster
            .sink_of(&owner)
            .expect("an attached owner has a path to reach");
        ping(&found, "envelope");

        assert_eq!(
            delivered(&mut inbox),
            Some("envelope".to_string()),
            "it must go out through that connection's outlet"
        );
    }

    /// A dropped owner has no path to reach — and merely not being visible is not enough; it must be released.
    ///
    /// Measures the strong reference really disappeared, not only absence in the query: an implementation keeping a tombstone holding the `Arc` passes the preceding assertion. It would keep the channel allocation and `Notify` for the daemon's lifetime and — worse downstream —
    /// the surviving entry makes [`CommandRoster::sink_of`] answer `Some` for a dead owner, a silent black hole instead of an honest "no owner".
    /// (Unrelated to the writer task's self-termination: the network layer `abort()`s the losing task before `on_disconnect`, so the registry's copy cannot make the writer a zombie.)
    #[test]
    fn a_detached_owner_releases_its_sink() {
        let roster = CommandRoster::new();
        let (sink, _inbox) = sink_with_inbox();
        let reclaimed = Arc::downgrade(&sink);
        roster.attach(1, &sink);
        drop(sink); // Now the table is the only strong reference.

        roster.detach(1);

        assert!(roster.sink_of(&CommandRoster::owner_of(1)).is_none());
        assert!(
            reclaimed.upgrade().is_none(),
            "the table must really release the handle — merely hiding it is a tombstone"
        );
        assert!(
            !roster.lock().live.contains_key(&1),
            "the entry itself must disappear — leaving a tombstone with only the handle emptied makes the table grow without bound as connections come and go"
        );
    }

    /// The withdrawn handle's destructor runs outside the critical section — moving `detach`'s removal back into the lock block would run it inside; the registry does not know what that `dyn` code does, which breaks the header's "lock, call once, release at once". Nothing else goes red on such a revert, so this is the only net.
    ///
    /// Measured by the destructor trying to take the registry lock: if the same thread holds it, `try_lock` returns `WouldBlock`, telling the two cases apart without deadlock.
    #[test]
    fn the_reclaimed_sink_is_dropped_outside_the_lock() {
        let roster = CommandRoster::new();
        let unlocked_at_drop = Arc::new(AtomicBool::new(false));
        let sink: Arc<dyn FrameSink> = Arc::new(LockProbeSink {
            roster: roster.clone(),
            unlocked_at_drop: unlocked_at_drop.clone(),
        });
        roster.attach(1, &sink);
        drop(sink); // The table is the only strong reference — `detach` runs the destructor.

        roster.detach(1);

        assert!(
            unlocked_at_drop.load(Ordering::Acquire),
            "the destructor ran while holding the roster lock"
        );
    }

    /// Registration, query and removal all use the stored token even when it is not of the derived form.
    ///
    /// An implementation re-deriving with [`CommandRoster::owner_of`] in any of the three goes red — a re-deriving `register` puts the name up under `conn-1`, a re-deriving `detach` erases nothing.
    /// This does not pre-decide slice B: what changes is one table value, not the signature of [`CommandRoster::attach`]; production still computes the token itself.
    #[test]
    fn the_roster_files_and_removes_under_the_stored_token() {
        let roster = CommandRoster::new();
        let (sink, _inbox) = sink_with_inbox();
        roster.attach(1, &sink);
        let stored = OwnerToken::new("shell-a1");
        roster.lock().live.get_mut(&1).expect("attached").owner = stored.clone();

        roster
            .register(1, vec![decl("tab.create")])
            .expect("register");

        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Available(stored.clone()),
            "the roster stands names addressed to the stored token"
        );
        assert!(
            roster.sink_of(&stored).is_some(),
            "lookups reach through the stored token too"
        );
        assert!(
            roster.sink_of(&CommandRoster::owner_of(1)).is_none(),
            "a derived form does not reach"
        );
        assert_eq!(roster.attached_owner(1), Some(stored));

        // Wrap with `capture_info` — this `detach` has names to erase so it emits `info!`: when a thread with no subscriber registers that callsite first, "no interest" is baked in and `a_detach_logs_the_names_it_removed` comes back empty (broke in a 4-thread run, 2026-08-18).
        //   Do not write "it is a global cache so it sticks once hit" — wrong; the actual condition (first registration across threads) and the evidence are in the `log_capture` module header, the authority.
        capture_info(|| roster.detach(1));

        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Unknown,
            "removal is done with the stored token too — rederiving would leave this name addressed to a dead owner"
        );
    }

    #[test]
    fn an_owner_that_never_attached_has_no_sink() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());

        assert!(
            roster.sink_of(&CommandRoster::owner_of(2)).is_none(),
            "a connection that never attached"
        );
        assert!(
            roster.sink_of(&OwnerToken::new("shell")).is_none(),
            "a token of a shape this daemon never issued"
        );
    }

    /// When several are attached, each gets its own outlet — mixed, one client's command would be delivered to another (executing something it did not order).
    #[test]
    fn each_owner_gets_its_own_sink() {
        let roster = CommandRoster::new();
        let (first, mut first_inbox) = sink_with_inbox();
        let (second, mut second_inbox) = sink_with_inbox();
        roster.attach(1, &first);
        roster.attach(2, &second);

        ping(
            &roster
                .sink_of(&CommandRoster::owner_of(1))
                .expect("owner 1"),
            "to-1",
        );
        ping(
            &roster
                .sink_of(&CommandRoster::owner_of(2))
                .expect("owner 2"),
            "to-2",
        );

        assert_eq!(delivered(&mut first_inbox), Some("to-1".to_string()));
        assert_eq!(delivered(&mut second_inbox), Some("to-2".to_string()));
        assert_eq!(
            delivered(&mut first_inbox),
            None,
            "someone else's got mixed into 1"
        );
        assert_eq!(
            delivered(&mut second_inbox),
            None,
            "someone else's got mixed into 2"
        );
    }

    /// Bites on the query not parsing the token.
    ///
    /// Each string below becomes 1 when `"conn-"` is stripped and the rest read as a number (`"01"` and `"+1"` pass Rust's integer parser): a query tracing the connection id back hands conn 1's outlet to these, while equality with the stored token comes up empty. Why tracing back is forbidden (provisional derivation): [`LiveConn`].
    /// One variant it cannot bite: parse then compare back with [`CommandRoster::owner_of`] also comes up empty for these strings and passes; it diverges only when the stored token is not of the derived form, caught by [`the_roster_files_and_removes_under_the_stored_token`] (which changes the table value directly).
    #[test]
    fn a_lookup_compares_the_stored_token_and_never_parses_it() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());

        for impostor in ["conn-01", "conn-+1"] {
            assert!(
                roster.sink_of(&OwnerToken::new(impostor)).is_none(),
                "a string different from the stored token: {impostor}"
            );
        }
        assert!(
            roster.sink_of(&CommandRoster::owner_of(1)).is_some(),
            "that stored token still reaches"
        );
    }
}
