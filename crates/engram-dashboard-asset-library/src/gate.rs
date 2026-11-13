//! The library lock and the scan-generation gate (ADR-0324) — every catalog write goes through here.
//!
//! ## Rules (invariants)
//!
//! - **One `RwLock` over [`State`]** (the catalog and the root). A read takes it once ([`Gate::with_read`]). Walking,
//!   reading and parsing run outside it; the write lock is taken only to apply. The write lock is never taken while
//!   the read lock is held (std has no upgrade).
//! - **The turn `Mutex` + `Condvar`** holds the ticket line, the three numbers ([`Numbers`]) and the first-scan state.
//!   It is a leaf: nothing else is taken while it is held, and nobody waits on its `Condvar` while holding the library
//!   lock. It is taken briefly inside the library write lock, where every number is raised, so numbers read there agree
//!   with the catalog. Lock order: library lock → turn mutex, never the reverse.
//! - **Tickets, FIFO**: a scan or a root change enters when every earlier ticket has finished; a change enters when
//!   every earlier ticket has entered and is a change (changes drawn in a row enter together and then meet at the write
//!   lock). A root change therefore stays exclusive from its ticket to its publish. A ticket is drawn and first checked
//!   in one hold of the turn mutex, and released on every exit, a panic included ([`Turn`]).
//! - **Class A**: every catalog write is per item onto the current catalog — a scan's `Catalog::apply_scan`, a late
//!   merge's `Catalog::apply_items`, a change's own per-item primitives. The one exception is the root change: a full
//!   fresh build of the new root, applied after `Catalog::clear` in the write lock that publishes the root.
//! - **Numbers**: a scan's apply raises the swap-in count, a change the change generation, a root change the root
//!   generation — each inside its write lock. A scan applies only if the change generation has not moved since its
//!   walk started (else it walks again). A late merge applies only if neither generation moved since the caller noted
//!   them before its disk read; if only the swap-in count moved it may update items but not insert paths.
//! - **First scan**: "raised" = one full scan or one root change has been applied. A reader starts one or waits for one
//!   pending; a reader whose waited-for scan was not published goes through once more, and that failure is its answer.
//! - **Refresh merging**: a refresh joins a covering scan generation that is queued and not yet walking; a covering
//!   walk already in progress does not count — the caller may have changed files before calling (PRD R41).
// ADR-0324
// TRD S25 step-7 §6

use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, RwLock};

use engram_dashboard_agent::launch::Reason;
use engram_dashboard_base::sync::{lock, read, write};
use engram_dashboard_platform::fs::PathKey;
use serde_json::json;

use crate::catalog::{key, preset_refs, within, Catalog, Item, ItemContent, ScanItem, ScanResult};
use crate::path::LibPath;
use crate::store::ContentTag;
use crate::RefreshReport;

/// The code of a preset reference whose target is not in the catalog (`args` = `{path}` — the target as the preset
/// spells it).
// TRD S25 step-7 B8
pub const NOT_FOUND: &str = "not-found";

/// Why a scan or a root change did not run: no library root is set.
pub(crate) const NO_ROOT: &str = "no library root is set";

/// The answer of a scan or root change whose ticket was released without one (it panicked).
const ENDED_UNANSWERED: &str = "the scan ended without an answer";

/// What the library lock guards.
pub struct State {
    pub catalog: Arc<Catalog>,
    /// `None` = no library root is set: nothing can be scanned.
    pub root: Option<PathBuf>,
}

/// The three numbers a late merge notes before its disk read and compares inside its write lock (ADR-0324 decision
/// 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Numbers {
    /// Raised by every change, inside its write lock.
    pub change: u64,
    /// Raised by every root change, inside the write lock that publishes the new root.
    pub root: u64,
    /// Raised by every scan's apply.
    pub swap_ins: u64,
}

/// The tag and racy mark of every catalog item in a scan's range, read under a brief read lock.
pub(crate) type Tags = HashMap<PathKey, (ContentTag, bool)>;

/// Builds one scan outside every lock from the root, the range and the range's current [`Tags`].
pub(crate) type Build<'a> = &'a dyn Fn(&Path, Option<&LibPath>, &Tags) -> io::Result<ScanResult>;

/// A scan's or root change's answer, shared by everyone who joined it. `Err` = nothing was applied, with the reason.
pub(crate) type Outcome = Result<Arc<ScanReport>, String>;

/// What one applied scan did, over its whole range.
#[derive(Debug, Default)]
pub(crate) struct ScanReport {
    /// The paths of the items it inserted, replaced or removed.
    changed: Vec<LibPath>,
    /// The problems of the files it re-read: a broken file's reason, a preset's line problems and the references it
    /// lists that the catalog does not hold ([`NOT_FOUND`]), a settings asset's refusals.
    problems: Vec<(LibPath, Reason)>,
    no_kind: Vec<LibPath>,
    skipped_links: Vec<LibPath>,
    errors: Vec<(Option<LibPath>, String)>,
}

impl ScanReport {
    /// The part of this report inside `folder` (`None` = all of it). A walk error charged to the root itself belongs
    /// only to the whole report.
    pub(crate) fn share(&self, folder: Option<&LibPath>) -> RefreshReport {
        let inside = |path: &LibPath| within(path, folder);
        RefreshReport {
            changed: self.changed.iter().filter(|path| inside(path)).count(),
            problems: self
                .problems
                .iter()
                .filter(|(path, _)| inside(path))
                .cloned()
                .collect(),
            no_kind: self
                .no_kind
                .iter()
                .filter(|path| inside(path))
                .cloned()
                .collect(),
            skipped_links: self
                .skipped_links
                .iter()
                .filter(|path| inside(path))
                .cloned()
                .collect(),
            errors: self
                .errors
                .iter()
                .filter(|(at, _)| at.as_ref().map_or(folder.is_none(), inside))
                .cloned()
                .collect(),
        }
    }
}

/// The library lock, the turn mutex and the sequence every read draws from.
pub(crate) struct Gate {
    lib: RwLock<State>,
    turns: Mutex<Turns>,
    moved: Condvar,
    seq: AtomicU64,
}

struct Turns {
    next: u64,
    /// Every ticket drawn and not yet released, in drawing order.
    line: VecDeque<Ticket>,
    numbers: Numbers,
    first: First,
}

#[derive(Default)]
struct First {
    raised: bool,
    /// Full scans and root changes drawn before the first scan was raised that have not ended.
    pending: usize,
    /// How many of those have ended — a waiter wakes when it moves.
    ended: u64,
    /// Why the last of them that ended without an apply failed.
    failure: Option<String>,
    /// Callers waiting at the first-scan entrance or on a joined generation (tests observe it).
    #[cfg(test)]
    waiting: usize,
}

struct Ticket {
    no: u64,
    kind: Kind,
    entered: bool,
    /// Counted in [`First::pending`].
    pending_first: bool,
}

enum Kind {
    Scan(Arc<Generation>),
    Change,
    Root,
}

/// One scan generation: its range and, once its ticket is released, its answer.
struct Generation {
    range: Option<LibPath>,
    outcome: OnceLock<Outcome>,
}

/// A drawn ticket. Dropping it releases the ticket and wakes every waiter — on every exit, a panic included.
struct Turn<'g> {
    gate: &'g Gate,
    no: u64,
    /// A scan's or root change's answer, handed to its generation and to the first-scan state on release.
    outcome: Option<Outcome>,
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        let outcome = self
            .outcome
            .take()
            .unwrap_or_else(|| Err(ENDED_UNANSWERED.to_owned()));
        let mut turns = lock(&self.gate.turns);
        let at = turns
            .line
            .iter()
            .position(|ticket| ticket.no == self.no)
            .expect("a drawn ticket stays in the line until it is released");
        let ticket = turns.line.remove(at).expect("the position is in the line");
        if let Kind::Scan(generation) = &ticket.kind {
            generation
                .outcome
                .set(outcome.clone())
                .expect("a generation is answered once, when its ticket is released");
        }
        if ticket.pending_first {
            let first = &mut turns.first;
            first.pending -= 1;
            first.ended += 1;
            if let Err(why) = &outcome {
                first.failure = Some(why.clone());
            }
        }
        self.gate.moved.notify_all();
    }
}

impl Gate {
    pub(crate) fn new(root: Option<PathBuf>) -> Self {
        Gate {
            lib: RwLock::new(State {
                catalog: Arc::new(Catalog::default()),
                root,
            }),
            turns: Mutex::new(Turns {
                next: 0,
                line: VecDeque::new(),
                numbers: Numbers {
                    change: 0,
                    root: 0,
                    swap_ins: 0,
                },
                first: First::default(),
            }),
            moved: Condvar::new(),
            seq: AtomicU64::new(0),
        }
    }

    /// The next number of the one increasing sequence — drawn before every disk read of the library.
    pub(crate) fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub(crate) fn numbers(&self) -> Numbers {
        lock(&self.turns).numbers
    }

    pub(crate) fn with_read<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        f(&read(&self.lib))
    }

    /// The first-scan entrance (ADR-0324 decisions 6 · 7): returns at once once raised; else waits for a pending full
    /// scan or root change (queued or walking), or runs a full scan itself. A waited-for one that was not published
    /// sends the caller through once more; `Err` = that second try failed too, or the caller's own scan failed.
    pub(crate) fn first_scan_join(&self, build: Build) -> Result<(), String> {
        let mut turns = lock(&self.turns);
        let mut went_again = false;
        loop {
            if turns.first.raised {
                return Ok(());
            }
            if turns.first.pending > 0 {
                turns = self.wait_first(turns);
                if turns.first.raised {
                    return Ok(());
                }
                if went_again {
                    return Err(failure(&turns));
                }
                went_again = true;
                continue;
            }
            let turn = self.draw(&mut turns, Kind::Scan(Arc::new(Generation::new(None))));
            drop(self.enter(turns, turn.no));
            return self.run_scan(turn, None, build).map(drop);
        }
    }

    /// A ticketed scan of `range` (ADR-0324 decision 7 — refresh merging). Joins a covering generation that is queued
    /// and not walking; otherwise draws its own ticket. A folder refresh before the first scan is raised joins a
    /// queued full scan or runs one itself and takes its share (decision 6); one that finds a full scan (or a
    /// first-start root change) already walking waits for it and then scans its folder like any refresh. When the one
    /// it joined or waited for is not published it goes through a full scan once more.
    // ADR-0366
    pub(crate) fn refresh(&self, range: Option<&LibPath>, build: Build) -> Outcome {
        let mut turns = lock(&self.turns);
        let mut went_again = false;
        loop {
            let before_first = range.is_some() && !turns.first.raised;
            if let Some(generation) = queued_covering(&turns.line, range) {
                let (back, outcome) = self.wait_outcome(turns, &generation);
                turns = back;
                if outcome.is_err() && before_first && !went_again {
                    went_again = true;
                    continue;
                }
                return outcome;
            }
            if before_first && turns.first.pending > 0 {
                turns = self.wait_first(turns);
                if !turns.first.raised {
                    if went_again {
                        return Err(failure(&turns));
                    }
                    went_again = true;
                }
                continue;
            }
            // ADR-0324 decision 6 · M48: before the first scan only a full scan is published.
            let own = if before_first { None } else { range };
            let generation = Arc::new(Generation::new(own.cloned()));
            let turn = self.draw(&mut turns, Kind::Scan(generation));
            drop(self.enter(turns, turn.no));
            return self.run_scan(turn, own, build);
        }
    }

    /// A change (TRD S25 step-7 §6 writer 5): its own ticket, then `f` under the write lock with the catalog, the root
    /// and the sequence (a removal draws its number there, so it is later than every read noted before). `f` writes
    /// per item and returns the touched keys; their derived data is recomputed and the change generation is raised
    /// before the write lock is released. `f` may call the profile edit port and read the profile registry — nothing
    /// else outward (ADR-0324 decision 1). The change's disk effect must be finished inside `f` (or before the call).
    pub(crate) fn change<R>(
        &self,
        f: impl FnOnce(&mut Catalog, Option<&Path>, &dyn Fn() -> u64) -> (R, Vec<PathKey>),
    ) -> R {
        let _turn = self.take_turn(Kind::Change);
        let mut state = write(&self.lib);
        let State { catalog, root } = &mut *state;
        let catalog = Arc::make_mut(catalog);
        let (answer, touched) = f(catalog, root.as_deref(), &|| self.next_seq());
        catalog.recompute_derived(&touched);
        lock(&self.turns).numbers.change += 1;
        answer
    }

    /// The root change (TRD S25 step-7 §6 writer 6, steps 1–4): exclusive from its ticket to its publish; `walk` builds
    /// the new root outside every lock with empty tags; one write lock clears the catalog, applies that build,
    /// publishes the root, raises the root generation and marks the first scan raised. A walk that fails at the root
    /// publishes nothing. (Step 0, the stat of the new root, is the caller's — before this call.)
    // ADR-0366
    pub(crate) fn change_root(
        &self,
        new_root: PathBuf,
        walk: impl FnOnce(&Path) -> io::Result<ScanResult>,
    ) -> Outcome {
        let mut turn = self.take_turn(Kind::Root);
        let outcome = self.publish_root(new_root, walk);
        turn.outcome = Some(outcome.clone());
        outcome
    }

    /// The conditional late merge (ADR-0324 decision 5): under the write lock, nothing when the change or root
    /// generation moved since `noted`; else `f(catalog, may_insert)` with `may_insert` false when a swap-in landed in
    /// between. `f` applies per item (the later `read_seq` wins — `Catalog::apply_items`) and returns the touched
    /// keys, whose derived data is recomputed here. Takes no ticket and never waits for a scan.
    // ADR-0366
    pub(crate) fn late_merge(
        &self,
        noted: Numbers,
        f: impl FnOnce(&mut Catalog, bool) -> Vec<PathKey>,
    ) -> Vec<PathKey> {
        let mut state = write(&self.lib);
        let now = self.numbers();
        if now.change != noted.change || now.root != noted.root {
            drop(state);
            tracing::debug!(
                ?noted,
                ?now,
                "라이브러리 늦은 합치기: 읽은 뒤 세대가 움직여 버렸다"
            );
            return Vec::new();
        }
        let catalog = Arc::make_mut(&mut state.catalog);
        let touched = f(catalog, now.swap_ins == noted.swap_ins);
        catalog.recompute_derived(&touched);
        touched
    }

    fn wait<'a>(&self, turns: MutexGuard<'a, Turns>) -> MutexGuard<'a, Turns> {
        self.moved
            .wait(turns)
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn draw<'a>(&'a self, turns: &mut Turns, kind: Kind) -> Turn<'a> {
        let no = turns.next;
        turns.next += 1;
        let pending_first = !turns.first.raised
            && match &kind {
                Kind::Scan(generation) => generation.range.is_none(),
                Kind::Root => true,
                Kind::Change => false,
            };
        if pending_first {
            turns.first.pending += 1;
        }
        turns.line.push_back(Ticket {
            no,
            kind,
            entered: false,
            pending_first,
        });
        self.moved.notify_all();
        Turn {
            gate: self,
            no,
            outcome: None,
        }
    }

    /// Waits until ticket `no` may enter (module rules), marks it entered and wakes the others — a change behind it
    /// may now enter too.
    fn enter<'a>(&self, mut turns: MutexGuard<'a, Turns>, no: u64) -> MutexGuard<'a, Turns> {
        while !may_enter(&turns.line, no) {
            turns = self.wait(turns);
        }
        turns
            .line
            .iter_mut()
            .find(|ticket| ticket.no == no)
            .expect("a waiting ticket is in the line")
            .entered = true;
        self.moved.notify_all();
        turns
    }

    fn take_turn(&self, kind: Kind) -> Turn<'_> {
        let mut turns = lock(&self.turns);
        let turn = self.draw(&mut turns, kind);
        drop(self.enter(turns, turn.no));
        turn
    }

    /// Waits until a pending first scan ends or the first scan is raised.
    fn wait_first<'a>(&self, mut turns: MutexGuard<'a, Turns>) -> MutexGuard<'a, Turns> {
        let ended = turns.first.ended;
        #[cfg(test)]
        {
            turns.first.waiting += 1;
            self.moved.notify_all();
        }
        while !turns.first.raised && turns.first.ended == ended {
            turns = self.wait(turns);
        }
        #[cfg(test)]
        {
            turns.first.waiting -= 1;
        }
        turns
    }

    fn wait_outcome<'a>(
        &self,
        mut turns: MutexGuard<'a, Turns>,
        generation: &Generation,
    ) -> (MutexGuard<'a, Turns>, Outcome) {
        #[cfg(test)]
        {
            turns.first.waiting += 1;
            self.moved.notify_all();
        }
        let outcome = loop {
            if let Some(outcome) = generation.outcome.get() {
                break outcome.clone();
            }
            turns = self.wait(turns);
        };
        #[cfg(test)]
        {
            turns.first.waiting -= 1;
        }
        (turns, outcome)
    }

    fn run_scan(&self, mut turn: Turn<'_>, range: Option<&LibPath>, build: Build) -> Outcome {
        let outcome = self.scan_until_applied(range, build);
        turn.outcome = Some(outcome.clone());
        outcome
    }

    fn scan_until_applied(&self, range: Option<&LibPath>, build: Build) -> Outcome {
        loop {
            let started = self.numbers().change;
            let (root, tags) =
                self.with_read(|state| (state.root.clone(), state.catalog.tags(range)));
            let root = root.ok_or_else(|| NO_ROOT.to_owned())?;
            let result = build(&root, range, &tags).map_err(|e| e.to_string())?;
            let mut state = write(&self.lib);
            // ADR-0324 decision 4 (M35): a change applied since the walk started could be undone by this result.
            if self.numbers().change != started {
                drop(state);
                tracing::debug!(
                    range = range.map(LibPath::as_str),
                    "라이브러리 훑기: 훑는 사이 변경 세대가 움직여 다시 훑는다"
                );
                continue;
            }
            let report = Arc::new(apply_reported(Arc::make_mut(&mut state.catalog), result));
            let mut turns = lock(&self.turns);
            turns.numbers.swap_ins += 1;
            if range.is_none() {
                turns.first.raised = true;
            }
            self.moved.notify_all();
            return Ok(report);
        }
    }

    fn publish_root(
        &self,
        new_root: PathBuf,
        walk: impl FnOnce(&Path) -> io::Result<ScanResult>,
    ) -> Outcome {
        let result = walk(&new_root).map_err(|e| e.to_string())?;
        let mut state = write(&self.lib);
        let catalog = Arc::make_mut(&mut state.catalog);
        catalog.clear();
        let report = Arc::new(apply_reported(catalog, result));
        state.root = Some(new_root);
        let mut turns = lock(&self.turns);
        turns.numbers.root += 1;
        turns.first.raised = true;
        self.moved.notify_all();
        Ok(report)
    }
}

impl Generation {
    fn new(range: Option<LibPath>) -> Self {
        Generation {
            range,
            outcome: OnceLock::new(),
        }
    }
}

fn may_enter(line: &VecDeque<Ticket>, no: u64) -> bool {
    let at = line
        .iter()
        .position(|ticket| ticket.no == no)
        .expect("a waiting ticket is in the line");
    match line[at].kind {
        Kind::Change => line
            .iter()
            .take(at)
            .all(|ticket| ticket.entered && matches!(ticket.kind, Kind::Change)),
        Kind::Scan(_) | Kind::Root => at == 0,
    }
}

fn queued_covering(line: &VecDeque<Ticket>, range: Option<&LibPath>) -> Option<Arc<Generation>> {
    line.iter().find_map(|ticket| match &ticket.kind {
        Kind::Scan(generation) if !ticket.entered && covers(generation.range.as_ref(), range) => {
            Some(Arc::clone(generation))
        }
        _ => None,
    })
}

fn covers(scan: Option<&LibPath>, want: Option<&LibPath>) -> bool {
    match (scan, want) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(scan), Some(want)) => within(want, Some(scan)),
    }
}

fn failure(turns: &Turns) -> String {
    turns
        .first
        .failure
        .clone()
        .unwrap_or_else(|| ENDED_UNANSWERED.to_owned())
}

/// `Catalog::apply_scan` plus the derived data, reporting what it did.
fn apply_reported(catalog: &mut Catalog, result: ScanResult) -> ScanReport {
    // The paths of the range's items before the apply, so a removed key can still be named.
    let before: HashMap<PathKey, LibPath> = catalog
        .tags(result.range.as_ref())
        .into_keys()
        .filter_map(|k| catalog.get(&k).map(|item| (k, item.path.clone())))
        .collect();
    let mut problems = Vec::new();
    let mut presets_read: Vec<(LibPath, Vec<LibPath>)> = Vec::new();
    for entry in &result.entries {
        let ScanItem::Fresh(item) = entry else {
            continue;
        };
        problems.extend(
            item_problems(item)
                .into_iter()
                .map(|reason| (item.path.clone(), reason)),
        );
        if let ItemContent::Preset { doc } = &item.content {
            presets_read.push((item.path.clone(), preset_refs(doc).cloned().collect()));
        }
    }
    let no_kind = result.no_kind.clone();
    let skipped_links = result.skipped_links.clone();
    let errors = result.errors.clone();
    let touched = catalog.apply_scan(result);
    catalog.recompute_derived(&touched);
    let changed = touched
        .iter()
        .filter_map(|k| {
            catalog
                .get(k)
                .map(|item| item.path.clone())
                .or_else(|| before.get(k).cloned())
        })
        .collect();
    for (preset, targets) in presets_read {
        for target in targets {
            if catalog.get(&key(&target)).is_none() {
                let reason = Reason {
                    code: NOT_FOUND.to_owned(),
                    args: json!({ "path": target.as_str() }),
                    line: None,
                };
                problems.push((preset.clone(), reason));
            }
        }
    }
    ScanReport {
        changed,
        problems,
        no_kind,
        skipped_links,
        errors,
    }
}

fn item_problems(item: &Item) -> Vec<Reason> {
    match &item.content {
        ItemContent::Broken { reason } => vec![reason.clone()],
        ItemContent::Preset { doc } => doc.line_problems.clone(),
        ItemContent::Settings { verdict, .. } => verdict
            .problems
            .iter()
            .map(|(_, reason)| reason.clone())
            .collect(),
        ItemContent::Instruction { .. } | ItemContent::Skill { .. } | ItemContent::SkillFile => {
            Vec::new()
        }
    }
}

/// Test views of the gate.
#[cfg(test)]
impl Gate {
    /// Waits (at most 10 s, then fails) until `ready` holds over the ticket line — `(kind, entered)` per ticket in
    /// drawing order, kinds "scan" · "change" · "root" — and the number of callers waiting at the first-scan entrance
    /// or on a joined generation.
    /// Every change of either wakes it.
    pub(crate) fn wait_for(
        &self,
        what: &str,
        ready: impl Fn(&[(&'static str, bool)], usize) -> bool,
    ) {
        use std::time::{Duration, Instant};
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut turns = lock(&self.turns);
        loop {
            let line: Vec<(&'static str, bool)> = turns
                .line
                .iter()
                .map(|ticket| {
                    let kind = match ticket.kind {
                        Kind::Scan(_) => "scan",
                        Kind::Change => "change",
                        Kind::Root => "root",
                    };
                    (kind, ticket.entered)
                })
                .collect();
            if ready(&line, turns.first.waiting) {
                return;
            }
            let now = Instant::now();
            assert!(
                now < deadline,
                "the gate never reached: {what} (line {line:?})"
            );
            turns = engram_dashboard_base::sync::wait_timeout(&self.moved, turns, deadline - now).0;
        }
    }

    /// The library lock itself — a test holds it or checks that nobody does.
    pub(crate) fn lib(&self) -> &RwLock<State> {
        &self.lib
    }

    /// Raises the change generation inside the write lock without a ticket, after `f` edits the catalog — a change the
    /// ticket line did not order, so a test can move the generation while a walk runs.
    pub(crate) fn change_unordered(&self, f: impl FnOnce(&mut Catalog)) {
        let mut state = write(&self.lib);
        let catalog = Arc::make_mut(&mut state.catalog);
        f(catalog);
        lock(&self.turns).numbers.change += 1;
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime};

    use super::*;

    /// A one-shot door a test opens; `wait` blocks until it is open (at most 10 s — a stuck door fails loudly).
    #[derive(Default)]
    struct Door {
        open: Mutex<bool>,
        moved: Condvar,
    }

    impl Door {
        fn open(&self) {
            *lock(&self.open) = true;
            self.moved.notify_all();
        }

        fn wait(&self) {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut open = lock(&self.open);
            while !*open {
                let now = Instant::now();
                assert!(now < deadline, "the door was never opened");
                open =
                    engram_dashboard_base::sync::wait_timeout(&self.moved, open, deadline - now).0;
            }
        }
    }

    fn lp(text: &str) -> LibPath {
        LibPath::parse(text).unwrap()
    }

    fn empty(range: Option<&LibPath>) -> ScanResult {
        ScanResult {
            range: range.cloned(),
            entries: Vec::new(),
            folders: Vec::new(),
            listed_at: HashMap::new(),
            absent_ranges: Vec::new(),
            kept: Vec::new(),
            skipped_links: Vec::new(),
            errors: Vec::new(),
            no_kind: Vec::new(),
            scanned_at: SystemTime::UNIX_EPOCH,
        }
    }

    fn gate() -> Arc<Gate> {
        Arc::new(Gate::new(Some(PathBuf::from("/lib"))))
    }

    /// A gate whose first scan is already raised, so a folder refresh scans only its folder.
    fn raised_gate() -> Arc<Gate> {
        let gate = gate();
        gate.refresh(None, &|_, range, _| Ok(empty(range))).unwrap();
        gate
    }

    /// Events in the order they happened.
    #[derive(Default)]
    struct Log(Mutex<Vec<String>>);

    impl Log {
        fn push(&self, event: impl Into<String>) {
            lock(&self.0).push(event.into());
        }

        fn take(&self) -> Vec<String> {
            lock(&self.0).clone()
        }
    }

    /// Joins `handle`, failing after 20 s instead of hanging.
    fn join<T>(handle: thread::JoinHandle<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !handle.is_finished() {
            assert!(Instant::now() < deadline, "a test thread hung");
            thread::sleep(Duration::from_millis(2));
        }
        handle.join().expect("the test thread panicked")
    }

    /// A scan on its own thread whose build logs `name`, waits at `door` (if any) and returns an empty result.
    fn scan_thread(
        gate: &Arc<Gate>,
        log: &Arc<Log>,
        name: &'static str,
        range: Option<&str>,
        door: Option<Arc<Door>>,
    ) -> thread::JoinHandle<Outcome> {
        let (gate, log) = (Arc::clone(gate), Arc::clone(log));
        let range = range.map(lp);
        thread::spawn(move || {
            gate.refresh(range.as_ref(), &|_, range, _| {
                log.push(format!("{name} walks"));
                if let Some(door) = &door {
                    door.wait();
                }
                Ok(empty(range))
            })
        })
    }

    fn change_thread(
        gate: &Arc<Gate>,
        log: &Arc<Log>,
        name: &'static str,
        door: Option<Arc<Door>>,
    ) -> thread::JoinHandle<Option<PathBuf>> {
        let (gate, log) = (Arc::clone(gate), Arc::clone(log));
        thread::spawn(move || {
            gate.change(|_, root, _| {
                log.push(format!("{name} enters"));
                if let Some(door) = &door {
                    door.wait();
                }
                (root.map(Path::to_path_buf), Vec::new())
            })
        })
    }

    fn kinds(line: &[(&'static str, bool)]) -> Vec<&'static str> {
        line.iter().map(|(kind, _)| *kind).collect()
    }

    #[test]
    fn a_change_waits_for_a_walking_scan_and_a_later_scan_does_not_overtake_it() {
        let (gate, log) = (gate(), Arc::new(Log::default()));
        let door = Arc::new(Door::default());
        let first = scan_thread(&gate, &log, "s1", None, Some(Arc::clone(&door)));
        gate.wait_for("s1 walking", |line, _| line == [("scan", true)]);
        let change = change_thread(&gate, &log, "c", None);
        gate.wait_for("c drawn", |line, _| kinds(line) == ["scan", "change"]);
        let second = scan_thread(&gate, &log, "s2", None, None);
        gate.wait_for("s2 drawn", |line, _| {
            kinds(line) == ["scan", "change", "scan"]
        });
        // Drawn and checked in one hold of the turn mutex: a ticket seen not entered is waiting.
        gate.wait_for("c and s2 wait", |line, _| {
            line == [("scan", true), ("change", false), ("scan", false)]
        });
        door.open();
        join(first).unwrap();
        join(change);
        join(second).unwrap();
        assert_eq!(log.take(), ["s1 walks", "c enters", "s2 walks"]);
    }

    #[test]
    fn a_change_drawn_behind_a_waiting_scan_does_not_overtake_it() {
        let (gate, log) = (gate(), Arc::new(Log::default()));
        let door = Arc::new(Door::default());
        let first = change_thread(&gate, &log, "c1", Some(Arc::clone(&door)));
        gate.wait_for("c1 in", |line, _| line == [("change", true)]);
        let scan = scan_thread(&gate, &log, "s", None, None);
        gate.wait_for("s drawn", |line, _| kinds(line) == ["change", "scan"]);
        let second = change_thread(&gate, &log, "c2", None);
        gate.wait_for("c2 waits behind s", |line, _| {
            line == [("change", true), ("scan", false), ("change", false)]
        });
        door.open();
        join(first);
        join(scan).unwrap();
        join(second);
        assert_eq!(log.take(), ["c1 enters", "s walks", "c2 enters"]);
    }

    #[test]
    fn changes_drawn_in_a_row_enter_together() {
        let (gate, log) = (gate(), Arc::new(Log::default()));
        let door = Arc::new(Door::default());
        let first = change_thread(&gate, &log, "c1", Some(Arc::clone(&door)));
        gate.wait_for("c1 in", |line, _| line == [("change", true)]);
        let second = change_thread(&gate, &log, "c2", None);
        // c2 has entered (counted in progress); it meets c1 only at the write lock c1 holds.
        gate.wait_for("both entered", |line, _| {
            line == [("change", true), ("change", true)]
        });
        door.open();
        join(first);
        join(second);
        assert_eq!(log.take(), ["c1 enters", "c2 enters"]);
    }

    #[test]
    fn a_change_drawn_during_a_root_change_enters_after_the_publish_and_sees_the_new_root() {
        let (gate, log) = (gate(), Arc::new(Log::default()));
        let door = Arc::new(Door::default());
        let root_change = {
            let (gate, log, door) = (Arc::clone(&gate), Arc::clone(&log), Arc::clone(&door));
            thread::spawn(move || {
                gate.change_root(PathBuf::from("/new"), |_| {
                    door.wait();
                    log.push("root walked");
                    Ok(empty(None))
                })
            })
        };
        gate.wait_for("root walking", |line, _| line == [("root", true)]);
        let change = change_thread(&gate, &log, "c", None);
        gate.wait_for("c waits", |line, _| {
            line == [("root", true), ("change", false)]
        });
        assert_eq!(
            gate.with_read(|state| state.root.clone()),
            Some(PathBuf::from("/lib")),
            "readers keep the old root during the walk"
        );
        door.open();
        join(root_change).unwrap();
        assert_eq!(join(change), Some(PathBuf::from("/new")));
        assert_eq!(log.take(), ["root walked", "c enters"]);
        assert_eq!(gate.numbers().root, 1);
    }

    #[test]
    fn a_root_walk_that_fails_publishes_nothing_and_releases_its_ticket() {
        let (gate, log) = (gate(), Arc::new(Log::default()));
        let outcome = gate.change_root(PathBuf::from("/new"), |_| {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        });
        assert!(outcome.is_err());
        assert_eq!(
            gate.with_read(|state| state.root.clone()),
            Some(PathBuf::from("/lib"))
        );
        assert_eq!(gate.numbers().root, 0);
        join(change_thread(&gate, &log, "c", None));
        assert_eq!(log.take(), ["c enters"]);
    }

    #[test]
    fn a_ticket_is_released_when_its_holder_panics() {
        let (gate, log) = (raised_gate(), Arc::new(Log::default()));
        let change = catch_unwind(AssertUnwindSafe(|| {
            gate.change(|_, _, _| -> ((), Vec<PathKey>) { panic!("a change panics on purpose") })
        }));
        assert!(change.is_err());
        // From here every gate call runs on a thread under the join timeout: a leaked ticket fails, not hangs.
        let root = {
            let gate = Arc::clone(&gate);
            thread::spawn(move || {
                catch_unwind(AssertUnwindSafe(|| {
                    gate.change_root(PathBuf::from("/new"), |_| {
                        panic!("a root walk panics on purpose")
                    })
                }))
                .is_err()
            })
        };
        assert!(join(root));
        // A scan that panics answers whoever joined it instead of leaving them waiting.
        let door = Arc::new(Door::default());
        let blocker = change_thread(&gate, &log, "blocker", Some(Arc::clone(&door)));
        gate.wait_for("blocker in", |line, _| line == [("change", true)]);
        let panicking = {
            let gate = Arc::clone(&gate);
            thread::spawn(move || {
                catch_unwind(AssertUnwindSafe(|| {
                    gate.refresh(None, &|_, _, _| panic!("a scan panics on purpose"))
                }))
                .is_err()
            })
        };
        gate.wait_for("scan queued", |line, _| kinds(line) == ["change", "scan"]);
        let joiner = scan_thread(&gate, &log, "joiner", Some("a"), None);
        gate.wait_for("joiner waits on the queued scan", |_, waiting| waiting == 1);
        door.open();
        join(blocker);
        assert!(join(panicking));
        assert_eq!(join(joiner).unwrap_err(), ENDED_UNANSWERED);
        join(change_thread(&gate, &log, "after", None));
        assert_eq!(log.take(), ["blocker enters", "after enters"]);
    }

    #[test]
    fn refreshes_join_only_a_covering_generation_that_has_not_started_walking() {
        let (gate, log) = (raised_gate(), Arc::new(Log::default()));
        let door = Arc::new(Door::default());
        let blocker = change_thread(&gate, &log, "blocker", Some(Arc::clone(&door)));
        gate.wait_for("blocker in", |line, _| line == [("change", true)]);
        let a = scan_thread(&gate, &log, "a", Some("a"), None);
        gate.wait_for("a queued", |line, _| line.len() == 2);
        // Covered by the queued `a`: joins it.
        let inner = scan_thread(&gate, &log, "a/x", Some("a/x"), None);
        let same = scan_thread(&gate, &log, "a again", Some("a"), None);
        gate.wait_for("both joined a", |_, waiting| waiting == 2);
        // Not covered by `a`: their own generations.
        let outside = scan_thread(&gate, &log, "b", Some("b"), None);
        gate.wait_for("b queued", |line, _| line.len() == 3);
        let whole = scan_thread(&gate, &log, "all", None, None);
        gate.wait_for("all queued", |line, _| line.len() == 4);
        // Covered by the queued whole-library generation.
        let under_whole = scan_thread(&gate, &log, "c", Some("c"), None);
        gate.wait_for("c joined all", |line, waiting| {
            line.len() == 4 && waiting == 3
        });
        door.open();
        join(blocker);
        let a = join(a).unwrap();
        assert!(Arc::ptr_eq(&a, &join(inner).unwrap()));
        assert!(Arc::ptr_eq(&a, &join(same).unwrap()));
        join(outside).unwrap();
        let whole = join(whole).unwrap();
        assert!(Arc::ptr_eq(&whole, &join(under_whole).unwrap()));
        assert_eq!(
            log.take(),
            ["blocker enters", "a walks", "b walks", "all walks"]
        );
    }

    #[test]
    fn a_refresh_arriving_at_a_walking_covering_scan_draws_its_own_ticket() {
        let (gate, log) = (raised_gate(), Arc::new(Log::default()));
        let door = Arc::new(Door::default());
        let walking = scan_thread(&gate, &log, "first", None, Some(Arc::clone(&door)));
        gate.wait_for("walking", |line, _| line == [("scan", true)]);
        let late = scan_thread(&gate, &log, "late", Some("a"), None);
        gate.wait_for("late queued", |line, _| {
            line == [("scan", true), ("scan", false)]
        });
        door.open();
        let first = join(walking).unwrap();
        let late = join(late).unwrap();
        assert!(!Arc::ptr_eq(&first, &late));
        assert_eq!(log.take(), ["first walks", "late walks"]);
    }

    #[test]
    fn readers_join_a_walking_first_scan_and_wake_when_it_is_raised() {
        let (gate, log) = (gate(), Arc::new(Log::default()));
        let door = Arc::new(Door::default());
        let reader = |door: Option<Arc<Door>>| {
            let (gate, log) = (Arc::clone(&gate), Arc::clone(&log));
            thread::spawn(move || {
                gate.first_scan_join(&|_, range, _| {
                    log.push("first walks");
                    if let Some(door) = &door {
                        door.wait();
                    }
                    Ok(empty(range))
                })
            })
        };
        let starter = reader(Some(Arc::clone(&door)));
        gate.wait_for("first walking", |line, _| line == [("scan", true)]);
        let joiner = reader(None);
        gate.wait_for("joiner waits", |_, waiting| waiting == 1);
        door.open();
        join(starter).unwrap();
        join(joiner).unwrap();
        assert_eq!(log.take(), ["first walks"]);
        // Raised: no more scans.
        reader(None).join().unwrap().unwrap();
        assert_eq!(log.take(), ["first walks"]);
    }

    #[test]
    fn a_reader_whose_waited_first_scan_failed_goes_through_once_more() {
        for second_fails in [false, true] {
            let gate = gate();
            let walks = Arc::new(Log::default());
            let door = Arc::new(Door::default());
            let starter = {
                let (gate, walks, door) =
                    (Arc::clone(&gate), Arc::clone(&walks), Arc::clone(&door));
                thread::spawn(move || {
                    gate.first_scan_join(&|_, _, _| {
                        walks.push("failing");
                        door.wait();
                        Err(io::Error::from(io::ErrorKind::PermissionDenied))
                    })
                })
            };
            gate.wait_for("first walking", |line, _| line == [("scan", true)]);
            let joiner = {
                let (gate, walks) = (Arc::clone(&gate), Arc::clone(&walks));
                thread::spawn(move || {
                    gate.first_scan_join(&|_, range, _| {
                        walks.push("again");
                        if second_fails {
                            Err(io::Error::other("still failing"))
                        } else {
                            Ok(empty(range))
                        }
                    })
                })
            };
            gate.wait_for("joiner waits", |_, waiting| waiting == 1);
            door.open();
            assert!(
                join(starter).is_err(),
                "the starter's own failure is its answer"
            );
            let answer = join(joiner);
            assert_eq!(walks.take(), ["failing", "again"]);
            assert_eq!(answer.is_err(), second_fails);
        }
    }

    #[test]
    fn a_folder_refresh_whose_joined_queued_first_scan_failed_goes_through_a_whole_one() {
        let (gate, log) = (gate(), Arc::new(Log::default()));
        let door = Arc::new(Door::default());
        let blocker = change_thread(&gate, &log, "blocker", Some(Arc::clone(&door)));
        gate.wait_for("blocker in", |line, _| line == [("change", true)]);
        let failing = {
            let (gate, log) = (Arc::clone(&gate), Arc::clone(&log));
            thread::spawn(move || {
                gate.refresh(None, &|_, _, _| {
                    log.push("failing walks");
                    Err(io::Error::from(io::ErrorKind::PermissionDenied))
                })
            })
        };
        gate.wait_for("the whole scan queued", |line, _| {
            line == [("change", true), ("scan", false)]
        });
        let folder = {
            let (gate, log) = (Arc::clone(&gate), Arc::clone(&log));
            thread::spawn(move || {
                gate.refresh(Some(&lp("a")), &|_, range, _| {
                    log.push(format!("again walks {:?}", range.map(LibPath::as_str)));
                    Ok(empty(range))
                })
            })
        };
        gate.wait_for("the folder refresh joined it", |_, waiting| waiting == 1);
        door.open();
        join(blocker);
        assert!(join(failing).is_err());
        assert!(join(folder).is_ok(), "went through once more");
        assert_eq!(
            log.take(),
            ["blocker enters", "failing walks", "again walks None"]
        );
    }

    #[test]
    fn a_root_change_enters_only_when_every_earlier_ticket_has_finished() {
        let (gate, log) = (raised_gate(), Arc::new(Log::default()));
        let root_thread = |door: Option<Arc<Door>>, to: &'static str| {
            let (gate, log) = (Arc::clone(&gate), Arc::clone(&log));
            thread::spawn(move || {
                gate.change_root(PathBuf::from(to), |_| {
                    log.push(format!("{to} walks"));
                    if let Some(door) = &door {
                        door.wait();
                    }
                    Ok(empty(None))
                })
            })
        };
        let change_door = Arc::new(Door::default());
        let change = change_thread(&gate, &log, "c", Some(Arc::clone(&change_door)));
        gate.wait_for("c in", |line, _| line == [("change", true)]);
        let root_door = Arc::new(Door::default());
        let first = root_thread(Some(Arc::clone(&root_door)), "/r1");
        gate.wait_for("r1 drawn", |line, _| line.len() == 2);
        // Drawn and checked in one hold: a root seen not entered behind an entered change is waiting.
        gate.wait_for("r1 waits behind c", |line, _| {
            line == [("change", true), ("root", false)]
        });
        change_door.open();
        join(change);
        gate.wait_for("r1 walking", |line, _| line == [("root", true)]);
        let second = root_thread(None, "/r2");
        gate.wait_for("r2 drawn", |line, _| line.len() == 2);
        gate.wait_for("r2 waits behind r1", |line, _| {
            line == [("root", true), ("root", false)]
        });
        root_door.open();
        join(first).unwrap();
        join(second).unwrap();
        assert_eq!(log.take(), ["c enters", "/r1 walks", "/r2 walks"]);
        assert_eq!(
            gate.with_read(|state| state.root.clone()),
            Some(PathBuf::from("/r2"))
        );
    }

    #[test]
    fn a_scan_whose_walk_saw_the_change_generation_move_walks_again() {
        let gate = gate();
        let walks = Arc::new(Log::default());
        let door = Arc::new(Door::default());
        let scan = {
            let (gate, walks, door) = (Arc::clone(&gate), Arc::clone(&walks), Arc::clone(&door));
            thread::spawn(move || {
                gate.refresh(None, &|_, range, _| {
                    walks.push("walk");
                    if walks.take().len() == 1 {
                        door.wait();
                    }
                    Ok(empty(range))
                })
            })
        };
        gate.wait_for("walking", |line, _| line == [("scan", true)]);
        gate.change_unordered(|_| {});
        door.open();
        join(scan).unwrap();
        assert_eq!(walks.take(), ["walk", "walk"]);
        assert_eq!(
            gate.numbers().swap_ins,
            1,
            "only the second walk was applied"
        );
    }

    #[test]
    fn a_walk_holds_no_library_guard() {
        let gate = gate();
        let door = Arc::new(Door::default());
        let walking = Arc::new(Door::default());
        let scan = {
            let (gate, door, walking) =
                (Arc::clone(&gate), Arc::clone(&door), Arc::clone(&walking));
            thread::spawn(move || {
                gate.refresh(None, &|_, range, _| {
                    walking.open();
                    door.wait();
                    Ok(empty(range))
                })
            })
        };
        walking.wait();
        assert!(
            gate.lib().try_write().is_ok(),
            "the walk holds a library guard"
        );
        door.open();
        join(scan).unwrap();
    }

    #[test]
    fn a_late_merge_is_dropped_when_a_generation_moved_and_may_not_insert_after_a_swap_in() {
        let gate = gate();
        let calls = |gate: &Gate, noted: Numbers| {
            let seen = Mutex::new(None);
            gate.late_merge(noted, |_, may_insert| {
                *lock(&seen) = Some(may_insert);
                Vec::new()
            });
            seen.into_inner().unwrap()
        };
        let noted = gate.numbers();
        assert_eq!(calls(&gate, noted), Some(true), "nothing moved");
        gate.refresh(None, &|_, range, _| Ok(empty(range))).unwrap();
        assert_eq!(
            calls(&gate, noted),
            Some(false),
            "a swap-in landed: updates only"
        );
        let noted = gate.numbers();
        gate.change(|_, _, _| ((), Vec::new()));
        assert_eq!(calls(&gate, noted), None, "the change generation moved");
        let noted = gate.numbers();
        gate.change_root(PathBuf::from("/new"), |_| Ok(empty(None)))
            .unwrap();
        assert_eq!(calls(&gate, noted), None, "the root generation moved");
    }

    #[test]
    fn covering_is_by_range() {
        assert!(covers(None, None));
        assert!(covers(None, Some(&lp("a"))));
        assert!(covers(Some(&lp("a")), Some(&lp("a/b"))));
        assert!(covers(Some(&lp("a")), Some(&lp("a"))));
        assert!(!covers(Some(&lp("a")), Some(&lp("ab"))));
        assert!(!covers(Some(&lp("a")), None));
    }
}
