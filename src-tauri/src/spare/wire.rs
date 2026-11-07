//! Spare window webview ↔ shell wire (TRD S21-slot-docking §3-9) — ts-rs bakes it into `src-tauri/bindings/`.
//! The answer of the window role query `window_role` and the shell → webview event [`EVT_SPARE_ADOPTED`] are here.

use ts_rs::TS;

/// The spare window entered the model — shell → that window's webview (target = that window's label) · body = [`SpareAdopted`]. On receiving it, draws in normal mode and
/// then carries the body's adoption number in the first-paint notification after that (`webview_painted`) (the webview-side name = `EVT_SPARE_ADOPTED` of `spareMode.ts`). The mode at mount time
/// is given by `window_role` — only changes after that come through this event. Emitter = commit ④
/// (the commit port of `commands::spare` — TRD S21-slot-docking §3-9 「`committed`」).
pub const EVT_SPARE_ADOPTED: &str = "spare:adopted";

/// Body of the event `spare:adopted` (`EVT_SPARE_ADOPTED`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct SpareAdopted {
    /// Adoption number of that handover (I26) — the shell accepts only those first-paint notifications of that window that carry this number.
    pub generation: u32,
}

/// Current role of the calling window (the answer of `window_role`).
///
/// - `model`: a window of the tab model — drawn normally.
/// - `spare`: a window of the spare window pool (hidden spare window) — drawn in waiting mode (empty frame · 0-tab self-close off · no drag-in retry ·
///   measurement reports as they are). The change to normal mode after that is announced by `spare:adopted`.
/// - `unknown`: in neither the tab model nor the pool (a popout the model deleted first on a close request · a prewarmed window that the shell removes from the pool and closes because it could not set up listening for `spare:adopted`
///   (`adoptable: false`), etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum WindowRole {
    Model,
    Spare,
    Unknown,
}

/// The answer of `window_role`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct WindowRoleReply {
    pub role: WindowRole,
    /// If it is `model` and a window waiting to be shown after adoption, that adoption number (I26 — a page that missed `spare:adopted` announces first paint with this number).
    /// Otherwise `null`.
    pub generation: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Words the webview (`spareMode.ts`) reads — if only one side changes, the spare window is silently drawn in normal mode.
    #[test]
    fn the_role_reply_carries_snake_case_words_and_the_adoption_number() {
        for (role, word) in [
            (WindowRole::Model, "model"),
            (WindowRole::Spare, "spare"),
            (WindowRole::Unknown, "unknown"),
        ] {
            assert_eq!(
                serde_json::to_value(WindowRoleReply {
                    role,
                    generation: None
                })
                .unwrap(),
                json!({ "role": word, "generation": null })
            );
        }
        assert_eq!(
            serde_json::to_value(WindowRoleReply {
                role: WindowRole::Model,
                generation: Some(3)
            })
            .unwrap(),
            json!({ "role": "model", "generation": 3 })
        );
    }

    // The webview echoes this number in the first-paint notification — if the name is off, the adopted window is shown only by the deadline.
    #[test]
    fn the_adopted_event_carries_the_adoption_number() {
        assert_eq!(
            serde_json::to_value(SpareAdopted { generation: 7 }).unwrap(),
            json!({ "generation": 7 })
        );
    }
}
