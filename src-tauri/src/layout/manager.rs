//! ViewManager — 레이아웃 권위 상태(ADR-0035 부분개정 · ADR-0057 탭 소유 모델). LayoutState 가
//! `Arc<Mutex<ViewManager>>` 로 소유.
//!
//! ★Tauri 의존 0★: 이 타입은 락·emit 을 모른다. 락 취득/해제·emit 은 command 레이어
//! (`commands/layout.rs`)가 한다(ADR-0006: 락 해제 후 emit). 그래서 mutation 메서드는 변경 결과
//! (영향받은 view_id·갱신된 스냅샷·탭 목록)를 **반환만** 하고, 여기서 직접 emit 하지 않는다 →
//! 단독 unit 테스트 가능(headless).
//!
//! invalid view_id/slot_id/window → no-op + Err(LayoutError)(패닉·부분변경 금지, TRD 하드 계약).
//!
//! ## ★탭 소유 모델(ADR-0057, TRD B-tabs §2)★
//! 한 창이 **탭 목록**(= 코드의 `View` 여러 벌)을 소유하고 그 안에서 전환한다. 전역 활성 뷰(옛
//! `active_view_id`)·창 바인딩(옛 `window_bindings`)은 없다. 대신:
//! - `views`      — 전역 View 풀(id lookup).
//! - `view_owner` — View → 소유 창(★유니크 소유 강제★, 캐시된 역인덱스).
//! - `windows`    — 창 → 탭 목록(`tabs: Vec<ViewId>`) + 그 창의 활성 탭(`active`).
//!
//! `agent-tree` 창은 이 모델 **밖**(config 창, /tree 렌더 — `windows` 에 키 없음, TRD §3-2).
//!
//! ### 불변식(★load-bearing — `// ADR-0057` 앵커로 박음★)
//! 1. **양방향 일관성:** `view_owner[v] == L` ⟺ `windows[L].tabs.contains(v)`. 갱신은 항상 쌍으로.
//! 2. **유니크 소유:** 모든 `v ∈ views` 는 `view_owner` 에 정확히 1개 엔트리(한 View 는 두 창 금지).
//! 3. **활성 소속:** `windows[L].active ∈ windows[L].tabs` 항상.
//! 4. **메인 최소 1탭 + non-closable:** `windows["main"].tabs.len() >= 1` 불변. `close_window("main")`
//!    은 금지(command 레이어가 거부) — 마지막 탭 close 는 빈 탭 강제로만 떨어진다.
//! 5. **에이전트 참조 다중 허용:** 같은 `agent_id` 가 서로 다른 두 View 슬롯에 배정 가능(두 창이 같은
//!    에이전트 봄, 진도 독립·ADR-0046). "한 View 두 창"(불변식 2 금지)과 다른 얘기.

use std::collections::HashMap;

use uuid::Uuid;

use super::tree;
use super::types::{LayoutNode, SplitDir, View, ViewMeta, ViewSnapshot};

/// 메인 창 label. 부팅 시 1탭으로 초기화되고 non-closable(불변식 4).
pub const MAIN_WINDOW_LABEL: &str = "main";

/// View 전역 식별자(창 간 이동·저장복원 후속 확장 위해 전역 UUID — ADR-0057).
pub type ViewId = Uuid;
/// Tauri 창 label(예: "main", "slot-popup-3").
pub type WindowLabel = String;

/// 레이아웃 연산 실패 사유. invalid id 는 no-op + 이 에러(부분변경 금지).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    #[error("view 없음: {0}")]
    ViewNotFound(Uuid),
    #[error("slot 없음: {0}")]
    SlotNotFound(Uuid),
    #[error("window 없음: {0}")]
    WindowNotFound(String),
    /// 메인 창은 닫을 수 없음(불변식 4 — hide only).
    #[error("메인 창은 닫을 수 없음")]
    MainNotClosable,
}

/// 한 창의 탭 목록 + 활성 탭(ADR-0057). `active` 는 항상 `tabs` 안(불변식 3).
#[derive(Debug, Clone)]
pub struct WindowTabs {
    /// 탭 순서(좌→우).
    pub tabs: Vec<ViewId>,
    /// 이 창의 활성 탭(불변식 3: 항상 `tabs` 안).
    pub active: ViewId,
}

/// 창별 탭 조회 결과(list_tabs 반환 / window:tabs-updated 페이로드 원천). ADR-0057.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowTabsSnapshot {
    pub label: WindowLabel,
    pub tabs: Vec<ViewMeta>,
    pub active: ViewId,
    pub version: u64,
}

/// 레이아웃 권위 상태(탭 소유 모델 — ADR-0057). invoke 스레드풀 동시접근 → LayoutState 가 Mutex 로 감싼다.
pub struct ViewManager {
    /// 전역 View 풀(id lookup).
    pub views: HashMap<ViewId, View>,
    /// View → 소유 창(★유니크 소유★ = View 당 정확히 1창). 캐시된 역인덱스(불변식 1·2). // ADR-0057
    pub view_owner: HashMap<ViewId, WindowLabel>,
    /// 창 → 탭 목록 + 활성 탭. `agent-tree` 는 여기 없음(모델 밖). // ADR-0057
    pub windows: HashMap<WindowLabel, WindowTabs>,
    /// 변경마다 +1(get_view race 용 — 팝업 pull↔listen 윈도). 0 부터 시작, 첫 변경에서 1.
    pub version: u64,
}

impl Default for ViewManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewManager {
    /// 부팅 초기 상태: main 창 1탭(빈 슬롯 View 1개 + 그게 active). agent-tree 는 windows 밖. // ADR-0057
    pub fn new() -> Self {
        let mut views = HashMap::new();
        let v0 = View {
            id: Uuid::new_v4(),
            name: "View 1".to_string(),
            layout: LayoutNode::new_empty_slot(),
            focused_slot_id: None,
        };
        let v0_id = v0.id;
        views.insert(v0_id, v0);

        let mut view_owner = HashMap::new();
        view_owner.insert(v0_id, MAIN_WINDOW_LABEL.to_string());

        let mut windows = HashMap::new();
        windows.insert(
            MAIN_WINDOW_LABEL.to_string(),
            WindowTabs {
                tabs: vec![v0_id],
                active: v0_id,
            },
        );

        let mut mgr = Self {
            views,
            view_owner,
            windows,
            version: 0,
        };
        // 첫 슬롯을 포커스로(빈 슬롯이라도 포커스 대상은 존재).
        if let Some(v) = mgr.views.get_mut(&v0_id) {
            Self::fixup_focus(v);
        }
        mgr
    }

    // ── 조회 ───────────────────────────────────────────────────────────────

    /// 창별 탭 목록 + 활성 + version(list_tabs / window:tabs-updated 원천). 없는 창 → Err.
    pub fn list_tabs(&self, label: &str) -> Result<WindowTabsSnapshot, LayoutError> {
        let wt = self
            .windows
            .get(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        // tabs 순서대로 메타를 만든다(유니크 소유라 tabs 가 곧 그 창 탭 목록 — 필터 불필요).
        let tabs: Vec<ViewMeta> = wt
            .tabs
            .iter()
            .filter_map(|vid| {
                self.views.get(vid).map(|v| ViewMeta {
                    id: v.id,
                    name: v.name.clone(),
                })
            })
            .collect();
        Ok(WindowTabsSnapshot {
            label: label.to_string(),
            tabs,
            active: wt.active,
            version: self.version,
        })
    }

    /// 창 label 목록(list_windows). agent-tree 등 모델 밖 창은 포함 안 함.
    pub fn list_windows(&self) -> Vec<WindowLabel> {
        self.windows.keys().cloned().collect()
    }

    /// view_id 의 스냅샷(get_view·layout:updated 페이로드). 없으면 Err.
    pub fn snapshot(&self, view_id: Uuid) -> Result<ViewSnapshot, LayoutError> {
        let v = self
            .views
            .get(&view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        Ok(ViewSnapshot {
            view_id: v.id,
            layout: v.layout.clone(),
            focused_slot_id: v.focused_slot_id,
            version: self.version,
        })
    }

    /// view 안 slot_id 슬롯에 배정된 agent_id(참조 문자열)를 반환. 빈 슬롯이면 Ok(None).
    /// ★슬롯 이동(move_slot_to_window)용★: 원본 슬롯의 agent 를 읽어 새 탭으로 옮길 때 쓴다(조회만).
    /// invalid view_id/slot_id → Err(no-op).
    pub fn slot_agent(&self, view_id: Uuid, slot_id: Uuid) -> Result<Option<String>, LayoutError> {
        let v = self
            .views
            .get(&view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        tree::find_slot(&v.layout, slot_id)
            .cloned()
            .ok_or(LayoutError::SlotNotFound(slot_id))
    }

    /// view_id 의 소속 창을 O(1) 파생(view_owner 역인덱스). view-id-키 command(assign/split/close_slot)
    /// 가 소속 창을 찾아 그 창에 이벤트를 쏠 때 쓴다. 없으면 None(고아 View — 정상 경로엔 없음).
    pub fn owner_of(&self, view_id: ViewId) -> Option<&WindowLabel> {
        self.view_owner.get(&view_id)
    }

    fn view_mut(&mut self, view_id: Uuid) -> Result<&mut View, LayoutError> {
        self.views
            .get_mut(&view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))
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

    // ── 내부 헬퍼(불변식 유지 — 쌍 갱신) ─────────────────────────────────────

    /// 새 빈-슬롯 View 를 만들어 삽입만 한다(소유/창 배정은 호출자). id 반환.
    fn make_view(&mut self, name: String) -> ViewId {
        let id = Uuid::new_v4();
        let first_slot = LayoutNode::new_empty_slot();
        let focus = tree::first_slot_id(&first_slot);
        self.views.insert(
            id,
            View {
                id,
                name,
                layout: first_slot,
                focused_slot_id: Some(focus),
            },
        );
        id
    }

    // ── mutation (각 메서드: 변경 후 version +1, focus 보정. emit 은 호출자) ─────────

    /// 창 `label` 에 새 빈-슬롯 탭 추가·활성화. 새 View id 반환. 없는 창 → Err.
    /// ★불변식 1 쌍 갱신★: `windows[L].tabs` push ↔ `view_owner[v]=L` 를 함께. // ADR-0057
    pub fn create_tab(&mut self, label: &str, name: Option<String>) -> Result<ViewId, LayoutError> {
        if !self.windows.contains_key(label) {
            return Err(LayoutError::WindowNotFound(label.to_string()));
        }
        let default_name = {
            let count = self.windows.get(label).map(|w| w.tabs.len()).unwrap_or(0);
            format!("View {}", count + 1)
        };
        let id = self.make_view(name.unwrap_or(default_name));
        // 쌍 갱신(불변식 1·2). // ADR-0057
        self.view_owner.insert(id, label.to_string());
        let wt = self.windows.get_mut(label).expect("존재 확인됨");
        wt.tabs.push(id);
        wt.active = id; // 새 탭 활성화(불변식 3).
        self.bump_version();
        Ok(id)
    }

    /// 새 빈 창(빈 탭 1개) 생성(create_window — D-6). label 은 호출자(command 레이어)가 발급.
    /// ★이미 존재하는 label 재사용 금지★(Tauri 창 label 재사용 에러 회피 — 호출자 카운터 단조).
    /// 새 창의 (유일) 탭 View id 반환.
    pub fn create_window(&mut self, label: &str) -> Result<ViewId, LayoutError> {
        // main 이나 기존 label 재생성 금지(부분 상태 방지).
        if self.windows.contains_key(label) {
            return Err(LayoutError::WindowNotFound(label.to_string()));
        }
        let id = self.make_view("View 1".to_string());
        self.view_owner.insert(id, label.to_string());
        self.windows.insert(
            label.to_string(),
            WindowTabs {
                tabs: vec![id],
                active: id,
            },
        );
        self.bump_version();
        Ok(id)
    }

    /// 창 `label` 의 활성 탭을 `view` 로 교체. 타 창 불변. keep-alive(ADR-0056)라 노출 집합 불변 —
    /// active 표시만 바뀐다. view 가 그 창 탭이 아니면 Err(no-op).
    pub fn switch_tab(&mut self, label: &str, view: ViewId) -> Result<(), LayoutError> {
        let wt = self
            .windows
            .get_mut(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        if !wt.tabs.contains(&view) {
            return Err(LayoutError::ViewNotFound(view));
        }
        wt.active = view; // 불변식 3 유지(tabs 안 확인함).
        self.bump_version();
        Ok(())
    }

    /// 창 `label` 의 탭 `view` 를 닫음(§5-2 상태기계, ADR-0057). 반환 = 이 close 로 창이 **닫혀야 하는지**
    /// (팝업 마지막 탭). command 레이어가 그때 close_window(OS) 를 실행한다.
    /// - 인접 탭 승계: active 를 닫으면 오른쪽 우선(없으면 왼쪽)으로 active 이동.
    /// - main 마지막 탭: 빈 탭 1개 강제(불변식 4).
    /// - 팝업 마지막 탭: 창 닫힘 신호 반환(command 가 close_window).
    /// view 가 그 창 탭이 아니면 Err(no-op).
    pub fn close_tab(&mut self, label: &str, view: ViewId) -> Result<CloseTabOutcome, LayoutError> {
        // 소속 검증(불변식 1·2).
        let wt = self
            .windows
            .get(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        let pos = wt
            .tabs
            .iter()
            .position(|v| *v == view)
            .ok_or(LayoutError::ViewNotFound(view))?;
        let was_active = wt.active == view;

        // View 1개 드롭(불변식 1 쌍 갱신). // ADR-0057
        self.views.remove(&view);
        self.view_owner.remove(&view);
        let wt = self.windows.get_mut(label).expect("존재 확인됨");
        wt.tabs.remove(pos);

        // active 승계(탭이 남아있을 때만). 오른쪽 우선(같은 pos), 없으면 왼쪽(pos-1). // ADR-0057
        if was_active && !wt.tabs.is_empty() {
            let new_idx = if pos < wt.tabs.len() { pos } else { pos - 1 };
            wt.active = wt.tabs[new_idx];
        }

        if wt.tabs.is_empty() {
            if label == MAIN_WINDOW_LABEL {
                // 메인 마지막 탭 → 빈 탭 1개 강제(불변식 4, main non-closable). // ADR-0057
                let id = self.make_view("View 1".to_string());
                self.view_owner.insert(id, MAIN_WINDOW_LABEL.to_string());
                let wt = self.windows.get_mut(label).expect("main 존재");
                wt.tabs.push(id);
                wt.active = id;
                self.bump_version();
                Ok(CloseTabOutcome::Stayed)
            } else {
                // 팝업 마지막 탭 → 창 통째 닫힘. windows 엔트리 제거(에이전트는 생존, D-3). // ADR-0057
                self.windows.remove(label);
                self.bump_version();
                Ok(CloseTabOutcome::WindowClosed)
            }
        } else {
            self.bump_version();
            Ok(CloseTabOutcome::Stayed)
        }
    }

    /// 창 `label` 을 통째로 닫음(모든 탭 View 드롭 + windows 엔트리 제거). 반환 = 드롭된 View id 들
    /// (command 레이어가 rebuild 후 Unsubscribe 델타에 반영). ★main 은 금지(불변식 4)★.
    /// 팝업 창 Destroyed 멀티탭 정리(§5-2/G1)의 코어 경로 — `tabs` 전부 순회 드롭. // ADR-0057
    pub fn close_window(&mut self, label: &str) -> Result<Vec<ViewId>, LayoutError> {
        if label == MAIN_WINDOW_LABEL {
            return Err(LayoutError::MainNotClosable); // 불변식 4. // ADR-0057
        }
        let wt = self
            .windows
            .remove(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        // 이 창의 모든 탭 View 를 드롭(불변식 1 쌍 갱신 — tabs 전부 순회). // ADR-0057
        let dropped = wt.tabs.clone();
        for vid in &wt.tabs {
            self.views.remove(vid);
            self.view_owner.remove(vid);
        }
        self.bump_version();
        Ok(dropped)
    }

    /// view 안 slot_id 슬롯을 분할. 새 슬롯에 focus. 새 슬롯 id 반환.
    /// view-id 전역 유니크라 시그니처 유지(소속 창은 view_owner 파생). invalid → Err(no-op).
    pub fn split_slot(
        &mut self,
        view_id: Uuid,
        slot_id: Uuid,
        dir: SplitDir,
    ) -> Result<Uuid, LayoutError> {
        let v = self.view_mut(view_id)?;
        match tree::split_in_tree(&mut v.layout, slot_id, dir) {
            Some(new_id) => {
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
    /// 같은 agent 가 다른 View 에도 배정될 수 있음(불변식 5 — 두 창 같은 에이전트). invalid → Err(no-op).
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

    // ── move_slot_to_window 2-phase 지원(§5-3, G4) ───────────────────────────

    /// ★phase A★: 소스 슬롯 agent 를 담은 임시 View 를 만든다(아직 **어느 창 tabs 에도 안 넣음** — orphan
    /// 방지, phase C 에서 삽입). 새 View id 반환. 소스 슬롯은 안 건드림(phase C 에서 close).
    /// 빈 슬롯이면 Err(pop-out 대상 없음).
    pub fn prepare_detached_view(
        &mut self,
        src_view: ViewId,
        src_slot: Uuid,
        name: String,
    ) -> Result<ViewId, LayoutError> {
        let agent_id = self
            .slot_agent(src_view, src_slot)?
            .ok_or(LayoutError::SlotNotFound(src_slot))?;
        let id = self.make_view(name);
        // 새 View 의 (유일) 슬롯에 원본 agent 배정(불변식 5 — 다중 참조 허용).
        let slot = {
            let v = self.views.get(&id).expect("방금 만든 View");
            tree::first_slot_id(&v.layout)
        };
        // view_owner 미배정(아직 어느 창에도 안 속함 — phase C 에서 삽입). 그래서 assign 은 tree 직접.
        if let Some(v) = self.views.get_mut(&id) {
            let _ = tree::assign_in_tree(&mut v.layout, slot, Some(agent_id));
        }
        self.bump_version();
        Ok(id)
    }

    /// ★phase A 롤백★: prepare_detached_view 로 만든 임시 View 를 제거(창 삽입 전이라 tabs 갱신 불필요).
    pub fn drop_detached_view(&mut self, view: ViewId) {
        self.views.remove(&view);
        self.view_owner.remove(&view); // 안전(정상 경로엔 owner 없음).
        self.bump_version();
    }

    /// ★phase C — 기존 창에 삽입★: 임시 View 를 `to_window` 의 새 탭으로 삽입·활성화(create_tab 상당).
    /// ★재검증(G4)★: `to_window` 가 여전히 존재할 때만 삽입 — 부재면 Err(호출자가 롤백). 삽입 후
    /// `view_owner[view]==to_window`(불변식 1·2 쌍 갱신). // ADR-0057
    pub fn insert_tab_into(&mut self, to_window: &str, view: ViewId) -> Result<(), LayoutError> {
        // to_window 재검증(phase B 언락 중 소멸했을 수 있음).
        if !self.windows.contains_key(to_window) {
            return Err(LayoutError::WindowNotFound(to_window.to_string()));
        }
        if !self.views.contains_key(&view) {
            return Err(LayoutError::ViewNotFound(view));
        }
        // 쌍 갱신(불변식 1·2). // ADR-0057
        self.view_owner.insert(view, to_window.to_string());
        let wt = self.windows.get_mut(to_window).expect("존재 확인됨");
        wt.tabs.push(view);
        wt.active = view;
        self.bump_version();
        Ok(())
    }

    /// ★phase C — 새 창 생성 + 임시 View 를 그 창 첫 탭으로★. label 은 호출자 발급(단조 카운터).
    /// 새 창 windows 엔트리 생성 + view_owner 쌍 갱신. label 재사용이면 Err.
    pub fn attach_view_as_new_window(
        &mut self,
        label: &str,
        view: ViewId,
    ) -> Result<(), LayoutError> {
        if self.windows.contains_key(label) {
            return Err(LayoutError::WindowNotFound(label.to_string()));
        }
        if !self.views.contains_key(&view) {
            return Err(LayoutError::ViewNotFound(view));
        }
        self.view_owner.insert(view, label.to_string());
        self.windows.insert(
            label.to_string(),
            WindowTabs {
                tabs: vec![view],
                active: view,
            },
        );
        self.bump_version();
        Ok(())
    }
}

/// close_tab 결과 — 창이 살아남았나(빈 탭 강제/인접 승계) vs 팝업 마지막 탭이라 창을 닫아야 하나.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseTabOutcome {
    /// 창은 유지(main 빈 탭 강제 or 인접 탭 승계).
    Stayed,
    /// 팝업 마지막 탭 → command 레이어가 OS 창을 닫아야 함(에이전트는 생존).
    WindowClosed,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_slot_of(mgr: &ViewManager, view_id: Uuid) -> Uuid {
        let v = mgr.views.get(&view_id).unwrap();
        tree::first_slot_id(&v.layout)
    }

    /// main 창 활성 탭 id.
    fn main_active(mgr: &ViewManager) -> ViewId {
        mgr.windows.get(MAIN_WINDOW_LABEL).unwrap().active
    }

    /// ★불변식 1·2 전역 검사(모든 테스트가 끝에 호출해 상태 정합 확인)★. // ADR-0057
    fn assert_invariants(mgr: &ViewManager) {
        // 불변식 2: 모든 View 는 view_owner 에 정확히 1개.
        for vid in mgr.views.keys() {
            assert!(
                mgr.view_owner.contains_key(vid),
                "불변식2: View {vid} 에 소유 창 없음"
            );
        }
        // view_owner 는 실재 View 만 가리킨다(고아 owner 없음).
        for (vid, label) in &mgr.view_owner {
            assert!(
                mgr.views.contains_key(vid),
                "view_owner 가 없는 View 가리킴"
            );
            assert!(
                mgr.windows.contains_key(label),
                "view_owner 가 없는 창 {label} 가리킴"
            );
        }
        // 불변식 1: view_owner[v]==L ⟺ windows[L].tabs ∋ v.
        for (label, wt) in &mgr.windows {
            for vid in &wt.tabs {
                assert_eq!(
                    mgr.view_owner.get(vid).map(|s| s.as_str()),
                    Some(label.as_str()),
                    "불변식1: windows[{label}].tabs 의 {vid} 소유 불일치"
                );
            }
            // 불변식 3: active ∈ tabs.
            assert!(
                wt.tabs.contains(&wt.active),
                "불변식3: windows[{label}].active 가 tabs 밖"
            );
            assert!(!wt.tabs.is_empty(), "빈 창은 존재 금지");
        }
        // 역방향 불변식 1: 모든 view_owner 엔트리는 그 창 tabs 에 있다.
        for (vid, label) in &mgr.view_owner {
            let wt = mgr.windows.get(label).expect("owner 창 존재");
            assert!(
                wt.tabs.contains(vid),
                "불변식1 역: view_owner[{vid}]={label} 인데 tabs 에 없음"
            );
        }
        // 불변식 4: main 최소 1탭.
        assert!(
            mgr.windows
                .get(MAIN_WINDOW_LABEL)
                .map(|w| !w.tabs.is_empty())
                .unwrap_or(false),
            "불변식4: main 최소 1탭"
        );
    }

    #[test]
    fn new_has_main_with_one_tab() {
        let mgr = ViewManager::new();
        assert_eq!(mgr.views.len(), 1);
        let wt = mgr.windows.get(MAIN_WINDOW_LABEL).unwrap();
        assert_eq!(wt.tabs.len(), 1);
        assert_eq!(wt.active, wt.tabs[0]);
        assert_eq!(mgr.view_owner.get(&wt.tabs[0]).unwrap(), MAIN_WINDOW_LABEL);
        assert_eq!(mgr.version, 0);
        // agent-tree 는 windows 밖.
        assert!(!mgr.windows.contains_key("agent-tree"));
        assert_invariants(&mgr);
    }

    #[test]
    fn create_tab_appends_and_activates_and_bumps_version() {
        let mut mgr = ViewManager::new();
        let v0 = mgr.version;
        let id = mgr
            .create_tab(MAIN_WINDOW_LABEL, Some("Custom".into()))
            .unwrap();
        let wt = mgr.windows.get(MAIN_WINDOW_LABEL).unwrap();
        assert_eq!(wt.tabs.len(), 2);
        assert_eq!(wt.active, id, "새 탭이 active");
        assert_eq!(mgr.views.get(&id).unwrap().name, "Custom");
        assert_eq!(mgr.view_owner.get(&id).unwrap(), MAIN_WINDOW_LABEL);
        assert_eq!(mgr.version, v0 + 1);
        assert_invariants(&mgr);
    }

    #[test]
    fn create_tab_unknown_window_is_err() {
        let mut mgr = ViewManager::new();
        let err = mgr.create_tab("no-such", None).unwrap_err();
        assert!(matches!(err, LayoutError::WindowNotFound(_)));
        assert_invariants(&mgr);
    }

    #[test]
    fn switch_tab_changes_active_only_that_window() {
        let mut mgr = ViewManager::new();
        let main0 = main_active(&mgr);
        let t1 = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        // 다른 창(팝업) 하나 만들어 불변 확인.
        let pv = mgr.create_window("slot-popup-1").unwrap();
        mgr.switch_tab(MAIN_WINDOW_LABEL, main0).unwrap();
        assert_eq!(main_active(&mgr), main0);
        // 팝업 active 불변.
        assert_eq!(mgr.windows.get("slot-popup-1").unwrap().active, pv);
        mgr.switch_tab(MAIN_WINDOW_LABEL, t1).unwrap();
        assert_eq!(main_active(&mgr), t1);
        assert_invariants(&mgr);
    }

    #[test]
    fn switch_tab_invalid_view_is_err_noop() {
        let mut mgr = ViewManager::new();
        let ver = mgr.version;
        assert!(mgr.switch_tab(MAIN_WINDOW_LABEL, Uuid::new_v4()).is_err());
        assert_eq!(mgr.version, ver);
    }

    #[test]
    fn create_window_makes_new_window_with_one_tab() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_window("slot-popup-1").unwrap();
        let wt = mgr.windows.get("slot-popup-1").unwrap();
        assert_eq!(wt.tabs, vec![v]);
        assert_eq!(wt.active, v);
        assert_eq!(mgr.view_owner.get(&v).unwrap(), "slot-popup-1");
        assert_invariants(&mgr);
    }

    #[test]
    fn create_window_duplicate_label_is_err() {
        let mut mgr = ViewManager::new();
        mgr.create_window("slot-popup-1").unwrap();
        assert!(mgr.create_window("slot-popup-1").is_err());
        // main 도 재생성 금지.
        assert!(mgr.create_window("main").is_err());
    }

    // ── close_tab 상태기계(§5-2) ─────────────────────────────────────────────

    #[test]
    fn close_active_tab_succeeds_right_neighbor() {
        // main 탭 3개 [a,b,c], active=b. b 닫으면 오른쪽(c) 승계.
        let mut mgr = ViewManager::new();
        let a = main_active(&mgr);
        let b = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let c = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        mgr.switch_tab(MAIN_WINDOW_LABEL, b).unwrap();
        let out = mgr.close_tab(MAIN_WINDOW_LABEL, b).unwrap();
        assert_eq!(out, CloseTabOutcome::Stayed);
        assert_eq!(main_active(&mgr), c, "오른쪽 탭 승계");
        let wt = mgr.windows.get(MAIN_WINDOW_LABEL).unwrap();
        assert_eq!(wt.tabs, vec![a, c]);
        assert!(!mgr.views.contains_key(&b), "닫은 View 드롭");
        assert!(!mgr.view_owner.contains_key(&b));
        assert_invariants(&mgr);
    }

    #[test]
    fn close_active_last_tab_succeeds_left_neighbor() {
        // main [a,b], active=b(마지막). b 닫으면 왼쪽(a) 승계.
        let mut mgr = ViewManager::new();
        let a = main_active(&mgr);
        let b = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap(); // active=b
        mgr.close_tab(MAIN_WINDOW_LABEL, b).unwrap();
        assert_eq!(main_active(&mgr), a, "왼쪽 탭 승계(오른쪽 없음)");
        assert_invariants(&mgr);
    }

    #[test]
    fn close_non_active_tab_keeps_active() {
        let mut mgr = ViewManager::new();
        let a = main_active(&mgr);
        let b = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap(); // active=b
        mgr.close_tab(MAIN_WINDOW_LABEL, a).unwrap(); // a 는 비활성
        assert_eq!(main_active(&mgr), b, "비활성 닫아도 active 유지");
        assert_invariants(&mgr);
    }

    #[test]
    fn close_main_last_tab_forces_empty_tab() {
        // main 마지막 탭 닫으면 빈 탭 1개 강제(불변식 4). 창은 유지.
        let mut mgr = ViewManager::new();
        let v0 = main_active(&mgr);
        let out = mgr.close_tab(MAIN_WINDOW_LABEL, v0).unwrap();
        assert_eq!(out, CloseTabOutcome::Stayed, "main 은 창 안 닫힘");
        let wt = mgr.windows.get(MAIN_WINDOW_LABEL).unwrap();
        assert_eq!(wt.tabs.len(), 1, "빈 탭 1개 강제");
        assert_ne!(wt.tabs[0], v0, "새 빈 View id");
        let new_id = wt.tabs[0];
        assert!(matches!(
            mgr.views.get(&new_id).unwrap().layout,
            LayoutNode::Slot { agent_id: None, .. }
        ));
        assert_eq!(wt.active, new_id);
        assert_invariants(&mgr);
    }

    #[test]
    fn close_popup_last_tab_closes_window() {
        // 팝업 마지막 탭 닫으면 창 통째 닫힘(WindowClosed). windows 엔트리·View 드롭.
        let mut mgr = ViewManager::new();
        let pv = mgr.create_window("slot-popup-1").unwrap();
        let out = mgr.close_tab("slot-popup-1", pv).unwrap();
        assert_eq!(out, CloseTabOutcome::WindowClosed);
        assert!(!mgr.windows.contains_key("slot-popup-1"), "창 제거");
        assert!(!mgr.views.contains_key(&pv), "탭 View 드롭");
        assert!(!mgr.view_owner.contains_key(&pv));
        assert_invariants(&mgr);
    }

    #[test]
    fn close_popup_non_last_tab_stays() {
        // 팝업 탭 2개 → 하나 닫아도 창 유지(WindowClosed 아님).
        let mut mgr = ViewManager::new();
        let p0 = mgr.create_window("slot-popup-1").unwrap();
        let p1 = mgr.create_tab("slot-popup-1", None).unwrap();
        let out = mgr.close_tab("slot-popup-1", p1).unwrap();
        assert_eq!(out, CloseTabOutcome::Stayed);
        assert!(mgr.windows.contains_key("slot-popup-1"));
        assert_eq!(mgr.windows.get("slot-popup-1").unwrap().tabs, vec![p0]);
        assert_invariants(&mgr);
    }

    #[test]
    fn close_tab_invalid_view_is_err_noop() {
        let mut mgr = ViewManager::new();
        let ver = mgr.version;
        let n = mgr.views.len();
        assert!(mgr.close_tab(MAIN_WINDOW_LABEL, Uuid::new_v4()).is_err());
        assert_eq!(mgr.version, ver);
        assert_eq!(mgr.views.len(), n);
        assert_invariants(&mgr);
    }

    #[test]
    fn close_tab_unknown_window_is_err() {
        let mut mgr = ViewManager::new();
        assert!(mgr.close_tab("no-such", Uuid::new_v4()).is_err());
    }

    // ── close_window(§5-2/G1 멀티탭 정리) ────────────────────────────────────

    #[test]
    fn close_window_main_is_rejected() {
        // ★불변식 4★: close_window("main") 은 거부(no-op).
        let mut mgr = ViewManager::new();
        let err = mgr.close_window(MAIN_WINDOW_LABEL).unwrap_err();
        assert!(matches!(err, LayoutError::MainNotClosable));
        assert_invariants(&mgr);
    }

    #[test]
    fn close_window_multitab_drops_all_views() {
        // ★G1: 멀티탭 팝업 강제 정리★ — tabs 전부 순회 드롭, 잔류 0.
        let mut mgr = ViewManager::new();
        let p0 = mgr.create_window("slot-popup-1").unwrap();
        let p1 = mgr.create_tab("slot-popup-1", None).unwrap();
        let p2 = mgr.create_tab("slot-popup-1", None).unwrap();
        let dropped = mgr.close_window("slot-popup-1").unwrap();
        // 세 탭 전부 드롭 반환(순서 무관 — 집합 비교).
        assert_eq!(dropped.len(), 3);
        for v in [p0, p1, p2] {
            assert!(dropped.contains(&v));
            assert!(!mgr.views.contains_key(&v), "View 잔류 0");
            assert!(!mgr.view_owner.contains_key(&v), "view_owner 잔류 0");
        }
        assert!(!mgr.windows.contains_key("slot-popup-1"), "창 엔트리 제거");
        assert_invariants(&mgr);
    }

    #[test]
    fn close_window_unknown_is_err() {
        let mut mgr = ViewManager::new();
        assert!(mgr.close_window("no-such").is_err());
    }

    // ── split/close_slot/assign (view-id 키, 소속 창 파생) ────────────────────

    #[test]
    fn split_slot_creates_new_slot_and_focuses_it() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let slot = first_slot_of(&mgr, view_id);
        let new_id = mgr.split_slot(view_id, slot, SplitDir::Horizontal).unwrap();
        let v = mgr.views.get(&view_id).unwrap();
        assert!(matches!(v.layout, LayoutNode::Split { .. }));
        assert_eq!(v.focused_slot_id, Some(new_id));
        assert_invariants(&mgr);
    }

    #[test]
    fn split_invalid_view_is_err() {
        let mut mgr = ViewManager::new();
        assert!(matches!(
            mgr.split_slot(Uuid::new_v4(), Uuid::new_v4(), SplitDir::Horizontal)
                .unwrap_err(),
            LayoutError::ViewNotFound(_)
        ));
    }

    #[test]
    fn split_invalid_slot_is_err_noop() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let before = mgr.views.get(&view_id).unwrap().layout.clone();
        let ver = mgr.version;
        assert!(matches!(
            mgr.split_slot(view_id, Uuid::new_v4(), SplitDir::Vertical)
                .unwrap_err(),
            LayoutError::SlotNotFound(_)
        ));
        assert_eq!(mgr.views.get(&view_id).unwrap().layout, before);
        assert_eq!(mgr.version, ver);
    }

    #[test]
    fn close_slot_focus_fallback_to_first() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let slot = first_slot_of(&mgr, view_id);
        let new_id = mgr.split_slot(view_id, slot, SplitDir::Horizontal).unwrap();
        mgr.close_slot(view_id, new_id).unwrap();
        let v = mgr.views.get(&view_id).unwrap();
        assert_eq!(v.focused_slot_id, Some(slot));
        assert_invariants(&mgr);
    }

    #[test]
    fn close_root_slot_keeps_view_empty() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let slot = first_slot_of(&mgr, view_id);
        mgr.assign_agent(view_id, slot, "agent-x".into()).unwrap();
        mgr.close_slot(view_id, slot).unwrap();
        let v = mgr.views.get(&view_id).unwrap();
        assert!(matches!(v.layout, LayoutNode::Slot { agent_id: None, .. }));
        assert!(v.focused_slot_id.is_some());
        assert_invariants(&mgr);
    }

    #[test]
    fn close_slot_invalid_is_err_noop() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let ver = mgr.version;
        assert!(mgr.close_slot(view_id, Uuid::new_v4()).is_err());
        assert_eq!(mgr.version, ver);
    }

    #[test]
    fn assign_agent_sets_ref() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let slot = first_slot_of(&mgr, view_id);
        mgr.assign_agent(view_id, slot, "agent-42".into()).unwrap();
        let v = mgr.views.get(&view_id).unwrap();
        assert_eq!(
            tree::find_slot(&v.layout, slot).unwrap().as_deref(),
            Some("agent-42")
        );
        assert_invariants(&mgr);
    }

    #[test]
    fn assign_same_agent_to_two_views_is_allowed() {
        // ★불변식 5★: 같은 agent 를 서로 다른 두 View 에 배정 가능(두 창 같은 에이전트).
        let mut mgr = ViewManager::new();
        let v1 = main_active(&mgr);
        let s1 = first_slot_of(&mgr, v1);
        mgr.assign_agent(v1, s1, "shared".into()).unwrap();
        let v2 = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let s2 = first_slot_of(&mgr, v2);
        // 두 번째 배정도 성공(dedup/거부 없음).
        mgr.assign_agent(v2, s2, "shared".into()).unwrap();
        assert_eq!(mgr.slot_agent(v1, s1).unwrap().as_deref(), Some("shared"));
        assert_eq!(mgr.slot_agent(v2, s2).unwrap().as_deref(), Some("shared"));
        assert_invariants(&mgr);
    }

    #[test]
    fn assign_agent_invalid_view_is_err() {
        let mut mgr = ViewManager::new();
        assert!(mgr
            .assign_agent(Uuid::new_v4(), Uuid::new_v4(), "x".into())
            .is_err());
    }

    #[test]
    fn owner_of_derives_window_o1() {
        let mut mgr = ViewManager::new();
        let main = main_active(&mgr);
        assert_eq!(mgr.owner_of(main).map(|s| s.as_str()), Some("main"));
        let pv = mgr.create_window("slot-popup-1").unwrap();
        assert_eq!(mgr.owner_of(pv).map(|s| s.as_str()), Some("slot-popup-1"));
    }

    // ── list_tabs / list_windows ─────────────────────────────────────────────

    #[test]
    fn list_tabs_returns_tabs_active_version() {
        let mut mgr = ViewManager::new();
        let t1 = mgr
            .create_tab(MAIN_WINDOW_LABEL, Some("Second".into()))
            .unwrap();
        let snap = mgr.list_tabs(MAIN_WINDOW_LABEL).unwrap();
        assert_eq!(snap.tabs.len(), 2);
        assert_eq!(snap.active, t1);
        assert_eq!(snap.version, mgr.version);
        assert_eq!(snap.tabs[1].name, "Second");
    }

    #[test]
    fn list_tabs_unknown_window_is_err() {
        let mgr = ViewManager::new();
        assert!(mgr.list_tabs("no-such").is_err());
    }

    #[test]
    fn list_windows_lists_main_only_initially() {
        let mgr = ViewManager::new();
        let ws = mgr.list_windows();
        assert_eq!(ws, vec![MAIN_WINDOW_LABEL.to_string()]);
    }

    // ── snapshot ─────────────────────────────────────────────────────────────

    #[test]
    fn snapshot_returns_version_and_layout() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
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

    // ── move_slot_to_window 2-phase 지원(§5-3, G4) ──────────────────────────

    #[test]
    fn prepare_detached_view_moves_agent_without_window() {
        // phase A: 임시 View 생성 — agent 담김, 아직 어느 창에도 안 속함(view_owner 미배정).
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.assign_agent(src, slot, "moving".into()).unwrap();
        let tmp = mgr
            .prepare_detached_view(src, slot, "Popup".into())
            .unwrap();
        // 임시 View 슬롯에 agent 담김.
        let tslot = first_slot_of(&mgr, tmp);
        assert_eq!(
            mgr.slot_agent(tmp, tslot).unwrap().as_deref(),
            Some("moving")
        );
        // 아직 창에 안 속함(orphan 방지 — phase C 에서 삽입).
        assert!(
            mgr.view_owner.get(&tmp).is_none(),
            "phase A 는 view_owner 미배정"
        );
        // 소스는 아직 그대로(phase C 에서 close).
        assert_eq!(
            mgr.slot_agent(src, slot).unwrap().as_deref(),
            Some("moving")
        );
    }

    #[test]
    fn prepare_detached_view_empty_slot_is_err() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        assert!(mgr.prepare_detached_view(src, slot, "P".into()).is_err());
    }

    #[test]
    fn insert_tab_into_existing_window_phase_c() {
        // phase C(기존 창): 임시 View 를 to_window 새 탭으로 삽입·활성화.
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.assign_agent(src, slot, "moving".into()).unwrap();
        // 기존 팝업 창.
        let existing = mgr.create_window("slot-popup-1").unwrap();
        let tmp = mgr
            .prepare_detached_view(src, slot, "Popup".into())
            .unwrap();
        mgr.insert_tab_into("slot-popup-1", tmp).unwrap();
        let wt = mgr.windows.get("slot-popup-1").unwrap();
        assert_eq!(wt.tabs, vec![existing, tmp]);
        assert_eq!(wt.active, tmp, "삽입 탭 활성화");
        assert_eq!(mgr.view_owner.get(&tmp).unwrap(), "slot-popup-1");
        assert_invariants(&mgr);
    }

    #[test]
    fn insert_tab_into_vanished_window_is_err_for_rollback() {
        // ★G4 재검증★: to_window 가 phase B 중 소멸했으면 삽입 안 하고 Err(호출자 롤백).
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.assign_agent(src, slot, "moving".into()).unwrap();
        let tmp = mgr
            .prepare_detached_view(src, slot, "Popup".into())
            .unwrap();
        // to_window 가 존재 안 함 → Err.
        let err = mgr.insert_tab_into("gone", tmp).unwrap_err();
        assert!(matches!(err, LayoutError::WindowNotFound(_)));
        // tmp 는 여전히 orphan(호출자가 drop_detached_view 로 롤백).
        assert!(mgr.view_owner.get(&tmp).is_none());
    }

    #[test]
    fn insert_same_agent_into_window_that_has_it_is_allowed() {
        // ★불변식 5 / G4 dedup 금지★: 같은 agent 를 이미 보고 있는 창으로 옮겨도 정상.
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.assign_agent(src, slot, "shared".into()).unwrap();
        // 팝업 창이 이미 같은 agent 를 봄.
        let existing = mgr.create_window("slot-popup-1").unwrap();
        let eslot = first_slot_of(&mgr, existing);
        mgr.assign_agent(existing, eslot, "shared".into()).unwrap();
        // src 슬롯의 shared 를 그 창으로 옮김 → 두 탭이 같은 agent(dedup 없이 허용).
        let tmp = mgr
            .prepare_detached_view(src, slot, "Popup".into())
            .unwrap();
        mgr.insert_tab_into("slot-popup-1", tmp).unwrap();
        assert_eq!(mgr.windows.get("slot-popup-1").unwrap().tabs.len(), 2);
        assert_invariants(&mgr);
    }

    #[test]
    fn attach_view_as_new_window_phase_c() {
        // phase C(새 창): 임시 View 를 새 창 첫 탭으로.
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.assign_agent(src, slot, "moving".into()).unwrap();
        let tmp = mgr
            .prepare_detached_view(src, slot, "Popup".into())
            .unwrap();
        mgr.attach_view_as_new_window("slot-popup-1", tmp).unwrap();
        let wt = mgr.windows.get("slot-popup-1").unwrap();
        assert_eq!(wt.tabs, vec![tmp]);
        assert_eq!(wt.active, tmp);
        assert_eq!(mgr.view_owner.get(&tmp).unwrap(), "slot-popup-1");
        assert_invariants(&mgr);
    }

    #[test]
    fn drop_detached_view_rolls_back_phase_a() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.assign_agent(src, slot, "moving".into()).unwrap();
        let tmp = mgr
            .prepare_detached_view(src, slot, "Popup".into())
            .unwrap();
        mgr.drop_detached_view(tmp);
        assert!(!mgr.views.contains_key(&tmp), "임시 View 제거");
        assert_invariants(&mgr);
    }
}
