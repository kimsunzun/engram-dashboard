// S12 Spike #1 — measured Windows Job Object breakaway (throwaway)
//
// Verifies the single point of failure for daemonization success (daemon-design.md §4·§6).
//
// Assumptions to verify:
//   A. If a child is spawned from inside the parent (= the current shell/IDE) Job with CREATE_BREAKAWAY_FROM_JOB|DETACHED_PROCESS,
//      does the child break away from the parent Job?
//      → if IsProcessInJob(child) == false, the breakaway succeeded.
//   B. Even if the parent is put in a KILL_ON_JOB_CLOSE Job and kills itself, does the child that broke away survive?
//      → if the child leaves "SURVIVED" in the marker file, survival is proven.
//   C. When breakaway fails (the parent Job does not allow JOB_OBJECT_LIMIT_BREAKAWAY_OK),
//      does a spawn through the fallback `cmd /c start /b` succeed in breaking away?
//
// In daemon mode the daemon puts the PTY child directly into its own Job (KILL_ON_JOB_CLOSE), so
// "Job ownership transfer" is not under verification (design §4: deleted as an illusion). This spike looks only at
// the single point of breakaway, "detaching the daemon from its parent so it survives".
//
// Run (orchestration is done by scripts):
//   cargo run --example spike_breakaway -- diag        # tests A·C diagnosis (read the output)
//   cargo run --example spike_breakaway -- selfkill <marker>  # test B (child survives after the parent kills itself)
//   cargo run --example spike_breakaway -- child <marker>     # child body (not called directly)

use std::fs::OpenOptions;
use std::io::Write as _;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::time::Duration;

// Win32 process creation flags (winbase.h). Fixed as raw values instead of windows crate constants.
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

#[cfg(windows)]
mod win {
    use windows::Win32::Foundation::{BOOL, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob,
        JobObjectExtendedLimitInformation, QueryInformationJobObject, SetInformationJobObject,
        TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// job=None → "any job".
    pub fn is_in_any_job(process: HANDLE) -> windows::core::Result<bool> {
        let mut result = BOOL(0);
        unsafe { IsProcessInJob(process, None, &mut result)? };
        Ok(result.as_bool())
    }

    /// Pseudo-handle of the current process (no need to close it).
    pub fn current_process() -> HANDLE {
        unsafe { GetCurrentProcess() }
    }

    /// The caller is responsible for CloseHandle — this is a spike and the OS cleans up at process exit, so the explicit close is omitted.
    pub fn open_for_query(pid: u32) -> windows::core::Result<HANDLE> {
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
    }

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

    /// hjob=None → the calling process's current Job.
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

/// Child body. Appends ALIVE to the marker right at start and SURVIVED 6 seconds later.
/// Even if the parent dies in between, SURVIVED is written if it broke away.
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

/// Tests A·C: diagnosis. Parent Job state + whether the child breaks away via breakaway spawn + fallback.
fn run_diag() {
    println!("=== spike_breakaway: DIAG ===");

    #[cfg(not(windows))]
    {
        println!("non-windows: SKIP");
    }

    #[cfg(windows)]
    {
        // 0. Is the parent (= this process) already inside a Job? (the IDE/terminal may have put it there)
        let parent_in_job = win::is_in_any_job(win::current_process())
            .map(|b| b.to_string())
            .unwrap_or_else(|e| format!("ERR {e:?}"));
        println!("[A] parent_in_job = {parent_in_job}");

        // 0-b. The parent Job's LimitFlags — whether KILL_ON_JOB_CLOSE / BREAKAWAY_OK are set is the key to the decision.
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

        // 1. breakaway spawn
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
                println!(
                    "[A] breakaway spawn FAILED: {e} (errno={:?}) ⚠",
                    e.raw_os_error()
                );
                println!("[A] => 부모 Job 이 breakaway 불허일 가능성 — fallback 검증 필수");
            }
        }

        // 2. fallback: cmd /c start /b
        let marker_c = marker_dir.join("engram_spike_diag_c.txt");
        let _ = std::fs::remove_file(&marker_c);
        let exe_str = exe.to_string_lossy().to_string();
        let marker_c_str = marker_c.to_string_lossy().to_string();
        let spawn_c = Command::new("cmd")
            .args(["/c", "start", "", "/b", &exe_str, "child", &marker_c_str])
            .creation_flags(DETACHED_PROCESS)
            .spawn();
        match spawn_c {
            Ok(_) => {
                // start returns immediately and the grandchild comes up.
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

/// Print whether the given pid belongs to (any) Job. For checking whether a process launched via WMI broke away.
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

/// Test D: WMI bypass. The parent puts itself in a KILL_ON_JOB_CLOSE Job and launches the child via WMI
/// (Win32_Process.Create). WmiPrvSE becomes the parent and does not inherit the caller's Job, so
/// even if the parent kills itself (TerminateJobObject), the WMI child should survive and leave SURVIVED.
/// The last workaround for the worst-case Job where breakaway · start/b are blocked.
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

        std::thread::sleep(Duration::from_millis(1200)); // Time the child's ALIVE was recorded
        println!("[D] 부모 자살(TerminateJobObject) — 외부에서 marker SURVIVED 확인");
        let _ = win::terminate(job, 1);
        std::thread::sleep(Duration::from_secs(2));
        println!("[D] (여기 출력되면 자살 실패)");
    }
}

/// Test B: the parent puts itself in a KILL_ON_JOB_CLOSE Job, spawns the child with breakaway, then kills itself with
/// TerminateJobObject. Only the child that broke away should survive and leave SURVIVED.
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
        // Put the parent (itself) into a KILL_ON_JOB_CLOSE Job — simulates the situation where Tauri is inside its own PTY job.
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

        // Time for the child to write ALIVE.
        std::thread::sleep(Duration::from_millis(800));
        println!("[B] 이제 부모 자살(TerminateJobObject) — 외부에서 marker 확인할 것");
        let _ = win::terminate(job, 1); // Terminate every process in the job (= the parent). The breakaway child is excluded.
                                        // May not be reached (it terminates itself).
        std::thread::sleep(Duration::from_secs(2));
        println!("[B] (여기 출력되면 자살 실패 — 부모가 job 에 안 들어갔을 수 있음)");
    }
}
