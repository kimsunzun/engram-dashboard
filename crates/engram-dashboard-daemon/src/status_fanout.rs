//! 상태 → wire 팬아웃(ADR-0129 — 에이전트 시스템 행).
//!
//! `AgentManager` 에 주입되는 **전역 `StatusSink`** 하나만 산다. 코어가 동기 스레드에서 밀어 올린
//! 상태 사실(status_changed / agent_list_updated / restore_result)을 `AgentEvent` JSON 으로 인코딩해
//! 팬아웃 포트(`engram_dashboard_net::frame_port::FrameFanout`)로 한 번 밀어 넣는다.
//!
//! ★왜 에이전트 시스템 쪽인가(분리 축)★: 이 파일은 코어 어휘(`AgentId`·`AgentStatus`·`AgentInfo`·
//!   `RestoreReport`)와 wire 어휘(`AgentEvent`)를 **둘 다** 안다 — 네트워크 행이 타입으로도 알아선
//!   안 되는 것들이다(ADR-0129 결정 1). 반대로 이 파일은 연결이 몇 개인지·누가 등록돼 있는지를
//!   모른다: 포트 너머로 넘길 수 있는 것은 불투명 text 하나뿐이다
//!   (ADR-0129 2026-08-04 note — 구멍은 연결당·전-연결 두 모양이다).
//! ★여기서 정책이 바뀌지 않는다★: terminal 판정·이벤트 합성은 코어 몫이고(ADR-0005 상태 알림 분담)
//!   이 sink 는 코어가 부른 것을 인코딩해 흘리기만 한다 — 판정하지도, 이벤트를 만들어 내지도 않는다.
//!   코어가 상태를 **한 갈래로만** 밀어 올리기로 한 구조(ADR-0028 — 기능마다 별도 관측자를 달지 않는다)의
//!   그 한 갈래 끝이 여기다.
//! ★"이벤트당 1회 송신" 이 아니다(오독 주의)★: 이 파일에서의 인코딩·팬아웃 호출은 이벤트당 1회지만
//!   **실제 송신은 등록된 연결 수만큼**이다 — 포트 구현이 연결마다 try_send 한다. ADR-0028 은 *푸시
//!   갈래가 하나*라는 결정이지 *전송이 한 번*이라는 보장이 아니다.
// ADR-0129
// ADR-0005
// ADR-0028

use engram_dashboard_agent::profile::RestoreReport as CoreRestoreReport;
use engram_dashboard_agent::types::{
    AgentId, AgentInfo as CoreAgentInfo, AgentStatus as CoreStatus, StatusSink,
};
use engram_dashboard_protocol::AgentEvent;

use crate::connection_core::{
    core_agents_to_wire, core_report_to_wire, core_status_to_wire, event_json,
};
use engram_dashboard_net::frame_port::FrameFanout;
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
        serde_json::from_str(&fanout.sole_text()).expect("wire 는 JSON")
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

    /// 위 `info` fixture 가 wire 로 나왔을 때의 **전체** 모양. 필드를 하나도 빼지 않고 적는다 —
    /// 부분 단언은 검사 안 한 필드의 오염·추가 필드를 통과시킨다(그게 이 헬퍼의 존재 이유).
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
        // ★payload 를 지닌 상태(`Exited{code}`)를 쓰는 이유★: 유닛 variant 만 태우면 상태 payload 를
        //   떨어뜨리는 회귀가 안 잡힌다.
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
        // ★이 sink 는 로스터를 **거르지 않는다**(load-bearing)★: 산 것만 추리는 술어(`is_live`)는 배달
        //   판정 쪽 관심사고, 프론트 목록은 죽은 것까지 봐야 terminal 전이를 판정한다(ADR-0005 —
        //   프론트는 status_changed 가 아니라 이 목록으로 terminal 을 판정한다). 그래서 fixture 에
        //   **비-Running 을 섞는다** — 여기에 필터가 생기면 프론트가 종료를 영영 못 본다.
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
            "목록은 받은 순서·내용 그대로 실린다(필터링·잘림·재정렬 없음)"
        );
    }

    #[test]
    fn every_restore_outcome_is_forwarded_with_its_payload() {
        // ★결말 **전 variant** 를 태운다★: 한 종류만 태우면 `Resumed` 를 하드코딩한 구현이 통과하고,
        //   payload 를 지닌 결말(FreshFallback/Failed/Blocked)이 깨져도 안 잡힌다. FreshFallback 은
        //   Uuid→String 변환까지 지나므로 그 경계도 여기서 함께 못 박힌다.
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
                    reason: "fresh 실패".into(),
                },
                json!({ "type": "Failed", "reason": "fresh 실패" }),
            ),
            (
                CoreRestoreOutcome::FreshFallback {
                    old_sid: Some(old),
                    new_sid: new,
                    reason: "resume 조기 종료".into(),
                },
                json!({
                    "type": "FreshFallback",
                    "old_sid": old.to_string(),
                    "new_sid": new.to_string(),
                    "reason": "resume 조기 종료"
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
}
