//! The pure decision of the drop command — from one command · the input read after the drag ended · the source window frame, it picks the docking request to apply, or cancel · ignore.
//! Zero Tauri · locks · OS calls, so it can be tested alone (ADR-0012). Entry point = [`plan`] · the new-window distinction inside it [`decide`] ·
//! the new-window placement [`new_window_origin`].
// ADR-0296 · ADR-0297
// TRD S21-slot-docking §3-3 · §3-5

use super::wire::{DropAnchor, DropTo, EndEffect, NewWindowBy, TabDrop};
use crate::layout::apply::{DockDest, DockRequest, WindowPlace};
use crate::layout::bands::{PointCss, SizeCss};
use crate::layout::dock::DockAnchor;
use crate::layout::geometry::Insets;
use crate::layout::manager::{DockMove, FocusPolicy, WindowLabel};

/// The ignore reason (the spelling of `TabDropReply::Ignored.why`).
pub const NO_CURSOR: &str = "no-cursor";
pub const LOST_DROP: &str = "lost-drop";
pub const ACCEPTED_OURS: &str = "accepted-ours";
pub const ACCEPTED_UNKNOWN: &str = "accepted-unknown";

/// A physical-px point on the screen (virtual-screen coordinates — origin at the primary monitor's top-left · negative for a monitor to the left · above). The adapter converts the same-named
/// type of the OS-layer crate — so this module does not lean on that crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PhysPoint {
    pub x: i32,
    pub y: i32,
}

/// The topmost top-level window under the cursor (z-order — invisible · minimized · see-through windows skipped).
///
/// - `Ours`: our webview window (that label) — the frame outside the webview (title bar · border) is this too.
/// - `Foreign`: another app's window · the desktop.
/// - `Unknown`: could not be read (cursor unreadable · outside Windows · the OS did not answer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopAt {
    Ours(WindowLabel),
    Foreign,
    Unknown,
}

/// The input the shell read after the drag ended — read right after receiving the command, before the lock · ledger (so it is the same moment as the drop point).
///
/// - `cursor`: `None` = could not be read.
/// - `primary_down` · `escape_down`: are they pressed at that moment. A `drop` command does not read them (false).
/// - `top`: only a `drag_end` command reads it (`drop` is `Unknown`).
/// - `scale`: the scale factor of the monitor under the cursor — if unreadable, it is the source window's scale factor. If it is not a finite positive number, [`SourceFrame::scale`] is used.
#[derive(Debug, Clone, PartialEq)]
pub struct InputNow {
    pub cursor: Option<PhysPoint>,
    pub primary_down: bool,
    pub escape_down: bool,
    pub top: TopAt,
    pub scale: f64,
}

/// The frame of the source window (the window of the dragged tab) — `insets` = window frame (CSS px · computed the same as `window:frame` [`super::frame_insets`]) · `scale` =
/// that window's scale factor. The new window's frame is taken to be the same as the source window's (the frames of popouts and main are the same).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceFrame {
    pub insets: Insets,
    pub scale: f64,
}

/// The answer of [`decide`] — `Place` = a new window at that cursor spot (or move a popout that has only one tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Cancel,
    Ignore(&'static str),
    Place(PhysPoint),
}

/// The answer of [`plan`] — only `Dock` writes the model.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    Dock(DockRequest),
    Cancel,
    Ignore(&'static str),
}

/// Convert one drop command into something to apply. `caller` = the window that called the command — the window of a tab-bar drop (not `drop.from`).
///
/// Every docking request is `FocusPolicy::Reveal`, and the next active (`mv.successor`) is `initial` only when `initial ≠ view`,
/// and is used only when the dragged tab is in the `from` window at apply time (`mv.successor_window = from` — if it is in another window,
/// that window follows the shell succession). The new window's size is carried only when it is a finite positive number.
///
/// What the drop pointed at (the dragged tab · target tab · target area · tab-bar window) is found again by the model by id in the current state — if present, it is applied matched to the current
/// state (a tab-bar position is clipped to the end of the current list · it also attaches to an area split in the meantime · a dragged tab that went to another window in the meantime is taken out of its current window), and if absent it is rejected
/// (model unchanged). ★Do not revive the cross-check that rejects by comparing the tab list · revision number · active tab · source window the drop saw★ —
/// if the result is slightly off, the user can drag again, and the cross-check only adds exceptions to reject.
// ADR-0297
// TRD S21-slot-docking §3-3
pub fn plan(drop: &TabDrop, caller: &str, input: &InputNow, frame: &SourceFrame) -> Plan {
    let successor = drop.initial.filter(|initial| *initial != drop.view);
    let dock = |to: DockDest| {
        Plan::Dock(DockRequest {
            view: drop.view,
            to,
            focus: FocusPolicy::Reveal,
            mv: DockMove {
                successor,
                successor_window: Some(drop.from.clone()),
            },
        })
    };
    match &drop.to {
        DropTo::TabBar { index } => dock(DockDest::TabBar {
            window: caller.to_owned(),
            index: Some(*index as usize),
            after: None,
        }),
        DropTo::Seam {
            target,
            anchor,
            side,
            share,
        } => dock(DockDest::Seam {
            target: *target,
            anchor: match anchor {
                DropAnchor::Slot { id } => DockAnchor::Slot(*id),
                DropAnchor::Outer => DockAnchor::Outer,
            },
            side: *side,
            share: *share,
        }),
        DropTo::NewWindow { by, size, grab } => match decide(by, input) {
            Decision::Cancel => Plan::Cancel,
            Decision::Ignore(why) => Plan::Ignore(why),
            Decision::Place(cursor) => {
                let scale = [input.scale, frame.scale]
                    .into_iter()
                    .find(|s| s.is_finite() && *s > 0.0)
                    .unwrap_or(1.0);
                let pos = new_window_origin(cursor, scale, (frame.insets.l, frame.insets.t), *grab);
                dock(DockDest::NewWindow {
                    at: WindowPlace {
                        pos: Some(pos),
                        size: size.and_then(usable_size),
                    },
                })
            }
        },
    }
}

/// Distinguish a new-window drop — the order below is the contract (the earlier wins). If the cursor could not be read, the topmost window is also treated as unread.
///
/// 1. `drop` → `Place` at the cursor spot (no cursor = `Ignore(no-cursor)`).
/// 2. `drag_end{none}` + (Esc pressed ∨ primary button pressed ∨ no cursor) → `Cancel`. `move` skips this line (somewhere received it).
/// 3. `drag_end` + topmost is our window → `Ignore` — `none` = `lost-drop` · `move` = `accepted-ours` (the receiving window's `drop` comes
///    separately). The title bar · border are also our window, so nothing happens.
/// 4. `drag_end` + another app · the desktop (both `none` · `move`) · `drag_end{none}` + unreadable → `Place`.
/// 5. `drag_end{move}` + unreadable → `Ignore(accepted-unknown)`.
// TRD S21-slot-docking §3-5
pub fn decide(by: &NewWindowBy, i: &InputNow) -> Decision {
    let effect = match by {
        NewWindowBy::Drop => {
            return i
                .cursor
                .map_or(Decision::Ignore(NO_CURSOR), Decision::Place)
        }
        NewWindowBy::DragEnd { effect } => *effect,
    };
    if effect == EndEffect::None && (i.escape_down || i.primary_down || i.cursor.is_none()) {
        return Decision::Cancel;
    }
    let Some(cursor) = i.cursor else {
        return Decision::Ignore(ACCEPTED_UNKNOWN);
    };
    match (&i.top, effect) {
        (TopAt::Ours(_), EndEffect::None) => Decision::Ignore(LOST_DROP),
        (TopAt::Ours(_), EndEffect::Move) => Decision::Ignore(ACCEPTED_OURS),
        (TopAt::Foreign, _) | (TopAt::Unknown, EndEffect::None) => Decision::Place(cursor),
        (TopAt::Unknown, EndEffect::Move) => Decision::Ignore(ACCEPTED_UNKNOWN),
    }
}

/// The new window's outer top-left (physical px) = `cursor − round((inset_dip + grab) × scale)` — the cursor lands at `grab` of the new window's viewport.
/// `inset_dip` = the window frame's (left, top) (CSS px) · `scale` = the scale factor of the monitor dropped on. Rounding takes .5 to the side away from 0.
// TRD S21-slot-docking §3-5
pub fn new_window_origin(
    cursor: PhysPoint,
    scale: f64,
    inset_dip: (f64, f64),
    grab: PointCss,
) -> (i32, i32) {
    let back = |inset: f64, at: f64| ((inset + at) * scale).round() as i32;
    (
        cursor.x.saturating_sub(back(inset_dip.0, grab.x)),
        cursor.y.saturating_sub(back(inset_dip.1, grab.y)),
    )
}

fn usable_size(size: SizeCss) -> Option<(f64, f64)> {
    let SizeCss { w, h } = size;
    (w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0).then_some((w, h))
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::layout::dock::DockSide;

    const MAIN: &str = "main";
    const POPUP: &str = "slot-popup-2";

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn at(x: i32, y: i32) -> PhysPoint {
        PhysPoint { x, y }
    }

    fn input(cursor: Option<PhysPoint>, button: bool, esc: bool, top: TopAt) -> InputNow {
        InputNow {
            cursor,
            primary_down: button,
            escape_down: esc,
            top,
            scale: 1.0,
        }
    }

    fn quiet(top: TopAt) -> InputNow {
        input(Some(at(500, 400)), false, false, top)
    }

    const FLAT: SourceFrame = SourceFrame {
        insets: Insets {
            t: 0.0,
            r: 0.0,
            b: 0.0,
            l: 0.0,
        },
        scale: 1.0,
    };

    fn drop_to(from: &str, initial: Option<Uuid>, to: DropTo) -> TabDrop {
        TabDrop {
            drag: id(100),
            view: id(1),
            from: from.into(),
            initial,
            to,
        }
    }

    fn request(p: Plan) -> DockRequest {
        match p {
            Plan::Dock(req) => req,
            other => panic!("도킹 요청이어야: {other:?}"),
        }
    }

    fn seam(anchor: DropAnchor) -> DropTo {
        DropTo::Seam {
            target: id(7),
            anchor,
            side: DockSide::Bottom,
            share: 0.3,
        }
    }

    fn new_window(by: NewWindowBy, size: Option<SizeCss>) -> DropTo {
        DropTo::NewWindow {
            by,
            size,
            grab: PointCss { x: 0.0, y: 0.0 },
        }
    }

    // ── ① Correspondence (§3-3) ──────────────────────────────────────────────────────────

    #[test]
    fn a_tab_bar_drop_targets_the_calling_window_without_preconditions() {
        let d = drop_to(MAIN, Some(id(3)), DropTo::TabBar { index: 2 });
        let req = request(plan(&d, POPUP, &quiet(TopAt::Unknown), &FLAT));
        assert_eq!(
            req,
            DockRequest {
                view: id(1),
                to: DockDest::TabBar {
                    window: POPUP.into(),
                    index: Some(2),
                    after: None,
                },
                focus: FocusPolicy::Reveal,
                mv: DockMove {
                    successor: Some(id(3)),
                    successor_window: Some(MAIN.into()),
                },
            },
            "탭바 창 = 부른 창(인자의 from 이 아니다)"
        );
    }

    #[test]
    fn the_successor_is_the_initial_tab_only_when_it_is_not_the_dragged_tab() {
        let bar = || DropTo::TabBar { index: 0 };
        for (initial, want) in [
            (Some(id(3)), Some(id(3))),
            (Some(id(1)), None),
            (None, None),
        ] {
            let req = request(plan(
                &drop_to(MAIN, initial, bar()),
                MAIN,
                &quiet(TopAt::Unknown),
                &FLAT,
            ));
            assert_eq!(req.mv.successor, want, "{initial:?}");
            assert_eq!(
                req.mv.successor_window.as_deref(),
                Some(MAIN),
                "from 창에서만"
            );
        }
    }

    #[test]
    fn a_seam_drop_maps_the_anchor_without_preconditions_whatever_the_calling_window() {
        for caller in [MAIN, POPUP] {
            let req = request(plan(
                &drop_to(MAIN, Some(id(3)), seam(DropAnchor::Slot { id: id(9) })),
                caller,
                &quiet(TopAt::Unknown),
                &FLAT,
            ));
            assert_eq!(
                req.to,
                DockDest::Seam {
                    target: id(7),
                    anchor: DockAnchor::Slot(id(9)),
                    side: DockSide::Bottom,
                    share: 0.3,
                },
                "{caller}"
            );
            assert_eq!(req.focus, FocusPolicy::Reveal);
            assert_eq!(req.mv.successor, Some(id(3)));
        }

        let outer = request(plan(
            &drop_to(MAIN, None, seam(DropAnchor::Outer)),
            MAIN,
            &quiet(TopAt::Unknown),
            &FLAT,
        ));
        assert!(matches!(
            outer.to,
            DockDest::Seam {
                anchor: DockAnchor::Outer,
                ..
            }
        ));
    }

    #[test]
    fn a_new_window_drop_places_at_the_origin_without_preconditions() {
        let size = SizeCss { w: 800.0, h: 600.0 };
        let d = drop_to(
            MAIN,
            Some(id(3)),
            DropTo::NewWindow {
                by: NewWindowBy::Drop,
                size: Some(size),
                grab: PointCss { x: 20.0, y: 10.0 },
            },
        );
        let frame = SourceFrame {
            insets: Insets {
                t: 31.0,
                r: 8.0,
                b: 8.0,
                l: 8.0,
            },
            scale: 1.0,
        };
        let req = request(plan(&d, POPUP, &quiet(TopAt::Unknown), &frame));
        assert_eq!(
            req.to,
            DockDest::NewWindow {
                at: WindowPlace {
                    pos: Some((500 - 28, 400 - 41)),
                    size: Some((800.0, 600.0)),
                },
            }
        );
        assert_eq!(req.mv.successor, Some(id(3)));
        assert_eq!(req.focus, FocusPolicy::Reveal);
    }

    #[test]
    fn a_missing_or_broken_new_window_size_leaves_the_size_to_the_shell() {
        let broken = [
            None,
            Some(SizeCss { w: 0.0, h: 600.0 }),
            Some(SizeCss { w: 800.0, h: -1.0 }),
            Some(SizeCss {
                w: f64::NAN,
                h: 600.0,
            }),
            Some(SizeCss {
                w: 800.0,
                h: f64::INFINITY,
            }),
        ];
        for size in broken {
            let req = request(plan(
                &drop_to(MAIN, None, new_window(NewWindowBy::Drop, size)),
                MAIN,
                &quiet(TopAt::Unknown),
                &FLAT,
            ));
            let DockDest::NewWindow { at } = req.to else {
                panic!("새 창이어야");
            };
            assert_eq!(at.size, None, "{size:?}");
        }
    }

    #[test]
    fn a_new_window_that_decides_cancel_or_ignore_plans_no_write() {
        let ended = |effect| NewWindowBy::DragEnd { effect };
        let canceled = plan(
            &drop_to(MAIN, None, new_window(ended(EndEffect::None), None)),
            MAIN,
            &input(Some(at(1, 1)), true, false, TopAt::Foreign),
            &FLAT,
        );
        assert_eq!(canceled, Plan::Cancel);
        let ignored = plan(
            &drop_to(MAIN, None, new_window(ended(EndEffect::Move), None)),
            MAIN,
            &quiet(TopAt::Ours(POPUP.into())),
            &FLAT,
        );
        assert_eq!(ignored, Plan::Ignore(ACCEPTED_OURS));
    }

    #[test]
    fn a_bad_cursor_monitor_scale_falls_back_to_the_source_window_scale() {
        let d = drop_to(
            MAIN,
            None,
            DropTo::NewWindow {
                by: NewWindowBy::Drop,
                size: None,
                grab: PointCss { x: 10.0, y: 10.0 },
            },
        );
        let frame = SourceFrame { scale: 2.0, ..FLAT };
        for bad in [0.0, -1.0, f64::NAN] {
            let i = InputNow {
                scale: bad,
                ..quiet(TopAt::Unknown)
            };
            let DockDest::NewWindow { at } = request(plan(&d, MAIN, &i, &frame)).to else {
                panic!("새 창이어야");
            };
            assert_eq!(at.pos, Some((480, 380)), "{bad}");
        }
    }

    // ── ② decide (the order of the §3-5 table) ──────────────────────────────────────────────

    fn tops() -> [TopAt; 3] {
        [TopAt::Ours(POPUP.into()), TopAt::Foreign, TopAt::Unknown]
    }

    const PRESSES: [(bool, bool); 4] = [(false, false), (true, false), (false, true), (true, true)];

    #[test]
    fn a_drop_places_at_the_cursor_whatever_the_buttons_and_top_and_ignores_without_a_cursor() {
        for top in tops() {
            for (button, esc) in PRESSES {
                let c = at(-30, 70);
                assert_eq!(
                    decide(
                        &NewWindowBy::Drop,
                        &input(Some(c), button, esc, top.clone())
                    ),
                    Decision::Place(c)
                );
                assert_eq!(
                    decide(&NewWindowBy::Drop, &input(None, button, esc, top.clone())),
                    Decision::Ignore(NO_CURSOR)
                );
            }
        }
    }

    #[test]
    fn a_none_drag_end_with_a_button_or_escape_held_cancels_on_every_top() {
        let by = NewWindowBy::DragEnd {
            effect: EndEffect::None,
        };
        for top in tops() {
            for (button, esc) in PRESSES.into_iter().filter(|p| *p != (false, false)) {
                assert_eq!(
                    decide(&by, &input(Some(at(5, 5)), button, esc, top.clone())),
                    Decision::Cancel,
                    "{top:?} {button} {esc}"
                );
            }
        }
    }

    #[test]
    fn a_none_drag_end_with_both_released_ignores_over_our_window_and_places_elsewhere() {
        let by = NewWindowBy::DragEnd {
            effect: EndEffect::None,
        };
        let c = at(5, 5);
        // The title bar · border are also our window — nothing happens (Q38).
        assert_eq!(
            decide(&by, &input(Some(c), false, false, TopAt::Ours(MAIN.into()))),
            Decision::Ignore(LOST_DROP)
        );
        assert_eq!(
            decide(&by, &input(Some(c), false, false, TopAt::Foreign)),
            Decision::Place(c)
        );
        assert_eq!(
            decide(&by, &input(Some(c), false, false, TopAt::Unknown)),
            Decision::Place(c)
        );
    }

    #[test]
    fn a_move_drag_end_skips_the_cancel_row_and_reads_only_the_top() {
        let by = NewWindowBy::DragEnd {
            effect: EndEffect::Move,
        };
        let c = at(5, 5);
        for (button, esc) in PRESSES {
            let at_top = |top| decide(&by, &input(Some(c), button, esc, top));
            assert_eq!(
                at_top(TopAt::Ours(MAIN.into())),
                Decision::Ignore(ACCEPTED_OURS)
            );
            assert_eq!(
                at_top(TopAt::Foreign),
                Decision::Place(c),
                "다른 앱이 받아도 새 창(Q39)"
            );
            assert_eq!(at_top(TopAt::Unknown), Decision::Ignore(ACCEPTED_UNKNOWN));
        }
    }

    #[test]
    fn a_drag_end_without_a_cursor_cancels_when_none_and_falls_to_accepted_unknown_when_move() {
        for top in tops() {
            for (button, esc) in PRESSES {
                assert_eq!(
                    decide(
                        &NewWindowBy::DragEnd {
                            effect: EndEffect::None
                        },
                        &input(None, button, esc, top.clone())
                    ),
                    Decision::Cancel
                );
                assert_eq!(
                    decide(
                        &NewWindowBy::DragEnd {
                            effect: EndEffect::Move
                        },
                        &input(None, button, esc, top.clone())
                    ),
                    Decision::Ignore(ACCEPTED_UNKNOWN),
                    "커서를 못 읽으면 맨 위 창도 못 읽은 것이다 — {top:?}"
                );
            }
        }
    }

    // ── ③ new_window_origin ────────────────────────────────────────────────────

    #[test]
    fn the_origin_backs_off_the_frame_and_grab_scaled_to_the_monitor() {
        let c = at(1000, 500);
        let grab = PointCss { x: 40.0, y: 16.0 };
        for (scale, inset, want) in [
            (1.0, (0.0, 0.0), (960, 484)),
            (1.0, (8.0, 31.0), (952, 453)),
            (1.25, (0.0, 0.0), (950, 480)),
            (1.25, (8.0, 31.0), (940, 441)),
            (2.0, (0.0, 0.0), (920, 468)),
            (2.0, (8.0, 31.0), (904, 406)),
        ] {
            assert_eq!(
                new_window_origin(c, scale, inset, grab),
                want,
                "{scale} {inset:?}"
            );
        }
    }

    #[test]
    fn the_origin_rounds_a_half_away_from_zero() {
        let c = at(0, 0);
        // 2 × 1.25 = 2.5 → 3 · −2 × 1.25 = −2.5 → −3.
        assert_eq!(
            new_window_origin(c, 1.25, (0.0, 0.0), PointCss { x: 2.0, y: -2.0 }),
            (-3, 3)
        );
        // 0.5 → 1 · −0.5 → −1.
        assert_eq!(
            new_window_origin(c, 1.0, (0.25, -0.25), PointCss { x: 0.25, y: -0.25 }),
            (-1, 1)
        );
    }

    #[test]
    fn a_lone_popout_dropped_with_the_pressed_point_as_grab_moves_by_the_drag() {
        // A popout with only one tab — grab = the pressed point (viewport). Cursor at press = window origin + (frame + pressed point) × scale factor.
        let (scale, inset) = (1.25, (8.0, 32.0));
        let grab = PointCss { x: 40.0, y: 16.0 };
        let origin = at(300, 200);
        let pressed = at(
            origin.x + ((inset.0 + grab.x) * scale) as i32,
            origin.y + ((inset.1 + grab.y) * scale) as i32,
        );
        let dragged = (137, -59);
        let released = at(pressed.x + dragged.0, pressed.y + dragged.1);
        assert_eq!(
            new_window_origin(released, scale, inset, grab),
            (origin.x + dragged.0, origin.y + dragged.1)
        );
    }
}
