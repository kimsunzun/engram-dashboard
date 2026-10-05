//! 실물 조회 스포너 — [`ProbeSpawner`] 를 OS 프로세스로 구현한다(ADR-0249).
//!
//! ★이 모듈은 OS 를 모른다★: 트리 뿌리로 띄우기 · 트리 kill · 실패한 셸의 「프로그램 없음」 판정은 OS 층
//!   ([`prepare_tree_root`] · [`TreeRoot`] · [`MissingProgramCheck`])이 하고, 여기는 그 답을 조회의 오류 · 종료
//!   정보로 옮기고 찍는다.
//!
//! 스레드 셋이 자식 하나를 받친다 — 아무도 join 하지 않으므로 drop 이 [`KILL_WAIT`] 보다 오래 걸리지 않는다:
//!   - stdin 라이터: stdin 을 닫을 때·자식 값이 drop 될 때(송신단이 사라질 때) 또는 쓰기가 실패할 때 끝난다.
//!     OS 쓰기에 막혀 있으면 트리 kill 이 파이프를 끊어 풀어 준다.
//!   - stdout 줄 리더: EOF(쓰기 끝을 쥔 프로세스가 모두 끝남) 또는 받는 쪽이 사라진 뒤의 첫 전달에서 끝난다.
//!     ★트리 kill 을 빠져나간 프로세스가 stdout 을 쥐고 있으면 그것이 끝날 때까지 이 스레드가 남는다★.
//!   - stderr 드레인: EOF 에서 끝난다. 내용은 버리고 가린 꼬리 몇 줄만 붙든다(로그에는 줄 수만).
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

// ── 자식 ─────────────────────────────────────────────────────────────────────────

struct OsProbeChild {
    child: Child,
    tree: TreeRoot,
    not_installed: MissingProgramCheck,
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
        // 여기부터 실패하면 이 값의 drop 이 트리를 끊는다.
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

/// 도는 중인 찾기에 막혀 판정을 「모른다」로 둔 것을 경고했나 — 경고는 프로세스당 한 번이고 그 뒤는 debug 다.
static STUCK_LOOKUP_WARNED: AtomicBool = AtomicBool::new(false);

fn judged_not_installed(check: &MissingProgramCheck, code: Option<i32>, deadline: Instant) -> bool {
    let verdict = check.judge(code, deadline);
    if let MissingProgramVerdict::Looked(found) = &verdict {
        match found {
            TargetLookup::Busy => {
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

/// 트리 kill + 유계 종료 대기. 정상 종료한 자식에도 kill 을 보낸다 — 자식이 남긴 손자가 있을 수 있다.
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

    /// ★트리 전부가 끝난다★ — 셸의 자식(손자 `ping` · 콘솔 호스트)까지. 멈춘 채 띄워 넣은 뒤 깨우므로 곧바로 띄운
    ///   손자도 트리를 빠지지 않고, drop 은 그 전부가 끝난 뒤에 돌아온다.
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

    /// `cmd.exe /c <없는 이름>` 은 뜬다 — 실패한 뒤의 종료 상태가 「없다」를 싣는다(종료 코드는 이 PC 에서 1 이라
    /// 코드만으로는 못 가른다).
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

    /// 판정 없는 종료 대기는 판정도 하지 않고 판정 없는 종료를 받아 두지도 않는다 — 뒤의 `wait_exit` 가 판정한다.
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
        // 대상이 그 사이에 사라진다 — 종료 대기가 판정해 받아 두었다면 「있다」로 굳어 뒤의 판정이 거짓이 된다.
        std::fs::remove_file(&shim).expect("대상 지우기");
        assert!(child.wait_exit(far()).expect("종료").looks_not_installed());
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
