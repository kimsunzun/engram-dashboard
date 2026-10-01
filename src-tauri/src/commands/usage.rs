//! 사용량 슬롯의 웹뷰 당김 — 셸 캐시 한 벌을 돌려준다(TRD S21 usage-limit-slot §1-7 「웹뷰가 값을 받는 길」 ②).
//! 데몬 왕복이 없다. 관심은 건드리지 않는다 — 관심은 셸이 레이아웃에서 계산한다.

use std::sync::Arc;
use std::time::Instant;

use engram_dashboard_protocol::UsageLimitSnapshot;
use tauri::State;

use crate::daemon_client::DaemonClient;

/// `get_usage_snapshot` 의 답 — ★칸 이름이 웹뷰와의 계약이다★(방송 `"usage-limits-updated"` 의 payload 와 같은
/// snake_case). `socket_epoch` = 지금 소켓 표식(ADR-0195 · `0` = 소켓 없음 — 그때 `snapshots` 는 빈다) ·
/// `snapshots` = 회사마다 그 소켓에서 받은 최고 revision 한 장, 받은 뒤 흐른 초만큼 상대 칸을 옮긴 것.
///
/// ts 바인딩을 굽지 않는다 — `snapshots` 의 원소가 protocol 타입이라 셸 `bindings/` 에서 가리킬 수 없고, 방송
/// payload 도 같은 이유로 굽지 않았다(`daemon_client::events`).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UsageSnapshotReply {
    pub socket_epoch: u64,
    pub snapshots: Vec<UsageLimitSnapshot>,
}

/// 웹뷰가 사용량 방송 수신을 건 **뒤** 부른다 — 먼저 부르면 그 사이의 방송을 놓친다(늦게 닿은 옛 사본은 받는 쪽이
/// revision 으로 가른다).
#[tauri::command]
pub fn get_usage_snapshot(client: State<'_, Arc<DaemonClient>>) -> UsageSnapshotReply {
    usage_snapshot_reply(&client, Instant::now())
}

/// [`get_usage_snapshot`] 의 본체 — `State` 없이 하네스가 태운다(`daemon_client/tests.rs`).
pub(crate) fn usage_snapshot_reply(client: &DaemonClient, now: Instant) -> UsageSnapshotReply {
    let pulled = client.usage_interest().lock().snapshot_for_webview(now);
    UsageSnapshotReply {
        socket_epoch: pulled.socket_epoch,
        snapshots: pulled.snapshots,
    }
}
