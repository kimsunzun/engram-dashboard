//! The shell's share of tab drop (Tauri 0) — the ledger [`DropBook`] and processing [`handle`] of the drop command `tab_drop`, the
//! computation [`frame_insets`] of the window fact `window:frame` and emit-only-on-change [`push_frame`]. The Tauri wrapper · port adapters = `commands::tab_drop`. Decisions are in the pure
//! module [`plan`], wire types in [`wire`].
//!
//! ★Drag commands come only after the drag has ended (ADR-0296 · ADR-0297)★ — at most two per drag. Window facts
//! flow independent of dragging. Do not add here a path that sends the drag's decisions · geometry · state to the shell while dragging — that boundary synchronization is the root of the defect ADR-0296
//! removed.
//!
//! - ★The drop does not compare against the state it saw★ — the model finds again, by id, what the drop pointed at in the current state, applies it
//!   matched to the current state if it exists and rejects if not (rules = [`plan::plan`]).
//! - The model is written only through one `layout::apply::dock_tab`, the same as human · LLM commands (ADR-0081 · CLAUDE.md 「LLM-우선 제어」).
//!   This command is not put on the bus command table — the LLM command for the same action, T-65 `tab.dock`, stands on the same function.
//! - ★Lock discipline★ — the locks of the ledger · window-fact table are leaves. While holding one, do not take the layout lock · emit · call a window. The result notification
//!   ([`DropShell::done`]) · focus are called after all locks are released.
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

/// The number of drags the ledger remembers — when it overflows, it forgets the oldest of the finished records first. Records in progress (a new window in progress · its rejection notification
/// in progress) may hold a parked `drop`, so they are not forgotten. The two commands of one drag arrive back to back, so a forgotten drag is assumed not to come
/// again.
const BOOK_CAP: usize = 64;

/// Where the command came from — `Drop` = a drop in our webview (tab bar · seam · `new_window{drop}`) · `DragEnd` = the source's dragend.
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

// One drop command that has been through the decision — it holds together what processing uses, so the ledger can park it.
#[derive(Debug)]
struct Handling {
    drop: TabDrop,
    caller: String,
    input: InputNow,
    top_class: Option<String>,
    plan: Plan,
}

// The branch the ledger records per drag.
#[derive(Debug)]
enum Mark {
    // The drop was processed (applied · rejected · ignored, all of them · including a parked drop that was chosen to be processed next).
    Drop,
    // drag_end did not write the model — a new window that had no parked drop after cancel · ignore · reject notification.
    Quiet,
    // The new window of drag_end is in progress — it parks the first drop that arrives meanwhile.
    Placing(Option<Box<Handling>>),
    // The new window of drag_end was rejected with no parked drop and is emitting its `done{Rejected}` — like `Placing` it parks the first drop,
    //   and after the notification `close` hands it over. If it were received and processed before the notification, that drop's `done` could go before the rejection.
    Rejecting(Option<Box<Handling>>),
    // The new window of drag_end was created.
    Placed,
}

impl Mark {
    // A record that may hold a parked drop — not forgotten even if the ledger overflows.
    fn in_flight(&self) -> bool {
        matches!(self, Mark::Placing(_) | Mark::Rejecting(_))
    }
}

// The answer of [`DropBook::admit`].
#[derive(Debug)]
enum Admit {
    Run(Handling),
    // The drag_end new window of the same drag is in progress (including its rejection notification in progress), so it was parked — if that new window is rejected, that processing continues
    //   with it.
    Parked,
    Duplicate,
}

// The answer of [`DropBook::settle`].
#[derive(Debug)]
enum Settle {
    // Not the record of a new window in progress — nothing to do.
    Untracked,
    // The new window stood — if there was a parked drop, that (it is discarded).
    Placed(Option<Box<Handling>>),
    // The new window was rejected and there is a parked drop — this processing does not emit its own `done` and continues with that one.
    Superseded(Box<Handling>),
    // The new window was rejected with nothing parked — the record is `Rejecting`. This processing emits `done{Rejected}` and then calls `close`.
    Rejecting,
}

/// Recent drag ids and their processing branches — whatever order the `drag_end` · `drop` of one drag arrive in, the model is written at most once and
/// `drop` wins. The decision · recording are inside one lock and that lock is a leaf (held only inside `admit` · `settle` · `close`).
#[derive(Debug, Default)]
pub struct DropBook {
    seen: Mutex<VecDeque<(Uuid, Mark)>>,
    // Has it emitted the warn that records in progress alone grew it past the limit — emitted only once per ledger.
    overflow_warned: AtomicBool,
}

impl DropBook {
    // - A drag seen for the first time → process it.
    // - A `drop` but the earlier record is a `drag_end` that did not write the model (cancel · ignore · a rejected new window) → process it (record becomes `drop`).
    // - A `drop` but the same drag's `drag_end` new window is in progress or in its rejection notification → park it (only the first · handing it over is by
    //   `settle` · `close`).
    // - Anything else → `duplicate`: two `drop`s of the same drag · a `drag_end` after a `drop` · a `drop` after a `drag_end` that created a new window.
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
                "놓기 장부 — 상한의 기록이 모두 진행 중이라 잊지 않고 늘린다(이 장부에서 한 번만 알린다)"
            );
        }
        Admit::Run(h)
    }

    // Record the result of the `drag_end` new window in progress. If applied, record it as 「새 창을 만든 `drag_end`」 (the drag_end that created a new window) and return the parked `drop`
    //   (discarded — a later `drop` is also `duplicate`). If rejected and something was parked, that one is continued, so the record is `drop`; if nothing was, leave it as
    //   `Rejecting` so a `drop` that arrives while the rejection notification is emitted is also parked (`close` hands it over).
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

    // Called after emitting the rejection notification — ends `Rejecting`. If a `drop` was parked in the meantime, record `drop` and return
    //   it (continued processing — its `done` comes after the rejection). If none, lower it to a `drag_end` that did not write the model, so a `drop` that arrives later
    //   wins. If it is not `Rejecting`, it is a no-op (`None`).
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

// Forget finished records from the oldest as many as exceed the limit — the record just inserted (the very last) and records in progress are not forgotten.
//   `true` if it grew because there was nothing to forget.
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

/// The shell that processing reaches from outside the layout — both are called ★only after all locks are released★.
pub trait DropShell: Send + Sync {
    /// Give focus to the window that holds the moved tab — only when a drop on the tab bar · seam was applied (a new window is opened active). Usually the window dropped
    ///   on, and if the seam's target tab went to another window in the meantime, that window.
    fn focus(&self, label: &str);
    /// Emit the event `tab_drop:done` only to the source window (`source` = the command's `from`) — if that window does not exist (closed empty, etc.), skip it.
    fn done(&self, source: &str, done: &TabDropDone);
}

/// The ports [`handle`] uses — the first five are exactly those of `layout::apply::dock_tab`.
pub struct DropPorts<'a> {
    pub state: &'a LayoutState,
    pub subs: &'a dyn SubscriptionSync,
    pub events: &'a dyn LayoutEvents,
    pub host: &'a dyn WindowHost,
    pub labels: &'a dyn LabelSource,
    pub shell: &'a dyn DropShell,
}

/// Process one drop command — decision ([`plan::plan`]) → ledger → apply (one `dock_tab`) → focus → result notification → one log line.
/// The input (`input`) and the source window frame (`frame`) are read by the caller before this, in that order (input first). `caller` = the window that called the command ·
/// `top_class` = the window class when the topmost is another app's window (for the log).
///
/// Receiving the same drag again is `Duplicate` with no notification. A `drop` that arrives while the same drag's `drag_end` new window is in progress (including while its rejection notification
/// is being emitted) is also answered `Duplicate`, but the ledger parks it — if that new window is rejected, that processing continues the parked `drop`
/// by the same path as an ordinary `drop` (its result goes only by `done`), and if the new window stands, the parked drop is discarded. If something was already parked at the rejection decision,
/// that `drag_end` processing does not emit its own `done` (log only · `superseded`) — the `done` for that drag that the source receives is
/// that `drop`'s alone. If it was parked after the rejection notification, that `done` follows the rejection. Otherwise it emits `done` once per
/// result — for an apply it comes after `dock_tab`'s model notification. A rejection leaves the model as is, and the `kind` of `Rejected{kind}` = the word in the parentheses of the docking error text.
/// Cancel · ignore do not write the model.
///
/// ★Do not call `new_window` from the main thread · a synchronous command★ — the new-window build deadlocks (`dock_tab` docs).
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
                "놓기 — 같은 끌기의 새 창이 진행 중이라 맡겨 둔다(그 새 창이 거절되면 이어 처리)"
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
                "놓기 — 같은 끌기를 이미 처리해 버린다"
            );
            return TabDropReply::Duplicate;
        }
    };
    let (reply, parked) = run(book, ports, item);
    if let Some(parked) = parked {
        // The call of the parked `drop` was already answered `Duplicate` — this processing's answer has nowhere to go.
        let _ = run(book, ports, *parked);
    }
    reply
}

// One received processing — apply → focus → result notification → log. If this processing is a rejected `drag_end` new window and the ledger has a parked `drop`, it returns
//   that (to be continued). If something was already parked at the rejection decision, this processing does not emit its own notification — if the rejection
//   `done` went first, the source would undo the swapped drawing and the dragged tab would flash (TRD §2-4 ①). If nothing was parked, after emitting the rejection notification
//   `close` returns what was parked in the meantime — its `done` comes after the rejection.
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
        // Close only after emitting the rejection notification — so that the `done` of a `drop` that arrived in the meantime does not go before the rejection.
        Settle::Rejecting => book.close(drop.drag),
        Settle::Placed(Some(parked)) => {
            tracing::debug!(
                module = "dock",
                drag = %parked.drop.drag,
                view = %parked.drop.view,
                caller = %parked.caller,
                "놓기 — 같은 끌기의 새 창이 섰다 — 맡겨 둔 drop 을 버린다"
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

// Where the dropped new window came from — a prewarmed window handed over · a newly built window · the source window moved with no new window. The prewarm meaning is not layered on
//   top of reuse (TRD S21-slot-docking §3-2 ⑦ · §3-9 「로그」).
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
        "놓기 처리"
    );
}

// The word in the last parentheses of the docking error text — text = reason + ` (종류)` (`DockError` docs). Parentheses inside the reason come before it.
fn kind_of(e: &DockError) -> String {
    let text = e.to_string();
    text.rsplit_once('(')
        .and_then(|(_, tail)| tail.strip_suffix(')'))
        .unwrap_or("Unknown")
        .to_owned()
}

// ── Window fact `window:frame` (TRD S21-slot-docking §3-8) ──────────────────────────

/// The OS geometry of one window — the origin and size of the outer and inner (physical px — exactly the four Tauri getters) and that window's scale factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameGeom {
    pub outer_pos: (i32, i32),
    pub outer_size: (u32, u32),
    pub inner_pos: (i32, i32),
    pub inner_size: (u32, u32),
    pub scale: f64,
}

/// Window frame = outer rectangle − inner rectangle (per side · CSS px — divided by that window's scale factor). The `insets` of `window:frame` and the `inset_dip`
/// of the new-window placement ([`source_frame`]) are computed with this one. `None` if the scale factor is not a finite positive number.
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

/// The source window's frame — if the geometry could not be read, frame 0 · scale factor 1 (the new window is merely off by the title-bar height).
pub fn source_frame(geom: Option<&FrameGeom>) -> SourceFrame {
    match geom.and_then(|g| frame_insets(g).map(|insets| (insets, g.scale))) {
        Some((insets, scale)) => SourceFrame { insets, scale },
        None => SourceFrame {
            insets: NO_INSETS,
            scale: 1.0,
        },
    }
}

/// One window fact — the restored size · maximize are the model's (`WindowAttrs`) values, and the front window · chrome shape are the values read as is.
pub fn window_frame(seen: &FrameSeen, insets: Insets) -> WindowFrame {
    WindowFrame {
        restored: seen.attrs.bounds.map(|b| SizeCss { w: b.w(), h: b.h() }),
        maximized: seen.attrs.maximized,
        insets,
        focused: seen.focused,
        caption: seen.caption,
    }
}

/// One window as read now — `minimized` · `geom` are getters (read holding no lock), `attrs` is that window's model value,
/// `focused` is the value of that window's focus event or the OS foreground-window decision, `caption` is an OS constant (TRD S26-window-chrome §6-5).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameSeen {
    pub minimized: bool,
    pub attrs: WindowAttrs,
    pub geom: Option<FrameGeom>,
    pub focused: bool,
    pub caption: CaptionChrome,
}

/// The last `window:frame` emitted per window — so it is emitted only when the value changes. The lock is a leaf. A closed window's entry is deleted when that window vanishes with
/// [`FrameBook::forget`].
#[derive(Debug, Default)]
pub struct FrameBook {
    last: Mutex<HashMap<String, WindowFrame>>,
}

impl FrameBook {
    fn last(&self, label: &str) -> Option<WindowFrame> {
        sync::lock(&self.last).get(label).copied()
    }

    /// Forget that window's last value — called after the window vanishes. A restored popout is reopened with its saved label, so if left, the
    /// first value of that window would not go out when it equals the old window's value.
    pub fn forget(&self, label: &str) {
        sync::lock(&self.last).remove(label);
    }
}

/// Called after the placement-recording path (`Moved` · `Resized` · the window port's `record_placement`) wrote to the model · by that window's focus event (`Focused`) —
/// only when that window's value (including the front window) differs from the last emitted one, it calls `emit` once, after releasing the table lock. While minimized ·
/// if the geometry could not be read, it neither emits nor records (a getter while minimized gives an off-screen value). The burst of `Moved` while moving a window by its title bar
/// has the same value, so it is not emitted.
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

/// The answer of `window_frame`, pulled at mount — the restored size · maximize are model values, and the frame is the value computed with the current getters. While minimized ·
/// if the geometry could not be read, it carries the last frame emitted for that window (0 if none). It is not recorded in the table.
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
        // The spot to push in the same drag's drop · the ledger · that drop — causes a race in the middle of processing the new-window drag_end.
        racing: Mutex<Option<(RaceAt, Arc<DropBook>, TabDrop)>>,
        // The answer of the drop that arrived that way.
        racing_replies: Mutex<Vec<TabDropReply>>,
        // The prewarmed window the window port will hand over once.
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
                "{who} 는 레이아웃 락을 놓은 뒤에 불려야 한다"
            );
        }

        fn take_log(&self) -> Vec<String> {
            std::mem::take(&mut *self.log.lock().unwrap())
        }

        fn mgr(&self) -> std::sync::MutexGuard<'_, ViewManager> {
            self.state.0.lock().unwrap()
        }

        // If the spot to push in is `at`, process the parked drop now (only once).
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
                Err("창을 못 만듦".into())
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

    // The spot that pushes in the same drag's drop in the middle of processing the new-window drag_end.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum RaceAt {
        // While opening the new window — before recording its result in the ledger.
        Open,
        // While emitting the first `done` to the source — if rejected, the ledger is `Rejecting` and it is before `close`.
        Done,
    }

    // (field name, value) for each event emitted on this thread — the wording is the `message` field.
    #[derive(Clone, Default)]
    struct Logs(Arc<Mutex<Vec<Vec<(String, String)>>>>);

    impl Logs {
        // The `name` field of each 「놓기 처리」 line — `None` if absent.
        fn handled(&self, name: &str) -> Vec<Option<String>> {
            let field = |line: &[(String, String)], name: &str| {
                line.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone())
            };
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter(|line| field(line, "message").as_deref() == Some("놓기 처리"))
                .map(|line| field(line, name))
                .collect()
        }
    }

    struct Fields<'a>(&'a mut Vec<(String, String)>);

    impl tracing::field::Visit for Fields<'_> {
        // String fields without quotes — written with Debug, quotes would be attached like `"fresh"`.
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

    // An answer that it moved to in-model window `window` — the identity is read from that window's entry.
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
            "같은 끌기의 drop 둘"
        );
        assert_eq!(
            admit(&book, id(1), Arrival::DragEnd, Plan::Cancel),
            "duplicate",
            "drop 뒤 drag_end"
        );
        assert_eq!(admit(&book, id(2), Arrival::DragEnd, Plan::Cancel), "run");
        assert_eq!(
            admit(&book, id(2), Arrival::DragEnd, Plan::Cancel),
            "duplicate",
            "같은 drag_end 두 번 배달"
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
                "{quiet:?} 뒤 drop"
            );
            assert_eq!(
                admit(&book, id(1), Arrival::Drop, plan_dock()),
                "duplicate",
                "기록이 drop 으로 바뀌었다"
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
            "맡는 것은 첫 drop 하나"
        );
        assert_eq!(settle(&book, id(1), DropOutcome::Applied), "placed+parked");
        assert_eq!(admit(&book, id(1), Arrival::Drop, plan_dock()), "duplicate");
        assert_eq!(
            settle(&book, id(1), DropOutcome::Rejected),
            "untracked",
            "끝난 새 창은 다시 settle 되지 않는다"
        );

        // There is a parked drop but the new window was rejected — that one stands in for the rejection, so a later drop is duplicate.
        assert_eq!(admit(&book, id(3), Arrival::DragEnd, plan_dock()), "run");
        assert_eq!(admit(&book, id(3), Arrival::Drop, plan_dock()), "parked");
        let Settle::Superseded(parked) = book.settle(id(3), DropOutcome::Rejected) else {
            panic!("맡은 drop 이 거절을 대신한다");
        };
        assert_eq!(parked.drop.drag, id(3));
        assert_eq!(admit(&book, id(3), Arrival::Drop, plan_dock()), "duplicate");
        assert!(book.close(id(3)).is_none(), "대신한 거절은 닫을 것이 없다");
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
            "같은 drag_end 두 번 배달"
        );
        assert_eq!(admit(&book, id(1), Arrival::Drop, plan_dock()), "parked");
        assert_eq!(
            admit(&book, id(1), Arrival::Drop, plan_dock()),
            "duplicate",
            "맡는 것은 첫 drop 하나"
        );
        let parked = book.close(id(1)).expect("맡은 drop");
        assert_eq!(parked.drop.drag, id(1));
        assert_eq!(
            admit(&book, id(1), Arrival::Drop, plan_dock()),
            "duplicate",
            "넘긴 drop 을 이어 처리한다"
        );
        assert!(
            book.close(id(1)).is_none(),
            "닫힌 기록은 다시 닫히지 않는다"
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
            "65 번째가 가장 오래된 것을 밀어냈다"
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
            "진행 중인 새 창은 잊지 않았다"
        );
        assert_eq!(
            admit(&book, id(1001), Arrival::Drop, plan_dock()),
            "parked",
            "거절 알림 중인 새 창도"
        );
        assert_eq!(
            admit(&book, id(0), Arrival::Drop, plan_dock()),
            "run",
            "끝난 기록 중 가장 오래된 것이 밀려났다"
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
            "모두 진행 중이면 늘린다"
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
            "끝난 뒤 다음 기록이 상한으로 되돌린다"
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
                DockError::WindowOpen("창 옮기기 Position 실패 — 접근 거부 (os error 5)".into()),
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
            "끄는 탭이 그사이 사라졌다"
        );
        assert_eq!(rig.mgr().version, before);
        assert_eq!(rig.mgr().list_tabs(MAIN).unwrap().tabs.len(), 2);
        assert_eq!(rig.take_log(), ["done:Rejected"], "알림 · 포커스 없음");
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
            "모델 · 창 · 구독 어느 포트도 불리지 않는다"
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
            "기존 창(탭바)도 그 창의 신원을 싣는다"
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
            .expect("모델 알림");
        let focus = log.iter().position(|s| s == "focus:main").expect("포커스");
        assert!(notified < focus, "모델 알림 → 포커스 → done: {log:?}");
        assert_eq!(log.iter().filter(|s| s.starts_with("done:")).count(), 1);

        assert_eq!(
            rig.handle(&book, reorder, MAIN, &released(TopAt::Unknown)),
            TabDropReply::Duplicate
        );
        assert!(rig.take_log().is_empty(), "중복은 알림도 없다");
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
            "거절 알림 · close 뒤에 닿은 drop 은 보통 drop 처럼 이긴다"
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
            "done 은 부른 창이 아니라 출처 창(from)에만 간다"
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
        assert_eq!(rig.mgr().owner_of(a), None, "a 는 b 에 붙어 사라졌다");
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
            "새 창이 진행 중인 동안 닿은 drop 은 맡기고 곧바로 답한다"
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
            "이어 처리한 drop 뒤의 같은 끌기 drop"
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
        assert_eq!(main_tabs(&rig), [b, a], "맡겨 둔 drop 이 적용됐다");
        assert_eq!(
            rig.outcomes(),
            [DropOutcome::Applied],
            "거절된 drag_end 는 done 을 내지 않는다 — 출처가 받는 done 은 drop 의 것 하나"
        );
        assert_eq!(
            logs.handled("superseded"),
            [Some("true".to_owned()), None],
            "drag_end 처리 줄은 남기되 자리를 내줬다고 적는다"
        );
        let log = rig.take_log();
        let focus = log.iter().position(|s| s == "focus:main").expect("포커스");
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
            "거절 알림 뒤 맡겨 둔 drop 이 적용됐다"
        );
        assert_eq!(
            rig.outcomes(),
            [DropOutcome::Rejected, DropOutcome::Applied],
            "거절 done 이 먼저 · 그 끌기의 마지막 done 은 drop 의 것"
        );
        assert_eq!(logs.handled("superseded"), [None, None]);
        let log = rig.take_log();
        let rejected = log.iter().position(|s| s == "done:Rejected").unwrap();
        let focus = log.iter().position(|s| s == "focus:main").expect("포커스");
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
        assert_eq!(ids(MAIN), [b], "맡겨 둔 drop 은 모델을 쓰지 않았다");
        assert_eq!(
            rig.outcomes(),
            [DropOutcome::Applied],
            "맡겨 둔 drop 은 알림도 없다"
        );
    }

    // ── ⑦ `via` of the drop-processing line (TRD §3-2 ⑦ · §3-9 「로그」 · ADR-0299) ─────────────────────────

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
        let [via] = logs.handled("via").try_into().expect("놓기 처리 줄 하나");
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
            "맡긴 창의 label 로 연다"
        );
        let log = prewarmed.take_log();
        let at = |entry: &str| log.iter().position(|s| s == entry).expect(entry);
        let notified = log
            .iter()
            .rposition(|s| s == "layout" || s.starts_with("tabs:"))
            .expect("모델 알림");
        assert!(
            at("claim:slot-popup-9") < at("open:slot-popup-9")
                && at("open:slot-popup-9") < notified
                && notified < at("committed:slot-popup-9"),
            "맡기기 → 열기 → 알림 → 확정: {log:?}"
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
        assert_eq!(logs.handled("via"), [None], "새 창이 아닌 놓기에는 없다");
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
                assert_eq!(window, label, "맡긴 창");
            }
            assert_eq!(rig.mgr().owner_of(a), Some(window), "{spare:?}");
            assert!(Uuid::parse_str(window_id).is_ok(), "{window_id}");
            assert_eq!(*number, 2, "{spare:?}: main 다음 첫 번호");
            assert_eq!(reply, applied(&rig, window, false), "{spare:?}");
        }

        let rig = Rig::new();
        let lone = rig.mgr().create_window("slot-popup-5").unwrap();
        let (_, reply) = new_window_drop_via(&rig, lone, "slot-popup-5");
        assert_eq!(reply, applied(&rig, "slot-popup-5", true), "재사용");
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
            "당기기도 크롬 모양을 싣는다"
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
            "잊은 창의 당기기는 틀 0 이다"
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
                assert!(book.last.try_lock().is_ok(), "표 락을 놓은 뒤에 낸다");
                sent.push(label);
            });
        }
        assert_eq!(sent, [MAIN, "slot-popup-1"], "창마다 따로 센다");
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
