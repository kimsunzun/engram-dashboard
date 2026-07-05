//! OutputRouter — agent_id → window-label 라우팅 테이블(lock-free 핫패스) + 구독 union diff.
//! S14 모듈①(ADR-0036) T5. **순수 로직 — Tauri 런타임 의존 0**(headless 단독 테스트 가능).
//!
//! ## 역할
//! 데몬에서 온 출력 프레임(`DecodedFrame{agent_id, ...}`)을 그 agent 가 현재 화면에 보이는
//! **모든 창(window label)** 으로 fan-out 하기 위한 라우팅 표를 제공한다. 표의 권위 소스는
//! `ViewManager`(ADR-0035) — 각 View 의 split 트리에서 agent 가 박힌 Slot 을 찾아
//! `agent_id → [window_label]` 로 역인덱싱한다.
//!
//! ## 핵심 불변식
//! - **핫패스(`targets`) 락 0** (ADR-0006): `ArcSwap::load()` 만 — 프레임마다 호출되므로 Mutex 금지.
//!   할당·재계산은 전부 저빈도 경로(`rebuild`)가 한다.
//! - **rebuild-always**(D2): 레이아웃 변경은 저빈도라 매 변경 시 snapshot 전체를 재계산하고
//!   `ArcSwap::store` 로 원자 교체한다. version-cache 분기 불채택(복잡도 대비 무이득).
//! - **AgentKey = `AgentId`(= `uuid::Uuid`)**: 프레임 `agent_id` 가 이미 `AgentId` 라 핫패스 변환 0.
//!   Slot 은 `agent_id: Option<String>` 을 저장하므로 **rebuild(저빈도)에서 String→Uuid 파싱**해
//!   경계에서 정규화한다. 파싱 실패 슬롯은 어차피 실 프레임과 매칭 불가라 무시. 이 선택은
//!   `protocol_state` 의 기존 `HashMap<AgentId, SubState>` 키와도 정합(연결 task 가 같은 키로 dedup).
//! - **F-B 구독 union = layout 파생**(ADR-0035, spike §8 F-B): 별도 ref-count 맵 없이 snapshot 의
//!   agent 집합 자체가 "현재 구독해야 할 집합"이다. rebuild 시 직전 집합과 diff 해서 0→1(Subscribe)·
//!   1→0(Unsubscribe) 델타를 **같은 트리 순회에 piggyback** 으로 산출한다(단일 패스). 실제 송신은 T6.
//!
//! ## T5 범위 밖(T6 이후 — 여기서 배선하지 않음)
//! - Tauri `Channel`/IPC, 창으로의 실제 전송, `commands/agent.rs`, `connection.rs` main_loop 편집.
//! - 트리거 배선: rebuild 호출은 T6 가 layout command 의 **ViewManager 락 보유 critical section 안**에서
//!   (layout mutation 직후, 같은 락으로 `router.rebuild(&mgr)` → table+delta 산출. RMW 직렬화 — FIX-1)
//!   하고, **델타 송신만 락 해제 후** cmd_tx 로 enqueue 한다. `targets` 사용은 `connection.rs:668`
//!   `Message::Binary` 자리에서, 델타→cmd_tx 송신도 T6.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use arc_swap::ArcSwap;

use crate::layout::manager::ViewManager;
use crate::layout::types::LayoutNode;

/// 캐시된 빈 라우팅 결과(미스 시 반환). `Arc::from(Vec::new())` 는 매번 Arc 헤더(refcount 블록)를
/// 힙에 할당한다 — 미스가 핫패스에서 빈번하면 그게 누적 비용이다. 한 번만 만들어 `Arc::clone`(refcount
/// +1, 할당 0)으로 돌려 미스 경로를 진짜 zero-alloc 으로 만든다(FIX-2).
static EMPTY_TARGETS: LazyLock<Arc<[WindowLabel]>> = LazyLock::new(|| Arc::from(Vec::new()));

/// 라우팅 키. 프레임 `agent_id`(= `engram_dashboard_protocol::AgentId` = `uuid::Uuid`)와 동일 타입.
/// Slot 의 `String` 은 rebuild 경계에서 이 타입으로 파싱·정규화한다(위 모듈 주석 AgentKey 결정).
pub type AgentKey = uuid::Uuid;

/// Tauri window label(예: "main", "slot-popup"). `window_bindings` 키와 동일 타입 → 별도 numeric
/// 레지스트리 불필요(spike §8 D1).
pub type WindowLabel = String;

/// 메인 창의 활성 View 를 가리키는 window label. active_view_id 에 박힌 agent 는 이 창으로 간다.
const MAIN_WINDOW_LABEL: &str = "main";

/// 라우팅 스냅샷. `Arc<[WindowLabel]>` 로 값을 공유 — 핫패스에서 clone 해도 포인터 복사뿐(요소 복사 X).
///
/// `by_agent` 에 없는 agent = 현재 어느 창에도 안 보임 → `targets` 가 빈 슬라이스 반환(전송 대상 0).
#[derive(Debug, Default)]
pub struct RoutingSnapshot {
    pub by_agent: HashMap<AgentKey, Arc<[WindowLabel]>>,
}

/// rebuild 1회의 구독 델타(F-B). 직전 snapshot 대비 agent-union diff 를 한 번의 트리 순회에서 산출한다.
///
/// ## agent 단위 diff
/// - `to_subscribe`: 이번에 새로 보이기 시작(0→1 agent). ★ADR-0046: layout 은 이걸로 wire Subscribe 를
///   **보내지 않는다**★ — wire 구독 형성은 뷰 주도 `request_replay` 단독이다(BLOCK-1 전면화). 델타 산출은
///   유지해 diff 보존 불변식 테스트/미래 진단에 쓴다.
/// - `to_unsubscribe`: 더 이상 어느 창에도 안 보임(1→0 agent) → 라우터가 wire `Unsubscribe` 를 발행(정리).
///
/// ★ADR-0046 — 축 B 제거★: 옛 slot=(window,agent) 단위 cursor 델타(`slots_to_replay`/`slots_to_drop`)는
///   미러 버퍼(cursor)와 함께 삭제됐다. remount/새 창은 데몬 ring 전량 재replay(뷰 주도)로 대체한다.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SubscriptionDelta {
    pub to_subscribe: Vec<AgentKey>,
    pub to_unsubscribe: Vec<AgentKey>,
}

impl SubscriptionDelta {
    /// 두 축 모두 비었으면 변화 없음(트리거 측이 송신 스킵 판단에 쓸 수 있게).
    pub fn is_empty(&self) -> bool {
        self.to_subscribe.is_empty() && self.to_unsubscribe.is_empty()
    }
}

/// agent_id → window 라우팅 테이블. 핫패스 읽기는 lock-free(`ArcSwap`), 재계산은 저빈도.
///
/// app-level 공유(재연결 task 수명을 넘어 산다) → `Arc<OutputRouter>` 로 manage/주입한다(T6).
pub struct OutputRouter {
    /// 핫패스가 `load()` 로 읽는 현재 스냅샷. rebuild 가 `store()` 로 통째 교체(부분 변경 없음).
    table: ArcSwap<RoutingSnapshot>,
}

impl Default for OutputRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputRouter {
    /// 빈 라우팅 테이블로 생성(어느 agent 도 매핑 없음 → 모든 `targets` 가 빈 슬라이스).
    pub fn new() -> Self {
        Self {
            table: ArcSwap::from_pointee(RoutingSnapshot::default()),
        }
    }

    /// ★핫패스★ agent 가 현재 보이는 모든 window label. 락 0 — `ArcSwap::load()` + `Arc` 포인터 clone 뿐.
    ///
    /// 반환은 `Arc<[WindowLabel]>` clone(요소 복사 X). 매핑 없으면 캐시된 빈 슬라이스 Arc 를 clone(할당 0,
    /// refcount +1 뿐 — `EMPTY_TARGETS` 단일 인스턴스). ★주의(spike §7 Pitfall)★: 반환 Arc 는 즉시 순회해
    /// 소비하라. (`load()` Guard 를 `.await` 너머로 들고 가면 슬롯 고갈 — 여기선 Guard 를 함수 안에서만 쓰고
    /// Arc 만 반환하므로 안전.)
    pub fn targets(&self, agent_id: AgentKey) -> Arc<[WindowLabel]> {
        let snap = self.table.load();
        match snap.by_agent.get(&agent_id) {
            Some(labels) => Arc::clone(labels),
            // 미스 = 캐시된 빈 Arc clone(할당 0). `Arc::from(Vec::new())` 는 매 미스마다 Arc 헤더를 새로
            // 할당하므로 쓰지 않는다(FIX-2).
            None => Arc::clone(&EMPTY_TARGETS),
        }
    }

    /// ★테스트 전용★: ViewManager 트리 구성 없이 라우팅 스냅샷을 직접 박는다. 각 agent 를 단일 "main" 창에
    /// 매핑한다. `pub(crate)` 라 crate 내 다른 테스트 모듈(daemon_client::tests)에서도 호출 가능.
    /// ★ADR-0046★: 옛 eager resubscribe(current_agents 순회)는 삭제됐다 — 헬퍼는 라우팅(targets) 테스트가
    ///   다시 쓸 수 있어 남긴다(현재는 미사용 → allow).
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn set_visible_agents_for_test(&self, agents: &[AgentKey]) {
        let mut by_agent: HashMap<AgentKey, Arc<[WindowLabel]>> = HashMap::new();
        for a in agents {
            by_agent.insert(*a, Arc::from(vec!["main".to_string()]));
        }
        self.table.store(Arc::new(RoutingSnapshot { by_agent }));
    }

    /// ★저빈도★ ViewManager 스냅샷으로 라우팅 테이블을 전부 재계산하고 원자 교체한다.
    ///
    /// 같은 트리 순회 한 번에 (1) `agent → [window]` 역인덱스와 (2) 현재 보이는 agent 집합을 동시에
    /// 산출(piggyback) → 직전 snapshot 의 agent 집합과 diff 해 구독 델타(F-B)를 반환한다.
    ///
    /// ★호출 계약(직렬화 — FIX-1)★: 이 함수는 **ViewManager 락을 보유한 채(layout mutation 과 같은
    ///  critical section 안에서)** 호출돼야 한다. 내부가 `load(prev) → delta 계산 → store(new)` 의
    ///  RMW 라, 락 밖에서 동시 호출되면(Tauri thread pool) 델타가 어긋난다 — 중복 Subscribe·누락
    ///  Unsubscribe·ABA(낡은 store 가 새 store 를 덮음). 락이 RMW 를 직렬화하고 `&mgr` 가 현재 상태임을
    ///  보장한다.
    ///
    /// ★ADR-0006★: 락 안에서 호출해도 위반 아님 — 본문은 **순수 계산 + lock-free `ArcSwap::store`** 뿐이고
    ///  락 보유 중 외부 호출(emit / DaemonClient / network I/O)이 0 이다. 반환된 델타의 **송신만** 락 해제
    ///  후 T6 가 cmd_tx 로 enqueue 한다(락 안에서 송신 금지).
    pub fn rebuild(&self, mgr: &ViewManager) -> SubscriptionDelta {
        // 새 역인덱스 + agent 집합을 한 번의 View/트리 순회로 만든다.
        let mut by_agent: HashMap<AgentKey, Vec<WindowLabel>> = HashMap::new();

        for view in &mgr.views {
            // 이 View 가 렌더되는 창 목록: active_view_id 면 메인 창 + window_bindings 의 매칭 label 들.
            // (한 View 가 메인 탭이면서 동시에 팝업에 바인딩될 수도 있어 양쪽 다 모은다.)
            let mut windows: Vec<&str> = Vec::new();
            if view.id == mgr.active_view_id {
                windows.push(MAIN_WINDOW_LABEL);
            }
            for (label, vid) in &mgr.window_bindings {
                if *vid == view.id {
                    windows.push(label.as_str());
                }
            }
            // 이 View 가 어느 창에도 안 걸리면(active 도 아니고 바인딩도 없음) 출력 소비자 없음 → 스킵.
            if windows.is_empty() {
                continue;
            }
            // 이 View 트리의 모든 배정된 슬롯을 모아 각 agent 를 위 창들에 매핑.
            collect_agents(&view.layout, &windows, &mut by_agent);
        }

        // Vec → Arc<[_]> 확정(이후 핫패스가 clone). agent 집합도 동시에 추린다.
        let mut snapshot = RoutingSnapshot {
            by_agent: HashMap::with_capacity(by_agent.len()),
        };
        let mut new_set: HashSet<AgentKey> = HashSet::with_capacity(by_agent.len());
        for (agent, mut labels) in by_agent {
            // 라벨 정렬 — diff/테스트 결정론(HashMap iteration 순서 비결정). 핫패스 의미엔 무관.
            labels.sort();
            labels.dedup();
            new_set.insert(agent);
            snapshot.by_agent.insert(agent, Arc::from(labels));
        }

        // 직전 snapshot 과 diff(F-B). store 전에 옛 집합을 읽는다.
        let prev = self.table.load();
        let prev_set: HashSet<AgentKey> = prev.by_agent.keys().copied().collect();
        // 축 A: new \ prev = 0→1(새로 보임) → Subscribe / prev \ new = 1→0(안 보이게 됨) → Unsubscribe.
        // `HashSet::difference` 순서는 비결정 — labels.sort() 와 같은 결정론 정책으로 정렬해 델타 순서를
        // 고정한다(테스트 재현성 + 송신 순서 안정, FIX-4).
        let mut to_subscribe: Vec<AgentKey> = new_set.difference(&prev_set).copied().collect();
        let mut to_unsubscribe: Vec<AgentKey> = prev_set.difference(&new_set).copied().collect();
        to_subscribe.sort();
        to_unsubscribe.sort();

        // ★ADR-0046: 축 B(slot=(window,agent) cursor 델타) 제거★ — 미러 버퍼가 사라져 slot 단위 cursor
        //   생명주기가 없다. remount/새 창은 데몬 ring 전량 재replay(뷰 주도 request_replay)로 대체한다.
        let delta = SubscriptionDelta {
            to_subscribe,
            to_unsubscribe,
        };

        // 원자 교체 — 이 시점 이후 핫패스는 새 표를 본다.
        self.table.store(Arc::new(snapshot));
        delta
    }
}

/// 한 View 트리를 순회하며 배정된(agent_id=Some) 슬롯의 agent 를 `windows` 전부에 매핑한다.
///
/// Slot 의 `agent_id: String` 을 `AgentKey`(Uuid)로 파싱 — 실패하면 무시(실 프레임과 매칭 불가).
/// rebuild(저빈도)에서만 호출되므로 파싱 비용은 핫패스와 무관(AgentKey 결정 근거).
fn collect_agents(
    node: &LayoutNode,
    windows: &[&str],
    by_agent: &mut HashMap<AgentKey, Vec<WindowLabel>>,
) {
    match node {
        LayoutNode::Slot { agent_id, .. } => {
            if let Some(s) = agent_id {
                match s.parse::<AgentKey>() {
                    Ok(key) => {
                        let entry = by_agent.entry(key).or_default();
                        for w in windows {
                            entry.push((*w).to_string());
                        }
                    }
                    // 파싱 실패 슬롯은 실 프레임과 매칭 불가라 무시하되, 조용히 버리면 디버깅 단서가
                    // 사라진다(ADR-0038). rebuild 는 저빈도라 debug 로그가 핫패스 부담 0(logging-conventions).
                    Err(_) => {
                        tracing::debug!(agent_id = %s, "rebuild: 슬롯 agent_id 가 UUID 아님 — 라우팅 스킵");
                    }
                }
            }
        }
        LayoutNode::Split { a, b, .. } => {
            collect_agents(a, windows, by_agent);
            collect_agents(b, windows, by_agent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::types::SplitDir;
    use uuid::Uuid;

    // ── 헬퍼 ────────────────────────────────────────────────────────────────

    /// 새 agent uuid + 그 문자열.
    fn agent() -> (Uuid, String) {
        let id = Uuid::new_v4();
        (id, id.to_string())
    }

    /// 단일 슬롯 View 트리를 가진 ViewManager(기본). active = 그 View.
    /// active View 의 첫(유일) 슬롯에 agent 를 배정한다. 슬롯 id 반환.
    fn assign_to_active(mgr: &mut ViewManager, agent_str: &str) -> Uuid {
        let view_id = mgr.active_view_id;
        let slot = {
            let v = mgr.views.iter().find(|v| v.id == view_id).unwrap();
            crate::layout::tree::first_slot_id(&v.layout)
        };
        mgr.assign_agent(view_id, slot, agent_str.to_string())
            .unwrap();
        slot
    }

    fn targets_set(router: &OutputRouter, agent: Uuid) -> Vec<String> {
        let mut v: Vec<String> = router.targets(agent).iter().cloned().collect();
        v.sort();
        v
    }

    // ── 라우팅: targets 가 올바른 window label 반환 ──────────────────────────

    #[test]
    fn agent_in_active_view_routes_to_main() {
        let mut mgr = ViewManager::new();
        let (aid, astr) = agent();
        assign_to_active(&mut mgr, &astr);

        let router = OutputRouter::new();
        router.rebuild(&mgr);

        assert_eq!(targets_set(&router, aid), vec!["main".to_string()]);
    }

    #[test]
    fn agent_absent_returns_empty() {
        let mgr = ViewManager::new(); // 빈 슬롯뿐 — 배정 0.
        let router = OutputRouter::new();
        router.rebuild(&mgr);
        let (aid, _) = agent();
        assert!(
            router.targets(aid).is_empty(),
            "배정 안 된 agent 는 빈 결과"
        );
    }

    #[test]
    fn agent_in_non_active_view_not_routed_without_binding() {
        // View1(active) 빈 슬롯, View2(비활성)에 agent 배정 — 어느 창에도 안 걸림 → 빈 결과.
        let mut mgr = ViewManager::new();
        let v1 = mgr.active_view_id;
        let v2 = mgr.create_view(None); // active 가 v2 로 바뀜
                                        // 다시 v1 을 active 로 → v2 는 비활성, 바인딩도 없음.
        mgr.switch_view(v1).unwrap();
        let (aid, astr) = agent();
        let slot = {
            let v = mgr.views.iter().find(|v| v.id == v2).unwrap();
            crate::layout::tree::first_slot_id(&v.layout)
        };
        mgr.assign_agent(v2, slot, astr).unwrap();

        let router = OutputRouter::new();
        router.rebuild(&mgr);
        assert!(
            router.targets(aid).is_empty(),
            "비활성·미바인딩 View 의 agent 는 라우팅 대상 0"
        );
    }

    #[test]
    fn agent_in_bound_popup_routes_to_popup_label() {
        // View2 를 팝업 창("slot-popup")에 바인딩 → active 아니어도 그 창으로 라우팅.
        let mut mgr = ViewManager::new();
        let v1 = mgr.active_view_id;
        let v2 = mgr.create_view(None);
        mgr.switch_view(v1).unwrap(); // active=v1, v2 는 팝업 전용
        mgr.window_bindings.insert("slot-popup".to_string(), v2);

        let (aid, astr) = agent();
        let slot = {
            let v = mgr.views.iter().find(|v| v.id == v2).unwrap();
            crate::layout::tree::first_slot_id(&v.layout)
        };
        mgr.assign_agent(v2, slot, astr).unwrap();

        let router = OutputRouter::new();
        router.rebuild(&mgr);
        assert_eq!(targets_set(&router, aid), vec!["slot-popup".to_string()]);
    }

    #[test]
    fn same_agent_in_active_and_popup_routes_to_both_windows() {
        // 같은 agent 가 active View(=main) 와 팝업 바인딩 View 양쪽 슬롯에 → 두 창 모두로.
        let mut mgr = ViewManager::new();
        let v1 = mgr.active_view_id;
        let v2 = mgr.create_view(None);
        mgr.switch_view(v1).unwrap();
        mgr.window_bindings.insert("slot-popup".to_string(), v2);

        let (aid, astr) = agent();
        assign_to_active(&mut mgr, &astr); // main 창(v1)
        let slot2 = {
            let v = mgr.views.iter().find(|v| v.id == v2).unwrap();
            crate::layout::tree::first_slot_id(&v.layout)
        };
        mgr.assign_agent(v2, slot2, astr.clone()).unwrap();

        let router = OutputRouter::new();
        router.rebuild(&mgr);
        assert_eq!(
            targets_set(&router, aid),
            vec!["main".to_string(), "slot-popup".to_string()],
            "같은 agent 가 두 창에 보이면 둘 다 라우팅"
        );
    }

    #[test]
    fn split_view_with_two_agents_routes_each() {
        // active View 를 분할해 두 슬롯에 서로 다른 agent → 각자 main 으로(같은 창, 다른 agent).
        let mut mgr = ViewManager::new();
        let view_id = mgr.active_view_id;
        let slot = {
            let v = mgr.views.iter().find(|v| v.id == view_id).unwrap();
            crate::layout::tree::first_slot_id(&v.layout)
        };
        let slot2 = mgr.split_slot(view_id, slot, SplitDir::Horizontal).unwrap();
        let (a1, a1s) = agent();
        let (a2, a2s) = agent();
        mgr.assign_agent(view_id, slot, a1s).unwrap();
        mgr.assign_agent(view_id, slot2, a2s).unwrap();

        let router = OutputRouter::new();
        router.rebuild(&mgr);
        assert_eq!(targets_set(&router, a1), vec!["main".to_string()]);
        assert_eq!(targets_set(&router, a2), vec!["main".to_string()]);
    }

    #[test]
    fn invalid_uuid_slot_string_is_skipped() {
        // Slot agent_id 가 UUID 가 아니면 라우팅 키로 못 들어감(실 프레임과 매칭 불가) → 무시.
        let mut mgr = ViewManager::new();
        assign_to_active(&mut mgr, "not-a-uuid");
        let router = OutputRouter::new();
        let delta = router.rebuild(&mgr);
        // 어떤 키도 안 생김 → 구독 델타도 비어야.
        assert!(delta.is_empty(), "파싱 불가 slot 은 구독 대상도 아님");
        // 라우팅 표도 비어 있음(임의 uuid 조회 → 빈).
        let (probe, _) = agent();
        assert!(router.targets(probe).is_empty());
    }

    // ── 락-free 읽기(구조적): OutputRouter 에 Mutex 없음 ─────────────────────
    //
    // ★주의(FIX-1)★: 아래는 동시 *읽기*(targets/load)와 store 의 안전성만 검증한다. 동시 *rebuild*
    // (writer 둘이 락 없이 load→delta→store)는 **계약 위반**이다 — rebuild 는 ViewManager 락 보유
    // critical section 안에서만 호출돼야 하고(직렬화 보장), 그래서 동시-writer 테스트는 두지 않는다
    // (오용을 테스트하는 꼴이라 무의미). 직렬화된 시퀀스 불변식은 delta_conservation_over_sequence 가 본다.

    #[test]
    fn targets_is_lock_free_concurrent_reads() {
        // 구조적 단언: targets 가 어떤 락도 안 잡으므로 여러 스레드가 동시에 막힘 없이 읽는다.
        // (OutputRouter 필드는 ArcSwap 하나뿐 — Mutex/RwLock 없음. 컴파일 + 동시 읽기로 확인.)
        use std::sync::Arc as StdArc;
        use std::thread;

        let mut mgr = ViewManager::new();
        let (aid, astr) = agent();
        assign_to_active(&mut mgr, &astr);
        let router = StdArc::new(OutputRouter::new());
        router.rebuild(&mgr);

        let mut handles = Vec::new();
        for _ in 0..8 {
            let r = StdArc::clone(&router);
            handles.push(thread::spawn(move || {
                for _ in 0..1000 {
                    // 락이 있으면 여기서 경합/직렬화 — 락 0 이라 자유 동시 읽기.
                    let t = r.targets(aid);
                    assert_eq!(&*t, &["main".to_string()][..]);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn rebuild_concurrent_with_reads_is_safe() {
        // rebuild(store)가 진행되는 동안 다른 스레드가 targets(load)해도 자료경합 없이 항상 일관된
        // 스냅샷(옛것 또는 새것)만 본다 — ArcSwap 의미. 빈 결과 또는 ["main"] 중 하나(찢긴 값 X).
        use std::sync::Arc as StdArc;
        use std::thread;

        let (aid, astr) = agent();
        let router = StdArc::new(OutputRouter::new());

        let reader = {
            let r = StdArc::clone(&router);
            let aid = aid;
            thread::spawn(move || {
                for _ in 0..5000 {
                    let t = r.targets(aid);
                    // 항상 빈 슬라이스이거나 정확히 ["main"] — 부분/찢긴 상태 없음.
                    assert!(t.is_empty() || &*t == &["main".to_string()][..]);
                }
            })
        };

        for _ in 0..200 {
            let mut mgr = ViewManager::new();
            assign_to_active(&mut mgr, &astr);
            router.rebuild(&mgr);
            // 빈 표로도 한 번씩 교체(0→1→0 토글)해 동시성 표면 넓힘.
            router.rebuild(&ViewManager::new());
        }
        reader.join().unwrap();
    }

    // ── F-B 구독 union diff ─────────────────────────────────────────────────

    #[test]
    fn diff_zero_to_one_subscribes() {
        let mut mgr = ViewManager::new();
        let (aid, astr) = agent();
        assign_to_active(&mut mgr, &astr);

        let router = OutputRouter::new();
        let delta = router.rebuild(&mgr);
        assert_eq!(delta.to_subscribe, vec![aid], "0→1 = Subscribe");
        assert!(delta.to_unsubscribe.is_empty());
    }

    #[test]
    fn diff_one_to_zero_unsubscribes() {
        let mut mgr = ViewManager::new();
        let (aid, astr) = agent();
        let slot = assign_to_active(&mut mgr, &astr);

        let router = OutputRouter::new();
        router.rebuild(&mgr); // 1 visible
                              // 슬롯을 닫아(agent 사라짐) 다시 rebuild → 1→0.
        mgr.close_slot(mgr.active_view_id, slot).unwrap();
        let delta = router.rebuild(&mgr);
        assert!(delta.to_subscribe.is_empty());
        assert_eq!(delta.to_unsubscribe, vec![aid], "1→0 = Unsubscribe");
    }

    #[test]
    fn diff_no_change_is_empty() {
        let mut mgr = ViewManager::new();
        let (_aid, astr) = agent();
        assign_to_active(&mut mgr, &astr);

        let router = OutputRouter::new();
        router.rebuild(&mgr);
        // 변화 없이 다시 rebuild → 델타 없음(non-vacuity 의 짝: no-op 입력엔 델타 0).
        let delta = router.rebuild(&mgr);
        assert!(
            delta.is_empty(),
            "레이아웃 불변이면 구독 델타 0(중복 Subscribe 방지)"
        );
    }

    #[test]
    fn diff_agent_in_two_windows_one_closes_stays_subscribed() {
        // §6 리스크3: 같은 agent 가 main+팝업 두 창에 → 한 창(팝업) 닫혀도 다른 창에 남으면 구독 유지.
        let mut mgr = ViewManager::new();
        let v1 = mgr.active_view_id;
        let v2 = mgr.create_view(None);
        mgr.switch_view(v1).unwrap();
        mgr.window_bindings.insert("slot-popup".to_string(), v2);

        let (aid, astr) = agent();
        assign_to_active(&mut mgr, &astr); // main
        let slot2 = {
            let v = mgr.views.iter().find(|v| v.id == v2).unwrap();
            crate::layout::tree::first_slot_id(&v.layout)
        };
        mgr.assign_agent(v2, slot2, astr.clone()).unwrap();

        let router = OutputRouter::new();
        let d1 = router.rebuild(&mgr);
        assert_eq!(d1.to_subscribe, vec![aid], "처음 0→1 Subscribe 한 번");

        // 팝업 창 닫힘 = 그 바인딩 제거(close_view 가 window_bindings retain 으로 정리하는 동작 모사).
        mgr.window_bindings.remove("slot-popup");
        let d2 = router.rebuild(&mgr);
        assert!(
            d2.is_empty(),
            "한 창 닫혀도 main 에 남아 1→1 — Unsubscribe 금지(아직 보임)"
        );
        // 여전히 main 으로 라우팅.
        assert_eq!(targets_set(&router, aid), vec!["main".to_string()]);
    }

    #[test]
    fn diff_switch_view_changes_visible_set() {
        // View1(agentA), View2(agentB). active=v1 → A 구독. switch_view(v2) → A 빠지고 B 들어옴.
        let mut mgr = ViewManager::new();
        let v1 = mgr.active_view_id;
        let (a1, a1s) = agent();
        assign_to_active(&mut mgr, &a1s); // v1 의 슬롯에 A

        let v2 = mgr.create_view(None); // active=v2 (create 가 전환)
        let (b1, b1s) = agent();
        let slot2 = {
            let v = mgr.views.iter().find(|v| v.id == v2).unwrap();
            crate::layout::tree::first_slot_id(&v.layout)
        };
        mgr.assign_agent(v2, slot2, b1s).unwrap();

        let router = OutputRouter::new();
        // active=v2 상태로 첫 rebuild → B 보임(A 안 보임).
        mgr.switch_view(v2).unwrap();
        let d1 = router.rebuild(&mgr);
        assert_eq!(d1.to_subscribe, vec![b1]);
        assert!(d1.to_unsubscribe.is_empty());

        // active 를 v1 으로 → A 들어오고 B 빠짐.
        mgr.switch_view(v1).unwrap();
        let d2 = router.rebuild(&mgr);
        assert_eq!(d2.to_subscribe, vec![a1], "switch 로 A 가 보이기 시작");
        assert_eq!(d2.to_unsubscribe, vec![b1], "B 는 안 보이게 됨");
        assert_eq!(targets_set(&router, a1), vec!["main".to_string()]);
        assert!(router.targets(b1).is_empty());
    }

    #[test]
    fn diff_assign_then_clear_agent() {
        // assign 으로 0→1, close_slot(=clear)로 1→0 — assign/close 경로 델타 검증.
        let mut mgr = ViewManager::new();
        let router = OutputRouter::new();
        // 초기 빈 → 델타 0.
        let d0 = router.rebuild(&mgr);
        assert!(d0.is_empty());

        let (aid, astr) = agent();
        let slot = assign_to_active(&mut mgr, &astr);
        let d1 = router.rebuild(&mgr);
        assert_eq!(d1.to_subscribe, vec![aid]);

        mgr.close_slot(mgr.active_view_id, slot).unwrap();
        let d2 = router.rebuild(&mgr);
        assert_eq!(d2.to_unsubscribe, vec![aid]);
    }

    /// ★non-vacuity 가드★: diff 로직이 no-op(항상 빈 델타 반환)이면 이 테스트가 실패한다.
    /// 두 개의 서로 다른 agent 가 들고나는 시퀀스에서 정확한 차집합을 단언 — 빈 델타로는 통과 불가.
    #[test]
    fn diff_non_vacuous_distinct_deltas() {
        let mut mgr = ViewManager::new();
        let router = OutputRouter::new();

        let (a, as_) = agent();
        assign_to_active(&mut mgr, &as_);
        let d1 = router.rebuild(&mgr);
        // 만약 diff 가 no-op 이면 d1.to_subscribe 가 비어 이 단언에서 실패.
        assert_eq!(d1.to_subscribe, vec![a]);
        assert!(d1.to_unsubscribe.is_empty());

        // A 를 B 로 교체(같은 슬롯에 재배정) → A unsubscribe + B subscribe 동시 발생.
        let view_id = mgr.active_view_id;
        let slot = {
            let v = mgr.views.iter().find(|v| v.id == view_id).unwrap();
            crate::layout::tree::first_slot_id(&v.layout)
        };
        let (b, bs) = agent();
        mgr.assign_agent(view_id, slot, bs).unwrap();
        let d2 = router.rebuild(&mgr);
        assert_eq!(d2.to_subscribe, vec![b], "B 새로 보임");
        assert_eq!(d2.to_unsubscribe, vec![a], "A 교체돼 사라짐");
        // no-op diff 였다면 위 두 단언 모두 실패(빈 vec ≠ [b]/[a]).
    }

    // ★ADR-0046: 축 B(slot=(window,agent) cursor 델타) 테스트 삭제★ — 미러 버퍼와 함께 slot 단위 cursor
    //   생명주기가 사라졌다. remount/새 창은 데몬 ring 전량 재replay(뷰 주도)로 대체(rust 검증은 single-flight
    //   상태기계 = daemon_client::replay_flight 단위테스트). 아래 axis A(agent 단위) diff·보존 불변식은 존속.

    /// ★델타 보존 불변식 가드★: **직렬화된** rebuild 시퀀스(= ViewManager 락 보유 호출, FIX-1)
    /// 전체에서, `(모든 to_subscribe 합집합) \ (모든 to_unsubscribe 합집합)` 가 최종 테이블의 agent 집합과
    /// 정확히 일치해야 한다 — 즉 최종에 보이는 agent 는 모두 net-구독됐고(빠짐없이), net-해제된 것은 하나도
    /// 없다. 누수(구독했는데 테이블엔 없음)·유실(테이블엔 있는데 구독 안 됨)을 한 번에 잡는 회귀 그물.
    #[test]
    fn delta_conservation_over_sequence() {
        let mut mgr = ViewManager::new();
        let router = OutputRouter::new();

        // 여러 agent 를 들이고 내는 시퀀스를 만든다(split 으로 슬롯 늘리고, close 로 줄임).
        let mut subscribed: HashSet<Uuid> = HashSet::new();
        let mut unsubscribed: HashSet<Uuid> = HashSet::new();

        // 누적 헬퍼: 한 번의 rebuild 델타를 합집합에 접는다.
        let mut fold = |router: &OutputRouter,
                        mgr: &ViewManager,
                        sub: &mut HashSet<Uuid>,
                        unsub: &mut HashSet<Uuid>| {
            let d = router.rebuild(mgr);
            for a in d.to_subscribe {
                sub.insert(a);
            }
            for a in d.to_unsubscribe {
                unsub.insert(a);
            }
        };

        // step1: active View 첫 슬롯에 A, 분할해 둘째 슬롯에 B (둘 다 0→1).
        let view_id = mgr.active_view_id;
        let slot_a = {
            let v = mgr.views.iter().find(|v| v.id == view_id).unwrap();
            crate::layout::tree::first_slot_id(&v.layout)
        };
        let slot_b = mgr
            .split_slot(view_id, slot_a, SplitDir::Horizontal)
            .unwrap();
        let (a, as_) = agent();
        let (b, bs) = agent();
        mgr.assign_agent(view_id, slot_a, as_).unwrap();
        mgr.assign_agent(view_id, slot_b, bs).unwrap();
        fold(&router, &mgr, &mut subscribed, &mut unsubscribed);

        // step2: B 슬롯을 닫는다(B 1→0). A 는 그대로.
        mgr.close_slot(view_id, slot_b).unwrap();
        fold(&router, &mgr, &mut subscribed, &mut unsubscribed);

        // step3: A 가 있던 슬롯에 C 재배정(A 1→0, C 0→1 동시).
        let slot_a2 = {
            let v = mgr.views.iter().find(|v| v.id == view_id).unwrap();
            crate::layout::tree::first_slot_id(&v.layout)
        };
        let (c, cs) = agent();
        mgr.assign_agent(view_id, slot_a2, cs).unwrap();
        fold(&router, &mgr, &mut subscribed, &mut unsubscribed);

        // 보존 불변식: net-구독(= sub \ unsub) == 최종 테이블 agent 집합.
        let net: HashSet<Uuid> = subscribed.difference(&unsubscribed).copied().collect();
        let final_set: HashSet<Uuid> = router.table.load().by_agent.keys().copied().collect();
        assert_eq!(
            net, final_set,
            "직렬화 rebuild 시퀀스의 net-구독 집합 = 최종 테이블 agent 집합(누수·유실 0)"
        );
        // 구체값 교차검증: 최종엔 C 만 보임. A·B 는 net-해제, C 는 net-구독.
        assert_eq!(final_set, HashSet::from([c]), "최종 테이블엔 C 만");
        assert!(net.contains(&c) && !net.contains(&a) && !net.contains(&b));
    }

    // ★ADR-0046: resync_filter_*(slots_for_window_agent) 테스트 삭제★ — 그 메서드는 옛 slot 단위 mount
    //   replay 필터용이었다. resync_output 은 이제 agent 단위 request_replay(뷰 주도 전량 재replay)로 흡수돼
    //   window·slot 필터가 필요 없다(라우팅은 targets() 가, 마커 순서는 single-flight 가 담당).
}
