//! AgentBackend — 백엔드별 명령 명세 산출 trait + 자유 함수 dispatch.
//!
//! transport(PtyTransport)는 claude/codex를 모른다. 누가 어떤 프로그램인지 아는 곳은
//! 오직 backend/다. manager(stage 6)가 이 dispatch 함수를 호출해 CommandSpec을 받아
//! PtyTransport에 주입한다.
//!
//! tauri import 0.

pub mod claude;
pub mod codex;
pub mod gemini;
pub mod shell;

pub use claude::ClaudeBackend;
pub use codex::CodexBackend;
pub use gemini::GeminiBackend;
pub use shell::ShellBackend;

use std::path::PathBuf;

use uuid::Uuid;

use crate::pty::profile::{AgentCommand, SpawnMode};
use crate::pty::types::CommandSpec;

/// 백엔드별 명령 명세 산출 인터페이스.
/// unit struct로 구현되어 &'static으로 사용된다 — 상태 없음.
pub trait AgentBackend: Send + Sync {
    /// 이 백엔드가 claude 세션 추적 대상인가.
    /// true면 manager(stage 6)가 sid를 발급·watcher를 부착한다.
    fn needs_session(&self) -> bool;

    /// 프로필 + 모드 → CommandSpec.
    /// cwd·env는 manager가 정규화한 값을 전달한다(stage 6에서 주입 예정).
    fn build_spec(
        &self,
        command: &AgentCommand,
        mode: SpawnMode,
        session_id: Option<Uuid>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
    ) -> CommandSpec;
}

// ── 정적 싱글턴 ────────────────────────────────────────────────────────────────

static CLAUDE_BACKEND: ClaudeBackend = ClaudeBackend;
static SHELL_BACKEND: ShellBackend = ShellBackend;

fn backend_for(c: &AgentCommand) -> &'static dyn AgentBackend {
    match c {
        AgentCommand::Claude { .. } => &CLAUDE_BACKEND,
        AgentCommand::Shell { .. } => &SHELL_BACKEND,
    }
}

// ── 자유 함수 dispatch ─────────────────────────────────────────────────────────

/// 이 명령이 claude 세션 추적 대상인가.
pub fn needs_session(c: &AgentCommand) -> bool {
    backend_for(c).needs_session()
}

/// 프로필 → CommandSpec. manager가 stage 6에서 호출한다.
pub fn build_command_spec(
    c: &AgentCommand,
    mode: SpawnMode,
    session_id: Option<Uuid>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
) -> CommandSpec {
    backend_for(c).build_spec(c, mode, session_id, cwd, env)
}
