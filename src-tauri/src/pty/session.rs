//! PtySession — 에이전트 1개당 PTY 자료구조 + 구독(subscribe/unsubscribe).
//!
//! 이 파일은 자료구조와 구독 등록만 담당한다. drain thread 로직(drain.rs)과
//! spawn/kill(manager.rs)은 여기 없다.
//!
//! tauri import는 0개여야 한다(JobObjectHandle은 crate::pty::platform 경유).
//!
//! **왜 PtySession 내부를 필드별 별도 Mutex로 분리하는가 (LLD §4):**
//! drain thread가 replay/subscribers lock만 잠그는 동안 write_stdin은 writer lock만
//! 잠글 수 있어 교착 없이 병행 가능하다. 전체 세션을 단일 Mutex로 묶으면 drain 중
//! stdin이 막히고 stdin 중 drain이 막힌다.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use base64::Engine as _;
use portable_pty::{Child, MasterPty};

use crate::pty::types::{AgentId, AgentStatus, OutputSink, PtyChunk, PtyEvent, SinkId};

#[cfg(windows)]
use crate::pty::platform::JobObjectHandle;

/// 에이전트 1개에 대응하는 PTY 세션. 필드별 독립 Mutex(상단 모듈 주석 참조).
pub struct PtySession {
    // ── 불변 (생성 후 변경 없음) ──────────────────────────────
    pub id: AgentId,
    pub cwd: PathBuf,

    // ── PTY I/O (각각 독립 lock) ──────────────────────────────
    // master: Option 필수 — kill 시 take()로 drop → ConPTY 종료 → reader EOF (spike 검증).
    pub master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    pub writer: Mutex<Box<dyn Write + Send>>,
    pub child: Mutex<Box<dyn Child + Send + Sync>>,

    // ── 상태 (독립 lock / atomic) ─────────────────────────────
    pub status: Mutex<AgentStatus>,
    pub cols: AtomicU16,
    pub rows: AtomicU16,

    // ── 출력 구독 (독립 lock) ─────────────────────────────────
    pub subscribers: Mutex<Vec<Arc<dyn OutputSink>>>,

    // ── Replay buffer (독립 lock) ─────────────────────────────
    pub replay: Mutex<ReplayBuffer>,

    // ── drain thread 제어 ─────────────────────────────────────
    pub seq: AtomicU64,
    pub shutdown: AtomicBool,
    pub drain_handle: Mutex<Option<JoinHandle<()>>>,
    pub drain_done_rx: Mutex<Option<Receiver<()>>>,

    // ── Windows 전용 ──────────────────────────────────────────
    #[cfg(windows)]
    pub job_handle: JobObjectHandle,
}

/// PtySession::new 생성 인자 — manager의 spawn_agent가 spawn 직후 채워 넘긴다.
/// drain_handle/drain_done_rx는 세션을 Arc로 감싼 뒤 drain thread를 띄우며
/// 사후에 채우므로 여기 포함하지 않는다(생성 시 None).
pub struct PtySessionInit {
    pub id: AgentId,
    pub cwd: PathBuf,
    pub master: Box<dyn MasterPty + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn Child + Send + Sync>,
    pub cols: u16,
    pub rows: u16,
    #[cfg(windows)]
    pub job_handle: JobObjectHandle,
}

impl PtySession {
    /// 새 세션 생성. status는 Running, seq 0, 구독자/replay 비어있는 상태로 시작한다.
    /// drain_handle/drain_done_rx는 None — manager가 drain thread 기동 후 채운다.
    pub fn new(init: PtySessionInit) -> Self {
        Self {
            id: init.id,
            cwd: init.cwd,
            master: Mutex::new(Some(init.master)),
            writer: Mutex::new(init.writer),
            child: Mutex::new(init.child),
            status: Mutex::new(AgentStatus::Running),
            cols: AtomicU16::new(init.cols),
            rows: AtomicU16::new(init.rows),
            subscribers: Mutex::new(Vec::new()),
            replay: Mutex::new(ReplayBuffer::new()),
            seq: AtomicU64::new(0),
            shutdown: AtomicBool::new(false),
            drain_handle: Mutex::new(None),
            drain_done_rx: Mutex::new(None),
            #[cfg(windows)]
            job_handle: init.job_handle,
        }
    }

    /// 구독자 등록 + replay 전송. SinkId 반환(unsubscribe용).
    ///
    /// **C4 (LLD §7, 절대 준수):** subscribers lock을 보유한 채로 replay를 전송한다.
    /// 이렇게 하면 drain thread의 live send와 이 replay send가 같은 subscribers lock으로
    /// 직렬화되어 replay→live 순서 역전이 원천 차단된다. drain은 step 5에서 subscribers
    /// lock을 잡으려다 잠깐 대기하지만, replay 전송은 일회성이라 허용된다.
    ///
    /// **락 순서 규칙 3 예외 (LLD §10):** subscribe 함수만 subscribers→replay 두 lock을
    /// 동시에 취득한다(항상 이 순서). drain thread는 두 lock 동시 보유 절대 금지.
    pub fn subscribe(&self, sink: Arc<dyn OutputSink>) -> SinkId {
        let sink_id = sink.sink_id();

        // (C4) subscribers lock 보유 시작 — drop 전까지 drain의 live send와 직렬화된다.
        let mut subscribers_guard = self.subscribers.lock().expect("subscribers poisoned");

        // (A) live 구독을 먼저 등록 → 이후 도착하는 live chunk는 이 sink에도 전달됨.
        subscribers_guard.push(sink.clone());

        // (B) subscribers 보유 중 replay 스냅샷 취득 (규칙 3의 유일한 허용 예외).
        let snapshot = {
            let replay_guard = self.replay.lock().expect("replay poisoned");
            replay_guard.snapshot()
        };

        // replay 전송 — snapshot의 seq와 이후 live chunk의 seq가 끊기지 않아 프론트가
        // seq로 dedup/정렬 가능. 막 등록된 sink라 send 실패는 unlikely → 무시(§7).
        for chunk in snapshot {
            let event = PtyEvent {
                agent_id: self.id,
                seq: chunk.seq,
                data_b64: base64::engine::general_purpose::STANDARD.encode(&chunk.data),
            };
            let _ = sink.send(event);
        }

        // lock 해제 → drain 재개. (명시적 drop으로 lock 보유 구간을 분명히 표시)
        drop(subscribers_guard);

        sink_id
    }

    /// 구독 해제 (창 닫힘 시 cleanup에서 호출). 해당 sink_id만 제거.
    pub fn unsubscribe(&self, sink_id: SinkId) {
        self.subscribers
            .lock()
            .expect("subscribers poisoned")
            .retain(|s| s.sink_id() != sink_id);
    }
}

/// 늦게 붙는 창을 위한 PTY 출력 ring buffer — 상한 2MB, 초과 시 앞부터 제거.
/// (types.rs에서 이동 — LLD §1/§4가 session.rs 소속으로 명시)
pub struct ReplayBuffer {
    chunks: VecDeque<PtyChunk>,
    total_bytes: usize,
    max_bytes: usize,
}

impl ReplayBuffer {
    pub fn new() -> Self {
        Self {
            chunks: VecDeque::new(),
            total_bytes: 0,
            max_bytes: 2 * 1024 * 1024,
        }
    }

    pub fn push(&mut self, chunk: PtyChunk) {
        self.total_bytes += chunk.data.len();
        self.chunks.push_back(chunk);
        while self.total_bytes > self.max_bytes {
            if let Some(oldest) = self.chunks.pop_front() {
                self.total_bytes -= oldest.data.len();
            } else {
                break;
            }
        }
    }

    pub fn snapshot(&self) -> Vec<PtyChunk> {
        self.chunks.iter().cloned().collect()
    }
}

impl Default for ReplayBuffer {
    fn default() -> Self {
        Self::new()
    }
}
