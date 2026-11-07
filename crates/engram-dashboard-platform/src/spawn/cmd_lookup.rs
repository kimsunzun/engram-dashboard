//! Lookup of the target of `cmd.exe /c <대상>` — the Windows branch of [`super::MissingProgramCheck`]. It looks by the same rules as cmd (working folder →
//! `PATH`, extension = `PATHEXT`), and a lookup for the same target runs once at a time across the whole process ([`LookupGate`]).
// ADR-0266

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use super::TargetLookup;

pub(super) fn is_cmd_shell(program: &str) -> bool {
    Path::new(program)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case("cmd"))
}

/// cmd's internal commands — these names are not looked up as files.
const CMD_BUILTINS: &[&str] = &[
    "assoc", "break", "call", "cd", "chdir", "cls", "color", "copy", "date", "del", "dir", "dpath",
    "echo", "endlocal", "erase", "exit", "for", "ftype", "goto", "if", "keys", "md", "mkdir",
    "mklink", "move", "path", "pause", "popd", "prompt", "pushd", "rd", "rem", "ren", "rename",
    "rmdir", "set", "setlocal", "shift", "start", "time", "title", "type", "ver", "verify", "vol",
];

/// The target of `cmd.exe … /c <대상> …` — only when it is a one-word program name (`None` for spaces, cmd special characters or an internal command).
fn cmd_target<'a>(program: &str, args: &'a [String]) -> Option<&'a str> {
    if !is_cmd_shell(program) {
        return None;
    }
    let mut args = args.iter();
    args.find(|arg| arg.eq_ignore_ascii_case("/c"))?;
    let target = args.next()?.as_str();
    let plain = !target.is_empty()
        && !target
            .chars()
            .any(|c| c.is_whitespace() || "&|<>()^\"%!".contains(c));
    let builtin = CMD_BUILTINS
        .iter()
        .any(|name| name.eq_ignore_ascii_case(target));
    (plain && !builtin).then_some(target)
}

/// The value of the env the child will receive — in the order `env_set` (the last wins) → `env_remove` → this process's own value. Case-insensitive,
/// per Windows name rules.
fn child_env(env_set: &[(String, String)], env_remove: &[String], key: &str) -> Option<OsString> {
    if let Some((_, value)) = env_set
        .iter()
        .rev()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
    {
        return Some(value.into());
    }
    if env_remove.iter().any(|k| k.eq_ignore_ascii_case(key)) {
        return None;
    }
    std::env::var_os(key)
}

/// Lookup of the target of `cmd.exe /c <대상>` — the env the child received (`PATH` · `PATHEXT`) and the working folder are captured at startup.
#[derive(Clone)]
pub(super) struct CmdLookup {
    target: String,
    cwd: PathBuf,
    path: Option<OsString>,
    pathext: Option<OsString>,
}

impl CmdLookup {
    const DEFAULT_PATHEXT: &'static str = ".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC";

    pub(super) fn from_command(
        program: &str,
        args: &[String],
        env_set: &[(String, String)],
        env_remove: &[String],
        cwd: &Path,
    ) -> Option<Self> {
        Some(Self {
            target: cmd_target(program, args)?.to_owned(),
            cwd: cwd.to_path_buf(),
            path: child_env(env_set, env_remove, "PATH"),
            pathext: child_env(env_set, env_remove, "PATHEXT"),
        })
    }

    pub(super) fn run_within(&self, deadline: Instant, gate: &'static LookupGate) -> TargetLookup {
        let lookup = self.clone();
        run_bounded(deadline, gate, &self.target, move || {
            lookup.resolve(deadline)
        })
    }

    /// ★Remote entries are skipped★ ([`is_remote`]) — a program that exists only in a skipped entry is decided 「없다」 (missing). We chose that over
    ///   a slow remote entry holding up the decision (it is wrong **only when the program in that entry failed**).
    fn resolve(&self, deadline: Instant) -> TargetLookup {
        let pathext = self
            .pathext
            .as_ref()
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut exts: Vec<&str> = pathext
            .split(';')
            .map(str::trim)
            .filter(|ext| !ext.is_empty())
            .collect();
        // If defined but empty (`PATHEXT=`), use the default list as if it were absent.
        if exts.is_empty() {
            exts = Self::DEFAULT_PATHEXT.split(';').collect();
        }
        // `symlink_metadata` = does not follow reparse points — an app execution alias (WindowsApps) cannot be opened if followed.
        let present = |path: &Path| std::fs::symlink_metadata(path).is_ok_and(|m| !m.is_dir());
        let found_in = |dir: &Path| {
            let exact = dir.join(&self.target);
            (exact.extension().is_some() && present(&exact))
                || exts.iter().any(|ext| {
                    let mut name = OsString::from(&self.target);
                    name.push(ext);
                    present(&dir.join(name))
                })
        };
        let verdict = |found: bool| {
            if found {
                TargetLookup::Found
            } else {
                TargetLookup::Missing
            }
        };
        // A target given as a path is looked at in that place only (relative path = relative to the working folder).
        if self.target.contains(['\\', '/', ':']) {
            if is_remote(&self.cwd.join(&self.target)) {
                return TargetLookup::Unknown;
            }
            return verdict(found_in(&self.cwd));
        }
        if !is_remote(&self.cwd) && found_in(&self.cwd) {
            return TargetLookup::Found;
        }
        let Some(path) = &self.path else {
            return TargetLookup::Missing;
        };
        // Empty entries are skipped · relative entries are relative to the working folder, like cmd (not this process's working folder).
        for entry in std::env::split_paths(path) {
            if entry.as_os_str().is_empty() {
                continue;
            }
            let dir = self.cwd.join(&entry);
            if is_remote(&dir) {
                continue;
            }
            if Instant::now() >= deadline {
                return TargetLookup::Unknown;
            }
            if found_in(&dir) {
                return TargetLookup::Found;
            }
        }
        TargetLookup::Missing
    }
}

/// The production lookup list — one across the whole process ([`run_bounded`]).
pub(super) static LOOKUPS_IN_FLIGHT: LookupGate = LookupGate::new();

/// The list of target lookups in progress — ★a lookup for the same target runs once at a time; different targets do not block each other★. The target name's
/// ASCII case is ignored (Windows names are case-insensitive — non-ASCII is not folded).
///
/// ★Do not merge it into a one-per-process marker★ — decisions run side by side (the agent usage query decides separately per vendor).
///   If both targets are missing, two shells fail together within a few ms, and the later decision is blocked by the earlier lookup and becomes 「모른다」 (unknown) instead of
///   「없다」 (missing).
pub(super) struct LookupGate(Mutex<BTreeSet<String>>);

impl LookupGate {
    const fn new() -> Self {
        Self(Mutex::new(BTreeSet::new()))
    }

    /// Did it claim the slot for `target` — false if one is already running. ★If the lock is poisoned (an earlier panic), it counts as running★ —
    /// it does not spread the panic and leaves the decision 「모른다」 (unknown). The poison is not cleared — afterwards decisions on this list are always 「모른다」 (unknown).
    fn claim(&self, target: &str) -> bool {
        match self.0.lock() {
            Ok(mut running) => running.insert(target.to_ascii_lowercase()),
            Err(_) => false,
        }
    }

    fn release(&self, target: &str) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&target.to_ascii_lowercase());
    }

    /// Each test gets its own list — so tests running in parallel do not see each other's lookups as 「도는 중」 (running) (production has one list).
    #[cfg(test)]
    pub(super) fn own() -> &'static Self {
        Box::leak(Box::new(Self::new()))
    }
}

/// When dropped it frees that target's slot — so the slot is not left occupied forever even if the lookup panics.
struct InFlight {
    gate: &'static LookupGate,
    target: String,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.gate.release(&self.target);
    }
}

/// Runs `resolve` on another thread and waits only until the deadline — even if one file-system query blocks for a long time on a remote entry
/// (a network drive that cannot be told apart etc.), the decision does not go past the deadline. The stuck thread ends by itself when that query is released.
///
/// ★While a lookup for `target` is running, it does not launch a new one but is [`TargetLookup::Busy`] right away★ ([`LookupGate`]) — if failed lookups
///   repeat while a thread left past the deadline is stuck, one stuck thread would pile up per lookup. The decision in between
///   is 「모른다」 (unknown), so it does not fold into 「없다」 (missing).
fn run_bounded(
    deadline: Instant,
    gate: &'static LookupGate,
    target: &str,
    resolve: impl FnOnce() -> TargetLookup + Send + 'static,
) -> TargetLookup {
    let Some(left) = remaining(deadline) else {
        return TargetLookup::Unknown;
    };
    if !gate.claim(target) {
        return TargetLookup::Busy;
    }
    let (tx, rx) = mpsc::channel();
    let started = thread::Builder::new()
        .name("cmd-target-lookup".to_owned())
        .spawn({
            let target = target.to_owned();
            move || {
                let release = InFlight { gate, target };
                let found = resolve();
                // Free it before sending the result — so that even if the receiver makes the next decision right away it does not look like 「아직 도는 중」 (still running).
                drop(release);
                let _ = tx.send(found);
            }
        });
    if let Err(e) = started {
        // The body did not run, so there is no guard to free the slot either — free it here.
        gate.release(target);
        return TargetLookup::NoThread(e.to_string());
    }
    rx.recv_timeout(left).unwrap_or(TargetLookup::Unknown)
}

fn remaining(deadline: Instant) -> Option<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
}

/// A location that may be remote — UNC (`\\서버\공유` · `\\?\UNC\…`) and non-drive device namespaces (`\\.\…` · `\\?\…`).
/// ★A network drive connected by drive letter cannot be told apart★ — the API that tells them apart (`GetDriveTypeW`) lives in the `windows` feature
///   (`Win32_Storage_FileSystem`), which this crate does not turn on. If such an entry blocks, the deadline of [`CmdLookup::run_within`] cuts it.
fn is_remote(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    match path.components().next() {
        Some(Component::Prefix(prefix)) => {
            !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use super::*;

    const FAR: Duration = Duration::from_secs(20);

    fn far() -> Instant {
        Instant::now() + FAR
    }

    /// A temp folder for one test — deleted when dropped.
    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "engram-platform-{label}-{}",
                uuid::Uuid::new_v4().simple()
            ));
            std::fs::create_dir_all(&path).expect("시험 임시 루트");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn cmd_c_args(target: &str) -> Vec<String> {
        vec!["/c".to_owned(), target.to_owned(), "--flag".to_owned()]
    }

    fn eventually(what: &str, mut check: impl FnMut() -> bool) {
        let until = Instant::now() + Duration::from_secs(5);
        while !check() {
            assert!(Instant::now() < until, "5초 안에 안 됐다: {what}");
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn cmd_target_is_only_a_plain_program_word() {
        let target = |args: &[&str]| {
            let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
            cmd_target("cmd.exe", &args).map(str::to_owned)
        };
        assert_eq!(target(&["/c", "tool", "--a"]).as_deref(), Some("tool"));
        assert_eq!(target(&["/d", "/C", "tool"]).as_deref(), Some("tool"));
        assert_eq!(target(&["/c", "echo", "hi"]), None, "내부 명령");
        assert_eq!(target(&["/c", "EXIT"]), None, "내부 명령(대소문자)");
        assert_eq!(target(&["/c", "set /p L="]), None, "한 낱말이 아니다");
        assert_eq!(target(&["/c", "a&b"]), None, "cmd 특수 문자");
        assert_eq!(target(&["/k", "tool"]), None);
        assert_eq!(
            cmd_target("other.exe", &cmd_c_args("tool")),
            None,
            "cmd 가 아니다"
        );
    }

    fn lookup(
        target: &str,
        cwd: &Path,
        env_set: &[(&str, &str)],
        env_remove: &[&str],
    ) -> TargetLookup {
        let env_set: Vec<(String, String)> = env_set
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let env_remove: Vec<String> = env_remove.iter().map(|k| (*k).to_owned()).collect();
        CmdLookup::from_command("cmd.exe", &cmd_c_args(target), &env_set, &env_remove, cwd)
            .expect("한 낱말 대상")
            .resolve(far())
    }

    #[test]
    fn cmd_target_resolution_follows_the_child_env() {
        let root = TempRoot::new("probe-resolve");
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).expect("bin");
        std::fs::write(bin.join("engram-probe-tool.cmd"), "@exit 0").expect("shim");
        let cwd = root.path().join("cwd");
        std::fs::create_dir(&cwd).expect("cwd");
        std::fs::write(cwd.join("engram-local-tool.exe"), "").expect("local");
        let bin_path = bin.to_string_lossy().into_owned();
        let path = [("PATH", bin_path.as_str())];

        assert_eq!(
            lookup("engram-probe-tool", &cwd, &path, &[]),
            TargetLookup::Found,
            "PATH × PATHEXT"
        );
        assert_eq!(
            lookup("engram-probe-tool.cmd", &cwd, &path, &[]),
            TargetLookup::Found,
            "확장자까지 준 이름"
        );
        assert_eq!(
            lookup("engram-local-tool", &cwd, &path, &[]),
            TargetLookup::Found,
            "작업 폴더 먼저"
        );
        assert_eq!(
            lookup(
                "engram-probe-tool",
                &cwd,
                &[path[0], ("PATHEXT", ".EXE")],
                &[]
            ),
            TargetLookup::Missing,
            "PATHEXT 밖의 확장자"
        );
        assert_eq!(
            lookup("engram-probe-tool", &cwd, &[], &["path"]),
            TargetLookup::Missing,
            "벗긴 PATH"
        );
        let absolute = bin.join("engram-probe-tool.cmd");
        assert_eq!(
            lookup(&absolute.to_string_lossy(), &cwd, &[], &["path"]),
            TargetLookup::Found,
            "경로로 준 대상"
        );
        // Relative entries are relative to the child's working folder, not this process's.
        assert_eq!(
            lookup("engram-probe-tool", root.path(), &[("PATH", "bin")], &[]),
            TargetLookup::Found,
            "상대 PATH 칸"
        );
    }

    /// A defined but empty `PATHEXT` is the same as absent — the default extension list is used.
    #[test]
    fn an_empty_pathext_means_the_default_list() {
        let root = TempRoot::new("probe-empty-pathext");
        std::fs::write(root.path().join("engram-probe-tool.cmd"), "@exit 0").expect("shim");
        for pathext in ["", " ", ";;"] {
            assert_eq!(
                lookup(
                    "engram-probe-tool",
                    root.path(),
                    &[("PATHEXT", pathext)],
                    &["PATH"]
                ),
                TargetLookup::Found,
                "{pathext:?}"
            );
        }
    }

    /// Remote entries are skipped — an unreachable UNC entry does not hold up the decision.
    #[test]
    fn remote_path_entries_are_skipped() {
        for remote in [
            r"\\engram-unreachable.invalid\share\bin",
            r"\\?\UNC\engram-unreachable.invalid\share",
            r"\\.\pipe\engram",
            r"\\?\Volume{00000000-0000-0000-0000-000000000000}\bin",
        ] {
            assert!(is_remote(Path::new(remote)), "{remote}");
        }
        for local in [
            r"C:\Windows",
            r"\\?\C:\Windows",
            r"relative\bin",
            r"\rooted",
        ] {
            assert!(!is_remote(Path::new(local)), "{local}");
        }

        let root = TempRoot::new("probe-remote-skip");
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).expect("bin");
        std::fs::write(bin.join("engram-probe-tool.cmd"), "@exit 0").expect("shim");
        let path = format!(
            r"\\engram-unreachable.invalid\share;{}",
            bin.to_string_lossy()
        );
        let started = Instant::now();
        assert_eq!(
            lookup(
                "engram-probe-tool",
                root.path(),
                &[("PATH", path.as_str())],
                &[]
            ),
            TargetLookup::Found
        );
        assert_eq!(
            lookup(
                "engram-no-such-tool",
                root.path(),
                &[("PATH", path.as_str())],
                &[]
            ),
            TargetLookup::Missing,
            "건너뛴 칸은 찾은 셈 치지 않는다"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "원격 칸을 건너뛰지 않았다: {:?}",
            started.elapsed()
        );
    }

    /// Past the deadline it is 「모른다」 (unknown) — not 「없다」 (missing).
    #[test]
    fn a_lookup_out_of_time_is_unknown() {
        let root = TempRoot::new("probe-lookup-late");
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).expect("bin");
        let env_set = [("PATH".to_owned(), bin.to_string_lossy().into_owned())];
        let lookup = CmdLookup::from_command(
            "cmd.exe",
            &cmd_c_args("engram-no-such-tool"),
            &env_set,
            &[],
            root.path(),
        )
        .expect("한 낱말 대상");
        let gate = LookupGate::own();
        assert_eq!(
            lookup.run_within(Instant::now(), gate),
            TargetLookup::Unknown
        );
        assert_eq!(lookup.resolve(Instant::now()), TargetLookup::Unknown);
        assert_eq!(lookup.run_within(far(), gate), TargetLookup::Missing);
    }

    /// ★A lookup for the same target runs once at a time★ — while a lookup stuck past the deadline remains, a decision on the same target (ASCII case
    /// ignored) does not look anew and is 「모른다」 (unknown). Other targets are still looked up in the meantime.
    #[test]
    fn a_lookup_still_in_flight_keeps_a_second_one_for_the_same_target_from_starting() {
        let gate = LookupGate::own();
        let (release_tx, release) = mpsc::channel::<()>();
        let first = run_bounded(
            Instant::now() + Duration::from_millis(200),
            gate,
            "engram-tool",
            move || {
                let _ = release.recv();
                TargetLookup::Missing
            },
        );
        assert_eq!(
            first,
            TargetLookup::Unknown,
            "막힌 찾기는 마감에서 「모른다」다"
        );

        let ran = Arc::new(AtomicBool::new(false));
        let second = {
            let ran = ran.clone();
            run_bounded(far(), gate, "ENGRAM-TOOL", move || {
                ran.store(true, Ordering::SeqCst);
                TargetLookup::Missing
            })
        };
        assert_eq!(second, TargetLookup::Busy);
        assert!(
            !ran.load(Ordering::SeqCst),
            "같은 대상의 앞선 찾기가 도는데 새로 찾았다"
        );
        assert_eq!(
            run_bounded(far(), gate, "engram-other-tool", || TargetLookup::Missing),
            TargetLookup::Missing,
            "다른 대상은 막히지 않는다"
        );

        drop(release_tx);
        eventually("막힌 찾기가 풀린 뒤의 다음 찾기", || {
            run_bounded(far(), gate, "engram-tool", || TargetLookup::Missing)
                == TargetLookup::Missing
        });
    }

    /// ★Different targets are looked up at the same time★ — even if two decisions overlap with both targets missing, both reach 「없다」 (missing). If each lookup
    /// waited until the other was done running, one being blocked would make the other 「모른다」 (unknown) at the deadline too.
    #[test]
    fn lookups_for_different_targets_run_side_by_side() {
        let gate = LookupGate::own();
        let both_running = Arc::new(std::sync::Barrier::new(2));
        let judge = move |target: &'static str| {
            let both_running = both_running.clone();
            thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                run_bounded(deadline, gate, target, move || {
                    both_running.wait();
                    TargetLookup::Missing
                })
            })
        };
        let (first, second) = (judge("engram-tool-a"), judge("engram-tool-b"));
        assert_eq!(first.join().expect("첫 판정"), TargetLookup::Missing);
        assert_eq!(second.join().expect("둘째 판정"), TargetLookup::Missing);
    }

    /// A poisoned list counts as 「도는 중」 (running) — it does not panic, does not look, and is 「모른다」 (unknown).
    #[test]
    fn a_poisoned_gate_means_unknown_without_a_panic() {
        let gate = LookupGate::own();
        let _ = thread::spawn(move || {
            let _held = gate.0.lock();
            panic!("명단을 쥔 채 패닉한다");
        })
        .join();
        assert!(gate.0.is_poisoned());
        let ran = Arc::new(AtomicBool::new(false));
        let found = {
            let ran = ran.clone();
            run_bounded(far(), gate, "engram-tool", move || {
                ran.store(true, Ordering::SeqCst);
                TargetLookup::Missing
            })
        };
        assert_eq!(found, TargetLookup::Busy);
        assert!(!ran.load(Ordering::SeqCst), "독에 걸린 명단에서 찾았다");
    }
}
