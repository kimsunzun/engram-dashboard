//! The seam of the OS message box shown to the user when the daemon rejects this shell's command **registration · delta** with `Error`,
//! and its repeat suppression.
//!
//! It is a sibling port of [`super::events::DaemonEvents`] — no method is added to that port (its doc forbids widening it). The box is
//! an OS window, not a frontend notification, so it does not go through the webview.
//!
//! - **The origin decision is not made here** — [`RefusalAlerts::surface`] receives only failures that the answer boundary has already classified as daemon-origin
//!   (`super::protocol_state::CommandFailure::Daemon`) and does not read the daemon text. So it is shown regardless of the code
//!   (`CONFLICT` · `INVALID_ARGUMENT` · an unknown code) — adding a code is a permitted extension. Shell-local
//!   failures (disconnect · not connected · send failure) do not come here.
//! - **The LLM is not told** (user decision — ADR-0270 decision 4). The cause is left every time in the caller's warn log.
//! - **No OS branch is brought into the shell** — the OS difference of the window is inside the plugin (rfd). Capability is not changed either —
//!   the Rust API does not go through IPC, and `dialog:allow-message` is the permission of the JS-side `message` command.
// ADR-0270
// ADR-0281

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use engram_dashboard_base::sync;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RefusalSite {
    /// Full registration per connection (`RegisterCommands`) — if rejected, none of this shell's commands are in the daemon registry.
    Register,
    /// The delta of webview screen commands (`UpdateCommands`) — if rejected, only that delta is missing.
    Update,
}

/// A port that shows one rejection box. Repeat suppression was already done by [`RefusalAlerts`] — an implementation shows it every time it is called.
///
/// ★Called only without holding a lock★ (ADR-0006). An implementation does not block (it is called from the connection task · command path) and swallows failure —
/// a box failing to appear is no reason to undo the connection (the same contract as [`super::events::DaemonEvents`]).
pub(crate) trait RefusalNotice: Send + Sync {
    fn show(&self, site: RefusalSite, daemon_message: &str);
}

/// Production adapter — shows an OS message box with the Rust API of tauri-plugin-dialog.
///
/// - **Use only `show` — `blocking_show` is forbidden.** `show` hands the window to the main thread and the waiting is borne by a separately launched
///   thread (`show_message_dialog` of plugin 2.7.1 `desktop.rs`). `blocking_show` stalls the calling thread
///   until the box is closed — the socket if it is the connection task, the app if it is the main thread.
/// - **Kind = `Error`** (user decision 2026-10-07 「강하게 경고 표시」).
/// - ★`dialog()` panics if there is no plugin state★ (`Manager::state`) — the shell's panic ends the app via the hook
///   (ADR-0292). `lib.rs` registers the plugin unconditionally in the builder, so it is not reached in production. If that registration
///   is removed, this box turns into a panic.
/// - Failure is swallowed — the plugin already discards the `Err` of the main-thread delegation (the same function).
///
/// The reason the field is not `pub` is the same as [`super::events::TauriEmitter`].
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

/// An assembly that discards the box — ★harness only★. The reason for `#[cfg(test)]` is the same as [`super::events::NoDaemonEvents`] —
/// if put in a production build, it opens a way to quietly assemble a client whose box does not appear even when a rejection comes.
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

/// The rejection-box filter — the same (place, daemon text) reaches the port only once in this process.
///
/// Key = (place, daemon text) · lifetime = the shell process:
/// - A rejection is deterministic for the same build · same name, so it comes again with the same text on every reconnect — showing it again gives no new information,
///   and when reconnects are frequent, like a daemon restart · resume from sleep, windows pile up. 「연결마다 한 번」 (once per connection) cannot stop this repetition.
/// - The daemon text carries the cause (an overlapping name · an overflowing size · the cap value), so a different incident is a different key and shows afresh.
/// - Putting the place in the key is because the weight of the two rejections differs (registration = shell commands 0 · delta = only the screen share's delta missing).
///
/// Cost — from the second occurrence of the same incident there is no box. The cause warn is printed by the caller every time.
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

    /// ★Called only for daemon-origin rejections★ (`CommandFailure::Daemon`) — the text is not read here to tell the origin apart.
    /// Shown / suppressed is left as one info log line — GUI measurement counts box decisions with this log.
    pub(crate) fn surface(&self, site: RefusalSite, daemon_message: &str) {
        // ADR-0006: only the decision is inside the lock — the log · port call is after releasing the lock, so even if the port reenters this filter it does not deadlock.
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

/// The box's (title, body). The daemon text is carried verbatim — the code prefix and the cause (an overlapping name · an overflowing size · the cap
/// value) are in it, and so no per-code text is kept separately.
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

    // ── repeat suppression ──

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

    // ── port call outside the lock (ADR-0006) ──

    // Inside `show`, call `surface` of the same filter again. Calling the port while holding the lock makes a std `Mutex` relock
    // deadlock (or panic), so it waits a bounded time for the end of the separately launched thread and ends in failure instead of hanging.
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
        // The first reentrant key appears afresh and reaches the port again, and the second reentry inside it is the same key and is suppressed.
        assert_eq!(
            *sync::lock(&port.shown),
            vec![
                (RefusalSite::Register, "CONFLICT: agent.rename".to_owned()),
                (RefusalSite::Update, "재진입".to_owned()),
            ]
        );
    }

    // ── box text ──

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
        // The shell bridge holds the rejected name without reverting it, so if the declaration stays the same, that name may be carried again on the next full registration
        // and rejected. But if the webview reports a changed list again, or the rejection stands only on the delta (CONFLICT with someone else's name, etc.), it does not repeat, so it is 「될 수 있다」 (may be) —
        // do not revert it to 「셸 명령은 그대로」 (shell commands stay as they are), nor to a definitive form.
        assert!(update.contains(
            "원인을 고치지 않은 채 재연결하면 이 셸의 명령 등록 전체가 거절될 수 있습니다."
        ));
    }
}
