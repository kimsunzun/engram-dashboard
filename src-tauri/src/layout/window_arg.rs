//! Window argument resolver (TRD S26-window-chrome §4-5) — resolves one window argument received by a bus verb · invoke shell to the label of a window in the current model.
//! Accepted forms = `main` · persistent id (the same window across restarts) · label of this run. A window number is not
//! accepted — numbers of closed windows are reused, so a number held could point to a different window.
//!
//! It only reads the model (`ViewManager`) and knows nothing of Tauri. The caller briefly takes the layout lock and hands it over — the lock is released before calling the apply
//! service with the resolved label, so if the window closes in between, the apply service rejects with 「창 없음」(no such window).

use uuid::Uuid;

use super::manager::{ViewManager, WindowLabel, MAIN_WINDOW_LABEL};

/// Why the window argument could not be resolved. The caller converts it to a code — on the bus `NumberGiven` = `INVALID_ARGUMENT` · `NotFound` =
/// `CONFLICT` (the same code as a layout apply failure).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WindowArgError {
    /// An argument given as digits only — read as a window number.
    #[error("창 번호는 창 인자로 받지 않는다 — 번호는 다시 쓰인다. window.list 의 details 에서 그 번호의 window_id 를 찾아 넘기거나, layout.get 줄의 window(label)를 넘기라")]
    NumberGiven,
    /// No such window in the current model — persistent id of a closed window · label of a previous run · label of a closed window · unknown word. Carries the argument received.
    #[error("window 없음: {0} — window.list 의 창 label 이나 그 창의 window_id 를 넘기라(label 은 그 실행 안에서만 맞는다)")]
    NotFound(String),
}

/// Resolves window argument `arg` (leading and trailing whitespace trimmed) to the label of a window in the model.
///
/// - `main` → `main`.
/// - Digits only (`"2"`) → [`WindowArgError::NumberGiven`] — decided before persistent id · label.
/// - If the spelling equals the persistent id of a window in the model, that window's label. A saved id is read from `state.json` without validation and may not be in canonical UUID
///   spelling — the spelling comparison comes first so that giving back an id the app handed out (`window.list` · creation reply) always resolves.
/// - If it reads as a UUID (also other spellings such as case · braces) → the label of the window whose persistent id has the same UUID value. If there is no such
///   window, it moves on to the next branch — a UUID-shaped label also resolves as a label.
/// - Otherwise → if the model has a window with that label, as is (alias within the same run). The production labels are only `main` and the popout prefix
///   (`slot-popup-`), so this branch is the 「접두 label」(prefix label) branch of the table. If a persistent id and a label have the same spelling, the id wins.
/// - If it is nowhere → `NotFound` — the id · label of a closing window is not in the model either, so it lands here.
// ADR-0302 — window state command
pub fn resolve(mgr: &ViewManager, arg: &str) -> Result<WindowLabel, WindowArgError> {
    let arg = arg.trim();
    if arg == MAIN_WINDOW_LABEL {
        return Ok(MAIN_WINDOW_LABEL.to_string());
    }
    reject_number(arg)?;
    let by_spelling = mgr
        .windows
        .iter()
        .find(|(_, window)| window.window_id == arg);
    if let Some((label, _)) = by_spelling {
        return Ok(label.clone());
    }
    if let Ok(id) = Uuid::parse_str(arg) {
        let by_value = mgr
            .windows
            .iter()
            .find(|(_, window)| Uuid::parse_str(&window.window_id).ok() == Some(id));
        if let Some((label, _)) = by_value {
            return Ok(label.clone());
        }
    }
    if mgr.windows.contains_key(arg) {
        return Ok(arg.to_string());
    }
    Err(WindowArgError::NotFound(arg.to_string()))
}

/// Rejects an argument that is digits only. It is opened separately so that branches outside the resolver also make the same rejection first (TRD §4-5 verb order ① —
/// `window.close { if_empty }`).
pub fn reject_number(arg: &str) -> Result<(), WindowArgError> {
    let arg = arg.trim();
    if !arg.is_empty() && arg.bytes().all(|b| b.is_ascii_digit()) {
        return Err(WindowArgError::NumberGiven);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: &str = "slot-popup-0a1b2c3d-1";

    fn model() -> (ViewManager, String) {
        let mut mgr = ViewManager::new();
        mgr.create_window(LIVE).unwrap();
        let id = mgr.windows[LIVE].window_id.clone();
        (mgr, id)
    }

    #[test]
    fn main_resolves_to_main() {
        let (mgr, _) = model();
        assert_eq!(resolve(&mgr, "main"), Ok("main".to_string()));
        assert_eq!(resolve(&mgr, "  main "), Ok("main".to_string()));
    }

    #[test]
    fn a_window_id_resolves_to_the_label_of_the_window_holding_it() {
        let (mgr, id) = model();
        assert_eq!(resolve(&mgr, &id), Ok(LIVE.to_string()));
        assert_eq!(
            resolve(&mgr, &id.to_uppercase()),
            Ok(LIVE.to_string()),
            "다른 철자도 같은 창"
        );
        assert_eq!(
            resolve(&mgr, &format!("{{{id}}}")),
            Ok(LIVE.to_string()),
            "중괄호 철자"
        );
    }

    // Even if a hand-edited state.json leaves a non-canonical id, both the spelling the app handed out and the canonical spelling are that window.
    #[test]
    fn a_non_canonical_stored_window_id_resolves_by_spelling_and_by_value() {
        let (mut mgr, id) = model();
        for stored in [id.to_uppercase(), format!("{{{id}}}")] {
            mgr.windows.get_mut(LIVE).unwrap().window_id = stored.clone();
            assert_eq!(resolve(&mgr, &stored), Ok(LIVE.to_string()), "{stored}");
            assert_eq!(resolve(&mgr, &id), Ok(LIVE.to_string()), "{stored}");
        }
        mgr.windows.get_mut(LIVE).unwrap().window_id = "w-0".to_string();
        assert_eq!(
            resolve(&mgr, "w-0"),
            Ok(LIVE.to_string()),
            "UUID 가 아닌 저장 id"
        );
    }

    #[test]
    fn an_unknown_window_id_is_not_found() {
        let (mgr, _) = model();
        let ghost = Uuid::new_v4().to_string();
        assert_eq!(resolve(&mgr, &ghost), Err(WindowArgError::NotFound(ghost)));
    }

    // The persistent id of a window that a close request removed from the model (the OS window still exists — `closing`) is also outside the model.
    #[test]
    fn the_window_id_of_a_closed_window_is_not_found() {
        let (mut mgr, id) = model();
        mgr.close_window(LIVE).unwrap();
        assert_eq!(mgr.closing_numbers(), [2], "번호는 아직 쥐고 있다");
        assert_eq!(
            resolve(&mgr, &id),
            Err(WindowArgError::NotFound(id.clone()))
        );
        assert_eq!(
            resolve(&mgr, LIVE),
            Err(WindowArgError::NotFound(LIVE.to_string())),
            "닫힌 창의 label"
        );
    }

    #[test]
    fn a_label_in_the_model_resolves_to_itself() {
        let (mgr, _) = model();
        assert_eq!(resolve(&mgr, LIVE), Ok(LIVE.to_string()));
    }

    #[test]
    fn a_uuid_shaped_label_resolves_to_itself_when_no_window_id_matches() {
        let (mut mgr, _) = model();
        let label = Uuid::new_v4().to_string();
        mgr.create_window(&label).unwrap();
        assert_eq!(resolve(&mgr, &label), Ok(label.clone()));
    }

    #[test]
    fn a_window_id_wins_over_an_identical_label() {
        let (mut mgr, id) = model();
        mgr.create_window(&id).unwrap();
        assert_eq!(resolve(&mgr, &id), Ok(LIVE.to_string()), "정확한 철자");
        assert_eq!(
            resolve(&mgr, &id.to_uppercase()),
            Ok(LIVE.to_string()),
            "UUID 값 일치"
        );
    }

    // A label written by a previous run — the old form with no run marker · a marker of a different run — does not point to a window of this run.
    #[test]
    fn a_label_from_a_previous_run_is_not_found() {
        let (mgr, _) = model();
        for old in [
            "slot-popup-1",
            "slot-popup-ffffffff-1",
            "agent-tree",
            "popup",
        ] {
            assert_eq!(
                resolve(&mgr, old),
                Err(WindowArgError::NotFound(old.to_string())),
                "{old}"
            );
        }
    }

    #[test]
    fn a_bare_number_is_refused_as_a_window_number() {
        let (mgr, _) = model();
        for number in ["1", "2", " 2 ", "02", "999999999999"] {
            assert_eq!(
                resolve(&mgr, number),
                Err(WindowArgError::NumberGiven),
                "{number:?}"
            );
        }
        let message = WindowArgError::NumberGiven.to_string();
        assert!(message.contains("window_id"), "{message}");
        assert!(message.contains("layout.get"), "{message}");
    }

    #[test]
    fn reject_number_passes_everything_that_is_not_only_digits() {
        for arg in ["", " ", "main", "-1", "+2", "2a", "#2", LIVE] {
            assert_eq!(reject_number(arg), Ok(()), "{arg:?}");
        }
        assert_eq!(reject_number("7"), Err(WindowArgError::NumberGiven));
    }

    #[test]
    fn not_found_names_the_argument() {
        let (mgr, _) = model();
        let err = resolve(&mgr, "slot-popup-9").unwrap_err();
        assert!(err.to_string().contains("slot-popup-9"), "{err}");
    }
}
