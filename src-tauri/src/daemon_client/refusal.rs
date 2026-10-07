//! 데몬이 이 셸의 명령 **등록 · 차분**을 `Error` 로 거절했을 때 사용자에게 띄우는 OS 메시지 박스의 seam 과
//! 그 되풀이 억제.
//!
//! [`super::events::DaemonEvents`] 의 형제 포트다 — 그 포트에 메서드를 더하지 않는다(그 doc 이 넓히기를
//! 금한다). 박스는 프론트 알림이 아니라 OS 창이라 웹뷰를 거치지 않는다.
//!
//! - **출처 판정은 여기서 하지 않는다** — [`RefusalAlerts::surface`] 는 답 경계가 이미 데몬 출처로 가른 실패
//!   (`super::protocol_state::CommandFailure::Daemon`)만 받고 데몬 문구를 읽지 않는다. 그래서 코드
//!   (`CONFLICT` · `INVALID_ARGUMENT` · 모르는 코드)와 무관하게 띄운다 — 코드 추가는 허용된 확장이다. 셸 로컬
//!   실패(끊김 · 미연결 · 송신 실패)는 이리 오지 않는다.
//! - **LLM 에게는 알리지 않는다**(사용자 결정 — ADR-0270 결정 4). 원인은 부르는 쪽의 warn 로그에 매번 남는다.
//! - **셸에 OS 분기를 들이지 않는다** — 창의 OS 차이는 플러그인(rfd) 안이다. capability 도 바꾸지 않는다 —
//!   Rust API 는 IPC 를 거치지 않고, `dialog:allow-message` 는 JS 쪽 `message` 명령의 권한이다.
// ADR-0270
// ADR-0281

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use engram_dashboard_base::sync;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RefusalSite {
    /// 연결마다의 전량 등록(`RegisterCommands`) — 거절되면 이 셸의 명령이 데몬 명부에 하나도 없다.
    Register,
    /// 웹뷰 화면 명령의 차분(`UpdateCommands`) — 거절되면 그 차분만 빠진다.
    Update,
}

/// 거절 박스 하나를 띄우는 포트. 되풀이 억제는 [`RefusalAlerts`] 가 이미 했다 — 구현은 불릴 때마다 띄운다.
///
/// ★락 미보유 상태에서만 불린다★(ADR-0006). 구현은 막지 않고(연결 태스크 · 명령 경로에서 불린다) 실패를
/// 삼킨다 — 박스가 못 뜬 것은 연결을 되돌릴 사유가 아니다([`super::events::DaemonEvents`] 와 같은 계약).
pub(crate) trait RefusalNotice: Send + Sync {
    fn show(&self, site: RefusalSite, daemon_message: &str);
}

/// 운영 어댑터 — tauri-plugin-dialog 의 Rust API 로 OS 메시지 박스를 띄운다.
///
/// - **`show` 만 쓴다 — `blocking_show` 금지.** `show` 는 창을 메인 스레드에 맡기고 기다림은 따로 띄운
///   스레드가 진다(플러그인 2.7.1 `desktop.rs` 의 `show_message_dialog`). `blocking_show` 는 부른 스레드를
///   박스가 닫힐 때까지 세운다 — 연결 태스크면 소켓이, 메인 스레드면 앱이 멈춘다.
/// - **종류 = `Error`**(사용자 결정 2026-10-07 「강하게 경고 표시」).
/// - ★`dialog()` 는 플러그인 상태가 없으면 패닉한다★(`Manager::state`) — 운영 빌드는 `panic = "abort"` 라
///   앱이 내려간다. `lib.rs` 가 빌더에서 플러그인을 무조건 등록하므로 운영에서는 닿지 않는다. 그 등록을
///   걷으면 이 박스가 패닉으로 바뀐다.
/// - 실패는 삼킨다 — 플러그인이 메인 스레드 위임의 `Err` 를 이미 버린다(같은 함수).
///
/// 필드가 `pub` 이 아닌 이유는 [`super::events::TauriEmitter`] 와 같다.
pub(crate) struct TauriRefusalBox(pub(super) tauri::AppHandle);

impl RefusalNotice for TauriRefusalBox {
    fn show(&self, site: RefusalSite, daemon_message: &str) {
        let (title, body) = refusal_text(site, daemon_message);
        self.0
            .dialog()
            .message(body)
            .title(title)
            .kind(MessageDialogKind::Error)
            .show(|_| {});
    }
}

/// 박스를 버리는 조립 — ★하네스 전용이다★. `#[cfg(test)]` 인 이유는 [`super::events::NoDaemonEvents`] 와
/// 같다 — 운영 빌드에 두면 거절이 와도 박스가 안 뜨는 클라이언트를 조용히 조립할 길이 열린다.
#[cfg(test)]
pub(crate) struct NoRefusalNotice;

#[cfg(test)]
impl RefusalNotice for NoRefusalNotice {
    fn show(&self, _site: RefusalSite, _daemon_message: &str) {}
}

#[cfg(test)]
pub(crate) struct RecordingRefusals {
    shown: Mutex<Vec<(RefusalSite, String)>>,
}

#[cfg(test)]
impl RecordingRefusals {
    pub(crate) fn new() -> Self {
        Self {
            shown: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn shown(&self) -> Vec<(RefusalSite, String)> {
        sync::lock(&self.shown).clone()
    }
}

#[cfg(test)]
impl RefusalNotice for RecordingRefusals {
    fn show(&self, site: RefusalSite, daemon_message: &str) {
        sync::lock(&self.shown).push((site, daemon_message.to_owned()));
    }
}

/// 거절 박스의 거르개 — 같은 (자리, 데몬 문구)는 이 프로세스에서 한 번만 포트에 닿는다.
///
/// 키 = (자리, 데몬 문구) · 수명 = 셸 프로세스:
/// - 거절은 같은 빌드 · 같은 이름이면 결정적이라 재연결마다 같은 문구로 다시 온다 — 다시 띄워도 새 정보가
///   없고, 데몬 재기동 · 절전 복귀처럼 재연결이 잦으면 창이 쌓인다. 「연결마다 한 번」은 이 반복을 못 막는다.
/// - 데몬 문구가 원인(겹친 이름 · 넘친 크기 · 상한 수치)을 싣기에 다른 사고는 다른 키라 새로 뜬다.
/// - 자리를 키에 넣는 것은 두 거절의 무게가 달라서다(등록 = 셸 명령 0 · 차분 = 화면 몫 차분만 빠짐).
///
/// 대가 — 같은 사고의 두 번째부터는 박스가 없다. 원인 warn 은 부르는 쪽이 매번 찍는다.
pub(crate) struct RefusalAlerts {
    port: Arc<dyn RefusalNotice>,
    shown: Mutex<HashSet<(RefusalSite, String)>>,
}

impl RefusalAlerts {
    pub(crate) fn new(port: Arc<dyn RefusalNotice>) -> Self {
        Self {
            port,
            shown: Mutex::new(HashSet::new()),
        }
    }

    /// ★데몬 출처의 거절에만 부른다★(`CommandFailure::Daemon`) — 여기서 문구를 읽어 출처를 가리지 않는다.
    /// 띄움 / 눌러 둠을 info 로그 한 줄로 남긴다 — GUI 실측이 박스 결정을 이 로그로 센다.
    pub(crate) fn surface(&self, site: RefusalSite, daemon_message: &str) {
        // ADR-0006: 판정만 락 안 — 로그 · 포트 호출은 락을 놓은 뒤라 포트가 이 거르개로 재진입해도 교착하지 않는다.
        let first = sync::lock(&self.shown).insert((site, daemon_message.to_owned()));
        if first {
            tracing::info!(site = ?site, "명령 거절 박스 — 띄움");
            self.port.show(site, daemon_message);
        } else {
            tracing::info!(
                site = ?site,
                "명령 거절 박스 — 눌러 둠(이 프로세스에서 같은 자리 · 같은 문구를 이미 띄웠다)"
            );
        }
    }
}

/// 박스의 (제목, 본문). 데몬 문구는 원문 그대로 싣는다 — 코드 접두와 원인(겹친 이름 · 넘친 크기 · 상한
/// 수치)이 거기 있고, 그래서 코드별 문구를 따로 두지 않는다.
pub(crate) fn refusal_text(site: RefusalSite, daemon_message: &str) -> (String, String) {
    let lead = match site {
        RefusalSite::Register => {
            "데몬이 이 셸의 명령 등록을 통째로 거절했습니다.\nLLM 이 창 · 탭 · 슬롯 명령을 하나도 쓸 수 없습니다(원인이 고쳐질 때까지)."
        }
        RefusalSite::Update => {
            "데몬이 화면 명령 명단 갱신을 거절했습니다.\n바뀐 화면 명령을 LLM 이 쓸 수 없고, 원인을 고치지 않은 채 재연결하면 이 셸의 명령 등록 전체가 거절될 수 있습니다."
        }
    };
    (
        "Engram — 명령 등록 거절".to_owned(),
        format!(
            "{lead}\n\n데몬 답:\n{daemon_message}\n\n원인이 된 명령 선언을 고친 뒤 다시 빌드하세요."
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, OnceLock, Weak};
    use std::time::Duration;

    fn recording() -> (Arc<RecordingRefusals>, RefusalAlerts) {
        let port = Arc::new(RecordingRefusals::new());
        let alerts = RefusalAlerts::new(port.clone());
        (port, alerts)
    }

    // ── 되풀이 억제 ──

    #[test]
    fn same_site_and_message_reaches_the_port_once() {
        let (port, alerts) = recording();
        alerts.surface(RefusalSite::Register, "CONFLICT: agent.rename");
        alerts.surface(RefusalSite::Register, "CONFLICT: agent.rename");
        assert_eq!(
            port.shown(),
            vec![(RefusalSite::Register, "CONFLICT: agent.rename".to_owned())]
        );
    }

    #[test]
    fn same_message_at_another_site_is_a_new_key() {
        let (port, alerts) = recording();
        alerts.surface(RefusalSite::Register, "CONFLICT: agent.rename");
        alerts.surface(RefusalSite::Update, "CONFLICT: agent.rename");
        assert_eq!(
            port.shown(),
            vec![
                (RefusalSite::Register, "CONFLICT: agent.rename".to_owned()),
                (RefusalSite::Update, "CONFLICT: agent.rename".to_owned()),
            ]
        );
    }

    #[test]
    fn another_message_at_the_same_site_is_a_new_key() {
        let (port, alerts) = recording();
        alerts.surface(RefusalSite::Update, "CONFLICT: agent.rename");
        alerts.surface(RefusalSite::Update, "INVALID_ARGUMENT: too long");
        assert_eq!(
            port.shown(),
            vec![
                (RefusalSite::Update, "CONFLICT: agent.rename".to_owned()),
                (RefusalSite::Update, "INVALID_ARGUMENT: too long".to_owned()),
            ]
        );
    }

    // ── 락 밖 포트 호출(ADR-0006) ──

    // `show` 안에서 같은 거르개의 `surface` 를 다시 부른다. 락을 쥔 채 포트를 부르면 std `Mutex` 재잠금이
    // 교착(또는 패닉)하므로, 따로 띄운 스레드의 끝을 유계로 기다려 매달리지 않고 실패로 끝낸다.
    struct Reentrant {
        alerts: OnceLock<Weak<RefusalAlerts>>,
        shown: Mutex<Vec<(RefusalSite, String)>>,
    }

    impl RefusalNotice for Reentrant {
        fn show(&self, site: RefusalSite, daemon_message: &str) {
            sync::lock(&self.shown).push((site, daemon_message.to_owned()));
            if let Some(alerts) = self.alerts.get().and_then(Weak::upgrade) {
                alerts.surface(RefusalSite::Update, "재진입");
            }
        }
    }

    #[test]
    fn a_port_that_reenters_surface_does_not_deadlock() {
        let port = Arc::new(Reentrant {
            alerts: OnceLock::new(),
            shown: Mutex::new(Vec::new()),
        });
        let alerts = Arc::new(RefusalAlerts::new(port.clone()));
        assert!(port.alerts.set(Arc::downgrade(&alerts)).is_ok());

        let (done_tx, done_rx) = mpsc::channel();
        let worker = alerts.clone();
        std::thread::spawn(move || {
            worker.surface(RefusalSite::Register, "CONFLICT: agent.rename");
            let _ = done_tx.send(());
        });
        assert!(
            done_rx.recv_timeout(Duration::from_secs(5)).is_ok(),
            "포트 안의 재진입이 5초 안에 안 끝났다 — 락을 쥔 채 포트를 부른다"
        );
        // 재진입 첫 키는 새로 떠서 포트에 다시 닿고, 그 안의 둘째 재진입은 같은 키라 눌린다.
        assert_eq!(
            *sync::lock(&port.shown),
            vec![
                (RefusalSite::Register, "CONFLICT: agent.rename".to_owned()),
                (RefusalSite::Update, "재진입".to_owned()),
            ]
        );
    }

    // ── 박스 글 ──

    #[test]
    fn refusal_text_carries_the_daemon_message_verbatim_and_the_action() {
        let daemon_message =
            "CONFLICT: names already answered by the daemon: agent.rename, {x}\n둘째 줄";
        for site in [RefusalSite::Register, RefusalSite::Update] {
            let (title, body) = refusal_text(site, daemon_message);
            assert_eq!(title, "Engram — 명령 등록 거절");
            assert!(
                body.contains(&format!("\n\n데몬 답:\n{daemon_message}\n\n")),
                "{site:?}: 데몬 문구가 원문 그대로 실려야 한다 — {body}"
            );
            assert!(
                body.ends_with("원인이 된 명령 선언을 고친 뒤 다시 빌드하세요."),
                "{site:?}: 할 일 줄 — {body}"
            );
        }
        let (_, register) = refusal_text(RefusalSite::Register, daemon_message);
        let (_, update) = refusal_text(RefusalSite::Update, daemon_message);
        assert!(register.starts_with("데몬이 이 셸의 명령 등록을 통째로 거절했습니다.\n"));
        assert!(update.starts_with("데몬이 화면 명령 명단 갱신을 거절했습니다.\n"));
        assert!(register.contains("(원인이 고쳐질 때까지)"));
        // 셸 다리는 거절된 이름을 되돌리지 않고 쥐므로 선언이 그대로면 다음 전량 등록에 그 이름이 다시 실려
        // 거절될 수 있다. 단 웹뷰가 바뀐 목록을 다시 보고하거나 차분에만 서는 거절(남의 이름 CONFLICT 등)이면 안 되풀이되어 「될 수 있다」다 —
        // 「셸 명령은 그대로」로도, 단정형으로도 되돌리지 말 것.
        assert!(update.contains(
            "원인을 고치지 않은 채 재연결하면 이 셸의 명령 등록 전체가 거절될 수 있습니다."
        ));
    }
}
