//! 슬롯 도킹의 대기 창 웹뷰 ↔ 셸 wire 타입(TRD S21-slot-docking §4 · §2-5) — ts-rs 가 `src-tauri/bindings/` 로 굽는다.
//! 창 역할 질의 `window_role` 의 답과 셸 → 웹뷰 사건 `dock:spare` 의 몸이 여기 있다.

use ts_rs::TS;

// TODO(U4c′-2): 아래 doc 셋을 고친다 — `WindowRole` 의 「안내 창」(걷었다) · 「끄는 중 비워 남긴 창」(지금 없다) ·
//   `SpareEvent` 의 「내는 쪽」(지금 없다 — 끌기 세션을 걷었다). doc 이 바인딩에 실리므로 이 타입들을 바꾸는 그 단위에서 함께 고친다.

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
