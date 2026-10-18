//! 슬롯 도킹의 웹뷰 ↔ 셸 wire 타입(TRD S21-slot-docking §4) — ts-rs 가 `src-tauri/bindings/` 로 굽는다.
//!
//! 탭 끌기 invoke 넷(`tab_drag_begin` · `tab_drag_reorder` · `tab_drag_leave` · `tab_drag_end`)의 인자 · 답과 셸 →
//! 웹뷰 사건 `dock:drag` · `dock:feed` 의 몸이 여기 있다. 좌표는 전부 부른(받는) 창 뷰포트의 CSS px 다. 세션 번호
//! `session` 은 앱 실행마다 1 부터 단조로 늘고 0 을 내지 않는다 — 웹뷰는 그 번호로 끝난 끌기의 늦은 사건을 가른다.

use ts_rs::TS;
use uuid::Uuid;

use crate::layout::chrome::{PointCss, RectCss};

/// 부른 창의 지금 역할(`window_role` 의 답).
///
/// - `model`: 탭 모델의 창 — 보통으로 그린다.
/// - `parked`: 대기 창 명부의 창(예비 창 · 끄는 중 비워 남긴 창) — 대기 모드로 그린다(빈 틀 · 0탭 자가닫힘 끔 ·
///   끌어오기 재시도 없음 · 재기 보고는 그대로). 그 뒤 모드 바뀜은 `dock:spare`([`SpareEvent`])가 알린다.
/// - `unknown`: 탭 모델에도 명부에도 없다(닫기 요청에서 모델이 먼저 지운 팝아웃 · 안내 창 등).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum WindowRole {
    Model,
    Parked,
    Unknown,
}

/// `window_role` 의 답 — `drag` 는 ★늘 `null` 이다★: 진행 중 세션 번호를 비추는 거울은 끌기 세션 축소안에서
/// 뺐다(다시 읽힌 웹뷰가 끌기 도중에 키 감시를 다시 켜는 몫 — 평소 쓰임 밖 · 추적 밖
/// `.claude/handoff/attachments/trd6-review-2026-10-18/u5-trim-plan.md` §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct WindowRoleReply {
    pub role: WindowRole,
    // ts-rs 의 u64 기본 매핑은 bigint 이나 serde_json 은 number 로 싣는다.
    #[ts(type = "number | null")]
    pub drag: Option<u64>,
}

/// `dock:spare` 의 몸 — 대기 창 명부의 창 웹뷰에 그 창의 모드가 바뀌었음을 알린다(TRD §4 · §2-5). 받는 쪽 = 그 창
/// 웹뷰(대기 모드 켜고 끔).
///
/// 내는 쪽 = 끌기 세션이 그 창을 모델에 들일 때(`adopted`) · 명부로 되돌릴 때(`parked`)뿐이다. 마운트 때의 모드는
/// `window_role` 이 준다 — 그 뒤 바뀜만 이 사건으로 온다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct SpareEvent {
    pub step: SpareStep,
}

/// [`SpareEvent`] 의 단계.
///
/// - `parked`: 명부에 들었다 — 대기 모드로(빈 틀).
/// - `adopted`: 모델에 들었다 — 보통 팝아웃으로.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SpareStep {
    Parked,
    Adopted,
}

/// 탭 안에서 잡은 자리 — 떼어진 창을 커서 밑에 같은 상대 자리로 놓는 재료(조사 A3 · E3).
///
/// - `x_ratio`: 잡은 x ÷ 그 탭 폭(0..1).
/// - `y`: 탭 위에서 잡은 y(CSS px).
/// - `single_x`: 떼어진 창(탭 하나)에서의 잡은 x(CSS px) = `x_ratio` × 그 창 탭 폭 — 탭 폭은 웹뷰 상수라 웹뷰가 셈해
///   싣는다(셸은 그 상수의 사본을 갖지 않는다).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct Grab {
    pub x_ratio: f64,
    pub y: f64,
    pub single_x: f64,
}

/// `tab_drag_begin` 의 인자 — 10 px 문턱을 넘은 순간 한 번.
///
/// - `down`: 누른 점.
/// - `area`: 지금 탭 끌기 영역 사각형(C3) — 셸이 탭바 구역(`zone`)을 셈한다.
/// - `initial_active`: 누르기 전 활성 탭 — 끄는 탭 자신이거나 `null` 이면 「끄는 탭이 활성이었다」(되돌리기가 끄는
///   탭을 다시 활성으로 둔다).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct TabDragBegin {
    #[ts(type = "string")]
    pub view: Uuid,
    pub down: PointCss,
    pub area: RectCss,
    pub grab: Grab,
    #[ts(type = "string | null")]
    pub initial_active: Option<Uuid>,
}

/// `tab_drag_begin` 의 답.
///
/// - `strip`: 탭 둘 이상 — 웹뷰가 탭바 안 순서 이동을 몬다. `zone` = 탭바 구역(이 사각형 밖으로 나가면
///   `tab_drag_leave` · 반열림).
/// - `whole_window`: 탭 하나 — 창째 OS 이동 루프로 따라온다(웹뷰는 키 끝만 보낸다).
/// - `busy`: 다른 끌기가 진행 중이다.
/// - `rejected`: 그 탭을 끌 수 없다(`reason` = 사람이 읽는 사유).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum BeginReply {
    Strip {
        #[ts(type = "number")]
        session: u64,
        zone: RectCss,
    },
    WholeWindow {
        #[ts(type = "number")]
        session: u64,
    },
    Busy,
    Rejected {
        reason: String,
    },
}

/// `tab_drag_reorder` 의 인자 — 탭바 안에서 끄는 탭의 자리가 바뀔 때. `index` = 결과 탭 목록에서의 자리 · `seq` =
/// 그 웹뷰가 이 세션에서 보낸 순서 이동의 번호(1 부터 단조 — 셸은 받은 것 이하를 버린다).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct TabDragReorder {
    #[ts(type = "number")]
    pub session: u64,
    pub seq: u32,
    pub index: u32,
}

/// `tab_drag_leave` 의 인자 — 점이 탭바 구역을 나갔다. 커서는 셸이 읽는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct TabDragLeave {
    #[ts(type = "number")]
    pub session: u64,
}

/// `tab_drag_leave` 의 답.
///
/// - `detached`: 떼었다 — 그 순간부터 이 끌기의 포인터 사건을 버린다(키 끝만 보낸다).
/// - `not_ready`: 아직 뗄 수 없다(예비 창 준비 전 · 놓기를 이미 정했다) — 탭바 안에 머물고 구역 밖 움직임마다 다시
///   청한다.
/// - `ended`: 세션이 끝났다 — 그 끌기를 버린다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LeaveReply {
    Detached,
    NotReady,
    Ended,
}

/// `tab_drag_end` 의 인자. `at` = 놓는 순간 웹뷰가 아는 끄는 탭의 자리(결과 탭 목록 — `reorder.index` 와 같은 뜻) ·
/// 그 탭을 쥔 창이 보낼 때만 싣고 다른 창의 키 감시는 `null`.
///
/// 받는 짝은 다섯이다 — `{drop, pointer}` · `{drop, release}` · `{drop, key}` · `{cancel, key}` ·
/// `{cancel, capture_lost}`. 그 밖의 짝은 셸이 버린다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct TabDragEnd {
    #[ts(type = "number")]
    pub session: u64,
    pub how: EndHow,
    pub src: EndSrc,
    pub at: Option<u32>,
}

/// 끝의 뜻 — `drop` = 지금 자리에 놓는다 · `cancel` = 되돌린다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EndHow {
    Drop,
    Cancel,
}

/// 끝을 본 곳.
///
/// - `pointer`: 탭을 쥔 창의 `pointerup`(포인터 캡처로 몰 때만).
/// - `release`: 셸의 `dock:feed{release}` 에 탭을 쥔 창이 지금 자리(`at`)로 답했다.
/// - `capture_lost`: 탭을 쥔 창이 탭바 안에서 포인터 캡처를 잃었다(`cancel` 과만).
/// - `key`: 그 웹뷰 keydown 또는 앱 전체 키 감시(`Escape` = `cancel` · 그 밖 = `drop`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EndSrc {
    Pointer,
    Release,
    CaptureLost,
    Key,
}

/// `dock:drag` 의 몸 — 끌기 세션이 섰다 · 끝났다. 받는 쪽 = 우리 웹뷰(안내 창 제외) — 앱 전체 키 감시를 켜고 끈다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct TabDragEvent {
    #[ts(type = "number")]
    pub session: u64,
    pub phase: TabDragPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TabDragPhase {
    Active,
    Ended,
}

/// `dock:feed` 의 몸 — 받는 쪽 = 끄는 탭 `view` 를 쥔 창 웹뷰 하나.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct TabDragFeed {
    #[ts(type = "number")]
    pub session: u64,
    #[ts(type = "string")]
    pub view: Uuid,
    pub step: FeedStep,
}

/// [`TabDragFeed`] 의 단계 — 좌표 = 받는 창 뷰포트 CSS px.
///
/// - `attach`: 끄는 탭이 이 창 탭바에 붙었다 — 문턱 없이 곧바로 자리를 셈해 `tab_drag_reorder` 를 보내고, 그 뒤 이
///   끌기의 입력은 셸이 미는 `move` 다. `zone` = 이 창 탭바 구역 · `grab` = 시작 때 받은 값 · `x` · `y` = 지금 커서.
/// - `move`: 커서가 움직였다(주 버튼은 눌린 채).
/// - `release`: 셸이 놓기를 봤다 — 지금 자리를 `at` 으로 실어 `tab_drag_end{drop, release}` 로 답하라.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum FeedStep {
    Attach {
        zone: RectCss,
        grab: Grab,
        x: f64,
        y: f64,
    },
    Move {
        x: f64,
        y: f64,
    },
    Release,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn replies_and_steps_carry_a_snake_case_type_tag() {
        let zone = RectCss {
            x0: 0.0,
            y0: -15.0,
            x1: 500.0,
            y1: 43.0,
        };
        assert_eq!(
            serde_json::to_value(BeginReply::Strip { session: 3, zone }).unwrap(),
            json!({"type": "strip", "session": 3, "zone": {"x0": 0.0, "y0": -15.0, "x1": 500.0, "y1": 43.0}})
        );
        assert_eq!(
            serde_json::to_value(BeginReply::WholeWindow { session: 4 }).unwrap(),
            json!({"type": "whole_window", "session": 4})
        );
        assert_eq!(
            serde_json::to_value(BeginReply::Busy).unwrap(),
            json!({"type": "busy"})
        );
        assert_eq!(
            serde_json::to_value(LeaveReply::NotReady).unwrap(),
            json!("not_ready")
        );
        assert_eq!(
            serde_json::to_value(FeedStep::Move { x: 1.5, y: 2.0 }).unwrap(),
            json!({"type": "move", "x": 1.5, "y": 2.0})
        );
        assert_eq!(
            serde_json::to_value(FeedStep::Release).unwrap(),
            json!({"type": "release"})
        );
    }

    #[test]
    fn an_end_and_a_begin_from_the_webview_read_back() {
        let end: TabDragEnd = serde_json::from_value(json!({
            "session": 7, "how": "cancel", "src": "capture_lost", "at": null
        }))
        .unwrap();
        assert_eq!(
            end,
            TabDragEnd {
                session: 7,
                how: EndHow::Cancel,
                src: EndSrc::CaptureLost,
                at: None,
            }
        );
        let begin: TabDragBegin = serde_json::from_value(json!({
            "view": "00000000-0000-0000-0000-000000000005",
            "down": {"x": 10.0, "y": 12.0},
            "area": {"x0": 4.0, "y0": 0.0, "x1": 504.0, "y1": 28.0},
            "grab": {"x_ratio": 0.25, "y": 12.0, "single_x": 40.0},
            "initial_active": null
        }))
        .unwrap();
        assert_eq!(begin.view, Uuid::from_u128(5));
        assert_eq!(begin.initial_active, None);
    }
}
