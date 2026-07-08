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

    /// 탭 바용 메타 목록. ★Fix 2B: 창에 바인딩된 View 는 제외★ — 팝업 등 별도 창에 묶인 View 는 main 탭
    /// 바에 유령 탭으로 뜨면 안 된다(window_bindings 에 있는 view_id 는 그 창 전용). 단 **active_view_id 는
    /// 절대 제외하지 않는다**(방어 가드): 미래에 어떤 창이 active/main View 에 바인딩되더라도 main 탭은 항상
    /// 남아야 한다(agent-tree 가 active 를 바인딩하는 함정 방지). 즉 v 유지 = active 이거나 어떤 바인딩에도
    /// 안 걸릴 때.
    pub fn view_metas(&self) -> Vec<ViewMeta> {
        self.views
            .iter()
            .filter(|v| {
                v.id == self.active_view_id || !self.window_bindings.values().any(|&bv| bv == v.id)
            })
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

    /// view 안 slot_id 슬롯에 배정된 agent_id(참조 문자열)를 반환. 빈 슬롯이면 Ok(None).
    /// ★팝업 분리(pop_out_slot)용★: 원본 슬롯의 agent 를 읽어 새 View 로 옮길 때 쓴다(조회만 — 변형·version 불변).
    /// invalid view_id/slot_id → Err(no-op).
    pub fn slot_agent(&self, view_id: Uuid, slot_id: Uuid) -> Result<Option<String>, LayoutError> {
        let v = self
            .views
            .iter()
            .find(|v| v.id == view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        tree::find_slot(&v.layout, slot_id)
            .cloned()
            .ok_or(LayoutError::SlotNotFound(slot_id))
    }

    /// window_label → view_id 바인딩을 삽입(팝업 창을 특정 View 에 고정). ★일반 라우팅 메커니즘(ADR-0046)★:
    /// OutputRouter.rebuild 가 이 맵을 읽어 그 label 로 View 의 agent 출력을 라우팅한다. close_view 가
    /// retain 으로, Destroyed 이벤트가 unbind_window 로 정리한다(누수 방지). version 은 올리지 않는다
    /// (바인딩은 탭 목록/레이아웃 트리와 무관 — 라우팅 표만 재계산하면 됨, 호출자가 rebuild).
    pub fn bind_window(&mut self, label: String, view_id: Uuid) -> Result<(), LayoutError> {
        if !self.views.iter().any(|v| v.id == view_id) {
            return Err(LayoutError::ViewNotFound(view_id));
        }
        self.window_bindings.insert(label, view_id);
        Ok(())
    }

    /// window_label 바인딩 제거(팝업 창 Destroyed 시 라우팅 정리). 없던 label 이면 조용히 no-op.
    /// ★수명/누수 임계(load-bearing)★: 이걸 안 부르면 죽은 창 label 이 window_bindings 에 남아
    /// OutputRouter 가 그 label 로 계속 라우팅을 시도한다(registry 미등록이라 실질 no-op 이나 stale binding).
    pub fn unbind_window(&mut self, label: &str) {
        self.window_bindings.remove(label);
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

    /// View 닫기. active 면 다른 View 로 전환(★첫 *언바인딩* View★ — ADR-0035), 남은 언바인딩 View 가
    /// 없거나 마지막이면 새 빈 View 생성(빈 상태). invalid view_id → no-op + Err.
    pub fn close_view(&mut self, view_id: Uuid) -> Result<(), LayoutError> {
        let idx = self
            .views
            .iter()
            .position(|v| v.id == view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        self.views.remove(idx);
        // 이 View 에 바인딩된 창(팝업/tree)들의 바인딩 정리.
        self.window_bindings.retain(|_, vid| *vid != view_id);

        // 닫은 게 active 였으면 새 active 를 다시 고른다. ★ADR-0035: window_bindings 에 묶인 View(팝업 등)
        //   는 절대 active(=main 창 전용 개념)로 승격하면 안 된다★ — 승격하면 view_metas 의 active-예외가
        //   그 View 를 탭으로 되살리고 OutputRouter 가 같은 agent 표면을 main-active + 팝업-bound 양쪽으로
        //   라우팅한다(직교 붕괴). 그래서 "첫 *언바인딩* View"를 고르고, 언바인딩 View 가 하나도 없으면
        //   (전부 창에 묶임) 새 빈 언바인딩 View 를 만들어 active 로 삼는다(마지막 View 닫힘 분기와 동형).
        if self.active_view_id == view_id {
            let first_unbound = self
                .views
                .iter()
                .find(|v| !self.window_bindings.values().any(|&bv| bv == v.id))
                .map(|v| v.id);
            match first_unbound {
                Some(id) => self.active_view_id = id,
                None => {
                    // 남은 언바인딩 View 0(빈 목록이거나 전부 바인딩됨) → 새 빈 언바인딩 View 생성.
                    //   (옛 "마지막 View 닫음" 분기를 흡수 — 빈 목록도 여기로 떨어진다. 새 View 는 어떤
                    //   window_bindings 에도 없으니 언바인딩 불변식 자동 성립.)
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
                }
            }
        }
        // 닫은 게 비-active 면 active 는 유지된다(active 는 방금 지운 view 가 아니므로 여전히 유효).
        //   비-active 를 닫아 목록이 비는 경우는 없다(단일 View 는 항상 active).
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
    fn close_active_never_promotes_bound_popup_view_to_active() {
        // Finding 2(ADR-0035): main active A(언바인딩) + 팝업 백킹 View P(바인딩). A 를 닫으면 새 active 는
        //   P 가 절대 아니어야 한다 — P 는 window_bindings 에 묶여 있어 main active 로 승격 금지. 남은
        //   언바인딩 View 가 없으므로(P 뿐) 새 빈 언바인딩 View 가 생겨 그게 active 가 된다.
        let mut mgr = ViewManager::new();
        let a = mgr.active_view_id; // main active(언바인딩)
        let p = mgr.create_view(Some("Popup 1".into())); // create_view 가 active 를 P 로 바꿈
        let _ = mgr.switch_view(a); // active=A 복원(팝업은 바인딩 전용, pop_out_slot 과 동형)
        mgr.bind_window("slot-popup-1".into(), p).unwrap(); // P 를 팝업 창에 바인딩

        mgr.close_view(a).unwrap(); // main 탭(A) 닫음

        // ★핵심 단언★: 새 active 는 P(바인딩된 팝업 View)가 아니다.
        assert_ne!(
            mgr.active_view_id, p,
            "바인딩된 팝업 View 를 active 로 승격 금지(ADR-0035)"
        );
        // 새 active 는 어떤 window_bindings 에도 안 걸린 언바인딩 View 여야 한다.
        assert!(
            !mgr.window_bindings
                .values()
                .any(|&bv| bv == mgr.active_view_id),
            "새 active 는 언바인딩 View"
        );
        // 그리고 그 새 active 는 방금 만든 빈 View(P 도, 닫은 A 도 아님).
        assert_ne!(mgr.active_view_id, a, "닫은 View 는 active 가 될 수 없음");
        // view_metas 는 P 를 노출하지 않는다(바인딩 + 비활성이라 탭 바에서 제외 — Fix 2B 유지).
        let metas = mgr.view_metas();
        assert!(
            !metas.iter().any(|m| m.id == p),
            "바인딩된 팝업 View 는 탭 목록에 뜨지 않음(active-예외에도 안 걸림)"
        );
        // 새 active 는 탭 목록에 있다(main 탭 존재 보장).
        assert!(
            metas.iter().any(|m| m.id == mgr.active_view_id),
            "새 active(언바인딩)는 탭 목록에 있음"
        );
    }

    #[test]
    fn close_active_picks_first_unbound_when_one_exists() {
        // 언바인딩 View 가 하나 남아 있으면 새 빈 View 를 만들지 말고 그 언바인딩 View 를 active 로.
        // 배치: A(active,언바인딩), B(언바인딩), P(바인딩). A 닫음 → active = B(첫 언바인딩), 새 View 생성 X.
        let mut mgr = ViewManager::new();
        let a = mgr.active_view_id;
        let b = mgr.create_view(Some("B".into()));
        let p = mgr.create_view(Some("Popup".into()));
        let _ = mgr.switch_view(a);
        mgr.bind_window("slot-popup-1".into(), p).unwrap();
        let count_before = mgr.views.len(); // A,B,P = 3

        mgr.close_view(a).unwrap();

        assert_eq!(
            mgr.active_view_id, b,
            "첫 언바인딩 View(B)로 전환 — 새 View 안 만듦"
        );
        assert_eq!(
            mgr.views.len(),
            count_before - 1,
            "A 만 제거(새 View 생성 없음)"
        );
        assert_ne!(mgr.active_view_id, p, "바인딩 View 는 active 금지");
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
        // 바인딩이 없으면 모든 View 를 나열한다(Fix 2B 필터가 무바인딩 상태는 안 건드림).
        let mut mgr = ViewManager::new();
        mgr.create_view(Some("Second".into()));
        let metas = mgr.view_metas();
        assert_eq!(metas.len(), 2);
        assert_eq!(metas[1].name, "Second");
    }

    #[test]
    fn view_metas_excludes_window_bound_view() {
        // Fix 2B: 팝업 창에 바인딩된 (비활성) View 는 main 탭 바 메타에서 제외된다(유령 탭 방지).
        let mut mgr = ViewManager::new();
        let main = mgr.active_view_id;
        let popup_view = mgr.create_view(Some("Popup 1".into()));
        let _ = mgr.switch_view(main); // active=main 유지(팝업은 바인딩 전용)
        mgr.bind_window("slot-popup-1".into(), popup_view).unwrap();

        let metas = mgr.view_metas();
        assert_eq!(metas.len(), 1, "바인딩된 팝업 View 는 탭 목록에서 빠짐");
        assert_eq!(metas[0].id, main, "남는 건 active/main View 뿐");
        assert!(
            !metas.iter().any(|m| m.id == popup_view),
            "바인딩된 View id 는 메타에 없음"
        );
    }

    #[test]
    fn view_metas_never_excludes_active_even_if_bound() {
        // Fix 2B 방어 가드: active_view_id 는 어떤 창이 바인딩하더라도 절대 제외되지 않는다
        // (agent-tree 가 active 를 바인딩하는 함정 방지 — main 탭은 항상 남아야 한다).
        let mut mgr = ViewManager::new();
        let main = mgr.active_view_id;
        // active(main) 자신을 바인딩해도 탭에서 사라지면 안 된다.
        mgr.bind_window("some-window".into(), main).unwrap();

        let metas = mgr.view_metas();
        assert!(
            metas.iter().any(|m| m.id == main),
            "active View 는 바인딩돼도 탭에 남는다(방어 가드)"
        );
    }
}
