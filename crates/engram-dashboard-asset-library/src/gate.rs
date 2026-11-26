//! The library lock and the scan-generation gate (ADR-0324) — every catalog write goes through here.
//!
//! ## Rules (invariants)
//!
//! - **One `RwLock` over [`State`]** (the catalog and the mounts). A read takes it once ([`Gate::with_read`]). Walking,
//!   reading and parsing run outside it; the write lock is taken only to apply. The write lock is never taken while
//!   the read lock is held (std has no upgrade).
//! - **The turn `Mutex` + `Condvar`** holds the ticket line, the three numbers ([`Numbers`]), the first-scan state
//!   and the view of the mounts a scan prepares from ([`MountView`]). It is a leaf: nothing else is taken while it is held, and nobody waits on its `Condvar` while holding the library
//!   lock. It is taken briefly inside the library write lock, where every number is raised, so numbers read there agree
//!   with the catalog. Lock order: library lock → turn mutex, never the reverse.
//! - **Tickets, FIFO**: a scan or a mount change (a `Root` ticket) enters when every earlier ticket has finished; a
//!   change enters when every earlier ticket has entered and is a change (changes drawn in a row enter together and then
//!   meet at the write lock). A mount change therefore stays exclusive from its ticket to its publish. A ticket is drawn
//!   and first checked in one hold of the turn mutex, and released on every exit, a panic included ([`Turn`]).
//! - **Class A**: every catalog write is per item onto the current catalog — a scan's `Catalog::apply_scan`, a late
//!   merge's `Catalog::apply_items`, a change's own per-item primitives. The one exception is a mount change
//!   ([`Gate::change_mount`]): an added or relocated mount's fresh walk, applied after `Catalog::drop_range` of that
//!   mount in the write lock that publishes it, and a removed mount's range dropped; other mounts' items stay (ADR-0366
//!   as ADR-0400 decision 16 amends it). A mount rename ([`Gate::rename_mount`]) moves its mount's range to the new
//!   name with `Catalog::rekey_range`, after its per-item rewrites, in the one write lock it holds throughout.
//! - **Scans walk per mount**: one scan walks each mount its range touches (every mount for the whole library), one
//!   after the other under its one ticket, and applies them all in one write lock. A range in no registered mount
//!   walks nothing and answers why. With no mount registered a whole scan applies nothing and still raises the first
//!   scan (the virtual root reads, empty — ADR-0400 decision 4).
//! - **A bounded stat before the ticket**: a scan stats every mount it would walk (the caller's [`Stat`], a bounded
//!   batch) before it draws its ticket, holding neither lock, so a mount that does not answer never holds the line; a
//!   mount whose stat is not `Ok` is not walked. It prepares from the [`MountView`] under the turn mutex, never from
//!   the library lock: a change drawn after a waiting scan holds the write lock across its body, and a scan that needed
//!   that lock to prepare would be overtaken (ADR-0324 decision 4). No stat is taken under a ticket: a mount registered
//!   or moved after the stats were taken is not walked by that scan (nothing is recorded for it), and a whole scan that
//!   left one out does not raise the first scan — its next reader or refresh walks it after a fresh stat. So does a
//!   mount the stat left unstated (`None` — the service's load-time overlap check was still running when the stat's
//!   wait for it ran out): nothing is recorded for it and the whole scan does not raise the first scan on it, so its
//!   waiters go round again on a fresh stat; only an answer of the mount's own (`Some` — a timeout included) is its
//!   state.
//!   Each mount's last known state and first-scan answer ([`MountSeen`]) is kept in [`State`], stamped with the
//!   sequence number drawn before its stat, and written in the apply's write lock; an older observation never replaces
//!   a newer state (a scan whose stat is older than the recorded state goes by the recorded one). A whole scan's mount
//!   that cannot be walked becomes that mount's state and an error of the report — the scan still applies the others
//!   and raises the first scan. A range scan in an unusable mount answers `Err`. A range in a mount not yet walked
//!   widens to that mount's top (decision 6 per mount). A mount a reader finds unusable is stat'ed again first; one that
//!   still does not stat `Ok` takes that state under a brief write lock without a ticket ([`Gate::observe`]), so a read
//!   into a mount that is not there does not queue behind a slow walk. A stat taken before the ticket can be stale by
//!   the walk (accepted). So can a mount's state between two scans: a mount last seen `Ok` and walked that has died
//!   since still gets a write's disk IO under the write lock (accepted — as `read` and the one root before mounts).
//!   (ADR-0400 decision 17.)
//! - **One writer of the mounts**: every write of `mounts`, `seen` or `stamps` goes through [`Gate::write_mounts`],
//!   which refreshes the [`MountView`] in the same write-lock hold; debug builds check the view at the end of every
//!   change and late merge. A scan matches its stats to the mounts it walks by that view, so a writer that bypassed it
//!   would send a reader round its first scan forever. The mount commands go through it too ([`Gate::change_mount`]): a
//!   mount added, removed or relocated has its `seen` and `stamps` entries cleared (as
//!   [`Gate::publish_mount_unwalked`] does) in that same call — an added or relocated one then recorded walked; a
//!   renamed one ([`Gate::rename_mount`]) has them moved to its new name.
//! - **A rename is one hold**: under its `Root` ticket a mount rename takes the write lock once and, inside it,
//!   rewrites the preset files (a bounded stat of a mount's folder when one of its files reads as not found), raises
//!   the root generation and the change number, rewrites the agents through the profile-edit port, re-keys the
//!   catalog and renames the registration (ADR-0400 decisions 12 and 16). It cannot be a
//!   change: a change never enters behind a held `Root` ticket. So an edit queued behind it meets the new name and a
//!   raised root generation, and an `agent.new` whose registry insert compares the change number meets the raised
//!   one or is rewritten by the agents pass (decision 18).
//! - **A change judges its mount again under the write lock**: a write's entrance walks an unusable mount again and
//!   refuses before the ticket, but a scan queued ahead of the change may record the mount gone after that; a change
//!   gets the recorded states ([`MountsNow`]) and judges its target's mount there, before any disk IO — no IO and no
//!   walk inside the lock (ADR-0400 decision 6).
//! - **Numbers**: a scan's apply raises the swap-in count, a change the change generation, a mount change the root
//!   generation and the change generation — each inside its write lock (a rename before its agents pass; a rename
//!   stopped by its file walk raises only the change generation, and only when it applied an item). A change refused inside its body that applied
//!   no catalog item raises nothing (its refusal applied nothing, so a walk or a late merge has nothing to miss). A
//!   scan applies only if the change generation has not moved since its walk started (else it walks again). A late
//!   merge applies only if neither generation moved since the caller noted them before its disk read; if only the
//!   swap-in count moved it may update items but not insert paths. A change given a root generation compares it inside
//!   its own write-lock take; a stale one writes and raises nothing ([`Gate::change_checked`] — ADR-0377).
//! - **First scan**: "raised" = one full scan has been applied. A reader starts one or waits for one pending; a reader
//!   whose waited-for scan was not published goes through once more, and that failure is its answer; only a scan that
//!   ended without an apply is a failed try — one that applied but left the first scan un-raised (a mount was published
//!   after its stats were taken, or its stat left one unstated) sends its waiters round again. A mount change is never
//!   pending and leaves it as it is (it walks one mount, not the library). The first start's publish without a walk
//!   ([`Gate::publish_mount_unwalked`]) lowers it again (a reader before it may have raised it over no mount), so the
//!   first scan of that mount is a real walk — the daemon's boot scan or a reader (ADR-0382).
//! - **Refresh merging**: a refresh joins a covering scan generation that is queued and not yet walking; a covering
//!   walk already in progress does not count — the caller may have changed files before calling (PRD R41).
//! - **A step-9b hold** ([`Gate::change_held`] — ADR-0409 M1): a `Change` ticket and one write-lock hold whose body gets
//!   a `raise` of the change number (not the root generation) to call before its agents pass, the root generation
//!   compared inside the write lock first (ADR-0377). A step that may touch `.trash` raises the trash waiting count before
//!   its ticket and takes the trash lock inside the hold through [`Hold::take_trash`]; a hold without that reservation
//!   never waits on it. The trash lock (`trash.rs` [`TrashLock`]) comes after the library lock and before the turn
//!   mutex — library lock → trash lock → turn mutex, never the reverse — and the waiting count keeps emptying out of it
//!   while a step waits (ADR-0409 decisions 14 and 15).
// ADR-0324
// ADR-0382
// ADR-0400
// ADR-0409
// TRD S25 step-7 §6

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, RwLock};

use engram_dashboard_agent::launch::Reason;
use engram_dashboard_base::sync::{lock, read, write};
use engram_dashboard_platform::fs::{same_path, PathKey};
use serde_json::json;

use crate::catalog::{key, preset_refs, within, Catalog, Item, ItemContent, ScanItem, ScanResult};
use crate::folder::FolderStatus;
use crate::mounts::{moved_to, virtual_path, Mount, MountName, MountPath, Mounts};
use crate::path::LibPath;
use crate::store::ContentTag;
use crate::trash::{TrashHeld, TrashLock, TrashReservation, TrashUnavailable};
use crate::RefreshReport;

/// The code of a preset reference whose target is not in the catalog (`args` = `{path}` — the target as the preset
/// spells it). Agent-preset edits reuse it for an unknown agent with `args` = `{agent}`.
// TRD S25 step-7 B8
pub const NOT_FOUND: &str = "not-found";

/// The answer of a scan whose ticket was released without one (it panicked).
const ENDED_UNANSWERED: &str = "the scan ended without an answer";

/// The `why` of a registered mount no scan has stat'ed or walked yet.
pub const NOT_WALKED: &str = "not scanned yet";

/// Why a scan did not walk a mount registered or moved after it took its stats: its next reader or refresh walks it
/// after a fresh stat.
const NOT_STATED: &str = "registered or moved after this scan's stat; not walked by it";

/// Why a scan did not walk a mount its stat left unstated (`None` from the [`Stat`]): the next scan stats it fresh.
const STAT_STARVED: &str =
    "the load-time overlap check had not finished within this scan's wait; not walked by this scan";

/// What the library lock guards.
pub struct State {
    pub catalog: Arc<Catalog>,
    /// The registered mounts; none = nothing can be scanned.
    // ADR-0400
    pub mounts: Mounts,
    /// Each mount's last known state and first-scan answer, by name key; a registered mount with no entry was never
    /// stat'ed or walked ([`State::seen`]).
    // ADR-0400 decision 17
    seen: HashMap<PathKey, MountSeen>,
    /// The sequence number each entry of `seen` was observed at (drawn before its stat), by name key: an older
    /// observation never replaces a newer state.
    stamps: HashMap<PathKey, u64>,
}

impl State {
    /// `name`'s last known state and first-scan answer; a mount no scan has reached yet is unreadable,
    /// [`NOT_WALKED`], and not walked.
    pub fn seen(&self, name: &MountName) -> MountSeen {
        seen_in(&self.seen, name)
    }

    /// The mounts with what the scans know of each, as a reader under this lock judges them — the view a change gets.
    pub(crate) fn mounts_now(&self) -> MountsNow<'_> {
        MountsNow {
            mounts: &self.mounts,
            seen: &self.seen,
        }
    }
}

/// The mounts as a change sees them inside its write lock: the registered mounts (it derefs to [`Mounts`]) and what
/// the scans know of each, as recorded — no IO, no walk again — so a write can judge its mount right before its disk
/// IO (ADR-0400 decision 6).
pub struct MountsNow<'a> {
    mounts: &'a Mounts,
    seen: &'a HashMap<PathKey, MountSeen>,
}

impl MountsNow<'_> {
    /// [`State::seen`] as the change's write lock holds it.
    pub fn seen(&self, name: &MountName) -> MountSeen {
        seen_in(self.seen, name)
    }
}

impl std::ops::Deref for MountsNow<'_> {
    type Target = Mounts;

    fn deref(&self) -> &Mounts {
        self.mounts
    }
}

fn seen_in(seen: &HashMap<PathKey, MountSeen>, name: &MountName) -> MountSeen {
    seen.get(&name.key()).cloned().unwrap_or(MountSeen {
        state: MountState::Unreadable {
            why: NOT_WALKED.to_owned(),
        },
        walked: false,
    })
}

/// A mount's last known state: from the bounded stat a scan took before its ticket, or its walk's failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MountState {
    Ok,
    /// Missing or not a folder (`why` = the stat's words) — absent (ADR-0387).
    Missing {
        why: String,
    },
    /// There but its stat or walk failed or did not answer — the OS error, a timeout, a share not reachable.
    Unreadable {
        why: String,
    },
}

impl MountState {
    /// The state a folder stat says.
    pub fn of(status: &FolderStatus) -> MountState {
        match status {
            FolderStatus::Ok => MountState::Ok,
            FolderStatus::Missing | FolderStatus::NotADirectory => {
                MountState::Missing { why: status.why() }
            }
            FolderStatus::Unreadable(why) => MountState::Unreadable { why: why.clone() },
        }
    }

    /// The state in one word: `ok`, `missing` or `unreadable` (the wire's mount `state`).
    pub fn word(&self) -> &'static str {
        match self {
            MountState::Ok => "ok",
            MountState::Missing { .. } => "missing",
            MountState::Unreadable { .. } => "unreadable",
        }
    }

    /// `ok`, or the reason it is not.
    pub fn why(&self) -> &str {
        match self {
            MountState::Ok => "ok",
            MountState::Missing { why } | MountState::Unreadable { why } => why,
        }
    }
}

/// What the scans know of one mount.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountSeen {
    pub state: MountState,
    /// The mount's first-scan answer: a walk of its whole folder was applied this run. Its items in the catalog come
    /// from that walk (and later ones), even when its last known state is no longer `Ok`.
    pub walked: bool,
}

impl MountSeen {
    /// Its items can be read: walked, and `Ok` when last seen.
    pub fn usable(&self) -> bool {
        self.walked && self.state == MountState::Ok
    }

    /// The state a reader is told: a mount `Ok` when last stat'ed but whose whole folder was never walked cannot be
    /// read either — unreadable, [`NOT_WALKED`].
    pub fn shown(&self) -> MountState {
        match (&self.state, self.walked) {
            (MountState::Ok, false) => MountState::Unreadable {
                why: NOT_WALKED.to_owned(),
            },
            (state, _) => state.clone(),
        }
    }
}

/// The three numbers a late merge notes before its disk read and compares inside its write lock (ADR-0324 decision
/// 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Numbers {
    /// Raised by every change that applied something (a change refused inside its body with no catalog item applied
    /// raises nothing) and by every mount change, inside its write lock.
    pub change: u64,
    /// Raised by every mount change, inside the write lock that publishes it — one generation for the whole tree;
    /// starts at the per-run base the daemon passes (`Gate::new`), not 0, and is compared only for equality (ADR-0381 ·
    /// ADR-0400 decision 16).
    pub root: u64,
    /// Raised by every scan's apply.
    pub swap_ins: u64,
}

/// The tag and racy mark of every catalog item in a scan's range, read under a brief read lock.
pub(crate) type Tags = HashMap<PathKey, (ContentTag, bool)>;

/// Builds one mount's scan outside every lock from the mount, the range inside it (`None` = its whole folder) and the
/// range's current [`Tags`].
pub(crate) type Build<'a> = &'a dyn Fn(&Mount, Option<&MountPath>, &Tags) -> io::Result<ScanResult>;

/// Stats the folders of `mounts` in one bounded batch, outside every lock, answering one status per mount in order —
/// the service's `BoundedIo` stat with its share probe. `None` = not stated: the stat asked nothing (the load-time
/// overlap check was still running when its wait for it ran out), so it says nothing about the mount — nothing is
/// recorded for it and it is not walked; the next stat asks again.
// ADR-0400 decisions 10 and 17
// ADR-0402 decision 5
pub(crate) type Stat<'a> = &'a dyn Fn(&[Mount]) -> Vec<Option<FolderStatus>>;

/// What a change's body answered, as the gate reads it to decide whether the change generation moves: a refusal that
/// applied no catalog item moves nothing ([`Gate::change_checked`]).
pub trait ChangeAnswer {
    /// The body refused — and so, by the refusal rule, applied nothing outside the catalog keys it touched.
    fn refused(&self) -> bool;
}

impl<T, E> ChangeAnswer for Result<T, E> {
    fn refused(&self) -> bool {
        self.is_err()
    }
}

impl ChangeAnswer for () {
    fn refused(&self) -> bool {
        false
    }
}

/// What a hold's body gets besides the catalog and the mounts ([`Gate::change_held`]).
pub struct Hold<'a> {
    seq: &'a dyn Fn() -> u64,
    raise: &'a dyn Fn(),
    trash: Option<&'a TrashReservation<'a>>,
}

impl Hold<'_> {
    /// The next number of the sequence (a removal draws its number here, so it is later than every read noted before).
    pub fn seq(&self) -> u64 {
        (self.seq)()
    }

    /// Raises the change number — once; later calls do nothing. A body that rewrites agents calls it before they pass,
    /// so an `agent.new` judged before the hold judges again (ADR-0400 decision 18). The root generation keeps its
    /// meaning: it does not move (ADR-0377).
    // ADR-0400 decision 18
    pub fn raise(&self) {
        (self.raise)()
    }

    /// The trash lock, for a hold whose step raised the waiting count before its ticket — taken once per hold for all
    /// its trash moves; it is released when the answer drops, before the write lock. `Err(NotReserved)` = the step did
    /// not raise the count: it never waits on the lock and leaves the entry or staging where it is
    /// (`trash::TRASH_NOT_RESERVED`). `Err(AlreadyHeld)` = this hold holds it already (a second take would wait on
    /// itself).
    // ADR-0409 decision 14
    pub fn take_trash(&self) -> Result<TrashHeld<'_>, TrashUnavailable> {
        match self.trash {
            Some(reservation) => reservation.take(),
            None => Err(TrashUnavailable::NotReserved),
        }
    }
}

/// Whether a mount rename goes on after its body ([`Gate::rename_mount`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RenameEnd {
    /// The references were rewritten: re-key the catalog and rename the registration.
    Finish,
    /// The walk or the agents' rewrite stopped (or the body refused): the old name stays registered.
    Stop,
}

/// What one mount change does ([`Gate::change_mount`] — ADR-0400 decisions 9, 11 and 13).
pub(crate) enum MountChange {
    /// Registers the mount at the end of the list; its folder is walked.
    Add(Mount),
    /// Re-points the mount named `name` at `folder`, keeping its name and place; the new folder is walked.
    Relocate { name: MountName, folder: PathBuf },
    /// Drops the mount named so; nothing is walked and no file is touched.
    Remove(MountName),
}

impl MountChange {
    fn name(&self) -> &MountName {
        match self {
            MountChange::Add(mount) => &mount.name,
            MountChange::Relocate { name, .. } | MountChange::Remove(name) => name,
        }
    }

    /// Whether the change fits `mounts`: `Ok(Some(mount))` = the mount to walk (as it will be registered), `Ok(None)` =
    /// nothing to walk; `Err(why)` = it does not fit.
    fn fits(&self, mounts: &Mounts) -> Result<Option<Mount>, String> {
        let registered = mounts.get(self.name().as_str());
        match (self, registered) {
            (MountChange::Add(mount), None) => Ok(Some(mount.clone())),
            (MountChange::Add(mount), Some(_)) => Err(format!(
                "the library mount name {:?} is taken",
                mount.name.as_str()
            )),
            (MountChange::Relocate { folder, .. }, Some(registered)) => Ok(Some(Mount {
                name: registered.name.clone(),
                folder: folder.clone(),
            })),
            (MountChange::Remove(_), Some(_)) => Ok(None),
            (MountChange::Relocate { name, .. } | MountChange::Remove(name), None) => {
                Err(format!("no library mount is named {:?}", name.as_str()))
            }
        }
    }
}

/// What a scan stat'ed before its ticket: each mount it would walk and that mount's status (`None` = not stated),
/// observed at sequence number `at` (drawn before the stat).
struct Stats {
    at: u64,
    of: Vec<(Mount, Option<FolderStatus>)>,
}

/// The mounts as a scan prepares from them before its ticket, kept under the turn mutex so the preparation takes no
/// library lock: a change drawn after a waiting scan holds the write lock across its body and would otherwise overtake
/// it. Every writer of the mounts or of what the scans know of them refreshes it in the same write-lock hold
/// ([`Gate::write_mounts`]).
// ADR-0324 decision 4
#[derive(Clone, Default, PartialEq)]
struct MountView {
    mounts: Mounts,
    /// The name keys of the mounts whose whole folder was walked this run.
    walked: HashSet<PathKey>,
}

impl MountView {
    fn of(state: &State) -> Self {
        MountView {
            mounts: state.mounts.clone(),
            walked: state
                .seen
                .iter()
                .filter(|(_, seen)| seen.walked)
                .map(|(name, _)| name.clone())
                .collect(),
        }
    }
}

/// A scan's answer, shared by everyone who joined it. `Err` = nothing was applied, with the reason.
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
    /// The part of this report inside `folder` (`None` = all of it). An error at `folder` or at one of its ancestors
    /// (its mount that could not be walked) is that folder's too.
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
                .filter(|(at, _)| match (at, folder) {
                    (_, None) => true,
                    (None, Some(_)) => false,
                    (Some(at), Some(folder)) => {
                        within(at, Some(folder)) || within(folder, Some(at))
                    }
                })
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
    /// The one trash lock over every mount's `.trash` — after the library lock, before the turn mutex (ADR-0409 decision
    /// 15).
    trash: TrashLock,
    /// How many [`Gate::with_read`] calls found the library lock held for writing — tests observe a waiter.
    #[cfg(test)]
    read_blocked: std::sync::atomic::AtomicUsize,
}

struct Turns {
    next: u64,
    /// Every ticket drawn and not yet released, in drawing order.
    line: VecDeque<Ticket>,
    numbers: Numbers,
    first: First,
    view: MountView,
}

#[derive(Default)]
struct First {
    raised: bool,
    /// Full scans drawn before the first scan was raised that have not ended.
    pending: usize,
    /// How many of those have ended — a waiter wakes when it moves.
    ended: u64,
    /// Why the last of them that ended without an apply failed.
    failure: Option<String>,
    /// How many of them ended without an apply — a waiter's failed try is one that moved it; one that applied but
    /// left the first scan un-raised (a mount published in its window, or left unstated) is not.
    failed: u64,
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
    /// A mount change or the first start's publish — exclusive, and never a pending first scan.
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
    /// A scan's answer, handed to its generation and to the first-scan state on release.
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
                first.failed += 1;
            }
        }
        self.gate.moved.notify_all();
    }
}

impl Gate {
    /// `root_generation` = the root generation this run starts at (`Numbers::root`).
    pub(crate) fn new(mounts: Mounts, root_generation: u64) -> Self {
        let state = State {
            catalog: Arc::new(Catalog::default()),
            mounts,
            seen: HashMap::new(),
            stamps: HashMap::new(),
        };
        let view = MountView::of(&state);
        Gate {
            lib: RwLock::new(state),
            turns: Mutex::new(Turns {
                next: 0,
                line: VecDeque::new(),
                numbers: Numbers {
                    change: 0,
                    root: root_generation,
                    swap_ins: 0,
                },
                first: First::default(),
                view,
            }),
            moved: Condvar::new(),
            seq: AtomicU64::new(0),
            trash: TrashLock::default(),
            #[cfg(test)]
            read_blocked: std::sync::atomic::AtomicUsize::new(0),
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
        #[cfg(test)]
        if matches!(
            self.lib.try_read(),
            Err(std::sync::TryLockError::WouldBlock)
        ) {
            self.read_blocked.fetch_add(1, Ordering::SeqCst);
        }
        f(&read(&self.lib))
    }

    /// The first-scan entrance (ADR-0324 decisions 6 · 7): returns at once once raised; else waits for a pending full
    /// scan (queued or walking), or runs a full scan itself — its mounts stat'ed with `stat` before its ticket. A
    /// waited-for one that was not published sends the caller through once more; `Err` = that second try failed too, or
    /// the caller's own scan failed. A whole scan fails only when it ended unanswered (it panicked): a mount it cannot
    /// walk is that mount's state, not the scan's failure (ADR-0400 decision 17). A whole scan that applied but left
    /// the first scan un-raised (a mount was published after its stats were taken, or left unstated) is prepared and
    /// run again.
    pub(crate) fn first_scan_join(&self, stat: Stat, build: Build) -> Result<(), String> {
        let mut turns = lock(&self.turns);
        let mut went_again = false;
        let mut stats: Option<Stats> = None;
        loop {
            if turns.first.raised {
                return Ok(());
            }
            if turns.first.pending > 0 {
                let failed = turns.first.failed;
                turns = self.wait_first(turns);
                stats = None;
                if turns.first.raised {
                    return Ok(());
                }
                // Applied but left un-raised: not a failed try — round again.
                if turns.first.failed == failed {
                    continue;
                }
                if went_again {
                    return Err(failure(&turns));
                }
                went_again = true;
                continue;
            }
            // ADR-0400 decision 17: the bounded stat runs before the ticket, holding neither lock.
            let Some(taken) = stats.take() else {
                let view = turns.view.clone();
                drop(turns);
                stats = Some(self.prepare(&view, None, stat).1);
                turns = lock(&self.turns);
                continue;
            };
            let turn = self.draw(&mut turns, Kind::Scan(Arc::new(Generation::new(None))));
            drop(self.enter(turns, turn.no));
            self.run_scan(turn, None, taken, build)?;
            turns = lock(&self.turns);
        }
    }

    /// A ticketed scan of `range` (ADR-0324 decision 7 — refresh merging). Joins a covering generation that is queued
    /// and not walking; otherwise stats the mounts its range touches with `stat`, holding no lock, and then draws its
    /// own ticket. A folder refresh before the first scan is raised joins a queued full scan or runs one itself and
    /// takes its share (decision 6); one that finds a full scan already walking waits for it and then scans its folder
    /// like any refresh. A range in a mount not walked
    /// yet widens to that mount's top (decision 6 per mount — ADR-0400 decision 17). When the one it joined or waited
    /// for is not published it goes through a full scan once more.
    // ADR-0366
    pub(crate) fn refresh(&self, range: Option<&LibPath>, stat: Stat, build: Build) -> Outcome {
        let mut turns = lock(&self.turns);
        let mut went_again = false;
        let mut prepared: Option<(Option<LibPath>, Option<LibPath>, Stats)> = None;
        loop {
            let before_first = range.is_some() && !turns.first.raised;
            if let Some(generation) = queued_covering(&turns.line, range) {
                let (back, outcome) = self.wait_outcome(turns, &generation);
                turns = back;
                prepared = None;
                if outcome.is_err() && before_first && !went_again {
                    went_again = true;
                    continue;
                }
                return outcome;
            }
            if before_first && turns.first.pending > 0 {
                let failed = turns.first.failed;
                turns = self.wait_first(turns);
                prepared = None;
                // Only one that ended without an apply is a failed try; an un-raised apply sends it round again.
                if !turns.first.raised && turns.first.failed != failed {
                    if went_again {
                        return Err(failure(&turns));
                    }
                    went_again = true;
                }
                continue;
            }
            // ADR-0324 decision 6 · M48: before the first scan only a full scan is published.
            let own = if before_first { None } else { range };
            let (widened, stats) = match prepared.take() {
                Some((asked, widened, stats)) if asked.as_ref() == own => (widened, stats),
                _ => {
                    // ADR-0400 decision 17: the bounded stat runs before the ticket, holding neither lock.
                    let view = turns.view.clone();
                    drop(turns);
                    let (widened, stats) = self.prepare(&view, own, stat);
                    prepared = Some((own.cloned(), widened, stats));
                    turns = lock(&self.turns);
                    continue;
                }
            };
            let generation = Arc::new(Generation::new(widened.clone()));
            let turn = self.draw(&mut turns, Kind::Scan(generation));
            drop(self.enter(turns, turn.no));
            return self.run_scan(turn, widened.as_ref(), stats, build);
        }
    }

    /// What a scan of `range` prepares before its ticket from `view` (the mounts as the turn mutex last saw them),
    /// holding no lock: the range widened to its mount's top when that mount was not walked yet, and the status of every
    /// mount the widened range touches, from one `stat` batch, stamped with a sequence number drawn before it. A range
    /// in no mount stats nothing (its scan answers why).
    fn prepare(
        &self,
        view: &MountView,
        range: Option<&LibPath>,
        stat: Stat,
    ) -> (Option<LibPath>, Stats) {
        let widened = range.map(|range| match view.mounts.locate(range) {
            Some((mount, Some(_))) if !view.walked.contains(&mount.name.key()) => {
                virtual_path(&mount.name, None)
            }
            _ => range.clone(),
        });
        let mounts: Vec<Mount> = walks(&view.mounts, widened.as_ref())
            .map(|walks| walks.into_iter().map(|(mount, _)| mount).collect())
            .unwrap_or_default();
        let at = self.next_seq();
        if mounts.is_empty() {
            return (widened, Stats { at, of: Vec::new() });
        }
        let statuses = stat(&mounts);
        let of = mounts.into_iter().zip(statuses).collect();
        (widened, Stats { at, of })
    }

    /// Stats `mounts` outside every lock and before any ticket, and records each that is not `Ok` as its last known
    /// state under a brief write lock, without a ticket — nothing to walk, so nothing to queue behind a slow walk.
    /// Answers the mounts that stat `Ok`; the caller walks them. A mount removed or moved since is not recorded, nor
    /// one the stat left unstated (neither recorded nor answered).
    // ADR-0400 decision 17
    pub(crate) fn observe(&self, mounts: Vec<Mount>, stat: Stat) -> Vec<Mount> {
        let at = self.next_seq();
        let statuses = stat(&mounts);
        let mut back = Vec::new();
        let mut gone = Vec::new();
        for (mount, status) in mounts.into_iter().zip(statuses) {
            match status {
                Some(FolderStatus::Ok) => back.push(mount),
                Some(status) => gone.push((mount, status)),
                None => {}
            }
        }
        self.record_unusable(gone, at);
        back
    }

    /// Records each of `gone` — a mount whose folder stat, drawn at sequence `at` before it was asked, was not `Ok` —
    /// as its last known state under a brief write lock, without a ticket. A mount removed or moved since is not
    /// recorded, and an older observation never replaces a newer state.
    // ADR-0400 decision 17
    pub(crate) fn record_unusable(&self, gone: Vec<(Mount, FolderStatus)>, at: u64) {
        if gone.is_empty() {
            return;
        }
        let mut state = write(&self.lib);
        let seen: Vec<Observed> = gone
            .into_iter()
            .filter(|(mount, _)| {
                state
                    .mounts
                    .get(mount.name.as_str())
                    .is_some_and(|registered| same_path(&registered.folder, &mount.folder))
            })
            .map(|(mount, status)| (mount.name, MountState::of(&status), false, at))
            .collect();
        self.write_mounts(&mut state, |state| record_seen(state, seen));
    }

    /// The one way to write the mounts or what the scans know of them (`seen`, `stamps`): `f` under the caller's write
    /// lock, then the view scans prepare from ([`MountView`]) refreshed in the same hold (library lock → turn mutex). A
    /// writer that bypassed it would leave a scan preparing from a stale view — its stats would never match the mounts
    /// it walks and its reader would prepare again forever.
    // ADR-0400 decision 17
    fn write_mounts<R>(&self, state: &mut State, f: impl FnOnce(&mut State) -> R) -> R {
        let answer = f(state);
        lock(&self.turns).view = MountView::of(state);
        answer
    }

    /// Debug builds check, at the end of a write-lock hold, that the view equals the state — a writer of the mounts
    /// that bypassed [`Gate::write_mounts`] fails here.
    fn debug_check_view(&self, state: &State) {
        debug_assert!(
            lock(&self.turns).view == MountView::of(state),
            "the mounts were written without Gate::write_mounts"
        );
    }

    /// A change (TRD S25 step-7 §6 writer 5): its own ticket, then `f` under the write lock with the catalog, the
    /// mounts as recorded ([`MountsNow`]) and the sequence (a removal draws its number there, so it is later than every read noted before). `f`
    /// writes per item and returns the touched keys; their derived data is recomputed and the change generation is
    /// raised before the write lock is released. `f` may read the profile registry, call the profile-edit port (its
    /// impl never calls back into the library — ADR-0380) and do the change's own disk IO — nothing else outward
    /// (ADR-0324 decision 1). The change's disk effect must be finished inside `f` (or before the call).
    pub(crate) fn change<R>(
        &self,
        f: impl FnOnce(&mut Catalog, &MountsNow, &dyn Fn() -> u64) -> (R, Vec<PathKey>),
    ) -> R {
        match self.change_in(None, f, |_| false) {
            Ok(answer) => answer,
            Err(_) => unreachable!("a change given no root generation is never stale"),
        }
    }

    /// [`Gate::change`] that first compares `given` (`None` = any) with the root generation inside the same write-lock
    /// take, so a mount change queued before it is seen, and that raises nothing when `f` refused
    /// ([`ChangeAnswer::refused`]) and touched no key. `Err(current)` = stale: `f` is not called and no generation
    /// moves.
    // ADR-0377
    pub(crate) fn change_checked<R: ChangeAnswer>(
        &self,
        given: Option<u64>,
        f: impl FnOnce(&mut Catalog, &MountsNow, &dyn Fn() -> u64) -> (R, Vec<PathKey>),
    ) -> Result<R, u64> {
        self.change_in(given, f, R::refused)
    }

    /// The body of [`Gate::change`] and [`Gate::change_checked`]; `refused` reads `f`'s answer.
    fn change_in<R>(
        &self,
        given: Option<u64>,
        f: impl FnOnce(&mut Catalog, &MountsNow, &dyn Fn() -> u64) -> (R, Vec<PathKey>),
        refused: impl FnOnce(&R) -> bool,
    ) -> Result<R, u64> {
        self.held_in(
            given,
            None,
            |catalog, mounts, hold| f(catalog, mounts, hold.seq),
            refused,
        )
    }

    /// A step-9b hold (ADR-0409 M1): [`Gate::change_checked`] whose body gets a [`Hold`] — the sequence, a `raise` of the
    /// change number to call before its agents pass, and, when `trash`, the trash lock. A `Change` ticket: no mount
    /// registration moves. A body that did not raise has the number raised after it unless it refused and touched no
    /// key. `trash` = the step may touch `.trash`: the trash waiting count is raised before the ticket and the call waits,
    /// holding no lock, until no bundle is being emptied; the count is lowered once the ticket and the write lock are
    /// released — on a stale generation, a refusal or a panic too (ADR-0409 decision 14).
    // ADR-0377
    // ADR-0409 decision 14
    pub(crate) fn change_held<R: ChangeAnswer>(
        &self,
        given: Option<u64>,
        trash: bool,
        f: impl FnOnce(&mut Catalog, &MountsNow, &Hold) -> (R, Vec<PathKey>),
    ) -> Result<R, u64> {
        let reservation = trash.then(|| self.trash.reserve());
        self.held_in(given, reservation.as_ref(), f, R::refused)
    }

    /// The trash lock and its waiting count (`trash.rs`) — emptying takes it per bundle, holding no library lock.
    // ADR-0409 decision 15
    pub(crate) fn trash(&self) -> &TrashLock {
        &self.trash
    }

    fn held_in<R>(
        &self,
        given: Option<u64>,
        trash: Option<&TrashReservation<'_>>,
        f: impl FnOnce(&mut Catalog, &MountsNow, &Hold) -> (R, Vec<PathKey>),
        refused: impl FnOnce(&R) -> bool,
    ) -> Result<R, u64> {
        let _turn = self.take_turn(Kind::Change);
        let mut state = write(&self.lib);
        let current = lock(&self.turns).numbers.root;
        if let Some(given) = given.filter(|&given| given != current) {
            drop(state);
            tracing::debug!(
                given,
                current,
                "library change: root generation stale; refused"
            );
            return Err(current);
        }
        let State {
            catalog,
            mounts,
            seen,
            ..
        } = &mut *state;
        let catalog = Arc::make_mut(catalog);
        let now = MountsNow { mounts, seen };
        let raised = std::cell::Cell::new(false);
        // Library lock → turn mutex: the allowed order; the turn mutex is held for the raise only.
        let raise = || {
            if !raised.replace(true) {
                lock(&self.turns).numbers.change += 1;
            }
        };
        let hold = Hold {
            seq: &|| self.next_seq(),
            raise: &raise,
            trash,
        };
        let (answer, touched) = f(catalog, &now, &hold);
        catalog.recompute_derived(&touched);
        // A refusal applied nothing: a scan walking meanwhile need not walk again, nor a late merge be dropped.
        if !refused(&answer) || !touched.is_empty() {
            raise();
        }
        self.debug_check_view(&state);
        Ok(answer)
    }

    /// A mount change (ADR-0400 decisions 9, 11, 13 and 16): a `Root` ticket, exclusive behind every earlier ticket and
    /// until its publish; an added or relocated mount is walked by `walk` at its new folder outside every lock with
    /// empty tags; then one write lock drops that mount's range from the catalog, applies the walk, writes the mounts
    /// ([`Gate::write_mounts`] — the mount's `seen` and `stamps` cleared, an added or relocated mount then recorded `Ok`
    /// and walked) and raises the root generation and the change generation. It leaves the first scan as it is. `Err` =
    /// nothing published and no number moved: the walk failed at the folder, or the change does not fit the mounts (a
    /// name taken, a mount not registered — the caller judged it under the mount-change mutex first). (The stat and the
    /// overlap judgement of the folder are the caller's — before this call.)
    // ADR-0400 decision 16
    pub(crate) fn change_mount(
        &self,
        change: MountChange,
        walk: impl FnOnce(&Mount) -> io::Result<ScanResult>,
    ) -> Result<RefreshReport, String> {
        let _turn = self.take_turn(Kind::Root);
        // The ticket is exclusive: the mounts read here are the ones the publish changes.
        let walked = self.with_read(|state| change.fits(&state.mounts))?;
        // ADR-0400 decision 17: an observation is stamped with a number drawn before it, as a scan's before its stat.
        let at = self.next_seq();
        let result = match &walked {
            Some(mount) => Some(walk(mount).map_err(|e| e.to_string())?),
            None => None,
        };
        let mut state = write(&self.lib);
        let catalog = Arc::make_mut(&mut state.catalog);
        let dropped = catalog.drop_range(&virtual_path(change.name(), None));
        catalog.recompute_derived(&dropped);
        let report = apply_reported(catalog, result.into_iter().collect());
        self.write_mounts(&mut state, |state| {
            let name = change.name().clone();
            state.seen.remove(&name.key());
            state.stamps.remove(&name.key());
            match change {
                MountChange::Add(mount) => state
                    .mounts
                    .push(mount)
                    .expect("fits() judged the name free under the exclusive ticket"),
                MountChange::Relocate { name, folder } => {
                    let moved = state.mounts.relocate(&name, folder);
                    debug_assert!(moved, "fits() found the mount under the exclusive ticket");
                }
                MountChange::Remove(name) => {
                    state.mounts.remove(&name);
                }
            }
            if walked.is_some() {
                record_seen(state, vec![(name, MountState::Ok, true, at)]);
            }
        });
        let mut turns = lock(&self.turns);
        turns.numbers.root += 1;
        turns.numbers.change += 1;
        self.moved.notify_all();
        drop(turns);
        Ok(report.share(None))
    }

    /// A mount rename (ADR-0400 decisions 12 and 16): a `Root` ticket, exclusive behind every earlier ticket and until
    /// its end, and one write lock across the whole of it — `given` compared with the root generation first (`Err(current)`
    /// = stale: `f` is not called and no number moves), then `f` with the catalog, the mounts as recorded, the sequence
    /// and `raise`. `f` rewrites the preset files (per item, as a change does), calls `raise` — the root generation and
    /// the change number raised once, the new root generation answered — before its agents pass, so an `agent.new`
    /// judged before it judges again (decision 18), and answers whether the rename goes on. On [`RenameEnd::Finish`]
    /// the numbers are raised (if `f` did not) and, when `register`, the catalog range of `from` is re-keyed to `to`
    /// and the registration renamed ([`Gate::write_mounts`] — its `seen` and `stamps` move with it); on
    /// [`RenameEnd::Stop`] the old name stays registered and, when `f` touched an item without raising, only the change
    /// number moves. `register` = `false`: the repeat of a rename whose save failed (`from` already gone, `to` live) —
    /// only the references are rewritten. The caller judged under the mount-change mutex that `from` is registered and
    /// `to` free (or the repeat's case); the mount-change mutex keeps that true through the hold (a rename the mounts
    /// no longer fit would fail a debug assertion in `write_mounts`, after the re-key).
    // ADR-0400 decision 12
    // ADR-0400 decision 16
    pub(crate) fn rename_mount<R>(
        &self,
        from: &MountName,
        to: &MountName,
        register: bool,
        given: Option<u64>,
        f: impl FnOnce(
            &mut Catalog,
            &MountsNow,
            &dyn Fn() -> u64,
            &dyn Fn() -> u64,
        ) -> (R, Vec<PathKey>, RenameEnd),
    ) -> Result<R, u64> {
        let _turn = self.take_turn(Kind::Root);
        let mut state = write(&self.lib);
        let current = lock(&self.turns).numbers.root;
        if let Some(given) = given.filter(|&given| given != current) {
            drop(state);
            tracing::debug!(
                given,
                current,
                "library mount rename: root generation stale; refused"
            );
            return Err(current);
        }
        let raised = std::cell::Cell::new(false);
        // Library lock → turn mutex: the allowed order; the turn mutex is held for the raise only.
        let raise = || {
            let mut turns = lock(&self.turns);
            if !raised.replace(true) {
                turns.numbers.root += 1;
                turns.numbers.change += 1;
            }
            turns.numbers.root
        };
        let (answer, end) = {
            let State {
                catalog,
                mounts,
                seen,
                ..
            } = &mut *state;
            let catalog = Arc::make_mut(catalog);
            let now = MountsNow { mounts, seen };
            let (answer, mut touched, end) = f(catalog, &now, &|| self.next_seq(), &raise);
            if end == RenameEnd::Finish {
                raise();
                if register {
                    let top = virtual_path(from, None);
                    touched.extend(catalog.rekey_range(&top, |path| {
                        moved_to(path, from, to).expect("a path within the old mount's range moves")
                    }));
                }
            }
            catalog.recompute_derived(&touched);
            if !raised.get() && !touched.is_empty() {
                lock(&self.turns).numbers.change += 1;
            }
            (answer, end)
        };
        if end == RenameEnd::Finish && register {
            self.write_mounts(&mut state, |state| {
                let renamed = state.mounts.rename(from, to.clone());
                debug_assert!(
                    renamed,
                    "the caller judged the rename under the mount-change mutex"
                );
                if let Some(seen) = state.seen.remove(&from.key()) {
                    state.seen.insert(to.key(), seen);
                }
                if let Some(at) = state.stamps.remove(&from.key()) {
                    state.stamps.insert(to.key(), at);
                }
            });
        }
        if raised.get() {
            self.moved.notify_all();
        }
        self.debug_check_view(&state);
        Ok(answer)
    }

    /// Publishes `mount` as the only mount without walking it: a Root ticket, exclusive behind every earlier ticket;
    /// one write lock sets the mounts and raises the root generation. This ticket is not counted as pending, and the
    /// first scan is lowered again — a whole scan of the empty list (a reader before the first start) raised it — so
    /// the first scan of `mount` is a real walk. `Err` = a mount is already published: nothing changed. (The stat of
    /// the mount's folder is the caller's — before this call.)
    ///
    /// Unlike a mount change ([`Gate::change_mount`]) it drops no range: it relies on the invariant that while no mount is
    /// published the catalog is empty — a scan, the list safety net and a launch overlay read under a mount, and a
    /// merge noted before this publish is dropped by the raised root generation. No change body may insert a catalog
    /// item while no mount is published (an agent-preset edit runs with no root but writes no item), or it could leave
    /// an item this publish would carry into the new mount.
    // ADR-0382
    pub(crate) fn publish_mount_unwalked(&self, mount: Mount) -> Result<(), String> {
        let _turn = self.take_turn(Kind::Root);
        let mut state = write(&self.lib);
        if let Some(published) = state.mounts.iter().next() {
            return Err(format!(
                "a library mount is already published: {} at {}",
                published.name.as_str(),
                published.folder.display()
            ));
        }
        self.write_mounts(&mut state, |state| {
            state.seen.remove(&mount.name.key());
            state.stamps.remove(&mount.name.key());
            state.mounts = Mounts::new(vec![mount]).expect("one mount takes no name");
        });
        let mut turns = lock(&self.turns);
        turns.numbers.root += 1;
        // ADR-0382: the empty list's first scan walked nothing; the new mount's must be a real walk.
        turns.first.raised = false;
        Ok(())
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
                "library late merge: generation moved after the read"
            );
            return Vec::new();
        }
        let catalog = Arc::make_mut(&mut state.catalog);
        let touched = f(catalog, now.swap_ins == noted.swap_ins);
        catalog.recompute_derived(&touched);
        self.debug_check_view(&state);
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
                Kind::Root | Kind::Change => false,
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

    fn run_scan(
        &self,
        mut turn: Turn<'_>,
        range: Option<&LibPath>,
        stats: Stats,
        build: Build,
    ) -> Outcome {
        let outcome = self.scan_until_applied(range, &stats, build);
        turn.outcome = Some(outcome.clone());
        outcome
    }

    /// Walks every mount the range touches whose state is `Ok`, and applies them in one write lock with each mount's
    /// [`MountSeen`]. A mount's state is its stat from `stats` (taken before the ticket), unless the scans recorded a
    /// newer observation of it meanwhile — that one wins. A mount `stats` has no status for (registered or moved after
    /// they were taken) is not walked by this scan and nothing is recorded for it: no stat is taken under the ticket,
    /// and a whole scan that left one out does not raise the first scan; its next reader or refresh walks it after a
    /// fresh stat.
    // ADR-0400 decision 17
    fn scan_until_applied(&self, range: Option<&LibPath>, stats: &Stats, build: Build) -> Outcome {
        loop {
            let started = self.numbers().change;
            let (walks, tags, recorded) = self.with_read(|state| {
                let recorded: HashMap<PathKey, (MountState, u64)> = state
                    .stamps
                    .iter()
                    .filter_map(|(name, at)| {
                        let seen = state.seen.get(name)?;
                        Some((name.clone(), (seen.state.clone(), *at)))
                    })
                    .collect();
                (
                    walks(&state.mounts, range),
                    state.catalog.tags(range),
                    recorded,
                )
            });
            let walks = walks?;
            let mut results = Vec::new();
            // Per mount walked or not: its state now, whether its whole folder was walked, and when it was observed.
            let mut seen: Vec<Observed> = Vec::new();
            let mut unusable: Vec<(LibPath, String)> = Vec::new();
            let mut unstated = false;
            for (mount, under) in walks {
                let top = virtual_path(&mount.name, None);
                // ADR-0402 decision 5: not stated — nothing recorded, the first scan not raised on it.
                let status = match status_in(stats, &mount) {
                    None => {
                        unstated = true;
                        unusable.push((top, NOT_STATED.to_owned()));
                        continue;
                    }
                    Some(None) => {
                        unstated = true;
                        unusable.push((top, STAT_STARVED.to_owned()));
                        continue;
                    }
                    Some(Some(status)) => status,
                };
                // ADR-0400 decision 17: an observation recorded after this scan's stat is newer than it.
                let (mount_state, at) = match recorded.get(&mount.name.key()) {
                    Some((newer, at)) if *at > stats.at => (newer.clone(), *at),
                    _ => (MountState::of(&status), stats.at),
                };
                if mount_state != MountState::Ok {
                    unusable.push((top, mount_state.why().to_owned()));
                    seen.push((mount.name, mount_state, false, at));
                    continue;
                }
                match build(&mount, under.as_ref(), &tags) {
                    Ok(result) => {
                        results.push(result);
                        seen.push((mount.name, MountState::Ok, under.is_none(), at));
                    }
                    // A whole mount that cannot be walked is unreadable; a folder inside it that cannot be is only
                    //   that scan's failure.
                    Err(e) if under.is_none() => {
                        let state = MountState::Unreadable { why: e.to_string() };
                        unusable.push((top, e.to_string()));
                        seen.push((mount.name, state, false, at));
                    }
                    Err(e) => {
                        unusable.push((top, e.to_string()));
                        seen.push((mount.name, MountState::Ok, false, at));
                    }
                }
            }
            let mut state = write(&self.lib);
            if let (Some(_), Some((_, why))) = (range, unusable.first()) {
                let why = why.clone();
                self.write_mounts(&mut state, |state| record_seen(state, seen));
                return Err(why);
            }
            // ADR-0324 decision 4 (M35): a change applied since the walk started could be undone by this result.
            if self.numbers().change != started {
                drop(state);
                tracing::debug!(
                    range = range.map(LibPath::as_str),
                    "library scan: change generation moved during the scan; rescanning"
                );
                continue;
            }
            let mut report = apply_reported(Arc::make_mut(&mut state.catalog), results);
            report
                .errors
                .extend(unusable.into_iter().map(|(top, why)| (Some(top), why)));
            self.write_mounts(&mut state, |state| record_seen(state, seen));
            let mut turns = lock(&self.turns);
            turns.numbers.swap_ins += 1;
            if range.is_none() && !unstated {
                turns.first.raised = true;
            }
            self.moved.notify_all();
            return Ok(Arc::new(report));
        }
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

/// The walks of a scan of `range` (`None` = the whole library): each mount it touches with the range inside it. `Err` =
/// the range's first name names no registered mount — nothing to walk.
fn walks(
    mounts: &Mounts,
    range: Option<&LibPath>,
) -> Result<Vec<(Mount, Option<MountPath>)>, String> {
    match range {
        None => Ok(mounts.iter().map(|mount| (mount.clone(), None)).collect()),
        Some(range) => match mounts.locate(range) {
            Some((mount, under)) => Ok(vec![(mount.clone(), under)]),
            None => Err(format!(
                "no library mount is named {:?}",
                range.as_str().split('/').next().unwrap_or_default()
            )),
        },
    }
}

/// The status `stats` holds for `mount` — the same name, at the same folder; `None` = not stat'ed (registered or moved
/// after the stat), `Some(None)` = stat'ed but left unstated.
fn status_in(stats: &Stats, mount: &Mount) -> Option<Option<FolderStatus>> {
    stats
        .of
        .iter()
        .find(|(stated, _)| {
            stated.name.key() == mount.name.key() && same_path(&stated.folder, &mount.folder)
        })
        .map(|(_, status)| status.clone())
}

/// One observation of a mount: its name, its state, whether its whole folder was walked, and the sequence number it
/// was observed at.
type Observed = (MountName, MountState, bool, u64);

/// Records each mount's state; a mount stays walked once its whole folder was walked this run — its first-scan answer.
/// An observation older than the one recorded keeps the recorded state (a stale `missing` never overwrites a newer
/// `ok`).
// ADR-0400 decision 17
fn record_seen(state: &mut State, seen: Vec<Observed>) {
    for (name, mount_state, whole, at) in seen {
        let key = name.key();
        let before = state.seen(&name);
        let newer = state
            .stamps
            .get(&key)
            .is_none_or(|&recorded| at >= recorded);
        let mount_state = match newer {
            true => {
                state.stamps.insert(key.clone(), at);
                mount_state
            }
            false => before.state,
        };
        state.seen.insert(
            key,
            MountSeen {
                state: mount_state,
                walked: whole || before.walked,
            },
        );
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

/// `Catalog::apply_scan` of each result (one per mount walked) plus the derived data, reporting what they did. A
/// preset's reference is judged missing only after every result is applied, so a reference into another mount of the
/// same scan is not reported.
fn apply_reported(catalog: &mut Catalog, results: Vec<ScanResult>) -> ScanReport {
    let mut report = ScanReport::default();
    let mut presets_read: Vec<(LibPath, Vec<LibPath>)> = Vec::new();
    for result in results {
        // The paths of the range's items before the apply, so a removed key can still be named.
        let before: HashMap<PathKey, LibPath> = catalog
            .tags(result.range.as_ref())
            .into_keys()
            .filter_map(|k| catalog.get(&k).map(|item| (k, item.path.clone())))
            .collect();
        for entry in &result.entries {
            let ScanItem::Fresh(item) = entry else {
                continue;
            };
            report.problems.extend(
                item_problems(item)
                    .into_iter()
                    .map(|reason| (item.path.clone(), reason)),
            );
            if let ItemContent::Preset { doc, .. } = &item.content {
                presets_read.push((item.path.clone(), preset_refs(doc).cloned().collect()));
            }
        }
        report.no_kind.extend(result.no_kind.iter().cloned());
        report
            .skipped_links
            .extend(result.skipped_links.iter().cloned());
        report.errors.extend(result.errors.iter().cloned());
        let touched = catalog.apply_scan(result);
        catalog.recompute_derived(&touched);
        report.changed.extend(touched.iter().filter_map(|k| {
            catalog
                .get(k)
                .map(|item| item.path.clone())
                .or_else(|| before.get(k).cloned())
        }));
    }
    for (preset, targets) in presets_read {
        for target in targets {
            if catalog.get(&key(&target)).is_none() {
                let reason = Reason {
                    code: NOT_FOUND.to_owned(),
                    args: json!({ "path": target.as_str() }),
                    line: None,
                };
                report.problems.push((preset.clone(), reason));
            }
        }
    }
    report
}

fn item_problems(item: &Item) -> Vec<Reason> {
    match &item.content {
        ItemContent::Broken { reason } => vec![reason.clone()],
        ItemContent::Preset { doc, .. } => doc.line_problems.clone(),
        ItemContent::Settings { verdict, .. } => verdict
            .problems
            .iter()
            .map(|(_, reason)| reason.clone())
            .collect(),
        // ADR-0420: a hook whose `hooks.json` fails the shape check.
        ItemContent::Hook { definition, .. } => {
            definition.as_ref().err().cloned().into_iter().collect()
        }
        ItemContent::Instruction { .. }
        | ItemContent::Skill { .. }
        | ItemContent::SkillFile
        | ItemContent::HookFile => Vec::new(),
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

    /// How many [`Gate::with_read`] calls so far found the library lock held for writing.
    pub(crate) fn read_blocked(&self) -> usize {
        self.read_blocked.load(Ordering::SeqCst)
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
    use std::path::Path;
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

    /// A stat that finds every mount there.
    fn ok(mounts: &[Mount]) -> Vec<Option<FolderStatus>> {
        vec![Some(FolderStatus::Ok); mounts.len()]
    }

    /// The empty result of a walk of `under` in `mount` (`None` = its whole folder).
    fn walked(mount: &Mount, under: Option<&MountPath>) -> ScanResult {
        empty(Some(&virtual_path(&mount.name, under)))
    }

    /// The first mount's folder.
    fn root_of(state: &State) -> Option<PathBuf> {
        state.mounts.iter().next().map(|first| first.folder.clone())
    }

    /// The mount `library` relocated to `to` by a mount change whose walk is `walk`.
    fn relocate(
        gate: &Gate,
        to: &str,
        walk: impl FnOnce(&Mount) -> io::Result<ScanResult>,
    ) -> Result<RefreshReport, String> {
        let change = MountChange::Relocate {
            name: MountName::library(),
            folder: PathBuf::from(to),
        };
        gate.change_mount(change, walk)
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

    /// The root generation the test gates start at — not 0, so an absolute value cannot pass by chance.
    const BASE: u64 = 7_000;

    /// The mount the first start registers, at `folder`.
    fn library_at(folder: &str) -> Mount {
        Mount {
            name: crate::mounts::MountName::library(),
            folder: PathBuf::from(folder),
        }
    }

    fn mounts_at(folder: &str) -> Mounts {
        Mounts::new(vec![library_at(folder)]).unwrap()
    }

    fn gate() -> Arc<Gate> {
        Arc::new(Gate::new(mounts_at("/lib"), BASE))
    }

    /// A gate whose first scan is already raised, so a folder refresh scans only its folder.
    fn raised_gate() -> Arc<Gate> {
        let gate = gate();
        gate.refresh(None, &ok, &|mount, under, _| Ok(walked(mount, under)))
            .unwrap();
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
            gate.refresh(range.as_ref(), &ok, &|mount, under, _| {
                log.push(format!("{name} walks"));
                if let Some(door) = &door {
                    door.wait();
                }
                Ok(walked(mount, under))
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
            gate.change(|_, mounts, _| {
                log.push(format!("{name} enters"));
                if let Some(door) = &door {
                    door.wait();
                }
                (
                    mounts.iter().next().map(|first| first.folder.clone()),
                    Vec::new(),
                )
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
    fn a_change_drawn_during_a_mount_change_enters_after_the_publish_and_sees_the_new_folder() {
        let (gate, log) = (gate(), Arc::new(Log::default()));
        let door = Arc::new(Door::default());
        let root_change = {
            let (gate, log, door) = (Arc::clone(&gate), Arc::clone(&log), Arc::clone(&door));
            thread::spawn(move || {
                relocate(&gate, "/new", |mount| {
                    door.wait();
                    log.push("root walked");
                    Ok(walked(mount, None))
                })
            })
        };
        gate.wait_for("root walking", |line, _| line == [("root", true)]);
        let change = change_thread(&gate, &log, "c", None);
        gate.wait_for("c waits", |line, _| {
            line == [("root", true), ("change", false)]
        });
        assert_eq!(
            gate.with_read(root_of),
            Some(PathBuf::from("/lib")),
            "readers keep the old root during the walk"
        );
        door.open();
        join(root_change).unwrap();
        assert_eq!(join(change), Some(PathBuf::from("/new")));
        assert_eq!(log.take(), ["root walked", "c enters"]);
        assert_eq!(gate.numbers().root, BASE + 1);
    }

    #[test]
    fn a_mount_change_whose_walk_fails_publishes_nothing_and_releases_its_ticket() {
        let (gate, log) = (gate(), Arc::new(Log::default()));
        let outcome = relocate(&gate, "/new", |_| {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        });
        assert!(outcome.is_err());
        assert_eq!(gate.with_read(root_of), Some(PathBuf::from("/lib")));
        assert_eq!(
            (gate.numbers().root, gate.numbers().change),
            (BASE, 0),
            "no number moved"
        );
        join(change_thread(&gate, &log, "c", None));
        assert_eq!(log.take(), ["c enters"]);
    }

    #[test]
    fn an_unwalked_first_publish_waits_behind_earlier_tickets_and_leaves_the_first_scan_to_a_real_walk(
    ) {
        let gate = Arc::new(Gate::new(Mounts::default(), BASE));
        let (log, door) = (Arc::new(Log::default()), Arc::new(Door::default()));
        let blocker = change_thread(&gate, &log, "blocker", Some(Arc::clone(&door)));
        gate.wait_for("blocker in", |line, _| line == [("change", true)]);
        let publish = {
            let gate = Arc::clone(&gate);
            thread::spawn(move || gate.publish_mount_unwalked(library_at("/lib")))
        };
        gate.wait_for("the publish waits behind the change", |line, _| {
            line == [("change", true), ("root", false)]
        });
        door.open();
        join(blocker);
        join(publish).unwrap();
        assert_eq!(gate.with_read(root_of), Some(PathBuf::from("/lib")));
        assert_eq!(
            (gate.numbers().root, gate.numbers().swap_ins),
            (BASE + 1, 0),
            "the root generation moved; nothing was walked or applied"
        );
        let walks = Log::default();
        let walk = |mount: &Mount, under: Option<&MountPath>, _: &Tags| -> io::Result<ScanResult> {
            walks.push(format!("walks {}", mount.folder.display()));
            Ok(walked(mount, under))
        };
        gate.first_scan_join(&ok, &walk).unwrap();
        assert_eq!(
            walks.take(),
            [format!("walks {}", Path::new("/lib").display())],
            "the first scan was left un-raised"
        );
        gate.first_scan_join(&ok, &walk).unwrap();
        assert_eq!(walks.take().len(), 1, "raised by that walk");

        let refused = gate.publish_mount_unwalked(library_at("/other"));
        assert!(refused.unwrap_err().contains("already published"));
        assert_eq!(
            (gate.with_read(root_of), gate.numbers().root),
            (Some(PathBuf::from("/lib")), BASE + 1),
            "a refused publish changes nothing"
        );
        join(change_thread(&gate, &log, "after", None));
        assert_eq!(
            log.take(),
            ["blocker enters", "after enters"],
            "tickets released"
        );
    }

    // ADR-0382: a reader before the first start raised the first scan over no mount; the publish lowers it again.
    #[test]
    fn a_first_publish_after_a_whole_scan_of_no_mount_still_leaves_the_mounts_first_scan_to_a_real_walk(
    ) {
        let gate = Arc::new(Gate::new(Mounts::default(), BASE));
        let walks = Log::default();
        let walk = |mount: &Mount, under: Option<&MountPath>, _: &Tags| -> io::Result<ScanResult> {
            walks.push(format!("walks {}", mount.folder.display()));
            Ok(walked(mount, under))
        };
        gate.first_scan_join(&ok, &walk).unwrap();
        assert!(walks.take().is_empty(), "no mount, nothing walked");
        gate.publish_mount_unwalked(library_at("/lib")).unwrap();
        gate.first_scan_join(&ok, &walk).unwrap();
        assert_eq!(
            walks.take(),
            [format!("walks {}", Path::new("/lib").display())],
            "the mount's first scan is a real walk"
        );
    }

    #[test]
    fn a_reader_arriving_while_an_unwalked_publish_waits_draws_its_own_scan_and_walks_the_new_root()
    {
        let gate = Arc::new(Gate::new(Mounts::default(), BASE));
        let (log, door) = (Arc::new(Log::default()), Arc::new(Door::default()));
        let blocker = change_thread(&gate, &log, "blocker", Some(Arc::clone(&door)));
        gate.wait_for("blocker in", |line, _| line == [("change", true)]);
        let publish = {
            let gate = Arc::clone(&gate);
            thread::spawn(move || gate.publish_mount_unwalked(library_at("/lib")))
        };
        gate.wait_for("the publish queued", |line, _| line.len() == 2);
        let stats = Arc::new(Log::default());
        let reader = {
            let (gate, log, stats) = (Arc::clone(&gate), Arc::clone(&log), Arc::clone(&stats));
            thread::spawn(move || {
                gate.first_scan_join(
                    &|mounts| {
                        // ADR-0400 decision 17: never stat'ed under the reader's own entered ticket.
                        let under_ticket = std::cell::Cell::new(false);
                        gate.wait_for("a look at the line", |line, _| {
                            under_ticket.set(
                                line.iter()
                                    .any(|&(kind, entered)| kind == "scan" && entered),
                            );
                            true
                        });
                        stats.push(format!(
                            "stat {} under ticket {}",
                            mounts.len(),
                            under_ticket.get()
                        ));
                        ok(mounts)
                    },
                    &|mount, under, _| {
                        log.push(format!("reader walks {}", mount.folder.display()));
                        Ok(walked(mount, under))
                    },
                )
            })
        };
        // Not a pending first scan: the reader does not wait on it, it queues its own full scan behind it.
        gate.wait_for("the reader's own scan queued", |line, waiting| {
            line == [("change", true), ("root", false), ("scan", false)] && waiting == 0
        });
        door.open();
        join(blocker);
        join(publish).unwrap();
        join(reader).unwrap();
        assert_eq!(
            log.take(),
            [
                "blocker enters".to_owned(),
                format!("reader walks {}", Path::new("/lib").display())
            ]
        );
        // Its first scan was prepared before the publish (no mount, nothing to stat): it walked nothing, applied, left
        //   the first scan un-raised, and the reader prepared again — the new mount's stat taken outside every ticket.
        assert_eq!(stats.take(), ["stat 1 under ticket false"]);
        assert_eq!(gate.numbers().swap_ins, 2, "two scans applied");
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
                    relocate(&gate, "/new", |_| panic!("a mount walk panics on purpose"))
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
                    gate.refresh(None, &ok, &|_, _, _| panic!("a scan panics on purpose"))
                }))
                .is_err()
            })
        };
        gate.wait_for("scan queued", |line, _| kinds(line) == ["change", "scan"]);
        let joiner = scan_thread(&gate, &log, "joiner", Some("library/a"), None);
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
        let a = scan_thread(&gate, &log, "a", Some("library/a"), None);
        gate.wait_for("a queued", |line, _| line.len() == 2);
        // Covered by the queued `a`: joins it.
        let inner = scan_thread(&gate, &log, "a/x", Some("library/a/x"), None);
        let same = scan_thread(&gate, &log, "a again", Some("library/a"), None);
        gate.wait_for("both joined a", |_, waiting| waiting == 2);
        // Not covered by `a`: their own generations.
        let outside = scan_thread(&gate, &log, "b", Some("library/b"), None);
        gate.wait_for("b queued", |line, _| line.len() == 3);
        let whole = scan_thread(&gate, &log, "all", None, None);
        gate.wait_for("all queued", |line, _| line.len() == 4);
        // Covered by the queued whole-library generation.
        let under_whole = scan_thread(&gate, &log, "c", Some("library/c"), None);
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
        let late = scan_thread(&gate, &log, "late", Some("library/a"), None);
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
                gate.first_scan_join(&ok, &|mount, under, _| {
                    log.push("first walks");
                    if let Some(door) = &door {
                        door.wait();
                    }
                    Ok(walked(mount, under))
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

    // ADR-0400 decision 17: a whole scan fails only when it ends unanswered (its holder panicked) — a mount it cannot
    //   walk is that mount's state, not the scan's failure.
    #[test]
    fn a_reader_whose_waited_first_scan_failed_goes_through_once_more() {
        let gate = gate();
        let walks = Arc::new(Log::default());
        let door = Arc::new(Door::default());
        let starter = {
            let (gate, walks, door) = (Arc::clone(&gate), Arc::clone(&walks), Arc::clone(&door));
            thread::spawn(move || {
                catch_unwind(AssertUnwindSafe(|| {
                    gate.first_scan_join(&ok, &|_, _, _| {
                        walks.push("failing");
                        door.wait();
                        panic!("a first scan panics on purpose")
                    })
                }))
                .is_err()
            })
        };
        gate.wait_for("first walking", |line, _| line == [("scan", true)]);
        let joiner = {
            let (gate, walks) = (Arc::clone(&gate), Arc::clone(&walks));
            thread::spawn(move || {
                gate.first_scan_join(&ok, &|mount, under, _| {
                    walks.push("again");
                    Ok(walked(mount, under))
                })
            })
        };
        gate.wait_for("joiner waits", |_, waiting| waiting == 1);
        door.open();
        assert!(join(starter), "the starter's own scan failed");
        let answer = join(joiner);
        assert_eq!(walks.take(), ["failing", "again"]);
        assert!(answer.is_ok(), "the joiner's own scan was applied");
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
                catch_unwind(AssertUnwindSafe(|| {
                    gate.refresh(None, &ok, &|_, _, _| {
                        log.push("failing walks");
                        panic!("a whole scan panics on purpose")
                    })
                }))
                .is_err()
            })
        };
        gate.wait_for("the whole scan queued", |line, _| {
            line == [("change", true), ("scan", false)]
        });
        let folder = {
            let (gate, log) = (Arc::clone(&gate), Arc::clone(&log));
            thread::spawn(move || {
                gate.refresh(Some(&lp("library/a")), &ok, &|mount, under, _| {
                    log.push(format!("again walks {:?}", under.map(MountPath::as_str)));
                    Ok(walked(mount, under))
                })
            })
        };
        gate.wait_for("the folder refresh joined it", |_, waiting| waiting == 1);
        door.open();
        join(blocker);
        assert!(join(failing), "the whole scan ended unanswered");
        assert!(join(folder).is_ok(), "went through once more");
        assert_eq!(
            log.take(),
            ["blocker enters", "failing walks", "again walks None"]
        );
    }

    #[test]
    fn a_mount_change_enters_only_when_every_earlier_ticket_has_finished() {
        let (gate, log) = (raised_gate(), Arc::new(Log::default()));
        let root_thread = |door: Option<Arc<Door>>, to: &'static str| {
            let (gate, log) = (Arc::clone(&gate), Arc::clone(&log));
            thread::spawn(move || {
                relocate(&gate, to, |mount| {
                    log.push(format!("{to} walks"));
                    if let Some(door) = &door {
                        door.wait();
                    }
                    Ok(walked(mount, None))
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
        assert_eq!(gate.with_read(root_of), Some(PathBuf::from("/r2")));
    }

    #[test]
    fn a_scan_whose_walk_saw_the_change_generation_move_walks_again() {
        let gate = gate();
        let walks = Arc::new(Log::default());
        let door = Arc::new(Door::default());
        let walking = Arc::new(Door::default());
        let scan = {
            let (gate, walks, door, walking) = (
                Arc::clone(&gate),
                Arc::clone(&walks),
                Arc::clone(&door),
                Arc::clone(&walking),
            );
            thread::spawn(move || {
                gate.refresh(None, &ok, &|mount, under, _| {
                    walks.push("walk");
                    if walks.take().len() == 1 {
                        walking.open();
                        door.wait();
                    }
                    Ok(walked(mount, under))
                })
            })
        };
        // Inside the walk, not merely at the ticket's entry: the scan notes the change generation between the two, and
        //   a change landing before that note is one the walk already sees.
        walking.wait();
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
                gate.refresh(None, &ok, &|mount, under, _| {
                    walking.open();
                    door.wait();
                    Ok(walked(mount, under))
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
        gate.refresh(None, &ok, &|mount, under, _| Ok(walked(mount, under)))
            .unwrap();
        assert_eq!(
            calls(&gate, noted),
            Some(false),
            "a swap-in landed: updates only"
        );
        let noted = gate.numbers();
        gate.change(|_, _, _| ((), Vec::new()));
        assert_eq!(calls(&gate, noted), None, "the change generation moved");
        let noted = gate.numbers();
        relocate(&gate, "/new", |mount| Ok(walked(mount, None))).unwrap();
        assert_eq!(calls(&gate, noted), None, "the root generation moved");
    }

    // ADR-0400: a whole scan walks every mount under its one ticket; a folder's walks only its mount; a range in no
    //   mount walks nothing and answers why.
    #[test]
    fn a_whole_scan_walks_every_mount_under_one_ticket_and_a_range_only_its_own() {
        let two = Mounts::new(vec![
            library_at("/lib"),
            Mount {
                name: MountName::parse("team").unwrap(),
                folder: PathBuf::from("/team"),
            },
        ])
        .unwrap();
        let gate = Gate::new(two, BASE);
        let walks = Log::default();
        let build =
            |mount: &Mount, under: Option<&MountPath>, _: &Tags| -> io::Result<ScanResult> {
                walks.push(format!(
                    "{} {:?}",
                    mount.name.as_str(),
                    under.map(MountPath::as_str)
                ));
                Ok(walked(mount, under))
            };
        gate.refresh(None, &ok, &build).unwrap();
        assert_eq!(walks.take(), ["library None", "team None"]);
        assert_eq!(gate.numbers().swap_ins, 1, "applied in one write lock");
        gate.refresh(Some(&lp("team/rules")), &ok, &build).unwrap();
        assert_eq!(walks.take()[2..], ["team Some(\"rules\")"]);
        let nowhere = gate.refresh(Some(&lp("nope/rules")), &ok, &build);
        assert!(nowhere.unwrap_err().contains("nope"));
        assert_eq!(walks.take().len(), 3, "nothing walked");
    }

    // ADR-0400 decision 17: the stat runs before the ticket, holding neither lock — a stat that does not answer leaves
    //   the line free, so a change drawn meanwhile enters and finishes.
    #[test]
    fn a_stat_that_does_not_answer_holds_no_ticket_and_no_lock() {
        let (gate, log) = (gate(), Arc::new(Log::default()));
        let (stating, door) = (Arc::new(Door::default()), Arc::new(Door::default()));
        let reader = {
            let (gate, log, stating, door) = (
                Arc::clone(&gate),
                Arc::clone(&log),
                Arc::clone(&stating),
                Arc::clone(&door),
            );
            thread::spawn(move || {
                gate.first_scan_join(
                    &|mounts| {
                        stating.open();
                        door.wait();
                        ok(mounts)
                    },
                    &|mount, under, _| {
                        log.push("the first scan walks");
                        Ok(walked(mount, under))
                    },
                )
            })
        };
        stating.wait();
        gate.wait_for("no ticket while the stat is held", |line, _| {
            line.is_empty()
        });
        assert!(
            gate.lib().try_write().is_ok(),
            "the stat holds a library guard"
        );
        join(change_thread(&gate, &log, "c", None));
        door.open();
        join(reader).unwrap();
        assert_eq!(log.take(), ["c enters", "the first scan walks"]);
    }

    /// Two mounts more: `team` at `/team` and `ops` at `/ops`.
    fn three_mounts() -> Mounts {
        Mounts::new(vec![
            library_at("/lib"),
            Mount {
                name: MountName::parse("team").unwrap(),
                folder: PathBuf::from("/team"),
            },
            Mount {
                name: MountName::parse("ops").unwrap(),
                folder: PathBuf::from("/ops"),
            },
        ])
        .unwrap()
    }

    // ADR-0402 decision 5: a mount the stat left unstated (`None`) is not walked, records nothing — no state, no
    //   stamp — and keeps a whole scan from raising the first scan, its error row saying why; `observe` neither
    //   records nor answers it. The stated mounts are walked and recorded as usual.
    #[test]
    fn an_unstated_mount_is_not_walked_records_nothing_and_does_not_raise_the_first_scan() {
        let gate = Gate::new(three_mounts(), BASE);
        let walks = Log::default();
        let stat = |mounts: &[Mount]| -> Vec<Option<FolderStatus>> {
            mounts
                .iter()
                .map(|mount| match mount.name.as_str() {
                    "team" => None,
                    _ => Some(FolderStatus::Ok),
                })
                .collect()
        };
        let build =
            |mount: &Mount, under: Option<&MountPath>, _: &Tags| -> io::Result<ScanResult> {
                walks.push(mount.name.as_str());
                Ok(walked(mount, under))
            };
        let report = gate.refresh(None, &stat, &build).unwrap();
        assert_eq!(
            walks.take(),
            ["library", "ops"],
            "the unstated mount is not walked"
        );
        assert_eq!(
            report.share(None).errors,
            [(Some(lp("team")), STAT_STARVED.to_owned())]
        );
        let team = MountName::parse("team").unwrap();
        let (seen, stamped) = gate.with_read(|state| {
            (
                state.seen.get(&team.key()).cloned(),
                state.stamps.contains_key(&team.key()),
            )
        });
        assert_eq!((seen, stamped), (None, false), "nothing recorded for it");
        assert!(
            gate.with_read(|state| state.seen(&MountName::parse("ops").unwrap()).usable()),
            "the stated mounts are recorded"
        );
        assert!(
            !lock(&gate.turns).first.raised,
            "the first scan is not raised on it"
        );

        let team_mount = gate.with_read(|state| state.mounts.get("team").cloned().unwrap());
        let unstated = |mounts: &[Mount]| -> Vec<Option<FolderStatus>> { vec![None; mounts.len()] };
        assert!(
            gate.observe(vec![team_mount], &unstated).is_empty(),
            "not answered"
        );
        let seen = gate.with_read(|state| state.seen.get(&team.key()).cloned());
        assert_eq!(seen, None, "observe records nothing for it");
    }

    // ADR-0400 decision 17: a whole scan walks the mounts that stat `Ok`; one that does not, or whose walk fails, is
    //   that mount's state and an error of the report — the others are applied and the first scan is raised.
    #[test]
    fn a_whole_scan_applies_the_mounts_it_can_walk_and_records_the_others() {
        let gate = Gate::new(three_mounts(), BASE);
        let walks = Log::default();
        let stat = |mounts: &[Mount]| -> Vec<Option<FolderStatus>> {
            mounts
                .iter()
                .map(|mount| match mount.name.as_str() {
                    "team" => Some(FolderStatus::Missing),
                    _ => Some(FolderStatus::Ok),
                })
                .collect()
        };
        let build =
            |mount: &Mount, under: Option<&MountPath>, _: &Tags| -> io::Result<ScanResult> {
                walks.push(mount.name.as_str());
                match mount.name.as_str() {
                    "ops" => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
                    _ => Ok(walked(mount, under)),
                }
            };
        let report = gate.refresh(None, &stat, &build).unwrap();
        assert_eq!(
            walks.take(),
            ["library", "ops"],
            "a mount not there is not walked"
        );
        let denied = io::Error::from(io::ErrorKind::PermissionDenied).to_string();
        assert_eq!(
            report.share(None).errors,
            [
                (Some(lp("team")), "missing".to_owned()),
                (Some(lp("ops")), denied.clone())
            ]
        );
        let seen =
            |name: &str| gate.with_read(|state| state.seen(&MountName::parse(name).unwrap()));
        assert_eq!(
            (seen("library"), seen("team"), seen("ops")),
            (
                MountSeen {
                    state: MountState::Ok,
                    walked: true
                },
                MountSeen {
                    state: MountState::Missing {
                        why: "missing".to_owned()
                    },
                    walked: false
                },
                MountSeen {
                    state: MountState::Unreadable { why: denied },
                    walked: false
                },
            )
        );
        assert_eq!(gate.numbers().swap_ins, 1);
        gate.first_scan_join(&|_| panic!("raised: no stat"), &|_, _, _| {
            panic!("raised: no walk")
        })
        .unwrap();
    }

    // ADR-0400 decision 17: a folder scan in a mount not walked yet walks the whole mount; in a walked mount only the
    //   folder; in a mount that does not stat `Ok` nothing, answering why and recording the mount's state.
    #[test]
    fn a_folder_scan_widens_to_a_mount_not_walked_yet_and_fails_on_one_not_there() {
        let gate = Gate::new(three_mounts(), BASE);
        let gone = std::sync::atomic::AtomicBool::new(true);
        let stat = |mounts: &[Mount]| -> Vec<Option<FolderStatus>> {
            mounts
                .iter()
                .map(
                    |mount| match (mount.name.as_str(), gone.load(Ordering::SeqCst)) {
                        ("team", true) => Some(FolderStatus::Missing),
                        _ => Some(FolderStatus::Ok),
                    },
                )
                .collect()
        };
        let walks = Log::default();
        let build =
            |mount: &Mount, under: Option<&MountPath>, _: &Tags| -> io::Result<ScanResult> {
                walks.push(format!(
                    "{} {:?}",
                    mount.name.as_str(),
                    under.map(MountPath::as_str)
                ));
                Ok(walked(mount, under))
            };
        gate.refresh(None, &stat, &build).unwrap();
        assert_eq!(walks.take(), ["library None", "ops None"]);
        let failed = gate.refresh(Some(&lp("team/rules")), &stat, &build);
        assert_eq!(failed.unwrap_err(), "missing");
        assert_eq!(walks.take().len(), 2, "nothing walked");
        gone.store(false, Ordering::SeqCst);
        gate.refresh(Some(&lp("team/rules")), &stat, &build)
            .unwrap();
        assert_eq!(walks.take()[2..], ["team None"], "widened to the mount");
        gate.refresh(Some(&lp("team/rules")), &stat, &build)
            .unwrap();
        assert_eq!(
            walks.take()[3..],
            ["team Some(\"rules\")"],
            "walked once: only the folder"
        );
        assert!(gate.with_read(|state| state.seen(&MountName::parse("team").unwrap()).usable()));
    }

    /// A stat that finds every mount there but `team`.
    fn team_missing(mounts: &[Mount]) -> Vec<Option<FolderStatus>> {
        mounts
            .iter()
            .map(|mount| match mount.name.as_str() {
                "team" => Some(FolderStatus::Missing),
                _ => Some(FolderStatus::Ok),
            })
            .collect()
    }

    fn walk_empty(mount: &Mount, under: Option<&MountPath>, _: &Tags) -> io::Result<ScanResult> {
        Ok(walked(mount, under))
    }

    // A range refresh that joins a whole scan takes the failure of the mount its range lies in — reported at that
    //   mount's top, an ancestor of the range — and not another mount's.
    #[test]
    fn a_range_refresh_joining_a_whole_scan_keeps_the_failure_of_its_mount() {
        let (gate, log) = (
            Arc::new(Gate::new(three_mounts(), BASE)),
            Arc::new(Log::default()),
        );
        gate.refresh(None, &ok, &walk_empty).unwrap();
        let door = Arc::new(Door::default());
        let blocker = change_thread(&gate, &log, "blocker", Some(Arc::clone(&door)));
        gate.wait_for("blocker in", |line, _| line == [("change", true)]);
        let whole = {
            let gate = Arc::clone(&gate);
            thread::spawn(move || gate.refresh(None, &team_missing, &walk_empty))
        };
        gate.wait_for("the whole scan queued", |line, _| line.len() == 2);
        let joiner = {
            let gate = Arc::clone(&gate);
            thread::spawn(move || gate.refresh(Some(&lp("team/sub")), &ok, &walk_empty))
        };
        gate.wait_for("the range refresh joined it", |_, waiting| waiting == 1);
        door.open();
        join(blocker);
        let whole = join(whole).unwrap();
        let joined = join(joiner).unwrap();
        assert!(Arc::ptr_eq(&whole, &joined));
        assert_eq!(
            joined.share(Some(&lp("team/sub"))).errors,
            [(Some(lp("team")), "missing".to_owned())]
        );
        assert!(joined.share(Some(&lp("library/sub"))).errors.is_empty());
        assert_eq!(joined.share(None).errors.len(), 1);
    }

    // ADR-0400 decision 17: a stat taken before another scan recorded a newer observation of the mount does not
    //   overwrite it — the older scan goes by the newer state.
    #[test]
    fn a_stale_stat_does_not_overwrite_a_newer_observation_of_its_mount() {
        let gate = Arc::new(Gate::new(three_mounts(), BASE));
        gate.refresh(None, &ok, &walk_empty).unwrap();
        let walks = Arc::new(Log::default());
        let (stating, door) = (Arc::new(Door::default()), Arc::new(Door::default()));
        let stale = {
            let (gate, walks, stating, door) = (
                Arc::clone(&gate),
                Arc::clone(&walks),
                Arc::clone(&stating),
                Arc::clone(&door),
            );
            thread::spawn(move || {
                gate.refresh(
                    Some(&lp("team/rules")),
                    &|mounts| {
                        let statuses = team_missing(mounts);
                        stating.open();
                        door.wait();
                        statuses
                    },
                    &|mount, under, _| {
                        walks.push(format!("stale walks {:?}", under.map(MountPath::as_str)));
                        Ok(walked(mount, under))
                    },
                )
            })
        };
        stating.wait();
        // Meanwhile a later scan finds the mount there and walks it.
        gate.refresh(Some(&lp("team")), &ok, &walk_empty).unwrap();
        door.open();
        assert!(join(stale).is_ok(), "the newer observation decides");
        assert_eq!(walks.take(), ["stale walks Some(\"rules\")"]);
        assert_eq!(
            gate.with_read(|state| state.seen(&MountName::parse("team").unwrap())),
            MountSeen {
                state: MountState::Ok,
                walked: true
            }
        );
    }

    // ADR-0400 decision 17: a mount that does not stat `Ok` is recorded without a ticket; one that does is answered
    //   for the caller to walk.
    #[test]
    fn observing_records_a_mount_not_there_without_a_ticket_and_answers_the_others() {
        let (gate, log) = (
            Arc::new(Gate::new(three_mounts(), BASE)),
            Arc::new(Log::default()),
        );
        gate.refresh(None, &ok, &walk_empty).unwrap();
        let door = Arc::new(Door::default());
        let walking = scan_thread(
            &gate,
            &log,
            "slow",
            Some("library"),
            Some(Arc::clone(&door)),
        );
        gate.wait_for("a walk holds the line", |line, _| line == [("scan", true)]);
        let mounts: Vec<Mount> =
            gate.with_read(|state| state.mounts.iter().skip(1).cloned().collect());
        let back = gate.observe(mounts, &team_missing);
        assert_eq!(
            back.iter()
                .map(|mount| mount.name.as_str())
                .collect::<Vec<_>>(),
            ["ops"]
        );
        let team = gate.with_read(|state| state.seen(&MountName::parse("team").unwrap()));
        assert_eq!(
            team,
            MountSeen {
                state: MountState::Missing {
                    why: "missing".to_owned()
                },
                walked: true
            }
        );
        door.open();
        join(walking).unwrap();
    }

    /// A whole refresh whose walk waits on `door` and then panics — a pending first scan that ends without an apply
    ///   (`ENDED_UNANSWERED`); its thread answers whether it panicked.
    fn failing_whole_scan(gate: &Arc<Gate>, door: &Arc<Door>) -> thread::JoinHandle<bool> {
        let (gate, door) = (Arc::clone(gate), Arc::clone(door));
        thread::spawn(move || {
            catch_unwind(AssertUnwindSafe(|| {
                gate.refresh(None, &ok, &|_, _, _| {
                    door.wait();
                    panic!("a whole scan fails on purpose")
                })
            }))
            .is_err()
        })
    }

    // A waiter's two failed tries answer the failure: the first-scan entrance goes through once more after one waited
    //   first scan failed, and a second waited failure is its answer.
    #[test]
    fn a_reader_whose_two_waited_first_scans_failed_answers_the_failure() {
        let gate = gate();
        let log = Arc::new(Log::default());
        let (blocker_door, first_door, second_door) = (
            Arc::new(Door::default()),
            Arc::new(Door::default()),
            Arc::new(Door::default()),
        );
        let blocker = change_thread(&gate, &log, "blocker", Some(Arc::clone(&blocker_door)));
        gate.wait_for("blocker in", |line, _| line == [("change", true)]);
        let first = failing_whole_scan(&gate, &first_door);
        gate.wait_for("the first failing try queued", |line, _| line.len() == 2);
        let reader = {
            let gate = Arc::clone(&gate);
            thread::spawn(move || {
                gate.first_scan_join(&ok, &|_, _, _| panic!("the reader never walks"))
            })
        };
        gate.wait_for("the reader waits on it", |_, waiting| waiting == 1);
        blocker_door.open();
        join(blocker);
        // Drawn once the first walks: a whole refresh joins only a covering scan not yet walking.
        gate.wait_for("the first walks", |line, _| line == [("scan", true)]);
        let second = failing_whole_scan(&gate, &second_door);
        gate.wait_for("the second failing try queued", |line, _| line.len() == 2);
        first_door.open();
        assert!(join(first));
        gate.wait_for("the reader waits on the second", |line, waiting| {
            line == [("scan", true)] && waiting == 1
        });
        second_door.open();
        assert!(join(second));
        assert_eq!(
            join(reader),
            Err(ENDED_UNANSWERED.to_owned()),
            "the second waited failure is the answer"
        );
    }

    /// A stat that first registers `name` (folder `/<name>`) unless it is registered — as if a mount were added after
    ///   the scan read the mounts it stats — and then answers `Ok` for the mounts it was given.
    fn registering(
        gate: &Arc<Gate>,
        name: &'static str,
    ) -> impl Fn(&[Mount]) -> Vec<Option<FolderStatus>> {
        let gate = Arc::clone(gate);
        move |mounts| {
            let mut state = write(&gate.lib);
            if state.mounts.get(name).is_some() {
                return ok(mounts);
            }
            gate.write_mounts(&mut state, |state| {
                let mut all: Vec<Mount> = state.mounts.iter().cloned().collect();
                all.push(Mount {
                    name: MountName::parse(name).unwrap(),
                    folder: PathBuf::from(format!("/{name}")),
                });
                state.mounts = Mounts::new(all).unwrap();
            });
            ok(mounts)
        }
    }

    /// A walk that waits on `door`.
    fn held_walk(
        door: &Arc<Door>,
    ) -> impl Fn(&Mount, Option<&MountPath>, &Tags) -> io::Result<ScanResult> {
        let door = Arc::clone(door);
        move |mount, under, _| {
            door.wait();
            Ok(walked(mount, under))
        }
    }

    // Two waited whole scans that applied but left the first scan un-raised (each found a mount registered after its
    //   stats) are no failed tries: the waiter goes round again each time and answers its own scan, not a failure.
    #[test]
    fn a_reader_whose_two_waited_first_scans_applied_without_raising_answers_ok() {
        let gate = gate();
        let (first_door, second_door) = (Arc::new(Door::default()), Arc::new(Door::default()));
        let first = {
            let gate2 = Arc::clone(&gate);
            let (stat, walk) = (registering(&gate, "t1"), held_walk(&first_door));
            thread::spawn(move || gate2.first_scan_join(&stat, &walk))
        };
        gate.wait_for("the first walking", |line, _| line == [("scan", true)]);
        let reader = {
            let gate = Arc::clone(&gate);
            thread::spawn(move || gate.first_scan_join(&ok, &walk_empty))
        };
        gate.wait_for("the reader waits on it", |_, waiting| waiting == 1);
        let second = {
            let gate2 = Arc::clone(&gate);
            let (stat, walk) = (registering(&gate, "t2"), held_walk(&second_door));
            thread::spawn(move || gate2.refresh(None, &stat, &walk))
        };
        gate.wait_for("the second queued behind it", |line, _| {
            line == [("scan", true), ("scan", false)]
        });
        first_door.open();
        // The first's own caller goes round again too, so both wait on the second.
        gate.wait_for("the reader waits on the second", |line, waiting| {
            line == [("scan", true)] && waiting == 2
        });
        assert!(
            !lock(&gate.turns).first.raised,
            "the first applied un-raised"
        );
        second_door.open();
        join(second).unwrap();
        assert_eq!(
            join(reader),
            Ok(()),
            "un-raised applies are no failed tries"
        );
        join(first).unwrap();
        assert!(
            lock(&gate.turns).first.raised,
            "the reader's own scan raised it"
        );
    }

    // A first scan that applied but left the first scan un-raised (a mount was registered in its window) is not a
    //   failed try for its waiters: only a scan that ended without an apply counts, so one such apply and one failure
    //   send a waiter round again instead of answering the failure.
    #[test]
    fn a_first_scan_that_applied_without_raising_is_not_a_failed_try_for_its_waiters() {
        let gate = gate();
        let log = Arc::new(Log::default());
        let (door1, door2, door3) = (
            Arc::new(Door::default()),
            Arc::new(Door::default()),
            Arc::new(Door::default()),
        );
        let blocker1 = change_thread(&gate, &log, "blocker1", Some(Arc::clone(&door1)));
        gate.wait_for("blocker1 in", |line, _| line == [("change", true)]);
        let team = Mount {
            name: MountName::parse("team").unwrap(),
            folder: PathBuf::from("/team"),
        };
        let add = {
            let gate = Arc::clone(&gate);
            thread::spawn(move || {
                gate.change_mount(MountChange::Add(team), |mount| Ok(walked(mount, None)))
            })
        };
        gate.wait_for("the add queued", |line, _| line.len() == 2);
        let blocker2 = change_thread(&gate, &log, "blocker2", Some(Arc::clone(&door2)));
        gate.wait_for("blocker2 queued", |line, _| line.len() == 3);
        // Prepared before the add: its scan finds `team` unstated — applied, the first scan un-raised. Its walks wait
        //   on `door3`, so the failing try below is drawn while it walks.
        let walk = {
            let door3 = Arc::clone(&door3);
            move |mount: &Mount, under: Option<&MountPath>, tags: &Tags| {
                door3.wait();
                walk_empty(mount, under, tags)
            }
        };
        let starter = {
            let (gate, walk) = (Arc::clone(&gate), walk.clone());
            thread::spawn(move || gate.first_scan_join(&ok, &walk))
        };
        gate.wait_for("the starter's scan queued", |line, _| line.len() == 4);
        let waiter = {
            let gate = Arc::clone(&gate);
            thread::spawn(move || gate.first_scan_join(&ok, &walk))
        };
        gate.wait_for("the waiter waits on it", |_, waiting| waiting == 1);
        door1.open();
        join(blocker1);
        join(add).unwrap();
        door2.open();
        join(blocker2);
        gate.wait_for("the starter's scan walks", |line, _| {
            line == [("scan", true)]
        });
        // Then a pending first scan that fails, queued behind the un-raised one (drawn while it walks: one drawn while
        //   it is queued would join it instead of drawing its own).
        let fail_door = Arc::new(Door::default());
        let failing = failing_whole_scan(&gate, &fail_door);
        gate.wait_for("the failing scan queued", |line, _| line.len() == 2);
        fail_door.open();
        door3.open();
        assert!(join(failing));
        join(starter).unwrap();
        assert!(
            join(waiter).is_ok(),
            "an un-raised apply and one failure are one failed try, not two"
        );
        gate.first_scan_join(&|_| panic!("raised: no stat"), &|_, _, _| {
            panic!("raised: no walk")
        })
        .unwrap();
    }

    // ADR-0400 decision 17: every write of the mounts or of what the scans know of them refreshes the view scans
    //   prepare from, in the same write-lock hold.
    #[test]
    fn every_write_of_the_mounts_keeps_the_view_scans_prepare_from() {
        let gate = Arc::new(Gate::new(Mounts::default(), BASE));
        let view_matches = |gate: &Gate| {
            let state = read(&gate.lib);
            lock(&gate.turns).view == MountView::of(&state)
        };
        assert!(view_matches(&gate), "new");
        gate.publish_mount_unwalked(library_at("/lib")).unwrap();
        assert!(view_matches(&gate), "an unwalked publish");
        gate.refresh(None, &ok, &walk_empty).unwrap();
        assert!(view_matches(&gate), "a scan's apply");
        let team = Mount {
            name: MountName::parse("team").unwrap(),
            folder: PathBuf::from("/team"),
        };
        gate.change_mount(MountChange::Add(team.clone()), |mount| {
            Ok(walked(mount, None))
        })
        .unwrap();
        assert!(view_matches(&gate), "an added mount");
        let walked_ok = MountSeen {
            state: MountState::Ok,
            walked: true,
        };
        assert_eq!(gate.with_read(|state| state.seen(&team.name)), walked_ok);
        gate.observe(vec![team.clone()], &|mounts| {
            vec![Some(FolderStatus::Missing); mounts.len()]
        });
        relocate(&gate, "/new", |mount| Ok(walked(mount, None))).unwrap();
        assert!(view_matches(&gate), "a relocated mount");
        assert_eq!(
            gate.with_read(|state| state.seen(&MountName::library())),
            walked_ok,
            "relocated: its state is its new walk's"
        );
        gate.change_mount(MountChange::Remove(team.name.clone()), |_| {
            unreachable!("a removal walks nothing")
        })
        .unwrap();
        assert!(view_matches(&gate), "a removed mount");
        assert_eq!(
            gate.with_read(|state| (
                state.seen.get(&team.name.key()).is_none(),
                state.stamps.get(&team.name.key()).is_none()
            )),
            (true, true),
            "a removed mount's state and stamp are gone"
        );
        let mounts: Vec<Mount> = gate.with_read(|state| state.mounts.iter().cloned().collect());
        gate.observe(mounts, &|mounts| {
            vec![Some(FolderStatus::Missing); mounts.len()]
        });
        assert!(view_matches(&gate), "an observation");
        assert!(gate
            .refresh(Some(&lp("library/a")), &team_missing, &walk_empty)
            .is_ok());
        assert!(view_matches(&gate), "a range scan's apply");
        let failed = gate.refresh(
            Some(&lp("library/a")),
            &|mounts| vec![Some(FolderStatus::Missing); mounts.len()],
            &walk_empty,
        );
        assert_eq!(failed.unwrap_err(), "missing");
        assert!(view_matches(&gate), "a range scan in a mount not there");
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

    // ── the step-9b hold and the trash lock (ADR-0409 M1 · decision 14) ──────────────────────────────────────────

    #[test]
    fn a_held_change_raises_the_change_number_once_before_its_agents_pass_and_not_the_root() {
        let gate = gate();
        let before = gate.numbers();
        let seen = gate
            .change_held(None, false, |_, _, hold| {
                let at_start = gate.numbers().change;
                hold.raise();
                // What an `agent.new` judged before the hold compares inside its insert.
                let before_agents = gate.numbers().change;
                hold.raise();
                (Ok::<_, ()>((at_start, before_agents)), Vec::new())
            })
            .unwrap()
            .unwrap();
        assert_eq!(
            seen,
            (before.change, before.change + 1),
            "raised inside the hold"
        );
        let after = gate.numbers();
        assert_eq!(after.change, before.change + 1, "raised once");
        assert_eq!(
            after.root, before.root,
            "the root generation keeps its meaning"
        );

        gate.change_held(None, false, |_, _, _| ((), Vec::new()))
            .unwrap();
        assert_eq!(
            gate.numbers().change,
            before.change + 2,
            "a body that did not raise is raised after"
        );
        let refused: Result<(), ()> = gate
            .change_held(None, false, |_, _, _| (Err(()), Vec::new()))
            .unwrap();
        assert!(refused.is_err());
        assert_eq!(
            gate.numbers().change,
            before.change + 2,
            "a refusal that touched nothing raises nothing"
        );
    }

    #[test]
    fn a_held_change_with_a_stale_root_generation_runs_nothing_and_releases_the_trash_count() {
        let gate = gate();
        let called = std::cell::Cell::new(false);
        let answer = gate.change_held(Some(BASE + 1), true, |_, _, _| {
            called.set(true);
            ((), Vec::new())
        });
        assert_eq!(answer.unwrap_err(), BASE);
        assert!(!called.get());
        assert_eq!(gate.trash().waiting(), 0, "the refusal released the count");
        // An emptying is not held back by a count left raised.
        drop(gate.trash().take_for_emptying());
        assert!(gate
            .change_held(Some(BASE), false, |_, _, _| ((), Vec::new()))
            .is_ok());
    }

    // ADR-0409 decision 14 (gate part).
    #[test]
    fn a_trash_step_raises_the_count_before_its_ticket_and_waits_out_the_bundle_being_emptied() {
        let gate = gate();
        let log = Arc::new(Log::default());
        // A fake removal: one bundle of an emptying holds the trash lock.
        let bundle = gate.trash().take_for_emptying();
        let step = {
            let (gate, log) = (Arc::clone(&gate), Arc::clone(&log));
            thread::spawn(move || {
                gate.change_held(None, true, |_, _, hold| {
                    let held = hold.take_trash().expect("the step reserved");
                    log.push("step holds the trash lock");
                    drop(held);
                    ((), Vec::new())
                })
                .unwrap();
            })
        };
        gate.trash()
            .wait_until("the step raised the count", |view| {
                view.waiting == 1 && view.emptying
            });
        gate.wait_for("the waiting step holds no ticket", |line, _| {
            line.is_empty()
        });
        assert!(
            gate.lib().try_write().is_ok(),
            "the waiting step holds no library lock"
        );
        // The emptying's next bundle waits for the count to reach 0: the step goes first.
        let next_bundle = {
            let (gate, log) = (Arc::clone(&gate), Arc::clone(&log));
            thread::spawn(move || {
                let held = gate.trash().take_for_emptying();
                log.push("next bundle");
                drop(held);
            })
        };
        drop(bundle);
        join(step);
        join(next_bundle);
        // The reservation drops when `change_held` returns: "next bundle" comes after the step's hold.
        assert_eq!(log.take(), ["step holds the trash lock", "next bundle"]);
        assert_eq!(gate.trash().waiting(), 0);
    }

    #[test]
    fn emptying_starts_no_bundle_while_a_step_waits_before_its_ticket() {
        let gate = gate();
        let log = Arc::new(Log::default());
        let reserved = gate.trash().reserve();
        let emptying = {
            let (gate, log) = (Arc::clone(&gate), Arc::clone(&log));
            thread::spawn(move || {
                let held = gate.trash().take_for_emptying();
                log.push("bundle");
                drop(held);
            })
        };
        gate.trash()
            .wait_until("the emptying waits for the count", |view| {
                view.emptiers == 1
            });
        log.push("step");
        drop(reserved);
        join(emptying);
        assert_eq!(log.take(), ["step", "bundle"]);
    }

    #[test]
    fn panic_unwinding_releases_the_trash_count_the_trash_lock_and_the_ticket() {
        let gate = gate();
        let unwound = catch_unwind(AssertUnwindSafe(|| {
            gate.change_held::<()>(None, true, |_, _, hold| {
                let _held = hold.take_trash().expect("the step reserved");
                panic!("a step panics holding the trash lock");
            })
        }));
        assert!(unwound.is_err());
        assert_eq!(gate.trash().waiting(), 0);
        drop(gate.trash().take_for_emptying());
        gate.wait_for("the ticket was released", |line, _| line.is_empty());
        assert!(gate
            .change_held(None, false, |_, _, _| ((), Vec::new()))
            .is_ok());

        let early = catch_unwind(AssertUnwindSafe(|| {
            let _reserved = gate.trash().reserve();
            panic!("a step fails before its ticket");
        }));
        assert!(early.is_err());
        assert_eq!(gate.trash().waiting(), 0, "dropped on the earlier exit too");
    }

    #[test]
    fn a_second_take_of_the_trash_lock_in_one_hold_is_refused_not_a_self_wait() {
        let gate = gate();
        let second = gate
            .change_held(None, true, |_, _, hold| {
                let _held = hold.take_trash().expect("the step reserved");
                (Ok::<_, ()>(hold.take_trash().map(drop)), Vec::new())
            })
            .unwrap()
            .unwrap();
        assert_eq!(second, Err(TrashUnavailable::AlreadyHeld));
        drop(gate.trash().take_for_emptying());
    }

    #[test]
    fn a_hold_that_raised_no_count_never_waits_on_the_trash_lock() {
        let gate = gate();
        let before = gate.numbers().change;
        let _bundle = gate.trash().take_for_emptying();
        let answer = gate
            .change_held(None, false, |_, _, hold| {
                (hold.take_trash().map(drop), Vec::new())
            })
            .unwrap();
        assert_eq!(answer, Err(TrashUnavailable::NotReserved));
        assert_eq!(gate.numbers().change, before, "a refusal raises nothing");
    }
}
