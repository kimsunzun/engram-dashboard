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

use engram_dashboard_protocol::{AgentCommand, AgentEvent, RequestId};

use crate::daemon_client::DaemonClient;
use crate::layout::{
    resolve_spatial as resolve_spatial_token, resolve_spawn_slot, CloseTabOutcome, LayoutState,
    SlotContent, SpatialToken, SplitDir, ViewManager, ViewMeta, ViewSnapshot, WindowTabsSnapshot,
    MAIN_WINDOW_LABEL,
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

/// view 안 slot_id 슬롯을 포커스로 지정(click-to-focus — ADR-0066 결정 1). ★백엔드 권위(ADR-0035)★:
/// focused_slot_id 를 백엔드가 소유하고, 프론트는 layout:updated emit 으로만 링을 갱신한다(낙관 프론트
/// 갱신 금지). 사람 클릭·팔레트·키바인딩·LLM(`__engramCmd.run('slot.focus', …)`)이 같은 이 핸들을 흔든다(§5).
///
/// ★라우팅 불변 → rebuild/구독 델타 없음★: 포커스 이동은 어느 슬롯이 어떤 agent 를 보는지(=출력 라우팅)를
/// 바꾸지 않는다 → split/close/assign 과 달리 `router.rebuild` 도 구독 델타도 필요 없다(layout:updated 만
/// emit). 락→변형→해제→emit 순서(ADR-0006)는 형제 command 와 동형. invalid view_id/slot_id → Err(no-op).
// ADR-0066
#[tauri::command]
pub fn focus_slot(
    app: AppHandle,
    state: State<'_, LayoutState>,
    view_id: Uuid,
    slot_id: Uuid,
) -> Result<(), String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.set_focused_slot(view_id, slot_id)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        // 탭 이름·목록은 안 바뀌나 형제 command 와 동형으로 창 탭바도 계약상 갱신(view_owner 파생).
        let tabs = mgr
            .owner_of(view_id)
            .cloned()
            .and_then(|label| tabs_payload(&mgr, &label));
        // 라우팅 불변이라 router.rebuild/send_subscription_delta 안 부른다(위 주석).
        (layout, tabs)
    }; // ← 락 드롭
    emit_after_unlock(&app, layout, tabs); // ★emit 은 락 밖(ADR-0006)★
    Ok(())
}

/// View 이름(탭 라벨) 교체 — §5 LLM 제어 표면(사람 더블클릭 인라인 편집 ↔ LLM `tab.rename` 이 같은 이
/// 핸들을 흔든다). ★백엔드 권위(ADR-0035)★: 이름은 백엔드 ViewManager 가 소유하고, 프론트는 emit 으로만
/// 반영한다(낙관 프론트 갱신 X). 소속 창은 view_owner 파생(split_slot 과 동형 — view-id-키).
///
/// ★탭 페이로드만 emit★: 이름은 `ViewMeta.name`(= window:tabs-updated 페이로드)에만 있고 `ViewSnapshot`
/// (layout:updated)엔 없다 → layout 스냅샷 emit 안 한다. 그리고 rename 은 어느 슬롯이 어떤 agent 를
/// 보는지(=출력 라우팅)도, 레이아웃 트리도 바꾸지 않는다 → `router.rebuild`/구독 델타도 필요 없다
/// (focus_slot 의 "라우팅 불변" 주석과 동형이나, focus 는 layout 을 emit 하는 반면 rename 은 tabs 만).
/// 그래서 router/client State 도 안 받는다(get_view/focus_slot 처럼 미사용 State 생략). 락→변형→해제→emit
/// 순서(ADR-0006)는 형제 command 와 동형. invalid view_id → Err(no-op).
// ADR-0057
#[tauri::command]
pub fn rename_tab(
    app: AppHandle,
    state: State<'_, LayoutState>,
    view_id: Uuid,
    name: String,
) -> Result<(), String> {
    let tabs = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.rename_tab(view_id, name).map_err(|e| e.to_string())?;
        // 소속 창의 탭바만 갱신(이름은 ViewMeta.name 에만 — view_owner 파생). layout 스냅샷·rebuild 없음(위 주석).
        mgr.owner_of(view_id)
            .cloned()
            .and_then(|label| tabs_payload(&mgr, &label))
    }; // ← 락 드롭
    if let Some(tabs) = tabs {
        emit_window_tabs(&app, &tabs); // ★emit 은 락 밖(ADR-0006)★ — window:tabs-updated 만.
    }
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

/// view 안 slot_id 슬롯의 콘텐츠를 `content`(SlotContent 제네릭)로 교체(ADR-0063 배치 제어 표면). assign_agent
/// 의 미러이나 에이전트 전용이 아니라 유니온 전체(Empty/Agent/AgentList/PresetPalette)를 받는다 — 트리·팔레트를
/// 슬롯에 배치하는 §5 LLM/사람 공용 command. ★락→변형→해제→emit(ADR-0006)은 assign_agent 와 동형★. invalid → Err(no-op).
#[tauri::command]
pub fn set_slot_content(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    view_id: Uuid,
    slot_id: Uuid,
    content: SlotContent,
) -> Result<(), String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.set_slot_content(view_id, slot_id, content)
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
    emit_after_unlock(&app, layout, tabs); // ★emit 은 락 밖(ADR-0006)★
    Ok(())
}

// ── 합성 command: spawn_into(D-7 배치 지정 스폰) ──────────────────────────────

/// ★spawn_into(D-7) — 스폰 + 탭 생성(필요 시) + 슬롯 배정을 한 방으로 조립★(TRD §6 · G9).
/// 데몬에 에이전트를 스폰하고, 그 agent 를 `window` 의 탭 슬롯에 배정한다. 성공 시 새 AgentId(String) 반환.
///
/// ## ★ordering(ADR-0006 동시성 계약 — CRITICAL)★
/// 스폰은 데몬 왕복(async)이라 **ViewManager 락을 잡지 않은 채** 먼저 끝낸다. 그 다음에만 락을 잡아
/// (탭/슬롯 해소 + 점유 검사 + 배정 + rebuild + 구독 델타)를 단일 임계구역으로 돌리고, emit 은 락을 드롭한
/// 뒤 한다(락 보유 중 await/emit 0). 옛 command(assign_agent/create_tab)의 임계구역 패턴을 그대로 따른다.
///
/// ## ★슬롯 정책(G9 — 추측 금지, USER DECISION 2b)★
/// - `tab=None`: 먼저 create_tab(window) 로 새 탭(빈 root 슬롯)을 만들고 거기 배정. (`slot=Some` 동반은
///   ★스폰 전에 거부★ — 새로 만들 탭엔 그 slot 이 없어 orphan 탭이 생긴다. 아래 pre-spawn 가드.)
/// - `tab=Some`·`slot=None`: 그 탭의 **첫 빈 슬롯**에 배정(빈 슬롯 없으면 에러 — 자동 split 안 함, 2b).
/// - `slot=Some`·비어있음: 그 슬롯에 배정.
/// - `slot=Some`·점유: **에러**(덮어쓰기 안 함 — 호출자가 split_slot 후 재시도).
///
/// ## ★실패 가시성(§5 손발-두뇌 분리 — spawn-first)★
/// 스폰이 먼저 일어나므로, 이후 배치(점유 슬롯·invalid view/window 등)가 실패해도 **에이전트를 kill 하지
/// 않는다**(하드 롤백 없음). 에이전트는 데몬에 살아 있고 list_agents 로 재부착 가능하다 — 스폰 뒤 모든
/// early-return 은 `alive_err` 로 생존 agent id 를 박아 invisible 에이전트를 막는다(락 획득 실패 포함).
///
/// ## ★backend fail-loud(USER DECISION 1a — ADR-0058)★
/// 현 데몬 스폰 wire(`SpawnByCwd{cwd}`)는 **cwd 만** 받고 backend 선택 인자가 없다 → 요청한 `backend` 는
/// 데몬까지 흐르지 못한다. 데몬의 `SpawnByCwd` 핸들러는 **무조건 데몬 기본 백엔드(현재 셸 =
/// `default_shell()`)** 를 스폰한다(claude 가 아니다 — connection_core.rs `SpawnByCwd` arm). 이전엔 warn 후
/// 무시(요청 backend 조용히 무시하고 셸 스폰)였으나, **명시된 backend 요청은 스폰 전에 거부**한다(호출자가
/// 원한 것과 다른 에이전트를 조용히 받는 것 방지). 통과 = `backend` 미지정(`None`/빈/공백)뿐 —
/// **`"claude"` 포함 어떤 명시값도 거부**(현재 스폰되는 건 셸이므로 "claude 지원"은 거짓말). backend 선택은
/// 데몬 spawn-protocol 확장이 필요하다(미구현 — 별도 ADR/후속).
#[tauri::command]
pub async fn spawn_into(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    window: String,
    tab: Option<Uuid>,
    slot: Option<Uuid>,
    backend: Option<String>,
    cwd: String,
) -> Result<String, String> {
    // ── 0) 스폰 전 검증(에이전트 생성 이전이라 alive_err 불필요 — 아직 아무것도 안 죽음) ──────────────
    // ADR-0058 ★FIX 1(1a) backend fail-loud★: SpawnByCwd wire 에 backend 선택 인자가 없다 → 데몬은 무조건 기본
    //   백엔드(현재 셸)를 스폰한다("claude" 도 스폰 안 됨). 명시된 backend 값을 통과시키면 호출자가 원한 것과
    //   다른 에이전트를 조용히 받는다 → 통과 = 미지정(None/빈/공백)뿐, "claude" 포함 어떤 명시값도 거부한다.
    if let Some(b) = &backend {
        let norm = b.trim();
        if !norm.is_empty() {
            return Err(format!(
                "backend '{b}' 선택은 아직 spawn_into 로 지원되지 않음 — 데몬 SpawnByCwd 는 항상 기본 백엔드(현재 셸)를 스폰하며 backend 선택 wire 가 없다(데몬 spawn-protocol 확장 필요, 후속). backend 를 생략하면 기본 백엔드로 스폰된다. 스폰 안 함."
            ));
        }
    }
    // ★FIX 3 orphan-tab 가드★: tab 미지정(새 탭 생성) + slot 지정은 모순이다 — 새로 만들 탭엔 그 slot 이
    //   없어 스폰 후 resolve 가 실패하고 빈 orphan 탭만 남는다. 스폰 전에 거부한다(탭 생성·스폰 0).
    if tab.is_none() && slot.is_some() {
        return Err(
            "새로 생성될 탭에 특정 slot 을 지정할 수 없음 — slot 을 생략하거나 tab 을 지정하시오. 스폰 안 함."
                .to_string(),
        );
    }

    // ── 1) 스폰(락 미보유 async — 데몬 왕복). Spawned 에 동봉된 AgentInfo.id 를 캡처 ────────────────
    let reply = client
        .send_command(AgentCommand::SpawnByCwd {
            cwd,
            request_id: RequestId::new(),
        })
        .await?;
    let agent_id = match reply {
        // SpawnByCwd 응답 = Spawned(AgentInfo 동봉) — 그 id 가 우리가 배정할 실제 agent id.
        AgentEvent::Spawned { agent, .. } => agent.id.to_string(),
        AgentEvent::Error { message, .. } => return Err(format!("spawn 실패: {message}")),
        other => return Err(format!("spawn 응답 예상 밖(Spawned 기대): {other:?}")),
    };

    // ── 2) 배치(락 보유 단일 임계구역 — 탭/슬롯 해소 + 점유 검사 + 배정 + rebuild + 델타). emit 은 락 밖 ──
    // ★spawn-first 실패 가시성★: 여기서 실패해도 에이전트는 데몬에 살아있다 → 에러 문자열에 agent id 를 박아
    //   호출자가 list_agents 로 재부착할 수 있게 한다(kill 안 함). alive_err 헬퍼로 문구 통일.
    //   ★FIX 4★: 스폰 뒤 모든 early-return(락 획득 실패 포함)은 반드시 alive_err 를 지난다.
    let alive_err = |detail: String| {
        format!("배치 실패({detail}) — 에이전트 {agent_id} 는 살아있음(list_agents 로 재부착 가능)")
    };
    let (view_id, layout, tabs) = {
        // 락 획득 실패(mutex poison)도 생존 agent id 를 남긴다(FIX 4 — id 유실 금지).
        let mut mgr = state
            .0
            .lock()
            .map_err(|e| alive_err(format!("레이아웃 락 획득 실패: {e}")))?;

        // 탭 해소: tab 미지정이면 새 탭(빈 root 슬롯) 생성(위 가드로 slot=None 확정). 지정이면 그 view.
        let view_id = match tab {
            Some(v) => {
                // 이 창의 탭인지 검증(소속 불일치·없는 view → 배치 실패, 에이전트 생존).
                if mgr.owner_of(v).map(|l| l.as_str()) != Some(window.as_str()) {
                    return Err(alive_err(format!("view {v} 가 창 {window} 의 탭이 아님")));
                }
                v
            }
            // tab=None → 새 탭(빈 root 1개). slot 은 위 가드에서 None 확정 → resolve 는 항상 그 빈 root 로
            // 성공한다(orphan 탭 불가 — FIX 3). create_tab 만이 여기서 유일한 실패 지점(창 부재).
            None => mgr
                .create_tab(&window, None)
                .map_err(|e| alive_err(e.to_string()))?,
        };

        // 슬롯 해소(순수 정책 — G9/2b): slot=None→첫 빈 슬롯 / Some+빈=그 슬롯 / 점유·부재=에러(덮어쓰기 X).
        let view = mgr
            .views
            .get(&view_id)
            .ok_or_else(|| alive_err(format!("view {view_id} 없음")))?;
        let target_slot = resolve_spawn_slot(view, slot).map_err(|e| alive_err(e.to_string()))?;

        // 배정(점유 검사는 위 resolve 가 이미 함 — assign 은 빈 슬롯 확정 후에만 닿음).
        mgr.assign_agent(view_id, target_slot, agent_id.clone())
            .map_err(|e| alive_err(e.to_string()))?;

        let layout = mgr.snapshot(view_id).ok();
        let tabs = tabs_payload(&mgr, &window);
        let delta = router.rebuild(&mgr); // ★락 안 rebuild(RMW 직렬화)★
        send_subscription_delta(&client, delta);
        (view_id, layout, tabs)
    }; // ← 락 드롭
    emit_after_unlock(&app, layout, tabs); // ★emit 은 락 밖(ADR-0006)★

    tracing::info!(agent = %agent_id, window = %window, view = %view_id, "spawn_into 완료(스폰+배치)");
    Ok(agent_id)
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

/// ★공간/방향 토큰 → slot id 해소(ADR-0068 — §5 백엔드 권위 resolver)★. ★조회만★(변형·emit 0).
/// `view_id` 지정이면 그 View, 미지정(None)이면 `window`(미지정 시 main) 의 활성 탭 View 를 대상으로 한다.
/// - 모서리 토큰 `top-left`/`top-right`/`bottom-left`/`bottom-right`: 트리 전체에서 그 코너에 가장 가까운 슬롯.
/// - 상대 방향 `left`/`right`/`up`/`down`: 그 View 의 `focused_slot_id` 기준 방향 이웃(없으면 null).
///
/// 논리 도면(split 방향·ratio) 파생이라 픽셀·창크기 무관(ADR-0068). 사람·팔레트·LLM(`__engramCmd.run`/
/// `slot.resolveSpatial`)이 같은 이 핸들을 흔든다(§5 단일 제어 표면). 모르는 토큰/없는 View → Err(fail-loud).
// ADR-0068
#[tauri::command]
pub fn resolve_spatial(
    state: State<'_, LayoutState>,
    token: String,
    window: Option<String>,
    view_id: Option<Uuid>,
) -> Result<Option<Uuid>, String> {
    let tok = SpatialToken::parse(&token)
        .ok_or_else(|| format!("알 수 없는 공간 토큰: '{token}' (top-left/top-right/bottom-left/bottom-right/left/right/up/down)"))?;
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    // 대상 View 해소: view_id 우선, 없으면 window(미지정 시 main) 의 활성 탭.
    let vid = match view_id {
        Some(v) => v,
        None => {
            let label = window.as_deref().unwrap_or(MAIN_WINDOW_LABEL);
            mgr.list_tabs(label).map_err(|e| e.to_string())?.active
        }
    };
    let v = mgr
        .views
        .get(&vid)
        .ok_or_else(|| format!("view 없음: {vid}"))?;
    Ok(resolve_spatial_token(&v.layout, v.focused_slot_id, tok))
}

// close_window("main") 거부는 ViewManager::close_window 가 LayoutError::MainNotClosable 로 강제한다
// (불변식 4). command 레이어는 그 Err 를 문자열로 전달만 한다(별도 가드 불필요 — 모델이 SSOT).
// main 창은 lib.rs CloseRequested arm 이 prevent_close+hide 로만 처리해 Destroyed 를 안 남긴다.
const _: &str = MAIN_WINDOW_LABEL; // 상수 참조 유지(문서 앵커).
