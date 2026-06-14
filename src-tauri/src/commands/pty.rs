use std::sync::Arc;

use tauri::State;
use uuid::Uuid;

use crate::{AppState, ChannelOutputSink};
use engram_dashboard_core::pty::types::{OutputChunk, PtyEvent, SinkId};

#[tauri::command]
pub async fn subscribe_agent_output(
    state: State<'_, AppState>,
    agent_id: String,
    channel: tauri::ipc::Channel<PtyEvent>,
) -> Result<SinkId, String> {
    let id = Uuid::parse_str(&agent_id).map_err(|e| e.to_string())?;
    let sink = Arc::new(ChannelOutputSink::new(channel));
    state.manager.subscribe(id, sink).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn unsubscribe_agent_output(
    state: State<'_, AppState>,
    agent_id: String,
    sink_id: String,
) -> Result<(), String> {
    let id = Uuid::parse_str(&agent_id).map_err(|e| e.to_string())?;
    let sid = Uuid::parse_str(&sink_id).map_err(|e| e.to_string())?;
    state
        .manager
        .unsubscribe(id, sid)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn write_stdin(
    state: State<'_, AppState>,
    agent_id: String,
    data: Vec<u8>,
) -> Result<(), String> {
    let id = Uuid::parse_str(&agent_id).map_err(|e| e.to_string())?;
    state
        .manager
        .write_stdin(id, &data)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn resize_pty(
    state: State<'_, AppState>,
    agent_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let id = Uuid::parse_str(&agent_id).map_err(|e| e.to_string())?;
    state
        .manager
        .resize(id, cols, rows)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_agent_snapshot(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<Vec<OutputChunk>, String> {
    let id = Uuid::parse_str(&agent_id).map_err(|e| e.to_string())?;
    state.manager.get_snapshot(id).map_err(|e| e.to_string())
}
