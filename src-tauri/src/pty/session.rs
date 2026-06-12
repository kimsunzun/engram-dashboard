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

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use base64::Engine as _;
use portable_pty::{Child, MasterPty};

use crate::pty::output_core::{OutputCore, ReplayBuffer};
use crate::pty::transport::AgentTransport;
use crate::pty::types::{
    AgentId, AgentStatus, Capabilities, InputEvent, OutputChunk, OutputSink, PtyError, PtyEvent,
    SinkId,
};

#[cfg(windows)]
use crate::pty::platform::JobObjectHandle;

/// 에이전트 1개에 대응하는 PTY 세션. 필드별 독립 Mutex(상단 모듈 주석 참조).
pub struct PtySession {
    // ── 불변 (생성 후 변경 없음) ──────────────────────────────
    pub id: AgentId,
    pub cwd: PathBuf,
    /// 이 세션 인스턴스가 spawn된 시점의 epoch. 재spawn마다 새 PtySession이 새 epoch로 생성되어
    /// 프론트 재구독 트리거가 된다(S9 §18-a). 세션 단위 불변값.
    pub epoch: u32,

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
    pub epoch: u32,
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
            epoch: init.epoch,
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

// ReplayBuffer 는 output_core.rs 로 이동(장기 소속). PtySession.replay가 stage 3까지 이걸 import.

// ─────────────────────────────────────────────────────────────────────────────
// AgentSession (stage 5) — OutputCore(출력 측) + Box<dyn AgentTransport>(채널/자원 측) 합성.
//
// 왜 PtySession과 병존하는가: manager는 아직 PtySession을 쓴다(stage 6에서 전환). 이 단계에선
// AgentSession을 독립 합성 struct로 완성하고 session_smoke로 실측만 한다 — manager/PtySession 불변.
//
// 소유권 분할(impl-spec 표): AgentSession은 id/cwd/epoch/cols/rows + core(Arc) + transport(Box)만 든다.
//   - master/child/shutdown/job/reader/writer → transport(PtyTransport) 안.
//   - subscribers/replay/seq/status/finalized → core(OutputCore) 안.
// 따라서 모든 메서드는 자기 필드(cols/rows atomic)를 만지거나 core/transport로 위임할 뿐이다.
// ─────────────────────────────────────────────────────────────────────────────

/// 에이전트 1개 = 출력 측(core) + 채널/자원 측(transport)의 합성. transport 종류(PTY/API)와
/// 무관한 공용 표면을 노출하고, 내부에서 core/transport로 위임한다.
pub struct AgentSession {
    pub id: AgentId,
    pub cwd: PathBuf,
    pub epoch: u32,
    /// 현 터미널 폭/높이. resize 성공 시에만 갱신(실패 시 옛 값 유지) — manager.agent_info가 직접 load.
    pub cols: AtomicU16,
    pub rows: AtomicU16,
    core: Arc<OutputCore>,
    transport: Box<dyn AgentTransport>,
}

impl AgentSession {
    /// 합성 세션 생성. **start는 여기서 호출하지 않는다** — manager가 new 이전에
    /// `transport.start(core.clone())`를 직접 부른다(impl-spec: 테스트 가시성·spawn 흐름 명시성).
    /// 즉 이 생성자는 이미 start된 transport와 core를 받아 묶기만 한다.
    pub fn new(
        id: AgentId,
        cwd: PathBuf,
        epoch: u32,
        cols: u16,
        rows: u16,
        core: Arc<OutputCore>,
        transport: Box<dyn AgentTransport>,
    ) -> Self {
        Self {
            id,
            cwd,
            epoch,
            cols: AtomicU16::new(cols),
            rows: AtomicU16::new(rows),
            core,
            transport,
        }
    }

    /// 입력 바이트 전달 → transport(PTY=writer). 콘솔은 Raw variant.
    pub fn write_input(&self, bytes: &[u8]) -> Result<(), PtyError> {
        self.transport.send_input(InputEvent::Raw(bytes.to_vec()))
    }

    /// 터미널 크기 변경. transport.resize 성공 후에만 cols/rows atomic 갱신(? 연산자로 실패 시 옛 값 유지).
    /// 현 manager.resize의 atomic 저장 책임이 여기로 이관.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        self.transport.resize(cols, rows)?;
        self.cols.store(cols, Ordering::Relaxed);
        self.rows.store(rows, Ordering::Relaxed);
        Ok(())
    }

    /// 진행 중 작업만 중단(≠kill). PTY=0x03 주입. 프로세스는 살아 있다.
    pub fn interrupt(&self) -> Result<(), PtyError> {
        self.transport.interrupt()
    }

    /// 자원 강제 종료 + pump 종료 대기. **이 2동사 순서(shutdown THEN join_pump)가 kill 인과의 핵심.**
    /// shutdown이 master를 drop해 pump read를 EOF로 깨우고(→core.finish(Killed)), join_pump가
    /// 그 pump 종료를 기다린다. 역전 시 hang(아직 살아있는 pump를 기다림).
    pub fn kill(&self, timeout: Duration) {
        self.transport.shutdown();
        self.core.join_pump(timeout);
    }

    /// 과도기 Exiting 전이 — kill 직전 manager가 먼저 호출(stage 6). core로 위임.
    /// terminal(이미 종료)이면 false. enter_exiting과 kill은 별개 동사다.
    pub fn enter_exiting(&self) -> bool {
        self.core.enter_exiting()
    }

    /// 이 transport가 지원하는 영역별 capability.
    pub fn capabilities(&self) -> Capabilities {
        self.transport.capabilities()
    }

    /// 구독자 등록 → core. SinkId 반환(unsubscribe용).
    pub fn subscribe(&self, sink: Arc<dyn OutputSink>) -> SinkId {
        self.core.subscribe(sink)
    }

    /// 구독 해제 → core.
    pub fn unsubscribe(&self, sink_id: SinkId) {
        self.core.unsubscribe(sink_id);
    }

    /// replay 스냅샷 → core. 늦게 붙는 창 초기 복원용.
    pub fn snapshot(&self) -> Vec<OutputChunk> {
        self.core.snapshot()
    }

    /// 현재 상태 → core.
    pub fn status(&self) -> AgentStatus {
        self.core.status()
    }

    /// 현 cols/rows 게터(pub atomic 직접 load도 가능 — manager.agent_info 편의).
    pub fn cols(&self) -> u16 {
        self.cols.load(Ordering::Relaxed)
    }

    pub fn rows(&self) -> u16 {
        self.rows.load(Ordering::Relaxed)
    }
}
