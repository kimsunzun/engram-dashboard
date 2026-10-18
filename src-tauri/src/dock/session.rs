//! 탭 끌기 세션(TRD S21-slot-docking §2-3) — 메인 스레드에서 사건을 하나씩 받아 포트(`ports`)를 동기로 부른다.
//! 진입점 = [`Drag::handle`]. 전이 이름 A0–A19 는 끌기 세션 축소안 표(2026-10-18 · 추적 밖
//! `.claude/handoff/attachments/trd6-review-2026-10-18/u5-trim-plan.md` §3)의 번호다 — TRD §2-3 의 전이 표(T01–T44)와
//! 번호가 다르다.
//!
//! - 세션은 한 번에 하나다. 탭바 안([`Phase::Strip`] — 웹뷰가 순서 이동을 몬다)과 OS 이동 루프([`Phase::Moving`] —
//!   셸이 몬다) 두 단계를 오간다. 표식(lease)은 시작에 세우고 끝([`Drag`] 의 `finish`)에서만 내린다 — 그 사이의
//!   레이아웃 쓰기는 전부 같은 표식을 쥔다(`DockWrite`).
//! - 사건 하나를 끝까지 처리한 뒤 다음 사건을 본다 — 쓰기의 결과는 같은 처리 안에서 받는다(포트가 동기다). 그래서
//!   쓰기와 그 결과 사이에 다른 사건이 끼지 않는다.
//! - 늦은 사건은 번호로 버린다 — 웹뷰 사건 = 세션 번호(+ 순서 이동은 `seq`) · 루프 알림 = 루프 번호 · 폴러 = 세대 ·
//!   시한 = 시한 번호. 넷 다 [`Drag`] 가 부팅 안에서 단조로 뽑는다(앞 세션의 늦은 사건이 새 세션 번호와 겹치지 않게).
//! - 표에 없는 조합은 debug 한 줄로 버린다 — 버려진 begin · leave 의 답은 보내는 쪽이 떨어져 그 invoke 가 오류로
//!   끝난다(웹뷰는 그 끌기를 버린다).
// ADR-0295

use std::fmt;
use std::time::Instant;

use super::geometry::zone_in_canvas;
use super::ports::{DetachPlan, DockWrite, Placement, Ports, SessionDock, WholePlan};
use super::sizing::{
    dragged_size, original_size, restored_size, whole_window_origin, work_area_near,
};
use super::strip::{client_point, strip_zone};
use super::targets::{judge, Target};
use super::units::{PhysPoint, PhysRect, SizeDip, WindowGeom};
use super::wire::{
    BeginReply, EndHow, EndSrc, FeedStep, Grab, LeaveReply, TabDragBegin, TabDragEvent,
    TabDragFeed, TabDragPhase,
};
use super::world::{World, WorldWindow};
use crate::layout::apply::DockOutcome;
use crate::layout::chrome::{DockChrome, PointCss, RectCss};
use crate::layout::manager::{EmptiedWindow, FocusPolicy, LeaseId, ViewId, WindowLabel};
use crate::layout::{LayoutNode, SlotContent, MAIN_WINDOW_LABEL};

#[cfg(test)]
mod tests;

/// begin · leave 의 답 — 한 번 보내거나 버린다. 버리면(답 없이 떨어지면) 기다리던 invoke 가 오류로 끝난다.
pub struct Reply<T>(Box<dyn FnOnce(T) + Send>);

impl<T> Reply<T> {
    pub fn new(send: impl FnOnce(T) + Send + 'static) -> Self {
        Reply(Box::new(send))
    }

    pub(crate) fn send(self, value: T) {
        (self.0)(value)
    }
}

impl<T> fmt::Debug for Reply<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Reply")
    }
}

/// 세션이 받는 사건 — 실행기가 메인 스레드 줄에서 하나씩 넘긴다.
///
/// - `Begin` · `Reorder` · `Leave` · `End`: 웹뷰 invoke(`tab_drag_*`) — `window` · `from` = 부른 창.
/// - `Moved`: 우리 창의 `WindowEvent::Moved`(실행기가 끄는 창 것만 넘겨도 되고 다 넘겨도 된다).
/// - `LoopEnded`: 이동 루프 끝 알림 — `cursor` = 알림을 받은 때의 커서.
/// - `WindowGone`: 우리 창이 사라졌다(명부가 먼저 지운 뒤).
/// - `Pointer`: 폴러 — 커서 · 주 버튼이 바뀌었다.
/// - `RaiseDue` · `ReleaseDue`: 시한.
#[derive(Debug)]
pub enum Event {
    Begin {
        window: WindowLabel,
        args: TabDragBegin,
        reply: Reply<BeginReply>,
    },
    Reorder {
        session: u64,
        from: WindowLabel,
        seq: u32,
        index: u32,
    },
    Leave {
        session: u64,
        from: WindowLabel,
        reply: Reply<LeaveReply>,
    },
    End {
        session: u64,
        from: WindowLabel,
        how: EndHow,
        src: EndSrc,
        at: Option<u32>,
    },
    Moved {
        label: WindowLabel,
    },
    LoopEnded {
        session: u64,
        loop_no: u32,
        end: LoopEnd,
        cursor: PhysPoint,
    },
    WindowGone {
        label: WindowLabel,
    },
    Pointer {
        gen: u32,
        cursor: PhysPoint,
        down: bool,
    },
    RaiseDue {
        seq: u32,
    },
    ReleaseDue {
        seq: u32,
    },
}

impl Event {
    fn name(&self) -> &'static str {
        match self {
            Event::Begin { .. } => "begin",
            Event::Reorder { .. } => "reorder",
            Event::Leave { .. } => "leave",
            Event::End { .. } => "end",
            Event::Moved { .. } => "moved",
            Event::LoopEnded { .. } => "loop_ended",
            Event::WindowGone { .. } => "window_gone",
            Event::Pointer { .. } => "pointer",
            Event::RaiseDue { .. } => "raise_due",
            Event::ReleaseDue { .. } => "release_due",
        }
    }
}

/// 이동 루프의 끝 까닭(platform `MoveLoopEnd` 를 실행기가 옮긴 것 — 들어가기 전이었나 · Esc 를 봤나 · 훅이 숨겼나는
/// 세션이 보지 않는다).
///
/// - `Released`: 주 버튼을 뗐다(놓기).
/// - `Ended`: 우리가 `end_move_loop` 로 끝냈다.
/// - `Canceled`: 뗌 없이 끝났다(Esc · 루프를 끝낸 다른 키 · 잠금 화면 · UAC) — 되돌린다.
/// - `Destroyed`: 그 창이 사라졌다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopEnd {
    Released,
    Ended,
    Canceled,
    Destroyed,
}

/// 탭바 안 단계의 입력 — `Pointer` = 출처 웹뷰가 포인터 캡처로 몬다 · `Shell` = 붙은 창 웹뷰를 셸 폴러가 민 커서로
/// 몬다(`dock:feed{move}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    Pointer,
    Shell,
}

/// 이동 루프로 따라오는 창 — `Detached` = 예비 창에 떼어 담았다 · `Whole` = 탭 하나 창이 창째(`main` = 메인 창의
/// 마지막 탭이라 메인 창이 따라온다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveKind {
    Detached,
    Whole { main: bool },
}

/// 우리가 루프를 끝낸 까닭 — 처음 정한 것이 이긴다(정한 뒤의 `Moved` · 키는 버린다).
///
/// - `Attach`: 그 창 탭바 구역에 닿았다 — 루프 끝에서 그 탭바 0 자리에 붙인다. `active_before` = 그 창의 붙기 전
///   활성 자리(C8).
/// - `DropByKey`: 루프 중 우리 웹뷰에 닿은 Esc 밖의 키 — 루프 끝 커서로 놓는다.
/// - `Revert`: 루프 중 우리 웹뷰에 닿은 Esc — 루프 끝에서 되돌린다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pending {
    Attach {
        to: WindowLabel,
        active_before: Option<usize>,
    },
    DropByKey,
    Revert,
}

/// 세션 단계.
///
/// - `Strip`: 끄는 탭이 `holder` 탭바 안에 있다. `releasing` = 놓기 확인을 기다리는 시한 번호(셸이 놓기를 봤거나
///   다른 창의 키 놓기 — 그 창이 지금 자리로 답하기를 기다린다) · `last_reorder_seq` = 이 holder 에서 받은 마지막
///   순서 이동 번호(holder 가 바뀌면 0 부터).
/// - `Moving`: `window` 가 OS 이동 루프로 따라온다. `raise` = 걸어 둔 앞으로 올리기 시한 번호 · `loop_no` = 그
///   루프의 번호.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    Strip {
        holder: WindowLabel,
        feed: Feed,
        releasing: Option<u32>,
        last_reorder_seq: u32,
    },
    Moving {
        window: WindowLabel,
        kind: MoveKind,
        pending: Option<Pending>,
        raise: Option<u32>,
        loop_no: u32,
    },
}

impl Phase {
    fn name(&self) -> &'static str {
        match self {
            Phase::Strip {
                releasing: Some(_), ..
            } => "strip_releasing",
            Phase::Strip {
                feed: Feed::Pointer,
                ..
            } => "strip_pointer",
            Phase::Strip {
                feed: Feed::Shell, ..
            } => "strip_shell",
            Phase::Moving { pending: None, .. } => "moving",
            Phase::Moving {
                pending: Some(Pending::Attach { .. }),
                ..
            } => "moving_attach",
            Phase::Moving {
                pending: Some(Pending::DropByKey),
                ..
            } => "moving_drop_key",
            Phase::Moving {
                pending: Some(Pending::Revert),
                ..
            } => "moving_revert",
        }
    }
}

/// 끌기 하나의 기록.
#[derive(Debug)]
pub struct Session {
    id: u64,
    view: ViewId,
    lease: LeaseId,
    origin: Origin,
    /// 출처 창 복원 바깥 크기(DIP — 떼어진 창 크기의 바탕 · 시작 때 한 번).
    restored: SizeDip,
    /// 붙는 크기의 원래 크기(§5-4) — 창째 끌기는 시작에서 · 떼기는 첫 떼기에서 놓은 직후 OS 기하로 한 번 적고
    /// 바꾸지 않는다(R7 · R21). `None` = 못 읽었다(띠를 내지 않는다).
    original: Option<SizeDip>,
    grab: Grab,
    /// 지금 붙어 있는 창과 그 창의 붙기 전 활성 자리(C8) — 떼면 지운다.
    before_attach: Option<(WindowLabel, Option<usize>)>,
    /// 창째 끌던 출처 창이 다른 창 탭바 · 띠에 붙어 비었다 — 되돌리기가 다시 들이거나 끝에 닫는다(R10 · AC26).
    emptied: Option<EmptiedWindow>,
    /// 메인 마지막 탭이 떠나며 메인에 생긴 빈 탭(R14).
    main_refill: Option<ViewId>,
    /// 지금 폴러의 세대.
    poll_gen: u32,
    counts: Counts,
    started: Instant,
    phase: Phase,
}

/// 되돌릴 곳 — 시작 때 적는다.
#[derive(Debug)]
struct Origin {
    window: WindowLabel,
    /// 그 창 탭 목록에서 끄는 탭의 자리.
    index: usize,
    /// 누르기 전 활성 탭 — `None` = 끄는 탭이 활성이었다.
    active: Option<ViewId>,
    start: Placement,
    /// 탭 하나 창이었다(창째 끌기).
    single: bool,
    /// 누른 점(화면).
    down: PhysPoint,
}

#[derive(Debug, Default)]
struct Counts {
    reorders: u32,
    detaches: u32,
    attaches: u32,
}

/// 끝 줄의 결과.
#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    /// 탭바 안에서 놓았다 — 배치는 이미 모델에 있다.
    Strip,
    /// 그 창의 띠에 붙였다.
    Dock {
        target: WindowLabel,
    },
    /// 떼어진 창이 그대로 새 창으로 남았다.
    NewWindow,
    /// 창째 끌던 창이 놓은 자리에 남았다.
    Moved,
    Reverted,
    /// 탭을 쥔 창이 사라졌다.
    Aborted,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Outcome::Strip => "strip",
            Outcome::Dock { .. } => "dock",
            Outcome::NewWindow => "new_window",
            Outcome::Moved => "moved",
            Outcome::Reverted => "reverted",
            Outcome::Aborted => "aborted",
        })
    }
}

/// 끝 줄의 끝낸 까닭.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndBy {
    Pointer,
    Release,
    Key,
    Esc,
    CaptureLost,
    ReleaseTimeout,
    LoopRelease,
    LoopCanceled,
    HolderGone,
    Failed,
}

impl From<EndSrc> for EndBy {
    fn from(src: EndSrc) -> Self {
        match src {
            EndSrc::Pointer => EndBy::Pointer,
            EndSrc::Release => EndBy::Release,
            EndSrc::Key => EndBy::Key,
            EndSrc::CaptureLost => EndBy::CaptureLost,
        }
    }
}

impl fmt::Display for EndBy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            EndBy::Pointer => "pointer",
            EndBy::Release => "release",
            EndBy::Key => "key",
            EndBy::Esc => "esc",
            EndBy::CaptureLost => "capture_lost",
            EndBy::ReleaseTimeout => "release_timeout",
            EndBy::LoopRelease => "loop_release",
            EndBy::LoopCanceled => "loop_canceled",
            EndBy::HolderGone => "holder_gone",
            EndBy::Failed => "failed",
        })
    }
}

/// 사건 하나가 고른 전이 — [`classify`] 가 단계 · 사건 · 번호로 고르고 [`Drag::step`] 이 돌린다. `holder` = 탭바 안
/// 단계의 탭을 쥔 창 · `window` = 이동 루프로 따라오는 창.
#[derive(Debug)]
enum Arm {
    /// A2.
    Busy {
        window: WindowLabel,
        reply: Reply<BeginReply>,
    },
    /// A3.
    Reorder {
        holder: WindowLabel,
        seq: u32,
        index: u32,
    },
    /// A4.
    Detach {
        holder: WindowLabel,
        reply: Reply<LeaveReply>,
    },
    /// A5.
    NotReady {
        holder: WindowLabel,
        reply: Reply<LeaveReply>,
    },
    /// A6.
    StripDrop {
        holder: WindowLabel,
        src: EndSrc,
        at: Option<u32>,
    },
    /// A7.
    ConfirmRelease { holder: WindowLabel },
    /// A8.
    StripRevert(EndBy),
    /// A9.
    FeedMove {
        holder: WindowLabel,
        cursor: PhysPoint,
    },
    /// A10.
    ReleaseTimeout,
    /// A11 · A19.
    HolderGone,
    /// A12.
    Follow { window: WindowLabel, kind: MoveKind },
    /// A13.
    Raise { window: WindowLabel },
    /// A14 · A15.
    LoopKey {
        window: WindowLabel,
        pending: Pending,
    },
    /// A16.
    LoopDrop {
        window: WindowLabel,
        kind: MoveKind,
        cursor: PhysPoint,
        end_by: EndBy,
    },
    /// A17.
    LoopAttach {
        window: WindowLabel,
        kind: MoveKind,
        cursor: PhysPoint,
        to: WindowLabel,
        active_before: Option<usize>,
    },
    /// A18.
    LoopRevert(EndBy),
    /// A0.
    Ignore(&'static str),
}

/// 세션이 있을 때 사건 하나의 전이를 고른다 — 상태를 바꾸지 않는다. 표에 없는 조합 = `Ignore`.
fn classify(s: &Session, ev: Event) -> Arm {
    let name = ev.name();
    let live = |session: u64| session == s.id;
    match (&s.phase, ev) {
        (_, Event::Begin { window, reply, .. }) => Arm::Busy { window, reply },
        (
            Phase::Strip {
                holder,
                last_reorder_seq,
                ..
            },
            Event::Reorder {
                session,
                from,
                seq,
                index,
            },
        ) if live(session) && from == *holder && seq > *last_reorder_seq => Arm::Reorder {
            holder: from,
            seq,
            index,
        },
        (
            Phase::Strip {
                holder, releasing, ..
            },
            Event::Leave {
                session,
                from,
                reply,
            },
        ) if live(session) && from == *holder => match releasing {
            None => Arm::Detach {
                holder: from,
                reply,
            },
            Some(_) => Arm::NotReady {
                holder: from,
                reply,
            },
        },
        (
            Phase::Strip {
                holder,
                feed,
                releasing,
                ..
            },
            Event::End {
                session,
                from,
                how,
                src,
                at,
            },
        ) if live(session) => {
            let own = from == *holder;
            let pointer = *feed == Feed::Pointer;
            let holder = holder.clone();
            match (how, src) {
                (EndHow::Drop, EndSrc::Pointer) if own && pointer => {
                    Arm::StripDrop { holder, src, at }
                }
                (EndHow::Drop, EndSrc::Release) if own && releasing.is_some() => {
                    Arm::StripDrop { holder, src, at }
                }
                (EndHow::Drop, EndSrc::Key) if own => Arm::StripDrop { holder, src, at },
                (EndHow::Drop, EndSrc::Key) if releasing.is_none() => {
                    Arm::ConfirmRelease { holder }
                }
                (EndHow::Cancel, EndSrc::Key) if releasing.is_none() => {
                    Arm::StripRevert(EndBy::Esc)
                }
                (EndHow::Cancel, EndSrc::CaptureLost) if own && pointer && releasing.is_none() => {
                    Arm::StripRevert(EndBy::CaptureLost)
                }
                _ => Arm::Ignore(name),
            }
        }
        (
            Phase::Strip {
                holder,
                feed: Feed::Shell,
                releasing: None,
                ..
            },
            Event::Pointer { gen, cursor, down },
        ) if gen == s.poll_gen => {
            let holder = holder.clone();
            if down {
                Arm::FeedMove { holder, cursor }
            } else {
                Arm::ConfirmRelease { holder }
            }
        }
        (
            Phase::Strip {
                releasing: Some(r), ..
            },
            Event::ReleaseDue { seq },
        ) if seq == *r => Arm::ReleaseTimeout,
        (Phase::Strip { holder, .. }, Event::WindowGone { label }) if label == *holder => {
            Arm::HolderGone
        }
        (
            Phase::Moving {
                window,
                kind,
                pending: None,
                ..
            },
            Event::Moved { label },
        ) if label == *window => Arm::Follow {
            window: label,
            kind: *kind,
        },
        (
            Phase::Moving {
                window,
                pending: None,
                raise: Some(r),
                ..
            },
            Event::RaiseDue { seq },
        ) if seq == *r => Arm::Raise {
            window: window.clone(),
        },
        (
            Phase::Moving {
                window,
                pending: None,
                ..
            },
            Event::End {
                session,
                how,
                src: EndSrc::Key,
                ..
            },
        ) if live(session) => Arm::LoopKey {
            window: window.clone(),
            pending: match how {
                EndHow::Drop => Pending::DropByKey,
                EndHow::Cancel => Pending::Revert,
            },
        },
        (
            Phase::Moving {
                window,
                kind,
                pending,
                loop_no,
                ..
            },
            Event::LoopEnded {
                session,
                loop_no: n,
                end,
                cursor,
            },
        ) if live(session) && n == *loop_no => {
            let (window, kind) = (window.clone(), *kind);
            match (end, pending) {
                (LoopEnd::Destroyed, _) => Arm::HolderGone,
                (LoopEnd::Released, None) => Arm::LoopDrop {
                    window,
                    kind,
                    cursor,
                    end_by: EndBy::LoopRelease,
                },
                (LoopEnd::Ended | LoopEnd::Canceled, None) => Arm::LoopRevert(EndBy::LoopCanceled),
                (_, Some(Pending::DropByKey)) => Arm::LoopDrop {
                    window,
                    kind,
                    cursor,
                    end_by: EndBy::Key,
                },
                (_, Some(Pending::Attach { to, active_before })) => Arm::LoopAttach {
                    window,
                    kind,
                    cursor,
                    to: to.clone(),
                    active_before: *active_before,
                },
                (_, Some(Pending::Revert)) => Arm::LoopRevert(EndBy::Esc),
            }
        }
        (Phase::Moving { window, .. }, Event::WindowGone { label }) if label == *window => {
            Arm::HolderGone
        }
        _ => Arm::Ignore(name),
    }
}

/// 끌기 세션의 주인 — 셸에 하나이고 메인 스레드 실행기가 쥔다. 번호(세션 · 루프 · 폴러 세대 · 시한)는 부팅 안에서
/// 단조로 뽑는다.
#[derive(Debug, Default)]
pub struct Drag {
    session: Option<Session>,
    last_session: u64,
    last_loop: u32,
    last_gen: u32,
    last_timer: u32,
}

impl Drag {
    pub fn new() -> Self {
        Self::default()
    }

    /// 지금 세션 번호 — `None` = 끌기가 없다.
    pub fn session_id(&self) -> Option<u64> {
        self.session.as_ref().map(|s| s.id)
    }

    /// 사건 하나를 끝까지 처리한다 — 계약은 모듈 머리 · 포트 계약은 `ports` 머리.
    pub fn handle(&mut self, ev: Event, p: &mut dyn Ports) {
        self.session = match self.session.take() {
            Some(s) => self.step(s, ev, p),
            None => match ev {
                Event::Begin {
                    window,
                    args,
                    reply,
                } => self.begin(window, args, reply, p),
                // 셸은 우리 창의 `Moved` 를 끌기와 무관하게 다 넣는다 — 손으로 창을 옮길 때마다 debug 를 불리지 않게.
                other @ Event::Moved { .. } => {
                    tracing::trace!(event = other.name(), "끌기 없음 — 사건 무시");
                    None
                }
                other => {
                    tracing::debug!(event = other.name(), "끌기 없음 — 사건 무시");
                    None
                }
            },
        };
    }

    fn step(&mut self, mut s: Session, ev: Event, p: &mut dyn Ports) -> Option<Session> {
        let timer = matches!(ev, Event::RaiseDue { .. } | Event::ReleaseDue { .. });
        match classify(&s, ev) {
            // 앞으로 올리기 시한은 `Moved` 마다 새로 걸려 지난 시한이 늘 버려진다 — debug 를 두 배로 불리지 않게.
            Arm::Ignore(event) if timer => {
                tracing::trace!(session = s.id, phase = ?s.phase, event, "끌기 사건 무시 — 지난 시한");
                Some(s)
            }
            Arm::Ignore(event) => {
                tracing::debug!(session = s.id, phase = ?s.phase, event, "끌기 사건 무시");
                Some(s)
            }
            Arm::Busy { window, reply } => {
                let at = s.phase.name();
                tracing::debug!(session = s.id, from = at, to = at, window = %window, "끌기 시작 거절 — 이미 끄는 중");
                reply.send(BeginReply::Busy);
                Some(s)
            }
            Arm::Reorder { holder, seq, index } => {
                if let Phase::Strip {
                    last_reorder_seq, ..
                } = &mut s.phase
                {
                    *last_reorder_seq = seq;
                }
                let write = s.write(
                    SessionDock::TabBar {
                        window: holder,
                        index: Some(index as usize),
                    },
                    FocusPolicy::Reveal,
                    None,
                );
                match p.dock(write) {
                    Ok(_) => s.counts.reorders += 1,
                    Err(e) => tracing::warn!(session = s.id, index, "순서 이동 쓰기 실패: {e}"),
                }
                Some(s)
            }
            Arm::Detach { holder, reply } => self.detach(s, holder, reply, p),
            Arm::NotReady { holder, reply } => {
                let at = s.phase.name();
                tracing::debug!(session = s.id, from = at, to = at, window = %holder, "떼기 보류 — 놓기 확인 중");
                reply.send(LeaveReply::NotReady);
                Some(s)
            }
            Arm::StripDrop { holder, src, at } => {
                if let Some(at) = at {
                    let now = p
                        .world()
                        .window(&holder)
                        .and_then(|w| w.tabs.iter().position(|t| *t == s.view));
                    if now != Some(at as usize) {
                        let write = s.write(
                            SessionDock::TabBar {
                                window: holder,
                                index: Some(at as usize),
                            },
                            FocusPolicy::Reveal,
                            None,
                        );
                        if let Err(e) = p.dock(write) {
                            tracing::warn!(session = s.id, at, "놓는 자리 쓰기 실패: {e}");
                        }
                    }
                }
                self.finish(s, Outcome::Strip, src.into(), p);
                None
            }
            Arm::ConfirmRelease { holder } => {
                self.last_timer += 1;
                let seq = self.last_timer;
                let from = s.phase.name();
                if let Phase::Strip { releasing, .. } = &mut s.phase {
                    *releasing = Some(seq);
                }
                tracing::debug!(session = s.id, from, to = s.phase.name(), window = %holder, seq, "놓기 확인 시작");
                p.poll_stop();
                p.feed(&holder, s.feed(FeedStep::Release));
                p.arm_release(seq);
                Some(s)
            }
            Arm::StripRevert(end_by) | Arm::LoopRevert(end_by) => {
                self.revert(s, end_by, p);
                None
            }
            Arm::FeedMove { holder, cursor } => {
                if let Some(g) = p.geom(&holder) {
                    let c = client_point(cursor, g.inner.origin(), g.scale);
                    p.feed(&holder, s.feed(FeedStep::Move { x: c.x, y: c.y }));
                }
                Some(s)
            }
            Arm::ReleaseTimeout => {
                self.finish(s, Outcome::Strip, EndBy::ReleaseTimeout, p);
                None
            }
            Arm::HolderGone => {
                self.finish(s, Outcome::Aborted, EndBy::HolderGone, p);
                None
            }
            Arm::Follow { window, kind } => {
                self.follow(&mut s, window, kind, p);
                Some(s)
            }
            Arm::Raise { window } => {
                let cursor = p.cursor();
                if let Some(under) = p.probe(cursor, &window) {
                    p.raise_no_activate(&under);
                    p.raise_no_activate(&window);
                }
                Some(s)
            }
            Arm::LoopKey { window, pending } => {
                if pending == Pending::Revert {
                    p.overlay(None);
                }
                s.decide(pending);
                p.end_move_loop(&window);
                Some(s)
            }
            Arm::LoopDrop {
                window,
                kind,
                cursor,
                end_by,
            } => {
                self.loop_drop(s, window, kind, cursor, end_by, p);
                None
            }
            Arm::LoopAttach {
                window,
                kind,
                cursor,
                to,
                active_before,
            } => self.loop_attach(s, window, kind, cursor, to, active_before, p),
        }
    }

    // A1 — 표식을 세우고 되돌릴 곳을 적는다. 탭 둘 이상이면 탭바 안으로 · 하나면 창째 이동 루프로.
    fn begin(
        &mut self,
        window: WindowLabel,
        args: TabDragBegin,
        reply: Reply<BeginReply>,
        p: &mut dyn Ports,
    ) -> Option<Session> {
        let view = args.view;
        let lease = match p.lease_tab(view) {
            Ok(lease) => lease,
            Err(e) => {
                tracing::debug!(view = %view, window = %window, "끌기 시작 거절: {e}");
                reply.send(BeginReply::Rejected {
                    reason: e.to_string(),
                });
                return None;
            }
        };
        let world = p.world();
        let found = world.window(&window).and_then(|w| {
            let index = w.tabs.iter().position(|t| *t == view)?;
            Some((w, index, w.geom?))
        });
        let Some((w, index, geom)) = found else {
            p.release_lease(lease);
            tracing::info!(view = %view, window = %window, "끌기 시작 거절 — 부른 창에 그 탭이 없거나 창 기하를 못 읽었다");
            reply.send(BeginReply::Rejected {
                reason: "부른 창에 그 탭이 없거나 창 기하를 못 읽었다".to_owned(),
            });
            return None;
        };
        let restored = restored_size(w.attrs.bounds, &geom);
        let start = start_placement(w, &geom, restored);
        let single = w.tabs.len() == 1;
        self.last_session += 1;
        let id = self.last_session;
        let mut s = Session {
            id,
            view,
            lease,
            origin: Origin {
                window: window.clone(),
                index,
                active: args.initial_active.filter(|a| *a != view),
                start,
                single,
                down: screen_point(args.down, &geom),
            },
            restored,
            original: None,
            grab: args.grab,
            before_attach: None,
            emptied: None,
            main_refill: None,
            poll_gen: 0,
            counts: Counts::default(),
            started: Instant::now(),
            phase: Phase::Strip {
                holder: window.clone(),
                feed: Feed::Pointer,
                releasing: None,
                last_reorder_seq: 0,
            },
        };
        tracing::info!(session = id, view = %view, window = %window, tabs = w.tabs.len(), "끌기 시작");
        reply.send(if single {
            BeginReply::WholeWindow { session: id }
        } else {
            BeginReply::Strip {
                session: id,
                zone: viewport_zone(args.area, w.chrome.as_ref()),
            }
        });
        p.drag(s.drag_event(TabDragPhase::Active), None);
        // 짓기에 실패해 명부에서 빠진 예비를 탭바를 나가기 전에 다시 짓게.
        p.fill_spare();
        if !single {
            return Some(s);
        }
        let cursor = p.cursor();
        let plan = whole_plan(
            &geom,
            start,
            restored,
            s.origin.down,
            cursor,
            &world.monitors,
        );
        p.place_whole(&window, plan);
        let canvas_origin = w.chrome.as_ref().map(|c| c.canvas_origin);
        s.original = p.geom(&window).and_then(|g| original_of(&g, canvas_origin));
        let main = window == MAIN_WINDOW_LABEL;
        self.start_loop(s, "idle", window, MoveKind::Whole { main }, p)
    }

    // A4 — 예비 창에 들이고 커서 밑에 놓은 뒤 이동 루프를 올린다. 예비 창이 아직 없으면 `NotReady`.
    fn detach(
        &mut self,
        mut s: Session,
        holder: WindowLabel,
        reply: Reply<LeaveReply>,
        p: &mut dyn Ports,
    ) -> Option<Session> {
        let world = p.world();
        let Some((spare, measures)) = world
            .parked
            .ready_spare()
            .map(|(label, m)| (label.to_owned(), m.clone()))
        else {
            let at = s.phase.name();
            tracing::debug!(session = s.id, from = at, to = at, window = %holder, "떼기 보류 — 예비 창이 아직 없다");
            reply.send(LeaveReply::NotReady);
            return Some(s);
        };
        let held = world.window(&holder);
        let successor = held.and_then(|w| s.detach_successor(w));
        let maximized = held
            .filter(|w| w.attrs.maximized)
            .and_then(|w| w.geom)
            .map(|g| outer_dip(&g));
        let chrome = measures.chrome.clone();
        let write = s.write(
            SessionDock::AdoptWindow {
                label: spare.clone(),
                seed: None,
                measures,
            },
            FocusPolicy::Reveal,
            successor,
        );
        match p.dock(write) {
            // 메인의 마지막 탭을 떼면 모델이 메인에 빈 탭을 채운다(R14) — 메인에 다시 붙거나 되돌릴 때 닫으려면(G14) 적어
            // 둔다.
            Ok(out) => {
                // 덮어쓰지 않는다 — 이 떼기가 메인을 비우지 않았으면 앞선 창째 끌기 · 떼기가 적은 빈 탭을 그대로 둔다.
                if let Some(refill) = out.main_refilled {
                    s.main_refill = Some(refill);
                }
            }
            Err(e) => {
                tracing::warn!(session = s.id, spare = %spare, "예비 창 들이기 실패 — 끝낸다: {e}");
                reply.send(LeaveReply::Ended);
                self.finish(s, Outcome::Strip, EndBy::Failed, p);
                return None;
            }
        }
        let from = s.phase.name();
        tracing::debug!(session = s.id, from, to = "moving", window = %spare, holder = %holder, "떼기 — 예비 창에 들였다");
        p.unpark(&spare);
        reply.send(LeaveReply::Detached);
        p.poll_stop();
        p.drag(s.drag_event(TabDragPhase::Active), Some(&spare));
        let cursor = p.cursor();
        let plan = DetachPlan {
            restored: s.restored,
            maximized,
            strip_origin: chrome
                .as_ref()
                .map_or(PointCss { x: 0.0, y: 0.0 }, |c| PointCss {
                    x: c.tab_bar.x0,
                    y: c.tab_bar.y0,
                }),
            grab: PointCss {
                x: s.grab.single_x,
                y: s.grab.y,
            },
        };
        p.place_detached(&spare, &plan, cursor);
        if s.original.is_none() {
            let canvas_origin = chrome.as_ref().map(|c| c.canvas_origin);
            s.original = p.geom(&spare).and_then(|g| original_of(&g, canvas_origin));
        }
        s.before_attach = None;
        s.counts.detaches += 1;
        self.start_loop(s, from, spare, MoveKind::Detached, p)
    }

    // 이동 루프를 올리고 Moving 으로 — 못 올리면 그 창이 놓인 자리에 남은 채 끝낸다(알림이 오지 않으니 기다리지
    // 않는다). `from` = 로그의 앞 단계 이름.
    fn start_loop(
        &mut self,
        mut s: Session,
        from: &'static str,
        window: WindowLabel,
        kind: MoveKind,
        p: &mut dyn Ports,
    ) -> Option<Session> {
        self.last_loop += 1;
        let loop_no = self.last_loop;
        let started = p.begin_move_loop(&window, kind == MoveKind::Detached, s.id, loop_no);
        s.phase = Phase::Moving {
            window: window.clone(),
            kind,
            pending: None,
            raise: None,
            loop_no,
        };
        match started {
            Ok(()) => {
                tracing::debug!(session = s.id, from, to = s.phase.name(), window = %window, loop_no, "이동 루프 시작");
                Some(s)
            }
            Err(e) => {
                tracing::warn!(session = s.id, window = %window, "이동 루프를 못 올렸다 — 끝낸다: {e}");
                self.finish(s, stays(kind), EndBy::Failed, p);
                None
            }
        }
    }

    // A12 — 탭바 구역이면 붙기를 정하고 루프를 끝낸다 · 아니면 안내 창을 그리고 앞으로 올리기 시한을 다시 건다.
    fn follow(&mut self, s: &mut Session, window: WindowLabel, kind: MoveKind, p: &mut dyn Ports) {
        let cursor = p.cursor();
        let world = p.world();
        let under = p.probe(cursor, &window);
        let j = judge(&world, cursor, under.as_deref(), s.original);
        if let Target::Attach(to) = j.target {
            let active_before = world.window(&to).and_then(WorldWindow::active_index);
            p.overlay(None);
            if kind != (MoveKind::Whole { main: true }) {
                p.hide(&window);
            }
            s.decide(Pending::Attach { to, active_before });
            p.end_move_loop(&window);
            return;
        }
        p.overlay(j.overlay);
        self.last_timer += 1;
        let seq = self.last_timer;
        if let Phase::Moving { raise, .. } = &mut s.phase {
            *raise = Some(seq);
        }
        p.arm_raise(seq);
    }

    // A16 — 놓은 순간 커서로 지금 세계에서 다시 판정한다(마지막으로 그린 안내가 아니다 — R13c). 붙기는 여기서
    // 보지 않는다(움직일 때만 — 조사 A7).
    fn loop_drop(
        &mut self,
        mut s: Session,
        window: WindowLabel,
        kind: MoveKind,
        cursor: PhysPoint,
        end_by: EndBy,
        p: &mut dyn Ports,
    ) {
        p.overlay(None);
        let world = p.world();
        let under = p.probe(cursor, &window);
        let Target::Band { label, view, hit } =
            judge(&world, cursor, under.as_deref(), s.original).target
        else {
            self.finish(s, stays(kind), end_by, p);
            return;
        };
        let write = s.write(
            SessionDock::Seam {
                target: view,
                anchor: hit.anchor,
                side: hit.side,
                share: hit.share,
            },
            FocusPolicy::Reveal,
            None,
        );
        match p.dock(write) {
            Ok(out) => {
                s.dispose_dragging(&window, kind, out, &label, false, p);
                self.finish(s, Outcome::Dock { target: label }, end_by, p);
            }
            Err(e) => {
                tracing::warn!(session = s.id, target = %label, "띠 붙이기 실패 — 끄는 창에 남긴다: {e}");
                self.finish(s, stays(kind), EndBy::Failed, p);
            }
        }
    }

    // A17 — 그 탭바 0 자리에 붙이고(C1) 비운 끄는 창을 치운 뒤 그 창 웹뷰가 셸 커서로 몰게 한다.
    fn loop_attach(
        &mut self,
        mut s: Session,
        window: WindowLabel,
        kind: MoveKind,
        cursor: PhysPoint,
        to: WindowLabel,
        active_before: Option<usize>,
        p: &mut dyn Ports,
    ) -> Option<Session> {
        let world = p.world();
        let close_refill =
            to == MAIN_WINDOW_LABEL && s.main_refill.is_some_and(|r| refill_is_blank(&world, r));
        let write = s.write(
            SessionDock::TabBar {
                window: to.clone(),
                index: Some(0),
            },
            FocusPolicy::Reveal,
            None,
        );
        match p.dock(write) {
            Ok(out) => s.dispose_dragging(&window, kind, out, &to, true, p),
            Err(e) => {
                tracing::warn!(session = s.id, target = %to, "탭바 붙이기 실패 — 끄는 창에 남긴다: {e}");
                self.finish(s, stays(kind), EndBy::Failed, p);
                return None;
            }
        }
        p.focus(&to);
        s.before_attach = Some((to.clone(), active_before));
        s.counts.attaches += 1;
        let chrome = world.window(&to).and_then(|w| w.chrome.as_ref());
        if let (Some(chrome), Some(g)) = (chrome, p.geom(&to)) {
            let c = client_point(cursor, g.inner.origin(), g.scale);
            let step = FeedStep::Attach {
                zone: viewport_zone(chrome.tab_bar, Some(chrome)),
                grab: s.grab,
                x: c.x,
                y: c.y,
            };
            p.feed(&to, s.feed(step));
        }
        self.last_gen += 1;
        s.poll_gen = self.last_gen;
        p.poll_start(&to, s.poll_gen);
        if close_refill {
            if let Some(refill) = s.main_refill.take() {
                p.close_tab(MAIN_WINDOW_LABEL, refill);
            }
        }
        let from = s.phase.name();
        s.phase = Phase::Strip {
            holder: to.clone(),
            feed: Feed::Shell,
            releasing: None,
            last_reorder_seq: 0,
        };
        tracing::debug!(session = s.id, from, to = s.phase.name(), window = %to, "붙기 — 그 창 탭바로");
        Some(s)
    }

    // 되돌리기(TRD §6-2) — 탭을 원래 창 O 의 원래 자리로 · 붙었던 창 W 의 붙기 전 선택(C8 · E11) · O 의 처음
    // 선택 · 창째 끌기면 O 의 원래 자리 · 최대화. 쓰기가 실패해도 끝까지 간다 — 탭은 지금 든 창에 남는다.
    fn revert(&mut self, mut s: Session, end_by: EndBy, p: &mut dyn Ports) {
        let world = p.world();
        let o = s.origin.window.clone();
        let holding = world.dragged.as_ref().and_then(|d| d.window.clone());
        let w_select = match (&holding, &s.before_attach) {
            (Some(h), Some((w, Some(a)))) if h == w && *h != o => world
                .window(w)
                .and_then(|ww| ww.tabs.iter().filter(|t| **t != s.view).nth(*a).copied())
                .map(|v| (w.clone(), v)),
            _ => None,
        };
        let refill_blank = s.main_refill.is_some_and(|r| refill_is_blank(&world, r));
        let back_tab_bar = SessionDock::TabBar {
            window: o.clone(),
            index: Some(s.origin.index),
        };
        let mut back = false;
        let mut readopted = false;
        if let Some(e) = s.emptied.take() {
            let write = s.write(
                SessionDock::AdoptWindow {
                    label: e.label.clone(),
                    seed: Some(e.seed.clone()),
                    measures: e.measures.clone(),
                },
                FocusPolicy::Keep,
                None,
            );
            match p.dock(write) {
                Ok(out) => {
                    give_back_emptied(out, p);
                    // 보이기(`restore_placement`) 전에 명부에서 빼 웹뷰가 대기 모드(빈 틀)를 먼저 풀게 한다.
                    p.unpark(&o);
                    p.restore_placement(&o, s.origin.start, None, true);
                    back = true;
                    readopted = true;
                }
                Err(err) => {
                    tracing::warn!(session = s.id, window = %o, "남긴 창 되살리기 실패 — 끝에 닫는다: {err}");
                    s.emptied = Some(e);
                }
            }
        } else if let Some(ow) = world.window(&o) {
            if holding.as_deref() == Some(o.as_str()) {
                let now = ow.tabs.iter().position(|t| *t == s.view);
                if now != Some(s.origin.index) {
                    if let Err(e) = p.dock(s.write(back_tab_bar, FocusPolicy::Keep, None)) {
                        tracing::warn!(session = s.id, "원래 자리 쓰기 실패: {e}");
                    }
                }
                back = true;
            } else {
                match p.dock(s.write(back_tab_bar, FocusPolicy::Keep, None)) {
                    Ok(out) => {
                        give_back_emptied(out, p);
                        back = true;
                        if refill_blank && o == MAIN_WINDOW_LABEL {
                            if let Some(refill) = s.main_refill.take() {
                                p.close_tab(MAIN_WINDOW_LABEL, refill);
                            }
                        }
                    }
                    Err(e) => tracing::warn!(
                        session = s.id,
                        "원래 창으로 되돌리기 실패 — 탭은 지금 창에 남는다: {e}"
                    ),
                }
            }
        }
        if back {
            if let Some((w, v)) = w_select {
                p.switch_tab(&w, v);
            }
            let want = s.origin.active.unwrap_or(s.view);
            if p.world().window(&o).is_some_and(|w| w.active != want) {
                p.switch_tab(&o, want);
            }
            if s.origin.single && !readopted {
                p.restore_placement(&o, s.origin.start, None, true);
            }
        }
        self.finish(s, Outcome::Reverted, end_by, p);
    }

    // 공통 마무리(TRD §6-1) — 어느 전이에서 끝나도 이 하나를 지난다.
    fn finish(&mut self, s: Session, outcome: Outcome, end_by: EndBy, p: &mut dyn Ports) {
        let holding = p.world().dragged.and_then(|d| d.window);
        p.overlay(None);
        p.poll_stop();
        p.release_lease(s.lease);
        // 이 세션이 숨기는 창은 붙기를 정한 끄는 창과(루프 Esc 훅이 숨긴) 예비 창뿐이다 — 빈 채 남은 것은 명부로
        // 가 숨은 채 기다리고 탭을 쥔 채 남은 것은 여기서 보인다.
        if let Some(w) = &holding {
            p.show_in_place(w);
        }
        if let Some(e) = &s.emptied {
            p.close_window(&e.label);
        }
        match (&outcome, &s.phase) {
            (Outcome::Strip, Phase::Strip { holder, .. }) if *holder != s.origin.window => {
                p.focus(holder)
            }
            (Outcome::Dock { target }, _) => p.focus(target),
            // 탭이 돌아간 출처를 늘 활성으로 — Chrome `TabDragController::RevertDrag` 가 끝에 출처 창을 조건 없이
            // 활성으로 한다(붙었던 창 · 숨는 예비 창이 쥐던 활성도 그래서 출처로 간다).
            (Outcome::Reverted, _) if holding.as_deref() == Some(s.origin.window.as_str()) => {
                p.focus(&s.origin.window)
            }
            (Outcome::NewWindow | Outcome::Moved, Phase::Moving { window, .. }) => {
                p.record_placement(window)
            }
            _ => {}
        }
        p.drag(s.drag_event(TabDragPhase::Ended), None);
        p.fill_spare();
        tracing::info!(
            session = s.id,
            outcome = %outcome,
            end_by = %end_by,
            reorders = s.counts.reorders,
            detaches = s.counts.detaches,
            attaches = s.counts.attaches,
            elapsed_ms = s.started.elapsed().as_millis() as u64,
            "끌기 끝"
        );
    }
}

impl Session {
    fn write(&self, to: SessionDock, focus: FocusPolicy, successor: Option<ViewId>) -> DockWrite {
        DockWrite {
            view: self.view,
            lease: self.lease,
            to,
            focus,
            successor,
        }
    }

    fn drag_event(&self, phase: TabDragPhase) -> TabDragEvent {
        TabDragEvent {
            session: self.id,
            phase,
        }
    }

    fn feed(&self, step: FeedStep) -> TabDragFeed {
        TabDragFeed {
            session: self.id,
            view: self.view,
            step,
        }
    }

    fn decide(&mut self, decided: Pending) {
        let from = self.phase.name();
        if let Phase::Moving { pending, raise, .. } = &mut self.phase {
            *pending = Some(decided);
            *raise = None;
        }
        if let Phase::Moving { window, .. } = &self.phase {
            tracing::debug!(session = self.id, from, to = self.phase.name(), window = %window, "루프 끝 까닭을 정했다");
        }
    }

    // 떼는 창의 다음 활성(TRD §7 ④) — 그 창에 붙었다가 떼는 것이면 붙기 전 활성 자리(끄는 탭을 뺀 목록에서 센다 —
    // E11) · 출처면 누르기 전 활성 · 아니면 승계 규칙.
    fn detach_successor(&self, holder: &WorldWindow) -> Option<ViewId> {
        match &self.before_attach {
            Some((w, a)) if *w == holder.label => a.and_then(|a| {
                holder
                    .tabs
                    .iter()
                    .filter(|t| **t != self.view)
                    .nth(a)
                    .copied()
            }),
            _ if holder.label == self.origin.window => self.origin.active,
            _ => None,
        }
    }

    // 쓰기로 비운 끄는 창을 치운다 — 떼어진 창은 예비로 · 창째 끌던 창은 남긴 창으로(`park` = 탭바 붙기 — 되돌리기가
    // 다시 들일 수 있게 숨긴 채 둔다 · 띠 붙기는 끝에 닫으므로 두지 않는다) · 메인은 빈 탭이 생겨 남으므로 원래
    // 자리로 붙은 창 뒤에(R14).
    fn dispose_dragging(
        &mut self,
        window: &str,
        kind: MoveKind,
        out: DockOutcome,
        target: &str,
        park: bool,
        p: &mut dyn Ports,
    ) {
        match kind {
            MoveKind::Detached => give_back_emptied(out, p),
            MoveKind::Whole { main: false } => {
                if let Some(e) = &out.emptied {
                    if park {
                        p.park_kept(&e.label, e.measures.clone());
                    }
                }
                self.emptied = out.emptied;
            }
            MoveKind::Whole { main: true } => {
                self.main_refill = out.main_refilled;
                p.restore_placement(window, self.origin.start, Some(target), false);
            }
        }
    }
}

fn give_back_emptied(out: DockOutcome, p: &mut dyn Ports) {
    if let Some(e) = out.emptied {
        p.give_back(&e.label, e.measures);
    }
}

fn stays(kind: MoveKind) -> Outcome {
    match kind {
        MoveKind::Detached => Outcome::NewWindow,
        MoveKind::Whole { .. } => Outcome::Moved,
    }
}

// G14 — 메인의 빈 탭이 아직 손대지 않은 그대로인가(활성이고 빈 칸 하나).
fn refill_is_blank(world: &World, refill: ViewId) -> bool {
    world.window(MAIN_WINDOW_LABEL).is_some_and(|m| {
        m.active == refill
            && matches!(
                m.layout,
                Some(LayoutNode::Slot {
                    content: SlotContent::Empty,
                    ..
                })
            )
    })
}

/// 탭 끌기 영역 `area`(뷰포트 CSS)의 탭바 구역 — 셸 판정(`targets::judge`)과 같은 정수 경계를 뷰포트 좌표로 되돌린
/// 것. 웹뷰가 「나갔나」를 보는 사각형과 떼어진 창이 「닿았나」를 보는 사각형이 같아야 한다(PRD D19). 크롬 보고가
/// 없으면 캔버스 원점을 0 으로 친다.
fn viewport_zone(area: RectCss, chrome: Option<&DockChrome>) -> RectCss {
    let o = chrome.map_or(PointCss { x: 0.0, y: 0.0 }, |c| c.canvas_origin);
    let z = zone_in_canvas(strip_zone(area), o);
    RectCss {
        x0: z.x0 + o.x,
        y0: z.y0 + o.y,
        x1: z.x1 + o.x,
        y1: z.y1 + o.y,
    }
}

fn screen_point(p: PointCss, geom: &WindowGeom) -> PhysPoint {
    PhysPoint {
        x: geom.inner.x0 + (p.x * geom.scale).round() as i32,
        y: geom.inner.y0 + (p.y * geom.scale).round() as i32,
    }
}

fn outer_dip(g: &WindowGeom) -> SizeDip {
    let size = g.outer.size();
    SizeDip {
        w: f64::from(size.w) / g.scale,
        h: f64::from(size.h) / g.scale,
    }
}

fn original_of(g: &WindowGeom, canvas_origin: Option<PointCss>) -> Option<SizeDip> {
    original_size(g.inner.size(), g.scale, canvas_origin?)
}

// 되돌릴 자리 — 최대화면 모델의 보통 자리 원점 + 복원 바깥 크기 · 아니면 지금 바깥 사각형.
fn start_placement(w: &WorldWindow, geom: &WindowGeom, restored: SizeDip) -> Placement {
    if !w.attrs.maximized {
        return Placement {
            normal: geom.outer,
            maximized: false,
        };
    }
    let origin = w.attrs.bounds.map_or(geom.outer.origin(), |b| PhysPoint {
        x: b.x().round() as i32,
        y: b.y().round() as i32,
    });
    Placement {
        normal: PhysRect::at(origin, restored.to_phys(geom.scale)),
        maximized: true,
    }
}

// C4 — 창째 끌기 전 자리: 최대화면 풀며 크기를 떼어진 창 크기로 · 원점은 시작 원점 + (커서 − 누른 점).
fn whole_plan(
    geom: &WindowGeom,
    start: Placement,
    restored: SizeDip,
    down: PhysPoint,
    cursor: PhysPoint,
    monitors: &[super::units::Monitor],
) -> WholePlan {
    if !start.maximized {
        return WholePlan::Move(whole_window_origin(geom.outer.origin(), down, cursor));
    }
    let maximized = outer_dip(geom);
    let work = work_area_near(monitors, cursor).unwrap_or(maximized);
    let size = dragged_size(restored, Some(maximized), work).to_phys(geom.scale);
    WholePlan::Restore(PhysRect::at(
        whole_window_origin(start.normal.origin(), down, cursor),
        size,
    ))
}
