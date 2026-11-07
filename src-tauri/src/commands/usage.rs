//! Webview pull of the usage slot — returns one copy of the shell cache (TRD S21 usage-limit-slot §1-7 「웹뷰가 값을 받는 길」 ②).
//! There is no daemon round trip. Interest is not touched — the shell computes interest from the layout.

use std::sync::Arc;
use std::time::Instant;

use engram_dashboard_protocol::UsageLimitSnapshot;
use tauri::State;

use crate::daemon_client::DaemonClient;

/// The answer of `get_usage_snapshot` — ★the field names are the contract with the webview★ (the same snake_case as the payload of broadcast `"usage-limits-updated"`).
/// `socket_epoch` = the current socket marker (ADR-0195 · `0` = no socket — then `snapshots` is empty) ·
/// `snapshots` = per company, the one highest-revision snapshot received from that socket, with relative fields shifted by the seconds elapsed since receipt.
///
/// No ts binding is baked — an element of `snapshots` is a protocol type that cannot be pointed at from the shell's `bindings/`, and the broadcast
/// payload was not baked for the same reason (`daemon_client::events`).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UsageSnapshotReply {
    pub socket_epoch: u64,
    pub snapshots: Vec<UsageLimitSnapshot>,
}

/// Called **after** the webview has set up receiving the usage broadcast — if called first, it misses the broadcasts in between (an old copy that arrives late is told apart by the receiving side
/// with the revision).
#[tauri::command]
pub fn get_usage_snapshot(client: State<'_, Arc<DaemonClient>>) -> UsageSnapshotReply {
    usage_snapshot_reply(&client, Instant::now())
}

/// [`get_usage_snapshot`] 의 본체 — `State` 없이 하네스가 태운다(`daemon_client/connection/tests.rs`).
pub(crate) fn usage_snapshot_reply(client: &DaemonClient, now: Instant) -> UsageSnapshotReply {
    let pulled = client.usage_interest().lock().snapshot_for_webview(now);
    UsageSnapshotReply {
        socket_epoch: pulled.socket_epoch,
        snapshots: pulled.snapshots,
    }
}
