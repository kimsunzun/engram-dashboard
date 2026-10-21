//! 예비 창 웹뷰 ↔ 셸 wire(TRD S21-slot-docking §3-9) — ts-rs 가 `src-tauri/bindings/` 로 굽는다.
//! 창 역할 질의 `window_role` 의 답과 셸 → 웹뷰 사건 [`EVT_SPARE_ADOPTED`] 가 여기 있다.

use ts_rs::TS;

/// 예비 창이 모델에 들었다 — 셸 → 그 창 웹뷰(대상 = 그 창 label) · 몸 없음(`()` → `null`). 받으면 보통 모드로
/// 그린다(웹뷰 쪽 이름 = `spareMode.ts` 의 `EVT_SPARE_ADOPTED`). 마운트 때의 모드는 `window_role` 이 준다 — 그 뒤
/// 바뀜만 이 사건으로 온다. ★지금 내는 쪽이 없다★ — 예열 창 채택(TRD S21-slot-docking §3-9 · U4d′)이 낸다.
pub const EVT_SPARE_ADOPTED: &str = "spare:adopted";

/// 부른 창의 지금 역할(`window_role` 의 답).
///
/// - `model`: 탭 모델의 창 — 보통으로 그린다.
/// - `spare`: 예비 창 풀의 창(숨긴 예비 창) — 대기 모드로 그린다(빈 틀 · 0탭 자가닫힘 끔 · 끌어오기 재시도 없음 ·
///   재기 보고는 그대로). 그 뒤 보통 모드로의 바뀜은 `spare:adopted` 가 알린다.
/// - `unknown`: 탭 모델에도 풀에도 없다(닫기 요청에서 모델이 먼저 지운 팝아웃 등).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum WindowRole {
    Model,
    Spare,
    Unknown,
}

/// `window_role` 의 답.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct WindowRoleReply {
    pub role: WindowRole,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // 웹뷰(`spareMode.ts`)가 읽는 낱말 — 한쪽만 바뀌면 예비 창이 조용히 보통 모드로 그려진다.
    #[test]
    fn the_role_reply_carries_snake_case_words_and_nothing_else() {
        for (role, word) in [
            (WindowRole::Model, "model"),
            (WindowRole::Spare, "spare"),
            (WindowRole::Unknown, "unknown"),
        ] {
            assert_eq!(
                serde_json::to_value(WindowRoleReply { role }).unwrap(),
                json!({ "role": word })
            );
        }
    }
}
