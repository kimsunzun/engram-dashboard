//! messaging_host — 메시징 커널(`engram-dashboard-messaging`)의 **호스트 어댑터 + 조립실**(ADR-0110).
//!
//! ★역할★: 커널이 소유한 계약(포트 trait)에 데몬의 실물을 꽂는다. 커널은 `AgentManager`·
//!   `TurnObservations`·`ControlRegistry` 를 **타입으로도 모르므로**(완전 상호무지 — ADR-0110 결정 2),
//!   그 셋을 아는 코드는 전부 이 파일에 모인다:
//!     - `ManagerDeliveryPort` — `DeliveryPort`(주입·로스터·이름) → `AgentManager`.
//!     - `ManagerTurnFacts` — `TurnFacts`(턴 관측 사실 조회) → 코어의 턴 관측 표(ADR-0113 결정 1).
//!     - `ControlRegistry` 의 `ControlPlanePort` 구현 — 봉투 포맷 조회 + 배달 관측 적재.
//!     - 조립 헬퍼(`messaging_for_manager`/`messaging_for_manager_gated`/`busy_gate_for_manager`).
//!
//! ★불변식(load-bearing)★: 정책은 커널에, 어댑터는 얇게. busy 불변식(positive-knowledge-only·상한·
//!   도어벨 규율)을 여기서 재구현하지 않는다 — 이 파일이 하는 일은 **타입 번역과 배선** 뿐이다
//!   (ADR-0110 영향/불변식 "포트는 얇게, 정책은 lib에").
//!
//! tauri import 0(daemon crate).
// ADR-0110
// ADR-0113

use std::sync::Arc;
use std::time::Instant;

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::turn::TurnObservations;
use engram_dashboard_messaging::busy::{BusyGate, BusyPolicy, IdleNotifier, TurnFact, TurnFacts};
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
/// ★정의는 core `AgentStatus::is_live` 로 내려갔다(ADR-0119 결정 4 — 에이전트 "사실" 계층은 코어)★:
///   명부(`AgentManager::roster`)도 같은 술어를 써야 하는데 코어는 데몬을 의존할 수 없다. 여기 남은 건
///   `AgentInfo` 를 받는 데몬측 호출 어댑터고, `ws.rs` 도 계속 이 이름을 부른다(복제본 금지 규율 유지).
// ADR-0116 (로스터 술어 = 상태만)
// ADR-0119
pub(crate) fn is_live(a: &engram_dashboard_core::agent::types::AgentInfo) -> bool {
    a.status.is_live()
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

    /// ★입구 판정 소스 한 장(spec §5 3분기 · ADR-0116)★ — 이제 **명부 단일 입구**
    /// (`AgentManager::roster()`, ADR-0119)를 한 번 부르고 커널 타입으로 번역만 한다.
    ///
    /// ★차집합 계산은 여기 없다(ADR-0119 결정 2)★: 옛날엔 이 함수가 산 목록과 프로필 목록을 각자 떠서
    ///   id 차집합으로 잠든 이름을 만들었다 — 그 합성이 프론트에도 사본으로 있어 한쪽만 고쳐지는 drift 가
    ///   확정적이었다. 이제 합성은 매니저 한 곳이고 여기는 포워딩이다. **여기서 다시 합치지 말 것.**
    ///   스냅샷 1회·id 축 차집합·동명 잠듦 미접기·override 있으면 fs 무접근 — 그 규율은 전부 `roster()`
    ///   doc 에 있고 그쪽이 정본이다.
    /// ★로스터 술어는 여전히 `is_live` 하나다★ — `roster()` 가 `AgentStatus::is_live` 를 쓰고 이 파일의
    ///   `is_live` 도 같은 함수를 부른다(정의 1곳).
    /// ★정렬★: 로스터는 `@all` 결정성 때문에 (이름, id) 정렬이 필수고(위 `sort_key` 주석), 잠든 이름도
    ///   같은 이유로 정렬해 둔다(중복은 접지 않는다 — 동명 판정 축이다).
    // ADR-0119 (명부 단일 입구 — 이 함수는 포워더)
    // ADR-0116 (판정 소스 2종)
    fn addressing_sources(&self) -> AddressingSources {
        let mut roster: Vec<LiveAgent> = Vec::new();
        let mut dormant_names: Vec<String> = Vec::new();
        for entry in self.manager.roster() {
            match entry.live {
                Some(info) => roster.push(to_live_agent(info)),
                None => dormant_names.push(entry.canonical_name),
            }
        }
        roster.sort_by(sort_key);
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

/// 운영 `TurnFacts` — 코어의 턴 관측 표(ADR-0113 사실 계층)를 커널 어휘로 번역하는 **읽기 전용** 창.
///
/// ★얇음이 계약이다★: 여기엔 판정이 없다 — 상한·폴백은 커널 `BusyPolicy` 소유다(ADR-0110 "포트는 얇게,
///   정책은 lib에"). 이 어댑터에 "늙었으면 안 보여 준다" 같은 조건을 넣으면 우편 정책이 데몬으로 새고,
///   같은 표를 보는 다른 소비자와 판정이 갈린다.
/// ★표를 직접 든다(manager 를 안 든다)★: 조회 경로에 sessions 락을 끼우지 않는다 — 이 조회는 배달 판정
///   경로에 있고, 표는 매니저와 무관한 leaf 락이다(ADR-0006).
pub struct ManagerTurnFacts {
    turns: Arc<TurnObservations>,
}

impl ManagerTurnFacts {
    pub fn new(manager: &Arc<AgentManager>) -> Self {
        Self {
            turns: manager.turns(),
        }
    }
}

impl TurnFacts for ManagerTurnFacts {
    fn turn_fact(&self, id: PeerId, epoch: u32) -> Option<TurnFact> {
        self.turns.get(id, epoch).map(|o| TurnFact {
            in_turn: o.in_turn,
            last_signal: o.last_signal,
        })
    }

    fn in_turn_snapshot(&self) -> Vec<(PeerId, u32, Instant)> {
        self.turns.in_turn_snapshot()
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

/// 운영 편의 조립 — 코어 턴 관측 표를 커널 정책으로 감싼 idle 게이트(데몬 부팅용).
pub fn busy_gate_for_manager(
    manager: Arc<AgentManager>,
    notifier: Arc<dyn IdleNotifier>,
) -> BusyPolicy {
    BusyPolicy::new(Arc::new(ManagerTurnFacts::new(&manager)), notifier)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_core::agent::types::AgentId;

    // ── 턴 사실 어댑터: 코어 표 → 커널 어휘(번역만, 판정 없음) ──────────────────────────────

    /// 사실 어댑터 조립 — manager 없이 표만 들고 같은 번역을 태운다.
    fn facts(turns: Arc<TurnObservations>) -> ManagerTurnFacts {
        ManagerTurnFacts { turns }
    }

    #[test]
    fn turn_facts_forwards_the_core_observation_verbatim() {
        use engram_dashboard_core::agent::turn::TurnSignal;
        let turns = Arc::new(TurnObservations::new());
        let f = facts(turns.clone());
        let id = AgentId::new_v4();
        assert_eq!(f.turn_fact(id, 0), None, "미관측은 미관측으로 넘긴다");

        let t0 = Instant::now();
        turns.observe_at(id, 0, 1, TurnSignal::Progress, t0);
        assert_eq!(
            f.turn_fact(id, 0),
            Some(TurnFact {
                in_turn: true,
                last_signal: t0
            })
        );
        assert_eq!(f.in_turn_snapshot(), vec![(id, 0, t0)]);

        // ★상한 판정은 여기 없다★: 아무리 늙어도 어댑터는 사실을 그대로 넘긴다(정책은 커널 소유).
        turns.observe_at(id, 0, 2, TurnSignal::Ended, t0);
        assert_eq!(
            f.turn_fact(id, 0),
            Some(TurnFact {
                in_turn: false,
                last_signal: t0
            })
        );
        assert!(
            f.in_turn_snapshot().is_empty(),
            "sweep 입구에는 턴 중인 것만 오른다"
        );
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
            // ★하네스 seam 으로 심는다(ADR-0120)★: 정상 경로(`create_agent`)는 이제 명부 전역 이름
            //   유일성을 강제해 동명 2건을 **만들 수 없다**. 이 테스트가 봉인하는 건 그 위층 규칙이
            //   아니라 **어댑터 산출물**(동명 잠듦을 접지 않는다)이라, 유일성을 우회해 상태를 직접 만든다.
            //   유일성이 데이터 전체에 적용되기 전(기존 agents.json)엔 이 상태가 실재할 수 있다.
            manager.seed_agent_bypassing_uniqueness(profile("raw-twin-a", "twin"));
            manager.seed_agent_bypassing_uniqueness(profile("raw-twin-b", "twin"));

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
            // ★하네스 seam★: 산 `twin` 이 이미 명부에 있으므로 정상 경로면 `twin(1)` 로 개명된다
            //   (ADR-0120). 이 테스트가 보는 건 **차집합 축이 id 라는 사실**이라 이름이 같아야 성립한다.
            manager.seed_agent_bypassing_uniqueness(profile("raw-dormant-twin", "twin"));

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
