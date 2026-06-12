use std::path::PathBuf;

use tauri::State;
use uuid::Uuid;

use crate::pty::manager::default_shell;
use crate::pty::profile::{AgentCommand, AgentProfile, SpawnMode};
use crate::pty::types::AgentInfo;
use crate::AppState;

/// 기존 thin spawn — cwd만 받아 기본 셸 에이전트를 띄운다(transient, auto_restore=false).
/// claude 프로필 CRUD/복원은 별도 커맨드(S9-6)에서 다룬다.
#[tauri::command]
pub async fn spawn_agent(state: State<'_, AppState>, cwd: String) -> Result<AgentInfo, String> {
    let profile = AgentProfile::new(
        cwd.clone(),
        AgentCommand::Shell {
            program: default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from(&cwd),
        vec![],
        false,
    );
    state
        .manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kill_agent(state: State<'_, AppState>, agent_id: String) -> Result<(), String> {
    let id = Uuid::parse_str(&agent_id).map_err(|e| e.to_string())?;
    state.manager.kill_agent(id).map_err(|e| e.to_string())
}

/// 진행 중 작업만 중단(≠kill). PTY=0x03 주입. 프로세스는 살아 있다.
#[tauri::command]
pub async fn interrupt_agent(state: State<'_, AppState>, agent_id: String) -> Result<(), String> {
    let id = Uuid::parse_str(&agent_id).map_err(|e| e.to_string())?;
    state.manager.interrupt(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_agents(state: State<'_, AppState>) -> Result<Vec<AgentInfo>, String> {
    Ok(state.manager.list_agents())
}
