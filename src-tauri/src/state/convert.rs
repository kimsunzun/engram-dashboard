//! 화면 상태의 메모리 모양(`ViewManager` · 트리 칸) ↔ 영속 모양(`schema::WindowEntry`) — TRD S21-storage §6-2 ·
//! §6-3. 진입점 = [`to_persisted`](기록기 · 부팅 실행 표식) · [`ViewManager::from_persisted`](부팅 · 런타임 수락) ·
//! [`StateRevision`](기록기의 변경 번호).
//!
//! ★두 방향 모두 칸을 이름으로 풀어 쓴다 — `..` 도 와일드카드 갈래도 쓰지 않는다★. `View` · `LayoutNode` ·
//! `SlotContent` · `WindowTabs` · `WindowAttrs` 나 영속 쪽에 칸 · 변형이 늘면 여기서 컴파일이 깨진다. `..` 로 넘기면
//! 새 칸이 조용히 영속에서 빠진다(§9-2 「P3a 가 넘긴 것」).

use std::collections::HashSet;
use std::fmt;

use serde_json::{Map, Value};
use uuid::Uuid;

use super::schema::{
    Bounds, PersistedContent, PersistedNode, TabEntry, TabStrip, WindowEntry, WindowKind,
};
use crate::layout::manager::{RestoredWindow, ViewId, WindowLabel, WindowTabs};
use crate::layout::{
    tree, LabelSource, LayoutNode, SlotContent, View, ViewManager, WindowAttrs, WindowBounds,
    MAIN_WINDOW_LABEL,
};
use crate::ui_settings::UiTheme;

/// 트리 창의 영속 id — label 과 같다(설정 창 · `tauri.conf.json`). main 도 label 이 곧 영속 id 다.
pub const TREE_WINDOW_ID: &str = "agent-tree";

/// 기록기의 변경 번호 — 레이아웃 `version` · 창 속성 `attrs_rev` · 트리 칸 번호. 기록기는 같은지만 본다(§6-4).
///
/// 두 락을 겹쳐 잡지 않는다(§6-3) — 레이아웃 쪽은 그 락 안에서 읽고 트리 번호는 따로 읽어 넘긴다. 둘 사이에
/// 원자성은 필요 없다 — 칸마다 같은지만 보고, 번호는 변경과 같은 임계구역에서 오른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateRevision {
    pub layout: u64,
    pub attrs: u64,
    pub tree: u64,
}

impl StateRevision {
    pub fn of(layout: &ViewManager, tree_rev: u64) -> Self {
        StateRevision {
            layout: layout.version,
            attrs: layout.attrs_rev(),
            tree: tree_rev,
        }
    }
}

// ── 메모리 → 영속 ───────────────────────────────────────────────────────────

/// 창 전부 — main · 트리 · 팝아웃(영속 id 순) 순서. 부르는 쪽은 `ViewManager` 락 안에서 부르고, 트리 칸은 그 락을
/// 잡기 전에 읽어 값으로 넘긴다(두 락을 겹쳐 잡지 않는다 — §6-3).
///
/// - 모르는 내용 슬롯(곁표 항목이 있고 메모리 내용이 `Empty`)은 원문으로 적는다 — 새 판이 쓴 슬롯이 이 판의 저장으로
///   지워지지 않는다(ADR-0060).
/// - 화면 실측값(`canvas` · `metrics`)은 싣지 않는다(§6-1).
/// - 자리를 본 적 없는 창(만들 때부터 최대화 · 첫 `Moved` 전)도 `bounds: null` 로 적는다 — ★자리 때문에 창을 빼지
///   않는다★: main 이면 그 탭이 다음 부팅에 사라진다.
pub fn to_persisted(layout: &ViewManager, tree: WindowAttrs) -> Vec<WindowEntry> {
    let mut entries = Vec::new();
    if let Some(main) = layout.windows.get(MAIN_WINDOW_LABEL) {
        entries.push(tabbed_entry(layout, main, WindowKind::Main));
    }
    entries.push(window_entry(
        TREE_WINDOW_ID.to_string(),
        WindowKind::Tree,
        tree,
    ));
    let mut popouts: Vec<&WindowTabs> = layout
        .windows
        .iter()
        .filter(|(label, _)| label.as_str() != MAIN_WINDOW_LABEL)
        .map(|(_, window)| window)
        .collect();
    // 표 순회 순서(HashMap)를 파일에 새지 않게 — 같은 화면이면 같은 글이 나온다.
    popouts.sort_by(|a, b| a.window_id.cmp(&b.window_id));
    for popout in popouts {
        entries.push(tabbed_entry(layout, popout, WindowKind::Popout));
    }
    entries
}

fn tabbed_entry(
    layout: &ViewManager,
    window: &WindowTabs,
    kind: fn(TabStrip) -> WindowKind,
) -> WindowEntry {
    let WindowTabs {
        tabs,
        active,
        canvas: _,
        metrics: _,
        window_id,
        attrs,
    } = window;
    let tabs = tabs
        .iter()
        .filter_map(|id| layout.views.get(id))
        .map(|view| tab_entry(layout, view))
        .collect();
    window_entry(
        window_id.clone(),
        kind(TabStrip {
            active_tab: *active,
            tabs,
        }),
        *attrs,
    )
}

fn window_entry(id: String, kind: WindowKind, attrs: WindowAttrs) -> WindowEntry {
    let WindowAttrs {
        theme,
        bounds,
        maximized,
    } = attrs;
    WindowEntry {
        id,
        kind,
        theme,
        bounds: bounds.map(|bounds| Bounds {
            x: bounds.x(),
            y: bounds.y(),
            w: bounds.w(),
            h: bounds.h(),
        }),
        maximized,
    }
}

fn tab_entry(layout: &ViewManager, view: &View) -> TabEntry {
    let View {
        id,
        name,
        layout: node,
        focused_slot_id,
    } = view;
    TabEntry {
        id: *id,
        name: name.clone(),
        focused_slot_id: *focused_slot_id,
        layout: persisted_node(layout, *id, node),
    }
}

fn persisted_node(layout: &ViewManager, view: ViewId, node: &LayoutNode) -> PersistedNode {
    match node {
        LayoutNode::Slot { id, content } => PersistedNode::Slot {
            id: *id,
            content: persisted_content(content, layout.unknown_content(view, *id)),
        },
        LayoutNode::Split {
            id,
            dir,
            ratio,
            a,
            b,
        } => PersistedNode::Split {
            id: *id,
            dir: *dir,
            ratio: *ratio,
            a: Box::new(persisted_node(layout, view, a)),
            b: Box::new(persisted_node(layout, view, b)),
        },
    }
}

// 원문은 메모리 내용이 아직 `Empty` 일 때만 되돌려 쓴다 — 그 슬롯에 무엇이든 쓰면 곁표 항목이 지워지지만, 어긋나도
// 사용자가 놓은 내용이 이긴다.
fn persisted_content(
    content: &SlotContent,
    unknown: Option<&Map<String, Value>>,
) -> PersistedContent {
    PersistedContent::Known(match content {
        SlotContent::Empty => match unknown {
            Some(raw) => return PersistedContent::Unknown(raw.clone()),
            None => SlotContent::Empty,
        },
        SlotContent::Agent { agent_id } => SlotContent::Agent {
            agent_id: agent_id.clone(),
        },
        SlotContent::AgentList => SlotContent::AgentList,
        SlotContent::PresetPalette => SlotContent::PresetPalette,
        SlotContent::Usage {
            show_claude,
            show_codex,
        } => SlotContent::Usage {
            show_claude: *show_claude,
            show_codex: *show_codex,
        },
    })
}

// ── 영속 → 메모리 ───────────────────────────────────────────────────────────

/// [`ViewManager::from_persisted`] 의 결과.
pub struct Restored {
    pub layout: ViewManager,
    pub tree: WindowAttrs,
    /// 고치거나 건너뛴 것 — 로그는 부르는 쪽이 낸다.
    pub warnings: Vec<RestoreWarning>,
}

/// 영속 창 묶음을 메모리 모델로 옮기며 고치거나 건너뛴 것 하나. 같은 id 가 겹치면 언제나 뒤엣것을 버린다 — 단
/// 건너뛴 창은 아무 id 도(창 · 탭 · 노드) 쥐지 않고, 건너뛴 이유만 남긴다(자리 · 비율 · 모르는 내용은 알리지 않는다).
#[derive(Debug, Clone, PartialEq)]
pub enum RestoreWarning {
    /// id 가 그 종류의 것이 아니다 — main = `main`, 트리 = `agent-tree`, 팝아웃 = UUID.
    IdKindMismatch {
        id: String,
        kind: &'static str,
    },
    DuplicateWindow {
        id: String,
    },
    DuplicateTab {
        window_id: String,
        view: Uuid,
    },
    /// 슬롯 · 분할 id 가 이 탭 안이나 앞선 탭과 겹친다 — 그 탭을 건너뛴다.
    DuplicateNode {
        window_id: String,
        view: Uuid,
        node: Uuid,
    },
    /// `ratio` = 받은 값. 범위로 자르고, 유한하지 않으면 기본 비율로 둔다.
    RatioRepaired {
        window_id: String,
        view: Uuid,
        split: Uuid,
        ratio: f64,
    },
    /// 활성 탭이 남은 탭에 없다 — 첫 탭을 활성으로 둔다.
    ActiveRepaired {
        window_id: String,
        missing: Uuid,
    },
    /// main 항목이 없거나 탭이 하나도 안 남았다 — 빈 탭 하나로 시작한다(불변식 4).
    MainWithoutTabs,
    PopoutWithoutTabs {
        id: String,
    },
    /// 발급기가 준 label 이 이미 쓰였다 — 그 팝아웃을 건너뛴다.
    LabelTaken {
        id: String,
        label: String,
    },
    /// 쓸 수 없는 자리(`WindowBounds::new` 가 거절) — 기본 자리로 연다.
    BoundsDropped {
        window_id: String,
    },
    /// 이 판이 모르는 슬롯 내용 — 원문을 곁표에 쥔 채 빈 칸으로 선다. `kind` = 원문의 `type` 문자열.
    UnknownContent {
        window_id: String,
        view: Uuid,
        slot: Uuid,
        kind: Option<String>,
    },
    /// 고친 입력으로도 모델을 못 세웠다 — 이 모듈의 결함이다. 기본 화면으로 시작한다.
    Internal(String),
}

impl fmt::Display for RestoreWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RestoreWarning::IdKindMismatch { id, kind } => {
                write!(f, "창 {id} 은 {kind} 창의 id 가 아니다 — 건너뛴다")
            }
            RestoreWarning::DuplicateWindow { id } => {
                write!(f, "창 {id} 이 겹친다 — 뒤엣것을 건너뛴다")
            }
            RestoreWarning::DuplicateTab { window_id, view } => {
                write!(f, "창 {window_id} 의 탭 {view} 가 겹친다 — 건너뛴다")
            }
            RestoreWarning::DuplicateNode {
                window_id,
                view,
                node,
            } => write!(
                f,
                "창 {window_id} 의 탭 {view} 에서 노드 {node} 가 겹친다 — 그 탭을 건너뛴다"
            ),
            RestoreWarning::RatioRepaired {
                window_id,
                view,
                split,
                ratio,
            } => write!(
                f,
                "창 {window_id} 의 탭 {view} 분할 {split} 의 비율 {ratio} 을 범위 안으로 고친다"
            ),
            RestoreWarning::ActiveRepaired { window_id, missing } => write!(
                f,
                "창 {window_id} 의 활성 탭 {missing} 이 없다 — 첫 탭을 활성으로 둔다"
            ),
            RestoreWarning::MainWithoutTabs => {
                write!(f, "main 창의 탭이 없다 — 빈 탭 하나로 시작한다")
            }
            RestoreWarning::PopoutWithoutTabs { id } => {
                write!(f, "팝아웃 {id} 에 탭이 없다 — 건너뛴다")
            }
            RestoreWarning::LabelTaken { id, label } => {
                write!(
                    f,
                    "팝아웃 {id} 에 줄 label {label} 이 이미 쓰였다 — 건너뛴다"
                )
            }
            RestoreWarning::BoundsDropped { window_id } => {
                write!(f, "창 {window_id} 의 자리를 쓸 수 없다 — 기본 자리로 연다")
            }
            RestoreWarning::UnknownContent {
                window_id,
                view,
                slot,
                kind,
            } => write!(
                f,
                "창 {window_id} 의 탭 {view} 슬롯 {slot} — 이 판이 모르는 내용({}) 을 그대로 쥔다",
                kind.as_deref().unwrap_or("종류 없음")
            ),
            RestoreWarning::Internal(reason) => {
                write!(
                    f,
                    "복원 모델을 세우지 못했다 — 기본 화면으로 시작한다: {reason}"
                )
            }
        }
    }
}

impl ViewManager {
    /// 영속 창 묶음(`codec::decode` 결과의 `windows`)으로 새 모델과 트리 칸을 세운다 — 순수하다(파일 · 창 · 락 0 ·
    /// 실패 없음). 코덱은 모양 검사를 하지 않으므로 불변식 1–4 와 창 묶음의 모양을 여기서 다시 세운다(고치는 규칙 =
    /// [`RestoreWarning`]). 팝아웃마다 `labels` 에서 새 label 을 받는다 — 영속 id 는 그대로, label 은 이 부팅만의
    /// 것이다(§6-3).
    ///
    /// ★결과의 `version` · `attrs_rev` 는 의미가 없다★ — 부팅은 기록기를 실행 표식 쓰기 때의 번호로 시작하므로 그대로
    /// 쓰면 된다. ★런타임 수락(P3c1 · §6-7 ③)은 이 결과를 살아 있는 모델과 바꿔치지 않는다★:
    /// - `version = 지금 version + 1` · `attrs_rev = 지금 attrs_rev + 1` 을 직접 세운다. 이 결과의 번호(0 근처)를 쓰면
    ///   번호가 되돌아가고, 기록기는 같은지만 보므로 마지막으로 본 값과 다시 같아지는 순간 그 사이의 변경을 놓친다.
    /// - 곁표(모르는 내용)를 View 와 함께 옮긴다 — View 만 옮기면 그 슬롯이 빈 칸으로 저장돼 원문이 사라진다. 옛
    ///   View 는 `remove_view` 로 지운다(그 곁표 항목을 함께 거둔다).
    /// - main 항목은 탭 · 속성만 바꾼다 — 통째로 바꾸면 측정값(`canvas` · `metrics`)을 웹뷰가 다시 보고할 때까지 잃는다.
    /// - 트리 칸은 `TreeAttrs::set` 으로 갈아끼운다(번호를 올린다).
    pub(crate) fn from_persisted(windows: Vec<WindowEntry>, labels: &dyn LabelSource) -> Restored {
        let mut restorer = Restorer::default();
        for entry in windows {
            restorer.window(entry, labels);
        }
        restorer.finish()
    }
}

#[derive(Default)]
struct Restorer {
    warnings: Vec<RestoreWarning>,
    main: Option<Pending>,
    tree: Option<WindowAttrs>,
    popouts: Vec<(WindowLabel, String, Pending)>,
    popout_ids: HashSet<Uuid>,
    views: HashSet<Uuid>,
    nodes: HashSet<Uuid>,
}

struct Pending {
    window: RestoredWindow,
    unknown: Vec<(ViewId, Uuid, Map<String, Value>)>,
}

/// 한 창의 탭 묶음 — 아직 `Restorer` 에 싣지 않았다(id 를 쥐지도 경고를 내지도 않았다). 창을 남기기로 정한 뒤
/// [`Restorer::commit`] 이 싣는다.
#[derive(Default)]
struct Staged {
    tabs: Vec<View>,
    active: Option<ViewId>,
    nodes: HashSet<Uuid>,
    unknown: Vec<(ViewId, Uuid, Map<String, Value>)>,
    warnings: Vec<RestoreWarning>,
}

impl Restorer {
    fn window(&mut self, entry: WindowEntry, labels: &dyn LabelSource) {
        let WindowEntry {
            id,
            kind,
            theme,
            bounds,
            maximized,
        } = entry;
        match kind {
            WindowKind::Main(strip) => {
                if id != MAIN_WINDOW_LABEL {
                    return self.mismatch(id, "main");
                }
                if self.main.is_some() {
                    return self.warn(RestoreWarning::DuplicateWindow { id });
                }
                let staged = self.stage(&id, strip);
                let attrs = self.attrs(&id, theme, bounds, maximized);
                self.main = Some(self.commit(attrs, staged));
            }
            WindowKind::Tree => {
                if id != TREE_WINDOW_ID {
                    return self.mismatch(id, "tree");
                }
                if self.tree.is_some() {
                    return self.warn(RestoreWarning::DuplicateWindow { id });
                }
                self.tree = Some(self.attrs(&id, theme, bounds, maximized));
            }
            WindowKind::Popout(strip) => {
                let Ok(uuid) = Uuid::parse_str(&id) else {
                    return self.mismatch(id, "popout");
                };
                // 같은 UUID 의 다른 철자(대문자 · 중괄호)도 같은 창이다 — 정규 철자로 적고 견준다.
                let id = uuid.to_string();
                if self.popout_ids.contains(&uuid) {
                    return self.warn(RestoreWarning::DuplicateWindow { id });
                }
                // 버릴지를 다 정한 뒤에만 싣는다(id · 경고 · 자리) — 버린 창이 쥔 id 에 뒤 창(main 포함)의 탭이
                // 겹친다고 밀려나면 살릴 수 있던 내용을 잃는다.
                let staged = self.stage(&id, strip);
                if staged.tabs.is_empty() {
                    // 탭이 겹쳐서 다 빠졌으면 그 경고가 곧 버린 이유다.
                    self.warnings.extend(staged.warnings);
                    return self.warn(RestoreWarning::PopoutWithoutTabs { id });
                }
                let label = labels.next_label();
                let taken = [MAIN_WINDOW_LABEL, TREE_WINDOW_ID].contains(&label.as_str())
                    || self.popouts.iter().any(|(used, _, _)| *used == label);
                if taken {
                    return self.warn(RestoreWarning::LabelTaken { id, label });
                }
                self.popout_ids.insert(uuid);
                let attrs = self.attrs(&id, theme, bounds, maximized);
                let pending = self.commit(attrs, staged);
                self.popouts.push((label, id, pending));
            }
        }
    }

    fn warn(&mut self, warning: RestoreWarning) {
        self.warnings.push(warning);
    }

    fn mismatch(&mut self, id: String, kind: &'static str) {
        self.warn(RestoreWarning::IdKindMismatch { id, kind });
    }

    fn attrs(
        &mut self,
        window_id: &str,
        theme: Option<UiTheme>,
        bounds: Option<Bounds>,
        maximized: bool,
    ) -> WindowAttrs {
        let bounds = bounds.and_then(|Bounds { x, y, w, h }| {
            let usable = WindowBounds::new(x, y, w, h);
            if usable.is_none() {
                self.warn(RestoreWarning::BoundsDropped {
                    window_id: window_id.to_string(),
                });
            }
            usable
        });
        WindowAttrs {
            theme,
            bounds,
            maximized,
        }
    }

    fn stage(&self, window_id: &str, strip: TabStrip) -> Staged {
        let TabStrip { active_tab, tabs } = strip;
        let mut staged = Staged::default();
        for tab in tabs {
            self.stage_tab(window_id, tab, &mut staged);
        }
        staged.active = if staged.tabs.iter().any(|v| v.id == active_tab) {
            Some(active_tab)
        } else {
            let first = staged.tabs.first().map(|v| v.id);
            if first.is_some() {
                staged.warnings.push(RestoreWarning::ActiveRepaired {
                    window_id: window_id.to_string(),
                    missing: active_tab,
                });
            }
            first
        };
        staged
    }

    fn stage_tab(&self, window_id: &str, tab: TabEntry, staged: &mut Staged) {
        let TabEntry {
            id,
            name,
            focused_slot_id,
            layout,
        } = tab;
        if self.views.contains(&id) || staged.tabs.iter().any(|v| v.id == id) {
            staged.warnings.push(RestoreWarning::DuplicateTab {
                window_id: window_id.to_string(),
                view: id,
            });
            return;
        }
        let mut found = Found::default();
        let layout = restore_node(layout, &mut found);
        let mut fresh = HashSet::new();
        if let Some(&node) = found.nodes.iter().find(|node| {
            self.nodes.contains(node) || staged.nodes.contains(node) || !fresh.insert(**node)
        }) {
            staged.warnings.push(RestoreWarning::DuplicateNode {
                window_id: window_id.to_string(),
                view: id,
                node,
            });
            return;
        }
        staged.nodes.extend(found.nodes);
        for (split, ratio) in found.ratios {
            staged.warnings.push(RestoreWarning::RatioRepaired {
                window_id: window_id.to_string(),
                view: id,
                split,
                ratio,
            });
        }
        for (slot, raw) in found.unknown {
            staged.warnings.push(RestoreWarning::UnknownContent {
                window_id: window_id.to_string(),
                view: id,
                slot,
                kind: raw.get("type").and_then(Value::as_str).map(str::to_string),
            });
            staged.unknown.push((id, slot, raw));
        }
        staged.tabs.push(View {
            id,
            name,
            layout,
            focused_slot_id,
        });
    }

    fn commit(&mut self, attrs: WindowAttrs, staged: Staged) -> Pending {
        let Staged {
            tabs,
            active,
            nodes,
            unknown,
            warnings,
        } = staged;
        self.views.extend(tabs.iter().map(|v| v.id));
        self.nodes.extend(nodes);
        self.warnings.extend(warnings);
        Pending {
            window: RestoredWindow {
                attrs,
                tabs,
                active,
            },
            unknown,
        }
    }

    fn finish(self) -> Restored {
        let Restorer {
            mut warnings,
            main,
            tree,
            popouts,
            popout_ids: _,
            views: _,
            nodes: _,
        } = self;
        let main = main.unwrap_or(Pending {
            window: RestoredWindow {
                attrs: WindowAttrs::default(),
                tabs: Vec::new(),
                active: None,
            },
            unknown: Vec::new(),
        });
        if main.window.tabs.is_empty() {
            warnings.push(RestoreWarning::MainWithoutTabs);
        }
        let mut unknown = main.unknown;
        let popouts = popouts
            .into_iter()
            .map(|(label, id, pending)| {
                unknown.extend(pending.unknown);
                (label, id, pending.window)
            })
            .collect();
        let tree = tree.unwrap_or_default();
        let mut layout = match ViewManager::from_restored(main.window, popouts) {
            Ok(layout) => layout,
            Err(reason) => {
                warnings.push(RestoreWarning::Internal(reason));
                return Restored {
                    layout: ViewManager::new(),
                    tree,
                    warnings,
                };
            }
        };
        for (view, slot, raw) in unknown {
            if let Err(e) = layout.set_unknown_content(view, slot, raw) {
                warnings.push(RestoreWarning::Internal(e.to_string()));
            }
        }
        Restored {
            layout,
            tree,
            warnings,
        }
    }
}

/// 한 탭의 트리를 내려가며 모은 것 — 노드 id(전위 순) · 모르는 내용 · 고친 비율(분할 id, 받은 값).
#[derive(Default)]
struct Found {
    nodes: Vec<Uuid>,
    unknown: Vec<(Uuid, Map<String, Value>)>,
    ratios: Vec<(Uuid, f64)>,
}

fn restore_node(node: PersistedNode, found: &mut Found) -> LayoutNode {
    match node {
        PersistedNode::Slot { id, content } => {
            found.nodes.push(id);
            let content = match content {
                PersistedContent::Known(content) => known_content(content),
                PersistedContent::Unknown(raw) => {
                    found.unknown.push((id, raw));
                    SlotContent::Empty
                }
            };
            LayoutNode::Slot { id, content }
        }
        PersistedNode::Split {
            id,
            dir,
            ratio,
            a,
            b,
        } => {
            found.nodes.push(id);
            // 범위 밖 비율은 칸 폭을 0 이나 음수로 만든다 — 쓰기 경로(`set_split_ratio`)와 같은 범위로 자른다.
            let fixed = if ratio.is_finite() {
                tree::clamp_ratio(ratio)
            } else {
                tree::SPLIT_RATIO
            };
            if fixed != ratio {
                found.ratios.push((id, ratio));
            }
            LayoutNode::Split {
                id,
                dir,
                ratio: fixed,
                a: Box::new(restore_node(*a, found)),
                b: Box::new(restore_node(*b, found)),
            }
        }
    }
}

// 그대로 옮긴다 — 칸 · 변형이 늘면 여기서 깨져, 복원 쪽에서 그 값을 어떻게 다룰지(못 쓸 값 버리기 등 — §6-0 ⑤)를
// 정하게 한다.
fn known_content(content: SlotContent) -> SlotContent {
    match content {
        SlotContent::Empty => SlotContent::Empty,
        SlotContent::Agent { agent_id } => SlotContent::Agent { agent_id },
        SlotContent::AgentList => SlotContent::AgentList,
        SlotContent::PresetPalette => SlotContent::PresetPalette,
        SlotContent::Usage {
            show_claude,
            show_codex,
        } => SlotContent::Usage {
            show_claude,
            show_codex,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use serde_json::{json, Map, Value};
    use uuid::Uuid;

    use super::{to_persisted, RestoreWarning, Restored, StateRevision, TREE_WINDOW_ID};
    use crate::layout::{
        tree, LabelSource, SlotContent, SplitDir, ViewManager, WindowAttrs, WindowBounds,
        WindowPlacement, MAIN_WINDOW_LABEL,
    };
    use crate::state::codec;
    use crate::state::schema::{
        Bounds, PersistedContent, PersistedNode, StateFile, TabEntry, TabStrip, WindowEntry,
        WindowKind, STATE_VERSION,
    };
    use crate::ui_settings::UiTheme;

    #[derive(Default)]
    struct Labels(AtomicU64);

    impl LabelSource for Labels {
        fn next_label(&self) -> String {
            format!("slot-popup-{}", self.0.fetch_add(1, Ordering::SeqCst) + 1)
        }

        fn tab_name(&self, label: &str) -> String {
            label.to_string()
        }
    }

    /// 늘 같은 label 을 준다.
    struct Fixed(&'static str);

    impl LabelSource for Fixed {
        fn next_label(&self) -> String {
            self.0.to_string()
        }

        fn tab_name(&self, label: &str) -> String {
            label.to_string()
        }
    }

    fn raw(kind: &str) -> Map<String, Value> {
        json!({ "type": kind, "opts": { "depth": 3, "list": [1, 2.5, null] } })
            .as_object()
            .unwrap()
            .clone()
    }

    fn bounds_at(x: f64) -> WindowBounds {
        WindowBounds::new(x, 10.0, 800.0, 600.0).unwrap()
    }

    fn place(mgr: &mut ViewManager, label: &str, x: f64, maximized: bool) {
        let normal = WindowPlacement {
            minimized: false,
            maximized: false,
            bounds: Some(bounds_at(x)),
        };
        mgr.observe_window_placement(label, normal).unwrap();
        if maximized {
            let max = WindowPlacement {
                minimized: false,
                maximized: true,
                bounds: None,
            };
            mgr.observe_window_placement(label, max).unwrap();
        }
    }

    fn main_active(mgr: &ViewManager) -> Uuid {
        mgr.windows[MAIN_WINDOW_LABEL].active
    }

    fn first_slot(mgr: &ViewManager, view: Uuid) -> Uuid {
        tree::first_slot_id(&mgr.views[&view].layout)
    }

    /// main 두 탭(분할 · 모든 내용 종류 · 모르는 내용 · 바꾼 비율 · 이름) + 팝아웃 둘 + 창 속성 전부 + 트리 칸.
    fn rich() -> (ViewManager, WindowAttrs) {
        let mut mgr = ViewManager::new();
        let first = main_active(&mgr);
        let root = first_slot(&mgr, first);
        let right = mgr.split_slot(first, root, SplitDir::LeftRight).unwrap();
        let bottom = mgr.split_slot(first, right, SplitDir::TopBottom).unwrap();
        mgr.assign_agent(first, root, "a-1".into()).unwrap();
        mgr.set_slot_content(
            first,
            right,
            SlotContent::Usage {
                show_claude: false,
                show_codex: true,
            },
        )
        .unwrap();
        mgr.set_unknown_content(first, bottom, raw("file_tree"))
            .unwrap();
        let split = tree::list_splits(&mgr.views[&first].layout)[0].id;
        mgr.set_split_ratio(first, split, 0.3).unwrap();
        mgr.set_focused_slot(first, root).unwrap();
        mgr.rename_tab(first, "작업".into()).unwrap();
        let second = mgr
            .create_tab(MAIN_WINDOW_LABEL, Some("둘".into()))
            .unwrap();
        let s = first_slot(&mgr, second);
        mgr.set_slot_content(second, s, SlotContent::AgentList)
            .unwrap();
        mgr.switch_tab(MAIN_WINDOW_LABEL, first).unwrap();
        mgr.set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::Light))
            .unwrap();
        place(&mut mgr, MAIN_WINDOW_LABEL, -1920.0, true);

        let p1 = mgr.create_window("slot-popup-7").unwrap();
        let s = first_slot(&mgr, p1);
        mgr.set_slot_content(p1, s, SlotContent::PresetPalette)
            .unwrap();
        mgr.set_window_theme("slot-popup-7", Some(UiTheme::EInk))
            .unwrap();
        place(&mut mgr, "slot-popup-7", 100.0, false);
        let p2 = mgr.create_window("slot-popup-8").unwrap();
        let s = first_slot(&mgr, p2);
        mgr.set_unknown_content(p2, s, raw("from_the_future"))
            .unwrap();
        mgr.create_tab("slot-popup-8", None).unwrap();
        place(&mut mgr, "slot-popup-8", 200.5, false);

        let tree = WindowAttrs {
            theme: Some(UiTheme::Dark),
            bounds: Some(bounds_at(5.0)),
            maximized: false,
        };
        (mgr, tree)
    }

    fn content_of(node: &PersistedNode, slot: Uuid) -> Option<&PersistedContent> {
        match node {
            PersistedNode::Slot { id, content } => (*id == slot).then_some(content),
            PersistedNode::Split { a, b, .. } => {
                content_of(a, slot).or_else(|| content_of(b, slot))
            }
        }
    }

    fn main_strip(entries: &[WindowEntry]) -> &TabStrip {
        match &entries[0].kind {
            WindowKind::Main(strip) => strip,
            other => panic!("main 이 먼저여야 한다: {other:?}"),
        }
    }

    // ── 왕복 ──

    #[test]
    fn memory_to_disk_to_memory_round_trips() {
        let (mgr, tree_attrs) = rich();
        let written = to_persisted(&mgr, tree_attrs);
        let text = codec::encode(&StateFile {
            version: STATE_VERSION,
            saved_at_ms: 1,
            clean_exit: true,
            resolved_crash_copy: None,
            windows: written.clone(),
        })
        .unwrap();
        let (decoded, decode_warnings) = codec::decode(&text).unwrap();
        assert_eq!(decode_warnings, vec![]);
        assert_eq!(decoded.windows, written);

        let back = ViewManager::from_persisted(decoded.windows, &Labels::default());
        assert_eq!(back.warnings.len(), 2, "{:?}", back.warnings);
        assert!(back
            .warnings
            .iter()
            .all(|w| matches!(w, RestoreWarning::UnknownContent { .. })));
        assert_eq!(back.tree, tree_attrs);
        assert_eq!(to_persisted(&back.layout, back.tree), written);

        let restored = &back.layout;
        assert_eq!(restored.views.len(), mgr.views.len());
        for (id, view) in &mgr.views {
            assert_eq!(restored.views.get(id), Some(view));
            for slot in tree::slot_ids(&view.layout) {
                assert_eq!(
                    restored.unknown_content(*id, slot),
                    mgr.unknown_content(*id, slot)
                );
            }
            assert_eq!(
                restored.snapshot(*id).unwrap().foreign_slots,
                mgr.snapshot(*id).unwrap().foreign_slots
            );
        }
        assert_eq!(restored.windows.len(), mgr.windows.len());
        for original in mgr.windows.values() {
            let (label, again) = restored
                .windows
                .iter()
                .find(|(_, w)| w.window_id == original.window_id)
                .expect("같은 영속 id 의 창");
            assert_eq!(
                (&again.tabs, again.active, again.attrs),
                (&original.tabs, original.active, original.attrs)
            );
            for view in &again.tabs {
                assert_eq!(restored.owner_of(*view), Some(label));
            }
        }
        let mut labels: Vec<&str> = restored.windows.keys().map(String::as_str).collect();
        labels.sort_unstable();
        assert_eq!(labels, ["main", "slot-popup-1", "slot-popup-2"]);
    }

    // ── 메모리 → 영속 ──

    #[test]
    fn windows_are_written_main_then_tree_then_popouts_by_id() {
        let (mgr, tree_attrs) = rich();
        let written = to_persisted(&mgr, tree_attrs);
        let mut popout_ids: Vec<&str> = mgr
            .windows
            .iter()
            .filter(|(label, _)| label.as_str() != MAIN_WINDOW_LABEL)
            .map(|(_, w)| w.window_id.as_str())
            .collect();
        popout_ids.sort_unstable();
        let ids: Vec<&str> = written.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["main", TREE_WINDOW_ID, popout_ids[0], popout_ids[1]]);
        assert!(matches!(written[1].kind, WindowKind::Tree));
        assert!(matches!(written[2].kind, WindowKind::Popout(_)));
        assert_eq!(
            (written[0].theme, written[0].maximized),
            (Some(UiTheme::Light), true)
        );
        assert_eq!(
            written[0].bounds,
            Some(Bounds {
                x: -1920.0,
                y: 10.0,
                w: 800.0,
                h: 600.0
            }),
            "최대화 전 보통 자리"
        );
        let strip = main_strip(&written);
        assert_eq!(strip.active_tab, main_active(&mgr));
        let names: Vec<&str> = strip.tabs.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["작업", "둘"]);
    }

    #[test]
    fn unknown_content_is_written_back_only_while_the_slot_is_untouched() {
        let mut mgr = ViewManager::new();
        place(&mut mgr, MAIN_WINDOW_LABEL, 0.0, false);
        let v = main_active(&mgr);
        let root = first_slot(&mgr, v);
        mgr.set_unknown_content(v, root, raw("x")).unwrap();
        let right = mgr.split_slot(v, root, SplitDir::LeftRight).unwrap();
        let content = |mgr: &ViewManager, slot| {
            let written = to_persisted(mgr, WindowAttrs::default());
            content_of(&main_strip(&written).tabs[0].layout, slot)
                .cloned()
                .unwrap()
        };
        assert_eq!(content(&mgr, root), PersistedContent::Unknown(raw("x")));
        assert_eq!(
            content(&mgr, right),
            PersistedContent::Known(SlotContent::Empty)
        );

        mgr.assign_agent(v, root, "a".into()).unwrap();
        assert_eq!(
            content(&mgr, root),
            PersistedContent::Known(SlotContent::Agent {
                agent_id: "a".into()
            })
        );

        mgr.set_unknown_content(v, root, raw("x")).unwrap();
        mgr.set_slot_content(v, root, SlotContent::Empty).unwrap();
        assert_eq!(
            content(&mgr, root),
            PersistedContent::Known(SlotContent::Empty),
            "빈 내용을 명시적으로 놓으면 원문은 사라진다"
        );
    }

    #[test]
    fn windows_whose_place_was_never_seen_are_written_and_round_trip() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let s = first_slot(&mgr, v);
        mgr.assign_agent(v, s, "a-1".into()).unwrap();
        mgr.create_tab(MAIN_WINDOW_LABEL, Some("둘".into()))
            .unwrap();
        mgr.create_window("slot-popup-1").unwrap();
        mgr.observe_window_placement(
            MAIN_WINDOW_LABEL,
            WindowPlacement {
                minimized: false,
                maximized: true,
                bounds: Some(bounds_at(0.0)),
            },
        )
        .unwrap();
        let tree_without_place = WindowAttrs {
            theme: Some(UiTheme::Dark),
            bounds: None,
            maximized: false,
        };
        let written = to_persisted(&mgr, tree_without_place);
        assert_eq!(written.len(), 3);
        assert!(written.iter().all(|w| w.bounds.is_none()));
        assert!(written[0].maximized, "최대화만 본 main");

        let text = codec::encode(&StateFile {
            version: STATE_VERSION,
            saved_at_ms: 1,
            clean_exit: false,
            resolved_crash_copy: None,
            windows: written.clone(),
        })
        .unwrap();
        let (decoded, decode_warnings) = codec::decode(&text).unwrap();
        assert_eq!(decode_warnings, vec![]);
        let back = ViewManager::from_persisted(decoded.windows, &Labels::default());
        assert_eq!(back.warnings, vec![]);
        assert_eq!(back.tree, tree_without_place);
        assert_eq!(
            main_tab_ids(&back.layout),
            mgr.windows[MAIN_WINDOW_LABEL].tabs,
            "자리를 몰라도 main 의 탭을 잃지 않는다"
        );
        assert_eq!(back.layout.views[&v], mgr.views[&v]);
        assert_eq!(to_persisted(&back.layout, back.tree), written);
    }

    // ── 영속 → 메모리: 고치기 ──

    fn dto_bounds() -> Bounds {
        Bounds {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        }
    }

    fn slot_tab(content: PersistedContent) -> (TabEntry, Uuid) {
        let slot = Uuid::new_v4();
        let tab = TabEntry {
            id: Uuid::new_v4(),
            name: "t".into(),
            focused_slot_id: Some(slot),
            layout: PersistedNode::Slot { id: slot, content },
        };
        (tab, slot)
    }

    fn empty_tab() -> TabEntry {
        slot_tab(PersistedContent::Known(SlotContent::Empty)).0
    }

    fn strip(tabs: Vec<TabEntry>) -> TabStrip {
        TabStrip {
            active_tab: tabs.first().map_or_else(Uuid::new_v4, |t| t.id),
            tabs,
        }
    }

    fn entry(id: &str, kind: WindowKind) -> WindowEntry {
        WindowEntry {
            id: id.into(),
            kind,
            theme: None,
            bounds: Some(dto_bounds()),
            maximized: false,
        }
    }

    fn main_entry(tabs: Vec<TabEntry>) -> WindowEntry {
        entry("main", WindowKind::Main(strip(tabs)))
    }

    fn popout_entry(id: &str, tabs: Vec<TabEntry>) -> WindowEntry {
        entry(id, WindowKind::Popout(strip(tabs)))
    }

    fn restore(windows: Vec<WindowEntry>) -> Restored {
        ViewManager::from_persisted(windows, &Labels::default())
    }

    fn main_tab_ids(layout: &ViewManager) -> Vec<Uuid> {
        layout.windows[MAIN_WINDOW_LABEL].tabs.clone()
    }

    #[test]
    fn a_window_whose_id_does_not_fit_its_kind_is_skipped() {
        let kept = empty_tab();
        let mut tree = entry(TREE_WINDOW_ID, WindowKind::Tree);
        tree.theme = Some(UiTheme::Light);
        let stray = Uuid::new_v4().to_string();
        let back = restore(vec![
            entry("main", WindowKind::Tree),
            popout_entry("main", vec![empty_tab()]),
            popout_entry(TREE_WINDOW_ID, vec![empty_tab()]),
            popout_entry("slot-popup-1", vec![empty_tab()]),
            entry("x", WindowKind::Main(strip(vec![empty_tab()]))),
            entry(&stray, WindowKind::Tree),
            main_entry(vec![kept.clone()]),
            tree,
        ]);
        let mismatched: Vec<(&str, &str)> = back
            .warnings
            .iter()
            .map(|w| match w {
                RestoreWarning::IdKindMismatch { id, kind } => (id.as_str(), *kind),
                other => panic!("id · 종류 어긋남만 있어야 한다: {other}"),
            })
            .collect();
        assert_eq!(
            mismatched,
            [
                ("main", "tree"),
                ("main", "popout"),
                (TREE_WINDOW_ID, "popout"),
                ("slot-popup-1", "popout"),
                ("x", "main"),
                (stray.as_str(), "tree"),
            ]
        );
        assert_eq!(back.layout.windows.len(), 1);
        assert_eq!(main_tab_ids(&back.layout), [kept.id]);
        assert_eq!(back.tree.theme, Some(UiTheme::Light), "제자리 트리 항목");
    }

    #[test]
    fn a_repeated_window_id_keeps_the_first() {
        let popout = Uuid::new_v4();
        let m1 = empty_tab();
        let p1 = empty_tab();
        let mut tree1 = entry(TREE_WINDOW_ID, WindowKind::Tree);
        tree1.theme = Some(UiTheme::EInk);
        let mut tree2 = entry(TREE_WINDOW_ID, WindowKind::Tree);
        tree2.theme = Some(UiTheme::Dark);
        let back = restore(vec![
            main_entry(vec![m1.clone()]),
            popout_entry(&popout.to_string(), vec![p1.clone()]),
            tree1,
            main_entry(vec![empty_tab()]),
            popout_entry(&popout.to_string().to_uppercase(), vec![empty_tab()]),
            tree2,
        ]);
        assert_eq!(
            back.warnings,
            vec![
                RestoreWarning::DuplicateWindow { id: "main".into() },
                RestoreWarning::DuplicateWindow {
                    id: popout.to_string()
                },
                RestoreWarning::DuplicateWindow {
                    id: TREE_WINDOW_ID.into()
                },
            ]
        );
        assert_eq!(main_tab_ids(&back.layout), [m1.id]);
        assert_eq!(back.layout.windows["slot-popup-1"].tabs, [p1.id]);
        assert_eq!(back.layout.windows.len(), 2);
        assert_eq!(back.tree.theme, Some(UiTheme::EInk));
    }

    #[test]
    fn a_repeated_view_or_node_id_drops_the_later_tab() {
        let a = empty_tab();
        let (b, b_slot) = slot_tab(PersistedContent::Known(SlotContent::Empty));
        let mut same_view = empty_tab();
        same_view.id = a.id;
        let (mut same_slot, _) = slot_tab(PersistedContent::Known(SlotContent::AgentList));
        same_slot.layout = PersistedNode::Slot {
            id: b_slot,
            content: PersistedContent::Known(SlotContent::AgentList),
        };
        let twin = Uuid::new_v4();
        let twin_slot = || PersistedNode::Slot {
            id: twin,
            content: PersistedContent::Known(SlotContent::Empty),
        };
        let mut self_twin = empty_tab();
        self_twin.layout = PersistedNode::Split {
            id: Uuid::new_v4(),
            dir: SplitDir::LeftRight,
            ratio: 0.5,
            a: Box::new(twin_slot()),
            b: Box::new(twin_slot()),
        };
        let kept = empty_tab();
        let popout = Uuid::new_v4().to_string();
        let back = restore(vec![
            main_entry(vec![a.clone(), b.clone(), self_twin.clone()]),
            popout_entry(&popout, vec![same_view, same_slot.clone(), kept.clone()]),
        ]);
        assert_eq!(
            back.warnings,
            vec![
                RestoreWarning::DuplicateNode {
                    window_id: "main".into(),
                    view: self_twin.id,
                    node: twin,
                },
                RestoreWarning::DuplicateTab {
                    window_id: popout.clone(),
                    view: a.id,
                },
                RestoreWarning::DuplicateNode {
                    window_id: popout.clone(),
                    view: same_slot.id,
                    node: b_slot,
                },
                RestoreWarning::ActiveRepaired {
                    window_id: popout.clone(),
                    missing: a.id,
                },
            ]
        );
        assert_eq!(main_tab_ids(&back.layout), [a.id, b.id]);
        let p = &back.layout.windows["slot-popup-1"];
        assert_eq!((p.tabs.clone(), p.active), (vec![kept.id], kept.id));
        assert_eq!(back.layout.views.len(), 3);
    }

    #[test]
    fn an_active_tab_that_is_gone_falls_back_to_the_first() {
        let (a, b) = (empty_tab(), empty_tab());
        let missing = Uuid::new_v4();
        let main = entry(
            "main",
            WindowKind::Main(TabStrip {
                active_tab: missing,
                tabs: vec![a.clone(), b],
            }),
        );
        let back = restore(vec![main]);
        assert_eq!(
            back.warnings,
            vec![RestoreWarning::ActiveRepaired {
                window_id: "main".into(),
                missing,
            }]
        );
        assert_eq!(main_active(&back.layout), a.id);
    }

    #[test]
    fn a_main_without_tabs_gets_one_blank_tab() {
        // 항목이 없다.
        let back = restore(vec![entry(TREE_WINDOW_ID, WindowKind::Tree)]);
        assert_eq!(back.warnings, vec![RestoreWarning::MainWithoutTabs]);
        assert_eq!(main_tab_ids(&back.layout).len(), 1);
        assert_eq!(back.layout.windows[MAIN_WINDOW_LABEL].window_id, "main");

        // 항목은 있으나 탭이 하나도 안 남았다 — 속성은 살린다.
        let mut main = main_entry(vec![]);
        main.theme = Some(UiTheme::Light);
        main.maximized = true;
        let back = restore(vec![main]);
        assert_eq!(back.warnings, vec![RestoreWarning::MainWithoutTabs]);
        assert_eq!(
            back.layout.window_attrs(MAIN_WINDOW_LABEL),
            Ok(WindowAttrs {
                theme: Some(UiTheme::Light),
                bounds: WindowBounds::new(0.0, 0.0, 800.0, 600.0),
                maximized: true,
            })
        );
        let only = main_active(&back.layout);
        assert_eq!(back.layout.views[&only].name, "View 1");
    }

    #[test]
    fn a_popout_without_tabs_is_dropped_without_taking_a_label() {
        let empty = Uuid::new_v4().to_string();
        let kept = Uuid::new_v4().to_string();
        let back = restore(vec![
            main_entry(vec![empty_tab()]),
            popout_entry(&empty, vec![]),
            popout_entry(&kept, vec![empty_tab()]),
        ]);
        assert_eq!(
            back.warnings,
            vec![RestoreWarning::PopoutWithoutTabs { id: empty }]
        );
        assert_eq!(back.layout.windows["slot-popup-1"].window_id, kept);
        assert_eq!(back.layout.windows.len(), 2);
    }

    #[test]
    fn popouts_get_fresh_labels_and_keep_their_canonical_ids() {
        let id = Uuid::new_v4();
        let braced = format!("{{{}}}", id.to_string().to_uppercase());
        let back = restore(vec![
            popout_entry(&braced, vec![empty_tab()]),
            main_entry(vec![empty_tab()]),
        ]);
        assert_eq!(back.warnings, vec![]);
        assert_eq!(
            back.layout.windows["slot-popup-1"].window_id,
            id.to_string()
        );
    }

    #[test]
    fn a_label_already_in_use_drops_the_popout() {
        let (first, second) = (Uuid::new_v4().to_string(), Uuid::new_v4().to_string());
        let windows = || {
            vec![
                main_entry(vec![empty_tab()]),
                popout_entry(&first, vec![empty_tab()]),
                popout_entry(&second, vec![empty_tab()]),
            ]
        };
        let back = ViewManager::from_persisted(windows(), &Fixed("slot-popup-1"));
        assert_eq!(
            back.warnings,
            vec![RestoreWarning::LabelTaken {
                id: second.clone(),
                label: "slot-popup-1".into(),
            }]
        );
        assert_eq!(back.layout.windows["slot-popup-1"].window_id, first);

        for reserved in [MAIN_WINDOW_LABEL, TREE_WINDOW_ID] {
            let back = ViewManager::from_persisted(windows(), &Fixed(reserved));
            assert_eq!(back.warnings.len(), 2, "{reserved}");
            assert_eq!(back.layout.windows.len(), 1, "{reserved}");
        }
    }

    #[test]
    fn a_dropped_popout_holds_no_ids_against_the_windows_after_it() {
        // main 보다 먼저 적힌 팝아웃이 main 과 탭 id · 슬롯 id 를 나눠 쓰다가 label 때문에 버려진다.
        let (shared_view, view_slot) = slot_tab(PersistedContent::Known(SlotContent::AgentList));
        let (shared_node, node_slot) =
            slot_tab(PersistedContent::Known(SlotContent::PresetPalette));
        let mut same_view = empty_tab();
        same_view.id = shared_view.id;
        let mut same_node = empty_tab();
        same_node.layout = PersistedNode::Slot {
            id: node_slot,
            content: PersistedContent::Known(SlotContent::Empty),
        };
        let popout = Uuid::new_v4().to_string();
        let back = ViewManager::from_persisted(
            vec![
                popout_entry(&popout, vec![same_view, same_node]),
                main_entry(vec![shared_view.clone(), shared_node.clone()]),
            ],
            &Fixed(MAIN_WINDOW_LABEL),
        );
        assert_eq!(
            back.warnings,
            vec![RestoreWarning::LabelTaken {
                id: popout,
                label: MAIN_WINDOW_LABEL.into(),
            }]
        );
        let layout = &back.layout;
        assert_eq!(main_tab_ids(layout), [shared_view.id, shared_node.id]);
        assert_eq!(
            layout.slot_content(shared_view.id, view_slot),
            Ok(SlotContent::AgentList)
        );
        assert_eq!(
            layout.slot_content(shared_node.id, node_slot),
            Ok(SlotContent::PresetPalette)
        );
        assert_eq!(layout.windows.len(), 1);

        // 탭이 다 겹쳐 버려진 팝아웃은 자기 창 id 도 쥐지 않는다 — 같은 id 의 뒤 항목이 산다.
        let main_tab = empty_tab();
        let mut dup = empty_tab();
        dup.id = main_tab.id;
        let id = Uuid::new_v4().to_string();
        let kept = empty_tab();
        let back = restore(vec![
            main_entry(vec![main_tab.clone()]),
            popout_entry(&id, vec![dup]),
            popout_entry(&id, vec![kept.clone()]),
        ]);
        assert_eq!(
            back.warnings,
            vec![
                RestoreWarning::DuplicateTab {
                    window_id: id.clone(),
                    view: main_tab.id,
                },
                RestoreWarning::PopoutWithoutTabs { id: id.clone() },
            ]
        );
        let p = &back.layout.windows["slot-popup-1"];
        assert_eq!(
            (p.window_id.as_str(), p.tabs.clone()),
            (id.as_str(), vec![kept.id])
        );
    }

    #[test]
    fn a_dropped_popout_reports_only_why_it_was_dropped() {
        let main_tab = empty_tab();
        let mut dup = empty_tab();
        dup.id = main_tab.id;
        let dup_id = dup.id;
        let no_tabs = Uuid::new_v4().to_string();
        let mut no_tabs_entry = popout_entry(&no_tabs, vec![dup]);
        no_tabs_entry.bounds = Some(Bounds {
            x: f64::NAN,
            ..dto_bounds()
        });

        // 남았다면 자리 · 비율 · 모르는 내용 · 활성 탭을 알렸을 팝아웃.
        let mut bad_ratio = empty_tab();
        bad_ratio.layout = PersistedNode::Split {
            id: Uuid::new_v4(),
            dir: SplitDir::LeftRight,
            ratio: 5.0,
            a: Box::new(empty_tab().layout),
            b: Box::new(empty_tab().layout),
        };
        let (unknown, _) = slot_tab(PersistedContent::Unknown(raw("file_tree")));
        let taken = Uuid::new_v4().to_string();
        let mut taken_entry = entry(
            &taken,
            WindowKind::Popout(TabStrip {
                active_tab: Uuid::new_v4(),
                tabs: vec![bad_ratio, unknown],
            }),
        );
        taken_entry.bounds = Some(Bounds {
            h: 0.0,
            ..dto_bounds()
        });

        let back = ViewManager::from_persisted(
            vec![main_entry(vec![main_tab]), no_tabs_entry, taken_entry],
            &Fixed(MAIN_WINDOW_LABEL),
        );
        assert_eq!(
            back.warnings,
            vec![
                RestoreWarning::DuplicateTab {
                    window_id: no_tabs.clone(),
                    view: dup_id,
                },
                RestoreWarning::PopoutWithoutTabs { id: no_tabs },
                RestoreWarning::LabelTaken {
                    id: taken,
                    label: MAIN_WINDOW_LABEL.into(),
                },
            ]
        );
        assert_eq!(back.layout.windows.len(), 1);
    }

    #[test]
    fn unknown_content_restores_as_an_occupied_empty_slot() {
        let (tab, slot) = slot_tab(PersistedContent::Unknown(raw("file_tree")));
        let back = restore(vec![main_entry(vec![tab.clone()])]);
        assert_eq!(
            back.warnings,
            vec![RestoreWarning::UnknownContent {
                window_id: "main".into(),
                view: tab.id,
                slot,
                kind: Some("file_tree".into()),
            }]
        );
        let layout = &back.layout;
        assert_eq!(layout.slot_content(tab.id, slot), Ok(SlotContent::Empty));
        assert_eq!(
            layout.unknown_content(tab.id, slot),
            Some(&raw("file_tree"))
        );
        assert_eq!(layout.slot_is_free(tab.id, slot), Ok(false));
        assert_eq!(layout.snapshot(tab.id).unwrap().foreign_slots, vec![slot]);
    }

    #[test]
    fn unusable_bounds_are_dropped_and_the_rest_kept() {
        let mut main = main_entry(vec![empty_tab()]);
        main.bounds = Some(Bounds {
            x: f64::NAN,
            ..dto_bounds()
        });
        main.theme = Some(UiTheme::Dark);
        main.maximized = true;
        let mut tree = entry(TREE_WINDOW_ID, WindowKind::Tree);
        tree.bounds = Some(Bounds {
            h: 0.0,
            ..dto_bounds()
        });
        let mut nowhere = popout_entry(&Uuid::new_v4().to_string(), vec![empty_tab()]);
        nowhere.bounds = None;
        let back = restore(vec![main, tree, nowhere]);
        assert_eq!(
            back.warnings,
            vec![
                RestoreWarning::BoundsDropped {
                    window_id: "main".into()
                },
                RestoreWarning::BoundsDropped {
                    window_id: TREE_WINDOW_ID.into()
                },
            ],
            "자리가 없는 것은 경고가 아니다"
        );
        assert_eq!(
            back.layout.window_attrs("slot-popup-1").unwrap().bounds,
            None
        );
        assert_eq!(
            back.layout.window_attrs(MAIN_WINDOW_LABEL),
            Ok(WindowAttrs {
                theme: Some(UiTheme::Dark),
                bounds: None,
                maximized: true,
            })
        );
        assert_eq!(back.tree.bounds, None);
    }

    #[test]
    fn a_ratio_out_of_range_is_repaired() {
        let split = |ratio: f64| {
            let mut tab = empty_tab();
            let id = Uuid::new_v4();
            tab.layout = PersistedNode::Split {
                id,
                dir: SplitDir::TopBottom,
                ratio,
                a: Box::new(empty_tab().layout),
                b: Box::new(empty_tab().layout),
            };
            (tab, id)
        };
        let ratio_after = |back: &Restored, tab: &TabEntry, id| {
            tree::split_ratio(&back.layout.views[&tab.id].layout, id)
        };
        for (given, fixed) in [
            (5.0, tree::RATIO_MAX),
            (-1.0, tree::RATIO_MIN),
            (f64::NAN, tree::SPLIT_RATIO),
        ] {
            let (tab, id) = split(given);
            let back = restore(vec![main_entry(vec![tab.clone()])]);
            assert!(
                matches!(
                    back.warnings.as_slice(),
                    [RestoreWarning::RatioRepaired { split, .. }] if *split == id
                ),
                "{given}: {:?}",
                back.warnings
            );
            assert_eq!(ratio_after(&back, &tab, id), Some(fixed), "{given}");
        }
        let (tab, id) = split(0.3);
        let back = restore(vec![main_entry(vec![tab.clone()])]);
        assert_eq!(back.warnings, vec![]);
        assert_eq!(ratio_after(&back, &tab, id), Some(0.3));
    }

    // ── 변경 번호 ──

    #[test]
    fn the_revision_moves_with_each_of_its_three_parts() {
        let mut mgr = ViewManager::new();
        let start = StateRevision::of(&mgr, 0);
        mgr.set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::Dark))
            .unwrap();
        let after_attrs = StateRevision::of(&mgr, 0);
        assert_ne!(after_attrs, start);
        assert_eq!(after_attrs.layout, start.layout);
        mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let after_layout = StateRevision::of(&mgr, 0);
        assert_ne!(after_layout, after_attrs);
        assert_eq!(after_layout.attrs, after_attrs.attrs);
        assert_ne!(StateRevision::of(&mgr, 1), after_layout);
    }
}
