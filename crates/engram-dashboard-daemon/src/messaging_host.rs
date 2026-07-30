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

use std::collections::HashSet;
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
    AddressingSources, ControlPlanePort, DeliveryPort, InjectReceipt, LiveAgent, MessagingService,
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

/// ★로스터 술어(load-bearing — 이 파일의 **유일한** 멤버십 조건)★: `Running|Exiting` 만 산 것으로 본다.
///
/// ★"list_agents 에 있음" 이 아니다★: 세션은 reap 까지 맵에 남으므로 단순 존재로 판정하면 시체가 섞인다
///   (그 결과 = 방금 종료된 에이전트에게 배달을 시도하고, 잠듦 파킹으로 넘어갈 이름이 배달 대상으로 오분류).
/// ★capability 조건은 **여기 없다**(ADR-0116 결정 7 — 4차 개정)★: 옛 술어는 `output.structured` 를 함께
///   걸어 터미널/콘솔 모드 세션을 로스터에서 뺐고, 그 결과가 `RECIPIENT_UNREACHABLE` 반려였다. 그 전제는
///   폐기됐다 — 그 CLI 는 자기 입력 큐로 턴 중 입력을 물고 있다가 소비하므로 관측 없이도 배달이 성립한다.
///   구조화 여부는 `LiveAgent::turn_signal` 로 실려 커널의 **타이밍** 판정(idle 게이트 vs 즉시 주입)에만 쓴다.
/// ★뮤테이션 실측 경고★: 이 조건을 지워도 데몬 412 테스트가 전부 초록이었다(비대칭 커버리지) → 이제 유일한
///   멤버십 게이트이므로 **실물 어댑터 레벨 봉인 테스트**가 이 모듈의 필수 산출물이다(아래 tests).
/// ★`pub(crate)` 인 이유 = split-brain 방지(리뷰 fix N4)★: flush 등장 diff(`ws.rs` 이름 축)도 **같은 술어**를
///   써야 한다. 예전엔 그쪽이 인라인 `matches!` 복제본 + "같은 조건" 이라는 주석이었는데, 술어가 바뀔 때 한쪽만
///   고치면 **발송 측과 flush 측이 다른 세계를 본다**(이번 라운드가 잡은 결함 부류 그 자체). 그래서 정의는
///   여기 하나만 두고 호출만 나눈다 — 복제본을 다시 만들지 말 것.
// ADR-0116 (로스터 술어 = 상태만)
pub(crate) fn is_live(a: &engram_dashboard_core::agent::types::AgentInfo) -> bool {
    matches!(a.status, AgentStatus::Running | AgentStatus::Exiting)
}

/// core `AgentInfo` → 커널 `LiveAgent`(경계 번역 — 필요한 4필드만).
///
/// ★`turn_signal` = `capabilities.output.structured`(프록시 — busy.rs 헤더가 근거 정본)★: 턴 이벤트
///   (MessageDone)는 백엔드 decoder 가 있는 에이전트에서만 나오고, decoder 는 구조화 출력 capability 와
///   **정확히 같은 조건**으로 존재한다. 이 값은 **멤버십이 아니라 타이밍**을 가른다(위 `is_live` 주석).
fn to_live_agent(a: engram_dashboard_core::agent::types::AgentInfo) -> LiveAgent {
    LiveAgent {
        id: a.id,
        name: a.name,
        epoch: a.epoch,
        turn_signal: a.capabilities.output.structured,
    }
}

/// (이름, id) 오름차순 — `@all` 결정성(아래 `live_agents` doc)의 정렬 키. 두 조회(`live_agents` ·
/// `addressing_sources`)가 같은 키를 쓰게 함수로 묶었다(한쪽만 정렬하면 판정이 다른 순서를 본다).
fn sort_key(a: &LiveAgent, b: &LiveAgent) -> std::cmp::Ordering {
    a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id))
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
    fn live_agents(&self) -> Vec<LiveAgent> {
        // list_agents 스냅샷 1회 → 산(Running|Exiting) 전원. capability 는 멤버십 조건이 아니다(`is_live`).
        let mut live: Vec<LiveAgent> = self
            .manager
            .list_agents()
            .into_iter()
            .filter(is_live)
            .map(to_live_agent)
            .collect();
        live.sort_by(sort_key);
        live
    }

    /// ★입구 판정 소스 한 장(spec §5 3분기 · ADR-0116)★ — **물리 조회 2회**: `list_agents()` 한 번(로스터) +
    /// `profiles().list()` 한 번(잠든 이름).
    ///
    /// ★이 함수의 존재 이유 = 두 소스를 **한 호출로 묶는 것**★: 로스터와 프로필을 호출자가 따로 뜨면 그 사이
    ///   spawn·종료·삭제가 끼어 **같은 발송의 두 수신자가 다른 세계를 본다**(ADR-0111 결정 2 금지 부류).
    ///   ★로스터 술어는 `is_live` 하나다★ — `live_agents()` 와 **글자 그대로 같은 조건**(4차 개정으로
    ///   capability 조건이 빠졌다: 턴 신호 없는 산 세션도 배달 대상이다, ADR-0116 결정 7).
    /// ★잠듦 = **id 기준**(이름 기준이 아니다)★: 프로필의 세션은 그 프로필 id 로 뜨므로(`activate_profile`),
    ///   "산 세션이 없는 프로필" 을 id 집합으로 정확히 가른다. 이름으로 빼면 동명 프로필 하나가 떠 있을 때
    ///   잠든 다른 프로필까지 함께 사라져 잠듦 층 동명 차단이 무력화된다.
    /// ★이름 파생 = `AgentProfile::canonical_name_when_live()`(단일 출처)★: 산 세션의
    ///   `resolve_canonical_name` 과 **같은 함수 + 같은 cwd 정규화**를 쓴다. 여기서 규칙을 복제하면(예:
    ///   `resolve_display_name(display_name, profile.cwd)`) 빈 override·placeholder cwd·상대/심링크 cwd 에서
    ///   파킹 키가 복원 후 이름과 어긋나 편지가 24h TTL 로 조용히 만료된다 — 잠듦 파킹이 막으려던 그 실패다.
    ///   ★fs 접근은 override 없는 프로필에서만 일어난다★(그 함수의 단축 — 리뷰 fix D3): 발송 임계 경로에
    ///   syscall 을 얹지 않기 위한 것이고, 이 호출은 **락 밖**이다(모듈 헤더 규율).
    /// ★정렬★: 로스터는 `@all` 결정성 때문에 (이름, id) 정렬이 필수고(위 `sort_key` 주석), 잠든 이름도
    ///   같은 이유로 정렬해 둔다(중복은 접지 않는다 — 동명 판정 축이다).
    // ADR-0116 (판정 소스 2종 — 물리 조회 2회)
    fn addressing_sources(&self) -> AddressingSources {
        // ★스냅샷 1회★ — 로스터와 "산 세션 id 집합"(잠듦 차집합의 기준)이 같은 장에서 나온다.
        let snapshot = self.manager.list_agents();
        let mut roster: Vec<LiveAgent> = Vec::with_capacity(snapshot.len());
        let mut live_ids: HashSet<uuid::Uuid> = HashSet::with_capacity(snapshot.len());
        for a in snapshot.into_iter().filter(is_live) {
            live_ids.insert(a.id);
            roster.push(to_live_agent(a));
        }
        roster.sort_by(sort_key);

        let mut dormant_names: Vec<String> = self
            .manager
            .profiles()
            .list()
            .into_iter()
            .filter(|p| !live_ids.contains(&p.id))
            .map(|p| p.canonical_name_when_live())
            .collect();
        dormant_names.sort();
        AddressingSources {
            roster,
            dormant_names,
        }
    }

    /// ★삭제 정리 게이트(spec §5 · ADR-0116 결정 3 — 리뷰 fix D1)★ — **id 축**이고 로스터와 **같은 술어**를
    /// 쓴다(`is_live`). 이름으로 물으면 프로필 삭제로 canonical 이름이 바뀐 산 세션을 놓쳐 정리가 늘 발동한다.
    fn is_agent_live(&self, id: PeerId) -> bool {
        self.manager
            .list_agents()
            .into_iter()
            .any(|a| a.id == id && is_live(&a))
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

    // ══════════════════════════════════════════════════════════════════════════════════════════
    // 리뷰 fix D9-a — 로스터 술어 **실물 어댑터** 봉인(ADR-0116 결정 1·7 · spec §7)
    // ══════════════════════════════════════════════════════════════════════════════════════════

    /// ★왜 실물 어댑터 레벨인가(뮤테이션 실측)★: `is_live`(= `Running|Exiting`) 조건을 **지워도** 데몬 412
    /// 테스트가 전부 초록이었다(구조화 조건을 지운 쪽은 잡혔는데 상태 조건은 무방비 — 비대칭 커버리지).
    /// 4차 개정으로 그 상태 술어가 **유일한 멤버십 게이트**가 됐으므로(capability 는 타이밍 축으로 내려갔다)
    /// 여기서 실 세션으로 못 박는다. 커널 단위 테스트는 하네스가 집합을 스크립트하므로 이 술어를 타지 않는다.
    ///
    /// ★플랫폼 게이트는 **spawn 하는 테스트에만** 건다★: 잠듦 축 봉인은 `upsert` 뿐이라 OS 의존이 없는데,
    /// 모듈째 `#[cfg(windows)]` 로 덮으면 non-Windows 에서 그 회귀가 초록으로 샌다.
    mod roster_predicate {
        use super::*;
        use engram_dashboard_core::agent::preset::PresetRegistry;
        #[cfg(windows)]
        use engram_dashboard_core::agent::profile::SpawnMode;
        use engram_dashboard_core::agent::profile::{AgentCommand, AgentProfile, ProfileRegistry};
        use engram_dashboard_core::agent::session_tracker::{SessionTracker, TrackerConfig};
        use engram_dashboard_core::agent::types::{AgentInfo, AgentStatus, StatusSink};
        use engram_dashboard_core::persistence::{FilePresetStore, FileProfileStore};
        use engram_dashboard_messaging::service::DeliveryPort;
        use std::time::Duration;

        struct NoopSink;
        impl StatusSink for NoopSink {
            fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
            fn agent_list_updated(&self, _a: Vec<AgentInfo>) {}
        }

        fn manager(tag: &str) -> Arc<AgentManager> {
            let sink: Arc<dyn StatusSink> = Arc::new(NoopSink);
            let dir = |k: &str| {
                std::env::temp_dir()
                    .join(format!("engram-roster-{k}-{tag}-{}", uuid::Uuid::new_v4()))
            };
            Arc::new(AgentManager::new(
                sink,
                Arc::new(ProfileRegistry::new(Arc::new(FileProfileStore::new(dir(
                    "prof",
                ))))),
                Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(dir(
                    "preset",
                ))))),
                Arc::new(SessionTracker::new(
                    TrackerConfig {
                        sessions_dir: None,
                        enabled: false,
                        poll_interval: Duration::from_secs(1),
                    },
                    Arc::new(|_, _| {}),
                )),
            ))
        }

        /// ★base 이름(`AgentProfile::name`)과 canonical 이름(`display_name`)을 **일부러 다르게** 만드는
        ///   fixture★: 두 값이 같으면 트랩 필드(`p.name`)로 잠든 이름을 뽑는 회귀가 그대로 통과한다
        ///   (ADR-0116 영향 절 — "그 필드로 잠든 이름을 뽑으면 조용히 어긋난다").
        fn profile(base: &str, canonical: &str) -> AgentProfile {
            let mut p = AgentProfile::new(
                base.to_string(),
                AgentCommand::Shell {
                    program: engram_dashboard_core::agent::manager::default_shell().to_string(),
                    args: vec![],
                },
                std::env::temp_dir(),
                vec![],
                false,
            );
            p.display_name = Some(canonical.to_string());
            p
        }

        /// 턴 신호 없는 산 세션(shell = structured false) — 4차 로스터의 **핵심 모집단**.
        #[cfg(windows)]
        fn shell(name: &str) -> AgentProfile {
            profile(name, name)
        }

        #[cfg(windows)]
        fn wait_until<F: Fn() -> bool>(cond: F) -> bool {
            for _ in 0..150 {
                if cond() {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            cond()
        }

        #[cfg(windows)]
        #[test]
        fn a_live_session_without_a_turn_signal_is_in_the_roster_with_turn_signal_false() {
            // ★멤버십 = 상태뿐 · capability = 타이밍★(ADR-0116 결정 1·7): 구조화 출력이 없어도 로스터에
            //   **있어야** 하고, 그 사실은 `turn_signal = false` 로만 나타나야 한다. 옛 술어(structured 필터)를
            //   되살리면 이 단언이 빈 로스터로 깨진다.
            let manager = manager("live");
            let port = ManagerDeliveryPort::new(manager.clone());
            let info = manager
                .spawn_agent(&shell("sheller"), SpawnMode::Fresh)
                .expect("shell spawn");
            assert!(wait_until(|| manager
                .list_agents()
                .iter()
                .any(|a| a.id == info.id)));

            let roster = port.live_agents();
            let entry = roster
                .iter()
                .find(|a| a.id == info.id)
                .expect("턴 신호 없는 산 세션도 로스터에 있어야(멤버십 조건은 상태뿐)");
            assert!(
                !entry.turn_signal,
                "그 사실은 turn_signal=false 로만 나타난다(= 즉시 주입 대상)"
            );
            // 입구 판정 소스도 **같은 술어**를 쓴다(두 조회가 갈리면 발송과 flush 가 다른 세계를 본다).
            let sources = port.addressing_sources();
            assert!(
                sources.roster.iter().any(|a| a.id == info.id),
                "addressing_sources 의 로스터도 같은 술어여야: {sources:?}"
            );
            assert!(
                port.is_agent_live(info.id),
                "삭제 정리 게이트도 같은 술어여야(리뷰 fix D1 — 이 부류를 놓치면 배달될 메일이 죽는다)"
            );

            manager.kill_agent(info.id).ok();
        }

        #[test]
        fn two_dormant_profiles_sharing_a_name_are_both_reported() {
            // ★막는 회귀 2종★(둘 다 뮤테이션 실측으로 무방비 확인 — 데몬 전 테스트 초록이었다):
            //   ① `dormant_names` 에 `.dedup()` — 접히면 동명 잠듦이 `RECIPIENT_AMBIGUOUS` 대신 이름 키로
            //      파킹돼 **먼저 복원된 쪽이 남의 편지를 조용히 받는다**(ADR-0116 결정 1 이 금지한 결말).
            //   ② 이름 파생을 트랩 필드 `p.name` 으로 바꾸기 — fixture 가 base ≠ canonical 이라 그 순간
            //      파킹 키가 복원 후 산 이름과 어긋난다(편지가 24h TTL 로 조용히 만료).
            //   커널 쪽 동명 테스트는 중복 목록을 fake 에 **스크립트**하므로 이 생산자를 타지 않는다.
            let manager = manager("dormant-dup");
            let port = ManagerDeliveryPort::new(manager.clone());
            // 스폰하지 않는다 — 산 세션이 없는 프로필이 곧 잠듦이다(`live_ids` 차집합).
            manager.profiles().upsert(profile("raw-twin-a", "twin"));
            manager.profiles().upsert(profile("raw-twin-b", "twin"));

            let sources = port.addressing_sources();
            assert!(
                sources.roster.is_empty(),
                "스폰이 없으므로 로스터는 비어야: {sources:?}"
            );
            assert_eq!(
                sources.dormant_names,
                vec!["twin".to_string(), "twin".to_string()],
                "동명 잠듦 2건은 canonical 이름 그대로 2건 올라와야: {sources:?}"
            );
        }

        #[cfg(windows)]
        #[test]
        fn a_live_namesake_does_not_hide_a_dormant_profile_with_the_same_name() {
            // ★봉인 대상 = 잠듦 차집합의 축이 **id** 라는 규칙 자체(ADR-0116 결정 1)★. 이 fixture 가 재현하는
            //   실패 모드는 **이름 우연 일치**다: 잠든 프로필의 canonical 이름이 산 세션의 이름과 같으면
            //   이름 축 차집합에서 그 프로필이 통째로 사라지고(`dormant_names == []`), id 축이면 산 것만 빠지고
            //   잠든 쪽은 남는다(아래 단언).
            // ★정직 명시 — 이 fixture 의 발송 결말은 뮤테이션과 무관하게 같다★: 산 동명이 있으면 잠듦 축은
            //   조회조차 되지 않는다(산 쪽이 이긴다 — `service.rs::a_live_agent_wins_over_a_dormant_namesake`).
            //   즉 이건 어댑터 산출물의 white-box 봉인이지 발송 결말 회귀가 아니다.
            let manager = manager("live-namesake");
            let port = ManagerDeliveryPort::new(manager.clone());
            let info = manager
                .spawn_agent(&profile("raw-live-twin", "twin"), SpawnMode::Fresh)
                .expect("shell spawn");
            assert!(wait_until(|| manager
                .list_agents()
                .iter()
                .any(|a| a.id == info.id)));
            manager
                .profiles()
                .upsert(profile("raw-dormant-twin", "twin"));

            let sources = port.addressing_sources();
            assert!(
                sources.roster.iter().any(|a| a.id == info.id),
                "산 동명이 로스터에 있어야(전제): {sources:?}"
            );
            assert_eq!(
                sources.dormant_names,
                vec!["twin".to_string()],
                "산 쪽은 id 로 빠지고 잠든 동명은 남아야(이름 축으로 빼면 비어 버린다): {sources:?}"
            );

            manager.kill_agent(info.id).ok();
        }

        // ★종료된 세션의 로스터 부재는 여기서 단언하지 않는다(정직 명시)★: 이 조립에서 실제 종료는
        // reaper 가 세션을 **맵에서 곧바로 제거**하므로(reaper.rs — 시체 보존은 프로필 축이다) "terminal
        // 상태인데 목록에 남아 있는" 상태를 실 세션으로 만들 수 없다. 그 술어의 봉인은 세션 주입 seam 이 있는
        // 통합 테스트가 맡는다(`tests/control_send.rs` — `roster_excludes_a_terminal_session_still_in_the_map`).
    }
}
