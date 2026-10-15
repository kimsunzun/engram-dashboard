//! 도킹 wire 페이로드 — 셸 → 웹뷰 가이드(TRD S21-slot-docking §1-5). ts-rs 로 `src-tauri/bindings/` 에 굽는다.
//! 웹뷰 → 셸 측정(크롬)은 레이아웃 창 항목에 두므로 `layout::chrome` 에 있다.
//!
//! 사각형은 [`PxRectWire`] = 그 창 캔버스의 CSS px 정수(캔버스 왼쪽 위 원점)다. 셸이 계산하고 웹뷰는 좌표 계산
//! 없이 그대로 그린다(ADR-0227 결정 3).

use ts_rs::TS;
use uuid::Uuid;

use crate::layout::dock::DockSide;
use crate::layout::geometry::PxRect;

/// 캔버스 CSS px 정수 사각형 — 캔버스 왼쪽 위 원점의 경계 꼴(`x0 <= x1` · `y0 <= y1` · 폭 = `x1 - x0`). 폭이나
/// 높이가 0 일 수 있다(그릴 것이 없다).
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

/// 띠 하나. `side` = 붙일 쪽 · `anchor` = 붙일 칸(`None` = 레이아웃 전체의 바깥 변) · `rect` = 그릴 사각형.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct Band {
    pub side: DockSide,
    #[ts(type = "string | null")]
    pub anchor: Option<Uuid>,
    pub rect: PxRectWire,
}

/// 한 창의 이음새 가이드(PRD R3–R5).
///
/// - `view_id`: 가이드를 그릴 탭 — 그 창에 지금 보이는 탭.
/// - `bands`: 커서 밑 칸의 유효한 띠. 사각형이 곧 놓았을 때 끼운 것이 차지할 사각형이다(R3). 결과에 면적 0 칸이
///   생길 띠는 빠진다(R8 — 빠진 띠에는 붙지 않는다).
/// - `outer`: 캔버스 네 변의 바깥 띠 그림(두께 `dock::hit::OUTER_BAND_PX`). 결과에 면적 0 칸이 생길 변은 빠진다.
/// - `hot`: 고른 이음새(`dock::hit::choose_seam`)의 결과 사각형(R4 · R5) — 바깥 변이면 레이아웃 전체 그 변의
///   1/3 이다. 커서가 칠하지 않은 바깥 띠 위면 그 밑 칸의 띠일 수 있다. `None` = 이음새 없음(칸 가운데 — 놓으면 새 창).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct GuideView {
    #[ts(type = "string")]
    pub view_id: Uuid,
    pub bands: Vec<Band>,
    pub outer: Vec<Band>,
    pub hot: Option<Band>,
}

/// 탭바 오버레이(PRD R21 · R6b).
///
/// - `insert_x`: 끼울 자리 — 탭바 상자 왼쪽(`DockChrome.tab_bar.x0`)에서 잰 CSS px. `None` = 끼울 자리를 그리지
///   않는다.
/// - `dwell`: 전환을 기다리는 탭 — 그 머리에 진행 표시를 그린다. `None` = 기다리는 탭 없음.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct TabBarHint {
    pub insert_x: Option<f64>,
    #[ts(type = "string | null")]
    pub dwell: Option<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock::hit::SIDE_ORDER;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
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
    fn guide_view_goes_out_in_snake_case_with_string_ids_and_integer_rects() {
        let g = GuideView {
            view_id: id(1),
            bands: vec![Band {
                side: DockSide::Left,
                anchor: Some(id(2)),
                rect: PxRect {
                    x0: 0,
                    y0: 0,
                    x1: 100,
                    y1: 240,
                }
                .into(),
            }],
            outer: vec![],
            hot: None,
        };
        assert_eq!(
            serde_json::to_value(&g).unwrap(),
            serde_json::json!({
                "view_id": id(1).to_string(),
                "bands": [{
                    "side": "Left",
                    "anchor": id(2).to_string(),
                    "rect": { "x0": 0, "y0": 0, "x1": 100, "y1": 240 },
                }],
                "outer": [],
                "hot": null,
            })
        );
        let outer = Band {
            side: DockSide::Bottom,
            anchor: None,
            rect: PxRectWire {
                x0: 0,
                y0: 768,
                x1: 1200,
                y1: 800,
            },
        };
        assert_eq!(
            serde_json::to_value(&outer).unwrap()["anchor"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn tab_bar_hint_absences_are_null() {
        let h = TabBarHint {
            insert_x: None,
            dwell: None,
        };
        assert_eq!(
            serde_json::to_value(&h).unwrap(),
            serde_json::json!({ "insert_x": null, "dwell": null })
        );
    }
}
