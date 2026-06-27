//! 레이아웃 invoke 핸들러 — §5 LLM 제어 표면(ADR-0035). 사람 클릭(`window.__engramLayout`)·LLM
//! 이 동일하게 부르는 단일 control surface.
//!
//! ★락 규율(ADR-0006)★: 모든 핸들러는 ViewManager 락을 **짧게 잡아 변형 + 필요한 데이터(스냅샷·
//! 뷰목록·active_view_id)를 복사**하고 **락을 드롭한 뒤** emit 한다. 락 보유 중 emit/외부 호출 0.
//! 락 드롭은 스코프(`{ ... }`)로 강제하고, emit 은 그 밖에서 한다.
//!
//! ★assign_agent 는 참조 문자열만 저장★ — 데몬에 실재 검증 호출 안 함(락 보유 중 외부 호출 0).
//! invalid view_id/slot_id → no-op + Err(String)(패닉·부분변경 금지).
//!
//! Tauri command 는 `AppHandle`(emit) 과 `State`(LayoutState) 를 동시에 주입받을 수 있다.

use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::layout::{LayoutState, SplitDir, ViewManager, ViewMeta, ViewSnapshot};

/// `layout:updated` 이벤트명 — 한 View 의 레이아웃이 바뀌었을 때(전 창이 미러).
const EVT_LAYOUT_UPDATED: &str = "layout:updated";
/// `view:list-updated` 이벤트명 — View 목록/active 가 바뀌었을 때(탭 바 미러).
const EVT_VIEW_LIST_UPDATED: &str = "view:list-updated";

/// `view:list-updated` 페이로드.
#[derive(serde::Serialize, Clone)]
struct ViewListPayload {
    views: Vec<ViewMeta>,
    active_view_id: Uuid,
}

/// 락 보유 중 mgr 에서 복사한 뷰목록 페이로드(락 드롭 후 emit 에 사용).
fn list_payload(mgr: &ViewManager) -> ViewListPayload {
    ViewListPayload {
        views: mgr.view_metas(),
        active_view_id: mgr.active_view_id,
    }
}

/// 락 드롭 후 호출 — layout:updated(있으면) + view:list-updated 발행. ★반드시 락 미보유 상태에서★.
fn emit_after_unlock(app: &AppHandle, layout: Option<ViewSnapshot>, list: ViewListPayload) {
    if let Some(snap) = layout {
        if let Err(e) = app.emit(EVT_LAYOUT_UPDATED, &snap) {
            tracing::warn!("[layout] {EVT_LAYOUT_UPDATED} emit 실패: {e}");
        }
    }
    if let Err(e) = app.emit(EVT_VIEW_LIST_UPDATED, &list) {
        tracing::warn!("[layout] {EVT_VIEW_LIST_UPDATED} emit 실패: {e}");
    }
}

/// 새 View 생성(빈 슬롯) → active. 새 View id 반환. (탭 바 `+`.)
#[tauri::command]
pub fn create_view(
    app: AppHandle,
    state: State<'_, LayoutState>,
    name: Option<String>,
) -> Result<Uuid, String> {
    let (id, layout, list) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        let id = mgr.create_view(name);
        let layout = mgr.snapshot(id).ok(); // 새 View 가 active → 그 레이아웃도 emit.
        let list = list_payload(&mgr);
        (id, layout, list)
    }; // ← 락 드롭
    emit_after_unlock(&app, layout, list);
    Ok(id)
}

/// View 닫기. active 면 다른 View 로 전환, 마지막이면 빈 상태. invalid view_id → Err(no-op).
#[tauri::command]
pub fn close_view(
    app: AppHandle,
    state: State<'_, LayoutState>,
    view_id: Uuid,
) -> Result<(), String> {
    let (layout, list) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.close_view(view_id).map_err(|e| e.to_string())?;
        // 닫은 뒤 active View 의 레이아웃을 emit(전환·빈 상태 반영).
        let active = mgr.active_view_id;
        let layout = mgr.snapshot(active).ok();
        let list = list_payload(&mgr);
        (layout, list)
    }; // ← 락 드롭
    emit_after_unlock(&app, layout, list);
    Ok(())
}

/// 메인 창 활성 탭 변경(active_view_id). 팝업엔 영향 없음. invalid view_id → Err(no-op).
#[tauri::command]
pub fn switch_view(
    app: AppHandle,
    state: State<'_, LayoutState>,
    view_id: Uuid,
) -> Result<(), String> {
    let (layout, list) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.switch_view(view_id).map_err(|e| e.to_string())?;
        // 전환된 active View 의 레이아웃도 emit(창이 바로 그 View 를 렌더).
        let layout = mgr.snapshot(view_id).ok();
        let list = list_payload(&mgr);
        (layout, list)
    }; // ← 락 드롭
    emit_after_unlock(&app, layout, list);
    Ok(())
}

/// view 안 slot_id 슬롯을 분할. 새 슬롯 id 반환. invalid view_id/slot_id → Err(no-op).
#[tauri::command]
pub fn split_slot(
    app: AppHandle,
    state: State<'_, LayoutState>,
    view_id: Uuid,
    slot_id: Uuid,
    dir: SplitDir,
) -> Result<Uuid, String> {
    let (new_id, layout, list) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        let new_id = mgr
            .split_slot(view_id, slot_id, dir)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        let list = list_payload(&mgr);
        (new_id, layout, list)
    }; // ← 락 드롭
    emit_after_unlock(&app, layout, list);
    Ok(new_id)
}

/// view 안 slot_id 슬롯을 닫음(형제 승격/root 슬롯 리셋). invalid view_id/slot_id → Err(no-op).
#[tauri::command]
pub fn close_slot(
    app: AppHandle,
    state: State<'_, LayoutState>,
    view_id: Uuid,
    slot_id: Uuid,
) -> Result<(), String> {
    let (layout, list) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.close_slot(view_id, slot_id)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        let list = list_payload(&mgr);
        (layout, list)
    }; // ← 락 드롭
    emit_after_unlock(&app, layout, list);
    Ok(())
}

/// view 안 slot_id 슬롯에 agent_id(참조 문자열) 배정. ★데몬에 실재 검증 안 함(ADR-0035/0006).
/// invalid view_id/slot_id → Err(no-op).
#[tauri::command]
pub fn assign_agent(
    app: AppHandle,
    state: State<'_, LayoutState>,
    view_id: Uuid,
    slot_id: Uuid,
    agent_id: String,
) -> Result<(), String> {
    let (layout, list) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.assign_agent(view_id, slot_id, agent_id)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        let list = list_payload(&mgr);
        (layout, list)
    }; // ← 락 드롭
    emit_after_unlock(&app, layout, list);
    Ok(())
}

/// view_id 의 스냅샷(version 포함) 조회. 팝업 pull↔listen race 용. invalid view_id → Err.
/// ★조회만★ — 변형 없음, emit 없음(version 안 올림).
#[tauri::command]
pub fn get_view(state: State<'_, LayoutState>, view_id: Uuid) -> Result<ViewSnapshot, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    mgr.snapshot(view_id).map_err(|e| e.to_string())
}
