//! 프로필 CRUD + 프로필 기반 spawn 커맨드 (S9-6).
//!
//! Tauri thin wrapper — 비즈니스 로직 없음. ProfileRegistry/AgentManager 호출만.
//! 자격증명은 env에 넣지 말 것(평문 persist — persistence가 경고).

use std::path::PathBuf;

use tauri::State;
use uuid::Uuid;

use crate::AppState;
use engram_dashboard_core::agent::profile::{AgentCommand, AgentProfile, SpawnMode};
use engram_dashboard_core::agent::types::AgentInfo;

/// 저장된 프로필 전체 조회.
#[tauri::command]
pub async fn list_profiles(state: State<'_, AppState>) -> Result<Vec<AgentProfile>, String> {
    Ok(state.manager.profiles().list())
}

/// claude 프로필 생성(스폰하지 않음 — 등록·persist만). 이후 `spawn_profile`로 띄운다.
#[tauri::command]
pub async fn create_claude_profile(
    state: State<'_, AppState>,
    name: String,
    cwd: String,
    extra_args: Vec<String>,
    env: Vec<(String, String)>,
    auto_restore: bool,
) -> Result<AgentProfile, String> {
    let profile = AgentProfile::new(
        name,
        AgentCommand::Claude { extra_args },
        PathBuf::from(cwd),
        env,
        auto_restore,
    );
    state.manager.profiles().upsert(profile.clone());
    Ok(profile)
}

/// 프로필 삭제(persist). 실행 중인 세션은 별도 `kill_agent`로 종료할 것.
#[tauri::command]
pub async fn delete_profile(state: State<'_, AppState>, agent_id: String) -> Result<(), String> {
    let id = Uuid::parse_str(&agent_id).map_err(|e| e.to_string())?;
    state.manager.profiles().remove(id);
    Ok(())
}

/// 저장된 프로필을 띄운다. `resume=true`면 기존 세션 이어받기(claude `--resume`).
#[tauri::command]
pub async fn spawn_profile(
    state: State<'_, AppState>,
    agent_id: String,
    resume: bool,
) -> Result<AgentInfo, String> {
    let id = Uuid::parse_str(&agent_id).map_err(|e| e.to_string())?;
    let profile = state
        .manager
        .profiles()
        .get(id)
        .ok_or_else(|| format!("profile not found: {id}"))?;
    let mode = if resume {
        SpawnMode::Resume
    } else {
        SpawnMode::Fresh
    };
    state
        .manager
        .spawn_agent(&profile, mode)
        .map_err(|e| e.to_string())
}

/// auto_restore 토글(persist).
#[tauri::command]
pub async fn set_profile_auto_restore(
    state: State<'_, AppState>,
    agent_id: String,
    auto_restore: bool,
) -> Result<(), String> {
    let id = Uuid::parse_str(&agent_id).map_err(|e| e.to_string())?;
    let ok = state
        .manager
        .profiles()
        .update_with(id, |p| p.auto_restore = auto_restore);
    if ok {
        Ok(())
    } else {
        Err(format!("profile not found: {id}"))
    }
}
