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

use crate::agent::profile::{AgentCommand, SpawnMode};
use crate::agent::types::CommandSpec;

/// 콘솔 CLI(claude/codex/gemini 등 npm 설치형)를 플랫폼에서 실행 가능한 (program, args)로 변환.
///
/// **왜 필요한가:** Windows에서 `claude`는 확장자 없는 npm shim이라, ConPTY가 쓰는 CreateProcessW가
/// 직접 못 띄운다(error 193 — PATHEXT/셸 해석을 안 함). `cmd.exe /c <prog> …`로 감싸면 cmd가
/// `<prog>.cmd` shim을 해석해 실제 프로세스를 띄운다. `cmd /c`는 대상이 종료되면 함께 종료되므로
/// "PTY 자식 = 에이전트" 수명이 유지된다(JobObject가 트리 통째 kill). 비Windows는 그대로 직접 실행.
///
/// shim이 아닌 일반 실행파일(Shell의 cmd.exe 등)에는 적용하지 않는다 — CLI 백엔드 전용.
pub(crate) fn console_command(program: &str, args: Vec<String>) -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        let mut wrapped = Vec::with_capacity(args.len() + 2);
        wrapped.push("/c".to_string());
        wrapped.push(program.to_string());
        wrapped.extend(args);
        ("cmd.exe".to_string(), wrapped)
    }
    #[cfg(not(windows))]
    {
        (program.to_string(), args)
    }
}

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
