//! State → wire fan-out (ADR-0129 — agent system layer).
//!
//! Only one global `StatusSink` injected into `AgentManager` lives here. It encodes the state facts the core pushes up from a sync thread (status_changed / agent_list_updated / restore_result) as `AgentEvent` JSON and pushes them once into the fan-out port (`crate::conn_port::FrameFanout`).
//!
//! ★Why the agent system side★: this file knows both the core vocabulary (`AgentId`·`AgentStatus`·`AgentInfo`·`RestoreReport`) and the wire vocabulary (`AgentEvent`) — which the network layer must not know even by type (ADR-0129 decision 1). It does not know how many connections exist; only one opaque text crosses the port (ADR-0129 — the holes have two shapes: per-connection and all-connections).
//! ★Policy does not change here★: the terminal decision and event synthesis belong to the core (ADR-0005 status notification split); this sink only encodes what the core called and lets it flow. It is the end of the single branch where the core pushes state up (ADR-0028 — no per-feature observer).
//! ★Not "one send per event"★: encoding and the fan-out call are once per event, but actual sending is as many as the registered connections (the port implementation does try_send per connection). ADR-0028 decides that there is one push branch, not that sending happens once.
// ADR-0129
// ADR-0005
// ADR-0028

use engram_dashboard_agent::profile::RestoreReport as CoreRestoreReport;
use engram_dashboard_agent::types::{
    AgentId, AgentInfo as CoreAgentInfo, AgentStatus as CoreStatus, StatusSink,
};
use engram_dashboard_protocol::AgentEvent;

use crate::conn_port::FrameFanout;
use crate::connection_core::{
    core_agents_to_wire, core_report_to_wire, core_status_to_wire, event_json,
};
use std::sync::Arc;

// ── DaemonStatusSink(global) ─────────────────────────────────────────────────────

pub struct DaemonStatusSink {
    fanout: Arc<dyn FrameFanout>,
}

impl DaemonStatusSink {
    pub fn new(fanout: Arc<dyn FrameFanout>) -> Self {
        Self { fanout }
    }
}

impl StatusSink for DaemonStatusSink {
    fn status_changed(&self, id: AgentId, status: CoreStatus, epoch: u32) {
        let ev = AgentEvent::StatusChanged {
            agent_id: id,
            status: core_status_to_wire(status),
            epoch,
        };
        if let Some(text) = event_json(&ev) {
            self.fanout.broadcast_text(text);
        }
    }

    fn agent_list_updated(&self, agents: Vec<CoreAgentInfo>) {
        let ev = AgentEvent::AgentListUpdated {
            agents: core_agents_to_wire(agents),
        };
        if let Some(text) = event_json(&ev) {
            self.fanout.broadcast_text(text);
        }
    }

    fn restore_result(&self, report: CoreRestoreReport) {
        let ev = AgentEvent::RestoreResult {
            report: core_report_to_wire(report),
        };
        if let Some(text) = event_json(&ev) {
            self.fanout.broadcast_text(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_doubles::RecordingFanout;
    use engram_dashboard_agent::profile::RestoreOutcome as CoreRestoreOutcome;
    use engram_dashboard_agent::types::{
        Capabilities, ControlCaps, InputCaps, ModelCaps, OutputCaps, SessionCaps,
    };
    use serde_json::json;

    fn sink_with_fanout() -> (DaemonStatusSink, Arc<RecordingFanout>) {
        let fanout = Arc::new(RecordingFanout::new());
        (DaemonStatusSink::new(fanout.clone()), fanout)
    }

    fn sole_event(fanout: &RecordingFanout) -> serde_json::Value {
        serde_json::from_str(&fanout.sole_text()).expect("wire is JSON")
    }

    fn info(id: AgentId, name: &str, epoch: u32, status: CoreStatus) -> CoreAgentInfo {
        CoreAgentInfo {
            id,
            name: name.to_string(),
            cwd: "C:\\work".to_string(),
            status,
            cols: 80,
            rows: 24,
            epoch,
            capabilities: Capabilities {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: true,
                    structured: false,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: true,
                    interrupt: false,
                    cancel: false,
                    graceful_shutdown: false,
                },
                session: SessionCaps {
                    resume: false,
                    snapshot: false,
                    cwd_env: true,
                },
                model: ModelCaps {
                    select: false,
                    temperature: false,
                    max_tokens: false,
                },
            },
            reads_messages: true,
        }
    }

    /// The whole shape of the `info` fixture above on the wire. Write it without dropping a field — a partial assertion lets pollution or an extra field through.
    fn expected_agent(
        id: AgentId,
        name: &str,
        epoch: u32,
        status: serde_json::Value,
    ) -> serde_json::Value {
        json!({
            "id": id.to_string(),
            "name": name,
            "cwd": "C:\\work",
            "status": status,
            "cols": 80,
            "rows": 24,
            "epoch": epoch,
            "capabilities": {
                "input": { "raw": true, "message": false, "attachment": false },
                "output": {
                    "terminal_bytes": true, "structured": false, "markdown": false,
                    "tool_events": false, "usage": false
                },
                "control": {
                    "resize": true, "interrupt": false, "cancel": false, "graceful_shutdown": false
                },
                "session": { "resume": false, "snapshot": false, "cwd_env": true },
                "model": { "select": false, "temperature": false, "max_tokens": false }
            }
        })
    }

    #[test]
    fn a_status_change_goes_out_as_exactly_one_whole_envelope() {
        // ★Why a state carrying a payload (`Exited{code}`)★: running only unit variants would not catch a regression that drops the state payload.
        let (sink, fanout) = sink_with_fanout();
        let id = AgentId::new_v4();

        sink.status_changed(id, CoreStatus::Exited { code: Some(3) }, 7);

        assert_eq!(
            sole_event(&fanout),
            json!({
                "StatusChanged": {
                    "agent_id": id.to_string(),
                    "status": { "type": "Exited", "code": 3 },
                    "epoch": 7
                }
            })
        );
    }

    #[test]
    fn a_roster_update_carries_the_whole_list_including_agents_that_are_not_running() {
        // ★This sink does not filter the roster (load-bearing)★: picking only the live (`is_live`) is the delivery decision side's concern, and the frontend list must see the dead too to decide a terminal transition (ADR-0005 — the frontend decides terminal by this list, not status_changed). So the fixture mixes in non-Running — a filter here would hide every exit from the frontend.
        let (sink, fanout) = sink_with_fanout();
        let (a, b, c) = (AgentId::new_v4(), AgentId::new_v4(), AgentId::new_v4());

        sink.agent_list_updated(vec![
            info(a, "alice", 0, CoreStatus::Running),
            info(b, "bob", 2, CoreStatus::Killed),
            info(
                c,
                "carol",
                1,
                CoreStatus::Failed {
                    message: "boom".into(),
                },
            ),
        ]);

        assert_eq!(
            sole_event(&fanout),
            json!({
                "AgentListUpdated": {
                    "agents": [
                        expected_agent(a, "alice", 0, json!({ "type": "Running" })),
                        expected_agent(b, "bob", 2, json!({ "type": "Killed" })),
                        expected_agent(c, "carol", 1, json!({ "type": "Failed", "message": "boom" })),
                    ]
                }
            }),
            "the list is carried in the received order and content as is (no filtering, truncation or reordering)"
        );
    }

    #[test]
    fn every_restore_outcome_is_forwarded_with_its_payload() {
        // ★Run all outcome variants★: one kind would pass an implementation hardcoding `Resumed` and miss a break in the payload-carrying outcomes (FreshFallback/Failed/Blocked). FreshFallback also passes the Uuid→String conversion, nailing that boundary.
        let old = uuid::Uuid::new_v4();
        let new = uuid::Uuid::new_v4();
        let cases = vec![
            (CoreRestoreOutcome::Resumed, json!({ "type": "Resumed" })),
            (CoreRestoreOutcome::Started, json!({ "type": "Started" })),
            (
                CoreRestoreOutcome::Blocked {
                    reason: "auto_restore=false".into(),
                },
                json!({ "type": "Blocked", "reason": "auto_restore=false" }),
            ),
            (
                CoreRestoreOutcome::Failed {
                    reason: "fresh failed".into(),
                },
                json!({ "type": "Failed", "reason": "fresh failed" }),
            ),
            (
                CoreRestoreOutcome::FreshFallback {
                    old_sid: Some(old),
                    new_sid: new,
                    reason: "resume exited early".into(),
                },
                json!({
                    "type": "FreshFallback",
                    "old_sid": old.to_string(),
                    "new_sid": new.to_string(),
                    "reason": "resume exited early"
                }),
            ),
        ];

        for (idx, (outcome, expected_outcome)) in cases.into_iter().enumerate() {
            let (sink, fanout) = sink_with_fanout();
            let id = AgentId::new_v4();
            let epoch = idx as u32;

            sink.restore_result(CoreRestoreReport {
                agent_id: id,
                epoch,
                outcome,
            });

            assert_eq!(
                sole_event(&fanout),
                json!({
                    "RestoreResult": {
                        "report": {
                            "agent_id": id.to_string(),
                            "epoch": epoch,
                            "outcome": expected_outcome
                        }
                    }
                })
            );
        }
    }

    /// ★Raw observations are not broadcast here★ — usage sends the one sheet the service merged only to connections subscribed to that vendor (`usage_service`); from here even unsubscribed connections would receive the pre-merge value.
    #[test]
    fn a_usage_observation_is_not_broadcast() {
        use engram_dashboard_agent::usage::{UsageObservation, UsageSource, UsageVendorKey};
        let (sink, fanout) = sink_with_fanout();

        sink.usage_observed(UsageObservation {
            vendor: UsageVendorKey::new("test-vendor"),
            five_hour: None,
            weekly: None,
            model_scoped: None,
            plan: None,
            source: UsageSource::Active,
            limits_unavailable: None,
        });

        assert!(fanout.texts().is_empty(), "{:?}", fanout.texts());
    }
}
