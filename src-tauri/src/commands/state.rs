//! The human path of the crash copy 「복원할까요?」 — two invoke shells that call the same restore
//! coordinator as the bus `restore.status` · `restore.answer` (`layout::commands`) (TRD S21-storage §6-7 · ADR-0081 decision 3 「두 껍데기, 서비스 하나」). When the state changes,
//! the coordinator's service emits `restore:changed` to main.

use std::sync::Arc;

use tauri::State;

use crate::state::restore::{AnswerReply, RestoreCoordinator, RestoreStatusView};

/// The human path of `restore.status` — does not block (one leaf lock).
#[tauri::command]
pub fn restore_status(restore: State<'_, Arc<RestoreCoordinator>>) -> RestoreStatusView {
    restore.status()
}

/// The human path of `restore.answer`. It does not tell kinds apart by the error string — the state after a failure is read again with `restore_status`
/// (`awaiting` = can answer again · `answered` = another answer finished first).
///
/// ★Runs on the blocking pool★ — acceptance builds windows while waiting for the event loop, and the answer waits for the recorder to finish (2 seconds). A synchronous
/// command runs on the main thread, so window creation waits for itself and stalls, and calling it directly from an async body would block that runtime
/// worker for that long. ★The body outlives the answer★ — even if the waiting side disappears, the answer goes through to the end.
#[tauri::command]
pub async fn restore_answer(
    restore: State<'_, Arc<RestoreCoordinator>>,
    accept: bool,
) -> Result<AnswerReply, String> {
    let restore = Arc::clone(&restore);
    match tauri::async_runtime::spawn_blocking(move || restore.answer(accept)).await {
        Ok(answered) => answered.map_err(|e| e.to_string()),
        Err(e) => Err(format!("복원 답이 끝나지 못했다: {e}")),
    }
}
