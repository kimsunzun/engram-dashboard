// S12 Spike #1 — Windows Job Object breakaway 실측 (throwaway)
//
// 데몬화 성패의 단일 장애점(daemon-design.md §4·§6)을 검증한다.
//
// 검증 가정:
//   A. 부모(=현재 셸/IDE) Job 안에서 자식을 CREATE_BREAKAWAY_FROM_JOB|DETACHED_PROCESS로
//      spawn 하면, 자식이 부모 Job에서 분리(breakaway)되는가?
//      → IsProcessInJob(child) == false 이면 분리 성공.
//   B. 부모를 KILL_ON_JOB_CLOSE Job에 넣고 자살시켜도, breakaway 한 자식은 생존하는가?
//      → 자식이 marker 파일에 "SURVIVED" 를 남기면 생존 입증.
//   C. breakaway 가 실패(부모 Job 이 JOB_OBJECT_LIMIT_BREAKAWAY_OK 불허)할 때
//      fallback `cmd /c start /b` 경유 spawn 이 분리에 성공하는가?
//
// 데몬 모드는 데몬이 PTY child 를 직접 자기 Job(KILL_ON_JOB_CLOSE)에 담으므로
// "Job 소유권 이전" 은 검증 대상이 아니다(설계 §4: 환상이라 삭제). 이 spike 는 오직
// "데몬을 부모에서 떼어내 살리는" breakaway 한 점만 본다.
//
// 실행(오케스트레이션은 scripts 가 함):
//   cargo run --example spike_breakaway -- diag        # 테스트 A·C 진단(출력 판독)
//   cargo run --example spike_breakaway -- selfkill <marker>  # 테스트 B(부모 자살 후 자식 생존)
//   cargo run --example spike_breakaway -- child <marker>     # 자식 본체(직접 호출 안 함)

use std::fs::OpenOptions;
use std::io::Write as _;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::time::Duration;

// Win32 process creation flags (winbase.h). windows crate 상수 대신 raw 로 고정.
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

#[cfg(windows)]
mod win {
    use windows::Win32::Foundation::{BOOL, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, QueryInformationJobObject,
        SetInformationJobObject, TerminateJobObject, JobObjectExtendedLimitInformation,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// 주어진 프로세스 핸들이 (어떤) Job 에 속해 있는지. job=None → "any job".
    pub fn is_in_any_job(process: HANDLE) -> windows::core::Result<bool> {
        let mut result = BOOL(0);
        unsafe { IsProcessInJob(process, None, &mut result)? };
        Ok(result.as_bool())
    }

    /// 현재 프로세스 의사 핸들(닫을 필요 없음).
    pub fn current_process() -> HANDLE {
        unsafe { GetCurrentProcess() }
    }

    /// pid 로 조회 전용 핸들 open(쿼리 최소 권한). 호출자가 CloseHandle 책임 — 여기선
    /// spike 라 프로세스 종료 시 OS 가 정리하므로 명시적 close 생략.
    pub fn open_for_query(pid: u32) -> windows::core::Result<HANDLE> {
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
    }

    /// KILL_ON_JOB_CLOSE Job 생성.
    pub fn create_kill_on_close_job() -> windows::core::Result<HANDLE> {
        unsafe {
            let h = CreateJobObjectW(None, None)?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                core::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )?;
            Ok(h)
        }
    }

    pub fn assign(job: HANDLE, process: HANDLE) -> windows::core::Result<()> {
        unsafe { AssignProcessToJobObject(job, process) }
    }

    /// 현재 프로세스가 속한 Job 의 LimitFlags 조회(hjob=None → 호출 프로세스의 현재 Job).
    /// 반환: raw LimitFlags 비트. 실패 시 Err.
    pub fn current_job_limit_flags() -> windows::core::Result<u32> {
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        let mut ret_len: u32 = 0;
        unsafe {
            QueryInformationJobObject(
                None,
                JobObjectExtendedLimitInformation,
                &mut info as *mut _ as *mut core::ffi::c_void,
                core::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                Some(&mut ret_len),
            )?;
        }
        Ok(info.BasicLimitInformation.LimitFlags.0)
    }

    pub fn terminate(job: HANDLE, code: u32) -> windows::core::Result<()> {
        unsafe { TerminateJobObject(job, code) }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(String::as_str).unwrap_or("diag");
    match mode {
        "child" => run_child(args.get(2).expect("child needs marker path")),
        "selfkill" => run_selfkill(args.get(2).expect("selfkill needs marker path")),
        "wmikill" => run_wmikill(args.get(2).expect("wmikill needs marker path")),
        "checkjob" => run_checkjob(args.get(2).expect("checkjob needs pid")),
        _ => run_diag(),
    }
}

/// 자식 본체. 시작 즉시 ALIVE, 6초 뒤 SURVIVED 를 marker 에 append.
/// 부모가 그 사이 죽어도 breakaway 했으면 SURVIVED 가 찍힌다.
fn run_child(marker: &str) {
    let pid = std::process::id();
    append_line(marker, &format!("ALIVE pid={pid}"));
    std::thread::sleep(Duration::from_secs(6));
    append_line(marker, &format!("SURVIVED pid={pid}"));
}

fn append_line(path: &str, line: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

/// 테스트 A·C: 진단. 부모 Job 상태 + breakaway spawn 으로 자식이 분리되는지 + fallback.
fn run_diag() {
    println!("=== spike_breakaway: DIAG ===");

    #[cfg(not(windows))]
    {
        println!("non-windows: SKIP");
    }

    #[cfg(windows)]
    {
        // 0. 부모(=이 프로세스) 가 이미 Job 안에 있나? (IDE/터미널이 넣었을 수 있음)
        let parent_in_job = win::is_in_any_job(win::current_process())
            .map(|b| b.to_string())
            .unwrap_or_else(|e| format!("ERR {e:?}"));
        println!("[A] parent_in_job = {parent_in_job}");

        // 0-b. 부모 Job 의 LimitFlags — KILL_ON_JOB_CLOSE / BREAKAWAY_OK 여부가 판정의 핵심.
        const KILL_ON_JOB_CLOSE: u32 = 0x2000;
        const BREAKAWAY_OK: u32 = 0x0800;
        const SILENT_BREAKAWAY_OK: u32 = 0x1000;
        match win::current_job_limit_flags() {
            Ok(flags) => {
                println!("[A] parent_job_limit_flags = 0x{flags:04x}");
                println!(
                    "[A]   KILL_ON_JOB_CLOSE={}  BREAKAWAY_OK={}  SILENT_BREAKAWAY_OK={}",
                    flags & KILL_ON_JOB_CLOSE != 0,
                    flags & BREAKAWAY_OK != 0,
                    flags & SILENT_BREAKAWAY_OK != 0,
                );
                if flags & KILL_ON_JOB_CLOSE == 0 {
                    println!("[A]   => 부모 Job 이 KILL_ON_JOB_CLOSE 아님 → 상속돼도 부모 종료 시 자식 생존(데몬화 무해) ✅");
                } else if flags & (BREAKAWAY_OK | SILENT_BREAKAWAY_OK) != 0 {
                    println!("[A]   => KILL_ON_JOB_CLOSE 이지만 breakaway 허용 → CREATE_BREAKAWAY 로 분리 가능 ✅");
                } else {
                    println!("[A]   => KILL_ON_JOB_CLOSE + breakaway 불허 → 데몬 분리 불가(worst-case) ❌");
                }
            }
            Err(e) => println!("[A] parent_job_limit_flags 조회 실패: {e:?}"),
        }

        let exe = std::env::current_exe().expect("current_exe");
        let marker_dir = std::env::temp_dir();

        // 1. breakaway spawn — 자식을 부모 Job 에서 분리 시도.
        let marker_a = marker_dir.join("engram_spike_diag_a.txt");
        let _ = std::fs::remove_file(&marker_a);
        let flags = CREATE_BREAKAWAY_FROM_JOB | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
        let spawn_a = Command::new(&exe)
            .arg("child")
            .arg(&marker_a)
            .creation_flags(flags)
            .spawn();

        match spawn_a {
            Ok(child) => {
                let pid = child.id();
                std::thread::sleep(Duration::from_millis(300));
                let in_job = win::open_for_query(pid)
                    .and_then(win::is_in_any_job)
                    .map(|b| b.to_string())
                    .unwrap_or_else(|e| format!("ERR {e:?}"));
                println!("[A] breakaway spawn OK, child pid={pid}, child_in_job={in_job}");
                if in_job == "false" {
                    println!("[A] => BREAKAWAY SUCCESS (자식이 부모 Job 에서 분리됨) ✅");
                } else if in_job == "true" {
                    println!("[A] => BREAKAWAY FAILED (자식이 여전히 Job 안 — fallback 필요) ⚠");
                } else {
                    println!("[A] => 판정 불가 ({in_job})");
                }
            }
            Err(e) => {
                // spawn 자체가 막힘 = 부모 Job 이 breakaway 불허(JOB_OBJECT_LIMIT_BREAKAWAY_OK X)
                println!("[A] breakaway spawn FAILED: {e} (errno={:?}) ⚠", e.raw_os_error());
                println!("[A] => 부모 Job 이 breakaway 불허일 가능성 — fallback 검증 필수");
            }
        }

        // 2. fallback: cmd /c start /b — start 가 새 프로세스를 부모 job 밖에서 띄우는지.
        let marker_c = marker_dir.join("engram_spike_diag_c.txt");
        let _ = std::fs::remove_file(&marker_c);
        let exe_str = exe.to_string_lossy().to_string();
        let marker_c_str = marker_c.to_string_lossy().to_string();
        // cmd /c start "" /b <exe> child <marker>
        let spawn_c = Command::new("cmd")
            .args([
                "/c",
                "start",
                "",
                "/b",
                &exe_str,
                "child",
                &marker_c_str,
            ])
            .creation_flags(DETACHED_PROCESS)
            .spawn();
        match spawn_c {
            Ok(_) => {
                // start 는 즉시 반환하고 손자가 뜬다. marker 가 곧 생기는지 확인.
                std::thread::sleep(Duration::from_millis(800));
                let alive = std::fs::read_to_string(&marker_c).unwrap_or_default();
                println!(
                    "[C] fallback cmd/start/b spawn OK, marker_contains_ALIVE={}",
                    alive.contains("ALIVE")
                );
                println!("[C] (Job 분리 여부는 selfkill 테스트로 최종 확인)");
            }
            Err(e) => println!("[C] fallback spawn FAILED: {e}"),
        }

        std::thread::sleep(Duration::from_millis(500));
        println!("=== DIAG 끝 (자식 marker 는 ~6초 뒤 SURVIVED 추가) ===");
    }
}

/// 주어진 pid 가 (어떤) Job 에 속해 있는지 출력. WMI 로 띄운 프로세스의 분리 여부 확인용.
fn run_checkjob(pid_str: &str) {
    #[cfg(not(windows))]
    {
        let _ = pid_str;
        println!("non-windows: SKIP");
    }
    #[cfg(windows)]
    {
        let pid: u32 = pid_str.parse().expect("pid must be u32");
        match win::open_for_query(pid).and_then(win::is_in_any_job) {
            Ok(b) => println!("checkjob pid={pid} in_job={b}"),
            Err(e) => println!("checkjob pid={pid} ERR {e:?}"),
        }
    }
}

/// 테스트 D: WMI 우회. 부모가 KILL_ON_JOB_CLOSE Job 에 자기를 넣고, 자식을 WMI
/// (Win32_Process.Create) 로 띄운다. WmiPrvSE 가 부모가 되어 호출자 Job 을 상속하지 않으므로,
/// 부모가 자살(TerminateJobObject)해도 WMI 자식은 살아 SURVIVED 를 남겨야 한다.
/// breakaway·start/b 가 막힌 worst-case Job 의 마지막 우회책.
fn run_wmikill(marker: &str) {
    let _ = std::fs::remove_file(marker);

    #[cfg(not(windows))]
    {
        let _ = marker;
        println!("non-windows: SKIP");
    }

    #[cfg(windows)]
    {
        println!("=== spike_breakaway: WMIKILL (marker={marker}) ===");
        let job = win::create_kill_on_close_job().expect("create job");
        match win::assign(job, win::current_process()) {
            Ok(()) => println!("[D] 부모를 KILL_ON_JOB_CLOSE Job 에 assign 함"),
            Err(e) => println!("[D] WARN 부모 assign 실패: {e:?}"),
        }

        let exe = std::env::current_exe().expect("current_exe");
        let exe_str = exe.to_string_lossy().replace('\'', "''");
        let marker_q = marker.replace('\'', "''");
        // PowerShell 로 WMI Create 호출. WmiPrvSE 가 실제 부모 → 우리 Job 미상속.
        // CommandLine 은 단일 문자열: "<exe>" child "<marker>"
        let ps = format!(
            "$cl = '\"{exe_str}\" child \"{marker_q}\"'; \
             $r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{{ CommandLine = $cl }}; \
             Write-Output (\"WMI ReturnValue=\" + $r.ReturnValue + \" Pid=\" + $r.ProcessId)"
        );
        let out = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
            .creation_flags(DETACHED_PROCESS)
            .output();
        match out {
            Ok(o) => {
                println!("[D] {}", String::from_utf8_lossy(&o.stdout).trim());
                if !o.stderr.is_empty() {
                    println!("[D] stderr: {}", String::from_utf8_lossy(&o.stderr).trim());
                }
            }
            Err(e) => println!("[D] WMI 호출 실패: {e}"),
        }

        std::thread::sleep(Duration::from_millis(1200)); // 자식 ALIVE 기록 시간
        println!("[D] 부모 자살(TerminateJobObject) — 외부에서 marker SURVIVED 확인");
        let _ = win::terminate(job, 1);
        std::thread::sleep(Duration::from_secs(2));
        println!("[D] (여기 출력되면 자살 실패)");
    }
}

/// 테스트 B: 부모가 KILL_ON_JOB_CLOSE Job 에 자기를 넣고 자식을 breakaway spawn 한 뒤
/// TerminateJobObject 로 자살. breakaway 한 자식만 살아 SURVIVED 를 남겨야 한다.
fn run_selfkill(marker: &str) {
    let _ = std::fs::remove_file(marker);

    #[cfg(not(windows))]
    {
        let _ = marker;
        println!("non-windows: SKIP");
    }

    #[cfg(windows)]
    {
        println!("=== spike_breakaway: SELFKILL (marker={marker}) ===");
        let job = win::create_kill_on_close_job().expect("create job");
        // 부모(자신)를 KILL_ON_JOB_CLOSE Job 에 편입 — Tauri 가 자기 PTY job 안에 있는 상황 모사.
        match win::assign(job, win::current_process()) {
            Ok(()) => println!("[B] 부모를 KILL_ON_JOB_CLOSE Job 에 assign 함"),
            Err(e) => println!("[B] WARN 부모 assign 실패(이미 breakaway 불가 nested?): {e:?}"),
        }

        let exe = std::env::current_exe().expect("current_exe");
        let flags = CREATE_BREAKAWAY_FROM_JOB | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
        let child = Command::new(&exe)
            .arg("child")
            .arg(marker)
            .creation_flags(flags)
            .spawn();
        match child {
            Ok(c) => println!("[B] breakaway 자식 spawn pid={}", c.id()),
            Err(e) => {
                println!("[B] breakaway spawn 실패: {e} — fallback 으로 재시도");
                let exe_str = exe.to_string_lossy().to_string();
                let _ = Command::new("cmd")
                    .args(["/c", "start", "", "/b", &exe_str, "child", marker])
                    .creation_flags(DETACHED_PROCESS)
                    .spawn();
            }
        }

        // 자식이 ALIVE 찍을 시간.
        std::thread::sleep(Duration::from_millis(800));
        println!("[B] 이제 부모 자살(TerminateJobObject) — 외부에서 marker 확인할 것");
        let _ = win::terminate(job, 1); // job 안의 모든 프로세스(=부모) 종료. breakaway 자식은 제외.
        // 도달 못 할 수 있음(자기 자신 종료).
        std::thread::sleep(Duration::from_secs(2));
        println!("[B] (여기 출력되면 자살 실패 — 부모가 job 에 안 들어갔을 수 있음)");
    }
}
