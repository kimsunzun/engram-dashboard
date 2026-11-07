//! Usage book — a pure state machine per vendor cell (TRD §1-4). It has no threads, locks, clocks, I/O or emission: the caller passes the time
//! ([`Now`]) and gets back what changed and what to do (start a query · emit · sleep). Locking, launching queries
//! and emitting are the service's share.
//!
//! ★revision rises only when something visible changes (§3 #49)★ — the values a snapshot carries (window % · reset · expiry · per model · plan ·
//!   status · detail) or the next automatic query deadline. Picking up the same value again so only its age is fresher is not a change. Exception = query
//!   start and end always raise it (`in_flight` changes).
//! ★Changes that arise merely from time passing (expiry latch · refusal end → query failure) are stamped onto the cell by [`UsageBook::eval_time`] and the
//!   revision rises then★ — ★calling this before taking a snapshot is the duty of the caller (the service)★. Pickup application · query
//!   end · [`UsageBook::plan_tick`] · the emission sheet ([`UsageBook::broadcast_sheet`] · [`UsageBook::coalesce_passive`]) call it
//!   themselves.
//! ★Debt — once revision rises, that cell owes one emission to all subscribers★ (a change in a pickup value = a coalescible debt ·
//!   anything else (latch · refusal end — including those that stood inside a pickup application · query start and end) = a debt to repay at once). ★The only thing that repays a debt is an emission to all
//!   subscribers★ — [`UsageBook::broadcast_sheet`] · [`Coalesce::PublishNow`] · [`TickPlan::publish`]
//!   repay and take a sheet in one call (if the lock is retaken to repay, a debt that stood in between is erased without being carried). The
//!   first sheet of a subscription add (`eval_time` + `snapshot`) goes only to that connection so it does not repay — the woken scheduler emits to all (a new connection
//!   gets one overlapping sheet). ★So even if the service misses an emission, for a subscribed cell the next [`UsageBook::plan_tick`] repays it★.
//! ★It does not panic whatever it receives★ — every place that calls the book is outside the containment mark so a panic = immediate daemon exit (hook ·
//! ADR-0292). All time arithmetic is saturating/checked.

use std::time::Duration;

use engram_dashboard_agent::usage::{
    ProbeError, ProbeFailure, ScopedWindowObs, UsageAccountKey, UsageDetail, UsageKey,
    UsageObservation, UsagePolicy, UsageVendorKey, WindowObs,
};
use engram_dashboard_protocol::{
    AgentBackendKind, UsageLimitSnapshot, UsageScopedWindow, UsageStateDetail, UsageVendorState,
    UsageWindow,
};
use serde::de::value::{Error as ValueError, StrDeserializer};
use serde::de::IntoDeserializer;
use serde::Deserialize;

/// Refusal wait — when upstream gave no usable wait (none · 0) (D4).
pub const REJECT_FALLBACK: Duration = Duration::from_secs(5 * 60);
/// Refusal wait cap — the wait upstream gave is cut here (§3 #37).
pub const REJECT_MAX: Duration = Duration::from_secs(24 * 60 * 60);
/// If two `resets_at` are within this, it is the same reset — boundary inclusive (60 s apart = same · 61 s apart = different, §3 #46).
pub const RESET_SAME_TOLERANCE: Duration = Duration::from_secs(60);
/// The cap on per-model windows accepted from one observation — pickup runs straight on the pump thread. Anything over is accepted only up to this many from the front.
pub const MODEL_SCOPED_MAX: usize = 16;
/// Coalesce pickup emission per cell over this window (§3 #60) — pickup changes only. Query start, end and latches go out at once.
/// It is not a source rule — it is frequency control of the changes crowding into the stream entrance ([`UsageBook::apply_passive`]) (§3 #88).
pub const USAGE_PUBLISH_COALESCE: Duration = Duration::from_secs(1);
/// Scheduler sleep cap (§3 #58) — the wait timer may not count sleep and the reset is wall clock, so lateness after resume or a clock shift
/// is bounded here.
pub const SCHEDULE_MAX_SLEEP: Duration = Duration::from_secs(60);

/// The value of reading [`super::clock::UsageClock`] once — `mono` = monotonic elapsed since that clock's origin, `wall` = epoch seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Now {
    pub mono: Duration,
    pub wall: i64,
}

/// The result of [`UsageBook::apply_passive`]. If `changed`, revision rose · if `next_auto_changed`, the scheduler must be
/// woken (D12 — `changed` also stands with it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PassiveApplied {
    pub changed: bool,
    pub next_auto_changed: bool,
}

/// Request kind (§1-4 request table). `Refresh` = ⟳ — ignores the cooldown but cannot get past a refusal (D11).
///
/// ★`Refresh` has no minimum interval (user decision 2026-09-29)★ — a ⟳ right after a query just ended is also a new query.
///   The only brakes left are the refusal deadline (R24) and joining an in-flight one — most peers have no manual interval (exceptions · unconfirmed =
///   `docs/research/usage-manual-refresh-peers-2026-09-29.md`), and if upstream blocks, the caller throttles itself.
// ADR-0258
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Get,
    Refresh,
}

/// The decision of [`UsageBook::judge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Judgment {
    /// Answer from the current cache without a query — a `Get` before the deadline.
    Cached,
    /// A `Refresh` before the refusal deadline — does not send out a query (R24).
    Rejected,
    /// There is a query in flight — wait for its end.
    Join,
    /// ★The book has already set `in_flight` and raised revision★ (same as [`UsageBook::begin_probe`]) — the caller
    /// emits with [`UsageBook::broadcast_sheet`] and launches the query. The end must be closed with [`UsageBook::finish_probe`]
    /// — if not closed, that cell stays in flight forever.
    Start,
}

/// The result of [`UsageBook::coalesce_passive`].
#[derive(Debug, Clone, PartialEq)]
pub enum Coalesce {
    /// One sheet to send to all of that cell's subscribers now — the book recorded the debt as repaid.
    PublishNow(UsageLimitSnapshot),
    /// There is no sheet to send now — the caller wakes the scheduler. If a debt remains, [`UsageBook::plan_tick`] emits the sheet of that moment
    /// at the coalescing deadline.
    Deferred,
}

/// The result of [`UsageBook::plan_tick`]. The order of both lists = the order the cells were made.
#[derive(Debug, Clone, PartialEq)]
pub struct TickPlan {
    /// Queries started — the book has already set `in_flight` (the same duty follows as [`Judgment::Start`]).
    pub start: Vec<UsageKey>,
    /// Sheets to send to all of that cell's subscribers now (key = the cell to find the target in the registry) — the book recorded the debt as repaid.
    pub publish: Vec<(UsageKey, UsageLimitSnapshot)>,
    /// Until the next wake — `0 < sleep ≤ SCHEDULE_MAX_SLEEP`.
    pub sleep: Duration,
}

// Written by hand — if derived, a driver that waits as is with sleep 0 would spin.
impl Default for TickPlan {
    fn default() -> Self {
        Self {
            start: Vec::new(),
            publish: Vec::new(),
            sleep: SCHEDULE_MAX_SLEEP,
        }
    }
}

/// One per cell. The key list is fixed at construction.
pub struct UsageBook {
    cells: Vec<Cell>,
}

impl UsageBook {
    /// One default-account cell per vendor received — if the same vendor comes twice only the first is seen. ★A word that cannot be turned into a wire vendor
    /// is not registered (one warn)★ — so a registered cell always takes a snapshot, and that vendor is an unknown key to every method.
    pub fn new(vendors: &[(UsageVendorKey, UsagePolicy)]) -> Self {
        let mut cells: Vec<Cell> = Vec::with_capacity(vendors.len());
        let mut seen: Vec<UsageVendorKey> = Vec::with_capacity(vendors.len());
        for &(vendor, policy) in vendors {
            if seen.contains(&vendor) {
                continue;
            }
            seen.push(vendor);
            if let Some(wire) = wire_vendor(vendor) {
                cells.push(Cell::new(vendor, wire, policy));
            }
        }
        Self { cells }
    }

    /// Order made.
    pub fn keys(&self) -> Vec<UsageKey> {
        self.cells.iter().map(|cell| cell.key.clone()).collect()
    }

    pub fn revision(&self, key: &UsageKey) -> Option<u64> {
        self.cell(key).map(|cell| cell.revision)
    }

    pub fn in_flight(&self, key: &UsageKey) -> Option<bool> {
        self.cell(key).map(|cell| cell.in_flight)
    }

    /// The `mono` deadline of the next automatic query (R29) = `max(max(last_query, fresh_reset) + cooldown, reject_until)` —
    /// `reject_until` if there is no reference point, and if not even that, `now.mono` (= the deadline is now). It does not look at whether one is in flight.
    /// `None` = unknown key.
    pub fn next_auto(&self, key: &UsageKey, now: Now) -> Option<Duration> {
        self.cell(key).map(|cell| cell.next_auto(now))
    }

    /// Pickup (pump) entrance — merges the value of one observation with the same merge as a query success (R16·D12 · does not distinguish the source).
    /// ★Only the value merge is the same★ — things only query end does (clearing the refusal deadline · the `last_query` reference point · clearing the D12 mark · folding success into
    /// `Ready`/`Unavailable`) are not here. A change in value is a coalescible debt ([`UsageBook::coalesce_passive`]).
    /// Cell = the observation's vendor + the default account. If the vendor is unknown it does nothing.
    pub fn apply_passive(&mut self, obs: &UsageObservation, now: Now) -> PassiveApplied {
        let account = UsageAccountKey::default();
        let Some(cell) = self
            .cells
            .iter_mut()
            .find(|cell| cell.key.vendor == obs.vendor && cell.key.account == account)
        else {
            return PassiveApplied::default();
        };
        let before = cell.visible(now);
        cell.merge(obs, now);
        let latched = cell.eval_time(now);
        let after = cell.visible(now);
        let changed = !before.same(&after);
        if changed {
            // Only a change in pickup value can be coalesced — latches and refusal ends stamped by the same call go out at once.
            cell.bump_owing(if latched {
                Owed::Now
            } else {
                Owed::Coalescable
            });
        }
        PassiveApplied {
            changed,
            next_auto_changed: before.next_auto != after.next_auto,
        }
    }

    /// Query start — set `in_flight` and revision +1. `false` if the key is unknown or one is already in flight (nothing changes).
    pub fn begin_probe(&mut self, key: &UsageKey) -> bool {
        match self.cell_mut(key) {
            Some(cell) if !cell.in_flight => {
                cell.start();
                true
            }
            _ => false,
        }
    }

    /// Query end (success · no limit info · failure — including timeout). The success value is merged through the same single path as pickup — a window
    /// not carried keeps the value it held. Success becomes `Ready` (if no limit info, empty the windows and `Unavailable`) · failure
    /// leaves the values untouched and folds into that failure state. `last_query = now.mono` becomes the new reference point (the D12 newly-arrived mark is
    /// also cleared here). Success (including no limit info) clears the refusal deadline. It applies even if none was in flight, and revision
    /// is always +1. If the key is unknown it does nothing.
    pub fn finish_probe(
        &mut self,
        key: &UsageKey,
        result: Result<UsageObservation, ProbeFailure>,
        now: Now,
    ) {
        let Some(cell) = self.cell_mut(key) else {
            return;
        };
        match result {
            Ok(obs) => cell.apply_success(&obs, now),
            Err(failure) => cell.apply_failure(failure, now.mono),
        }
        cell.last_query = Some(now.mono);
        cell.fresh_five_hour = false;
        cell.fresh_weekly = false;
        cell.in_flight = false;
        // If upstream gave an already-past reset, it shows as expired at the point received — carried in the revision that rises anyway.
        cell.eval_time(now);
        cell.bump();
    }

    /// Stamp changes that arise merely from time passing onto the cell — the expiry latch (R32: once per window with `resets_at <= now.wall`) · refusal end
    /// (`Rejected` → `Failed` where `reject_until <= now.mono`, detail kept — R31). If anything was stamped, revision +1 and `true`.
    pub fn eval_time(&mut self, key: &UsageKey, now: Now) -> bool {
        let Some(cell) = self.cell_mut(key) else {
            return false;
        };
        let changed = cell.eval_time(now);
        if changed {
            cell.bump();
        }
        changed
    }

    /// One wire sheet as of `now` — does not repay debt (the first sheet of a subscription is taken with this). `None` = unknown key.
    /// ★Call [`UsageBook::eval_time`] first★ — even if not called, an ended refusal shows as a query failure, but that change is not in
    /// the revision.
    pub fn snapshot(&self, key: &UsageKey, now: Now) -> Option<UsageLimitSnapshot> {
        self.cell(key).map(|cell| cell.sheet(now))
    }

    /// One sheet to send to all of that cell's subscribers — emission on query start · end · a latch at request time. `eval_time` (revision +1 if changed)
    /// → repay debt (previous emission = `now`) → snapshot, all in one call (header 「빚」). `None` = unknown key.
    pub fn broadcast_sheet(&mut self, key: &UsageKey, now: Now) -> Option<UsageLimitSnapshot> {
        let cell = self.cell_mut(key)?;
        if cell.eval_time(now) {
            cell.bump();
        }
        Some(cell.discharge(now))
    }

    /// The decision for one request — look at the §1-4 request table **row by row from the top**: ① `Get` and `now < next_auto` → cache
    /// ② `Refresh` and `now < reject_until` → refusal ③ otherwise → join if in flight, else start. ★Joining exists only
    /// in ③★ — even if in flight, if it hits the first two rows it answers with that row. `None` = unknown key (nothing changes).
    ///
    /// The decision does not read what [`UsageBook::eval_time`] stamps (latches · refusal end) — when the caller calls it before deciding
    /// (the request-arrival row) it is to emit that change, not to change the decision.
    pub fn judge(&mut self, key: &UsageKey, kind: RequestKind, now: Now) -> Option<Judgment> {
        let cell = self.cell_mut(key)?;
        let judgment = match kind {
            RequestKind::Get if now.mono < cell.next_auto(now) => Judgment::Cached,
            RequestKind::Refresh if cell.reject_until.is_some_and(|until| now.mono < until) => {
                Judgment::Rejected
            }
            _ if cell.in_flight => Judgment::Join,
            _ => {
                cell.start();
                Judgment::Start
            }
        };
        Some(judgment)
    }

    /// Called with the same `now` after pickup changed a cell (`PassiveApplied.changed`) — coalesces pickup only (§3 #60). If there is a debt to repay at once
    /// (latch · refusal end · query start/end etc.) `PublishNow` · if only coalescible debt, `PublishNow` when there is no previous emission or
    /// [`USAGE_PUBLISH_COALESCE`] or more has passed, else `Deferred` · if no debt `Deferred`.
    /// `None` = unknown key.
    pub fn coalesce_passive(&mut self, key: &UsageKey, now: Now) -> Option<Coalesce> {
        let cell = self.cell_mut(key)?;
        Some(if cell.owes_at(now.mono) {
            Coalesce::PublishNow(cell.discharge(now))
        } else {
            Coalesce::Deferred
        })
    }

    /// One scheduler pass (§1-4 「스케줄러 깸」 (scheduler wake) · §3 #58·#85). For each subscribed cell: [`UsageBook::eval_time`] → not in flight ∧
    /// `now ≥ next_auto` starts a query (the same start as the request path — a later request joins) → if there is a debt to repay at once or
    /// the coalescing deadline of a coalescible debt is reached, repay the debt and carry that sheet in `publish`. ★A debt the service missed
    /// emitting is also repaid here★ — for a subscribed cell it is reached by the next wake at the latest (≤ [`SCHEDULE_MAX_SLEEP`]).
    ///
    /// `sleep` = until the earliest of the subscribed cells' `next_auto` (excluded if in flight) · the `resets_at` of a latch not yet stood (wall-clock difference) · the `Rejected` cell's
    /// `reject_until` · the coalescing deadline of a coalescible debt, capped at [`SCHEDULE_MAX_SLEEP`]. ★Since a reached deadline is all handled in this one
    /// pass, `sleep` is not 0★. If there is no subscribed cell there is neither a start nor a deadline (cap sleep).
    ///
    /// ★A cell outside any subscription is only skipped and its debt is not touched★ — `subscribed` is a copy taken under the registry lock so it may be stale:
    /// a stale copy must not erase an emission that a just-subscribed connection should receive. The remaining debt merely goes out later as an overlapping sheet
    /// (revision tells them apart). Duplicates and unknown vendors in `subscribed` are ignored.
    pub fn plan_tick<'a>(
        &mut self,
        subscribed: impl IntoIterator<Item = &'a UsageVendorKey>,
        now: Now,
    ) -> TickPlan {
        let subscribed: Vec<UsageVendorKey> = subscribed.into_iter().copied().collect();
        let mut plan = TickPlan::default();
        for cell in &mut self.cells {
            if !subscribed.contains(&cell.key.vendor) {
                continue;
            }
            if cell.eval_time(now) {
                cell.bump();
            }
            if !cell.in_flight && now.mono >= cell.next_auto(now) {
                cell.start();
                plan.start.push(cell.key.clone());
            }
            if cell.owes_at(now.mono) {
                plan.publish.push((cell.key.clone(), cell.discharge(now)));
            }
            if let Some(wait) = cell.next_wake(now) {
                plan.sleep = plan.sleep.min(wait);
            }
        }
        plan
    }

    fn cell(&self, key: &UsageKey) -> Option<&Cell> {
        self.cells.iter().find(|cell| cell.key == *key)
    }

    fn cell_mut(&mut self, key: &UsageKey) -> Option<&mut Cell> {
        self.cells.iter_mut().find(|cell| cell.key == *key)
    }
}

/// The state a cell holds. All five abnormal ones carry detail (§1-4 「실패 추적」 (failure trace)) — even when a `Rejected` ends and shows as `Failed`
/// it stays. An observation carrying window values returns any state to `Ready` ([`Cell::merge`]).
#[derive(Debug, Clone, PartialEq, Eq)]
enum CellState {
    Ready,
    NotInstalled(UsageDetail),
    NeedsLogin(UsageDetail),
    Unavailable(UsageDetail),
    Failed(UsageDetail),
    Rejected(UsageDetail),
}

/// The emission debt a cell owes all subscribers (header 「빚」). Declaration order is size — when overlapping the larger one remains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Owed {
    None,
    /// Only a change in pickup value — repaid once [`USAGE_PUBLISH_COALESCE`] has passed since the previous emission.
    Coalescable,
    /// Repaid at once.
    Now,
}

struct Cell {
    key: UsageKey,
    policy: UsagePolicy,
    wire_vendor: AgentBackendKind,
    five_hour: Option<Window>,
    weekly: Option<Window>,
    /// In label order (same labels in received order) — sorted on receipt so the upstream's listing order is not read as a change.
    model_scoped: Vec<(String, Window)>,
    plan: Option<String>,
    state: CellState,
    last_query: Option<Duration>,
    /// D12 — the `mono` at which both windows carried a new `used_pct` and the cooldown reference point stood again.
    fresh_reset: Option<Duration>,
    /// ★Lives separately from the state★ — even if a value arrives and shows as `Ready`, neither an automatic query nor ⟳ goes out before this deadline (R24).
    /// ★Kept in memory only★ — not carried across a daemon restart, so a restarted daemon starts without a refusal deadline.
    // ADR-0284
    reject_until: Option<Duration>,
    /// D12 — windows for which `used_pct` came in after the current reference point (§3 #4 — a window with only a reset is not counted).
    fresh_five_hour: bool,
    fresh_weekly: bool,
    in_flight: bool,
    revision: u64,
    /// Emission debt and coalescing reference point (§3 #60) — not something visible (outside revision). `last_publish` = the `mono` when the debt was last repaid.
    owed: Owed,
    last_publish: Option<Duration>,
}

impl Cell {
    fn new(vendor: UsageVendorKey, wire_vendor: AgentBackendKind, policy: UsagePolicy) -> Self {
        Self {
            key: UsageKey {
                vendor,
                account: UsageAccountKey::default(),
            },
            policy,
            wire_vendor,
            five_hour: None,
            weekly: None,
            model_scoped: Vec::new(),
            plan: None,
            state: CellState::Ready,
            last_query: None,
            fresh_reset: None,
            reject_until: None,
            fresh_five_hour: false,
            fresh_weekly: false,
            in_flight: false,
            revision: 0,
            owed: Owed::None,
            last_publish: None,
        }
    }

    fn bump(&mut self) {
        self.bump_owing(Owed::Now);
    }

    fn bump_owing(&mut self, owed: Owed) {
        self.revision = self.revision.saturating_add(1);
        self.owed = self.owed.max(owed);
    }

    fn start(&mut self) {
        self.in_flight = true;
        self.bump();
    }

    /// Repay the debt and take that sheet — it is taken even with no debt (an overlapping sheet is told apart by revision).
    fn discharge(&mut self, now: Now) -> UsageLimitSnapshot {
        // Do not rewind even if called with a late-read time — rewinding would pull the coalescing window earlier and emit twice within 1 second.
        self.last_publish = Some(
            self.last_publish
                .map_or(now.mono, |last| last.max(now.mono)),
        );
        self.owed = Owed::None;
        self.sheet(now)
    }

    fn owes_at(&self, mono: Duration) -> bool {
        match self.owed {
            Owed::None => false,
            Owed::Coalescable => self.coalesce_deadline() <= mono,
            Owed::Now => true,
        }
    }

    /// The `mono` at which the coalescing window closes after the previous emission — 0 if never emitted (= already closed).
    fn coalesce_deadline(&self) -> Duration {
        self.last_publish.map_or(Duration::ZERO, |last| {
            last.saturating_add(USAGE_PUBLISH_COALESCE)
        })
    }

    /// This cell's share of [`UsageBook::plan_tick`] — the time left until this cell's earliest next deadline. A reached deadline was already
    /// handled by the caller so only deadlines still ahead are counted — hence it does not yield 0. `None` if there is no deadline.
    fn next_wake(&self, now: Now) -> Option<Duration> {
        let ahead = |deadline: Duration| deadline.checked_sub(now.mono).filter(|d| !d.is_zero());
        let auto = (!self.in_flight)
            .then(|| self.next_auto(now))
            .and_then(ahead);
        // Use the wall-clock difference as the sleep as is — adding it to `mono` and subtracting would saturate at the end of `mono` and become 0.
        let reset = self
            .windows()
            .filter(|window| !window.expired)
            .filter_map(|window| window.resets_at)
            .filter(|&at| at > now.wall)
            .map(|at| Duration::from_secs(at.abs_diff(now.wall)))
            .min();
        let reject = match self.state {
            CellState::Rejected(_) => self.reject_until.and_then(ahead),
            _ => None,
        };
        let coalesce = (self.owed == Owed::Coalescable)
            .then(|| self.coalesce_deadline())
            .and_then(ahead);
        [auto, reset, reject, coalesce].into_iter().flatten().min()
    }

    fn windows(&self) -> impl Iterator<Item = &Window> {
        self.five_hour
            .iter()
            .chain(self.weekly.iter())
            .chain(self.model_scoped.iter().map(|(_, w)| w))
    }

    fn next_auto(&self, now: Now) -> Duration {
        match self.last_query.max(self.fresh_reset) {
            Some(reference) => {
                let due = reference.saturating_add(self.policy.cooldown);
                self.reject_until.map_or(due, |until| due.max(until))
            }
            // A refusal is set only at the end of a query so it does not come without a reference point — this is a defense so that even if it did, no query happens before the refusal end.
            None => self.reject_until.unwrap_or(now.mono),
        }
    }

    fn visible(&self, now: Now) -> Visible {
        Visible {
            five_hour: self.five_hour.clone(),
            weekly: self.weekly.clone(),
            model_scoped: self.model_scoped.clone(),
            plan: self.plan.clone(),
            state: self.state.clone(),
            in_flight: self.in_flight,
            reject_until: self.reject_until,
            next_auto: self.next_auto(now),
        }
    }

    /// Merge the value of one observation — the one path common to pickup and query success. Carried windows/fields overwrite and uncarried ones stay
    /// (`model_scoped` and `plan` are replaced whole if carried). ★If even one window value (% or reset) is carried, it is
    /// `Ready` in any state★ — values coming in is the proof of normal (§3 #52). The refusal deadline is not touched.
    // ADR-0256
    fn merge(&mut self, obs: &UsageObservation, now: Now) {
        if let Some(window) = obs.five_hour {
            merge_window(&mut self.five_hour, window, now).mark(&mut self.fresh_five_hour);
        }
        if let Some(window) = obs.weekly {
            merge_window(&mut self.weekly, window, now).mark(&mut self.fresh_weekly);
        }
        if let Some(list) = &obs.model_scoped {
            self.model_scoped = scoped_windows(list, now.mono);
        }
        if let Some(plan) = &obs.plan {
            self.plan = Some(plan.clone());
        }
        if self.fresh_five_hour && self.fresh_weekly {
            self.fresh_reset = Some(now.mono);
            self.fresh_five_hour = false;
            self.fresh_weekly = false;
        }
        if carries_value(obs) {
            self.state = CellState::Ready;
        }
    }

    fn apply_success(&mut self, obs: &UsageObservation, now: Now) {
        // Upstream has given an answer so the refusal is over — if left, a past deadline would hold the next automatic query.
        self.reject_until = None;
        self.merge(obs, now);
        self.state = match &obs.limits_unavailable {
            // It is an answer about the account — wire rule 「`Unavailable` 은 값을 싣지 않는다」 (Unavailable carries no values).
            Some(detail) => {
                self.five_hour = None;
                self.weekly = None;
                self.model_scoped.clear();
                CellState::Unavailable(detail.clone())
            }
            None => CellState::Ready,
        };
    }

    fn apply_failure(&mut self, failure: ProbeFailure, mono: Duration) {
        let ProbeFailure { error, detail } = failure;
        let detail = detail.unwrap_or_else(|| kind_only(&error));
        // No wildcard — when the classification grows, compilation stops here and makes us decide the visible state.
        self.state = match error {
            ProbeError::NotInstalled => CellState::NotInstalled(detail),
            ProbeError::Unauthenticated => CellState::NeedsLogin(detail),
            ProbeError::RateLimited { retry_after } => {
                self.reject_until = Some(reject_deadline(mono, retry_after));
                CellState::Rejected(detail)
            }
            ProbeError::Timeout
            | ProbeError::Unsupported
            | ProbeError::Spawn(_)
            | ProbeError::Io(_)
            | ProbeError::Parse(_)
            | ProbeError::Upstream(_) => CellState::Failed(detail),
        };
    }

    /// This cell's share of [`UsageBook::eval_time`] — revision is raised by the caller. It looks at all windows without stopping at one.
    fn eval_time(&mut self, now: Now) -> bool {
        let mut changed = false;
        for window in self
            .five_hour
            .iter_mut()
            .chain(self.weekly.iter_mut())
            .chain(self.model_scoped.iter_mut().map(|(_, w)| w))
        {
            changed |= window.latch(now.wall);
        }
        if let CellState::Rejected(detail) = &self.state {
            if self.reject_until.is_none_or(|until| until <= now.mono) {
                self.state = CellState::Failed(detail.clone());
                changed = true;
            }
        }
        changed
    }

    fn sheet(&self, now: Now) -> UsageLimitSnapshot {
        UsageLimitSnapshot {
            vendor: self.wire_vendor,
            account_key: self.key.account.as_str().to_owned(),
            five_hour: self.five_hour.as_ref().and_then(|w| w.wire(now.mono)),
            weekly: self.weekly.as_ref().and_then(|w| w.wire(now.mono)),
            model_scoped: self
                .model_scoped
                .iter()
                .filter_map(|(label, w)| {
                    w.wire(now.mono).map(|window| UsageScopedWindow {
                        label: label.clone(),
                        window,
                    })
                })
                .collect(),
            plan: self.plan.clone(),
            in_flight: self.in_flight,
            state: self.wire_state(now),
            revision: self.revision,
        }
    }

    fn wire_state(&self, now: Now) -> UsageVendorState {
        let failed = |detail: &UsageDetail| UsageVendorState::Failed {
            next_attempt_in_secs: self.next_auto(now).saturating_sub(now.mono).as_secs(),
            detail: Some(wire_detail(detail)),
        };
        match &self.state {
            CellState::Ready => UsageVendorState::Ready,
            CellState::NotInstalled(detail) => UsageVendorState::NotInstalled {
                detail: Some(wire_detail(detail)),
            },
            CellState::NeedsLogin(detail) => UsageVendorState::NeedsLogin {
                detail: Some(wire_detail(detail)),
            },
            CellState::Unavailable(detail) => UsageVendorState::Unavailable {
                detail: Some(wire_detail(detail)),
            },
            CellState::Failed(detail) => failed(detail),
            // An ended refusal → query failure is stamped by `eval_time`. This branch is the fallback for a snapshot taken without it.
            CellState::Rejected(detail) => match self.reject_until {
                Some(until) if until > now.mono => UsageVendorState::Rejected {
                    retry_in_secs: ceil_secs(until - now.mono),
                    detail: Some(wire_detail(detail)),
                },
                _ => failed(detail),
            },
        }
    }
}

/// A copy for change decisions — everything the snapshot makes visible, minus only the age (`Window::observed`).
struct Visible {
    five_hour: Option<Window>,
    weekly: Option<Window>,
    model_scoped: Vec<(String, Window)>,
    plan: Option<String>,
    state: CellState,
    in_flight: bool,
    reject_until: Option<Duration>,
    next_auto: Duration,
}

impl Visible {
    fn same(&self, other: &Self) -> bool {
        let same_slot = |a: &Option<Window>, b: &Option<Window>| match (a, b) {
            (Some(a), Some(b)) => a.looks_same(b),
            (None, None) => true,
            _ => false,
        };
        same_slot(&self.five_hour, &other.five_hour)
            && same_slot(&self.weekly, &other.weekly)
            && self.model_scoped.len() == other.model_scoped.len()
            && self
                .model_scoped
                .iter()
                .zip(&other.model_scoped)
                .all(|((la, a), (lb, b))| la == lb && a.looks_same(b))
            && self.plan == other.plan
            && self.state == other.state
            && self.in_flight == other.in_flight
            && self.reject_until == other.reject_until
            && self.next_auto == other.next_auto
    }
}

#[derive(Debug, Clone)]
struct Window {
    used_pct: Option<f64>,
    resets_at: Option<i64>,
    /// Expiry latch (R32) — stands once `wall >= resets_at` is seen, and lowers only this window's new-value mark (`used_pct`). It is not lowered even if the wall clock
    /// rewinds. The % is not erased — hiding it is the receiving side's job.
    expired: bool,
    /// The `mono` at which this value was observed — the origin of the age.
    observed: Duration,
}

impl Window {
    /// `None` if both fields are absent — no window without a value is kept.
    fn observed(obs: WindowObs, mono: Duration) -> Option<Self> {
        let obs = ingest(obs);
        (obs.used_pct.is_some() || obs.resets_at.is_some()).then_some(Self {
            used_pct: obs.used_pct,
            resets_at: obs.resets_at,
            expired: false,
            observed: mono,
        })
    }

    fn looks_same(&self, other: &Self) -> bool {
        same_pct(self.used_pct, other.used_pct)
            && self.resets_at == other.resets_at
            && self.expired == other.expired
    }

    fn latch(&mut self, wall: i64) -> bool {
        match self.resets_at {
            Some(at) if !self.expired && at <= wall => {
                self.expired = true;
                true
            }
            _ => false,
        }
    }

    fn wire(&self, mono: Duration) -> Option<UsageWindow> {
        let resets_at = self.resets_at.and_then(|at| u64::try_from(at).ok());
        (self.used_pct.is_some() || resets_at.is_some()).then(|| UsageWindow {
            used_pct: self.used_pct,
            resets_at,
            age_secs: mono.saturating_sub(self.observed).as_secs(),
            expired: self.expired,
        })
    }
}

/// Normalization at the receiving spot — a reset of 0 or below is read as absent (a value the wire cannot carry entering a cell becomes an invisible change).
fn ingest(obs: WindowObs) -> WindowObs {
    WindowObs {
        resets_at: obs.resets_at.filter(|&at| at > 0),
        ..obs
    }
}

/// What [`merge_window`] did to that window's D12 「새로 들어옴」 (newly arrived).
enum Fresh {
    /// `used_pct` was carried — stands.
    Carried,
    /// A changed reset erased the % — also lower what stood earlier (so the cooldown is not reset by an erased %).
    Cleared,
    Untouched,
}

impl Fresh {
    fn mark(self, flag: &mut bool) {
        match self {
            Self::Carried => *flag = true,
            Self::Cleared => *flag = false,
            Self::Untouched => {}
        }
    }
}

/// Merge one carried window into the window held (§1-4 「줍기 관측」 (pickup observation) · §3 #3·#46) — common to pickup and query.
fn merge_window(slot: &mut Option<Window>, obs: WindowObs, now: Now) -> Fresh {
    let obs = ingest(obs);
    let carried = if obs.used_pct.is_some() {
        Fresh::Carried
    } else {
        Fresh::Untouched
    };
    let Some(window) = slot.as_mut() else {
        *slot = Window::observed(obs, now.mono);
        return carried;
    };
    let mut fresh = carried;
    match (obs.used_pct, window.resets_at, obs.resets_at) {
        (Some(pct), kept, new) => {
            window.used_pct = Some(pct);
            window.expired = false;
            window.observed = now.mono;
            // A % that comes after the reset has passed is the value of a new window and that window's reset is unknown (R32 「새 값이 올 때까지」 (until a new value comes)) — if the past reset is
            // kept it would show as expired again at the point received.
            if new.is_none() && kept.is_some_and(|at| at <= now.wall) {
                window.resets_at = None;
            }
        }
        (None, Some(old), Some(new)) if old.abs_diff(new) > RESET_SAME_TOLERANCE.as_secs() => {
            // The old % of a new window is a wrong value. The only value that remains is this observation's reset, so the age is also this observation's.
            window.used_pct = None;
            window.observed = now.mono;
            fresh = Fresh::Cleared;
        }
        _ => {}
    }
    if obs.resets_at.is_some() {
        window.resets_at = obs.resets_at;
    }
    fresh
}

/// For whole-list replacement — `Some` is not merged but replaced whole (the `UsageObservation` contract).
fn scoped_windows(list: &[ScopedWindowObs], mono: Duration) -> Vec<(String, Window)> {
    if list.len() > MODEL_SCOPED_MAX {
        // The two current producers carry only up to the cap — if this is hit, the producer side is off (the type does not
        // declare the cap).
        tracing::warn!(
            received = list.len(),
            kept = MODEL_SCOPED_MAX,
            "모델별 창이 상한을 넘었다 — 앞에서부터 상한만큼만 받는다"
        );
    }
    let mut windows: Vec<(String, Window)> = list
        .iter()
        .take(MODEL_SCOPED_MAX)
        .filter_map(|scoped| {
            Window::observed(scoped.window, mono).map(|w| (scoped.label.clone(), w))
        })
        .collect();
    windows.sort_by(|(a, _), (b, _)| a.cmp(b));
    windows
}

/// Does a window value remain even after passing the receiving spot ([`ingest`] · [`MODEL_SCOPED_MAX`]) — values to be discarded are not counted.
fn carries_value(obs: &UsageObservation) -> bool {
    let has = |w: WindowObs| {
        let w = ingest(w);
        w.used_pct.is_some() || w.resets_at.is_some()
    };
    obs.five_hour.is_some_and(has)
        || obs.weekly.is_some_and(has)
        || obs
            .model_scoped
            .iter()
            .flat_map(|list| list.iter().take(MODEL_SCOPED_MAX))
            .any(|scoped| has(scoped.window))
}

/// NaN is also seen as equal to NaN — producers promise finite numbers, but if broken the same value would read as a change every time it comes.
fn same_pct(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a == b || a.to_bits() == b.to_bits(),
        (None, None) => true,
        _ => false,
    }
}

fn reject_deadline(mono: Duration, retry_after: Option<Duration>) -> Duration {
    let wait = match retry_after {
        Some(wait) if !wait.is_zero() => wait.min(REJECT_MAX),
        _ => REJECT_FALLBACK,
    };
    // If it overflows, leave it as a refusal to the end — reading overflow as no refusal would immediately hammer upstream again.
    mono.checked_add(wait).unwrap_or(Duration::MAX)
}

fn kind_only(error: &ProbeError) -> UsageDetail {
    UsageDetail {
        kind: error.kind_word(),
        code: None,
        upstream: None,
    }
}

/// Round up — during a refusal it does not show as 0 seconds.
fn ceil_secs(d: Duration) -> u64 {
    d.as_secs().saturating_add(u64::from(d.subsec_nanos() > 0))
}

fn wire_detail(detail: &UsageDetail) -> UsageStateDetail {
    UsageStateDetail {
        kind: detail.kind.to_owned(),
        code: detail.code,
        upstream: detail.upstream.as_ref().map(|u| u.as_str().to_owned()),
    }
}

/// Prober word → wire vendor (§1-6 key conversion) — leave it to wire deserialization without a vendor match (it folds case).
// ADR-0004
pub(crate) fn wire_vendor(vendor: UsageVendorKey) -> Option<AgentBackendKind> {
    let word: StrDeserializer<'_, ValueError> = vendor.as_str().into_deserializer();
    match AgentBackendKind::deserialize(word) {
        Ok(kind) => Some(kind),
        Err(e) => {
            tracing::warn!(
                vendor = vendor.as_str(),
                error = %e,
                "조회기 낱말이 wire 벤더가 아니다 — 이 벤더는 칸을 두지 않는다"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_capture::capture_loud;
    use engram_dashboard_agent::backend::usage_probes;
    use engram_dashboard_agent::usage::{UpstreamText, UsageSource};

    const H: i64 = 3_600;
    const T0: i64 = 1_900_000_000;

    // Make cells only from prober keys — do not put vendor literals in tests either.
    fn key(i: usize) -> UsageKey {
        UsageKey {
            vendor: usage_probes()[i].key(),
            account: UsageAccountKey::default(),
        }
    }

    fn cooldown(i: usize) -> Duration {
        usage_probes()[i].policy().cooldown
    }

    fn book() -> UsageBook {
        UsageBook::new(&usage_probes().map(|p| (p.key(), p.policy())))
    }

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    fn at(mono_s: u64, wall: i64) -> Now {
        Now {
            mono: secs(mono_s),
            wall,
        }
    }

    fn w(used_pct: Option<f64>, resets_at: Option<i64>) -> Option<WindowObs> {
        Some(WindowObs {
            used_pct,
            resets_at,
        })
    }

    fn obs(
        i: usize,
        source: UsageSource,
        five_hour: Option<WindowObs>,
        weekly: Option<WindowObs>,
    ) -> UsageObservation {
        UsageObservation {
            vendor: usage_probes()[i].key(),
            five_hour,
            weekly,
            model_scoped: None,
            plan: None,
            source,
            limits_unavailable: None,
        }
    }

    fn passive(i: usize, five: Option<WindowObs>, weekly: Option<WindowObs>) -> UsageObservation {
        obs(i, UsageSource::Passive, five, weekly)
    }

    fn active(i: usize, five: Option<WindowObs>, weekly: Option<WindowObs>) -> UsageObservation {
        obs(i, UsageSource::Active, five, weekly)
    }

    fn scoped(label: &str, window: Option<WindowObs>) -> ScopedWindowObs {
        ScopedWindowObs {
            label: label.to_owned(),
            window: window.expect("창"),
        }
    }

    fn succeed(b: &mut UsageBook, i: usize, o: UsageObservation, now: Now) {
        assert!(b.begin_probe(&key(i)));
        b.finish_probe(&key(i), Ok(o), now);
    }

    fn fail(b: &mut UsageBook, i: usize, failure: ProbeFailure, now: Now) {
        assert!(b.begin_probe(&key(i)));
        b.finish_probe(&key(i), Err(failure), now);
    }

    fn reject_until(b: &UsageBook, i: usize) -> Option<Duration> {
        b.cell(&key(i)).expect("아는 키").reject_until
    }

    fn snap(b: &UsageBook, i: usize, now: Now) -> UsageLimitSnapshot {
        b.snapshot(&key(i), now).expect("아는 키")
    }

    fn pct(window: &Option<UsageWindow>) -> Option<f64> {
        window.as_ref().and_then(|w| w.used_pct)
    }

    fn reset(window: &Option<UsageWindow>) -> Option<u64> {
        window.as_ref().and_then(|w| w.resets_at)
    }

    fn expired(window: &Option<UsageWindow>) -> bool {
        window.as_ref().expect("창").expired
    }

    /// State tag word (wire `kind`).
    fn tag(state: &UsageVendorState) -> String {
        serde_json::to_value(state).expect("직렬화")["kind"]
            .as_str()
            .expect("태그")
            .to_owned()
    }

    fn detail_of(state: &UsageVendorState) -> Option<&UsageStateDetail> {
        match state {
            UsageVendorState::Ready => None,
            UsageVendorState::NotInstalled { detail }
            | UsageVendorState::NeedsLogin { detail }
            | UsageVendorState::Unavailable { detail }
            | UsageVendorState::Failed { detail, .. }
            | UsageVendorState::Rejected { detail, .. } => detail.as_ref(),
        }
    }

    fn rich_detail() -> UsageDetail {
        UsageDetail {
            kind: "some_kind",
            code: Some(429),
            upstream: Some(UpstreamText::new("slow down")),
        }
    }

    /// A book that took both windows 40% · 10% by query (mono 100 · the reset is far later).
    fn seeded() -> UsageBook {
        let mut b = book();
        succeed(
            &mut b,
            0,
            active(
                0,
                w(Some(40.0), Some(T0 + 5 * H)),
                w(Some(10.0), Some(T0 + 100 * H)),
            ),
            at(100, T0),
        );
        b
    }

    // ── Merge (R16·§3 #3·#46·#88) ──

    #[test]
    fn passive_keeps_windows_it_does_not_carry() {
        let mut b = seeded();
        b.apply_passive(&passive(0, w(Some(55.0), None), None), at(200, T0));
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(pct(&s.five_hour), Some(55.0));
        assert_eq!(
            (pct(&s.weekly), reset(&s.weekly)),
            (Some(10.0), Some((T0 + 100 * H) as u64))
        );

        let mut plan_only = passive(0, None, None);
        plan_only.plan = Some("plan-a".to_owned());
        b.apply_passive(&plan_only, at(300, T0));
        let s = snap(&b, 0, at(300, T0));
        assert_eq!(
            (pct(&s.five_hour), pct(&s.weekly)),
            (Some(55.0), Some(10.0))
        );
        assert_eq!(s.plan.as_deref(), Some("plan-a"));
    }

    #[test]
    fn a_probe_result_keeps_windows_it_does_not_carry() {
        let mut b = book();
        let mut full = active(
            0,
            w(Some(40.0), Some(T0 + H)),
            w(Some(10.0), Some(T0 + 9 * H)),
        );
        full.model_scoped = Some(vec![scoped("m", w(Some(3.0), None))]);
        full.plan = Some("plan-a".to_owned());
        succeed(&mut b, 0, full, at(100, T0));
        let held = snap(&b, 0, at(200, T0));

        // A null window · missing window · missing list · missing plan — all stay as is, same as pickup.
        succeed(&mut b, 0, active(0, w(None, None), None), at(200, T0));
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(
            (&s.five_hour, &s.weekly, &s.model_scoped, &s.plan),
            (
                &held.five_hour,
                &held.weekly,
                &held.model_scoped,
                &held.plan
            )
        );
        assert_eq!(s.state, UsageVendorState::Ready);

        // Only the carried window overwrites.
        succeed(&mut b, 0, active(0, None, w(Some(12.0), None)), at(300, T0));
        let s = snap(&b, 0, at(300, T0));
        assert_eq!(
            (pct(&s.five_hour), pct(&s.weekly), reset(&s.weekly)),
            (Some(40.0), Some(12.0), Some((T0 + 9 * H) as u64))
        );
        assert_eq!(s.model_scoped.len(), 1);
        assert_eq!(s.plan.as_deref(), Some("plan-a"));
    }

    #[test]
    fn null_is_not_zero() {
        let mut b = book();
        b.apply_passive(&passive(0, w(None, Some(T0 + H)), None), at(10, T0));
        let s = snap(&b, 0, at(10, T0));
        let five = s.five_hour.expect("리셋만 아는 창");
        assert_eq!(five.used_pct, None);
        assert_eq!(s.weekly, None, "안 실린 창은 0% 창이 아니라 없음이다");

        b.apply_passive(&passive(0, w(Some(0.0), None), None), at(20, T0));
        assert_eq!(pct(&snap(&b, 0, at(20, T0)).five_hour), Some(0.0));
    }

    #[test]
    fn reset_only_for_the_same_reset_keeps_pct_and_takes_the_new_reset() {
        let mut b = seeded();
        b.apply_passive(
            &passive(0, w(None, Some(T0 + 5 * H + 30)), None),
            at(200, T0),
        );
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(
            (pct(&s.five_hour), reset(&s.five_hour)),
            (Some(40.0), Some((T0 + 5 * H + 30) as u64))
        );
    }

    #[test]
    fn reset_only_for_a_changed_reset_clears_pct() {
        let mut b = seeded();
        b.apply_passive(&passive(0, w(None, Some(T0 + 10 * H)), None), at(200, T0));
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(
            (pct(&s.five_hour), reset(&s.five_hour)),
            (None, Some((T0 + 10 * H) as u64))
        );
        assert_eq!(s.five_hour.expect("리셋은 남는다").age_secs, 0);
    }

    #[test]
    fn reset_tolerance_is_sixty_seconds_inclusive() {
        for (delta, keeps) in [(60, true), (-60, true), (61, false), (-61, false)] {
            let mut b = seeded();
            let moved = T0 + 5 * H + delta;
            b.apply_passive(&passive(0, w(None, Some(moved)), None), at(200, T0));
            let s = snap(&b, 0, at(200, T0));
            assert_eq!(pct(&s.five_hour), keeps.then_some(40.0), "차 {delta}초");
            assert_eq!(reset(&s.five_hour), Some(moved as u64), "차 {delta}초");
        }
    }

    #[test]
    fn learning_a_reset_keeps_pct() {
        let mut b = book();
        succeed(&mut b, 0, active(0, w(Some(40.0), None), None), at(100, T0));
        b.apply_passive(&passive(0, w(None, Some(T0 + H)), None), at(200, T0));
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(
            (pct(&s.five_hour), reset(&s.five_hour)),
            (Some(40.0), Some((T0 + H) as u64))
        );
    }

    #[test]
    fn passive_none_reset_keeps_and_pct_without_reset_replaces_only_pct() {
        let mut b = seeded();
        let before = b.revision(&key(0));
        let applied = b.apply_passive(&passive(0, w(None, None), None), at(200, T0));
        assert_eq!(applied, PassiveApplied::default());
        assert_eq!(b.revision(&key(0)), before);

        b.apply_passive(&passive(0, w(Some(55.0), None), None), at(300, T0));
        let s = snap(&b, 0, at(300, T0));
        assert_eq!(
            (pct(&s.five_hour), reset(&s.five_hour)),
            (Some(55.0), Some((T0 + 5 * H) as u64))
        );
        assert_eq!(s.five_hour.expect("창").age_secs, 0);
    }

    #[test]
    fn model_scoped_list_is_replaced_whole_and_kept_when_absent() {
        let mut b = book();
        let mut full = active(0, None, None);
        full.model_scoped = Some(vec![
            scoped("b", w(Some(2.0), None)),
            scoped("a", w(Some(1.0), None)),
            scoped("empty", w(None, None)),
        ]);
        succeed(&mut b, 0, full, at(100, T0));
        let labels = |b: &UsageBook| -> Vec<String> {
            snap(b, 0, at(100, T0))
                .model_scoped
                .into_iter()
                .map(|s| s.label)
                .collect()
        };
        assert_eq!(labels(&b), ["a", "b"], "라벨 순 · 값 없는 창은 빠진다");

        b.apply_passive(&passive(0, w(Some(5.0), None), None), at(150, T0));
        assert_eq!(labels(&b), ["a", "b"], "안 실린 목록은 유지");

        let mut one = passive(0, None, None);
        one.model_scoped = Some(vec![scoped("c", w(Some(9.0), None))]);
        b.apply_passive(&one, at(200, T0));
        assert_eq!(labels(&b), ["c"], "실린 목록은 전량 교체");

        let mut opus = active(0, None, None);
        opus.model_scoped = Some(vec![scoped("Opus", w(Some(30.0), None))]);
        succeed(&mut b, 0, opus, at(300, T0));
        assert_eq!(labels(&b), ["Opus"]);
        let mut emptied = active(0, None, None);
        emptied.model_scoped = Some(vec![]);
        succeed(&mut b, 0, emptied, at(400, T0));
        assert!(
            labels(&b).is_empty(),
            "빈 목록도 실린 것이다 — 사라진 모델 창을 지운다"
        );
    }

    // ── D12 ──

    #[test]
    fn two_fresh_windows_arriving_separately_reset_the_cooldown() {
        let mut b = seeded();
        let cd = cooldown(0);
        assert_eq!(b.next_auto(&key(0), at(100, T0)), Some(secs(100) + cd));

        let first = b.apply_passive(&passive(0, w(Some(41.0), None), None), at(200, T0));
        assert_eq!(
            first,
            PassiveApplied {
                changed: true,
                next_auto_changed: false
            }
        );
        assert_eq!(b.next_auto(&key(0), at(200, T0)), Some(secs(100) + cd));

        let second = b.apply_passive(&passive(0, None, w(Some(11.0), None)), at(300, T0));
        assert_eq!(
            second,
            PassiveApplied {
                changed: true,
                next_auto_changed: true
            }
        );
        assert_eq!(b.next_auto(&key(0), at(300, T0)), Some(secs(300) + cd));

        // It was reset, so the next two must gather again.
        b.apply_passive(&passive(0, w(Some(42.0), None), None), at(400, T0));
        assert_eq!(b.next_auto(&key(0), at(400, T0)), Some(secs(300) + cd));
    }

    #[test]
    fn a_finished_probe_is_a_new_reference_point_for_freshness() {
        let mut b = book();
        let cd = cooldown(0);
        b.apply_passive(&passive(0, w(Some(1.0), None), None), at(50, T0));
        succeed(
            &mut b,
            0,
            active(0, w(Some(1.0), None), w(Some(2.0), None)),
            at(100, T0),
        );
        b.apply_passive(&passive(0, None, w(Some(3.0), None)), at(150, T0));
        assert_eq!(
            b.next_auto(&key(0), at(150, T0)),
            Some(secs(100) + cd),
            "조회 전의 새로 들어옴은 안 센다"
        );
    }

    #[test]
    fn only_windows_carrying_pct_count_as_fresh() {
        let mut b = seeded();
        b.apply_passive(&passive(0, w(Some(41.0), None), None), at(200, T0));
        let applied = b.apply_passive(
            &passive(0, None, w(None, Some(T0 + 100 * H + 10))),
            at(300, T0),
        );
        assert!(!applied.next_auto_changed);
        assert_eq!(
            b.next_auto(&key(0), at(300, T0)),
            Some(secs(100) + cooldown(0))
        );
    }

    #[test]
    fn fresh_windows_without_a_reference_point_start_the_cooldown() {
        let mut b = book();
        assert_eq!(
            b.next_auto(&key(0), at(70, T0)),
            Some(secs(70)),
            "기준점 없음 = 지금"
        );
        let applied = b.apply_passive(
            &passive(0, w(Some(1.0), None), w(Some(2.0), None)),
            at(70, T0),
        );
        assert!(applied.next_auto_changed);
        assert_eq!(
            b.next_auto(&key(0), at(80, T0)),
            Some(secs(70) + cooldown(0))
        );
    }

    #[test]
    fn a_passive_cooldown_reset_does_not_shorten_a_rejection() {
        let mut b = seeded();
        fail(
            &mut b,
            0,
            ProbeError::RateLimited {
                retry_after: Some(REJECT_MAX),
            }
            .into(),
            at(100, T0),
        );
        let until = secs(100) + REJECT_MAX;
        b.apply_passive(
            &passive(0, w(Some(41.0), None), w(Some(11.0), None)),
            at(200, T0),
        );
        assert_eq!(
            b.next_auto(&key(0), at(200, T0)),
            Some((secs(200) + cooldown(0)).max(until))
        );
        assert_eq!(snap(&b, 0, at(200, T0)).state, UsageVendorState::Ready);
    }

    // ── State (R22·R31·§3 #52) ──

    #[test]
    fn failure_keeps_values_and_folds_by_error() {
        let cases = [
            (ProbeError::NotInstalled, "NotInstalled"),
            (ProbeError::Unauthenticated, "NeedsLogin"),
            (ProbeError::RateLimited { retry_after: None }, "Rejected"),
            (ProbeError::Timeout, "Failed"),
            (ProbeError::Unsupported, "Failed"),
            (ProbeError::Spawn("x".to_owned()), "Failed"),
            (ProbeError::Io("x".to_owned()), "Failed"),
            (ProbeError::Parse("x".to_owned()), "Failed"),
            (ProbeError::Upstream("x".to_owned()), "Failed"),
        ];
        for (error, expected) in cases {
            let mut b = seeded();
            let kind = error.kind_word();
            fail(&mut b, 0, error.into(), at(200, T0));
            let s = snap(&b, 0, at(200, T0));
            assert_eq!(tag(&s.state), expected, "{kind}");
            assert_eq!(
                (pct(&s.five_hour), pct(&s.weekly)),
                (Some(40.0), Some(10.0)),
                "{kind}"
            );
            let detail = detail_of(&s.state).expect("비정상 상태는 detail 을 든다");
            assert_eq!(
                (
                    detail.kind.as_str(),
                    detail.code,
                    detail.upstream.as_deref()
                ),
                (kind, None, None),
                "근거가 없으면 분류 낱말만"
            );
        }
    }

    #[test]
    fn failure_detail_is_carried_as_given() {
        let mut b = seeded();
        fail(
            &mut b,
            0,
            ProbeFailure::with_detail(ProbeError::Timeout, rich_detail()),
            at(200, T0),
        );
        let s = snap(&b, 0, at(200, T0));
        let detail = detail_of(&s.state).expect("detail");
        assert_eq!(
            (
                detail.kind.as_str(),
                detail.code,
                detail.upstream.as_deref()
            ),
            ("some_kind", Some(429), Some("slow down"))
        );
    }

    #[test]
    fn limits_unavailable_clears_windows_and_waits_a_normal_cooldown() {
        let mut b = seeded();
        let mut unavailable = active(0, None, None);
        unavailable.plan = Some("plan-b".to_owned());
        unavailable.limits_unavailable = Some(UsageDetail {
            kind: "no_limits_kind",
            code: None,
            upstream: Some(UpstreamText::new("no limits here")),
        });
        succeed(&mut b, 0, unavailable, at(1_000, T0));

        let s = snap(&b, 0, at(1_000, T0));
        assert_eq!((s.five_hour, s.weekly), (None, None));
        assert!(s.model_scoped.is_empty());
        assert_eq!(s.plan.as_deref(), Some("plan-b"));
        assert_eq!(
            s.state,
            UsageVendorState::Unavailable {
                detail: Some(UsageStateDetail {
                    kind: "no_limits_kind".to_owned(),
                    code: None,
                    upstream: Some("no limits here".to_owned()),
                })
            }
        );
        assert_eq!(
            b.next_auto(&key(0), at(1_000, T0)),
            Some(secs(1_000) + cooldown(0))
        );

        // An answer that carries no plan keeps the plan it held.
        let mut no_plan = active(0, None, None);
        no_plan.limits_unavailable = Some(rich_detail());
        succeed(&mut b, 0, no_plan, at(2_000, T0));
        let s = snap(&b, 0, at(2_000, T0));
        assert_eq!(tag(&s.state), "Unavailable");
        assert_eq!(s.plan.as_deref(), Some("plan-b"));
    }

    #[test]
    fn ready_clears_detail() {
        let mut b = seeded();
        fail(
            &mut b,
            0,
            ProbeFailure::with_detail(ProbeError::Timeout, rich_detail()),
            at(200, T0),
        );
        succeed(&mut b, 0, active(0, w(Some(50.0), None), None), at(300, T0));
        assert_eq!(snap(&b, 0, at(300, T0)).state, UsageVendorState::Ready);
    }

    fn unavailable() -> UsageBook {
        let mut b = book();
        let mut unavailable = active(0, None, None);
        unavailable.limits_unavailable = Some(rich_detail());
        succeed(&mut b, 0, unavailable, at(100, T0));
        b
    }

    #[test]
    fn unavailable_turns_ready_on_any_window_value() {
        // An observation with no window values (plan only · a per-model window without a value) does not change the state — the plan is merged.
        let mut b = unavailable();
        let mut no_value = passive(0, w(None, None), None);
        no_value.plan = Some("plan-a".to_owned());
        no_value.model_scoped = Some(vec![scoped("m", w(None, None))]);
        assert!(b.apply_passive(&no_value, at(200, T0)).changed);
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(tag(&s.state), "Unavailable");
        assert_eq!(s.plan.as_deref(), Some("plan-a"));
        assert_eq!((s.five_hour, s.weekly), (None, None));
        assert!(s.model_scoped.is_empty());

        // Even a carried reset alone is a value.
        let applied = b.apply_passive(&passive(0, w(None, Some(T0 + H)), None), at(300, T0));
        assert!(applied.changed);
        let s = snap(&b, 0, at(300, T0));
        assert_eq!(s.state, UsageVendorState::Ready);
        assert_eq!(
            (pct(&s.five_hour), reset(&s.five_hour)),
            (None, Some((T0 + H) as u64))
        );

        // The same goes for a % · a per-model window's value · a query success.
        let mut b = unavailable();
        assert!(
            b.apply_passive(&passive(0, None, w(Some(7.0), None)), at(200, T0))
                .changed
        );
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(
            (s.state, pct(&s.weekly)),
            (UsageVendorState::Ready, Some(7.0))
        );

        let mut b = unavailable();
        let mut scoped_value = passive(0, None, None);
        scoped_value.model_scoped = Some(vec![scoped("m", w(None, Some(T0 + H)))]);
        assert!(b.apply_passive(&scoped_value, at(200, T0)).changed);
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(s.state, UsageVendorState::Ready);
        assert_eq!(s.model_scoped.len(), 1);

        let mut b = unavailable();
        succeed(&mut b, 0, active(0, None, None), at(200, T0));
        assert_eq!(snap(&b, 0, at(200, T0)).state, UsageVendorState::Ready);
    }

    #[test]
    fn a_value_turns_every_abnormal_state_ready_and_keeps_merging() {
        let errors = [
            ProbeError::NotInstalled,
            ProbeError::Unauthenticated,
            ProbeError::RateLimited { retry_after: None },
            ProbeError::Timeout,
        ];
        for error in errors {
            let mut b = seeded();
            let kind = error.kind_word();
            fail(
                &mut b,
                0,
                ProbeFailure::with_detail(error, rich_detail()),
                at(200, T0),
            );
            assert_ne!(tag(&snap(&b, 0, at(250, T0)).state), "Ready", "{kind}");
            let applied = b.apply_passive(&passive(0, w(Some(70.0), None), None), at(250, T0));
            assert!(applied.changed, "{kind}");
            let s = snap(&b, 0, at(250, T0));
            assert_eq!(s.state, UsageVendorState::Ready, "{kind}: detail 도 지운다");
            assert_eq!(
                (pct(&s.five_hour), pct(&s.weekly)),
                (Some(70.0), Some(10.0)),
                "{kind}: 실린 창만 덮는다"
            );
        }

        // The same goes for an ended refusal (after it was stamped as `Failed`).
        let mut b = seeded();
        fail(&mut b, 0, rate_limited(Some(secs(90))), at(200, T0));
        assert!(b.eval_time(&key(0), at(290, T0)));
        assert_eq!(tag(&snap(&b, 0, at(290, T0)).state), "Failed");
        b.apply_passive(&passive(0, None, w(None, Some(T0 + 100 * H))), at(300, T0));
        assert_eq!(snap(&b, 0, at(300, T0)).state, UsageVendorState::Ready);
    }

    #[test]
    fn a_value_during_a_rejection_shows_ready_but_keeps_the_deadline() {
        let mut b = settled();
        fail(&mut b, 0, rate_limited(Some(secs(90))), at(110, T0));
        let until = secs(200);
        assert_eq!(reject_until(&b, 0), Some(until));
        b.apply_passive(&passive(0, w(Some(41.0), None), None), at(150, T0));
        assert_eq!(snap(&b, 0, at(150, T0)).state, UsageVendorState::Ready);

        // The deadlines live on as they are — both the ⟳ refusal and the next automatic deadline.
        assert_eq!(reject_until(&b, 0), Some(until));
        let before_end = Now {
            mono: until - Duration::from_millis(1),
            wall: T0,
        };
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, before_end),
            Judgment::Rejected
        );
        assert!(b.next_auto(&key(0), before_end) >= Some(until));
        assert!(tick(&mut b, &[0], before_end).start.is_empty());

        // A refusal end does not turn `Ready` back into a query failure — there is nothing to stamp or emit.
        let end = Now {
            mono: until,
            wall: T0,
        };
        let rev = b.revision(&key(0));
        assert!(!b.eval_time(&key(0), end));
        assert_eq!(b.revision(&key(0)), rev);
        assert_eq!(snap(&b, 0, end).state, UsageVendorState::Ready);
        assert_eq!(judge(&mut b, 0, RequestKind::Refresh, end), Judgment::Start);
    }

    #[test]
    fn rejection_ending_folds_to_failed_and_keeps_detail() {
        let mut b = seeded();
        let wait = secs(90);
        fail(
            &mut b,
            0,
            ProbeFailure::with_detail(
                ProbeError::RateLimited {
                    retry_after: Some(wait),
                },
                rich_detail(),
            ),
            at(100, T0),
        );
        let expected_detail = Some(wire_detail(&rich_detail()));

        let s = snap(&b, 0, at(100, T0));
        assert_eq!(
            s.state,
            UsageVendorState::Rejected {
                retry_in_secs: 90,
                detail: expected_detail.clone()
            }
        );

        let almost = Now {
            mono: secs(189) + Duration::from_millis(500),
            wall: T0,
        };
        let s = b.snapshot(&key(0), almost).expect("키");
        assert_eq!(
            s.state,
            UsageVendorState::Rejected {
                retry_in_secs: 1,
                detail: expected_detail.clone()
            }
        );

        let s = snap(&b, 0, at(190, T0));
        let next = (secs(100) + cooldown(0)).max(secs(190));
        assert_eq!(
            s.state,
            UsageVendorState::Failed {
                next_attempt_in_secs: (next - secs(190)).as_secs(),
                detail: expected_detail,
            }
        );
    }

    #[test]
    fn six_states_fold_with_next_attempt_always_present() {
        let failures: [(Option<ProbeError>, &str); 5] = [
            (None, "Ready"),
            (Some(ProbeError::NotInstalled), "NotInstalled"),
            (Some(ProbeError::Unauthenticated), "NeedsLogin"),
            (Some(ProbeError::Timeout), "Failed"),
            (
                Some(ProbeError::RateLimited { retry_after: None }),
                "Rejected",
            ),
        ];
        for (error, expected) in failures {
            let mut b = book();
            match error {
                Some(error) => {
                    fail(&mut b, 0, error.into(), at(100, T0));
                }
                None => succeed(&mut b, 0, active(0, None, None), at(100, T0)),
            }
            let s = snap(&b, 0, at(100, T0));
            assert_eq!(tag(&s.state), expected);
            assert_eq!(
                detail_of(&s.state).is_some(),
                expected != "Ready",
                "{expected}"
            );
        }
        let mut b = book();
        let mut unavailable = active(0, None, None);
        unavailable.limits_unavailable = Some(rich_detail());
        succeed(&mut b, 0, unavailable, at(100, T0));
        assert_eq!(tag(&snap(&b, 0, at(100, T0)).state), "Unavailable");

        // Next attempt = until the next automatic deadline — stops at 0 once past.
        let mut b = book();
        fail(&mut b, 0, ProbeError::Timeout.into(), at(100, T0));
        let due = secs(100) + cooldown(0);
        let next_attempt = |b: &UsageBook, now: Now| match snap(b, 0, now).state {
            UsageVendorState::Failed {
                next_attempt_in_secs,
                ..
            } => next_attempt_in_secs,
            other => panic!("Failed 여야 한다: {other:?}"),
        };
        assert_eq!(next_attempt(&b, at(100, T0)), cooldown(0).as_secs());
        assert_eq!(
            next_attempt(
                &b,
                Now {
                    mono: due,
                    wall: T0
                }
            ),
            0
        );
        assert_eq!(
            next_attempt(
                &b,
                Now {
                    mono: due + secs(3_600),
                    wall: T0
                }
            ),
            0
        );
    }

    #[test]
    fn rate_limit_wait_falls_back_and_is_clamped() {
        let cases = [
            (None, REJECT_FALLBACK),
            (Some(Duration::ZERO), REJECT_FALLBACK),
            (Some(secs(90)), secs(90)),
            (Some(REJECT_MAX + secs(1)), REJECT_MAX),
            (Some(Duration::MAX), REJECT_MAX),
        ];
        for (retry_after, expected) in cases {
            let mut b = book();
            fail(
                &mut b,
                0,
                ProbeError::RateLimited { retry_after }.into(),
                at(100, T0),
            );
            assert_eq!(
                reject_until(&b, 0),
                Some(secs(100) + expected),
                "{retry_after:?}"
            );
            let s = snap(&b, 0, at(100, T0));
            assert!(
                matches!(s.state, UsageVendorState::Rejected { retry_in_secs, .. } if retry_in_secs == expected.as_secs()),
                "{retry_after:?}: {:?}",
                s.state
            );
            assert_eq!(
                b.next_auto(&key(0), at(100, T0)),
                Some((secs(100) + cooldown(0)).max(secs(100) + expected))
            );
        }

        // A failure that is not a refusal does not touch the deadline.
        let mut b = book();
        fail(&mut b, 0, rate_limited(Some(secs(90))), at(100, T0));
        fail(&mut b, 0, ProbeError::Timeout.into(), at(150, T0));
        assert_eq!(reject_until(&b, 0), Some(secs(190)));
    }

    // ── Latch (R32·§3 #29) ──

    #[test]
    fn latch_sets_once_after_reset_and_survives_a_wall_rewind() {
        let mut b = book();
        let r = T0 + 100;
        succeed(
            &mut b,
            0,
            active(0, w(Some(40.0), Some(r)), None),
            at(10, T0),
        );
        let rev = b.revision(&key(0)).expect("키");

        assert!(!b.eval_time(&key(0), at(20, r - 1)));
        assert_eq!(b.revision(&key(0)), Some(rev));
        assert!(b.eval_time(&key(0), at(30, r)));
        assert_eq!(b.revision(&key(0)), Some(rev + 1));
        assert!(!b.eval_time(&key(0), at(40, r + 5)), "한 번만 선다");
        assert_eq!(b.revision(&key(0)), Some(rev + 1));

        // Kept even if the wall clock rewinds · the % is not erased.
        assert!(!b.eval_time(&key(0), at(50, T0)));
        let s = snap(&b, 0, at(50, T0));
        assert!(expired(&s.five_hour));
        assert_eq!(pct(&s.five_hour), Some(40.0));
    }

    #[test]
    fn latch_is_cleared_only_by_a_new_value() {
        let mut b = book();
        let r = T0 + 100;
        succeed(
            &mut b,
            0,
            active(0, w(Some(40.0), Some(r)), None),
            at(10, T0),
        );
        assert!(b.eval_time(&key(0), at(20, r)));

        b.apply_passive(&passive(0, w(None, Some(r + 30)), None), at(30, T0));
        assert!(
            expired(&snap(&b, 0, at(30, T0)).five_hour),
            "같은 리셋만 — 새 값이 아니다"
        );
        b.apply_passive(&passive(0, w(None, Some(r + 5 * H)), None), at(40, T0));
        let s = snap(&b, 0, at(40, T0));
        assert!(
            expired(&s.five_hour),
            "바뀐 리셋만 — % 는 지워도 새 값은 아니다"
        );
        assert_eq!(pct(&s.five_hour), None);

        b.apply_passive(&passive(0, w(Some(3.0), None), None), at(50, T0));
        assert!(
            !expired(&snap(&b, 0, at(50, T0)).five_hour),
            "줍기 % 가 내린다"
        );

        // A query success also lowers it.
        assert!(b.eval_time(&key(0), at(60, r + 5 * H)));
        succeed(
            &mut b,
            0,
            active(0, w(Some(4.0), Some(r + 10 * H)), None),
            at(70, r + 5 * H),
        );
        assert!(!expired(&snap(&b, 0, at(70, r + 5 * H)).five_hour));

        // Even on a query success, if that window was not carried (null · missing) the latch stays — releasing it would revive the old %.
        let later = r + 10 * H;
        assert!(b.eval_time(&key(0), at(80, later)));
        succeed(
            &mut b,
            0,
            active(0, w(None, None), w(Some(5.0), None)),
            at(90, later),
        );
        let s = snap(&b, 0, at(90, later));
        assert!(expired(&s.five_hour));
        assert_eq!(pct(&s.five_hour), Some(4.0));
        assert!(!expired(&s.weekly));
    }

    #[test]
    fn latch_covers_every_window_and_a_past_reset_latches_on_arrival() {
        let mut b = book();
        let r = T0 + 100;
        let mut full = active(0, w(Some(1.0), Some(r + H)), w(Some(2.0), Some(r)));
        full.model_scoped = Some(vec![scoped("m", w(Some(3.0), Some(r)))]);
        succeed(&mut b, 0, full, at(10, T0));
        assert!(b.eval_time(&key(0), at(20, r)));
        let s = snap(&b, 0, at(20, r));
        assert!(!expired(&s.five_hour));
        assert!(expired(&s.weekly));
        assert!(s.model_scoped[0].window.expired);

        // If an already-past reset is received it is expired at the point received — for both pickup and query.
        let mut b = book();
        succeed(
            &mut b,
            0,
            active(0, w(Some(5.0), Some(T0 - 1)), None),
            at(10, T0),
        );
        assert!(expired(&snap(&b, 0, at(10, T0)).five_hour));
        let applied = b.apply_passive(&passive(0, None, w(Some(6.0), Some(T0 - 1))), at(20, T0));
        assert!(applied.changed);
        assert!(expired(&snap(&b, 0, at(20, T0)).weekly));
    }

    // ── revision(§3 #49) ──

    #[test]
    fn revision_rises_on_change_and_not_on_a_repeat() {
        let mut b = book();
        assert_eq!(b.revision(&key(0)), Some(0));
        let o = passive(0, w(Some(40.0), Some(T0 + H)), None);
        assert!(b.apply_passive(&o, at(10, T0)).changed);
        assert_eq!(b.revision(&key(0)), Some(1));

        // Picking up the same value again only makes the age fresher.
        assert_eq!(snap(&b, 0, at(70, T0)).five_hour.expect("창").age_secs, 60);
        assert!(!b.apply_passive(&o, at(70, T0)).changed);
        assert_eq!(b.revision(&key(0)), Some(1));
        assert_eq!(snap(&b, 0, at(70, T0)).five_hour.expect("창").age_secs, 0);

        let mut planned = passive(0, None, None);
        planned.plan = Some("plan-a".to_owned());
        assert!(b.apply_passive(&planned, at(80, T0)).changed);
        assert!(!b.apply_passive(&planned, at(90, T0)).changed);
        assert_eq!(b.revision(&key(0)), Some(2));
    }

    #[test]
    fn revision_rises_when_only_the_next_auto_moves() {
        let mut b = book();
        b.apply_passive(
            &passive(0, w(Some(40.0), None), w(Some(10.0), None)),
            at(200, T0),
        );
        let rev = b.revision(&key(0)).expect("키");
        assert!(
            !b.apply_passive(&passive(0, w(Some(40.0), None), None), at(300, T0))
                .changed
        );
        // The values are the same but both windows came in new so the deadline moves (D12).
        let applied = b.apply_passive(&passive(0, None, w(Some(10.0), None)), at(400, T0));
        assert_eq!(
            applied,
            PassiveApplied {
                changed: true,
                next_auto_changed: true
            }
        );
        assert_eq!(b.revision(&key(0)), Some(rev + 1));
    }

    #[test]
    fn begin_and_finish_always_bump() {
        let mut b = book();
        assert!(b.begin_probe(&key(0)));
        assert_eq!(
            (b.revision(&key(0)), b.in_flight(&key(0))),
            (Some(1), Some(true))
        );
        assert!(!b.begin_probe(&key(0)), "이미 진행 중");
        assert_eq!(b.revision(&key(0)), Some(1));
        assert!(snap(&b, 0, at(0, T0)).in_flight);

        let o = active(0, w(Some(1.0), None), None);
        b.finish_probe(&key(0), Ok(o.clone()), at(10, T0));
        assert_eq!(
            (b.revision(&key(0)), b.in_flight(&key(0))),
            (Some(2), Some(false))
        );
        // Even with the same result the end rises · it applies even if not in flight.
        b.finish_probe(&key(0), Ok(o), at(10, T0));
        assert_eq!(b.revision(&key(0)), Some(3));
    }

    #[test]
    fn cells_are_independent() {
        let mut b = book();
        b.apply_passive(&passive(0, w(Some(40.0), None), None), at(10, T0));
        assert!(b.begin_probe(&key(0)));
        assert_eq!(b.revision(&key(1)), Some(0));
        assert_eq!(b.in_flight(&key(1)), Some(false));
        assert_eq!(snap(&b, 1, at(10, T0)).five_hour, None);
        assert!(b.begin_probe(&key(1)), "다른 칸은 따로 시작한다");
    }

    #[test]
    fn unknown_keys_are_no_ops() {
        let mut b = book();
        let stranger = UsageKey {
            vendor: UsageVendorKey::new("no-such-vendor"),
            account: UsageAccountKey::default(),
        };
        assert_eq!(b.revision(&stranger), None);
        assert_eq!(b.in_flight(&stranger), None);
        assert_eq!(b.next_auto(&stranger, at(0, T0)), None);
        assert_eq!(b.snapshot(&stranger, at(0, T0)), None);
        assert!(!b.begin_probe(&stranger));
        assert!(!b.eval_time(&stranger, at(0, T0)));
        b.finish_probe(
            &stranger,
            Err(ProbeError::RateLimited { retry_after: None }.into()),
            at(0, T0),
        );
        let mut o = passive(0, w(Some(1.0), None), None);
        o.vendor = stranger.vendor;
        assert_eq!(b.apply_passive(&o, at(0, T0)), PassiveApplied::default());
        for k in b.keys() {
            assert_eq!(b.revision(&k), Some(0));
        }
    }

    // ── Snapshot (§1-6) ──

    #[test]
    fn snapshot_carries_the_key_and_window_fields() {
        let mut b = book();
        for (i, k) in b.keys().iter().enumerate() {
            let s = snap(&b, i, at(0, T0));
            let word = serde_json::to_value(s.vendor).expect("직렬화");
            assert!(
                word.as_str()
                    .is_some_and(|w| w.eq_ignore_ascii_case(k.vendor.as_str())),
                "{word}"
            );
            assert_eq!(s.account_key, UsageAccountKey::default().as_str());
            assert_eq!(s.revision, 0);
        }

        succeed(
            &mut b,
            0,
            active(0, w(Some(40.0), Some(T0 + H)), None),
            at(100, T0),
        );
        b.apply_passive(&passive(0, None, w(Some(9.0), None)), at(130, T0));
        let s = snap(&b, 0, at(160, T0));
        assert_eq!(
            s.five_hour,
            Some(UsageWindow {
                used_pct: Some(40.0),
                resets_at: Some((T0 + H) as u64),
                age_secs: 60,
                expired: false,
            })
        );
        let weekly = s.weekly.expect("주간");
        assert_eq!((weekly.age_secs, weekly.resets_at), (30, None));
    }

    #[test]
    fn a_vendor_word_the_wire_cannot_name_is_not_registered() {
        let odd = UsageVendorKey::new("no-such-backend");
        let (mut b, loud) = capture_loud(|| {
            UsageBook::new(&[
                (odd, usage_probes()[0].policy()),
                (usage_probes()[0].key(), usage_probes()[0].policy()),
                (odd, usage_probes()[1].policy()),
                (usage_probes()[0].key(), usage_probes()[1].policy()),
            ])
        });
        assert_eq!(loud.len(), 1, "벤더마다 한 번: {loud:?}");
        assert_eq!(
            b.keys(),
            vec![key(0)],
            "못 바꾸는 벤더는 없고 같은 벤더는 한 칸"
        );
        let k = UsageKey {
            vendor: odd,
            account: UsageAccountKey::default(),
        };
        assert_eq!(b.revision(&k), None);
        assert!(!b.begin_probe(&k));
        assert_eq!(b.snapshot(&k, at(0, T0)), None);
        assert_eq!(b.broadcast_sheet(&k, at(0, T0)), None);
        assert_eq!(b.coalesce_passive(&k, at(0, T0)), None);
        assert_eq!(b.judge(&k, RequestKind::Get, at(0, T0)), None);
        let mut o = passive(0, w(Some(1.0), None), None);
        o.vendor = odd;
        assert_eq!(b.apply_passive(&o, at(0, T0)), PassiveApplied::default());
        assert_eq!(b.plan_tick(&[odd], at(0, T0)), TickPlan::default());
    }

    #[test]
    fn extreme_values_do_not_panic() {
        let mut b = book();
        // Reset boundary values — 0 or below is taken as absent · the difference between the two ends also does not overflow.
        succeed(
            &mut b,
            0,
            active(0, w(Some(1.0), Some(i64::MIN)), w(None, Some(i64::MAX))),
            at(0, i64::MIN),
        );
        let s = snap(&b, 0, at(0, i64::MIN));
        assert_eq!((pct(&s.five_hour), reset(&s.five_hour)), (Some(1.0), None));
        assert_eq!(reset(&s.weekly), Some(i64::MAX as u64));
        b.apply_passive(
            &passive(0, w(None, Some(1)), w(None, Some(i64::MIN))),
            at(1, i64::MIN),
        );
        let s = snap(&b, 0, at(1, i64::MIN));
        assert_eq!(
            (pct(&s.five_hour), reset(&s.five_hour)),
            (Some(1.0), Some(1))
        );
        assert_eq!(
            reset(&s.weekly),
            Some(i64::MAX as u64),
            "음수 리셋은 안 실린 것"
        );
        b.apply_passive(&passive(0, None, w(None, Some(1))), at(2, i64::MIN));
        assert_eq!(reset(&snap(&b, 0, at(2, i64::MIN)).weekly), Some(1));
        b.eval_time(&key(0), at(3, i64::MAX));
        b.eval_time(&key(0), at(4, i64::MIN));

        // Monotonic time boundary values — the refusal deadline · next automatic · age saturate.
        let mut b = book();
        let edge = Now {
            mono: Duration::MAX,
            wall: T0,
        };
        succeed(&mut b, 0, active(0, w(Some(1.0), None), None), edge);
        assert_eq!(b.next_auto(&key(0), edge), Some(Duration::MAX));
        fail(
            &mut b,
            0,
            ProbeError::RateLimited {
                retry_after: Some(Duration::MAX),
            }
            .into(),
            edge,
        );
        // A saturated deadline = now, so it is stamped as a query failure at the point received.
        let s = snap(&b, 0, edge);
        assert!(
            matches!(
                s.state,
                UsageVendorState::Failed {
                    next_attempt_in_secs: 0,
                    ..
                }
            ),
            "{:?}",
            s.state
        );
        // A time that went backwards — the age is 0 and the stamped state stays.
        let s = snap(&b, 0, at(0, T0));
        assert_eq!(s.five_hour.expect("창").age_secs, 0);
        assert!(
            matches!(s.state, UsageVendorState::Failed { .. }),
            "{:?}",
            s.state
        );

        // A % that broke the finite-number promise — even if the same NaN keeps coming it is not a change.
        let mut b = book();
        let nan = passive(0, w(Some(f64::NAN), None), None);
        assert!(b.apply_passive(&nan, at(0, T0)).changed);
        assert!(!b.apply_passive(&nan, at(1, T0)).changed);
    }

    // ── Review round 1 corrections ──

    #[test]
    fn rejection_ending_is_written_once_by_eval_time_and_keeps_detail() {
        let mut b = seeded();
        fail(
            &mut b,
            0,
            ProbeFailure::with_detail(ProbeError::RateLimited { retry_after: None }, rich_detail()),
            at(100, T0),
        );
        let until = secs(100) + REJECT_FALLBACK;
        let rev = b.revision(&key(0)).expect("키");
        let before_end = Now {
            mono: until - secs(1),
            wall: T0,
        };
        assert!(!b.eval_time(&key(0), before_end));
        assert_eq!(tag(&snap(&b, 0, before_end).state), "Rejected");

        let end = Now {
            mono: until,
            wall: T0,
        };
        let fallback = snap(&b, 0, end);
        assert!(b.eval_time(&key(0), end));
        assert_eq!(b.revision(&key(0)), Some(rev + 1));
        let written = snap(&b, 0, end);
        assert_eq!(written.state, fallback.state, "폴백과 새긴 값이 같다");
        assert_eq!(written.revision, fallback.revision + 1);
        assert_eq!(tag(&written.state), "Failed");
        assert_eq!(
            detail_of(&written.state),
            Some(&wire_detail(&rich_detail()))
        );

        let later = Now {
            mono: until + secs(60),
            wall: T0,
        };
        assert!(!b.eval_time(&key(0), later), "한 번만 새긴다");
        assert_eq!(b.revision(&key(0)), Some(rev + 1));
    }

    #[test]
    fn passive_application_also_writes_a_finished_rejection() {
        let mut b = seeded();
        fail(
            &mut b,
            0,
            ProbeError::RateLimited { retry_after: None }.into(),
            at(100, T0),
        );
        let end = Now {
            mono: secs(100) + REJECT_FALLBACK,
            wall: T0,
        };
        // A pickup with no window values — the only change is the refusal end.
        let mut plan_only = passive(0, None, None);
        plan_only.plan = Some("plan-a".to_owned());
        assert!(b.apply_passive(&plan_only, end).changed);
        assert!(!b.eval_time(&key(0), end));
        assert_eq!(tag(&snap(&b, 0, end).state), "Failed");
    }

    #[test]
    fn a_successful_probe_clears_the_rejection() {
        let mut b = seeded();
        let rate_limited = || {
            ProbeFailure::from(ProbeError::RateLimited {
                retry_after: Some(REJECT_MAX),
            })
        };
        fail(&mut b, 0, rate_limited(), at(100, T0));
        let mut unavailable = active(0, None, None);
        unavailable.limits_unavailable = Some(rich_detail());
        assert!(b.begin_probe(&key(0)));
        b.finish_probe(&key(0), Ok(unavailable), at(200, T0));
        assert_eq!(reject_until(&b, 0), None);
        assert_eq!(
            b.next_auto(&key(0), at(200, T0)),
            Some(secs(200) + cooldown(0))
        );

        fail(&mut b, 0, rate_limited(), at(300, T0));
        succeed(&mut b, 0, active(0, w(Some(1.0), None), None), at(400, T0));
        assert_eq!(
            b.next_auto(&key(0), at(400, T0)),
            Some(secs(400) + cooldown(0))
        );
        assert_eq!(snap(&b, 0, at(400, T0)).state, UsageVendorState::Ready);
    }

    #[test]
    fn non_positive_resets_are_read_as_absent() {
        let mut b = book();
        for bad in [0, -1, i64::MIN] {
            let applied = b.apply_passive(
                &passive(0, w(None, Some(bad)), w(None, Some(bad))),
                at(10, T0),
            );
            assert_eq!(applied, PassiveApplied::default(), "{bad}");
        }
        assert_eq!(b.revision(&key(0)), Some(0));
        let s = snap(&b, 0, at(10, T0));
        assert_eq!((s.five_hour, s.weekly), (None, None));

        succeed(
            &mut b,
            0,
            active(0, w(None, Some(-5)), w(Some(3.0), Some(0))),
            at(20, T0),
        );
        let s = snap(&b, 0, at(20, T0));
        assert_eq!(s.five_hour, None, "리셋만 있던 창은 안 선다");
        assert_eq!((pct(&s.weekly), reset(&s.weekly)), (Some(3.0), None));

        // An observation of 0 or below neither erases nor changes the reset held.
        b.apply_passive(&passive(0, None, w(None, Some(T0 + H))), at(30, T0));
        let rev = b.revision(&key(0));
        assert!(
            !b.apply_passive(&passive(0, None, w(None, Some(-1))), at(40, T0))
                .changed
        );
        assert_eq!(b.revision(&key(0)), rev);
        assert_eq!(
            reset(&snap(&b, 0, at(40, T0)).weekly),
            Some((T0 + H) as u64)
        );
    }

    #[test]
    fn both_entries_merge_whatever_the_observation_says_its_source_is() {
        let mut b = seeded();
        let (applied, loud) =
            capture_loud(|| b.apply_passive(&active(0, w(Some(99.0), None), None), at(200, T0)));
        assert!(applied.changed);
        assert!(loud.is_empty(), "{loud:?}");
        let s = snap(&b, 0, at(200, T0));
        assert_eq!(
            (pct(&s.five_hour), pct(&s.weekly)),
            (Some(99.0), Some(10.0))
        );

        assert!(b.begin_probe(&key(0)));
        let (_, loud) = capture_loud(|| {
            b.finish_probe(
                &key(0),
                Ok(passive(0, w(Some(5.0), None), None)),
                at(300, T0),
            )
        });
        assert!(loud.is_empty(), "{loud:?}");
        assert_eq!(b.in_flight(&key(0)), Some(false));
        assert_eq!(
            b.next_auto(&key(0), at(300, T0)),
            Some(secs(300) + cooldown(0))
        );
        let s = snap(&b, 0, at(300, T0));
        assert_eq!((pct(&s.five_hour), pct(&s.weekly)), (Some(5.0), Some(10.0)));
    }

    #[test]
    fn model_scoped_entries_are_capped() {
        let over = MODEL_SCOPED_MAX + 4;
        let many = |pct_from: usize| -> Vec<ScopedWindowObs> {
            (0..over)
                .map(|i| {
                    let pct = (i >= pct_from).then_some(i as f64);
                    scoped(&format!("m{i:02}"), w(pct, Some(T0 + H)))
                })
                .collect()
        };
        let mut b = book();
        let mut o = passive(0, None, None);
        o.model_scoped = Some(many(0));
        let (_, loud) = capture_loud(|| b.apply_passive(&o, at(10, T0)));
        assert_eq!(loud.len(), 1, "생산자가 약속을 어겼다: {loud:?}");
        let s = snap(&b, 0, at(10, T0));
        assert_eq!(s.model_scoped.len(), MODEL_SCOPED_MAX);
        assert_eq!(
            s.model_scoped.last().map(|m| m.label.clone()),
            Some(format!("m{:02}", MODEL_SCOPED_MAX - 1)),
            "앞에서부터 받는다"
        );

        let mut full = active(0, None, None);
        full.model_scoped = Some(many(0));
        let ((), loud) = capture_loud(|| succeed(&mut b, 0, full, at(20, T0)));
        assert_eq!(loud.len(), 1, "{loud:?}");
        assert_eq!(snap(&b, 0, at(20, T0)).model_scoped.len(), MODEL_SCOPED_MAX);

        // The value of a discarded entry is not counted even as proof of normal.
        let mut b = unavailable();
        let mut beyond = passive(0, None, None);
        beyond.model_scoped = Some(
            (0..over)
                .map(|i| {
                    let value = (i >= MODEL_SCOPED_MAX).then_some(i as f64);
                    scoped(&format!("m{i:02}"), w(value, None))
                })
                .collect(),
        );
        let (applied, _) = capture_loud(|| b.apply_passive(&beyond, at(200, T0)));
        assert_eq!(applied, PassiveApplied::default());
        assert_eq!(tag(&snap(&b, 0, at(200, T0)).state), "Unavailable");
    }

    #[test]
    fn a_pct_arriving_after_the_reset_drops_the_stale_reset() {
        let r = T0 + 100;
        let mut b = book();
        succeed(
            &mut b,
            0,
            active(0, w(Some(40.0), Some(r)), None),
            at(10, T0),
        );
        assert!(b.eval_time(&key(0), at(20, r)));
        b.apply_passive(&passive(0, w(Some(3.0), None), None), at(30, r + 10));
        let five = snap(&b, 0, at(30, r + 10)).five_hour.expect("창");
        assert_eq!(
            (five.used_pct, five.resets_at, five.expired),
            (Some(3.0), None, false)
        );

        // A % that came before the reset protects the reset held.
        let mut b = book();
        succeed(
            &mut b,
            0,
            active(0, w(Some(40.0), Some(r)), None),
            at(10, T0),
        );
        b.apply_passive(&passive(0, w(Some(41.0), None), None), at(20, r - 1));
        assert_eq!(reset(&snap(&b, 0, at(20, r - 1)).five_hour), Some(r as u64));
    }

    #[test]
    fn a_cleared_pct_no_longer_counts_as_fresh() {
        let mut b = seeded();
        b.apply_passive(&passive(0, w(Some(41.0), None), None), at(200, T0));
        // A changed reset erases the 5-hour window's % — also lowers the newly-arrived mark that stood earlier.
        b.apply_passive(&passive(0, w(None, Some(T0 + 10 * H)), None), at(250, T0));
        let applied = b.apply_passive(&passive(0, None, w(Some(11.0), None)), at(300, T0));
        assert!(!applied.next_auto_changed);
        assert_eq!(
            b.next_auto(&key(0), at(300, T0)),
            Some(secs(100) + cooldown(0))
        );
    }

    fn at_ms(mono_ms: u64, wall: i64) -> Now {
        Now {
            mono: Duration::from_millis(mono_ms),
            wall,
        }
    }

    /// The premise of [`seeded`] + test arithmetic — within a few minutes after the query end (mono 100) the next automatic deadline does not come.
    fn settled() -> UsageBook {
        assert!(
            cooldown(0) > secs(300) + SCHEDULE_MAX_SLEEP,
            "시험 전제: 쿨타임이 몇 분보다 길다"
        );
        seeded()
    }

    fn judge(b: &mut UsageBook, i: usize, kind: RequestKind, now: Now) -> Judgment {
        b.judge(&key(i), kind, now).expect("아는 키")
    }

    fn rate_limited(retry_after: Option<Duration>) -> ProbeFailure {
        ProbeError::RateLimited { retry_after }.into()
    }

    /// One scheduler pass + invariants: sleep ∈ (0, cap] · running again at the same moment finds nothing to do (no 0-second sleep repetition).
    fn tick(b: &mut UsageBook, subscribed: &[usize], now: Now) -> TickPlan {
        let vendors: Vec<UsageVendorKey> = subscribed.iter().map(|&i| key(i).vendor).collect();
        let plan = b.plan_tick(&vendors, now);
        assert!(
            plan.sleep > Duration::ZERO && plan.sleep <= SCHEDULE_MAX_SLEEP,
            "{plan:?}"
        );
        let again = b.plan_tick(&vendors, now);
        assert_eq!(
            again,
            TickPlan {
                sleep: plan.sleep,
                ..TickPlan::default()
            },
            "찬 기한은 한 번에 처리한다"
        );
        plan
    }

    // ── Decision (§1-4 request table) ──

    #[test]
    fn get_before_the_deadline_is_cached_and_starts_nothing() {
        let mut b = settled();
        let due = secs(100) + cooldown(0);
        let rev = b.revision(&key(0));
        let before = Now {
            mono: due - Duration::from_millis(1),
            wall: T0,
        };
        assert_eq!(judge(&mut b, 0, RequestKind::Get, before), Judgment::Cached);
        assert_eq!(
            (b.revision(&key(0)), b.in_flight(&key(0))),
            (rev, Some(false))
        );

        let at_due = Now {
            mono: due,
            wall: T0,
        };
        assert_eq!(judge(&mut b, 0, RequestKind::Get, at_due), Judgment::Start);
        assert_eq!(
            (b.revision(&key(0)), b.in_flight(&key(0))),
            (rev.map(|r| r + 1), Some(true))
        );

        // If there is no reference point, now is the deadline.
        let mut b = book();
        assert_eq!(
            judge(&mut b, 0, RequestKind::Get, at(0, T0)),
            Judgment::Start
        );
    }

    #[test]
    fn a_rejection_ending_or_a_reset_passing_does_not_pull_the_probe_forward() {
        let mut b = settled();
        fail(&mut b, 0, rate_limited(Some(secs(90))), at(200, T0));
        let ended = at(300, T0);
        assert!(b.eval_time(&key(0), ended), "거절 끝을 새긴다");
        assert_eq!(judge(&mut b, 0, RequestKind::Get, ended), Judgment::Cached);
        assert!(tick(&mut b, &[0], at(301, T0)).start.is_empty());

        let mut b = settled();
        let past_reset = at(300, T0 + 5 * H + 1);
        assert!(b.eval_time(&key(0), past_reset), "리셋이 지나 래치가 선다");
        assert_eq!(
            judge(&mut b, 0, RequestKind::Get, past_reset),
            Judgment::Cached
        );
        assert!(tick(&mut b, &[0], past_reset).start.is_empty());
        assert_eq!(b.in_flight(&key(0)), Some(false));
    }

    #[test]
    fn a_passive_cooldown_reset_during_a_rejection_cannot_shorten_it() {
        let mut b = settled();
        fail(&mut b, 0, rate_limited(Some(REJECT_MAX)), at(100, T0));
        let until = secs(100) + REJECT_MAX;
        b.apply_passive(
            &passive(0, w(Some(41.0), None), w(Some(11.0), None)),
            at(200, T0),
        );
        let after_cooldown = Now {
            mono: secs(200) + cooldown(0) + secs(1),
            wall: T0,
        };
        assert!(after_cooldown.mono < until);
        assert_eq!(
            judge(&mut b, 0, RequestKind::Get, after_cooldown),
            Judgment::Cached
        );
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, after_cooldown),
            Judgment::Rejected
        );
        assert!(tick(&mut b, &[0], after_cooldown).start.is_empty());

        let ended = Now {
            mono: until,
            wall: T0,
        };
        assert_eq!(judge(&mut b, 0, RequestKind::Get, ended), Judgment::Start);
    }

    /// ★There is no minimum interval (user decision 2026-09-29)★ — a ⟳ at the same moment as the query end is also a new query.
    #[test]
    fn refresh_ignores_the_cooldown_and_starts_right_after_a_finished_probe() {
        let mut b = settled();
        let done = at(100, T0);
        assert!(done.mono < secs(100) + cooldown(0), "쿨타임 전이다");
        assert_eq!(judge(&mut b, 0, RequestKind::Get, done), Judgment::Cached);
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, done),
            Judgment::Start
        );
        assert_eq!(b.in_flight(&key(0)), Some(true));

        b.finish_probe(&key(0), Ok(active(0, w(Some(1.0), None), None)), done);
        assert_eq!(b.in_flight(&key(0)), Some(false));
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, done),
            Judgment::Start,
            "끝난 그 순간에도"
        );

        let mut b = book();
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, at(0, T0)),
            Judgment::Start
        );
    }

    #[test]
    fn refresh_during_a_rejection_is_rejected_until_it_ends() {
        // A past time can only be raised to 0 by `Duration` — that share is the `Some(0)` cell.
        let cases = [
            (None, REJECT_FALLBACK),
            (Some(Duration::ZERO), REJECT_FALLBACK),
            (Some(secs(90)), secs(90)),
            (Some(secs(100 * 24 * 3_600)), REJECT_MAX),
        ];
        for (retry_after, wait) in cases {
            let mut b = book();
            fail(&mut b, 0, rate_limited(retry_after), at(100, T0));
            let before_end = Now {
                mono: secs(100) + wait - Duration::from_millis(1),
                wall: T0,
            };
            assert_eq!(
                judge(&mut b, 0, RequestKind::Refresh, before_end),
                Judgment::Rejected,
                "{retry_after:?}"
            );
            assert_eq!(b.in_flight(&key(0)), Some(false), "{retry_after:?}");
            let end = Now {
                mono: secs(100) + wait,
                wall: T0,
            };
            assert_eq!(
                judge(&mut b, 0, RequestKind::Refresh, end),
                Judgment::Start,
                "{retry_after:?}"
            );
        }
    }

    #[test]
    fn a_three_hour_mono_jump_starts_exactly_one_probe() {
        let mut b = settled();
        let jumped = at(100 + 3 * 3_600, T0);
        assert_eq!(judge(&mut b, 0, RequestKind::Get, jumped), Judgment::Start);
        assert_eq!(judge(&mut b, 0, RequestKind::Get, jumped), Judgment::Join);
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, jumped),
            Judgment::Join
        );
        assert!(tick(&mut b, &[0], jumped).start.is_empty());
    }

    #[test]
    fn a_wall_jump_does_not_change_the_judgment() {
        let judgments = |wall: i64| {
            let mut b = settled();
            let before_due = at(500, wall);
            let due = Now {
                mono: secs(100) + cooldown(0),
                wall,
            };
            b.eval_time(&key(0), before_due);
            [
                judge(&mut b, 0, RequestKind::Get, before_due),
                judge(&mut b, 0, RequestKind::Refresh, at(110, wall)),
                judge(&mut b, 0, RequestKind::Get, due),
            ]
        };
        let expected = [Judgment::Cached, Judgment::Start, Judgment::Join];
        for wall in [T0 - 2 * H, T0, T0 + 2 * H] {
            assert_eq!(judgments(wall), expected, "wall {wall}");
        }
    }

    #[test]
    fn a_timeout_fails_and_waits_a_normal_cooldown() {
        let mut b = settled();
        fail(&mut b, 0, ProbeError::Timeout.into(), at(200, T0));
        assert_eq!(tag(&snap(&b, 0, at(200, T0)).state), "Failed");
        let due = secs(200) + cooldown(0);
        assert_eq!(b.next_auto(&key(0), at(200, T0)), Some(due));
        let before = Now {
            mono: due - secs(1),
            wall: T0,
        };
        assert_eq!(judge(&mut b, 0, RequestKind::Get, before), Judgment::Cached);
        let at_due = Now {
            mono: due,
            wall: T0,
        };
        assert_eq!(judge(&mut b, 0, RequestKind::Get, at_due), Judgment::Start);
    }

    #[test]
    fn requests_join_a_probe_in_flight_only_past_the_earlier_rows() {
        let mut b = book();
        assert!(b.begin_probe(&key(0)));
        let rev = b.revision(&key(0));
        assert_eq!(
            judge(&mut b, 0, RequestKind::Get, at(0, T0)),
            Judgment::Join
        );
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, at(0, T0)),
            Judgment::Join
        );
        assert_eq!(b.revision(&key(0)), rev, "합류는 아무것도 안 바꾼다");

        // Exactly in table order — even when in flight, a `Get` before the deadline is the cache. ⟳ joins even if the previous query just ended.
        let mut b = settled();
        assert!(b.begin_probe(&key(0)));
        assert_eq!(
            judge(&mut b, 0, RequestKind::Get, at(100, T0)),
            Judgment::Cached
        );
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, at(100, T0)),
            Judgment::Join
        );

        let stranger = UsageKey {
            vendor: UsageVendorKey::new("no-such-vendor"),
            account: UsageAccountKey::default(),
        };
        assert_eq!(b.judge(&stranger, RequestKind::Get, at(0, T0)), None);
    }

    // ── Pickup coalescing (§3 #60) ──

    /// A pickup that changes only the 5-hour window % + coalescing — carries just one window so the D12 reset does not stand.
    fn passive_pct(b: &mut UsageBook, pct: f64, now: Now) -> Option<Coalesce> {
        assert!(
            b.apply_passive(&passive(0, w(Some(pct), None), None), now)
                .changed
        );
        b.coalesce_passive(&key(0), now)
    }

    #[test]
    fn passive_changes_within_a_second_publish_now_once_and_once_at_the_deadline() {
        let mut b = settled();
        let first = at_ms(200_000, T0);
        let now_sheet = passive_pct(&mut b, 50.0, first);
        assert_eq!(now_sheet, Some(Coalesce::PublishNow(snap(&b, 0, first))));
        for (pct, t) in [(51.0, 200_300), (52.0, 200_600)] {
            assert_eq!(
                passive_pct(&mut b, pct, at_ms(t, T0)),
                Some(Coalesce::Deferred),
                "{t}"
            );
        }

        let plan = tick(&mut b, &[0], at_ms(200_600, T0));
        assert!(plan.publish.is_empty());
        assert_eq!(plan.sleep, Duration::from_millis(400), "합침 기한에 깬다");

        let deadline = at_ms(201_000, T0);
        let plan = tick(&mut b, &[0], deadline);
        assert_eq!(
            plan.publish,
            vec![(key(0), snap(&b, 0, deadline))],
            "끝 발행 한 장"
        );
        assert_eq!(pct(&plan.publish[0].1.five_hour), Some(52.0), "마지막 값");
        assert!(plan.start.is_empty());
    }

    #[test]
    fn passive_changes_a_second_or_more_apart_publish_each() {
        let mut b = settled();
        for (n, t) in [200_000, 201_000, 202_500].into_iter().enumerate() {
            assert!(
                matches!(
                    passive_pct(&mut b, 50.0 + n as f64, at_ms(t, T0)),
                    Some(Coalesce::PublishNow(_))
                ),
                "{t}"
            );
        }
        assert!(tick(&mut b, &[0], at_ms(202_600, T0)).publish.is_empty());
    }

    #[test]
    fn any_other_broadcast_before_the_deadline_carries_and_clears_the_pending_change() {
        let mut b = settled();
        passive_pct(&mut b, 50.0, at_ms(200_000, T0));
        assert_eq!(
            passive_pct(&mut b, 51.0, at_ms(200_300, T0)),
            Some(Coalesce::Deferred)
        );
        // The query-end emission goes out first — that one sheet carries the value that was held back.
        let sheet = b
            .broadcast_sheet(&key(0), at_ms(200_500, T0))
            .expect("한 장");
        assert_eq!(pct(&sheet.five_hour), Some(51.0));
        let plan = tick(&mut b, &[0], at_ms(201_000, T0));
        assert!(plan.publish.is_empty(), "{plan:?}");
        assert_eq!(
            plan.sleep, SCHEDULE_MAX_SLEEP,
            "대기가 없으니 합침 기한도 없다"
        );

        // The window is measured again from that emission.
        assert_eq!(
            passive_pct(&mut b, 52.0, at_ms(201_200, T0)),
            Some(Coalesce::Deferred)
        );
        assert_eq!(
            tick(&mut b, &[0], at_ms(201_200, T0)).sleep,
            Duration::from_millis(300)
        );

        // A late-read time does not rewind the previous emission.
        b.broadcast_sheet(&key(0), at_ms(203_000, T0));
        b.broadcast_sheet(&key(0), at_ms(202_000, T0));
        assert_eq!(
            passive_pct(&mut b, 53.0, at_ms(203_500, T0)),
            Some(Coalesce::Deferred)
        );

        let stranger = UsageKey {
            vendor: UsageVendorKey::new("no-such-vendor"),
            account: UsageAccountKey::default(),
        };
        assert_eq!(b.coalesce_passive(&stranger, at(0, T0)), None);
        assert_eq!(b.broadcast_sheet(&stranger, at(0, T0)), None);
    }

    #[test]
    fn broadcast_sheet_evaluates_marks_and_snapshots_in_one_call() {
        let mut b = settled();
        let rev = b.revision(&key(0)).expect("키");
        let past_reset = at(200, T0 + 5 * H);
        let sheet = b.broadcast_sheet(&key(0), past_reset).expect("한 장");
        assert!(expired(&sheet.five_hour), "래치를 새긴 뒤 뜬다");
        assert_eq!(sheet.revision, rev + 1);
        assert_eq!(sheet, snap(&b, 0, past_reset));
        let unchanged = b.broadcast_sheet(&key(0), past_reset).expect("한 장");
        assert_eq!(
            unchanged.revision,
            rev + 1,
            "바뀐 게 없으면 revision 그대로"
        );

        // It was marked too — a pickup within 1 second is deferred.
        assert_eq!(
            passive_pct(&mut b, 60.0, at_ms(200_400, T0 + 5 * H)),
            Some(Coalesce::Deferred)
        );
        // Carry the value that was held back and repay — if no debt remains after that, the coalescing entrance has nothing to send either.
        let sheet = b
            .broadcast_sheet(&key(0), at_ms(200_500, T0 + 5 * H))
            .expect("한 장");
        assert_eq!(pct(&sheet.five_hour), Some(60.0));
        assert_eq!(
            b.coalesce_passive(&key(0), at_ms(200_600, T0 + 5 * H)),
            Some(Coalesce::Deferred)
        );
        assert!(tick(&mut b, &[0], at_ms(202_000, T0 + 5 * H))
            .publish
            .is_empty());
    }

    #[test]
    fn a_value_that_ends_a_visible_rejection_is_coalesced_unless_it_latches() {
        for latches in [false, true] {
            let mut b = settled();
            fail(&mut b, 0, rate_limited(Some(REJECT_MAX)), at(110, T0));
            b.broadcast_sheet(&key(0), at(200, T0));
            // If the wall clock is past the 5-hour window's reset (T0 + 5H), the same call stamps the latch.
            let wall = if latches { T0 + 5 * H } else { T0 };
            let now = at_ms(200_300, wall);
            let applied = b.apply_passive(&passive(0, None, w(Some(11.0), None)), now);
            assert!(applied.changed, "latches {latches}");
            let coalesced = b.coalesce_passive(&key(0), now);
            if latches {
                match coalesced {
                    Some(Coalesce::PublishNow(sheet)) => {
                        assert_eq!(sheet.state, UsageVendorState::Ready);
                        assert!(expired(&sheet.five_hour));
                    }
                    other => panic!("래치는 합치지 않는다: {other:?}"),
                }
            } else {
                assert_eq!(
                    coalesced,
                    Some(Coalesce::Deferred),
                    "상태가 바뀌어도 값의 바뀜이다"
                );
                let plan = tick(&mut b, &[0], at_ms(201_000, wall));
                assert_eq!(plan.publish.len(), 1, "합침 기한에 한 장");
                assert_eq!(plan.publish[0].1.state, UsageVendorState::Ready);
            }
        }
    }

    #[test]
    fn a_passive_that_latches_is_owed_at_once_even_inside_the_window() {
        let mut b = settled();
        passive_pct(&mut b, 50.0, at(200, T0));
        // A pickup that changed only a value is coalesced.
        let calm = at_ms(200_300, T0);
        assert!(
            b.apply_passive(&passive(0, None, w(Some(11.0), None)), calm)
                .changed
        );
        assert_eq!(b.coalesce_passive(&key(0), calm), Some(Coalesce::Deferred));
        // If the same call passes the 5-hour window's reset and stamps a latch, it repays at once.
        let latched = at_ms(200_600, T0 + 5 * H);
        assert!(
            b.apply_passive(&passive(0, None, w(Some(12.0), None)), latched)
                .changed
        );
        let Some(Coalesce::PublishNow(sheet)) = b.coalesce_passive(&key(0), latched) else {
            panic!("래치는 합치지 않는다");
        };
        assert!(expired(&sheet.five_hour));
        assert_eq!(pct(&sheet.weekly), Some(12.0));
        assert_eq!(sheet, snap(&b, 0, latched));
        assert!(tick(&mut b, &[0], at_ms(201_000, T0 + 5 * H))
            .publish
            .is_empty());
    }

    #[test]
    fn a_bump_the_service_did_not_broadcast_goes_out_on_the_next_tick() {
        let mut b = book();
        assert!(b.begin_probe(&key(0)));
        let plan = tick(&mut b, &[0], at(0, T0));
        assert!(plan.start.is_empty());
        assert_eq!(plan.publish, vec![(key(0), snap(&b, 0, at(0, T0)))]);
        assert!(plan.publish[0].1.in_flight, "조회 시작의 빚");

        b.finish_probe(&key(0), Ok(active(0, w(Some(5.0), None), None)), at(5, T0));
        let plan = tick(&mut b, &[0], at(5, T0));
        assert_eq!(plan.publish, vec![(key(0), snap(&b, 0, at(5, T0)))]);
        assert!(!plan.publish[0].1.in_flight, "조회 끝의 빚");

        let start = at(40, T0);
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, start),
            Judgment::Start
        );
        let plan = tick(&mut b, &[0], start);
        assert_eq!(
            plan.publish,
            vec![(key(0), snap(&b, 0, start))],
            "판정 시작의 빚"
        );

        // A repaid debt does not go out again.
        b.broadcast_sheet(&key(0), at(41, T0));
        assert!(tick(&mut b, &[0], at(41, T0)).publish.is_empty());
    }

    #[test]
    fn a_first_sheet_latch_is_still_owed_to_every_subscriber() {
        let mut b = settled();
        tick(&mut b, &[0], at(100, T0));
        // The first sheet of a subscription add = eval_time + snapshot — it goes only to that connection so it does not repay the debt.
        let now = at(200, T0 + 5 * H);
        assert!(b.eval_time(&key(0), now));
        let first_sheet = snap(&b, 0, now);
        let plan = tick(&mut b, &[0], now);
        assert_eq!(
            plan.publish,
            vec![(key(0), first_sheet)],
            "전부에게 나간다 — 새 연결엔 한 장이 겹친다"
        );
    }

    #[test]
    fn published_sheets_carry_their_keys_and_unsubscribed_debts_wait() {
        let mut b = book();
        assert!(b.begin_probe(&key(0)));
        assert!(b.begin_probe(&key(1)));
        let plan = tick(&mut b, &[0], at(0, T0));
        assert_eq!(plan.publish, vec![(key(0), snap(&b, 0, at(0, T0)))]);
        let plan = tick(&mut b, &[1, 0], at(1, T0));
        assert_eq!(
            plan.publish,
            vec![(key(1), snap(&b, 1, at(1, T0)))],
            "구독 밖에서 기다린 빚"
        );

        let mut b = book();
        assert!(b.begin_probe(&key(0)));
        assert!(b.begin_probe(&key(1)));
        let plan = tick(&mut b, &[1, 0], at(0, T0));
        assert_eq!(
            plan.publish,
            vec![
                (key(0), snap(&b, 0, at(0, T0))),
                (key(1), snap(&b, 1, at(0, T0)))
            ],
            "칸을 만든 순서"
        );
    }

    #[test]
    fn a_sub_second_coalesce_deadline_does_not_spin() {
        let mut b = settled();
        passive_pct(&mut b, 50.0, at_ms(200_000, T0));
        passive_pct(&mut b, 51.0, at_ms(200_999, T0));
        let plan = tick(&mut b, &[0], at_ms(200_999, T0));
        assert_eq!(plan.sleep, Duration::from_millis(1));
        let plan = tick(&mut b, &[0], at_ms(201_000, T0));
        assert_eq!(plan.publish.len(), 1);
        assert_eq!(plan.sleep, SCHEDULE_MAX_SLEEP);
    }

    #[test]
    fn a_scheduled_start_carries_and_clears_a_pending_change() {
        let mut b = book();
        passive_pct(&mut b, 50.0, at(0, T0));
        assert_eq!(
            passive_pct(&mut b, 51.0, at_ms(300, T0)),
            Some(Coalesce::Deferred)
        );
        let now = at_ms(500, T0);
        let plan = tick(&mut b, &[0], now);
        assert_eq!(plan.start, vec![key(0)], "기준점이 없어 지금 기한");
        assert_eq!(plan.publish, vec![(key(0), snap(&b, 0, now))]);
        assert!(plan.publish[0].1.in_flight);
        assert_eq!(pct(&plan.publish[0].1.five_hour), Some(51.0));
        assert_eq!(
            plan.sleep, SCHEDULE_MAX_SLEEP,
            "옛 합침 기한은 깰 때가 아니다"
        );
        assert!(tick(&mut b, &[0], at(1, T0)).publish.is_empty());
    }

    #[test]
    fn a_pending_change_on_a_probe_in_flight_goes_out_at_its_deadline() {
        let mut b = settled();
        let start = at(200, T0);
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, start),
            Judgment::Start
        );
        assert!(b.broadcast_sheet(&key(0), start).expect("한 장").in_flight);
        assert_eq!(
            passive_pct(&mut b, 55.0, at_ms(200_300, T0)),
            Some(Coalesce::Deferred)
        );
        assert_eq!(
            tick(&mut b, &[0], at_ms(200_300, T0)).sleep,
            Duration::from_millis(700),
            "진행 중이라 자동 기한은 빠진다"
        );
        let deadline = at(201, T0);
        let plan = tick(&mut b, &[0], deadline);
        assert!(plan.start.is_empty());
        assert_eq!(plan.publish, vec![(key(0), snap(&b, 0, deadline))]);
        assert!(plan.publish[0].1.in_flight);
        assert_eq!(pct(&plan.publish[0].1.five_hour), Some(55.0));
    }

    // ── Scheduler (`plan_tick`) ──

    #[test]
    fn a_due_subscribed_vendor_starts_one_probe_and_requests_join_it() {
        let mut b = book();
        let plan = tick(&mut b, &[0], at(0, T0));
        assert_eq!(plan.start, vec![key(0)]);
        assert_eq!(
            plan.publish,
            vec![(key(0), snap(&b, 0, at(0, T0)))],
            "조회 시작은 발행한다"
        );
        assert!(plan.publish[0].1.in_flight);
        assert_eq!(
            (b.in_flight(&key(0)), b.revision(&key(0))),
            (Some(true), Some(1))
        );
        assert_eq!(
            judge(&mut b, 0, RequestKind::Get, at(0, T0)),
            Judgment::Join
        );
        assert_eq!(
            judge(&mut b, 0, RequestKind::Refresh, at(0, T0)),
            Judgment::Join
        );

        // If a request started first, the scheduler does not start again.
        let mut b = book();
        assert_eq!(
            judge(&mut b, 0, RequestKind::Get, at(0, T0)),
            Judgment::Start
        );
        assert!(tick(&mut b, &[0], at(0, T0)).start.is_empty());
    }

    #[test]
    fn a_probe_in_flight_is_not_restarted_and_its_deadline_is_not_a_wake() {
        let near = Now {
            mono: secs(100) + cooldown(0) - secs(10),
            wall: T0,
        };
        let mut b = settled();
        assert_eq!(tick(&mut b, &[0], near).sleep, secs(10));

        let mut b = settled();
        assert!(b.begin_probe(&key(0)));
        assert_eq!(tick(&mut b, &[0], near).sleep, SCHEDULE_MAX_SLEEP);
        let past_due = Now {
            mono: near.mono + secs(20),
            wall: T0,
        };
        let plan = tick(&mut b, &[0], past_due);
        assert!(plan.start.is_empty());
        assert_eq!(plan.sleep, SCHEDULE_MAX_SLEEP);

        // Query end → wake at the next deadline.
        b.finish_probe(&key(0), Ok(active(0, None, None)), past_due);
        let almost = Now {
            mono: past_due.mono + cooldown(0) - secs(7),
            wall: T0,
        };
        assert_eq!(tick(&mut b, &[0], almost).sleep, secs(7));
    }

    #[test]
    fn a_reset_wakes_the_scheduler_and_its_latch_is_published() {
        let mut b = settled();
        let r = T0 + 30;
        succeed(
            &mut b,
            0,
            active(0, w(Some(40.0), Some(r)), None),
            at(100, T0),
        );
        let plan = tick(&mut b, &[0], at(100, T0));
        assert_eq!(plan.sleep, secs(30), "벽시계 차");
        let rev = b.revision(&key(0)).expect("키");

        let plan = tick(&mut b, &[0], at(131, r));
        assert_eq!(plan.publish, vec![(key(0), snap(&b, 0, at(131, r)))]);
        assert!(plan.start.is_empty());
        assert_eq!(plan.publish[0].1.revision, rev + 1);
        assert!(expired(&plan.publish[0].1.five_hour));
    }

    #[test]
    fn a_rejection_end_wakes_the_scheduler_and_its_fold_is_published() {
        let mut b = settled();
        fail(&mut b, 0, rate_limited(Some(secs(20))), at(100, T0));
        assert_eq!(tick(&mut b, &[0], at(100, T0)).sleep, secs(20));
        let plan = tick(&mut b, &[0], at(120, T0));
        assert_eq!(plan.publish, vec![(key(0), snap(&b, 0, at(120, T0)))]);
        assert!(plan.start.is_empty(), "거절 끝은 조회를 앞당기지 않는다");
        assert_eq!(tag(&plan.publish[0].1.state), "Failed");
    }

    #[test]
    fn a_rejection_longer_than_the_cooldown_folds_and_starts_in_one_tick() {
        let mut b = settled();
        let wait = cooldown(0) + secs(600);
        fail(&mut b, 0, rate_limited(Some(wait)), at(100, T0));
        let until = secs(100) + wait;
        assert_eq!(
            b.next_auto(&key(0), at(100, T0)),
            Some(until),
            "시험 전제: 자동 기한 = 거절 끝"
        );
        tick(&mut b, &[0], at(100, T0));

        let before_end = Now {
            mono: until - Duration::from_millis(1),
            wall: T0,
        };
        assert_eq!(
            tick(&mut b, &[0], before_end),
            TickPlan {
                sleep: Duration::from_millis(1),
                ..TickPlan::default()
            }
        );

        let end = Now {
            mono: until,
            wall: T0,
        };
        let rev = b.revision(&key(0)).expect("아는 키");
        let plan = tick(&mut b, &[0], end);
        assert_eq!(plan.start, vec![key(0)]);
        assert_eq!(plan.publish, vec![(key(0), snap(&b, 0, end))], "한 장");
        assert_eq!(b.revision(&key(0)), Some(rev + 2), "거절 끝 · 조회 시작");
        assert_eq!(tag(&plan.publish[0].1.state), "Failed");
        assert!(plan.publish[0].1.in_flight);
        assert_eq!(b.in_flight(&key(0)), Some(true));
    }

    #[test]
    fn no_subscribed_vendor_means_no_probe_and_the_longest_sleep() {
        assert_eq!(
            TickPlan::default().sleep,
            SCHEDULE_MAX_SLEEP,
            "기본값도 0 이 아니다"
        );
        let mut b = settled();
        passive_pct(&mut b, 50.0, at(200, T0));
        assert_eq!(
            passive_pct(&mut b, 51.0, at_ms(200_300, T0)),
            Some(Coalesce::Deferred)
        );
        let rev = (b.revision(&key(0)), b.revision(&key(1)));
        let plan = tick(&mut b, &[], at(201, T0 + 200 * H));
        assert_eq!(plan, TickPlan::default());
        assert_eq!(
            (b.revision(&key(0)), b.revision(&key(1))),
            rev,
            "래치도 안 본다"
        );
        assert_eq!(b.in_flight(&key(1)), Some(false), "기한이 지난 칸도 조회 0");

        // Even outside a subscription the wait remains — the subscription copy may be stale. After being subscribed, one sheet goes out.
        let later = at(202, T0);
        let plan = tick(&mut b, &[0], later);
        assert_eq!(plan.publish, vec![(key(0), snap(&b, 0, later))]);
        assert_eq!(pct(&plan.publish[0].1.five_hour), Some(51.0));
    }

    #[test]
    fn an_unsubscribed_due_vendor_is_not_probed() {
        let mut b = book();
        let plan = tick(&mut b, &[1, 1], at(0, T0));
        assert_eq!(plan.start, vec![key(1)], "중복은 한 번");
        assert_eq!(b.in_flight(&key(0)), Some(false));

        let odd = UsageVendorKey::new("no-such-vendor");
        let plan = b.plan_tick(&[odd], at(1, T0));
        assert_eq!(plan.start, Vec::<UsageKey>::new());
    }

    #[test]
    fn subscribing_late_starts_a_past_deadline_at_once() {
        let mut b = settled();
        let late = Now {
            mono: secs(100) + cooldown(0) + secs(500),
            wall: T0,
        };
        assert!(tick(&mut b, &[], late).start.is_empty());
        assert_eq!(tick(&mut b, &[0], late).start, vec![key(0)]);
    }

    #[test]
    fn a_three_hour_mono_jump_is_one_scheduled_probe_not_one_per_interval() {
        let mut b = settled();
        let jumped = at(100 + 3 * 3_600, T0);
        assert_eq!(tick(&mut b, &[0], jumped).start, vec![key(0)]);
        let done = at(100 + 3 * 3_600 + 5, T0);
        b.finish_probe(&key(0), Ok(active(0, None, None)), done);
        assert!(tick(&mut b, &[0], done).start.is_empty());
    }

    #[test]
    fn extreme_times_do_not_panic_and_sleep_stays_bounded() {
        const YEAR_9999: i64 = 253_402_300_799;
        let mut b = book();
        let pre_1970 = at(10, -1_000_000);
        succeed(
            &mut b,
            0,
            active(0, w(Some(1.0), Some(YEAR_9999)), w(None, Some(i64::MAX))),
            pre_1970,
        );
        let plan = tick(&mut b, &[0, 1], pre_1970);
        assert_eq!(plan.start, vec![key(1)]);
        tick(&mut b, &[0, 1], at(20, i64::MIN));
        tick(&mut b, &[0, 1], at(30, i64::MAX));

        for mono in [Duration::MAX - Duration::from_nanos(1), Duration::MAX] {
            let edge = Now { mono, wall: T0 };
            let mut b = book();
            succeed(&mut b, 0, active(0, w(Some(1.0), Some(T0 + H)), None), edge);
            b.coalesce_passive(&key(0), edge);
            b.coalesce_passive(&key(0), edge);
            fail(&mut b, 1, rate_limited(Some(REJECT_MAX)), edge);
            tick(&mut b, &[0, 1], edge);
            judge(&mut b, 0, RequestKind::Refresh, edge);
            judge(&mut b, 1, RequestKind::Get, edge);
        }
    }
}
