//! 창 크롬 측정 — 웹뷰가 보고할 탭바 · 탭 머리 · 캔버스 원점(TRD S21-slot-docking §1-5). 도킹 판정(`dock::hit`)의
//! 재료다. 계약: 웹뷰가 `report_dock_chrome`(`commands::dock`)으로 보고하면 셸은 이 값을 창 항목(`WindowTabs`)에
//! 측정으로 두고, 측정이라 `version` 을 올리지도 알리지도 저장하지도 않는다(ADR-0227 의 보고 규칙).
//!
//! 좌표는 전부 그 창 웹뷰의 CSS px 다(소수 허용 · 창 안쪽 = 웹뷰 뷰포트 왼쪽 위 원점). ts-rs 로
//! `src-tauri/bindings/` 에 굽는다.

use ts_rs::TS;
use uuid::Uuid;

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

/// 탭 머리 하나 — x 는 탭바 **내용 좌표**(잰 뷰포트 x + 그때 탭바 `scrollLeft` = 스크롤 0 일 때의 뷰포트 x), y 는
/// 뷰포트다. 탭바 상자 밖으로 밀려난 머리도 싣는다. 화면 x = 내용 x − 지금 `scrollLeft`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct TabHeader {
    #[ts(type = "string")]
    pub view_id: Uuid,
    pub rect: RectCss,
}

/// 창 하나의 판정 재료 — 웹뷰가 평소에 `report_dock_chrome` 으로 보고할 값(TRD §1-5).
///
/// - `canvas_origin`: 탭 캔버스의 왼쪽 위.
/// - `tab_bar`: 보이는 탭바 상자(스크롤 상자 자신 · 뷰포트 — 스크롤로 안 움직인다).
/// - `tabs`: 탭 머리 전부(x = 내용 좌표 — [`TabHeader`]) — 순서는 보지 않는다(셸이 가운데 x 로 센다). 끄는 탭의
///   머리가 남아 있어도 된다 — 셸이 id 로 거른다(크롬 보고는 떼어진 표시보다 늦게 올 수 있다).
/// - `scroll`: 잰 때의 탭바 `scrollLeft`(CSS px). 없음(키 없음 · `null`) = 0 — 셸은 끌기 보고에 스크롤이 없을 때만
///   이 값으로 판정한다(TRD §1-5).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct DockChrome {
    pub canvas_origin: PointCss,
    pub tab_bar: RectCss,
    pub tabs: Vec<TabHeader>,
    pub scroll: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_chrome_reads_what_the_webview_sends() {
        let view = Uuid::from_u128(7);
        let json = serde_json::json!({
            "canvas_origin": { "x": 0.0, "y": 28.5 },
            "tab_bar": { "x0": 0.0, "y0": 0.0, "x1": 640.0, "y1": 28.0 },
            "tabs": [{ "view_id": view.to_string(), "rect": { "x0": 4.0, "y0": 0.0, "x1": 120.25, "y1": 28.0 } }],
        });
        let c: DockChrome = serde_json::from_value(json).unwrap();
        assert_eq!(
            c,
            DockChrome {
                canvas_origin: PointCss { x: 0.0, y: 28.5 },
                tab_bar: RectCss {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 640.0,
                    y1: 28.0,
                },
                tabs: vec![TabHeader {
                    view_id: view,
                    rect: RectCss {
                        x0: 4.0,
                        y0: 0.0,
                        x1: 120.25,
                        y1: 28.0,
                    },
                }],
                scroll: None,
            }
        );
    }

    #[test]
    fn dock_chrome_scroll_is_optional_and_null_reads_as_absent() {
        let base = serde_json::json!({
            "canvas_origin": { "x": 0.0, "y": 28.0 },
            "tab_bar": { "x0": 0.0, "y0": 0.0, "x1": 300.0, "y1": 28.0 },
            "tabs": [],
        });
        let read = |extra: Option<serde_json::Value>| {
            let mut v = base.clone();
            if let Some(s) = extra {
                v["scroll"] = s;
            }
            serde_json::from_value::<DockChrome>(v).unwrap().scroll
        };
        assert_eq!(read(None), None, "키 없음");
        assert_eq!(read(Some(serde_json::Value::Null)), None);
        assert_eq!(read(Some(serde_json::json!(120.5))), Some(120.5));
    }
}
