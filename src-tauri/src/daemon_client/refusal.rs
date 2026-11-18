//! The seam of the OS message box shown to the user when the daemon rejects this shell's command registration · delta with `Error`, and its repeat suppression. A sibling port of [`super::events::DaemonEvents`] (its doc forbids widening it); the box is an OS window, not a frontend notification, so it does not go through the webview.
//! - The origin decision is not made here: [`RefusalAlerts::surface`] receives only failures the answer boundary already classified as daemon-origin (`super::protocol_state::CommandFailure::Daemon`) and does not read the daemon text, so it shows regardless of code (`CONFLICT` · `INVALID_ARGUMENT` · an unknown code — adding a code is a permitted extension). Shell-local failures (disconnect · not connected · send failure) do not come here.
//! - The LLM is not told (user decision — ADR-0270 decision 4); the cause is left every time in the caller's warn log.
//! - No OS branch in the shell — the window's OS difference is inside the plugin (rfd). Capability is unchanged too: the Rust API does not go through IPC, and `dialog:allow-message` is the permission of the JS-side `message` command.
// ADR-0270 · ADR-0281

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use engram_dashboard_base::sync;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

use crate::ui_text::{self, Lang};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RefusalSite {
    Register,
    Update,
}

/// A port that shows one rejection box. Repeat suppression was already done by [`RefusalAlerts`] — an implementation shows it every time it is called. Called only without holding a lock (ADR-0006); an implementation does not block (it is called from the connection task · command path) and swallows failure — a box failing to appear is no reason to undo the connection (same contract as [`super::events::DaemonEvents`]).
pub(crate) trait RefusalNotice: Send + Sync {
    fn show(&self, site: RefusalSite, daemon_message: &str);
}

/// Production adapter — shows an OS message box with the Rust API of tauri-plugin-dialog.
/// - Use only `show`, never `blocking_show`: `show` hands the window to the main thread and a separately launched thread bears the wait (`show_message_dialog` of plugin 2.7.1 `desktop.rs`); `blocking_show` stalls the calling thread until the box closes — the socket if it is the connection task, the app if the main thread.
/// - Kind = `Error` (user decision 2026-10-07 「show a strong warning」).
/// - `dialog()` panics without plugin state (`Manager::state`), and a shell panic ends the app via the hook (ADR-0292); `lib.rs` registers the plugin unconditionally in the builder so production does not reach it — removing that registration turns this box into a panic.
/// - Failure is swallowed — the plugin already discards the `Err` of the main-thread delegation (same function).
/// The field is not `pub` for the same reason as [`super::events::TauriEmitter`].
pub(crate) struct TauriRefusalBox(pub(super) tauri::AppHandle);

impl RefusalNotice for TauriRefusalBox {
    fn show(&self, site: RefusalSite, daemon_message: &str) {
        // ADR-0373
        let lang = crate::ui_language::current(&self.0);
        let (title, body) = refusal_text(lang, site, daemon_message);
        self.0
            .dialog()
            .message(body)
            .title(title)
            .kind(MessageDialogKind::Error)
            .show(|_| {});
    }
}

/// An assembly that discards the box — harness only; `#[cfg(test)]` for the same reason as [`super::events::NoDaemonEvents`]: in a production build it would let a client be assembled whose box never appears even when a rejection comes.
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

/// The rejection-box filter — the same (place, daemon text) reaches the port only once in this process. Key = (place, daemon text), lifetime = the shell process:
/// - A rejection is deterministic for the same build · same name, so it returns with the same text on every reconnect — showing it again adds nothing, and frequent reconnects (daemon restart · resume from sleep) would pile up windows; 「once per connection」 cannot stop this repetition.
/// - The daemon text carries the cause (an overlapping name · an overflowing size · the cap value), so a different incident is a different key and shows afresh.
/// - The place is in the key because the two rejections weigh differently (registration = shell commands 0 · delta = only the screen part's delta missing).
/// Cost: from the second occurrence of the same incident there is no box; the cause warn is printed by the caller every time.
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

    /// Called only for daemon-origin rejections (`CommandFailure::Daemon`) — the text is not read here to tell the origin apart. Shown / suppressed is left as one info log line — GUI measurement counts box decisions with it.
    pub(crate) fn surface(&self, site: RefusalSite, daemon_message: &str) {
        // ADR-0006: only the decision is inside the lock — the log · port call is after releasing the lock, so even if the port reenters this filter it does not deadlock.
        let first = sync::lock(&self.shown).insert((site, daemon_message.to_owned()));
        if first {
            tracing::info!(site = ?site, "command refusal box — shown");
            self.port.show(site, daemon_message);
        } else {
            tracing::info!(
                site = ?site,
                "command refusal box — suppressed (this process already showed the same site and text)"
            );
        }
    }
}

/// The box's (title, body) in `lang`. The daemon text is carried verbatim — the code prefix and the cause (overlapping name · overflowing size · cap value) are in it, so no per-code text is kept.
pub(crate) fn refusal_text(
    lang: Lang,
    site: RefusalSite,
    daemon_message: &str,
) -> (String, String) {
    (
        ui_text::refusal_title(lang).to_owned(),
        ui_text::refusal_body(lang, ui_text::refusal_lead(lang, site), daemon_message),
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

    // Inside `show`, call `surface` of the same filter again. Calling the port while holding the lock would make a std `Mutex` relock deadlock (or panic), so it waits a bounded time for the separately launched thread and ends in failure instead of hanging.
    struct Reentrant {
        alerts: OnceLock<Weak<RefusalAlerts>>,
        shown: Mutex<Vec<(RefusalSite, String)>>,
    }

    impl RefusalNotice for Reentrant {
        fn show(&self, site: RefusalSite, daemon_message: &str) {
            sync::lock(&self.shown).push((site, daemon_message.to_owned()));
            if let Some(alerts) = self.alerts.get().and_then(Weak::upgrade) {
                alerts.surface(RefusalSite::Update, "reentry");
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
            "reentry inside the port did not finish within 5 s — the port is called while holding the lock"
        );
        // The first reentrant key appears afresh and reaches the port again, and the second reentry inside it is the same key and is suppressed.
        assert_eq!(
            *sync::lock(&port.shown),
            vec![
                (RefusalSite::Register, "CONFLICT: agent.rename".to_owned()),
                (RefusalSite::Update, "reentry".to_owned()),
            ]
        );
    }

    // ── box text ──

    #[test]
    fn refusal_text_carries_the_daemon_message_verbatim_and_the_action() {
        let daemon_message =
            "CONFLICT: names already answered by the daemon: agent.rename, {x}\nsecond line";
        for site in [RefusalSite::Register, RefusalSite::Update] {
            let (title, body) = refusal_text(Lang::Ko, site, daemon_message);
            assert_eq!(title, "Engram — 명령 등록 거절"); // hangul-ok: the Korean UI table
            assert!(
                body.contains(&format!("\n\n데몬 답:\n{daemon_message}\n\n")), // hangul-ok: the Korean UI table
                "{site:?}: the daemon text must be carried verbatim — {body}"
            );
            assert!(
                body.ends_with("원인이 된 명령 선언을 고친 뒤 다시 빌드하세요."), // hangul-ok: the Korean UI table
                "{site:?}: the action line — {body}"
            );

            let (title, body) = refusal_text(Lang::En, site, daemon_message);
            assert_eq!(title, "Engram — Command registration refused");
            assert!(
                body.contains(&format!("\n\nDaemon reply:\n{daemon_message}\n\n")),
                "{site:?}: the daemon text verbatim — {body}"
            );
            assert!(
                body.ends_with("Fix the command declaration that caused it, then rebuild."),
                "{site:?}: the action line — {body}"
            );
        }
        let (_, register) = refusal_text(Lang::Ko, RefusalSite::Register, daemon_message);
        let (_, update) = refusal_text(Lang::Ko, RefusalSite::Update, daemon_message);
        assert!(register.starts_with("데몬이 이 셸의 명령 등록을 통째로 거절했습니다.\n")); // hangul-ok: the Korean UI table
        assert!(update.starts_with("데몬이 화면 명령 명단 갱신을 거절했습니다.\n")); // hangul-ok: the Korean UI table
        assert!(register.contains("(원인이 고쳐질 때까지)")); // hangul-ok: the Korean UI table
                                                              // The shell bridge holds the rejected name without reverting it, so with the declaration unchanged that name may be carried again on the next full registration and rejected. But if the webview reports a changed list again, or the rejection stands only on the delta (CONFLICT with someone else's name, etc.), it does not repeat — so "may be"; do not revert it to "shell commands stay as they are", nor to a definitive form. The English "may" is the same.
        assert!(update.contains(
            "원인을 고치지 않은 채 재연결하면 이 셸의 명령 등록 전체가 거절될 수 있습니다." // hangul-ok: the Korean UI table
        ));
        let (_, register) = refusal_text(Lang::En, RefusalSite::Register, daemon_message);
        let (_, update) = refusal_text(Lang::En, RefusalSite::Update, daemon_message);
        assert!(
            register.starts_with("The daemon refused all of this shell's command registrations.\n")
        );
        assert!(update.starts_with("The daemon refused to update the list of screen commands.\n"));
        assert!(register.contains("until the cause is fixed."));
        assert!(update.contains(
            "If the shell reconnects before the cause is fixed, the daemon may refuse all of its command registrations."
        ));
    }
}
