//! 연결 태스크가 **프론트로 내는 알림**의 seam — 실물은 `AppHandle::emit`, 하네스는 기록형 가짜 또는 no-op.
//!
//! ★이 파일의 존재 이유가 그 한 줄이다★(ADR-0012 · 형제 seam = [`super::inbound::ViewCommandPort`]):
//! `connection.rs` 가 `AppHandle` 을 쥐던 유일한 사유는 emit 이었는데, 실물 `AppHandle`(= `AppHandle<Wry>`)
//! 은 창 없이 못 세운다 — `tauri::test::mock_app()` 이 주는 것은 `MockRuntime` 이라 타입이 다르고,
//! 클라이언트를 `R: Runtime` 로 일반화하면 `AppHandle` 을 이름으로 적는 커맨드 시그니처 전부로 번진다.
//! 그래서 **emit 만** 포트로 끊는다. 그 대가로 연결 태스크는 소켓만 있으면 서고, 하네스가 실코드로
//! 핸드셰이크·재연결·pending 왕복을 잰다.
//!
//! ★포트가 재는 것과 안 재는 것★: 이 포트는 **연결 태스크가 프론트에 무엇을 알리나**를 끊을 뿐,
//! 그 알림이 실제로 창에 닿는지는 재지 않는다 — 그건 실 `AppHandle` 몫이고, 그것을 하네스에 세우는
//! 값이 바로 위 문단의 그 값이다(`R: Runtime` 일반화가 커맨드 시그니처 전부로 번진다). ★불가능이
//! 아니라 **치르지 않기로 한 대가**다★.
//! ★꽂는 자리는 `super::DaemonClient::new_with_events` 다★ — 기록형 가짜를 그리로 넣으면 **무엇이
//! 어떤 순서로 발화되나**를 실 소켓 왕복 위에서 단언할 수 있다(`tests.rs` 의
//! `recording_events_capture_connected_then_broadcasts` 가 그 하나다). 나머지 발화는 여전히 무검증이고,
//! 늘리는 자리는 그 테스트 옆이다 — 주입점이 없던 시절처럼 "열 방법이 없다" 는 아니다.
//!
//! ## 이름 규약
//! Tauri 이벤트 **이름**(`"agent-list-updated"` 등)은 어댑터가 소유한다 — `crate::layout::LayoutEvents` /
//! `crate::commands::layout::TauriEvents` 짝과 같은 가름이다. 호출부(`connection.rs`)는 **무슨 일이
//! 일어났나**만 말하고, 그것이 어떤 이벤트 이름으로 나가는지는 모른다.
// ADR-0012

use engram_dashboard_protocol::{
    AgentId, AgentInfo, AgentProfile, AgentStatus, Preset, RestoreReport,
};
use tauri::Emitter;

/// 연결 수명 상태 알림이 실을 수 있는 값 **전부**.
///
/// ★왜 문자열이 아니라 타입인가★: [`ConnectionStateEvent::as_str`] 이 내는 세 리터럴은 **프론트 계약**이다
/// (`src/api/tauriTransport.ts` 의 `applyConnectionState` 가 그 값을 그대로 비교하고, `crate::commands::discovery`
/// 의 상태 조회도 같은 어휘를 낸다). 호출부가 리터럴을 직접 적으면 오타가 컴파일에 안 걸리고 **화면만 조용히
/// 멈춘다** — 이벤트 *이름*은 어댑터가 소유하면서 *payload 어휘*만 호출부에 흘려 두던 비대칭을 여기서 닫는다.
///
/// ★**이 경로에서** 문자열을 굽는 곳은 [`ConnectionStateEvent::as_str`] 하나다★ — 어댑터가 그것을 부른다.
/// 저장소 전체로는 하나가 아니다: 위 문단이 가리킨 그 소비자 쪽, `crate::commands::discovery` 의 상태 조회
/// invoke 가 같은 세 리터럴을 **독립으로** 굽어 같은 프론트에 낸다. 그쪽까지 이 타입으로 모으는 것은 별건이고,
/// 여기서 보증하는 것은 「연결 태스크가 내는 알림에는 리터럴이 흩어지지 않는다」까지다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectionStateEvent {
    /// Hello 수신 = 인증 성공(첫 연결·재연결 회복 공통).
    Connected,
    /// 비의도 끊김 후 백오프 재연결 진입.
    Reconnecting,
    /// 재연결 소진·stale 종료의 종착.
    Down,
}

impl ConnectionStateEvent {
    /// ★이 세 리터럴은 살아 있는 프론트 계약이다 — 바꾸면 `tauriTransport.ts` 와 **함께** 바꾼다.★
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ConnectionStateEvent::Connected => "connected",
            ConnectionStateEvent::Reconnecting => "reconnecting",
            ConnectionStateEvent::Down => "down",
        }
    }
}

/// 데몬 연결이 프론트에 내는 알림 포트.
///
/// ★락 미보유 상태에서만 불린다★(락 보유 중 외부 호출 금지 — ADR-0006 원칙). 실패는 **구현이 삼킨다**
/// — 창이 아직 없거나 채널이 닫힌 것은 연결을 되돌릴 사유가 아니고, 프론트는 다음 broadcast·재조회로
/// 회복한다(`crate::layout::LayoutEvents` 와 같은 계약).
///
/// ★여기 있는 여섯 가지가 연결 태스크가 하는 emit 전부다★ — 그보다 넓히지 말 것. 넓히는 순간 이
/// 포트는 「연결 태스크가 프론트에 알리는 것」이 아니라 「AppHandle 대용품」이 되고, 그러면 다시
/// 창 없이 못 세우는 것들이 딸려 들어온다.
pub(crate) trait DaemonEvents: Send + Sync {
    /// 연결 수명 상태 한 줄. ★wire 에 실리는 payload 는 여전히 **문자열**이고 그것이 프론트와의 계약이다★
    /// — 바뀐 것은 그 문자열을 **누가 짓나**뿐이다([`ConnectionStateEvent`] 가 어휘를 쥐고 어댑터가 굽는다).
    fn connection_state(&self, state: ConnectionStateEvent);

    /// 전체 에이전트 목록 갱신. ★프론트의 terminal 판정은 이것으로만 선다★(status_changed 아님 —
    /// ADR-0005 「상태 알림 분담」).
    fn agent_list_updated(&self, agents: &[AgentInfo]);

    /// 개별 상태 전이 통보. `epoch` = 화신 표식(비교는 일치/불일치만 — ADR-0163).
    fn status_changed(&self, agent_id: &AgentId, status: &AgentStatus, epoch: u32);

    /// 부팅 복원 결과 보고.
    fn restore_result(&self, report: &RestoreReport);

    /// 프로필 목록 갱신(CRUD 후 전 창 동기화).
    fn profile_list_updated(&self, profiles: &[AgentProfile]);

    /// 프리셋 목록 갱신(ADR-0061 — CRUD 후 전 창 동기화).
    fn preset_list_updated(&self, presets: &[Preset]);
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
    fn connection_state(&self, state: ConnectionStateEvent) {
        // ★문자열은 여기서 단 한 번 구워진다★ — 호출부는 어휘만 고른다.
        let _ = self.0.emit("daemon-connection-state", state.as_str());
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

    fn preset_list_updated(&self, presets: &[Preset]) {
        let _ = self.0.emit("preset-list-updated", presets);
    }
}

/// 알림을 버리는 조립 — ★하네스 전용이다★.
///
/// ★버리는 것이 계약이다★(이걸 꽂은 하네스는 emit 내용을 재지 않는다 — 발화를 재려면 기록형 가짜를
/// `super::DaemonClient::new_with_events` 로 넣는다). 알림 유실은 연결·재연결·명령 왕복 어느 불변식도
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
    fn connection_state(&self, _state: ConnectionStateEvent) {}
    fn agent_list_updated(&self, _agents: &[AgentInfo]) {}
    fn status_changed(&self, _agent_id: &AgentId, _status: &AgentStatus, _epoch: u32) {}
    fn restore_result(&self, _report: &RestoreReport) {}
    fn profile_list_updated(&self, _profiles: &[AgentProfile]) {}
    fn preset_list_updated(&self, _presets: &[Preset]) {}
}
