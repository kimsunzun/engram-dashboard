use std::path::Path;

use tauri::State;
use uuid::Uuid;

use crate::pty::types::AgentInfo;
use crate::AppState;

#[tauri::command]
pub async fn spawn_agent(state: State<'_, AppState>, cwd: String) -> Result<AgentInfo, String> {
    state
        .manager
        .spawn_agent(Path::new(&cwd))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kill_agent(state: State<'_, AppState>, agent_id: String) -> Result<(), String> {
    let id = Uuid::parse_str(&agent_id).map_err(|e| e.to_string())?;
    state.manager.kill_agent(id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_agents(state: State<'_, AppState>) -> Result<Vec<AgentInfo>, String> {
    Ok(state.manager.list_agents())
}
