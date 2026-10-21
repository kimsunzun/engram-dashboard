//! 놓기 명령 `tab_drop` · 창 사실 `window:frame` 의 웹뷰 ↔ 셸 wire 타입 — ts-rs 가 `src-tauri/bindings/` 로 굽는다.
//! 좌표 · 크기는 그 창 뷰포트의 CSS px(논리)다. 셸이 읽는 커서 · 입력 상태는 여기 실리지 않는다.
// TRD S21-slot-docking §3-1 · §3-8

use ts_rs::TS;
use uuid::Uuid;

use crate::layout::bands::{PointCss, SizeCss};
use crate::layout::dock::DockSide;
use crate::layout::geometry::Insets;

/// `tab_drop` 의 인자 — 끌기 하나에 많아야 둘(출처 dragend 의 `drag_end` · 받은 창의 `drop`)이 온다. 부른 창 label 은
/// 인자가 아니라 Tauri 가 넣어 주는 창에서 꺼낸다.
///
/// - `drag`: 출처가 dragstart 에 뽑는 끌기 id — 셸 장부의 열쇠다(같은 끌기의 두 번째 처리는 `duplicate`).
/// - `view`: 끄는 탭.
/// - `from`: dragstart 때 그 탭의 창 label — 결과 알림(`tab_drop:done`)을 받는 창이다. 그 탭이 그사이 다른 창으로
///   갔으면 셸은 지금 창에서 옮긴다.
/// - `initial`: 누르기 전 `from` 창의 모델 활성 탭 — `view` 가 아니고 끄는 탭이 놓을 때 `from` 에 있으면 놓은 뒤
///   `from` 에 남은 탭 중 활성이 된다. 그 밖(`null` · `view` 와 같음 · 끄는 탭이 다른 창에 있음)은 셸 규칙(오른쪽
///   이웃 → 왼쪽)이다.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct TabDrop {
    #[ts(type = "string")]
    pub drag: Uuid,
    #[ts(type = "string")]
    pub view: Uuid,
    pub from: String,
    #[ts(type = "string | null")]
    pub initial: Option<Uuid>,
    pub to: DropTo,
}

/// 놓을 곳 — 셸은 놓기가 본 상태와 견주지 않는다. 가리킨 것이 놓을 때 있으면 지금 상태에 맞춰 적용하고, 없으면
/// `rejected` 다.
///
/// - `tab_bar`: 부른 창의 탭바. `index` = 결과 탭 목록에서의 자리 — 셸이 지금 목록(끄는 탭을 뺀 것)에 넣고, 넘치면
///   맨 뒤다. 부른 창이 없으면 `WindowNotFound`.
/// - `seam`: 부른 창에 보이던 탭 `target` 의 이음새(`side` 철자 = [`DockSide`] variant 이름). `share` = 감싼 자리에서
///   끄는 탭이 차지하는 몫(`RATIO_MIN(0.01) ≤ share ≤ 0.5` — 밖이면 `InvalidShare`). `target` 의 지금 배치에 붙인다 —
///   그사이 나뉘었거나 다른 창으로 갔어도 `target` 과 기준 칸이 있으면 붙고, 없으면 `TabNotFound` · `SlotNotFound`(기준
///   칸이 다른 탭으로 갔으면 `SlotNotInTab`).
/// - `new_window`: 그 탭만 든 창. `size` = 새 창 안쪽 크기(유한 양수가 아니면 셸 기본 크기 · `null` = 탭 하나뿐인
///   팝아웃 — 새 창 없이 그 창을 크기 그대로 옮긴다) · `grab` = 새 창 뷰포트에서 커서가 올 점.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum DropTo {
    TabBar {
        index: u32,
    },
    Seam {
        #[ts(type = "string")]
        target: Uuid,
        anchor: DropAnchor,
        side: DockSide,
        share: f64,
    },
    NewWindow {
        by: NewWindowBy,
        size: Option<SizeCss>,
        grab: PointCss,
    },
}

/// 새 창 놓기를 보낸 쪽.
///
/// - `drop`: 우리 웹뷰의 drop — 놓기가 확실하다.
/// - `drag_end`: 출처의 dragend(`dropEffect` 가 무엇이든 늘 하나) — 셸이 끌기가 끝난 뒤의 입력 상태 · 커서 밑 맨 위
///   창으로 취소 · 무시 · 새 창을 가른다. 같은 끌기의 `drop` 이 따로 오면 그것이 이긴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum NewWindowBy {
    Drop,
    DragEnd { effect: EndEffect },
}

/// 출처 dragend 의 `dropEffect` — wire 값은 그 문자열 그대로(`none` · `move`). `move` = 어딘가(우리 웹뷰 · 다른 앱)가
/// 놓기를 받았다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EndEffect {
    None,
    Move,
}

/// 이음새의 기준 — `slot` = 대상 탭의 그 칸 · `outer` = 대상 탭 전체(바깥 변).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum DropAnchor {
    Slot {
        #[ts(type = "string")]
        id: Uuid,
    },
    Outer,
}

/// `tab_drop` 의 답. `duplicate` 밖의 결과는 출처 창(`from`)에 [`TabDropDone`] 으로도 간다.
///
/// - `applied`: 옮겼다(같은 창 같은 자리면 바뀐 것 없이). `window` = 옮긴 탭이 든 창 · `reused` = 새 창 없이 출처 창을
///   옮겼다(모델은 그대로).
/// - `canceled`: 취소다(Esc · 주 버튼을 쥔 채 끝남 · 커서를 못 읽음) — 셸은 모델을 쓰지 않았다.
/// - `ignored`: 아무 일 없다 — 셸은 모델을 쓰지 않았다. `why` = `no-cursor`(drop 인데 커서를 못 읽음) ·
///   `lost-drop`(우리 창 위에서 떼었는데 받은 drop 이 없음 — 제목 표시줄 · 테두리 포함) · `accepted-ours`(우리 창이
///   받음 — 그 창의 `drop` 이 따로 온다) · `accepted-unknown`(어딘가가 받았는데 커서 밑 창을 못 읽음).
/// - `duplicate`: 같은 끌기를 이미 처리했다 — 이 호출은 알림도 없다. 같은 끌기의 `drag_end` 새 창이 진행 중일 때 닿은
///   `drop` 도 이것으로 답하되 셸이 맡아 둔다 — 그 새 창이 거절되면 셸이 그 `drop` 을 이어 처리하고 그 결과는
///   [`TabDropDone`] 으로만 간다.
/// - `rejected`: 모델이 거절했다 — 모델 그대로. `kind` = 거절 종류 낱말(`TabNotFound` · `WindowNotFound` ·
///   `SlotNotFound` · `ZeroArea` · `WindowOpen` 따위 — 셸 도킹 오류 문구의 괄호 속 낱말) · `reason` = 사람이 읽는 문장.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum TabDropReply {
    Applied { window: String, reused: bool },
    Canceled,
    Ignored { why: String },
    Duplicate,
    Rejected { kind: String, reason: String },
}

/// 사건 `tab_drop:done` 의 몸 — `tab_drop` 처리마다 출처 창(명령의 `from`)에 많아야 한 번. 내지 않는 것 = `duplicate` ·
/// 거절된 `drag_end` 새 창 중 그 거절 전에 같은 끌기의 `drop` 을 맡아 둔 것(그 `drop` 의 결과가 대신 간다) · 그 창이
/// 닫혔음. 적용이면 그 쓰기의 모델 알림(`layout:updated` · `window:tabs-updated`) 뒤에 온다. 같은 `drag` 에 둘이 올
/// 수 있다(무시된 `drag_end` · 맡은 것 없이 거절된 `drag_end` 새 창 뒤 이기는 `drop`) — 뒤의 것이 그 끌기의 결과다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct TabDropDone {
    #[ts(type = "string")]
    pub drag: Uuid,
    pub outcome: DropOutcome,
}

/// [`TabDropDone`] 의 결과 — 뜻은 [`TabDropReply`] 의 같은 이름.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DropOutcome {
    Applied,
    Canceled,
    Ignored,
    Rejected,
}

/// 창 사실 — 셸 → 그 창 사건 `window:frame`(값이 바뀔 때만) · 마운트 때 당기는 `window_frame` 의 답.
///
/// - `restored`: 그 창의 보통(복원) 안쪽 크기 — 최대화 중이어도 최대화 전 크기다. 아직 모르면 `null`.
/// - `maximized`: 최대화돼 있나.
/// - `insets`: 창 틀 = 바깥 사각형 − 안쪽 사각형(CSS px — 그 창 배율로 나눈 값 · 보이지 않는 크기 조절 테두리 포함).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct WindowFrame {
    pub restored: Option<SizeCss>,
    pub maximized: bool,
    pub insets: Insets,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn a_seam_drop_reads_what_the_webview_sends() {
        let sent = json!({
            "drag": id(1), "view": id(2), "from": "main", "initial": id(3),
            "to": {
                "type": "seam", "target": id(4), "anchor": { "type": "slot", "id": id(5) },
                "side": "Left", "share": 0.25,
            },
        });
        let drop: TabDrop = serde_json::from_value(sent).unwrap();
        assert_eq!(
            drop,
            TabDrop {
                drag: id(1),
                view: id(2),
                from: "main".into(),
                initial: Some(id(3)),
                to: DropTo::Seam {
                    target: id(4),
                    anchor: DropAnchor::Slot { id: id(5) },
                    side: DockSide::Left,
                    share: 0.25,
                },
            }
        );
    }

    #[test]
    fn tab_bar_and_new_window_drops_read_what_the_webview_sends() {
        let tab_bar: DropTo =
            serde_json::from_value(json!({ "type": "tab_bar", "index": 2 })).unwrap();
        assert_eq!(tab_bar, DropTo::TabBar { index: 2 });
        let ended: DropTo = serde_json::from_value(json!({
            "type": "new_window", "by": { "type": "drag_end", "effect": "move" },
            "size": null, "grab": { "x": 12.5, "y": 8.0 },
        }))
        .unwrap();
        assert_eq!(
            ended,
            DropTo::NewWindow {
                by: NewWindowBy::DragEnd {
                    effect: EndEffect::Move
                },
                size: None,
                grab: PointCss { x: 12.5, y: 8.0 },
            }
        );
        let dropped: DropTo = serde_json::from_value(json!({
            "type": "new_window", "by": { "type": "drop" },
            "size": { "w": 800.0, "h": 600.0 }, "grab": { "x": 0.0, "y": 0.0 },
        }))
        .unwrap();
        assert!(matches!(
            dropped,
            DropTo::NewWindow {
                by: NewWindowBy::Drop,
                size: Some(SizeCss { w, h }),
                ..
            } if w == 800.0 && h == 600.0
        ));
        let outer: DropAnchor = serde_json::from_value(json!({ "type": "outer" })).unwrap();
        assert_eq!(outer, DropAnchor::Outer);
        let none: EndEffect = serde_json::from_value(json!("none")).unwrap();
        assert_eq!(none, EndEffect::None);
    }

    #[test]
    fn replies_and_done_spell_the_wire_names() {
        assert_eq!(
            serde_json::to_value(TabDropReply::Applied {
                window: "slot-popup-2".into(),
                reused: false
            })
            .unwrap(),
            json!({ "type": "applied", "window": "slot-popup-2", "reused": false })
        );
        assert_eq!(
            serde_json::to_value(TabDropReply::Ignored {
                why: "lost-drop".into()
            })
            .unwrap(),
            json!({ "type": "ignored", "why": "lost-drop" })
        );
        assert_eq!(
            serde_json::to_value(TabDropReply::Duplicate).unwrap(),
            json!({ "type": "duplicate" })
        );
        assert_eq!(
            serde_json::to_value(TabDropReply::Rejected {
                kind: "SlotNotFound".into(),
                reason: "슬롯 없음: x (SlotNotFound)".into()
            })
            .unwrap(),
            json!({ "type": "rejected", "kind": "SlotNotFound", "reason": "슬롯 없음: x (SlotNotFound)" })
        );
        assert_eq!(
            serde_json::to_value(TabDropDone {
                drag: id(9),
                outcome: DropOutcome::Canceled
            })
            .unwrap(),
            json!({ "drag": id(9), "outcome": "canceled" })
        );
        assert_eq!(
            serde_json::to_value(WindowFrame {
                restored: Some(SizeCss { w: 640.0, h: 480.0 }),
                maximized: true,
                insets: Insets {
                    t: 31.0,
                    r: 8.0,
                    b: 8.0,
                    l: 8.0
                },
            })
            .unwrap(),
            json!({
                "restored": { "w": 640.0, "h": 480.0 }, "maximized": true,
                "insets": { "t": 31.0, "r": 8.0, "b": 8.0, "l": 8.0 },
            })
        );
    }
}
