//! S14 레이아웃 권위 모듈(ADR-0035) — 레이아웃 SSOT 는 src-tauri(데몬 UI 불가지론).
//!
//! 구성:
//! - `types`   — wire 타입(View/LayoutNode/SplitDir/ViewMeta/ViewSnapshot, ts-rs 미러). src-tauri 한정.
//! - `tree`    — 순수 split-트리 연산(Tauri 의존 0, headless 테스트 — ADR-0012).
//! - `spatial` — 슬롯 공간 타깃 파생(neighbors/ordinal/방향 토큰, Tauri·픽셀 0 — ADR-0068).
//! - `manager` — ViewManager 상태 + mutation(Tauri 의존 0, emit 은 command 레이어가 — ADR-0006).
//!
//! AppState 가 `Arc<Mutex<ViewManager>>` 로 소유, command(`commands/layout.rs`)가 락→변형→해제→emit.

pub mod manager;
pub mod spatial;
pub mod tree;
pub mod types;

pub use manager::{
    resolve_spawn_slot, CloseTabOutcome, LayoutError, SpawnSlotError, ViewManager,
    WindowTabsSnapshot, MAIN_WINDOW_LABEL,
};
// ADR-0068: 공간 타깃 파생(논리 도면) — neighbors/ordinal 스냅샷 필드 + 방향 토큰 resolver.
pub use spatial::{compute_spatial, resolve_spatial, Neighbors, SlotSpatial, SpatialToken};
pub use types::{LayoutNode, SlotContent, SplitDir, View, ViewMeta, ViewSnapshot};

use std::sync::{Arc, Mutex};

/// AppState — src-tauri 가 manage 하는 레이아웃 권위 핸들. invoke 스레드풀 동시접근 → Mutex.
/// ★Tauri async_runtime::Mutex 가 아닌 std Mutex★: mutation 은 짧은 동기 구간이고 락 보유 중
/// await(외부 호출)가 없다(ADR-0006: 락 보유 중 외부 호출 0) → std Mutex 로 충분.
#[derive(Clone, Default)]
pub struct LayoutState(pub Arc<Mutex<ViewManager>>);

impl LayoutState {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(ViewManager::new())))
    }
}

// ts-rs 바인딩 export 는 각 타입의 `#[ts(export)]` derive 가 자동 생성하는
// `export_bindings_<type>` 테스트가 `src-tauri/bindings/` 에 .ts 를 쓴다(단일 출처).
// 수동 export_all_to 미러는 derive 와 이중출처라 제거(FIX-2, rot 방지). protocol crate
// 의 bindings/ 와 분리 = UI 불가지론(ADR-0035) 유지.
