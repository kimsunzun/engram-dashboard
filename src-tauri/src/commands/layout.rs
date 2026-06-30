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
//! ## ★근원1 FIX(BLOCK) — enqueue 순서 = rebuild 순서(락 안 enqueue)★
//! 이전 구현은 rebuild 를 락 안에서 하고 **델타 enqueue 만 락 밖**에서 했다(ADR-0006 "락 안 송신 금지"
//! 를 근거로). 그 결과 동시 layout invoke 둘의 enqueue 가 **rebuild 순서와 무관하게 인터리브**될 수
//! 있었다 — 예컨대 invoke#1(재배정 → `ReplaySlots`)이 먼저 rebuild 됐는데, 락을 푼 뒤 invoke#2(닫힘 →
//! `DropSlots`)의 enqueue 가 actor 큐에 *먼저* 들어가면 drop 이 replay 를 덮어 좀비 cursor·영구 빈
//! 화면이 난다(빠른 toggle/drag 재현, /review code deep 적출 BLOCK-1).
//!
//! ★ADR-0006 재확인 — try_send 는 금지 대상이 아니다★: `send_subscription_delta` 가 부르는 4개 메서드
//! (`subscribe`/`unsubscribe`/`replay_slots`/`drop_slots`)는 전부 `cmd_tx.try_send`(bounded mpsc 의
//! **동기·non-blocking** enqueue — `.await` 도 network I/O 도 아님)다. ADR-0006 이 락 보유 중 금하는
//! 것은 **외부 await·네트워크 호출·emit**(스케줄러 양보·블로킹·재진입 위험)이지, 채널 슬롯에 값 하나
//! 꽂는 동기 try_send 가 아니다. lifecycle 락은 이 ViewManager 락과 **독립**(겹침 0)이라 데드락도 없다.
//! 그래서 enqueue 를 락 안으로 들여 "enqueue 순서 = 락 순서 = rebuild 순서"를 세운다 — 락이 RMW 와
//! enqueue 를 함께 직렬화하므로 두 invoke 의 (rebuild, enqueue) 쌍이 통째로 직렬된다(인터리브 0).
//! ★emit 은 여전히 락 밖★(`emit_after_unlock`) — emit 은 ADR-0006 금지 대상이라 락 해제 후 한다.

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

/// ★ViewManager 락 보유 중 호출 — 구독 델타를 DaemonClient 로 enqueue(fire-and-forget)★. ★근원1 FIX★:
/// rebuild 와 **같은 critical section 안**에서 불러 "enqueue 순서 = rebuild 순서"를 세운다(락 밖 호출은
/// 동시 invoke 의 ReplaySlots↔DropSlots 인터리브로 drop 이 replay 를 덮음 — BLOCK). 부르는 4개 메서드는
/// 전부 동기 `try_send`(await/network 0)라 락 안에서 ADR-0006 위반 아님(lifecycle 락도 독립 — 데드락 0).
/// 0→1 = `subscribe`(데몬에 Subscribe), 1→0 = `unsubscribe`.
/// 비연결이면 DaemonClient 가 조용히 no-op — connect 진입 시 연결 task 가 router.current_agents()(이
/// rebuild 가 갱신한 현재 보이는 agent SSOT)를 순회하며 재구독해 복구한다(C1, connection.rs main_loop).
///
/// ## ★ADR-0040 2단계 버퍼 hook(두 축의 layout 트리거 — BLOCK-2/FIX-3)★
/// 데몬 wire 구독(축 A — 클라가 데몬에 "이 agent 출력 보내라/멈춰라", agent 단위)과 **별개로**, 클라측
/// 공유 버퍼의 cursor 생명주기(축 B — "어느 창이 어디까지 봤나", **slot=(window,agent) 단위**)도 같은
/// 델타에서 토글한다:
/// - **축 A 0→1(`to_subscribe`)**: `subscribe` 가 데몬에 wire Subscribe(epoch/after_seq=버퍼 최신은 연결
///   task 가 채움). 1→0(`to_unsubscribe`)은 `unsubscribe`.
/// - **축 B `slots_to_replay`**: `replay_slots` 가 새로 생긴 slot 들에 mount-즉시-replay. **agent-union
///   diff 가 아니라 slot 쌍 diff 라** 0→1(새 agent)뿐 아니라 **1→2(이미 보던 agent 에 새 창)** 도 잡아
///   둘째 창이 빈 화면이 안 된다(FIX-3/검증2). actor 경유 → on_frame 과 직렬(BLOCK-2 — replay→live 역전
///   방지). 조용한 agent·재연결 대기 새 창의 빈 화면을 메운다(frame 안 와도, 수용기준 5).
/// - **축 B `slots_to_drop`**: `drop_slots` 가 사라진 slot cursor 제거(마지막이면 content drop).
///   2→1(부분 닫힘)도 slot 쌍 단위로 잡아 죽은 cursor 잔존 0(FIX-3). frame 도착과 **독립**이라
///   terminal(frame 0)+창 닫힘도 정상 폐기(누수 0 — TRD §4 폐기 트리거 = 배정 해제).
///
/// ★순서★: 축 A(wire 구독)를 먼저, 축 B(버퍼 cursor)를 나중에 — 둘은 독립(데몬 스트림 ↔ 클라 cursor)이라
/// 순서가 무손실에 영향 없으나, subscribe 시 데몬 wire 를 먼저 보내 두면 빈 버퍼라도 데몬 replay 가 곧
/// 채워 on_frame 이 마저 전달한다(replay_slots 는 *이미 있는* 버퍼만 즉시 replay). 축 B 는 둘 다 actor
/// enqueue 라 actor 안에서 도착 순서대로 직렬 처리된다.
fn send_subscription_delta(client: &DaemonClient, delta: SubscriptionDelta) {
    // 축 A: agent 단위 데몬 wire 구독/해제.
    for agent_id in delta.to_subscribe {
        client.subscribe(agent_id);
    }
    for agent_id in delta.to_unsubscribe {
        client.unsubscribe(agent_id);
    }
    // 축 B: slot=(window,agent) 단위 버퍼 cursor 생명주기(actor 경유 — on_frame 과 직렬).
    //   ★fresh=false(배정 트리거 — FIX-2)★: layout 배정 델타는 *불가침* 신설(cursor 없을 때만 replay)이다.
    //   정상 mount 에서 등록 트리거(subscribe_output)와 둘 다 전체 replay 를 연속으로 내지 않게(중복 제거) —
    //   배정은 신설 1회만, reload 시 fresh 전체 replay 는 등록 트리거(fresh=true)가 담당한다.
    client.replay_slots(delta.slots_to_replay, false);
    client.drop_slots(delta.slots_to_drop);
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
