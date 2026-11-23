//! The memory shape of the screen state (`ViewManager`) ↔ the persisted shape (`schema::WindowEntry`) — TRD S21-storage §6-2 · §6-3. Entry points: [`to_persisted`] (writer · boot run marker) · [`ViewManager::from_persisted`] (boot · runtime accept) · [`StateRevision`] (the writer's change number).
//! Both directions spell out fields by name — no `..` or wildcard branches, so growing fields · variants of `View` · `LayoutNode` · `SlotContent` · `WindowTabs` · `WindowAttrs` or of the persisted side breaks compilation here; passing with `..` would let a new field silently drop out of persistence (§9-2 「what P3a passed over」).

use std::collections::HashSet;
use std::fmt;

use serde_json::{Map, Value};
use uuid::Uuid;

use super::schema::{
    Bounds, PersistedContent, PersistedNode, TabEntry, TabStrip, WindowEntry, WindowKind,
};
use crate::layout::manager::{RestoredWindow, ViewId, WindowLabel, WindowTabs};
use crate::layout::tab_name::{self, effective_tab_name};
use crate::layout::{
    tree, LabelSource, LayoutNode, SlotContent, TabName, View, ViewManager, WindowAttrs,
    WindowBounds, MAIN_WINDOW_LABEL,
};
use crate::theme::UiTheme;

/// The writer's change numbers — the layout `version` · window attribute `attrs_rev`; the writer only checks equality (§6-4). Read inside the layout lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateRevision {
    pub layout: u64,
    pub attrs: u64,
}

impl StateRevision {
    pub fn of(layout: &ViewManager) -> Self {
        StateRevision {
            layout: layout.version,
            attrs: layout.attrs_rev(),
        }
    }
}

// ── memory → persisted ───────────────────────────────────────────────────────────

/// All windows, in main · popouts (persisted id order) order; the caller calls it inside the `ViewManager` lock.
/// - Unknown content slots (a side table entry exists and the memory content is `Empty`) are written as raw text, so a slot written by a newer version is not erased by this version's save (ADR-0060).
/// - Measured screen values (`canvas` · `metrics`) are not included (§6-1).
/// - A window whose position was never seen (maximized from creation · before the first `Moved`) is written with `bounds: null` — a window is not dropped over its position: for main its tabs would vanish at the next boot.
pub fn to_persisted(layout: &ViewManager) -> Vec<WindowEntry> {
    let mut entries = Vec::new();
    if let Some(main) = layout.windows.get(MAIN_WINDOW_LABEL) {
        entries.push(tabbed_entry(layout, main, WindowKind::Main));
    }
    let mut popouts: Vec<&WindowTabs> = layout
        .windows
        .iter()
        .filter(|(label, _)| label.as_str() != MAIN_WINDOW_LABEL)
        .map(|(_, window)| window)
        .collect();
    // Keep the table iteration order (HashMap) from leaking into the file — the same screen yields the same text.
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
        number,
        attrs,
        placement_memo: _,
    } = window;
    let tabs = tabs
        .iter()
        .filter_map(|id| layout.views.get(id))
        .map(|view| tab_entry(layout, view))
        .collect();
    window_entry(
        window_id.clone(),
        *number,
        kind(TabStrip {
            active_tab: *active,
            tabs,
        }),
        *attrs,
    )
}

fn window_entry(id: String, number: u32, kind: WindowKind, attrs: WindowAttrs) -> WindowEntry {
    let WindowAttrs {
        theme,
        bounds,
        maximized,
    } = attrs;
    WindowEntry {
        id,
        number: Some(number),
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

// Name = the visible name at that moment + whether it was set (TRD S26-window-chrome §3-3). The text of an auto tab is not used as a name by the next boot.
fn tab_entry(layout: &ViewManager, view: &View) -> TabEntry {
    let View {
        id,
        name,
        layout: node,
        focused_slot_id,
    } = view;
    TabEntry {
        id: *id,
        name: effective_tab_name(layout, view),
        named: Some(name.is_named()),
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

// The raw text is written back only while the memory content is still `Empty` — writing anything to that slot removes the side table entry, but even if they diverge the content the user placed wins.
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
        SlotContent::Peers => SlotContent::Peers,
    })
}

// ── persisted → memory ───────────────────────────────────────────────────────────

// Only set names are revived — if the field is absent (old file) or false it is auto (D16). A set name that is empty text folds to auto (AC36).
fn restored_name(name: String, named: Option<bool>) -> TabName {
    match named {
        Some(true) => TabName::named(name).unwrap_or(TabName::Auto),
        Some(false) | None => TabName::Auto,
    }
}

pub struct Restored {
    pub layout: ViewManager,
    pub warnings: Vec<RestoreWarning>,
}

/// One thing fixed or skipped while moving a persisted window set into the memory model. When the same id repeats the later one is always discarded; a skipped window holds no id (window · tab · node) and leaves only the reason (position · ratio · unknown content are not reported).
#[derive(Debug, Clone, PartialEq)]
pub enum RestoreWarning {
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
    DuplicateNode {
        window_id: String,
        view: Uuid,
        node: Uuid,
    },
    RatioRepaired {
        window_id: String,
        view: Uuid,
        split: Uuid,
        ratio: f64,
    },
    ActiveRepaired {
        window_id: String,
        missing: Uuid,
    },
    /// The main entry is absent or no tab remained — starts with one empty tab (invariant 4).
    MainWithoutTabs,
    PopoutWithoutTabs {
        id: String,
    },
    LabelTaken {
        id: String,
        label: String,
    },
    BoundsDropped {
        window_id: String,
    },
    /// Slot content this version does not know — stands as an empty field holding the raw text in the side table. `kind` = the raw `type` string.
    UnknownContent {
        window_id: String,
        view: Uuid,
        slot: Uuid,
        kind: Option<String>,
    },
    /// A popout did not get a saved number — absent or unusable value (`saved` = `None`) · main's 1 · overlap with an earlier window. Gives the smallest free number `given` (TRD S26-window-chrome §3-4 · D3).
    NumberReassigned {
        window_id: String,
        saved: Option<u32>,
        given: u32,
    },
    Internal(String),
}

impl fmt::Display for RestoreWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RestoreWarning::IdKindMismatch { id, kind } => {
                write!(f, "window {id} is not an id of a {kind} window — skipping")
            }
            RestoreWarning::DuplicateWindow { id } => {
                write!(f, "window {id} is duplicated — skipping the later one")
            }
            RestoreWarning::DuplicateTab { window_id, view } => {
                write!(f, "tab {view} of window {window_id} is duplicated — skipping")
            }
            RestoreWarning::DuplicateNode {
                window_id,
                view,
                node,
            } => write!(
                f,
                "node {node} in tab {view} of window {window_id} is duplicated — skipping that tab"
            ),
            RestoreWarning::RatioRepaired {
                window_id,
                view,
                split,
                ratio,
            } => write!(
                f,
                "fixing ratio {ratio} of split {split} in tab {view} of window {window_id} into the range"
            ),
            RestoreWarning::ActiveRepaired { window_id, missing } => write!(
                f,
                "active tab {missing} of window {window_id} is missing — making the first tab active"
            ),
            RestoreWarning::MainWithoutTabs => {
                write!(f, "the main window has no tabs — starting with one empty tab")
            }
            RestoreWarning::PopoutWithoutTabs { id } => {
                write!(f, "popout {id} has no tabs — skipping")
            }
            RestoreWarning::LabelTaken { id, label } => {
                write!(
                    f,
                    "label {label} to give popout {id} is already used — skipping"
                )
            }
            RestoreWarning::BoundsDropped { window_id } => {
                write!(f, "the position of window {window_id} is unusable — opening at the default position")
            }
            RestoreWarning::UnknownContent {
                window_id,
                view,
                slot,
                kind,
            } => write!(
                f,
                "window {window_id} tab {view} slot {slot} — holding content this version does not know ({}) as is",
                kind.as_deref().unwrap_or("no kind")
            ),
            RestoreWarning::NumberReassigned {
                window_id,
                saved,
                given,
            } => match saved {
                Some(saved) => write!(
                    f,
                    "number {saved} of popout {window_id} is unusable — giving {given}"
                ),
                None => write!(f, "popout {window_id} has no number — giving {given}"),
            },
            RestoreWarning::Internal(reason) => {
                write!(
                    f,
                    "could not build the restore model — starting with the default screen: {reason}"
                )
            }
        }
    }
}

impl ViewManager {
    /// Builds a new model from a persisted window set (the `windows` of the `codec::decode` result) — pure (0 files · windows · locks, no failure). The codec does no shape check, so invariants 1–4 and the window set's shape are re-established here (fix rules = [`RestoreWarning`]). Each popout gets a new label from `labels` — the persisted id stays, the label belongs to this boot only (§6-3).
    /// The result's `version` · `attrs_rev` mean nothing: boot starts the writer with the numbers at the run marker write, so they are used as is. A runtime accept (P3c1 · §6-7 ③) does not swap this result in for the live model:
    /// - It sets `version = current + 1` · `attrs_rev = current + 1` directly — this result's numbers (near 0) would move the numbers back, and since the writer only checks equality it would miss changes in between once it equals the last seen value again.
    /// - It moves the side table (unknown content) with the View — moving only the View would save that slot as an empty field and lose the raw text. The old View is removed with `remove_view` (reaping its side table entries).
    /// - The main entry changes only tabs · attributes — replacing it wholesale would lose the measurements (`canvas` · `metrics`) until the webview reports again.
    /// - The result's agent name ledger is only the seed captured from the saved file (`agent_name_seed`); the accept adds only to the empty entries of the current ledger (`adopt_restored`).
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
    popouts: Vec<(WindowLabel, String, Pending)>,
    popout_ids: HashSet<Uuid>,
    views: HashSet<Uuid>,
    nodes: HashSet<Uuid>,
    // Agent name seeds captured from the kept tabs (agent id, name — `agent_name_seed`) · in saved order.
    names: Vec<(String, String)>,
}

struct Pending {
    window: RestoredWindow,
    unknown: Vec<(ViewId, Uuid, Map<String, Value>)>,
}

/// One window's tab set — not yet loaded into the `Restorer` (it holds no id and has raised no warning); [`Restorer::commit`] loads it after deciding to keep the window.
#[derive(Default)]
struct Staged {
    tabs: Vec<View>,
    active: Option<ViewId>,
    nodes: HashSet<Uuid>,
    unknown: Vec<(ViewId, Uuid, Map<String, Value>)>,
    names: Vec<(String, String)>,
    warnings: Vec<RestoreWarning>,
}

impl Restorer {
    fn window(&mut self, entry: WindowEntry, labels: &dyn LabelSource) {
        let WindowEntry {
            id,
            number,
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
                self.main = Some(self.commit(attrs, number, staged));
            }
            WindowKind::Popout(strip) => {
                let Ok(uuid) = Uuid::parse_str(&id) else {
                    return self.mismatch(id, "popout");
                };
                // Other spellings of the same UUID (uppercase · braces) are the same window — written and compared in the canonical spelling.
                let id = uuid.to_string();
                if self.popout_ids.contains(&uuid) {
                    return self.warn(RestoreWarning::DuplicateWindow { id });
                }
                // Loaded only after all discards (ids · warnings · positions) are decided — a later window's (main included) tabs overlapping ids held by a discarded window would be pushed out, losing content that could have been saved.
                let staged = self.stage(&id, strip);
                if staged.tabs.is_empty() {
                    // If all tabs dropped out by overlapping, that warning is the reason for discarding.
                    self.warnings.extend(staged.warnings);
                    return self.warn(RestoreWarning::PopoutWithoutTabs { id });
                }
                let label = labels.next_label();
                let taken = label == MAIN_WINDOW_LABEL
                    || self.popouts.iter().any(|(used, _, _)| *used == label);
                if taken {
                    return self.warn(RestoreWarning::LabelTaken { id, label });
                }
                self.popout_ids.insert(uuid);
                let attrs = self.attrs(&id, theme, bounds, maximized);
                let pending = self.commit(attrs, number, staged);
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
            named,
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
        staged
            .names
            .extend(agent_name_seed(&name, named, &layout, focused_slot_id));
        staged.tabs.push(View {
            id,
            name: restored_name(name, named),
            layout,
            focused_slot_id,
        });
    }

    fn commit(&mut self, attrs: WindowAttrs, number: Option<u32>, staged: Staged) -> Pending {
        let Staged {
            tabs,
            active,
            nodes,
            unknown,
            names,
            warnings,
        } = staged;
        self.views.extend(tabs.iter().map(|v| v.id));
        self.nodes.extend(nodes);
        self.names.extend(names);
        self.warnings.extend(warnings);
        Pending {
            window: RestoredWindow {
                attrs,
                tabs,
                active,
                number,
            },
            unknown,
        }
    }

    fn finish(self) -> Restored {
        let Restorer {
            mut warnings,
            main,
            popouts,
            popout_ids: _,
            views: _,
            nodes: _,
            names,
        } = self;
        let main = main.unwrap_or(Pending {
            window: RestoredWindow {
                attrs: WindowAttrs::default(),
                tabs: Vec::new(),
                active: None,
                number: None,
            },
            unknown: Vec::new(),
        });
        if main.window.tabs.is_empty() {
            warnings.push(RestoreWarning::MainWithoutTabs);
        }
        let mut unknown = main.unknown;
        let saved: Vec<(WindowLabel, String, Option<u32>)> = popouts
            .iter()
            .map(|(label, id, pending)| (label.clone(), id.clone(), pending.window.number))
            .collect();
        let popouts = popouts
            .into_iter()
            .map(|(label, id, pending)| {
                unknown.extend(pending.unknown);
                (label, id, pending.window)
            })
            .collect();
        let mut layout = match ViewManager::from_restored(main.window, popouts) {
            Ok(layout) => layout,
            Err(reason) => {
                warnings.push(RestoreWarning::Internal(reason));
                return Restored {
                    layout: ViewManager::new(),
                    warnings,
                };
            }
        };
        for (label, window_id, saved) in saved {
            let Some(given) = layout.windows.get(&label).map(|w| w.number) else {
                continue;
            };
            if saved != Some(given) {
                warnings.push(RestoreWarning::NumberReassigned {
                    window_id,
                    saved,
                    given,
                });
            }
        }
        for (view, slot, raw) in unknown {
            if let Err(e) = layout.set_unknown_content(view, slot, raw) {
                warnings.push(RestoreWarning::Internal(e.to_string()));
            }
        }
        layout.seed_agent_names(names);
        Restored { layout, warnings }
    }
}

/// Captures the agent name ledger seed from the name an auto-named tab saved (TRD S26-window-chrome §4-3 · main decision M16): that name is the [`slot_label`](crate::layout::tab_name::slot_label) of the focused slot at save time, so if that slot is an agent it is the agent's display name — keeping the tab name · OS title from blinking once as 「Agent」 before the daemon list arrives.
/// Not a seed: `named` is not `Some(false)` (a set name, or an old file without the field whose text may be an old default like 「View 3」) · empty text · text of the word table (「Agent」 of an agent not in the ledger — not a name) · the focused slot is not an agent (an unknown content slot of this version has memory content `Empty`, so it falls here). With no focus or focus outside the tree, the tree's first slot is used — the rule by which the restore model fixes focus.
fn agent_name_seed(
    name: &str,
    named: Option<bool>,
    layout: &LayoutNode,
    focused: Option<Uuid>,
) -> Option<(String, String)> {
    if named != Some(false) || name.trim().is_empty() || tab_name::is_word(name) {
        return None;
    }
    let slot = focused
        .filter(|&id| tree::contains_slot(layout, id))
        .unwrap_or_else(|| tree::first_slot_id(layout));
    let agent_id = tree::find_slot(layout, slot)?.agent_id()?;
    Some((agent_id.to_string(), name.to_string()))
}

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
            // An out-of-range ratio makes an area width 0 or negative — clamped to the same range as the write path (`set_split_ratio`).
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

// Moved as is — if fields · variants grow it breaks here, making the restore side decide how to handle that value (discarding unusable values etc. — §6-0 ⑤).
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
        SlotContent::Peers => SlotContent::Peers,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

    use serde_json::{json, Map, Value};
    use uuid::Uuid;

    use super::{agent_name_seed, to_persisted, RestoreWarning, Restored, StateRevision};
    use crate::layout::{
        tree, LabelSource, LayoutNode, SlotContent, SplitDir, TabName, ViewManager, WindowAttrs,
        WindowBounds, WindowPlacement, MAIN_WINDOW_LABEL,
    };
    use crate::state::codec;
    use crate::state::schema::{
        Bounds, PersistedContent, PersistedNode, StateFile, TabEntry, TabStrip, WindowEntry,
        WindowKind, STATE_VERSION,
    };
    use crate::theme::UiTheme;

    #[derive(Default)]
    struct Labels(AtomicU64);

    impl LabelSource for Labels {
        fn next_label(&self) -> String {
            format!("slot-popup-{}", self.0.fetch_add(1, Ordering::SeqCst) + 1)
        }
    }

    struct Fixed(&'static str);

    impl LabelSource for Fixed {
        fn next_label(&self) -> String {
            self.0.to_string()
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

    fn rich() -> ViewManager {
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
        mgr.rename_tab(first, "Work".into()).unwrap();
        let second = mgr
            .create_tab(MAIN_WINDOW_LABEL, Some("Two".into()))
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
        mgr
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
            other => panic!("main must come first: {other:?}"),
        }
    }

    // ── round trip ──

    #[test]
    fn memory_to_disk_to_memory_round_trips() {
        let mgr = rich();
        let written = to_persisted(&mgr);
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
        assert_eq!(to_persisted(&back.layout), written);

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
                .expect("the window with the same persisted id");
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

    // ── memory → persisted ──

    #[test]
    fn windows_are_written_main_then_popouts_by_id() {
        let mgr = rich();
        let written = to_persisted(&mgr);
        let mut popout_ids: Vec<&str> = mgr
            .windows
            .iter()
            .filter(|(label, _)| label.as_str() != MAIN_WINDOW_LABEL)
            .map(|(_, w)| w.window_id.as_str())
            .collect();
        popout_ids.sort_unstable();
        let ids: Vec<&str> = written.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["main", popout_ids[0], popout_ids[1]]);
        assert!(matches!(written[1].kind, WindowKind::Popout(_)));
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
            "normal position before maximizing"
        );
        let strip = main_strip(&written);
        assert_eq!(strip.active_tab, main_active(&mgr));
        let names: Vec<&str> = strip.tabs.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Work", "Two"]);
    }

    // TRD S26-window-chrome §3-3 — the name field is the visible name at that moment + whether it was set (auto tabs are written as text too).
    #[test]
    fn a_tab_is_written_with_its_visible_name_and_whether_it_was_named() {
        let mut mgr = ViewManager::new();
        let auto = main_active(&mgr);
        let slot = first_slot(&mgr, auto);
        mgr.set_slot_content(auto, slot, SlotContent::AgentList)
            .unwrap();
        let named = mgr
            .create_tab(MAIN_WINDOW_LABEL, Some("Work".into()))
            .unwrap();
        let foreign = mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let slot = first_slot(&mgr, foreign);
        mgr.set_unknown_content(foreign, slot, raw("from_the_future"))
            .unwrap();

        let written = to_persisted(&mgr);
        let tabs: Vec<(Uuid, &str, Option<bool>)> = main_strip(&written)
            .tabs
            .iter()
            .map(|t| (t.id, t.name.as_str(), t.named))
            .collect();
        assert_eq!(
            tabs,
            [
                (auto, "Agent tree", Some(false)),
                (named, "Work", Some(true)),
                (foreign, "Unknown content", Some(false)),
            ]
        );
    }

    // D16 · AC36 — only tabs whose `named` is true revive the name. The text of an old file (no field) may be an old default like 「View 3」.
    #[test]
    fn only_a_tab_marked_named_restores_its_name() {
        let tab = |name: &str, named: Option<bool>| {
            let mut tab = empty_tab();
            tab.name = name.into();
            tab.named = named;
            tab
        };
        let back = restore(vec![main_entry(vec![
            tab("Work", Some(true)),
            tab("myproj", Some(false)),
            tab("View 3", None),
            tab("  ", Some(true)),
        ])]);
        assert_eq!(back.warnings, vec![]);
        let names: Vec<TabName> = main_tab_ids(&back.layout)
            .iter()
            .map(|id| back.layout.views[id].name.clone())
            .collect();
        assert_eq!(
            names,
            [
                TabName::Named("Work".into()),
                TabName::Auto,
                TabName::Auto,
                TabName::Auto,
            ]
        );
    }

    // ── Agent name seeds (TRD S26-window-chrome §4-3) ──

    fn agent_tab(agent: &str, name: &str, named: Option<bool>) -> TabEntry {
        let (mut tab, _) = slot_tab(PersistedContent::Known(SlotContent::Agent {
            agent_id: agent.into(),
        }));
        tab.name = name.into();
        tab.named = named;
        tab
    }

    fn listed_names(layout: &ViewManager) -> Vec<String> {
        layout
            .list_tabs(MAIN_WINDOW_LABEL)
            .unwrap()
            .tabs
            .into_iter()
            .map(|t| t.name)
            .collect()
    }

    // Only the saved name of an auto-named tab (`named: false`) is a seed — a set name · the text of an old file without the field · word-table text are not planted, or auto tabs watching the same agent would show that text.
    #[test]
    fn only_an_auto_tabs_saved_agent_name_seeds_the_name_book() {
        let back = restore(vec![main_entry(vec![
            agent_tab("a1", "myproj", Some(false)),
            agent_tab("a2", "chosen", Some(true)),
            agent_tab("a2", "에이전트", Some(false)), // hangul-ok: the Korean word-table entry
            agent_tab("a3", "View 3", None),
            agent_tab("a4", "Agent", Some(false)),
        ])]);
        assert_eq!(back.warnings, vec![]);
        assert_eq!(
            listed_names(&back.layout),
            ["myproj", "chosen", "Agent", "Agent", "Agent"],
            "neither language's agent word is seeded as a name"
        );
    }

    #[test]
    fn a_seed_comes_from_the_focused_slot_or_else_the_first_slot() {
        let slot = Uuid::new_v4();
        let other = Uuid::new_v4();
        let agent = LayoutNode::Slot {
            id: slot,
            content: SlotContent::Agent {
                agent_id: "a1".into(),
            },
        };
        let seed = Some(("a1".to_string(), "myproj".to_string()));
        for focus in [Some(slot), None, Some(Uuid::new_v4())] {
            assert_eq!(
                agent_name_seed("myproj", Some(false), &agent, focus),
                seed,
                "{focus:?}"
            );
        }
        let split = LayoutNode::Split {
            id: Uuid::new_v4(),
            dir: SplitDir::LeftRight,
            ratio: 0.5,
            a: Box::new(agent),
            b: Box::new(LayoutNode::Slot {
                id: other,
                content: SlotContent::AgentList,
            }),
        };
        assert_eq!(
            agent_name_seed("myproj", Some(false), &split, Some(other)),
            None,
            "the focused field is not an agent"
        );
        assert_eq!(agent_name_seed("myproj", Some(false), &split, None), seed);
    }

    // Text of the word table is not a name — the tab of an agent not in the ledger is saved as 「Agent」 (main decision · U3a review).
    #[test]
    fn no_word_of_the_table_and_no_blank_is_a_seed() {
        let slot = Uuid::new_v4();
        let agent = LayoutNode::Slot {
            id: slot,
            content: SlotContent::Agent {
                agent_id: "a1".into(),
            },
        };
        for text in [
            // hangul-ok-begin: the Korean word-table entries
            "빈 칸",           // hangul-ok: the Korean word-table entry
            "에이전트",        // hangul-ok: the Korean word-table entry
            "에이전트 트리",   // hangul-ok: the Korean word-table entry
            "프리셋",          // hangul-ok: the Korean word-table entry
            "사용량",          // hangul-ok: the Korean word-table entry
            "알 수 없는 내용", // hangul-ok: the Korean word-table entry
            // hangul-ok-end
            "Empty slot",
            "Agent",
            "Agent tree",
            "Presets",
            "Usage",
            "Unknown content",
            "",
            "  ",
        ] {
            assert_eq!(
                agent_name_seed(text, Some(false), &agent, Some(slot)),
                None,
                "{text:?}"
            );
        }
    }

    // If the placeholder text were planted, that entry would be filled, and the real name of a copy accepted later could not enter as a seed that only fills empty entries.
    #[test]
    fn a_placeholder_saved_at_boot_does_not_keep_a_restored_copys_name_out() {
        let mut live = restore(vec![main_entry(vec![agent_tab(
            "a1",
            "에이전트", // hangul-ok: the Korean word-table entry
            Some(false),
        )])])
        .layout;
        let copy = restore(vec![main_entry(vec![agent_tab(
            "a1",
            "myproj",
            Some(false),
        )])])
        .layout;

        live.adopt_restored(copy).unwrap();

        assert_eq!(listed_names(&live), ["myproj"]);
    }

    #[test]
    fn a_skipped_tab_seeds_nothing() {
        let kept = agent_tab("a1", "에이전트", Some(false)); // hangul-ok: the Korean word-table entry
        let mut twin = agent_tab("a1", "myproj", Some(false));
        twin.id = kept.id;
        let back = restore(vec![main_entry(vec![kept, twin])]);
        assert!(matches!(
            back.warnings.as_slice(),
            [RestoreWarning::DuplicateTab { .. }]
        ));
        assert_eq!(listed_names(&back.layout), ["Agent"]);
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
            let written = to_persisted(mgr);
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
            "explicitly placing empty content makes the raw text vanish"
        );
    }

    #[test]
    fn windows_whose_place_was_never_seen_are_written_and_round_trip() {
        let mut mgr = ViewManager::new();
        let v = main_active(&mgr);
        let s = first_slot(&mgr, v);
        mgr.assign_agent(v, s, "a-1".into()).unwrap();
        mgr.create_tab(MAIN_WINDOW_LABEL, Some("Two".into()))
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
        let written = to_persisted(&mgr);
        assert_eq!(written.len(), 2);
        assert!(written.iter().all(|w| w.bounds.is_none()));
        assert!(written[0].maximized, "main that only saw maximized");

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
        assert_eq!(
            main_tab_ids(&back.layout),
            mgr.windows[MAIN_WINDOW_LABEL].tabs,
            "main's tabs are not lost even if the position is unknown"
        );
        assert_eq!(back.layout.views[&v], mgr.views[&v]);
        assert_eq!(to_persisted(&back.layout), written);
    }

    // ── persisted → memory: fixes ──

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
            named: Some(false),
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

    // A saved number that never overlaps per call — so the number-fix warning does not slip into the warning lists of tests that do not measure numbers.
    fn fresh_number() -> u32 {
        static NEXT: AtomicU32 = AtomicU32::new(2);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    fn entry(id: &str, kind: WindowKind) -> WindowEntry {
        let number = match kind {
            WindowKind::Main(_) => 1,
            WindowKind::Popout(_) => fresh_number(),
        };
        WindowEntry {
            id: id.into(),
            number: Some(number),
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
        let back = restore(vec![
            popout_entry("main", vec![empty_tab()]),
            popout_entry("slot-popup-1", vec![empty_tab()]),
            entry("x", WindowKind::Main(strip(vec![empty_tab()]))),
            main_entry(vec![kept.clone()]),
        ]);
        let mismatched: Vec<(&str, &str)> = back
            .warnings
            .iter()
            .map(|w| match w {
                RestoreWarning::IdKindMismatch { id, kind } => (id.as_str(), *kind),
                other => panic!("only id · kind mismatches expected: {other}"),
            })
            .collect();
        assert_eq!(
            mismatched,
            [
                ("main", "popout"),
                ("slot-popup-1", "popout"),
                ("x", "main")
            ]
        );
        assert_eq!(back.layout.windows.len(), 1);
        assert_eq!(main_tab_ids(&back.layout), [kept.id]);
    }

    #[test]
    fn a_repeated_window_id_keeps_the_first() {
        let popout = Uuid::new_v4();
        let m1 = empty_tab();
        let p1 = empty_tab();
        let back = restore(vec![
            main_entry(vec![m1.clone()]),
            popout_entry(&popout.to_string(), vec![p1.clone()]),
            main_entry(vec![empty_tab()]),
            popout_entry(&popout.to_string().to_uppercase(), vec![empty_tab()]),
        ]);
        assert_eq!(
            back.warnings,
            vec![
                RestoreWarning::DuplicateWindow { id: "main".into() },
                RestoreWarning::DuplicateWindow {
                    id: popout.to_string()
                },
            ]
        );
        assert_eq!(main_tab_ids(&back.layout), [m1.id]);
        assert_eq!(back.layout.windows["slot-popup-1"].tabs, [p1.id]);
        assert_eq!(back.layout.windows.len(), 2);
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
        // No entry.
        let back = restore(vec![]);
        assert_eq!(back.warnings, vec![RestoreWarning::MainWithoutTabs]);
        assert_eq!(main_tab_ids(&back.layout).len(), 1);
        assert_eq!(back.layout.windows[MAIN_WINDOW_LABEL].window_id, "main");

        // The entry exists but no tab remained — the attributes are kept.
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
        assert_eq!(back.layout.views[&only].name, TabName::Auto);
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

        let back = ViewManager::from_persisted(windows(), &Fixed(MAIN_WINDOW_LABEL));
        assert_eq!(back.warnings.len(), 2);
        assert_eq!(back.layout.windows.len(), 1);
    }

    #[test]
    fn a_dropped_popout_holds_no_ids_against_the_windows_after_it() {
        // A popout written before main shares tab ids · slot ids with main and is discarded because of the label.
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

        // A popout discarded because all its tabs overlapped does not hold its own window id either — the later entry with the same id lives.
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

        // A popout that, had it remained, would have reported position · ratio · unknown content · active tab.
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
        let mut nowhere = popout_entry(&Uuid::new_v4().to_string(), vec![empty_tab()]);
        nowhere.bounds = None;
        let flat_id = Uuid::new_v4().to_string();
        let mut flat = popout_entry(&flat_id, vec![empty_tab()]);
        flat.bounds = Some(Bounds {
            h: 0.0,
            ..dto_bounds()
        });
        let back = restore(vec![main, nowhere, flat]);
        assert_eq!(
            back.warnings,
            vec![
                RestoreWarning::BoundsDropped {
                    window_id: "main".into()
                },
                RestoreWarning::BoundsDropped { window_id: flat_id },
            ],
            "having no position is not a warning"
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
        assert_eq!(
            back.layout.window_attrs("slot-popup-2").unwrap().bounds,
            None
        );
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

    // ── Window numbers (TRD S26-window-chrome §3-3 · §3-4) ──

    fn numbers_by_id(layout: &ViewManager) -> Vec<(String, u32)> {
        let mut numbers: Vec<(String, u32)> = layout
            .windows
            .values()
            .map(|w| (w.window_id.clone(), w.number))
            .collect();
        numbers.sort();
        numbers
    }

    #[test]
    fn window_numbers_round_trip_and_closing_numbers_are_not_written() {
        let mut mgr = ViewManager::new();
        mgr.create_window("slot-popup-1").unwrap();
        mgr.create_window("slot-popup-2").unwrap();
        mgr.create_window("slot-popup-3").unwrap();
        mgr.close_window("slot-popup-2").unwrap();
        assert_eq!(mgr.closing_numbers(), [3]);

        let written = to_persisted(&mgr);
        assert_eq!(written[0].number, Some(1), "main writes 1 too");
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
        assert_eq!(numbers_by_id(&back.layout), numbers_by_id(&mgr));
        assert!(back.layout.closing_numbers().is_empty());
        back.layout.assert_number_invariants();
        assert_eq!(to_persisted(&back.layout), written);
    }

    #[test]
    fn a_popout_whose_number_cannot_stand_gets_the_smallest_free_one_and_is_kept() {
        let ids: Vec<String> = (0..4).map(|_| Uuid::new_v4().to_string()).collect();
        let popout = |id: &String, number: Option<u32>| {
            let mut entry = popout_entry(id, vec![empty_tab()]);
            entry.number = number;
            entry
        };
        let mut main = main_entry(vec![empty_tab()]);
        main.number = None;
        let back = ViewManager::from_persisted(
            vec![
                main,
                popout(&ids[0], Some(3)),
                popout(&ids[1], Some(3)),
                popout(&ids[2], None),
                popout(&ids[3], Some(1)),
            ],
            &Labels::default(),
        );
        let given: Vec<u32> = (1..=4)
            .map(|i| back.layout.windows[&format!("slot-popup-{i}")].number)
            .collect();
        assert_eq!(given, [3, 2, 4, 5]);
        assert_eq!(back.layout.windows[MAIN_WINDOW_LABEL].number, 1);
        assert_eq!(
            back.warnings,
            vec![
                RestoreWarning::NumberReassigned {
                    window_id: ids[1].clone(),
                    saved: Some(3),
                    given: 2,
                },
                RestoreWarning::NumberReassigned {
                    window_id: ids[2].clone(),
                    saved: None,
                    given: 4,
                },
                RestoreWarning::NumberReassigned {
                    window_id: ids[3].clone(),
                    saved: Some(1),
                    given: 5,
                },
            ],
            "main's missing number is not reported"
        );
        back.layout.assert_number_invariants();
    }

    // ── change numbers ──

    #[test]
    fn the_revision_moves_with_each_of_its_parts() {
        let mut mgr = ViewManager::new();
        let start = StateRevision::of(&mgr);
        mgr.set_window_theme(MAIN_WINDOW_LABEL, Some(UiTheme::Dark))
            .unwrap();
        let after_attrs = StateRevision::of(&mgr);
        assert_ne!(after_attrs, start);
        assert_eq!(after_attrs.layout, start.layout);
        mgr.create_tab(MAIN_WINDOW_LABEL, None).unwrap();
        let after_layout = StateRevision::of(&mgr);
        assert_ne!(after_layout, after_attrs);
        assert_eq!(after_layout.attrs, after_attrs.attrs);
    }
}
