//! What a mount's `.trash` holds and how Engram touches it (ADR-0409 — ADR-0400 decision 23, ADR-0320 as amended): the
//! trash lock and its waiting count, the trash-folder check, bundle ids, the bundle record, the bundle shell and the
//! `.git/info/exclude` line. The file operations and emptying stand on these; nothing here takes the library lock.
//!
//! ## Rules (invariants)
//!
//! - **Never through a link** (ADR-0365 · ADR-0409 decisions 5, 6 and 9): `.trash` is checked from the mount folder down
//!   without following ([`check_trash_folder`]) — a link or a non-folder there makes that mount's trash unusable
//!   ([`TRASH_UNUSABLE`]); every name from the bundle down to a row is checked without following
//!   ([`LibraryStore::check_names`]); folders are made with `create_dir`, never `create_dir_all`; pruning removes only empty
//!   real folders and never walks into a link. A name swapped for a link between a check and the call is not caught
//!   (local write access is trusted — ADR-0409).
//! - **The trash lock** (ADR-0324 decision 9 as ADR-0409 decisions 14 and 15 amend it): one [`TrashLock`] for every
//!   mount's `.trash`, taken after the library lock and before the turn mutex. A step that may touch `.trash` raises the waiting count just
//!   before its ticket and waits, holding nothing, until no bundle is being emptied ([`TrashLock::reserve`]); only that
//!   reservation takes the lock inside the hold ([`TrashReservation::take`]), so a hold without one never waits on it —
//!   it answers [`TRASH_NOT_RESERVED`]. Emptying takes the lock per bundle only while the count is 0
//!   ([`TrashLock::take_for_emptying`]), so it starts no bundle while a step waits and a step finds the lock free. The
//!   reservation lowers the count when dropped — after the hold or on any earlier exit, panic unwinding included; a held
//!   lock cannot outlive its reservation.
//! - **What writes inside `.trash` takes a [`TrashHeld`]**: [`ensure_trash_folder`], [`make_bundle`],
//!   [`publish_record`] and [`prune_shell`] run only under the trash lock, and so do the trash step's
//!   [`LibraryStore::trash`] moves and a restore's rename out of a bundle.
//! - **A bundle** is a direct child of `.trash` that is a real folder and whose name parses exactly as [`BundleId`];
//!   anything else there is foreign and never touched. One id per operation; a taken id draws a new one.
//! - **The record** ([`RECORD_NAME`], ADR-0409 decision 8): one dot-named JSON file per bundle naming its roots, written
//!   create-new to a base temp name and published by a no-replace rename, never rewritten. Roots are read filtered by
//!   existence — absent only when a no-follow check says a name on its path is missing or not a folder; a record that does not parse, or names a root that is
//!   no valid library path, is [`RECORD_UNREADABLE`] and the bundle's files are its rows ([`fallback_rows`]).
//! - **The shell** ([`prune_shell`]): when a root goes, the folders between it and the bundle that are now empty go,
//!   bottom-up; the record (with any record temp left by a crash) and then the bundle folder go only when nothing else is
//!   left in the bundle. Surviving content is never removed.
//! - **The exclude line** ([`exclude_trash`], ADR-0400 decision 24 · ADR-0409 M12): an anchored, escaped
//!   `/<mount path>/.trash/` line appended to the repository's `info/exclude` when absent, under one leaf mutex, through a
//!   create-new temp; skipped with a reason, never a failure of the step. `.git` and its `gitdir:` / `commondir` targets
//!   must be real folders (a linked `.git` is skipped); `info` and `exclude` are checked without following. It runs
//!   with std `fs` on a job of the service's spawner, not through `BoundedIo`: a share that stops answering holds that
//!   job and the exclude mutex (later exclude jobs queue behind it), never a library lock or the step.
// ADR-0320
// ADR-0324
// ADR-0365
// ADR-0400 decisions 23 and 24
// ADR-0409

use std::collections::hash_map::RandomState;
use std::ffi::OsStr;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use engram_dashboard_agent::launch::escape_controls;
use engram_dashboard_base::file::{is_temp_name, temp_path, temp_target};
use engram_dashboard_base::sync::lock;
use serde::{Deserialize, Serialize};

use crate::mounts::MountPath;
use crate::store::{EntryKind, LibraryStore, NameWalk, TrashBundle, OS_HOOKS, TRASH_DIR};

/// The code of a trash step in a mount whose `.trash` is a link or not a folder (`args` = `{mount}`): the item stays.
// ADR-0409 decision 5
pub const TRASH_UNUSABLE: &str = "trash-unusable";

/// The code of a hold that meets a trash step without having raised the waiting count before its ticket: the entry or
/// staging folder stays where it is and its path is answered.
// ADR-0409 decision 14
pub const TRASH_NOT_RESERVED: &str = "trash-not-reserved";

/// The bundle code of a record that exists but does not parse or names a root that is no valid library path.
// ADR-0409 decision 8
pub const RECORD_UNREADABLE: &str = "record-unreadable";

/// The record's name at a bundle's top — a dot name, so it never collides with a root (roots are library paths).
// ADR-0409 decision 8
pub const RECORD_NAME: &str = ".engram-bundle.json";

/// The largest record read.
const RECORD_CAP: u64 = 16 * 1024 * 1024;

/// How many ids [`make_bundle`] (and temp names [`publish_record`]) draw before giving up on taken names.
const DRAWS: usize = 16;

// ── the trash lock ───────────────────────────────────────────────────────────────────────────────────────────────

/// The one trash lock over every mount's `.trash`, with its waiting count (module rules).
#[derive(Default)]
pub struct TrashLock {
    state: Mutex<TrashState>,
    moved: Condvar,
}

#[derive(Default)]
struct TrashState {
    /// Steps that raised the count and have not lowered it.
    waiting: usize,
    holder: Option<Holder>,
    /// Emptiers waiting in [`TrashLock::take_for_emptying`] (tests observe one).
    #[cfg(test)]
    emptiers: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Holder {
    Step,
    Emptying,
}

impl TrashLock {
    /// Raises the waiting count, then waits — call it holding no lock, after any probe or copy and just before the
    /// ticket — until no bundle is being emptied. Emptying starts no bundle while the count is above 0, so the lock is
    /// free when the hold takes it. Dropping the answer lowers the count.
    // ADR-0409 decision 14
    pub fn reserve(&self) -> TrashReservation<'_> {
        let mut state = lock(&self.state);
        state.waiting += 1;
        self.moved.notify_all();
        while state.holder == Some(Holder::Emptying) {
            state = self.wait(state);
        }
        drop(state);
        TrashReservation { trash: self }
    }

    /// Takes the lock for one bundle of an emptying (or its brief listing): waits, holding no other lock, until the
    /// waiting count is 0 and nobody holds it. Release it after each bundle so a waiting step gets its turn.
    // ADR-0324 decision 9
    pub fn take_for_emptying(&self) -> TrashHeld<'_> {
        let mut state = lock(&self.state);
        while state.waiting > 0 || state.holder.is_some() {
            #[cfg(test)]
            {
                state.emptiers += 1;
                self.moved.notify_all();
            }
            state = self.wait(state);
            #[cfg(test)]
            {
                state.emptiers -= 1;
            }
        }
        state.holder = Some(Holder::Emptying);
        TrashHeld { trash: self }
    }

    fn wait<'a>(&self, state: MutexGuard<'a, TrashState>) -> MutexGuard<'a, TrashState> {
        self.moved
            .wait(state)
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// A raised waiting count (module rules); dropping it lowers the count and wakes every waiter.
pub struct TrashReservation<'t> {
    trash: &'t TrashLock,
}

impl TrashReservation<'_> {
    /// Takes the trash lock inside the hold (library lock → trash lock), once per hold. It waits only while an emptying
    /// holds it, which the waiting count rules out. `Err(AlreadyHeld)` = a step holds it: under the library write lock
    /// that can only be this hold, which would wait on itself.
    pub fn take(&self) -> Result<TrashHeld<'_>, TrashUnavailable> {
        let mut state = lock(&self.trash.state);
        loop {
            match state.holder {
                None => break,
                Some(Holder::Step) => return Err(TrashUnavailable::AlreadyHeld),
                Some(Holder::Emptying) => state = self.trash.wait(state),
            }
        }
        state.holder = Some(Holder::Step);
        Ok(TrashHeld { trash: self.trash })
    }
}

impl Drop for TrashReservation<'_> {
    fn drop(&mut self) {
        lock(&self.trash.state).waiting -= 1;
        self.trash.moved.notify_all();
    }
}

/// The trash lock held; dropping it releases the lock and wakes every waiter.
pub struct TrashHeld<'t> {
    trash: &'t TrashLock,
}

impl Drop for TrashHeld<'_> {
    fn drop(&mut self) {
        lock(&self.trash.state).holder = None;
        self.trash.moved.notify_all();
    }
}

/// Why a hold did not get the trash lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrashUnavailable {
    /// The step did not raise the waiting count before its ticket ([`TRASH_NOT_RESERVED`]): it never waits on the lock.
    NotReserved,
    /// This hold already holds it (it is taken once per hold).
    AlreadyHeld,
}

/// The trash lock's state as a test sees it.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct TrashView {
    /// The waiting count.
    pub(crate) waiting: usize,
    /// Held by an emptying.
    pub(crate) emptying: bool,
    /// Emptiers waiting for the count to reach 0.
    pub(crate) emptiers: usize,
}

/// Test views of the trash lock.
#[cfg(test)]
impl TrashLock {
    /// Waits (at most 10 s, then fails) until `ready` holds over the lock's state.
    pub(crate) fn wait_until(&self, what: &str, ready: impl Fn(&TrashView) -> bool) {
        use std::time::Instant;
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut state = lock(&self.state);
        loop {
            let view = TrashView {
                waiting: state.waiting,
                emptying: state.holder == Some(Holder::Emptying),
                emptiers: state.emptiers,
            };
            if ready(&view) {
                return;
            }
            let now = Instant::now();
            assert!(now < deadline, "the trash lock never reached: {what}");
            state = engram_dashboard_base::sync::wait_timeout(&self.moved, state, deadline - now).0;
        }
    }

    /// The waiting count now.
    pub(crate) fn waiting(&self) -> usize {
        lock(&self.state).waiting
    }
}

// ── bundle ids ───────────────────────────────────────────────────────────────────────────────────────────────────

/// A bundle id: `<UTC basic time, to the second>-<6 lowercase hex>`, e.g. `20261123T081500Z-a1b2c3` — the parse rule of a
/// bundle (ADR-0409 decision 6); the time is when the operation deleted.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BundleId {
    text: String,
    secs: u64,
}

impl BundleId {
    /// A fresh id for an operation that runs at `now` (a clock before 1970 counts as 1970), with a random short id.
    pub fn new(now: SystemTime) -> Self {
        let secs = now
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        Self::at(secs, &format!("{:06x}", short_id()))
    }

    /// `name` as a bundle id when it is exactly the form [`BundleId::new`] writes (a real date and time), else `None` —
    /// a foreign name.
    pub fn parse(name: &str) -> Option<Self> {
        let bytes = name.as_bytes();
        let digits = |from: usize, to: usize| bytes[from..to].iter().all(u8::is_ascii_digit);
        let shaped = bytes.len() == 23
            && digits(0, 8)
            && bytes[8] == b'T'
            && digits(9, 15)
            && bytes[15] == b'Z'
            && bytes[16] == b'-'
            && bytes[17..]
                .iter()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c));
        if !shaped {
            return None;
        }
        let number = |from: usize, to: usize| -> u32 { name[from..to].parse().expect("digits") };
        let (month, day) = (number(4, 6), number(6, 8));
        let (hour, minute, second) = (number(9, 11), number(11, 13), number(13, 15));
        if !(1..=12).contains(&month)
            || !(1..=31).contains(&day)
            || hour > 23
            || minute > 59
            || second > 59
        {
            return None;
        }
        let days = u64::try_from(days_from_civil(i64::from(number(0, 4)), month, day)).ok()?;
        let secs = days * 86_400 + u64::from(hour * 3_600 + minute * 60 + second);
        // A day past its month's end (`0230`) reads back as another date.
        let id = Self::at(secs, &name[17..]);
        (id.text == name).then_some(id)
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// When the operation that made it deleted (to the second).
    pub fn deleted_at(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(self.secs)
    }

    /// The store's name for this bundle folder.
    pub fn bundle(&self) -> TrashBundle {
        TrashBundle(self.text.clone())
    }

    fn at(secs: u64, short: &str) -> Self {
        let days = i64::try_from(secs / 86_400).expect("a u64 of days fits");
        let (year, month, day) = civil_from_days(days);
        let rest = secs % 86_400;
        let text = format!(
            "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z-{short}",
            rest / 3_600,
            rest / 60 % 60,
            rest % 60
        );
        BundleId { text, secs }
    }
}

/// 24 random bits: std's per-process random hasher keys over a counter and the clock.
fn short_id() -> u32 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(NEXT.fetch_add(1, Ordering::Relaxed));
    hasher.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos()),
    );
    (hasher.finish() & 0xff_ffff) as u32
}

/// The proleptic Gregorian date of day `days` since 1970-01-01 (H. Hinnant's civil-from-days).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Days since 1970-01-01 of a proleptic Gregorian date (the inverse of [`civil_from_days`]).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let month = i64::from(month);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// ── paths ────────────────────────────────────────────────────────────────────────────────────────────────────────

/// The bundle folder's path inside the mount: `.trash/<id>`.
pub fn bundle_path(id: &BundleId) -> MountPath {
    MountPath::parse(&format!("{TRASH_DIR}/{}", id.as_str()))
        .expect("the trash name and a bundle id are valid names")
}

/// A row's path inside the mount: `.trash/<id>/<row>` — the row keeps the item's path in its mount.
pub fn row_path(id: &BundleId, row: &MountPath) -> MountPath {
    MountPath::parse(&format!("{TRASH_DIR}/{}/{}", id.as_str(), row.as_str()))
        .expect("the trash name, a bundle id and a mount path are valid names")
}

/// Whether `name`, at a bundle's top, is the record or a record temp (a base temp whose whole target is
/// [`RECORD_NAME`] — Engram's own, left by a crash between the temp write and the publication).
pub fn is_record_name(name: &OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| name == RECORD_NAME || temp_target(name) == Some(RECORD_NAME))
}

// ── the trash folder ─────────────────────────────────────────────────────────────────────────────────────────────

/// Why a mount's trash cannot be used ([`TRASH_UNUSABLE`]): `.trash` is a link or not a folder, or its stat failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashUnusable {
    pub why: String,
}

/// `.trash` of the mount at `folder`, checked without following (the mount folder itself may be a link — ADR-0365):
/// `Ok(true)` = a real folder, `Ok(false)` = absent.
// ADR-0409 decision 5
pub fn check_trash_folder(store: &dyn LibraryStore, folder: &Path) -> Result<bool, TrashUnusable> {
    match store.stat(&folder.join(TRASH_DIR)) {
        Ok(EntryKind::Folder) => Ok(true),
        Ok(EntryKind::Link) => Err(TrashUnusable {
            why: format!("{TRASH_DIR} is a link"),
        }),
        Ok(EntryKind::File) => Err(TrashUnusable {
            why: format!("{TRASH_DIR} is not a folder"),
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(TrashUnusable { why: e.to_string() }),
    }
}

/// [`check_trash_folder`] for a trash step: an absent `.trash` is made with `create_dir` (never through a link).
pub fn ensure_trash_folder(
    store: &dyn LibraryStore,
    folder: &Path,
    _held: &TrashHeld<'_>,
) -> Result<(), TrashUnusable> {
    if check_trash_folder(store, folder)? {
        return Ok(());
    }
    match store.create_dir(&folder.join(TRASH_DIR)) {
        Ok(()) => Ok(()),
        // Made meanwhile: judged again as found.
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            if check_trash_folder(store, folder)? {
                Ok(())
            } else {
                Err(TrashUnusable { why: e.to_string() })
            }
        }
        Err(e) => Err(TrashUnusable { why: e.to_string() }),
    }
}

/// Makes the bundle folder `.trash/<wanted>` with `create_dir` and answers its id; when that name is taken, another id
/// is drawn at the same time (the operation keeps `wanted` for its other mounts). `.trash` must be there ([`ensure_trash_folder`]).
// ADR-0409 decision 7
pub fn make_bundle(
    store: &dyn LibraryStore,
    folder: &Path,
    wanted: &BundleId,
    _held: &TrashHeld<'_>,
) -> io::Result<BundleId> {
    let mut id = wanted.clone();
    for _ in 0..DRAWS {
        match store.create_dir(&bundle_path(&id).to_path(folder)) {
            Ok(()) => return Ok(id),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                id = BundleId::new(wanted.deleted_at());
            }
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "every bundle id drawn was taken",
    ))
}

// ── the record ───────────────────────────────────────────────────────────────────────────────────────────────────

/// One root a bundle's record names: where it sits inside the bundle (its path in its mount).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordRoot {
    pub path: MountPath,
    /// A staging folder or file (a failed or partial copy, under a base temp name) and the path it was meant for —
    /// shown, never restorable.
    pub staging: Option<MountPath>,
}

#[derive(Serialize, Deserialize)]
struct RecordFile {
    roots: Vec<RootLine>,
}

#[derive(Serialize, Deserialize)]
struct RootLine {
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    staging: Option<StagingLine>,
}

#[derive(Serialize, Deserialize)]
struct StagingLine {
    intended: String,
}

/// Writes the record of bundle `id` naming `roots`: create-new at a fresh base temp name in the bundle (a taken temp
/// name draws another), then published at [`RECORD_NAME`] by the store's no-replace rename — never a partial record
/// under the real name, never a rewrite. When the publication (the rename) fails the temp is removed and the error
/// answered; a write failing after the temp was made leaves it (the store removes no temp), and the shell prunes it. The
/// bundle's files then read as fallback rows. Call it inside the hold, after that mount's moves of the operation.
// ADR-0409 decision 8
pub fn publish_record(
    store: &dyn LibraryStore,
    folder: &Path,
    id: &BundleId,
    roots: &[RecordRoot],
    _held: &TrashHeld<'_>,
) -> io::Result<()> {
    check_bundle(store, folder, id)?;
    let file = RecordFile {
        roots: roots
            .iter()
            .map(|root| RootLine {
                path: root.path.as_str().to_owned(),
                staging: root.staging.as_ref().map(|intended| StagingLine {
                    intended: intended.as_str().to_owned(),
                }),
            })
            .collect(),
    };
    let bytes = serde_json::to_vec_pretty(&file).map_err(io::Error::other)?;
    let record = bundle_path(id).to_path(folder).join(RECORD_NAME);
    let mut temp = None;
    for _ in 0..DRAWS {
        let name = temp_path(&record)?;
        match store.create_new(&name, &bytes) {
            Ok(()) => {
                temp = Some(name);
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    let temp = temp.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "every record temp name drawn was taken",
        )
    })?;
    if let Err(e) = store.rename(&temp, &record) {
        // Engram's own temp in its own bundle; one a crash leaves goes with the shell.
        if let Err(left) = store.remove_file(&temp) {
            tracing::debug!(
                path = %escape_controls(&temp.to_string_lossy()),
                "trash record: temp not removed: {left}"
            );
        }
        return Err(e);
    }
    Ok(())
}

/// `.trash/<id>` checked from the mount folder down without following: `Ok` = both are real folders; `NotFound` = one
/// is missing; `NotADirectory` = one is a link or a file; any other stat error as is.
// ADR-0409 decisions 5 and 6
fn check_bundle(store: &dyn LibraryStore, folder: &Path, id: &BundleId) -> io::Result<()> {
    let not_a_folder = |at: &MountPath, what: &str| {
        Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("{} is {what}", at.as_str()),
        ))
    };
    let bundle = bundle_path(id);
    match store.check_names(folder, &bundle)? {
        NameWalk::Present(EntryKind::Folder) => Ok(()),
        NameWalk::Present(EntryKind::Link) => not_a_folder(&bundle, "a link"),
        NameWalk::Present(EntryKind::File) => not_a_folder(&bundle, "not a folder"),
        NameWalk::Link(at) => not_a_folder(&at, "a link"),
        NameWalk::NotAFolder(at) => not_a_folder(&at, "not a folder"),
        NameWalk::Missing(at) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is missing", at.as_str()),
        )),
    }
}

/// What a bundle's record says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordRead {
    /// No record (a crash between the moves and the publication, or a failed publication): the bundle's files are its
    /// rows ([`fallback_rows`]).
    Missing,
    /// It exists but cannot be read, does not parse or names a root that is no valid library path
    /// ([`RECORD_UNREADABLE`]): the bundle's files are its rows too.
    Unreadable { why: String },
    /// The roots it names that still exist (a restored or removed root drops out), in record order.
    Roots(Vec<RootRow>),
    /// `.trash` or the bundle folder is missing, a link or not a folder: nothing was read through it (a foreign or
    /// unusable place — ADR-0409 decisions 5 and 6).
    NotABundle { why: String },
}

/// One root of a readable record that is still there (or whose presence cannot be told).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootRow {
    pub root: RecordRoot,
    pub state: RootState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootState {
    /// There; a link row is moved or removed as a link.
    Present(EntryKind),
    /// A link lies between the bundle and the row (or above it): the row is left untouched (`remaining
    /// {path-through-link}`).
    ThroughLink,
    /// A stat on the way failed other than `NotFound`: kept, never read as absent (`remaining {code}`).
    Failed(String),
}

/// Reads bundle `id`'s record and its roots filtered by existence: a root is dropped only when a no-follow check of
/// every name from the mount folder down says one is absent (`NotFound`), or a file stands where a folder of its path
/// would be. `.trash` and the bundle folder are checked without following first. Takes no lock (a view reads it holding
/// none).
// ADR-0409 decisions 6 and 8
pub fn read_record(store: &dyn LibraryStore, folder: &Path, id: &BundleId) -> RecordRead {
    if let Err(e) = check_bundle(store, folder, id) {
        return RecordRead::NotABundle { why: e.to_string() };
    }
    let record = bundle_path(id).to_path(folder).join(RECORD_NAME);
    match store.stat(&record) {
        Ok(EntryKind::File) => {}
        Ok(kind) => {
            return RecordRead::Unreadable {
                why: format!("the record is a {kind:?}, not a file"),
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => return RecordRead::Missing,
        Err(e) => return RecordRead::Unreadable { why: e.to_string() },
    }
    let bytes = match store.read(&record, RECORD_CAP) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return RecordRead::Missing,
        Err(e) => return RecordRead::Unreadable { why: e.to_string() },
    };
    let roots = match parse_record(&bytes) {
        Ok(roots) => roots,
        Err(why) => return RecordRead::Unreadable { why },
    };
    let rows = roots
        .into_iter()
        .filter_map(|root| {
            let state = match store.check_names(folder, &row_path(id, &root.path)) {
                Ok(NameWalk::Present(kind)) => RootState::Present(kind),
                Ok(NameWalk::Missing(_) | NameWalk::NotAFolder(_)) => return None,
                Ok(NameWalk::Link(_)) => RootState::ThroughLink,
                Err(e) => RootState::Failed(e.to_string()),
            };
            Some(RootRow { root, state })
        })
        .collect();
    RecordRead::Roots(rows)
}

fn parse_record(bytes: &[u8]) -> Result<Vec<RecordRoot>, String> {
    let file: RecordFile = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let library_path = |text: &str| {
        // A library path never has a dot name; a staging name is a base temp name, never a dot name either.
        MountPath::parse(text)
            .ok()
            .filter(|path| !path.as_str().split('/').any(|name| name.starts_with('.')))
            .ok_or_else(|| format!("{text:?} is not a library path"))
    };
    file.roots
        .iter()
        .map(|line| {
            Ok(RecordRoot {
                path: library_path(&line.path)?,
                staging: line
                    .staging
                    .as_ref()
                    .map(|staging| library_path(&staging.intended))
                    .transpose()?,
            })
        })
        .collect()
}

/// One row of a bundle read without its record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FallbackRow {
    pub path: MountPath,
    pub kind: EntryKind,
    /// A name of its path is in base's temp shape: staging, never restorable.
    pub staging: bool,
}

/// The rows of bundle `id` when its record is missing or unreadable: every file and link found walking the bundle
/// without following (a link is a row, never entered), skipping the record and its temps at the bundle top; sorted by
/// path. A name that cannot be a library path is left out (logged). `.trash` and the bundle folder are checked without
/// following first: a missing one is `NotFound`, a link or a file `NotADirectory` — nothing listed through it.
// ADR-0409 decisions 6 and 8
pub fn fallback_rows(
    store: &dyn LibraryStore,
    folder: &Path,
    id: &BundleId,
) -> io::Result<Vec<FallbackRow>> {
    check_bundle(store, folder, id)?;
    let mut rows = Vec::new();
    let mut folders: Vec<Option<MountPath>> = vec![None];
    while let Some(at) = folders.pop() {
        let disk = match &at {
            None => bundle_path(id).to_path(folder),
            Some(at) => row_path(id, at).to_path(folder),
        };
        for (name, kind) in store.list(&disk)? {
            if at.is_none() && is_record_name(&name) {
                continue;
            }
            let spelled = match &at {
                None => name.to_str().map(str::to_owned),
                Some(at) => name.to_str().map(|name| format!("{}/{name}", at.as_str())),
            };
            let Some(path) = spelled
                .as_deref()
                .and_then(|text| MountPath::parse(text).ok())
            else {
                tracing::warn!(
                    ?name,
                    bundle = id.as_str(),
                    "trash: left out a name that cannot be a library path"
                );
                continue;
            };
            match kind {
                EntryKind::Folder => folders.push(Some(path)),
                EntryKind::File | EntryKind::Link => rows.push(FallbackRow {
                    staging: path.as_str().split('/').any(is_temp_name),
                    path,
                    kind,
                }),
            }
        }
    }
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(rows)
}

// ── the shell ────────────────────────────────────────────────────────────────────────────────────────────────────

/// Prunes bundle `id` after its root `gone` went (`None` = no root: a bundle that received none, or a crash bundle):
/// the folders between `gone` and the bundle that are now empty, deepest first; then, when nothing but the record and
/// record temps is left at the top, those (the record last) and the empty bundle folder. Stops at the first folder that
/// holds anything, or that is a link or a file; never removes surviving content. Answers whether the bundle folder is
/// gone (`false` too when `.trash` itself is missing, though the bundle is then gone as well). A bundle that is not a
/// real folder is left alone.
// ADR-0409 decision 8
pub fn prune_shell(
    store: &dyn LibraryStore,
    folder: &Path,
    id: &BundleId,
    gone: Option<&MountPath>,
    _held: &TrashHeld<'_>,
) -> io::Result<bool> {
    match store.check_names(folder, &bundle_path(id))? {
        NameWalk::Present(EntryKind::Folder) => {}
        NameWalk::Missing(at) if at == bundle_path(id) => return Ok(true),
        _ => return Ok(false),
    }
    let mut at = gone.and_then(MountPath::parent);
    while let Some(dir) = at {
        let rel = row_path(id, &dir);
        match store.check_names(folder, &rel)? {
            NameWalk::Present(EntryKind::Folder) => {}
            // Gone already; the folder above may be empty now.
            NameWalk::Missing(_) => {
                at = dir.parent();
                continue;
            }
            // A link or a file on the way: never pruned through.
            _ => return Ok(false),
        }
        let disk = rel.to_path(folder);
        if !store.list(&disk)?.is_empty() {
            return Ok(false);
        }
        store.remove_dir(&disk)?;
        at = dir.parent();
    }
    let top = bundle_path(id).to_path(folder);
    let mut left = store.list(&top)?;
    if left
        .iter()
        .any(|(name, kind)| *kind != EntryKind::File || !is_record_name(name))
    {
        return Ok(false);
    }
    // The record last: while it stays, the bundle still reads by its record.
    left.sort_by_key(|(name, _)| name == RECORD_NAME);
    for (name, _) in left {
        store.remove_file(&top.join(name))?;
    }
    store.remove_dir(&top)?;
    Ok(true)
}

// ── the exclude line ─────────────────────────────────────────────────────────────────────────────────────────────

/// What [`exclude_trash`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Excluded {
    /// The line was appended to `file`.
    Added { file: PathBuf },
    /// `file` already holds the line.
    Present { file: PathBuf },
    /// No `.git` folder or file above the mount folder.
    NoRepository,
    /// Nothing written, and why (a link, a read failure other than `NotFound`, an unusable `.git` file, a failed write).
    Skipped { why: String },
}

/// Makes git ignore the mount's `.trash` (ADR-0400 decision 24 · ADR-0409 M12): walks up from `mount_folder` to the
/// nearest `.git` folder or file (a `gitdir:` file — a worktree or a submodule — leads to its git folder, and a
/// `commondir` there to the common one), then appends `/<mount path from the repository top>/.trash/`, anchored and with
/// gitignore's metacharacters escaped, to `info/exclude` when no line equals it — the file's bytes kept, a newline
/// added first when it does not end with one, the file replaced through a create-new temp beside it. `serial` (a leaf
/// mutex, taken holding no other lock) is held from the read to the write. Skipped when the common git folder is not a
/// real folder holding `HEAD` (a stale `gitdir:`), when the mount path below the repository top holds a control
/// character, when `.git` or the common git folder is a link, when `info` or the exclude file is a link, or when a read fails other than `NotFound`; only `info` itself
/// is made (one level). Never an error: call it after the hold, holding no library lock, and log a skip.
// ADR-0400 decision 24
pub fn exclude_trash(mount_folder: &Path, serial: &Mutex<()>) -> Excluded {
    let skip = |why: String| Excluded::Skipped { why };
    let (top, git_dir) = match find_repository(mount_folder) {
        Ok(Some(found)) => found,
        Ok(None) => return Excluded::NoRepository,
        Err(why) => return skip(why),
    };
    let common = match fs::read_to_string(git_dir.join("commondir")) {
        Ok(text) => git_dir.join(text.trim_end_matches(['\r', '\n'])),
        Err(e) if e.kind() == io::ErrorKind::NotFound => git_dir,
        Err(e) => return skip(format!("commondir not read: {e}")),
    };
    // A stale `gitdir:` or a folder merely named `.git` is no repository: nothing is made for it.
    if !is_git_folder(&common) {
        return skip(format!("{} is not a git folder", common.display()));
    }
    let line = match exclude_line(&top, mount_folder) {
        Ok(line) => line,
        Err(why) => return skip(why),
    };
    let info = common.join("info");
    let file = info.join("exclude");
    let _serial = lock(serial);
    for path in [&info, &file] {
        match fs::symlink_metadata(path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return skip(format!("{} is a link", path.display()))
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return skip(format!("{} not read: {e}", path.display())),
        }
    }
    if let Err(e) = fs::create_dir(&info) {
        if e.kind() != io::ErrorKind::AlreadyExists {
            return skip(format!("{} not made: {e}", info.display()));
        }
    }
    let mut bytes = match fs::read(&file) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return skip(format!("{} not read: {e}", file.display())),
    };
    let present = bytes
        .split(|&b| b == b'\n')
        .any(|have| have.strip_suffix(b"\r").unwrap_or(have) == line.as_bytes());
    if present {
        return Excluded::Present { file };
    }
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(line.as_bytes());
    bytes.push(b'\n');
    match replace_file(&file, &bytes) {
        Ok(()) => Excluded::Added { file },
        Err(e) => skip(format!("{} not written: {e}", file.display())),
    }
}

/// A real folder holding a `HEAD` file — what git's common folder holds.
fn is_git_folder(path: &Path) -> bool {
    let folder = fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir());
    folder && fs::symlink_metadata(path.join("HEAD")).is_ok_and(|meta| meta.is_file())
}

/// Replaces `file` with `bytes`: a fresh base temp name beside it made create-new (a taken name — a file or a link
/// planted there — draws another, never opened), filled and synced, then renamed over `file`. Not base
/// `write_atomic`, whose temp open follows a link already at the temp name.
fn replace_file(file: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut made = None;
    for _ in 0..DRAWS {
        let temp = temp_path(file)?;
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(handle) => {
                made = Some((temp, handle));
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    let (temp, mut handle) = made.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "every temp name drawn was taken",
        )
    })?;
    let written = handle
        .write_all(bytes)
        .and_then(|()| handle.sync_all())
        .and_then(|()| {
            drop(handle);
            (OS_HOOKS.retry)(&mut || fs::rename(&temp, file))
        });
    if let Err(e) = written {
        // The temp this call made, beside the exclude file.
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    if let Some(folder) = file.parent() {
        let _ = (OS_HOOKS.sync_dir)(folder);
    }
    Ok(())
}

/// The nearest repository at or above `folder`: (its working-tree top, its git folder). `Err` = a `.git` that cannot be
/// read or understood.
fn find_repository(folder: &Path) -> Result<Option<(PathBuf, PathBuf)>, String> {
    for dir in folder.ancestors() {
        let dot_git = dir.join(".git");
        match fs::metadata(&dot_git) {
            Ok(meta) if meta.is_dir() => return Ok(Some((dir.to_path_buf(), dot_git))),
            Ok(_) => {
                let text = fs::read_to_string(&dot_git)
                    .map_err(|e| format!("{} not read: {e}", dot_git.display()))?;
                let target = text
                    .lines()
                    .find_map(|line| line.strip_prefix("gitdir:"))
                    .map(str::trim)
                    .filter(|target| !target.is_empty())
                    .ok_or_else(|| format!("{} names no gitdir", dot_git.display()))?;
                return Ok(Some((dir.to_path_buf(), dir.join(target))));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("{} not read: {e}", dot_git.display())),
        }
    }
    Ok(None)
}

/// `/<names of mount_folder below top>/.trash/`, each name escaped for gitignore.
fn exclude_line(top: &Path, mount_folder: &Path) -> Result<String, String> {
    let below = mount_folder
        .strip_prefix(top)
        .map_err(|_| format!("{} is not inside {}", mount_folder.display(), top.display()))?;
    let mut line = String::from("/");
    for component in below.components() {
        let Component::Normal(name) = component else {
            return Err(format!(
                "{} is not a plain path below the repository",
                below.display()
            ));
        };
        let name = name
            .to_str()
            .ok_or_else(|| format!("{} is not UTF-8", below.display()))?;
        // A control character (a newline above all) would split the line.
        if name.chars().any(char::is_control) {
            return Err(format!("{:?} holds a control character", name));
        }
        line.push_str(&escape_glob(name));
        line.push('/');
    }
    line.push_str(TRASH_DIR);
    line.push('/');
    Ok(line)
}

/// `name` with gitignore's metacharacters (`\ * ? [ # !`) and its trailing spaces escaped by a backslash.
fn escape_glob(name: &str) -> String {
    let kept = name.trim_end_matches(' ');
    let mut escaped = String::with_capacity(name.len() + 4);
    for c in kept.chars() {
        if matches!(c, '\\' | '*' | '?' | '[' | '#' | '!') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    for _ in kept.len()..name.len() {
        escaped.push_str("\\ ");
    }
    escaped
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use engram_dashboard_platform::fs::link_dir;

    use super::*;
    use crate::store::mem::MemStore;
    use crate::store::FsStore;

    const ROOT: &str = "/lib";

    fn mp(text: &str) -> MountPath {
        MountPath::parse(text).unwrap()
    }

    fn id() -> BundleId {
        BundleId::parse("20261123T081500Z-a1b2c3").unwrap()
    }

    fn root() -> &'static Path {
        Path::new(ROOT)
    }

    /// A store with `.trash/<id()>` made.
    fn bundled() -> MemStore {
        let store = MemStore::new(root());
        store.dir(&format!(".trash/{}", id().as_str()));
        store
    }

    fn inside(rel: &str) -> String {
        format!(".trash/{}/{rel}", id().as_str())
    }

    fn there(store: &dyn LibraryStore, folder: &Path, rel: &str) -> bool {
        store.stat(&mp(rel).to_path(folder)).is_ok()
    }

    fn write(folder: &Path, rel: &str, text: &str) -> PathBuf {
        let path = mp(rel).to_path(folder);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }

    // ── bundle ids ──

    #[test]
    fn bundle_ids_parse_exactly_the_form_they_are_made_in() {
        let now = UNIX_EPOCH + Duration::from_secs(1_795_421_700); // 2026-11-23T08:15:00Z
        let made = BundleId::new(now);
        assert!(
            made.as_str().starts_with("20261123T081500Z-"),
            "{}",
            made.as_str()
        );
        assert_eq!(BundleId::parse(made.as_str()), Some(made.clone()));
        assert_eq!(made.deleted_at(), now);
        assert_eq!(made.bundle().0, made.as_str());
        let leap = BundleId::parse("20240229T235959Z-000000").unwrap();
        assert_eq!(BundleId::parse(leap.as_str()), Some(leap));
        for bad in [
            "20261123T081500Z-A1B2C3",
            "20261123T081500Z-a1b2c",
            "20261123T081500Z-a1b2c3d",
            "20261123T081500-a1b2c3",
            "20261123 081500Z-a1b2c3",
            "20260230T081500Z-a1b2c3",
            "20230229T081500Z-a1b2c3",
            "20261323T081500Z-a1b2c3",
            "20261100T081500Z-a1b2c3",
            "20261123T241500Z-a1b2c3",
            "20261123T086000Z-a1b2c3",
            "20261123T081560Z-a1b2c3",
            "19691231T235959Z-a1b2c3",
            "x0261123T081500Z-a1b2c3",
            "",
            ".engram-bundle.json",
        ] {
            assert_eq!(BundleId::parse(bad), None, "{bad}");
        }
    }

    // ── the trash folder ──

    #[test]
    fn an_absent_trash_folder_is_made_and_a_link_or_a_file_is_unusable() {
        let trash = TrashLock::default();
        let held = trash.take_for_emptying();
        let store = MemStore::new(root());
        assert_eq!(check_trash_folder(&store, root()), Ok(false));
        ensure_trash_folder(&store, root(), &held).unwrap();
        assert_eq!(check_trash_folder(&store, root()), Ok(true));

        let file = MemStore::new(root());
        file.file(".trash", "not a folder");
        assert!(check_trash_folder(&file, root()).is_err());
        assert!(ensure_trash_folder(&file, root(), &held).is_err());

        let link = MemStore::new(root());
        link.link(".trash");
        assert!(ensure_trash_folder(&link, root(), &held).is_err());
        assert!(!there(&link, root(), &format!(".trash/{}", id().as_str())));

        let failing = MemStore::new(root());
        failing.dir(".trash");
        failing.fail_stat(".trash");
        assert!(
            check_trash_folder(&failing, root()).is_err(),
            "a failed stat is no absence"
        );
    }

    // ADR-0409 decision 5: `.trash` itself a link — unusable, nothing made through it.
    #[test]
    fn a_trash_folder_that_is_a_link_on_disk_is_unusable_and_nothing_goes_through_it() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("lib");
        let keep = dir.path().join("keep");
        fs::create_dir_all(&folder).unwrap();
        fs::create_dir_all(&keep).unwrap();
        link_dir(&keep, &folder.join(TRASH_DIR)).unwrap();
        let trash = TrashLock::default();
        let held = trash.take_for_emptying();

        let unusable = ensure_trash_folder(&FsStore, &folder, &held).unwrap_err();
        assert!(unusable.why.contains("link"), "{unusable:?}");
        assert_eq!(
            fs::read_dir(&keep).unwrap().count(),
            0,
            "nothing made behind the link"
        );

        let real = dir.path().join("real");
        fs::create_dir_all(&real).unwrap();
        ensure_trash_folder(&FsStore, &real, &held).unwrap();
        assert!(real.join(TRASH_DIR).is_dir());
    }

    #[test]
    fn a_taken_bundle_id_draws_a_new_one_and_a_free_one_is_kept() {
        let trash = TrashLock::default();
        let held = trash.take_for_emptying();
        let store = MemStore::new(root());
        store.dir(".trash");
        assert_eq!(make_bundle(&store, root(), &id(), &held).unwrap(), id());
        let other = make_bundle(&store, root(), &id(), &held).unwrap();
        assert_ne!(other, id());
        assert_eq!(
            other.deleted_at(),
            id().deleted_at(),
            "drawn at the operation's time"
        );
        assert!(there(&store, root(), bundle_path(&other).as_str()));
        assert!(there(&store, root(), bundle_path(&id()).as_str()));
    }

    // ── the record ──

    fn roots() -> Vec<RecordRoot> {
        vec![
            RecordRoot {
                path: mp("rules/a.md"),
                staging: None,
            },
            RecordRoot {
                path: mp("skills/tool"),
                staging: None,
            },
            RecordRoot {
                path: mp("copy.md.tmp7.1"),
                staging: Some(mp("copy.md")),
            },
        ]
    }

    #[test]
    fn the_record_round_trips_and_its_roots_are_read_filtered_by_existence() {
        let trash = TrashLock::default();
        let held = trash.take_for_emptying();
        let store = bundled();
        store.file(&inside("rules/a.md"), "a");
        store.file(&inside("skills/tool/SKILL.md"), "s");
        store.file(&inside("copy.md.tmp7.1"), "partial");
        publish_record(&store, root(), &id(), &roots(), &held).unwrap();

        let RecordRead::Roots(rows) = read_record(&store, root(), &id()) else {
            panic!("a readable record")
        };
        let read: Vec<RecordRoot> = rows.iter().map(|row| row.root.clone()).collect();
        assert_eq!(read, roots());
        let states: Vec<&RootState> = rows.iter().map(|row| &row.state).collect();
        assert_eq!(
            states,
            [
                &RootState::Present(EntryKind::File),
                &RootState::Present(EntryKind::Folder),
                &RootState::Present(EntryKind::File)
            ]
        );

        store.remove(&inside("rules/a.md"));
        store.fail_stat(&inside("skills/tool"));
        let RecordRead::Roots(rows) = read_record(&store, root(), &id()) else {
            panic!("a readable record")
        };
        let paths: Vec<&str> = rows.iter().map(|row| row.root.path.as_str()).collect();
        assert_eq!(
            paths,
            ["skills/tool", "copy.md.tmp7.1"],
            "a removed root drops out"
        );
        assert!(
            matches!(rows[0].state, RootState::Failed(_)),
            "a failed stat keeps the root: {:?}",
            rows[0].state
        );
        let names: Vec<String> = store
            .list(&bundle_path(&id()).to_path(root()))
            .unwrap()
            .into_iter()
            .map(|(name, _)| name.into_string().unwrap())
            .collect();
        assert!(
            !names
                .iter()
                .any(|name| name.contains(".tmp") && name.starts_with(RECORD_NAME)),
            "no temp left: {names:?}"
        );
    }

    #[test]
    fn a_record_that_does_not_parse_or_names_no_library_path_is_unreadable() {
        let store = bundled();
        assert_eq!(read_record(&store, root(), &id()), RecordRead::Missing);
        for bytes in [
            "not json",
            r#"{"roots":[{"path":"../x.md"}]}"#,
            r#"{"roots":[{"path":".trash/x.md"}]}"#,
            r#"{"roots":[{"path":"a.md","staging":{"intended":"b:c"}}]}"#,
            r#"{"roots":"a.md"}"#,
        ] {
            store.file(&inside(RECORD_NAME), bytes);
            assert!(
                matches!(
                    read_record(&store, root(), &id()),
                    RecordRead::Unreadable { .. }
                ),
                "{bytes}"
            );
        }
    }

    // ADR-0409 decision 8: the moves stand, the record does not — the files are the rows.
    #[test]
    fn a_failed_record_publication_leaves_the_files_for_the_fallback_rows() {
        let trash = TrashLock::default();
        let held = trash.take_for_emptying();
        let store = bundled();
        store.file(&inside("rules/a.md"), "a");
        store.file(&inside("skills/tool/SKILL.md"), "s");
        store.file(&inside("copy.md.tmp7.1/x.md"), "partial");
        store.link(&inside("linked"));
        store.fail_rename_to(&inside(RECORD_NAME));

        assert!(publish_record(&store, root(), &id(), &roots(), &held).is_err());

        assert_eq!(read_record(&store, root(), &id()), RecordRead::Missing);
        let top = store.list(&bundle_path(&id()).to_path(root())).unwrap();
        assert!(
            !top.iter().any(|(name, _)| is_record_name(name)),
            "the failed publication's temp is removed"
        );
        // A record temp a crash left is no row either.
        store.file(&inside(&format!("{RECORD_NAME}.tmp99.3")), "half");
        let rows = fallback_rows(&store, root(), &id()).unwrap();
        let seen: Vec<(&str, EntryKind, bool)> = rows
            .iter()
            .map(|row| (row.path.as_str(), row.kind, row.staging))
            .collect();
        assert_eq!(
            seen,
            [
                ("copy.md.tmp7.1/x.md", EntryKind::File, true),
                ("linked", EntryKind::Link, false),
                ("rules/a.md", EntryKind::File, false),
                ("skills/tool/SKILL.md", EntryKind::File, false),
            ]
        );
    }

    #[test]
    fn the_record_temp_shape_is_the_record_names_alone() {
        assert!(is_record_name(OsStr::new(RECORD_NAME)));
        assert!(is_record_name(OsStr::new(&format!(
            "{RECORD_NAME}.tmp12.3"
        ))));
        assert!(!is_record_name(OsStr::new(&format!("{RECORD_NAME}.tmp12"))));
        assert!(!is_record_name(OsStr::new("a.md.tmp12.3")));
        assert!(!is_record_name(OsStr::new(&format!("x{RECORD_NAME}"))));
        assert!(!is_record_name(OsStr::new(&format!(
            "{RECORD_NAME}.tmpX.tmp1.2"
        ))));
    }

    #[test]
    fn a_temp_of_another_name_is_no_record_temp_it_is_a_row_and_keeps_the_bundle() {
        let trash = TrashLock::default();
        let held = trash.take_for_emptying();
        let store = bundled();
        let foreign = format!("{RECORD_NAME}.tmpX.tmp1.2");
        store.file(&inside(&foreign), "not ours");
        let rows = fallback_rows(&store, root(), &id()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path.as_str(), foreign);
        assert!(!prune_shell(&store, root(), &id(), None, &held).unwrap());
        assert!(there(&store, root(), &inside(&foreign)), "never removed");
    }

    #[test]
    fn a_second_record_is_refused_and_the_first_kept() {
        let trash = TrashLock::default();
        let held = trash.take_for_emptying();
        let store = bundled();
        let first = [RecordRoot {
            path: mp("a.md"),
            staging: None,
        }];
        let second = [RecordRoot {
            path: mp("b.md"),
            staging: None,
        }];
        store.file(&inside("a.md"), "a");
        store.file(&inside("b.md"), "b");
        publish_record(&store, root(), &id(), &first, &held).unwrap();
        let refused = publish_record(&store, root(), &id(), &second, &held).unwrap_err();
        assert_eq!(refused.kind(), io::ErrorKind::AlreadyExists);
        let RecordRead::Roots(rows) = read_record(&store, root(), &id()) else {
            panic!("a readable record")
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].root.path, mp("a.md"));
        let top = store.list(&bundle_path(&id()).to_path(root())).unwrap();
        assert_eq!(
            top.iter().filter(|(name, _)| is_record_name(name)).count(),
            1,
            "the refused one's temp is removed"
        );
    }

    #[test]
    fn a_root_behind_a_file_is_read_as_absent() {
        let trash = TrashLock::default();
        let held = trash.take_for_emptying();
        let store = bundled();
        store.file(&inside("a"), "a file where the root's folder was");
        let behind = [RecordRoot {
            path: mp("a/b.md"),
            staging: None,
        }];
        publish_record(&store, root(), &id(), &behind, &held).unwrap();
        assert_eq!(
            read_record(&store, root(), &id()),
            RecordRead::Roots(Vec::new())
        );
    }

    // ADR-0409 decisions 5 and 6: nothing is read or listed through a linked bundle or `.trash`.
    #[test]
    fn a_linked_bundle_or_trash_folder_is_never_read_through() {
        let dir = tempfile::tempdir().unwrap();
        let keep = dir.path().join("keep");
        write(&keep, RECORD_NAME, r#"{"roots":[{"path":"x.md"}]}"#);
        write(&keep, "x.md", "keep");
        write(
            &keep,
            &format!("{}/{RECORD_NAME}", id().as_str()),
            r#"{"roots":[{"path":"x.md"}]}"#,
        );
        write(&keep, &format!("{}/x.md", id().as_str()), "keep");

        let linked_bundle = dir.path().join("a");
        fs::create_dir_all(linked_bundle.join(TRASH_DIR)).unwrap();
        link_dir(&keep, &bundle_path(&id()).to_path(&linked_bundle)).unwrap();
        let linked_trash = dir.path().join("b");
        fs::create_dir_all(&linked_trash).unwrap();
        link_dir(&keep, &linked_trash.join(TRASH_DIR)).unwrap();

        for folder in [&linked_bundle, &linked_trash] {
            assert!(
                matches!(
                    read_record(&FsStore, folder, &id()),
                    RecordRead::NotABundle { .. }
                ),
                "{folder:?}"
            );
            let error = fallback_rows(&FsStore, folder, &id()).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::NotADirectory, "{folder:?}");
        }
        assert_eq!(fs::read_to_string(keep.join("x.md")).unwrap(), "keep");
    }

    #[test]
    fn a_failed_trash_move_leaves_no_folder_it_made_and_the_bundle_then_prunes() {
        let trash = TrashLock::default();
        let held = trash.take_for_emptying();
        let store = bundled();
        store.file("a/b/c.md", "c");
        store.fail_rename_to(&inside("a/b/c.md"));
        let path = mp("a/b/c.md").to_path(root());
        assert!(store.trash(root(), &path, &id().bundle()).is_err());
        assert!(
            !there(&store, root(), &inside("a")),
            "the folders the move made are gone"
        );
        assert!(there(&store, root(), "a/b/c.md"), "the item stays");
        assert!(prune_shell(&store, root(), &id(), None, &held).unwrap());
    }

    // ── the shell ──

    // ADR-0409 decision 8: a nested root gone → the bundle folder gone.
    #[test]
    fn a_nested_root_gone_takes_the_empty_folders_the_record_and_the_bundle_folder() {
        let trash = TrashLock::default();
        let held = trash.take_for_emptying();
        let store = bundled();
        store.file(&inside("a/b/c.md"), "c");
        let only = [RecordRoot {
            path: mp("a/b/c.md"),
            staging: None,
        }];
        publish_record(&store, root(), &id(), &only, &held).unwrap();
        store.file(&inside(&format!("{RECORD_NAME}.tmp99.3")), "a crash's temp");
        // The root restored or removed.
        store.remove(&inside("a/b/c.md"));

        assert!(prune_shell(&store, root(), &id(), Some(&mp("a/b/c.md")), &held).unwrap());
        assert!(!there(&store, root(), bundle_path(&id()).as_str()));
        assert!(there(&store, root(), ".trash"), ".trash itself stays");
    }

    #[test]
    fn pruning_climbs_past_a_folder_already_gone() {
        let trash = TrashLock::default();
        let held = trash.take_for_emptying();
        let store = bundled();
        store.file(&inside("a/b/c.md"), "c");
        store.remove(&inside("a/b"));
        assert!(prune_shell(&store, root(), &id(), Some(&mp("a/b/c.md")), &held).unwrap());
        assert!(!there(&store, root(), bundle_path(&id()).as_str()));
    }

    #[test]
    fn surviving_content_is_never_pruned() {
        let trash = TrashLock::default();
        let held = trash.take_for_emptying();
        let store = bundled();
        store.file(&inside("a/b.md"), "b");
        store.file(&inside("a/c.md"), "c");
        store.file(&inside("Thumbs.db"), "litter");
        store.dir(&inside("d/e"));
        let both = [
            RecordRoot {
                path: mp("a/b.md"),
                staging: None,
            },
            RecordRoot {
                path: mp("a/c.md"),
                staging: None,
            },
        ];
        publish_record(&store, root(), &id(), &both, &held).unwrap();
        store.remove(&inside("a/b.md"));

        assert!(!prune_shell(&store, root(), &id(), Some(&mp("a/b.md")), &held).unwrap());
        assert!(there(&store, root(), &inside("a/c.md")));
        assert!(there(&store, root(), &inside(RECORD_NAME)));

        store.remove(&inside("a/c.md"));
        assert!(!prune_shell(&store, root(), &id(), Some(&mp("a/c.md")), &held).unwrap());
        assert!(
            !there(&store, root(), &inside("a")),
            "the emptied folder goes"
        );
        assert!(
            there(&store, root(), &inside("Thumbs.db")),
            "litter keeps the bundle"
        );
        assert!(
            there(&store, root(), &inside(RECORD_NAME)),
            "so does the record"
        );
        assert!(
            there(&store, root(), &inside("d/e")),
            "an empty folder that held no root is litter too"
        );
    }

    #[test]
    fn a_crash_bundle_with_no_record_and_nothing_in_it_is_removable() {
        let trash = TrashLock::default();
        let held = trash.take_for_emptying();
        let store = bundled();
        assert!(prune_shell(&store, root(), &id(), None, &held).unwrap());
        assert!(!there(&store, root(), bundle_path(&id()).as_str()));
        assert!(
            prune_shell(&store, root(), &id(), None, &held).unwrap(),
            "gone stays gone"
        );

        let kept = bundled();
        kept.file(&inside("a.md"), "a root");
        assert!(!prune_shell(&kept, root(), &id(), None, &held).unwrap());
        assert!(there(&kept, root(), &inside("a.md")));

        let foreign = MemStore::new(root());
        foreign.link(&format!(".trash/{}", id().as_str()));
        assert!(!prune_shell(&foreign, root(), &id(), None, &held).unwrap());
        assert!(
            there(&foreign, root(), bundle_path(&id()).as_str()),
            "a link is no bundle"
        );
    }

    // ADR-0409 decision 9: a link between the bundle and a row.
    #[test]
    fn a_link_between_the_bundle_and_a_row_is_reported_and_never_pruned_through() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("lib");
        let keep = dir.path().join("keep");
        let kept = write(&keep, "b/x.md", "keep");
        let bundle = bundle_path(&id()).to_path(&folder);
        fs::create_dir_all(&bundle).unwrap();
        link_dir(&keep, &bundle.join("a")).unwrap();
        let trash = TrashLock::default();
        let held = trash.take_for_emptying();
        let row = [RecordRoot {
            path: mp("a/b/x.md"),
            staging: None,
        }];
        publish_record(&FsStore, &folder, &id(), &row, &held).unwrap();

        assert_eq!(
            FsStore
                .check_names(&folder, &row_path(&id(), &mp("a/b/x.md")))
                .unwrap(),
            NameWalk::Link(row_path(&id(), &mp("a")))
        );
        let RecordRead::Roots(rows) = read_record(&FsStore, &folder, &id()) else {
            panic!("a readable record")
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].state, RootState::ThroughLink);

        fs::remove_file(&kept).unwrap();
        assert!(!prune_shell(&FsStore, &folder, &id(), Some(&mp("a/b/x.md")), &held).unwrap());
        assert!(keep.join("b").is_dir(), "nothing removed behind the link");
        assert!(fs::symlink_metadata(bundle.join("a")).is_ok());
        assert!(bundle.join(RECORD_NAME).is_file());
    }

    #[test]
    fn a_real_bundle_prunes_on_disk_and_its_fallback_skips_the_record() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        let trash = TrashLock::default();
        let held = trash.take_for_emptying();
        ensure_trash_folder(&FsStore, folder, &held).unwrap();
        let made = make_bundle(&FsStore, folder, &id(), &held).unwrap();
        write(folder, &format!(".trash/{}/a/b.md", made.as_str()), "b");
        let only = [RecordRoot {
            path: mp("a/b.md"),
            staging: None,
        }];
        publish_record(&FsStore, folder, &made, &only, &held).unwrap();
        let rows = fallback_rows(&FsStore, folder, &made).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, mp("a/b.md"));

        fs::remove_file(row_path(&made, &mp("a/b.md")).to_path(folder)).unwrap();
        assert!(prune_shell(&FsStore, folder, &made, Some(&mp("a/b.md")), &held).unwrap());
        assert!(!bundle_path(&made).to_path(folder).exists());
        assert!(folder.join(TRASH_DIR).is_dir());
    }

    // ── the exclude line ──

    fn exclude(mount: &Path) -> Excluded {
        exclude_trash(mount, &Mutex::new(()))
    }

    /// A \`.git\` folder at \`top\` that reads as a repository (it holds \`HEAD\`); answers it.
    fn repo(top: &Path) -> PathBuf {
        let git = top.join(".git");
        write(top, ".git/HEAD", "ref: refs/heads/main\n");
        git
    }

    fn skipped_for(answer: Excluded, reason: &str) {
        match answer {
            Excluded::Skipped { why } => assert!(why.contains(reason), "{why}"),
            other => panic!("not skipped: {other:?}"),
        }
    }

    #[test]
    fn the_exclude_line_is_anchored_at_the_repository_top_and_written_once() {
        let dir = tempfile::tempdir().unwrap();
        let repo_top = dir.path().join("repo");
        let file = repo(&repo_top).join("info").join("exclude");

        assert_eq!(exclude(&repo_top), Excluded::Added { file: file.clone() });
        assert_eq!(fs::read_to_string(&file).unwrap(), "/.trash/\n");
        assert_eq!(exclude(&repo_top), Excluded::Present { file: file.clone() });
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "/.trash/\n",
            "idempotent"
        );

        let sub = repo_top.join("notes").join("lib");
        fs::create_dir_all(&sub).unwrap();
        assert_eq!(exclude(&sub), Excluded::Added { file: file.clone() });
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "/.trash/\n/notes/lib/.trash/\n"
        );
    }

    // What lies above the test folder is not the test's: it writes only when no repository is found at all.
    #[test]
    fn no_repository_above_the_mount_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = dir.path().join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        match find_repository(&elsewhere) {
            Ok(None) => assert_eq!(exclude(&elsewhere), Excluded::NoRepository),
            Ok(Some((top, _))) => assert!(!top.starts_with(dir.path()), "{top:?}"),
            Err(_) => {}
        }
    }

    #[test]
    fn the_exclude_file_keeps_its_bytes_and_gains_a_newline_first() {
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path();
        repo(top);
        let file = write(top, ".git/info/exclude", "# mine\r\n*.log");
        assert_eq!(exclude(top), Excluded::Added { file: file.clone() });
        assert_eq!(fs::read(&file).unwrap(), b"# mine\r\n*.log\n/.trash/\n");
        let names: Vec<_> = fs::read_dir(file.parent().unwrap()).unwrap().collect();
        assert_eq!(names.len(), 1, "no temp left beside it");

        let crlf = tempfile::tempdir().unwrap();
        repo(crlf.path());
        let file = write(crlf.path(), ".git/info/exclude", "/.trash/\r\n");
        assert_eq!(
            exclude(crlf.path()),
            Excluded::Present { file: file.clone() }
        );
        assert_eq!(fs::read(&file).unwrap(), b"/.trash/\r\n");
    }

    #[test]
    fn the_exclude_line_escapes_gitignore_metacharacters_and_refuses_control_characters() {
        let dir = tempfile::tempdir().unwrap();
        let top_dir = dir.path();
        repo(top_dir);
        // `*`, `?` and control characters cannot be in a Windows name: those are judged on the line alone.
        let top = Path::new("/repo");
        let line = exclude_line(top, &top.join("a*b").join("c?d")).unwrap();
        assert_eq!(line, "/a\\*b/c\\?d/.trash/");
        let split = exclude_line(top, &top.join("a\nb")).unwrap_err();
        assert!(split.contains("control character"), "{split}");
        assert!(exclude_line(top, &top.join("a\rb").join("c")).is_err());
        let mount = top_dir.join("#c").join("[e]").join("!f");
        fs::create_dir_all(&mount).unwrap();
        assert!(matches!(exclude(&mount), Excluded::Added { .. }));
        let text = fs::read_to_string(top_dir.join(".git").join("info").join("exclude")).unwrap();
        assert_eq!(text, "/\\#c/\\[e]/\\!f/.trash/\n");
        assert_eq!(escape_glob("x  "), "x\\ \\ ");
        assert_eq!(escape_glob("a\\b"), "a\\\\b");
    }

    #[test]
    fn a_worktree_and_a_submodule_write_to_their_git_folders_exclude() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main");
        repo(&main);
        let worktree_git = main.join(".git").join("worktrees").join("wt");
        fs::create_dir_all(&worktree_git).unwrap();
        fs::write(worktree_git.join("commondir"), "../..\n").unwrap();
        let wt = dir.path().join("wt");
        fs::create_dir_all(wt.join("lib")).unwrap();
        fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", worktree_git.display()),
        )
        .unwrap();

        assert!(matches!(exclude(&wt.join("lib")), Excluded::Added { .. }));
        let common = main.join(".git").join("info").join("exclude");
        assert_eq!(fs::read_to_string(&common).unwrap(), "/lib/.trash/\n");

        let modules = main.join(".git").join("modules").join("sub");
        write(&modules, "HEAD", "ref: refs/heads/main\n");
        let sub = main.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join(".git"), "gitdir: ../.git/modules/sub\n").unwrap();
        assert!(matches!(exclude(&sub), Excluded::Added { .. }));
        let own = modules.join("info").join("exclude");
        assert_eq!(fs::read_to_string(&own).unwrap(), "/.trash/\n");
        assert_eq!(
            fs::read_to_string(&common).unwrap(),
            "/lib/.trash/\n",
            "the outer repository is untouched"
        );
    }

    #[test]
    fn a_stale_gitdir_or_a_folder_that_is_no_repository_makes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let stale = dir.path().join("stale");
        let gone = dir
            .path()
            .join("gone")
            .join(".git")
            .join("worktrees")
            .join("wt");
        write(&stale, ".git", &format!("gitdir: {}\n", gone.display()));
        skipped_for(exclude(&stale), "not a git folder");
        assert!(
            !dir.path().join("gone").exists(),
            "nothing made outside the mount"
        );

        let plain = dir.path().join("plain");
        fs::create_dir_all(plain.join(".git")).unwrap();
        skipped_for(exclude(&plain), "not a git folder");
        assert!(!plain.join(".git").join("info").exists());
    }

    #[test]
    fn a_linked_or_unreadable_exclude_file_is_skipped_and_nothing_fails() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = dir.path().join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();

        let linked = dir.path().join("linked");
        fs::create_dir_all(repo(&linked).join("info")).unwrap();
        link_dir(
            &elsewhere,
            &linked.join(".git").join("info").join("exclude"),
        )
        .unwrap();
        skipped_for(exclude(&linked), "is a link");

        let linked_info = dir.path().join("linked-info");
        link_dir(&elsewhere, &repo(&linked_info).join("info")).unwrap();
        skipped_for(exclude(&linked_info), "is a link");
        assert_eq!(
            fs::read_dir(&elsewhere).unwrap().count(),
            0,
            "nothing written through a link"
        );

        let unreadable = dir.path().join("unreadable");
        fs::create_dir_all(repo(&unreadable).join("info").join("exclude")).unwrap();
        skipped_for(exclude(&unreadable), "not read");

        let unwritable = dir.path().join("unwritable");
        repo(&unwritable);
        write(&unwritable, ".git/info", "a file where the folder goes");
        assert!(matches!(exclude(&unwritable), Excluded::Skipped { .. }));

        let broken = dir.path().join("broken");
        write(&broken, ".git", "no gitdir here");
        skipped_for(exclude(&broken), "names no gitdir");
    }
}
