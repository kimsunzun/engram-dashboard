//! The seam for the connection driver's **notices to the frontend** — real = `AppHandle::emit`, harness = a recording
//! fake or a no-op.
//!
//! ★That one line is why this file exists★ (ADR-0012 · sibling seam = [`super::inbound::ViewCommandPort`]): the only
//! reason a connection driver held an `AppHandle` was emitting, and a real `AppHandle` (= `AppHandle<Wry>`) cannot be
//! built without a window — `tauri::test::mock_app()` gives a `MockRuntime`, a different type, and making the client
//! generic over `R: Runtime` spreads to every command signature that names `AppHandle`. So **only the emits** are cut
//! into a port. In return the driver stands on a socket (or a memory link) alone, and the harness measures handshake,
//! reconnect and round trips on the real code.
//! Sibling ports = [`super::refusal`] (command refusal OS box) · [`super::AgentNamesSink`] (tab name book) — both cut
//! separately rather than widening this port (no widening = the [`DaemonEvents`] doc below).
//!
//! ★What the port measures and what it does not★: it cuts **what the driver tells the frontend**, not whether that
//! notice reaches a window — that is the real `AppHandle`'s share, and standing it up in the harness costs exactly the
//! price in the paragraph above. ★Not impossible — a price chosen not to pay★.
//! ★Plug a recording fake in through the client's test constructor★ (`new_on_link`) to assert **what fires in which
//! order** over a round trip on a memory network.
//!
//! ## Naming rule
//! Tauri event **names** (`"agent-list-updated"` etc.) belong to the adapter — the same split as the
//! `crate::layout::LayoutEvents` / `crate::commands::layout::TauriEvents` pair. The caller (the driver) says only
//! **what happened**, not which event name it goes out as.
// ADR-0012

use engram_dashboard_protocol::{
    AgentId, AgentInfo, AgentProfile, AgentStatus, RestoreReport, UsageLimitSnapshot,
};
use tauri::Emitter;

use super::ConnectionStatus;

/// 데몬 연결이 프론트에 내는 알림 포트.
///
/// ★락 미보유 상태에서만 불린다★(락 보유 중 외부 호출 금지 — ADR-0006 원칙). 실패는 **구현이 삼킨다**
/// — 창이 아직 없거나 채널이 닫힌 것은 연결을 되돌릴 사유가 아니고, 프론트는 다음 broadcast·재조회로
/// 회복한다(`crate::layout::LayoutEvents` 와 같은 계약).
///
/// ★여기 있는 일곱 가지가 연결 태스크가 하는 emit 전부다★ — 그보다 넓히지 말 것. 넓히는 순간 이
/// 포트는 「연결 태스크가 프론트에 알리는 것」이 아니라 「AppHandle 대용품」이 되고, 그러면 다시
/// 창 없이 못 세우는 것들이 딸려 들어온다.
/// ★일곱째([`Self::usage_limits_updated`])가 그 경고에 안 걸리는 이유★: 데몬 방송 한 장을 1:1 로 옮기는
/// 같은 부류이고, 덧붙는 `labels`·`socket_epoch` 는 호출 자리가 순수 모듈의 결과와 자기 소켓 표식을 넘길
/// 뿐이다 — 대조·재전송은 포트 밖(호출 자리와 `super::usage_interest`)에 있고 포트는 계산된 값만 받는다
/// (TRD S21 usage-limit-slot §6 #23).
pub(crate) trait DaemonEvents: Send + Sync {
    /// The link's connection status changed — `(state, band)`, either part. ★The payload is the frontend contract★ —
    /// the serialized [`ConnectionStatus`] (`src/api/tauriTransport.ts` parses it; the `daemon_connection_state` pull
    /// returns the same value).
    fn connection_status(&self, status: &ConnectionStatus);

    /// 전체 에이전트 목록 갱신. ★프론트의 terminal 판정은 이것으로만 선다★(status_changed 아님 —
    /// ADR-0005 「상태 알림 분담」).
    fn agent_list_updated(&self, agents: &[AgentInfo]);

    /// 개별 상태 전이 통보. `epoch` = 화신 표식(비교는 일치/불일치만 — ADR-0163).
    fn status_changed(&self, agent_id: &AgentId, status: &AgentStatus, epoch: u32);

    /// 부팅 복원 결과 보고.
    fn restore_result(&self, report: &RestoreReport);

    /// 프로필 목록 갱신(CRUD 후 전 창 동기화).
    fn profile_list_updated(&self, profiles: &[AgentProfile]);

    /// 사용량 한 장 — 데몬 방송 하나당 한 번. `labels` = 그 한 장을 받을 창(사용량 슬롯이 있는 창 — 비지
    /// 않는다), `socket_epoch` = 그 한 장이 온 소켓의 표식(ADR-0195 · 받는 쪽은 이 값이 커질 때만 revision 을
    /// 잊는다). 창을 고르는 것은 받는 웹뷰다 — 전 webview 로 내보낸다.
    fn usage_limits_updated(
        &self,
        snapshot: &UsageLimitSnapshot,
        labels: &[String],
        socket_epoch: u64,
    );
}

/// `"usage-limits-updated"` 의 payload — ★칸 이름이 웹뷰와의 계약이다★(TRD S21 usage-limit-slot §1-8 의
/// `{labels, socket_epoch, snapshot}`). 바꾸면 웹뷰 수신 쪽과 함께 바꾼다.
#[derive(serde::Serialize, Clone)]
struct UsageLimitsUpdatedPayload<'a> {
    labels: &'a [String],
    socket_epoch: u64,
    snapshot: &'a UsageLimitSnapshot,
}

/// 운영 어댑터 — 실 `AppHandle` 로 전 webview 에 push 한다.
///
/// `emit` 의 `Err` 는 전부 삼킨다(위 trait 계약). 소유로 드는 이유는 연결 태스크가 재연결을 넘어
/// 사는 `'static` 태스크라서다 — 빌려주면 그 수명을 태스크에 맞출 방법이 없다.
///
/// ★필드가 `pub` 이 아닌 것이 요점이다★ — 이 어댑터를 조립하는 자리는 운영 생성자
/// (`super::DaemonClient::new_real_with_owned_runtime`) 하나이고, 그 자리는 같은 부모 모듈에 있다.
/// 밖으로 열면 `AppHandle` 을 이 타입에서 도로 꺼내 쓰는 경로가 생기고, 그 순간 이 어댑터는 seam 이
/// 아니라 `AppHandle` 운반통이 된다.
pub(crate) struct TauriEmitter(pub(super) tauri::AppHandle);

impl DaemonEvents for TauriEmitter {
    fn connection_status(&self, status: &ConnectionStatus) {
        let _ = self.0.emit("daemon-connection-state", status);
    }

    fn agent_list_updated(&self, agents: &[AgentInfo]) {
        let _ = self.0.emit("agent-list-updated", agents);
    }

    fn status_changed(&self, agent_id: &AgentId, status: &AgentStatus, epoch: u32) {
        let _ = self.0.emit(
            "status-changed",
            serde_json::json!({
                "agentId": agent_id,
                "status": status,
                "epoch": epoch,
            }),
        );
    }

    fn restore_result(&self, report: &RestoreReport) {
        let _ = self.0.emit(
            "restore-result",
            serde_json::json!({
                "result": report,
            }),
        );
    }

    fn profile_list_updated(&self, profiles: &[AgentProfile]) {
        let _ = self.0.emit("profile-list-updated", profiles);
    }

    fn usage_limits_updated(
        &self,
        snapshot: &UsageLimitSnapshot,
        labels: &[String],
        socket_epoch: u64,
    ) {
        let _ = self.0.emit(
            "usage-limits-updated",
            UsageLimitsUpdatedPayload {
                labels,
                socket_epoch,
                snapshot,
            },
        );
    }
}

/// 알림을 버리는 조립 — ★하네스 전용이다★.
///
/// ★버리는 것이 계약이다★(이걸 꽂은 하네스는 emit 내용을 재지 않는다 — 발화를 재려면 기록형 가짜를
/// `super::DaemonClient::new_on_link` 로 넣는다). 알림 유실은 연결·재연결·명령 왕복 어느 불변식도
/// 건드리지 않는다: 프론트 알림은 **단방향 통지**이고 연결 태스크는 그 성공 여부를 읽지 않는다(실물
/// 어댑터도 `Err` 를 삼킨다).
///
/// ★`#[cfg(test)]` 인 것이 요점이다★ — 운영 빌드에 남겨 두면 "알림을 버리는 클라이언트"를 조용히 조립할
/// 길이 열린다. 그 길로 만들어진 클라이언트는 소켓·명령 왕복이 전부 정상인 채 **화면만 갱신되지 않고**,
/// 프론트의 자가복구는 연결 상태만 메우지 `agent-list-updated` 는 못 메운다(`tauriTransport.ts` Fix-D).
#[cfg(test)]
pub(crate) struct NoDaemonEvents;

#[cfg(test)]
impl DaemonEvents for NoDaemonEvents {
    fn connection_status(&self, _status: &ConnectionStatus) {}
    fn agent_list_updated(&self, _agents: &[AgentInfo]) {}
    fn status_changed(&self, _agent_id: &AgentId, _status: &AgentStatus, _epoch: u32) {}
    fn restore_result(&self, _report: &RestoreReport) {}
    fn profile_list_updated(&self, _profiles: &[AgentProfile]) {}
    fn usage_limits_updated(
        &self,
        _snapshot: &UsageLimitSnapshot,
        _labels: &[String],
        _socket_epoch: u64,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_protocol::{AgentBackendKind, UsageVendorState};

    // ★웹뷰 계약 = snake_case 세 칸★ — 받는 쪽(`src/api/tauriTransport.ts` 의 `usage-limits-updated` 수신)이 이
    //   이름으로 읽는다. 셸 캐시 pull 반환의 칸 이름 시험(`connection/tests.rs`)과 짝이다.
    #[test]
    fn usage_limits_updated_payload_keys_are_the_webview_contract() {
        let snapshot = UsageLimitSnapshot {
            vendor: AgentBackendKind::Codex,
            account_key: "default".into(),
            five_hour: None,
            weekly: None,
            model_scoped: Vec::new(),
            plan: None,
            in_flight: false,
            state: UsageVendorState::Ready,
            revision: 3,
        };
        let labels = vec!["main".to_string()];
        let wire = serde_json::to_value(UsageLimitsUpdatedPayload {
            labels: &labels,
            socket_epoch: 7,
            snapshot: &snapshot,
        })
        .unwrap();
        let mut keys: Vec<&str> = wire
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["labels", "snapshot", "socket_epoch"],
            "웹뷰 계약 = snake_case 세 칸"
        );
        assert_eq!(wire["labels"], serde_json::json!(["main"]));
        assert_eq!(wire["socket_epoch"], 7);
        assert_eq!(wire["snapshot"]["revision"], 3);
    }

    // The `daemon-connection-state` payload and the pull reply, byte for byte — `src/api/tauriTransport.ts` reads these
    //   words and keys; `tauriTransport.test.ts` feeds the same shapes as objects.
    #[test]
    fn connection_status_payload_is_the_frontend_contract() {
        use crate::daemon_client::{Band, ConnectionState};
        let golden = [
            (
                ConnectionStatus {
                    state: ConnectionState::Connected,
                    band: None,
                },
                r#"{"state":"connected","band":null}"#,
            ),
            (
                ConnectionStatus {
                    state: ConnectionState::Reconnecting,
                    band: Some(Band::NotConnected { refused: false }),
                },
                r#"{"state":"reconnecting","band":{"kind":"not_connected","refused":false}}"#,
            ),
            (
                ConnectionStatus {
                    state: ConnectionState::Down,
                    band: Some(Band::NotConnected { refused: true }),
                },
                r#"{"state":"down","band":{"kind":"not_connected","refused":true}}"#,
            ),
            (
                ConnectionStatus {
                    state: ConnectionState::Down,
                    band: Some(Band::Version { can_stop: true }),
                },
                r#"{"state":"down","band":{"kind":"version","can_stop":true}}"#,
            ),
        ];
        for (status, wire) in golden {
            assert_eq!(serde_json::to_string(&status).unwrap(), wire);
        }
    }
}
