//! AgentSession — 에이전트 1개 = OutputCore(출력 측) + Box<dyn AgentTransport>(채널/자원 측) 합성.
//!
//! transport 종류(PTY/API)와 무관한 공용 표면을 노출하고, 내부에서 core/transport로 위임한다.
//!
//! 소유권 분할(impl-spec 표): AgentSession은 id/cwd/epoch/cols/rows + core(Arc) + transport(Box)만 든다.
//!   - master/child/shutdown/job/reader/writer → transport(PtyTransport) 안.
//!   - subscribers/replay/seq/status/finalized → core(OutputCore) 안.
//!
//! 따라서 모든 메서드는 자기 필드(cols/rows atomic)를 만지거나 core/transport로 위임할 뿐이다.
//!
//! tauri import 0.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::pty::output_core::OutputCore;
use crate::pty::transport::AgentTransport;
use crate::pty::types::{
    AgentId, AgentStatus, Capabilities, InputEvent, OutputChunk, OutputSink, PtyError, SinkId,
};

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
