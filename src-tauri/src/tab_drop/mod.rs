//! The shell's share of tab drop (Tauri 0): the ledger [`DropBook`] and processing [`handle`] of the drop command `tab_drop`, the window fact `window:frame` computation [`frame_insets`] and emit-only-on-change [`push_frame`]. Tauri wrapper · port adapters = `commands::tab_drop`; decisions are in the pure module [`plan`], wire types in [`wire`].
//! Drag commands come only after the drag has ended (ADR-0296 · ADR-0297) — at most two per drag; window facts flow independent of dragging. Do not add a path sending the drag's decisions · geometry · state to the shell while dragging — that boundary synchronization is the root of the defect ADR-0296 removed.
//! - The drop does not compare against the state it saw: the model finds again, by id, what the drop pointed at in the current state, applies it matched to the current state if it exists and rejects if not (rules = [`plan::plan`]).
// hangul-ok-begin
//! - The model is written only through one `layout::apply::dock_tab`, the same as human · LLM commands (ADR-0081 · CLAUDE.md 「LLM-우선 제어」); this command is not on the bus command table — the LLM command for the same action, T-65 `tab.dock`, stands on the same function.
// hangul-ok-end
//! - Lock discipline: the ledger · window-fact table locks are leaves; while holding one, take no layout lock, emit or call a window. The result notification ([`DropShell::done`]) · focus are called after all locks are released.
// ADR-0296 · ADR-0297
// TRD S21-slot-docking §3

pub mod plan;
pub mod wire;

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use engram_dashboard_base::sync;
use uuid::Uuid;

use self::plan::{InputNow, Plan, SourceFrame, TopAt};
use self::wire::{
    CaptionChrome, DropOutcome, DropTo, EndEffect, NewWindowBy, TabDrop, TabDropDone, TabDropReply,
    WindowFrame,
};
use crate::layout::apply::{self, DockDest, DockOutcome};
use crate::layout::bands::SizeCss;
use crate::layout::geometry::Insets;
use crate::layout::manager::DockError;
use crate::layout::{
    LabelSource, LayoutEvents, LayoutState, SubscriptionSync, WindowAttrs, WindowHost,
};

/// Number of drags the ledger remembers — on overflow the oldest finished records are forgotten first. Records in progress (new window in progress · its rejection notification in progress) may hold a parked `drop` and are never forgotten. The two commands of one drag arrive back to back, so a forgotten drag is assumed not to come again.
const BOOK_CAP: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    Drop,
    DragEnd,
}

impl Arrival {
    pub fn of(to: &DropTo) -> Arrival {
        match to {
            DropTo::NewWindow {
                by: NewWindowBy::DragEnd { .. },
                ..
            } => Arrival::DragEnd,
            _ => Arrival::Drop,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Arrival::Drop => "drop",
            Arrival::DragEnd => "drag_end",
        }
    }
}

#[derive(Debug)]
struct Handling {
    drop: TabDrop,
    caller: String,
    input: InputNow,
    top_class: Option<String>,
    plan: Plan,
}

#[derive(Debug)]
enum Mark {
    Drop,
    Quiet,
    Placing(Option<Box<Handling>>),
    // The new window of drag_end was rejected with no parked drop and is emitting its `done{Rejected}` — like `Placing` it parks the first drop, and after the notification `close` hands it over. Processed before the notification, that drop's `done` could go before the rejection.
    Rejecting(Option<Box<Handling>>),
    Placed,
}

impl Mark {
    fn in_flight(&self) -> bool {
        matches!(self, Mark::Placing(_) | Mark::Rejecting(_))
    }
}

#[derive(Debug)]
enum Admit {
    Run(Handling),
    // The same drag's drag_end new window is in progress (or emitting its rejection notification), so the drop was parked — if that new window is rejected, that processing continues with it.
    Parked,
    Duplicate,
}

#[derive(Debug)]
enum Settle {
    Untracked,
    Placed(Option<Box<Handling>>),
    Superseded(Box<Handling>),
    Rejecting,
}

/// Recent drag ids and their processing branches — in whatever order a drag's `drag_end` · `drop` arrive the model is written at most once and `drop` wins. Decision · recording are inside one lock, a leaf (held only inside `admit` · `settle` · `close`).
#[derive(Debug, Default)]
pub struct DropBook {
    seen: Mutex<VecDeque<(Uuid, Mark)>>,
    overflow_warned: AtomicBool,
}

impl DropBook {
    // - A drag seen for the first time → process.
    // - A `drop` whose earlier record is a `drag_end` that did not write the model (cancel · ignore · rejected new window) → process (record becomes `drop`).
    // - A `drop` while the same drag's `drag_end` new window is in progress or in its rejection notification → park (only the first; hand-over by `settle` · `close`).
    // - Anything else → `duplicate`: two `drop`s of one drag · a `drag_end` after a `drop` · a `drop` after a `drag_end` that created a new window.
    // TRD S21-slot-docking §3-4
    fn admit(&self, h: Handling) -> Admit {
        let drag = h.drop.drag;
        let arrival = Arrival::of(&h.drop.to);
        let grew = {
            let mut seen = sync::lock(&self.seen);
            if let Some((_, mark)) = seen.iter_mut().find(|(d, _)| *d == drag) {
                return match mark {
                    Mark::Quiet if arrival == Arrival::Drop => {
                        *mark = Mark::Drop;
                        Admit::Run(h)
                    }
                    Mark::Placing(parked) | Mark::Rejecting(parked)
                        if arrival == Arrival::Drop && parked.is_none() =>
                    {
                        *parked = Some(Box::new(h));
                        Admit::Parked
                    }
                    _ => Admit::Duplicate,
                };
            }
            let mark = match (arrival, &h.plan) {
                (Arrival::Drop, _) => Mark::Drop,
                (Arrival::DragEnd, Plan::Dock(_)) => Mark::Placing(None),
                (Arrival::DragEnd, Plan::Cancel | Plan::Ignore(_)) => Mark::Quiet,
            };
            seen.push_back((drag, mark));
            evict(&mut seen)
        }; // ← ledger lock dropped
        if grew && !self.overflow_warned.swap(true, Ordering::Relaxed) {
            tracing::warn!(
                module = "dock",
                cap = BOOK_CAP,
                "drop ledger — every record at the cap is in progress, so it grows instead of forgetting (announced once per ledger)"
            );
        }
        Admit::Run(h)
    }

    // Record the result of the in-progress `drag_end` new window. Applied → record "the `drag_end` that created a new window" and return the parked `drop` (discarded — a later `drop` is also `duplicate`). Rejected with something parked → that one continues, record `drop`; nothing parked → leave `Rejecting` so a `drop` arriving while the rejection notification is emitted is parked too (`close` hands it over).
    // TRD S21-slot-docking §3-4
    fn settle(&self, drag: Uuid, outcome: DropOutcome) -> Settle {
        let mut seen = sync::lock(&self.seen);
        let Some((_, mark)) = seen.iter_mut().find(|(d, _)| *d == drag) else {
            return Settle::Untracked;
        };
        let Mark::Placing(parked) = mark else {
            return Settle::Untracked;
        };
        let parked = parked.take();
        match (outcome, parked) {
            (DropOutcome::Rejected, Some(parked)) => {
                *mark = Mark::Drop;
                Settle::Superseded(parked)
            }
            (DropOutcome::Rejected, None) => {
                *mark = Mark::Rejecting(None);
                Settle::Rejecting
            }
            (_, parked) => {
                *mark = Mark::Placed;
                Settle::Placed(parked)
            }
        }
    }

    // Called after emitting the rejection notification — ends `Rejecting`. A `drop` parked meanwhile is recorded as `drop` and returned (continued; its `done` follows the rejection). If none, lower to a `drag_end` that did not write the model so a later `drop` wins. Not `Rejecting` → no-op (`None`).
    // TRD S21-slot-docking §3-4
    fn close(&self, drag: Uuid) -> Option<Box<Handling>> {
        let mut seen = sync::lock(&self.seen);
        let (_, mark) = seen.iter_mut().find(|(d, _)| *d == drag)?;
        let Mark::Rejecting(parked) = mark else {
            return None;
        };
        let parked = parked.take();
        *mark = match parked {
            Some(_) => Mark::Drop,
            None => Mark::Quiet,
        };
        parked
    }
}

// Forget finished records from the oldest as many as exceed the limit — the just-inserted (last) record and in-progress ones are not forgotten; `true` if it grew because nothing could be forgotten.
fn evict(seen: &mut VecDeque<(Uuid, Mark)>) -> bool {
    while seen.len() > BOOK_CAP {
        let older = seen.len() - 1;
        let Some(at) = seen.iter().take(older).position(|(_, m)| !m.in_flight()) else {
            return true;
        };
        seen.remove(at);
    }
    false
}

pub trait DropShell: Send + Sync {
    /// Give focus to the window holding the moved tab — only when a drop on the tab bar · seam was applied (a new window opens active). Usually the window dropped on; if the seam's target tab went to another window meanwhile, that window.
    fn focus(&self, label: &str);
    fn done(&self, source: &str, done: &TabDropDone);
}

pub struct DropPorts<'a> {
    pub state: &'a LayoutState,
    pub subs: &'a dyn SubscriptionSync,
    pub events: &'a dyn LayoutEvents,
    pub host: &'a dyn WindowHost,
    pub labels: &'a dyn LabelSource,
    pub shell: &'a dyn DropShell,
}

/// Process one drop command — decision ([`plan::plan`]) → ledger → apply (one `dock_tab`) → focus → result notification → one log line. The caller reads `input` and the source window frame `frame` before this, input first. `caller` = the window that called the command; `top_class` = the window class when the topmost is another app's window (for the log).
/// The same drag again is `Duplicate` with no notification. A `drop` arriving while the same drag's `drag_end` new window is in progress (including while its rejection notification is emitted) is also answered `Duplicate`, but the ledger parks it: if that new window is rejected the processing continues the parked `drop` by the ordinary `drop` path (its result goes only by `done`); if the new window stands the parked drop is discarded. If something was parked at the rejection decision, that `drag_end` processing emits no `done` of its own (log only · `superseded`) — the `done` the source receives is that `drop`'s alone; if parked after the rejection notification, that `done` follows the rejection. Otherwise it emits `done` once per result — for an apply after `dock_tab`'s model notification. A rejection leaves the model as is, and `Rejected{kind}`'s `kind` = the word in the parentheses of the docking error text. Cancel · ignore do not write the model.
/// Do not call `new_window` from the main thread or a synchronous command — the new-window build deadlocks (`dock_tab` docs).
// TRD S21-slot-docking §3-2 · §3-4
pub fn handle(
    book: &DropBook,
    ports: &DropPorts<'_>,
    drop: TabDrop,
    caller: &str,
    input: &InputNow,
    frame: &SourceFrame,
    top_class: Option<&str>,
) -> TabDropReply {
    let arrival = Arrival::of(&drop.to);
    let (drag, view) = (drop.drag, drop.view);
    let plan = plan::plan(&drop, caller, input, frame);
    let item = Handling {
        drop,
        caller: caller.to_owned(),
        input: input.clone(),
        top_class: top_class.map(str::to_owned),
        plan,
    };
    let item = match book.admit(item) {
        Admit::Run(item) => item,
        Admit::Parked => {
            tracing::debug!(
                module = "dock",
                %drag,
                by = arrival.name(),
                %view,
                caller,
                "drop — a new window of the same drag is in progress, so it is parked (continued if that new window is rejected)"
            );
            return TabDropReply::Duplicate;
        }
        Admit::Duplicate => {
            tracing::debug!(
                module = "dock",
                %drag,
                by = arrival.name(),
                %view,
                caller,
                "drop — the same drag was already handled; discarding"
            );
            return TabDropReply::Duplicate;
        }
    };
    let (reply, parked) = run(book, ports, item);
    if let Some(parked) = parked {
        let _ = run(book, ports, *parked);
    }
    reply
}

// One received processing — apply → focus → result notification → log. If it is a rejected `drag_end` new window and the ledger has a parked `drop`, it returns that to continue. With something parked at the rejection decision it emits no notification of its own — a rejection `done` going first would make the source undo the swapped drawing and flash the dragged tab (TRD §2-4 ①); with nothing parked it emits the rejection notification then `close` returns what was parked meanwhile (its `done` follows the rejection).
fn run(
    book: &DropBook,
    ports: &DropPorts<'_>,
    item: Handling,
) -> (TabDropReply, Option<Box<Handling>>) {
    let Handling {
        drop,
        caller,
        input,
        top_class,
        plan,
    } = item;
    let mut via = None;
    let (decision, outcome, reply, settled) = match plan {
        Plan::Dock(req) => {
            let in_place = !matches!(req.to, DockDest::NewWindow { .. });
            let applied = apply::dock_tab(
                ports.state,
                ports.subs,
                ports.events,
                ports.host,
                ports.labels,
                req,
            );
            let outcome = match applied {
                Ok(_) => DropOutcome::Applied,
                Err(_) => DropOutcome::Rejected,
            };
            if let (Ok(out), false) = (&applied, in_place) {
                via = Some(new_window_via(out));
            }
            let settled = book.settle(drop.drag, outcome);
            let reply = match applied {
                Ok(out) => {
                    if in_place {
                        ports.shell.focus(&out.window);
                    }
                    let (window_id, number) = out.identity.map(|w| (w.window_id, w.number)).unzip();
                    TabDropReply::Applied {
                        window: out.window,
                        window_id,
                        number,
                        reused: out.reused,
                    }
                }
                Err(e) => TabDropReply::Rejected {
                    kind: kind_of(&e),
                    reason: e.to_string(),
                },
            };
            ("dock", outcome, reply, settled)
        }
        Plan::Cancel => (
            "cancel",
            DropOutcome::Canceled,
            TabDropReply::Canceled,
            Settle::Untracked,
        ),
        Plan::Ignore(why) => (
            "ignore",
            DropOutcome::Ignored,
            TabDropReply::Ignored { why: why.into() },
            Settle::Untracked,
        ),
    };
    let superseded = matches!(settled, Settle::Superseded(_));
    if !superseded {
        ports.shell.done(
            &drop.from,
            &TabDropDone {
                drag: drop.drag,
                outcome,
            },
        );
    }
    let replay = match settled {
        Settle::Superseded(parked) => Some(parked),
        Settle::Rejecting => book.close(drop.drag),
        Settle::Placed(Some(parked)) => {
            tracing::debug!(
                module = "dock",
                drag = %parked.drop.drag,
                view = %parked.drop.view,
                caller = %parked.caller,
                "drop — a new window of the same drag stood up — discarding the parked drop"
            );
            None
        }
        Settle::Placed(None) | Settle::Untracked => None,
    };
    log_handled(
        &drop,
        &caller,
        &input,
        top_class.as_deref(),
        decision,
        outcome,
        &reply,
        superseded,
        via,
    );
    (reply, replay)
}

// Where the dropped new window came from — a handed-over prewarmed window · a newly built window · the source window moved with no new window. Prewarm is not layered over reuse (TRD S21-slot-docking §3-2 ⑦ · §3-9 「log」).
// ADR-0299
fn new_window_via(out: &DockOutcome) -> &'static str {
    if out.reused {
        "reuse"
    } else if out.prewarmed {
        "spare"
    } else {
        "fresh"
    }
}

#[allow(clippy::too_many_arguments)]
fn log_handled(
    drop: &TabDrop,
    caller: &str,
    input: &InputNow,
    top_class: Option<&str>,
    decision: &str,
    outcome: DropOutcome,
    reply: &TabDropReply,
    superseded: bool,
    via: Option<&str>,
) {
    let (to, effect) = match &drop.to {
        DropTo::TabBar { .. } => ("tab_bar", None),
        DropTo::Seam { .. } => ("seam", None),
        DropTo::NewWindow { by, .. } => (
            "new_window",
            match by {
                NewWindowBy::DragEnd { effect } => Some(match effect {
                    EndEffect::None => "none",
                    EndEffect::Move => "move",
                }),
                NewWindowBy::Drop => None,
            },
        ),
    };
    let top = match &input.top {
        TopAt::Ours(label) => format!("ours={label}"),
        TopAt::Foreign => "foreign".to_owned(),
        TopAt::Unknown => "unknown".to_owned(),
    };
    let (kind, why) = match reply {
        TabDropReply::Rejected { kind, .. } => (Some(kind.as_str()), None),
        TabDropReply::Ignored { why } => (None, Some(why.as_str())),
        _ => (None, None),
    };
    tracing::info!(
        module = "dock",
        drag = %drop.drag,
        by = Arrival::of(&drop.to).name(),
        effect,
        view = %drop.view,
        from = %drop.from,
        caller,
        to,
        decision,
        cursor = ?input.cursor.map(|c| (c.x, c.y)),
        button = input.primary_down,
        esc = input.escape_down,
        top = %top,
        top_class,
        ?outcome,
        kind,
        why,
        superseded = superseded.then_some(true),
        via,
        "drop handled"
    );
}

// The word in the last parentheses of the docking error text — text = reason + ` (kind)` (`DockError` docs). Parentheses inside the reason come before it.
fn kind_of(e: &DockError) -> String {
    let text = e.to_string();
    text.rsplit_once('(')
        .and_then(|(_, tail)| tail.strip_suffix(')'))
        .unwrap_or("Unknown")
        .to_owned()
}

// ── Window fact `window:frame` (TRD S21-slot-docking §3-8) ──────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameGeom {
    pub outer_pos: (i32, i32),
    pub outer_size: (u32, u32),
    pub inner_pos: (i32, i32),
    pub inner_size: (u32, u32),
    pub scale: f64,
}

/// Window frame = outer rectangle − inner rectangle (per side · CSS px, divided by that window's scale factor). `window:frame`'s `insets` and the new-window placement's `inset_dip` ([`source_frame`]) use this. `None` if the scale factor is not a finite positive number.
// TRD S21-slot-docking §3-5 · §3-8
pub fn frame_insets(g: &FrameGeom) -> Option<Insets> {
    if !(g.scale.is_finite() && g.scale > 0.0) {
        return None;
    }
    let (ox, oy) = (f64::from(g.outer_pos.0), f64::from(g.outer_pos.1));
    let (ow, oh) = (f64::from(g.outer_size.0), f64::from(g.outer_size.1));
    let (ix, iy) = (f64::from(g.inner_pos.0), f64::from(g.inner_pos.1));
    let (iw, ih) = (f64::from(g.inner_size.0), f64::from(g.inner_size.1));
    Some(Insets {
        t: (iy - oy) / g.scale,
        r: (ox + ow - (ix + iw)) / g.scale,
        b: (oy + oh - (iy + ih)) / g.scale,
        l: (ix - ox) / g.scale,
    })
}

const NO_INSETS: Insets = Insets {
    t: 0.0,
    r: 0.0,
    b: 0.0,
    l: 0.0,
};

pub fn source_frame(geom: Option<&FrameGeom>) -> SourceFrame {
    match geom.and_then(|g| frame_insets(g).map(|insets| (insets, g.scale))) {
        Some((insets, scale)) => SourceFrame { insets, scale },
        None => SourceFrame {
            insets: NO_INSETS,
            scale: 1.0,
        },
    }
}

pub fn window_frame(seen: &FrameSeen, insets: Insets) -> WindowFrame {
    WindowFrame {
        restored: seen.attrs.bounds.map(|b| SizeCss { w: b.w(), h: b.h() }),
        maximized: seen.attrs.maximized,
        insets,
        focused: seen.focused,
        caption: seen.caption,
    }
}

/// One window as read now — `minimized` · `geom` are getters (read holding no lock), `attrs` is that window's model value, `focused` is that window's focus event value or the OS foreground-window decision, `caption` is an OS constant (TRD S26-window-chrome §6-5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameSeen {
    pub minimized: bool,
    pub attrs: WindowAttrs,
    pub geom: Option<FrameGeom>,
    pub focused: bool,
    pub caption: CaptionChrome,
}

/// The last `window:frame` emitted per window — emitted only when the value changes. Leaf lock. A closed window's entry is deleted by [`FrameBook::forget`].
#[derive(Debug, Default)]
pub struct FrameBook {
    last: Mutex<HashMap<String, WindowFrame>>,
}

impl FrameBook {
    fn last(&self, label: &str) -> Option<WindowFrame> {
        sync::lock(&self.last).get(label).copied()
    }

    /// Forget that window's last value after it vanishes — a restored popout reopens with its saved label, and a left entry would suppress the first value when it equals the old window's.
    pub fn forget(&self, label: &str) {
        sync::lock(&self.last).remove(label);
    }
}

/// Called after the placement-recording path (`Moved` · `Resized` · the window port's `record_placement`) wrote to the model, and by that window's focus event (`Focused`): only when that window's value (the front window included) differs from the last emitted, it calls `emit` once after releasing the table lock. While minimized or when the geometry could not be read it neither emits nor records (a getter while minimized gives an off-screen value). The burst of `Moved` while dragging a title bar has the same value and is not emitted.
// TRD S21-slot-docking §3-8 · TRD S26-window-chrome §6-5
pub fn push_frame(book: &FrameBook, label: &str, seen: FrameSeen, emit: impl FnOnce(&WindowFrame)) {
    if seen.minimized {
        return;
    }
    let Some(insets) = seen.geom.as_ref().and_then(frame_insets) else {
        return;
    };
    let frame = window_frame(&seen, insets);
    let changed = {
        let mut last = sync::lock(&book.last);
        let changed = last.get(label) != Some(&frame);
        if changed {
            last.insert(label.to_owned(), frame);
        }
        changed
    }; // ← table lock dropped
    if changed {
        emit(&frame);
    }
}

/// The answer of `window_frame`, pulled at mount — restored size · maximize are model values, the frame is computed with the current getters. While minimized or when geometry could not be read it carries the last frame emitted for that window (0 if none). Not recorded in the table.
pub fn pull_frame(book: &FrameBook, label: &str, seen: FrameSeen) -> WindowFrame {
    let fresh = if seen.minimized {
        None
    } else {
        seen.geom.as_ref().and_then(frame_insets)
    };
    let insets = fresh
        .or_else(|| book.last(label).map(|f| f.insets))
        .unwrap_or(NO_INSETS);
    window_frame(&seen, insets)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use uuid::Uuid;

    use super::plan::{InputNow, PhysPoint, Plan, SourceFrame, TopAt};
    use super::wire::{DropAnchor, DropOutcome, DropTo, EndEffect, NewWindowBy, TabDrop};
    use super::*;
    use crate::commands::popout::PopupCounter;
    use crate::layout::apply::{
        DockDest, DockRequest, MoveFailed, SpareClaim, WindowPlace, WindowTabsPayload,
    };
    use crate::layout::bands::PointCss;
    use crate::layout::dock::DockSide;
    use crate::layout::manager::{DockMove, DockTo, FocusPolicy};
    use crate::layout::{ViewManager, ViewSnapshot, WindowBounds, MAIN_WINDOW_LABEL};

    const MAIN: &str = MAIN_WINDOW_LABEL;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    // ── Fake ports — record the order called on one line, and check by `try_lock` on the spot what must be outside the lock ──

    struct Rig {
        state: LayoutState,
        log: Mutex<Vec<String>>,
        // (receiving window, body)
        dones: Mutex<Vec<(String, TabDropDone)>>,
        open_fails: bool,
        // Where to push in the same drag's drop (with the ledger and that drop) — a race in the middle of processing the new-window drag_end.
        racing: Mutex<Option<(RaceAt, Arc<DropBook>, TabDrop)>>,
        racing_replies: Mutex<Vec<TabDropReply>>,
        spare: Mutex<Option<SpareClaim>>,
    }

    impl Rig {
        fn new() -> Self {
            Rig {
                state: LayoutState::new(),
                log: Mutex::new(Vec::new()),
                dones: Mutex::new(Vec::new()),
                open_fails: false,
                racing: Mutex::new(None),
                racing_replies: Mutex::new(Vec::new()),
                spare: Mutex::new(None),
            }
        }

        fn outcomes(&self) -> Vec<DropOutcome> {
            self.dones
                .lock()
                .unwrap()
                .iter()
                .map(|(_, d)| d.outcome)
                .collect()
        }

        fn note(&self, what: String) {
            self.log.lock().unwrap().push(what);
        }

        fn outside(&self, who: &str) {
            assert!(
                self.state.0.try_lock().is_ok(),
                "{who} must be called after the layout lock is released"
            );
        }

        fn take_log(&self) -> Vec<String> {
            std::mem::take(&mut *self.log.lock().unwrap())
        }

        fn mgr(&self) -> std::sync::MutexGuard<'_, ViewManager> {
            self.state.0.lock().unwrap()
        }

        fn race(&self, at: RaceAt) {
            let racing = {
                let mut slot = self.racing.lock().unwrap();
                match &*slot {
                    Some((when, ..)) if *when == at => slot.take(),
                    _ => None,
                }
            };
            if let Some((_, book, drop)) = racing {
                let reply = self.handle(&book, drop, MAIN, &released(TopAt::Unknown));
                self.racing_replies.lock().unwrap().push(reply);
            }
        }

        fn handle(
            &self,
            book: &DropBook,
            drop: TabDrop,
            caller: &str,
            input: &InputNow,
        ) -> TabDropReply {
            let labels = PopupCounter::default();
            let ports = DropPorts {
                state: &self.state,
                subs: self,
                events: self,
                host: self,
                labels: &labels,
                shell: self,
            };
            handle(book, &ports, drop, caller, input, &FLAT, None)
        }
    }

    impl SubscriptionSync for Rig {
        fn resync(&self, _mgr: &ViewManager) {
            self.note("resync".into());
        }
    }

    impl LayoutEvents for Rig {
        fn layout_updated(&self, _snapshot: &ViewSnapshot) {
            self.outside("layout_updated");
            self.note("layout".into());
        }

        fn window_tabs_updated(&self, tabs: &WindowTabsPayload) {
            self.outside("window_tabs_updated");
            self.note(format!("tabs:{}", tabs.label));
        }
    }

    impl WindowHost for Rig {
        fn open(&self, _label: &str) -> Result<(), String> {
            Ok(())
        }

        fn open_at(&self, label: &str, _at: &WindowPlace, _activate: bool) -> Result<(), String> {
            self.note(format!("open:{label}"));
            self.race(RaceAt::Open);
            if self.open_fails {
                Err("could not create the window".into())
            } else {
                Ok(())
            }
        }

        fn close(&self, label: &str) {
            self.note(format!("close:{label}"));
        }

        fn is_open(&self, _label: &str) -> bool {
            true
        }

        fn move_window(&self, label: &str, _at: &WindowPlace) -> Result<(), MoveFailed> {
            self.note(format!("move:{label}"));
            Ok(())
        }

        fn claim_spare(&self) -> Option<SpareClaim> {
            let claim = self.spare.lock().unwrap().take()?;
            self.note(format!("claim:{}", claim.label));
            Some(claim)
        }

        fn committed(&self, claim: &SpareClaim) {
            self.outside("committed");
            self.note(format!("committed:{}", claim.label));
        }

        fn release_spare(&self, claim: &SpareClaim) {
            self.outside("release_spare");
            self.note(format!("release:{}", claim.label));
        }
    }

    impl DropShell for Rig {
        fn focus(&self, label: &str) {
            self.outside("focus");
            self.note(format!("focus:{label}"));
        }

        fn done(&self, source: &str, done: &TabDropDone) {
            self.outside("done");
            self.note(format!("done:{:?}", done.outcome));
            self.dones.lock().unwrap().push((source.to_owned(), *done));
            self.race(RaceAt::Done);
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum RaceAt {
        Open,
        Done,
    }

    // (field name, value) for each event emitted on this thread — the wording is the `message` field.
    #[derive(Clone, Default)]
    struct Logs(Arc<Mutex<Vec<Vec<(String, String)>>>>);

    impl Logs {
        fn handled(&self, name: &str) -> Vec<Option<String>> {
            let field = |line: &[(String, String)], name: &str| {
                line.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone())
            };
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter(|line| field(line, "message").as_deref() == Some("drop handled"))
                .map(|line| field(line, name))
                .collect()
        }
    }

    struct Fields<'a>(&'a mut Vec<(String, String)>);

    impl tracing::field::Visit for Fields<'_> {
        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.0.push((field.name().to_owned(), value.to_owned()));
        }

        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.push((field.name().to_owned(), format!("{value:?}")));
        }
    }

    impl tracing::Subscriber for Logs {
        fn register_callsite(
            &self,
            _: &'static tracing::Metadata<'static>,
        ) -> tracing::subscriber::Interest {
            tracing::subscriber::Interest::sometimes()
        }
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut line = Vec::new();
            event.record(&mut Fields(&mut line));
            self.0.lock().unwrap().push(line);
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    const FLAT: SourceFrame = SourceFrame {
        insets: NO_INSETS,
        scale: 1.0,
    };

    fn input(top: TopAt, button: bool, esc: bool) -> InputNow {
        InputNow {
            cursor: Some(PhysPoint { x: 400, y: 300 }),
            primary_down: button,
            escape_down: esc,
            top,
            scale: 1.0,
        }
    }

    fn released(top: TopAt) -> InputNow {
        input(top, false, false)
    }

    fn drag_end(effect: EndEffect) -> DropTo {
        DropTo::NewWindow {
            by: NewWindowBy::DragEnd { effect },
            size: None,
            grab: PointCss { x: 0.0, y: 0.0 },
        }
    }

    fn drop(drag: Uuid, view: Uuid, from: &str, initial: Option<Uuid>, to: DropTo) -> TabDrop {
        TabDrop {
            drag,
            view,
            from: from.into(),
            initial,
            to,
        }
    }

    fn applied(rig: &Rig, window: &str, reused: bool) -> TabDropReply {
        let mgr = rig.mgr();
        let w = &mgr.windows[window];
        TabDropReply::Applied {
            window: window.into(),
            window_id: Some(w.window_id.clone()),
            number: Some(w.number),
            reused,
        }
    }

    // main = [a, b] · active a.
    fn two_tabs(rig: &Rig) -> (Uuid, Uuid) {
        let mut mgr = rig.mgr();
        let a = mgr.list_tabs(MAIN).unwrap().active;
        let b = mgr.create_tab(MAIN, None).unwrap();
        mgr.switch_tab(MAIN, a).unwrap();
        (a, b)
    }

    fn plan_dock() -> Plan {
        Plan::Dock(DockRequest {
            view: id(1),
            to: DockDest::TabBar {
                window: MAIN.into(),
                index: None,
                after: None,
            },
            focus: FocusPolicy::Reveal,
            mv: DockMove::default(),
        })
    }

    // ── ④ Ledger (§3-4) ──────────────────────────────────────────────────────────

    // Admit one command of that drag into the ledger — answer = "run" · "parked" · "duplicate".
    fn admit(book: &DropBook, drag: Uuid, arrival: Arrival, plan: Plan) -> &'static str {
        let to = match arrival {
            Arrival::Drop => DropTo::TabBar { index: 0 },
            Arrival::DragEnd => drag_end(EndEffect::None),
        };
        let item = Handling {
            drop: drop(drag, id(1), MAIN, None, to),
            caller: MAIN.into(),
            input: released(TopAt::Unknown),
            top_class: None,
            plan,
        };
        match book.admit(item) {
            Admit::Run(_) => "run",
            Admit::Parked => "parked",
            Admit::Duplicate => "duplicate",
        }
    }

    // Record the new-window result of that drag in the ledger — answer = "untracked" · "placed" · "placed+parked" · "superseded" · "rejecting".
    fn settle(book: &DropBook, drag: Uuid, outcome: DropOutcome) -> &'static str {
        match book.settle(drag, outcome) {
            Settle::Untracked => "untracked",
            Settle::Placed(None) => "placed",
            Settle::Placed(Some(_)) => "placed+parked",
            Settle::Superseded(_) => "superseded",
            Settle::Rejecting => "rejecting",
        }
    }

    #[test]
    fn the_book_takes_only_the_first_handling_of_a_drag() {
        let book = DropBook::default();
        assert_eq!(admit(&book, id(1), Arrival::Drop, plan_dock()), "run");
        assert_eq!(
            admit(&book, id(1), Arrival::Drop, plan_dock()),
            "duplicate",
            "two drops of the same drag"
        );
        assert_eq!(
            admit(&book, id(1), Arrival::DragEnd, Plan::Cancel),
            "duplicate",
            "drag_end after drop"
        );
        assert_eq!(admit(&book, id(2), Arrival::DragEnd, Plan::Cancel), "run");
        assert_eq!(
            admit(&book, id(2), Arrival::DragEnd, Plan::Cancel),
            "duplicate",
            "the same drag_end delivered twice"
        );
    }

    #[test]
    fn a_drop_wins_over_an_earlier_drag_end_that_wrote_nothing() {
        for quiet in [Plan::Cancel, Plan::Ignore(plan::LOST_DROP)] {
            let book = DropBook::default();
            assert_eq!(admit(&book, id(1), Arrival::DragEnd, quiet.clone()), "run");
            assert_eq!(
                admit(&book, id(1), Arrival::Drop, plan_dock()),
                "run",
                "drop after {quiet:?}"
            );
            assert_eq!(
                admit(&book, id(1), Arrival::Drop, plan_dock()),
                "duplicate",
                "the record turned into a drop"
            );
        }
    }

    #[test]
    fn a_drop_during_a_drag_end_new_window_is_parked_and_settled_by_its_outcome() {
        let book = DropBook::default();
        // The new window stood — get back the parked drop and discard it, and a later drop is duplicate.
        assert_eq!(admit(&book, id(1), Arrival::DragEnd, plan_dock()), "run");
        assert_eq!(admit(&book, id(1), Arrival::Drop, plan_dock()), "parked");
        assert_eq!(
            admit(&book, id(1), Arrival::Drop, plan_dock()),
            "duplicate",
            "only the first drop is parked"
        );
        assert_eq!(settle(&book, id(1), DropOutcome::Applied), "placed+parked");
        assert_eq!(admit(&book, id(1), Arrival::Drop, plan_dock()), "duplicate");
        assert_eq!(
            settle(&book, id(1), DropOutcome::Rejected),
            "untracked",
            "a finished new window is not settled again"
        );

        // There is a parked drop but the new window was rejected — that one stands in for the rejection, so a later drop is duplicate.
        assert_eq!(admit(&book, id(3), Arrival::DragEnd, plan_dock()), "run");
        assert_eq!(admit(&book, id(3), Arrival::Drop, plan_dock()), "parked");
        let Settle::Superseded(parked) = book.settle(id(3), DropOutcome::Rejected) else {
            panic!("the parked drop stands in for the rejection");
        };
        assert_eq!(parked.drop.drag, id(3));
        assert_eq!(admit(&book, id(3), Arrival::Drop, plan_dock()), "duplicate");
        assert!(book.close(id(3)).is_none(), "a rejection that was stood in for has nothing to close");
    }

    #[test]
    fn a_rejected_new_window_without_a_parked_drop_keeps_parking_until_closed() {
        let book = DropBook::default();
        // A drop that arrives during the rejection notification (after `settle` · before `close`) is parked, and `close` hands it over.
        assert_eq!(admit(&book, id(1), Arrival::DragEnd, plan_dock()), "run");
        assert_eq!(settle(&book, id(1), DropOutcome::Rejected), "rejecting");
        assert_eq!(
            admit(&book, id(1), Arrival::DragEnd, Plan::Cancel),
            "duplicate",
            "the same drag_end delivered twice"
        );
        assert_eq!(admit(&book, id(1), Arrival::Drop, plan_dock()), "parked");
        assert_eq!(
            admit(&book, id(1), Arrival::Drop, plan_dock()),
            "duplicate",
            "only the first drop is parked"
        );
        let parked = book.close(id(1)).expect("the parked drop");
        assert_eq!(parked.drop.drag, id(1));
        assert_eq!(
            admit(&book, id(1), Arrival::Drop, plan_dock()),
            "duplicate",
            "continues handling the handed-over drop"
        );
        assert!(
            book.close(id(1)).is_none(),
            "a closed record is not closed again"
        );

        // F6 — if it closes with nothing parked, a later drop wins.
        assert_eq!(admit(&book, id(2), Arrival::DragEnd, plan_dock()), "run");
        assert_eq!(settle(&book, id(2), DropOutcome::Rejected), "rejecting");
        assert!(book.close(id(2)).is_none());
        assert_eq!(admit(&book, id(2), Arrival::Drop, plan_dock()), "run");
        assert_eq!(admit(&book, id(2), Arrival::Drop, plan_dock()), "duplicate");
    }

    #[test]
    fn a_rejected_drop_stays_recorded() {
        let book = DropBook::default();
        assert_eq!(admit(&book, id(1), Arrival::Drop, plan_dock()), "run");
        assert_eq!(settle(&book, id(1), DropOutcome::Rejected), "untracked");
        assert_eq!(admit(&book, id(1), Arrival::Drop, plan_dock()), "duplicate");
        assert_eq!(
            admit(&book, id(1), Arrival::DragEnd, Plan::Cancel),
            "duplicate"
        );
    }

    #[test]
    fn the_book_forgets_the_oldest_drag_after_sixty_four() {
        let book = DropBook::default();
        for n in 0..=(BOOK_CAP as u128) {
            assert_eq!(admit(&book, id(n), Arrival::Drop, plan_dock()), "run");
        }
        assert_eq!(
            admit(&book, id(0), Arrival::Drop, plan_dock()),
            "run",
            "the 65th pushed out the oldest one"
        );
        assert_eq!(admit(&book, id(2), Arrival::Drop, plan_dock()), "duplicate");
    }

    #[test]
    fn the_book_never_forgets_a_drag_whose_new_window_is_in_flight() {
        let book = DropBook::default();
        // The two oldest = new window in progress · its rejection notification in progress.
        assert_eq!(admit(&book, id(1000), Arrival::DragEnd, plan_dock()), "run");
        assert_eq!(admit(&book, id(1001), Arrival::DragEnd, plan_dock()), "run");
        assert_eq!(settle(&book, id(1001), DropOutcome::Rejected), "rejecting");
        // Two + 63 = limit + 1 — the oldest finished id (0) is pushed out.
        for n in 0..(BOOK_CAP as u128 - 1) {
            assert_eq!(admit(&book, id(n), Arrival::Drop, plan_dock()), "run");
        }
        assert_eq!(
            admit(&book, id(1000), Arrival::Drop, plan_dock()),
            "parked",
            "a new window in progress was not forgotten"
        );
        assert_eq!(
            admit(&book, id(1001), Arrival::Drop, plan_dock()),
            "parked",
            "a new window in rejection notification too"
        );
        assert_eq!(
            admit(&book, id(0), Arrival::Drop, plan_dock()),
            "run",
            "the oldest of the finished records was pushed out"
        );
        assert_eq!(admit(&book, id(2), Arrival::Drop, plan_dock()), "duplicate");
        assert!(!book.overflow_warned.load(Ordering::Relaxed));
    }

    #[test]
    fn a_book_full_of_in_flight_drags_grows_then_shrinks_back() {
        let book = DropBook::default();
        for n in 0..=(BOOK_CAP as u128) {
            assert_eq!(admit(&book, id(n), Arrival::DragEnd, plan_dock()), "run");
        }
        assert_eq!(
            book.seen.lock().unwrap().len(),
            BOOK_CAP + 1,
            "grows if all are in progress"
        );
        assert!(book.overflow_warned.load(Ordering::Relaxed));
        assert_eq!(admit(&book, id(0), Arrival::Drop, plan_dock()), "parked");
        assert_eq!(settle(&book, id(0), DropOutcome::Applied), "placed+parked");
        for n in 1..=(BOOK_CAP as u128) {
            assert_eq!(settle(&book, id(n), DropOutcome::Applied), "placed");
        }
        assert_eq!(admit(&book, id(999), Arrival::Drop, plan_dock()), "run");
        assert_eq!(
            book.seen.lock().unwrap().len(),
            BOOK_CAP,
            "after finishing, the next record returns it to the cap"
        );
    }

    #[test]
    fn only_a_new_window_drag_end_arrives_as_a_drag_end() {
        assert_eq!(Arrival::of(&drag_end(EndEffect::Move)), Arrival::DragEnd);
        let by_drop = DropTo::NewWindow {
            by: NewWindowBy::Drop,
            size: None,
            grab: PointCss { x: 0.0, y: 0.0 },
        };
        assert_eq!(Arrival::of(&by_drop), Arrival::Drop);
        assert_eq!(Arrival::of(&DropTo::TabBar { index: 0 }), Arrival::Drop);
    }

    // ── ⑤ Rejected move · cancel · ignore ────────────────────────────────────────────

    #[test]
    fn each_dock_error_kind_is_the_word_in_its_closing_parentheses() {
        let cases = [
            (DockError::TabNotFound(id(1)), "TabNotFound"),
            (DockError::WindowNotFound("w".into()), "WindowNotFound"),
            (DockError::SlotNotFound(id(1)), "SlotNotFound"),
            (
                DockError::SlotNotInTab {
                    slot: id(1),
                    tab: id(2),
                },
                "SlotNotInTab",
            ),
            (DockError::SelfTarget, "SelfTarget"),
            (DockError::ZeroArea, "ZeroArea"),
            (DockError::InvalidShare("0.9".into()), "InvalidShare"),
            (DockError::WindowClosed("w".into()), "WindowClosed"),
            (DockError::WindowExists("w".into()), "WindowExists"),
            (DockError::SlotIdOverlap, "SlotIdOverlap"),
            (
                DockError::WindowOpen("moving the window Position failed — access denied (os error 5)".into()),
                "WindowOpen",
            ),
        ];
        for (e, want) in cases {
            assert_eq!(kind_of(&e), want, "{e}");
        }
    }

    #[test]
    fn a_rejected_drop_reports_the_kind_and_leaves_the_model() {
        let rig = Rig::new();
        two_tabs(&rig);
        let before = rig.mgr().version;
        let gone = id(42);
        let reply = rig.handle(
            &DropBook::default(),
            drop(id(9), gone, MAIN, None, DropTo::TabBar { index: 1 }),
            MAIN,
            &released(TopAt::Unknown),
        );
        assert_eq!(
            reply,
            TabDropReply::Rejected {
                kind: "TabNotFound".into(),
                reason: DockError::TabNotFound(gone).to_string(),
            },
            "the dragged tab vanished in the meantime"
        );
        assert_eq!(rig.mgr().version, before);
        assert_eq!(rig.mgr().list_tabs(MAIN).unwrap().tabs.len(), 2);
        assert_eq!(rig.take_log(), ["done:Rejected"], "no notification · no focus");
    }

    #[test]
    fn a_cancel_or_ignore_writes_nothing() {
        let rig = Rig::new();
        let (a, _) = two_tabs(&rig);
        let book = DropBook::default();
        let before = rig.mgr().version;
        let cases = [
            (
                drag_end(EndEffect::None),
                input(TopAt::Foreign, true, false),
                TabDropReply::Canceled,
            ),
            (
                drag_end(EndEffect::None),
                released(TopAt::Ours(MAIN.into())),
                TabDropReply::Ignored {
                    why: plan::LOST_DROP.into(),
                },
            ),
            (
                drag_end(EndEffect::Move),
                released(TopAt::Unknown),
                TabDropReply::Ignored {
                    why: plan::ACCEPTED_UNKNOWN.into(),
                },
            ),
        ];
        for (n, (to, now, want)) in cases.into_iter().enumerate() {
            let reply = rig.handle(&book, drop(id(n as u128), a, MAIN, None, to), MAIN, &now);
            assert_eq!(reply, want);
        }
        assert_eq!(rig.mgr().version, before);
        assert_eq!(
            rig.take_log(),
            ["done:Canceled", "done:Ignored", "done:Ignored"],
            "none of the model, window or subscription ports is called"
        );
    }

    // ── ⑨ `tab_drop:done` ──────────────────────────────────────────────────────

    #[test]
    fn done_comes_once_per_handling_after_the_model_news_and_never_for_a_duplicate() {
        let rig = Rig::new();
        let (a, _) = two_tabs(&rig);
        let book = DropBook::default();
        let reorder = drop(id(7), a, MAIN, Some(a), DropTo::TabBar { index: 1 });
        let reply = rig.handle(&book, reorder.clone(), MAIN, &released(TopAt::Unknown));
        assert_eq!(
            reply,
            TabDropReply::Applied {
                window: MAIN.into(),
                window_id: Some(MAIN.into()),
                number: Some(1),
                reused: false
            },
            "an existing window (tab bar) carries that window's identity too"
        );
        assert_eq!(rig.mgr().list_tabs(MAIN).unwrap().tabs[1].id, a);
        let log = rig.take_log();
        assert_eq!(
            log.last().map(String::as_str),
            Some("done:Applied"),
            "{log:?}"
        );
        let notified = log
            .iter()
            .rposition(|s| s == "layout" || s.starts_with("tabs:"))
            .expect("model notification");
        let focus = log.iter().position(|s| s == "focus:main").expect("focus");
        assert!(notified < focus, "model notification → focus → done: {log:?}");
        assert_eq!(log.iter().filter(|s| s.starts_with("done:")).count(), 1);

        assert_eq!(
            rig.handle(&book, reorder, MAIN, &released(TopAt::Unknown)),
            TabDropReply::Duplicate
        );
        assert!(rig.take_log().is_empty(), "a duplicate has no notification either");
    }

    #[test]
    fn an_ignored_drag_end_then_the_winning_drop_each_announce_once() {
        let rig = Rig::new();
        let (a, _) = two_tabs(&rig);
        let book = DropBook::default();
        let ignored = rig.handle(
            &book,
            drop(id(5), a, MAIN, Some(a), drag_end(EndEffect::Move)),
            MAIN,
            &released(TopAt::Ours(MAIN.into())),
        );
        assert_eq!(
            ignored,
            TabDropReply::Ignored {
                why: plan::ACCEPTED_OURS.into()
            }
        );
        let won = rig.handle(
            &book,
            drop(id(5), a, MAIN, Some(a), DropTo::TabBar { index: 1 }),
            MAIN,
            &released(TopAt::Unknown),
        );
        assert!(matches!(won, TabDropReply::Applied { .. }), "{won:?}");
        assert_eq!(rig.outcomes(), [DropOutcome::Ignored, DropOutcome::Applied]);
    }

    #[test]
    fn a_drop_after_a_rejected_drag_end_new_window_is_applied() {
        let rig = Rig {
            open_fails: true,
            ..Rig::new()
        };
        let (a, _) = two_tabs(&rig);
        let book = DropBook::default();
        let rejected = rig.handle(
            &book,
            drop(id(6), a, MAIN, Some(a), drag_end(EndEffect::None)),
            MAIN,
            &released(TopAt::Foreign),
        );
        assert!(
            matches!(&rejected, TabDropReply::Rejected { kind, .. } if kind == "WindowOpen"),
            "{rejected:?}"
        );
        let won = rig.handle(
            &book,
            drop(id(6), a, MAIN, Some(a), DropTo::TabBar { index: 1 }),
            MAIN,
            &released(TopAt::Unknown),
        );
        assert!(matches!(won, TabDropReply::Applied { .. }), "{won:?}");
        assert_eq!(
            rig.outcomes(),
            [DropOutcome::Rejected, DropOutcome::Applied],
            "a drop that arrives after the rejection notification · close wins like a normal drop"
        );
    }

    #[test]
    fn a_seam_drop_into_another_window_applies_with_focus_on_that_window() {
        let rig = Rig::new();
        let (a, _) = two_tabs(&rig);
        let target = rig.mgr().create_window("slot-popup-1").unwrap();
        let seam = DropTo::Seam {
            target,
            anchor: DropAnchor::Outer,
            side: DockSide::Right,
            share: 0.5,
        };
        let reply = rig.handle(
            &DropBook::default(),
            drop(id(8), a, MAIN, None, seam),
            "slot-popup-1",
            &released(TopAt::Unknown),
        );
        assert_eq!(reply, applied(&rig, "slot-popup-1", false));
        let log = rig.take_log();
        assert!(log.contains(&"focus:slot-popup-1".to_owned()), "{log:?}");
        assert_eq!(log.last().map(String::as_str), Some("done:Applied"));
        let dones = rig.dones.lock().unwrap();
        assert_eq!(
            dones.iter().map(|(to, _)| to.as_str()).collect::<Vec<_>>(),
            [MAIN],
            "done goes only to the source window (from), not to the calling window"
        );
    }

    #[test]
    fn a_seam_drop_whose_target_tab_moved_to_another_window_docks_there_and_focuses_that_window() {
        let rig = Rig::new();
        let (a, b) = two_tabs(&rig);
        // Before dropping on the seam of b that was visible in main, b went to a popout (the LLM, etc.).
        let popup = {
            let mut mgr = rig.mgr();
            mgr.create_window("slot-popup-1").unwrap();
            mgr.dock(
                b,
                DockTo::TabBar {
                    window: "slot-popup-1".into(),
                    index: None,
                    after: None,
                },
                FocusPolicy::Keep,
                DockMove::default(),
            )
            .unwrap();
            "slot-popup-1"
        };
        let seam = DropTo::Seam {
            target: b,
            anchor: DropAnchor::Outer,
            side: DockSide::Right,
            share: 0.5,
        };
        let reply = rig.handle(
            &DropBook::default(),
            drop(id(8), a, MAIN, None, seam),
            MAIN,
            &released(TopAt::Unknown),
        );
        assert_eq!(reply, applied(&rig, popup, false));
        let log = rig.take_log();
        assert!(log.contains(&format!("focus:{popup}")), "{log:?}");
        assert!(!log.contains(&"focus:main".to_owned()), "{log:?}");
        assert_eq!(rig.mgr().owner_of(a), None, "a attached to b and vanished");
    }

    // ── ⑩ The drop of the same drag as an in-progress drag_end new window ──────────────────────────────

    // Drag id (6), dragging a from main = [a, b] — while the drag_end (new window) is being processed, a tab-bar drop arrives at spot `at`.
    fn race_new_window_with_a_drop(
        rig: &Rig,
        at: RaceAt,
    ) -> (Uuid, Uuid, Arc<DropBook>, TabDropReply) {
        let (a, b) = two_tabs(rig);
        let book = Arc::new(DropBook::default());
        let reorder = drop(id(6), a, MAIN, Some(a), DropTo::TabBar { index: 1 });
        *rig.racing.lock().unwrap() = Some((at, book.clone(), reorder));
        let ended = rig.handle(
            &book,
            drop(id(6), a, MAIN, Some(a), drag_end(EndEffect::None)),
            MAIN,
            &released(TopAt::Foreign),
        );
        assert_eq!(
            *rig.racing_replies.lock().unwrap(),
            [TabDropReply::Duplicate],
            "a drop that arrives while a new window is in progress is parked and answered right away"
        );
        (a, b, book, ended)
    }

    fn main_tabs(rig: &Rig) -> Vec<Uuid> {
        rig.mgr()
            .list_tabs(MAIN)
            .unwrap()
            .tabs
            .iter()
            .map(|t| t.id)
            .collect()
    }

    // Even if one more drop of the same drag arrives, it is duplicate and there is no notification.
    fn assert_drag_is_over(rig: &Rig, book: &DropBook, a: Uuid) {
        let again = drop(id(6), a, MAIN, Some(a), DropTo::TabBar { index: 0 });
        assert_eq!(
            rig.handle(book, again, MAIN, &released(TopAt::Unknown)),
            TabDropReply::Duplicate,
            "a drop of the same drag after the continued drop"
        );
        assert!(rig.take_log().is_empty());
    }

    #[test]
    fn a_drop_parked_before_the_new_window_is_rejected_replaces_its_done() {
        let rig = Rig {
            open_fails: true,
            ..Rig::new()
        };
        let logs = Logs::default();
        let (a, b, book, ended) = tracing::subscriber::with_default(logs.clone(), || {
            race_new_window_with_a_drop(&rig, RaceAt::Open)
        });
        assert!(
            matches!(&ended, TabDropReply::Rejected { kind, .. } if kind == "WindowOpen"),
            "{ended:?}"
        );
        assert_eq!(main_tabs(&rig), [b, a], "the parked drop was applied");
        assert_eq!(
            rig.outcomes(),
            [DropOutcome::Applied],
            "a rejected drag_end emits no done — the done the source receives is the drop's alone"
        );
        assert_eq!(
            logs.handled("superseded"),
            [Some("true".to_owned()), None],
            "the drag_end handling line is kept but records that it yielded its place"
        );
        let log = rig.take_log();
        let focus = log.iter().position(|s| s == "focus:main").expect("focus");
        let applied = log.iter().position(|s| s == "done:Applied").unwrap();
        assert!(focus < applied, "{log:?}");
        assert_drag_is_over(&rig, &book, a);
    }

    #[test]
    fn a_drop_during_the_rejected_announcement_is_parked_and_applied_after_it() {
        let rig = Rig {
            open_fails: true,
            ..Rig::new()
        };
        let logs = Logs::default();
        let (a, b, book, ended) = tracing::subscriber::with_default(logs.clone(), || {
            race_new_window_with_a_drop(&rig, RaceAt::Done)
        });
        assert!(
            matches!(&ended, TabDropReply::Rejected { kind, .. } if kind == "WindowOpen"),
            "{ended:?}"
        );
        assert_eq!(
            main_tabs(&rig),
            [b, a],
            "after the rejection notification the parked drop was applied"
        );
        assert_eq!(
            rig.outcomes(),
            [DropOutcome::Rejected, DropOutcome::Applied],
            "the rejection done comes first · the drag's last done is the drop's"
        );
        assert_eq!(logs.handled("superseded"), [None, None]);
        let log = rig.take_log();
        let rejected = log.iter().position(|s| s == "done:Rejected").unwrap();
        let focus = log.iter().position(|s| s == "focus:main").expect("focus");
        let applied = log.iter().position(|s| s == "done:Applied").unwrap();
        assert!(rejected < focus && focus < applied, "{log:?}");
        assert_drag_is_over(&rig, &book, a);
    }

    #[test]
    fn a_drop_parked_during_a_new_window_that_opened_is_a_duplicate_without_done() {
        let rig = Rig::new();
        let (a, b, _book, ended) = race_new_window_with_a_drop(&rig, RaceAt::Open);
        let TabDropReply::Applied {
            window,
            reused: false,
            ..
        } = ended
        else {
            panic!("{ended:?}");
        };
        let ids = |label: &str| -> Vec<Uuid> {
            rig.mgr()
                .list_tabs(label)
                .unwrap()
                .tabs
                .iter()
                .map(|t| t.id)
                .collect()
        };
        assert_eq!(ids(&window), [a]);
        assert_eq!(ids(MAIN), [b], "the parked drop did not use the model");
        assert_eq!(
            rig.outcomes(),
            [DropOutcome::Applied],
            "the parked drop has no notification either"
        );
    }

    // ── ⑦ `via` of the drop-processing line (TRD §3-2 ⑦ · §3-9 「log」 · ADR-0299) ─────────────────────────

    // The `via` field of one processing and its answer — drag `view` from the `from` window to the desktop (new window).
    fn new_window_drop_via(rig: &Rig, view: Uuid, from: &str) -> (Option<String>, TabDropReply) {
        let logs = Logs::default();
        let reply = tracing::subscriber::with_default(logs.clone(), || {
            rig.handle(
                &DropBook::default(),
                drop(id(8), view, from, Some(view), drag_end(EndEffect::None)),
                from,
                &released(TopAt::Foreign),
            )
        });
        let [via] = logs.handled("via").try_into().expect("one drop-handled line");
        (via, reply)
    }

    #[test]
    fn a_new_window_drop_logs_whether_the_window_was_prewarmed_built_or_reused() {
        let fresh = Rig::new();
        let (a, _) = two_tabs(&fresh);
        assert_eq!(
            new_window_drop_via(&fresh, a, MAIN).0.as_deref(),
            Some("fresh")
        );

        let prewarmed = Rig::new();
        let (a, _) = two_tabs(&prewarmed);
        let spare = "slot-popup-9";
        *prewarmed.spare.lock().unwrap() = Some(SpareClaim {
            label: spare.into(),
            generation: 4,
        });
        let (via, reply) = new_window_drop_via(&prewarmed, a, MAIN);
        assert_eq!(via.as_deref(), Some("spare"));
        assert_eq!(
            reply,
            applied(&prewarmed, spare, false),
            "opens with the handed-over window's label"
        );
        let log = prewarmed.take_log();
        let at = |entry: &str| log.iter().position(|s| s == entry).expect(entry);
        let notified = log
            .iter()
            .rposition(|s| s == "layout" || s.starts_with("tabs:"))
            .expect("model notification");
        assert!(
            at("claim:slot-popup-9") < at("open:slot-popup-9")
                && at("open:slot-popup-9") < notified
                && notified < at("committed:slot-popup-9"),
            "entrust → open → notify → confirm: {log:?}"
        );

        let reused = Rig::new();
        let lone = reused.mgr().create_window("slot-popup-5").unwrap();
        let (via, reply) = new_window_drop_via(&reused, lone, "slot-popup-5");
        assert!(
            matches!(reply, TabDropReply::Applied { reused: true, .. }),
            "{reply:?}"
        );
        assert_eq!(via.as_deref(), Some("reuse"));

        let in_place = Rig::new();
        let (a, _) = two_tabs(&in_place);
        let logs = Logs::default();
        tracing::subscriber::with_default(logs.clone(), || {
            in_place.handle(
                &DropBook::default(),
                drop(id(9), a, MAIN, Some(a), DropTo::TabBar { index: 1 }),
                MAIN,
                &released(TopAt::Unknown),
            )
        });
        assert_eq!(logs.handled("via"), [None], "absent for a drop that is not a new window");
    }

    // The answer of a new-window drop carries the persistent id · number of the window that holds the moved tab — in all cases: a newly built window · a prewarmed window handed over · a reused source window
    // (TRD S26-window-chrome §3-2).
    #[test]
    fn a_new_window_drop_answers_the_identity_of_the_window_the_tab_landed_in() {
        for spare in [None, Some("slot-popup-9")] {
            let rig = Rig::new();
            let (a, _) = two_tabs(&rig);
            if let Some(label) = spare {
                *rig.spare.lock().unwrap() = Some(SpareClaim {
                    label: label.into(),
                    generation: 4,
                });
            }
            let (_, reply) = new_window_drop_via(&rig, a, MAIN);
            let TabDropReply::Applied {
                window,
                window_id: Some(window_id),
                number: Some(number),
                reused: false,
            } = &reply
            else {
                panic!("{spare:?}: {reply:?}");
            };
            if let Some(label) = spare {
                assert_eq!(window, label, "handed-over window");
            }
            assert_eq!(rig.mgr().owner_of(a), Some(window), "{spare:?}");
            assert!(Uuid::parse_str(window_id).is_ok(), "{window_id}");
            assert_eq!(*number, 2, "{spare:?}: first number after main");
            assert_eq!(reply, applied(&rig, window, false), "{spare:?}");
        }

        let rig = Rig::new();
        let lone = rig.mgr().create_window("slot-popup-5").unwrap();
        let (_, reply) = new_window_drop_via(&rig, lone, "slot-popup-5");
        assert_eq!(reply, applied(&rig, "slot-popup-5", true), "reuse");
    }

    // ── ⑥ `window:frame` ───────────────────────────────────────────────────────

    // Outer (100, 50) 816 × 639 · inner (108, 81) 800 × 600 — frame = top 31 · the rest 8.
    const GEOM: FrameGeom = FrameGeom {
        outer_pos: (100, 50),
        outer_size: (816, 639),
        inner_pos: (108, 81),
        inner_size: (800, 600),
        scale: 1.0,
    };

    fn attrs(maximized: bool) -> WindowAttrs {
        WindowAttrs {
            theme: None,
            bounds: WindowBounds::new(100.0, 50.0, 800.0, 600.0),
            maximized,
        }
    }

    const CAPTION: CaptionChrome = CaptionChrome {
        buttons: true,
        leading_inset: 0.0,
        snap_overlay: false,
    };

    fn seen(maximized: bool, geom: Option<FrameGeom>) -> FrameSeen {
        FrameSeen {
            minimized: false,
            attrs: attrs(maximized),
            geom,
            focused: false,
            caption: CAPTION,
        }
    }

    #[test]
    fn frame_insets_are_outer_minus_inner_divided_by_the_scale() {
        assert_eq!(
            frame_insets(&GEOM),
            Some(Insets {
                t: 31.0,
                r: 8.0,
                b: 8.0,
                l: 8.0
            })
        );
        let scaled = FrameGeom {
            outer_pos: (0, 0),
            outer_size: (1020, 799),
            inner_pos: (10, 39),
            inner_size: (1000, 750),
            scale: 1.25,
        };
        assert_eq!(
            frame_insets(&scaled),
            Some(Insets {
                t: 31.2,
                r: 8.0,
                b: 8.0,
                l: 8.0
            })
        );
        for bad in [0.0, -1.0, f64::NAN] {
            assert_eq!(frame_insets(&FrameGeom { scale: bad, ..GEOM }), None);
        }
    }

    #[test]
    fn the_frame_goes_out_only_when_it_changes() {
        let book = FrameBook::default();
        let mut sent = Vec::new();
        let mut push = |s: FrameSeen| push_frame(&book, MAIN, s, |f| sent.push(*f));
        push(seen(false, Some(GEOM)));
        // The burst of `Moved` while moving by the title bar — size · frame · maximize are the same.
        for x in 0..5 {
            push(seen(
                false,
                Some(FrameGeom {
                    outer_pos: (100 + x, 50),
                    inner_pos: (108 + x, 81),
                    ..GEOM
                }),
            ));
        }
        push(seen(true, Some(GEOM)));
        push(seen(true, Some(GEOM)));
        let thicker = FrameGeom {
            inner_pos: (110, 81),
            outer_size: (820, 639),
            ..GEOM
        };
        push(seen(true, Some(thicker)));
        assert_eq!(
            sent,
            [
                window_frame(&seen(false, None), frame_insets(&GEOM).unwrap()),
                window_frame(&seen(true, None), frame_insets(&GEOM).unwrap()),
                window_frame(&seen(true, None), frame_insets(&thicker).unwrap()),
            ]
        );
        assert_eq!(sent[0].restored, Some(SizeCss { w: 800.0, h: 600.0 }));
    }

    // If the front window changes it is emitted even if the other fields are the same — a burst of focus events with the same value is not emitted (TRD S26-window-chrome §6-5).
    #[test]
    fn a_focus_change_alone_sends_the_frame_once() {
        let book = FrameBook::default();
        let mut sent = Vec::new();
        let focused = |on: bool| FrameSeen {
            focused: on,
            ..seen(false, Some(GEOM))
        };
        for on in [false, true, true, false, false] {
            push_frame(&book, MAIN, focused(on), |f| sent.push(f.focused));
        }
        assert_eq!(sent, [false, true, false]);
        assert_eq!(
            pull_frame(&book, MAIN, focused(true)).caption,
            CAPTION,
            "a pull carries the chrome shape too"
        );
    }

    #[test]
    fn a_forgotten_window_sends_its_first_frame_again_and_others_are_kept() {
        let book = FrameBook::default();
        let mut sent = Vec::new();
        for label in ["slot-popup-1", MAIN] {
            push_frame(&book, label, seen(false, Some(GEOM)), |_| sent.push(label));
        }
        book.forget("slot-popup-1");
        // A restored popout is reopened with the same label — it is emitted even if the first value equals the old window's value.
        for label in ["slot-popup-1", MAIN] {
            push_frame(&book, label, seen(false, Some(GEOM)), |_| sent.push(label));
        }
        assert_eq!(sent, ["slot-popup-1", MAIN, "slot-popup-1"]);
        assert_eq!(
            pull_frame(&book, "slot-popup-1", seen(false, None)).insets,
            frame_insets(&GEOM).unwrap()
        );
        book.forget("slot-popup-1");
        assert_eq!(
            pull_frame(&book, "slot-popup-1", seen(false, None)).insets,
            NO_INSETS,
            "a pull of a forgotten window has frame 0"
        );
    }

    #[test]
    fn no_frame_goes_out_while_minimized_or_unreadable() {
        let book = FrameBook::default();
        let mut sent = 0;
        let minimized = FrameSeen {
            minimized: true,
            ..seen(false, Some(GEOM))
        };
        push_frame(&book, MAIN, minimized, |_| sent += 1);
        push_frame(&book, MAIN, seen(false, None), |_| sent += 1);
        assert_eq!(sent, 0);
        // It was not recorded either — the first value after it is released is emitted.
        push_frame(&book, MAIN, seen(false, Some(GEOM)), |_| sent += 1);
        assert_eq!(sent, 1);
        // A value that could not be read during minimize does not change the table, so after release the same value is not emitted again.
        push_frame(&book, MAIN, minimized, |_| sent += 1);
        push_frame(&book, MAIN, seen(false, Some(GEOM)), |_| sent += 1);
        assert_eq!(sent, 1);
    }

    #[test]
    fn the_frame_is_emitted_outside_the_table_lock_and_per_window() {
        let book = FrameBook::default();
        let mut sent = Vec::new();
        for label in [MAIN, "slot-popup-1"] {
            push_frame(&book, label, seen(false, Some(GEOM)), |_| {
                assert!(book.last.try_lock().is_ok(), "emitted after the table lock is released");
                sent.push(label);
            });
        }
        assert_eq!(sent, [MAIN, "slot-popup-1"], "counted separately per window");
    }

    #[test]
    fn the_pull_answers_the_model_values_with_fresh_insets_and_does_not_touch_the_table() {
        let book = FrameBook::default();
        assert_eq!(
            pull_frame(&book, MAIN, seen(true, Some(GEOM))),
            WindowFrame {
                restored: Some(SizeCss { w: 800.0, h: 600.0 }),
                maximized: true,
                insets: frame_insets(&GEOM).unwrap(),
                focused: false,
                caption: CAPTION,
            }
        );
        let unknown = FrameSeen {
            attrs: WindowAttrs::default(),
            ..seen(false, Some(GEOM))
        };
        assert_eq!(pull_frame(&book, MAIN, unknown).restored, None);
        // While minimized · unreadable = the last frame emitted (0 if none).
        let minimized = FrameSeen {
            minimized: true,
            ..seen(false, Some(GEOM))
        };
        assert_eq!(pull_frame(&book, MAIN, minimized).insets, NO_INSETS);
        let mut sent = 0;
        push_frame(&book, MAIN, seen(false, Some(GEOM)), |_| sent += 1);
        assert_eq!(
            pull_frame(&book, MAIN, minimized).insets,
            frame_insets(&GEOM).unwrap()
        );
        assert_eq!(
            pull_frame(&book, MAIN, seen(false, None)).insets,
            frame_insets(&GEOM).unwrap()
        );
        // A pull does not change the table — even after pulling a different value, a push of the same value is not emitted.
        let _ = pull_frame(&book, MAIN, seen(true, Some(GEOM)));
        push_frame(&book, MAIN, seen(false, Some(GEOM)), |_| sent += 1);
        assert_eq!(sent, 1);
    }

    #[test]
    fn the_new_window_inset_and_the_pushed_frame_insets_come_from_one_function() {
        let book = FrameBook::default();
        let mut pushed = None;
        push_frame(&book, MAIN, seen(false, Some(GEOM)), |f| {
            pushed = Some(f.insets)
        });
        let source = source_frame(Some(&GEOM));
        assert_eq!(Some(source.insets), pushed);
        assert_eq!(source.scale, GEOM.scale);
        assert_eq!(
            source_frame(None),
            SourceFrame {
                insets: NO_INSETS,
                scale: 1.0
            }
        );
    }
}
