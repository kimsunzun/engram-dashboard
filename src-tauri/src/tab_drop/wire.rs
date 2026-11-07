//! The webview ↔ shell wire types of the drop command `tab_drop` · the window fact `window:frame` — ts-rs bakes them into `src-tauri/bindings/`.
//! Coordinates · sizes are CSS px (logical) of that window's viewport. The cursor · input state the shell reads is not carried here.
// TRD S21-slot-docking §3-1 · §3-8

use ts_rs::TS;
use uuid::Uuid;

use crate::layout::bands::{PointCss, SizeCss};
use crate::layout::dock::DockSide;
use crate::layout::geometry::Insets;

/// The argument of `tab_drop` — at most two come per drag (the source dragend's `drag_end` · the receiving window's `drop`). The calling window's label is taken
/// from the window Tauri injects, not from the argument.
///
/// - `drag`: the drag id the source draws at dragstart — the key of the shell ledger (the second processing of the same drag is `duplicate`).
/// - `view`: the dragged tab.
/// - `from`: the label of that tab's window at dragstart — the window that receives the result notification (`tab_drop:done`). If that tab went to another window
///   in the meantime, the shell moves it from the current window.
/// - `initial`: the model's active tab of the `from` window before pressing — if it is not `view` and the dragged tab is in `from` at drop time, it becomes the active one
///   among the tabs left in `from` after the drop. Otherwise (`null` · same as `view` · the dragged tab is in another window) it is the shell rule (right
///   neighbor → left).
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

/// Where to drop — the shell does not compare against the state the drop saw. If what was pointed at exists at drop time, it is applied matched to the current state, and if not,
/// it is `rejected`.
///
/// - `tab_bar`: the calling window's tab bar. `index` = the position in the resulting tab list — the shell inserts it into the current list (without the dragged tab), and if it overflows, it is
///   the very end. If there is no calling window, `WindowNotFound`.
/// - `seam`: the seam of tab `target` that was visible in the calling window (`side` spelling = [`DockSide`] variant name). `share` = the share the dragged tab takes
///   of the wrapped spot (`RATIO_MIN(0.01) ≤ share ≤ 0.5` — outside it, `InvalidShare`). It attaches to `target`'s current layout —
///   even if it was split in the meantime or went to another window, it attaches if `target` and the reference area exist, and if not, `TabNotFound` · `SlotNotFound` (if the reference
///   area went to another tab, `SlotNotInTab`).
/// - `new_window`: a window holding only that tab. `size` = the new window's inner size (if not a finite positive number, the shell default size · `null` = a popout with only
///   one tab — move that window as is at its size with no new window) · `grab` = the point in the new window's viewport where the cursor will land.
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

/// The side that sent the new-window drop.
///
/// - `drop`: a drop in our webview — the drop is certain.
/// - `drag_end`: the source's dragend (always exactly one whatever `dropEffect` is) — the shell distinguishes cancel · ignore · new window by the input state after the drag ended · the topmost
///   window under the cursor. If the same drag's `drop` comes separately, it wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum NewWindowBy {
    Drop,
    DragEnd { effect: EndEffect },
}

/// The `dropEffect` of the source dragend — the wire value is that string as is (`none` · `move`). `move` = somewhere (our webview · another app) received
/// the drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EndEffect {
    None,
    Move,
}

/// The reference of the seam — `slot` = that area of the target tab · `outer` = the whole target tab (outer edge).
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

/// The answer of `tab_drop`. Results other than `duplicate` also go to the source window (`from`) as [`TabDropDone`].
///
/// - `applied`: moved (with nothing changed if the same window and same position). `window` = the label of the window holding the moved tab · `window_id` · `number`
///   = that window's persistent id · number (the meaning is the same as for `NewWindow` — that window's, whether new or existing. The value to pass as a window argument is
///   `window_id`) · `reused` = the source window was moved with no new window (the model is unchanged). `window_id` · `number` are together
///   `null` or together values — `null` = on the reuse path, that window was closed while moving it.
/// - `canceled`: a cancel (Esc · ended while holding the primary button · cursor unreadable) — the shell did not write the model.
/// - `ignored`: nothing happens — the shell did not write the model. `why` = `no-cursor` (a drop but the cursor could not be read) ·
///   `lost-drop` (released over our window but no drop was received — including the title bar · border) · `accepted-ours` (our window
///   received it — that window's `drop` comes separately) · `accepted-unknown` (somewhere received it but the window under the cursor could not be read).
/// - `duplicate`: the same drag was already processed — this call has no notification either. A `drop` that arrives while the same drag's `drag_end` new window is in progress
///   is also answered with this, but the shell parks it — if that new window is rejected, the shell continues that `drop` and its result goes
///   only by [`TabDropDone`].
/// - `rejected`: the model rejected it — the model is unchanged. `kind` = the rejection kind word (`TabNotFound` · `WindowNotFound` ·
///   `SlotNotFound` · `ZeroArea` · `WindowOpen`, etc. — the word in the parentheses of the shell docking error text) · `reason` = a human-readable sentence.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum TabDropReply {
    Applied {
        window: String,
        window_id: Option<String>,
        number: Option<u32>,
        reused: bool,
    },
    Canceled,
    Ignored {
        why: String,
    },
    Duplicate,
    Rejected {
        kind: String,
        reason: String,
    },
}

/// The body of the event `tab_drop:done` — at most once to the source window (the command's `from`) per `tab_drop` processing. What is not emitted = `duplicate` ·
/// a rejected `drag_end` new window that had parked the same drag's `drop` before that rejection (that `drop`'s result goes instead) · that window
/// having been closed. If applied, it comes after that write's model notification (`layout:updated` · `window:tabs-updated`). Two can come for the same `drag`
/// (after an ignored `drag_end` · a `drag_end` new window rejected with nothing parked, the winning `drop`) — the later one is that drag's result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct TabDropDone {
    #[ts(type = "string")]
    pub drag: Uuid,
    pub outcome: DropOutcome,
}

/// The result of [`TabDropDone`] — the meaning is the same-named one of [`TabDropReply`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DropOutcome {
    Applied,
    Canceled,
    Ignored,
    Rejected,
}

/// Window fact — shell → that window's event `window:frame` (only when the value changes) · the answer of `window_frame`, pulled at mount.
///
/// - `restored`: that window's normal (restored) inner size — even while maximized it is the size before maximize. `null` if not yet known.
/// - `maximized`: is it maximized.
/// - `insets`: window frame = outer rectangle − inner rectangle (CSS px — divided by that window's scale factor · including the invisible resize border).
/// - `focused`: is that window the OS foreground window (for dimming the title bar — TRD S26-window-chrome §6-5). The value pushed by that window's focus event is
///   that event's value, and a pull · any other push is the OS foreground-window decision at that time — the two values may briefly disagree (unverified · S12).
/// - `caption`: the title-bar chrome shape — an OS constant, so the same for every window · throughout a run.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
// ADR-0303 — Chrome-style title bar
pub struct WindowFrame {
    pub restored: Option<SizeCss>,
    pub maximized: bool,
    pub insets: Insets,
    pub focused: bool,
    pub caption: CaptionChrome,
}

/// The title-bar chrome shape (TRD S26-window-chrome §3-2 · §7-11).
///
/// - `buttons`: the web draws the minimize · maximize · close buttons. False = the OS draws them (macOS's share — always true for now).
/// - `leading_inset`: the width the web leaves empty at the left of the title bar (CSS px — the OS draws buttons there · always 0 for now).
/// - `snap_overlay`: is this a build where the shell turned on the snap-layout overlay — only when true does the web report the maximize-button rectangle (only a build made with the shell feature
///   `snap-overlay` is true · off by default · §8-4).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, TS)]
#[ts(export)]
pub struct CaptionChrome {
    pub buttons: bool,
    pub leading_inset: f64,
    pub snap_overlay: bool,
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
                window_id: Some("w-2".into()),
                number: Some(2),
                reused: false
            })
            .unwrap(),
            json!({
                "type": "applied", "window": "slot-popup-2", "window_id": "w-2", "number": 2,
                "reused": false,
            })
        );
        assert_eq!(
            serde_json::to_value(TabDropReply::Applied {
                window: "slot-popup-2".into(),
                window_id: None,
                number: None,
                reused: true
            })
            .unwrap(),
            json!({
                "type": "applied", "window": "slot-popup-2", "window_id": null, "number": null,
                "reused": true,
            }),
            "닫힌 재사용 창 — 칸은 빠지지 않고 null 이다"
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
                focused: true,
                caption: CaptionChrome {
                    buttons: true,
                    leading_inset: 0.0,
                    snap_overlay: false,
                },
            })
            .unwrap(),
            json!({
                "restored": { "w": 640.0, "h": 480.0 }, "maximized": true,
                "insets": { "t": 31.0, "r": 8.0, "b": 8.0, "l": 8.0 },
                "focused": true,
                "caption": { "buttons": true, "leading_inset": 0.0, "snap_overlay": false },
            })
        );
    }
}
