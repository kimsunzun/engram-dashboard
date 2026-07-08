//! ViewManager — 레이아웃 권위 상태(ADR-0035). AppState 가 `Arc<Mutex<ViewManager>>` 로 소유.
//!
//! ★Tauri 의존 0★: 이 타입은 락·emit 을 모른다. 락 취득/해제·emit 은 command 레이어
//! (`commands/layout.rs`)가 한다(ADR-0006: 락 해제 후 emit). 그래서 mutation 메서드는 변경 결과
//! (영향받은 view_id·갱신된 스냅샷·뷰 목록)를 **반환만** 하고, 여기서 직접 emit 하지 않는다 →
//! 단독 unit 테스트 가능(headless).
//!
//! invalid view_id/slot_id → no-op + Err(LayoutError)(패닉·부분변경 금지, TRD 하드 계약).

use std::collections::HashMap;

use uuid::Uuid;

use super::tree;
use super::types::{LayoutNode, SplitDir, View, ViewMeta, ViewSnapshot};

/// 레이아웃 연산 실패 사유. invalid id 는 no-op + 이 에러(부분변경 금지).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    #[error("view 없음: {0}")]
    ViewNotFound(Uuid),
    #[error("slot 없음: {0}")]
    SlotNotFound(Uuid),
}

/// 레이아웃 권위 상태. invoke 스레드풀 동시접근 → AppState 가 Mutex 로 감싼다.
pub struct ViewManager {
    pub views: Vec<View>,
    /// 메인 창의 활성 탭(ADR-0035/TRD: switch_view 가 이걸 바꾼다). 보조 창(tree 등)은 window_bindings.
    pub active_view_id: Uuid,
    /// window_label → view_id. ★일반 라우팅 메커니즘(ADR-0046)★: 메인 외 창(예: agent-tree)이 고정
    /// View 에 바인딩돼 자기 뷰만 렌더한다. OutputRouter.rebuild 가 이 맵을 읽어 (active=main) + (바인딩된
    /// label 들)로 출력을 라우팅한다 — 특정 창에 묶이지 않은 범용 표면이라 미래 창(임의 view→창 pop 등)도
    /// 이 맵에 label 을 넣기만 하면 흡수된다. (옛 정적 slot-popup 창은 제거됐고 그 정적 target 도 사라졌다.)
    pub window_bindings: HashMap<String, Uuid>,
    /// 변경마다 +1(get_view race 용 — 팝업 pull↔listen 윈도). 0 부터 시작, 첫 변경에서 1.
    pub version: u64,
}

impl Default for ViewManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewManager {
    /// 초기 상태: 빈 슬롯 하나를 담은 기본 View 1개 + 그게 활성.
    pub fn new() -> Self {
        let view = View {
            id: Uuid::new_v4(),
            name: "View 1".to_string(),
            layout: LayoutNode::new_empty_slot(),
            focused_slot_id: None,
        };
        let active_view_id = view.id;
        // 첫 슬롯을 포커스로(빈 슬롯이라도 포커스 대상은 존재).
        let mut mgr = Self {
            active_view_id,
            views: vec![view],
            window_bindings: HashMap::new(),
            version: 0,
        };
        mgr.refocus_active();
        mgr
    }

    // ── 조회 ───────────────────────────────────────────────────────────────

    /// 탭 바용 메타 목록.
    pub fn view_metas(&self) -> Vec<ViewMeta> {
        self.views
            .iter()
            .map(|v| ViewMeta {
                id: v.id,
                name: v.name.clone(),
            })
            .collect()
    }

    /// view_id 의 스냅샷(get_view·layout:updated 페이로드). 없으면 Err.
    pub fn snapshot(&self, view_id: Uuid) -> Result<ViewSnapshot, LayoutError> {
        let v = self
            .views
            .iter()
            .find(|v| v.id == view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        Ok(ViewSnapshot {
            view_id: v.id,
            layout: v.layout.clone(),
            focused_slot_id: v.focused_slot_id,
            version: self.version,
        })
    }

    fn view_mut(&mut self, view_id: Uuid) -> Result<&mut View, LayoutError> {
        self.views
            .iter_mut()
            .find(|v| v.id == view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))
    }

    /// active View 의 focus 가 유효한 슬롯을 가리키게 보정(없으면 첫 슬롯). 초기화·연산 후 호출.
    fn refocus_active(&mut self) {
        let active_id = self.active_view_id;
        if let Some(v) = self.views.iter_mut().find(|v| v.id == active_id) {
            Self::fixup_focus(v);
        }
    }

    /// focus fallback — focused_slot_id 가 가리키던 슬롯이 사라지면 트리 첫 슬롯으로(항상 ≥1 슬롯).
    fn fixup_focus(view: &mut View) {
        let valid = view
            .focused_slot_id
            .map(|fid| tree::contains_slot(&view.layout, fid))
            .unwrap_or(false);
        if !valid {
            view.focused_slot_id = Some(tree::first_slot_id(&view.layout));
        }
    }

    fn bump_version(&mut self) {
        self.version += 1;
    }

    // ── mutation (각 메서드: 변경 후 version +1, focus 보정. emit 은 호출자) ─────────

    /// 새 View 생성(빈 슬롯 하나) → 그 View 를 active 로. 새 View id 반환.
    pub fn create_view(&mut self, name: Option<String>) -> Uuid {
        let id = Uuid::new_v4();
        let name = name.unwrap_or_else(|| format!("View {}", self.views.len() + 1));
        let first_slot = LayoutNode::new_empty_slot();
        let focus = tree::first_slot_id(&first_slot);
        self.views.push(View {
            id,
            name,
            layout: first_slot,
            focused_slot_id: Some(focus),
        });
        self.active_view_id = id;
        self.bump_version();
        id
    }

    /// View 닫기. active 면 다른 View 로 전환(목록 첫 View), 마지막이면 새 빈 View 생성(빈 상태).
    /// invalid view_id → no-op + Err.
    pub fn close_view(&mut self, view_id: Uuid) -> Result<(), LayoutError> {
        let idx = self
            .views
            .iter()
            .position(|v| v.id == view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        self.views.remove(idx);
        // 이 View 에 바인딩된 창(팝업/tree)들의 바인딩 정리.
        self.window_bindings.retain(|_, vid| *vid != view_id);

        if self.views.is_empty() {
            // 마지막 View 닫음 → 빈 상태 유지: 새 빈 View 1개 생성(빈 화면 + `+`).
            let id = Uuid::new_v4();
            let first_slot = LayoutNode::new_empty_slot();
            let focus = tree::first_slot_id(&first_slot);
            self.views.push(View {
                id,
                name: "View 1".to_string(),
                layout: first_slot,
                focused_slot_id: Some(focus),
            });
            self.active_view_id = id;
        } else if self.active_view_id == view_id {
            // 닫은 게 active 면 목록 첫 View 로 전환.
            self.active_view_id = self.views[0].id;
        }
        self.bump_version();
        Ok(())
    }

    /// 메인 창 활성 탭 변경(active_view_id). 팝업엔 영향 없음(TRD switch_view 의미론).
    /// invalid view_id → no-op + Err.
    pub fn switch_view(&mut self, view_id: Uuid) -> Result<(), LayoutError> {
        if !self.views.iter().any(|v| v.id == view_id) {
            return Err(LayoutError::ViewNotFound(view_id));
        }
        self.active_view_id = view_id;
        self.bump_version();
        Ok(())
    }

    /// view 안 slot_id 슬롯을 분할. 새 슬롯에 focus. 새 슬롯 id 반환.
    /// invalid view_id/slot_id → no-op + Err(슬롯 못 찾으면 트리 불변).
    pub fn split_slot(
        &mut self,
        view_id: Uuid,
        slot_id: Uuid,
        dir: SplitDir,
    ) -> Result<Uuid, LayoutError> {
        let v = self.view_mut(view_id)?;
        match tree::split_in_tree(&mut v.layout, slot_id, dir) {
            Some(new_id) => {
                // 새로 만든 슬롯으로 focus 이동.
                v.focused_slot_id = Some(new_id);
                self.bump_version();
                Ok(new_id)
            }
            None => Err(LayoutError::SlotNotFound(slot_id)),
        }
    }

    /// view 안 slot_id 슬롯을 닫음(형제 승격, root 슬롯이면 빈 슬롯 리셋). focus 보정.
    /// invalid view_id/slot_id → no-op + Err.
    pub fn close_slot(&mut self, view_id: Uuid, slot_id: Uuid) -> Result<(), LayoutError> {
        let v = self.view_mut(view_id)?;
        if !tree::close_in_tree(&mut v.layout, slot_id) {
            return Err(LayoutError::SlotNotFound(slot_id));
        }
        Self::fixup_focus(v);
        self.bump_version();
        Ok(())
    }

    /// view 안 slot_id 슬롯에 agent_id(참조 문자열) 배정. ★데몬에 실재 검증 안 함(ADR-0035/0006).
    /// invalid view_id/slot_id → no-op + Err.
    pub fn assign_agent(
        &mut self,
        view_id: Uuid,
        slot_id: Uuid,
        agent_id: String,
    ) -> Result<(), LayoutError> {
        let v = self.view_mut(view_id)?;
        if !tree::assign_in_tree(&mut v.layout, slot_id, Some(agent_id)) {
            return Err(LayoutError::SlotNotFound(slot_id));
        }
        self.bump_version();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_slot_of(mgr: &ViewManager, view_id: Uuid) -> Uuid {
        let v = mgr.views.iter().find(|v| v.id == view_id).unwrap();
        tree::first_slot_id(&v.layout)
    }

    #[test]
    fn new_has_one_view_with_focus() {
        let mgr = ViewManager::new();
        assert_eq!(mgr.views.len(), 1);
        assert_eq!(mgr.active_view_id, mgr.views[0].id);
        assert!(mgr.views[0].focused_slot_id.is_some(), "초기 focus 설정됨");
        assert_eq!(mgr.version, 0);
    }

    #[test]
    fn create_view_appends_and_activates_and_bumps_version() {
        let mut mgr = ViewManager::new();
        let v0 = mgr.version;
        let id = mgr.create_view(Some("Custom".into()));
        assert_eq!(mgr.views.len(), 2);
        assert_eq!(mgr.active_view_id, id, "새 View 가 active");
        assert_eq!(mgr.views[1].name, "Custom");
        assert!(mgr.views[1].focused_slot_id.is_some());
        assert_eq!(mgr.version, v0 + 1);
    }

    #[test]
    fn create_view_default_name() {
        let mut mgr = ViewManager::new();
        mgr.create_view(None);
        assert_eq!(mgr.views[1].name, "View 2");
    }

    #[test]
    fn close_active_view_switches_to_first() {
        let mut mgr = ViewManager::new();
        let v1 = mgr.views[0].id;
        let v2 = mgr.create_view(None); // active = v2
        assert_eq!(mgr.active_view_id, v2);
        mgr.close_view(v2).unwrap();
        assert_eq!(mgr.views.len(), 1);
        assert_eq!(mgr.active_view_id, v1, "active 닫으면 첫 View 로 전환");
    }

    #[test]
    fn close_non_active_view_keeps_active() {
        let mut mgr = ViewManager::new();
        let v1 = mgr.views[0].id;
        let v2 = mgr.create_view(None);
        // active = v2. v1(비활성) 닫음.
        mgr.close_view(v1).unwrap();
        assert_eq!(mgr.active_view_id, v2, "비활성 닫아도 active 유지");
        assert_eq!(mgr.views.len(), 1);
    }

    #[test]
    fn close_last_view_resets_to_empty_view() {
        let mut mgr = ViewManager::new();
        let v1 = mgr.views[0].id;
        mgr.close_view(v1).unwrap();
        // 마지막 View 닫으면 새 빈 View 1개 (빈 상태 + `+`).
        assert_eq!(mgr.views.len(), 1);
        assert_ne!(mgr.views[0].id, v1, "새 빈 View id");
        assert!(matches!(
            mgr.views[0].layout,
            LayoutNode::Slot { agent_id: None, .. }
        ));
        assert_eq!(mgr.active_view_id, mgr.views[0].id);
    }

    #[test]
    fn close_view_clears_window_bindings() {
        let mut mgr = ViewManager::new();
        let v2 = mgr.create_view(None);
        mgr.window_bindings.insert("popup-1".into(), v2);
        mgr.close_view(v2).unwrap();
        assert!(
            !mgr.window_bindings.values().any(|&id| id == v2),
            "닫힌 View 바인딩 정리"
        );
    }

    #[test]
    fn close_invalid_view_is_err_noop() {
        let mut mgr = ViewManager::new();
        let before_len = mgr.views.len();
        let before_version = mgr.version;
        let err = mgr.close_view(Uuid::new_v4()).unwrap_err();
        assert!(matches!(err, LayoutError::ViewNotFound(_)));
        assert_eq!(mgr.views.len(), before_len, "no-op");
        assert_eq!(mgr.version, before_version, "version 안 올림");
    }

    #[test]
    fn switch_view_changes_active() {
        let mut mgr = ViewManager::new();
        let v1 = mgr.views[0].id;
        let v2 = mgr.create_view(None);
        mgr.switch_view(v1).unwrap();
        assert_eq!(mgr.active_view_id, v1);
        mgr.switch_view(v2).unwrap();
        assert_eq!(mgr.active_view_id, v2);
    }

    #[test]
    fn switch_invalid_view_is_err_noop() {
        let mut mgr = ViewManager::new();
        let active_before = mgr.active_view_id;
        let version_before = mgr.version;
        assert!(mgr.switch_view(Uuid::new_v4()).is_err());
        assert_eq!(mgr.active_view_id, active_before);
        assert_eq!(mgr.version, version_before);
    }

    #[test]
    fn split_slot_creates_new_slot_and_focuses_it() {
        let mut mgr = ViewManager::new();
        let view_id = mgr.active_view_id;
        let slot = first_slot_of(&mgr, view_id);
        let new_id = mgr.split_slot(view_id, slot, SplitDir::Horizontal).unwrap();
        let v = mgr.views.iter().find(|v| v.id == view_id).unwrap();
        assert!(matches!(v.layout, LayoutNode::Split { .. }));
        assert_eq!(v.focused_slot_id, Some(new_id), "split 후 새 슬롯에 focus");
    }

    #[test]
    fn split_invalid_view_is_err() {
        let mut mgr = ViewManager::new();
        let err = mgr
            .split_slot(Uuid::new_v4(), Uuid::new_v4(), SplitDir::Horizontal)
            .unwrap_err();
        assert!(matches!(err, LayoutError::ViewNotFound(_)));
    }

    #[test]
    fn split_invalid_slot_is_err_noop() {
        let mut mgr = ViewManager::new();
        let view_id = mgr.active_view_id;
        let before = mgr
            .views
            .iter()
            .find(|v| v.id == view_id)
            .unwrap()
            .layout
            .clone();
        let version_before = mgr.version;
        let err = mgr
            .split_slot(view_id, Uuid::new_v4(), SplitDir::Vertical)
            .unwrap_err();
        assert!(matches!(err, LayoutError::SlotNotFound(_)));
        let after = mgr
            .views
            .iter()
            .find(|v| v.id == view_id)
            .unwrap()
            .layout
            .clone();
        assert_eq!(before, after, "트리 불변");
        assert_eq!(mgr.version, version_before);
    }

    #[test]
    fn close_slot_focus_fallback_to_first() {
        let mut mgr = ViewManager::new();
        let view_id = mgr.active_view_id;
        let slot = first_slot_of(&mgr, view_id);
        let new_id = mgr.split_slot(view_id, slot, SplitDir::Horizontal).unwrap();
        // focus 는 현재 new_id. new_id 를 닫으면 focus 가 사라지므로 첫 슬롯(slot)으로 폴백.
        mgr.close_slot(view_id, new_id).unwrap();
        let v = mgr.views.iter().find(|v| v.id == view_id).unwrap();
        assert_eq!(
            v.focused_slot_id,
            Some(slot),
            "focus 사라지면 트리 첫 슬롯으로 폴백"
        );
    }

    #[test]
    fn close_root_slot_keeps_view_empty() {
        let mut mgr = ViewManager::new();
        let view_id = mgr.active_view_id;
        let slot = first_slot_of(&mgr, view_id);
        mgr.assign_agent(view_id, slot, "agent-x".into()).unwrap();
        mgr.close_slot(view_id, slot).unwrap();
        let v = mgr.views.iter().find(|v| v.id == view_id).unwrap();
        // 빈 슬롯으로 리셋(View 는 살아있음).
        assert!(matches!(v.layout, LayoutNode::Slot { agent_id: None, .. }));
        assert!(v.focused_slot_id.is_some(), "빈 슬롯에도 focus");
    }

    #[test]
    fn close_slot_invalid_is_err_noop() {
        let mut mgr = ViewManager::new();
        let view_id = mgr.active_view_id;
        let version_before = mgr.version;
        assert!(mgr.close_slot(view_id, Uuid::new_v4()).is_err());
        assert_eq!(mgr.version, version_before);
    }

    #[test]
    fn assign_agent_sets_ref() {
        let mut mgr = ViewManager::new();
        let view_id = mgr.active_view_id;
        let slot = first_slot_of(&mgr, view_id);
        mgr.assign_agent(view_id, slot, "agent-42".into()).unwrap();
        let v = mgr.views.iter().find(|v| v.id == view_id).unwrap();
        assert_eq!(
            tree::find_slot(&v.layout, slot).unwrap().as_deref(),
            Some("agent-42")
        );
    }

    #[test]
    fn assign_agent_invalid_view_is_err() {
        let mut mgr = ViewManager::new();
        assert!(mgr
            .assign_agent(Uuid::new_v4(), Uuid::new_v4(), "x".into())
            .is_err());
    }

    #[test]
    fn assign_agent_invalid_slot_is_err_noop() {
        let mut mgr = ViewManager::new();
        let view_id = mgr.active_view_id;
        let version_before = mgr.version;
        assert!(mgr
            .assign_agent(view_id, Uuid::new_v4(), "x".into())
            .is_err());
        assert_eq!(mgr.version, version_before);
    }

    #[test]
    fn snapshot_returns_version_and_layout() {
        let mut mgr = ViewManager::new();
        let view_id = mgr.active_view_id;
        let slot = first_slot_of(&mgr, view_id);
        mgr.split_slot(view_id, slot, SplitDir::Horizontal).unwrap();
        let snap = mgr.snapshot(view_id).unwrap();
        assert_eq!(snap.view_id, view_id);
        assert_eq!(snap.version, mgr.version);
        assert!(matches!(snap.layout, LayoutNode::Split { .. }));
    }

    #[test]
    fn snapshot_invalid_view_is_err() {
        let mgr = ViewManager::new();
        assert!(mgr.snapshot(Uuid::new_v4()).is_err());
    }

    #[test]
    fn view_metas_lists_all() {
        let mut mgr = ViewManager::new();
        mgr.create_view(Some("Second".into()));
        let metas = mgr.view_metas();
        assert_eq!(metas.len(), 2);
        assert_eq!(metas[1].name, "Second");
    }
}
