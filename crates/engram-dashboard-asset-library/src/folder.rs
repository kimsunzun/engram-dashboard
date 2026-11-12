//! Bounded, concurrent folder IO on a per-launch budget. Every stat, listing, canonicalize and file read on a folder
//! outside the library (folder entries, native skill roots, the working folder, the library root) goes through
//! [`BoundedIo`]; library files inside the root are read with the store.
//!
//! ## Rules
//!
//! - **Budgets** (ADR-0360): a launch ([`BoundedIo::begin_launch`]) waits at most [`LaunchWaits::required`] for
//!   required IO and, on a separate allowance never taken from it, [`LaunchWaits::advisory`] for advisory IO; a read
//!   computation outside a launch ([`BoundedIo::begin_read`]) gets one limit. A budget is charged only by the time a
//!   call here spends waiting on IO — never by lock waits, parsing or other work between calls.
//! - **Class B**: required and advisory asks never share a batch or a wait. A call holding both runs its required
//!   asks first and alone, then its advisory asks on the advisory allowance. A key asked both ways is required. This
//!   holds for every `*_all` call — a caller that must skip advisory work after a block splits its calls.
//! - **Nothing left = no worker**: once a phase's allowance is spent, a key without an answer in the [`LaunchIo`]
//!   answers [`NOT_CHECKED`] at once — distinct from [`TIMEOUT`] (an op that started and did not answer in time).
//! - **One worker per key**: a call starts a worker only for a key with no call in flight. A required ask joins a
//!   running worker of any age; an advisory ask joins only one that has run for at most one limit, else it answers
//!   [`NOT_CHECKED`]. A fresh read never joins and is never joined.
//! - **Answers live in the asking [`LaunchIo`]** and die with it. Stored: what a worker answered, and a required
//!   timeout. Never stored: [`NOT_CHECKED`] and an advisory wait that ended first — so a later required ask of that key
//!   asks again (joining the worker if it still runs).
//! - **Entry lifetime**: a new call is inserted under the in-flight lock, the lock is released, then the worker is
//!   spawned. The worker's completion guard fills the answer and wakes the waiters under the answer mutex, releases it,
//!   then removes the entry (only its own) under the in-flight lock — the two are never nested — on every exit: an
//!   answer, a panic, or a spawn that dropped the job unrun. (In the daemon the panic hook ends the process before a
//!   panic unwinds to the guard — ADR-0292 — so the panic answer is a net for test builds only; do not rely on it.)
//!   For an instant after the answer is filled the map still holds the completed call, and a new ask may join it — the
//!   same outcome as joining it a moment earlier while it ran.
//! - **One thread per [`LaunchIo`]**: a launch's checks, prepare and canonicalize run one after another; two threads
//!   asking on one `LaunchIo` at once could each wait up to the same `left`.
//! - **Leaf locks**: the in-flight mutex, each pending call's answer mutex and a `LaunchIo`'s mutexes are never held
//!   while taking another lock, running an op or waiting.
//! - No OS branch here: OS behaviour comes from std, `dunce` and platform (the busy-file retry).
// ADR-0341
// ADR-0360
// TRD S25 step-7 §3-8 · §6

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use engram_dashboard_agent::backend::settings_check::FolderEntry;
use engram_dashboard_agent::launch::FOLDER_LISTING_CAP;
use engram_dashboard_agent::types::{
    LAUNCH_ADVISORY_FOLDER_WAIT_SECS, LAUNCH_REQUIRED_FOLDER_WAIT_SECS,
};
use engram_dashboard_base::file::read_capped;
use engram_dashboard_base::sync::lock;
use engram_dashboard_platform::fs::{path_key, PathKey};

use crate::store::OS_HOOKS;

/// The required budget of one read computation outside a launch, and the age past which an advisory ask no longer
/// joins a running worker.
// ADR-0360
pub const IO_LIMIT: Duration = Duration::from_secs(5);

/// The most entries one listing counts (files, links and folders, and skipped entries whose type could not be read;
/// it may read one more to tell a cut from an exact end); past it the listing stops and is marked
/// [`FolderListing::truncated`]. The value of agent `FOLDER_LISTING_CAP`, which the `folder-listing-partial` notice
/// states.
// TRD S25 U2
pub const LIST_CAP: usize = FOLDER_LISTING_CAP;

/// The largest folder file [`BoundedIo::read_one`] reads; a longer file is an `InvalidData` error.
pub const READ_CAP: u64 = 16 * 1024 * 1024;

/// The diagnostic of an op that was started and did not answer before its phase's wait ended.
pub const TIMEOUT: &str = "timeout";

/// The diagnostic of a key no worker was started for — the phase's allowance was spent, or an advisory ask found only
/// a worker older than one limit.
pub const NOT_CHECKED: &str = "not checked (budget spent)";

/// The advisory allowance of a read computation.
const READ_ADVISORY_WAIT: Duration = Duration::from_secs(1);
const WORKER_NAME: &str = "folder-io";
const WORKER_PANICKED: &str = "worker panicked";
const SPAWN_FAILED: &str = "spawn failed";

/// One bounded op on one absolute path — the unit of de-duplication and of a stored answer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum IoKey {
    Stat(PathKey),
    List(PathKey),
    Canonical(PathKey),
    Read(PathKey),
}

impl IoKey {
    fn path_key(&self) -> &PathKey {
        match self {
            IoKey::Stat(key) | IoKey::List(key) | IoKey::Canonical(key) | IoKey::Read(key) => key,
        }
    }
}

/// Which phase an ask belongs to (class B). `Required` = its failure blocks the launch; `Advisory` = it only feeds a
/// notice or a best-effort value and is skipped when it does not answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    Required,
    Advisory,
}

/// The folder ops [`BoundedIo`] bounds. Production = [`StdFolderOps`]; tests use gate-controlled fakes.
pub trait FolderOps: Send + Sync {
    fn stat(&self, p: &Path) -> FolderStatus;
    /// A read-only listing of folder `p`, at most `cap` entries — folders count toward the cap (an implementation may
    /// read one more to tell a cut from an exact end). Order: per level, files (and links) first, then folders, each
    /// by name; a folder's contents follow it depth-first — callers and fakes rely on this order. `Err` = `p` is
    /// missing, not a folder, or could not be listed itself; a subfolder or entry below it that cannot be read is left
    /// out and named in [`FolderListing::skipped`].
    fn list(&self, p: &Path, cap: usize) -> Result<FolderListing, FolderStatus>;
    /// The canonical path the agent manager would use for `p` (`dunce::canonicalize` — agent `manager.rs`).
    fn canonical(&self, p: &Path) -> io::Result<PathBuf>;
    /// The text of file `p`, at most `cap` bytes — longer or not UTF-8 = `InvalidData`.
    fn read(&self, p: &Path, cap: u64) -> io::Result<String>;
}

/// Starts a worker. The job must run on another thread — run inline, it would run on the caller unbounded and
/// uncharged. `Err` = it was not started, and `job` has been dropped unrun. Production = [`ThreadSpawn`].
pub trait Spawn: Send + Sync {
    fn spawn(&self, name: &str, job: Box<dyn FnOnce() + Send>) -> io::Result<()>;
}

/// What a stat (or a failed listing or canonicalize) found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FolderStatus {
    Ok,
    Missing,
    NotADirectory,
    /// An OS error, [`TIMEOUT`], [`NOT_CHECKED`] or a failed worker ("worker panicked", "spawn failed") — all count
    /// as unreadable.
    Unreadable(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderListing {
    /// In listing order. Paths are relative to the listed folder.
    pub entries: Vec<FolderEntry>,
    /// The listing stopped at its cap; more entries exist.
    pub truncated: bool,
    /// The subfolders and entries below the listed folder that could not be read and were left out — folder-relative
    /// paths with `/` separators, in walk order. Empty = nothing was left out.
    pub skipped: Vec<String>,
}

/// The IO time left to one [`LaunchIo`] — charged only by time spent waiting on bounded IO.
#[derive(Clone)]
pub struct IoBudget {
    left: Arc<Mutex<Duration>>,
    /// A separate allowance, never taken from `left`.
    advisory_left: Arc<Mutex<Duration>>,
}

impl IoBudget {
    fn new(required: Duration, advisory: Duration) -> Self {
        IoBudget {
            left: Arc::new(Mutex::new(required)),
            advisory_left: Arc::new(Mutex::new(advisory)),
        }
    }

    fn allowance(&self, need: Need) -> &Mutex<Duration> {
        match need {
            Need::Required => &self.left,
            Need::Advisory => &self.advisory_left,
        }
    }
}

/// The two folder waits of one launch. Production = [`LaunchWaits::LAUNCH`]; tests shorten both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LaunchWaits {
    pub required: Duration,
    pub advisory: Duration,
}

impl LaunchWaits {
    /// agent `types.rs` `LAUNCH_REQUIRED_FOLDER_WAIT_SECS` · `LAUNCH_ADVISORY_FOLDER_WAIT_SECS`.
    // ADR-0360
    pub const LAUNCH: LaunchWaits = LaunchWaits {
        required: Duration::from_secs(LAUNCH_REQUIRED_FOLDER_WAIT_SECS),
        advisory: Duration::from_secs(LAUNCH_ADVISORY_FOLDER_WAIT_SECS),
    };
}

/// One launch's (or one read computation's) IO context: its budget and every answer it got. Every check, the prepare
/// and the working-folder canonicalize of one launch share one.
pub struct LaunchIo {
    pub budget: IoBudget,
    answers: Mutex<HashMap<IoKey, IoAnswer>>,
}

impl LaunchIo {
    fn new(required: Duration, advisory: Duration) -> Self {
        LaunchIo {
            budget: IoBudget::new(required, advisory),
            answers: Mutex::new(HashMap::new()),
        }
    }
}

#[derive(Clone, Debug)]
enum IoAnswer {
    Stat(FolderStatus),
    List(Result<FolderListing, FolderStatus>),
    Canonical(Result<PathBuf, FolderStatus>),
    Read(Result<String, ReadFail>),
}

/// An `io::Error` kept as parts, so one answer can be handed to every joiner and stored.
#[derive(Clone, Debug)]
struct ReadFail {
    kind: io::ErrorKind,
    text: String,
}

struct Pending {
    started: Instant,
    answer: Mutex<Option<IoAnswer>>,
    done: Condvar,
}

impl Pending {
    fn new() -> Self {
        Pending {
            started: Instant::now(),
            answer: Mutex::new(None),
            done: Condvar::new(),
        }
    }

    fn wait_until(&self, deadline: Instant) -> Option<IoAnswer> {
        let mut slot = lock(&self.answer);
        loop {
            if let Some(answer) = slot.as_ref() {
                return Some(answer.clone());
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            slot = self
                .done
                .wait_timeout(slot, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

type Inflight = Arc<Mutex<HashMap<IoKey, Arc<Pending>>>>;

/// The worker's completion guard — it runs on every exit of the job, including a panic (test builds — in the daemon
/// the panic hook ends the process first) and a job dropped unrun.
struct Completion {
    key: IoKey,
    pending: Arc<Pending>,
    inflight: Inflight,
    answer: Option<IoAnswer>,
    ran: bool,
}

impl Drop for Completion {
    fn drop(&mut self) {
        let answer = self.answer.take().unwrap_or_else(|| {
            let reason = if self.ran {
                WORKER_PANICKED
            } else {
                SPAWN_FAILED
            };
            unanswered(&self.key, reason, io::ErrorKind::Other)
        });
        {
            let mut slot = lock(&self.pending.answer);
            *slot = Some(answer);
            self.pending.done.notify_all();
        }
        let mut inflight = lock(&self.inflight);
        if inflight
            .get(&self.key)
            .is_some_and(|entry| Arc::ptr_eq(entry, &self.pending))
        {
            inflight.remove(&self.key);
        }
    }
}

/// Bounded folder IO shared by every launch and read computation of one library service.
pub struct BoundedIo {
    limit: Duration,
    launch: LaunchWaits,
    list_cap: usize,
    inflight: Inflight,
    ops: Arc<dyn FolderOps>,
    spawn: Arc<dyn Spawn>,
}

impl BoundedIo {
    /// `limit` = a read computation's required budget and the advisory join cutoff (production [`IO_LIMIT`]);
    /// `list_cap` = production [`LIST_CAP`].
    pub fn new(
        limit: Duration,
        launch: LaunchWaits,
        list_cap: usize,
        ops: Arc<dyn FolderOps>,
        spawn: Arc<dyn Spawn>,
    ) -> Self {
        BoundedIo {
            limit,
            launch,
            list_cap,
            inflight: Arc::new(Mutex::new(HashMap::new())),
            ops,
            spawn,
        }
    }

    /// The spawner of the workers — the library service starts its short-lived merge jobs with it too.
    pub(crate) fn spawner(&self) -> &dyn Spawn {
        &*self.spawn
    }

    /// A launch: a fresh required budget and advisory allowance from [`LaunchWaits`], no answers.
    pub fn begin_launch(&self) -> Arc<LaunchIo> {
        Arc::new(LaunchIo::new(self.launch.required, self.launch.advisory))
    }

    /// A read computation outside a launch: one limit required plus a 1 s advisory allowance, no answers.
    pub fn begin_read(&self) -> Arc<LaunchIo> {
        Arc::new(LaunchIo::new(self.limit, READ_ADVISORY_WAIT))
    }

    /// One answer per distinct path key of `paths` (module rules).
    pub fn stat_all(
        &self,
        io: &LaunchIo,
        paths: &[(PathBuf, Need)],
    ) -> HashMap<PathKey, FolderStatus> {
        self.ask(io, IoKey::Stat, paths, false)
            .into_iter()
            .map(|(key, answer)| match answer {
                IoAnswer::Stat(status) => (key, status),
                _ => unreachable!("a stat key holds a stat answer"),
            })
            .collect()
    }

    /// One answer per distinct path key of `paths` (module rules).
    pub fn list_all(
        &self,
        io: &LaunchIo,
        paths: &[(PathBuf, Need)],
    ) -> HashMap<PathKey, Result<FolderListing, FolderStatus>> {
        self.ask(io, IoKey::List, paths, false)
            .into_iter()
            .map(|(key, answer)| match answer {
                IoAnswer::List(listing) => (key, listing),
                _ => unreachable!("a list key holds a list answer"),
            })
            .collect()
    }

    /// One answer per distinct path key of `paths` (module rules). An OS error is `Missing` when the path is not
    /// found, else `Unreadable`.
    pub fn canonical_all(
        &self,
        io: &LaunchIo,
        paths: &[(PathBuf, Need)],
    ) -> HashMap<PathKey, Result<PathBuf, FolderStatus>> {
        self.ask(io, IoKey::Canonical, paths, false)
            .into_iter()
            .map(|(key, answer)| match answer {
                IoAnswer::Canonical(path) => (key, path),
                _ => unreachable!("a canonical key holds a canonical answer"),
            })
            .collect()
    }

    /// The text of the file at the absolute `path`, at most [`READ_CAP`] bytes. `fresh` = ignore a stored answer and
    /// any read in flight — start a new read. An op that did not answer, or a key not checked because the allowance
    /// was spent, is `ErrorKind::TimedOut` with [`TIMEOUT`] or [`NOT_CHECKED`] as its text.
    pub fn read_one(
        &self,
        io: &LaunchIo,
        path: &Path,
        need: Need,
        fresh: bool,
    ) -> io::Result<String> {
        match self
            .ask(io, IoKey::Read, &[(path.to_path_buf(), need)], fresh)
            .into_values()
            .next()
        {
            Some(IoAnswer::Read(text)) => text.map_err(|fail| io::Error::new(fail.kind, fail.text)),
            _ => unreachable!("one read ask holds one read answer"),
        }
    }

    fn ask(
        &self,
        io: &LaunchIo,
        key_of: fn(PathKey) -> IoKey,
        paths: &[(PathBuf, Need)],
        fresh: bool,
    ) -> HashMap<PathKey, IoAnswer> {
        let required: HashSet<PathKey> = paths
            .iter()
            .filter(|(_, need)| *need == Need::Required)
            .map(|(path, _)| path_key(path))
            .collect();
        let mut seen = HashSet::new();
        let (mut required_asks, mut advisory_asks) = (Vec::new(), Vec::new());
        for (path, _) in paths {
            let key = path_key(path);
            if !seen.insert(key.clone()) {
                continue;
            }
            let phase = if required.contains(&key) {
                &mut required_asks
            } else {
                &mut advisory_asks
            };
            phase.push((key_of(key), path.clone()));
        }
        let mut answers = self.phase(io, required_asks, Need::Required, fresh);
        answers.extend(self.phase(io, advisory_asks, Need::Advisory, fresh));
        answers
            .into_iter()
            .map(|(key, answer)| (key.path_key().clone(), answer))
            .collect()
    }

    /// One batch of one phase: stored answers, then joins and new workers, then one wait against one deadline.
    fn phase(
        &self,
        io: &LaunchIo,
        asks: Vec<(IoKey, PathBuf)>,
        need: Need,
        fresh: bool,
    ) -> HashMap<IoKey, IoAnswer> {
        let mut out = HashMap::new();
        let mut todo = Vec::new();
        {
            let stored = lock(&io.answers);
            for (key, path) in asks {
                match stored.get(&key) {
                    Some(answer) if !fresh => {
                        out.insert(key, answer.clone());
                    }
                    _ => todo.push((key, path)),
                }
            }
        }
        if todo.is_empty() {
            return out;
        }
        let allowance = io.budget.allowance(need);
        if lock(allowance).is_zero() {
            for (key, _) in todo {
                let answer = unanswered(&key, NOT_CHECKED, io::ErrorKind::TimedOut);
                out.insert(key, answer);
            }
            return out;
        }

        let mut waits = Vec::new();
        let mut starts = Vec::new();
        {
            let mut inflight = lock(&self.inflight);
            for (key, path) in todo {
                if !fresh {
                    if let Some(pending) = inflight.get(&key) {
                        if need == Need::Advisory && pending.started.elapsed() > self.limit {
                            let answer = unanswered(&key, NOT_CHECKED, io::ErrorKind::TimedOut);
                            out.insert(key, answer);
                        } else {
                            waits.push((key, Arc::clone(pending)));
                        }
                        continue;
                    }
                }
                let pending = Arc::new(Pending::new());
                if !fresh {
                    inflight.insert(key.clone(), Arc::clone(&pending));
                }
                starts.push((key.clone(), path, Arc::clone(&pending)));
                waits.push((key, pending));
            }
        }
        for (key, path, pending) in starts {
            self.start(key, path, pending);
        }
        if waits.is_empty() {
            return out;
        }

        let left = *lock(allowance);
        let began = Instant::now();
        let deadline = began + left;
        let mut answered = Vec::with_capacity(waits.len());
        for (key, pending) in waits {
            match pending.wait_until(deadline) {
                Some(answer) => answered.push((key, answer, true)),
                None => {
                    let answer = unanswered(&key, TIMEOUT, io::ErrorKind::TimedOut);
                    answered.push((key, answer, need == Need::Required));
                }
            }
        }
        let waited = began.elapsed().min(left);
        {
            let mut allowance = lock(allowance);
            *allowance = allowance.saturating_sub(waited);
        }
        {
            let mut stored = lock(&io.answers);
            for (key, answer, keep) in &answered {
                if *keep {
                    stored.insert(key.clone(), answer.clone());
                }
            }
        }
        out.extend(answered.into_iter().map(|(key, answer, _)| (key, answer)));
        out
    }

    fn start(&self, key: IoKey, path: PathBuf, pending: Arc<Pending>) {
        let guard = Completion {
            key,
            pending,
            inflight: Arc::clone(&self.inflight),
            answer: None,
            ran: false,
        };
        let ops = Arc::clone(&self.ops);
        let list_cap = self.list_cap;
        let job_path = path.clone();
        let job = Box::new(move || {
            let mut guard = guard;
            guard.ran = true;
            guard.answer = Some(run_op(&*ops, &guard.key, &job_path, list_cap));
        });
        if let Err(e) = self.spawn.spawn(WORKER_NAME, job) {
            tracing::warn!(?path, "폴더 IO: 작업 스레드를 띄우지 못했다: {e}");
        }
    }
}

fn run_op(ops: &dyn FolderOps, key: &IoKey, path: &Path, list_cap: usize) -> IoAnswer {
    match key {
        IoKey::Stat(_) => IoAnswer::Stat(ops.stat(path)),
        IoKey::List(_) => IoAnswer::List(ops.list(path, list_cap)),
        IoKey::Canonical(_) => IoAnswer::Canonical(ops.canonical(path).map_err(|e| status_of(&e))),
        IoKey::Read(_) => IoAnswer::Read(ops.read(path, READ_CAP).map_err(|e| ReadFail {
            kind: e.kind(),
            text: e.to_string(),
        })),
    }
}

/// The answer of a key whose op gave none — `reason` as the status text, or as a read error of `read_kind`.
fn unanswered(key: &IoKey, reason: &str, read_kind: io::ErrorKind) -> IoAnswer {
    let status = FolderStatus::Unreadable(reason.to_owned());
    match key {
        IoKey::Stat(_) => IoAnswer::Stat(status),
        IoKey::List(_) => IoAnswer::List(Err(status)),
        IoKey::Canonical(_) => IoAnswer::Canonical(Err(status)),
        IoKey::Read(_) => IoAnswer::Read(Err(ReadFail {
            kind: read_kind,
            text: reason.to_owned(),
        })),
    }
}

fn status_of(error: &io::Error) -> FolderStatus {
    if error.kind() == io::ErrorKind::NotFound {
        FolderStatus::Missing
    } else {
        FolderStatus::Unreadable(error.to_string())
    }
}

/// Production folder ops: std + `dunce`; a file read retries a briefly held file as the store does (`store::OS_HOOKS`).
///
/// The listing walks depth-first with std `read_dir` per level — not walkdir's `sort_by`, which collects a whole level
/// before the walk can count it (TRD S25 step-7 premise 52). Each level reads at most the entries left under the cap
/// (plus one, to tell a cut from an exact end), then sorts what it read: files (and links) first, then folders, each
/// by name — so a folder's own files and its dot folders come before `node_modules`. A link (file or folder,
/// junctions included) is listed as [`FolderEntry::UnenteredLink`] and never entered; the listed folder itself may be
/// one. Past the cap the walk stops and marks the listing truncated (a level cut part-way is sorted over what was
/// read). If the listed folder itself cannot be read (at open or part-way), the whole listing fails as `Unreadable`
/// (ADR-0341 — an unreadable folder entry blocks). Below it, a subfolder whose read fails (at open or part-way —
/// entries already read are kept) or an entry whose type cannot be read is left out and named in
/// [`FolderListing::skipped`], and the walk goes on; one warning per listing names the first item left out and the
/// count.
pub struct StdFolderOps;

impl FolderOps for StdFolderOps {
    fn stat(&self, p: &Path) -> FolderStatus {
        match fs::metadata(p) {
            Ok(meta) if meta.is_dir() => FolderStatus::Ok,
            Ok(_) => FolderStatus::NotADirectory,
            Err(e) => status_of(&e),
        }
    }

    fn list(&self, p: &Path, cap: usize) -> Result<FolderListing, FolderStatus> {
        match self.stat(p) {
            FolderStatus::Ok => {}
            other => return Err(other),
        }
        walk(p, cap, &mut read_level).map_err(|e| FolderStatus::Unreadable(e.to_string()))
    }

    fn canonical(&self, p: &Path) -> io::Result<PathBuf> {
        dunce::canonicalize(p)
    }

    fn read(&self, p: &Path, cap: u64) -> io::Result<String> {
        let mut text = String::new();
        (OS_HOOKS.retry)(&mut || {
            text = read_capped(fs::File::open(p)?, cap)?;
            Ok(())
        })?;
        Ok(text)
    }
}

/// Production spawner: one named std thread per job.
pub struct ThreadSpawn;

impl Spawn for ThreadSpawn {
    fn spawn(&self, name: &str, job: Box<dyn FnOnce() + Send>) -> io::Result<()> {
        std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(job)
            .map(drop)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryKind {
    File,
    Link,
    Folder,
}

struct LevelEntry {
    name: OsString,
    kind: EntryKind,
}

/// One level's entries as read: `Err` = the level's read failed there; an entry whose type could not be read carries
/// its name with an `Err` kind.
type Level = Box<dyn Iterator<Item = io::Result<(OsString, io::Result<EntryKind>)>>>;

fn read_level(dir: &Path) -> io::Result<Level> {
    Ok(Box::new(fs::read_dir(dir)?.map(|entry| {
        let entry = entry?;
        let kind = entry.file_type().map(|file_type| {
            if file_type.is_symlink() {
                EntryKind::Link
            } else if file_type.is_dir() {
                EntryKind::Folder
            } else {
                EntryKind::File
            }
        });
        Ok((entry.file_name(), kind))
    })))
}

/// The walk of [`StdFolderOps::list`] over a level reader (tests count the entries it reads).
fn walk(
    root: &Path,
    cap: usize,
    read_level: &mut dyn FnMut(&Path) -> io::Result<Level>,
) -> io::Result<FolderListing> {
    let mut entries = Vec::new();
    let mut remaining = cap;
    let mut cut = false;
    let mut skipped: Vec<PathBuf> = Vec::new();
    let mut first_error: Option<io::Error> = None;
    let mut folders = vec![PathBuf::new()];
    while let Some(rel) = folders.pop() {
        let top = rel.as_os_str().is_empty();
        let dir = root.join(&rel);
        let mut level = match read_level(&dir) {
            Ok(level) => level,
            Err(e) if top => return Err(e),
            Err(e) => {
                first_error.get_or_insert(e);
                skipped.push(rel);
                continue;
            }
        };
        let mut read = Vec::new();
        // Entries whose type could not be read count toward the cap too, so the walk stays bounded.
        let mut taken = 0;
        while taken < remaining {
            match level.next() {
                Some(Ok((name, Ok(kind)))) => read.push(LevelEntry { name, kind }),
                Some(Ok((name, Err(e)))) => {
                    first_error.get_or_insert(e);
                    skipped.push(rel.join(name));
                }
                Some(Err(e)) if top => return Err(e),
                // A failed level read is not resumed — a read_dir that failed may fail again on every call.
                Some(Err(e)) => {
                    first_error.get_or_insert(e);
                    skipped.push(rel.clone());
                    break;
                }
                None => break,
            }
            taken += 1;
        }
        remaining -= taken;
        // One look past the cap tells a full stop from a level that ended exactly there.
        if remaining == 0 && level.next().is_some() {
            cut = true;
        }
        read.sort_by(|a, b| {
            (a.kind == EntryKind::Folder, &a.name).cmp(&(b.kind == EntryKind::Folder, &b.name))
        });
        let mut subfolders = Vec::new();
        for entry in read {
            let path = rel.join(&entry.name);
            match entry.kind {
                EntryKind::File => entries.push(FolderEntry::File(path)),
                EntryKind::Link => entries.push(FolderEntry::UnenteredLink(path)),
                EntryKind::Folder => subfolders.push(path),
            }
        }
        if cut {
            break;
        }
        folders.extend(subfolders.into_iter().rev());
    }
    // Every skip records its error, so the first error belongs to the first skipped path.
    if let Some(error) = first_error {
        let path = root.join(&skipped[0]);
        let count = skipped.len();
        tracing::warn!(
            ?path,
            count,
            error = %error,
            "폴더 목록: 읽을 수 없는 하위 폴더 · 항목을 건너뛰었다"
        );
    }
    Ok(FolderListing {
        entries,
        truncated: cut,
        skipped: skipped.iter().map(|path| slash_path(path)).collect(),
    })
}

/// `path`'s components joined with `/` — the same spelling on every OS.
fn slash_path(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::thread;

    use engram_dashboard_platform::fs::link_dir;

    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn p(text: &str) -> PathBuf {
        PathBuf::from(text)
    }

    fn k(text: &str) -> PathKey {
        path_key(Path::new(text))
    }

    fn req(text: &str) -> (PathBuf, Need) {
        (p(text), Need::Required)
    }

    fn adv(text: &str) -> (PathBuf, Need) {
        (p(text), Need::Advisory)
    }

    fn unreadable(text: &str) -> FolderStatus {
        FolderStatus::Unreadable(text.to_owned())
    }

    fn left(io: &LaunchIo) -> Duration {
        *lock(&io.budget.left)
    }

    fn advisory_left(io: &LaunchIo) -> Duration {
        *lock(&io.budget.advisory_left)
    }

    /// Runs `f` on its own thread and fails the test if it does not finish in 20 s — a lock-order bug shows as a
    /// failure, not a hung run.
    fn within<F: FnOnce() + Send + 'static>(f: F) {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            f();
            let _ = tx.send(());
        });
        rx.recv_timeout(Duration::from_secs(20))
            .expect("the test body hung or panicked");
    }

    // ── the gate-controlled fake ops ──────────────────────────────────────────────────────────────────────

    #[derive(Default)]
    struct FakeState {
        held: HashSet<PathBuf>,
        /// Every op call in start order.
        calls: Vec<(&'static str, PathBuf)>,
        missing: HashSet<PathBuf>,
        files: HashMap<PathBuf, String>,
        listings: HashMap<PathBuf, FolderListing>,
        panics: HashSet<PathBuf>,
    }

    #[derive(Default)]
    struct FakeOps {
        state: Mutex<FakeState>,
        changed: Condvar,
        /// Called with every op as it starts, before the call is recorded.
        probe: Mutex<Option<Probe>>,
    }

    type Probe = Box<dyn Fn(&'static str, &Path) + Send + Sync>;

    impl FakeOps {
        fn hold(&self, path: &str) {
            lock(&self.state).held.insert(p(path));
        }

        fn release(&self, path: &str) {
            lock(&self.state).held.remove(&p(path));
            self.changed.notify_all();
        }

        fn set_probe(&self, probe: Probe) {
            *lock(&self.probe) = Some(probe);
        }

        fn set_file(&self, path: &str, text: &str) {
            lock(&self.state).files.insert(p(path), text.to_owned());
        }

        fn count(&self, op: &str, path: &str) -> usize {
            lock(&self.state)
                .calls
                .iter()
                .filter(|(o, c)| *o == op && *c == p(path))
                .count()
        }

        /// Waits until `op` on `path` has started `n` times (fails after 10 s).
        fn wait_started(&self, op: &str, path: &str, n: usize) {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut state = lock(&self.state);
            while state
                .calls
                .iter()
                .filter(|(o, c)| *o == op && *c == p(path))
                .count()
                < n
            {
                let now = Instant::now();
                assert!(now < deadline, "{op} {path} did not start");
                state = self
                    .changed
                    .wait_timeout(state, deadline - now)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
        }

        /// Runs the probe and records the call; `gate` then blocks while `path` is held (at most 10 s — a stuck gate
        /// fails the op loudly).
        fn enter(&self, op: &'static str, path: &Path) -> std::sync::MutexGuard<'_, FakeState> {
            if let Some(probe) = lock(&self.probe).as_ref() {
                probe(op, path);
            }
            let mut state = lock(&self.state);
            state.calls.push((op, path.to_path_buf()));
            self.changed.notify_all();
            state
        }

        fn gate<'a>(
            &'a self,
            mut state: std::sync::MutexGuard<'a, FakeState>,
            path: &Path,
        ) -> std::sync::MutexGuard<'a, FakeState> {
            let deadline = Instant::now() + Duration::from_secs(10);
            while state.held.contains(path) {
                let now = Instant::now();
                assert!(now < deadline, "gate on {path:?} never released");
                state = self
                    .changed
                    .wait_timeout(state, deadline - now)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
            if state.panics.contains(path) {
                drop(state);
                panic!("fake op panics on purpose");
            }
            state
        }
    }

    impl FolderOps for FakeOps {
        fn stat(&self, path: &Path) -> FolderStatus {
            let state = self.gate(self.enter("stat", path), path);
            if state.missing.contains(path) {
                FolderStatus::Missing
            } else {
                FolderStatus::Ok
            }
        }

        fn list(&self, path: &Path, _cap: usize) -> Result<FolderListing, FolderStatus> {
            let state = self.gate(self.enter("list", path), path);
            Ok(state.listings.get(path).cloned().unwrap_or(FolderListing {
                entries: Vec::new(),
                truncated: false,
                skipped: Vec::new(),
            }))
        }

        fn canonical(&self, path: &Path) -> io::Result<PathBuf> {
            let _state = self.gate(self.enter("canonical", path), path);
            Ok(PathBuf::from(format!("{}#canonical", path.display())))
        }

        fn read(&self, path: &Path, _cap: u64) -> io::Result<String> {
            let state = self.enter("read", path);
            // The bytes are taken when the read starts — a held read models one that began before a replacement.
            let text = state.files.get(path).cloned();
            let _state = self.gate(state, path);
            text.ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
    }

    struct CountingSpawn(AtomicUsize);

    impl Spawn for CountingSpawn {
        fn spawn(&self, name: &str, job: Box<dyn FnOnce() + Send>) -> io::Result<()> {
            self.0.fetch_add(1, Ordering::SeqCst);
            ThreadSpawn.spawn(name, job)
        }
    }

    struct FailingSpawn;

    impl Spawn for FailingSpawn {
        fn spawn(&self, _name: &str, job: Box<dyn FnOnce() + Send>) -> io::Result<()> {
            drop(job);
            Err(io::Error::other("no threads left"))
        }
    }

    fn bounded(ops: &Arc<FakeOps>, limit: Duration, waits: LaunchWaits) -> Arc<BoundedIo> {
        let ops: Arc<dyn FolderOps> = Arc::clone(ops) as Arc<dyn FolderOps>;
        Arc::new(BoundedIo::new(
            limit,
            waits,
            LIST_CAP,
            ops,
            Arc::new(ThreadSpawn),
        ))
    }

    fn waits(required: Duration, advisory: Duration) -> LaunchWaits {
        LaunchWaits { required, advisory }
    }

    /// Fails unless every worker has removed its entry within 10 s.
    fn assert_inflight_empty(io: &BoundedIo) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !lock(&io.inflight).is_empty() {
            assert!(Instant::now() < deadline, "in-flight entries left behind");
            thread::sleep(ms(5));
        }
    }

    /// Waits until `waiters` asks hold the pending call of `key` (fails after 10 s). The map entry and the worker's
    /// guard hold one reference each, every waiting ask one more.
    fn wait_joined(io: &BoundedIo, key: &IoKey, waiters: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let held = lock(&io.inflight).get(key).map(Arc::strong_count);
            if held.is_some_and(|count| count >= 2 + waiters) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{key:?} never got {waiters} waiters"
            );
            thread::sleep(ms(5));
        }
    }

    /// `stat_all` of `paths` on `launch`, on its own thread.
    fn stat_on(
        io: &Arc<BoundedIo>,
        launch: &Arc<LaunchIo>,
        paths: Vec<(PathBuf, Need)>,
    ) -> thread::JoinHandle<HashMap<PathKey, FolderStatus>> {
        let (io, launch) = (Arc::clone(io), Arc::clone(launch));
        thread::spawn(move || io.stat_all(&launch, &paths))
    }

    // ── values ────────────────────────────────────────────────────────────────────────────────────────────

    #[test]
    fn the_launch_waits_are_agents_launch_folder_waits() {
        assert_eq!(LaunchWaits::LAUNCH.required, Duration::from_secs(60));
        assert_eq!(LaunchWaits::LAUNCH.advisory, Duration::from_secs(2));
    }

    #[test]
    fn begin_launch_and_begin_read_hand_out_fresh_budgets() {
        let ops = Arc::new(FakeOps::default());
        let io = bounded(&ops, ms(700), waits(ms(9_000), ms(300)));
        let launch = io.begin_launch();
        assert_eq!(
            (left(&launch), advisory_left(&launch)),
            (ms(9_000), ms(300))
        );
        let read = io.begin_read();
        assert_eq!(
            (left(&read), advisory_left(&read)),
            (ms(700), READ_ADVISORY_WAIT)
        );
    }

    // ── answers through the worker ────────────────────────────────────────────────────────────────────────

    #[test]
    fn each_op_answers_through_a_worker_and_a_listing_answers() {
        let ops = Arc::new(FakeOps::default());
        let listing = FolderListing {
            entries: vec![FolderEntry::File(p("AGENTS.md"))],
            truncated: false,
            skipped: Vec::new(),
        };
        lock(&ops.state).listings.insert(p("/f"), listing.clone());
        ops.set_file("/f/AGENTS.md", "text");
        let spawn = Arc::new(CountingSpawn(AtomicUsize::new(0)));
        let dyn_ops: Arc<dyn FolderOps> = Arc::clone(&ops) as Arc<dyn FolderOps>;
        let io = BoundedIo::new(
            ms(5_000),
            waits(ms(5_000), ms(1_000)),
            LIST_CAP,
            dyn_ops,
            Arc::clone(&spawn) as Arc<dyn Spawn>,
        );
        let launch = io.begin_launch();

        assert_eq!(
            io.stat_all(&launch, &[req("/f")])[&k("/f")],
            FolderStatus::Ok
        );
        assert_eq!(io.list_all(&launch, &[req("/f")])[&k("/f")], Ok(listing));
        assert_eq!(
            io.canonical_all(&launch, &[adv("/f")])[&k("/f")],
            Ok(p("/f#canonical"))
        );
        assert_eq!(
            io.read_one(&launch, Path::new("/f/AGENTS.md"), Need::Required, false)
                .unwrap(),
            "text"
        );
        assert_eq!(spawn.0.load(Ordering::SeqCst), 4, "one worker per op");
        assert_inflight_empty(&io);
    }

    #[test]
    fn a_missing_answer_dies_with_its_launch() {
        let ops = Arc::new(FakeOps::default());
        lock(&ops.state).missing.insert(p("/f"));
        let io = bounded(&ops, ms(5_000), waits(ms(5_000), ms(1_000)));
        let first = io.begin_launch();
        assert_eq!(
            io.stat_all(&first, &[req("/f")])[&k("/f")],
            FolderStatus::Missing
        );

        lock(&ops.state).missing.remove(&p("/f"));
        assert_eq!(
            io.stat_all(&first, &[req("/f")])[&k("/f")],
            FolderStatus::Missing,
            "the same launch reuses its answer"
        );
        assert_eq!(ops.count("stat", "/f"), 1);
        // The worker removes its entry after it answers; until then a new ask would join the finished call.
        assert_inflight_empty(&io);
        let next = io.begin_launch();
        assert_eq!(io.stat_all(&next, &[req("/f")])[&k("/f")], FolderStatus::Ok);
        assert_eq!(ops.count("stat", "/f"), 2);
        assert_inflight_empty(&io);
    }

    #[test]
    fn an_answer_is_reused_within_a_launch() {
        let ops = Arc::new(FakeOps::default());
        ops.set_file("/f/a.md", "one");
        let io = bounded(&ops, ms(5_000), waits(ms(5_000), ms(1_000)));
        let launch = io.begin_launch();
        let path = Path::new("/f/a.md");
        assert_eq!(
            io.read_one(&launch, path, Need::Required, false).unwrap(),
            "one"
        );
        ops.set_file("/f/a.md", "two");
        assert_eq!(
            io.read_one(&launch, path, Need::Required, false).unwrap(),
            "one"
        );
        assert_eq!(ops.count("read", "/f/a.md"), 1);
        assert_inflight_empty(&io);
    }

    #[test]
    fn a_fresh_read_asks_again() {
        let ops = Arc::new(FakeOps::default());
        ops.set_file("/f/a.md", "one");
        let io = bounded(&ops, ms(5_000), waits(ms(5_000), ms(1_000)));
        let launch = io.begin_launch();
        let path = Path::new("/f/a.md");
        assert_eq!(
            io.read_one(&launch, path, Need::Required, false).unwrap(),
            "one"
        );
        ops.set_file("/f/a.md", "two");
        assert_eq!(
            io.read_one(&launch, path, Need::Required, true).unwrap(),
            "two"
        );
        assert_eq!(ops.count("read", "/f/a.md"), 2);
        assert_inflight_empty(&io);
    }

    #[test]
    fn a_fresh_read_never_joins_a_pending_reader() {
        let ops = Arc::new(FakeOps::default());
        ops.set_file("/f/a.md", "before");
        ops.hold("/f/a.md");
        let io = bounded(&ops, ms(5_000), waits(ms(5_000), ms(1_000)));

        let early = {
            let io = Arc::clone(&io);
            thread::spawn(move || {
                let launch = io.begin_launch();
                io.read_one(&launch, Path::new("/f/a.md"), Need::Required, false)
            })
        };
        ops.wait_started("read", "/f/a.md", 1);
        // An atomic replacement lands while the first read is held.
        ops.set_file("/f/a.md", "after");
        let fresh = {
            let io = Arc::clone(&io);
            thread::spawn(move || {
                let launch = io.begin_launch();
                io.read_one(&launch, Path::new("/f/a.md"), Need::Required, true)
            })
        };
        ops.wait_started("read", "/f/a.md", 2);
        ops.release("/f/a.md");

        assert_eq!(fresh.join().unwrap().unwrap(), "after");
        assert_eq!(early.join().unwrap().unwrap(), "before");
        assert_eq!(ops.count("read", "/f/a.md"), 2);
        assert_inflight_empty(&io);
    }

    // ── one budget, charged by waiting ────────────────────────────────────────────────────────────────────

    #[test]
    fn held_keys_in_one_batch_end_when_the_budget_does() {
        let ops = Arc::new(FakeOps::default());
        for path in ["/a", "/b", "/c", "/d"] {
            ops.hold(path);
        }
        let io = bounded(&ops, ms(5_000), waits(ms(1_000), ms(1_000)));
        let launch = io.begin_launch();
        let began = Instant::now();
        let answers = io.stat_all(&launch, &[req("/a"), req("/b"), req("/c"), req("/d")]);
        let took = began.elapsed();

        for path in ["/a", "/b", "/c", "/d"] {
            assert_eq!(answers[&k(path)], unreadable(TIMEOUT), "{path}");
        }
        // One budget (1 s), not four (4 s): the loose bound is the limit + 1 s.
        assert!(
            took >= ms(1_000) && took < ms(2_000),
            "one budget, not four: {took:?}"
        );
        assert_eq!(left(&launch), Duration::ZERO);
        for path in ["/a", "/b", "/c", "/d"] {
            ops.release(path);
        }
        assert_inflight_empty(&io);
    }

    #[test]
    fn a_batch_is_charged_once_for_its_wait_not_per_key() {
        let ops = Arc::new(FakeOps::default());
        for path in ["/a", "/b", "/c", "/d"] {
            ops.hold(path);
        }
        let io = bounded(&ops, ms(5_000), waits(ms(10_000), ms(1_000)));
        let launch = io.begin_launch();
        let began = Instant::now();
        let call = stat_on(
            &io,
            &launch,
            vec![req("/a"), req("/b"), req("/c"), req("/d")],
        );
        for path in ["/a", "/b", "/c", "/d"] {
            ops.wait_started("stat", path, 1);
        }
        thread::sleep(ms(1_000));
        for path in ["/a", "/b", "/c", "/d"] {
            ops.release(path);
        }
        let answers = call.join().unwrap();
        let call_took = began.elapsed();

        assert!(answers.values().all(|status| *status == FolderStatus::Ok));
        // Four keys waited together: the charge is the batch's one wait — at least half of the 1 s held (the budget
        // clock may start a little after the test sees the op start), never more than the whole call took (four
        // charges would be about four times that).
        let charged = ms(10_000) - left(&launch);
        assert!(
            charged >= ms(500) && charged <= call_took,
            "charged {charged:?}, call took {call_took:?}"
        );
        assert_inflight_empty(&io);
    }

    #[test]
    fn two_batches_share_one_budget() {
        let ops = Arc::new(FakeOps::default());
        ops.hold("/a");
        ops.hold("/b");
        let io = bounded(&ops, ms(5_000), waits(ms(3_000), ms(1_000)));
        let launch = io.begin_launch();

        // Batch 1 is held for about half the budget, then released by the test.
        let first = stat_on(&io, &launch, vec![req("/a")]);
        ops.wait_started("stat", "/a", 1);
        thread::sleep(ms(1_500));
        ops.release("/a");
        assert_eq!(first.join().unwrap()[&k("/a")], FolderStatus::Ok);
        let rest = left(&launch);
        // At least half of the 1.5 s held is charged (the budget clock may start a little after the test sees the
        // op start); an uncharged batch would leave the whole 3 s.
        assert!(rest <= ms(2_250), "batch 1 was charged its wait: {rest:?}");

        let began = Instant::now();
        let second = io.list_all(&launch, &[req("/b")]);
        let took = began.elapsed();

        assert_eq!(second[&k("/b")], Err(unreadable(TIMEOUT)));
        assert_eq!(left(&launch), Duration::ZERO);
        // Batch 2 waits only what batch 1 left (a fresh budget would be 3 s): the loose bound is that rest + 1 s.
        assert!(
            took < rest + ms(1_000),
            "cut at the shared deadline: {took:?}, rest {rest:?}"
        );
        ops.release("/b");
        assert_inflight_empty(&io);
    }

    #[test]
    fn a_spent_budget_starts_no_worker_and_answers_not_checked() {
        let ops = Arc::new(FakeOps::default());
        let io = bounded(&ops, ms(5_000), waits(Duration::ZERO, ms(1_000)));
        let launch = io.begin_launch();

        let answers = io.stat_all(&launch, &[req("/a")]);
        assert_eq!(answers[&k("/a")], unreadable(NOT_CHECKED));
        assert_ne!(NOT_CHECKED, TIMEOUT);
        assert_eq!(ops.count("stat", "/a"), 0, "no worker started");
        assert!(lock(&io.inflight).is_empty());
    }

    #[test]
    fn time_between_batches_is_not_charged() {
        let ops = Arc::new(FakeOps::default());
        let io = bounded(&ops, ms(5_000), waits(ms(1_000), ms(1_000)));
        let launch = io.begin_launch();
        assert_eq!(
            io.stat_all(&launch, &[req("/a")])[&k("/a")],
            FolderStatus::Ok
        );

        // Work outside folder IO (a lock wait, parsing) — longer than the whole budget.
        thread::sleep(ms(1_200));
        ops.hold("/b");
        let second = stat_on(&io, &launch, vec![req("/b")]);
        ops.wait_started("stat", "/b", 1);
        ops.release("/b");
        assert_eq!(second.join().unwrap()[&k("/b")], FolderStatus::Ok);
        assert!(left(&launch) > Duration::ZERO);
        assert_inflight_empty(&io);
    }

    #[test]
    fn a_held_read_ends_a_read_sequence_inside_the_budget() {
        let ops = Arc::new(FakeOps::default());
        for n in 1..=4 {
            ops.set_file(&format!("/f/{n}.md"), "x");
        }
        ops.hold("/f/1.md");
        let io = bounded(&ops, ms(5_000), waits(ms(200), ms(1_000)));
        let launch = io.begin_launch();

        let began = Instant::now();
        let reads: Vec<io::Result<String>> = (1..=4)
            .map(|n| {
                let path = p(&format!("/f/{n}.md"));
                io.read_one(&launch, &path, Need::Required, false)
            })
            .collect();
        let took = began.elapsed();

        assert!(
            took < ms(1_200),
            "the sequence ends inside the budget (+ 1 s): {took:?}"
        );
        let first = reads[0].as_ref().unwrap_err();
        assert_eq!(
            (first.kind(), first.to_string()),
            (io::ErrorKind::TimedOut, TIMEOUT.into())
        );
        for (n, read) in reads.iter().enumerate().skip(1) {
            let error = read.as_ref().unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::TimedOut, "read {}", n + 1);
            assert_eq!(error.to_string(), NOT_CHECKED);
            assert_eq!(ops.count("read", &format!("/f/{}.md", n + 1)), 0);
        }
        ops.release("/f/1.md");
        assert_inflight_empty(&io);
    }

    #[test]
    fn a_required_timeout_is_stored_and_reused() {
        let ops = Arc::new(FakeOps::default());
        ops.hold("/k");
        let io = bounded(&ops, ms(5_000), waits(ms(100), ms(1_000)));
        let launch = io.begin_launch();
        assert_eq!(
            io.stat_all(&launch, &[req("/k")])[&k("/k")],
            unreadable(TIMEOUT)
        );
        ops.release("/k");
        assert_inflight_empty(&io);

        assert_eq!(
            io.stat_all(&launch, &[req("/k")])[&k("/k")],
            unreadable(TIMEOUT)
        );
        assert_eq!(ops.count("stat", "/k"), 1);
    }

    // ── class B: required first and alone, advisory on its own allowance ──────────────────────────────────

    #[test]
    fn a_mixed_call_runs_required_keys_first_and_alone() {
        let ops = Arc::new(FakeOps::default());
        // The required keys stay held, so the required phase can end only by timing out; the advisory op may start
        // only after that — when the phase has stored its answers in the launch.
        ops.hold("/req");
        ops.hold("/both");
        let io = bounded(&ops, ms(5_000), waits(ms(500), ms(1_000)));
        let launch = io.begin_launch();
        let required_done_at_advisory_start = Arc::new(Mutex::new(None));
        {
            let launch = Arc::clone(&launch);
            let seen = Arc::clone(&required_done_at_advisory_start);
            ops.set_probe(Box::new(move |_, path| {
                if path == Path::new("/adv") {
                    let stored = lock(&launch.answers);
                    *lock(&seen) = Some(
                        stored.contains_key(&IoKey::Stat(k("/req")))
                            && stored.contains_key(&IoKey::Stat(k("/both"))),
                    );
                }
            }));
        }
        let answers = io.stat_all(
            &launch,
            &[adv("/adv"), req("/req"), adv("/both"), req("/both")],
        );

        assert_eq!(
            *lock(&required_done_at_advisory_start),
            Some(true),
            "the advisory op started only after the required phase finished"
        );
        assert_eq!(answers.len(), 3);
        assert_eq!(answers[&k("/req")], unreadable(TIMEOUT));
        assert_eq!(
            answers[&k("/both")],
            unreadable(TIMEOUT),
            "asked as required"
        );
        assert_eq!(answers[&k("/adv")], FolderStatus::Ok);
        assert_eq!(
            ops.count("stat", "/both"),
            1,
            "a key asked both ways is asked once, as required"
        );
        ops.release("/req");
        ops.release("/both");
        assert_inflight_empty(&io);
    }

    #[test]
    fn advisory_batches_share_one_allowance_and_never_reduce_the_required_budget() {
        let ops = Arc::new(FakeOps::default());
        ops.hold("/x");
        ops.hold("/y");
        let io = bounded(&ops, ms(5_000), waits(ms(5_000), ms(3_000)));
        let launch = io.begin_launch();

        // Advisory batch 1 is held for about half the allowance, then released by the test.
        let first = stat_on(&io, &launch, vec![adv("/x")]);
        ops.wait_started("stat", "/x", 1);
        thread::sleep(ms(1_500));
        ops.release("/x");
        assert_eq!(first.join().unwrap()[&k("/x")], FolderStatus::Ok);
        let rest = advisory_left(&launch);
        // At least half of the 1.5 s held is charged (the budget clock may start a little after the test sees the
        // op start); an uncharged batch would leave the whole 3 s.
        assert!(rest <= ms(2_250), "batch 1 was charged its wait: {rest:?}");

        let began = Instant::now();
        let second = io.stat_all(&launch, &[adv("/y")]);
        let took = began.elapsed();

        assert_eq!(second[&k("/y")], unreadable(TIMEOUT));
        assert_eq!(advisory_left(&launch), Duration::ZERO);
        // Batch 2 waits only what batch 1 left (a fresh allowance would be 3 s): the loose bound is that rest + 1 s.
        assert!(
            took < rest + ms(1_000),
            "the second batch gets only what the first left: {took:?}, rest {rest:?}"
        );
        assert_eq!(
            left(&launch),
            ms(5_000),
            "advisory waiting never touches the required budget"
        );
        ops.release("/y");
        assert_inflight_empty(&io);
    }

    #[test]
    fn a_spent_advisory_allowance_starts_no_worker() {
        let ops = Arc::new(FakeOps::default());
        let io = bounded(&ops, ms(5_000), waits(ms(5_000), Duration::ZERO));
        let launch = io.begin_launch();

        assert_eq!(
            io.stat_all(&launch, &[adv("/z")])[&k("/z")],
            unreadable(NOT_CHECKED)
        );
        assert_eq!(ops.count("stat", "/z"), 0);
        assert_eq!(
            io.stat_all(&launch, &[req("/z")])[&k("/z")],
            FolderStatus::Ok
        );
        assert_eq!(ops.count("stat", "/z"), 1);
        assert_inflight_empty(&io);
    }

    #[test]
    fn a_required_ask_joins_the_worker_an_advisory_miss_left_running() {
        let ops = Arc::new(FakeOps::default());
        ops.hold("/k");
        let io = bounded(&ops, ms(5_000), waits(ms(5_000), ms(100)));
        let launch = io.begin_launch();
        assert_eq!(
            io.stat_all(&launch, &[adv("/k")])[&k("/k")],
            unreadable(TIMEOUT)
        );

        let required = stat_on(&io, &launch, vec![req("/k")]);
        wait_joined(&io, &IoKey::Stat(k("/k")), 1);
        ops.release("/k");
        assert_eq!(required.join().unwrap()[&k("/k")], FolderStatus::Ok);
        assert_eq!(ops.count("stat", "/k"), 1);
        assert_inflight_empty(&io);
    }

    #[test]
    fn a_required_ask_asks_again_when_an_advisory_miss_left_no_worker() {
        let ops = Arc::new(FakeOps::default());
        ops.hold("/k");
        let io = bounded(&ops, ms(5_000), waits(ms(5_000), ms(100)));
        let launch = io.begin_launch();
        assert_eq!(
            io.stat_all(&launch, &[adv("/k")])[&k("/k")],
            unreadable(TIMEOUT)
        );
        ops.release("/k");
        assert_inflight_empty(&io);

        assert_eq!(
            io.stat_all(&launch, &[req("/k")])[&k("/k")],
            FolderStatus::Ok
        );
        assert_eq!(ops.count("stat", "/k"), 2);
        assert_inflight_empty(&io);
    }

    #[test]
    fn an_aged_worker_still_serves_a_required_ask() {
        let ops = Arc::new(FakeOps::default());
        ops.hold("/k");
        let io = bounded(&ops, ms(200), waits(ms(5_000), ms(50)));
        let launch = io.begin_launch();
        assert_eq!(
            io.stat_all(&launch, &[adv("/k")])[&k("/k")],
            unreadable(TIMEOUT)
        );

        // Non-IO work carries the worker past the one-limit cutoff; the required budget is untouched.
        thread::sleep(ms(250));
        assert_eq!(left(&launch), ms(5_000));
        let required = stat_on(&io, &launch, vec![req("/k")]);
        wait_joined(&io, &IoKey::Stat(k("/k")), 1);
        ops.release("/k");
        assert_eq!(required.join().unwrap()[&k("/k")], FolderStatus::Ok);
        assert_eq!(
            ops.count("stat", "/k"),
            1,
            "the required ask joined the aged worker"
        );
        assert_inflight_empty(&io);
    }

    #[test]
    fn an_advisory_ask_does_not_join_a_worker_older_than_one_limit() {
        let ops = Arc::new(FakeOps::default());
        ops.hold("/k");
        let io = bounded(&ops, ms(200), waits(ms(5_000), ms(50)));
        let first = io.begin_launch();
        assert_eq!(
            io.stat_all(&first, &[adv("/k")])[&k("/k")],
            unreadable(TIMEOUT)
        );
        thread::sleep(ms(250));

        let next = io.begin_launch();
        assert_eq!(
            io.stat_all(&next, &[adv("/k")])[&k("/k")],
            unreadable(NOT_CHECKED)
        );
        assert_eq!(
            advisory_left(&next),
            ms(50),
            "answered at once — no advisory time spent"
        );
        assert_eq!(ops.count("stat", "/k"), 1, "no second op");
        ops.release("/k");
        assert_inflight_empty(&io);
    }

    #[test]
    fn an_advisory_ask_joins_a_worker_younger_than_one_limit() {
        let ops = Arc::new(FakeOps::default());
        ops.hold("/k");
        let io = bounded(&ops, ms(10_000), waits(ms(10_000), ms(10_000)));
        let required = stat_on(&io, &io.begin_launch(), vec![req("/k")]);
        ops.wait_started("stat", "/k", 1);
        let advisory = stat_on(&io, &io.begin_launch(), vec![adv("/k")]);
        wait_joined(&io, &IoKey::Stat(k("/k")), 2);
        ops.release("/k");

        assert_eq!(advisory.join().unwrap()[&k("/k")], FolderStatus::Ok);
        assert_eq!(required.join().unwrap()[&k("/k")], FolderStatus::Ok);
        assert_eq!(ops.count("stat", "/k"), 1, "the advisory ask joined");
        assert_inflight_empty(&io);
    }

    // ── workers: joiners, failure, panic, recovery ────────────────────────────────────────────────────────

    #[test]
    fn a_joiner_gets_the_pending_answer() {
        let ops = Arc::new(FakeOps::default());
        ops.hold("/k");
        let io = bounded(&ops, ms(5_000), waits(ms(5_000), ms(1_000)));
        let ask = |io: &Arc<BoundedIo>| {
            let io = Arc::clone(io);
            thread::spawn(move || {
                let launch = io.begin_launch();
                io.stat_all(&launch, &[req("/k")])[&k("/k")].clone()
            })
        };
        let first = ask(&io);
        ops.wait_started("stat", "/k", 1);
        let joiner = ask(&io);
        wait_joined(&io, &IoKey::Stat(k("/k")), 2);
        ops.release("/k");

        assert_eq!(first.join().unwrap(), FolderStatus::Ok);
        assert_eq!(joiner.join().unwrap(), FolderStatus::Ok);
        assert_eq!(ops.count("stat", "/k"), 1);
        assert_inflight_empty(&io);
    }

    #[test]
    fn a_spawn_failure_answers_and_removes_the_entry() {
        within(|| {
            let ops = Arc::new(FakeOps::default());
            let dyn_ops: Arc<dyn FolderOps> = Arc::clone(&ops) as Arc<dyn FolderOps>;
            let io = BoundedIo::new(
                ms(5_000),
                waits(ms(5_000), ms(1_000)),
                LIST_CAP,
                dyn_ops,
                Arc::new(FailingSpawn),
            );
            let launch = io.begin_launch();
            let answer = io.stat_all(&launch, &[req("/k")])[&k("/k")].clone();

            assert!(
                matches!(&answer, FolderStatus::Unreadable(text) if text.starts_with("spawn failed")),
                "{answer:?}"
            );
            assert_eq!(ops.count("stat", "/k"), 0);
            assert!(lock(&io.inflight).is_empty());
        });
    }

    #[test]
    fn the_guard_fills_the_answer_before_it_takes_the_inflight_lock() {
        within(|| {
            let ops = Arc::new(FakeOps::default());
            ops.hold("/k");
            let io = bounded(&ops, ms(5_000), waits(ms(10_000), ms(1_000)));
            let asker = {
                let io = Arc::clone(&io);
                thread::spawn(move || {
                    let launch = io.begin_launch();
                    io.stat_all(&launch, &[req("/k")])[&k("/k")].clone()
                })
            };
            ops.wait_started("stat", "/k", 1);

            let inflight = lock(&io.inflight);
            let pending = Arc::clone(&inflight[&IoKey::Stat(k("/k"))]);
            ops.release("/k");
            // While this thread holds `inflight`, the guard must still fill the answer and wake its waiters.
            let answer = pending.wait_until(Instant::now() + Duration::from_secs(10));
            assert!(matches!(answer, Some(IoAnswer::Stat(FolderStatus::Ok))));
            drop(inflight);

            assert_eq!(asker.join().unwrap(), FolderStatus::Ok);
            assert_inflight_empty(&io);
        });
    }

    #[test]
    fn a_worker_panic_answers_wakes_waiters_and_removes_the_entry() {
        within(|| {
            let ops = Arc::new(FakeOps::default());
            ops.hold("/k");
            lock(&ops.state).panics.insert(p("/k"));
            let io = bounded(&ops, ms(5_000), waits(ms(10_000), ms(1_000)));
            let ask = |io: &Arc<BoundedIo>| {
                let io = Arc::clone(io);
                thread::spawn(move || {
                    let launch = io.begin_launch();
                    io.stat_all(&launch, &[req("/k")])[&k("/k")].clone()
                })
            };
            let first = ask(&io);
            ops.wait_started("stat", "/k", 1);
            let joiner = ask(&io);
            wait_joined(&io, &IoKey::Stat(k("/k")), 2);
            ops.release("/k");

            assert_eq!(first.join().unwrap(), unreadable(WORKER_PANICKED));
            assert_eq!(joiner.join().unwrap(), unreadable(WORKER_PANICKED));
            assert_eq!(ops.count("stat", "/k"), 1);
            assert_inflight_empty(&io);
        });
    }

    #[test]
    fn an_abandoned_call_is_removed_when_it_completes_and_the_next_ask_starts_fresh() {
        let ops = Arc::new(FakeOps::default());
        ops.hold("/k");
        let io = bounded(&ops, ms(5_000), waits(ms(100), ms(1_000)));
        let first = io.begin_launch();
        assert_eq!(
            io.stat_all(&first, &[req("/k")])[&k("/k")],
            unreadable(TIMEOUT)
        );
        assert_eq!(
            lock(&io.inflight).len(),
            1,
            "the abandoned call is still running"
        );

        ops.release("/k");
        assert_inflight_empty(&io);
        let next = io.begin_launch();
        assert_eq!(io.stat_all(&next, &[req("/k")])[&k("/k")], FolderStatus::Ok);
        assert_eq!(ops.count("stat", "/k"), 2);
        assert_inflight_empty(&io);
    }

    // ── the production listing ────────────────────────────────────────────────────────────────────────────

    fn names(listing: &FolderListing) -> Vec<String> {
        listing
            .entries
            .iter()
            .map(|entry| match entry {
                FolderEntry::File(path) => path.to_string_lossy().replace('\\', "/"),
                FolderEntry::UnenteredLink(path) => {
                    format!("link:{}", path.to_string_lossy().replace('\\', "/"))
                }
            })
            .collect()
    }

    fn touch(root: &Path, rel: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "x").unwrap();
    }

    #[test]
    fn the_real_listing_is_sorted_per_level_and_stops_at_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for n in 0..30 {
            touch(root, &format!("node_modules/f{n:02}"));
        }
        touch(root, "zz.txt");
        touch(root, "AGENTS.md");
        touch(root, ".agents/skills/x/SKILL.md");

        // Root: 4 entries read · .agents: 1 · skills: 1 · x: 1 · node_modules: the 3 left of the cap of 10. Which 3
        // of node_modules' files come first depends on the file system's enumeration order, so only their count,
        // folder and order among themselves are checked.
        let cut = StdFolderOps.list(root, 10).unwrap();
        let got = names(&cut);
        assert_eq!(
            got[..3],
            ["AGENTS.md", "zz.txt", ".agents/skills/x/SKILL.md"]
        );
        let tail = &got[3..];
        assert_eq!(tail.len(), 3, "{got:?}");
        assert!(tail.iter().all(|name| name.starts_with("node_modules/")));
        assert!(tail.windows(2).all(|pair| pair[0] < pair[1]), "{tail:?}");
        assert!(cut.truncated);

        let whole = StdFolderOps.list(root, LIST_CAP).unwrap();
        assert_eq!(whole.entries.len(), 33);
        assert!(!whole.truncated);
    }

    #[test]
    fn a_level_that_ends_exactly_at_the_cap_is_not_truncated() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "a");
        touch(dir.path(), "b");
        let listing = StdFolderOps.list(dir.path(), 2).unwrap();
        assert_eq!(names(&listing), ["a", "b"]);
        assert!(!listing.truncated);
    }

    /// One item a fake level yields.
    #[derive(Clone, Copy)]
    enum Item {
        E(&'static str, EntryKind),
        /// An entry whose type cannot be read.
        TypeFails(&'static str),
        /// The level's read fails here.
        ReadFails,
    }
    use EntryKind::{File, Folder, Link};
    use Item::{ReadFails, TypeFails, E};

    /// A level reader over a fake tree under `/r`, keyed by the folder relative to it (`""` = the listed folder); a
    /// folder absent from the tree cannot be opened.
    fn fake_tree(tree: HashMap<&'static str, Vec<Item>>) -> impl FnMut(&Path) -> io::Result<Level> {
        move |dir: &Path| {
            let rel = dir
                .strip_prefix("/r")
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let items = tree
                .get(rel.as_str())
                .ok_or_else(|| io::Error::from(io::ErrorKind::PermissionDenied))?
                .clone();
            Ok(Box::new(items.into_iter().map(|item| match item {
                E(name, kind) => Ok((OsString::from(name), Ok(kind))),
                TypeFails(name) => Ok((
                    OsString::from(name),
                    Err(io::Error::from(io::ErrorKind::PermissionDenied)),
                )),
                ReadFails => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            })) as Level)
        }
    }

    #[test]
    fn a_subfolder_that_cannot_be_opened_is_skipped_and_named() {
        let mut level = fake_tree(HashMap::from([
            (
                "",
                vec![
                    E("a.md", File),
                    E("bad", Folder),
                    E("good", Folder),
                    E("z.md", File),
                ],
            ),
            ("good", vec![E("g.md", File)]),
        ]));
        let listing = walk(Path::new("/r"), LIST_CAP, &mut level).unwrap();
        assert_eq!(names(&listing), ["a.md", "z.md", "good/g.md"]);
        assert_eq!(listing.skipped, ["bad"]);
        assert!(!listing.truncated, "a skip is not a cut");
    }

    #[test]
    fn a_part_way_failure_below_the_top_keeps_what_was_read_and_names_the_folder() {
        let mut level = fake_tree(HashMap::from([
            (
                "",
                vec![
                    E("sub", Folder),
                    E("r.md", File),
                    E("zed", Folder),
                    E("next", Folder),
                ],
            ),
            (
                "sub",
                vec![
                    E("y.md", File),
                    E("x.md", File),
                    ReadFails,
                    E("never.md", File),
                ],
            ),
            ("next", vec![E("n.md", File)]),
            ("zed", vec![E("z.md", File)]),
        ]));
        let listing = walk(Path::new("/r"), LIST_CAP, &mut level).unwrap();
        // `zed` sorts after the failing `sub`: the walk goes on past the failure.
        assert_eq!(
            names(&listing),
            ["r.md", "next/n.md", "sub/x.md", "sub/y.md", "zed/z.md"]
        );
        assert_eq!(listing.skipped, ["sub"]);
        assert!(!listing.truncated, "a skip is not a cut");
    }

    #[test]
    fn a_listed_folder_that_cannot_be_read_fails_the_listing() {
        let mut unopened = fake_tree(HashMap::new());
        assert!(walk(Path::new("/r"), LIST_CAP, &mut unopened).is_err());
        let mut cut_part_way = fake_tree(HashMap::from([(
            "",
            vec![E("a.md", File), ReadFails, E("b.md", File)],
        )]));
        assert!(walk(Path::new("/r"), LIST_CAP, &mut cut_part_way).is_err());
    }

    #[test]
    fn entries_whose_type_cannot_be_read_count_toward_the_cap() {
        let reads = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&reads);
        let mut tree = fake_tree(HashMap::from([(
            "",
            vec![
                TypeFails("x1"),
                TypeFails("x2"),
                TypeFails("x3"),
                E("a.md", File),
                E("b.md", File),
            ],
        )]));
        let mut level = move |dir: &Path| -> io::Result<Level> {
            let counter = Arc::clone(&counter);
            Ok(Box::new(tree(dir)?.inspect(move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
            })))
        };
        let listing = walk(Path::new("/r"), 3, &mut level).unwrap();

        assert!(listing.entries.is_empty(), "{:?}", names(&listing));
        assert_eq!(listing.skipped, ["x1", "x2", "x3"]);
        assert!(listing.truncated);
        assert_eq!(
            reads.load(Ordering::SeqCst),
            4,
            "the cap of 3 plus one look past it"
        );
    }

    #[test]
    fn an_entry_whose_type_cannot_be_read_is_skipped_and_named() {
        let mut level = fake_tree(HashMap::from([
            (
                "",
                vec![
                    E("b.md", File),
                    TypeFails("odd"),
                    E("a.md", File),
                    E("sub", Folder),
                ],
            ),
            ("sub", vec![TypeFails("q"), E("s.md", File)]),
        ]));
        let listing = walk(Path::new("/r"), LIST_CAP, &mut level).unwrap();
        assert_eq!(names(&listing), ["a.md", "b.md", "sub/s.md"]);
        assert_eq!(listing.skipped, ["odd", "sub/q"]);
        assert!(!listing.truncated, "a skip is not a cut");
    }

    #[test]
    fn a_listing_that_skips_and_reaches_the_cap_reports_both() {
        let mut level = fake_tree(HashMap::from([
            (
                "",
                vec![E("a.md", File), E("bad", Folder), E("big", Folder)],
            ),
            (
                "big",
                vec![E("f0", File), E("f1", File), E("f2", File), E("f3", File)],
            ),
        ]));
        // Root 3 + the first 2 of big = the cap of 5.
        let listing = walk(Path::new("/r"), 5, &mut level).unwrap();
        assert_eq!(names(&listing), ["a.md", "big/f0", "big/f1"]);
        assert_eq!(listing.skipped, ["bad"]);
        assert!(listing.truncated);
    }

    #[test]
    fn a_walk_without_failures_under_the_cap_is_not_partial() {
        let mut level = fake_tree(HashMap::from([
            ("", vec![E("d", Folder), E("a.md", File)]),
            ("d", vec![E("e.md", File)]),
        ]));
        let listing = walk(Path::new("/r"), LIST_CAP, &mut level).unwrap();
        assert_eq!(names(&listing), ["a.md", "d/e.md"]);
        assert!(!listing.truncated);
        assert!(listing.skipped.is_empty());
    }

    #[test]
    fn the_walk_sorts_each_level_files_first_then_folders_whatever_the_read_order() {
        // Each level arrives in a scrambled order; node_modules is cut part-way with its read part unsorted.
        let mut level = fake_tree(HashMap::from([
            (
                "",
                vec![
                    E("node_modules", Folder),
                    E("zz.txt", File),
                    E(".agents", Folder),
                    E("link", Link),
                    E("b.md", File),
                    E("AGENTS.md", File),
                ],
            ),
            (".agents", vec![E("z.md", File), E("a.md", File)]),
            (
                "node_modules",
                vec![
                    E("f03", File),
                    E("f01", File),
                    E("f02", File),
                    E("f00", File),
                    E("f04", File),
                ],
            ),
        ]));
        // Root 6 + .agents 2 + the first 3 of node_modules = the cap of 11.
        let listing = walk(Path::new("/r"), 11, &mut level).unwrap();

        assert_eq!(
            names(&listing),
            [
                "AGENTS.md",
                "b.md",
                "link:link",
                "zz.txt",
                ".agents/a.md",
                ".agents/z.md",
                "node_modules/f01",
                "node_modules/f02",
                "node_modules/f03"
            ]
        );
        assert!(listing.truncated);
        assert!(listing.skipped.is_empty());
    }

    #[test]
    fn a_large_level_is_read_only_up_to_the_cap() {
        let reads = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&reads);
        let mut level = move |_: &Path| -> io::Result<Level> {
            let counter = Arc::clone(&counter);
            Ok(Box::new((0..20_000).map(move |n| {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok((OsString::from(format!("f{n:05}")), Ok(EntryKind::File)))
            })))
        };
        let listing = walk(Path::new("/big"), LIST_CAP, &mut level).unwrap();

        assert_eq!(listing.entries.len(), LIST_CAP);
        assert!(listing.truncated);
        assert!(listing.skipped.is_empty());
        assert_eq!(
            reads.load(Ordering::SeqCst),
            LIST_CAP + 1,
            "the cap plus one look past it"
        );
    }

    #[test]
    fn the_real_listing_does_not_enter_links() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("folder");
        touch(&root, "inner/a.md");
        touch(dir.path(), "outside/b.md");
        link_dir(&dir.path().join("outside"), &root.join("link")).unwrap();

        let listing = StdFolderOps.list(&root, LIST_CAP).unwrap();
        assert_eq!(names(&listing), ["link:link", "inner/a.md"]);
    }

    #[test]
    fn the_real_ops_answer_stat_canonical_and_read() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "a.md");
        let file = dir.path().join("a.md");
        let gone = dir.path().join("gone");

        assert_eq!(StdFolderOps.stat(dir.path()), FolderStatus::Ok);
        assert_eq!(StdFolderOps.stat(&file), FolderStatus::NotADirectory);
        assert_eq!(StdFolderOps.stat(&gone), FolderStatus::Missing);
        assert_eq!(
            StdFolderOps.list(&gone, LIST_CAP),
            Err(FolderStatus::Missing)
        );
        assert_eq!(
            StdFolderOps.canonical(dir.path()).unwrap(),
            dunce::canonicalize(dir.path()).unwrap()
        );
        assert_eq!(StdFolderOps.read(&file, 1).unwrap(), "x");
        assert_eq!(
            StdFolderOps.read(&file, 0).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn a_canonical_os_error_is_a_status() {
        let io = BoundedIo::new(
            ms(5_000),
            waits(ms(5_000), ms(1_000)),
            LIST_CAP,
            Arc::new(StdFolderOps),
            Arc::new(ThreadSpawn),
        );
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone");
        let launch = io.begin_launch();
        let answers = io.canonical_all(&launch, &[(gone.clone(), Need::Advisory)]);
        assert_eq!(answers[&path_key(&gone)], Err(FolderStatus::Missing));
        assert_inflight_empty(&io);
    }
}
