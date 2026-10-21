//! 도킹 띠 기하의 정본(TRD S21-slot-docking §4-5) — 창 하나의 캔버스 위 탭바 구역 · 바깥 띠 · 칸 띠 · 그 유효 영역 ·
//! 붙는 몫 · 붙을 사각형(미리보기) · 점의 띠 고르기 · 칸 구역 선. 좌표 = 그 창 캔버스의 CSS px(캔버스 왼쪽 위 원점) ·
//! 띠 · 영역 · 붙을 사각형은 반열림 정수 사각형.
//!
//! ★웹 거울과 한 벌이다★ — 끄는 동안 끌기는 판정을 셸과 맞추지 않아(ADR-0296 · ADR-0297) 웹(`dock/bands.ts` ·
//! `dockedTree.ts` · `judge.ts`)이 같은 규칙을 혼자 센다. 둘은 골든 `bands_golden.json` 을 함께 먹는다 — 규칙을
//! 고치면 골든을 다시 뜨고 TS 판을 같은 변경에서 고친다(한쪽만 고치면 다른 쪽이 빨개진다). 숫자는 이 파일 머리의
//! `pub const` 한 벌이고 웹은 생성물 `src-tauri/bindings/DockConstants.ts` 로 읽는다 — 이 파일 몸 안에 같은 숫자를 다시
//! 적지 않는다.
//!
//! [`choose`] 는 유효 영역(`BandHit::areas`)만 짚는다. 미리보기는 고른 띠의 `landing` 이고, 칸 구역 선
//! ([`slot_zones`])의 대각선은 이웃한 두 칸 띠의 [`choose`] 경계와 같은 선이다. 붙을 사각형과 놓은 결과는 같은
//! 몫(`BandHit::share`)으로 같은 `docked_tree` 를 지난다(PRD R4 · R7) — 놓기는 그 `share` 를 그대로 `Seam` 에 싣고
//! 다시 셈하지 않는다.
//!
//! 앞서는 차례(R6a) = 탭바 구역 > 바깥 띠 > 칸 띠. 막힌 띠(R8 — 놓으면 새로 면적 0 칸이 생김)는 없는 것으로 친다 —
//! 고르지 않고, 막힌 바깥 띠는 밑 칸 띠를 덮지 않는다(AC36). 칸 띠는 칸 틀이 아니라 그 칸의 띠 틀([`band_frame`] —
//! 유효한 바깥 띠 몫을 뺀 칸 틀) 위에 선다 — 그래서 캔버스 변에 닿은 칸도 그 변의 칸 띠가 남는다.
//!
//! 진입점: [`strip_zone`] → [`zone_in_canvas`](탭바 구역) · [`effective_bands`](창 하나의 유효 띠 전부) ·
//! [`bands_at`](바깥 띠 + 점 밑 칸의 띠) → [`choose`] · [`slot_zones`](점에서 그릴 칸 구역 선).

use ts_rs::TS;
use uuid::Uuid;

use super::dock::{docked_tree, incoming_rect, DockAnchor, DockSide};
use super::geometry::{self as layout_geometry, px_edge, PxRect};
use super::{tree, CanvasPx, LayoutNode};

/// 칸 띠 두께 = 칸 길이의 10%(ADR-0295 결정 3 · PRD R3 — VS Code `editorDropTarget.ts` 의 0.1 꼴).
// ADR-0295
pub const BAND_SHARE: f64 = 0.10;

/// 칸 띠의 최소 두께(CSS px) — 포인터로 짚을 수 있는 가장 얇은 띠(G5 — 피어 Lumino 뿌리 위 가장자리). 폭 50 칸
/// (AC39)에서도 좌우 띠가 칸 절반을 넘지 않는다.
// ADR-0295
pub const BAND_MIN_PX: u32 = 12;

/// 바깥 띠 두께(CSS px — PRD R5). [`STRIP_ZONE_PX`] 보다 커야 바깥 위 띠의 유효 부분이 탭바 구역 바로 아래에 남는다
/// (AC44 — 아래 단언). 20 = 사용자 결정 2026-10-19(「바깥은 트리거 되는 영역을 좀 작게」) — 탭바가 캔버스 바로
/// 위면 탭바 밑에 남는 바깥 위 띠는 5 px 다.
pub const OUTER_BAND_PX: u32 = 20;

/// 탭바 구역 = 탭 끌기 영역을 위아래로 넓히는 폭(CSS px) — Chrome `kVerticalDetachMagnetism`(TDC#401 · 조사 A5).
/// 터치 값(50)은 쓰지 않는다 — 마우스만 다룬다.
// ADR-0295
pub const STRIP_ZONE_PX: f64 = 15.0;

/// 붙는 몫의 상한 — 원래 크기가 커도 붙는 칸(바깥 띠면 캔버스)의 절반까지다(ADR-0295 결정 4 · PRD R7).
// ADR-0295
pub const ATTACH_SHARE_MAX: f64 = 0.5;

/// Chrome `kMaximizedWindowInset`(TDC#129) — 새 창을 작업 영역보다 한쪽마다 이만큼(DIP) 안쪽으로 줄여 만들자마자
/// 최대화되지 않게(`drag_size::dragged_size`).
pub const MAXIMIZED_INSET_DIP: f64 = 10.0;

/// 동률 깨기 순서 — 바깥 띠 · 칸 띠 모서리 둘 다(R6a ③).
pub const SIDE_ORDER: [DockSide; 4] = [
    DockSide::Left,
    DockSide::Right,
    DockSide::Top,
    DockSide::Bottom,
];

const _: () = assert!(OUTER_BAND_PX as f64 > STRIP_ZONE_PX);

// 상한을 모델 상한보다 올리면 미리보기는 그려지는데 놓기가 거절된다.
const _: () = assert!(ATTACH_SHARE_MAX <= crate::layout::dock::SHARE_MAX);

/// 논리(DIP) px 크기 — 웹뷰 확대 1 의 CSS px 와 같다.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SizeDip {
    pub w: f64,
    pub h: f64,
}

/// 점 — CSS px(소수 허용). 원점은 쓰는 자리가 정한다: 웹뷰 wire(`TabDrop` 등)는 그 창 뷰포트 왼쪽 위 · 띠 판정은
/// 그 창 캔버스 왼쪽 위.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct PointCss {
    pub x: f64,
    pub y: f64,
}

/// 사각형 — CSS px 경계 꼴(`getBoundingClientRect` 의 left · top · right · bottom · 원점은 [`PointCss`] 와 같이 쓰는
/// 자리가 정한다). `x0 <= x1` · `y0 <= y1` 이어야 한다. 반열림(왼쪽 · 위 포함, 오른쪽 · 아래 제외)으로 읽는다
/// ([`contains`]).
// ts 내보내기를 달지 않는다 — 웹은 `dock/overlayTypes.ts` 의 TS 고유 타입을 쓰고 두 판은 골든이 묶는다(TRD §5-4).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RectCss {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// 띠 하나 — 고른 띠의 판정 · 그림 · 놓기 재료가 다 여기 있다.
#[derive(Debug, Clone, PartialEq)]
pub struct BandHit {
    pub anchor: DockAnchor,
    pub side: DockSide,
    /// 띠 사각형 전체(앞선 영역을 빼기 전) — `side` 쪽 변이 칸 띠 틀([`band_frame`] · 바깥 띠면 캔버스) 변과 같다.
    /// [`choose`] 의 정규화 거리(변까지 거리 ÷ 두께) 재료.
    pub band: PxRect,
    /// 붙는 몫([`attach_share`]) — `landing` 이 이 값으로 셈했고 놓기의 `Seam.share` 도 이 값이다.
    pub share: f64,
    /// 놓으면 끼운 탭이 차지할 사각형(미리보기 — R4).
    pub landing: PxRect,
    /// 유효 영역 — 띠에서 앞서는 영역을 뺀 사각형 목록(비지 않는다 · 서로 겹치지 않는다). [`choose`] 가 짚는 곳.
    pub areas: Vec<PxRect>,
}

/// 창 하나 몫의 입력.
pub struct WindowTargets<'a> {
    pub canvas: CanvasPx,
    /// 그 창에 보이는 탭의 배치.
    pub layout: &'a LayoutNode,
    /// 끄는 탭의 배치.
    pub incoming: &'a LayoutNode,
    /// 끄는 탭의 원래 크기(R7) — 모르면(크기 칸이 없거나 깨짐) NaN 을 넣는다: 몫이 상한([`ATTACH_SHARE_MAX`])이 된다.
    pub original: SizeDip,
    /// 그 창 탭바 구역 — [`zone_in_canvas`] 의 값(캔버스 좌표 · 정수 경계 · 위로 음수일 수 있다). 탭바 판정이 쓴
    /// 경계와 같아야 한다. `None` = 탭바를 모른다(띠에서 빼지 않는다).
    pub zone: Option<RectCss>,
}

/// 칸 하나의 구역 선(캔버스 CSS px) — 그릴 것 = `centre` 테두리 + `slot` 과 `centre` 의 같은 쪽 모서리를 잇는 선
/// 넷(이웃한 두 칸 띠의 경계). `centre` 는 `slot` 안이고 폭이나 높이가 0 일 수 있다(좁은 칸 — 선분).
// ts 내보내기를 달지 않는다 — 웹은 `dock/overlayTypes.ts` 의 TS 고유 타입을 쓰고 두 판은 골든이 묶는다(TRD §5-4 —
// 생성 바인딩 수를 늘리지 않는다).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlotZones {
    /// 칸 띠가 서는 틀([`band_frame`]) — 캔버스 변에 닿은 칸이면 칸 틀보다 작다(그 몫은 바깥 띠라 선을 긋지 않는다).
    pub slot: PxRect,
    /// 어느 칸 띠에도 들지 않는 가운데([`slot_centre`]).
    pub centre: RectCss,
}

/// 탭바 구역 — `area`(탭 끌기 영역 사각형 · 뷰포트 CSS px)의 가로 그대로 · 세로 위아래 [`STRIP_ZONE_PX`]. 가로는
/// 탭들 범위다 — 첫 머리 왼쪽부터 보이는 마지막 머리 오른 끝(`+` 앞)까지(웹뷰 `useTabDragSource.ts` `geometryOf` · 사용자 결정
/// 2026-10-19 「도킹 영역을 + 전까지로」). 세로 판정은 Chrome `DoesTabStripContain`(TDC#1461-1477 ·
/// `DoesRectContainVerticalPointExpanded` TDC#234) 그대로고, 가로는 Chrome(쓸 수 있는 폭 전체 — TRD G7)과 다르다.
// ADR-0295
pub fn strip_zone(area: RectCss) -> RectCss {
    RectCss {
        x0: area.x0,
        y0: area.y0 - STRIP_ZONE_PX,
        x1: area.x1,
        y1: area.y1 + STRIP_ZONE_PX,
    }
}

/// 반열림 판정(왼쪽 · 위 포함) — 이웃 사각형이 같은 경계를 나눠도 점 하나가 한쪽에만 든다. NaN 점은 어디에도 없다.
pub fn contains(r: &RectCss, p: PointCss) -> bool {
    (r.x0..r.x1).contains(&p.x) && (r.y0..r.y1).contains(&p.y)
}

/// 탭바 구역(그 창 뷰포트 CSS — [`strip_zone`])을 캔버스 좌표로 옮기고 정수 경계로 바깥으로 넓힌 것. 탭바 판정 ·
/// 띠에서 빼기가 모두 이 경계 하나를 쓴다. 넓힌 1 px 미만은 탭바 쪽이다.
// 소수 경계(배율 125% 의 탭바 등)를 탭바 판정에는 그대로, 띠에는 정수로 쓰면 그 사이 줄이 탭바도 띠도 아니게 된다
// (예 — 구역 끝 43.125 · 커서 43.2 가 바깥 위 띠 위인데 아무것도 아님).
// ADR-0295
pub fn zone_in_canvas(zone: RectCss, canvas_origin: PointCss) -> RectCss {
    RectCss {
        x0: (zone.x0 - canvas_origin.x).floor(),
        y0: (zone.y0 - canvas_origin.y).floor(),
        x1: (zone.x1 - canvas_origin.x).ceil(),
        y1: (zone.y1 - canvas_origin.y).ceil(),
    }
}

/// 붙는 몫 — 칸(바깥 띠면 캔버스) `target` 의 `side` 쪽에 붙을 때 새것이 감싼 자리에서 차지하는 몫. 나누는 방향의
/// 원래 길이 `L` 과 칸 길이 `T` 로 `min(L, T ÷ 2) ÷ T` 를 `[RATIO_MIN, ATTACH_SHARE_MAX]` 로 자른 것(ADR-0295 결정 4 ·
/// PRD R7).
///
/// ★어떤 입력에도 유한한 `[tree::RATIO_MIN, ATTACH_SHARE_MAX]` 값이다★ — 미리보기([`landing_rect`])와 놓기
/// (`Seam.share`)가 같은 이 값을 `docked_tree` 에 넘기고, 놓기 쪽은 범위 밖을 `DockError::InvalidShare` 로 거절한다.
/// 그래서 칸 길이 0 이면 상한 · `L` 이 NaN · +∞ 면 상한 · 0 이하면 하한이다.
// ADR-0295
pub fn attach_share(original: SizeDip, side: DockSide, target: PxRect) -> f64 {
    let (l, t) = match side {
        DockSide::Left | DockSide::Right => (original.w, target.x1.saturating_sub(target.x0)),
        DockSide::Top | DockSide::Bottom => (original.h, target.y1.saturating_sub(target.y0)),
    };
    if t == 0 {
        return ATTACH_SHARE_MAX;
    }
    let t = f64::from(t);
    // `f64::min` 은 NaN 이 아닌 쪽을 돌려준다 — NaN `L` 은 상한으로 간다.
    let share = l.min(t * ATTACH_SHARE_MAX) / t;
    share.clamp(tree::RATIO_MIN, ATTACH_SHARE_MAX)
}

/// 칸 `frame` 의 띠 틀 — 칸 띠([`slot_band`]) · 칸 구역 선([`slot_centre`])이 서는 사각형. 칸 틀에서 바깥 띠가 선
/// 변(`outer` — 유효한 바깥 띠의 변)의 캔버스 변 안쪽 [`OUTER_BAND_PX`] 를 뺀다. 바깥 띠가 막힌 변은 빼지 않는다 —
/// 그 몫은 칸 띠 차지다(AC36). 칸이 그 몫 안에 다 들면 빈 사각형이다.
// 칸 틀 변에 붙이지 말 것 — 칸 띠(10% · 최소 12)가 바깥 띠보다 얇으면 통째로 덮여 그 변으로 나눌 길이 없어지고, 그
// 칸 변을 겨눈 커서에 뜬 바깥 띠 미리보기가 그 칸과 같은 사각형이라 칸 띠로 보인다(캔버스 882 × 544 · 위아래 반분의
// 아래 칸 = 띠 27 이 바깥 띠 32 에 덮였다 · 2026-10-19).
// ADR-0295
pub fn band_frame(frame: PxRect, canvas: CanvasPx, outer: &[DockSide]) -> PxRect {
    let cut = |side| {
        if outer.contains(&side) {
            OUTER_BAND_PX
        } else {
            0
        }
    };
    let x0 = frame.x0.max(cut(DockSide::Left)).min(frame.x1);
    let y0 = frame.y0.max(cut(DockSide::Top)).min(frame.y1);
    PxRect {
        x0,
        y0,
        x1: frame
            .x1
            .min(canvas.w.saturating_sub(cut(DockSide::Right)))
            .max(x0),
        y1: frame
            .y1
            .min(canvas.h.saturating_sub(cut(DockSide::Bottom)))
            .max(y0),
    }
}

/// 칸 `frame` 의 `side` 띠 — 두께 = `max(round(BAND_SHARE × 칸 길이), BAND_MIN_PX)`(좌우 = 폭 · 위아래 = 높이 · 칸 틀
/// 길이로 센다) · 띠 틀 `room`([`band_frame`])의 `side` 변에 붙여 `room` 안으로 자른다(두께가 `room` 보다 크면 `room`
/// 전체). `round` 는 .5 를 0 에서 멀리(G19) — 그린 띠와 판정이 같은 정수다.
// ADR-0295
pub fn slot_band(frame: PxRect, room: PxRect, side: DockSide) -> PxRect {
    let (tw, th) = room_thickness(frame, room);
    match side {
        DockSide::Left => PxRect {
            x1: room.x0 + tw,
            ..room
        },
        DockSide::Right => PxRect {
            x0: room.x1 - tw,
            ..room
        },
        DockSide::Top => PxRect {
            y1: room.y0 + th,
            ..room
        },
        DockSide::Bottom => PxRect {
            y0: room.y1 - th,
            ..room
        },
    }
}

fn band_thickness(len: u32) -> u32 {
    let share = (f64::from(len) * BAND_SHARE).round() as u32;
    share.max(BAND_MIN_PX).min(len)
}

// (좌우 띠 두께, 위아래 띠 두께) — 칸 틀 길이로 세고 띠 틀 안으로 자른다.
fn room_thickness(frame: PxRect, room: PxRect) -> (u32, u32) {
    let len = |a: u32, b: u32| b.saturating_sub(a);
    (
        band_thickness(len(frame.x0, frame.x1)).min(len(room.x0, room.x1)),
        band_thickness(len(frame.y0, frame.y1)).min(len(room.y0, room.y1)),
    )
}

/// 칸 `frame`(띠 틀 `room` — [`band_frame`])의 가운데 사각형(캔버스 CSS px · 소수) — 칸 띠([`slot_band`]) 넷 어디에도
/// 들지 않는 곳. `room` 모서리와 이 사각형의 같은 쪽 모서리를 잇는 선이 이웃한 두 칸 띠의 [`choose`] 경계(정규화
/// 거리가 같은 곳)다. 좌우(위아래) 띠가 `room` 절반을 넘어 겹치는 좁은 칸이면 그 축의 폭이 0 인 선분까지 줄인다 —
/// 그때 겹친 두 띠의 경계가 그 선분이다. 막힌 칸 띠 · 탭바 구역에 깎인 곳은 반영하지 않는다(띠 두께만으로 센 모양).
// 대각선 위 점 = 모서리 + s·(좌우 두께, 위아래 두께) — s 가 1 이면 가운데 모서리, 띠 틀 가운데 선에 먼저 닿으면
// 거기서 멈춘다(그 너머는 겹친 맞은편 띠가 더 가깝다).
pub fn slot_centre(frame: PxRect, room: PxRect) -> RectCss {
    let (tw, th) = room_thickness(frame, room);
    let (tw, th) = (f64::from(tw), f64::from(th));
    let reach = |a: u32, b: u32, t: f64| {
        if t > 0.0 {
            f64::from(b.saturating_sub(a)) / (2.0 * t)
        } else {
            1.0
        }
    };
    let s = reach(room.x0, room.x1, tw)
        .min(reach(room.y0, room.y1, th))
        .min(1.0);
    RectCss {
        x0: f64::from(room.x0) + s * tw,
        y0: f64::from(room.y0) + s * th,
        x1: f64::from(room.x1) - s * tw,
        y1: f64::from(room.y1) - s * th,
    }
}

/// 캔버스 네 변의 바깥 띠([`SIDE_ORDER`] 순) — 두께 [`OUTER_BAND_PX`] · 캔버스를 넘지 않는다.
pub fn outer_strips(canvas: CanvasPx) -> [(DockSide, PxRect); 4] {
    let (w, h) = (canvas.w, canvas.h);
    let (tw, th) = (OUTER_BAND_PX.min(w), OUTER_BAND_PX.min(h));
    [
        (
            DockSide::Left,
            PxRect {
                x0: 0,
                y0: 0,
                x1: tw,
                y1: h,
            },
        ),
        (
            DockSide::Right,
            PxRect {
                x0: w - tw,
                y0: 0,
                x1: w,
                y1: h,
            },
        ),
        (
            DockSide::Top,
            PxRect {
                x0: 0,
                y0: 0,
                x1: w,
                y1: th,
            },
        ),
        (
            DockSide::Bottom,
            PxRect {
                x0: 0,
                y0: h - th,
                x1: w,
                y1: h,
            },
        ),
    ]
}

/// 보이는 탭 배치 `layout` 의 칸 틀(캔버스 CSS px · 트리 전위 순) — 이웃 칸이 같은 정수 경계를 나눈다.
pub fn frames(layout: &LayoutNode, canvas: CanvasPx) -> Vec<(Uuid, PxRect)> {
    layout_geometry::compute(layout)
        .slots
        .iter()
        .map(|r| (r.slot_id, layout_geometry::frame_px(canvas.w, canvas.h, r)))
        .collect()
}

/// 점 `p`(캔버스 CSS px)를 품은 칸 — `None` = 캔버스 밖. 칸 틀은 반열림이라 이웃 칸 경계의 점은 오른쪽 · 아래 칸이다.
pub fn frame_at(layout: &LayoutNode, canvas: CanvasPx, p: PointCss) -> Option<(Uuid, PxRect)> {
    frames(layout, canvas)
        .into_iter()
        .find(|(_, frame)| contains_px(frame, (p.x, p.y)))
}

/// 대상 트리 `target` 의 `anchor` 의 `side` 에 `incoming` 을 몫 `share` 로 끼웠을 때 끼운 것이 차지할 캔버스 px
/// 사각형 — 미리보기이자 놓은 뒤 화면이 그릴 끼운 칸들의 외접 사각형이다(PRD R4). 결과에 새로 면적 0 칸이 생기거나
/// (R8) 기준 칸이 없으면 `None`. `share` 는 모델 몫 범위
/// `tree::RATIO_MIN ..=` [`crate::layout::dock::SHARE_MAX`] 여야 한다(`docked_tree` 계약 — [`attach_share`] 의 값은 늘 그
/// 안이다).
// 정규화 외접 사각형의 네 경계를 각각 `px_edge` 로 옮긴다 — `px_edge` 가 단조라 놓은 뒤 그려질 칸 틀들
// (`frame_px`)의 외접 사각형과 같은 정수가 나온다.
// ADR-0227
pub fn landing_rect(
    target: &LayoutNode,
    anchor: DockAnchor,
    side: DockSide,
    incoming: &LayoutNode,
    canvas: CanvasPx,
    share: f64,
) -> Option<PxRect> {
    let result = docked_tree(target, anchor, side, incoming, share).ok()?;
    let r = incoming_rect(
        &layout_geometry::compute(&result),
        &tree::slot_ids(incoming),
    )?;
    Some(PxRect {
        x0: px_edge(canvas.w, r.x0),
        y0: px_edge(canvas.h, r.y0),
        x1: px_edge(canvas.w, r.x1),
        y1: px_edge(canvas.h, r.y1),
    })
}

/// 사각형 `r` 에서 `cut` 을 뺀 나머지 — 서로 겹치지 않는 사각형 넷까지(위 · 아래 통째 · 그 사이 왼쪽 · 오른쪽 —
/// 이 차례로 싣는다). 빈 조각은 싣지 않는다.
pub fn subtract(r: PxRect, cut: PxRect) -> Vec<PxRect> {
    if is_empty(&r) {
        return Vec::new();
    }
    if is_empty(&cut) || !overlaps(&r, &cut) {
        return vec![r];
    }
    let (y0, y1) = (r.y0.max(cut.y0), r.y1.min(cut.y1));
    [
        PxRect { y1: cut.y0, ..r },
        PxRect { y0: cut.y1, ..r },
        PxRect {
            x1: cut.x0,
            y0,
            y1,
            ..r
        },
        PxRect {
            x0: cut.x1,
            y0,
            y1,
            ..r
        },
    ]
    .into_iter()
    .filter(|p| !is_empty(p))
    .collect()
}

fn subtract_all(r: PxRect, cuts: &[PxRect]) -> Vec<PxRect> {
    cuts.iter().fold(vec![r], |pieces, cut| {
        pieces.into_iter().flat_map(|p| subtract(p, *cut)).collect()
    })
}

/// 창 하나의 유효 띠 전부 — 바깥 띠([`SIDE_ORDER`]) 뒤에 칸마다([`frames`] 순) 그 칸의 띠([`SIDE_ORDER`]). 막힌
/// 띠와 앞선 영역에 전부 덮인 띠는 없다.
// ADR-0295
pub fn effective_bands(w: &WindowTargets) -> Vec<BandHit> {
    collect(w, |_| true)
}

/// 바깥 띠 + 점 `p`(캔버스 CSS px)를 품은 칸의 띠 — [`choose`] 가 그 점에서 볼 전부다. 순서 · 값은
/// [`effective_bands`] 에서 그 칸 밖의 칸 띠를 뺀 것과 같다.
pub fn bands_at(w: &WindowTargets, p: PointCss) -> Vec<BandHit> {
    collect(w, |frame| contains_px(frame, (p.x, p.y)))
}

/// `bands`([`effective_bands`] · [`bands_at`] 의 값)에 든 바깥 띠의 변 — [`band_frame`] 의 `outer`.
pub fn outer_sides(bands: &[BandHit]) -> Vec<DockSide> {
    bands
        .iter()
        .filter(|b| b.anchor == DockAnchor::Outer)
        .map(|b| b.side)
        .collect()
}

fn collect(w: &WindowTargets, keep: impl Fn(&PxRect) -> bool) -> Vec<BandHit> {
    let zone = zone_cut(w.zone);
    let whole = PxRect {
        x0: 0,
        y0: 0,
        x1: w.canvas.w,
        y1: w.canvas.h,
    };
    let mut out: Vec<BandHit> = outer_strips(w.canvas)
        .into_iter()
        .filter_map(|(side, strip)| {
            band_hit(w, DockAnchor::Outer, side, strip, whole, zone.as_slice())
        })
        .collect();
    // 칸 띠에서 빼는 것 = 탭바 구역 + 유효 바깥 띠 영역 전부(R6a ① · 막힌 바깥 띠는 이미 빠졌다 — AC36).
    let cuts: Vec<PxRect> = zone
        .into_iter()
        .chain(out.iter().flat_map(|b| b.areas.iter().copied()))
        .collect();
    let outer = outer_sides(&out);
    for (slot, frame) in frames(w.layout, w.canvas) {
        if !keep(&frame) {
            continue;
        }
        let room = band_frame(frame, w.canvas, &outer);
        for side in SIDE_ORDER {
            let band = slot_band(frame, room, side);
            out.extend(band_hit(
                w,
                DockAnchor::Slot(slot),
                side,
                band,
                frame,
                &cuts,
            ));
        }
    }
    out
}

fn band_hit(
    w: &WindowTargets,
    anchor: DockAnchor,
    side: DockSide,
    band: PxRect,
    target: PxRect,
    cuts: &[PxRect],
) -> Option<BandHit> {
    let areas = subtract_all(band, cuts);
    if areas.is_empty() {
        return None;
    }
    let share = attach_share(w.original, side, target);
    let landing = landing_rect(w.layout, anchor, side, w.incoming, w.canvas, share)?;
    Some(BandHit {
        anchor,
        side,
        band,
        share,
        landing,
        areas,
    })
}

// 캔버스 밖(음수)을 잘라 정수 사각형으로 — 정수가 아닌 값이 와도 바깥으로 넓혀 구역 안 점을 품은 영역이 남지 않게.
fn zone_cut(zone: Option<RectCss>) -> Option<PxRect> {
    let z = zone?;
    let lo = |v: f64| v.floor().max(0.0) as u32;
    let hi = |v: f64| v.ceil().max(0.0) as u32;
    let r = PxRect {
        x0: lo(z.x0),
        y0: lo(z.y0),
        x1: hi(z.x1),
        y1: hi(z.y1),
    };
    (!is_empty(&r)).then_some(r)
}

/// 점 `p`(캔버스 CSS px)에서 고르는 띠의 자리 — ① 점을 품은 바깥 띠 영역이 있으면 그중 정규화 거리(띠 변까지 거리
/// ÷ 두께)가 가장 작은 것 ② 아니면 점을 품은 칸 띠 영역 중 같은 기준 ③ 같으면 `bands` 의 앞의 것(그래서
/// [`SIDE_ORDER`]) ④ 없으면 `None`(칸 가운데 · 캔버스 밖 — R6). 탭바 구역은 이 함수 앞에서 걸러진다.
// ADR-0295
pub fn choose(bands: &[BandHit], p: PointCss) -> Option<usize> {
    let p = (p.x, p.y);
    let nearest = |outer: bool| {
        let mut best: Option<(usize, f64)> = None;
        for (i, b) in bands.iter().enumerate() {
            if (b.anchor == DockAnchor::Outer) != outer
                || !b.areas.iter().any(|a| contains_px(a, p))
            {
                continue;
            }
            let d = edge_gap(p, &b.band, b.side) / thickness(&b.band, b.side);
            if best.is_none_or(|(_, x)| d < x) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| i)
    };
    nearest(true).or_else(|| nearest(false))
}

/// 점 `p`(캔버스 CSS px)에서 그릴 칸 구역 선 — `bands` = 같은 창의 [`effective_bands`] 또는 그 점의 [`bands_at`].
/// `None` = 고른 띠가 바깥 띠다(그때 안내는 레이아웃 틀이다) · `p` 가 캔버스 밖이다 · 점 밑 칸에 유효한 칸 띠가
/// 없다. 탭바 구역은 이 함수 앞에서 걸러진다(R6a — 탭바에서는 선을 그리지 않는다).
// 사용자 결정 2026-10-19 — 안내 = 커서 칸의 얇은 선 · 바깥 틀은 바깥 띠에 닿을 때만.
pub fn slot_zones(w: &WindowTargets, bands: &[BandHit], p: PointCss) -> Option<SlotZones> {
    if choose(bands, p).is_some_and(|i| bands[i].anchor == DockAnchor::Outer) {
        return None;
    }
    let (slot, frame) = frame_at(w.layout, w.canvas, p)?;
    if !bands.iter().any(|b| b.anchor == DockAnchor::Slot(slot)) {
        return None;
    }
    let room = band_frame(frame, w.canvas, &outer_sides(bands));
    Some(SlotZones {
        slot: room,
        centre: slot_centre(frame, room),
    })
}

fn is_empty(r: &PxRect) -> bool {
    r.x0 >= r.x1 || r.y0 >= r.y1
}

fn overlaps(a: &PxRect, b: &PxRect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}

fn contains_px(r: &PxRect, p: (f64, f64)) -> bool {
    (f64::from(r.x0)..f64::from(r.x1)).contains(&p.0)
        && (f64::from(r.y0)..f64::from(r.y1)).contains(&p.1)
}

/// 점에서 `frame` 의 `side` 변까지 — 안쪽이면 0 이상.
fn edge_gap(p: (f64, f64), frame: &PxRect, side: DockSide) -> f64 {
    match side {
        DockSide::Left => p.0 - f64::from(frame.x0),
        DockSide::Right => f64::from(frame.x1) - p.0,
        DockSide::Top => p.1 - f64::from(frame.y0),
        DockSide::Bottom => f64::from(frame.y1) - p.1,
    }
}

// 점을 품은 띠만 부른다 — 그러면 두께가 양수라 나눗셈이 선다.
fn thickness(r: &PxRect, side: DockSide) -> f64 {
    match side {
        DockSide::Left | DockSide::Right => f64::from(r.x1 - r.x0),
        DockSide::Top | DockSide::Bottom => f64::from(r.y1 - r.y0),
    }
}

#[cfg(test)]
mod golden;

#[cfg(test)]
mod tests;
