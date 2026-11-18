//! Usage limit service — vendor-neutral.
//!
//! ★Vendor names · key lists · policy (cooldown · deadline) do not belong here★ — all of it comes from the probers handed in (real = `engram_dashboard_agent::backend::usage_probes()`). A vendor match or literal in this module violates 「backend extension」.
//!
//! [`UsageService`] holds the book ([`book::UsageBook`]) · the subscription registry ([`watch::UsageWatch`]) · the clock · the encoder · the probers · the scheduler wake sender, and drives pickup application · subscription replacement · requests · query launching · emission (TRD §1-4).
//! ★Lock order = registry → book · the book is a leaf★ — while holding the book, do not touch the registry · outlets · encoder · wake · joiner wake · query thread start.
//! ★Emission is 「apply first, enumerate targets later」★ — after the book lock is released, take targets from the roster and carry them without any lock ([`watch::deliver`]). Paired with 「register first, snapshot later」 of subscription replacement, overlapping replacement and emission make that connection receive at least the book's value (TRD §3 #57).
//! ★Arrival order at one connection is not guaranteed across paths★ — sending is outside the lock, so a smaller revision can arrive late or the same sheet twice. The receiving side keeps only the larger revision per cell (TRD §1-7 · §1-8).
//! ★A query ends the same way regardless of who started it★ — [`ProbeGuard`] closes it and a joined request waits for that end. The query thread holds the service only as a `Weak`.
// ADR-0004
// ADR-0006

pub mod book;
mod bus;
pub mod clock;
#[cfg(test)]
pub(crate) mod fakes;
pub mod observe;
pub mod schedule;
pub mod watch;

use std::collections::{BTreeSet, HashMap};
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use crate::conn_port::ConnId;
use engram_dashboard_agent::usage::{
    ProbeEnv, ProbeError, ProbeFailure, ProbeSpawner, UsageAccountKey, UsageKey, UsageObservation,
    UsageProbe, UsageVendorKey,
};
use engram_dashboard_base::sync;
use engram_dashboard_protocol::UsageLimitSnapshot;

use book::{Coalesce, Judgment, Now, RequestKind, TickPlan, UsageBook};
use clock::UsageClock;
use watch::{deliver, Replaced, UsageEncoder, UsageOutlet, UsageWatch};

/// The cap on how long a request waits for a query end (§3 #16) — under the bus deadline of 7 seconds and the shell reply cap of 30 seconds. If exceeded it answers with cache + `in_flight` and the query continues.
pub const REPLY_WAIT_MAX: Duration = Duration::from_secs(5);

/// Whether the request's answer came from the query that request waited on.
///
/// ★Used only inside the daemon — the wire snapshot does not carry it★: the wire snapshot flows only to broadcasts and the shell cache, where it is always `Cached` (TRD §3 #50·#81). The bus row carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageServed {
    /// The value was obtained from a successful query that this request waited on (or a later one).
    Fresh,
    /// No new value was received during this request (no query · failure · wait cap exceeded) — the value the service was holding.
    Cached,
}

/// The answer to one request — one sheet at the moment of answering.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageAnswer {
    pub snapshot: UsageLimitSnapshot,
    pub served: UsageServed,
}

/// The handle that launches a query thread — real = [`OsProbeThreads`].
pub trait ProbeThreads: Send + Sync {
    /// Runs `run` once on a new thread and returns at once. `Err` = could not launch — the caller closes the query as a failure on the spot. Whether `run` is dropped or held for later, the query is closed only once.
    fn spawn(&self, run: Box<dyn FnOnce() + Send>) -> io::Result<()>;
}

/// A detached std thread named `usage-probe` — not the tokio blocking pool, so runtime drop does not wait, and the child is reaped by the Job with the daemon (§3 #39).
pub struct OsProbeThreads;

impl ProbeThreads for OsProbeThreads {
    fn spawn(&self, run: Box<dyn FnOnce() + Send>) -> io::Result<()> {
        std::thread::Builder::new()
            .name("usage-probe".to_owned())
            .spawn(run)
            .map(drop)
    }
}

/// What [`UsageService::new`] takes — the assembly gives the real ones.
pub struct UsageParts {
    /// One default account cell per prober (key and policy from the prober · duplicate key: first wins · a key that cannot become a wire vendor has no cell — [`UsageBook::new`]).
    pub probes: Vec<&'static dyn UsageProbe>,
    pub spawner: Arc<dyn ProbeSpawner>,
    /// Parent folder in which a temp folder is made per query ([`ProbeEnv::scratch_root`]) — daemon-only.
    pub scratch_root: PathBuf,
    pub threads: Arc<dyn ProbeThreads>,
    pub clock: Arc<dyn UsageClock>,
    pub encoder: Arc<dyn UsageEncoder>,
}

/// What one book lock holds together — the book and each cell's query end record. Joiner registration and query end application stand in turn inside the same lock, so a joined request never waits for the wrong query's end.
struct Desk {
    book: UsageBook,
    runs: HashMap<UsageKey, Runs>,
}

/// One cell's query end record — at most one query is in flight per cell, so the in-flight query is always the `finished + 1`-th end.
#[derive(Default)]
struct Runs {
    finished: u64,
    /// Number of the last successful end (0 = none) — a success after the end waited for makes the answer `Fresh`.
    last_ok: u64,
    /// A bus request waiting for an in-flight query — the completion guard takes it out and wakes it (discarded if the receiver already left).
    blocking: Vec<SyncSender<()>>,
}

/// One per daemon.
///
/// ★Only two requests wait ([`UsageService::request`]·[`UsageService::request_blocking`])★ — everything else is non-blocking (short in-memory locks, `try_send` outlets and wakes, detached query threads), so it is called directly from the pump thread (pickup) and the connection task (subscription replacement).
pub struct UsageService {
    /// Book lock — a leaf.
    desk: Mutex<Desk>,
    watch: UsageWatch,
    probes: Vec<&'static dyn UsageProbe>,
    spawner: Arc<dyn ProbeSpawner>,
    scratch_root: PathBuf,
    threads: Arc<dyn ProbeThreads>,
    /// Finished queries per cell — async joiners wait on it. Follows [`Runs::finished`], never shrinks.
    finished: HashMap<UsageKey, tokio::sync::watch::Sender<u64>>,
    clock: Arc<dyn UsageClock>,
    encoder: Arc<dyn UsageEncoder>,
    /// Scheduler wake — capacity 1. ★No copy goes outside (query thread etc.)★ — otherwise the receiver never sees `Disconnected` after the service is gone and the scheduler does not end.
    wake: SyncSender<()>,
    reply_wait: Duration,
    /// Self-reference for the completion guard — weak, so the query thread does not hold the service.
    me: Weak<Self>,
}

/// After the request decision — answer at once, or wait until that end number.
enum Opened {
    Answer(UsageAnswer),
    Wait(u64),
}

impl UsageService {
    /// The returned receiver is the scheduler thread's share (for [`schedule::spawn_scheduler`]) — at most one wake is pending, and it becomes `Disconnected` when the service is dropped. Without a receiver the service still runs; wakes go into the void.
    pub fn new(parts: UsageParts) -> (Arc<Self>, Receiver<()>) {
        Self::build(parts, REPLY_WAIT_MAX)
    }

    fn build(parts: UsageParts, reply_wait: Duration) -> (Arc<Self>, Receiver<()>) {
        let UsageParts {
            probes,
            spawner,
            scratch_root,
            threads,
            clock,
            encoder,
        } = parts;
        let vendors: Vec<_> = probes.iter().map(|p| (p.key(), p.policy())).collect();
        let book = UsageBook::new(&vendors);
        let keys = book.keys();
        let runs = keys
            .iter()
            .map(|key| (key.clone(), Runs::default()))
            .collect();
        let finished = keys
            .into_iter()
            .map(|key| (key, tokio::sync::watch::channel(0).0))
            .collect();
        let (wake, wakes) = mpsc::sync_channel(1);
        let service = Arc::new_cyclic(|me| Self {
            desk: Mutex::new(Desk { book, runs }),
            watch: UsageWatch::new(),
            probes,
            spawner,
            scratch_root,
            threads,
            finished,
            clock,
            encoder,
            wake,
            reply_wait,
            me: me.clone(),
        });
        (service, wakes)
    }

    /// One pickup observation — called on the spot by the pump thread. No change means neither emission nor wake; a vendor with no cell does nothing.
    pub fn observe(&self, obs: &UsageObservation) {
        let key = account_key(obs.vendor);
        let (applied, coalesce) = {
            let mut desk = self.desk();
            // Time is read inside the book lock so the application order and time order of two pump threads stay consistent.
            let now = self.now();
            let applied = desk.book.apply_passive(obs, now);
            let coalesce = if applied.changed {
                desk.book.coalesce_passive(&key, now)
            } else {
                None
            };
            (applied, coalesce)
        };
        let deferred = matches!(coalesce, Some(Coalesce::Deferred));
        if let Some(Coalesce::PublishNow(sheet)) = coalesce {
            self.publish(key.vendor, &sheet);
        }
        if applied.next_auto_changed || deferred {
            self.wake_scheduler();
        }
    }

    /// `UsageSubscribe` — replaces that connection's whole subscription set (empty = unsubscribe). One connection's replacements are called in arrival order.
    ///
    /// Order: ① time evaluation for each requested vendor — anything stamped (latch · refusal end) goes as one sheet to all current subscribers of that vendor ② registry replacement — for each newly added vendor, the first sheet only to this connection ③ scheduler wake (the first sheet does not repay the debt, and a newly added vendor's automatic query deadline may have passed). A replacement arriving after the drop is discarded — the cell is not revived and nothing is sent to that connection. A vendor with no cell in the book is held only in the registry set and has no sheet.
    pub fn replace_subscription(&self, conn: ConnId, vendors: BTreeSet<UsageVendorKey>) {
        let latched: Vec<(UsageVendorKey, UsageLimitSnapshot)> = {
            let mut desk = self.desk();
            let now = self.now();
            vendors
                .iter()
                .filter_map(|&vendor| {
                    let key = account_key(vendor);
                    if !desk.book.eval_time(&key, now) {
                        return None;
                    }
                    desk.book
                        .broadcast_sheet(&key, now)
                        .map(|sheet| (vendor, sheet))
                })
                .collect()
        };
        for (vendor, sheet) in &latched {
            self.publish(*vendor, sheet);
        }

        // Called inside the registry lock (registry → book) — only the book's in-memory work.
        let replaced = self.watch.replace(conn, vendors, |added| {
            let mut desk = self.desk();
            let now = self.now();
            added
                .iter()
                .filter_map(|&vendor| {
                    let key = account_key(vendor);
                    desk.book.eval_time(&key, now);
                    desk.book.snapshot(&key, now)
                })
                .collect()
        });
        let Some(Replaced {
            first_sheets,
            target,
        }) = replaced
        else {
            tracing::debug!(conn, "a usage subscription replacement for a disconnected connection arrived late — discarding it");
            return;
        };
        for sheet in &first_sheets {
            deliver(sheet, vec![target.clone()], self.encoder.as_ref());
        }
        self.wake_scheduler();
    }

    /// A connection attached (`on_connect`) — registry cell with an empty set ([`UsageWatch::attach`]).
    pub fn attach(&self, conn: ConnId, outlet: impl UsageOutlet + 'static) {
        self.watch.attach(conn, outlet);
    }

    /// A connection dropped (`on_disconnect`) — erase its registry cell and withdraw its outlet before returning ([`UsageWatch::detach`]). Returns whether there was a cell.
    pub fn detach(&self, conn: ConnId) -> bool {
        self.watch.detach(conn)
    }

    /// One ⟳ request (connection task) (§1-4 request table) — `None` = a vendor with no cell.
    ///
    /// Stamp the time evaluation before the decision (latch · refusal end — stamped means one sheet to all subscribers), then answer by the table's row:
    /// - Cache · refusal → one sheet now (`Cached`). ★A refusal is the same branch whether visible or not★ — a visibly refused sheet is `Rejected{retry_in_secs}`; once a value has come and shows `Ready` it is a quiet cache (§3 #88 ⓑ).
    /// - Start → emit one 「refreshing」 sheet to all subscribers, then launch the query · join → attach to the in-flight query. Both wait up to `REPLY_WAIT_MAX` for that query's end, then answer one sheet of the moment — `Fresh` if a query succeeded after the end waited for, else `Cached` (failure → failure state · cap exceeded → `in_flight = true` and the query continues, reaching subscribers by broadcast when it ends).
    pub async fn request(&self, vendor: UsageVendorKey, kind: RequestKind) -> Option<UsageAnswer> {
        let key = account_key(vendor);
        let target = match self.open(&key, kind, None)? {
            Opened::Answer(answer) => return Some(answer),
            Opened::Wait(target) => target,
        };
        // Receiving after the decision does not miss the end — `wait_for` looks from the current value.
        if let Some(done) = self.finished.get(&key) {
            let mut finished = done.subscribe();
            // Whether the cap is exceeded or the end comes, answer with the book now — an overrun answer carries `in_flight`.
            let _ =
                tokio::time::timeout(self.reply_wait, finished.wait_for(|&seen| seen >= target))
                    .await;
        }
        self.close(&key, target)
    }

    /// One request from the bus (`usage.*` — blocking pool thread) — decision and answer as [`UsageService::request`].
    ///
    /// The wait is the std channel's `recv_timeout` — ★no tokio timer★: if the runtime takes down the time driver during shutdown, the timer poll can panic outside the containment mark and end the daemon (hook · ADR-0292 · §3 #17).
    pub fn request_blocking(
        &self,
        vendor: UsageVendorKey,
        kind: RequestKind,
    ) -> Option<UsageAnswer> {
        let key = account_key(vendor);
        let (joined, woken) = mpsc::sync_channel(1);
        let target = match self.open(&key, kind, Some(joined))? {
            Opened::Answer(answer) => return Some(answer),
            Opened::Wait(target) => target,
        };
        // Whether woken or the cap is exceeded, answer with the book now — an overrun answer carries `in_flight`.
        let _ = woken.recv_timeout(self.reply_wait);
        self.close(&key, target)
    }

    /// Front share of a request — inside the book lock: time evaluation · decision · joiner registration (only when waiting); after releasing it: emission → query launch → wake. `joined` = the wake sender of a bus request.
    fn open(
        &self,
        key: &UsageKey,
        kind: RequestKind,
        joined: Option<SyncSender<()>>,
    ) -> Option<Opened> {
        let (judgment, sheet, opened) = {
            let mut guard = self.desk();
            let desk = &mut *guard;
            let now = self.now();
            let latched = desk.book.eval_time(key, now);
            let judgment = desk.book.judge(key, kind, now)?;
            let sheet = if latched || judgment == Judgment::Start {
                desk.book.broadcast_sheet(key, now)
            } else {
                None
            };
            let opened = match judgment {
                Judgment::Cached | Judgment::Rejected => {
                    let snapshot = match &sheet {
                        Some(sheet) => sheet.clone(),
                        None => desk.book.snapshot(key, now)?,
                    };
                    Opened::Answer(UsageAnswer {
                        snapshot,
                        served: UsageServed::Cached,
                    })
                }
                Judgment::Start | Judgment::Join => {
                    let runs = desk.runs.entry(key.clone()).or_default();
                    if let Some(joined) = joined {
                        runs.blocking.push(joined);
                    }
                    Opened::Wait(runs.finished.saturating_add(1))
                }
            };
            (judgment, sheet, opened)
        };
        if let Some(sheet) = &sheet {
            self.publish(key.vendor, sheet);
        }
        if judgment == Judgment::Start {
            self.start_probe(key.clone());
        }
        if sheet.is_some() {
            self.wake_scheduler();
        }
        Some(opened)
    }

    /// Back share of a request — take one sheet of the moment (a latch standing in between is stamped and also emitted to all subscribers). `target` = the end number waited for.
    fn close(&self, key: &UsageKey, target: u64) -> Option<UsageAnswer> {
        let (answer, latched) = {
            let mut guard = self.desk();
            let desk = &mut *guard;
            let now = self.now();
            let latched = if desk.book.eval_time(key, now) {
                desk.book.broadcast_sheet(key, now)
            } else {
                None
            };
            let snapshot = match &latched {
                Some(sheet) => sheet.clone(),
                None => desk.book.snapshot(key, now)?,
            };
            let fresh = desk
                .runs
                .get(key)
                .is_some_and(|runs| runs.last_ok >= target);
            let served = if fresh {
                UsageServed::Fresh
            } else {
                UsageServed::Cached
            };
            (UsageAnswer { snapshot, served }, latched)
        };
        if let Some(sheet) = &latched {
            self.publish(key.vendor, sheet);
            self.wake_scheduler();
        }
        Some(answer)
    }

    /// Carry out one scheduler pass's plan — emit the sheets to all subscribers, then launch the started queries (§1-4 path table; the 「refreshing」 sheet is already in `plan.publish`). Returns `plan.sleep`.
    ///
    /// ★Call after the book lock that took the plan is released★ — holding it is a deadlock (emission takes the registry, and the guard of a query that failed to launch takes the book). The plan's query already has `in_flight` set, so it must be launched here or that cell stays in flight forever.
    fn carry_out(&self, plan: TickPlan) -> Duration {
        let TickPlan {
            start,
            publish,
            sleep,
        } = plan;
        for (key, sheet) in &publish {
            self.publish(key.vendor, sheet);
        }
        for key in start {
            self.start_probe(key);
        }
        sleep
    }

    /// Launch one query for which the book already set `in_flight` — after [`Judgment::Start`] · [`TickPlan::start`], with the book lock released. Non-blocking.
    ///
    /// ★The end comes exactly once★ — the completion guard is made before launching and put in a cell; the job takes it out when it starts running. If the thread cannot be launched it is taken out here (closed as launch failure); if the query panics it is dropped by unwinding (hook-less test build — in production an out-of-mark panic is ended by the hook, ADR-0292). The guard is single, so closing happens once whichever side takes it. The query returns within deadline + `KILL_WAIT` + temp folder deletion retry (about 0.2 seconds) (contract of [`UsageProbe::query`]).
    fn start_probe(&self, key: UsageKey) {
        let vendor = key.vendor;
        let mut guard = ProbeGuard::new(self.me.clone(), key);
        let Some(probe) = self.probe_for(vendor) else {
            // A cell stands only from a prober, so this branch should not come — if it does, close rather than leave in flight.
            tracing::warn!(
                vendor = vendor.as_str(),
                "a query started for a cell with no prober — closing it at once"
            );
            guard.outcome = Err(ProbeError::Unsupported.into());
            return;
        };
        // Do not lean on the thread side dropping a job that failed to launch — until `Ok` the guard can be taken out from this side.
        let slot = Arc::new(Mutex::new(Some(guard)));
        let handed = slot.clone();
        let spawner = self.spawner.clone();
        let scratch_root = self.scratch_root.clone();
        let run = Box::new(move || {
            let Some(mut guard) = take_guard(&handed) else {
                return;
            };
            guard.outcome =
                Err(ProbeError::Io("the usage query ended without an answer".to_owned()).into());
            // An overflowing deadline becomes "now" — the prober returns at once with `Timeout` (no panic).
            let deadline = Instant::now()
                .checked_add(probe.policy().timeout)
                .unwrap_or_else(Instant::now);
            guard.outcome = probe.query(&ProbeEnv {
                spawner: spawner.as_ref(),
                deadline,
                scratch_root: &scratch_root,
            });
        });
        if let Err(e) = self.threads.spawn(run) {
            // Closed with the default result (launch failure) — outside the cell lock.
            drop(take_guard(&slot));
            tracing::warn!(
                vendor = vendor.as_str(),
                error = %e,
                "could not launch the usage query thread — closed the query as failed"
            );
        }
    }

    /// Close one query — only [`ProbeGuard`] calls it (§1-4 「query end」). Inside the book lock: apply the result · end number · one sheet · take out joiners; outside: emit → wake scheduler → wake joiners. Emission happens even if nothing changed (`in_flight` was released).
    fn finish_probe(&self, key: &UsageKey, outcome: Result<UsageObservation, ProbeFailure>) {
        let ok = outcome.is_ok();
        let (sheet, finished, joiners) = {
            let mut guard = self.desk();
            let desk = &mut *guard;
            let now = self.now();
            desk.book.finish_probe(key, outcome, now);
            let sheet = desk.book.broadcast_sheet(key, now);
            let runs = desk.runs.entry(key.clone()).or_default();
            runs.finished = runs.finished.saturating_add(1);
            if ok {
                runs.last_ok = runs.finished;
            }
            (sheet, runs.finished, std::mem::take(&mut runs.blocking))
        };
        if let Some(sheet) = &sheet {
            self.publish(key.vendor, sheet);
        }
        self.wake_scheduler();
        if let Some(done) = self.finished.get(key) {
            // Keep only the larger end number so the value is not rewound if two ends' notifications swap outside the lock.
            done.send_if_modified(|seen| {
                let newer = finished > *seen;
                if newer {
                    *seen = finished;
                }
                newer
            });
        }
        for joiner in joiners {
            // Send only once at capacity 1 — failure = the waiting request has already left.
            let _ = joiner.try_send(());
        }
    }

    /// Carry the sheet whose debt was repaid to all current subscribers of that vendor. ★Called while holding no lock★.
    fn publish(&self, vendor: UsageVendorKey, sheet: &UsageLimitSnapshot) {
        deliver(sheet, self.watch.targets(vendor), self.encoder.as_ref());
    }

    fn wake_scheduler(&self) {
        // `Full` = a wake is already pending · `Disconnected` = no scheduler to receive. Either way the waking side (including the pump thread) does not block.
        let _ = self.wake.try_send(());
    }

    fn probe_for(&self, vendor: UsageVendorKey) -> Option<&'static dyn UsageProbe> {
        self.probes.iter().copied().find(|p| p.key() == vendor)
    }

    fn now(&self) -> Now {
        Now {
            mono: self.clock.mono(),
            wall: self.clock.wall(),
        }
    }

    fn desk(&self) -> MutexGuard<'_, Desk> {
        // Tolerates poison — a panic here would end the daemon in production. Book methods do not panic, and a panic holding this lock is outside the containment mark (the pump side is mark-off), so in production the hook ends the process before unwinding; poison arises only in hook-less test builds (ADR-0292).
        sync::lock(&self.desk)
    }
}

/// Carries the end of one query — wherever it is dropped (query finished · panicked and unwound · thread could not launch so the caller took it out) it closes once with [`UsageService::finish_probe`]. If the service is gone (upgrade fails) the result is discarded.
///
/// ★Holds the service only as a `Weak`★ — a blocked query holding the service would delay service drop → scheduler end (wake `Disconnected`) to the query deadline.
struct ProbeGuard {
    service: Weak<UsageService>,
    key: UsageKey,
    /// The result to apply on drop — if dropped before the query answered, that spot's default failure.
    outcome: Result<UsageObservation, ProbeFailure>,
}

impl ProbeGuard {
    fn new(service: Weak<UsageService>, key: UsageKey) -> Self {
        Self {
            service,
            key,
            outcome: Err(
                ProbeError::Spawn("could not launch the usage query thread".to_owned()).into(),
            ),
        }
    }
}

impl Drop for ProbeGuard {
    fn drop(&mut self) {
        // The guard is dropped only once — the placeholder is never read.
        let outcome = std::mem::replace(&mut self.outcome, Err(ProbeError::Timeout.into()));
        let Some(service) = self.service.upgrade() else {
            tracing::debug!(
                vendor = self.key.vendor.as_str(),
                "the usage service went away first — discarding the query result"
            );
            return;
        };
        service.finish_probe(&self.key, outcome);
    }
}

fn take_guard(slot: &Mutex<Option<ProbeGuard>>) -> Option<ProbeGuard> {
    sync::lock(slot).take()
}

/// A vendor's default account cell (the account word is always the default for now — `UsageAccountKey`).
fn account_key(vendor: UsageVendorKey) -> UsageKey {
    UsageKey {
        vendor,
        account: UsageAccountKey::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_service::clock::ManualUsageClock;
    use crate::usage_service::watch::tests::{recording, words, Counting, Log};
    use crate::usage_service::watch::UsageFrame;
    use engram_dashboard_agent::backend::usage_probes;
    use engram_dashboard_agent::usage::{
        ProbeChild, ProbeCommand, UsagePolicy, UsageSource, WindowObs,
    };
    use engram_dashboard_protocol::{AgentBackendKind, UsageVendorState};
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::mpsc::TryRecvError;
    use std::sync::PoisonError;
    use std::sync::{Barrier, OnceLock};

    const H: i64 = 3_600;
    pub(super) const T0: i64 = 1_900_000_000;
    /// Request wait for tests that must wait for a wake — a vanished wake is exposed at this cap.
    const LONG_WAIT: Duration = Duration::from_secs(10);
    /// Cap on every test-side wait — a deadlock or lost wake fails instead of hanging.
    const BOUND: Duration = Duration::from_secs(10);

    // Cells only from prober keys — no vendor literals in tests either.
    fn vendor(i: usize) -> UsageVendorKey {
        usage_probes()[i].key()
    }

    pub(super) fn key(i: usize) -> UsageKey {
        account_key(vendor(i))
    }

    pub(super) fn set(indices: &[usize]) -> BTreeSet<UsageVendorKey> {
        indices.iter().map(|&i| vendor(i)).collect()
    }

    /// The script for one run of the fake prober.
    pub(super) enum Scripted {
        Answer(Result<UsageObservation, ProbeFailure>),
        Panic,
    }

    /// Fake prober — blocks on each query until it receives one script; ends as a timeout if the sending side ([`Rig`]) disappears. Key and policy come from the real prober. A prober is `'static`, so one leaks per test.
    struct FakeProbe {
        key: UsageVendorKey,
        policy: UsagePolicy,
        queries: AtomicUsize,
        script: Mutex<Receiver<Scripted>>,
    }

    impl UsageProbe for FakeProbe {
        fn key(&self) -> UsageVendorKey {
            self.key
        }

        fn policy(&self) -> UsagePolicy {
            self.policy
        }

        fn query(&self, _env: &ProbeEnv<'_>) -> Result<UsageObservation, ProbeFailure> {
            self.queries.fetch_add(1, Ordering::SeqCst);
            let next = self
                .script
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .recv();
            match next {
                Ok(Scripted::Answer(result)) => result,
                Ok(Scripted::Panic) => panic!("test — prober panic"),
                Err(_) => Err(ProbeError::Timeout.into()),
            }
        }
    }

    /// The fake prober launches no child — calling it is a failure.
    struct NoChildren;

    impl ProbeSpawner for NoChildren {
        fn spawn(
            &self,
            _cmd: &ProbeCommand,
            _deadline: Instant,
        ) -> Result<Box<dyn ProbeChild>, ProbeError> {
            Err(ProbeError::Spawn(
                "test — does not spawn a child".to_owned(),
            ))
        }
    }

    #[derive(Default)]
    struct ThreadCounts {
        spawned: AtomicUsize,
        /// Jobs finished — including the completion guard being dropped.
        ended: AtomicUsize,
        panicked: AtomicUsize,
    }

    /// Query thread stand-in — if `refuse`, it does not launch, holds the received job (`kept`) and returns `Err`. Measures that the service does not lean on a failed-to-launch job being dropped; the test can run the held job late.
    #[derive(Default)]
    struct TestThreads {
        refuse: AtomicBool,
        kept: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
        counts: Arc<ThreadCounts>,
    }

    impl ProbeThreads for TestThreads {
        fn spawn(&self, run: Box<dyn FnOnce() + Send>) -> io::Result<()> {
            if self.refuse.load(Ordering::SeqCst) {
                self.kept.lock().unwrap().push(run);
                return Err(io::Error::other("test — cannot launch the thread"));
            }
            self.counts.spawned.fetch_add(1, Ordering::SeqCst);
            let counts = self.counts.clone();
            std::thread::spawn(move || {
                if catch_unwind(AssertUnwindSafe(run)).is_err() {
                    counts.panicked.fetch_add(1, Ordering::SeqCst);
                }
                counts.ended.fetch_add(1, Ordering::SeqCst);
            });
            Ok(())
        }
    }

    pub(super) struct Rig {
        pub(super) service: Arc<UsageService>,
        pub(super) wakes: Receiver<()>,
        encoder: Arc<Counting>,
        probes: Vec<&'static FakeProbe>,
        scripts: Vec<mpsc::Sender<Scripted>>,
        threads: Arc<TestThreads>,
    }

    /// Cells = the first `count` real probers (fakes borrowing only key and policy).
    fn rig_full(clock: Arc<dyn UsageClock>, count: usize, reply_wait: Duration) -> Rig {
        let (probes, scripts): (Vec<&'static FakeProbe>, Vec<_>) = usage_probes()[..count]
            .iter()
            .map(|real| {
                let (tx, rx) = mpsc::channel();
                let fake: &'static FakeProbe = Box::leak(Box::new(FakeProbe {
                    key: real.key(),
                    policy: real.policy(),
                    queries: AtomicUsize::new(0),
                    script: Mutex::new(rx),
                }));
                (fake, tx)
            })
            .unzip();
        let encoder = Arc::new(Counting::default());
        let threads = Arc::new(TestThreads::default());
        let (service, wakes) = UsageService::build(
            UsageParts {
                probes: probes
                    .iter()
                    .map(|&p| p as &'static dyn UsageProbe)
                    .collect(),
                spawner: Arc::new(NoChildren),
                scratch_root: std::env::temp_dir(),
                threads: threads.clone(),
                clock,
                encoder: encoder.clone(),
            },
            reply_wait,
        );
        Rig {
            service,
            wakes,
            encoder,
            probes,
            scripts,
            threads,
        }
    }

    pub(super) fn rig_with(clock: Arc<dyn UsageClock>, count: usize) -> Rig {
        rig_full(clock, count, LONG_WAIT)
    }

    pub(super) fn rig() -> (Rig, Arc<ManualUsageClock>) {
        let clock = Arc::new(ManualUsageClock::new(Duration::from_secs(100), T0));
        (rig_with(clock.clone(), usage_probes().len()), clock)
    }

    impl Rig {
        /// Attach and subscribe — first sheet and wake left empty.
        pub(super) fn subscriber(&self, conn: ConnId, indices: &[usize]) -> Arc<Log> {
            let (outlet, log) = recording();
            self.service.attach(conn, outlet);
            self.service.replace_subscription(conn, set(indices));
            log.frames.lock().unwrap().clear();
            self.drain_wakes();
            log
        }

        fn drain_wakes(&self) -> usize {
            self.wakes.try_iter().count()
        }

        fn revision(&self, i: usize) -> u64 {
            self.service
                .desk()
                .book
                .revision(&key(i))
                .expect("known key")
        }

        fn wire(&self, i: usize) -> AgentBackendKind {
            self.sheet(i).vendor
        }

        fn sheet(&self, i: usize) -> UsageLimitSnapshot {
            let now = self.service.now();
            self.service
                .desk()
                .book
                .snapshot(&key(i), now)
                .expect("known key")
        }

        pub(super) fn answer(&self, i: usize, scripted: Scripted) {
            self.scripts[i].send(scripted).expect("fake prober");
        }

        pub(super) fn queries(&self, i: usize) -> usize {
            self.probes[i].queries.load(Ordering::SeqCst)
        }

        /// Requests waiting for that cell's in-flight query — bus + async.
        pub(super) fn joiners(&self, i: usize) -> usize {
            let blocking = self.service.desk().runs[&key(i)].blocking.len();
            blocking + self.service.finished[&key(i)].receiver_count()
        }

        pub(super) fn in_flight(&self, i: usize) -> bool {
            self.service
                .desk()
                .book
                .in_flight(&key(i))
                .expect("known key")
        }
    }

    pub(super) fn five(i: usize, pct: f64, reset: i64) -> UsageObservation {
        UsageObservation {
            vendor: vendor(i),
            five_hour: Some(WindowObs {
                used_pct: Some(pct),
                resets_at: Some(reset),
            }),
            weekly: None,
            model_scoped: None,
            plan: None,
            source: UsageSource::Passive,
            limits_unavailable: None,
        }
    }

    pub(super) fn far_reset() -> i64 {
        T0 + 5 * H
    }

    fn both(i: usize, five_pct: f64, weekly_pct: f64) -> UsageObservation {
        UsageObservation {
            weekly: Some(WindowObs {
                used_pct: Some(weekly_pct),
                resets_at: Some(T0 + 100 * H),
            }),
            ..five(i, five_pct, far_reset())
        }
    }

    pub(super) fn frames(log: &Log) -> Vec<UsageFrame> {
        log.frames.lock().unwrap().clone()
    }

    pub(super) fn last_pct(log: &Log) -> Option<f64> {
        frames(log)
            .last()
            .and_then(|f| f.snapshot.five_hour.as_ref())
            .and_then(|w| w.used_pct)
    }

    // ── Pickup ──

    #[test]
    fn an_observed_change_goes_to_that_vendors_subscribers_only_and_a_repeat_sends_nothing() {
        let (rig, _clock) = rig();
        let a = rig.subscriber(1, &[0]);
        let b = rig.subscriber(2, &[1]);
        let c = rig.subscriber(3, &[]);

        rig.service.observe(&five(0, 40.0, far_reset()));
        let got = frames(&a);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].snapshot.revision, rig.revision(0));
        assert_eq!(&*got[0].json, words(&set(&[0])));
        assert_eq!((b.count(), c.count()), (0, 0));

        rig.service.observe(&five(0, 40.0, far_reset()));
        assert_eq!(
            a.count(),
            1,
            "picked up the same value again — not a change"
        );
    }

    #[test]
    fn without_subscribers_the_book_still_moves_and_nothing_is_encoded() {
        let (rig, _clock) = rig();
        let before = rig.revision(0);
        rig.service.observe(&five(0, 40.0, far_reset()));
        assert!(rig.revision(0) > before);
        assert_eq!(rig.encoder.calls(), 0);
    }

    #[test]
    fn a_change_inside_the_coalesce_window_waits_for_the_scheduler() {
        let (rig, clock) = rig();
        let a = rig.subscriber(1, &[0]);
        rig.service.observe(&five(0, 40.0, far_reset()));
        rig.drain_wakes();

        clock.advance_both(Duration::from_millis(500));
        rig.service.observe(&five(0, 41.0, far_reset()));
        assert_eq!(a.count(), 1, "a second change within 1 second is merged");
        assert_eq!(
            rig.drain_wakes(),
            1,
            "the closing emit is done by the scheduler"
        );
    }

    #[test]
    fn an_observation_that_moves_the_next_auto_query_wakes_the_scheduler_once() {
        let (rig, clock) = rig();
        let a = rig.subscriber(1, &[0]);
        {
            let now = rig.service.now();
            let mut desk = rig.service.desk();
            let book = &mut desk.book;
            assert!(book.begin_probe(&key(0)));
            book.finish_probe(&key(0), Ok(both(0, 10.0, 10.0)), now);
            book.broadcast_sheet(&key(0), now);
        }

        clock.advance_both(Duration::from_secs(2));
        rig.service.observe(&five(0, 20.0, far_reset()));
        assert_eq!(
            (a.count(), rig.drain_wakes()),
            (1, 0),
            "only one window was newly set — the next automatic query stays"
        );

        clock.advance_both(Duration::from_secs(2));
        let now = rig.service.now();
        let before = rig.service.desk().book.next_auto(&key(0), now);
        rig.service.observe(&both(0, 30.0, 30.0));
        assert_eq!(
            (a.count(), rig.drain_wakes()),
            (2, 1),
            "two windows were newly set, so the cooldown origin moved (D12)"
        );
        assert_ne!(rig.service.desk().book.next_auto(&key(0), now), before);
    }

    #[test]
    fn an_observation_still_applies_after_the_book_lock_is_poisoned() {
        let (rig, _clock) = rig();
        let a = rig.subscriber(1, &[0]);
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _held = rig.service.desk.lock().unwrap();
            panic!("test — panics while holding the book lock");
        }));
        assert!(panicked.is_err());
        assert!(
            rig.service.desk.is_poisoned(),
            "the test took the poison branch"
        );

        rig.service.observe(&five(0, 40.0, far_reset()));
        assert_eq!(last_pct(&a), Some(40.0));
    }

    // If the query start (a debt to repay at once) has not been emitted yet, a pickup within 1 second emits that too.
    #[test]
    fn a_passive_right_after_an_unpublished_probe_start_publishes_at_once() {
        let (rig, clock) = rig();
        let a = rig.subscriber(1, &[0]);
        rig.service.observe(&five(0, 40.0, far_reset()));
        assert!(rig.service.desk().book.begin_probe(&key(0)));

        clock.advance_both(Duration::from_millis(500));
        rig.service.observe(&five(0, 42.0, far_reset()));
        let got = frames(&a);
        assert_eq!(got.len(), 2);
        assert!(got[1].snapshot.in_flight);
        assert_eq!(last_pct(&a), Some(42.0));
        assert_eq!(got[1].snapshot.revision, rig.revision(0));
    }

    #[test]
    fn a_vendor_without_a_cell_gets_no_sheet_and_its_observation_does_nothing() {
        let clock = Arc::new(ManualUsageClock::new(Duration::from_secs(100), T0));
        let rig = rig_with(clock, 1);
        let (outlet, log) = recording();
        rig.service.attach(1, outlet);
        rig.service.replace_subscription(1, set(&[0, 1]));
        let got = frames(&log);
        assert_eq!(
            got.len(),
            1,
            "a first sheet only for vendors that have a cell"
        );
        assert_eq!(&*got[0].json, words(&set(&[0, 1])), "the set as requested");
        rig.drain_wakes();

        rig.service.observe(&five(1, 40.0, far_reset()));
        assert_eq!(log.count(), 1);
        assert_eq!(rig.drain_wakes(), 0);
    }

    // ── Subscription replacement ──

    #[test]
    fn a_latch_due_at_subscription_is_published_to_existing_subscribers_too() {
        let (rig, clock) = rig();
        let a = rig.subscriber(1, &[0]);
        rig.service.observe(&five(0, 40.0, T0 + 10));
        assert_eq!(a.count(), 1);
        clock.set_wall(T0 + 20);

        let (outlet, b) = recording();
        rig.service.attach(2, outlet);
        rig.service.replace_subscription(2, set(&[0]));

        let latched = frames(&a);
        assert_eq!(
            latched.len(),
            2,
            "one latch sheet to the existing subscriber"
        );
        let sheet = &latched[1].snapshot;
        assert!(sheet.five_hour.as_ref().expect("window").expired);
        assert_eq!(sheet.revision, rig.revision(0));
        let first = frames(&b);
        assert_eq!(
            first.len(),
            1,
            "the new subscriber gets only the first sheet"
        );
        assert_eq!(first[0].snapshot, *sheet);
        assert_eq!(rig.drain_wakes(), 1);

        rig.service.replace_subscription(1, set(&[0]));
        rig.service.replace_subscription(2, set(&[0]));
        assert_eq!(
            (a.count(), b.count()),
            (2, 1),
            "replacement with the same set — not a single sheet"
        );
    }

    #[test]
    fn a_first_sheet_goes_to_the_new_subscriber_only_and_wakes_the_scheduler() {
        let (rig, _clock) = rig();
        let a = rig.subscriber(1, &[0]);
        rig.service.observe(&five(0, 40.0, far_reset()));
        let (outlet, b) = recording();
        rig.service.attach(2, outlet);
        rig.drain_wakes();

        rig.service.replace_subscription(2, set(&[0, 1]));
        assert_eq!(a.count(), 1);
        let first = frames(&b);
        assert_eq!(first.len(), 2, "one sheet per newly added vendor");
        assert!(first.iter().all(|f| &*f.json == words(&set(&[0, 1]))));
        assert_eq!(rig.drain_wakes(), 1);
    }

    #[test]
    fn a_late_replace_after_detach_is_dropped() {
        let (rig, _clock) = rig();
        let (outlet, log) = recording();
        rig.service.attach(1, outlet);
        assert!(rig.service.detach(1));
        assert_eq!(log.revokes(), 1);
        rig.service.observe(&five(0, 40.0, far_reset()));
        rig.drain_wakes();

        rig.service.replace_subscription(1, set(&[0]));
        assert_eq!(log.count(), 0);
        assert_eq!(Arc::strong_count(&log), 1, "the outlet did not come back");
        assert!(rig.service.watch.union().is_empty());
        assert_eq!(rig.drain_wakes(), 0);
        assert_eq!(rig.encoder.calls(), 0);
    }

    #[test]
    fn a_late_replace_after_detach_still_publishes_a_due_latch_to_the_other_subscribers() {
        let (rig, clock) = rig();
        let other = rig.subscriber(2, &[0]);
        let (outlet, gone) = recording();
        rig.service.attach(1, outlet);
        assert!(rig.service.detach(1));
        rig.service.observe(&five(0, 40.0, T0 + 10));
        clock.set_wall(T0 + 20);

        rig.service.replace_subscription(1, set(&[0]));
        let got = frames(&other);
        assert_eq!(got.len(), 2, "one pickup sheet + one latch sheet");
        assert!(got[1].snapshot.five_hour.as_ref().expect("window").expired);
        assert_eq!(gone.count(), 0, "nothing for the disconnected connection");
    }

    #[test]
    fn adding_a_vendor_while_another_latches_sends_the_latch_first_then_the_first_sheet() {
        let (rig, clock) = rig();
        let a = rig.subscriber(1, &[0]);
        rig.service.observe(&five(0, 40.0, T0 + 10));
        a.frames.lock().unwrap().clear();
        clock.set_wall(T0 + 20);

        rig.service.replace_subscription(1, set(&[0, 1]));
        let got = frames(&a);
        let order: Vec<_> = got
            .iter()
            .map(|f| (f.snapshot.vendor, f.json.to_string()))
            .collect();
        assert_eq!(
            order,
            [
                (rig.wire(0), words(&set(&[0]))),
                (rig.wire(1), words(&set(&[0, 1]))),
            ],
            "latch (set before replacement) then the first sheet (set after replacement)"
        );
        assert!(got[0].snapshot.five_hour.as_ref().expect("window").expired);
    }

    // ── Locks ──

    /// An outlet that calls back into the service when first received — pickup · replacement · self-release.
    struct CallsBack {
        service: Arc<OnceLock<Weak<UsageService>>>,
        calls: Arc<AtomicUsize>,
    }

    impl UsageOutlet for CallsBack {
        fn send(&self, _frame: &UsageFrame) {
            if self.calls.fetch_add(1, Ordering::SeqCst) > 0 {
                return;
            }
            let service = self.service.get().and_then(Weak::upgrade).expect("service");
            service.observe(&five(0, 77.0, far_reset()));
            service.replace_subscription(1, set(&[0, 1]));
            service.detach(1);
        }

        fn revoke(&self) {}
    }

    #[test]
    fn an_outlet_may_call_back_into_the_service() {
        // Another thread, so a deadlock fails instead of hanging.
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (rig, _clock) = rig();
            let slot = Arc::new(OnceLock::new());
            slot.set(Arc::downgrade(&rig.service)).expect("once");
            let calls = Arc::new(AtomicUsize::new(0));
            rig.service.attach(
                1,
                CallsBack {
                    service: slot,
                    calls: calls.clone(),
                },
            );
            rig.service.replace_subscription(1, set(&[0]));
            let union = rig.service.watch.union();
            done_tx.send((calls.load(Ordering::SeqCst), union)).unwrap();
        });
        let (calls, union) = done_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("ends without deadlock");
        assert!(calls >= 1);
        assert!(
            union.is_empty(),
            "the detach called inside the outlet cleared the entry"
        );
    }

    // ── Concurrency ──

    /// A clock whose monotonic time flows 2 seconds per read — every change finds the coalescing window closed and goes out at once (no scheduler here to emit the end emission). The wall clock is stopped so no latch stands.
    struct Ticking(AtomicU64);

    impl UsageClock for Ticking {
        fn mono(&self) -> Duration {
            Duration::from_secs(self.0.fetch_add(2, Ordering::SeqCst))
        }

        fn wall(&self) -> i64 {
            T0
        }
    }

    /// One round: two connections change subscriptions while two pickups change two vendors; overlap differs per round, the amount of work is fixed. Connection 0 changes the fixed amount, empties, and resubscribes after the round. Connection 1 changes the fixed amount, empties, and resubscribes at the same moment as pickup 0's last two changes — whether the last value arrives by emission or by the first sheet depends on that overlap (where the two passes of TRD §3 #57 interlock). Returns one line per mismatched cell.
    fn race_trial(trial: usize) -> Vec<String> {
        const OBSERVERS: usize = 2;
        const OBSERVATIONS: usize = 16;
        const REPLACES: usize = 8;
        const CHURN: usize = 24;
        let rig = rig_with(Arc::new(Ticking(AtomicU64::new(100))), 2);
        let logs: Vec<Arc<Log>> = (0..2)
            .map(|conn| {
                let (outlet, log) = recording();
                rig.service.attach(conn, outlet);
                log
            })
            .collect();
        let cycle: [&[usize]; 5] = [&[0], &[0, 1], &[1], &[], &[1, 0]];
        let barrier = Arc::new(Barrier::new(2 + OBSERVERS));
        let endgame = Arc::new(Barrier::new(2));

        let mut threads = Vec::new();
        {
            let (service, barrier) = (rig.service.clone(), barrier.clone());
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                for round in 0..CHURN {
                    service.replace_subscription(0, set(cycle[round % cycle.len()]));
                }
                service.replace_subscription(0, set(&[]));
            }));
        }
        {
            let (service, barrier, endgame) =
                (rig.service.clone(), barrier.clone(), endgame.clone());
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                for round in 0..REPLACES {
                    service.replace_subscription(1, set(cycle[(round + 1) % cycle.len()]));
                }
                service.replace_subscription(1, set(&[]));
                endgame.wait();
                service.replace_subscription(1, set(&[0, 1]));
            }));
        }
        for observer in 0..OBSERVERS {
            let (service, barrier, endgame) =
                (rig.service.clone(), barrier.clone(), endgame.clone());
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                for n in 0..OBSERVATIONS {
                    let pct = ((observer * 7_919 + n * 13) % 1_000) as f64 / 10.0;
                    service.observe(&five(n % 2, pct, far_reset()));
                }
                if observer == 0 {
                    // Preceding values are all below 100, so it is always a change.
                    endgame.wait();
                    for i in 0..2 {
                        service.observe(&five(i, 100.0, far_reset()));
                    }
                }
            }));
        }
        for thread in threads {
            thread.join().expect("test thread");
        }
        rig.service.replace_subscription(0, set(&[0, 1]));

        let mut misses = Vec::new();
        for i in 0..2 {
            let (wire, last) = (rig.wire(i), rig.revision(i));
            for (conn, log) in logs.iter().enumerate() {
                let highest = frames(log)
                    .iter()
                    .filter(|f| f.snapshot.vendor == wire)
                    .map(|f| f.snapshot.revision)
                    .max();
                if last == 0 || highest != Some(last) {
                    misses.push(format!(
                        "trial {trial} · connection {conn} · vendor {i}: highest received {highest:?} · book {last}"
                    ));
                }
            }
        }
        misses
    }

    /// Per round, the highest revision a connection received = the book's last revision; holds in correct code regardless of overlap (clock flows 2 seconds per read, wall clock stopped).
    #[test]
    fn racing_replaces_and_observations_leave_each_subscriber_at_the_final_revision() {
        const TRIALS: usize = 2_000;
        // Another thread, so a lock-order deadlock fails instead of hanging.
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let misses: Vec<String> = (0..TRIALS).flat_map(race_trial).collect();
            done_tx.send(misses).unwrap();
        });
        let misses = done_rx
            .recv_timeout(Duration::from_secs(120))
            .expect("the test thread did not end (deadlock or panic)");
        let total = misses.len();
        assert!(
            misses.is_empty(),
            "{total} mismatches — first: {:?}",
            misses.first()
        );
    }

    // ── Requests · queries ──

    /// Request wait for tests that measure the wait cap.
    const SHORT_WAIT: Duration = Duration::from_millis(200);

    pub(super) fn active(i: usize, pct: f64) -> UsageObservation {
        UsageObservation {
            source: UsageSource::Active,
            ..five(i, pct, far_reset())
        }
    }

    pub(super) fn ok(obs: UsageObservation) -> Scripted {
        Scripted::Answer(Ok(obs))
    }

    /// Poll while resting briefly until the condition holds — fails past [`BOUND`].
    pub(super) fn wait_until(what: &str, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + BOUND;
        while !ready() {
            assert!(Instant::now() < deadline, "{what} — wait cap");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A bus request on another thread — the receiving side's cap ([`answered`]) turns a hang into a failure.
    pub(super) fn bus_request(
        service: &Arc<UsageService>,
        i: usize,
        kind: RequestKind,
    ) -> Receiver<Option<UsageAnswer>> {
        let (tx, rx) = mpsc::channel();
        let service = service.clone();
        std::thread::spawn(move || {
            let _ = tx.send(service.request_blocking(vendor(i), kind));
        });
        rx
    }

    pub(super) fn answered(rx: &Receiver<Option<UsageAnswer>>) -> UsageAnswer {
        rx.recv_timeout(BOUND)
            .expect("the request ends within the cap")
            .expect("known vendor")
    }

    pub(super) fn pct(sheet: &UsageLimitSnapshot) -> Option<f64> {
        sheet.five_hour.as_ref().and_then(|w| w.used_pct)
    }

    fn failed_kind(sheet: &UsageLimitSnapshot) -> Option<String> {
        match &sheet.state {
            UsageVendorState::Failed { detail, .. } => detail.as_ref().map(|d| d.kind.clone()),
            _ => None,
        }
    }

    fn ended(rig: &Rig) -> usize {
        rig.threads.counts.ended.load(Ordering::SeqCst)
    }

    #[test]
    fn a_probe_thread_that_cannot_start_closes_the_probe_as_a_spawn_failure_and_publishes() {
        let (rig, _clock) = rig();
        let log = rig.subscriber(1, &[0]);
        rig.threads.refuse.store(true, Ordering::SeqCst);

        let started = Instant::now();
        let answer = rig
            .service
            .request_blocking(vendor(0), RequestKind::Refresh)
            .expect("known vendor");
        assert!(
            started.elapsed() < LONG_WAIT,
            "joiners of a closed query wake at once"
        );
        assert_eq!(answer.served, UsageServed::Cached);
        assert!(!answer.snapshot.in_flight);
        assert_eq!(failed_kind(&answer.snapshot).as_deref(), Some("spawn"));
        assert!(!rig.in_flight(0), "does not stay in progress");
        assert_eq!(rig.queries(0), 0);

        let got = frames(&log);
        assert_eq!(got.len(), 2, "one start sheet + one end sheet");
        assert!(got[0].snapshot.in_flight);
        assert!(!got[1].snapshot.in_flight);
        assert_eq!(got[1].snapshot.revision, answer.snapshot.revision);

        let kept: Vec<_> = rig.threads.kept.lock().unwrap().drain(..).collect();
        assert_eq!(
            kept.len(),
            1,
            "the stand-in held the job it could not launch"
        );
        for job in kept {
            job();
        }
        assert_eq!(log.count(), 2, "the job that ran late does nothing");
        assert_eq!(rig.queries(0), 0);
        assert_eq!(rig.service.desk().runs[&key(0)].finished, 1, "closed once");
    }

    #[test]
    fn concurrent_requests_join_one_probe_and_all_get_its_answer() {
        let (rig, _clock) = rig();
        let first = bus_request(&rig.service, 0, RequestKind::Refresh);
        wait_until("the query started", || rig.queries(0) == 1);

        let buses: Vec<_> = [RequestKind::Get, RequestKind::Refresh, RequestKind::Get]
            .into_iter()
            .map(|kind| bus_request(&rig.service, 0, kind))
            .collect();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_time()
            .build()
            .expect("runtime");
        let tasks: Vec<_> = [RequestKind::Refresh, RequestKind::Get]
            .into_iter()
            .map(|kind| {
                let service = rig.service.clone();
                runtime.spawn(async move { service.request(vendor(0), kind).await })
            })
            .collect();
        wait_until("all six requests joined", || rig.joiners(0) == 6);

        rig.answer(0, ok(active(0, 33.0)));
        let mut answers = vec![answered(&first)];
        answers.extend(buses.iter().map(answered));
        for task in tasks {
            let answer = runtime
                .block_on(async { tokio::time::timeout(BOUND, task).await })
                .expect("ends within the cap")
                .expect("request task")
                .expect("known vendor");
            answers.push(answer);
        }
        assert_eq!(
            (
                rig.queries(0),
                rig.threads.counts.spawned.load(Ordering::SeqCst)
            ),
            (1, 1),
            "one query"
        );
        for answer in &answers {
            assert_eq!(answer.served, UsageServed::Fresh);
            assert_eq!(pct(&answer.snapshot), Some(33.0));
            assert!(!answer.snapshot.in_flight);
        }
    }

    #[test]
    fn a_request_gives_up_after_the_reply_wait_with_the_cache_and_in_flight_and_the_probe_goes_on()
    {
        let clock = Arc::new(ManualUsageClock::new(Duration::from_secs(100), T0));
        let rig = rig_full(clock, 2, SHORT_WAIT);
        let log = rig.subscriber(1, &[0]);

        let started = Instant::now();
        let answer = rig
            .service
            .request_blocking(vendor(0), RequestKind::Refresh)
            .expect("known vendor");
        assert!(started.elapsed() >= SHORT_WAIT);
        assert!(started.elapsed() < LONG_WAIT);
        assert_eq!(answer.served, UsageServed::Cached);
        assert!(answer.snapshot.in_flight, "the query continues");

        // async has the same cap — it joins an in-flight query, waits, and gives up.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime");
        let answer = runtime
            .block_on(rig.service.request(vendor(0), RequestKind::Get))
            .expect("known vendor");
        assert_eq!(answer.served, UsageServed::Cached);
        assert!(answer.snapshot.in_flight);
        assert_eq!(rig.queries(0), 1);

        rig.answer(0, ok(active(0, 55.0)));
        wait_until("the query end reaches the subscriber", || {
            last_pct(&log) == Some(55.0)
        });
        assert!(!rig.in_flight(0));
        assert!(!frames(&log).last().expect("one sheet").snapshot.in_flight);
    }

    #[test]
    fn a_probe_outliving_the_service_drops_its_result_without_a_panic() {
        let (rig, _clock) = rig();
        assert!(rig.service.desk().book.begin_probe(&key(0)));
        rig.service.start_probe(key(0));
        wait_until("the query started", || rig.queries(0) == 1);

        let Rig {
            service,
            wakes,
            scripts,
            threads,
            ..
        } = rig;
        let weak = Arc::downgrade(&service);
        drop(service);
        assert!(
            weak.upgrade().is_none(),
            "the query thread does not hold on to the service"
        );
        wakes.try_iter().count();
        assert!(
            matches!(wakes.try_recv(), Err(TryRecvError::Disconnected)),
            "the scheduler sees the end of the service even while a blocked query is running"
        );

        scripts[0].send(ok(active(0, 10.0))).expect("fake prober");
        wait_until("the query thread ended", || {
            threads.counts.ended.load(Ordering::SeqCst) == 1
        });
        assert_eq!(threads.counts.panicked.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_refresh_during_a_rejection_answers_from_the_cache_without_a_probe_seen_or_hidden() {
        let (rig, _clock) = rig();
        assert!(rig.service.desk().book.begin_probe(&key(0)));
        rig.service.start_probe(key(0));
        rig.answer(
            0,
            Scripted::Answer(Err(ProbeError::RateLimited {
                retry_after: Some(Duration::from_secs(600)),
            }
            .into())),
        );
        wait_until("a query that ended in refusal", || ended(&rig) == 1);

        let seen = rig
            .service
            .request_blocking(vendor(0), RequestKind::Refresh)
            .expect("known vendor");
        assert!(matches!(
            seen.snapshot.state,
            UsageVendorState::Rejected { .. }
        ));
        assert_eq!(seen.served, UsageServed::Cached);

        rig.service.observe(&five(0, 40.0, far_reset()));
        let hidden = rig
            .service
            .request_blocking(vendor(0), RequestKind::Refresh)
            .expect("known vendor");
        assert!(
            matches!(hidden.snapshot.state, UsageVendorState::Ready),
            "a value came, so the refusal is not visible"
        );
        assert_eq!(hidden.served, UsageServed::Cached);
        assert!(!hidden.snapshot.in_flight);
        assert_eq!(pct(&hidden.snapshot), Some(40.0));
        assert_eq!(
            rig.threads.counts.spawned.load(Ordering::SeqCst),
            1,
            "no query went out after the refused query"
        );
    }

    #[test]
    fn a_probe_start_reaches_subscribers_before_its_result() {
        let (rig, _clock) = rig();
        let log = rig.subscriber(1, &[0]);
        let other = rig.subscriber(2, &[1]);
        let pending = bus_request(&rig.service, 0, RequestKind::Refresh);
        wait_until("one 「refreshing」 sheet", || log.count() == 1);
        let start = frames(&log)[0].snapshot.clone();
        assert!(start.in_flight);
        assert_eq!(pct(&start), None);

        rig.answer(0, ok(active(0, 61.0)));
        let answer = answered(&pending);
        assert_eq!(answer.served, UsageServed::Fresh);
        let got = frames(&log);
        assert_eq!(
            got.len(),
            2,
            "the end sheet went out before the joiner woke"
        );
        let end = &got[1].snapshot;
        assert!(!end.in_flight);
        assert!(end.revision > start.revision);
        assert_eq!(pct(end), Some(61.0));
        assert!(answer.snapshot.revision >= end.revision);
        assert_eq!(
            other.count(),
            0,
            "nothing for subscribers of another vendor"
        );
    }

    #[test]
    fn a_panicking_probe_still_closes_the_probe() {
        let (rig, _clock) = rig();
        rig.answer(0, Scripted::Panic);
        let answer = answered(&bus_request(&rig.service, 0, RequestKind::Refresh));
        assert_eq!(answer.served, UsageServed::Cached);
        assert!(!answer.snapshot.in_flight);
        assert_eq!(failed_kind(&answer.snapshot).as_deref(), Some("io"));
        wait_until("the query thread was released", || ended(&rig) == 1);
        assert_eq!(rig.threads.counts.panicked.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_dropped_request_future_leaves_the_probe_to_its_guard() {
        let (rig, _clock) = rig();
        let log = rig.subscriber(1, &[0]);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime");
        let dropped = runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_millis(50),
                rig.service.request(vendor(0), RequestKind::Refresh),
            )
            .await
        });
        assert!(
            dropped.is_err(),
            "the request was dropped while waiting for the end"
        );
        assert!(rig.in_flight(0));
        assert_eq!(rig.joiners(0), 0, "the dropped request let go of the wait");

        rig.answer(0, ok(active(0, 8.0)));
        wait_until("the end sheet", || last_pct(&log) == Some(8.0));
        assert!(!rig.in_flight(0));
    }

    #[test]
    fn a_stuck_probe_does_not_hold_up_another_vendor() {
        let (rig, _clock) = rig();
        let stuck = bus_request(&rig.service, 0, RequestKind::Refresh);
        wait_until("the query started", || rig.queries(0) == 1);

        rig.answer(1, ok(active(1, 12.0)));
        let answer = answered(&bus_request(&rig.service, 1, RequestKind::Refresh));
        assert_eq!(answer.served, UsageServed::Fresh);
        assert!(rig.in_flight(0), "the earlier vendor is still blocked");

        rig.answer(0, ok(active(0, 1.0)));
        assert_eq!(answered(&stuck).served, UsageServed::Fresh);
    }

    #[test]
    fn a_tick_plan_publishes_its_sheets_then_starts_its_probes() {
        let (rig, _clock) = rig();
        let log = rig.subscriber(1, &[0]);
        // A query that could not be launched is closed at once inside `carry_out` and emits an end sheet — the two sheets' order is the emission-then-launch order.
        rig.threads.refuse.store(true, Ordering::SeqCst);
        let plan = {
            let now = rig.service.now();
            rig.service.desk().book.plan_tick(&set(&[0]), now)
        };
        assert_eq!(
            plan.start,
            [key(0)],
            "a subscribed cell with no reference point — query at once"
        );
        assert_eq!(plan.publish.len(), 1);
        let sleep = plan.sleep;
        assert_eq!(rig.service.carry_out(plan), sleep);

        let got = frames(&log);
        assert_eq!(got.len(), 2);
        assert!(
            got[0].snapshot.in_flight,
            "the plan's 「refreshing」 sheet comes first"
        );
        assert!(
            !got[1].snapshot.in_flight,
            "the launch (closed at once here) comes later"
        );
        assert!(got[0].snapshot.revision < got[1].snapshot.revision);
        assert!(!rig.in_flight(0));
    }

    /// ★⟳ has no minimum interval (user decision)★ — a second ⟳ with the clock not moved even one tick also queries.
    #[test]
    fn a_refresh_right_after_a_finished_probe_starts_a_new_probe() {
        let (rig, _clock) = rig();
        let log = rig.subscriber(1, &[0]);
        rig.answer(0, ok(active(0, 30.0)));
        let first = rig
            .service
            .request_blocking(vendor(0), RequestKind::Refresh)
            .expect("known vendor");
        assert_eq!(first.served, UsageServed::Fresh);
        assert!(!rig.in_flight(0));

        rig.answer(0, ok(active(0, 31.0)));
        let second = rig
            .service
            .request_blocking(vendor(0), RequestKind::Refresh)
            .expect("known vendor");
        assert_eq!(rig.queries(0), 2);
        assert_eq!(second.served, UsageServed::Fresh);
        assert_eq!(pct(&second.snapshot), Some(31.0));
        assert_eq!(last_pct(&log), Some(31.0), "it reached the subscriber too");
    }

    #[test]
    fn a_failed_probe_after_an_earlier_success_answers_cached() {
        let (rig, _clock) = rig();
        rig.answer(0, ok(active(0, 30.0)));
        let first = rig
            .service
            .request_blocking(vendor(0), RequestKind::Refresh)
            .expect("known vendor");
        assert_eq!(first.served, UsageServed::Fresh);

        rig.answer(0, Scripted::Answer(Err(ProbeError::Timeout.into())));
        let second = rig
            .service
            .request_blocking(vendor(0), RequestKind::Refresh)
            .expect("known vendor");
        assert_eq!(
            rig.queries(0),
            2,
            "a ⟳ right after the end launches a new query too"
        );
        assert_eq!(
            second.served,
            UsageServed::Cached,
            "the waited query failed — the earlier success is not a new value for this request"
        );
        assert_eq!(pct(&second.snapshot), Some(30.0), "the value is kept");
        assert_eq!(failed_kind(&second.snapshot).as_deref(), Some("timeout"));
    }

    #[test]
    fn a_joiner_that_gives_up_after_an_earlier_success_answers_cached() {
        let clock = Arc::new(ManualUsageClock::new(Duration::from_secs(100), T0));
        let rig = rig_full(clock, 2, SHORT_WAIT);
        // The preceding success is launched and ended without a request, so it is not caught by the short wait.
        assert!(rig.service.desk().book.begin_probe(&key(0)));
        rig.service.start_probe(key(0));
        rig.answer(0, ok(active(0, 30.0)));
        wait_until("the earlier query ended", || ended(&rig) == 1);
        assert_eq!(rig.service.desk().runs[&key(0)].last_ok, 1);

        let answer = rig
            .service
            .request_blocking(vendor(0), RequestKind::Refresh)
            .expect("known vendor");
        assert!(answer.snapshot.in_flight, "gave up at the cap");
        assert_eq!(answer.served, UsageServed::Cached);
        assert_eq!(pct(&answer.snapshot), Some(30.0));
    }

    /// An outlet that, once armed, stops at the end sheet received (not `in_flight`) and stays inside `send` until `release` comes. No time cap — freed when the `release` sender disappears on test failure.
    struct Parking {
        armed: Arc<AtomicBool>,
        parked: mpsc::Sender<()>,
        release: Mutex<Receiver<()>>,
    }

    impl UsageOutlet for Parking {
        fn send(&self, frame: &UsageFrame) {
            if !self.armed.load(Ordering::SeqCst) || frame.snapshot.in_flight {
                return;
            }
            let _ = self.parked.send(());
            let _ = self.release.lock().unwrap().recv();
        }

        fn revoke(&self) {}
    }

    #[test]
    fn joiners_wake_only_after_the_end_sheet_is_published() {
        let clock = Arc::new(ManualUsageClock::new(Duration::from_secs(100), T0));
        // Make the joiner's wait more generous than the test-side cap — removes the branch answering with a wait overrun while stopped.
        let rig = rig_full(clock, 2, BOUND * 6);
        let armed = Arc::new(AtomicBool::new(false));
        let (parked_tx, parked) = mpsc::channel();
        let (release, release_rx) = mpsc::channel();
        rig.service.attach(
            1,
            Parking {
                armed: armed.clone(),
                parked: parked_tx,
                release: Mutex::new(release_rx),
            },
        );
        rig.service.replace_subscription(1, set(&[0]));
        armed.store(true, Ordering::SeqCst);

        let pending = bus_request(&rig.service, 0, RequestKind::Refresh);
        wait_until("the query started", || rig.queries(0) == 1);
        // One bus joiner the test holds directly — sees whether the wake came without timing (the query is in flight awaiting a script).
        let (joined, woken) = mpsc::sync_channel(1);
        rig.service
            .desk()
            .runs
            .get_mut(&key(0))
            .expect("entry")
            .blocking
            .push(joined);

        rig.answer(0, ok(active(0, 5.0)));
        parked
            .recv_timeout(BOUND)
            .expect("the end sheet stopped inside the outlet");
        // While stopped — if the wake was ahead of the emission, it has already arrived.
        assert!(
            matches!(woken.try_recv(), Err(TryRecvError::Empty)),
            "woke the bus joiner before the emit finished"
        );
        assert_eq!(
            *rig.service.finished[&key(0)].borrow(),
            0,
            "woke the async joiner before the emit finished"
        );
        assert!(matches!(pending.try_recv(), Err(TryRecvError::Empty)));

        release.send(()).expect("outlet");
        woken.recv_timeout(BOUND).expect("wakes after the emit");
        assert_eq!(answered(&pending).served, UsageServed::Fresh);
    }
}
