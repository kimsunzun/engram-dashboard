//! 도킹 wire 페이로드(TRD S21-slot-docking §1-5) — 셸 → 웹뷰 영역([`DockRegions`] · `dock:regions`) · 세션 사건
//! ([`SessionEvent`] · `dock:session`) · 당긴 답([`DockPullReply`]), 웹뷰 → 셸 시작 측정([`StripStart`]) · 보고
//! ([`DragOverReport`] · [`DropReport`] · [`DragEffect`] · 끝 스냅숏 [`DrawnReport`]).
//! ts-rs 로 `src-tauri/bindings/` 에 굽는다. 웹뷰 → 셸 측정(크롬)은 레이아웃 창 항목에 두므로 `layout::chrome` 에
//! 있다.
//!
//! 칸 · 띠 사각형은 [`PxRectWire`] = 그 창 캔버스의 CSS px 정수(캔버스 왼쪽 위 원점), 탭바 · 구역 · 점은 그 창
//! 뷰포트의 CSS px 다. 셸이 계산하고 웹뷰는 그 사각형 그대로 점-사각형 검사를 하고 그린다(ADR-0227 결정 3). 예외는
//! 탭 머리 x · 끼울 x · 끄는 머리 왼쪽(탭바 내용 좌표 — `TabHeader`)뿐이다: 웹뷰는 그것과 견줄 때 점 x 에 탭바
//! 스크롤을 더하고, 그릴 때 지금 스크롤을 뺀다.
//!
//! 철자: 구조체 칸은 snake_case · 데이터를 단 enum 은 `type` 판별자에 snake_case variant(레이아웃 타입과 같은 꼴) ·
//! id 는 문자열 · `DockSide` 만 variant 이름 그대로다(`layout::dock` 의 rename 금지).

use ts_rs::TS;
use uuid::Uuid;

use crate::layout::chrome::{PointCss, RectCss, TabHeader};
use crate::layout::dock::DockSide;
use crate::layout::geometry::PxRect;
use crate::layout::CanvasPx;

/// 캔버스 CSS px 정수 사각형 — 캔버스 왼쪽 위 원점의 경계 꼴(`x0 <= x1` · `y0 <= y1` · 폭 = `x1 - x0`). 폭이나
/// 높이가 0 일 수 있다(그릴 것이 없다). 판정은 반열림(왼쪽 · 위 포함)이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct PxRectWire {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl From<PxRect> for PxRectWire {
    fn from(r: PxRect) -> Self {
        Self {
            x0: r.x0,
            y0: r.y0,
            x1: r.x1,
            y1: r.y1,
        }
    }
}

impl From<PxRectWire> for PxRect {
    fn from(r: PxRectWire) -> Self {
        Self {
            x0: r.x0,
            y0: r.y0,
            x1: r.x1,
            y1: r.y1,
        }
    }
}

/// 칸 띠 하나. `side` = 붙일 쪽 · `anchor` = 붙일 칸 — [`SlotRegion::bands`] 안에서는 늘 `Some(그 칸)` 이다(바깥
/// 변은 [`OuterBand`] 가 따로 싣는다) · `rect` = 그릴 사각형이자 판정 사각형이고, 놓았을 때 끼운 것이 차지할
/// 사각형이다(PRD R3).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct Band {
    pub side: DockSide,
    #[ts(type = "string | null")]
    pub anchor: Option<Uuid>,
    pub rect: PxRectWire,
}

/// 한 창의 끌기 영역(`dock:regions` — TRD §1-3 「영역」 · §2-4). 웹뷰는 점이 어느 사각형인가만 본다(`pick` 의 거울).
///
/// - `drag_id`: 그 끌기 — 웹뷰는 `drag_id` 별로 가장 큰 `rev` 만 쓴다.
/// - `rev`: 창별 · 세션 안 단조. 영역이나 그 창의 놓기 기대값이 바뀌면 오른다 — 이음새 놓기의 대조 기준.
/// - `bar_rev`: 창별 · 세션 안 단조. 탭바 몫(`tab_bar` 기하 · 탭 목록 기대값 · `reorder` 의 생김 · 빠짐)이 바뀔
///   때만 오른다 — 탭바 놓기의 대조 기준이다(활성 전환은 `rev` 만 올린다 · 탭바 `scroll` 만 바뀌어도 오르지 않는다).
/// - `blocked`: 출처 탭 배치가 끌기 시작 뒤 바뀌었다 — 놓기를 받지 않는다(`dropEffect = 'none'`). `true` 면 그
///   아래 칸은 전부 빈 값이다(`view_id` = `null` · 원점 · 캔버스 = 0 · 목록 = 빈 것 · `tab_bar` · `reorder` = `null`).
/// - `view_id`: 이 창에 보이는 탭(끄는 탭이 그 창 활성이면 떼어진 자리의 승계 탭). `null` = 보이는 탭 없음(메인
///   마지막 탭을 뗐다) — 그때 캔버스 위는 늘 가이드 없음이다. 웹뷰는 자기 캔버스가 보이던 탭이 이것과 다르면 칸 ·
///   바깥 띠를 그리지 않는다(탭바는 그린다).
/// - `canvas_origin`: 뷰포트 안 캔버스 왼쪽 위(크롬 보고 전 = (0, 0)). `canvas`: 캔버스 크기(보고 전 = 0×0).
/// - `outer`: 놓을 수 있는 바깥 변만(PRD R8 — 결과에 면적 0 칸이 생길 변은 빠진다).
/// - `slots`: 보이는 탭의 칸 전부. `outer` · `slots` 는 크롬 · 캔버스 보고가 둘 다 있고 보이는 탭이 있을 때만 싣는다.
/// - `tab_bar`: `null` = 탭바 놓기를 받지 않는다(크롬 보고 전).
/// - `reorder`: 출처 창에만 서는 순서 이동 영역(PRD R23 · TRD §2-4) — 점이 그 `zone` 안이면 다른 어느 사각형보다
///   먼저 `reorder` 판정이다. `null` = 순서 이동 없음(다른 창 · 시작 측정 없음 · 그 측정이 모델 탭 목록과 어긋남 ·
///   `blocked`) — 그 창 탭바는 `tab_bar` 로 판정한다.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct DockRegions {
    pub drag_id: String,
    // ts-rs 의 u64 기본 매핑은 bigint 이나 serde_json 은 number 로 싣는다.
    #[ts(type = "number")]
    pub rev: u64,
    #[ts(type = "number")]
    pub bar_rev: u64,
    pub blocked: bool,
    #[ts(type = "string | null")]
    pub view_id: Option<Uuid>,
    pub canvas_origin: PointCss,
    pub canvas: CanvasPx,
    pub outer: Vec<OuterBand>,
    pub slots: Vec<SlotRegion>,
    pub tab_bar: Option<TabBarRegion>,
    pub reorder: Option<ReorderRegion>,
}

/// 캔버스 바깥 변 하나(PRD R5) — `strip` = 그리고 판정하는 띠(두께 `dock::hit::OUTER_BAND_PX` · 캔버스를 넘지
/// 않는다) · `landing` = 놓았을 때 끼운 것이 차지할 사각형(레이아웃 전체의 그 변 1/3 — 칠할 것).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct OuterBand {
    pub side: DockSide,
    pub strip: PxRectWire,
    pub landing: PxRectWire,
}

/// 칸 하나 — `frame` = 칸 틀(반열림 · 이웃 칸과 같은 정수 경계를 나눈다) · `bands` = 그 칸의 놓을 수 있는 띠
/// (`anchor` = 이 칸 · R8 로 빠진 변은 없다). 두 띠가 겹치는 모서리는 칸 변까지 거리 ÷ 띠 두께가 작은 쪽이다.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct SlotRegion {
    #[ts(type = "string")]
    pub slot_id: Uuid,
    pub frame: PxRectWire,
    pub bands: Vec<Band>,
}

/// 탭바(PRD R21) — `rect` = 탭바 상자(뷰포트 CSS px · 반열림 · 스크롤로 안 움직인다) · `heads` = 끄는 탭을 뺀
/// 머리(크롬 보고 순서 · x = 내용 좌표 — 끼울 자리는 순서와 무관하게 가운데 x 로 센다) · `insert_xs[i]` = 자리
/// `i`(`0..=heads.len()`)에 그릴 끼울 x(`rect.x0` 에서 잰 내용 좌표 CSS px — 그릴 땐 지금 `scrollLeft` 를 뺀다) ·
/// `scroll` = 크롬을 잰 때의 탭바 `scrollLeft`(보고에 없었으면 0) — 판정에 스크롤이 주어지지 않을 때 쓴다.
/// 판정은 점의 내용 x(뷰포트 x + 스크롤)를 머리 가운데와 견주고, 「탭바 안」은 뷰포트 점 그대로 본다.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct TabBarRegion {
    pub rect: RectCss,
    pub heads: Vec<TabHeader>,
    pub insert_xs: Vec<f64>,
    pub scroll: f64,
}

/// 출처 창의 순서 이동 영역(PRD R23 · TRD §2-5) — 끌기 시작 때 잰 탭바([`StripStart`])를 세션 내내 얼린 값이다.
/// 웹뷰는 구역 포함 검사 · 자리 · 끄는 머리 왼쪽을 이 값만으로 셸과 같은 식으로 낸다(`dock::hit` 의 `reorder_*`).
///
/// - `zone`: 순서 이동 구역(뷰포트 CSS px · 반열림) — 탭바 상자의 x 그대로 · 위아래로 `dock::hit::REORDER_SLACK_PX`
///   만큼 넓힌 것(`dock::hit::reorder_zone`).
/// - `heads`: 머리 전부(끄는 머리 포함 · 화면 순서 · x = 내용 좌표). 순서가 모델 탭 목록과 같다.
/// - `dragged`: `heads` 안 끄는 머리 자리.
/// - `grab_x`: 끄는 머리 왼쪽에서 잰 잡은 자리(CSS px).
/// - `scroll`: 시작 때의 탭바 `scrollLeft` — 판정에 스크롤이 주어지지 않을 때 쓴다.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct ReorderRegion {
    pub zone: RectCss,
    pub heads: Vec<TabHeader>,
    pub dragged: u32,
    pub grab_x: f64,
    pub scroll: f64,
}

/// 끌기 시작 때 출처 웹뷰가 `dragstart` 안에서 잰 탭바(`dock_drag_start` 의 선택 칸 — TRD §1-5). 셸은 세션 내내
/// 그대로 쥐고 출처 창 영역의 [`ReorderRegion`] 을 이것으로 만든다.
///
/// - `bar`: 탭바 상자(스크롤 상자 자신 · 뷰포트 CSS px).
/// - `heads`: 머리 전부 — 끄는 머리 포함 · 화면(DOM) 순서 · x = 내용 좌표(잰 x + `scroll`).
/// - `grab_x`: `dragstart.clientX` − 끄는 머리의 뷰포트 x0(스크롤과 무관).
/// - `scroll`: `dragstart` 때 탭바 `scrollLeft`(CSS px).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct StripStart {
    pub bar: RectCss,
    pub heads: Vec<TabHeader>,
    pub grab_x: f64,
    pub scroll: f64,
}

/// 점 하나의 판정 결과 — 셸 `dock::regions::pick` 과 웹뷰 `pickTarget` 이 같은 영역에서 같은 값을 낸다(골든
/// `src-tauri/src/dock/pick_golden.json`).
///
/// - `none`: 가이드 없음(칸 가운데 · 캔버스 밖 · 보이는 탭 없음) — 셸도 그 자리를 가이드 없음으로 판정하면 놓기 =
///   새 창(R22).
/// - `seam`: `anchor` = 붙일 칸(`null` = 레이아웃 전체의 바깥 변) · `side` = 붙일 쪽.
/// - `tab_bar`: `index` = 끄는 탭을 뺀 탭 목록에서 끼울 자리(가운데 x 가 점의 내용 x 이하인 머리 수) · `over` =
///   점을 품은 머리(`null` = 없음 — 머리 사이 · 끝 뒤). 탭바 놓기의 대조는 `index` 만 본다.
/// - `reorder`: 출처 창 순서 이동 구역 안 — `index` = 끄는 탭이 놓일 자리(끄는 탭을 뺀 결과 탭 목록의 자리 ·
///   탭바 놓기의 `index` 와 같은 뜻 · 제자리면 `ReorderRegion.dragged`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum Picked {
    None,
    Seam {
        #[ts(type = "string | null")]
        anchor: Option<Uuid>,
        side: DockSide,
    },
    TabBar {
        index: u32,
        #[ts(type = "string | null")]
        over: Option<Uuid>,
    },
    Reorder {
        index: u32,
    },
}

/// `dock:session` 의 `started` · `display` 가 같이 싣는 칸 — 어느 것이 먼저 닿아도 같은 상태가 선다.
///
/// - `source_label` · `source_view`: 끄는 탭과 그 창.
/// - `display_active`: 끄는 탭이 그 창 활성이면 떼어진 자리에 보일 탭(탭 닫기와 같은 승계). `null` = 끄는 탭이
///   활성이 아니다(모델 활성을 그대로 보인다) · 또는 승계할 탭이 없다(메인 마지막 탭 — 빈 캔버스).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct Started {
    pub source_label: String,
    #[ts(type = "string")]
    pub source_view: Uuid,
    #[ts(type = "string | null")]
    pub display_active: Option<Uuid>,
}

/// `dock:session` 의 단계. `ended` 는 그 `drag_id` 의 마지막이다.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum SessionPhase {
    Started(Started),
    Display(Started),
    Ended,
}

/// `dock:session`(전체 emit) — `seq` 는 세션 안 단조이고 웹뷰는 `drag_id` 별로 가장 큰 `seq` 의 상태만 쓴다.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct SessionEvent {
    pub drag_id: String,
    #[ts(type = "number")]
    pub seq: u64,
    pub phase: SessionPhase,
}

/// `dock_pull` 의 답(셸 → 부른 웹뷰) — 명령은 이것을 `null` 로 감싸 돌려준다(`null` = 세션이 없다).
///
/// - `started`: 지금 세션 — `dock:session` 의 `started` 와 같은 칸.
/// - `regions`: 부른 창의 지금 영역. `null` = 그 창에 보낼 영역이 없다(숨긴 출처 · 재료 없음 · 사라진 창).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct DockPullReply {
    pub started: Started,
    pub regions: Option<DockRegions>,
}

/// `dock_drag_over` 의 보고 — 웹뷰는 `shown` · `rev` · `displayed` 중 하나가 바뀔 때만 `over` 를, 창 밖으로 나갈 때
/// `left` 를 보낸다.
///
/// - `rev` · `bar_rev`: 그 판정에 쓴 영역의 번호(영역을 받기 전 = 0).
/// - `displayed`: 그 창 캔버스가 보이던 탭(떼어진 자리의 승계 탭 포함 · `null` = 없음).
/// - `shown`: 그린 것 · `point`: 뷰포트 CSS px 의 점.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum DragOverReport {
    Over {
        #[ts(type = "number")]
        rev: u64,
        #[ts(type = "number")]
        bar_rev: u64,
        #[ts(type = "string | null")]
        displayed: Option<Uuid>,
        shown: Picked,
        point: PointCss,
    },
    Left,
}

/// `dock_drop` 의 페이로드 — `view_id` = 끄는 탭(끌기 데이터) · `displayed` = 놓은 창이 보이던 탭 · 나머지 칸은
/// [`DragOverReport::Over`] 와 같다(`shown` = 마지막으로 그린 것).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct DropReport {
    pub drag_id: String,
    #[ts(type = "string")]
    pub view_id: Uuid,
    #[ts(type = "number")]
    pub rev: u64,
    #[ts(type = "number")]
    pub bar_rev: u64,
    #[ts(type = "string | null")]
    pub displayed: Option<Uuid>,
    pub shown: Picked,
    pub point: PointCss,
}

/// `dock_drag_end` 의 선택 칸 `last`(끝 스냅숏 — TRD §1-3 「끝 신호」) — 출처 창이 끝에 아직 위였으면 그 창이 마지막으로
/// **그린** 판정. 앞 여섯 칸은 [`DragOverReport::Over`] 와 같다.
///
/// - `scroll`: 그 판정 때 그 창 탭바 `scrollLeft`(CSS px). `null` = 영역마다 잰 때의 것.
/// - `left_before_end_ms`: 그 판정 뒤 떠남 → `dragend` 사건 시각 간격(ms · 출처 웹뷰 한 엔진 시계). `null` = 떠남
///   없음. 셸은 이것을 `CANCEL_LEAVE_MS` 와 견줘 엔진이 끌기를 접으며 낸 떠남(끝이 출처 위)과 실제 떠남을 가른다.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct DrawnReport {
    #[ts(type = "number")]
    pub rev: u64,
    #[ts(type = "number")]
    pub bar_rev: u64,
    #[ts(type = "string | null")]
    pub displayed: Option<Uuid>,
    pub shown: Picked,
    pub point: PointCss,
    pub scroll: Option<f64>,
    pub left_before_end_ms: Option<f64>,
}

/// `dock_drag_end` 의 `effect` — 출처 `dragend` 의 `dropEffect`. `move` · `other`(copy · link) = 받은 창이 있다 ·
/// `none` = 받은 창이 없다(창 밖 · 틈 · 바탕화면에서 뗌 · Esc · 시스템 취소).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DragEffect {
    Move,
    None,
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock::hit::SIDE_ORDER;
    use serde_json::json;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn rect(x0: u32, y0: u32, x1: u32, y1: u32) -> PxRectWire {
        PxRectWire { x0, y0, x1, y1 }
    }

    #[test]
    fn dock_side_spells_the_same_in_typescript_and_on_the_wire() {
        let union = SIDE_ORDER
            .iter()
            .map(|s| serde_json::to_string(s).unwrap())
            .collect::<Vec<_>>()
            .join(" | ");
        assert_eq!(DockSide::inline(), union);
        assert_eq!(union, r#""Left" | "Right" | "Top" | "Bottom""#);
    }

    #[test]
    fn regions_go_out_in_snake_case_with_string_ids_and_integer_rects() {
        let r = DockRegions {
            drag_id: "d-1".into(),
            rev: 3,
            bar_rev: 2,
            blocked: false,
            view_id: Some(id(1)),
            canvas_origin: PointCss { x: 0.0, y: 28.5 },
            canvas: CanvasPx { w: 1200, h: 800 },
            outer: vec![OuterBand {
                side: DockSide::Bottom,
                strip: rect(0, 768, 1200, 800),
                landing: rect(0, 533, 1200, 800),
            }],
            slots: vec![SlotRegion {
                slot_id: id(2),
                frame: rect(0, 0, 600, 800),
                bands: vec![Band {
                    side: DockSide::Left,
                    anchor: Some(id(2)),
                    rect: rect(0, 0, 200, 800),
                }],
            }],
            tab_bar: Some(TabBarRegion {
                rect: RectCss {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 600.0,
                    y1: 28.0,
                },
                heads: vec![TabHeader {
                    view_id: id(3),
                    rect: RectCss {
                        x0: 4.0,
                        y0: 0.0,
                        x1: 120.25,
                        y1: 28.0,
                    },
                }],
                insert_xs: vec![4.0, 120.25],
                scroll: 0.0,
            }),
            reorder: None,
        };
        let wire = json!({
            "drag_id": "d-1",
            "rev": 3,
            "bar_rev": 2,
            "blocked": false,
            "view_id": id(1).to_string(),
            "canvas_origin": { "x": 0.0, "y": 28.5 },
            "canvas": { "w": 1200, "h": 800 },
            "outer": [{
                "side": "Bottom",
                "strip": { "x0": 0, "y0": 768, "x1": 1200, "y1": 800 },
                "landing": { "x0": 0, "y0": 533, "x1": 1200, "y1": 800 },
            }],
            "slots": [{
                "slot_id": id(2).to_string(),
                "frame": { "x0": 0, "y0": 0, "x1": 600, "y1": 800 },
                "bands": [{
                    "side": "Left",
                    "anchor": id(2).to_string(),
                    "rect": { "x0": 0, "y0": 0, "x1": 200, "y1": 800 },
                }],
            }],
            "tab_bar": {
                "rect": { "x0": 0.0, "y0": 0.0, "x1": 600.0, "y1": 28.0 },
                "heads": [{
                    "view_id": id(3).to_string(),
                    "rect": { "x0": 4.0, "y0": 0.0, "x1": 120.25, "y1": 28.0 },
                }],
                "insert_xs": [4.0, 120.25],
                "scroll": 0.0,
            },
            "reorder": null,
        });
        assert_eq!(serde_json::to_value(&r).unwrap(), wire);
        assert_eq!(serde_json::from_value::<DockRegions>(wire).unwrap(), r);
    }

    #[test]
    fn absent_view_and_tab_bar_are_null() {
        let r = DockRegions {
            drag_id: String::new(),
            rev: 0,
            bar_rev: 0,
            blocked: true,
            view_id: None,
            canvas_origin: PointCss { x: 0.0, y: 0.0 },
            canvas: CanvasPx { w: 0, h: 0 },
            outer: vec![],
            slots: vec![],
            tab_bar: None,
            reorder: None,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["view_id"], serde_json::Value::Null);
        assert_eq!(v["tab_bar"], serde_json::Value::Null);
        assert_eq!(v["reorder"], serde_json::Value::Null);
    }

    fn head(n: u128, x0: f64, x1: f64) -> TabHeader {
        TabHeader {
            view_id: id(n),
            rect: RectCss {
                x0,
                y0: 0.0,
                x1,
                y1: 28.0,
            },
        }
    }

    #[test]
    fn the_reorder_region_and_the_tab_bar_scroll_go_out_in_snake_case() {
        let zone = RectCss {
            x0: 40.0,
            y0: -15.0,
            x1: 640.0,
            y1: 43.0,
        };
        let r = ReorderRegion {
            zone,
            heads: vec![head(1, 40.0, 140.0), head(2, 140.0, 260.5)],
            dragged: 1,
            grab_x: 30.25,
            scroll: 12.5,
        };
        let wire = json!({
            "zone": { "x0": 40.0, "y0": -15.0, "x1": 640.0, "y1": 43.0 },
            "heads": [
                { "view_id": id(1).to_string(), "rect": { "x0": 40.0, "y0": 0.0, "x1": 140.0, "y1": 28.0 } },
                { "view_id": id(2).to_string(), "rect": { "x0": 140.0, "y0": 0.0, "x1": 260.5, "y1": 28.0 } },
            ],
            "dragged": 1,
            "grab_x": 30.25,
            "scroll": 12.5,
        });
        assert_eq!(serde_json::to_value(&r).unwrap(), wire);
        assert_eq!(serde_json::from_value::<ReorderRegion>(wire).unwrap(), r);

        let bar = TabBarRegion {
            rect: zone,
            heads: vec![],
            insert_xs: vec![0.0],
            scroll: 80.0,
        };
        assert_eq!(serde_json::to_value(&bar).unwrap()["scroll"], json!(80.0));
    }

    #[test]
    fn strip_start_reads_what_the_webview_measures() {
        let s: StripStart = serde_json::from_value(json!({
            "bar": { "x0": 0.0, "y0": 0.0, "x1": 300.0, "y1": 28.0 },
            "heads": [{ "view_id": id(3).to_string(), "rect": { "x0": 120.0, "y0": 0.0, "x1": 220.0, "y1": 28.0 } }],
            "grab_x": 50.0,
            "scroll": 120.0,
        }))
        .unwrap();
        assert_eq!(
            s,
            StripStart {
                bar: RectCss {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 300.0,
                    y1: 28.0,
                },
                heads: vec![head(3, 120.0, 220.0)],
                grab_x: 50.0,
                scroll: 120.0,
            }
        );
    }

    #[test]
    fn picked_is_tagged_by_type_in_snake_case() {
        let cases = [
            (Picked::None, json!({ "type": "none" })),
            (
                Picked::Seam {
                    anchor: Some(id(5)),
                    side: DockSide::Right,
                },
                json!({ "type": "seam", "anchor": id(5).to_string(), "side": "Right" }),
            ),
            (
                Picked::Seam {
                    anchor: None,
                    side: DockSide::Top,
                },
                json!({ "type": "seam", "anchor": null, "side": "Top" }),
            ),
            (
                Picked::TabBar {
                    index: 2,
                    over: None,
                },
                json!({ "type": "tab_bar", "index": 2, "over": null }),
            ),
            (
                Picked::Reorder { index: 3 },
                json!({ "type": "reorder", "index": 3 }),
            ),
        ];
        for (picked, wire) in cases {
            assert_eq!(serde_json::to_value(&picked).unwrap(), wire);
            assert_eq!(serde_json::from_value::<Picked>(wire).unwrap(), picked);
        }
    }

    #[test]
    fn started_and_display_carry_the_same_fields() {
        let s = Started {
            source_label: "slot-popup-1".into(),
            source_view: id(7),
            display_active: Some(id(8)),
        };
        let fields = json!({
            "source_label": "slot-popup-1",
            "source_view": id(7).to_string(),
            "display_active": id(8).to_string(),
        });
        for (phase, tag) in [
            (SessionPhase::Started(s.clone()), "started"),
            (SessionPhase::Display(s.clone()), "display"),
        ] {
            let mut want = fields.clone();
            want["type"] = json!(tag);
            let e = SessionEvent {
                drag_id: "d-1".into(),
                seq: 4,
                phase,
            };
            let wire = json!({ "drag_id": "d-1", "seq": 4, "phase": want });
            assert_eq!(serde_json::to_value(&e).unwrap(), wire);
            assert_eq!(serde_json::from_value::<SessionEvent>(wire).unwrap(), e);
        }
        let ended = SessionEvent {
            drag_id: "d-1".into(),
            seq: 5,
            phase: SessionPhase::Ended,
        };
        assert_eq!(
            serde_json::to_value(&ended).unwrap(),
            json!({ "drag_id": "d-1", "seq": 5, "phase": { "type": "ended" } })
        );
    }

    #[test]
    fn reports_read_what_the_webview_sends() {
        let over: DragOverReport = serde_json::from_value(json!({
            "type": "over",
            "rev": 4,
            "bar_rev": 2,
            "displayed": id(1).to_string(),
            "shown": { "type": "tab_bar", "index": 0, "over": id(3).to_string() },
            "point": { "x": 10.5, "y": 3.0 },
        }))
        .unwrap();
        assert_eq!(
            over,
            DragOverReport::Over {
                rev: 4,
                bar_rev: 2,
                displayed: Some(id(1)),
                shown: Picked::TabBar {
                    index: 0,
                    over: Some(id(3)),
                },
                point: PointCss { x: 10.5, y: 3.0 },
            }
        );
        let left: DragOverReport = serde_json::from_value(json!({ "type": "left" })).unwrap();
        assert_eq!(left, DragOverReport::Left);

        for (effect, wire) in [
            (DragEffect::Move, "move"),
            (DragEffect::None, "none"),
            (DragEffect::Other, "other"),
        ] {
            assert_eq!(serde_json::to_value(effect).unwrap(), json!(wire));
            assert_eq!(
                serde_json::from_value::<DragEffect>(json!(wire)).unwrap(),
                effect
            );
        }

        let drop: DropReport = serde_json::from_value(json!({
            "drag_id": "d-1",
            "view_id": id(9).to_string(),
            "rev": 0,
            "bar_rev": 0,
            "displayed": null,
            "shown": { "type": "none" },
            "point": { "x": 1.0, "y": 2.0 },
        }))
        .unwrap();
        assert_eq!(
            drop,
            DropReport {
                drag_id: "d-1".into(),
                view_id: id(9),
                rev: 0,
                bar_rev: 0,
                displayed: None,
                shown: Picked::None,
                point: PointCss { x: 1.0, y: 2.0 },
            }
        );
    }

    #[test]
    fn the_drawn_report_reads_with_or_without_its_optional_fields() {
        let full: DrawnReport = serde_json::from_value(json!({
            "rev": 3,
            "bar_rev": 2,
            "displayed": id(1).to_string(),
            "shown": { "type": "reorder", "index": 2 },
            "point": { "x": 150.0, "y": 10.0 },
            "scroll": 40.5,
            "left_before_end_ms": 3.25,
        }))
        .unwrap();
        assert_eq!(
            full,
            DrawnReport {
                rev: 3,
                bar_rev: 2,
                displayed: Some(id(1)),
                shown: Picked::Reorder { index: 2 },
                point: PointCss { x: 150.0, y: 10.0 },
                scroll: Some(40.5),
                left_before_end_ms: Some(3.25),
            }
        );
        // 떠남 없음 · 스크롤 없음 — `null` 과 빠진 키가 같다.
        let bare: DrawnReport = serde_json::from_value(json!({
            "rev": 0,
            "bar_rev": 0,
            "displayed": null,
            "shown": { "type": "none" },
            "point": { "x": 1.0, "y": 2.0 },
        }))
        .unwrap();
        assert_eq!((bare.scroll, bare.left_before_end_ms), (None, None));
        assert_eq!(
            serde_json::to_value(&bare).unwrap()["left_before_end_ms"],
            json!(null)
        );
    }
}
