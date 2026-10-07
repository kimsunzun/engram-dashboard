//! 자식 프로세스를 띄울 때의 OS 설정 — 창 없이 띄우기 · 트리 뿌리로 띄우기와 그 트리를 한 번에 끊는 손잡이 ·
//! 실패한 셸의 「프로그램 없음」 판정 · 이 프로세스의 Job 밖에서 띄우기(WMI).
// ADR-0230
// ADR-0266

use std::io;
use std::path::Path;
use std::process::{Child, Command};
use std::time::Instant;

#[cfg(windows)]
mod cmd_lookup;
mod wmi;

/// 콘솔 창 없이 띄운다 — Windows = `CREATE_NO_WINDOW`, 그 밖의 OS = 무동작. 창 없는 프로세스(데몬)가 콘솔 앱을
/// 그냥 띄우면 Windows 가 새 콘솔 창을 연다.
///
/// ★생성 플래그를 통째로 정한다★ — std 의 `creation_flags` 는 더하지 않고 바꾼다. 이 앞에 준 생성 플래그는 이것이
/// 지우고, 이 뒤에 생성 플래그를 다시 주면 이것이 지워진다. 트리 뿌리로 띄울 때는 이것 대신
/// [`prepare_tree_root`] 를 부른다(창 없이 띄우기를 함께 한다).
pub fn hide_console_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows::Win32::System::Threading::CREATE_NO_WINDOW;
        cmd.creation_flags(CREATE_NO_WINDOW.0);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// 띄울 자식을 [`TreeRoot`] 의 뿌리로 준비한다 — Windows = 콘솔 창 없음 + 멈춘 채(무리에 넣은 뒤
/// [`TreeRoot::start`] 로 깨운다), POSIX = 자식 pid 를 id 로 하는 새 프로세스 그룹.
///
/// ★Windows 에서 멈춘 채 띄우는 까닭★ — 무리(Job)는 그 안에 든 **뒤에** 만들어진 프로세스에만 물려진다. 깨운 채
///   띄우면 넣기 전에 자식이 띄운 손자가 트리 kill 을 빠져나간다.
/// ★Windows 에서는 생성 플래그를 통째로 정한다★ — [`hide_console_window`] 와 같다(그것과 함께 부르지 않는다).
pub fn prepare_tree_root(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};
        cmd.creation_flags(CREATE_NO_WINDOW.0 | CREATE_SUSPENDED.0);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
}

/// 자식과 그 자손 전부를 한 번에 끊는 손잡이 — [`prepare_tree_root`] 로 준비해 띄운 자식에 [`Self::attach`] 하고
/// [`Self::start`] 로 깨운다. Windows = 이 손잡이가 주인인 자기 무리(Job — 버리면 남은 멤버를 OS 가 끝낸다) ·
/// POSIX = 자식의 프로세스 그룹(버려도 아무것도 끝내지 않는다).
///
/// ★POSIX 갈래는 컴파일된 적이 없다★(이 crate 들이는 규칙 3) — 그룹 kill 은 새 의존 없이 std 만으로 하려고 POSIX
///   `kill` 유틸리티를 띄운다.
// ADR-0275
pub struct TreeRoot {
    #[cfg(windows)]
    group: crate::group::GroupOwner,
    #[cfg(unix)]
    pgid: u32,
}

impl TreeRoot {
    /// 띄운 자식을 트리의 뿌리로 묶는다. Windows `Err` = 무리 만들기 · 넣기의 OS 실패 — 그때 자식은 멈춘 채
    /// 남으니 부르는 쪽이 끊는다.
    pub fn attach(child: &Child) -> io::Result<Self> {
        #[cfg(windows)]
        {
            let group = crate::group::GroupOwner::new()?;
            group.adopt(child.id())?;
            Ok(Self { group })
        }
        #[cfg(unix)]
        {
            Ok(Self { pgid: child.id() })
        }
    }

    /// 멈춘 채 띄운 자식을 깨운다 — [`Self::attach`] **뒤에** 부른다. POSIX = 깨울 것이 없다(`Ok`).
    ///
    /// ★Windows 에서 멈춘 스레드를 하나도 못 깨웠으면 `Err` 다★ — [`prepare_tree_root`] 를 빠뜨려 멈추지 않은 채
    ///   띄웠으면 넣기 전의 틈이 조용히 되살아나지 않고 기동이 실패로 드러난다.
    pub fn start(&self, child: &Child) -> io::Result<()> {
        #[cfg(windows)]
        {
            crate::group::resume_suspended_process(child.id())
        }
        #[cfg(unix)]
        {
            let _ = child;
            Ok(())
        }
    }

    /// 트리 전부를 끊고 자식도 끊는다 — 끝내기를 시작만 하고 돌아온다. 다 끝났는지는 자식의 종료와
    /// [`Self::members_gone`] 으로 본다. 자식 kill 의 결과는 버린다 — 이미 끝난 자식에도 부른다.
    ///
    /// `Err` = 트리 kill(Windows 무리 끝내기 · POSIX `kill` 띄우기)이 실패했다 — 그때도 자식은 끊었다.
    ///
    /// ★POSIX 의 알려진 틈★: 자식을 이미 거둔(wait) 뒤에 부르면 — 구성원이 남아 있으면 그 번호는 재사용되지 않지만,
    ///   하나도 없고 그 사이 번호가 새 그룹에 넘어갔다면 남의 그룹을 맞힌다(창 = 마이크로초).
    pub fn kill(&self, child: &mut Child) -> io::Result<()> {
        #[cfg(windows)]
        let tree = self.group.terminate(1);
        #[cfg(unix)]
        let tree = {
            use std::process::Stdio;
            // 음수 pid = 그 프로세스 그룹. `--` 가 없으면 `-<id>` 를 신호 이름으로 읽는다.
            let group = format!("-{}", self.pgid);
            Command::new("kill")
                .args(["-s", "KILL", "--", &group])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(drop)
        };
        let _ = child.kill();
        tree
    }

    /// 트리의 자손이 다 끝났나. Windows = 무리 안이 비었나(조회가 실패하면 참 — 자식 종료만 믿는다) · POSIX = 늘
    /// 참(그룹 구성원을 셀 수단이 std 에 없다 — 자식 종료만 믿는다).
    pub fn members_gone(&self) -> bool {
        #[cfg(windows)]
        {
            self.group.active_processes().map_or(true, |n| n == 0)
        }
        #[cfg(unix)]
        {
            true
        }
    }
}

/// 「띄우려던 프로그램이 없다」 판정 — 띄울 때 명령에서 필요한 것을 떠 두고, 0 이 아닌 종료를 받았을 때
/// [`Self::judge`] 로 판정한다.
///
/// ★Windows 에서 종료 코드만으로는 못 가른다★ — `cmd.exe /c <없는 이름>` 의 종료 코드가 빌드마다 다르다
///   (실측 2026-09-27, Windows 11 26200 = **1**, 그 안의 errorlevel 은 9009 · 널리 알려진 값은 9009). 1 은
///   대상 프로그램 자신의 실패와 구별되지 않는다. 그래서 9009 가 아닌 실패면 `cmd.exe /c <대상> …` 의 대상을 cmd 와
///   같은 규칙(작업 폴더 → `PATH`, 확장자 = `PATHEXT`)으로 찾아보고, 없을 때만 「없다」다. 대상을 가려낼 수 없는
///   모양이면 9009 만 남는다.
/// ★띄우기 전에 찾지 않는다 — 되살리지 말 것★: 응답 없는 원격 `PATH` 칸 하나가 기동을 마감 너머까지 붙잡았고
///   (실측 21초), cmd 의 AutoRun 이 `PATH` 를 바꾸면 띄우기 전의 판정은 늘 틀린다. 실패 뒤의 판정도 같은 env 를
///   보므로 AutoRun 이 `PATH` 를 바꾼 PC 에서는 틀릴 수 있다 — 단 대상이 실제로 실패했을 때뿐이다.
/// POSIX 는 기동이 `NotFound` 로 실패하므로 판정할 거리가 없다 — Windows 밖에서는 늘
/// [`MissingProgramVerdict::NotJudged`] 다.
pub struct MissingProgramCheck {
    #[cfg(windows)]
    through_cmd: bool,
    #[cfg(windows)]
    lookup: Option<cmd_lookup::CmdLookup>,
    /// 운영 = 프로세스에 하나인 찾기 명단. 시험은 제 것을 넣는다 — 병렬로 도는 시험끼리 서로의 찾기를 막지 않게.
    #[cfg(windows)]
    gate: &'static cmd_lookup::LookupGate,
}

/// `cmd.exe` 가 「내부 또는 외부 명령 … 이 아닙니다」로 끝날 때의 종료 코드.
#[cfg(windows)]
const CMD_NOT_FOUND_EXIT: i32 = 9009;

impl MissingProgramCheck {
    /// 띄운 명령 그대로 — `env_set` 은 `env_remove` 보다 나중에 적용된 것으로 본다(같은 키가 두 목록에 있으면
    /// 남는다). `cwd` = 자식의 작업 폴더.
    pub fn new(
        program: &str,
        args: &[String],
        env_set: &[(String, String)],
        env_remove: &[String],
        cwd: &Path,
    ) -> Self {
        #[cfg(windows)]
        {
            Self::with_gate(
                program,
                args,
                env_set,
                env_remove,
                cwd,
                &cmd_lookup::LOOKUPS_IN_FLIGHT,
            )
        }
        #[cfg(not(windows))]
        {
            let _ = (program, args, env_set, env_remove, cwd);
            Self {}
        }
    }

    #[cfg(windows)]
    fn with_gate(
        program: &str,
        args: &[String],
        env_set: &[(String, String)],
        env_remove: &[String],
        cwd: &Path,
        gate: &'static cmd_lookup::LookupGate,
    ) -> Self {
        Self {
            through_cmd: cmd_lookup::is_cmd_shell(program),
            lookup: cmd_lookup::CmdLookup::from_command(program, args, env_set, env_remove, cwd),
            gate,
        }
    }

    /// `code` 로 끝난 자식이 「없다」였나. 대상 찾기는 `deadline` 까지만 기다린다 — 마감 안에 다 못 찾으면 모른다.
    /// 성공(`Some(0)`)이면 판정하지 않는다.
    pub fn judge(&self, code: Option<i32>, deadline: Instant) -> MissingProgramVerdict {
        #[cfg(windows)]
        {
            if !self.through_cmd || code == Some(0) {
                return MissingProgramVerdict::NotJudged;
            }
            if code == Some(CMD_NOT_FOUND_EXIT) {
                return MissingProgramVerdict::NotFoundExitCode;
            }
            match &self.lookup {
                Some(lookup) => {
                    MissingProgramVerdict::Looked(lookup.run_within(deadline, self.gate))
                }
                None => MissingProgramVerdict::NotJudged,
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (code, deadline);
            MissingProgramVerdict::NotJudged
        }
    }
}

/// [`MissingProgramCheck::judge`] 의 답. 「없다」인가는 [`Self::is_missing`] 이고, 나머지는 부르는 쪽이 찍을 거리다
/// (이 crate 는 찍지 않는다).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissingProgramVerdict {
    /// 판정할 거리가 없다 — 셸을 거치지 않았다 · 성공했다 · 셸의 대상을 프로그램 이름 한 낱말로 가려낼 수 없다.
    NotJudged,
    /// 셸이 「없는 명령」 종료 코드로 끝났다 — 「없다」.
    NotFoundExitCode,
    /// 실패한 셸의 대상을 찾아봤다.
    Looked(TargetLookup),
}

impl MissingProgramVerdict {
    /// 「없다」인가 — 모르면 거짓이다.
    pub fn is_missing(&self) -> bool {
        matches!(
            self,
            Self::NotFoundExitCode | Self::Looked(TargetLookup::Missing)
        )
    }
}

/// 실패한 셸의 대상을 찾아본 결과. ★모르는 갈래(`Unknown` · `Busy` · `NoThread`)를 「없다」로 접지 않는다★.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetLookup {
    Found,
    Missing,
    /// 모른다 — 마감 안에 다 못 찾았거나, 경로로 준 대상이 원격 자리다.
    Unknown,
    /// 모른다 — 같은 대상의 앞선 찾기가 아직 돌고 있어(또는 찾기 명단이 앞선 패닉으로 독에 걸려) 새로 찾지 않았다.
    /// 보통은 앞선 판정의 찾기가 마감을 넘겨 막힌 파일 시스템 조회에 남은 것이다 — 같은 대상의 찾기는 프로세스
    /// 전체에서 한 번에 하나라, 막힌 찾기가 판정마다 쌓이지 않는다.
    Busy,
    /// 모른다 — 찾기 스레드를 띄우지 못했다(그 OS 오류의 문구).
    NoThread(String),
}

/// [`spawn_outside_job`] 의 실패.
#[derive(Debug)]
pub enum DetachedSpawnError {
    /// `Win32_Process.Create` 가 띄우지 않았다 — `rv` = 그 `ReturnValue`(0 이 아닌 값 · 예: 9 = 경로 없음 ·
    /// 21 = 잘못된 인자). 답 객체나 그 값을 못 얻었으면 `u32::MAX` 다.
    Refused { rv: u32 },
    /// 그 밖의 실패 — COM 초기화 · WMI 호출의 실패는 종류 `Other`(문구에 그 HRESULT 가 든다) · Windows 밖 =
    /// `Unsupported`(이 수단이 없다).
    Io(io::Error),
}

/// `exe` 를 이 프로세스가 든 Job 밖에서 띄운다 — Windows = WMI `Win32_Process.Create`. 띄운 프로세스의 부모가 이
/// 프로세스가 아니라 WMI 제공자(WmiPrvSE)라, 이 프로세스의 Job(`KILL_ON_JOB_CLOSE` 포함)을 물려받지 않는다(실측 —
/// spike #1). 띄웠는지만 돌려준다 — pid 도, 뜬 프로세스를 기다리는 일도 없다.
///
/// - 명령줄 = 따옴표로 감싼 `exe` 하나 — 인자를 넘기지 않는다. 환경변수는 이 수단으로 넘길 수 없다.
/// - `exe` 는 절대경로여야 한다 — 상대경로면 `Refused { rv: 9 }`(Path not found).
/// - `console` = 참이면 새 콘솔 창과 함께 띄운다(`CREATE_NEW_CONSOLE`) · 거짓이면 생성 플래그를 넘기지 않는다.
///   ★거짓이어도 창이 안 뜬다는 보장은 아니다★ — 콘솔 서브시스템 exe 는 이렇게 띄워도 콘솔 창이 뜨고, windows
///   서브시스템 exe 는 안 뜬다(실측 2026-06-19). 창 없이 띄우기는 exe 의 서브시스템으로만 된다.
/// - 부르는 스레드에 COM 을 다중 스레드 아파트로 초기화하고 돌아오기 전에 해제한다. 이미 다른 아파트 모드로
///   초기화된 스레드면 그 아파트로 부르고 해제하지 않는다.
///
/// Windows 밖 = `Io` 의 `Unsupported`.
// ADR-0021
// ADR-0271
pub fn spawn_outside_job(exe: &Path, console: bool) -> Result<(), DetachedSpawnError> {
    // ★`CREATE_NO_WINDOW`(0x0800_0000)를 넘기지 말 것★ — WMI 가 `rv` 21(Invalid Parameter)로 거절한다(실측
    //   2026-06-17 · 셸 `discovery` 시험의 `real_wmi_spawn_flag_matrix`). CreateProcess 직접 호출용 플래그라 WMI Create 의
    //   허용 집합 밖이다. `CREATE_NEW_CONSOLE` 은 받는다.
    const CREATE_NEW_CONSOLE: i32 = 0x0000_0010;
    let create_flags = console.then_some(CREATE_NEW_CONSOLE);
    match wmi::create(exe, create_flags)? {
        0 => Ok(()),
        rv => Err(DetachedSpawnError::Refused { rv }),
    }
}

/// [`spawn_outside_job`] 의 원시 호출 — `ReturnValue` 를 오류로 올리지 않고 그대로 돌려준다. `create_flags` =
/// `None` 이면 시작 정보를 넘기지 않고, `Some` 이면 `Win32_ProcessStartup.CreateFlags` 로 넘긴다. 어느 생성
/// 플래그를 WMI 가 거절하나를 재는 진단 시험 몫이다. `Err(Refused)` = 필요한 WMI 객체가 비어 띄우기 전에 멈췄다
/// (`rv` = `u32::MAX`).
#[cfg(any(test, feature = "test-support"))]
pub fn wmi_create_raw(exe: &Path, create_flags: Option<i32>) -> Result<u32, DetachedSpawnError> {
    wmi::create(exe, create_flags)
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::io::{Read, Write};
    #[cfg(windows)]
    use std::process::Stdio;
    #[cfg(windows)]
    use std::time::Duration;

    use super::*;

    #[cfg(windows)]
    fn far() -> Instant {
        Instant::now() + Duration::from_secs(20)
    }

    /// 「없다」는 둘뿐이다 — 모르는 갈래(`Unknown` · `Busy` · `NoThread`)도 판정 거리 없음도 「없다」로 접히지 않는다.
    #[test]
    fn only_a_not_found_exit_or_a_missing_target_is_missing() {
        use MissingProgramVerdict::{Looked, NotFoundExitCode, NotJudged};
        assert!(NotFoundExitCode.is_missing());
        assert!(Looked(TargetLookup::Missing).is_missing());
        for verdict in [
            NotJudged,
            Looked(TargetLookup::Found),
            Looked(TargetLookup::Unknown),
            Looked(TargetLookup::Busy),
            Looked(TargetLookup::NoThread("스레드 없음".to_owned())),
        ] {
            assert!(!verdict.is_missing(), "{verdict:?}");
        }
    }

    /// 9009 는 곧바로 「없다」, 그 밖의 실패는 대상을 찾아본 뒤, 성공·cmd 아님·마감 지남은 늘 「없다」가 아니다.
    #[cfg(windows)]
    #[test]
    fn the_judgment_reads_9009_and_looks_only_after_a_failure() {
        let cwd = std::env::temp_dir();
        let missing = format!("engram-no-such-program-{}", uuid::Uuid::new_v4().simple());
        let check = |target: &str, program: &str| {
            let args = ["/c", target, "--flag"].map(str::to_owned);
            MissingProgramCheck::with_gate(
                program,
                &args,
                &[],
                &[],
                &cwd,
                cmd_lookup::LookupGate::own(),
            )
        };
        let absent = check(&missing, "cmd.exe");
        assert!(absent.judge(Some(9009), far()).is_missing());
        assert!(
            absent.judge(Some(1), far()).is_missing(),
            "없는 대상 + 실패"
        );
        assert!(!absent.judge(Some(0), far()).is_missing(), "성공");
        assert!(
            !absent.judge(Some(1), Instant::now()).is_missing(),
            "마감이 지나면 모른다 — 「없다」가 아니다"
        );
        let present = check("ping", r"C:\Windows\System32\CMD.EXE");
        assert!(present.judge(Some(9009), far()).is_missing());
        assert!(
            !present.judge(Some(1), far()).is_missing(),
            "있는 대상의 실패는 대상 자신의 실패다"
        );
        let direct = check(&missing, "other.exe");
        assert!(
            !direct.judge(Some(9009), far()).is_missing(),
            "cmd 가 아니다"
        );
        assert!(!direct.judge(Some(1), far()).is_missing());
    }

    /// 멈춘 채 띄운 자식은 깨어난 채로 넘어온다 — 대화가 되고, 멈춘 스레드가 하나도 남지 않았다(다시 깨우기 = 실패).
    #[cfg(windows)]
    #[test]
    fn the_suspended_spawn_hands_over_a_running_child() {
        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/v:on", "/c", "set /p L=& echo got:!L!"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        prepare_tree_root(&mut cmd);
        let mut child = cmd.spawn().expect("기동");
        let tree = TreeRoot::attach(&child).expect("무리에 넣기");
        tree.start(&child).expect("깨우기");
        assert!(
            crate::group::resume_suspended_process(child.id()).is_err(),
            "깨운 뒤에도 멈춘 스레드가 남았다"
        );

        let mut stdin = child.stdin.take().expect("stdin 파이프");
        stdin.write_all(b"hello\n").expect("쓰기");
        drop(stdin);
        let mut out = String::new();
        child
            .stdout
            .take()
            .expect("stdout 파이프")
            .read_to_string(&mut out)
            .expect("읽기");
        assert_eq!(out.lines().collect::<Vec<_>>(), ["got:hello"]);
        let _ = child.wait();
    }

    /// ★트리 전부가 무리 안이다★ — cmd 와 그 자식 `ping` 이 무리의 멤버다. 멈춘 채 띄워 넣은 뒤 깨우므로 곧바로 띄운
    ///   손자도 빠지지 않는다. 끊기 전에는 「다 끝났다」가 아니고, 끊은 뒤에야 무리가 비고 그 `ping` 이 끝난다 —
    ///   멤버 수를 못 읽거나 덜 세면 「다 끝났다」가 일찍 서서 부르는 쪽이 기다림을 일찍 멈춘다.
    #[cfg(windows)]
    #[test]
    fn the_tree_holds_every_member_until_it_is_killed() {
        use crate::testing::{is_ping, wait_until};

        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/v:on", "/c", "ping -n 120 127.0.0.1 > NUL"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        prepare_tree_root(&mut cmd);
        let mut child = cmd.spawn().expect("기동");
        let tree = match TreeRoot::attach(&child) {
            Ok(tree) => tree,
            Err(e) => {
                // 멈춘 채 무리 밖에 남은 자식을 거두고 실패한다.
                let _ = child.kill();
                let _ = child.wait();
                panic!("무리에 넣기: {e}");
            }
        };
        tree.start(&child).expect("깨우기");

        let members = tree.group.downgrade().expect("무리의 약한 손잡이");
        // 붙든 핸들로 끝남을 본다 — 번호로 보면 끝난 뒤 그 번호를 이어받은 남의 프로세스를 「산다」로 읽는다.
        let ping = wait_until("무리 안의 ping", || {
            let pid = members
                .member_pids()
                .ok()?
                .into_iter()
                .find(|&pid| is_ping(pid))?;
            members.pin_member(pid, false).ok()?
        });
        assert_eq!(ping.facts().ppid, child.id(), "cmd 가 띄운 ping 이 아니다");
        assert!(
            members.member_pids().expect("명단").contains(&child.id()),
            "cmd 가 무리 밖이다"
        );
        assert!(
            !ping.exited().expect("ping 생존 확인"),
            "kill 전에 ping 이 이미 끝났다"
        );
        assert!(tree.group.active_processes().expect("멤버 수") >= 2);
        assert!(!tree.members_gone(), "멤버가 사는데 다 끝났다고 했다");

        tree.kill(&mut child).expect("트리 kill");
        wait_until("무리가 빈다", || {
            matches!(tree.group.active_processes(), Ok(0)).then_some(())
        });
        assert!(
            ping.wait_exit(Duration::from_secs(10)).expect("ping 대기"),
            "트리 kill 뒤에도 그 ping 이 산다"
        );
        assert!(tree.members_gone());
        let _ = child.wait();
    }

    /// ★멈춘 적 없는 프로세스를 깨우라고 하면 실패한다★ — 멈춘 채 띄우기를 잃으면 기동이 실패로 드러난다.
    /// `ping` 을 셸 없이 직접 띄운다 — 셸을 거치면 셸만 끊겨 손자가 남는다(이 시험엔 무리가 없다).
    #[cfg(windows)]
    #[test]
    fn resuming_a_process_that_was_never_suspended_fails() {
        let mut cmd = Command::new("ping.exe");
        cmd.args(["-n", "30", "127.0.0.1"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_console_window(&mut cmd);
        let mut running = cmd.spawn().expect("기동");
        let result = crate::group::resume_suspended_process(running.id());
        let _ = running.kill();
        let _ = running.wait();
        assert!(result.is_err(), "멈추지 않은 프로세스를 깨웠다고 했다");
    }
}
