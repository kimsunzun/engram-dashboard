//! 창 크롬 측정 — 웹뷰가 보고할 탭 끌기 영역 · 캔버스 원점(TRD S21-slot-docking §4). 탭 끌기(`dock::targets` ·
//! `dock::session`)의 재료다. 계약: 웹뷰가 `report_dock_chrome`(`commands::dock`)으로 보고하면 셸은 이 값을 창 항목
//! (`WindowTabs`) — 대기 창 명부의 창이면 명부(`dock::parked`) — 에 측정으로 두고, 측정이라 `version` 을 올리지도
//! 알리지도 저장하지도 않는다(ADR-0227 의 보고 규칙).
//!
//! 좌표는 전부 그 창 웹뷰의 CSS px 다(소수 허용 · 창 안쪽 = 웹뷰 뷰포트 왼쪽 위 원점). ts-rs 로
//! `src-tauri/bindings/` 에 굽는다.

use ts_rs::TS;

/// 점 — 그 창 안쪽(웹뷰 뷰포트) 왼쪽 위 원점의 CSS px.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct PointCss {
    pub x: f64,
    pub y: f64,
}

/// 사각형 — 그 창 안쪽 원점의 CSS px 경계 꼴(`getBoundingClientRect` 의 left · top · right · bottom).
/// `x0 <= x1` · `y0 <= y1` 이어야 한다. 셸은 반열림(왼쪽 · 위 포함, 오른쪽 · 아래 제외)으로 읽는다.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct RectCss {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// 창 하나의 크롬 재기 — 웹뷰가 평소에 `report_dock_chrome` 으로 보고할 값(TRD §4).
///
/// - `canvas_origin`: 탭 캔버스의 왼쪽 위. 셸은 커서를 이 원점의 캔버스 좌표로 옮겨 판정하고(`dock::targets`), 끄는
///   창의 원래 크기를 OS 안쪽 크기 − 이 값으로 센다(§5-4).
/// - `tab_bar`: 탭 끌기 영역 사각형(C3) — x = 첫 탭 왼쪽 ~ 그 + 끌기 영역 폭(§5-1) · y = 탭바 줄. `tab_drag_begin` 의
///   `area` 와 같은 사각형이어야 한다 — 셸은 둘에서 같은 탭바 구역(`dock::strip`)을 세어 웹뷰의 「나갔나」와 떼어진
///   창의 「닿았나」를 같은 사각형으로 맞춘다(PRD D19). 떼어진 창은 이 왼쪽 위를 기준으로 놓는다(§5-5).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct DockChrome {
    pub canvas_origin: PointCss,
    pub tab_bar: RectCss,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_chrome_reads_what_the_webview_sends() {
        let json = serde_json::json!({
            "canvas_origin": { "x": 0.0, "y": 28.5 },
            "tab_bar": { "x0": 8.0, "y0": 2.0, "x1": 608.25, "y1": 30.0 },
        });
        let c: DockChrome = serde_json::from_value(json).unwrap();
        assert_eq!(
            c,
            DockChrome {
                canvas_origin: PointCss { x: 0.0, y: 28.5 },
                tab_bar: RectCss {
                    x0: 8.0,
                    y0: 2.0,
                    x1: 608.25,
                    y1: 30.0,
                },
            }
        );
    }
}
