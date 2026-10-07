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
//!
//! ## ★모르는 슬롯 내용 곁표(TRD S21-storage §6-2)★
//! 슬롯 내용 쓰기는 `write_slot_content` 하나로만, 뷰 지우기는 `remove_view` 하나로만 한다 — 그 둘과
//! `close_slot` 이 곁표(`unknown_content`) 항목을 거둔다. `tree::set_in_tree` 를 직접 부르거나 `views` 에서 직접
//! 지우면 사용자가 바꾼 슬롯에 옛 원문이 저장으로 되살아난다(§12 R6). 빈/점유 판정은 `slot_is_free` 하나다.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};
use uuid::Uuid;

use super::geometry::{self, PxRect, RectF64, SlotRect};
use super::tree;
use super::types::{
    LayoutNode, SlotContent, SplitDir, SplitRatioOutcome, UiMetrics, View, ViewMeta, ViewSnapshot,
};
use crate::theme::UiTheme;

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
    #[error("ui 지표 거절: {0}")]
    InvalidMetrics(String),
    // ADR-0227
    #[error("split 없음: {0}")]
    SplitNotFound(Uuid),
    // 값을 문자열로 싣는 이유: `f64` 를 담으면 이 enum 의 `Eq` 가 깨진다.
    #[error("비율 거절: {0} — 유한한 수여야 한다")]
    InvalidRatio(String),
    #[error("칸이 너무 작아 더 나눌 수 없음 — 새 경계가 그 칸 안에 표현되지 않는다(부동소수 한계). 이 칸을 품은 분할의 비율을 넓히거나 다른 칸을 나누시오")]
    SplitTooDeep,
}

/// `ViewManager::set_split_ratio` 의 결과 — `ratio` 는 셸이 지금 가진 값이다(`Applied` 면 방금 쓴 값,
/// 아니면 손대지 않은 원래 값).
// ADR-0227
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitRatioResult {
    pub ratio: f64,
    pub outcome: SplitRatioOutcome,
}

// 보고 지표 방어선(TRD §2d) — 정책 값이 아니다. 테두리 폭·최소 칸 크기의 실제 값은 화면이 정한다.
const INSET_MAX_PX: f64 = 64.0;
const MIN_PANE_PX_MAX: u32 = 1000;

/// 창의 탭 내용 영역 크기(정수 CSS px). 그 창의 탭 전부가 이 영역에 겹쳐 그려진다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanvasPx {
    pub w: u32,
    pub h: u32,
}

/// 한 칸의 px 사각형 — `frame` = 칸 틀(캔버스 원점 기준 정수 CSS px) · `content` = 틀을 칸 틀 안쪽 여백만큼
/// 줄인 영역(폭·높이 ≥ 0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlotPx {
    pub frame: PxRect,
    pub content: RectF64,
}

/// 창의 마지막 보통(최소화도 최대화도 아닌) 자리. `x`·`y` = 물리 픽셀 바깥 위치(`outer_position` 그대로 — 정수
/// 값), `w`·`h` = 논리 안쪽 크기(`inner_size` 를 그 창의 배율로 나눈 값) — 복원은 같은 짝으로 놓는다(TRD
/// S21-storage §6-3 · 위치를 논리로 두지 않는 이유 = `state::placement` 머리).
///
/// 네 값은 언제나 유한하고 크기(`w` · `h`)는 0 보다 크다 — [`Self::new`] 만 만든다. 유한하지 않은 실수는 JSON 에
/// `null` 로 나가고, 크기 0 인 창은 복원해도 보이지 않는다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowBounds {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl WindowBounds {
    /// 하나라도 유한하지 않거나(NaN · ±∞) `w` · `h` 가 0 이하면 `None`.
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Option<Self> {
        let finite = [x, y, w, h].iter().all(|v| v.is_finite());
        (finite && w > 0.0 && h > 0.0).then_some(WindowBounds { x, y, w, h })
    }

    pub fn x(self) -> f64 {
        self.x
    }

    pub fn y(self) -> f64 {
        self.y
    }

    pub fn w(self) -> f64 {
        self.w
    }

    pub fn h(self) -> f64 {
        self.h
    }
}

/// 창 게터로 읽은 한 벌(`is_minimized` · `is_maximized` · 자리). ★`ViewManager` · 트리 칸 락을 잡기 전에 전부
/// 읽는다★ — 레이아웃 락 보유 중 OS 호출 금지(TRD S21-storage §6-3 · `apply.rs` 머리 「락 규율」).
///
/// `bounds` = 그때 읽은 자리 — 못 읽었으면(게터 실패 · [`WindowBounds::new`] 거절) `None`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowPlacement {
    pub minimized: bool,
    pub maximized: bool,
    pub bounds: Option<WindowBounds>,
}

/// 창 하나의 화면 속성 — 창 항목과 같이 살고 같이 죽는다. label 을 키로 창 밖에 두면 재시작 뒤 같은 label 의
/// 다른 창에 조용히 적용된다(ADR-0167 「근거」).
///
/// 레이아웃 창은 `WindowTabs::attrs`, 트리 창은 `state::tree_attrs` 가 든다. 바꾸는 길은 그 둘의 setter 뿐이다 —
/// 거기서 변경 번호가 오른다(레이아웃 `version` 이 아니라 `ViewManager::attrs_rev` — 창을 끌 때마다 레이아웃
/// 번호가 튀지 않게).
// ADR-0265
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WindowAttrs {
    /// `None` = 전역 테마(설정 `theme.default`)를 따른다.
    pub theme: Option<UiTheme>,
    /// `None` = 이 창의 보통 자리를 아직 못 봤다.
    pub bounds: Option<WindowBounds>,
    /// 복원은 `bounds` 로 놓은 뒤 최대화한다(사용자 결정 F8). 트리 창은 언제나 `false` 다(`state::tree_attrs` 머리).
    pub maximized: bool,
}

impl WindowAttrs {
    /// 게터 한 벌을 적는다 — 바뀌었으면 `true`. `memo` = 이 창의 바로 앞 읽기 기억([`PlacementMemo`]).
    ///
    /// - 최소화 중이면 아무것도 바꾸지 않는다 — 최소화를 풀면 그 앞 상태(최대화 여부 포함)로 돌아가므로, 그 앞
    ///   값이 다음 복원이 세울 상태다.
    /// - 최대화면 그 표식만 세운다 — `bounds` 는 최대화 전 보통 자리로 남는다(F8). ★단 바로 앞 읽기가 보통으로
    ///   적은 자리가 지금 읽은 자리와 같으면 그 읽기를 되돌린다★ — 사용자가 최대화하면 tao 가 `Moved` 를 최대화
    ///   표식이 서기 전에 내서(tao 0.35 `platform_impl/windows/event_loop.rs` — `WM_WINDOWPOSCHANGED` 에서 `Moved`,
    ///   그 뒤 `WM_SIZE` 가 표식을 세운다) 최대화된 사각형이 보통 자리로 한 번 적힌다. tauri-plugin-window-state 가
    ///   `Moved` 마다 앞 위치를 `prev_x` · `prev_y` 로 밀어 두고 최대화면 그것으로 놓는 것과 같은 생각이다
    ///   (tauri-apps/plugins-workspace `3d8a3c877b` 의 `plugins/window-state/src/lib.rs` L492-493 · L213-222).
    ///   ★「같은 자리」 조건을 빼지 말 것★ — tao 의 `maximize` 는 표식을 먼저 세워 잘못 적힌 읽기가 없으므로, 조건
    ///   없이 되돌리면 그 앞의 진짜 보통 읽기(부팅 복원이 입힌 크기 등)를 버린다.
    /// - 보통이면 표식을 내리고 `bounds` 를 적는다(`None` 이면 앞 값을 둔다).
    pub(crate) fn observe(&mut self, seen: WindowPlacement, memo: &mut PlacementMemo) -> bool {
        let before = *self;
        let WindowPlacement {
            minimized,
            maximized,
            bounds,
        } = seen;
        let last = memo.last.take();
        if !minimized {
            self.maximized = maximized;
            if maximized {
                if let (Some((written, prior)), Some(now)) = (last, bounds) {
                    if written == now {
                        self.bounds = prior;
                    }
                }
            } else if let Some(bounds) = bounds {
                memo.last = Some((bounds, self.bounds));
                self.bounds = Some(bounds);
            }
        }
        *self != before
    }

    /// 바뀌었으면 `true`.
    pub(crate) fn set_theme(&mut self, theme: Option<UiTheme>) -> bool {
        let changed = self.theme != theme;
        self.theme = theme;
        changed
    }
}

/// 창 하나의 바로 앞 읽기 기억 — 그 읽기가 보통 자리를 적었으면 (적은 자리, 그 앞 자리). 쓰는 곳은
/// [`WindowAttrs::observe`] 뿐이고 다음 읽기가 늘 지운다. 영속하지 않는다 — 창 항목 · 트리 칸과 같이 산다.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlacementMemo {
    last: Option<(WindowBounds, Option<WindowBounds>)>,
}

#[derive(Debug, Clone)]
pub struct WindowTabs {
    // 탭 순서(좌→우).
    pub tabs: Vec<ViewId>,
    pub active: ViewId,
    // 둘 다 그 창 웹뷰가 보고하기 전엔 None 이고 창 엔트리와 함께 사라진다. 탭마다가 아니라 창마다다.
    // ADR-0227
    pub canvas: Option<CanvasPx>,
    // ADR-0227
    pub metrics: Option<UiMetrics>,
    /// 영속 신원 — main 은 `main`, 팝아웃은 그 창 항목이 처음 생길 때 뽑은 UUID(하이픈 소문자). 저장 · 복원은 이
    /// 값을 쓴다. 런타임 label(맵 키)은 한 부팅 안에서만의 신원이다 — 팝아웃 label 은 실행마다 1 부터 다시
    /// 센다(TRD S21-storage §6-3).
    // ADR-0167
    pub window_id: String,
    pub attrs: WindowAttrs,
    pub(crate) placement_memo: PlacementMemo,
}

impl WindowTabs {
    // 새 창 엔트리는 전부 여기서 만든다 — 측정값은 그 창 웹뷰가 보고할 때까지 비어 있어야 한다.
    fn new(window_id: String, attrs: WindowAttrs, tabs: Vec<ViewId>, active: ViewId) -> Self {
        WindowTabs {
            tabs,
            active,
            canvas: None,
            metrics: None,
            window_id,
            attrs,
            placement_memo: PlacementMemo::default(),
        }
    }

    fn new_popout(view: ViewId) -> Self {
        Self::new(
            Uuid::new_v4().to_string(),
            WindowAttrs::default(),
            vec![view],
            view,
        )
    }
}

/// 영속에서 되살린 창 하나 — 메모리 모양. 영속 모양의 해석 · 수리는 `state::convert` 가 끝낸 뒤 넘긴다.
#[derive(Debug, Clone)]
pub(crate) struct RestoredWindow {
    pub attrs: WindowAttrs,
    /// 탭 순서 그대로.
    pub tabs: Vec<View>,
    /// `tabs` 가 있으면 그 안의 하나여야 한다.
    pub active: Option<ViewId>,
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
    // 창 속성(`WindowTabs::attrs`)이 바뀔 때마다 +1 — `version` 은 올리지 않는다. 기록기는 둘을 같은지만 본다
    // (TRD S21-storage §6-4) — ★되돌리거나 초기화하지 않는다★: 마지막으로 본 값과 다시 같아지면 그 사이의 변경을
    // 못 본다.
    attrs_rev: u64,
    // 이 판이 모르는 슬롯 내용의 원문(뷰 → 슬롯 → JSON 객체). 그 슬롯의 메모리 내용은 `Empty` 이고 항목이 있는
    // 동안 「점유」다. 프론트로 나가지 않는다 — 스냅숏은 슬롯 id 만 싣는다(`foreign_slots`). 뷰로 먼저 가르는
    // 이유: 탭이 다른 창으로 옮겨도 그대로 따라가고, 뷰를 지울 때 한 번에 거둔다.
    unknown_content: HashMap<ViewId, HashMap<Uuid, Map<String, Value>>>,
}

impl Default for ViewManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewManager {
    // ★부팅 기본 = 다른 새 뷰와 같은 단일 빈 슬롯★ — 트리를 부팅에 깔지 않는다. 트리는 빈 슬롯 메뉴
    // (`set_slot_content`)로 사용자가 놓는다. // ADR-0222
    pub fn new() -> Self {
        let mut mgr = Self {
            views: HashMap::new(),
            view_owner: HashMap::new(),
            windows: HashMap::new(),
            version: 0,
            attrs_rev: 0,
            unknown_content: HashMap::new(),
        };
        let v0 = mgr.make_view("View 1".to_string());
        mgr.view_owner.insert(v0, MAIN_WINDOW_LABEL.to_string());
        mgr.windows.insert(
            MAIN_WINDOW_LABEL.to_string(),
            WindowTabs::new(
                MAIN_WINDOW_LABEL.to_string(),
                WindowAttrs::default(),
                vec![v0],
                v0,
            ),
        );
        mgr
    }

    /// 되살린 창들로 새 모델을 세운다 — `version` · `attrs_rev` 는 0, 곁표는 비었다(모르는 내용은 부르는 쪽이
    /// `set_unknown_content` 로 세운다). 각 View 의 포커스는 트리 첫 슬롯으로 고친다(`fixup_focus` 와 같은 규칙).
    ///
    /// main 의 탭이 없으면 빈 탭 하나(`View 1`)를 세운다(불변식 4). 그 밖에 불변식 1–4 를 세울 수 없는 입력은 `Err`
    /// 이고 아무것도 만들지 않는다 — 탭 없는 팝아웃 · 탭 밖이거나 없는 활성 탭 · 겹친 View id · `main` 이거나 겹친
    /// 팝아웃 label. 팝아웃 항목 = (label, 영속 id, 창).
    pub(crate) fn from_restored(
        main: RestoredWindow,
        popouts: Vec<(WindowLabel, String, RestoredWindow)>,
    ) -> Result<Self, String> {
        let mut mgr = Self {
            views: HashMap::new(),
            view_owner: HashMap::new(),
            windows: HashMap::new(),
            version: 0,
            attrs_rev: 0,
            unknown_content: HashMap::new(),
        };
        let RestoredWindow {
            attrs,
            tabs,
            active,
        } = main;
        let main_window = if tabs.is_empty() {
            let v0 = mgr.make_view("View 1".to_string());
            mgr.view_owner.insert(v0, MAIN_WINDOW_LABEL.to_string());
            WindowTabs::new(MAIN_WINDOW_LABEL.to_string(), attrs, vec![v0], v0)
        } else {
            let (tabs, active) = mgr.adopt_tabs(MAIN_WINDOW_LABEL, tabs, active)?;
            WindowTabs::new(MAIN_WINDOW_LABEL.to_string(), attrs, tabs, active)
        };
        mgr.windows
            .insert(MAIN_WINDOW_LABEL.to_string(), main_window);

        for (label, window_id, restored) in popouts {
            if mgr.windows.contains_key(&label) {
                return Err(format!("팝아웃 label {label} 이 겹친다"));
            }
            let RestoredWindow {
                attrs,
                tabs,
                active,
            } = restored;
            if tabs.is_empty() {
                return Err(format!("팝아웃 {window_id} 에 탭이 없다"));
            }
            let (tabs, active) = mgr.adopt_tabs(&label, tabs, active)?;
            mgr.windows
                .insert(label, WindowTabs::new(window_id, attrs, tabs, active));
        }
        Ok(mgr)
    }

    // 창 `label` 의 탭을 들인다 — 소유 쌍(불변식 1·2)과 활성 검사(불변식 3). 돌려주는 것 = (탭 순서, 활성 탭).
    fn adopt_tabs(
        &mut self,
        label: &str,
        tabs: Vec<View>,
        active: Option<ViewId>,
    ) -> Result<(Vec<ViewId>, ViewId), String> {
        let ids: Vec<ViewId> = tabs.iter().map(|v| v.id).collect();
        let active = match active {
            Some(active) if ids.contains(&active) => active,
            _ => {
                return Err(format!(
                    "창 {label} 의 활성 탭 {active:?} 이 탭 목록 밖이다"
                ))
            }
        };
        for mut view in tabs {
            if self.views.contains_key(&view.id) {
                return Err(format!("View {} 가 겹친다", view.id));
            }
            Self::fixup_focus(&mut view);
            self.view_owner.insert(view.id, label.to_string());
            self.views.insert(view.id, view);
        }
        Ok((ids, active))
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
        let geo = geometry::compute(&v.layout);
        Ok(ViewSnapshot {
            view_id: v.id,
            layout: v.layout.clone(),
            focused_slot_id: v.focused_slot_id,
            foreign_slots: self.foreign_slots(v),
            slot_spatial: super::spatial::compute_spatial(&v.layout),
            slot_rects: geo.slots,
            split_rects: geo.splits,
            ratio_min: tree::RATIO_MIN,
            ratio_max: tree::RATIO_MAX,
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

    /// 자동 배치가 이 슬롯에 내용을 놓아도 되는가 — 비었고(`SlotContent::Empty`) 모르는 내용도 쥐지 않았다.
    /// 없는 view·slot 은 `Err`. ADR-0059 의 빈/점유 판정은 이 하나로 낸다 — `SlotContent::is_empty` 만 보면
    /// 모르는 내용을 쥔 슬롯을 덮는다(TRD S21-storage §6-2).
    pub fn slot_is_free(&self, view_id: ViewId, slot_id: Uuid) -> Result<bool, LayoutError> {
        let v = self
            .views
            .get(&view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        self.free_in(v, slot_id)
            .ok_or(LayoutError::SlotNotFound(slot_id))
    }

    // 없는 slot 은 None.
    fn free_in(&self, view: &View, slot_id: Uuid) -> Option<bool> {
        let content = tree::find_slot(&view.layout, slot_id)?;
        Some(content.is_empty() && self.unknown_content(view.id, slot_id).is_none())
    }

    /// 슬롯 `slot_id` 가 쥔 모르는 내용의 원문 — 없으면 `None`(모르는 내용이 아니거나 없는 view·slot). 저장은 이
    /// 값이 있는 슬롯을 메모리 내용(`Empty`) 대신 이 원문으로 적는다.
    pub(crate) fn unknown_content(
        &self,
        view_id: ViewId,
        slot_id: Uuid,
    ) -> Option<&Map<String, Value>> {
        self.unknown_content.get(&view_id)?.get(&slot_id)
    }

    // 트리 전위 순 — 표 순회 순서(HashMap)를 스냅숏에 새지 않게 트리를 따라 거른다.
    fn foreign_slots(&self, view: &View) -> Vec<Uuid> {
        match self.unknown_content.get(&view.id) {
            None => Vec::new(),
            Some(slots) => tree::slot_ids(&view.layout)
                .into_iter()
                .filter(|s| slots.contains_key(s))
                .collect(),
        }
    }

    /// `spawn_into`(D-7) 의 슬롯 해소(TRD §6 G9) — 스폰·락·emit 은 command 레이어가 하고 여기는 정책 판정만
    /// 한다. 배정 자체는 `assign_agent` 가 한다.
    /// - `slot=None`(USER DECISION 2b): 트리를 전위 순회(좌측 우선)해 **첫 번째 빈 슬롯**(`slot_is_free`)을
    ///   고른다. 하나도 없으면 `NoEmptySlot` — 자동 split·덮어쓰기 안 함.
    /// - `slot=Some(s)`: 비었으면 `s` · 점유(모르는 내용 포함)면 `SlotOccupied` · 트리에 없으면 `SlotNotFound`.
    // ADR-0059
    pub fn resolve_spawn_slot(
        &self,
        view_id: ViewId,
        slot: Option<Uuid>,
    ) -> Result<Uuid, SpawnSlotError> {
        let v = self
            .views
            .get(&view_id)
            .ok_or(SpawnSlotError::ViewNotFound(view_id))?;
        match slot {
            Some(target) => match self.free_in(v, target) {
                Some(true) => Ok(target),
                Some(false) => Err(SpawnSlotError::SlotOccupied(target)),
                None => Err(SpawnSlotError::SlotNotFound(target)),
            },
            None => tree::slot_ids(&v.layout)
                .into_iter()
                .find(|&s| self.free_in(v, s) == Some(true))
                .ok_or(SpawnSlotError::NoEmptySlot),
        }
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

    // ── 내부 헬퍼 ───────────────────────────────────────────────────────────

    // 슬롯 내용 쓰기의 유일한 문 — 모르는 내용 항목을 지운다(무엇을 쓰든, `Empty` 여도). version 은 호출자가 올린다.
    fn write_slot_content(
        &mut self,
        view_id: ViewId,
        slot_id: Uuid,
        content: SlotContent,
    ) -> Result<(), LayoutError> {
        let v = self.view_mut(view_id)?;
        if !tree::set_in_tree(&mut v.layout, slot_id, content) {
            return Err(LayoutError::SlotNotFound(slot_id));
        }
        self.forget_unknown(view_id, slot_id);
        Ok(())
    }

    fn remember_unknown(&mut self, view_id: ViewId, slot_id: Uuid, raw: Map<String, Value>) {
        self.unknown_content
            .entry(view_id)
            .or_default()
            .insert(slot_id, raw);
    }

    fn forget_unknown(&mut self, view_id: ViewId, slot_id: Uuid) {
        if let Some(slots) = self.unknown_content.get_mut(&view_id) {
            slots.remove(&slot_id);
            if slots.is_empty() {
                self.unknown_content.remove(&view_id);
            }
        }
    }

    // 뷰를 지우는 유일한 자리. 창 탭 목록(`windows[*].tabs`)은 호출자가 맞춘다(불변식 1).
    fn remove_view(&mut self, view: ViewId) {
        self.views.remove(&view);
        self.view_owner.remove(&view);
        self.unknown_content.remove(&view);
    }

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
        self.windows
            .insert(label.to_string(), WindowTabs::new_popout(id));
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
        self.remove_view(view);
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
            self.remove_view(*vid);
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
        // 표현 가능성 가드: 늘 같은 쪽 자식을 나누면 새 경계가 부모 끝과 비트 단위로 같아져 한쪽 칸의 폭이
        // 0 이 된다. 새 경계를 실제 기하와 같은 식·같은 비율로 미리 재서 칸 안에 엄격히 들 때만 나눈다.
        // 이미 면적 0 인 칸(쓰기 경로로는 안 생기고 심긴 트리에만 있다)은 축과 무관하게 거절한다 — 다른 축으로
        // 나누면 경계는 칸 안에 들어도 두 새 칸이 다 면적 0 이다.
        // `min_pane_px` 보다 작은 칸을 만드는 분할은 막지 않는다(R11 — 사용자 결정).
        // ADR-0227
        let geo = geometry::compute(&v.layout);
        if let Some(r) = geo.slots.iter().find(|s| s.slot_id == slot_id) {
            if !has_area(r) {
                return Err(LayoutError::SplitTooDeep);
            }
            let (lo, hi) = match dir {
                SplitDir::LeftRight => (r.x0, r.x1),
                SplitDir::TopBottom => (r.y0, r.y1),
            };
            let at = geometry::boundary(lo, hi, tree::SPLIT_RATIO);
            if !(lo < at && at < hi) {
                return Err(LayoutError::SplitTooDeep);
            }
        }
        match tree::split_in_tree(&mut v.layout, slot_id, dir) {
            Some(new_id) => {
                v.focused_slot_id = Some(new_id);
                self.bump_version();
                Ok(new_id)
            }
            None => Err(LayoutError::SlotNotFound(slot_id)),
        }
    }

    // 분할 `split_id` 의 비율(a 쪽 = 왼쪽/위 칸의 몫 — ADR-0140)을 쓴다. 판정 순서가 계약이다:
    // ① 비유한 → `InvalidRatio` — 이 관리자가 모든 호출자의 단일 관문이다(Rust 에서 직접 부르는 쪽이나
    //    JSON 이 아닌 전송은 버스·IPC 역직렬화의 거름을 안 거친다).
    // ② 없는 view → `ViewNotFound`, 그 view 에 없는 분할 → `SplitNotFound`(무변경).
    // ③ `[RATIO_MIN, RATIO_MAX]` 클램프.
    // ④ 소유 창의 캔버스·지표를 둘 다 알면 분할의 두 쪽(서브트리 통째)이 각각 `min_pane_px` 이상이 되게 한 번 더
    //    클램프한다(거절하지 않는다). 그 범위가 비면 지금 값 그대로 `TooSmall`. 화면 드래그의 쌍둥이
    //    `src/components/layout/splitPreview.ts` 의 `ratioRange` 가 같은 식·같은 입력(보고한 정수 캔버스 ×
    //    스냅샷 상자)으로 범위를 구한다 — 한쪽만 바꾸면 확정값이 여기서 다시 잘려 뗄 때 튄다(TRD §2c/§2f).
    //    둘 중 하나라도 모르면(보고 전·팝아웃 직후) ③ 만 걸려 `min_pane_px` 보다 얇은 칸이 나올 수 있다 —
    //    ③ 의 한계를 작게 둔 대가로 받아들였다(ADR-0260).
    // ⑤ 그 값을 쓰면 **새로** 면적 0 이 되는 칸이 있으면 `TooSmall` — 조상 비율이 깊은 자손을 무너뜨리는
    //    경로를 막는다. 쓰기 전부터 면적 0 이던 칸(쓰기 경로로는 안 생긴다)은 비교에서 뺀다 — 안 빼면 그런
    //    칸 하나가 이 뷰의 모든 비율 쓰기를 잠근다.
    // ⑥ 지금 값과 같으면 `Unchanged`(version 불변). ⑦ 아니면 쓰고 version +1 → `Applied`.
    // ADR-0227
    pub fn set_split_ratio(
        &mut self,
        view_id: ViewId,
        split_id: Uuid,
        ratio: f64,
    ) -> Result<SplitRatioResult, LayoutError> {
        if !ratio.is_finite() {
            return Err(LayoutError::InvalidRatio(ratio.to_string()));
        }
        let px = self.px_context(view_id)?;
        let v = self
            .views
            .get(&view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        let current =
            tree::split_ratio(&v.layout, split_id).ok_or(LayoutError::SplitNotFound(split_id))?;
        let before = geometry::compute(&v.layout);
        let rect = before
            .splits
            .iter()
            .find(|s| s.split_id == split_id)
            .ok_or(LayoutError::SplitNotFound(split_id))?;
        let untouched = SplitRatioResult {
            ratio: current,
            outcome: SplitRatioOutcome::TooSmall,
        };

        let mut wanted = tree::clamp_ratio(ratio);
        if let Some((canvas, metrics)) = px {
            let (canvas_len, extent) = match rect.dir {
                SplitDir::LeftRight => (canvas.w, rect.x1 - rect.x0),
                SplitDir::TopBottom => (canvas.h, rect.y1 - rect.y0),
            };
            let len = f64::from(canvas_len) * extent;
            let edge = f64::from(metrics.min_pane_px) / len;
            let lo = tree::RATIO_MIN.max(edge);
            let hi = tree::RATIO_MAX.min(1.0 - edge);
            // `f64::clamp` 는 뒤집힌 범위에서 패닉한다. 폭 0 상자(`len == 0`)는 `edge = ∞` 라 여기서 빈 범위로
            // 빠진다.
            if lo > hi {
                return Ok(untouched);
            }
            wanted = wanted.clamp(lo, hi);
        }

        let mut candidate = v.layout.clone();
        tree::set_ratio_in_tree(&mut candidate, split_id, wanted);
        let after = geometry::compute(&candidate);
        // 비율만 바뀌어 잎 집합·전위 순이 같으므로 자리끼리 맞댄다.
        let collapses = before
            .slots
            .iter()
            .zip(&after.slots)
            .any(|(b, a)| has_area(b) && !has_area(a));
        if collapses {
            return Ok(untouched);
        }
        if wanted == current {
            return Ok(SplitRatioResult {
                ratio: wanted,
                outcome: SplitRatioOutcome::Unchanged,
            });
        }
        self.view_mut(view_id)?.layout = candidate;
        self.bump_version();
        Ok(SplitRatioResult {
            ratio: wanted,
            outcome: SplitRatioOutcome::Applied,
        })
    }

    pub fn list_splits(&self, view_id: ViewId) -> Result<Vec<tree::SplitInfo>, LayoutError> {
        let v = self
            .views
            .get(&view_id)
            .ok_or(LayoutError::ViewNotFound(view_id))?;
        Ok(tree::list_splits(&v.layout))
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
        self.forget_unknown(view_id, slot_id);
        self.bump_version();
        Ok(())
    }

    // view 안 slot_id 슬롯에 agent_id(참조 문자열) 배정. ★데몬에 실재 검증 안 함(ADR-0035/0006).
    // ★덮어쓰기 시맨틱★: 점유 슬롯도 무조건 교체 — 점유 방어는 `resolve_spawn_slot` 층(ADR-0059).
    pub fn assign_agent(
        &mut self,
        view_id: Uuid,
        slot_id: Uuid,
        agent_id: String,
    ) -> Result<(), LayoutError> {
        self.write_slot_content(view_id, slot_id, SlotContent::Agent { agent_id })?;
        self.bump_version();
        Ok(())
    }

    // view 안 slot_id 슬롯의 콘텐츠를 `content`(SlotContent 제네릭)로 교체한다(ADR-0063 배치 제어 표면).
    // assign_agent 의 미러이나 에이전트 전용이 아니라 유니온 전체(Empty/Agent/AgentList/PresetPalette/Usage)를
    // 받는다 — 트리(에이전트)·팔레트를 슬롯에 배치하는 §5 LLM/사람 공용 경로. ★덮어쓰기 시맨틱(assign 과
    // 동형)★: 점유 슬롯도 무조건 교체(점유 방어는 없음 — 배치 command 는 명시적 교체 의도).
    pub fn set_slot_content(
        &mut self,
        view_id: Uuid,
        slot_id: Uuid,
        content: SlotContent,
    ) -> Result<(), LayoutError> {
        self.write_slot_content(view_id, slot_id, content)?;
        self.bump_version();
        Ok(())
    }

    /// 슬롯 `slot_id` 를 이 판이 모르는 내용 `raw` 를 쥔 슬롯으로 세운다 — 메모리 내용은 `Empty` 로 쓰고 원문은
    /// 곁표에 둔다. 복원(영속 DTO 의 `PersistedContent::Unknown`)이 부르는 문이다 — IPC·버스 명령은 여기 닿지
    /// 않는다(모르는 종류는 역직렬화에서 거절된다 — TRD S21-storage §6-2). 없는 view·slot 은 `Err` · 무변경.
    pub(crate) fn set_unknown_content(
        &mut self,
        view_id: ViewId,
        slot_id: Uuid,
        raw: Map<String, Value>,
    ) -> Result<(), LayoutError> {
        self.write_slot_content(view_id, slot_id, SlotContent::Empty)?;
        self.remember_unknown(view_id, slot_id, raw);
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
    // 방지, phase C 에서 삽입). 소스 슬롯은 안 건드림(phase C 에서 close). 거절은 없는 view/slot 뿐이다.
    //
    // ★ADR-0064 — 콘텐츠 일반화★: 모든 슬롯 종류가 팝업 가능(불변식 5 — 다중 참조 허용은 Agent 뿐 아니라
    // 콘텐츠 일반에 적용). 반환한 SlotContent 로 호출자가 agent 구독 마이그레이션(still-ours close 가드)이
    // 필요한지(= Agent 인지)를 판별한다.
    // ★빈 슬롯(Empty)도 옮긴다 — 거절을 되살리지 말 것(사용자 결정)★: 결과는 빈 칸 하나짜리 새 탭이다.
    // 모르는 내용을 쥔 슬롯이면 원문도 새 슬롯으로 따라간다 — 옮기기는 교체가 아니다. 소스 쪽 항목은 phase C 의
    // `close_slot` 이 거둔다.
    // ADR-0064
    // ADR-0228
    pub fn prepare_detached_view(
        &mut self,
        src_view: ViewId,
        src_slot: Uuid,
        name: String,
    ) -> Result<(ViewId, SlotContent), LayoutError> {
        let content = self.slot_content(src_view, src_slot)?;
        let carried = self.unknown_content(src_view, src_slot).cloned();
        let id = self.make_view(name);
        let slot = {
            let v = self.views.get(&id).expect("방금 만든 View");
            tree::first_slot_id(&v.layout)
        };
        // ADR-0064: 에이전트 전용 배정이 아니라 제네릭 쓰기로 콘텐츠 종류 전체를 옮긴다.
        let _ = self.write_slot_content(id, slot, content.clone());
        if let Some(raw) = carried {
            self.remember_unknown(id, slot, raw);
        }
        self.bump_version();
        Ok((id, content))
    }

    // ★phase A 롤백★: prepare_detached_view 로 만든 임시 View 를 제거(창 삽입 전이라 tabs 갱신 불필요).
    pub fn drop_detached_view(&mut self, view: ViewId) {
        self.remove_view(view);
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
        self.windows
            .insert(label.to_string(), WindowTabs::new_popout(view));
        self.bump_version();
        Ok(())
    }

    // ── 런타임 복원 수락(TRD S21-storage §6-7 ③) ─────────────────────────────

    /// 창 묶음을 `restored`(`from_persisted` 의 결과)로 갈아끼운다 — main 은 탭 · 활성 · 속성만 바꾸고(측정값 ·
    /// 영속 id 는 남는다), 옛 팝아웃은 항목째 지우고, `restored` 의 팝아웃을 그 곁표 원문과 함께 들인다. 돌려주는
    /// 것 = 지운 옛 팝아웃 label(정렬) — 그 OS 창은 부르는 쪽이 락 밖에서 거둔다.
    ///
    /// - `version` · `attrs_rev` 는 지금 값에서 하나씩 오른다 — `restored` 의 번호는 버린다. 기록기는 같은지만 보므로
    ///   되감긴 번호가 마지막으로 본 값과 같아지면 그 사이의 변경을 놓친다.
    /// - owner 없는 View(창 만들기 중인 `prepare_detached_view` 의 임시 View)는 그대로 둔다 — 그 일의 phase C 가
    ///   붙이거나 거둔다.
    /// - `Err` 면 아무것도 바뀌지 않았다: `restored` 에 main 이 없다 · 팝아웃 label 이 지금 모델의 창과 겹친다 ·
    ///   View id 가 남는 View 와 겹친다.
    pub(crate) fn adopt_restored(
        &mut self,
        restored: ViewManager,
    ) -> Result<Vec<WindowLabel>, LayoutError> {
        let ViewManager {
            views,
            view_owner,
            mut windows,
            version: _,
            attrs_rev: _,
            unknown_content,
        } = restored;
        let main = windows
            .remove(MAIN_WINDOW_LABEL)
            .ok_or_else(|| LayoutError::WindowNotFound(MAIN_WINDOW_LABEL.to_string()))?;
        if !self.windows.contains_key(MAIN_WINDOW_LABEL) {
            return Err(LayoutError::WindowNotFound(MAIN_WINDOW_LABEL.to_string()));
        }
        if let Some(label) = windows.keys().find(|l| self.windows.contains_key(*l)) {
            return Err(LayoutError::WindowNotFound(label.clone()));
        }
        let owned: HashSet<ViewId> = self
            .windows
            .values()
            .flat_map(|w| w.tabs.iter().copied())
            .collect();
        if let Some(&view) = views
            .keys()
            .find(|v| self.views.contains_key(*v) && !owned.contains(*v))
        {
            return Err(LayoutError::ViewNotFound(view));
        }

        for view in owned {
            self.remove_view(view);
        }
        let mut removed: Vec<WindowLabel> = self
            .windows
            .keys()
            .filter(|l| l.as_str() != MAIN_WINDOW_LABEL)
            .cloned()
            .collect();
        removed.sort();
        for label in &removed {
            self.windows.remove(label);
        }
        let live = self
            .windows
            .get_mut(MAIN_WINDOW_LABEL)
            .expect("위에서 확인했다");
        live.tabs = main.tabs;
        live.active = main.active;
        live.attrs = main.attrs;
        live.placement_memo = PlacementMemo::default();
        self.views.extend(views);
        self.view_owner.extend(view_owner);
        self.unknown_content.extend(unknown_content);
        self.windows.extend(windows);
        self.bump_version();
        self.attrs_rev += 1;
        Ok(removed)
    }

    // ── 창 속성(TRD S21-storage §6-3) ───────────────────────────────────────
    //
    // ★version 이 아니라 attrs_rev 를 올린다★ — 바뀌었을 때만. 값은 부르는 쪽이 락 밖에서 읽어 넘긴다(OS 호출 0).
    // 트리 창(`agent-tree`)은 이 모델 밖이라 `WindowNotFound` — 그 창은 `state::tree_attrs` 로 간다.

    pub fn window_attrs(&self, label: &str) -> Result<WindowAttrs, LayoutError> {
        self.windows
            .get(label)
            .map(|w| w.attrs)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))
    }

    pub fn attrs_rev(&self) -> u64 {
        self.attrs_rev
    }

    /// `None` = 창별 테마를 지운다(전역 테마를 따른다).
    pub fn set_window_theme(
        &mut self,
        label: &str,
        theme: Option<UiTheme>,
    ) -> Result<(), LayoutError> {
        self.update_attrs(label, |attrs| attrs.set_theme(theme))
    }

    /// `Moved` · `Resized` 때 읽은 게터 한 벌을 적는다 — 규칙은 [`WindowAttrs::observe`].
    pub fn observe_window_placement(
        &mut self,
        label: &str,
        seen: WindowPlacement,
    ) -> Result<(), LayoutError> {
        let window = self
            .windows
            .get_mut(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        if window.attrs.observe(seen, &mut window.placement_memo) {
            self.attrs_rev += 1;
        }
        Ok(())
    }

    fn update_attrs(
        &mut self,
        label: &str,
        change: impl FnOnce(&mut WindowAttrs) -> bool,
    ) -> Result<(), LayoutError> {
        let window = self
            .windows
            .get_mut(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        if change(&mut window.attrs) {
            self.attrs_rev += 1;
        }
        Ok(())
    }

    // ── 측정 보고(웹뷰 → 셸) ────────────────────────────────────────────────
    //
    // ★version 을 올리지 않는다★ — 측정이지 레이아웃 변경이 아니다. 스냅샷에도 안 실리고 알림도 없다
    // (적용 서비스가 포트를 안 받는다). 셸 계산이 읽을 때 그 자리에서 본다.
    // ADR-0227

    // `w`·`h` 중 하나라도 0 이면 무시하고 직전 값을 유지한다(`Ok`) — 0 크기 관측은 실재하고, 저장하면 그
    // 창의 모든 칸이 0 px 가 된다.
    pub fn set_window_canvas(&mut self, label: &str, w: u32, h: u32) -> Result<(), LayoutError> {
        let wt = self
            .windows
            .get_mut(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        if w == 0 || h == 0 {
            return Ok(());
        }
        wt.canvas = Some(CanvasPx { w, h });
        Ok(())
    }

    // 범위 밖이면 `InvalidMetrics` 로 거절하고 직전 값을 유지한다(범위 = `UiMetrics` 문서).
    pub fn set_ui_metrics(&mut self, label: &str, m: UiMetrics) -> Result<(), LayoutError> {
        let wt = self
            .windows
            .get_mut(label)
            .ok_or_else(|| LayoutError::WindowNotFound(label.to_string()))?;
        check_metrics(&m)?;
        wt.metrics = Some(m);
        Ok(())
    }

    // view 를 소유한 창의 캔버스·지표. 둘 중 하나라도 모르면 `Ok(None)`.
    // 소유 창이 없는 View(`prepare_detached_view` 의 임시 View)도 `Ok(None)` 이다.
    // ADR-0227
    pub(crate) fn px_context(
        &self,
        view: ViewId,
    ) -> Result<Option<(CanvasPx, UiMetrics)>, LayoutError> {
        if !self.views.contains_key(&view) {
            return Err(LayoutError::ViewNotFound(view));
        }
        Ok(self
            .view_owner
            .get(&view)
            .and_then(|label| self.windows.get(label))
            .and_then(|wt| wt.canvas.zip(wt.metrics)))
    }

    // 칸 `slot` 의 px 사각형. `frame` 은 `geometry::frame_px`, `content` 는 `geometry::content_rect` 규칙이다
    // (구분선 두께는 빼지 않는다 — 구분선은 공간을 먹지 않는 오버레이다).
    // 캔버스나 지표를 모르면 `Ok(None)` — 지표 없이 내면 content 가 틀린 값이 된다. 없는 view·slot 은 그보다
    // 먼저 `Err` 다. 숨은 탭도 같은 창 캔버스로 계산한다.
    // ADR-0227
    pub fn slot_px(&self, view: ViewId, slot: Uuid) -> Result<Option<SlotPx>, LayoutError> {
        let v = self
            .views
            .get(&view)
            .ok_or(LayoutError::ViewNotFound(view))?;
        if !tree::contains_slot(&v.layout, slot) {
            return Err(LayoutError::SlotNotFound(slot));
        }
        let Some((canvas, metrics)) = self.px_context(view)? else {
            return Ok(None);
        };
        let geo = geometry::compute(&v.layout);
        let rect = geo
            .slots
            .iter()
            .find(|r| r.slot_id == slot)
            .ok_or(LayoutError::SlotNotFound(slot))?;
        let frame = geometry::frame_px(canvas.w, canvas.h, rect);
        Ok(Some(SlotPx {
            frame,
            content: geometry::content_rect(&frame, &metrics.frame_insets),
        }))
    }
}

fn has_area(r: &SlotRect) -> bool {
    r.x0 < r.x1 && r.y0 < r.y1
}

// `RangeInclusive::contains` 는 NaN 에 거짓이다 — 그래서 NaN·±∞ 도 이 한 검사에서 함께 걸린다.
fn check_metrics(m: &UiMetrics) -> Result<(), LayoutError> {
    let i = &m.frame_insets;
    for (name, v) in [("t", i.t), ("r", i.r), ("b", i.b), ("l", i.l)] {
        if !(0.0..=INSET_MAX_PX).contains(&v) {
            return Err(LayoutError::InvalidMetrics(format!(
                "frame_insets.{name}={v} — 유한하고 0~{INSET_MAX_PX} 이어야 한다"
            )));
        }
    }
    if !(1..=MIN_PANE_PX_MAX).contains(&m.min_pane_px) {
        return Err(LayoutError::InvalidMetrics(format!(
            "min_pane_px={} — 1~{MIN_PANE_PX_MAX} 이어야 한다",
            m.min_pane_px
        )));
    }
    Ok(())
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
    #[error("view {0} 없음")]
    ViewNotFound(Uuid),
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
        for (vid, slots) in &mgr.unknown_content {
            let v = mgr
                .views
                .get(vid)
                .unwrap_or_else(|| panic!("곁표가 없는 View {vid} 를 가리킴"));
            assert!(!slots.is_empty(), "곁표: View {vid} 의 빈 칸 남음");
            for sid in slots.keys() {
                assert_eq!(
                    tree::find_slot(&v.layout, *sid),
                    Some(&SlotContent::Empty),
                    "곁표: View {vid} 의 슬롯 {sid} 가 없거나 메모리 내용이 Empty 가 아님"
                );
            }
        }
    }

    fn raw(kind: &str) -> Map<String, Value> {
        serde_json::json!({ "type": kind, "from": "a newer build" })
            .as_object()
            .unwrap()
            .clone()
    }

    // main 활성 탭의 root 를 모르는 내용 슬롯으로 세운다.
    fn foreign_root(mgr: &mut ViewManager) -> (ViewId, Uuid) {
        let v = main_active(mgr);
        let s = first_slot_of(mgr, v);
        mgr.set_unknown_content(v, s, raw("from_the_future"))
            .unwrap();
        (v, s)
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
    fn new_main_default_layout_is_single_empty_slot_focused() {
        let mgr = ViewManager::new();
        let v = mgr.views.get(&main_active(&mgr)).unwrap();
        let LayoutNode::Slot {
            id,
            content: SlotContent::Empty,
        } = &v.layout
        else {
            panic!(
                "부팅 기본은 단일 빈 슬롯이어야 함(트리 없음): {:?}",
                v.layout
            );
        };
        assert_eq!(v.focused_slot_id, Some(*id), "그 빈 슬롯에 포커스");
    }

    #[test]
    fn boot_view_has_the_same_shape_as_any_new_tab() {
        let mut mgr = ViewManager::new();
        let boot = main_active(&mgr);
        let t = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        for view in [boot, t] {
            let v = mgr.views.get(&view).unwrap();
            assert!(matches!(
                v.layout,
                LayoutNode::Slot {
                    content: SlotContent::Empty,
                    ..
                }
            ));
            assert_eq!(v.focused_slot_id, Some(first_slot_of(&mgr, view)));
        }
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
        let new_id = mgr.split_slot(view_id, slot, SplitDir::LeftRight).unwrap();
        let v = mgr.views.get(&view_id).unwrap();
        assert!(matches!(v.layout, LayoutNode::Split { .. }));
        assert_eq!(v.focused_slot_id, Some(new_id));
        assert_invariants(&mgr);
    }

    #[test]
    fn split_invalid_view_is_err() {
        let mut mgr = ViewManager::new();
        assert!(matches!(
            mgr.split_slot(Uuid::new_v4(), Uuid::new_v4(), SplitDir::LeftRight)
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
            mgr.split_slot(view_id, Uuid::new_v4(), SplitDir::TopBottom)
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
        let new_id = mgr.split_slot(view_id, slot, SplitDir::LeftRight).unwrap();
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
        let left = first_slot_of(&mgr, view_id);
        let right = mgr.split_slot(view_id, left, SplitDir::LeftRight).unwrap();
        assert_eq!(
            mgr.views.get(&view_id).unwrap().focused_slot_id,
            Some(right)
        );
        let ver = mgr.version;
        mgr.set_focused_slot(view_id, left).unwrap();
        assert_eq!(
            mgr.views.get(&view_id).unwrap().focused_slot_id,
            Some(left),
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
        mgr.split_slot(view_id, slot, SplitDir::LeftRight).unwrap();
        let snap = mgr.snapshot(view_id).unwrap();
        assert_eq!(snap.view_id, view_id);
        assert_eq!(snap.version, mgr.version);
        assert!(matches!(snap.layout, LayoutNode::Split { .. }));
    }

    #[test]
    fn snapshot_carries_geometry_and_ratio_bounds() {
        let mut mgr = ViewManager::new();
        let view_id = main_active(&mgr);
        let slot = first_slot_of(&mgr, view_id);
        let right = mgr.split_slot(view_id, slot, SplitDir::LeftRight).unwrap();
        mgr.split_slot(view_id, right, SplitDir::TopBottom).unwrap();
        let snap = mgr.snapshot(view_id).unwrap();
        let geo = geometry::compute(&snap.layout);
        assert_eq!(snap.slot_rects, geo.slots);
        assert_eq!(snap.split_rects, geo.splits);
        assert_eq!(snap.slot_rects.len(), 3);
        assert_eq!(snap.split_rects.len(), 2);
        assert_eq!(snap.ratio_min, tree::RATIO_MIN);
        assert_eq!(snap.ratio_max, tree::RATIO_MAX);
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

    // ADR-0228
    #[test]
    fn prepare_detached_view_moves_an_empty_slot() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let slot = first_slot_of(&mgr, src);
        mgr.set_slot_content(src, slot, SlotContent::Empty).unwrap();
        let (tmp, content) = mgr
            .prepare_detached_view(src, slot, "Popup".into())
            .unwrap();
        assert_eq!(content, SlotContent::Empty, "반환 콘텐츠 = Empty");
        let tslot = first_slot_of(&mgr, tmp);
        assert_eq!(
            tree::find_slot(&mgr.views.get(&tmp).unwrap().layout, tslot).unwrap(),
            &SlotContent::Empty
        );
        assert!(
            mgr.view_owner.get(&tmp).is_none(),
            "phase A 는 view_owner 미배정"
        );
        // phase C 에서 close.
        assert_eq!(
            tree::find_slot(&mgr.views.get(&src).unwrap().layout, slot).unwrap(),
            &SlotContent::Empty
        );
    }

    #[test]
    fn prepare_detached_view_missing_slot_or_view_is_err_without_a_view() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let views_before = mgr.views.len();
        let ghost = Uuid::new_v4();
        assert_eq!(
            mgr.prepare_detached_view(src, ghost, "P".into()),
            Err(LayoutError::SlotNotFound(ghost))
        );
        assert_eq!(
            mgr.prepare_detached_view(ghost, ghost, "P".into()),
            Err(LayoutError::ViewNotFound(ghost))
        );
        assert_eq!(
            mgr.views.len(),
            views_before,
            "거절은 임시 View 를 남기지 않는다"
        );
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
        assert_eq!(
            mgr.resolve_spawn_slot(v, None).unwrap(),
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
        assert_eq!(
            mgr.resolve_spawn_slot(v, None),
            Err(SpawnSlotError::NoEmptySlot)
        );
    }

    #[test]
    fn resolve_none_on_split_tab_picks_first_empty_not_leftmost() {
        // ★USER DECISION 2b 회귀★: 2b 이전엔 leftmost-only 라 SlotOccupied 였다.
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        let right = mgr.split_slot(v, root, SplitDir::LeftRight).unwrap();
        mgr.assign_agent(v, root, "occupied".into()).unwrap();
        assert_eq!(
            mgr.resolve_spawn_slot(v, None).unwrap(),
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
        let right = mgr.split_slot(v, root, SplitDir::LeftRight).unwrap();
        mgr.assign_agent(v, root, "a".into()).unwrap();
        mgr.assign_agent(v, right, "b".into()).unwrap();
        assert_eq!(
            mgr.resolve_spawn_slot(v, None),
            Err(SpawnSlotError::NoEmptySlot)
        );
    }

    #[test]
    fn resolve_none_after_create_tab_targets_fresh_root() {
        // 2b 에서도 유지.
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        assert_eq!(mgr.resolve_spawn_slot(v, None).unwrap(), root);
    }

    #[test]
    fn resolve_then_assign_holds_invariants() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let target = mgr.resolve_spawn_slot(v, None).unwrap();
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
        let new_slot = mgr.split_slot(v, root, SplitDir::LeftRight).unwrap();
        assert_eq!(
            mgr.resolve_spawn_slot(v, Some(new_slot)).unwrap(),
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
        assert_eq!(
            mgr.resolve_spawn_slot(v, Some(root)),
            Err(SpawnSlotError::SlotOccupied(root))
        );
    }

    #[test]
    fn resolve_some_usage_slot_is_occupied() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let root = first_slot_of(&mgr, v);
        mgr.set_slot_content(
            v,
            root,
            SlotContent::Usage {
                show_claude: false,
                show_codex: false,
            },
        )
        .unwrap();
        assert_eq!(
            mgr.resolve_spawn_slot(v, Some(root)),
            Err(SpawnSlotError::SlotOccupied(root))
        );
        assert_eq!(
            mgr.resolve_spawn_slot(v, None),
            Err(SpawnSlotError::NoEmptySlot)
        );
    }

    #[test]
    fn resolve_some_missing_slot_errors() {
        let mut mgr = ViewManager::new();
        let v = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let bogus = Uuid::new_v4();
        assert_eq!(
            mgr.resolve_spawn_slot(v, Some(bogus)),
            Err(SpawnSlotError::SlotNotFound(bogus))
        );
    }

    #[test]
    fn resolve_on_a_missing_view_is_view_not_found() {
        let mgr = ViewManager::new();
        let ghost = Uuid::new_v4();
        assert_eq!(
            mgr.resolve_spawn_slot(ghost, None),
            Err(SpawnSlotError::ViewNotFound(ghost))
        );
    }

    // ── 모르는 슬롯 내용 곁표 (TRD S21-storage §6-2 · §12 R6) ────────────────────

    #[test]
    fn an_unknown_content_slot_reads_empty_but_is_not_free() {
        let mut mgr = ViewManager::new();
        let before = mgr.version;
        let (v, s) = foreign_root(&mut mgr);
        assert!(mgr.version > before, "곁표에 세우는 것도 변경이다");
        assert_eq!(mgr.slot_content(v, s).unwrap(), SlotContent::Empty);
        assert_eq!(mgr.slot_is_free(v, s), Ok(false));
        assert_eq!(mgr.unknown_content(v, s), Some(&raw("from_the_future")));

        let right = mgr.split_slot(v, s, SplitDir::LeftRight).unwrap();
        assert_eq!(mgr.slot_is_free(v, right), Ok(true));
        let ghost = Uuid::new_v4();
        assert_eq!(
            mgr.slot_is_free(v, ghost),
            Err(LayoutError::SlotNotFound(ghost))
        );
        assert_eq!(
            mgr.slot_is_free(ghost, s),
            Err(LayoutError::ViewNotFound(ghost))
        );
        assert_invariants(&mgr);
    }

    #[test]
    fn set_unknown_content_writes_empty_over_an_occupied_slot() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let s = first_slot_of(&mgr, v);
        mgr.assign_agent(v, s, "a".into()).unwrap();
        mgr.set_unknown_content(v, s, raw("x")).unwrap();
        assert_eq!(mgr.slot_content(v, s).unwrap(), SlotContent::Empty);
        assert_eq!(mgr.slot_is_free(v, s), Ok(false));
        assert_invariants(&mgr);
    }

    #[test]
    fn set_unknown_content_on_a_missing_view_or_slot_changes_nothing() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let ghost = Uuid::new_v4();
        let before = mgr.version;
        assert_eq!(
            mgr.set_unknown_content(v, ghost, raw("x")),
            Err(LayoutError::SlotNotFound(ghost))
        );
        assert_eq!(
            mgr.set_unknown_content(ghost, ghost, raw("x")),
            Err(LayoutError::ViewNotFound(ghost))
        );
        assert_eq!(mgr.version, before);
        assert!(mgr.unknown_content.is_empty());
    }

    #[test]
    fn auto_spawn_skips_an_unknown_content_slot_for_another_empty_one() {
        let mut mgr = ViewManager::new();
        let (v, root) = foreign_root(&mut mgr);
        let right = mgr.split_slot(v, root, SplitDir::LeftRight).unwrap();
        assert_eq!(
            mgr.unknown_content(v, root),
            Some(&raw("from_the_future")),
            "나누기는 원래 슬롯을 a 쪽에 그대로 둔다 — 곁표 항목도 남는다"
        );
        assert_eq!(mgr.resolve_spawn_slot(v, None), Ok(right));
        assert_eq!(
            mgr.resolve_spawn_slot(v, Some(root)),
            Err(SpawnSlotError::SlotOccupied(root))
        );
        assert_eq!(mgr.resolve_spawn_slot(v, Some(right)), Ok(right));
    }

    #[test]
    fn auto_spawn_with_no_free_slot_besides_unknown_content_is_no_empty_slot() {
        let mut mgr = ViewManager::new();
        let (v, root) = foreign_root(&mut mgr);
        assert_eq!(
            mgr.resolve_spawn_slot(v, None),
            Err(SpawnSlotError::NoEmptySlot)
        );
        let right = mgr.split_slot(v, root, SplitDir::LeftRight).unwrap();
        mgr.assign_agent(v, right, "a".into()).unwrap();
        assert_eq!(
            mgr.resolve_spawn_slot(v, None),
            Err(SpawnSlotError::NoEmptySlot)
        );
    }

    #[test]
    fn every_content_write_clears_the_unknown_entry() {
        type Write = fn(&mut ViewManager, ViewId, Uuid);
        let writes: [(&str, Write); 6] = [
            ("assign_agent", |m, v, s| {
                m.assign_agent(v, s, "a".into()).unwrap()
            }),
            ("set_slot_content(empty)", |m, v, s| {
                m.set_slot_content(v, s, SlotContent::Empty).unwrap()
            }),
            ("set_slot_content(agent)", |m, v, s| {
                m.set_slot_content(
                    v,
                    s,
                    SlotContent::Agent {
                        agent_id: "a".into(),
                    },
                )
                .unwrap()
            }),
            ("set_slot_content(agent_list)", |m, v, s| {
                m.set_slot_content(v, s, SlotContent::AgentList).unwrap()
            }),
            ("set_slot_content(preset_palette)", |m, v, s| {
                m.set_slot_content(v, s, SlotContent::PresetPalette)
                    .unwrap()
            }),
            ("set_slot_content(usage)", |m, v, s| {
                m.set_slot_content(
                    v,
                    s,
                    SlotContent::Usage {
                        show_claude: true,
                        show_codex: true,
                    },
                )
                .unwrap()
            }),
        ];
        for (name, write) in writes {
            let mut mgr = ViewManager::new();
            let (v, s) = foreign_root(&mut mgr);
            write(&mut mgr, v, s);
            assert_eq!(mgr.unknown_content(v, s), None, "{name}");
            assert!(mgr.snapshot(v).unwrap().foreign_slots.is_empty(), "{name}");
            assert_invariants(&mgr);
        }

        let mut mgr = ViewManager::new();
        let (v, s) = foreign_root(&mut mgr);
        mgr.set_slot_content(v, s, SlotContent::Empty).unwrap();
        assert_eq!(
            mgr.slot_is_free(v, s),
            Ok(true),
            "빈 내용을 명시적으로 쓰면 원문을 버리고 빈 슬롯이 된다"
        );
    }

    #[test]
    fn closing_the_slot_clears_the_unknown_entry() {
        // 형제가 있는 슬롯 — 형제가 승격된다.
        let mut mgr = ViewManager::new();
        let (v, root) = foreign_root(&mut mgr);
        let right = mgr.split_slot(v, root, SplitDir::LeftRight).unwrap();
        mgr.close_slot(v, root).unwrap();
        assert!(mgr.unknown_content.is_empty());
        assert!(mgr.snapshot(v).unwrap().foreign_slots.is_empty());
        assert_eq!(mgr.resolve_spawn_slot(v, None), Ok(right));
        assert_invariants(&mgr);

        // 하나뿐인 슬롯 — 새 빈 슬롯으로 리셋된다.
        let mut mgr = ViewManager::new();
        let (v, root) = foreign_root(&mut mgr);
        mgr.close_slot(v, root).unwrap();
        assert!(mgr.unknown_content.is_empty());
        let fresh = first_slot_of(&mgr, v);
        assert_eq!(mgr.slot_is_free(v, fresh), Ok(true));
        assert_invariants(&mgr);
    }

    #[test]
    fn closing_a_tab_or_window_drops_its_unknown_entries() {
        let mut mgr = ViewManager::new();
        let first = mgr.create_window("slot-popup-1").unwrap();
        let fs = first_slot_of(&mgr, first);
        mgr.set_unknown_content(first, fs, raw("x")).unwrap();
        let second = mgr.create_tab("slot-popup-1", None).unwrap();
        let ss = first_slot_of(&mgr, second);
        mgr.set_unknown_content(second, ss, raw("y")).unwrap();

        mgr.close_tab("slot-popup-1", second).unwrap();
        assert!(!mgr.unknown_content.contains_key(&second));
        assert_eq!(mgr.unknown_content(first, fs), Some(&raw("x")));
        mgr.close_window("slot-popup-1").unwrap();
        assert!(mgr.unknown_content.is_empty());

        // main 의 마지막 탭 — 빈 탭이 강제로 서고 닫힌 탭의 항목은 사라진다.
        let (v, _) = foreign_root(&mut mgr);
        mgr.close_tab(MAIN_WINDOW_LABEL, v).unwrap();
        assert!(mgr.unknown_content.is_empty());
        assert_invariants(&mgr);
    }

    #[test]
    fn snapshot_lists_exactly_the_unknown_content_slots_in_tree_order() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let s1 = first_slot_of(&mgr, v);
        let s2 = mgr.split_slot(v, s1, SplitDir::LeftRight).unwrap();
        let s3 = mgr.split_slot(v, s2, SplitDir::TopBottom).unwrap();
        let s4 = mgr.split_slot(v, s3, SplitDir::LeftRight).unwrap();
        assert_eq!(tree::slot_ids(&mgr.views[&v].layout), vec![s1, s2, s3, s4]);
        // 트리 순과 거꾸로 세운다 — 세운 순서가 아니라 트리 순으로 실려야 한다.
        mgr.set_unknown_content(v, s4, raw("b")).unwrap();
        mgr.set_unknown_content(v, s1, raw("a")).unwrap();
        mgr.assign_agent(v, s2, "agent".into()).unwrap();

        assert_eq!(mgr.snapshot(v).unwrap().foreign_slots, vec![s1, s4]);
        let other = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        assert!(
            mgr.snapshot(other).unwrap().foreign_slots.is_empty(),
            "다른 탭에 새지 않는다"
        );
        assert_invariants(&mgr);
    }

    #[test]
    fn snapshot_after_unrelated_changes_still_lists_the_unknown_slot() {
        let mut mgr = ViewManager::new();
        let (v, root) = foreign_root(&mut mgr);
        let right = mgr.split_slot(v, root, SplitDir::LeftRight).unwrap();
        mgr.assign_agent(v, right, "a".into()).unwrap();
        mgr.set_focused_slot(v, right).unwrap();
        mgr.rename_tab(v, "renamed".into()).unwrap();
        let split = tree::list_splits(&mgr.views[&v].layout)[0].id;
        mgr.set_split_ratio(v, split, 0.3).unwrap();
        let other = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        mgr.switch_tab(MAIN_WINDOW_LABEL, v).unwrap();
        mgr.close_tab(MAIN_WINDOW_LABEL, other).unwrap();

        let snap = mgr.snapshot(v).unwrap();
        assert_eq!(snap.foreign_slots, vec![root]);
        assert_eq!(snap.version, mgr.version);
        assert_eq!(
            mgr.unknown_content(v, root),
            Some(&raw("from_the_future")),
            "원문 그대로"
        );
        assert_invariants(&mgr);
    }

    #[test]
    fn moving_an_unknown_content_slot_carries_its_raw_content() {
        let mut mgr = ViewManager::new();
        let (src, slot) = foreign_root(&mut mgr);
        let (tmp, content) = mgr
            .prepare_detached_view(src, slot, "Popup".into())
            .unwrap();
        assert_eq!(content, SlotContent::Empty);
        let tslot = first_slot_of(&mgr, tmp);
        assert_eq!(
            mgr.unknown_content(tmp, tslot),
            Some(&raw("from_the_future"))
        );

        // phase C — 새 창에 붙이고 소스 슬롯을 닫는다.
        mgr.attach_view_as_new_window("slot-popup-1", tmp).unwrap();
        mgr.close_slot(src, slot).unwrap();
        assert_eq!(mgr.unknown_content(src, slot), None);
        assert_eq!(mgr.snapshot(tmp).unwrap().foreign_slots, vec![tslot]);
        assert_eq!(mgr.slot_is_free(tmp, tslot), Ok(false));
        assert_invariants(&mgr);
    }

    #[test]
    fn rolling_back_a_move_drops_only_the_carried_entry() {
        let mut mgr = ViewManager::new();
        let (src, slot) = foreign_root(&mut mgr);
        let (tmp, _content) = mgr
            .prepare_detached_view(src, slot, "Popup".into())
            .unwrap();
        mgr.drop_detached_view(tmp);
        assert!(!mgr.unknown_content.contains_key(&tmp));
        assert_eq!(
            mgr.unknown_content(src, slot),
            Some(&raw("from_the_future")),
            "소스는 그대로"
        );
        assert_invariants(&mgr);
    }

    // ── 창 신원 · 창 속성 (TRD S21-storage §6-3) ─────────────────────────────

    fn bounds(x: f64) -> WindowBounds {
        WindowBounds::new(x, 60.0, 1280.0, 800.0).unwrap()
    }

    fn normal_at(x: f64) -> WindowPlacement {
        WindowPlacement {
            minimized: false,
            maximized: false,
            bounds: Some(bounds(x)),
        }
    }

    #[test]
    fn main_is_main_and_each_popout_gets_its_own_uuid() {
        let mut mgr = ViewManager::new();
        assert_eq!(window(&mgr, MAIN_WINDOW_LABEL).window_id, "main");
        mgr.create_window("slot-popup-1").unwrap();
        let src = main_active(&mgr);
        let (tmp, _) = mgr
            .prepare_detached_view(src, first_slot_of(&mgr, src), "P".into())
            .unwrap();
        mgr.attach_view_as_new_window("slot-popup-2", tmp).unwrap();

        let first = window(&mgr, "slot-popup-1").window_id;
        let second = window(&mgr, "slot-popup-2").window_id;
        for id in [&first, &second] {
            assert_eq!(
                Uuid::parse_str(id).unwrap().to_string(),
                *id,
                "하이픈 소문자 UUID"
            );
        }
        assert_ne!(first, second);

        mgr.create_tab("slot-popup-1", None).unwrap();
        assert_eq!(
            window(&mgr, "slot-popup-1").window_id,
            first,
            "탭을 더해도 그대로"
        );
    }

    #[test]
    fn window_bounds_accept_only_finite_values_and_a_positive_size() {
        assert!(WindowBounds::new(-1920.0, -8.5, 0.5, 1e9).is_some());
        for (w, h) in [(0.0, 1.0), (1.0, 0.0), (-1.0, 1.0), (1.0, -0.5)] {
            assert_eq!(WindowBounds::new(0.0, 0.0, w, h), None, "{w}x{h}");
        }
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(WindowBounds::new(bad, 0.0, 1.0, 1.0), None);
            assert_eq!(WindowBounds::new(0.0, bad, 1.0, 1.0), None);
            assert_eq!(WindowBounds::new(0.0, 0.0, bad, 1.0), None);
            assert_eq!(WindowBounds::new(0.0, 0.0, 1.0, bad), None);
        }
        let b = bounds(-1920.0);
        assert_eq!((b.x(), b.y(), b.w(), b.h()), (-1920.0, 60.0, 1280.0, 800.0));
    }

    #[test]
    fn attribute_changes_bump_attrs_rev_and_not_version() {
        let mut mgr = ViewManager::new();
        let version = mgr.version;
        assert_eq!(mgr.attrs_rev(), 0);

        mgr.set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::Light))
            .unwrap();
        assert_eq!(mgr.attrs_rev(), 1);
        mgr.observe_window_placement(MAIN_WINDOW_LABEL, normal_at(80.0))
            .unwrap();
        assert_eq!(mgr.attrs_rev(), 2);
        assert_eq!(
            mgr.version, version,
            "창 속성은 레이아웃 번호를 올리지 않는다"
        );
        assert_eq!(
            mgr.window_attrs(MAIN_WINDOW_LABEL),
            Ok(WindowAttrs {
                theme: Some(UiTheme::Light),
                bounds: Some(bounds(80.0)),
                maximized: false,
            })
        );

        // 같은 값 = 변경 없음 — 번호도 그대로.
        mgr.set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::Light))
            .unwrap();
        mgr.observe_window_placement(MAIN_WINDOW_LABEL, normal_at(80.0))
            .unwrap();
        assert_eq!(mgr.attrs_rev(), 2);

        mgr.set_window_theme(MAIN_WINDOW_LABEL, None).unwrap();
        assert_eq!(mgr.attrs_rev(), 3);
        assert_eq!(mgr.window_attrs(MAIN_WINDOW_LABEL).unwrap().theme, None);
        assert_eq!(mgr.version, version);
    }

    #[test]
    fn attribute_setters_on_an_unknown_window_change_nothing() {
        let mut mgr = ViewManager::new();
        for label in ["agent-tree", "slot-popup-9"] {
            assert_eq!(
                mgr.set_window_theme(label, Some(UiTheme::Dark)),
                Err(LayoutError::WindowNotFound(label.into()))
            );
            assert_eq!(
                mgr.observe_window_placement(label, normal_at(0.0)),
                Err(LayoutError::WindowNotFound(label.into()))
            );
            assert_eq!(
                mgr.window_attrs(label),
                Err(LayoutError::WindowNotFound(label.into()))
            );
        }
        assert_eq!((mgr.attrs_rev(), mgr.version), (0, 0));
    }

    #[test]
    fn placement_records_bounds_only_in_the_normal_state() {
        let mut mgr = ViewManager::new();
        let attrs = |m: &ViewManager| m.window_attrs(MAIN_WINDOW_LABEL).unwrap();
        mgr.observe_window_placement(MAIN_WINDOW_LABEL, normal_at(80.0))
            .unwrap();

        // 최대화 — 표식만, 자리는 최대화 전 보통 자리 그대로.
        mgr.observe_window_placement(
            MAIN_WINDOW_LABEL,
            WindowPlacement {
                minimized: false,
                maximized: true,
                bounds: Some(bounds(-8.0)),
            },
        )
        .unwrap();
        assert_eq!(
            (attrs(&mgr).bounds, attrs(&mgr).maximized),
            (Some(bounds(80.0)), true)
        );

        // 최소화 — 아무것도 바꾸지 않는다(최대화에서 최소화해도 다음 복원은 최대화).
        let rev = mgr.attrs_rev();
        mgr.observe_window_placement(
            MAIN_WINDOW_LABEL,
            WindowPlacement {
                minimized: true,
                maximized: false,
                bounds: Some(bounds(-32000.0)),
            },
        )
        .unwrap();
        assert_eq!(mgr.attrs_rev(), rev);
        assert_eq!(
            (attrs(&mgr).bounds, attrs(&mgr).maximized),
            (Some(bounds(80.0)), true)
        );

        // 보통으로 돌아옴 — 표식을 내리고 새 자리를 적는다.
        mgr.observe_window_placement(MAIN_WINDOW_LABEL, normal_at(300.0))
            .unwrap();
        assert_eq!(
            (attrs(&mgr).bounds, attrs(&mgr).maximized),
            (Some(bounds(300.0)), false)
        );

        // 보통인데 자리를 못 읽음 — 앞 자리를 둔다.
        mgr.observe_window_placement(
            MAIN_WINDOW_LABEL,
            WindowPlacement {
                minimized: false,
                maximized: false,
                bounds: None,
            },
        )
        .unwrap();
        assert_eq!(attrs(&mgr).bounds, Some(bounds(300.0)));
    }

    fn reading(maximized: bool, x: f64) -> WindowPlacement {
        WindowPlacement {
            minimized: false,
            maximized,
            bounds: Some(bounds(x)),
        }
    }

    #[test]
    fn a_user_maximize_reported_first_as_a_normal_move_keeps_the_normal_rect() {
        let mut mgr = ViewManager::new();
        let state = |m: &ViewManager| {
            let attrs = m.window_attrs(MAIN_WINDOW_LABEL).unwrap();
            (attrs.bounds, attrs.maximized)
        };
        // 보통 A(80) → 최대화: `Moved` 가 최대화된 사각형(-8)을 표식 없이 → `Resized` 가 같은 사각형을 표식과 함께.
        for seen in [
            reading(false, 80.0),
            reading(false, -8.0),
            reading(true, -8.0),
        ] {
            mgr.observe_window_placement(MAIN_WINDOW_LABEL, seen)
                .unwrap();
        }
        assert_eq!(state(&mgr), (Some(bounds(80.0)), true));
        // 한 번만 되돌린다.
        mgr.observe_window_placement(MAIN_WINDOW_LABEL, reading(true, -8.0))
            .unwrap();
        assert_eq!(state(&mgr), (Some(bounds(80.0)), true));

        // 최대화 풀기: `Moved` 는 아직 표식이 선 채 → `Resized` 가 표식 없이 A 로.
        for seen in [reading(true, 80.0), reading(false, 80.0)] {
            mgr.observe_window_placement(MAIN_WINDOW_LABEL, seen)
                .unwrap();
        }
        assert_eq!(state(&mgr), (Some(bounds(80.0)), false));
    }

    #[test]
    fn a_maximize_that_sets_the_flag_first_keeps_the_last_normal_reading() {
        let mut mgr = ViewManager::new();
        // 빌더가 만든 자리(20) → 복원이 입힌 자리(80) → tao `maximize`(표식이 먼저 서 잘못 적힌 읽기가 없다).
        for seen in [
            reading(false, 20.0),
            reading(false, 80.0),
            reading(true, -8.0),
            reading(true, -8.0),
        ] {
            mgr.observe_window_placement(MAIN_WINDOW_LABEL, seen)
                .unwrap();
        }
        let attrs = mgr.window_attrs(MAIN_WINDOW_LABEL).unwrap();
        assert_eq!((attrs.bounds, attrs.maximized), (Some(bounds(80.0)), true));
    }

    #[test]
    fn a_window_whose_first_reading_is_the_maximized_rect_has_no_normal_rect() {
        let mut mgr = ViewManager::new();
        mgr.create_window("slot-popup-1").unwrap();
        for seen in [reading(false, -8.0), reading(true, -8.0)] {
            mgr.observe_window_placement("slot-popup-1", seen).unwrap();
        }
        let attrs = mgr.window_attrs("slot-popup-1").unwrap();
        assert_eq!((attrs.bounds, attrs.maximized), (None, true));
    }

    #[test]
    fn attributes_are_per_window_and_die_with_the_window() {
        let mut mgr = ViewManager::new();
        mgr.create_window("slot-popup-1").unwrap();
        mgr.set_window_theme("slot-popup-1", Some(UiTheme::EInk))
            .unwrap();
        assert_eq!(mgr.window_attrs(MAIN_WINDOW_LABEL).unwrap().theme, None);
        mgr.close_window("slot-popup-1").unwrap();
        mgr.create_window("slot-popup-1").unwrap();
        assert_eq!(
            mgr.window_attrs("slot-popup-1"),
            Ok(WindowAttrs::default()),
            "같은 label 의 새 창은 옛 창의 속성을 받지 않는다(ADR-0167)"
        );
    }

    // ── from_restored ───────────────────────────────────────────────────────

    fn view_named(name: &str) -> View {
        let layout = LayoutNode::new_empty_slot();
        View {
            id: Uuid::new_v4(),
            name: name.into(),
            focused_slot_id: Some(tree::first_slot_id(&layout)),
            layout,
        }
    }

    fn restored(tabs: Vec<View>, active: Option<ViewId>) -> RestoredWindow {
        RestoredWindow {
            attrs: WindowAttrs {
                theme: Some(UiTheme::Dark),
                bounds: Some(bounds(1.0)),
                maximized: true,
            },
            tabs,
            active,
        }
    }

    #[test]
    fn from_restored_rebuilds_ownership_and_keeps_order_attrs_and_ids() {
        let (a, b, p) = (view_named("a"), view_named("b"), view_named("p"));
        let popout_id = Uuid::new_v4().to_string();
        let mgr = ViewManager::from_restored(
            restored(vec![a.clone(), b.clone()], Some(b.id)),
            vec![(
                "slot-popup-1".into(),
                popout_id.clone(),
                restored(vec![p.clone()], Some(p.id)),
            )],
        )
        .unwrap();
        assert_invariants(&mgr);
        let main = window(&mgr, MAIN_WINDOW_LABEL);
        assert_eq!((main.tabs, main.active), (vec![a.id, b.id], b.id));
        assert_eq!(main.window_id, "main");
        assert_eq!(main.attrs, restored(vec![], None).attrs);
        assert_eq!((main.canvas, main.metrics), (None, None));
        let popout = window(&mgr, "slot-popup-1");
        assert_eq!((popout.tabs, popout.window_id), (vec![p.id], popout_id));
        assert_eq!(mgr.views[&a.id], a);
        assert_eq!((mgr.version, mgr.attrs_rev()), (0, 0));
    }

    #[test]
    fn from_restored_gives_an_empty_main_one_blank_tab_and_fixes_focus() {
        let mgr = ViewManager::from_restored(restored(vec![], None), vec![]).unwrap();
        assert_invariants(&mgr);
        let main = window(&mgr, MAIN_WINDOW_LABEL);
        assert_eq!(main.tabs.len(), 1);
        assert_eq!(mgr.views[&main.active].name, "View 1");
        assert_eq!(main.attrs, restored(vec![], None).attrs, "속성은 살린다");

        let mut lost = view_named("lost");
        lost.focused_slot_id = Some(Uuid::new_v4());
        let mut none = view_named("none");
        none.focused_slot_id = None;
        let mgr = ViewManager::from_restored(
            restored(vec![lost.clone(), none.clone()], Some(lost.id)),
            vec![],
        )
        .unwrap();
        for v in [&lost, &none] {
            assert_eq!(
                mgr.views[&v.id].focused_slot_id,
                Some(tree::first_slot_id(&v.layout))
            );
        }
    }

    #[test]
    fn from_restored_refuses_what_it_cannot_make_consistent() {
        let a = view_named("a");
        let p = view_named("p");
        let popout = |tabs: Vec<View>, active| {
            vec![(
                "slot-popup-1".to_string(),
                Uuid::new_v4().to_string(),
                restored(tabs, active),
            )]
        };
        let main = || restored(vec![a.clone()], Some(a.id));
        let cases = [
            ("탭 없는 팝아웃", main(), popout(vec![], None)),
            ("활성 없음", restored(vec![a.clone()], None), vec![]),
            ("탭 밖 활성", restored(vec![a.clone()], Some(p.id)), vec![]),
            (
                "팝아웃 탭 밖 활성",
                main(),
                popout(vec![p.clone()], Some(a.id)),
            ),
            ("겹친 View", main(), popout(vec![a.clone()], Some(a.id))),
            (
                "한 창 안 겹친 View",
                restored(vec![a.clone(), a.clone()], Some(a.id)),
                vec![],
            ),
            (
                "main label 의 팝아웃",
                main(),
                vec![(
                    MAIN_WINDOW_LABEL.to_string(),
                    Uuid::new_v4().to_string(),
                    restored(vec![p.clone()], Some(p.id)),
                )],
            ),
            (
                "겹친 팝아웃 label",
                main(),
                vec![
                    (
                        "x".to_string(),
                        "1".to_string(),
                        restored(vec![p.clone()], Some(p.id)),
                    ),
                    {
                        let q = view_named("q");
                        let q_id = q.id;
                        (
                            "x".to_string(),
                            "2".to_string(),
                            restored(vec![q], Some(q_id)),
                        )
                    },
                ],
            ),
        ];
        for (name, main, popouts) in cases {
            assert!(ViewManager::from_restored(main, popouts).is_err(), "{name}");
        }
    }

    // ── adopt_restored (TRD S21-storage §6-7 ③) ─────────────────────────────

    fn fingerprint(mgr: &ViewManager) -> (u64, u64, Vec<WindowLabel>, Vec<ViewId>) {
        let mut labels = mgr.list_windows();
        labels.sort();
        let mut views: Vec<ViewId> = mgr.views.keys().copied().collect();
        views.sort();
        (mgr.version, mgr.attrs_rev(), labels, views)
    }

    fn restored_with_popout(label: &str) -> (ViewManager, View, View, View) {
        let (a, b, p) = (view_named("a"), view_named("b"), view_named("p"));
        let mgr = ViewManager::from_restored(
            restored(vec![a.clone(), b.clone()], Some(b.id)),
            vec![(
                label.into(),
                "4b7f6a7e-0d5e-4f5a-9b0e-6a8f0c2d1e3f".into(),
                restored(vec![p.clone()], Some(p.id)),
            )],
        )
        .unwrap();
        (mgr, a, b, p)
    }

    #[test]
    fn adopt_restored_swaps_the_windows_bumps_both_counters_once_and_moves_the_side_table() {
        let mut mgr = ViewManager::new();
        let old_main = main_active(&mgr);
        let old_popout = mgr.create_window("slot-popup-1").unwrap();
        let old_slot = first_slot_of(&mgr, old_popout);
        mgr.set_unknown_content(old_popout, old_slot, raw("old"))
            .unwrap();
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 800, 600).unwrap();
        mgr.set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::Light))
            .unwrap();
        let (version, attrs_rev) = (mgr.version, mgr.attrs_rev());

        let (mut fresh, a, b, p) = restored_with_popout("slot-popup-2");
        let p_slot = first_slot_of(&fresh, p.id);
        fresh
            .set_unknown_content(p.id, p_slot, raw("future"))
            .unwrap();

        let removed = mgr.adopt_restored(fresh).unwrap();

        assert_eq!(removed, ["slot-popup-1"]);
        assert_invariants(&mgr);
        assert_eq!(
            (mgr.version, mgr.attrs_rev()),
            (version + 1, attrs_rev + 1),
            "지금 번호에서 하나씩 — 되감지 않는다"
        );
        let main = window(&mgr, MAIN_WINDOW_LABEL);
        assert_eq!((main.tabs, main.active), (vec![a.id, b.id], b.id));
        assert_eq!(main.attrs, restored(vec![], None).attrs);
        assert_eq!(main.window_id, "main");
        assert_eq!(
            main.canvas,
            Some(CanvasPx { w: 800, h: 600 }),
            "측정값은 남는다"
        );
        let popout = window(&mgr, "slot-popup-2");
        assert_eq!(
            (popout.tabs, popout.window_id.as_str()),
            (vec![p.id], "4b7f6a7e-0d5e-4f5a-9b0e-6a8f0c2d1e3f")
        );
        assert!(!mgr.views.contains_key(&old_main));
        assert!(!mgr.views.contains_key(&old_popout));
        assert_eq!(
            mgr.unknown_content(old_popout, old_slot),
            None,
            "옛 곁표는 거둔다"
        );
        assert_eq!(
            mgr.unknown_content(p.id, p_slot),
            Some(&raw("future")),
            "새 곁표는 View 와 함께 온다"
        );
        assert_eq!(mgr.snapshot(p.id).unwrap().foreign_slots, [p_slot]);
    }

    #[test]
    fn adopt_restored_refuses_a_collision_or_a_missing_main_and_changes_nothing() {
        let mut mgr = ViewManager::new();
        mgr.create_window("slot-popup-1").unwrap();
        let src = main_active(&mgr);
        let src_slot = first_slot_of(&mgr, src);
        let (tmp, _) = mgr
            .prepare_detached_view(src, src_slot, "옮기는 중".into())
            .unwrap();
        let before = fingerprint(&mgr);

        let (live_label, ..) = restored_with_popout("slot-popup-1");
        assert_eq!(
            mgr.adopt_restored(live_label).unwrap_err(),
            LayoutError::WindowNotFound("slot-popup-1".into()),
            "떠 있는 창의 label"
        );

        let (mut no_main, ..) = restored_with_popout("slot-popup-2");
        no_main.windows.remove(MAIN_WINDOW_LABEL);
        assert_eq!(
            mgr.adopt_restored(no_main).unwrap_err(),
            LayoutError::WindowNotFound(MAIN_WINDOW_LABEL.into())
        );

        let mut clash = view_named("clash");
        clash.id = tmp;
        let clash = ViewManager::from_restored(restored(vec![clash], Some(tmp)), vec![]).unwrap();
        assert_eq!(
            mgr.adopt_restored(clash).unwrap_err(),
            LayoutError::ViewNotFound(tmp),
            "남는 View(owner 없는 임시 View)와 겹친다"
        );

        assert_eq!(fingerprint(&mgr), before);
    }

    #[test]
    fn adopt_restored_leaves_an_ownerless_detached_view_to_its_move() {
        let mut mgr = ViewManager::new();
        let src = main_active(&mgr);
        let src_slot = first_slot_of(&mgr, src);
        let (tmp, _) = mgr
            .prepare_detached_view(src, src_slot, "옮기는 중".into())
            .unwrap();

        let (fresh, ..) = restored_with_popout("slot-popup-2");
        mgr.adopt_restored(fresh).unwrap();

        assert!(mgr.views.contains_key(&tmp));
        mgr.attach_view_as_new_window("slot-popup-3", tmp).unwrap();
        assert_invariants(&mgr);
    }

    // ── 측정 보고 · slot_px (ADR-0227) ───────────────────────────────────────

    fn metrics(t: f64, r: f64, b: f64, l: f64, min_pane_px: u32) -> UiMetrics {
        UiMetrics {
            frame_insets: geometry::Insets { t, r, b, l },
            min_pane_px,
        }
    }

    fn one_px_border() -> UiMetrics {
        metrics(1.0, 1.0, 1.0, 1.0, 30)
    }

    fn window(mgr: &ViewManager, label: &str) -> WindowTabs {
        mgr.windows.get(label).expect("창 존재").clone()
    }

    #[test]
    fn canvas_is_stored_per_window() {
        let mut mgr = ViewManager::new();
        mgr.create_window("popup-1").unwrap();
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1200, 800).unwrap();
        mgr.set_window_canvas("popup-1", 640, 480).unwrap();

        assert_eq!(
            window(&mgr, MAIN_WINDOW_LABEL).canvas,
            Some(CanvasPx { w: 1200, h: 800 })
        );
        assert_eq!(
            window(&mgr, "popup-1").canvas,
            Some(CanvasPx { w: 640, h: 480 })
        );
    }

    #[test]
    fn hidden_tab_uses_the_window_canvas_for_slot_px() {
        let mut mgr = ViewManager::new();
        let hidden = main_active(&mgr);
        let shown = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        assert_ne!(main_active(&mgr), hidden, "전제: 첫 탭이 숨었다");
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1200, 800).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, one_px_border())
            .unwrap();

        let hidden_px = mgr
            .slot_px(hidden, first_slot_of(&mgr, hidden))
            .unwrap()
            .expect("숨은 탭도 창 캔버스로 계산");
        let shown_px = mgr
            .slot_px(shown, first_slot_of(&mgr, shown))
            .unwrap()
            .expect("활성 탭");
        let full = PxRect {
            x0: 0,
            y0: 0,
            x1: 1200,
            y1: 800,
        };
        assert_eq!(hidden_px.frame, full);
        assert_eq!(shown_px.frame, full);
    }

    #[test]
    fn a_new_window_starts_without_canvas_or_metrics() {
        let mut mgr = ViewManager::new();
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1200, 800).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, one_px_border())
            .unwrap();

        let fresh = mgr.create_window("popup-1").unwrap();
        let src = main_active(&mgr);
        let (detached, _) = mgr
            .prepare_detached_view(src, first_slot_of(&mgr, src), "Tab".into())
            .unwrap();
        mgr.attach_view_as_new_window("popup-2", detached).unwrap();

        for (label, view) in [("popup-1", fresh), ("popup-2", detached)] {
            let wt = window(&mgr, label);
            assert_eq!(
                wt.canvas, None,
                "{label}: 캔버스는 그 웹뷰 보고 전까지 없다"
            );
            assert_eq!(wt.metrics, None, "{label}: 지표도 없다");
            assert_eq!(
                mgr.slot_px(view, first_slot_of(&mgr, view)),
                Ok(None),
                "{label}: 메인 값을 빌려 쓰지 않는다"
            );
        }
    }

    #[test]
    fn canvas_and_metrics_vanish_with_the_window() {
        let mut mgr = ViewManager::new();
        mgr.create_window("popup-1").unwrap();
        let tab = mgr.create_window("popup-2").unwrap();
        for label in ["popup-1", "popup-2"] {
            mgr.set_window_canvas(label, 640, 480).unwrap();
            mgr.set_ui_metrics(label, one_px_border()).unwrap();
        }

        mgr.close_window("popup-1").unwrap();
        assert_eq!(
            mgr.close_tab("popup-2", tab).unwrap(),
            CloseTabOutcome::WindowClosed
        );

        for label in ["popup-1", "popup-2"] {
            assert!(!mgr.windows.contains_key(label));
            assert_eq!(
                mgr.set_window_canvas(label, 640, 480),
                Err(LayoutError::WindowNotFound(label.to_string()))
            );
        }
    }

    #[test]
    fn reports_do_not_bump_version() {
        let mut mgr = ViewManager::new();
        let before = mgr.version;
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1200, 800).unwrap();
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 0, 800).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, one_px_border())
            .unwrap();
        let _ = mgr.set_ui_metrics(MAIN_WINDOW_LABEL, metrics(-1.0, 1.0, 1.0, 1.0, 30));
        let _ = mgr.set_window_canvas("no-such", 1, 1);
        assert_eq!(mgr.version, before);
    }

    #[test]
    fn a_zero_size_canvas_report_is_ignored_and_keeps_the_previous_value() {
        let mut mgr = ViewManager::new();
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1200, 800).unwrap();
        for (w, h) in [(0, 800), (1200, 0), (0, 0)] {
            assert_eq!(mgr.set_window_canvas(MAIN_WINDOW_LABEL, w, h), Ok(()));
            assert_eq!(
                window(&mgr, MAIN_WINDOW_LABEL).canvas,
                Some(CanvasPx { w: 1200, h: 800 }),
                "{w}x{h} 보고가 직전 값을 덮었다"
            );
        }
    }

    #[test]
    fn a_zero_size_first_report_leaves_the_canvas_unknown() {
        let mut mgr = ViewManager::new();
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 0, 0).unwrap();
        assert_eq!(window(&mgr, MAIN_WINDOW_LABEL).canvas, None);
    }

    #[test]
    fn out_of_range_metrics_are_rejected_and_keep_the_previous_value() {
        let good = one_px_border();
        let bad = [
            ("t < 0", metrics(-0.5, 1.0, 1.0, 1.0, 30)),
            ("r > 64", metrics(1.0, 64.5, 1.0, 1.0, 30)),
            ("b NaN", metrics(1.0, 1.0, f64::NAN, 1.0, 30)),
            ("l +inf", metrics(1.0, 1.0, 1.0, f64::INFINITY, 30)),
            ("t -inf", metrics(f64::NEG_INFINITY, 1.0, 1.0, 1.0, 30)),
            ("min_pane_px 0", metrics(1.0, 1.0, 1.0, 1.0, 0)),
            ("min_pane_px 1001", metrics(1.0, 1.0, 1.0, 1.0, 1001)),
        ];
        for (why, m) in bad {
            let mut mgr = ViewManager::new();
            mgr.set_ui_metrics(MAIN_WINDOW_LABEL, good).unwrap();
            assert!(
                matches!(
                    mgr.set_ui_metrics(MAIN_WINDOW_LABEL, m),
                    Err(LayoutError::InvalidMetrics(_))
                ),
                "{why}: 거절돼야 한다"
            );
            assert_eq!(
                window(&mgr, MAIN_WINDOW_LABEL).metrics,
                Some(good),
                "{why}: 직전 값을 유지해야 한다"
            );
        }
    }

    #[test]
    fn exact_metric_bounds_are_accepted() {
        let mut mgr = ViewManager::new();
        for m in [
            metrics(0.0, 0.0, 0.0, 0.0, 1),
            metrics(64.0, 64.0, 64.0, 64.0, 1000),
            metrics(0.8, 1.25, 0.0, 64.0, 30),
        ] {
            assert_eq!(mgr.set_ui_metrics(MAIN_WINDOW_LABEL, m), Ok(()));
            assert_eq!(window(&mgr, MAIN_WINDOW_LABEL).metrics, Some(m));
        }
    }

    #[test]
    fn reports_to_an_unknown_window_are_window_not_found() {
        let mut mgr = ViewManager::new();
        assert_eq!(
            mgr.set_window_canvas("no-such", 100, 100),
            Err(LayoutError::WindowNotFound("no-such".into()))
        );
        assert_eq!(
            mgr.set_ui_metrics("no-such", one_px_border()),
            Err(LayoutError::WindowNotFound("no-such".into()))
        );
        // agent-tree 창은 탭 모델 밖이라 보고할 자리가 없다.
        assert_eq!(
            mgr.set_window_canvas("agent-tree", 100, 100),
            Err(LayoutError::WindowNotFound("agent-tree".into()))
        );
    }

    #[test]
    fn slot_px_rounds_frame_edges_from_the_canvas_and_subtracts_insets() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let left = first_slot_of(&mgr, v);
        let right = mgr.split_slot(v, left, SplitDir::LeftRight).unwrap();
        // 폭 1001 × 비율 0.5 = 500.5 → 동률은 0 에서 먼 쪽이라 501.
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1001, 600).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, metrics(1.0, 2.0, 3.0, 4.0, 30))
            .unwrap();

        let l = mgr.slot_px(v, left).unwrap().unwrap();
        let r = mgr.slot_px(v, right).unwrap().unwrap();
        assert_eq!(
            l.frame,
            PxRect {
                x0: 0,
                y0: 0,
                x1: 501,
                y1: 600
            }
        );
        assert_eq!(
            r.frame,
            PxRect {
                x0: 501,
                y0: 0,
                x1: 1001,
                y1: 600
            }
        );
        assert_eq!(
            l.content,
            RectF64 {
                x0: 4.0,
                y0: 1.0,
                x1: 499.0,
                y1: 597.0
            }
        );
        assert_eq!(
            r.content,
            RectF64 {
                x0: 505.0,
                y0: 1.0,
                x1: 999.0,
                y1: 597.0
            }
        );
    }

    #[test]
    fn slot_px_content_never_goes_below_zero() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let s = first_slot_of(&mgr, v);
        // 틀(3×2)이 여백 합(좌우 4 · 위아래 4)보다 좁다.
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 3, 2).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, metrics(2.0, 2.0, 2.0, 2.0, 30))
            .unwrap();

        let px = mgr.slot_px(v, s).unwrap().unwrap();
        assert!(px.content.x1 - px.content.x0 >= 0.0, "{:?}", px.content);
        assert!(px.content.y1 - px.content.y0 >= 0.0, "{:?}", px.content);
        assert_eq!(px.content.x1, px.content.x0);
        assert_eq!(px.content.y1, px.content.y0);
    }

    #[test]
    fn slot_px_is_none_until_both_canvas_and_metrics_are_known() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let s = first_slot_of(&mgr, v);
        assert_eq!(mgr.slot_px(v, s), Ok(None), "둘 다 없음");

        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1200, 800).unwrap();
        assert_eq!(mgr.slot_px(v, s), Ok(None), "지표 없음");
        assert_eq!(mgr.px_context(v), Ok(None));

        let mut only_metrics = ViewManager::new();
        let v2 = main_active(&only_metrics);
        let s2 = first_slot_of(&only_metrics, v2);
        only_metrics
            .set_ui_metrics(MAIN_WINDOW_LABEL, one_px_border())
            .unwrap();
        assert_eq!(only_metrics.slot_px(v2, s2), Ok(None), "캔버스 없음");

        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, one_px_border())
            .unwrap();
        assert!(mgr.slot_px(v, s).unwrap().is_some(), "둘 다 있음");
        assert_eq!(
            mgr.px_context(v),
            Ok(Some((CanvasPx { w: 1200, h: 800 }, one_px_border())))
        );
    }

    #[test]
    fn slot_px_unknown_view_or_slot_is_err_even_without_canvas() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let other = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let bogus = Uuid::new_v4();
        assert_eq!(
            mgr.slot_px(bogus, first_slot_of(&mgr, v)),
            Err(LayoutError::ViewNotFound(bogus))
        );
        assert_eq!(mgr.slot_px(v, bogus), Err(LayoutError::SlotNotFound(bogus)));
        // 다른 탭의 칸은 이 view 의 칸이 아니다.
        let foreign = first_slot_of(&mgr, other);
        assert_eq!(
            mgr.slot_px(v, foreign),
            Err(LayoutError::SlotNotFound(foreign))
        );
        assert_eq!(mgr.px_context(bogus), Err(LayoutError::ViewNotFound(bogus)));
    }

    #[test]
    fn px_context_of_an_ownerless_detached_view_is_none() {
        let mut mgr = ViewManager::new();
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1200, 800).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, one_px_border())
            .unwrap();
        let src = main_active(&mgr);
        let (tmp, _) = mgr
            .prepare_detached_view(src, first_slot_of(&mgr, src), "Tab".into())
            .unwrap();
        assert_eq!(mgr.px_context(tmp), Ok(None));
    }

    // ── 비율 쓰기 · 분할 표현 가능성 (ADR-0227) ──────────────────────────────

    // 전위 순 — 루트가 맨 앞, 같은 쪽으로 판 사슬이면 가장 깊은 것이 맨 뒤다.
    fn split_ids(mgr: &ViewManager, view: ViewId) -> Vec<Uuid> {
        tree::list_splits(&mgr.views[&view].layout)
            .into_iter()
            .map(|s| s.id)
            .collect()
    }

    fn ratio_in(mgr: &ViewManager, view: ViewId, split: Uuid) -> f64 {
        tree::split_ratio(&mgr.views[&view].layout, split).expect("분할 있어야")
    }

    fn slot_rect(mgr: &ViewManager, view: ViewId, slot: Uuid) -> SlotRect {
        *geometry::compute(&mgr.views[&view].layout)
            .slots
            .iter()
            .find(|r| r.slot_id == slot)
            .expect("칸 있어야")
    }

    fn assert_every_leaf_has_area(mgr: &ViewManager, view: ViewId) {
        for r in geometry::compute(&mgr.views[&view].layout).slots {
            assert!(has_area(&r), "면적 0 칸: {r:?}");
        }
    }

    // main 활성 탭의 첫 칸을 `dir` 로 한 번 나눈 세계 — 반환 = (관리자, 탭, 그 분할).
    fn one_split(dir: SplitDir) -> (ViewManager, ViewId, Uuid) {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let s = first_slot_of(&mgr, v);
        mgr.split_slot(v, s, dir).unwrap();
        let split = split_ids(&mgr, v)[0];
        (mgr, v, split)
    }

    fn applied(ratio: f64) -> SplitRatioResult {
        SplitRatioResult {
            ratio,
            outcome: SplitRatioOutcome::Applied,
        }
    }

    // 값을 안 바꾼 결말 — 트리·version 이 그대로인지도 함께 본다.
    fn assert_untouched(
        mgr: &mut ViewManager,
        view: ViewId,
        split: Uuid,
        ratio: f64,
        expect: SplitRatioResult,
    ) {
        let before = mgr.views[&view].clone();
        let ver = mgr.version;
        assert_eq!(mgr.set_split_ratio(view, split, ratio), Ok(expect));
        assert_eq!(mgr.views[&view], before, "무변경이어야 한다");
        assert_eq!(mgr.version, ver, "version 을 안 올린다");
    }

    #[test]
    fn set_split_ratio_writes_the_split_with_that_id_and_bumps_version() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let x = first_slot_of(&mgr, v);
        let y = mgr.split_slot(v, x, SplitDir::LeftRight).unwrap();
        mgr.split_slot(v, y, SplitDir::TopBottom).unwrap();
        let ids = split_ids(&mgr, v);
        assert_eq!(ids.len(), 2, "분할 둘");
        let (outer, inner) = (ids[0], ids[1]);
        let ver = mgr.version;

        assert_eq!(mgr.set_split_ratio(v, inner, 0.7), Ok(applied(0.7)));
        assert_eq!(ratio_in(&mgr, v, inner), 0.7);
        assert_eq!(
            ratio_in(&mgr, v, outer),
            tree::SPLIT_RATIO,
            "다른 분할은 그대로"
        );
        assert_eq!(mgr.version, ver + 1);
        assert_eq!(mgr.snapshot(v).unwrap().version, ver + 1);
    }

    #[test]
    fn set_split_ratio_of_a_missing_split_or_view_changes_nothing() {
        let (mut mgr, v, _split) = one_split(SplitDir::LeftRight);
        let other = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let other_slot = first_slot_of(&mgr, other);
        mgr.split_slot(other, other_slot, SplitDir::TopBottom)
            .unwrap();
        let foreign = split_ids(&mgr, other)[0];
        let bogus = Uuid::new_v4();
        let slot = first_slot_of(&mgr, v);
        let before = mgr.views[&v].clone();
        let ver = mgr.version;

        for missing in [bogus, slot, foreign] {
            assert_eq!(
                mgr.set_split_ratio(v, missing, 0.3),
                Err(LayoutError::SplitNotFound(missing)),
                "칸 id·다른 탭의 분할도 이 탭의 분할이 아니다"
            );
        }
        assert_eq!(
            mgr.set_split_ratio(bogus, foreign, 0.3),
            Err(LayoutError::ViewNotFound(bogus))
        );
        assert_eq!(mgr.views[&v], before);
        assert_eq!(ratio_in(&mgr, other, foreign), tree::SPLIT_RATIO);
        assert_eq!(mgr.version, ver);
    }

    #[test]
    fn set_split_ratio_refuses_non_finite_values_at_the_manager() {
        let (mut mgr, v, split) = one_split(SplitDir::LeftRight);
        let before = mgr.views[&v].clone();
        let ver = mgr.version;
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(
                matches!(
                    mgr.set_split_ratio(v, split, bad),
                    Err(LayoutError::InvalidRatio(_))
                ),
                "{bad}: 거절돼야 한다"
            );
            // 없는 view 보다 먼저 걸린다 — 판정 순서가 계약이다.
            assert!(matches!(
                mgr.set_split_ratio(Uuid::new_v4(), split, bad),
                Err(LayoutError::InvalidRatio(_))
            ));
        }
        assert_eq!(mgr.views[&v], before);
        assert_eq!(mgr.version, ver);
    }

    #[test]
    fn set_split_ratio_clamps_to_the_ratio_bounds() {
        let (mut mgr, v, split) = one_split(SplitDir::LeftRight);
        assert_eq!(
            mgr.set_split_ratio(v, split, 0.0),
            Ok(applied(tree::RATIO_MIN))
        );
        assert_eq!(ratio_in(&mgr, v, split), tree::RATIO_MIN);
        assert_eq!(
            mgr.set_split_ratio(v, split, 1.0),
            Ok(applied(tree::RATIO_MAX))
        );
        assert_eq!(
            mgr.set_split_ratio(v, split, -5.0),
            Ok(applied(tree::RATIO_MIN))
        );
        // 잘린 값이 지금 값과 같으면 무변경이다.
        assert_untouched(
            &mut mgr,
            v,
            split,
            0.005,
            SplitRatioResult {
                ratio: tree::RATIO_MIN,
                outcome: SplitRatioOutcome::Unchanged,
            },
        );
    }

    #[test]
    fn ratio_0_3_round_trips_exactly() {
        let (mut mgr, v, split) = one_split(SplitDir::TopBottom);
        let got = mgr.set_split_ratio(v, split, 0.3).unwrap();
        assert_eq!(got.ratio.to_bits(), 0.3f64.to_bits());
        assert_eq!(ratio_in(&mgr, v, split).to_bits(), 0.3f64.to_bits());
        let wire = serde_json::to_value(mgr.snapshot(v).unwrap().layout).unwrap();
        assert_eq!(wire["ratio"], serde_json::json!(0.3), "{wire}");
        assert_eq!(serde_json::to_string(&got.ratio).unwrap(), "0.3");
    }

    #[test]
    fn the_same_value_is_unchanged_without_a_version_bump() {
        let (mut mgr, v, split) = one_split(SplitDir::LeftRight);
        assert_untouched(
            &mut mgr,
            v,
            split,
            tree::SPLIT_RATIO,
            SplitRatioResult {
                ratio: tree::SPLIT_RATIO,
                outcome: SplitRatioOutcome::Unchanged,
            },
        );
        mgr.set_split_ratio(v, split, 0.3).unwrap();
        assert_untouched(
            &mut mgr,
            v,
            split,
            0.3,
            SplitRatioResult {
                ratio: 0.3,
                outcome: SplitRatioOutcome::Unchanged,
            },
        );
    }

    #[test]
    fn px_minimum_keeps_both_sides_at_least_min_pane_px_on_the_split_axis() {
        // 가로 분할 둘: 바깥(상자 폭 1000px) · 오른쪽 칸 안쪽(상자 폭 500px). m = 100.
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let x = first_slot_of(&mgr, v);
        let y = mgr.split_slot(v, x, SplitDir::LeftRight).unwrap();
        mgr.split_slot(v, y, SplitDir::LeftRight).unwrap();
        let ids = split_ids(&mgr, v);
        assert_eq!(ids.len(), 2, "분할 둘");
        let (outer, inner) = (ids[0], ids[1]);
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 1000, 400).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, metrics(1.0, 1.0, 1.0, 1.0, 100))
            .unwrap();

        // 안쪽: L = 1000 × 0.5 = 500 → 허용 [0.2, 0.8]. 캔버스 전체 폭으로 재면 0.1 이 통과해 버린다.
        assert_eq!(mgr.set_split_ratio(v, inner, 0.1), Ok(applied(0.2)));
        assert_eq!(ratio_in(&mgr, v, inner), 0.2);
        assert_eq!(mgr.set_split_ratio(v, inner, 0.95), Ok(applied(0.8)));
        assert_eq!(
            mgr.set_split_ratio(v, inner, 0.4),
            Ok(applied(0.4)),
            "범위 안은 그대로"
        );
        // 바깥: L = 1000 → m/L = 0.1 이 비율 한계보다 커서 px 가 이긴다.
        assert_eq!(mgr.set_split_ratio(v, outer, 0.05), Ok(applied(0.1)));
        assert_eq!(mgr.set_split_ratio(v, outer, 0.99), Ok(applied(0.9)));

        // 위아래 분할은 캔버스 높이로 잰다: L = 400 → 허용 [0.25, 0.75].
        let (mut tb, tv, split) = one_split(SplitDir::TopBottom);
        tb.set_window_canvas(MAIN_WINDOW_LABEL, 1000, 400).unwrap();
        tb.set_ui_metrics(MAIN_WINDOW_LABEL, metrics(1.0, 1.0, 1.0, 1.0, 100))
            .unwrap();
        assert_eq!(tb.set_split_ratio(tv, split, 0.1), Ok(applied(0.25)));
        assert_eq!(tb.set_split_ratio(tv, split, 0.9), Ok(applied(0.75)));
    }

    // ADR-0260
    #[test]
    fn at_the_screen_minimum_either_pane_of_a_top_bottom_split_stops_at_30px() {
        // 화면 최소 칸 = 30px(프론트 `MIN_PANE_PX`). 위 칸의 벽은 하한, 아래 칸의 벽은 상한이다.
        let (mut mgr, v, split) = one_split(SplitDir::TopBottom);
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 800, 1000).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, metrics(1.0, 1.0, 1.0, 1.0, 30))
            .unwrap();
        let row = mgr
            .list_splits(v)
            .unwrap()
            .into_iter()
            .next()
            .expect("분할 하나");
        let (top, bottom) = (row.a_slots[0], row.b_slots[0]);
        let height = |mgr: &ViewManager, slot| {
            let f = mgr.slot_px(v, slot).unwrap().unwrap().frame;
            f.y1 - f.y0
        };

        // L = 1000 → 허용 [0.03, 0.97].
        assert_eq!(mgr.set_split_ratio(v, split, 0.01), Ok(applied(0.03)));
        assert_eq!(height(&mgr, top), 30);
        assert_untouched(
            &mut mgr,
            v,
            split,
            0.0,
            SplitRatioResult {
                ratio: 0.03,
                outcome: SplitRatioOutcome::Unchanged,
            },
        );
        assert_eq!(mgr.set_split_ratio(v, split, 0.99), Ok(applied(0.97)));
        assert_eq!(height(&mgr, bottom), 30);

        // L 이 3000px 를 넘으면 30/L 이 비율 한계보다 작아 비율 한계가 이긴다(L = 5000 → 0.006).
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 800, 5000).unwrap();
        assert_eq!(
            mgr.set_split_ratio(v, split, 0.0),
            Ok(applied(tree::RATIO_MIN))
        );
        assert_eq!(
            mgr.set_split_ratio(v, split, 1.0),
            Ok(applied(tree::RATIO_MAX))
        );
    }

    #[test]
    fn an_empty_px_range_is_too_small_and_keeps_the_current_ratio() {
        let (mut mgr, v, split) = one_split(SplitDir::LeftRight);
        mgr.set_split_ratio(v, split, 0.3).unwrap();
        // L = 300 < 2m = 400.
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 300, 300).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, metrics(1.0, 1.0, 1.0, 1.0, 200))
            .unwrap();
        let too_small = SplitRatioResult {
            ratio: 0.3,
            outcome: SplitRatioOutcome::TooSmall,
        };
        for asked in [0.6, 0.3, 0.1] {
            assert_untouched(&mut mgr, v, split, asked, too_small);
        }
    }

    #[test]
    fn without_canvas_or_metrics_only_the_ratio_bounds_apply() {
        // 둘 중 하나만 있어도 px 최소는 안 건다 — 둘 다 있으면 [0.2, 0.8] 로 잘릴 세계다.
        let (mut only_canvas, v1, s1) = one_split(SplitDir::LeftRight);
        only_canvas
            .set_window_canvas(MAIN_WINDOW_LABEL, 500, 500)
            .unwrap();
        assert_eq!(only_canvas.set_split_ratio(v1, s1, 0.1), Ok(applied(0.1)));
        assert_eq!(
            only_canvas.set_split_ratio(v1, s1, 0.0),
            Ok(applied(tree::RATIO_MIN))
        );

        let (mut only_metrics, v2, s2) = one_split(SplitDir::LeftRight);
        only_metrics
            .set_ui_metrics(MAIN_WINDOW_LABEL, metrics(1.0, 1.0, 1.0, 1.0, 100))
            .unwrap();
        assert_eq!(only_metrics.set_split_ratio(v2, s2, 0.1), Ok(applied(0.1)));
        assert_eq!(
            only_metrics.set_split_ratio(v2, s2, 0.001),
            Ok(applied(tree::RATIO_MIN))
        );
        assert_eq!(
            only_metrics.set_split_ratio(v2, s2, 0.999),
            Ok(applied(tree::RATIO_MAX))
        );
    }

    #[test]
    fn a_split_that_makes_panes_smaller_than_min_pane_px_is_allowed() {
        // R11: 캔버스·지표를 알아도 px 로 분할을 거절하지 않는다.
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let x = first_slot_of(&mgr, v);
        mgr.set_window_canvas(MAIN_WINDOW_LABEL, 40, 40).unwrap();
        mgr.set_ui_metrics(MAIN_WINDOW_LABEL, metrics(0.0, 0.0, 0.0, 0.0, 30))
            .unwrap();
        let before = mgr.views[&v].layout.clone();

        let y = mgr
            .split_slot(v, x, SplitDir::LeftRight)
            .expect("30px 미만 칸을 만드는 분할도 성공");
        assert_ne!(mgr.views[&v].layout, before, "트리가 바뀐다");
        for slot in [x, y] {
            let f = mgr.slot_px(v, slot).unwrap().unwrap().frame;
            assert!(f.x1 - f.x0 < 30, "{slot}: {f:?}");
        }
        mgr.split_slot(v, y, SplitDir::TopBottom)
            .expect("더 작게도 나뉜다");
    }

    #[test]
    fn repeated_same_side_splits_stop_with_split_too_deep_before_a_zero_width_slot() {
        for dir in [SplitDir::LeftRight, SplitDir::TopBottom] {
            let mut mgr = ViewManager::new();
            let v = main_active(&mgr);
            // 새 칸(b = 오른쪽/아래)을 계속 나눈다 — 경계가 1.0 쪽으로 몰린다.
            let mut target = first_slot_of(&mgr, v);
            let mut splits = 0;
            let err = loop {
                let before = mgr.views[&v].clone();
                let ver = mgr.version;
                match mgr.split_slot(v, target, dir) {
                    Ok(new) => {
                        target = new;
                        splits += 1;
                        assert!(splits < 200, "{dir:?}: 가드가 안 선다");
                    }
                    Err(e) => {
                        assert_eq!(mgr.views[&v], before, "{dir:?}: 거절은 트리 불변");
                        assert_eq!(mgr.version, ver);
                        break e;
                    }
                }
            };
            assert_eq!(err, LayoutError::SplitTooDeep);
            // f64 한계에서 멈췄다(일찍 거절한 게 아니다) — 반분 사슬은 약 54 단에서 닿는다.
            assert!(splits > 40, "{dir:?}: {splits} 단에서 멈춤");
            // 바로 그 직전이다: 칸은 아직 폭이 있지만 다음 경계가 칸 끝과 같아진다.
            let r = slot_rect(&mgr, v, target);
            let (lo, hi) = match dir {
                SplitDir::LeftRight => (r.x0, r.x1),
                SplitDir::TopBottom => (r.y0, r.y1),
            };
            assert!(lo < hi, "{dir:?}: {r:?}");
            let at = geometry::boundary(lo, hi, tree::SPLIT_RATIO);
            assert!(at == lo || at == hi, "{dir:?}: at={at} lo={lo} hi={hi}");
            assert_every_leaf_has_area(&mgr, v);
            mgr.snapshot(v).expect("이 깊이의 스냅샷도 패닉 없이 선다");
        }
    }

    // 사슬의 모든 분할을 0.9 로 둔다 — 나누는 새 칸(b)은 10% 몫이라 경계가 훨씬 빨리 1.0 에 몰린다.
    fn ninety_chain(mgr: &mut ViewManager, v: ViewId) -> (usize, LayoutError) {
        let mut target = first_slot_of(mgr, v);
        let mut levels = 0;
        loop {
            let before = mgr.views[&v].clone();
            match mgr.split_slot(v, target, SplitDir::LeftRight) {
                Ok(new) => {
                    let deepest = *split_ids(mgr, v).last().unwrap();
                    assert_eq!(
                        mgr.set_split_ratio(v, deepest, 0.9),
                        Ok(applied(0.9)),
                        "{levels} 단"
                    );
                    target = new;
                    levels += 1;
                    assert!(levels < 200, "가드가 안 선다");
                }
                Err(e) => {
                    assert_eq!(mgr.views[&v], before, "거절은 트리 불변");
                    return (levels, e);
                }
            }
        }
    }

    #[test]
    fn a_ninety_percent_chain_also_stops_with_split_too_deep() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let (levels, err) = ninety_chain(&mut mgr, v);
        assert_eq!(err, LayoutError::SplitTooDeep);
        // 10% 씩 줄면 약 17 단에서 닿는다 — 반분 사슬보다 훨씬 얕다.
        assert!((10..30).contains(&levels), "{levels} 단");
        assert_every_leaf_has_area(&mgr, v);
    }

    #[test]
    fn an_ancestor_ratio_that_would_collapse_a_deep_leaf_is_too_small() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let mut target = first_slot_of(&mgr, v);
        while let Ok(new) = mgr.split_slot(v, target, SplitDir::LeftRight) {
            target = new;
        }
        let root = split_ids(&mgr, v)[0];
        // 루트를 0.9 로 두면 사슬 전체가 폭 0.1 에서 시작해 맨 끝 칸들이 폭 0 이 된다. 한계 끝은 더 좁다.
        for asked in [0.9, tree::RATIO_MAX] {
            assert_untouched(
                &mut mgr,
                v,
                root,
                asked,
                SplitRatioResult {
                    ratio: tree::SPLIT_RATIO,
                    outcome: SplitRatioOutcome::TooSmall,
                },
            );
        }
        assert_every_leaf_has_area(&mgr, v);
        // 대조: 사슬이 더 넓어지는 쪽은 아무 칸도 안 무너뜨려 적용된다 — 거름이 일괄 거절이 아니다.
        assert_eq!(mgr.set_split_ratio(v, root, 0.1), Ok(applied(0.1)));
        assert_eq!(
            mgr.set_split_ratio(v, root, tree::RATIO_MIN),
            Ok(applied(tree::RATIO_MIN))
        );
        assert_every_leaf_has_area(&mgr, v);
    }

    // 쓰기 경로로는 못 만드는 트리: 좌우 반분 사슬(늘 b 쪽이 다음 단) — 54 단째부터 칸 폭이 0 이다.
    fn planted_half_chain(depth: usize) -> LayoutNode {
        let mut node = LayoutNode::new_empty_slot();
        for _ in 0..depth {
            node = LayoutNode::Split {
                id: Uuid::new_v4(),
                dir: SplitDir::LeftRight,
                ratio: 0.5,
                a: Box::new(LayoutNode::new_empty_slot()),
                b: Box::new(node),
            };
        }
        node
    }

    #[test]
    fn a_planted_zero_width_slot_is_too_deep_to_split_on_the_other_axis_too() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        mgr.views.get_mut(&v).unwrap().layout = planted_half_chain(60);
        let zero = geometry::compute(&mgr.views[&v].layout)
            .slots
            .into_iter()
            .find(|r| !has_area(r))
            .expect("전제: 면적 0 칸이 심겼다");
        assert!(
            zero.x0 == zero.x1 && zero.y0 < zero.y1,
            "전제: 폭만 0 이다: {zero:?}"
        );
        // 위아래로 나누면 새 경계는 높이 안에 엄격히 들지만 두 새 칸이 다 폭 0 이다.
        let before = mgr.views[&v].clone();
        let ver = mgr.version;
        assert_eq!(
            mgr.split_slot(v, zero.slot_id, SplitDir::TopBottom),
            Err(LayoutError::SplitTooDeep)
        );
        assert_eq!(mgr.views[&v], before, "거절은 트리 불변");
        assert_eq!(mgr.version, ver);
    }

    #[test]
    fn a_leaf_that_already_had_zero_area_does_not_lock_the_view() {
        // 위 = 가로 분할 Q{x, y} · 아래 = 심은 반분 사슬 60 단.
        let q = Uuid::new_v4();
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        mgr.views.get_mut(&v).unwrap().layout = LayoutNode::Split {
            id: Uuid::new_v4(),
            dir: SplitDir::TopBottom,
            ratio: 0.5,
            a: Box::new(LayoutNode::Split {
                id: q,
                dir: SplitDir::LeftRight,
                ratio: 0.5,
                a: Box::new(LayoutNode::new_empty_slot()),
                b: Box::new(LayoutNode::new_empty_slot()),
            }),
            b: Box::new(planted_half_chain(60)),
        };
        let zero = geometry::compute(&mgr.views[&v].layout)
            .slots
            .iter()
            .filter(|r| !has_area(r))
            .count();
        assert!(zero > 0, "전제: 면적 0 칸이 심겼다");

        assert_eq!(mgr.set_split_ratio(v, q, 0.3), Ok(applied(0.3)));
        assert_eq!(ratio_in(&mgr, v, q), 0.3);
    }

    #[test]
    fn list_splits_of_a_missing_view_is_view_not_found() {
        let (mgr, v, split) = one_split(SplitDir::LeftRight);
        let rows = mgr.list_splits(v).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, split);
        let bogus = Uuid::new_v4();
        assert_eq!(
            mgr.list_splits(bogus),
            Err(LayoutError::ViewNotFound(bogus))
        );
    }
}
