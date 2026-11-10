//! Recover the codex thread id as the **owner of a lock file** — the name of the lock file held by the child we
//! spawned is that child's thread id (ADR-0218 decision 1).
//!
//! ★This file holds only two pieces of codex knowledge★ — the location of the lock directory ([`lock_dir`]) and that the file name is the id.
//! 「이 파일을 누가 쥐고 있나」 (who holds this file) and 「이 PID 아래 무엇이 살아 있나」 (what is alive under this PID) need zero domain knowledge, so the OS-layer crate's
//! [`engram_dashboard_platform::file_holders`] and [`engram_dashboard_platform::process::subtree`] answer them
//! (ADR-0004 · ADR-0218 decision 11 · ADR-0266). ★**The file content is never read**★ — it is 0 bytes, and starting to read it would make us depend on the vendor
//! storage format (ADR-0203).
//!
//! Two entry points — [`plan_capture`] (chooses whether to run recovery for this spawn, and its inputs)
//! and [`spawn_capture`] (starts the polling thread with those inputs). [`scan_for_child`] carries the decision, and
//! [`CaptureClock`] carries the waiting and the child liveness check. All filesystem and OS queries sit behind those two and [`LockHolderProbe`],
//! so the loop rules are measured without an OS (ADR-0012).
//!
//! ★The recovered value leaves through exactly one path, [`crate::backend::SessionIdSink`]★ — the same verb
//! the assembly point handed over bound to the incarnation marker, and this file knows neither the profile nor its schema (ADR-0004 · ADR-0218 decision 6). Do not
//! create a second write path — the moment you do, there is a branch that bypasses the incarnation guard.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use engram_dashboard_platform::env::env_key_eq;
use engram_dashboard_platform::file_holders::{self, Holder};
use engram_dashboard_platform::process::{self, ProcessIdentity};
use uuid::Uuid;

use crate::backend::SessionIdSink;

/// The name of the folder under the codex home where the writer locks live.
///
/// ★This is an undocumented layout★ — if upstream changes it, we silently get 0 candidates instead of an error (ADR-0218 「근거」 (rationale),
/// unmeasured item ③). The failure mode is 「못 받음」 (not received), so that silence does not spread into false detections.
const LOCK_DIR: &str = "thread-writer-locks";

/// The extension of the lock file. The rest of the name is the thread id.
const LOCK_EXTENSION: &str = "lock";

/// The codex home when [`CODEX_HOME_ENV`] is absent (a path relative to the user home).
const CODEX_HOME_SUBDIR: &str = ".codex";

/// The env variable that moves the whole codex state directory — its existence is confirmed by measurement, and it applies **only to the process that sets it**
/// (the `CODEX_HOME` section of `docs/research/codex-session-id-recovery-survey-2026-09-19.md`).
pub(super) const CODEX_HOME_ENV: &str = "CODEX_HOME";

/// Every place where this module meets the OS — file holders · folder listing · our process tree.
///
/// ★The three sit on one seam so the decision rules can be measured without an OS★ — that way [`scan_for_child`] runs
/// without a single real file or a single real process (ADR-0012).
pub(crate) trait LockHolderProbe {
    /// **No distinction** between "could not ask" and "no holders" — both are an empty list. The decision lands in the same
    /// place either way (「후보 아님」 (not a candidate)), and polling asks again on the next round.
    fn holders(&self, path: &Path) -> Vec<Holder>;

    /// The identities of the child we spawned and every **live** descendant under it. An empty list = that PID is no longer our
    /// child.
    ///
    /// ★Why a tree — the PID we hold is not codex★: on Windows the spawn passes through a `cmd.exe /c codex …`
    ///   layer ([`engram_dashboard_platform::shell::console_command`]), so the PID the path returned is
    ///   **that wrapper**. The one holding the lock is the `codex.exe` below it (ADR-0218 「근거」 (rationale) — the holder was neither the wrapper nor the child but
    ///   codex.exe). ★Compare only the wrapper PID and a match **never** shows up, and the symptom is silence, not an error★
    ///   — the thread spins empty-handed for the whole agent lifetime (measured 2026-09-21: `cmd.exe /c …` →
    ///   wrapper 38240 · 4816,19468 below it).
    /// ★Resolve it again every round — do not reuse a list copied aside once★: the shim launches codex late and
    ///   codex launches more tools, so the tree grows over time. Freeze it and the late-launched codex goes unseen.
    /// ★What the tree rules block in this recovery, and the residue we accepted★ — the factual source of record for the rules (root identity check · dropping
    ///   entries born before their parent) and for the cases ①②③ they cannot filter is the doc of [`process::subtree`].
    ///   Here we only write what that means for codex recovery.
    ///   - **The real sequence the ordering rule blocks:** a `codex.exe` the user launched by hand becomes an orphan and stays with the recorded ppid
    ///     `P` intact, and Windows **reuses** `P` for our wrapper. Without the rule that stranger is
    ///     enumerated as our child, and since the stranger's own identity matches its own lock holder, it becomes **the only candidate for 0.7–21 seconds
    ///     before our codex creates its lock** — not even `Ambiguous` shows up, and another party's thread is written into our
    ///     handle (the silent failure ADR-0218 「영향/불변식」 (impact/invariants) names).
    ///   - ★**We accept residue ①②③ (do not argue it again — decision of 2026-09-21)**★. ① PID reuse between enumeration and start-time reading
    ///     has no atomic fix and the window is microseconds. ② Blocking the same tick would lose a normal
    ///     child. ③ For an explicit-parent case to become a **real false detection**, the stranger would have to hold a `<uuid>.lock` under our `CODEX_HOME`
    ///     as well — closing the common path (an orphan with a reused ppid) keeps the rule's
    ///     worth intact.
    ///   - A tree enumeration that fails and returns less is also not treated as an error — there is no way to handle it differently from 「후보 아님」 (not a candidate),
    ///     and the next round asks again.
    fn our_processes(&self, root_pid: u32, root_start_time: u64) -> Vec<ProcessIdentity> {
        process::subtree(root_pid, root_start_time)
    }

    /// All **regular file** paths directly under `dir` (the name decision belongs to [`scan_for_child`]). If it cannot be read, an empty
    /// list: a directory that does not exist yet is a normal path, so we do not raise an error.
    ///
    /// ★Directories and reparse points (symlink·junction) are left out of the candidates★ — a folder named `<uuid>.lock`
    ///   is not a lock, and passing a reparse point to the holder query opens a target we did not choose.
    ///   `file_type()` does not follow links, so `is_file()` filters out both. Entries whose type
    ///   cannot be read are left out too — we do not promote what we do not know to a candidate.
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

/// The implementation used in production — asks [`engram_dashboard_platform::file_holders`], and if the query itself fails folds to an empty
/// list (the reason for that folding is the contract of [`LockHolderProbe::holders`]).
pub(crate) struct RestartManagerProbe;

impl LockHolderProbe for RestartManagerProbe {
    fn holders(&self, path: &Path) -> Vec<Holder> {
        match file_holders::holders_of(path) {
            Ok(holders) => holders,
            Err(err) => {
                tracing::debug!(path = %path.display(), %err, "락 파일 홀더 조회 실패");
                Vec::new()
            }
        }
    }
}

/// The answer of [`scan_for_child`]. ★Only `Found` can be adopted★.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScanOutcome {
    /// Exactly one lock held by a process in our tree — the thread id of that file name.
    Found(Uuid),
    /// 0 candidates. No lock yet, or this child does not create a lock — keep waiting (ADR-0218 decision 4).
    NotYet,
    /// n (≥2) candidates — ★adopt nothing this round★. It is unmeasured whether one codex process can hold two or more locks,
    /// so we refuse instead of guessing (ADR-0218 decision 3 · not receiving is
    /// better than receiving wrongly). ★「거절」 (refusal) is not 「포기」 (giving up) — polling continues★ (this outcome is not in decision 4's list of stops).
    Ambiguous(usize),
}

/// Within `lock_dir`, find the lock file **held by a process in our tree** and return the thread id of its name.
///
/// Decision rules (all ADR-0218):
/// - **The comparison target is not `child_pid` alone but the whole tree rooted at that PID**
///   ([`LockHolderProbe::our_processes`] is the source of record for the reason — the PID we hold is the `cmd.exe` wrapper and the one holding the
///   lock is the `codex.exe` below it).
/// - Candidate = a file named `<uuid>.lock` where some holder matches an entry of the tree on **both PID and start time**.
///   ★A match on only one of them is not a candidate★ — PIDs are reused, so that decision would write another party's thread
///   into our handle (decision 2). Widening the tree does not loosen this pair: the start time of descendants is
///   read together at the same place where they are enumerated.
/// - ★**What we count is not files but 「맞은 홀더」 (matched holders)**★ — decision 3's 「후보가 정확히 하나」 (exactly one candidate) is the number of **pairs**, not
///   the number of file names. If two of our processes hold one file together, 「그 락이 누구 것인가」 (who owns that lock) is
///   not narrowed to one, and counting only files lets that state pass as `Found`.
///   ★We chose this knowing its cost★: if a child inherits the handle (a tool launched by codex inherits the same lock handle),
///   adoption does not happen during that whole span. It is tolerable because [`run_capture`] does **not stop** on `Ambiguous`
///   — once the inheriting side ends, we adopt on the next round. Whether inheritance really happens is
///   unmeasured, and if it does, the symptom is 「회수가 늦어진다」 (recovery gets late), not a false detection.
///   ★**There is dissent on this cost — read this line before raising it again**★ (2026-09-21): there was a counterargument 「둘 다 **우리** 것이면
///   그 락은 우리 것이니 채택이 옳다」 (if both are ours, the lock is ours so adopting is right), and that decision itself was outside this round's scope, so the code was
///   left as is. That is, the current shape is **a hold, not a decision**. To change it, first measure whether inheritance exists and
///   reopen ADR-0218 decision 3 on top of that measurement — do not flip it silently here.
/// - Exactly one matched holder gives [`ScanOutcome::Found`], 0 gives [`ScanOutcome::NotYet`], two or more give
///   [`ScanOutcome::Ambiguous`] (decision 3).
/// - If `child_pid` or `child_start_time` is 0, the answer is `NotYet` **without even asking**. 0 means 「미상」 (unknown), and counting unknown as
///   a match leaves a PID-only comparison, which is the decision decision 2 forbids.
/// - If the tree is empty (the child died or the PID was reused), `NotYet` without asking about a single file —
///   the holder query opens one Restart Manager session per candidate file per round and is an expensive question, so we do not burn it
///   where the answer is already fixed.
///
/// Many processes may have the same file open — if one of them is inside our tree, it is a candidate.
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

/// The lock directory as seen from **our process's** environment. `None` if the home cannot be found.
///
/// ★This function does not answer for a spawn that carries its own [`CODEX_HOME_ENV`]★ — a child that set that variable
/// through the profile env uses another home. In that case pass that home directly to [`lock_dir_under`].
pub(crate) fn lock_dir() -> Option<PathBuf> {
    Some(lock_dir_under(&codex_home()?))
}

/// For callers that know the home — `<codex home>/thread-writer-locks`.
pub(crate) fn lock_dir_under(codex_home: &Path) -> PathBuf {
    codex_home.join(LOCK_DIR)
}

/// Lock file name → thread id. `None` if it is not `<uuid>.lock` — it is **skipped**, not an error (measured: files that are not ids, like `.coordination.lock`,
/// live in the same folder).
///
/// ★Accept only the canonical form (36 chars with hyphens) — do not leave `Uuid::parse_str` alone★: that function
///   also accepts 32 chars without hyphens, braces and `urn:uuid:`, so names codex never creates come up as candidates. One more
///   candidate flips the decision from `Found` to `Ambiguous` and **recovery stops entirely** — and it happens just because
///   one file we did not create sits in someone else's folder. Case is not distinguished (Windows file names behave that way).
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

/// Only the rule of [`codex_home`] — split off so it can be measured without touching env, and so the settings
/// check can apply it to an agent's own env. `None` only when `user_home` is `None` and no override is set.
pub(super) fn codex_home_from(
    override_var: Option<OsString>,
    user_home: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(dir) = override_var {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    user_home.map(|home| home.join(CODEX_HOME_SUBDIR))
}

/// The wait until the first scan. ★Non-zero is intended★ — the measured minimum of the spawn→lock-creation delay is 0.735 s
/// (ADR-0218 「근거」 (rationale)), so scanning right at launch buys one certain empty-handed round for free.
const FIRST_DELAY: Duration = Duration::from_millis(500);

/// The cap when the wait is doubled after each empty-handed round.
///
/// ★Why backoff — a round is not free★: [`scan_for_child`] opens one Restart
///   Manager session per candidate file. A real user's lock folder had 8 files (measured). So a fixed short interval,
///   under a child that never creates a lock (upstream changed the layout, or it is `codex exec`), burns several RM sessions per second
///   for as long as that agent lives.
/// ★Even so, we set no upper **time** limit (ADR-0218 decision 4)★ — the decision is ownership, not time, so
///   catching late adds no false detections. Slowing down without giving up is the difference between backoff and a time window.
/// ★Why the first few rounds are dense★: 0.735 · 0.879 · 0.907 · 1.891 s are the measured creation delays, and there is one
///   21 s case that was not reproduced. Doubling from 0.5 s puts the sixth round near 30 s, which covers that tail too.
const MAX_DELAY: Duration = Duration::from_secs(15);

/// The other two places where the polling loop meets the OS — whether the child is still alive, and the wait until the next round.
///
/// ★It is split from [`LockHolderProbe`] because the axis differs★ — that one is 「무엇이 보이나」 (what is visible), this one is
///   「언제 다시 보나 · 볼 이유가 남았나」 (when to look again, and whether a reason to look remains). Merging them mixes a place where measuring the loop rules would force faking the clock too
///   with a place where only the list needs to change.
pub(crate) trait CaptureClock {
    /// `false` = that PID is no longer **our child** (it died or the PID was reused). It is the loop's normal exit
    /// condition and not an error.
    fn child_alive(&self) -> bool;

    /// Wait until the next round. ★Called while holding no lock at all★ — this is the place where the loop
    /// spends the most time.
    fn sleep(&self, delay: Duration);
}

/// The clock used in production — the child decision looks at PID and start time **together** (the same rule as ADR-0218 decision 2).
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

/// One set of inputs for running recovery in this spawn. ★That it was built at all means 「돌릴 조건을 다 채웠다」 (all conditions to run are met)★ —
/// if [`plan_capture`] misses even one it is `None`, so no conditional is left inside [`run_capture`].
pub(crate) struct CapturePlan {
    /// The lock folder used by this **child** — not ours (the contract of [`plan_capture`]).
    pub(crate) lock_dir: PathBuf,
    pub(crate) child_pid: u32,
    pub(crate) child_start_time: u64,
    /// The record verb the assembly point handed over bound to the incarnation marker. It is the **only** path the recovered value leaves through.
    ///
    /// ★What does this `Arc` keep alive — checked again★: the inner closure holds an `Arc<ProfileRegistry>`.
    ///   But that registry is a single owned object that `AgentManager` holds **for the whole daemon lifetime**, so this
    ///   clone does not extend that lifetime for even a moment. What it extends is only the thread's own lifetime, and that
    ///   ends when the child dies (the stop condition of [`run_capture`]).
    ///   ★Why it is not a `Weak`★: there is no place to hold the strong side — the assembly point passes this verb to
    ///   `open_spawn` and lets go. To make the session hold it, the ownership split of `AgentSession` (the
    ///   「소유권 분할」 (ownership split) invariant) would have to change, and that is not a change this recovery path alone should demand.
    pub(crate) sink: SessionIdSink,
}

/// Should this spawn run lock recovery — if so, its inputs, otherwise `None`.
///
/// Conditions (if any one is missing, `None`):
/// - **This spawn does not emit resume argv** (`resumes_by_argv == false`). ★The point is that what is asked is
///   not 「저장된 손잡이가 있나」 (is there a stored handle) but 「그 손잡이가 실제로 argv 로 나갔나」 (did that handle actually go out as argv)★ —
///   even with a handle, if its value cannot be loaded (a value `resume_argument` refuses) codex comes up as a **new conversation**,
///   and nobody writes the id of that incarnation. The old form that decided on 「있다」 (there is one) was exactly that hole
///   (found 2026-09-21). We make the two decisions look at the **same single predicate** to remove that mismatch
///   (ADR-0218 decision 5 · source of record for the predicate = `super::resume_argument`).
///   ★The combination of Fresh with a handle coming along is blocked by the assembly point★ — `manager::resume_handle_for`
///   (its doc and its tests are the source of record). So this entry holds without taking the mode.
/// - There is somewhere to record (`sink`). Without it, there is nowhere to keep what we receive.
/// - Both the child's PID and **start time** are known. If either is unknown, the decision sinks to a PID-only comparison
///   and becomes the shape ADR-0218 decision 2 forbids — so we cut it off here.
/// - The lock folder is known.
///
/// ★Resolve the lock folder from the **child's** environment — do not substitute ours★: `CODEX_HOME` applies only to the process that sets it,
///   so if the profile sets that value, the child's lock **never** appears in the folder we look at. That mismatch is
///   not an error but 「계속 빈손」 (still empty-handed), so there is no signal on screen.
/// `our_lock_dir` = the value to use when the child inherited our environment as is ([`lock_dir`]). It is taken as an argument
///   so this decision can be measured without env.
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

/// Only the folder rule of [`plan_capture`] — if the spawn env sets `CODEX_HOME`, that home wins, otherwise ours does.
///
/// ★Pick the **last** entry★ — the spawn env is a list and the same key can be loaded twice, and the path pushes that list
///   into `cmd.env` in order, so what the child actually receives is the later one.
/// ★Key comparison follows the OS rule for environment variable names ([`env_key_eq`])★ — on Windows case is not distinguished.
/// ★An empty value counts as 「안 걸었다」 (not set)★ — [`codex_home_from`] reads our side's env that way, so we use the same rule.
/// ★Known hole — relative paths★: such a value is resolved later against **our** cwd, but the child would resolve it
///   against its own cwd, so the two can diverge. The symptom is not a false detection but 「계속 빈손」 (still empty-handed) — candidate decision still requires the child's
///   PID and start time. Whether such a profile really exists is unconfirmed, so we do not fix it and only write it down.
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

/// Start the recovery polling on a **separate thread** — the spawn goes ahead even if it fails.
///
/// ★It blocks neither the spawn path nor the pump★: this thread only does `sleep` and OS queries and holds none of the session's
///   locks. It does not hold the session either — what it holds is just one record verb, so the agent is not reaped late
///   because this thread remains.
/// ★If the thread cannot be started, only a warning is left and it ends★ — the failure mode is 「id 를 못 받음」 (cannot receive the id) and that is not a reason
///   to cut the spawn (the same handling as ADR-0218 decision 10).
pub(crate) fn spawn_capture(plan: CapturePlan) {
    let clock = ChildClock {
        pid: plan.child_pid,
        start_time: plan.child_start_time,
    };
    tracing::debug!(
        pid = plan.child_pid,
        lock_dir = %plan.lock_dir.display(),
        "codex 세션 id 회수 폴링을 시작한다"
    );
    let spawned = std::thread::Builder::new()
        .name("codex-thread-lock".to_string())
        .spawn(move || run_capture(&plan, &RestartManagerProbe, &clock));
    if let Err(err) = spawned {
        tracing::warn!(%err, "codex 세션 id 회수 스레드를 띄우지 못했다 — 이 화신은 id 를 못 받는다");
    }
}

/// Poll until caught — ★there are only three stop conditions and a time limit is not among them★ (ADR-0218 decision 4).
///
/// 1. [`ScanOutcome::Found`] — adopt and stop.
/// 2. The child died ([`CaptureClock::child_alive`]).
/// 3. The session goes down — ★having no separate signal is intended★: shutdown starts with the path killing the child
///    (「kill 인과」 (kill causality)), so its outcome arrives as item 2. Attaching a second signal would put this observer into the shutdown order.
///
/// ★[`ScanOutcome::Ambiguous`] is not a stop — it means 「이번 바퀴에 채택 안 함」 (not adopted this round)★: what ADR-0218 decision 3
///   says is **refusal**, and this outcome is not in decision 4's list of stops. The overlap may be momentary
///   (a short overlap at the moment the thread switches), and if the thread ended there the agent would lose recovery **forever**.
///   The warning is emitted only on the first round — emitting it every round would bury the log under that one fact.
///
/// Adoption goes out only through [`CapturePlan::sink`] — even if that verb discards the value (refused
/// because it is a dead incarnation's record), we end right there without leaking through another path.
///
/// ★The window between kill and record was not closed — a conclusion we left after re-checking★: if the child dies
///   after the liveness check, this round's record still goes out. **That is correct** — the written value is the thread id
///   that incarnation, which just died, actually used, and the point is for the next activation to resume that conversation. The only harmful combination is
///   the case where **the next incarnation is already up** in between, and then the markers differ and the assembly point's incarnation guard refuses.
///   ★Why that 「거절한다」 (refuses) has no gap — we leave the grounds, not only the conclusion (so the next reader does not derive it
///   again)★:
///     1. The only place that **raises** the incarnation marker is `ProfileRegistry::epoch_for_spawn`. The one other
///        write (`merge_preserving_live`) only **preserves** the live value and does not raise it.
///     2. `AgentManager::spawn_agent` calls it exactly once per spawn, and that call is **before** `release_session_id`,
///        where Fresh clears the handle.
///     So there is no span where 「표식은 그대로인데 손잡이만 비워진」 (the marker is unchanged but only the handle is cleared) exists. At the moment a late record can arrive with the old marker,
///     the clearing has not yet happened, and if the clearing has happened the marker has already changed. When the profile is
///     deleted entirely, the lookup itself fails in the assembly point's record verb (`commit_session_id`) and it
///     drops out at the same place.
///   ★「기록이 나간다」 (the record goes out) means it is **handed** to this incarnation's first-submission latch (ADR-0226)★ — it is written to the registry only
///     when there was a submission. The value of an incarnation that died without a submission is not written (there is no conversation to resume).
///   That is why we did not add a new cancellation signal here — it is not a hole to close but an **intended outcome**.
///
/// ★Once caught, it does not follow later thread changes — a known unconfirmed item★: if the user opens
///   a new thread mid-conversation, codex will take over the lock anew (unmeasured), but this loop has already stopped, so the profile keeps the first
///   thread id. Resuming in that state opens that first conversation, not the conversation the user was last looking at.
///   「잡으면 즉시 멈춘다」 (stop immediately once caught) is a choice of ADR-0218 decision 4, so we do not flip it here.
fn run_capture(plan: &CapturePlan, probe: &dyn LockHolderProbe, clock: &dyn CaptureClock) {
    let mut delay = FIRST_DELAY;
    let mut warned_ambiguous = false;
    loop {
        clock.sleep(delay);
        if !clock.child_alive() {
            tracing::debug!(
                pid = plan.child_pid,
                "codex 자식이 먼저 끝나 세션 id 회수를 접는다"
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
                        "codex 나무가 쥔 락이 둘 이상이라 이번 바퀴에는 세션 id 를 채택하지 않는다"
                    );
                }
                delay = next_delay(delay);
            }
            ScanOutcome::NotYet => delay = next_delay(delay),
        }
    }
}

/// The next wait after an empty-handed round — doubled each time, stopping at [`MAX_DELAY`] (reason = the doc of that constant).
fn next_delay(previous: Duration) -> Duration {
    (previous * 2).min(MAX_DELAY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // ★The fake tree is **the shape that actually appears in production** — if it is flat, the whole test becomes a lie★:
    //   the PID we hold is the `cmd.exe` wrapper (`engram_dashboard_platform::shell::console_command`) and the one holding the lock is the
    //   `codex.exe` below it. If the holder is planted with the wrapper PID, even an implementation that 「래퍼만 대조하는」 (compares only the wrapper) goes green — we actually
    //   planted it that way once and it passed an implementation whose recovery never happens (found 2026-09-21).
    const WRAPPER_PID: u32 = 4242;
    const WRAPPER_START: u64 = 134_344_468_098_866_343;
    /// The wrapper's child = the process that actually holds the lock.
    const CODEX_PID: u32 = 4816;
    const CODEX_START: u64 = 134_344_468_099_111_111;
    /// A tool launched by codex — must be caught even if the tree is one level deeper.
    const TOOL_PID: u32 = 19_468;
    const TOOL_START: u64 = 134_344_468_099_222_222;

    const ID_A: &str = "01a0c2c4-97f5-75e2-8ea0-4855b7a0997d";
    const ID_B: &str = "01a0c2c3-4f4c-7ee2-a858-3db9f72c2e78";

    /// A fake holding the list, holders and tree whole — it touches neither the filesystem nor the OS.
    struct FakeProbe {
        entries: Vec<PathBuf>,
        holders: HashMap<PathBuf, Vec<Holder>>,
        tree: Vec<ProcessIdentity>,
    }

    impl Default for FakeProbe {
        /// Default tree = wrapper + codex, two levels. Only tests that need a deeper tree widen it with [`FakeProbe::with_tool`].
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
        /// If the root's identity does not match there is no tree — the same rule as the real `process::subtree`.
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

    /// The side holding the lock — ★not the PID we hold but its **child**★.
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

    /// ★Always ask with the **wrapper** PID — that is what we hold in production★.
    fn scan(probe: &FakeProbe) -> ScanOutcome {
        scan_for_child(Path::new("Z:/locks"), WRAPPER_PID, WRAPPER_START, probe)
    }

    fn found(id: &str) -> ScanOutcome {
        ScanOutcome::Found(Uuid::parse_str(id).unwrap())
    }

    // ── Adoption ────────────────────────────────────────────────────────────────────

    /// ★What this entry guards = 「래퍼 PID 하나만 대조하면 안 된다」 (do not compare only the wrapper PID)★: the PID asked is the wrapper but the holder is its
    /// child. Unless the comparison target is widened to the tree, `NotYet` comes out here.
    #[test]
    fn a_lock_held_by_our_childs_child_is_found() {
        let probe = FakeProbe::default().file(&format!("{ID_A}.lock"), &[codex()]);
        assert_eq!(scan(&probe), found(ID_A));
    }

    /// Caught even when the tree is one level deeper — a lock held by a tool launched by codex.
    #[test]
    fn a_lock_held_by_a_deeper_descendant_is_found() {
        let probe = FakeProbe::default()
            .with_tool()
            .file(&format!("{ID_A}.lock"), &[tool()]);
        assert_eq!(scan(&probe), found(ID_A));
    }

    /// A lock held by a process the enumeration did not see is not a candidate — there is no ground to say it is ours.
    #[test]
    fn a_holder_outside_the_reported_tree_is_not_a_candidate() {
        let probe = FakeProbe::default().file(&format!("{ID_A}.lock"), &[tool()]);
        assert_eq!(scan(&probe), ScanOutcome::NotYet);
    }

    /// If the tree is empty (the child died or the PID was reused) we do not even ask about files.
    #[test]
    fn an_empty_tree_short_circuits_to_not_yet() {
        let probe = FakeProbe::default()
            .with_no_tree()
            .file(&format!("{ID_A}.lock"), &[codex()]);
        assert_eq!(scan(&probe), ScanOutcome::NotYet);
    }

    /// If the root's identity is off, the tree itself does not stand — we do not call another party's PID tree ours.
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
            "시작시각 미상(0)이면 남는 것이 PID 단독 대조라 채택하지 않는다"
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
            "id 가 아닌 이름은 후보 수를 늘리지 않는다"
        );
    }

    #[test]
    fn a_held_lock_with_an_unparseable_name_is_not_a_candidate() {
        let probe = FakeProbe::default().file(".coordination.lock", &[codex()]);
        assert_eq!(scan(&probe), ScanOutcome::NotYet);
    }

    /// ★A uuid name that is not in canonical form is not a candidate★ — if accepted, one file we did not create raises the candidate count,
    /// flips the decision to `Ambiguous`, and that agent's recovery stops entirely.
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
                "{name} 이 후보로 올라왔다"
            );
        }
    }

    /// The uppercase hyphenated form is accepted — Windows file names do not distinguish case.
    #[test]
    fn an_uppercase_canonical_name_is_still_a_candidate() {
        let probe = FakeProbe::default().file(&format!("{}.lock", ID_A.to_uppercase()), &[codex()]);
        assert_eq!(scan(&probe), found(ID_A));
    }

    /// ★What we count is not files but matched **holders**★ — if two of our processes hold one lock, 「그 락이 누구 것인가」 (who owns that lock)
    /// has not been narrowed to one yet. Counting only files lets that state pass as `Found`.
    #[test]
    fn two_of_our_processes_on_one_lock_are_ambiguous() {
        let probe = FakeProbe::default()
            .with_tool()
            .file(&format!("{ID_A}.lock"), &[codex(), tool()]);
        assert_eq!(scan(&probe), ScanOutcome::Ambiguous(2));
    }

    /// Something held jointly by others does not raise the count — we count only **our** holders.
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

    /// The field that counts rounds — the fake clock raises it and the fake probe reads it, writing 「n 번째 바퀴에는 이것이 보인다」 (this is visible on round n).
    /// Both fakes looking at the same field is the whole of this bundle.
    type Ticks = Rc<Cell<usize>>;

    struct FakeClock {
        ticks: Ticks,
        /// The child is alive only up to this round count.
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

    /// A probe that shows a different folder state each round — when the list runs out the last state keeps going.
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
                .expect("적어도 한 바퀴는 적어 둔다")
        }
    }

    impl LockHolderProbe for ScriptedProbe {
        fn holders(&self, path: &Path) -> Vec<Holder> {
            self.round().holders(path)
        }
        /// ★Delegate **all** three methods — miss even one and only that spot silently goes to the real OS★.
        /// When this was actually missed, the tree query leaked to the default implementation, scanned the real process table with a fake PID,
        /// and since the answer was always empty-handed the loop never ended (measured 2026-09-21 — three items hung for over 60 seconds).
        fn our_processes(&self, root_pid: u32, root_start_time: u64) -> Vec<ProcessIdentity> {
            self.round().our_processes(root_pid, root_start_time)
        }
        fn entries(&self, dir: &Path) -> Vec<PathBuf> {
            self.round().entries(dir)
        }
    }

    /// A record verb that accumulates the strings it receives as is — the **same type** the assembly point hands over.
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
        assert_eq!(
            got,
            vec![ID_A.to_string()],
            "세 번째 바퀴에서 채택해야 한다"
        );
        assert_eq!(rounds, 3, "빈손 두 바퀴를 지나 세 번째에 멈춰야 한다");
    }

    /// ★A refusal, not a giving up (ADR-0218 decision 3 vs decision 4)★ — once the overlap clears, we adopt on the next round.
    /// If the thread were ended here, one short overlap at the moment the thread switches would end that agent's recovery
    /// for good.
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
            "겹침이 풀린 다음 바퀴에 채택해야 한다"
        );
        assert_eq!(rounds, 2, "겹친 바퀴에서 스레드를 끝내 버렸다");
    }

    /// If the overlap never clears, adoption never happens — the 「거절」 (refusal) side stays as is.
    #[test]
    fn an_ambiguous_scan_that_never_clears_adopts_nothing() {
        let (got, rounds) = drive(
            vec![FakeProbe::default()
                .file(&format!("{ID_A}.lock"), &[codex()])
                .file(&format!("{ID_B}.lock"), &[codex()])],
            |ticks| FakeClock::dies_after(ticks, 3),
        );
        assert!(got.is_empty(), "둘 이상인데 채택했다: {got:?}");
        assert_eq!(rounds, 4, "자식이 죽을 때까지는 계속 돌아야 한다");
    }

    #[test]
    fn the_child_exiting_ends_the_loop() {
        let (got, rounds) = drive(vec![FakeProbe::default()], |ticks| {
            FakeClock::dies_after(ticks, 2)
        });
        assert!(got.is_empty(), "자식이 죽었는데 채택했다: {got:?}");
        assert_eq!(rounds, 3, "생존 확인이 실패한 바퀴에서 끝나야 한다");
    }

    /// ★Adoption goes out through the handed-over verb **alone** — even if that verb discards the value there is no second path★.
    ///
    /// The fake used here has the same shape as a dead incarnation's record verb (it receives but leaves nothing in the profile).
    /// That the production verb really carries the incarnation guard is measured on the assembly point's side by
    /// `manager::tests::a_dead_incarnations_sink_cannot_overwrite_a_live_value` —
    /// ADR-0004 says `ProfileRegistry` does not enter the backend, so this spot is set up with a **refusing verb**.
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
            "기록 동사가 정확히 한 번 불려야 한다"
        );
        assert_eq!(clock.rounds(), 1, "거절을 재시도로 읽고 계속 돌면 안 된다");
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

    /// ★The plan carries the **handed-over** record verb as is★ — if another verb is invented here, the incarnation guard the assembly point
    /// bound goes missing entirely (the real guard is `manager::session_id_sink`).
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
        .expect("fresh 는 돈다");

        (made.sink)("probe");

        assert_eq!(
            *seen.lock().expect("sink poisoned"),
            vec!["probe".to_string()],
            "계획이 다른 동사를 들고 있다"
        );
    }

    #[test]
    fn a_fresh_incarnation_gets_a_plan() {
        let made = plan(&[], false, Some(WRAPPER_PID), Some(WRAPPER_START)).expect("fresh 는 돈다");
        assert_eq!(made.child_pid, WRAPPER_PID);
        assert_eq!(made.child_start_time, WRAPPER_START);
        assert_eq!(made.lock_dir, lock_dir_under(Path::new(OUR_HOME)));
    }

    /// ★What the gate looks at is 「argv 가 실제로 이어받았나」 (did argv actually resume)★ — not 「손잡이가 있나」 (is there a handle). The spot that
    /// tells the two apart (a handle that cannot be loaded) is measured by this folder's
    /// `tests::a_handle_we_refuse_to_resume_with_is_still_captured`.
    #[test]
    fn a_resume_incarnation_never_starts_the_loop() {
        assert!(
            plan(&[], true, Some(WRAPPER_PID), Some(WRAPPER_START)).is_none(),
            "이어받기 화신은 id 를 argv 로 이미 들고 나갔다 — 회수할 것이 없다"
        );
    }

    #[test]
    fn an_unknown_pid_or_start_time_never_starts_the_loop() {
        assert!(plan(&[], false, None, Some(WRAPPER_START)).is_none());
        assert!(
            plan(&[], false, Some(WRAPPER_PID), None).is_none(),
            "시작시각을 모르면 남는 것이 PID 단독 대조다"
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
            "받아도 남길 데가 없다"
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

    /// ★A `CODEX_HOME` set by the profile beats ours★ — that variable applies only to the process that sets it, so if we
    /// scan our folder the child's lock is never seen (empty-handed forever, with no error).
    #[test]
    fn the_lock_dir_comes_from_the_childs_environment() {
        let made = plan(
            &[("CODEX_HOME", "D:/alt-codex")],
            false,
            Some(WRAPPER_PID),
            Some(WRAPPER_START),
        )
        .expect("fresh 는 돈다");
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
        .expect("fresh 는 돈다");
        let expected = if env_key_eq("codex_home", CODEX_HOME_ENV) {
            "D:/third"
        } else {
            "D:/second"
        };
        assert_eq!(
            made.lock_dir,
            lock_dir_under(Path::new(expected)),
            "통로가 순서대로 env 를 밀어 넣으므로 자식이 받는 것은 같은 키로 보는 것 중 뒤엣것이다"
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
        .expect("fresh 는 돈다");
        assert_eq!(made.lock_dir, lock_dir_under(Path::new(OUR_HOME)));
    }

    // ── Coupling with the real probe (Windows) ──────────────────────────────────────

    /// One round with [`RestartManagerProbe`] instead of a fake — it finds the `<uuid>.lock` this process opened, pretending it is the child
    /// (asking with its own PID and start time). We put in the same folder a lock that is **not held** and a name that is not an id,
    /// so the list-side rules also pass through the real thing.
    #[cfg(windows)]
    #[test]
    fn the_real_probe_finds_a_lock_this_process_holds() {
        let dir = std::env::temp_dir().join(format!(
            "engram-thread-lock-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("임시 락 폴더 생성");
        std::fs::write(dir.join(format!("{ID_B}.lock")), b"").expect("안 쥔 락");
        std::fs::write(dir.join(".coordination.lock"), b"").expect("id 아닌 이름");
        let file = std::fs::File::create(dir.join(format!("{ID_A}.lock"))).expect("쥔 락");

        let me = std::process::id();
        let start = engram_dashboard_platform::process::process_creation_time(me)
            .expect("자기 creation time 조회 가능");
        let outcome = scan_for_child(&dir, me, start, &RestartManagerProbe);

        drop(file);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(outcome, found(ID_A));
    }
}
