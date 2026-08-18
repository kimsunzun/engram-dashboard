//! 순수 split-트리 연산 — Tauri 의존 0(ADR-0012 격리: 단독 headless 테스트 가능).
//!
//! ViewManager(상위)는 실제 트리 변형을 전부 여기로 위임한다.
//! 이 모듈은 `LayoutNode` 만 알고 Tauri/AppState/락 을 모른다 → `#[cfg(test)]` 로 단독 회귀 단언.
//!
//! ★불변식★
//! - assign: 대상 Slot 의 content(SlotContent) 만 교체(트리 구조 불변, ADR-0060).

use uuid::Uuid;

use super::types::{LayoutNode, SlotContent};

pub fn clamp_ratio(r: f32) -> f32 {
    r.clamp(0.0, 1.0)
}

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

pub fn contains_slot(node: &LayoutNode, slot_id: Uuid) -> bool {
    find_slot(node, slot_id).is_some()
}

// 트리는 항상 ≥1 슬롯이라 무한.
pub fn first_slot_id(node: &LayoutNode) -> Uuid {
    match node {
        LayoutNode::Slot { id, .. } => *id,
        LayoutNode::Split { a, .. } => first_slot_id(a),
    }
}

// spawn_into 의 slot=None 정책(첫 빈 슬롯 배치, USER DECISION 2b)의 코어 — first_slot_id 가 점유 여부를
// 안 보는 것과 달리 이건 빈 슬롯만 고른다. // ADR-0059
pub fn first_empty_slot_id(node: &LayoutNode) -> Option<Uuid> {
    match node {
        LayoutNode::Slot { id, content } => {
            if content.is_empty() {
                Some(*id)
            } else {
                None
            }
        }
        LayoutNode::Split { a, b, .. } => first_empty_slot_id(a).or_else(|| first_empty_slot_id(b)),
    }
}

// 대상 Slot 을 `Split{dir, ratio:0.5, a=원래 슬롯, b=새 빈 슬롯}` 으로 치환하고 **새 빈 슬롯의
// id 를 반환**(호출자가 focus 이동·검증에 사용). slot_id 가 없으면 트리 불변 + None 반환(no-op).
pub fn split_in_tree(
    node: &mut LayoutNode,
    slot_id: Uuid,
    dir: super::types::SplitDir,
) -> Option<Uuid> {
    match node {
        LayoutNode::Slot { id, .. } => {
            if *id == slot_id {
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

// - 닫는 슬롯이 어떤 Split 의 **직접 자식**이면 → 그 Split 을 **형제(다른 자식)로 치환**(형제 승격).
// - 닫는 슬롯이 **root 자체**(트리에 슬롯 하나뿐)면 → 새 빈 슬롯으로 리셋(View 는 빈 상태 유지).
// - slot_id 가 없으면 트리 불변(no-op, false 반환).
//
// 반환: 실제로 닫혔으면 true(no-op 이면 false). 호출자는 false 면 invalid id 로 Err.
pub fn close_in_tree(node: &mut LayoutNode, slot_id: Uuid) -> bool {
    if let LayoutNode::Slot { id, .. } = node {
        if *id == slot_id {
            *node = LayoutNode::new_empty_slot();
            return true;
        }
        return false;
    }

    if let LayoutNode::Split { a, b, .. } = node {
        let a_is_target = matches!(a.as_ref(), LayoutNode::Slot { id, .. } if *id == slot_id);
        let b_is_target = matches!(b.as_ref(), LayoutNode::Slot { id, .. } if *id == slot_id);

        if a_is_target {
            let sibling = std::mem::replace(b.as_mut(), LayoutNode::new_empty_slot());
            *node = sibling;
            return true;
        }
        if b_is_target {
            let sibling = std::mem::replace(a.as_mut(), LayoutNode::new_empty_slot());
            *node = sibling;
            return true;
        }

        if close_in_tree(a, slot_id) {
            return true;
        }
        return close_in_tree(b, slot_id);
    }

    false
}

// slot_id 없으면 no-op(false).
// ★덮어쓰기 시맨틱 유지(ADR-0058)★: 점유 슬롯이어도 무조건 교체(점유 방어는 resolve_spawn_slot 층).
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

// assign_in_tree 의 미러이나 Option<String> agent 래핑 없이 SlotContent 를 직접 받는다 —
// 비-에이전트 콘텐츠(AgentList/PresetPalette)를 슬롯에 배치하는 배치 경로(ADR-0063 set_slot_content).
// ★덮어쓰기 시맨틱★: 점유 슬롯이어도 무조건 교체(assign_in_tree 와 동형).
pub fn set_in_tree(node: &mut LayoutNode, slot_id: Uuid, content: SlotContent) -> bool {
    match node {
        LayoutNode::Slot { id, content: c } => {
            if *id == slot_id {
                *c = content;
                true
            } else {
                false
            }
        }
        LayoutNode::Split { a, b, .. } => {
            // a 먼저(전역 고유 — 한쪽에서 처리됐으면 다른 쪽 안 봄). content 소유권 분기 이동.
            if contains_slot(a, slot_id) {
                set_in_tree(a, slot_id, content)
            } else {
                set_in_tree(b, slot_id, content)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{SlotContent, SplitDir};
    use super::*;

    fn agent_slot(id: Uuid, agent: &str) -> LayoutNode {
        LayoutNode::Slot {
            id,
            content: SlotContent::Agent {
                agent_id: agent.to_string(),
            },
        }
    }

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
        let new_id = split_in_tree(&mut node, id, SplitDir::LeftRight).expect("split 성공");
        assert_ne!(new_id, id);
        match &node {
            LayoutNode::Split { dir, ratio, a, b } => {
                assert_eq!(*dir, SplitDir::LeftRight);
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
        let (mut node, id) = single_slot();
        let mid = split_in_tree(&mut node, id, SplitDir::LeftRight).unwrap();
        let deep = split_in_tree(&mut node, mid, SplitDir::TopBottom).expect("중첩 split 성공");
        assert!(contains_slot(&node, id));
        assert!(contains_slot(&node, mid));
        assert!(contains_slot(&node, deep));
        assert_eq!(count_splits(&node), 2);
    }

    #[test]
    fn split_missing_slot_is_noop() {
        let (mut node, _id) = single_slot();
        let before = node.clone();
        assert!(split_in_tree(&mut node, Uuid::new_v4(), SplitDir::LeftRight).is_none());
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
        let new_id = split_in_tree(&mut node, id, SplitDir::LeftRight).unwrap();
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
        let new_id = split_in_tree(&mut node, id, SplitDir::TopBottom).unwrap();
        assert!(close_in_tree(&mut node, id));
        match &node {
            LayoutNode::Slot { id: rid, .. } => assert_eq!(*rid, new_id),
            _ => panic!("close 후 단일 슬롯이어야 함"),
        }
    }

    #[test]
    fn close_nested_promotes_subtree() {
        let (mut node, id) = single_slot();
        let b_id = split_in_tree(&mut node, id, SplitDir::LeftRight).unwrap();
        let y_id = split_in_tree(&mut node, b_id, SplitDir::TopBottom).unwrap();
        // 이제 트리: Split{ Slot(id), Split{ Slot(b_id), Slot(y_id) } }
        assert!(close_in_tree(&mut node, b_id), "중첩 슬롯 close");
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
        let new_id = split_in_tree(&mut node, id, SplitDir::LeftRight).unwrap();
        assert!(assign_in_tree(&mut node, new_id, Some("agent-b".into())));
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

    // ── set_in_tree (제네릭 콘텐츠 교체 — ADR-0063 set_slot_content) ─────────────────

    #[test]
    fn set_in_tree_replaces_with_agent_list() {
        let (mut node, id) = single_slot();
        assert!(set_in_tree(&mut node, id, SlotContent::AgentList));
        assert_eq!(find_slot(&node, id).unwrap(), &SlotContent::AgentList);
    }

    #[test]
    fn set_in_tree_replaces_with_preset_palette() {
        let (mut node, id) = single_slot();
        assert!(set_in_tree(&mut node, id, SlotContent::PresetPalette));
        assert_eq!(find_slot(&node, id).unwrap(), &SlotContent::PresetPalette);
    }

    #[test]
    fn set_in_tree_can_clear_to_empty() {
        let id = Uuid::new_v4();
        let mut node = agent_slot(id, "occupant");
        assert!(set_in_tree(&mut node, id, SlotContent::Empty));
        assert!(find_slot(&node, id).unwrap().is_empty());
    }

    #[test]
    fn set_in_tree_overwrites_occupied_slot() {
        let id = Uuid::new_v4();
        let mut node = agent_slot(id, "old");
        assert!(set_in_tree(&mut node, id, SlotContent::AgentList));
        assert_eq!(find_slot(&node, id).unwrap(), &SlotContent::AgentList);
    }

    #[test]
    fn set_in_tree_targets_correct_slot_in_split() {
        let (mut node, id) = single_slot();
        let new_id = split_in_tree(&mut node, id, SplitDir::LeftRight).unwrap();
        assert!(set_in_tree(&mut node, new_id, SlotContent::PresetPalette));
        assert_eq!(
            find_slot(&node, new_id).unwrap(),
            &SlotContent::PresetPalette
        );
        assert!(find_slot(&node, id).unwrap().is_empty());
    }

    #[test]
    fn set_in_tree_missing_slot_is_noop() {
        let (mut node, _id) = single_slot();
        let before = node.clone();
        assert!(!set_in_tree(
            &mut node,
            Uuid::new_v4(),
            SlotContent::AgentList
        ));
        assert_eq!(node, before, "없는 slot set 은 트리 불변");
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
        let _new_id = split_in_tree(&mut node, id, SplitDir::LeftRight).unwrap();
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
        let (mut node, id) = single_slot();
        let new_id = split_in_tree(&mut node, id, SplitDir::LeftRight).unwrap();
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
        let new_id = split_in_tree(&mut node, id, SplitDir::LeftRight).unwrap();
        assert!(assign_in_tree(&mut node, id, Some("x".into())));
        assert!(assign_in_tree(&mut node, new_id, Some("y".into())));
        assert_eq!(first_empty_slot_id(&node), None, "전부 점유 → None");
    }
}
