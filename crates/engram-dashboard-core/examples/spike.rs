// Phase 0 Spike — Windows PTY kill 시퀀스 실측 검증 (throwaway)
//
// 검증 가정:
//   1. portable-pty 0.8.1 로 Windows에서 child spawn + stdout read 가능
//   2. kill 시퀀스 후 drain(reader) 스레드가 5초 이내 EOF로 종료
//   3. Job Object + KILL_ON_JOB_CLOSE 로 손자 프로세스까지 정리
//
// 실행: cd src-tauri && cargo run --example spike

use std::io::Read;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

#[cfg(windows)]
mod job {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

    pub struct Job(pub HANDLE);

    impl Job {
        pub fn create() -> windows::core::Result<Job> {
            unsafe {
                let h = CreateJobObjectW(None, None)?;
                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    h,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )?;
                Ok(Job(h))
            }
        }

        pub fn assign(&self, pid: u32) -> windows::core::Result<()> {
            unsafe {
                let proc = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid)?;
                let r = AssignProcessToJobObject(self.0, proc);
                let _ = CloseHandle(proc);
                r
            }
        }

        pub fn terminate(&self, exit_code: u32) -> windows::core::Result<()> {
            unsafe { TerminateJobObject(self.0, exit_code) }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

fn main() {
    let t0 = Instant::now();
    eprintln!("[spike] start");

    // 1. PTY 생성
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty failed");
    eprintln!("[spike] openpty ok @ {:?}", t0.elapsed());

    // 2. child spawn (cmd.exe)
    let mut cmd = CommandBuilder::new("cmd.exe");
    cmd.cwd(std::env::current_dir().expect("cwd"));
    let mut child = pair.slave.spawn_command(cmd).expect("spawn_command failed");
    let pid = child.process_id();
    eprintln!("[spike] spawned child pid={:?} @ {:?}", pid, t0.elapsed());

    // 3. Job Object 생성 + assign (Windows)
    #[cfg(windows)]
    let job = {
        let job = job::Job::create().expect("CreateJobObject failed");
        eprintln!("[spike] job created @ {:?}", t0.elapsed());
        if let Some(pid) = pid {
            match job.assign(pid) {
                Ok(()) => eprintln!("[spike] assigned pid {} to job @ {:?}", pid, t0.elapsed()),
                Err(e) => eprintln!("[spike] WARN assign failed: {e:?}"),
            }
        }
        job
    };

    // slave는 더 이상 필요 없음 → drop (FD 누수 방지)
    drop(pair.slave);

    // 4. reader 스레드
    let mut reader = pair.master.try_clone_reader().expect("try_clone_reader");
    let (done_tx, done_rx) = mpsc::channel::<(u64, &'static str)>();
    let reader_t0 = Instant::now();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut total: u64 = 0;
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    eprintln!("[reader] EOF (read==0), total={total}B");
                    let _ = done_tx.send((total, "EOF"));
                    break;
                }
                Ok(n) => {
                    total += n as u64;
                    eprintln!("[reader] +{n}B (total={total})");
                }
                Err(e) => {
                    eprintln!("[reader] Err: {e} (total={total}B)");
                    let _ = done_tx.send((total, "Err"));
                    break;
                }
            }
        }
    });

    // 5. 2초간 출력 수신
    std::thread::sleep(Duration::from_secs(2));
    eprintln!(
        "[spike] --- 2s elapsed, begin kill sequence @ {:?} ---",
        t0.elapsed()
    );

    // 6. kill 시퀀스 (각 단계 타임스탬프)
    let k0 = Instant::now();
    match child.kill() {
        Ok(()) => eprintln!("[kill] child.kill() ok @ +{:?}", k0.elapsed()),
        Err(e) => eprintln!("[kill] child.kill() Err: {e} @ +{:?}", k0.elapsed()),
    }
    match child.wait() {
        Ok(status) => eprintln!("[kill] child.wait() -> {status:?} @ +{:?}", k0.elapsed()),
        Err(e) => eprintln!("[kill] child.wait() Err: {e} @ +{:?}", k0.elapsed()),
    }
    #[cfg(windows)]
    {
        match job.terminate(1) {
            Ok(()) => eprintln!("[kill] TerminateJobObject ok @ +{:?}", k0.elapsed()),
            Err(e) => eprintln!("[kill] TerminateJobObject Err: {e:?} @ +{:?}", k0.elapsed()),
        }
    }
    drop(pair.master);
    eprintln!("[kill] drop(master) @ +{:?}", k0.elapsed());

    // 7. reader 종료 대기 (5초 타임아웃)
    match done_rx.recv_timeout(Duration::from_secs(5)) {
        Ok((total, why)) => {
            eprintln!(
                "[spike] ✅ reader 종료 ({why}) — join {:?} after master-drop, kill-seq {:?}, total {}B",
                reader_t0.elapsed(),
                k0.elapsed(),
                total
            );
            eprintln!("[spike] RESULT: PASS");
        }
        Err(_) => {
            eprintln!("[spike] ❌ reader 5초 내 종료 안 됨 — 가정 2 깨짐");
            eprintln!("[spike] RESULT: FAIL");
        }
    }

    eprintln!("[spike] total {:?}", t0.elapsed());
}
