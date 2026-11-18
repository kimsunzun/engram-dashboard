//! S14 layout authority module (ADR-0035) — the layout SSOT is src-tauri (the daemon is UI-agnostic).

pub mod apply;
pub mod bands;
// ADR-0155 decision 1: the `window`/`tab`/`slot` declarations live next to their body (the apply service).
pub mod commands;
pub mod dock;
pub mod drag_size;
pub mod geometry;
pub mod manager;
pub mod numbering;
pub mod rows;
pub mod spatial;
pub mod tab_name;
pub mod target;
pub mod tree;
pub mod types;
pub mod window_arg;
pub mod window_ctl;

pub use apply::{
    AgentSpawner, DockDest, DockOutcome, DockRequest, LabelSource, LayoutEvents, MoveFailed,
    MoveStep, NewWindow, SlotMove, SpareClaim, SubscriptionSync, WindowHost, WindowPlace,
    WindowTabsPayload,
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

// Concurrent access from the invoke thread pool → Mutex.
// ★std Mutex, not Tauri async_runtime::Mutex★: a mutation is a short synchronous section and there is no
// await (external call) while the lock is held (ADR-0006: zero external calls while holding a lock) → std Mutex is enough.
#[derive(Clone, Default)]
pub struct LayoutState(pub Arc<Mutex<ViewManager>>);

impl LayoutState {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(ViewManager::new())))
    }
}

/// Releasing the number for the `Destroyed` part of a popout OS window `label` — a stand-in for when there is no state to call the destroy cleanup (`commands::popout::cleanup_popup_window`) (TRD S26-window-chrome §4-1). A closing window releases that number; a window still in the model has its whole entry removed and the number released at once. It does not recompute the routing table, clean up subscriptions or leave a tombstone — so when the cleanup can be called, do not call this (the cleanup releases the number in the same lock as those three). For main · an unknown label · a second call it does nothing, and a poisoned lock does nothing at all.
// ADR-0300
pub fn release_window_number(state: &LayoutState, label: &str) {
    if label == MAIN_WINDOW_LABEL {
        return;
    }
    let Ok(mut mgr) = state.0.lock() else {
        tracing::warn!(
            label,
            "releasing the window number: lock poisoned — skipping"
        );
        return;
    };
    mgr.release_closing(label);
    if mgr.windows.contains_key(label) {
        let _ = mgr.drop_window_gone(label);
    }
}

// For the ts-rs binding export, the `export_bindings_<type>` test that each type's `#[ts(export)]` derive generates writes the .ts into `src-tauri/bindings/` (single source). The manual export_all_to mirror was removed as a double source with the derive (FIX-2); bindings/ stays separate from the protocol crate's, preserving UI-agnostic (ADR-0035).
