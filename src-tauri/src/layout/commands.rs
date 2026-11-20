//! The shell-owned command table: `window`/`tab`/`slot` declarations and bodies live next to the apply service (ADR-0155 decision 1).
//!
//! Human clicks (`#[tauri::command]` in `commands/layout.rs`) and relayed LLM calls (the inbound receiver) land on **the same apply service** (`super::apply`); this file is the second thin wrapper onto it, not a second control surface (ADR-0081 decision 3).
//!
//! ★Shell commands outside the layout also live here★ — `window.setTheme` · `window.getTheme` · `settings.*` · `restore.*` skip the apply service but are shell-owned names. A second declaration block would split the registration packet, generation number and duplicate check per table (the macro also allows one block per module), so the ports carry their own services ([`LayoutPorts::themes`] · [`LayoutPorts::settings`] · [`LayoutPorts::settings_events`] · [`LayoutPorts::restore`]).
//!
//! ## ★Where the declaration diverges from the service★
//! The declaration macro's type alphabet has no `Uuid`, nested enums or recursive types:
//! - **All ids are `String`**; the handler parses them (malformed = `INVALID_ARGUMENT`).
//! - **`SlotContent` arrives as a tag + side fields `agent_id` · `assign` · `show_claude` · `show_codex`** (macro enums are fieldless); [`slot_content`] checks the combination.
//! - **`get_view` is not declared** (it returns the recursive `ViewSnapshot`), so the queries are `tab.list` · `window.list` · `slot.resolveSpatial`.
//!
//! ## ★Codes of apply failures★ — layout commands only
//! `window.*Theme` · `settings.*` · `restore.answer` get typed errors from their services and map by kind ([`theme_error`] · [`settings_error`] · [`restore_error`]). For the apply service:
//! - **Verbs taking ids** (id version of the apply service — `tab.create` · `split.setRatio` …) get failures only as `String`. A code must not be synthesized from the text (no machine branching on `message`), so they use **the one code true for every failure**: `CONFLICT` = "cannot be applied in the current state" (`NOT_FOUND` is false for the main window refusal, `INTERNAL` for a mistyped id). The text carries the reason.
//! - **Docking verbs taking a target** (`tab.move` · `tab.dock` · `tab.detach`) map by kind from the typed target-version errors ([`dock_target_error`] · [`locate_error`]). `layout.get` has no apply failure and emits only window argument errors ([`window_arg_error`]).
//! - **Widened area · tab verbs** (`slot.*` · `layout.setSlotContent` · `tab.switch` · `tab.close` · `tab.rename`) map target-version refusals with [`target_error`]. ★A tab or area given by id that is missing stays exactly what the id version emitted (`CONFLICT` + the manager text — [`id_miss`])★; what was picked by agent or word is [`locate_error`].
//!
//! ## ★Agent-picking arguments arrive only as ids★
//! Fields marked `#[agent_ref]` (`tab_of` · `slot_of` · `assign` …): the daemon resolves names to ids before relaying and answers NOT_FOUND / CONFLICT itself without relaying (ADR-0308). So handlers see only ids there, and a non-UUID is a call that bypassed the daemon: INVALID_ARGUMENT ([`agent_arg`]). The shell does not resolve names; its display name cache is for display (ADR-0132).
// ADR-0308 (TRD §4-⑦; TRD S28 §5-3; TRD S28 §7)

use std::sync::Arc;

use uuid::Uuid;

use engram_dashboard_base::path::normalize_spelling;
use engram_dashboard_command::{
    blocking_handler, declare_commands, CommandError, CommandFuture, CommandHandler, CommandTable,
    ErrorCode, RetryMode,
};

use super::apply::{
    self, DockOutcome, SpawnAtError, SpawnRefusal, TabBarSpot, TargetDest, TargetDockError,
    TargetDockRequest, TargetError, WindowPlace,
};
use super::dock;
use super::manager::{DockError, DockMove, FocusPolicy, LayoutError};
use super::rows::{self, RowFilter};
use super::spatial::SpatialToken;
use super::target::{self, LocateError, Place, Target};
use super::tree;
use super::window_arg::{self, WindowArgError};
use super::window_ctl::{
    self, Activation, Applied, CloseOutcome, WindowAction, WindowControl, WindowCtlError,
};
use super::{
    AgentSpawner, LabelSource, LayoutEvents, LayoutState, SlotContent, SplitDir, SubscriptionSync,
    WindowHost, MAIN_WINDOW_LABEL,
};
use crate::settings::{
    reset_and_notify, set_and_notify, SettingsError, SettingsEvents, SettingsService,
};
use crate::state::restore::{AnswerError, RestoreCoordinator};
use crate::theme::{ThemeControl, ThemeError, UiTheme, WindowTheme};

// Command names reuse the ids the frontend registry already registers (`tab.create`, `slot.focus`, ...); a second spelling would
// give one action two names. Names without a frontend twin (queries, `slot.split` with the direction as an argument,
// `window.*Theme`, `split.*`) are coined here.
// `catalog_version` is bumped whenever the declarations change, including answer shapes and the vocabulary or summaries of a
// field, not only when names grow (macro contract: the `CATALOG_VERSION` item of `declare_commands!`). Not branching material
// for receivers (`CommandEnvelope::proto_ver`), and unrelated to `engram_dashboard_protocol::PROTOCOL_VERSION`.
// Generation 22: the slot kind `PresetPalette` is gone (TRD S25-preset-library §5-2 1 · ADR-0314 decision 5); the `agent.spawnInto`
//   answer gains `notices`, its refusals carry `args.reasons` / `args.agent_id` (TRD S25 step 20 · ADR-0348), and it is declared with
//   the launch deadline class (ADR-0369).
// ADR-0291
// ADR-0227
// ADR-0225
// ADR-0276
// ADR-0219 (TRD §6; TRD S21-storage §10 F7; TRD S28 §4 · §7; TRD S26-window-chrome §6-1 · §6-6; TRD S21-storage §6-5; TRD S21-storage §6-5 · §6-7; TRD S21-storage §6-7; TRD S21-storage §5-4; TRD S21 usage-limit-slot §1-7; TRD S26-window-chrome §3-2 · §4-5 · §5-3; TRD S21-storage §5-6 · §5-7)
declare_commands! {
    catalog_version: 22;

    /// One tab bar entry — `name` = the visible name · `named` = whether the name was set (if false, `name` changes following the focused area's content).
    struct TabRow {
        id: String,
        name: String,
        named: bool,
    }

    /// The direction to split — the name states the resulting layout (`LeftRight` = left/right, `TopBottom` = top/bottom). // ADR-0140
    ///
    /// ★The spelling differs from the Tauri invoke path (`left_right`/`top_bottom`)★ — the declaration macro cannot attach serde rename,
    /// so the Rust variant name becomes the wire value as is. That the two surfaces take the same meaning in different spellings is left
    /// knowingly, and the handler does the conversion.
    enum SplitDirection {
        LeftRight,
        TopBottom,
    }

    /// What the slot holds — only `Agent` also takes `agent_id`, and only `Usage` takes `show_claude` · `show_codex`.
    enum SlotContentKind {
        Empty,
        Agent,
        AgentList,
        Usage,
    }

    /// The outcome of a ratio write — `Applied` = changed · `Unchanged` = the clamped value equals the current value (no change) ·
    /// `TooSmall` = untouched (the split is not big enough to give both sides the minimum area size, or with that value some area's
    /// width · height becomes 0).
    ///
    /// The twin inside the shell is `super::types::SplitRatioOutcome` (the answer of Tauri `set_split_ratio`) — the type names
    /// differ and the variant spellings are the same (the same relation as `SplitDirection`↔`SplitDir` · a test holding them against each other guards it). The handler
    /// does the conversion.
    // ADR-0227
    enum RatioOutcome {
        Applied,
        Unchanged,
        TooSmall,
    }

    /// One split — a = left/top, b = right/bottom (ADR-0140). `ratio` = the share of side a's area.
    /// `a_slots` · `b_slots` = the slot ids in that side's subtree (tree preorder).
    // ADR-0227
    struct SplitRow {
        split_id: String,
        dir: SplitDirection,
        ratio: f64,
        a_slots: Vec<String>,
        b_slots: Vec<String>,
    }

    /// One shell setting line — value is the canonical string, is_default = no overriding value, so the default is in use.
    struct SettingRow {
        key: String,
        value: String,
        is_default: bool,
    }

    /// The shape of one shell settings key.
    struct SettingSchemaRow {
        key: String,
        kind: String,
        default: String,
        choices: Option<Vec<String>>,
        min: Option<String>,
        max: Option<String>,
        description: String,
    }

    /// One window — window_id = the same window across restarts (the value to pass as the window argument) · window = label (only within that run) · number = the visible
    /// window number (reused — not accepted as a window argument) · main = whether it is the main window · visible · minimized · maximized = the OS
    /// state at that moment · focused = whether that window is the OS foreground window. A window whose OS window could not be read has all four false.
    struct WindowRow {
        window_id: String,
        window: String,
        number: u32,
        main: bool,
        visible: bool,
        minimized: bool,
        maximized: bool,
        focused: bool,
    }

    /// The outcome of window.close — Closed = closed the popout · HiddenAll = it is main, so instead of closing, all windows were hidden · Kept =
    /// if_empty but the window has tabs or was already closed, so it was left as is.
    enum WindowCloseOutcome {
        Closed,
        HiddenAll,
        Kept,
    }

    /// The kind of what an area holds — Unknown = content this version does not know (an area saved by a newer version). The twin inside the shell is `rows::SlotKind`.
    // ADR-0307
    enum SlotKind {
        Empty,
        Agent,
        AgentList,
        Usage,
        Unknown,
    }

    /// The side where the attached tab goes — it attaches to that edge of the area to attach to (or the whole tab). The twin inside the shell is `dock::DockSide` (the macro cannot
    /// carry enums outside the block, so one more with the same spelling is kept).
    // ADR-0307
    enum DockSide {
        Left,
        Right,
        Top,
        Bottom,
    }

    /// One area's row — the field meanings are in layout.get's summary. The four ids (window_id · tab · slot_id · agent_id) are included only with layout.get's
    /// ids = true — omitted fields are left out of the answer entirely (not null).
    // ADR-0307
    struct LayoutRow {
        window: String,
        window_number: u32,
        tab_index: u32,
        tab_name: String,
        tab_active: bool,
        content: SlotKind,
        agent: Option<String>,
        focused: bool,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        window_id: Option<String>,
        tab: Option<String>,
        slot_id: Option<String>,
        agent_id: Option<String>,
    }

    /// A launch notice (`agent.spawnInto`) — something the launch left out, on purpose or because it was not found.
    /// Not a refusal: the launch went ahead. The same row as the agent block's `agent.spawn` notices (ADR-0348).
    struct LaunchNoticeRow {
        /// Fixed notice code.
        code: String,
        /// The code's arguments (keys depend on `code`).
        args: Json,
        /// 1-based line in the file it names — `null` when it names none.
        line: Option<u32>,
        /// The notice as one readable line.
        text: String,
    }

    /// Creates one more empty tab in the window and activates it. Without name it is an auto name — it changes following that tab's focused area (for an agent, its name)
    /// (named = false in tab.list). With name it is a set name (leading/trailing whitespace is trimmed).
    /// window = window_id from window.list (the same window across restarts) or a label within the same run. Numbers are not accepted (they are
    /// reused — INVALID_ARGUMENT). A label is valid only within that run.
    #[effect(Write)]
    #[since(1)]
    "tab.create" => args TabCreateArgs {
        window: String,
        name: Option<String>,
    } -> ok TabCreateOk {
        view_id: String,
    } errors [CONFLICT];

    // ADR-0307
    /// Changes the visible tab of the window (other windows unchanged). Tab = one of view_id | tab_of (the tab holding that agent — name or id).
    /// window may be omitted (known from the tab) — if given, it must be that tab's window and is a window_id or a label within the same run (numbers are not
    /// accepted).
    /// Errors: NOT_FOUND if tab_of is in no area or that name does not exist · CONFLICT if it is up in several tabs (retry with a view_id= value of the candidates
    /// at the end of the text as view_id) · CONFLICT if there are two or more agents with that name (candidates = agent ids — retry with tab_of) ·
    /// CONFLICT for a missing view_id.
    #[effect(Write)]
    #[since(1)]
    "tab.switch" => args TabSwitchArgs {
        window: Option<String>,
        view_id: Option<String>,
        #[agent_ref]
        tab_of: Option<String>,
    } -> ok TabSwitchOk {} errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// Closes the tab — if it is the window's last tab, that window closes too. Tab = one of view_id | tab_of (the tab holding that agent — name or
    /// id). window may be omitted (known from the tab) — if given, it must be that tab's window and is a window_id or a label within the same run (numbers are
    /// not accepted).
    /// Answer = layout (the area rows of the tab now visible in that window — layout.get rows shape · no ids · an empty list if the whole window closed).
    /// Errors: NOT_FOUND if tab_of is in no area or that name does not exist · CONFLICT if it is up in several tabs (retry with a view_id= value of the candidates
    /// at the end of the text as view_id) · CONFLICT if there are two or more agents with that name (candidates = agent ids — retry with tab_of) ·
    /// CONFLICT for a missing view_id.
    #[effect(Write)]
    #[since(1)]
    "tab.close" => args TabCloseArgs {
        window: Option<String>,
        view_id: Option<String>,
        #[agent_ref]
        tab_of: Option<String>,
    } -> ok TabCloseOk {
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// Renames the tab (leading/trailing whitespace is trimmed · the window is known from the tab). Tab = one of view_id | tab_of (the tab holding that agent — name or
    /// id). Afterwards the name is fixed — even if it is the same text as the auto name now visible, it no longer follows the focused area · agent
    /// name, and there is no command that reverts to the auto name.
    /// Errors: NOT_FOUND if tab_of is in no area or that name does not exist · CONFLICT if it is up in several tabs (retry with a view_id= value of the candidates
    /// at the end of the text as view_id) · CONFLICT if there are two or more agents with that name (candidates = agent ids — retry with tab_of) ·
    /// CONFLICT for a missing view_id.
    #[effect(Write)]
    #[since(1)]
    "tab.rename" => args TabRenameArgs {
        view_id: Option<String>,
        #[agent_ref]
        tab_of: Option<String>,
        name: String,
    } -> ok TabRenameOk {} errors [NOT_FOUND, CONFLICT];

    /// The window's tab list + active tab + version. The window in the answer is that window's label.
    /// window = window_id from window.list (the same window across restarts) or a label within the same run. Numbers are not accepted (they are
    /// reused — INVALID_ARGUMENT). A label is valid only within that run.
    #[effect(Read)]
    #[since(1)]
    "tab.list" => args TabListArgs {
        window: String,
    } -> ok TabListOk {
        window: String,
        tabs: Vec<TabRow>,
        active: String,
        version: u64,
    } errors [CONFLICT];

    /// Opens a new window holding one empty tab. Answer = window (label — valid only within that run) · window_id (the same window across restarts
    /// — the value to pass as the window argument) · number (the visible window number — closed windows' numbers are reused, so it is not accepted as a window argument).
    #[effect(Write)]
    #[since(1)]
    "window.create" => args WindowCreateArgs {}
                    -> ok   WindowCreateOk { window: String, window_id: String, number: u32 }
                    errors [CONFLICT];

    /// Closes the window — a popout closes with its tabs (outcome = Closed), main is not closed but all windows are hidden (HiddenAll — the same as
    /// window.hideAll · window.showAll or the tray 「Show」 bring them back). With if_empty = true, window must be that window's window_id, and
    /// if tabs have come into that window or it is already closed, it is not closed (Kept — not an error). window is the same as tab.create (window_id
    /// or a label within the same run · numbers are not accepted — INVALID_ARGUMENT even with if_empty).
    #[effect(Write)]
    #[since(1)]
    "window.close" => args WindowCloseArgs {
        window: String,
        if_empty: Option<bool>,
    } -> ok WindowCloseOk {
        outcome: WindowCloseOutcome,
    } errors [CONFLICT];

    /// The windows open now. windows = all labels · details = per window window_id · label · number · whether main · OS state (in number order ·
    /// field meanings in the row shape). Window numbers reuse the smallest free number — while a closing window has not gone yet, its number does not
    /// become free (that window is not in the list). To point at a window, pass the window_id, not the number.
    #[effect(Read)]
    #[since(1)]
    "window.list" => args WindowListArgs {}
                  -> ok   WindowListOk { windows: Vec<String>, details: Vec<WindowRow> }
                  errors [CONFLICT];

    /// Minimizes the window (no-op if already minimized). A window that is not visible is CONFLICT — for a hidden window first window.raise or
    /// window.showAll; a window before its first paint shows soon. window is the same as tab.create (window_id or a label within the same run ·
    /// numbers are not accepted).
    #[effect(Write)]
    #[since(20)]
    "window.minimize" => args WindowMinimizeArgs {
        window: String,
    } -> ok WindowMinimizeOk {} errors [CONFLICT];

    /// Maximizes the window (no-op if already maximized). A window that is not visible (hidden · before its first paint) is deferred without touching the screen,
    /// and shows maximized when that window is shown — deferred = true. window is the same as tab.create (window_id or a label within the same run ·
    /// numbers are not accepted).
    #[effect(Write)]
    #[since(20)]
    "window.maximize" => args WindowMaximizeArgs {
        window: String,
    } -> ok WindowMaximizeOk {
        deferred: bool,
    } errors [CONFLICT];

    /// Restores a maximized window to its original size (no-op if not maximized · also a no-op for a visible minimized window — first window.raise). A window that is not
    /// visible (hidden · before its first paint) is deferred without touching the screen, and shows at its original size when that window is shown — deferred =
    /// true. window is the same as tab.create (window_id or a label within the same run · numbers are not accepted).
    #[effect(Write)]
    #[since(20)]
    "window.unmaximize" => args WindowUnmaximizeArgs {
        window: String,
    } -> ok WindowUnmaximizeOk {
        deferred: bool,
    } errors [CONFLICT];

    /// Brings the window forward — if hidden, shows all windows (shown_all = true), if minimized restores it, raises it to the top, then requests the foreground.
    /// It does not steal: if another app is in front, it is only raised to the top with focused = false (focused is whether that window is then the OS foreground window, as is).
    /// A window before its first paint is not touched (it shows soon — pending = true). window is the same as tab.create (window_id or a label within the same
    /// run · numbers are not accepted).
    #[effect(Write)]
    #[since(20)]
    "window.raise" => args WindowRaiseArgs {
        window: String,
    } -> ok WindowRaiseOk {
        shown_all: bool,
        focused: bool,
        pending: bool,
    } errors [CONFLICT];

    /// Hides all windows (to the tray — the same as closing main). To bring them back, window.showAll or window.raise.
    #[effect(Write)]
    #[since(20)]
    "window.hideAll" => args WindowHideAllArgs {} -> ok WindowHideAllOk {} errors [];

    /// Shows all hidden windows — without stealing focus (if another app is in front, it only shows them). Windows before their first paint are skipped (they show
    /// by themselves soon). focused = whether main is then the OS foreground window.
    #[effect(Write)]
    #[since(20)]
    "window.showAll" => args WindowShowAllArgs {} -> ok WindowShowAllOk {
        focused: bool,
    } errors [];

    // ADR-0265
    /// Reads one window's theme. window = window_id from window.list (the same window across restarts) or a label within the same run —
    /// numbers are not accepted (they are reused — INVALID_ARGUMENT).
    /// theme = the theme set for that window only (dark · light · e-ink) — null means not set, following the global theme (settings'
    /// theme.default). effective = the theme painted on that window now (theme.default if theme is null).
    /// A missing window = CONFLICT.
    #[effect(Read)]
    #[since(14)]
    "window.getTheme" => args WindowGetThemeArgs {
        window: String,
    } -> ok WindowGetThemeOk {
        theme: Option<String>,
        effective: String,
    } errors [CONFLICT];

    // ADR-0265
    /// Sets a theme for one window only — theme = dark · light · e-ink (case-insensitive), or null (clears that window's theme so it follows the global
    /// theme.default). Omitting the theme field is INVALID_ARGUMENT — to clear, give null. window is the same as
    /// window.getTheme (window_id or a label within the same run · numbers are not accepted).
    /// It is saved with that window in the screen state and survives restarts (it vanishes with the window when closed · a run where restore.status's saves is false
    /// does not save) and sends the new theme to that window — slots are not remounted (the conversation stays). To change all windows
    /// at once, settings.set theme.default.
    /// Answer = theme · effective after the write (the same shape as window.getTheme) — success means it was sent to that window, not that the painting was
    /// confirmed (a window not up yet receives it when it comes up). A missing window = CONFLICT · an unknown theme = INVALID_ARGUMENT · set but
    /// could not be sent to that window = INTERNAL (the value remains and window.getTheme answers the new value).
    #[effect(Write)]
    #[since(14)]
    "window.setTheme" => args WindowSetThemeArgs {
        window: String,
        theme: Option<Option<String>>,
    } -> ok WindowSetThemeOk {
        theme: Option<String>,
        effective: String,
    } errors [CONFLICT];

    // ADR-0307
    /// Splits an area in two — dir = LeftRight (two areas left/right) · TopBottom (two areas top/bottom). Area = one of slot_id (if view_id is given too, it must be an area of that
    /// tab) | slot_of (the area holding that agent — name or id) | at (a spatial word — in window's visible tab · main if window
    /// is omitted). at = top-left · top-right · bottom-left · bottom-right (that corner area) · left · right · up ·
    /// down (that neighbor of the focused area). window is used only with at and is a window_id or a label within the same run (numbers are not accepted).
    /// Answer = slot_id (the newly created area) · layout (the area rows of that tab — layout.get rows shape · no ids).
    /// Errors: NOT_FOUND if slot_of is in no area or that name does not exist · or if the word picks no area · CONFLICT if slot_of is up in several
    /// areas (retry with a slot_id= value of the candidates at the end of the text as slot_id) · CONFLICT if there are two or more agents with that name
    /// (candidates = agent ids — retry with slot_of) · CONFLICT for a missing slot_id · view_id.
    #[effect(Write)]
    #[since(1)]
    "slot.split" => args SlotSplitArgs {
        view_id: Option<String>,
        slot_id: Option<String>,
        dir: SplitDirection,
        #[agent_ref]
        slot_of: Option<String>,
        at: Option<String>,
        window: Option<String>,
    } -> ok SlotSplitOk {
        slot_id: String,
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// Closes an area (a sibling inherits its place). Area = one of slot_id (if view_id is given too, it must be an area of that tab) | slot_of (the area
    /// holding that agent — name or id) | at (a spatial word — in window's visible tab · main if window is omitted). at =
    /// top-left · top-right · bottom-left · bottom-right (that corner area) · left · right · up · down (that neighbor of the focused area).
    /// window is used only with at and is a window_id or a label within the same run (numbers are not accepted).
    /// Answer = layout (the remaining area rows of that tab — layout.get rows shape · no ids).
    /// Errors: NOT_FOUND if slot_of is in no area or that name does not exist · or if the word picks no area · CONFLICT if slot_of is up in several
    /// areas (retry with a slot_id= value of the candidates at the end of the text as slot_id) · CONFLICT if there are two or more agents with that name
    /// (candidates = agent ids — retry with slot_of) · CONFLICT for a missing slot_id · view_id.
    #[effect(Write)]
    #[since(1)]
    "slot.close" => args SlotCloseArgs {
        view_id: Option<String>,
        slot_id: Option<String>,
        #[agent_ref]
        slot_of: Option<String>,
        at: Option<String>,
        window: Option<String>,
    } -> ok SlotCloseOk {
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0227
    // ADR-0140
    /// Sets the ratio of one split (divider). ratio = the share of side a's (left/top) area — the share within the region that split divides, not the whole
    /// window, and if side a has several areas, the share of that whole group. With 0.3, the left (the top for a top/bottom
    /// split) gets 30% of that region. split_id comes from split.list.
    /// An out-of-range value is not an error: the shell clamps it to 0.01~0.99, and if it knows the window's size, clamps once more so each side is at least
    /// the screen's minimum area size, then applies that value and returns it as the answer's ratio.
    /// outcome — Applied = changed · Unchanged = the value clamped that way equals the current value, so nothing changed ·
    /// TooSmall = untouched because the split is too small (or that value would make some area vanish). If not Applied,
    /// ratio is the current untouched value.
    #[effect(Write)]
    #[since(8)]
    "split.setRatio" => args SplitSetRatioArgs {
        view_id: String,
        split_id: String,
        ratio: f64,
    } -> ok SplitSetRatioOk {
        ratio: f64,
        outcome: RatioOutcome,
    } errors [CONFLICT];

    // ADR-0227
    // ADR-0140
    /// All splits (dividers) of that tab — in tree preorder. Row = split_id · dir (LeftRight = left/right, TopBottom = top/bottom) ·
    /// ratio (= the share of side a's (left/top) area — the share within the region that split divides, not the whole window, and if side a has several
    /// areas, the share of that whole group) · a_slots (slot ids on side a = left/top) · b_slots (b = right/bottom side).
    /// The split separating slots x and y is the row where x is in one side's list and y in the other (there is only one).
    /// Nesting is read as list inclusion — if a row's a_slots · b_slots all fall into one side's list of another row, that row
    /// is a split that divides that side's region again.
    /// To change a ratio, call split.setRatio with that split_id.
    #[effect(Read)]
    #[since(8)]
    "split.list" => args SplitListArgs {
        view_id: String,
    } -> ok SplitListOk {
        splits: Vec<SplitRow>,
    } errors [CONFLICT];

    // ★A known transitional split — the same id **takes and gives** different things on the two surfaces★: the frontend registry also
    //   has `slot.popout` (`src/commands/slotCommands.ts`). Takes — that side has no destination argument, so it is **always a new
    //   window** (it uses only the focused coordinates), while here `to_window` also picks an existing window. Gives — that side gives
    //   `{window, tab, window_id, number}`, here `{window, new_view_id, window_id, number}`. Do not fix only one side — the join is the job of the follow-up step that retires
    //   the frontend's own registry (the screen-side registration), and changing one side now only grows what that step
    //   has to move. The other side carries the same memo.
    // ADR-0307
    /// Moves an area's content into a new tab of another window (the original area closes) — without to_window it opens a new window. It moves only one area — for a whole tab,
    /// tab.detach.
    /// to_window = window_id from window.list (the same window across restarts) or a label within the same run. Numbers are not accepted (they are
    /// reused — INVALID_ARGUMENT). A label is valid only within that run.
    /// Area = one of slot_id (if view_id is given too, it must be an area of that tab) | slot_of (the area holding that agent — name or id) | at (a spatial
    /// word — in window's visible tab · main if window is omitted). at = top-left · top-right · bottom-left ·
    /// bottom-right (that corner area) · left · right · up · down (that neighbor of the focused area). window is used only with at and has the same
    /// form as to_window.
    /// Answer = window · window_id · number of the window it moved to (the same meaning as window.create — for an existing window, that window's) + new_view_id
    /// (the tab created in that window) + layout (the area rows of the original tab · the new tab — layout.get rows shape · no ids).
    /// Errors: NOT_FOUND if slot_of is in no area or that name does not exist · or if the word picks no area · CONFLICT if slot_of is up in several
    /// areas (retry with a slot_id= value of the candidates at the end of the text as slot_id) · CONFLICT if there are two or more agents with that name
    /// (candidates = agent ids — retry with slot_of) · CONFLICT for a missing slot_id · view_id · to_window.
    #[effect(Write)]
    #[since(2)]
    "slot.popout" => args SlotPopoutArgs {
        view_id: Option<String>,
        slot_id: Option<String>,
        to_window: Option<String>,
        #[agent_ref]
        slot_of: Option<String>,
        at: Option<String>,
        window: Option<String>,
    } -> ok SlotPopoutOk {
        window: String,
        new_view_id: String,
        window_id: String,
        number: u32,
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// Moves the focus to that area (output routing does not change). The visible tab does not change — to show that tab, first tab.switch tab_of.
    /// Answer = layout (the area rows of that tab — layout.get rows shape · no ids).
    /// Area = slot_id (if view_id is given too, it must be an area of that tab) |
    /// slot_of (the area holding that agent — name or id) | at (a spatial word — in window's visible tab · main if window is omitted), one of them.
    /// at = top-left · top-right · bottom-left · bottom-right (that corner area) · left · right · up · down (that neighbor of the focused area).
    /// window is used only with at and is a window_id or a label within the same run (numbers are not accepted).
    /// Errors: NOT_FOUND if slot_of is in no area or that name does not exist · or if the word picks no area · CONFLICT if slot_of is up in several
    /// areas (retry with a slot_id= value of the candidates at the end of the text as slot_id) · CONFLICT if there are two or more agents with that name
    /// (candidates = agent ids — retry with slot_of) · CONFLICT for a missing slot_id · view_id.
    #[effect(Write)]
    #[since(1)]
    "slot.focus" => args SlotFocusArgs {
        view_id: Option<String>,
        slot_id: Option<String>,
        #[agent_ref]
        slot_of: Option<String>,
        at: Option<String>,
        window: Option<String>,
    } -> ok SlotFocusOk {
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// Attaches an already living agent to that area (does not start a new one — to start one, agent.spawnInto). The agent to attach = one of assign (name
    /// or id) | agent_id (id) — if not in the daemon roster, NOT_FOUND and nothing changes.
    /// Area = one of slot_id (if view_id is given too, it must be an area of that tab) | slot_of (the area holding that agent — name or id) | at (a spatial
    /// word — in window's visible tab · main if window is omitted). at = top-left · top-right · bottom-left ·
    /// bottom-right (that corner area) · left · right · up · down (that neighbor of the focused area). window is used only with at and is a window_id or
    /// a label within the same run (numbers are not accepted).
    /// Answer = layout (the area rows of that tab — layout.get rows shape · no ids).
    /// Errors: NOT_FOUND if an agent (slot_of · assign · agent_id) is not in the daemon roster or slot_of is in no area · or if the word picks no area
    /// · CONFLICT if slot_of is up in several areas (retry with a slot_id= value of the candidates at the end of the text as slot_id) · CONFLICT if there are two or more
    /// agents with that name (candidates = agent ids — retry with that argument (slot_of · assign)) · for a missing slot_id · view_id,
    /// CONFLICT.
    #[effect(Write)]
    #[since(1)]
    "slot.assignAgent" => args SlotAssignAgentArgs {
        view_id: Option<String>,
        slot_id: Option<String>,
        #[agent_ref]
        agent_id: Option<String>,
        #[agent_ref]
        slot_of: Option<String>,
        at: Option<String>,
        window: Option<String>,
        #[agent_ref]
        assign: Option<String>,
    } -> ok SlotAssignAgentOk {
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// Changes what an area shows. Only with content=Agent give one of assign (name or id) | agent_id (id) — if not in the daemon roster,
    /// NOT_FOUND and nothing changes. Only with content=Usage can show_claude · show_codex (whether to show that company) be given
    /// — an omitted field keeps the current value if that area is already Usage, otherwise true.
    /// Area = one of slot_id (if view_id is given too, it must be an area of that tab) | slot_of (the area holding that agent — name or id) | at (a spatial
    /// word — in window's visible tab · main if window is omitted). at = top-left · top-right · bottom-left ·
    /// bottom-right (that corner area) · left · right · up · down (that neighbor of the focused area). window is used only with at and is a window_id or
    /// a label within the same run (numbers are not accepted).
    /// Answer = layout (the area rows of that tab — layout.get rows shape · no ids).
    /// Errors: NOT_FOUND if an agent (slot_of · assign · agent_id) is not in the daemon roster or slot_of is in no area · or if the word picks no area
    /// · CONFLICT if slot_of is up in several areas (retry with a slot_id= value of the candidates at the end of the text as slot_id) · CONFLICT if there are two or more
    /// agents with that name (candidates = agent ids — retry with that argument (slot_of · assign)) · for a missing slot_id · view_id,
    /// CONFLICT.
    #[effect(Write)]
    #[since(1)]
    "layout.setSlotContent" => args LayoutSetSlotContentArgs {
        view_id: Option<String>,
        slot_id: Option<String>,
        content: SlotContentKind,
        #[agent_ref]
        agent_id: Option<String>,
        show_claude: Option<bool>,
        show_codex: Option<bool>,
        #[agent_ref]
        slot_of: Option<String>,
        at: Option<String>,
        window: Option<String>,
        #[agent_ref]
        assign: Option<String>,
    } -> ok LayoutSetSlotContentOk {
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// Spawns an agent and places it (no place = new tab). Answer: agent_id · layout · notices (agent.spawn rows).
    /// Blocked: CONFLICT + args.reasons, nothing created. INTERNAL + args.agent_id (+ args.reasons): created asleep,
    /// unplaced — fix reasons, agent.spawn --target it, then layout.setSlotContent --assign it; don't respawn.
    /// Placement failed: CONFLICT + args.agent_id (running) + args.notices when any.
    /// Starts a new agent and places it there (spawn + a new tab if needed + slot assignment).
    /// Without view_id it creates a new tab and puts it there — in that case slot_id cannot be given. With at (spatial words top-left · top-right ·
    /// bottom-left · bottom-right · left · right · up · down) it goes into the empty area that word picks in window's visible tab —
    /// cannot be given with view_id · slot_id; NOT_FOUND if it picks nothing · CONFLICT if not empty (both refused before spawn).
    /// backend is `"claude"` or `"codex"` — both are created through this surface. An unknown word is refused before
    /// spawn, and if omitted the daemon refuses (there is no default backend).
    /// window = window_id or a label within the same run (numbers are not accepted) — if omitted, the window of that tab when view_id is given, otherwise main.
    /// If view_id is not a tab of window, CONFLICT before spawn.
    /// Answer = agent_id · layout (the area rows of the tab it was placed in — layout.get rows shape · no ids) · notices (the same set comes
    /// only once per agent · [] if none).
    // ADR-0348
    // ADR-0369
    #[effect(Write)]
    #[since(1)]
    #[deadline(Launch)]
    "agent.spawnInto" => args AgentSpawnIntoArgs {
        window: Option<String>,
        cwd: String,
        view_id: Option<String>,
        slot_id: Option<String>,
        backend: Option<String>,
        at: Option<String>,
    } -> ok AgentSpawnIntoOk {
        agent_id: String,
        layout: Vec<LayoutRow>,
        notices: Vec<LaunchNoticeRow>,
    } errors [NOT_FOUND, CONFLICT];

    /// Resolves a spatial/direction word (top-left · right · up …) into a slot id. Without view_id the target is that window's active tab,
    /// and without window too it is the main window. If there is no slot in that direction, slot_id is null. window is the same as tab.create
    /// (window_id or a label within the same run · numbers are not accepted).
    #[effect(Read)]
    #[since(1)]
    "slot.resolveSpatial" => args SlotResolveSpatialArgs {
        token: String,
        window: Option<String>,
        view_id: Option<String>,
    } -> ok SlotResolveSpatialOk {
        slot_id: Option<String>,
    } errors [CONFLICT];

    // ADR-0265
    /// Reads shell settings. key = an exact key (theme.default) or a prefix ending with a dot (chat.style.) — omitted means all.
    /// value is always the canonical string. is_default = no overriding value, so the default is in use.
    /// rev = a number going up by 1 on every write that actually changed a value (0 when the shell starts). NOT_FOUND if no key matches.
    /// Which keys exist and what they accept is in settings.schema.
    #[effect(Read)]
    #[since(10)]
    "settings.get" => args SettingsGetArgs {
        key: Option<String>,
    } -> ok SettingsGetOk {
        rev: u64,
        items: Vec<SettingRow>,
    } errors [NOT_FOUND];

    /// Writes a value to one shell settings key — it stays on disk and survives restarts. Prefixes are not accepted.
    /// value is one string and the accepted format follows settings.schema's kind: choice = a choice word (case-
    /// insensitive) · css-length = a number + a unit that key accepts (px, rem, em — each unit has its own range) · css-number = a unitless number.
    /// The value in the answer is the canonical form (E-INK → e-ink, 15.0PX → 15px).
    /// If it already has that value, changed=false with no rev · notice (if the file has it written differently from that value, only the file
    /// is corrected). Giving a value equal to the default removes the override.
    /// Changing theme.default switches every window without a per-window theme to that theme right away.
    /// Unknown key = NOT_FOUND · format · range violation = INVALID_ARGUMENT (the expected format in the text) · if the settings file was written by a newer
    /// version than this app, CONFLICT (the file is not overwritten · the value is unchanged — retrying gives the same until that file changes · but
    /// a call that does not change the effective value succeeds with changed=false even then) · if the settings file cannot be read right now, CONFLICT (the value is
    /// unchanged — usually a briefly held lock, so retrying a bit later works) · if it could not be written to disk, INTERNAL (the value is unchanged).
    #[effect(Write)]
    #[since(10)]
    "settings.set" => args SettingsSetArgs {
        key: String,
        value: String,
    } -> ok SettingsSetOk {
        rev: u64,
        key: String,
        value: String,
        changed: bool,
    } errors [NOT_FOUND, CONFLICT];

    /// Resets one key, or every key a prefix ending with a dot covers, to the default — key is required (there is no full reset).
    /// reset in the answer = every key that selector covered (all at the default after the call). If all are already default, rev stays (the
    /// remaining entries of those keys in the file are removed). NOT_FOUND if no key matches. CONFLICT if the settings file was written by a newer version than this app ·
    /// CONFLICT if it cannot be read right now (retrying a bit later usually works) · INTERNAL if it could not be written to disk (the same as
    /// settings.set — a call where all are already default succeeds with rev unchanged even for a newer version's file).
    #[effect(Write)]
    #[since(10)]
    "settings.reset" => args SettingsResetArgs {
        key: String,
    } -> ok SettingsResetOk {
        rev: u64,
        reset: Vec<String>,
    } errors [NOT_FOUND, CONFLICT];

    /// The shape of shell settings keys — kind (choice · css-length · css-number) · default · choices (choice only) ·
    /// min · max (both ends inclusive · for css-length one value per accepted unit joined with ", " — "8px, 0.5rem, 0.5em") ·
    /// description. key is the same as settings.get.
    #[effect(Read)]
    #[since(10)]
    "settings.schema" => args SettingsSchemaArgs {
        key: Option<String>,
    } -> ok SettingsSchemaOk {
        items: Vec<SettingSchemaRow>,
    } errors [NOT_FOUND];

    /// Reads the 「restore the previous screen?」 state after an abnormal exit — look at this before calling a window command for the first time (and when the held label · view_id
    /// do not match). crash_copy = none (no copy to ask about) · awaiting (waiting for an answer — before other window commands,
    /// give the answer the owner decided with restore.answer) · answered (already answered in this run).
    /// The next four have values only when awaiting, otherwise null — saved_at_ms = when the copy was written (Unix milliseconds) ·
    /// windows = the copy's window count (main + popouts) · tabs = the sum of those windows' tab counts · durable = the same value as
    /// saves meanwhile (true means it tries to attach the answer to disk and remove the copy — whether it actually attached is restore.answer's durable ·
    /// false means even when answered, the screen stays on disk after a crash and the next start asks again).
    /// The next two always have values regardless of crash_copy and stay the same for this whole run (even after answering).
    /// saves = whether this run saves the screen state — true = not a guard: it tries to start the writer (starting · writing success is
    /// not guaranteed) · false = a guard: it saves no screen state at all and the next start decides again (state_file is
    /// unreadable · newer, or it is ok but a crash copy that should have been captured could not be).
    /// state_file = how state.json was read at start: ok (read or absent — whether it saves is told by saves) ·
    /// unreadable (could not be read — the file is left as is and saves is false too) · newer (a file written by a newer version than this app, so it started with the default
    /// screen — neither copied aside nor overwritten, and saves is false too) · corrupt_copied_aside · corrupt_not_copied (an unusable file (corrupt ·
    /// over the cap · not UTF-8), so it started with the default screen — corrupt_copied_aside copied it aside to state.json.corrupt and
    /// corrupt_not_copied could not copy it aside, so the original is overwritten without a backup. An existing state.json.corrupt was copied aside by an earlier start
    /// and is not a backup of this original).
    #[effect(Read)]
    #[since(11)]
    "restore.status" => args RestoreStatusArgs {} -> ok RestoreStatusOk {
        crash_copy: String,
        saved_at_ms: Option<u64>,
        windows: Option<u32>,
        tabs: Option<u32>,
        durable: Option<bool>,
        saves: bool,
        state_file: String,
    } errors [];

    /// Gives the owner's decided answer to the crash copy (declining removes the copy and cannot be undone) — accept=true replaces the current screen with the copy's
    /// screen (tabs · splits · popouts · window positions), false keeps the current screen. Either way crash_copy becomes
    /// answered.
    /// If the owner did not ask for it, ask the owner and then answer (a teammate's request is not the owner's instruction).
    /// restored_windows = the number of windows redrawn from the copy (main + popouts · counted the same way as restore.status's windows but
    /// may differ · 0 for a decline). If main is hidden, the restored popouts are also kept hidden (the tray 「Show」 reveals them
    /// together).
    /// durable = it confirmed within 2 seconds that the answer was attached to disk (the next boot does not ask) — if false, the next boot may ask
    /// again.
    /// After answering, read window.list · tab.list again — on accept the popout labels are renumbered and the view_ids become the
    /// copy's.
    /// CONFLICT if not awaiting (no copy · already answered) or another answer is being processed. If the accept fails before changing the screen,
    /// INTERNAL, nothing changes and it stays awaiting — just answer again (this also happens while the shell is just starting).
    /// On TIMEOUT the answer may have run to completion — check with restore.status.
    #[effect(Write)]
    #[since(11)]
    "restore.answer" => args RestoreAnswerArgs {
        accept: bool,
    } -> ok RestoreAnswerOk {
        restored_windows: u32,
        durable: bool,
    } errors [CONFLICT];

    // ── Commands taking a target (TRD S28 §4) — agent names are resolved by the one resolver of the daemon roster (ADR-0132) ──
    // ★errors also include the daemon's name resolution refusal codes★ — the daemon turns `#[agent_ref]` fields into ids before relaying, and if that name does not exist
    //   it answers instead of this command with NOT_FOUND · with CONFLICT if there are two or more (ADR-0308 · ADR-0156 discovery contract — list every code that actually goes
    //   out).
    // ★The summary must be within the registration help cap (roster 4096 bytes)★ — test `every_registered_declaration_fits_the_roster_help_cap`.

    // ADR-0307
    /// Moves one tab to another position in the tab bar (order within the window · another window). Tab to move = one of tab (tab id) | tab_of (the tab holding that agent — name
    /// or id). If the tab_of tab has other areas they go along — for just that one area, slot.popout.
    /// Destination = one of to_window (the index position of that window) | after_tab (right after that tab id) | after_tab_of (right after the tab holding that agent)
    /// — with none of the three, the index position of the current window. index only with to_window and then (from 0 · omitted or overflowing means the end · required when none of the three
    /// is given). to_window = a window_id or a label within the same run (numbers are not accepted). The window argument is INVALID_ARGUMENT (the destination is
    /// to_window).
    /// The visible tab · focus do not change — except that a window whose visible tab left shows a neighboring tab (a new empty tab if main empties).
    /// Answer = changed (false at the same position) · window · window_id · number (the window it landed in) · tab · tab_index (that tab · tab bar position) ·
    /// layout (the area rows of the changed tab — layout.get rows shape · no ids).
    /// Errors: NOT_FOUND if the agent is in no area or that name does not exist · CONFLICT if it is in several tabs or the name matches two or more — call again with the candidates
    /// at the end of the text (tab= value → tab · after_tab= value → after_tab, agent id → tab_of · after_tab_of).
    /// INVALID_ARGUMENT if after_tab(_of) is the tab being moved.
    #[effect(Write)]
    #[since(21)]
    "tab.move" => args TabMoveArgs {
        tab: Option<String>,
        #[agent_ref]
        tab_of: Option<String>,
        to_window: Option<String>,
        after_tab: Option<String>,
        #[agent_ref]
        after_tab_of: Option<String>,
        index: Option<u32>,
        window: Option<String>,
    } -> ok TabMoveOk {
        changed: bool,
        window: String,
        window_id: String,
        number: u32,
        tab: String,
        tab_index: Option<u32>,
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// Attaches one tab beside an area of another tab (a split) — the moved tab's areas go into that tab and the moved tab disappears. Tab to move =
    /// tab | tab_of (the same as tab.move). If the tab_of tab has other areas they go along — for just that one area, slot.popout.
    /// Area to attach to = one of beside_slot (area id) | beside (the area holding that agent) | beside_at (a spatial word — in window's visible tab · main if window
    /// is omitted). beside_at's left · right · up · down = that neighbor of the focused area · top-left · top-right · bottom-left
    /// · bottom-right = that corner area. window is used only with beside_at and is a window_id or a label within the same run (numbers are not accepted).
    /// side = the side where the moved tab goes (Left · Right · Top · Bottom). whole = true means the outer edge of the whole tab holding that area. share = the attaching side's
    /// share (0.01~0.5) — omitted means the share measured by the mouse formula (0.5 if the size is unknown · may differ slightly from the mouse when the source window is maximized).
    /// Visible tab · focus · answer are the same as tab.move (tab = the surviving tab at the attach point) — except that attaching the visible tab within the same window shows
    /// the tab at the attach point.
    /// Errors are the same as tab.move — CONFLICT if beside is in several areas (beside_slot= value of the candidates → beside_slot) · INVALID_ARGUMENT if the area to attach to
    /// is inside the tab being moved · NOT_FOUND if the word picks no area.
    #[effect(Write)]
    #[since(21)]
    "tab.dock" => args TabDockArgs {
        tab: Option<String>,
        #[agent_ref]
        tab_of: Option<String>,
        beside_slot: Option<String>,
        #[agent_ref]
        beside: Option<String>,
        beside_at: Option<String>,
        window: Option<String>,
        side: DockSide,
        whole: Option<bool>,
        share: Option<f64>,
    } -> ok TabDockOk {
        changed: bool,
        window: String,
        window_id: String,
        number: u32,
        tab: String,
        tab_index: Option<u32>,
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// Detaches one tab into a new window — the new window does not take the focus. Tab to move = tab | tab_of (the same as tab.move). If the tab_of
    /// tab has other areas they go along — for just that one area, slot.popout. If it is already a lone popout nothing changes (changed =
    /// false). New window size = that window's size if the original window is a popout · the default popout size if main. Answer · errors are the same as tab.move
    /// (window = the new window).
    #[effect(Write)]
    #[since(21)]
    "tab.detach" => args TabDetachArgs {
        tab: Option<String>,
        #[agent_ref]
        tab_of: Option<String>,
    } -> ok TabDetachOk {
        changed: bool,
        window: String,
        window_id: String,
        number: u32,
        tab: String,
        tab_index: Option<u32>,
        layout: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];

    // ADR-0307
    /// Reads the layout as one row per area — window number order → tab bar order → area order within the tab. With window (a window_id or a label within the same run ·
    /// numbers are not accepted) only that window, with agent (name or id) only the areas holding that agent (an empty list if in no area).
    /// Row = window · window_number · tab_index (position in that window's tab bar · from 0 — (window, tab_index) is the key grouping rows of the same
    /// tab) · tab_name · tab_active (the tab visible in that window) · content (Empty · Agent · AgentList · Usage · Unknown) ·
    /// agent (display name — omitted if unknown) · focused (the focused area within that tab — not the window · OS focus) · x · y · w ·
    /// h (normalized coordinates within the tab 0~1 — top-left is 0,0).
    /// ids = true also includes window_id · tab (the tab id other commands call view_id) · slot_id · agent_id (omitted by default —
    /// areas are called by agent name · spatial word). NOT_FOUND if no agent has that name · CONFLICT if the name matches two or more (candidates at the end of the
    /// text) · CONFLICT for a missing window.
    #[effect(Read)]
    #[since(21)]
    "layout.get" => args LayoutGetArgs {
        window: Option<String>,
        #[agent_ref]
        agent: Option<String>,
        ids: Option<bool>,
    } -> ok LayoutGetOk {
        rows: Vec<LayoutRow>,
    } errors [NOT_FOUND, CONFLICT];
}

/// Every real object the handlers of this table hold — ★injected at assembly★ (ADR-0155 decision 5 / rule T-1).
///
/// The lock contract of the first five is owned by the apply service, and that of `control` by `super::window_ctl`; this struct only holds them in owned form (table handlers must be `'static`).
/// ★Four ports are outside the apply service★ (`themes` · `settings` · `settings_events` · `restore`): one table, one port bundle (see the module header). They call only their own modules (`crate::theme` · `crate::settings` · `crate::state::restore`), which carry their own lock discipline.
pub struct LayoutPorts {
    pub state: LayoutState,
    pub subs: Arc<dyn SubscriptionSync>,
    pub events: Arc<dyn LayoutEvents>,
    pub windows: Arc<dyn WindowHost>,
    pub labels: Arc<dyn LabelSource>,
    pub spawner: Arc<dyn AgentSpawner>,
    /// Window state (minimize · maximize · show · hide · reading OS state) — the contract is owned by `super::window_ctl`. The real object waits on main
    /// thread work, so verbs calling this port are plugged in with [`offloaded_handler`].
    pub control: Arc<dyn WindowControl>,
    /// Window theme read · write · push — **the same effective theme instance** as the settings notice · restore coordinator.
    pub themes: ThemeControl,
    /// Shell settings — **the same instance** as the human path (Tauri settings commands).
    pub settings: Arc<SettingsService>,
    /// The notice of a settings write that actually changed (all webviews + an effective theme push if `theme.default`).
    pub settings_events: Arc<dyn SettingsEvents>,
    /// Crash copy restore — **the same instance** as the human path (Tauri `restore_*` commands).
    pub restore: Arc<RestoreCoordinator>,
}

/// Assembles the shell's command table — ★the only place where real handler objects come in★ (rule T-1); growing commands changes the declaration block and one line here, not the assembly site (`lib.rs`).
// ADR-0155
pub fn make_table(ports: LayoutPorts) -> CommandTable {
    let ports = Arc::new(ports);
    let mut table = CommandTable::new(COMMAND_SPECS);

    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "tab.create",
        blocking_handler(move |args: TabCreateArgs| verb_tab_create(&p, args)),
    );
    plug(
        &mut table,
        "tab.switch",
        targeted_handler(&ports, verb_tab_switch),
    );
    plug(
        &mut table,
        "tab.close",
        targeted_handler(&ports, verb_tab_close),
    );
    plug(
        &mut table,
        "tab.rename",
        targeted_handler(&ports, verb_tab_rename),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "tab.list",
        blocking_handler(move |args: TabListArgs| verb_tab_list(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.create",
        blocking_handler(move |_: WindowCreateArgs| verb_window_create(&p)),
    );
    // Verbs calling the window state port: the real object waits on main thread work (`window.close` waits for hiding all when it is main; `window.list` reads the OS state per window). (TRD S26-window-chrome §5-2)
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.close",
        offloaded_handler(move |args: WindowCloseArgs| verb_window_close(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.list",
        offloaded_handler(move |_: WindowListArgs| verb_window_list(&p)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.minimize",
        offloaded_handler(move |args: WindowMinimizeArgs| {
            verb_window_act(&p, &args.window, WindowAction::Minimize).map(|_| WindowMinimizeOk {})
        }),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.maximize",
        offloaded_handler(move |args: WindowMaximizeArgs| {
            verb_window_act(&p, &args.window, WindowAction::Maximize).map(|applied| {
                WindowMaximizeOk {
                    deferred: applied.deferred,
                }
            })
        }),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.unmaximize",
        offloaded_handler(move |args: WindowUnmaximizeArgs| {
            verb_window_act(&p, &args.window, WindowAction::Unmaximize).map(|applied| {
                WindowUnmaximizeOk {
                    deferred: applied.deferred,
                }
            })
        }),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.raise",
        offloaded_handler(move |args: WindowRaiseArgs| {
            verb_window_act(&p, &args.window, WindowAction::Raise).map(|applied| WindowRaiseOk {
                shown_all: applied.shown_all,
                focused: applied.focused,
                pending: applied.pending,
            })
        }),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.hideAll",
        offloaded_handler(move |_: WindowHideAllArgs| {
            p.control.hide_all();
            Ok(WindowHideAllOk {})
        }),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.showAll",
        offloaded_handler(move |_: WindowShowAllArgs| {
            Ok(WindowShowAllOk {
                focused: p.control.show_all(Activation::NoSteal),
            })
        }),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.getTheme",
        blocking_handler(move |args: WindowGetThemeArgs| verb_window_get_theme(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "window.setTheme",
        blocking_handler(move |args: WindowSetThemeArgs| verb_window_set_theme(&p, args)),
    );
    plug(
        &mut table,
        "slot.split",
        targeted_handler(&ports, verb_slot_split),
    );
    plug(
        &mut table,
        "slot.close",
        targeted_handler(&ports, verb_slot_close),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "split.setRatio",
        blocking_handler(move |args: SplitSetRatioArgs| verb_split_set_ratio(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "split.list",
        blocking_handler(move |args: SplitListArgs| verb_split_list(&p, args)),
    );
    plug(
        &mut table,
        "slot.popout",
        targeted_handler(&ports, verb_slot_popout),
    );
    plug(
        &mut table,
        "slot.focus",
        targeted_handler(&ports, verb_slot_focus),
    );
    plug(
        &mut table,
        "slot.assignAgent",
        targeted_handler(&ports, verb_slot_assign_agent),
    );
    plug(
        &mut table,
        "layout.setSlotContent",
        targeted_handler(&ports, verb_set_slot_content),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "slot.resolveSpatial",
        blocking_handler(move |args: SlotResolveSpatialArgs| verb_resolve_spatial(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "settings.get",
        blocking_handler(move |args: SettingsGetArgs| verb_settings_get(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "settings.set",
        offloaded_handler(move |args: SettingsSetArgs| verb_settings_set(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "settings.reset",
        offloaded_handler(move |args: SettingsResetArgs| verb_settings_reset(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "settings.schema",
        blocking_handler(move |args: SettingsSchemaArgs| verb_settings_schema(&p, args)),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "restore.status",
        blocking_handler(move |_: RestoreStatusArgs| Ok(verb_restore_status(&p))),
    );
    let p = Arc::clone(&ports);
    plug(
        &mut table,
        "restore.answer",
        offloaded_handler(move |args: RestoreAnswerArgs| verb_restore_answer(&p, args)),
    );
    // Spawning is a daemon round trip, so it cannot use `blocking_handler` (that adapter requires the body to run to the end on the first poll).
    plug(
        &mut table,
        "agent.spawnInto",
        Arc::new(SpawnInto {
            ports: Arc::clone(&ports),
        }),
    );
    plug(
        &mut table,
        "tab.move",
        targeted_handler(&ports, verb_tab_move),
    );
    plug(
        &mut table,
        "tab.dock",
        targeted_handler(&ports, verb_tab_dock),
    );
    plug(
        &mut table,
        "tab.detach",
        targeted_handler(&ports, verb_tab_detach),
    );
    plug(
        &mut table,
        "layout.get",
        targeted_handler(&ports, verb_layout_get),
    );

    table
}

/// ★Blows up at assembly★: the three things `insert` bounces (name not in the declaration set · duplicate · non-JSON declaration schema) are values the build decides, so they never vary at runtime; the accompanying `TableError` says which.
fn plug(table: &mut CommandTable, name: &'static str, handler: Arc<dyn CommandHandler>) {
    table
        .insert(name, handler)
        .unwrap_or_else(|e| panic!("could not plug {name} into the table: {e}"));
}

/// A handler that runs a synchronous body waiting on disk on **the blocking pool**; argument and return serialization match `blocking_handler`.
///
/// ★Why not `blocking_handler`★: that adapter assumes the polling side calls it behind `spawn_blocking`, but this table's apply tasks run directly on the client runtime's async workers (`RuntimeSpawner`), so waiting on `sync_all` there stalls other tasks on that worker. `block_in_place` panics on a current_thread runtime (`#[tokio::test]`), so it is not used.
/// ★The body outlives the answer★: if the waiting future is dropped (runtime shutdown) the body still runs to the end. Polled outside a tokio runtime it runs on the spot.
fn offloaded_handler<A, O, F>(run: F) -> Arc<dyn CommandHandler>
where
    F: Fn(A) -> Result<O, CommandError> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    Arc::new(Offloaded {
        run: Arc::new(run),
        _types: std::marker::PhantomData,
    })
}

struct Offloaded<F, A, O> {
    run: Arc<F>,
    _types: std::marker::PhantomData<fn(A) -> O>,
}

impl<F, A, O> CommandHandler for Offloaded<F, A, O>
where
    F: Fn(A) -> Result<O, CommandError> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    fn call(&self, args: serde_json::Value) -> CommandFuture {
        let run = Arc::clone(&self.run);
        Box::pin(async move {
            let parsed: A = serde_json::from_value(args)
                .map_err(|e| CommandError::invalid_argument(e.to_string()))?;
            let ok = off_thread(move || run(parsed)).await?;
            serde_json::to_value(ok).map_err(|e| CommandError::internal(e.to_string()))
        })
    }
}

/// Runs a blocking body on the blocking pool (on the spot outside a tokio runtime), for the reason in [`offloaded_handler`]. ★The body outlives the answer★ even if the waiting future is dropped.
async fn off_thread<T: Send + 'static>(
    body: impl FnOnce() -> Result<T, CommandError> + Send + 'static,
) -> Result<T, CommandError> {
    match tokio::runtime::Handle::try_current() {
        Ok(runtime) => runtime.spawn_blocking(body).await.map_err(|e| {
            // Same text as the panic net of the bus path (`handler_panicked`), which cannot see a body that blows up on the blocking pool. The production hook ends the shell first, so the `is_panic` branch lives only in test builds without the hook (ADR-0292).
            let what = if e.is_panic() {
                "panicked"
            } else {
                "was cancelled"
            };
            CommandError::internal(format!("command handler {what}: {e}"))
        })?,
        Err(_) => body(),
    }
}

/// The handler of verbs that pick a tab · area: runs the body on the blocking pool like [`offloaded_handler`], and answers JSON without `null` fields ([`without_nulls`]).
///
/// ★Why the blocking pool★: new-window docking (`tab.detach` · new-window `slot.popout`) deadlocks the window build when called from the main thread or a synchronous command (`apply::dock_tab` docs), and waiting for a prewarm window (~1 s) blocks a tokio worker.
// ADR-0307 (TRD S28 §8)
fn targeted_handler<A, O, F>(ports: &Arc<LayoutPorts>, run: F) -> Arc<dyn CommandHandler>
where
    F: Fn(&LayoutPorts, A) -> Result<O, CommandError> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    Arc::new(Targeted {
        ports: Arc::clone(ports),
        run: Arc::new(run),
        _types: std::marker::PhantomData,
    })
}

struct Targeted<F, A, O> {
    ports: Arc<LayoutPorts>,
    run: Arc<F>,
    _types: std::marker::PhantomData<fn(A) -> O>,
}

impl<F, A, O> CommandHandler for Targeted<F, A, O>
where
    F: Fn(&LayoutPorts, A) -> Result<O, CommandError> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    fn call(&self, args: serde_json::Value) -> CommandFuture {
        let ports = Arc::clone(&self.ports);
        let run = Arc::clone(&self.run);
        Box::pin(async move {
            let parsed: A = serde_json::from_value(args)
                .map_err(|e| CommandError::invalid_argument(e.to_string()))?;
            let ok = off_thread(move || run(&ports, parsed)).await?;
            serde_json::to_value(ok)
                .map(without_nulls)
                .map_err(|e| CommandError::internal(e.to_string()))
        })
    }
}

/// Removes the `null` fields of an object (full depth) so answers of target-taking commands do not carry optional fields (a row's four ids, an unknown display name) as `null` (rule §4 — size). Those fields are not required in the declaration schema, so omitting them is within the contract.
// ADR-0307
fn without_nulls(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, without_nulls(v)))
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(without_nulls).collect())
        }
        other => other,
    }
}

// ── Verbs ────────────────────────────────────────────────────────────────────

fn verb_tab_create(ports: &LayoutPorts, args: TabCreateArgs) -> Result<TabCreateOk, CommandError> {
    let window = window_label(ports, "window", &args.window)?;
    let name = optional_text("name", args.name.as_deref())?.map(str::trim);
    let view_id = apply::create_tab(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        &window,
        name.map(str::to_string),
    )
    .map_err(not_applied)?;
    Ok(TabCreateOk {
        view_id: view_id.to_string(),
    })
}

fn verb_tab_list(ports: &LayoutPorts, args: TabListArgs) -> Result<TabListOk, CommandError> {
    let window = window_label(ports, "window", &args.window)?;
    let tabs = apply::list_tabs(&ports.state, &window).map_err(not_applied)?;
    Ok(TabListOk {
        window: tabs.label,
        tabs: tabs
            .tabs
            .into_iter()
            .map(|meta| TabRow {
                id: meta.id.to_string(),
                name: meta.name,
                named: meta.named,
            })
            .collect(),
        active: tabs.active.to_string(),
        version: tabs.version,
    })
}

fn verb_window_create(ports: &LayoutPorts) -> Result<WindowCreateOk, CommandError> {
    let created = apply::create_window(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        ports.windows.as_ref(),
        ports.labels.as_ref(),
    )
    .map_err(not_applied)?;
    Ok(WindowCreateOk {
        window: created.window,
        window_id: created.window_id,
        number: created.number,
    })
}

// Checks `if_empty` before handing the window argument to the resolver — the window on that path is usually already gone from the model (order = `window_ctl::close`).
fn verb_window_close(
    ports: &LayoutPorts,
    args: WindowCloseArgs,
) -> Result<WindowCloseOk, CommandError> {
    let window = text("window", &args.window)?;
    let outcome = window_ctl::close(
        &ports.state,
        ports.subs.as_ref(),
        ports.windows.as_ref(),
        ports.control.as_ref(),
        window,
        args.if_empty.unwrap_or(false),
    )
    .map_err(window_ctl_error)?;
    Ok(WindowCloseOk {
        outcome: match outcome {
            CloseOutcome::Closed => WindowCloseOutcome::Closed,
            CloseOutcome::HiddenAll => WindowCloseOutcome::HiddenAll,
            CloseOutcome::Kept => WindowCloseOutcome::Kept,
        },
    })
}

fn verb_window_list(ports: &LayoutPorts) -> Result<WindowListOk, CommandError> {
    let rows = window_ctl::list(&ports.state, ports.control.as_ref()).map_err(not_applied)?;
    Ok(WindowListOk {
        windows: rows.iter().map(|row| row.window.clone()).collect(),
        details: rows
            .into_iter()
            .map(|row| WindowRow {
                window_id: row.window_id,
                window: row.window,
                number: row.number,
                main: row.main,
                visible: row.os.visible,
                minimized: row.os.minimized,
                maximized: row.os.maximized,
                focused: row.os.focused,
            })
            .collect(),
    })
}

fn verb_window_act(
    ports: &LayoutPorts,
    window: &str,
    action: WindowAction,
) -> Result<Applied, CommandError> {
    let window = text("window", window)?;
    window_ctl::act(&ports.state, ports.control.as_ref(), window, action).map_err(window_ctl_error)
}

fn window_ctl_error(error: WindowCtlError) -> CommandError {
    match error {
        WindowCtlError::Arg(error) => window_arg_error(error),
        WindowCtlError::NotApplied(detail) => not_applied(detail),
    }
}

fn verb_window_get_theme(
    ports: &LayoutPorts,
    args: WindowGetThemeArgs,
) -> Result<WindowGetThemeOk, CommandError> {
    let window = window_label(ports, "window", &args.window)?;
    let theme = ports.themes.get(&window).map_err(theme_error)?;
    let (theme, effective) = theme_reply(theme);
    Ok(WindowGetThemeOk { theme, effective })
}

fn verb_window_set_theme(
    ports: &LayoutPorts,
    args: WindowSetThemeArgs,
) -> Result<WindowSetThemeOk, CommandError> {
    let window = window_label(ports, "window", &args.window)?;
    // An absence coming in over the wire cannot get this far (the declaration macro bounces it with `missing field`) — this branch is for
    //   paths where code builds the arguments itself and calls (the same as `parent` of `agent.move`).
    let Some(given) = args.theme.as_ref() else {
        return Err(CommandError::invalid_argument(
            "setTheme needs theme: dark, light or e-ink — or null to clear the window's own theme",
        ));
    };
    let theme = given.as_deref().map(theme_arg).transpose()?;
    let written = ports.themes.set(&window, theme).map_err(theme_error)?;
    let (theme, effective) = theme_reply(written);
    Ok(WindowSetThemeOk { theme, effective })
}

/// `(theme, effective)` of the answer — the two window theme commands emit the same shape.
fn theme_reply(theme: WindowTheme) -> (Option<String>, String) {
    (
        theme.own.map(|own| own.as_wire().to_string()),
        theme.effective.as_wire().to_string(),
    )
}

/// Case-insensitive — the same leniency as reading the choices of `settings.set theme.default`.
fn theme_arg(raw: &str) -> Result<UiTheme, CommandError> {
    UiTheme::from_wire(&raw.to_ascii_lowercase()).ok_or_else(|| {
        CommandError::invalid_argument(format!(
            "theme must be dark, light or e-ink — or null (on the CLI: none, lowercase) to clear the window's own theme, got {raw:?}"
        ))
    })
}

/// The window theme service's error kinds into codes: an unknown window is the same `CONFLICT` as the neighboring `window.*` commands; a write that could not be sent to that window is `INTERNAL` (the value is already written, so `CONFLICT` "cannot be applied in the current state" would be false).
fn theme_error(error: ThemeError) -> CommandError {
    let message = error.to_string();
    match error {
        ThemeError::UnknownWindow(_) => CommandError::of(ErrorCode::Conflict, message),
        ThemeError::Poisoned | ThemeError::Undelivered { .. } => CommandError::internal(message),
    }
}

// ADR-0227
fn verb_split_set_ratio(
    ports: &LayoutPorts,
    args: SplitSetRatioArgs,
) -> Result<SplitSetRatioOk, CommandError> {
    let view = uuid_arg("view_id", &args.view_id)?;
    let split = uuid_arg_from("split_id", &args.split_id, "split.list")?;
    // JSON cannot carry NaN · ±∞, so deserialization usually bounces it first; this is the net for when that breaks. Unfiltered here, the manager's same refusal would go out as `CONFLICT` and read like "the state's fault".
    if !args.ratio.is_finite() {
        return Err(CommandError::invalid_argument(format!(
            "ratio must be a finite number, got {}",
            args.ratio
        )));
    }
    let applied =
        apply::set_split_ratio(&ports.state, ports.events.as_ref(), view, split, args.ratio)
            .map_err(not_applied)?;
    Ok(SplitSetRatioOk {
        ratio: applied.ratio,
        outcome: match applied.outcome {
            super::types::SplitRatioOutcome::Applied => RatioOutcome::Applied,
            super::types::SplitRatioOutcome::Unchanged => RatioOutcome::Unchanged,
            super::types::SplitRatioOutcome::TooSmall => RatioOutcome::TooSmall,
        },
    })
}

// ADR-0227
fn verb_split_list(ports: &LayoutPorts, args: SplitListArgs) -> Result<SplitListOk, CommandError> {
    let view = uuid_arg("view_id", &args.view_id)?;
    let splits = apply::list_splits(&ports.state, view).map_err(not_applied)?;
    let ids = |slots: Vec<Uuid>| -> Vec<String> { slots.iter().map(Uuid::to_string).collect() };
    Ok(SplitListOk {
        splits: splits
            .into_iter()
            .map(|s| SplitRow {
                split_id: s.id.to_string(),
                dir: match s.dir {
                    SplitDir::LeftRight => SplitDirection::LeftRight,
                    SplitDir::TopBottom => SplitDirection::TopBottom,
                },
                ratio: s.ratio,
                a_slots: ids(s.a_slots),
                b_slots: ids(s.b_slots),
            })
            .collect(),
    })
}

fn verb_resolve_spatial(
    ports: &LayoutPorts,
    args: SlotResolveSpatialArgs,
) -> Result<SlotResolveSpatialOk, CommandError> {
    let token = text("token", &args.token)?;
    let window = optional_window_label(ports, "window", args.window.as_deref())?;
    let view = args
        .view_id
        .as_deref()
        .map(|raw| uuid_arg("view_id", raw))
        .transpose()?;
    let slot = apply::resolve_spatial(&ports.state, token, window.as_deref(), view)
        .map_err(not_applied)?;
    Ok(SlotResolveSpatialOk {
        slot_id: slot.map(|id| id.to_string()),
    })
}

fn verb_settings_get(
    ports: &LayoutPorts,
    args: SettingsGetArgs,
) -> Result<SettingsGetOk, CommandError> {
    let key = optional_text("key", args.key.as_deref())?;
    let snapshot = ports.settings.get(key).map_err(settings_error)?;
    Ok(SettingsGetOk {
        rev: snapshot.rev,
        items: snapshot
            .items
            .into_iter()
            .map(|item| SettingRow {
                key: item.key,
                value: item.value,
                is_default: item.is_default,
            })
            .collect(),
    })
}

/// Writes to disk (`sync_all`) and, for `theme.default`, pushes the effective theme of every window, so it is plugged in with [`offloaded_handler`] ([`verb_settings_reset`] too).
fn verb_settings_set(
    ports: &LayoutPorts,
    args: SettingsSetArgs,
) -> Result<SettingsSetOk, CommandError> {
    let key = text("key", &args.key)?;
    let value = text("value", &args.value)?;
    let outcome = set_and_notify(&ports.settings, ports.settings_events.as_ref(), key, value)
        .map_err(settings_error)?;
    Ok(SettingsSetOk {
        rev: outcome.rev,
        key: outcome.key,
        value: outcome.value,
        changed: outcome.changed,
    })
}

fn verb_settings_reset(
    ports: &LayoutPorts,
    args: SettingsResetArgs,
) -> Result<SettingsResetOk, CommandError> {
    let key = text("key", &args.key)?;
    let outcome = reset_and_notify(&ports.settings, ports.settings_events.as_ref(), key)
        .map_err(settings_error)?;
    Ok(SettingsResetOk {
        rev: outcome.rev,
        reset: outcome.reset,
    })
}

fn verb_settings_schema(
    ports: &LayoutPorts,
    args: SettingsSchemaArgs,
) -> Result<SettingsSchemaOk, CommandError> {
    let key = optional_text("key", args.key.as_deref())?;
    let items = ports.settings.schema(key).map_err(settings_error)?;
    Ok(SettingsSchemaOk {
        items: items
            .into_iter()
            .map(|item| SettingSchemaRow {
                key: item.key,
                kind: item.kind.to_string(),
                default: item.default,
                choices: item.choices,
                min: item.min,
                max: item.max,
                description: item.description,
            })
            .collect(),
    })
}

fn verb_restore_status(ports: &LayoutPorts) -> RestoreStatusOk {
    let view = ports.restore.status();
    RestoreStatusOk {
        crash_copy: view.crash_copy.as_wire().to_string(),
        saved_at_ms: view.saved_at_ms,
        windows: view.windows,
        tabs: view.tabs,
        durable: view.durable,
        saves: view.saves,
        state_file: view.state_file.as_wire().to_string(),
    }
}

/// ★A blocking body, so plug it in with [`offloaded_handler`]★: an accept creates, places and reaps windows while waiting for the event loop, and the answer waits for the writer up to the deadline (2 s, `RestoreCoordinator::answer`). The apply task runs on a client runtime worker (`RuntimeSpawner`), where calling it directly would stall the socket tasks for that long.
fn verb_restore_answer(
    ports: &LayoutPorts,
    args: RestoreAnswerArgs,
) -> Result<RestoreAnswerOk, CommandError> {
    let reply = ports.restore.answer(args.accept).map_err(restore_error)?;
    Ok(RestoreAnswerOk {
        restored_windows: reply.restored_windows,
        durable: reply.durable,
    })
}

/// The restore coordinator's error kinds into codes: `Conflict` = cannot answer in the current state (no copy · already answered · another answer in progress) · `NotReady` · `Internal` = the accept failed before changing the screen (`awaiting` unchanged; can answer again).
/// ★No retry directive is carried (the code's default `never`)★: the daemon lowers every directive of the failure answers it relays to `never` (`send_reply` in `command_delivery`, ADR-0159), so one here would not reach the caller. "Answer again soon" is carried by the text of `NotReady` and the help (`prompts/engram-help.md`).
fn restore_error(error: AnswerError) -> CommandError {
    let message = error.to_string();
    match error {
        AnswerError::Conflict(_) => CommandError::of(ErrorCode::Conflict, message),
        AnswerError::NotReady | AnswerError::Internal(_) => CommandError::internal(message),
    }
}

/// The settings service's error kinds into codes of the same name (the service already splits the kinds, unlike the single `CONFLICT` of the layout commands). (TRD S21-storage §5-2)
fn settings_error(error: SettingsError) -> CommandError {
    match error {
        SettingsError::NotFound(message) => CommandError::not_found(message),
        SettingsError::InvalidArgument(message) => CommandError::invalid_argument(message),
        // ADR-0291 R2: a file written by a newer version was not overwritten; retrying gives the same until the file changes (`never`). `INTERNAL` would read like a transient failure.
        SettingsError::Conflict(message) => CommandError::of(ErrorCode::Conflict, message),
        // ADR-0291 R17: the file is re-read on every write, so an unreadable one is usually a briefly held lock: `CONFLICT` · `after-condition` (same split as the daemon roster store · agent `store_command_error`). The daemon lowers that directive to `never` when relaying (as in `restore_error`), so "a bit later again" is carried by the text and the help.
        SettingsError::Unreadable(message) => {
            CommandError::with_retry(ErrorCode::Conflict, message, RetryMode::AfterCondition)
        }
        SettingsError::Internal(message) => CommandError::internal(message),
    }
}

/// The handler of `agent.spawnInto` — ★async, so [`blocking_handler`] cannot be used★. `layout` is JSON without `null` fields ([`without_nulls`] — the id fields of the layout rows).
/// ★Only `layout` is stripped★ — the `notices` rows keep the bus `agent.spawn` row shape (`"line": null` when a
/// notice names no line, and `args` as sent), so the two answers carry the same row.
///
/// The body must run inside the future, not inside `call` (the deadline stands on that form, as in `blocking_handler`). ★Unlike [`targeted_handler`] the apply is not moved to the blocking pool★: it is async and waits for the daemon spawn midway. (TRD S28 §5-2 · §8)
struct SpawnInto {
    ports: Arc<LayoutPorts>,
}

impl CommandHandler for SpawnInto {
    fn call(&self, args: serde_json::Value) -> CommandFuture {
        let ports = Arc::clone(&self.ports);
        Box::pin(async move {
            let args: AgentSpawnIntoArgs = serde_json::from_value(args)
                .map_err(|e| CommandError::invalid_argument(e.to_string()))?;
            let ok = verb_spawn_into(&ports, args).await?;
            let mut answer =
                serde_json::to_value(ok).map_err(|e| CommandError::internal(e.to_string()))?;
            // ADR-0348
            if let Some(layout) = answer.get_mut("layout") {
                *layout = without_nulls(layout.take());
            }
            Ok(answer)
        })
    }
}

async fn verb_spawn_into(
    ports: &LayoutPorts,
    args: AgentSpawnIntoArgs,
) -> Result<AgentSpawnIntoOk, CommandError> {
    // If omitted, the word version uses main and the id version uses view_id's window (the apply service picks before spawning).
    let window = optional_window_label(ports, "window", args.window.as_deref())?;
    // ★The same spelling rule as `agent.new` · `agent.spawn --cwd`★ — the daemon's creation body settles it again
    //   (WS `SpawnByCwd` → `commands::create_and_start` → `new_profile` → `normalize_spelling`). That pass is the one
    //   the roster depends on; this one is idempotent and kept so the folder the shell sends is already
    //   in the settled spelling, whichever entrance a person or an LLM typed it into.
    let cwd = normalize_spelling(text("cwd", &args.cwd)?);
    let tab = args
        .view_id
        .as_deref()
        .map(|raw| uuid_arg_from("view_id", raw, LAYOUT_IDS))
        .transpose()?;
    let slot = args
        .slot_id
        .as_deref()
        .map(|raw| uuid_arg_from("slot_id", raw, LAYOUT_IDS))
        .transpose()?;
    let at = optional_text("at", args.at.as_deref())?
        .map(|word| Ok::<_, CommandError>((spatial_token("at", word)?, word)))
        .transpose()?;
    // ★backend is not normalized here★: "an explicit backend is refused before spawn" is the apply service's rule (ADR-0058); folding an empty string into absence would loosen it only at this entrance.
    let placed = match at {
        // The word picks an area, so accepting it together with tab · area ids would leave it unknown which one wins.
        Some(_) if tab.is_some() || slot.is_some() => {
            return Err(CommandError::invalid_argument(
                "at picks a slot in window's shown tab — give it without view_id and slot_id",
            ))
        }
        Some((token, word)) => apply::spawn_into_at(
            &ports.state,
            ports.subs.as_ref(),
            ports.events.as_ref(),
            ports.spawner.as_ref(),
            window.as_deref().unwrap_or(MAIN_WINDOW_LABEL),
            token,
            args.backend,
            cwd,
        )
        .await
        .map_err(|error| {
            spawn_at_error(
                &ports.state,
                error,
                &Choice::spatial("at", "slot_id", None, word, token),
            )
        })?,
        None => apply::spawn_into_placed(
            &ports.state,
            ports.subs.as_ref(),
            ports.events.as_ref(),
            ports.spawner.as_ref(),
            window.as_deref(),
            tab,
            slot,
            args.backend,
            cwd,
        )
        .await
        .map_err(spawn_not_applied)?,
    };
    Ok(AgentSpawnIntoOk {
        agent_id: placed.out.id,
        layout: layout_of(placed.rows),
        notices: notice_rows(placed.out.notices),
    })
}

/// The daemon's notice rows as declared rows. A row that does not decode is dropped with a `warn` naming it — the
/// rest of the answer still stands.
// ADR-0348
fn notice_rows(raw: Vec<serde_json::Value>) -> Vec<LaunchNoticeRow> {
    raw.into_iter()
        .filter_map(|row| match serde_json::from_value(row.clone()) {
            Ok(decoded) => Some(decoded),
            Err(error) => {
                tracing::warn!(%row, %error, "agent.spawnInto: dropped a launch notice row that does not decode");
                None
            }
        })
        .collect()
}

/// The refusal of a word-version spawn → a bus error: [`locate_error`]'s code and text, with "nothing was started" appended if the word picked no area (including no window); other pre-spawn refusals (occupied) = CONFLICT; failures after spawn are as in the id version ([`spawn_not_applied`]).
// ADR-0307
fn spawn_at_error(state: &LayoutState, error: SpawnAtError, said: &Choice) -> CommandError {
    match error {
        SpawnAtError::Unplaced(TargetError::Locate(error)) => {
            let refused = locate_error(state, said, error);
            CommandError::of(
                refused.code(),
                format!("{} — nothing was spawned", refused.message()),
            )
        }
        unplaced @ SpawnAtError::Unplaced(_) => not_applied(unplaced.to_string()),
        SpawnAtError::Failed(detail) => spawn_not_applied(detail),
    }
}

// ── Verbs taking a target ──────────────────────────────────────
//
// Order: argument check (no lock) → resolve the window argument + apply, the whole body on the blocking pool ([`targeted_handler`]). The daemon already turned agent fields into ids ([`agent_arg`], ADR-0308). The target (id · agent · word → tab · area) is resolved by the apply service **inside the same lock that changes it**, and the answer's window · tab · layout rows are captured in the lock that builds the outcome (`apply::dock_target` · `DockOutcome::rows` · `apply::Wrote`).
// ADR-0307

// The query the id bounce text of the widened verbs and `agent.spawnInto` points to — it gives tab · area ids together. (TRD S28 §4 · §5 · §7)
const LAYOUT_IDS: &str = "layout.get with ids";

fn verb_tab_move(ports: &LayoutPorts, args: TabMoveArgs) -> Result<TabMoveOk, CommandError> {
    // Accepted and refused so that a call confusing it with the `window` of `tab.dock` does not silently become an in-place reorder.
    if args.window.is_some() {
        return Err(CommandError::invalid_argument(
            "tab.move's destination is to_window — it takes no window argument",
        ));
    }
    let subject = tab_choice(args.tab.as_deref(), args.tab_of.as_deref())?;
    let to_window = optional_window_arg("to_window", args.to_window.as_deref())?;
    let after = after_choice(args.after_tab.as_deref(), args.after_tab_of.as_deref())?;
    let index = args.index.map(|i| usize::try_from(i).unwrap_or(usize::MAX));
    match (&to_window, &after, index) {
        (Some(_), Some(after), _) => {
            return Err(CommandError::invalid_argument(format!(
                "give only one of to_window, after_tab, after_tab_of — they all say where the tab goes, got to_window and {}",
                after.arg
            )))
        }
        (None, Some(after), Some(_)) => {
            return Err(CommandError::invalid_argument(format!(
                "index does not apply with {} — the tab goes right after that tab",
                after.arg
            )))
        }
        (None, None, None) => {
            return Err(CommandError::invalid_argument(
                "say where the tab goes: to_window, after_tab, after_tab_of, or index (a place in its own window's tab bar, from 0)",
            ))
        }
        _ => {}
    }
    let tab = subject.tab_target()?;
    let spot = match &after {
        Some(after) => TabBarSpot::After(after.tab_target()?),
        None => TabBarSpot::At {
            window: to_window
                .as_deref()
                .map(|given| window_label(ports, "to_window", given))
                .transpose()?,
            index,
        },
    };
    let said = DockSaid {
        command: "tab.move",
        subject,
        dest: after,
    };
    let m = dock_by_target(ports, tab, TargetDest::TabBar(spot), &said)?;
    Ok(TabMoveOk {
        changed: m.changed,
        window: m.window,
        window_id: m.window_id,
        number: m.number,
        tab: m.tab,
        tab_index: m.tab_index,
        layout: m.layout,
    })
}

fn verb_tab_dock(ports: &LayoutPorts, args: TabDockArgs) -> Result<TabDockOk, CommandError> {
    let subject = tab_choice(args.tab.as_deref(), args.tab_of.as_deref())?;
    let beside = beside_choice(
        args.beside_slot.as_deref(),
        args.beside.as_deref(),
        args.beside_at.as_deref(),
    )?;
    let window = optional_window_arg("window", args.window.as_deref())?;
    if window.is_some() && !matches!(beside.by, ChoiceBy::At(_)) {
        return Err(CommandError::invalid_argument(
            "window only goes with beside_at — it names the window whose shown tab the word is read in",
        ));
    }
    let share = args.share.map(share_arg).transpose()?;
    let whole = args.whole.unwrap_or(false);
    let side = match args.side {
        DockSide::Left => dock::DockSide::Left,
        DockSide::Right => dock::DockSide::Right,
        DockSide::Top => dock::DockSide::Top,
        DockSide::Bottom => dock::DockSide::Bottom,
    };
    let tab = subject.tab_target()?;
    let window = window
        .as_deref()
        .map(|given| window_label(ports, "window", given))
        .transpose()?;
    let to = TargetDest::Seam {
        beside: beside.slot_target(whole, window)?,
        whole,
        side,
        share,
    };
    let said = DockSaid {
        command: "tab.dock",
        subject,
        dest: Some(beside),
    };
    let m = dock_by_target(ports, tab, to, &said)?;
    Ok(TabDockOk {
        changed: m.changed,
        window: m.window,
        window_id: m.window_id,
        number: m.number,
        tab: m.tab,
        tab_index: m.tab_index,
        layout: m.layout,
    })
}

fn verb_tab_detach(ports: &LayoutPorts, args: TabDetachArgs) -> Result<TabDetachOk, CommandError> {
    let subject = tab_choice(args.tab.as_deref(), args.tab_of.as_deref())?;
    let tab = subject.tab_target()?;
    // No position or size is picked: a new window takes the label default position and the apply service's size, and a lone popout (reuse) moves to the received position — so leave them empty for that window to stay in place. (TRD S28 §4-3)
    let to = TargetDest::NewWindow {
        at: WindowPlace {
            pos: None,
            size: None,
        },
    };
    let said = DockSaid {
        command: "tab.detach",
        subject,
        dest: None,
    };
    let m = dock_by_target(ports, tab, to, &said)?;
    Ok(TabDetachOk {
        changed: m.changed,
        window: m.window,
        window_id: m.window_id,
        number: m.number,
        tab: m.tab,
        tab_index: m.tab_index,
        layout: m.layout,
    })
}

fn verb_layout_get(ports: &LayoutPorts, args: LayoutGetArgs) -> Result<LayoutGetOk, CommandError> {
    let window = optional_window_arg("window", args.window.as_deref())?;
    let agent = optional_text("agent", args.agent.as_deref())?
        .map(|given| agent_arg("agent", given))
        .transpose()?;
    let with_ids = args.ids.unwrap_or(false);
    let window = window
        .as_deref()
        .map(|given| window_label(ports, "window", given))
        .transpose()?;
    // The only failure of this read is lock poisoning — not the state's fault (CONFLICT).
    let rows = apply::layout_rows(&ports.state, &RowFilter { window, agent })
        .map_err(CommandError::internal)?;
    Ok(LayoutGetOk {
        rows: rows.into_iter().map(|row| row_of(row, with_ids)).collect(),
    })
}

// ── Widened tab verbs — tab = view_id | tab_of · window (known from the tab if omitted) ── (TRD S28 §7)

fn verb_tab_switch(ports: &LayoutPorts, args: TabSwitchArgs) -> Result<TabSwitchOk, CommandError> {
    let tab = view_choice(args.view_id.as_deref(), args.tab_of.as_deref())?;
    let window = optional_window_arg("window", args.window.as_deref())?;
    let target = tab.tab_target()?;
    let window = window
        .as_deref()
        .map(|given| window_label(ports, "window", given))
        .transpose()?;
    apply::switch_tab_at(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        window.as_deref(),
        &target,
    )
    .map_err(|error| target_error(&ports.state, error, &tab))?;
    Ok(TabSwitchOk {})
}

fn verb_tab_close(ports: &LayoutPorts, args: TabCloseArgs) -> Result<TabCloseOk, CommandError> {
    let tab = view_choice(args.view_id.as_deref(), args.tab_of.as_deref())?;
    let window = optional_window_arg("window", args.window.as_deref())?;
    let target = tab.tab_target()?;
    let window = window
        .as_deref()
        .map(|given| window_label(ports, "window", given))
        .transpose()?;
    let closed = apply::close_tab_at(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        ports.windows.as_ref(),
        window.as_deref(),
        &target,
    )
    .map_err(|error| target_error(&ports.state, error, &tab))?;
    Ok(TabCloseOk {
        layout: layout_of(closed.rows),
    })
}

fn verb_tab_rename(ports: &LayoutPorts, args: TabRenameArgs) -> Result<TabRenameOk, CommandError> {
    let tab = view_choice(args.view_id.as_deref(), args.tab_of.as_deref())?;
    let name = text("name", &args.name)?.trim().to_string();
    let target = tab.tab_target()?;
    apply::rename_tab_at(&ports.state, ports.events.as_ref(), &target, name)
        .map_err(|error| target_error(&ports.state, error, &tab))?;
    Ok(TabRenameOk {})
}

// ── Widened area verbs — area = slot_id (+view_id) | slot_of | at (+window) ── (TRD S28 §7)

fn verb_slot_focus(ports: &LayoutPorts, args: SlotFocusArgs) -> Result<SlotFocusOk, CommandError> {
    let pick = SlotPick::read(
        args.view_id.as_deref(),
        args.slot_id.as_deref(),
        args.slot_of.as_deref(),
        args.at.as_deref(),
        args.window.as_deref(),
    )?;
    let target = pick.target(ports)?;
    let focused = apply::focus_slot_at(&ports.state, ports.events.as_ref(), &target)
        .map_err(|error| target_error(&ports.state, error, &pick.said))?;
    Ok(SlotFocusOk {
        layout: layout_of(focused.rows),
    })
}

fn verb_slot_split(ports: &LayoutPorts, args: SlotSplitArgs) -> Result<SlotSplitOk, CommandError> {
    let pick = SlotPick::read(
        args.view_id.as_deref(),
        args.slot_id.as_deref(),
        args.slot_of.as_deref(),
        args.at.as_deref(),
        args.window.as_deref(),
    )?;
    let dir = match args.dir {
        SplitDirection::LeftRight => SplitDir::LeftRight,
        SplitDirection::TopBottom => SplitDir::TopBottom,
    };
    let target = pick.target(ports)?;
    let split = apply::split_slot_at(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        &target,
        dir,
    )
    .map_err(|error| target_error(&ports.state, error, &pick.said))?;
    Ok(SlotSplitOk {
        slot_id: split.out.to_string(),
        layout: layout_of(split.rows),
    })
}

fn verb_slot_close(ports: &LayoutPorts, args: SlotCloseArgs) -> Result<SlotCloseOk, CommandError> {
    let pick = SlotPick::read(
        args.view_id.as_deref(),
        args.slot_id.as_deref(),
        args.slot_of.as_deref(),
        args.at.as_deref(),
        args.window.as_deref(),
    )?;
    let target = pick.target(ports)?;
    let closed = apply::close_slot_at(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        &target,
    )
    .map_err(|error| target_error(&ports.state, error, &pick.said))?;
    Ok(SlotCloseOk {
        layout: layout_of(closed.rows),
    })
}

fn verb_slot_popout(
    ports: &LayoutPorts,
    args: SlotPopoutArgs,
) -> Result<SlotPopoutOk, CommandError> {
    let pick = SlotPick::read(
        args.view_id.as_deref(),
        args.slot_id.as_deref(),
        args.slot_of.as_deref(),
        args.at.as_deref(),
        args.window.as_deref(),
    )?;
    let to_window = optional_window_arg("to_window", args.to_window.as_deref())?;
    let target = pick.target(ports)?;
    let to_window = to_window
        .as_deref()
        .map(|given| window_label(ports, "to_window", given))
        .transpose()?;
    let popped = apply::move_slot_to_window_at(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        ports.windows.as_ref(),
        ports.labels.as_ref(),
        &target,
        to_window,
    )
    .map_err(|error| target_error(&ports.state, error, &pick.said))?;
    // ★Not called `view_id`★: this command's arguments already have `view_id` (the **original** tab to detach from); the same name in the answer would mean the opposite side, and a caller feeding the answer back would pick the new tab instead of the original. `new_` keeps that the value is a view id, so it plugs into `view_id` of `tab.rename` · `tab.switch`. The service-side name is `tab` (frontend wire).
    Ok(SlotPopoutOk {
        window: popped.out.window,
        new_view_id: popped.out.tab.to_string(),
        window_id: popped.out.window_id,
        number: popped.out.number,
        layout: layout_of(popped.rows),
    })
}

fn verb_slot_assign_agent(
    ports: &LayoutPorts,
    args: SlotAssignAgentArgs,
) -> Result<SlotAssignAgentOk, CommandError> {
    let pick = SlotPick::read(
        args.view_id.as_deref(),
        args.slot_id.as_deref(),
        args.slot_of.as_deref(),
        args.at.as_deref(),
        args.window.as_deref(),
    )?;
    let agent = Assignee::read(args.agent_id.as_deref(), args.assign.as_deref())?.ok_or_else(
        || {
            CommandError::invalid_argument(
                "the agent to put in the slot is missing — give one of assign (its name or id), agent_id",
            )
        },
    )?;
    let target = pick.target(ports)?;
    let assigned = apply::assign_agent_at(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        &target,
        agent.id.to_string(),
    )
    .map_err(|error| target_error(&ports.state, error, &pick.said))?;
    Ok(SlotAssignAgentOk {
        layout: layout_of(assigned.rows),
    })
}

fn verb_set_slot_content(
    ports: &LayoutPorts,
    args: LayoutSetSlotContentArgs,
) -> Result<LayoutSetSlotContentOk, CommandError> {
    let pick = SlotPick::read(
        args.view_id.as_deref(),
        args.slot_id.as_deref(),
        args.slot_of.as_deref(),
        args.at.as_deref(),
        args.window.as_deref(),
    )?;
    let agent = Assignee::read(args.agent_id.as_deref(), args.assign.as_deref())?;
    let write = slot_content(args.content, agent, args.show_claude, args.show_codex)?;
    let target = pick.target(ports)?;
    let (state, subs, events) = (&ports.state, ports.subs.as_ref(), ports.events.as_ref());
    let written = match write {
        SlotContentWrite::Replace(content) => {
            apply::set_slot_content_at(state, subs, events, &target, content)
        }
        SlotContentWrite::Usage {
            show_claude,
            show_codex,
        } => apply::set_usage_slot_at(state, subs, events, &target, show_claude, show_codex),
    }
    .map_err(|error| target_error(&ports.state, error, &pick.said))?;
    Ok(LayoutSetSlotContentOk {
        layout: layout_of(written.rows),
    })
}

// ── The chosen words ──

/// The words that chose a tab · area — exactly one from one line of arguments choosing the same thing. `arg` · `word` = the received argument name and value (the error text echoes them) · `id_arg` = the line's id argument (where candidates go when ambiguous; per tab for a tab line) · `agent_arg` = the line's agent-choosing argument (recommended when the word picked no area; `None` if absent).
struct Choice {
    arg: &'static str,
    id_arg: &'static str,
    agent_arg: Option<&'static str>,
    word: String,
    by: ChoiceBy,
}

#[derive(Clone, Copy)]
enum ChoiceBy {
    Id(Uuid),
    /// The tab · area holding that agent — the agent id the daemon put in place of the name (ADR-0308).
    Agent(Uuid),
    At(SpatialToken),
}

impl Choice {
    fn spatial(
        arg: &'static str,
        id_arg: &'static str,
        agent_arg: Option<&'static str>,
        word: &str,
        token: SpatialToken,
    ) -> Self {
        Choice {
            arg,
            id_arg,
            agent_arg,
            word: word.to_string(),
            by: ChoiceBy::At(token),
        }
    }

    fn names_a_tab(&self) -> bool {
        matches!(self.id_arg, "tab" | "after_tab" | "view_id")
    }

    fn tab_target(&self) -> Result<Target, CommandError> {
        match self.by {
            ChoiceBy::Id(id) => Ok(Target::Tab(id)),
            ChoiceBy::Agent(agent) => Ok(Target::TabOf(agent)),
            ChoiceBy::At(_) => Err(CommandError::internal(format!(
                "{} cannot name a tab",
                self.arg
            ))),
        }
    }

    // `window` = the window to read the word in (an already resolved label · `None` = main).
    fn slot_target(&self, whole: bool, window: Option<String>) -> Result<Target, CommandError> {
        match self.by {
            ChoiceBy::Id(slot) => Ok(Target::Slot { view: None, slot }),
            // Attaching to the outer edge of the whole tab makes the tab the reference, so it is not ambiguous even if up in several areas of that tab.
            ChoiceBy::Agent(agent) if whole => Ok(Target::TabOf(agent)),
            ChoiceBy::Agent(agent) => Ok(Target::SlotOf(agent)),
            ChoiceBy::At(token) => Ok(Target::At { token, window }),
        }
    }
}

/// The line choosing a tab — exactly one of `id_arg` (the command's tab id argument) · `tab_of`. `what` = what the line chooses (for the text) · `source` = the command that returns that id (the bounce text points to it).
fn tab_line(
    id_arg: &'static str,
    what: &str,
    source: &str,
    id: Option<&str>,
    tab_of: Option<&str>,
) -> Result<Choice, CommandError> {
    let (index, arg, word) = one_of(&[(id_arg, id), ("tab_of", tab_of)], what)?;
    let by = match index {
        0 => ChoiceBy::Id(id_choice(
            arg,
            word,
            source,
            "tab_of (an agent shown in that tab)",
        )?),
        _ => ChoiceBy::Agent(agent_arg(arg, word)?),
    };
    Ok(Choice {
        arg,
        id_arg,
        agent_arg: Some("tab_of"),
        word: word.to_string(),
        by,
    })
}

fn tab_choice(tab: Option<&str>, tab_of: Option<&str>) -> Result<Choice, CommandError> {
    tab_line(
        "tab",
        "the tab to move",
        "tab.list / layout.get with ids",
        tab,
        tab_of,
    )
}

fn view_choice(view_id: Option<&str>, tab_of: Option<&str>) -> Result<Choice, CommandError> {
    tab_line("view_id", "the tab", LAYOUT_IDS, view_id, tab_of)
}

/// The 「right after that tab」 line (`after_tab` · `after_tab_of`) — `None` if neither (the rest of the destination line is `to_window` · `index`).
fn after_choice(
    after_tab: Option<&str>,
    after_tab_of: Option<&str>,
) -> Result<Option<Choice>, CommandError> {
    let after_tab = optional_text("after_tab", after_tab)?;
    let after_tab_of = optional_text("after_tab_of", after_tab_of)?;
    let (arg, word, by) = match (after_tab, after_tab_of) {
        (None, None) => return Ok(None),
        (Some(_), Some(_)) => {
            return Err(CommandError::invalid_argument(
                "give only one of to_window, after_tab, after_tab_of — they all say where the tab goes, got after_tab and after_tab_of",
            ))
        }
        (Some(word), None) => (
            "after_tab",
            word,
            ChoiceBy::Id(id_choice(
                "after_tab",
                word,
                "tab.list / layout.get with ids",
                "after_tab_of (an agent shown in that tab)",
            )?),
        ),
        (None, Some(word)) => (
            "after_tab_of",
            word,
            ChoiceBy::Agent(agent_arg("after_tab_of", word)?),
        ),
    };
    Ok(Some(Choice {
        arg,
        id_arg: "after_tab",
        agent_arg: Some("after_tab_of"),
        word: word.to_string(),
        by,
    }))
}

fn beside_choice(
    slot: Option<&str>,
    agent: Option<&str>,
    at: Option<&str>,
) -> Result<Choice, CommandError> {
    let (index, arg, word) = one_of(
        &[("beside_slot", slot), ("beside", agent), ("beside_at", at)],
        "the slot to attach beside",
    )?;
    let by = match index {
        0 => ChoiceBy::Id(id_choice(
            arg,
            word,
            "layout.get with ids",
            "beside (an agent shown in that slot) or beside_at (a spatial word)",
        )?),
        1 => ChoiceBy::Agent(agent_arg(arg, word)?),
        _ => ChoiceBy::At(spatial_token(arg, word)?),
    };
    Ok(Choice {
        arg,
        id_arg: "beside_slot",
        agent_arg: Some("beside"),
        word: word.to_string(),
        by,
    })
}

/// The line choosing an area (`slot_id` · `slot_of` · `at`) and its side arguments, shared by the widened area verbs. `view` = the `view_id` given with `slot_id` (it must be an area of that tab) · `window` = the window to read `at` in (shape-checked only; resolved to a model window right before applying). (TRD S28 §7)
struct SlotPick {
    said: Choice,
    view: Option<Uuid>,
    window: Option<String>,
}

impl SlotPick {
    fn read(
        view_id: Option<&str>,
        slot_id: Option<&str>,
        slot_of: Option<&str>,
        at: Option<&str>,
        window: Option<&str>,
    ) -> Result<Self, CommandError> {
        // The tab id is checked first — the same check order as the id callers before widening.
        let view = optional_text("view_id", view_id)?
            .map(|raw| uuid_arg_from("view_id", raw, LAYOUT_IDS))
            .transpose()?;
        let (index, arg, word) = one_of(
            &[("slot_id", slot_id), ("slot_of", slot_of), ("at", at)],
            "the slot",
        )?;
        let by = match index {
            0 => ChoiceBy::Id(id_choice(
                arg,
                word,
                LAYOUT_IDS,
                "slot_of (an agent shown in that slot) or at (a spatial word)",
            )?),
            1 => ChoiceBy::Agent(agent_arg(arg, word)?),
            _ => ChoiceBy::At(spatial_token(arg, word)?),
        };
        if view.is_some() && !matches!(by, ChoiceBy::Id(_)) {
            return Err(CommandError::invalid_argument(
                "view_id only goes with slot_id — it names the tab that slot is in",
            ));
        }
        let window = optional_window_arg("window", window)?;
        if window.is_some() && !matches!(by, ChoiceBy::At(_)) {
            return Err(CommandError::invalid_argument(
                "window only goes with at — it names the window whose shown tab the word is read in",
            ));
        }
        Ok(SlotPick {
            said: Choice {
                arg,
                id_arg: "slot_id",
                agent_arg: Some("slot_of"),
                word: word.to_string(),
                by,
            },
            view,
            window,
        })
    }

    fn target(&self, ports: &LayoutPorts) -> Result<Target, CommandError> {
        if let ChoiceBy::Id(slot) = self.said.by {
            return Ok(Target::Slot {
                view: self.view,
                slot,
            });
        }
        let window = self
            .window
            .as_deref()
            .map(|given| window_label(ports, "window", given))
            .transpose()?;
        self.said.slot_target(false, window)
    }
}

/// The agent to put in the area — one of `assign` | `agent_id`, both `#[agent_ref]` (the daemon resolves them against the roster and refuses itself if absent — ADR-0308). `arg` = the received argument name (for the text).
struct Assignee {
    arg: &'static str,
    id: Uuid,
}

impl Assignee {
    fn read(agent_id: Option<&str>, assign: Option<&str>) -> Result<Option<Self>, CommandError> {
        let agent_id = optional_text("agent_id", agent_id)?;
        let assign = optional_text("assign", assign)?;
        let (arg, word) = match (agent_id, assign) {
            (None, None) => return Ok(None),
            (Some(_), Some(_)) => return Err(CommandError::invalid_argument(
                "give only one of assign, agent_id — they both name the agent to put in the slot",
            )),
            (Some(word), None) => ("agent_id", word),
            (None, Some(word)) => ("assign", word),
        };
        Ok(Some(Assignee {
            arg,
            id: agent_arg(arg, word)?,
        }))
    }
}

/// The value of an `#[agent_ref]` field → an agent id. A non-UUID is a call that bypassed the daemon (which replaces names with ids before relaying, ADR-0308); the shell does not resolve names and bounces it (ADR-0132).
// ADR-0308
fn agent_arg(field: &str, given: &str) -> Result<Uuid, CommandError> {
    Uuid::parse_str(given.trim()).map_err(|_| {
        CommandError::invalid_argument(format!(
            "{field} must be an agent id (a UUID) here, got {given:?} — give the agent's name or id; names are turned into ids by the daemon before a command reaches this app"
        ))
    })
}

/// An id argument; if not a UUID, appends the paired name · word argument (`alternative`) to the bounce of [`uuid_arg_from`], so a call that put a name into the id field can fix it.
fn id_choice(
    field: &str,
    given: &str,
    source: &str,
    alternative: &str,
) -> Result<Uuid, CommandError> {
    uuid_arg_from(field, given, source).map_err(|error| {
        CommandError::invalid_argument(format!(
            "{} — to name it another way, use {alternative}",
            error.message()
        ))
    })
}

fn spatial_token(arg: &str, word: &str) -> Result<SpatialToken, CommandError> {
    SpatialToken::parse(word).ok_or_else(|| {
        CommandError::invalid_argument(format!(
            "{arg} must be one of top-left, top-right, bottom-left, bottom-right, left, right, up, down — got {word:?}"
        ))
    })
}

/// Exactly one from one line of arguments choosing the same thing — (position in the line, argument name, value). An empty value is bounced like [`optional_text`]. `what` = what the line chooses (for the text).
fn one_of<'a>(
    fields: &[(&'static str, Option<&'a str>)],
    what: &str,
) -> Result<(usize, &'static str, &'a str), CommandError> {
    let mut given = Vec::new();
    for (index, (field, value)) in fields.iter().enumerate() {
        if let Some(value) = optional_text(field, *value)? {
            given.push((index, *field, value));
        }
    }
    let all = || {
        fields
            .iter()
            .map(|(field, _)| *field)
            .collect::<Vec<_>>()
            .join(", ")
    };
    match given.as_slice() {
        [one] => Ok(*one),
        [] => Err(CommandError::invalid_argument(format!(
            "{what} is missing — give one of {}",
            all()
        ))),
        many => Err(CommandError::invalid_argument(format!(
            "give only one of {} — they all name {what}, got {}",
            all(),
            many.iter()
                .map(|(_, field, _)| *field)
                .collect::<Vec<_>>()
                .join(" and ")
        ))),
    }
}

/// Checks only the shape of the window argument first (empty value · number = `INVALID_ARGUMENT`); resolving it in the model is [`window_label`] right before applying (a missing window = `CONFLICT`).
fn optional_window_arg(field: &str, given: Option<&str>) -> Result<Option<String>, CommandError> {
    let Some(given) = optional_text(field, given)? else {
        return Ok(None);
    };
    window_arg::reject_number(given).map_err(window_arg_error)?;
    Ok(Some(given.to_string()))
}

// The range is the model contract (`tree::RATIO_MIN..=dock::SHARE_MAX`); the apply service refuses the same range (`InvalidShare`), this just bounces before taking the apply lock.
fn share_arg(share: f64) -> Result<f64, CommandError> {
    if share.is_finite() && (tree::RATIO_MIN..=dock::SHARE_MAX).contains(&share) {
        return Ok(share);
    }
    Err(CommandError::invalid_argument(format!(
        "share is the attached side's part, from {} to {}, got {share}",
        tree::RATIO_MIN,
        dock::SHARE_MAX
    )))
}

// ── Outcome · refusal of target-version docking ──

/// The material to turn a target-version docking refusal into code · text: the words the command used to choose the tab to move, and `dest` = the words choosing the destination as a target (`after_tab` · `after_tab_of` · the attach area line; `None` for a window · new window).
struct DockSaid {
    command: &'static str,
    subject: Choice,
    dest: Option<Choice>,
}

impl DockSaid {
    fn gave_id(&self, id: Uuid) -> bool {
        std::iter::once(&self.subject)
            .chain(self.dest.as_ref())
            .any(|choice| matches!(choice.by, ChoiceBy::Id(given) if given == id))
    }
}

/// The answer fields the three docking commands carry together — all one snapshot of the lock that builds the outcome (`DockOutcome::rows` · `identity`).
struct Moved {
    changed: bool,
    window: String,
    window_id: String,
    number: u32,
    tab: String,
    tab_index: Option<u32>,
    layout: Vec<LayoutRow>,
}

// LLM commands do not change the window's active tab · visible tab · focus (`Keep`; the exception is the apply service's `dock` succession).
fn dock_by_target(
    ports: &LayoutPorts,
    tab: Target,
    to: TargetDest,
    said: &DockSaid,
) -> Result<Moved, CommandError> {
    let outcome = apply::dock_target(
        &ports.state,
        ports.subs.as_ref(),
        ports.events.as_ref(),
        ports.windows.as_ref(),
        ports.labels.as_ref(),
        TargetDockRequest {
            tab,
            to,
            focus: FocusPolicy::Keep,
            mv: DockMove::default(),
        },
    )
    .map_err(|error| dock_target_error(&ports.state, error, said))?;
    moved(outcome)
}

fn moved(outcome: DockOutcome) -> Result<Moved, CommandError> {
    // The target version finds the moved tab again in the lock that builds the outcome, and a vanished window was already refused with `Moved`; this branch is the net for when that contract breaks. (TRD S28 §4-4)
    let Some(identity) = outcome.identity else {
        return Err(CommandError::of(
            ErrorCode::Conflict,
            TargetDockError::Moved.to_string(),
        ));
    };
    // The outcome rows always carry `tab` (first tab of `views`, same lock); if absent that contract broke. The move already happened, so it still answers success with only that field omitted (an applied write must not read as a failure — ADR-0159).
    let tab_index = outcome
        .rows
        .iter()
        .find(|row| row.tab == outcome.tab)
        .map(|row| row.tab_index);
    if tab_index.is_none() {
        tracing::warn!(
            tab = %outcome.tab,
            "the docking outcome rows have no result tab — answering success without tab_index"
        );
    }
    Ok(Moved {
        changed: outcome.changed,
        window: identity.window,
        window_id: identity.window_id,
        number: identity.number,
        tab: outcome.tab.to_string(),
        tab_index,
        layout: layout_of(outcome.rows),
    })
}

/// A target-version docking refusal → a bus error.
///
/// - Target unresolved = [`locate_error`].
/// - `NotASlot` · `SelfTarget` · `InvalidShare` = INVALID_ARGUMENT (the arguments contradict themselves); the `SelfTarget` text tells each command the fix (`tab.dock` = take one area out first with `slot.popout`).
/// - A tab · area the caller **gave by id** is missing (`TabNotFound` · `SlotNotFound` · `SlotNotInTab`) = NOT_FOUND; if the id came from an agent · word, CONFLICT (it vanished after resolving — a race).
/// - `WindowNotFound` = CONFLICT (ADR-0302 decision 9, as in [`window_arg_error`]) · `Moved` and the rest = CONFLICT.
// ADR-0307 (TRD S28 §5-3)
fn dock_target_error(state: &LayoutState, error: TargetDockError, said: &DockSaid) -> CommandError {
    let message = error.to_string();
    match error {
        TargetDockError::Subject(e) => locate_error(state, &said.subject, e),
        TargetDockError::Dest(e) => {
            locate_error(state, said.dest.as_ref().unwrap_or(&said.subject), e)
        }
        TargetDockError::NotASlot => CommandError::invalid_argument(message),
        TargetDockError::Moved => CommandError::of(ErrorCode::Conflict, message),
        TargetDockError::Dock(e) => dock_error(state, e, said),
    }
}

fn dock_error(state: &LayoutState, error: DockError, said: &DockSaid) -> CommandError {
    let code = match &error {
        DockError::SelfTarget | DockError::InvalidShare(_) => ErrorCode::InvalidArgument,
        DockError::TabNotFound(id)
        | DockError::SlotNotFound(id)
        | DockError::SlotNotInTab { slot: id, .. }
            if said.gave_id(*id) =>
        {
            ErrorCode::NotFound
        }
        _ => ErrorCode::Conflict,
    };
    let message = match (&error, &said.dest) {
        (DockError::SelfTarget, Some(dest)) if said.command == "tab.dock" => format!(
            "{}: both are in one tab; tab.dock moves whole tabs — slot.popout that slot first, then tab.dock",
            said_head(state, dest)
        ),
        (DockError::SelfTarget, Some(dest)) => format!(
            "{}: that is the tab being moved — a tab cannot go right after itself",
            said_head(state, dest)
        ),
        _ => error.to_string(),
    };
    CommandError::of(code, message)
}

/// The target could not be resolved → a bus error. Text = [`said_head`] + `: <reason>` + tail.
///
/// - The agent is in no area = NOT_FOUND + ` — candidates: <names that are up>` (`none` if none).
/// - The agent is in several places = CONFLICT + guidance to fix it with that line's id argument + candidates (at most 5), headed by that id argument name so the value can be copied as is: tab lines (`tab` · `after_tab` · `view_id`) one per tab `<arg>=<id> (window=<label>, tab_index=<n>)`; area lines (`slot_id` · `beside_slot`) one per area `<arg>=<id> (…)`.
/// - The word picks no area = NOT_FOUND + guidance on corner words · that line's agent argument (`beside` · `slot_of`); no window = CONFLICT (ADR-0302 decision 9).
/// - A tab · area given by id is missing = NOT_FOUND (only for id targets; vanishing after resolving is [`dock_error`]'s job).
// ADR-0307 (TRD S28 §5-2 · §5-3)
fn locate_error(state: &LayoutState, said: &Choice, error: LocateError) -> CommandError {
    let head = format!("{}: {error}", said_head(state, said));
    let (code, message) = match &error {
        LocateError::AgentNotShown { .. } => (
            ErrorCode::NotFound,
            format!("{head}{}", candidates_tail(&error.candidates_as(said.id_arg))),
        ),
        LocateError::AgentAmbiguous { places, .. } => {
            let candidates = if said.names_a_tab() {
                tab_candidates(said.id_arg, places)
            } else {
                error.candidates_as(said.id_arg)
            };
            (
                ErrorCode::Conflict,
                format!(
                    "{head}; pass one of these as {}{}",
                    said.id_arg,
                    candidates_tail(&candidates)
                ),
            )
        }
        LocateError::NoSlotAt { .. } => (
            ErrorCode::NotFound,
            format!(
                "{head} — left/right/up/down are read from the focused slot; try a corner word (top-left, top-right, bottom-left, bottom-right){}",
                said.agent_arg.map(|agent| format!(" or {agent}")).unwrap_or_default()
            ),
        ),
        LocateError::WindowNotFound(_) => (ErrorCode::Conflict, head),
        LocateError::TabNotFound(_)
        | LocateError::SlotNotFound(_)
        | LocateError::SlotNotInTab { .. } => (ErrorCode::NotFound, head),
    };
    CommandError::of(code, message)
}

/// The received words head `<arg> '<received words>'`; for an agent field `<arg> '<display name>' (<id>)` instead of the id the daemon put in (name ledger, display only — ADR-0132), just the id if not in the ledger.
// ★Do not call while holding the lock★ — it takes the layout lock to read the name ledger (call only in the error mapping after the apply returned).
fn said_head(state: &LayoutState, said: &Choice) -> String {
    match said.by {
        ChoiceBy::Agent(id) => match apply::agent_display_name(state, id) {
            Some(name) => format!("{} '{name}' ({id})", said.arg),
            None => format!("{} '{}'", said.arg, said.word),
        },
        ChoiceBy::Id(_) | ChoiceBy::At(_) => format!("{} '{}'", said.arg, said.word),
    }
}

// Per-tab ambiguous candidates, one line per tab (position of its first area), at most [`target::MAX_CANDIDATES`]. `key` = the argument name to pass that value to.
fn tab_candidates(key: &str, places: &[Place]) -> Vec<String> {
    let mut seen: Vec<Uuid> = Vec::new();
    places
        .iter()
        .filter(|place| {
            let first = !seen.contains(&place.view);
            seen.push(place.view);
            first
        })
        .take(target::MAX_CANDIDATES)
        .map(|place| {
            format!(
                "{key}={} (window={}, tab_index={})",
                place.view, place.window, place.tab_index
            )
        })
        .collect()
}

// A candidate tail in the daemon resolver's form — `none` if empty, so "no candidates" and "a text carrying no candidates" stay distinguishable.
fn candidates_tail(candidates: &[String]) -> String {
    if candidates.is_empty() {
        return " — candidates: none".to_string();
    }
    format!(" — candidates: {}", candidates.join(", "))
}

/// A refusal of a target-version area · tab write → a bus error.
///
/// - A tab · area given by id is missing = [`id_miss`]; a target chosen by agent · word that could not be resolved = [`locate_error`].
/// - Resolved but the apply was refused = CONFLICT + the service text (as in the id version — [`not_applied`]).
/// - A tab target for an area write = INTERNAL (area verbs build only area targets; the net for when that breaks).
// ADR-0307 (TRD S28 §5-3 · §7)
fn target_error(state: &LayoutState, error: TargetError, said: &Choice) -> CommandError {
    match error {
        TargetError::Locate(error) if matches!(said.by, ChoiceBy::Id(_)) => id_miss(error),
        TargetError::Locate(error) => locate_error(state, said, error),
        TargetError::NotASlot => CommandError::internal(TargetError::NotASlot.to_string()),
        TargetError::Apply(detail) => not_applied(detail),
    }
}

/// A tab · area given by id is missing — the text and code the id version (manager write) always emitted (CONFLICT): a missing tab = `ViewNotFound`, an area not in that tab = `SlotNotFound`. The target version decides earlier with `locate`, so only the text is turned back into the manager's; if the pair diverges, the text comparison in `tests/layout_commands.rs` goes red.
fn id_miss(error: LocateError) -> CommandError {
    let detail = match error {
        LocateError::TabNotFound(view) => LayoutError::ViewNotFound(view).to_string(),
        LocateError::SlotNotFound(slot) | LocateError::SlotNotInTab { slot, .. } => {
            LayoutError::SlotNotFound(slot).to_string()
        }
        other => other.to_string(),
    };
    not_applied(detail)
}

// The rows of the tab the write changed, without ids (rule §4 · areas are called by name · word).
fn layout_of(rows: Vec<rows::LayoutRow>) -> Vec<LayoutRow> {
    rows.into_iter().map(|row| row_of(row, false)).collect()
}

// With ids = false the four ids are emptied and left out of the answer ([`without_nulls`]).
fn row_of(row: rows::LayoutRow, ids: bool) -> LayoutRow {
    LayoutRow {
        window: row.window,
        window_number: row.window_number,
        tab_index: row.tab_index,
        tab_name: row.tab_name,
        tab_active: row.tab_active,
        content: match row.content {
            rows::SlotKind::Empty => SlotKind::Empty,
            rows::SlotKind::Agent => SlotKind::Agent,
            rows::SlotKind::AgentList => SlotKind::AgentList,
            rows::SlotKind::Usage => SlotKind::Usage,
            rows::SlotKind::Unknown => SlotKind::Unknown,
        },
        agent: row.agent,
        focused: row.focused,
        x: row.x,
        y: row.y,
        w: row.w,
        h: row.h,
        window_id: ids.then_some(row.window_id),
        tab: ids.then(|| row.tab.to_string()),
        slot_id: ids.then(|| row.slot.to_string()),
        agent_id: row.agent_id.filter(|_| ids),
    }
}

// ── Argument checks ────────────────────────────────────────────────────────────────

/// Carries the apply service's failure text as is — the branch of verbs taking ids (see the module header).
fn not_applied(detail: String) -> CommandError {
    CommandError::of(ErrorCode::Conflict, detail)
}

/// Failures of `agent.spawnInto`: a failure the daemon answered as `CODE: text` (store refusal = `CONFLICT` · store write failure = `INTERNAL` — `store_error_text` of daemon `connection_core`) uses that code as the outer code and the prefix is stripped; the retry directive is that code's default.
/// ★Do not wrap `CONFLICT` around it again★: two codes (`CONFLICT: INTERNAL: …`) would make a write failure read as "a bounce where nothing was applied". Codes this command does not advertise or texts without a code prefix (placement failure · policy refusal …) are [`not_applied`].
///
/// `args` = the refusal's structured part (ADR-0348): the daemon's launch-block object verbatim (`reasons`), plus
/// `agent_id` — the agent the daemon created that did not start (the same shape as bus `agent.spawn`), or the running
/// agent a placement failure left unplaced — plus `notices` when that spawn had any (M11), as the same declared rows
/// as the success answer ([`notice_rows`]). None of them = no `args`.
/// A created-asleep refusal also says in its text that the agent was not placed — the daemon's text is
/// `agent.spawn`'s, which has no slot to speak of.
// ADR-0291 R17 · ADR-0348
fn spawn_not_applied(refusal: SpawnRefusal) -> CommandError {
    let SpawnRefusal {
        message: detail,
        launch_blocked,
        created_agent_id,
        live_agent_id,
        notices,
    } = refusal;
    let mut args = launch_blocked.unwrap_or_default();
    let asleep = created_agent_id.map(|id| id.to_string());
    if let Some(id) = asleep.clone().or(live_agent_id) {
        args.insert("agent_id".to_string(), serde_json::Value::String(id));
    }
    let rows: Vec<serde_json::Value> = notice_rows(notices)
        .into_iter()
        .filter_map(|row| serde_json::to_value(row).ok())
        .collect();
    if !rows.is_empty() {
        args.insert("notices".to_string(), serde_json::Value::Array(rows));
    }
    let advertised = AgentSpawnIntoArgs::SPEC.advertised_errors();
    let (code, text) = match detail.split_once(": ").and_then(|(prefix, message)| {
        ErrorCode::from_wire(prefix)
            .filter(|code| advertised.contains(code))
            .map(|code| (code, message))
    }) {
        Some((code, message)) => (Some(code), message.to_string()),
        None => (None, detail),
    };
    let text = match asleep {
        Some(id) => format!(
            "{text} — it was not placed in a slot either: once it runs, place it with layout.setSlotContent <slot> --content Agent --assign {id}"
        ),
        None => text,
    };
    match code {
        Some(code) => CommandError::of(code, text),
        None => not_applied(text),
    }
    .with_args(args)
}

/// ★A whitespace-only value is bounced, not folded into absence★: an unset shell variable expanding to an empty argument (`--window "$UNSET"`) is realistic, and folding it into "not given" would let one typo touch another window (as `agent.*` does on the agent side).
fn text<'a>(field: &str, given: &'a str) -> Result<&'a str, CommandError> {
    if given.trim().is_empty() {
        return Err(CommandError::invalid_argument(format!(
            "{field} needs a real value — an empty argument is usually an unset shell variable"
        )));
    }
    Ok(given)
}

/// Resolves the window argument to the label of a window in the current model: digits only = `INVALID_ARGUMENT`; a window not in the model (including a label of an earlier run) = `CONFLICT` (as an apply failure). The layout lock is held only while resolving; if the window closes after that, the apply service refuses with the same `CONFLICT`. (TRD S26-window-chrome §4-5)
fn window_label(ports: &LayoutPorts, field: &str, given: &str) -> Result<String, CommandError> {
    let given = text(field, given)?;
    let mgr = ports
        .state
        .0
        .lock()
        .map_err(|e| not_applied(e.to_string()))?;
    window_arg::resolve(&mgr, given).map_err(window_arg_error)
}

fn optional_window_label(
    ports: &LayoutPorts,
    field: &str,
    given: Option<&str>,
) -> Result<Option<String>, CommandError> {
    optional_text(field, given)?
        .map(|given| window_label(ports, field, given))
        .transpose()
}

fn window_arg_error(error: WindowArgError) -> CommandError {
    let message = error.to_string();
    match error {
        WindowArgError::NumberGiven => CommandError::invalid_argument(message),
        WindowArgError::NotFound(_) => not_applied(message),
    }
}

fn optional_text<'a>(field: &str, given: Option<&'a str>) -> Result<Option<&'a str>, CommandError> {
    match given {
        None => Ok(None),
        Some(value) if value.trim().is_empty() => Err(CommandError::invalid_argument(format!(
            "{field} must either carry a value or be left out entirely — an empty argument is usually an unset shell variable"
        ))),
        Some(value) => Ok(Some(value)),
    }
}

/// ★Malformed values are bounced before applying★: applying with an unparsed id would give "a missing id" and "a malformed id" the same answer, and the caller would suspect a deleted tab and dig through the list.
fn uuid_arg(field: &str, given: &str) -> Result<Uuid, CommandError> {
    uuid_arg_from(field, given, "tab.list / layout.get with ids")
}

/// `source` = the command that returns that id; the bounce text guides the caller there.
fn uuid_arg_from(field: &str, given: &str, source: &str) -> Result<Uuid, CommandError> {
    Uuid::parse_str(given.trim()).map_err(|_| {
        CommandError::invalid_argument(format!(
            "{field} must be a UUID (the id shown by {source}), got {given:?}"
        ))
    })
}

/// What one bus call writes into a slot. `Usage` must fill omitted fields with the slot's current values, so it becomes a [`SlotContent`] only inside the ViewManager lock (`apply::set_usage_slot_at`).
enum SlotContentWrite {
    Replace(SlotContent),
    Usage {
        show_claude: Option<bool>,
        show_codex: Option<bool>,
    },
}

/// Tag + side fields → what to write into the slot. ★A mismatched combination is bounced, not silently fixed★: `Agent` without an agent would become an empty slot, `Empty` with an agent would make the caller think it is attached, and non-`Usage` with `show_*` would make the caller think a company was turned on.
fn slot_content(
    kind: SlotContentKind,
    agent: Option<Assignee>,
    show_claude: Option<bool>,
    show_codex: Option<bool>,
) -> Result<SlotContentWrite, CommandError> {
    let shows = show_claude.is_some() || show_codex.is_some();
    let write = match (kind, agent) {
        (SlotContentKind::Agent, Some(agent)) => SlotContentWrite::Replace(SlotContent::Agent {
            agent_id: agent.id.to_string(),
        }),
        (SlotContentKind::Agent, None) => {
            return Err(CommandError::invalid_argument(
                "content=Agent needs assign (the agent's name or id) or agent_id — use slot.assignAgent if you only want to attach a running agent",
            ))
        }
        (_, Some(agent)) => {
            return Err(CommandError::invalid_argument(format!(
                "{} only applies to content=Agent, so drop this field",
                agent.arg
            )))
        }
        (SlotContentKind::Usage, None) => {
            return Ok(SlotContentWrite::Usage {
                show_claude,
                show_codex,
            })
        }
        (SlotContentKind::Empty, None) => SlotContentWrite::Replace(SlotContent::Empty),
        (SlotContentKind::AgentList, None) => SlotContentWrite::Replace(SlotContent::AgentList),
    };
    if shows {
        return Err(CommandError::invalid_argument(
            "show_claude/show_codex only apply to content=Usage, so drop these fields",
        ));
    }
    Ok(write)
}

// Most assertions are in `tests/layout_commands.rs` (through the envelope path); these cover only outcomes that path cannot make. (`--test lib_unit`, ADR-0174)
#[cfg(test)]
mod tests {
    use super::*;

    // Even if the outcome rows have no result tab (the net for when the apply service contract breaks), an already moved docking answers success — only that field is omitted.
    #[test]
    fn a_dock_whose_rows_miss_its_tab_still_answers_success_without_tab_index() {
        let tab = Uuid::new_v4();
        let outcome = DockOutcome {
            changed: true,
            tab,
            views: vec![tab],
            window: "main".to_string(),
            slot_ids: Vec::new(),
            emptied: None,
            main_refilled: None,
            reused: false,
            prewarmed: false,
            identity: Some(apply::NewWindow {
                window: "main".to_string(),
                window_id: "main".to_string(),
                number: 1,
            }),
            rows: Vec::new(),
        };

        let m = moved(outcome).expect("success");

        assert!(m.changed);
        assert_eq!(m.tab, tab.to_string());
        assert_eq!(m.tab_index, None);
        let answer = serde_json::to_value(TabMoveOk {
            changed: m.changed,
            window: m.window,
            window_id: m.window_id,
            number: m.number,
            tab: m.tab,
            tab_index: m.tab_index,
            layout: m.layout,
        })
        .map(without_nulls)
        .expect("JSON");
        assert!(answer.get("tab_index").is_none(), "{answer}");
    }
}
