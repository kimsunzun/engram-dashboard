//! The real query spawner — implements [`ProbeSpawner`] with an OS process (ADR-0249).
//!
//! ★This module does not know the OS★: spawning as a tree root · tree kill · the 「프로그램 없음」 (program missing) decision for a failed shell is done by the OS layer
//!   ([`prepare_tree_root`] · [`TreeRoot`] · [`MissingProgramCheck`]), and here we move its answer into the query's error · exit
//!   info and print it.
//!
//! Three threads back one child — no one joins them, so a drop takes no longer than [`KILL_WAIT`]:
//!   - stdin writer: ends when stdin is closed · when the child value is dropped (when the sending end disappears) or when a write fails.
//!     If blocked on an OS write, the tree kill breaks the pipe and frees it.
//!   - stdout line reader: ends at EOF (every process holding the write end has ended) or at the first delivery after the receiving side has gone.
//!     ★If a process that escaped the tree kill holds stdout, this thread stays until it ends★.
//!   - stderr drain: ends at EOF. It discards the content and keeps only the last few masked lines (only the line count goes to logs).
// ADR-0230

use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStderr, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use engram_dashboard_base::logging::mask_secrets;
use engram_dashboard_base::sync;
use engram_dashboard_platform::spawn::{
    prepare_tree_root, MissingProgramCheck, MissingProgramVerdict, TargetLookup, TreeRoot,
};

use super::{ExitInfo, ProbeChild, ProbeCommand, ProbeError, ProbeSpawner};

/// Upper bound on waiting after kill for the child (on Windows, everything in the Job) to end. It is the share that waits for file locks to be
/// released before the scratch folder is deleted (Windows cannot delete open files · a working folder).
pub const KILL_WAIT: Duration = Duration::from_secs(2);

const WAIT_POLL: Duration = Duration::from_millis(20);

/// Bound of one line of stdout. A query response is a few KB and even the longest line (the initialization notice) is far smaller than this.
const STDOUT_LINE_MAX: usize = 1024 * 1024;
/// Bound of stdout lines not yet read — when full the reader stops and the child blocks on the pipe (memory bound).
const STDOUT_QUEUE: usize = 16;

const STDERR_READ_MAX: usize = 4096;
const STDERR_TAIL_LINES: usize = 8;
const STDERR_TAIL_CHARS: usize = 240;

/// A spawner that spawns real OS processes.
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

// ── Line reading ──────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
enum LineEnd {
    /// The stream ended without a single byte.
    Eof,
    Complete,
    /// Hit the bound — ★the rest of the line is not read yet★. Call [`skip_rest_of_line`] before reading the next line.
    Cut,
}

/// Puts one line (including the trailing `\n`; a last piece ending without a newline is also a line) into `out` up to `cap` bytes.
/// If the bound is hit, it is `Cut` right away without waiting for the rest — it returns soon even for output pouring in without newlines.
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
                        // Exactly filled the bound without a newline — waiting for the next byte would block until the deadline in front of a stalled child.
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

/// Reads and discards the remaining bytes of the line up to the newline (inclusive) or EOF — does not hold them in memory.
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

/// Shape one stderr line for the tail — ★masking comes before truncating★ (if truncated first and masked after, a token straddling the boundary
/// is cut below the pattern length and slips through).
///
/// A line cut at the read bound is used only up to before the last whitespace — a masked token shape has no whitespace, so a token piece straddling that boundary
/// cannot slip through the masking. If there is no whitespace, the line is dropped whole.
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

// ── Thread bodies ─────────────────────────────────────────────────────────────

fn pump_stdin(mut stdin: ChildStdin, lines: Receiver<Vec<u8>>, acks: Sender<io::Result<()>>) {
    for bytes in lines {
        let result = stdin.write_all(&bytes).and_then(|()| stdin.flush());
        let failed = result.is_err();
        if acks.send(result).is_err() || failed {
            return;
        }
    }
}

/// A line that hit the bound without a newline (a line over the bound · a piece that exactly fills the bound without a newline) sends `Io` the moment it hits, and the
/// rest of that line is read and discarded before moving to the next line.
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
        let mut kept = sync::lock(&tail);
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

// ── Child ─────────────────────────────────────────────────────────────────────

struct OsProbeChild {
    child: Child,
    tree: TreeRoot,
    not_installed: MissingProgramCheck,
    /// `None` = stdin has closed (write failure · writer ended · [`ProbeChild::close_stdin`]).
    stdin_tx: Option<Sender<Vec<u8>>>,
    write_acks: Receiver<io::Result<()>>,
    stdout_rx: Receiver<Result<String, ProbeError>>,
    stdout_eof: bool,
    stderr_tail: Arc<Mutex<StderrTail>>,
    exit: Option<ExitInfo>,
    /// Cut by the deadline — every later action is immediately `Timeout`.
    cut: bool,
}

impl OsProbeChild {
    fn spawn(cmd: &ProbeCommand, deadline: Instant) -> Result<Self, ProbeError> {
        if remaining(deadline).is_none() {
            return Err(ProbeError::Timeout);
        }
        // A working folder that does not exist falls to `NotFound` on POSIX and is mistaken for 「미설치」 (not installed) — tell them apart first.
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
        prepare_tree_root(&mut command);

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
        let tree = match TreeRoot::attach(&child) {
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
        // If anything fails from here on, the drop of this value cuts the tree.
        let this = Self {
            child,
            tree,
            not_installed: MissingProgramCheck::new(
                &cmd.program,
                &cmd.args,
                &cmd.env_set,
                &cmd.env_remove,
                &cmd.cwd,
            ),
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

    fn kill_tree(&mut self) {
        if let Err(e) = self.tree.kill(&mut self.child) {
            tracing::debug!(
                pid = self.child.id(),
                "사용량 조회: 그룹 kill 실패 — 자식만 끊는다: {e}"
            );
        }
    }

    fn cut_at_deadline(&mut self, during: &'static str) -> ProbeError {
        if !self.cut {
            self.cut = true;
            self.kill_tree();
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

    /// Exit code of the directly spawned process — the decision is the caller's share ([`ProbeChild::wait_exit`]).
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

/// Whether we warned that a decision was left as 「모른다」 (unknown) because it was blocked by a running search — the warning is once per process and after that it is debug.
static STUCK_LOOKUP_WARNED: AtomicBool = AtomicBool::new(false);

fn judged_not_installed(check: &MissingProgramCheck, code: Option<i32>, deadline: Instant) -> bool {
    let verdict = check.judge(code, deadline);
    if let MissingProgramVerdict::Looked(found) = &verdict {
        match found {
            TargetLookup::Busy => {
                // Usually an earlier query's search was left blocked past its deadline — abnormal but a safe fallback (「모른다」 (unknown)).
                if STUCK_LOOKUP_WARNED.swap(true, Ordering::Relaxed) {
                    tracing::debug!(
                        "사용량 조회: 같은 대상의 앞선 찾기가 아직 끝나지 않아 새로 찾지 않는다"
                    );
                } else {
                    tracing::warn!(
                        "사용량 조회: 같은 대상의 앞선 「미설치」 찾기가 아직 끝나지 않아 판정을 「모른다」로 둔다 — 막힌 파일 시스템 조회가 남았을 수 있다(이 경고는 프로세스당 한 번)"
                    );
                }
            }
            TargetLookup::NoThread(e) => {
                tracing::debug!("사용량 조회: 대상 찾기 스레드를 띄우지 못했다: {e}");
            }
            TargetLookup::Found | TargetLookup::Missing | TargetLookup::Unknown => {}
        }
        tracing::debug!(?found, "사용량 조회: 실패한 셸의 대상을 찾아봤다");
    }
    verdict.is_missing()
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
        let exit = ExitInfo::new(
            code,
            judged_not_installed(&self.not_installed, code, deadline),
        );
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
        sync::lock(&self.stderr_tail)
            .lines
            .iter()
            .cloned()
            .collect()
    }
}

/// Tree kill + bounded wait for exit. Sends kill even to a child that exited normally — the child may have left a grandchild.
impl Drop for OsProbeChild {
    fn drop(&mut self) {
        self.kill_tree();
        self.stdin_tx = None;
        let pid = self.child.id();
        if !self.wait_tree_gone(KILL_WAIT) {
            tracing::warn!(
                pid,
                wait_ms = KILL_WAIT.as_millis() as u64,
                "사용량 조회 프로세스가 kill 뒤 대기 상한 안에 끝나지 않았다 — 임시 폴더가 남을 수 있다"
            );
        }
        let stderr_lines = sync::lock(&self.stderr_tail).total_lines;
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

    /// A command that runs one shell line — on Windows `cmd.exe /v:on /c` (delayed expansion `!VAR!`), on POSIX `sh -c`.
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

    /// A child that waits on stdin and never ends.
    fn waits_for_stdin(cwd: &Path) -> ProbeCommand {
        shell("set /p L=", "read l", cwd)
    }

    /// A child that does not read stdin and lives for a while (on Windows it places a grandchild `ping`).
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

    // ── Pure helpers ──

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
        // A line exactly at the bound including the newline is not truncated — the bound counts the newline byte.
        assert_eq!(read(5), (LineEnd::Complete, b"wxyz\n".to_vec()));
        assert_eq!(read(8), (LineEnd::Complete, b"last".to_vec()));
        assert_eq!(read(8), (LineEnd::Eof, Vec::new()));
    }

    /// A read end that blocks until `release` disappears after all the data is given — the stdout of a child that stopped without a newline.
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

    /// ★The moment it hits the bound it is `Io`★ — it does not wait until a newline comes (= until the deadline if it never comes). A line that fills the bound exactly
    ///   without a newline and stops is the same (it does not wait for the next byte).
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
        // The read bound cut right through the middle of a token — the cut piece cannot be masked, so use only up to before the last whitespace.
        let cut = format!("key {}", &token[..12]);
        assert_eq!(tail_line(cut.as_bytes(), true), "key");
        assert_eq!(tail_line(token.as_bytes(), true), "");
        let long = "x".repeat(STDERR_TAIL_CHARS * 2);
        assert_eq!(
            tail_line(long.as_bytes(), false).chars().count(),
            STDERR_TAIL_CHARS
        );
    }

    /// An exit status received after the end is also `Timeout` once cut by the deadline — every action after being cut is `Timeout`.
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

    // ── Real child ──

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

        // After being cut, anything is immediately Timeout.
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
        // Far larger than the pipe buffer — it surely blocks in front of a child that does not read.
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

    /// The child's share of the test below — respawns this test binary and runs only this item. If the marker env is absent, it does nothing.
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
        // SAFETY: closes the stdout handle of this test child process once — after closing, nothing is written to stdout.
        drop(unsafe { OwnedHandle::from_raw_handle(raw) });
    }

    #[cfg(unix)]
    fn close_own_stdout() {
        use std::os::unix::io::{FromRawFd, OwnedFd};
        // SAFETY: closes fd 1 of this test child process once — after closing, nothing is written to stdout.
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

        // The test harness's header line may come first — drain to EOF.
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

    /// ★The whole tree ends★ — down to the shell's children (grandchild `ping` · console host). It is spawned suspended, put into the tree, and then woken, so even a
    ///   grandchild spawned right away does not leave the tree, and drop returns after all of it has ended.
    #[cfg(windows)]
    #[test]
    fn drop_kills_the_whole_tree() {
        use engram_dashboard_platform::process::{child_pids, pid_alive};
        use engram_dashboard_platform::testing::is_ping;

        let root = TempRoot::new("probe-tree");
        let child = OsProbeChild::spawn(&ignores_stdin(root.path()), far()).expect("기동");
        let pid = child.pid();
        eventually("손자 ping 이 떴다", || {
            child_pids(pid).into_iter().any(is_ping)
        });
        let descendants = child_pids(pid);

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

    /// `cmd.exe /c <없는 이름>` does start — the exit status after the failure carries 「없다」 (missing) (the exit code is 1 on this PC, so
    /// the code alone cannot tell).
    #[cfg(windows)]
    #[test]
    fn cmd_with_a_missing_target_spawns_and_its_exit_says_not_installed() {
        let root = TempRoot::new("probe-missing-cmd");
        let missing = format!("engram-no-such-program-{}", Uuid::new_v4().simple());
        let mut child = OsProbeChild::spawn(&cmd_c(&missing, root.path()), far()).expect("기동");
        assert_eq!(read_to_eof(&mut child), Vec::<String>::new());
        let exit = child.wait_exit(far()).expect("종료");
        assert_ne!(exit.code(), Some(0));
        assert!(exit.looks_not_installed(), "{exit:?}");
    }

    /// A wait for exit without a decision neither makes a decision nor keeps an exit without a decision — the later `wait_exit` decides.
    #[cfg(windows)]
    #[test]
    fn the_unjudged_wait_leaves_the_judgment_to_wait_exit() {
        let root = TempRoot::new("probe-unjudged-wait");
        let target = format!("engram-probe-gone-{}", Uuid::new_v4().simple());
        let shim = root.path().join(format!("{target}.cmd"));
        std::fs::write(&shim, "@exit 1").expect("shim");
        let mut child = OsProbeChild::spawn(&cmd_c(&target, root.path()), far()).expect("기동");
        read_to_eof(&mut child);
        assert_eq!(child.wait_exit_code(far()).expect("종료"), Some(1));
        // The target disappears in the meantime — if the wait for exit had decided and kept it, it would harden to 「있다」 (exists) and the later decision would be false.
        std::fs::remove_file(&shim).expect("대상 지우기");
        assert!(child.wait_exit(far()).expect("종료").looks_not_installed());
    }

    /// Even if a target that exists fails by itself, that is not 「없다」 (missing) — the decision stands only after the target has been looked for.
    #[cfg(windows)]
    #[test]
    fn a_present_cmd_target_that_fails_is_not_mistaken_for_not_installed() {
        let root = TempRoot::new("probe-present-cmd");
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).expect("bin");
        std::fs::write(bin.join("engram-probe-fail.cmd"), "@exit 5").expect("shim");
        let mut cmd = cmd_c("engram-probe-fail", root.path());
        cmd.env_set = vec![("PATH".to_owned(), bin.to_string_lossy().into_owned())];
        let mut child = OsProbeChild::spawn(&cmd, far()).expect("기동");
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
        // Writing to the stdin of an ended child fails without hanging.
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
        // Strips the variable that the parent always has.
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

    /// Windows cannot delete a folder the child holds as its working folder — dropping the child first lets it be deleted.
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
