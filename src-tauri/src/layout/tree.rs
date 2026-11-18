//! Pure split-tree operations — zero Tauri dependency (ADR-0012: testable standalone, headless). ViewManager delegates every real tree mutation here; this module knows only `LayoutNode` — not Tauri, AppState or locks — so it is asserted standalone under `#[cfg(test)]`.
//! Invariant: content is written by exactly one function, [`set_in_tree`] — it replaces only the target slot's content (SlotContent) and leaves the tree structure unchanged (ADR-0060); `ViewManager` calls it from a single write statement only (the side table for unknown content — head of `manager.rs`).

use uuid::Uuid;

use super::types::{LayoutNode, SlotContent, SplitDir};

/// Shell-wide limit of the split ratio (= the share taken by side a, the left/top area).
// ADR-0227 · ADR-0140
// ADR-0260: the area minimum is handled by the px clamp (`min_pane_px`) — with this value that clamp wins up to a 3000px axis. But it must be greater than 0: `geometry::boundary` uses this limit to cut extreme values planted directly in the tree, preventing a zero-area leaf.
pub const RATIO_MIN: f64 = 0.01;
pub const RATIO_MAX: f64 = 0.99;

pub const SPLIT_RATIO: f64 = 0.5;

// NaN stays NaN — filtering it is the caller's job (on the write path the manager accepts only finite values, and geometry reads NaN as a half split).
pub fn clamp_ratio(r: f64) -> f64 {
    r.clamp(RATIO_MIN, RATIO_MAX)
}

/// Summary of one split — a row of `list_splits`. `a_slots` · `b_slots` = the area ids in that split's a (left/top) · b (right/bottom) subtree, each in tree preorder.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitInfo {
    pub id: Uuid,
    pub dir: SplitDir,
    pub ratio: f64,
    pub a_slots: Vec<Uuid>,
    pub b_slots: Vec<Uuid>,
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

// A tree always has at least one slot, so this never fails.
pub fn first_slot_id(node: &LayoutNode) -> Uuid {
    match node {
        LayoutNode::Slot { id, .. } => *id,
        LayoutNode::Split { a, .. } => first_slot_id(a),
    }
}

// Unlike first_slot_id (which ignores occupancy) this picks only an empty slot. Do not use it for auto placement: it sees only the tree and answers that a slot holding unknown content (`Empty` in memory) is empty — auto placement goes through `ViewManager::resolve_spawn_slot` · `slot_is_free` (TRD S21-storage §6-2). // ADR-0059
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

// Replaces the target Slot with `Split{id=new id, dir, ratio:0.5, a=original slot, b=new empty slot}` and returns the new empty slot's id (for focus movement and validation). slot_id absent → tree unchanged, None (no-op).
pub fn split_in_tree(
    node: &mut LayoutNode,
    slot_id: Uuid,
    dir: super::types::SplitDir,
) -> Option<Uuid> {
    match node {
        LayoutNode::Slot { id, .. } => {
            if *id == slot_id {
                let new_slot = LayoutNode::new_empty_slot();
                let new_slot_id = match &new_slot {
                    LayoutNode::Slot { id, .. } => *id,
                    _ => unreachable!("new_empty_slot is always a Slot"),
                };
                let original = std::mem::replace(node, LayoutNode::new_empty_slot());
                *node = LayoutNode::Split {
                    id: Uuid::new_v4(), // ADR-0223 — a new id for every split
                    dir,
                    ratio: SPLIT_RATIO,
                    a: Box::new(original),
                    b: Box::new(new_slot),
                };
                Some(new_slot_id)
            } else {
                None
            }
        }
        LayoutNode::Split { a, b, .. } => {
            if let Some(found) = split_in_tree(a, slot_id, dir) {
                Some(found)
            } else {
                split_in_tree(b, slot_id, dir)
            }
        }
    }
}

/// Writes the ratio of split `split_id` as `ratio` as is — clamping and validation are the manager's job. Absent → tree unchanged, `false`.
// ADR-0227
pub fn set_ratio_in_tree(node: &mut LayoutNode, split_id: Uuid, ratio: f64) -> bool {
    match node {
        LayoutNode::Slot { .. } => false,
        LayoutNode::Split {
            id, ratio: r, a, b, ..
        } => {
            if *id == split_id {
                *r = ratio;
                true
            } else {
                set_ratio_in_tree(a, split_id, ratio) || set_ratio_in_tree(b, split_id, ratio)
            }
        }
    }
}

pub fn split_ratio(node: &LayoutNode, split_id: Uuid) -> Option<f64> {
    match node {
        LayoutNode::Slot { .. } => None,
        LayoutNode::Split {
            id, ratio, a, b, ..
        } => {
            if *id == split_id {
                Some(*ratio)
            } else {
                split_ratio(a, split_id).or_else(|| split_ratio(b, split_id))
            }
        }
    }
}

pub fn slot_ids(node: &LayoutNode) -> Vec<Uuid> {
    fn walk(node: &LayoutNode, out: &mut Vec<Uuid>) {
        match node {
            LayoutNode::Slot { id, .. } => out.push(*id),
            LayoutNode::Split { a, b, .. } => {
                walk(a, out);
                walk(b, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(node, &mut out);
    out
}

pub fn list_splits(node: &LayoutNode) -> Vec<SplitInfo> {
    fn walk(node: &LayoutNode, out: &mut Vec<SplitInfo>) {
        if let LayoutNode::Split {
            id,
            dir,
            ratio,
            a,
            b,
        } = node
        {
            out.push(SplitInfo {
                id: *id,
                dir: *dir,
                ratio: *ratio,
                a_slots: slot_ids(a),
                b_slots: slot_ids(b),
            });
            walk(a, out);
            walk(b, out);
        }
    }
    let mut out = Vec::new();
    walk(node, &mut out);
    out
}

// - A closing slot that is a direct child of some Split → replace that Split with the sibling (sibling promotion); the sibling moves whole as a node, since regenerating it would change the split id. // ADR-0223
// - A closing slot that is the root itself (only one slot in the tree) → reset to a new empty slot (the View stays empty).
// - slot_id absent → tree unchanged, false (no-op). Returns true if actually closed; on false the caller returns Err for an invalid id.
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

// slot_id absent → no-op (false). Overwrite semantics (ADR-0058): an occupied slot is replaced unconditionally too — occupancy defense is `ViewManager::resolve_spawn_slot`'s layer.
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
        let found = find_slot(&node, id).expect("the slot must be found");
        assert_eq!(found.agent_id(), Some("agent-x"));
    }

    // ── SlotContent helper methods + serde round-trip (golden, ADR-0060) ─────────────

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
        // ADR-0060
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
        let new_id = split_in_tree(&mut node, id, SplitDir::LeftRight).expect("split succeeds");
        assert_ne!(new_id, id);
        match &node {
            LayoutNode::Split {
                dir, ratio, a, b, ..
            } => {
                assert_eq!(*dir, SplitDir::LeftRight);
                assert_eq!(*ratio, 0.5, "split default ratio 0.5");
                assert!(matches!(a.as_ref(), LayoutNode::Slot { id: aid, .. } if *aid == id));
                assert!(matches!(b.as_ref(), LayoutNode::Slot { id: bid, .. } if *bid == new_id));
            }
            _ => panic!("after split the root must be Split"),
        }
        assert!(contains_slot(&node, id));
        assert!(contains_slot(&node, new_id));
    }

    #[test]
    fn split_nested_targets_deep_leaf() {
        let (mut node, id) = single_slot();
        let mid = split_in_tree(&mut node, id, SplitDir::LeftRight).unwrap();
        let deep =
            split_in_tree(&mut node, mid, SplitDir::TopBottom).expect("nested split succeeds");
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
        assert_eq!(
            node, before,
            "splitting a nonexistent slot leaves the tree unchanged"
        );
    }

    fn count_splits(node: &LayoutNode) -> usize {
        match node {
            LayoutNode::Slot { .. } => 0,
            LayoutNode::Split { a, b, .. } => 1 + count_splits(a) + count_splits(b),
        }
    }

    fn split_id(node: &LayoutNode) -> Uuid {
        match node {
            LayoutNode::Split { id, .. } => *id,
            LayoutNode::Slot { .. } => panic!("must be a Split: {node:?}"),
        }
    }

    // Split(R){x, Split(S){y, z}} — returns (tree, x, y, z, R, S).
    fn nested_split() -> (LayoutNode, Uuid, Uuid, Uuid, Uuid, Uuid) {
        let (mut node, x) = single_slot();
        let y = split_in_tree(&mut node, x, SplitDir::LeftRight).unwrap();
        let root = split_id(&node);
        let z = split_in_tree(&mut node, y, SplitDir::TopBottom).unwrap();
        let LayoutNode::Split { b, .. } = &node else {
            unreachable!()
        };
        let inner = split_id(b);
        (node, x, y, z, root, inner)
    }

    #[test]
    fn every_split_gets_a_fresh_id() {
        let (node, _x, _y, _z, root, inner) = nested_split();
        assert_ne!(root, inner);
        assert_eq!(
            split_id(&node),
            root,
            "the inner split does not change the outer split's id"
        );
    }

    // ── close: sibling promote ─────────────────────────────────────────────

    #[test]
    fn close_promotes_sibling() {
        let (mut node, id) = single_slot();
        let new_id = split_in_tree(&mut node, id, SplitDir::LeftRight).unwrap();
        assert!(close_in_tree(&mut node, new_id));
        match &node {
            LayoutNode::Slot { id: rid, .. } => {
                assert_eq!(*rid, id, "sibling a is promoted to root")
            }
            _ => panic!("after close it must be a single slot"),
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
            _ => panic!("after close it must be a single slot"),
        }
    }

    #[test]
    fn close_nested_promotes_subtree() {
        let (mut node, id) = single_slot();
        let b_id = split_in_tree(&mut node, id, SplitDir::LeftRight).unwrap();
        let y_id = split_in_tree(&mut node, b_id, SplitDir::TopBottom).unwrap();
        // The tree now: Split{ Slot(id), Split{ Slot(b_id), Slot(y_id) } }
        assert!(close_in_tree(&mut node, b_id), "closing a nested slot");
        assert!(contains_slot(&node, id));
        assert!(contains_slot(&node, y_id));
        assert!(!contains_slot(&node, b_id));
        assert_eq!(count_splits(&node), 1);
    }

    #[test]
    fn close_promotes_sibling_split_with_its_own_id() {
        let (mut node, x, _y, _z, root, inner) = nested_split();
        assert!(close_in_tree(&mut node, x));
        assert_eq!(
            split_id(&node),
            inner,
            "the promoted split brings its own id up"
        );
        assert_ne!(split_id(&node), root);
    }

    #[test]
    fn close_inside_a_child_keeps_the_parent_split_id() {
        let (mut node, x, y, z, root, _inner) = nested_split();
        assert!(close_in_tree(&mut node, y));
        assert_eq!(
            split_id(&node),
            root,
            "a split whose only children changed keeps its id"
        );
        let LayoutNode::Split { a, b, .. } = &node else {
            unreachable!()
        };
        assert_eq!(first_slot_id(a), x);
        assert!(matches!(b.as_ref(), LayoutNode::Slot { id, .. } if *id == z));
    }

    // ── close: root slot → reset to empty ───────────────────────────────────

    #[test]
    fn close_root_slot_resets_to_empty() {
        let id = Uuid::new_v4();
        let mut node = agent_slot(id, "agent-x");
        assert!(
            close_in_tree(&mut node, id),
            "closing the root slot is true"
        );
        match &node {
            LayoutNode::Slot { id: rid, content } => {
                assert_ne!(*rid, id, "new empty slot id");
                assert!(content.is_empty(), "empty slot");
            }
            _ => panic!("still a single slot after closing the root slot"),
        }
    }

    #[test]
    fn close_missing_slot_is_noop() {
        let (mut node, _id) = single_slot();
        let before = node.clone();
        assert!(
            !close_in_tree(&mut node, Uuid::new_v4()),
            "closing a nonexistent slot is false"
        );
        assert_eq!(node, before, "tree unchanged");
    }

    // ── set_in_tree (generic content replacement — ADR-0063 set_slot_content) ─────────────────

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
        assert_eq!(
            node, before,
            "setting a nonexistent slot leaves the tree unchanged"
        );
    }

    // ── ratio clamp ────────────────────────────────────────────────────────

    #[test]
    fn ratio_clamps_to_the_shell_bounds() {
        assert_eq!(clamp_ratio(-0.5), RATIO_MIN);
        assert_eq!(clamp_ratio(0.0), RATIO_MIN);
        assert_eq!(clamp_ratio(0.005), RATIO_MIN);
        assert_eq!(clamp_ratio(1.5), RATIO_MAX);
        assert_eq!(clamp_ratio(1.0), RATIO_MAX);
        assert_eq!(clamp_ratio(0.995), RATIO_MAX);
        assert_eq!(clamp_ratio(f64::INFINITY), RATIO_MAX);
        assert_eq!(clamp_ratio(f64::NEG_INFINITY), RATIO_MIN);
        assert_eq!(clamp_ratio(0.01), 0.01);
        assert_eq!(clamp_ratio(0.99), 0.99);
        assert_eq!(clamp_ratio(0.05), 0.05);
        assert_eq!(clamp_ratio(0.95), 0.95);
        assert_eq!(clamp_ratio(0.3), 0.3);
    }

    // ── set_ratio_in_tree · list_splits (ADR-0227) ───────────────────────────

    fn ratio_of(node: &LayoutNode, split: Uuid) -> f64 {
        split_ratio(node, split).expect("the split must exist")
    }

    #[test]
    fn set_ratio_in_tree_writes_only_the_split_with_that_id() {
        let (mut node, _x, _y, _z, root, inner) = nested_split();
        assert!(set_ratio_in_tree(&mut node, inner, 0.3));
        assert_eq!(
            ratio_of(&node, inner),
            0.3,
            "the received value goes in as is (f64)"
        );
        assert_eq!(
            ratio_of(&node, root),
            SPLIT_RATIO,
            "other splits stay as they are"
        );

        assert!(set_ratio_in_tree(&mut node, root, 0.7));
        assert_eq!(ratio_of(&node, root), 0.7);
        assert_eq!(ratio_of(&node, inner), 0.3);
    }

    #[test]
    fn set_ratio_in_tree_missing_split_is_noop() {
        let (mut node, x, _y, _z, _root, _inner) = nested_split();
        let before = node.clone();
        assert!(!set_ratio_in_tree(&mut node, Uuid::new_v4(), 0.3));
        // An area id is not a split id.
        assert!(!set_ratio_in_tree(&mut node, x, 0.3));
        assert_eq!(
            node, before,
            "writing a nonexistent split leaves the tree unchanged"
        );

        let (mut lone, id) = single_slot();
        let before = lone.clone();
        assert!(!set_ratio_in_tree(&mut lone, id, 0.3));
        assert_eq!(lone, before);
    }

    #[test]
    fn list_splits_is_preorder_with_the_slots_under_each_side() {
        // Split(R, LeftRight){x, Split(S, TopBottom){y, z}}
        let (mut node, x, y, z, root, inner) = nested_split();
        assert!(set_ratio_in_tree(&mut node, inner, 0.25));
        let rows = list_splits(&node);
        assert_eq!(
            rows,
            vec![
                SplitInfo {
                    id: root,
                    dir: SplitDir::LeftRight,
                    ratio: SPLIT_RATIO,
                    a_slots: vec![x],
                    b_slots: vec![y, z],
                },
                SplitInfo {
                    id: inner,
                    dir: SplitDir::TopBottom,
                    ratio: 0.25,
                    a_slots: vec![y],
                    b_slots: vec![z],
                },
            ]
        );
        assert!(
            list_splits(&single_slot().0).is_empty(),
            "no split with a single area"
        );
    }

    // ── first_slot_id (the core of focus fallback) ────────────────────────────────

    #[test]
    fn first_slot_id_is_leftmost() {
        let (mut node, id) = single_slot();
        let _new_id = split_in_tree(&mut node, id, SplitDir::LeftRight).unwrap();
        assert_eq!(first_slot_id(&node), id);
    }

    // ── first_empty_slot_id (spawn_into slot=None policy — USER DECISION 2b) ────────

    #[test]
    fn first_empty_slot_id_single_empty() {
        let (node, id) = single_slot();
        assert_eq!(
            first_empty_slot_id(&node),
            Some(id),
            "an empty single slot is itself"
        );
    }

    #[test]
    fn first_empty_slot_id_single_occupied_is_none() {
        let id = Uuid::new_v4();
        let node = agent_slot(id, "a");
        assert_eq!(
            first_empty_slot_id(&node),
            None,
            "an occupied single slot gives no empty slot"
        );
    }

    #[test]
    fn first_empty_slot_id_skips_occupied_leftmost() {
        let (mut node, id) = single_slot();
        let new_id = split_in_tree(&mut node, id, SplitDir::LeftRight).unwrap();
        assert!(set_in_tree(&mut node, id, SlotContent::AgentList));
        assert_eq!(
            first_empty_slot_id(&node),
            Some(new_id),
            "skips the occupied left and gives the first empty slot (right)"
        );
    }

    #[test]
    fn first_empty_slot_id_all_occupied_is_none() {
        let (mut node, id) = single_slot();
        let new_id = split_in_tree(&mut node, id, SplitDir::LeftRight).unwrap();
        assert!(set_in_tree(&mut node, id, SlotContent::AgentList));
        assert!(set_in_tree(&mut node, new_id, SlotContent::PresetPalette));
        assert_eq!(first_empty_slot_id(&node), None, "all occupied → None");
    }
}
