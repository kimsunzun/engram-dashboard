//! Shared handle to the command owner registry — **every connection sees the same one copy** (ADR-0155/0156 · TRD §3-7).
//!
//! The registry's rules (full registration last-wins · diff · removal on disconnect · cap) are all
//! owned by the tool crate's [`Roster`]. What this file adds is two things: **connection lifetime and the path to
//! the owner** — the registry knows only names and their owners; it knows neither which connections are still attached
//! nor how to send to an owner, and the daemon is the side that knows both (ADR-0154).
//!
//! ★The net that stops late packets from a dropped connection is **only here**★ (`Shared::refuse_if_detached`):
//! [`Roster`] erases a dropped owner's registrations (ADR-0150 decision 3), so it has no place to remember that owner's death, and
//! it cannot tell a late registration or diff from a real handover. If this net is removed, a dead connection's packets pass through and the name
//! points at a **nonexistent owner** and answers `Available` for the daemon's whole lifetime (no cleanup will ever come again for that connection).
//! Do not build the same net again on the [`Roster`] side — keeping per-owner state separately makes that list grow without bound,
//! exactly like the trace (the same reason ADR-0150 dropped the trace).
//!
//! ★The liveness check and the registry change happen inside **the same critical section**★: if split, cleanup can slip in
//! between the check and the change, and the registration of an already-dropped connection lands afterwards. `on_disconnect` is not a quiet
//! moment but a moment that **can overlap with `on_text`** (the `frame_port::ConnectionHandler` contract —
//! the network layer only calls abort and does not wait for completion), so that overlap is not theoretical.
//!
//! ★Do not confuse this with the other things that share the name★: this crate's `RosterBroadcast`, `RosterChanged`, `RosterDiff`
//! are the notification port for the **agent/profile list** (ADR-0132). The registry here maps **command name → owner**, and the two are
//! unrelated.
//!
//! ★The lock is taken and released only inside this file★ (ADR-0006): the methods below lock, then either call [`Roster`] once
//! or take a handle copy from the table and release at once — **the caller does the sending outside the lock**. If the guard
//! is passed outside, one delivery waiting for a reply stalls registration, connection cleanup and query
//! all at once (the authoritative rationale = the [`engram_dashboard_command::OwnerLookupSource`] comment).
//! ★Known cost★: [`CommandRoster::entries`] clones the whole registry while locked — for a registry filled to the cap
//! that span is not short. Reducing it would mean changing the shape [`Roster`] lends out, which is outside this slice.
//! [`CommandRoster::sink_of`] also scans the table linearly while locked — the number of connections has no cap and it is a query
//! run once per delivery (details in the comment at that spot).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::conn_port::{ConnId, FrameSink};
use engram_dashboard_base::sync;
use engram_dashboard_command::{
    CommandDecl, CommandError, ErrorCode, OwnerLookup, OwnerLookupSource, OwnerToken, Roster,
    RosterEntry,
};

use crate::connection_core::sanitize_for_log;

/// How **many names** to write on one disconnect line.
///
/// ★Without a cap, the peer decides the size of one log line★: one owner can put up 512 names
/// ([`Roster::MAX_NAMES_PER_OWNER`]) and each can be up to 128 B (`Roster::MAX_NAME_BYTES`), so without truncation this one
/// line becomes 64 KiB — and all of those values are strings the client chose.
/// ★What truncation loses★: **the count is not lost** (the `names` field carries the exact number separately). What is lost is the tail of
/// 「어느 이름이었나」 (which names were they), and what diagnosis uses is the first few.
const MAX_LOGGED_NAMES: usize = 8;

/// Turn the names a client registered into one log field — **the shape is bounded by the sanitizer, the count by [`MAX_LOGGED_NAMES`]**.
///
/// ★The same value coming in through the same door is not written in two shapes★: the registration bounce already chose the same form
/// (`connection_core::ConnectionCore::refuse_names_i_answer` — joins `'이름'` ('name') with commas). If one value has a different shape
/// in each place, whoever scans the log cannot see the same thing as the same thing.
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

/// The wording used when bouncing the registration or diff of a connection that is not attached.
///
/// ★It is a constant because of tests★: the same `CONFLICT` is also raised by [`Roster`]'s check on other owners' names, so if the wording is not
/// looked at, 「연결 수명 그물이 잡았다」 (the connection-lifetime net caught it) and 「명부 검사가 잡았다」 (the registry check caught it) cannot be told apart — then there would be a test
/// that stays green even if this file's net is deleted entirely.
pub(crate) const DETACHED_REFUSAL: &str =
    "this connection is not attached — a registration lands only while its connection is up";

#[derive(Clone, Default)]
pub struct CommandRoster {
    inner: Arc<Mutex<Shared>>,
}

#[derive(Default)]
struct Shared {
    roster: Roster,
    /// The connections attached now and the path to each of them. It must sit **under the same lock as the registry** so no gap opens
    /// between the check and the change.
    // ADR-0154
    live: BTreeMap<ConnId, LiveConn>,
}

/// One attached connection — **whose it is** and **how to reach it**.
///
/// ★The owner token is computed when attaching and **stored** — it is not traced back at query time★: today's owner token is
/// derived from the connection id by [`CommandRoster::owner_of`], but that derivation is a provisional rule and slice B
/// replaces it with a client-made identifier (ADR-0150). A query that traces the connection id back from the token string would quietly come up
/// empty-handed that day, and that empty result is indistinguishable from 「주인이 없다」 (there is no owner). If it is stored, the
/// query still holds even when the derivation rule changes.
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

    /// The owner token of one connection — **derived from the connection id**. This is the only derivation rule, and the only caller is
    /// [`CommandRoster::attach`] (policy and its rationale in
    /// `connection_core::ConnectionSession::owner_token`).
    ///
    /// ★Registration, cleanup and query look at the value attach **stored**, not this function★ — so the three do not
    /// diverge the day this derivation changes (ADR-0154). To ask for that stored value from outside, use [`CommandRoster::attached_owner`].
    ///
    /// ★This derivation is **today's provisional rule** and slice B replaces it★: the owner key becomes **an identifier the client made and carried
    /// in its first greeting** (ADR-0150 decisions 1 and 2), and the connection-id derivation remains only as the fail-open
    /// branch for connections that did not send it (the impact section of that decision). That identifier is not in the code yet — so for now this derivation is everything.
    // ADR-0150
    pub fn owner_of(conn_id: ConnId) -> OwnerToken {
        OwnerToken::new(format!("{}{conn_id}", Self::OWNER_TOKEN_PREFIX))
    }

    /// The connection is up — from here its registrations are accepted, and envelopes addressed to its owner go out through `frames`.
    ///
    /// ★Hold a clone of `frames`★: what the connection-lifetime hook hands over is a **borrowed reference**, so as it stands it cannot outlive
    /// the connection (the `crate::conn_port::ConnectionHandler::on_connect` contract). To put it in the table
    /// a `'static` shared handle is needed, and that clone is this one line.
    ///
    /// ★The path comes **before** the registry entry★: names get onto the registry through a registration packet, and that packet is
    /// accepted only after this call ([`Shared::refuse_if_detached`]), so a name on the registry is always reachable. If the order
    /// is reversed, a name that is 「명부엔 있는데 닿을 길이 없는」 (in the registry but has no path to reach) stands, and that window gives no signal at runtime
    /// (ADR-0154 insertion order).
    ///
    /// ★Precondition — do not call twice with the same [`ConnId`]★: the second overwrites the first's outlet, so an envelope for a name
    /// registered by the earlier connection goes to the later connection. So **no refusal logic is added** and the overwrite is left as is, but
    /// one `error!` line is emitted.
    /// ★That unreachability depends on **two callers**, so both sequences must hold together★ — writing down only one is half a rationale:
    /// socket connections come from the network layer's monotonically increasing counter and are not reused (`ConnRegistry::alloc_id`
    /// in `ws.rs`), and the control route's fake caller connection **counts down** from `u64::MAX`
    /// (`command_delivery::CommandBus::open_caller` — the calculation that the two sequences would meet only after 2^63 allocations is
    /// in that function's doc). If the nature of either of the two issuers changes, the precondition here breaks with it.
    /// ★Do not panic here — do not revive it★: this function runs inside `on_connect`. If it dies there, the production
    /// process ends outright (outside the containment mark — hook · ADR-0292), and in a hook-less test build
    /// cleanup hooks and registry release are both skipped entirely, a dead queue stays as a fan-out target (the authoritative reason =
    /// the same prohibition in the network layer's `ws.rs`), and an entry that is never erased stays in the table so [`CommandRoster::sink_of`]
    /// answers **`Some` for a dead owner**. `debug_assert` is also out for the same reason — it vanishes in release, so
    /// it gives no observation, while only debug produces that accident.
    ///
    /// ★The derivation happens only once, here★ — [`CommandRoster::detach`], [`CommandRoster::register`] and
    /// [`CommandRoster::update`] below do not call [`CommandRoster::owner_of`] again; they
    /// read the value stored here. That way, the day the derivation rule changes, registry entry, removal and query see **the same token** (ADR-0154 reopen
    /// trigger ②).
    // ADR-0154
    pub fn attach(&self, conn_id: ConnId, frames: &Arc<dyn FrameSink>) {
        // ★Strong references are dropped **outside** the lock★ — this table does not know what a destructor does (rationale = this file's header).
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
        // The overwritten entry is itself the proof of the conflict — so even when logged after the lock is released, it still describes the event as it was (the reason for moving the log
        //   out of the critical section is the same as in [`CommandRoster::detach`]).
        // ★Debt to repay — this token is logged **unsanitized**★: today it is the daemon-made `conn-<id>`, so its length and
        //   shape are ours. The moment slice B stores a **client-sent identifier** here, this line becomes
        //   a spot that logs an unverified peer string verbatim — megabyte-sized log lines and control-character
        //   forged entries open up then. ★The commit that changes the value `attach` stores must fix this line too★ —
        //   the sanitizer is in the same crate (`connection_core::sanitize_for_log` — this file already uses it below in
        //   [`logged_names`]) so all that is left to do then is to call that function.
        if let Some(previous) = &replaced {
            tracing::error!(
                conn = conn_id,
                previous_owner = %previous.owner,
                "같은 연결 id 로 두 번 붙었다 — 먼저 붙은 주인의 출구를 덮었다(연결 id 는 재사용되지 않는다)"
            );
        }
        drop(replaced);
    }

    /// The owner token stored when that connection attached. **Absent = not attached now** (so no
    /// name can stand for this connection — [`Shared::refuse_if_detached`]).
    ///
    /// ★This is the place to ask **who the registry's owner is**★ — if [`CommandRoster::owner_of`] is called again from outside,
    /// the day that derivation changes the value asked for and the stored value diverge (ADR-0154).
    ///
    /// ★The answer **goes stale the moment the lock is released** — do not put it on a decision path★: that connection's
    /// `detach` can slip in between this call and the next (the basis that the overlap is real is this file's header). So, as with 「붙어 있으니 등록이 통과할
    /// 것」 (it is attached so the registration will pass) and 「붙어 있으니 봉투가 나갈 것」 (it is attached so the envelope will go out), you must not **decide what follows from the value read** — those decisions
    /// happen atomically with the lock inside [`CommandRoster::register`] and [`CommandRoster::sink_of`].
    /// That is also why today's only consumer (`note_claimed_owner` in `connection_core`) is safe: what it picks is only
    /// the **log branch**, and whether a registration succeeds is still decided by [`Shared::refuse_if_detached`] inside one critical
    /// section.
    // ADR-0154
    pub fn attached_owner(&self, conn_id: ConnId) -> Option<OwnerToken> {
        self.lock()
            .live
            .get(&conn_id)
            .map(|conn| conn.owner.clone())
    }

    /// The frame outlet of the connection the owner token is currently attached to. **Absent (`None`) = no currently attached connection holds that
    /// token** — that is all this function knows, so it **cannot tell** 「한 번도 붙은 적 없다」 (never attached) from 「왕복 도중
    /// 끊겼다」 (dropped mid round trip). In the race where the query answered `Available` and then the absence comes out here,
    /// the caller answers `OUTCOME_UNKNOWN` (ADR-0154). ★Do not fold it into owner-absent — that is false
    /// certainty★ (a precedent of the same kind = the tool crate's `route.rs` `forwarding_unknown` and the correlation-key mismatch branch).
    ///
    /// ★Only a handle copy goes out and the lock ends here★ — the caller does the sending outside the lock (ADR-0006 ·
    /// this file's header). If the guard went out with it, one delivery waiting for a reply would stall registration, connection cleanup and query all at once.
    ///
    /// ★The query is an **equality comparison with the stored token**★ — it does not trace the connection id back from the token (rationale = [`LiveConn`]).
    ///
    /// ★Use the received handle **at once and drop it** — do not keep it in a wait ticket or correlation table★: the registry has no means to
    /// take back a copy it handed out, so the moment it is kept, that connection's send queue outlives the registry (what a surviving
    /// `Sender` copy breaks is written in the network layer `ws.rs` under 「사본 전수」 (all copies)).
    ///
    /// ★With several connections for one owner, **which one comes out is not guaranteed**★: today
    /// [`CommandRoster::owner_of`] is 1:1 with the connection so this is unreachable, and it becomes reachable when slice B changes the owner key into a client-made
    /// identifier. The selection rule then belongs to that slice, so no particular winner is fixed as a contract here — the order that comes out now is just the
    /// table's iteration order, not a promise (the mismatch with the same root =
    /// the 「알려진 어긋남」 (known mismatch) of [`CommandRoster::detach`]).
    ///
    /// ★Known cost★: it **scans the table linearly** while locked. The number of attached connections has no cap (the network layer's accept
    /// loop has no concurrent-connection limit) and delivery makes this query once per command, so when both grow together
    /// this place contends with registration and connection cleanup for the same lock. A reverse index is outside this slice.
    // ADR-0154
    pub fn sink_of(&self, owner: &OwnerToken) -> Option<Arc<dyn FrameSink>> {
        self.lock()
            .live
            .values()
            .find(|conn| conn.owner == *owner)
            .map(|conn| conn.sink.clone())
    }

    /// The frame outlet of **one connection** — used when a delivery sends the reply back (its sibling [`CommandRoster::sink_of`] looks up by owner
    /// token).
    ///
    /// ★Why the two lookups are not merged★: the destination of an outgoing envelope is the **owner**, and the destination of the returning reply is
    /// the **connection that asked**, so the keys differ. Today the two are 1:1 so they look at the same table, but when slice B changes the owner key into a
    /// client-made identifier, one owner will have several connections and the reply would then go to the wrong connection.
    ///
    /// ★The received handle is used at once and dropped★ — the rationale for the no-keeping rule is the same as [`CommandRoster::sink_of`].
    /// **Absent = that connection has already dropped** (it means there is nowhere to send the reply, and it is not an error).
    ///
    /// ★The only basis for pointing at a connection by [`ConnId`] alone being safe is the fact that **the allocator does not reuse the value**★
    /// — there is no generation number, so the day reuse appears, a long-lingering correlation-table entry hands someone else's reply to a different peer
    /// that newly got the same number. The measurement and the remedy then are in `Pending::origin` of `command_delivery`.
    // ADR-0154
    pub fn sink_for_conn(&self, conn_id: ConnId) -> Option<Arc<dyn FrameSink>> {
        self.lock().live.get(&conn_id).map(|conn| conn.sink.clone())
    }

    /// Is that connection still attached — ★a query that **makes no copy of the outlet**★.
    ///
    /// A place that only needs to know whether it is alive (checking the caller before producing side effects) and uses the sibling [`CommandRoster::sink_for_conn`]
    /// keeps one strong reference alive for the lifetime of that place. If that place is a blocking body of several seconds, the copy stays
    /// alive meanwhile and **the dropped connection's writer cannot finish itself** (ADR-0154's 「내준 사본을 왕복 너머로 들지
    /// 않는다」 (do not hold a handed-out copy across a round trip) — the leak that sentence blocks, exactly). So a caller that needs only the decision uses this one.
    /// ★`ConnId` is not reused★ — this is the basis on which the predicate also reads as 「같은 피어인가」 (is it the same peer), and the authoritative measurement is
    /// `Pending::origin` of `command_delivery`.
    // ADR-0154
    pub fn is_attached(&self, conn_id: ConnId) -> bool {
        self.lock().live.contains_key(&conn_id)
    }

    /// Test only — swap only the stored owner token to deterministically make a **torn window** (the registry query says `Available` but there is no path to reach).
    /// On the production path that state arises only from a `detach` slipping between query and delivery, and cannot be set up by hand.
    #[cfg(test)]
    pub(crate) fn overwrite_stored_owner(&self, conn_id: ConnId, owner: OwnerToken) {
        self.lock()
            .live
            .get_mut(&conn_id)
            .expect("붙어 있는 연결이어야 한다")
            .owner = owner;
    }

    /// The connection dropped — erase that owner's names from the registry and withdraw the path to that connection. **Both happen inside one lock**
    /// (a concurrently running registration cannot get in between).
    ///
    /// ★It uses the removal order ADR-0154 fixed (registry first, path later)★ — today the two are inside one
    /// lock so nobody can observe the 「명부엔 있는데 닿을 수 없는」 (in the registry but unreachable) intermediate state, but the order has to
    /// stay in the code so that window does not open the day the table is split outside the lock.
    ///
    /// ## ★There are two call sites, and they withdraw **different kinds of connection**★
    ///
    /// ① **Drop of a real connection** — only `AgentConnection::on_disconnect`. It is the only path that knows about a drop, so
    ///    making a second removal point of this kind would split the causality (ADR-0150). That clause remains valid.
    /// ② **End of a fake caller connection's round trip** — the destructor of `command_delivery::CallerSeat` (ADR-0160). It is the connection
    ///    one control-route call sets up to receive a reply, and it **never reaches `on_disconnect`** (there is no socket).
    ///
    /// ★So do not write ①'s invariant back as 「전체 하나」 (one for the whole)★ — that sentence is now false, and if left false,
    /// an edit that deletes ② would look justified by the 「단일 제거 지점」 (single removal point). If ② is deleted, that slot stays for the whole process
    /// lifetime and permanently lengthens the linear scan in [`CommandRoster::sink_of`].
    /// ★The basis that the carve-out holds = **the two kinds cannot touch each other's entries**★: fake connections' numbers count down from `u64::MAX`
    /// and real connections count up from 1 (that issuer's doc), and fake connections register no names so there is no registry
    /// entry to erase at all. That is, ② does nothing beyond removing its own slot from `live`.
    /// ★Log what was erased — this line is that event's **only diagnostic surface**★: the registry keeps no trace of a dropped owner
    /// (ADR-0150 decision 3), so there is no way to trace the vanished names back through a query. If this line is removed, there is no material anywhere to answer 「어느 명령이 왜
    /// 없어졌나」 (which command vanished and why) (the bounce log tells only about **refused packets**). The level is `info!` because it is a connection-lifetime
    /// event (`docs/reference/logging-conventions.md` level table · the connection-lifetime item of the same document's 「계측 의무」).
    /// Only a drop with no names to remove drops to `debug!` — with nothing erased this line has no event to tell, and
    /// the connection's own end is already logged by the network layer, and **registering clients are still at zero, so that branch is
    /// the usual case in full**.
    /// ★The log is called **after** the lock is released★ — the file sink writes synchronously (logging convention 「인프라」), so calling
    /// inside the critical section stalls registration, query and other connections' cleanup for as long as that IO takes.
    /// `help` is not included — it is a string the client sent and its cap is 32 times that of a name (`Roster::MAX_HELP_BYTES`).
    ///
    /// ★Known mismatch — the removal unit is the **owner token** but the key of the liveness table (`Shared::live`) is the **[`ConnId`]**★:
    /// today it is harmless because [`CommandRoster::owner_of`] is 1:1 with the connection. If slice B changes the owner key into a client-made
    /// identifier so that **several connections share one owner**, `detach(conn1)` erases even the registrations of the still-alive `conn2`,
    /// and `conn2` keeps passing `Shared::refuse_if_detached`, so it **does not know its names were erased**
    /// (registration happens only once at attach, so there is no occasion to put them up again). ★Slice B must close this★ — to
    /// fix it now the removal unit would have to be narrowed to the connection, but [`ConnId`] is a type the tool crate does not know, so the act of fixing
    /// itself would pre-decide that slice's owner model (user decision 2026-08-17 — `docs/process/step-log.md` ㉲).
    ///
    /// ★The owner to erase is **read from the table** — not derived again here★: if derived, the day it diverges from the token [`CommandRoster::attach`]
    /// stored, `disconnect` could erase nothing, and those names would stay `Available` **toward a dead owner**
    /// for the daemon's whole lifetime (ADR-0154). If the connection is not in the table there is nothing to erase either — because a registration itself
    /// stands only by passing [`Shared::refuse_if_detached`].
    // ADR-0150
    // ADR-0154
    pub fn detach(&self, conn_id: ConnId) {
        // ★Drop the withdrawn entry **outside** the lock★: this may be the last strong reference to the frame outlet (a client that
        //   registered but subscribed to no agent is such a case), and then channel closing and waker waking run inside the critical
        //   section. That is outside the 「잠그고 한 번 부른 뒤 즉시 놓는다」 (lock, call once, then release at once) this file's header promised.
        let (removed, reclaimed) = {
            // ★Do not panic on a lock failure — both callers of this function are **cleanup paths**★ (① and ② above). ②
            //   is a destructor, so a panic here is an immediate abort if it happens during unwinding, and even if not, the destructor's
            //   **next line** (correlation-table cleanup) would not run at all and both tables would leak together. And what this function does is
            //   **removal only**, so the risk of continuing to use a poisoned table is also smaller than for its sibling verbs (registration, query) —
            //   the siblings carry the panic of [`CommandRoster::lock`] as is.
            //   The authoritative source of the paired discipline = `command_delivery::CommandDeliveries::lock_for_cleanup`.
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
            tracing::debug!(conn = conn_id, "연결 끊김 — 명부에서 내릴 이름이 없다");
        } else {
            tracing::info!(
                conn = conn_id,
                names = removed.len(),
                removed = %logged_names(&removed),
                "연결 끊김 — 명령 명부에서 이 주인의 이름을 지웠다"
            );
        }
    }

    /// Full registration at attach time. A dropped connection's late packet is bounced with `CONFLICT`. The owner a name stands for is
    /// the token [`CommandRoster::attach`] stored — it is not derived again here (ADR-0154).
    pub fn register(&self, conn_id: ConnId, decls: Vec<CommandDecl>) -> Result<(), CommandError> {
        let mut shared = self.lock();
        let owner = shared.refuse_if_detached(conn_id)?;
        shared.roster.register(&owner, decls)
    }

    /// Diff while attached. The bounce rule and the source of the owner token are the same as [`CommandRoster::register`].
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

    /// Snapshot of the whole registry. If [`Roster::entries`]'s iterator were handed out as is, the lock would be held while the caller iterates,
    /// so it is drained here.
    pub fn entries(&self) -> Vec<RosterEntry> {
        self.lock().roster.entries().collect()
    }

    /// A copy of the agent-reference slots (in registration order) that this name's owner marked — delivery reads it before relaying
    /// ([`Roster::agent_args`]). The lock ends inside this call.
    // ADR-0308
    pub fn agent_args_of(&self, name: &str) -> Vec<String> {
        self.lock().roster.agent_args(name).to_vec()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.inner.lock().expect("command roster poisoned")
    }

    /// Lock acquisition that does not panic on failure — ★**for the cleanup path only**★. The only caller is
    /// [`CommandRoster::detach`], and the reason is in the comment inside that function.
    ///
    /// ★A production process cannot reach this branch (known scope)★ — a panic holding this table is outside the containment mark so the hook
    /// ends the process before unwinding and no poisoning arises at all (ADR-0292). It is kept anyway because in a hook-less test build that
    /// branch is real, and a table that leaks there ruins the observation of regression tests as it is.
    fn lock_for_cleanup(&self) -> std::sync::MutexGuard<'_, Shared> {
        sync::lock(&self.inner)
    }
}

/// The **one thing** stage 2 of the 3-stage delivery asks the registry (`engram_dashboard_command::route`).
///
/// ★The lock ends inside this call — that is the reason this trait exists★: if the reference were handed over whole, the registry would stay locked for as long as the delivery
/// waits for a reply, and registration, connection cleanup and other deliveries would all stall (authoritative rationale = that trait's comment).
// ADR-0154
impl OwnerLookupSource for CommandRoster {
    fn lookup(&self, name: &str) -> OwnerLookup {
        self.lock().roster.lookup(name)
    }
}

impl Shared {
    /// ★This is the **only** net that stops late packets from a dropped connection★ — the rationale and 「두 번째를 세우지 말 것」 (do not build a second one) are in this file's
    /// header.
    ///
    /// If it passes, it returns the **owner token** stored when that connection attached — so the caller does not derive it again
    /// (the single derivation point is [`CommandRoster::attach`]. ADR-0154). The refusal branch has no token, and that is
    /// the meaning of this net.
    ///
    /// ★Code choice★: the packet is fine (not `INVALID_ARGUMENT`) and it is not the caller's fault — what is refused is the
    /// **state** (that connection is not on the list now), so it is `CONFLICT`. That code's retry instruction is `never` — resending over the same
    /// connection is pointless since that connection is already gone, and when it attaches again it gets a **new token**, so it is not a retry but a
    /// re-registration.
    ///
    /// ★Why the wording is not narrowed to 「끊겼다」 (dropped)★: the same branch also stands for a connection that **never attached**. Today
    /// it does not occur on the production path (the network layer's order that `on_connect` comes before dispatch) but the type does not
    /// prevent it — `register`/`update` take a bare [`ConnId`].
    ///
    /// ★No separate log is left for this bounce★ — the caller (`reply_roster` in `connection_core`) already leaves every registry refusal
    /// as a `warn!` (conn · verb · code · wording). One more line here would write the same event twice,
    /// and that line would go out **while holding the lock** (the same reason [`CommandRoster::detach`] moved its log outside the lock).
    /// ★Known mismatch — this decision's unit is [`ConnId`] but the removal unit is the owner token★: today the two are 1:1 so it is
    /// harmless, but if slice B makes several connections share one owner, there will be connections that **pass this net even after
    /// their own registration was erased by someone else's `detach`** — that connection does not know its names are gone, and registration is only once at attach
    /// so there is no occasion to undo it. The authoritative rationale and 「슬라이스 B 가 닫는다」 (slice B closes it) are in the [`CommandRoster::detach`] comment.
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

    /// One outlet to go in the table and **where that outlet reaches**. The reason handles are not compared by `Arc` pointer is that what this table
    /// guards is 「그 핸들로 보내면 그 연결로 간다」 (sending through that handle goes to that connection), not 「같은 객체가 나왔다」 (the same object came out).
    fn sink_with_inbox() -> (Arc<dyn FrameSink>, mpsc::Receiver<Frame>) {
        let (tx, rx) = mpsc::channel::<Frame>(4);
        (Arc::new(FakeFrameSink::new(tx)), rx)
    }

    /// Used as a placeholder when a test that does not look at the path needs to fill the slot.
    fn some_sink() -> Arc<dyn FrameSink> {
        sink_with_inbox().0
    }

    /// An outlet that records **whether the registry lock was released** at the time of its destruction. It drops frames — what this double watches is
    /// not delivery but when it itself dies.
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

    /// Mark the handle so we can tell which receiver it went out to.
    fn ping(sink: &Arc<dyn FrameSink>, mark: &str) {
        sink.try_send(Frame::Text(mark.to_string()))
            .expect("가짜 출구는 비어 있다");
    }

    fn delivered(rx: &mut mpsc::Receiver<Frame>) -> Option<String> {
        match rx.try_recv() {
            Ok(Frame::Text(text)) => Some(text),
            Ok(other) => panic!("Text 여야 함: {other:?}"),
            Err(_) => None,
        }
    }

    /// Ask the registry directly — what delivery sees is this answer (not the projection of `entries`).
    fn lookup(roster: &CommandRoster, name: &str) -> OwnerLookup {
        let shared = roster.lock();
        shared.roster.lookup(name)
    }

    // ── Registration running concurrently with connection cleanup (the remaining race of the frame_port::ConnectionHandler contract) ──────
    //
    // Nail down four branches: {registration, diff} × {a name nobody holds, a name a live connection holds}. The registry has no
    // trace of a dropped owner (ADR-0150), so this net can stand only on **connection lifetime**.
    //
    // ★Looking at the code alone is not enough — look at the wording too★: the same `CONFLICT` is also raised by `Roster::check_added_are_not_taken`,
    // so if only the code is asserted, some of these stay green **for another reason** even when this file's net is deleted entirely.

    /// Also check whether the name comes back to life — if a late registration puts back a name cleanup erased, that name becomes `Available` toward a
    /// **nonexistent owner** for the daemon's whole lifetime (no cleanup will ever come again for that connection).
    #[test]
    fn a_registration_that_lands_after_its_disconnect_is_refused() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());
        roster.register(1, vec![decl("tab.create")]).expect("등록");
        roster.detach(1);

        let err = roster
            .register(1, vec![decl("tab.create")])
            .expect_err("정리 뒤에 내려앉은 등록");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(
            err.message(),
            DETACHED_REFUSAL,
            "연결 수명 그물이 잡은 것이어야 한다"
        );
        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Unknown,
            "정리가 지운 이름이 되살아나면 안 된다"
        );
        assert!(roster.entries().is_empty(), "명부에 남는 것이 없다");
    }

    /// ★A branch the registry alone cannot stop★ — a dropped owner's registrations are erased, so `Roster` has no place to remember that death,
    /// and a late registration passes as if taking over a live connection's names by handover. What stops it here is the connection
    /// lifetime.
    #[test]
    fn a_late_registration_cannot_take_a_live_connections_name() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());
        roster.register(1, vec![decl("tab.create")]).expect("등록");
        roster.detach(1);
        roster.attach(2, &some_sink());
        roster
            .register(2, vec![decl("tab.create")])
            .expect("산 연결이 이름을 이어받는다");

        let err = roster
            .register(1, vec![decl("tab.create")])
            .expect_err("죽은 연결의 늦은 등록");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(
            err.message(),
            DETACHED_REFUSAL,
            "연결 수명 그물이 잡은 것이어야 한다"
        );
        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Available(CommandRoster::owner_of(2)),
            "산 연결이 그대로 주인이다"
        );
    }

    #[test]
    fn a_delta_that_lands_after_its_disconnect_is_refused() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());
        roster.register(1, vec![decl("tab.create")]).expect("등록");
        roster.detach(1);

        let err = roster
            .update(1, vec![decl("tab.split")], vec![])
            .expect_err("정리 뒤에 내려앉은 차분");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(
            err.message(),
            DETACHED_REFUSAL,
            "연결 수명 그물이 잡은 것이어야 한다"
        );
        assert_eq!(lookup(&roster, "tab.split"), OwnerLookup::Unknown);
    }

    #[test]
    fn a_late_delta_cannot_take_a_live_connections_name() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());
        roster.register(1, vec![decl("tab.create")]).expect("등록");
        roster.detach(1);
        roster.attach(2, &some_sink());
        roster
            .register(2, vec![decl("tab.create")])
            .expect("산 연결이 이름을 이어받는다");

        let err = roster
            .update(1, vec![decl("tab.create")], vec![])
            .expect_err("죽은 연결의 늦은 차분");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(
            err.message(),
            DETACHED_REFUSAL,
            "연결 수명 그물이 잡은 것이어야 한다"
        );
        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Available(CommandRoster::owner_of(2))
        );
    }

    /// ★A spot the registry check bypasses entirely — here connection lifetime is the only net★
    ///
    /// A diff that adds a name nobody holds is passed by `Roster::check_added_are_not_taken`, and the registry has no check at all
    /// that would tell a dropped owner apart (ADR-0150). If the net is missing, `tab.split` becomes `Available` toward a **nonexistent owner**
    /// and stays stuck for the daemon's whole lifetime.
    #[test]
    fn a_late_delta_adding_an_unclaimed_name_is_refused() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());
        roster.register(1, vec![decl("tab.create")]).expect("등록");
        roster.detach(1);
        roster.attach(2, &some_sink());
        roster
            .register(2, vec![decl("tab.create")])
            .expect("산 연결이 같은 이름을 새로 얹는다");

        let err = roster
            .update(1, vec![decl("tab.split")], vec![])
            .expect_err("죽은 연결의 늦은 차분");

        assert_eq!(err.code(), ErrorCode::Conflict);
        assert_eq!(
            err.message(),
            DETACHED_REFUSAL,
            "연결 수명 그물이 잡은 것이어야 한다"
        );
        assert_eq!(
            lookup(&roster, "tab.split"),
            OwnerLookup::Unknown,
            "죽은 연결 앞으로 새 이름이 서면 안 된다"
        );
    }

    /// ★The log must say **what the disconnect erased**★
    ///
    /// Since no trace is kept (ADR-0150 decision 3), the vanished names **do not remain in the data** — there is no way to trace them back
    /// through a query, so this one line is the only surface that answers 「어느 명령이 왜 없어졌나」 (which command vanished and why). The bounce log (`reply_roster` in `connection_core`)
    /// tells only about **refused packets**, so it is no substitute.
    ///
    /// We look not at the wording but at **the level and the fields**: it is `info!` because it is a connection-lifetime event
    /// (`docs/reference/logging-conventions.md` level table), and since the count alone cannot point out which command vanished,
    /// the names are carried too.
    #[test]
    fn a_detach_logs_the_names_it_removed() {
        let roster = CommandRoster::new();
        roster.attach(7, &some_sink());
        roster
            .register(7, vec![decl("tab.create"), decl("tab.close")])
            .expect("등록");

        let logged = capture_info(|| roster.detach(7));

        assert!(logged.contains("conn=7"), "어느 연결인지: {logged:?}");
        assert!(
            logged.contains("tab.create") && logged.contains("tab.close"),
            "지운 이름이 실려야 한다: {logged:?}"
        );
    }

    /// ★The peer cannot decide this line's **shape**★ — a single newline would split the log line in two, and the second line would look like an entry
    /// we wrote (a forged entry).
    ///
    /// The registry looks only at the **byte length** of a name and not the character set (the tool crate `Roster`'s registration check), so a name
    /// containing a newline really does get registered — that is this test's premise.
    /// ★Put up **only one** name — do not merge this axis with the count axis into one test★: when I actually merged them,
    /// the moment the fill exceeded [`MAX_LOGGED_NAMES`] the forged name fell **on the truncated side** of the name-order sort, so it stayed green even with the sanitizer
    /// removed (that is, that form never measures the shape axis even once). The count axis is measured by the sibling below.
    #[test]
    fn a_control_character_in_a_removed_name_never_reaches_the_log_field_verbatim() {
        let roster = CommandRoster::new();
        roster.attach(9, &some_sink());
        let forged = "tab.forged\n2026-08-19T00:00:00Z  WARN forged entry";
        roster.register(9, vec![decl(forged)]).expect("등록");

        let logged = capture_info(|| roster.detach(9));

        assert!(
            logged.contains("tab.forged"),
            "전제: 그 이름이 이 줄에 실렸다: {logged:?}"
        );
        assert!(
            !logged.contains('\n'),
            "개행이 원문으로 실렸다 — 로그 줄이 쪼개진다: {logged:?}"
        );
    }

    /// ★The peer cannot decide this line's **size**★ — one owner can put up 512 names (the tool crate's
    /// `Roster::MAX_NAMES_PER_OWNER`), so without truncation this one line becomes 64 KiB.
    ///
    /// ★`names` is not truncated★ — the exact count must remain so 「몇 개가 사라졌나」 (how many vanished) does not blur. So what gets cut
    /// is only the tail of 「어느 이름이었나」 (which names were they), and that fact is also written on the line.
    #[test]
    fn the_removed_names_line_says_how_many_it_elided() {
        let roster = CommandRoster::new();
        roster.attach(9, &some_sink());
        let total = MAX_LOGGED_NAMES + 3;
        let decls: Vec<CommandDecl> = (0..total).map(|i| decl(&format!("tab.n{i:02}"))).collect();
        roster.register(9, decls).expect("등록");

        let logged = capture_info(|| roster.detach(9));

        assert!(
            logged.contains(&format!("names={total}")),
            "개수는 정확해야 한다: {logged:?}"
        );
        assert!(
            logged.contains(&format!("(+{} more)", total - MAX_LOGGED_NAMES)),
            "자른 사실을 말해야 한다: {logged:?}"
        );
        assert!(
            !logged.contains(&format!("tab.n{:02}", total - 1)),
            "마지막 이름은 잘려 나가야 한다: {logged:?}"
        );
        // With a generous cap, only pin down 「상대가 크기를 정하지 못한다」 (the peer cannot decide the size) — pinning an exact number would break
        //   every time a field is added.
        assert!(
            logged.len() < 1024,
            "줄 크기를 상대가 정한다: {} bytes",
            logged.len()
        );
    }

    /// A disconnect with no names to remove does not emit `info!` — with nothing erased this line has no event to tell, and the
    /// connection's own end is already logged by the network layer. Since registering clients are still at zero (TRD §3-7) this branch is
    /// **the usual case in full**, so if it were not separated here, the default path would be buried under pointless lines.
    #[test]
    fn a_detach_with_nothing_to_remove_stays_quiet_at_info() {
        let roster = CommandRoster::new();
        roster.attach(7, &some_sink());

        let logged = capture_info(|| roster.detach(7));

        assert!(
            logged.is_empty(),
            "지운 것이 없으면 info 는 비어야 한다: {logged:?}"
        );
    }

    /// Cleanup of a connection that never attached — there is no entry in the table so there is no owner to read and no name to erase. This branch arose when `detach`
    /// began to **read the owner token from the table**. It must be quiet, and above all it must not touch a live connection's
    /// names.
    #[test]
    fn a_detach_for_a_connection_that_never_attached_is_quiet() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());
        roster.register(1, vec![decl("tab.create")]).expect("등록");

        let logged = capture_info(|| roster.detach(2));

        assert!(
            logged.is_empty(),
            "지울 것이 없으면 info 는 비어야 한다: {logged:?}"
        );
        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Available(CommandRoster::owner_of(1)),
            "남의 정리가 산 연결의 이름을 건드리면 안 된다"
        );
    }

    /// A minimal collector that gathers only the fields of INFO events — the reason for not using the format layer is that what this test looks at is
    /// only one thing, 「그 필드가 실린 INFO 이벤트가 났는가」 (did an INFO event carrying that field occur) (the same form as in `control::tests`).
    /// `with_default` is installed **only on this thread**, so it does not mix with parallel tests.
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

    /// A reconnect comes with a **new connection id** so it is not blocked — the boundary that the bounce above does not lock the normal path.
    #[test]
    fn reconnecting_under_a_new_connection_id_registers_normally() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());
        roster.register(1, vec![decl("tab.create")]).expect("등록");
        roster.detach(1);

        roster.attach(2, &some_sink());
        roster
            .register(2, vec![decl("tab.create")])
            .expect("재연결은 새 토큰으로 온다");

        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Available(CommandRoster::owner_of(2))
        );
    }

    // ── Owner token → the path to that owner (ADR-0154) ──────────────────────────────────
    //
    // Measure in exactly the order delivery follows: ask the registry by name to get the owner (`lookup`), and with that owner get the
    // outlet (`sink_of`). So **the lookup key is a value the registry really holds**, not a string the test
    // made up by hand.

    #[test]
    fn an_attached_owner_resolves_to_that_connections_sink() {
        let roster = CommandRoster::new();
        let (sink, mut inbox) = sink_with_inbox();
        roster.attach(1, &sink);
        roster.register(1, vec![decl("tab.create")]).expect("등록");

        let owner = match lookup(&roster, "tab.create") {
            OwnerLookup::Available(owner) => owner,
            other => panic!("주인이 있어야 함: {other:?}"),
        };
        let found = roster
            .sink_of(&owner)
            .expect("붙어 있는 주인은 닿는 길이 있다");
        ping(&found, "envelope");

        assert_eq!(
            delivered(&mut inbox),
            Some("envelope".to_string()),
            "그 연결의 출구로 나가야 한다"
        );
    }

    /// A dropped owner has no path to reach — and ★merely not being visible is not enough; it must be **released**★.
    ///
    /// So we measure not only the absence in the query but also whether the strong reference really disappeared. An implementation that keeps only a tombstone and holds
    /// the `Arc` is green with the preceding assertion alone. Two things such an implementation breaks: the channel allocation and `Notify` stay for the daemon's
    /// whole lifetime, and — this one matters more downstream — **the surviving table entry makes
    /// [`CommandRoster::sink_of`] answer `Some` for a dead owner**, so delivery becomes a silent
    /// black hole instead of an honest 「주인 없음」 (no owner).
    /// (It is unrelated to the writer task's self-termination — the network layer calls `abort()` on the losing task
    /// **before** `on_disconnect`, so the registry's copy cannot turn the writer into a zombie.)
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
            "표가 핸들을 실제로 놓아야 한다 — 안 보이기만 하면 묘비다"
        );
        assert!(
            !roster.lock().live.contains_key(&1),
            "항목 자체가 사라져야 한다 — 핸들만 비운 묘비를 남기면 연결이 오갈수록 표가 무한히 자란다"
        );
    }

    /// ★The withdrawn handle's destructor runs **outside the critical section**★ — if `detach`'s removal is moved back inside the lock block, the
    /// destructor runs inside it. The registry does not know what code running at that spot does (it is `dyn`), and that is outside the
    /// 「잠그고 한 번 부른 뒤 즉시 놓는다」 (lock, call once, then release at once) this file's header promised. Nothing else in the repository goes red on such a revert,
    /// so this test is the only net for it.
    ///
    /// How it measures: the destructor tries to take the registry lock at that moment. If the same thread already holds it, `try_lock` returns
    /// `WouldBlock`, so the two cases are told apart **without deadlock**.
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
            "소멸자가 명부 잠금을 쥔 채 돌았다"
        );
    }

    /// ★Measure that registration, query and removal all use the stored token even when it is **not** of the derived form★
    ///
    /// An implementation that re-derives with [`CommandRoster::owner_of`] in any of the three goes red here — a `register` that re-derives
    /// puts the name up under `conn-1`, and a `detach` that re-derives erases nothing.
    /// ★This does not pre-decide slice B★: what changes is one value in the table, not the signature of [`CommandRoster::attach`] —
    /// the production path still computes the token by itself.
    #[test]
    fn the_roster_files_and_removes_under_the_stored_token() {
        let roster = CommandRoster::new();
        let (sink, _inbox) = sink_with_inbox();
        roster.attach(1, &sink);
        let stored = OwnerToken::new("shell-a1");
        roster.lock().live.get_mut(&1).expect("붙어 있다").owner = stored.clone();

        roster.register(1, vec![decl("tab.create")]).expect("등록");

        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Available(stored.clone()),
            "명부는 저장된 토큰 앞으로 이름을 세운다"
        );
        assert!(
            roster.sink_of(&stored).is_some(),
            "조회도 저장된 토큰으로 닿는다"
        );
        assert!(
            roster.sink_of(&CommandRoster::owner_of(1)).is_none(),
            "파생형으로는 닿지 않는다"
        );
        assert_eq!(roster.attached_owner(1), Some(stored));

        // ★Wrap with `capture_info` — this `detach` has names to erase so it emits `info!`★: when a thread with no subscriber
        //   registers that callsite **first**, 「관심 없음」 (no interest) gets baked in, and `a_detach_logs_the_names_it_removed`, which measures that line afterwards,
        //   comes back empty-handed (it really broke that way in a 4-thread run,
        //   2026-08-18). ★Do not write it as 「전역 캐시라 한 번 때리면 굳는다」 (it is a global cache so it sticks once hit) — that statement is wrong★: the
        //   actual condition of the flaw (first registration across threads) and the measured evidence are in the `log_capture` module header, the authority.
        capture_info(|| roster.detach(1));

        assert_eq!(
            lookup(&roster, "tab.create"),
            OwnerLookup::Unknown,
            "제거도 저장된 토큰으로 한다 — 재파생하면 이 이름이 죽은 주인 앞에 남는다"
        );
    }

    #[test]
    fn an_owner_that_never_attached_has_no_sink() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());

        assert!(
            roster.sink_of(&CommandRoster::owner_of(2)).is_none(),
            "붙은 적 없는 연결"
        );
        assert!(
            roster.sink_of(&OwnerToken::new("shell")).is_none(),
            "이 데몬이 낸 적 없는 모양의 토큰"
        );
    }

    /// When several are attached, **each gets its own outlet** — if mixed, one client's command is delivered to another
    /// client (which executes something it did not order).
    #[test]
    fn each_owner_gets_its_own_sink() {
        let roster = CommandRoster::new();
        let (first, mut first_inbox) = sink_with_inbox();
        let (second, mut second_inbox) = sink_with_inbox();
        roster.attach(1, &first);
        roster.attach(2, &second);

        ping(
            &roster.sink_of(&CommandRoster::owner_of(1)).expect("주인 1"),
            "to-1",
        );
        ping(
            &roster.sink_of(&CommandRoster::owner_of(2)).expect("주인 2"),
            "to-2",
        );

        assert_eq!(delivered(&mut first_inbox), Some("to-1".to_string()));
        assert_eq!(delivered(&mut second_inbox), Some("to-2".to_string()));
        assert_eq!(delivered(&mut first_inbox), None, "1 번에 남의 것이 섞였다");
        assert_eq!(
            delivered(&mut second_inbox),
            None,
            "2 번에 남의 것이 섞였다"
        );
    }

    /// ★Bite on the fact that the query does **not parse** the token★
    ///
    /// Every string below becomes **1** when `"conn-"` is stripped and the rest read as a number (`"01"` and `"+1"` are accepted by Rust's integer
    /// parser). That is, a query that strips the prefix to trace the connection id back hands conn 1's outlet to these, while a query that compares by
    /// equality with the stored token comes up empty-handed. The rationale for forbidding tracing back, since that derivation is a provisional rule, is
    /// [`LiveConn`].
    ///
    /// ★There is one variant this test cannot bite★: an implementation that parses and then **compares back** with [`CommandRoster::owner_of`] comes up empty-handed
    /// for the strings above too, so it passes here. That variant only diverges when the stored token is not of the derived form, so
    /// [`the_roster_files_and_removes_under_the_stored_token`] catches it —
    /// that one changes the table's value directly to create that state.
    #[test]
    fn a_lookup_compares_the_stored_token_and_never_parses_it() {
        let roster = CommandRoster::new();
        roster.attach(1, &some_sink());

        for impostor in ["conn-01", "conn-+1"] {
            assert!(
                roster.sink_of(&OwnerToken::new(impostor)).is_none(),
                "저장된 토큰과 다른 문자열이다: {impostor}"
            );
        }
        assert!(
            roster.sink_of(&CommandRoster::owner_of(1)).is_some(),
            "저장된 그 토큰은 여전히 닿는다"
        );
    }
}
