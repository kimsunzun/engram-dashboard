//! AgentTransport — 에이전트 1개의 데이터 채널 + 자원 제어 seam.
//!
//! transport는 바이트·이벤트를 만들어 `OutputCore::emit`/`finish`로 넘기기만 하면 된다.
//! 출력 fanout·종료 전이·구독 같은 공용 로직은 OutputCore가 담당하고, transport는
//! 자기 자원(PTY master/child/job 혹은 HTTP 스트림)의 수명만 책임진다.
//!
//! 콘솔(claude/codex/gemini)은 PtyTransport 한 벌을 공유하고, API는 ApiTransport(stage 7).
//!
//! tauri import 0.

use std::sync::Arc;

use crate::pty::output_core::OutputCore;
use crate::pty::types::{Capabilities, InputEvent, PtyError};

pub mod pty;

/// 에이전트 백엔드(PTY/API)를 추상화하는 seam. AgentSession이 `Box<dyn AgentTransport>`로 보유.
///
/// 제어 동사: start · send_input · resize · interrupt · shutdown · capabilities.
/// (reconfigure/graceful_shutdown은 API 도입 때 단계적으로 추가.)
pub trait AgentTransport: Send + Sync {
    /// 출력 pump/stream 기동 → core 연결. spawn 직후 1회 호출.
    /// PtyTransport: 보관해둔 reader를 take해 pump 스레드 spawn, core.attach_pump 호출.
    fn start(&self, core: Arc<OutputCore>);

    /// 입력 이벤트 전달. PTY=Raw 바이트를 writer로 흘려보낸다.
    fn send_input(&self, input: InputEvent) -> Result<(), PtyError>;

    /// 터미널 크기 변경. cols/rows의 보존(atomic 저장)은 AgentSession 책임.
    fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError>;

    /// ≠kill. 진행 중 작업만 중단(PTY=0x03 주입 / API=cancel). 프로세스는 살아 있다.
    fn interrupt(&self) -> Result<(), PtyError>;

    /// 자원 강제 종료(멱등). PtyTransport: shutdown flag set → child.kill+wait → job.terminate
    /// → master.take(drop). 반환 전 master drop 보장(pump read가 EOF로 깸).
    /// pump 종료 대기는 여기서 안 함(core.join_pump 몫).
    fn shutdown(&self);

    /// 이 transport가 지원하는 영역별 capability.
    fn capabilities(&self) -> Capabilities;
}
