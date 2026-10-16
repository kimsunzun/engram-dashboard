//! 끌기 장부(TRD S21-slot-docking §1-3) — 끌기 세션 하나의 순수 상태 기계. 입력([`Input`]) 하나를 받아 할 일
//! ([`Action`])과 답([`Reply`])을 낸다. Tauri · 락 · 시계 읽기 · OS 호출 0 — 시각 · 커서 · 창 사각형 · 모델
//! 스냅숏은 전부 입력이라 같은 입력 열이면 같은 할 일 열이다.
//!
//! 진입점: [`Book::step`] · [`Book::admits`](시작을 받을지 — 실행기가 레이아웃 락을 잡기 **전에** 묻는다).
//!
//! 부르는 쪽(실행기)의 몫:
//! - 장부를 잎 락(`Mutex<Book>`)으로 쥐고, 쥔 채 레이아웃 락 · emit · 창 호출 · 적용 서비스 · 커서 읽기를 하지
//!   않는다 — 레이아웃 락을 쥔 채 장부 락을 잡는 것은 되고, 거꾸로는 안 된다(시작도 `admits` 뒤 장부 락을 놓고
//!   레이아웃 락을 잡는다). 할 일은 장부 락 · 레이아웃 락을 모두 놓은 뒤 낸 순서대로 실행한다(`ReleaseLease` 는
//!   레이아웃 락을 잡고 `Show` · `Hide` · `Focus` 는 창 호출이다) — 단 `Apply` 는 먹인 스레드와 무관하게 늘
//!   async 로 넘긴다(동기 명령 안에서 새 창을 만들면 교착 — TRD §1-3 「명령의 실행 스레드」).
//! - 세션 깃발(세대)은 시작의 레이아웃 락 안에서 세우고, 자기 세대일 때만(비교-교체) 0 으로 내린다 — 끝은
//!   [`EndLog::generation`] 으로, `Start` 의 거절은 [`Reply::Rejected`] 를 받은 그 자리에서.
//! - 시작을 거절하면(`admits` · `Start` 어느 쪽이든) [`Input::Refused`] 를 먹인다. `AlreadyDragging{applying:
//!   false}` 면 옛 세션도 [`Input::Superseded`] 로 끝낸다 — 「마지막 점에 놓기」는 커서 · 창 사각형이 있어야 해서
//!   장부가 거절하는 자리에서 혼자 끝내지 못한다.
//! - [`Input::Applied`] · [`Input::Timeout`] 은 낸 할 일에 정확히 맞춰 먹인다(각 문서) — 어긋나면 세션이 끝을
//!   잃는다. [`Snap`] 은 뜨는 자리가 정해져 있다(그 문서).
//!
//! ★장부는 패닉하지 않는다(TRD §11-2 #12)★ — 모르는 label · id · 순서가 어긋난 입력은 debug 로그를 남기고
//! 버린다. 예외 하나 — [`WindowSnap::visible`] 이 `regions::visible_tab` 과 어긋나면 debug 빌드는
//! `regions::regions_for` 가 멈춘다(스냅숏을 뜨는 쪽의 결함 신호).

use std::collections::{HashMap, HashSet, VecDeque};

use super::regions::{self, Expect, WindowSnap};
use super::types::{
    DockRegions, DragEffect, DropReport, Picked, SessionEvent, SessionPhase, Started,
};
use crate::layout::apply::{DockDest, DockOutcome, DockRequest, WindowPlace};
use crate::layout::chrome::PointCss;
use crate::layout::dock::DockAnchor;
use crate::layout::manager::{
    succession, DockError, DockGuard, FocusPolicy, LeaseId, TargetExpect, ViewId, WindowLabel,
};
use crate::layout::LayoutNode;

/// 출처 끝(none) 앞의 대상 떠남이 이보다 가까우면 Esc 취소가 낸 떠남으로 보고 그 창을 「아직 위」로 친다 — S1 이
/// 간격을 다시 잰다.
pub const LEAVE_GRACE_MS: u64 = 100;
/// 출처 끝(move) 뒤 대상의 놓기를 기다리는 시간 — 두 웹뷰의 invoke 순서는 보장되지 않는다.
pub const DROP_WAIT_MS: u64 = 500;
/// 끝난 뒤의 포인터 이동을 본 뒤 끝 신호를 기다리는 시간 — 넘으면 신호 유실로 친다.
pub const END_GRACE_MS: u64 = 300;
/// 모르는 `drag_id` 의 끝 신호를 그 시작을 기다리며 붙드는 시간 — S1 ⑧ 이 잰다.
pub const EARLY_SIGNAL_MS: u64 = 500;

// 늦은 신호(끝난 세션)와 이른 신호(아직 안 온 시작)를 가를 만큼만 — 사람 손의 끌기가 넷을 겹치지 않는다.
const ENDED_KEPT: usize = 4;

/// 출처 웹뷰가 끌기마다 만든 UUID 문자열.
pub type DragId = String;

/// 단조 시계의 ms — 실행기가 읽어 싣는다. 원점은 실행기 몫이고 장부는 차만 본다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct MonoMs(pub u64);

impl MonoMs {
    /// `earlier` 부터 지난 ms — 거꾸로면 0.
    pub fn since(self, earlier: MonoMs) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
}

/// 화면의 물리 px 점(Tauri `cursor_position` 과 같은 단위).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhysPoint {
    pub x: f64,
    pub y: f64,
}

/// 우리 창 하나의 클라이언트 영역 — `origin` = 물리 안쪽 왼쪽 위(`inner_position`) · `size` = 물리 안쪽 크기
/// (`inner_size`) · `scale` = 그 창 배율.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClientRect {
    pub origin: (i32, i32),
    pub size: (u32, u32),
    pub scale: f64,
}

/// 끝 신호 순간의 창 사각형 — 실행기가 레이아웃 락도 장부 락도 쥐지 않고 읽는다(TRD §1-3 「창 게터」).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Rects {
    /// 보이고 최소화 안 된 우리 창만. 없는 label = 소멸 · 숨김 · 최소화 · 못 읽음.
    pub windows: HashMap<WindowLabel, ClientRect>,
    /// 출처 창 배율 — 숨긴 출처도 읽는다. `None` = 못 읽음(새 창 자리를 1 로 셈한다).
    pub source_scale: Option<f64>,
}

/// 웹뷰가 보인 것 하나 — 대상 창의 `Over` 보고와 놓기가 같이 싣는 칸(뜻 = [`super::types::DragOverReport`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub rev: u64,
    pub bar_rev: u64,
    pub displayed: Option<ViewId>,
    pub shown: Picked,
    pub point: PointCss,
}

impl From<&DropReport> for Report {
    fn from(d: &DropReport) -> Self {
        Self {
            rev: d.rev,
            bar_rev: d.bar_rev,
            displayed: d.displayed,
            shown: d.shown.clone(),
            point: d.point,
        }
    }
}

/// 모델 스냅숏 — 실행기가 레이아웃 락 안에서 뜨고 같은 락 안에서 올린 번호를 단다. `source_layout` 과 창마다의
/// `visible` 은 모델 표식이 쥔 View(끄는 탭)로 그 같은 락 안에서 뜬다 — 장부나 락 밖에 둔 값으로 뜨면, `Start`
/// 전에 장부가 들고 있다 이어 먹는 스냅숏이 다른 끌기의 View 로 떠져 debug 빌드의 `regions::regions_for` 대조가
/// 멈춘다.
#[derive(Debug, Clone, PartialEq)]
pub struct Snap {
    /// 레이아웃 락 안의 단조 셈 — 장부는 마지막으로 먹은 번호 이하를 버린다(엇갈린 두 계기의 옛 스냅숏이 새
    /// 영역을 덮지 않게).
    pub number: u64,
    /// 끄는 탭의 지금 배치. `None` = 그 View 가 모델에 없다.
    pub source_layout: Option<LayoutNode>,
    /// 모든 창의 영역 재료. `visible` 은 `regions::visible_tab` 으로 채운다 · `hidden` · `is_source` 는 장부가
    /// 덮어쓴다(숨김은 장부 값이다 — `hide()` 직후의 보임 게터는 늦을 수 있다).
    pub windows: Vec<WindowSnap>,
}

/// 장부에 먹이는 사건.
#[derive(Debug, Clone)]
pub enum Input {
    /// 시작 확정 — 실행기가 [`Book::admits`] 를 거친 뒤 레이아웃 락 한 번 안에서 표식(`lease`) · 세션 세대
    /// (`generation`) · 출처 배치 복제(`expect_source`) · 시작 스냅숏(`snap`)을 뜬 다음 먹인다.
    Start {
        drag_id: DragId,
        view_id: ViewId,
        /// 끌기 그림 안의 커서 자리 — 새 창 자리 = 커서 − 이것 × 출처 배율.
        hotspot: PointCss,
        lease: LeaseId,
        generation: u64,
        source_label: WindowLabel,
        expect_source: LayoutNode,
        /// 출처가 탭 하나뿐인 팝아웃 — 끄는 동안 숨긴다.
        single_tab_popout: bool,
        snap: Snap,
        now: MonoMs,
    },
    /// 그 id 의 시작을 거절했다(장부 · 실행기 어느 쪽이든) — 그 id 로 붙든 끝 신호를 버리고 그 id 를 끝난 id 로
    /// 적는다(뒤늦은 끝 신호가 다시 붙들려 다른 id 의 이른 신호를 밀어내지 않게).
    Refused {
        drag_id: DragId,
    },
    Snapshot(Snap),
    Over {
        drag_id: DragId,
        label: WindowLabel,
        report: Report,
        now: MonoMs,
    },
    Left {
        drag_id: DragId,
        label: WindowLabel,
        now: MonoMs,
    },
    /// 대상 창 `label` 의 놓기. `view_id` = 끌기 데이터의 탭 · `cursor` = 그 순간 셸 커서(`None` = 못 읽음).
    Drop {
        drag_id: DragId,
        view_id: ViewId,
        label: WindowLabel,
        report: Report,
        cursor: Option<PhysPoint>,
        rects: Rects,
        now: MonoMs,
    },
    /// 출처의 `dragend`. `cursor` · `rects` = 그 순간 읽은 것 — `effect ≠ none` 이면 놓기를 기다리다 시한이 지났을
    /// 때도 이 커서로 놓는다.
    SourceEnd {
        drag_id: DragId,
        effect: DragEffect,
        cursor: Option<PhysPoint>,
        rects: Rects,
        now: MonoMs,
    },
    /// 끝난 뒤의 포인터 이동 · 누름을 본 웹뷰가 알렸다.
    PointerSeen {
        drag_id: DragId,
        now: MonoMs,
    },
    /// [`Action::ArmTimer`] 가 건 시한 — `kind` · `number` 는 그 할 일 그대로 싣는다. 번호가 지금 걸린 시한이
    /// 아니면 버리고, 번호가 맞아도 종류가 지금 단계와 다르면 버린다 — 그때 걸린 시한은 그대로라 다시 오지 않아
    /// 세션이 끝을 잃는다. `cursor` · `rects` = 그 순간 읽은 것(`Early` 는 쓰지 않는다 · `Drop` 은 사각형만 쓰고
    /// 커서는 끝 신호 때 것을 쓴다).
    Timeout {
        kind: TimerKind,
        number: u64,
        cursor: Option<PhysPoint>,
        rects: Rects,
        now: MonoMs,
    },
    /// 새 시작이 살아 있는 옛 세션 `drag_id` 를 만났다 — 그 세션은 끝 신호를 잃었으니 「마지막 점에 놓기」로
    /// 끝낸다. 실행기는 시작을 `AlreadyDragging{applying: false}` 로 거절한 뒤([`Book::admits`] · [`Input::Start`]
    /// 어느 쪽이든) 그때의 [`Book::live_drag_id`] 를 싣고, async 한 건에서 커서 · 창 사각형을 읽어 먹인다(TRD
    /// §1-3 「시작」 1 · §7 #12). 그 사이 그 세션이 끝났거나 적용 중이 됐으면 버린다.
    Superseded {
        drag_id: DragId,
        cursor: Option<PhysPoint>,
        rects: Rects,
        now: MonoMs,
    },
    /// 우리 창 하나가 사라졌다(창 `Destroyed`).
    Destroyed {
        label: WindowLabel,
    },
    /// [`Action::Apply`] 의 결과 — 적용 서비스가 알림 · 묶음 끝 · 빈 팝아웃 닫기까지 마친 뒤 같은 스레드에서.
    /// `Apply` 하나마다 정확히 한 번(Ok · Err 무관) 먹인다 — 적용 중(`Applying`)에는 시한이 없어 빠뜨리면 세션이
    /// 표식째 남는다.
    Applied(Result<DockOutcome, DockError>),
    /// 영역 없이 끌기를 만난 웹뷰가 당긴다 — 장부가 가진 마지막 스냅숏 재료로 다시 만든다(새 재료가 필요하면
    /// 실행기가 스냅숏을 먼저 먹인다).
    Pull {
        label: WindowLabel,
    },
}

/// 장부가 낸 할 일 — 낸 순서대로 실행한다.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// `dock:session` 전체 emit.
    Broadcast(SessionEvent),
    /// `dock:regions` 를 그 창에만.
    Regions {
        label: WindowLabel,
        regions: DockRegions,
    },
    Hide(WindowLabel),
    /// 숨긴 출처를 되보인다 — 미룬 최대화를 입히는 보이기 경로로.
    Show(WindowLabel),
    Focus(WindowLabel),
    Apply(DockRequest),
    ArmTimer {
        kind: TimerKind,
        number: u64,
        after_ms: u64,
    },
    /// 이미 내려간 표식이면(적용 성공) 무동작이다.
    ReleaseLease(LeaseId),
    EndLog(EndLog),
}

/// 시한 셋 — `Drop` · `Grace` 는 세션 시한(한 번에 하나) · `Early` 는 붙든 이른 끝 신호의 시한(따로 하나).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerKind {
    Drop,
    Grace,
    Early,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Step {
    pub actions: Vec<Action>,
    pub reply: Reply,
}

impl From<Vec<Action>> for Step {
    fn from(actions: Vec<Action>) -> Self {
        Self {
            actions,
            reply: Reply::Nothing,
        }
    }
}

/// 부른 invoke 에 돌려줄 답 — 그 입력만 낸다(`Start` = 거절 · `Pull` = 당긴 것).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Reply {
    #[default]
    Nothing,
    /// 시작 거절 — 실행기는 함께 낸 할 일(`ReleaseLease`)을 실행하고 자기 세대의 세션 깃발을 비교-교체로 0 으로
    /// 내린 뒤, 사유를 오류로 돌려주고 [`Input::Refused`] 를 먹인다. `Start` 의 거절은 `admits` 를 통과한 뒤
    /// 그 사이 그 id 가 끝난 id 로 적힌 경우다(이른 신호 시한 · 다른 모르는 id 의 밀어냄).
    Rejected(Rejection),
    /// `None` = 세션이 없다.
    Pulled(Option<PullReply>),
}

/// `dock_pull` 의 답 — `regions` = `None` 이면 그 창에 보낼 영역이 없다(숨긴 출처 · 재료 없음 · 사라진 창).
#[derive(Debug, Clone, PartialEq)]
pub struct PullReply {
    pub started: Started,
    pub regions: Option<DockRegions>,
}

/// 장부가 정하는 시작 거절 — 실행기가 정하는 거절(`TabNotFound` · `Dragging` · `LayoutPoisoned`)은 장부까지 오지
/// 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// 세션이 있다. `applying` = 그 세션이 적용 중 — 끝내지 않고 `Applied` 가 끝을 정한다. 아니면 그 세션은 끝
    /// 신호를 잃었다 — 실행기가 [`Input::Superseded`] 로 끝낸다.
    AlreadyDragging { applying: bool },
    /// 최근 끝난 id — 붙들었던 끝 신호를 이미 버린 끌기라 열면 끝 신호 없는 세션이 된다.
    LateStart,
}

impl Rejection {
    /// 거절 로그의 `reason`.
    pub fn word(self) -> &'static str {
        match self {
            Rejection::AlreadyDragging { applying: true } => "applying",
            Rejection::AlreadyDragging { applying: false } => "AlreadyDragging",
            Rejection::LateStart => "LateStart",
        }
    }
}

/// 끝 한 줄의 재료(TRD §1-7 「장부 끝」) — 실행기가 `errors` 셈 · `elapsed_ms`(지금 − `started_at`)를 더해 info 로
/// 남기고, `generation` 이 자기 세대일 때만 세션 깃발을 내린다.
#[derive(Debug, Clone, PartialEq)]
pub struct EndLog {
    pub drag_id: DragId,
    pub generation: u64,
    pub outcome: Outcome,
    /// `None` = 붙음뿐이다.
    pub reason: Option<EndReason>,
    pub end_by: Option<EndBy>,
    /// 적용을 낸 모드 — `None` = 적용 없이 끝났다.
    pub mode: Option<Mode>,
    /// 대상 창 — 새 창은 적용 결과에서 안다.
    pub window: Option<WindowLabel>,
    /// 판정한 창 W.
    pub over_window: Option<WindowLabel>,
    /// 「마지막 점에 놓기」의 끝 신호와 W 떠남 보고의 간격.
    pub leave_gap_ms: Option<u64>,
    /// 커서를 읽었나 — `None` = 커서를 쓰지 않는 끝.
    pub cursor_read: Option<bool>,
    /// 붙든 이른 신호로 끝났나.
    pub early: bool,
    pub reports: u64,
    pub regions_sent: u64,
    pub regions_resent: u64,
    pub started_at: MonoMs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Applied,
    /// 판정했으나 바꾸지 않았다(장부 대조 · 적용 서비스 거절).
    Noop,
    /// 판정 없이 끝났다 — 출처가 사라졌다.
    Aborted,
}

impl Outcome {
    pub fn word(self) -> &'static str {
        match self {
            Outcome::Applied => "applied",
            Outcome::Noop => "noop",
            Outcome::Aborted => "aborted",
        }
    }
}

/// 끝 로그의 `reason` — 장부가 정한 것과 적용 서비스가 거절한 것.
#[derive(Debug, Clone, PartialEq)]
pub enum EndReason {
    /// 출처 탭 배치가 시작 때와 다르다(장부 `blocked`).
    SourceChanged,
    /// 보인 가이드와 지금 판정이 다르다(장부 대조).
    TargetChanged,
    SourceDestroyed,
    SourceGone,
    Refused(DockError),
}

impl EndReason {
    pub fn word(&self) -> &'static str {
        match self {
            EndReason::SourceChanged | EndReason::Refused(DockError::SourceChanged) => {
                "SourceChanged"
            }
            EndReason::TargetChanged | EndReason::Refused(DockError::TargetChanged) => {
                "TargetChanged"
            }
            EndReason::SourceDestroyed => "SourceDestroyed",
            EndReason::SourceGone => "SourceGone",
            EndReason::Refused(e) => match e {
                DockError::TabNotFound(_) => "TabNotFound",
                DockError::WindowNotFound(_) => "WindowNotFound",
                DockError::SlotNotFound(_) => "SlotNotFound",
                DockError::SlotNotInTab { .. } => "SlotNotInTab",
                DockError::SelfTarget => "SelfTarget",
                DockError::ZeroArea => "ZeroArea",
                DockError::Dragging => "Dragging",
                DockError::WindowClosed(_) => "WindowClosed",
                DockError::StaleLease => "StaleLease",
                DockError::SourceChanged => "SourceChanged",
                DockError::TargetChanged => "TargetChanged",
                DockError::WindowExists(_) => "WindowExists",
                DockError::SlotIdOverlap => "SlotIdOverlap",
                DockError::WindowOpen(_) => "WindowOpen",
            },
        }
    }
}

/// 끝을 부른 것 — 끝 로그의 `end_by`. `SourceGone` 은 출처 View 없음과 출처 창 소멸 둘 다다(사유가 가른다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndBy {
    Drop,
    SourceNone,
    DropWait,
    SignalLost,
    NewStart,
    SourceGone,
}

impl EndBy {
    pub fn word(self) -> &'static str {
        match self {
            EndBy::Drop => "drop",
            EndBy::SourceNone => "source_none",
            EndBy::DropWait => "drop_wait",
            EndBy::SignalLost => "signal_lost",
            EndBy::NewStart => "new_start",
            EndBy::SourceGone => "source_gone",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Seam,
    TabBar,
    NewWindow,
    /// 새 창 판정이 출처 팝아웃을 옮겼다(모델 변경 없음).
    Reuse,
}

impl Mode {
    pub fn word(self) -> &'static str {
        match self {
            Mode::Seam => "seam",
            Mode::TabBar => "tab_bar",
            Mode::NewWindow => "new_window",
            Mode::Reuse => "reuse",
        }
    }
}

/// 끌기 장부 — 세션은 한 번에 하나다.
#[derive(Debug, Default)]
pub struct Book {
    // 끝난 세션은 `Phase::Ended` 로 다음 시작까지 남는다.
    drag: Option<Drag>,
    last_snap: u64,
    // 세션 전(시작 락 뒤 · `Start` 전)에 온 스냅숏 중 가장 새것 — `Start` 가 이어 먹는다.
    held_snap: Option<Snap>,
    // 모르는 drag_id 의 끝 신호 — 한 번에 한 id.
    early: Option<Early>,
    ended: VecDeque<DragId>,
    // 시한 번호 — 세션 · 이른 신호가 같은 셈을 써서 지난 세션의 시한과도 겹치지 않는다.
    timer_seq: u64,
}

#[derive(Debug)]
struct Drag {
    id: DragId,
    generation: u64,
    lease: LeaseId,
    seq: u64,
    hotspot: PointCss,
    source_label: WindowLabel,
    source_view: ViewId,
    expect_source: LayoutNode,
    source_hidden: bool,
    // 적용 중에 출처 창이 사라짐 — 끝의 되보이기를 건너뛴다.
    source_destroyed: bool,
    display_active: Option<ViewId>,
    // 한 번 서면 내리지 않는다 — 시작 때 배치로 돌아와도 그 사이 바뀐 적이 있다.
    blocked: bool,
    // 마지막 스냅숏의 창별 재료(`Pull` 이 다시 만든다).
    windows: HashMap<WindowLabel, WindowSnap>,
    // 이 세션에 `Destroyed` 가 온 창 — 그 뒤 도착한 옛 스냅숏이 되살리지 않는다(팝아웃 label 은 한 부팅 안에서
    // 다시 쓰이지 않는다).
    gone: HashSet<WindowLabel>,
    // 창별 마지막 (rev, bar_rev) — 영역을 지워도 남아 다시 보낼 때 단조를 지킨다.
    revs: HashMap<WindowLabel, (u64, u64)>,
    // 창별 지금 영역 — 놓기 대조의 기준.
    sent: HashMap<WindowLabel, Sent>,
    seen: HashMap<WindowLabel, Seen>,
    // Over · Left 의 도착 순번.
    arrivals: u64,
    // 지금 걸린 세션 시한 번호.
    timer: Option<u64>,
    phase: Phase,
    started_at: MonoMs,
    cause: Option<EndCause>,
    mode: Option<Mode>,
    window: Option<WindowLabel>,
    reports: u64,
    regions_sent: u64,
    regions_resent: u64,
}

#[derive(Debug, Clone)]
struct Sent {
    // drag_id · rev · bar_rev 를 채운 그대로.
    regions: DockRegions,
    expect: Expect,
}

#[derive(Debug, Default)]
struct Seen {
    over: Option<OverSeen>,
    // Left 는 Over 를 지우지 않는다 — 유예 안 Left 의 그 Over 를 되찾아야 한다.
    left: Option<Arrived>,
}

#[derive(Debug)]
struct OverSeen {
    report: Report,
    arrived: Arrived,
}

#[derive(Debug, Clone, Copy)]
struct Arrived {
    order: u64,
    at: MonoMs,
}

#[derive(Debug)]
struct Early {
    drag_id: DragId,
    // 도착 순서대로.
    signals: Vec<EndSignal>,
    timer: u64,
}

// 붙들 수 있는 끝 신호 — 놓기와 출처 끝(이른 신호는 이 둘뿐이다).
#[derive(Debug)]
enum EndSignal {
    Drop {
        view_id: ViewId,
        label: WindowLabel,
        report: Report,
        cursor: Option<PhysPoint>,
        rects: Rects,
    },
    SourceEnd {
        effect: DragEffect,
        cursor: Option<PhysPoint>,
        rects: Rects,
        now: MonoMs,
    },
}

// `at` = 끝 신호(유실이면 끝 증거)의 도착 — 「마지막 점에 놓기」의 떠남 유예 기준.
#[derive(Debug)]
enum Phase {
    Live,
    // `cursor` = 그 끝 신호 때의 커서 · `early` = 그 끝 신호가 붙들었던 이른 신호였다.
    AwaitDrop {
        at: MonoMs,
        cursor: Option<PhysPoint>,
        early: bool,
    },
    Grace {
        at: MonoMs,
    },
    Applying,
    Ended,
}

// 「마지막 Over 창」 W 와 그 마지막 보고.
struct LastOver {
    label: WindowLabel,
    report: Report,
    still_over: bool,
    // W 의 마지막 Over 뒤에 온 Left 와 끝 신호의 간격 — 없으면 `None`.
    leave_gap_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
struct EndCause {
    end_by: EndBy,
    over_window: Option<WindowLabel>,
    cursor_read: Option<bool>,
    early: bool,
    leave_gap_ms: Option<u64>,
}

impl EndCause {
    fn by(end_by: EndBy) -> Self {
        Self {
            end_by,
            over_window: None,
            cursor_read: None,
            early: false,
            leave_gap_ms: None,
        }
    }
}

enum Ending {
    Applied(DockOutcome),
    Noop(EndReason),
    Aborted(EndReason),
}

enum Verdict {
    Refuse(EndReason),
    NewWindow,
    Dock {
        to: DockDest,
        expect: Option<TargetExpect>,
        mode: Mode,
        window: WindowLabel,
    },
}

fn is_live(d: &Drag) -> bool {
    !matches!(d.phase, Phase::Ended)
}

impl Book {
    pub fn new() -> Self {
        Self::default()
    }

    /// 지금 시작 `drag_id` 를 받을지. 실행기는 표식 · 세션 깃발을 세우기 전에(장부 락 안에서) 묻는다 — `Start` 도
    /// 같은 대조를 다시 하지만, 그때는 이미 깃발을 새 세대로 덮어 옛 세션의 계기가 끊긴 뒤다. 시작은 메인 스레드
    /// 동기 명령이라 이 물음과 `Start` 사이에 세션이 새로 서지 않는다.
    pub fn admits(&self, drag_id: &str) -> Result<(), Rejection> {
        if let Some(d) = self.drag.as_ref().filter(|d| is_live(d)) {
            return Err(Rejection::AlreadyDragging {
                applying: matches!(d.phase, Phase::Applying),
            });
        }
        if self.ended.iter().any(|id| id == drag_id) {
            return Err(Rejection::LateStart);
        }
        Ok(())
    }

    /// 살아 있는(끝나지 않은) 세션의 id — 시작을 `AlreadyDragging{applying: false}` 로 거절한 뒤 실행기가
    /// [`Input::Superseded`] 에 싣는다. 거절한 `admits` 와 같은 장부 락 안에서 읽는다 — 락을 놓은 뒤 읽으면 그
    /// 사이 선 다른 세션을 끝낼 수 있다.
    pub fn live_drag_id(&self) -> Option<&str> {
        self.live().map(|d| d.id.as_str())
    }

    pub fn step(&mut self, input: Input) -> Step {
        match input {
            Input::Start {
                drag_id,
                view_id,
                hotspot,
                lease,
                generation,
                source_label,
                expect_source,
                single_tab_popout,
                snap,
                now,
            } => self.start(
                Drag {
                    id: drag_id,
                    generation,
                    lease,
                    seq: 0,
                    hotspot,
                    source_label,
                    source_view: view_id,
                    expect_source,
                    source_hidden: single_tab_popout,
                    source_destroyed: false,
                    display_active: None,
                    blocked: false,
                    windows: HashMap::new(),
                    gone: HashSet::new(),
                    revs: HashMap::new(),
                    sent: HashMap::new(),
                    seen: HashMap::new(),
                    arrivals: 0,
                    timer: None,
                    phase: Phase::Live,
                    started_at: now,
                    cause: None,
                    mode: None,
                    window: None,
                    reports: 0,
                    regions_sent: 0,
                    regions_resent: 0,
                },
                snap,
            ),
            Input::Refused { drag_id } => {
                self.refused(&drag_id);
                Step::default()
            }
            Input::Snapshot(snap) => self.snapshot(snap).into(),
            Input::Over {
                drag_id,
                label,
                report,
                now,
            } => self.over(&drag_id, label, report, now).into(),
            Input::Left {
                drag_id,
                label,
                now,
            } => {
                self.left(&drag_id, label, now);
                Step::default()
            }
            Input::Drop {
                drag_id,
                view_id,
                label,
                report,
                cursor,
                rects,
                now: _,
            } => {
                let signal = EndSignal::Drop {
                    view_id,
                    label,
                    report,
                    cursor,
                    rects,
                };
                self.end_signal(drag_id, signal, false).into()
            }
            Input::SourceEnd {
                drag_id,
                effect,
                cursor,
                rects,
                now,
            } => {
                let signal = EndSignal::SourceEnd {
                    effect,
                    cursor,
                    rects,
                    now,
                };
                self.end_signal(drag_id, signal, false).into()
            }
            Input::PointerSeen { drag_id, now } => self.pointer_seen(&drag_id, now).into(),
            Input::Timeout {
                kind,
                number,
                cursor,
                rects,
                now: _,
            } => self.timeout(kind, number, cursor, &rects).into(),
            Input::Superseded {
                drag_id,
                cursor,
                rects,
                now,
            } => self.superseded(&drag_id, cursor, &rects, now).into(),
            Input::Destroyed { label } => self.destroyed(label).into(),
            Input::Applied(result) => self.applied(result).into(),
            Input::Pull { label } => Step {
                actions: Vec::new(),
                reply: Reply::Pulled(self.pull(&label)),
            },
        }
    }

    fn live(&self) -> Option<&Drag> {
        self.drag.as_ref().filter(|d| is_live(d))
    }

    fn live_mut(&mut self) -> Option<&mut Drag> {
        self.drag.as_mut().filter(|d| is_live(d))
    }

    fn next_timer(&mut self) -> u64 {
        self.timer_seq += 1;
        self.timer_seq
    }

    // 세션 시한은 한 번에 하나 — 새로 걸면 걸려 있던 것은 번호가 어긋나 지난 시한이 된다.
    fn arm_session(&mut self, kind: TimerKind, after_ms: u64) -> Vec<Action> {
        let number = self.next_timer();
        let Some(drag) = self.live_mut() else {
            return Vec::new();
        };
        drag.timer = Some(number);
        vec![Action::ArmTimer {
            kind,
            number,
            after_ms,
        }]
    }

    // 이미 있는 id 는 맨 뒤로 옮긴다 — `LateStart` 거절 뒤의 `Refused` 가 같은 id 로 칸을 하나 더 쓰지 않게.
    fn remember_ended(&mut self, id: DragId) {
        self.ended.retain(|e| *e != id);
        self.ended.push_back(id);
        while self.ended.len() > ENDED_KEPT {
            self.ended.pop_front();
        }
    }

    // TRD S21-slot-docking §1-3 「시작」 3
    fn start(&mut self, mut drag: Drag, snap: Snap) -> Step {
        if let Err(rejection) = self.admits(&drag.id) {
            // `admits` 를 거친 뒤에 여기서 거절되는 것은 그 사이 그 id 가 `ended` 에 적힌 경우다 — 그 id 의 이른
            // 신호 시한이 지났거나 다른 모르는 id 의 끝 신호가 그것을 밀어냈다.
            return Step {
                actions: vec![Action::ReleaseLease(drag.lease)],
                reply: Reply::Rejected(rejection),
            };
        }
        let mut actions = Vec::new();
        if drag.source_hidden {
            actions.push(Action::Hide(drag.source_label.clone()));
        }
        drag.display_active = display_active(&snap.windows, &drag.source_label, drag.source_view);
        let started = drag.started();
        actions.push(Action::Broadcast(
            drag.event(SessionPhase::Started(started)),
        ));
        actions.extend(drag.refresh(snap.windows));
        let start_number = snap.number;
        self.last_snap = self.last_snap.max(start_number);
        let id = drag.id.clone();
        self.drag = Some(drag);
        if let Some(held) = self.held_snap.take() {
            let action = if held.number > start_number {
                "replay"
            } else {
                "drop"
            };
            tracing::debug!(
                drag = %self.drag.as_ref().map_or("", |d| d.id.as_str()),
                snap = held.number,
                last = start_number,
                action,
                "세션 전 스냅숏"
            );
            if held.number > start_number {
                actions.extend(self.snapshot(held));
            }
        }
        // 판정이 낸 `Apply` 도 할 일일 뿐이다 — 이 시작이 동기 명령이어도 실행기가 async 로 넘긴다(TRD §11-2 #23).
        if let Some(early) = self.early.take_if(|e| e.drag_id == id) {
            for signal in early.signals {
                tracing::debug!(drag = %id, event = signal.event(), action = "replay", cause = "start", "붙든 이른 끝 신호를 시작에 이어 먹인다");
                actions.extend(self.end_signal(id.clone(), signal, true));
            }
        }
        actions.into()
    }

    // TRD S21-slot-docking §1-3 「영역」 · 「적용 중」
    fn snapshot(&mut self, snap: Snap) -> Vec<Action> {
        if snap.number <= self.last_snap {
            tracing::debug!(
                drag = ?self.live().map(|d| &d.id),
                snap = snap.number,
                last = self.last_snap,
                action = "drop",
                "옛 스냅숏 번호 — 버린다"
            );
            return Vec::new();
        }
        if self.live().is_none() {
            if self
                .held_snap
                .as_ref()
                .is_none_or(|held| snap.number > held.number)
            {
                tracing::debug!(
                    snap = snap.number,
                    last = self.last_snap,
                    action = "hold",
                    "세션 전 스냅숏을 든다"
                );
                self.held_snap = Some(snap);
            } else {
                tracing::debug!(
                    snap = snap.number,
                    last = self.last_snap,
                    action = "drop",
                    "든 것보다 옛 세션 전 스냅숏 — 버린다"
                );
            }
            return Vec::new();
        }
        self.last_snap = snap.number;
        let Some(drag) = self.live_mut() else {
            return Vec::new();
        };
        if matches!(drag.phase, Phase::Applying) {
            tracing::debug!(drag = %drag.id, event = "snapshot", snap = snap.number, "적용 중 — 번호만 적는다");
            return Vec::new();
        }
        match drag.absorb(snap) {
            Some(actions) => actions,
            None => {
                drag.cause = Some(EndCause::by(EndBy::SourceGone));
                self.end(Ending::Aborted(EndReason::SourceGone))
            }
        }
    }

    // 그 id 의 살아 있는 세션 — 아니면 debug 로 버린다.
    fn session_for(&mut self, drag_id: &str, event: &'static str) -> Option<&mut Drag> {
        if self.live().is_some_and(|d| d.id == drag_id) {
            return self.live_mut();
        }
        tracing::debug!(
            drag = drag_id,
            current = ?self.live().map(|d| &d.id),
            event,
            late = self.ended.iter().any(|id| id == drag_id),
            "그 세션이 아닌 신호 — 버린다"
        );
        None
    }

    // TRD S21-slot-docking §1-3 「보인 것 보고」 · 「영역」 순서
    fn over(
        &mut self,
        drag_id: &str,
        label: WindowLabel,
        report: Report,
        now: MonoMs,
    ) -> Vec<Action> {
        let Some(drag) = self.session_for(drag_id, "over") else {
            return Vec::new();
        };
        drag.reports += 1;
        drag.arrivals += 1;
        let rev = report.rev;
        drag.seen.entry(label.clone()).or_default().over = Some(OverSeen {
            report,
            arrived: Arrived {
                order: drag.arrivals,
                at: now,
            },
        });
        let Some(current) = drag.sent.get(&label).map(|s| s.regions.clone()) else {
            return Vec::new();
        };
        if rev >= current.rev {
            return Vec::new();
        }
        // 웹뷰는 가장 큰 rev 만 쓰므로 옛 rev 에 머문 것은 그 emit 을 잃었다 — 두면 모든 놓기가 무변경이다.
        tracing::debug!(drag = %drag.id, label = %label, rev, current = current.rev, "옛 rev 의 보고 — 지금 영역을 다시 보낸다");
        drag.regions_resent += 1;
        vec![Action::Regions {
            label,
            regions: current,
        }]
    }

    fn left(&mut self, drag_id: &str, label: WindowLabel, now: MonoMs) {
        let Some(drag) = self.session_for(drag_id, "left") else {
            return;
        };
        drag.reports += 1;
        drag.arrivals += 1;
        drag.seen.entry(label).or_default().left = Some(Arrived {
            order: drag.arrivals,
            at: now,
        });
    }

    // 끝 신호를 그 세션 · 늦은 신호(최근 끝난 id) · 이른 신호(그 밖)로 가른다. `early` = 붙들었던 신호를 시작에 이어
    // 먹이는 중 — 그때는 시작이 그 id 로 세션을 세웠고, 그 세션이 끝나면 id 가 `ended` 에 들어 다시 붙들지 않는다.
    // TRD S21-slot-docking §1-3 「놓기 · 끝 신호 대조」
    fn end_signal(&mut self, drag_id: DragId, signal: EndSignal, early: bool) -> Vec<Action> {
        let event = signal.event();
        if !self.live().is_some_and(|d| d.id == drag_id) {
            if self.ended.contains(&drag_id) {
                tracing::debug!(drag = %drag_id, current = ?self.live().map(|d| &d.id), event, "끝난 세션의 늦은 신호 — 버린다");
                return Vec::new();
            }
            return self.park(drag_id, signal);
        }
        match signal {
            EndSignal::Drop {
                view_id,
                label,
                report,
                cursor,
                rects,
            } => self.drop_signal(view_id, &label, &report, cursor, &rects, early),
            EndSignal::SourceEnd {
                effect,
                cursor,
                rects,
                now,
            } => self.source_end(effect, cursor, &rects, now, early),
        }
    }

    fn drop_signal(
        &mut self,
        view_id: ViewId,
        label: &str,
        report: &Report,
        cursor: Option<PhysPoint>,
        rects: &Rects,
        early: bool,
    ) -> Vec<Action> {
        let Some(drag) = self.live() else {
            return Vec::new();
        };
        if view_id != drag.source_view {
            tracing::debug!(drag = %drag.id, view = %view_id, "끄는 탭이 아닌 놓기 — 버린다");
            return Vec::new();
        }
        let at = cursor.or_else(|| screen_point(rects, label, report.point));
        let cause = EndCause {
            end_by: EndBy::Drop,
            over_window: Some(label.to_string()),
            cursor_read: Some(cursor.is_some()),
            early,
            leave_gap_ms: None,
        };
        self.judge(label, report, at, rects.source_scale, cause)
    }

    // 출처 `dragend` — 받은 창이 있으면(`effect ≠ none`) 그 창의 놓기를 기다리고, 없으면 「마지막 점에 놓기」.
    // TRD S21-slot-docking §1-3 「끝 신호 셋」
    fn source_end(
        &mut self,
        effect: DragEffect,
        cursor: Option<PhysPoint>,
        rects: &Rects,
        now: MonoMs,
        early: bool,
    ) -> Vec<Action> {
        let Some(drag) = self.live_mut() else {
            return Vec::new();
        };
        // 유예 중의 끝 신호는 평소대로다 — 놓기 기다리기로 갈아타면 유예 시한은 번호가 어긋나 버려진다.
        if !matches!(drag.phase, Phase::Live | Phase::Grace { .. }) {
            tracing::debug!(drag = %drag.id, event = "source_end", phase = ?drag.phase, "끝 신호를 이미 받은 세션 — 늦은 신호를 버린다");
            return Vec::new();
        }
        if effect == DragEffect::None {
            return self.last_point(EndBy::SourceNone, now, cursor, rects, true, early);
        }
        tracing::debug!(drag = %drag.id, from = ?drag.phase, to = "await_drop", reason = "source_end", ?effect, early, "받은 창의 놓기를 기다린다");
        drag.phase = Phase::AwaitDrop {
            at: now,
            cursor,
            early,
        };
        self.arm_session(TimerKind::Drop, DROP_WAIT_MS)
    }

    // 끝난 뒤의 포인터 이동 — 끝 신호를 잃었을 수 있으니 유예를 둔다. 끝을 기다리는 중(놓기 기다리기 · 이미 유예)
    // 이면 버린다 — 이동이 이어져도 유예 시한을 늘리지 않는다.
    // TRD S21-slot-docking §1-3 「`dock_pointer_seen`」
    fn pointer_seen(&mut self, drag_id: &str, now: MonoMs) -> Vec<Action> {
        let Some(drag) = self.session_for(drag_id, "pointer_seen") else {
            return Vec::new();
        };
        if !matches!(drag.phase, Phase::Live) {
            tracing::debug!(drag = %drag.id, event = "pointer_seen", phase = ?drag.phase, "끝을 기다리는 중 — 포인터 이동을 버린다");
            return Vec::new();
        }
        tracing::debug!(drag = %drag.id, from = "live", to = "grace", reason = "pointer_seen", "끝난 뒤의 포인터 이동 — 끝 신호를 기다린다");
        drag.phase = Phase::Grace { at: now };
        self.arm_session(TimerKind::Grace, END_GRACE_MS)
    }

    fn timeout(
        &mut self,
        kind: TimerKind,
        number: u64,
        cursor: Option<PhysPoint>,
        rects: &Rects,
    ) -> Vec<Action> {
        if kind == TimerKind::Early {
            match self.early.take_if(|e| e.timer == number) {
                Some(e) => {
                    tracing::debug!(drag = %e.drag_id, event = "early", action = "discard", cause = "timeout", signals = e.signals.len(), "시작이 오지 않은 이른 끝 신호 — 버린다");
                    // 그 뒤 온 시작은 끝 신호 없는 세션이 되므로 `LateStart` 로 거절되게 한다.
                    self.remember_ended(e.drag_id);
                }
                None => {
                    tracing::debug!(?kind, number, armed = ?self.early.as_ref().map(|e| e.timer), "지난 시한 — 버린다");
                }
            }
            return Vec::new();
        }
        let armed = self.live().and_then(|d| d.timer);
        if armed != Some(number) {
            tracing::debug!(?kind, number, ?armed, "지난 시한 — 버린다");
            return Vec::new();
        }
        let Some(drag) = self.live_mut() else {
            return Vec::new();
        };
        let ending = match (kind, &drag.phase) {
            (
                TimerKind::Drop,
                Phase::AwaitDrop {
                    at,
                    cursor: held,
                    early,
                },
            ) => Some((EndBy::DropWait, *at, *held, true, *early)),
            // 유실 — 커서는 끝난 뒤 움직였으므로 W 사각형 검사를 하지 않는다.
            (TimerKind::Grace, Phase::Grace { at }) => {
                Some((EndBy::SignalLost, *at, cursor, false, false))
            }
            _ => None,
        };
        let Some((end_by, at, cursor, rect_check, early)) = ending else {
            tracing::debug!(drag = %drag.id, ?kind, number, phase = ?drag.phase, "단계와 맞지 않는 시한 — 버린다");
            return Vec::new();
        };
        drag.timer = None;
        self.last_point(end_by, at, cursor, rects, rect_check, early)
    }

    // 놓기 기다리기의 옛 세션은 그 끝 신호의 커서 · 도착으로, 끝 신호가 없는 옛 세션은 유실처럼 놓는다 — 새 누름이
    // 있었으니 지금 커서는 놓은 자리가 아니다.
    // TRD S21-slot-docking §1-3 「시작」 1 · §7 #12
    fn superseded(
        &mut self,
        drag_id: &str,
        cursor: Option<PhysPoint>,
        rects: &Rects,
        now: MonoMs,
    ) -> Vec<Action> {
        let Some(drag) = self.session_for(drag_id, "superseded") else {
            return Vec::new();
        };
        let (at, cursor, rect_check, early) = match drag.phase {
            Phase::Live => (now, cursor, false, false),
            Phase::Grace { at } => (at, cursor, false, false),
            Phase::AwaitDrop {
                at,
                cursor: held,
                early,
            } => (at, held, true, early),
            Phase::Applying | Phase::Ended => {
                tracing::debug!(drag = %drag.id, event = "superseded", phase = ?drag.phase, "이미 판정한 세션 — 버린다");
                return Vec::new();
            }
        };
        self.last_point(EndBy::NewStart, at, cursor, rects, rect_check, early)
    }

    // 거절된 끌기의 OS 끌기는 이어지므로 그 끝 신호는 뒤늦게 온다 — 끝난 id 로 적어 늦은 신호로 버리게 한다.
    fn refused(&mut self, drag_id: &str) {
        if let Some(e) = self.early.take_if(|e| e.drag_id == drag_id) {
            tracing::debug!(drag = %e.drag_id, event = "refused", action = "discard", cause = "refused", signals = e.signals.len(), "시작이 거절된 이른 끝 신호 — 버린다");
        }
        self.remember_ended(drag_id.to_string());
    }

    // 모르는 id 의 끝 신호를 그 시작을 기다리며 붙든다 — 한 번에 한 id. 같은 id 의 다음 신호는 첫 시한 그대로다.
    // TRD S21-slot-docking §1-3 「이른 끝 신호」
    fn park(&mut self, drag_id: DragId, signal: EndSignal) -> Vec<Action> {
        let event = signal.event();
        // 다른 세션이 살아 있는 동안 온 것이면 그 시작은 `AlreadyDragging` 으로 거절돼 곧 버려진다.
        let cause = if self.live().is_some() {
            "other_session"
        } else {
            "before_start"
        };
        if let Some(e) = self.early.as_mut().filter(|e| e.drag_id == drag_id) {
            e.signals.push(signal);
            tracing::debug!(drag = %drag_id, event, action = "park", cause, signals = e.signals.len(), "이른 끝 신호를 더 붙든다");
            return Vec::new();
        }
        if let Some(old) = self.early.take() {
            tracing::debug!(drag = %old.drag_id, event, action = "discard", cause = "evicted", by = %drag_id, "다른 id 의 이른 끝 신호가 밀어냈다 — 버린다");
            self.remember_ended(old.drag_id);
        }
        let timer = self.next_timer();
        tracing::debug!(drag = %drag_id, event, action = "park", cause, current = ?self.live().map(|d| &d.id), "모르는 id 의 끝 신호 — 시작을 기다리며 붙든다");
        self.early = Some(Early {
            drag_id,
            signals: vec![signal],
            timer,
        });
        vec![Action::ArmTimer {
            kind: TimerKind::Early,
            number: timer,
            after_ms: EARLY_SIGNAL_MS,
        }]
    }

    /// 「마지막 점에 놓기」 — 끝 신호가 놓은 자리를 싣지 않을 때(출처 끝 none · 놓기 기다리기 시한 · 유실 · 새 시작).
    /// `at` = 끝 신호(유실이면 끝 증거)의 도착 · `cursor` = 놓은 자리로 쓸 커서 · `rect_check` = 커서가 W 사각형
    /// 안인지 본다(커서가 끝 뒤에 움직였으면 보지 않는다).
    // TRD S21-slot-docking §1-3 「끝 신호 셋과 「마지막 점에 놓기」」
    fn last_point(
        &mut self,
        end_by: EndBy,
        at: MonoMs,
        cursor: Option<PhysPoint>,
        rects: &Rects,
        rect_check: bool,
        early: bool,
    ) -> Vec<Action> {
        let Some(drag) = self.live() else {
            return Vec::new();
        };
        let last = drag.last_over(at);
        let cause = EndCause {
            end_by,
            over_window: last.as_ref().map(|l| l.label.clone()),
            cursor_read: Some(cursor.is_some()),
            early,
            leave_gap_ms: last.as_ref().and_then(|l| l.leave_gap_ms),
        };
        let scale = rects.source_scale;
        let Some(LastOver {
            label: w,
            report,
            still_over,
            ..
        }) = last
        else {
            return self.new_window(cursor, scale, cause);
        };
        let Some(c) = cursor else {
            // 잠금 화면 · 보안 데스크톱(AC21) — 마지막으로 알려진 점 = W 의 마지막 Over 점.
            let known = screen_point(rects, &w, report.point);
            return if still_over {
                self.judge(&w, &report, known, scale, cause)
            } else {
                self.new_window(known, scale, cause)
            };
        };
        if !still_over {
            return self.new_window(cursor, scale, cause);
        }
        if !rect_check {
            return self.judge(&w, &report, cursor, scale, cause);
        }
        match rects.windows.get(&w) {
            Some(r) if contains(r, c) => self.judge(&w, &report, cursor, scale, cause),
            // 창 밖으로 빠르게 나가 뗀 떠남 보고가 아직 오는 중이거나 끝 신호 뒤 커서가 움직였다 — 그 자리엔
            // 사용자에게 보인 가이드가 없다(§11-2 #1).
            Some(_) => self.new_window(cursor, scale, cause),
            // 사라진 대상(소멸 · 숨김 · 최소화 · 못 읽음)의 본 가이드에는 붙이지 않는다(Q18).
            None if report.shown != Picked::None => self.unjudged(
                Verdict::Refuse(EndReason::TargetChanged),
                None,
                scale,
                cause,
            ),
            None => self.new_window(cursor, scale, cause),
        }
    }

    /// 놓기 판정 — 창 `w` 위에 보인 것 `report` 와 장부의 지금 판정이 같을 때만 무엇이든 바뀐다. `at` = 새 창 자리의
    /// 기준점(커서 · 그것을 못 읽으면 마지막으로 알려진 점의 화면 자리 · `None` = 기본 자리) · `source_scale` =
    /// 출처 배율. 이미 판정한 세션(적용 중)이면 늦은 신호로 버린다. 적용을 내면 같은 걸음에서 `Applying` 이 선다 —
    /// 한 세션에 적용은 많아야 한 번이다.
    // TRD S21-slot-docking §1-3 「놓기 판정」
    fn judge(
        &mut self,
        w: &str,
        report: &Report,
        at: Option<PhysPoint>,
        source_scale: Option<f64>,
        cause: EndCause,
    ) -> Vec<Action> {
        let Some(drag) = self.judgeable(cause) else {
            return Vec::new();
        };
        let verdict = if drag.blocked {
            Verdict::Refuse(EndReason::SourceChanged)
        } else {
            drag.verdict(w, report)
        };
        self.settle(verdict, at, source_scale)
    }

    /// 판정 없이 R22 새 창 — 「마지막 점에 놓기」에서 W 의 보인 가이드가 없는 갈래. `blocked` 면 무변경이다.
    fn new_window(
        &mut self,
        at: Option<PhysPoint>,
        source_scale: Option<f64>,
        cause: EndCause,
    ) -> Vec<Action> {
        self.unjudged(Verdict::NewWindow, at, source_scale, cause)
    }

    // 놓기 판정을 거치지 않고 정한 결말 — 그래도 `blocked` 가 먼저다(놓기 판정 1).
    fn unjudged(
        &mut self,
        verdict: Verdict,
        at: Option<PhysPoint>,
        source_scale: Option<f64>,
        cause: EndCause,
    ) -> Vec<Action> {
        let Some(drag) = self.judgeable(cause) else {
            return Vec::new();
        };
        let verdict = if drag.blocked {
            Verdict::Refuse(EndReason::SourceChanged)
        } else {
            verdict
        };
        self.settle(verdict, at, source_scale)
    }

    fn judgeable(&mut self, cause: EndCause) -> Option<&mut Drag> {
        let drag = self.live_mut()?;
        if matches!(drag.phase, Phase::Applying) {
            tracing::debug!(drag = %drag.id, event = ?cause.end_by, "이미 판정한 세션 — 늦은 신호를 버린다");
            return None;
        }
        drag.cause = Some(cause);
        Some(drag)
    }

    fn settle(
        &mut self,
        verdict: Verdict,
        at: Option<PhysPoint>,
        source_scale: Option<f64>,
    ) -> Vec<Action> {
        let Some(drag) = self.live_mut() else {
            return Vec::new();
        };
        let (to, expect_target, mode, window) = match verdict {
            Verdict::Refuse(reason) => return self.end(Ending::Noop(reason)),
            Verdict::NewWindow => (
                DockDest::NewWindow {
                    at: WindowPlace {
                        pos: at.map(|p| new_window_pos(p, drag.hotspot, source_scale)),
                        size: None,
                    },
                },
                None,
                Mode::NewWindow,
                None,
            ),
            Verdict::Dock {
                to,
                expect,
                mode,
                window,
            } => (to, expect, mode, Some(window)),
        };
        drag.phase = Phase::Applying;
        drag.timer = None;
        drag.mode = Some(mode);
        drag.window = window;
        vec![Action::Apply(DockRequest {
            view: drag.source_view,
            to,
            guard: DockGuard {
                lease: Some(drag.lease),
                expect_source: Some(drag.expect_source.clone()),
                expect_target,
            },
            focus: FocusPolicy::Reveal,
        })]
    }

    fn destroyed(&mut self, label: WindowLabel) -> Vec<Action> {
        let Some(drag) = self.live_mut() else {
            return Vec::new();
        };
        if label != drag.source_label {
            drag.gone.insert(label.clone());
            drag.sent.remove(&label);
            drag.windows.remove(&label);
            return Vec::new();
        }
        drag.source_destroyed = true;
        // 성공한 적용이 빈 출처 팝아웃을 닫은 `Destroyed` 가 `Applied` 보다 먼저 닿을 수 있다 — 여기서 끝내면 대상
        // 포커스를 잃고 `Ended` 가 적용 알림을 앞지른다.
        if matches!(drag.phase, Phase::Applying) {
            tracing::debug!(drag = %drag.id, event = "source_destroyed", "적용 중 — 적기만 한다");
            return Vec::new();
        }
        drag.cause = Some(EndCause::by(EndBy::SourceGone));
        self.end(Ending::Aborted(EndReason::SourceDestroyed))
    }

    // TRD S21-slot-docking §1-3 「적용 중」
    fn applied(&mut self, result: Result<DockOutcome, DockError>) -> Vec<Action> {
        if !self
            .live()
            .is_some_and(|d| matches!(d.phase, Phase::Applying))
        {
            tracing::debug!(drag = ?self.live().map(|d| &d.id), ok = result.is_ok(), "적용 중인 세션이 없는 적용 결과 — 버린다");
            return Vec::new();
        }
        match result {
            Ok(outcome) => self.end(Ending::Applied(outcome)),
            Err(e) => self.end(Ending::Noop(EndReason::Refused(e))),
        }
    }

    fn pull(&mut self, label: &str) -> Option<PullReply> {
        let drag = self.live_mut()?;
        let regions = match drag.windows.get(label).cloned() {
            Some(w) => {
                if drag.renew(&w) {
                    drag.regions_sent += 1;
                }
                drag.sent.get(label).map(|s| s.regions.clone())
            }
            None => None,
        };
        Some(PullReply {
            started: drag.started(),
            regions,
        })
    }

    /// 세션을 끝낸다 — 할 일은 `Ended` → 숨긴 출처 되보이기 → 대상 포커스 → 표식 내리기 → 끝 로그 순서다. 끝난
    /// id 는 `ended` 에 남는다. 성공이면 적용 서비스의 알림이 이미 나갔으므로 출처 창은 탭이 빠진 목록을 `Ended`
    /// 보다 먼저 받는다.
    // TRD S21-slot-docking §1-3 「끝」
    fn end(&mut self, ending: Ending) -> Vec<Action> {
        let Some(drag) = self.live_mut() else {
            return Vec::new();
        };
        let mut out = vec![Action::Broadcast(drag.event(SessionPhase::Ended))];
        let vanished = drag.source_destroyed || matches!(ending, Ending::Aborted(_));
        let (outcome, reason, applied) = match ending {
            Ending::Applied(o) => (Outcome::Applied, None, Some(o)),
            Ending::Noop(r) => (Outcome::Noop, Some(r), None),
            Ending::Aborted(r) => (Outcome::Aborted, Some(r), None),
        };
        // 숨긴 출처는 적용이 비워 닫았을 때만 그대로 둔다 — 재사용(옮긴 창)이나, 끄는 동안 다른 경로가 탭을 더 넣어
        // 성공한 적용 뒤에도 남은 팝아웃은 되보여야 한다(안 보이면 그 탭에 닿을 길이 없다).
        let closed = applied
            .as_ref()
            .is_some_and(|o| o.closed_window.as_deref() == Some(drag.source_label.as_str()));
        if drag.source_hidden && !closed && !vanished {
            out.push(Action::Show(drag.source_label.clone()));
        }
        if let Some(o) = &applied {
            if o.reused {
                drag.mode = Some(Mode::Reuse);
            }
            drag.window = Some(o.window.clone());
            out.push(Action::Focus(o.window.clone()));
        }
        out.push(Action::ReleaseLease(drag.lease));
        let cause = drag.cause.clone();
        out.push(Action::EndLog(EndLog {
            drag_id: drag.id.clone(),
            generation: drag.generation,
            outcome,
            reason,
            end_by: cause.as_ref().map(|c| c.end_by),
            mode: drag.mode,
            window: drag.window.clone(),
            over_window: cause.as_ref().and_then(|c| c.over_window.clone()),
            leave_gap_ms: cause.as_ref().and_then(|c| c.leave_gap_ms),
            cursor_read: cause.as_ref().and_then(|c| c.cursor_read),
            early: cause.as_ref().is_some_and(|c| c.early),
            reports: drag.reports,
            regions_sent: drag.regions_sent,
            regions_resent: drag.regions_resent,
            started_at: drag.started_at,
        }));
        drag.phase = Phase::Ended;
        drag.timer = None;
        let id = drag.id.clone();
        self.remember_ended(id);
        out
    }
}

impl EndSignal {
    fn event(&self) -> &'static str {
        match self {
            EndSignal::Drop { .. } => "drop",
            EndSignal::SourceEnd { .. } => "source_end",
        }
    }
}

impl Drag {
    fn started(&self) -> Started {
        Started {
            source_label: self.source_label.clone(),
            source_view: self.source_view,
            display_active: self.display_active,
        }
    }

    fn event(&mut self, phase: SessionPhase) -> SessionEvent {
        self.seq += 1;
        SessionEvent {
            drag_id: self.id.clone(),
            seq: self.seq,
            phase,
        }
    }

    // 스냅숏이 함께 판정하는 것(TRD §1-3 「영역」) — `None` = 출처 View 가 없다(부르는 쪽이 끝낸다).
    fn absorb(&mut self, snap: Snap) -> Option<Vec<Action>> {
        let layout = snap.source_layout?;
        let mut out = Vec::new();
        if !self.blocked && layout != self.expect_source {
            self.blocked = true;
            tracing::info!(drag = %self.id, view = %self.source_view, "출처 배치가 바뀌어 놓기를 막는다(모든 창 영역 비움)");
        }
        let display = display_active(&snap.windows, &self.source_label, self.source_view);
        if display != self.display_active {
            self.display_active = display;
            let started = self.started();
            out.push(Action::Broadcast(
                self.event(SessionPhase::Display(started)),
            ));
        }
        out.extend(self.refresh(snap.windows));
        Some(out)
    }

    // 창마다 (영역, 기대값)이 마지막 보낸 것과 다르면 번호를 올려 보낸다. 스냅숏에 없는 창은 지금 영역을 지운다 —
    // 본 것(`seen`)은 남겨야 놓기 판정이 「대상 사라짐」을 안다.
    fn refresh(&mut self, windows: Vec<WindowSnap>) -> Vec<Action> {
        let mut out = Vec::new();
        let mut material = HashMap::new();
        for mut w in windows {
            if self.gone.contains(&w.label) {
                continue;
            }
            w.is_source = w.label == self.source_label;
            w.hidden = w.is_source && self.source_hidden;
            if self.renew(&w) {
                if let Some(s) = self.sent.get(&w.label) {
                    self.regions_sent += 1;
                    out.push(Action::Regions {
                        label: w.label.clone(),
                        regions: s.regions.clone(),
                    });
                }
            }
            material.insert(w.label.clone(), w);
        }
        self.sent.retain(|label, _| material.contains_key(label));
        self.windows = material;
        out
    }

    // 창 하나의 영역을 다시 만든다 — 마지막 보낸 것과 다르면 번호를 올려 지금 영역으로 적고 `true`.
    fn renew(&mut self, w: &WindowSnap) -> bool {
        let Some((mut regions, expect)) =
            regions::regions_for(w, self.source_view, &self.expect_source, self.blocked)
        else {
            self.sent.remove(&w.label);
            return false;
        };
        let bumps = regions::bumps(
            self.sent.get(&w.label).map(|s| (&s.regions, &s.expect)),
            (&regions, &expect),
        );
        if !bumps.rev && !bumps.bar_rev {
            return false;
        }
        let revs = self.revs.entry(w.label.clone()).or_insert((0, 0));
        if bumps.rev {
            revs.0 += 1;
        }
        if bumps.bar_rev {
            revs.1 += 1;
        }
        regions.drag_id = self.id.clone();
        (regions.rev, regions.bar_rev) = *revs;
        self.sent.insert(w.label.clone(), Sent { regions, expect });
        true
    }

    // 「마지막 Over 창」 W = 마지막 Over 의 도착 순번이 가장 늦은 창 — 뒤에 Left 가 와도 W 다(숨긴 출처는 후보가
    // 아니다). 아직 위 = 그 Over 뒤에 Left 가 없거나, 그 Left 가 끝 신호 도착 `at` 전 `LEAVE_GRACE_MS` 미만 —
    // Esc 취소의 떠남은 `dragend` 보다 먼저 닿을 수 있다. `at` 뒤에 도착한 Left 는 간격 0 이라 아직 위다(끝 신호
    // 때는 오지 않았던 보고다).
    fn last_over(&self, at: MonoMs) -> Option<LastOver> {
        let (label, seen, over) = self
            .seen
            .iter()
            .filter(|(label, _)| !(self.source_hidden && **label == self.source_label))
            .filter_map(|(label, s)| s.over.as_ref().map(|o| (label, s, o)))
            .max_by_key(|(_, _, o)| o.arrived.order)?;
        let leave_gap_ms = seen
            .left
            .filter(|l| l.order > over.arrived.order)
            .map(|l| at.since(l.at));
        Some(LastOver {
            label: label.clone(),
            report: over.report.clone(),
            still_over: leave_gap_ms.is_none_or(|gap| gap < LEAVE_GRACE_MS),
            leave_gap_ms,
        })
    }

    // 놓기 판정 2–4(1 = blocked 는 부르는 쪽이 먼저 본다).
    fn verdict(&self, w: &str, report: &Report) -> Verdict {
        let current = self.sent.get(w);
        let judged = current.map_or(Picked::None, |s| {
            regions::pick(&s.regions, report.point, self.source_view)
        });
        match (&report.shown, current) {
            (Picked::None, _) if judged == Picked::None => Verdict::NewWindow,
            // 이음새 — 그 rev 의 영역 · 보이던 탭 · 다시 고른 것이 모두 같을 때만.
            (Picked::Seam { anchor, side }, Some(s))
                if report.rev == s.regions.rev
                    && report.displayed == s.regions.view_id
                    && judged == report.shown =>
            {
                match (s.regions.view_id, s.expect.seam()) {
                    (Some(target), Some(expect)) => Verdict::Dock {
                        to: DockDest::Seam {
                            target,
                            anchor: anchor.map_or(DockAnchor::Outer, DockAnchor::Slot),
                            side: *side,
                        },
                        expect: Some(expect),
                        mode: Mode::Seam,
                        window: w.to_string(),
                    },
                    _ => Verdict::Refuse(EndReason::TargetChanged),
                }
            }
            // 탭바 — 탭바 몫과 끼울 자리만 본다. 활성 전환은 rev 만 올리므로 rev · 보이던 탭까지 보면 전환 알림
            // 틈의 탭바 놓기가 무변경이 된다.
            (Picked::TabBar { index, .. }, Some(s))
                if report.bar_rev == s.regions.bar_rev
                    && matches!(judged, Picked::TabBar { index: j, .. } if j == *index) =>
            {
                Verdict::Dock {
                    to: DockDest::TabBar {
                        window: w.to_string(),
                        index: usize::try_from(*index).ok(),
                    },
                    expect: Some(s.expect.tabs()),
                    mode: Mode::TabBar,
                    window: w.to_string(),
                }
            }
            _ => Verdict::Refuse(EndReason::TargetChanged),
        }
    }
}

// 출처가 그 창의 활성이면 떼어진 자리에 보일 승계 탭, 아니면 없음(모델 활성을 그대로 보인다).
fn display_active(
    windows: &[WindowSnap],
    source_label: &str,
    source_view: ViewId,
) -> Option<ViewId> {
    let w = windows.iter().find(|w| w.label == source_label)?;
    if w.active == source_view {
        succession(&w.tabs, source_view)
    } else {
        None
    }
}

// 창 `label` 의 뷰포트 점을 화면 물리 px 로 — 그 창 사각형이 없으면 `None`.
fn screen_point(rects: &Rects, label: &str, p: PointCss) -> Option<PhysPoint> {
    let r = rects.windows.get(label)?;
    Some(PhysPoint {
        x: f64::from(r.origin.0) + p.x * r.scale,
        y: f64::from(r.origin.1) + p.y * r.scale,
    })
}

// 네 변 반열림 — 오른쪽 · 아래 변은 밖이다.
fn contains(r: &ClientRect, p: PhysPoint) -> bool {
    let (x0, y0) = (f64::from(r.origin.0), f64::from(r.origin.1));
    p.x >= x0 && p.x < x0 + f64::from(r.size.0) && p.y >= y0 && p.y < y0 + f64::from(r.size.1)
}

// 그림이 있던 자리에 창이 뜨게 — 그림은 출처 웹뷰가 출처 배율로 만든다.
fn new_window_pos(at: PhysPoint, hotspot: PointCss, source_scale: Option<f64>) -> (i32, i32) {
    let scale = source_scale.unwrap_or(1.0);
    // `as` 는 넘치면 끝값 · NaN 이면 0 으로 채운다(패닉 없음).
    (
        (at.x - hotspot.x * scale).round() as i32,
        (at.y - hotspot.y * scale).round() as i32,
    )
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::dock::regions::visible_tab;
    use crate::layout::chrome::{DockChrome, RectCss, TabHeader};
    use crate::layout::{CanvasPx, SlotContent, SplitDir};

    const SRC: u128 = 99;
    const MAIN: &str = "main";
    const POP: &str = "slot-popup-1";
    const POP2: &str = "slot-popup-2";
    const BAR_H: f64 = 28.0;
    const LEASE: LeaseId = LeaseId(7);
    const GEN: u64 = 3;
    const HOTSPOT: PointCss = PointCss { x: 10.0, y: 6.0 };
    // 칸 1(왼쪽 반)의 왼쪽 띠 · 칸 1 가운데 · 탭바(머리 10 = 0..100 · 11 = 100..200).
    const SEAM_P: PointCss = PointCss {
        x: 100.0,
        y: BAR_H + 400.0,
    };
    const NONE_P: PointCss = PointCss {
        x: 300.0,
        y: BAR_H + 400.0,
    };
    const BAR_P: PointCss = PointCss { x: 120.0, y: 10.0 };

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn ids(ns: &[u128]) -> Vec<Uuid> {
        ns.iter().map(|&n| id(n)).collect()
    }

    fn slot(n: u128) -> LayoutNode {
        LayoutNode::Slot {
            id: id(n),
            content: SlotContent::Empty,
        }
    }

    fn halves(a: u128, b: u128) -> LayoutNode {
        LayoutNode::Split {
            id: id(a * 1000 + b),
            dir: SplitDir::LeftRight,
            ratio: 0.5,
            a: Box::new(slot(a)),
            b: Box::new(slot(b)),
        }
    }

    // 탭 머리를 탭 순서대로 100px 씩.
    fn chrome(tabs: &[u128]) -> DockChrome {
        DockChrome {
            canvas_origin: PointCss { x: 0.0, y: BAR_H },
            tab_bar: RectCss {
                x0: 0.0,
                y0: 0.0,
                x1: 600.0,
                y1: BAR_H,
            },
            tabs: tabs
                .iter()
                .enumerate()
                .map(|(i, &n)| TabHeader {
                    view_id: id(n),
                    rect: RectCss {
                        x0: 100.0 * i as f64,
                        y0: 0.0,
                        x1: 100.0 * (i + 1) as f64,
                        y1: BAR_H,
                    },
                })
                .collect(),
        }
    }

    fn win(label: &str, tabs: &[u128], active: u128, layout: LayoutNode) -> WindowSnap {
        WindowSnap {
            label: label.into(),
            tabs: ids(tabs),
            active: id(active),
            visible: visible_tab(&ids(tabs), id(active), id(SRC)).map(|v| (v, layout)),
            canvas: Some(CanvasPx { w: 1200, h: 800 }),
            chrome: Some(chrome(tabs)),
            hidden: false,
            is_source: false,
        }
    }

    fn snap(number: u64, source: Option<LayoutNode>, windows: Vec<WindowSnap>) -> Snap {
        Snap {
            number,
            source_layout: source,
            windows,
        }
    }

    fn start_input(drag: &str, source: &str, single: bool, lease: LeaseId, snap: Snap) -> Input {
        Input::Start {
            drag_id: drag.into(),
            view_id: id(SRC),
            hotspot: HOTSPOT,
            lease,
            generation: GEN,
            source_label: source.into(),
            expect_source: slot(50),
            single_tab_popout: single,
            snap,
            now: MonoMs(1000),
        }
    }

    fn main_win() -> WindowSnap {
        win(MAIN, &[10, 11], 10, halves(1, 2))
    }

    fn pop2_win() -> WindowSnap {
        win(POP2, &[20], 20, halves(3, 4))
    }

    fn pop_win() -> WindowSnap {
        win(POP, &[SRC], SRC, slot(50))
    }

    // 출처 = 탭 하나뿐인 팝아웃 POP(숨김) · 대상 = main · POP2.
    fn popout_windows() -> Vec<WindowSnap> {
        vec![main_win(), pop_win(), pop2_win()]
    }

    fn popout_case() -> (Book, Step) {
        let mut book = Book::new();
        let step = book.step(start_input(
            "d-1",
            POP,
            true,
            LEASE,
            snap(1, Some(slot(50)), popout_windows()),
        ));
        (book, step)
    }

    fn fresh() -> Book {
        popout_case().0
    }

    fn sent_to<'a>(actions: &'a [Action], label: &str) -> Option<&'a DockRegions> {
        actions.iter().find_map(|a| match a {
            Action::Regions { label: l, regions } if l == label => Some(regions),
            _ => None,
        })
    }

    fn regions_count(actions: &[Action]) -> usize {
        actions
            .iter()
            .filter(|a| matches!(a, Action::Regions { .. }))
            .count()
    }

    fn end_log(actions: &[Action]) -> &EndLog {
        actions
            .iter()
            .find_map(|a| match a {
                Action::EndLog(l) => Some(l),
                _ => None,
            })
            .expect("끝 로그가 있어야")
    }

    // 할 일의 종류만 순서대로(영역은 label · rev/bar_rev 까지).
    fn kinds(actions: &[Action]) -> Vec<String> {
        actions
            .iter()
            .map(|a| match a {
                Action::Broadcast(e) => match &e.phase {
                    SessionPhase::Started(_) => format!("started#{}", e.seq),
                    SessionPhase::Display(_) => format!("display#{}", e.seq),
                    SessionPhase::Ended => format!("ended#{}", e.seq),
                },
                Action::Regions { label, regions } => {
                    format!("regions:{label}#{}/{}", regions.rev, regions.bar_rev)
                }
                Action::Hide(l) => format!("hide:{l}"),
                Action::Show(l) => format!("show:{l}"),
                Action::Focus(l) => format!("focus:{l}"),
                Action::Apply(_) => "apply".into(),
                Action::ArmTimer { .. } => "timer".into(),
                Action::ReleaseLease(l) => format!("release:{}", l.0),
                Action::EndLog(_) => "endlog".into(),
            })
            .collect()
    }

    fn applied_req(actions: &[Action]) -> &DockRequest {
        match actions {
            [Action::Apply(req)] => req,
            other => panic!("적용 하나여야 — {:?}", kinds(other)),
        }
    }

    fn report(
        rev: u64,
        bar_rev: u64,
        displayed: Option<u128>,
        shown: Picked,
        point: PointCss,
    ) -> Report {
        Report {
            rev,
            bar_rev,
            displayed: displayed.map(id),
            shown,
            point,
        }
    }

    fn rects() -> Rects {
        let mut windows = HashMap::new();
        windows.insert(
            MAIN.to_string(),
            ClientRect {
                origin: (100, 50),
                size: (2400, 1656),
                scale: 2.0,
            },
        );
        Rects {
            windows,
            source_scale: Some(1.5),
        }
    }

    fn drop_on(book: &mut Book, label: &str, r: Report, cursor: Option<PhysPoint>) -> Vec<Action> {
        drop_with(book, "d-1", id(SRC), label, r, cursor, rects())
    }

    fn drop_with(
        book: &mut Book,
        drag: &str,
        view: Uuid,
        label: &str,
        r: Report,
        cursor: Option<PhysPoint>,
        rects: Rects,
    ) -> Vec<Action> {
        book.step(Input::Drop {
            drag_id: drag.into(),
            view_id: view,
            label: label.into(),
            report: r,
            cursor,
            rects,
            now: MonoMs(2000),
        })
        .actions
    }

    fn over(book: &mut Book, label: &str, r: Report) -> Vec<Action> {
        book.step(Input::Over {
            drag_id: "d-1".into(),
            label: label.into(),
            report: r,
            now: MonoMs(1500),
        })
        .actions
    }

    fn current(book: &Book, label: &str) -> DockRegions {
        book.drag.as_ref().unwrap().sent[label].regions.clone()
    }

    fn cursor() -> Option<PhysPoint> {
        Some(PhysPoint { x: 500.0, y: 400.0 })
    }

    // 출처 = 탭 하나뿐인 팝아웃 POP — 재사용이 아닌 적용은 그 창을 비워 닫는다.
    fn ok_outcome(window: &str, reused: bool) -> Result<DockOutcome, DockError> {
        Ok(DockOutcome {
            window: window.into(),
            slot_ids: vec![],
            closed_window: (!reused).then(|| POP.to_string()),
            reused,
        })
    }

    fn is_applying(book: &Book) -> bool {
        matches!(book.drag.as_ref().unwrap().phase, Phase::Applying)
    }

    fn noop_end(actions: &[Action], reason: EndReason) {
        assert_eq!(
            kinds(actions),
            vec!["ended#2", "show:slot-popup-1", "release:7", "endlog"]
        );
        let log = end_log(actions);
        assert_eq!(log.outcome, Outcome::Noop);
        assert_eq!(log.reason, Some(reason));
    }

    // ── 시작 ─────────────────────────────────────────────────────────────────────

    #[test]
    fn starting_from_a_single_tab_popout_hides_it_and_sends_regions_to_the_others() {
        let (_, step) = popout_case();
        assert_eq!(step.reply, Reply::Nothing);
        assert_eq!(
            kinds(&step.actions),
            vec![
                "hide:slot-popup-1",
                "started#1",
                "regions:main#1/1",
                "regions:slot-popup-2#1/1"
            ]
        );
        assert_eq!(
            step.actions[1],
            Action::Broadcast(SessionEvent {
                drag_id: "d-1".into(),
                seq: 1,
                phase: SessionPhase::Started(Started {
                    source_label: POP.into(),
                    source_view: id(SRC),
                    display_active: None,
                }),
            })
        );
        let main = sent_to(&step.actions, MAIN).unwrap();
        assert_eq!(main.drag_id, "d-1");
        assert_eq!(main.view_id, Some(id(10)));
        assert!(!main.blocked && main.tab_bar.is_some() && !main.slots.is_empty());
    }

    #[test]
    fn starting_from_a_multi_tab_window_hides_nothing_and_shows_the_successor() {
        let mut book = Book::new();
        let source = win(MAIN, &[10, SRC, 11], SRC, halves(1, 2));
        let step = book.step(start_input(
            "d-1",
            MAIN,
            false,
            LEASE,
            snap(1, Some(slot(50)), vec![source, pop2_win()]),
        ));
        assert_eq!(
            kinds(&step.actions),
            vec!["started#1", "regions:main#1/1", "regions:slot-popup-2#1/1"]
        );
        let Action::Broadcast(SessionEvent {
            phase: SessionPhase::Started(s),
            ..
        }) = &step.actions[0]
        else {
            panic!("Started 여야");
        };
        assert_eq!(s.display_active, Some(id(11)));
        assert_eq!(sent_to(&step.actions, MAIN).unwrap().view_id, Some(id(11)));

        // 끄는 탭이 활성이 아니면 떼어진 자리에 대신 보일 탭도 없다.
        let mut book = Book::new();
        let source = win(MAIN, &[10, SRC, 11], 10, halves(1, 2));
        let step = book.step(start_input(
            "d-1",
            MAIN,
            false,
            LEASE,
            snap(1, Some(slot(50)), vec![source]),
        ));
        assert_eq!(book.drag.as_ref().unwrap().display_active, None);
        assert_eq!(sent_to(&step.actions, MAIN).unwrap().view_id, Some(id(10)));
    }

    #[test]
    fn dragging_the_main_windows_last_tab_has_no_display_active() {
        let mut book = Book::new();
        let step = book.step(start_input(
            "d-1",
            MAIN,
            false,
            LEASE,
            snap(1, Some(slot(50)), vec![win(MAIN, &[SRC], SRC, slot(50))]),
        ));
        assert_eq!(kinds(&step.actions), vec!["started#1", "regions:main#1/1"]);
        assert_eq!(book.drag.as_ref().unwrap().display_active, None);
        let main = sent_to(&step.actions, MAIN).unwrap();
        assert_eq!(main.view_id, None);
        assert!(main.slots.is_empty() && main.outer.is_empty());
    }

    #[test]
    fn a_start_while_a_session_runs_is_refused_and_returns_its_lease() {
        let mut book = fresh();
        let step = book.step(start_input(
            "d-2",
            MAIN,
            false,
            LeaseId(8),
            snap(2, Some(slot(50)), popout_windows()),
        ));
        assert_eq!(
            step.reply,
            Reply::Rejected(Rejection::AlreadyDragging { applying: false })
        );
        assert_eq!(step.actions, vec![Action::ReleaseLease(LeaseId(8))]);
        assert_eq!(book.drag.as_ref().unwrap().id, "d-1");
        assert_eq!(
            book.admits("d-2"),
            Err(Rejection::AlreadyDragging { applying: false })
        );
    }

    // ── 영역 · 스냅숏 ────────────────────────────────────────────────────────────

    #[test]
    fn only_windows_whose_regions_or_expectation_changed_get_a_new_rev() {
        let mut book = fresh();
        let unchanged = book.step(Input::Snapshot(snap(2, Some(slot(50)), popout_windows())));
        assert!(unchanged.actions.is_empty());

        // main 의 활성만 바뀜 — rev 만 오른다(POP2 는 그대로라 안 보낸다).
        let switched = win(MAIN, &[10, 11], 11, halves(5, 6));
        let step = book.step(Input::Snapshot(snap(
            3,
            Some(slot(50)),
            vec![switched, pop_win(), pop2_win()],
        )));
        assert_eq!(kinds(&step.actions), vec!["regions:main#2/1"]);

        // 탭 순서가 바뀜 — 둘 다.
        let reordered = win(MAIN, &[11, 10], 11, halves(5, 6));
        let step = book.step(Input::Snapshot(snap(
            4,
            Some(slot(50)),
            vec![reordered, pop_win(), pop2_win()],
        )));
        assert_eq!(kinds(&step.actions), vec!["regions:main#3/2"]);

        // 탭바 기하만 바뀜 — 둘 다.
        let mut moved = win(MAIN, &[11, 10], 11, halves(5, 6));
        moved.chrome.as_mut().unwrap().tabs[0].rect.x1 = 120.0;
        let step = book.step(Input::Snapshot(snap(
            5,
            Some(slot(50)),
            vec![moved, pop_win(), pop2_win()],
        )));
        assert_eq!(kinds(&step.actions), vec!["regions:main#4/3"]);
    }

    #[test]
    fn a_content_only_change_with_the_same_geometry_bumps_rev() {
        let mut book = fresh();
        let content = LayoutNode::Split {
            id: id(1002),
            dir: SplitDir::LeftRight,
            ratio: 0.5,
            a: Box::new(LayoutNode::Slot {
                id: id(1),
                content: SlotContent::AgentList,
            }),
            b: Box::new(slot(2)),
        };
        let before = current(&book, MAIN);
        let step = book.step(Input::Snapshot(snap(
            2,
            Some(slot(50)),
            vec![win(MAIN, &[10, 11], 10, content), pop_win(), pop2_win()],
        )));
        assert_eq!(kinds(&step.actions), vec!["regions:main#2/1"]);
        let after = sent_to(&step.actions, MAIN).unwrap();
        assert_eq!(
            (&after.slots, &after.outer, &after.tab_bar),
            (&before.slots, &before.outer, &before.tab_bar),
            "전제: 기하는 같다"
        );
    }

    #[test]
    fn a_snapshot_number_at_or_below_the_last_is_dropped() {
        let mut book = fresh();
        let changed = vec![win(MAIN, &[10, 11], 11, slot(5)), pop_win(), pop2_win()];
        assert!(book
            .step(Input::Snapshot(snap(1, Some(slot(50)), changed.clone())))
            .actions
            .is_empty());
        assert!(book
            .step(Input::Snapshot(snap(0, Some(slot(50)), changed.clone())))
            .actions
            .is_empty());
        assert_eq!(
            kinds(
                &book
                    .step(Input::Snapshot(snap(2, Some(slot(50)), changed)))
                    .actions
            ),
            vec!["regions:main#2/1"]
        );
    }

    #[test]
    fn a_pre_session_snapshot_is_replayed_after_start_only_when_newer() {
        let changed = || vec![win(MAIN, &[10, 11], 11, slot(5)), pop_win(), pop2_win()];
        let mut book = Book::new();
        assert!(book
            .step(Input::Snapshot(snap(5, Some(slot(50)), changed())))
            .actions
            .is_empty());
        // 더 옛것은 든 것을 밀어내지 않는다.
        book.step(Input::Snapshot(snap(4, Some(slot(50)), popout_windows())));
        let step = book.step(start_input(
            "d-1",
            POP,
            true,
            LEASE,
            snap(3, Some(slot(50)), popout_windows()),
        ));
        assert_eq!(
            kinds(&step.actions),
            vec![
                "hide:slot-popup-1",
                "started#1",
                "regions:main#1/1",
                "regions:slot-popup-2#1/1",
                "regions:main#2/1"
            ]
        );
        assert_eq!(book.last_snap, 5);
        assert!(book.held_snap.is_none());

        let mut book = Book::new();
        book.step(Input::Snapshot(snap(2, Some(slot(50)), changed())));
        let step = book.step(start_input(
            "d-1",
            POP,
            true,
            LEASE,
            snap(3, Some(slot(50)), popout_windows()),
        ));
        assert_eq!(regions_count(&step.actions), 2, "옛 것은 버린다");
        assert!(book.held_snap.is_none());
    }

    #[test]
    fn a_changed_source_layout_blocks_and_empties_every_window_for_good() {
        let mut book = fresh();
        let step = book.step(Input::Snapshot(snap(2, Some(slot(51)), popout_windows())));
        assert_eq!(
            kinds(&step.actions),
            vec!["regions:main#2/2", "regions:slot-popup-2#2/2"]
        );
        for label in [MAIN, POP2] {
            let r = sent_to(&step.actions, label).unwrap();
            assert!(r.blocked && r.view_id.is_none() && r.tab_bar.is_none() && r.slots.is_empty());
        }
        // 시작 때 배치로 돌아와도 막힌 채다.
        let step = book.step(Input::Snapshot(snap(3, Some(slot(50)), popout_windows())));
        assert!(step.actions.is_empty());
        assert!(book.drag.as_ref().unwrap().blocked);
    }

    #[test]
    fn display_is_broadcast_only_when_display_active_changes() {
        let mut book = Book::new();
        let source = |tabs: &[u128], active| win(MAIN, tabs, active, halves(1, 2));
        book.step(start_input(
            "d-1",
            MAIN,
            false,
            LEASE,
            snap(1, Some(slot(50)), vec![source(&[10, SRC, 11], SRC)]),
        ));
        // 오른쪽 이웃 11 이 닫힘 — 승계 탭이 10 으로.
        let step = book.step(Input::Snapshot(snap(
            2,
            Some(slot(50)),
            vec![source(&[10, SRC], SRC)],
        )));
        assert_eq!(kinds(&step.actions), vec!["display#2", "regions:main#2/2"]);
        assert_eq!(
            step.actions[0],
            Action::Broadcast(SessionEvent {
                drag_id: "d-1".into(),
                seq: 2,
                phase: SessionPhase::Display(Started {
                    source_label: MAIN.into(),
                    source_view: id(SRC),
                    display_active: Some(id(10)),
                }),
            })
        );
        let same = book.step(Input::Snapshot(snap(
            3,
            Some(slot(50)),
            vec![source(&[10, SRC], SRC)],
        )));
        assert!(same.actions.is_empty());
        // 출처 창 활성이 다른 탭으로 — 대신 보일 탭이 없어진다.
        let step = book.step(Input::Snapshot(snap(
            4,
            Some(slot(50)),
            vec![source(&[10, SRC], 10)],
        )));
        assert_eq!(kinds(&step.actions), vec!["display#3", "regions:main#3/2"]);
    }

    #[test]
    fn a_snapshot_without_the_source_view_ends_as_source_gone_without_reshowing() {
        let mut book = fresh();
        let step = book.step(Input::Snapshot(snap(2, None, popout_windows())));
        assert_eq!(kinds(&step.actions), vec!["ended#2", "release:7", "endlog"]);
        let log = end_log(&step.actions);
        assert_eq!(log.outcome, Outcome::Aborted);
        assert_eq!(log.reason, Some(EndReason::SourceGone));
        assert_eq!(log.end_by, Some(EndBy::SourceGone));
        assert_eq!((log.generation, log.drag_id.as_str()), (GEN, "d-1"));
        assert!(book.ended.contains(&"d-1".to_string()));
    }

    #[test]
    fn the_source_window_destroyed_ends_as_source_destroyed_without_reshowing() {
        let mut book = fresh();
        let step = book.step(Input::Destroyed { label: POP.into() });
        assert_eq!(kinds(&step.actions), vec!["ended#2", "release:7", "endlog"]);
        assert_eq!(
            end_log(&step.actions).reason,
            Some(EndReason::SourceDestroyed)
        );
    }

    #[test]
    fn a_window_missing_from_a_snapshot_loses_its_regions_but_keeps_what_it_saw() {
        let mut book = fresh();
        let shown = regions::pick(&current(&book, POP2), SEAM_P, id(SRC));
        over(
            &mut book,
            POP2,
            report(1, 1, Some(20), shown.clone(), SEAM_P),
        );
        let step = book.step(Input::Snapshot(snap(
            2,
            Some(slot(50)),
            vec![main_win(), pop_win()],
        )));
        assert!(step.actions.is_empty());
        let drag = book.drag.as_ref().unwrap();
        assert!(!drag.sent.contains_key(POP2));
        assert!(drag.seen[POP2].over.is_some());
        // 보인 가이드의 창이 사라졌다 — 무변경.
        let out = drop_on(
            &mut book,
            POP2,
            report(1, 1, Some(20), shown, SEAM_P),
            cursor(),
        );
        noop_end(&out, EndReason::TargetChanged);
    }

    #[test]
    fn a_destroyed_window_loses_its_regions_and_a_late_snapshot_does_not_revive_it() {
        let mut book = fresh();
        assert!(book
            .step(Input::Destroyed { label: POP2.into() })
            .actions
            .is_empty());
        assert!(!book.drag.as_ref().unwrap().sent.contains_key(POP2));
        let step = book.step(Input::Snapshot(snap(2, Some(slot(50)), popout_windows())));
        assert!(step.actions.is_empty());
        assert!(!book.drag.as_ref().unwrap().sent.contains_key(POP2));
    }

    #[test]
    fn a_window_that_returns_to_the_snapshot_keeps_counting_up() {
        let mut book = fresh();
        book.step(Input::Snapshot(snap(
            2,
            Some(slot(50)),
            vec![main_win(), pop_win()],
        )));
        let step = book.step(Input::Snapshot(snap(3, Some(slot(50)), popout_windows())));
        assert_eq!(kinds(&step.actions), vec!["regions:slot-popup-2#2/2"]);
    }

    #[test]
    fn an_over_on_an_old_rev_resends_the_current_regions_with_the_same_rev() {
        let mut book = fresh();
        book.step(Input::Snapshot(snap(
            2,
            Some(slot(50)),
            vec![win(MAIN, &[10, 11], 11, slot(5)), pop_win(), pop2_win()],
        )));
        let step = over(
            &mut book,
            MAIN,
            report(1, 1, Some(10), Picked::None, NONE_P),
        );
        assert_eq!(kinds(&step), vec!["regions:main#2/1"]);
        assert_eq!(sent_to(&step, MAIN), Some(&current(&book, MAIN)));
        assert!(over(
            &mut book,
            MAIN,
            report(2, 1, Some(11), Picked::None, NONE_P)
        )
        .is_empty());
        let drag = book.drag.as_ref().unwrap();
        assert_eq!((drag.reports, drag.regions_resent), (2, 1));
    }

    #[test]
    fn left_does_not_erase_the_last_over() {
        let mut book = fresh();
        over(
            &mut book,
            MAIN,
            report(1, 1, Some(10), Picked::None, NONE_P),
        );
        book.step(Input::Left {
            drag_id: "d-1".into(),
            label: MAIN.into(),
            now: MonoMs(1600),
        });
        let seen = &book.drag.as_ref().unwrap().seen[MAIN];
        let (o, l) = (seen.over.as_ref().unwrap(), seen.left.unwrap());
        assert_eq!((o.arrived.order, o.arrived.at), (1, MonoMs(1500)));
        assert_eq!((l.order, l.at), (2, MonoMs(1600)));
        assert_eq!(o.report.point, NONE_P);
    }

    #[test]
    fn signals_of_another_drag_id_do_not_touch_the_session() {
        let mut book = fresh();
        let step = book.step(Input::Over {
            drag_id: "other".into(),
            label: MAIN.into(),
            report: report(0, 0, None, Picked::None, NONE_P),
            now: MonoMs(1),
        });
        assert!(step.actions.is_empty());
        assert!(book.drag.as_ref().unwrap().seen.is_empty());
        // 끝 신호는 그 id 의 시작을 기다리며 붙든다(「이른 끝 신호」) — 세션은 그대로다.
        let out = drop_with(
            &mut book,
            "other",
            id(SRC),
            MAIN,
            report(0, 0, None, Picked::None, NONE_P),
            cursor(),
            rects(),
        );
        assert_eq!(kinds(&out), vec!["timer"]);
        assert!(matches!(book.drag.as_ref().unwrap().phase, Phase::Live));
    }

    #[test]
    fn pull_rebuilds_the_window_regions_from_the_last_snapshot() {
        let mut book = Book::new();
        assert_eq!(
            book.step(Input::Pull { label: MAIN.into() }).reply,
            Reply::Pulled(None)
        );
        let mut book = fresh();
        let main = current(&book, MAIN);
        let step = book.step(Input::Pull { label: MAIN.into() });
        assert!(step.actions.is_empty());
        let Reply::Pulled(Some(pulled)) = step.reply else {
            panic!("세션이 있다");
        };
        assert_eq!(pulled.regions, Some(main));
        assert_eq!(pulled.started.source_label, POP);
        for label in [POP, "slot-popup-9"] {
            let Reply::Pulled(Some(p)) = book
                .step(Input::Pull {
                    label: label.into(),
                })
                .reply
            else {
                panic!("세션이 있다");
            };
            assert_eq!(p.regions, None, "{label}");
        }
    }

    // ── 놓기 판정 ────────────────────────────────────────────────────────────────

    #[test]
    fn a_blocked_drop_changes_nothing() {
        let mut book = fresh();
        book.step(Input::Snapshot(snap(2, Some(slot(51)), popout_windows())));
        let out = drop_on(
            &mut book,
            MAIN,
            report(2, 2, None, Picked::None, NONE_P),
            cursor(),
        );
        noop_end(&out, EndReason::SourceChanged);
    }

    #[test]
    fn no_guide_shown_and_none_judged_opens_a_new_window_whatever_the_rev() {
        let mut book = fresh();
        let out = drop_on(
            &mut book,
            MAIN,
            report(0, 0, None, Picked::None, NONE_P),
            cursor(),
        );
        assert_eq!(
            applied_req(&out),
            &DockRequest {
                view: id(SRC),
                to: DockDest::NewWindow {
                    // 커서 (500, 400) − 그림 커서 자리 (10, 6) × 출처 배율 1.5.
                    at: WindowPlace {
                        pos: Some((485, 391)),
                        size: None,
                    },
                },
                guard: DockGuard {
                    lease: Some(LEASE),
                    expect_source: Some(slot(50)),
                    expect_target: None,
                },
                focus: FocusPolicy::Reveal,
            }
        );
        assert!(is_applying(&book));
    }

    #[test]
    fn a_new_window_without_a_cursor_uses_the_drop_point_or_the_default_place() {
        let mut book = fresh();
        let out = drop_on(
            &mut book,
            MAIN,
            report(0, 0, None, Picked::None, NONE_P),
            None,
        );
        // main 안쪽 원점 (100, 50) + 점 (300, 428) × 2 = (700, 906) − (15, 9).
        let DockDest::NewWindow { at } = &applied_req(&out).to else {
            panic!("새 창이어야");
        };
        assert_eq!(at.pos, Some((685, 897)));

        let mut book = fresh();
        let out = drop_with(
            &mut book,
            "d-1",
            id(SRC),
            MAIN,
            report(0, 0, None, Picked::None, NONE_P),
            None,
            Rects::default(),
        );
        let DockDest::NewWindow { at } = &applied_req(&out).to else {
            panic!("새 창이어야");
        };
        assert_eq!(at.pos, None);
    }

    #[test]
    fn no_guide_shown_where_a_guide_is_judged_now_changes_nothing() {
        let mut book = fresh();
        let out = drop_on(
            &mut book,
            MAIN,
            report(1, 1, Some(10), Picked::None, SEAM_P),
            cursor(),
        );
        noop_end(&out, EndReason::TargetChanged);
    }

    #[test]
    fn a_seam_drop_applies_only_when_rev_displayed_and_repick_all_match() {
        let shown = regions::pick(&current(&fresh(), MAIN), SEAM_P, id(SRC));
        let Picked::Seam {
            anchor: Some(anchor),
            side,
        } = shown
        else {
            panic!("전제: 칸 띠여야 — {shown:?}");
        };
        for (r, why) in [
            (report(0, 1, Some(10), shown.clone(), SEAM_P), "rev 다름"),
            (
                report(1, 1, Some(11), shown.clone(), SEAM_P),
                "보이던 탭 다름",
            ),
            (report(1, 1, None, shown.clone(), SEAM_P), "보이던 탭 없음"),
            (
                report(1, 1, Some(10), shown.clone(), NONE_P),
                "다시 고른 것 다름",
            ),
        ] {
            let mut book = fresh();
            let out = drop_on(&mut book, MAIN, r, cursor());
            assert_eq!(
                end_log(&out).reason,
                Some(EndReason::TargetChanged),
                "{why}"
            );
        }

        let mut book = fresh();
        let out = drop_on(
            &mut book,
            MAIN,
            report(1, 1, Some(10), shown, SEAM_P),
            cursor(),
        );
        assert_eq!(
            applied_req(&out),
            &DockRequest {
                view: id(SRC),
                to: DockDest::Seam {
                    target: id(10),
                    anchor: DockAnchor::Slot(anchor),
                    side,
                },
                guard: DockGuard {
                    lease: Some(LEASE),
                    expect_source: Some(slot(50)),
                    expect_target: Some(TargetExpect::Seam {
                        layout: halves(1, 2),
                        active: id(10),
                    }),
                },
                focus: FocusPolicy::Reveal,
            }
        );
        assert!(is_applying(&book));
        assert_eq!(book.drag.as_ref().unwrap().mode, Some(Mode::Seam));
    }

    #[test]
    fn a_tab_bar_drop_matches_bar_rev_and_index_only() {
        let judged = regions::pick(&current(&fresh(), MAIN), BAR_P, id(SRC));
        assert_eq!(
            judged,
            Picked::TabBar {
                index: 1,
                over: Some(id(11)),
            },
            "전제"
        );
        // 활성 전환 직후 — rev · 보이던 탭이 달라도 · 머리(`over`)가 달라도 들어간다.
        let mut book = fresh();
        book.step(Input::Snapshot(snap(
            2,
            Some(slot(50)),
            vec![win(MAIN, &[10, 11], 11, slot(5)), pop_win(), pop2_win()],
        )));
        assert_eq!(current(&book, MAIN).bar_rev, 1);
        let shown = Picked::TabBar {
            index: 1,
            over: Some(id(10)),
        };
        let out = drop_on(
            &mut book,
            MAIN,
            report(1, 1, Some(10), shown.clone(), BAR_P),
            cursor(),
        );
        assert_eq!(
            applied_req(&out),
            &DockRequest {
                view: id(SRC),
                to: DockDest::TabBar {
                    window: MAIN.into(),
                    index: Some(1),
                },
                guard: DockGuard {
                    lease: Some(LEASE),
                    expect_source: Some(slot(50)),
                    expect_target: Some(TargetExpect::Tabs(ids(&[10, 11]))),
                },
                focus: FocusPolicy::Reveal,
            }
        );

        let mut book = fresh();
        let out = drop_on(
            &mut book,
            MAIN,
            report(1, 0, Some(10), shown, BAR_P),
            cursor(),
        );
        assert_eq!(
            end_log(&out).reason,
            Some(EndReason::TargetChanged),
            "bar_rev 다름"
        );

        let mut book = fresh();
        let other_slot = Picked::TabBar {
            index: 2,
            over: None,
        };
        let out = drop_on(
            &mut book,
            MAIN,
            report(1, 1, Some(10), other_slot, BAR_P),
            cursor(),
        );
        assert_eq!(
            end_log(&out).reason,
            Some(EndReason::TargetChanged),
            "자리 다름"
        );
    }

    #[test]
    fn a_drop_of_another_tab_is_dropped() {
        let mut book = fresh();
        let out = drop_with(
            &mut book,
            "d-1",
            id(10),
            MAIN,
            report(0, 0, None, Picked::None, NONE_P),
            cursor(),
            rects(),
        );
        assert!(out.is_empty());
        assert!(matches!(book.drag.as_ref().unwrap().phase, Phase::Live));
    }

    #[test]
    fn a_drop_after_the_session_ended_is_late() {
        let mut book = fresh();
        book.step(Input::Destroyed { label: POP.into() });
        let out = drop_on(
            &mut book,
            MAIN,
            report(0, 0, None, Picked::None, NONE_P),
            cursor(),
        );
        assert!(out.is_empty());
    }

    // ── 적용 중 · 끝 ─────────────────────────────────────────────────────────────

    fn seam_applying() -> Book {
        let mut book = fresh();
        let shown = regions::pick(&current(&book, MAIN), SEAM_P, id(SRC));
        let out = drop_on(
            &mut book,
            MAIN,
            report(1, 1, Some(10), shown, SEAM_P),
            cursor(),
        );
        assert_eq!(kinds(&out), vec!["apply"]);
        book
    }

    #[test]
    fn while_applying_a_snapshot_without_the_source_only_records_its_number() {
        let mut book = seam_applying();
        let step = book.step(Input::Snapshot(snap(
            2,
            None,
            vec![win(MAIN, &[10, 11], 10, slot(7)), pop2_win()],
        )));
        assert!(step.actions.is_empty());
        assert!(is_applying(&book));
        assert_eq!(book.last_snap, 2);

        let out = book.step(Input::Applied(ok_outcome(MAIN, false))).actions;
        assert_eq!(
            kinds(&out),
            vec!["ended#2", "focus:main", "release:7", "endlog"]
        );
        let log = end_log(&out);
        assert_eq!(
            (log.outcome, log.mode, log.window.as_deref()),
            (Outcome::Applied, Some(Mode::Seam), Some(MAIN))
        );
        assert_eq!(log.reason, None);
        assert_eq!(log.end_by, Some(EndBy::Drop));
        assert_eq!(log.over_window.as_deref(), Some(MAIN));
        assert_eq!(log.cursor_read, Some(true));
    }

    #[test]
    fn the_source_popout_destroyed_while_applying_does_not_end_the_session() {
        let mut book = seam_applying();
        assert!(book
            .step(Input::Destroyed { label: POP.into() })
            .actions
            .is_empty());
        assert!(is_applying(&book));
        let out = book.step(Input::Applied(ok_outcome(MAIN, false))).actions;
        assert_eq!(
            kinds(&out),
            vec!["ended#2", "focus:main", "release:7", "endlog"]
        );
        assert_eq!(end_log(&out).outcome, Outcome::Applied);
    }

    #[test]
    fn a_source_destroyed_while_applying_is_not_reshown_after_a_refused_apply() {
        let mut book = seam_applying();
        book.step(Input::Destroyed { label: POP.into() });
        let out = book
            .step(Input::Applied(Err(DockError::StaleLease)))
            .actions;
        assert_eq!(kinds(&out), vec!["ended#2", "release:7", "endlog"]);
    }

    #[test]
    fn a_reused_source_destroyed_while_applying_is_not_reshown() {
        let mut book = fresh();
        drop_on(
            &mut book,
            MAIN,
            report(0, 0, None, Picked::None, NONE_P),
            cursor(),
        );
        book.step(Input::Destroyed { label: POP.into() });
        let out = book.step(Input::Applied(ok_outcome(POP, true))).actions;
        assert_eq!(
            kinds(&out),
            vec!["ended#2", "focus:slot-popup-1", "release:7", "endlog"]
        );
    }

    #[test]
    fn a_successful_dock_reshows_the_hidden_source_unless_it_closed_it() {
        // 끄는 동안 다른 경로가 숨긴 출처 팝아웃에 탭을 넣었다(출처 배치는 그대로라 막히지 않는다) — 적용 뒤에도
        // 그 창은 남는다.
        let mut book = fresh();
        book.step(Input::Snapshot(snap(
            2,
            Some(slot(50)),
            vec![main_win(), win(POP, &[SRC, 30], SRC, slot(50)), pop2_win()],
        )));
        assert!(!book.drag.as_ref().unwrap().blocked);
        let r = seam_report(&book, MAIN, 10);
        assert_eq!(seam_target(&drop_on(&mut book, MAIN, r, cursor())), id(10));
        let kept = Ok(DockOutcome {
            window: MAIN.into(),
            slot_ids: vec![],
            closed_window: None,
            reused: false,
        });
        let out = book.step(Input::Applied(kept)).actions;
        assert_eq!(
            kinds(&out),
            vec![
                "ended#3",
                "show:slot-popup-1",
                "focus:main",
                "release:7",
                "endlog"
            ]
        );

        // 탭 하나뿐이던 출처는 적용이 비워 닫았다.
        let mut book = seam_applying();
        let out = book.step(Input::Applied(ok_outcome(MAIN, false))).actions;
        assert_eq!(
            kinds(&out),
            vec!["ended#2", "focus:main", "release:7", "endlog"]
        );
    }

    #[test]
    fn a_refused_apply_ends_as_noop_with_that_reason() {
        let mut book = seam_applying();
        let out = book
            .step(Input::Applied(Err(DockError::StaleLease)))
            .actions;
        noop_end(&out, EndReason::Refused(DockError::StaleLease));
        assert_eq!(end_log(&out).reason.as_ref().unwrap().word(), "StaleLease");
    }

    #[test]
    fn a_session_applies_at_most_once() {
        let mut book = seam_applying();
        let again = drop_on(
            &mut book,
            MAIN,
            report(0, 0, None, Picked::None, NONE_P),
            cursor(),
        );
        assert!(again.is_empty());
        assert!(is_applying(&book));
        // 적용 중이 아닌 세션의 적용 결과는 버린다.
        let mut book = fresh();
        assert!(book
            .step(Input::Applied(ok_outcome(MAIN, false)))
            .actions
            .is_empty());
        assert!(matches!(book.drag.as_ref().unwrap().phase, Phase::Live));
    }

    #[test]
    fn a_new_start_while_applying_is_refused_and_the_old_session_ends_by_its_result() {
        let mut book = seam_applying();
        let step = book.step(start_input(
            "d-2",
            MAIN,
            false,
            LeaseId(8),
            snap(5, Some(slot(50)), popout_windows()),
        ));
        assert_eq!(
            step.reply,
            Reply::Rejected(Rejection::AlreadyDragging { applying: true })
        );
        assert_eq!(
            Rejection::AlreadyDragging { applying: true }.word(),
            "applying"
        );
        assert_eq!(step.actions, vec![Action::ReleaseLease(LeaseId(8))]);
        assert!(is_applying(&book));
        let out = book.step(Input::Applied(ok_outcome(MAIN, false))).actions;
        assert_eq!(end_log(&out).drag_id, "d-1");
        assert_eq!(book.admits("d-2"), Ok(()));
    }

    #[test]
    fn a_reused_popout_is_shown_then_focused_after_ended() {
        let mut book = fresh();
        drop_on(
            &mut book,
            MAIN,
            report(0, 0, None, Picked::None, NONE_P),
            cursor(),
        );
        let out = book.step(Input::Applied(ok_outcome(POP, true))).actions;
        assert_eq!(
            kinds(&out),
            vec![
                "ended#2",
                "show:slot-popup-1",
                "focus:slot-popup-1",
                "release:7",
                "endlog"
            ]
        );
        let log = end_log(&out);
        assert_eq!(
            (log.mode, log.window.as_deref()),
            (Some(Mode::Reuse), Some(POP))
        );
    }

    #[test]
    fn session_seq_rises_and_ended_ids_are_kept_for_late_starts() {
        let mut book = Book::new();
        book.step(start_input(
            "d-1",
            MAIN,
            false,
            LEASE,
            snap(
                1,
                Some(slot(50)),
                vec![win(MAIN, &[10, SRC, 11], SRC, halves(1, 2))],
            ),
        ));
        let display = book.step(Input::Snapshot(snap(
            2,
            Some(slot(50)),
            vec![win(MAIN, &[10, SRC], SRC, halves(1, 2))],
        )));
        let ended = book.step(Input::Snapshot(snap(3, None, vec![])));
        let seqs: Vec<u64> = display
            .actions
            .iter()
            .chain(&ended.actions)
            .filter_map(|a| match a {
                Action::Broadcast(e) => Some(e.seq),
                _ => None,
            })
            .collect();
        assert_eq!(seqs, vec![2, 3]);

        // 끝난 id 의 늦은 시작 — 표식만 돌려주고 아무것도 하지 않는다.
        let late = book.step(start_input(
            "d-1",
            MAIN,
            false,
            LeaseId(8),
            snap(4, Some(slot(50)), popout_windows()),
        ));
        assert_eq!(late.reply, Reply::Rejected(Rejection::LateStart));
        assert_eq!(late.actions, vec![Action::ReleaseLease(LeaseId(8))]);

        // 넷만 남는다.
        for n in 2..=5 {
            let drag = format!("d-{n}");
            book.step(start_input(
                &drag,
                MAIN,
                false,
                LEASE,
                snap(10 * n, Some(slot(50)), vec![main_win()]),
            ));
            book.step(Input::Snapshot(snap(10 * n + 1, None, vec![])));
        }
        assert_eq!(book.ended, ["d-2", "d-3", "d-4", "d-5"].map(String::from));
        assert_eq!(book.admits("d-1"), Ok(()));
        assert_eq!(book.admits("d-5"), Err(Rejection::LateStart));
    }

    #[test]
    fn a_stale_timeout_is_dropped() {
        let mut book = fresh();
        let step = book.step(Input::Timeout {
            kind: TimerKind::Drop,
            number: 42,
            cursor: None,
            rects: Rects::default(),
            now: MonoMs(9),
        });
        assert!(step.actions.is_empty());
        assert!(matches!(book.drag.as_ref().unwrap().phase, Phase::Live));
    }

    #[test]
    fn end_reason_words_follow_the_log_vocabulary() {
        assert_eq!(EndReason::SourceChanged.word(), "SourceChanged");
        assert_eq!(
            EndReason::Refused(DockError::TargetChanged).word(),
            "TargetChanged"
        );
        assert_eq!(
            EndReason::Refused(DockError::WindowOpen("x".into())).word(),
            "WindowOpen"
        );
        assert_eq!(Outcome::Aborted.word(), "aborted");
        assert_eq!(EndBy::SourceNone.word(), "source_none");
        assert_eq!(Mode::TabBar.word(), "tab_bar");
    }

    // ── 끝 신호 ──────────────────────────────────────────────────────────────────

    fn src_end(
        book: &mut Book,
        effect: DragEffect,
        cursor: Option<PhysPoint>,
        rects: Rects,
        now: u64,
    ) -> Vec<Action> {
        src_end_of(book, "d-1", effect, cursor, rects, now)
    }

    fn src_end_of(
        book: &mut Book,
        drag: &str,
        effect: DragEffect,
        cursor: Option<PhysPoint>,
        rects: Rects,
        now: u64,
    ) -> Vec<Action> {
        book.step(Input::SourceEnd {
            drag_id: drag.into(),
            effect,
            cursor,
            rects,
            now: MonoMs(now),
        })
        .actions
    }

    fn over_at(book: &mut Book, label: &str, r: Report, now: u64) {
        book.step(Input::Over {
            drag_id: "d-1".into(),
            label: label.into(),
            report: r,
            now: MonoMs(now),
        });
    }

    fn left_at(book: &mut Book, label: &str, now: u64) {
        book.step(Input::Left {
            drag_id: "d-1".into(),
            label: label.into(),
            now: MonoMs(now),
        });
    }

    fn pointer_seen(book: &mut Book, drag: &str, now: u64) -> Vec<Action> {
        book.step(Input::PointerSeen {
            drag_id: drag.into(),
            now: MonoMs(now),
        })
        .actions
    }

    fn fire(
        book: &mut Book,
        kind: TimerKind,
        number: u64,
        cursor: Option<PhysPoint>,
        rects: Rects,
    ) -> Vec<Action> {
        book.step(Input::Timeout {
            kind,
            number,
            cursor,
            rects,
            now: MonoMs(9000),
        })
        .actions
    }

    fn superseded(book: &mut Book, drag: &str, cursor: Option<PhysPoint>) -> Vec<Action> {
        book.step(Input::Superseded {
            drag_id: drag.into(),
            cursor,
            rects: rects(),
            now: MonoMs(3000),
        })
        .actions
    }

    // 걸린 시한 하나 — (종류, 번호, 시간).
    fn armed(actions: &[Action]) -> (TimerKind, u64, u64) {
        match actions {
            [Action::ArmTimer {
                kind,
                number,
                after_ms,
            }] => (*kind, *number, *after_ms),
            other => panic!("시한 하나여야 — {:?}", kinds(other)),
        }
    }

    fn new_window_at(actions: &[Action]) -> Option<(i32, i32)> {
        match &applied_req(actions).to {
            DockDest::NewWindow { at } => at.pos,
            other => panic!("새 창이어야 — {other:?}"),
        }
    }

    fn seam_target(actions: &[Action]) -> Uuid {
        match &applied_req(actions).to {
            DockDest::Seam { target, .. } => *target,
            other => panic!("이음새여야 — {other:?}"),
        }
    }

    fn seam_report(book: &Book, label: &str, displayed: u128) -> Report {
        let shown = regions::pick(&current(book, label), SEAM_P, id(SRC));
        assert!(matches!(shown, Picked::Seam { .. }), "전제: 칸 띠여야");
        report(1, 1, Some(displayed), shown, SEAM_P)
    }

    // main 사각형 (100, 50) 2400×1656 밖.
    fn outside() -> Option<PhysPoint> {
        Some(PhysPoint {
            x: 3000.0,
            y: 400.0,
        })
    }

    fn without_main() -> Rects {
        Rects {
            windows: HashMap::new(),
            source_scale: Some(1.5),
        }
    }

    fn applied_log(book: &mut Book, window: &str, reused: bool) -> EndLog {
        end_log(
            &book
                .step(Input::Applied(ok_outcome(window, reused)))
                .actions,
        )
        .clone()
    }

    fn phase_is_await_drop(book: &Book) -> bool {
        matches!(book.drag.as_ref().unwrap().phase, Phase::AwaitDrop { .. })
    }

    const START_KINDS: [&str; 4] = [
        "hide:slot-popup-1",
        "started#1",
        "regions:main#1/1",
        "regions:slot-popup-2#1/1",
    ];

    fn start_d(book: &mut Book, drag: &str) -> Step {
        book.step(start_input(
            drag,
            POP,
            true,
            LEASE,
            snap(1, Some(slot(50)), popout_windows()),
        ))
    }

    #[test]
    fn a_source_end_move_after_the_drop_is_late() {
        let mut book = fresh();
        let out = drop_on(
            &mut book,
            MAIN,
            report(0, 0, None, Picked::None, NONE_P),
            cursor(),
        );
        assert_eq!(kinds(&out), vec!["apply"]);
        assert!(src_end(&mut book, DragEffect::Move, cursor(), rects(), 2100).is_empty());
        assert!(is_applying(&book));
        book.step(Input::Applied(ok_outcome(POP, true)));
        assert!(src_end(&mut book, DragEffect::Move, cursor(), rects(), 2200).is_empty());
        assert!(book.early.is_none(), "끝난 id 는 붙들지 않는다");
    }

    #[test]
    fn a_source_end_with_a_receiver_waits_for_its_drop() {
        for effect in [DragEffect::Move, DragEffect::Other] {
            let mut book = fresh();
            let (kind, number, after) = armed(&src_end(&mut book, effect, cursor(), rects(), 2000));
            assert_eq!((kind, after), (TimerKind::Drop, DROP_WAIT_MS));
            assert!(phase_is_await_drop(&book));
            let r = seam_report(&book, MAIN, 10);
            let out = drop_on(&mut book, MAIN, r, cursor());
            assert_eq!(seam_target(&out), id(10));
            // 판정한 뒤 그 시한은 지난 것이다.
            assert!(fire(&mut book, TimerKind::Drop, number, cursor(), rects()).is_empty());
            assert!(is_applying(&book));
            assert_eq!(
                applied_log(&mut book, MAIN, false).end_by,
                Some(EndBy::Drop)
            );
        }
    }

    #[test]
    fn when_the_drop_wait_runs_out_it_drops_at_the_last_point_with_the_end_signal_cursor() {
        let mut book = fresh();
        let number = armed(&src_end(
            &mut book,
            DragEffect::Move,
            cursor(),
            rects(),
            2000,
        ))
        .1;
        // 시한 때의 커서는 쓰지 않는다 — 커서 (500, 400) − (10, 6) × 1.5.
        let out = fire(&mut book, TimerKind::Drop, number, outside(), rects());
        assert_eq!(new_window_at(&out), Some((485, 391)));
        let log = applied_log(&mut book, POP, true);
        assert_eq!(
            (log.end_by, log.cursor_read, log.over_window),
            (Some(EndBy::DropWait), Some(true), None)
        );

        // W 위에서 끝났으면 그 보고로 판정한다.
        let mut book = fresh();
        let r = seam_report(&book, MAIN, 10);
        over_at(&mut book, MAIN, r, 1500);
        let number = armed(&src_end(
            &mut book,
            DragEffect::Move,
            cursor(),
            rects(),
            2000,
        ))
        .1;
        let out = fire(&mut book, TimerKind::Drop, number, outside(), rects());
        assert_eq!(seam_target(&out), id(10));
    }

    fn ended_by_source_none_after_left(left: Option<u64>) -> (Book, Vec<Action>) {
        let mut book = fresh();
        let r = seam_report(&book, POP2, 20);
        over_at(&mut book, POP2, r, 1400);
        let r = seam_report(&book, MAIN, 10);
        over_at(&mut book, MAIN, r, 1500);
        if let Some(t) = left {
            left_at(&mut book, MAIN, t);
        }
        let out = src_end(&mut book, DragEffect::None, cursor(), rects(), 2000);
        (book, out)
    }

    #[test]
    fn source_end_none_judges_the_last_over_window_while_still_over_with_the_cursor_inside() {
        // 떠남 없음 · 유예(100ms 미만) 안의 떠남 · 끝 신호 뒤에 도착한 떠남(간격 0) — W 는 뒤에 Left 가 와도 W 다.
        for (left, gap) in [
            (None, None),
            (Some(1950), Some(50)),
            (Some(1901), Some(99)),
            (Some(2100), Some(0)),
        ] {
            let (mut book, out) = ended_by_source_none_after_left(left);
            assert_eq!(seam_target(&out), id(10), "{left:?}");
            let log = applied_log(&mut book, MAIN, false);
            assert_eq!(
                (
                    log.end_by,
                    log.over_window.as_deref(),
                    log.leave_gap_ms,
                    log.cursor_read
                ),
                (Some(EndBy::SourceNone), Some(MAIN), gap, Some(true)),
                "{left:?}"
            );
        }
    }

    #[test]
    fn a_left_at_or_beyond_the_leave_grace_opens_a_new_window_at_the_cursor() {
        for left in [1900, 1500] {
            let (_, out) = ended_by_source_none_after_left(Some(left));
            assert_eq!(new_window_at(&out), Some((485, 391)), "{left}");
        }
        // Over 앞의 Left 는 떠남이 아니다.
        let mut book = fresh();
        left_at(&mut book, MAIN, 1400);
        let r = seam_report(&book, MAIN, 10);
        over_at(&mut book, MAIN, r, 1500);
        let out = src_end(&mut book, DragEffect::None, cursor(), rects(), 2000);
        assert_eq!(seam_target(&out), id(10));
    }

    #[test]
    fn still_over_with_the_cursor_outside_w_opens_a_new_window() {
        let mut book = fresh();
        let r = seam_report(&book, MAIN, 10);
        over_at(&mut book, MAIN, r, 1500);
        let out = src_end(&mut book, DragEffect::None, outside(), rects(), 2000);
        assert_eq!(new_window_at(&out), Some((2985, 391)));
    }

    #[test]
    fn w_without_a_rect_changes_nothing_if_it_showed_a_guide_and_opens_a_new_window_if_not() {
        let mut book = fresh();
        let r = seam_report(&book, MAIN, 10);
        over_at(&mut book, MAIN, r, 1500);
        let out = src_end(&mut book, DragEffect::None, cursor(), without_main(), 2000);
        noop_end(&out, EndReason::TargetChanged);

        let mut book = fresh();
        over_at(
            &mut book,
            MAIN,
            report(1, 1, Some(10), Picked::None, NONE_P),
            1500,
        );
        let out = src_end(&mut book, DragEffect::None, cursor(), without_main(), 2000);
        assert_eq!(new_window_at(&out), Some((485, 391)));
    }

    #[test]
    fn no_reports_open_a_new_window_at_the_cursor() {
        let mut book = fresh();
        let out = src_end(&mut book, DragEffect::None, cursor(), rects(), 2000);
        assert_eq!(new_window_at(&out), Some((485, 391)));
        let log = applied_log(&mut book, POP, true);
        assert_eq!((log.over_window, log.leave_gap_ms), (None, None));
    }

    #[test]
    fn an_unreadable_cursor_falls_back_to_the_last_point_of_w() {
        // 아직 위 = 사각형 검사 없이 그 보고로 판정.
        let mut book = fresh();
        let r = seam_report(&book, MAIN, 10);
        over_at(&mut book, MAIN, r, 1500);
        let out = src_end(&mut book, DragEffect::None, None, without_main(), 2000);
        assert_eq!(seam_target(&out), id(10));
        assert_eq!(applied_log(&mut book, MAIN, false).cursor_read, Some(false));

        // 떠남 = 그 점의 화면 자리에 새 창 — main 원점 (100, 50) + (300, 428) × 2 − (15, 9).
        let left = |rects: Rects| {
            let mut book = fresh();
            over_at(
                &mut book,
                MAIN,
                report(1, 1, Some(10), Picked::None, NONE_P),
                1500,
            );
            left_at(&mut book, MAIN, 1600);
            src_end(&mut book, DragEffect::None, None, rects, 2000)
        };
        assert_eq!(new_window_at(&left(rects())), Some((685, 897)));
        // W 사각형도 못 읽으면 기본 자리.
        assert_eq!(new_window_at(&left(without_main())), None);

        // W 없음 = 기본 자리.
        let mut book = fresh();
        let out = src_end(&mut book, DragEffect::None, None, rects(), 2000);
        assert_eq!(new_window_at(&out), None);
    }

    #[test]
    fn the_hidden_source_is_never_the_last_over_window() {
        let mut book = fresh();
        let r = seam_report(&book, MAIN, 10);
        over_at(&mut book, MAIN, r, 1500);
        over_at(
            &mut book,
            POP,
            report(0, 0, None, Picked::None, NONE_P),
            1600,
        );
        let out = src_end(&mut book, DragEffect::None, cursor(), rects(), 2000);
        assert_eq!(seam_target(&out), id(10));
    }

    #[test]
    fn a_blocked_session_changes_nothing_at_any_end() {
        let blocked = |over: bool| {
            let mut book = fresh();
            if over {
                let r = seam_report(&book, MAIN, 10);
                over_at(&mut book, MAIN, r, 1500);
            }
            book.step(Input::Snapshot(snap(2, Some(slot(51)), popout_windows())));
            book
        };
        // W 위 · 커서 안.
        let out = src_end(
            &mut blocked(true),
            DragEffect::None,
            cursor(),
            rects(),
            2000,
        );
        noop_end(&out, EndReason::SourceChanged);
        // 보고 없는 자리 — 새 창도 만들지 않는다.
        let out = src_end(
            &mut blocked(false),
            DragEffect::None,
            outside(),
            rects(),
            2000,
        );
        noop_end(&out, EndReason::SourceChanged);
        // 놓기 기다리기 시한.
        let mut book = blocked(false);
        let n = armed(&src_end(
            &mut book,
            DragEffect::Move,
            cursor(),
            rects(),
            2000,
        ))
        .1;
        noop_end(
            &fire(&mut book, TimerKind::Drop, n, cursor(), rects()),
            EndReason::SourceChanged,
        );
        // 유실.
        let mut book = blocked(true);
        let n = armed(&pointer_seen(&mut book, "d-1", 2000)).1;
        noop_end(
            &fire(&mut book, TimerKind::Grace, n, cursor(), rects()),
            EndReason::SourceChanged,
        );
    }

    // ── 시한 ─────────────────────────────────────────────────────────────────────

    #[test]
    fn a_source_end_move_during_grace_switches_to_awaiting_the_drop() {
        let mut book = fresh();
        let grace = armed(&pointer_seen(&mut book, "d-1", 1800));
        assert_eq!((grace.0, grace.2), (TimerKind::Grace, END_GRACE_MS));
        let wait = armed(&src_end(
            &mut book,
            DragEffect::Move,
            cursor(),
            rects(),
            1900,
        ));
        assert_eq!(wait.0, TimerKind::Drop);
        assert!(
            fire(&mut book, TimerKind::Grace, grace.1, cursor(), rects()).is_empty(),
            "유예 시한은 지난 것"
        );
        // 번호가 맞아도 종류가 단계와 다르면 버리고, 걸린 시한은 그대로다.
        assert!(fire(&mut book, TimerKind::Grace, wait.1, cursor(), rects()).is_empty());
        assert!(phase_is_await_drop(&book));
        assert!(
            pointer_seen(&mut book, "d-1", 1950).is_empty(),
            "놓기를 기다리는 중의 포인터 이동은 버린다"
        );
        assert!(
            drop_with(
                &mut book,
                "d-1",
                id(10),
                MAIN,
                report(0, 0, None, Picked::None, NONE_P),
                cursor(),
                rects()
            )
            .is_empty(),
            "끄는 탭이 아닌 놓기는 버린다"
        );
        assert!(
            src_end(&mut book, DragEffect::None, outside(), rects(), 1960).is_empty(),
            "두 번째 출처 끝은 늦은 신호"
        );
        assert!(phase_is_await_drop(&book));
        let out = fire(&mut book, TimerKind::Drop, wait.1, outside(), rects());
        assert_eq!(new_window_at(&out), Some((485, 391)));
    }

    #[test]
    fn pointer_seen_of_another_id_or_during_grace_arms_nothing() {
        let mut book = Book::new();
        assert!(pointer_seen(&mut book, "d-1", 1800).is_empty());
        let mut book = fresh();
        assert!(pointer_seen(&mut book, "other", 1800).is_empty());
        assert!(matches!(book.drag.as_ref().unwrap().phase, Phase::Live));
        armed(&pointer_seen(&mut book, "d-1", 1800));
        assert!(
            pointer_seen(&mut book, "d-1", 1900).is_empty(),
            "유예를 늘리지 않는다"
        );
    }

    // ── 유실 ─────────────────────────────────────────────────────────────────────

    #[test]
    fn an_end_signal_within_the_grace_is_handled_as_usual() {
        let mut book = fresh();
        armed(&pointer_seen(&mut book, "d-1", 1800));
        let r = seam_report(&book, MAIN, 10);
        assert_eq!(seam_target(&drop_on(&mut book, MAIN, r, cursor())), id(10));

        let mut book = fresh();
        let grace = armed(&pointer_seen(&mut book, "d-1", 1800)).1;
        let out = src_end(&mut book, DragEffect::None, cursor(), rects(), 1900);
        assert_eq!(new_window_at(&out), Some((485, 391)));
        assert!(fire(&mut book, TimerKind::Grace, grace, cursor(), rects()).is_empty());
        assert_eq!(
            applied_log(&mut book, POP, true).end_by,
            Some(EndBy::SourceNone)
        );
    }

    #[test]
    fn when_the_grace_runs_out_w_still_over_is_judged_with_its_report_without_a_rect_check() {
        let mut book = fresh();
        let r = seam_report(&book, MAIN, 10);
        over_at(&mut book, MAIN, r, 1500);
        // 끝 증거(1800) 전 50ms 의 떠남은 유예 안이다.
        left_at(&mut book, MAIN, 1750);
        let grace = armed(&pointer_seen(&mut book, "d-1", 1800)).1;
        let out = fire(
            &mut book,
            TimerKind::Grace,
            grace,
            outside(),
            without_main(),
        );
        assert_eq!(seam_target(&out), id(10));
        assert_eq!(
            applied_log(&mut book, MAIN, false).end_by,
            Some(EndBy::SignalLost)
        );

        // W 가 사라졌으면 판정이 「지금 영역 없음」으로 무변경.
        let mut book = fresh();
        let r = seam_report(&book, POP2, 20);
        over_at(&mut book, POP2, r, 1500);
        book.step(Input::Destroyed { label: POP2.into() });
        let grace = armed(&pointer_seen(&mut book, "d-1", 1800)).1;
        noop_end(
            &fire(&mut book, TimerKind::Grace, grace, cursor(), rects()),
            EndReason::TargetChanged,
        );
    }

    #[test]
    fn when_the_grace_runs_out_after_w_was_left_a_new_window_opens_at_the_current_cursor() {
        let mut book = fresh();
        let r = seam_report(&book, MAIN, 10);
        over_at(&mut book, MAIN, r, 1500);
        left_at(&mut book, MAIN, 1600);
        let grace = armed(&pointer_seen(&mut book, "d-1", 1800)).1;
        let out = fire(&mut book, TimerKind::Grace, grace, outside(), rects());
        assert_eq!(new_window_at(&out), Some((2985, 391)));
    }

    // ── 이른 끝 신호 ─────────────────────────────────────────────────────────────

    fn early_drop(book: &mut Book, drag: &str, point: PointCss) -> Vec<Action> {
        drop_with(
            book,
            drag,
            id(SRC),
            MAIN,
            report(0, 0, None, Picked::None, point),
            cursor(),
            rects(),
        )
    }

    #[test]
    fn an_unknown_ids_drop_is_held_and_judged_after_its_start() {
        let mut book = Book::new();
        let (kind, _, after) = armed(&early_drop(&mut book, "d-1", NONE_P));
        assert_eq!((kind, after), (TimerKind::Early, EARLY_SIGNAL_MS));
        let step = start_d(&mut book, "d-1");
        assert_eq!(step.reply, Reply::Nothing);
        assert_eq!(kinds(&step.actions[..4]), START_KINDS);
        assert_eq!(new_window_at(&step.actions[4..]), Some((485, 391)));
        assert!(book.early.is_none());
        assert!(applied_log(&mut book, POP, true).early);
    }

    #[test]
    fn a_held_drop_with_no_guide_where_the_start_judges_a_guide_changes_nothing() {
        let mut book = Book::new();
        armed(&early_drop(&mut book, "d-1", SEAM_P));
        let step = start_d(&mut book, "d-1");
        assert_eq!(kinds(&step.actions[..4]), START_KINDS);
        noop_end(&step.actions[4..], EndReason::TargetChanged);
        assert!(end_log(&step.actions).early);
    }

    #[test]
    fn held_signals_are_fed_in_arrival_order() {
        // 출처 끝(move) → 놓기 — 기다리다 그 놓기로 판정한다. 같은 id 의 다음 신호는 시한을 새로 걸지 않는다.
        let mut book = Book::new();
        armed(&src_end_of(
            &mut book,
            "d-1",
            DragEffect::Move,
            cursor(),
            rects(),
            900,
        ));
        assert!(early_drop(&mut book, "d-1", NONE_P).is_empty());
        let step = start_d(&mut book, "d-1");
        assert_eq!(kinds(&step.actions[4..]), vec!["timer", "apply"]);
        assert_eq!(new_window_at(&step.actions[5..]), Some((485, 391)));

        // 놓기 → 출처 끝(move) — 놓기로 판정하고 출처 끝은 늦은 신호다.
        let mut book = Book::new();
        armed(&early_drop(&mut book, "d-1", NONE_P));
        src_end_of(&mut book, "d-1", DragEffect::Move, cursor(), rects(), 900);
        let step = start_d(&mut book, "d-1");
        assert_eq!(kinds(&step.actions[4..]), vec!["apply"]);
    }

    #[test]
    fn a_held_signal_is_dropped_when_its_timer_runs_out_and_its_late_start_is_refused() {
        let mut book = Book::new();
        let n = armed(&early_drop(&mut book, "d-9", NONE_P)).1;
        assert!(fire(&mut book, TimerKind::Early, n, None, Rects::default()).is_empty());
        assert!(book.early.is_none());
        assert_eq!(book.admits("d-9"), Err(Rejection::LateStart));
        let step = book.step(start_input(
            "d-9",
            POP,
            true,
            LeaseId(8),
            snap(1, Some(slot(50)), popout_windows()),
        ));
        assert_eq!(step.reply, Reply::Rejected(Rejection::LateStart));
        assert_eq!(step.actions, vec![Action::ReleaseLease(LeaseId(8))]);
        assert!(book.drag.is_none());
        // 그 거절의 `Refused` 는 같은 id 로 칸을 하나 더 쓰지 않는다.
        book.step(Input::Refused {
            drag_id: "d-9".into(),
        });
        assert_eq!(book.ended, ["d-9"].map(String::from));
    }

    #[test]
    fn a_refused_start_drops_its_held_signals_and_marks_the_id_ended() {
        let mut book = fresh();
        // 세션이 살아 있는 동안 온 모르는 id 의 끝 신호도 붙든다 — 그 시작은 거절된다.
        let n = armed(&early_drop(&mut book, "d-2", NONE_P)).1;
        let step = book.step(start_input(
            "d-2",
            MAIN,
            false,
            LeaseId(8),
            snap(2, Some(slot(50)), popout_windows()),
        ));
        assert_eq!(
            step.reply,
            Reply::Rejected(Rejection::AlreadyDragging { applying: false })
        );
        // 다른 id 의 거절은 붙든 것을 건드리지 않는다.
        book.step(Input::Refused {
            drag_id: "d-3".into(),
        });
        assert_eq!(book.early.as_ref().unwrap().drag_id, "d-2");
        book.step(Input::Refused {
            drag_id: "d-2".into(),
        });
        assert!(book.early.is_none());
        assert!(fire(&mut book, TimerKind::Early, n, None, Rects::default()).is_empty());
        assert_eq!(book.ended, ["d-3", "d-2"].map(String::from));
        assert!(matches!(book.drag.as_ref().unwrap().phase, Phase::Live));
        // 거절된 끌기의 뒤늦은 끝 신호는 늦은 신호다.
        assert!(early_drop(&mut book, "d-2", NONE_P).is_empty());
        assert!(book.early.is_none());
    }

    #[test]
    fn a_refused_drags_late_end_signal_does_not_evict_another_held_id() {
        let mut book = Book::new();
        armed(&early_drop(&mut book, "d-a", NONE_P));
        book.step(Input::Refused {
            drag_id: "d-a".into(),
        });
        armed(&early_drop(&mut book, "d-b", NONE_P));
        assert!(src_end_of(&mut book, "d-a", DragEffect::None, cursor(), rects(), 900).is_empty());
        assert_eq!(book.early.as_ref().unwrap().drag_id, "d-b");
        assert_eq!(book.admits("d-b"), Ok(()));
    }

    #[test]
    fn another_unknown_id_evicts_the_held_one_into_ended() {
        let mut book = Book::new();
        let first = armed(&early_drop(&mut book, "d-8", NONE_P)).1;
        let second = armed(&src_end_of(
            &mut book,
            "d-9",
            DragEffect::None,
            cursor(),
            rects(),
            900,
        ))
        .1;
        assert_ne!(first, second);
        assert_eq!(book.early.as_ref().unwrap().drag_id, "d-9");
        assert_eq!(book.admits("d-8"), Err(Rejection::LateStart));
        assert!(fire(&mut book, TimerKind::Early, first, None, Rects::default()).is_empty());
        assert_eq!(
            book.early.as_ref().unwrap().drag_id,
            "d-9",
            "밀려난 id 의 시한은 지난 것"
        );
        assert_eq!(book.admits("d-9"), Ok(()));
    }

    #[test]
    fn an_ended_ids_signals_are_not_held() {
        let mut book = fresh();
        book.step(Input::Destroyed { label: POP.into() });
        assert!(early_drop(&mut book, "d-1", NONE_P).is_empty());
        assert!(src_end(&mut book, DragEffect::None, cursor(), rects(), 2000).is_empty());
        assert!(book.early.is_none());
    }

    // ── 새 시작 ──────────────────────────────────────────────────────────────────

    #[test]
    fn a_new_start_meeting_a_live_session_is_refused_and_the_old_one_drops_at_its_last_point() {
        let mut book = fresh();
        let r = seam_report(&book, MAIN, 10);
        over_at(&mut book, MAIN, r, 1500);
        let step = book.step(start_input(
            "d-2",
            MAIN,
            false,
            LeaseId(8),
            snap(2, Some(slot(50)), popout_windows()),
        ));
        assert_eq!(
            step.reply,
            Reply::Rejected(Rejection::AlreadyDragging { applying: false })
        );
        assert_eq!(book.live_drag_id(), Some("d-1"));
        assert!(superseded(&mut book, "d-7", cursor()).is_empty());
        // 새 누름이 있었으니 지금 커서는 놓은 자리가 아니다 — W 를 떠나지 않았으면 그 보고로 판정한다.
        let out = superseded(&mut book, "d-1", outside());
        assert_eq!(seam_target(&out), id(10));
        assert_eq!(
            applied_log(&mut book, MAIN, false).end_by,
            Some(EndBy::NewStart)
        );
        assert!(superseded(&mut book, "d-1", cursor()).is_empty());
        assert_eq!(book.live_drag_id(), None);
        assert_eq!(book.admits("d-2"), Ok(()));
    }

    #[test]
    fn a_superseded_session_awaiting_its_drop_drops_at_its_end_signal_cursor() {
        let mut book = fresh();
        armed(&src_end(
            &mut book,
            DragEffect::Move,
            cursor(),
            rects(),
            2000,
        ));
        let out = superseded(&mut book, "d-1", outside());
        assert_eq!(new_window_at(&out), Some((485, 391)));
        assert!(
            superseded(&mut book, "d-1", outside()).is_empty(),
            "적용 중이면 버린다"
        );
        assert!(is_applying(&book));
    }
}
