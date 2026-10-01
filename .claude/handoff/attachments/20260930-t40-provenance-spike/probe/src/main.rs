//! T-40 measurement spike (scratch only — not repo code).
//!
//! Mirrors the dashboard's claude JSON spawn exactly:
//!   - program/args = console_command("claude", <StreamJson args>) = `cmd.exe /c claude …`
//!   - std::process::Command, piped stdin/stdout/stderr, CREATE_NO_WINDOW (stdio.rs:90-108)
//!   - spawn, take pipes, THEN JobObjectHandle::new() (KILL_ON_JOB_CLOSE only), THEN assign(pid)
//!     (stdio.rs:110-127 · platform/windows.rs:30-75)
//! Measurement-only additions (do not change membership semantics):
//!   - a job completion port associated AFTER assign (exact NEW/EXIT membership events)
//!   - read-only snapshots; the only kill paths are (a) TRD §3-5 terminate_member (same-handle
//!     IsProcessInJob(our job) + creation-time match + STILL_ACTIVE) and (b) TerminateJobObject(our job).
//!
//! 2026-09-30 extension (provenance spike, copy of the 2026-09-29 harness):
//!   - A: follow-up after a HANG waits 120 s; on follow-up timeout a stall capture (job members +
//!     chains, pending hook ids, command_lifecycle, last stream lines, then one interrupt) is written.
//!   - B: per claude process one foreground Bash-tool turn (T40FG=1; sleep 5; echo done) and one
//!     run_in_background Bash-tool turn (T40BG=1; while true; do sleep 2; done), observation window,
//!     then one report-only interrupt trial with the background loop alive (kills only hook-chain
//!     candidates if it hangs).
//!   - every stdout line carries the wall clock at receipt; every job record is dumped to pN.procs.tsv
//!     (creation/exit FILETIME, ppid, image, full cmdline) for offline correlation.
//!   - cleanup also terminates any process OUTSIDE our job whose cmdline carries T40BG=1/T40FG=1
//!     (only our own markers; reported).
#![allow(non_snake_case)]

use std::collections::VecDeque;
use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::core::PWSTR;
use windows::Wdk::System::Threading::{NtQueryInformationProcess, PROCESSINFOCLASS};
use windows::Win32::Foundation::{CloseHandle, BOOL, FILETIME, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, QueryInformationJobObject,
    SetInformationJobObject, TerminateJobObject, JobObjectExtendedLimitInformation, JOBOBJECTINFOCLASS,
    JOBOBJECT_ASSOCIATE_COMPLETION_PORT, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
    TerminateProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
    PROCESS_TERMINATE,
};
use windows::Win32::System::IO::{CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED};

const WRAPPER_DEPTH: usize = 1; // console_wrapper_depth() on Windows per TRD §3-4
const STILL_ACTIVE: u32 = 259;

// ───────────────────────── small utils ─────────────────────────

#[derive(Clone, Copy)]
struct H(isize);
unsafe impl Send for H {}
unsafe impl Sync for H {}
impl H {
    fn h(self) -> HANDLE {
        HANDLE(self.0 as *mut c_void)
    }
    fn of(h: HANDLE) -> H {
        H(h.0 as isize)
    }
}
fn null_h() -> HANDLE {
    HANDLE(std::ptr::null_mut())
}
fn close(h: H) {
    unsafe {
        let _ = CloseHandle(h.h());
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Ident {
    pid: u32,
    start: u64,
}

const EPOCH_DIFF: u64 = 116_444_736_000_000_000;
fn wall_ft() -> u64 {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    EPOCH_DIFF + (d.as_nanos() / 100) as u64
}
fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}
/// (b - a) in ms
fn dms(a: u64, b: u64) -> f64 {
    (b as i64 - a as i64) as f64 / 10_000.0
}
fn ms_since(t0: Instant) -> f64 {
    t0.elapsed().as_secs_f64() * 1000.0
}
fn sleep_ms(ms: f64) {
    if ms > 0.0 {
        thread::sleep(Duration::from_micros((ms * 1000.0) as u64));
    }
}
fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}
fn base(p: &str) -> String {
    p.rsplit(['\\', '/']).next().unwrap_or(p).to_string()
}
fn field(line: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":\"");
    let i = line.find(&pat)? + pat.len();
    let j = line[i..].find('"')?;
    Some(line[i..i + j].to_string())
}
/// top-level (depth-1) string field of a JSON object line — key order independent.
fn top_field(line: &str, key: &str) -> Option<String> {
    let b = line.as_bytes();
    let mut depth = 0i32;
    let mut i = 0usize;
    let mut expect_key = false;
    while i < b.len() {
        match b[i] {
            b'{' | b'[' => {
                depth += 1;
                expect_key = b[i] == b'{' && depth == 1;
                i += 1;
            }
            b'}' | b']' => {
                depth -= 1;
                i += 1;
            }
            b',' => {
                expect_key = depth == 1;
                i += 1;
            }
            b'"' => {
                let s = i + 1;
                let mut j = s;
                while j < b.len() && b[j] != b'"' {
                    if b[j] == b'\\' {
                        j += 1;
                    }
                    j += 1;
                }
                let tok = &line[s..j.min(b.len())];
                i = j + 1;
                if depth == 1 && expect_key {
                    expect_key = false;
                    if tok == key {
                        // skip ws and ':'
                        while i < b.len() && (b[i] == b':' || b[i] == b' ') {
                            i += 1;
                        }
                        if i < b.len() && b[i] == b'"' {
                            let vs = i + 1;
                            let mut k = vs;
                            while k < b.len() && b[k] != b'"' {
                                if b[k] == b'\\' {
                                    k += 1;
                                }
                                k += 1;
                            }
                            return Some(line[vs..k.min(b.len())].to_string());
                        }
                        return None;
                    }
                }
            }
            _ => i += 1,
        }
    }
    None
}
static CNT: AtomicU64 = AtomicU64::new(1);
fn uuid() -> String {
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64;
    let mut x = n ^ CNT.fetch_add(1, Ordering::SeqCst).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ ((std::process::id() as u64) << 32)
        | 1;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let a = next();
    let b = next();
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (a >> 32) as u32,
        (a >> 16) as u16,
        (a & 0xfff) as u16,
        ((b >> 48) as u16 & 0x3fff) | 0x8000,
        b & 0xffff_ffff_ffff
    )
}

// ───────────────────────── process queries ─────────────────────────

fn open_q(pid: u32) -> Option<HANDLE> {
    if pid == 0 {
        return None;
    }
    unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()
}
fn times(h: HANDLE) -> Option<(u64, u64)> {
    let (mut c, mut e, mut k, mut u) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    unsafe { GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) }.ok()?;
    Some((ft(c), ft(e)))
}
fn exit_code(h: HANDLE) -> Option<u32> {
    let mut c = 0u32;
    unsafe { GetExitCodeProcess(h, &mut c) }.ok()?;
    Some(c)
}
fn is_alive(h: HANDLE) -> bool {
    exit_code(h) == Some(STILL_ACTIVE)
}
fn in_job(h: HANDLE, job: HANDLE) -> Option<bool> {
    let mut b = BOOL(0);
    unsafe { IsProcessInJob(h, job, &mut b) }.ok()?;
    Some(b.as_bool())
}
fn image(h: HANDLE) -> String {
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    if unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len) }
        .is_ok()
    {
        String::from_utf16_lossy(&buf[..len as usize])
    } else {
        String::new()
    }
}
#[repr(C)]
struct Pbi {
    exit_status: i32,
    peb: *mut c_void,
    affinity: usize,
    base_pri: i32,
    pid: usize,
    ppid: usize,
}
fn ppid_of(h: HANDLE) -> Option<u32> {
    let mut p: Pbi = unsafe { std::mem::zeroed() };
    let mut ret = 0u32;
    let st = unsafe {
        NtQueryInformationProcess(
            h,
            PROCESSINFOCLASS(0),
            &mut p as *mut _ as *mut c_void,
            std::mem::size_of::<Pbi>() as u32,
            &mut ret,
        )
    };
    if st.0 >= 0 {
        Some(p.ppid as u32)
    } else {
        None
    }
}
fn cmdline(h: HANDLE) -> String {
    let mut buf = vec![0u64; 8192];
    let mut ret = 0u32;
    let st = unsafe {
        NtQueryInformationProcess(
            h,
            PROCESSINFOCLASS(60),
            buf.as_mut_ptr() as *mut c_void,
            (buf.len() * 8) as u32,
            &mut ret,
        )
    };
    if st.0 < 0 {
        return String::new();
    }
    unsafe {
        let len = *(buf.as_ptr() as *const u16) as usize;
        let ptr = *((buf.as_ptr() as *const u8).add(8) as *const *const u16);
        if ptr.is_null() {
            return String::new();
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len / 2))
    }
}
fn creation(pid: u32) -> Option<u64> {
    let h = open_q(pid)?;
    let r = times(h).map(|t| t.0);
    close(H::of(h));
    r
}

struct SRow {
    pid: u32,
    ppid: u32,
    exe: String,
}
fn snapshot_all() -> Vec<SRow> {
    let mut out = Vec::new();
    let snap = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) } {
        Ok(h) => h,
        Err(_) => return out,
    };
    let mut e = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    if unsafe { Process32FirstW(snap, &mut e) }.is_ok() {
        loop {
            let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(260);
            out.push(SRow {
                pid: e.th32ProcessID,
                ppid: e.th32ParentProcessID,
                exe: String::from_utf16_lossy(&e.szExeFile[..n]),
            });
            if unsafe { Process32NextW(snap, &mut e) }.is_err() {
                break;
            }
        }
    }
    close(H::of(snap));
    out
}
/// same as engram_dashboard_base::platform::child_pids (fresh snapshot per call)
fn child_pids(parent: u32) -> Vec<u32> {
    snapshot_all().into_iter().filter(|r| r.ppid == parent).map(|r| r.pid).collect()
}

// ───────────────────────── job object ─────────────────────────

/// Exact copy of JobObjectHandle::new (platform/windows.rs:30-56).
fn job_new() -> Result<H, String> {
    let handle = unsafe { CreateJobObjectW(None, None) }.map_err(|e| e.to_string())?;
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let r = unsafe {
        SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if let Err(e) = r {
        close(H::of(handle));
        return Err(e.to_string());
    }
    Ok(H::of(handle))
}
/// Exact copy of JobObjectHandle::assign (platform/windows.rs:58-75).
fn job_assign(job: H, pid: u32) -> Result<(), String> {
    let p = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid) }
        .map_err(|e| e.to_string())?;
    let r = unsafe { AssignProcessToJobObject(job.h(), p) };
    close(H::of(p));
    r.map_err(|e| e.to_string())
}
fn assoc_port(job: H) -> Result<H, String> {
    let port = unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, null_h(), 0, 1) }
        .map_err(|e| e.to_string())?;
    let info = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
        CompletionKey: 1usize as *mut c_void,
        CompletionPort: port,
    };
    unsafe {
        SetInformationJobObject(
            job.h(),
            JOBOBJECTINFOCLASS(7),
            &info as *const _ as *const c_void,
            std::mem::size_of::<JOBOBJECT_ASSOCIATE_COMPLETION_PORT>() as u32,
        )
    }
    .map_err(|e| e.to_string())?;
    Ok(H::of(port))
}
/// TRD §3-5 member_pids: QueryInformationJobObject(JobObjectBasicProcessIdList), grow once on more-data.
fn job_members(job: H) -> Vec<u32> {
    let mut cap = 64usize;
    for _ in 0..4 {
        let mut buf = vec![0usize; 1 + cap];
        let size = (buf.len() * std::mem::size_of::<usize>()) as u32;
        let r = unsafe {
            QueryInformationJobObject(
                job.h(),
                JOBOBJECTINFOCLASS(3),
                buf.as_mut_ptr() as *mut c_void,
                size,
                None,
            )
        };
        let hdr = buf.as_ptr() as *const u32;
        let (assigned, inlist) = unsafe { (*hdr, *hdr.add(1)) };
        if r.is_ok() {
            return (0..inlist as usize).map(|i| buf[1 + i] as u32).collect();
        }
        if assigned as usize > cap {
            cap = assigned as usize + 16;
            continue;
        }
        return Vec::new();
    }
    Vec::new()
}
/// TRD §3-5 terminate_member — verify with the SAME handle before TerminateProcess.
fn terminate_member(job: H, pid: u32, expected_start: u64) -> String {
    let h = match unsafe {
        OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
    } {
        Ok(h) => h,
        Err(_) => return "Gone(open failed)".into(),
    };
    let res = (|| {
        match in_job(h, job.h()) {
            Some(true) => {}
            Some(false) => return "NotOurs(not in job)".to_string(),
            None => return "Err(IsProcessInJob)".to_string(),
        }
        match times(h) {
            Some((c, _)) if c == expected_start => {}
            _ => return "NotOurs(start mismatch)".to_string(),
        }
        if exit_code(h) != Some(STILL_ACTIVE) {
            return "Gone(exited)".to_string();
        }
        match unsafe { TerminateProcess(h, 1) } {
            Ok(()) => "Terminated".to_string(),
            Err(e) => format!("Err({e})"),
        }
    })();
    close(H::of(h));
    res
}
fn self_job() -> String {
    let mut b = BOOL(0);
    let ok = unsafe { IsProcessInJob(GetCurrentProcess(), null_h(), &mut b) }.is_ok();
    if !ok {
        return "IsProcessInJob(self) failed".into();
    }
    if !b.as_bool() {
        return "harness in job: no".into();
    }
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    let r = unsafe {
        QueryInformationJobObject(
            null_h(),
            JobObjectExtendedLimitInformation,
            &mut info as *mut _ as *mut c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            None,
        )
    };
    match r {
        Ok(()) => format!(
            "harness in job: yes; its job LimitFlags=0x{:x}",
            info.BasicLimitInformation.LimitFlags.0
        ),
        Err(e) => format!("harness in job: yes; query failed {e}"),
    }
}

// ───────────────────────── TRD §3-4 selector ─────────────────────────

/// process_tree::levels per TRD: verify root identity, walk live children, drop children born before
/// parent, visited keyed by identity, depth limit.
fn levels(root: Ident, max_depth: usize) -> Vec<(Ident, usize)> {
    if creation(root.pid) != Some(root.start) {
        return Vec::new();
    }
    let mut out = vec![(root, 0)];
    let mut visited = vec![root];
    let mut frontier = vec![(root, 0usize)];
    while let Some((p, d)) = frontier.pop() {
        if d >= max_depth {
            continue;
        }
        for c in child_pids(p.pid) {
            let Some(st) = creation(c) else { continue };
            let ci = Ident { pid: c, start: st };
            if st < p.start || visited.contains(&ci) {
                continue;
            }
            visited.push(ci);
            frontier.push((ci, d + 1));
            out.push((ci, d + 1));
        }
    }
    out
}
struct Sel {
    members: Vec<Ident>,
    unreadable: usize,
    levels: Vec<(Ident, usize)>,
    cands: Option<Vec<Ident>>, // None = claude layer empty -> no cleanup
}
fn trd_select(job: H, root: Ident, threshold: u64) -> Sel {
    // order per TRD §3-4: ① member list → ② level walk → ③ select
    let pids = job_members(job);
    let mut members = Vec::new();
    let mut unreadable = 0;
    for pid in pids {
        match creation(pid) {
            Some(st) => members.push(Ident { pid, start: st }),
            None => unreadable += 1,
        }
    }
    let lv = levels(root, WRAPPER_DEPTH + 1);
    let keep: Vec<Ident> = lv.iter().filter(|(_, d)| *d <= WRAPPER_DEPTH + 1).map(|(i, _)| *i).collect();
    let claude_layer = lv.iter().any(|(_, d)| *d == WRAPPER_DEPTH);
    let cands = if claude_layer {
        Some(
            members
                .iter()
                .filter(|m| !keep.contains(m) && m.start >= threshold)
                .copied()
                .collect(),
        )
    } else {
        None
    };
    Sel { members, unreadable, levels: lv, cands }
}

// ───────────────────────── job membership monitor ─────────────────────────

struct Rec {
    id: Ident, // start 0 = unreadable
    ppid: u32,
    exe: String,
    cmd: String,
    in_job_at_open: Option<bool>,
    seen_t: f64,
    exit_msg_t: Option<f64>,
    abnormal: bool,
    h: Option<H>,
    src: &'static str,
}
fn gather(pid: u32, job: H, t: f64, src: &'static str) -> Rec {
    let mut r = Rec {
        id: Ident { pid, start: 0 },
        ppid: 0,
        exe: String::new(),
        cmd: String::new(),
        in_job_at_open: None,
        seen_t: t,
        exit_msg_t: None,
        abnormal: false,
        h: None,
        src,
    };
    if let Some(h) = open_q(pid) {
        r.in_job_at_open = in_job(h, job.h());
        if let Some((c, _)) = times(h) {
            r.id.start = c;
        }
        r.exe = image(h);
        r.ppid = ppid_of(h).unwrap_or(0);
        r.cmd = cmdline(h);
        r.h = Some(H::of(h));
    }
    r
}
fn push_rec(recs: &Mutex<Vec<Rec>>, rec: Rec) {
    let mut v = recs.lock().unwrap();
    if rec.id.start != 0 && v.iter().any(|x| x.id == rec.id) {
        if let Some(h) = rec.h {
            close(h);
        }
        return;
    }
    v.push(rec);
}
fn monitor(port: H, job: H, t0: Instant, recs: Arc<Mutex<Vec<Rec>>>, stop: Arc<AtomicBool>) {
    loop {
        let mut n = 0u32;
        let mut key = 0usize;
        let mut ov: *mut OVERLAPPED = std::ptr::null_mut();
        let r = unsafe { GetQueuedCompletionStatus(port.h(), &mut n, &mut key, &mut ov, 50) };
        if r.is_err() {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            continue;
        }
        let pid = ov as usize as u32;
        let t = ms_since(t0);
        match n {
            6 => push_rec(&recs, gather(pid, job, t, "port")),
            7 | 8 => {
                let mut v = recs.lock().unwrap();
                if let Some(x) = v.iter_mut().rev().find(|x| x.id.pid == pid && x.exit_msg_t.is_none()) {
                    x.exit_msg_t = Some(t);
                    x.abnormal = n == 8;
                }
            }
            _ => {}
        }
    }
}

// ───────────────────────── claude process ─────────────────────────

struct Line {
    t: f64,
    wall: u64,
    kind: String,
    sub: String,
    raw: Option<String>,
}
struct Claude {
    child: Child,
    stdin: Option<ChildStdin>,
    job: H,
    port: H,
    root: Ident,
    t0: Instant,
    start_wall: u64,
    lines: Arc<Mutex<Vec<Line>>>,
    ring: Arc<Mutex<VecDeque<String>>>,
    recs: Arc<Mutex<Vec<Rec>>>,
    stop: Arc<AtomicBool>,
    mon: Option<JoinHandle<()>>,
    raw: Arc<Mutex<File>>,
    det: File,
    pi: usize,
    out_dir: String,
}
impl Claude {
    fn now(&self) -> f64 {
        ms_since(self.t0)
    }
    fn nlines(&self) -> usize {
        self.lines.lock().unwrap().len()
    }
    fn find(&self, from: usize, kind: &str, sub: Option<&str>) -> Option<(f64, Option<String>)> {
        let v = self.lines.lock().unwrap();
        v.iter()
            .skip(from)
            .find(|l| l.kind == kind && sub.map_or(true, |s| l.sub == s))
            .map(|l| (l.t, l.raw.clone()))
    }
    fn wait(&self, from: usize, kind: &str, sub: Option<&str>, deadline: f64) -> Option<(f64, Option<String>)> {
        loop {
            if let Some(x) = self.find(from, kind, sub) {
                return Some(x);
            }
            if self.now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }
    fn send(&mut self, s: &str, label: &str) {
        let t = self.now();
        let w0 = wall_ft();
        if let Some(w) = self.stdin.as_mut() {
            let _ = w.write_all(s.as_bytes());
            let _ = w.write_all(b"\n");
            let _ = w.flush();
        }
        let _ = writeln!(self.raw.lock().unwrap(), "{t:.1} w{w0} IN[{label}] {}", trunc(s, 400));
        self.ring.lock().unwrap().push_back(format!("{t:.1} IN[{label}] {}", trunc(s, 300)));
    }
    fn mark(&mut self, s: &str) {
        let t = self.now();
        let w0 = wall_ft();
        let _ = writeln!(self.raw.lock().unwrap(), "{t:.1} w{w0} MARK {s}");
    }
    fn d(&mut self, s: String) {
        let _ = writeln!(self.det, "{s}");
    }
    fn turn(&mut self, text: &str, label: &str, timeout: f64) -> String {
        let from = self.nlines();
        let t = self.now();
        self.send(&user_line(text), label);
        match self.wait(from, "result", None, t + timeout) {
            Some((tr, raw)) => format!(
                "{}:{:.0}ms",
                raw.as_deref().and_then(|r| top_field(r, "subtype")).unwrap_or_default(),
                tr - t
            ),
            None => format!("TIMEOUT>{timeout:.0}ms"),
        }
    }
}
fn user_line(text: &str) -> String {
    format!(
        "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}]}},\"uuid\":\"{}\"}}",
        uuid()
    )
}
fn interrupt_line() -> String {
    format!(
        "{{\"type\":\"control_request\",\"request_id\":\"interrupt:{}\",\"request\":{{\"subtype\":\"interrupt\"}}}}",
        uuid()
    )
}

struct Cfg {
    out: String,
    cwd: String,
    procs: usize,
    per: usize,
    delay: f64,
    q4: bool,
    model: String,
    settings: Option<String>,
    max_trials: usize,
    stop_after_hangs: usize,
    b_target: usize,
    fu_timeout: f64,
}

fn spawn_claude(cfg: &Cfg, pi: usize) -> (Claude, String) {
    let sid = uuid();
    let mut args: Vec<String> = vec!["/c".into(), "claude".into()];
    for a in [
        "--permission-mode",
        "bypassPermissions",
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--replay-user-messages",
        "--verbose",
        "--include-partial-messages",
        "--session-id",
    ] {
        args.push(a.into());
    }
    args.push(sid.clone());
    if cfg.model != "default" {
        args.push("--model".into());
        args.push(cfg.model.clone());
    }
    args.push("--include-hook-events".into());
    if let Some(s) = &cfg.settings {
        args.push("--settings".into());
        args.push(s.clone());
    }
    let mut cmd = Command::new("cmd.exe");
    cmd.args(&args);
    cmd.current_dir(&cfg.cwd);
    let mut claude_vars = Vec::new();
    for (k, _) in std::env::vars() {
        let ku = k.to_uppercase();
        if ku.starts_with("CLAUDE") {
            claude_vars.push(k.clone());
        }
        // strip only session markers of a parent claude session (should be absent under WMI launch)
        if ku == "CLAUDECODE" || ku == "CLAUDE_CODE_ENTRYPOINT" || ku == "CLAUDE_CODE_SSE_PORT" {
            cmd.env_remove(&k);
        }
    }
    cmd.env("MAX_THINKING_TOKENS", "8000"); // as backend/claude injects for StreamJson
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    let pre_wall = wall_ft();
    let t0 = Instant::now();
    let mut child = cmd.spawn().expect("spawn");
    let t_spawned = t0.elapsed();
    let pid = child.id();
    let stdin = child.stdin.take();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    // mirror stdio.rs:120-127
    let job = job_new().expect("job_new");
    let assign_res = job_assign(job, pid);
    let t_assigned = t0.elapsed();
    let port = assoc_port(job).expect("assoc_port");
    let root_start = creation(pid).unwrap_or(0);
    let root = Ident { pid, start: root_start };

    let raw = Arc::new(Mutex::new(
        OpenOptions::new().create(true).append(true).open(format!("{}/p{pi}.raw.log", cfg.out)).unwrap(),
    ));
    let det = OpenOptions::new().create(true).append(true).open(format!("{}/p{pi}.detail.log", cfg.out)).unwrap();
    let lines = Arc::new(Mutex::new(Vec::<Line>::new()));
    let ring = Arc::new(Mutex::new(VecDeque::<String>::new()));
    let recs = Arc::new(Mutex::new(Vec::<Rec>::new()));
    let stop = Arc::new(AtomicBool::new(false));
    {
        let (r, l, rg) = (raw.clone(), lines.clone(), ring.clone());
        thread::spawn(move || {
            let mut rd = BufReader::new(stdout);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match rd.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                let s = String::from_utf8_lossy(&buf).trim_end().to_string();
                if s.is_empty() {
                    continue;
                }
                let t = ms_since(t0);
                let w = wall_ft();
                let kind = top_field(&s, "type").unwrap_or_default();
                let sub = top_field(&s, "subtype").unwrap_or_default();
                let keep = kind != "stream_event";
                if keep {
                    let _ = writeln!(r.lock().unwrap(), "{t:.1} w{w} OUT {}", trunc(&s, 12000));
                }
                {
                    let mut q = rg.lock().unwrap();
                    q.push_back(format!("{t:.1} OUT {}", trunc(&s, 400)));
                    while q.len() > 80 {
                        q.pop_front();
                    }
                }
                let rawv = if kind == "result" || (kind == "system" && sub == "init") { Some(s.clone()) } else if keep { Some(trunc(&s, 12000)) } else { None };
                l.lock().unwrap().push(Line { t, wall: w, kind, sub, raw: rawv });
            }
            let _ = writeln!(r.lock().unwrap(), "{:.1} w{} STDOUT-EOF", ms_since(t0), wall_ft());
        });
    }
    {
        let (r, rg) = (raw.clone(), ring.clone());
        thread::spawn(move || {
            let mut e = stderr;
            let mut buf = [0u8; 4096];
            loop {
                match e.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let s = String::from_utf8_lossy(&buf[..n]).to_string();
                        let t = ms_since(t0);
                        let _ = writeln!(r.lock().unwrap(), "{t:.1} w{} ERR {}", wall_ft(), trunc(&s, 2000));
                        rg.lock().unwrap().push_back(format!("{t:.1} ERR {}", trunc(&s, 300)));
                    }
                }
            }
        });
    }
    let mon = {
        let (rc, st) = (recs.clone(), stop.clone());
        thread::spawn(move || monitor(port, job, t0, rc, st))
    };
    // members already present at association (root, maybe more)
    for p in job_members(job) {
        push_rec(&recs, gather(p, job, ms_since(t0), "pre"));
    }
    let mut info = format!(
        "spawn: argv=cmd.exe {} | spawn() {}us, assign done {}us after spawn start (gap spawn-return->assign {}us), assign={:?} | root cmd.exe start - pre_spawn_wall = {:.3}ms | CLAUDE* env names present: {:?}",
        args.join(" ").replace(&sid, "<sid>"),
        t_spawned.as_micros(),
        t_assigned.as_micros(),
        (t_assigned - t_spawned).as_micros(),
        assign_res,
        dms(pre_wall, root_start),
        claude_vars
    );
    let c = Claude {
        child,
        stdin,
        job,
        port,
        root,
        t0,
        start_wall: pre_wall,
        lines,
        ring,
        recs,
        stop,
        mon: Some(mon),
        raw,
        det,
        pi,
        out_dir: cfg.out.clone(),
    };
    info.push_str(&format!(" | pid-root={}", pid));
    (c, info)
}

/// Full tree under root (walk rule, no depth limit) with job membership — report only.
fn tree_report(c: &Claude, base_wall: u64) -> Vec<String> {
    let mut out = Vec::new();
    for (id, d) in levels(c.root, 16) {
        let mut exe = String::new();
        let mut cmd = String::new();
        let mut ij = None;
        let mut pp = 0;
        if let Some(h) = open_q(id.pid) {
            exe = image(h);
            cmd = cmdline(h);
            ij = in_job(h, c.job.h());
            pp = ppid_of(h).unwrap_or(0);
            close(H::of(h));
        }
        out.push(format!(
            "d{d} {}:{} ppid={pp} created={:+.1}ms inOurJob={:?} path={} cmd={}",
            base(&exe),
            id.pid,
            dms(base_wall, id.start),
            ij,
            exe,
            trunc(&cmd, 220)
        ));
    }
    out
}

fn hooky(s: &str) -> bool {
    let l = s.to_lowercase();
    if l.contains("shell-snapshots") { return false; }
    ["handoff-trigger", "wiki-preconsult", "plugin-notice", ".orca", "claude-hook", "t40-spike", "nmfc-origin", "hooks\\", "hooks/", "t40bg=1", "t40fg=1"]
        .iter()
        .any(|p| l.contains(p))
}
/// short provenance tag from a command line (report only)
fn tag_of(cmd: &str) -> String {
    let l = cmd.to_lowercase();
    for (p, t) in [
        ("t40bg=1", "bg"),
        ("t40fg=1", "fg"),
        ("shell-snapshots", "tool"),
        ("handoff-trigger", "handoff-trigger"),
        ("wiki-preconsult", "wiki-preconsult"),
        ("plugin-notice", "plugin-notice"),
        ("plugin-autoupdate", "plugin-autoupdate"),
        ("pre-build-check", "pre-build-check"),
        ("claude-hook", "orca-hook"),
        ("session-lifecycle-hook", "codex-hook"),
        ("stop-review-gate", "codex-hook"),
        ("python -c", "py-hook"),
    ] {
        if l.contains(p) {
            return t.into();
        }
    }
    String::new()
}
fn parent_rec<'a>(recs: &'a [Rec], r: &Rec) -> Option<&'a Rec> {
    recs.iter()
        .filter(|x| x.id.pid == r.ppid && x.id.start != 0 && x.id.start <= r.id.start && x.id != r.id)
        .max_by_key(|x| x.id.start)
}
/// id, parent, grandparent, … (job records only; handles held → no pid reuse)
fn chain(recs: &[Rec], id: Ident) -> Vec<Ident> {
    let mut out = vec![id];
    let mut cur = id;
    for _ in 0..40 {
        let Some(r) = recs.iter().find(|x| x.id == cur) else { break };
        match parent_rec(recs, r) {
            Some(p) => {
                out.push(p.id);
                cur = p.id;
            }
            None => break,
        }
    }
    out
}
fn claude_ident(recs: &[Rec], root: Ident) -> Option<Ident> {
    recs.iter()
        .filter(|r| r.ppid == root.pid && r.id.start >= root.start && base(&r.exe).eq_ignore_ascii_case("claude.exe"))
        .map(|r| r.id)
        .next()
}
/// the direct child of claude.exe that this process descends from (None = not under claude / is claude)
fn launcher_of(recs: &[Rec], id: Ident, claude: Ident) -> Option<Ident> {
    let ch = chain(recs, id);
    let pos = ch.iter().position(|x| *x == claude)?;
    if pos == 0 {
        return None;
    }
    Some(ch[pos - 1])
}
/// launcher shape: "tool" if the claude-direct-child cmdline sources a shell snapshot, else its tag / exe name.
fn launcher_shape(recs: &[Rec], l: Ident) -> String {
    let Some(r) = recs.iter().find(|x| x.id == l) else { return "?".into() };
    let t = tag_of(&r.cmd);
    let b = base(&r.exe);
    if t.is_empty() {
        b
    } else {
        format!("{b}[{t}]")
    }
}
fn chain_str(recs: &[Rec], id: Ident) -> String {
    chain(recs, id)
        .iter()
        .map(|i| {
            let alive = recs.iter().find(|r| r.id == *i).and_then(|r| r.h).map_or(false, |h| is_alive(h.h()));
            format!("{}:{}{}", rec_name(recs, *i), i.pid, if alive { "" } else { "(dead)" })
        })
        .collect::<Vec<_>>()
        .join("<-")
}
fn is_hook_launcher(recs: &[Rec], l: Ident) -> bool {
    let Some(r) = recs.iter().find(|x| x.id == l) else { return false };
    base(&r.exe).eq_ignore_ascii_case("bash.exe") && !r.cmd.to_lowercase().contains("shell-snapshots")
}
/// Report-only: processes created at/after `since` that are NOT in our job.
fn outside_scan(job: H, since: u64) -> (Vec<String>, Vec<String>) {
    let mut all = Vec::new();
    let mut hook = Vec::new();
    for r in snapshot_all() {
        let Some(h) = open_q(r.pid) else { continue };
        if let Some((cr, _)) = times(h) {
            if cr >= since && in_job(h, job.h()) == Some(false) {
                let cmd = cmdline(h);
                let alive = is_alive(h);
                let row = format!(
                    "{}:{} ppid={} created={:+.1}ms alive={} cmd={}",
                    r.exe,
                    r.pid,
                    r.ppid,
                    dms(since, cr),
                    alive,
                    trunc(&cmd, 200)
                );
                let lx = r.exe.to_lowercase();
                if hooky(&cmd) || lx == "bash.exe" || lx == "sh.exe" {
                    hook.push(row.clone());
                }
                all.push(row);
            }
        }
        close(H::of(h));
    }
    (all, hook)
}

fn rec_name(recs: &[Rec], id: Ident) -> String {
    recs.iter()
        .find(|r| r.id == id)
        .map(|r| {
            let b = base(&r.exe);
            let t = tag_of(&r.cmd);
            let hint = if t.is_empty() { String::new() } else { format!("({t})") };
            format!("{b}{hint}")
        })
        .unwrap_or_else(|| "?".into())
}
/// depth below root and immediate parent, from job-membership records (handles held → no pid reuse).
fn depth_parent(recs: &[Rec], id: Ident, root: Ident) -> (Option<usize>, String) {
    let mut d = 0usize;
    let mut cur = id;
    let mut parent = String::new();
    loop {
        if cur == root {
            return (Some(d), parent);
        }
        let Some(r) = recs.iter().find(|x| x.id == cur) else { return (None, parent) };
        let p = recs
            .iter()
            .filter(|x| x.id.pid == r.ppid && x.id.start != 0 && x.id.start <= r.id.start)
            .max_by_key(|x| x.id.start);
        match p {
            None => {
                if d == 0 {
                    parent = format!("?:{} (not a job member)", r.ppid);
                }
                return (None, parent);
            }
            Some(p) => {
                if d == 0 {
                    parent = format!("{}:{}", rec_name(recs, p.id), p.id.pid);
                }
                cur = p.id;
                d += 1;
                if d > 40 {
                    return (None, parent);
                }
            }
        }
    }
}

struct TrialRec {
    label: String,
    esc_wall: u64,
    end_wall: u64,
    alive3: Vec<Ident>,
    cand3: Vec<Ident>,
    killed: Vec<Ident>,
    outcome: String,
}

fn capture(c: &mut Claude, esc_wall: u64, label: &str) -> (Sel, Vec<Ident>, String) {
    let t = dms(esc_wall, wall_ft());
    let sel = trd_select(c.job, c.root, esc_wall);
    // post-Esc job records alive now (held handles)
    let (alive_post, member_lines) = {
        let recs = c.recs.lock().unwrap();
        let mut alive = Vec::new();
        for r in recs.iter() {
            if r.id.start >= esc_wall {
                if let Some(h) = r.h {
                    if is_alive(h.h()) {
                        alive.push(r.id);
                    }
                }
            }
        }
        let mut ml = Vec::new();
        for m in &sel.members {
            let (d, p) = depth_parent(&recs, *m, c.root);
            let cmd = recs.iter().find(|r| r.id == *m).map(|r| trunc(&r.cmd, 300)).unwrap_or_default();
            ml.push(format!(
                "    member {}:{} depth={:?} parent={} created_vs_esc={:+.1}ms cand={} chain={} cmd={}",
                rec_name(&recs, *m),
                m.pid,
                d,
                p,
                dms(esc_wall, m.start),
                sel.cands.as_ref().map_or(false, |cs| cs.contains(m)),
                chain_str(&recs, *m),
                cmd
            ));
        }
        (alive, ml)
    };
    let (outside_all, outside_hook) = outside_scan(c.job, esc_wall);
    let recs = c.recs.lock().unwrap();
    let lv: Vec<String> = sel.levels.iter().map(|(i, d)| format!("d{d}:{}:{}", rec_name(&recs, *i), i.pid)).collect();
    let cl = claude_ident(&recs, c.root);
    let cands: Vec<String> = match &sel.cands {
        None => vec!["<claude layer empty: no cleanup>".into()],
        Some(cs) => cs
            .iter()
            .map(|i| {
                let ls = cl
                    .and_then(|cl| launcher_of(&recs, *i, cl))
                    .map(|l| launcher_shape(&recs, l))
                    .unwrap_or_else(|| "?".into());
                format!("{}:{}{:+.0}ms{{L={ls}}}", rec_name(&recs, *i), i.pid, dms(esc_wall, i.start))
            })
            .collect(),
    };
    drop(recs);
    let mut s = format!(
        "  [{label} @esc{:+.0}ms] members={} unreadable={} levels(0..=2)=[{}] TRD-cands=[{}] outsideJobSinceEsc: total={} hooklike={:?}",
        t,
        sel.members.len(),
        sel.unreadable,
        lv.join(", "),
        cands.join(", "),
        outside_all.len(),
        outside_hook
    );
    let det = format!(
        "{s}\n{}\n    outside-all: {}",
        member_lines.join("\n"),
        outside_all.join("\n      ")
    );
    c.d(det);
    (sel, alive_post, s)
}

fn run_trial(c: &mut Claude, cfg: &Cfg, label: &str, bg: bool) -> (TrialRec, bool, bool) {
    // returns (rec, hang, keep_process)
    sleep_ms(1500.0);
    c.mark(&format!("TRIAL-START {label} bg={bg}"));
    let from = c.nlines();
    let t_send = c.now();
    c.send(&user_line("say hi"), "test");
    sleep_ms(t_send + cfg.delay - c.now());
    let esc_wall = wall_ft();
    let esc_t = c.now();
    c.send(&interrupt_line(), "interrupt");
    let ctrl = c.wait(from, "control_response", None, esc_t + 3000.0).map(|x| x.0 - esc_t);
    let mut res = c.wait(from, "result", None, esc_t + 3000.0);
    sleep_ms(esc_t + 3000.0 - c.now());
    let (sel3, alive3, s3) = capture(c, esc_wall, "S3");
    let cand3 = sel3.cands.clone().unwrap_or_default();
    let mut hang = false;
    let mut late = false;
    let mut keep = true;
    let mut killed = Vec::new();
    let mut extra = String::new();
    if res.is_none() {
        res = c.wait(from, "result", None, esc_t + 10000.0);
        if res.is_some() {
            late = true;
        } else {
            hang = true;
            let (sel10, _a, s10) = capture(c, esc_wall, "S10");
            extra.push_str(&format!("\n{s10}"));
            let same = sel10.cands == sel3.cands;
            extra.push_str(&format!("\n  cands@3s == cands@10s: {same}"));
            if cfg.q4 {
                let mut kills = Vec::new();
                let tk = c.now();
                if let Some(cs) = &sel10.cands {
                    for id in cs {
                        let (nm, hookish) = {
                            let recs = c.recs.lock().unwrap();
                            let hk = claude_ident(&recs, c.root)
                                .and_then(|cl| launcher_of(&recs, *id, cl))
                                .map_or(false, |l| is_hook_launcher(&recs, l));
                            (rec_name(&recs, *id), hk)
                        };
                        if bg && !hookish {
                            // bg trial: only hook-chain candidates are killed
                            kills.push(format!("{nm}:{}=SKIPPED(not hook chain)", id.pid));
                            continue;
                        }
                        let r = terminate_member(c.job, id.pid, id.start);
                        if r == "Terminated" {
                            killed.push(*id);
                        }
                        kills.push(format!("{nm}:{}={r}", id.pid));
                    }
                }
                let tk_done = c.now();
                let r2 = c.wait(from, "result", None, tk + 15000.0);
                match r2 {
                    Some((tr, raw)) => {
                        extra.push_str(&format!(
                            "\n  Q4: kills=[{}] killStart->result={:.0}ms killDone->result={:.0}ms result.subtype={}",
                            kills.join(", "),
                            tr - tk,
                            tr - tk_done,
                            raw.as_deref().and_then(|r| top_field(r, "subtype")).unwrap_or_default()
                        ));
                        res = Some((tr, raw));
                        let alive = c.child.try_wait().ok().flatten().is_none();
                        extra.push_str(&format!(" claudeAlive={alive}"));
                    }
                    None => {
                        extra.push_str(&format!(
                            "\n  Q4: kills=[{}] NO result within 15s after kill -> unresolved (process will be ended)",
                            kills.join(", ")
                        ));
                        keep = false;
                    }
                }
            } else {
                keep = false;
            }
        }
    }
    let end_wall = wall_ft();
    let hooks: Vec<String> = {
        let v = c.lines.lock().unwrap();
        v.iter()
            .skip(from)
            .filter(|l| l.kind == "system" && l.sub.starts_with("hook_response"))
            .map(|l| {
                let r = l.raw.as_deref().unwrap_or("");
                format!(
                    "{}:{}@{:+.0}",
                    field(r, "hook_name").unwrap_or_default(),
                    field(r, "outcome").unwrap_or_default(),
                    l.t - esc_t
                )
            })
            .collect()
    };
    let res_s = match &res {
        Some((tr, raw)) => format!(
            "result@esc+{:.0}ms({})",
            tr - esc_t,
            raw.as_deref().and_then(|r| top_field(r, "subtype")).unwrap_or_default()
        ),
        None => "no-result".into(),
    };
    let kind = if hang { "HANG" } else if late { "LATE" } else { "ok" };
    let mut outcome = format!(
        "TRIAL {label} {kind} | esc@send+{:.0}ms ctrl+{} {res_s} | hookResponses={:?}\n{s3}{extra}",
        esc_t - t_send,
        ctrl.map_or("none".into(), |x| format!("{x:.0}ms")),
        hooks
    );
    if hang && keep && cfg.q4 {
        sleep_ms(500.0);
        let fu_from = c.nlines();
        c.mark(&format!("FOLLOWUP-START {label}"));
        let fu = c.turn("say ok", "followup", cfg.fu_timeout);
        outcome.push_str(&format!("\n  followUp(same claude)={fu}"));
        if fu.starts_with("TIMEOUT") {
            let st = capture_stall(c, fu_from, esc_wall, label);
            outcome.push_str(&format!("\n{st}"));
            keep = false;
        }
    }
    (
        TrialRec { label: label.into(), esc_wall, end_wall, alive3, cand3, killed, outcome },
        hang,
        keep,
    )
}

fn capture_stall(c: &mut Claude, from: usize, esc_wall: u64, label: &str) -> String {
    let mut out = vec![format!("  STALL {label}: follow-up got no result")];
    let pids = job_members(c.job);
    {
        let recs = c.recs.lock().unwrap();
        out.push(format!("  stall members={} pids={:?}", pids.len(), pids));
        for pid in &pids {
            let st = creation(*pid).unwrap_or(0);
            let id = Ident { pid: *pid, start: st };
            let cmd = recs
                .iter()
                .find(|r| r.id == id)
                .map(|r| trunc(&r.cmd, 400))
                .unwrap_or_else(|| "<no job record>".into());
            out.push(format!(
                "    member {}:{} created_vs_esc={:+.1}ms chain={} cmd={}",
                rec_name(&recs, id),
                pid,
                dms(esc_wall, st),
                chain_str(&recs, id),
                cmd
            ));
        }
        let alive: Vec<String> = recs
            .iter()
            .filter(|r| r.h.map_or(false, |h| is_alive(h.h())))
            .map(|r| format!("{}:{}", rec_name(&recs, r.id), r.id.pid))
            .collect();
        out.push(format!("  stall alive job records: {alive:?}"));
    }
    {
        let v = c.lines.lock().unwrap();
        let mut started: Vec<(String, String, f64)> = Vec::new();
        let mut responded: Vec<String> = Vec::new();
        let mut life = Vec::new();
        for l in v.iter().skip(from) {
            let r = l.raw.as_deref().unwrap_or("");
            if l.kind == "system" && l.sub == "hook_started" {
                started.push((field(r, "hook_id").unwrap_or_default(), field(r, "hook_name").unwrap_or_default(), l.t));
            } else if l.kind == "system" && l.sub == "hook_response" {
                responded.push(field(r, "hook_id").unwrap_or_default());
            } else if l.kind == "command_lifecycle" {
                life.push(format!("{}@{:.0}", field(r, "state").unwrap_or_default(), l.t));
            }
        }
        let pending: Vec<String> = started
            .iter()
            .filter(|s| !responded.contains(&s.0))
            .map(|s| format!("{}:{}@{:.0}", s.1, s.0, s.2))
            .collect();
        out.push(format!(
            "  stall hooks since follow-up: started={} responded={} pending={:?} lifecycle={:?}",
            started.len(),
            responded.len(),
            pending,
            life
        ));
    }
    {
        let q = c.ring.lock().unwrap();
        out.push("  stall last stream lines (oldest first):".into());
        let n = q.len();
        for l in q.iter().skip(n.saturating_sub(40)) {
            out.push(format!("    | {l}"));
        }
    }
    let from2 = c.nlines();
    let ti = c.now();
    c.send(&interrupt_line(), "stall-interrupt");
    let ctrl = c.wait(from2, "control_response", None, ti + 5000.0).map(|x| x.0 - ti);
    let res = c
        .wait(from2, "result", None, ti + 15000.0)
        .map(|x| (x.0 - ti, x.1.as_deref().and_then(|r| top_field(r, "subtype")).unwrap_or_default()));
    out.push(format!("  stall interrupt: control_response_ms={ctrl:?} result={res:?}"));
    let s = out.join("\n");
    c.d(s.clone());
    s
}

fn b_turn(c: &mut Claude, text: &str, label: &str, timeout: f64) -> (String, usize, u64, u64) {
    let from = c.nlines();
    c.mark(&format!("B-START {label}"));
    let send_wall = wall_ft();
    let t = c.now();
    c.send(&user_line(text), label);
    let r = c.wait(from, "result", None, t + timeout);
    let s = match r {
        Some((tr, raw)) => format!(
            "{}:{:.0}ms",
            raw.as_deref().and_then(|x| top_field(x, "subtype")).unwrap_or_default(),
            tr - t
        ),
        None => {
            let f2 = c.nlines();
            let ti = c.now();
            c.send(&interrupt_line(), &format!("{label}-timeout-interrupt"));
            let r2 = c.wait(f2, "result", None, ti + 15000.0);
            format!("TIMEOUT>{timeout:.0}ms (interrupt -> result after {:?}ms)", r2.map(|x| x.0 - ti))
        }
    };
    let end_wall = wall_ft();
    c.mark(&format!("B-END {label} {s}"));
    (s, from, send_wall, end_wall)
}

fn snippet(r: &str, key: &str, n: usize) -> String {
    match r.find(key) {
        Some(i) => trunc(&r[i..], n),
        None => String::new(),
    }
}

/// stream events since `from`, times relative to base_wall (ms)
fn b_events(c: &Claude, from: usize, base_wall: u64) -> Vec<String> {
    let v = c.lines.lock().unwrap();
    let mut out = Vec::new();
    for l in v.iter().skip(from) {
        let Some(r) = l.raw.as_deref() else { continue };
        let rel = dms(base_wall, l.wall);
        let s = match (l.kind.as_str(), l.sub.as_str()) {
            ("system", "hook_started") => format!(
                "hook_started name={} event={} id={}",
                field(r, "hook_name").unwrap_or_default(),
                field(r, "hook_event").unwrap_or_default(),
                field(r, "hook_id").unwrap_or_default()
            ),
            ("system", "hook_response") => format!(
                "hook_response name={} id={} outcome={} exit={} stdout={}",
                field(r, "hook_name").unwrap_or_default(),
                field(r, "hook_id").unwrap_or_default(),
                field(r, "outcome").unwrap_or_default(),
                snippet(r, "\"exit_code\":", 16),
                trunc(&field(r, "stdout").unwrap_or_default(), 80)
            ),
            ("assistant", _) if r.contains("\"tool_use\"") => format!("assistant {}", snippet(r, "\"type\":\"tool_use\"", 500)),
            ("user", _) if r.contains("tool_result") => format!("user {}", snippet(r, "\"tool_use_id\"", 500)),
            ("user", _) => format!("user(replay?) {}", trunc(r, 160)),
            ("assistant", _) => format!("assistant {}", snippet(r, "\"content\":", 200)),
            ("system", sub) if sub != "status" && sub != "thinking_tokens" && sub != "init" => format!("system/{sub} {}", trunc(r, 400)),
            ("command_lifecycle", _) => format!("command_lifecycle {}", field(r, "state").unwrap_or_default()),
            ("result", _) => format!("result {}", top_field(r, "subtype").unwrap_or_default()),
            (k, "") if k != "system" && k != "rate_limit_event" => format!("{k} {}", trunc(r, 200)),
            _ => continue,
        };
        out.push(format!("    ev {rel:+.1}ms {s}"));
    }
    out
}

/// job records created in [w0, w1], times relative to w0
fn b_procs(c: &Claude, w0: u64, w1: u64) -> Vec<String> {
    let recs = c.recs.lock().unwrap();
    let cl = claude_ident(&recs, c.root);
    let mut out = Vec::new();
    for r in recs.iter().filter(|r| r.id.start >= w0 && r.id.start <= w1) {
        let (d, _) = depth_parent(&recs, r.id, c.root);
        let l = cl.and_then(|cl| launcher_of(&recs, r.id, cl));
        let ls = l.map(|l| format!("{}:{}", launcher_shape(&recs, l), l.pid)).unwrap_or_else(|| "-".into());
        let life = match r.h.and_then(|h| times(h.h())) {
            Some((cr, ex)) if ex != 0 => format!("{:.0}ms", dms(cr, ex)),
            _ => "alive".into(),
        };
        let direct = cl.map_or(false, |cl| r.ppid == cl.pid && Some(r.id) == l);
        out.push(format!(
            "    proc {:+.1}ms d{} {}:{} ppid={} L={} life={}{}",
            dms(w0, r.id.start),
            d.map_or("?".into(), |x| x.to_string()),
            rec_name(&recs, r.id),
            r.id.pid,
            r.ppid,
            ls,
            life,
            if direct { format!(" CLAUDE-CHILD exe={} cmd={}", r.exe, trunc(&r.cmd, 700)) } else { format!(" cmd={}", trunc(&r.cmd, 200)) }
        ));
    }
    out
}

fn run_b(c: &mut Claude) -> (String, bool) {
    sleep_ms(1500.0);
    let mut det = Vec::new();
    let mut summ = Vec::new();
    let fg_text = "Use the Bash tool to run exactly this command, unmodified: T40FG=1; sleep 5; echo done   Then reply with its output only.";
    let (s, from, w0, w1) = b_turn(c, fg_text, "B-fg", 120000.0);
    summ.push(format!("  B-fg: {s}"));
    det.push(format!("  B-fg: {s} (times relative to send)"));
    det.extend(b_events(c, from, w0));
    det.extend(b_procs(c, w0, w1));
    sleep_ms(1500.0);
    let bg_text = "Use the Bash tool with run_in_background set to true to run exactly this command, unmodified: T40BG=1; while true; do sleep 2; done   Do not wait for it and do not check on it. After starting it, reply with the single word: started";
    let (s, from, w0, _w1) = b_turn(c, bg_text, "B-bg", 150000.0);
    summ.push(format!("  B-bg: {s}"));
    sleep_ms(7000.0);
    c.mark("B-bg observation window end");
    let w2 = wall_ft();
    det.push(format!("  B-bg: {s} (times relative to send; window = send .. result+7s)"));
    det.extend(b_events(c, from, w0));
    det.extend(b_procs(c, w0, w2));
    // alive members now, with chain
    let mut bg_alive = false;
    {
        let recs = c.recs.lock().unwrap();
        let pids = job_members(c.job);
        det.push(format!("  B members after bg window: {}", pids.len()));
        for pid in pids {
            let st = creation(pid).unwrap_or(0);
            let id = Ident { pid, start: st };
            let cmd = recs.iter().find(|r| r.id == id).map(|r| r.cmd.clone()).unwrap_or_default();
            if cmd.to_lowercase().contains("t40bg=1") {
                bg_alive = true;
            }
            det.push(format!("    member {} chain={} cmd={}", pid, chain_str(&recs, id), trunc(&cmd, 400)));
        }
    }
    summ.push(format!("  B bg loop alive in job after window: {bg_alive}"));
    let d = det.join("\n");
    c.d(d.clone());
    // also dump B detail to its own file
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(format!("{}/p{}.B.log", c.out_dir, c.pi)) {
        let _ = writeln!(f, "{d}");
    }
    (summ.join("\n"), bg_alive)
}

fn esc_tsv(s: &str) -> String {
    // backslashes are left as-is (paths); only field/record separators are made visible
    s.replace('\t', "<TAB>").replace('\n', "<LF>").replace('\r', "<CR>")
}

fn finish(mut c: Claude, trials: &[TrialRec], out: &mut File) {
    drop(c.stdin.take());
    for _ in 0..50 {
        if c.child.try_wait().ok().flatten().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let exited = c.child.try_wait().ok().flatten();
    let cleanup_wall = wall_ft();
    let tj = unsafe { TerminateJobObject(c.job.h(), 1) }; // cleanup = our job only
    thread::sleep(Duration::from_millis(1500));
    let mut lines = vec![format!(
        "  cleanup: claude exit after stdin EOF = {:?}; TerminateJobObject(our job) = {:?}",
        exited.map(|s| s.code()),
        tj.is_ok()
    )];
    // Q3 tables
    let recs = c.recs.lock().unwrap();
    let survivors: Vec<String> = recs
        .iter()
        .filter(|r| r.h.map_or(false, |h| is_alive(h.h())))
        .map(|r| format!("{}:{}", rec_name(&recs, r.id), r.id.pid))
        .collect();
    lines.push(format!("  job records alive after TerminateJobObject: {survivors:?}"));
    let unopened: Vec<String> = recs.iter().filter(|r| r.h.is_none()).map(|r| format!("pid{}@{:.0}", r.id.pid, r.seen_t)).collect();
    let reused: Vec<String> = recs.iter().filter(|r| r.in_job_at_open == Some(false)).map(|r| format!("{}:{}", base(&r.exe), r.id.pid)).collect();
    lines.push(format!(
        "  job records total={} unopened(exited before open)={:?} opened-but-not-in-job(pid reuse)={:?}",
        recs.len(),
        unopened,
        reused
    ));
    let mut compact = Vec::new();
    for tr in trials {
        lines.push(format!("  Q3 {} — processes that joined our job with creation >= Esc (until trial end):", tr.label));
        let mut n = 0;
        let mut groups: Vec<(String, usize, f64)> = Vec::new(); // (name@depth, count, max life ms)
        let mut notable = Vec::new();
        for r in recs.iter().filter(|r| r.id.start >= tr.esc_wall && r.id.start < tr.end_wall) {
            n += 1;
            let (d, p) = depth_parent(&recs, r.id, c.root);
            let (life, how) = match r.h.and_then(|h| times(h.h())) {
                Some((cr, ex)) if ex != 0 => (
                    format!("{:.0}ms", dms(cr, ex)),
                    if ex >= cleanup_wall { "reaped-by-cleanup" } else { "exited" },
                ),
                _ => ("?".into(), if r.h.is_none() { "unopened" } else { "alive?" }),
            };
            let life_ms: f64 = life.trim_end_matches("ms").parse().unwrap_or(-1.0);
            let key = format!("{}@d{}", rec_name(&recs, r.id), d.map_or("?".into(), |x| x.to_string()));
            match groups.iter_mut().find(|g| g.0 == key) {
                Some(g) => {
                    g.1 += 1;
                    g.2 = g.2.max(life_ms);
                }
                None => groups.push((key.clone(), 1, life_ms)),
            }
            if tr.alive3.contains(&r.id) || tr.cand3.contains(&r.id) || life_ms >= 1000.0 || life_ms < 0.0 {
                notable.push(format!(
                    "{key}:{} parent={} created=esc{:+.0}ms life={life}({how}) alive@3s={} cand@3s={} killedByQ4={}",
                    r.id.pid,
                    p,
                    dms(tr.esc_wall, r.id.start),
                    tr.alive3.contains(&r.id),
                    tr.cand3.contains(&r.id),
                    tr.killed.contains(&r.id)
                ));
            }
        }
        compact.push(format!(
            "Q3c {} n={} groups=[{}] notable=[{}]",
            tr.label,
            n,
            groups.iter().map(|g| format!("{} x{} max{:.0}ms", g.0, g.1, g.2)).collect::<Vec<_>>().join("; "),
            notable.join(" | ")
        ));
        for r in recs.iter().filter(|r| r.id.start >= tr.esc_wall && r.id.start < tr.end_wall) {
            let (d, p) = depth_parent(&recs, r.id, c.root);
            let (life, how) = match r.h.and_then(|h| times(h.h())) {
                Some((cr, ex)) if ex != 0 => (
                    format!("{:.0}ms", dms(cr, ex)),
                    if ex >= cleanup_wall { "reaped-by-cleanup" } else { "exited" },
                ),
                _ => ("?".into(), if r.h.is_none() { "unopened" } else { "alive?" }),
            };
            lines.push(format!(
                "    {}:{} depth={:?} parent={} created=esc{:+.1}ms life={} ({}) alive@3s={} cand@3s={} killedByQ4={} exitMsg={:?}{} cmd={}",
                rec_name(&recs, r.id),
                r.id.pid,
                d,
                p,
                dms(tr.esc_wall, r.id.start),
                life,
                how,
                tr.alive3.contains(&r.id),
                tr.cand3.contains(&r.id),
                tr.killed.contains(&r.id),
                r.exit_msg_t.map(|t| format!("{t:.0}")),
                if r.abnormal { " abnormal" } else { "" },
                trunc(&r.cmd, 140)
            ));
        }
        if n == 0 {
            lines.push("    (none)".into());
        }
    }
    // full job-record dump for offline correlation
    if let Ok(mut f) = File::create(format!("{}/p{}.procs.tsv", c.out_dir, c.pi)) {
        let _ = writeln!(
            f,
            "#start_wall={} root_pid={} root_start={} claude={:?} cleanup_wall={}",
            c.start_wall,
            c.root.pid,
            c.root.start,
            claude_ident(&recs, c.root),
            cleanup_wall
        );
        let _ = writeln!(f, "pid\tstart\texit\tppid\tparent_start\tin_job_at_open\tseen_t\texit_msg_t\tabnormal\tsrc\texe\tcmd");
        for r in recs.iter() {
            let ex = r.h.and_then(|h| times(h.h())).map(|t| t.1).unwrap_or(0);
            let ps = parent_rec(&recs, r).map(|p| p.id.start).unwrap_or(0);
            let _ = writeln!(
                f,
                "{}\t{}\t{}\t{}\t{}\t{:?}\t{:.1}\t{}\t{}\t{}\t{}\t{}",
                r.id.pid,
                r.id.start,
                ex,
                r.ppid,
                ps,
                r.in_job_at_open,
                r.seen_t,
                r.exit_msg_t.map_or(String::new(), |t| format!("{t:.1}")),
                r.abnormal,
                r.src,
                esc_tsv(&r.exe),
                esc_tsv(&r.cmd)
            );
        }
    }
    drop(recs);
    // processes OUTSIDE our job that carry our own markers (escaped Bash-tool loops) → terminate + report
    let mut esc_kill = Vec::new();
    for r in snapshot_all() {
        let Some(h) = open_q(r.pid) else { continue };
        let cmd = cmdline(h);
        let lc = cmd.to_lowercase();
        let ours = lc.contains("t40bg=1") || lc.contains("t40fg=1");
        let inj = in_job(h, c.job.h());
        close(H::of(h));
        if ours && inj != Some(true) {
            let res = match unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION, false, r.pid) } {
                Ok(h2) => {
                    let x = if is_alive(h2) { format!("{:?}", unsafe { TerminateProcess(h2, 1) }.is_ok()) } else { "already-exited".into() };
                    close(H::of(h2));
                    x
                }
                Err(_) => "open-failed".into(),
            };
            esc_kill.push(format!("{}:{} ppid={} inJob={:?} terminated={res} cmd={}", r.exe, r.pid, r.ppid, inj, trunc(&cmd, 160)));
        }
    }
    lines.push(format!("  outside-our-job processes with T40 markers (terminated): {esc_kill:?}"));
    // outside-job leftovers since process start (report only; never killed)
    let (_all, hook) = outside_scan(c.job, c.start_wall);
    lines.push(format!("  outside-our-job hook-like processes created since this claude started (NOT killed): {hook:?}"));
    let txt = lines.join("\n");
    c.d(txt.clone());
    let _ = writeln!(out, "{txt}");
    for l in &compact {
        println!("{l}");
    }
    println!("{}", lines[..3.min(lines.len())].join("\n"));
    let n = lines.len();
    println!("{}", lines[n.saturating_sub(2)..].join("\n"));
    c.stop.store(true, Ordering::SeqCst);
    if let Some(m) = c.mon.take() {
        let _ = m.join();
    }
    for r in c.recs.lock().unwrap().iter() {
        if let Some(h) = r.h {
            close(h);
        }
    }
    close(c.port);
    close(c.job); // KILL_ON_JOB_CLOSE (already terminated)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let opt = |k: &str| a.iter().position(|x| x == k).and_then(|i| a.get(i + 1)).cloned();
    let cfg = Cfg {
        out: opt("--out").expect("--out"),
        cwd: opt("--cwd").expect("--cwd"),
        procs: opt("--procs").map_or(1, |v| v.parse().unwrap()),
        per: opt("--per").map_or(1, |v| v.parse().unwrap()),
        delay: opt("--delay").map_or(200.0, |v| v.parse().unwrap()),
        q4: a.iter().any(|x| x == "--q4"),
        model: opt("--model").unwrap_or_else(|| "haiku".into()),
        settings: opt("--settings"),
        max_trials: opt("--max-trials").map_or(15, |v| v.parse().unwrap()),
        stop_after_hangs: opt("--stop-after-hangs").map_or(99, |v| v.parse().unwrap()),
        b_target: opt("--b").map_or(0, |v| v.parse().unwrap()),
        fu_timeout: opt("--fu-timeout").map_or(120000.0, |v| v.parse().unwrap()),
    };
    std::fs::create_dir_all(&cfg.out).unwrap();
    std::fs::create_dir_all(&cfg.cwd).unwrap();
    // hard cap watchdog: exiting closes our job handles -> KILL_ON_JOB_CLOSE reaps only our jobs
    let cap_min: u64 = opt("--cap-min").map_or(60, |v| v.parse().unwrap());
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(cap_min * 60));
        println!("HARD CAP reached");
        std::process::exit(3);
    });
    let mut out = OpenOptions::new().create(true).append(true).open(format!("{}/summary.txt", cfg.out)).unwrap();
    let hdr = format!(
        "RUN {} | {} | cfg procs={} per={} delay={} q4={} model={} settings={:?} max_trials={} stop_after_hangs={} b={} fu_timeout={}",
        wall_ft(),
        self_job(),
        cfg.procs,
        cfg.per,
        cfg.delay,
        cfg.q4,
        cfg.model,
        cfg.settings,
        cfg.max_trials,
        cfg.stop_after_hangs,
        cfg.b_target,
        cfg.fu_timeout
    );
    println!("{hdr}");
    let _ = writeln!(out, "{hdr}");
    let mut total = 0usize;
    let mut hangs = 0usize;
    let mut b_done = 0usize;
    let mut fu_stats: Vec<String> = Vec::new();
    for pi in 1..=cfg.procs {
        let a_exhausted = total >= cfg.max_trials || hangs >= cfg.stop_after_hangs;
        if a_exhausted && b_done >= cfg.b_target {
            break;
        }
        let (mut c, info) = spawn_claude(&cfg, pi);
        let t_spawn_snap = c.now();
        thread::sleep(Duration::from_millis(60));
        let tree0 = tree_report(&c, c.start_wall);
        let mut head = vec![format!("PROC p{pi} {info}"), format!("  tree@spawn+{:.0}ms:", t_spawn_snap + 60.0)];
        head.extend(tree0.iter().map(|x| format!("    {x}")));
        thread::sleep(Duration::from_millis(1000));
        head.push(format!("  tree@spawn+{:.0}ms:", c.now()));
        head.extend(tree_report(&c, c.start_wall).iter().map(|x| format!("    {x}")));
        thread::sleep(Duration::from_millis(2000));
        let warm = c.turn("Reply with exactly: WARM", "warm", 120000.0);
        let ver = c
            .find(0, "system", Some("init"))
            .and_then(|(_, r)| r.and_then(|r| field(&r, "claude_code_version")))
            .unwrap_or_default();
        head.push(format!("  warm={warm} claude_code_version={ver}"));
        head.push("  tree@after-warm:".into());
        head.extend(tree_report(&c, c.start_wall).iter().map(|x| format!("    {x}")));
        let h = head.join("\n");
        println!("{h}");
        let _ = writeln!(out, "{h}");
        c.d(h);
        let mut trials = Vec::new();
        let mut keep_all = !warm.starts_with("TIMEOUT");
        for k in 1..=cfg.per {
            if !keep_all || total >= cfg.max_trials || hangs >= cfg.stop_after_hangs {
                break;
            }
            total += 1;
            let label = format!("p{pi}t{k}(#{total})");
            let (tr, hang, keep) = run_trial(&mut c, &cfg, &label, false);
            if hang {
                hangs += 1;
                let fu = tr.outcome.lines().find(|l| l.contains("followUp")).unwrap_or("followUp=<none>").trim().to_string();
                fu_stats.push(format!("{label}: {fu}"));
            }
            println!("{}", tr.outcome);
            let _ = writeln!(out, "{}", tr.outcome);
            c.d(tr.outcome.clone());
            trials.push(tr);
            if !keep {
                keep_all = false;
            }
        }
        if keep_all && b_done < cfg.b_target {
            let (bs, bg_alive) = run_b(&mut c);
            b_done += 1;
            let hdr = format!("B p{pi} (#{b_done})\n{bs}");
            println!("{hdr}");
            let _ = writeln!(out, "{hdr}");
            if bg_alive {
                let label = format!("p{pi}bg");
                let (tr, hang, _keep) = run_trial(&mut c, &cfg, &label, true);
                let o = format!("{} [bg-loop alive; report-only except hook-chain kills; hang={hang}]", tr.outcome);
                println!("{o}");
                let _ = writeln!(out, "{o}");
                c.d(o);
                trials.push(tr);
            }
        }
        finish(c, &trials, &mut out);
        println!("PROC p{pi} done (A total={total} hangs={hangs} B done={b_done})");
    }
    println!("FOLLOWUPS: {fu_stats:?}");
    let _ = writeln!(out, "FOLLOWUPS: {fu_stats:?}");
    let f = format!("DONE total_trials={total} hangs={hangs} b_done={b_done}");
    println!("{f}");
    let _ = writeln!(out, "{f}");
}
