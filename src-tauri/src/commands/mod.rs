// ADR-0020 Stage 4a: 옛 개별 command(agent/pty/profile)는 agent_command(embedded_carrier)가
// AgentCommand 전 variant 를 처리하므로 삭제됨. discovery(비-에이전트, daemon 모드 부팅)만 잔류.
pub mod discovery;

pub use discovery::*;
