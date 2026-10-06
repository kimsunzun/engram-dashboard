//! `cmd.exe /c <대상>` 의 대상 찾기 — [`super::MissingProgramCheck`] 의 Windows 갈래. cmd 와 같은 규칙(작업 폴더 →
//! `PATH`, 확장자 = `PATHEXT`)으로 찾고, 같은 대상의 찾기는 프로세스 전체에서 한 번에 하나다([`LookupGate`]).
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

/// cmd 의 내부 명령 — 이 이름은 파일로 찾지 않는다.
const CMD_BUILTINS: &[&str] = &[
    "assoc", "break", "call", "cd", "chdir", "cls", "color", "copy", "date", "del", "dir", "dpath",
    "echo", "endlocal", "erase", "exit", "for", "ftype", "goto", "if", "keys", "md", "mkdir",
    "mklink", "move", "path", "pause", "popd", "prompt", "pushd", "rd", "rem", "ren", "rename",
    "rmdir", "set", "setlocal", "shift", "start", "time", "title", "type", "ver", "verify", "vol",
];

/// `cmd.exe … /c <대상> …` 의 대상 — 프로그램 이름 한 낱말일 때만(공백·cmd 특수 문자·내부 명령이면 `None`).
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

/// 자식이 받을 env 의 값 — `env_set`(마지막이 이김) → `env_remove` → 이 프로세스 자신의 값 순. Windows 이름
/// 규칙대로 대소문자를 가리지 않는다.
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

/// `cmd.exe /c <대상>` 의 대상 찾기 — 자식이 받은 env(`PATH`·`PATHEXT`)와 작업 폴더를 기동 때 떠 둔다.
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

    /// ★원격 칸은 건너뛴다★([`is_remote`]) — 건너뛴 칸에만 있는 프로그램은 「없다」로 판정된다. 느린 원격 칸이
    ///   판정을 붙잡는 것보다 그쪽을 택했다(그 칸의 프로그램이 **실패했을 때만** 틀린다).
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
        // 정의됐지만 비었으면(`PATHEXT=`) 없는 것과 같게 기본 목록을 쓴다.
        if exts.is_empty() {
            exts = Self::DEFAULT_PATHEXT.split(';').collect();
        }
        // `symlink_metadata` = 재분석 지점을 따라가지 않는다 — 앱 실행 별칭(WindowsApps)은 따라가면 열리지 않는다.
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
        // 경로로 준 대상은 그 자리만 본다(상대 경로 = 작업 폴더 기준).
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
        // 빈 칸은 건너뛴다 · 상대 칸은 cmd 처럼 작업 폴더 기준이다(이 프로세스의 작업 폴더가 아니다).
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

/// 운영의 찾기 명단 — 프로세스 전체에 하나다([`run_bounded`]).
pub(super) static LOOKUPS_IN_FLIGHT: LookupGate = LookupGate::new();

/// 도는 중인 대상 찾기의 명단 — ★같은 대상의 찾기는 한 번에 하나, 대상이 다르면 서로 막지 않는다★. 대상 이름의
/// ASCII 대소문자는 가리지 않는다(Windows 이름은 대소문자를 안 가린다 — ASCII 밖은 접지 않는다).
///
/// ★프로세스에 하나인 표식으로 합치지 말 것★ — 판정은 나란히 돈다(agent 사용량 조회는 벤더마다 따로 판정한다).
///   두 대상이 다 없으면 두 셸이 몇 ms 안에 함께 실패하고, 뒤의 판정이 앞의 찾기에 막혀 「없다」 대신 「모른다」가
///   된다.
pub(super) struct LookupGate(Mutex<BTreeSet<String>>);

impl LookupGate {
    const fn new() -> Self {
        Self(Mutex::new(BTreeSet::new()))
    }

    /// `target` 의 자리를 잡았나 — 이미 도는 중이면 거짓. ★잠금이 독에 걸렸으면(앞선 패닉) 도는 중으로 친다★ —
    /// 패닉을 퍼뜨리지 않고 판정을 「모른다」로 둔다. 독은 걷지 않는다 — 그 뒤 이 명단의 판정은 늘 「모른다」다.
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

    /// 시험마다 제 명단 — 병렬로 도는 시험끼리 서로의 찾기를 「도는 중」으로 보지 않게(운영 명단은 하나다).
    #[cfg(test)]
    pub(super) fn own() -> &'static Self {
        Box::leak(Box::new(Self::new()))
    }
}

/// drop 되면 그 대상의 자리를 비운다 — 찾기가 패닉해도 자리가 영영 차 있지 않게.
struct InFlight {
    gate: &'static LookupGate,
    target: String,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.gate.release(&self.target);
    }
}

/// `resolve` 를 딴 스레드에서 돌리고 마감까지만 기다린다 — 파일 시스템 조회 하나가 원격 칸에서 오래 막혀도
/// (가르지 못한 네트워크 드라이브 등) 판정이 마감을 넘기지 않는다. 막힌 스레드는 그 조회가 풀릴 때 스스로 끝난다.
///
/// ★`target` 의 찾기가 도는 동안은 새로 띄우지 않고 곧바로 [`TargetLookup::Busy`] 다★([`LookupGate`]) — 마감을
///   넘겨 남은 스레드가 막혀 있는 동안 실패한 조회가 되풀이되면 막힌 스레드가 조회마다 하나씩 쌓인다. 그 사이의
///   판정은 「모른다」라 「없다」로 접히지 않는다.
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
                // 결과를 보내기 전에 비운다 — 받은 쪽이 곧바로 다음 판정을 해도 「아직 도는 중」으로 보이지 않게.
                drop(release);
                let _ = tx.send(found);
            }
        });
    if let Err(e) = started {
        // 몸체가 돌지 않았으니 자리를 비울 가드도 없다 — 여기서 비운다.
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

/// 원격일 수 있는 자리 — UNC(`\\서버\공유` · `\\?\UNC\…`)와 드라이브가 아닌 장치 이름공간(`\\.\…` · `\\?\…`).
/// ★드라이브 문자로 연결한 네트워크 드라이브는 못 가른다★ — 가르는 API(`GetDriveTypeW`)가 이 crate 가 켜 둔
///   `windows` 기능(`Win32_Storage_FileSystem`) 밖이다. 그런 칸이 막히면 [`CmdLookup::run_within`] 의 마감이 끊는다.
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

    /// 시험 하나의 임시 폴더 — drop 하면 지운다.
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
        // 상대 칸은 이 프로세스가 아니라 자식의 작업 폴더 기준이다.
        assert_eq!(
            lookup("engram-probe-tool", root.path(), &[("PATH", "bin")], &[]),
            TargetLookup::Found,
            "상대 PATH 칸"
        );
    }

    /// 정의됐지만 빈 `PATHEXT` 는 없는 것과 같다 — 기본 확장자 목록을 쓴다.
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

    /// 원격 칸은 건너뛴다 — 닿지 않는 UNC 칸이 판정을 붙잡지 않는다.
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

    /// 마감이 지나면 「모른다」다 — 「없다」가 아니다.
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

    /// ★같은 대상의 찾기는 한 번에 하나다★ — 마감을 넘겨 막힌 찾기가 남아 있는 동안 같은 대상(ASCII 대소문자
    /// 무시)의 판정은 새로 찾지 않고 「모른다」다. 다른 대상은 그 사이에도 찾는다.
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

    /// ★대상이 다르면 동시에 찾는다★ — 두 대상이 다 없어 두 판정이 겹쳐도 둘 다 「없다」에 닿는다. 각 찾기가
    /// 다른 쪽이 도는 중일 때까지 기다리므로, 한쪽이 막히면 다른 쪽도 마감에서 「모른다」가 된다.
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

    /// 독에 걸린 명단은 「도는 중」으로 친다 — 패닉하지 않고, 찾지도 않고 「모른다」다.
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
