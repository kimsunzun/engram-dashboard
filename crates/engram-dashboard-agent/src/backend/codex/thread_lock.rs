//! Recover the codex thread id as the **owner of a lock file** — the lock file held by the child we spawned is named by that child's
//!   thread id (ADR-0218 decision 1).
//!
//! ★Only two pieces of codex knowledge live here★ — the lock directory ([`lock_dir`]) and that the file name is the id. 「Who holds this
//!   file」 and 「what is alive under this PID」 need no domain knowledge, so
//! [`engram_dashboard_platform::file_holders`] and [`engram_dashboard_platform::process::subtree`] answer them (ADR-0004 · ADR-0218
//!   decision 11 · ADR-0266). ★**The file content is never read**★ (0 bytes; reading it would tie us
//! to the vendor storage format, ADR-0203).
//!
//! Entry points: [`plan_capture`] (whether to run recovery for this spawn, and its inputs) and [`spawn_capture`] (starts the polling
//!   thread). [`scan_for_child`] carries the decision, [`CaptureClock`] the waiting and child
//! liveness; all filesystem and OS queries sit behind those and [`LockHolderProbe`], so the loop rules are measured without an OS (ADR-0012).
//!
//! ★The recovered value leaves through exactly one path, [`crate::backend::SessionIdSink`]★ — the verb the assembly point handed over,
//!   bound to the incarnation marker; this file knows neither the profile nor its schema
//! (ADR-0004 · ADR-0218 decision 6). A second write path would bypass the incarnation guard.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use engram_dashboard_platform::env::env_key_eq;
use engram_dashboard_platform::file_holders::{self, Holder};
use engram_dashboard_platform::process::{self, ProcessIdentity};
use uuid::Uuid;

use crate::backend::SessionIdSink;

/// The folder under the codex home where the writer locks live.
///
/// ★Undocumented layout★ — if upstream changes it we silently get 0 candidates instead of an error (ADR-0218 rationale, unmeasured item
///   ③); the failure mode is 「not received」, never a false detection.
const LOCK_DIR: &str = "thread-writer-locks";

const LOCK_EXTENSION: &str = "lock";

const CODEX_HOME_SUBDIR: &str = ".codex";

/// The env variable that moves the whole codex state directory — confirmed by measurement; it applies **only to the process that sets it**
/// (`docs/research/codex-session-id-recovery-survey-2026-09-19.md`, `CODEX_HOME` section).
pub(super) const CODEX_HOME_ENV: &str = "CODEX_HOME";

/// Every place where this module meets the OS — file holders · folder listing · our process tree — on one seam, so [`scan_for_child`] runs
///   without a real file or process (ADR-0012).
pub(crate) trait LockHolderProbe {
    /// **No distinction** between "could not ask" and "no holders" — both are an empty list (「not a candidate」); polling asks again next round.
    fn holders(&self, path: &Path) -> Vec<Holder>;

    /// The identities of the child we spawned and every **live** descendant under it. An empty list = that PID is no longer our child.
    ///
    /// ★Why a tree — the PID we hold is not codex★: on Windows the spawn passes through `cmd.exe /c codex …`
    ///   ([`engram_dashboard_platform::shell::console_command`]), so the returned PID is **the wrapper**; the lock holder is the
    ///   `codex.exe` below it (ADR-0218 rationale). ★Comparing only the wrapper PID **never** matches, and the symptom is silence, not an
    ///   error★ (measured 2026-09-21: wrapper 38240 · 4816,19468 below it).
    /// ★Resolve again every round — never reuse a copied list★: the shim launches codex late and codex launches more tools, so the tree grows.
    /// ★What the tree rules block, and the accepted residue★ — the source of record for the rules (root identity check · dropping entries
    ///   born before their parent) and the unfilterable cases ①②③ is the doc of [`process::subtree`].
    ///   - **The sequence the ordering rule blocks:** a hand-launched `codex.exe` becomes an orphan keeping the recorded ppid `P`, and
    ///   Windows **reuses** `P` for our wrapper; without the rule that stranger is enumerated as our child and,
    ///     matching its own lock holder, is **the only candidate for 0.7–21 s before our codex creates its lock** — not even `Ambiguous` —
    ///   and another party's thread is written into our handle (ADR-0218 impact/invariants).
    ///   - ★**Residue ①②③ is accepted (do not argue again)**★: ① PID reuse between enumeration and start-time reading has no atomic fix
    ///   (microsecond window); ② blocking the same tick would lose a normal child; ③ a **real false detection**
    ///     also needs the stranger to hold a `<uuid>.lock` under our `CODEX_HOME`.
    ///   - A tree enumeration that fails and returns less is not an error — it cannot be handled differently from 「not a candidate」; the
    ///   next round asks again.
    fn our_processes(&self, root_pid: u32, root_start_time: u64) -> Vec<ProcessIdentity> {
        process::subtree(root_pid, root_start_time)
    }

    /// All **regular file** paths directly under `dir` (the name decision belongs to [`scan_for_child`]). Unreadable → empty list (a
    ///   directory that does not exist yet is normal).
    ///
    /// ★Directories and reparse points (symlink·junction) are not candidates★ — a folder named `<uuid>.lock` is not a lock, and passing a
    ///   reparse point to the holder query opens a target we did not choose. `file_type()` does not
    ///   follow links, so `is_file()` filters both; entries whose type cannot be read are left out too.
    fn entries(&self, dir: &Path) -> Vec<PathBuf> {
        let Ok(read) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        read.flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.path())
            .collect()
    }
}

/// Production implementation — asks [`engram_dashboard_platform::file_holders`]; a failed query folds to an empty list (contract of
///   [`LockHolderProbe::holders`]).
pub(crate) struct RestartManagerProbe;

impl LockHolderProbe for RestartManagerProbe {
    fn holders(&self, path: &Path) -> Vec<Holder> {
        match file_holders::holders_of(path) {
            Ok(holders) => holders,
            Err(err) => {
                tracing::debug!(path = %path.display(), %err, "lock file holder lookup failed");
                Vec::new()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScanOutcome {
    /// Exactly one lock held by a process in our tree — the thread id of that file name.
    Found(Uuid),
    /// 0 candidates. No lock yet, or this child does not create a lock — keep waiting (ADR-0218 decision 4).
    NotYet,
    /// n (≥2) candidates — ★adopt nothing this round★: whether one codex process can hold two or more locks is unmeasured, so we refuse
    ///   rather than guess (ADR-0218 decision 3). ★「Refusal」 is not 「giving up」 — polling continues★
    /// (not in decision 4's stop list).
    Ambiguous(usize),
}

/// Within `lock_dir`, find the lock file **held by a process in our tree** and return the thread id of its name.
///
/// Decision rules (all ADR-0218):
/// - **The comparison target is the whole tree rooted at `child_pid`**, not `child_pid` alone ([`LockHolderProbe::our_processes`] has the
///   reason).
/// - Candidate = a file named `<uuid>.lock` with a holder matching a tree entry on **both PID and start time**. ★One alone is not a
///   candidate★ — PIDs are reused (decision 2); descendants' start times are read where they are enumerated.
/// - ★**We count 「matched holders」, not files**★ — decision 3's 「exactly one candidate」 is the number of **pairs**; two of our processes
///   holding one file leaves 「who owns that lock」 unnarrowed, and counting files would let it pass as `Found`.
///   ★The cost, chosen knowingly★: if a child inherits the handle (a tool launched by codex), adoption waits for that span; tolerable
///   because [`run_capture`] does **not stop** on `Ambiguous`. Whether inheritance happens is unmeasured; the symptom would be
///   「recovery gets late」, not a false detection.
///   ★**Dissent exists — read before raising again**★: the counterargument 「if both are **ours**, the lock is ours, so adopt」 was outside
///   that round's scope, so the shape is **a hold, not a decision**. To change it, first measure inheritance and reopen
///   ADR-0218 decision 3 — do not flip it silently.
/// - One matched holder → [`ScanOutcome::Found`], 0 → [`ScanOutcome::NotYet`], two or more → [`ScanOutcome::Ambiguous`] (decision 3).
/// - `child_pid` or `child_start_time` 0 → `NotYet` **without asking** (0 = 「unknown」; counting it as a match leaves a PID-only
///   comparison, forbidden by decision 2).
/// - Empty tree (child died or PID reused) → `NotYet` without asking about a single file — the holder query opens one Restart Manager
///   session per candidate file per round, an expensive question.
///
/// Many processes may have the same file open — if one is inside our tree it is a candidate.
pub(crate) fn scan_for_child(
    lock_dir: &Path,
    child_pid: u32,
    child_start_time: u64,
    probe: &dyn LockHolderProbe,
) -> ScanOutcome {
    if child_pid == 0 || child_start_time == 0 {
        return ScanOutcome::NotYet;
    }
    let ours = probe.our_processes(child_pid, child_start_time);
    if ours.is_empty() {
        return ScanOutcome::NotYet;
    }

    let mut matches: usize = 0;
    let mut first: Option<Uuid> = None;
    for path in probe.entries(lock_dir) {
        let Some(id) = thread_id_from_lock_name(&path) else {
            continue;
        };
        let held_by_us = probe
            .holders(&path)
            .iter()
            .filter(|holder| {
                ours.iter()
                    .any(|mine| mine.pid == holder.pid && mine.start_time == holder.start_time)
            })
            .count();
        if held_by_us > 0 {
            matches += held_by_us;
            first.get_or_insert(id);
        }
    }

    match (matches, first) {
        (1, Some(id)) => ScanOutcome::Found(id),
        (n, Some(_)) if n > 1 => ScanOutcome::Ambiguous(n),
        _ => ScanOutcome::NotYet,
    }
}

/// The lock directory as seen from **our process's** environment; `None` if the home cannot be found.
///
/// ★Does not answer for a spawn carrying its own [`CODEX_HOME_ENV`]★ (profile env) — pass that home to [`lock_dir_under`].
pub(crate) fn lock_dir() -> Option<PathBuf> {
    Some(lock_dir_under(&codex_home()?))
}

pub(crate) fn lock_dir_under(codex_home: &Path) -> PathBuf {
    codex_home.join(LOCK_DIR)
}

/// Lock file name → thread id. `None` if it is not `<uuid>.lock` — **skipped**, not an error (measured: non-id files like
///   `.coordination.lock` live in the same folder).
///
/// ★Accept only the canonical form (36 chars with hyphens) — not bare `Uuid::parse_str`★: it also accepts 32 chars without hyphens, braces
///   and `urn:uuid:`, so names codex never creates become candidates; one extra candidate flips
///   `Found` to `Ambiguous` and **stops recovery entirely**. Case is not distinguished (Windows file names).
fn thread_id_from_lock_name(path: &Path) -> Option<Uuid> {
    if !path.extension()?.eq_ignore_ascii_case(LOCK_EXTENSION) {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    let id = Uuid::parse_str(stem).ok()?;
    (id.hyphenated().to_string() == stem.to_ascii_lowercase()).then_some(id)
}

fn codex_home() -> Option<PathBuf> {
    codex_home_from(
        std::env::var_os(CODEX_HOME_ENV),
        engram_dashboard_platform::env::home_dir(),
    )
}

fn codex_home_from(override_var: Option<OsString>, user_home: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(dir) = override_var {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    user_home.map(|home| home.join(CODEX_HOME_SUBDIR))
}

/// The wait until the first scan. ★Non-zero is intended★ — the measured minimum spawn→lock-creation delay is 0.735 s (ADR-0218 rationale),
///   so scanning at launch buys one certain empty round.
const FIRST_DELAY: Duration = Duration::from_millis(500);

/// The cap when the wait is doubled after each empty-handed round.
///
/// ★Why backoff★: a round is not free — [`scan_for_child`] opens one Restart Manager session per candidate file (a real user's folder had
///   8). A fixed short interval under a child that never creates a lock (layout changed, or `codex exec`)
///   burns several RM sessions per second for the agent's lifetime.
/// ★Still no upper **time** limit (ADR-0218 decision 4)★ — the decision is ownership, not time, so catching late adds no false detections.
/// ★Dense early rounds★: measured creation delays 0.735 · 0.879 · 0.907 · 1.891 s plus one unreproduced 21 s case; doubling from 0.5 s
///   puts the sixth round near 30 s, covering that tail.
const MAX_DELAY: Duration = Duration::from_secs(15);

/// The other two places where the polling loop meets the OS — whether the child is still alive, and the wait until the next round.
///
/// ★Split from [`LockHolderProbe`] because the axis differs★: that one is 「what is visible」, this one 「when to look again and whether a
///   reason remains」; merging would force faking the clock where only the list needs to change.
pub(crate) trait CaptureClock {
    /// `false` = that PID is no longer **our child** (died or PID reused) — the loop's normal exit, not an error.
    fn child_alive(&self) -> bool;

    /// Wait until the next round. ★Called holding no lock★ — the loop spends most of its time here.
    fn sleep(&self, delay: Duration);
}

struct ChildClock {
    pid: u32,
    start_time: u64,
}

impl CaptureClock for ChildClock {
    fn child_alive(&self) -> bool {
        engram_dashboard_platform::process::pid_alive_with_start_time(self.pid, self.start_time)
    }

    fn sleep(&self, delay: Duration) {
        std::thread::sleep(delay);
    }
}

/// One set of inputs for running recovery in this spawn. ★Its existence means 「all conditions to run are met」★ — [`plan_capture`] returns
///   `None` otherwise, so [`run_capture`] holds no conditional.
pub(crate) struct CapturePlan {
    pub(crate) lock_dir: PathBuf,
    pub(crate) child_pid: u32,
    pub(crate) child_start_time: u64,
    /// The record verb the assembly point handed over, bound to the incarnation marker — the **only** path the recovered value leaves through.
    ///
    /// ★What this `Arc` keeps alive★: the inner closure holds an `Arc<ProfileRegistry>`, but that registry is owned by `AgentManager` for
    ///   the whole daemon lifetime, so this clone extends nothing; the thread's own lifetime ends when the
    ///   child dies ([`run_capture`]). ★Not a `Weak`★: nobody holds the strong side — the assembly point passes the verb to `open_spawn`
    ///   and lets go; making the session hold it would change the `AgentSession` ownership split.
    pub(crate) sink: SessionIdSink,
}

/// Should this spawn run lock recovery — if so, its inputs, otherwise `None`.
///
/// Conditions (any missing → `None`):
/// - **This spawn does not emit resume argv** (`resumes_by_argv == false`). ★The question is not 「is there a stored handle」 but 「did it
///   actually go out as argv」★ — a handle whose value `resume_argument` refuses makes codex come up as a
///   **new conversation** and nobody would write that incarnation's id (the old form deciding on 「there is one」 was that hole). Both
///   decisions look at the **same predicate** (ADR-0218 decision 5; source of record `super::resume_argument`).
///   ★Fresh with a handle coming along is blocked by the assembly point★ (`manager::resume_handle_for`), so this entry holds without
///   taking the mode.
/// - There is somewhere to record (`sink`).
/// - Both the child's PID and **start time** are known — otherwise the decision sinks to PID-only, forbidden by ADR-0218 decision 2.
/// - The lock folder is known.
///
/// ★Resolve the lock folder from the **child's** environment — not ours★: `CODEX_HOME` applies only to the process that sets it, so a
///   profile-set value means the child's lock **never** appears in our folder — not an error but 「still empty-handed」, no
/// on-screen signal. `our_lock_dir` = the value when the child inherited our environment ([`lock_dir`]); an argument so the decision is
///   measured without env.
// ADR-0218
pub(crate) fn plan_capture(
    child_env: &[(String, String)],
    resumes_by_argv: bool,
    child_pid: Option<u32>,
    child_start_time: Option<u64>,
    sink: Option<SessionIdSink>,
    our_lock_dir: Option<PathBuf>,
) -> Option<CapturePlan> {
    if resumes_by_argv {
        return None;
    }
    Some(CapturePlan {
        lock_dir: child_lock_dir(child_env, our_lock_dir)?,
        child_pid: child_pid?,
        child_start_time: child_start_time?,
        sink: sink?,
    })
}

/// Only the folder rule of [`plan_capture`] — a spawn env `CODEX_HOME` wins, otherwise ours.
///
/// ★Pick the **last** entry★ — the env is a list and the same key can be loaded twice; the path pushes it into `cmd.env` in order, so the
///   child receives the later one.
/// ★Key comparison follows the OS rule ([`env_key_eq`])★ — Windows is case-insensitive. ★An empty value counts as 「not set」★
///   ([`codex_home_from`] reads our env the same way).
/// ★Known hole — relative paths★: resolved here against **our** cwd but by the child against its own, so they can diverge; the symptom is
///   「still empty-handed」, not a false detection (the PID + start time still apply). Whether such a profile
/// exists is unconfirmed, so it is documented, not fixed.
fn child_lock_dir(
    child_env: &[(String, String)],
    our_lock_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    let overridden = child_env
        .iter()
        .rev()
        .find(|(key, _)| env_key_eq(key, CODEX_HOME_ENV))
        .map(|(_, value)| value.as_str())
        .filter(|value| !value.is_empty());
    match overridden {
        Some(home) => Some(lock_dir_under(Path::new(home))),
        None => our_lock_dir,
    }
}

/// Start the recovery polling on a **separate thread** — the spawn proceeds even if it fails.
///
/// ★Blocks neither the spawn path nor the pump★: the thread only sleeps and queries the OS, holds none of the session's locks, and holds
///   only one record verb (not the session), so the agent is not reaped late because of it.
/// ★A thread that cannot start leaves only a warning★ — the failure mode is 「cannot receive the id」, not a reason to cut the spawn (as
///   ADR-0218 decision 10).
pub(crate) fn spawn_capture(plan: CapturePlan) {
    let clock = ChildClock {
        pid: plan.child_pid,
        start_time: plan.child_start_time,
    };
    tracing::debug!(
        pid = plan.child_pid,
        lock_dir = %plan.lock_dir.display(),
        "starting codex session id recovery polling"
    );
    let spawned = std::thread::Builder::new()
        .name("codex-thread-lock".to_string())
        .spawn(move || run_capture(&plan, &RestartManagerProbe, &clock));
    if let Err(err) = spawned {
        tracing::warn!(%err, "could not start the codex session id recovery thread — this incarnation does not receive an id");
    }
}

/// Poll until caught — ★three stop conditions, no time limit★ (ADR-0218 decision 4).
///
/// 1. [`ScanOutcome::Found`] — adopt and stop. 2. The child died ([`CaptureClock::child_alive`]). 3. The session goes down — ★no separate
///   signal, intentionally★: shutdown starts with the path killing the child (「kill causality」), so it arrives as item 2;
///    a second signal would put this observer into the shutdown order.
///
/// ★[`ScanOutcome::Ambiguous`] is not a stop — 「not adopted this round」★ (decision 3 says **refusal**, not in decision 4's stops): the
///   overlap may be momentary, and ending here would lose recovery **forever**. The warning is emitted on the first round only.
///
/// Adoption goes out only through [`CapturePlan::sink`] — even if that verb discards the value (a dead incarnation's record), we end there
///   without another path.
///
/// ★The window between kill and record is left open — conclusion after re-checking★: if the child dies after the liveness check, this
///   round's record still goes out. **That is correct** — it is the thread id that just-dead incarnation used, and the next
///   activation should resume that conversation. The only harmful case is **the next incarnation already being up**, where the markers
///   differ and the assembly point's incarnation guard refuses. ★Why that 「refuses」 has no gap (grounds, so nobody re-derives it)★:
///     1. The only place that **raises** the marker is `ProfileRegistry::epoch_for_spawn` (`merge_preserving_live` only preserves).
///     2. `AgentManager::spawn_agent` calls it once per spawn, **before** `release_session_id` clears the handle in Fresh.
///   So there is no span with 「marker unchanged but handle cleared」; a late record with the old marker arrives before the clearing, and
///   after the clearing the marker has changed. A deleted profile fails the lookup in `commit_session_id` and drops out there.
///   ★「The record goes out」 = **handed** to this incarnation's first-submission latch (ADR-0226)★ — written to the registry only if there
///   was a submission; a dead incarnation without one writes nothing (no conversation to resume).
///   Hence no new cancellation signal: it is an **intended outcome**, not a hole.
///
/// ★Once caught, later thread changes are not followed — known, unconfirmed★: if the user opens a new thread mid-conversation codex will
///   take over the lock anew (unmeasured) but this loop has stopped, so the profile keeps the first thread id and Resume
///   opens that first conversation. 「Stop immediately once caught」 is ADR-0218 decision 4; not flipped here.
fn run_capture(plan: &CapturePlan, probe: &dyn LockHolderProbe, clock: &dyn CaptureClock) {
    let mut delay = FIRST_DELAY;
    let mut warned_ambiguous = false;
    loop {
        clock.sleep(delay);
        if !clock.child_alive() {
            tracing::debug!(
                pid = plan.child_pid,
                "the codex child ended first, folding session id recovery"
            );
            return;
        }
        match scan_for_child(&plan.lock_dir, plan.child_pid, plan.child_start_time, probe) {
            ScanOutcome::Found(id) => {
                (plan.sink)(&id.to_string());
                return;
            }
            ScanOutcome::Ambiguous(n) => {
                if !warned_ambiguous {
                    warned_ambiguous = true;
                    tracing::warn!(
                        pid = plan.child_pid,
                        candidates = n,
                        lock_dir = %plan.lock_dir.display(),
                        "the codex tree holds two or more locks, so the session id is not adopted this round"
                    );
                }
                delay = next_delay(delay);
            }
            ScanOutcome::NotYet => delay = next_delay(delay),
        }
    }
}

fn next_delay(previous: Duration) -> Duration {
    (previous * 2).min(MAX_DELAY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // ★The fake tree is **the production shape — flat would make the test a lie**★: the PID we hold is the `cmd.exe` wrapper
    //   (`engram_dashboard_platform::shell::console_command`) and the lock holder is the `codex.exe` below it. Planting the holder
    //   with the wrapper PID passes even a 「compare only the wrapper」 implementation whose recovery never happens (it did once, 2026-09-21).
    const WRAPPER_PID: u32 = 4242;
    const WRAPPER_START: u64 = 134_344_468_098_866_343;
    const CODEX_PID: u32 = 4816;
    const CODEX_START: u64 = 134_344_468_099_111_111;
    const TOOL_PID: u32 = 19_468;
    const TOOL_START: u64 = 134_344_468_099_222_222;

    const ID_A: &str = "01a0c2c4-97f5-75e2-8ea0-4855b7a0997d";
    const ID_B: &str = "01a0c2c3-4f4c-7ee2-a858-3db9f72c2e78";

    struct FakeProbe {
        entries: Vec<PathBuf>,
        holders: HashMap<PathBuf, Vec<Holder>>,
        tree: Vec<ProcessIdentity>,
    }

    impl Default for FakeProbe {
        fn default() -> Self {
            Self {
                entries: Vec::new(),
                holders: HashMap::new(),
                tree: vec![wrapper_id(), codex_id()],
            }
        }
    }

    impl FakeProbe {
        fn file(mut self, name: &str, holders: &[Holder]) -> Self {
            let path = PathBuf::from("Z:/locks").join(name);
            self.entries.push(path.clone());
            self.holders.insert(path, holders.to_vec());
            self
        }
        fn with_tool(mut self) -> Self {
            self.tree.push(tool_id());
            self
        }
        fn with_no_tree(mut self) -> Self {
            self.tree.clear();
            self
        }
    }

    impl LockHolderProbe for FakeProbe {
        fn holders(&self, path: &Path) -> Vec<Holder> {
            self.holders.get(path).cloned().unwrap_or_default()
        }
        fn our_processes(&self, root_pid: u32, root_start_time: u64) -> Vec<ProcessIdentity> {
            if root_pid != WRAPPER_PID || root_start_time != WRAPPER_START {
                return Vec::new();
            }
            self.tree.clone()
        }
        fn entries(&self, _dir: &Path) -> Vec<PathBuf> {
            self.entries.clone()
        }
    }

    fn wrapper_id() -> ProcessIdentity {
        ProcessIdentity {
            pid: WRAPPER_PID,
            start_time: WRAPPER_START,
        }
    }

    fn codex_id() -> ProcessIdentity {
        ProcessIdentity {
            pid: CODEX_PID,
            start_time: CODEX_START,
        }
    }

    fn tool_id() -> ProcessIdentity {
        ProcessIdentity {
            pid: TOOL_PID,
            start_time: TOOL_START,
        }
    }

    fn codex() -> Holder {
        Holder {
            pid: CODEX_PID,
            start_time: CODEX_START,
        }
    }

    fn tool() -> Holder {
        Holder {
            pid: TOOL_PID,
            start_time: TOOL_START,
        }
    }

    fn scan(probe: &FakeProbe) -> ScanOutcome {
        scan_for_child(Path::new("Z:/locks"), WRAPPER_PID, WRAPPER_START, probe)
    }

    fn found(id: &str) -> ScanOutcome {
        ScanOutcome::Found(Uuid::parse_str(id).unwrap())
    }

    // ── Adoption ────────────────────────────────────────────────────────────────────

    /// ★Guards 「do not compare only the wrapper PID」★: the asked PID is the wrapper but the holder is its child; without widening to the
    ///   tree this gives `NotYet`.
    #[test]
    fn a_lock_held_by_our_childs_child_is_found() {
        let probe = FakeProbe::default().file(&format!("{ID_A}.lock"), &[codex()]);
        assert_eq!(scan(&probe), found(ID_A));
    }

    #[test]
    fn a_lock_held_by_a_deeper_descendant_is_found() {
        let probe = FakeProbe::default()
            .with_tool()
            .file(&format!("{ID_A}.lock"), &[tool()]);
        assert_eq!(scan(&probe), found(ID_A));
    }

    #[test]
    fn a_holder_outside_the_reported_tree_is_not_a_candidate() {
        let probe = FakeProbe::default().file(&format!("{ID_A}.lock"), &[tool()]);
        assert_eq!(scan(&probe), ScanOutcome::NotYet);
    }

    #[test]
    fn an_empty_tree_short_circuits_to_not_yet() {
        let probe = FakeProbe::default()
            .with_no_tree()
            .file(&format!("{ID_A}.lock"), &[codex()]);
        assert_eq!(scan(&probe), ScanOutcome::NotYet);
    }

    #[test]
    fn a_root_whose_start_time_drifted_owns_no_tree() {
        let probe = FakeProbe::default().file(&format!("{ID_A}.lock"), &[codex()]);
        assert_eq!(
            scan_for_child(
                Path::new("Z:/locks"),
                WRAPPER_PID,
                WRAPPER_START - 1,
                &probe
            ),
            ScanOutcome::NotYet
        );
    }

    #[test]
    fn other_holders_on_the_same_file_do_not_block_the_match() {
        let holders = [
            Holder {
                pid: 7,
                start_time: 9,
            },
            codex(),
            Holder {
                pid: 8,
                start_time: 10,
            },
        ];
        let probe = FakeProbe::default().file(&format!("{ID_A}.lock"), &holders);
        assert_eq!(scan(&probe), found(ID_A));
    }

    #[test]
    fn locks_held_by_other_processes_are_ignored() {
        let probe = FakeProbe::default()
            .file(
                &format!("{ID_B}.lock"),
                &[Holder {
                    pid: 999,
                    start_time: 1,
                }],
            )
            .file(&format!("{ID_A}.lock"), &[codex()]);
        assert_eq!(scan(&probe), found(ID_A));
    }

    // ── Both fields must match **together** (ADR-0218 decision 2) ───────────────────

    #[test]
    fn same_pid_with_a_different_start_time_is_not_a_match() {
        let stale = Holder {
            pid: CODEX_PID,
            start_time: CODEX_START - 1,
        };
        let probe = FakeProbe::default().file(&format!("{ID_A}.lock"), &[stale]);
        assert_eq!(scan(&probe), ScanOutcome::NotYet);
    }

    #[test]
    fn same_start_time_with_a_different_pid_is_not_a_match() {
        let other = Holder {
            pid: CODEX_PID + 1,
            start_time: CODEX_START,
        };
        let probe = FakeProbe::default().file(&format!("{ID_A}.lock"), &[other]);
        assert_eq!(scan(&probe), ScanOutcome::NotYet);
    }

    #[test]
    fn an_unknown_child_start_time_never_matches() {
        let probe = FakeProbe::default().file(
            &format!("{ID_A}.lock"),
            &[Holder {
                pid: WRAPPER_PID,
                start_time: 0,
            }],
        );
        assert_eq!(
            scan_for_child(Path::new("Z:/locks"), WRAPPER_PID, 0, &probe),
            ScanOutcome::NotYet,
            "with an unknown start time (0), only a PID-alone comparison remains, so it is not adopted"
        );
    }

    #[test]
    fn an_unknown_child_pid_never_matches() {
        let probe = FakeProbe::default().file(
            &format!("{ID_A}.lock"),
            &[Holder {
                pid: 0,
                start_time: WRAPPER_START,
            }],
        );
        assert_eq!(
            scan_for_child(Path::new("Z:/locks"), 0, WRAPPER_START, &probe),
            ScanOutcome::NotYet
        );
    }

    // ── 0 candidates / 2 or more ────────────────────────────────────────────────────

    #[test]
    fn an_empty_directory_is_not_yet() {
        assert_eq!(scan(&FakeProbe::default()), ScanOutcome::NotYet);
    }

    #[test]
    fn a_lock_with_no_holder_is_not_yet() {
        let probe = FakeProbe::default().file(&format!("{ID_A}.lock"), &[]);
        assert_eq!(scan(&probe), ScanOutcome::NotYet);
    }

    #[test]
    fn two_matching_locks_are_ambiguous() {
        let probe = FakeProbe::default()
            .file(&format!("{ID_A}.lock"), &[codex()])
            .file(&format!("{ID_B}.lock"), &[codex()]);
        assert_eq!(scan(&probe), ScanOutcome::Ambiguous(2));
    }

    // ── Name decision ───────────────────────────────────────────────────────────────

    #[test]
    fn names_that_are_not_a_uuid_lock_are_skipped() {
        let probe = FakeProbe::default()
            .file(".coordination.lock", &[codex()])
            .file("not-a-uuid.lock", &[codex()])
            .file(&format!("{ID_B}.tmp"), &[codex()])
            .file(ID_B, &[codex()])
            .file(&format!("{ID_A}.lock"), &[codex()]);
        assert_eq!(
            scan(&probe),
            found(ID_A),
            "a name that is not an id does not increase the candidate count"
        );
    }

    #[test]
    fn a_held_lock_with_an_unparseable_name_is_not_a_candidate() {
        let probe = FakeProbe::default().file(".coordination.lock", &[codex()]);
        assert_eq!(scan(&probe), ScanOutcome::NotYet);
    }

    /// ★A non-canonical uuid name is not a candidate★ — accepted, one file we did not create would flip the decision to `Ambiguous` and
    ///   stop that agent's recovery.
    #[test]
    fn non_canonical_uuid_names_are_not_candidates() {
        let simple = ID_A.replace('-', "");
        for name in [
            format!("{simple}.lock"),
            format!("{{{ID_A}}}.lock"),
            format!("urn:uuid:{ID_A}.lock"),
        ] {
            let probe = FakeProbe::default().file(&name, &[codex()]);
            assert_eq!(
                scan(&probe),
                ScanOutcome::NotYet,
                "{name} came up as a candidate"
            );
        }
    }

    #[test]
    fn an_uppercase_canonical_name_is_still_a_candidate() {
        let probe = FakeProbe::default().file(&format!("{}.lock", ID_A.to_uppercase()), &[codex()]);
        assert_eq!(scan(&probe), found(ID_A));
    }

    /// ★Count matched **holders**, not files★ — two of our processes holding one lock leaves 「who owns it」 unnarrowed; counting files
    ///   would pass it as `Found`.
    #[test]
    fn two_of_our_processes_on_one_lock_are_ambiguous() {
        let probe = FakeProbe::default()
            .with_tool()
            .file(&format!("{ID_A}.lock"), &[codex(), tool()]);
        assert_eq!(scan(&probe), ScanOutcome::Ambiguous(2));
    }

    #[test]
    fn strangers_sharing_the_lock_do_not_make_it_ambiguous() {
        let probe = FakeProbe::default().file(
            &format!("{ID_A}.lock"),
            &[
                Holder {
                    pid: 7,
                    start_time: 9,
                },
                codex(),
            ],
        );
        assert_eq!(scan(&probe), found(ID_A));
    }

    // ── Path derivation ─────────────────────────────────────────────────────────────

    #[test]
    fn the_lock_dir_hangs_off_the_codex_home() {
        assert_eq!(
            lock_dir_under(Path::new("C:/Users/x/.codex")),
            PathBuf::from("C:/Users/x/.codex").join("thread-writer-locks")
        );
    }

    #[test]
    fn codex_home_prefers_the_env_override() {
        assert_eq!(
            codex_home_from(
                Some(OsString::from("D:/alt-codex")),
                Some(PathBuf::from("C:/Users/x"))
            ),
            Some(PathBuf::from("D:/alt-codex"))
        );
    }

    #[test]
    fn an_empty_or_absent_override_falls_back_to_the_user_home() {
        assert_eq!(
            codex_home_from(Some(OsString::new()), Some(PathBuf::from("C:/Users/x"))),
            Some(PathBuf::from("C:/Users/x").join(".codex"))
        );
        assert_eq!(
            codex_home_from(None, Some(PathBuf::from("C:/Users/x"))),
            Some(PathBuf::from("C:/Users/x").join(".codex"))
        );
    }

    #[test]
    fn no_home_means_no_lock_dir() {
        assert_eq!(codex_home_from(None, None), None);
    }

    // ── Recovery loop (ADR-0218 decisions 4·5·6) ────────────────────────────────────

    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    type Ticks = Rc<Cell<usize>>;

    struct FakeClock {
        ticks: Ticks,
        alive_through: usize,
        slept: RefCell<Vec<Duration>>,
    }

    impl FakeClock {
        fn immortal(ticks: Ticks) -> Self {
            Self {
                ticks,
                alive_through: usize::MAX,
                slept: RefCell::new(Vec::new()),
            }
        }
        fn dies_after(ticks: Ticks, rounds: usize) -> Self {
            Self {
                ticks,
                alive_through: rounds,
                slept: RefCell::new(Vec::new()),
            }
        }
        fn rounds(&self) -> usize {
            self.slept.borrow().len()
        }
    }

    impl CaptureClock for FakeClock {
        fn child_alive(&self) -> bool {
            self.ticks.get() <= self.alive_through
        }
        fn sleep(&self, delay: Duration) {
            self.slept.borrow_mut().push(delay);
            self.ticks.set(self.ticks.get() + 1);
        }
    }

    struct ScriptedProbe {
        ticks: Ticks,
        rounds: Vec<FakeProbe>,
    }

    impl ScriptedProbe {
        fn round(&self) -> &FakeProbe {
            let at = self.ticks.get().saturating_sub(1);
            self.rounds
                .get(at)
                .or_else(|| self.rounds.last())
                .expect("at least one round is recorded")
        }
    }

    impl LockHolderProbe for ScriptedProbe {
        fn holders(&self, path: &Path) -> Vec<Holder> {
            self.round().holders(path)
        }
        /// ★Delegate **all** three methods — a missed one silently goes to the real OS★ (when missed, the tree query scanned the real
        ///   process table with a fake PID, always came back empty, and the loop never ended: 3 items hung >60 s, 2026-09-21).
        fn our_processes(&self, root_pid: u32, root_start_time: u64) -> Vec<ProcessIdentity> {
            self.round().our_processes(root_pid, root_start_time)
        }
        fn entries(&self, dir: &Path) -> Vec<PathBuf> {
            self.round().entries(dir)
        }
    }

    fn recording_sink() -> (SessionIdSink, Arc<Mutex<Vec<String>>>) {
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let write = seen.clone();
        let sink: SessionIdSink = Arc::new(move |raw: &str| {
            write.lock().expect("sink poisoned").push(raw.to_string());
        });
        (sink, seen)
    }

    fn plan_with(sink: SessionIdSink) -> CapturePlan {
        CapturePlan {
            lock_dir: PathBuf::from("Z:/locks"),
            child_pid: WRAPPER_PID,
            child_start_time: WRAPPER_START,
            sink,
        }
    }

    fn drive(rounds: Vec<FakeProbe>, clock_for: fn(Ticks) -> FakeClock) -> (Vec<String>, usize) {
        let ticks: Ticks = Rc::new(Cell::new(0));
        let probe = ScriptedProbe {
            ticks: ticks.clone(),
            rounds,
        };
        let clock = clock_for(ticks);
        let (sink, seen) = recording_sink();
        run_capture(&plan_with(sink), &probe, &clock);
        let got = seen.lock().expect("sink poisoned").clone();
        (got, clock.rounds())
    }

    #[test]
    fn the_loop_keeps_waiting_until_the_lock_shows_up_and_then_adopts_it() {
        let (got, rounds) = drive(
            vec![
                FakeProbe::default(),
                FakeProbe::default().file(&format!("{ID_B}.lock"), &[]),
                FakeProbe::default().file(&format!("{ID_A}.lock"), &[codex()]),
            ],
            FakeClock::immortal,
        );
        assert_eq!(got, vec![ID_A.to_string()], "must adopt on the third round");
        assert_eq!(
            rounds, 3,
            "must stop on the third round after two empty-handed rounds"
        );
    }

    /// ★A refusal, not a giving up (ADR-0218 decision 3 vs 4)★ — once the overlap clears we adopt next round; ending the thread would end
    ///   that agent's recovery for good after one short overlap.
    #[test]
    fn an_ambiguous_scan_adopts_nothing_but_keeps_polling() {
        let (got, rounds) = drive(
            vec![
                FakeProbe::default()
                    .file(&format!("{ID_A}.lock"), &[codex()])
                    .file(&format!("{ID_B}.lock"), &[codex()]),
                FakeProbe::default().file(&format!("{ID_A}.lock"), &[codex()]),
            ],
            FakeClock::immortal,
        );
        assert_eq!(
            got,
            vec![ID_A.to_string()],
            "must adopt on the round after the overlap clears"
        );
        assert_eq!(rounds, 2, "ended the thread on the overlapping round");
    }

    #[test]
    fn an_ambiguous_scan_that_never_clears_adopts_nothing() {
        let (got, rounds) = drive(
            vec![FakeProbe::default()
                .file(&format!("{ID_A}.lock"), &[codex()])
                .file(&format!("{ID_B}.lock"), &[codex()])],
            |ticks| FakeClock::dies_after(ticks, 3),
        );
        assert!(
            got.is_empty(),
            "adopted although there are two or more: {got:?}"
        );
        assert_eq!(rounds, 4, "must keep going until the child dies");
    }

    #[test]
    fn the_child_exiting_ends_the_loop() {
        let (got, rounds) = drive(vec![FakeProbe::default()], |ticks| {
            FakeClock::dies_after(ticks, 2)
        });
        assert!(got.is_empty(), "adopted although the child died: {got:?}");
        assert_eq!(
            rounds, 3,
            "must end on the round where the liveness check failed"
        );
    }

    /// ★Adoption goes out through the handed-over verb **alone** — even if it discards the value there is no second path★.
    ///
    /// The fake has the shape of a dead incarnation's record verb (receives, leaves nothing in the profile). That the production verb
    ///   carries the incarnation guard is measured on the assembly side by
    /// `manager::tests::a_dead_incarnations_sink_cannot_overwrite_a_live_value` (ADR-0004: `ProfileRegistry` does not enter the backend),
    ///   so this spot uses a **refusing verb**.
    #[test]
    fn a_sink_that_drops_the_value_does_not_make_the_loop_look_elsewhere() {
        let calls = Arc::new(Mutex::new(0usize));
        let count = calls.clone();
        let sink: SessionIdSink = Arc::new(move |_raw: &str| {
            *count.lock().expect("sink poisoned") += 1;
        });
        let ticks: Ticks = Rc::new(Cell::new(0));
        let probe = ScriptedProbe {
            ticks: ticks.clone(),
            rounds: vec![FakeProbe::default().file(&format!("{ID_A}.lock"), &[codex()])],
        };
        let clock = FakeClock::immortal(ticks);

        run_capture(&plan_with(sink), &probe, &clock);

        assert_eq!(
            *calls.lock().expect("sink poisoned"),
            1,
            "the record verb must be called exactly once"
        );
        assert_eq!(
            clock.rounds(),
            1,
            "must not read the refusal as a retry and keep going"
        );
    }

    #[test]
    fn the_delay_doubles_and_stops_at_the_ceiling() {
        assert_eq!(next_delay(FIRST_DELAY), FIRST_DELAY * 2);
        assert_eq!(next_delay(MAX_DELAY), MAX_DELAY);
        assert_eq!(
            next_delay(MAX_DELAY / 2 + Duration::from_secs(1)),
            MAX_DELAY
        );
    }

    // ── Conditions to run recovery (ADR-0218 decision 5) ────────────────────────────

    const OUR_HOME: &str = "C:/Users/x/.codex";

    fn ours() -> Option<PathBuf> {
        Some(lock_dir_under(Path::new(OUR_HOME)))
    }

    fn plan(
        env: &[(&str, &str)],
        resumes_by_argv: bool,
        pid: Option<u32>,
        start: Option<u64>,
    ) -> Option<CapturePlan> {
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        plan_capture(
            &env,
            resumes_by_argv,
            pid,
            start,
            Some(recording_sink().0),
            ours(),
        )
    }

    /// ★The plan carries the **handed-over** record verb as is★ — inventing another here would lose the incarnation guard bound at the
    ///   assembly point (`manager::session_id_sink`).
    #[test]
    fn the_plan_carries_the_sink_it_was_handed() {
        let (sink, seen) = recording_sink();
        let made = plan_capture(
            &[],
            false,
            Some(WRAPPER_PID),
            Some(WRAPPER_START),
            Some(sink),
            ours(),
        )
        .expect("fresh runs");

        (made.sink)("probe");

        assert_eq!(
            *seen.lock().expect("sink poisoned"),
            vec!["probe".to_string()],
            "the plan carries a different verb"
        );
    }

    #[test]
    fn a_fresh_incarnation_gets_a_plan() {
        let made = plan(&[], false, Some(WRAPPER_PID), Some(WRAPPER_START)).expect("fresh runs");
        assert_eq!(made.child_pid, WRAPPER_PID);
        assert_eq!(made.child_start_time, WRAPPER_START);
        assert_eq!(made.lock_dir, lock_dir_under(Path::new(OUR_HOME)));
    }

    /// ★The gate looks at 「did argv actually resume」, not 「is there a handle」★; the tell-apart case (an unloadable handle) is measured by
    ///   this folder's `tests::a_handle_we_refuse_to_resume_with_is_still_captured`.
    #[test]
    fn a_resume_incarnation_never_starts_the_loop() {
        assert!(
            plan(&[], true, Some(WRAPPER_PID), Some(WRAPPER_START)).is_none(),
            "a resume incarnation already went out carrying the id as argv — there is nothing to recover"
        );
    }

    #[test]
    fn an_unknown_pid_or_start_time_never_starts_the_loop() {
        assert!(plan(&[], false, None, Some(WRAPPER_START)).is_none());
        assert!(
            plan(&[], false, Some(WRAPPER_PID), None).is_none(),
            "without the start time, only a PID-alone comparison remains"
        );
    }

    #[test]
    fn without_a_sink_or_a_lock_dir_there_is_nothing_to_run() {
        assert!(
            plan_capture(
                &[],
                false,
                Some(WRAPPER_PID),
                Some(WRAPPER_START),
                None,
                ours()
            )
            .is_none(),
            "even if received, there is nowhere to keep it"
        );
        assert!(plan_capture(
            &[],
            false,
            Some(WRAPPER_PID),
            Some(WRAPPER_START),
            Some(recording_sink().0),
            None,
        )
        .is_none());
    }

    /// ★A profile-set `CODEX_HOME` beats ours★ — it applies only to the process that sets it, so scanning our folder never sees the
    ///   child's lock (empty-handed forever, no error).
    #[test]
    fn the_lock_dir_comes_from_the_childs_environment() {
        let made = plan(
            &[("CODEX_HOME", "D:/alt-codex")],
            false,
            Some(WRAPPER_PID),
            Some(WRAPPER_START),
        )
        .expect("fresh runs");
        assert_eq!(made.lock_dir, lock_dir_under(Path::new("D:/alt-codex")));
    }

    #[test]
    fn the_childs_env_key_follows_the_os_rule_and_last_one_wins() {
        let made = plan(
            &[
                ("CODEX_HOME", "D:/first"),
                ("CODEX_HOME", "D:/second"),
                ("codex_home", "D:/third"),
            ],
            false,
            Some(WRAPPER_PID),
            Some(WRAPPER_START),
        )
        .expect("fresh runs");
        let expected = if env_key_eq("codex_home", CODEX_HOME_ENV) {
            "D:/third"
        } else {
            "D:/second"
        };
        assert_eq!(
            made.lock_dir,
            lock_dir_under(Path::new(expected)),
            "the path pushes env in order, so what the child receives is the later of those seen as the same key"
        );
    }

    #[test]
    fn an_empty_codex_home_falls_back_to_ours() {
        let made = plan(
            &[("CODEX_HOME", "")],
            false,
            Some(WRAPPER_PID),
            Some(WRAPPER_START),
        )
        .expect("fresh runs");
        assert_eq!(made.lock_dir, lock_dir_under(Path::new(OUR_HOME)));
    }

    // ── Coupling with the real probe (Windows) ──────────────────────────────────────

    /// One round with [`RestartManagerProbe`] instead of a fake — finds the `<uuid>.lock` this process opened, pretending it is the child
    ///   (own PID and start time). A **not held** lock and a non-id name sit in the same folder so list-side rules pass through the real
    ///   thing too.
    #[cfg(windows)]
    #[test]
    fn the_real_probe_finds_a_lock_this_process_holds() {
        let dir = std::env::temp_dir().join(format!(
            "engram-thread-lock-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create the temp lock folder");
        std::fs::write(dir.join(format!("{ID_B}.lock")), b"").expect("unheld lock");
        std::fs::write(dir.join(".coordination.lock"), b"").expect("a name that is not an id");
        let file = std::fs::File::create(dir.join(format!("{ID_A}.lock"))).expect("held lock");

        let me = std::process::id();
        let start = engram_dashboard_platform::process::process_creation_time(me)
            .expect("own creation time is readable");
        let outcome = scan_for_child(&dir, me, start, &RestartManagerProbe);

        drop(file);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(outcome, found(ID_A));
    }

    /// The real [`ChildClock`] stops the capture loop for a child that exited while its object is still held (the spawner keeps the
    ///   handle) — start time alone still matched it (TRD S21 trd-3-2-u5 R4).
    #[cfg(windows)]
    #[test]
    fn the_real_child_clock_sees_an_exited_child_as_gone() {
        let mut child = engram_dashboard_platform::testing::spawn_gated_cmd("rem", 0);
        let pid = child.id();
        let start_time =
            engram_dashboard_platform::process::process_creation_time(pid).expect("start time");
        let clock = ChildClock { pid, start_time };
        let alive_before = clock.child_alive();

        engram_dashboard_platform::testing::open_gate(&mut child);
        let _ = child.wait();
        let alive_after = clock.child_alive();
        drop(child);

        assert!(alive_before, "a running child is alive");
        assert!(
            !alive_after,
            "an exited child whose handle is held must read as gone"
        );
    }
}
