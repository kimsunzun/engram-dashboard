//! Usage subscription roster — the vendor set each connection subscribed to and that connection's outlet (TRD §1-4 「subscription roster」 · ADR-0254). [`deliver`], which carries one emission sheet to the targets, is here too.
//!
//! ★Cells are made only by [`UsageWatch::attach`] and erased only by [`UsageWatch::detach`]★ — [`UsageWatch::replace`] only fixes existing cells, so a late replacement overlapping disconnect cleanup cannot revive an erased cell (§3 #56 — same shape as `attach` · `refuse_if_detached` of the command registry).
//! ★Erasing a cell withdraws its outlet ([`UsageOutlet::revoke`])★ — the frame outlet it holds is dropped before [`UsageWatch::detach`] returns (duty of `conn_port::ConnectionHandler::on_disconnect`), and any [`Target`] copy left outside goes quiet; only one temporary copy per send in progress remains.
//! ★Never build or send while holding the registry lock★ — inside the lock only the targets (outlet copy + that connection's current set) are taken; encoding and sending are done by [`deliver`] without a lock, so an outlet or encoder calling back into this registry cannot deadlock.
//! ★Lock order = registry → book★ — the only code run under the registry lock is `first_sheets` of [`UsageWatch::replace`]. Taking the book there is fine; calling the registry again is a deadlock (std `Mutex` is not reentrant), and nothing may call the registry while holding the book.
// ADR-0006

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::conn_port::ConnId;
use engram_dashboard_agent::usage::UsageVendorKey;
use engram_dashboard_base::sync;
use engram_dashboard_protocol::UsageLimitSnapshot;

/// The outlet of one connection — the real one is made fresh per connection in `on_connect`: it holds one cell (`Mutex<Option<Arc<dyn FrameSink>>>`); `send` locks the cell, copies the `Arc` (if present), releases, then `try_send`s outside the cell; `revoke` locks the cell and `take`s it out.
///
/// ★Never send while holding the cell (ADR-0006)★ — inside the cell only copying and taking out: no registry · service · other lock, and logging only after releasing. So `revoke` does not wait for a send in progress.
/// Both methods are called without the registry lock, so calling the registry again inside cannot deadlock.
pub trait UsageOutlet: Send + Sync {
    /// ★Must be non-blocking★ — called from the pump thread · scheduler · query completion guard, and the connection task (first sheet of a subscription · request-path emission). Failure (saturation · closed) is handled by the outlet itself, not returned.
    fn send(&self, frame: &UsageFrame);

    /// Withdraws this outlet — called once per cell when the registry erases a cell (disconnect · another outlet overwrote it with the same connection id), outside the registry lock. It is on the sync connection cleanup path, so it must not block.
    ///
    /// ★After it returns the outlet holds no frame outlet and `send` does nothing★ — a [`Target`] left outside goes quiet; only one copy per `send` in progress (per thread) remains and is dropped after that one `try_send` (TRD §1-4 「concurrency · locks」 · §3 #87).
    fn revoke(&self);
}

/// One emission sheet built for one `subscribed` set — `json` is the one-line event carrying `snapshot` and that set. Targets of the same set share the same `json`.
#[derive(Debug, Clone)]
pub struct UsageFrame {
    pub snapshot: UsageLimitSnapshot,
    pub json: Arc<str>,
}

/// The port that builds the event JSON — the assembly gives the real one.
///
/// Called outside the registry lock, once per distinct `subscribed` in one emission. `None` = building failed — that set's targets skip this emission. ★The encoder logs the failure itself★ — [`deliver`] does not.
pub trait UsageEncoder: Send + Sync {
    fn encode(
        &self,
        snapshot: &UsageLimitSnapshot,
        subscribed: &BTreeSet<UsageVendorKey>,
    ) -> Option<Arc<str>>;
}

/// One emission target — that connection's outlet copy taken inside the registry lock and the set at that time.
///
/// ★The copy goes stale once the lock is released★ — `subscribed` can be one step late (the shell's cross-check just sends the same set one more sheet · TRD §1-7), and the connection may already have dropped (the withdrawn outlet does nothing).
/// ★Pass it to [`deliver`] where it was taken — do not pile them up or hold them across a wait★: a stale set is carried, and the dropped connection's outlet shell lives that long.
#[derive(Clone)]
pub struct Target {
    pub outlet: Arc<dyn UsageOutlet>,
    pub subscribed: Arc<BTreeSet<UsageVendorKey>>,
}

/// Result of [`UsageWatch::replace`] — `first_sheets` are the first sheets of the vendors newly added by this replacement and `target` is that one connection (`subscribed` = the set after replacement). The first sheet is [`deliver`]ed to `target` only.
pub struct Replaced {
    pub first_sheets: Vec<UsageLimitSnapshot>,
    pub target: Target,
}

struct Entry {
    // A replacement sets up a new `Arc`, shared across targets instead of copying the set per target.
    vendors: Arc<BTreeSet<UsageVendorKey>>,
    outlet: Arc<dyn UsageOutlet>,
}

pub struct UsageWatch {
    entries: Mutex<HashMap<ConnId, Entry>>,
}

impl Default for UsageWatch {
    fn default() -> Self {
        Self::new()
    }
}

impl UsageWatch {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// A connection attached (`on_connect`) — makes a cell with an empty set. ★The only place that makes cells★.
    /// The same connection id again overwrites the earlier cell and withdraws the earlier outlet (ids are not reused — if they are, an error log).
    ///
    /// ★Take the outlet by value and wrap it in an `Arc` here — each cell has its own outlet object★: so withdrawing cannot silence another cell's outlet (the `revoke` of an erased cell runs after the lock is released and can overlap a cell newly added in between). Do not take an `Arc` made outside — a blanket `UsageOutlet` impl for `Arc` · `Box`, or an outlet sharing the cell (lock) with another outlet (a `Clone`able `Arc` wrapper) opens the same path. The real one holds the cell directly.
    pub fn attach(&self, conn: ConnId, outlet: impl UsageOutlet + 'static) {
        let replaced = self.lock().insert(
            conn,
            Entry {
                vendors: Arc::new(BTreeSet::new()),
                outlet: Arc::new(outlet),
            },
        );
        if let Some(previous) = replaced {
            tracing::error!(
                conn,
                "attached to the usage roster twice with the same connection id — overwrote the earlier entry (connection ids are not reused)"
            );
            // Withdraw and drop outside the lock — same reason as `detach`.
            previous.outlet.revoke();
        }
    }

    /// A connection dropped (`on_disconnect`) — erases the cell and withdraws its outlet before returning ([`UsageOutlet::revoke`]), so the frame outlet it holds is dropped inside this call (duty of `conn_port::ConnectionHandler::on_disconnect`) and any [`Target`] copy outside goes quiet. ★The only place that erases cells★. Returns whether there was a cell.
    pub fn detach(&self, conn: ConnId) -> bool {
        // ★Withdraw and drop outside the lock★ — this registry does not know what withdrawing or destruction does (the real one closes the queue sender · wakes). Same reason as the command registry `detach`.
        let Some(removed) = self.lock().remove(&conn) else {
            return false;
        };
        removed.outlet.revoke();
        true
    }

    /// `UsageSubscribe` — replaces that connection's set whole (empty set = unsubscribe · TRD §3 #54). Even if the set is unchanged it is `Some`.
    /// ★No cell (a replacement arriving after the drop) → does nothing and `None`★ — the cell is not revived and nothing is logged here; the caller logs (connection id · debug) outside the lock (same as the command registry `refuse_if_detached`).
    ///
    /// ★`first_sheets` is called once, inside the registry lock, only when a vendor was newly added★ (argument = the newly added vendors, in key order) — inside it the book is taken and the first sheet built (lock order `roster → book`). ★Only short in-memory work in the book (`eval_time` · snapshot) — no I/O · waiting · calls to this registry★ (a deadlock). While it runs, the pump's and scheduler's [`UsageWatch::targets`] are blocked. It is called after the set is fixed so 「register first, snapshot later」 holds (§3 #57): an overlapping emission is either in this snapshot or sees this connection as a target with the changed set.
    #[must_use = "the caller sends the first sheet to that connection"]
    pub fn replace(
        &self,
        conn: ConnId,
        vendors: BTreeSet<UsageVendorKey>,
        first_sheets: impl FnOnce(&[UsageVendorKey]) -> Vec<UsageLimitSnapshot>,
    ) -> Option<Replaced> {
        let mut entries = self.lock();
        let entry = entries.get_mut(&conn)?;
        let previous = std::mem::replace(&mut entry.vendors, Arc::new(vendors));
        let added: Vec<UsageVendorKey> = entry.vendors.difference(&previous).copied().collect();
        let target = Target {
            outlet: entry.outlet.clone(),
            subscribed: entry.vendors.clone(),
        };
        let first_sheets = if added.is_empty() {
            Vec::new()
        } else {
            first_sheets(&added)
        };
        Some(Replaced {
            first_sheets,
            target,
        })
    }

    /// The union of all connections' sets — the vendors the scheduler looks after.
    pub fn union(&self) -> BTreeSet<UsageVendorKey> {
        self.lock()
            .values()
            .flat_map(|entry| entry.vendors.iter().copied())
            .collect()
    }

    /// One target per connection subscribed to `vendor`, carrying that connection's current set; order not fixed. Called on every pump-thread emission — inside the lock it only copies an `Arc`.
    pub fn targets(&self, vendor: UsageVendorKey) -> Vec<Target> {
        self.lock()
            .values()
            .filter(|entry| entry.vendors.contains(&vendor))
            .map(|entry| Target {
                outlet: entry.outlet.clone(),
                subscribed: entry.vendors.clone(),
            })
            .collect()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<ConnId, Entry>> {
        // Tolerates poison — this registry is on the pump thread's emission path. Inside the lock only insert · remove · one set replacement, so cells stay intact: if `first_sheets` panics the new set stands and only that first sheet is lost (the next revision's emission reaches it). A panic holding this lock is outside the containment mark (the pump side is mark-off), so in production the hook ends the process before unwinding — this branch stands only in hook-less test builds (ADR-0292).
        sync::lock(&self.entries)
    }
}

/// Carries one sheet to the targets — ★call with no lock held★. Builds once per distinct `subscribed` among the targets (none → no build) and sends to each target of that set; `None` from building skips only that set's targets. Drops every target it received before returning.
pub fn deliver(snapshot: &UsageLimitSnapshot, targets: Vec<Target>, encoder: &dyn UsageEncoder) {
    // Sets never outnumber connections and production has one shell, so usually 1 — linear grouping.
    type Group = (Arc<BTreeSet<UsageVendorKey>>, Vec<Arc<dyn UsageOutlet>>);
    let mut groups: Vec<Group> = Vec::new();
    for Target { outlet, subscribed } in targets {
        match groups
            .iter_mut()
            .find(|(set, _)| Arc::ptr_eq(set, &subscribed) || **set == *subscribed)
        {
            Some((_, outlets)) => outlets.push(outlet),
            None => groups.push((subscribed, vec![outlet])),
        }
    }
    for (subscribed, outlets) in groups {
        let Some(json) = encoder.encode(snapshot, &subscribed) else {
            continue;
        };
        let frame = UsageFrame {
            snapshot: snapshot.clone(),
            json,
        };
        for outlet in outlets {
            outlet.send(&frame);
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::usage_service::book::{Now, UsageBook};
    use engram_dashboard_agent::backend::usage_probes;
    use engram_dashboard_agent::usage::{UsageAccountKey, UsageKey};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Weak};
    use std::time::Duration;

    // Sets only from prober keys — no vendor literals in tests either.
    fn vendor(i: usize) -> UsageVendorKey {
        usage_probes()[i].key()
    }

    fn set(indices: &[usize]) -> BTreeSet<UsageVendorKey> {
        indices.iter().map(|&i| vendor(i)).collect()
    }

    fn sheet_of(vendor: UsageVendorKey) -> UsageLimitSnapshot {
        let book = UsageBook::new(&usage_probes().map(|p| (p.key(), p.policy())));
        let key = UsageKey {
            vendor,
            account: UsageAccountKey::default(),
        };
        let now = Now {
            mono: Duration::ZERO,
            wall: 1_900_000_000,
        };
        book.snapshot(&key, now).expect("known key")
    }

    fn sheet(i: usize) -> UsageLimitSnapshot {
        sheet_of(vendor(i))
    }

    pub(crate) fn words(subscribed: &BTreeSet<UsageVendorKey>) -> String {
        subscribed
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }

    fn vendors_of(watch: &UsageWatch, conn: ConnId) -> Option<BTreeSet<UsageVendorKey>> {
        watch
            .lock()
            .get(&conn)
            .map(|entry| (*entry.vendors).clone())
    }

    fn no_sheets(_: &[UsageVendorKey]) -> Vec<UsageLimitSnapshot> {
        Vec::new()
    }

    /// A record the test inspects after the outlet has gone into the registry. While the outlet is alive there is one more strong reference — `Arc::strong_count(&log) == 1` means the outlet is destroyed. Service tests use it too.
    #[derive(Default)]
    pub(crate) struct Log {
        pub(crate) frames: Mutex<Vec<UsageFrame>>,
        revoked: AtomicUsize,
        after_revoke: AtomicUsize,
    }

    impl Log {
        pub(crate) fn jsons(&self) -> Vec<String> {
            self.frames
                .lock()
                .unwrap()
                .iter()
                .map(|f| f.json.to_string())
                .collect()
        }

        pub(crate) fn count(&self) -> usize {
            self.frames.lock().unwrap().len()
        }

        pub(crate) fn revokes(&self) -> usize {
            self.revoked.load(Ordering::SeqCst)
        }
    }

    /// Writes the received frame into the [`Log`]; a send after withdrawal is not written but counted separately.
    pub(crate) struct Recording(Arc<Log>);

    pub(crate) fn recording() -> (Recording, Arc<Log>) {
        let log = Arc::new(Log::default());
        (Recording(log.clone()), log)
    }

    impl UsageOutlet for Recording {
        fn send(&self, frame: &UsageFrame) {
            if self.0.revokes() > 0 {
                self.0.after_revoke.fetch_add(1, Ordering::SeqCst);
                return;
            }
            self.0.frames.lock().unwrap().push(frame.clone());
        }

        fn revoke(&self) {
            self.0.revoked.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// JSON = the words of the set; `None` for the `fail_for` set.
    #[derive(Default)]
    pub(crate) struct Counting {
        calls: AtomicUsize,
        fail_for: Option<BTreeSet<UsageVendorKey>>,
    }

    impl Counting {
        pub(crate) fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl UsageEncoder for Counting {
        fn encode(
            &self,
            _snapshot: &UsageLimitSnapshot,
            subscribed: &BTreeSet<UsageVendorKey>,
        ) -> Option<Arc<str>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_for.as_ref() == Some(subscribed) {
                return None;
            }
            Some(Arc::from(words(subscribed)))
        }
    }

    /// One target that did not go through the registry, and its record.
    fn target(subscribed: BTreeSet<UsageVendorKey>) -> (Target, Arc<Log>) {
        let (outlet, log) = recording();
        let target = Target {
            outlet: Arc::new(outlet),
            subscribed: Arc::new(subscribed),
        };
        (target, log)
    }

    // ── Cell lifetime ──

    #[test]
    fn attach_starts_with_an_empty_set() {
        let watch = UsageWatch::new();
        watch.attach(1, recording().0);
        assert_eq!(vendors_of(&watch, 1), Some(BTreeSet::new()));
        assert!(watch.union().is_empty());
        assert!(watch.targets(vendor(0)).is_empty());
    }

    #[test]
    fn replace_swaps_the_whole_set() {
        let watch = UsageWatch::new();
        watch.attach(1, recording().0);
        watch.replace(1, set(&[0, 1]), no_sheets).expect("entry");
        watch.replace(1, set(&[1]), no_sheets).expect("entry");
        assert_eq!(
            vendors_of(&watch, 1),
            Some(set(&[1])),
            "the second overwrites the first"
        );
        watch.replace(1, set(&[0]), no_sheets).expect("entry");
        assert_eq!(vendors_of(&watch, 1), Some(set(&[0])));
    }

    #[test]
    fn an_empty_replace_unsubscribes() {
        let watch = UsageWatch::new();
        watch.attach(1, recording().0);
        watch.replace(1, set(&[0, 1]), no_sheets).expect("entry");
        let replaced = watch.replace(1, BTreeSet::new(), no_sheets).expect("entry");
        assert!(replaced.target.subscribed.is_empty());
        assert!(watch.union().is_empty());
        assert!(watch.targets(vendor(0)).is_empty());
        assert_eq!(
            vendors_of(&watch, 1),
            Some(BTreeSet::new()),
            "the entry remains"
        );
    }

    #[test]
    fn a_replace_after_detach_is_dropped_without_reviving_the_entry() {
        let watch = UsageWatch::new();
        watch.attach(1, recording().0);
        assert!(watch.detach(1));
        let called = AtomicUsize::new(0);
        let late = watch.replace(1, set(&[0]), |_| {
            called.fetch_add(1, Ordering::SeqCst);
            vec![sheet(0)]
        });
        assert!(late.is_none());
        assert_eq!(
            called.load(Ordering::SeqCst),
            0,
            "does not build the first sheet"
        );
        assert_eq!(vendors_of(&watch, 1), None, "the entry does not come back");
        assert!(watch.union().is_empty());
        assert!(watch.targets(vendor(0)).is_empty());
        assert!(
            watch.replace(9, set(&[0]), no_sheets).is_none(),
            "the same for a connection that was never attached"
        );
    }

    #[test]
    fn detach_revokes_the_outlet_once_and_a_held_target_goes_quiet() {
        let watch = UsageWatch::new();
        let (outlet, log) = recording();
        watch.attach(1, outlet);
        watch.replace(1, set(&[0]), no_sheets).expect("entry");
        let held = watch.targets(vendor(0));
        assert_eq!(
            Arc::strong_count(&held[0].outlet),
            2,
            "roster + held target"
        );

        assert!(watch.detach(1));
        assert_eq!(log.revokes(), 1);
        assert_eq!(
            Arc::strong_count(&held[0].outlet),
            1,
            "the copy the roster held was released"
        );

        let encoder = Counting::default();
        deliver(&sheet(0), held, &encoder);
        assert_eq!(log.count(), 0, "a send after reaping is discarded");
        assert_eq!(log.after_revoke.load(Ordering::SeqCst), 1);
        assert_eq!(
            Arc::strong_count(&log),
            1,
            "carrying released the target and the outlet was dropped"
        );

        assert!(!watch.detach(1), "the second detach has no entry");
        assert_eq!(log.revokes(), 1, "reaped once");
    }

    #[test]
    fn an_overwriting_attach_revokes_the_old_outlet_and_keeps_the_new_one_live() {
        let watch = UsageWatch::new();
        let (old, old_log) = recording();
        let (new, new_log) = recording();
        watch.attach(2, old);
        watch.attach(2, new);
        assert_eq!(old_log.revokes(), 1);
        assert_eq!(
            Arc::strong_count(&old_log),
            1,
            "the earlier outlet was dropped"
        );
        assert_eq!(new_log.revokes(), 0);

        watch.replace(2, set(&[0]), no_sheets).expect("entry");
        deliver(&sheet(0), watch.targets(vendor(0)), &Counting::default());
        assert_eq!(
            (old_log.count(), new_log.count()),
            (0, 1),
            "the new outlet lives"
        );

        assert!(watch.detach(2));
        assert_eq!(new_log.revokes(), 1);
        assert_eq!(Arc::strong_count(&new_log), 1);
    }

    /// An outlet that records whether the registry lock was released at withdrawal and at destruction.
    struct LockProbe {
        watch: Weak<UsageWatch>,
        log: Arc<Mutex<Vec<(&'static str, Option<bool>)>>>,
    }

    impl LockProbe {
        fn note(&self, what: &'static str) {
            let free = self
                .watch
                .upgrade()
                .map(|watch| watch.entries.try_lock().is_ok());
            self.log.lock().unwrap().push((what, free));
        }
    }

    impl UsageOutlet for LockProbe {
        fn send(&self, _frame: &UsageFrame) {}

        fn revoke(&self) {
            self.note("revoke");
        }
    }

    impl Drop for LockProbe {
        fn drop(&mut self) {
            self.note("drop");
        }
    }

    #[test]
    fn removed_outlets_are_revoked_then_dropped_outside_the_lock() {
        let watch = Arc::new(UsageWatch::new());
        let expected = [("revoke", Some(true)), ("drop", Some(true))];

        let detached = Arc::new(Mutex::new(Vec::new()));
        watch.attach(
            1,
            LockProbe {
                watch: Arc::downgrade(&watch),
                log: detached.clone(),
            },
        );
        watch.detach(1);
        assert_eq!(*detached.lock().unwrap(), expected);

        let overwritten = Arc::new(Mutex::new(Vec::new()));
        watch.attach(
            2,
            LockProbe {
                watch: Arc::downgrade(&watch),
                log: overwritten.clone(),
            },
        );
        watch.attach(2, recording().0);
        assert_eq!(*overwritten.lock().unwrap(), expected);
    }

    // ── Union · targets ──

    #[test]
    fn union_is_the_union_of_connections() {
        let watch = UsageWatch::new();
        watch.attach(1, recording().0);
        watch.attach(2, recording().0);
        watch.replace(1, set(&[0]), no_sheets).expect("entry");
        watch.replace(2, set(&[1]), no_sheets).expect("entry");
        assert_eq!(watch.union(), set(&[0, 1]));
        watch.detach(1);
        assert_eq!(
            watch.union(),
            set(&[1]),
            "the remaining connection's vendor remains"
        );
    }

    #[test]
    fn targets_are_exactly_the_subscribers_with_their_current_sets() {
        let watch = UsageWatch::new();
        let (a, a_log) = recording();
        let (b, b_log) = recording();
        let (c, c_log) = recording();
        watch.attach(1, a);
        watch.attach(2, b);
        watch.attach(3, c);
        watch.replace(1, set(&[0]), no_sheets).expect("entry");
        watch.replace(2, set(&[0, 1]), no_sheets).expect("entry");

        let encoder = Counting::default();
        deliver(&sheet(0), watch.targets(vendor(0)), &encoder);
        assert_eq!(a_log.jsons(), [words(&set(&[0]))]);
        assert_eq!(b_log.jsons(), [words(&set(&[0, 1]))]);
        assert_eq!(c_log.count(), 0, "a connection that subscribed to nothing");

        deliver(&sheet(1), watch.targets(vendor(1)), &encoder);
        assert_eq!(
            a_log.count(),
            1,
            "a connection that did not subscribe to that vendor"
        );
        assert_eq!(b_log.count(), 2);

        watch.replace(1, set(&[0, 1]), no_sheets).expect("entry");
        let after: Vec<_> = watch
            .targets(vendor(0))
            .into_iter()
            .map(|t| (*t.subscribed).clone())
            .collect();
        assert_eq!(
            after,
            [set(&[0, 1]), set(&[0, 1])],
            "after the change it carries the new set"
        );
    }

    // ── First sheet ──

    #[test]
    fn first_sheets_are_only_for_newly_added_vendors_and_only_for_that_connection() {
        let watch = UsageWatch::new();
        let (mine, mine_log) = recording();
        let (other, other_log) = recording();
        watch.attach(1, mine);
        watch.attach(2, other);
        watch.replace(2, set(&[0, 1]), no_sheets).expect("entry");
        let asked: Mutex<Vec<Vec<UsageVendorKey>>> = Mutex::new(Vec::new());
        let encoder = Counting::default();
        let replace = |vendors: BTreeSet<UsageVendorKey>| {
            let replaced = watch
                .replace(1, vendors, |added| {
                    asked.lock().unwrap().push(added.to_vec());
                    added.iter().map(|&v| sheet_of(v)).collect()
                })
                .expect("entry");
            for first in &replaced.first_sheets {
                deliver(first, vec![replaced.target.clone()], &encoder);
            }
            replaced
        };

        let r = replace(set(&[0]));
        assert_eq!(r.first_sheets.len(), 1);
        assert_eq!(
            *r.target.subscribed,
            set(&[0]),
            "the set after the replacement"
        );
        let r = replace(set(&[0, 1]));
        assert_eq!(
            r.first_sheets.len(),
            1,
            "an unchanged vendor is not built again"
        );
        assert_eq!(*r.target.subscribed, set(&[0, 1]));
        assert_eq!(replace(set(&[0, 1])).first_sheets.len(), 0, "unchanged");
        assert_eq!(replace(set(&[1])).first_sheets.len(), 0, "only removed");

        assert_eq!(*asked.lock().unwrap(), [vec![vendor(0)], vec![vendor(1)]]);
        assert_eq!(
            mine_log.jsons(),
            [words(&set(&[0])), words(&set(&[0, 1]))],
            "each first sheet carries the set after the replacement"
        );
        assert_eq!(other_log.count(), 0, "does not go to other connections");
    }

    #[test]
    fn two_new_vendors_are_asked_in_one_call_in_key_order() {
        let watch = UsageWatch::new();
        watch.attach(1, recording().0);
        let mut expected = vec![vendor(0), vendor(1)];
        expected.sort();
        let asked: Mutex<Vec<Vec<UsageVendorKey>>> = Mutex::new(Vec::new());
        let replaced = watch
            .replace(1, set(&[1, 0]), |added| {
                asked.lock().unwrap().push(added.to_vec());
                added.iter().map(|&v| sheet_of(v)).collect()
            })
            .expect("entry");
        assert_eq!(*asked.lock().unwrap(), [expected]);
        assert_eq!(replaced.first_sheets.len(), 2);
    }

    // ── Carrying ──

    #[test]
    fn deliver_encodes_once_for_one_set_and_releases_the_targets() {
        let (targets, logs): (Vec<_>, Vec<_>) = (0..3).map(|_| target(set(&[0]))).unzip();
        let encoder = Counting::default();
        deliver(&sheet(0), targets, &encoder);
        assert_eq!(encoder.calls(), 1);
        for log in &logs {
            assert_eq!(log.count(), 1);
            assert_eq!(Arc::strong_count(log), 1, "no target copy remains");
        }
        let first = logs[0].frames.lock().unwrap()[0].json.clone();
        let second = logs[1].frames.lock().unwrap()[0].json.clone();
        assert!(Arc::ptr_eq(&first, &second), "shares the same JSON");
    }

    #[test]
    fn deliver_encodes_once_per_distinct_set() {
        let (a, a_log) = target(set(&[0]));
        let (b, b_log) = target(set(&[0, 1]));
        let (c, c_log) = target(set(&[0]));
        let encoder = Counting::default();
        deliver(&sheet(0), vec![a, b, c], &encoder);
        assert_eq!(encoder.calls(), 2);
        assert_eq!(a_log.jsons(), [words(&set(&[0]))]);
        assert_eq!(b_log.jsons(), [words(&set(&[0, 1]))]);
        assert_eq!(c_log.jsons(), [words(&set(&[0]))]);
    }

    #[test]
    fn deliver_without_targets_does_not_encode() {
        let encoder = Counting::default();
        deliver(&sheet(0), Vec::new(), &encoder);
        assert_eq!(encoder.calls(), 0);
    }

    #[test]
    fn a_failed_encoding_skips_only_its_set() {
        let (a, a_log) = target(set(&[0]));
        let (b, b_log) = target(set(&[0, 1]));
        let encoder = Counting {
            fail_for: Some(set(&[0])),
            ..Counting::default()
        };
        deliver(&sheet(0), vec![a, b], &encoder);
        assert_eq!(encoder.calls(), 2);
        assert_eq!(a_log.count(), 0);
        assert_eq!(
            Arc::strong_count(&a_log),
            1,
            "the target of the skipped part is released too"
        );
        assert_eq!(b_log.jsons(), [words(&set(&[0, 1]))]);
    }

    // ── Locks ──

    /// An outlet that calls the registry again while sending — replacement · targets · release (its own connection, so its own `revoke` overlaps).
    struct Reentrant {
        watch: Weak<UsageWatch>,
        conn: ConnId,
        sent: Arc<AtomicUsize>,
        revoked: Arc<AtomicUsize>,
    }

    impl UsageOutlet for Reentrant {
        fn send(&self, _frame: &UsageFrame) {
            let watch = self.watch.upgrade().expect("roster");
            let _ = watch.replace(2, set(&[1]), no_sheets);
            watch.targets(vendor(0));
            watch.detach(self.conn);
            self.sent.fetch_add(1, Ordering::SeqCst);
        }

        fn revoke(&self) {
            self.revoked.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// An encoder that reads the union while building.
    struct UnionReading {
        watch: Weak<UsageWatch>,
        seen: Mutex<Option<BTreeSet<UsageVendorKey>>>,
    }

    impl UsageEncoder for UnionReading {
        fn encode(
            &self,
            _snapshot: &UsageLimitSnapshot,
            subscribed: &BTreeSet<UsageVendorKey>,
        ) -> Option<Arc<str>> {
            *self.seen.lock().unwrap() = Some(self.watch.upgrade().expect("roster").union());
            Some(Arc::from(words(subscribed)))
        }
    }

    #[test]
    fn outlets_and_encoders_may_call_back_into_the_watch() {
        // Another thread, so a deadlock fails instead of hanging.
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let watch = Arc::new(UsageWatch::new());
            let sent = Arc::new(AtomicUsize::new(0));
            let revoked = Arc::new(AtomicUsize::new(0));
            watch.attach(
                1,
                Reentrant {
                    watch: Arc::downgrade(&watch),
                    conn: 1,
                    sent: sent.clone(),
                    revoked: revoked.clone(),
                },
            );
            watch.attach(2, recording().0);
            watch.replace(1, set(&[0]), no_sheets).expect("entry");
            watch.replace(2, set(&[0]), no_sheets).expect("entry");
            let encoder = UnionReading {
                watch: Arc::downgrade(&watch),
                seen: Mutex::new(None),
            };
            deliver(&sheet(0), watch.targets(vendor(0)), &encoder);
            let seen = encoder.seen.lock().unwrap().clone();
            done_tx
                .send((
                    sent.load(Ordering::SeqCst),
                    revoked.load(Ordering::SeqCst),
                    seen,
                    vendors_of(&watch, 1),
                ))
                .unwrap();
        });
        let (sent, revoked, seen, after) = done_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("ends without deadlock");
        assert_eq!(sent, 1);
        assert_eq!(
            revoked, 1,
            "the detach called inside the outlet reaped that outlet"
        );
        assert_eq!(seen, Some(set(&[0])));
        assert_eq!(after, None);
    }

    #[test]
    fn a_poisoned_lock_stays_usable() {
        let watch = UsageWatch::new();
        let (outlet, log) = recording();
        watch.attach(1, outlet);
        let panicked = catch_unwind(AssertUnwindSafe(|| {
            watch.replace(1, set(&[0]), |_| {
                panic!("test — panics while building the first sheet")
            })
        }));
        assert!(panicked.is_err());
        assert!(
            watch.entries.is_poisoned(),
            "the test took the poison branch"
        );

        assert_eq!(watch.union(), set(&[0]), "the new set stays standing");
        watch.replace(1, set(&[1]), no_sheets).expect("entry");
        assert_eq!(watch.targets(vendor(1)).len(), 1);
        watch.attach(2, recording().0);
        assert!(watch.detach(1));
        assert_eq!(log.revokes(), 1);
        assert_eq!(Arc::strong_count(&log), 1);
    }
}
