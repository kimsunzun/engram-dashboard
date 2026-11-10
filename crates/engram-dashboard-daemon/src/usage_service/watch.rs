//! Usage subscription registry — the vendor set each connection subscribed to and that connection's outlet (TRD §1-4 「구독 명부」 (subscription registry) · ADR-0254). [`deliver`], which carries
//! one emission sheet to the targets, is here too.
//!
//! ★There is exactly one place that makes cells, [`UsageWatch::attach`], and exactly one that erases them, [`UsageWatch::detach`]★ —
//!   [`UsageWatch::replace`] only fixes cells that exist. So a late replacement that arrives overlapping disconnect cleanup cannot revive an erased
//!   cell (§3 #56 — the same shape as `attach` · `refuse_if_detached` of the command registry).
//! ★Erasing a cell withdraws its outlet ([`UsageOutlet::revoke`])★ — the connection's frame outlet that the outlet holds is
//!   dropped before [`UsageWatch::detach`] returns (the duty of the connection cleanup port — `conn_port::ConnectionHandler::on_disconnect`),
//!   and any [`Target`] copy left outside goes quiet. What remains is only one temporary copy for each send in progress at that time.
//! ★Never build or send while holding the registry lock★ — inside the lock only the targets (outlet copy + that connection's current set) are taken,
//!   and encoding and sending are done by [`deliver`] without a lock. So there is no deadlock even if the outlet or encoder calls back into this registry.
//! ★Lock order = registry → book★ — the only place inside the registry lock that calls someone else's code is the `first_sheets` of [`UsageWatch::replace`].
//!   Taking the book there is fine and calling the registry again is a deadlock (std `Mutex` is not reentrant). There must be no place that calls the registry
//!   while holding the book.
// ADR-0006

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::conn_port::ConnId;
use engram_dashboard_agent::usage::UsageVendorKey;
use engram_dashboard_base::sync;
use engram_dashboard_protocol::UsageLimitSnapshot;

/// The outlet of one connection — the real one is made fresh per connection by the connection layer in `on_connect`: it holds one cell
/// (`Mutex<Option<Arc<dyn FrameSink>>>`), `send` = lock the cell, take only an `Arc` copy (if present), release, then `try_send` **outside the cell**,
/// and `revoke` = lock the cell, take it out (`take`), and release.
///
/// ★Never send while holding the cell (ADR-0006)★ — the work inside the cell is only copying and taking out: it calls no registry · service · other lock,
///   and logs only after releasing the cell. So `revoke` does not wait for a send in progress.
/// Both methods are called without holding this registry's lock — so on the registry side there is no deadlock even if it calls the registry again inside.
pub trait UsageOutlet: Send + Sync {
    /// ★Must be non-blocking★ — it is called from the pump thread · scheduler · query completion guard, and the connection task (the first sheet of a subscription · emission of the request
    /// path). Failure (saturation · closed) is handled by the outlet itself and is not returned to the caller.
    fn send(&self, frame: &UsageFrame);

    /// Withdraw this outlet — called once per cell when the registry erases a cell (disconnect · another outlet overwrote it with the same connection id), outside the registry
    /// lock. It is the connection cleanup path (sync) so it must not block.
    ///
    /// ★After it returns the outlet holds no frame outlet and `send` does nothing★ — a [`Target`] left outside
    ///   goes quiet. What remains is only one copy taken for each `send` in progress at that time (per thread), and it is dropped after that one `try_send`
    ///   (the temporary copy of TRD §1-4 「동시성·락」 (concurrency · locks) · §3 #87).
    fn revoke(&self);
}

/// One emission sheet built for one `subscribed` set — `json` is the one-line event carrying `snapshot` and that set. Targets of the same
/// set share the same `json`.
#[derive(Debug, Clone)]
pub struct UsageFrame {
    pub snapshot: UsageLimitSnapshot,
    pub json: Arc<str>,
}

/// The port that builds the event JSON — the assembly gives the real one.
///
/// Called outside the registry lock, once per distinct `subscribed` in one emission. `None` = building failed — the targets of that set
/// skip this emission. ★The encoder leaves the failure log itself★ — [`deliver`] does not log.
pub trait UsageEncoder: Send + Sync {
    fn encode(
        &self,
        snapshot: &UsageLimitSnapshot,
        subscribed: &BTreeSet<UsageVendorKey>,
    ) -> Option<Arc<str>>;
}

/// One emission target — the outlet copy of that connection taken inside the registry lock and the set at that time.
///
/// ★The copy goes stale the moment the lock is released★ — `subscribed` can be one step late (the shell's cross-check just sends the same set one more sheet
///   · TRD §1-7), and that connection may already have dropped (the outlet was withdrawn so sending does nothing).
/// ★Pass it to [`deliver`] where it was taken — do not pile them up or hold them across a wait★: while held you would carry a set that goes stale,
///   and the dropped connection's outlet shell would live that long too.
#[derive(Clone)]
pub struct Target {
    pub outlet: Arc<dyn UsageOutlet>,
    pub subscribed: Arc<BTreeSet<UsageVendorKey>>,
}

/// The result of [`UsageWatch::replace`] — `first_sheets` are the first sheets of the vendors newly added by this replacement and `target` is that
/// one connection (`subscribed` = the set **after** replacement). The first sheet is [`deliver`]ed to `target` only.
pub struct Replaced {
    pub first_sheets: Vec<UsageLimitSnapshot>,
    pub target: Target,
}

struct Entry {
    // A replacement sets up a new `Arc` — it is shared across targets rather than copying the set per target.
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

    /// A connection attached (`on_connect`) — make a cell with an empty set. ★This is the only place that makes cells★.
    /// If the same connection id comes again it overwrites the earlier cell and withdraws the earlier outlet (connection ids are not reused — if it does, an error log).
    ///
    /// ★Take the outlet by value and wrap it in an `Arc` here — each cell has its own outlet object★: so withdrawing cannot make another cell's outlet
    ///   quiet (the `revoke` of an erased cell runs after the lock is released and can overlap a cell newly added in between). Do not take an
    ///   `Arc` made outside — a blanket `UsageOutlet` impl for `Arc` · `Box`, or an outlet that shares the cell (the lock) with another outlet
    ///   (a `Clone`able `Arc` wrapper) opens the same path. The real one holds the cell directly.
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
                "같은 연결 id 로 사용량 명부에 두 번 붙었다 — 앞 칸을 덮었다(연결 id 는 재사용되지 않는다)"
            );
            // Withdraw and drop outside the lock — the reason is the same as `detach`.
            previous.outlet.revoke();
        }
    }

    /// A connection dropped (`on_disconnect`) — erase the cell and withdraw its outlet **before returning** ([`UsageOutlet::revoke`]).
    /// So the connection's frame outlet that the outlet holds is dropped inside this call (the duty of the connection cleanup port —
    /// `conn_port::ConnectionHandler::on_disconnect`) and any [`Target`] copy left outside goes quiet. ★This is the only place that erases cells★. Return = there was a cell.
    pub fn detach(&self, conn: ConnId) -> bool {
        // ★Withdrawing and dropping are done **outside** the lock★ — this registry does not know what withdrawing or destruction does (the real one = closing the connection queue sender · waking).
        //   The reason is the same as the command registry `detach`.
        let Some(removed) = self.lock().remove(&conn) else {
            return false;
        };
        removed.outlet.revoke();
        true
    }

    /// `UsageSubscribe` — replace that connection's set whole (empty set = unsubscribe · TRD §3 #54). Even if the set stays the same it is `Some`.
    /// ★If there is no cell (a replacement that arrives late after the drop) it does nothing and `None`★ — it does not revive the cell, and does not even log here:
    /// the log (connection id · debug) is left by the caller outside the lock (the same division as the command registry `refuse_if_detached`).
    ///
    /// ★`first_sheets` is called once, inside the registry lock, only when there is a newly added vendor★ (argument = the newly added vendors, in key order) —
    ///   inside it the book is taken and the first sheet is built (lock order `명부 → 책` (registry → book)). ★Only short in-memory work in the book (`eval_time` · snapshot)
    ///   — no I/O · waiting · calls to this registry★ (calling the registry is a deadlock). While it runs, the pump's and scheduler's [`UsageWatch::targets`]
    ///   are blocked. It is called **after** the set is fixed so 「등록 먼저, 스냅숏 나중」 (register first, snapshot later) holds (§3 #57): an overlapping emission is either
    ///   included in this snapshot or sees this connection as a target with the changed set.
    #[must_use = "첫 한 장은 부르는 쪽이 그 연결에 보낸다"]
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

    /// One target per connection that subscribed to `vendor` — it carries that connection's current set. The order is not fixed.
    /// Called on every emission of the pump thread — inside the lock it only copies an `Arc`.
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
        // Tolerates poison — this registry is on the pump thread's emission path. Changes inside the lock are only insert · remove · replacing a set once
        //   so cells are always intact: if `first_sheets` panics the new set stands and only that first sheet does not go out
        //   (the next revision's emission reaches it). A panic holding this lock is outside the containment mark (the pump side is inside mark-off) so in a production
        //   process the hook ends it before unwinding — a branch that stands only in hook-less test builds (ADR-0292).
        sync::lock(&self.entries)
    }
}

/// Carry one sheet to the targets — ★called while holding no lock★. Build once per distinct `subscribed` the targets carry
/// (if there are no targets, do not build) and send to each target of that set. If building gives `None`, only that set's targets are skipped.
/// It drops every target it received before returning.
pub fn deliver(snapshot: &UsageLimitSnapshot, targets: Vec<Target>, encoder: &dyn UsageEncoder) {
    // The number of sets does not exceed the number of connections and in production it is one shell so it is usually 1 — grouped by linear search.
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

    // Make sets only from prober keys — do not put vendor literals in tests either.
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
        book.snapshot(&key, now).expect("아는 키")
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

    /// A record the test looks into even after the outlet has gone into the registry. While the outlet is alive there is one more strong reference —
    /// `Arc::strong_count(&log) == 1` means the outlet is destroyed. Service tests use it too.
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

    /// Write the frame received into the [`Log`]. A send after withdrawal is not written and is counted separately.
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

    /// JSON = the words of the set. `None` if it is the `fail_for` set.
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

    /// One target that did not go through the registry and its record.
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
        watch.replace(1, set(&[0, 1]), no_sheets).expect("칸");
        watch.replace(1, set(&[1]), no_sheets).expect("칸");
        assert_eq!(
            vendors_of(&watch, 1),
            Some(set(&[1])),
            "두 번째가 첫 번째를 덮는다"
        );
        watch.replace(1, set(&[0]), no_sheets).expect("칸");
        assert_eq!(vendors_of(&watch, 1), Some(set(&[0])));
    }

    #[test]
    fn an_empty_replace_unsubscribes() {
        let watch = UsageWatch::new();
        watch.attach(1, recording().0);
        watch.replace(1, set(&[0, 1]), no_sheets).expect("칸");
        let replaced = watch.replace(1, BTreeSet::new(), no_sheets).expect("칸");
        assert!(replaced.target.subscribed.is_empty());
        assert!(watch.union().is_empty());
        assert!(watch.targets(vendor(0)).is_empty());
        assert_eq!(vendors_of(&watch, 1), Some(BTreeSet::new()), "칸은 남는다");
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
        assert_eq!(called.load(Ordering::SeqCst), 0, "첫 한 장을 뜨지 않는다");
        assert_eq!(vendors_of(&watch, 1), None, "칸이 되살아나지 않는다");
        assert!(watch.union().is_empty());
        assert!(watch.targets(vendor(0)).is_empty());
        assert!(
            watch.replace(9, set(&[0]), no_sheets).is_none(),
            "붙은 적 없는 연결도 같다"
        );
    }

    #[test]
    fn detach_revokes_the_outlet_once_and_a_held_target_goes_quiet() {
        let watch = UsageWatch::new();
        let (outlet, log) = recording();
        watch.attach(1, outlet);
        watch.replace(1, set(&[0]), no_sheets).expect("칸");
        let held = watch.targets(vendor(0));
        assert_eq!(Arc::strong_count(&held[0].outlet), 2, "명부 + 쥔 대상");

        assert!(watch.detach(1));
        assert_eq!(log.revokes(), 1);
        assert_eq!(
            Arc::strong_count(&held[0].outlet),
            1,
            "명부가 쥔 사본이 놓였다"
        );

        let encoder = Counting::default();
        deliver(&sheet(0), held, &encoder);
        assert_eq!(log.count(), 0, "거둔 뒤의 보내기는 버려진다");
        assert_eq!(log.after_revoke.load(Ordering::SeqCst), 1);
        assert_eq!(
            Arc::strong_count(&log),
            1,
            "나르기가 대상을 놓아 출구가 소멸했다"
        );

        assert!(!watch.detach(1), "두 번째 해제는 칸이 없다");
        assert_eq!(log.revokes(), 1, "거두기는 한 번");
    }

    #[test]
    fn an_overwriting_attach_revokes_the_old_outlet_and_keeps_the_new_one_live() {
        let watch = UsageWatch::new();
        let (old, old_log) = recording();
        let (new, new_log) = recording();
        watch.attach(2, old);
        watch.attach(2, new);
        assert_eq!(old_log.revokes(), 1);
        assert_eq!(Arc::strong_count(&old_log), 1, "앞 출구가 소멸했다");
        assert_eq!(new_log.revokes(), 0);

        watch.replace(2, set(&[0]), no_sheets).expect("칸");
        deliver(&sheet(0), watch.targets(vendor(0)), &Counting::default());
        assert_eq!((old_log.count(), new_log.count()), (0, 1), "새 출구는 산다");

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
        watch.replace(1, set(&[0]), no_sheets).expect("칸");
        watch.replace(2, set(&[1]), no_sheets).expect("칸");
        assert_eq!(watch.union(), set(&[0, 1]));
        watch.detach(1);
        assert_eq!(watch.union(), set(&[1]), "남은 연결의 벤더는 남는다");
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
        watch.replace(1, set(&[0]), no_sheets).expect("칸");
        watch.replace(2, set(&[0, 1]), no_sheets).expect("칸");

        let encoder = Counting::default();
        deliver(&sheet(0), watch.targets(vendor(0)), &encoder);
        assert_eq!(a_log.jsons(), [words(&set(&[0]))]);
        assert_eq!(b_log.jsons(), [words(&set(&[0, 1]))]);
        assert_eq!(c_log.count(), 0, "아무것도 구독하지 않은 연결");

        deliver(&sheet(1), watch.targets(vendor(1)), &encoder);
        assert_eq!(a_log.count(), 1, "그 벤더를 구독하지 않은 연결");
        assert_eq!(b_log.count(), 2);

        watch.replace(1, set(&[0, 1]), no_sheets).expect("칸");
        let after: Vec<_> = watch
            .targets(vendor(0))
            .into_iter()
            .map(|t| (*t.subscribed).clone())
            .collect();
        assert_eq!(
            after,
            [set(&[0, 1]), set(&[0, 1])],
            "바뀐 뒤엔 새 집합을 싣는다"
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
        watch.replace(2, set(&[0, 1]), no_sheets).expect("칸");
        let asked: Mutex<Vec<Vec<UsageVendorKey>>> = Mutex::new(Vec::new());
        let encoder = Counting::default();
        let replace = |vendors: BTreeSet<UsageVendorKey>| {
            let replaced = watch
                .replace(1, vendors, |added| {
                    asked.lock().unwrap().push(added.to_vec());
                    added.iter().map(|&v| sheet_of(v)).collect()
                })
                .expect("칸");
            for first in &replaced.first_sheets {
                deliver(first, vec![replaced.target.clone()], &encoder);
            }
            replaced
        };

        let r = replace(set(&[0]));
        assert_eq!(r.first_sheets.len(), 1);
        assert_eq!(*r.target.subscribed, set(&[0]), "교체한 뒤의 집합");
        let r = replace(set(&[0, 1]));
        assert_eq!(r.first_sheets.len(), 1, "그대로인 벤더는 다시 안 뜬다");
        assert_eq!(*r.target.subscribed, set(&[0, 1]));
        assert_eq!(replace(set(&[0, 1])).first_sheets.len(), 0, "그대로");
        assert_eq!(replace(set(&[1])).first_sheets.len(), 0, "빠지기만 함");

        assert_eq!(*asked.lock().unwrap(), [vec![vendor(0)], vec![vendor(1)]]);
        assert_eq!(
            mine_log.jsons(),
            [words(&set(&[0])), words(&set(&[0, 1]))],
            "첫 한 장마다 교체한 뒤의 집합을 싣는다"
        );
        assert_eq!(other_log.count(), 0, "다른 연결에는 안 간다");
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
            .expect("칸");
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
            assert_eq!(Arc::strong_count(log), 1, "대상 사본이 남지 않는다");
        }
        let first = logs[0].frames.lock().unwrap()[0].json.clone();
        let second = logs[1].frames.lock().unwrap()[0].json.clone();
        assert!(Arc::ptr_eq(&first, &second), "같은 JSON 을 나눠 쓴다");
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
        assert_eq!(Arc::strong_count(&a_log), 1, "건너뛴 몫의 대상도 놓는다");
        assert_eq!(b_log.jsons(), [words(&set(&[0, 1]))]);
    }

    // ── Locks ──

    /// An outlet that calls the registry again while sending — replacement · targets · release (its own connection — so its own `revoke` overlaps).
    struct Reentrant {
        watch: Weak<UsageWatch>,
        conn: ConnId,
        sent: Arc<AtomicUsize>,
        revoked: Arc<AtomicUsize>,
    }

    impl UsageOutlet for Reentrant {
        fn send(&self, _frame: &UsageFrame) {
            let watch = self.watch.upgrade().expect("명부");
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
            *self.seen.lock().unwrap() = Some(self.watch.upgrade().expect("명부").union());
            Some(Arc::from(words(subscribed)))
        }
    }

    #[test]
    fn outlets_and_encoders_may_call_back_into_the_watch() {
        // Run on another thread so a deadlock fails instead of hanging.
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
            watch.replace(1, set(&[0]), no_sheets).expect("칸");
            watch.replace(2, set(&[0]), no_sheets).expect("칸");
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
            .expect("교착 없이 끝난다");
        assert_eq!(sent, 1);
        assert_eq!(revoked, 1, "출구 안에서 부른 해제가 그 출구를 거뒀다");
        assert_eq!(seen, Some(set(&[0])));
        assert_eq!(after, None);
    }

    #[test]
    fn a_poisoned_lock_stays_usable() {
        let watch = UsageWatch::new();
        let (outlet, log) = recording();
        watch.attach(1, outlet);
        let panicked = catch_unwind(AssertUnwindSafe(|| {
            watch.replace(1, set(&[0]), |_| panic!("시험 — 첫 한 장을 뜨다 패닉"))
        }));
        assert!(panicked.is_err());
        assert!(watch.entries.is_poisoned(), "시험이 poison 갈래를 탔다");

        assert_eq!(watch.union(), set(&[0]), "새 집합이 선 채 남는다");
        watch.replace(1, set(&[1]), no_sheets).expect("칸");
        assert_eq!(watch.targets(vendor(1)).len(), 1);
        watch.attach(2, recording().0);
        assert!(watch.detach(1));
        assert_eq!(log.revokes(), 1);
        assert_eq!(Arc::strong_count(&log), 1);
    }
}
