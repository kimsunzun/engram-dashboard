//! 대상 풀기(TRD S28 §5-2) — LLM 명령이 id · 에이전트 · 공간 낱말로 고른 대상을 지금 모델의 (창, 탭, 칸)으로 푼다.
//!
//! ★적용 서비스가 바꾸는 그 락 안에서 부른다★ — 풀고 나서 락을 놓았다가 다시 잡아 적용하면, 그 사이 다른 명령 ·
//! 마우스가 그 에이전트를 옮겼을 때 엉뚱한 탭 · 칸을 바꾼다. 그래서 모델(`ViewManager`)을 읽기만 하고 락을 모른다.
//!
//! 에이전트 **이름** → id 는 여기 몫이 아니다 — 데몬 명부의 해석기 하나가 푼다(ADR-0132 · 규칙 §1-2). 여기 오는
//! 에이전트는 이미 id 다. 셸의 이름 장부(`agent_names`)는 후보 · 배치 줄을 보여 주는 데만 쓴다.
//!
//! 순서(후보 · 줄)는 `window.list` 와 같은 창 번호 순 → 탭 줄 순 → 트리 전위 순이다.
// ADR-0307

use uuid::Uuid;

use super::bands::{self, SizeCss};
use super::dock::{DockAnchor, DockSide};
use super::geometry::{self, PxRect};
use super::manager::{CanvasPx, ViewId, ViewManager, WindowLabel, WindowTabs, MAIN_WINDOW_LABEL};
use super::spatial::{resolve_spatial, SpatialToken};
use super::tree;
use super::types::{SlotContent, View};

/// 후보 · 이름 목록의 상한(규칙 §1-3 — 다음 호출 한 번에 고칠 만큼만).
pub const MAX_CANDIDATES: usize = 5;

/// 붙는 몫을 셀 크기를 모를 때(웹뷰가 아직 캔버스를 안 알림)의 몫(TRD S28 §4-2 · §11-2).
pub const UNMEASURED_SHARE: f64 = 0.5;

/// 명령이 고른 대상. 에이전트 갈래의 id 는 데몬이 푼 것이고, 칸의 에이전트 id 와 UUID 값으로 견준다(철자 무관).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// id 로 준 탭.
    Tab(ViewId),
    /// 그 에이전트가 든 탭 — 한 탭 안 여러 칸에 떠 있어도 그 탭 하나다. 여러 탭이면 모호하다.
    TabOf(Uuid),
    /// id 로 준 칸 — `view` 를 주면 그 탭의 칸이어야 한다.
    Slot { view: Option<ViewId>, slot: Uuid },
    /// 그 에이전트가 든 칸 — 여러 칸이면(같은 탭 안이어도) 모호하다.
    SlotOf(Uuid),
    /// 창 `window`(label — 창 인자는 부르는 쪽이 이미 풀었다 · `None` = main)의 보이는 탭에서 낱말이 집는 칸
    /// (`slot.resolveSpatial` 과 같은 규칙 · ADR-0068).
    At {
        token: SpatialToken,
        window: Option<WindowLabel>,
    },
}

/// 푼 대상. `slot` = 칸을 고르는 대상(`Slot` · `SlotOf` · `At`)이면 그 칸, 탭을 고르는 대상이면 `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub window: WindowLabel,
    pub view: ViewId,
    pub slot: Option<Uuid>,
}

/// 에이전트가 떠 있는 칸 하나 — 모호 후보 · [`agent_places`] 의 한 줄. `tab_index` = 그 창 탭 줄 자리(0 부터).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub window: WindowLabel,
    pub window_id: String,
    pub number: u32,
    pub view: ViewId,
    pub tab_index: usize,
    pub slot: Uuid,
}

/// 대상을 어떤 말로 골랐나 — 버스 오류 코드를 가르는 데 쓴다(TRD S28 §5-2 · §5-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetBy {
    Id,
    Agent,
    Spatial,
}

/// 대상을 못 풀었다 — 모델은 그대로다. 문구 = 한국어 사유 + 괄호 속 종류 낱말(`DockError` 와 같은 꼴). 후보는
/// 문구에 넣지 않는다 — [`Self::candidates`] 로 꺼내 부르는 쪽이 붙인다.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocateError {
    #[error("탭 없음: {0} (TabNotFound)")]
    TabNotFound(ViewId),
    #[error("칸 없음: {0} (SlotNotFound)")]
    SlotNotFound(Uuid),
    #[error("칸 {slot} 은 탭 {tab} 소속이 아님 (SlotNotInTab)")]
    SlotNotInTab { slot: Uuid, tab: ViewId },
    /// 그 에이전트가 어느 칸에도 없다. `shown` = 지금 칸에 떠 있는 에이전트의 표시 이름(장부에 없으면 id) —
    /// [`MAX_CANDIDATES`] 개까지.
    #[error("에이전트 {agent} 가 어느 칸에도 없음 (AgentNotShown)")]
    AgentNotShown { agent: Uuid, shown: Vec<String> },
    /// 그 에이전트가 여러 칸(탭을 고르는 대상이면 여러 탭)에 떠 있다. `places` = 그 칸 전부.
    #[error("에이전트 {agent} 가 여러 곳에 떠 있음 (AgentAmbiguous)")]
    AgentAmbiguous { agent: Uuid, places: Vec<Place> },
    #[error("창 없음: {0} (WindowNotFound)")]
    WindowNotFound(WindowLabel),
    /// 낱말이 그 창의 보이는 탭에서 칸을 못 집었다(그 방향에 이웃이 없다 등).
    #[error("'{}' 이 창 {window} 에서 칸을 못 집음 (NoSlotAt)", .token.as_str())]
    NoSlotAt {
        token: SpatialToken,
        window: WindowLabel,
    },
}

impl LocateError {
    pub fn by(&self) -> TargetBy {
        match self {
            LocateError::TabNotFound(_)
            | LocateError::SlotNotFound(_)
            | LocateError::SlotNotInTab { .. } => TargetBy::Id,
            LocateError::AgentNotShown { .. } | LocateError::AgentAmbiguous { .. } => {
                TargetBy::Agent
            }
            LocateError::WindowNotFound(_) | LocateError::NoSlotAt { .. } => TargetBy::Spatial,
        }
    }

    /// 다음 호출 한 번에 고칠 단서(영어 — 버스 문구 끝 `— candidates: …` 에 그대로 붙인다 · TRD S28 §5-3). 없는 이름
    /// = 떠 있는 에이전트 이름 · 모호 = 그 칸들의 `slot_id` · 든 탭(`in_tab=` — 탭 id 인자 `tab` 과 헷갈리지 않게) · 창.
    /// [`MAX_CANDIDATES`] 개까지, 그 밖의 오류는 빈 목록.
    /// 칸 id 의 머리는 `slot_id=` 다 — 다른 인자 이름이면 [`Self::candidates_as`].
    pub fn candidates(&self) -> Vec<String> {
        self.candidates_as("slot_id")
    }

    /// [`Self::candidates`] 와 같되 모호 후보의 칸 id 머리를 `key=` 로 단다 — 그 값을 넘길 인자 이름(`slot_id` ·
    /// `beside_slot`)을 쓰면 다음 호출이 그대로 옮겨 적는다.
    pub fn candidates_as(&self, key: &str) -> Vec<String> {
        match self {
            LocateError::AgentNotShown { shown, .. } => {
                shown.iter().take(MAX_CANDIDATES).cloned().collect()
            }
            LocateError::AgentAmbiguous { places, .. } => places
                .iter()
                .take(MAX_CANDIDATES)
                .map(|p| {
                    format!(
                        "{key}={} (in_tab={} · window={} · window_id={})",
                        p.slot, p.view, p.window, p.window_id
                    )
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// `target` 을 지금 모델에서 푼다 — 창 탭 목록에 든 탭만 본다(창에 아직 안 든 임시 탭은 대상이 아니다).
// ADR-0307
pub fn locate(mgr: &ViewManager, target: &Target) -> Result<Located, LocateError> {
    match target {
        Target::Tab(view) => {
            let window = tab_window(mgr, *view).ok_or(LocateError::TabNotFound(*view))?;
            Ok(Located {
                window,
                view: *view,
                slot: None,
            })
        }
        Target::TabOf(agent) => {
            let places = agent_places(mgr, *agent);
            let first = one_or_ambiguous(mgr, *agent, &places, |a, b| a.view == b.view)?;
            Ok(Located {
                window: first.window.clone(),
                view: first.view,
                slot: None,
            })
        }
        Target::Slot {
            view: Some(view),
            slot,
        } => {
            let window = tab_window(mgr, *view).ok_or(LocateError::TabNotFound(*view))?;
            if tree::contains_slot(&mgr.views[view].layout, *slot) {
                return Ok(Located {
                    window,
                    view: *view,
                    slot: Some(*slot),
                });
            }
            Err(match slot_home(mgr, *slot) {
                Some(_) => LocateError::SlotNotInTab {
                    slot: *slot,
                    tab: *view,
                },
                None => LocateError::SlotNotFound(*slot),
            })
        }
        Target::Slot { view: None, slot } => {
            let (window, view) = slot_home(mgr, *slot).ok_or(LocateError::SlotNotFound(*slot))?;
            Ok(Located {
                window,
                view,
                slot: Some(*slot),
            })
        }
        Target::SlotOf(agent) => {
            let places = agent_places(mgr, *agent);
            let first = one_or_ambiguous(mgr, *agent, &places, |a, b| a.slot == b.slot)?;
            Ok(Located {
                window: first.window.clone(),
                view: first.view,
                slot: Some(first.slot),
            })
        }
        Target::At { token, window } => {
            let label = window.as_deref().unwrap_or(MAIN_WINDOW_LABEL);
            let shown = mgr
                .windows
                .get(label)
                .and_then(|w| mgr.views.get(&w.active))
                .ok_or_else(|| LocateError::WindowNotFound(label.to_string()))?;
            let slot =
                resolve_spatial(&shown.layout, shown.focused_slot_id, *token).ok_or_else(|| {
                    LocateError::NoSlotAt {
                        token: *token,
                        window: label.to_string(),
                    }
                })?;
            Ok(Located {
                window: label.to_string(),
                view: shown.id,
                slot: Some(slot),
            })
        }
    }
}

// 자리가 하나로 모이면 그 첫 자리, 없으면 `AgentNotShown`, 갈리면 `AgentAmbiguous`. `same` = 같은 대상으로 칠 두 자리.
fn one_or_ambiguous<'p>(
    mgr: &ViewManager,
    agent: Uuid,
    places: &'p [Place],
    same: impl Fn(&Place, &Place) -> bool,
) -> Result<&'p Place, LocateError> {
    let first = places.first().ok_or_else(|| LocateError::AgentNotShown {
        agent,
        shown: shown_agents(mgr, MAX_CANDIDATES),
    })?;
    if places.iter().all(|p| same(p, first)) {
        Ok(first)
    } else {
        Err(LocateError::AgentAmbiguous {
            agent,
            places: places.to_vec(),
        })
    }
}

/// 에이전트 `agent` 가 떠 있는 칸 전부 — 창 번호 순 → 탭 줄 순 → 트리 전위 순.
pub fn agent_places(mgr: &ViewManager, agent: Uuid) -> Vec<Place> {
    let mut out = Vec::new();
    for (label, w) in windows_in_order(mgr) {
        for (tab_index, view) in tabs_of(mgr, w) {
            for slot in tree::slot_ids(&view.layout) {
                let content = tree::find_slot(&view.layout, slot);
                if content.is_some_and(|c| holds(c, agent)) {
                    out.push(Place {
                        window: label.clone(),
                        window_id: w.window_id.clone(),
                        number: w.number,
                        view: view.id,
                        tab_index,
                        slot,
                    });
                }
            }
        }
    }
    out
}

/// 지금 칸에 떠 있는 에이전트의 표시 이름(장부에 없으면 그 id) — 겹침 없이 `max` 개까지 · 순서는 모듈 머리.
pub fn shown_agents(mgr: &ViewManager, max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (_, w) in windows_in_order(mgr) {
        for (_, view) in tabs_of(mgr, w) {
            for slot in tree::slot_ids(&view.layout) {
                let Some(id) = tree::find_slot(&view.layout, slot).and_then(SlotContent::agent_id)
                else {
                    continue;
                };
                let name = mgr.agent_name(id).unwrap_or(id).to_string();
                if !out.contains(&name) {
                    out.push(name);
                    if out.len() >= max {
                        return out;
                    }
                }
            }
        }
    }
    out
}

/// 이음새 도킹의 기본 몫 — 마우스와 같은 공식 [`bands::attach_share`](원래 크기 = 출처 탭 창의 캔버스 · 칸 = 대상 칸
/// px 틀, `Outer` 면 대상 창 캔버스 전체). 어느 캔버스든 모르거나 대상 칸이 없으면 [`UNMEASURED_SHARE`].
///
/// ★마우스와 다르다★: 마우스의 원래 크기는 웹이 잰 끌기 크기(원본 창이 최대화면 복원 크기)이고 모델에는 그 입력이
/// 없어 캔버스로 갈음한다(TRD S28 §4-2 · §11-6).
pub fn seam_share(
    mgr: &ViewManager,
    source: ViewId,
    target: ViewId,
    anchor: DockAnchor,
    side: DockSide,
) -> f64 {
    let (Some(original), Some(canvas)) = (canvas_of(mgr, source), canvas_of(mgr, target)) else {
        return UNMEASURED_SHARE;
    };
    let frame = match anchor {
        DockAnchor::Outer => PxRect {
            x0: 0,
            y0: 0,
            x1: canvas.w,
            y1: canvas.h,
        },
        DockAnchor::Slot(slot) => {
            let rect = mgr.views.get(&target).and_then(|v| {
                geometry::compute(&v.layout)
                    .slots
                    .into_iter()
                    .find(|r| r.slot_id == slot)
            });
            match rect {
                Some(rect) => geometry::frame_px(canvas.w, canvas.h, &rect),
                None => return UNMEASURED_SHARE,
            }
        }
    };
    let original = SizeCss {
        w: f64::from(original.w),
        h: f64::from(original.h),
    };
    bands::attach_share(original, side, frame)
}

fn canvas_of(mgr: &ViewManager, view: ViewId) -> Option<CanvasPx> {
    mgr.view_owner
        .get(&view)
        .and_then(|label| mgr.windows.get(label))
        .and_then(|w| w.canvas)
}

/// 모델의 창 — 창 번호 순(`window.list` 와 같다).
pub(crate) fn windows_in_order(mgr: &ViewManager) -> Vec<(&WindowLabel, &WindowTabs)> {
    let mut windows: Vec<_> = mgr.windows.iter().collect();
    windows.sort_by_key(|(_, w)| w.number);
    windows
}

/// 창 `w` 의 탭 — (탭 줄 자리, 탭). View 가 없는 항목은 건너뛴다(불변식 1 이 서면 없다).
pub(crate) fn tabs_of<'m>(
    mgr: &'m ViewManager,
    w: &'m WindowTabs,
) -> impl Iterator<Item = (usize, &'m View)> + 'm {
    w.tabs
        .iter()
        .enumerate()
        .filter_map(|(i, id)| mgr.views.get(id).map(|v| (i, v)))
}

fn holds(content: &SlotContent, agent: Uuid) -> bool {
    content
        .agent_id()
        .and_then(|id| Uuid::parse_str(id).ok())
        .is_some_and(|id| id == agent)
}

// `view` 를 탭으로 가진 창 — View · 소유 쌍 · 탭 목록이 다 서야 한다(`ViewManager` 의 도킹 판정과 같은 규칙).
fn tab_window(mgr: &ViewManager, view: ViewId) -> Option<WindowLabel> {
    if !mgr.views.contains_key(&view) {
        return None;
    }
    let label = mgr.view_owner.get(&view)?;
    mgr.windows
        .get(label)?
        .tabs
        .contains(&view)
        .then(|| label.clone())
}

// 칸 `slot` 이 든 탭(창 탭 목록에 든 것만) — (창, 탭).
fn slot_home(mgr: &ViewManager, slot: Uuid) -> Option<(WindowLabel, ViewId)> {
    windows_in_order(mgr).into_iter().find_map(|(label, w)| {
        tabs_of(mgr, w)
            .find(|(_, v)| tree::contains_slot(&v.layout, slot))
            .map(|(_, v)| (label.clone(), v.id))
    })
}
