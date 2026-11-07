//! The recorder — the only one that writes `state.json` after the boot steps and the only one that deletes the crash copy `state.crash.json`
//! (TRD S21-storage §6-4). One thread runs, and other threads only send requests through [`SaverHandle`] and wait for the answer
//! — there is a single writer, so writes do not overlap.
//!
//! - **Periodic save:** every [`POLL_INTERVAL`] it reads the change number, and if it changed, it writes the whole snapshot when it has been
//!   quiet for [`QUIET`] after the last change or [`MAX_DELAY`] has passed after the first change.
//! - **A write failure = log only** (user decision D8). If the snapshot source fails · on a disk error, that change is left as not written
//!   and the debounce writes again; a snapshot the codec rejected (a shape the next boot cannot read) waits for the next change. A clean-exit
//!   write (`Final`) is not redone even on failure — the recorder just ends.
//! - **Right before every write it decides again on the current `state.json`** (the base `file` rules — down to the header shape). If it is a
//!   file written by a newer version, it does not write and, like a codec rejection, waits for the next change; if it could not read it, it decides again after the debounce, like a disk error.
//!   A corrupt file · a usable file that was placed while this run had not yet written is copied aside as `state.json.corrupt` and then written — it writes
//!   even if copying aside fails (D8).
//! - **Resolved slot** = the hash of the crash copy that was answered. ★It stands when the recorder **takes out** `Resolve`★ — if the sender sets it, a snapshot
//!   taken before it is taken out (one that does not yet hold the post-answer screen) carries the hash (I8). From then on it is carried by every snapshot written, and each time a write
//!   succeeds it rereads the copy and deletes it only if the hash is the same (N1). If deleted · absent · a different copy, the slot is cleared.
//! - **Closed** = a marker set when [`SaverHandle::finish`] misses its deadline. Once it stands, neither publishing (rename) nor deleting the copy nor
//!   copying aside happens — the next boot reads the preceding `clean_exit:false` and asks.
//!
//! The recorder holds no lock — the snapshot source copies inside its own lock and hands it over, and serialization · disk happen outside it
//! (§6-3).

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use engram_dashboard_base::file::{self, Claim, Refused, WriteOutcome, WritePolicy};
use engram_dashboard_base::time::{now_epoch_ms, Clock, SystemClock};

use super::codec::{self, EncodeError, STATE_READ_CAP};
use super::schema::{StateFile, WindowEntry, STATE_VERSION};
use crate::file_hooks::OS_HOOKS;

pub const POLL_INTERVAL: Duration = Duration::from_millis(500);
pub const QUIET: Duration = Duration::from_secs(1);
pub const MAX_DELAY: Duration = Duration::from_secs(5);
/// The deadline for which exit (`Final`) and restore settlement (`Resolve`) wait for the answer (TRD §6-4 · §6-7). ★The boot's shell-run lock wait (about
/// 3 seconds — §6-5 ①) must be longer than this★ — if shorter, a new instance does not wait for the previous instance's clean-exit write.
pub const REPLY_DEADLINE: Duration = Duration::from_secs(2);

/// The snapshot source — production is `ViewManager` ([`super::boot_plugin::LiveSource`]). Called on the recorder thread.
///
/// - ★It does not panic★ — the shell isolates nothing, so a panic on this thread makes the hook end the whole app (ADR-0292).
///   A poisoned lock is revived (`engram_dashboard_base::sync`) or [`Self::snapshot`] returns `Err`.
/// - ★It does not wait for the thread that called [`SaverHandle::finish`] · [`SaverHandle::resolve`]★ — that thread is standing
///   waiting for the answer. If it tries to take a lock that thread holds, the recorder stands until the deadline, and for `finish` a clean exit
///   would miss the deadline every time and every clean exit would be read as an abnormal exit.
/// - ★It does not call Tauri window getters (`outer_position` · `inner_size` · `is_maximized`, etc.)★ — called from off the main thread,
///   it stands until the main thread answers, but `finish` is called by exactly that main thread in `RunEvent::Exit`, which then
///   waits for the answer. Position · size are written into the model at `WindowEvent` (P3b) and here only the model is read.
pub trait SnapshotSource: Send + 'static {
    /// The change number — the recorder only checks whether it is the same (it does not look at magnitude). ★Do not roll the number back or reset it★ — if it becomes the same again as
    /// the value the recorder last saw, it misses the changes in between.
    type Revision: Clone + PartialEq + Send + 'static;

    /// Called every 0.5 seconds — it must finish quickly.
    ///
    /// ★The number must go up in the same critical section in which that change becomes visible to [`Self::snapshot`], or after it★ — the recorder
    /// reads the number first, takes the snapshot, and then treats that number as 「쓴 것」 (written). If the number goes up before the change, a snapshot that missed that change
    /// stays as the last state on disk until the next change.
    fn revision(&self) -> Self::Revision;

    /// All windows — copied inside its own lock and returned as an owned value. The file header (`version` · `saved_at_ms` ·
    /// `clean_exit` · `resolved_crash_copy`) is filled in by the recorder.
    ///
    /// `Err` = it cannot be taken this time (one reason line) — the recorder treats it as a failed write like a disk error and takes it again
    /// after the debounce (for a clean-exit write it just ends).
    fn snapshot(&self) -> Result<Vec<WindowEntry>, String>;
}

/// The two files the recorder touches — production is [`Fs`].
pub trait StateFiles: Send + 'static {
    /// Atomically swap `state.json` — ask `skip` before every rename, and if it says stand, do not publish and return
    /// [`WriteOutcome::Skipped`] (the same contract as `file::write_atomic_unless`). Create the folder if it is missing.
    fn write_state(&self, text: &str, skip: &dyn Fn() -> bool) -> io::Result<WriteOutcome>;

    /// The original text of `state.json` — the source of the re-decision right before saving. The same contract as [`Self::read_crash_copy`].
    // ADR-0291 R16
    fn read_state(&self) -> io::Result<String>;

    /// Copy `state.json` aside as `state.json.corrupt` — returns the copy's location (`file::copy_aside`).
    fn copy_aside_state(&self) -> io::Result<PathBuf>;

    /// The crash copy's original text — `NotFound` if absent, `InvalidData` if over the limit · not UTF-8
    /// (the same contract as `file::read_file_capped`).
    fn read_crash_copy(&self) -> io::Result<String>;

    /// `NotFound` if absent.
    fn remove_crash_copy(&self) -> io::Result<()>;
}

/// The recorder's clock — the current read ([`Clock::now`]) only measures the debounce, and this trait adds the snapshot's `saved_at_ms`. The real one =
/// [`SystemClock`].
// ADR-0275 · ADR-0291
pub trait SaverClock: Clock + 'static {
    /// The snapshot's `saved_at_ms` — Unix time in ms.
    fn wall_ms(&self) -> u64;
}

impl SaverClock for SystemClock {
    fn wall_ms(&self) -> u64 {
        u64::try_from(now_epoch_ms()).unwrap_or(0)
    }
}

/// Production files — the caller decides the path (placement = [`super::boot_plugin`]).
pub struct Fs {
    state: PathBuf,
    crash_copy: PathBuf,
}

impl Fs {
    pub fn new(state: PathBuf, crash_copy: PathBuf) -> Self {
        Self { state, crash_copy }
    }
}

impl StateFiles for Fs {
    fn write_state(&self, text: &str, skip: &dyn Fn() -> bool) -> io::Result<WriteOutcome> {
        file::write_atomic_unless(&self.state, text.as_bytes(), OS_HOOKS, skip)
    }

    fn read_state(&self) -> io::Result<String> {
        file::read_file_capped(&self.state, STATE_READ_CAP, OS_HOOKS)
    }

    fn copy_aside_state(&self) -> io::Result<PathBuf> {
        file::copy_aside(&self.state, OS_HOOKS)
    }

    fn read_crash_copy(&self) -> io::Result<String> {
        file::read_file_capped(&self.crash_copy, STATE_READ_CAP, OS_HOOKS)
    }

    // No lock-retry — if it cannot be deleted, the resolved slot stays and is deleted again at every later successful write.
    fn remove_crash_copy(&self) -> io::Result<()> {
        std::fs::remove_file(&self.crash_copy)
    }
}

/// The result of one recorder write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    /// Published (rename).
    Written,
    /// Not published because it is closed — not a failure and not redone.
    Skipped,
    /// Snapshot source failure · codec rejection ([`EncodeError`]) · rejection by the re-decision right before saving (newer version · unreadable) · disk error.
    /// The resolved slot stays as is, so the next successful write carries it — after a failure of the clean-exit write (`Final`) there is no next write.
    Failed,
}

/// The result of sending a request and waiting until the deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestOutcome {
    Done(SaveOutcome),
    /// No answer within the deadline — the request stays in the queue and may be processed later.
    TimedOut,
    /// No answer will come — the recorder thread has already ended or died while processing.
    SaverGone,
}

/// The handle for sending requests to the recorder — cloned and shared. When all handles are gone, the recorder ends without a clean-exit
/// write (the next boot reads it as an abnormal exit).
#[derive(Clone)]
pub struct SaverHandle {
    requests: Sender<Request>,
    closed: Arc<AtomicBool>,
}

impl SaverHandle {
    /// Attach the hash of the answered crash copy to disk (TRD §6-7 ⑤). `Done(Written)` = a `state.json` carrying that hash
    /// was published — it does not carry whether deleting the copy succeeded.
    ///
    /// ★While waiting, do not hold a lock (`ViewManager`) that [`SnapshotSource`] will take★ — if held, the recorder stands
    /// at that lock until the deadline and the answer becomes `TimedOut`.
    pub fn resolve(&self, hash: String, deadline: Duration) -> RequestOutcome {
        match self.send(|reply| Request::Resolve { hash, reply }) {
            Some(answer) => wait(answer, deadline),
            None => RequestOutcome::SaverGone,
        }
    }

    /// Write `clean_exit:true` and end the recorder. ★If the deadline is missed, set closed and return — it does not wait for the stuck
    /// thread★. After that the recorder does not publish — except a write that passed rename just before closed was set has already been published
    /// (its content is this `Final` or an earlier `clean_exit:false` — §12 R12).
    ///
    /// ★While waiting, do not hold a lock (`ViewManager`) that [`SnapshotSource`] will take★ — if held, `Final` always
    /// misses the deadline and every clean exit is read as an abnormal exit.
    pub fn finish(&self, deadline: Duration) -> RequestOutcome {
        let outcome = match self.send(|reply| Request::Final { reply }) {
            Some(answer) => wait(answer, deadline),
            None => RequestOutcome::SaverGone,
        };
        if outcome == RequestOutcome::TimedOut {
            self.closed.store(true, Ordering::SeqCst);
            tracing::warn!(
                module = "state",
                deadline_ms = deadline.as_millis() as u64,
                "정상 종료 쓰기가 마감 안에 끝나지 않아 기록기를 닫는다 — 다음 부팅은 비정상 종료로 읽는다"
            );
        }
        outcome
    }

    /// Set closed — after that the recorder does not publish (writes · `Resolve` · `Final` are all `Skipped`). The thread ends when all
    /// handles are gone.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn send(
        &self,
        request: impl FnOnce(Sender<SaveOutcome>) -> Request,
    ) -> Option<Receiver<SaveOutcome>> {
        let (reply, answer) = mpsc::channel();
        self.requests.send(request(reply)).ok().map(|()| answer)
    }
}

/// The recorder's closed marker — made before [`spawn`] and shared with the side that starts it. When set, that recorder does not publish
/// (the same marker as [`SaverHandle::close`]). The side that has no handle yet while starting (exit) can also close with it.
#[derive(Clone, Default)]
pub struct CloseFlag(Arc<AtomicBool>);

impl CloseFlag {
    pub fn close(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn wait(answer: Receiver<SaveOutcome>, deadline: Duration) -> RequestOutcome {
    match answer.recv_timeout(deadline) {
        Ok(outcome) => RequestOutcome::Done(outcome),
        Err(RecvTimeoutError::Timeout) => RequestOutcome::TimedOut,
        Err(RecvTimeoutError::Disconnected) => RequestOutcome::SaverGone,
    }
}

/// Start the recorder thread. `boot_revision` = the change number carried by the boot's first write (§6-5 ⑤) — the first period carries what changed after that.
/// `carry_resolved` = the hash of the answered copy that boot could not delete — the first write already carried it, so retry the deletion
/// after the next successful write. `claim` = whether this run has already claimed `state.json` as its own (boot decides —
/// `boot::saver_claim`) — if `NotYet`, copy aside the usable file in that spot before the first write. `closed` = this recorder's
/// closed marker — if set before starting, it does not publish from the start.
///
/// `Err` = the thread could not be started.
pub fn spawn<S, F, C>(
    source: S,
    files: F,
    clock: C,
    boot_revision: S::Revision,
    carry_resolved: Option<String>,
    claim: Claim,
    closed: CloseFlag,
) -> io::Result<SaverHandle>
where
    S: SnapshotSource,
    F: StateFiles,
    C: SaverClock,
{
    let saver = Saver::new(
        source,
        files,
        clock,
        closed.0,
        boot_revision,
        carry_resolved,
        claim,
    );
    start(saver, POLL_INTERVAL).map(|(handle, _detached)| handle)
}

fn start<S, F, C>(
    saver: Saver<S, F, C>,
    poll: Duration,
) -> io::Result<(SaverHandle, JoinHandle<()>)>
where
    S: SnapshotSource,
    F: StateFiles,
    C: SaverClock,
{
    let (requests, inbox) = mpsc::channel();
    let handle = SaverHandle {
        requests,
        closed: Arc::clone(&saver.closed),
    };
    let thread = std::thread::Builder::new()
        .name("state-saver".into())
        .spawn(move || run(saver, inbox, poll))?;
    Ok((handle, thread))
}

enum Request {
    Resolve {
        hash: String,
        reply: Sender<SaveOutcome>,
    },
    Final {
        reply: Sender<SaveOutcome>,
    },
}

fn run<S, F, C>(mut saver: Saver<S, F, C>, inbox: Receiver<Request>, poll: Duration)
where
    S: SnapshotSource,
    F: StateFiles,
    C: SaverClock,
{
    let _panic_note = PanicNote;
    tracing::info!(module = "state", "기록기 시작");
    // Measure by deadline time so that checks do not get pushed back even when requests are frequent.
    let mut next_tick = saver.clock.now() + poll;
    loop {
        let wait = next_tick.saturating_duration_since(saver.clock.now());
        match inbox.recv_timeout(wait) {
            Ok(Request::Resolve { hash, reply }) => {
                let outcome = saver.resolve(hash);
                deliver(reply, outcome);
            }
            Ok(Request::Final { reply }) => {
                let outcome = saver.save(Cause::Final);
                deliver(reply, outcome);
                tracing::info!(
                    module = "state",
                    ?outcome,
                    "기록기 종료 — 정상 종료 쓰기 뒤"
                );
                return;
            }
            Err(RecvTimeoutError::Timeout) => {
                saver.tick();
                next_tick = saver.clock.now() + poll;
            }
            Err(RecvTimeoutError::Disconnected) => {
                tracing::warn!(
                    module = "state",
                    "손잡이가 모두 사라져 기록기가 정상 종료 쓰기 없이 끝난다 — 다음 부팅은 비정상 종료로 읽는다"
                );
                return;
            }
        }
    }
}

/// Leave one line when the recorder thread is released by a panic. ★It is reached only in test builds without the hook★ — in the shell the hook leaves
/// a reason line and ends the whole process before unwinding, so it never gets to this line (ADR-0292).
struct PanicNote;

impl Drop for PanicNote {
    fn drop(&mut self) {
        if std::thread::panicking() {
            tracing::error!(
                module = "state",
                "기록기 스레드가 패닉으로 죽었다 — 이 실행은 상태를 더 저장하지 않는다"
            );
        }
    }
}

fn deliver(reply: Sender<SaveOutcome>, outcome: SaveOutcome) {
    if reply.send(outcome).is_err() {
        tracing::debug!(
            module = "state",
            ?outcome,
            "요청한 쪽이 마감을 넘겨 답을 받지 않았다"
        );
    }
}

#[derive(Debug, Clone, Copy)]
enum Cause {
    Periodic,
    Resolve,
    Final,
}

#[derive(Debug)]
enum Failure {
    Snapshot(String),
    Rejected(EncodeError),
    /// Rejection by the re-decision right before saving — the current `state.json` was written by a version newer than this shell (that version).
    Newer {
        found: u64,
    },
    /// Rejection by the re-decision right before saving — the current `state.json` could not be read, so it is unknown whether overwriting is allowed.
    Unreadable(io::Error),
    Disk(io::Error),
}

impl Failure {
    fn kind(&self) -> FailureKind {
        match self {
            Failure::Snapshot(_) => FailureKind::Snapshot,
            Failure::Rejected(_) => FailureKind::Rejected,
            Failure::Newer { .. } => FailureKind::Newer,
            Failure::Unreadable(_) => FailureKind::Unreadable,
            Failure::Disk(_) => FailureKind::Disk,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Snapshot(reason) => f.write_str(reason),
            Failure::Rejected(error) => error.fmt(f),
            Failure::Newer { found } => write!(
                f,
                "state.json 이 이 셸보다 새 판(version {found} — 이 셸은 {STATE_VERSION} 까지)이 쓴 것이다"
            ),
            Failure::Unreadable(error) => write!(f, "state.json 을 못 읽었다: {error}"),
            Failure::Disk(error) => error.fmt(f),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    Snapshot,
    Rejected,
    Newer,
    Unreadable,
    Disk,
}

/// The branch where cleaning up the answered copy (N1) failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyFailure {
    Read,
    Remove,
}

/// A failure that continues without a break. Only when the branch is the first or changes is it logged at its own level, and a repeat of the same branch is folded to debug
/// — if a disk error continues it rewrites every second, so without folding the file log fills with that line.
#[derive(Debug, PartialEq, Eq)]
struct Streak<K> {
    current: Option<(K, u32)>,
}

impl<K> Default for Streak<K> {
    fn default() -> Self {
        Self { current: None }
    }
}

impl<K: Copy + PartialEq> Streak<K> {
    /// Count one failure — `true` = it is the turn to log at its own level.
    fn fail(&mut self, kind: K) -> bool {
        match &mut self.current {
            Some((current, failures)) => {
                *failures += 1;
                let changed = *current != kind;
                *current = kind;
                changed
            }
            None => {
                self.current = Some((kind, 1));
                true
            }
        }
    }

    fn failures(&self) -> u32 {
        self.current.map_or(0, |(_, failures)| failures)
    }

    /// `Some(n)` = the failure that was continuing ended after n times.
    fn end(&mut self) -> Option<u32> {
        self.current.take().map(|(_, failures)| failures)
    }
}

struct Saver<S: SnapshotSource, F: StateFiles, C: SaverClock> {
    source: S,
    files: F,
    clock: C,
    closed: Arc<AtomicBool>,
    /// The last number seen — if different from this, it is a new change.
    seen: S::Revision,
    /// The time something not yet on disk (a change · a failed write) arose — `None` = nothing to write.
    pending_since: Option<Instant>,
    last_change_at: Instant,
    resolved: Option<String>,
    /// Has this run already claimed `state.json` as its own — `Adopted` after the first successful write.
    claim: Claim,
    write_failures: Streak<FailureKind>,
    copy_failures: Streak<CopyFailure>,
    /// Continuation of copy-aside failures — the write still happens, so it is folded separately from the write-failure line.
    aside_failures: Streak<()>,
}

impl<S: SnapshotSource, F: StateFiles, C: SaverClock> Saver<S, F, C> {
    fn new(
        source: S,
        files: F,
        clock: C,
        closed: Arc<AtomicBool>,
        boot_revision: S::Revision,
        carry_resolved: Option<String>,
        claim: Claim,
    ) -> Self {
        let now = clock.now();
        Self {
            source,
            files,
            clock,
            closed,
            seen: boot_revision,
            pending_since: None,
            last_change_at: now,
            resolved: carry_resolved,
            claim,
            write_failures: Streak::default(),
            copy_failures: Streak::default(),
            aside_failures: Streak::default(),
        }
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn tick(&mut self) {
        if self.is_closed() {
            return;
        }
        let now = self.clock.now();
        let revision = self.source.revision();
        if revision != self.seen {
            self.seen = revision;
            self.last_change_at = now;
            self.pending_since.get_or_insert(now);
        }
        let Some(since) = self.pending_since else {
            return;
        };
        if now.duration_since(self.last_change_at) >= QUIET
            || now.duration_since(since) >= MAX_DELAY
        {
            self.save(Cause::Periodic);
        }
    }

    fn resolve(&mut self, hash: String) -> SaveOutcome {
        self.resolved = Some(hash);
        self.save(Cause::Resolve)
    }

    fn save(&mut self, cause: Cause) -> SaveOutcome {
        if self.is_closed() {
            return SaveOutcome::Skipped;
        }
        // Read the number before the snapshot — a change in between is in the snapshot but remains 「안 씀」 (not written) by number, so it is merely written one more time.
        // The other way round, a change not in the snapshot would be treated as written.
        let revision = self.source.revision();
        let windows = match self.source.snapshot() {
            Ok(windows) => windows,
            Err(reason) => {
                self.failed(cause, Failure::Snapshot(reason), revision);
                return SaveOutcome::Failed;
            }
        };
        let state = StateFile {
            version: STATE_VERSION,
            saved_at_ms: self.clock.wall_ms(),
            clean_exit: matches!(cause, Cause::Final),
            resolved_crash_copy: self.resolved.clone(),
            windows,
        };
        let text = match codec::encode(&state) {
            Ok(text) => text,
            Err(rejected) => {
                self.failed(cause, Failure::Rejected(rejected), revision);
                return SaveOutcome::Failed;
            }
        };
        if let Err(refused) = self.judge_state() {
            self.failed(cause, refused, revision);
            return SaveOutcome::Failed;
        }
        let closed = &self.closed;
        match self
            .files
            .write_state(&text, &|| closed.load(Ordering::SeqCst))
        {
            Ok(WriteOutcome::Written) => {
                if let Some(failures) = self.write_failures.end() {
                    tracing::info!(
                        module = "state",
                        failures,
                        "상태 파일 쓰기가 실패 뒤 다시 성공했다"
                    );
                }
                // ADR-0291 R15: once a write succeeds, the copy-aside failure count also restarts from the beginning (the next failure is error again).
                self.aside_failures.end();
                match cause {
                    Cause::Periodic => {
                        tracing::debug!(module = "state", bytes = text.len(), "상태 파일을 썼다")
                    }
                    Cause::Resolve | Cause::Final => tracing::info!(
                        module = "state",
                        ?cause,
                        resolved = self.resolved.is_some(),
                        "상태 파일을 썼다"
                    ),
                }
                self.seen = revision;
                self.pending_since = None;
                self.claim = Claim::Adopted;
                self.drop_answered_copy();
                SaveOutcome::Written
            }
            Ok(WriteOutcome::Skipped) => {
                tracing::info!(
                    module = "state",
                    ?cause,
                    "기록기가 닫혀 상태 파일을 발행하지 않았다"
                );
                SaveOutcome::Skipped
            }
            Err(error) => {
                self.failed(cause, Failure::Disk(error), revision);
                SaveOutcome::Failed
            }
        }
    }

    /// Right before writing, read the current `state.json` again and decide — `Ok` if overwriting is allowed (if it must be copied aside, after copying aside · `Ok` even if copying
    /// aside fails), and if rejected, the reason.
    ///
    /// ★Do not hold the decision from load time★ — do not overwrite a file that changed to a newer version during the run, and after the first write do not
    /// copy our own file aside on every write.
    // ADR-0291 R8 · R16 (D5 · D14 · E5)
    fn judge_state(&mut self) -> Result<(), Failure> {
        let current = file::classify(self.files.read_state(), &codec::STATE_SPEC);
        match current.write_policy(self.claim) {
            WritePolicy::Write => Ok(()),
            WritePolicy::CopyAsideFirst => {
                // After closed, that write is not published, so there is no reason to copy aside — do not touch the file of a new instance that passed
                //   the gate.
                if !self.is_closed() {
                    self.copy_state_aside(&current);
                }
                Ok(())
            }
            WritePolicy::Refuse(Refused::Newer { found }) => Err(Failure::Newer { found }),
            WritePolicy::Refuse(Refused::ReadFailed) => Err(Failure::Unreadable(match current {
                file::Loaded::Failed(error) => error,
                // The decision table issues this rejection only for read failures.
                _ => io::Error::other("state.json 을 못 읽었다"),
            })),
        }
    }

    /// Copy `state.json` aside before overwriting — the write is done even if it fails (D8 — for screen state, progress comes first).
    fn copy_state_aside(&mut self, current: &file::Loaded) {
        let what = match current {
            file::Loaded::Parsed(file::Parsed::Usable { .. }) => {
                "이 실행이 아직 쓰지 않은 동안 놓인 state.json"
            }
            _ => "못 쓰는 state.json",
        };
        match self.files.copy_aside_state() {
            Ok(to) => {
                self.aside_failures.end();
                tracing::info!(
                    module = "state",
                    to = %to.display(),
                    "{what} 을 덮기 전에 떠 뒀다"
                );
            }
            Err(error) if self.aside_failures.fail(()) => tracing::error!(
                module = "state",
                "{what} 을 떠 두지 못했다 — 그래도 쓴다: 하나뿐인 원본을 백업 없이 덮는다(D8): {error}"
            ),
            Err(error) => tracing::debug!(
                module = "state",
                failures = self.aside_failures.failures(),
                "{what} 을 또 떠 두지 못했다 — 그래도 쓴다: {error}"
            ),
        }
    }

    /// Decide when to write again. For snapshot · disk failures and a rejection where the current file could not be read, the debounce is measured again from now and it writes again
    /// after [`QUIET`] (deciding again then). A codec rejection and a newer-version rejection would be the same if retried with the same number, so treat
    /// that number as seen and wait for the next change — the newer-version file is not reread every period. For `Final` the thread
    /// ends soon and neither comes.
    // ADR-0291 (D5)
    fn failed(&mut self, cause: Cause, failure: Failure, revision: S::Revision) {
        match failure {
            Failure::Snapshot(_) | Failure::Disk(_) | Failure::Unreadable(_) => {
                let now = self.clock.now();
                self.pending_since = Some(now);
                self.last_change_at = now;
            }
            Failure::Rejected(_) | Failure::Newer { .. } => {
                self.seen = revision;
                self.pending_since = None;
            }
        }
        if !self.write_failures.fail(failure.kind()) {
            tracing::debug!(
                module = "state",
                ?cause,
                failures = self.write_failures.failures(),
                "상태 파일 쓰기가 같은 사유로 또 실패했다: {failure}"
            );
            return;
        }
        match &failure {
            Failure::Snapshot(_) => tracing::error!(
                module = "state",
                ?cause,
                "상태 스냅숏을 못 떴다 — 주기 저장이 다시 뜬다(정상 종료 쓰기면 그대로 끝난다): {failure}"
            ),
            Failure::Rejected(_) => tracing::error!(
                module = "state",
                ?cause,
                "상태 스냅숏을 다음 부팅이 못 읽을 모양이라 쓰지 않았다 — 화면이 바뀌어 풀릴 때까지 메모리에만 있다: {failure}"
            ),
            Failure::Newer { .. } => tracing::error!(
                module = "state",
                ?cause,
                "덮지 않고 쓰지 않았다 — 화면이 바뀌면 다시 판정한다: {failure}"
            ),
            Failure::Unreadable(_) => tracing::error!(
                module = "state",
                ?cause,
                "덮어도 되는지 몰라 쓰지 않았다 — 주기 저장이 다시 판정한다(정상 종료 쓰기면 그대로 끝난다): {failure}"
            ),
            Failure::Disk(_) => tracing::warn!(
                module = "state",
                ?cause,
                "상태 파일을 못 썼다 — 주기 저장이 다시 쓴다(정상 종료 쓰기면 그대로 끝난다): {failure}"
            ),
        }
    }

    /// N1 — reread the copy and delete it only if it equals the hash in the resolved slot. This keeps a stale recorder from deleting a new copy that a new
    /// instance made, in a boot that went on without the lock or in a deadline race.
    ///
    /// ★Between the read and the delete is not atomic★ — if a new copy comes in in that gap, it is deleted. What blocks it is the shell-run lock (I1) and
    /// the closed check right before deleting, and that check only narrows the path where those two are missing.
    fn drop_answered_copy(&mut self) {
        let Some(hash) = self.resolved.clone() else {
            return;
        };
        let clear = match self.files.read_crash_copy() {
            Ok(text) if codec::crash_copy_hash(&text) == hash => {
                if self.is_closed() {
                    return;
                }
                match self.files.remove_crash_copy() {
                    Ok(()) => {
                        tracing::info!(module = "state", hash = %hash, "답한 크래시 사본을 지웠다");
                        true
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        tracing::info!(module = "state", hash = %hash, "답한 크래시 사본이 이미 없다");
                        true
                    }
                    Err(error) => {
                        self.copy_failed(CopyFailure::Remove, &hash, &error);
                        false
                    }
                }
            }
            Ok(_) => {
                tracing::warn!(
                    module = "state",
                    hash = %hash,
                    "크래시 사본이 답한 것과 다르다 — 새 사본이라 지우지 않는다"
                );
                true
            }
            // The classification of read errors is a single base decision.
            Err(error) => match file::classify(Err(error), &codec::STATE_SPEC) {
                file::Loaded::Missing => {
                    tracing::info!(module = "state", hash = %hash, "답한 크래시 사본이 이미 없다");
                    true
                }
                // The answered copy was UTF-8 within the limit — a copy that is not is a different copy.
                file::Loaded::Parsed(parsed) => {
                    tracing::warn!(
                        module = "state",
                        hash = %hash,
                        parsed = ?parsed,
                        "크래시 사본이 답한 것과 다르다(못 쓰는 내용) — 지우지 않는다"
                    );
                    true
                }
                file::Loaded::Failed(error) => {
                    self.copy_failed(CopyFailure::Read, &hash, &error);
                    false
                }
            },
        };
        if clear {
            self.resolved = None;
            if let Some(failures) = self.copy_failures.end() {
                tracing::info!(
                    module = "state",
                    hash = %hash,
                    failures,
                    "답한 크래시 사본 정리가 실패 뒤 끝났다"
                );
            }
        }
    }

    fn copy_failed(&mut self, kind: CopyFailure, hash: &str, error: &io::Error) {
        if !self.copy_failures.fail(kind) {
            tracing::debug!(
                module = "state",
                hash = %hash,
                ?kind,
                failures = self.copy_failures.failures(),
                "답한 크래시 사본 정리가 같은 사유로 또 실패했다: {error}"
            );
            return;
        }
        match kind {
            CopyFailure::Remove => tracing::warn!(
                module = "state",
                hash = %hash,
                "답한 크래시 사본을 못 지웠다 — 다음 성공 쓰기 뒤 다시 지운다: {error}"
            ),
            CopyFailure::Read => tracing::warn!(
                module = "state",
                hash = %hash,
                "크래시 사본을 못 읽어 답한 사본인지 모른다 — 다음 성공 쓰기 뒤 다시 본다: {error}"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::schema::{Bounds, TabStrip, WindowKind};
    use std::sync::atomic::{AtomicU64, AtomicUsize};
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    const COPY: &str = "{\"version\":1}\n";

    #[derive(Clone, Default)]
    struct FakeSource {
        revision: Arc<AtomicU64>,
        oversized: Arc<AtomicBool>,
        fail_snapshot: Arc<AtomicBool>,
        snapshots: Arc<AtomicUsize>,
    }

    impl FakeSource {
        fn change(&self) {
            self.revision.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl SnapshotSource for FakeSource {
        type Revision = u64;

        fn revision(&self) -> u64 {
            self.revision.load(Ordering::SeqCst)
        }

        /// Carry a number in the window id so the test can read which moment's snapshot it is.
        fn snapshot(&self) -> Result<Vec<WindowEntry>, String> {
            self.snapshots.fetch_add(1, Ordering::SeqCst);
            if self.fail_snapshot.load(Ordering::SeqCst) {
                return Err("독 든 락".into());
            }
            let id = if self.oversized.load(Ordering::SeqCst) {
                "x".repeat(STATE_READ_CAP as usize + 1)
            } else {
                format!("rev-{}", self.revision())
            };
            Ok(vec![WindowEntry {
                id,
                number: Some(2),
                kind: WindowKind::Popout(TabStrip {
                    active_tab: uuid::Uuid::nil(),
                    tabs: Vec::new(),
                }),
                theme: None,
                bounds: Some(Bounds {
                    x: 0.0,
                    y: 0.0,
                    w: 800.0,
                    h: 600.0,
                }),
                maximized: false,
            }])
        }
    }

    #[derive(Default)]
    struct Disk {
        published: Vec<StateFile>,
        write_calls: usize,
        fail_writes: bool,
        /// The current `state.json` original text — if published it becomes that text. `None` = absent.
        state: Option<String>,
        /// If set, `read_state` fails with this kind.
        fail_state_read: Option<io::ErrorKind>,
        state_reads: usize,
        /// The original text of the copied-aside `state.json.corrupt` — there is only one, so it is overwritten each time it is copied.
        state_aside: Option<String>,
        aside_calls: usize,
        fail_aside: bool,
        crash_copy: Option<String>,
        fail_read: Option<io::ErrorKind>,
        fail_remove: bool,
        remove_calls: usize,
        /// Set right after publishing — imitates a race where closed was set after passing rename.
        close_after_publish: Option<Arc<AtomicBool>>,
    }

    struct Gate {
        entered: Sender<()>,
        release: Receiver<()>,
    }

    #[derive(Clone, Default)]
    struct FakeFiles {
        disk: Arc<Mutex<Disk>>,
        gate: Arc<Mutex<Option<Gate>>>,
    }

    impl FakeFiles {
        fn with_copy(text: &str) -> Self {
            let files = Self::default();
            files.disk().crash_copy = Some(text.to_owned());
            files
        }

        fn disk(&self) -> std::sync::MutexGuard<'_, Disk> {
            self.disk.lock().unwrap()
        }

        /// The counts of (`state.json` reads, `write_state` calls, copy-aside calls).
        fn counts(&self) -> (usize, usize, usize) {
            let disk = self.disk();
            (disk.state_reads, disk.write_calls, disk.aside_calls)
        }

        /// Stop the next `write_state` before rename — see from the returned receiver that it has stepped in, and release it with the sender.
        fn arm_gate(&self) -> (Receiver<()>, Sender<()>) {
            let (entered, entered_rx) = mpsc::channel();
            let (release_tx, release) = mpsc::channel();
            *self.gate.lock().unwrap() = Some(Gate { entered, release });
            (entered_rx, release_tx)
        }
    }

    impl StateFiles for FakeFiles {
        fn write_state(&self, text: &str, skip: &dyn Fn() -> bool) -> io::Result<WriteOutcome> {
            let gate = self.gate.lock().unwrap().take();
            if let Some(gate) = gate {
                gate.entered.send(()).unwrap();
                let _ = gate.release.recv();
            }
            let mut disk = self.disk();
            disk.write_calls += 1;
            if disk.fail_writes {
                return Err(io::Error::other("디스크 가득"));
            }
            if skip() {
                return Ok(WriteOutcome::Skipped);
            }
            let (state, _) = codec::decode(text).expect("기록기가 쓴 글은 다시 읽힌다");
            disk.published.push(state);
            disk.state = Some(text.to_owned());
            if let Some(closed) = &disk.close_after_publish {
                closed.store(true, Ordering::SeqCst);
            }
            Ok(WriteOutcome::Written)
        }

        fn read_state(&self) -> io::Result<String> {
            let mut disk = self.disk();
            disk.state_reads += 1;
            if let Some(kind) = disk.fail_state_read {
                return Err(io::Error::from(kind));
            }
            disk.state
                .clone()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }

        fn copy_aside_state(&self) -> io::Result<PathBuf> {
            let mut disk = self.disk();
            disk.aside_calls += 1;
            if disk.fail_aside {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            let Some(text) = disk.state.clone() else {
                return Err(io::Error::from(io::ErrorKind::NotFound));
            };
            disk.state_aside = Some(text);
            Ok(PathBuf::from("state.json.corrupt"))
        }

        fn read_crash_copy(&self) -> io::Result<String> {
            let disk = self.disk();
            if let Some(kind) = disk.fail_read {
                return Err(io::Error::from(kind));
            }
            disk.crash_copy
                .clone()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }

        fn remove_crash_copy(&self) -> io::Result<()> {
            let mut disk = self.disk();
            disk.remove_calls += 1;
            if disk.fail_remove {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            disk.crash_copy
                .take()
                .map(drop)
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
    }

    #[derive(Clone)]
    struct FakeClock {
        base: Instant,
        elapsed: Arc<Mutex<Duration>>,
    }

    impl FakeClock {
        fn new() -> Self {
            Self {
                base: Instant::now(),
                elapsed: Arc::default(),
            }
        }

        fn advance(&self, by: Duration) {
            *self.elapsed.lock().unwrap() += by;
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.base + *self.elapsed.lock().unwrap()
        }
    }

    impl SaverClock for FakeClock {
        fn wall_ms(&self) -> u64 {
            1_759_400_000_000 + self.elapsed.lock().unwrap().as_millis() as u64
        }
    }

    struct Rig {
        saver: Saver<FakeSource, FakeFiles, FakeClock>,
        source: FakeSource,
        files: FakeFiles,
        clock: FakeClock,
    }

    fn rig(files: FakeFiles, carry_resolved: Option<String>) -> Rig {
        rig_claiming(files, carry_resolved, Claim::Adopted)
    }

    fn rig_claiming(files: FakeFiles, carry_resolved: Option<String>, claim: Claim) -> Rig {
        let source = FakeSource::default();
        let clock = FakeClock::new();
        let saver = Saver::new(
            source.clone(),
            files.clone(),
            clock.clone(),
            Arc::new(AtomicBool::new(false)),
            0,
            carry_resolved,
            claim,
        );
        Rig {
            saver,
            source,
            files,
            clock,
        }
    }

    impl Rig {
        /// Check once after [`POLL_INTERVAL`] like the production thread.
        fn tick(&mut self) {
            self.clock.advance(POLL_INTERVAL);
            self.saver.tick();
        }

        fn ticks(&mut self, count: usize) {
            for _ in 0..count {
                self.tick();
            }
        }

        fn published(&self) -> Vec<StateFile> {
            self.files.disk().published.clone()
        }

        fn last(&self) -> StateFile {
            self.published().pop().expect("발행된 스냅숏이 있다")
        }

        /// Check until 1 second has passed quietly after a change — one periodic write.
        fn change_and_settle(&mut self) {
            self.source.change();
            self.ticks(3);
        }
    }

    fn hash(text: &str) -> String {
        codec::crash_copy_hash(text)
    }

    // ── Periodic save ──

    #[test]
    fn nothing_is_written_while_the_revision_stays() {
        let mut rig = rig(FakeFiles::default(), None);
        rig.ticks(40);
        assert_eq!(rig.files.disk().write_calls, 0);
    }

    #[test]
    fn a_change_is_written_after_one_quiet_second() {
        let mut rig = rig(FakeFiles::default(), None);
        rig.source.change();
        rig.tick(); // 0.5 s — sees the change
        rig.tick(); // 1.0 s — 0.5 s quiet
        assert_eq!(rig.files.disk().write_calls, 0);
        rig.tick(); // 1.5 s — 1 s quiet
        assert_eq!(rig.published().len(), 1);
        let state = rig.last();
        assert_eq!(state.windows[0].id, "rev-1");
        assert!(!state.clean_exit);
        assert_eq!(state.saved_at_ms, 1_759_400_000_000 + 1_500);

        rig.ticks(20);
        assert_eq!(
            rig.files.disk().write_calls,
            1,
            "새 변경이 없으면 다시 쓰지 않는다"
        );
    }

    #[test]
    fn continuous_changes_are_written_five_seconds_after_the_first() {
        let mut rig = rig(FakeFiles::default(), None);
        for _ in 0..10 {
            rig.source.change();
            rig.tick();
        }
        assert_eq!(rig.files.disk().write_calls, 0, "첫 변경(0.5초) 뒤 4.5초");
        rig.source.change();
        rig.tick();
        assert_eq!(rig.published().len(), 1, "첫 변경 뒤 5초");
        assert_eq!(rig.last().windows[0].id, "rev-11");
    }

    #[test]
    fn a_failed_write_is_retried_after_the_quiet_period() {
        let mut rig = rig(FakeFiles::default(), None);
        rig.files.disk().fail_writes = true;
        rig.change_and_settle();
        assert_eq!(rig.files.disk().write_calls, 1);
        rig.files.disk().fail_writes = false;
        rig.tick();
        assert_eq!(
            rig.files.disk().write_calls,
            1,
            "실패 뒤 0.5초 — 아직 조용하지 않다"
        );
        rig.tick();
        assert_eq!(rig.published().len(), 1);
        assert_eq!(rig.last().windows[0].id, "rev-1");
    }

    #[test]
    fn a_rejected_snapshot_is_never_handed_to_the_disk_and_waits_for_a_change() {
        let mut rig = rig(FakeFiles::default(), None);
        rig.source.oversized.store(true, Ordering::SeqCst);
        rig.change_and_settle();
        assert_eq!(rig.source.snapshots.load(Ordering::SeqCst), 1);
        rig.ticks(20);
        assert_eq!(
            rig.source.snapshots.load(Ordering::SeqCst),
            1,
            "같은 스냅숏은 또 거절되므로 다시 뜨지 않는다"
        );
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Failed);
        assert_eq!(rig.files.disk().write_calls, 0);

        rig.source.oversized.store(false, Ordering::SeqCst);
        rig.change_and_settle();
        assert_eq!(
            rig.last().resolved_crash_copy,
            Some(hash(COPY)),
            "거절된 답은 다음 성공 쓰기가 싣는다"
        );
    }

    #[test]
    fn the_handle_can_live_in_shared_app_state() {
        fn shareable<T: Send + Sync + 'static>() {}
        shareable::<SaverHandle>();
    }

    #[test]
    fn a_failed_snapshot_is_retried_like_a_disk_failure() {
        let mut rig = rig(FakeFiles::default(), None);
        rig.source.fail_snapshot.store(true, Ordering::SeqCst);
        rig.change_and_settle();
        assert_eq!(rig.saver.resolve("abc".into()), SaveOutcome::Failed);
        assert_eq!(rig.saver.save(Cause::Final), SaveOutcome::Failed);
        assert_eq!(rig.files.disk().write_calls, 0);

        rig.source.fail_snapshot.store(false, Ordering::SeqCst);
        rig.ticks(2);
        let state = rig.last();
        assert_eq!(state.resolved_crash_copy.as_deref(), Some("abc"));
        assert!(!state.clean_exit);
    }

    #[test]
    fn repeated_failures_fold_per_kind() {
        let mut streak = Streak::default();
        assert!(streak.fail(FailureKind::Disk));
        assert!(!streak.fail(FailureKind::Disk));
        assert!(
            streak.fail(FailureKind::Rejected),
            "갈래가 바뀌면 다시 제 레벨"
        );
        assert!(!streak.fail(FailureKind::Rejected));
        assert!(streak.fail(FailureKind::Disk));
        assert_eq!(streak.end(), Some(5));
        assert_eq!(streak.end(), None);
        assert!(
            streak.fail(FailureKind::Disk),
            "끝난 뒤 첫 실패는 다시 제 레벨"
        );
    }

    #[test]
    fn the_saver_counts_each_failure_kind_into_one_streak() {
        let mut rig = rig(FakeFiles::default(), None);
        rig.files.disk().fail_writes = true;
        rig.change_and_settle();
        rig.ticks(2);
        assert_eq!(
            rig.saver.write_failures.current,
            Some((FailureKind::Disk, 2))
        );
        rig.source.oversized.store(true, Ordering::SeqCst);
        rig.ticks(2);
        assert_eq!(
            rig.saver.write_failures.current,
            Some((FailureKind::Rejected, 3))
        );

        rig.source.oversized.store(false, Ordering::SeqCst);
        rig.files.disk().fail_writes = false;
        rig.change_and_settle();
        assert_eq!(rig.saver.write_failures.current, None);
        assert_eq!(rig.published().len(), 1);
    }

    // ── Re-decision right before saving (ADR-0291 R16) ──

    const NEWER: &str = r#"{"version":2,"saved_at_ms":1,"clean_exit":false,"windows":[]}"#;

    fn a_usable_file() -> String {
        codec::encode(&StateFile {
            version: STATE_VERSION,
            saved_at_ms: 3,
            clean_exit: true,
            resolved_crash_copy: None,
            windows: Vec::new(),
        })
        .unwrap()
    }

    /// D5: a newer-version rejection waits for the next change like a codec rejection — the file is not reread with the same number.
    #[test]
    fn a_newer_state_file_is_never_overwritten_and_is_judged_again_only_on_a_change() {
        let mut rig = rig(FakeFiles::default(), None);
        rig.files.disk().state = Some(NEWER.to_owned());
        rig.change_and_settle();
        assert_eq!(rig.files.counts(), (1, 0, 0));

        rig.ticks(20);
        assert_eq!(
            rig.files.counts(),
            (1, 0, 0),
            "같은 변경 번호로는 다시 읽지 않는다"
        );

        rig.change_and_settle();
        assert_eq!(rig.files.counts(), (2, 0, 0));
        assert_eq!(rig.saver.save(Cause::Final), SaveOutcome::Failed);
        assert_eq!(rig.files.disk().state.as_deref(), Some(NEWER));

        rig.files.disk().state = None;
        rig.change_and_settle();
        assert_eq!(
            rig.published().len(),
            1,
            "새 판 파일이 치워지면 다음 변경이 쓴다"
        );
    }

    /// D5: an unreadable rejection is decided again after the debounce like a disk error — once the lock is released it continues writing.
    #[test]
    fn an_unreadable_state_file_is_judged_again_after_the_quiet_period() {
        let mut rig = rig(FakeFiles::default(), None);
        rig.files.disk().fail_state_read = Some(io::ErrorKind::PermissionDenied);
        rig.change_and_settle();
        assert_eq!(rig.files.counts(), (1, 0, 0));
        rig.tick();
        assert_eq!(
            rig.files.counts(),
            (1, 0, 0),
            "실패 뒤 0.5초 — 아직 조용하지 않다"
        );
        rig.tick();
        assert_eq!(rig.files.counts(), (2, 0, 0), "디바운스 뒤 다시 판정한다");

        rig.files.disk().fail_state_read = None;
        rig.ticks(2);
        assert_eq!(rig.published().len(), 1);
        assert_eq!(
            rig.files.disk().aside_calls,
            0,
            "못 읽은 파일은 떠 두지 않는다"
        );
    }

    /// D1: a file whose header cannot be read is also corrupt, it merely passed the version gate — copy it aside and then write.
    #[test]
    fn a_corrupt_state_file_is_copied_aside_once_before_it_is_replaced() {
        for broken in ["{broken", r#"{"version":1}"#] {
            let mut rig = rig(FakeFiles::default(), None);
            rig.files.disk().state = Some(broken.to_owned());
            rig.change_and_settle();
            assert_eq!(
                rig.files.disk().state_aside.as_deref(),
                Some(broken),
                "{broken}"
            );
            assert_eq!(rig.published().len(), 1, "{broken}");

            rig.change_and_settle();
            assert_eq!(
                rig.files.disk().aside_calls,
                1,
                "갈아끼운 뒤엔 우리 파일이라 떠 두지 않는다 — {broken}"
            );
            assert_eq!(rig.published().len(), 2, "{broken}");
        }
    }

    /// D14: if this run has not yet claimed that file (`NotYet`), the usable file in that spot is also copied aside before overwriting.
    #[test]
    fn a_usable_file_is_copied_aside_only_while_the_run_has_not_claimed_it() {
        let placed = a_usable_file();

        let mut adopted = rig(FakeFiles::default(), None);
        adopted.files.disk().state = Some(placed.clone());
        adopted.change_and_settle();
        assert_eq!(adopted.files.counts(), (1, 1, 0), "잡은 파일은 그냥 쓴다");

        let mut rig = rig_claiming(FakeFiles::default(), None, Claim::NotYet);
        rig.files.disk().state = Some(placed.clone());
        rig.change_and_settle();
        assert_eq!(
            rig.files.disk().state_aside.as_deref(),
            Some(placed.as_str())
        );
        assert_eq!(rig.published().len(), 1);

        rig.change_and_settle();
        assert_eq!(
            rig.files.disk().aside_calls,
            1,
            "첫 쓰기 뒤엔 이 실행의 것이다"
        );
    }

    /// E5 · D8: even if copying aside fails, that write is published — for screen state, progress comes first.
    #[test]
    fn a_failed_copy_aside_still_publishes_the_write() {
        let mut rig = rig(FakeFiles::default(), None);
        {
            let mut disk = rig.files.disk();
            disk.state = Some("{broken".to_owned());
            disk.fail_aside = true;
        }
        rig.change_and_settle();
        let disk = rig.files.disk();
        assert_eq!((disk.aside_calls, disk.write_calls), (1, 1));
        assert_eq!(disk.state_aside, None);
        assert_eq!(disk.published.len(), 1, "떠 두기가 실패해도 쓴다");
    }

    /// R15: once a write succeeds, the continuation of copy-aside failures ends too — a copy-aside failure after that is the first failure of a new continuation, so
    /// it is logged at error again.
    #[test]
    fn a_successful_write_ends_the_copy_aside_failure_streak() {
        let mut rig = rig(FakeFiles::default(), None);
        {
            let mut disk = rig.files.disk();
            disk.state = Some("{broken".to_owned());
            disk.fail_aside = true;
        }
        rig.change_and_settle();
        assert_eq!(rig.published().len(), 1);
        assert_eq!(rig.saver.aside_failures.current, None, "쓰기가 성공했다");

        {
            let mut disk = rig.files.disk();
            disk.state = Some("{broken".to_owned());
            disk.fail_writes = true;
        }
        rig.change_and_settle();
        assert_eq!(rig.files.disk().aside_calls, 2);
        assert_eq!(
            rig.saver.aside_failures.current,
            Some(((), 1)),
            "앞 실패를 잇지 않는다 — 제 레벨(error)로 남을 차례였다"
        );
    }

    // ── Resolved slot ──

    #[test]
    fn resolve_writes_at_once_and_carries_the_hash() {
        let mut rig = rig(FakeFiles::default(), None);
        rig.change_and_settle();
        assert_eq!(
            rig.last().resolved_crash_copy,
            None,
            "꺼내기 전 쓰기엔 해시가 없다"
        );

        assert_eq!(rig.saver.resolve("abc".into()), SaveOutcome::Written);
        let state = rig.last();
        assert_eq!(state.resolved_crash_copy.as_deref(), Some("abc"));
        assert!(!state.clean_exit);
        assert_eq!(rig.published().len(), 2, "번호가 그대로여도 곧바로 쓴다");
    }

    #[test]
    fn the_same_copy_is_removed_and_the_slot_cleared() {
        let mut rig = rig(FakeFiles::with_copy(COPY), None);
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Written);
        assert_eq!(rig.files.disk().crash_copy, None);

        rig.change_and_settle();
        assert_eq!(
            rig.last().resolved_crash_copy,
            None,
            "지운 뒤엔 해시를 싣지 않는다"
        );
        assert_eq!(rig.files.disk().remove_calls, 1);
    }

    #[test]
    fn a_different_copy_is_kept_and_the_slot_cleared() {
        let mut rig = rig(FakeFiles::with_copy("새 인스턴스가 뜬 사본"), None);
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Written);
        assert_eq!(rig.files.disk().remove_calls, 0);
        assert_eq!(
            rig.files.disk().crash_copy.as_deref(),
            Some("새 인스턴스가 뜬 사본")
        );

        rig.change_and_settle();
        assert_eq!(rig.last().resolved_crash_copy, None);
        assert_eq!(rig.files.disk().remove_calls, 0);
    }

    #[test]
    fn a_missing_copy_clears_the_slot() {
        let mut rig = rig(FakeFiles::default(), None);
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Written);
        rig.change_and_settle();
        assert_eq!(rig.last().resolved_crash_copy, None);
        assert_eq!(rig.files.disk().remove_calls, 0);
    }

    #[test]
    fn an_unusable_copy_is_not_the_answered_one() {
        let mut rig = rig(FakeFiles::with_copy(COPY), None);
        rig.files.disk().fail_read = Some(io::ErrorKind::InvalidData);
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Written);
        rig.files.disk().fail_read = None;
        rig.change_and_settle();
        assert_eq!(rig.last().resolved_crash_copy, None);
        assert_eq!(rig.files.disk().remove_calls, 0);
    }

    #[test]
    fn a_failed_remove_keeps_the_slot_and_retries_after_each_write() {
        let mut rig = rig(FakeFiles::with_copy(COPY), None);
        rig.files.disk().fail_remove = true;
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Written);
        assert_eq!(rig.files.disk().remove_calls, 1);

        rig.ticks(20);
        assert_eq!(
            rig.files.disk().remove_calls,
            1,
            "쓰기 없이는 다시 지우지 않는다"
        );

        rig.change_and_settle();
        assert_eq!(
            rig.last().resolved_crash_copy,
            Some(hash(COPY)),
            "못 지운 동안은 해시를 싣는다"
        );
        assert_eq!(rig.files.disk().remove_calls, 2);

        rig.files.disk().fail_remove = false;
        rig.change_and_settle();
        assert_eq!(rig.files.disk().remove_calls, 3);
        assert_eq!(rig.files.disk().crash_copy, None);

        rig.change_and_settle();
        assert_eq!(rig.last().resolved_crash_copy, None);
        assert_eq!(rig.files.disk().remove_calls, 3);
    }

    #[test]
    fn an_unreadable_copy_keeps_the_slot_until_a_later_write() {
        let mut rig = rig(FakeFiles::with_copy(COPY), None);
        rig.files.disk().fail_read = Some(io::ErrorKind::PermissionDenied);
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Written);
        assert_eq!(rig.files.disk().remove_calls, 0);

        rig.files.disk().fail_read = None;
        rig.change_and_settle();
        assert_eq!(rig.last().resolved_crash_copy, Some(hash(COPY)));
        assert_eq!(rig.files.disk().crash_copy, None);
    }

    #[test]
    fn a_failed_resolve_write_keeps_the_slot_for_the_next_write() {
        let mut rig = rig(FakeFiles::with_copy(COPY), None);
        rig.files.disk().fail_writes = true;
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Failed);
        assert_eq!(rig.files.disk().remove_calls, 0);

        rig.files.disk().fail_writes = false;
        rig.ticks(2);
        assert_eq!(
            rig.published().len(),
            1,
            "번호가 그대로여도 실패한 답을 다시 쓴다"
        );
        assert_eq!(rig.last().resolved_crash_copy, Some(hash(COPY)));
        assert_eq!(rig.files.disk().crash_copy, None);
    }

    #[test]
    fn a_hash_carried_from_boot_is_written_and_its_copy_removed() {
        let mut rig = rig(FakeFiles::with_copy(COPY), Some(hash(COPY)));
        rig.change_and_settle();
        assert_eq!(rig.last().resolved_crash_copy, Some(hash(COPY)));
        assert_eq!(rig.files.disk().crash_copy, None);
    }

    // ── Clean exit · closed ──

    #[test]
    fn final_writes_a_clean_exit_and_leaves_an_unanswered_copy() {
        let mut rig = rig(FakeFiles::with_copy(COPY), None);
        assert_eq!(rig.saver.save(Cause::Final), SaveOutcome::Written);
        let state = rig.last();
        assert!(state.clean_exit);
        assert_eq!(state.resolved_crash_copy, None);
        assert_eq!(
            rig.files.disk().remove_calls,
            0,
            "D6 — 정상 종료는 사본을 지우지 않는다"
        );
        assert_eq!(rig.files.disk().crash_copy.as_deref(), Some(COPY));
    }

    #[test]
    fn final_carries_the_slot_and_retries_the_remove() {
        let mut rig = rig(FakeFiles::with_copy(COPY), Some(hash(COPY)));
        assert_eq!(rig.saver.save(Cause::Final), SaveOutcome::Written);
        let state = rig.last();
        assert!(state.clean_exit);
        assert_eq!(state.resolved_crash_copy, Some(hash(COPY)));
        assert_eq!(rig.files.disk().crash_copy, None);
    }

    #[test]
    fn once_closed_nothing_touches_the_disk() {
        let mut rig = rig(FakeFiles::with_copy(COPY), None);
        rig.saver.closed.store(true, Ordering::SeqCst);
        rig.change_and_settle();
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Skipped);
        assert_eq!(rig.saver.save(Cause::Final), SaveOutcome::Skipped);
        let disk = rig.files.disk();
        assert_eq!((disk.write_calls, disk.remove_calls), (0, 0));
    }

    #[test]
    fn closing_between_the_write_and_the_delete_keeps_the_copy() {
        let mut rig = rig(FakeFiles::with_copy(COPY), None);
        rig.files.disk().close_after_publish = Some(Arc::clone(&rig.saver.closed));
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Written);
        let disk = rig.files.disk();
        assert_eq!(disk.remove_calls, 0);
        assert_eq!(disk.crash_copy.as_deref(), Some(COPY));
    }

    // ── Thread ──

    fn started(
        files: FakeFiles,
        carry_resolved: Option<String>,
        poll: Duration,
    ) -> (SaverHandle, JoinHandle<()>, FakeSource, FakeClock) {
        let source = FakeSource::default();
        let clock = FakeClock::new();
        let saver = Saver::new(
            source.clone(),
            files,
            clock.clone(),
            Arc::new(AtomicBool::new(false)),
            0,
            carry_resolved,
            Claim::Adopted,
        );
        let (handle, thread) = start(saver, poll).unwrap();
        (handle, thread, source, clock)
    }

    #[test]
    fn finish_writes_a_clean_exit_and_ends_the_thread() {
        let files = FakeFiles::default();
        let (handle, thread, _, _) = started(files.clone(), None, POLL_INTERVAL);
        assert_eq!(
            handle.finish(REPLY_DEADLINE),
            RequestOutcome::Done(SaveOutcome::Written)
        );
        thread.join().unwrap();
        assert!(files.disk().published.last().unwrap().clean_exit);
        assert_eq!(
            handle.resolve(hash(COPY), REPLY_DEADLINE),
            RequestOutcome::SaverGone
        );
        assert_eq!(handle.finish(REPLY_DEADLINE), RequestOutcome::SaverGone);
    }

    #[test]
    fn a_finish_past_its_deadline_closes_and_the_stuck_write_never_publishes() {
        let files = FakeFiles::with_copy(COPY);
        let (entered, release) = files.arm_gate();
        let (handle, thread, _, _) = started(files.clone(), None, POLL_INTERVAL);

        // Exit arrives while stuck before rename in writing an answer that deletes the copy.
        let stuck = handle
            .send(|reply| Request::Resolve {
                hash: hash(COPY),
                reply,
            })
            .unwrap();
        entered.recv().unwrap();
        assert_eq!(
            handle.finish(Duration::from_millis(50)),
            RequestOutcome::TimedOut
        );
        assert!(handle.closed.load(Ordering::SeqCst));
        release.send(()).unwrap();
        assert_eq!(
            wait(stuck, REPLY_DEADLINE),
            RequestOutcome::Done(SaveOutcome::Skipped)
        );
        thread.join().unwrap();

        let disk = files.disk();
        assert_eq!(
            disk.write_calls, 1,
            "닫힌 뒤 꺼낸 Final 은 디스크를 건드리지 않는다"
        );
        assert!(
            disk.published.is_empty(),
            "풀려난 rename 은 발행하지 않는다"
        );
        assert_eq!(disk.remove_calls, 0);
        assert_eq!(disk.crash_copy.as_deref(), Some(COPY));
    }

    #[test]
    fn dropping_every_handle_ends_the_thread_without_writing() {
        let files = FakeFiles::default();
        let (handle, thread, _, _) = started(files.clone(), None, POLL_INTERVAL);
        let copy = handle.clone();
        drop(handle);
        drop(copy);
        thread.join().unwrap();
        assert_eq!(files.disk().write_calls, 0);
    }

    #[test]
    fn a_resolve_still_queued_is_not_in_the_write_in_progress() {
        let files = FakeFiles::default();
        let (entered, release) = files.arm_gate();
        let (handle, thread, source, clock) =
            started(files.clone(), None, Duration::from_millis(1));

        source.change();
        let mut attempts = 0;
        while entered.recv_timeout(Duration::from_millis(20)).is_err() {
            attempts += 1;
            assert!(attempts < 500, "주기 쓰기가 시작되지 않았다");
            clock.advance(QUIET);
        }
        // Send it while a periodic write is standing before rename — it has not been taken out yet.
        let answer = handle
            .send(|reply| Request::Resolve {
                hash: "abc".into(),
                reply,
            })
            .unwrap();
        release.send(()).unwrap();
        assert_eq!(
            wait(answer, REPLY_DEADLINE),
            RequestOutcome::Done(SaveOutcome::Written)
        );

        let published = files.disk().published.clone();
        assert_eq!(published.len(), 2);
        assert_eq!(published[0].resolved_crash_copy, None);
        assert_eq!(published[1].resolved_crash_copy.as_deref(), Some("abc"));

        assert_eq!(
            handle.finish(REPLY_DEADLINE),
            RequestOutcome::Done(SaveOutcome::Written)
        );
        thread.join().unwrap();
    }

    // ── Production files ──

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "engram-saver-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("시계")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn fs_writes_reads_and_removes_the_real_files() {
        let dir = temp_dir("fs");
        let state_path = dir.join("state.json");
        let crash_path = dir.join("state.crash.json");
        let fs = Fs::new(state_path.clone(), crash_path.clone());

        assert_eq!(
            fs.write_state("첫 글", &|| false).unwrap(),
            WriteOutcome::Written
        );
        assert_eq!(
            fs.write_state("닫힌 뒤 글", &|| true).unwrap(),
            WriteOutcome::Skipped
        );
        assert_eq!(std::fs::read_to_string(&state_path).unwrap(), "첫 글");

        assert_eq!(
            fs.read_crash_copy().unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        std::fs::write(&crash_path, COPY).unwrap();
        assert_eq!(fs.read_crash_copy().unwrap(), COPY);
        fs.remove_crash_copy().unwrap();
        assert!(!crash_path.exists());
        assert_eq!(
            fs.remove_crash_copy().unwrap_err().kind(),
            io::ErrorKind::NotFound
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The two re-decision operations of the real seam + R11 (even a folder deleted during the run is recreated by a write).
    #[test]
    fn fs_reads_and_copies_aside_the_real_state_file_and_remakes_a_missing_folder() {
        let root = temp_dir("fs-state");
        let dir = root.join("state");
        let fs = Fs::new(dir.join("state.json"), dir.join("state.crash.json"));

        assert_eq!(fs.read_state().unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(
            fs.write_state("글", &|| false).unwrap(),
            WriteOutcome::Written
        );
        assert_eq!(fs.read_state().unwrap(), "글");
        let to = fs.copy_aside_state().unwrap();
        assert_eq!(to, dir.join("state.json.corrupt"));
        assert_eq!(std::fs::read_to_string(&to).unwrap(), "글");
        assert_eq!(fs.read_state().unwrap(), "글", "원본은 그 자리에 남는다");

        std::fs::remove_dir_all(&root).unwrap();
    }
}
