//! **Selection rules** for cleaning up leftovers after a claude interrupt: candidates of one pass round ([`candidates`]) and the four-rule
//!   AND per candidate
//! ([`select`]). Hook command shape · taskkill markers: [`is_hook_command`] · [`taskkill_names`].
//!
//! ★The rules are pure★ — no OS calls, locks or threads; they compile on every OS (ADR-0012 · ADR-0230) and read only facts already read
//!   through held handles
//!   ([`ProcessFacts`]); 「has it exited」 is injected through [`Chain::exited`].
//! ★Narrowing only★ — anything untrustworthy (link not found · empty field · query error · unexpected shape) means do not kill. No marker
//!   ever widens the candidates;
//!   if claude changes how it wraps hooks or interrupts, everything misses (the safe side).
//!
//! Births come from the **birth record** ([`Recorder`], fed by a listener thread; [`BirthWatch`] starts port and thread once per
//!   incarnation). [`GateCell`] (turn-open
//! gate + interrupt episode, one per incarnation) lets a worker run one pass ([`run_pass`]) N seconds after the interrupt line if no turn
//!   end came; without a
//! [`Cleaner`] it only opens and closes the gate.
//! ★The gate lock is a leaf★ — while held: fields · monotonic time · atomics only; no logging · thread start · allocation or free · OS
//!   calls · other locks (one
//!   exception: the single kill call in the kill commit). What was taken out is dropped after release. Never held together with the record lock.
//! Spec = `docs/process/S21-chat-ux/trd-t40.md` §3-2 · §3-3 · §3-4 · §3-5 · §3-7.
// ADR-0262

use std::fmt;
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use engram_dashboard_base::panic::payload_text;
use engram_dashboard_base::sync;
use engram_dashboard_base::time::{Clock, SystemClock};

use crate::transport::input_queue::OnWritten;
use crate::transport::process_group::{
    Births, MemberKill, Pinned, PortEvent, ProcessFacts, ProcessGroup, RetiringSignal, GROUP_GONE,
};
use crate::transport::stdio::InterruptOut;
use crate::types::AgentId;

/// Cap on the ancestor walk — the number of links from D(A₁) up to the top T. Measured chains are four or fewer (p3t3); more than that is
///   treated as a broken chain.
pub(super) const ANCESTOR_MAX: usize = 16;

/// One link the selection looks at — a held process's number and the facts read through its handle.
///
/// - `seq` = order of the birth record · `None` = snapshot member.
/// - `killable` = if `false` the chain still connects through it but it cannot be a candidate.
#[derive(Debug, Clone, Copy)]
pub(super) struct Link<'a> {
    pub pid: u32,
    pub facts: &'a ProcessFacts,
    pub seq: Option<u64>,
    pub killable: bool,
}

/// A view over one record — the snapshot plus its births are all the 「known links」.
///
/// - `root_pid` = root of the path (`cmd /c`); the link whose recorded parent is this number is taken to be claude. `0` = unknown.
/// - `exited` = has a known link exited now; `Err` is read as 「unknown」.
pub(super) struct Chain<'a> {
    pub root_pid: u32,
    pub snapshot: &'a [Link<'a>],
    pub births: &'a [Link<'a>],
    pub exited: &'a dyn Fn(&Link<'_>) -> io::Result<bool>,
}

/// The decision of [`select`] — everything except `Cleanup` means 「do not kill」; names pair with the log reasons (TRD §3-4 · §3-8).
///
/// - `Cleanup.chain` = numbers from candidate C up to the top T (C first, T last; claude excluded).
/// - `Unknown` = empty field · exit query error · root number unknown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Verdict {
    Cleanup { chain: Vec<u32> },
    ParentAlive,
    ParentUnknown,
    NotHookCopy,
    AncestorAlive { pid: u32 },
    ChainCut,
    NotHook,
    NoTaskkill,
    Unknown,
}

/// Candidates of one pass round — births whose number is not in the write list `w` (compared by number only), with kill access held, not
///   in the snapshot,
/// and not exited (dropped if the exit query is `Err`), in ascending `seq` order. A link without `seq` is not a birth.
pub(super) fn candidates<'a>(view: &Chain<'a>, w: &[u32]) -> Vec<&'a Link<'a>> {
    let mut out: Vec<&'a Link<'a>> = view
        .births
        .iter()
        .filter(|b| {
            b.seq.is_some()
                && b.killable
                && !w.contains(&b.pid)
                && !view
                    .snapshot
                    .iter()
                    .any(|s| s.pid == b.pid && s.facts.create == b.facts.create)
                && matches!((view.exited)(b), Ok(false))
        })
        .collect();
    out.sort_by_key(|b| b.seq);
    out
}

/// Decide one candidate C by the AND of four rules — the first rule that catches it gives the decision.
///
/// 1. Parent D is a known link and has exited. 2. C is a hook copy of D (same executable; same command line, or D is a hook command).
/// 3. Every ancestor from D up to just before claude is a known link and has exited (parent creation ≤ child creation per link). 4. The
///   top T is a hook runner,
///    and this record has a `taskkill /PID <T> /T /F` launched by claude, born after T.
///
/// `exited` is called only for D and the ancestors up to just before claude.
// ADR-0262
pub(super) fn select(c: &Link<'_>, view: &Chain<'_>) -> Verdict {
    if view.root_pid == 0 {
        return Verdict::Unknown;
    }

    let Some(d) = parent_of(view, c.pid, c.facts) else {
        return Verdict::ParentUnknown;
    };
    match (view.exited)(d) {
        Ok(true) => {}
        Ok(false) => return Verdict::ParentAlive,
        Err(_) => return Verdict::Unknown,
    }

    if [c.facts, d.facts]
        .iter()
        .any(|f| f.image.is_empty() || f.cmdline.is_empty())
    {
        return Verdict::Unknown;
    }
    let hook_copy = c.facts.image == d.facts.image
        && (c.facts.cmdline == d.facts.cmdline || is_hook_command(d.facts));
    if !hook_copy {
        return Verdict::NotHookCopy;
    }

    let (walked, claude) = match ancestors(c, d, view) {
        Ok(found) => found,
        Err(verdict) => return verdict,
    };
    let Some(&top) = walked.last() else {
        return Verdict::ChainCut;
    };

    if !(image_is(&top.facts.image, "bash.exe") && is_hook_command(top.facts)) {
        return Verdict::NotHook;
    }
    let named_by_claude = view.births.iter().any(|b| {
        taskkill_names(b.facts, claude.pid) == Some(top.pid)
            && b.facts.create >= top.facts.create
            && parent_of(view, b.pid, b.facts)
                .is_some_and(|p| p.pid == claude.pid && p.facts.create == claude.facts.create)
    });
    if !named_by_claude {
        return Verdict::NoTaskkill;
    }

    Verdict::Cleanup {
        chain: std::iter::once(c.pid)
            .chain(walked.iter().map(|l| l.pid))
            .collect(),
    }
}

/// Is it a hook command shape — 「first token, the -c flag, a double-quoted argument S」 where S starts with `bash ` and does not contain
///   `shell-snapshots`
/// (how claude 2.1.284 wraps `.sh` hooks: 291 hooks vs 6 Bash tool · 0 ambiguous). False if S has a line break — the measured regex did
///   not accept it.
pub(super) fn is_hook_command(f: &ProcessFacts) -> bool {
    let Some(rest) = after_program(&f.cmdline) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix("-c") else {
        return false;
    };
    let arg = rest.trim_start();
    if arg.len() == rest.len() {
        return false;
    }
    let Some(s) = arg
        .trim_end()
        .strip_prefix('"')
        .and_then(|a| a.strip_suffix('"'))
    else {
        return false;
    };
    s.starts_with("bash ")
        && !s.contains("shell-snapshots")
        && !s.contains(|ch: char| ch == '\r' || ch == '\n')
}

/// The `n` of a `taskkill /PID n /T /F` launched by claude — parent `claude`, executable `taskkill.exe` (case-insensitive), arguments
///   exactly `/PID` · decimal number
/// · `/T` · `/F`. Case variants · `/PID:n` · reordering · extra arguments · leading zeros · `0` are `None` (all 218 spike taskkills have
///   this shape).
pub(super) fn taskkill_names(f: &ProcessFacts, claude: u32) -> Option<u32> {
    if f.ppid != claude || !image_is(&f.image, "taskkill.exe") {
        return None;
    }
    let mut args = after_program(&f.cmdline)?.split_whitespace();
    let (Some("/PID"), Some(n), Some("/T"), Some("/F"), None) = (
        args.next(),
        args.next(),
        args.next(),
        args.next(),
        args.next(),
    ) else {
        return None;
    };
    if n.starts_with('0') || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse().ok()
}

/// Find the recorded parent among the known links: number = `child.ppid`, creation not 0 and ≤ the child's; the latest creation wins (a
///   held number is not reused,
/// so normally at most one matches). Equal creation (a snapshot member re-notified): the snapshot one wins. Parent number 0 · own number ·
///   child creation 0 are unknown.
fn parent_of<'a>(view: &Chain<'a>, child_pid: u32, child: &ProcessFacts) -> Option<&'a Link<'a>> {
    if child.ppid == 0 || child.ppid == child_pid || child.create == 0 {
        return None;
    }
    let mut best: Option<&'a Link<'a>> = None;
    for link in view.snapshot.iter().chain(view.births.iter()) {
        if link.pid != child.ppid || link.facts.create == 0 || link.facts.create > child.create {
            continue;
        }
        if best.map_or(true, |b| link.facts.create > b.facts.create) {
            best = Some(link);
        }
    }
    best
}

/// Rule 3 — walk from D(A₁) along the recorded parents and return `(A₁ … Aₖ₋₁, Aₖ)`. `Aₖ` = the link whose recorded parent is the root (claude).
/// D's exit was already checked by rule 1.
fn ancestors<'a>(
    c: &Link<'_>,
    d: &'a Link<'a>,
    view: &Chain<'a>,
) -> Result<(Vec<&'a Link<'a>>, &'a Link<'a>), Verdict> {
    let mut seen = vec![(c.pid, c.facts.create)];
    let mut walked: Vec<&'a Link<'a>> = Vec::new();
    let mut a = d;
    loop {
        if a.pid == view.root_pid || seen.contains(&(a.pid, a.facts.create)) {
            return Err(Verdict::ChainCut);
        }
        seen.push((a.pid, a.facts.create));
        if a.facts.ppid == view.root_pid {
            // If k = 1 (D itself is the claude position), there is no top.
            return if walked.is_empty() {
                Err(Verdict::ChainCut)
            } else {
                Ok((walked, a))
            };
        }
        if !walked.is_empty() {
            match (view.exited)(a) {
                Ok(true) => {}
                Ok(false) => return Err(Verdict::AncestorAlive { pid: a.pid }),
                Err(_) => return Err(Verdict::Unknown),
            }
        }
        walked.push(a);
        if walked.len() > ANCESTOR_MAX {
            return Err(Verdict::ChainCut);
        }
        a = parent_of(view, a.pid, a.facts).ok_or(Verdict::ChainCut)?;
    }
}

/// The rest of the command line after skipping the first token (the program) — the first token is either double-quoted or a run without
///   spaces, and
/// at least one space must follow it.
fn after_program(cmdline: &str) -> Option<&str> {
    let rest = if let Some(quoted) = cmdline.strip_prefix('"') {
        let close = quoted.find('"')?;
        quoted.get(close + 1..)?
    } else {
        let end = cmdline.find(char::is_whitespace)?;
        if end == 0 {
            return None;
        }
        cmdline.get(end..)?
    };
    let trimmed = rest.trim_start();
    (trimmed.len() < rest.len()).then_some(trimmed)
}

/// Is the file name of the executable path `name` (ASCII case-insensitive). Both `\` · `/` count as path separators — on whichever OS
/// it runs, Windows paths are split the same way.
fn image_is(image: &str, name: &str) -> bool {
    image
        .rsplit(|ch: char| ch == '\\' || ch == '/')
        .next()
        .is_some_and(|file| file.eq_ignore_ascii_case(name))
}

// ── Birth record · listening for join notifications ──────────────────────────────────────────

/// Cap on births put into one record — those over it are not put in and [`Recorder::full`] is raised (that link stays unknown, on the miss
///   side).
pub(super) const BIRTH_MAX: usize = 512;

/// How long the listener thread waits for one notification — at each interval it checks whether the group is gone · retiring.
const LISTEN_WAIT: Duration = Duration::from_millis(500);

/// The FILETIME scale (100 ns) from 1601-01-01 to 1970-01-01.
const UNIX_EPOCH_FILETIME: u64 = 116_444_736_000_000_000;

/// One process held right when its join notification arrived while the record was on — its number is not reused while held, and its facts
///   remain after it exits.
/// Dropping the last `Arc` closes the handle — drop it outside the record lock.
///
/// - `seq` = order notifications were taken out in this incarnation (not creation order).
/// - `facts` = facts read when `pin` held it.
pub(super) struct Birth {
    pub pid: u32,
    pub seq: u64,
    pub facts: ProcessFacts,
    pub pin: Box<dyn Pinned>,
    pub killable: bool,
}

impl Birth {
    pub(super) fn link(&self) -> Link<'_> {
        Link {
            pid: self.pid,
            facts: &self.facts,
            seq: Some(self.seq),
            killable: self.killable,
        }
    }
}

/// The birth record turned on and off by an interrupt episode — one per incarnation, shared by the opener · listener thread · pass.
///
/// - Records are told apart by number: [`Self::start`] turns on a new number; only whoever holds it turns it off ([`Self::stop`]) and
///   copies it ([`Self::copy`]).
///   The active number (`0` = off) is read lock-free through [`Self::active`].
/// - [`Self::port_failed`] = the join-notification port failed for real — once raised it stays for the incarnation.
///
/// ★The record lock is a leaf★ — while held: fields · `Vec` · `Arc` clone · take-out only; no OS calls · handle closing (the last drop of
///   `Arc<Birth>`) · logging ·
/// other locks. Never held together with the gate lock; a taken-out record is dropped after release. A poisoned lock is used as it is.
// ADR-0262
pub(super) struct Recorder {
    /// Write only inside the lock — the 「is it the same record」 decision and the insert · take-out form one section.
    active: AtomicU64,
    port_failed: AtomicBool,
    inner: Mutex<RecInner>,
}

struct RecInner {
    next_rec: u64,
    next_seq: u64,
    births: Vec<Arc<Birth>>,
    full: bool,
}

struct Caught {
    pid: u32,
    facts: ProcessFacts,
    pin: Box<dyn Pinned>,
    killable: bool,
}

enum Admit {
    Recorded {
        seq: u64,
    },
    /// The record to receive it was turned off or replaced by another record.
    Stale(Caught),
    /// The same (number, creation) is already there — a nested Job's re-report · the re-notification on attach.
    Known(Caught),
    /// The cap was reached — `first` = this is the first overflow in this record.
    Full {
        caught: Caught,
        first: bool,
    },
}

impl Recorder {
    pub(super) fn new() -> Self {
        Self {
            active: AtomicU64::new(0),
            port_failed: AtomicBool::new(false),
            inner: Mutex::new(RecInner {
                next_rec: 0,
                next_seq: 0,
                births: Vec::new(),
                full: false,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, RecInner> {
        sync::lock(&self.inner)
    }

    /// Turn on a new record and give its number (not `0`) — lower the overflow flag, and take out and return what was left in the previous
    ///   record (the
    /// receiver drops it).
    pub(super) fn start(&self) -> (u64, Vec<Arc<Birth>>) {
        let mut inner = self.lock();
        let r = match inner.next_rec.wrapping_add(1) {
            0 => 1,
            r => r,
        };
        inner.next_rec = r;
        inner.full = false;
        self.active.store(r, Ordering::Release);
        (r, std::mem::take(&mut inner.births))
    }

    /// Turn off only when `r` is on, and take out and return that record (the receiver drops it). Otherwise change nothing and return an
    ///   empty one.
    pub(super) fn stop(&self, r: u64) -> Vec<Arc<Birth>> {
        if r == 0 {
            return Vec::new();
        }
        let mut inner = self.lock();
        if self.active.load(Ordering::Acquire) != r {
            return Vec::new();
        }
        self.active.store(0, Ordering::Release);
        std::mem::take(&mut inner.births)
    }

    pub(super) fn active(&self) -> u64 {
        self.active.load(Ordering::Acquire)
    }

    pub(super) fn copy(&self, r: u64) -> Vec<Arc<Birth>> {
        if r == 0 {
            return Vec::new();
        }
        let inner = self.lock();
        if self.active.load(Ordering::Acquire) != r {
            return Vec::new();
        }
        inner.births.clone()
    }

    /// In the last record turned on, there was a birth that could not be put in because the cap was reached.
    pub(super) fn full(&self) -> bool {
        self.lock().full
    }

    pub(super) fn len(&self) -> usize {
        self.lock().births.len()
    }

    pub(super) fn port_failed(&self) -> bool {
        self.port_failed.load(Ordering::Acquire)
    }

    fn fail_port(&self) {
        self.port_failed.store(true, Ordering::Release);
    }

    /// Put into record `r` — only while `r` is still on · (number, creation) is new · and within the cap.
    fn admit(&self, r: u64, caught: Caught) -> Admit {
        let mut inner = self.lock();
        if r == 0 || self.active.load(Ordering::Acquire) != r {
            return Admit::Stale(caught);
        }
        if inner
            .births
            .iter()
            .any(|b| b.pid == caught.pid && b.facts.create == caught.facts.create)
        {
            return Admit::Known(caught);
        }
        if inner.births.len() >= BIRTH_MAX {
            let first = !inner.full;
            inner.full = true;
            return Admit::Full { caught, first };
        }
        let seq = inner.next_seq;
        inner.next_seq = seq.wrapping_add(1);
        let Caught {
            pid,
            facts,
            pin,
            killable,
        } = caught;
        inner.births.push(Arc::new(Birth {
            pid,
            seq,
            facts,
            pin,
            killable,
        }));
        Admit::Recorded { seq }
    }
}

/// Log attribution — carries the agent id if the spawn has a control channel, and always the root PID (ADR-0217 — a spawn without a
///   control channel
/// has no id, so it is linked by the root).
#[derive(Debug, Clone, Copy)]
pub(super) struct LogTag {
    agent: Option<AgentId>,
    root_pid: u32,
}

impl LogTag {
    pub(super) fn new(agent: Option<AgentId>, root_pid: u32) -> Self {
        Self { agent, root_pid }
    }

    fn agent(&self) -> Option<tracing::field::DisplayValue<AgentId>> {
        self.agent.map(tracing::field::display)
    }
}

/// What the listener thread · port acquisition · opener · write confirmation ask the group — the real thing = [`ProcessGroup`]. Tests swap in
/// a fake.
pub(super) trait Group: Send + Sync + 'static {
    fn member_pids(&self) -> io::Result<Vec<u32>>;
    fn pin(&self, pid: u32, kill: bool) -> io::Result<Option<Box<dyn Pinned>>>;
    fn watch_births(&self, start: impl FnOnce(Arc<dyn Births>) -> io::Result<()>)
        -> io::Result<()>;
    fn unwatch_births(&self) -> io::Result<()>;
    fn is_gone(&self) -> bool;
    fn retiring(&self) -> RetiringSignal;
}

// A path call picks the inherent method first — it does not come back to this trait method.
impl Group for ProcessGroup {
    fn member_pids(&self) -> io::Result<Vec<u32>> {
        ProcessGroup::member_pids(self)
    }

    fn pin(&self, pid: u32, kill: bool) -> io::Result<Option<Box<dyn Pinned>>> {
        ProcessGroup::pin(self, pid, kill)
    }

    fn watch_births(
        &self,
        start: impl FnOnce(Arc<dyn Births>) -> io::Result<()>,
    ) -> io::Result<()> {
        ProcessGroup::watch_births(self, start)
    }

    fn unwatch_births(&self) -> io::Result<()> {
        ProcessGroup::unwatch_births(self)
    }

    fn is_gone(&self) -> bool {
        ProcessGroup::is_gone(self)
    }

    fn retiring(&self) -> RetiringSignal {
        ProcessGroup::retiring(self)
    }
}

/// Acquire one incarnation's join-notification port — only the first caller creates the port, starts the listener thread and attaches the
///   group; later callers
/// read that result (a second `watch_births` never reaches the group; not reliant on the OS refusing).
// ADR-0262
pub(super) struct BirthWatch {
    acquired: OnceLock<Result<(), ()>>,
}

impl BirthWatch {
    pub(super) fn new() -> Self {
        Self {
            acquired: OnceLock::new(),
        }
    }

    /// Can births be recorded — `Ok` = listener thread running and group attached. `Err` = port unusable (that episode is `Failed(port)`):
    ///   acquisition failed · group
    /// already gone · listener thread latched a failure ([`Recorder::port_failed`]).
    ///
    /// Still `Ok` after the listener ended because the group vanished or retired — a record turned on later receives no births (the miss
    ///   side). The first call holds
    /// the Job strongly during attach but does not block.
    fn ensure_with<G: Group>(
        &self,
        group: &Arc<G>,
        recorder: &Arc<Recorder>,
        tag: &LogTag,
        spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> io::Result<()>,
    ) -> Result<(), ()> {
        let acquired = *self
            .acquired
            .get_or_init(|| acquire(group, recorder, *tag, spawn));
        if recorder.port_failed() {
            return Err(());
        }
        acquired
    }
}

#[derive(Clone, Copy)]
enum Stage {
    BeforeStart,
    StartFailed,
    Started,
}

/// Port → start the listener thread → (only when the start is `Ok`) attach — so a draining thread already runs when notifications can
///   start to pile up.
/// Only a real OS failure is latched; a group already gone or an OS without groups fails only that episode.
fn acquire<G: Group>(
    group: &Arc<G>,
    recorder: &Arc<Recorder>,
    tag: LogTag,
    spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> io::Result<()>,
) -> Result<(), ()> {
    let (attached_tx, attached_rx) = mpsc::sync_channel(1);
    let mut stage = Stage::BeforeStart;
    let watched = group.watch_births(|port| {
        let listener = Listener {
            port,
            attached: attached_rx,
            group: Arc::clone(group),
            recorder: Arc::clone(recorder),
            tag,
        };
        let spawned = spawn(Box::new(move || listen(listener)));
        stage = match spawned {
            Ok(()) => Stage::Started,
            Err(_) => Stage::StartFailed,
        };
        spawned
    });
    // If there is no running thread (failure before start · start failed), there is no receiver so the send fails — there is just nobody
    //   to tell.
    let _ = attached_tx.send(watched.is_ok());
    let Err(e) = watched else {
        return Ok(());
    };
    match stage {
        Stage::BeforeStart if matches!(e.kind(), GROUP_GONE | io::ErrorKind::Unsupported) => {
            tracing::debug!(
                agent = tag.agent(),
                root_pid = tag.root_pid,
                "no group, so the join notification port was not created: {e}"
            );
            return Err(());
        }
        Stage::BeforeStart => tracing::warn!(
            agent = tag.agent(),
            root_pid = tag.root_pid,
            "could not create the join notification port — this incarnation does not clean up interrupt leftovers: {e}"
        ),
        Stage::StartFailed => tracing::warn!(
            agent = tag.agent(),
            root_pid = tag.root_pid,
            "could not start the thread listening for join notifications, so the port was not attached — this incarnation does not clean up interrupt leftovers: {e}"
        ),
        Stage::Started => tracing::warn!(
            agent = tag.agent(),
            root_pid = tag.root_pid,
            "could not attach the group to the join notification port — the listening thread ends and this incarnation does not clean up interrupt leftovers: {e}"
        ),
    }
    recorder.fail_port();
    Err(())
}

/// What the listener thread holds. It holds the group only through a neutral handle — the Job is upgraded only while the handle is being called.
struct Listener<G> {
    port: Arc<dyn Births>,
    attached: Receiver<bool>,
    group: Arc<G>,
    recorder: Arc<Recorder>,
    tag: LogTag,
}

#[derive(Clone, Copy)]
enum End {
    Gone,
    Retiring,
}

enum Failure {
    Take(io::Error),
    Panic(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Take(e) => write!(f, "take {e}"),
            Failure::Panic(message) => write!(f, "panic {message}"),
        }
    }
}

/// Listener thread body — runs for the incarnation's lifetime; join notifications arriving while the record is on are held and put in
///   right away, the rest dropped.
///
/// - Start: first receive the opener's 「did it attach」; `false` → nothing to detach, end. A gone sender (opener panicked after attaching)
///   → run as usual.
/// - End: at each notification · timeout, group gone · retiring → turn off the record that was on and end (retiring: detach first).
/// - Failure: take-out error · panic in a round → latch the port, turn off the record, detach → detached: end; not: keep taking out and
///   discarding until the end
///   above (an undrained attached port grows the kernel queue); if taking out fails too, end.
///
/// ★Each round is wrapped in `catch_unwind`, but this thread is outside the containment marker★ — the production hook ends a panic, so the
///   body must not
/// panic (no unwrap · no indexing · poison tolerated); the wrapper is a safety net for test builds without the hook.
// ADR-0262
// ADR-0292
fn listen<G: Group>(l: Listener<G>) {
    if let Ok(false) = l.attached.recv() {
        return;
    }
    let attached_at = Instant::now();
    let retiring = l.group.retiring();
    let mut draining = false;
    loop {
        let round = panic::catch_unwind(AssertUnwindSafe(|| {
            l.round(&retiring, draining, attached_at)
        }));
        let failure = match round {
            Ok(Ok(None)) => continue,
            Ok(Ok(Some(end))) => {
                l.finish(end);
                return;
            }
            Ok(Err(e)) => Failure::Take(e),
            Err(payload) => Failure::Panic(payload_text(&*payload)),
        };
        if draining {
            tracing::debug!(
                agent = l.tag.agent(),
                root_pid = l.tag.root_pid,
                "failed to drain the join notification port that could not be detached, ending the listening: {failure}"
            );
            return;
        }
        if l.fail(failure) {
            return;
        }
        draining = true;
    }
}

impl<G: Group> Listener<G> {
    /// Take out one notification, and give the reason when it is time to end. Unless merely draining, record the join. ★Do not take out while
    /// holding an upgraded Job★ — it is upgraded only when called through the handle.
    fn round(
        &self,
        retiring: &RetiringSignal,
        draining: bool,
        attached_at: Instant,
    ) -> io::Result<Option<End>> {
        let event = self.port.next(LISTEN_WAIT)?;
        if self.group.is_gone() {
            return Ok(Some(End::Gone));
        }
        if retiring.is_set() {
            return Ok(Some(End::Retiring));
        }
        if let (PortEvent::Joined(pid), false) = (event, draining) {
            self.record(pid, attached_at);
        }
        Ok(None)
    }

    /// If the record is on, hold it right away (outside the lock) — the notification's number must be opened and held for it to be both
    ///   alive and not reused. Then
    /// put it in only if it is the same record, and drop what could not be put in after the record lock is released.
    fn record(&self, pid: u32, attached_at: Instant) {
        let r = self.recorder.active();
        if r == 0 {
            return;
        }
        let Some((pin, killable)) = self.pin(pid) else {
            return;
        };
        let facts = pin.facts().clone();
        let born_ago_us = born_ago_us(facts.create);
        let caught = Caught {
            pid,
            facts,
            pin,
            killable,
        };
        match self.recorder.admit(r, caught) {
            Admit::Recorded { seq } => tracing::debug!(
                agent = self.tag.agent(),
                root_pid = self.tag.root_pid,
                pid,
                seq,
                killable,
                born_ago_us,
                since_attach_ms = millis(attached_at.elapsed()),
                "held a joined process and put it in the birth record"
            ),
            Admit::Full { caught, first } => {
                drop(caught);
                if first {
                    tracing::debug!(
                        agent = self.tag.agent(),
                        root_pid = self.tag.root_pid,
                        limit = BIRTH_MAX,
                        "the birth record reached its cap, no more are added — overflowing links are not known"
                    );
                }
            }
            Admit::Stale(caught) | Admit::Known(caught) => drop(caught),
        }
    }

    /// Also hold kill access — if refused, hold once more with query access only and use it only as a link (`false`).
    fn pin(&self, pid: u32) -> Option<(Box<dyn Pinned>, bool)> {
        let refused = match self.group.pin(pid, true) {
            Ok(found) => return found.map(|pin| (pin, true)),
            Err(e) => e,
        };
        match self.group.pin(pid, false) {
            Ok(Some(pin)) => {
                tracing::debug!(
                    agent = self.tag.agent(),
                    root_pid = self.tag.root_pid,
                    pid,
                    "kill access was refused, so it is held only as a link: {refused}"
                );
                Some((pin, false))
            }
            Ok(None) => None,
            Err(e) => {
                tracing::debug!(
                    agent = self.tag.agent(),
                    root_pid = self.tag.root_pid,
                    pid,
                    "could not hold a joined process: {e}"
                );
                None
            }
        }
    }

    /// Normal end — turn off the record that was on. A retiring group is still attached, so detach before releasing the port (the rule of
    ///   `Births`).
    fn finish(&self, end: End) {
        drop(self.recorder.stop(self.recorder.active()));
        let reason = match end {
            End::Gone => "gone",
            End::Retiring => {
                // The group is closing soon, so a detach failure is only logged.
                if let Err(e) = self.group.unwatch_births() {
                    tracing::debug!(
                        agent = self.tag.agent(),
                        root_pid = self.tag.root_pid,
                        "could not detach the join notification port from the retreating group: {e}"
                    );
                }
                "retiring"
            }
        };
        tracing::debug!(
            agent = self.tag.agent(),
            root_pid = self.tag.root_pid,
            reason,
            "ending the join notification listening"
        );
    }

    /// Failure mode — latch the port, turn off the record that was on, and detach. True = detached (a group already gone also counts as
    ///   detached).
    fn fail(&self, failure: Failure) -> bool {
        self.recorder.fail_port();
        drop(self.recorder.stop(self.recorder.active()));
        let (detached, port) = match self.group.unwatch_births() {
            Ok(()) => (true, "detached".to_owned()),
            Err(e) if e.kind() == GROUP_GONE => (true, "the group is already gone".to_owned()),
            Err(e) => (false, format!("could not detach, only draining ({e})")),
        };
        match failure {
            Failure::Take(e) => tracing::warn!(
                agent = self.tag.agent(),
                root_pid = self.tag.root_pid,
                detached,
                "could not take a join notification, so the birth record stopped — this incarnation does not clean up interrupt leftovers · port: {port}: {e}"
            ),
            Failure::Panic(message) => tracing::error!(
                agent = self.tag.agent(),
                root_pid = self.tag.root_pid,
                detached,
                "the join notification listening panicked, so the birth record stopped — this incarnation does not clean up interrupt leftovers · port: {port}: {message}"
            ),
        }
        detached
    }
}

/// Time from creation until now (µs) — used only for the 「join → open」 delay field in the log (selection does not look at the wall clock —
///   TRD §3-6). If creation could not be
/// read or the clock is before it, `None`.
fn born_ago_us(create: u64) -> Option<u64> {
    if create == 0 {
        return None;
    }
    let since = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    let now = u64::try_from(since.as_nanos() / 100)
        .ok()?
        .checked_add(UNIX_EPOCH_FILETIME)?;
    now.checked_sub(create).map(|ticks| ticks / 10)
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

// ── Interrupt gate · episode · worker ───────────────────────────────────────────────────────

/// If this long passes with no turn end after the most recent interrupt line, open a pass (also the check interval after a pass that
///   killed anything).
/// Basis (spike 2026-09-29, 11/11): non-stalling interrupt `result` ≤ 1394 ms · longest normal process born after an interrupt 1171 ms ·
///   what lived at +3 s was stall leftovers.
// ADR-0257
// ADR-0262
pub(super) const INTERRUPT_LEFTOVER_GRACE: Duration = Duration::from_secs(3);

/// Exit code of a process this cleanup killed — a post-mortem can tell it from the path's `shutdown()` group-wide kill (1).
// ADR-0262
// ADR-0275
const LEFTOVER_EXIT_CODE: u32 = 0x7440;

/// Cap on the number of members a snapshot holds — if the list is longer, that episode is `Failed(too_many)`.
pub(super) const PIN_MAX: usize = 256;

/// Number of root children accepted in a snapshot — console host · claude (Q2 — inference). Over it is `Failed(shape)`.
const ROOT_CHILDREN_MAX: usize = 2;

/// Cap on rounds per pass — the longest measured pass is 2 (p3bg: owner → its fork), plus one margin. Remaining candidates are not killed;
///   warns `rounds_exhausted`.
// ADR-0262
pub(super) const PASS_ROUNDS: u32 = 3;

/// Deadline for confirming the exits of what one round killed — one is allotted per round. So a pass's waiting is at most
/// [`PASS_ROUNDS`] × this.
pub(super) const KILL_CONFIRM: Duration = Duration::from_millis(200);

const WORKER_THREAD: &str = "engram-claude-leftover";
const LISTENER_THREAD: &str = "engram-claude-births";

/// Monotonic clock · waiting · thread start the cleaner reads — real = [`SystemClock`]; the worker and listener are started through it.
///
/// ★[`now`](Clock::now) is also called inside the gate lock★ — it takes or waits on no other lock.
// ADR-0262
// ADR-0275
pub(super) trait LeftoverClock: Clock {
    fn sleep(&self, d: Duration);
    /// Start `body` as thread `name`. Nobody joins. `Err` = `body` was dropped and did not run.
    fn spawn(&self, name: &str, body: Box<dyn FnOnce() + Send>) -> io::Result<()>;
}

impl LeftoverClock for SystemClock {
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }

    fn spawn(&self, name: &str, body: Box<dyn FnOnce() + Send>) -> io::Result<()> {
        std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(body)
            .map(drop)
    }
}

/// Phase of the episode.
///
/// - `Waiting` = waiting for a pass (written: most recent interrupt + N) · `Cleaning` = the worker runs a pass · `Done` = no more passes.
/// - `Spent.at` = when a pass that killed anything ended — still the same episode at `at + N` → warn and `Done`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Waiting,
    Cleaning,
    Spent { at: Instant },
    Done,
}

/// Why the snapshot was not set up — that episode's pass kills nothing (warn `snapshot_failed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapCause {
    /// The list was incomplete.
    List,
    /// The list has no root, or the root could not be held.
    Root,
    TooMany,
    /// Root children exceed [`ROOT_CHILDREN_MAX`].
    Shape,
    /// The join-notification port is unusable · that record was already turned off at pass time.
    Port,
}

impl SnapCause {
    fn label(self) -> &'static str {
        match self {
            SnapCause::List => "list",
            SnapCause::Root => "root",
            SnapCause::TooMany => "too_many",
            SnapCause::Shape => "shape",
            SnapCause::Port => "port",
        }
    }
}

struct Member {
    pid: u32,
    pin: Box<dyn Pinned>,
}

/// Members that were in our group when the new episode was set up — held with query access only (while held, the number is not reused).
/// The root is always held. Dropping the last `Arc` closes the handle — drop it outside both locks.
struct Snapshot {
    root: Member,
    others: Vec<Member>,
}

impl Snapshot {
    /// The link the selection looks at — the root is first. Held without kill access, so `killable` is false.
    fn links(&self) -> Vec<Link<'_>> {
        std::iter::once(&self.root)
            .chain(&self.others)
            .map(|m| Link {
                pid: m.pid,
                facts: m.pin.facts(),
                seq: None,
                killable: false,
            })
            .collect()
    }
}

/// The snapshot the episode holds. `Released` = the pass ended and let go.
#[derive(Clone)]
enum SnapState {
    Taken(Arc<Snapshot>),
    Failed(SnapCause),
    Released,
}

impl SnapState {
    fn label(&self) -> &'static str {
        match self {
            SnapState::Taken(_) => "taken",
            SnapState::Failed(cause) => cause.label(),
            SnapState::Released => "released",
        }
    }

    fn members(&self) -> usize {
        match self {
            SnapState::Taken(s) => s.others.len().saturating_add(1),
            SnapState::Failed(_) | SnapState::Released => 0,
        }
    }
}

impl fmt::Debug for SnapState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// One interrupt episode — starts with the interrupt that set up a new episode, ends with turn end · new input · swap · retirement · pass end.
///
/// - `last_mono` = the most recent interrupt (later of accepted · written time) — the basis of N.
/// - `written` = the line was written and the write list `w` (sorted) obtained — no pass before that.
/// - `rec` = record number this episode turned on · `0` = released.
#[derive(Debug)]
struct Episode {
    gen: u64,
    first_mono: Instant,
    last_mono: Instant,
    written: bool,
    w: Option<Arc<[u32]>>,
    snap: SnapState,
    rec: u64,
    killed_any: bool,
    phase: Phase,
}

impl Episode {
    fn is_live(&self) -> bool {
        matches!(self.phase, Phase::Waiting | Phase::Cleaning)
    }

    fn release(&mut self) -> Released {
        let snap = match std::mem::replace(&mut self.snap, SnapState::Released) {
            SnapState::Taken(snap) => Some(snap),
            SnapState::Failed(_) | SnapState::Released => None,
        };
        Released {
            snap,
            w: None,
            rec: std::mem::take(&mut self.rec),
        }
    }

    fn into_released(mut self) -> Released {
        let mut released = self.release();
        released.w = self.w.take();
        released
    }
}

/// What was taken out of the gate lock — after releasing the lock the receiver drops it through [`GateCell::release`] and turns off the record.
#[must_use]
#[derive(Default)]
struct Released {
    snap: Option<Arc<Snapshot>>,
    w: Option<Arc<[u32]>>,
    rec: u64,
}

/// The value the opener took in section ① — section ② tells apart what came in between.
#[derive(Debug, Clone, Copy)]
struct Mark {
    closes: u64,
    delivers: u64,
    next_gen: u64,
}

/// State inside the lock of [`GateCell`]. Transitions are pure — time · atomics come in as arguments; no OS calls, allocation or free,
///   panics. What was taken out is
/// returned and the caller drops it after releasing the lock.
///
/// - `open` = a turn that can be interrupted is open (only the decoder uses it) · `closes` · `delivers` = counts of turn ends · new inputs.
/// - `opening` = one opener is between the two sections · `worker` = a worker is up (including while starting).
#[derive(Debug, Default)]
struct GateState {
    open: bool,
    closes: u64,
    delivers: u64,
    opening: bool,
    episode: Option<Episode>,
    next_gen: u64,
    worker: bool,
}

#[derive(Debug)]
enum Begin {
    /// The gate is closed — no line.
    Closed,
    /// Appended to a live episode — write confirmation is received with that generation.
    Continue { gen: u64 },
    /// Another interrupt is opening — only the line is given (that episode takes this turn).
    Busy,
    /// This interrupt is the opener.
    Open(Mark),
}

enum Settle {
    /// The turn closed between the two sections — no line.
    NoLine(SnapState),
    /// Only the line is given, without an episode.
    LineOnly { snap: SnapState, why: Stale },
    /// `old` = the old episode that was swapped out.
    Opened { gen: u64, old: Option<Episode> },
}

/// Why section ② did not set up an episode (debug `stale`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stale {
    /// The record that was turned on is already off (listener thread ended · failed).
    Record,
    Delivered,
    Generation,
    /// A live episode exists — a branch that is not reached because there is only one opener.
    Live,
}

impl Stale {
    fn label(self) -> &'static str {
        match self {
            Stale::Record => "record",
            Stale::Delivered => "delivered",
            Stale::Generation => "generation",
            Stale::Live => "live",
        }
    }
}

/// Result of one write confirmation. `back` = the write list that was not used — the caller drops it after releasing.
struct Noted {
    back: Option<Arc<[u32]>>,
    note: WriteNote,
    spawn: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriteNote {
    /// First write — received the write list.
    First { w_len: usize },
    /// Written but without a list, so not counted as written.
    Unlisted,
    /// A later interrupt of an already written episode.
    Again,
    /// There is no live episode of that generation.
    Stale,
}

enum Step {
    /// The worker ends (`worker` was already lowered). `retired` = the episode cleared because the group is retiring.
    Exit {
        retired: Option<Episode>,
        released: Released,
    },
    Sleep(Duration),
    Pass(Ticket),
    /// Still the same episode at N after a pass that killed anything — the episode is already `Done`.
    StillHung {
        waited: Duration,
    },
}

/// Input of one pass — taken in the lock section that switches to `Cleaning`; the held `Arc` is dropped outside the lock after the pass.
///
/// - `rec` = this episode's record number — the pass copies that record (empty if `active` differs) · `w` = write list of the first write
///   (sorted).
struct Ticket {
    gen: u64,
    first_mono: Instant,
    rec: u64,
    w: Arc<[u32]>,
    snap: SnapState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Commit {
    Go,
    Retiring,
    /// Mid-pass turn end · new interrupt · new input — stops the whole pass (debug `superseded`).
    Superseded,
}

impl GateState {
    fn open_turn(&mut self) {
        self.open = true;
    }

    /// New input reached the CLI — drop the episode and open the gate.
    fn deliver(&mut self) -> Option<Episode> {
        self.delivers = self.delivers.wrapping_add(1);
        self.open = true;
        self.episode.take()
    }

    fn close_turn(&mut self) -> Option<Episode> {
        self.open = false;
        self.closes = self.closes.wrapping_add(1);
        self.episode.take()
    }

    /// Section ①. The pass running when appending to a live episode stops at the next kill commit, and waits again for the most recent
    ///   interrupt + N.
    fn begin_interrupt(&mut self, now: Instant) -> Begin {
        if !self.open {
            return Begin::Closed;
        }
        if let Some(ep) = self.episode.as_mut().filter(|ep| ep.is_live()) {
            ep.phase = Phase::Waiting;
            ep.last_mono = ep.last_mono.max(now);
            return Begin::Continue { gen: ep.gen };
        }
        if self.opening {
            return Begin::Busy;
        }
        self.opening = true;
        Begin::Open(Mark {
            closes: self.closes,
            delivers: self.delivers,
            next_gen: self.next_gen,
        })
    }

    /// Section ② — set up a new episode with the record `rec` the opener turned on (the number now on = `active`) and the snapshot it took.
    fn settle_interrupt(
        &mut self,
        mark: Mark,
        rec: u64,
        active: u64,
        snap: SnapState,
        now: Instant,
    ) -> Settle {
        self.opening = false;
        if !self.open || self.closes != mark.closes {
            return Settle::NoLine(snap);
        }
        let stale = if active != rec {
            Some(Stale::Record)
        } else if self.delivers != mark.delivers {
            Some(Stale::Delivered)
        } else if self.next_gen != mark.next_gen {
            Some(Stale::Generation)
        } else if self.episode.as_ref().is_some_and(Episode::is_live) {
            Some(Stale::Live)
        } else {
            None
        };
        if let Some(why) = stale {
            return Settle::LineOnly { snap, why };
        }
        let gen = self.next_gen;
        self.next_gen = gen.wrapping_add(1);
        let old = self.episode.replace(Episode {
            gen,
            first_mono: now,
            last_mono: now,
            written: false,
            w: None,
            snap,
            rec,
            killed_any: false,
            phase: Phase::Waiting,
        });
        Settle::Opened { gen, old }
    }

    /// Write confirmation — if it is a live episode of the same generation, pull the most recent interrupt forward; the first write counts
    ///   as written only when there is a list `w`.
    /// Other `w` are returned. If the written episode has no worker, give the job of starting one.
    fn note_written(&mut self, gen: u64, w: Option<Arc<[u32]>>, now: Instant) -> Noted {
        let worker = self.worker;
        let Some(ep) = self
            .episode
            .as_mut()
            .filter(|ep| ep.gen == gen && ep.is_live())
        else {
            return Noted {
                back: w,
                note: WriteNote::Stale,
                spawn: false,
            };
        };
        ep.last_mono = ep.last_mono.max(now);
        let (back, note) = match (ep.written, w) {
            (true, w) => (w, WriteNote::Again),
            (false, Some(w)) => {
                let w_len = w.len();
                ep.written = true;
                ep.w = Some(w);
                (None, WriteNote::First { w_len })
            }
            (false, None) => (None, WriteNote::Unlisted),
        };
        let spawn = ep.written && !worker;
        if spawn {
            self.worker = true;
        }
        Noted { back, note, spawn }
    }

    /// One worker step (TRD §3-3) — if the marker is set, clear the episode and end · none · `Done` · unwritten `Waiting` → end · written
    /// `Waiting` sleeps if before N · if past, `Cleaning` + pass · `Spent` sleeps if before N · if past, `Done` + warn.
    fn next_step(&mut self, now: Instant, retiring: bool) -> Step {
        if retiring {
            self.worker = false;
            return Step::Exit {
                retired: self.episode.take(),
                released: Released::default(),
            };
        }
        let Some(ep) = self.episode.as_mut() else {
            self.worker = false;
            return Step::Exit {
                retired: None,
                released: Released::default(),
            };
        };
        let released = match ep.phase {
            Phase::Done => Released::default(),
            Phase::Waiting if !ep.written => Released::default(),
            Phase::Waiting => match ep.last_mono.checked_add(INTERRUPT_LEFTOVER_GRACE) {
                Some(due) if now < due => {
                    return Step::Sleep(due.saturating_duration_since(now));
                }
                Some(_) => {
                    if let Some(w) = ep.w.clone() {
                        ep.phase = Phase::Cleaning;
                        return Step::Pass(Ticket {
                            gen: ep.gen,
                            first_mono: ep.first_mono,
                            rec: ep.rec,
                            w,
                            snap: ep.snap.clone(),
                        });
                    }
                    // A spot where 「if written, a list exists」 broke — there is no list to set up a pass.
                    ep.phase = Phase::Done;
                    ep.release()
                }
                None => {
                    ep.phase = Phase::Done;
                    ep.release()
                }
            },
            // Only the worker sets `Cleaning`, and the end of that pass always clears it, so it is not reached without a future defect.
            //   Even if it were, an episode
            // completes only one pass, so it does not run again.
            Phase::Cleaning => {
                ep.phase = Phase::Done;
                ep.release()
            }
            Phase::Spent { at } => match at.checked_add(INTERRUPT_LEFTOVER_GRACE) {
                Some(due) if now < due => {
                    return Step::Sleep(due.saturating_duration_since(now));
                }
                _ => {
                    ep.phase = Phase::Done;
                    return Step::StillHung {
                        waited: now.saturating_duration_since(ep.first_mono),
                    };
                }
            },
        };
        self.worker = false;
        Step::Exit {
            retired: None,
            released,
        }
    }

    /// Recheck of the kill commit — no marker · same generation · written · still `Cleaning` (no interrupt · turn end · new input during
    ///   the pass) ·
    /// most recent interrupt + N ≤ now.
    fn still_due(&self, gen: u64, now: Instant, retiring: bool) -> bool {
        !retiring
            && self.episode.as_ref().is_some_and(|ep| {
                ep.gen == gen
                    && ep.written
                    && ep.phase == Phase::Cleaning
                    && ep
                        .last_mono
                        .checked_add(INTERRUPT_LEFTOVER_GRACE)
                        .is_some_and(|due| due <= now)
            })
    }

    fn commit_check(&self, gen: u64, now: Instant, retiring: bool) -> Commit {
        if retiring {
            Commit::Retiring
        } else if self.still_due(gen, now, retiring) {
            Commit::Go
        } else {
            Commit::Superseded
        }
    }

    fn note_kill(&mut self, gen: u64) {
        if let Some(ep) = self.episode.as_mut().filter(|ep| ep.gen == gen) {
            ep.killed_any = true;
        }
    }

    /// Pass end — only if the same generation is still `Cleaning`, close it out as `Spent` (killed something)/`Done` and hand out the
    ///   snapshot · record.
    /// If an interrupt mid-pass returned it to `Waiting`, keep holding and wait for the next pass.
    fn finish_pass(&mut self, gen: u64, killed: bool, now: Instant) -> Released {
        let Some(ep) = self.episode.as_mut().filter(|ep| ep.gen == gen) else {
            return Released::default();
        };
        ep.killed_any |= killed;
        let released = if ep.phase == Phase::Cleaning {
            ep.phase = if ep.killed_any {
                Phase::Spent { at: now }
            } else {
                Phase::Done
            };
            ep.release()
        } else {
            Released::default()
        };
        released
    }

    /// The worker is gone (start failure · panic) — whatever the generation, bring the current episode down to `Done`. Because an episode
    ///   another interrupt set up in the meantime
    /// also believed 「a worker is up」 and nobody started one. Do not immediately start again.
    fn abandon(&mut self) -> Released {
        self.worker = false;
        match self.episode.as_mut() {
            Some(ep) => {
                ep.phase = Phase::Done;
                ep.release()
            }
            None => Released::default(),
        }
    }
}

/// One incarnation's cleaner — group handle · birth record · port acquisition · clock · root · retirement flag · log attribution. If it
///   cannot be assembled,
/// it is not created (then [`GateCell`] only opens and closes the gate).
// ADR-0262
pub(super) struct Cleaner<G: Group = ProcessGroup> {
    group: Arc<G>,
    recorder: Arc<Recorder>,
    watch: BirthWatch,
    clock: Arc<dyn LeftoverClock>,
    root_pid: u32,
    retiring: RetiringSignal,
    tag: LogTag,
    unlisted_warned: AtomicBool,
}

impl Cleaner<ProcessGroup> {
    pub(super) fn new(
        group: Arc<ProcessGroup>,
        clock: Arc<dyn LeftoverClock>,
        root_pid: u32,
        tag: LogTag,
    ) -> Self {
        Self::with_parts(group, Arc::new(Recorder::new()), clock, root_pid, tag)
    }
}

impl<G: Group> Cleaner<G> {
    fn with_parts(
        group: Arc<G>,
        recorder: Arc<Recorder>,
        clock: Arc<dyn LeftoverClock>,
        root_pid: u32,
        tag: LogTag,
    ) -> Self {
        let retiring = group.retiring();
        Self {
            group,
            recorder,
            watch: BirthWatch::new(),
            clock,
            root_pid,
            retiring,
            tag,
            unlisted_warned: AtomicBool::new(false),
        }
    }

    fn ensure_port(&self) -> Result<(), ()> {
        self.watch
            .ensure_with(&self.group, &self.recorder, &self.tag, |body| {
                self.clock.spawn(LISTENER_THREAD, body)
            })
    }
}

/// Snapshot of a new episode — hold every member of the complete list with query access only and read its facts. Non-root members that
///   could not be held
/// (exited · outside the group · refused) are left out as unknown links, so what is below them misses. Called outside both locks.
// ADR-0262
fn take_snapshot<G: Group>(group: &G, root_pid: u32) -> SnapState {
    let Ok(pids) = group.member_pids() else {
        return SnapState::Failed(SnapCause::List);
    };
    if pids.len() > PIN_MAX {
        return SnapState::Failed(SnapCause::TooMany);
    }
    if root_pid == 0 || !pids.contains(&root_pid) {
        return SnapState::Failed(SnapCause::Root);
    }
    let mut root = None;
    let mut others = Vec::with_capacity(pids.len());
    for pid in pids {
        let Ok(Some(pin)) = group.pin(pid, false) else {
            continue;
        };
        let member = Member { pid, pin };
        if pid == root_pid {
            root = Some(member);
        } else {
            others.push(member);
        }
    }
    let Some(root) = root else {
        return SnapState::Failed(SnapCause::Root);
    };
    let root_children = others
        .iter()
        .filter(|m| m.pin.facts().ppid == root_pid)
        .count();
    if root_children > ROOT_CHILDREN_MAX {
        return SnapState::Failed(SnapCause::Shape);
    }
    SnapState::Taken(Arc::new(Snapshot { root, others }))
}

enum Unlisted {
    List(io::Error),
    /// The list is empty — the path is gone.
    Gone,
}

impl fmt::Display for Unlisted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unlisted::List(e) => write!(f, "list {e}"),
            Unlisted::Gone => f.write_str("no group"),
        }
    }
}

/// Write list W — complete member numbers right after the line was written (sorted). Called with no lock (the handle upgrades the Job only
///   for that call).
/// Candidates must be outside W, so if W cannot be taken, no pass is set up for that write.
fn write_list<G: Group>(group: &G) -> Result<Arc<[u32]>, Unlisted> {
    let mut pids = group.member_pids().map_err(Unlisted::List)?;
    if pids.is_empty() {
        return Err(Unlisted::Gone);
    }
    pids.sort_unstable();
    Ok(pids.into())
}

/// Turn-open gate + interrupt episode — one per incarnation, held by the same `Arc` by the interrupt-line function · decoder · write
///   confirmation · worker.
// ADR-0262
pub(super) struct GateCell<G: Group = ProcessGroup> {
    state: Mutex<GateState>,
    cleaner: Option<Arc<Cleaner<G>>>,
}

#[derive(Debug, Clone, Copy)]
struct Seen {
    open: bool,
    opening: bool,
    episode: Option<(u64, Phase, bool)>,
}

impl Seen {
    fn of(state: &GateState) -> Self {
        Self {
            open: state.open,
            opening: state.opening,
            episode: state
                .episode
                .as_ref()
                .map(|ep| (ep.gen, ep.phase, ep.written)),
        }
    }
}

impl<G: Group> fmt::Debug for GateCell<G> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Write after the lock is released — the formatter may write to a file · console and must not wait on this lock.
        // ADR-0275: not base `sync` (only a waiting `lock`); only the poisoned branch is recovered here.
        let seen = match self.state.try_lock() {
            Ok(state) => Some(Seen::of(&state)),
            Err(TryLockError::Poisoned(state)) => Some(Seen::of(&state.into_inner())),
            Err(TryLockError::WouldBlock) => None,
        };
        let mut out = f.debug_struct("GateCell");
        match seen {
            Some(seen) => out
                .field("open", &seen.open)
                .field("opening", &seen.opening)
                .field("episode", &seen.episode),
            None => out.field("state", &format_args!("<locked>")),
        };
        out.field("cleaner", &self.cleaner.is_some()).finish()
    }
}

/// Opener guard — set up after becoming the opener in section ①. On every exit that set up no new episode (panic · no line · line only):
///   take the gate lock, lower
/// `opening` (only if section ② was not reached) and turn off the record that was on **after** releasing it.
///
/// ★After section ② do not touch `opening`★ — ② already lowered it and another interrupt may be the opener now (clearing it would make two
///   openers).
struct Opener<'a, G: Group> {
    cell: &'a GateCell<G>,
    recorder: &'a Recorder,
    rec: u64,
    holds_opening: bool,
    holds_record: bool,
}

impl<G: Group> Drop for Opener<'_, G> {
    fn drop(&mut self) {
        if self.holds_opening {
            self.cell.lock().opening = false;
        }
        if self.holds_record {
            drop(self.recorder.stop(self.rec));
        }
    }
}

impl<G: Group> GateCell<G> {
    /// If `cleaner` is `None`, only open and close the gate — an interrupt line is given only when the gate is open, and there is no episode.
    pub(super) fn new(cleaner: Option<Arc<Cleaner<G>>>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(GateState::default()),
            cleaner,
        })
    }

    fn lock(&self) -> MutexGuard<'_, GateState> {
        sync::lock(&self.state)
    }

    #[cfg(test)]
    pub(super) fn is_open(&self) -> bool {
        self.lock().open
    }

    /// Body of the interrupt-line function — `None` (no line) if the gate is closed. `on_written` exists only when this interrupt set up
    ///   an episode or appended to a live
    /// one (another opener · record · input · generation changed between the two sections → line only).
    ///
    /// One opener: acquire the port outside the lock → turn on the record → take the snapshot. ★Turn the record on before the snapshot★ —
    ///   what is born in between is
    /// either recorded (after the listing) or in the snapshot (before it).
    // ADR-0262
    pub(super) fn interrupt(self: &Arc<Self>, bytes: Vec<u8>) -> Option<InterruptOut> {
        let Some(cleaner) = self.cleaner.as_ref() else {
            let open = self.lock().open;
            return open.then(|| InterruptOut {
                bytes,
                on_written: None,
            });
        };
        let begun = {
            let mut state = self.lock();
            let now = cleaner.clock.now();
            state.begin_interrupt(now)
        };
        let mark = match begun {
            Begin::Closed => return None,
            Begin::Continue { gen } => {
                return Some(InterruptOut {
                    bytes,
                    on_written: Some(self.on_written(gen)),
                });
            }
            Begin::Busy => {
                tracing::debug!(
                    agent = cleaner.tag.agent(),
                    root_pid = cleaner.tag.root_pid,
                    "another interrupt is opening an episode, so only the interrupt line is given"
                );
                return Some(InterruptOut {
                    bytes,
                    on_written: None,
                });
            }
            Begin::Open(mark) => mark,
        };

        let mut opener = Opener {
            cell: self,
            recorder: &cleaner.recorder,
            rec: 0,
            holds_opening: true,
            holds_record: false,
        };
        let port = cleaner.ensure_port();
        let (rec, left) = cleaner.recorder.start();
        opener.rec = rec;
        opener.holds_record = true;
        drop(left);
        let snap = match port {
            Ok(()) => take_snapshot(&*cleaner.group, cleaner.root_pid),
            Err(()) => SnapState::Failed(SnapCause::Port),
        };
        let (snapshot, members) = (snap.label(), snap.members());
        let settled = {
            let mut state = self.lock();
            let active = cleaner.recorder.active();
            let now = cleaner.clock.now();
            state.settle_interrupt(mark, rec, active, snap, now)
        };
        opener.holds_opening = false;

        match settled {
            Settle::Opened { gen, old } => {
                opener.holds_record = false;
                drop(opener);
                if let Some(old) = old {
                    self.release(old.into_released());
                }
                tracing::debug!(
                    agent = cleaner.tag.agent(),
                    root_pid = cleaner.tag.root_pid,
                    gen,
                    rec,
                    snapshot,
                    members,
                    births = cleaner.recorder.len(),
                    "opened an interrupt episode"
                );
                Some(InterruptOut {
                    bytes,
                    on_written: Some(self.on_written(gen)),
                })
            }
            Settle::NoLine(snap) => {
                drop(opener);
                drop(snap);
                tracing::debug!(
                    agent = cleaner.tag.agent(),
                    root_pid = cleaner.tag.root_pid,
                    "the turn closed while the episode was being opened, so the interrupt line is not given"
                );
                None
            }
            Settle::LineOnly { snap, why } => {
                drop(opener);
                drop(snap);
                if why == Stale::Live {
                    tracing::warn!(
                        agent = cleaner.tag.agent(),
                        root_pid = cleaner.tag.root_pid,
                        "there is one opener, yet a live episode sits where the episode is set up — only the line is given"
                    );
                } else {
                    tracing::debug!(
                        agent = cleaner.tag.agent(),
                        root_pid = cleaner.tag.root_pid,
                        reason = why.label(),
                        "stale — the record or input changed while the episode was being opened, so only the interrupt line is given"
                    );
                }
                debug_assert!(why != Stale::Live, "there were two openers");
                Some(InterruptOut {
                    bytes,
                    on_written: None,
                })
            }
        }
    }

    fn on_written(self: &Arc<Self>, gen: u64) -> OnWritten {
        let cell = Arc::clone(self);
        Box::new(move || cell.written(gen))
    }

    fn written(self: &Arc<Self>, gen: u64) {
        let Some(cleaner) = self.cleaner.as_ref() else {
            return;
        };
        let began = cleaner.clock.now();
        let listed = write_list(&*cleaner.group);
        let w_us = micros(cleaner.clock.now().saturating_duration_since(began));
        let (w, unlisted) = match listed {
            Ok(w) => (Some(w), None),
            Err(e) => (None, Some(e)),
        };
        let Noted { back, note, spawn } = {
            let mut state = self.lock();
            let now = cleaner.clock.now();
            state.note_written(gen, w, now)
        };
        drop(back);
        match (note, unlisted) {
            (WriteNote::First { w_len }, _) => tracing::debug!(
                agent = cleaner.tag.agent(),
                root_pid = cleaner.tag.root_pid,
                gen,
                w_len,
                w_us,
                "the interrupt line was written and the write list was captured"
            ),
            (WriteNote::Unlisted, Some(e)) => {
                if cleaner.unlisted_warned.swap(true, Ordering::AcqRel) {
                    tracing::debug!(
                        agent = cleaner.tag.agent(),
                        root_pid = cleaner.tag.root_pid,
                        gen,
                        "the interrupt line was written but the write list could not be captured: {e}"
                    );
                } else {
                    tracing::warn!(
                        agent = cleaner.tag.agent(),
                        root_pid = cleaner.tag.root_pid,
                        gen,
                        "the interrupt line was written but the write list could not be captured, so this interrupt does not clean up leftovers: {e}"
                    );
                }
            }
            _ => {}
        }
        if spawn {
            self.spawn_worker(cleaner);
        }
    }

    fn spawn_worker(self: &Arc<Self>, cleaner: &Arc<Cleaner<G>>) {
        let cell = Arc::clone(self);
        let worker_cleaner = Arc::clone(cleaner);
        let spawned = cleaner.clock.spawn(
            WORKER_THREAD,
            Box::new(move || run_worker(&cell, &worker_cleaner)),
        );
        if let Err(e) = spawned {
            let released = self.lock().abandon();
            self.release(released);
            tracing::warn!(
                agent = cleaner.tag.agent(),
                root_pid = cleaner.tag.root_pid,
                "could not start the leftover cleanup worker, so this interrupt does not clean up: {e}"
            );
        }
    }

    pub(super) fn open_turn(&self) {
        self.lock().open_turn();
    }

    /// When the decoder sees new input (live `started`) — drop the episode and open the gate (one section).
    pub(super) fn deliver(&self) {
        let dropped = self.lock().deliver();
        let Some(ep) = dropped else {
            return;
        };
        if ep.is_live() {
            if let Some(cleaner) = &self.cleaner {
                tracing::debug!(
                    agent = cleaner.tag.agent(),
                    root_pid = cleaner.tag.root_pid,
                    gen = ep.gen,
                    "delivered — new input arrived, so the interrupt episode was dropped"
                );
            }
        }
        self.release(ep.into_released());
    }

    pub(super) fn close_turn(&self) {
        let dropped = self.lock().close_turn();
        if let Some(ep) = dropped {
            self.release(ep.into_released());
        }
    }

    /// Pass end (TRD §3-3).
    fn end_pass(&self, cleaner: &Cleaner<G>, gen: u64, killed: bool) {
        let released = {
            let mut state = self.lock();
            let now = cleaner.clock.now();
            state.finish_pass(gen, killed, now)
        };
        self.release(released);
    }

    /// Drop what was taken out and turn off that record — called outside both locks.
    fn release(&self, released: Released) {
        let Released { snap, w, rec } = released;
        drop(snap);
        drop(w);
        if let Some(cleaner) = &self.cleaner {
            drop(cleaner.recorder.stop(rec));
        }
    }
}

/// Worker body — walks until there is no episode · it was not written · it becomes `Done` or retires.
///
/// ★The panic guard stands only in test builds without the hook — this thread is outside the containment marker★; the body must not panic
///   (no unwrap · no indexing ·
/// poison tolerated). On unwind, bring the current episode to `Done` and release what is held, so no written episode is left without a worker.
// ADR-0262
// ADR-0292
fn run_worker<G: Group>(cell: &Arc<GateCell<G>>, cleaner: &Arc<Cleaner<G>>) {
    let mut guard = WorkerGuard { cell, armed: true };
    while worker_turn(cell, cleaner) {}
    guard.armed = false;
}

struct WorkerGuard<'a, G: Group> {
    cell: &'a GateCell<G>,
    armed: bool,
}

impl<G: Group> Drop for WorkerGuard<'_, G> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let released = self.cell.lock().abandon();
        self.cell.release(released);
        if let Some(cleaner) = &self.cell.cleaner {
            tracing::error!(
                agent = cleaner.tag.agent(),
                root_pid = cleaner.tag.root_pid,
                "the leftover cleanup worker panicked, so this interrupt does not clean up"
            );
        }
    }
}

fn worker_turn<G: Group>(cell: &Arc<GateCell<G>>, cleaner: &Arc<Cleaner<G>>) -> bool {
    let step = {
        let mut state = cell.lock();
        let now = cleaner.clock.now();
        state.next_step(now, cleaner.retiring.is_set())
    };
    match step {
        Step::Exit { retired, released } => {
            cell.release(released);
            if let Some(ep) = retired {
                cell.release(ep.into_released());
                tracing::debug!(
                    agent = cleaner.tag.agent(),
                    root_pid = cleaner.tag.root_pid,
                    "retiring — the group is retreating, so the interrupt episode is cleared and the cleanup worker ends"
                );
            }
            false
        }
        Step::Sleep(d) => {
            cleaner.clock.sleep(d);
            true
        }
        Step::StillHung { waited } => {
            tracing::warn!(
                agent = cleaner.tag.agent(),
                root_pid = cleaner.tag.root_pid,
                waited_ms = millis(waited),
                "still no turn end after the pass that ended leftovers — not doing more"
            );
            true
        }
        Step::Pass(ticket) => {
            run_pass(cell, cleaner, ticket);
            true
        }
    }
}

#[derive(Debug)]
enum PassStop {
    /// Pass steps 1–3 did not hold, or the recheck before kill · retirement in the commit · root exit was seen.
    NotReady(NotReady),
    /// The commit's recheck was refused — mid-pass turn end · new interrupt · new input (debug `superseded`).
    Superseded,
    /// The round's record copy is empty and that record is already off — the episode was dropped or the listener thread ended (that side logs
    /// separately).
    RecordGone,
    /// Candidates to kill remain even after finishing [`PASS_ROUNDS`] rounds (warn `rounds_exhausted`).
    RoundsExhausted,
}

/// One killed item — the `terminated` field of the cleaned log. `chain` = from the candidate up to the top T ([`Verdict::Cleanup`]).
struct Killed {
    pid: u32,
    ppid: u32,
    chain: Vec<u32>,
    round: u32,
}

impl fmt::Debug for Killed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "pid={} ppid={} chain={:?} round={}",
            self.pid, self.ppid, self.chain, self.round
        )
    }
}

/// Number of candidates dropped per rule — those of the last round selected. A candidate dropped while alive is counted again in the next
///   round, so rounds are not
/// added together.
#[derive(Debug, Default, Clone, Copy)]
struct Excluded {
    parent_alive: usize,
    parent_unknown: usize,
    not_hook_copy: usize,
    ancestor_alive: usize,
    chain_cut: usize,
    not_hook: usize,
    no_taskkill: usize,
    unknown: usize,
}

impl Excluded {
    fn note(&mut self, verdict: &Verdict) {
        let slot = match verdict {
            Verdict::Cleanup { .. } => return,
            Verdict::ParentAlive => &mut self.parent_alive,
            Verdict::ParentUnknown => &mut self.parent_unknown,
            Verdict::NotHookCopy => &mut self.not_hook_copy,
            Verdict::AncestorAlive { .. } => &mut self.ancestor_alive,
            Verdict::ChainCut => &mut self.chain_cut,
            Verdict::NotHook => &mut self.not_hook,
            Verdict::NoTaskkill => &mut self.no_taskkill,
            Verdict::Unknown => &mut self.unknown,
        };
        *slot = slot.saturating_add(1);
    }
}

/// Result of one pass — read by the log.
///
/// - `stop` = `None` if it ran up to a round with no new `Cleanup` · `rounds` = rounds that reached the commit (the last empty one is not
///   counted).
/// - `terminated` = kill calls accepted · `confirmed` = of those, exits confirmed within the round's deadline (a pass stopped midway does
///   not confirm).
/// - `candidates` · `full` = first round's candidate count and whether its record overflowed · `births` = size of the last copied record.
/// - `cleanups` = reached the kill call · `failed` = kill errors · `gone` = exited by themselves meanwhile.
#[derive(Debug, Default)]
struct PassReport {
    stop: Option<PassStop>,
    rounds: u32,
    terminated: Vec<Killed>,
    confirmed: usize,
    terminate_max: Duration,
    candidates: usize,
    births: usize,
    full: bool,
    excluded: Excluded,
    cleanups: usize,
    failed: usize,
    gone: usize,
}

/// One cleanup pass (TRD §3-3 「rounds of a pass」 · §3-4 「steps of a pass」) — if steps 1–3 hold it goes through the rounds; otherwise
///   nothing is killed.
///
/// One round = copy the record → candidates · rules → kill commit of the `Cleanup` items in ascending `seq` order → confirm within one
///   deadline ([`KILL_CONFIRM`]).
/// What was killed shows up next round as an exited known link and its copies become candidates. Stops at a round with no new `Cleanup`: a
///   candidate that reached the
/// commit is never selected again (one that did not exit in time or whose kill failed is not killed repeatedly).
///
/// Always closes with the pass end; the log · copied record · `ticket` are dropped outside both locks.
// ADR-0262
fn run_pass<G: Group>(
    cell: &Arc<GateCell<G>>,
    cleaner: &Arc<Cleaner<G>>,
    ticket: Ticket,
) -> PassReport {
    let mut report = PassReport::default();
    let killed = match precheck(
        &ticket,
        cleaner.recorder.active(),
        cleaner.retiring.is_set(),
        cleaner.recorder.port_failed(),
    ) {
        Ok(snapshot) => kill_rounds(cell, cleaner, &ticket, snapshot, &mut report),
        Err(why) => {
            report.stop = Some(PassStop::NotReady(why));
            false
        }
    };
    cell.end_pass(cleaner, ticket.gen, killed);
    let waited = cleaner
        .clock
        .now()
        .saturating_duration_since(ticket.first_mono);
    log_pass(cleaner, &report, waited);
    drop(ticket);
    report
}

fn kill_rounds<G: Group>(
    cell: &GateCell<G>,
    cleaner: &Cleaner<G>,
    ticket: &Ticket,
    snapshot: &Snapshot,
    report: &mut PassReport,
) -> bool {
    let snapshot_links = snapshot.links();
    let mut attempted: Vec<u64> = Vec::new();
    let mut killed_any = false;
    for round in 1..=PASS_ROUNDS.saturating_add(1) {
        let births = cleaner.recorder.copy(ticket.rec);
        report.births = births.len();
        if round == 1 {
            report.full = cleaner.recorder.full();
        }
        if births.is_empty() && cleaner.recorder.active() != ticket.rec {
            report.stop = Some(PassStop::RecordGone);
            break;
        }
        let picks = pick(
            snapshot,
            &snapshot_links,
            &births,
            &ticket.w,
            &attempted,
            round,
            report,
        );
        if picks.is_empty() {
            break;
        }
        if round > PASS_ROUNDS {
            report.stop = Some(PassStop::RoundsExhausted);
            break;
        }
        report.rounds = round;

        let mut confirming: Vec<&Birth> = Vec::new();
        for (seq, chain) in picks {
            let Some(birth) = births.iter().find(|b| b.seq == seq) else {
                continue;
            };
            attempted.push(seq);
            if let Err(why) = recheck(snapshot, &cleaner.retiring) {
                report.stop = Some(PassStop::NotReady(why));
                break;
            }
            // ★Kill commit — only the recheck and one kill call are inside the lock★ (TRD §3-3 「lock rules」 ⓑ); deciding · logging ·
            //   dropping handles come after.
            //   The marker is read once more here; the remaining overlap is the single kill right after this read.
            // ADR-0262
            let committed = {
                let mut state = cell.lock();
                let now = cleaner.clock.now();
                match state.commit_check(ticket.gen, now, cleaner.retiring.is_set()) {
                    Commit::Go => {
                        let raw = birth.pin.terminate_raw(LEFTOVER_EXIT_CODE);
                        let took = cleaner.clock.now().saturating_duration_since(now);
                        if raw.is_ok() {
                            state.note_kill(ticket.gen);
                        }
                        Ok((raw, took))
                    }
                    Commit::Retiring => Err(PassStop::NotReady(NotReady::Retiring)),
                    Commit::Superseded => Err(PassStop::Superseded),
                }
            };
            let (raw, took) = match committed {
                Ok(called) => called,
                Err(stop) => {
                    report.stop = Some(stop);
                    break;
                }
            };
            report.cleanups = report.cleanups.saturating_add(1);
            report.terminate_max = report.terminate_max.max(took);
            killed_any |= raw.is_ok();
            match birth.pin.classify(raw) {
                Ok(MemberKill::Terminated) => {
                    report.terminated.push(Killed {
                        pid: birth.pid,
                        ppid: birth.facts.ppid,
                        chain,
                        round,
                    });
                    confirming.push(birth);
                }
                Ok(MemberKill::Gone) => report.gone = report.gone.saturating_add(1),
                Err(e) => {
                    report.failed = report.failed.saturating_add(1);
                    tracing::warn!(
                        agent = cleaner.tag.agent(),
                        root_pid = cleaner.tag.root_pid,
                        pid = birth.pid,
                        round,
                        "could not end one interrupt leftover: {e}"
                    );
                }
            }
        }
        if report.stop.is_some() {
            break;
        }
        report.confirmed = report
            .confirmed
            .saturating_add(confirm(&*cleaner.clock, &confirming));
    }
    killed_any
}

/// Selection of one round — gives the `Cleanup` items of this pass that have not yet gone to the commit, in ascending `seq` order. Writes
///   the first round's candidate count and
/// the per-rule dropped counts into the report.
fn pick(
    snapshot: &Snapshot,
    snapshot_links: &[Link<'_>],
    births: &[Arc<Birth>],
    w: &[u32],
    attempted: &[u64],
    round: u32,
    report: &mut PassReport,
) -> Vec<(u64, Vec<u32>)> {
    let birth_links: Vec<Link<'_>> = births.iter().map(|b| b.link()).collect();
    let exited = |link: &Link<'_>| link_exited(snapshot, births, link);
    let view = Chain {
        root_pid: snapshot.root.pid,
        snapshot: snapshot_links,
        births: &birth_links,
        exited: &exited,
    };
    let found = candidates(&view, w);
    if round == 1 {
        report.candidates = found.len();
    }
    let mut excluded = Excluded::default();
    let mut picks = Vec::new();
    for c in found {
        let Some(seq) = c.seq else {
            continue;
        };
        match select(c, &view) {
            Verdict::Cleanup { chain } => {
                if !attempted.contains(&seq) {
                    picks.push((seq, chain));
                }
            }
            other => excluded.note(&other),
        }
    }
    report.excluded = excluded;
    picks
}

/// Has one known link exited now — asked through the handle holding that link (a birth is found by `seq`, a snapshot member by (number,
///   creation)). If not found, unknown (`Err`) —
/// which the selection reads on the side of not killing.
fn link_exited(snapshot: &Snapshot, births: &[Arc<Birth>], link: &Link<'_>) -> io::Result<bool> {
    let pin = match link.seq {
        Some(seq) => births.iter().find(|b| b.seq == seq).map(|b| &b.pin),
        None => std::iter::once(&snapshot.root)
            .chain(&snapshot.others)
            .find(|m| m.pid == link.pid && m.pin.facts().create == link.facts.create)
            .map(|m| &m.pin),
    };
    match pin {
        Some(pin) => pin.exited(),
        None => Err(io::Error::from(io::ErrorKind::NotFound)),
    }
}

fn recheck(snapshot: &Snapshot, retiring: &RetiringSignal) -> Result<(), NotReady> {
    if retiring.is_set() {
        return Err(NotReady::Retiring);
    }
    root_alive(snapshot)
}

/// Confirm the exits of what one round killed — one deadline is shared (the later ones wait as much less as the earlier ones waited).
///   Gives the number whose exit was
/// confirmed within the deadline.
fn confirm(clock: &dyn LeftoverClock, killed: &[&Birth]) -> usize {
    if killed.is_empty() {
        return 0;
    }
    let deadline = clock.now().checked_add(KILL_CONFIRM);
    killed
        .iter()
        .filter(|birth| {
            let left =
                deadline.map_or(Duration::ZERO, |d| d.saturating_duration_since(clock.now()));
            matches!(birth.pin.wait_exit(left), Ok(true))
        })
        .count()
}

/// The pass's log (TRD §3-8) — after releasing the lock. If it killed anything, leave one cleaned warn; the stop reason goes at that
///   reason's level;
/// a pass that ran through without killing anything gets one could-not-kill warn.
fn log_pass<G: Group>(cleaner: &Cleaner<G>, report: &PassReport, waited: Duration) {
    let (agent, root_pid) = (cleaner.tag.agent(), cleaner.tag.root_pid);
    let waited_ms = millis(waited);
    let x = report.excluded;
    let killed = report.terminated.len();
    if killed > 0 {
        tracing::warn!(
            agent,
            root_pid,
            waited_ms,
            terminated = ?report.terminated,
            rounds = report.rounds,
            terminate_max_us = micros(report.terminate_max),
            unconfirmed = killed.saturating_sub(report.confirmed),
            births = report.births,
            parent_alive = x.parent_alive,
            parent_unknown = x.parent_unknown,
            not_hook_copy = x.not_hook_copy,
            ancestor_alive = x.ancestor_alive,
            chain_cut = x.chain_cut,
            not_hook = x.not_hook,
            no_taskkill = x.no_taskkill,
            unknown = x.unknown,
            "no turn end after the interrupt, so the hook leftovers inside our Job were ended"
        );
    }
    const NOTHING: &str = "no turn end after the interrupt, yet no leftover was ended";
    match &report.stop {
        Some(PassStop::NotReady(NotReady::Retiring)) => tracing::debug!(
            agent,
            root_pid,
            reason = "retiring",
            killed,
            "leftover cleanup — the group is retreating, so the pass stopped"
        ),
        Some(PassStop::NotReady(NotReady::Snapshot(cause))) => tracing::warn!(
            agent,
            root_pid,
            waited_ms,
            reason = "snapshot_failed",
            cause = cause.label(),
            "{NOTHING} — the snapshot was not set up"
        ),
        Some(PassStop::NotReady(NotReady::Released)) => tracing::warn!(
            agent,
            root_pid,
            waited_ms,
            reason = "released",
            "{NOTHING} — the snapshot was already released"
        ),
        Some(PassStop::NotReady(NotReady::RootGone)) => tracing::debug!(
            agent,
            root_pid,
            reason = "root_gone",
            killed,
            "leftover cleanup — the root ended, so the pass stopped"
        ),
        Some(PassStop::NotReady(NotReady::RootUnknown(e))) => tracing::warn!(
            agent,
            root_pid,
            waited_ms,
            reason = "root_unknown",
            killed,
            "not knowing whether the root ended, the leftover cleanup pass stopped: {e}"
        ),
        Some(PassStop::Superseded) => tracing::debug!(
            agent,
            root_pid,
            reason = "superseded",
            killed,
            "leftover cleanup — a turn end, a new interrupt or new input came during the pass, so the remaining candidates and the next round were dropped"
        ),
        Some(PassStop::RecordGone) => tracing::debug!(
            agent,
            root_pid,
            reason = "record_gone",
            killed,
            "leftover cleanup — the birth record was turned off during the pass, so the pass stopped"
        ),
        Some(PassStop::RoundsExhausted) => tracing::warn!(
            agent,
            root_pid,
            waited_ms,
            reason = "rounds_exhausted",
            rounds = report.rounds,
            killed,
            "leftover cleanup reached the round cap, so the remaining candidates were not ended"
        ),
        None if killed > 0 => {}
        None if report.candidates == 0 => tracing::warn!(
            agent,
            root_pid,
            waited_ms,
            reason = "no_new_member",
            births = report.births,
            full = report.full,
            "{NOTHING} — no live process was born after the write list"
        ),
        None if report.cleanups == 0 => tracing::warn!(
            agent,
            root_pid,
            waited_ms,
            reason = "parent_rule",
            births = report.births,
            parent_alive = x.parent_alive,
            parent_unknown = x.parent_unknown,
            not_hook_copy = x.not_hook_copy,
            ancestor_alive = x.ancestor_alive,
            chain_cut = x.chain_cut,
            not_hook = x.not_hook,
            no_taskkill = x.no_taskkill,
            unknown = x.unknown,
            "{NOTHING} — every candidate was excluded by the rules"
        ),
        None => tracing::warn!(
            agent,
            root_pid,
            waited_ms,
            reason = "all_failed",
            failed = report.failed,
            gone = report.gone,
            "{NOTHING} — could not end any of the chosen leftovers"
        ),
    }
}

/// Why pass steps 1–3 (TRD §3-4) did not hold.
#[derive(Debug)]
enum NotReady {
    /// The group is retiring (debug `retiring`).
    Retiring,
    /// warn `snapshot_failed` — `Port` if that record was off at pass time (listener thread ended · failed).
    Snapshot(SnapCause),
    /// The pass end already released the snapshot — a branch that is not reached.
    Released,
    /// The held root has exited (debug `root_gone`).
    RootGone,
    /// The root exit query failed (warn `root_unknown`).
    RootUnknown(io::Error),
}

/// Pass steps 1–3 — no marker → port not latched (`port_failed`) · snapshot set up and that record still on (`active` = number now on) →
///   held root alive.
/// A latched port may have missed members, so it is `snapshot_failed(port)`, not 「no new members」. The root exit query is an OS call:
///   outside both locks.
fn precheck(
    ticket: &Ticket,
    active: u64,
    retiring: bool,
    port_failed: bool,
) -> Result<&Snapshot, NotReady> {
    if retiring {
        return Err(NotReady::Retiring);
    }
    if port_failed {
        return Err(NotReady::Snapshot(SnapCause::Port));
    }
    let snap = match &ticket.snap {
        SnapState::Taken(snap) if ticket.rec != 0 && active == ticket.rec => snap,
        SnapState::Taken(_) => return Err(NotReady::Snapshot(SnapCause::Port)),
        SnapState::Failed(cause) => return Err(NotReady::Snapshot(*cause)),
        SnapState::Released => return Err(NotReady::Released),
    };
    root_alive(snap)?;
    Ok(snap)
}

fn root_alive(snapshot: &Snapshot) -> Result<(), NotReady> {
    match snapshot.root.pin.exited() {
        Ok(false) => Ok(()),
        Ok(true) => Err(NotReady::RootGone),
        Err(e) => Err(NotReady::RootUnknown(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real values from `.claude/handoff/attachments/20260930-t40-provenance-spike/procs/*.procs.tsv` (claude 2.1.284); times are ms from
    //   the interrupt line write
    //   (first taskkill − 5 ms · p3bg: TRD §3-0 table basis). Command-line tails unrelated to the rules are shortened.

    const CMD: &str = r"C:\Windows\System32\cmd.exe";
    const CONHOST: &str = r"C:\Windows\System32\conhost.exe";
    const CLAUDE: &str = r"C:\Users\kimsunzun\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe";
    const BIN_BASH: &str = r"C:\Program Files\Git\bin\bash.exe";
    const USR_BASH: &str = r"C:\Program Files\Git\usr\bin\bash.exe";
    const SLEEP: &str = r"C:\Program Files\Git\usr\bin\sleep.exe";
    const CAT: &str = r"C:\Program Files\Git\usr\bin\cat.exe";
    const TASKKILL: &str = r"C:\Windows\System32\taskkill.exe";
    const NODE: &str = r"C:\Program Files\nodejs\node.exe";

    const ROOT_CMD: &str =
        r#""cmd.exe" /c claude --permission-mode bypassPermissions -p --input-format stream-json"#;
    const CONHOST_CMD: &str = r"\??\C:\WINDOWS\system32\conhost.exe 0xffffffff -ForceV1";
    const CLAUDE_CMD: &str = r#""C:\Users\kimsunzun\AppData\Roaming\npm\\node_modules\@anthropic-ai\claude-code\bin\claude.exe"    --permission-mode bypassPermissions -p --input-format stream-json"#;

    const WIKI: &str = "wiki-preconsult";
    const HANDOFF: &str = "handoff-trigger";

    /// FILETIME when the interrupt line was written — the table's times are measured in ms from this point.
    const T0: i64 = 134_352_256_000_000_000;
    const TICKS_PER_MS: i64 = 10_000;
    /// pass = when the line was written + 3 s.
    const PASS_MS: i64 = 3_000;

    /// Hook runner — the shape of the top T that claude starts for each hook.
    fn launcher(hook: &str) -> String {
        format!(
            r#""C:\Program Files\Git\bin\bash.exe" -c "bash ${{CLAUDE_PLUGIN_ROOT}}/hooks/{hook}.sh""#
        )
    }

    /// Wrapper shell — the `usr\bin` bash started by the runner. Its command line is the same `-c "bash …"` as the runner's.
    fn shell(hook: &str) -> String {
        format!(
            r#""C:\Program Files\Git\bin\..\usr\bin\bash.exe" -c "bash ${{CLAUDE_PLUGIN_ROOT}}/hooks/{hook}.sh""#
        )
    }

    /// The hook script the wrapper shell exec'd — its fork copies have the same command line too.
    fn script(hook: &str) -> String {
        format!(
            r#""C:\Program Files\Git\usr\bin\bash.exe" C:/Users/kimsunzun/.claude/plugins/cache/nmfc-origin-plugins/nmfc-origin-dev-harness/2.11.0/hooks/{hook}.sh"#
        )
    }

    fn taskkill(target: u32) -> String {
        format!(r"C:\WINDOWS\System32\taskkill.exe /PID {target} /T /F")
    }

    /// Command lines of the Bash tool runner · its wrapper shell · fork (p1's background loop `while true; do sleep 2; done`).
    fn bash_tool(program: &str) -> String {
        format!(
            r#""{program}" -c "source /c/Users/kimsunzun/.claude/shell-snapshots/snapshot-bash-1790752041336-sy2let.sh 2>/dev/null || true && eval 'T40BG=1; while true; do sleep 2; done' < /dev/null && pwd -P >| '/c/Users/KIMSUN~1/AppData/Local/Temp/claude-5f7e-cwd'""#
        )
    }

    fn facts(ppid: u32, create: u64, image: &str, cmdline: &str) -> ProcessFacts {
        ProcessFacts {
            ppid,
            create,
            image: image.to_owned(),
            cmdline: cmdline.to_owned(),
        }
    }

    fn snap(pid: u32, facts: &ProcessFacts) -> Link<'_> {
        Link {
            pid,
            facts,
            seq: None,
            killable: true,
        }
    }

    fn birth(pid: u32, seq: u64, facts: &ProcessFacts) -> Link<'_> {
        Link {
            pid,
            facts,
            seq: Some(seq),
            killable: true,
        }
    }

    struct Proc {
        pid: u32,
        born_ms: i64,
        /// `None` = alive at pass time.
        exit_ms: Option<i64>,
        /// `false` = the join notification was missed (not in the record).
        recorded: bool,
        facts: ProcessFacts,
    }

    /// A fake world modeled on the table — what was alive at the time the line was written (0 ms) is the snapshot, those with 0 < creation
    ///   ≤ pass are the birth record (in creation order
    /// `seq`), and the write list W is the numbers of those alive at 0 ms.
    pub(super) struct World {
        root: u32,
        procs: Vec<Proc>,
        failing: Vec<u32>,
    }

    impl World {
        fn new(root: u32) -> Self {
            World {
                root,
                procs: Vec::new(),
                failing: Vec::new(),
            }
        }

        /// Root (`cmd /c`) · its console host · claude — all three are alive at pass time.
        fn hosted(root: u32, conhost: u32, claude: u32) -> Self {
            let mut w = World::new(root);
            w.add(root, 26792, -60_000, None, CMD, ROOT_CMD)
                .add(conhost, root, -60_000, None, CONHOST, CONHOST_CMD)
                .add(claude, root, -59_900, None, CLAUDE, CLAUDE_CMD);
            w
        }

        fn add(
            &mut self,
            pid: u32,
            ppid: u32,
            born_ms: i64,
            exit_ms: Option<i64>,
            image: &str,
            cmdline: impl AsRef<str>,
        ) -> &mut Self {
            let create = (T0 + born_ms * TICKS_PER_MS) as u64;
            self.procs.push(Proc {
                pid,
                born_ms,
                exit_ms,
                recorded: true,
                facts: facts(ppid, create, image, cmdline.as_ref()),
            });
            self
        }

        fn proc_mut(&mut self, pid: u32, born_ms: i64) -> &mut Proc {
            self.procs
                .iter_mut()
                .find(|p| p.pid == pid && p.born_ms == born_ms)
                .expect("a process that is in the world")
        }

        fn lose(&mut self, pid: u32, born_ms: i64) -> &mut Self {
            self.proc_mut(pid, born_ms).recorded = false;
            self
        }

        fn set_exit(&mut self, pid: u32, born_ms: i64, exit_ms: Option<i64>) -> &mut Self {
            self.proc_mut(pid, born_ms).exit_ms = exit_ms;
            self
        }

        fn alive_at(p: &Proc, ms: i64) -> bool {
            p.born_ms <= ms && p.exit_ms.map_or(true, |e| e > ms)
        }

        fn links(&self) -> (Vec<Link<'_>>, Vec<Link<'_>>) {
            let snapshot = self
                .procs
                .iter()
                .filter(|p| p.recorded && Self::alive_at(p, 0))
                .map(|p| snap(p.pid, &p.facts))
                .collect();
            let mut born: Vec<&Proc> = self
                .procs
                .iter()
                .filter(|p| p.recorded && p.born_ms > 0 && p.born_ms <= PASS_MS)
                .collect();
            born.sort_by_key(|p| p.born_ms);
            let births = born
                .iter()
                .zip(0u64..)
                .map(|(p, seq)| birth(p.pid, seq, &p.facts))
                .collect();
            (snapshot, births)
        }

        fn write_list(&self) -> Vec<u32> {
            let mut w: Vec<u32> = self
                .procs
                .iter()
                .filter(|p| Self::alive_at(p, 0))
                .map(|p| p.pid)
                .collect();
            w.sort_unstable();
            w
        }

        fn exited(&self, link: &Link<'_>, killed: &[u32]) -> io::Result<bool> {
            if self.failing.contains(&link.pid) {
                return Err(io::Error::new(io::ErrorKind::Other, "fake query failure"));
            }
            let p = self
                .procs
                .iter()
                .find(|p| p.pid == link.pid && p.facts.create == link.facts.create)
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "a link not in the world")
                })?;
            Ok(killed.contains(&p.pid) || p.exit_ms.is_some_and(|e| e <= PASS_MS))
        }

        fn judge(&self, pid: u32, killed: &[u32]) -> Verdict {
            let (snapshot, births) = self.links();
            let view = Chain {
                root_pid: self.root,
                snapshot: &snapshot,
                births: &births,
                exited: &|l: &Link<'_>| self.exited(l, killed),
            };
            let c = births
                .iter()
                .rev()
                .find(|l| l.pid == pid)
                .expect("the candidate is in the birth record");
            select(c, &view)
        }

        fn round(&self, killed: &[u32]) -> Vec<u32> {
            let (snapshot, births) = self.links();
            let view = Chain {
                root_pid: self.root,
                snapshot: &snapshot,
                births: &births,
                exited: &|l: &Link<'_>| self.exited(l, killed),
            };
            candidates(&view, &self.write_list())
                .into_iter()
                .filter(|c| matches!(select(c, &view), Verdict::Cleanup { .. }))
                .map(|c| c.pid)
                .collect()
        }

        pub(super) fn staged(&self) -> Staged {
            let state = |p: &Proc| {
                (
                    p.pid,
                    p.facts.clone(),
                    p.exit_ms.is_some_and(|e| e <= PASS_MS),
                )
            };
            let mut born: Vec<&Proc> = self
                .procs
                .iter()
                .filter(|p| p.recorded && p.born_ms > 0 && p.born_ms <= PASS_MS)
                .collect();
            born.sort_by_key(|p| p.born_ms);
            Staged {
                root: self.root,
                listing: self.write_list(),
                members: self
                    .procs
                    .iter()
                    .filter(|p| p.recorded && Self::alive_at(p, 0))
                    .map(state)
                    .collect(),
                births: born.into_iter().map(state).collect(),
            }
        }
    }

    /// [`World::staged`] — number · facts · whether it had exited at pass time.
    ///
    /// - `listing` = the listing when the line was written (= write list W), including those whose notification was missed.
    pub(super) struct Staged {
        pub(super) root: u32,
        pub(super) listing: Vec<u32>,
        pub(super) members: Vec<(u32, ProcessFacts, bool)>,
        pub(super) births: Vec<(u32, ProcessFacts, bool)>,
    }

    fn cleanup(chain: &[u32]) -> Verdict {
        Verdict::Cleanup {
            chain: chain.to_vec(),
        }
    }

    // ── Spike shapes ─────────────────────────────────────────────────────────────

    /// 09-30 p1 — owner ← hook bash D ← wrapper shell ← runner T; D and everything above it is the snapshot.
    pub(super) fn p1_chain() -> World {
        let mut w = World::hosted(29284, 36360, 35724);
        w.add(26208, 35724, -182, Some(330), BIN_BASH, launcher(WIKI))
            .add(22700, 26208, -115, Some(330), USR_BASH, shell(WIKI))
            .add(36088, 22700, -56, Some(330), USR_BASH, script(WIKI))
            .add(38108, 36088, 298, None, USR_BASH, script(WIKI));
        w
    }

    pub(super) fn p1() -> World {
        let mut w = p1_chain();
        w.add(24140, 35724, 5, Some(342), TASKKILL, taskkill(33432))
            .add(7164, 35724, 9, Some(334), TASKKILL, taskkill(26208))
            .add(25660, 35724, 1511, Some(1903), TASKKILL, taskkill(33432))
            .add(3548, 35724, 1516, Some(1909), TASKKILL, taskkill(26208));
        w
    }

    /// Add to p1 one more owner 38200, a copy of the same hook bash — two `Cleanup` in one round (not a measured shape).
    pub(super) fn p1_two_owners() -> World {
        let mut w = p1();
        w.add(38200, 36088, 310, None, USR_BASH, script(WIKI));
        w
    }

    /// 09-30 p3t3 — owner ← D fork (birth) ← hook bash ← wrapper shell ← runner.
    pub(super) fn p3t3() -> World {
        let mut w = World::hosted(37152, 34960, 40848);
        w.add(33144, 40848, -173, Some(540), BIN_BASH, launcher(WIKI))
            .add(10400, 33144, -85, Some(539), USR_BASH, shell(WIKI))
            .add(27108, 10400, -3, Some(534), USR_BASH, script(WIKI))
            .add(26732, 27108, 282, Some(533), USR_BASH, script(WIKI))
            .add(23604, 26732, 457, None, USR_BASH, script(WIKI))
            .add(26780, 40848, 5, Some(548), TASKKILL, taskkill(38240))
            .add(38612, 40848, 10, Some(549), TASKKILL, taskkill(33144))
            .add(27144, 40848, 1511, Some(1911), TASKKILL, taskkill(38240))
            .add(32532, 40848, 1518, Some(1940), TASKKILL, taskkill(33144));
        w
    }

    /// pilot p1bg — owner ← D hook bash (birth) ← wrapper shell ← runner.
    pub(super) fn pilot() -> World {
        let mut w = World::hosted(19888, 28960, 5212);
        w.add(25096, 5212, -96, Some(392), BIN_BASH, launcher(WIKI))
            .add(37948, 25096, -36, Some(392), USR_BASH, shell(WIKI))
            .add(37552, 37948, 20, Some(392), USR_BASH, script(WIKI))
            .add(3256, 37552, 356, None, USR_BASH, script(WIKI))
            .add(28116, 5212, 5, Some(378), TASKKILL, taskkill(33556))
            .add(38848, 5212, 9, Some(397), TASKKILL, taskkill(25096))
            .add(30872, 5212, 1510, Some(1801), TASKKILL, taskkill(33556))
            .add(32396, 5212, 1515, Some(1795), TASKKILL, taskkill(25096));
        w
    }

    /// 09-30 p3bg — runner · wrapper shell · owner are all births; the owner is a script exec'd by the wrapper shell (command line
    ///   differs) and parents fork 24776,
    /// alive at pass time; the other births alive at pass time (taskkill · conhost · Bash-loop `sleep` · another hook's `cat`) are included.
    pub(super) fn p3bg() -> World {
        let mut w = World::hosted(37152, 34960, 40848);
        w.add(35688, 40848, 74, Some(1722), BIN_BASH, launcher(HANDOFF))
            .add(12180, 40848, 125, Some(1761), TASKKILL, taskkill(35688))
            .add(
                31756,
                20868,
                860,
                Some(1375),
                USR_BASH,
                bash_tool(r"C:\Program Files\Git\bin\..\usr\bin\bash.exe"),
            )
            .add(25028, 35688, 871, Some(1695), USR_BASH, shell(HANDOFF))
            .add(
                9032,
                31756,
                1078,
                None,
                SLEEP,
                r#""C:\Program Files\Git\usr\bin\sleep.exe" 2"#,
            )
            .add(32228, 25028, 1432, None, USR_BASH, script(HANDOFF))
            .add(25768, 32228, 1659, Some(1731), USR_BASH, script(HANDOFF))
            .add(29864, 40848, 1665, None, TASKKILL, taskkill(27196))
            .add(32648, 40848, 1671, None, TASKKILL, taskkill(35688))
            .add(39424, 32648, 1708, None, CONHOST, CONHOST_CMD)
            .add(1672, 28436, 1738, Some(2150), USR_BASH, script(WIKI))
            .add(
                30144,
                1672,
                1808,
                None,
                CAT,
                r#""C:\Program Files\Git\usr\bin\cat.exe""#,
            )
            .add(38576, 29864, 2073, None, CONHOST, CONHOST_CMD)
            .add(24776, 32228, 2755, None, USR_BASH, script(HANDOFF));
        w
    }

    /// Two more forks follow below p3bg's fork 24776 (24776 → 24800 → 24810) — one becomes a candidate per round, so four rounds are
    /// needed (not a measured shape).
    pub(super) fn p3bg_deeper() -> World {
        let mut w = p3bg();
        w.add(24800, 24776, 2800, None, USR_BASH, script(HANDOFF))
            .add(24810, 24800, 2850, None, USR_BASH, script(HANDOFF));
        w
    }

    #[test]
    fn p1_owner_is_cleaned_with_its_chain_and_nothing_else() {
        let w = p1();
        assert_eq!(w.judge(38108, &[]), cleanup(&[38108, 36088, 22700, 26208]));
        assert_eq!(w.round(&[]), vec![38108]);
        assert!(w.round(&[38108]).is_empty());
    }

    #[test]
    fn p3t3_owner_under_a_recorded_fork_is_cleaned() {
        let w = p3t3();
        assert_eq!(
            w.judge(23604, &[]),
            cleanup(&[23604, 26732, 27108, 10400, 33144])
        );
        assert_eq!(w.round(&[]), vec![23604]);
        assert!(w.round(&[23604]).is_empty());
    }

    #[test]
    fn pilot_owner_is_cleaned() {
        let w = pilot();
        assert_eq!(w.judge(3256, &[]), cleanup(&[3256, 37552, 37948, 25096]));
        assert_eq!(w.round(&[]), vec![3256]);
        assert!(w.round(&[3256]).is_empty());
    }

    #[test]
    fn p3bg_owner_first_then_its_fork_once_the_owner_is_gone() {
        let w = p3bg();
        assert_eq!(w.judge(32228, &[]), cleanup(&[32228, 25028, 35688]));
        assert_eq!(w.judge(24776, &[]), Verdict::ParentAlive);
        assert_eq!(w.round(&[]), vec![32228]);

        assert_eq!(
            w.judge(24776, &[32228]),
            cleanup(&[24776, 32228, 25028, 35688])
        );
        assert_eq!(w.round(&[32228]), vec![24776]);
        assert!(w.round(&[32228, 24776]).is_empty());
    }

    #[test]
    fn p3bg_other_births_alive_at_the_pass_are_left_alone() {
        let w = p3bg();
        assert_eq!(w.judge(32648, &[]), Verdict::ParentAlive, "taskkill");
        assert_eq!(
            w.judge(29864, &[]),
            Verdict::ParentAlive,
            "another hook's taskkill"
        );
        assert_eq!(
            w.judge(39424, &[]),
            Verdict::ParentAlive,
            "taskkill's conhost"
        );
        assert_eq!(w.judge(9032, &[]), Verdict::NotHookCopy, "the loop's sleep");
        assert_eq!(
            w.judge(30144, &[]),
            Verdict::NotHookCopy,
            "cat launched by the hook"
        );
    }

    // ── taskkill markers ─────────────────────────────────────────────────────────────

    #[test]
    fn no_taskkill_birth_is_no_taskkill() {
        assert_eq!(p1_chain().judge(38108, &[]), Verdict::NoTaskkill);
    }

    #[test]
    fn taskkill_naming_another_pid_is_no_taskkill() {
        let mut w = p1_chain();
        w.add(24140, 35724, 5, Some(342), TASKKILL, taskkill(33432))
            .add(25660, 35724, 1511, Some(1903), TASKKILL, taskkill(33432));
        assert_eq!(w.judge(38108, &[]), Verdict::NoTaskkill);
    }

    #[test]
    fn taskkill_not_spawned_by_claude_is_no_taskkill() {
        let mut w = p1_chain();
        w.add(7164, 36360, 9, Some(334), TASKKILL, taskkill(26208));
        assert_eq!(w.judge(38108, &[]), Verdict::NoTaskkill);
    }

    #[test]
    fn taskkill_older_than_the_top_is_no_taskkill() {
        // Imitating number reuse — a taskkill that pointed at the old top was born before the new T of the same number.
        let mut w = p3bg();
        w.proc_mut(12180, 125).facts.create = (T0 + 60 * TICKS_PER_MS) as u64;
        w.proc_mut(32648, 1671).facts.create = (T0 + 70 * TICKS_PER_MS) as u64;
        assert_eq!(w.judge(32228, &[]), Verdict::NoTaskkill);
    }

    // ── Chain ──────────────────────────────────────────────────────────────────────

    #[test]
    fn reused_number_younger_than_the_child_is_not_its_parent() {
        // The notification for D fork 26732 was missed and its number was reused by a process born later than the owner.
        let mut w = p3t3();
        w.lose(26732, 282)
            .add(26732, 27108, 600, Some(900), USR_BASH, script(WIKI));
        assert_eq!(w.judge(23604, &[]), Verdict::ParentUnknown);

        // When the number of the middle link (wrapper shell 25028) is reused — the chain breaks at the fork of round 2.
        let mut w = p3bg();
        w.lose(25028, 871)
            .add(25028, 40848, 2000, Some(2500), USR_BASH, shell(HANDOFF));
        assert_eq!(w.judge(32228, &[]), Verdict::ParentUnknown);
        assert_eq!(w.judge(24776, &[32228]), Verdict::ChainCut);
    }

    #[test]
    fn lost_middle_link_cuts_the_chain() {
        let mut w = p3bg();
        w.lose(25028, 871);
        assert_eq!(w.judge(24776, &[32228]), Verdict::ChainCut);
        assert!(w.round(&[32228]).is_empty());
    }

    /// p1's Bash tool background loop — runner 37280 · looping wrapper shell 36052 (both snapshot · alive) · the per-iteration fork
    /// 35732 · its `sleep` 9408. 35740 is a subshell (copy) of the same fork, added artificially.
    fn bash_tool_loop() -> World {
        let usr = r"C:\Program Files\Git\bin\..\usr\bin\bash.exe";
        let mut w = World::hosted(29284, 36360, 35724);
        w.add(37280, 35724, -10_216, None, BIN_BASH, bash_tool(BIN_BASH))
            .add(36052, 37280, -10_188, None, USR_BASH, bash_tool(usr))
            .add(35732, 36052, 2318, Some(2372), USR_BASH, bash_tool(usr))
            .add(35740, 35732, 2330, None, USR_BASH, bash_tool(usr))
            .add(
                9408,
                35732,
                2350,
                None,
                SLEEP,
                r#""C:\Program Files\Git\usr\bin\sleep.exe" 2"#,
            );
        w
    }

    #[test]
    fn loop_sleep_is_not_a_hook_copy() {
        assert_eq!(bash_tool_loop().judge(9408, &[]), Verdict::NotHookCopy);
    }

    #[test]
    fn subshell_under_a_live_loop_is_ancestor_alive() {
        assert_eq!(
            bash_tool_loop().judge(35740, &[]),
            Verdict::AncestorAlive { pid: 36052 }
        );
    }

    #[test]
    fn subshell_after_the_whole_bash_tool_chain_died_is_not_hook() {
        let mut w = bash_tool_loop();
        w.set_exit(36052, -10_188, Some(2400))
            .set_exit(37280, -10_216, Some(2450));
        assert_eq!(w.judge(35740, &[]), Verdict::NotHook);
        assert!(w.round(&[]).is_empty());
    }

    #[test]
    fn cycle_of_equal_creates_cuts_the_chain() {
        let mut w = World::hosted(1, 2, 3);
        w.add(901, 902, 100, Some(1000), USR_BASH, script(WIKI))
            .add(902, 901, 100, Some(1000), USR_BASH, script(WIKI))
            .add(900, 901, 200, None, USR_BASH, script(WIKI));
        assert_eq!(w.judge(900, &[]), Verdict::ChainCut);
    }

    /// claude ← runner 100 ← 101 ← … ← `100 + n − 1`(D) ← candidate 200 — `n` ancestors · all exited · taskkill present.
    fn tower(n: u32) -> World {
        let mut w = World::hosted(1, 2, 3);
        w.add(100, 3, 10, Some(1000), BIN_BASH, launcher(WIKI));
        for i in 1..n {
            w.add(
                100 + i,
                100 + i - 1,
                10 + i64::from(i),
                Some(1000),
                USR_BASH,
                script(WIKI),
            );
        }
        w.add(200, 100 + n - 1, 500, None, USR_BASH, script(WIKI))
            .add(300, 3, 40, Some(400), TASKKILL, taskkill(100));
        w
    }

    #[test]
    fn ancestor_walk_stops_past_the_cap() {
        let at_cap = tower(16).judge(200, &[]);
        let expected: Vec<u32> = std::iter::once(200).chain((100..116).rev()).collect();
        assert_eq!(at_cap, Verdict::Cleanup { chain: expected });
        assert_eq!(tower(17).judge(200, &[]), Verdict::ChainCut);
    }

    #[test]
    fn parent_in_claudes_seat_is_k_equals_one() {
        // Treat claude as exited and make its copy the candidate — D is itself a child of the root, so there is no top.
        let mut w = World::hosted(1, 2, 3);
        w.set_exit(3, -59_900, Some(1000))
            .add(200, 3, 500, None, CLAUDE, CLAUDE_CMD);
        assert_eq!(w.judge(200, &[]), Verdict::ChainCut);
    }

    #[test]
    fn reaching_the_root_first_cuts_the_chain() {
        let mut w = World::hosted(1, 2, 3);
        w.set_exit(1, -60_000, Some(1000))
            .add(200, 1, 500, None, CMD, ROOT_CMD);
        assert_eq!(w.judge(200, &[]), Verdict::ChainCut);
    }

    /// A launcher sits between the root and claude — root 1 ← launcher 4 ← claude 3 ← hook chain.
    fn behind_a_launcher() -> World {
        let mut w = World::new(1);
        w.add(1, 26792, -60_000, None, CMD, ROOT_CMD)
            .add(
                4,
                1,
                -60_000,
                None,
                NODE,
                r#""C:\Program Files\nodejs\node.exe" claude.js"#,
            )
            .add(3, 4, -59_900, None, CLAUDE, CLAUDE_CMD)
            .add(100, 3, 10, Some(1000), BIN_BASH, launcher(WIKI))
            .add(101, 100, 20, Some(1000), USR_BASH, shell(WIKI))
            .add(102, 101, 30, Some(1000), USR_BASH, script(WIKI))
            .add(200, 102, 500, None, USR_BASH, script(WIKI))
            .add(300, 3, 15, Some(400), TASKKILL, taskkill(100));
        w
    }

    #[test]
    fn launcher_between_root_and_claude_is_missed() {
        // The walk meets claude as an ancestor — if alive, it stops there; if exited, the top is claude.exe so it is missed.
        assert_eq!(
            behind_a_launcher().judge(200, &[]),
            Verdict::AncestorAlive { pid: 3 }
        );
        let mut w = behind_a_launcher();
        w.set_exit(3, -59_900, Some(2000));
        assert_eq!(w.judge(200, &[]), Verdict::NotHook);
    }

    // ── Unknown ──────────────────────────────────────────────────────────────────

    #[test]
    fn liveness_errors_and_empty_fields_are_unknown() {
        let mut w = p1();
        w.failing.push(36088);
        assert_eq!(
            w.judge(38108, &[]),
            Verdict::Unknown,
            "D's exit query failed"
        );

        let mut w = p1();
        w.failing.push(22700);
        assert_eq!(
            w.judge(38108, &[]),
            Verdict::Unknown,
            "an ancestor's exit query failed"
        );

        let mut w = p1();
        w.proc_mut(38108, 298).facts.cmdline.clear();
        assert_eq!(
            w.judge(38108, &[]),
            Verdict::Unknown,
            "candidate command line empty"
        );

        let mut w = p1();
        w.proc_mut(36088, -56).facts.image.clear();
        assert_eq!(w.judge(38108, &[]), Verdict::Unknown, "D executable empty");

        let mut w = p1();
        w.root = 0;
        assert_eq!(w.judge(38108, &[]), Verdict::Unknown, "root pid unknown");
    }

    #[test]
    fn unreadable_parent_number_or_create_is_parent_unknown() {
        let mut w = p1();
        w.proc_mut(38108, 298).facts.ppid = 0;
        assert_eq!(w.judge(38108, &[]), Verdict::ParentUnknown);

        let mut w = p1();
        w.proc_mut(36088, -56).facts.create = 0;
        assert_eq!(w.judge(38108, &[]), Verdict::ParentUnknown);

        let mut w = p1();
        w.proc_mut(38108, 298).facts.create = 0;
        assert_eq!(w.judge(38108, &[]), Verdict::ParentUnknown);
    }

    // ── Finding the parent ─────────────────────────────────────────────────────────

    fn never(_: &Link<'_>) -> io::Result<bool> {
        Err(io::Error::new(io::ErrorKind::Other, "not called"))
    }

    #[test]
    fn parent_is_the_latest_holder_born_no_later_than_the_child() {
        let old = facts(1, 100, USR_BASH, "a");
        let new = facts(1, 300, USR_BASH, "b");
        let zero = facts(1, 0, USR_BASH, "c");
        let snapshot = [snap(10, &old)];
        let births = [birth(10, 0, &new), birth(10, 1, &zero)];
        let view = Chain {
            root_pid: 1,
            snapshot: &snapshot,
            births: &births,
            exited: &never,
        };
        let parent = |create: u64| {
            let child = facts(10, create, USR_BASH, "x");
            parent_of(&view, 20, &child).map(|l| l.facts.create)
        };
        assert_eq!(parent(350), Some(300));
        assert_eq!(
            parent(300),
            Some(300),
            "the same creation can be the parent"
        );
        assert_eq!(parent(200), Some(100));
        assert_eq!(parent(50), None);

        let child = facts(10, 350, USR_BASH, "x");
        assert!(parent_of(&view, 10, &child).is_none(), "its own pid");
    }

    #[test]
    fn equal_holder_in_snapshot_and_births_resolves_to_the_snapshot() {
        let f = facts(1, 100, USR_BASH, "a");
        let snapshot = [snap(10, &f)];
        let births = [birth(10, 0, &f)];
        let view = Chain {
            root_pid: 1,
            snapshot: &snapshot,
            births: &births,
            exited: &never,
        };
        let child = facts(10, 200, USR_BASH, "x");
        assert_eq!(parent_of(&view, 20, &child).map(|l| l.seq), Some(None));
    }

    // ── Candidates ─────────────────────────────────────────────────────────────────

    #[test]
    fn candidates_are_new_killable_live_births_in_seq_order() {
        let member = facts(1, 100, USR_BASH, "m");
        let late = facts(1, 500, USR_BASH, "late");
        let early = facts(1, 400, USR_BASH, "early");
        let chain_only = facts(1, 450, USR_BASH, "chain-only");
        let in_w = facts(1, 300, USR_BASH, "in-w");
        let reused_member_number = facts(1, 600, USR_BASH, "reused");
        let dead = facts(1, 700, USR_BASH, "dead");
        let unreadable = facts(1, 800, USR_BASH, "unreadable");

        let snapshot = [snap(50, &member)];
        let births = [
            birth(60, 3, &late),
            birth(61, 1, &early),
            Link {
                killable: false,
                ..birth(62, 2, &chain_only)
            },
            birth(63, 0, &in_w),
            birth(50, 4, &member),
            birth(64, 5, &dead),
            birth(65, 6, &unreadable),
            Link {
                seq: None,
                ..birth(66, 7, &early)
            },
            birth(50, 8, &reused_member_number),
        ];
        let exited = |l: &Link<'_>| match l.pid {
            64 => Ok(true),
            65 => Err(io::Error::new(io::ErrorKind::Other, "query failure")),
            _ => Ok(false),
        };
        let view = Chain {
            root_pid: 1,
            snapshot: &snapshot,
            births: &births,
            exited: &exited,
        };
        let picked: Vec<(u32, Option<u64>)> = candidates(&view, &[63, 70])
            .iter()
            .map(|l| (l.pid, l.seq))
            .collect();
        assert_eq!(picked, vec![(61, Some(1)), (60, Some(3)), (50, Some(8))]);
    }

    // ── Markers ────────────────────────────────────────────────────────────────────

    fn cmd(cmdline: &str) -> ProcessFacts {
        facts(3, 1, BIN_BASH, cmdline)
    }

    #[test]
    fn hook_command_accepts_the_launcher_and_shell_shapes() {
        assert!(is_hook_command(&cmd(&launcher(WIKI))));
        assert!(is_hook_command(&cmd(&shell(HANDOFF))));
        assert!(is_hook_command(&cmd(r#"bash -c "bash x.sh""#)));
        assert!(is_hook_command(&cmd(&format!("{}  \t", launcher(WIKI)))));
    }

    #[test]
    fn hook_command_rejects_other_shapes() {
        let rejected = [
            bash_tool(BIN_BASH),
            r#""C:\Program Files\Git\bin\bash.exe" -c "bash /c/Users/x/.claude/shell-snapshots/a.sh""#.to_owned(),
            r#""C:\Program Files\Git\bin\bash.exe" -c "node \"${CLAUDE_PLUGIN_ROOT}/scripts/stop-review-gate-hook.mjs\"""#.to_owned(),
            r#""C:\Program Files\Git\bin\bash.exe" -c "C:/Users/kimsunzun/.orca/agent-hooks/claude-hook.cmd || echo {}""#.to_owned(),
            r#""C:\Program Files\Git\bin\..\usr\bin\bash.exe" -lc "echo \"$PATH\"""#.to_owned(),
            r#""C:\Program Files\Git\bin\bash.exe" -lc "bash x.sh""#.to_owned(),
            r#""C:\Program Files\Git\bin\..\usr\bin\bash.exe" -c -l "SNAPSHOT_FILE='C:\Users\kimsunzun\.claude\shell-snapshots\snapshot-bash-1-a.sh'""#.to_owned(),
            script(WIKI),
            r#""C:\Program Files\Git\bin\bash.exe" -c bash x.sh"#.to_owned(),
            r#""C:\Program Files\Git\bin\bash.exe" -c "bash""#.to_owned(),
            r#""C:\Program Files\Git\bin\bash.exe" -cx "bash x.sh""#.to_owned(),
            "\"C:\\Program Files\\Git\\bin\\bash.exe\" -c \"bash x.sh\nrm -rf y\"".to_owned(),
            r#" bash -c "bash x.sh""#.to_owned(),
            r#""C:\Program Files\Git\bin\bash.exe""#.to_owned(),
            String::new(),
        ];
        for cmdline in rejected {
            assert!(!is_hook_command(&cmd(&cmdline)), "{cmdline}");
        }
    }

    #[test]
    fn taskkill_names_reads_only_the_exact_claude_shape() {
        let tk = |image: &str, ppid: u32, cmdline: &str| {
            taskkill_names(&facts(ppid, 1, image, cmdline), 40848)
        };
        assert_eq!(
            tk(TASKKILL, 40848, "taskkill.exe /PID 35688 /T /F"),
            Some(35688)
        );
        assert_eq!(tk(TASKKILL, 40848, &taskkill(35688)), Some(35688));
        assert_eq!(
            tk(
                TASKKILL,
                40848,
                r#""C:\WINDOWS\System32\taskkill.exe" /PID 35688 /T /F "#
            ),
            Some(35688)
        );
        assert_eq!(
            tk(r"C:\WINDOWS\SYSTEM32\TASKKILL.EXE", 40848, &taskkill(35688)),
            Some(35688)
        );

        let rejected = [
            "taskkill.exe /PID 35688 /F",
            "taskkill.exe /PID 35688 /T",
            "taskkill.exe /T /F /PID 35688",
            "taskkill.exe /pid 35688 /t /f",
            "taskkill.exe /PID:35688 /T /F",
            "taskkill.exe /PID 35688 /T /F /IM bash.exe",
            "taskkill.exe /PID abc /T /F",
            "taskkill.exe /PID +35688 /T /F",
            "taskkill.exe /PID 035688 /T /F",
            "taskkill.exe /PID 0 /T /F",
            "taskkill.exe /PID 99999999999 /T /F",
            "/PID 35688 /T /F",
            "",
        ];
        for cmdline in rejected {
            assert_eq!(tk(TASKKILL, 40848, cmdline), None, "{cmdline}");
        }
        assert_eq!(
            tk(CMD, 40848, &taskkill(35688)),
            None,
            "a different executable"
        );
        assert_eq!(tk("", 40848, &taskkill(35688)), None, "executable empty");
        assert_eq!(
            tk(TASKKILL, 40849, &taskkill(35688)),
            None,
            "the parent is not claude"
        );
    }
}

#[cfg(test)]
mod birth_tests {
    use super::*;

    use super::test_support::{Capture, FakePin, Line, Probe};

    use std::collections::HashMap;
    use std::sync::mpsc::Sender;
    use std::sync::{PoisonError, Weak};
    use std::thread::{self, JoinHandle};

    use tracing::Level;

    /// How long the test waits for the listener thread — over that, it is hung.
    const PATIENCE: Duration = Duration::from_secs(10);
    const ROOT: u32 = 7;

    // ── Fakes ──────────────────────────────────────────────────────────────────────────

    fn facts(ppid: u32, create: u64) -> ProcessFacts {
        ProcessFacts {
            ppid,
            create,
            image: r"C:\Program Files\Git\usr\bin\bash.exe".to_owned(),
            cmdline: "bash x.sh".to_owned(),
        }
    }

    fn caught(pid: u32, create: u64, probe: &Arc<Probe>) -> Caught {
        Caught {
            pid,
            facts: facts(1, create),
            pin: Box::new(FakePin::new(facts(1, create), probe)),
            killable: true,
        }
    }

    fn recorded(recorder: &Recorder, r: u64, c: Caught) -> u64 {
        match recorder.admit(r, c) {
            Admit::Recorded { seq } => seq,
            _ => panic!("a birth that should have gone in did not"),
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Call {
        Port,
        Spawn,
        Attach,
        Unwatch,
        Pin(u32, bool),
        PortDrop,
    }

    type Calls = Arc<Mutex<Vec<Call>>>;

    fn push(calls: &Calls, call: Call) {
        calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }

    enum Feed {
        Event(PortEvent),
        Fail,
    }

    /// A port the test feeds notifications to one at a time — each time it enters take-out, it tells the test the Job's strong count at
    ///   that moment.
    struct FakePort {
        feed: Mutex<mpsc::Receiver<Feed>>,
        waiting: Sender<usize>,
        job: Weak<()>,
        calls: Calls,
    }

    impl Births for FakePort {
        fn next(&self, _: Duration) -> io::Result<PortEvent> {
            let _ = self.waiting.send(self.job.strong_count());
            let fed = self
                .feed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .recv_timeout(PATIENCE);
            match fed {
                Ok(Feed::Event(event)) => Ok(event),
                Ok(Feed::Fail) => Err(io::Error::other("imitated take failure")),
                // There is no one to feed — it loops on timeouts like the real one (resting a little so it does not spin).
                Err(_) => {
                    thread::sleep(Duration::from_millis(10));
                    Ok(PortEvent::Timeout)
                }
            }
        }
    }

    impl Drop for FakePort {
        fn drop(&mut self) {
            push(&self.calls, Call::PortDrop);
        }
    }

    #[derive(Clone)]
    enum Answer {
        Ours(ProcessFacts),
        Denied,
        Panic,
    }

    /// Like the real handle, it holds the Job weakly and upgrades it only while called. Records the call order.
    struct FakeGroup {
        job: Weak<()>,
        retiring: Arc<AtomicBool>,
        port: Mutex<Option<Arc<FakePort>>>,
        answers: Mutex<HashMap<(u32, bool), Answer>>,
        refuse_watch: Mutex<Option<io::ErrorKind>>,
        attach_fails: AtomicBool,
        unwatch_fails: AtomicBool,
        probe: Arc<Probe>,
        calls: Calls,
    }

    impl Group for FakeGroup {
        fn member_pids(&self) -> io::Result<Vec<u32>> {
            Ok(Vec::new())
        }

        fn pin(&self, pid: u32, kill: bool) -> io::Result<Option<Box<dyn Pinned>>> {
            let Some(_job) = self.job.upgrade() else {
                return Ok(None);
            };
            if self.probe.held() {
                self.probe.held_calls.fetch_add(1, Ordering::SeqCst);
            }
            push(&self.calls, Call::Pin(pid, kill));
            let answer = self.answers.lock().unwrap().get(&(pid, kill)).cloned();
            match answer {
                None => Ok(None),
                Some(Answer::Ours(facts)) => Ok(Some(Box::new(FakePin::new(facts, &self.probe)))),
                Some(Answer::Denied) => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
                Some(Answer::Panic) => panic!("imitated hold panic"),
            }
        }

        fn watch_births(
            &self,
            start: impl FnOnce(Arc<dyn Births>) -> io::Result<()>,
        ) -> io::Result<()> {
            let refused = *self.refuse_watch.lock().unwrap();
            if let Some(kind) = refused {
                return Err(io::Error::new(kind, "imitated missing group"));
            }
            let port = self
                .port
                .lock()
                .unwrap()
                .take()
                .expect("the port is created only once");
            push(&self.calls, Call::Port);
            let shared: Arc<dyn Births> = Arc::clone(&port) as Arc<dyn Births>;
            start(shared)?;
            push(&self.calls, Call::Attach);
            let attached = if self.attach_fails.load(Ordering::SeqCst) {
                Err(io::Error::other("imitated attach failure"))
            } else {
                Ok(())
            };
            drop(port);
            attached
        }

        fn unwatch_births(&self) -> io::Result<()> {
            push(&self.calls, Call::Unwatch);
            if self.job.strong_count() == 0 {
                Err(io::Error::new(GROUP_GONE, "imitated missing group"))
            } else if self.unwatch_fails.load(Ordering::SeqCst) {
                Err(io::Error::other("imitated detach failure"))
            } else {
                Ok(())
            }
        }

        fn is_gone(&self) -> bool {
            self.job.strong_count() == 0
        }

        fn retiring(&self) -> RetiringSignal {
            RetiringSignal::of(&self.retiring)
        }
    }

    // ── Pass ────────────────────────────────────────────────────────────────────────────

    /// A set of fake group · port · record · port acquisition. The listener thread runs one round per notification the test feeds, and the
    ///   test reads the state after seeing that
    /// thread enter the next take-out — the two never run overlapped.
    struct Rig {
        job: Option<Arc<()>>,
        group: Arc<FakeGroup>,
        recorder: Arc<Recorder>,
        probe: Arc<Probe>,
        port: Weak<FakePort>,
        watch: BirthWatch,
        feed: Sender<Feed>,
        waits: mpsc::Receiver<usize>,
        thread: Arc<Mutex<Option<JoinHandle<()>>>>,
        done_tx: Sender<()>,
        done: mpsc::Receiver<()>,
        lines: Arc<Mutex<Vec<Line>>>,
    }

    impl Rig {
        fn new() -> Self {
            let job = Arc::new(());
            let recorder = Arc::new(Recorder::new());
            let probe = Probe::new(&recorder);
            let calls = Calls::default();
            let (feed, fed) = mpsc::channel();
            let (waiting, waits) = mpsc::channel();
            let port = Arc::new(FakePort {
                feed: Mutex::new(fed),
                waiting,
                job: Arc::downgrade(&job),
                calls: Arc::clone(&calls),
            });
            let group = Arc::new(FakeGroup {
                job: Arc::downgrade(&job),
                retiring: Arc::default(),
                port: Mutex::new(Some(Arc::clone(&port))),
                answers: Mutex::default(),
                refuse_watch: Mutex::default(),
                attach_fails: AtomicBool::new(false),
                unwatch_fails: AtomicBool::new(false),
                probe: Arc::clone(&probe),
                calls,
            });
            let (done_tx, done) = mpsc::channel();
            Self {
                job: Some(job),
                group,
                recorder,
                probe,
                port: Arc::downgrade(&port),
                watch: BirthWatch::new(),
                feed,
                waits,
                thread: Arc::default(),
                done_tx,
                done,
                lines: Arc::default(),
            }
        }

        /// Start the listener thread as the test's thread — collect that thread's logs, and signal when the body ends.
        fn spawner(&self) -> impl FnOnce(Box<dyn FnOnce() + Send>) -> io::Result<()> {
            let calls = Arc::clone(&self.group.calls);
            let lines = Arc::clone(&self.lines);
            let slot = Arc::clone(&self.thread);
            let done = self.done_tx.clone();
            move |body| {
                push(&calls, Call::Spawn);
                let handle = thread::Builder::new().spawn(move || {
                    tracing::subscriber::with_default(Capture(lines), body);
                    let _ = done.send(());
                })?;
                *slot.lock().unwrap() = Some(handle);
                Ok(())
            }
        }

        fn failing_spawner(&self) -> impl FnOnce(Box<dyn FnOnce() + Send>) -> io::Result<()> {
            let calls = Arc::clone(&self.group.calls);
            move |body| {
                push(&calls, Call::Spawn);
                drop(body);
                Err(io::Error::other("imitated start failure"))
            }
        }

        fn ensure(
            &self,
            spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> io::Result<()>,
        ) -> Result<(), ()> {
            tracing::subscriber::with_default(Capture(Arc::clone(&self.lines)), || {
                self.watch
                    .ensure_with(&self.group, &self.recorder, &LogTag::new(None, ROOT), spawn)
            })
        }

        /// Until it has acquired and the listener thread enters its first take-out.
        fn listening(&self) {
            assert_eq!(self.ensure(self.spawner()), Ok(()));
            self.wait_next();
        }

        /// Until the listener thread enters take-out — the handling of the previous notification is all done. Then give the Job's strong count.
        fn wait_next(&self) -> usize {
            self.waits
                .recv_timeout(PATIENCE)
                .expect("the listening thread did not come back to take")
        }

        fn feed(&self, event: PortEvent) -> usize {
            self.feed.send(Feed::Event(event)).expect("feed");
            self.wait_next()
        }

        /// Feed one take-out error, and wait for the thread to enter the next take-out without ending.
        fn fail_take(&self) {
            self.feed.send(Feed::Fail).expect("feed");
            self.wait_next();
        }

        fn feed_last(&self, feed: Feed) {
            self.feed.send(feed).expect("feed");
            self.ended();
        }

        fn ended(&self) {
            self.done
                .recv_timeout(PATIENCE)
                .expect("the listening thread did not end");
            let handle = self.thread.lock().unwrap().take();
            if let Some(handle) = handle {
                handle
                    .join()
                    .expect("the listening thread died with a panic");
            }
        }

        fn answer(&self, pid: u32, kill: bool, answer: Answer) {
            self.group
                .answers
                .lock()
                .unwrap()
                .insert((pid, kill), answer);
        }

        fn calls(&self) -> Vec<Call> {
            self.group.calls.lock().unwrap().clone()
        }

        fn pins(&self) -> Vec<Call> {
            self.calls()
                .into_iter()
                .filter(|call| matches!(call, Call::Pin(..)))
                .collect()
        }

        /// The path releases the Job — the group disappears.
        fn vanish(&mut self) {
            self.job = None;
        }

        fn retire(&self) {
            self.group.retiring.store(true, Ordering::Release);
        }

        fn count(&self, level: Level) -> usize {
            self.lines
                .lock()
                .unwrap()
                .iter()
                .filter(|(l, _)| *l == level)
                .count()
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            // If a listener thread is still running, make it end — remove the group and run one round.
            self.job = None;
            let _ = self.feed.send(Feed::Event(PortEvent::Timeout));
        }
    }

    // ── Birth record ─────────────────────────────────────────────────────────────────────

    #[test]
    fn a_record_takes_each_pid_create_pair_once_in_seq_order() {
        let recorder = Arc::new(Recorder::new());
        let probe = Probe::new(&recorder);
        assert_eq!(recorder.active(), 0);
        let (r, left) = recorder.start();
        assert_ne!(r, 0);
        assert!(left.is_empty());
        assert_eq!(recorder.active(), r);

        let first = recorded(&recorder, r, caught(10, 100, &probe));
        let second = recorded(&recorder, r, caught(10, 200, &probe));
        assert!(first < second);
        match recorder.admit(r, caught(10, 100, &probe)) {
            Admit::Known(again) => drop(again),
            _ => panic!("the same (pid, creation) went in twice"),
        }
        assert_eq!(probe.drops(), 1);

        let births = recorder.copy(r);
        let got: Vec<_> = births
            .iter()
            .map(|b| (b.pid, b.facts.create, b.seq))
            .collect();
        assert_eq!(got, [(10, 100, first), (10, 200, second)]);
        let link = births[0].link();
        assert_eq!((link.pid, link.seq, link.killable), (10, Some(first), true));
        assert_eq!(recorder.len(), 2);
        drop(births);
        assert_eq!(probe.held_drops(), 0);
    }

    /// Those over the cap are not put in and `full` is raised (only the first overflow is `first`). A new record lowers `full` and hands
    ///   back the remaining record,
    /// and when the receiver drops it — outside the lock — the handles close.
    #[test]
    fn a_full_record_refuses_past_the_cap_and_a_new_record_clears_it() {
        let recorder = Arc::new(Recorder::new());
        let probe = Probe::new(&recorder);
        let (r, _) = recorder.start();
        for pid in 0..BIRTH_MAX as u32 {
            recorded(&recorder, r, caught(pid, 1, &probe));
        }
        assert!(!recorder.full());
        for (pid, expect_first) in [(9_999, true), (9_998, false)] {
            match recorder.admit(r, caught(pid, 1, &probe)) {
                Admit::Full { caught, first } => {
                    assert_eq!(first, expect_first);
                    drop(caught);
                }
                _ => panic!("went in past the cap"),
            }
        }
        assert!(recorder.full());
        assert_eq!(recorder.len(), BIRTH_MAX);

        let (next, left) = recorder.start();
        assert_ne!(next, r);
        assert!(!recorder.full());
        assert_eq!(recorder.len(), 0);
        assert_eq!(left.len(), BIRTH_MAX);
        let before = probe.drops();
        drop(left);
        assert_eq!(probe.drops() - before, BIRTH_MAX);
        assert_eq!(probe.held_drops(), 0);
    }

    #[test]
    fn only_the_running_record_is_copied_stopped_or_admitted() {
        let recorder = Arc::new(Recorder::new());
        let probe = Probe::new(&recorder);
        let (old, _) = recorder.start();
        recorded(&recorder, old, caught(1, 1, &probe));
        let (r, left) = recorder.start();
        assert_eq!(left.len(), 1);
        drop(left);

        assert!(recorder.copy(old).is_empty());
        assert!(recorder.stop(old).is_empty());
        assert_eq!(recorder.active(), r);
        match recorder.admit(old, caught(2, 1, &probe)) {
            Admit::Stale(late) => drop(late),
            _ => panic!("went into a record that was off"),
        }
        recorded(&recorder, r, caught(3, 1, &probe));
        assert_eq!(recorder.copy(r).len(), 1);
        assert!(recorder.copy(0).is_empty());
        assert!(recorder.stop(0).is_empty());

        let stopped = recorder.stop(r);
        assert_eq!(stopped.len(), 1);
        assert_eq!(recorder.active(), 0);
        assert!(recorder.copy(r).is_empty());
        match recorder.admit(r, caught(4, 1, &probe)) {
            Admit::Stale(late) => drop(late),
            _ => panic!("went into a record that was turned off"),
        }
        drop(stopped);
        assert_eq!(probe.held_drops(), 0);
    }

    #[test]
    fn a_poisoned_record_lock_still_records() {
        let recorder = Arc::new(Recorder::new());
        let probe = Probe::new(&recorder);
        let poisoned = panic::catch_unwind(AssertUnwindSafe(|| {
            let _held = recorder.inner.lock();
            panic!("imitated poison");
        }));
        assert!(poisoned.is_err());
        assert!(recorder.inner.is_poisoned());

        let (r, _) = recorder.start();
        recorded(&recorder, r, caught(1, 1, &probe));
        assert_eq!(recorder.copy(r).len(), 1);
        assert_eq!(recorder.len(), 1);
        assert!(!recorder.full());
        assert_eq!(recorder.stop(r).len(), 1);
        assert_eq!(recorder.active(), 0);
    }

    // ── Listener thread ──────────────────────────────────────────────────────────────────

    /// Only joins while the record is on are held right away with kill access and put in — while it is off, it does not even hold; those
    ///   outside the group · exited ·
    /// other notifications are dropped. Holding is outside the record lock, and on entering take-out the Job is not held strongly.
    #[test]
    fn the_listener_pins_joined_members_with_the_kill_right_only_while_recording() {
        let rig = Rig::new();
        rig.answer(10, true, Answer::Ours(facts(5, 100)));
        rig.listening();

        assert_eq!(rig.feed(PortEvent::Joined(10)), 1);
        assert!(rig.pins().is_empty(), "held while off");

        let (r, _) = rig.recorder.start();
        for event in [
            PortEvent::Joined(10),
            PortEvent::Joined(11),
            PortEvent::Other,
            PortEvent::Timeout,
        ] {
            assert_eq!(
                rig.feed(event),
                1,
                "held the Job strongly when entering take"
            );
        }
        assert_eq!(rig.pins(), [Call::Pin(10, true), Call::Pin(11, true)]);
        let births = rig.recorder.copy(r);
        assert_eq!(births.len(), 1);
        let birth = &births[0];
        assert_eq!(
            (birth.pid, birth.killable, &birth.facts),
            (10, true, &facts(5, 100))
        );
        assert_eq!(birth.link().seq, Some(birth.seq));
        assert_eq!(rig.probe.held_calls(), 0);
        assert_eq!(rig.probe.held_drops(), 0);
    }

    /// If kill access is refused, hold again with query access only and put it in only as a link — it cannot be a candidate but it keeps
    ///   the chain connected. If even that cannot be
    /// held, drop it.
    #[test]
    fn a_member_refused_the_kill_right_is_kept_as_a_chain_link_only() {
        let rig = Rig::new();
        rig.answer(20, true, Answer::Denied);
        rig.answer(20, false, Answer::Ours(facts(5, 100)));
        rig.answer(21, true, Answer::Denied);
        rig.answer(21, false, Answer::Denied);
        rig.answer(22, true, Answer::Denied);
        rig.listening();
        let (r, _) = rig.recorder.start();
        for pid in [20, 21, 22] {
            rig.feed(PortEvent::Joined(pid));
        }

        assert_eq!(
            rig.pins(),
            [
                Call::Pin(20, true),
                Call::Pin(20, false),
                Call::Pin(21, true),
                Call::Pin(21, false),
                Call::Pin(22, true),
                Call::Pin(22, false),
            ]
        );
        let births = rig.recorder.copy(r);
        assert_eq!(births.len(), 1);
        let link = births[0].link();
        assert_eq!((link.pid, link.killable), (20, false));

        let child = facts(20, 150);
        let links = [
            link,
            Link {
                pid: 30,
                facts: &child,
                seq: link.seq.map(|s| s + 1),
                killable: true,
            },
        ];
        let alive = |_: &Link<'_>| -> io::Result<bool> { Ok(false) };
        let view = Chain {
            root_pid: ROOT,
            snapshot: &[],
            births: &links,
            exited: &alive,
        };
        let picked: Vec<u32> = candidates(&view, &[]).iter().map(|l| l.pid).collect();
        assert_eq!(picked, [30], "a link-only entry became a candidate");
        assert_eq!(parent_of(&view, 30, &child).map(|l| l.pid), Some(20));
    }

    #[test]
    fn the_listener_releases_the_record_and_ends_once_the_group_is_gone() {
        let mut rig = Rig::new();
        rig.answer(10, true, Answer::Ours(facts(5, 100)));
        rig.listening();
        rig.recorder.start();
        rig.feed(PortEvent::Joined(10));
        assert_eq!(rig.recorder.len(), 1);

        rig.vanish();
        rig.feed_last(Feed::Event(PortEvent::Timeout));
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.recorder.len(), 0);
        assert_eq!(rig.probe.drops(), 1);
        assert_eq!(rig.probe.held_drops(), 0);
        assert!(!rig.calls().contains(&Call::Unwatch));
        assert!(
            rig.port.upgrade().is_none(),
            "the ended thread is holding the port"
        );
        assert!(!rig.recorder.port_failed());
        assert_eq!(rig.ensure(rig.spawner()), Ok(()));
        assert_eq!(rig.count(Level::WARN) + rig.count(Level::ERROR), 0);
    }

    #[test]
    fn a_retiring_group_is_detached_before_the_port_is_let_go() {
        let rig = Rig::new();
        rig.answer(10, true, Answer::Ours(facts(5, 100)));
        rig.listening();
        rig.recorder.start();
        rig.feed(PortEvent::Joined(10));

        rig.retire();
        rig.feed_last(Feed::Event(PortEvent::Timeout));
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.probe.drops(), 1);
        assert!(rig.calls().ends_with(&[Call::Unwatch, Call::PortDrop]));
        assert!(!rig.recorder.port_failed());
    }

    #[test]
    fn a_failed_take_latches_the_port_stops_the_record_and_detaches() {
        let rig = Rig::new();
        rig.answer(10, true, Answer::Ours(facts(5, 100)));
        rig.listening();
        rig.recorder.start();
        rig.feed(PortEvent::Joined(10));

        rig.feed_last(Feed::Fail);
        assert!(rig.recorder.port_failed());
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.probe.drops(), 1);
        assert_eq!(rig.probe.held_drops(), 0);
        assert!(rig.calls().ends_with(&[Call::Unwatch, Call::PortDrop]));
        assert_eq!(rig.count(Level::WARN), 1);
        assert_eq!(rig.count(Level::ERROR), 0);

        assert_eq!(rig.ensure(rig.spawner()), Err(()));
        let ports = rig.calls().iter().filter(|c| **c == Call::Port).count();
        assert_eq!(ports, 1, "created the port twice");
    }

    /// If even detaching fails in the failure mode, it does not end and only takes out and discards (it does not put into a record turned
    ///   on later either) — it ends when the group
    /// disappears.
    #[test]
    fn a_failed_detach_drains_without_recording_until_the_group_is_gone() {
        let mut rig = Rig::new();
        rig.group.unwatch_fails.store(true, Ordering::SeqCst);
        rig.answer(10, true, Answer::Ours(facts(5, 100)));
        rig.listening();
        rig.recorder.start();

        rig.fail_take();
        assert!(rig.recorder.port_failed());
        assert_eq!(rig.recorder.active(), 0);

        let (r, _) = rig.recorder.start();
        rig.feed(PortEvent::Joined(10));
        assert!(rig.pins().is_empty(), "held while only draining");
        assert_eq!(rig.recorder.len(), 0);
        assert_eq!(rig.recorder.active(), r);

        rig.vanish();
        rig.feed_last(Feed::Event(PortEvent::Timeout));
        assert_eq!(rig.recorder.active(), 0);
        let unwatches = rig.calls().iter().filter(|c| **c == Call::Unwatch).count();
        assert_eq!(unwatches, 1);
        assert!(rig.port.upgrade().is_none());
        assert_eq!(rig.count(Level::WARN), 1);
    }

    #[test]
    fn a_panicking_round_latches_the_port_like_a_failed_take() {
        let rig = Rig::new();
        rig.answer(10, true, Answer::Panic);
        rig.listening();
        rig.recorder.start();

        rig.feed_last(Feed::Event(PortEvent::Joined(10)));
        assert!(rig.recorder.port_failed());
        assert_eq!(rig.recorder.active(), 0);
        assert!(rig.calls().ends_with(&[Call::Unwatch, Call::PortDrop]));
        assert_eq!(rig.count(Level::ERROR), 1);
        assert_eq!(rig.count(Level::WARN), 0);
    }

    #[test]
    fn a_lost_attached_sender_keeps_the_listener_running() {
        let mut rig = Rig::new();
        rig.answer(10, true, Answer::Ours(facts(5, 100)));
        let port = rig.group.port.lock().unwrap().take().expect("port");
        let (attached_tx, attached) = mpsc::sync_channel::<bool>(1);
        drop(attached_tx);
        let listener = Listener {
            port,
            attached,
            group: Arc::clone(&rig.group),
            recorder: Arc::clone(&rig.recorder),
            tag: LogTag::new(None, ROOT),
        };
        (rig.spawner())(Box::new(move || listen(listener))).expect("start");
        rig.wait_next();

        let (r, _) = rig.recorder.start();
        rig.feed(PortEvent::Joined(10));
        assert_eq!(rig.recorder.copy(r).len(), 1);

        rig.vanish();
        rig.feed_last(Feed::Event(PortEvent::Timeout));
        assert_eq!(rig.recorder.active(), 0);
    }

    // ── Port acquisition ─────────────────────────────────────────────────────────────────

    #[test]
    fn the_port_comes_first_then_the_listener_then_the_attach() {
        let rig = Rig::new();
        assert_eq!(rig.ensure(rig.spawner()), Ok(()));
        rig.wait_next();
        assert_eq!(rig.calls(), [Call::Port, Call::Spawn, Call::Attach]);

        assert_eq!(rig.ensure(rig.spawner()), Ok(()));
        assert_eq!(rig.calls(), [Call::Port, Call::Spawn, Call::Attach]);
        assert!(!rig.recorder.port_failed());
        assert_eq!(rig.count(Level::WARN), 0);
    }

    #[test]
    fn a_listener_that_fails_to_start_is_never_attached_and_latches() {
        let rig = Rig::new();
        assert_eq!(rig.ensure(rig.failing_spawner()), Err(()));
        assert_eq!(rig.calls(), [Call::Port, Call::Spawn, Call::PortDrop]);
        assert!(rig.recorder.port_failed());
        assert_eq!(rig.count(Level::WARN), 1);

        assert_eq!(rig.ensure(rig.spawner()), Err(()));
        assert_eq!(rig.calls(), [Call::Port, Call::Spawn, Call::PortDrop]);
    }

    #[test]
    fn a_failed_attach_latches_and_the_listener_ends_without_detaching() {
        let rig = Rig::new();
        rig.group.attach_fails.store(true, Ordering::SeqCst);
        assert_eq!(rig.ensure(rig.spawner()), Err(()));
        rig.ended();

        assert!(
            rig.waits.try_recv().is_err(),
            "took from a port that was not attached"
        );
        assert_eq!(
            rig.calls(),
            [Call::Port, Call::Spawn, Call::Attach, Call::PortDrop]
        );
        assert!(rig.recorder.port_failed());
        assert_eq!(rig.count(Level::WARN), 1);
    }

    #[test]
    fn a_missing_group_fails_the_episode_without_latching() {
        for kind in [GROUP_GONE, io::ErrorKind::Unsupported] {
            let rig = Rig::new();
            *rig.group.refuse_watch.lock().unwrap() = Some(kind);
            assert_eq!(rig.ensure(rig.spawner()), Err(()), "{kind:?}");
            assert!(rig.calls().is_empty(), "{kind:?}: {:?}", rig.calls());
            assert!(!rig.recorder.port_failed(), "{kind:?}");
            assert_eq!(rig.count(Level::WARN), 0, "{kind:?}");
        }
    }

    // ── Real processes (Windows) ───────────────────────────────────────────────────────────

    #[cfg(windows)]
    #[test]
    fn a_birth_in_the_job_is_recorded_with_its_facts_through_the_real_port() {
        use crate::transport::process_group::tests::new_group;
        use engram_dashboard_platform::testing::{open_gate, spawn_gated_cmd, wait_until};
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;

        let (job, group) = new_group();
        let group = Arc::new(group);
        let mut x = spawn_gated_cmd("ping -n 30 127.0.0.1", CREATE_NO_WINDOW);
        job.adopt(x.id()).expect("adopt into Job");
        let recorder = Arc::new(Recorder::new());
        let watch = BirthWatch::new();
        assert_eq!(
            watch.ensure_with(&group, &recorder, &LogTag::new(None, x.id()), |body| {
                SystemClock.spawn(LISTENER_THREAD, body)
            }),
            Ok(())
        );
        let (r, _) = recorder.start();
        open_gate(&mut x);

        let ping = wait_until("recorded ping", || {
            recorder
                .copy(r)
                .into_iter()
                .find(|b| b.facts.image.to_ascii_lowercase().ends_with("\\ping.exe"))
        });
        assert_eq!(ping.facts.ppid, x.id(), "parent: {:?}", ping.facts);
        assert_ne!(ping.facts.create, 0);
        assert!(
            ping.facts.cmdline.contains("ping"),
            "command line: {:?}",
            ping.facts
        );
        assert!(ping.killable);
        assert!(!ping.pin.exited().expect("exited?"));
        drop(ping);
        drop(recorder.stop(r));

        job.terminate(1).expect("end the Job");
        let _ = x.wait();
        // Turn the record on **before** releasing the Job — the listener thread turns off only the record that was on when it saw the
        //   group disappear. If it is turned on after releasing,
        // a thread that woke in between may end without a record on, and nobody is left to turn off a record turned on later.
        let (after, _) = recorder.start();
        drop(job);
        wait_until(
            "a record the listening thread turned off because the group disappeared",
            || (recorder.active() != after).then_some(()),
        );
        assert_eq!(recorder.active(), 0);
        assert!(!recorder.port_failed());
    }
}

#[cfg(test)]
mod test_support {
    //! Fakes shared by the birth record · gate tests — what is held · lock detection · log capture.

    use super::*;

    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU8, AtomicUsize};
    use std::sync::Weak;

    pub(super) const ALIVE: u8 = 0;
    pub(super) const EXITED: u8 = 1;
    pub(super) const FAILING: u8 = 2;

    /// [`FakePin`]'s imitation of kill.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Kill {
        /// The kill call is refused.
        Refuse,
        /// Accepted and exits immediately.
        Exit,
        /// Accepted but does not exit — it overruns the confirmation deadline.
        Linger,
    }

    /// A call the held item received — [`Probe::calls`] records it together with whether either of the two locks was held at that time.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum PinCall {
        Exited,
        Wait,
        Terminate,
        Classify,
    }

    type Hook = Arc<dyn Fn() + Send + Sync>;

    /// Counts whether the record or gate lock was held when dropping a held item · asking the group (tests do not overlap other threads,
    ///   so a held lock was held by
    /// the calling thread). Records held-item calls, inserts the test's job into the exit query, advances the test clock.
    pub(super) struct Probe {
        recorder: Weak<Recorder>,
        gate: OnceLock<Box<dyn Fn() -> bool + Send + Sync>>,
        pub(super) drops: AtomicUsize,
        pub(super) held_drops: AtomicUsize,
        pub(super) held_calls: AtomicUsize,
        calls: Mutex<Vec<(u32, PinCall, bool)>>,
        hooks: Mutex<BTreeMap<u32, Hook>>,
        clock: OnceLock<Box<dyn Fn(Duration) + Send + Sync>>,
        waits: Mutex<Vec<Duration>>,
        terminate_cost: Mutex<Duration>,
    }

    impl Probe {
        pub(super) fn new(recorder: &Arc<Recorder>) -> Arc<Self> {
            Arc::new(Self {
                recorder: Arc::downgrade(recorder),
                gate: OnceLock::new(),
                drops: AtomicUsize::new(0),
                held_drops: AtomicUsize::new(0),
                held_calls: AtomicUsize::new(0),
                calls: Mutex::default(),
                hooks: Mutex::default(),
                clock: OnceLock::new(),
                waits: Mutex::default(),
                terminate_cost: Mutex::default(),
            })
        }

        pub(super) fn drive(&self, advance: impl Fn(Duration) + Send + Sync + 'static) {
            let _ = self.clock.set(Box::new(advance));
        }

        pub(super) fn set_terminate_cost(&self, d: Duration) {
            *self.terminate_cost.lock().unwrap() = d;
        }

        /// A job to run at every exit query of the held item with `pid` — if called inside a lock, it is counted rather than run (so as
        ///   not to hang on the same lock).
        pub(super) fn on_exited(&self, pid: u32, hook: impl Fn() + Send + Sync + 'static) {
            self.hooks.lock().unwrap().insert(pid, Arc::new(hook));
        }

        /// Cuts the cycle formed by the gate the inserted job holds · the cleaner.
        pub(super) fn clear_hooks(&self) {
            if let Ok(mut hooks) = self.hooks.lock() {
                hooks.clear();
            }
        }

        /// Calls the held item received — (number, call, whether a lock was held then), in call order.
        pub(super) fn calls(&self) -> Vec<(u32, PinCall, bool)> {
            self.calls.lock().unwrap().clone()
        }

        pub(super) fn waits(&self) -> Vec<Duration> {
            self.waits.lock().unwrap().clone()
        }

        fn note(&self, pid: u32, call: PinCall) -> bool {
            let held = self.held();
            if held && call != PinCall::Terminate {
                self.held_calls.fetch_add(1, Ordering::SeqCst);
            }
            self.calls.lock().unwrap().push((pid, call, held));
            held
        }

        fn spend(&self, d: Duration) {
            if let Some(advance) = self.clock.get() {
                advance(d);
            }
        }

        fn hook(&self, pid: u32) -> Option<Hook> {
            self.hooks.lock().unwrap().get(&pid).cloned()
        }

        pub(super) fn watch_gate<G: Group>(&self, cell: &Arc<GateCell<G>>) {
            let cell = Arc::downgrade(cell);
            let _ = self.gate.set(Box::new(move || {
                let Some(cell) = cell.upgrade() else {
                    return false;
                };
                let held = matches!(cell.state.try_lock(), Err(TryLockError::WouldBlock));
                held
            }));
        }

        pub(super) fn held(&self) -> bool {
            let record = match self.recorder.upgrade() {
                Some(recorder) => {
                    let held = matches!(recorder.inner.try_lock(), Err(TryLockError::WouldBlock));
                    held
                }
                None => false,
            };
            record || self.gate.get().is_some_and(|gate| gate())
        }

        pub(super) fn drops(&self) -> usize {
            self.drops.load(Ordering::SeqCst)
        }

        pub(super) fn held_drops(&self) -> usize {
            self.held_drops.load(Ordering::SeqCst)
        }

        pub(super) fn held_calls(&self) -> usize {
            self.held_calls.load(Ordering::SeqCst)
        }
    }

    /// Imitation of a held process — exit is decided by the field, and on drop it records whether the two locks were held. Kill is decided
    ///   by `kill`
    /// (default = refused). `pid` is only a label for the call record (default 0).
    pub(super) struct FakePin {
        facts: ProcessFacts,
        probe: Arc<Probe>,
        exit: Arc<AtomicU8>,
        pid: u32,
        kill: Kill,
    }

    impl FakePin {
        pub(super) fn new(facts: ProcessFacts, probe: &Arc<Probe>) -> Self {
            Self::with_exit(facts, probe, Arc::new(AtomicU8::new(ALIVE)))
        }

        pub(super) fn with_exit(
            facts: ProcessFacts,
            probe: &Arc<Probe>,
            exit: Arc<AtomicU8>,
        ) -> Self {
            Self {
                facts,
                probe: Arc::clone(probe),
                exit,
                pid: 0,
                kill: Kill::Refuse,
            }
        }

        pub(super) fn named(mut self, pid: u32) -> Self {
            self.pid = pid;
            self
        }

        pub(super) fn killed_by(mut self, kill: Kill) -> Self {
            self.kill = kill;
            self
        }

        fn state(&self) -> io::Result<bool> {
            match self.exit.load(Ordering::SeqCst) {
                ALIVE => Ok(false),
                EXITED => Ok(true),
                _ => Err(io::Error::other("imitated exit query failure")),
            }
        }
    }

    impl Pinned for FakePin {
        fn exited(&self) -> io::Result<bool> {
            let held = self.probe.note(self.pid, PinCall::Exited);
            if let (false, Some(hook)) = (held, self.probe.hook(self.pid)) {
                hook();
            }
            self.state()
        }

        fn wait_exit(&self, d: Duration) -> io::Result<bool> {
            self.probe.note(self.pid, PinCall::Wait);
            if let Ok(false) = self.state() {
                self.probe.waits.lock().unwrap().push(d);
                self.probe.spend(d);
            }
            self.state()
        }

        fn facts(&self) -> &ProcessFacts {
            &self.facts
        }

        /// Answers with failure unless it is the exit code the cleanup chose — a test that asserts it was killed turns red.
        fn terminate_raw(&self, exit_code: u32) -> io::Result<()> {
            self.probe.note(self.pid, PinCall::Terminate);
            let cost = *self.probe.terminate_cost.lock().unwrap();
            self.probe.spend(cost);
            if exit_code != LEFTOVER_EXIT_CODE {
                return Err(io::Error::other(format!("exit code {exit_code:#x}")));
            }
            match self.kill {
                Kill::Refuse => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
                Kill::Exit => {
                    self.exit.store(EXITED, Ordering::SeqCst);
                    Ok(())
                }
                Kill::Linger => Ok(()),
            }
        }

        /// Like the real thing — even if the kill failed, `Gone` if it exited in the meantime.
        fn classify(&self, raw: io::Result<()>) -> io::Result<MemberKill> {
            self.probe.note(self.pid, PinCall::Classify);
            match raw {
                Ok(()) => Ok(MemberKill::Terminated),
                Err(_) if matches!(self.state(), Ok(true)) => Ok(MemberKill::Gone),
                Err(e) => Err(e),
            }
        }
    }

    impl Drop for FakePin {
        fn drop(&mut self) {
            self.probe.drops.fetch_add(1, Ordering::SeqCst);
            if self.probe.held() {
                self.probe.held_drops.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    // ── Log capture ──────────────────────────────────────────────────────────────────────

    pub(super) use crate::log_capture::{Capture, Line};
}

#[cfg(test)]
mod gate_tests {
    use super::test_support::{
        Capture, FakePin, Kill, Line, PinCall, Probe, ALIVE, EXITED, FAILING,
    };
    use super::tests::{p1, p1_chain, p1_two_owners, p3bg, p3bg_deeper, p3t3, pilot, Staged};
    use super::*;

    use std::collections::{BTreeMap, HashMap};
    use std::sync::atomic::{AtomicU8, AtomicUsize};

    use tracing::Level;

    const ROOT: u32 = 100;
    const CONHOST: u32 = 101;
    const CLAUDE: u32 = 102;
    const N: Duration = INTERRUPT_LEFTOVER_GRACE;
    const LINE: &[u8] = b"{\"type\":\"control_request\"}\n";

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn facts(ppid: u32, create: u64) -> ProcessFacts {
        ProcessFacts {
            ppid,
            create,
            image: r"C:\Program Files\Git\usr\bin\bash.exe".to_owned(),
            cmdline: "bash x.sh".to_owned(),
        }
    }

    // ── Fakes ──────────────────────────────────────────────────────────────────────────

    /// A clock driven by the test — sleep advances time by that amount (and records it), start piles up the body (the test decides whether
    ///   to run it).
    struct FakeClock {
        base: Instant,
        nanos: AtomicU64,
        sleeps: Mutex<Vec<Duration>>,
        bodies: Mutex<Vec<(String, Box<dyn FnOnce() + Send>)>>,
        keep_workers: AtomicBool,
        refuse_worker: AtomicBool,
        panic_in_sleep: AtomicBool,
    }

    impl FakeClock {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                base: Instant::now(),
                nanos: AtomicU64::new(0),
                sleeps: Mutex::default(),
                bodies: Mutex::default(),
                keep_workers: AtomicBool::new(true),
                refuse_worker: AtomicBool::new(false),
                panic_in_sleep: AtomicBool::new(false),
            })
        }

        fn advance(&self, d: Duration) {
            let nanos = u64::try_from(d.as_nanos()).expect("test time");
            self.nanos.fetch_add(nanos, Ordering::SeqCst);
        }

        fn at(&self, d: Duration) -> Instant {
            self.base + d
        }

        fn sleeps(&self) -> Vec<Duration> {
            self.sleeps.lock().unwrap().clone()
        }

        fn take(&self, name: &str) -> Vec<Box<dyn FnOnce() + Send>> {
            let mut bodies = self.bodies.lock().unwrap();
            let (taken, kept): (Vec<_>, Vec<_>) =
                bodies.drain(..).partition(|(n, _)| n.as_str() == name);
            *bodies = kept;
            taken.into_iter().map(|(_, body)| body).collect()
        }

        fn spawned(&self, name: &str) -> usize {
            self.bodies
                .lock()
                .unwrap()
                .iter()
                .filter(|(n, _)| n.as_str() == name)
                .count()
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.base + Duration::from_nanos(self.nanos.load(Ordering::SeqCst))
        }
    }

    impl LeftoverClock for FakeClock {
        fn sleep(&self, d: Duration) {
            self.sleeps.lock().unwrap().push(d);
            if self.panic_in_sleep.load(Ordering::SeqCst) {
                panic!("imitated sleep panic");
            }
            self.advance(d);
        }

        fn spawn(&self, name: &str, body: Box<dyn FnOnce() + Send>) -> io::Result<()> {
            if name == WORKER_THREAD {
                if self.refuse_worker.load(Ordering::SeqCst) {
                    return Err(io::Error::other("imitated start failure"));
                }
                if !self.keep_workers.load(Ordering::SeqCst) {
                    drop(body);
                    return Ok(());
                }
            }
            self.bodies.lock().unwrap().push((name.to_owned(), body));
            Ok(())
        }
    }

    #[derive(Clone)]
    enum Answer {
        Ours(ProcessFacts),
        Denied,
        Panic,
    }

    /// The group of the gate tests — the listing · hold answers are decided by the test, and one test job is inserted when the listing is
    ///   asked (imitates events between the two sections ·
    /// between write and listing).
    struct GateGroup {
        /// `None` = listing failure.
        listing: Mutex<Option<Vec<u32>>>,
        answers: Mutex<HashMap<u32, Answer>>,
        exits: Mutex<HashMap<u32, Arc<AtomicU8>>>,
        refuse_watch: Mutex<Option<io::ErrorKind>>,
        retiring: Arc<AtomicBool>,
        hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
        probe: Arc<Probe>,
        lists: AtomicUsize,
        pins: AtomicUsize,
        watches: AtomicUsize,
    }

    impl GateGroup {
        fn exit_of(&self, pid: u32) -> Arc<AtomicU8> {
            Arc::clone(
                self.exits
                    .lock()
                    .unwrap()
                    .entry(pid)
                    .or_insert_with(|| Arc::new(AtomicU8::new(ALIVE))),
            )
        }
    }

    impl Group for GateGroup {
        fn member_pids(&self) -> io::Result<Vec<u32>> {
            self.lists.fetch_add(1, Ordering::SeqCst);
            if self.probe.held() {
                // Called while holding the lock — it is not run, so the inserted job does not hang by taking the same lock.
                self.probe.held_calls.fetch_add(1, Ordering::SeqCst);
            } else {
                let hook = self.hook.lock().unwrap().take();
                if let Some(hook) = hook {
                    hook();
                }
            }
            let listing = self.listing.lock().unwrap().clone();
            listing.ok_or_else(|| io::Error::other("imitated list failure"))
        }

        fn pin(&self, pid: u32, _kill: bool) -> io::Result<Option<Box<dyn Pinned>>> {
            self.pins.fetch_add(1, Ordering::SeqCst);
            if self.probe.held() {
                self.probe.held_calls.fetch_add(1, Ordering::SeqCst);
            }
            let answer = self.answers.lock().unwrap().get(&pid).cloned();
            match answer {
                None => Ok(None),
                Some(Answer::Ours(facts)) => Ok(Some(Box::new(
                    FakePin::with_exit(facts, &self.probe, self.exit_of(pid)).named(pid),
                ))),
                Some(Answer::Denied) => Err(io::Error::from(io::ErrorKind::PermissionDenied)),
                Some(Answer::Panic) => panic!("imitated hold panic"),
            }
        }

        fn watch_births(
            &self,
            start: impl FnOnce(Arc<dyn Births>) -> io::Result<()>,
        ) -> io::Result<()> {
            self.watches.fetch_add(1, Ordering::SeqCst);
            let refused = *self.refuse_watch.lock().unwrap();
            if let Some(kind) = refused {
                return Err(io::Error::new(kind, "imitated port refusal"));
            }
            start(Arc::new(IdlePort))
        }

        fn unwatch_births(&self) -> io::Result<()> {
            Ok(())
        }

        fn is_gone(&self) -> bool {
            false
        }

        fn retiring(&self) -> RetiringSignal {
            RetiringSignal::of(&self.retiring)
        }
    }

    struct IdlePort;

    impl Births for IdlePort {
        fn next(&self, _: Duration) -> io::Result<PortEvent> {
            Ok(PortEvent::Timeout)
        }
    }

    // ── Pass ────────────────────────────────────────────────────────────────────────────

    /// A group holding the root (`cmd /c`) · console host · claude, and its gate — the gate is closed. The listener thread · worker body
    ///   only pile up on the clock
    /// and do not run unless the test runs them (the test puts births directly into the record).
    struct GateRig {
        root: u32,
        clock: Arc<FakeClock>,
        group: Arc<GateGroup>,
        recorder: Arc<Recorder>,
        probe: Arc<Probe>,
        cleaner: Arc<Cleaner<GateGroup>>,
        cell: Arc<GateCell<GateGroup>>,
        lines: Arc<Mutex<Vec<Line>>>,
    }

    impl GateRig {
        fn new() -> Self {
            Self::build(
                ROOT,
                // An unsorted listing — the write list is sorted.
                vec![CLAUDE, ROOT, CONHOST],
                HashMap::from([
                    (ROOT, Answer::Ours(facts(1, 10))),
                    (CONHOST, Answer::Ours(facts(ROOT, 20))),
                    (CLAUDE, Answer::Ours(facts(ROOT, 30))),
                ]),
                None,
            )
        }

        fn build(
            root: u32,
            listing: Vec<u32>,
            answers: HashMap<u32, Answer>,
            agent: Option<AgentId>,
        ) -> Self {
            let recorder = Arc::new(Recorder::new());
            let probe = Probe::new(&recorder);
            let group = Arc::new(GateGroup {
                listing: Mutex::new(Some(listing)),
                answers: Mutex::new(answers),
                exits: Mutex::default(),
                refuse_watch: Mutex::default(),
                retiring: Arc::default(),
                hook: Mutex::default(),
                probe: Arc::clone(&probe),
                lists: AtomicUsize::new(0),
                pins: AtomicUsize::new(0),
                watches: AtomicUsize::new(0),
            });
            let clock = FakeClock::new();
            let driven = Arc::clone(&clock);
            probe.drive(move |d| driven.advance(d));
            let cleaner = Arc::new(Cleaner::with_parts(
                Arc::clone(&group),
                Arc::clone(&recorder),
                Arc::clone(&clock) as Arc<dyn LeftoverClock>,
                root,
                LogTag::new(agent, root),
            ));
            let cell = GateCell::new(Some(Arc::clone(&cleaner)));
            probe.watch_gate(&cell);
            Self {
                root,
                clock,
                group,
                recorder,
                probe,
                cleaner,
                cell,
                lines: Arc::default(),
            }
        }

        fn logged<R>(&self, f: impl FnOnce() -> R) -> R {
            tracing::subscriber::with_default(Capture(Arc::clone(&self.lines)), f)
        }

        fn open(&self) {
            self.cell.open_turn();
        }

        fn esc(&self) -> Option<InterruptOut> {
            self.logged(|| self.cell.interrupt(LINE.to_vec()))
        }

        /// Treat the line as written — if there is something to call, call it with no lock, like the writer.
        fn write(&self, out: InterruptOut) {
            if let Some(on_written) = out.on_written {
                self.logged(on_written);
            }
        }

        /// An interrupt that receives write confirmation — both the line and the thing to call must be present.
        fn esc_with_check(&self) -> InterruptOut {
            let out = self.esc().expect("interrupt line");
            assert_eq!(out.bytes, LINE);
            assert!(out.on_written.is_some(), "no write confirmation");
            out
        }

        fn turn(&self) -> bool {
            self.logged(|| worker_turn(&self.cell, &self.cleaner))
        }

        /// Run the piled-up worker bodies to the end on this thread.
        fn run_workers(&self) {
            for body in self.clock.take(WORKER_THREAD) {
                self.logged(body);
            }
        }

        /// A job to run once at the next listing query (snapshot · write list).
        fn on_list(&self, f: impl FnOnce() + Send + 'static) {
            *self.group.hook.lock().unwrap() = Some(Box::new(f));
        }

        fn answer(&self, pid: u32, answer: Answer) {
            self.group.answers.lock().unwrap().insert(pid, answer);
        }

        fn listing(&self, listing: Option<Vec<u32>>) {
            *self.group.listing.lock().unwrap() = listing;
        }

        fn state<R>(&self, f: impl FnOnce(&GateState) -> R) -> R {
            let state = self.cell.lock();
            f(&state)
        }

        fn episode<R>(&self, f: impl FnOnce(&Episode) -> R) -> R {
            self.state(|st| f(st.episode.as_ref().expect("episode")))
        }

        fn has_episode(&self) -> bool {
            self.state(|st| st.episode.is_some())
        }

        fn worker(&self) -> bool {
            self.state(|st| st.worker)
        }

        fn next_rec(&self) -> u64 {
            self.recorder.lock().next_rec
        }

        fn count(&self, level: Level) -> usize {
            self.lines
                .lock()
                .unwrap()
                .iter()
                .filter(|(l, _)| *l == level)
                .count()
        }

        fn logged_reason(&self, reason: &str) -> bool {
            self.lines
                .lock()
                .unwrap()
                .iter()
                .any(|(_, fields)| fields.get("reason").is_some_and(|r| r == reason))
        }

        fn logged_text(&self, text: &str) -> bool {
            self.lines
                .lock()
                .unwrap()
                .iter()
                .any(|(_, fields)| fields.get("message").is_some_and(|m| m.contains(text)))
        }

        /// Put one birth into this record — as if the listener thread had held it and put it in.
        fn admit(&self, rec: u64, pid: u32, facts: ProcessFacts, exit: u8) {
            admit_into(&self.recorder, &self.probe, rec, pid, facts, exit);
        }

        /// Load world `w` up to just before the pass — interrupt · write · births (the order of `births` is `seq` · the imitation of kill
        ///   is `kills`, others are
        /// [`Kill::Exit`]) · load the exits at pass time and advance N. The exit field is one per number ([`GateGroup::exit_of`]).
        fn staged(w: &Staged, kills: &[(u32, Kill)]) -> Self {
            let answers = w
                .members
                .iter()
                .map(|(pid, facts, _)| (*pid, Answer::Ours(facts.clone())))
                .collect();
            let rig = Self::build(
                w.root,
                w.listing.clone(),
                answers,
                Some(AgentId::from_u128(0x7440)),
            );
            rig.open();
            let out = rig.esc_with_check();
            rig.write(out);
            let rec = rig.recorder.active();
            for (pid, facts, exited) in &w.births {
                let exit = rig.group.exit_of(*pid);
                exit.store(if *exited { EXITED } else { ALIVE }, Ordering::SeqCst);
                let kill = kills
                    .iter()
                    .find(|(p, _)| p == pid)
                    .map_or(Kill::Exit, |(_, k)| *k);
                let pin = FakePin::with_exit(facts.clone(), &rig.probe, exit)
                    .named(*pid)
                    .killed_by(kill);
                admit_pin(&rig.recorder, rec, *pid, facts.clone(), pin);
            }
            for (pid, _, exited) in &w.members {
                if *exited {
                    rig.group.exit_of(*pid).store(EXITED, Ordering::SeqCst);
                }
            }
            rig.clock.advance(N);
            rig
        }

        /// One pass for the worker to run — the pass must hold.
        fn pass(&self) -> PassReport {
            let step = {
                let mut state = self.cell.lock();
                state.next_step(self.clock.now(), self.cleaner.retiring.is_set())
            };
            let Step::Pass(ticket) = step else {
                panic!("a pass must stand");
            };
            self.logged(|| run_pass(&self.cell, &self.cleaner, ticket))
        }

        /// A job to run once at the `nth` root exit query — 1 = pass step 3 · 2 = recheck before the first kill · 3 = before the second kill.
        /// Imitates events between selection and commit.
        fn on_root_check(&self, nth: usize, f: impl Fn() + Send + Sync + 'static) {
            let seen = AtomicUsize::new(0);
            self.probe.on_exited(self.root, move || {
                if seen.fetch_add(1, Ordering::SeqCst) + 1 == nth {
                    f();
                }
            });
        }

        /// Numbers that received a kill call — in call order.
        fn terminates(&self) -> Vec<u32> {
            self.probe
                .calls()
                .into_iter()
                .filter(|(_, call, _)| *call == PinCall::Terminate)
                .map(|(pid, _, _)| pid)
                .collect()
        }

        /// Every kill call is inside the gate lock and one per section — between two kills there is always a call outside the lock (deciding ·
        /// recheck). Other calls · handle closing · group queries are not inside the lock.
        fn assert_lock_sections(&self) {
            let mut outside_since_last = true;
            for (pid, call, held) in self.probe.calls() {
                if call == PinCall::Terminate {
                    assert!(held, "ended outside the lock: {pid}");
                    assert!(outside_since_last, "ended two in one section: {pid}");
                    outside_since_last = false;
                } else {
                    assert!(!held, "{call:?} inside the lock: {pid}");
                    outside_since_last = true;
                }
            }
            self.check();
        }

        /// A line whose `reason` field is that value — level and field.
        fn line_with_reason(&self, reason: &str) -> Option<Line> {
            self.lines
                .lock()
                .unwrap()
                .iter()
                .find(|(_, fields)| fields.get("reason").is_some_and(|r| r == reason))
                .cloned()
        }

        /// A cleaned line (a warn with the `terminated` field).
        fn cleaned_line(&self) -> Option<BTreeMap<String, String>> {
            self.lines
                .lock()
                .unwrap()
                .iter()
                .find(|(level, fields)| *level == Level::WARN && fields.contains_key("terminated"))
                .map(|(_, fields)| fields.clone())
        }

        /// The invariants of every moment visible outside the lock (TRD §3-2) + nothing was dropped or asked inside the lock.
        fn check(&self) {
            let active = self.recorder.active();
            self.state(|st| {
                if let Some(ep) = &st.episode {
                    if ep.is_live() {
                        assert!(st.open, "a live episode but the gate is closed: {ep:?}");
                    }
                    if ep.written {
                        assert!(ep.w.is_some(), "written but no write list: {ep:?}");
                    }
                    if (ep.written && ep.is_live()) || matches!(ep.phase, Phase::Spent { .. }) {
                        assert!(st.worker, "{ep:?} without a worker");
                    }
                    if matches!(ep.snap, SnapState::Taken(_)) || ep.rec != 0 {
                        assert!(
                            ep.is_live(),
                            "an ended episode held the snapshot or the record: {ep:?}"
                        );
                    }
                    assert!(ep.last_mono >= ep.first_mono);
                }
                assert!(!st.opening, "opening is set with no opener");
                if active != 0 {
                    assert!(
                        st.episode
                            .as_ref()
                            .is_some_and(|ep| ep.is_live() && ep.rec == active),
                        "no live episode holds the active record {active}: {:?}",
                        st.episode
                    );
                }
            });
            assert_eq!(
                self.probe.held_drops(),
                0,
                "closed a handle inside the lock"
            );
            assert_eq!(
                self.probe.held_calls(),
                0,
                "asked the group inside the lock"
            );
        }
    }

    impl Drop for GateRig {
        fn drop(&mut self) {
            // Cuts the cycle formed by the inserted job · piled-up body holding the gate.
            if let Ok(mut hook) = self.group.hook.lock() {
                hook.take();
            }
            self.probe.clear_hooks();
            if let Ok(mut bodies) = self.clock.bodies.lock() {
                bodies.clear();
            }
        }
    }

    /// Put one birth into record `rec` — as if the listener thread had held it and put it in.
    fn admit_into(
        recorder: &Recorder,
        probe: &Arc<Probe>,
        rec: u64,
        pid: u32,
        facts: ProcessFacts,
        exit: u8,
    ) {
        let pin =
            FakePin::with_exit(facts.clone(), probe, Arc::new(AtomicU8::new(exit))).named(pid);
        admit_pin(recorder, rec, pid, facts, pin);
    }

    fn admit_pin(recorder: &Recorder, rec: u64, pid: u32, facts: ProcessFacts, pin: FakePin) {
        let caught = Caught {
            pid,
            facts,
            pin: Box::new(pin),
            killable: true,
        };
        match recorder.admit(rec, caught) {
            Admit::Recorded { .. } => {}
            _ => panic!("the birth did not go into the record"),
        }
    }

    /// A written `Waiting` episode — for pure transition tests.
    fn written_episode(gen: u64, at: Instant, rec: u64) -> Episode {
        Episode {
            gen,
            first_mono: at,
            last_mono: at,
            written: true,
            w: Some(Arc::from(&[ROOT][..])),
            snap: SnapState::Failed(SnapCause::List),
            rec,
            killed_any: false,
            phase: Phase::Waiting,
        }
    }

    // ── Two sections · opener ─────────────────────────────────────────────────────────────

    /// Without a cleaner it only looks at the gate — if closed, no line · if open, a line without write confirmation.
    #[test]
    fn without_a_cleaner_the_gate_only_answers_open_or_closed() {
        let cell = GateCell::<GateGroup>::new(None);
        assert!(cell.interrupt(LINE.to_vec()).is_none());
        cell.open_turn();
        let out = cell.interrupt(LINE.to_vec()).expect("open gate");
        assert_eq!(out.bytes, LINE);
        assert!(out.on_written.is_none());
        cell.deliver();
        assert!(cell.is_open());
        cell.close_turn();
        assert!(!cell.is_open());
        assert!(cell.interrupt(LINE.to_vec()).is_none());
        assert!(cell.lock().episode.is_none());
    }

    /// A closed gate gives no line and turns on or asks nothing.
    #[test]
    fn a_closed_gate_gives_no_line_and_touches_nothing() {
        let rig = GateRig::new();
        assert!(rig.esc().is_none());
        assert!(!rig.has_episode());
        assert_eq!(rig.next_rec(), 0);
        assert_eq!(rig.group.lists.load(Ordering::SeqCst), 0);
        assert_eq!(rig.group.watches.load(Ordering::SeqCst), 0);
        rig.check();
    }

    /// The first interrupt is the opener — port acquisition → record on → snapshot (the record is already on then) → an unwritten `Waiting`
    /// episode. The snapshot is the set of three held members with the root first.
    #[test]
    fn the_first_interrupt_records_before_the_snapshot_and_opens_a_waiting_episode() {
        let rig = GateRig::new();
        rig.open();
        let at_snapshot = Arc::new(AtomicU64::new(0));
        let (seen, recorder) = (Arc::clone(&at_snapshot), Arc::clone(&rig.recorder));
        rig.on_list(move || seen.store(recorder.active(), Ordering::SeqCst));

        rig.esc_with_check();
        let rec = rig.recorder.active();
        assert_ne!(rec, 0);
        assert_eq!(
            at_snapshot.load(Ordering::SeqCst),
            rec,
            "the record was off at snapshot time"
        );
        rig.episode(|ep| {
            assert_eq!(ep.gen, 0);
            assert_eq!(ep.phase, Phase::Waiting);
            assert!(!ep.written);
            assert_eq!(ep.rec, rec);
            assert_eq!(
                (ep.first_mono, ep.last_mono),
                (rig.clock.at(ms(0)), rig.clock.at(ms(0)))
            );
            let SnapState::Taken(snap) = &ep.snap else {
                panic!("snapshot: {:?}", ep.snap);
            };
            let links = snap.links();
            let pids: Vec<u32> = links.iter().map(|l| l.pid).collect();
            assert_eq!(pids.first(), Some(&ROOT));
            assert_eq!(links.len(), 3);
            assert!(links.iter().all(|l| l.seq.is_none() && !l.killable));
        });
        assert_eq!(rig.group.watches.load(Ordering::SeqCst), 1);
        assert_eq!(rig.group.lists.load(Ordering::SeqCst), 1);
        assert_eq!(rig.clock.spawned(LISTENER_THREAD), 1);
        assert_eq!(
            rig.clock.spawned(WORKER_THREAD),
            0,
            "started the worker before it was written"
        );
        assert!(!rig.worker());
        assert!(rig.logged_text("opened an interrupt episode"));
        rig.check();
    }

    /// A later interrupt of a live episode appends — write confirmation is received with the same generation, and the listing · record on
    ///   · port are not made again. A running
    /// pass (`Cleaning`) returns to `Waiting` and the most recent interrupt is pulled forward.
    #[test]
    fn a_later_interrupt_continues_the_live_episode_without_listing_or_recording() {
        let rig = GateRig::new();
        rig.open();
        rig.esc_with_check();
        rig.clock.advance(ms(500));
        rig.esc_with_check();
        rig.episode(|ep| {
            assert_eq!(ep.gen, 0);
            assert_eq!(ep.last_mono, rig.clock.at(ms(500)));
        });
        rig.cell.lock().episode.as_mut().expect("episode").phase = Phase::Cleaning;
        rig.clock.advance(ms(100));
        rig.esc_with_check();
        rig.episode(|ep| {
            assert_eq!(ep.phase, Phase::Waiting);
            assert_eq!(ep.last_mono, rig.clock.at(ms(600)));
        });
        assert_eq!(rig.group.lists.load(Ordering::SeqCst), 1);
        assert_eq!(rig.group.watches.load(Ordering::SeqCst), 1);
        assert_eq!(rig.next_rec(), 1);
        rig.check();
    }

    /// If the turn closes between the two sections, there is no line — turn off the record, drop the snapshot taken outside the lock, and
    ///   lower `opening`.
    #[test]
    fn a_turn_end_between_the_sections_gives_no_line() {
        let rig = GateRig::new();
        rig.open();
        let cell = Arc::clone(&rig.cell);
        rig.on_list(move || cell.close_turn());
        assert!(rig.esc().is_none());
        assert!(!rig.has_episode());
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.probe.drops(), 3);
        // Even if the turn closes and reopens (the generation changed meanwhile), there is no line.
        let cell = Arc::clone(&rig.cell);
        rig.open();
        rig.on_list(move || {
            cell.close_turn();
            cell.open_turn();
        });
        assert!(rig.esc().is_none());
        assert!(!rig.has_episode());
        assert_eq!(rig.recorder.active(), 0);
        rig.check();
    }

    /// If new input arrives between the two sections or the record turned on is turned off (the listener thread ended), only the line is
    ///   given without an episode — turning off the record · dropping the snapshot
    /// are outside the lock.
    #[test]
    fn new_input_or_a_lost_record_between_the_sections_gives_the_line_only() {
        for (lost, reason) in [(0, "delivered"), (1, "record")] {
            let rig = GateRig::new();
            rig.open();
            if lost == 0 {
                let cell = Arc::clone(&rig.cell);
                rig.on_list(move || cell.deliver());
            } else {
                let recorder = Arc::clone(&rig.recorder);
                rig.on_list(move || drop(recorder.stop(recorder.active())));
            }
            let out = rig.esc().expect("line only");
            assert_eq!(out.bytes, LINE);
            assert!(
                out.on_written.is_none(),
                "{reason}: write confirmation on a line without an episode"
            );
            assert!(!rig.has_episode(), "{reason}");
            assert_eq!(rig.recorder.active(), 0, "{reason}");
            assert_eq!(rig.probe.drops(), 3, "{reason}");
            assert!(rig.logged_reason(reason), "{reason}");
            rig.check();
        }
    }

    /// If two Esc interleave, the second sees the opener and receives only a line (record · listing · port 0). The third Esc after the
    ///   opener's ② appends
    /// to the live episode.
    #[test]
    fn a_second_escape_while_opening_gets_the_line_only() {
        let rig = GateRig::new();
        rig.open();
        let nested: Arc<Mutex<Option<Option<bool>>>> = Arc::default();
        let (cell, slot) = (Arc::clone(&rig.cell), Arc::clone(&nested));
        rig.on_list(move || {
            let out = cell.interrupt(LINE.to_vec());
            *slot.lock().unwrap() = Some(out.map(|o| o.on_written.is_some()));
        });
        rig.esc_with_check();
        assert_eq!(*nested.lock().unwrap(), Some(Some(false)));
        assert_eq!(rig.group.lists.load(Ordering::SeqCst), 1);
        assert_eq!(rig.group.watches.load(Ordering::SeqCst), 1);
        assert_eq!(rig.next_rec(), 1);
        assert!(rig.logged_text("is opening"));

        rig.esc_with_check();
        rig.episode(|ep| assert_eq!(ep.gen, 0));
        assert_eq!(rig.group.lists.load(Ordering::SeqCst), 1);
        assert_eq!(rig.next_rec(), 1);
        rig.check();
    }

    /// Opener guard — even if it panics during the snapshot, it lowers `opening` and turns off the record after releasing, and also drops
    ///   a half-taken snapshot outside the lock.
    /// The next interrupt opens again.
    #[test]
    fn the_opener_guard_cleans_up_after_a_panic() {
        let rig = GateRig::new();
        rig.open();
        rig.answer(CONHOST, Answer::Panic);
        let caught = panic::catch_unwind(AssertUnwindSafe(|| rig.esc()));
        assert!(caught.is_err());
        assert!(!rig.has_episode());
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(
            rig.probe.drops(),
            2,
            "a half-captured snapshot (claude · root)"
        );
        rig.check();

        rig.answer(CONHOST, Answer::Ours(facts(ROOT, 20)));
        rig.esc_with_check();
        assert_eq!(rig.next_rec(), 2);
        rig.check();
    }

    /// If there is a live episode in section ② (a branch not reached because there is one opener), only the line is given and that episode
    ///   is left as it is. Even if the generation
    /// changed, it is line only.
    #[test]
    fn the_second_section_never_replaces_a_live_episode_or_a_newer_generation() {
        let t = Instant::now();
        let mut st = GateState {
            open: true,
            ..GateState::default()
        };
        let Begin::Open(mark) = st.begin_interrupt(t) else {
            panic!("must become the opener");
        };
        st.episode = Some(written_episode(7, t, 3));
        match st.settle_interrupt(mark, 5, 5, SnapState::Failed(SnapCause::List), t) {
            Settle::LineOnly {
                why: Stale::Live, ..
            } => {}
            _ => panic!("replaced a live episode"),
        }
        assert!(!st.opening);
        assert_eq!(st.episode.as_ref().map(|ep| ep.gen), Some(7));

        let mut st = GateState {
            open: true,
            ..GateState::default()
        };
        let Begin::Open(mark) = st.begin_interrupt(t) else {
            panic!("must become the opener");
        };
        st.next_gen = 9;
        assert!(matches!(
            st.settle_interrupt(mark, 5, 5, SnapState::Released, t),
            Settle::LineOnly {
                why: Stale::Generation,
                ..
            }
        ));
        assert!(st.episode.is_none());
    }

    // ── Snapshot ────────────────────────────────────────────────────────────────────────

    /// If the snapshot is not set up, the episode opens with that reason (the record is turned on) — listing failure · no root · root not
    ///   held · cap
    /// exceeded (nothing is held) · three root children · no port (the listing is not even asked) · port latched. What was held is dropped
    ///   outside the lock.
    #[test]
    fn a_snapshot_that_cannot_stand_fails_with_its_cause() {
        type Setup = fn(&GateRig);
        let cases: [(&str, Setup, SnapCause, usize); 7] = [
            ("list", |rig| rig.listing(None), SnapCause::List, 0),
            (
                "root_missing",
                |rig| rig.listing(Some(vec![CONHOST, CLAUDE])),
                SnapCause::Root,
                0,
            ),
            (
                "root_unpinned",
                |rig| drop(rig.group.answers.lock().unwrap().remove(&ROOT)),
                SnapCause::Root,
                2,
            ),
            (
                "too_many",
                |rig| rig.listing(Some((ROOT..=ROOT + PIN_MAX as u32).collect())),
                SnapCause::TooMany,
                0,
            ),
            (
                "shape",
                |rig| {
                    rig.listing(Some(vec![ROOT, CONHOST, CLAUDE, 103]));
                    rig.answer(103, Answer::Ours(facts(ROOT, 40)));
                },
                SnapCause::Shape,
                4,
            ),
            (
                "port_gone",
                |rig| *rig.group.refuse_watch.lock().unwrap() = Some(GROUP_GONE),
                SnapCause::Port,
                0,
            ),
            (
                "port_latched",
                |rig| rig.recorder.fail_port(),
                SnapCause::Port,
                0,
            ),
        ];
        for (name, setup, cause, drops) in cases {
            let rig = GateRig::new();
            setup(&rig);
            rig.open();
            rig.esc_with_check();
            rig.episode(|ep| {
                assert!(
                    matches!(ep.snap, SnapState::Failed(c) if c == cause),
                    "{name}: {:?}",
                    ep.snap
                );
                assert_eq!(ep.rec, rig.recorder.active(), "{name}");
            });
            match name {
                "too_many" => assert_eq!(rig.group.pins.load(Ordering::SeqCst), 0, "{name}"),
                "port_gone" | "port_latched" => {
                    assert_eq!(rig.group.lists.load(Ordering::SeqCst), 0, "{name}")
                }
                _ => {}
            }
            assert_eq!(
                rig.probe.drops(),
                drops,
                "{name}: what was held is dropped without setting up the snapshot"
            );
            rig.check();
        }
    }

    /// If a non-root member cannot be held (refused), only that member is left out and the snapshot is set up.
    #[test]
    fn an_unpinnable_member_is_left_out_of_the_snapshot() {
        let rig = GateRig::new();
        rig.answer(CONHOST, Answer::Denied);
        rig.open();
        rig.esc_with_check();
        rig.episode(|ep| assert_eq!(ep.snap.members(), 2));
        rig.check();
    }

    // ── Write confirmation · write list W ───────────────────────────────────────────────────────

    /// First write — take the list once outside the lock and sort it, pull the most recent interrupt to the write time, and start the
    ///   worker. A later
    /// interrupt's write only pulls the time, and its list is handed back and dropped (no more workers are started).
    #[test]
    fn the_first_write_takes_the_write_list_and_asks_for_one_worker() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.clock.advance(ms(120));
        rig.write(out);
        rig.episode(|ep| {
            assert!(ep.written);
            assert_eq!(ep.w.as_deref(), Some(&[ROOT, CONHOST, CLAUDE][..]));
            assert_eq!(ep.last_mono, rig.clock.at(ms(120)));
            assert_eq!(ep.first_mono, rig.clock.at(ms(0)));
        });
        assert!(rig.worker());
        assert_eq!(rig.clock.spawned(WORKER_THREAD), 1);
        assert_eq!(rig.group.lists.load(Ordering::SeqCst), 2);
        rig.check();

        rig.listing(Some(vec![ROOT, CONHOST, CLAUDE, 300]));
        rig.clock.advance(ms(200));
        let again = rig.esc_with_check();
        rig.write(again);
        rig.episode(|ep| {
            assert_eq!(ep.w.as_deref(), Some(&[ROOT, CONHOST, CLAUDE][..]));
            assert_eq!(ep.last_mono, rig.clock.at(ms(320)));
        });
        assert_eq!(rig.clock.spawned(WORKER_THREAD), 1);
        assert_eq!(rig.group.lists.load(Ordering::SeqCst), 3);
        rig.check();
    }

    /// A later write's list does not enter the state and is handed back — the receiver drops it outside the lock.
    #[test]
    fn a_later_write_hands_its_list_back() {
        let t = Instant::now();
        let mut st = GateState {
            open: true,
            worker: true,
            episode: Some(written_episode(4, t, 1)),
            ..GateState::default()
        };
        let later: Arc<[u32]> = Arc::from(&[ROOT, 7][..]);
        let noted = st.note_written(4, Some(Arc::clone(&later)), t + ms(10));
        assert_eq!(noted.note, WriteNote::Again);
        assert!(noted.back.is_some_and(|back| Arc::ptr_eq(&back, &later)));
        assert!(!noted.spawn);
        let ep = st.episode.as_ref().expect("episode");
        assert_eq!(ep.w.as_deref(), Some(&[ROOT][..]));
        assert_eq!(ep.last_mono, t + ms(10));

        let stale = st.note_written(5, Some(later), t + ms(20));
        assert_eq!(stale.note, WriteNote::Stale);
        assert!(stale.back.is_some());
        assert_eq!(st.episode.as_ref().map(|ep| ep.last_mono), Some(t + ms(10)));
    }

    /// If the write list cannot be taken (list failure) or the group is gone (empty list), it does not count as written — no worker and no
    ///   pass. warn is
    /// once per incarnation. If a later write of the same episode gets the list, it counts as written from then (candidates are only the
    ///   births after that list).
    #[test]
    fn an_unlisted_write_never_counts_as_written() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.listing(None);
        rig.write(out);
        rig.episode(|ep| assert!(!ep.written && ep.w.is_none()));
        assert!(!rig.worker());
        assert_eq!(rig.clock.spawned(WORKER_THREAD), 0);
        assert_eq!(rig.count(Level::WARN), 1);
        rig.check();

        rig.listing(Some(Vec::new()));
        let again = rig.esc_with_check();
        rig.write(again);
        rig.episode(|ep| assert!(!ep.written));
        assert_eq!(rig.count(Level::WARN), 1, "warn once");
        rig.check();

        rig.listing(Some(vec![ROOT, CONHOST, CLAUDE]));
        let listed = rig.esc_with_check();
        rig.write(listed);
        rig.episode(|ep| assert!(ep.written));
        assert!(rig.worker());
        rig.check();
    }

    /// A write of an ended episode · a swapped-out generation is dropped — it does not turn a new episode into a written one.
    #[test]
    fn a_write_for_another_episode_is_dropped() {
        let rig = GateRig::new();
        rig.open();
        let first = rig.esc_with_check();
        rig.cell.deliver();
        let second = rig.esc_with_check();
        rig.write(first);
        rig.episode(|ep| {
            assert_eq!(ep.gen, 1);
            assert!(!ep.written);
        });
        assert!(!rig.worker());
        rig.write(second);
        rig.episode(|ep| assert!(ep.written));

        let third = rig.esc_with_check();
        rig.cell.close_turn();
        rig.write(third);
        assert!(!rig.has_episode());
        rig.check();
    }

    /// Candidates are only births that joined after the write list W — independent of when notifications piled up.
    ///
    /// - X: joined before the write, alive → in W (even if the port delivers the join late) → not a candidate.
    /// - Z: joined between the write and W → in W → left out (miss window).
    /// - P′: a birth after the write reuses a number in W → compared by number only → left out (miss).
    /// - Q: joined before the write, exited before W → outside W but exited → link only.
    /// - Y: joined after W → candidate.
    #[test]
    fn only_births_after_the_write_list_are_candidates() {
        const X: u32 = 200;
        const Z: u32 = 201;
        const P: u32 = 202;
        const Q: u32 = 203;
        const Y: u32 = 204;
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        let rec = rig.recorder.active();
        rig.admit(rec, Q, facts(CLAUDE, 40), EXITED);
        rig.listing(Some(vec![ROOT, CONHOST, CLAUDE, X, P]));
        let (group, recorder, probe) = (
            Arc::clone(&rig.group),
            Arc::clone(&rig.recorder),
            Arc::clone(&rig.probe),
        );
        // Z is born after the line was written · before the list is taken, and is recorded right away.
        rig.on_list(move || {
            if let Some(listing) = group.listing.lock().unwrap().as_mut() {
                listing.push(Z);
            }
            admit_into(&recorder, &probe, rec, Z, facts(CLAUDE, 60), ALIVE);
        });
        rig.write(out);
        for (pid, create) in [(X, 50), (P, 990), (Y, 70)] {
            rig.admit(rec, pid, facts(CLAUDE, create), ALIVE);
        }
        rig.clock.advance(N);

        let step = rig.cell.lock().next_step(rig.clock.now(), false);
        let Step::Pass(ticket) = step else {
            panic!("a pass must stand");
        };
        assert_eq!(&*ticket.w, &[ROOT, CONHOST, CLAUDE, X, Z, P][..]);
        let snapshot =
            precheck(&ticket, rig.recorder.active(), false, false).expect("steps 1–3 of the pass");
        let births = rig.recorder.copy(ticket.rec);
        let snapshot_links = snapshot.links();
        let birth_links: Vec<Link<'_>> = births.iter().map(|b| b.link()).collect();
        let exited = |l: &Link<'_>| -> io::Result<bool> {
            births
                .iter()
                .find(|b| b.pid == l.pid && b.facts.create == l.facts.create)
                .map_or(Ok(false), |b| b.pin.exited())
        };
        let view = Chain {
            root_pid: ROOT,
            snapshot: &snapshot_links,
            births: &birth_links,
            exited: &exited,
        };
        let picked: Vec<u32> = candidates(&view, &ticket.w).iter().map(|l| l.pid).collect();
        assert_eq!(picked, [Y]);

        drop(view);
        drop(birth_links);
        drop(snapshot_links);
        drop(births);
        rig.cell.end_pass(&rig.cleaner, ticket.gen, false);
        drop(ticket);
        rig.check();
    }

    // ── Worker · pass end ──────────────────────────────────────────────────────────────────

    /// The worker sleeps until the most recent write + N and runs one pass — the pass end closes it out as `Done`, and after the snapshot
    ///   · record are released outside the lock
    /// the worker ends.
    #[test]
    fn the_worker_sleeps_until_n_after_the_last_write_then_ends_the_pass_once() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.clock.advance(ms(100));
        rig.write(out);
        rig.clock.advance(ms(400));
        rig.run_workers();

        assert_eq!(rig.clock.sleeps(), [ms(2_600)]);
        rig.episode(|ep| {
            assert_eq!(ep.phase, Phase::Done);
            assert!(matches!(ep.snap, SnapState::Released));
            assert_eq!(ep.rec, 0);
        });
        assert!(!rig.worker());
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.probe.drops(), 3);
        rig.check();

        // An interrupt after an ended episode is a new episode.
        rig.esc_with_check();
        rig.episode(|ep| assert_eq!((ep.gen, ep.phase), (1, Phase::Waiting)));
        rig.check();
    }

    /// A later interrupt's write postpones the pass — N counts from the most recently written interrupt.
    #[test]
    fn a_later_write_moves_the_pass_back() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.write(out);
        rig.clock.advance(ms(2_000));
        let later = rig.esc_with_check();
        rig.write(later);
        assert!(rig.turn());
        assert_eq!(rig.clock.sleeps(), [N]);
        assert!(rig.turn());
        rig.episode(|ep| assert_eq!(ep.phase, Phase::Done));
        assert!(!rig.turn());
        rig.check();
    }

    /// An interrupt mid-pass returns the episode to `Waiting`, and the end of that pass leaves the episode held (it is not a completed pass).
    #[test]
    fn a_pass_superseded_by_a_new_interrupt_keeps_the_episode() {
        let t = Instant::now();
        let mut st = GateState {
            open: true,
            worker: true,
            episode: Some(written_episode(2, t, 6)),
            ..GateState::default()
        };
        let Step::Pass(ticket) = st.next_step(t + N, false) else {
            panic!("pass");
        };
        assert!(matches!(
            st.begin_interrupt(t + N + ms(5)),
            Begin::Continue { gen: 2 }
        ));
        let released = st.finish_pass(ticket.gen, true, t + N + ms(9));
        assert_eq!(released.rec, 0);
        let ep = st.episode.as_ref().expect("episode");
        assert_eq!((ep.phase, ep.rec), (Phase::Waiting, 6));
        assert!(ep.killed_any);
        // Waits for N again.
        assert!(matches!(st.next_step(t + N + ms(10), false), Step::Sleep(d) if d == N - ms(5)));
    }

    /// A pass that killed anything is `Spent` — if it is still the same episode after N, warn once (`StillHung`) and `Done`, and the
    ///   worker ends.
    /// A pass that killed nothing goes straight to `Done`.
    #[test]
    fn a_pass_that_killed_waits_n_then_reports_once_and_ends() {
        let t = Instant::now();
        let mut st = GateState {
            open: true,
            worker: true,
            episode: Some(written_episode(1, t, 4)),
            ..GateState::default()
        };
        let Step::Pass(ticket) = st.next_step(t + N, false) else {
            panic!("pass");
        };
        let at = t + N + ms(300);
        let released = st.finish_pass(ticket.gen, true, at);
        assert_eq!(released.rec, 4);
        assert!(st.episode.as_ref().is_some_and(|ep| ep.killed_any));
        assert_eq!(
            st.episode.as_ref().map(|ep| ep.phase),
            Some(Phase::Spent { at })
        );
        assert!(
            matches!(st.next_step(at + ms(1_000), false), Step::Sleep(d) if d == N - ms(1_000))
        );
        assert!(matches!(
            st.next_step(at + N, false),
            Step::StillHung { waited } if waited == at + N - t
        ));
        assert_eq!(st.episode.as_ref().map(|ep| ep.phase), Some(Phase::Done));
        assert!(matches!(
            st.next_step(at + N, false),
            Step::Exit { retired: None, .. }
        ));
        assert!(!st.worker);

        let mut st = GateState {
            open: true,
            worker: true,
            episode: Some(written_episode(1, t, 4)),
            ..GateState::default()
        };
        let Step::Pass(ticket) = st.next_step(t + N, false) else {
            panic!("pass");
        };
        let _ = st.finish_pass(ticket.gen, false, t + N);
        assert!(st.episode.as_ref().is_some_and(|ep| !ep.killed_any));
        assert_eq!(st.episode.as_ref().map(|ep| ep.phase), Some(Phase::Done));
    }

    /// The recheck of the kill commit holds only for the same generation · written `Cleaning` · when N has passed. If the marker is set,
    ///   `Retiring`. The killed mark is
    /// only for the same generation.
    #[test]
    fn the_commit_check_goes_only_for_the_same_due_cleaning_episode() {
        let t = Instant::now();
        let mut st = GateState {
            open: true,
            worker: true,
            episode: Some(written_episode(3, t, 1)),
            ..GateState::default()
        };
        assert_eq!(
            st.commit_check(3, t + N, false),
            Commit::Superseded,
            "Waiting"
        );
        assert!(matches!(st.next_step(t + N, false), Step::Pass(_)));
        assert_eq!(st.commit_check(3, t + N, false), Commit::Go);
        assert_eq!(st.commit_check(3, t + N - ms(1), false), Commit::Superseded);
        assert_eq!(st.commit_check(4, t + N, false), Commit::Superseded);
        assert_eq!(st.commit_check(3, t + N, true), Commit::Retiring);
        st.note_kill(4);
        assert!(!st.episode.as_ref().expect("episode").killed_any);
        st.note_kill(3);
        assert!(st.episode.as_ref().expect("episode").killed_any);
        let _ = st.close_turn();
        assert_eq!(st.commit_check(3, t + N, false), Commit::Superseded);
    }

    /// When the retirement flag is set, the worker clears the episode (releasing the snapshot · record outside the lock) and ends.
    #[test]
    fn retiring_clears_the_episode_and_ends_the_worker() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.write(out);
        rig.group.retiring.store(true, Ordering::Release);
        assert!(!rig.turn());
        assert!(!rig.has_episode());
        assert!(!rig.worker());
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.probe.drops(), 3);
        assert!(rig.logged_text("retiring"));
        rig.check();
    }

    /// If the worker cannot be started, bring the episode down to `Done` and release the record · snapshot (warn). The next interrupt
    ///   opens a new episode.
    #[test]
    fn a_worker_that_fails_to_spawn_drops_the_episode() {
        let rig = GateRig::new();
        rig.clock.refuse_worker.store(true, Ordering::SeqCst);
        rig.open();
        let out = rig.esc_with_check();
        rig.write(out);
        rig.episode(|ep| {
            assert_eq!(ep.phase, Phase::Done);
            assert!(matches!(ep.snap, SnapState::Released));
            assert_eq!(ep.rec, 0);
        });
        assert!(!rig.worker());
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.probe.drops(), 3);
        assert_eq!(rig.count(Level::WARN), 1);
        rig.check();

        rig.esc_with_check();
        rig.episode(|ep| assert_eq!((ep.gen, ep.phase), (1, Phase::Waiting)));
        rig.check();
    }

    /// If the worker body panics, the guard brings the episode down to `Done` and releases what it holds (error).
    #[test]
    fn a_panicking_worker_drops_the_episode_through_its_guard() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.write(out);
        rig.clock.panic_in_sleep.store(true, Ordering::SeqCst);
        let bodies = rig.clock.take(WORKER_THREAD);
        assert_eq!(bodies.len(), 1);
        for body in bodies {
            let ran = panic::catch_unwind(AssertUnwindSafe(|| rig.logged(body)));
            assert!(ran.is_err());
        }
        rig.episode(|ep| {
            assert_eq!(ep.phase, Phase::Done);
            assert_eq!(ep.rec, 0);
        });
        assert!(!rig.worker());
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.probe.drops(), 3);
        assert_eq!(rig.count(Level::ERROR), 1);
        rig.check();
    }

    /// For an unwritten episode the worker simply ends (the episode stays as it is) and comes up again when that episode is written. A
    ///   turn end · new
    /// input drops the episode and releases the record · snapshot outside the lock — a sleeping worker wakes and ends quietly.
    #[test]
    fn decoder_hooks_drop_the_episode_and_the_worker_follows() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.write(out);
        rig.logged(|| rig.cell.deliver());
        assert!(!rig.has_episode());
        assert_eq!(rig.state(|st| (st.open, st.delivers)), (true, 1));
        assert!(rig.logged_text("delivered"));
        assert_eq!(rig.recorder.active(), 0);
        assert_eq!(rig.probe.drops(), 3);
        rig.check();

        rig.esc_with_check();
        assert!(!rig.turn(), "a worker remained on an unwritten episode");
        rig.episode(|ep| assert!(ep.is_live() && !ep.written));
        assert_ne!(rig.recorder.active(), 0);
        rig.check();
        let next = rig.esc_with_check();
        rig.write(next);
        assert!(rig.worker());
        assert_eq!(rig.clock.spawned(WORKER_THREAD), 2);

        rig.cell.close_turn();
        assert_eq!(rig.state(|st| (st.open, st.closes)), (false, 1));
        assert!(!rig.has_episode());
        assert_eq!(rig.recorder.active(), 0);
        assert!(!rig.turn());
        rig.check();

        rig.open();
        rig.open();
        assert_eq!(
            rig.state(|st| (st.open, st.closes, st.delivers)),
            (true, 1, 1)
        );
    }

    // ── Pass steps 1–3 ─────────────────────────────────────────────────────────────────

    /// The pass looks first at the marker · snapshot · record · root — if the listener thread ended and the record is off (`active ≠ rec`)
    /// it is `snapshot_failed(port)`.
    #[test]
    fn the_pass_checks_retiring_snapshot_record_and_root_first() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.write(out);
        rig.clock.advance(N);
        let step = rig.cell.lock().next_step(rig.clock.now(), false);
        let Step::Pass(ticket) = step else {
            panic!("pass");
        };
        let active = rig.recorder.active();
        assert!(precheck(&ticket, active, false, false).is_ok());
        assert!(matches!(
            precheck(&ticket, active, true, false),
            Err(NotReady::Retiring)
        ));

        rig.group.exit_of(ROOT).store(EXITED, Ordering::SeqCst);
        assert!(matches!(
            precheck(&ticket, active, false, false),
            Err(NotReady::RootGone)
        ));
        rig.group.exit_of(ROOT).store(FAILING, Ordering::SeqCst);
        assert!(matches!(
            precheck(&ticket, active, false, false),
            Err(NotReady::RootUnknown(_))
        ));

        drop(rig.recorder.stop(active));
        assert!(matches!(
            precheck(&ticket, rig.recorder.active(), false, false),
            Err(NotReady::Snapshot(SnapCause::Port))
        ));
        rig.cell.end_pass(&rig.cleaner, ticket.gen, false);
        drop(ticket);
        rig.check();

        let failed = GateRig::new();
        failed.listing(None);
        failed.open();
        let out = failed.esc_with_check();
        failed.listing(Some(vec![ROOT]));
        failed.write(out);
        failed.clock.advance(N);
        let step = failed.cell.lock().next_step(failed.clock.now(), false);
        let Step::Pass(ticket) = step else {
            panic!("pass");
        };
        assert!(matches!(
            precheck(&ticket, failed.recorder.active(), false, false),
            Err(NotReady::Snapshot(SnapCause::List))
        ));
    }

    /// If the port is latched as failed, the pass does not hold even if the snapshot · record are fine — it is not `no_new_member`, it is
    /// `snapshot_failed(port)`. The marker comes first.
    #[test]
    fn a_failed_port_stops_the_pass_as_a_port_snapshot_failure() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.write(out);
        rig.clock.advance(N);
        let step = rig.cell.lock().next_step(rig.clock.now(), false);
        let Step::Pass(ticket) = step else {
            panic!("pass");
        };
        let active = rig.recorder.active();
        assert!(precheck(&ticket, active, false, rig.recorder.port_failed()).is_ok());
        rig.recorder.fail_port();
        assert!(matches!(
            precheck(&ticket, active, false, rig.recorder.port_failed()),
            Err(NotReady::Snapshot(SnapCause::Port))
        ));
        assert!(matches!(
            precheck(&ticket, active, true, rig.recorder.port_failed()),
            Err(NotReady::Retiring)
        ));
        rig.cell.end_pass(&rig.cleaner, ticket.gen, false);
        drop(ticket);
        rig.check();
    }

    // ── Rounds of a pass ─────────────────────────────────────────────────────────────────────

    fn pids_rounds(report: &PassReport) -> Vec<(u32, u32)> {
        report.terminated.iter().map(|k| (k.pid, k.round)).collect()
    }

    /// The three measured shapes each kill one owner in one round and confirm it, then stop at the second round with nothing new to kill —
    ///   the pass is closed out as `Spent`
    /// and only one cleaned warn remains.
    #[test]
    fn measured_shapes_end_their_owner_in_one_round() {
        for (name, world, owner, chain) in [
            ("p1", p1(), 38108, vec![38108, 36088, 22700, 26208]),
            (
                "p3t3",
                p3t3(),
                23604,
                vec![23604, 26732, 27108, 10400, 33144],
            ),
            ("pilot", pilot(), 3256, vec![3256, 37552, 37948, 25096]),
        ] {
            let rig = GateRig::staged(&world.staged(), &[]);
            let report = rig.pass();
            assert_eq!(pids_rounds(&report), [(owner, 1)], "{name}");
            assert_eq!(report.terminated[0].chain, chain, "{name}");
            assert!(report.stop.is_none(), "{name}: {:?}", report.stop);
            assert_eq!((report.rounds, report.confirmed), (1, 1), "{name}");
            assert_eq!(rig.terminates(), [owner], "{name}");
            assert!(
                rig.probe.waits().is_empty(),
                "{name}: what has ended is not waited for"
            );
            assert!(
                matches!(rig.episode(|ep| ep.phase), Phase::Spent { .. }),
                "{name}"
            );
            assert!(rig.cleaned_line().is_some(), "{name}");
            assert_eq!(rig.count(Level::WARN), 1, "{name}");
            rig.assert_lock_sections();
        }
    }

    /// p3bg — if round 1 kills owner 32228 and confirms it, in round 2 it shows up as an exited link and fork 24776 becomes a candidate.
    ///   In round 3
    /// there is nothing new to kill. Other births alive at pass time (taskkill · conhost · sleep · cat) are not touched.
    #[test]
    fn p3bg_ends_the_owner_then_its_fork_in_the_next_round() {
        let rig = GateRig::staged(&p3bg().staged(), &[]);
        let report = rig.pass();
        assert_eq!(pids_rounds(&report), [(32228, 1), (24776, 2)]);
        assert_eq!(report.terminated[1].chain, [24776, 32228, 25028, 35688]);
        assert!(report.stop.is_none(), "{:?}", report.stop);
        assert_eq!((report.rounds, report.confirmed), (2, 2));
        assert_eq!(rig.terminates(), [32228, 24776]);
        assert_eq!(
            (report.excluded.parent_alive, report.excluded.not_hook_copy),
            (4, 2)
        );
        rig.assert_lock_sections();
    }

    /// One round's `Cleanup` items are killed in ascending `seq` order — the order in which they entered the record, not creation order.
    #[test]
    fn a_round_ends_its_cleanups_in_seq_order() {
        let rig = GateRig::staged(&p1_two_owners().staged(), &[]);
        assert_eq!(pids_rounds(&rig.pass()), [(38108, 1), (38200, 1)]);
        rig.assert_lock_sections();

        let mut w = p1_two_owners().staged();
        let at = |pid: u32| {
            w.births
                .iter()
                .position(|(p, _, _)| *p == pid)
                .expect("birth")
        };
        let (a, b) = (at(38108), at(38200));
        w.births.swap(a, b);
        let rig = GateRig::staged(&w, &[]);
        assert_eq!(pids_rounds(&rig.pass()), [(38200, 1), (38108, 1)]);
        assert_eq!(rig.terminates(), [38200, 38108]);
        rig.assert_lock_sections();
    }

    /// The confirmation of what one round killed shares one deadline (200 ms) — if an earlier one uses up the deadline, later ones do not wait.
    /// Those that did not exit remain unconfirmed and are not killed again in the next round.
    #[test]
    fn a_round_shares_one_confirm_deadline() {
        let rig = GateRig::staged(
            &p1_two_owners().staged(),
            &[(38108, Kill::Linger), (38200, Kill::Linger)],
        );
        let report = rig.pass();
        let waits = rig.probe.waits();
        assert_eq!(waits, [KILL_CONFIRM, Duration::ZERO]);
        assert!(waits.iter().sum::<Duration>() <= KILL_CONFIRM);
        assert_eq!(pids_rounds(&report), [(38108, 1), (38200, 1)]);
        assert_eq!((report.rounds, report.confirmed), (1, 0));
        assert_eq!(rig.terminates(), [38108, 38200]);
        assert_eq!(rig.cleaned_line().expect("cleaned")["unconfirmed"], "2");
        rig.assert_lock_sections();
    }

    /// An owner that did not exit within the deadline shows up as alive in the next round, so its fork is left out (a miss).
    #[test]
    fn an_owner_not_gone_by_the_deadline_keeps_its_fork_out() {
        let rig = GateRig::staged(&p3bg().staged(), &[(32228, Kill::Linger)]);
        let report = rig.pass();
        assert_eq!(pids_rounds(&report), [(32228, 1)]);
        assert_eq!(rig.terminates(), [32228]);
        assert_eq!(rig.probe.waits(), [KILL_CONFIRM]);
        assert_eq!(
            report.excluded.parent_alive, 5,
            "the parent of 24776 is alive"
        );
        assert!(report.stop.is_none(), "{:?}", report.stop);
        assert_eq!(rig.cleaned_line().expect("cleaned")["unconfirmed"], "1");
        rig.assert_lock_sections();
    }

    /// A shape that needs four rounds is killed only up to three rounds and warns `rounds_exhausted` — the fourth candidate is left.
    #[test]
    fn a_fourth_round_is_never_run() {
        let rig = GateRig::staged(&p3bg_deeper().staged(), &[]);
        let report = rig.pass();
        assert_eq!(pids_rounds(&report), [(32228, 1), (24776, 2), (24800, 3)]);
        assert!(matches!(report.stop, Some(PassStop::RoundsExhausted)));
        assert_eq!(report.rounds, PASS_ROUNDS);
        assert!(!rig.terminates().contains(&24810));
        let (level, fields) = rig
            .line_with_reason("rounds_exhausted")
            .expect("rounds_exhausted");
        assert_eq!((level, fields["rounds"].as_str()), (Level::WARN, "3"));
        assert_eq!(rig.count(Level::WARN), 2, "cleaned + rounds_exhausted");
        rig.assert_lock_sections();
    }

    #[derive(Debug, Clone, Copy)]
    enum Cut {
        TurnEnd,
        Escape,
        Input,
        Retire,
    }

    /// If a turn end · new interrupt · new input · retirement comes between selection and commit (the recheck before the second
    ///   candidate), the commit is refused, and the remaining candidates and
    /// the next round are dropped (debug). What was killed earlier remains in the cleaned log.
    #[test]
    fn a_refused_commit_drops_the_rest_of_the_pass() {
        for cut in [Cut::TurnEnd, Cut::Escape, Cut::Input, Cut::Retire] {
            let rig = GateRig::staged(&p1_two_owners().staged(), &[]);
            let (cell, retiring) = (Arc::clone(&rig.cell), Arc::clone(&rig.group.retiring));
            rig.on_root_check(3, move || match cut {
                Cut::TurnEnd => cell.close_turn(),
                Cut::Escape => drop(cell.interrupt(LINE.to_vec())),
                Cut::Input => cell.deliver(),
                Cut::Retire => retiring.store(true, Ordering::Release),
            });
            let report = rig.pass();
            assert_eq!(pids_rounds(&report), [(38108, 1)], "{cut:?}");
            assert_eq!(rig.terminates(), [38108], "{cut:?}");
            let (stopped, reason) = match cut {
                Cut::Retire => (
                    matches!(report.stop, Some(PassStop::NotReady(NotReady::Retiring))),
                    "retiring",
                ),
                _ => (
                    matches!(report.stop, Some(PassStop::Superseded)),
                    "superseded",
                ),
            };
            assert!(stopped, "{cut:?}: {:?}", report.stop);
            let (level, _) = rig.line_with_reason(reason).expect(reason);
            assert_eq!(level, Level::DEBUG, "{cut:?}");
            assert!(rig.cleaned_line().is_some(), "{cut:?}");
            assert_eq!(rig.count(Level::WARN), 1, "{cut:?}");
            rig.assert_lock_sections();
        }

        // The next round is dropped too — the turn ends before p3bg's round-2 fork.
        let rig = GateRig::staged(&p3bg().staged(), &[]);
        let cell = Arc::clone(&rig.cell);
        rig.on_root_check(3, move || cell.close_turn());
        let report = rig.pass();
        assert_eq!(pids_rounds(&report), [(32228, 1)]);
        assert!(matches!(report.stop, Some(PassStop::Superseded)));
        assert!(!rig.has_episode());
        rig.assert_lock_sections();
    }

    /// Recheck before kill — if the root has exited (debug `root_gone`) · is unknown (warn `root_unknown`) · retires (debug
    /// `retiring`), nothing is killed and the pass stops.
    #[test]
    fn the_recheck_before_a_kill_stops_the_pass() {
        for (state, reason, level) in [
            (EXITED, "root_gone", Level::DEBUG),
            (FAILING, "root_unknown", Level::WARN),
        ] {
            let rig = GateRig::staged(&p1().staged(), &[]);
            let root = rig.group.exit_of(rig.root);
            rig.on_root_check(2, move || root.store(state, Ordering::SeqCst));
            let report = rig.pass();
            assert!(report.terminated.is_empty(), "{reason}");
            assert!(rig.terminates().is_empty(), "{reason}");
            assert_eq!(
                rig.line_with_reason(reason).map(|(l, _)| l),
                Some(level),
                "{reason}"
            );
            assert_eq!(
                rig.count(Level::WARN),
                usize::from(level == Level::WARN),
                "{reason}"
            );
            rig.episode(|ep| assert_eq!(ep.phase, Phase::Done));
            rig.assert_lock_sections();
        }

        // A marker raised after pass step 3 (the first query) is seen before the kill.
        let rig = GateRig::staged(&p1().staged(), &[]);
        let retiring = Arc::clone(&rig.group.retiring);
        rig.on_root_check(1, move || retiring.store(true, Ordering::Release));
        let report = rig.pass();
        assert!(matches!(
            report.stop,
            Some(PassStop::NotReady(NotReady::Retiring))
        ));
        assert!(rig.terminates().is_empty());
        assert_eq!(
            rig.line_with_reason("retiring").map(|(l, _)| l),
            Some(Level::DEBUG)
        );
        assert_eq!(rig.count(Level::WARN), 0);
        rig.assert_lock_sections();
    }

    /// Each kill error is warned one by one, and a pass that could not kill any of what it selected warns `all_failed` — one that exited
    ///   by itself meanwhile
    /// (`Gone`) also does not count as killed. A failed one is not killed again within one pass.
    #[test]
    fn a_pass_that_ends_nothing_it_picked_is_all_failed() {
        let rig = GateRig::staged(&p1().staged(), &[(38108, Kill::Refuse)]);
        let report = rig.pass();
        assert!(report.terminated.is_empty());
        assert_eq!((report.cleanups, report.failed, report.gone), (1, 1, 0));
        assert_eq!(rig.terminates(), [38108]);
        assert!(rig.logged_text("could not end one"));
        let (level, fields) = rig.line_with_reason("all_failed").expect("all_failed");
        assert_eq!((level, fields["failed"].as_str()), (Level::WARN, "1"));
        assert_eq!(rig.count(Level::WARN), 2);
        rig.episode(|ep| assert_eq!(ep.phase, Phase::Done));
        rig.assert_lock_sections();

        let rig = GateRig::staged(&p1().staged(), &[(38108, Kill::Refuse)]);
        let owner = rig.group.exit_of(38108);
        rig.on_root_check(2, move || owner.store(EXITED, Ordering::SeqCst));
        let report = rig.pass();
        assert_eq!((report.cleanups, report.failed, report.gone), (1, 0, 1));
        let (_, fields) = rig.line_with_reason("all_failed").expect("all_failed");
        assert_eq!(fields["gone"], "1");
        assert_eq!(rig.count(Level::WARN), 1);
        rig.assert_lock_sections();
    }

    /// Could not kill — if the first round has no candidates, warn `no_new_member` (`births` · `full`); if all candidates drop out by the
    ///   rules, warn
    /// `parent_rule` (per-rule counts); if the snapshot was not set up, warn `snapshot_failed` (`cause`).
    #[test]
    fn a_pass_without_a_cleanup_says_why() {
        let rig = GateRig::new();
        rig.open();
        let out = rig.esc_with_check();
        rig.write(out);
        rig.admit(rig.recorder.active(), 300, facts(CLAUDE, 40), EXITED);
        rig.clock.advance(N);
        let report = rig.pass();
        assert_eq!((report.candidates, report.births), (0, 1));
        let (level, fields) = rig
            .line_with_reason("no_new_member")
            .expect("no_new_member");
        assert_eq!(level, Level::WARN);
        assert_eq!(
            (fields["births"].as_str(), fields["full"].as_str()),
            ("1", "false")
        );
        assert_eq!(rig.count(Level::WARN), 1);
        rig.assert_lock_sections();

        let rig = GateRig::staged(&p1_chain().staged(), &[]);
        let report = rig.pass();
        assert_eq!((report.candidates, report.cleanups), (1, 0));
        let (level, fields) = rig.line_with_reason("parent_rule").expect("parent_rule");
        assert_eq!(level, Level::WARN);
        assert_eq!(
            (
                fields["no_taskkill"].as_str(),
                fields["parent_alive"].as_str()
            ),
            ("1", "0")
        );
        assert!(rig.terminates().is_empty());
        assert_eq!(rig.count(Level::WARN), 1);
        rig.assert_lock_sections();

        let rig = GateRig::new();
        rig.listing(None);
        rig.open();
        let out = rig.esc_with_check();
        rig.listing(Some(vec![ROOT]));
        rig.write(out);
        rig.clock.advance(N);
        rig.pass();
        let (level, fields) = rig
            .line_with_reason("snapshot_failed")
            .expect("snapshot_failed");
        assert_eq!((level, fields["cause"].as_str()), (Level::WARN, "list"));
        rig.assert_lock_sections();
    }

    /// The cleaned warn carries every field of TRD §3-8.
    #[test]
    fn the_cleaned_line_carries_every_field() {
        let rig = GateRig::staged(&p3bg().staged(), &[]);
        rig.probe.set_terminate_cost(Duration::from_micros(150));
        rig.pass();
        let line = rig.cleaned_line().expect("cleaned");
        for key in [
            "agent",
            "root_pid",
            "waited_ms",
            "terminated",
            "rounds",
            "terminate_max_us",
            "unconfirmed",
            "births",
            "parent_alive",
            "parent_unknown",
            "not_hook_copy",
            "ancestor_alive",
            "chain_cut",
            "not_hook",
            "no_taskkill",
            "unknown",
        ] {
            assert!(
                line.contains_key(key),
                "the {key} field is missing: {line:?}"
            );
        }
        assert_eq!(line["agent"], AgentId::from_u128(0x7440).to_string());
        assert_eq!(line["root_pid"], "37152");
        assert_eq!(line["waited_ms"], "3000");
        assert_eq!(line["rounds"], "2");
        assert_eq!(line["terminate_max_us"], "150");
        assert_eq!(line["unconfirmed"], "0");
        assert_eq!(line["births"], "14");
        assert_eq!(
            line["terminated"],
            "[pid=32228 ppid=25028 chain=[32228, 25028, 35688] round=1, \
             pid=24776 ppid=32228 chain=[24776, 32228, 25028, 35688] round=2]"
        );
        assert_eq!(rig.count(Level::WARN), 1);
        rig.assert_lock_sections();
    }

    /// Running as the worker — sleep N after the pass that killed, and if it is still the same episode then, warn once and end.
    #[test]
    fn the_worker_reports_once_more_n_after_a_pass_that_killed() {
        let rig = GateRig::staged(&pilot().staged(), &[]);
        assert!(rig.turn());
        assert_eq!(rig.terminates(), [3256]);
        assert!(matches!(rig.episode(|ep| ep.phase), Phase::Spent { .. }));
        assert!(rig.turn());
        assert_eq!(rig.clock.sleeps(), [N]);
        assert!(rig.turn());
        assert!(rig.logged_text("not doing more"));
        assert!(!rig.turn());
        assert_eq!(rig.count(Level::WARN), 2);
        assert_eq!(rig.recorder.active(), 0);
        rig.assert_lock_sections();
    }

    /// Debug does not wait on the lock.
    #[test]
    fn the_debug_view_never_waits_for_the_lock() {
        let rig = GateRig::new();
        rig.open();
        assert!(format!("{:?}", rig.cell).contains("open: true"));
        let held = rig.cell.lock();
        let text = format!("{:?}", rig.cell);
        drop(held);
        assert!(text.contains("<locked>"), "{text}");
    }

    // ── State machine · invariants ─────────────────────────────────────────────────────────────

    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: u64) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x % n
        }
    }

    /// After every arbitrary transition sequence (interrupt · event between two sections · write confirmation · dropped confirmation ·
    ///   three decoder calls · time · worker step · list failure · listener
    /// thread end · worker start failure · retirement) the TRD §3-2 invariants and the lock rules hold. It is deterministic (fixed seed).
    #[test]
    fn random_transition_sequences_keep_every_invariant() {
        let (mut passes, mut opened, mut line_only, mut no_line) = (0, 0, 0, 0);
        for seed in 1..=200u64 {
            let rig = GateRig::new();
            rig.clock.keep_workers.store(false, Ordering::SeqCst);
            let pending: Arc<Mutex<Vec<OnWritten>>> = Arc::default();
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let take = |out: Option<InterruptOut>, line_only: &mut u32, no_line: &mut u32| match out
            {
                Some(InterruptOut {
                    on_written: Some(cb),
                    ..
                }) => pending.lock().unwrap().push(cb),
                Some(_) => *line_only += 1,
                None => *no_line += 1,
            };
            for _ in 0..200 {
                match rng.below(32) {
                    0..=5 => take(rig.esc(), &mut line_only, &mut no_line),
                    6 | 7 => {
                        let (cell, recorder, pend) = (
                            Arc::clone(&rig.cell),
                            Arc::clone(&rig.recorder),
                            Arc::clone(&pending),
                        );
                        let inner = rng.below(6);
                        rig.on_list(move || match inner {
                            0 => cell.close_turn(),
                            1 => cell.deliver(),
                            2 => {
                                cell.close_turn();
                                cell.open_turn();
                            }
                            3 => drop(recorder.stop(recorder.active())),
                            4 => {
                                if let Some(InterruptOut {
                                    on_written: Some(cb),
                                    ..
                                }) = cell.interrupt(LINE.to_vec())
                                {
                                    pend.lock().unwrap().push(cb);
                                }
                            }
                            _ => cell.open_turn(),
                        });
                        take(rig.esc(), &mut line_only, &mut no_line);
                    }
                    8..=12 => {
                        let cb = {
                            let mut p = pending.lock().unwrap();
                            if p.is_empty() {
                                None
                            } else {
                                let i = rng.below(p.len() as u64) as usize;
                                Some(p.swap_remove(i))
                            }
                        };
                        if let Some(cb) = cb {
                            rig.logged(cb);
                        }
                    }
                    13 => {
                        let mut p = pending.lock().unwrap();
                        if !p.is_empty() {
                            let i = rng.below(p.len() as u64) as usize;
                            drop(p.swap_remove(i));
                        }
                    }
                    14 | 15 => rig.open(),
                    16 => rig.logged(|| rig.cell.deliver()),
                    17 => rig.logged(|| rig.cell.close_turn()),
                    18..=20 => rig.clock.advance(ms(rng.below(4_000))),
                    21..=26 => {
                        if rig.worker() {
                            let now = rig.clock.now();
                            let due = rig.state(|st| {
                                st.episode.as_ref().is_some_and(|ep| {
                                    ep.phase == Phase::Waiting
                                        && ep.written
                                        && ep.last_mono + N <= now
                                })
                            });
                            rig.turn();
                            if due && !rig.group.retiring.load(Ordering::SeqCst) {
                                passes += 1;
                            }
                        }
                    }
                    27 => {
                        let failing = rig.group.listing.lock().unwrap().is_none();
                        rig.listing((failing).then(|| vec![CLAUDE, ROOT, CONHOST]));
                    }
                    28 => drop(rig.recorder.stop(rig.recorder.active())),
                    29 => {
                        let refuse = rng.below(3) == 0;
                        rig.clock.refuse_worker.store(refuse, Ordering::SeqCst);
                    }
                    _ => {
                        if rng.below(20) == 0 {
                            rig.group.retiring.store(true, Ordering::Release);
                        }
                    }
                }
                rig.check();
            }
            opened += rig.state(|st| st.next_gen);
            rig.logged(|| rig.cell.close_turn());
            pending.lock().unwrap().clear();
            rig.check();
            assert_eq!(rig.recorder.active(), 0);
            assert!(!rig.has_episode());
        }
        let seen = format!(
            "passes {passes} · episodes {opened} · line only {line_only} · no line {no_line}"
        );
        assert!(
            passes > 40 && opened > 500 && line_only > 50 && no_line > 50,
            "{seen}"
        );
    }
}

/// Connects the real group · port · listener thread · write confirmation · worker · pass in one line (TRD §5 real process 12); each test
///   starts four or fewer
/// (including the console host). ★The kill path is not measured★ — under the real hook runner shape copies exceed four, so the pure
///   spike-shape tests measure it.
/// Here: the pass's reason is left in the log and nothing is killed.
#[cfg(all(test, windows))]
mod real_tests {
    use super::test_support::{Capture, Line};
    use super::*;

    use std::collections::BTreeMap;
    use std::io::Write;

    use crate::transport::process_group::tests::new_group;
    use engram_dashboard_platform::testing::{is_ping, open_gate, spawn_gated_cmd, wait_until};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const LINE: &[u8] = b"{\"type\":\"control_request\"}\n";

    /// Sleeps do not wait and advance monotonic time by that amount — the worker passes N immediately. The worker body is piled up and the
    ///   test thread
    /// runs it (to capture the pass's log). The listener thread is started for real.
    #[derive(Default)]
    struct SkipClock {
        skipped_ns: AtomicU64,
        workers: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
    }

    impl Clock for SkipClock {
        fn now(&self) -> Instant {
            Instant::now() + Duration::from_nanos(self.skipped_ns.load(Ordering::SeqCst))
        }
    }

    impl LeftoverClock for SkipClock {
        fn sleep(&self, d: Duration) {
            let ns = u64::try_from(d.as_nanos()).expect("test sleep");
            self.skipped_ns.fetch_add(ns, Ordering::SeqCst);
        }

        fn spawn(&self, name: &str, body: Box<dyn FnOnce() + Send>) -> io::Result<()> {
            if name == WORKER_THREAD {
                self.workers.lock().unwrap().push(body);
                Ok(())
            } else {
                SystemClock.spawn(name, body)
            }
        }
    }

    /// The cleaner of root `root` and its gate — the assembly of `open_spawn` with only the clock swapped.
    fn cleaner_for(
        group: &Arc<ProcessGroup>,
        root: u32,
    ) -> (Arc<SkipClock>, Arc<Cleaner>, Arc<GateCell>) {
        let clock = Arc::new(SkipClock::default());
        let cleaner = Arc::new(Cleaner::new(
            Arc::clone(group),
            Arc::clone(&clock) as Arc<dyn LeftoverClock>,
            root,
            LogTag::new(None, root),
        ));
        let cell = GateCell::new(Some(Arc::clone(&cleaner)));
        (clock, cleaner, cell)
    }

    /// Receive one interrupt in an open turn and treat its line as written — write confirmation takes the write list and calls one worker.
    /// Returns = that episode's record number.
    fn interrupt_and_write(cell: &Arc<GateCell>, cleaner: &Cleaner, clock: &SkipClock) -> u64 {
        cell.open_turn();
        let out = cell.interrupt(LINE.to_vec()).expect("interrupt line");
        let rec = cleaner.recorder.active();
        assert_ne!(rec, 0, "the record did not turn on");
        let on_written = out.on_written.expect("write confirmation");
        on_written();
        assert_eq!(
            clock.workers.lock().unwrap().len(),
            1,
            "the written interrupt did not call the worker"
        );
        rec
    }

    /// Run the piled-up workers to the end on this thread and give the logs in the meantime.
    fn run_workers(clock: &SkipClock) -> Vec<Line> {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let bodies: Vec<_> = clock.workers.lock().unwrap().drain(..).collect();
        for body in bodies {
            tracing::subscriber::with_default(Capture(Arc::clone(&lines)), body);
        }
        let taken = lines.lock().unwrap().clone();
        taken
    }

    /// One reason line the pass left — nothing was killed (there is no cleaned line).
    fn the_pass_line(lines: &[Line]) -> BTreeMap<String, String> {
        assert!(
            !lines
                .iter()
                .any(|(_, fields)| fields.contains_key("terminated")),
            "ended something: {lines:?}"
        );
        let mut reasons = lines
            .iter()
            .filter(|(_, fields)| fields.contains_key("reason"));
        let (_, fields) = reasons.next().expect("the pass's reason line");
        assert!(
            reasons.next().is_none(),
            "there are two reason lines: {lines:?}"
        );
        fields.clone()
    }

    /// Wait until R in the windowless console starts the console host — before the gate opens, that is R's only child. That host
    /// is sometimes born after R enters the Job (measured), so if not waited for, it slips in as a live birth after the write list (parent
    ///   R alive).
    fn console_host_born(r: &std::process::Child) {
        wait_until("R's console host", || {
            (!engram_dashboard_platform::process::child_pids(r.id()).is_empty()).then_some(())
        });
    }

    /// After the interrupt, B (`cmd`) started by R launched P (ping) and exited — P's parent has exited, but P is not a copy of B. P is alive.
    /// Four: R · its console host · B · P.
    #[test]
    fn a_ping_left_by_an_exited_cmd_is_not_a_hook_copy_and_survives() {
        let (job, group) = new_group();
        let group = Arc::new(group);
        // R = gate → B → waits for a second line (so the root is alive at pass time). B = launches P in the background, waits for one
        //   line, then exits.
        let mut r = spawn_gated_cmd(
            r#"cmd /d /c "start "" /b ping -n 30 127.0.0.1 >nul & set /p _=" & set /p _="#,
            CREATE_NO_WINDOW,
        );
        job.adopt(r.id()).expect("adopt into Job");
        console_host_born(&r);
        let (clock, cleaner, cell) = cleaner_for(&group, r.id());
        let rec = interrupt_and_write(&cell, &cleaner, &clock);

        let mut stdin = r.stdin.take().expect("stdin pipe");
        stdin.write_all(b"go\r\n").expect("R's gate");
        let (b, p) = wait_until("recorded B · P", || {
            let births = cleaner.recorder.copy(rec);
            let b = births
                .iter()
                .find(|x| x.facts.ppid == r.id() && image_is(&x.facts.image, "cmd.exe"))
                .cloned()?;
            let p = births
                .iter()
                .find(|x| x.facts.ppid == b.pid && image_is(&x.facts.image, "ping.exe"))
                .cloned()?;
            Some((b, p))
        });
        assert!(b.killable && p.killable);
        stdin.write_all(b"b\r\n").expect("B's line");
        wait_until("ended B", || {
            matches!(b.pin.exited(), Ok(true)).then_some(())
        });

        let pass = the_pass_line(&run_workers(&clock));
        assert_eq!(pass["reason"], "parent_rule", "{pass:?}");
        assert_eq!(pass["not_hook_copy"], "1", "{pass:?}");
        assert_eq!(pass["parent_alive"], "0", "{pass:?}");
        assert!(!p.pin.exited().expect("exited?"), "P ended");

        drop((b, p));
        job.terminate(1).expect("end the Job");
        drop(stdin);
        let _ = r.wait();
    }

    /// Only things born before the interrupt exist — they are in the write list, so there is no candidate (even if they enter the record
    ///   by re-notification on attach). Three: R · its console
    /// host · ping.
    #[test]
    fn births_before_the_interrupt_leave_no_new_member() {
        let (job, group) = new_group();
        let group = Arc::new(group);
        let mut r = spawn_gated_cmd("ping -n 30 127.0.0.1 >nul", CREATE_NO_WINDOW);
        job.adopt(r.id()).expect("adopt into Job");
        open_gate(&mut r);
        wait_until("ping in the list", || {
            group
                .member_pids()
                .ok()?
                .into_iter()
                .find(|&pid| is_ping(pid))
        });
        let (clock, cleaner, cell) = cleaner_for(&group, r.id());
        interrupt_and_write(&cell, &cleaner, &clock);

        let pass = the_pass_line(&run_workers(&clock));
        assert_eq!(pass["reason"], "no_new_member", "{pass:?}");

        job.terminate(1).expect("end the Job");
        let _ = r.wait();
    }

    /// Born after the interrupt, but the parent (root R) is alive. Three: R · its console host · ping.
    #[test]
    fn a_birth_under_a_live_parent_is_parent_alive() {
        let (job, group) = new_group();
        let group = Arc::new(group);
        let mut r = spawn_gated_cmd("ping -n 30 127.0.0.1 >nul", CREATE_NO_WINDOW);
        job.adopt(r.id()).expect("adopt into Job");
        console_host_born(&r);
        let (clock, cleaner, cell) = cleaner_for(&group, r.id());
        let rec = interrupt_and_write(&cell, &cleaner, &clock);

        open_gate(&mut r);
        let p = wait_until("recorded ping", || {
            cleaner
                .recorder
                .copy(rec)
                .into_iter()
                .find(|x| x.facts.ppid == r.id() && image_is(&x.facts.image, "ping.exe"))
        });

        let pass = the_pass_line(&run_workers(&clock));
        assert_eq!(pass["reason"], "parent_rule", "{pass:?}");
        assert_eq!(pass["parent_alive"], "1", "{pass:?}");
        assert_eq!(pass["not_hook_copy"], "0", "{pass:?}");
        assert!(!p.pin.exited().expect("exited?"), "P ended");

        drop(p);
        job.terminate(1).expect("end the Job");
        let _ = r.wait();
    }
}
