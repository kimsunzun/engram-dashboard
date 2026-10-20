//! 놓기 명령의 순수 판정 — 명령 하나 · 끌기가 끝난 뒤 읽은 입력 · 출처 창 틀에서 적용할 도킹 요청 또는 취소 · 무시를
//! 고른다. Tauri · 락 · OS 호출 0 이라 단독 시험이 된다(ADR-0012). 진입점 = [`plan`] · 그 안의 새 창 가름 [`decide`] ·
//! 새 창 자리 [`new_window_origin`].
// ADR-0296
// TRD S21-slot-docking §3-3 · §3-5

use super::wire::{DropAnchor, DropTo, EndEffect, NewWindowBy, SizeCss, TabDrop};
use crate::layout::apply::{DockDest, DockRequest, OnEmptied, WindowPlace};
use crate::layout::chrome::PointCss;
use crate::layout::dock::DockAnchor;
use crate::layout::geometry::Insets;
use crate::layout::manager::{DockGuard, DockMove, FocusPolicy, TargetExpect, WindowLabel};

/// 무시 사유(`TabDropReply::Ignored.why` 의 철자).
pub const NO_CURSOR: &str = "no-cursor";
pub const LOST_DROP: &str = "lost-drop";
pub const ACCEPTED_OURS: &str = "accepted-ours";
pub const ACCEPTED_UNKNOWN: &str = "accepted-unknown";

/// 화면의 물리 px 점(가상 화면 좌표 — 주 모니터 왼쪽 위 원점 · 왼쪽 · 위 모니터면 음수). OS 층 crate 의 같은 이름
/// 타입은 어댑터가 옮긴다 — 이 모듈이 그 crate 에 기대지 않게.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PhysPoint {
    pub x: i32,
    pub y: i32,
}

/// 커서 밑 맨 위 최상위 창(z 순서 — 보이지 않음 · 최소화 · 비쳐 보이는 창은 건너뛴 것).
///
/// - `Ours`: 우리 웹뷰 창(그 label) — 웹뷰 밖 틀(제목 표시줄 · 테두리)도 이것이다.
/// - `Foreign`: 다른 앱 창 · 바탕화면.
/// - `Unknown`: 못 읽었다(커서를 못 읽음 · Windows 밖 · OS 가 답하지 않음).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopAt {
    Ours(WindowLabel),
    Foreign,
    Unknown,
}

/// 끌기가 끝난 뒤 셸이 읽은 입력 — 명령을 받자마자 락 · 장부보다 먼저 읽는다(놓은 점과 같은 순간이 되게).
///
/// - `cursor`: `None` = 못 읽었다.
/// - `primary_down` · `escape_down`: 그 순간 눌려 있나. `drop` 명령은 읽지 않는다(거짓).
/// - `top`: `drag_end` 명령만 읽는다(`drop` 은 `Unknown`).
/// - `scale`: 커서 밑 모니터의 배율 — 못 읽으면 출처 창 배율이다. 유한 양수가 아니면 [`SourceFrame::scale`] 을 쓴다.
#[derive(Debug, Clone, PartialEq)]
pub struct InputNow {
    pub cursor: Option<PhysPoint>,
    pub primary_down: bool,
    pub escape_down: bool,
    pub top: TopAt,
    pub scale: f64,
}

/// 출처 창(끄는 탭의 창)의 틀 — `insets` = 창 틀(CSS px · `window:frame` 과 같은 셈 [`super::frame_insets`]) · `scale` =
/// 그 창 배율. 새 창 틀은 출처 창 틀과 같다고 본다(팝아웃과 main 의 틀이 같다).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceFrame {
    pub insets: Insets,
    pub scale: f64,
}

/// [`decide`] 의 답 — `Place` = 그 커서 자리에 새 창(또는 탭 하나뿐인 팝아웃을 옮김).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Cancel,
    Ignore(&'static str),
    Place(PhysPoint),
}

/// [`plan`] 의 답 — `Dock` 만 모델을 쓴다.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    Dock(DockRequest),
    Cancel,
    Ignore(&'static str),
}

/// 놓기 명령 하나를 적용할 것으로 옮긴다. `caller` = 명령을 부른 창 — 탭바 · 이음새의 창이다(`drop.from` 이 아니다).
///
/// 모든 도킹 요청이 `FocusPolicy::Reveal` · `OnEmptied::Close` · 표식 없음 · `expect_window = from` 이고, 다음 활성
/// (`mv.successor`)은 `initial ≠ view` 일 때만 `initial` 이다. 사전조건 = `tab_bar` → `Tabs(expect)` · `seam` →
/// `SeamAt{window: caller, rev, active, or_initial}`(`or_initial` 은 `caller = from` 일 때만 `initial ≠ view` 인 `initial`)
/// · `new_window` → 없음. 새 창의 크기는 유한 양수일 때만 싣는다.
// TRD S21-slot-docking §3-3
pub fn plan(drop: &TabDrop, caller: &str, input: &InputNow, frame: &SourceFrame) -> Plan {
    let successor = drop.initial.filter(|initial| *initial != drop.view);
    let dock = |to: DockDest, expect_target: Option<TargetExpect>| {
        Plan::Dock(DockRequest {
            view: drop.view,
            to,
            guard: DockGuard {
                expect_window: Some(drop.from.clone()),
                expect_target,
                ..DockGuard::NONE
            },
            focus: FocusPolicy::Reveal,
            mv: DockMove { successor },
            on_emptied: OnEmptied::Close,
        })
    };
    match &drop.to {
        DropTo::TabBar { index, expect } => dock(
            DockDest::TabBar {
                window: caller.to_owned(),
                index: Some(*index as usize),
            },
            Some(TargetExpect::Tabs(expect.clone())),
        ),
        DropTo::Seam {
            target,
            anchor,
            side,
            share,
            rev,
            active,
        } => dock(
            DockDest::Seam {
                target: *target,
                anchor: match anchor {
                    DropAnchor::Slot { id } => DockAnchor::Slot(*id),
                    DropAnchor::Outer => DockAnchor::Outer,
                },
                side: *side,
                share: *share,
            },
            Some(TargetExpect::SeamAt {
                window: caller.to_owned(),
                rev: *rev,
                active: *active,
                or_initial: if caller == drop.from { successor } else { None },
            }),
        ),
        DropTo::NewWindow { by, size, grab } => match decide(by, input) {
            Decision::Cancel => Plan::Cancel,
            Decision::Ignore(why) => Plan::Ignore(why),
            Decision::Place(cursor) => {
                let scale = [input.scale, frame.scale]
                    .into_iter()
                    .find(|s| s.is_finite() && *s > 0.0)
                    .unwrap_or(1.0);
                let pos = new_window_origin(cursor, scale, (frame.insets.l, frame.insets.t), *grab);
                dock(
                    DockDest::NewWindow {
                        at: WindowPlace {
                            pos: Some(pos),
                            size: size.and_then(usable_size),
                        },
                    },
                    None,
                )
            }
        },
    }
}

/// 새 창 놓기를 가른다 — 아래 차례가 계약이다(앞의 것이 이긴다). 커서를 못 읽었으면 맨 위 창도 못 읽은 것으로 본다.
///
/// 1. `drop` → 커서 자리에 `Place`(커서 없음 = `Ignore(no-cursor)`).
/// 2. `drag_end{none}` + (Esc 눌림 ∨ 주 버튼 눌림 ∨ 커서 없음) → `Cancel`. `move` 는 이 줄을 건너뛴다(어딘가가 받았다).
/// 3. `drag_end` + 맨 위가 우리 창 → `Ignore` — `none` = `lost-drop` · `move` = `accepted-ours`(받은 창의 `drop` 이 따로
///    온다). 제목 표시줄 · 테두리도 우리 창이라 아무 일 없다.
/// 4. `drag_end` + 다른 앱 · 바탕화면(`none` · `move` 둘 다) · `drag_end{none}` + 못 읽음 → `Place`.
/// 5. `drag_end{move}` + 못 읽음 → `Ignore(accepted-unknown)`.
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

/// 새 창의 바깥 왼쪽 위(물리 px) = `cursor − round((inset_dip + grab) × scale)` — 커서가 새 창 뷰포트의 `grab` 에 온다.
/// `inset_dip` = 창 틀의 (왼쪽, 위)(CSS px) · `scale` = 놓는 모니터 배율. 반올림은 .5 를 0 에서 먼 쪽으로 한다.
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
            rev: 42,
            active: id(1),
        }
    }

    fn new_window(by: NewWindowBy, size: Option<SizeCss>) -> DropTo {
        DropTo::NewWindow {
            by,
            size,
            grab: PointCss { x: 0.0, y: 0.0 },
        }
    }

    // ── ① 대응 (§3-3) ──────────────────────────────────────────────────────────

    #[test]
    fn a_tab_bar_drop_targets_the_calling_window_with_the_tab_list_and_source_window_preconditions()
    {
        let d = drop_to(
            MAIN,
            Some(id(3)),
            DropTo::TabBar {
                index: 2,
                expect: vec![id(3), id(1), id(4)],
            },
        );
        let req = request(plan(&d, POPUP, &quiet(TopAt::Unknown), &FLAT));
        assert_eq!(
            req,
            DockRequest {
                view: id(1),
                to: DockDest::TabBar {
                    window: POPUP.into(),
                    index: Some(2),
                },
                guard: DockGuard {
                    lease: None,
                    keep_lease: false,
                    expect_window: Some(MAIN.into()),
                    expect_source: None,
                    expect_target: Some(TargetExpect::Tabs(vec![id(3), id(1), id(4)])),
                },
                focus: FocusPolicy::Reveal,
                mv: DockMove {
                    successor: Some(id(3))
                },
                on_emptied: OnEmptied::Close,
            },
            "탭바 창 = 부른 창(인자의 from 이 아니다)"
        );
    }

    #[test]
    fn the_successor_is_the_initial_tab_only_when_it_is_not_the_dragged_tab() {
        let bar = || DropTo::TabBar {
            index: 0,
            expect: vec![id(1)],
        };
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
        }
    }

    #[test]
    fn a_seam_drop_maps_the_anchor_and_carries_the_seam_preconditions() {
        // 다른 창 대상 — or_initial 없음.
        let req = request(plan(
            &drop_to(MAIN, Some(id(3)), seam(DropAnchor::Slot { id: id(9) })),
            POPUP,
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
            }
        );
        assert_eq!(
            req.guard,
            DockGuard {
                expect_window: Some(MAIN.into()),
                expect_target: Some(TargetExpect::SeamAt {
                    window: POPUP.into(),
                    rev: 42,
                    active: id(1),
                    or_initial: None,
                }),
                ..DockGuard::NONE
            }
        );
        assert_eq!(req.focus, FocusPolicy::Reveal);
        assert_eq!(req.mv.successor, Some(id(3)));
        assert_eq!(req.on_emptied, OnEmptied::Close);

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
    fn or_initial_is_set_only_for_a_seam_in_the_source_window_with_a_different_initial_tab() {
        let cases = [
            (MAIN, Some(id(3)), Some(id(3))),
            (MAIN, Some(id(1)), None),
            (MAIN, None, None),
            (POPUP, Some(id(3)), None),
        ];
        for (caller, initial, want) in cases {
            let req = request(plan(
                &drop_to(MAIN, initial, seam(DropAnchor::Outer)),
                caller,
                &quiet(TopAt::Unknown),
                &FLAT,
            ));
            let Some(TargetExpect::SeamAt {
                or_initial, window, ..
            }) = req.guard.expect_target
            else {
                panic!("SeamAt 이어야");
            };
            assert_eq!(or_initial, want, "{caller} {initial:?}");
            assert_eq!(window, caller);
        }
    }

    #[test]
    fn a_new_window_drop_places_at_the_origin_with_only_the_source_window_precondition() {
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
        assert_eq!(
            req.guard,
            DockGuard {
                expect_window: Some(MAIN.into()),
                ..DockGuard::NONE
            }
        );
        assert_eq!(req.mv.successor, Some(id(3)));
        assert_eq!(req.focus, FocusPolicy::Reveal);
        assert_eq!(req.on_emptied, OnEmptied::Close);
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

    // ── ② decide (§3-5 표의 차례) ──────────────────────────────────────────────

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
        // 제목 표시줄 · 테두리도 우리 창이다 — 아무 일 없음(Q38).
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
        // 탭 하나뿐인 팝아웃 — grab = 누른 점(뷰포트). 누를 때 커서 = 창 원점 + (틀 + 누른 점) × 배율.
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
