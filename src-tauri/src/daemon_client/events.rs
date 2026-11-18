//! The seam for the connection driver's notices to the frontend — real = `AppHandle::emit`, harness = a recording fake or a no-op.
//! Why this file exists (ADR-0012 · sibling seam = [`super::inbound::ViewCommandPort`]): the only reason a connection driver held an `AppHandle` was emitting, and a real `AppHandle` (= `AppHandle<Wry>`) cannot be built without a window — `tauri::test::mock_app()` gives a `MockRuntime`, a different type, and making the client generic over `R: Runtime` spreads to every command signature naming `AppHandle`. So only the emits are cut into a port; the driver then stands on a socket (or a memory link) alone, and the harness measures handshake, reconnect and round trips on the real code. Sibling ports = [`super::refusal`] (command refusal OS box) · [`super::AgentNamesSink`] (tab name book) — cut separately rather than widening this port (no widening — [`DaemonEvents`] doc below).
//! What the port measures: what the driver tells the frontend, not whether that notice reaches a window (the real `AppHandle`'s job, whose standing-up costs the price above — not impossible, a price chosen not to pay). Plug a recording fake through the client's test constructor (`new_on_link`) to assert what fires in which order over a round trip on a memory network.
//! Naming rule: Tauri event names (`"agent-list-updated"` etc.) belong to the adapter — the same split as `crate::layout::LayoutEvents` / `crate::commands::layout::TauriEvents`; the caller says only what happened.
// ADR-0012

use engram_dashboard_protocol::{
    AgentId, AgentInfo, AgentProfile, AgentStatus, Preset, RestoreReport, UsageLimitSnapshot,
};
use tauri::Emitter;

use super::ConnectionStatus;

/// Notification port from the daemon connection to the frontend. Called only while no lock is held (ADR-0006). The implementation swallows failures: a window that does not exist yet or a closed channel is no reason to roll back the connection; the frontend recovers on the next broadcast or re-query (same contract as `crate::layout::LayoutEvents`).
/// These seven are all the emits the connection task makes — do not widen it: the port would stop being "what the connection task tells the frontend" and become an "AppHandle stand-in", dragging in things that cannot be set up without a window. The seventh ([`Self::usage_limits_updated`]) escapes that warning: same kind (a 1:1 relay of one daemon broadcast), and the added `labels` · `socket_epoch` are only what the call site hands over — a pure module's result and its own socket marker; matching and re-sending live outside the port (the call site and `super::usage_interest`), the port receives only computed values (TRD S21 usage-limit-slot §6 #23).
pub(crate) trait DaemonEvents: Send + Sync {
    /// The link's connection status changed — `(state, band)`, either part. The payload is the frontend contract: the serialized [`ConnectionStatus`] (`src/api/tauriTransport.ts` parses it; the `daemon_connection_state` pull returns the same value).
    fn connection_status(&self, status: &ConnectionStatus);

    /// The full agent list was updated. The frontend's terminal decision stands on this alone (not status_changed — ADR-0005 「status notification split」).
    fn agent_list_updated(&self, agents: &[AgentInfo]);

    /// ADR-0163
    fn status_changed(&self, agent_id: &AgentId, status: &AgentStatus, epoch: u32);

    fn restore_result(&self, report: &RestoreReport);

    fn profile_list_updated(&self, profiles: &[AgentProfile]);

    /// ADR-0061
    fn preset_list_updated(&self, presets: &[Preset]);

    /// One usage snapshot — once per daemon broadcast. `labels` = the windows that receive it (those with a usage slot — never empty), `socket_epoch` = marker of the socket it came from (ADR-0195 · the receiver forgets the revision only when this grows). Choosing the window is the receiving webview's job — it is sent to all webviews.
    fn usage_limits_updated(
        &self,
        snapshot: &UsageLimitSnapshot,
        labels: &[String],
        socket_epoch: u64,
    );
}

/// Payload of `"usage-limits-updated"` — the field names are the contract with the webview (`{labels, socket_epoch, snapshot}` of TRD S21 usage-limit-slot §1-8); change the receiving side together.
#[derive(serde::Serialize, Clone)]
struct UsageLimitsUpdatedPayload<'a> {
    labels: &'a [String],
    socket_epoch: u64,
    snapshot: &'a UsageLimitSnapshot,
}

/// Production adapter — pushes to all webviews through the real `AppHandle`. Every `Err` from `emit` is swallowed (trait contract above). It owns the handle because the connection task is `'static` and lives across reconnects. The field is deliberately not `pub`: the only assembler is the production constructor (`super::DaemonClient::new_real_with_owned_runtime`, same parent module) — opened outward, a path would appear taking the `AppHandle` back out of this type and it would stop being a seam and become an `AppHandle` carrier.
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

    fn preset_list_updated(&self, presets: &[Preset]) {
        let _ = self.0.emit("preset-list-updated", presets);
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

/// Assembly that drops notifications — harness only. Dropping is the contract: a harness plugged with this does not measure emit content (to measure firing pass a recording fake through `super::DaemonClient::new_on_link`); losing notifications touches no invariant of connection, reconnection or command round trip — frontend notifications are one-way and the connection task does not read their success (the real adapter swallows `Err` too).
/// `#[cfg(test)]` is the point: left in a production build, it opens a way to quietly assemble a "client that drops notifications" — socket and command round trips all normal while only the screen is not updated, and the frontend's self-recovery fills only the connection state, not `agent-list-updated` (`tauriTransport.ts` Fix-D).
#[cfg(test)]
pub(crate) struct NoDaemonEvents;

#[cfg(test)]
impl DaemonEvents for NoDaemonEvents {
    fn connection_status(&self, _status: &ConnectionStatus) {}
    fn agent_list_updated(&self, _agents: &[AgentInfo]) {}
    fn status_changed(&self, _agent_id: &AgentId, _status: &AgentStatus, _epoch: u32) {}
    fn restore_result(&self, _report: &RestoreReport) {}
    fn profile_list_updated(&self, _profiles: &[AgentProfile]) {}
    fn preset_list_updated(&self, _presets: &[Preset]) {}
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

    // Webview contract = three snake_case fields — the receiving side (`usage-limits-updated` receive in `src/api/tauriTransport.ts`) reads by these names; paired with the field-name test of the shell cache pull return (`connection/tests.rs`).
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
            "webview contract = three snake_case fields"
        );
        assert_eq!(wire["labels"], serde_json::json!(["main"]));
        assert_eq!(wire["socket_epoch"], 7);
        assert_eq!(wire["snapshot"]["revision"], 3);
    }

    // The `daemon-connection-state` payload and the pull reply, byte for byte — `src/api/tauriTransport.ts` reads these words and keys; `tauriTransport.test.ts` feeds the same shapes as objects.
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
