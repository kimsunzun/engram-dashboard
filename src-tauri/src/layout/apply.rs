//! 레이아웃 적용 서비스 — 사람 클릭(Tauri invoke)과 중계된 LLM 호출이 **같은 함수**에 떨어지는 전송
//! 중립 단일 경로(ADR-0081 결정 3). ADR-0081 「거부한 대안」이 거부한 형태 = 호출자가 `ViewManager` 를
//! 직접 잠그고 부르는 것(사람 경로와 LLM 경로가 갈려 2 코드 경로가 된다) — 이 모듈이 그 유일한 대안이다.
//!
//! ★Tauri 의존 0★: 전부 `AppHandle`·`State<..>` 를 인자에서 걷어낸 자유 함수다 — 창 없이 단독 호출·
//! 테스트가 서야 하기 때문이다(TRD S20 §7 「셸 적용 서비스」). 외부 세계(프론트 알림·OS 창·데몬 왕복·
//! 구독 표)에는 아래 포트 5개로만 닿는다. `#[tauri::command]` 껍데기는 `commands/layout.rs`.
//!
//! ## ★락 규율★
//! 쓰기 함수는 ViewManager 락을 **짧게 잡아 변형 + 필요한 데이터(스냅샷·탭목록)를 복사**하고 **락을
//! 드롭한 뒤** 알린다. 락 드롭은 스코프(`{ ... }`)로 강제한다. 락 보유 중 부르는 포트는
//! `SubscriptionSync` 하나뿐(동기·비블로킹 계약)이고 나머지 넷(`LayoutEvents`·`WindowHost`·`LabelSource`·
//! `AgentSpawner`)은 항상 락 밖이다. **두 규칙의 근거가 다르다:**
//! - **락 보유 중 외부 호출 0**(알림·OS 창·데몬 왕복·await) = ADR-0006 의 원칙(「lock 보유 중 외부 호출
//!   금지」)을 이 락에 적용한 것. ★그 ADR 본문은 코어의 sessions/status 락만 다룬다 — 레이아웃 락이나
//!   구독 델타 조항을 거기서 찾지 말 것.★
//! - **구독 재동기만 락 안** = ADR 이 아니라 `output_router::rebuild` 의 호출 계약(load→계산→store 의
//!   RMW 직렬화)과 그 위에 얹힌 F1/F2 수정이 근거다. 정본 = 그 함수 주석.
//!
//! invalid view_id/slot_id/window → no-op + Err(패닉·부분변경 금지). 오류는 문자열이고, 도킹(`dock_tab`)만 종류로
//! 가를 수 있는 `DockError` 다.
//!
//! ## 함수가 받는 포트 = 그 명령이 건드리는 것
//! 인자에 없는 포트는 **그 경로가 그것을 건드리지 않는다는 뜻**이다 — `focus_slot`·`rename_tab`·
//! `set_split_ratio` 에 `SubscriptionSync` 가 없는 것은 누락이 아니라 라우팅 불변(ADR-0066)의 표현이고,
//! `close_window` 에 `LayoutEvents` 가 없는 것은 그 경로가 프론트에 아무것도 안 쏜다는 사실이다.
//!
//! read-only 5종(`get_view`·`list_tabs`·`list_windows`·`resolve_spatial`·`list_splits`)은 변형이 없어
//! 포트를 하나도 받지 않는다(앞 넷은 ADR-0156 결정 2로 v1 명령 범위에 합류 · `list_splits` 는 ADR-0227).
//!
//! 측정 보고 2종(`report_window_canvas`·`report_ui_metrics`)도 포트를 받지 않는다 — 셸
//! 상태에 쓰지만 version·알림·구독 어느 것도 건드리지 않는다(측정이지 제어가 아니다 — ADR-0227).

use std::future::Future;
use std::pin::Pin;

use engram_dashboard_base::sync;
use engram_dashboard_protocol::AgentBackendKind;
use uuid::Uuid;

use super::dock::{DockAnchor, DockSide};
use super::manager::{
    CloseTabOutcome, DockError, DockMove, DockTo, Docked, FocusPolicy, NewWindowPlan, ViewId,
    ViewManager, WindowLabel, WindowTabsSnapshot, MAIN_WINDOW_LABEL,
};
use super::spatial::{resolve_spatial as resolve_spatial_token, SpatialToken};
use super::tree::{self, SplitInfo};
use super::types::{
    SlotContent, SplitDir, SplitRatioApplied, SplitRatioOutcome, UiMetrics, ViewMeta, ViewSnapshot,
};
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
///
/// ★레이아웃에서 파생되는 사용량 관심의 재계산도 같은 자리다(TRD S21 usage-limit-slot §1-7)★ — 이 포트를 받는
/// 쓰기가 곧 슬롯 내용·탭·창을 바꾸는 쓰기라, 관심이 바뀔 수 있는 자리가 전부 여기로 온다(포트를 안 받는 쓰기는
/// 관심도 안 바꾼다 — 모듈 머리 「함수가 받는 포트」). 그 재계산이 하는 일(메모리 안 계산 · 비블로킹 넛지 ·
/// 타이머 태스크 기동)도 위 계약 안이다.
pub trait SubscriptionSync: Send + Sync {
    fn resync(&self, mgr: &ViewManager);
}

/// 프론트 알림 포트. ★락 미보유 상태에서만 불린다★(락 보유 중 외부 호출 금지 — ADR-0006 원칙). 실패는
/// 구현이 삼킨다 — 알림 유실은 레이아웃 변형을 되돌릴 사유가 아니다(프론트가 read-only pull 로 복구한다,
/// §3-3).
pub trait LayoutEvents: Send + Sync {
    fn layout_updated(&self, snapshot: &ViewSnapshot);
    fn window_tabs_updated(&self, tabs: &WindowTabsPayload);
    /// 쓰기 하나의 알림 묶음 끝 — 그 쓰기가 낼 알림을 다 낸 뒤 정확히 한 번 불린다([`announce`]). 알린 것이 없는
    /// 쓰기(실패 · 무변경 결말)에는 불리지 않는다. 알림 하나마다가 아니라 묶음 끝이라, 받는 쪽은 그 쓰기의 알림이
    /// 다 나간 뒤의 레이아웃을 본다. ★운영 소비자는 지금 없다★ — 운영 구현(`TauriEvents`)은 아무것도 하지 않고, 이
    /// 계약을 재는 것은 시험 이중뿐이다.
    /// ★기본 본문을 두지 않는다★ — 두면 넘기기 어댑터가 빈 몸을 물려받아, 소비자가 다시 붙어도 계기가 말없이 끊긴다.
    // TRD S21-slot-docking §1-2 「알림 묶음 끝」
    fn settled(&self);
}

/// 창 포트가 창을 놓을 자리 — 도킹의 새 창 · 옮길 창(TRD S21-slot-docking §1-2 「적용 서비스」).
///
/// - `pos` = **물리** 바깥 왼쪽 위(`outer_position` 과 같은 단위 — 논리 위치는 배율이 다른 모니터가 섞이면 한 값이
///   두 모니터를 가리킨다 · `state::placement` 머리). `None` = 자리를 고르지 않는다.
/// - `size` = **논리** 안쪽 크기. `None` = 크기를 고르지 않는다.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowPlace {
    pub pos: Option<(i32, i32)>,
    pub size: Option<(f64, f64)>,
}

/// 창 포트의 세터 한 걸음 — 입히는 순서도 이 순서다(위치 → 크기).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveStep {
    Position,
    Size,
}

/// 못 입힌 걸음과 그 사유(OS · 런타임이 준 문장).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveFailed {
    pub step: MoveStep,
    pub error: String,
}

/// 창 포트가 한 호출자에게 맡긴 숨은 예열 창의 맡김표 — [`WindowHost::claim_spare`] 가 주고, 부른 쪽이
/// [`WindowHost::committed`] · [`WindowHost::release_spare`] 중 하나로 한 번 돌려준다(맡긴 창의 `open` · `open_at` 이
/// `Err` 면 어느 쪽도 부르지 않는다 — 창 포트가 그 창을 이미 거뒀다).
///
/// - `label` = 맡긴 창의 label — 새 창은 이 label 로 연다([`LabelSource::next_label`] 을 부르지 않는다).
/// - `generation` = 맡길 때마다 새로 뽑는 채택 번호. 창 포트는 맡김표가 지금의 맡김과 label · 번호 둘 다 같을 때만
///   듣는다 — 되물린 뒤 같은 label 을 다시 맡겼으면 앞 맡김표는 낡아 무동작이다.
// TRD S21-slot-docking §3-9 「모양」 · I27
// ADR-0299
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpareClaim {
    pub label: WindowLabel,
    pub generation: u32,
}

/// OS 창 호스트 포트. ★전부 락 밖에서만 불린다★ — 창 만들기 · 세터 · 게터는 OS 호출이고(「락 규율」), 아래
/// `open` · `record_placement` · `close` · `committed` 는 락 안으로 옮기면 그 자리에서 교착이다.
///
/// - `open`: 창 빌드가 이벤트 루프·락을 요구한다.
/// - `record_placement`: 구현이 창 게터(OS 호출)를 부른 뒤 같은 ViewManager 락을 잡아 적는다.
/// - `committed`: 구현이 같은 ViewManager 락을 잡아 맡긴 창이 쥔 재기를 모델로 옮긴다(TRD S21-slot-docking §3-9
///   「`committed`」).
/// - `close`: 구현이 OS 창을 destroy 하면 그 창의 `Destroyed` 이벤트 처리기가 **같은 ViewManager 락**을
///   다시 잡는다(`popout::destroy_window` → `Destroyed` → `cleanup_popup_window` → `state.0.lock()`).
///   워커 스레드가 가드를 쥔 채 부르면 destroy 는 이벤트 루프를 기다리고 이벤트 루프는 그 가드를
///   기다린다. 이미 닫힌 label 이면 no-op(호출자가 존재를 확인하지 않는다).
/// - `is_open`: **OS 창**이 살아 있나 — 모델(`ViewManager`)이 아니다. 둘은 갈릴 수 있다(닫기 요청 없이 OS
///   창이 사라졌으나 `Destroyed` 정리가 아직 모델 엔트리를 안 지운 순간) — 그 순간을 거르는 것은 이 답뿐이고,
///   `insert_tab_into` 의 모델 재검증은 그 상태를 통과시킨다.
///
/// pop-out 은 창 확인과 모델 변형 사이에서 락을 한 번 놓는다. 그 틈에 사용자가 대상 창을 닫아도 잔해가 남지
/// 않는 것은 팝아웃의 닫기 요청이 모델에서 그 창을 먼저 지우고 label 에 묘비를 달기 때문이다(같은 락 한 번 —
/// `commands::popout::close_requested_popup_window` · TRD S21-slot-docking §1-2 「닫히는 창」). 기존 창 타깃은
/// phase C 의 `insert_tab_into` 가 창 없음으로, 새 창 타깃은 `attach_view_as_new_window` 가 묘비로 실패해 롤백을
/// 탄다(옮기려던 슬롯은 출처에 남는다). 그 전제(팝아웃의 닫기 요청은 되돌려지지 않는다)와 점검 명령은 `lib.rs` 의
/// `CloseRequested` 처리기에 있다.
///
/// ★`open_at` · `move_window` · 예열 창 셋(`claim_spare` · `committed` · `release_spare`)에 기본 본문을 두지 않는다★ —
/// 두면 넘기기 어댑터(`OwnedWindowHost`)가 물려받아 자리 · 크기 · 활성 · 예열 창을 말없이 버린다.
pub trait WindowHost: Send + Sync {
    fn open(&self, label: &str) -> Result<(), String>;
    /// [`Self::open`] 과 같되 자리 · 활성을 고른다. `at.pos` 가 없거나 어느 모니터에도 안 걸치면 label 의 기본
    /// 자리, `at.size` 가 없으면 팝아웃 기본 크기다. `activate = false` = 만들되 포커스를 가져가지 않는다.
    /// `Err` = 창을 못 만들었거나, 만든 뒤 고른 자리를 못 입혀 그 창을 거뒀다 — 어느 쪽이든 떠 있는 창이 없다(부르는
    /// 쪽은 모델을 바꾸지 않고 끝낸다).
    fn open_at(&self, label: &str, at: &WindowPlace, activate: bool) -> Result<(), String>;
    fn close(&self, label: &str);
    fn is_open(&self, label: &str) -> bool;
    /// 떠 있는 창을 `at` 으로 옮긴다 — 위치 → 크기, ★첫 실패에서 멈추고★ 그 걸음을 돌려준다. 빈 칸 · 어느
    /// 모니터에도 안 걸치는 위치는 그 걸음을 건너뛴다(창은 지금 자리). 걸침은 창이 옮긴 뒤 가질 크기로 잰다(`at.size`
    /// · 없으면 그 창의 보통 크기). 창이 없으면 위치 걸음의 실패다. 최대화된 창은 위치 걸음이 최대화를 푼다 — 보이는
    /// 창은 보통 크기로 보이고, 숨은 창은 `SW_RESTORE` 뒤 `SW_HIDE` 라 잠깐 깜빡이고 숨은 채 남는다(tao 0.35.3
    /// `platform_impl/windows/window_state.rs` `apply_diff` · GUI 미검증). 실패면 창이 반쯤 옮겨졌거나 최대화가 풀렸을
    /// 수 있다 — 되돌리는 포트는 없다(재사용 도킹은 놓은 자리에 둔다 · [`dock_tab`]).
    fn move_window(&self, label: &str, at: &WindowPlace) -> Result<(), MoveFailed>;
    /// `open` · `open_at` 으로 연 창이 모델에 든 뒤 한 번 — 그 창의 첫 자리를 모델에 적는다. 한 번도 끌지 않은 창도
    /// 다음 부팅에 그 자리로 연다(TRD S21-storage §6-3). 재사용 도킹으로 옮긴 창도 옮긴 뒤 한 번 부른다(`dock_tab`).
    /// 기본 = 아무것도 안 한다.
    fn record_placement(&self, _label: &str) {}
    /// 새 label 로 창을 열기 전에 묻는다 — 이 호출자에게 맡긴 숨은 예열 창, 없으면 `None`(새 label 로 새로 짓는다).
    /// 활성으로 열든 아니든 묻는다. 맡긴 창은 모델 밖에 숨은 채 남고, 그 label 의 `open` · `open_at` 은 짓지 않고 그
    /// 창을 놓는다 — `Err` 계약은 같다(그 창을 거둔 뒤다). ★메인 스레드에서 부르지 말 것★ — 짓는 중인 예열 창을
    /// 기다리는데 그 짓기가 메인 스레드에서 돈다(TRD S21-slot-docking §3-9 「포트」 I15).
    fn claim_spare(&self) -> Option<SpareClaim>;
    /// 맡긴 창의 새 창 항목이 모델에 들고 그 알림을 냈다 — 이제 그 창을 그리고 보여도 된다. `record_placement` 보다
    /// 먼저 부른다. 그사이 그 label 이 모델에서 빠졌어도 부른다(그 창을 닫는 것은 구현 몫). 낡은 맡김표면 무동작.
    fn committed(&self, claim: &SpareClaim);
    /// 맡긴 창을 확정 전에 되물린다(모델에 넣지 못한 롤백) — 숨은 채 다시 맡길 수 있게 된다. 낡은 맡김표면 무동작.
    /// 새로 지은 창의 롤백은 [`Self::close`] 다 — `close` 는 예열 창 label 이어도 늘 닫는다.
    fn release_spare(&self, claim: &SpareClaim);
}

/// 새 창 label 발급 포트. ★단조★ — 닫힌 label 을 재사용하면 그 label 의 창을 다시 만들 수 없다.
/// `WindowHost` 와 가른 이유: 창을 닫기만 하는 경로(`close_tab`·`close_window`)는 발급기를 손에 쥐지
/// 않는다 — 포트를 합치면 그 경로가 안 쓰는 의존을 끌고 와야 한다.
///
/// ★둘 다 락 밖에서만 불린다★(형제 포트와 같은 규율 — 「락 규율」). 오늘 구현이 순수하다는 것에 기대지
/// 말 것: 상태를 만지는 구현이 락 안에서 불리면 `Destroyed → cleanup_popup_window → state.0.lock()` 와
/// 교착한다.
pub trait LabelSource: Send + Sync {
    fn next_label(&self) -> String;
    /// 그 label 로 연 창의 첫 탭에 붙일 이름. ★label 형식을 아는 쪽이 짓는다★ — 적용 서비스는 label 이
    /// 어떤 모양인지 모르고(그 prefix 는 Tauri 쪽 상수다), 그걸 여기서 다시 파싱하면 형식이 두 곳에 산다.
    /// 인자는 **이 발급기가 낸 label** 을 전제하고, 그 밖의 문자열엔 label 을 그대로 쓴 이름을 준다.
    fn tab_name(&self, label: &str) -> String;
}

/// 에이전트 스폰 포트 — 성공 시 agent id. ★락 미보유 상태에서 await 된다★(데몬 왕복).
///
/// 반환 오류는 그대로 호출자에게 나간다(fail-loud). 응답 해석(어떤 프레임이 성공인가)은 구현 몫이다 —
/// 그건 전송 계약이라 이 서비스가 알 일이 아니다.
///
/// `backend` 가 **wire 어휘 그대로**인 이유: 여기서 자체 어휘를 세우면 셸이 통과시킨 낱말과 데몬이 아는
/// 낱말이 갈릴 수 있고, 그 갈림은 조용히 다른 백엔드가 뜨는 형태로만 발현한다. `None` = 고르지 않음 —
/// **데몬이 거절한다**(셸은 기본값을 지어내지 않는다).
pub trait AgentSpawner: Send + Sync {
    fn spawn_by_cwd<'a>(
        &'a self,
        cwd: String,
        backend: Option<AgentBackendKind>,
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
    announce(events, layout.as_ref(), tabs.as_ref());
}

/// 쓰기 하나의 알림 묶음을 낸다 — 레이아웃 스냅숏들 → 창 탭 목록들, 낸 것이 있으면 끝에서
/// [`LayoutEvents::settled`] 를 한 번. 낼 것이 없으면(무변경 결말) 끝도 내지 않는다. ★락을 놓은 뒤에만 부른다★
/// (「락 규율」). 알리는 쓰기는 전부 여기를 지난다 — 복원 수락(`state::restore`)도.
// TRD S21-slot-docking §1-2 「알림 묶음 끝」
pub(crate) fn announce<'a>(
    events: &dyn LayoutEvents,
    layouts: impl IntoIterator<Item = &'a ViewSnapshot>,
    tabs: impl IntoIterator<Item = &'a WindowTabsPayload>,
) {
    let mut sent = false;
    for snapshot in layouts {
        events.layout_updated(snapshot);
        sent = true;
    }
    for payload in tabs {
        events.window_tabs_updated(payload);
        sent = true;
    }
    if sent {
        events.settled();
    }
}

// ── 새 창 세 자리의 맡김 몫(TRD S21-slot-docking §3-9 「자리별 차례」) ─────────────
//
// 새 label 로 창을 여는 자리(`create_window` · `move_slot_to_window` · 도킹 새 창)는 셋 다 창 포트에 예열 창을 먼저
// 묻고, 맡긴 것이 있으면 그 맡김표를 끝까지 들고 간다 — 확정(`committed`)이든 되물림(`release_spare`)이든 한 번. 그
// 창의 열기가 `Err` 면 어느 쪽도 부르지 않는다 — 창 포트가 그 창을 이미 거뒀다(`SpareClaim` 문서).
// ADR-0299

// 새 창의 label — 맡긴 예열 창이 있으면 그 label, 없으면 발급기의 새 label.
fn new_window_label(
    host: &dyn WindowHost,
    labels: &dyn LabelSource,
) -> (WindowLabel, Option<SpareClaim>) {
    match host.claim_spare() {
        Some(claim) => (claim.label.clone(), Some(claim)),
        None => (labels.next_label(), None),
    }
}

// 연 새 창을 모델에 넣지 못했다 — 맡긴 예열 창은 되물리고 새로 지은 창은 닫는다. ★맡긴 창을 `close` 로 거두지 말
//   것★ — `close` 는 늘 닫아 멀쩡한 예열 창을 버리고 다시 짓게 한다(되물림은 맡김표로만 — TRD §3-9 I27).
fn withdraw_new_window(host: &dyn WindowHost, label: &str, claim: Option<&SpareClaim>) {
    match claim {
        Some(claim) => host.release_spare(claim),
        None => host.close(label),
    }
}

// 새 창 항목이 모델에 들고 알렸다 — 맡긴 창이면 확정하고, 그 뒤에 첫 자리를 적는다(첫 `window:frame` 이 확정 뒤에
//   난다 — TRD §3-8).
fn settle_new_window(host: &dyn WindowHost, label: &str, claim: Option<&SpareClaim>) {
    if let Some(claim) = claim {
        host.committed(claim);
    }
    host.record_placement(label);
}

// ── 쓰기 14종 ────────────────────────────────────────────────────────────────

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
//
// ★OS 창을 연 뒤에 모델에 넣는다★ — 새 창을 여는 다른 두 자리(`move_slot_to_window` · 도킹 새 창)와 같은 차례다(TRD
// S21-slot-docking §3-9 「자리별 차례」). 열기와 넣기 사이에 그 창이 닫히면(닫기 요청 · `Destroyed`) 그 정리가 label 에
// 묘비를 달아 넣기가 `WindowClosed` 로 거절되고, 연 창을 거둔다(이미 닫혔으면 무동작).
// ADR-0299
pub fn create_window(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    host: &dyn WindowHost,
    labels: &dyn LabelSource,
) -> Result<String, String> {
    let (label, claim) = new_window_label(host, labels);
    host.open(&label)?;

    let inserted = state
        .0
        .lock()
        .map_err(|e| e.to_string())
        .and_then(|mut mgr| {
            mgr.create_window(&label).map_err(|e| e.to_string())?;
            subs.resync(&mgr);
            Ok(mgr.windows.len())
        }); // ← 락 드롭
    let n_windows = match inserted {
        Ok(n) => n,
        Err(e) => {
            withdraw_new_window(host, &label, claim.as_ref());
            tracing::warn!(label = %label, "create_window: 연 창을 모델에 넣지 못해 거둔다 — {e}");
            return Err(e);
        }
    };
    if n_windows >= 3 {
        tracing::info!(
            windows = n_windows,
            "create_window: 창 수 증가 — 보이는 슬롯 상한(≤16, ADR-0056) 근접 주의"
        );
    }
    settle_new_window(host, &label, claim.as_ref());

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

/// 그 창 활성이 `expect` 일 때만 [`switch_tab`] — 바꿨으면 `true` 이고 알림 · 재동기도 `switch_tab` 과 같다. 활성이 달라
/// `false` 면 모델 · 구독 · 알림 무엇도 건드리지 않는다. 오류(없는 창 · 그 창 탭이 아닌 `view`)는 `switch_tab` 과 같다.
///
/// 탭 끌기의 원래 선택 되돌리기 전용 웹 → 셸 명령이다(TRD S21-slot-docking §4-6) — 되돌리기가 닿기 전에 사용자 · LLM 이
/// 고른 탭을 덮지 않는다. LLM 버스 표에는 없다(LLM 은 `tab.switch` 를 쓴다 · TRD §3-6).
pub fn switch_tab_if(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    window: &str,
    view: Uuid,
    expect: Uuid,
) -> Result<bool, String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        if !mgr
            .switch_tab_if(window, view, expect)
            .map_err(|e| e.to_string())?
        {
            drop(mgr);
            tracing::debug!(
                window,
                view = %view,
                expect = %expect,
                "switch_tab_if: 활성이 expect 가 아니라 바꾸지 않는다"
            );
            return Ok(false);
        }
        let layout = mgr.snapshot(view).ok();
        let tabs = tabs_payload(&mgr, window);
        subs.resync(&mgr);
        (layout, tabs)
    }; // ← 락 드롭
    notify(events, layout, tabs);
    Ok(true)
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

// 사람의 구분선 드래그(Tauri `set_split_ratio`)와 LLM 의 `split.setRatio` 가 같이 떨어지는 자리.
// ★레이아웃 알림은 `Applied` 일 때만, 탭 알림은 없다★ — 무변경 결말(`Unchanged`·`TooSmall`)을 알리면 화면이
// 바뀌지 않은 스냅샷을 또 받는다. 출력 라우팅이 안 바뀌어 재동기도 없다(`focus_slot` 과 같은 이유).
// ★반환 version 은 같은 임계 구역에서 뜬 스냅샷의 것이다★ — 락을 놓은 뒤 전역 카운터를 다시 읽으면 그
// 사이 끼어든 다른 쓰기의 값을 돌려줘, 화면이 통지되지 않을 version 을 기다린다.
// ADR-0227
pub fn set_split_ratio(
    state: &LayoutState,
    events: &dyn LayoutEvents,
    view_id: Uuid,
    split_id: Uuid,
    ratio: f64,
) -> Result<SplitRatioApplied, String> {
    let (applied, layout) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        let result = mgr
            .set_split_ratio(view_id, split_id, ratio)
            .map_err(|e| e.to_string())?;
        let layout = match result.outcome {
            SplitRatioOutcome::Applied => mgr.snapshot(view_id).ok(),
            SplitRatioOutcome::Unchanged | SplitRatioOutcome::TooSmall => None,
        };
        let version = layout.as_ref().map_or(mgr.version, |snap| snap.version);
        let applied = SplitRatioApplied {
            ratio: result.ratio,
            outcome: result.outcome,
            version,
        };
        (applied, layout)
    }; // ← 락 드롭
    notify(events, layout, None);
    Ok(applied)
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
    announce(events, None::<&ViewSnapshot>, tabs.as_ref());
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

/// 슬롯을 사용량 뷰로 만든다 — 빠진 칸(`None`)은 그 슬롯이 이미 `Usage` 면 지금 값, 아니면 `true`.
///
/// ★지금 값을 읽는 것과 쓰는 것이 한 락 안이다★ — 호출자가 먼저 읽어 온 값으로 병합하면 그 사이에 든 다른
/// 토글을 덮는다(한 칸만 바꾸는 호출이 다른 칸을 옛 값으로 되돌린다).
pub fn set_usage_slot(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    view_id: Uuid,
    slot_id: Uuid,
    show_claude: Option<bool>,
    show_codex: Option<bool>,
) -> Result<(), String> {
    let (layout, tabs) = {
        let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
        let (claude_now, codex_now) = match mgr
            .slot_content(view_id, slot_id)
            .map_err(|e| e.to_string())?
        {
            SlotContent::Usage {
                show_claude,
                show_codex,
            } => (show_claude, show_codex),
            _ => (true, true),
        };
        let content = SlotContent::Usage {
            show_claude: show_claude.unwrap_or(claude_now),
            show_codex: show_codex.unwrap_or(codex_now),
        };
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
// ## ★backend — 셸은 모르는 낱말만 막는다(오탈자 그물)★
// 데몬 스폰 wire 에 backend 칸이 생겼으므로(`SpawnByCwd.backend`) 요청은 이제 **데몬까지 흐른다**. 그래서
// **wire 가 아는 낱말이라는 이유만으로** 거부할 근거는 죽었다 — 예전의 전량 거부는 「고를 칸이 wire 에
// 없다」 위에 서 있었다(ADR-0058 폐기 대상). 셸에 남는 거절은 모르는 낱말 하나다(정책 축은 아래 — 데몬).
// ★그래도 그물을 통째로 걷지 않는다★: 오탈자(`codx`)를 그냥 흘리면 데몬이 그것을 모르는 낱말로 거절할
// 때 **스폰 왕복 한 번을 낭비한 뒤**에야 알려지고, 옛 데몬을 만나면 그 칸째 무시돼 조용히 다른 백엔드가
// 뜬다. ADR-0058 이 지키려던 성질(호출자가 원한 것과 다른 에이전트를 조용히 받지 않는다)은 살아 있고,
// 폐기되는 것은 그 성질을 지키던 수단(전량 거부)뿐이다.
// ★이 판정은 약속이 아니라 오탈자 그물이다★ — 통과시켰다고 그 백엔드가 뜬다는 보장은 여기가 못 한다
// (그건 데몬이 한다). 그래서 어휘를 손으로 적지 않고 **wire 어휘 자신에게 물어본다** — 두 곳에 적으면
// 갈린다.
// ★미지정은 여기서 안 막는다★ — 부재의 거절은 데몬이 지고(어느 칸을 채우라는 문구까지 데몬이 낸다),
// 셸이 그것을 흉내 내면 같은 규칙이 두 곳에 살게 된다.
//
// ## ★정책 축(아는 낱말이어도 지금 만드나)은 셸에 없다 — 데몬 `SpawnByCwd` 처리부가 진다★
// 백엔드 생성 정책(agent 의 `LLM_BACKEND_POLICY`)은 데몬이 스폰 전에 묻는다(ADR-0279). 그 거절은 아래 1)
// 스폰 단계의 `?` 로 돌아와 2) 배치에 닿지 않는다 — 레이아웃이 안 바뀐다. 문구는 데몬 것이 접두 없이 간다.
// ★정책 검사를 여기 되살리지 말 것★ — 그러려면 셸이 agent crate 를 다시 의존해야 한다(ADR-0270 결정 1 ·
// CI 게이트 `shell gate 1`).
// ★두 거절을 한 문구로 뭉치지 말 것★ — 데몬의 정책 거절은 「아는 낱말이지만 … 만들지 않는다」, 이 그물은
// 「를 모른다」다. 뭉치면 호출자가 있지도 않은 오탈자를 고치려 든다.
/// 오탈자 그물 — ★어휘를 손으로 적지 않는다★. 판정도 「무엇이 통하는가」 문구도 wire enum 자신에게서
/// 나오므로(serde 의 unknown-variant 오류가 기대 낱말을 나열한다) 여기와 데몬이 갈릴 수 없다.
// ADR-0270 · ADR-0279
fn parse_backend(word: &str) -> Result<AgentBackendKind, String> {
    serde_json::from_value::<AgentBackendKind>(serde_json::Value::String(word.to_string()))
        .map_err(|e| format!("backend '{word}' 를 모른다({e}). 스폰 안 함."))
}

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
    let backend = match backend.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        None => None,
        Some(word) => Some(parse_backend(word)?),
    };
    if tab.is_none() && slot.is_some() {
        return Err(
            "새로 생성될 탭에 특정 slot 을 지정할 수 없음 — slot 을 생략하거나 tab 을 지정하시오. 스폰 안 함."
                .to_string(),
        );
    }

    // ── 1) 스폰(락 미보유 async — 데몬 왕복) ──────────────────────────────────────────────────
    let agent_id = spawner.spawn_by_cwd(cwd, backend).await?;

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

        let target_slot = mgr
            .resolve_spawn_slot(view_id, slot)
            .map_err(|e| alive_err(e.to_string()))?;

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

/// 슬롯 분리(pop-out)의 결말 — 콘텐츠가 내려앉은 창 label + 그것을 실은 **새 탭**의 View id.
///
/// 필드 이름은 wire 계약이다 — 프론트(`viewStore.moveSlotToWindow`)가 `{window, tab}` 으로 읽는다.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SlotMove {
    pub window: String,
    pub tab: Uuid,
}

// ★슬롯을 다른 창의 새 탭으로 MOVE(mirror 아님)★: 원본 슬롯의 **콘텐츠(SlotContent)** 를 새 탭으로 옮기고
// 원본 슬롯을 원본 View 에서 제거한다. agent 프로세스는 안 건드린다 — 표시 표면만 이동한다.
// `to_window` 미지정 → 새 창(label = 창 포트가 맡긴 예열 창의 것 · 없으면 발급기의 새 것) · 지정 → 그 기존 창의 새 탭.
//
// ★ADR-0064 — 모든 슬롯 콘텐츠★: agent 슬롯뿐 아니라 agent_list/preset_palette 와 빈 슬롯(ADR-0228)도
// 옮긴다. 비-에이전트 콘텐츠는 백엔드 출력 구독이 없어 재동기가 자연히 no-op 이다(라우팅은 Agent 슬롯만
// 훑는다). 거절은 없는 view/slot 뿐이고 그 사유(`LayoutError`)를 그대로 돌려준다.
//
// ## ★2-phase 롤백 + 기존창 타깃 orphan 방지(G4)★
// 락이 풀린 사이 대상 창이 소멸/동시 close 될 수 있어 **탭 삽입을 phase C 로 이연**하고, 거기서 대상 창을
// 다시 확인한 뒤에만 삽입한다(부재면 롤백). 소스 detach 는 still-ours 가드로 2차 락에서 close.
// ADR-0057
// ADR-0064
// ADR-0228
pub fn move_slot_to_window(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    labels: &dyn LabelSource,
    view_id: Uuid,
    slot_id: Uuid,
    to_window: Option<String>,
) -> Result<SlotMove, String> {
    let is_new_window = to_window.is_none();
    let (target_label, claim) = match to_window {
        Some(label) => (label, None),
        None => new_window_label(host, labels),
    };
    // ★탭 이름은 락을 잡기 **전에** 짓는다★ — `target_label` 에만 의존해 임계구역 안에 둘 이유가 없는데,
    //   두면 `LabelSource` 가 「락 안에서 불리는 포트」가 되어 상태를 만지는 다음 구현이 이 락과
    //   `Destroyed → cleanup_popup_window → state.0.lock()` 사이에서 교착한다. 기존 창 타깃은 label 형식과
    //   무관한 고정 이름이라 발급기를 아예 안 부른다.
    let tab_name = if is_new_window {
        labels.tab_name(&target_label)
    } else {
        "Tab".to_string()
    };

    // ── phase A(락): 소스 콘텐츠 → 임시 View(아직 어느 창 tabs 에도 안 넣음 — orphan 방지) ──────────
    // ★SlotContent 를 락 밖으로 반출★(MOVE 원자성): 창 build 로 락이 풀린 사이 원본 슬롯이 다른 콘텐츠로
    //   재배정될 수 있다 — 2차 락에서 close 전에 이 값과 재조회 결과를 대조해 "옮긴 그 콘텐츠 그대로일 때만"
    //   원본을 닫는다(엉뚱한 콘텐츠 삭제 방지).
    //
    // ★owner-less tmp_view 가 phase B(언락) 동안 views 에 있어도 안전한 이유★: 이 View 는 `views` 에는 있으나
    //   `view_owner`/`windows[*].tabs` 어디에도 없다(「모든 View 는 owner 1개」를 phase B 동안 일시 위배).
    //   그럼에도 안전한 근거 넷: ① 이 view id 는 이 op 만 손에 쥔다 ② 어느 창 tabs 에도 없어 라우팅 순회
    //   (창→tabs walk)에 안 걸린다 ③ 소스 콘텐츠는 소스 슬롯이 아직 살아 있어 계속 보인다 ④ 종점은 항상
    //   phase C attach(owner 부여) 또는 롤백(`drop_detached_view`) 둘 중 하나다.
    //   ⚠️ 「views 전체를 순회하며 owner 를 요구/가정」하는 코드를 나중에 넣을 때는 이 일시 owner-less View 를
    //   전제로 깔아야 한다(무조건 owner 있음 가정 = 이 op 중 패닉).
    let detached = state
        .0
        .lock()
        .map_err(|e| e.to_string())
        .and_then(|mut mgr| {
            let detached = mgr
                .prepare_detached_view(view_id, slot_id, tab_name)
                .map_err(|e| e.to_string())?;
            subs.resync(&mgr);
            Ok(detached)
        }); // ← 락 드롭
    let (tmp_view, src_content) = match detached {
        Ok(detached) => detached,
        Err(e) => {
            // 아직 연 창이 없다 — 맡긴 예열 창만 되물린다.
            if let Some(claim) = &claim {
                host.release_spare(claim);
            }
            return Err(e);
        }
    };

    // ── phase B(락 밖): 새 창 타깃이면 웹뷰 빌드 / 기존 창 타깃이면 존재 확인만 ─────────────────
    if is_new_window {
        if let Err(e) = host.open(&target_label) {
            rollback_detached(state, subs, tmp_view);
            return Err(e);
        }
    } else if !host.is_open(&target_label) {
        rollback_detached(state, subs, tmp_view);
        return Err(format!("대상 창 없음: {target_label}"));
    }

    // ── phase C(락): 임시 View 를 타깃 창 탭으로 삽입(★기존창 재검증★) + 소스 슬롯 close ───────────
    let (src_tabs, tgt_tabs, src_layout) = {
        // 락이 오염됐으면 임시 View 는 거두지 못하지만(`rollback_detached` 와 같다) 연 창은 거둔다 — `Err` 면 떠 있는
        //   새 창이 없다(창 포트 계약).
        let mut mgr = match state.0.lock().map_err(|e| e.to_string()) {
            Ok(mgr) => mgr,
            Err(e) => {
                tracing::warn!(
                    window = %target_label,
                    view = %tmp_view,
                    new_window = is_new_window,
                    "슬롯 MOVE: lock poisoned — 임시 View 롤백 스킵 · 새 창이면 거둔다"
                );
                if is_new_window {
                    withdraw_new_window(host, &target_label, claim.as_ref());
                }
                return Err(e);
            }
        };

        let inserted = if is_new_window {
            mgr.attach_view_as_new_window(&target_label, tmp_view)
        } else {
            mgr.insert_tab_into(&target_label, tmp_view)
        };
        if let Err(e) = inserted {
            // ★롤백 갈래는 둘이다★: 기존 창 타깃은 phase B(언락) 동안 대상 창이 모델에서 사라지면
            //   `insert_tab_into` 가 실패한다. 새 창 타깃은 label 이 단조 발급이라 겹치지 않지만, phase B 동안 그 새
            //   창이 닫혀 묘비가 섰으면(`ViewManager::mark_closed`) `attach_view_as_new_window` 가 `WindowClosed` 로
            //   실패한다 — 그때 방금 연 창을 거둔다(이미 닫혔으면 무동작 · 맡긴 예열 창이면 되물린다).
            mgr.drop_detached_view(tmp_view);
            subs.resync(&mgr);
            drop(mgr);
            if is_new_window {
                withdraw_new_window(host, &target_label, claim.as_ref());
            }
            return Err(format!("탭 삽입 실패(롤백): {e}"));
        }

        // ★MOVE→COPY 열화는 의도된 best-effort★: phase B(언락) 동안 소스 슬롯이 다른 콘텐츠로 재배정되면
        //   still_ours=false → close 스킵. 즉 "재배정된 엉뚱한 콘텐츠를 지우지 않는 것"이 최우선이고, 그 대가로
        //   원래 콘텐츠가 타깃 탭 + 소스 슬롯 양쪽에 남는다. 이 중복은 같은 콘텐츠 두 View 를 허용하는 모델
        //   불변식으로 무해하므로(진도 독립 — ADR-0046) 엄격 롤백 대신 이대로 둔다.
        //   옮긴 것이 빈 칸이어도 같은 대조다 — 그 사이 채워졌으면 불일치라 채운 콘텐츠를 지우지 않는다(ADR-0228).
        // ★load-bearing★: 소스 View 자체가 gap 중 소멸(탭/창 닫힘)했으면 `slot_content` 가 `Err` 를 준다 →
        //   아래 대조가 실패 → close 스킵. 이 `Err→스킵` 이 이미-사라진 소스를 다시 close 하려다 나는
        //   오작동/패닉을 막는다(수정 금지).
        let still_ours = matches!(
            mgr.slot_content(view_id, slot_id),
            Ok(ref c) if *c == src_content
        );
        if still_ours {
            let _ = mgr.close_slot(view_id, slot_id);
        } else {
            tracing::warn!(
                view = %view_id, slot = %slot_id,
                "원본 슬롯이 창 생성 중 재배정/제거됨 — MOVE 의 close 스킵(대상 탭은 그대로 유지)"
            );
        }

        let src_tabs = owner_tabs(&mgr, view_id);
        let tgt_tabs = tabs_payload(&mgr, &target_label);
        let src_layout = mgr.snapshot(view_id).ok();
        subs.resync(&mgr);
        (src_tabs, tgt_tabs, src_layout)
    }; // ← 락 드롭

    announce(
        events,
        src_layout.as_ref(),
        [src_tabs.as_ref(), tgt_tabs.as_ref()].into_iter().flatten(),
    );
    // 확정 · 첫 자리 기록은 알림 뒤 — 그 기록이 내는 창 사실(`window:frame`)이 새 창의 탭 알림 뒤에 난다(도킹 새 창과
    //   같은 차례 · TRD S21-slot-docking §3-9 「자리별 차례」).
    // ADR-0299
    if is_new_window {
        settle_new_window(host, &target_label, claim.as_ref());
    }

    tracing::info!(window = %target_label, view = %tmp_view, "슬롯 MOVE 완료(detach)");
    Ok(SlotMove {
        window: target_label,
        tab: tmp_view,
    })
}

// phase A 임시 View 롤백(창 삽입 전이라 tabs 갱신 불필요). 소스 슬롯은 유지 — 사용자가 슬롯을 잃지 않는다.
// 락을 못 잡으면(poison) 롤백을 포기한다: 여기서 터지면 이미 실패한 op 가 프로세스를 데려간다.
fn rollback_detached(state: &LayoutState, subs: &dyn SubscriptionSync, tmp_view: Uuid) {
    let Ok(mut mgr) = state.0.lock() else {
        tracing::warn!("rollback_detached: lock poisoned — 롤백 스킵");
        return;
    };
    mgr.drop_detached_view(tmp_view);
    subs.resync(&mgr);
}

// ── 도킹 ─────────────────────────────────────────────────────────────────────

/// 탭 하나를 옮기는 요청 — [`dock_tab`] 이 받는다. `focus` · `mv` 의 뜻은 [`ViewManager::dock`].
#[derive(Debug, Clone, PartialEq)]
pub struct DockRequest {
    pub view: ViewId,
    pub to: DockDest,
    pub focus: FocusPolicy,
    pub mv: DockMove,
}

/// 옮길 곳. `Seam` · `TabBar` 는 [`DockTo`] 의 같은 이름과 같다.
#[derive(Debug, Clone, PartialEq)]
pub enum DockDest {
    Seam {
        target: ViewId,
        anchor: DockAnchor,
        side: DockSide,
        share: f64,
    },
    TabBar {
        window: WindowLabel,
        index: Option<usize>,
    },
    /// 그 탭만 든 창(PRD R22) — label 은 적용 서비스가 고른다. 출처가 탭 하나뿐인 팝아웃이면 새 창 없이 그 창을
    /// `at` 으로 옮긴다(재사용). `at.size` 가 없으면 새 창 = 출처 팝아웃의 보통 크기(메인이면 팝아웃 기본
    /// 크기) · 재사용 = 그 창 크기 그대로.
    NewWindow { at: WindowPlace },
}

/// [`dock_tab`] 의 결말.
#[derive(Debug, Clone, PartialEq)]
pub struct DockOutcome {
    /// 옮긴 것이 든 창.
    pub window: WindowLabel,
    /// 옮긴 탭의 슬롯 id(트리 전위 순) — 옮겨도 id 는 그대로다.
    pub slot_ids: Vec<Uuid>,
    /// 비어 모델에서 지운 출처 팝아웃.
    pub emptied: Option<WindowLabel>,
    /// 비어서 OS 창을 닫은 출처 팝아웃. 늘 `emptied` 와 같다 — 비어 지운 팝아웃은 언제나 닫고, 창을 옮겨 쓴 길은 둘 다 `None` 이다.
    pub closed_window: Option<WindowLabel>,
    /// main 의 마지막 탭이 떠나 main 에 새로 세운 빈 탭.
    pub main_refilled: Option<ViewId>,
    /// 새 창 없이 출처 창을 옮겼다 — 모델은 그대로다(`version` 불변 · 알림 없음).
    pub reused: bool,
    /// 새 창을 창 포트가 맡긴 예열 창으로 열었다([`WindowHost::claim_spare`]). `reused` 면 늘 거짓이다(새 창이 없다).
    // ADR-0299
    pub prewarmed: bool,
}

/// 탭 `req.view` 를 `req.to` 로 옮긴다. `Err` 면 모델이 그대로이고 알림도 묶음 끝도 없다 — 그사이 연 새 창은
/// 거둔다. 판정 · 적용 규칙의 정본은 [`ViewManager::dock`] · [`ViewManager::plan_new_window`].
///
/// - `Seam` · `TabBar` — 락 한 번에 바꾸고, 락 밖에서 알린 뒤 비어 지운 출처 팝아웃의 창을 닫는다.
/// - `NewWindow`(새 창) — 창을 락 밖에서 먼저 열고, 같은 판정을 다시 해 붙인다.
/// - `NewWindow`(재사용) — 출처 창을 락 밖에서 옮긴다(모델 변경 없음). 다시 판정하지 않고 되돌리지도 않는다.
///
/// 부르는 스레드: ★`NewWindow` 를 메인 스레드 · 동기 명령 안에서 부르지 말 것★ — 새 창 빌드가 교착한다
/// (`commands::popout::move_slot_to_window` 가 async 인 까닭과 같다 · TRD S21-slot-docking §1-3 「명령의 실행
/// 스레드」). `Seam` · `TabBar` 는 창을 열지 않아 어느 스레드에서나 부를 수 있다 — 비운 출처 창은
/// 닫기만 한다.
// TRD S21-slot-docking §1-2 「적용 서비스」 · §7 B9
pub fn dock_tab(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    labels: &dyn LabelSource,
    req: DockRequest,
) -> Result<DockOutcome, DockError> {
    let DockRequest {
        view,
        to,
        focus,
        mv,
    } = req;
    let ask = DockAsk { view, focus, mv };
    let (mode, result) = match to {
        DockDest::Seam {
            target,
            anchor,
            side,
            share,
        } => (
            "seam",
            dock_in_place(
                state,
                subs,
                events,
                host,
                &ask,
                DockTo::Seam {
                    target,
                    anchor,
                    side,
                    share,
                },
            ),
        ),
        DockDest::TabBar { window, index } => (
            "tab_bar",
            dock_in_place(
                state,
                subs,
                events,
                host,
                &ask,
                DockTo::TabBar { window, index },
            ),
        ),
        DockDest::NewWindow { at } => (
            "new_window",
            dock_into_window(state, subs, events, host, labels, &ask, at),
        ),
    };
    match &result {
        Ok((out, notes)) => tracing::info!(
            view = %view,
            mode = if out.reused { "reuse" } else { mode },
            window = %out.window,
            closed_window = out.closed_window.as_deref(),
            emptied = out.emptied.as_deref(),
            reused = out.reused,
            main_refilled = out.main_refilled.is_some(),
            changed = notes.unchanged.then_some(false),
            "도킹 적용"
        ),
        Err(e) => tracing::debug!(view = %view, mode, reason = %e, "도킹 거절 — 모델 그대로"),
    }
    result.map(|(out, _)| out)
}

struct DockAsk {
    view: ViewId,
    focus: FocusPolicy,
    mv: DockMove,
}

// 끝 로그 몫 — 결말(`DockOutcome`)에 싣지 않는 것.
#[derive(Default)]
struct DockNotes {
    // 같은 창 같은 자리의 탭바 놓기 — 모델이 그대로다.
    unchanged: bool,
}

// `ViewManager::dock` 바로 앞의 모습 중 알릴 것을 고르는 데 쓰는 것 — 같은 락 안에서 뜬다.
struct BeforeDock {
    version: u64,
    source_active: Option<ViewId>,
}

impl BeforeDock {
    fn take(mgr: &ViewManager, view: ViewId) -> Self {
        BeforeDock {
            version: mgr.version,
            source_active: mgr
                .owner_of(view)
                .and_then(|label| mgr.windows.get(label))
                .map(|w| w.active),
        }
    }
}

// 성공한 `dock` 이 낼 알림 — 락 안에서 복사한다. 빈 값 = 무변경(같은 창 같은 자리의 탭바 놓기).
#[derive(Default)]
struct DockNews {
    layouts: Vec<ViewSnapshot>,
    tabs: Vec<WindowTabsPayload>,
}

impl DockNews {
    // `shown` = 대상에서 보일 탭 — `Seam` 은 대상 탭, 그 밖은 옮긴 탭.
    //
    // ★남은 출처 창의 활성이 바뀌었으면 그 탭의 배치도 싣는다★ — 프론트는 그 탭 id 를 탭 목록으로만 받으면 배치가
    // 없다(`close_tab` 이 남은 창의 활성 배치를 싣는 것과 같다). main 의 마지막 탭이 떠나 세운 빈 탭(PRD R14)도
    // 이 갈래로 실린다.
    // ★비어 지운 출처에는 탭 목록을 싣지 않는다★ — 탭 0 개 알림을 받은 웹뷰는 제 창을 닫는다.
    fn collect(mgr: &ViewManager, before: &BeforeDock, docked: &Docked, shown: ViewId) -> Self {
        if mgr.version == before.version {
            return DockNews::default();
        }
        let mut views = vec![shown];
        let mut windows = vec![docked.window.as_str()];
        if docked.source_window != docked.window && docked.emptied.is_none() {
            windows.push(&docked.source_window);
            let active = mgr.windows.get(&docked.source_window).map(|w| w.active);
            if let Some(active) = active.filter(|a| Some(*a) != before.source_active && *a != shown)
            {
                views.push(active);
            }
        }
        DockNews {
            layouts: views.iter().filter_map(|v| mgr.snapshot(*v).ok()).collect(),
            tabs: windows
                .into_iter()
                .filter_map(|label| tabs_payload(mgr, label))
                .collect(),
        }
    }
}

// 모델을 바꾼 도킹의 락 밖 몫 — 알림 → 비어 지운 출처 팝아웃의 OS 창 닫기(모델이 먼저 지웠다 —
// `close_tab` 과 같다). 결말을 돌려준다.
// ADR-0057
fn finish_docked(
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    docked: Docked,
    news: &DockNews,
) -> DockOutcome {
    announce(events, &news.layouts, &news.tabs);
    let closed_window = docked.emptied.as_ref().map(|emptied| {
        host.close(emptied);
        emptied.clone()
    });
    DockOutcome {
        window: docked.window,
        slot_ids: docked.slot_ids,
        emptied: docked.emptied,
        closed_window,
        main_refilled: docked.main_refilled,
        reused: false,
        prewarmed: false,
    }
}

// `Seam` · `TabBar` — 판정 · 변경 · 스냅숏 · 재동기가 락 한 번이다.
fn dock_in_place(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    ask: &DockAsk,
    to: DockTo,
) -> Result<(DockOutcome, DockNotes), DockError> {
    let shown = match &to {
        DockTo::Seam { target, .. } => *target,
        _ => ask.view,
    };
    let (docked, news, unchanged) = {
        let mut mgr = sync::lock(&state.0);
        let before = BeforeDock::take(&mgr, ask.view);
        let docked = mgr.dock(ask.view, to, ask.focus, ask.mv.clone())?;
        let news = DockNews::collect(&mgr, &before, &docked, shown);
        let unchanged = mgr.version == before.version;
        if !unchanged {
            subs.resync(&mgr);
        }
        (docked, news, unchanged)
    }; // ← 락 드롭
    let out = finish_docked(events, host, docked, &news);
    Ok((out, DockNotes { unchanged }))
}

// `NewWindow` — phase A(락): 새 창이 필요한가.
fn dock_into_window(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    labels: &dyn LabelSource,
    ask: &DockAsk,
    at: WindowPlace,
) -> Result<(DockOutcome, DockNotes), DockError> {
    let plan = sync::lock(&state.0).plan_new_window(ask.view)?; // ← 락 드롭
    match plan {
        NewWindowPlan::Reuse { label } => reuse_source_window(state, host, ask, label, &at),
        NewWindowPlan::Create { size } => {
            let place = WindowPlace {
                pos: at.pos,
                size: at.size.or(size),
            };
            let out = open_new_window(state, subs, events, host, labels, ask, &place)?;
            Ok((out, DockNotes::default()))
        }
    }
}

// 새 창 — phase B(락 밖) 창 열기 → phase C(락) 같은 판정을 다시 해 붙이기. phase C 가 거절하면(그 사이 창이 닫혀
//   묘비가 섰거나 출처 탭이 사라짐) 연 창을 거둔다 — 출처 창은 건드린 적이 없다.
// ★예열 창은 `focus` 와 상관없이 묻는다★ — `Keep` 이어도 맡긴 창을 쓰고, 활성 여부는 `open_at` 이 그 맡김에 적는다
//   (TRD S21-slot-docking §3-9 I4 · 사용자 결정 2026-10-21).
// ADR-0299
fn open_new_window(
    state: &LayoutState,
    subs: &dyn SubscriptionSync,
    events: &dyn LayoutEvents,
    host: &dyn WindowHost,
    labels: &dyn LabelSource,
    ask: &DockAsk,
    place: &WindowPlace,
) -> Result<DockOutcome, DockError> {
    let (label, claim) = new_window_label(host, labels);
    if let Err(e) = host.open_at(&label, place, ask.focus == FocusPolicy::Reveal) {
        tracing::warn!(label = %label, "도킹 새 창을 열지 못함 — 모델 그대로: {e}");
        return Err(DockError::WindowOpen(e));
    }

    let committed = {
        let mut mgr = sync::lock(&state.0);
        let before = BeforeDock::take(&mgr, ask.view);
        mgr.dock(
            ask.view,
            DockTo::NewWindow {
                label: label.clone(),
            },
            ask.focus,
            ask.mv.clone(),
        )
        .map(|docked| {
            let news = DockNews::collect(&mgr, &before, &docked, ask.view);
            subs.resync(&mgr);
            (docked, news)
        })
    }; // ← 락 드롭
    let (docked, news) = match committed {
        Ok(done) => done,
        Err(e) => {
            withdraw_new_window(host, &label, claim.as_ref());
            tracing::warn!(label = %label, reason = %e, "도킹 새 창 — 붙이기 전 판정이 바뀌어 연 창을 거둔다");
            return Err(e);
        }
    };
    let out = DockOutcome {
        prewarmed: claim.is_some(),
        ..finish_docked(events, host, docked, &news)
    };
    settle_new_window(host, &label, claim.as_ref());
    Ok(out)
}

// 재사용 — B′(락 밖) 출처 창 옮기기 → 락 안에서 결말에 실을 것만 읽는다(모델 변경 0).
//   ★다시 판정하지 않고 되돌리지도 않는다(사용자 결정 2026-10-16)★ — 끄기가 서면 놓은 자리에 둔다. 옮기는 사이 다른 경로가
//   출처를 바꿨으면(탭 · 슬롯 더함 · 탭 닫힘) 모델은 그 쓰기가 남긴 그대로다 — 탭이 늘었으면 그 창째 옮겨진 것이고, 탭이
//   닫혔으면 창도 그 닫기가 거둔다.
// TRD S21-slot-docking §1-2 「적용 서비스」 재사용
fn reuse_source_window(
    state: &LayoutState,
    host: &dyn WindowHost,
    ask: &DockAsk,
    label: WindowLabel,
    at: &WindowPlace,
) -> Result<(DockOutcome, DockNotes), DockError> {
    if let Err(failed) = host.move_window(&label, at) {
        tracing::warn!(
            label = %label,
            step = ?failed.step,
            "재사용 도킹 — 출처 창을 못 옮겼다(되돌리지 않는다): {}",
            failed.error
        );
        return Err(DockError::WindowOpen(format!(
            "창 옮기기 {:?} 실패 — {}",
            failed.step, failed.error
        )));
    }

    let (slot_ids, still_there) = {
        let mgr = sync::lock(&state.0);
        let slot_ids = mgr
            .views
            .get(&ask.view)
            .map(|v| tree::slot_ids(&v.layout))
            .unwrap_or_default();
        (slot_ids, mgr.windows.contains_key(&label))
    }; // ← 락 드롭

    // 그 사이 닫혀 모델에 없는 창은 적을 항목이 없다.
    if still_there {
        host.record_placement(&label);
    }
    Ok((
        DockOutcome {
            window: label,
            slot_ids,
            emptied: None,
            closed_window: None,
            main_refilled: None,
            reused: true,
            prewarmed: false,
        },
        DockNotes::default(),
    ))
}

// ── 측정 보고 2종 ────────────────────────────────────────────────────────────
//
// `window` 는 보고한 웹뷰의 창 label 이다 — 껍데기가 Tauri 가 넣어 준 창에서 꺼내 넘긴다(인자로 받지 않음).
// 0 크기 무시·범위 거절과 직전 값 유지는 관리자(`ViewManager::set_window_canvas`·`set_ui_metrics`)가
// 진다.
// ADR-0227

pub fn report_window_canvas(
    state: &LayoutState,
    window: &str,
    w: u32,
    h: u32,
) -> Result<(), String> {
    let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
    mgr.set_window_canvas(window, w, h)
        .map_err(|e| e.to_string())
}

pub fn report_ui_metrics(
    state: &LayoutState,
    window: &str,
    metrics: UiMetrics,
) -> Result<(), String> {
    let mut mgr = state.0.lock().map_err(|e| e.to_string())?;
    mgr.set_ui_metrics(window, metrics)
        .map_err(|e| e.to_string())
}

// ── read-only 5종 ────────────────────────────────────────────────────────────

// LLM 이 split id 를 찾는 길 — 스냅샷(`get_view`)은 재귀 타입이라 명령 버스에 못 싣는다.
// ADR-0227
pub fn list_splits(state: &LayoutState, view_id: Uuid) -> Result<Vec<SplitInfo>, String> {
    let mgr = state.0.lock().map_err(|e| e.to_string())?;
    mgr.list_splits(view_id).map_err(|e| e.to_string())
}

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
