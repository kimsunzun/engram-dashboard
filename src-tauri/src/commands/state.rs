//! 크래시 사본 「복원할까요?」의 사람 경로 — 버스 `restore.status` · `restore.answer`(`layout::commands`)와 같은 복원
//! 조율자를 부르는 invoke 껍데기 둘(TRD S21-storage §6-7 · ADR-0081 결정 3 「두 껍데기, 서비스 하나」). 상태가 바뀌면
//! 조율자의 서비스가 `restore:changed` 를 main 에 낸다.

use std::sync::Arc;

use tauri::State;

use crate::state::restore::{AnswerReply, RestoreCoordinator, RestoreStatusView};

/// `restore.status` 의 사람 경로 — 막지 않는다(잎 락 하나).
#[tauri::command]
pub fn restore_status(restore: State<'_, Arc<RestoreCoordinator>>) -> RestoreStatusView {
    restore.status()
}

/// `restore.answer` 의 사람 경로. 오류 문자열로 종류를 가르지 않는다 — 실패 뒤의 상태는 `restore_status` 로 다시
/// 읽는다(`awaiting` = 다시 답할 수 있다 · `answered` = 다른 답이 먼저 끝났다).
///
/// ★블로킹 풀에서 돈다★ — 수락은 창을 만들며 이벤트 루프를 기다리고 답은 기록기를 마감(2초)까지 기다린다. 동기
/// command 는 메인 스레드에서 돌아 창 만들기가 자기 자신을 기다려 멈추고, async 본문에서 그대로 부르면 그 런타임
/// 워커가 그만큼 선다. ★답보다 본문이 오래 산다★ — 기다리던 쪽이 사라져도 답은 끝까지 간다.
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
