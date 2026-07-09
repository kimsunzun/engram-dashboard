//! 순수 split-트리 연산 — Tauri 의존 0(ADR-0012 격리: 단독 headless 테스트 가능).
//!
//! ViewManager(상위)는 락·emit·AppState 를 다루고, 실제 트리 변형은 전부 여기로 위임한다.
//! 이 모듈은 `LayoutNode` 만 알고 Tauri/AppState/락 을 모른다 → `#[cfg(test)]` 로 단독 회귀 단언.
//!
//! ★불변식★
//! - split: 대상 Slot 을 Split{a=원래 슬롯, b=새 빈 슬롯, ratio=0.5}로 치환. 새 슬롯 id 반환.
//! - close: 닫는 Slot 의 형제를 부모 자리로 승격(2-자식 Split 붕괴). root 슬롯이면 빈 슬롯으로 리셋.
//! - assign: 대상 Slot 의 content(SlotContent) 만 교체(트리 구조 불변, ADR-0060).
//! - ratio: 0.0~1.0 클램프(split 기본 0.5).

use uuid::Uuid;

use super::types::{LayoutNode, SlotContent};

/// ratio 를 [0.0, 1.0] 으로 클램프. split 기본값은 0.5.
pub fn clamp_ratio(r: f32) -> f32 {
    r.clamp(0.0, 1.0)
}

/// 트리에서 slot_id 를 가진 Slot 을 찾아 그 콘텐츠(점유자) 를 반환.
/// 반환: Some(SlotContent::Agent{..}) = 배정됨 · Some(SlotContent::Empty) = 빈 슬롯 · None = 그 slot_id 없음.
pub fn find_slot(node: &LayoutNode, slot_id: Uuid) -> Option<&SlotContent> {
    match node {
        LayoutNode::Slot { id, content } => {
            if *id == slot_id {
                Some(content)
            } else {
                None
            }
        }
        LayoutNode::Split { a, b, .. } => find_slot(a, slot_id).or_else(|| find_slot(b, slot_id)),
    }
}

/// slot_id 가 트리에 존재하는지.
pub fn contains_slot(node: &LayoutNode, slot_id: Uuid) -> bool {
    find_slot(node, slot_id).is_some()
}

/// 트리를 전위 순회하며 첫 번째(가장 왼쪽 = a 우선) Slot 의 id 를 반환. 트리는 항상 ≥1 슬롯이라 무한.
pub fn first_slot_id(node: &LayoutNode) -> Uuid {
    match node {
        LayoutNode::Slot { id, .. } => *id,
        LayoutNode::Split { a, .. } => first_slot_id(a),
    }
}

/// 트리를 전위 순회(a 우선 — first_slot_id 와 동일 순서)하며 **첫 번째 빈 슬롯(SlotContent::Empty)의 id** 를
/// 반환한다. 빈 슬롯이 하나도 없으면 None. spawn_into 의 slot=None 정책(첫 빈 슬롯 배치, USER DECISION 2b)의
/// 코어 — first_slot_id 가 점유 여부를 안 보는 것과 달리 이건 빈 슬롯만 고른다. // ADR-0059
pub fn first_empty_slot_id(node: &LayoutNode) -> Option<Uuid> {
    match node {
        LayoutNode::Slot { id, content } => {
            // ADR-0060: 빈 슬롯 = SlotContent::Empty(옛 agent_id.is_none()).
            if content.is_empty() {
                Some(*id)
            } else {
                None
            }
        }
        // a 먼저(전위·좌측 우선), 없으면 b.
        LayoutNode::Split { a, b, .. } => first_empty_slot_id(a).or_else(|| first_empty_slot_id(b)),
    }
}

/// slot_id 슬롯을 Split 으로 분할한다.
///
/// 대상 Slot 을 `Split{dir, ratio:0.5, a=원래 슬롯, b=새 빈 슬롯}` 으로 치환하고 **새 빈 슬롯의
/// id 를 반환**(호출자가 focus 이동·검증에 사용). slot_id 가 없으면 트리 불변 + None 반환(no-op).
///
/// 중첩 분할도 지원: 한 번 split 된 트리의 어느 말단 Slot 이든 다시 split 가능(재귀가 깊이 무관).
pub fn split_in_tree(
    node: &mut LayoutNode,
    slot_id: Uuid,
    dir: super::types::SplitDir,
) -> Option<Uuid> {
    match node {
        LayoutNode::Slot { id, .. } => {
            if *id == slot_id {
                // 원래 슬롯(node)을 통째로 a 로 옮기고, b 에 새 빈 슬롯을 둔다.
                // std::mem::replace 로 node 의 소유권을 빼내 a 박스에 넣는다(클론 회피).
                let new_slot = LayoutNode::new_empty_slot();
                let new_slot_id = match &new_slot {
                    LayoutNode::Slot { id, .. } => *id,
                    _ => unreachable!("new_empty_slot 은 항상 Slot"),
                };
                let original = std::mem::replace(node, LayoutNode::new_empty_slot());
                *node = LayoutNode::Split {
                    dir,
                    ratio: 0.5,
                    a: Box::new(original),
                    b: Box::new(new_slot),
                };
                Some(new_slot_id)
            } else {
                None
            }
        }
        LayoutNode::Split { a, b, .. } => {
            // a 먼저 시도, 못 찾으면 b. 한쪽에서 찾으면 다른 쪽은 안 봄(slot id 전역 고유).
            if let Some(found) = split_in_tree(a, slot_id, dir) {
                Some(found)
            } else {
                split_in_tree(b, slot_id, dir)
            }
        }
    }
}

/// slot_id 슬롯을 닫는다.
///
/// - 닫는 슬롯이 어떤 Split 의 **직접 자식**이면 → 그 Split 을 **형제(다른 자식)로 치환**(형제 승격).
/// - 닫는 슬롯이 **root 자체**(트리에 슬롯 하나뿐)면 → 새 빈 슬롯으로 리셋(View 는 빈 상태 유지).
/// - slot_id 가 없으면 트리 불변(no-op, false 반환).
///
/// 반환: 실제로 닫혔으면 true(no-op 이면 false). 호출자는 false 면 invalid id 로 Err.
pub fn close_in_tree(node: &mut LayoutNode, slot_id: Uuid) -> bool {
    // root 가 바로 그 슬롯이면 빈 슬롯으로 리셋(트리에 슬롯 하나뿐인 경우 포함).
    if let LayoutNode::Slot { id, .. } = node {
        if *id == slot_id {
            *node = LayoutNode::new_empty_slot();
            return true;
        }
        return false;
    }

    // node 는 Split. 직접 자식 중 닫을 슬롯이 있으면 형제를 승격(Split 붕괴).
    if let LayoutNode::Split { a, b, .. } = node {
        let a_is_target = matches!(a.as_ref(), LayoutNode::Slot { id, .. } if *id == slot_id);
        let b_is_target = matches!(b.as_ref(), LayoutNode::Slot { id, .. } if *id == slot_id);

        if a_is_target {
            // 형제 b 를 부모(node) 자리로 승격. mem::replace 로 b 의 소유권을 빼낸다.
            let sibling = std::mem::replace(b.as_mut(), LayoutNode::new_empty_slot());
            *node = sibling;
            return true;
        }
        if b_is_target {
            let sibling = std::mem::replace(a.as_mut(), LayoutNode::new_empty_slot());
            *node = sibling;
            return true;
        }

        // 직접 자식이 아니면 더 깊이 재귀(자식이 Split 인 경우).
        if close_in_tree(a, slot_id) {
            return true;
        }
        return close_in_tree(b, slot_id);
    }

    false
}

/// slot_id 슬롯에 agent_id(참조 문자열) 를 배정. agent=None 이면 해제(빈 슬롯 = SlotContent::Empty).
/// 데몬에 실재 검증 안 함(ADR-0035/0006 — 락 보유 중 외부 호출 0). slot_id 없으면 no-op(false).
/// ★덮어쓰기 시맨틱 유지(ADR-0058)★: 점유 슬롯이어도 무조건 교체(점유 방어는 resolve_spawn_slot 층).
/// ADR-0060: 입력 Option<String> 을 SlotContent(Some→Agent / None→Empty)로 매핑해 콘텐츠를 통째 치환한다.
pub fn assign_in_tree(node: &mut LayoutNode, slot_id: Uuid, agent: Option<String>) -> bool {
    match node {
        LayoutNode::Slot { id, content } => {
            if *id == slot_id {
                *content = match agent {
                    Some(agent_id) => SlotContent::Agent { agent_id },
                    None => SlotContent::Empty,
                };
                true
            } else {
                false
            }
        }
        LayoutNode::Split { a, b, .. } => {
            // a 를 먼저 만지고, 거기서 처리됐으면 b 는 안 봄(전역 고유). agent 소유권 분기 처리.
            if contains_slot(a, slot_id) {
                assign_in_tree(a, slot_id, agent)
            } else {
                assign_in_tree(b, slot_id, agent)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{SlotContent, SplitDir};
    use super::*;

    /// 배정된 슬롯 노드(테스트 헬퍼) — id + Agent{agent_id}.
    fn agent_slot(id: Uuid, agent: &str) -> LayoutNode {
        LayoutNode::Slot {
            id,
            content: SlotContent::Agent {
                agent_id: agent.to_string(),
            },
        }
    }

    /// 단일 빈 슬롯 트리 + 그 슬롯 id 반환.
    fn single_slot() -> (LayoutNode, Uuid) {
        let node = LayoutNode::new_empty_slot();
        let id = first_slot_id(&node);
        (node, id)
    }

    // ── find / contains ────────────────────────────────────────────────────

    #[test]
    fn find_slot_returns_none_for_missing() {
        let (node, _id) = single_slot();
        assert!(find_slot(&node, Uuid::new_v4()).is_none());
    }

    #[test]
    fn find_slot_returns_agent_ref() {
        let id = Uuid::new_v4();
        let node = agent_slot(id, "agent-x");
        let found = find_slot(&node, id).expect("슬롯 찾아야 함");
        assert_eq!(found.agent_id(), Some("agent-x"));
    }

    // ── SlotContent 편의 메서드 + serde round-trip(golden, ADR-0060) ─────────────

    #[test]
    fn slot_content_is_empty_and_agent_id() {
        assert!(SlotContent::Empty.is_empty());
        assert_eq!(SlotContent::Empty.agent_id(), None);
        let a = SlotContent::Agent {
            agent_id: "a-1".into(),
        };
        assert!(!a.is_empty());
        assert_eq!(a.agent_id(), Some("a-1"));
    }

    #[test]
    fn slot_content_serde_round_trip_golden() {
        // 내부태깅 JSON golden — 프론트 discriminated union 과 정합(ADR-0060).
        let empty = SlotContent::Empty;
        let empty_json = serde_json::to_string(&empty).unwrap();
        assert_eq!(empty_json, r#"{"type":"empty"}"#);
        assert_eq!(
            serde_json::from_str::<SlotContent>(&empty_json).unwrap(),
            empty
        );

        let agent = SlotContent::Agent {
            agent_id: "abc".into(),
        };
        let agent_json = serde_json::to_string(&agent).unwrap();
        assert_eq!(agent_json, r#"{"type":"agent","agent_id":"abc"}"#);
        assert_eq!(
            serde_json::from_str::<SlotContent>(&agent_json).unwrap(),
            agent
        );
    }

    // ── split ────────────────────────────────────────────────────────────────

    #[test]
    fn split_replaces_slot_with_split_and_returns_new_id() {
        let (mut node, id) = single_slot();
        let new_id = split_in_tree(&mut node, id, SplitDir::Horizontal).expect("split 성공");
        // 새 슬롯 id 는 원래와 다름.
        assert_ne!(new_id, id);
        // 이제 root 는 Split, 두 자식 모두 슬롯으로 존재.
        match &node {
            LayoutNode::Split { dir, ratio, a, b } => {
                assert_eq!(*dir, SplitDir::Horizontal);
                assert_eq!(*ratio, 0.5, "split 기본 ratio 0.5");
                assert!(matches!(a.as_ref(), LayoutNode::Slot { id: aid, .. } if *aid == id));
                assert!(matches!(b.as_ref(), LayoutNode::Slot { id: bid, .. } if *bid == new_id));
            }
            _ => panic!("split 후 root 는 Split 이어야 함"),
        }
        assert!(contains_slot(&node, id));
        assert!(contains_slot(&node, new_id));
    }

    #[test]
    fn split_nested_targets_deep_leaf() {
        // 한 번 split → 새 슬롯을 다시 split(중첩 분할).
        let (mut node, id) = single_slot();
        let mid = split_in_tree(&mut node, id, SplitDir::Horizontal).unwrap();
        let deep = split_in_tree(&mut node, mid, SplitDir::Vertical).expect("중첩 split 성공");
        // 슬롯 3개 모두 존재.
        assert!(contains_slot(&node, id));
        assert!(contains_slot(&node, mid));
        assert!(contains_slot(&node, deep));
        // 깊은 split 의 방향이 Vertical 인지(중첩이 a/b 어느쪽이든 트리에 박혀야).
        assert_eq!(count_splits(&node), 2);
    }

    #[test]
    fn split_missing_slot_is_noop() {
        let (mut node, _id) = single_slot();
        let before = node.clone();
        assert!(split_in_tree(&mut node, Uuid::new_v4(), SplitDir::Horizontal).is_none());
        assert_eq!(node, before, "없는 slot split 은 트리 불변");
    }

    fn count_splits(node: &LayoutNode) -> usize {
        match node {
            LayoutNode::Slot { .. } => 0,
            LayoutNode::Split { a, b, .. } => 1 + count_splits(a) + count_splits(b),
        }
    }

    // ── close: sibling promote ─────────────────────────────────────────────

    #[test]
    fn close_promotes_sibling() {
        let (mut node, id) = single_slot();
        let new_id = split_in_tree(&mut node, id, SplitDir::Horizontal).unwrap();
        // b(new_id) 를 닫으면 a(id) 가 root 로 승격.
        assert!(close_in_tree(&mut node, new_id));
        match &node {
            LayoutNode::Slot { id: rid, .. } => assert_eq!(*rid, id, "형제 a 가 root 로 승격"),
            _ => panic!("close 후 단일 슬롯이어야 함"),
        }
        assert!(!contains_slot(&node, new_id));
    }

    #[test]
    fn close_promotes_sibling_when_closing_a() {
        let (mut node, id) = single_slot();
        let new_id = split_in_tree(&mut node, id, SplitDir::Vertical).unwrap();
        // a(id) 를 닫으면 b(new_id) 가 승격.
        assert!(close_in_tree(&mut node, id));
        match &node {
            LayoutNode::Slot { id: rid, .. } => assert_eq!(*rid, new_id),
            _ => panic!("close 후 단일 슬롯이어야 함"),
        }
    }

    #[test]
    fn close_nested_promotes_subtree() {
        // 트리: Split{ a=Slot(id), b=Split{ x, y } }. b 안의 x 를 닫으면 y 가 b 자리로 승격.
        let (mut node, id) = single_slot();
        let b_id = split_in_tree(&mut node, id, SplitDir::Horizontal).unwrap();
        let y_id = split_in_tree(&mut node, b_id, SplitDir::Vertical).unwrap();
        // 이제 트리: Split{ Slot(id), Split{ Slot(b_id), Slot(y_id) } }
        assert!(close_in_tree(&mut node, b_id), "중첩 슬롯 close");
        // y 가 b 의 Split 자리로 승격 → 트리: Split{ Slot(id), Slot(y_id) }
        assert!(contains_slot(&node, id));
        assert!(contains_slot(&node, y_id));
        assert!(!contains_slot(&node, b_id));
        assert_eq!(count_splits(&node), 1);
    }

    // ── close: root slot → reset to empty ───────────────────────────────────

    #[test]
    fn close_root_slot_resets_to_empty() {
        let id = Uuid::new_v4();
        let mut node = agent_slot(id, "agent-x");
        assert!(close_in_tree(&mut node, id), "root 슬롯 close 는 true");
        // 빈 슬롯으로 리셋(새 id, content Empty) — View 는 빈 상태 유지.
        match &node {
            LayoutNode::Slot { id: rid, content } => {
                assert_ne!(*rid, id, "새 빈 슬롯 id");
                assert!(content.is_empty(), "빈 슬롯");
            }
            _ => panic!("root 슬롯 close 후에도 단일 슬롯"),
        }
    }

    #[test]
    fn close_missing_slot_is_noop() {
        let (mut node, _id) = single_slot();
        let before = node.clone();
        assert!(
            !close_in_tree(&mut node, Uuid::new_v4()),
            "없는 slot close 는 false"
        );
        assert_eq!(node, before, "트리 불변");
    }

    // ── assign ───────────────────────────────────────────────────────────────

    #[test]
    fn assign_sets_agent_ref() {
        let (mut node, id) = single_slot();
        assert!(assign_in_tree(&mut node, id, Some("agent-7".into())));
        assert_eq!(find_slot(&node, id).unwrap().agent_id(), Some("agent-7"));
    }

    #[test]
    fn assign_in_split_targets_correct_slot() {
        let (mut node, id) = single_slot();
        let new_id = split_in_tree(&mut node, id, SplitDir::Horizontal).unwrap();
        assert!(assign_in_tree(&mut node, new_id, Some("agent-b".into())));
        // 대상만 바뀌고 형제는 빈 채.
        assert_eq!(
            find_slot(&node, new_id).unwrap().agent_id(),
            Some("agent-b")
        );
        assert!(find_slot(&node, id).unwrap().is_empty());
    }

    #[test]
    fn assign_can_clear_agent() {
        let id = Uuid::new_v4();
        let mut node = agent_slot(id, "agent-x");
        // ADR-0060: agent=None 은 SlotContent::Empty 로 해제.
        assert!(assign_in_tree(&mut node, id, None));
        assert!(find_slot(&node, id).unwrap().is_empty());
    }

    #[test]
    fn assign_overwrites_occupied_slot() {
        // ★ADR-0058 덮어쓰기 시맨틱 유지★: 점유 슬롯에 재배정하면 무조건 교체(점유 방어는 상위 층).
        let id = Uuid::new_v4();
        let mut node = agent_slot(id, "old");
        assert!(assign_in_tree(&mut node, id, Some("new".into())));
        assert_eq!(find_slot(&node, id).unwrap().agent_id(), Some("new"));
    }

    #[test]
    fn assign_missing_slot_is_noop() {
        let (mut node, _id) = single_slot();
        let before = node.clone();
        assert!(!assign_in_tree(&mut node, Uuid::new_v4(), Some("x".into())));
        assert_eq!(node, before);
    }

    // ── ratio clamp ────────────────────────────────────────────────────────

    #[test]
    fn ratio_clamps_out_of_range() {
        assert_eq!(clamp_ratio(-0.5), 0.0);
        assert_eq!(clamp_ratio(1.5), 1.0);
        assert_eq!(clamp_ratio(0.3), 0.3);
        assert_eq!(clamp_ratio(0.0), 0.0);
        assert_eq!(clamp_ratio(1.0), 1.0);
    }

    // ── first_slot_id (focus fallback 의 핵심) ────────────────────────────────

    #[test]
    fn first_slot_id_is_leftmost() {
        let (mut node, id) = single_slot();
        let _new_id = split_in_tree(&mut node, id, SplitDir::Horizontal).unwrap();
        // a 가 원래 슬롯(id) → 전위 순회 첫 슬롯은 id.
        assert_eq!(first_slot_id(&node), id);
    }

    // ── first_empty_slot_id (spawn_into slot=None 정책 — USER DECISION 2b) ────────

    #[test]
    fn first_empty_slot_id_single_empty() {
        let (node, id) = single_slot();
        assert_eq!(
            first_empty_slot_id(&node),
            Some(id),
            "빈 단일 슬롯은 자기 자신"
        );
    }

    #[test]
    fn first_empty_slot_id_single_occupied_is_none() {
        let id = Uuid::new_v4();
        let node = agent_slot(id, "a");
        assert_eq!(
            first_empty_slot_id(&node),
            None,
            "점유 단일 슬롯은 빈 슬롯 0"
        );
    }

    #[test]
    fn first_empty_slot_id_skips_occupied_leftmost() {
        // 좌측(a=id)을 점유, 우측(b=new_id)은 빈 채 → 좌측을 건너뛰고 우측을 고른다(2b).
        let (mut node, id) = single_slot();
        let new_id = split_in_tree(&mut node, id, SplitDir::Horizontal).unwrap();
        assert!(assign_in_tree(&mut node, id, Some("occupied".into())));
        assert_eq!(
            first_empty_slot_id(&node),
            Some(new_id),
            "점유된 좌측 건너뛰고 첫 빈 슬롯(우측)"
        );
    }

    #[test]
    fn first_empty_slot_id_all_occupied_is_none() {
        let (mut node, id) = single_slot();
        let new_id = split_in_tree(&mut node, id, SplitDir::Horizontal).unwrap();
        assert!(assign_in_tree(&mut node, id, Some("x".into())));
        assert!(assign_in_tree(&mut node, new_id, Some("y".into())));
        assert_eq!(first_empty_slot_id(&node), None, "전부 점유 → None");
    }
}
