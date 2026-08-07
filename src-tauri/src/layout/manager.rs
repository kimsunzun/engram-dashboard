//! ViewManager — 레이아웃 권위 상태(ADR-0035 부분개정 · ADR-0057 탭 소유 모델). LayoutState 가
//! `Arc<Mutex<ViewManager>>` 로 소유.
//!
//! ★Tauri 의존 0★: 이 타입은 락·emit 을 모른다. 락 취득/해제·emit 은 command 레이어
//! (`commands/layout.rs`)가 한다. 그래서 mutation 메서드는 변경 결과
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
//!    예외: `prepare_detached_view` 가 만든 tmp_view 는 phase C 삽입/롤백 전까지 owner 가 없다.
//! 3. **활성 소속:** `windows[L].active ∈ windows[L].tabs` 항상.
//! 4. **메인 최소 1탭 + non-closable:** `windows["main"].tabs.len() >= 1` 불변. `close_window("main")`
//!    은 금지(`MainNotClosable` 로 거부) — 마지막 탭 close 는 빈 탭 강제로만 떨어진다.
//! 5. **에이전트 참조 다중 허용:** 같은 `agent_id` 가 서로 다른 두 View 슬롯에 배정 가능(두 창이 같은
//!    에이전트 봄, 진도 독립·ADR-0046). "한 View 두 창"(불변식 2 금지)과 다른 얘기.

use std::collections::HashMap;

use uuid::Uuid;

use super::tree;
use super::types::{LayoutNode, SlotContent, SplitDir, View, ViewMeta, ViewSnapshot};

pub const MAIN_WINDOW_LABEL: &str = "main";

// View 전역 식별자(창 간 이동·저장복원 후속 확장 위해 전역 UUID — ADR-0057).
pub type ViewId = Uuid;
// Tauri 창 label(예: "main", "slot-popup-3").
pub type WindowLabel = String;

// invalid id 는 no-op + 이 에러(부분변경 금지).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    #[error("view 없음: {0}")]
    ViewNotFound(Uuid),
    #[error("slot 없음: {0}")]
    SlotNotFound(Uuid),
    #[error("window 없음: {0}")]
    WindowNotFound(String),
    #[error("메인 창은 닫을 수 없음")]
    MainNotClosable,
}

#[derive(Debug, Clone)]
pub struct WindowTabs {
    // 탭 순서(좌→우).
    pub tabs: Vec<ViewId>,
    pub active: ViewId,
}

// 창별 탭 조회 결과(list_tabs 반환 / window:tabs-updated 페이로드 원천). ADR-0057.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowTabsSnapshot {
    pub label: WindowLabel,
    pub tabs: Vec<ViewMeta>,
    pub active: ViewId,
    pub version: u64,
}

// 레이아웃 권위 상태(탭 소유 모델 — ADR-0057). invoke 스레드풀 동시접근 → LayoutState 가 Mutex 로 감싼다.
pub struct ViewManager {
    pub views: HashMap<ViewId, View>,
    pub view_owner: HashMap<ViewId, WindowLabel>,
    pub windows: HashMap<WindowLabel, WindowTabs>,
    // 변경마다 +1(get_view race 용 — 팝업 pull↔listen 윈도). 0 부터 시작, 첫 변경에서 1.
    pub version: u64,
}

impl Default for ViewManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewManager {
    // ★부팅 기본 = 슬롯화된 트리(ADR-0063)★: 옛 고정 좌측 사이드패널(AppLayout Sidebar)이 하던 "트리 좌측
    // 상시 노출"을 슬롯으로 재현한다. 좌측을 작게(ratio 0.2) 두어 사이드패널 UX(좁은 트리 + 넓은 작업
    // 영역)를 무손실 대체한다. ★main 첫 뷰만★ — create_tab 로 만드는 새 탭은 종전대로 단일 빈 슬롯이다
    // (트리를 모든 탭에 강제하지 않음).
    pub fn new() -> Self {
        let mut views = HashMap::new();
        let v0 = View {
            id: Uuid::new_v4(),
            name: "View 1".to_string(),
            layout: Self::default_main_layout(),
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
        if let Some(v) = mgr.views.get_mut(&v0_id) {
            Self::fixup_focus(v);
        }
        mgr
    }

    // ── 조회 ───────────────────────────────────────────────────────────────

    pub fn list_tabs(&self, label: &str) -> Result<WindowTabsSnapshot, LayoutError> {
        let wt = self
            .windows
            .get(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        // 유니크 소유라 tabs 가 곧 그 창 탭 목록 — 필터 불필요.
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

    pub fn list_windows(&self) -> Vec<WindowLabel> {
        self.windows.keys().cloned().collect()
    }

    pub fn snapshot(&self, view_id: Uuid) -> Result<ViewSnapshot, LayoutError> {
        let v = self
            .views
            .get(&view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        Ok(ViewSnapshot {
            view_id: v.id,
            layout: v.layout.clone(),
            focused_slot_id: v.focused_slot_id,
            slot_spatial: super::spatial::compute_spatial(&v.layout),
            version: self.version,
        })
    }

    pub fn slot_agent(&self, view_id: Uuid, slot_id: Uuid) -> Result<Option<String>, LayoutError> {
        let v = self
            .views
            .get(&view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        // ADR-0060: 슬롯 부재(None)와 빈 슬롯(Some(Empty)→None)을 구분 — 부재만 SlotNotFound.
        tree::find_slot(&v.layout, slot_id)
            .map(|content| content.agent_id().map(str::to_string))
            .ok_or(LayoutError::SlotNotFound(slot_id))
    }

    // 없으면 None(고아 View — 정상 경로엔 없음).
    pub fn owner_of(&self, view_id: ViewId) -> Option<&WindowLabel> {
        self.view_owner.get(&view_id)
    }

    fn view_mut(&mut self, view_id: Uuid) -> Result<&mut View, LayoutError> {
        self.views
            .get_mut(&view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))
    }

    // focus fallback — focused_slot_id 가 가리키던 슬롯이 사라지면 트리 첫 슬롯으로(항상 ≥1 슬롯).
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

    // ★직접 Split 구성★: split_in_tree 는 ratio 0.5 고정 + 새 슬롯이 항상 Empty 라 여기 요구(좌=AgentList,
    //   ratio 0.2)를 못 맞춘다 → LayoutNode::Split 을 직접 짓는다(types.rs Split 노드 형태 그대로).
    fn default_main_layout() -> LayoutNode {
        LayoutNode::Split {
            dir: SplitDir::Horizontal,
            ratio: 0.2, // 좌측(AgentList)을 작게 — 사이드패널 폭 재현.
            a: Box::new(LayoutNode::Slot {
                id: Uuid::new_v4(),
                content: SlotContent::AgentList,
            }),
            b: Box::new(LayoutNode::new_empty_slot()),
        }
    }

    // ── 내부 헬퍼 ───────────────────────────────────────────────────────────

    // 소유/창 배정은 호출자.
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

    // ── mutation ────────────────────────────────────────────────────────────

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
        wt.active = id;
        self.bump_version();
        Ok(id)
    }

    // label 은 호출자(command 레이어)가 발급(D-6).
    pub fn create_window(&mut self, label: &str) -> Result<ViewId, LayoutError> {
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

    // keep-alive(ADR-0056)라 노출 집합 불변 — active 표시만 바뀐다.
    pub fn switch_tab(&mut self, label: &str, view: ViewId) -> Result<(), LayoutError> {
        let wt = self
            .windows
            .get_mut(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        if !wt.tabs.contains(&view) {
            return Err(LayoutError::ViewNotFound(view));
        }
        wt.active = view;
        self.bump_version();
        Ok(())
    }

    // 창 `label` 의 탭 `view` 를 닫음(§5-2 상태기계, ADR-0057). 반환 = 이 close 로 창이 **닫혀야 하는지**.
    pub fn close_tab(&mut self, label: &str, view: ViewId) -> Result<CloseTabOutcome, LayoutError> {
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
                let id = self.make_view("View 1".to_string());
                self.view_owner.insert(id, MAIN_WINDOW_LABEL.to_string());
                let wt = self.windows.get_mut(label).expect("main 존재");
                wt.tabs.push(id);
                wt.active = id;
                self.bump_version();
                Ok(CloseTabOutcome::Stayed)
            } else {
                self.windows.remove(label);
                self.bump_version();
                Ok(CloseTabOutcome::WindowClosed)
            }
        } else {
            self.bump_version();
            Ok(CloseTabOutcome::Stayed)
        }
    }

    // 창 `label` 을 통째로 닫음(모든 탭 View 드롭 + windows 엔트리 제거). 반환 = 드롭된 View id 들
    // (command 레이어가 rebuild 후 Unsubscribe 델타에 반영).
    // 팝업 창 Destroyed 멀티탭 정리(§5-2/G1)의 코어 경로. // ADR-0057
    pub fn close_window(&mut self, label: &str) -> Result<Vec<ViewId>, LayoutError> {
        if label == MAIN_WINDOW_LABEL {
            return Err(LayoutError::MainNotClosable);
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

    // view-id 전역 유니크라 시그니처 유지(소속 창은 view_owner 파생).
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

    // view 안 slot_id 슬롯을 포커스로 지정(click-to-focus — ADR-0066 결정 1). ★그 슬롯이 이 View 트리에
    // 실재할 때만★ `focused_slot_id` 를 갱신하고 version 을 올린다 — 부재면 no-op + SlotNotFound(부분변경
    // 금지).
    //
    // ★백엔드 권위(ADR-0035/0066)★: focused_slot_id 는 백엔드가 소유하고, 프론트는 emit(layout:updated)로만
    // 반영한다(낙관 갱신 금지 — command 레이어가 스냅샷 emit). auto-focus-on-split(split_slot)은 그대로
    // 유지 — 이건 클릭 리포커스를 추가할 뿐 대체 아님.
    // ADR-0066
    pub fn set_focused_slot(&mut self, view_id: Uuid, slot_id: Uuid) -> Result<(), LayoutError> {
        let v = self.view_mut(view_id)?;
        if !tree::contains_slot(&v.layout, slot_id) {
            return Err(LayoutError::SlotNotFound(slot_id));
        }
        v.focused_slot_id = Some(slot_id);
        self.bump_version();
        Ok(())
    }

    // view-id-키(name 은 View 속성 — split_slot 과 동형으로 소속 창은 view_owner 파생).
    // ★이름 정규화는 프론트 경계 몫★: 여기선 받은 문자열을 그대로 저장한다(trim/공백거부는 사람 UI·
    // LLM command 어댑터가 invoke 전에 처리).
    pub fn rename_tab(&mut self, view_id: Uuid, name: String) -> Result<(), LayoutError> {
        let v = self.view_mut(view_id)?;
        v.name = name;
        self.bump_version();
        Ok(())
    }

    pub fn close_slot(&mut self, view_id: Uuid, slot_id: Uuid) -> Result<(), LayoutError> {
        let v = self.view_mut(view_id)?;
        if !tree::close_in_tree(&mut v.layout, slot_id) {
            return Err(LayoutError::SlotNotFound(slot_id));
        }
        Self::fixup_focus(v);
        self.bump_version();
        Ok(())
    }

    // view 안 slot_id 슬롯에 agent_id(참조 문자열) 배정. ★데몬에 실재 검증 안 함(ADR-0035/0006).
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

    // view 안 slot_id 슬롯의 콘텐츠를 `content`(SlotContent 제네릭)로 교체한다(ADR-0063 배치 제어 표면).
    // assign_agent 의 미러이나 에이전트 전용이 아니라 유니온 전체(Empty/Agent/AgentList/PresetPalette)를
    // 받는다 — 트리(에이전트)·팔레트를 슬롯에 배치하는 §5 LLM/사람 공용 경로. ★덮어쓰기 시맨틱(assign 과
    // 동형)★: 점유 슬롯도 무조건 교체(점유 방어는 없음 — 배치 command 는 명시적 교체 의도).
    pub fn set_slot_content(
        &mut self,
        view_id: Uuid,
        slot_id: Uuid,
        content: SlotContent,
    ) -> Result<(), LayoutError> {
        let v = self.view_mut(view_id)?;
        if !tree::set_in_tree(&mut v.layout, slot_id, content) {
            return Err(LayoutError::SlotNotFound(slot_id));
        }
        self.bump_version();
        Ok(())
    }

    // ── move_slot_to_window 2-phase 지원(§5-3, G4) ───────────────────────────

    // view 안 slot_id 슬롯의 콘텐츠(SlotContent)를 clone 해 반환한다(참조 아님 — 락 밖 반출용).
    pub fn slot_content(&self, view_id: Uuid, slot_id: Uuid) -> Result<SlotContent, LayoutError> {
        let v = self
            .views
            .get(&view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        tree::find_slot(&v.layout, slot_id)
            .cloned()
            .ok_or(LayoutError::SlotNotFound(slot_id))
    }

    // ★phase A★: 소스 슬롯 콘텐츠를 담은 임시 View 를 만든다(아직 **어느 창 tabs 에도 안 넣음** — orphan
    // 방지, phase C 에서 삽입). 소스 슬롯은 안 건드림(phase C 에서 close). 빈 슬롯(Empty)이면 Err
    // (pop-out 대상 없음 — 메뉴가 empty 를 hideOn 으로 숨기나 코어도 방어).
    //
    // ★ADR-0064 — 콘텐츠 일반화★: 모든 슬롯 종류가 팝업 가능(불변식 5 — 다중 참조 허용은 Agent 뿐 아니라
    // 콘텐츠 일반에 적용). 반환한 SlotContent 로 호출자가 agent 구독 마이그레이션(still-ours close 가드)이
    // 필요한지(= Agent 인지)를 판별한다.
    // ADR-0064
    pub fn prepare_detached_view(
        &mut self,
        src_view: ViewId,
        src_slot: Uuid,
        name: String,
    ) -> Result<(ViewId, SlotContent), LayoutError> {
        let content = self.slot_content(src_view, src_slot)?;
        if content.is_empty() {
            return Err(LayoutError::SlotNotFound(src_slot));
        }
        let id = self.make_view(name);
        let slot = {
            let v = self.views.get(&id).expect("방금 만든 View");
            tree::first_slot_id(&v.layout)
        };
        // view_owner 미배정. 그래서 배치는 tree 직접.
        // ADR-0064: assign_in_tree(agent 전용) 대신 set_in_tree(제네릭)로 콘텐츠 종류 전체를 옮긴다.
        if let Some(v) = self.views.get_mut(&id) {
            let _ = tree::set_in_tree(&mut v.layout, slot, content.clone());
        }
        self.bump_version();
        Ok((id, content))
    }

    // ★phase A 롤백★: prepare_detached_view 로 만든 임시 View 를 제거(창 삽입 전이라 tabs 갱신 불필요).
    pub fn drop_detached_view(&mut self, view: ViewId) {
        self.views.remove(&view);
        self.view_owner.remove(&view); // 안전(정상 경로엔 owner 없음).
        self.bump_version();
    }

    // ★phase C — 기존 창에 삽입★: 임시 View 를 `to_window` 의 새 탭으로 삽입·활성화(create_tab 상당).
    // ★재검증(G4)★: `to_window` 가 여전히 존재할 때만 삽입 — 부재면 Err(호출자가 롤백).
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

    // ★phase C — 새 창 생성 + 임시 View 를 그 창 첫 탭으로★. label 은 호출자 발급(단조 카운터).
    // 새 창 windows 엔트리 생성 + view_owner 쌍 갱신.
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

// `spawn_into`(D-7) 슬롯 해소 실패 사유(TRD §6 G9). command 레이어가 문자열로 옮긴다.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpawnSlotError {
    #[error("슬롯 {0} 이미 점유됨(덮어쓰기 금지 — split_slot 으로 빈 슬롯을 만든 뒤 재시도)")]
    SlotOccupied(Uuid),
    #[error("슬롯 {0} 없음")]
    SlotNotFound(Uuid),
    // USER DECISION 2b — 자동 split/덮어쓰기 안 함. // ADR-0059
    #[error("이 탭에 빈 슬롯 없음(slot 미지정 — split_slot 으로 빈 슬롯을 만들거나 다른 탭 사용)")]
    NoEmptySlot,
}

// ★spawn_into 슬롯 해소(TRD §6 G9)★.
// - `slot=None`(USER DECISION 2b): 트리를 전위 순회(좌측 우선)해 **첫 번째 빈 슬롯**을 타깃한다. 빈 슬롯이
//   하나도 없으면 `NoEmptySlot`(자동 split·덮어쓰기 안 함). ★2b 이전(leftmost-only)과 다름★ — split 된
//   탭에서 좌측이 점유돼도 다른 빈 슬롯이 있으면 거기로 간다.
//
// ★왜 순수 함수로 분리했나★: 스폰(데몬 async)·락·emit 은 command 레이어가 다루고, 여기 "정책 판정"만 떼어
//   Tauri 무링크 throwaway-mount 로 회귀 단언한다(ADR-0012 격리). 배정 자체는 assign_agent 가 한다.
pub fn resolve_spawn_slot(view: &View, slot: Option<Uuid>) -> Result<Uuid, SpawnSlotError> {
    match slot {
        Some(target) => match tree::find_slot(&view.layout, target) {
            Some(SlotContent::Empty) => Ok(target),
            // ADR-0060: Agent 외 콘텐츠(AgentList/PresetPalette)도 슬롯을 점유 중 — 스폰 덮어쓰기 금지.
            Some(SlotContent::Agent { .. })
            | Some(SlotContent::AgentList)
            | Some(SlotContent::PresetPalette) => Err(SpawnSlotError::SlotOccupied(target)),
            None => Err(SpawnSlotError::SlotNotFound(target)),
        },
        None => tree::first_empty_slot_id(&view.layout).ok_or(SpawnSlotError::NoEmptySlot),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseTabOutcome {
    Stayed,
    // 팝업 마지막 탭 → command 레이어가 OS 창을 닫아야 함(에이전트는 생존).
    WindowClosed,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_slot_of(mgr: &ViewManager, view_id: Uuid) -> Uuid {
        let v = mgr.views.get(&view_id).unwrap();
        tree::first_slot_id(&v.layout)
    }

    fn main_active(mgr: &ViewManager) -> ViewId {
        mgr.windows.get(MAIN_WINDOW_LABEL).unwrap().active
    }

    fn assert_invariants(mgr: &ViewManager) {
        for vid in mgr.views.keys() {
            assert!(
                mgr.view_owner.contains_key(vid),
                "불변식2: View {vid} 에 소유 창 없음"
            );
        }
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
        for (label, wt) in &mgr.windows {
            for vid in &wt.tabs {
                assert_eq!(
                    mgr.view_owner.get(vid).map(|s| s.as_str()),
                    Some(label.as_str()),
                    "불변식1: windows[{label}].tabs 의 {vid} 소유 불일치"
                );
            }
            assert!(
                wt.tabs.contains(&wt.active),
                "불변식3: windows[{label}].active 가 tabs 밖"
            );
            assert!(!wt.tabs.is_empty(), "빈 창은 존재 금지");
        }
        for (vid, label) in &mgr.view_owner {
            let wt = mgr.windows.get(label).expect("owner 창 존재");
            assert!(
                wt.tabs.contains(vid),
                "불변식1 역: view_owner[{vid}]={label} 인데 tabs 에 없음"
            );
        }
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
        assert!(!mgr.windows.contains_key("agent-tree"));
        assert_invariants(&mgr);
    }

    #[test]
    fn new_main_default_layout_is_agent_list_split_empty() {
        let mgr = ViewManager::new();
        let v0 = main_active(&mgr);
        let layout = &mgr.views.get(&v0).unwrap().layout;
        match layout {
            LayoutNode::Split { dir, ratio, a, b } => {
                assert_eq!(*dir, SplitDir::Horizontal, "가로 분할");
                assert_eq!(*ratio, 0.2, "좌측(트리)을 작게");
                assert!(
                    matches!(
                        a.as_ref(),
                        LayoutNode::Slot {
                            content: SlotContent::AgentList,
                            ..
                        }
                    ),
                    "좌측 = AgentList 슬롯"
                );
                assert!(
                    matches!(
                        b.as_ref(),
                        LayoutNode::Slot {
                            content: SlotContent::Empty,
                            ..
                        }
                    ),
                    "우측 = Empty 슬롯"
                );
            }
            _ => panic!("부팅 기본은 Split 이어야 함"),
        }
    }

    #[test]
    fn create_tab_stays_single_empty_slot() {
        // ★ADR-0063★: 부팅 기본만 분할 — 트리를 모든 탭에 강제 안 함.
        let mut mgr = ViewManager::new();
        let t = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        assert!(matches!(
            mgr.views.get(&t).unwrap().layout,
            LayoutNode::Slot {
                content: SlotContent::Empty,
                ..
            }
        ));
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
        let pv = mgr.create_window("slot-popup-1").unwrap();
        mgr.switch_tab(MAIN_WINDOW_LABEL, main0).unwrap();
        assert_eq!(main_active(&mgr), main0);
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
        assert!(mgr.create_window("main").is_err());
    }

    // ── close_tab 상태기계(§5-2) ─────────────────────────────────────────────

    #[test]
    fn close_active_tab_succeeds_right_neighbor() {
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
        let mut mgr = ViewManager::new();
        let a = main_active(&mgr);
        let b = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        mgr.close_tab(MAIN_WINDOW_LABEL, b).unwrap();
        assert_eq!(main_active(&mgr), a, "왼쪽 탭 승계(오른쪽 없음)");
        assert_invariants(&mgr);
    }

    #[test]
    fn close_non_active_tab_keeps_active() {
        let mut mgr = ViewManager::new();
        let a = main_active(&mgr);
        let b = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        mgr.close_tab(MAIN_WINDOW_LABEL, a).unwrap();
        assert_eq!(main_active(&mgr), b, "비활성 닫아도 active 유지");
        assert_invariants(&mgr);
    }

    #[test]
    fn close_main_last_tab_forces_empty_tab() {
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
            LayoutNode::Slot {
                content: SlotContent::Empty,
                ..
            }
        ));
        assert_eq!(wt.active, new_id);
        assert_invariants(&mgr);
    }

    #[test]
    fn close_popup_last_tab_closes_window() {
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
        let mut mgr = ViewManager::new();
        let err = mgr.close_window(MAIN_WINDOW_LABEL).unwrap_err();
        assert!(matches!(err, LayoutError::MainNotClosable));
        assert_invariants(&mgr);
    }

    #[test]
    fn close_window_multitab_drops_all_views() {
        let mut mgr = ViewManager::new();
        let p0 = mgr.create_window("slot-popup-1").unwrap();
        let p1 = mgr.create_tab("slot-popup-1", None).unwrap();
        let p2 = mgr.create_tab("slot-popup-1", None).unwrap();
        let dropped = mgr.close_window("slot-popup-1").unwrap();
        // 순서 무관 — 집합 비교.
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
        assert!(matches!(
            v.layout,
            LayoutNode::Slot {
                content: SlotContent::Empty,
                ..
            }
        ));
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

    // ── set_focused_slot (click-to-focus — ADR-0066 결정 1) ───────────────────

    #[test]
    fn set_focused_slot_updates_focus_and_bumps_version() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let (left, right) = {
            let v = mgr.views.get(&view_id).unwrap();
            match &v.layout {
                LayoutNode::Split { a, b, .. } => (tree::first_slot_id(a), tree::first_slot_id(b)),
                _ => panic!("부팅 기본은 Split"),
            }
        };
        assert_eq!(mgr.views.get(&view_id).unwrap().focused_slot_id, Some(left));
        let ver = mgr.version;
        mgr.set_focused_slot(view_id, right).unwrap();
        assert_eq!(
            mgr.views.get(&view_id).unwrap().focused_slot_id,
            Some(right),
            "포커스가 클릭한 슬롯으로 이동"
        );
        assert_eq!(mgr.version, ver + 1, "성공 시 version +1");
        assert_invariants(&mgr);
    }

    #[test]
    fn set_focused_slot_invalid_view_is_err() {
        let mut mgr = ViewManager::new();
        assert!(matches!(
            mgr.set_focused_slot(Uuid::new_v4(), Uuid::new_v4())
                .unwrap_err(),
            LayoutError::ViewNotFound(_)
        ));
    }

    #[test]
    fn set_focused_slot_absent_slot_is_err_noop() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let before = mgr.views.get(&view_id).unwrap().focused_slot_id;
        let ver = mgr.version;
        assert!(matches!(
            mgr.set_focused_slot(view_id, Uuid::new_v4()).unwrap_err(),
            LayoutError::SlotNotFound(_)
        ));
        assert_eq!(
            mgr.views.get(&view_id).unwrap().focused_slot_id,
            before,
            "실패 시 focused_slot_id 불변"
        );
        assert_eq!(mgr.version, ver, "실패 시 version 불변(no-op)");
        assert_invariants(&mgr);
    }

    #[test]
    fn rename_tab_renames_and_bumps_version() {
        let mut mgr = ViewManager::new();
        let view_id = mgr
            .create_tab(MAIN_WINDOW_LABEL, Some("Old".to_string()))
            .unwrap();
        let ver = mgr.version;
        mgr.rename_tab(view_id, "New Name".to_string()).unwrap();
        assert_eq!(
            mgr.views.get(&view_id).unwrap().name,
            "New Name",
            "View.name 이 교체됨"
        );
        assert_eq!(mgr.version, ver + 1, "성공 시 version +1");
        assert_invariants(&mgr);
    }

    #[test]
    fn rename_tab_invalid_view_is_err_noop() {
        let mut mgr = ViewManager::new();
        let ver = mgr.version;
        assert!(matches!(
            mgr.rename_tab(Uuid::new_v4(), "X".to_string()).unwrap_err(),
            LayoutError::ViewNotFound(_)
        ));
        assert_eq!(mgr.version, ver, "실패 시 version 불변(no-op)");
        assert_invariants(&mgr);
    }

    #[test]
    fn assign_agent_sets_ref() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let slot = first_slot_of(&mgr, view_id);
        mgr.assign_agent(view_id, slot, "agent-42".into()).unwrap();
        let v = mgr.views.get(&view_id).unwrap();
        assert_eq!(
            tree::find_slot(&v.layout, slot).unwrap().agent_id(),
            Some("agent-42")
        );
        assert_invariants(&mgr);
    }

    #[test]
    fn assign_same_agent_to_two_views_is_allowed() {
        let mut mgr = ViewManager::new();
        let v1 = main_active(&mgr);
        let s1 = first_slot_of(&mgr, v1);
        mgr.assign_agent(v1, s1, "shared".into()).unwrap();
        let v2 = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let s2 = first_slot_of(&mgr, v2);
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

    // ── set_slot_content (제네릭 배치 command — ADR-0063) ─────────────────────

    #[test]
    fn set_slot_content_places_agent_list() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let slot = first_slot_of(&mgr, v);
        let ver = mgr.version;
        mgr.set_slot_content(v, slot, SlotContent::AgentList)
            .unwrap();
        assert_eq!(
            tree::find_slot(&mgr.views.get(&v).unwrap().layout, slot).unwrap(),
            &SlotContent::AgentList
        );
        assert_eq!(mgr.version, ver + 1, "성공 시 version +1");
        assert_invariants(&mgr);
    }

    #[test]
    fn set_slot_content_can_clear_to_empty() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let slot = first_slot_of(&mgr, v);
        mgr.assign_agent(v, slot, "occupant".into()).unwrap();
        mgr.set_slot_content(v, slot, SlotContent::Empty).unwrap();
        assert!(tree::find_slot(&mgr.views.get(&v).unwrap().layout, slot)
            .unwrap()
            .is_empty());
        assert_invariants(&mgr);
    }

    #[test]
    fn set_slot_content_invalid_view_is_err() {
        let mut mgr = ViewManager::new();
        assert!(matches!(
            mgr.set_slot_content(Uuid::new_v4(), Uuid::new_v4(), SlotContent::AgentList)
                .unwrap_err(),
            LayoutError::ViewNotFound(_)
        ));
    }

    #[test]
    fn set_slot_content_invalid_slot_is_err_noop() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let ver = mgr.version;
        assert!(matches!(
            mgr.set_slot_content(v, Uuid::new_v4(), SlotContent::PresetPalette)
                .unwrap_err(),
            LayoutError::SlotNotFound(_)
        ));
        assert_eq!(mgr.version, ver, "실패 시 version 불변(no-op)");
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
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.assign_agent(src, slot, "moving".into()).unwrap();
        let (tmp, content) = mgr
            .prepare_detached_view(src, slot, "Popup".into())
            .unwrap();
        assert_eq!(
            content,
            SlotContent::Agent {
                agent_id: "moving".into()
            }
        );
        let tslot = first_slot_of(&mgr, tmp);
        assert_eq!(
            mgr.slot_agent(tmp, tslot).unwrap().as_deref(),
            Some("moving")
        );
        // orphan 방지 — phase C 에서 삽입.
        assert!(
            mgr.view_owner.get(&tmp).is_none(),
            "phase A 는 view_owner 미배정"
        );
        // phase C 에서 close.
        assert_eq!(
            mgr.slot_agent(src, slot).unwrap().as_deref(),
            Some("moving")
        );
    }

    #[test]
    fn prepare_detached_view_moves_agent_list_content() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.set_slot_content(src, slot, SlotContent::AgentList)
            .unwrap();
        let (tmp, content) = mgr
            .prepare_detached_view(src, slot, "Popup".into())
            .unwrap();
        assert_eq!(content, SlotContent::AgentList, "반환 콘텐츠 = AgentList");
        let tslot = first_slot_of(&mgr, tmp);
        assert_eq!(
            tree::find_slot(&mgr.views.get(&tmp).unwrap().layout, tslot).unwrap(),
            &SlotContent::AgentList
        );
        // phase C 에서 close.
        assert_eq!(
            tree::find_slot(&mgr.views.get(&src).unwrap().layout, slot).unwrap(),
            &SlotContent::AgentList
        );
    }

    #[test]
    fn prepare_detached_view_empty_slot_is_err() {
        // ADR-0064: Empty 만 거부(팝업 대상 없음). agent_list/preset_palette 는 위 테스트대로 허용.
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        assert!(mgr.prepare_detached_view(src, slot, "P".into()).is_err());
    }

    #[test]
    fn insert_tab_into_existing_window_phase_c() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.assign_agent(src, slot, "moving".into()).unwrap();
        let existing = mgr.create_window("slot-popup-1").unwrap();
        let (tmp, _content) = mgr
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
        let (tmp, _content) = mgr
            .prepare_detached_view(src, slot, "Popup".into())
            .unwrap();
        let err = mgr.insert_tab_into("gone", tmp).unwrap_err();
        assert!(matches!(err, LayoutError::WindowNotFound(_)));
        assert!(mgr.view_owner.get(&tmp).is_none());
    }

    #[test]
    fn insert_same_agent_into_window_that_has_it_is_allowed() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.assign_agent(src, slot, "shared".into()).unwrap();
        let existing = mgr.create_window("slot-popup-1").unwrap();
        let eslot = first_slot_of(&mgr, existing);
        mgr.assign_agent(existing, eslot, "shared".into()).unwrap();
        let (tmp, _content) = mgr
            .prepare_detached_view(src, slot, "Popup".into())
            .unwrap();
        mgr.insert_tab_into("slot-popup-1", tmp).unwrap();
        assert_eq!(mgr.windows.get("slot-popup-1").unwrap().tabs.len(), 2);
        assert_invariants(&mgr);
    }

    #[test]
    fn attach_view_as_new_window_phase_c() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.assign_agent(src, slot, "moving".into()).unwrap();
        let (tmp, _content) = mgr
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
        let (tmp, _content) = mgr
            .prepare_detached_view(src, slot, "Popup".into())
            .unwrap();
        mgr.drop_detached_view(tmp);
        assert!(!mgr.views.contains_key(&tmp), "임시 View 제거");
        assert_invariants(&mgr);
    }

    // ── resolve_spawn_slot (spawn_into 순수 슬롯 정책 — TRD §6 G9) ─────────────────

    #[test]
    fn resolve_none_targets_empty_root_slot() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        let view = mgr.views.get(&v).unwrap();
        assert_eq!(
            resolve_spawn_slot(view, None).unwrap(),
            root,
            "slot=None → 빈 root 슬롯"
        );
    }

    #[test]
    fn resolve_none_on_single_occupied_slot_is_no_empty_slot() {
        // ★USER DECISION 2b★: 자동 split·덮어쓰기 안 함. 2b 이전엔 SlotOccupied 였으나, 이제 "빈 슬롯
        //   스캔"이라 빈 슬롯 부재를 NoEmptySlot 으로 신고한다.
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        mgr.assign_agent(v, root, "existing".into()).unwrap();
        let view = mgr.views.get(&v).unwrap();
        assert_eq!(
            resolve_spawn_slot(view, None),
            Err(SpawnSlotError::NoEmptySlot)
        );
    }

    #[test]
    fn resolve_none_on_split_tab_picks_first_empty_not_leftmost() {
        // ★USER DECISION 2b 회귀★: 2b 이전엔 leftmost-only 라 SlotOccupied 였다.
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        let right = mgr.split_slot(v, root, SplitDir::Horizontal).unwrap();
        mgr.assign_agent(v, root, "occupied".into()).unwrap();
        let view = mgr.views.get(&v).unwrap();
        assert_eq!(
            resolve_spawn_slot(view, None).unwrap(),
            right,
            "slot=None → 점유 좌측 건너뛰고 첫 빈 슬롯(우측)"
        );
    }

    #[test]
    fn resolve_none_on_fully_occupied_split_tab_is_no_empty_slot() {
        // 자동 split·덮어쓰기 안 함.
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        let right = mgr.split_slot(v, root, SplitDir::Horizontal).unwrap();
        mgr.assign_agent(v, root, "a".into()).unwrap();
        mgr.assign_agent(v, right, "b".into()).unwrap();
        let view = mgr.views.get(&v).unwrap();
        assert_eq!(
            resolve_spawn_slot(view, None),
            Err(SpawnSlotError::NoEmptySlot)
        );
    }

    #[test]
    fn resolve_none_after_create_tab_targets_fresh_root() {
        // 2b 에서도 유지.
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        let view = mgr.views.get(&v).unwrap();
        assert_eq!(resolve_spawn_slot(view, None).unwrap(), root);
    }

    #[test]
    fn resolve_then_assign_holds_invariants() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let target = {
            let view = mgr.views.get(&v).unwrap();
            resolve_spawn_slot(view, None).unwrap()
        };
        mgr.assign_agent(v, target, "spawned-agent".into()).unwrap();
        assert_eq!(
            mgr.slot_agent(v, target).unwrap().as_deref(),
            Some("spawned-agent")
        );
        assert_invariants(&mgr);
    }

    #[test]
    fn owner_of_rejects_view_from_other_window() {
        // ★spawn_into tab=Some 소유 검증 predicate★: 다른 창이 소유한 view 를 대상 창(window)의 탭이라
        //   주장하면 owner_of 불일치로 거부해야 한다(spawn_into 가 이 predicate 로 배치 전 검증).
        let mut mgr = ViewManager::new();
        let popup_view = mgr.create_window("slot-popup-1").unwrap();
        assert_eq!(
            mgr.owner_of(popup_view).map(|s| s.as_str()),
            Some("slot-popup-1")
        );
        assert_ne!(
            mgr.owner_of(popup_view).map(|s| s.as_str()),
            Some(MAIN_WINDOW_LABEL),
            "다른 창 소유 view 는 main 의 탭이 아님(spawn_into 배치 전 거부)"
        );
    }

    #[test]
    fn resolve_some_empty_returns_that_slot() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        let new_slot = mgr.split_slot(v, root, SplitDir::Horizontal).unwrap();
        let view = mgr.views.get(&v).unwrap();
        assert_eq!(
            resolve_spawn_slot(view, Some(new_slot)).unwrap(),
            new_slot,
            "slot=Some+빈 → 그 슬롯"
        );
    }

    #[test]
    fn resolve_some_occupied_errors_no_overwrite() {
        // 자동 split/replace 안 함(G9).
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        mgr.assign_agent(v, root, "existing".into()).unwrap();
        let view = mgr.views.get(&v).unwrap();
        assert_eq!(
            resolve_spawn_slot(view, Some(root)),
            Err(SpawnSlotError::SlotOccupied(root))
        );
    }

    #[test]
    fn resolve_some_missing_slot_errors() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let bogus = Uuid::new_v4();
        let view = mgr.views.get(&v).unwrap();
        assert_eq!(
            resolve_spawn_slot(view, Some(bogus)),
            Err(SpawnSlotError::SlotNotFound(bogus))
        );
    }
}
