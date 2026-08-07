//! S14 레이아웃 권위 모듈(ADR-0035) — 레이아웃 SSOT 는 src-tauri(데몬 UI 불가지론).

pub mod manager;
pub mod spatial;
pub mod tree;
pub mod types;

pub use manager::{
    resolve_spawn_slot, CloseTabOutcome, LayoutError, SpawnSlotError, ViewManager,
    WindowTabsSnapshot, MAIN_WINDOW_LABEL,
};
pub use spatial::{compute_spatial, resolve_spatial, Neighbors, SlotSpatial, SpatialToken};
pub use types::{LayoutNode, SlotContent, SplitDir, View, ViewMeta, ViewSnapshot};

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

// ts-rs 바인딩 export 는 각 타입의 `#[ts(export)]` derive 가 자동 생성하는
// `export_bindings_<type>` 테스트가 `src-tauri/bindings/` 에 .ts 를 쓴다(단일 출처).
// 수동 export_all_to 미러는 derive 와 이중출처라 제거(FIX-2, rot 방지). protocol crate
// 의 bindings/ 와 분리 = UI 불가지론(ADR-0035) 유지.
