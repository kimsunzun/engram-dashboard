//! 레이아웃 invoke 핸들러 — §5 LLM 제어 표면(ADR-0035/0057). 사람 클릭(`window.__engramLayout`)·LLM
//! 이 동일하게 부르는 단일 control surface. 탭 소유 모델(ADR-0057) — 창별 탭 command.
//!
//! ★락 규율(ADR-0006)★: 모든 핸들러는 ViewManager 락을 **짧게 잡아 변형 + 필요한 데이터(스냅샷·
//! 탭목록)를 복사**하고 **락을 드롭한 뒤** emit 한다. 락 보유 중 emit/외부 호출/webview 빌드 0.
//! 락 드롭은 스코프(`{ ... }`)로 강제하고, emit 은 그 밖에서 한다.
//!
//! ★assign_agent 는 참조 문자열만 저장★ — 데몬에 실재 검증 호출 안 함(락 보유 중 외부 호출 0).
//! invalid view_id/slot_id/window → no-op + Err(String)(패닉·부분변경 금지).
//!
//! ## ★출력 구독 배선(FIX-1/D3 — 동시성 핵심)★
//! 각 mutation 은 **락 보유 critical section 안**에서 `router.rebuild(&mgr)` 를 호출해 라우팅 표를
//! 재계산하고 구독 델타(`SubscriptionDelta`)를 산출하고, **그 자리에서 곧바로 `send_subscription_delta`
//! 로 enqueue 까지** 한다(load→delta→store→enqueue 를 ViewManager 락이 한 critical section 으로
//! 직렬화). read-only(get_view/list_tabs/list_windows)는 변형이 없어 rebuild 안 한다.
//!
//! ## ★ADR-0046 — 라우터는 Unsubscribe(정리)만 발행(BLOCK-1 전면화)★
//! `send_subscription_delta` 가 wire 로 보내는 건 `delta.to_unsubscribe`(1→0 정리)뿐이고, eager
//! Subscribe(0→1)와 옛 축 B cursor 델타는 미러 버퍼와 함께 삭제됐다 — replay 형성은 뷰 주도
//! `request_replay` 단독. try_send 는 동기·non-blocking 이라 락 보유 중 써도 ADR-0006 위반 아님(금지
//! 대상 = 외부 await·네트워크·emit). ★emit 은 여전히 락 밖★.
//!
//! ## ★이벤트: 창별 `window:tabs-updated`(ADR-0057)★
//! 옛 전역 `view:list-updated` 는 창별 `window:tabs-updated{label,tabs,active,version}` 로 대체됐다.
//! `view:closed` 는 엔드투엔드 은퇴(더는 emit 안 함 — §5-2/G2). `layout:updated`(뷰 스냅샷)는 유지.

use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::daemon_client::DaemonClient;
use crate::layout::{
    CloseTabOutcome, LayoutState, SplitDir, ViewManager, ViewMeta, ViewSnapshot,
    WindowTabsSnapshot, MAIN_WINDOW_LABEL,
};
use crate::output_router::{OutputRouter, SubscriptionDelta};

/// `layout:updated` 이벤트명 — 한 View 의 레이아웃이 바뀌었을 때(전 창이 미러).
const EVT_LAYOUT_UPDATED: &str = "layout:updated";
/// `window:tabs-updated` 이벤트명 — 한 창의 탭 목록/활성이 바뀌었을 때(창별 탭바 미러, ADR-0057).
/// 프론트는 `label` 이 자기 창과 일치할 때만 반응한다(§7-1).
const EVT_WINDOW_TABS_UPDATED: &str = "window:tabs-updated";

/// `window:tabs-updated` 페이로드(ADR-0057). `version` 부착(G10 — stale emit 프론트 폐기).
/// `list_tabs` 반환 타입으로도 쓰여(부팅 init) — pub command 반환이라 pub 필요(generate_handler 확장 위치).
#[derive(serde::Serialize, Clone)]
pub struct WindowTabsPayload {
    label: String,
    tabs: Vec<ViewMeta>,
    #[serde(rename = "active")]
    active: Uuid,
    version: u64,
}

impl From<WindowTabsSnapshot> for WindowTabsPayload {
    fn from(s: WindowTabsSnapshot) -> Self {
        WindowTabsPayload {
            label: s.label,
            tabs: s.tabs,
            active: s.active,
            version: s.version,
        }
    }
}

/// ★ViewManager 락 보유 중 호출 — 구독 정리 델타를 DaemonClient 로 enqueue(fire-and-forget)★. rebuild 와
/// **같은 critical section 안**에서 불러 "enqueue 순서 = rebuild 순서"를 세운다(동시 invoke 인터리브 방지).
/// 부르는 메서드는 동기 `try_send`(await/network 0)라 락 안에서 ADR-0006 위반 아님(lifecycle 락도 독립 —
/// 데드락 0). 비연결이면 DaemonClient 가 조용히 no-op.
///
/// ## ★ADR-0046 — 라우터는 Unsubscribe(정리)만 발행(BLOCK-1 전면화)★
/// wire 로 보내는 건 **1→0(어느 창에도 안 보이게 된 agent)의 `Unsubscribe`** 뿐. wire 구독 형성은
/// **뷰 주도 `request_replay`** 단독이고(0→1 eager Subscribe 삭제), `delta.to_subscribe` 는 산출은 되나
/// (진단/보존 불변식 테스트용) 여기서 wire 로 보내지 않는다.
pub(crate) fn send_subscription_delta(client: &DaemonClient, delta: SubscriptionDelta) {
    for agent_id in delta.to_unsubscribe {
        client.unsubscribe(agent_id);
    }
}

/// 락 보유 중 mgr 에서 창별 탭 페이로드를 복사(락 드롭 후 emit 에 사용). 없는 창이면 None.
fn tabs_payload(mgr: &ViewManager, label: &str) -> Option<WindowTabsPayload> {
    mgr.list_tabs(label).ok().map(Into::into)
}

/// 락 드롭 후 호출 — layout:updated(있으면) + window:tabs-updated 발행. ★반드시 락 미보유 상태에서★.
/// `tabs` 가 None(창 없음)이면 탭 이벤트 스킵(예: 팝업 자가닫힘으로 창이 이미 제거됨).
fn emit_after_unlock(
    app: &AppHandle,
    layout: Option<ViewSnapshot>,
    tabs: Option<WindowTabsPayload>,
) {
    if let Some(snap) = layout {
        if let Err(e) = app.emit(EVT_LAYOUT_UPDATED, &snap) {
            tracing::warn!("[layout] {EVT_LAYOUT_UPDATED} emit 실패: {e}");
        }
    }
    if let Some(tabs) = tabs {
        emit_window_tabs(app, &tabs);
    }
}

/// window:tabs-updated 를 발행(창별 탭바 미러). ★락 미보유 상태에서★.
pub(crate) fn emit_window_tabs(app: &AppHandle, tabs: &WindowTabsPayload) {
    if let Err(e) = app.emit(EVT_WINDOW_TABS_UPDATED, tabs) {
        tracing::warn!("[layout] {EVT_WINDOW_TABS_UPDATED} emit 실패: {e}");
    }
}

// ── 탭 command (창별 — ADR-0057) ─────────────────────────────────────────────

/// 창 `window` 에 새 빈-슬롯 탭 추가·활성화. 새 View id 반환. (탭바 `+`.)
#[tauri::command]
pub fn create_tab(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    window: String,
    name: Option<String>,
) -> Result<Uuid, String> {
    let (id, layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        let id = mgr.create_tab(&window, name).map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(id).ok(); // 새 탭이 active → 그 레이아웃도 emit.
        let tabs = tabs_payload(&mgr, &window);
        let delta = router.rebuild(&mgr); // ★락 안 rebuild(RMW 직렬화)★
        send_subscription_delta(&client, delta);
        (id, layout, tabs)
    }; // ← 락 드롭
    emit_after_unlock(&app, layout, tabs); // ★emit 은 락 밖(ADR-0006)★
    Ok(id)
}

/// 빈 새 창(빈 탭 1개) 생성 + 웹뷰 빌드(D-6). label = `PopupCounter`/`slot-popup-*` prefix 재사용(G8).
/// ★async fn 필수★: WebviewWindowBuilder 데드락 회피(락 밖 빌드). 성공 시 새 창 label 반환.
#[tauri::command]
pub async fn create_window(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    counter: State<'_, Arc<crate::commands::popout::PopupCounter>>,
    client: State<'_, Arc<DaemonClient>>,
) -> Result<String, String> {
    crate::commands::popout::create_empty_window(&app, &state, &router, &counter, &client).await
}

/// 창 `window` 의 활성 탭을 `view` 로 교체(그 창만, 타 창 불변). keep-alive 라 노출 집합 불변(rebuild 는
/// 계약상 호출 — 활성 표시만 바뀜). invalid → Err(no-op).
#[tauri::command]
pub fn switch_tab(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    window: String,
    view: Uuid,
) -> Result<(), String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.switch_tab(&window, view).map_err(|e| e.to_string())?;
        // 전환된 활성 탭의 레이아웃도 emit(창이 바로 그 View 를 렌더).
        let layout = mgr.snapshot(view).ok();
        let tabs = tabs_payload(&mgr, &window);
        let delta = router.rebuild(&mgr);
        send_subscription_delta(&client, delta);
        (layout, tabs)
    }; // ← 락 드롭
    emit_after_unlock(&app, layout, tabs);
    Ok(())
}

/// 창 `window` 의 탭 `view` 닫기(§5-2 상태기계). 메인 마지막=빈탭 강제·팝업 마지막=창 닫힘(에이전트 생존).
/// invalid → Err(no-op).
#[tauri::command]
pub async fn close_tab(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    window: String,
    view: Uuid,
) -> Result<(), String> {
    let (outcome, layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        let outcome = mgr.close_tab(&window, view).map_err(|e| e.to_string())?;
        // 창이 닫힌 게 아니면 그 창의 활성 탭 레이아웃/탭목록을 emit. 닫혔으면 None(창 소멸).
        let (layout, tabs) = match outcome {
            CloseTabOutcome::Stayed => {
                let active = mgr.list_tabs(&window).ok().map(|s| s.active);
                let layout = active.and_then(|a| mgr.snapshot(a).ok());
                (layout, tabs_payload(&mgr, &window))
            }
            CloseTabOutcome::WindowClosed => (None, None),
        };
        let delta = router.rebuild(&mgr);
        send_subscription_delta(&client, delta);
        (outcome, layout, tabs)
    }; // ← 락 드롭

    match outcome {
        CloseTabOutcome::Stayed => emit_after_unlock(&app, layout, tabs),
        // ★팝업 마지막 탭 → 창 닫힘 = 백엔드 close_window 단일 소스(§5-2/G2)★. windows 엔트리는 이미
        //   close_tab 이 제거했으니 OS 창을 닫는다 → Destroyed → cleanup_popup_window 가 registry/Channel
        //   잔여 정리. (view:closed 이중 발화 없음 — 은퇴.)
        CloseTabOutcome::WindowClosed => {
            crate::commands::popout::destroy_window(&app, &window);
        }
    }
    Ok(())
}

/// 창 `window` 통째 닫기(모든 탭). ★`"main"` 금지(불변식 4 — main 은 hide only)★. invalid → Err.
/// 모델에서 창을 지운 뒤 OS 창을 destroy 한다(Destroyed → cleanup 이 registry/Channel 정리).
#[tauri::command]
pub async fn close_window(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    window: String,
) -> Result<(), String> {
    {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        // main 거부(불변식 4) + 모든 탭 View 드롭.
        mgr.close_window(&window).map_err(|e| e.to_string())?;
        let delta = router.rebuild(&mgr);
        send_subscription_delta(&client, delta);
    }; // ← 락 드롭
       // OS 창 destroy(Destroyed → cleanup_popup_window 가 registry/Channel 잔여 정리).
    crate::commands::popout::destroy_window(&app, &window);
    Ok(())
}

/// view 안 slot_id 슬롯을 분할. 새 슬롯 id 반환. 소속 창은 view_owner 파생(O(1)). invalid → Err(no-op).
#[tauri::command]
pub fn split_slot(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    view_id: Uuid,
    slot_id: Uuid,
    dir: SplitDir,
) -> Result<Uuid, String> {
    let (new_id, layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        let new_id = mgr
            .split_slot(view_id, slot_id, dir)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        // 소속 창의 탭바(이름 변화는 없으나 계약상 갱신 — view_owner 파생).
        let tabs = mgr
            .owner_of(view_id)
            .cloned()
            .and_then(|label| tabs_payload(&mgr, &label));
        let delta = router.rebuild(&mgr);
        send_subscription_delta(&client, delta);
        (new_id, layout, tabs)
    }; // ← 락 드롭
    emit_after_unlock(&app, layout, tabs);
    Ok(new_id)
}

/// view 안 slot_id 슬롯을 닫음(형제 승격/root 슬롯 리셋). invalid → Err(no-op).
#[tauri::command]
pub fn close_slot(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    view_id: Uuid,
    slot_id: Uuid,
) -> Result<(), String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.close_slot(view_id, slot_id)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        let tabs = mgr
            .owner_of(view_id)
            .cloned()
            .and_then(|label| tabs_payload(&mgr, &label));
        let delta = router.rebuild(&mgr);
        send_subscription_delta(&client, delta);
        (layout, tabs)
    }; // ← 락 드롭
    emit_after_unlock(&app, layout, tabs);
    Ok(())
}

/// view 안 slot_id 슬롯에 agent_id(참조 문자열) 배정. ★데몬에 실재 검증 안 함(ADR-0035/0006).
/// 같은 agent 가 다른 View 에도 배정될 수 있음(불변식 5). invalid → Err(no-op).
#[tauri::command]
pub fn assign_agent(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    view_id: Uuid,
    slot_id: Uuid,
    agent_id: String,
) -> Result<(), String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.assign_agent(view_id, slot_id, agent_id)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        let tabs = mgr
            .owner_of(view_id)
            .cloned()
            .and_then(|label| tabs_payload(&mgr, &label));
        let delta = router.rebuild(&mgr);
        send_subscription_delta(&client, delta);
        (layout, tabs)
    }; // ← 락 드롭
    emit_after_unlock(&app, layout, tabs);
    Ok(())
}

// ── read-only 조회 ───────────────────────────────────────────────────────────

/// view_id 의 스냅샷(version 포함) 조회. 팝업 pull↔listen race 용. invalid view_id → Err.
/// ★조회만★ — 변형 없음, emit 없음(version 안 올림).
#[tauri::command]
pub fn get_view(state: State<'_, LayoutState>, view_id: Uuid) -> Result<ViewSnapshot, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    mgr.snapshot(view_id).map_err(|e| e.to_string())
}

/// 창 `window` 의 탭 목록 + 활성 + version 조회(= window:tabs-updated 페이로드와 동형). ★조회만★.
///
/// 왜 필요한가: 창이 mount 되면 자기 활성 탭을 확정해야 하는데(팝업은 `?window=` label 로만 자기 창을
/// 알 뿐 활성 탭은 백엔드가 권위), 변경 핸들러는 변경 직후에만 emit 한다 → 부팅/mount 직후엔 이 read-only
/// pull 로 `{tabs,active,version}` 을 받아 초기 렌더한다(§3-3/G3). 없는 창이면 Err.
#[tauri::command]
pub fn list_tabs(
    state: State<'_, LayoutState>,
    window: String,
) -> Result<WindowTabsPayload, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    mgr.list_tabs(&window)
        .map(Into::into)
        .map_err(|e| e.to_string())
}

/// 창 label 목록 조회(부팅·진단용). ★조회만★.
#[tauri::command]
pub fn list_windows(state: State<'_, LayoutState>) -> Result<Vec<String>, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    Ok(mgr.list_windows())
}

// close_window("main") 거부는 ViewManager::close_window 가 LayoutError::MainNotClosable 로 강제한다
// (불변식 4). command 레이어는 그 Err 를 문자열로 전달만 한다(별도 가드 불필요 — 모델이 SSOT).
// main 창은 lib.rs CloseRequested arm 이 prevent_close+hide 로만 처리해 Destroyed 를 안 남긴다.
const _: &str = MAIN_WINDOW_LABEL; // 상수 참조 유지(문서 앵커).
