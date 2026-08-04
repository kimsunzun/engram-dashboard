//! 상태 → wire 팬아웃(ADR-0129 — 에이전트 시스템 행).
//!
//! `AgentManager` 에 주입되는 **전역 `StatusSink`** 하나만 산다. 코어가 동기 스레드에서 밀어 올린
//! 상태 사실(status_changed / agent_list_updated / restore_result)을 `AgentEvent` JSON 으로 인코딩해
//! 연결 레지스트리의 **모든** 연결로 논블록 fanout 한다.
//!
//! ★왜 에이전트 시스템 쪽인가(분리 축)★: 이 파일은 코어 어휘(`AgentId`·`AgentStatus`·`AgentInfo`·
//!   `RestoreReport`)와 wire 어휘(`AgentEvent`)를 **둘 다** 안다 — 네트워크 행이 타입으로도 알아선
//!   안 되는 것들이다(ADR-0129 결정 1). 네트워크 쪽에 남는 것은 불투명 text 를 받는 팬아웃 표면
//!   (`ws::ConnRegistry::broadcast_text`)뿐이고, 그 표면의 포트화는 결정 3(얇은 조립)에서 배선된다
//!   (ADR-0129 2026-08-04 note — 구멍은 연결당·전-연결 두 모양이다).
//! ★여기서 정책이 바뀌지 않는다★: terminal 판정·이벤트 합성은 코어 몫이고(ADR-0005 상태 알림 분담)
//!   이 sink 는 코어가 부른 것을 인코딩해 흘리기만 한다 — 판정하지도, 이벤트를 만들어 내지도 않는다.
//!   코어가 상태를 **한 갈래로만** 밀어 올리기로 한 구조(ADR-0028 — 기능마다 별도 관측자를 달지 않는다)의
//!   그 한 갈래 끝이 여기다.
//! ★"이벤트당 1회 송신" 이 아니다(오독 주의)★: 인코딩은 이벤트당 1회지만 **송신은 등록된 연결 수만큼**
//!   이다 — `ConnRegistry::broadcast_text` 가 연결마다 try_send 한다. ADR-0028 은 *푸시 갈래가 하나*라는
//!   결정이지 *전송이 한 번*이라는 보장이 아니다. 느린 연결 하나가 실패해도 나머지는 그대로 나간다.
// ADR-0129
// ADR-0005
// ADR-0028

use engram_dashboard_core::agent::profile::RestoreReport as CoreRestoreReport;
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo as CoreAgentInfo, AgentStatus as CoreStatus, StatusSink,
};
use engram_dashboard_protocol::AgentEvent;

use crate::connection_core::{
    core_agents_to_wire, core_report_to_wire, core_status_to_wire, event_json,
};
use crate::ws::ConnRegistry;

// ── DaemonStatusSink(global) ─────────────────────────────────────────────────────

/// AgentManager 에 주입되는 전역 StatusSink. status_changed/agent_list_updated/restore_result
/// 를 AgentEvent JSON 으로 직렬화해 레지스트리의 모든 conn_tx 에 try_send(Text) 한다.
/// (LogStatusSink 대체 — build_manager 가 이걸 주입.)
///
/// ★호출 컨텍스트: pump/manager 의 동기 스레드★ → 절대 block 금지. broadcast_text 가 try_send 만 쓴다.
pub struct DaemonStatusSink {
    registry: ConnRegistry,
}

impl DaemonStatusSink {
    pub fn new(registry: ConnRegistry) -> Self {
        Self { registry }
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
            self.registry.broadcast_text(text);
        }
    }

    fn agent_list_updated(&self, agents: Vec<CoreAgentInfo>) {
        let ev = AgentEvent::AgentListUpdated {
            agents: core_agents_to_wire(agents),
        };
        if let Some(text) = event_json(&ev) {
            self.registry.broadcast_text(text);
        }
    }

    fn restore_result(&self, report: CoreRestoreReport) {
        let ev = AgentEvent::RestoreResult {
            report: core_report_to_wire(report),
        };
        if let Some(text) = event_json(&ev) {
            self.registry.broadcast_text(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_port::Frame;
    use engram_dashboard_core::agent::profile::RestoreOutcome as CoreRestoreOutcome;
    use engram_dashboard_core::agent::types::{
        Capabilities, ControlCaps, InputCaps, ModelCaps, OutputCaps, SessionCaps,
    };
    use serde_json::json;
    use tokio::sync::mpsc;

    /// 소켓 없는 격리 하네스 — 연결 n개가 등록된 레지스트리와 각 연결의 수신단.
    /// (등록 경로는 ws 모듈 private 이라 `register_for_test` 로 심는다 — 그 함수 주석이 근거.)
    fn registry_with(n: usize) -> (ConnRegistry, Vec<mpsc::Receiver<Frame>>) {
        let registry = ConnRegistry::new();
        let mut rxs = Vec::new();
        for _ in 0..n {
            let (tx, rx) = mpsc::channel::<Frame>(8);
            registry.register_for_test(tx);
            rxs.push(rx);
        }
        (registry, rxs)
    }

    /// 이 연결이 받은 **유일한** Text 프레임의 원문. 프레임이 0개거나 2개 이상이면 실패 —
    /// "연결당 정확히 1프레임" 이 이 테스트들의 관심사라 개수를 여기서 못 박는다.
    /// ★원문(String)을 돌려주는 이유★: 호출자가 연결 간 **바이트 동일성**까지 볼 수 있게(파싱하면
    ///   그 정보가 사라진다 — 연결마다 다르게 만들어 보내는 회귀를 못 잡는다).
    fn sole_text(rx: &mut mpsc::Receiver<Frame>) -> String {
        let first = rx.try_recv().expect("등록된 연결은 이벤트를 받아야");
        let text = match first {
            Frame::Text(s) => s,
            other => panic!("status fanout 은 Text 프레임이어야: {other:?}"),
        };
        assert!(
            rx.try_recv().is_err(),
            "이벤트 1건은 연결당 정확히 1프레임이어야(중복 송신 회귀)"
        );
        text
    }

    fn sole_event(rx: &mut mpsc::Receiver<Frame>) -> serde_json::Value {
        serde_json::from_str(&sole_text(rx)).expect("wire 는 JSON")
    }

    /// 테스트용 core AgentInfo — 로스터 인코딩 경로에 태울 최소 스냅샷(상태는 인자로).
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
    fn a_status_change_reaches_every_connection_as_one_identical_envelope() {
        // ★이 sink 의 계약★: 코어 사실 1건 → wire 봉투 1개 → **등록된 연결 전부**에 같은 바이트로.
        //   ★봉투 **전체**와 비교한다★: 고른 필드만 보면 필드 추가·미검사 필드 오염이 통과한다.
        //   ★payload 를 지닌 상태(`Exited{code}`)를 쓰는 이유★: 유닛 variant 만 태우면 상태 payload 를
        //     떨어뜨리는 회귀가 안 잡힌다.
        //   ★"인코딩 1회" 는 단언하지 않는다(관측 불가 — 연결마다 다시 인코딩해도 결과 문자열이 같다)★.
        //     대신 관측 가능한 것을 못 박는다: 연결들이 받은 원문이 **서로 바이트 동일**하다.
        let (registry, mut rxs) = registry_with(3);
        let sink = DaemonStatusSink::new(registry);
        let id = AgentId::new_v4();

        sink.status_changed(id, CoreStatus::Exited { code: Some(3) }, 7);

        let texts: Vec<String> = rxs.iter_mut().map(sole_text).collect();
        assert!(
            texts.windows(2).all(|w| w[0] == w[1]),
            "연결마다 다른 바이트가 나갔다(단일 인코딩 결과를 그대로 fanout 해야): {texts:?}"
        );
        let got: serde_json::Value = serde_json::from_str(&texts[0]).expect("wire 는 JSON");
        assert_eq!(
            got,
            json!({
                "StatusChanged": {
                    "agent_id": id.to_string(),
                    // 상태는 **내부 태깅**(`#[serde(tag = "type")]`) — 프론트가 discriminated union 으로 받는다.
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
        // ★배열 전체를 비교한다★: 길이+몇 필드만 보면 잘림·필드 오염이 통과한다.
        let (registry, mut rxs) = registry_with(1);
        let sink = DaemonStatusSink::new(registry);
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
            sole_event(&mut rxs[0]),
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
        //   Uuid→String 변환까지 지나므로 그 경계도 여기서 함께 못 박힌다(연결 코어 주석 참조).
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
            let (registry, mut rxs) = registry_with(1);
            let sink = DaemonStatusSink::new(registry);
            let id = AgentId::new_v4();
            let epoch = idx as u32;

            sink.restore_result(CoreRestoreReport {
                agent_id: id,
                epoch,
                outcome,
            });

            assert_eq!(
                sole_event(&mut rxs[0]),
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

    #[test]
    fn a_full_connection_does_not_stop_the_others() {
        // ★fanout 은 연결 하나의 포화에 걸리지 않는다★: 느린 소비자 처리는 네트워크 행(try_send 실패 시
        //   경고 + 그 연결 종료 신호)의 몫이고, 이 sink 는 나머지 연결 배달을 계속해야 한다. 막히면
        //   슬로우 클라 하나가 전 클라의 상태 갱신을 세운다.
        //
        // ★반복하는 이유 = 매 회차 HashMap 순회 순서를 다시 뽑으려는 것(패딩 아님)★: `broadcast_text` 는
        //   레지스트리 맵을 순회해 Vec 으로 뜨므로 방문 순서가 곧 배달 순서다.
        //   - **정상 코드는 순서와 무관하게 통과한다** → 이 루프가 위양성(flake)을 만들 수는 없다. 반복이
        //     바꾸는 것은 오직 **탐지력**이다.
        //   - 잡으려는 회귀 = "첫 try_send 실패에서 fanout 중단". 그 회귀는 포화 연결이 **마지막에** 방문된
        //     회차에서는 멀쩡한 연결들이 이미 다 받은 뒤라 살아남는다(S18.17 이 기록한 "포화 경로를 한 번도
        //     안 밟고 통과" 와 같은 부류). 그래서 회차마다 **새 레지스트리**로 순서를 다시 뽑는다.
        //     ※ 같은 레지스트리를 재사용하면 순서가 고정돼 반복이 무의미하다(그래서 루프 **안**에서 만든다).
        // ★탐지력은 경험적이지 증명이 아니다(정직 명시)★: std 는 `RandomState` 가 인스턴스마다 다른 씨앗을
        //   쓴다고만 하고, **인스턴스 간 순서 독립성도 특정 분포도 보장하지 않는다**. 그러니 "K회면 놓칠 확률
        //   2^-K" 같은 계산을 여기 적을 근거가 없다 — 그건 관측을 보장으로 격상하는 것이다.
        //   ★실측(2026-08-04 · 이 형태 = 포화 1 + 멀쩡 2)★: 위 회귀를 심고 **10회 시도 전부** 잡혔다.
        //   잡히는 회차(round 0~1)도, 굶은 연결(ok0/ok1)도 실행마다 달랐다 — 순서가 실제로 매 회차 다시
        //   뽑힌다는 증거. **보장이 아니라 측정치다.**
        // ★결정적 탐지를 원하면 순회 순서를 통제해야 한다★ = `ConnRegistry` 의 맵 타입 교체(정렬 맵 등).
        //   이사 슬라이스(ADR-0129)의 범위 밖이라 하지 않는다 — 이게 **하드 보장**이어야 할 날이 오면 그때
        //   그 교체가 정공법이고, 그 전까지 K 는 탐지력 손잡이일 뿐이다(임계값 튜닝 대상 아님 — ADR-0038).
        // ★멀쩡한 연결을 2개 두는 이유★: 포화가 **가운데**에 오는 배치까지 덮는다. 회귀가 한 회차를
        //   살아남으려면 포화 연결이 **맨 뒤**에 와야 하는데, 멀쩡한 연결이 1개면 "뒤" 가 두 자리 중
        //   하나이고 2개면 세 자리 중 하나다 — 즉 회차당 생존 여지가 좁아진다(분포 보장이 없으므로
        //   이것도 확률 계산이 아니라 **자리 수 논증**이다).
        const K: usize = 20;
        for round in 0..K {
            let registry = ConnRegistry::new();
            let (full_tx, mut full_rx) = mpsc::channel::<Frame>(1);
            full_tx
                .try_send(Frame::Text("선점".into()))
                .expect("cap 1 을 미리 채운다");
            registry.register_for_test(full_tx);
            let mut oks: Vec<mpsc::Receiver<Frame>> = (0..2)
                .map(|_| {
                    let (ok_tx, ok_rx) = mpsc::channel::<Frame>(8);
                    registry.register_for_test(ok_tx);
                    ok_rx
                })
                .collect();

            let sink = DaemonStatusSink::new(registry);
            sink.status_changed(AgentId::new_v4(), CoreStatus::Killed, 0);

            // 포화 연결엔 새 이벤트가 못 들어갔다(선점 프레임만 남아 있다).
            assert!(
                matches!(full_rx.try_recv(), Ok(Frame::Text(s)) if s == "선점"),
                "round {round}: 포화 연결엔 선점 프레임만 있어야"
            );
            assert!(
                full_rx.try_recv().is_err(),
                "round {round}: 포화 연결은 이번 건을 못 받는다"
            );
            // 그래도 멀쩡한 연결은 **전부** 받는다 — 포화 연결의 방문 위치와 무관하게.
            // ★여기서 `sole_text` 를 안 쓰는 이유★: 이 테스트가 잡는 회귀(첫 포화에서 fanout 중단)는
            //   "아무것도 못 받음" 으로 나타나므로, 그 갈래에 **회차·연결 번호와 원인**을 담은 메시지를
            //   붙인다(공용 헬퍼의 일반 메시지로는 어느 회차가 걸렸는지 안 보인다).
            for (n, ok_rx) in oks.iter_mut().enumerate() {
                let v = match ok_rx.try_recv() {
                    Ok(Frame::Text(s)) => {
                        serde_json::from_str::<serde_json::Value>(&s).expect("wire 는 JSON")
                    }
                    other => panic!(
                        "round {round}/ok{n}: 멀쩡한 연결이 이벤트를 못 받았다(fanout 이 첫 실패에서 멈춘 회귀): {other:?}"
                    ),
                };
                assert!(
                    ok_rx.try_recv().is_err(),
                    "round {round}/ok{n}: 연결당 정확히 1프레임"
                );
                assert_eq!(
                    v["StatusChanged"]["status"],
                    json!({ "type": "Killed" }),
                    "round {round}/ok{n}: 한 연결의 포화가 다른 연결의 fanout 을 막지 않는다"
                );
            }
        }
    }
}
