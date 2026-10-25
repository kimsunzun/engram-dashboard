//! 스냅 레이아웃 오버레이의 Tauri 어댑터 — invoke `report_max_button_rect`(TRD S26-window-chrome §8-4). ★셸 기능
//! `snap-overlay` 뒤 · 기본 꺼짐★(사용자 결정 Q1 2026-10-24) — 이 파일 · 그 모듈 선언(`commands/mod.rs`) · invoke 등록
//! (`lib.rs`)이 그 기능으로만 컴파일된다. 꺼진 빌드에서 창 사실 `caption.snap_overlay` 는 거짓이라(`commands/tab_drop.rs` 의
//! `caption_chrome`) 웹이 이 invoke 를 부르지 않는다.
//!
//! - 웹이 자기 최대화 단추 사각형(이 창 클라이언트의 물리 px)을 보고하면 platform `window::snap` 이 그 자리에 오버레이를
//!   놓거나 옮기고, `rect: null` 이면 걷는다. 오버레이가 커서를 가져가므로 맴돎 바뀜은 그 창에 사건 [`EVT_SNAP_HOVER`]
//!   (몸 = bool)로 돌려준다 — 웹이 단추 맴돎을 그 값으로 그린다(`src/components/layout/titleRow/snapOverlay.ts`).
//! - 창 label 을 인자로 받지 않는다 — Tauri 가 넣어 주는 창이 그 값의 유일한 권위다(`report_window_min_width` 와 같은 규칙).
//!   버스 명령이 아니다 — 측정 보고다(ADR-0227).
//! - ★비동기로 바꾸지 말 것★ — 동기 명령은 메인 스레드에서 돌고, 오버레이 창은 부모 창 스레드(메인)에서 지어야 한다
//!   (platform `window::snap` 머리).
//! - OS `cfg` 를 두지 않는다 — OS 마다 다른 일은 platform 이 한다(ADR-0266 · TRD §8-3).

use engram_dashboard_platform::window::{snap, PhysRect};
use tauri::{Emitter, WebviewWindow};

/// 오버레이 맴돎 바뀜 — 셸 → 그 창. 몸 = bool(참 = 들어옴). 웹의 같은 이름 상수(`snapOverlay.ts`)와 짝이다.
pub const EVT_SNAP_HOVER: &str = "window:snap-hover";

/// 최대화 단추 사각형 — 이 창 클라이언트의 물리 px(웹이 CSS px 에 `devicePixelRatio` 를 곱해 바깥으로 맞춘 값).
#[derive(Debug, Clone, Copy, serde::Deserialize)]
pub struct MaxButtonRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// `invoke('report_max_button_rect', { rect })` — `rect` 가 있으면 오버레이를 놓거나 옮기고, `null` 이면 걷는다. 너비 · 높이가
/// 1 보다 작거나 OS 호출이 실패하면 `Err` 다(그 창의 오버레이는 그대로).
#[tauri::command]
pub fn report_max_button_rect(
    window: WebviewWindow,
    rect: Option<MaxButtonRect>,
) -> Result<(), String> {
    let Some(r) = rect else {
        return snap::remove_overlay(&window).map_err(|e| e.to_string());
    };
    let emitter = window.clone();
    snap::place_overlay(
        &window,
        PhysRect {
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
        },
        Box::new(move |hover| {
            if let Err(e) = emitter.emit_to(emitter.label(), EVT_SNAP_HOVER, hover) {
                tracing::warn!(window = emitter.label(), "{EVT_SNAP_HOVER} emit 실패: {e}");
            }
        }),
    )
    .map_err(|e| e.to_string())
}
