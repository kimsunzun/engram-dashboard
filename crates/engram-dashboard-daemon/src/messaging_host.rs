//! messaging_host — 메시징 커널(`engram-dashboard-messaging`)의 **호스트 어댑터 + 조립실**(ADR-0110).
//!
//! ★역할★: 커널이 소유한 계약(포트 trait)에 데몬의 실물을 꽂는다. 커널은 `AgentManager`·
//!   `TurnObservations`·`ControlRegistry` 를 **타입으로도 모르므로**(완전 상호무지 — ADR-0110 결정 2),
//!   그 셋을 아는 코드는 전부 이 파일에 모인다.
//!
//! ★★접합 원장(ledger) — 이 파일의 거주자 전수. 아래 불변식 2가 이 목록을 입장 심사로 쓴다★★
//!   *형식: `타입/함수` — 어느 **커널 포트·seam** 에 묶이나 → 어느 **데몬 실물** 에 꽂나.*
//!   *입도(정직 명시): 항목 단위는 **접합 단위**다 — 한 항목에만 쓰이는 사적 번역 헬퍼
//!    (`to_live_agent`·`receipt`·`sort_key` 등)는 그 소유 항목에 딸린 것으로 보고 따로 적지 않는다.*
//!     - `ManagerDeliveryPort` — `DeliveryPort`(주입·로스터·이름) → `AgentManager`.
//!     - `is_live` — 로스터 술어(코어 `AgentStatus::is_live` 호출 어댑터). 항목이 **둘 이상**(위 포트와
//!       아래 `RosterDiff`)에 걸려 있어 따로 적는다.
//!     - `ManagerTurnFacts` — `TurnFacts`(턴 관측 사실 조회) → 코어의 턴 관측 표(ADR-0113 결정 1).
//!     - `ControlRegistry` 의 `ControlPlanePort` 구현 — 봉투 포맷 조회 + 배달 관측 적재.
//!     - 조립 헬퍼(`messaging_for_manager`/`messaging_for_manager_gated`/`busy_gate_for_manager`) —
//!       `MessagingService`/`BusyPolicy` 생성 seam → manager + control registry 배선.
//!     - `ChannelIdleNotifier` — 커널 `IdleNotifier`(상한 sweep 깨우기)·`FlushTrigger`(서비스 도어벨)
//!       → flush 채널 송신단. (`IdleCoalescer` 는 이 출구의 내부 상태다.)
//!     - `MessagingFlushSink` — 커널엔 포트가 없다. 묶이는 seam = **코어 `StatusSink`**(데코레이터)이고,
//!       그 위에서 등장/epoch bump 를 diff 해(`RosterDiff`) 위 도어벨과 **같은 채널**로 flush 를 건다.
//!     - `spawn_flush_worker`/`run_flush_worker`/`run_flush_lane`/`FlushWorkerHandles`/`FlushWiring` —
//!       위 채널의 소비단 → `MessagingSlot`(늦은 주입된 `MessagingService`)의 `flush_for`.
//!
//! ★불변식 1 — 커널 판정 재구현 금지(load-bearing · ADR-0110 "포트는 얇게, 정책은 lib에")★: 커널이
//!   내리는 판정을 어댑터가 되풀이하거나 앞질러 내리지 않는다. busy 판정(positive-knowledge-only·상한)·
//!   발송 3분기·파킹 TTL 은 전부 커널 소유다.
//!   ★이 제약이 거는 대상은 **포트**다 — "이 파일에 정책이 하나도 없다" 가 아니다★:
//!   flush 파이프라인은 **호스트 소유 정책**을 담는다 — Idle coalescing(`IdleCoalescer`),
//!   로스터 diff 시퀀싱(`RosterDiff` — 락 보유 중 enqueue 라는 의도된 ADR-0006 이탈), blocking 격리
//!   (`run_flush_lane` 의 `spawn_blocking`). 셋 다 **커널이 알 수 없는 사실**에 대한 판정이라 커널로
//!   올릴 수 없다(코어 "표 갱신 → 통지" 순서 · status 콜백 블록 금지 · tokio executor 굶주림). 각 근거는
//!   그 타입의 헤더가 정본이고, 이 헤더는 "그것들이 여기 있어도 되는 이유" 까지만 말한다.
//! ★불변식 2 — 입장 심사(위 문장을 백지수표로 읽지 말 것)★: 새 타입이 여기 들어오려면 **위 접합 원장에
//!   한 줄로 적을 수 있어야 한다** — "어느 커널 포트·seam 에 묶이고 어느 데몬 실물에 꽂는가". 적을 수
//!   없으면 들어오지 않는다(원장에 안 적고 넣는 것도 위반 — 심사를 무력화한다).
//!   ★왜 "메시징과 관련 있으면 OK" 가 아닌가★: 그 기준은 자기 증명이다(넣는 사람은 이미 관련 있다고
//!   믿는다). ADR-0129 가 기록한 실제 부식(제어 평면·메시징 접합이 `ws.rs` 로 흘러든 과정)은 **매 단계
//!   주제상 관련 있었다** — 그러니 "관련성" 은 아무것도 막지 못한다. 반면 포트/seam 이름을 대라는 요구는
//!   기계적이라, 댈 이름이 없다는 사실 자체가 거절 사유가 된다.
//!   그 위에서, 원장에 적히더라도 그 자리는 **접합**이지 데몬 살림이 아니다 — 살림이 쌓이면 ADR-0129 가
//!   없애려는 "전부를 아는 자리" 의 재발이다.
//! ★flush 파이프라인이 하필 여기 사는 이유(ADR-0129)★: 그것은 네트워크 살림이 아니라 메시징 접합이다 —
//!   커널 포트(`IdleNotifier`/`FlushTrigger`)를 구현하고 커널 서비스(`MessagingSlot`)를 소비하며, 로스터
//!   술어(`is_live`)를 발송 측과 공유해야 한다. 데몬 lib 3층 분리에서 이 파일은 통째로 에이전트 시스템 쪽이다.
//!
//! tauri import 0(daemon crate).
// ADR-0110
// ADR-0129

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_core::agent::manager::AgentManager;
use engram_dashboard_core::agent::profile::RestoreReport as CoreRestoreReport;
use engram_dashboard_core::agent::turn::TurnObservations;
use engram_dashboard_core::agent::types::{
    AgentId, AgentInfo as CoreAgentInfo, AgentStatus as CoreStatus, StatusSink,
};
use engram_dashboard_messaging::busy::{BusyGate, BusyPolicy, IdleNotifier, TurnFact, TurnFacts};
use engram_dashboard_messaging::envelope::{DeliveryObservation, EnvelopeFormat};
use engram_dashboard_messaging::service::{
    AddressingSources, ControlPlanePort, DeliveryPort, InjectReceipt, LiveAgent, MessagingService,
};
use engram_dashboard_messaging::PeerId;

use tokio::sync::mpsc;

use crate::control::registry::ControlRegistry;
use crate::status_fanout::DaemonStatusSink;

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

/// ★로스터 술어(load-bearing — 이 파일의 **유일한** 멤버십 조건)★.
///
/// ★정의는 core `AgentStatus::is_live`(ADR-0119 결정 4 — 에이전트 "사실" 계층은 코어)★: 명부
///   (`AgentManager::roster`)도 같은 술어를 써야 하는데 코어는 데몬을 의존할 수 없다. 여기 남은 건
///   `AgentInfo` 를 받는 데몬측 호출 어댑터다. (`pub(crate)` 는 크로스-모듈 호출의 잔재로 남겨 둔다.)
// ADR-0119
pub(crate) fn is_live(a: &engram_dashboard_core::agent::types::AgentInfo) -> bool {
    a.status.is_live()
}

fn to_live_agent(a: engram_dashboard_core::agent::types::AgentInfo) -> LiveAgent {
    LiveAgent {
        id: a.id,
        name: a.name,
        epoch: a.epoch,
        turn_signal: a.capabilities.output.structured,
    }
}

/// `@all` 결정성(아래 `live_agents` doc)의 정렬 키. 두 조회(`live_agents` ·
/// `addressing_sources`)가 같은 키를 쓰게 함수로 묶었다(한쪽만 정렬하면 판정이 다른 순서를 본다).
fn sort_key(a: &LiveAgent, b: &LiveAgent) -> std::cmp::Ordering {
    a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id))
}

fn receipt(o: engram_dashboard_core::agent::types::WriteOutcome) -> InjectReceipt {
    InjectReceipt {
        bytes_requested: o.bytes_requested,
        bytes_written: o.bytes_written,
        msg_uuid: o.msg_uuid,
        epoch: o.epoch,
    }
}

impl DeliveryPort for ManagerDeliveryPort {
    /// ★`submit_stdin_observed` 여야 한다(`write_stdin_observed` 로 되돌리지 마라)★: 봉투 바이트만 쓰면
    ///   터미널(TUI) 수신자는 그걸 입력창에 담아 둔 채 턴을 시작하지 않는다 — 배달은 "바이트가 닿았다" 가
    ///   아니라 "수신자가 턴으로 받았다" 까지다(`DeliveryPort::inject` 계약). 제출이 필요한 백엔드인지의
    ///   판정은 core seam 뒤 backend 소유라(ADR-0004) 이 어댑터는 동사만 고른다.
    fn inject(&self, to_id: PeerId, bytes: &[u8]) -> Result<InjectReceipt, String> {
        self.manager
            .submit_stdin_observed(to_id, bytes)
            .map(receipt)
            .map_err(|e| e.to_string())
    }

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
    /// ★편지를 읽는 주체만 명단에 올린다(사용자 결정 2026-08-17)★: 상태 술어(`is_live`)에 더해
    ///   `reads_messages` 로 셸을 뺀다 — 셸에 도착한 봉투는 읽히는 게 아니라 **명령으로 실행된다**.
    ///   ★ADR-0116 결정 1·7 로의 회귀가 아니다★: 그건 "턴을 관측할 수 없으니 배달할 수 없다" 를 기각한
    ///   것이고(터미널 claude 는 그대로 받는다), 이건 관측이 아니라 **입력이 무엇으로 해석되는가** 축이다.
    /// ★`addressing_sources` 와 같은 술어여야 한다★: 한쪽만 걸면 입구 판정과 배달 명단이 갈려 셸이
    ///   `@all` 로 해석된 뒤 주입까지 간다.
    /// ★자격은 **스냅샷 필드**로 읽는다(매니저에 되묻지 마라)★: 항목마다 재조회하면 TOCTOU·비원자성·
    ///   세션당 락이 한꺼번에 생긴다(`AgentInfo::reads_messages` doc).
    fn live_agents(&self) -> Vec<LiveAgent> {
        let mut live: Vec<LiveAgent> = self
            .manager
            .list_agents()
            .into_iter()
            .filter(|a| is_live(a) && a.reads_messages)
            .map(to_live_agent)
            .collect();
        live.sort_by(sort_key);
        live
    }

    /// ★차집합 계산은 여기 없다(ADR-0119 결정 2)★: 옛날엔 이 함수가 산 목록과 프로필 목록을 각자 떠서
    ///   id 차집합으로 잠든 이름을 만들었다 — 그 합성이 프론트에도 사본으로 있어 한쪽만 고쳐지는 drift 가
    ///   확정적이었다. 이제 합성은 매니저 한 곳이고 여기는 포워딩이다. **여기서 다시 합치지 말 것.**
    /// ★정렬★: 잠든 이름도 `@all` 결정성 때문에 정렬해 둔다(위 `live_agents` doc).
    // ADR-0119 (명부 단일 입구 — 이 함수는 포워더)
    fn addressing_sources(&self) -> AddressingSources {
        let mut roster: Vec<LiveAgent> = Vec::new();
        let mut dormant_names: Vec<String> = Vec::new();
        for entry in self.manager.roster() {
            match entry.live {
                // 산 명단은 `live_agents` 와 **같은 술어**여야 한다(위 doc) — 여기만 걸고 한쪽을 빠뜨리면
                // 셸이 입구에선 해석되고 배달 명단엔 없어 판정이 갈린다. 자격은 이 한 스냅샷의 필드다.
                Some(info) if info.reads_messages => roster.push(to_live_agent(info)),
                // ★잠든 이름은 거르지 않는다(알려진 잔여 — 배달 안전과 무관)★: 잠든 셸 이름 앞으로 편지가
                //   파킹될 수는 있으나, 깨어나도 산 명단에 오르지 않으므로 **주입은 일어나지 않는다**(TTL 로
                //   만료). 여기까지 거르면 셸 프로필을 산 에이전트 대역으로 쓰는 기존 통합 테스트의 전제가
                //   무너진다 — 그 fixture 전략은 별도 결정 사항이라 이 수정에서 건드리지 않는다.
                Some(_) => {}
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

    fn is_agent_live(&self, id: PeerId) -> bool {
        self.manager
            .list_agents()
            .into_iter()
            .any(|a| a.id == id && is_live(&a))
    }

    /// ★우편 자격을 걸지 않는다(그 이유는 trait doc)★ — 술어는 `is_agent_live` 와 같은 **생존 한 축**이고
    /// 축만 id 에서 이름으로 바뀐다. 이름 파생은 `live_agents` 와 같은 `list_agents()` 출처를 쓴다.
    fn live_id_for_name(&self, name: &str) -> Option<PeerId> {
        self.manager
            .list_agents()
            .into_iter()
            .find(|a| is_live(a) && a.name == name)
            .map(|a| a.id)
    }

    fn canonical_name(&self, id: PeerId) -> Option<String> {
        self.manager.canonical_name(id)
    }
}

// ── 제어 평면 어댑터 ────────────────────────────────────────────────────────────────────────────

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

// ── 조립 헬퍼 ──────────────────────────────────────────────────────────────────────────────────

/// ★게이트 없음(즉시 주입)★ — 실험 bin 등 idle 게이트를 쓰지 않는 조립용. 데몬 부팅은
///   `messaging_for_manager_gated` 를 쓴다.
pub fn messaging_for_manager(
    manager: Arc<AgentManager>,
    registry: Arc<ControlRegistry>,
) -> MessagingService {
    MessagingService::new(Arc::new(ManagerDeliveryPort::new(manager)), registry)
}

/// ★idle 게이트 주입★ — 데몬 부팅·통합 테스트용.
pub fn messaging_for_manager_gated(
    manager: Arc<AgentManager>,
    registry: Arc<ControlRegistry>,
    busy: Arc<dyn BusyGate>,
) -> MessagingService {
    MessagingService::new_gated(Arc::new(ManagerDeliveryPort::new(manager)), registry, busy)
}

pub fn busy_gate_for_manager(
    manager: Arc<AgentManager>,
    notifier: Arc<dyn IdleNotifier>,
) -> BusyPolicy {
    BusyPolicy::new(Arc::new(ManagerTurnFacts::new(&manager)), notifier)
}

// ── MessagingFlushSink(등장/epoch flush 트리거, ADR-0104 — C1/C2) ──────────────────────────

/// flush worker 로 흐르는 작업 단위(C1 등장 flush + C2 idle 게이트 도어벨).
///
/// ★하나의 채널·하나의 소비자★: 두 종류를 따로 나르면 같은 에이전트의 등장과 턴 종료가 서로 앞질러
///   배선 추론이 어려워진다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlushMsg {
    /// 로스터 등장/epoch bump(그 이름의 도달 후보가 **유일**할 때만) — 그 **이름** 앞 파킹을 일괄 flush.
    Appear { name: String, id: AgentId },
    /// 턴 종료(idle 전이) 관측 — 그 id 의 파킹을 오래된 순 일괄 주입(C2 idle 게이트, ADR-0104 결정 3).
    Idle { id: AgentId },
}

/// ★Idle 통지 coalescer(C2 리뷰 fix 10)★ — 같은 id 의 **미처리** Idle 이 이미 큐에 있으면 새 enqueue 를 접는다.
///
/// ★왜 필요한가(유계 채널 압력)★: 통지는 MessageDone **마다** 나간다(누락 < 잉여 — busy.rs `IdleNotifier`).
///   에이전트가 도구 호출을 연달아 돌리면 짧은 시간에 MessageDone 이 여러 번 나올 수 있고, unbounded 채널
///   이라 그만큼 항목이 쌓인다(메모리·처리 낭비 — 대부분 빈 큐 no-op). flush 는 **큐 전체를 drain** 하므로
///   같은 id 의 Idle N개는 1개와 결과가 같다 → 접어도 의미가 보존된다.
/// ★lost wakeup 이 없는 이유(load-bearing)★: 코어가 "① 턴 관측 표 갱신 → ② 통지" 순서를 지키므로
///   (output_core.rs emit), 접힌 통지가 가리키는 상태 변화는 **아직 처리 안 된 그 Idle** 이 대표한다.
///   소비자는 **집어들 때 먼저 집합에서 지우고**(그 뒤에 게이트를 보고 flush) 처리하므로, 처리 도중 도착한
///   새 턴 종료 신호는 다시 enqueue 된다.
/// ★Appear 는 절대 접지 않는다★: 등장은 고유한 사건이라 무손실이어야 한다. 이 coalescer 는 **Idle 전용**이다.
#[derive(Debug, Default)]
pub struct IdleCoalescer {
    pending: Mutex<std::collections::HashSet<AgentId>>,
}

impl IdleCoalescer {
    pub fn new() -> Self {
        Self::default()
    }

    /// 이 id 의 Idle 을 enqueue 해야 하나(= 미처리 항목이 아직 없나). true 면 호출자가 send 한다.
    fn claim(&self, id: AgentId) -> bool {
        self.pending
            .lock()
            .expect("idle coalescer poisoned")
            .insert(id)
    }

    /// 소비자가 이 id 의 Idle 을 집어들었다 — 이후 도착하는 통지는 다시 enqueue 돼야 한다.
    fn taken(&self, id: AgentId) {
        self.pending
            .lock()
            .expect("idle coalescer poisoned")
            .remove(&id);
    }
}

/// 커널 → flush worker 통지 구현 — `IdleNotifier`(상한 sweep 의 깨우기)와 `FlushTrigger`(서비스 도어벨)를 **같은
///   채널**로 잇는다(C2). 둘 다 결과가 "그 id 의 파킹 큐를 flush" 라 메시지를 나눌 이유가 없다.
///
/// ★논블록 계약(load-bearing)★: `notify_idle` 은 **pump 스레드**에서, `request_flush` 는 발신(MCP/HTTP)
///   스레드에서 불린다. unbounded 채널의 `send` 는 논블록·무-await 라 그 계약을 만족한다. 채널이 닫혔으면
///   (worker 종료) 결과를 버린다 — 그 시점엔 데몬이 내려가는 중이고 파킹은 인메모리라 잃을 게 없다
///   (spec §0 영속화 없음).
pub struct ChannelIdleNotifier {
    tx: mpsc::UnboundedSender<FlushMsg>,
    coalescer: Arc<IdleCoalescer>,
}

impl ChannelIdleNotifier {
    pub fn new(tx: mpsc::UnboundedSender<FlushMsg>, coalescer: Arc<IdleCoalescer>) -> Self {
        Self { tx, coalescer }
    }

    fn enqueue(&self, id: AgentId) {
        if self.coalescer.claim(id) {
            let _ = self.tx.send(FlushMsg::Idle { id });
        }
    }
}

impl engram_dashboard_messaging::busy::IdleNotifier for ChannelIdleNotifier {
    fn notify_idle(&self, id: AgentId) {
        self.enqueue(id);
    }
}

impl engram_dashboard_messaging::service::FlushTrigger for ChannelIdleNotifier {
    fn request_flush(&self, id: AgentId) {
        self.enqueue(id);
    }
}

/// ★파킹 flush 트리거(ADR-0104 · S18 메시징 v1 C1)★: `DaemonStatusSink` 를 **감싸** 로스터 변화를
///   데몬측에서 관측하고, 새로 살아났거나 epoch 이 bump 된 이름 앞으로 파킹된 메시지를 flush 시킨다.
///
/// ★왜 sink 를 감싸나(core seam 무변경 — ADR-0104)★: 코어는 메시징을 몰라야 한다(격리 ADR-0028/0104).
///   AgentManager 의 상태 sink 가 이미 `agent_list_updated(Vec<AgentInfo>)` 로 로스터 스냅샷을 push 하므로
///   (ADR-0028 single-push broadcast), 그 사실을 **데몬측에서 diff** 해 flush 를 건다 — 코어에 새 seam 을
///   내지 않고 이미 흐르는 이벤트에 얹는다. wrap 이라 기존 broadcast(프론트 fanout)는 그대로 delegate 된다.
///
/// ★flush 작업을 status-sink 콜백에서 분리(finding 5 · load-bearing)★: 예전엔 이 sink 가
///   `agent_list_updated` **안에서 동기적으로** `flush_for` 를 돌렸다 — 이 콜백은 manager/reaper 스레드가
///   부르는 block 금지 경로인데, 큰 배치가 여러 blocking write 를 마칠 때까지 로스터 이벤트 forwarding 이
///   막혀 spawn/reap/프론트 업데이트가 지연됐다. 이제 sink 는 **싼 diff 만** 하고 `FlushMsg` 를
///   unbounded 채널에 push 한 뒤 status 이벤트를 **즉시** forward 한다. 실제 flush 는 flush worker 몫이다.
///
/// ★락 규율(ADR-0006) — **의도된 이탈**★: prev 스냅샷 Mutex 는 diff 뿐 아니라 **채널 enqueue 까지 든 채로**
///   끝난다(`RosterDiff::dispatch` — 근거는 그 헤더가 정본). ★"락 놓고 send" 로 되돌리지 말 것★: 그게 이
///   이탈이 막는 회귀다.
/// ★그 락을 든 채 **밖으로 나가는 호출은 둘**이다(전수 — 나머지는 전부 이 crate 안의 순수 자료 조작:
///   `is_live` 술어 · HashMap 조작 · clone)★: ① flush 채널 unbounded send ② 동명 다수 skip 경로의
///   `tracing::debug!`. ★안전 근거 = 재진입 불가★ — 둘 중 어느 것도 `RosterDiff::inner` 로 되돌아올 수
///   없다(채널 수신자는 flush worker 고, tracing subscriber 는 이 타입을 모른다). 그래서 데드락은 구조적으로
///   없다. ★다만 "논블록" 은 **채널 쪽에만** 성립한다★: tracing 은 사용자가 꽂은 subscriber 로 나가는
///   호출이라 그쪽이 느린 writer 면(동기 파일/네트워크 appender) 이 락 구간이 그만큼 늘어난다 — 그 경로는
///   흔치 않은 skip 분기이고 데몬 기본 subscriber 는 논블록이지만, **"락 구간은 무조건 논블록" 으로 읽지 말 것**.
///   실제 flush(messaging 락 + port 호출 + blocking write)는 worker 스레드로 옮겨져 이 락 **밖**에 있다.
pub struct MessagingFlushSink {
    /// ★Box<dyn>★: 운영은 DaemonStatusSink(프론트 broadcast), 통합 테스트는 NoopSink 를 감싸 flush 만
    ///   검증한다 — 감싼 대상이 무엇이든 diff/flush 로직은 동일하므로 trait object 로 받는다.
    inner: Box<dyn StatusSink>,
    /// flush 작업(FlushMsg)을 flush worker 로 보내는 채널(unbounded — status 콜백을 절대 막지 않게).
    ///   worker 미가동/드롭이어도 send 실패는 무시(파킹은 다음 등장에 재시도 — 무손실 유지).
    flush_tx: mpsc::UnboundedSender<FlushMsg>,
    /// 로스터 diff 시퀀싱 상태(스냅샷 + enqueue 직렬화).
    diff: RosterDiff,
    /// 턴 종료 push(코어 `StatusSink::turn_ended`)를 도어벨로 옮기는 출구 — coalescer 를 통지 측과 공유한다.
    idle: ChannelIdleNotifier,
}

/// ★로스터 diff 시퀀싱 상태(C2 리뷰 fix 7)★ — 직전 스냅샷을 락 아래 두고, 그 락을 **든 채로 채널
///   enqueue 까지** 끝낸다.
///
/// ★왜 락 보유 중 send 인가(load-bearing)★: 스냅샷을 락 안에서 갱신하고 락을 놓은 뒤 send 하면,
///   `agent_list_updated` 콜백 둘이 동시에 들어올 때(코어는 이 콜백의 직렬화를 보장하지 않는다) 스냅샷
///   갱신 순서와 enqueue 순서가 **갈릴 수 있다** — 옛 스냅샷이 만든 Appear 가 새 스냅샷의 것보다 뒤에
///   도착해 사라진 incarnation 으로 flush 를 건다. enqueue 를 락 안으로 넣으면 "스냅샷 순서 = 채널 순서"
///   가 구조적으로 보장된다. unbounded send 는 논블록이라 락 보유 구간이 여전히 짧다(콜백 blocking 금지
///   규율 유지).
#[derive(Debug, Default)]
pub struct RosterDiff {
    inner: Mutex<RosterSnapshots>,
}

#[derive(Debug, Default)]
struct RosterSnapshots {
    prev: HashMap<String, (u32, AgentId)>,
}

impl RosterDiff {
    pub fn new() -> Self {
        Self::default()
    }

    /// 로스터 업데이트 1회 처리 — diff 를 계산해 **락 보유 중** 순서대로 enqueue 한다.
    fn dispatch(&self, agents: &[CoreAgentInfo], flush_tx: &mpsc::UnboundedSender<FlushMsg>) {
        let mut st = self.inner.lock().expect("flush roster diff poisoned");

        // 1) ★산(Running|Exiting) 후보 전원★을 **이름별로 그룹핑**한다. ★4차 개정(ADR-0116 결정 7)★:
        //   여기엔 **structured 조건을 걸지 않는다** — 로스터 자격에서 capability 가 빠졌으므로 턴 신호 없는
        //   세션도 파킹을 들고 있을 수 있다(그 부류의 유일한 파킹 경로 = **주입 실패**, spec §5 분기 3).
        //   그 조건을 되살리면 그 파킹분에 재등장 flush 계기가 **영원히 없어** 24h TTL 로 조용히 만료된다.
        // ★finding 2(BLOCK): 동명 다수 skip(last-write-wins 금지)★: 예전엔 같은 이름을 마지막 것으로
        //   덮어(last-write-wins) 임의 incarnation 으로 flush 했다 — 이름-키 파킹이 엉뚱한 동명 에이전트로
        //   갈 수 있어 send-side RECIPIENT_AMBIGUOUS 정책과 어긋난다. 이제 그 이름을 지닌 도달 가능
        //   후보가 **정확히 1개**일 때만 flush 대상으로 삼고, 동명 다수는 건너뛴다(tracing::debug) — 파킹
        //   메일은 그 이름이 다시 유일해지거나 TTL 로 만료될 때까지 대기한다.
        let mut by_name: HashMap<String, Vec<(u32, AgentId)>> = HashMap::new();
        for a in agents {
            if !crate::messaging_host::is_live(a) {
                continue;
            }
            by_name
                .entry(a.name.clone())
                .or_default()
                .push((a.epoch, a.id));
        }
        // 2) 동명 다수는 prev 에도 안 남긴다 — 다시 유일해지면 "새로 등장" 으로 잡혀 flush 되게.
        let mut next: HashMap<String, (u32, AgentId)> = HashMap::new();
        for (name, candidates) in by_name {
            if candidates.len() != 1 {
                tracing::debug!(
                    name = %name,
                    count = candidates.len(),
                    "flush skip: 동명 도달 후보 다수 — 유일해질 때까지 파킹 대기(finding 2)"
                );
                continue;
            }
            next.insert(name, candidates[0]);
        }
        for (name, (epoch, id)) in &next {
            // ★flush 트리거 조건(finding 3 — id 반영)★: ① 새로 등장(이전에 없던 이름/동명 해소로 재-유일)
            //   OR ② **id 변경**(같은 이름의 **다른** 에이전트 = 새 AgentId — 예: 같은 이름의 새 프로필)
            //   OR ③ 같은 id + epoch bump(같은 incarnation 재스폰/재활성화). ②가 load-bearing: 옛 diff 는
            //   이름별 epoch 만 비교해, id 가 다른데 epoch 이 이전 것보다 ≤ 이면(새 프로필 epoch 0 < 옛
            //   epoch 3) "새로 살아남" 을 놓쳐 그 이름 앞 파킹이 영영 stranded 됐다. id 가 바뀌면 그건
            //   별개 에이전트의 등장이니 epoch 대소와 무관하게 flush 후보다.
            let trigger = match st.prev.get(name) {
                None => true, // ① 새로 등장(또는 동명 해소로 다시 유일).
                Some((prev_epoch, prev_id)) => {
                    id != prev_id // ② 동명 다른 에이전트(새 AgentId) — epoch 대소와 무관.
                        || epoch > prev_epoch // ③ 같은 id + epoch bump(재스폰/재활성화).
                }
            };
            if trigger {
                let _ = flush_tx.send(FlushMsg::Appear {
                    name: name.clone(),
                    id: *id,
                });
            }
        }
        st.prev = next;
    }
}

impl MessagingFlushSink {
    /// 운영 생성자 — `idle` 은 flush 레인과 **공유하는** Idle coalescer 다.
    pub fn new(
        inner: DaemonStatusSink,
        flush_tx: mpsc::UnboundedSender<FlushMsg>,
        idle: Arc<IdleCoalescer>,
    ) -> Self {
        Self::new_boxed(Box::new(inner), flush_tx, idle)
    }

    /// 테스트 생성자 — 임의 inner StatusSink(NoopSink 등)를 감싼다. flush 로직만 검증할 때.
    pub fn new_test(
        inner: Box<dyn StatusSink>,
        flush_tx: mpsc::UnboundedSender<FlushMsg>,
        idle: Arc<IdleCoalescer>,
    ) -> Self {
        Self::new_boxed(inner, flush_tx, idle)
    }

    fn new_boxed(
        inner: Box<dyn StatusSink>,
        flush_tx: mpsc::UnboundedSender<FlushMsg>,
        idle: Arc<IdleCoalescer>,
    ) -> Self {
        Self {
            idle: ChannelIdleNotifier::new(flush_tx.clone(), idle),
            inner,
            flush_tx,
            diff: RosterDiff::new(),
        }
    }
}

/// ★2-레인 파이프라인(C2 리뷰 fix 3 — head-of-line blocking 격리, load-bearing)★:
///   - **main lane(이 함수)** — 채널에서 꺼내 flush 레인으로 **forward** 만 한다(논블록). 여기가 막히지
///     않아야 status 콜백이 넣은 작업이 계속 흡수된다.
///   - **flush lane(`run_flush_lane`)** — 자체 task + 자체 채널. `spawn_blocking` 이 자식 stdin write 로
///     막히는 곳이 여기다. 레인 내부는 **여전히 직렬**이라 같은 수신자 배치 순서(오래된 순)는 보존된다
///     (병렬화하면 순서가 깨진다).
///
/// ★레인 task 는 **호출자(부팅)가 소유**한다 — 이 함수가 spawn 하지 않는다(round-3 finding 1, BLOCK)★:
///   레인을 이 future **안에서** spawn 하고 JoinHandle 을 지역 변수로 들면, 종료 경로가 main lane 을 abort
///   할 때 그 핸들이 **그냥 drop**(= detach, abort 아님)되므로 레인은 계속 살아 있고 lib.rs 의 5s join belt 는
///   **정작 blocking 작업이 없는** main lane 만 감시하게 된다(모든 blocking inject 는 레인에 있다) → 진짜
///   blocking 을 지닌 task 가 belt 밖에 남아 런타임 drop 이 종료 시점에 hang 할 수 있다. 그래서 두 task 를
///   **둘 다 호출자가 들고** 각각 abort + belt 로 내린다(`spawn_flush_worker` / `FlushWorkerHandles::shutdown`).
/// ★수명★: main lane 이 끝나면(또는 abort 되면) `lane_tx` 가 drop 되어 레인도 자연 종료한다 — 단 레인은
///   **큐에 남은 배달을 다 처리한 뒤** 끝나므로 즉시 멈추지 않는다.
pub async fn run_flush_worker(
    mut flush_rx: mpsc::UnboundedReceiver<FlushMsg>,
    lane_tx: mpsc::UnboundedSender<FlushMsg>,
) {
    while let Some(msg) = flush_rx.recv().await {
        match msg {
            other @ (FlushMsg::Appear { .. } | FlushMsg::Idle { .. }) => {
                // ★send 실패를 삼키지 않는다(round-3 finding 1)★: 레인이 죽었다면(패닉 등) 이 경로의 조용한
                //   `let _ =` 는 **모든 배달이 영구 정지**한 사실을 감춘다(파킹만 쌓이다 TTL 로 만료 —
                //   메시징 최악 실패 모드인데 로그 한 줄도 없다). 그래서 warn 으로 표면화한다. 여기서
                //   복구(레인 재기동)는 하지 않는다 — 레인은 개별 flush 의 패닉을 격리하므로(run_flush_lane,
                //   단 **debug 한정** — release 는 panic=abort 라 패닉 = 프로세스 종료다) 이 실패는
                //   "종료 중" 이거나 진짜 버그이고, 후자면 로그가 유일한 단서다.
                if let Err(e) = lane_tx.send(other) {
                    tracing::warn!("flush 레인 forward 실패(레인 종료/패닉 — 이후 배달 정지): {e}");
                }
            }
        }
    }
    tracing::debug!("flush worker(main lane) 종료(채널 닫힘)");
}

/// flush worker **2-레인 묶음** 핸들 — 부팅(운영 `run()` / 테스트 서버)이 두 task 를 함께 소유한다.
///
/// ★왜 묶음인가(round-3 finding 1)★: 레인이 detach 되면 종료 belt 가 무의미해진다(위 `run_flush_worker`
///   주석). 조립·종료를 한 타입에 모아 호출자가 **한쪽만 내리는 실수**를 구조적으로 못 하게 한다.
pub struct FlushWorkerHandles {
    /// 수신 레인 — 채널에서 꺼내 배달 레인으로 넘기기만 한다(blocking 작업 없음).
    main: tokio::task::JoinHandle<()>,
    /// 배달 레인(Appear/Idle) — 여기 `spawn_blocking` 이 자식 stdin write 로 막힐 수 있다.
    lane: tokio::task::JoinHandle<()>,
}

impl FlushWorkerHandles {
    /// 두 레인을 내린다 — **호출 전에 `shutdown_all`(자식 kill·파이프 닫기)이 끝나 있어야 한다**(순서가
    ///   load-bearing: lib.rs 종료 주석). 각 abort 뒤 join 을 5s belt 로 감싸, 예측 못 한 blocking 이
    ///   남아도 데몬 종료를 hang 시키지 않고 warn 후 detach 한다(프로세스 종료가 스레드를 회수).
    ///
    /// ★수용된 잔여(residual) — abort·belt 로 `spawn_blocking` **본문**을 끊을 수는 없다(round-4 finding 2)★:
    ///   abort 는 task 의 `.await` 지점에서만 먹으므로 blocking pool 스레드가 syscall 안에 있으면 그대로 돈다.
    ///   그래도 이 종료 경로가 hang 하지 않는 이유는 belt 가 아니라 **호출 순서**다:
    ///     ① 여기 오기 전에 `shutdown_all` 이 끝나 있다 → 자식이 kill 되고 stdin 파이프가 닫힌다 → 그 파이프에
    ///        막혀 있던 `inject`(동기 write_all+flush)가 **에러로 풀려** 클로저가 스스로 반환한다.
    ///     ② 배달 레인 밖에는 blocking 작업이 없다(main lane 은 forward 만 한다).
    ///   즉 **kill-first 순서가 실제 보증이고, abort + 5s belt 는 관측 장치**다 — belt 가 실제로 발화한다면
    ///   그건 위 두 전제 중 하나가 깨졌다는 신호(warn 로그)이고, 그러려면 kill 된 자식의 파이프 write 가 에러도
    ///   안 내고 영원히 blocking 하는 **병리적 OS 동작**이 필요하다. 그래서 여기서 더 강한 취소 수단(별도
    ///   프로세스·스레드 강제 종료 등)을 도입하지 않는다 — 비용은 크고 막는 실패는 가정상 존재하지 않는다.
    pub async fn shutdown(self) {
        // main 먼저 — abort 시 lane_tx 가 drop 되어 레인이 새 작업을 받지 않는다.
        self.main.abort();
        if tokio::time::timeout(FLUSH_JOIN_BELT, self.main)
            .await
            .is_err()
        {
            tracing::warn!("flush worker(main lane) 종료 {FLUSH_JOIN_BELT:?} 타임아웃 — detach");
        }
        // 그 다음 배달 레인. 여기 남은 배달은 버린다(파킹은 인메모리 — 프로세스와 함께 소멸, spec §0).
        self.lane.abort();
        if tokio::time::timeout(FLUSH_JOIN_BELT, self.lane)
            .await
            .is_err()
        {
            tracing::warn!("flush 레인 종료 {FLUSH_JOIN_BELT:?} 타임아웃 — detach(종료 hang 방지)");
        }
    }
}

/// 레인 채널은 **unbounded** 다: forward 가 논블록이어야 한다(main lane 이 레인 진행을 기다리면 head-of-line
///   blocking 이 되살아난다 — `run_flush_worker` 헤더).
pub fn spawn_flush_worker(
    flush_rx: mpsc::UnboundedReceiver<FlushMsg>,
    wiring: FlushWiring,
) -> FlushWorkerHandles {
    let (lane_tx, lane_rx) = mpsc::unbounded_channel::<FlushMsg>();
    let lane = tokio::spawn(run_flush_lane(lane_rx, wiring.messaging, wiring.idle));
    let main = tokio::spawn(run_flush_worker(flush_rx, lane_tx));
    FlushWorkerHandles { main, lane }
}

/// 종료 join belt — abort 후 이 시간 안에 안 끝나면 warn 후 detach(데몬 종료 hang 방지, round-3 finding 1).
const FLUSH_JOIN_BELT: Duration = Duration::from_secs(5);

/// ★flush 레인(C2 리뷰 fix 3)★ — 배달 작업(Appear/Idle) 전용 **직렬** 소비자. 여기의 blocking write 가
///   막혀도 수신 레인은 계속 돌아 채널을 비운다.
///
/// ★직렬 유지가 load-bearing★: 같은 수신자의 배치는 "오래된 순" 을 지켜야 하므로(ADR-0104) 이 레인 안에서
///   병렬 실행하지 않는다. 서로 다른 수신자끼리도 직렬이라 한 막힌 수신자가 다른 수신자의 배달을 늦출 수는
///   있으나(수용된 잔여 — 사람 대화 수준 메시지율), 채널 수신 자체는 그 뒤에 서지 않는다.
///
/// ★spawn_blocking 격리(round-4 finding 1 — executor starvation)★: `flush_for` 안의 `inject` 는
///   transport.send_input = **동기 blocking write_all+flush**다(논블록 채널 send 가 아니라 실제 자식
///   stdin 파이프 write — `PtyTransport`/`StdioTransport::send_input`). 이걸 async task 본문에서 **직접** 부르면
///   그 blocking 이 runtime worker 스레드를 점유한다 — current-thread/단일 worker 런타임에선 이 한 task 가
///   executor 를 독점해 다른 task(종료 시 shutdown_all·5s join belt 등)가 폴링될 틈이 없다(실제로 통합
///   테스트가 이 굶주림 때문에 multi_thread 로 우회해야 했다). 그래서 각 flush 를 `spawn_blocking` 으로
///   던져 blocking pool 스레드에서 돌린다: (1) blocking write 는 runtime worker 가 아닌 blocking pool 을
///   점유하고 (2) abort 는 아래 `.await` 지점에서 즉시 먹으며 (3) 5s join belt·종료 task 가 계속 폴링돼
///   current-thread 런타임도 건강하게 유지된다. 이 fix 가 고치는 건 executor 굶주림뿐이다 — 종료 순서
///   의존은 그대로 남는다(`FlushWorkerHandles::shutdown`).
async fn run_flush_lane(
    mut lane_rx: mpsc::UnboundedReceiver<FlushMsg>,
    messaging: Arc<crate::control::mcp_server::MessagingSlot>,
    idle: Arc<IdleCoalescer>,
) {
    while let Some(msg) = lane_rx.recv().await {
        match msg {
            FlushMsg::Appear { name, id } => {
                let Some(svc) = messaging.get() else {
                    // 서비스 미주입(부팅 초기) — 파킹이 없으니 스킵. 다음 등장 이벤트에 다시 온다.
                    continue;
                };
                let svc = svc.clone();
                let join = tokio::task::spawn_blocking(move || svc.flush_for(&name, id));
                if let Err(e) = join.await {
                    // blocking task 실패(패닉 또는 취소). 레인은 죽지 않고 다음 대상으로 계속 — 한 flush 의
                    //   실패가 이후 배달을 막지 않게(유계 격리).
                    // ★release 빌드엔 **패닉 갈래가 없다**(리뷰 fix 9 — 옛 주석 보정)★: 워크스페이스
                    //   `[profile.release] panic = "abort"` 라 blocking task 가 패닉하면 프로세스가 즉시
                    //   죽는다(JoinError::is_panic 을 볼 기회 자체가 없다). 즉 이 격리가 실제로 작동하는
                    //   건 **debug/테스트 빌드**이고, release 에서 이 갈래는 사실상 abort(Cancelled) —
                    //   런타임 종료 중 — 뿐이다. "패닉해도 운영에서 레인이 살아남는다" 로 읽지 말 것.
                    tracing::warn!("flush blocking task 실패(레인 계속 — 패닉 격리는 debug 한정, release=abort): {e}");
                }
            }
            FlushMsg::Idle { id } => {
                // ★집어들 때 coalescing 집합에서 먼저 지운다(fix 10 — lost wakeup 방지)★: 이 flush 처리
                //   도중 도착하는 새 MessageDone 은 다시 enqueue 돼야 한다(그 턴 종료는 이 배치가 대표하지
                //   않는다). 지우는 시점이 flush **전**인 게 핵심이다.
                idle.taken(id);
                let Some(svc) = messaging.get() else {
                    continue;
                };
                let svc = svc.clone();
                let join = tokio::task::spawn_blocking(move || svc.flush_for_agent(id));
                if let Err(e) = join.await {
                    tracing::warn!("idle flush blocking task 실패(레인 계속 — 패닉 격리는 debug 한정, release=abort): {e}");
                }
            }
        }
    }
    tracing::debug!("flush 레인 종료(채널 닫힘)");
}

/// flush worker 배선 묶음 — 인자 수를 줄이고 "무엇을 공유하는가" 를 한눈에 보이게 한다(부팅에서 조립).
#[derive(Clone)]
pub struct FlushWiring {
    pub messaging: Arc<crate::control::mcp_server::MessagingSlot>,
    /// Idle coalescing 집합 — 통지 측(`MessagingFlushSink`)과 공유(집어들 때 해제).
    pub idle: Arc<IdleCoalescer>,
}

impl StatusSink for MessagingFlushSink {
    fn status_changed(&self, id: AgentId, status: CoreStatus, epoch: u32) {
        self.inner.status_changed(id, status, epoch);
    }

    fn agent_list_updated(&self, agents: Vec<CoreAgentInfo>) {
        self.diff.dispatch(&agents, &self.flush_tx);
        self.inner.agent_list_updated(agents);
    }

    fn restore_result(&self, report: CoreRestoreReport) {
        self.inner.restore_result(report);
    }

    /// ★턴 종료 push → flush 도어벨(ADR-0113 결정 3 — 데몬은 중계만)★. 여기서 하는 일은 coalescing
    ///   판정 + 논블록 채널 send 뿐이다(그 계약은 `StatusSink::turn_ended`).
    // ADR-0113
    fn turn_ended(&self, id: AgentId, epoch: u32) {
        // epoch 을 도어벨에 싣지 않는 이유: flush 는 **에이전트 단위** 큐를 여는 동작이고, 어느 화신이
        //   끝났든 그 시점의 현재 화신에게 배달하는 게 맞다(메일은 논리 에이전트를 향한다 — ADR-0086 §F5).
        self.idle.enqueue(id);
        // ★감싼 sink 로도 반드시 흘린다(decorator 계약)★: 이 wrapper 가 데몬이 설치하는 **유일한**
        //   StatusSink 이고 이 훅은 기본 구현이 no-op 이라, 빠뜨리면 안쪽이 이 훅을 구현하는 날
        //   **컴파일 에러 없이** 조용히 죽는다. 턴 상태를 프론트/LLM 제어 표면으로 내보내는 경로가
        //   그 안쪽에 생길 예정이다(ADR-0113 §영향 — §5 정합).
        self.inner.turn_ended(id, epoch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_core::agent::types::AgentId;

    // ── 턴 사실 어댑터 ──────────────────────────────────────────────────────────────────────

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

        /// ★셸은 편지를 읽는 주체가 아니다(사용자 결정 2026-08-17)★ — 셸에 도착한 봉투는 읽히는 게
        /// 아니라 명령으로 실행되므로 수신자 명단에서 뺀다(`@all` 자동 포함도 이걸로 닫힌다).
        ///
        /// ★삭제 정리 게이트(`is_agent_live`)는 **같이 걸지 않는다**★: 그 게이트가 묻는 건 "이 세션이
        ///   지금 살아 있나" 지 "편지를 받나" 가 아니다. 함께 걸면 셸 프로필을 지울 때 다른 축의 정리가
        ///   엉뚱하게 발동한다 — 그래서 두 축이 갈린다는 사실을 여기서 못 박는다.
        /// ★멤버십 술어의 capability 무관성(ADR-0116 결정 1·7)은 별도 테스트가 지킨다★
        ///   (`tests/control_send.rs` — `roster_includes_a_terminal_agent_without_a_turn_signal_no_claude`)
        ///   — 이 fixture 는 셸이라 그 축을 더 이상 대변하지 못한다(터미널 claude 는 실 바이너리가 필요해
        ///   여기서 못 띄우고, 세션 주입 seam 은 통합 테스트 쪽에 있다).
        #[cfg(windows)]
        #[test]
        fn a_live_shell_is_excluded_from_the_mail_roster_but_still_counts_as_live() {
            let manager = manager("live");
            let port = ManagerDeliveryPort::new(manager.clone());
            let info = manager
                .spawn_agent(&shell("sheller"), SpawnMode::Fresh)
                .expect("shell spawn");
            assert!(wait_until(|| manager
                .list_agents()
                .iter()
                .any(|a| a.id == info.id)));

            assert!(
                !port.live_agents().iter().any(|a| a.id == info.id),
                "셸은 배달 명단에 오르면 안 된다(봉투가 명령으로 실행된다)"
            );
            let sources = port.addressing_sources();
            assert!(
                !sources.roster.iter().any(|a| a.id == info.id),
                "입구 판정 소스도 같은 술어여야(한쪽만 걸면 해석과 배달이 갈린다): {sources:?}"
            );
            assert!(
                port.is_agent_live(info.id),
                "삭제 정리 게이트는 '살아 있나' 축이라 그대로 true 여야(리뷰 fix D1)"
            );

            // ★실제로 뚫렸던 구멍★: `DeleteProfile` 은 산 세션을 죽이지 않는다. 판정을 프로필에서 뽑으면
            //   여기서 "모름" 이 되어 셸이 명단으로 되돌아온다 — 판정 근거가 세션에 있어야 이 단언이 산다.
            manager.delete_agent(info.id);
            assert!(
                !port.live_agents().iter().any(|a| a.id == info.id),
                "프로필이 사라져도 산 셸은 여전히 배달 명단 밖이어야"
            );
            assert!(
                !port
                    .addressing_sources()
                    .roster
                    .iter()
                    .any(|a| a.id == info.id),
                "입구 판정 소스도 마찬가지"
            );

            manager.kill_agent(info.id).ok();
        }

        // ADR-0116 결정 1·7(capability ≠ 멤버십)의 봉인은 어댑터 **산출 함수 두 개**를 실제로 태워야
        // 성립하므로 세션 주입 seam 이 있는 통합 테스트가 맡는다
        // (`tests/control_send.rs` — `roster_includes_a_terminal_agent_without_a_turn_signal_no_claude`).
        // ★여기서 `is_live` 만 부르는 테스트로 대신하지 말 것★: 그 술어는 상태 한 줄이라 capability 를
        //   읽지도 않아 항상 참이다(항진명제). 실제로 그렇게 썼다가 리뷰에서 걸렸다.

        #[test]
        fn two_dormant_profiles_sharing_a_name_are_both_reported() {
            let manager = manager("dormant-dup");
            let port = ManagerDeliveryPort::new(manager.clone());
            // ★하네스 seam 으로 심는다(ADR-0120)★: 정상 경로(`create_agent`)는 명부 전역 이름 유일성을
            //   강제해 동명 2건을 **만들 수 없다**. 유일성이 데이터 전체에 적용되기 전(기존 agents.json)엔
            //   이 상태가 실재할 수 있다.
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
            // ★전제가 바뀌었다★: 산 쪽은 셸이라 배달 명단엔 안 오른다(`reads_messages`). 그래도 이 테스트가
            //   보는 축은 그대로다 — 차집합은 **core `roster()`** 가 id 로 끝내고 어댑터는 그 뒤에서 거를
            //   뿐이라, 이름 축으로 빼는 회귀가 생기면 산 `twin` 이 잠든 `twin` 을 삼켜 아래가 비어 버린다.
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

    // ── 7. MessagingFlushSink diff/enqueue 로직 — worker 없이 순수 diff 검증 ──────────────────────
    use engram_dashboard_core::agent::types::{
        AgentInfo as TAgentInfo, Capabilities, ControlCaps, InputCaps, ModelCaps, OutputCaps,
        SessionCaps,
    };

    fn flush_info(
        id: AgentId,
        name: &str,
        epoch: u32,
        structured: bool,
        status: CoreStatus,
    ) -> TAgentInfo {
        TAgentInfo {
            id,
            name: name.to_string(),
            cwd: ".".to_string(),
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
                    terminal_bytes: !structured,
                    structured,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: false,
                    interrupt: false,
                    cancel: false,
                    graceful_shutdown: false,
                },
                session: SessionCaps {
                    resume: false,
                    snapshot: false,
                    cwd_env: false,
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

    /// 테스트용 no-op inner StatusSink — broadcast 는 무관(diff 만 검증).
    struct TestNoopSink;
    impl StatusSink for TestNoopSink {
        fn status_changed(&self, _: AgentId, _: CoreStatus, _: u32) {}
        fn agent_list_updated(&self, _: Vec<CoreAgentInfo>) {}
        fn restore_result(&self, _: CoreRestoreReport) {}
    }

    /// flush 작업만 뽑는 sink + 그 채널 수신단을 만든다(worker 미배선 — diff 만 관측).
    fn flush_sink() -> (MessagingFlushSink, mpsc::UnboundedReceiver<FlushMsg>) {
        let (tx, rx) = mpsc::unbounded_channel::<FlushMsg>();
        let sink = MessagingFlushSink::new_test(
            Box::new(TestNoopSink),
            tx,
            Arc::new(IdleCoalescer::new()),
        );
        (sink, rx)
    }

    fn drain_msgs(rx: &mut mpsc::UnboundedReceiver<FlushMsg>) -> Vec<FlushMsg> {
        let mut out = Vec::new();
        while let Ok(t) = rx.try_recv() {
            out.push(t);
        }
        out
    }

    /// 이름 축 flush 대상(Appear)만 추린다 — C1 diff 단언을 그대로 유지하기 위한 필터.
    fn drain_targets(rx: &mut mpsc::UnboundedReceiver<FlushMsg>) -> Vec<(String, AgentId)> {
        drain_msgs(rx)
            .into_iter()
            .filter_map(|m| match m {
                FlushMsg::Appear { name, id } => Some((name, id)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn flush_sink_enqueues_newly_live_unique_name() {
        let (sink, mut rx) = flush_sink();
        let id = AgentId::new_v4();
        sink.agent_list_updated(vec![flush_info(id, "alice", 0, true, CoreStatus::Running)]);
        let targets = drain_targets(&mut rx);
        assert_eq!(
            targets,
            vec![("alice".to_string(), id)],
            "유일 등장 이름 flush"
        );
    }

    #[test]
    fn flush_sink_skips_ambiguous_name() {
        let (sink, mut rx) = flush_sink();
        let a = AgentId::new_v4();
        let b = AgentId::new_v4();
        sink.agent_list_updated(vec![
            flush_info(a, "dup", 0, true, CoreStatus::Running),
            flush_info(b, "dup", 0, true, CoreStatus::Running),
        ]);
        assert!(
            drain_targets(&mut rx).is_empty(),
            "동명 다수는 flush 대상에서 제외(임의 incarnation 배달 금지)"
        );
    }

    #[test]
    fn flush_sink_reflushes_when_name_becomes_unique_again() {
        let (sink, mut rx) = flush_sink();
        let a = AgentId::new_v4();
        let b = AgentId::new_v4();
        sink.agent_list_updated(vec![
            flush_info(a, "dup", 0, true, CoreStatus::Running),
            flush_info(b, "dup", 0, true, CoreStatus::Running),
        ]);
        assert!(drain_targets(&mut rx).is_empty());
        sink.agent_list_updated(vec![flush_info(a, "dup", 0, true, CoreStatus::Running)]);
        assert_eq!(
            drain_targets(&mut rx),
            vec![("dup".to_string(), a)],
            "동명 해소로 유일해지면 다시 flush 대상"
        );
    }

    #[test]
    fn flush_sink_enqueues_on_epoch_bump_but_not_same_epoch() {
        let (sink, mut rx) = flush_sink();
        let id = AgentId::new_v4();
        sink.agent_list_updated(vec![flush_info(id, "a", 0, true, CoreStatus::Running)]);
        assert_eq!(drain_targets(&mut rx), vec![("a".to_string(), id)]);
        sink.agent_list_updated(vec![flush_info(id, "a", 0, true, CoreStatus::Running)]);
        assert!(
            drain_targets(&mut rx).is_empty(),
            "같은 epoch 재-push 는 flush 안 함"
        );
        sink.agent_list_updated(vec![flush_info(id, "a", 1, true, CoreStatus::Running)]);
        assert_eq!(
            drain_targets(&mut rx),
            vec![("a".to_string(), id)],
            "epoch bump 은 flush(재스폰/재활성화)"
        );
    }

    #[test]
    fn flush_sink_enqueues_when_same_name_different_id_lower_epoch() {
        let (sink, mut rx) = flush_sink();
        let old = AgentId::new_v4();
        let new = AgentId::new_v4();
        sink.agent_list_updated(vec![flush_info(old, "svc", 3, true, CoreStatus::Running)]);
        assert_eq!(drain_targets(&mut rx), vec![("svc".to_string(), old)]);
        sink.agent_list_updated(vec![flush_info(new, "svc", 0, true, CoreStatus::Running)]);
        assert_eq!(
            drain_targets(&mut rx),
            vec![("svc".to_string(), new)],
            "동명 다른 에이전트(새 id)는 epoch 이 낮아도 flush(finding 3)"
        );
        sink.agent_list_updated(vec![flush_info(new, "svc", 0, true, CoreStatus::Running)]);
        assert!(
            drain_targets(&mut rx).is_empty(),
            "같은 id+epoch 재-push 는 flush 안 함"
        );
    }

    #[test]
    fn flush_sink_appears_for_a_turn_signal_less_agent_and_ignores_the_dead() {
        let (sink, mut rx) = flush_sink();
        let tui = AgentId::new_v4();
        let dead = AgentId::new_v4();
        sink.agent_list_updated(vec![
            flush_info(tui, "tui", 0, false, CoreStatus::Running), // 비-structured = 턴 신호 없음
            flush_info(dead, "dead", 0, true, CoreStatus::Killed), // terminal
        ]);
        assert_eq!(
            drain_msgs(&mut rx),
            vec![FlushMsg::Appear {
                name: "tui".to_string(),
                id: tui,
            }],
            "턴 신호 없는 산 세션 = Appear · terminal 은 아무것도 아님"
        );
    }

    // ── 7b. 턴 종료 push → 도어벨(ADR-0113 — 데몬은 중계만) ─────────────────────────────

    #[test]
    fn a_turn_end_push_from_the_core_becomes_an_idle_doorbell() {
        let (sink, mut rx) = flush_sink();
        let id = AgentId::new_v4();
        sink.turn_ended(id, 3);
        assert_eq!(drain_msgs(&mut rx), vec![FlushMsg::Idle { id }]);
    }

    #[test]
    fn turn_end_pushes_are_coalesced_per_agent_until_taken() {
        let (sink, mut rx) = flush_sink();
        let id = AgentId::new_v4();
        sink.turn_ended(id, 0);
        sink.turn_ended(id, 0);
        sink.turn_ended(id, 1);
        assert_eq!(
            drain_msgs(&mut rx),
            vec![FlushMsg::Idle { id }],
            "미처리분이 있으면 접는다(소비자가 집어들면 다시 열린다 — IdleCoalescer)"
        );
    }

    #[test]
    fn idle_coalescer_folds_pending_notifications_until_taken() {
        use engram_dashboard_messaging::busy::IdleNotifier;
        let (tx, mut rx) = mpsc::unbounded_channel::<FlushMsg>();
        let coalescer = Arc::new(IdleCoalescer::new());
        let notifier = ChannelIdleNotifier::new(tx, coalescer.clone());
        let a = AgentId::new_v4();
        let b = AgentId::new_v4();
        notifier.notify_idle(a);
        notifier.notify_idle(a);
        notifier.notify_idle(a);
        notifier.notify_idle(b);
        assert_eq!(
            drain_msgs(&mut rx),
            vec![FlushMsg::Idle { id: a }, FlushMsg::Idle { id: b }],
            "id 별로 미처리 1건씩만(다른 id 는 서로 접히지 않는다)"
        );
        coalescer.taken(a);
        notifier.notify_idle(a);
        assert_eq!(drain_msgs(&mut rx), vec![FlushMsg::Idle { id: a }]);
    }

    #[test]
    fn service_doorbell_shares_the_idle_channel_and_coalescing() {
        use engram_dashboard_messaging::service::FlushTrigger;
        let (tx, mut rx) = mpsc::unbounded_channel::<FlushMsg>();
        let coalescer = Arc::new(IdleCoalescer::new());
        let notifier = ChannelIdleNotifier::new(tx, coalescer.clone());
        let id = AgentId::new_v4();
        notifier.request_flush(id);
        notifier.request_flush(id);
        assert_eq!(drain_msgs(&mut rx), vec![FlushMsg::Idle { id }]);
    }

    // ── 9b. flush worker: 2-레인 소유/종료(round-3 finding 1) ────────────────────────────────

    fn lane_wiring() -> FlushWiring {
        FlushWiring {
            messaging: Arc::new(crate::control::mcp_server::MessagingSlot::new()),
            idle: Arc::new(IdleCoalescer::new()),
        }
    }

    #[tokio::test]
    async fn flush_worker_handles_shutdown_stops_both_lanes() {
        let (tx, rx) = mpsc::unbounded_channel::<FlushMsg>();
        let handles = spawn_flush_worker(rx, lane_wiring());
        // 배달 작업 1건을 넣어 레인이 실제로 돌게 한다(messaging slot 미주입이라 즉시 skip = 결정적).
        tx.send(FlushMsg::Idle {
            id: AgentId::new_v4(),
        })
        .expect("수신 레인 수신");
        tokio::time::timeout(Duration::from_secs(6), handles.shutdown())
            .await
            .expect("shutdown 이 belt 안에 반환해야");
        assert!(
            tx.send(FlushMsg::Idle {
                id: AgentId::new_v4()
            })
            .is_err(),
            "shutdown 후 수신 레인은 더 이상 수신하지 않는다"
        );
    }
}
