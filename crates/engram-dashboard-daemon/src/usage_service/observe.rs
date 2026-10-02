//! 줍기 관측을 사용량 서비스로 곧장 넘기는 [`StatusSink`] decorator(TRD §1-4 「줍기 경로」 · ADR-0247) — 쌓아 두지 않고
//! 부른 스레드(출력 pump)에서 [`UsageService::observe`] 를 부른다.
//!
//! ★모든 훅을 감싼 sink 로 그대로 넘긴다 — 사용량 관측도★. 훅은 기본 구현이 no-op 이라 하나를 빠뜨리면 안쪽이
//!   그 훅을 구현해도 컴파일 에러 없이 조용히 죽는다 — 훅이 늘면 여기에도 한 줄을 더한다(`MessagingFlushSink` 와
//!   같은 decorator 계약).

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

    /// 훅마다 받은 것을 한 줄로 적는다 — ★기본 구현에 기대지 않고 훅을 전부 손으로 구현한다★(기대면 그 훅을 안
    /// 넘겨도 이 시험이 모른다). 훅이 늘면 여기와 아래 시험에 한 줄씩.
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
