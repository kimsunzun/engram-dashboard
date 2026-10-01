//! 실물 조회 스포너 — [`ProbeSpawner`] 를 OS 프로세스로 구현한다.
//!
//! ★조회 쪽 OS 분기는 이 모듈 안에만 있다★: 기동 플래그(Windows = 콘솔 창 없음 + 멈춘 채 띄워 Job 에 넣은 뒤
//!   깨움 · POSIX = 새 프로세스 그룹) · 트리 kill(Windows = KILL_ON_JOB_CLOSE Job Object · POSIX = 그룹 전체에
//!   SIGKILL) · 「프로그램 없음」 판정(POSIX = 기동 `NotFound` · Windows = 실패한 `cmd.exe /c <대상>` 의 종료
//!   코드 9009 또는 그 대상 찾기 — [`NotInstalledCheck`]). 부르는 쪽(벤더 조회기)은 OS 를 모른다.
//!
//! 스레드 셋이 자식 하나를 받친다 — 아무도 join 하지 않으므로 drop 이 [`KILL_WAIT`] 보다 오래 걸리지 않는다:
//!   - stdin 라이터: stdin 을 닫을 때·자식 값이 drop 될 때(송신단이 사라질 때) 또는 쓰기가 실패할 때 끝난다.
//!     OS 쓰기에 막혀 있으면 트리 kill 이 파이프를 끊어 풀어 준다.
//!   - stdout 줄 리더: EOF(쓰기 끝을 쥔 프로세스가 모두 끝남) 또는 받는 쪽이 사라진 뒤의 첫 전달에서 끝난다.
//!     ★트리 kill 을 빠져나간 프로세스가 stdout 을 쥐고 있으면 그것이 끝날 때까지 이 스레드가 남는다★.
//!   - stderr 드레인: EOF 에서 끝난다. 내용은 버리고 가린 꼬리 몇 줄만 붙든다(로그에는 줄 수만).
//!
//! ★POSIX 갈래는 이 저장소에서 컴파일된 적이 없다★(개발·CI 가 Windows 뿐이다). 그룹 kill 은 새 의존 없이
//!   std 만으로 하려고 POSIX `kill` 유틸리티를 띄운다.
// ADR-0230

#[cfg(windows)]
use std::collections::BTreeSet;
use std::collections::VecDeque;
#[cfg(windows)]
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Read, Write};
#[cfg(windows)]
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, Command, Stdio};
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use engram_dashboard_base::logging::mask_secrets;

use super::{ExitInfo, ProbeChild, ProbeCommand, ProbeError, ProbeSpawner};

/// kill 뒤 자식(Windows 는 Job 안 전부)이 끝나기를 기다리는 상한. 임시 폴더를 지우기 전에 파일 잠금이
/// 풀리기를 기다리는 몫이다(Windows 는 열린 파일·작업 폴더를 못 지운다).
pub const KILL_WAIT: Duration = Duration::from_secs(2);

const WAIT_POLL: Duration = Duration::from_millis(20);

/// stdout 한 줄의 상한. 조회 응답은 몇 KB 이고 가장 긴 줄(초기화 알림)도 이보다 훨씬 작다.
const STDOUT_LINE_MAX: usize = 1024 * 1024;
/// 아직 안 읽은 stdout 줄의 상한 — 차면 리더가 멈추고 자식이 파이프에 막힌다(메모리 상한).
const STDOUT_QUEUE: usize = 16;

const STDERR_READ_MAX: usize = 4096;
const STDERR_TAIL_LINES: usize = 8;
const STDERR_TAIL_CHARS: usize = 240;

/// 실제 OS 프로세스를 띄우는 스포너.
#[derive(Debug, Clone, Copy, Default)]
pub struct OsProbeSpawner;

impl ProbeSpawner for OsProbeSpawner {
    fn spawn(
        &self,
        cmd: &ProbeCommand,
        deadline: Instant,
    ) -> Result<Box<dyn ProbeChild>, ProbeError> {
        Ok(Box::new(OsProbeChild::spawn(cmd, deadline)?))
    }
}

// ── OS 분기 ──────────────────────────────────────────────────────────────────────

/// `cmd.exe` 가 「내부 또는 외부 명령 … 이 아닙니다」로 끝날 때의 종료 코드.
#[cfg(windows)]
const CMD_NOT_FOUND_EXIT: i32 = 9009;

fn configure_os(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // 데몬은 창 없는 프로세스일 수 있다 — 콘솔 프로그램이 새 콘솔 창을 띄우는 깜빡임을 막는다.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        // ★멈춘 채 띄운다★ — Job 에 넣은 뒤에 깨운다([`ProcessTree::start`]). Job 은 그 안에 든 **뒤에** 만들어진
        //   프로세스에만 물려진다 — 깨운 채 띄우면 넣기 전에 자식이 띄운 손자가 트리 kill 을 빠져나간다.
        const CREATE_SUSPENDED: u32 = 0x0000_0004;
        command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 그룹 id = 자식 pid — 트리 kill 이 그룹 전체에 신호를 보낸다.
        command.process_group(0);
    }
}

#[cfg(windows)]
fn is_cmd_shell(program: &str) -> bool {
    Path::new(program)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case("cmd"))
}

/// 「띄우려던 프로그램이 없다」 판정 — 기동 때 명령에서 필요한 것을 떠 두고, 0 이 아닌 종료를 받았을 때 판정한다.
///
/// ★Windows 에서 종료 코드만으로는 못 가른다★ — `cmd.exe /c <없는 이름>` 의 종료 코드가 빌드마다 다르다
///   (실측 2026-09-27, Windows 11 26200 = **1**, 그 안의 errorlevel 은 9009 · 널리 알려진 값은 9009). 1 은
///   대상 프로그램 자신의 실패와 구별되지 않는다. 그래서 9009 가 아닌 실패면 `cmd.exe /c <대상> …` 의 대상을 cmd 와
///   같은 규칙(작업 폴더 → `PATH`, 확장자 = `PATHEXT`)으로 찾아보고, 없을 때만 「없다」다. 대상을 가려낼 수 없는
///   모양이면 9009 만 남는다.
/// ★띄우기 전에 찾지 않는다 — 되살리지 말 것★: 응답 없는 원격 `PATH` 칸 하나가 기동을 마감 너머까지 붙잡았고
///   (실측 21초), cmd 의 AutoRun 이 `PATH` 를 바꾸면 띄우기 전의 판정은 늘 틀린다. 실패 뒤의 판정도 같은 env 를
///   보므로 AutoRun 이 `PATH` 를 바꾼 PC 에서는 틀릴 수 있다 — 단 대상이 실제로 실패했을 때뿐이다.
/// POSIX 는 기동이 `NotFound` 로 실패하므로([`OsProbeChild::spawn`]) 여기서는 늘 거짓.
#[cfg(windows)]
struct NotInstalledCheck {
    through_cmd: bool,
    lookup: Option<CmdLookup>,
    /// 운영 = [`LOOKUPS_IN_FLIGHT`]. 시험은 제 것을 넣는다 — 병렬로 도는 시험끼리 서로의 찾기를 막지 않게.
    gate: &'static LookupGate,
}

#[cfg(windows)]
impl NotInstalledCheck {
    fn new(cmd: &ProbeCommand) -> Self {
        Self::with_gate(cmd, &LOOKUPS_IN_FLIGHT)
    }

    fn with_gate(cmd: &ProbeCommand, gate: &'static LookupGate) -> Self {
        Self {
            through_cmd: is_cmd_shell(&cmd.program),
            lookup: CmdLookup::from_command(cmd),
            gate,
        }
    }

    /// `code` 로 끝난 자식이 「없다」였나 — 모르면([`Lookup::Unknown`] — 마감 · 같은 대상의 앞선 찾기가 아직
    /// 도는 중 등) 거짓.
    fn judge(&self, code: Option<i32>, deadline: Instant) -> bool {
        if !self.through_cmd || code == Some(0) {
            return false;
        }
        if code == Some(CMD_NOT_FOUND_EXIT) {
            return true;
        }
        let Some(lookup) = &self.lookup else {
            return false;
        };
        let found = lookup.run_within(deadline, self.gate);
        tracing::debug!(?found, "사용량 조회: 실패한 셸의 대상을 찾아봤다");
        found == Lookup::Missing
    }
}

#[cfg(not(windows))]
struct NotInstalledCheck;

#[cfg(not(windows))]
impl NotInstalledCheck {
    fn new(_cmd: &ProbeCommand) -> Self {
        Self
    }

    fn judge(&self, _code: Option<i32>, _deadline: Instant) -> bool {
        false
    }
}

/// cmd 의 내부 명령 — 이 이름은 파일로 찾지 않는다.
#[cfg(windows)]
const CMD_BUILTINS: &[&str] = &[
    "assoc", "break", "call", "cd", "chdir", "cls", "color", "copy", "date", "del", "dir", "dpath",
    "echo", "endlocal", "erase", "exit", "for", "ftype", "goto", "if", "keys", "md", "mkdir",
    "mklink", "move", "path", "pause", "popd", "prompt", "pushd", "rd", "rem", "ren", "rename",
    "rmdir", "set", "setlocal", "shift", "start", "time", "title", "type", "ver", "verify", "vol",
];

/// `cmd.exe … /c <대상> …` 의 대상 — 프로그램 이름 한 낱말일 때만(공백·cmd 특수 문자·내부 명령이면 `None`).
#[cfg(windows)]
fn cmd_target(cmd: &ProbeCommand) -> Option<&str> {
    if !is_cmd_shell(&cmd.program) {
        return None;
    }
    let mut args = cmd.args.iter();
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

/// 자식이 받을 env 의 값 — `env_set`(마지막이 이김) → `env_remove` → 데몬 자신의 값 순. Windows 이름 규칙대로
/// 대소문자를 가리지 않는다.
#[cfg(windows)]
fn child_env(cmd: &ProbeCommand, key: &str) -> Option<OsString> {
    if let Some((_, value)) = cmd
        .env_set
        .iter()
        .rev()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
    {
        return Some(value.into());
    }
    if cmd.env_remove.iter().any(|k| k.eq_ignore_ascii_case(key)) {
        return None;
    }
    std::env::var_os(key)
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lookup {
    Found,
    Missing,
    /// 모른다 — 「없다」로 접지 않는다. 마감 안에 다 못 찾았거나, 같은 대상의 앞선 찾기가 아직 돌고 있어 새로
    /// 찾지 않았거나([`run_bounded`] — 대상마다 한 번에 하나), 경로로 준 대상이 원격 자리다.
    Unknown,
}

/// `cmd.exe /c <대상>` 의 대상 찾기 — 자식이 받은 env(`PATH`·`PATHEXT`)와 작업 폴더를 기동 때 떠 둔다.
#[cfg(windows)]
#[derive(Clone)]
struct CmdLookup {
    target: String,
    cwd: PathBuf,
    path: Option<OsString>,
    pathext: Option<OsString>,
}

#[cfg(windows)]
impl CmdLookup {
    const DEFAULT_PATHEXT: &'static str = ".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC";

    fn from_command(cmd: &ProbeCommand) -> Option<Self> {
        Some(Self {
            target: cmd_target(cmd)?.to_owned(),
            cwd: cmd.cwd.clone(),
            path: child_env(cmd, "PATH"),
            pathext: child_env(cmd, "PATHEXT"),
        })
    }

    fn run_within(&self, deadline: Instant, gate: &'static LookupGate) -> Lookup {
        let lookup = self.clone();
        run_bounded(deadline, gate, &self.target, move || {
            lookup.resolve(deadline)
        })
    }

    /// ★원격 칸은 건너뛴다★([`is_remote`]) — 건너뛴 칸에만 있는 프로그램은 「없다」로 판정된다. 느린 원격 칸이
    ///   판정을 붙잡는 것보다 그쪽을 택했다(그 칸의 프로그램이 **실패했을 때만** 틀린다).
    fn resolve(&self, deadline: Instant) -> Lookup {
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
                Lookup::Found
            } else {
                Lookup::Missing
            }
        };
        // 경로로 준 대상은 그 자리만 본다(상대 경로 = 작업 폴더 기준).
        if self.target.contains(['\\', '/', ':']) {
            if is_remote(&self.cwd.join(&self.target)) {
                return Lookup::Unknown;
            }
            return verdict(found_in(&self.cwd));
        }
        if !is_remote(&self.cwd) && found_in(&self.cwd) {
            return Lookup::Found;
        }
        let Some(path) = &self.path else {
            return Lookup::Missing;
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
                return Lookup::Unknown;
            }
            if found_in(&dir) {
                return Lookup::Found;
            }
        }
        Lookup::Missing
    }
}

/// 운영의 찾기 명단 — 프로세스 전체에 하나다([`run_bounded`]).
#[cfg(windows)]
static LOOKUPS_IN_FLIGHT: LookupGate = LookupGate::new();

/// 도는 중인 찾기에 막혀 판정을 「모른다」로 둔 것을 경고했나 — 경고는 프로세스당 한 번이고 그 뒤는 debug 다.
#[cfg(windows)]
static STUCK_LOOKUP_WARNED: AtomicBool = AtomicBool::new(false);

/// 도는 중인 대상 찾기의 명단 — ★같은 대상의 찾기는 한 번에 하나, 대상이 다르면 서로 막지 않는다★. 대상 이름의
/// ASCII 대소문자는 가리지 않는다(Windows 이름은 대소문자를 안 가린다 — ASCII 밖은 접지 않는다).
///
/// ★프로세스에 하나인 표식으로 합치지 말 것★ — 벤더 조회는 나란히 돈다. 두 CLI 가 다 없으면 두 셸이 몇 ms 안에
///   함께 실패하고, 뒤의 판정이 앞의 찾기에 막혀 그 벤더가 「미설치」 대신 「조회 실패」로 보인다.
#[cfg(windows)]
struct LookupGate(Mutex<BTreeSet<String>>);

#[cfg(windows)]
impl LookupGate {
    const fn new() -> Self {
        Self(Mutex::new(BTreeSet::new()))
    }

    /// `target` 의 자리를 잡았나 — 이미 도는 중이면 거짓. ★잠금이 독에 걸렸으면(앞선 패닉) 도는 중으로 친다★ —
    /// 패닉을 퍼뜨리지 않고 판정을 「모른다」로 둔다.
    fn claim(&self, target: &str) -> bool {
        match self.0.lock() {
            Ok(mut running) => running.insert(target.to_ascii_lowercase()),
            Err(_) => false,
        }
    }

    fn release(&self, target: &str) {
        lock(&self.0).remove(&target.to_ascii_lowercase());
    }
}

/// drop 되면 그 대상의 자리를 비운다 — 찾기가 패닉해도 자리가 영영 차 있지 않게.
#[cfg(windows)]
struct InFlight {
    gate: &'static LookupGate,
    target: String,
}

#[cfg(windows)]
impl Drop for InFlight {
    fn drop(&mut self) {
        self.gate.release(&self.target);
    }
}

/// `resolve` 를 딴 스레드에서 돌리고 마감까지만 기다린다 — 파일 시스템 조회 하나가 원격 칸에서 오래 막혀도
/// (가르지 못한 네트워크 드라이브 등) 판정이 마감을 넘기지 않는다. 막힌 스레드는 그 조회가 풀릴 때 스스로 끝난다.
///
/// ★`target` 의 찾기가 도는 동안은 새로 띄우지 않고 곧바로 [`Lookup::Unknown`] 이다★([`LookupGate`]) — 마감을
///   넘겨 남은 스레드가 막혀 있는 동안 실패한 조회가 되풀이되면 막힌 스레드가 조회마다 하나씩 쌓인다. 그 사이의
///   판정은 「모른다」라 「없다」로 접히지 않는다.
#[cfg(windows)]
fn run_bounded(
    deadline: Instant,
    gate: &'static LookupGate,
    target: &str,
    resolve: impl FnOnce() -> Lookup + Send + 'static,
) -> Lookup {
    let Some(left) = remaining(deadline) else {
        return Lookup::Unknown;
    };
    if !gate.claim(target) {
        // 보통은 앞선 조회의 찾기가 마감을 넘겨 막힌 채 남은 것이다 — 비정상이나 안전한 폴백(「모른다」).
        if STUCK_LOOKUP_WARNED.swap(true, Ordering::Relaxed) {
            tracing::debug!(
                "사용량 조회: 같은 대상의 앞선 찾기가 아직 끝나지 않아 새로 찾지 않는다"
            );
        } else {
            tracing::warn!(
                "사용량 조회: 같은 대상의 앞선 「미설치」 찾기가 아직 끝나지 않아 판정을 「모른다」로 둔다 — 막힌 파일 시스템 조회가 남았을 수 있다(이 경고는 프로세스당 한 번)"
            );
        }
        return Lookup::Unknown;
    }
    let (tx, rx) = mpsc::channel();
    let started = thread::Builder::new()
        .name("usage-probe-lookup".to_owned())
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
        tracing::debug!("사용량 조회: 대상 찾기 스레드를 띄우지 못했다: {e}");
        return Lookup::Unknown;
    }
    rx.recv_timeout(left).unwrap_or(Lookup::Unknown)
}

/// 원격일 수 있는 자리 — UNC(`\\서버\공유` · `\\?\UNC\…`)와 드라이브가 아닌 장치 이름공간(`\\.\…` · `\\?\…`).
/// ★드라이브 문자로 연결한 네트워크 드라이브는 못 가른다★ — 가르는 API(`GetDriveTypeW`)가 이 crate 가 켜 둔
///   `windows` 기능(`Win32_Storage_FileSystem`) 밖이다. 그런 칸이 막히면 [`CmdLookup::run_within`] 의 마감이 끊는다.
#[cfg(windows)]
fn is_remote(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    match path.components().next() {
        Some(Component::Prefix(prefix)) => {
            !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        }
        _ => false,
    }
}

/// 자식과 그 자손 전부를 한 번에 끊는 손잡이.
#[cfg(windows)]
struct ProcessTree {
    job: crate::platform::JobObjectHandle,
}

#[cfg(windows)]
impl ProcessTree {
    fn attach(child: &Child) -> io::Result<Self> {
        let job = crate::platform::JobObjectHandle::new()?;
        job.assign(child.id())?;
        Ok(Self { job })
    }

    /// 멈춘 채 띄운 자식([`configure_os`])을 깨운다 — Job 에 넣은 **뒤에** 부른다.
    fn start(&self, child: &Child) -> io::Result<()> {
        crate::platform::resume_suspended_process(child.id())
    }

    fn kill(&self, child: &mut Child) {
        if let Err(e) = self.job.terminate(1) {
            tracing::debug!(
                pid = child.id(),
                "사용량 조회: Job 종료 실패 — 자식만 끊는다: {e}"
            );
        }
        let _ = child.kill();
    }

    /// Job 안이 비었나. 조회가 실패하면 모른다 — 자식 종료만 믿는다.
    fn members_gone(&self) -> bool {
        self.job.active_processes().map_or(true, |n| n == 0)
    }
}

#[cfg(unix)]
struct ProcessTree {
    group: u32,
}

#[cfg(unix)]
impl ProcessTree {
    fn attach(child: &Child) -> io::Result<Self> {
        Ok(Self { group: child.id() })
    }

    /// 그룹은 기동 때 이미 정해졌다([`configure_os`]) — 깨울 것이 없다.
    fn start(&self, _child: &Child) -> io::Result<()> {
        Ok(())
    }

    /// ★알려진 틈★: 정상 종료 뒤의 drop 은 leader 를 이미 거둔(wait) 다음이다. 구성원이 남아 있으면 그 번호는
    /// 재사용되지 않지만, 하나도 없고 그 사이 번호가 새 그룹에 넘어갔다면 남의 그룹을 맞힌다(창 = 마이크로초).
    fn kill(&self, child: &mut Child) {
        // 음수 pid = 그 프로세스 그룹. `--` 가 없으면 `-<id>` 를 신호 이름으로 읽는다.
        let group = format!("-{}", self.group);
        if let Err(e) = Command::new("kill")
            .args(["-s", "KILL", "--", &group])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            tracing::debug!(
                pid = child.id(),
                "사용량 조회: 그룹 kill 실패 — 자식만 끊는다: {e}"
            );
        }
        let _ = child.kill();
    }

    /// 그룹 구성원을 셀 수단이 std 에 없다 — 자식 종료만 믿는다.
    fn members_gone(&self) -> bool {
        true
    }
}

// ── 줄 읽기 ──────────────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
enum LineEnd {
    /// 한 바이트도 없이 스트림이 끝났다.
    Eof,
    Complete,
    /// 상한에 닿았다 — ★줄의 나머지는 아직 안 읽었다★. 다음 줄을 읽기 전에 [`skip_rest_of_line`] 을 부른다.
    Cut,
}

/// 줄 하나(끝의 `\n` 포함, 개행 없이 끝난 마지막 조각도 한 줄)를 `out` 에 최대 `cap` 바이트까지 담는다.
/// 상한에 닿으면 나머지를 기다리지 않고 곧바로 `Cut` 이다 — 개행 없이 쏟아지는 출력에도 곧 돌아온다.
fn read_capped_line(
    reader: &mut impl BufRead,
    cap: usize,
    out: &mut Vec<u8>,
) -> io::Result<LineEnd> {
    let mut seen_any = false;
    loop {
        let (used, end) = {
            let available = match reader.fill_buf() {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            if available.is_empty() {
                return Ok(if seen_any {
                    LineEnd::Complete
                } else {
                    LineEnd::Eof
                });
            }
            seen_any = true;
            let (chunk, found_newline) = match available.iter().position(|&b| b == b'\n') {
                Some(i) => (available.get(..=i).unwrap_or(available), true),
                None => (available, false),
            };
            let room = cap.saturating_sub(out.len());
            match chunk.get(..room) {
                Some(fits) if fits.len() < chunk.len() => {
                    out.extend_from_slice(fits);
                    (fits.len(), Some(LineEnd::Cut))
                }
                _ => {
                    out.extend_from_slice(chunk);
                    let end = if found_newline {
                        Some(LineEnd::Complete)
                    } else if out.len() >= cap {
                        // 개행 없이 상한을 딱 채웠다 — 다음 바이트를 기다리면 멈춘 자식 앞에서 마감까지 막힌다.
                        Some(LineEnd::Cut)
                    } else {
                        None
                    };
                    (chunk.len(), end)
                }
            }
        };
        reader.consume(used);
        if let Some(end) = end {
            return Ok(end);
        }
    }
}

/// 줄의 남은 바이트를 개행(포함)이나 EOF 까지 읽어 버린다 — 메모리에 담지 않는다.
fn skip_rest_of_line(reader: &mut impl BufRead) -> io::Result<()> {
    loop {
        let (used, done) = {
            let available = match reader.fill_buf() {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            if available.is_empty() {
                return Ok(());
            }
            match available.iter().position(|&b| b == b'\n') {
                Some(i) => (i + 1, true),
                None => (available.len(), false),
            }
        };
        reader.consume(used);
        if done {
            return Ok(());
        }
    }
}

fn line_text(mut raw: Vec<u8>) -> String {
    if raw.last() == Some(&b'\n') {
        raw.pop();
        if raw.last() == Some(&b'\r') {
            raw.pop();
        }
    }
    String::from_utf8(raw).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

/// stderr 한 줄을 꼬리에 실을 모양으로 — ★가리기가 자르기보다 먼저다★(자른 뒤 가리면 경계에 걸친 토큰이
/// 패턴 길이 밑으로 잘려 빠져나간다).
///
/// 읽기 상한에서 잘린 줄은 마지막 공백 앞까지만 쓴다 — 가리는 토큰 모양에는 공백이 없으므로 그 경계에 걸친
/// 토큰 조각이 가리기를 빠져나가지 못한다. 공백이 없으면 그 줄은 통째로 버린다.
fn tail_line(raw: &[u8], cut_at_read: bool) -> String {
    let raw = if cut_at_read {
        match raw.iter().rposition(|b| b.is_ascii_whitespace()) {
            Some(i) => &raw[..i],
            None => &[][..],
        }
    } else {
        raw
    };
    let text = String::from_utf8_lossy(raw);
    let masked = mask_secrets(text.trim_end_matches(['\r', '\n']));
    masked.chars().take(STDERR_TAIL_CHARS).collect()
}

#[derive(Default)]
struct StderrTail {
    lines: VecDeque<String>,
    total_lines: u64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── 스레드 몸체 ──────────────────────────────────────────────────────────────────

fn pump_stdin(mut stdin: ChildStdin, lines: Receiver<Vec<u8>>, acks: Sender<io::Result<()>>) {
    for bytes in lines {
        let result = stdin.write_all(&bytes).and_then(|()| stdin.flush());
        let failed = result.is_err();
        if acks.send(result).is_err() || failed {
            return;
        }
    }
}

/// 개행 없이 상한에 닿은 줄(상한을 넘는 줄 · 개행 없이 상한을 딱 채운 조각)은 닿는 순간 `Io` 를 보내고, 그 줄의
/// 나머지는 읽어 버린 뒤 다음 줄로 간다.
fn pump_stdout(stdout: impl Read, lines: SyncSender<Result<String, ProbeError>>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut raw = Vec::new();
        let (item, cut) = match read_capped_line(&mut reader, STDOUT_LINE_MAX, &mut raw) {
            Ok(LineEnd::Eof) => return,
            Ok(LineEnd::Complete) => (Ok(line_text(raw)), false),
            Ok(LineEnd::Cut) => (
                Err(ProbeError::Io(format!(
                    "stdout 한 줄이 개행 없이 상한 {STDOUT_LINE_MAX} 바이트에 닿았다"
                ))),
                true,
            ),
            Err(e) => {
                let _ = lines.send(Err(ProbeError::Io(format!("stdout 읽기 실패: {e}"))));
                return;
            }
        };
        if lines.send(item).is_err() {
            return;
        }
        if cut {
            if let Err(e) = skip_rest_of_line(&mut reader) {
                let _ = lines.send(Err(ProbeError::Io(format!("stdout 읽기 실패: {e}"))));
                return;
            }
        }
    }
}

fn drain_stderr(stderr: ChildStderr, tail: Arc<Mutex<StderrTail>>) {
    let mut reader = BufReader::new(stderr);
    loop {
        let mut raw = Vec::new();
        let end = match read_capped_line(&mut reader, STDERR_READ_MAX, &mut raw) {
            Ok(LineEnd::Eof) | Err(_) => return,
            Ok(end) => end,
        };
        if end == LineEnd::Cut && skip_rest_of_line(&mut reader).is_err() {
            return;
        }
        let line = tail_line(&raw, end == LineEnd::Cut);
        let mut kept = lock(&tail);
        kept.total_lines += 1;
        if line.is_empty() {
            continue;
        }
        if kept.lines.len() == STDERR_TAIL_LINES {
            kept.lines.pop_front();
        }
        kept.lines.push_back(line);
    }
}

fn start_thread(name: &str, body: impl FnOnce() + Send + 'static) -> Result<(), ProbeError> {
    thread::Builder::new()
        .name(name.to_owned())
        .spawn(body)
        .map(drop)
        .map_err(|e| ProbeError::Spawn(format!("{name} 스레드를 띄우지 못했다: {e}")))
}

fn remaining(deadline: Instant) -> Option<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
}

// ── 자식 ─────────────────────────────────────────────────────────────────────────

struct OsProbeChild {
    child: Child,
    tree: ProcessTree,
    not_installed: NotInstalledCheck,
    /// `None` = stdin 이 닫혔다(쓰기 실패 · 라이터 종료 · [`ProbeChild::close_stdin`]).
    stdin_tx: Option<Sender<Vec<u8>>>,
    write_acks: Receiver<io::Result<()>>,
    stdout_rx: Receiver<Result<String, ProbeError>>,
    stdout_eof: bool,
    stderr_tail: Arc<Mutex<StderrTail>>,
    exit: Option<ExitInfo>,
    /// 마감으로 끊었다 — 이후 모든 동작은 곧바로 `Timeout`.
    cut: bool,
}

impl OsProbeChild {
    fn spawn(cmd: &ProbeCommand, deadline: Instant) -> Result<Self, ProbeError> {
        if remaining(deadline).is_none() {
            return Err(ProbeError::Timeout);
        }
        // 없는 작업 폴더는 POSIX 에서 `NotFound` 로 떨어져 「미설치」로 오인된다 — 먼저 가른다.
        if !cmd.cwd.is_dir() {
            return Err(ProbeError::Spawn("작업 폴더가 없다".to_owned()));
        }
        let mut command = Command::new(&cmd.program);
        command
            .args(&cmd.args)
            .current_dir(&cmd.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for key in &cmd.env_remove {
            command.env_remove(key);
        }
        for (key, value) in &cmd.env_set {
            command.env(key, value);
        }
        configure_os(&mut command);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                tracing::debug!(program = %cmd.program, "사용량 조회: 프로그램을 찾지 못했다");
                return Err(ProbeError::NotInstalled);
            }
            Err(e) => {
                tracing::debug!(program = %cmd.program, "사용량 조회 프로세스 기동 실패: {e}");
                return Err(ProbeError::Spawn(e.to_string()));
            }
        };
        let pid = child.id();
        let tree = match ProcessTree::attach(&child) {
            Ok(tree) => tree,
            Err(e) => {
                let _ = child.kill();
                if !reap_within(&mut child, KILL_WAIT) {
                    tracing::warn!(
                        pid,
                        "사용량 조회: 트리에 못 묶은 자식이 kill 뒤에도 안 끝났다"
                    );
                }
                return Err(ProbeError::Spawn(format!(
                    "프로세스 트리에 묶지 못했다: {e}"
                )));
            }
        };

        let pipes = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let (stdin_tx, stdin_rx) = mpsc::channel();
        let (ack_tx, write_acks) = mpsc::channel();
        let (stdout_tx, stdout_rx) = mpsc::sync_channel(STDOUT_QUEUE);
        let stderr_tail = Arc::new(Mutex::new(StderrTail::default()));
        // 여기부터 실패하면 이 값의 drop 이 트리를 끊는다.
        let this = Self {
            child,
            tree,
            not_installed: NotInstalledCheck::new(cmd),
            stdin_tx: Some(stdin_tx),
            write_acks,
            stdout_rx,
            stdout_eof: false,
            stderr_tail: stderr_tail.clone(),
            exit: None,
            cut: false,
        };
        let (Some(stdin), Some(stdout), Some(stderr)) = pipes else {
            return Err(ProbeError::Spawn("자식의 파이프를 받지 못했다".to_owned()));
        };
        start_thread("usage-probe-stdin", move || {
            pump_stdin(stdin, stdin_rx, ack_tx)
        })?;
        start_thread("usage-probe-stdout", move || pump_stdout(stdout, stdout_tx))?;
        start_thread("usage-probe-stderr", move || {
            drain_stderr(stderr, stderr_tail)
        })?;
        if let Err(e) = this.tree.start(&this.child) {
            tracing::debug!(
                pid,
                "사용량 조회: 멈춘 채 띄운 프로세스를 깨우지 못했다 — 끊는다: {e}"
            );
            return Err(ProbeError::Spawn(
                "멈춘 채 띄운 프로세스를 깨우지 못했다".to_owned(),
            ));
        }
        tracing::debug!(program = %cmd.program, args = cmd.args.len(), pid, "사용량 조회 프로세스 기동");
        Ok(this)
    }

    fn cut_at_deadline(&mut self, during: &'static str) -> ProbeError {
        if !self.cut {
            self.cut = true;
            self.tree.kill(&mut self.child);
            tracing::debug!(
                pid = self.child.id(),
                during,
                "사용량 조회: 마감을 넘겨 프로세스 트리를 끊었다"
            );
        }
        ProbeError::Timeout
    }

    fn tree_gone(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_))) && self.tree.members_gone()
    }

    /// 직접 띄운 프로세스의 종료 코드 — 판정은 부르는 쪽 몫이다([`ProbeChild::wait_exit`]).
    fn wait_status(&mut self, deadline: Instant) -> Result<Option<i32>, ProbeError> {
        if remaining(deadline).is_none() {
            return Err(self.cut_at_deadline("wait"));
        }
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return Ok(status.code()),
                Ok(None) => {}
                Err(e) => return Err(ProbeError::Io(format!("종료 상태를 못 읽었다: {e}"))),
            }
            let Some(left) = remaining(deadline) else {
                return Err(self.cut_at_deadline("wait"));
            };
            thread::sleep(left.min(WAIT_POLL));
        }
    }

    fn wait_tree_gone(&mut self, budget: Duration) -> bool {
        let until = Instant::now() + budget;
        loop {
            if self.tree_gone() {
                return true;
            }
            if Instant::now() >= until {
                return false;
            }
            thread::sleep(WAIT_POLL);
        }
    }

    #[cfg(all(test, windows))]
    fn pid(&self) -> u32 {
        self.child.id()
    }

    #[cfg(test)]
    fn has_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }
}

fn reap_within(child: &mut Child, budget: Duration) -> bool {
    let until = Instant::now() + budget;
    loop {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        thread::sleep(WAIT_POLL);
    }
}

impl ProbeChild for OsProbeChild {
    fn write_line(&mut self, line: &str, deadline: Instant) -> Result<(), ProbeError> {
        if self.cut {
            return Err(ProbeError::Timeout);
        }
        let Some(left) = remaining(deadline) else {
            return Err(self.cut_at_deadline("write"));
        };
        let Some(tx) = self.stdin_tx.as_ref() else {
            return Err(ProbeError::Io("stdin 이 닫혔다".to_owned()));
        };
        let mut bytes = Vec::with_capacity(line.len() + 1);
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
        if tx.send(bytes).is_err() {
            self.stdin_tx = None;
            return Err(ProbeError::Io("stdin 라이터가 끝났다".to_owned()));
        }
        match self.write_acks.recv_timeout(left) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => {
                self.stdin_tx = None;
                Err(ProbeError::Io(format!("stdin 쓰기 실패: {e}")))
            }
            Err(RecvTimeoutError::Timeout) => Err(self.cut_at_deadline("write")),
            Err(RecvTimeoutError::Disconnected) => {
                self.stdin_tx = None;
                Err(ProbeError::Io("stdin 라이터가 끝났다".to_owned()))
            }
        }
    }

    fn read_line(&mut self, deadline: Instant) -> Result<Option<String>, ProbeError> {
        if self.cut {
            return Err(ProbeError::Timeout);
        }
        if self.stdout_eof {
            return Ok(None);
        }
        let Some(left) = remaining(deadline) else {
            return Err(self.cut_at_deadline("read"));
        };
        match self.stdout_rx.recv_timeout(left) {
            Ok(Ok(line)) => Ok(Some(line)),
            Ok(Err(e)) => Err(e),
            Err(RecvTimeoutError::Timeout) => Err(self.cut_at_deadline("read")),
            Err(RecvTimeoutError::Disconnected) => {
                self.stdout_eof = true;
                Ok(None)
            }
        }
    }

    fn wait_exit(&mut self, deadline: Instant) -> Result<ExitInfo, ProbeError> {
        if self.cut {
            return Err(ProbeError::Timeout);
        }
        if let Some(exit) = self.exit {
            return Ok(exit);
        }
        let code = self.wait_status(deadline)?;
        let exit = ExitInfo::new(code, self.not_installed.judge(code, deadline));
        self.exit = Some(exit);
        Ok(exit)
    }

    fn wait_exit_code(&mut self, deadline: Instant) -> Result<Option<i32>, ProbeError> {
        if self.cut {
            return Err(ProbeError::Timeout);
        }
        if let Some(exit) = self.exit {
            return Ok(exit.code());
        }
        self.wait_status(deadline)
    }

    fn close_stdin(&mut self) {
        self.stdin_tx = None;
    }

    fn stderr_tail(&self) -> Vec<String> {
        lock(&self.stderr_tail).lines.iter().cloned().collect()
    }
}

/// 트리 kill + 유계 종료 대기. 정상 종료한 자식에도 kill 을 보낸다 — 자식이 남긴 손자가 있을 수 있다.
impl Drop for OsProbeChild {
    fn drop(&mut self) {
        self.tree.kill(&mut self.child);
        self.stdin_tx = None;
        let pid = self.child.id();
        if !self.wait_tree_gone(KILL_WAIT) {
            tracing::warn!(
                pid,
                wait_ms = KILL_WAIT.as_millis() as u64,
                "사용량 조회 프로세스가 kill 뒤 대기 상한 안에 끝나지 않았다 — 임시 폴더가 남을 수 있다"
            );
        }
        let stderr_lines = lock(&self.stderr_tail).total_lines;
        tracing::debug!(pid, stderr_lines, "사용량 조회 프로세스 정리");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::testing::TempRoot;
    use crate::usage::ScratchDir;
    use std::io::Cursor;
    use std::path::Path;
    use uuid::Uuid;

    const FAR: Duration = Duration::from_secs(20);

    fn far() -> Instant {
        Instant::now() + FAR
    }

    /// 셸 한 줄을 도는 명령 — Windows 는 `cmd.exe /v:on /c`(지연 확장 `!VAR!`), POSIX 는 `sh -c`.
    fn shell(windows: &str, posix: &str, cwd: &Path) -> ProbeCommand {
        #[cfg(windows)]
        let (program, args) = {
            let _ = posix;
            (
                "cmd.exe",
                vec!["/v:on".to_owned(), "/c".to_owned(), windows.to_owned()],
            )
        };
        #[cfg(unix)]
        let (program, args) = {
            let _ = windows;
            ("sh", vec!["-c".to_owned(), posix.to_owned()])
        };
        ProbeCommand {
            program: program.to_owned(),
            args,
            cwd: cwd.to_path_buf(),
            env_set: Vec::new(),
            env_remove: Vec::new(),
        }
    }

    /// stdin 을 기다리며 영영 안 끝나는 자식.
    fn waits_for_stdin(cwd: &Path) -> ProbeCommand {
        shell("set /p L=", "read l", cwd)
    }

    /// stdin 을 읽지 않고 한동안 사는 자식(Windows 는 손자 `ping` 을 둔다).
    fn ignores_stdin(cwd: &Path) -> ProbeCommand {
        shell("ping -n 30 127.0.0.1 > NUL", "sleep 30", cwd)
    }

    fn eventually(what: &str, mut check: impl FnMut() -> bool) {
        let until = Instant::now() + Duration::from_secs(5);
        while !check() {
            assert!(Instant::now() < until, "5초 안에 안 됐다: {what}");
            thread::sleep(WAIT_POLL);
        }
    }

    fn read_to_eof(child: &mut OsProbeChild) -> Vec<String> {
        let mut lines = Vec::new();
        while let Some(line) = child.read_line(far()).expect("읽기") {
            lines.push(line);
        }
        lines
    }

    // ── 순수 도우미 ──

    #[test]
    fn capped_reader_splits_lines_cuts_long_ones_and_keeps_the_stream_in_step() {
        let mut reader =
            BufReader::with_capacity(4, Cursor::new(b"ab\r\nabcdefgh\nwxyz\nlast".to_vec()));
        let mut read = |cap| {
            let mut out = Vec::new();
            let end = read_capped_line(&mut reader, cap, &mut out).expect("읽기");
            if end == LineEnd::Cut {
                skip_rest_of_line(&mut reader).expect("나머지 버림");
            }
            (end, out)
        };
        assert_eq!(read(8), (LineEnd::Complete, b"ab\r\n".to_vec()));
        assert_eq!(read(4), (LineEnd::Cut, b"abcd".to_vec()));
        // 개행까지 딱 상한이면 자르지 않는다 — 상한은 개행 바이트를 센다.
        assert_eq!(read(5), (LineEnd::Complete, b"wxyz\n".to_vec()));
        assert_eq!(read(8), (LineEnd::Complete, b"last".to_vec()));
        assert_eq!(read(8), (LineEnd::Eof, Vec::new()));
    }

    /// 데이터를 다 준 뒤 `release` 가 사라질 때까지 막히는 읽기 끝 — 개행 없이 멈춘 자식의 stdout.
    struct StallingReader {
        data: Cursor<Vec<u8>>,
        release: Receiver<()>,
    }

    impl Read for StallingReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.data.read(buf)? {
                0 => {
                    let _ = self.release.recv();
                    Ok(0)
                }
                n => Ok(n),
            }
        }
    }

    /// ★상한에 닿는 순간 `Io` 다★ — 개행이 올 때까지(= 끝내 안 오면 마감까지) 기다리지 않는다. 상한을 개행 없이
    ///   딱 채우고 멈춘 줄도 같다(다음 바이트를 기다리지 않는다).
    #[test]
    fn an_over_long_line_is_reported_before_its_newline_arrives() {
        for len in [STDOUT_LINE_MAX + 1, STDOUT_LINE_MAX] {
            let (release_tx, release) = mpsc::channel();
            let reader = StallingReader {
                data: Cursor::new(vec![b'x'; len]),
                release,
            };
            let (tx, rx) = mpsc::sync_channel(STDOUT_QUEUE);
            let pump = thread::spawn(move || pump_stdout(reader, tx));

            let first = rx.recv_timeout(Duration::from_secs(5));
            assert!(
                matches!(first, Ok(Err(ProbeError::Io(_)))),
                "{len} 바이트: 개행 전에 Io 가 와야 한다: {first:?}"
            );
            drop(release_tx);
            pump.join().expect("리더 스레드");
            assert!(rx.recv().is_err(), "{len} 바이트: EOF 뒤 더 오는 것이 없다");
        }
    }

    #[test]
    fn line_text_strips_one_line_ending_and_survives_bad_utf8() {
        assert_eq!(line_text(b"x\r\n".to_vec()), "x");
        assert_eq!(line_text(b"x".to_vec()), "x");
        assert_eq!(line_text(vec![b'a', 0xff, b'\n']), "a\u{fffd}");
    }

    #[test]
    fn tail_line_masks_before_it_trims() {
        let token = format!("sk-{}", "a".repeat(30));
        assert_eq!(
            tail_line(format!("key {token}\r\n").as_bytes(), false),
            "key ***"
        );
        // 읽기 상한이 토큰 한가운데를 잘랐다 — 잘린 조각은 가릴 수 없으니 마지막 공백 앞까지만.
        let cut = format!("key {}", &token[..12]);
        assert_eq!(tail_line(cut.as_bytes(), true), "key");
        assert_eq!(tail_line(token.as_bytes(), true), "");
        let long = "x".repeat(STDERR_TAIL_CHARS * 2);
        assert_eq!(
            tail_line(long.as_bytes(), false).chars().count(),
            STDERR_TAIL_CHARS
        );
    }

    /// 9009 는 곧바로 「없다」, 그 밖의 실패는 대상을 찾아본 뒤, 성공·cmd 아님·마감 지남은 늘 거짓.
    #[cfg(windows)]
    #[test]
    fn the_judgment_reads_9009_and_looks_only_after_a_failure() {
        let root = TempRoot::new("probe-judge");
        let missing = format!("engram-no-such-program-{}", Uuid::new_v4().simple());
        let check = |target: &str, program: &str| {
            let mut cmd = cmd_c(target, root.path());
            cmd.program = program.to_owned();
            NotInstalledCheck::with_gate(&cmd, own_gate())
        };
        let absent = check(&missing, "cmd.exe");
        assert!(absent.judge(Some(9009), far()));
        assert!(absent.judge(Some(1), far()), "없는 대상 + 실패");
        assert!(!absent.judge(Some(0), far()), "성공");
        assert!(
            !absent.judge(Some(1), Instant::now()),
            "마감이 지나면 모른다 — 「없다」가 아니다"
        );
        let present = check("ping", r"C:\Windows\System32\CMD.EXE");
        assert!(present.judge(Some(9009), far()));
        assert!(
            !present.judge(Some(1), far()),
            "있는 대상의 실패는 대상 자신의 실패다"
        );
        let direct = check(&missing, "other.exe");
        assert!(!direct.judge(Some(9009), far()), "cmd 가 아니다");
        assert!(!direct.judge(Some(1), far()));
    }

    /// 끝난 뒤 받아 둔 종료 상태도 마감에 끊긴 뒤에는 `Timeout` 이다 — 끊긴 뒤의 모든 동작은 `Timeout`.
    #[test]
    fn a_cut_after_the_exit_turns_wait_exit_into_a_timeout() {
        let root = TempRoot::new("probe-cut-after-exit");
        let mut child =
            OsProbeChild::spawn(&shell("exit 3", "exit 3", root.path()), far()).expect("기동");
        read_to_eof(&mut child);
        assert_eq!(child.wait_exit(far()).map(|e| e.code()), Ok(Some(3)));
        assert_eq!(
            child.write_line("late", Instant::now()),
            Err(ProbeError::Timeout)
        );
        assert_eq!(child.wait_exit(far()), Err(ProbeError::Timeout));
    }

    #[test]
    fn a_spawn_past_the_deadline_creates_no_process() {
        let root = TempRoot::new("probe-spawn-late");
        let marker = root.path().join("ran");
        let cmd = shell("echo x> ran", "echo x > ran", root.path());
        assert!(matches!(
            OsProbeChild::spawn(&cmd, Instant::now()),
            Err(ProbeError::Timeout)
        ));
        thread::sleep(Duration::from_millis(300));
        assert!(!marker.exists(), "마감이 지났는데 자식이 돌았다");
    }

    // ── 실 자식 ──

    #[test]
    fn a_line_round_trips_through_a_real_child() {
        let root = TempRoot::new("probe-roundtrip");
        let cmd = shell(
            "set /p L=& echo got:!L!",
            r#"read l; echo "got:$l""#,
            root.path(),
        );
        let mut child = OsProbeChild::spawn(&cmd, far()).expect("기동");

        child.write_line("hello", far()).expect("쓰기");
        assert_eq!(read_to_eof(&mut child), vec!["got:hello".to_owned()]);
        let exit = child.wait_exit(far()).expect("종료");
        assert_eq!(exit.code(), Some(0));
        assert!(!exit.looks_not_installed());
    }

    #[test]
    fn a_read_past_the_deadline_times_out_and_cuts_the_child() {
        let root = TempRoot::new("probe-read-deadline");
        let mut child = OsProbeChild::spawn(&waits_for_stdin(root.path()), far()).expect("기동");

        let started = Instant::now();
        let result = child.read_line(Instant::now() + Duration::from_millis(300));
        assert_eq!(result, Err(ProbeError::Timeout));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "마감을 넘겨 매달렸다"
        );
        eventually("마감 kill 뒤 자식 종료", || child.has_exited());

        // 끊긴 뒤에는 무엇이든 곧바로 Timeout.
        assert_eq!(child.write_line("x", far()), Err(ProbeError::Timeout));
        assert_eq!(child.read_line(far()), Err(ProbeError::Timeout));
        assert_eq!(child.wait_exit(far()), Err(ProbeError::Timeout));
    }

    #[test]
    fn an_already_passed_deadline_cuts_without_waiting() {
        let root = TempRoot::new("probe-past-deadline");
        let mut child = OsProbeChild::spawn(&waits_for_stdin(root.path()), far()).expect("기동");
        assert_eq!(
            child.write_line("x", Instant::now()),
            Err(ProbeError::Timeout)
        );
        eventually("마감 kill 뒤 자식 종료", || child.has_exited());
    }

    #[test]
    fn a_child_that_never_reads_stdin_blocks_the_write_until_the_deadline() {
        let root = TempRoot::new("probe-write-blocks");
        let mut child = OsProbeChild::spawn(&ignores_stdin(root.path()), far()).expect("기동");
        // 파이프 버퍼보다 훨씬 크다 — 안 읽는 자식 앞에서 반드시 막힌다.
        let huge = "x".repeat(1024 * 1024);

        let started = Instant::now();
        let result = child.write_line(&huge, Instant::now() + Duration::from_millis(500));
        assert_eq!(result, Err(ProbeError::Timeout));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "마감을 넘겨 매달렸다"
        );
        eventually("마감 kill 뒤 자식 종료", || child.has_exited());
    }

    /// 아래 시험의 자식 몫 — 이 시험 바이너리를 다시 띄워 이 항목만 돌린다. 표식 env 가 없으면 아무것도 안 한다.
    const LINGER_ENV: &str = "ENGRAM_USAGE_PROBE_TEST_LINGER";
    const LINGER_TEST: &str = "usage::process::tests::helper_closes_stdout_and_lingers";

    #[test]
    #[ignore = "시험 자식 프로세스 몫 — a_child_that_closes_stdout_but_lives_is_cut_at_the_deadline 이 띄운다"]
    fn helper_closes_stdout_and_lingers() {
        if std::env::var_os(LINGER_ENV).is_none() {
            return;
        }
        close_own_stdout();
        thread::sleep(Duration::from_secs(60));
    }

    #[cfg(windows)]
    fn close_own_stdout() {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        let raw = std::io::stdout().as_raw_handle();
        // SAFETY: 이 시험 자식 프로세스의 stdout 핸들을 한 번 닫는다 — 닫은 뒤로는 stdout 에 쓰지 않는다.
        drop(unsafe { OwnedHandle::from_raw_handle(raw) });
    }

    #[cfg(unix)]
    fn close_own_stdout() {
        use std::os::unix::io::{FromRawFd, OwnedFd};
        // SAFETY: 이 시험 자식 프로세스의 fd 1 을 한 번 닫는다 — 닫은 뒤로는 stdout 에 쓰지 않는다.
        drop(unsafe { OwnedFd::from_raw_fd(1) });
    }

    #[test]
    fn a_child_that_closes_stdout_but_lives_is_cut_at_the_deadline() {
        let root = TempRoot::new("probe-stdout-closed");
        let exe = std::env::current_exe().expect("시험 바이너리 경로");
        let cmd = ProbeCommand {
            program: exe.to_string_lossy().into_owned(),
            args: [
                "--ignored",
                "--exact",
                LINGER_TEST,
                "--nocapture",
                "--test-threads=1",
            ]
            .map(str::to_owned)
            .to_vec(),
            cwd: root.path().to_path_buf(),
            env_set: vec![(LINGER_ENV.to_owned(), "1".to_owned())],
            env_remove: Vec::new(),
        };
        let mut child = OsProbeChild::spawn(&cmd, far()).expect("기동");

        // 시험 하네스의 머리 줄이 먼저 올 수 있다 — EOF 까지 흘린다.
        read_to_eof(&mut child);
        let started = Instant::now();
        let result = child.wait_exit(Instant::now() + Duration::from_millis(500));
        assert_eq!(
            result,
            Err(ProbeError::Timeout),
            "stdout 을 닫은 자식이 벌써 끝났다"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "마감을 넘겨 매달렸다"
        );
        eventually("마감 kill 뒤 자식 종료", || child.has_exited());
    }

    /// ★트리 전부가 Job 안이다★ — cmd·콘솔 호스트·손자 `ping` 셋이 Job 에 든다(실측 2026-09-27 ActiveProcesses =
    ///   3). 멈춘 채 띄워 넣은 뒤 깨우므로 곧바로 띄운 손자도 빠지지 않고, drop 은 그 전부가 끝난 뒤에 돌아온다.
    #[cfg(windows)]
    #[test]
    fn drop_kills_the_whole_tree() {
        use engram_dashboard_base::platform::{child_pids, pid_alive};

        let root = TempRoot::new("probe-tree");
        let child = OsProbeChild::spawn(&ignores_stdin(root.path()), far()).expect("기동");
        let pid = child.pid();
        eventually("손자까지 Job 안", || {
            child.tree.job.active_processes().is_ok_and(|n| n >= 3)
        });
        let descendants = child_pids(pid);
        assert!(!descendants.is_empty(), "cmd.exe 의 자식이 안 보인다");

        drop(child);
        assert!(!pid_alive(pid), "직접 자식이 drop 뒤에도 산다");
        let alive: Vec<u32> = descendants
            .iter()
            .copied()
            .filter(|&d| pid_alive(d))
            .collect();
        assert!(
            alive.is_empty(),
            "drop 이 돌아온 뒤에도 사는 자손: {alive:?}"
        );
    }

    /// 멈춘 채 띄운 자식은 깨어난 채로 넘어온다 — 대화가 되고, 멈춘 스레드가 하나도 남지 않았다(다시 깨우기 = 실패).
    #[cfg(windows)]
    #[test]
    fn the_suspended_spawn_hands_over_a_running_child() {
        let root = TempRoot::new("probe-resumed");
        let cmd = shell("set /p L=& echo got:!L!", "", root.path());
        let mut child = OsProbeChild::spawn(&cmd, far()).expect("기동");
        assert!(
            crate::platform::resume_suspended_process(child.pid()).is_err(),
            "기동이 깨운 뒤에도 멈춘 스레드가 남았다"
        );
        child.write_line("hello", far()).expect("쓰기");
        assert_eq!(read_to_eof(&mut child), vec!["got:hello".to_owned()]);
    }

    /// ★멈춘 적 없는 프로세스를 깨우라고 하면 실패한다★ — 멈춘 채 띄우기를 잃으면 기동이 실패로 드러난다.
    /// `ping` 을 셸 없이 직접 띄운다 — 셸을 거치면 셸만 끊겨 손자가 남는다(이 시험엔 Job 이 없다).
    #[cfg(windows)]
    #[test]
    fn resuming_a_process_that_was_never_suspended_fails() {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut running = Command::new("ping.exe")
            .args(["-n", "30", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("기동");
        let result = crate::platform::resume_suspended_process(running.id());
        let _ = running.kill();
        let _ = running.wait();
        assert!(result.is_err(), "멈추지 않은 프로세스를 깨웠다고 했다");
    }

    #[test]
    fn a_missing_program_is_not_installed() {
        let root = TempRoot::new("probe-missing");
        let cmd = ProbeCommand {
            program: format!("engram-no-such-program-{}", Uuid::new_v4().simple()),
            args: Vec::new(),
            cwd: root.path().to_path_buf(),
            env_set: Vec::new(),
            env_remove: Vec::new(),
        };
        assert!(matches!(
            OsProbeChild::spawn(&cmd, far()),
            Err(ProbeError::NotInstalled)
        ));
    }

    /// 시험마다 제 찾기 명단 — 병렬로 도는 시험끼리 서로의 찾기를 「도는 중」으로 보지 않게(운영 명단은 하나다).
    #[cfg(windows)]
    fn own_gate() -> &'static LookupGate {
        Box::leak(Box::new(LookupGate::new()))
    }

    /// 판정이 대상 찾기에 기대는 실 자식 — 운영 명단 대신 제 명단을 쓴다([`own_gate`]).
    #[cfg(windows)]
    fn spawn_with_own_gate(cmd: &ProbeCommand) -> OsProbeChild {
        let mut child = OsProbeChild::spawn(cmd, far()).expect("기동");
        child.not_installed = NotInstalledCheck::with_gate(cmd, own_gate());
        child
    }

    #[cfg(windows)]
    fn cmd_c(target: &str, cwd: &Path) -> ProbeCommand {
        ProbeCommand {
            program: "cmd.exe".to_owned(),
            args: vec!["/c".to_owned(), target.to_owned(), "--flag".to_owned()],
            cwd: cwd.to_path_buf(),
            env_set: Vec::new(),
            env_remove: Vec::new(),
        }
    }

    /// `cmd.exe /c <없는 이름>` 은 뜬다 — 실패한 뒤의 종료 상태가 「없다」를 싣는다(종료 코드는 이 PC 에서 1 이라
    /// 코드만으로는 못 가른다).
    #[cfg(windows)]
    #[test]
    fn cmd_with_a_missing_target_spawns_and_its_exit_says_not_installed() {
        let root = TempRoot::new("probe-missing-cmd");
        let missing = format!("engram-no-such-program-{}", Uuid::new_v4().simple());
        let mut child = spawn_with_own_gate(&cmd_c(&missing, root.path()));
        assert_eq!(read_to_eof(&mut child), Vec::<String>::new());
        let exit = child.wait_exit(far()).expect("종료");
        assert_ne!(exit.code(), Some(0));
        assert!(exit.looks_not_installed(), "{exit:?}");
    }

    /// 판정 없는 종료 대기는 판정도 하지 않고 판정 없는 종료를 받아 두지도 않는다 — 뒤의 `wait_exit` 가 판정한다.
    #[cfg(windows)]
    #[test]
    fn the_unjudged_wait_leaves_the_judgment_to_wait_exit() {
        let root = TempRoot::new("probe-unjudged-wait");
        let missing = format!("engram-no-such-program-{}", Uuid::new_v4().simple());
        let cmd = cmd_c(&missing, root.path());
        let mut child = OsProbeChild::spawn(&cmd, far()).expect("기동");
        let gate = own_gate();
        child.not_installed = NotInstalledCheck::with_gate(&cmd, gate);
        read_to_eof(&mut child);
        // 그 대상의 찾기가 도는 동안의 판정은 「모른다」다 — 여기서 판정해 받아 두었다면 뒤의 판정이 거짓으로 굳는다.
        assert!(gate.claim(&missing));
        let code = child.wait_exit_code(far()).expect("종료");
        assert_ne!(code, Some(0));
        gate.release(&missing);
        assert!(child.wait_exit(far()).expect("종료").looks_not_installed());
    }

    #[cfg(windows)]
    #[test]
    fn cmd_target_is_only_a_plain_program_word() {
        let root = TempRoot::new("probe-cmd-target");
        let target = |args: &[&str]| {
            let mut cmd = cmd_c("x", root.path());
            cmd.args = args.iter().map(|a| (*a).to_owned()).collect();
            cmd_target(&cmd).map(str::to_owned)
        };
        assert_eq!(target(&["/c", "tool", "--a"]).as_deref(), Some("tool"));
        assert_eq!(target(&["/d", "/C", "tool"]).as_deref(), Some("tool"));
        assert_eq!(target(&["/c", "echo", "hi"]), None, "내부 명령");
        assert_eq!(target(&["/c", "EXIT"]), None, "내부 명령(대소문자)");
        assert_eq!(target(&["/c", "set /p L="]), None, "한 낱말이 아니다");
        assert_eq!(target(&["/c", "a&b"]), None, "cmd 특수 문자");
        assert_eq!(target(&["/k", "tool"]), None);
        let mut other = cmd_c("tool", root.path());
        other.program = "other.exe".to_owned();
        assert_eq!(cmd_target(&other), None, "cmd 가 아니다");
    }

    #[cfg(windows)]
    fn lookup(target: &str, cwd: &Path, env_set: &[(&str, &str)], env_remove: &[&str]) -> Lookup {
        let mut cmd = cmd_c(target, cwd);
        cmd.env_set = env_set
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        cmd.env_remove = env_remove.iter().map(|k| (*k).to_owned()).collect();
        CmdLookup::from_command(&cmd)
            .expect("한 낱말 대상")
            .resolve(far())
    }

    #[cfg(windows)]
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
            Lookup::Found,
            "PATH × PATHEXT"
        );
        assert_eq!(
            lookup("engram-probe-tool.cmd", &cwd, &path, &[]),
            Lookup::Found,
            "확장자까지 준 이름"
        );
        assert_eq!(
            lookup("engram-local-tool", &cwd, &path, &[]),
            Lookup::Found,
            "작업 폴더 먼저"
        );
        assert_eq!(
            lookup(
                "engram-probe-tool",
                &cwd,
                &[path[0], ("PATHEXT", ".EXE")],
                &[]
            ),
            Lookup::Missing,
            "PATHEXT 밖의 확장자"
        );
        assert_eq!(
            lookup("engram-probe-tool", &cwd, &[], &["path"]),
            Lookup::Missing,
            "벗긴 PATH"
        );
        let absolute = bin.join("engram-probe-tool.cmd");
        assert_eq!(
            lookup(&absolute.to_string_lossy(), &cwd, &[], &["path"]),
            Lookup::Found,
            "경로로 준 대상"
        );
        // 상대 칸은 이 프로세스가 아니라 자식의 작업 폴더 기준이다.
        assert_eq!(
            lookup("engram-probe-tool", root.path(), &[("PATH", "bin")], &[]),
            Lookup::Found,
            "상대 PATH 칸"
        );
    }

    /// 정의됐지만 빈 `PATHEXT` 는 없는 것과 같다 — 기본 확장자 목록을 쓴다.
    #[cfg(windows)]
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
                Lookup::Found,
                "{pathext:?}"
            );
        }
    }

    /// 원격 칸은 건너뛴다 — 닿지 않는 UNC 칸이 판정을 붙잡지 않는다.
    #[cfg(windows)]
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
            Lookup::Found
        );
        assert_eq!(
            lookup(
                "engram-no-such-tool",
                root.path(),
                &[("PATH", path.as_str())],
                &[]
            ),
            Lookup::Missing,
            "건너뛴 칸은 찾은 셈 치지 않는다"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "원격 칸을 건너뛰지 않았다: {:?}",
            started.elapsed()
        );
    }

    /// 마감이 지나면 「모른다」다 — 「없다」가 아니다.
    #[cfg(windows)]
    #[test]
    fn a_lookup_out_of_time_is_unknown() {
        let root = TempRoot::new("probe-lookup-late");
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).expect("bin");
        let mut cmd = cmd_c("engram-no-such-tool", root.path());
        cmd.env_set = vec![("PATH".to_owned(), bin.to_string_lossy().into_owned())];
        let lookup = CmdLookup::from_command(&cmd).expect("한 낱말 대상");
        let gate = own_gate();
        assert_eq!(lookup.run_within(Instant::now(), gate), Lookup::Unknown);
        assert_eq!(lookup.resolve(Instant::now()), Lookup::Unknown);
        assert_eq!(lookup.run_within(far(), gate), Lookup::Missing);
    }

    /// ★같은 대상의 찾기는 한 번에 하나다★ — 마감을 넘겨 막힌 찾기가 남아 있는 동안 같은 대상(ASCII 대소문자
    /// 무시)의 판정은 새로 찾지 않고 「모른다」다. 다른 대상은 그 사이에도 찾는다.
    #[cfg(windows)]
    #[test]
    fn a_lookup_still_in_flight_keeps_a_second_one_for_the_same_target_from_starting() {
        let gate = own_gate();
        let (release_tx, release) = mpsc::channel::<()>();
        let first = run_bounded(
            Instant::now() + Duration::from_millis(200),
            gate,
            "engram-tool",
            move || {
                let _ = release.recv();
                Lookup::Missing
            },
        );
        assert_eq!(first, Lookup::Unknown, "막힌 찾기는 마감에서 「모른다」다");

        let ran = Arc::new(AtomicBool::new(false));
        let second = {
            let ran = ran.clone();
            run_bounded(far(), gate, "ENGRAM-TOOL", move || {
                ran.store(true, Ordering::SeqCst);
                Lookup::Missing
            })
        };
        assert_eq!(second, Lookup::Unknown);
        assert!(
            !ran.load(Ordering::SeqCst),
            "같은 대상의 앞선 찾기가 도는데 새로 찾았다"
        );
        assert_eq!(
            run_bounded(far(), gate, "engram-other-tool", || Lookup::Missing),
            Lookup::Missing,
            "다른 대상은 막히지 않는다"
        );

        drop(release_tx);
        eventually("막힌 찾기가 풀린 뒤의 다음 찾기", || {
            run_bounded(far(), gate, "engram-tool", || Lookup::Missing) == Lookup::Missing
        });
    }

    /// ★대상이 다르면 동시에 찾는다★ — 두 벤더의 CLI 가 다 없어 두 판정이 겹쳐도 둘 다 「없다」에 닿는다. 각 찾기가
    /// 다른 쪽이 도는 중일 때까지 기다리므로, 한쪽이 막히면 다른 쪽도 마감에서 「모른다」가 된다.
    #[cfg(windows)]
    #[test]
    fn lookups_for_different_targets_run_side_by_side() {
        let gate = own_gate();
        let both_running = Arc::new(std::sync::Barrier::new(2));
        let judge = move |target: &'static str| {
            let both_running = both_running.clone();
            thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                run_bounded(deadline, gate, target, move || {
                    both_running.wait();
                    Lookup::Missing
                })
            })
        };
        let (first, second) = (judge("engram-tool-a"), judge("engram-tool-b"));
        assert_eq!(first.join().expect("첫 판정"), Lookup::Missing);
        assert_eq!(second.join().expect("둘째 판정"), Lookup::Missing);
    }

    /// 독에 걸린 명단은 「도는 중」으로 친다 — 패닉하지 않고, 찾지도 않고 「모른다」다.
    #[cfg(windows)]
    #[test]
    fn a_poisoned_gate_means_unknown_without_a_panic() {
        let gate = own_gate();
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
                Lookup::Missing
            })
        };
        assert_eq!(found, Lookup::Unknown);
        assert!(!ran.load(Ordering::SeqCst), "독에 걸린 명단에서 찾았다");
    }

    /// 있는 대상이 스스로 실패해도 「없다」가 아니다 — 판정은 대상을 찾아본 뒤에만 선다.
    #[cfg(windows)]
    #[test]
    fn a_present_cmd_target_that_fails_is_not_mistaken_for_not_installed() {
        let root = TempRoot::new("probe-present-cmd");
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).expect("bin");
        std::fs::write(bin.join("engram-probe-fail.cmd"), "@exit 5").expect("shim");
        let mut cmd = cmd_c("engram-probe-fail", root.path());
        cmd.env_set = vec![("PATH".to_owned(), bin.to_string_lossy().into_owned())];
        let mut child = spawn_with_own_gate(&cmd);
        assert_eq!(read_to_eof(&mut child), Vec::<String>::new());
        let exit = child.wait_exit(far()).expect("종료");
        assert_eq!(exit.code(), Some(5));
        assert!(!exit.looks_not_installed());
    }

    #[test]
    fn an_ordinary_exit_code_is_not_mistaken_for_not_installed() {
        let root = TempRoot::new("probe-exit-code");
        let mut child =
            OsProbeChild::spawn(&shell("exit 3", "exit 3", root.path()), far()).expect("기동");
        assert_eq!(read_to_eof(&mut child), Vec::<String>::new());
        let exit = child.wait_exit(far()).expect("종료");
        assert_eq!(exit.code(), Some(3));
        assert!(!exit.looks_not_installed());
        // 끝난 자식의 stdin 에 쓰면 매달리지 않고 실패한다.
        assert!(matches!(
            child.write_line("late", far()),
            Err(ProbeError::Io(_))
        ));
    }

    #[test]
    fn a_missing_cwd_is_a_spawn_error_not_not_installed() {
        let root = TempRoot::new("probe-missing-cwd");
        let cmd = shell("exit 0", "exit 0", &root.path().join("absent"));
        assert!(matches!(
            OsProbeChild::spawn(&cmd, far()),
            Err(ProbeError::Spawn(_))
        ));
    }

    #[test]
    fn env_set_and_env_remove_reach_the_child() {
        let root = TempRoot::new("probe-env");
        // 부모에게 늘 있는 변수를 벗긴다.
        #[cfg(windows)]
        let inherited = "WINDIR";
        #[cfg(unix)]
        let inherited = "HOME";
        let mut cmd = shell(
            "echo [!ENGRAM_PROBE_SET!][!ENGRAM_PROBE_BOTH!]& if defined WINDIR (echo removed:no) else (echo removed:yes)",
            r#"echo "[$ENGRAM_PROBE_SET][$ENGRAM_PROBE_BOTH]"; if [ -n "${HOME+x}" ]; then echo removed:no; else echo removed:yes; fi"#,
            root.path(),
        );
        cmd.env_set = vec![
            ("ENGRAM_PROBE_SET".to_owned(), "one".to_owned()),
            ("ENGRAM_PROBE_BOTH".to_owned(), "kept".to_owned()),
        ];
        cmd.env_remove = vec![inherited.to_owned(), "ENGRAM_PROBE_BOTH".to_owned()];
        let mut child = OsProbeChild::spawn(&cmd, far()).expect("기동");

        assert_eq!(
            read_to_eof(&mut child),
            vec!["[one][kept]".to_owned(), "removed:yes".to_owned()]
        );
    }

    #[test]
    fn stderr_tail_keeps_the_last_lines_masked() {
        let root = TempRoot::new("probe-stderr");
        let token = format!("sk-{}", "b".repeat(30));
        let cmd = shell(
            &format!("(for /l %i in (1,1,12) do @echo line%i 1>&2)& echo key {token} 1>&2"),
            &format!(
                r#"for i in 1 2 3 4 5 6 7 8 9 10 11 12; do echo "line$i" >&2; done; echo "key {token}" >&2"#
            ),
            root.path(),
        );
        let mut child = OsProbeChild::spawn(&cmd, far()).expect("기동");
        read_to_eof(&mut child);
        child.wait_exit(far()).expect("종료");

        eventually("stderr 마지막 줄", || {
            child
                .stderr_tail()
                .last()
                .is_some_and(|line| line.starts_with("key"))
        });
        let tail: Vec<String> = child
            .stderr_tail()
            .iter()
            .map(|line| line.trim_end().to_owned())
            .collect();
        assert_eq!(tail.len(), STDERR_TAIL_LINES);
        assert_eq!(tail.first().map(String::as_str), Some("line6"));
        assert_eq!(tail.last().map(String::as_str), Some("key ***"));
    }

    /// 자식이 작업 폴더로 쥐고 있으면 Windows 는 그 폴더를 못 지운다 — 자식을 먼저 drop 하면 지워진다.
    #[test]
    fn a_scratch_dir_used_as_cwd_is_deleted_after_the_child_is_dropped() {
        let root = TempRoot::new("probe-scratch-cwd");
        let scratch = ScratchDir::create(root.path()).expect("임시 폴더");
        let path = scratch.path().to_path_buf();
        let child = OsProbeChild::spawn(&waits_for_stdin(&path), far()).expect("기동");

        drop(child);
        drop(scratch);
        assert!(!path.exists(), "자식을 끊은 뒤에도 임시 폴더가 남았다");
    }

    #[test]
    fn the_spawner_hands_out_a_working_child() {
        let root = TempRoot::new("probe-spawner");
        let cmd = shell("echo ready", "echo ready", root.path());
        let mut child = OsProbeSpawner.spawn(&cmd, far()).expect("기동");
        assert_eq!(child.read_line(far()), Ok(Some("ready".to_owned())));
        assert_eq!(child.read_line(far()), Ok(None));
    }
}
