//! messaging_host — 메시징 커널(`engram-dashboard-messaging`)의 **호스트 어댑터 + 조립실**(ADR-0110).
//!
//! ★역할★: 커널이 소유한 계약(포트 trait)에 데몬의 실물을 꽂는다. 커널은 `AgentManager`·`OutputSink`·
//!   `ControlRegistry` 를 **타입으로도 모르므로**(완전 상호무지 — ADR-0110 결정 2), 그 셋을 아는 코드는
//!   전부 이 파일에 모인다:
//!     - `ManagerDeliveryPort` — `DeliveryPort`(주입·로스터·이름) → `AgentManager`.
//!     - `ManagerTapHost` + `TurnTapSink` — `TapHost`(턴 관측 배선) → `AgentManager` 출력 구독.
//!       ★여기가 백엔드 지식의 자리★: "어떤 출력 이벤트가 턴 진행이고 어떤 게 턴 종료인가" 는 claude
//!       stream-json 의 지식이라 커널이 아니라 이 어댑터가 안다(ADR-0110 결정 4 · ADR-0004 와 같은 결).
//!     - `ControlRegistry` 의 `ControlPlanePort` 구현 — 봉투 포맷 조회 + 배달 관측 적재.
//!     - 조립 헬퍼(`messaging_for_manager`/`messaging_for_manager_gated`/`busy_tracker_for_manager`) —
//!       옛 커널 편의 생성자(`MessagingService::for_manager*`·`BusyTracker::for_manager`)의 후계.
//!
//! ★불변식(load-bearing)★: 정책은 커널에, 어댑터는 얇게. busy 불변식(positive-knowledge-only·유령 busy
//!   차단·콜백 규율·상한 sweep)을 여기서 재구현하지 않는다 — 이 파일이 하는 일은 **타입 번역과 배선**
//!   뿐이다(ADR-0110 영향/불변식 "포트는 얇게, 정책은 lib에").
//!
//! tauri import 0(daemon crate).
// ADR-0110

use std::sync::Arc;

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::types::{
    AgentStatus, OutputEvent, OutputFrame, OutputPayload, OutputSink, SinkError, SinkId,
};
use engram_dashboard_messaging::busy::{
    BusyGate, BusyTracker, IdleNotifier, SubscribeError, TapHost, TurnProbe,
};
use engram_dashboard_messaging::envelope::{DeliveryObservation, EnvelopeFormat};
use engram_dashboard_messaging::service::{
    ControlPlanePort, DeliveryPort, InjectReceipt, LiveAgent, MessagingService,
};
use engram_dashboard_messaging::PeerId;

use crate::control::registry::ControlRegistry;

// ── 배달 어댑터 ────────────────────────────────────────────────────────────────────────────────

/// 운영 DeliveryPort — Arc<AgentManager> 얇은 래퍼. manager 공개 API 만 부른다(ADR-0006 각 호출이
///   내부에서 sessions lock 을 clone 후 즉시 해제하는 규율을 그대로 탄다).
pub struct ManagerDeliveryPort {
    manager: Arc<AgentManager>,
}

impl ManagerDeliveryPort {
    pub fn new(manager: Arc<AgentManager>) -> Self {
        Self { manager }
    }
}

/// core `WriteOutcome` → 커널 `InjectReceipt` 4필드 복사(ADR-0110 경계 번역 — 필드 의미 동일).
fn receipt(o: engram_dashboard_core::agent::types::WriteOutcome) -> InjectReceipt {
    InjectReceipt {
        bytes_requested: o.bytes_requested,
        bytes_written: o.bytes_written,
        msg_uuid: o.msg_uuid,
        epoch: o.epoch,
    }
}

impl DeliveryPort for ManagerDeliveryPort {
    fn inject(&self, to_id: PeerId, bytes: &[u8]) -> Result<InjectReceipt, String> {
        // manager.rs — 배달-경계 계측판. 완결성 = Ok/Err(InjectReceipt 주석).
        self.manager
            .write_stdin_observed(to_id, bytes)
            .map(receipt)
            .map_err(|e| e.to_string())
    }

    // ★옛 `inject_if_epoch`(epoch-조건부 주입) 어댑터는 제거됐다(ADR-0111 결정 6)★: 그 동사는 그룹 방송의
    //   incarnation 결박 전용이었고 결박 자체가 폐지됐다. core 의 `write_stdin_observed_if_epoch` 도 이제
    //   **소비자가 하나도 없다**(그 헤더가 그 사실을 명시한다 — 옛 "다른 소비자 소유" 서술은 거짓이었다).
    //   되살릴 일이 생기면 어댑터만 다시 얹으면 되지만, 그 전에 "발송 순간 화신에게만" 을 v2 개인 메일
    //   옵션으로 정식 재론해야 한다(spec §8).
    // ADR-0111

    /// ★정렬 = (이름, id) 오름차순(C4 리뷰 fix H · load-bearing — `@all` 결정성)★.
    ///
    /// ★왜 여기서 정렬하나★: `@all` 명단은 이 로스터를 **verbatim** 쓴다(groups.rs 계약 축 1 — 해석기가
    ///   정렬하면 안 된다). 그런데 manager 의 세션 저장소는 HashMap 이라 순회 순서가 실행마다 다르다 —
    ///   그대로 두면 같은 방송이 실행마다 **다른 주입 순서·다른 `results[]` 순서**를 낸다(발신 LLM 이 보는
    ///   회계가 흔들리고, 재현 불가한 순서 의존 버그를 숨긴다). 커널 단위 테스트는 Vec fake 라 이 비결정성을
    ///   **못 잡는다** — 그래서 결정성을 운영 구현체 쪽에 박는다.
    /// ★seam 계약은 그대로★: `DeliveryPort` 는 "산·도달 가능 로스터" 만 약속하고 순서는 구현 자유다. 여기서
    ///   정렬해도 해석기(`GroupSource`)의 verbatim 계약과 충돌하지 않는다 — 정렬 위치가 **소스 앞단**(로스터
    ///   생산 지점)이라 해석기는 여전히 받은 그대로 돌려준다.
    /// ★2차 키가 id 인 이유★: 동명 다수(dup-name)여도 순서가 안정되게 — 이름만으로는 두 항목의 상대 순서가
    ///   여전히 HashMap 순서에 좌우된다.
    fn live_reachable_agents(&self) -> Vec<LiveAgent> {
        // list_agents 스냅샷 1회 → 산(Running|Exiting) + structured(제어 채널 도달 가능)만.
        let mut live: Vec<LiveAgent> = self
            .manager
            .list_agents()
            .into_iter()
            .filter(|a| {
                matches!(a.status, AgentStatus::Running | AgentStatus::Exiting)
                    && a.capabilities.output.structured
            })
            .map(|a| LiveAgent {
                id: a.id,
                name: a.name,
                epoch: a.epoch,
            })
            .collect();
        live.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        live
    }

    fn canonical_name(&self, id: PeerId) -> Option<String> {
        self.manager.canonical_name(id)
    }
}

// ── 제어 평면 어댑터 ────────────────────────────────────────────────────────────────────────────

/// `ControlRegistry` 를 커널의 `ControlPlanePort` 로 노출한다 — 실사용 2메서드만 통과시키는 얇은 impl.
///   (커널은 registry 의 나머지 표면(토큰 발급·검증)을 알 이유가 없다 — ADR-0110 결정 3.)
impl ControlPlanePort for ControlRegistry {
    // ★자기 재귀 함정(load-bearing)★: 아래 `ControlRegistry::…(self)` 경로는 **고유(inherent) 메서드가
    //   트레잇 메서드보다 우선 해석**되기에 고유 쪽에 묶인다. 고유 메서드를 개명/제거하면 같은 경로가
    //   이 트레잇 메서드 **자신**에 조용히 재바인딩돼(컴파일 에러 없음) 배달 핫패스에서 무한 재귀 →
    //   스택 오버플로가 된다. registry 쪽 개명 시 여기를 반드시 함께 고칠 것.
    fn envelope_format(&self) -> EnvelopeFormat {
        ControlRegistry::envelope_format(self)
    }
    fn record_delivery(&self, obs: DeliveryObservation) {
        ControlRegistry::record_delivery(self, obs)
    }
}

// ── 턴 관측 어댑터 ──────────────────────────────────────────────────────────────────────────────

/// 운영 TapHost — `Arc<AgentManager>` 얇은 래퍼(공개 API 만 부른다, ADR-0006 락 규율 그대로 탄다).
pub struct ManagerTapHost {
    manager: Arc<AgentManager>,
}

impl ManagerTapHost {
    pub fn new(manager: Arc<AgentManager>) -> Self {
        Self { manager }
    }
}

impl TapHost for ManagerTapHost {
    fn subscribe_output(
        &self,
        id: PeerId,
        expect_epoch: u32,
        probe: Arc<TurnProbe>,
    ) -> Result<(), SubscribeError> {
        // ★구독 지점 epoch 검증(round-3 finding 5 — 유령 tap 누수 차단)★: core 에는 "이 epoch 이면 구독" 이라는
        //   원자적 API 가 없다(core 는 읽기 전용 — 새 seam 을 내지 않는다). 그래서 **구독 직전 + 직후** 두 번
        //   현재 epoch 을 확인하고, 사후 확인이 어긋나면 방금 받은 SinkId 로 **되돌린다**(unsubscribe). 이러면
        //   창이 남더라도 그 창에서 만들어진 유령 구독은 우리 손으로 회수되므로 "붙었는데 아무도 안 지우는 tap"
        //   이 남지 않는다. 사전 확인만으로는(옛 구현) 그 사이 재시작이 끼면 새 core 에 유령이 영구 잔류했다.
        if let Some(err) = self.stale_check(id, expect_epoch) {
            return Err(err);
        }
        let sink: Arc<dyn OutputSink> = Arc::new(TurnTapSink {
            sink_id: uuid::Uuid::new_v4(),
            probe,
        });
        // ★live-only 구독(load-bearing — busy 모듈 헤더 "replay 부트스트랩 거부")★: `after_seq = u64::MAX` 는
        //   "이 seq 까지는 이미 봤다" 는 뜻이라, core `subscribe_from` 의 Resumed 분기가 보낼 tail 을
        //   `partition_point(seq <= u64::MAX)` = **전체 skip** 으로 계산한다 → replay 프레임이 tap 에 한
        //   건도 들어오지 않고 그 뒤 live emit 만 받는다. `epoch_matches = true` 를 넘기는 이유: false 면
        //   core 가 "안전 기본값" 으로 **전체 replay** 를 보낸다(FromOldest) — tap 에는 그게 바로 위험이다.
        //   `epoch_matches = true` 가 정당한 이유는 위 사전/사후 검증이 "구독한 core = expect_epoch 의 core"
        //   를 확인해 주기 때문이다.
        // SinkId 는 정상 경로에선 버린다 — tap 은 명시 unsubscribe 를 하지 않는다(수명 = 그 epoch 의
        //   OutputCore). 근거: 세션이 reap 되면 OutputCore 가 drop 되며 구독자도 함께 사라지고, 살아 있는
        //   동안엔 계속 관측해야 한다. epoch 교체는 새 core = 새 attach(로스터 diff 가 건다). 단 **사후
        //   검증이 어긋난 경우에만** 이 id 를 써서 유령 구독을 즉시 회수한다(위 헤더).
        // on_ready 는 no-op — 데몬 WS 경로의 SubscribeAck 큐잉용 hook 이라 tap 엔 할 일이 없다(그리고
        //   core 의 subscribers 락 보유 중 불리므로 블로킹 금지 계약이 걸려 있다).
        let outcome = self
            .manager
            .subscribe_from(id, sink, Some(u64::MAX), true, |_| {})
            .map_err(|e| SubscribeError::Failed(e.to_string()))?;
        if let Some(err) = self.stale_check(id, expect_epoch) {
            // 구독↔검증 사이 epoch 이 바뀌었다 = 방금 붙은 sink 는 유령이다. 되돌린다(best-effort —
            //   그 사이 또 교체됐으면 대상 core 가 이미 drop 되는 중이라 구독도 함께 사라진다).
            let _ = self.manager.unsubscribe(id, outcome.sink_id);
            return Err(err);
        }
        Ok(())
    }

    fn current_epoch(&self, id: PeerId) -> Option<u32> {
        // list_agents 스냅샷 1회 — attach 빈도는 로스터 변화 빈도라 비용 무관.
        self.manager
            .list_agents()
            .into_iter()
            .find(|a| a.id == id)
            .map(|a| a.epoch)
    }
}

impl ManagerTapHost {
    /// 현재 epoch 이 기대와 다르면 `StaleEpoch` 를 만든다(같으면 None). 구독 전·후 두 지점에서 쓴다.
    fn stale_check(&self, id: PeerId, expect_epoch: u32) -> Option<SubscribeError> {
        let current = self.current_epoch(id);
        if current == Some(expect_epoch) {
            None
        } else {
            Some(SubscribeError::StaleEpoch { current })
        }
    }
}

/// ★TurnTapSink — 출력 스트림에 붙어 턴 경계만 읽는 `OutputSink`(C2 · ADR-0110 결정 4)★.
///
/// ★왜 커널이 아니라 여기 사나(load-bearing 경계)★: 아래 분류표는 **백엔드 지식**(claude stream-json 이
///   어떤 이벤트로 턴을 진행/종료하는가)이다. 커널은 신호 어휘(`TurnProbe::on_progress`/`on_turn_done`)만
///   갖고, 그 어휘로 번역하는 이 7줄이 호스트 몫이다(ADR-0004 백엔드 지식 격리와 같은 결).
///
/// ★상태머신(spec §5 · ADR-0104 결정 3)★:
///   - `TextDelta` / `ToolCall` / `Structured` → **busy**. 왜 이 셋인가: 어시스턴트 응답(delta)·도구 호출은
///     턴 진행의 직접 증거고, `Structured` 는 백엔드별 이벤트 탈출구인데 claude 는 **입력 시점 유저 에코**를
///     여기로 낸다 — 즉 대시보드 사용자가 터미널에 직접 입력해 시작한 턴(MessagingService 를 우회하는 경로)도
///     이 variant 로 잡힌다(그래서 반드시 포함해야 한다). 우리 자신의 주입도 같은 에코로 busy 가 되므로,
///     주입 직후 도착한 다음 메시지는 자동으로 다음 턴 경계까지 파킹된다(의도된 동작).
///   - `MessageDone` → **idle** + flush 트리거 통지(턴 종료).
///   - `Usage` / `Error` / `TerminalBytes` → **무시**(상태 불변). Usage 는 턴 중간에도 오고, Error 는
///     스트림 내부 오류지 턴 종료가 아니며(종료는 MessageDone/terminal 상태), TerminalBytes 는 tap 을
///     붙이지 않는 비-structured 경로의 payload 다.
///
/// ★상관 키가 없다(honest scope)★: claude 의 MessageDone 은 `turn_id`/`message_id` 가 모두 None 이라
///   "어느 턴의 종료인가" 를 상관시킬 키가 없다. 그래서 이 tap 은 **턴 카운팅/펜싱을 하지 않고** 단순
///   최신-관측 상태만 유지한다(마지막 이벤트가 결정한다). 중첩 서브에이전트 result 누수 가능성은 busy
///   모듈 헤더의 미확인 항목.
/// ★항상 Ok 반환★: Err 는 코어가 dead-sink 로 판단해 구독을 제거하는 신호다 — tap 은 스스로 빠지지 않고
///   그 세션(epoch)의 수명 동안 관측을 유지한다(정리는 세션 drop).
/// ★콜백 규율★: `send` 는 pump 스레드가 부르는 동기 콜백이다 — 여기서 하는 일은 분류 + 커널 수신구 호출
///   (짧은 락 + 논블록 채널 send)뿐이다. 주입·manager 호출·blocking IO 금지(busy 모듈 헤더).
struct TurnTapSink {
    sink_id: SinkId,
    probe: Arc<TurnProbe>,
}

impl OutputSink for TurnTapSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        // 구조화 이벤트만 본다. Bytes(터미널 payload)는 턴 경계 정보가 없다.
        let OutputPayload::Event(ev) = frame.payload else {
            return Ok(());
        };
        // ★상태 키 = 프레임이 신고한 (agent_id, epoch)★: 이 tap 이 붙은 OutputCore 의 (id, epoch) 와
        //   by-construction 동일하다(코어가 자기 값으로 프레임을 채운다). 프레임 값을 쓰면 게이트가 보는
        //   로스터 epoch 과 같은 축으로 정렬되고, tap 이 중복 필드를 들고 있을 필요가 없다.
        match ev {
            OutputEvent::TextDelta { .. }
            | OutputEvent::ToolCall { .. }
            | OutputEvent::Structured { .. } => self.probe.on_progress(frame.agent_id, frame.epoch),
            OutputEvent::MessageDone { .. } => self.probe.on_turn_done(frame.agent_id, frame.epoch),
            // 상태 불변(위 상태머신 주석).
            OutputEvent::Usage { .. } | OutputEvent::Error(_) | OutputEvent::TerminalBytes(_) => {}
        }
        Ok(())
    }

    fn sink_id(&self) -> SinkId {
        self.sink_id
    }
}

// ── 조립 헬퍼(옛 커널 편의 생성자의 후계 — ADR-0110 결정 3 "데몬이 유일한 조립·배선실") ────────────

/// 운영 편의 조립 — Arc<AgentManager> 를 `ManagerDeliveryPort` 로 감싼다. ★게이트 없음(즉시 주입)★ —
///   실험 bin 등 idle 게이트를 쓰지 않는 조립용. 데몬 부팅은 `messaging_for_manager_gated` 를 쓴다.
pub fn messaging_for_manager(
    manager: Arc<AgentManager>,
    registry: Arc<ControlRegistry>,
) -> MessagingService {
    MessagingService::new(Arc::new(ManagerDeliveryPort::new(manager)), registry)
}

/// 운영 편의 조립(C2) — manager 래핑 + idle 게이트 주입(데몬 부팅·통합 테스트용).
pub fn messaging_for_manager_gated(
    manager: Arc<AgentManager>,
    registry: Arc<ControlRegistry>,
    busy: Arc<dyn BusyGate>,
) -> MessagingService {
    MessagingService::new_gated(Arc::new(ManagerDeliveryPort::new(manager)), registry, busy)
}

/// 운영 편의 조립 — manager 를 `ManagerTapHost` 로 감싼 `BusyTracker`(데몬 부팅용).
pub fn busy_tracker_for_manager(
    manager: Arc<AgentManager>,
    notifier: Arc<dyn IdleNotifier>,
) -> BusyTracker {
    BusyTracker::new(Arc::new(ManagerTapHost::new(manager)), notifier)
}

/// ★하네스 전용★ — 커널 수신구를 **운영과 같은 분류 어댑터**로 감싼 sink 를 만든다(구독은 하지 않는다).
///   통합 하네스가 실 claude 턴 없이 `OutputFrame` 을 손으로 먹여 idle 게이트·배치 flush 를 결정적으로
///   구동하려고 쓴다 — 분류(이벤트→턴 신호)까지 운영 경로 그대로 태우는 게 요점이다.
///   짝: `BusyTracker::probe_for_test` + `mark_attached_for_test`(양성 attach 게이트 통과).
#[cfg(any(test, feature = "test-harness"))]
pub fn turn_tap_sink_for_test(probe: Arc<TurnProbe>) -> Arc<dyn OutputSink> {
    Arc::new(TurnTapSink {
        sink_id: uuid::Uuid::new_v4(),
        probe,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_core::agent::types::AgentId;
    use engram_dashboard_messaging::busy::AttachOutcome;
    use std::sync::Mutex as StdMutex;

    /// 통지 기록용 IdleNotifier — 커널 tracker 를 붙여 분류 결과가 상태로 이어지는지 본다.
    struct RecordingNotifier {
        seen: StdMutex<Vec<AgentId>>,
    }
    impl IdleNotifier for RecordingNotifier {
        fn notify_idle(&self, id: AgentId) {
            self.seen.lock().unwrap().push(id);
        }
    }

    /// 배선 없이 `TurnProbe` 만 내주는 TapHost — 분류 어댑터만 단독 검증한다(실 manager/PTY 없음).
    struct ProbeOnlyHost;
    impl TapHost for ProbeOnlyHost {
        fn subscribe_output(
            &self,
            _id: PeerId,
            _expect_epoch: u32,
            _probe: Arc<TurnProbe>,
        ) -> Result<(), SubscribeError> {
            Ok(())
        }
        fn current_epoch(&self, _id: PeerId) -> Option<u32> {
            Some(0)
        }
    }

    /// 분류 검증용 조립 — tracker + 그 수신구를 감싼 실제 어댑터 sink.
    fn tap() -> (Arc<BusyTracker>, Arc<dyn OutputSink>, AgentId) {
        let t = Arc::new(BusyTracker::new(
            Arc::new(ProbeOnlyHost),
            Arc::new(RecordingNotifier {
                seen: StdMutex::new(Vec::new()),
            }),
        ));
        let id = AgentId::new_v4();
        // 양성 attach 게이트(busy fix 9) 통과 — 부착 표시 없는 키의 관측은 커널이 무시한다.
        assert_eq!(t.attach(id, 0), AttachOutcome::Attached);
        let sink: Arc<dyn OutputSink> = Arc::new(TurnTapSink {
            sink_id: uuid::Uuid::new_v4(),
            probe: t.probe_for_test(),
        });
        (t, sink, id)
    }

    fn feed(sink: &Arc<dyn OutputSink>, id: AgentId, epoch: u32, ev: &OutputEvent) {
        sink.send(OutputFrame {
            agent_id: id,
            epoch,
            seq: 1,
            payload: OutputPayload::Event(ev),
        })
        .expect("tap 은 항상 Ok");
    }

    #[test]
    fn delta_and_done_map_to_progress_and_turn_done() {
        let (t, sink, id) = tap();
        feed(
            &sink,
            id,
            0,
            &OutputEvent::TextDelta {
                text: "x".into(),
                turn_id: None,
                message_id: None,
            },
        );
        assert!(t.is_busy(id, 0), "TextDelta = 턴 진행 → busy");
        feed(
            &sink,
            id,
            0,
            &OutputEvent::MessageDone {
                turn_id: None,
                message_id: None,
            },
        );
        assert!(!t.is_busy(id, 0), "MessageDone = 턴 종료 → idle");
    }

    #[test]
    fn tool_call_and_structured_user_echo_map_to_progress() {
        // Structured 포함이 load-bearing: claude 는 **입력 시점 유저 에코**를 Structured 로 낸다 —
        //   대시보드 사용자 직접 입력으로 시작된 턴(MessagingService 우회)도 이걸로 잡힌다.
        let (t, sink, id) = tap();
        feed(
            &sink,
            id,
            0,
            &OutputEvent::ToolCall {
                name: "Bash".into(),
                args_json: "{}".into(),
                id: None,
                turn_id: None,
                message_id: None,
            },
        );
        assert!(t.is_busy(id, 0), "ToolCall → busy");

        let (t2, sink2, id2) = tap();
        feed(
            &sink2,
            id2,
            0,
            &OutputEvent::Structured {
                kind: "user".into(),
                json: "{}".into(),
            },
        );
        assert!(t2.is_busy(id2, 0), "Structured(유저 에코) → busy");
    }

    #[test]
    fn usage_error_and_terminal_bytes_are_ignored() {
        let (t, sink, id) = tap();
        // idle 상태에서 Usage/Error/Bytes → 여전히 idle.
        feed(
            &sink,
            id,
            0,
            &OutputEvent::Usage {
                input_tokens: 1,
                output_tokens: 2,
                turn_id: None,
            },
        );
        feed(&sink, id, 0, &OutputEvent::Error("stream hiccup".into()));
        sink.send(OutputFrame {
            agent_id: id,
            epoch: 0,
            seq: 2,
            payload: OutputPayload::Bytes(b"raw vt bytes"),
        })
        .expect("항상 Ok");
        assert!(
            !t.is_busy(id, 0),
            "Usage/Error/Bytes 는 턴 시작 신호가 아니다"
        );

        // busy 상태에서도 상태를 바꾸지 않는다(종료 신호는 MessageDone 뿐).
        feed(
            &sink,
            id,
            0,
            &OutputEvent::TextDelta {
                text: "x".into(),
                turn_id: None,
                message_id: None,
            },
        );
        feed(
            &sink,
            id,
            0,
            &OutputEvent::Usage {
                input_tokens: 1,
                output_tokens: 2,
                turn_id: None,
            },
        );
        feed(&sink, id, 0, &OutputEvent::Error("stream hiccup".into()));
        assert!(t.is_busy(id, 0), "Usage/Error 는 턴 종료가 아니다");
    }
}
