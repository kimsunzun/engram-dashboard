//! S14 레이아웃 권위 모듈(ADR-0035) — 레이아웃 SSOT 는 src-tauri(데몬 UI 불가지론).

pub mod apply;
pub mod bands;
// ADR-0155 결정 1: `window`/`tab`/`slot` 선언은 그 본문(적용 서비스) 옆에 산다.
pub mod commands;
pub mod dock;
pub mod drag_size;
pub mod geometry;
pub mod manager;
pub mod numbering;
pub mod spatial;
pub mod tab_name;
pub mod tree;
pub mod types;

pub use apply::{
    AgentSpawner, DockDest, DockOutcome, DockRequest, LabelSource, LayoutEvents, MoveFailed,
    MoveStep, SlotMove, SpareClaim, SubscriptionSync, WindowHost, WindowPlace, WindowTabsPayload,
};
pub use manager::{
    CanvasPx, CloseTabOutcome, LayoutError, SlotPx, SpawnSlotError, SplitRatioResult, ViewManager,
    WindowAttrs, WindowBounds, WindowPlacement, WindowTabsSnapshot, MAIN_WINDOW_LABEL,
};
pub use spatial::{compute_spatial, resolve_spatial, Neighbors, SlotSpatial, SpatialToken};
pub use types::{
    LayoutNode, SlotContent, SplitDir, SplitRatioApplied, SplitRatioOutcome, TabName, UiMetrics,
    View, ViewMeta, ViewSnapshot,
};

use std::sync::{Arc, Mutex};

// invoke 스레드풀 동시접근 → Mutex.
// ★Tauri async_runtime::Mutex 가 아닌 std Mutex★: mutation 은 짧은 동기 구간이고 락 보유 중
// await(외부 호출)가 없다(ADR-0006: 락 보유 중 외부 호출 0) → std Mutex 로 충분.
#[derive(Clone, Default)]
pub struct LayoutState(pub Arc<Mutex<ViewManager>>);

impl LayoutState {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(ViewManager::new())))
    }
}

/// 팝아웃 OS 창 `label` 의 `Destroyed` 몫의 번호 풀기 — 소멸 정리(`commands::popout::cleanup_popup_window`)를 부를
/// 상태가 없을 때의 대신이다(TRD S26-window-chrome §4-1). 닫히는 창이면 그 번호를 풀고, 아직 모델에 있던 창이면
/// 항목째 빼 곧바로 푼다. 라우팅 표 재계산 · 구독 정리 · 묘비는 하지 않는다 — 그래서 정리를 부를 수 있으면 이것을
/// 부르지 않는다(정리가 번호 풀기를 그 셋과 한 락 안에서 한다). main · 모르는 label · 두 번째 부름은 무동작이고,
/// 락이 오염됐으면 아무것도 하지 않는다.
pub fn release_window_number(state: &LayoutState, label: &str) {
    if label == MAIN_WINDOW_LABEL {
        return;
    }
    let Ok(mut mgr) = state.0.lock() else {
        tracing::warn!(label, "창 번호 풀기: lock poisoned — 건너뛴다");
        return;
    };
    mgr.release_closing(label);
    if mgr.windows.contains_key(label) {
        let _ = mgr.drop_window_gone(label);
    }
}

// ts-rs 바인딩 export 는 각 타입의 `#[ts(export)]` derive 가 자동 생성하는
// `export_bindings_<type>` 테스트가 `src-tauri/bindings/` 에 .ts 를 쓴다(단일 출처).
// 수동 export_all_to 미러는 derive 와 이중출처라 제거(FIX-2, rot 방지). protocol crate
// 의 bindings/ 와 분리 = UI 불가지론(ADR-0035) 유지.
