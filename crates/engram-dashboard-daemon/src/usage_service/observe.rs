//! A [`StatusSink`] decorator that hands pickup observations straight to the usage service (TRD §1-4 「줍기 경로」 (pickup path) · ADR-0247) — without piling them up,
//! it calls [`UsageService::observe`] on the calling thread (the output pump).
//!
//! ★Pass every hook through to the wrapped sink as is — usage observation too★. The hook's default implementation is a no-op so if one is missed, even if the inner one
//!   implements that hook, it quietly dies with no compile error — when hooks grow, add one line here too (the same decorator contract as
//!   `MessagingFlushSink`).

use std::sync::Arc;

use engram_dashboard_agent::profile::RestoreReport;
use engram_dashboard_agent::types::{AgentId, AgentInfo, AgentStatus, StatusSink};
use engram_dashboard_agent::usage::UsageObservation;

use super::UsageService;

pub struct UsageObserveSink {
    inner: Box<dyn StatusSink>,
    service: Arc<UsageService>,
}

impl UsageObserveSink {
    pub fn new(inner: Box<dyn StatusSink>, service: Arc<UsageService>) -> Self {
        Self { inner, service }
    }
}

impl StatusSink for UsageObserveSink {
    fn status_changed(&self, id: AgentId, status: AgentStatus, epoch: u32) {
        self.inner.status_changed(id, status, epoch);
    }

    fn agent_list_updated(&self, agents: Vec<AgentInfo>) {
        self.inner.agent_list_updated(agents);
    }

    fn restore_result(&self, report: RestoreReport) {
        self.inner.restore_result(report);
    }

    fn usage_observed(&self, obs: UsageObservation) {
        self.service.observe(&obs);
        self.inner.usage_observed(obs);
    }

    fn turn_ended(&self, id: AgentId, epoch: u32) {
        self.inner.turn_ended(id, epoch);
    }

    fn inputs_drained(&self, id: AgentId, epoch: u32) {
        self.inner.inputs_drained(id, epoch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage_service::tests::{far_reset, five, last_pct, rig};
    use engram_dashboard_agent::profile::RestoreOutcome;
    use std::sync::Mutex;

    /// Write what each hook received in one line — ★implement every hook by hand without leaning on the default implementation★ (if leaning, this test would not know even if that hook were not
    /// passed on). When hooks grow, one line each here and in the test below.
    struct Recording(Arc<Mutex<Vec<String>>>);

    impl Recording {
        fn note(&self, line: String) {
            self.0.lock().unwrap().push(line);
        }
    }

    impl StatusSink for Recording {
        fn status_changed(&self, id: AgentId, status: AgentStatus, epoch: u32) {
            self.note(format!("status_changed {id} {status:?} {epoch}"));
        }

        fn agent_list_updated(&self, agents: Vec<AgentInfo>) {
            self.note(format!("agent_list_updated {}", agents.len()));
        }

        fn restore_result(&self, report: RestoreReport) {
            self.note(format!(
                "restore_result {} {} {:?}",
                report.agent_id, report.epoch, report.outcome
            ));
        }

        fn usage_observed(&self, obs: UsageObservation) {
            self.note(format!("usage_observed {obs:?}"));
        }

        fn turn_ended(&self, id: AgentId, epoch: u32) {
            self.note(format!("turn_ended {id} {epoch}"));
        }

        fn inputs_drained(&self, id: AgentId, epoch: u32) {
            self.note(format!("inputs_drained {id} {epoch}"));
        }
    }

    #[test]
    fn every_hook_reaches_the_inner_sink_unchanged() {
        let (rig, _clock) = rig();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = UsageObserveSink::new(Box::new(Recording(seen.clone())), rig.service.clone());
        let id = AgentId::new_v4();
        let obs = five(0, 40.0, far_reset());

        sink.status_changed(id, AgentStatus::Killed, 3);
        sink.agent_list_updated(Vec::new());
        sink.restore_result(RestoreReport {
            agent_id: id,
            epoch: 4,
            outcome: RestoreOutcome::Resumed,
        });
        sink.usage_observed(obs.clone());
        sink.turn_ended(id, 5);
        sink.inputs_drained(id, 6);

        assert_eq!(
            *seen.lock().unwrap(),
            [
                format!("status_changed {id} Killed 3"),
                "agent_list_updated 0".to_owned(),
                format!("restore_result {id} 4 Resumed"),
                format!("usage_observed {obs:?}"),
                format!("turn_ended {id} 5"),
                format!("inputs_drained {id} 6"),
            ]
        );
    }

    #[test]
    fn an_observation_reaches_the_service_at_once() {
        let (rig, _clock) = rig();
        let log = rig.subscriber(1, &[0]);
        let sink = UsageObserveSink::new(
            Box::new(Recording(Arc::new(Mutex::new(Vec::new())))),
            rig.service.clone(),
        );
        sink.usage_observed(five(0, 40.0, far_reset()));
        assert_eq!(log.count(), 1, "쌓지 않고 그 자리에서 발행까지");
        assert_eq!(last_pct(&log), Some(40.0));
    }
}
