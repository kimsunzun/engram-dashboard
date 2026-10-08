//! 기록기 — 부팅 단계 뒤 `state.json` 을 쓰는 유일한 쪽이자 크래시 사본 `state.crash.json` 을 지우는 유일한
//! 쪽(TRD S21-storage §6-4). 스레드 하나가 돌고, 다른 스레드는 [`SaverHandle`] 로 요청을 보내 답을 기다릴
//! 뿐이다 — 쓰는 쪽이 하나라 쓰기끼리 겹치지 않는다.
//!
//! - **주기 저장:** [`POLL_INTERVAL`] 마다 변경 번호를 읽어, 바뀌었으면 마지막 변경 뒤 [`QUIET`] 동안
//!   조용하거나 첫 변경 뒤 [`MAX_DELAY`] 가 지났을 때 스냅숏을 통째로 쓴다.
//! - **쓰기 실패 = 로그만**(사용자 결정 D8). 스냅숏 원천 실패 · 디스크 오류면 그 변경을 안 쓴 것으로 남겨
//!   디바운스가 다시 쓰고, 코덱이 거절한 스냅숏(다음 부팅이 못 읽을 모양)은 다음 변경을 기다린다. 정상 종료
//!   쓰기(`Final`)는 실패해도 다시 하지 않는다 — 기록기가 그대로 끝난다.
//! - **해결 칸** = 답한 크래시 사본의 해시. ★기록기가 `Resolve` 를 **꺼낼 때** 선다★ — 보내는 쪽이 세우면 꺼내기
//!   전에 뜬 스냅숏(답 뒤의 화면을 아직 안 담은 것)이 해시를 싣는다(I8). 선 뒤로 쓰는 스냅숏마다 실리고, 쓰기가
//!   성공할 때마다 사본을 다시 읽어 해시가 같을 때만 지운다(N1). 지웠거나 · 없거나 · 다른 사본이면 칸을 비운다.
//! - **닫힘** = [`SaverHandle::finish`] 가 마감을 넘기면 세우는 표지. 선 뒤로는 발행(rename)도 사본 지우기도
//!   하지 않는다 — 다음 부팅이 직전의 `clean_exit:false` 를 읽고 묻는다.
//!
//! 기록기는 락을 쥐지 않는다 — 스냅숏 원천이 자기 락 안에서 복사해 건네고, 직렬화 · 디스크는 그 밖에서 한다
//! (§6-3).

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::codec::{self, EncodeError, STATE_READ_CAP};
use super::schema::{StateFile, WindowEntry, STATE_VERSION};
use crate::fsutil::WriteOutcome;

pub const POLL_INTERVAL: Duration = Duration::from_millis(500);
pub const QUIET: Duration = Duration::from_secs(1);
pub const MAX_DELAY: Duration = Duration::from_secs(5);
/// 종료(`Final`)와 복원 확정(`Resolve`)이 답을 기다리는 마감(TRD §6-4 · §6-7). ★부팅의 셸 실행 잠금 대기(약
/// 3초 — §6-5 ①)가 이보다 길어야 한다★ — 짧으면 새 인스턴스가 앞 인스턴스의 정상 종료 쓰기를 기다리지 않는다.
pub const REPLY_DEADLINE: Duration = Duration::from_secs(2);

/// 스냅숏 원천 — 운영은 `ViewManager` 다([`super::boot_plugin::LiveSource`]). 기록기 스레드에서 불린다.
///
/// - ★패닉하지 않는다★ — 릴리스는 `panic = "abort"`(워크스페이스 `Cargo.toml`)라 이 스레드의 패닉이 앱을
///   통째로 죽인다. 독 든 락은 `PoisonError::into_inner` 로 되살리거나 [`Self::snapshot`] 이 `Err` 를 돌려준다.
/// - ★[`SaverHandle::finish`] · [`SaverHandle::resolve`] 를 부른 스레드를 기다리지 않는다★ — 그 스레드는 답을
///   기다리며 서 있다. 그 스레드가 쥔 락을 잡으려 들면 기록기가 마감까지 서 있게 되고, `finish` 면 정상 종료가
///   매번 마감을 넘겨 모든 정상 종료가 비정상 종료로 읽힌다.
/// - ★Tauri 창 게터(`outer_position` · `inner_size` · `is_maximized` 등)를 부르지 않는다★ — 메인 스레드 밖에서
///   부르면 메인 스레드가 답할 때까지 서는데, `finish` 는 `RunEvent::Exit` 에서 바로 그 메인 스레드가 부르고
///   답을 기다린다. 위치 · 크기는 `WindowEvent` 에서 모델에 적어 두고(P3b) 여기서는 모델만 읽는다.
pub trait SnapshotSource: Send + 'static {
    /// 변경 번호 — 기록기는 같은지만 본다(대소를 보지 않는다). ★번호를 되돌리거나 초기화하지 않는다★ — 기록기가
    /// 마지막으로 본 값과 다시 같아지면 그 사이의 변경을 못 본다.
    type Revision: Clone + PartialEq + Send + 'static;

    /// 0.5초마다 부른다 — 짧게 끝나야 한다.
    ///
    /// ★번호는 그 변경이 [`Self::snapshot`] 에 보이는 것과 같은 임계구역에서, 또는 그 뒤에 올라야 한다★ — 기록기는
    /// 번호를 먼저 읽고 스냅숏을 뜬 뒤 그 번호를 「쓴 것」으로 친다. 번호가 변경보다 먼저 오르면 그 변경을 빠뜨린
    /// 스냅숏이 다음 변경 때까지 디스크의 마지막 상태로 남는다.
    fn revision(&self) -> Self::Revision;

    /// 창 전부 — 자기 락 안에서 복사해 소유한 값으로 돌려준다. 파일 머리(`version` · `saved_at_ms` ·
    /// `clean_exit` · `resolved_crash_copy`)는 기록기가 채운다.
    ///
    /// `Err` = 이번엔 못 뜬다(사유 한 줄) — 기록기는 디스크 오류와 같이 실패한 쓰기로 치고 디바운스 뒤 다시
    /// 뜬다(정상 종료 쓰기면 그대로 끝난다).
    fn snapshot(&self) -> Result<Vec<WindowEntry>, String>;
}

/// 기록기가 만지는 두 파일 — 운영은 [`Fs`].
pub trait StateFiles: Send + 'static {
    /// `state.json` 을 원자적으로 갈아끼운다 — `skip` 을 rename 앞마다 물어 서 있으면 발행하지 않고
    /// [`WriteOutcome::Skipped`](`crate::fsutil::write_atomic_unless` 와 같은 계약).
    fn write_state(&self, text: &str, skip: &dyn Fn() -> bool) -> io::Result<WriteOutcome>;

    /// 크래시 사본의 원문 — 없으면 `NotFound`, 상한 초과 · UTF-8 아님은 `InvalidData`
    /// (`crate::fsutil::read_file_capped` 와 같은 계약).
    fn read_crash_copy(&self) -> io::Result<String>;

    /// 없으면 `NotFound`.
    fn remove_crash_copy(&self) -> io::Result<()>;
}

pub trait Clock: Send + 'static {
    /// 디바운스만 잰다.
    fn now(&self) -> Instant;
    /// 스냅숏의 `saved_at_ms` — 유닉스 시각 ms.
    fn wall_ms(&self) -> u64;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn wall_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as u64)
    }
}

/// 운영 파일 — 경로는 부르는 쪽이 정한다(배치 = [`super::boot_plugin`]).
pub struct Fs {
    state: PathBuf,
    crash_copy: PathBuf,
}

impl Fs {
    pub fn new(state: PathBuf, crash_copy: PathBuf) -> Self {
        Self { state, crash_copy }
    }
}

impl StateFiles for Fs {
    fn write_state(&self, text: &str, skip: &dyn Fn() -> bool) -> io::Result<WriteOutcome> {
        crate::fsutil::write_atomic_unless(&self.state, text, skip)
    }

    fn read_crash_copy(&self) -> io::Result<String> {
        crate::fsutil::read_file_capped(&self.crash_copy, STATE_READ_CAP)
    }

    // 잠김 재시도를 하지 않는다 — 못 지우면 해결 칸이 남아 뒤의 성공 쓰기마다 다시 지운다.
    fn remove_crash_copy(&self) -> io::Result<()> {
        std::fs::remove_file(&self.crash_copy)
    }
}

/// 기록기 쓰기 하나의 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    /// 발행했다(rename).
    Written,
    /// 닫힌 뒤라 발행하지 않았다 — 실패가 아니고 다시 하지 않는다.
    Skipped,
    /// 스냅숏 원천 실패 · 코덱 거절([`EncodeError`]) · 디스크 오류. 해결 칸은 그대로라 다음 성공 쓰기가
    /// 싣는다 — 정상 종료 쓰기(`Final`)의 실패 뒤엔 다음 쓰기가 없다.
    Failed,
}

/// 요청을 보내고 마감까지 기다린 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestOutcome {
    Done(SaveOutcome),
    /// 마감 안에 답이 없었다 — 요청은 줄에 남아 나중에 처리될 수 있다.
    TimedOut,
    /// 답이 오지 않는다 — 기록기 스레드가 이미 끝났거나 처리 도중 죽었다.
    SaverGone,
}

/// 기록기에 요청을 보내는 손잡이 — 복제해 나눠 쥔다. 모든 손잡이가 사라지면 기록기는 정상 종료 쓰기 없이
/// 끝난다(다음 부팅은 비정상 종료로 읽는다).
#[derive(Clone)]
pub struct SaverHandle {
    requests: Sender<Request>,
    closed: Arc<AtomicBool>,
}

impl SaverHandle {
    /// 답한 크래시 사본의 해시를 디스크에 붙인다(TRD §6-7 ⑤). `Done(Written)` = 그 해시를 실은 `state.json` 이
    /// 발행됐다 — 사본 지우기의 성패는 담지 않는다.
    ///
    /// ★기다리는 동안 [`SnapshotSource`] 가 잡을 락(`ViewManager`)을 쥐지 않는다★ — 쥐면 기록기가 그
    /// 락에서 마감까지 서 있어 답이 `TimedOut` 이 된다.
    pub fn resolve(&self, hash: String, deadline: Duration) -> RequestOutcome {
        match self.send(|reply| Request::Resolve { hash, reply }) {
            Some(answer) => wait(answer, deadline),
            None => RequestOutcome::SaverGone,
        }
    }

    /// `clean_exit:true` 를 쓰고 기록기를 끝낸다. ★마감을 넘기면 닫힘을 세우고 돌아간다 — 갇힌 스레드를 기다리지
    /// 않는다★. 그 뒤 기록기는 발행하지 않는다 — 단 닫힘을 세우기 직전에 rename 을 지난 쓰기는 이미 발행됐다
    /// (그 내용은 이 `Final` 이거나 더 이른 `clean_exit:false` 다 — §12 R12).
    ///
    /// ★기다리는 동안 [`SnapshotSource`] 가 잡을 락(`ViewManager`)을 쥐지 않는다★ — 쥐면 `Final` 이 늘
    /// 마감을 넘겨 모든 정상 종료가 비정상 종료로 읽힌다.
    pub fn finish(&self, deadline: Duration) -> RequestOutcome {
        let outcome = match self.send(|reply| Request::Final { reply }) {
            Some(answer) => wait(answer, deadline),
            None => RequestOutcome::SaverGone,
        };
        if outcome == RequestOutcome::TimedOut {
            self.closed.store(true, Ordering::SeqCst);
            tracing::warn!(
                module = "state",
                deadline_ms = deadline.as_millis() as u64,
                "정상 종료 쓰기가 마감 안에 끝나지 않아 기록기를 닫는다 — 다음 부팅은 비정상 종료로 읽는다"
            );
        }
        outcome
    }

    /// 닫힘을 세운다 — 그 뒤 기록기는 발행하지 않는다(쓰기 · `Resolve` · `Final` 모두 `Skipped`). 스레드는 손잡이가
    /// 모두 사라질 때 끝난다.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    fn send(
        &self,
        request: impl FnOnce(Sender<SaveOutcome>) -> Request,
    ) -> Option<Receiver<SaveOutcome>> {
        let (reply, answer) = mpsc::channel();
        self.requests.send(request(reply)).ok().map(|()| answer)
    }
}

/// 기록기의 닫힘 표지 — [`spawn`] 앞에 만들어 띄우는 쪽과 나눠 쥔다. 세우면 그 기록기는 발행하지 않는다
/// ([`SaverHandle::close`] 와 같은 표지). 띄우는 동안 손잡이가 아직 없는 쪽(종료)도 이것으로 닫을 수 있다.
#[derive(Clone, Default)]
pub struct CloseFlag(Arc<AtomicBool>);

impl CloseFlag {
    pub fn close(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn wait(answer: Receiver<SaveOutcome>, deadline: Duration) -> RequestOutcome {
    match answer.recv_timeout(deadline) {
        Ok(outcome) => RequestOutcome::Done(outcome),
        Err(RecvTimeoutError::Timeout) => RequestOutcome::TimedOut,
        Err(RecvTimeoutError::Disconnected) => RequestOutcome::SaverGone,
    }
}

/// 기록기 스레드를 띄운다. `boot_revision` = 부팅 첫 쓰기(§6-5 ⑤)가 담은 변경 번호 — 그 뒤 바뀐 것을 첫 주기가
/// 싣는다. `carry_resolved` = 부팅이 못 지운 답한 사본의 해시 — 첫 쓰기가 이미 실었으므로 다음 성공 쓰기 뒤
/// 지우기를 다시 해 본다. `closed` = 이 기록기의 닫힘 표지 — 띄우기 전에 서 있으면 처음부터 발행하지 않는다.
///
/// `Err` = 스레드를 못 띄웠다.
pub fn spawn<S, F, C>(
    source: S,
    files: F,
    clock: C,
    boot_revision: S::Revision,
    carry_resolved: Option<String>,
    closed: CloseFlag,
) -> io::Result<SaverHandle>
where
    S: SnapshotSource,
    F: StateFiles,
    C: Clock,
{
    let saver = Saver::new(
        source,
        files,
        clock,
        closed.0,
        boot_revision,
        carry_resolved,
    );
    start(saver, POLL_INTERVAL).map(|(handle, _detached)| handle)
}

fn start<S, F, C>(
    saver: Saver<S, F, C>,
    poll: Duration,
) -> io::Result<(SaverHandle, JoinHandle<()>)>
where
    S: SnapshotSource,
    F: StateFiles,
    C: Clock,
{
    let (requests, inbox) = mpsc::channel();
    let handle = SaverHandle {
        requests,
        closed: Arc::clone(&saver.closed),
    };
    let thread = std::thread::Builder::new()
        .name("state-saver".into())
        .spawn(move || run(saver, inbox, poll))?;
    Ok((handle, thread))
}

enum Request {
    Resolve {
        hash: String,
        reply: Sender<SaveOutcome>,
    },
    Final {
        reply: Sender<SaveOutcome>,
    },
}

fn run<S, F, C>(mut saver: Saver<S, F, C>, inbox: Receiver<Request>, poll: Duration)
where
    S: SnapshotSource,
    F: StateFiles,
    C: Clock,
{
    let _panic_note = PanicNote;
    tracing::info!(module = "state", "기록기 시작");
    // 요청이 잦아도 확인이 밀리지 않게 마감 시각으로 잰다.
    let mut next_tick = saver.clock.now() + poll;
    loop {
        let wait = next_tick.saturating_duration_since(saver.clock.now());
        match inbox.recv_timeout(wait) {
            Ok(Request::Resolve { hash, reply }) => {
                let outcome = saver.resolve(hash);
                deliver(reply, outcome);
            }
            Ok(Request::Final { reply }) => {
                let outcome = saver.save(Cause::Final);
                deliver(reply, outcome);
                tracing::info!(
                    module = "state",
                    ?outcome,
                    "기록기 종료 — 정상 종료 쓰기 뒤"
                );
                return;
            }
            Err(RecvTimeoutError::Timeout) => {
                saver.tick();
                next_tick = saver.clock.now() + poll;
            }
            Err(RecvTimeoutError::Disconnected) => {
                tracing::warn!(
                    module = "state",
                    "손잡이가 모두 사라져 기록기가 정상 종료 쓰기 없이 끝난다 — 다음 부팅은 비정상 종료로 읽는다"
                );
                return;
            }
        }
    }
}

/// 기록기 스레드가 패닉으로 풀릴 때 한 줄 남긴다. ★디버그 · 시험 빌드에서만 닿는다★ — 릴리스는
/// `panic = "abort"` 라 패닉이 이 줄 없이 프로세스째 끝낸다.
struct PanicNote;

impl Drop for PanicNote {
    fn drop(&mut self) {
        if std::thread::panicking() {
            tracing::error!(
                module = "state",
                "기록기 스레드가 패닉으로 죽었다 — 이 실행은 상태를 더 저장하지 않는다"
            );
        }
    }
}

fn deliver(reply: Sender<SaveOutcome>, outcome: SaveOutcome) {
    if reply.send(outcome).is_err() {
        tracing::debug!(
            module = "state",
            ?outcome,
            "요청한 쪽이 마감을 넘겨 답을 받지 않았다"
        );
    }
}

#[derive(Debug, Clone, Copy)]
enum Cause {
    Periodic,
    Resolve,
    Final,
}

#[derive(Debug)]
enum Failure {
    Snapshot(String),
    Rejected(EncodeError),
    Disk(io::Error),
}

impl Failure {
    fn kind(&self) -> FailureKind {
        match self {
            Failure::Snapshot(_) => FailureKind::Snapshot,
            Failure::Rejected(_) => FailureKind::Rejected,
            Failure::Disk(_) => FailureKind::Disk,
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Snapshot(reason) => f.write_str(reason),
            Failure::Rejected(error) => error.fmt(f),
            Failure::Disk(error) => error.fmt(f),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    Snapshot,
    Rejected,
    Disk,
}

/// 답한 사본 정리(N1)가 실패한 갈래.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyFailure {
    Read,
    Remove,
}

/// 끊기지 않고 이어진 실패. 갈래가 처음이거나 바뀔 때만 제 레벨로 남기고, 같은 갈래의 되풀이는 debug 로 접는다
/// — 디스크 오류가 이어지면 1초마다 다시 쓰므로 접지 않으면 파일 로그가 그 줄로 찬다.
#[derive(Debug, PartialEq, Eq)]
struct Streak<K> {
    current: Option<(K, u32)>,
}

impl<K> Default for Streak<K> {
    fn default() -> Self {
        Self { current: None }
    }
}

impl<K: Copy + PartialEq> Streak<K> {
    /// 실패 하나를 센다 — `true` = 제 레벨로 남길 차례다.
    fn fail(&mut self, kind: K) -> bool {
        match &mut self.current {
            Some((current, failures)) => {
                *failures += 1;
                let changed = *current != kind;
                *current = kind;
                changed
            }
            None => {
                self.current = Some((kind, 1));
                true
            }
        }
    }

    fn failures(&self) -> u32 {
        self.current.map_or(0, |(_, failures)| failures)
    }

    /// `Some(n)` = 이어지던 실패가 n번 만에 끝났다.
    fn end(&mut self) -> Option<u32> {
        self.current.take().map(|(_, failures)| failures)
    }
}

struct Saver<S: SnapshotSource, F: StateFiles, C: Clock> {
    source: S,
    files: F,
    clock: C,
    closed: Arc<AtomicBool>,
    /// 마지막으로 본 번호 — 이것과 다르면 새 변경이다.
    seen: S::Revision,
    /// 디스크에 아직 안 간 것(변경 · 실패한 쓰기)이 생긴 때 — `None` = 쓸 것이 없다.
    pending_since: Option<Instant>,
    last_change_at: Instant,
    resolved: Option<String>,
    write_failures: Streak<FailureKind>,
    copy_failures: Streak<CopyFailure>,
}

impl<S: SnapshotSource, F: StateFiles, C: Clock> Saver<S, F, C> {
    fn new(
        source: S,
        files: F,
        clock: C,
        closed: Arc<AtomicBool>,
        boot_revision: S::Revision,
        carry_resolved: Option<String>,
    ) -> Self {
        let now = clock.now();
        Self {
            source,
            files,
            clock,
            closed,
            seen: boot_revision,
            pending_since: None,
            last_change_at: now,
            resolved: carry_resolved,
            write_failures: Streak::default(),
            copy_failures: Streak::default(),
        }
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    fn tick(&mut self) {
        if self.is_closed() {
            return;
        }
        let now = self.clock.now();
        let revision = self.source.revision();
        if revision != self.seen {
            self.seen = revision;
            self.last_change_at = now;
            self.pending_since.get_or_insert(now);
        }
        let Some(since) = self.pending_since else {
            return;
        };
        if now.duration_since(self.last_change_at) >= QUIET
            || now.duration_since(since) >= MAX_DELAY
        {
            self.save(Cause::Periodic);
        }
    }

    fn resolve(&mut self, hash: String) -> SaveOutcome {
        self.resolved = Some(hash);
        self.save(Cause::Resolve)
    }

    fn save(&mut self, cause: Cause) -> SaveOutcome {
        if self.is_closed() {
            return SaveOutcome::Skipped;
        }
        // 번호를 스냅숏보다 먼저 읽는다 — 그 사이의 변경은 스냅숏에 들고도 번호로는 「안 씀」으로 남아 한 번 더
        // 쓸 뿐이다. 거꾸로면 스냅숏에 없는 변경을 쓴 것으로 친다.
        let revision = self.source.revision();
        let windows = match self.source.snapshot() {
            Ok(windows) => windows,
            Err(reason) => {
                self.failed(cause, Failure::Snapshot(reason), revision);
                return SaveOutcome::Failed;
            }
        };
        let state = StateFile {
            version: STATE_VERSION,
            saved_at_ms: self.clock.wall_ms(),
            clean_exit: matches!(cause, Cause::Final),
            resolved_crash_copy: self.resolved.clone(),
            windows,
        };
        let text = match codec::encode(&state) {
            Ok(text) => text,
            Err(rejected) => {
                self.failed(cause, Failure::Rejected(rejected), revision);
                return SaveOutcome::Failed;
            }
        };
        let closed = &self.closed;
        match self
            .files
            .write_state(&text, &|| closed.load(Ordering::SeqCst))
        {
            Ok(WriteOutcome::Written) => {
                if let Some(failures) = self.write_failures.end() {
                    tracing::info!(
                        module = "state",
                        failures,
                        "상태 파일 쓰기가 실패 뒤 다시 성공했다"
                    );
                }
                match cause {
                    Cause::Periodic => {
                        tracing::debug!(module = "state", bytes = text.len(), "상태 파일을 썼다")
                    }
                    Cause::Resolve | Cause::Final => tracing::info!(
                        module = "state",
                        ?cause,
                        resolved = self.resolved.is_some(),
                        "상태 파일을 썼다"
                    ),
                }
                self.seen = revision;
                self.pending_since = None;
                self.drop_answered_copy();
                SaveOutcome::Written
            }
            Ok(WriteOutcome::Skipped) => {
                tracing::info!(
                    module = "state",
                    ?cause,
                    "기록기가 닫혀 상태 파일을 발행하지 않았다"
                );
                SaveOutcome::Skipped
            }
            Err(error) => {
                self.failed(cause, Failure::Disk(error), revision);
                SaveOutcome::Failed
            }
        }
    }

    /// 다시 쓸 때를 정한다. 스냅숏 · 디스크 실패는 디바운스를 지금부터 다시 재 [`QUIET`] 뒤에 다시 쓴다. 코덱
    /// 거절은 같은 스냅숏이 또 거절되므로 그 번호를 본 것으로 치고 다음 변경을 기다린다. `Final` 이면 스레드가 곧
    /// 끝나 어느 쪽도 오지 않는다.
    fn failed(&mut self, cause: Cause, failure: Failure, revision: S::Revision) {
        match failure {
            Failure::Snapshot(_) | Failure::Disk(_) => {
                let now = self.clock.now();
                self.pending_since = Some(now);
                self.last_change_at = now;
            }
            Failure::Rejected(_) => {
                self.seen = revision;
                self.pending_since = None;
            }
        }
        if !self.write_failures.fail(failure.kind()) {
            tracing::debug!(
                module = "state",
                ?cause,
                failures = self.write_failures.failures(),
                "상태 파일 쓰기가 같은 사유로 또 실패했다: {failure}"
            );
            return;
        }
        match &failure {
            Failure::Snapshot(_) => tracing::error!(
                module = "state",
                ?cause,
                "상태 스냅숏을 못 떴다 — 주기 저장이 다시 뜬다(정상 종료 쓰기면 그대로 끝난다): {failure}"
            ),
            Failure::Rejected(_) => tracing::error!(
                module = "state",
                ?cause,
                "상태 스냅숏을 다음 부팅이 못 읽을 모양이라 쓰지 않았다 — 화면이 바뀌어 풀릴 때까지 메모리에만 있다: {failure}"
            ),
            Failure::Disk(_) => tracing::warn!(
                module = "state",
                ?cause,
                "상태 파일을 못 썼다 — 주기 저장이 다시 쓴다(정상 종료 쓰기면 그대로 끝난다): {failure}"
            ),
        }
    }

    /// N1 — 사본을 다시 읽어 해결 칸의 해시와 같을 때만 지운다. 잠금 없이 진행한 부팅이나 마감 경합에서 새
    /// 인스턴스가 뜬 새 사본을 낡은 기록기가 지우지 않게 한다.
    ///
    /// ★읽기와 지우기 사이는 원자적이지 않다★ — 그 틈에 새 사본이 들어오면 지운다. 막는 것은 셸 실행 잠금(I1)과
    /// 지우기 직전의 닫힘 확인이고, 이 확인은 그 둘이 빠진 길을 좁힐 뿐이다.
    fn drop_answered_copy(&mut self) {
        let Some(hash) = self.resolved.clone() else {
            return;
        };
        let clear = match self.files.read_crash_copy() {
            Ok(text) if codec::crash_copy_hash(&text) == hash => {
                if self.is_closed() {
                    return;
                }
                match self.files.remove_crash_copy() {
                    Ok(()) => {
                        tracing::info!(module = "state", hash = %hash, "답한 크래시 사본을 지웠다");
                        true
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        tracing::info!(module = "state", hash = %hash, "답한 크래시 사본이 이미 없다");
                        true
                    }
                    Err(error) => {
                        self.copy_failed(CopyFailure::Remove, &hash, &error);
                        false
                    }
                }
            }
            Ok(_) => {
                tracing::warn!(
                    module = "state",
                    hash = %hash,
                    "크래시 사본이 답한 것과 다르다 — 새 사본이라 지우지 않는다"
                );
                true
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                tracing::info!(module = "state", hash = %hash, "답한 크래시 사본이 이미 없다");
                true
            }
            // 답한 사본은 상한 안의 UTF-8 이었다 — 그렇지 않은 사본은 다른 사본이다.
            Err(error) if codec::unusable_read(&error).is_some() => {
                tracing::warn!(
                    module = "state",
                    hash = %hash,
                    "크래시 사본이 답한 것과 다르다(못 쓰는 내용) — 지우지 않는다: {error}"
                );
                true
            }
            Err(error) => {
                self.copy_failed(CopyFailure::Read, &hash, &error);
                false
            }
        };
        if clear {
            self.resolved = None;
            if let Some(failures) = self.copy_failures.end() {
                tracing::info!(
                    module = "state",
                    hash = %hash,
                    failures,
                    "답한 크래시 사본 정리가 실패 뒤 끝났다"
                );
            }
        }
    }

    fn copy_failed(&mut self, kind: CopyFailure, hash: &str, error: &io::Error) {
        if !self.copy_failures.fail(kind) {
            tracing::debug!(
                module = "state",
                hash = %hash,
                ?kind,
                failures = self.copy_failures.failures(),
                "답한 크래시 사본 정리가 같은 사유로 또 실패했다: {error}"
            );
            return;
        }
        match kind {
            CopyFailure::Remove => tracing::warn!(
                module = "state",
                hash = %hash,
                "답한 크래시 사본을 못 지웠다 — 다음 성공 쓰기 뒤 다시 지운다: {error}"
            ),
            CopyFailure::Read => tracing::warn!(
                module = "state",
                hash = %hash,
                "크래시 사본을 못 읽어 답한 사본인지 모른다 — 다음 성공 쓰기 뒤 다시 본다: {error}"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::schema::{Bounds, TabStrip, WindowKind};
    use std::sync::atomic::{AtomicU64, AtomicUsize};
    use std::sync::Mutex;

    const COPY: &str = "{\"version\":1}\n";

    #[derive(Clone, Default)]
    struct FakeSource {
        revision: Arc<AtomicU64>,
        oversized: Arc<AtomicBool>,
        fail_snapshot: Arc<AtomicBool>,
        snapshots: Arc<AtomicUsize>,
    }

    impl FakeSource {
        fn change(&self) {
            self.revision.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl SnapshotSource for FakeSource {
        type Revision = u64;

        fn revision(&self) -> u64 {
            self.revision.load(Ordering::SeqCst)
        }

        /// 창 id 에 번호를 실어 어느 때의 스냅숏인지 시험이 읽는다.
        fn snapshot(&self) -> Result<Vec<WindowEntry>, String> {
            self.snapshots.fetch_add(1, Ordering::SeqCst);
            if self.fail_snapshot.load(Ordering::SeqCst) {
                return Err("독 든 락".into());
            }
            let id = if self.oversized.load(Ordering::SeqCst) {
                "x".repeat(STATE_READ_CAP as usize + 1)
            } else {
                format!("rev-{}", self.revision())
            };
            Ok(vec![WindowEntry {
                id,
                kind: WindowKind::Popout(TabStrip {
                    active_tab: uuid::Uuid::nil(),
                    tabs: Vec::new(),
                }),
                theme: None,
                bounds: Some(Bounds {
                    x: 0.0,
                    y: 0.0,
                    w: 800.0,
                    h: 600.0,
                }),
                maximized: false,
            }])
        }
    }

    #[derive(Default)]
    struct Disk {
        published: Vec<StateFile>,
        write_calls: usize,
        fail_writes: bool,
        crash_copy: Option<String>,
        fail_read: Option<io::ErrorKind>,
        fail_remove: bool,
        remove_calls: usize,
        /// 발행 직후 세운다 — rename 을 지난 뒤 닫힘이 선 경합을 흉내 낸다.
        close_after_publish: Option<Arc<AtomicBool>>,
    }

    struct Gate {
        entered: Sender<()>,
        release: Receiver<()>,
    }

    #[derive(Clone, Default)]
    struct FakeFiles {
        disk: Arc<Mutex<Disk>>,
        gate: Arc<Mutex<Option<Gate>>>,
    }

    impl FakeFiles {
        fn with_copy(text: &str) -> Self {
            let files = Self::default();
            files.disk().crash_copy = Some(text.to_owned());
            files
        }

        fn disk(&self) -> std::sync::MutexGuard<'_, Disk> {
            self.disk.lock().unwrap()
        }

        /// 다음 `write_state` 하나를 rename 앞에서 세운다 — 돌려받은 수신자로 들어섰음을 보고, 송신자로 푼다.
        fn arm_gate(&self) -> (Receiver<()>, Sender<()>) {
            let (entered, entered_rx) = mpsc::channel();
            let (release_tx, release) = mpsc::channel();
            *self.gate.lock().unwrap() = Some(Gate { entered, release });
            (entered_rx, release_tx)
        }
    }

    impl StateFiles for FakeFiles {
        fn write_state(&self, text: &str, skip: &dyn Fn() -> bool) -> io::Result<WriteOutcome> {
            let gate = self.gate.lock().unwrap().take();
            if let Some(gate) = gate {
                gate.entered.send(()).unwrap();
                let _ = gate.release.recv();
            }
            let mut disk = self.disk();
            disk.write_calls += 1;
            if disk.fail_writes {
                return Err(io::Error::other("디스크 가득"));
            }
            if skip() {
                return Ok(WriteOutcome::Skipped);
            }
            let (state, _) = codec::decode(text).expect("기록기가 쓴 글은 다시 읽힌다");
            disk.published.push(state);
            if let Some(closed) = &disk.close_after_publish {
                closed.store(true, Ordering::SeqCst);
            }
            Ok(WriteOutcome::Written)
        }

        fn read_crash_copy(&self) -> io::Result<String> {
            let disk = self.disk();
            if let Some(kind) = disk.fail_read {
                return Err(io::Error::from(kind));
            }
            disk.crash_copy
                .clone()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }

        fn remove_crash_copy(&self) -> io::Result<()> {
            let mut disk = self.disk();
            disk.remove_calls += 1;
            if disk.fail_remove {
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            disk.crash_copy
                .take()
                .map(drop)
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }
    }

    #[derive(Clone)]
    struct FakeClock {
        base: Instant,
        elapsed: Arc<Mutex<Duration>>,
    }

    impl FakeClock {
        fn new() -> Self {
            Self {
                base: Instant::now(),
                elapsed: Arc::default(),
            }
        }

        fn advance(&self, by: Duration) {
            *self.elapsed.lock().unwrap() += by;
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.base + *self.elapsed.lock().unwrap()
        }

        fn wall_ms(&self) -> u64 {
            1_759_400_000_000 + self.elapsed.lock().unwrap().as_millis() as u64
        }
    }

    struct Rig {
        saver: Saver<FakeSource, FakeFiles, FakeClock>,
        source: FakeSource,
        files: FakeFiles,
        clock: FakeClock,
    }

    fn rig(files: FakeFiles, carry_resolved: Option<String>) -> Rig {
        let source = FakeSource::default();
        let clock = FakeClock::new();
        let saver = Saver::new(
            source.clone(),
            files.clone(),
            clock.clone(),
            Arc::new(AtomicBool::new(false)),
            0,
            carry_resolved,
        );
        Rig {
            saver,
            source,
            files,
            clock,
        }
    }

    impl Rig {
        /// 운영 스레드처럼 [`POLL_INTERVAL`] 뒤에 한 번 확인한다.
        fn tick(&mut self) {
            self.clock.advance(POLL_INTERVAL);
            self.saver.tick();
        }

        fn ticks(&mut self, count: usize) {
            for _ in 0..count {
                self.tick();
            }
        }

        fn published(&self) -> Vec<StateFile> {
            self.files.disk().published.clone()
        }

        fn last(&self) -> StateFile {
            self.published().pop().expect("발행된 스냅숏이 있다")
        }

        /// 바꾸고 조용히 1초가 지날 때까지 확인한다 — 주기 쓰기 한 번.
        fn change_and_settle(&mut self) {
            self.source.change();
            self.ticks(3);
        }
    }

    fn hash(text: &str) -> String {
        codec::crash_copy_hash(text)
    }

    // ── 주기 저장 ──

    #[test]
    fn nothing_is_written_while_the_revision_stays() {
        let mut rig = rig(FakeFiles::default(), None);
        rig.ticks(40);
        assert_eq!(rig.files.disk().write_calls, 0);
    }

    #[test]
    fn a_change_is_written_after_one_quiet_second() {
        let mut rig = rig(FakeFiles::default(), None);
        rig.source.change();
        rig.tick(); // 0.5초 — 변경을 본다
        rig.tick(); // 1.0초 — 0.5초 조용
        assert_eq!(rig.files.disk().write_calls, 0);
        rig.tick(); // 1.5초 — 1초 조용
        assert_eq!(rig.published().len(), 1);
        let state = rig.last();
        assert_eq!(state.windows[0].id, "rev-1");
        assert!(!state.clean_exit);
        assert_eq!(state.saved_at_ms, 1_759_400_000_000 + 1_500);

        rig.ticks(20);
        assert_eq!(
            rig.files.disk().write_calls,
            1,
            "새 변경이 없으면 다시 쓰지 않는다"
        );
    }

    #[test]
    fn continuous_changes_are_written_five_seconds_after_the_first() {
        let mut rig = rig(FakeFiles::default(), None);
        for _ in 0..10 {
            rig.source.change();
            rig.tick();
        }
        assert_eq!(rig.files.disk().write_calls, 0, "첫 변경(0.5초) 뒤 4.5초");
        rig.source.change();
        rig.tick();
        assert_eq!(rig.published().len(), 1, "첫 변경 뒤 5초");
        assert_eq!(rig.last().windows[0].id, "rev-11");
    }

    #[test]
    fn a_failed_write_is_retried_after_the_quiet_period() {
        let mut rig = rig(FakeFiles::default(), None);
        rig.files.disk().fail_writes = true;
        rig.change_and_settle();
        assert_eq!(rig.files.disk().write_calls, 1);
        rig.files.disk().fail_writes = false;
        rig.tick();
        assert_eq!(
            rig.files.disk().write_calls,
            1,
            "실패 뒤 0.5초 — 아직 조용하지 않다"
        );
        rig.tick();
        assert_eq!(rig.published().len(), 1);
        assert_eq!(rig.last().windows[0].id, "rev-1");
    }

    #[test]
    fn a_rejected_snapshot_is_never_handed_to_the_disk_and_waits_for_a_change() {
        let mut rig = rig(FakeFiles::default(), None);
        rig.source.oversized.store(true, Ordering::SeqCst);
        rig.change_and_settle();
        assert_eq!(rig.source.snapshots.load(Ordering::SeqCst), 1);
        rig.ticks(20);
        assert_eq!(
            rig.source.snapshots.load(Ordering::SeqCst),
            1,
            "같은 스냅숏은 또 거절되므로 다시 뜨지 않는다"
        );
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Failed);
        assert_eq!(rig.files.disk().write_calls, 0);

        rig.source.oversized.store(false, Ordering::SeqCst);
        rig.change_and_settle();
        assert_eq!(
            rig.last().resolved_crash_copy,
            Some(hash(COPY)),
            "거절된 답은 다음 성공 쓰기가 싣는다"
        );
    }

    #[test]
    fn the_handle_can_live_in_shared_app_state() {
        fn shareable<T: Send + Sync + 'static>() {}
        shareable::<SaverHandle>();
    }

    #[test]
    fn a_failed_snapshot_is_retried_like_a_disk_failure() {
        let mut rig = rig(FakeFiles::default(), None);
        rig.source.fail_snapshot.store(true, Ordering::SeqCst);
        rig.change_and_settle();
        assert_eq!(rig.saver.resolve("abc".into()), SaveOutcome::Failed);
        assert_eq!(rig.saver.save(Cause::Final), SaveOutcome::Failed);
        assert_eq!(rig.files.disk().write_calls, 0);

        rig.source.fail_snapshot.store(false, Ordering::SeqCst);
        rig.ticks(2);
        let state = rig.last();
        assert_eq!(state.resolved_crash_copy.as_deref(), Some("abc"));
        assert!(!state.clean_exit);
    }

    #[test]
    fn repeated_failures_fold_per_kind() {
        let mut streak = Streak::default();
        assert!(streak.fail(FailureKind::Disk));
        assert!(!streak.fail(FailureKind::Disk));
        assert!(
            streak.fail(FailureKind::Rejected),
            "갈래가 바뀌면 다시 제 레벨"
        );
        assert!(!streak.fail(FailureKind::Rejected));
        assert!(streak.fail(FailureKind::Disk));
        assert_eq!(streak.end(), Some(5));
        assert_eq!(streak.end(), None);
        assert!(
            streak.fail(FailureKind::Disk),
            "끝난 뒤 첫 실패는 다시 제 레벨"
        );
    }

    #[test]
    fn the_saver_counts_each_failure_kind_into_one_streak() {
        let mut rig = rig(FakeFiles::default(), None);
        rig.files.disk().fail_writes = true;
        rig.change_and_settle();
        rig.ticks(2);
        assert_eq!(
            rig.saver.write_failures.current,
            Some((FailureKind::Disk, 2))
        );
        rig.source.oversized.store(true, Ordering::SeqCst);
        rig.ticks(2);
        assert_eq!(
            rig.saver.write_failures.current,
            Some((FailureKind::Rejected, 3))
        );

        rig.source.oversized.store(false, Ordering::SeqCst);
        rig.files.disk().fail_writes = false;
        rig.change_and_settle();
        assert_eq!(rig.saver.write_failures.current, None);
        assert_eq!(rig.published().len(), 1);
    }

    // ── 해결 칸 ──

    #[test]
    fn resolve_writes_at_once_and_carries_the_hash() {
        let mut rig = rig(FakeFiles::default(), None);
        rig.change_and_settle();
        assert_eq!(
            rig.last().resolved_crash_copy,
            None,
            "꺼내기 전 쓰기엔 해시가 없다"
        );

        assert_eq!(rig.saver.resolve("abc".into()), SaveOutcome::Written);
        let state = rig.last();
        assert_eq!(state.resolved_crash_copy.as_deref(), Some("abc"));
        assert!(!state.clean_exit);
        assert_eq!(rig.published().len(), 2, "번호가 그대로여도 곧바로 쓴다");
    }

    #[test]
    fn the_same_copy_is_removed_and_the_slot_cleared() {
        let mut rig = rig(FakeFiles::with_copy(COPY), None);
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Written);
        assert_eq!(rig.files.disk().crash_copy, None);

        rig.change_and_settle();
        assert_eq!(
            rig.last().resolved_crash_copy,
            None,
            "지운 뒤엔 해시를 싣지 않는다"
        );
        assert_eq!(rig.files.disk().remove_calls, 1);
    }

    #[test]
    fn a_different_copy_is_kept_and_the_slot_cleared() {
        let mut rig = rig(FakeFiles::with_copy("새 인스턴스가 뜬 사본"), None);
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Written);
        assert_eq!(rig.files.disk().remove_calls, 0);
        assert_eq!(
            rig.files.disk().crash_copy.as_deref(),
            Some("새 인스턴스가 뜬 사본")
        );

        rig.change_and_settle();
        assert_eq!(rig.last().resolved_crash_copy, None);
        assert_eq!(rig.files.disk().remove_calls, 0);
    }

    #[test]
    fn a_missing_copy_clears_the_slot() {
        let mut rig = rig(FakeFiles::default(), None);
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Written);
        rig.change_and_settle();
        assert_eq!(rig.last().resolved_crash_copy, None);
        assert_eq!(rig.files.disk().remove_calls, 0);
    }

    #[test]
    fn an_unusable_copy_is_not_the_answered_one() {
        let mut rig = rig(FakeFiles::with_copy(COPY), None);
        rig.files.disk().fail_read = Some(io::ErrorKind::InvalidData);
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Written);
        rig.files.disk().fail_read = None;
        rig.change_and_settle();
        assert_eq!(rig.last().resolved_crash_copy, None);
        assert_eq!(rig.files.disk().remove_calls, 0);
    }

    #[test]
    fn a_failed_remove_keeps_the_slot_and_retries_after_each_write() {
        let mut rig = rig(FakeFiles::with_copy(COPY), None);
        rig.files.disk().fail_remove = true;
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Written);
        assert_eq!(rig.files.disk().remove_calls, 1);

        rig.ticks(20);
        assert_eq!(
            rig.files.disk().remove_calls,
            1,
            "쓰기 없이는 다시 지우지 않는다"
        );

        rig.change_and_settle();
        assert_eq!(
            rig.last().resolved_crash_copy,
            Some(hash(COPY)),
            "못 지운 동안은 해시를 싣는다"
        );
        assert_eq!(rig.files.disk().remove_calls, 2);

        rig.files.disk().fail_remove = false;
        rig.change_and_settle();
        assert_eq!(rig.files.disk().remove_calls, 3);
        assert_eq!(rig.files.disk().crash_copy, None);

        rig.change_and_settle();
        assert_eq!(rig.last().resolved_crash_copy, None);
        assert_eq!(rig.files.disk().remove_calls, 3);
    }

    #[test]
    fn an_unreadable_copy_keeps_the_slot_until_a_later_write() {
        let mut rig = rig(FakeFiles::with_copy(COPY), None);
        rig.files.disk().fail_read = Some(io::ErrorKind::PermissionDenied);
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Written);
        assert_eq!(rig.files.disk().remove_calls, 0);

        rig.files.disk().fail_read = None;
        rig.change_and_settle();
        assert_eq!(rig.last().resolved_crash_copy, Some(hash(COPY)));
        assert_eq!(rig.files.disk().crash_copy, None);
    }

    #[test]
    fn a_failed_resolve_write_keeps_the_slot_for_the_next_write() {
        let mut rig = rig(FakeFiles::with_copy(COPY), None);
        rig.files.disk().fail_writes = true;
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Failed);
        assert_eq!(rig.files.disk().remove_calls, 0);

        rig.files.disk().fail_writes = false;
        rig.ticks(2);
        assert_eq!(
            rig.published().len(),
            1,
            "번호가 그대로여도 실패한 답을 다시 쓴다"
        );
        assert_eq!(rig.last().resolved_crash_copy, Some(hash(COPY)));
        assert_eq!(rig.files.disk().crash_copy, None);
    }

    #[test]
    fn a_hash_carried_from_boot_is_written_and_its_copy_removed() {
        let mut rig = rig(FakeFiles::with_copy(COPY), Some(hash(COPY)));
        rig.change_and_settle();
        assert_eq!(rig.last().resolved_crash_copy, Some(hash(COPY)));
        assert_eq!(rig.files.disk().crash_copy, None);
    }

    // ── 정상 종료 · 닫힘 ──

    #[test]
    fn final_writes_a_clean_exit_and_leaves_an_unanswered_copy() {
        let mut rig = rig(FakeFiles::with_copy(COPY), None);
        assert_eq!(rig.saver.save(Cause::Final), SaveOutcome::Written);
        let state = rig.last();
        assert!(state.clean_exit);
        assert_eq!(state.resolved_crash_copy, None);
        assert_eq!(
            rig.files.disk().remove_calls,
            0,
            "D6 — 정상 종료는 사본을 지우지 않는다"
        );
        assert_eq!(rig.files.disk().crash_copy.as_deref(), Some(COPY));
    }

    #[test]
    fn final_carries_the_slot_and_retries_the_remove() {
        let mut rig = rig(FakeFiles::with_copy(COPY), Some(hash(COPY)));
        assert_eq!(rig.saver.save(Cause::Final), SaveOutcome::Written);
        let state = rig.last();
        assert!(state.clean_exit);
        assert_eq!(state.resolved_crash_copy, Some(hash(COPY)));
        assert_eq!(rig.files.disk().crash_copy, None);
    }

    #[test]
    fn once_closed_nothing_touches_the_disk() {
        let mut rig = rig(FakeFiles::with_copy(COPY), None);
        rig.saver.closed.store(true, Ordering::SeqCst);
        rig.change_and_settle();
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Skipped);
        assert_eq!(rig.saver.save(Cause::Final), SaveOutcome::Skipped);
        let disk = rig.files.disk();
        assert_eq!((disk.write_calls, disk.remove_calls), (0, 0));
    }

    #[test]
    fn closing_between_the_write_and_the_delete_keeps_the_copy() {
        let mut rig = rig(FakeFiles::with_copy(COPY), None);
        rig.files.disk().close_after_publish = Some(Arc::clone(&rig.saver.closed));
        assert_eq!(rig.saver.resolve(hash(COPY)), SaveOutcome::Written);
        let disk = rig.files.disk();
        assert_eq!(disk.remove_calls, 0);
        assert_eq!(disk.crash_copy.as_deref(), Some(COPY));
    }

    // ── 스레드 ──

    fn started(
        files: FakeFiles,
        carry_resolved: Option<String>,
        poll: Duration,
    ) -> (SaverHandle, JoinHandle<()>, FakeSource, FakeClock) {
        let source = FakeSource::default();
        let clock = FakeClock::new();
        let saver = Saver::new(
            source.clone(),
            files,
            clock.clone(),
            Arc::new(AtomicBool::new(false)),
            0,
            carry_resolved,
        );
        let (handle, thread) = start(saver, poll).unwrap();
        (handle, thread, source, clock)
    }

    #[test]
    fn finish_writes_a_clean_exit_and_ends_the_thread() {
        let files = FakeFiles::default();
        let (handle, thread, _, _) = started(files.clone(), None, POLL_INTERVAL);
        assert_eq!(
            handle.finish(REPLY_DEADLINE),
            RequestOutcome::Done(SaveOutcome::Written)
        );
        thread.join().unwrap();
        assert!(files.disk().published.last().unwrap().clean_exit);
        assert_eq!(
            handle.resolve(hash(COPY), REPLY_DEADLINE),
            RequestOutcome::SaverGone
        );
        assert_eq!(handle.finish(REPLY_DEADLINE), RequestOutcome::SaverGone);
    }

    #[test]
    fn a_finish_past_its_deadline_closes_and_the_stuck_write_never_publishes() {
        let files = FakeFiles::with_copy(COPY);
        let (entered, release) = files.arm_gate();
        let (handle, thread, _, _) = started(files.clone(), None, POLL_INTERVAL);

        // 사본을 지울 답을 쓰다가 rename 앞에 갇힌 채로 종료를 맞는다.
        let stuck = handle
            .send(|reply| Request::Resolve {
                hash: hash(COPY),
                reply,
            })
            .unwrap();
        entered.recv().unwrap();
        assert_eq!(
            handle.finish(Duration::from_millis(50)),
            RequestOutcome::TimedOut
        );
        assert!(handle.closed.load(Ordering::SeqCst));
        release.send(()).unwrap();
        assert_eq!(
            wait(stuck, REPLY_DEADLINE),
            RequestOutcome::Done(SaveOutcome::Skipped)
        );
        thread.join().unwrap();

        let disk = files.disk();
        assert_eq!(
            disk.write_calls, 1,
            "닫힌 뒤 꺼낸 Final 은 디스크를 건드리지 않는다"
        );
        assert!(
            disk.published.is_empty(),
            "풀려난 rename 은 발행하지 않는다"
        );
        assert_eq!(disk.remove_calls, 0);
        assert_eq!(disk.crash_copy.as_deref(), Some(COPY));
    }

    #[test]
    fn dropping_every_handle_ends_the_thread_without_writing() {
        let files = FakeFiles::default();
        let (handle, thread, _, _) = started(files.clone(), None, POLL_INTERVAL);
        let copy = handle.clone();
        drop(handle);
        drop(copy);
        thread.join().unwrap();
        assert_eq!(files.disk().write_calls, 0);
    }

    #[test]
    fn a_resolve_still_queued_is_not_in_the_write_in_progress() {
        let files = FakeFiles::default();
        let (entered, release) = files.arm_gate();
        let (handle, thread, source, clock) =
            started(files.clone(), None, Duration::from_millis(1));

        source.change();
        let mut attempts = 0;
        while entered.recv_timeout(Duration::from_millis(20)).is_err() {
            attempts += 1;
            assert!(attempts < 500, "주기 쓰기가 시작되지 않았다");
            clock.advance(QUIET);
        }
        // 주기 쓰기가 rename 앞에 서 있는 동안 보낸다 — 아직 꺼내지 않았다.
        let answer = handle
            .send(|reply| Request::Resolve {
                hash: "abc".into(),
                reply,
            })
            .unwrap();
        release.send(()).unwrap();
        assert_eq!(
            wait(answer, REPLY_DEADLINE),
            RequestOutcome::Done(SaveOutcome::Written)
        );

        let published = files.disk().published.clone();
        assert_eq!(published.len(), 2);
        assert_eq!(published[0].resolved_crash_copy, None);
        assert_eq!(published[1].resolved_crash_copy.as_deref(), Some("abc"));

        assert_eq!(
            handle.finish(REPLY_DEADLINE),
            RequestOutcome::Done(SaveOutcome::Written)
        );
        thread.join().unwrap();
    }

    // ── 운영 파일 ──

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "engram-saver-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("시계")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn fs_writes_reads_and_removes_the_real_files() {
        let dir = temp_dir("fs");
        let state_path = dir.join("state.json");
        let crash_path = dir.join("state.crash.json");
        let fs = Fs::new(state_path.clone(), crash_path.clone());

        assert_eq!(
            fs.write_state("첫 글", &|| false).unwrap(),
            WriteOutcome::Written
        );
        assert_eq!(
            fs.write_state("닫힌 뒤 글", &|| true).unwrap(),
            WriteOutcome::Skipped
        );
        assert_eq!(std::fs::read_to_string(&state_path).unwrap(), "첫 글");

        assert_eq!(
            fs.read_crash_copy().unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        std::fs::write(&crash_path, COPY).unwrap();
        assert_eq!(fs.read_crash_copy().unwrap(), COPY);
        fs.remove_crash_copy().unwrap();
        assert!(!crash_path.exists());
        assert_eq!(
            fs.remove_crash_copy().unwrap_err().kind(),
            io::ErrorKind::NotFound
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
