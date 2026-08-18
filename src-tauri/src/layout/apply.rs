//! 레이아웃 적용 서비스 — 사람 클릭(Tauri invoke)과 중계된 LLM 호출이 **같은 함수**에 떨어지는 전송
//! 중립 단일 경로(ADR-0081 결정 3). ADR-0081 「거부한 대안」이 거부한 형태 = 호출자가 `ViewManager` 를
//! 직접 잠그고 부르는 것(사람 경로와 LLM 경로가 갈려 2 코드 경로가 된다) — 이 모듈이 그 유일한 대안이다.
//!
//! ★Tauri 의존 0★: 전부 `AppHandle`·`State<..>` 를 인자에서 걷어낸 자유 함수다 — 창 없이 단독 호출·
//! 테스트가 서야 하기 때문이다(TRD S20 §7 「셸 적용 서비스」). 외부 세계(프론트 알림·OS 창·데몬 왕복·
//! 구독 표)에는 아래 포트 4개로만 닿는다. `#[tauri::command]` 껍데기는 `commands/layout.rs`.
//!
//! ## ★락 규율★
//! 쓰기 함수는 ViewManager 락을 **짧게 잡아 변형 + 필요한 데이터(스냅샷·탭목록)를 복사**하고 **락을
//! 드롭한 뒤** 알린다. 락 드롭은 스코프(`{ ... }`)로 강제한다. 락 보유 중 부르는 포트는
//! `SubscriptionSync` 하나뿐(동기·비블로킹 계약)이고 `LayoutEvents`·`WindowHost`·`AgentSpawner` 는 항상
//! 락 밖이다. **두 규칙의 근거가 다르다:**
//! - **락 보유 중 외부 호출 0**(알림·OS 창·데몬 왕복·await) = ADR-0006 의 원칙(「lock 보유 중 외부 호출
//!   금지」)을 이 락에 적용한 것. ★그 ADR 본문은 코어의 sessions/status 락만 다룬다 — 레이아웃 락이나
//!   구독 델타 조항을 거기서 찾지 말 것.★
//! - **구독 재동기만 락 안** = ADR 이 아니라 `output_router::rebuild` 의 호출 계약(load→계산→store 의
//!   RMW 직렬화)과 그 위에 얹힌 F1/F2 수정이 근거다. 정본 = 그 함수 주석.
//!
//! invalid view_id/slot_id/window → no-op + Err(String)(패닉·부분변경 금지).
//!
//! ## 함수가 받는 포트 = 그 명령이 건드리는 것
//! 인자에 없는 포트는 **그 경로가 그것을 건드리지 않는다는 뜻**이다 — `focus_slot`·`rename_tab` 에
//! `SubscriptionSync` 가 없는 것은 누락이 아니라 라우팅 불변(ADR-0066)의 표현이고, `close_window` 에
//! `LayoutEvents` 가 없는 것은 그 경로가 프론트에 아무것도 안 쏜다는 사실이다.
//!
//! read-only 4종(`get_view`·`list_tabs`·`list_windows`·`resolve_spatial`)은 변형이 없어 포트를 하나도
//! 받지 않는다(ADR-0150 결정 2로 v1 명령 범위에 합류).

use std::future::Future;
use std::pin::Pin;

use uuid::Uuid;

use super::manager::{
    resolve_spawn_slot, CloseTabOutcome, ViewManager, WindowTabsSnapshot, MAIN_WINDOW_LABEL,
};
use super::spatial::{resolve_spatial as resolve_spatial_token, SpatialToken};
use super::types::{SlotContent, SplitDir, ViewMeta, ViewSnapshot};
use super::LayoutState;

/// 창별 탭바 알림 페이로드(ADR-0057). 프론트는 `label` 이 자기 창과 일치할 때만 반응하고(§7-1),
/// `version` 으로 stale 알림을 폐기한다(G10). 필드 이름은 wire 계약 — 프론트가 이 이름으로 읽는다.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct WindowTabsPayload {
    pub label: String,
    pub tabs: Vec<ViewMeta>,
    pub active: Uuid,
    pub version: u64,
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

/// 라우팅 표 재계산 + 더 이상 어느 창에도 안 보이는 agent 의 구독 해제 발화를 한 동작으로 묶은 포트.
///
/// ★호출 규약★: 적용 서비스가 **ViewManager 락 보유 중** 부른다 — 재계산과 발화가 같은 임계구역에
/// 있어야 "계산 순서 = 발화 순서"가 서고, 사이가 벌어지면 그 틈에 재추가된 agent 를 stale 해제가
/// 죽인다(F1/F2). ★이 「락 안」 요구의 근거는 ADR 조항이 아니라 `output_router::rebuild` 의 호출
/// 계약(RMW 직렬화)이다★ — 정본은 그 함수 주석이고, ADR-0006 에는 구독 델타 조항이 없다.
/// 그래서 구현은 동기·비블로킹이어야 한다(await·network 0 — 락 보유 중 외부 호출 금지라는 ADR-0006 원칙).
pub trait SubscriptionSync: Send + Sync {
    fn resync(&self, mgr: &ViewManager);
}

/// 프론트 알림 포트. ★락 미보유 상태에서만 불린다★(락 보유 중 외부 호출 금지 — ADR-0006 원칙). 실패는
/// 구현이 삼킨다 — 알림 유실은 레이아웃 변형을 되돌릴 사유가 아니다(프론트가 read-only pull 로 복구한다,
/// §3-3).
pub trait LayoutEvents: Send + Sync {
    fn layout_updated(&self, snapshot: &ViewSnapshot);
    fn window_tabs_updated(&self, tabs: &WindowTabsPayload);
}

/// OS 창 호스트 포트. ★둘 다 락 밖에서만 불린다 — 락 안으로 옮기면 그 자리에서 교착이다★.
///
/// - `open`: 창 빌드가 이벤트 루프·락을 요구한다.
/// - `close`: 구현이 OS 창을 destroy 하면 그 창의 `Destroyed` 이벤트 처리기가 **같은 ViewManager 락**을
///   다시 잡는다(`popout::destroy_window` → `Destroyed` → `cleanup_popup_window` → `state.0.lock()`).
///   워커 스레드가 가드를 쥔 채 부르면 destroy 는 이벤트 루프를 기다리고 이벤트 루프는 그 가드를
///   기다린다. 이미 닫힌 label 이면 no-op(호출자가 존재를 확인하지 않는다).
pub trait WindowHost: Send + Sync {
    fn open(&self, label: &str) -> Result<(), String>;
    fn close(&self, label: &str);
}

/// 새 창 label 발급 포트. ★단조★ — 닫힌 label 을 재사용하면 그 label 의 창을 다시 만들 수 없다.
/// `WindowHost` 와 가른 이유: 창을 닫기만 하는 경로(`close_tab`·`close_window`)는 발급기를 손에 쥐지
/// 않는다 — 포트를 합치면 그 경로가 안 쓰는 의존을 끌고 와야 한다.
pub trait LabelSource: Send + Sync {
    fn next_label(&self) -> String;
}

/// 에이전트 스폰 포트 — 성공 시 agent id. ★락 미보유 상태에서 await 된다★(데몬 왕복).
///
/// 반환 오류는 그대로 호출자에게 나간다(fail-loud). 응답 해석(어떤 프레임이 성공인가)은 구현 몫이다 —
/// 그건 전송 계약이라 이 서비스가 알 일이 아니다.
pub trait AgentSpawner: Send + Sync {
    fn spawn_by_cwd<'a>(
        &'a self,
        cwd: String,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>>;
}

// 창이 이미 없으면 None → 탭 알림 스킵(예: 팝업 마지막 탭 close 로 창 엔트리가 방금 제거됨).
fn tabs_payload(mgr: &ViewManager, label: &str) -> Option<WindowTabsPayload> {
    mgr.list_tabs(label).ok().map(Into::into)
}

fn owner_tabs(mgr: &ViewManager, view_id: Uuid) -> Option<WindowTabsPayload> {
    mgr.owner_of(view_id)
        .cloned()
        .and_then(|label| tabs_payload(mgr, &label))
}

fn notify(
    events: &dyn LayoutEvents,
    layout: Option<ViewSnapshot>,
    tabs: Option<WindowTabsPayload>,
) {
    if let Some(snap) = layout {
        events.layout_updated(&snap);
    }
    if let Some(tabs) = tabs {
        events.window_tabs_updated(&tabs);
    }
}

// ── 쓰기 12종 ────────────────────────────────────────────────────────────────

pub fn create_tab(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    window: &str,
    name: Option<String>,
) -> Result<Uuid, String> {
    let (id, layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        let id = mgr.create_tab(window, name).map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(id).ok();
        let tabs = tabs_payload(&mgr, window);
        subs.resync(&mgr);
        (id, layout, tabs)
    }; // ← 락 드롭
    notify(events, layout, tabs);
    Ok(id)
}

// 빈 새 창(빈 탭 1개) + OS 창. 성공 시 새 창 label.
//
// ★창 수 상한 근접 로그(ADR-0056/§4-2)★: 보이는 슬롯 상한(≤16)에 다가가는 것을 로그로만 남긴다 —
// 하드 블록이 아니다(초과 레이아웃은 프론트 onContextLoss→DOM graceful degrade + ADR-0056 재검토 트리거).
//
// 창 mount 시 프론트가 `list_tabs(label)` pull + 탭 알림 listen 으로 초기 렌더하므로(§3-3) 성공 경로에
// 별도 알림이 없다.
pub fn create_window(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    host: &dyn WindowHost,
    labels: &dyn LabelSource,
) -> Result<String, String> {
    let label = labels.next_label();

    {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.create_window(&label).map_err(|e| e.to_string())?;
        subs.resync(&mgr);
        let n_windows = mgr.windows.len();
        if n_windows >= 3 {
            tracing::info!(
                windows = n_windows,
                "create_window: 창 수 증가 — 보이는 슬롯 상한(≤16, ADR-0056) 근접 주의"
            );
        }
    } // ← 락 드롭

    if let Err(e) = host.open(&label) {
        // ★롤백 재동기도 락 안★: 락 밖으로 내면 계산~발화 사이에 그 agent 가 다시 배정돼 stale 1→0
        //   해제가 방금 형성된 라이브 구독을 죽인다(F1/F2 와 같은 클래스 — 되돌리지 말 것). 근거는
        //   `output_router::rebuild` 의 호출 계약이고, 그 주석이 이 롤백 경로를 **유일한 예외**로 적어
        //   두었던 것을 이 서비스가 없앴다(ADR 조항이 아니다).
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        let _ = mgr.close_window(&label);
        subs.resync(&mgr);
        return Err(e);
    }

    tracing::info!(label = %label, "빈 새 창 생성 완료(create_window)");
    Ok(label)
}

// keep-alive 라 노출 집합이 안 바뀌지만(활성 표시만 바뀜) 계약상 재동기한다.
pub fn switch_tab(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    window: &str,
    view: Uuid,
) -> Result<(), String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.switch_tab(window, view).map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view).ok();
        let tabs = tabs_payload(&mgr, window);
        subs.resync(&mgr);
        (layout, tabs)
    }; // ← 락 드롭
    notify(events, layout, tabs);
    Ok(())
}

// ★마지막 탭 close 로 창이 사라지면 OS 창 닫기는 여기가 단일 소스(§5-2/G2)★ — `windows` 엔트리는
// `close_tab` 이 이미 지웠으니 OS 창을 닫는다. 그 뒤 Destroyed 이벤트가 registry/Channel 잔여를 정리한다
// (프론트로 별도 `view:closed` 를 쏘지 않는다 — 이중 발화 방지).
pub fn close_tab(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    window: &str,
    view: Uuid,
) -> Result<(), String> {
    let (outcome, layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        let outcome = mgr.close_tab(window, view).map_err(|e| e.to_string())?;
        let (layout, tabs) = match outcome {
            CloseTabOutcome::Stayed => {
                let active = mgr.list_tabs(window).ok().map(|s| s.active);
                let layout = active.and_then(|a| mgr.snapshot(a).ok());
                (layout, tabs_payload(&mgr, window))
            }
            CloseTabOutcome::WindowClosed => (None, None),
        };
        subs.resync(&mgr);
        (outcome, layout, tabs)
    }; // ← 락 드롭

    match outcome {
        CloseTabOutcome::Stayed => notify(events, layout, tabs),
        CloseTabOutcome::WindowClosed => host.close(window),
    }
    Ok(())
}

// main 창 거부 가드를 여기 두지 않는다 — 모델(`ViewManager`)이 SSOT 라 그 Err 를 문자열로 전달만 한다.
pub fn close_window(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    host: &dyn WindowHost,
    window: &str,
) -> Result<(), String> {
    {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.close_window(window).map_err(|e| e.to_string())?;
        subs.resync(&mgr);
    } // ← 락 드롭
    host.close(window);
    Ok(())
}

pub fn split_slot(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
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
        let tabs = owner_tabs(&mgr, view_id);
        subs.resync(&mgr);
        (new_id, layout, tabs)
    }; // ← 락 드롭
    notify(events, layout, tabs);
    Ok(new_id)
}

pub fn close_slot(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    view_id: Uuid,
    slot_id: Uuid,
) -> Result<(), String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.close_slot(view_id, slot_id)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        let tabs = owner_tabs(&mgr, view_id);
        subs.resync(&mgr);
        (layout, tabs)
    }; // ← 락 드롭
    notify(events, layout, tabs);
    Ok(())
}

// ★라우팅 불변 → 재동기 없음(그래서 `SubscriptionSync` 를 안 받는다)★: 포커스 이동은 어느 슬롯이 어떤
// agent 를 보는지(=출력 라우팅)를 바꾸지 않는다.
// ADR-0066
pub fn focus_slot(
    state: &LayoutState,
    events: &dyn LayoutEvents,
    view_id: Uuid,
    slot_id: Uuid,
) -> Result<(), String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.set_focused_slot(view_id, slot_id)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        // 탭 이름·목록은 안 바뀌나 형제 명령과 동형으로 창 탭바도 계약상 갱신(view_owner 파생).
        let tabs = owner_tabs(&mgr, view_id);
        (layout, tabs)
    }; // ← 락 드롭
    notify(events, layout, tabs);
    Ok(())
}

// ★탭 알림만★: 이름은 `ViewMeta.name`(= 탭 페이로드)에만 있고 `ViewSnapshot` 엔 없다 → 레이아웃
// 스냅샷을 안 쏜다. 그리고 rename 은 출력 라우팅도 레이아웃 트리도 안 바꿔 재동기가 필요 없다
// (`focus_slot` 의 "라우팅 불변"과 동형이나, focus 는 레이아웃을 쏘고 rename 은 탭만 쏜다).
// ADR-0057
pub fn rename_tab(
    state: &LayoutState,
    events: &dyn LayoutEvents,
    view_id: Uuid,
    name: String,
) -> Result<(), String> {
    let tabs = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.rename_tab(view_id, name).map_err(|e| e.to_string())?;
        owner_tabs(&mgr, view_id)
    }; // ← 락 드롭
    if let Some(tabs) = tabs {
        events.window_tabs_updated(&tabs);
    }
    Ok(())
}

pub fn assign_agent(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    view_id: Uuid,
    slot_id: Uuid,
    agent_id: String,
) -> Result<(), String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.assign_agent(view_id, slot_id, agent_id)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        let tabs = owner_tabs(&mgr, view_id);
        subs.resync(&mgr);
        (layout, tabs)
    }; // ← 락 드롭
    notify(events, layout, tabs);
    Ok(())
}

pub fn set_slot_content(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    view_id: Uuid,
    slot_id: Uuid,
    content: SlotContent,
) -> Result<(), String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        mgr.set_slot_content(view_id, slot_id, content)
            .map_err(|e| e.to_string())?;
        let layout = mgr.snapshot(view_id).ok();
        let tabs = owner_tabs(&mgr, view_id);
        subs.resync(&mgr);
        (layout, tabs)
    }; // ← 락 드롭
    notify(events, layout, tabs);
    Ok(())
}

// ★spawn_into(D-7) — 스폰 + 탭 생성(필요 시) + 슬롯 배정을 한 방으로 조립★(TRD §6 · G9). 성공 시 새 agent id.
//
// ## ★ordering(ADR-0006 동시성 계약 — CRITICAL)★
// 스폰은 데몬 왕복(async)이라 **ViewManager 락을 잡지 않은 채** 먼저 끝낸다. 그 다음에만 락을 잡아
// (탭/슬롯 해소 + 점유 검사 + 배정 + 재동기)를 단일 임계구역으로 돌리고, 알림은 락을 드롭한 뒤 한다
// (락 보유 중 await/알림 0).
//
// ## ★슬롯 정책(G9 — 추측 금지, USER DECISION 2b)★
// - `tab=None`: 먼저 새 탭(빈 root 슬롯)을 만들고 거기 배정. (`slot=Some` 동반은 ★스폰 전에 거부★ —
//   새로 만들 탭엔 그 slot 이 없어 orphan 탭이 생긴다. 아래 pre-spawn 가드.)
//
// ## ★실패 가시성(§5 손발-두뇌 분리 — spawn-first)★
// 스폰이 먼저 일어나므로, 이후 배치(점유 슬롯·invalid view/window 등)가 실패해도 **에이전트를 kill 하지
// 않는다**(하드 롤백 없음). 에이전트는 데몬에 살아 있고 목록 조회로 재부착 가능하다 — 스폰 뒤 모든
// early-return 은 `alive_err` 로 생존 agent id 를 박아 invisible 에이전트를 막는다(락 획득 실패 포함).
//
// ## ★backend fail-loud(USER DECISION 1a — ADR-0058)★
// 현 데몬 스폰 wire 는 **cwd 만** 받고 backend 선택 인자가 없다 → 요청한 `backend` 는 데몬까지 흐르지
// 못하고 데몬은 무조건 기본 백엔드(현재 셸)를 스폰한다. 그래서 **명시된 backend 요청은 스폰 전에
// 거부**한다(호출자가 원한 것과 다른 에이전트를 조용히 받는 것 방지). 통과 = `backend`
// 미지정(`None`/빈/공백)뿐 — **`"claude"` 포함 어떤 명시값도 거부**(현재 스폰되는 건 셸이므로 "claude
// 지원"은 거짓말). backend 선택은 데몬 spawn-protocol 확장이 필요하다(미구현 — 별도 ADR/후속).
pub async fn spawn_into(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    spawner: &dyn AgentSpawner,
    window: &str,
    tab: Option<Uuid>,
    slot: Option<Uuid>,
    backend: Option<String>,
    cwd: String,
) -> Result<String, String> {
    // ── 0) 스폰 전 검증(에이전트 생성 이전이라 alive_err 불필요 — 아직 아무것도 안 죽음) ──────────────
    // ADR-0058 FIX 1(1a)
    if let Some(b) = &backend {
        let norm = b.trim();
        if !norm.is_empty() {
            return Err(format!(
                "backend '{b}' 선택은 아직 spawn_into 로 지원되지 않음 — 데몬 SpawnByCwd 는 항상 기본 백엔드(현재 셸)를 스폰하며 backend 선택 wire 가 없다(데몬 spawn-protocol 확장 필요, 후속). backend 를 생략하면 기본 백엔드로 스폰된다. 스폰 안 함."
            ));
        }
    }
    if tab.is_none() && slot.is_some() {
        return Err(
            "새로 생성될 탭에 특정 slot 을 지정할 수 없음 — slot 을 생략하거나 tab 을 지정하시오. 스폰 안 함."
                .to_string(),
        );
    }

    // ── 1) 스폰(락 미보유 async — 데몬 왕복) ──────────────────────────────────────────────────
    let agent_id = spawner.spawn_by_cwd(cwd).await?;

    // ── 2) 배치(락 보유 단일 임계구역) ────────────────────────────────────────────────────────
    let alive_err = |detail: String| {
        format!("배치 실패({detail}) — 에이전트 {agent_id} 는 살아있음(list_agents 로 재부착 가능)")
    };
    let (view_id, layout, tabs) = {
        // 락 획득 실패(mutex poison)도 생존 agent id 를 남긴다(FIX 4 — id 유실 금지).
        let mut mgr = state
            .0
            .lock()
            .map_err(|e| alive_err(format!("레이아웃 락 획득 실패: {e}")))?;

        let view_id = match tab {
            Some(v) => {
                if mgr.owner_of(v).map(|l| l.as_str()) != Some(window) {
                    return Err(alive_err(format!("view {v} 가 창 {window} 의 탭이 아님")));
                }
                v
            }
            // tab=None → 새 탭(빈 root 1개). slot 은 위 가드에서 None 확정 → resolve 는 항상 그 빈 root 로
            // 성공한다(orphan 탭 불가 — FIX 3). create_tab 만이 여기서 유일한 실패 지점(창 부재).
            None => mgr
                .create_tab(window, None)
                .map_err(|e| alive_err(e.to_string()))?,
        };

        let view = mgr
            .views
            .get(&view_id)
            .ok_or_else(|| alive_err(format!("view {view_id} 없음")))?;
        let target_slot = resolve_spawn_slot(view, slot).map_err(|e| alive_err(e.to_string()))?;

        // 배정(점유 검사는 위 resolve 가 이미 함 — assign 은 빈 슬롯 확정 후에만 닿음).
        mgr.assign_agent(view_id, target_slot, agent_id.clone())
            .map_err(|e| alive_err(e.to_string()))?;

        let layout = mgr.snapshot(view_id).ok();
        let tabs = tabs_payload(&mgr, window);
        subs.resync(&mgr);
        (view_id, layout, tabs)
    }; // ← 락 드롭
    notify(events, layout, tabs);

    tracing::info!(agent = %agent_id, window = %window, view = %view_id, "spawn_into 완료(스폰+배치)");
    Ok(agent_id)
}

// ── read-only 4종 ────────────────────────────────────────────────────────────

// 팝업 pull↔listen race 용. ★조회만★ — 변형 없음, 알림 없음(version 안 올림).
pub fn get_view(state: &LayoutState, view_id: Uuid) -> Result<ViewSnapshot, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    mgr.snapshot(view_id).map_err(|e| e.to_string())
}

// 왜 필요한가: 창이 mount 되면 자기 활성 탭을 확정해야 하는데(팝업은 자기 창 label 로만 자기 창을 알 뿐
// 활성 탭은 백엔드가 권위), 쓰기 경로는 변경 직후에만 알린다 → 부팅/mount 직후엔 이 조회로
// `{tabs,active,version}` 을 받아 초기 렌더한다(§3-3/G3).
pub fn list_tabs(state: &LayoutState, window: &str) -> Result<WindowTabsPayload, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    mgr.list_tabs(window)
        .map(Into::into)
        .map_err(|e| e.to_string())
}

pub fn list_windows(state: &LayoutState) -> Result<Vec<String>, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    Ok(mgr.list_windows())
}

// ★공간/방향 토큰 → slot id 해소(백엔드 권위 resolver)★. `view_id` 지정이면 그 View, 미지정이면
// `window`(미지정 시 main) 의 활성 탭 View 를 대상으로 한다. 모르는 토큰/없는 View → Err(fail-loud).
// ADR-0068
pub fn resolve_spatial(
    state: &LayoutState,
    token: &str,
    window: Option<&str>,
    view_id: Option<Uuid>,
) -> Result<Option<Uuid>, String> {
    let tok = SpatialToken::parse(token)
        .ok_or_else(|| format!("알 수 없는 공간 토큰: '{token}' (top-left/top-right/bottom-left/bottom-right/left/right/up/down)"))?;
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    let vid = match view_id {
        Some(v) => v,
        None => {
            let label = window.unwrap_or(MAIN_WINDOW_LABEL);
            mgr.list_tabs(label).map_err(|e| e.to_string())?.active
        }
    };
    let v = mgr
        .views
        .get(&vid)
        .ok_or_else(|| format!("view 없음: {vid}"))?;
    Ok(resolve_spatial_token(&v.layout, v.focused_slot_id, tok))
}
