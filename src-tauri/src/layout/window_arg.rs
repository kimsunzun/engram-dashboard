//! 창 인자 해석기(TRD S26-window-chrome §4-5) — 버스 동사 · invoke 껍데기가 받은 창 인자 하나를 지금 모델 안 창의
//! label 로 푼다. 받는 꼴 = `main` · 영속 id(재시작을 넘어 같은 창) · 이번 실행의 label. 창 번호는
//! 받지 않는다 — 번호는 닫힌 창의 것을 다시 쓰므로 쥐고 있던 번호가 다른 창을 가리킬 수 있다.
//!
//! 모델(`ViewManager`)만 읽고 Tauri 를 모른다. 부르는 쪽이 레이아웃 락을 잠깐 잡아 넘긴다 — 푼 label 로 적용
//! 서비스를 부르기 전에 락을 놓으므로, 그 사이 창이 닫히면 적용 서비스가 「창 없음」으로 거절한다.

use uuid::Uuid;

use super::manager::{ViewManager, WindowLabel, MAIN_WINDOW_LABEL};

/// 창 인자를 못 푼 까닭. 부르는 쪽이 코드로 옮긴다 — 버스는 `NumberGiven` = `INVALID_ARGUMENT` · `NotFound` =
/// `CONFLICT`(레이아웃 적용 실패와 같은 코드).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WindowArgError {
    /// 숫자만 준 인자 — 창 번호로 읽힌다.
    #[error("창 번호는 창 인자로 받지 않는다 — 번호는 다시 쓰인다. window.list 의 details 에서 그 번호의 window_id 를 찾아 넘기라")]
    NumberGiven,
    /// 지금 모델에 그 창이 없다 — 닫힌 창의 영속 id · 앞 실행의 label · 닫힌 창의 label · 모르는 낱말. 받은 인자를 싣는다.
    #[error("window 없음: {0} — window.list 의 창 label 이나 그 창의 window_id 를 넘기라(label 은 그 실행 안에서만 맞는다)")]
    NotFound(String),
}

/// 창 인자 `arg`(앞뒤 공백은 깎는다)를 모델 안 창의 label 로 푼다.
///
/// - `main` → `main`.
/// - 숫자만(`"2"`) → [`WindowArgError::NumberGiven`] — 영속 id · label 보다 먼저 가른다.
/// - 모델 안 창의 영속 id 와 철자가 같으면 그 창의 label. 저장된 id 는 `state.json` 에서 검사 없이 읽혀 정규 UUID
///   철자가 아닐 수 있다 — 앱이 내준 id(`window.list` · 생성 답)를 그대로 돌려주면 언제나 풀리게 철자 비교가 먼저다.
/// - UUID 로 읽히면(대소문자 · 중괄호 등 다른 철자도) → UUID 값이 같은 영속 id 를 가진 창의 label. 그런 창이
///   없으면 다음 갈래로 넘어간다 — UUID 꼴 label 도 label 로 풀린다.
/// - 그 밖 → 모델에 그 label 의 창이 있으면 그대로(같은 실행 안 별칭). 운영 label 은 `main` 과 팝아웃 접두
///   (`slot-popup-`)뿐이라 이 갈래가 표의 「접두 label」 갈래다. 영속 id 와 label 이 같은 철자면 id 가 이긴다.
/// - 어디에도 없으면 `NotFound` — 닫히는 창의 id · label 도 모델에 없어 여기다.
// ADR-0302 — 창 상태 명령
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

/// 숫자만인 인자를 거절한다. 해석기 밖 갈래도 같은 거절을 먼저 하도록 따로 연다(TRD §4-5 동사 차례 ① —
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

    // 손으로 고친 state.json 이 정규형이 아닌 id 를 남겨도 앱이 내준 그 철자 · 정규 철자 둘 다 그 창이다.
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

    // 닫기 요청이 모델에서 뺀 창(OS 창은 아직 있다 — `closing`)의 영속 id 도 모델 밖이다.
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

    // 앞 실행이 쓴 label — 실행 표식이 없는 옛 꼴 · 다른 실행 표식 — 은 이번 실행의 창을 가리키지 않는다.
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
        assert!(WindowArgError::NumberGiven
            .to_string()
            .contains("window_id"));
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
