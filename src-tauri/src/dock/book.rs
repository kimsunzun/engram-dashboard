//! 끌기 장부(TRD S21-slot-docking §1-3) — 끌기 세션 하나의 순수 상태 기계. 입력([`Input`]) 하나를 받아 할 일
//! ([`Action`])과 답([`Reply`])을 낸다. Tauri · 락 · 시계 읽기 · OS 호출 0 — 시각 · 커서 · 창 사각형 · 모델
//! 스냅숏은 전부 입력이라 같은 입력 열이면 같은 할 일 열이다.
//!
//! 진입점: [`Book::step`] · [`Book::admits`](시작을 받을지 — 실행기가 레이아웃 락을 잡기 **전에** 묻는다) ·
//! [`Book::visibility`](출처 창을 지금 숨겨 둘지 — 숨기기 · 보이기 맞추기가 읽는다).
//!
//! 부르는 쪽(실행기)의 몫:
//! - 장부를 잎 락(`Mutex<Book>`)으로 쥐고, 쥔 채 레이아웃 락 · emit · 창 호출 · 적용 서비스 · 커서 읽기를 하지
//!   않는다 — 레이아웃 락을 쥔 채 장부 락을 잡는 것은 되고, 거꾸로는 안 된다(시작도 `admits` 뒤 장부 락을 놓고
//!   레이아웃 락을 잡는다). 할 일은 장부 락 · 레이아웃 락을 모두 놓은 뒤 낸 순서대로 실행한다(`ReleaseLease` 는
//!   레이아웃 락을 잡고 `Focus` 는 창 호출이다) — 단 `Apply` 는 먹인 스레드와 무관하게 늘 async 로 넘긴다(동기
//!   명령 안에서 새 창을 만들면 교착 — TRD §1-3 「명령의 실행 스레드」).
//! - `Hide` · `Show` 는 그대로 부르지 않고 메인 스레드 줄에 「맞추기」로 올린다 — 돌 때 [`Book::visibility`] 를 읽어
//!   그 값을 입힌다(TRD §1-3 「숨기기 · 보이기」). 여러 스레드가 낸 창 호출이 이벤트 루프에 닿는 순서는 장부가 낸
//!   순서가 아니라서, 보이기 → 숨기기로 닿으면 끝난 세션의 팝아웃이 숨은 채 남는다. 같은 끝의 `Focus` 는 같은 줄에
//!   그 `Show` 뒤로 올린다. 할 일에 실린 세대는 지난 세션의 할 일을 가르는 데 쓴다.
//! - 끝 신호 · 시한의 `now` 는 세계(커서 · 창 사각형)를 읽기 **전에** 찍는다 — 도착 시각이다. 읽은 뒤에 찍으면 그
//!   사이 먹힌 떠남 시한 뒤의 끝으로 보여 「시한이 앞지른 끝」을 못 가른다.
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
    DockRegions, DragEffect, DrawnReport, DropReport, Picked, SessionEvent, SessionPhase, Started,
    StripStart,
};
use crate::layout::apply::{DockDest, DockOutcome, DockRequest, WindowPlace};
use crate::layout::chrome::PointCss;
use crate::layout::dock::DockAnchor;
use crate::layout::manager::{
    succession, DockError, DockGuard, FocusPolicy, LeaseId, TargetExpect, ViewId, WindowLabel,
};
use crate::layout::LayoutNode;

/// 출처 끝(none) 앞의 대상 떠남이 이보다 가까우면 Esc 취소가 낸 떠남으로 보고 그 창을 「아직 위」로 친다 — S1 이
/// 간격을 다시 잰다. 같은 경계를 셋이 더 쓴다(두 판단이 같은 자리에서 갈리게): 탭 하나 팝아웃의 미룬 숨기기가 출처
/// 떠남을 증거로 삼기 전 기다리는 시간 · 「시한이 앞지른 끝」(숨김을 부른 떠남이 끝 도착 전 이 안이면 숨기지 않은
/// 것으로 친다) · 출처 웹뷰가 끝 스냅숏을 실을지.
pub const LEAVE_GRACE_MS: u64 = 100;
/// 출처 끝 스냅숏의 떠남 → `dragend` 간격(출처 엔진 시계 · ms)이 이 안이면(또는 떠남 없음) 그 떠남은 엔진이 끌기를
/// 접으며 현재 대상에 낸 것이라 끝이 출처 위에서 났다고 본다 — 사각형 검사 없이 그 스냅숏 자리로 간다. 첫 값(엔진
/// 접기 → `dragend` 2–4 ms 실측 · Esc 의 떠남 → 끝 간격 자체는 미검) — S1 ① · S7 ⑬ 이 잰다.
pub const CANCEL_LEAVE_MS: f64 = 15.0;
/// 출처 `none` 끝이 실제로 떠났다고 말하는데 도착 순번 W 가 출처이거나 없을 때, 다른 창의 오는 중 `Over` 를 기다리며
/// 그 끝을 붙드는 시간 — invoke 왕복 실측 최대 ~50 ms(TRD §8-1). 넘겨 닿은 `Over` 는 붙들기 없는 판정과 같다. S1 ①
/// 이 끝 줄 `over_wait` 로 다시 잰다.
pub const OVER_WAIT_MS: u64 = 50;
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

/// 웹뷰가 보인 것 하나 — 대상 창의 `Over` 보고 · 놓기 · 출처 끝 스냅숏이 같이 싣는 칸(뜻 =
/// [`super::types::DragOverReport`] · [`DrawnReport`]). `scroll` = 그 판정 때 그 창 탭바 `scrollLeft` — `None` =
/// 영역마다 잰 때의 것(순서 이동 = 시작 측정 · 탭바 = 크롬 보고).
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    pub rev: u64,
    pub bar_rev: u64,
    pub displayed: Option<ViewId>,
    pub shown: Picked,
    pub point: PointCss,
    pub scroll: Option<f64>,
}

impl From<&DropReport> for Report {
    fn from(d: &DropReport) -> Self {
        Self {
            rev: d.rev,
            bar_rev: d.bar_rev,
            displayed: d.displayed,
            shown: d.shown.clone(),
            point: d.point,
            // 9b-2: 놓기 wire 에 `scroll` 칸이 서면 그 값을 싣는다(TRD §1-5) — 그때까지 영역마다 잰 때의 것.
            scroll: None,
        }
    }
}

impl From<&DrawnReport> for Report {
    fn from(d: &DrawnReport) -> Self {
        Self {
            rev: d.rev,
            bar_rev: d.bar_rev,
            displayed: d.displayed,
            shown: d.shown.clone(),
            point: d.point,
            scroll: d.scroll,
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
    /// (`generation`) · 출처 배치 복제(`expect_source`) · 시작 스냅숏(`snap`)을 뜨고, 그 락을 놓은 뒤 출처 창
    /// 사각형(`source_rect`)을 읽어 먹인다.
    Start {
        drag_id: DragId,
        view_id: ViewId,
        /// 끌기 그림 안의 커서 자리 — 새 창 자리 = 커서 − 이것 × 출처 배율.
        hotspot: PointCss,
        lease: LeaseId,
        generation: u64,
        source_label: WindowLabel,
        expect_source: LayoutNode,
        /// 출처가 탭 하나뿐인 팝아웃 — 끄는 동안 숨긴다. 출처 영역에 순서 이동이 서면 도킹 모드 증거까지 미룬다
        /// (그 탭바 안에 머무는 동안은 창이 보여야 한다 — PRD D18).
        single_tab_popout: bool,
        /// 출처 웹뷰가 `dragstart` 안에서 잰 탭바 — 세션 내내 그대로 쥐고 출처 창 영역의 순서 이동을 이것으로만
        /// 판정한다(얼린 기하 · TRD §2-4). `None` = 순서 이동 없음.
        strip: Option<StripStart>,
        /// 출처 창 클라이언트 사각형 — `None` = 못 읽음. 출처가 든 `Rects` 가 올 때마다 갈아 끼우고, 「시한이 앞지른
        /// 끝」의 사각형 검사에만 쓴다(숨긴 뒤의 `Rects` 에는 출처가 없다).
        source_rect: Option<ClientRect>,
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
    /// 때도 이 커서로 놓는다. `now` = 도착 시각(세계를 읽기 전).
    SourceEnd {
        drag_id: DragId,
        effect: DragEffect,
        /// 엔진이 접은 끌기 — 출처 웹뷰가 「시작 사건 시각 + `ENGINE_CANCEL_MS` 안의 `none` 끝」으로 가른 것.
        /// `effect = none` 일 때만 본다(판정 없이 무변경).
        canceled: bool,
        /// 끝 스냅숏 — 도착 순번으로 고른 W 가 출처일 때(또는 W 가 없고 떠남 간격이 엔진 접기일 때)만 셸이 받은 출처
        /// 마지막 `Over` 대신 쓴다. W 를 정하지는 않는다.
        last: Option<DrawnReport>,
        cursor: Option<PhysPoint>,
        rects: Rects,
        now: MonoMs,
    },
    /// 끝난 뒤의 포인터 이동 · 누름을 본 웹뷰가 알렸다.
    PointerSeen {
        drag_id: DragId,
        now: MonoMs,
    },
    /// [`Action::ArmTimer`] 가 건 시한 — `kind` · `generation` · `number` 는 그 할 일 그대로 싣는다. 세대가 지금
    /// 세션 것이 아니면 종류 무관 버린다(`Early` 는 세션 밖 시한이라 세대를 보지 않는다 — 끝난 세션의 떠남 시한이
    /// 새 세션 중에 닿아 새 출처를 숨기지 않게). 번호가 지금 걸린 시한이 아니면 버리고, 번호가 맞아도 종류가 지금
    /// 단계와 다르면 버린다 — 그때 걸린 시한은 그대로라 다시 오지 않아 세션이 끝을 잃는다. `cursor` · `rects` = 그
    /// 순간 읽은 것(`Early` · `Leave` · `OverWait` 는 쓰지 않는다 · `Drop` 은 사각형만 쓰고 커서는 끝 신호 때 것을
    /// 쓴다) · `now` = 도착 시각(세계를 읽기 전).
    Timeout {
        kind: TimerKind,
        generation: u64,
        number: u64,
        cursor: Option<PhysPoint>,
        rects: Rects,
        now: MonoMs,
    },
    /// 새 시작이 살아 있는 옛 세션 `drag_id` 를 만났다 — 그 세션은 끝 신호를 잃었으니(또는 끝을 붙들고 있으니)
    /// 「마지막 점에 놓기」로 끝낸다. 실행기는 시작을 `AlreadyDragging{applying: false}` 로 거절한 뒤([`Book::admits`]
    /// · [`Input::Start`] 어느 쪽이든) 그때의 [`Book::live_drag_id`] 를 싣고, async 한 건에서 커서 · 창 사각형을 읽어
    /// 먹인다(TRD §1-3 「시작」 1 · §7 #12). 그 사이 그 세션이 끝났거나 적용 중이 됐으면 버린다.
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

/// 장부가 낸 할 일 — 낸 순서대로 실행한다(`Hide` · `Show` · `Focus` 는 모듈 머리의 맞추기로).
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// `dock:session` 전체 emit.
    Broadcast(SessionEvent),
    /// `dock:regions` 를 그 창에만.
    Regions {
        label: WindowLabel,
        regions: DockRegions,
    },
    /// 출처 창을 숨긴다 — `generation` = 낸 세션. 맞추기는 [`Book::visibility`] 의 지금 값을 입힌다(지난 세션의
    /// `Hide` 는 아무것도 숨기지 않는다).
    Hide {
        label: WindowLabel,
        generation: u64,
    },
    /// 숨긴 출처를 되보인다 — 미룬 최대화를 입히는 보이기 경로로. 맞추기는 `Hide` 와 같다(지난 세션의 `Show` 는
    /// 새 세션이 그 창을 숨겨 두지 않았으면 보인다).
    Show {
        label: WindowLabel,
        generation: u64,
    },
    /// 같은 끝의 `Show` 뒤에 돈다 — 보이기 전에 포커스를 주지 않게.
    Focus(WindowLabel),
    Apply(DockRequest),
    ArmTimer {
        kind: TimerKind,
        generation: u64,
        number: u64,
        after_ms: u64,
    },
    /// 이미 내려간 표식이면(적용 성공) 무동작이다.
    ReleaseLease(LeaseId),
    EndLog(EndLog),
}

/// 시한 — `Drop` · `Grace` · `OverWait` 는 세션 시한(한 번에 하나) · `Leave` 는 미룬 숨기기의 떠남 시한(세션 안에서
/// 따로 하나) · `Early` 는 붙든 이른 끝 신호의 시한(세션 밖에서 따로 하나 — 세대가 없어 `generation` 은 0 이다).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerKind {
    Drop,
    Grace,
    Early,
    Leave,
    OverWait,
}

/// 출처 창의 지금 보임 — [`Book::visibility`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Hidden,
    Shown,
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
    /// 신호를 잃었거나 끝을 붙들고 있다 — 실행기가 [`Input::Superseded`] 로 끝낸다.
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
    /// 커서 판정으로 끝났나 — 빈 보고 · 보고 없는 끝(S7 ⑥).
    pub unseen: bool,
    /// 시작 측정이 있었나.
    pub strip: bool,
    /// 탭 하나 팝아웃의 숨기기를 미뤘나 — 실제로 숨겼나는 `hidden`.
    pub hide_deferred: bool,
    /// 끝 때 출처를 숨겨 두었나.
    pub hidden: bool,
    pub last: LastUse,
    /// 끝 스냅숏의 떠남 → `dragend` 간격(출처 사건 시각 · ms) 그대로.
    pub last_gap_ms: Option<f64>,
    /// 그 간격이 엔진 접기라 사각형 검사를 건너뛰었나.
    pub tight: bool,
    /// W 가 없어 끝 스냅숏으로 W = 출처를 세웠나.
    pub lifted: bool,
    /// 쓴 끝 스냅숏의 `shown` 이 셸이 받은 출처 마지막 `Over` 와 달랐나 — 보고 줄과 끝 신호의 경주가 실제로 났나.
    pub last_differs: bool,
    /// 떠남 시한이 숨긴 출처를 숨기지 않은 것으로 쳤나(「시한이 앞지른 끝」).
    pub late_hide: bool,
    /// 숨긴 출처의 쥔 사각형으로 판정했나.
    pub held_rect: bool,
    pub over_wait: OverWaitEnd,
    /// 붙든 시간 — `None` = 붙들지 않았다.
    pub over_wait_ms: Option<u64>,
    /// 붙든 동안 `Over` 가 닿은 다른 창(풀어 판정한 창이 있으면 그 창).
    pub over_from: Option<WindowLabel>,
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
    /// 엔진이 접은 끌기 — 사용자는 아무것도 보지 못했다.
    EngineCanceled,
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
            EndReason::EngineCanceled => "EngineCanceled",
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
/// 오는 중 `Over` 붙들기에서 풀려 끝난 것은 그 끝 신호(`SourceNone`)다 — 무엇이 풀었나는 `over_wait` 가 가른다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndBy {
    Drop,
    SourceNone,
    DropWait,
    SignalLost,
    NewStart,
    SourceGone,
    EngineCancel,
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
            EndBy::EngineCancel => "engine_cancel",
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
    /// 출처 창 탭바 안의 순서 이동(PRD R23).
    Reorder,
}

impl Mode {
    pub fn word(self) -> &'static str {
        match self {
            Mode::Seam => "seam",
            Mode::TabBar => "tab_bar",
            Mode::NewWindow => "new_window",
            Mode::Reuse => "reuse",
            Mode::Reorder => "reorder",
        }
    }
}

/// 끝 스냅숏을 어떻게 했나 — 끝 로그의 `last`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LastUse {
    /// 실리지 않았다.
    None,
    Used,
    /// 숨긴 출처의 것이라 버렸다(그 웹뷰는 떠남을 받지 못해 「안」으로 남는다).
    Dropped,
    /// 도착 순번 W 가 다른 창이다 — W 가 없는데 실제로 떠난 끝도 여기다.
    NotW,
}

impl LastUse {
    pub fn word(self) -> &'static str {
        match self {
            LastUse::None => "none",
            LastUse::Used => "used",
            LastUse::Dropped => "dropped",
            LastUse::NotW => "not_w",
        }
    }
}

/// 오는 중 `Over` 붙들기가 어떻게 끝났나 — 끝 로그의 `over_wait`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverWaitEnd {
    /// 붙들지 않았다.
    None,
    /// 끝 커서를 품은 창의 `Left` 로 풀었다.
    Hit,
    Timeout,
    /// 새 시작이 끊었다.
    Cut,
}

impl OverWaitEnd {
    pub fn word(self) -> &'static str {
        match self {
            OverWaitEnd::None => "none",
            OverWaitEnd::Hit => "hit",
            OverWaitEnd::Timeout => "timeout",
            OverWaitEnd::Cut => "cut",
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
    // 출처를 숨겼으면 그 까닭 — 한 번 숨기면 그 세션에서 되보이지 않는다(끝의 되보이기는 따로).
    hidden: Option<HiddenBy>,
    // 적용 중에 출처 창이 사라짐 — 끝의 되보이기를 건너뛴다.
    source_destroyed: bool,
    display_active: Option<ViewId>,
    // 한 번 서면 내리지 않는다 — 시작 때 배치로 돌아와도 그 사이 바뀐 적이 있다.
    blocked: bool,
    strip: Option<StripStart>,
    source_rect: Option<ClientRect>,
    // 탭 하나 팝아웃의 숨기기를 도킹 모드 증거까지 미뤘고 아직 숨기지 않았다.
    hide_pending: bool,
    hide_deferred: bool,
    // 출처 `Left` 가 건 숨기기 시한.
    leave: Option<LeaveTimer>,
    // 마지막 스냅숏의 창별 재료(`Pull` 이 다시 만든다).
    windows: HashMap<WindowLabel, WindowSnap>,
    // 이 세션에 `Destroyed` 가 온 창 — 그 뒤 도착한 옛 스냅숏이 되살리지 않는다(팝아웃 label 은 한 부팅 안에서
    // 다시 쓰이지 않는다).
    gone: HashSet<WindowLabel>,
    // 창별 마지막 (rev, bar_rev) — 영역을 지워도 남아 다시 보낼 때 단조를 지킨다.
    revs: HashMap<WindowLabel, (u64, u64)>,
    // 창별 지금 영역 — 놓기 대조의 기준. 숨긴 출처 몫은 끝까지 남긴다(「시한이 앞지른 끝」이 그 영역으로 판정한다).
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

#[derive(Debug, Clone, Copy, PartialEq)]
enum HiddenBy {
    // 시작 — 출처 영역에 순서 이동이 없다(5.6 그대로).
    Start,
    // 도킹 모드 증거 ① · ③ — 커서가 이미 출처 캔버스 · 출처 밖이었다.
    Evidence,
    // 증거 ② — `left_at` = 그 시한을 건 출처 `Left` 의 도착.
    Leave { left_at: MonoMs },
}

#[derive(Debug, Clone, Copy)]
struct LeaveTimer {
    number: u64,
    left_at: MonoMs,
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
        canceled: bool,
        last: Option<DrawnReport>,
        cursor: Option<PhysPoint>,
        rects: Rects,
        now: MonoMs,
    },
}

// `at` = 끝 신호(유실이면 끝 증거)의 도착 — 「마지막 점에 놓기」의 떠남 유예 기준.
#[derive(Debug)]
enum Phase {
    Live,
    // `cursor` · `last` = 그 끝 신호가 실은 것 · `early` = 그 끝 신호가 붙들었던 이른 신호였다.
    AwaitDrop {
        at: MonoMs,
        cursor: Option<PhysPoint>,
        early: bool,
        last: Option<DrawnReport>,
    },
    Grace {
        at: MonoMs,
    },
    // 오는 중 `Over` 붙들기 — `end` = 그 끝 신호가 실은 것 그대로(판정은 늘 이것으로 한다 — 붙드는 동안 움직인
    // 커서로 판정하면 끝 뒤 움직임의 틈이 넓어진다) · `from` = 끝 커서를 품은 다른 창 중 붙든 동안 `Over` 가 닿은
    // 마지막 · `other` = 붙든 동안 `Over` 가 닿은 다른 창의 마지막(로그) · `overlap` = 끝 커서를 품은 다른 창이 둘
    // 이상이다 — 어느 창이 앞인지 모르니 `Left` 로 풀지 않고 시한 뒤 도착 순번으로 고른다.
    AwaitOver {
        at: MonoMs,
        end: HeldEnd,
        from: Option<WindowLabel>,
        other: Option<WindowLabel>,
        overlap: bool,
    },
    Applying,
    Ended,
}

#[derive(Debug)]
struct HeldEnd {
    cursor: Option<PhysPoint>,
    rects: Rects,
    last: Option<DrawnReport>,
    early: bool,
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
    unseen: bool,
    last: LastUse,
    last_gap_ms: Option<f64>,
    tight: bool,
    lifted: bool,
    last_differs: bool,
    late_hide: bool,
    held_rect: bool,
    over_wait: OverWaitEnd,
    over_wait_ms: Option<u64>,
    over_from: Option<WindowLabel>,
}

impl EndCause {
    fn by(end_by: EndBy) -> Self {
        Self {
            end_by,
            over_window: None,
            cursor_read: None,
            early: false,
            leave_gap_ms: None,
            unseen: false,
            last: LastUse::None,
            last_gap_ms: None,
            tight: false,
            lifted: false,
            last_differs: false,
            late_hide: false,
            held_rect: false,
            over_wait: OverWaitEnd::None,
            over_wait_ms: None,
            over_from: None,
        }
    }
}

// 「마지막 점에 놓기」 한 번의 재료 — `at` = 끝 신호(유실이면 끝 증거)의 도착 · `cursor` = 놓은 자리로 쓸 커서 ·
// `rect_check` = 커서가 놓은 자리다(끝 뒤에 움직였으면 아니다) · `last` = 출처 끝 스냅숏 · `release` = 붙들기에서
// 풀려 왔다.
struct LastPoint {
    end_by: EndBy,
    at: MonoMs,
    cursor: Option<PhysPoint>,
    rects: Rects,
    rect_check: bool,
    early: bool,
    last: Option<DrawnReport>,
    release: Option<Release>,
}

struct Release {
    how: OverWaitEnd,
    from: Option<WindowLabel>,
    other: Option<WindowLabel>,
    waited_ms: u64,
}

// 끝 스냅숏의 떠남 간격.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gap {
    // 떠남 없음 · `CANCEL_LEAVE_MS` 안 — 엔진이 끌기를 접으며 낸 떠남이라 끝이 출처 위에서 났다.
    Tight,
    // 실제로 떠났다.
    Left,
    // 유한하지 않은 값 — 실제 떠남처럼 사각형 검사를 하되 붙들지도 W 를 세우지도 않는다.
    Unknown,
}

fn gap_of(last: &DrawnReport) -> Gap {
    match last.left_before_end_ms {
        None => Gap::Tight,
        Some(g) if !g.is_finite() => Gap::Unknown,
        Some(g) if g <= CANCEL_LEAVE_MS => Gap::Tight,
        Some(_) => Gap::Left,
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

// 놓기 판정의 갈래.
enum Judge<'a> {
    // 보고 대조 — 빈 보고면 커서 판정으로 넘긴다. `cursor` = 그때 판정할 점(그 창 뷰포트 · 끝 순간 커서) — `None` =
    // 보고의 점.
    Report {
        w: &'a str,
        report: &'a Report,
        cursor: Option<PointCss>,
    },
    // 보고 없는 끝 — 출처 창 영역과 끝 순간 커서(그 창 뷰포트)로.
    Source {
        p: PointCss,
    },
    // 판정 없이 정한 결말 — 그래도 `blocked` 가 먼저다(놓기 판정 1).
    Fixed(Verdict),
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

    /// 창 `label` 을 지금 숨겨 둘지 — 살아 있는(끝나지 않은) 세션의 출처이고 장부가 숨겼으면 [`Visibility::Hidden`],
    /// 그 밖은 [`Visibility::Shown`](끝난 세션의 출처 포함). 맞추기는 돌 때마다 이 값을 읽어 입히므로 마지막으로 돈
    /// 맞추기가 장부의 마지막 값이다.
    pub fn visibility(&self, label: &str) -> Visibility {
        match self.live() {
            Some(d) if d.source_label == label && d.hidden.is_some() => Visibility::Hidden,
            _ => Visibility::Shown,
        }
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
                strip,
                source_rect,
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
                    hidden: None,
                    source_destroyed: false,
                    display_active: None,
                    blocked: false,
                    strip,
                    source_rect,
                    hide_pending: false,
                    hide_deferred: false,
                    leave: None,
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
                single_tab_popout,
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
            } => self.left(&drag_id, label, now).into(),
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
                canceled,
                last,
                cursor,
                rects,
                now,
            } => {
                let signal = EndSignal::SourceEnd {
                    effect,
                    canceled,
                    last,
                    cursor,
                    rects,
                    now,
                };
                self.end_signal(drag_id, signal, false).into()
            }
            Input::PointerSeen { drag_id, now } => self.pointer_seen(&drag_id, now).into(),
            Input::Timeout {
                kind,
                generation,
                number,
                cursor,
                rects,
                now,
            } => self
                .timeout(kind, generation, number, cursor, rects, now)
                .into(),
            Input::Superseded {
                drag_id,
                cursor,
                rects,
                now,
            } => self.superseded(&drag_id, cursor, rects, now).into(),
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
            generation: drag.generation,
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

    // TRD S21-slot-docking §1-3 「시작」 3 · 「순서 이동 · 도킹 모드 증거」
    fn start(&mut self, mut drag: Drag, snap: Snap, single_tab_popout: bool) -> Step {
        if let Err(rejection) = self.admits(&drag.id) {
            // `admits` 를 거친 뒤에 여기서 거절되는 것은 그 사이 그 id 가 `ended` 에 적힌 경우다 — 그 id 의 이른
            // 신호 시한이 지났거나 다른 모르는 id 의 끝 신호가 그것을 밀어냈다.
            return Step {
                actions: vec![Action::ReleaseLease(drag.lease)],
                reply: Reply::Rejected(rejection),
            };
        }
        let reorders = match (
            &drag.strip,
            snap.windows.iter().find(|w| w.label == drag.source_label),
        ) {
            (Some(strip), Some(w)) => match regions::reorder_miss(strip, &w.tabs, drag.source_view)
            {
                None => true,
                Some(cause) => {
                    tracing::debug!(drag = %drag.id, cause, "시작 측정이 있는데 출처 영역에 순서 이동이 서지 않는다 — 원래 창 탭바는 끼울 자리로 판정한다");
                    false
                }
            },
            _ => false,
        };
        let mut actions = Vec::new();
        if single_tab_popout {
            if reorders {
                drag.hide_pending = true;
                drag.hide_deferred = true;
            } else {
                drag.hidden = Some(HiddenBy::Start);
                actions.push(Action::Hide {
                    label: drag.source_label.clone(),
                    generation: drag.generation,
                });
            }
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

    // TRD S21-slot-docking §1-3 「보인 것 보고」 · 「영역」 순서 · 「도킹 모드 증거」 ① · ③ · 「오는 중 `Over` 붙들기」
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
        // 붙든 동안의 출처 Over 는 적지 않는다 — 출처는 떠났다고 말했고 그 줄의 늦은 Over 는 끝 스냅숏보다 옛 그림이다.
        // 적으면 풀 때 도착 순번이 W 를 출처로 되돌려(겹친 창처럼 `from` 이 없는 풀기) 사용자가 본 다른 창 가이드 대신
        // 새 창이 되고, 끝 스냅숏 대조(`last_differs`)도 그 늦은 그림과 견주게 된다.
        if label == drag.source_label && matches!(drag.phase, Phase::AwaitOver { .. }) {
            tracing::debug!(drag = %drag.id, label = %label, event = "over", phase = "await_over", "붙든 동안의 출처 Over — 붙들기 전의 것을 그대로 둔다");
            return Vec::new();
        }
        drag.arrivals += 1;
        let rev = report.rev;
        let shown = report.shown.clone();
        drag.seen.entry(label.clone()).or_default().over = Some(OverSeen {
            report,
            arrived: Arrived {
                order: drag.arrivals,
                at: now,
            },
        });
        let mut out = Vec::new();
        // 숨긴 출처에는 영역을 보내지 않는다 — 그 몫은 끝까지 남겨 두기만 한다.
        let hidden_source = label == drag.source_label && drag.hidden.is_some();
        if let Some(current) = drag
            .sent
            .get(&label)
            .filter(|_| !hidden_source)
            .map(|s| s.regions.clone())
        {
            if rev < current.rev {
                // 웹뷰는 가장 큰 rev 만 쓰므로 옛 rev 에 머문 것은 그 emit 을 잃었다 — 두면 모든 놓기가 무변경이다.
                tracing::debug!(drag = %drag.id, label = %label, rev, current = current.rev, "옛 rev 의 보고 — 지금 영역을 다시 보낸다");
                drag.regions_resent += 1;
                out.push(Action::Regions {
                    label: label.clone(),
                    regions: current,
                });
            }
        }
        if matches!(drag.phase, Phase::Live) {
            out.extend(drag.hide_on_over(&label, rev, &shown));
        } else if let Phase::AwaitOver {
            end,
            from,
            other,
            overlap,
            ..
        } = &mut drag.phase
        {
            // 다른 창이다(출처 Over 는 위에서 돌려보냈다). 끝 커서를 품은 창의 줄만 기다린다 — 출처 → X → Y 에서 X 의
            // 늦은 보고로 풀면 Y 의 오는 중 Over 를 놓친다.
            *other = Some(label.clone());
            let holds = end.cursor.is_some_and(|c| {
                end.rects
                    .windows
                    .get(&label)
                    .is_some_and(|r| contains(r, c))
            });
            if holds && !*overlap {
                *from = Some(label);
            }
        }
        out
    }

    // TRD S21-slot-docking §1-3 「보인 것 보고」 · 「도킹 모드 증거」 ② · 「오는 중 `Over` 붙들기」 풀기
    fn left(&mut self, drag_id: &str, label: WindowLabel, now: MonoMs) -> Vec<Action> {
        let Some(drag) = self.session_for(drag_id, "left") else {
            return Vec::new();
        };
        drag.reports += 1;
        drag.arrivals += 1;
        drag.seen.entry(label.clone()).or_default().left = Some(Arrived {
            order: drag.arrivals,
            at: now,
        });
        // 같은 창의 Over · Left 는 한 줄로 낸 순서대로 닿는다 — 그 Left 가 닿았으면 그 창이 앞서 낸 Over 는 다 닿았다.
        if matches!(&drag.phase, Phase::AwaitOver { from: Some(f), overlap: false, .. } if *f == label)
        {
            return self.release(OverWaitEnd::Hit, now);
        }
        // 구역 안 Esc 도 출처에 떠남을 낸다 — 곧바로 숨기면 끝에서 출처가 W 후보에서 빠져 팝아웃이 커서 자리로 뛴다.
        // 그래서 시한만 걸고, 그 전에 끝 신호 · 출처 Over 가 오면 숨기지 않는다.
        if !(matches!(drag.phase, Phase::Live) && drag.hide_pending && label == drag.source_label) {
            return Vec::new();
        }
        let number = self.next_timer();
        let Some(drag) = self.live_mut() else {
            return Vec::new();
        };
        drag.leave = Some(LeaveTimer {
            number,
            left_at: now,
        });
        tracing::debug!(drag = %drag.id, label = %label, after_ms = LEAVE_GRACE_MS, "출처 떠남 — 미룬 숨기기 시한을 건다");
        vec![Action::ArmTimer {
            kind: TimerKind::Leave,
            generation: drag.generation,
            number,
            after_ms: LEAVE_GRACE_MS,
        }]
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
                canceled,
                last,
                cursor,
                rects,
                now,
            } => self.source_end(effect, canceled, last, cursor, rects, now, early),
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
        if matches!(drag.phase, Phase::AwaitOver { .. }) {
            tracing::debug!(drag = %drag.id, event = "drop", phase = drag.phase.word(), "끝을 붙든 세션의 놓기 — 출처 끝이 none 이라 받은 창이 없다. 늦은 신호로 버린다");
            return Vec::new();
        }
        let accepted = !early && !matches!(drag.phase, Phase::Applying);
        if let Some(drag) = self.live_mut().filter(|_| accepted) {
            drag.note_rects(rects);
        }
        let at = cursor.or_else(|| rects.windows.get(label).map(|r| screen_of(r, report.point)));
        let cause = EndCause {
            over_window: Some(label.to_string()),
            cursor_read: Some(cursor.is_some()),
            early,
            ..EndCause::by(EndBy::Drop)
        };
        self.judge(
            Judge::Report {
                w: label,
                report,
                cursor: None,
            },
            at,
            rects.source_scale,
            cause,
        )
    }

    // 출처 `dragend` — 받은 창이 있으면(`effect ≠ none`) 그 창의 놓기를 기다리고, 없으면 엔진 취소 · 붙들기 ·
    // 「마지막 점에 놓기」.
    // TRD S21-slot-docking §1-3 「끝 신호 셋」 · 「엔진 취소 끝」 · 「오는 중 `Over` 붙들기」
    #[allow(clippy::too_many_arguments)]
    fn source_end(
        &mut self,
        effect: DragEffect,
        canceled: bool,
        last: Option<DrawnReport>,
        cursor: Option<PhysPoint>,
        rects: Rects,
        now: MonoMs,
        early: bool,
    ) -> Vec<Action> {
        let Some(drag) = self.live_mut() else {
            return Vec::new();
        };
        // 유예 중의 끝 신호는 평소대로다 — 놓기 기다리기로 갈아타면 유예 시한은 번호가 어긋나 버려진다.
        if !matches!(drag.phase, Phase::Live | Phase::Grace { .. }) {
            tracing::debug!(drag = %drag.id, event = "source_end", phase = drag.phase.word(), "끝 신호를 이미 받은 세션 — 늦은 신호를 버린다");
            return Vec::new();
        }
        if !early {
            drag.note_rects(&rects);
        }
        if effect != DragEffect::None {
            tracing::debug!(drag = %drag.id, from = drag.phase.word(), to = "await_drop", reason = "source_end", ?effect, canceled, early, "받은 창의 놓기를 기다린다");
            drag.phase = Phase::AwaitDrop {
                at: now,
                cursor,
                early,
                last,
            };
            return self.arm_session(TimerKind::Drop, DROP_WAIT_MS);
        }
        if canceled {
            // OS 끌기가 열리지 않았거나 문턱 직후 곧바로 끝나 커서가 누른 자리 그대로다 — 사용자는 아무것도 못 봤다.
            tracing::debug!(drag = %drag.id, from = drag.phase.word(), to = "ended", reason = "engine_cancel", early, "엔진이 접은 끌기 — 판정 없이 끝낸다");
            drag.cause = Some(EndCause {
                early,
                ..EndCause::by(EndBy::EngineCancel)
            });
            return self.end(Ending::Noop(EndReason::EngineCanceled));
        }
        if let Some(overlap) = drag.holds_for_over(now, cursor, &rects, last.as_ref()) {
            tracing::debug!(drag = %drag.id, from = drag.phase.word(), to = "await_over", reason = "source_end", overlap, early, "출처는 떠났다는데 다른 창 Over 가 아직 없다 — 오는 중 Over 를 기다리며 끝을 붙든다");
            drag.phase = Phase::AwaitOver {
                at: now,
                end: HeldEnd {
                    cursor,
                    rects,
                    last,
                    early,
                },
                from: None,
                other: None,
                overlap,
            };
            return self.arm_session(TimerKind::OverWait, OVER_WAIT_MS);
        }
        self.last_point(LastPoint {
            end_by: EndBy::SourceNone,
            at: now,
            cursor,
            rects,
            rect_check: true,
            early,
            last,
            release: None,
        })
    }

    // 끝난 뒤의 포인터 이동 — 끝 신호를 잃었을 수 있으니 유예를 둔다. 끝을 기다리는 중(놓기 기다리기 · 이미 유예 ·
    // 붙들기)이면 버린다 — 이동이 이어져도 유예 시한을 늘리지 않는다.
    // TRD S21-slot-docking §1-3 「`dock_pointer_seen`」
    fn pointer_seen(&mut self, drag_id: &str, now: MonoMs) -> Vec<Action> {
        let Some(drag) = self.session_for(drag_id, "pointer_seen") else {
            return Vec::new();
        };
        if !matches!(drag.phase, Phase::Live) {
            tracing::debug!(drag = %drag.id, event = "pointer_seen", phase = drag.phase.word(), "끝을 기다리는 중 — 포인터 이동을 버린다");
            return Vec::new();
        }
        tracing::debug!(drag = %drag.id, from = "live", to = "grace", reason = "pointer_seen", "끝난 뒤의 포인터 이동 — 끝 신호를 기다린다");
        drag.phase = Phase::Grace { at: now };
        self.arm_session(TimerKind::Grace, END_GRACE_MS)
    }

    fn timeout(
        &mut self,
        kind: TimerKind,
        generation: u64,
        number: u64,
        cursor: Option<PhysPoint>,
        rects: Rects,
        now: MonoMs,
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
        let current = self.drag.as_ref().map(|d| d.generation);
        if current != Some(generation) {
            tracing::debug!(
                ?kind,
                number,
                generation,
                ?current,
                "지난 세션의 시한 — 버린다"
            );
            return Vec::new();
        }
        if kind == TimerKind::Leave {
            return self.leave_timeout(number);
        }
        let armed = self.live().and_then(|d| d.timer);
        if armed != Some(number) {
            tracing::debug!(?kind, number, ?armed, "지난 시한 — 버린다");
            return Vec::new();
        }
        let Some(drag) = self.live_mut() else {
            return Vec::new();
        };
        match (kind, std::mem::replace(&mut drag.phase, Phase::Live)) {
            (
                TimerKind::Drop,
                Phase::AwaitDrop {
                    at,
                    cursor: held,
                    early,
                    last,
                },
            ) => {
                drag.timer = None;
                drag.note_rects(&rects);
                self.last_point(LastPoint {
                    end_by: EndBy::DropWait,
                    at,
                    cursor: held,
                    rects,
                    rect_check: true,
                    early,
                    last,
                    release: None,
                })
            }
            // 유실 — 커서는 끝난 뒤 움직였으므로 W 사각형 검사를 하지 않는다.
            (TimerKind::Grace, Phase::Grace { at }) => {
                drag.timer = None;
                drag.note_rects(&rects);
                self.last_point(LastPoint {
                    end_by: EndBy::SignalLost,
                    at,
                    cursor,
                    rects,
                    rect_check: false,
                    early: false,
                    last: None,
                    release: None,
                })
            }
            (
                TimerKind::OverWait,
                Phase::AwaitOver {
                    at,
                    end,
                    from,
                    other,
                    ..
                },
            ) => {
                drag.timer = None;
                self.release_hold(at, end, from, other, OverWaitEnd::Timeout, now)
            }
            (_, phase) => {
                tracing::debug!(drag = %drag.id, ?kind, number, phase = phase.word(), "단계와 맞지 않는 시한 — 버린다");
                drag.phase = phase;
                Vec::new()
            }
        }
    }

    // 도킹 모드 증거 ② — 떠남 뒤 시한까지 끝 신호도 출처의 새 Over 도 없었다. `Live` 가 아니면(끝 신호가 먼저 왔다)
    // 숨기지 않는다 — 구역 안 Esc 의 떠남이 그렇다.
    fn leave_timeout(&mut self, number: u64) -> Vec<Action> {
        let Some(drag) = self.drag.as_mut() else {
            return Vec::new();
        };
        let Some(leave) = drag.leave.filter(|l| l.number == number) else {
            tracing::debug!(kind = ?TimerKind::Leave, number, armed = ?drag.leave.map(|l| l.number), "지난 시한 — 버린다");
            return Vec::new();
        };
        drag.leave = None;
        if !(matches!(drag.phase, Phase::Live) && drag.hide_pending) {
            tracing::debug!(drag = %drag.id, label = %drag.source_label, event = "left_canceled", phase = drag.phase.word(), "끝 신호가 앞지른 떠남 시한 — 숨기지 않는다");
            return Vec::new();
        }
        drag.hide_now(
            HiddenBy::Leave {
                left_at: leave.left_at,
            },
            "left_timeout",
            None,
            None,
        )
    }

    // 놓기 기다리기의 옛 세션은 그 끝 신호의 커서 · 도착으로, 끝 신호가 없는 옛 세션은 유실처럼 놓는다 — 새 누름이
    // 있었으니 지금 커서는 놓은 자리가 아니다. 끝을 붙든 옛 세션은 붙든 끝으로 곧바로 놓는다(시한이 닿은 것과 같다).
    // TRD S21-slot-docking §1-3 「시작」 1 · §7 #12
    fn superseded(
        &mut self,
        drag_id: &str,
        cursor: Option<PhysPoint>,
        rects: Rects,
        now: MonoMs,
    ) -> Vec<Action> {
        let Some(drag) = self.session_for(drag_id, "superseded") else {
            return Vec::new();
        };
        let (at, cursor, rect_check, early, last) = match std::mem::replace(
            &mut drag.phase,
            Phase::Live,
        ) {
            Phase::Live => (now, cursor, false, false, None),
            Phase::Grace { at } => (at, cursor, false, false, None),
            Phase::AwaitDrop {
                at,
                cursor: held,
                early,
                last,
            } => (at, held, true, early, last),
            Phase::AwaitOver {
                at,
                end,
                from,
                other,
                ..
            } => {
                drag.timer = None;
                return self.release_hold(at, end, from, other, OverWaitEnd::Cut, now);
            }
            phase @ (Phase::Applying | Phase::Ended) => {
                tracing::debug!(drag = %drag.id, event = "superseded", phase = phase.word(), "이미 판정한 세션 — 버린다");
                drag.phase = phase;
                return Vec::new();
            }
        };
        drag.note_rects(&rects);
        self.last_point(LastPoint {
            end_by: EndBy::NewStart,
            at,
            cursor,
            rects,
            rect_check,
            early,
            last,
            release: None,
        })
    }

    // 붙들기를 푼다 — 붙든 끝이 아닌 단계면 무동작.
    fn release(&mut self, how: OverWaitEnd, now: MonoMs) -> Vec<Action> {
        let Some(drag) = self.live_mut() else {
            return Vec::new();
        };
        match std::mem::replace(&mut drag.phase, Phase::Live) {
            Phase::AwaitOver {
                at,
                end,
                from,
                other,
                ..
            } => {
                drag.timer = None;
                self.release_hold(at, end, from, other, how, now)
            }
            phase => {
                drag.phase = phase;
                Vec::new()
            }
        }
    }

    // 붙든 끝으로 「마지막 점에 놓기」를 그대로 돈다 — `from` 이 있으면 W = 그 창(사용자가 본 그 가이드).
    // TRD S21-slot-docking §1-3 「오는 중 `Over` 붙들기」 풀기
    fn release_hold(
        &mut self,
        at: MonoMs,
        end: HeldEnd,
        from: Option<WindowLabel>,
        other: Option<WindowLabel>,
        how: OverWaitEnd,
        now: MonoMs,
    ) -> Vec<Action> {
        tracing::debug!(
            drag = ?self.live().map(|d| &d.id),
            from = "await_over",
            to = "judge",
            reason = how.word(),
            window = ?from,
            waited_ms = now.since(at),
            "붙든 끝을 푼다"
        );
        self.last_point(LastPoint {
            end_by: EndBy::SourceNone,
            at,
            cursor: end.cursor,
            rects: end.rects,
            rect_check: true,
            early: end.early,
            last: end.last,
            release: Some(Release {
                how,
                from,
                other,
                waited_ms: now.since(at),
            }),
        })
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
            generation: 0,
            number: timer,
            after_ms: EARLY_SIGNAL_MS,
        }]
    }

    /// 「마지막 점에 놓기」 — 끝 신호가 놓은 자리를 싣지 않을 때(출처 끝 none · 놓기 기다리기 시한 · 유실 · 새 시작 ·
    /// 붙들기 풀기). W 는 늘 도착 순번으로 고르고(붙들기가 끝 커서를 품은 창을 정했으면 그 창), 끝 스냅숏은 W 가
    /// 출처일 때 그 마지막 보고 자리에 들어갈 뿐이다.
    // TRD S21-slot-docking §1-3 「끝 신호 셋과 「마지막 점에 놓기」」 · 「끝 스냅숏 `last`」 · 「시한이 앞지른 끝」
    fn last_point(&mut self, lp: LastPoint) -> Vec<Action> {
        let LastPoint {
            end_by,
            at,
            cursor,
            rects,
            rect_check,
            early,
            last,
            release,
        } = lp;
        let Some(drag) = self.live() else {
            return Vec::new();
        };
        let source = drag.source_label.clone();
        let late_hide = drag.late_hide(at, last.as_ref());
        let usable = drag.hidden.is_none() || late_hide;
        let gap = last.as_ref().map(gap_of);
        let mut w = match release.as_ref().and_then(|r| r.from.as_deref()) {
            Some(from) => drag.over_of(from, at).map(|o| LastOver {
                still_over: true,
                ..o
            }),
            None => drag.last_over(at, late_hide),
        };
        let mut cause = EndCause {
            cursor_read: Some(cursor.is_some()),
            early,
            last: match (&last, usable) {
                (None, _) => LastUse::None,
                (Some(_), false) => LastUse::Dropped,
                (Some(_), true) => LastUse::NotW,
            },
            last_gap_ms: last.as_ref().and_then(|l| l.left_before_end_ms),
            late_hide,
            ..EndCause::by(end_by)
        };
        if let Some(r) = &release {
            cause.over_wait = r.how;
            cause.over_wait_ms = Some(r.waited_ms);
            cause.over_from = r.from.clone().or_else(|| r.other.clone());
        }
        if let (Some(drawn), true) = (&last, usable) {
            let seen_shown = drag
                .seen
                .get(&source)
                .and_then(|s| s.over.as_ref())
                .map(|o| &o.report.shown);
            match &mut w {
                // 출처의 「아직 위」는 출처 시계가 이미 쟀다 — 셸 도착 유예를 보지 않는다.
                Some(c) if c.label == source => {
                    cause.last_differs = seen_shown != Some(&drawn.shown);
                    c.report = Report::from(drawn);
                    c.still_over = true;
                    cause.last = LastUse::Used;
                    cause.tight = gap == Some(Gap::Tight);
                }
                // W 없음 — 출처 첫 그린 Over 가 아직 오는 중이다. 다른 창 Over 가 하나도 없으니 다른 창이 고른 W 를
                // 갈아 치우는 것이 아니다.
                None if gap == Some(Gap::Tight) => {
                    cause.last_differs = seen_shown != Some(&drawn.shown);
                    w = Some(LastOver {
                        label: source.clone(),
                        report: Report::from(drawn),
                        still_over: true,
                        leave_gap_ms: None,
                    });
                    cause.last = LastUse::Used;
                    cause.lifted = true;
                    cause.tight = true;
                }
                _ => {}
            }
        }
        cause.over_window = w.as_ref().map(|l| l.label.clone());
        cause.leave_gap_ms = w.as_ref().and_then(|l| l.leave_gap_ms);
        let scale = rects.source_scale;
        let Some(LastOver {
            label: w,
            report,
            still_over,
            ..
        }) = w
        else {
            // W 없음 — 멈춘 채 떼기 · 짧은 톡은 OLE 가 다음 마우스 메시지 전에 DragEnter 를 내지 않아 어느 창도
            // 보고가 없다(실측 2026-10-17). 출처는 누르는 순간 앞으로 와 그 사각형 안의 맨 위 창이다(TRD §11-2
            // #28). 유실 · 새 시작은 커서가 끝 뒤에 움직였으므로 보지 않는다.
            let (rect, held) = drag.rect_of(&source, &rects, late_hide);
            if let (true, Some(c), Some(r)) = (rect_check, cursor, rect) {
                if contains(&r, c) {
                    cause.held_rect = held;
                    return self.judge(
                        Judge::Source {
                            p: viewport_point(&r, c),
                        },
                        cursor,
                        scale,
                        cause,
                    );
                }
            }
            return self.judge(Judge::Fixed(Verdict::NewWindow), cursor, scale, cause);
        };
        let (w_rect, held) = drag.rect_of(&w, &rects, late_hide);
        if cause.tight {
            // 끝이 출처 위에서 났다 — 셸이 커서를 읽은 것은 끝 신호가 닿은 뒤라, 그 사이 커서가 출처 밖으로 나가도
            // 사용자가 본 그 자리에 놓는다.
            let known = match cursor {
                Some(c) => Some(c),
                None => {
                    cause.held_rect = held;
                    w_rect.map(|r| screen_of(&r, report.point))
                }
            };
            return self.judge(
                Judge::Report {
                    w: &w,
                    report: &report,
                    cursor: None,
                },
                known,
                scale,
                cause,
            );
        }
        let Some(c) = cursor else {
            // 잠금 화면 · 보안 데스크톱(AC21) — 마지막으로 알려진 점 = W 의 마지막 Over 점.
            cause.held_rect = held;
            let known = w_rect.map(|r| screen_of(&r, report.point));
            let how = if still_over {
                Judge::Report {
                    w: &w,
                    report: &report,
                    cursor: None,
                }
            } else {
                Judge::Fixed(Verdict::NewWindow)
            };
            return self.judge(how, known, scale, cause);
        };
        if !still_over {
            return self.judge(Judge::Fixed(Verdict::NewWindow), cursor, scale, cause);
        }
        if !rect_check {
            return self.judge(
                Judge::Report {
                    w: &w,
                    report: &report,
                    cursor: None,
                },
                cursor,
                scale,
                cause,
            );
        }
        cause.held_rect = held;
        let how = match w_rect {
            Some(r) if contains(&r, c) => Judge::Report {
                w: &w,
                report: &report,
                cursor: Some(viewport_point(&r, c)),
            },
            // 창 밖으로 빠르게 나가 뗀 떠남 보고가 아직 오는 중이거나 끝 신호 뒤 커서가 움직였다 — 그 자리엔
            // 사용자에게 보인 가이드가 없다(§11-2 #1).
            Some(_) => Judge::Fixed(Verdict::NewWindow),
            // 사라진 대상(소멸 · 숨김 · 최소화 · 못 읽음)의 본 가이드에는 붙이지 않는다(Q18).
            None if report.shown != Picked::None => {
                Judge::Fixed(Verdict::Refuse(EndReason::TargetChanged))
            }
            None => Judge::Fixed(Verdict::NewWindow),
        };
        self.judge(how, cursor, scale, cause)
    }

    /// 놓기 판정 — 보인 것과 장부의 지금 판정이 같을 때만 무엇이든 바뀐다(빈 보고 · 보고 없는 끝은 커서 판정). `at` =
    /// 새 창 자리의 기준점(커서 · 그것을 못 읽으면 마지막으로 알려진 점의 화면 자리 · `None` = 기본 자리) ·
    /// `source_scale` = 출처 배율. 이미 판정한 세션(적용 중)이면 늦은 신호로 버린다. 적용을 내면 같은 걸음에서
    /// `Applying` 이 선다 — 한 세션에 적용은 많아야 한 번이다.
    // TRD S21-slot-docking §1-3 「놓기 판정」 · 「커서 판정」
    fn judge(
        &mut self,
        how: Judge<'_>,
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
            match how {
                Judge::Report { w, report, cursor } => drag.verdict(w, report, cursor),
                Judge::Source { p } => {
                    let source = drag.source_label.clone();
                    // 보고 없는 끌기는 출처 웹뷰가 한 번도 판정하지 못해 순서 이동 모습 그대로였다 — 캔버스는 모델
                    // 활성을 보였다.
                    let shown = drag.sent.get(&source).map(|s| s.expect.active);
                    drag.cursor_verdict(&source, p, None, shown, 0)
                }
                Judge::Fixed(v) => v,
            }
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
        // 숨긴 출처는 영역을 받지 않는다 — 장부가 남겨 둔 그 몫은 놓기 판정 재료일 뿐이다.
        let hidden_source = label == drag.source_label && drag.hidden.is_some();
        let regions = match drag.windows.get(label).cloned() {
            Some(w) if !hidden_source => {
                if drag.renew(&w) {
                    drag.regions_sent += 1;
                }
                drag.sent.get(label).map(|s| s.regions.clone())
            }
            _ => None,
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
        // 숨긴 출처는 적용이 비워 닫았을 때만 그대로 둔다 — 재사용(옮긴 창) · 대상이 출처인 적용(시한이 앞지른 끝의
        // 순서 이동)이나, 끄는 동안 다른 경로가 탭을 더 넣어 성공한 적용 뒤에도 남은 팝아웃은 되보여야 한다(안
        // 보이면 그 탭에 닿을 길이 없다). 미룬 채 끝났으면 숨긴 적이 없어 되보이기도 없다.
        let closed = applied
            .as_ref()
            .is_some_and(|o| o.closed_window.as_deref() == Some(drag.source_label.as_str()));
        if drag.hidden.is_some() && !closed && !vanished {
            out.push(Action::Show {
                label: drag.source_label.clone(),
                generation: drag.generation,
            });
        }
        if let Some(o) = &applied {
            if o.reused {
                drag.mode = Some(Mode::Reuse);
            }
            drag.window = Some(o.window.clone());
            out.push(Action::Focus(o.window.clone()));
        }
        out.push(Action::ReleaseLease(drag.lease));
        let end_by = drag.cause.as_ref().map(|c| c.end_by);
        let cause = drag
            .cause
            .clone()
            .unwrap_or_else(|| EndCause::by(EndBy::SourceGone));
        out.push(Action::EndLog(EndLog {
            drag_id: drag.id.clone(),
            generation: drag.generation,
            outcome,
            reason,
            end_by,
            mode: drag.mode,
            window: drag.window.clone(),
            over_window: cause.over_window,
            leave_gap_ms: cause.leave_gap_ms,
            cursor_read: cause.cursor_read,
            early: cause.early,
            unseen: cause.unseen,
            strip: drag.strip.is_some(),
            hide_deferred: drag.hide_deferred,
            hidden: drag.hidden.is_some(),
            last: cause.last,
            last_gap_ms: cause.last_gap_ms,
            tight: cause.tight,
            lifted: cause.lifted,
            last_differs: cause.last_differs,
            late_hide: cause.late_hide,
            held_rect: cause.held_rect,
            over_wait: cause.over_wait,
            over_wait_ms: cause.over_wait_ms,
            over_from: cause.over_from,
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

impl Phase {
    fn word(&self) -> &'static str {
        match self {
            Phase::Live => "live",
            Phase::AwaitDrop { .. } => "await_drop",
            Phase::Grace { .. } => "grace",
            Phase::AwaitOver { .. } => "await_over",
            Phase::Applying => "applying",
            Phase::Ended => "ended",
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
    // 본 것(`seen`)은 남겨야 놓기 판정이 「대상 사라짐」을 안다. 숨긴 출처 몫은 건너뛴다(지우지도 고치지도 않는다).
    fn refresh(&mut self, windows: Vec<WindowSnap>) -> Vec<Action> {
        let mut out = Vec::new();
        let mut material = HashMap::new();
        for mut w in windows {
            if self.gone.contains(&w.label) {
                continue;
            }
            w.is_source = w.label == self.source_label;
            w.hidden = w.is_source && self.hidden.is_some();
            if !w.hidden && self.renew(&w) {
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

    // 창 하나의 영역을 다시 만든다 — 마지막 보낸 것과 다르면 번호를 올려 지금 영역으로 적고 `true`. 숨긴 출처는
    // 부르지 않는다(영역이 없다).
    fn renew(&mut self, w: &WindowSnap) -> bool {
        let Some((mut regions, expect)) = regions::regions_for(
            w,
            self.source_view,
            &self.expect_source,
            self.blocked,
            self.strip.as_ref(),
        ) else {
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

    // 도킹 모드 증거 ① · ③(TRD §1-3) — `Live` 에서만 부른다. 영역 전 · 막힌 영역 · 옛 rev 의 출처 보고는 웹뷰가
    // 판정하지 못한 것이라(그 웹뷰는 모드를 바꾸지 않는다) 증거가 아니다 — 치면 커서가 아직 탭바 안인데 팝아웃이
    // 사라진다.
    fn hide_on_over(&mut self, label: &str, rev: u64, shown: &Picked) -> Vec<Action> {
        if !self.hide_pending {
            return Vec::new();
        }
        if label != self.source_label {
            // 커서가 출처 창 밖이라는 OS 의 증거다(출처 창은 누름으로 맨 앞 — §11-2 #28).
            return self.hide_now(HiddenBy::Evidence, "other_over", Some(label), Some(shown));
        }
        // 출처의 새 Over 는 걸린 떠남 시한을 버린다 — 그 보고로 가른다.
        self.leave = None;
        if matches!(shown, Picked::Reorder { .. }) {
            return Vec::new();
        }
        let current = self
            .sent
            .get(label)
            .map(|s| (s.regions.rev, s.regions.blocked));
        let filtered = match current {
            None => Some("noregion"),
            // 영역 전 — 세션 사건이 영역보다 먼저 닿은 웹뷰도 세션 id 만으로 `rev` 0 보고를 낸다(실측 3/10).
            _ if rev == 0 => Some("rev0"),
            Some((_, true)) => Some("blocked"),
            Some((sent_rev, _)) if sent_rev != rev => Some("stale"),
            Some(_) => None,
        };
        if let Some(filtered) = filtered {
            tracing::debug!(drag = %self.id, label, event = "over", shown = ?shown, filtered, "도킹 모드 증거가 아닌 출처 보고 — 숨기지 않는다");
            return Vec::new();
        }
        self.hide_now(HiddenBy::Evidence, "over", None, Some(shown))
    }

    // 미룬 숨기기를 낸다 — 출처 영역은 지우지 않고 남긴다(그 뒤 스냅숏은 그 몫을 건너뛴다). `by_window` = 증거를 낸 다른
    // 창(`other_over` 일 때만).
    fn hide_now(
        &mut self,
        by: HiddenBy,
        event: &'static str,
        by_window: Option<&str>,
        shown: Option<&Picked>,
    ) -> Vec<Action> {
        self.hide_pending = false;
        self.leave = None;
        self.hidden = Some(by);
        tracing::debug!(drag = %self.id, label = %self.source_label, event, by = by_window, shown = ?shown, "미룬 숨기기 — 도킹 모드 증거로 출처를 숨긴다");
        vec![Action::Hide {
            label: self.source_label.clone(),
            generation: self.generation,
        }]
    }

    // 받아들인 입력의 `Rects` 가 출처를 들었으면 쥔 출처 사각형을 갈아 끼운다 — 숨긴 뒤의 `Rects` 에는 출처가 없어 숨기기
    // 직전 값이 남는다(숨기기는 창을 옮기지 않으므로 그것이 지금 자리다). 버리는 입력(지난 시한 · 늦은 신호)과 시작에 이어
    // 먹이는 붙든 이른 신호는 갈아 끼우지 않는다 — 시작 뒤에 읽은 사각형을 그보다 옛 읽기로 덮게 된다.
    fn note_rects(&mut self, rects: &Rects) {
        if let Some(r) = rects.windows.get(&self.source_label) {
            self.source_rect = Some(*r);
        }
    }

    // 「시한이 앞지른 끝」 — 떠남 시한이 숨긴 출처를, 그 떠남의 도착이 끝 도착 전 `LEAVE_GRACE_MS` 안이면(끝이 시한
    // 전에 닿았다) · 또는 끝이 스냅숏을 실었으면(웹뷰가 자기 사건 시각으로 같은 유예를 이미 쟀다) 숨기지 않은 것으로
    // 친다. 증거 ① · ③ 으로 숨긴 출처는 아니다(커서가 출처 캔버스 · 출처 밖이었다).
    fn late_hide(&self, at: MonoMs, last: Option<&DrawnReport>) -> bool {
        matches!(self.hidden, Some(HiddenBy::Leave { left_at })
            if at.since(left_at) < LEAVE_GRACE_MS || last.is_some())
    }

    // 창 `label` 의 사각형과 그것이 쥔 출처 사각형인지 — 끝 신호 때 읽은 것, 없고 「시한이 앞지른 끝」의 출처면 쥔
    // `source_rect`.
    fn rect_of(&self, label: &str, rects: &Rects, late_hide: bool) -> (Option<ClientRect>, bool) {
        if let Some(r) = rects.windows.get(label) {
            return (Some(*r), false);
        }
        if late_hide && label == self.source_label {
            return (self.source_rect, self.source_rect.is_some());
        }
        (None, false)
    }

    // 「마지막 Over 창」 W = 마지막 Over 의 도착 순번이 가장 늦은 창 — 뒤에 Left 가 와도 W 다(숨긴 출처는 후보가
    // 아니다 — 「시한이 앞지른 끝」이면 후보다).
    fn last_over(&self, at: MonoMs, late_hide: bool) -> Option<LastOver> {
        let hidden_source = self.hidden.is_some() && !late_hide;
        let label = self
            .seen
            .iter()
            .filter(|(label, _)| !(hidden_source && **label == self.source_label))
            .filter_map(|(label, s)| s.over.as_ref().map(|o| (label, o.arrived.order)))
            .max_by_key(|(_, order)| *order)?
            .0;
        self.over_of(label, at)
    }

    // 창 `label` 의 마지막 Over. 아직 위 = 그 Over 뒤에 Left 가 없거나, 그 Left 가 끝 신호 도착 `at` 전
    // `LEAVE_GRACE_MS` 미만 — Esc 취소의 떠남은 `dragend` 보다 먼저 닿을 수 있다. `at` 뒤에 도착한 Left 는 간격 0 이라
    // 아직 위다(끝 신호 때는 오지 않았던 보고다 — 붙든 동안 닿은 떠남도 그렇다).
    fn over_of(&self, label: &str, at: MonoMs) -> Option<LastOver> {
        let seen = self.seen.get(label)?;
        let over = seen.over.as_ref()?;
        let leave_gap_ms = seen
            .left
            .filter(|l| l.order > over.arrived.order)
            .map(|l| at.since(l.at));
        Some(LastOver {
            label: label.to_string(),
            report: over.report.clone(),
            still_over: leave_gap_ms.is_none_or(|gap| gap < LEAVE_GRACE_MS),
            leave_gap_ms,
        })
    }

    // 오는 중 `Over` 붙들기의 조건(TRD §1-3) — 출처 끝 스냅숏이 실제로 떠났다고 말하는데 도착 순번 W 가 출처이거나
    // 없고, 끝 커서를 품은 다른 우리 창이 있다. 붙들면 `Some(겹침 — 품은 창이 둘 이상)`. 기다릴 대상 = 그 창들이다 —
    // 출처는 떠났다고 말했고, 아무 창도 품지 않으면 창 밖이다.
    fn holds_for_over(
        &self,
        at: MonoMs,
        cursor: Option<PhysPoint>,
        rects: &Rects,
        last: Option<&DrawnReport>,
    ) -> Option<bool> {
        let last = last?;
        if self.blocked || gap_of(last) != Gap::Left {
            return None;
        }
        let late_hide = self.late_hide(at, Some(last));
        if self.hidden.is_some() && !late_hide {
            return None;
        }
        if self
            .last_over(at, late_hide)
            .is_some_and(|w| w.label != self.source_label)
        {
            return None;
        }
        let c = cursor?;
        let holders = rects
            .windows
            .iter()
            .filter(|(label, r)| **label != self.source_label && contains(r, c))
            .count();
        (holders > 0).then_some(holders > 1)
    }

    // 놓기 판정 2–4(1 = blocked 는 부르는 쪽이 먼저 본다). `cursor` = 빈 보고를 판정할 점 — `None` = 보고의 점.
    fn verdict(&mut self, w: &str, report: &Report, cursor: Option<PointCss>) -> Verdict {
        // 빈 보고 — 판정할 영역이 없던 보고는 「사용자가 가이드 없음을 봤다」의 증거가 못 된다. 0 이 아닌 옛 rev 의
        // 「가이드 없음」은 실제 영역으로 판정해 그린 것이라 보고다.
        if report.shown == Picked::None && (report.rev == 0 || !self.sent.contains_key(w)) {
            return self.cursor_verdict(
                w,
                cursor.unwrap_or(report.point),
                report.scroll,
                report.displayed,
                report.rev,
            );
        }
        let current = self.sent.get(w);
        let judged = current.map_or(Picked::None, |s| {
            regions::pick(&s.regions, report.point, report.scroll, self.source_view)
        });
        match (&report.shown, current) {
            (Picked::None, _) if judged == Picked::None => Verdict::NewWindow,
            // 이음새 — 그 rev 의 영역 · 보이던 탭 · 다시 고른 것이 모두 같을 때만.
            (Picked::Seam { .. }, Some(s))
                if report.rev == s.regions.rev
                    && report.displayed == s.regions.view_id
                    && judged == report.shown =>
            {
                dock_at(w, &report.shown, s)
            }
            // 탭바 — 탭바 몫과 끼울 자리만 본다. 활성 전환은 rev 만 올리므로 rev · 보이던 탭까지 보면 전환 알림
            // 틈의 탭바 놓기가 무변경이 된다.
            (Picked::TabBar { index, .. }, Some(s))
                if report.bar_rev == s.regions.bar_rev
                    && matches!(judged, Picked::TabBar { index: j, .. } if j == *index) =>
            {
                dock_at(w, &report.shown, s)
            }
            // 순서 이동(4c) — 순서 이동 영역은 세션 내내 얼린 시작 측정이라 같은 점의 판정은 늘 같다. 번호 · 보이던
            // 탭까지 보면 구역에 돌아온 직후의 놓기가 크롬 보고 왕복 틈에 무변경이 된다(그 번호가 오르는 까닭은 이
            // 판정과 무관하다).
            (Picked::Reorder { .. }, Some(s))
                if w == self.source_label && judged == report.shown =>
            {
                dock_at(w, &report.shown, s)
            }
            _ => Verdict::Refuse(EndReason::TargetChanged),
        }
    }

    // 커서 판정(TRD §1-3) — 창 `t` 의 지금 영역 위 점 `p` 로 판정한다. `v` = 그 창 캔버스가 보이던 탭(캔버스 거름 —
    // 웹뷰 그리기와 같은 거름) · `rev` = 그 보고의 번호(로그).
    fn cursor_verdict(
        &mut self,
        t: &str,
        p: PointCss,
        scroll: Option<f64>,
        v: Option<ViewId>,
        rev: u64,
    ) -> Verdict {
        let from = match &mut self.cause {
            Some(c) => {
                c.unseen = true;
                match (c.end_by, c.early) {
                    (EndBy::Drop, true) => "held",
                    (EndBy::Drop, false) => "drop",
                    _ => "last_point",
                }
            }
            None => "last_point",
        };
        let Some(s) = self.sent.get(t) else {
            tracing::debug!(drag = %self.id, label = t, rev, current = ?None::<u64>, judged = ?Picked::None, from, "빈 보고 → 커서 판정 — 그 창 영역이 없다");
            return Verdict::NewWindow;
        };
        let mut judged = regions::pick(&s.regions, p, scroll, self.source_view);
        // 끄는 탭이 활성인 출처의 보고 없는 끝이 화면에 한 번도 안 보인 승계 탭 칸에 붙지 않게(PRD R15). 탭바 · 순서
        // 이동은 어느 탭이 보이는지와 무관하다.
        if matches!(judged, Picked::Seam { .. }) && v != s.regions.view_id {
            judged = Picked::None;
        }
        tracing::debug!(drag = %self.id, label = t, rev, current = s.regions.rev, judged = ?judged, from, "빈 보고 → 커서 판정");
        match judged {
            Picked::None => Verdict::NewWindow,
            Picked::Reorder { .. } if t == self.source_label => dock_at(t, &judged, s),
            Picked::Reorder { .. } => Verdict::Refuse(EndReason::TargetChanged),
            // 사용자 결정 Q21(TRD §11-1b ①) — 창 안으로 옮겨 온 것 자체가 마우스 움직임이라 뗀 순간 커서가 있는
            // 곳이 목적지다. 고르지 않은 안 = 무변경.
            Picked::Seam { .. } | Picked::TabBar { .. } => dock_at(t, &judged, s),
        }
    }
}

// 창 `w` 의 지금 영역 `s` 위에서 고른 자리 `picked` 의 적용 — 이음새 = 보이는 탭 배치 · 창 활성 기대값, 탭바 ·
// 순서 이동 = 탭 순서 기대값.
fn dock_at(w: &str, picked: &Picked, s: &Sent) -> Verdict {
    let tab_bar = |index: u32, mode: Mode| Verdict::Dock {
        to: DockDest::TabBar {
            window: w.to_string(),
            index: usize::try_from(index).ok(),
        },
        expect: Some(s.expect.tabs()),
        mode,
        window: w.to_string(),
    };
    match picked {
        Picked::Seam { anchor, side } => match (s.regions.view_id, s.expect.seam()) {
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
        },
        Picked::TabBar { index, .. } => tab_bar(*index, Mode::TabBar),
        Picked::Reorder { index } => tab_bar(*index, Mode::Reorder),
        Picked::None => Verdict::NewWindow,
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

// 창 뷰포트 점을 화면 물리 px 로.
fn screen_of(r: &ClientRect, p: PointCss) -> PhysPoint {
    PhysPoint {
        x: f64::from(r.origin.0) + p.x * r.scale,
        y: f64::from(r.origin.1) + p.y * r.scale,
    }
}

// 화면 물리 px 점을 그 창 뷰포트 CSS px 로 — `screen_of` 의 역. 못 읽은 배율(0 · 유한하지 않음)은 1 로 셈한다.
fn viewport_point(r: &ClientRect, c: PhysPoint) -> PointCss {
    let scale = if r.scale.is_finite() && r.scale > 0.0 {
        r.scale
    } else {
        1.0
    };
    PointCss {
        x: (c.x - f64::from(r.origin.0)) / scale,
        y: (c.y - f64::from(r.origin.1)) / scale,
    }
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
            scroll: None,
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
            strip: None,
            source_rect: None,
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
                Action::Hide { label, .. } => format!("hide:{label}"),
                Action::Show { label, .. } => format!("show:{label}"),
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
            scroll: None,
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
        let shown = regions::pick(&current(&book, POP2), SEAM_P, None, id(SRC));
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
        let shown = regions::pick(&current(&fresh(), MAIN), SEAM_P, None, id(SRC));
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
        let judged = regions::pick(&current(&fresh(), MAIN), BAR_P, None, id(SRC));
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
        let shown = regions::pick(&current(&book, MAIN), SEAM_P, None, id(SRC));
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
            generation: GEN,
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
            canceled: false,
            last: None,
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
            generation: GEN,
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
                ..
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
        let shown = regions::pick(&current(book, label), SEAM_P, None, id(SRC));
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
    fn a_held_drop_with_no_guide_is_cursor_judged_and_applies_the_guide_at_its_point() {
        // 시작 전의 웹뷰는 영역이 없어 빈 보고를 싣는다 — 그 놓은 점의 지금 판정이 가이드면 그 자리에 붙인다(Q21).
        // 그 창 캔버스는 모델 활성 10 을 보였다(세션을 모르는 웹뷰 — 떼어진 표시 없음).
        let mut book = Book::new();
        armed(&drop_with(
            &mut book,
            "d-1",
            id(SRC),
            MAIN,
            report(0, 0, Some(10), Picked::None, SEAM_P),
            cursor(),
            rects(),
        ));
        let step = start_d(&mut book, "d-1");
        assert_eq!(kinds(&step.actions[..4]), START_KINDS);
        assert_eq!(seam_target(&step.actions[4..]), id(10));
        let log = applied_log(&mut book, MAIN, false);
        assert!(log.early && log.unseen);
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

    // ── 순서 이동 · 커서 판정 · 미룬 숨기기 · 끝 스냅숏 · 붙들기 (5.7) ──────────────────

    const MAIN_RECT: ClientRect = ClientRect {
        origin: (100, 50),
        size: (2400, 1656),
        scale: 2.0,
    };
    const POP2_RECT: ClientRect = ClientRect {
        origin: (3000, 100),
        size: (2400, 1656),
        scale: 2.0,
    };
    const POP_RECT: ClientRect = ClientRect {
        origin: (6000, 100),
        size: (1200, 856),
        scale: 1.0,
    };
    // 순서 이동 구역 안의 점 — 머리 [10, SRC, 11](각 100) · 잡은 자리 50 · 스크롤 0 이면 IN_PLACE_P = 자리 1(제자리) ·
    // RIGHT_P = 자리 2. 스크롤 110 이면 IN_PLACE_P 도 자리 2 다.
    const IN_PLACE_P: PointCss = PointCss { x: 150.0, y: 10.0 };
    const RIGHT_P: PointCss = PointCss { x: 260.0, y: 10.0 };
    // 탭 하나뿐인 팝아웃의 구역 안 — 늘 자리 0.
    const LONE_P: PointCss = PointCss { x: 50.0, y: 10.0 };

    fn on(r: ClientRect, p: PointCss) -> Option<PhysPoint> {
        Some(screen_of(&r, p))
    }

    // 우리 창 어디에도 없는 화면 점.
    fn nowhere() -> Option<PhysPoint> {
        Some(PhysPoint {
            x: 9000.0,
            y: 9000.0,
        })
    }

    fn strip_of(tabs: &[u128], scroll: f64) -> StripStart {
        let c = chrome(tabs);
        StripStart {
            bar: c.tab_bar,
            heads: c.tabs,
            grab_x: 50.0,
            scroll,
        }
    }

    fn rects_of(windows: &[(&str, ClientRect)]) -> Rects {
        Rects {
            windows: windows
                .iter()
                .map(|(label, r)| (label.to_string(), *r))
                .collect(),
            source_scale: Some(1.5),
        }
    }

    fn all_rects() -> Rects {
        rects_of(&[(MAIN, MAIN_RECT), (POP2, POP2_RECT), (POP, POP_RECT)])
    }

    // 숨긴 출처 slot-popup-1 이 빠진 사각형.
    fn rects_hidden() -> Rects {
        rects_of(&[(MAIN, MAIN_RECT), (POP2, POP2_RECT)])
    }

    #[allow(clippy::too_many_arguments)]
    fn start_full(
        book: &mut Book,
        drag: &str,
        generation: u64,
        source: &str,
        single: bool,
        strip: Option<StripStart>,
        source_rect: Option<ClientRect>,
        windows: Vec<WindowSnap>,
    ) -> Step {
        let number = book.last_snap + 1;
        book.step(Input::Start {
            drag_id: drag.into(),
            view_id: id(SRC),
            hotspot: HOTSPOT,
            lease: LEASE,
            generation,
            source_label: source.into(),
            expect_source: slot(50),
            single_tab_popout: single,
            strip,
            source_rect,
            snap: snap(number, Some(slot(50)), windows),
            now: MonoMs(1000),
        })
    }

    fn reorder_main_win(active: u128) -> WindowSnap {
        win(MAIN, &[10, SRC, 11], active, halves(1, 2))
    }

    // 출처 = main(탭 [10, SRC, 11] · 활성 10) · 시작 측정 있음(스크롤 `scroll`) · 대상 = slot-popup-2.
    fn reorder_case(scroll: f64) -> Book {
        let mut book = Book::new();
        start_full(
            &mut book,
            "d-1",
            GEN,
            MAIN,
            false,
            Some(strip_of(&[10, SRC, 11], scroll)),
            Some(MAIN_RECT),
            vec![reorder_main_win(10), pop2_win()],
        );
        book
    }

    // 출처 = 탭 하나뿐인 팝아웃 slot-popup-1 · 시작 측정 있음 — 숨기기를 도킹 모드 증거까지 미룬다.
    fn lone_case() -> (Book, Step) {
        lone_case_with(Some(POP_RECT))
    }

    fn lone_case_with(source_rect: Option<ClientRect>) -> (Book, Step) {
        let mut book = Book::new();
        let step = start_full(
            &mut book,
            "d-1",
            GEN,
            POP,
            true,
            Some(strip_of(&[SRC], 0.0)),
            source_rect,
            popout_windows(),
        );
        (book, step)
    }

    fn scrolled(mut r: Report, scroll: f64) -> Report {
        r.scroll = Some(scroll);
        r
    }

    fn drawn(r: Report, gap: Option<f64>) -> DrawnReport {
        DrawnReport {
            rev: r.rev,
            bar_rev: r.bar_rev,
            displayed: r.displayed,
            shown: r.shown,
            point: r.point,
            scroll: r.scroll,
            left_before_end_ms: gap,
        }
    }

    fn reorder(index: u32) -> Picked {
        Picked::Reorder { index }
    }

    fn src_end_full(
        book: &mut Book,
        effect: DragEffect,
        canceled: bool,
        last: Option<DrawnReport>,
        cursor: Option<PhysPoint>,
        rects: Rects,
        now: u64,
    ) -> Vec<Action> {
        book.step(Input::SourceEnd {
            drag_id: "d-1".into(),
            effect,
            canceled,
            last,
            cursor,
            rects,
            now: MonoMs(now),
        })
        .actions
    }

    fn src_none(
        book: &mut Book,
        last: Option<DrawnReport>,
        cursor: Option<PhysPoint>,
        rects: Rects,
        now: u64,
    ) -> Vec<Action> {
        src_end_full(book, DragEffect::None, false, last, cursor, rects, now)
    }

    fn over_t(book: &mut Book, label: &str, r: Report, now: u64) -> Vec<Action> {
        book.step(Input::Over {
            drag_id: "d-1".into(),
            label: label.into(),
            report: r,
            now: MonoMs(now),
        })
        .actions
    }

    fn left_t(book: &mut Book, label: &str, now: u64) -> Vec<Action> {
        book.step(Input::Left {
            drag_id: "d-1".into(),
            label: label.into(),
            now: MonoMs(now),
        })
        .actions
    }

    fn fire_gen(
        book: &mut Book,
        kind: TimerKind,
        generation: u64,
        number: u64,
        now: u64,
    ) -> Vec<Action> {
        book.step(Input::Timeout {
            kind,
            generation,
            number,
            cursor: nowhere(),
            rects: Rects::default(),
            now: MonoMs(now),
        })
        .actions
    }

    // 걸린 시한 하나 — (종류, 세대, 번호, 시간).
    fn armed_full(actions: &[Action]) -> (TimerKind, u64, u64, u64) {
        match actions {
            [Action::ArmTimer {
                kind,
                generation,
                number,
                after_ms,
            }] => (*kind, *generation, *number, *after_ms),
            other => panic!("시한 하나여야 — {:?}", kinds(other)),
        }
    }

    fn tab_bar_at(actions: &[Action]) -> (String, Option<usize>) {
        match &applied_req(actions).to {
            DockDest::TabBar { window, index } => (window.clone(), *index),
            other => panic!("탭바여야 — {other:?}"),
        }
    }

    fn mode_of(book: &Book) -> Option<Mode> {
        book.drag.as_ref().unwrap().mode
    }

    // 적용이 출처를 닫지 않았다(같은 창 순서 이동 · 다른 탭이 남은 창).
    fn kept(window: &str) -> Result<DockOutcome, DockError> {
        Ok(DockOutcome {
            window: window.into(),
            slot_ids: vec![],
            closed_window: None,
            reused: false,
        })
    }

    fn kept_log(book: &mut Book, window: &str) -> (Vec<String>, EndLog) {
        let out = book.step(Input::Applied(kept(window))).actions;
        (kinds(&out), end_log(&out).clone())
    }

    fn hide_count(actions: &[Action]) -> usize {
        actions
            .iter()
            .filter(|a| matches!(a, Action::Hide { .. }))
            .count()
    }

    // ─ 순서 이동 놓기(4c) ─

    #[test]
    fn a_reorder_drop_applies_when_the_repick_matches_whatever_its_numbers() {
        assert!(current(&reorder_case(0.0), MAIN).reorder.is_some(), "전제");
        // 번호 · 보이던 탭은 보지 않는다 — 순서 이동 영역은 세션 내내 얼린 시작 측정이다.
        for (rev, bar_rev, displayed) in [(1, 1, Some(10)), (7, 9, Some(11)), (2, 0, None)] {
            let mut book = reorder_case(0.0);
            let out = drop_with(
                &mut book,
                "d-1",
                id(SRC),
                MAIN,
                report(rev, bar_rev, displayed, reorder(2), RIGHT_P),
                on(MAIN_RECT, RIGHT_P),
                rects(),
            );
            assert_eq!(
                applied_req(&out),
                &DockRequest {
                    view: id(SRC),
                    to: DockDest::TabBar {
                        window: MAIN.into(),
                        index: Some(2),
                    },
                    guard: DockGuard {
                        lease: Some(LEASE),
                        expect_source: Some(slot(50)),
                        expect_target: Some(TargetExpect::Tabs(ids(&[10, SRC, 11]))),
                    },
                    focus: FocusPolicy::Reveal,
                },
                "{rev} {bar_rev} {displayed:?}"
            );
            assert_eq!(mode_of(&book), Some(Mode::Reorder));
        }
        // 다시 고른 자리가 다르다.
        let mut book = reorder_case(0.0);
        let out = drop_with(
            &mut book,
            "d-1",
            id(SRC),
            MAIN,
            report(1, 1, Some(10), reorder(0), RIGHT_P),
            on(MAIN_RECT, RIGHT_P),
            rects(),
        );
        assert_eq!(end_log(&out).reason, Some(EndReason::TargetChanged));
    }

    #[test]
    fn a_reorder_drop_after_the_zone_left_the_regions_or_on_another_window_changes_nothing() {
        // 끄는 동안 다른 경로가 출처 창에 탭을 더했다 — 얼린 머리가 모델과 갈라져 구역이 빠진다.
        let mut book = reorder_case(0.0);
        book.step(Input::Snapshot(snap(
            2,
            Some(slot(50)),
            vec![win(MAIN, &[10, SRC, 11, 12], 10, halves(1, 2)), pop2_win()],
        )));
        assert_eq!(current(&book, MAIN).reorder, None, "전제");
        let out = drop_with(
            &mut book,
            "d-1",
            id(SRC),
            MAIN,
            report(2, 2, Some(10), reorder(2), RIGHT_P),
            on(MAIN_RECT, RIGHT_P),
            rects(),
        );
        assert_eq!(end_log(&out).reason, Some(EndReason::TargetChanged));

        let mut book = reorder_case(0.0);
        let out = drop_with(
            &mut book,
            "d-1",
            id(SRC),
            POP2,
            report(1, 1, Some(20), reorder(0), LONE_P),
            on(POP2_RECT, LONE_P),
            all_rects(),
        );
        assert_eq!(end_log(&out).reason, Some(EndReason::TargetChanged));
    }

    #[test]
    fn esc_inside_the_zone_lands_where_the_source_last_showed() {
        let mut book = reorder_case(0.0);
        over_at(
            &mut book,
            MAIN,
            report(1, 1, Some(10), reorder(2), RIGHT_P),
            1500,
        );
        // 취소는 현재 대상에 떠남을 먼저 낼 수 있다.
        left_at(&mut book, MAIN, 1990);
        let out = src_none(&mut book, None, on(MAIN_RECT, RIGHT_P), rects(), 2000);
        assert_eq!(tab_bar_at(&out), (MAIN.to_string(), Some(2)));
        let (_, log) = kept_log(&mut book, MAIN);
        assert_eq!(
            (log.end_by, log.mode, log.unseen, log.strip),
            (Some(EndBy::SourceNone), Some(Mode::Reorder), false, true)
        );
    }

    // ─ 빈 보고 · 커서 판정 ─

    #[test]
    fn an_empty_report_is_cursor_judged_at_its_point_with_the_canvas_filter() {
        // 영역 전의 놓기(rev 0) — 그 점의 지금 판정이 이음새면 그 자리에 붙인다(Q21).
        let mut book = fresh();
        let out = drop_on(
            &mut book,
            MAIN,
            report(0, 0, Some(10), Picked::None, SEAM_P),
            cursor(),
        );
        assert_eq!(seam_target(&out), id(10));
        assert!(applied_log(&mut book, MAIN, false).unseen);
        // 보이던 탭이 영역의 보이는 탭이 아니면 캔버스 가이드를 거른다 — 웹뷰 그리기와 같다.
        for displayed in [Some(11), None] {
            let mut book = fresh();
            let out = drop_on(
                &mut book,
                MAIN,
                report(0, 0, displayed, Picked::None, SEAM_P),
                cursor(),
            );
            assert_eq!(new_window_at(&out), Some((485, 391)), "{displayed:?}");
        }
        // 탭바 자리는 거르지 않는다.
        let mut book = fresh();
        let out = drop_on(
            &mut book,
            MAIN,
            report(0, 0, Some(11), Picked::None, BAR_P),
            cursor(),
        );
        assert_eq!(tab_bar_at(&out), (MAIN.to_string(), Some(1)));
        assert_eq!(mode_of(&book), Some(Mode::TabBar));
        // 그 창의 지금 영역이 없다(스냅숏에서 빠짐) — 판정할 영역이 없으니 새 창.
        let mut book = fresh();
        book.step(Input::Snapshot(snap(
            2,
            Some(slot(50)),
            vec![main_win(), pop_win()],
        )));
        let out = drop_on(
            &mut book,
            POP2,
            report(1, 1, Some(20), Picked::None, SEAM_P),
            cursor(),
        );
        assert_eq!(new_window_at(&out), Some((485, 391)));
        assert!(applied_log(&mut book, POP, true).unseen);
    }

    #[test]
    fn a_none_report_on_a_real_region_is_still_a_report() {
        // 0 이 아닌 옛 rev 의 「가이드 없음」은 실제 영역으로 판정해 그린 것이다 — 지금 가이드면 무변경(5.6 그대로).
        let mut book = fresh();
        book.step(Input::Snapshot(snap(
            2,
            Some(slot(50)),
            vec![
                win(MAIN, &[10, 11], 11, halves(1, 2)),
                pop_win(),
                pop2_win(),
            ],
        )));
        assert_eq!(current(&book, MAIN).rev, 2, "전제");
        let out = drop_on(
            &mut book,
            MAIN,
            report(1, 1, Some(10), Picked::None, SEAM_P),
            cursor(),
        );
        noop_end(&out, EndReason::TargetChanged);
        assert!(!end_log(&out).unseen);
    }

    #[test]
    fn with_no_reports_an_end_cursor_inside_the_source_is_judged_on_its_regions() {
        let end =
            |book: &mut Book, p: PointCss| src_none(book, None, on(MAIN_RECT, p), rects(), 2000);
        // 구역 = 순서 이동.
        let mut book = reorder_case(0.0);
        assert_eq!(
            tab_bar_at(&end(&mut book, RIGHT_P)),
            (MAIN.to_string(), Some(2))
        );
        assert_eq!(mode_of(&book), Some(Mode::Reorder));
        let (_, log) = kept_log(&mut book, MAIN);
        assert!(log.unseen && log.over_window.is_none());
        // 칸 가운데 = 새 창 · 이음새 = 적용(Q21).
        let mut book = reorder_case(0.0);
        assert_eq!(
            new_window_at(&end(&mut book, NONE_P)),
            Some((
                (100.0 + NONE_P.x * 2.0 - 15.0) as i32,
                (50.0 + NONE_P.y * 2.0 - 9.0) as i32
            ))
        );
        let mut book = reorder_case(0.0);
        assert_eq!(seam_target(&end(&mut book, SEAM_P)), id(10));
        // 끼울 자리 = 적용 — 순서 이동 측정이 없어 원래 창 탭바가 끼울 자리다.
        let mut book = Book::new();
        start_full(
            &mut book,
            "d-1",
            GEN,
            MAIN,
            false,
            None,
            Some(MAIN_RECT),
            vec![reorder_main_win(10), pop2_win()],
        );
        assert_eq!(
            tab_bar_at(&end(&mut book, BAR_P)),
            (MAIN.to_string(), Some(1))
        );
    }

    #[test]
    fn with_no_reports_an_end_cursor_outside_the_source_opens_a_new_window() {
        // 다른 우리 창 안이어도 — 그 창은 이 끌기를 한 번도 보지 못했다(맨 위인지 모른다).
        for rects in [rects(), all_rects()] {
            let mut book = reorder_case(0.0);
            let out = src_none(&mut book, None, on(POP2_RECT, SEAM_P), rects, 2000);
            assert!(matches!(applied_req(&out).to, DockDest::NewWindow { .. }));
        }
    }

    #[test]
    fn the_canvas_counts_as_showing_the_model_active_without_a_source_report() {
        // 끄는 탭이 활성 — 셸 영역의 보이는 탭은 승계 탭이지만 화면에는 한 번도 안 보였다.
        let start = || {
            let mut book = Book::new();
            start_full(
                &mut book,
                "d-1",
                GEN,
                MAIN,
                false,
                Some(strip_of(&[10, SRC, 11], 0.0)),
                Some(MAIN_RECT),
                vec![reorder_main_win(SRC), pop2_win()],
            );
            book
        };
        let mut book = start();
        let out = src_none(&mut book, None, on(MAIN_RECT, SEAM_P), rects(), 2000);
        assert!(matches!(applied_req(&out).to, DockDest::NewWindow { .. }));
        // 구역은 거르지 않는다.
        let mut book = start();
        let out = src_none(&mut book, None, on(MAIN_RECT, RIGHT_P), rects(), 2000);
        assert_eq!(tab_bar_at(&out), (MAIN.to_string(), Some(2)));
    }

    #[test]
    fn an_empty_last_over_is_cursor_judged_at_the_end_cursor_or_at_its_point() {
        // 아직 위 + 커서 안 = 끝 순간 커서로(그 보고의 점은 칸 가운데).
        let mut book = fresh();
        over_at(
            &mut book,
            MAIN,
            report(0, 0, Some(10), Picked::None, NONE_P),
            1500,
        );
        let out = src_none(&mut book, None, on(MAIN_RECT, SEAM_P), rects(), 2000);
        assert_eq!(seam_target(&out), id(10));
        // 커서를 못 읽음 = 그 보고의 점으로.
        let mut book = fresh();
        over_at(
            &mut book,
            MAIN,
            report(0, 0, Some(10), Picked::None, SEAM_P),
            1500,
        );
        let out = src_none(&mut book, None, None, rects(), 2000);
        assert_eq!(seam_target(&out), id(10));
        // 유실 + 아직 위 = 그 보고의 점으로(커서는 끝 뒤에 움직였다).
        let mut book = fresh();
        over_at(
            &mut book,
            MAIN,
            report(0, 0, Some(10), Picked::None, SEAM_P),
            1500,
        );
        let grace = armed(&pointer_seen(&mut book, "d-1", 1800)).1;
        let out = fire(&mut book, TimerKind::Grace, grace, outside(), rects());
        assert_eq!(seam_target(&out), id(10));
        assert!(applied_log(&mut book, MAIN, false).unseen);
    }

    #[test]
    fn signal_loss_and_a_new_start_do_not_cursor_judge_the_source() {
        let mut book = reorder_case(0.0);
        let grace = armed(&pointer_seen(&mut book, "d-1", 1800)).1;
        let out = fire(
            &mut book,
            TimerKind::Grace,
            grace,
            on(MAIN_RECT, RIGHT_P),
            rects(),
        );
        assert!(matches!(applied_req(&out).to, DockDest::NewWindow { .. }));
        let mut book = reorder_case(0.0);
        let out = superseded(&mut book, "d-1", on(MAIN_RECT, RIGHT_P));
        assert!(matches!(applied_req(&out).to, DockDest::NewWindow { .. }));
    }

    #[test]
    fn the_report_scroll_repicks_the_reorder_slot_and_no_report_uses_the_start_scroll() {
        let mut book = reorder_case(0.0);
        let out = drop_with(
            &mut book,
            "d-1",
            id(SRC),
            MAIN,
            scrolled(report(1, 1, Some(10), reorder(2), IN_PLACE_P), 110.0),
            on(MAIN_RECT, IN_PLACE_P),
            rects(),
        );
        assert_eq!(tab_bar_at(&out), (MAIN.to_string(), Some(2)));
        // 같은 점 · 스크롤 없음 = 시작 스크롤 0 으로 자리 1 — 그린 자리 2 와 달라 무변경.
        let mut book = reorder_case(0.0);
        let out = drop_with(
            &mut book,
            "d-1",
            id(SRC),
            MAIN,
            report(1, 1, Some(10), reorder(2), IN_PLACE_P),
            on(MAIN_RECT, IN_PLACE_P),
            rects(),
        );
        assert_eq!(end_log(&out).reason, Some(EndReason::TargetChanged));
        // 보고 없는 끝 = 시작 스크롤.
        let mut book = reorder_case(110.0);
        let out = src_none(&mut book, None, on(MAIN_RECT, IN_PLACE_P), rects(), 2000);
        assert_eq!(tab_bar_at(&out), (MAIN.to_string(), Some(2)));
    }

    // ─ 미룬 숨기기 ─

    #[test]
    fn a_lone_popout_with_a_reorder_zone_defers_its_hide() {
        let (book, step) = lone_case();
        assert_eq!(
            kinds(&step.actions),
            vec![
                "started#1",
                "regions:main#1/1",
                "regions:slot-popup-1#1/1",
                "regions:slot-popup-2#1/1"
            ]
        );
        assert!(current(&book, POP).reorder.is_some());
        assert_eq!(book.visibility(POP), Visibility::Shown);
        // 순서 이동이 서지 않으면 시작에 숨긴다(5.6 그대로).
        let mut book = Book::new();
        let step = start_full(
            &mut book,
            "d-1",
            GEN,
            POP,
            true,
            Some(strip_of(&[SRC, 30], 0.0)),
            Some(POP_RECT),
            popout_windows(),
        );
        assert_eq!(kinds(&step.actions[..1]), vec!["hide:slot-popup-1"]);
        assert_eq!(book.visibility(POP), Visibility::Hidden);
    }

    #[test]
    fn the_first_docking_mode_evidence_hides_once_and_keeps_the_source_regions() {
        // ① 출처의 그린 보고(지금 rev · 막히지 않음)가 순서 이동이 아니다.
        let (mut book, _) = lone_case();
        assert!(over_t(&mut book, POP, report(1, 1, None, reorder(0), LONE_P), 1400).is_empty());
        let before = current(&book, POP);
        let out = over_t(
            &mut book,
            POP,
            report(1, 1, None, Picked::None, NONE_P),
            1450,
        );
        assert_eq!(
            out,
            vec![Action::Hide {
                label: POP.into(),
                generation: GEN,
            }]
        );
        assert_eq!(book.visibility(POP), Visibility::Hidden);
        assert_eq!(
            hide_count(&over_t(
                &mut book,
                POP2,
                report(0, 0, None, Picked::None, NONE_P),
                1460
            )),
            0,
            "둘째 증거"
        );
        // 숨긴 뒤의 스냅숏은 그 창 몫을 건너뛴다 — 영역을 보내지도 지우지도 고치지도 않는다.
        let mut changed = pop_win();
        changed.canvas = Some(CanvasPx { w: 900, h: 600 });
        let step = book.step(Input::Snapshot(snap(
            2,
            Some(slot(50)),
            vec![main_win(), changed, pop2_win()],
        )));
        assert!(step.actions.is_empty(), "{:?}", kinds(&step.actions));
        assert_eq!(current(&book, POP), before);
        assert_eq!(
            book.step(Input::Pull { label: POP.into() }).reply,
            Reply::Pulled(Some(PullReply {
                started: book.drag.as_ref().unwrap().started(),
                regions: None,
            }))
        );

        // ③ 다른 창의 보고 — 빈 보고여도.
        let (mut book, _) = lone_case();
        assert_eq!(
            hide_count(&over_t(
                &mut book,
                POP2,
                report(0, 0, None, Picked::None, NONE_P),
                1400
            )),
            1
        );
    }

    #[test]
    fn source_reports_the_webview_could_not_judge_are_no_evidence() {
        // 영역 전 — 지금 영역을 다시 보낼 뿐이다.
        let (mut book, _) = lone_case();
        let out = over_t(
            &mut book,
            POP,
            report(0, 0, None, Picked::None, LONE_P),
            1400,
        );
        assert_eq!(kinds(&out), vec!["regions:slot-popup-1#1/1"]);
        // 옛 rev — 지금 영역을 다시 보낼 뿐이다.
        let mut changed = pop_win();
        changed.canvas = Some(CanvasPx { w: 900, h: 600 });
        book.step(Input::Snapshot(snap(
            2,
            Some(slot(50)),
            vec![main_win(), changed, pop2_win()],
        )));
        let out = over_t(
            &mut book,
            POP,
            report(1, 1, None, Picked::None, NONE_P),
            1450,
        );
        assert_eq!(kinds(&out), vec!["regions:slot-popup-1#2/1"]);
        // 막힌 영역.
        book.step(Input::Snapshot(snap(
            3,
            Some(slot(51)),
            vec![main_win(), pop_win(), pop2_win()],
        )));
        let blocked = current(&book, POP);
        assert!(blocked.blocked, "전제");
        let out = over_t(
            &mut book,
            POP,
            report(blocked.rev, blocked.bar_rev, None, Picked::None, NONE_P),
            1500,
        );
        assert_eq!(hide_count(&out), 0);
        assert_eq!(book.visibility(POP), Visibility::Shown);
    }

    #[test]
    fn a_source_report_without_a_current_source_region_is_no_evidence() {
        // 스냅숏에서 출처 창이 빠졌다 — 그 창의 지금 영역이 없으니 웹뷰가 판정한 것이 아니다.
        let (mut book, _) = lone_case();
        book.step(Input::Snapshot(snap(
            2,
            Some(slot(50)),
            vec![main_win(), pop2_win()],
        )));
        let out = over_t(
            &mut book,
            POP,
            report(1, 1, None, Picked::None, NONE_P),
            1400,
        );
        assert_eq!(hide_count(&out), 0);
        assert_eq!(book.visibility(POP), Visibility::Shown);
    }

    #[test]
    fn a_source_left_arms_a_leave_timer_that_hides_only_while_live() {
        let (mut book, _) = lone_case();
        let (kind, generation, n, after) = armed_full(&left_t(&mut book, POP, 1500));
        assert_eq!(
            (kind, generation, after),
            (TimerKind::Leave, GEN, LEAVE_GRACE_MS)
        );
        assert_eq!(
            fire_gen(&mut book, TimerKind::Leave, GEN, n, 1600),
            vec![Action::Hide {
                label: POP.into(),
                generation: GEN,
            }]
        );

        // 그 전에 출처 Over 가 다시 오면 그 번호를 버린다.
        let (mut book, _) = lone_case();
        let n = armed_full(&left_t(&mut book, POP, 1500)).2;
        assert!(over_t(&mut book, POP, report(1, 1, None, reorder(0), LONE_P), 1550).is_empty());
        assert!(fire_gen(&mut book, TimerKind::Leave, GEN, n, 1600).is_empty());

        // `Live` 가 아니면(끝 신호 · 포인터 이동이 먼저) 버린다.
        let (mut book, _) = lone_case();
        let n = armed_full(&left_t(&mut book, POP, 1500)).2;
        armed(&pointer_seen(&mut book, "d-1", 1550));
        assert!(fire_gen(&mut book, TimerKind::Leave, GEN, n, 1600).is_empty());

        // 적용 중에는 증거도 떠남 시한도 없다.
        let (mut book, _) = lone_case();
        let r = seam_report(&book, POP2, 20);
        drop_with(&mut book, "d-1", id(SRC), POP2, r, cursor(), all_rects());
        assert!(is_applying(&book));
        let out = over_t(
            &mut book,
            POP2,
            report(0, 0, None, Picked::None, NONE_P),
            1400,
        );
        assert_eq!(hide_count(&out), 0);
        assert!(left_t(&mut book, POP, 1500).is_empty());
    }

    #[test]
    fn esc_in_the_zone_beats_the_leave_timer_and_lands_without_ever_hiding() {
        let (mut book, _) = lone_case();
        over_at(&mut book, POP, report(1, 1, None, reorder(0), LONE_P), 1400);
        let n = armed_full(&left_t(&mut book, POP, 1500)).2;
        let out = src_none(&mut book, None, on(POP_RECT, LONE_P), all_rects(), 1560);
        assert_eq!(tab_bar_at(&out), (POP.to_string(), Some(0)));
        assert!(fire_gen(&mut book, TimerKind::Leave, GEN, n, 1600).is_empty());
        // 숨긴 적이 없으니 되보이기도 없다.
        let (kinds, log) = kept_log(&mut book, POP);
        assert_eq!(
            kinds,
            vec!["ended#2", "focus:slot-popup-1", "release:7", "endlog"]
        );
        assert_eq!((log.hide_deferred, log.hidden), (true, false));
    }

    // ─ 엔진 취소 ─

    #[test]
    fn an_engine_canceled_end_changes_nothing() {
        let mut book = fresh();
        let out = src_end_full(
            &mut book,
            DragEffect::None,
            true,
            None,
            cursor(),
            rects(),
            1100,
        );
        noop_end(&out, EndReason::EngineCanceled);
        assert_eq!(end_log(&out).end_by, Some(EndBy::EngineCancel));
        assert_eq!(EndReason::EngineCanceled.word(), "EngineCanceled");
        assert_eq!(EndBy::EngineCancel.word(), "engine_cancel");

        // 붙든 이른 신호로 와도 같다.
        let mut book = Book::new();
        armed(
            &book
                .step(Input::SourceEnd {
                    drag_id: "d-1".into(),
                    effect: DragEffect::None,
                    canceled: true,
                    last: None,
                    cursor: cursor(),
                    rects: rects(),
                    now: MonoMs(900),
                })
                .actions,
        );
        let step = start_d(&mut book, "d-1");
        noop_end(&step.actions[4..], EndReason::EngineCanceled);
        assert!(end_log(&step.actions).early);

        // 받은 창이 있는 끝은 가르지 않는다.
        let mut book = fresh();
        let out = src_end_full(
            &mut book,
            DragEffect::Move,
            true,
            None,
            cursor(),
            rects(),
            1100,
        );
        assert_eq!(armed(&out).0, TimerKind::Drop);
    }

    // ─ 끝 스냅숏 ─

    #[test]
    fn the_end_snapshot_stands_in_for_the_sources_last_over_when_w_is_the_source() {
        // 가장자리 스크롤 중 늦은 Over + Esc — 그린 자리 2 가 그 Over 보다 먼저 닿았다.
        let mut book = reorder_case(0.0);
        over_at(
            &mut book,
            MAIN,
            report(1, 1, Some(10), reorder(1), IN_PLACE_P),
            1500,
        );
        let last = drawn(
            scrolled(report(1, 1, Some(10), reorder(2), IN_PLACE_P), 110.0),
            None,
        );
        let out = src_none(
            &mut book,
            Some(last),
            on(MAIN_RECT, IN_PLACE_P),
            rects(),
            2000,
        );
        assert_eq!(tab_bar_at(&out), (MAIN.to_string(), Some(2)));
        let (_, log) = kept_log(&mut book, MAIN);
        assert_eq!(
            (log.last, log.tight, log.last_differs, log.lifted),
            (LastUse::Used, true, true, false)
        );
        // 뒤늦은 그 Over 는 끝난 세션의 늦은 신호다.
        assert!(over_t(
            &mut book,
            MAIN,
            report(1, 1, Some(10), reorder(2), IN_PLACE_P),
            2100
        )
        .is_empty());

        // 스냅숏이 없으면 받은 마지막 Over 그대로.
        let mut book = reorder_case(0.0);
        over_at(
            &mut book,
            MAIN,
            report(1, 1, Some(10), reorder(1), IN_PLACE_P),
            1500,
        );
        let out = src_none(&mut book, None, on(MAIN_RECT, IN_PLACE_P), rects(), 2000);
        assert_eq!(tab_bar_at(&out), (MAIN.to_string(), Some(1)));
        assert_eq!(kept_log(&mut book, MAIN).1.last, LastUse::None);
    }

    #[test]
    fn the_end_snapshot_never_chooses_w() {
        // 출처 → slot-popup-2(가이드) → 출처 떠남 100 ms 안 Esc — W = slot-popup-2 가 본 그 가이드.
        let mut book = reorder_case(0.0);
        over_at(
            &mut book,
            MAIN,
            report(1, 1, Some(10), reorder(1), IN_PLACE_P),
            1400,
        );
        left_at(&mut book, MAIN, 1450);
        let r = seam_report(&book, POP2, 20);
        over_at(&mut book, POP2, r, 1500);
        left_at(&mut book, POP2, 1960);
        let last = drawn(report(1, 1, Some(10), reorder(2), IN_PLACE_P), Some(60.0));
        let out = src_none(
            &mut book,
            Some(last.clone()),
            on(POP2_RECT, SEAM_P),
            all_rects(),
            2000,
        );
        assert_eq!(seam_target(&out), id(20));
        assert_eq!(applied_log(&mut book, POP2, false).last, LastUse::NotW);

        // slot-popup-2 에서 출처로 돌아왔는데 그 Over 가 아직 오는 중 — W = slot-popup-2 · 커서가 그 밖 = 새 창.
        let mut book = reorder_case(0.0);
        over_at(
            &mut book,
            MAIN,
            report(1, 1, Some(10), reorder(1), IN_PLACE_P),
            1400,
        );
        left_at(&mut book, MAIN, 1450);
        let r = seam_report(&book, POP2, 20);
        over_at(&mut book, POP2, r, 1500);
        left_at(&mut book, POP2, 1600);
        let out = src_none(
            &mut book,
            Some(drawn(report(1, 1, Some(10), reorder(2), IN_PLACE_P), None)),
            on(MAIN_RECT, IN_PLACE_P),
            all_rects(),
            1650,
        );
        assert!(matches!(applied_req(&out).to, DockDest::NewWindow { .. }));
    }

    #[test]
    fn an_end_snapshot_that_really_left_is_rect_checked_and_a_tight_one_is_not() {
        let end = |gap: Option<f64>| {
            let mut book = reorder_case(0.0);
            over_at(
                &mut book,
                MAIN,
                report(1, 1, Some(10), reorder(1), IN_PLACE_P),
                1500,
            );
            let last = drawn(report(1, 1, Some(10), reorder(2), RIGHT_P), gap);
            let out = src_none(&mut book, Some(last), nowhere(), all_rects(), 2000);
            (book, out)
        };
        let (_, out) = end(Some(40.0));
        assert!(matches!(applied_req(&out).to, DockDest::NewWindow { .. }));
        for gap in [Some(5.0), Some(CANCEL_LEAVE_MS), None] {
            let (mut book, out) = end(gap);
            assert_eq!(tab_bar_at(&out), (MAIN.to_string(), Some(2)), "{gap:?}");
            let log = kept_log(&mut book, MAIN).1;
            assert_eq!((log.tight, log.last_gap_ms), (true, gap));
        }
    }

    #[test]
    fn the_sources_own_clock_keeps_it_still_over() {
        // 셸이 받은 출처 떠남은 끝보다 500 ms 앞섰지만 출처 시계로는 그 떠남이 엔진 접기다.
        let end = |last: Option<DrawnReport>| {
            let mut book = reorder_case(0.0);
            over_at(
                &mut book,
                MAIN,
                report(1, 1, Some(10), reorder(1), IN_PLACE_P),
                1500,
            );
            left_at(&mut book, MAIN, 1500);
            src_none(&mut book, last, on(MAIN_RECT, RIGHT_P), rects(), 2000)
        };
        let last = drawn(report(1, 1, Some(10), reorder(2), RIGHT_P), Some(5.0));
        assert_eq!(tab_bar_at(&end(Some(last))), (MAIN.to_string(), Some(2)));
        assert!(matches!(
            applied_req(&end(None)).to,
            DockDest::NewWindow { .. }
        ));
    }

    #[test]
    fn an_empty_end_snapshot_is_cursor_judged_at_its_own_point() {
        let mut book = reorder_case(0.0);
        over_at(
            &mut book,
            MAIN,
            report(1, 1, Some(10), reorder(1), IN_PLACE_P),
            1500,
        );
        let last = drawn(report(0, 0, Some(10), Picked::None, RIGHT_P), None);
        let out = src_none(
            &mut book,
            Some(last),
            on(MAIN_RECT, IN_PLACE_P),
            rects(),
            2000,
        );
        assert_eq!(tab_bar_at(&out), (MAIN.to_string(), Some(2)));
        assert!(kept_log(&mut book, MAIN).1.unseen);
    }

    #[test]
    fn an_evidence_hidden_sources_end_snapshot_is_dropped() {
        let (mut book, _) = lone_case();
        over_at(
            &mut book,
            POP,
            report(1, 1, None, Picked::None, NONE_P),
            1400,
        );
        assert_eq!(book.visibility(POP), Visibility::Hidden, "전제 — 증거 ①");
        let r = seam_report(&book, MAIN, 10);
        over_at(&mut book, MAIN, r, 1500);
        let last = drawn(report(1, 1, None, reorder(0), LONE_P), None);
        let out = src_none(
            &mut book,
            Some(last.clone()),
            cursor(),
            rects_hidden(),
            2000,
        );
        assert_eq!(seam_target(&out), id(10));
        assert_eq!(applied_log(&mut book, MAIN, false).last, LastUse::Dropped);

        // W 없음 — 끝 스냅숏으로 출처를 세우지 않는다(그 웹뷰는 떠남을 받지 못해 「안」으로 남는다).
        let (mut book, _) = lone_case();
        over_at(
            &mut book,
            POP,
            report(1, 1, None, Picked::None, NONE_P),
            1400,
        );
        let out = src_none(
            &mut book,
            Some(last),
            on(POP_RECT, LONE_P),
            rects_hidden(),
            1410,
        );
        assert!(matches!(applied_req(&out).to, DockDest::NewWindow { .. }));
        let log = book
            .step(Input::Applied(ok_outcome(POP, true)))
            .actions
            .into_iter()
            .find_map(|a| match a {
                Action::EndLog(l) => Some(l),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            (log.last, log.lifted, log.late_hide),
            (LastUse::Dropped, false, false)
        );
    }

    #[test]
    fn a_drop_wait_timeout_still_uses_the_end_snapshot() {
        let mut book = reorder_case(0.0);
        let last = drawn(
            scrolled(report(1, 1, Some(10), reorder(2), IN_PLACE_P), 110.0),
            None,
        );
        let n = armed(&src_end_full(
            &mut book,
            DragEffect::Move,
            false,
            Some(last),
            on(MAIN_RECT, IN_PLACE_P),
            rects(),
            2000,
        ))
        .1;
        let out = fire(&mut book, TimerKind::Drop, n, nowhere(), rects());
        assert_eq!(tab_bar_at(&out), (MAIN.to_string(), Some(2)));
        assert_eq!(kept_log(&mut book, MAIN).1.end_by, Some(EndBy::DropWait));
    }

    // ─ W 없음 + 끝 스냅숏 ─

    #[test]
    fn with_no_w_a_tight_end_snapshot_lifts_the_source_to_w() {
        // 출처 첫 그린 Over 가 아직 안 닿았다 — 시작 스크롤 0 으로 다시 고르면 자리 1 이다.
        let last = drawn(
            scrolled(report(1, 1, Some(10), reorder(2), IN_PLACE_P), 110.0),
            None,
        );
        for (gap, at) in [(None, on(MAIN_RECT, IN_PLACE_P)), (Some(10.0), nowhere())] {
            let mut book = reorder_case(0.0);
            let last = DrawnReport {
                left_before_end_ms: gap,
                ..last.clone()
            };
            let out = src_none(&mut book, Some(last), at, rects(), 2000);
            assert_eq!(tab_bar_at(&out), (MAIN.to_string(), Some(2)), "{gap:?}");
            let log = kept_log(&mut book, MAIN).1;
            assert_eq!(
                (log.lifted, log.last, log.over_window.as_deref()),
                (true, LastUse::Used, Some(MAIN))
            );
        }
        // 빈 스냅숏 = 그 점 · 그 스크롤로 출처 커서 판정.
        let mut book = reorder_case(0.0);
        let empty = drawn(
            scrolled(report(0, 0, Some(10), Picked::None, IN_PLACE_P), 110.0),
            None,
        );
        let out = src_none(&mut book, Some(empty), nowhere(), rects(), 2000);
        assert_eq!(tab_bar_at(&out), (MAIN.to_string(), Some(2)));
    }

    #[test]
    fn with_no_w_an_end_snapshot_that_really_left_waits_then_takes_the_no_w_branch() {
        let mut book = reorder_case(0.0);
        let last = drawn(report(1, 1, Some(10), reorder(2), RIGHT_P), Some(40.0));
        let (kind, _, n, _) = armed_full(&src_none(
            &mut book,
            Some(last),
            on(POP2_RECT, SEAM_P),
            all_rects(),
            2000,
        ));
        assert_eq!(kind, TimerKind::OverWait);
        let out = fire_gen(&mut book, TimerKind::OverWait, GEN, n, 2050);
        assert!(matches!(applied_req(&out).to, DockDest::NewWindow { .. }));
        let log = applied_log(&mut book, POP, true);
        assert_eq!(
            (log.last, log.lifted, log.over_wait, log.over_wait_ms),
            (LastUse::NotW, false, OverWaitEnd::Timeout, Some(50))
        );
    }

    // ─ 오는 중 Over 붙들기 ─

    // 출처 slot-popup-1 이 순서 이동 보고 → 떠남 → 끝(떠남 간격 60 · 끝 커서는 slot-popup-2 의 이음새) — 붙든다.
    fn held(rects: Rects) -> (Book, u64) {
        let (mut book, _) = lone_case();
        over_at(&mut book, POP, report(1, 1, None, reorder(0), LONE_P), 1400);
        armed_full(&left_t(&mut book, POP, 1450));
        let last = drawn(report(1, 1, None, reorder(0), LONE_P), Some(60.0));
        let (kind, generation, n, after) = armed_full(&src_none(
            &mut book,
            Some(last),
            on(POP2_RECT, SEAM_P),
            rects,
            1500,
        ));
        assert_eq!(
            (kind, generation, after),
            (TimerKind::OverWait, GEN, OVER_WAIT_MS)
        );
        (book, n)
    }

    fn held_log(book: &mut Book, window: &str) -> EndLog {
        applied_log(book, window, false)
    }

    #[test]
    fn a_held_end_lands_on_the_guide_whose_over_was_still_coming() {
        let (mut book, n) = held(all_rects());
        assert!(matches!(
            book.drag.as_ref().unwrap().phase,
            Phase::AwaitOver { .. }
        ));
        let r = seam_report(&book, POP2, 20);
        // 붙든 동안의 다른 창 Over 는 숨기기 증거가 아니다.
        assert!(over_t(&mut book, POP2, r, 1520).is_empty());
        assert_eq!(book.visibility(POP), Visibility::Shown);
        let out = left_t(&mut book, POP2, 1522);
        assert_eq!(seam_target(&out), id(20));
        // 풀린 뒤의 시한 · 떠남 시한은 버린다.
        assert!(fire_gen(&mut book, TimerKind::OverWait, GEN, n, 1550).is_empty());
        let log = held_log(&mut book, POP2);
        assert_eq!(
            (
                log.end_by,
                log.last,
                log.over_wait,
                log.over_wait_ms,
                log.over_from.as_deref()
            ),
            (
                Some(EndBy::SourceNone),
                LastUse::NotW,
                OverWaitEnd::Hit,
                Some(22),
                Some(POP2)
            )
        );

        // 떠남이 안 오면 시한에서 — 시한 콜백의 커서 · 사각형은 쓰지 않는다.
        let (mut book, n) = held(all_rects());
        let r = seam_report(&book, POP2, 20);
        over_at(&mut book, POP2, r, 1520);
        let out = fire_gen(&mut book, TimerKind::OverWait, GEN, n, 1550);
        assert_eq!(seam_target(&out), id(20));
        assert_eq!(held_log(&mut book, POP2).over_wait, OverWaitEnd::Timeout);
    }

    #[test]
    fn a_held_end_waits_for_the_last_over_of_the_window_under_the_cursor() {
        // 「가이드 없음」 → 가이드 — 첫 Over 에서 풀지 않는다.
        let (mut book, _) = held(all_rects());
        over_at(
            &mut book,
            POP2,
            report(1, 1, Some(20), Picked::None, NONE_P),
            1510,
        );
        let r = seam_report(&book, POP2, 20);
        over_at(&mut book, POP2, r, 1515);
        // 붙든 동안 닿은 늦은 출처 Over 는 W 를 출처로 되돌리지 않는다.
        over_at(&mut book, POP, report(1, 1, None, reorder(0), LONE_P), 1518);
        assert_eq!(seam_target(&left_t(&mut book, POP2, 1520)), id(20));

        // 출처 → main → slot-popup-2 — main 의 늦은 보고가 먼저 닿아도 풀지 않는다(끝 커서는 slot-popup-2 안).
        let (mut book, _) = held(all_rects());
        let r = seam_report(&book, MAIN, 10);
        over_at(&mut book, MAIN, r, 1510);
        assert!(left_t(&mut book, MAIN, 1512).is_empty());
        let r = seam_report(&book, POP2, 20);
        over_at(&mut book, POP2, r, 1530);
        assert_eq!(seam_target(&left_t(&mut book, POP2, 1532)), id(20));

        // slot-popup-2 가 끝내 안 오면 시한에서 그대로 — W = main(도착 순번) · 커서가 그 밖 = 새 창.
        let (mut book, n) = held(all_rects());
        let r = seam_report(&book, MAIN, 10);
        over_at(&mut book, MAIN, r, 1510);
        left_at(&mut book, MAIN, 1512);
        let out = fire_gen(&mut book, TimerKind::OverWait, GEN, n, 1550);
        assert!(matches!(applied_req(&out).to, DockDest::NewWindow { .. }));
    }

    #[test]
    fn overlapping_windows_under_the_cursor_wait_for_the_timer_and_take_arrival_order() {
        // main 과 slot-popup-2 가 같은 자리 — 어느 쪽이 앞인지 셸은 모른다.
        let overlap = rects_of(&[(MAIN, MAIN_RECT), (POP2, MAIN_RECT), (POP, POP_RECT)]);
        let (mut book, n) = {
            let (mut book, _) = lone_case();
            over_at(&mut book, POP, report(1, 1, None, reorder(0), LONE_P), 1400);
            left_at(&mut book, POP, 1450);
            let last = drawn(report(1, 1, None, reorder(0), LONE_P), Some(60.0));
            let n = armed_full(&src_none(
                &mut book,
                Some(last),
                on(MAIN_RECT, SEAM_P),
                overlap,
                1500,
            ))
            .2;
            (book, n)
        };
        let r = seam_report(&book, MAIN, 10);
        over_at(&mut book, MAIN, r, 1510);
        assert!(left_t(&mut book, MAIN, 1512).is_empty());
        let r = seam_report(&book, POP2, 20);
        over_at(&mut book, POP2, r, 1530);
        assert!(left_t(&mut book, POP2, 1532).is_empty());
        let out = fire_gen(&mut book, TimerKind::OverWait, GEN, n, 1550);
        assert_eq!(seam_target(&out), id(20));
    }

    #[test]
    fn a_held_end_with_no_over_matches_judging_without_the_hold() {
        let (mut book, n) = held(all_rects());
        let out = fire_gen(&mut book, TimerKind::OverWait, GEN, n, 1550);
        assert!(matches!(applied_req(&out).to, DockDest::NewWindow { .. }));
        let log = applied_log(&mut book, POP, true);
        assert_eq!((log.over_wait, log.over_from), (OverWaitEnd::Timeout, None));
    }

    #[test]
    fn ends_that_do_not_hold() {
        let not_held = |out: &[Action]| {
            !out.iter()
                .any(|a| matches!(a, Action::ArmTimer { kind, .. } if *kind == TimerKind::OverWait))
        };
        let src = || {
            let (mut book, _) = lone_case();
            over_at(&mut book, POP, report(1, 1, None, reorder(0), LONE_P), 1400);
            left_at(&mut book, POP, 1450);
            book
        };
        let last = |gap| Some(drawn(report(1, 1, None, reorder(0), LONE_P), gap));
        // 간격이 엔진 접기 · 떠남 없음 · 스냅숏 없음.
        for l in [last(Some(10.0)), last(None), None] {
            let out = src_none(&mut src(), l, on(POP2_RECT, SEAM_P), all_rects(), 1500);
            assert!(not_held(&out) && !out.is_empty());
        }
        // 도착 순번 W 가 다른 창.
        let mut book = src();
        let r = seam_report(&book, POP2, 20);
        over_at(&mut book, POP2, r, 1480);
        let out = src_none(
            &mut book,
            last(Some(60.0)),
            on(POP2_RECT, SEAM_P),
            all_rects(),
            1500,
        );
        assert_eq!(seam_target(&out), id(20));
        // 끝 커서가 우리 창 어디에도 없다.
        let out = src_none(&mut src(), last(Some(60.0)), nowhere(), all_rects(), 1500);
        assert!(not_held(&out) && !out.is_empty());
        // 막힘 — 판정 없이 무변경.
        let mut book = src();
        book.step(Input::Snapshot(snap(2, Some(slot(51)), popout_windows())));
        let out = src_none(
            &mut book,
            last(Some(60.0)),
            on(POP2_RECT, SEAM_P),
            all_rects(),
            1500,
        );
        assert_eq!(end_log(&out).reason, Some(EndReason::SourceChanged));
        // 놓기 기다리기 시한.
        let mut book = src();
        let n = armed(&src_end_full(
            &mut book,
            DragEffect::Move,
            false,
            last(Some(60.0)),
            on(POP2_RECT, SEAM_P),
            all_rects(),
            1500,
        ))
        .1;
        let out = fire(&mut book, TimerKind::Drop, n, nowhere(), all_rects());
        assert!(not_held(&out) && !out.is_empty());
    }

    #[test]
    fn while_held_late_signals_are_dropped_and_the_session_stays_alive() {
        let (mut book, n) = held(all_rects());
        assert_eq!(book.live_drag_id(), Some("d-1"));
        assert!(!book.ended.contains(&"d-1".to_string()));
        let r = seam_report(&book, POP2, 20);
        assert!(drop_with(&mut book, "d-1", id(SRC), POP2, r, cursor(), all_rects()).is_empty());
        assert!(src_none(&mut book, None, cursor(), all_rects(), 1510).is_empty());
        assert!(pointer_seen(&mut book, "d-1", 1515).is_empty());
        // 지난 세대 · 지난 번호의 시한은 버린다.
        assert!(fire_gen(&mut book, TimerKind::OverWait, GEN + 1, n, 1520).is_empty());
        assert!(fire_gen(&mut book, TimerKind::OverWait, GEN, n + 100, 1520).is_empty());
        assert!(matches!(
            book.drag.as_ref().unwrap().phase,
            Phase::AwaitOver { .. }
        ));
        // 스냅숏은 평소대로 — 막힘이 서면 풀 때 무변경이다.
        book.step(Input::Snapshot(snap(2, Some(slot(51)), popout_windows())));
        let out = fire_gen(&mut book, TimerKind::OverWait, GEN, n, 1550);
        assert_eq!(end_log(&out).reason, Some(EndReason::SourceChanged));

        // 출처 소멸 = 그냥 끝.
        let (mut book, _) = held(all_rects());
        let out = book.step(Input::Destroyed { label: POP.into() }).actions;
        assert_eq!(end_log(&out).reason, Some(EndReason::SourceDestroyed));
    }

    #[test]
    fn a_new_start_cuts_the_hold_and_lands_the_held_end() {
        let (mut book, n) = held(all_rects());
        let r = seam_report(&book, POP2, 20);
        over_at(&mut book, POP2, r, 1520);
        let step = book.step(start_input(
            "d-2",
            MAIN,
            false,
            LeaseId(8),
            snap(5, Some(slot(50)), popout_windows()),
        ));
        assert_eq!(
            step.reply,
            Reply::Rejected(Rejection::AlreadyDragging { applying: false })
        );
        // 새 누름 뒤의 커서는 쓰지 않는다 — 붙든 끝 커서로, 붙든 동안 닿은 그 창에.
        let out = superseded(&mut book, "d-1", nowhere());
        assert_eq!(seam_target(&out), id(20));
        assert!(fire_gen(&mut book, TimerKind::OverWait, GEN, n, 1560).is_empty());
        assert_eq!(held_log(&mut book, POP2).over_wait, OverWaitEnd::Cut);
        assert_eq!(book.admits("d-2"), Ok(()));
    }

    // ─ 세션 번호 ─

    #[test]
    fn an_ended_sessions_timers_are_dropped_in_the_next_session() {
        let (mut book, _) = lone_case();
        let (_, generation, n, _) = armed_full(&left_t(&mut book, POP, 1500));
        let r = seam_report(&book, POP2, 20);
        drop_with(&mut book, "d-1", id(SRC), POP2, r, cursor(), all_rects());
        book.step(Input::Applied(ok_outcome(POP2, false)));
        let step = start_full(
            &mut book,
            "d-2",
            GEN + 1,
            POP,
            true,
            Some(strip_of(&[SRC], 0.0)),
            Some(POP_RECT),
            popout_windows(),
        );
        assert_eq!(step.reply, Reply::Nothing);
        assert!(fire_gen(&mut book, TimerKind::Leave, generation, n, 1600).is_empty());
        assert_eq!(book.visibility(POP), Visibility::Shown);

        // 세대가 다르면 번호가 맞아도 종류 무관 버린다.
        let mut book = fresh();
        let n = armed(&src_end(
            &mut book,
            DragEffect::Move,
            cursor(),
            rects(),
            2000,
        ))
        .1;
        assert!(fire_gen(&mut book, TimerKind::Drop, GEN + 1, n, 2600).is_empty());
        assert!(phase_is_await_drop(&book));
    }

    // ─ 시한이 앞지른 끝 ─

    // 출처 순서 이동 보고 → 떠남(1500) → 떠남 시한이 끝보다 먼저 먹혀 숨김.
    fn hidden_by_leave(source_rect: Option<ClientRect>) -> Book {
        let (mut book, _) = lone_case_with(source_rect);
        over_at(&mut book, POP, report(1, 1, None, reorder(0), LONE_P), 1400);
        let n = armed_full(&left_t(&mut book, POP, 1500)).2;
        assert_eq!(
            hide_count(&fire_gen(&mut book, TimerKind::Leave, GEN, n, 1600)),
            1
        );
        book
    }

    #[test]
    fn an_end_that_reached_before_the_leave_timer_counts_the_source_as_not_hidden() {
        let mut book = hidden_by_leave(Some(POP_RECT));
        let out = src_none(&mut book, None, on(POP_RECT, LONE_P), rects_hidden(), 1595);
        assert_eq!(tab_bar_at(&out), (POP.to_string(), Some(0)));
        // 출처가 제자리에 남으니 끝에서 되보인다.
        let (kinds, log) = kept_log(&mut book, POP);
        assert_eq!(
            kinds,
            vec![
                "ended#2",
                "show:slot-popup-1",
                "focus:slot-popup-1",
                "release:7",
                "endlog"
            ]
        );
        assert_eq!(
            (log.late_hide, log.held_rect, log.hidden, log.hide_deferred),
            (true, true, true, true)
        );
        assert_eq!(book.visibility(POP), Visibility::Shown);

        // 바탕화면에 빠르게 던졌다 — 쥔 사각형 밖 = 새 창(재사용). 시한이 끝보다 늦게 돌아도 같다.
        let mut book = hidden_by_leave(Some(POP_RECT));
        let out = src_none(&mut book, None, nowhere(), rects_hidden(), 1595);
        assert!(matches!(applied_req(&out).to, DockDest::NewWindow { .. }));
        let (mut book, _) = lone_case();
        over_at(&mut book, POP, report(1, 1, None, reorder(0), LONE_P), 1400);
        left_at(&mut book, POP, 1500);
        let out = src_none(&mut book, None, nowhere(), all_rects(), 1595);
        assert!(matches!(applied_req(&out).to, DockDest::NewWindow { .. }));
    }

    #[test]
    fn a_late_hide_with_a_tight_snapshot_or_no_held_rect() {
        // 엔진 접기 간격의 스냅숏 = 사각형 검사 없이 그 자리.
        let mut book = hidden_by_leave(Some(POP_RECT));
        let last = drawn(report(1, 1, None, reorder(0), LONE_P), Some(5.0));
        let out = src_none(
            &mut book,
            Some(last.clone()),
            nowhere(),
            rects_hidden(),
            1595,
        );
        assert_eq!(tab_bar_at(&out), (POP.to_string(), Some(0)));
        // 시한 뒤의 끝이어도 스냅숏을 실었으면 숨기지 않은 것으로 친다(웹뷰가 같은 유예를 이미 쟀다).
        let mut book = hidden_by_leave(Some(POP_RECT));
        let late = DrawnReport {
            left_before_end_ms: None,
            ..last
        };
        let out = src_none(
            &mut book,
            Some(late),
            on(POP_RECT, LONE_P),
            rects_hidden(),
            1700,
        );
        assert_eq!(tab_bar_at(&out), (POP.to_string(), Some(0)));
        // 시작에 사각형을 못 읽었다 — 본 가이드(순서 이동)가 있으니 「사각형 없음」 = 무변경.
        let mut book = hidden_by_leave(None);
        let out = src_none(&mut book, None, on(POP_RECT, LONE_P), rects_hidden(), 1595);
        noop_end(&out, EndReason::TargetChanged);
    }

    #[test]
    fn an_end_after_the_leave_grace_is_a_hidden_end() {
        // 떠남 105 ms 뒤 · 스냅숏 없음 — 숨긴 출처는 후보가 아니고 W 가 없다.
        let mut book = hidden_by_leave(Some(POP_RECT));
        let out = src_none(&mut book, None, on(POP_RECT, LONE_P), rects_hidden(), 1605);
        assert!(matches!(applied_req(&out).to, DockDest::NewWindow { .. }));
        assert!(!applied_log(&mut book, POP, true).late_hide);
    }

    fn source_rect(book: &Book) -> Option<ClientRect> {
        book.drag.as_ref().unwrap().source_rect
    }

    const MOVED: ClientRect = ClientRect {
        origin: (7000, 200),
        size: (1200, 856),
        scale: 1.0,
    };

    #[test]
    fn the_held_source_rect_follows_accepted_rects_that_carry_the_source() {
        let (mut book, _) = lone_case();
        assert_eq!(source_rect(&book), Some(POP_RECT));
        // 받아들인 출처 끝(move)이 든 사각형 — 갈아 끼운다.
        let n = armed(&src_end_full(
            &mut book,
            DragEffect::Move,
            false,
            None,
            cursor(),
            rects_of(&[(MAIN, MAIN_RECT), (POP, MOVED)]),
            1500,
        ))
        .1;
        assert_eq!(source_rect(&book), Some(MOVED));
        // 출처가 없는 사각형(숨김 · 최소화) — 그대로 둔다.
        fire(&mut book, TimerKind::Drop, n, cursor(), rects_hidden());
        assert_eq!(source_rect(&book), Some(MOVED));
    }

    #[test]
    fn rejected_inputs_do_not_touch_the_held_source_rect() {
        let carrying = || rects_of(&[(MAIN, MAIN_RECT), (POP, MOVED)]);
        let (mut book, _) = lone_case();
        // 지난 세대 · 지난 번호의 시한 · 다른 id 의 새 시작 끝내기.
        for (generation, number) in [(GEN + 1, 1), (GEN, 999)] {
            book.step(Input::Timeout {
                kind: TimerKind::Drop,
                generation,
                number,
                cursor: None,
                rects: carrying(),
                now: MonoMs(1500),
            });
        }
        book.step(Input::Superseded {
            drag_id: "d-9".into(),
            cursor: cursor(),
            rects: carrying(),
            now: MonoMs(1500),
        });
        assert_eq!(source_rect(&book), Some(POP_RECT));
        // 끝 신호를 이미 받은 세션의 둘째 출처 끝 · 끄는 탭이 아닌 놓기.
        armed(&src_end(
            &mut book,
            DragEffect::Move,
            cursor(),
            rects(),
            1600,
        ));
        src_end_full(
            &mut book,
            DragEffect::None,
            false,
            None,
            cursor(),
            carrying(),
            1610,
        );
        drop_with(
            &mut book,
            "d-1",
            id(10),
            MAIN,
            report(0, 0, None, Picked::None, NONE_P),
            cursor(),
            carrying(),
        );
        assert_eq!(source_rect(&book), Some(POP_RECT));

        // 시작에 이어 먹이는 붙든 이른 신호 — 시작 뒤에 읽은 사각형을 그보다 옛 읽기로 덮지 않는다.
        let mut book = Book::new();
        armed(&drop_with(
            &mut book,
            "d-1",
            id(SRC),
            MAIN,
            report(0, 0, Some(10), Picked::None, NONE_P),
            cursor(),
            carrying(),
        ));
        start_full(
            &mut book,
            "d-1",
            GEN,
            POP,
            true,
            Some(strip_of(&[SRC], 0.0)),
            Some(POP_RECT),
            popout_windows(),
        );
        assert_eq!(source_rect(&book), Some(POP_RECT));
    }

    #[test]
    fn a_late_source_over_while_held_does_not_turn_w_back_to_the_source() {
        // 겹친 창(main · slot-popup-2 가 같은 자리) — 풀기에 `from` 이 없어 도착 순번으로 고른다.
        let overlap = rects_of(&[(MAIN, MAIN_RECT), (POP2, MAIN_RECT), (POP, POP_RECT)]);
        let (mut book, _) = lone_case();
        over_at(&mut book, POP, report(1, 1, None, reorder(0), LONE_P), 1400);
        left_at(&mut book, POP, 1450);
        let last = drawn(report(1, 1, None, reorder(0), LONE_P), Some(60.0));
        let n = armed_full(&src_none(
            &mut book,
            Some(last),
            on(MAIN_RECT, SEAM_P),
            overlap,
            1500,
        ))
        .2;
        let r = seam_report(&book, MAIN, 10);
        over_at(&mut book, MAIN, r, 1510);
        let r = seam_report(&book, POP2, 20);
        over_at(&mut book, POP2, r, 1520);
        // 출처 줄의 늦은 Over — 끝 스냅숏보다 옛 그림이다.
        assert!(over_t(&mut book, POP, report(1, 1, None, reorder(0), LONE_P), 1530).is_empty());
        let out = fire_gen(&mut book, TimerKind::OverWait, GEN, n, 1550);
        assert_eq!(seam_target(&out), id(20));
        let log = held_log(&mut book, POP2);
        assert_eq!(
            (log.over_window.as_deref(), log.last),
            (Some(POP2), LastUse::NotW)
        );
    }

    #[test]
    fn a_late_source_over_while_held_does_not_mask_last_differs() {
        // 출처가 W 로 남는 풀기 — 끝 스냅숏(자리 2)은 붙들기 전의 출처 Over(자리 1)와 견준다.
        let mut book = reorder_case(0.0);
        over_at(
            &mut book,
            MAIN,
            report(1, 1, Some(10), reorder(1), IN_PLACE_P),
            1400,
        );
        left_at(&mut book, MAIN, 1450);
        let last = drawn(report(1, 1, Some(10), reorder(2), RIGHT_P), Some(60.0));
        let n = armed_full(&src_none(
            &mut book,
            Some(last),
            on(POP2_RECT, SEAM_P),
            all_rects(),
            1500,
        ))
        .2;
        over_at(
            &mut book,
            MAIN,
            report(1, 1, Some(10), reorder(2), RIGHT_P),
            1510,
        );
        let out = fire_gen(&mut book, TimerKind::OverWait, GEN, n, 1550);
        // 실제로 떠났고 끝 커서는 출처 밖 — 새 창.
        assert!(matches!(applied_req(&out).to, DockDest::NewWindow { .. }));
        let log = applied_log(&mut book, POP, true);
        assert_eq!(
            (log.over_window.as_deref(), log.last, log.last_differs),
            (Some(MAIN), LastUse::Used, true)
        );
    }

    #[test]
    fn a_new_start_during_the_drop_wait_cursor_judges_the_source_at_the_end_cursor() {
        // 놓기 기다리기의 커서는 뗀 순간의 것이라 끝 뒤에 움직이지 않았다 — 출처 커서 판정에 닿는다(메인 판단).
        let mut book = reorder_case(0.0);
        armed(&src_end_full(
            &mut book,
            DragEffect::Move,
            false,
            None,
            on(MAIN_RECT, RIGHT_P),
            rects(),
            2000,
        ));
        let out = superseded(&mut book, "d-1", nowhere());
        assert_eq!(tab_bar_at(&out), (MAIN.to_string(), Some(2)));
        let log = kept_log(&mut book, MAIN).1;
        assert_eq!((log.end_by, log.unseen), (Some(EndBy::NewStart), true));
    }

    #[test]
    fn visibility_is_hidden_only_for_the_live_sessions_hidden_source() {
        let mut book = fresh();
        assert_eq!(book.visibility(POP), Visibility::Hidden);
        assert_eq!(book.visibility(MAIN), Visibility::Shown);
        book.step(Input::Snapshot(snap(2, None, vec![])));
        assert_eq!(book.visibility(POP), Visibility::Shown, "끝난 뒤");
        assert_eq!(Book::new().visibility(POP), Visibility::Shown);
    }
}
