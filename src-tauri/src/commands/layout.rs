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
//!
//! ## ★T6b 출력 구독 배선(FIX-1/D3 — 동시성 핵심)★
//! 각 mutation 은 **락 보유 critical section 안**에서 `router.rebuild(&mgr)` 를 호출해 라우팅 표를
//! 재계산하고 구독 델타(`SubscriptionDelta`)를 산출하고, **그 자리에서 곧바로 `send_subscription_delta`
//! 로 enqueue 까지** 한다(load→delta→store→enqueue 를 ViewManager 락이 한 critical section 으로
//! 직렬화). read-only(get_view/list_views)는 변형이 없어 rebuild 안 한다.
//!
//! ## ★enqueue 순서 = rebuild 순서(락 안 enqueue)★
//! rebuild 와 델타 enqueue 를 같은 critical section 안에서 하여 동시 layout invoke 둘의 enqueue 가
//! rebuild 순서와 어긋나 인터리브되지 않게 한다(락이 RMW+enqueue 를 통째로 직렬화). ★ADR-0046 이후★:
//! send_subscription_delta 가 wire 로 보내는 건 `unsubscribe`(1→0 정리)뿐이고, 옛 축 B cursor 델타
//! (`replay_slots`/`drop_slots`)와 eager `subscribe` 는 미러 버퍼와 함께 삭제됐다 — replay 형성은 뷰 주도
//! `request_replay` 단독(BLOCK-1 전면화).
//!
//! ★ADR-0006 재확인 — try_send 는 금지 대상이 아니다★: `unsubscribe` 가 부르는 `cmd_tx.try_send` 는
//! bounded mpsc 의 **동기·non-blocking** enqueue(`.await` 도 network I/O 도 아님)라 락 보유 중 써도
//! ADR-0006 위반이 아니다(금지 대상 = 외부 await·네트워크·emit). lifecycle 락은 ViewManager 락과 독립
//! (겹침 0)이라 데드락도 없다. ★emit 은 여전히 락 밖★(`emit_after_unlock`) — ADR-0006 금지 대상이라 락 해제 후.

use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::daemon_client::DaemonClient;
use crate::layout::{LayoutState, SplitDir, ViewManager, ViewMeta, ViewSnapshot};
use crate::output_router::{OutputRouter, SubscriptionDelta};

/// `layout:updated` 이벤트명 — 한 View 의 레이아웃이 바뀌었을 때(전 창이 미러).
const EVT_LAYOUT_UPDATED: &str = "layout:updated";
/// `view:list-updated` 이벤트명 — View 목록/active 가 바뀌었을 때(탭 바 미러).
const EVT_VIEW_LIST_UPDATED: &str = "view:list-updated";

/// `view:list-updated` 페이로드. list_views(read-only 조회) 반환 타입으로도 쓰여(부팅 init) crate
/// 가시성 필요 — pub command 반환 타입은 private 이면 안 됨(generate_handler 매크로 확장 위치 가시성).
#[derive(serde::Serialize, Clone)]
pub(crate) struct ViewListPayload {
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

/// ★ViewManager 락 보유 중 호출 — 구독 정리 델타를 DaemonClient 로 enqueue(fire-and-forget)★. rebuild 와
/// **같은 critical section 안**에서 불러 "enqueue 순서 = rebuild 순서"를 세운다(동시 invoke 인터리브 방지).
/// 부르는 메서드는 동기 `try_send`(await/network 0)라 락 안에서 ADR-0006 위반 아님(lifecycle 락도 독립 —
/// 데드락 0). 비연결이면 DaemonClient 가 조용히 no-op.
///
/// ## ★ADR-0046 — 라우터는 Unsubscribe(정리)만 발행(BLOCK-1 전면화)★
/// 미러 버퍼 제거 후 layout 이 wire 로 보내는 건 **1→0(어느 창에도 안 보이게 된 agent)의 `Unsubscribe`**
/// 뿐이다. wire 구독 형성(Subscribe)은 **뷰 주도 `request_replay`** 단독이고(0→1 eager Subscribe 삭제),
/// 옛 축 B slot cursor 델타(`replay_slots`/`drop_slots`)도 cursor 와 함께 삭제됐다 — remount/새 창은 데몬
/// ring 전량 재replay(뷰가 mount 시 requestReplay)로 대체한다. `delta.to_subscribe` 는 산출은 되나(진단/
/// 보존 불변식 테스트용) 여기서 wire 로 보내지 않는다.
fn send_subscription_delta(client: &DaemonClient, delta: SubscriptionDelta) {
    // 1→0: 더는 어느 창에도 안 보이는 agent 를 데몬에서 구독 해제(정리). 0→1(to_subscribe)은 뷰 주도
    //   request_replay 가 형성하므로 여기서 안 보낸다(ADR-0046 BLOCK-1).
    for agent_id in delta.to_unsubscribe {
        client.unsubscribe(agent_id);
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
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    name: Option<String>,
) -> Result<Uuid, String> {
    let (id, layout, list) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        let id = mgr.create_view(name);
        let layout = mgr.snapshot(id).ok(); // 새 View 가 active → 그 레이아웃도 emit.
        let list = list_payload(&mgr);
        let delta = router.rebuild(&mgr); // ★락 안 rebuild(RMW 직렬화 — FIX-1/D3)★
                                          // ★근원1 FIX★: enqueue 도 락 안에서 — rebuild 와 한 critical section(enqueue 순서=rebuild 순서).
        send_subscription_delta(&client, delta);
        (id, layout, list)
    }; // ← 락 드롭
    emit_after_unlock(&app, layout, list); // ★emit 은 락 밖(ADR-0006)★
    Ok(id)
}

/// View 닫기. active 면 다른 View 로 전환, 마지막이면 빈 상태. invalid view_id → Err(no-op).
#[tauri::command]
pub fn close_view(
    app: AppHandle,
    state: State<'_, LayoutState>,
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    view_id: Uuid,
) -> Result<(), String> {
    let (layout, list) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.close_view(view_id).map_err(|e| e.to_string())?;
        // 닫은 뒤 active View 의 레이아웃을 emit(전환·빈 상태 반영).
        let active = mgr.active_view_id;
        let layout = mgr.snapshot(active).ok();
        let list = list_payload(&mgr);
        let delta = router.rebuild(&mgr);
        send_subscription_delta(&client, delta); // ★근원1 FIX: 락 안 enqueue★
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
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    view_id: Uuid,
) -> Result<(), String> {
    let (layout, list) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.switch_view(view_id).map_err(|e| e.to_string())?;
        // 전환된 active View 의 레이아웃도 emit(창이 바로 그 View 를 렌더).
        let layout = mgr.snapshot(view_id).ok();
        let list = list_payload(&mgr);
        // ★switch_view 도 rebuild★: active_view_id 변경으로 main 창에 보이는 agent 집합이 바뀐다
        //   (옛 active 의 agent unsubscribe + 새 active 의 agent subscribe).
        let delta = router.rebuild(&mgr);
        send_subscription_delta(&client, delta); // ★근원1 FIX: 락 안 enqueue★
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
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
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
        let delta = router.rebuild(&mgr);
        send_subscription_delta(&client, delta); // ★근원1 FIX: 락 안 enqueue★
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
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
    view_id: Uuid,
    slot_id: Uuid,
) -> Result<(), String> {
    let (layout, list) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.close_slot(view_id, slot_id)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        let list = list_payload(&mgr);
        let delta = router.rebuild(&mgr);
        send_subscription_delta(&client, delta); // ★근원1 FIX: 락 안 enqueue★
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
    router: State<'_, Arc<OutputRouter>>,
    client: State<'_, Arc<DaemonClient>>,
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
        let delta = router.rebuild(&mgr);
        send_subscription_delta(&client, delta); // ★근원1 FIX: 락 안 enqueue★
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

/// View 목록 + active_view_id 조회(= view:list-updated 페이로드와 동형). ★조회만★.
///
/// 왜 필요한가: ViewManager 는 부팅 시 기본 View("View 1")를 자동 생성하지만, 그 변경은 *부팅 전*에
/// 일어나 emit 으로 webview 에 닿지 않는다. 변경 핸들러들은 변경 직후에만 emit 하므로, 부팅 직후의
/// webview 는 active view id 를 발견할 경로가 없어 화면이 비어 있다(첫 create/split 전까지). 이 read-only
/// 조회로 webview 가 부팅 때 현재 active view id 를 물어 → get_view 로 그 레이아웃을 그린다(유령 View 생성 없이).
///
/// 왜 emit 안 하나: 상태를 바꾸지 않는 pull 이라 version 을 올리지 않고 누구에게도 broadcast 하지 않는다
/// (get_view 와 동형 — 락 짧게 잡아 복사 후 drop, 보유 중 외부 호출·emit 0, ADR-0006).
#[tauri::command]
pub fn list_views(state: State<'_, LayoutState>) -> Result<ViewListPayload, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    Ok(list_payload(&mgr))
}
