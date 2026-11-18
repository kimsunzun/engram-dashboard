//! S14 layout types (wire mirror) — layout authority = src-tauri (ADR-0035). Defined and exported only inside src-tauri, never in the protocol/daemon crate (the daemon is UI-agnostic and knows nothing of View — ADR-0035); mirrored to the frontend (`src/api/layoutTypes.ts`) with ts-rs, a channel separate from the daemon wire contract (the protocol crate).

use ts_rs::TS;
use uuid::Uuid;

use super::geometry::{Insets, SlotRect, SplitRect};
use super::spatial::SlotSpatial;

/// The name states the resulting layout — `LeftRight` is always left/right, `TopBottom` is always top/bottom.
/// Do not go back to the axis vocabulary (`Horizontal`/`Vertical`): the tmux (by axis) and vim (by divider line) conventions
/// are opposite, so the name alone did not tell which layout it was. // ADR-0140
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SplitDir {
    LeftRight,
    TopBottom,
}

/// Slot occupant = typed union (ADR-0060).
///
/// Later content kinds (FileTree/ControlPanel) are added as variants, and user customization (a preset button set) is expressed not as a new
/// variant but as config data inside a variant (prevents enum explosion — the key insight of ADR-0060).
///
/// ★Invariant (ADR-0060)★: the `Agent` variant holds **only the binding (the agent_id reference string)** of a daemon agent —
/// the live output stream is not held here; `OutputRouter` routes it separately by agent_id (ADR-0041/0042/0046).
/// The epoch (incarnation marker, ADR-0163) is also outside the layout tree (owned by agentStore) — the trigger for re-subscription is not this field but
/// observation of the authoritative roster (ADR-0164 decision 8).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum SlotContent {
    /// "empty" in the resolve_spawn_slot occupancy decision (ADR-0059).
    Empty,
    /// "occupied" in resolve_spawn_slot (ADR-0059).
    Agent { agent_id: String },
    /// MVP = a unit with no fields — only names the render target, and the data is held by agentStore (expresses only the content kind, ADR-0060).
    AgentList,
    /// Preset palette (a button set of registered cwd presets) view. MVP = a unit with no fields — the preset list data is
    /// owned by the daemon (presets.json, ADR-0061), so it is not held here and flows separately over the PresetRegistry wire.
    PresetPalette,
    /// Usage limit view. The two fields = the companies this slot switched on — the shell computes the daemon usage subscription (interest) from them
    /// (`daemon_client::usage_interest`). The values themselves are not held here.
    /// An input missing a field is read as `true` — a bare `{"type":"usage"}` is a slot with both companies on.
    Usage {
        #[serde(default = "enabled")]
        show_claude: bool,
        #[serde(default = "enabled")]
        show_codex: bool,
    },
}

fn enabled() -> bool {
    true
}

impl SlotContent {
    pub fn is_empty(&self) -> bool {
        matches!(self, SlotContent::Empty)
    }

    pub fn agent_id(&self) -> Option<&str> {
        match self {
            SlotContent::Agent { agent_id } => Some(agent_id),
            SlotContent::Empty
            | SlotContent::AgentList
            | SlotContent::PresetPalette
            | SlotContent::Usage { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum LayoutNode {
    /// id = globally unique across windows (UUID). // ADR-0035
    Slot {
        #[ts(type = "string")]
        id: Uuid,
        content: SlotContent,
    },
    /// ratio = the share taken by a (clamped to `tree::RATIO_MIN`..`RATIO_MAX`, default 0.5).
    Split {
        /// Identity of this split node — drawn anew for every split, and follows the node even when it moves place (sibling promotion). The frontend
        /// uses this value (also carried as `split_id` of the snapshot's `split_rects`) as the key of the divider (`Splitter`) and as the marker of the split
        /// being dragged in the drag preview (`ViewLayoutRenderer` · `splitPreview`).
        // ADR-0223
        #[ts(type = "string")]
        id: Uuid,
        dir: SplitDir,
        // ADR-0227
        ratio: f64,
        a: Box<LayoutNode>,
        b: Box<LayoutNode>,
    },
}

/// Origin of a tab name (TRD S26-window-chrome §3-1 · PRD R4) — `Auto` = no name was set (the visible name is computed each time by
/// `tab_name::effective_tab_name` from the focus slot) · `Named` = a name set by a person or the LLM (empty text and whitespace-only
/// text are not held).
// The name of an auto tab is not held as a string — if someone reads a stale string (「View 1」) directly, the OS title shows that text (AC3).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type", content = "name", rename_all = "snake_case")]
#[ts(export)]
// ADR-0301 — tab name · OS title
pub enum TabName {
    Auto,
    Named(String),
}

impl TabName {
    /// The name that was set — `None` if empty or whitespace-only (AC36). The text is held as is, without trimming.
    pub fn named(name: String) -> Option<Self> {
        (!name.trim().is_empty()).then_some(TabName::Named(name))
    }

    pub fn is_named(&self) -> bool {
        matches!(self, TabName::Named(_))
    }
}

/// One View (a tab/popup) = layout tree + focus slot.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct View {
    #[ts(type = "string")]
    pub id: Uuid,
    pub name: TabName,
    pub layout: LayoutNode,
    /// If the slot it pointed to disappears, falls back to the first slot of the tree.
    #[ts(type = "string | null")]
    pub focused_slot_id: Option<Uuid>,
}

/// View meta for the tab bar (without the layout body — the `window:tabs-updated` payload).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct ViewMeta {
    #[ts(type = "string")]
    pub id: Uuid,
    /// The visible name — the name that was set, or if none was set, the auto name at that moment (the name of the focus slot).
    pub name: String,
    /// Whether a name was set — if false, `name` changes following slot content and focus. Sending a rename with the same text makes it
    /// a set name (TRD S26-window-chrome §3-1).
    pub named: bool,
}

/// The `get_view` response + the `layout:updated` payload.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct ViewSnapshot {
    #[ts(type = "string")]
    pub view_id: Uuid,
    pub layout: LayoutNode,
    #[ts(type = "string | null")]
    pub focused_slot_id: Option<Uuid>,
    /// Slot ids holding content this version does not know (a slot kind saved by a newer version), in tree preorder. The `content` of those slots is
    /// carried as `empty` but they are not empty — the empty-slot decision (auto placement · choosing the target to open) treats the slots in this list as
    /// occupied. Only the shell holds the raw text. Putting content there (including `empty`) or closing the slot removes it from this list.
    #[ts(type = "Array<string>")]
    pub foreign_slots: Vec<Uuid>,
    /// ★Slot spatial target derivation (ADR-0068)★: the directional neighbours (up/down/left/right) + order (ordinal) of each leaf slot.
    /// Computed from the logical plan (split direction · ratio) — independent of pixels · getBoundingClientRect (backend authority, ADR-0035).
    /// Held in ordinal order. Normalized coordinates are carried not by this field but by `slot_rects` (ADR-0227 — amends ADR-0068 decision 3).
    pub slot_spatial: Vec<SlotSpatial>,
    /// The rectangle of each leaf slot — normalized [0,1] boundary form `x0,y0,x1,y1` relative to the view (not pixels). Shared boundary values are equal
    /// bit for bit. Held in tree preorder (node → a subtree → b subtree) — different from the ordinal order of `slot_spatial`,
    /// so match pairs by `slot_id`.
    // ADR-0227
    pub slot_rects: Vec<SlotRect>,
    /// The box of each split node (the area joining its two children, the same boundary form as `slot_rects`) + the split boundary `at`
    /// (the x-axis value if `dir` is `left_right`, the y-axis value if `top_bottom`). Held in tree preorder.
    // ADR-0227
    pub split_rects: Vec<SplitRect>,
    /// The shell limits of the split ratio (a side's share) — the shell reads ratios clamped to `[ratio_min, ratio_max]`, and `slot_rects` ·
    /// `split_rects` are values computed with the ratio clamped that way. The screen uses these two values and keeps no limit constants of its own.
    // ADR-0227
    pub ratio_min: f64,
    pub ratio_max: f64,
    /// +1 per change (ViewManager.version).
    /// The ts-rs u64 default mapping is bigint, but serde_json serializes it as a number (runtime = JS number) → the type is also fixed to number
    /// (on a mismatch the frontend race guard `snap.version > pulled` raises a bigint/number mixing error, FIX-1). As a counter, 2^53 is unrealistic.
    #[ts(type = "number")]
    pub version: u64,
}

/// The outcome of a ratio write (`set_split_ratio`).
///
/// - `Applied`: the value changed — version rose and the layout snapshot of that view is notified.
/// - `Unchanged`: the clamped value equals the current value — no change, no notification.
/// - `TooSmall`: nothing was touched — no change, no notification. There are two reasons — the split box is not large enough to give both sides
///   at least `min_pane_px` (the px range is empty), or writing that value would make the width/height of some area 0.
///
/// The spelling is the same as the `outcome` of the command bus `split.setRatio`.
// ★Do not attach `#[serde(rename_all)]` here★ — the bus declaration macro cannot attach a rename, so the variant names become
// the wire values as they are. Changing only here makes the two surfaces speak the same outcome in different spellings (the bus-side twin = `commands::RatioOutcome` ·
// the test that checks the spellings against each other = `both_surfaces_spell_the_split_ratio_outcome_the_same_way` in `tests/layout_commands.rs`).
// ADR-0227
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub enum SplitRatioOutcome {
    Applied,
    Unchanged,
    TooSmall,
}

/// The answer of the `set_split_ratio` invoke. `ratio` = the ratio the shell actually holds (a side = the share of the left/top area — ADR-0140) —
/// the value just written if `Applied`, otherwise the current value left untouched.
// ADR-0227
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct SplitRatioApplied {
    pub ratio: f64,
    pub outcome: SplitRatioOutcome,
    /// Meaningful only for `Applied` — equal to the `version` of the layout snapshot notified by this write (a value taken inside the same lock).
    /// For `Unchanged` · `TooSmall` there is no notification and this value is just the global counter at that moment, so there is
    /// nothing to wait for. The reason for fixing it to number is the same as `ViewSnapshot.version`.
    #[ts(type = "number")]
    pub version: u64,
}

/// Basic area-frame metrics — the argument of `report_ui_metrics` that the webview reports to the shell (`{ metrics: UiMetrics }`).
///
/// - `frame_insets`: the four inner border widths of the area frame (CSS px). They are **measured values** — fractions can occur depending on scale.
///   Each value must be finite and `0 ≤ v ≤ 64`.
/// - `min_pane_px`: the minimum area size (CSS px). It is not a measurement but a **policy constant** of the screen, and its only source of truth is
///   the screen — the shell only receives and uses it and keeps no constant of its own. Must be `1 ≤ v ≤ 1000`.
///
/// If out of range the shell refuses (`Err`) and keeps that window's previous value. A report neither raises version nor notifies
/// anyone.
// ADR-0227
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct UiMetrics {
    pub frame_insets: Insets,
    pub min_pane_px: u32,
}

impl LayoutNode {
    pub fn new_empty_slot() -> Self {
        LayoutNode::Slot {
            id: Uuid::new_v4(),
            content: SlotContent::Empty,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SlotContent;

    #[test]
    fn a_usage_slot_reads_missing_flags_as_on() {
        let both: SlotContent = serde_json::from_str(r#"{"type":"usage"}"#).unwrap();
        assert_eq!(
            both,
            SlotContent::Usage {
                show_claude: true,
                show_codex: true
            }
        );
        let one: SlotContent =
            serde_json::from_str(r#"{"type":"usage","show_codex":false}"#).unwrap();
        assert_eq!(
            one,
            SlotContent::Usage {
                show_claude: true,
                show_codex: false
            }
        );
        assert_eq!(
            serde_json::to_value(SlotContent::Usage {
                show_claude: false,
                show_codex: true
            })
            .unwrap(),
            serde_json::json!({ "type": "usage", "show_claude": false, "show_codex": true })
        );
    }
}
