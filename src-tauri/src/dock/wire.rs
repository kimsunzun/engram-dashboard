//! 슬롯 도킹의 웹뷰 ↔ 셸 wire 타입(TRD S21-slot-docking §4) — ts-rs 가 `src-tauri/bindings/` 로 굽는다.

use ts_rs::TS;

/// 부른 창의 지금 역할(`window_role` 의 답).
///
/// - `model`: 탭 모델의 창 — 보통으로 그린다.
/// - `parked`: 대기 창 명부의 창(예비 창 · 끄는 중 비워 남긴 창) — 대기 모드로 그린다(빈 틀 · 0탭 자가닫힘 끔 ·
///   끌어오기 재시도 없음 · 재기 보고는 그대로). 모드가 바뀌면 `dock:spare` 가 알린다.
/// - `unknown`: 탭 모델에도 명부에도 없다(닫기 요청에서 모델이 먼저 지운 팝아웃 · 안내 창 등).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum WindowRole {
    Model,
    Parked,
    Unknown,
}

/// `window_role` 의 답 — `drag` = 진행 중 끌기 세션 번호(1 부터 단조). `null` = 진행 중 끌기가 없다 — ★아직 늘
/// `null` 이다★(세션 번호 거울은 TRD §13 U6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, TS)]
#[ts(export)]
pub struct WindowRoleReply {
    pub role: WindowRole,
    // ts-rs 의 u64 기본 매핑은 bigint 이나 serde_json 은 number 로 싣는다.
    #[ts(type = "number | null")]
    pub drag: Option<u64>,
}
