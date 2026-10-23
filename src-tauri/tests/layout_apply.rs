//! 레이아웃 적용 서비스(`layout::apply`) 통합 테스트 — 창·데몬·Tauri 0 (ADR-0012 seam 격리).
//!
//! ★이 파일이 `tests/`(통합 타깃)에 있는 이유★: 재는 대상이 **실 소켓·실 `AppHandle` 없이는 못 세우는
//! 배선**이라, 그 자리를 포트로 끊어 세우는 통합 하네스가 제자리다(그래서 하네스 자신은 창·데몬·Tauri 를
//! 하나도 안 쓴다). 새 단언의 배치 기준도 그것이다 — **순수 단위는 모듈 옆 `#[cfg(test)]`**(그쪽은
//! `--test lib_unit` 으로 돈다 · 그 타깃을 세운 결정 = ADR-0174 · 현황 = CLAUDE.md 「빌드·검증 명령」),
//! **배선은 여기.**
//! 실행: `cargo test -p engram-dashboard --test layout_apply` — ★`-- --test-threads=4` 를 붙이지 않는다★
//! (이 스위트는 자식 프로세스를 하나도 안 띄운다. 그 플래그의 근거·판정 규칙 정본 = CLAUDE.md 「빌드·검증 명령」)
//!
//! CI 는 이 타깃만 따로 부르는 전용 스텝도 갖는다(`.github/workflows/ci.yml`).
//!
//! 창 label 발급은 **실 `PopupCounter`**(프로브 껍데기만 덧씌운다), 구독 재동기는 **실 `OutputRouter`** 를
//! 쓴다 — 가짜로 흉내내면 label 단조성·라우팅 계약을 검증하는 게 아니라 가짜를 검증한다. 나머지 세
//! 포트(알림·OS 창·스폰)는 기록용 가짜다.
//!
//! ## ★락 위치 프로브(이 하네스의 핵심)★
//! 이 리팩터가 지키려는 불변식은 "어느 포트가 락 **안**에서 불리고 어느 포트가 **밖**에서 불리나"인데,
//! 호출을 세는 것만으로는 그게 안 잡힌다 — 위치를 옮겨도 개수가 같다. 그래서 각 가짜 포트가 호출된
//! **그 자리에서** `try_lock` 으로 락 보유 여부를 직접 본다:
//! - `LayoutEvents`·`WindowHost`·`LabelSource`·`AgentSpawner` → `WouldBlock` 이 아님(락 **밖**이어야 한다 — 오염된
//!   락도 밖이다).
//!   `WindowHost::close` 를 락 안으로 옮기면 운영에서 확정 교착이다(`w.destroy()` → `Destroyed` → 같은 락
//!   재취득).
//! - `SubscriptionSync::resync` → `is_err()`(락 **안**이어야 한다). 밖으로 내면 F1/F2 stale-unsubscribe
//!   결함이 부활한다.
//!
//! 결정론 근거: std `Mutex::try_lock` 은 **같은 스레드가 이미 쥐고 있어도 블록하지 않고 `WouldBlock`** 을
//! 준다 → 스레드·타이밍 0으로 갈린다. ★단 이 판정은 테스트가 단일 스레드로 도는 것을 전제한다★(다른
//! 스레드가 락을 쥐고 있으면 `is_err()` 가 거짓 통과할 수 있다 — 이 파일은 스레드를 만들지 않는다).
//! 실 Tauri 어댑터는 창이 필요해 여기서 못 돌린다(그건 GUI 실측 몫).

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, TryLockError};

use engram_dashboard_protocol::AgentBackendKind;
use uuid::Uuid;

use engram_dashboard_lib::commands::popout::PopupCounter;
use engram_dashboard_lib::layout::apply::{
    self, AgentSpawner, DockDest, DockOutcome, DockRequest, LabelSource, LayoutEvents, MoveFailed,
    MoveStep, SpareClaim, SubscriptionSync, WindowHost, WindowPlace, WindowTabsPayload,
};
use engram_dashboard_lib::layout::bands::PointCss;
use engram_dashboard_lib::layout::dock::{DockAnchor, DockSide, DOCK_NEW_SHARE};
use engram_dashboard_lib::layout::geometry::{Insets, PxRect};
use engram_dashboard_lib::layout::manager::{DockError, DockMove, FocusPolicy};
use engram_dashboard_lib::layout::{
    tree, LayoutNode, LayoutState, SlotContent, SplitDir, SplitRatioOutcome, UiMetrics,
    ViewManager, ViewSnapshot, WindowBounds, WindowPlacement, MAIN_WINDOW_LABEL,
};
use engram_dashboard_lib::output_router::OutputRouter;
use engram_dashboard_lib::tab_drop::plan::{
    self as drop_plan, InputNow, PhysPoint, Plan, SourceFrame, TopAt,
};
use engram_dashboard_lib::tab_drop::wire::{DropAnchor, DropTo, NewWindowBy, TabDrop};

// ── 락 위치 프로브 ───────────────────────────────────────────────────────────

// 같은 `Arc<Mutex<ViewManager>>` 를 공유해 호출 시점의 락 보유 여부를 본다(헤더 §락 위치 프로브).
struct Probe(LayoutState);

impl Probe {
    fn new(state: &LayoutState) -> Self {
        Probe(state.clone())
    }

    // 오염된 락은 아무도 쥐지 않은 것이다 — 락 오염 시험이 오염 뒤의 포트 호출도 이 프로브로 본다.
    fn assert_outside(&self, who: &str) {
        assert!(
            !matches!(self.0 .0.try_lock(), Err(TryLockError::WouldBlock)),
            "{who} 는 락을 드롭한 뒤에 불려야 한다(락 안이면 창 destroy·emit 이 교착·재진입한다)"
        );
    }

    fn assert_inside(&self, who: &str) {
        assert!(
            self.0 .0.try_lock().is_err(),
            "{who} 는 락 보유 중에 불려야 한다(밖이면 계산~발화 사이 재추가로 stale 해제 — F1/F2)"
        );
    }
}

// ── 가짜 포트 ────────────────────────────────────────────────────────────────

/// 창 포트와 알림 포트를 가로지른 차례 — 둘이 한 기록을 나눠 쓴다(TRD S21-slot-docking §8-1 ⑬ 자리별 차례). 창 포트 =
/// `"open:<label>"` · `"open_at:<label>"` · `"close:<label>"` · `"place:<label>"` · 예열 창 셋 `"claim:<label>"`(맡겼을
/// 때만) · `"committed:<label>"` · `"release:<label>"`, 알림 = `"layout"` · `"tabs:<label>"`.
#[derive(Clone, Default)]
struct Journal(Arc<Mutex<Vec<String>>>);

impl Journal {
    fn note(&self, entry: String) {
        self.0.lock().unwrap().push(entry);
    }

    /// 지금까지의 기록을 비우며 돌려준다.
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

struct Recorder {
    probe: Probe,
    layout: Mutex<Vec<ViewSnapshot>>,
    tabs: Mutex<Vec<WindowTabsPayload>>,
    /// 알림이 온 차례 — `"layout"` · `"tabs"`.
    order: Mutex<Vec<&'static str>>,
    journal: Journal,
    /// 첫 탭 알림에서 락 밖으로 한 번 — 새 창 항목이 모델에 든 뒤 · 확정 전에 다른 경로가 모델을 바꾼 상태를 세운다.
    on_tabs: Mutex<Option<DuringOpen>>,
}

impl Recorder {
    fn new(state: &LayoutState, journal: Journal) -> Self {
        Recorder {
            probe: Probe::new(state),
            layout: Mutex::new(Vec::new()),
            tabs: Mutex::new(Vec::new()),
            order: Mutex::new(Vec::new()),
            journal,
            on_tabs: Mutex::new(None),
        }
    }

    /// 지금까지 온 차례를 비우며 돌려준다.
    fn take_order(&self) -> Vec<&'static str> {
        std::mem::take(&mut *self.order.lock().unwrap())
    }
}

impl LayoutEvents for Recorder {
    fn layout_updated(&self, snapshot: &ViewSnapshot) {
        self.probe.assert_outside("LayoutEvents::layout_updated");
        self.layout.lock().unwrap().push(snapshot.clone());
        self.order.lock().unwrap().push("layout");
        self.journal.note("layout".to_string());
    }

    fn window_tabs_updated(&self, tabs: &WindowTabsPayload) {
        self.probe
            .assert_outside("LayoutEvents::window_tabs_updated");
        self.tabs.lock().unwrap().push(tabs.clone());
        self.order.lock().unwrap().push("tabs");
        self.journal.note(format!("tabs:{}", tabs.label));
        if let Some(hook) = self.on_tabs.lock().unwrap().take() {
            hook(&self.probe.0);
        }
    }
}

struct Subs {
    probe: Probe,
    router: OutputRouter,
    resyncs: AtomicUsize,
    unsubscribed: Mutex<Vec<Uuid>>,
}

impl Subs {
    fn new(state: &LayoutState) -> Self {
        Subs {
            probe: Probe::new(state),
            router: OutputRouter::new(),
            resyncs: AtomicUsize::new(0),
            unsubscribed: Mutex::new(Vec::new()),
        }
    }
}

impl SubscriptionSync for Subs {
    fn resync(&self, mgr: &ViewManager) {
        self.probe.assert_inside("SubscriptionSync::resync");
        self.resyncs.fetch_add(1, Ordering::Relaxed);
        let delta = self.router.rebuild(mgr);
        self.unsubscribed
            .lock()
            .unwrap()
            .extend(delta.to_unsubscribe);
    }
}

/// `open` 이 락 밖에서 한 번 부르는 끼어들기 — 창 빌드(phase B) 사이 다른 명령이 모델을 바꾼 상태를 세운다.
/// `move_window` 의 끼어들기(`during_move`)도 같은 꼴이다.
type DuringOpen = Box<dyn FnOnce(&LayoutState) + Send>;

struct Host {
    probe: Probe,
    /// `open` · `open_at` 둘 다 실패한다.
    fail_open: bool,
    /// 연 창 label — `open` · `open_at` 둘 다.
    opened: Mutex<Vec<String>>,
    closed: Mutex<Vec<String>>,
    placed: Mutex<Vec<String>>,
    during_open: Mutex<Option<DuringOpen>>,
    /// `open_at` 이 받은 `(label, 자리, 활성)`.
    opened_at: Mutex<Vec<(String, WindowPlace, bool)>>,
    /// `move_window` 이 받은 `(label, 자리)`.
    moves: Mutex<Vec<(String, WindowPlace)>>,
    /// `move_window` 가 실제로 부른 세터 걸음(순서대로) — 실패는 `fail_steps` 로 심는다.
    steps: Mutex<Vec<MoveStep>>,
    fail_steps: Mutex<Vec<MoveStep>>,
    /// `move_window` 가 세터를 부르기 전 락 밖에서 한 번 — 옮기는 사이 다른 경로가 출처를 바꾼 상태를 세운다.
    during_move: Mutex<Option<DuringOpen>>,
    journal: Journal,
    /// 맡길 숨은 예열 창의 label — 맡기면 비고, 되물리면 돌아온다(TRD S21-slot-docking §8-1 ⑬). 풀의 상태 기계(짓기 ·
    /// 기다림 · 다시 짓기)는 흉내 내지 않는다 — 부른 차례와 맡김표만 잰다.
    spare: Mutex<Option<String>>,
    /// 지금 맡겨 둔 맡김표 — 확정 · 되물림이 이것과 같은 맡김표를 한 번 돌려줘야 한다.
    claimed: Mutex<Option<SpareClaim>>,
    /// `claim_spare` 가 불린 수 — 맡길 창이 없어 `None` 을 낸 것도 센다.
    claims_asked: AtomicUsize,
    generation: AtomicU32,
    /// 모델에 든 뒤 · 확정 전에 시험이 모델에서 지운 label — `record_placement` 가 그것에는 모델 항목을 요구하지 않는다.
    vanished: Mutex<Vec<String>>,
}

impl Host {
    fn new(state: &LayoutState, fail_open: bool, journal: Journal) -> Self {
        Host {
            probe: Probe::new(state),
            journal,
            fail_open,
            opened: Mutex::new(Vec::new()),
            closed: Mutex::new(Vec::new()),
            placed: Mutex::new(Vec::new()),
            during_open: Mutex::new(None),
            opened_at: Mutex::new(Vec::new()),
            moves: Mutex::new(Vec::new()),
            steps: Mutex::new(Vec::new()),
            fail_steps: Mutex::new(Vec::new()),
            during_move: Mutex::new(None),
            spare: Mutex::new(None),
            claimed: Mutex::new(None),
            claims_asked: AtomicUsize::new(0),
            generation: AtomicU32::new(0),
            vanished: Mutex::new(Vec::new()),
        }
    }

    // 맡긴 창이 그 맡김표로 돌아왔다 — 같은 맡김표를 두 번 받거나 다른 맡김표를 받으면 터진다.
    fn take_claim(&self, claim: &SpareClaim, who: &str) {
        let held = self.claimed.lock().unwrap().take();
        assert_eq!(
            held.as_ref(),
            Some(claim),
            "{who} 는 맡긴 맡김표 그대로 한 번만 받아야 한다"
        );
    }

    // 창 열기 몫 — `open` · `open_at` 이 함께 쓴다.
    fn opening(&self, kind: &str, label: &str) -> Result<(), String> {
        assert!(
            !self.probe.0 .0.lock().unwrap().windows.contains_key(label),
            "{kind}({label}) 은 창이 모델에 들기 전에 불려야 한다"
        );
        self.journal.note(format!("{kind}:{label}"));
        if let Some(hook) = self.during_open.lock().unwrap().take() {
            hook(&self.probe.0);
        }
        self.opened.lock().unwrap().push(label.to_string());
        if self.fail_open {
            // 포트 계약 — 맡긴 예열 창을 못 놓았으면 그 창을 거둔 뒤 `Err` 다. 거둔 맡김은 끝났다.
            let mut claimed = self.claimed.lock().unwrap();
            if claimed.as_ref().is_some_and(|c| c.label == label) {
                *claimed = None;
            }
            return Err("창 생성 실패(테스트)".to_string());
        }
        Ok(())
    }

    fn fail_step(&self, step: MoveStep) {
        self.fail_steps.lock().unwrap().push(step);
    }

    fn steps(&self) -> Vec<MoveStep> {
        self.steps.lock().unwrap().clone()
    }

    fn setter(&self, step: MoveStep) -> Result<(), MoveFailed> {
        self.steps.lock().unwrap().push(step);
        if self.fail_steps.lock().unwrap().contains(&step) {
            return Err(MoveFailed {
                step,
                error: format!("{step:?} 세터 실패(테스트)"),
            });
        }
        Ok(())
    }
}

impl WindowHost for Host {
    // ★모델에 들기 전이어야 한다★ — 새 창을 여는 세 자리가 모두 창을 먼저 열고 모델에 넣는다(TRD S21-slot-docking §3-9
    // 「자리별 차례」 · ADR-0299).
    fn open(&self, label: &str) -> Result<(), String> {
        self.probe.assert_outside("WindowHost::open");
        self.opening("open", label)
    }

    fn open_at(&self, label: &str, at: &WindowPlace, activate: bool) -> Result<(), String> {
        self.probe.assert_outside("WindowHost::open_at");
        self.opened_at
            .lock()
            .unwrap()
            .push((label.to_string(), *at, activate));
        self.opening("open_at", label)
    }

    fn claim_spare(&self) -> Option<SpareClaim> {
        self.probe.assert_outside("WindowHost::claim_spare");
        self.claims_asked.fetch_add(1, Ordering::Relaxed);
        let label = self.spare.lock().unwrap().take()?;
        let claim = SpareClaim {
            label: label.clone(),
            generation: self.generation.fetch_add(1, Ordering::Relaxed) + 1,
        };
        *self.claimed.lock().unwrap() = Some(claim.clone());
        self.journal.note(format!("claim:{label}"));
        Some(claim)
    }

    // 그 label 이 모델에 없어도 받는다 — 확정 전에 빠진 창을 닫는 것은 구현 몫이다(포트 문서).
    fn committed(&self, claim: &SpareClaim) {
        self.probe.assert_outside("WindowHost::committed");
        self.take_claim(claim, "committed");
        self.journal.note(format!("committed:{}", claim.label));
    }

    fn release_spare(&self, claim: &SpareClaim) {
        self.probe.assert_outside("WindowHost::release_spare");
        self.take_claim(claim, "release_spare");
        self.journal.note(format!("release:{}", claim.label));
        *self.spare.lock().unwrap() = Some(claim.label.clone());
    }

    // 계약 = 포트 문서 — 위치 → 크기, 첫 실패에서 멈춘다. 빈 칸은 그 걸음을 건너뛴다.
    fn move_window(&self, label: &str, at: &WindowPlace) -> Result<(), MoveFailed> {
        self.probe.assert_outside("WindowHost::move_window");
        self.moves.lock().unwrap().push((label.to_string(), *at));
        if let Some(hook) = self.during_move.lock().unwrap().take() {
            hook(&self.probe.0);
        }
        if at.pos.is_some() {
            self.setter(MoveStep::Position)?;
        }
        if at.size.is_some() {
            self.setter(MoveStep::Size)?;
        }
        Ok(())
    }

    fn close(&self, label: &str) {
        self.probe.assert_outside("WindowHost::close");
        self.journal.note(format!("close:{label}"));
        self.closed.lock().unwrap().push(label.to_string());
    }

    // ★모델에 든 뒤여야 한다★ — 운영 구현은 그 label 의 모델 항목에 자리를 적으므로, 먼저 불리면 첫 자리가 버려진다.
    fn record_placement(&self, label: &str) {
        self.probe.assert_outside("WindowHost::record_placement");
        assert!(
            self.probe.0 .0.lock().unwrap().windows.contains_key(label)
                || self.vanished.lock().unwrap().iter().any(|l| l == label),
            "record_placement({label}) 는 창이 모델에 든 뒤에 불려야 한다"
        );
        self.journal.note(format!("place:{label}"));
        self.placed.lock().unwrap().push(label.to_string());
    }

    // ★main 을 특례로 참이라 답한다★ — 정적 config 창이라 이 가짜가 연 적이 없는데, 실 앱에서는 항상 떠
    // 있다(팝업 슬롯을 main 으로 되돌리는 것이 실사용 경로다). 나머지는 이 가짜가 연 뒤 안 닫은 것뿐.
    fn is_open(&self, label: &str) -> bool {
        self.probe.assert_outside("WindowHost::is_open");
        label == MAIN_WINDOW_LABEL
            || (self.opened.lock().unwrap().iter().any(|l| l == label)
                && !self.closed.lock().unwrap().iter().any(|l| l == label))
    }
}

// ★실 발급기를 **감싼다**(대체하지 않는다)★: label 단조성은 안쪽 실물이 계속 지고, 이 껍데기는 호출
// **위치**만 덧본다. 오늘 구현은 순수해서 락 안에서 불려도 안 터지므로, 그 자리를 못 박는 것은 이 프로브뿐이다
// — 상태를 만지는 구현으로 바뀌는 날 `Destroyed → cleanup_popup_window → state.0.lock()` 와 교착한다.
struct Labels {
    probe: Probe,
    inner: PopupCounter,
}

impl LabelSource for Labels {
    fn next_label(&self) -> String {
        self.probe.assert_outside("LabelSource::next_label");
        LabelSource::next_label(&self.inner)
    }

    fn tab_name(&self, label: &str) -> String {
        self.probe.assert_outside("LabelSource::tab_name");
        LabelSource::tab_name(&self.inner, label)
    }
}

struct Spawner {
    probe: Probe,
    reply: Result<String, String>,
    calls: AtomicUsize,
    cwds: Mutex<Vec<String>>,
    /// ★고른 백엔드가 포트까지 갔나★ — 스폰 호출 횟수만으로는 그 값이 흘렀는지 안 흘렀는지 못 가른다.
    backends: Mutex<Vec<Option<AgentBackendKind>>>,
}

impl Spawner {
    fn ok(state: &LayoutState, agent: Uuid) -> Self {
        Spawner::new(state, Ok(agent.to_string()))
    }

    fn failing(state: &LayoutState) -> Self {
        Spawner::new(state, Err("spawn 실패: 데몬 거절".to_string()))
    }

    fn new(state: &LayoutState, reply: Result<String, String>) -> Self {
        Spawner {
            probe: Probe::new(state),
            reply,
            calls: AtomicUsize::new(0),
            cwds: Mutex::new(Vec::new()),
            backends: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }
}

impl AgentSpawner for Spawner {
    fn spawn_by_cwd<'a>(
        &'a self,
        cwd: String,
        backend: Option<AgentBackendKind>,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>> {
        Box::pin(async move {
            // 락 안에서 await 하면 명령 future 가 Send 를 잃어 컴파일도 안 되지만, 그 벽이 서 있는지를
            // 여기서도 본다(형제 포트와 같은 프로브 — 벽이 하나면 조용히 무너진다).
            self.probe.assert_outside("AgentSpawner::spawn_by_cwd");
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.cwds.lock().unwrap().push(cwd);
            self.backends.lock().unwrap().push(backend);
            self.reply.clone()
        })
    }
}

// ── 하네스 ───────────────────────────────────────────────────────────────────

struct World {
    state: LayoutState,
    subs: Subs,
    ev: Recorder,
    host: Host,
    // 실물 발급기(프로브만 덧씌움) — prefix(`slot-popup-`)는 capabilities/popup.json glob 과 짝이고
    // 단조성은 닫힌 label 재-build 에러를 막는 계약이다.
    labels: Labels,
    /// `ev` · `host` 가 나눠 쓰는 차례 기록.
    journal: Journal,
}

impl World {
    fn new() -> Self {
        World::build(false)
    }

    fn failing_host() -> Self {
        World::build(true)
    }

    fn build(fail_open: bool) -> Self {
        let state = LayoutState::new();
        let journal = Journal::default();
        World {
            subs: Subs::new(&state),
            ev: Recorder::new(&state, journal.clone()),
            host: Host::new(&state, fail_open, journal.clone()),
            labels: Labels {
                probe: Probe::new(&state),
                inner: PopupCounter::default(),
            },
            journal,
            state,
        }
    }

    fn main_active(&self) -> Uuid {
        apply::list_tabs(&self.state, MAIN_WINDOW_LABEL)
            .expect("main 창은 항상 있다")
            .active
    }

    fn snapshot(&self, view: Uuid) -> ViewSnapshot {
        apply::get_view(&self.state, view).expect("view 존재")
    }

    fn slots(&self, view: Uuid) -> Vec<Uuid> {
        self.snapshot(view)
            .slot_spatial
            .iter()
            .map(|s| s.slot_id)
            .collect()
    }

    fn empty_slot(&self, view: Uuid) -> Uuid {
        tree::first_empty_slot_id(&self.snapshot(view).layout).expect("빈 슬롯 존재")
    }

    fn resyncs(&self) -> usize {
        self.subs.resyncs.load(Ordering::Relaxed)
    }

    fn layout_events(&self) -> usize {
        self.ev.layout.lock().unwrap().len()
    }

    fn tab_events(&self) -> usize {
        self.ev.tabs.lock().unwrap().len()
    }

    fn last_tabs(&self) -> WindowTabsPayload {
        self.ev
            .tabs
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("탭 알림")
    }

    fn unsubscribed(&self) -> Vec<Uuid> {
        self.subs.unsubscribed.lock().unwrap().clone()
    }

    // 팝업 창 하나(빈 탭 1개) — WindowClosed/close_window 경로의 전제.
    fn popup(&self) -> String {
        apply::create_window(&self.state, &self.subs, &self.host, &self.labels).expect("창 생성")
    }

    // 창 포트가 맡길 숨은 예열 창 하나 — 운영처럼 지을 때 발급기에서 label 을 받는다. 반환 = 그 label.
    fn prewarm(&self) -> String {
        let label = LabelSource::next_label(&self.labels.inner);
        *self.host.spare.lock().unwrap() = Some(label.clone());
        label
    }

    // 창 포트가 그 맡김을 아직 쥐고 있나(확정 · 되물림 · 거둠 어느 것도 안 됐다).
    fn claim_outstanding(&self) -> bool {
        self.host.claimed.lock().unwrap().is_some()
    }

    fn claims_asked(&self) -> usize {
        self.host.claims_asked.load(Ordering::Relaxed)
    }

    // 새 창을 열지 않는 길은 예열 창을 묻지도 않는다 — 끝내지 않은 맡김은 하나뿐인 풀 칸을 막는다(TRD S21-slot-docking
    //   §3-9 I27). `asked` = 그 길을 부르기 전의 물음 수.
    fn assert_spare_not_asked(&self, spare: &str, asked: usize) {
        assert_eq!(
            self.claims_asked(),
            asked,
            "새 창이 아닌 길이 예열 창을 물었다"
        );
        assert_eq!(
            self.host.spare.lock().unwrap().as_deref(),
            Some(spare),
            "예열 창은 맡겨지지 않은 채 남는다"
        );
    }
}

// 연 바로 뒤 락을 오염시킨다 — 쥔 채 패닉한 다른 쓰기를 흉내 낸다(같은 스레드 · 되감기로 놓는다).
fn poison(state: &LayoutState) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = state.0.lock().unwrap();
        panic!("락 오염(테스트)");
    }));
    assert!(state.0.is_poisoned());
}

// ── 프로브 자체 검증 ─────────────────────────────────────────────────────────

// ★프로브가 양방향으로 실제로 문다는 증거★. 「밖」 프로브는 서비스 코드를 락 안으로 옮겨 실패시켜 봤지만
// (WindowHost·LayoutEvents 실측), 「안」 프로브는 **컴파일 가능한 반례를 만들 수 없다** — `resync` 가
// `&ViewManager` 를 받아 가드 없이는 부를 방법이 없기 때문이다(타입이 이미 그 절반을 지킨다). 그래서
// 프로브가 죽은 단언이 아니라는 것을 여기서 직접 증명한다: 이 프로브의 남은 일은 포트 모양이 바뀔 때
// (예: 소유 스냅샷을 받게 되면 가드 없이도 부를 수 있다) 그 순간을 잡는 것이다.
#[test]
fn probe_fires_on_both_sides_of_the_lock() {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    let state = LayoutState::new();
    let probe = Probe::new(&state);

    probe.assert_outside("락 미보유");
    assert!(
        catch_unwind(AssertUnwindSafe(|| probe.assert_inside("거짓 주장"))).is_err(),
        "락 미보유인데 「안」이라 주장하면 프로브가 터져야 한다"
    );

    let guard = state.0.lock().unwrap();
    probe.assert_inside("락 보유");
    assert!(
        catch_unwind(AssertUnwindSafe(|| probe.assert_outside("거짓 주장"))).is_err(),
        "락 보유 중인데 「밖」이라 주장하면 프로브가 터져야 한다"
    );
    drop(guard);
}

// 도킹 시험이 이 가짜의 걸음 기록 · 실패 심기에 기대므로, 가짜가 포트 계약(옮기기 = 첫 실패에서 멈춤 · 빈 칸은
// 건너뜀)을 흉내 내는지 먼저 못 박는다.
#[test]
fn the_fake_window_host_keeps_the_port_contract() {
    let w = World::new();
    let label = w.popup();
    let at = WindowPlace {
        pos: Some((10, 20)),
        size: Some((300.0, 200.0)),
    };

    w.host.fail_step(MoveStep::Position);
    let err = w.host.move_window(&label, &at).unwrap_err();
    assert_eq!(err.step, MoveStep::Position);
    assert_eq!(
        w.host.steps(),
        [MoveStep::Position],
        "첫 실패 뒤 크기 걸음은 안 불린다"
    );
    let size_only = WindowPlace {
        pos: None,
        size: Some((300.0, 200.0)),
    };
    *w.host.during_move.lock().unwrap() = Some(Box::new(|state: &LayoutState| {
        assert!(state.0.try_lock().is_ok(), "끼어들기는 락 밖");
    }));
    w.host.move_window(&label, &size_only).unwrap();
    assert_eq!(
        w.host.steps()[1..],
        [MoveStep::Size],
        "빈 칸은 그 걸음을 건너뛴다"
    );
    assert_eq!(w.host.moves.lock().unwrap().len(), 2);

    w.host.open_at("slot-popup-99", &at, false).unwrap();
    assert!(w.host.is_open("slot-popup-99"), "open_at 도 연 창이다");
    assert_eq!(
        w.host.opened_at.lock().unwrap().last(),
        Some(&("slot-popup-99".to_string(), at, false))
    );
}

// ── create_tab ───────────────────────────────────────────────────────────────

#[test]
fn create_tab_adds_tab_and_notifies() {
    let w = World::new();
    let id = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();

    let tabs = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();
    assert_eq!(tabs.active, id, "새 탭이 활성");
    assert_eq!(tabs.tabs.len(), 2);
    assert_eq!(w.layout_events(), 1);
    assert_eq!(w.tab_events(), 1);
    assert_eq!(w.resyncs(), 1);
}

#[test]
fn create_tab_unknown_window_is_err_without_notify() {
    let w = World::new();
    let err = apply::create_tab(&w.state, &w.subs, &w.ev, "no-such", None).unwrap_err();
    assert!(err.contains("window 없음"), "err={err}");
    assert_eq!(w.layout_events(), 0);
    assert_eq!(w.tab_events(), 0);
    assert_eq!(w.resyncs(), 0, "변형 실패 = 재동기 없음");
}

// ── create_window ────────────────────────────────────────────────────────────

#[test]
fn create_window_registers_model_and_opens_os_window() {
    let w = World::new();
    let label = apply::create_window(&w.state, &w.subs, &w.host, &w.labels).unwrap();

    assert_eq!(&*w.host.opened.lock().unwrap(), &[label.clone()]);
    assert_eq!(
        &*w.host.placed.lock().unwrap(),
        &[label.clone()],
        "첫 자리 기록 1회"
    );
    assert!(apply::list_windows(&w.state).unwrap().contains(&label));
    assert_eq!(
        apply::list_tabs(&w.state, &label).unwrap().tabs.len(),
        1,
        "빈 탭 1개"
    );
    // 성공 경로는 프론트에 아무것도 안 쏜다(창 mount 시 list_tabs pull — §3-3).
    assert_eq!(w.layout_events() + w.tab_events(), 0);
}

// TRD S21-slot-docking §8-1 ⑬ — 새로 짓기 차례 `open` → 모델 넣기 → `record_placement`(알림 없음). 모델에 들기 전에
// 연다는 것은 가짜 `open` 이 스스로 잰다. 넣기가 거절되는 갈래(묘비)는 `commands::popout` 의 셸 단위 시험이 잰다 — 묘비를
// 다는 함수가 이 crate 에서 닿지 않는다.
#[test]
fn create_window_opens_then_inserts_then_records_the_placement() {
    let w = World::new();
    *w.host.during_open.lock().unwrap() = Some(Box::new(|state: &LayoutState| {
        assert!(
            !state.0.lock().unwrap().windows.contains_key("slot-popup-1"),
            "여는 동안 모델은 그 창을 모른다"
        );
    }));

    let label = apply::create_window(&w.state, &w.subs, &w.host, &w.labels).unwrap();

    assert_eq!(
        w.journal.take(),
        [format!("open:{label}"), format!("place:{label}")]
    );
    assert_eq!(w.resyncs(), 1, "넣기 한 번");
}

// 포트 계약 — `open` 이 `Err` 면 떠 있는 창이 없다. 모델은 손대지 않았으니 되돌릴 것도 거둘 창도 없다.
#[test]
fn create_window_open_failure_leaves_the_model_untouched() {
    let w = World::failing_host();
    let version = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
        .unwrap()
        .version;

    let err = apply::create_window(&w.state, &w.subs, &w.host, &w.labels).unwrap_err();
    assert!(err.contains("창 생성 실패"), "err={err}");

    let label = w.host.opened.lock().unwrap()[0].clone();
    assert!(
        !apply::list_windows(&w.state).unwrap().contains(&label),
        "못 연 창은 모델에 들지 않는다 — 유령 창 금지"
    );
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .version,
        version
    );
    assert_eq!(w.resyncs(), 0);
    assert_eq!(
        w.journal.take(),
        [format!("open:{label}")],
        "닫기 · 자리 기록 없음"
    );
}

// ── create_window: 예열 창(TRD S21-slot-docking §8-1 ⑬ · ADR-0299) ──

// 예열 차례 `claim_spare` → `open` → 모델 넣기 → `committed` → `record_placement`(알림 없음).
#[test]
fn create_window_opens_the_prewarmed_window_inserts_then_commits_before_the_placement() {
    let w = World::new();
    let spare = w.prewarm();

    let label = apply::create_window(&w.state, &w.subs, &w.host, &w.labels).unwrap();

    assert_eq!(label, spare, "맡긴 창의 label 로 연다");
    assert_eq!(
        w.journal.take(),
        [
            format!("claim:{label}"),
            format!("open:{label}"),
            format!("committed:{label}"),
            format!("place:{label}"),
        ]
    );
    assert!(apply::list_windows(&w.state).unwrap().contains(&label));
    assert!(!w.claim_outstanding());
    let next = apply::create_window(&w.state, &w.subs, &w.host, &w.labels).unwrap();
    assert_eq!(next, "slot-popup-2", "맡길 창이 없으면 발급기의 새 label");
}

// 맡긴 창의 `open` 이 `Err` 면 창 포트가 그 창을 이미 거뒀다 — 확정도 되물림도 없고 모델은 그대로다.
#[test]
fn create_window_whose_prewarmed_window_fails_to_open_neither_commits_nor_releases() {
    let w = World::failing_host();
    let spare = w.prewarm();

    let err = apply::create_window(&w.state, &w.subs, &w.host, &w.labels).unwrap_err();

    assert!(err.contains("창 생성 실패"), "err={err}");
    assert_eq!(
        w.journal.take(),
        [format!("claim:{spare}"), format!("open:{spare}")]
    );
    assert!(!apply::list_windows(&w.state).unwrap().contains(&spare));
    assert_eq!(w.resyncs(), 0);
}

// 모델 넣기가 거절되면(여는 사이 같은 label 의 창이 모델에 섰다) 맡긴 창은 닫지 않고 되물린다.
#[test]
fn create_window_insert_refusal_gives_the_prewarmed_window_back() {
    let w = World::new();
    let spare = w.prewarm();
    let taken = spare.clone();
    *w.host.during_open.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        state.0.lock().unwrap().create_window(&taken).unwrap();
    }));

    assert!(apply::create_window(&w.state, &w.subs, &w.host, &w.labels).is_err());

    assert_eq!(
        w.journal.take(),
        [
            format!("claim:{spare}"),
            format!("open:{spare}"),
            format!("release:{spare}"),
        ],
        "닫기 · 확정 · 자리 기록 없음"
    );
    assert!(!w.claim_outstanding());
}

// 연 뒤 락이 오염돼 넣지 못해도 연 창은 남지 않는다 — 새로 지은 창은 닫고, 맡긴 창은 되물린다.
#[test]
fn create_window_on_a_poisoned_lock_withdraws_the_window_it_opened() {
    for prewarmed in [false, true] {
        let w = World::new();
        let spare = prewarmed.then(|| w.prewarm());
        *w.host.during_open.lock().unwrap() = Some(Box::new(poison));

        assert!(apply::create_window(&w.state, &w.subs, &w.host, &w.labels).is_err());

        let journal = w.journal.take();
        let expected = match spare {
            Some(label) => vec![
                format!("claim:{label}"),
                format!("open:{label}"),
                format!("release:{label}"),
            ],
            None => vec![
                "open:slot-popup-1".to_string(),
                "close:slot-popup-1".to_string(),
            ],
        };
        assert_eq!(journal, expected, "prewarmed={prewarmed}");
        assert!(w.host.placed.lock().unwrap().is_empty());
    }
}

// ★실 `PopupCounter` 를 통과시킨다★: prefix 는 capabilities/popup.json 의 glob 과 짝이고(다른 label 이면
// Destroyed 정리 게이트가 스킵돼 라우팅·구독·Channel 이 샌다), 단조성은 닫힌 label 재-build 에러를 막는다.
// 이 계약의 단위 테스트는 `popout.rs` 안에 따로 있지만 이 단언은 그대로 둔다 — ★**서비스 경유로 재는 것은
// 다른 층**★이라 중복이 아니다.
#[test]
fn create_window_issues_monotonic_popup_labels_from_real_counter() {
    let w = World::new();
    let a = apply::create_window(&w.state, &w.subs, &w.host, &w.labels).unwrap();
    let b = apply::create_window(&w.state, &w.subs, &w.host, &w.labels).unwrap();
    assert_eq!(a, "slot-popup-1");
    assert_eq!(b, "slot-popup-2");

    apply::close_window(&w.state, &w.subs, &w.host, &a).unwrap();
    let c = apply::create_window(&w.state, &w.subs, &w.host, &w.labels).unwrap();
    assert_eq!(c, "slot-popup-3", "닫힌 label 을 되돌려 쓰지 않는다");
    assert!(apply::list_windows(&w.state).unwrap().contains(&c));
}

// ── switch_tab ───────────────────────────────────────────────────────────────

#[test]
fn switch_tab_changes_active_tab() {
    let w = World::new();
    let first = w.main_active();
    let second = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();

    apply::switch_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, first).unwrap();
    assert_eq!(w.main_active(), first);
    assert_ne!(w.main_active(), second);
    assert_eq!(w.last_tabs().active, first, "탭 알림이 활성 탭을 나른다");
}

#[test]
fn switch_tab_foreign_view_is_err() {
    let w = World::new();
    let label = w.popup();
    let foreign = apply::list_tabs(&w.state, &label).unwrap().active;
    let before = w.main_active();

    let err = apply::switch_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, foreign).unwrap_err();
    assert!(err.contains("view 없음"), "err={err}");
    assert_eq!(
        w.main_active(),
        before,
        "타 창 탭으로 전환 불가 — 활성 불변"
    );
}

// ── switch_tab_if(끌기 되돌리기 — TRD S21-slot-docking §4-6) ───────────────────

#[test]
fn switch_tab_if_switches_when_active_matches() {
    let w = World::new();
    let first = w.main_active();
    let dragged = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    assert_eq!(w.main_active(), dragged, "전제: 새 탭이 활성");
    w.ev.take_order();
    let resyncs = w.resyncs();

    let switched =
        apply::switch_tab_if(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, first, dragged).unwrap();
    assert!(switched);
    assert_eq!(w.main_active(), first);
    assert_eq!(w.last_tabs().active, first, "탭 알림이 활성 탭을 나른다");
    assert_eq!(w.resyncs(), resyncs + 1);
    assert_notified(&w, "switch_tab_if");
}

// 되돌리기가 닿기 전에 사용자 · LLM 이 다른 탭을 골랐다 — 그 선택을 덮지 않고 아무것도 안 건드린다.
#[test]
fn switch_tab_if_mismatch_is_noop_without_notification() {
    let w = World::new();
    let first = w.main_active();
    let dragged = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let chosen = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    assert_eq!(
        w.main_active(),
        chosen,
        "전제: 끄는 탭이 아닌 다른 탭이 활성"
    );
    w.ev.take_order();
    let (resyncs, layouts, tabs) = (w.resyncs(), w.layout_events(), w.tab_events());
    let version = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
        .unwrap()
        .version;

    let switched =
        apply::switch_tab_if(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, first, dragged).unwrap();
    assert!(!switched);
    assert_eq!(w.main_active(), chosen, "고른 탭이 그대로");
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .version,
        version
    );
    assert_eq!(
        (w.resyncs(), w.layout_events(), w.tab_events()),
        (resyncs, layouts, tabs),
        "재동기 · 알림 없음"
    );
    assert!(w.ev.take_order().is_empty(), "알림도 없음");
}

#[test]
fn switch_tab_if_errors_like_switch_tab() {
    let w = World::new();
    let label = w.popup();
    let foreign = apply::list_tabs(&w.state, &label).unwrap().active;
    let active = w.main_active();

    let err = apply::switch_tab_if(&w.state, &w.subs, &w.ev, "no-such-window", active, active)
        .unwrap_err();
    assert!(err.contains("window 없음"), "err={err}");
    // 조건이 맞든 어긋나든 타 창 탭은 오류다.
    for expect in [active, foreign] {
        let err =
            apply::switch_tab_if(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, foreign, expect)
                .unwrap_err();
        assert!(err.contains("view 없음"), "err={err}");
    }
    assert_eq!(w.main_active(), active, "활성 불변");
}

// ── close_tab ────────────────────────────────────────────────────────────────

#[test]
fn close_tab_keeps_main_alive_with_fresh_tab() {
    let w = World::new();
    let only = w.main_active();

    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, MAIN_WINDOW_LABEL, only).unwrap();

    let tabs = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();
    assert_eq!(tabs.tabs.len(), 1, "main 은 최소 1탭(불변식 4)");
    assert_ne!(tabs.active, only, "새 빈 탭으로 교체");
    assert!(w.host.closed.lock().unwrap().is_empty(), "main OS 창 유지");
    assert_eq!(w.tab_events(), 1);
}

#[test]
fn close_tab_last_popup_tab_closes_os_window_without_notify() {
    let w = World::new();
    let label = w.popup();
    let view = apply::list_tabs(&w.state, &label).unwrap().active;
    let events_before = w.layout_events() + w.tab_events();

    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, &label, view).unwrap();

    assert_eq!(&*w.host.closed.lock().unwrap(), &[label.clone()]);
    assert!(!apply::list_windows(&w.state).unwrap().contains(&label));
    assert_eq!(
        w.layout_events() + w.tab_events(),
        events_before,
        "창이 사라진 뒤엔 그 창 탭 알림을 쏘지 않는다"
    );
}

#[test]
fn close_tab_unknown_view_is_err() {
    let w = World::new();
    let err = apply::close_tab(
        &w.state,
        &w.subs,
        &w.ev,
        &w.host,
        MAIN_WINDOW_LABEL,
        Uuid::new_v4(),
    )
    .unwrap_err();
    assert!(err.contains("view 없음"), "err={err}");
    assert!(w.host.closed.lock().unwrap().is_empty());
}

// ── close_window ─────────────────────────────────────────────────────────────

#[test]
fn close_window_drops_model_closes_os_window_and_unsubscribes() {
    let w = World::new();
    let label = w.popup();
    let view = apply::list_tabs(&w.state, &label).unwrap().active;
    let slot = w.empty_slot(view);
    let agent = Uuid::new_v4();
    apply::assign_agent(&w.state, &w.subs, &w.ev, view, slot, agent.to_string()).unwrap();

    apply::close_window(&w.state, &w.subs, &w.host, &label).unwrap();

    assert!(!apply::list_windows(&w.state).unwrap().contains(&label));
    assert_eq!(&*w.host.closed.lock().unwrap(), &[label]);
    assert_eq!(
        w.unsubscribed(),
        vec![agent],
        "어느 창에도 안 남은 agent 는 구독 해제"
    );
}

#[test]
fn close_window_main_is_rejected_and_os_window_untouched() {
    let w = World::new();
    let err = apply::close_window(&w.state, &w.subs, &w.host, MAIN_WINDOW_LABEL).unwrap_err();
    assert!(err.contains("메인 창은 닫을 수 없음"), "err={err}");
    assert!(
        w.host.closed.lock().unwrap().is_empty(),
        "거부 시 OS 창 close 0"
    );
    assert!(apply::list_windows(&w.state)
        .unwrap()
        .contains(&MAIN_WINDOW_LABEL.to_string()));
}

// ── split_slot / close_slot ──────────────────────────────────────────────────

#[test]
fn split_slot_returns_new_slot_and_notifies() {
    let w = World::new();
    let view = w.main_active();
    let before = w.slots(view).len();
    let target = w.empty_slot(view);

    let new_id =
        apply::split_slot(&w.state, &w.subs, &w.ev, view, target, SplitDir::TopBottom).unwrap();

    let slots = w.slots(view);
    assert_eq!(slots.len(), before + 1);
    assert!(slots.contains(&new_id));
    assert_eq!(w.layout_events(), 1);
    assert_eq!(w.tab_events(), 1);
}

#[test]
fn split_slot_unknown_slot_is_err() {
    let w = World::new();
    let view = w.main_active();
    let err = apply::split_slot(
        &w.state,
        &w.subs,
        &w.ev,
        view,
        Uuid::new_v4(),
        SplitDir::TopBottom,
    )
    .unwrap_err();
    assert!(err.contains("slot 없음"), "err={err}");
    assert_eq!(w.layout_events(), 0);
}

#[test]
fn close_slot_removes_slot_and_unsubscribes_vanished_agent() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);
    let agent = Uuid::new_v4();
    apply::assign_agent(&w.state, &w.subs, &w.ev, view, slot, agent.to_string()).unwrap();
    assert!(w.unsubscribed().is_empty(), "배정 직후엔 해제 없음");

    apply::close_slot(&w.state, &w.subs, &w.ev, view, slot).unwrap();

    assert!(!w.slots(view).contains(&slot));
    assert_eq!(w.unsubscribed(), vec![agent]);
}

#[test]
fn close_slot_unknown_view_is_err() {
    let w = World::new();
    let view = w.main_active();
    let slots_before = w.slots(view);

    let err =
        apply::close_slot(&w.state, &w.subs, &w.ev, Uuid::new_v4(), Uuid::new_v4()).unwrap_err();

    assert!(err.contains("view 없음"), "err={err}");
    // 오류 문자열만 보면 "실패했지만 그 전에 재동기·알림은 나갔다"를 통과시킨다 — 거절은 무행위여야 한다.
    assert_eq!(w.slots(view), slots_before, "부분변경 금지");
    assert_eq!(w.resyncs(), 0);
    assert_eq!(w.layout_events() + w.tab_events(), 0);
}

// ── focus_slot (ADR-0066 라우팅 불변) ────────────────────────────────────────

#[test]
fn focus_slot_notifies_without_routing_resync() {
    let w = World::new();
    let view = w.main_active();
    let target = w.empty_slot(view);
    // 분할이 새 칸으로 포커스를 가져가므로 `target` 은 이제 포커스 밖이다 — 그래야 이동을 잰다.
    apply::split_slot(&w.state, &w.subs, &w.ev, view, target, SplitDir::LeftRight).unwrap();
    assert_ne!(w.snapshot(view).focused_slot_id, Some(target));
    let (layout_before, tabs_before, resyncs_before) =
        (w.layout_events(), w.tab_events(), w.resyncs());

    apply::focus_slot(&w.state, &w.ev, view, target).unwrap();

    assert_eq!(w.snapshot(view).focused_slot_id, Some(target));
    assert_eq!(w.layout_events(), layout_before + 1);
    assert_eq!(w.tab_events(), tabs_before + 1);
    assert_eq!(
        w.resyncs(),
        resyncs_before,
        "포커스는 출력 라우팅을 안 바꾼다"
    );
}

#[test]
fn focus_slot_unknown_slot_is_err() {
    let w = World::new();
    let view = w.main_active();
    let before = w.snapshot(view).focused_slot_id;
    let err = apply::focus_slot(&w.state, &w.ev, view, Uuid::new_v4()).unwrap_err();
    assert!(err.contains("slot 없음"), "err={err}");
    assert_eq!(w.snapshot(view).focused_slot_id, before, "부분변경 금지");
}

// ── rename_tab (ADR-0057 탭 알림만) ──────────────────────────────────────────

#[test]
fn rename_tab_notifies_tabs_only() {
    let w = World::new();
    let view = w.main_active();

    apply::rename_tab(&w.state, &w.ev, view, "새 이름".to_string()).unwrap();

    assert_eq!(w.layout_events(), 0, "이름은 뷰 스냅샷에 없다");
    assert_eq!(w.tab_events(), 1);
    assert_eq!(w.resyncs(), 0);
    let named = w.last_tabs();
    assert_eq!(
        named.tabs.iter().find(|t| t.id == view).map(|t| &t.name),
        Some(&"새 이름".to_string())
    );
}

#[test]
fn rename_tab_unknown_view_is_err() {
    let w = World::new();
    let err = apply::rename_tab(&w.state, &w.ev, Uuid::new_v4(), "x".to_string()).unwrap_err();
    assert!(err.contains("view 없음"), "err={err}");
    assert_eq!(w.tab_events(), 0);
}

// ── assign_agent / set_slot_content ──────────────────────────────────────────

#[test]
fn assign_agent_binds_slot_and_resyncs() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);
    let agent = Uuid::new_v4();

    apply::assign_agent(&w.state, &w.subs, &w.ev, view, slot, agent.to_string()).unwrap();

    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&SlotContent::Agent {
            agent_id: agent.to_string()
        })
    );
    assert_eq!(w.resyncs(), 1);
    assert_eq!(w.layout_events(), 1);
}

#[test]
fn assign_agent_unknown_slot_is_err() {
    let w = World::new();
    let view = w.main_active();
    let err = apply::assign_agent(
        &w.state,
        &w.subs,
        &w.ev,
        view,
        Uuid::new_v4(),
        Uuid::new_v4().to_string(),
    )
    .unwrap_err();
    assert!(err.contains("slot 없음"), "err={err}");
    assert_eq!(w.resyncs(), 0);
}

#[test]
fn set_slot_content_replaces_content() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);

    apply::set_slot_content(&w.state, &w.subs, &w.ev, view, slot, SlotContent::AgentList).unwrap();

    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&SlotContent::AgentList)
    );
    assert_eq!(w.layout_events(), 1);
}

#[test]
fn set_slot_content_unknown_view_is_err() {
    let w = World::new();
    let err = apply::set_slot_content(
        &w.state,
        &w.subs,
        &w.ev,
        Uuid::new_v4(),
        Uuid::new_v4(),
        SlotContent::AgentList,
    )
    .unwrap_err();
    assert!(err.contains("view 없음"), "err={err}");
    assert_eq!(w.layout_events(), 0);
}

// ── set_usage_slot (TRD S21 usage-limit-slot §1-7) ──────────────────────────

fn usage_of(w: &World, view: Uuid, slot: Uuid) -> Option<SlotContent> {
    tree::find_slot(&w.snapshot(view).layout, slot).cloned()
}

/// 사용량이 아니던 슬롯 — 뺀 칸은 `true`. 에이전트 슬롯을 덮으면 그 구독이 풀린다(재동기가 락 안에서 돈다).
#[test]
fn set_usage_slot_over_other_content_defaults_missing_flags_to_true() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);
    let agent = Uuid::new_v4();
    apply::assign_agent(&w.state, &w.subs, &w.ev, view, slot, agent.to_string()).unwrap();
    let before = w.resyncs();

    apply::set_usage_slot(&w.state, &w.subs, &w.ev, view, slot, None, Some(false)).unwrap();

    assert_eq!(
        usage_of(&w, view, slot),
        Some(SlotContent::Usage {
            show_claude: true,
            show_codex: false
        })
    );
    assert_eq!(w.resyncs(), before + 1);
    assert_eq!(w.unsubscribed(), vec![agent]);
    assert_eq!(w.layout_events(), 2);
}

/// 이미 사용량이면 뺀 칸은 지금 값 그대로 — 한 칸 토글이 다른 칸을 되돌리지 않는다.
#[test]
fn set_usage_slot_keeps_the_current_value_of_a_missing_flag() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);
    apply::set_usage_slot(
        &w.state,
        &w.subs,
        &w.ev,
        view,
        slot,
        Some(false),
        Some(true),
    )
    .unwrap();

    apply::set_usage_slot(&w.state, &w.subs, &w.ev, view, slot, None, Some(false)).unwrap();
    assert_eq!(
        usage_of(&w, view, slot),
        Some(SlotContent::Usage {
            show_claude: false,
            show_codex: false
        })
    );

    apply::set_usage_slot(&w.state, &w.subs, &w.ev, view, slot, Some(true), None).unwrap();
    assert_eq!(
        usage_of(&w, view, slot),
        Some(SlotContent::Usage {
            show_claude: true,
            show_codex: false
        })
    );
}

#[test]
fn set_usage_slot_unknown_slot_is_err_without_resync() {
    let w = World::new();
    let view = w.main_active();
    let err = apply::set_usage_slot(
        &w.state,
        &w.subs,
        &w.ev,
        view,
        Uuid::new_v4(),
        Some(true),
        Some(true),
    )
    .unwrap_err();
    assert!(err.contains("slot 없음"), "err={err}");
    assert_eq!(w.resyncs(), 0);
    assert_eq!(w.layout_events(), 0);
}

#[test]
fn set_usage_slot_unknown_view_is_err_without_notify() {
    let w = World::new();
    let err = apply::set_usage_slot(
        &w.state,
        &w.subs,
        &w.ev,
        Uuid::new_v4(),
        Uuid::new_v4(),
        None,
        Some(true),
    )
    .unwrap_err();
    assert!(err.contains("view 없음"), "err={err}");
    assert_eq!(w.resyncs(), 0);
    assert_eq!(w.layout_events(), 0);
}

/// 팝업의 두 토글이 방송 전에 연달아 온 경우 — 칸 하나씩만 쓰므로 어느 순서로 닿아도 끝은 같다.
#[test]
fn set_usage_slot_single_field_writes_from_both_vendors_land_in_either_order() {
    type Write = (Option<bool>, Option<bool>);
    let codex_on: Write = (None, Some(true));
    let claude_off: Write = (Some(false), None);
    for order in [[codex_on, claude_off], [claude_off, codex_on]] {
        let w = World::new();
        let view = w.main_active();
        let slot = w.empty_slot(view);
        apply::set_usage_slot(
            &w.state,
            &w.subs,
            &w.ev,
            view,
            slot,
            Some(true),
            Some(false),
        )
        .unwrap();
        for (claude, codex) in order {
            apply::set_usage_slot(&w.state, &w.subs, &w.ev, view, slot, claude, codex).unwrap();
        }
        assert_eq!(
            usage_of(&w, view, slot),
            Some(SlotContent::Usage {
                show_claude: false,
                show_codex: true
            }),
            "order={order:?}"
        );
        assert_eq!(w.layout_events(), 3);
    }
}

// ── move_slot_to_window ──────────────────────────────────────────────────────

impl World {
    fn view_with_filled_slot(&self) -> (Uuid, Uuid, SlotContent) {
        let view = self.main_active();
        let slot = self.empty_slot(view);
        let content = SlotContent::Agent {
            agent_id: Uuid::new_v4().to_string(),
        };
        apply::set_slot_content(
            &self.state,
            &self.subs,
            &self.ev,
            view,
            slot,
            content.clone(),
        )
        .unwrap();
        (view, slot, content)
    }

    // ★롤백을 재는 유일한 눈금★: `views` 는 창 목록과 달리 owner 없는 임시 View 까지 센다. 창 수를 세면
    // 실패 경로엔 창 엔트리가 애초에 안 생겨 롤백을 지워도 초록이다(그 단언은 아무것도 안 잰다).
    fn view_count(&self) -> usize {
        self.state.0.lock().unwrap().views.len()
    }
}

#[test]
fn move_slot_to_window_detaches_content_into_a_new_window() {
    let w = World::new();
    let (view, slot, content) = w.view_with_filled_slot();

    let moved = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    assert_eq!(&*w.host.opened.lock().unwrap(), &[moved.window.clone()]);
    assert_eq!(
        &*w.host.placed.lock().unwrap(),
        &[moved.window.clone()],
        "새 창은 모델에 든 뒤 첫 자리를 적는다"
    );
    assert!(!w.slots(view).contains(&slot), "원본 슬롯은 닫힌다(MOVE)");
    let landed = apply::list_tabs(&w.state, &moved.window).unwrap();
    assert_eq!(landed.active, moved.tab, "새 탭이 그 창의 활성 탭이다");
    let landed_slot = w.slots(moved.tab)[0];
    assert_eq!(
        tree::find_slot(&w.snapshot(moved.tab).layout, landed_slot),
        Some(&content),
        "콘텐츠가 그대로 새 탭에 실린다"
    );
}

/// ★알림이 안 나가면 화면은 옛 배치를 그린다★ — 이 명령은 창 **둘**을 바꾸므로(콘텐츠를 잃은 원본 창,
/// 탭이 하나 는 대상 창) 탭바 알림도 둘이다. 대상 창 몫을 빠뜨리면 새 창 탭바가 빈 채로 뜬다.
#[test]
fn move_slot_to_window_notifies_both_windows() {
    let w = World::new();
    let (view, slot, _) = w.view_with_filled_slot();
    let layout_before = w.layout_events();
    let tabs_before = w.tab_events();

    let moved = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    assert_eq!(
        w.layout_events() - layout_before,
        1,
        "원본 뷰의 새 배치 1회"
    );
    let tab_payloads = w.ev.tabs.lock().unwrap();
    let sent: Vec<&str> = tab_payloads[tabs_before..]
        .iter()
        .map(|t| t.label.as_str())
        .collect();
    assert_eq!(
        sent,
        vec![MAIN_WINDOW_LABEL, moved.window.as_str()],
        "원본 창 → 대상 창 순으로 탭바가 갱신된다"
    );
}

// TRD S21-slot-docking §8-1 ⑬ — 새로 짓기 차례 단계 A(임시 View) → `open` → 단계 C → 알림 → `record_placement`. 첫 자리
// 기록은 알림 뒤다.
#[test]
fn move_slot_to_window_into_a_new_window_records_the_placement_after_the_notifications() {
    let w = World::new();
    let (view, slot, _) = w.view_with_filled_slot();
    let views_before = w.view_count();
    *w.host.during_open.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        assert_eq!(
            state.0.lock().unwrap().views.len(),
            views_before + 1,
            "단계 A 의 임시 View 가 열기 전에 선다"
        );
    }));
    w.journal.take();

    let moved = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    let label = moved.window;
    assert_eq!(
        w.journal.take(),
        [
            format!("open:{label}"),
            "layout".to_string(),
            format!("tabs:{MAIN_WINDOW_LABEL}"),
            format!("tabs:{label}"),
            format!("place:{label}"),
        ]
    );
}

// ── move_slot_to_window: 예열 창(TRD S21-slot-docking §8-1 ⑬ · ADR-0299) ──

// 예열 차례 `claim_spare` → 단계 A → `open` → 단계 C → 알림 → `committed` → `record_placement`.
#[test]
fn move_slot_to_window_opens_the_prewarmed_window_and_commits_after_the_notifications() {
    let w = World::new();
    let (view, slot, content) = w.view_with_filled_slot();
    let spare = w.prewarm();
    let views_before = w.view_count();
    *w.host.during_open.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        assert_eq!(
            state.0.lock().unwrap().views.len(),
            views_before + 1,
            "단계 A 의 임시 View 가 열기 전에 선다"
        );
    }));
    w.journal.take();

    let moved = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    assert_eq!(moved.window, spare, "맡긴 창의 label 로 연다");
    assert_eq!(
        w.journal.take(),
        [
            format!("claim:{spare}"),
            format!("open:{spare}"),
            "layout".to_string(),
            format!("tabs:{MAIN_WINDOW_LABEL}"),
            format!("tabs:{spare}"),
            format!("committed:{spare}"),
            format!("place:{spare}"),
        ]
    );
    assert_eq!(
        apply::list_tabs(&w.state, &spare).unwrap().tabs[0].name,
        "Popup 1",
        "탭 이름도 맡긴 label 로 짓는다"
    );
    let moved_layout = w.snapshot(moved.tab).layout;
    assert_eq!(
        tree::find_slot(&moved_layout, tree::first_slot_id(&moved_layout)),
        Some(&content)
    );
}

// 단계 A 가 거절하면(없는 슬롯) 연 창이 없다 — 맡긴 창만 되물린다.
#[test]
fn move_slot_to_window_refused_before_the_open_gives_the_prewarmed_window_back() {
    let w = World::new();
    let view = w.main_active();
    let spare = w.prewarm();
    w.journal.take();

    let err = apply::move_slot_to_window(
        &w.state,
        &w.subs,
        &w.ev,
        &w.host,
        &w.labels,
        view,
        Uuid::new_v4(),
        None,
    )
    .unwrap_err();

    assert!(err.contains("slot 없음"), "err={err}");
    assert_eq!(
        w.journal.take(),
        [format!("claim:{spare}"), format!("release:{spare}")]
    );
    assert!(w.host.opened.lock().unwrap().is_empty());
}

// 맡긴 창의 `open` 이 `Err` 면 창 포트가 그 창을 거뒀다 — 임시 View 만 되돌리고 확정 · 되물림은 없다.
#[test]
fn move_slot_to_window_whose_prewarmed_window_fails_to_open_rolls_back_without_a_release() {
    let w = World::failing_host();
    let (view, slot, content) = w.view_with_filled_slot();
    let spare = w.prewarm();
    let views_before = w.view_count();
    w.journal.take();

    assert!(apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .is_err());

    assert_eq!(
        w.journal.take(),
        [format!("claim:{spare}"), format!("open:{spare}")]
    );
    assert_eq!(w.view_count(), views_before, "임시 View 를 거뒀다");
    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&content)
    );
}

// 단계 C 가 거절하면(여는 사이 같은 label 의 창이 모델에 섰다) 맡긴 창은 닫지 않고 되물린다.
#[test]
fn move_slot_to_window_refused_at_the_attach_gives_the_prewarmed_window_back() {
    let w = World::new();
    let (view, slot, content) = w.view_with_filled_slot();
    let spare = w.prewarm();
    let taken = spare.clone();
    *w.host.during_open.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        state.0.lock().unwrap().create_window(&taken).unwrap();
    }));
    w.journal.take();

    let err = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap_err();

    assert!(err.contains("탭 삽입 실패"), "err={err}");
    assert_eq!(
        w.journal.take(),
        [
            format!("claim:{spare}"),
            format!("open:{spare}"),
            format!("release:{spare}"),
        ]
    );
    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&content)
    );
}

// 알리는 사이(첫 탭 알림) · 확정 전에 그 창이 모델에서 빠져도 확정은 맡김표를 그대로 넘긴다 — 그 창을 닫는 것은 창 포트 몫이다(TRD
//   §3-9 「자리별 차례」 · I27 · I31).
#[test]
fn move_slot_to_window_commits_the_claim_even_if_the_window_left_the_model_before_it() {
    let w = World::new();
    let (view, slot, _) = w.view_with_filled_slot();
    let spare = w.prewarm();
    let gone = spare.clone();
    *w.ev.on_tabs.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        state.0.lock().unwrap().close_window(&gone).unwrap();
    }));
    w.host.vanished.lock().unwrap().push(spare.clone());
    w.journal.take();

    apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    let journal = w.journal.take();
    assert_eq!(
        journal[journal.len() - 2..],
        [format!("committed:{spare}"), format!("place:{spare}")]
    );
    assert!(!w.window_exists(&spare));
    assert!(
        w.host.closed.lock().unwrap().is_empty(),
        "적용 서비스는 닫지 않는다"
    );
}

// 단계 C 에서 락이 오염돼 있어도 연 창은 남지 않는다 — 새로 지은 창은 닫고, 맡긴 창은 되물린다(임시 View 는 거두지
//   못한다 — `rollback_detached` 와 같다).
#[test]
fn move_slot_to_window_on_a_poisoned_lock_withdraws_the_window_it_opened() {
    for prewarmed in [false, true] {
        let w = World::new();
        let (view, slot, _) = w.view_with_filled_slot();
        let spare = prewarmed.then(|| w.prewarm());
        *w.host.during_open.lock().unwrap() = Some(Box::new(poison));
        w.journal.take();

        assert!(apply::move_slot_to_window(
            &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
        )
        .is_err());

        let expected = match spare {
            Some(label) => vec![
                format!("claim:{label}"),
                format!("open:{label}"),
                format!("release:{label}"),
            ],
            None => vec![
                "open:slot-popup-1".to_string(),
                "close:slot-popup-1".to_string(),
            ],
        };
        assert_eq!(w.journal.take(), expected, "prewarmed={prewarmed}");
        assert!(w.host.placed.lock().unwrap().is_empty());
    }
}

/// ★구독은 창을 갈아타는 동안에도 끊기지 않는다★: agent 는 원본 슬롯에서 사라지지만 같은 순간 대상 탭에
/// 나타난다. 재동기가 attach 전에 계산되거나 close 후에만 돌면 1→0 이 잡혀 살아 있는 구독이 죽는다.
#[test]
fn move_slot_to_window_keeps_the_agent_subscribed_across_the_move() {
    let w = World::new();
    let (view, slot, _) = w.view_with_filled_slot();

    apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    assert!(
        w.unsubscribed().is_empty(),
        "옮겨간 창에서 계속 보이므로 해제할 것이 없다: {:?}",
        w.unsubscribed()
    );
}

#[test]
fn move_slot_to_window_into_an_existing_window_adds_a_tab() {
    let w = World::new();
    let target = w.popup();
    let tabs_before = apply::list_tabs(&w.state, &target).unwrap().tabs.len();
    let placed_before = w.host.placed.lock().unwrap().len();
    let (view, slot, _) = w.view_with_filled_slot();
    let spare = w.prewarm();
    let asked = w.claims_asked();

    let moved = apply::move_slot_to_window(
        &w.state,
        &w.subs,
        &w.ev,
        &w.host,
        &w.labels,
        view,
        slot,
        Some(target.clone()),
    )
    .unwrap();

    assert_eq!(moved.window, target, "새 창을 열지 않는다");
    assert_eq!(
        w.host.placed.lock().unwrap().len(),
        placed_before,
        "이미 있는 창의 자리는 사건이 적는다"
    );
    let tabs = apply::list_tabs(&w.state, &target).unwrap();
    assert_eq!(tabs.tabs.len(), tabs_before + 1);
    assert_eq!(tabs.active, moved.tab);
    w.assert_spare_not_asked(&spare, asked);
}

// ADR-0228
#[test]
fn move_slot_to_window_moves_an_empty_slot_and_promotes_its_sibling() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);
    let sibling =
        apply::split_slot(&w.state, &w.subs, &w.ev, view, slot, SplitDir::LeftRight).expect("분할");

    let moved = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    assert_eq!(&*w.host.opened.lock().unwrap(), &[moved.window.clone()]);
    assert_eq!(
        w.slots(view),
        vec![sibling],
        "원본 슬롯은 닫히고 형제가 자리를 물려받는다"
    );
    let landed = apply::list_tabs(&w.state, &moved.window).unwrap();
    assert_eq!(landed.active, moved.tab);
    let landed_slots = w.slots(moved.tab);
    assert_eq!(landed_slots.len(), 1);
    assert_eq!(
        tree::find_slot(&w.snapshot(moved.tab).layout, landed_slots[0]),
        Some(&SlotContent::Empty),
        "새 탭은 빈 칸 하나다"
    );
    assert!(w.unsubscribed().is_empty(), "{:?}", w.unsubscribed());
}

// 탭에 칸이 하나뿐이면 닫힌 자리에 새 빈 칸이 선다(`close_slot` 과 같은 규칙) — 원본 탭이 비지 않는다.
#[test]
fn move_slot_to_window_of_a_lone_empty_slot_leaves_a_fresh_empty_slot() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);

    apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    let left = w.slots(view);
    assert_eq!(left.len(), 1);
    assert_ne!(left[0], slot, "옮긴 칸이 아니라 새로 선 칸이다");
    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, left[0]),
        Some(&SlotContent::Empty)
    );
}

// ★거절 사유가 진짜 원인을 말한다★ — 없는 칸·없는 탭을 「빈 슬롯」으로 부르면 호출자가 엉뚱한 것을 고친다.
#[test]
fn move_slot_to_window_refuses_a_missing_slot_or_view_with_the_real_cause() {
    let w = World::new();
    let view = w.main_active();
    let ghost = Uuid::new_v4();
    let views_before = w.view_count();

    let err = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, ghost, None,
    )
    .unwrap_err();
    assert!(err.contains("slot 없음"), "err={err}");
    assert!(!err.contains("빈 슬롯"), "err={err}");

    let err = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, ghost, ghost, None,
    )
    .unwrap_err();
    assert!(err.contains("view 없음"), "err={err}");

    assert!(w.host.opened.lock().unwrap().is_empty(), "창도 안 연다");
    assert_eq!(w.view_count(), views_before, "임시 View 를 남기지 않는다");
}

/// ★옮기던 빈 칸이 창 빌드 사이 채워지면 원본을 닫지 않는다★ — 대조 기준이 `Empty` 여도 「옮긴 그대로일
/// 때만 닫는다」가 선다. 닫으면 그 사이 누군가 채운 콘텐츠가 어느 화면에도 안 남는다.
#[test]
fn move_slot_to_window_keeps_an_empty_source_that_was_filled_during_the_window_build() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);
    *w.host.during_open.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        state
            .0
            .lock()
            .unwrap()
            .set_slot_content(view, slot, SlotContent::AgentList)
            .unwrap();
    }));

    let moved = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap();

    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&SlotContent::AgentList),
        "그 사이 채운 콘텐츠를 지우지 않는다"
    );
    let landed_slot = w.slots(moved.tab)[0];
    assert_eq!(
        tree::find_slot(&w.snapshot(moved.tab).layout, landed_slot),
        Some(&SlotContent::Empty),
        "대상 탭은 옮긴 그대로(빈 칸) 남는다"
    );
}

/// ★임시 View 가 실제로 회수되는지를 잰다★ — 창 수를 세면 실패 경로엔 창 엔트리가 안 생겨 롤백을
/// 통째로 지워도 초록이고, 그 사이 owner 없는 View 가 재시도마다 `views` 에 무한히 쌓인다
/// (`apply.rs` 「owner-less tmp_view」 불변식의 ④ 종점이 사라진다).
#[test]
fn move_slot_to_window_rolls_back_when_the_window_cannot_be_built() {
    let w = World::failing_host();
    let (view, slot, content) = w.view_with_filled_slot();
    let views_before = w.view_count();
    let resyncs_before = w.resyncs();

    let err = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap_err();

    assert!(err.contains("창 생성 실패"), "err={err}");
    assert!(
        w.host.placed.lock().unwrap().is_empty(),
        "못 연 창은 적지 않는다"
    );
    assert_eq!(
        w.view_count(),
        views_before,
        "임시 View 가 회수되지 않았다 — 실패한 pop-out 마다 유령 View 가 쌓인다"
    );
    assert_eq!(
        w.resyncs() - resyncs_before,
        2,
        "phase A 재동기 + 롤백 재동기 = 2(롤백이 빠지면 1)"
    );
    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&content),
        "빌드 실패 시 소스 슬롯은 그대로 — 사용자가 콘텐츠를 잃지 않는다"
    );
}

/// ★`is_open` 만이 거를 수 있는 그 상태를 세운다★: 대상 창이 **모델에는 살아 있는데** OS 창은 이미 사라진
/// 순간(`Destroyed` 정리가 아직 모델을 안 지웠다). `insert_tab_into` 의 모델 재검증은 이것을 통과시키므로,
/// `is_open` 이 없으면 콘텐츠가 화면 없는 창의 탭으로 들어가고 소스 슬롯은 닫혀 어느 화면에도 안 남는다.
#[test]
fn move_slot_to_window_rejects_a_target_whose_os_window_is_gone() {
    let w = World::new();
    let target = w.popup();
    // OS 창만 사라진 상태 — 모델 엔트리는 그대로다.
    w.host.closed.lock().unwrap().push(target.clone());
    let (view, slot, content) = w.view_with_filled_slot();
    let views_before = w.view_count();
    let tabs_before = apply::list_tabs(&w.state, &target).unwrap().tabs.len();

    let err = apply::move_slot_to_window(
        &w.state,
        &w.subs,
        &w.ev,
        &w.host,
        &w.labels,
        view,
        slot,
        Some(target.clone()),
    )
    .unwrap_err();

    assert!(err.contains("대상 창 없음"), "err={err}");
    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&content),
        "거절은 무행위여야 한다 — 소스 슬롯을 닫으면 콘텐츠가 어느 화면에도 안 남는다"
    );
    assert_eq!(
        apply::list_tabs(&w.state, &target).unwrap().tabs.len(),
        tabs_before,
        "화면 없는 창에 탭을 밀어 넣지 않는다"
    );
    assert_eq!(w.view_count(), views_before, "임시 View 도 회수된다");
}

// 모델에도 없는 label 은 `is_open` 이 먼저 걸러 「대상 창 없음」으로 나간다(모델 재검증까지 가지 않는다).
#[test]
fn move_slot_to_window_rejects_an_unknown_target_label() {
    let w = World::new();
    let (view, slot, content) = w.view_with_filled_slot();
    let views_before = w.view_count();

    let err = apply::move_slot_to_window(
        &w.state,
        &w.subs,
        &w.ev,
        &w.host,
        &w.labels,
        view,
        slot,
        Some("no-such".to_string()),
    )
    .unwrap_err();

    assert!(err.contains("대상 창 없음"), "err={err}");
    assert_eq!(
        tree::find_slot(&w.snapshot(view).layout, slot),
        Some(&content),
        "거절은 무행위여야 한다"
    );
    assert_eq!(w.view_count(), views_before, "임시 View 회수");
}

// ── spawn_into ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn spawn_into_creates_tab_and_places_agent() {
    let w = World::new();
    let agent = Uuid::new_v4();
    let spawner = Spawner::ok(&w.state, agent);

    let id = apply::spawn_into(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        None,
        None,
        None,
        "C:/tmp".to_string(),
    )
    .await
    .unwrap();

    assert_eq!(id, agent.to_string());
    assert_eq!(spawner.calls(), 1);
    assert_eq!(&*spawner.cwds.lock().unwrap(), &["C:/tmp".to_string()]);
    let placed = w.main_active();
    let slots = w.slots(placed);
    assert_eq!(slots.len(), 1, "새 탭 = 빈 root 1개");
    assert_eq!(
        tree::find_slot(&w.snapshot(placed).layout, slots[0]),
        Some(&SlotContent::Agent {
            agent_id: agent.to_string()
        })
    );
    assert_eq!(w.layout_events(), 1);
}

/// ★모르는 낱말만 막는다★ — 오탈자가 통과하면 그 왕복은 데몬에서 죽거나(새 데몬) 조용히 다른 백엔드를
/// 띄운다(칸을 모르는 옛 데몬). 거절 문구는 어휘를 손으로 적지 않고 wire enum 에서 받아 온다.
#[tokio::test]
async fn spawn_into_rejects_an_unknown_backend_before_spawning() {
    let w = World::new();
    let spawner = Spawner::ok(&w.state, Uuid::new_v4());

    let err = apply::spawn_into(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        None,
        None,
        Some("codx".to_string()),
        "C:/tmp".to_string(),
    )
    .await
    .unwrap_err();

    assert!(err.contains("스폰 안 함"), "err={err}");
    assert!(
        err.contains("claude") && err.contains("codex"),
        "무엇이 통하는지 말해야: {err}"
    );
    assert_eq!(spawner.calls(), 0, "★스폰 전에 거부★");
}

/// 아는 낱말은 **포트까지 그대로 간다** — 이 줄이 초록이면 ADR-0058 의 전량 거부가 실제로 걷힌 것이다.
///
/// ★셸은 정책을 안 본다 — wire 가 아는 낱말은 전부 여기 선다★(정책은 데몬 `SpawnByCwd` 처리부 · ADR-0279).
/// 그 목록에서 낱말을 빼면 이 파일은 그 낱말이 실제로 포트까지 가는지를 아무 데서도 안 재게 된다.
#[tokio::test]
async fn spawn_into_forwards_a_known_backend() {
    for (word, expected) in [
        ("claude", AgentBackendKind::Claude),
        (" claude ", AgentBackendKind::Claude),
        ("codex", AgentBackendKind::Codex),
        (" codex ", AgentBackendKind::Codex),
    ] {
        let w = World::new();
        let agent = Uuid::new_v4();
        let spawner = Spawner::ok(&w.state, agent);

        let id = apply::spawn_into(
            &w.state,
            &w.subs,
            &w.ev,
            &spawner,
            MAIN_WINDOW_LABEL,
            None,
            None,
            Some(word.to_string()),
            "C:/tmp".to_string(),
        )
        .await
        .unwrap();

        assert_eq!(id, agent.to_string());
        assert_eq!(
            &*spawner.backends.lock().unwrap(),
            &[Some(expected)],
            "{word}"
        );
    }
}

#[tokio::test]
async fn spawn_into_accepts_blank_backend_as_unspecified() {
    let w = World::new();
    let agent = Uuid::new_v4();
    let spawner = Spawner::ok(&w.state, agent);
    let tabs_before = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
        .unwrap()
        .tabs
        .len();

    let id = apply::spawn_into(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        None,
        None,
        Some("   ".to_string()),
        "C:/tmp".to_string(),
    )
    .await
    .unwrap();

    // 공백 backend 가 "미지정"으로 통과했는지는 스폰 호출만으로는 안 잡힌다 — 그 뒤 배치까지 끝나야
    // 통과 경로와 거절 경로가 갈린다(거절 경로는 스폰도 배치도 0).
    assert_eq!(spawner.calls(), 1);
    // ★셸은 기본값을 지어내지 않는다★ — 미지정은 미지정인 채로 포트에 간다(그 부재를 거절하는 것은
    //   데몬이고, 그래야 「어느 칸을 채워라」 문구가 한 곳에서만 나온다).
    assert_eq!(&*spawner.backends.lock().unwrap(), &[None]);
    assert_eq!(id, agent.to_string());
    let placed = w.main_active();
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .tabs
            .len(),
        tabs_before + 1,
        "tab=None → 새 탭"
    );
    let slots = w.slots(placed);
    assert_eq!(
        tree::find_slot(&w.snapshot(placed).layout, slots[0]),
        Some(&SlotContent::Agent {
            agent_id: agent.to_string()
        })
    );
}

#[tokio::test]
async fn spawn_into_rejects_slot_without_tab_before_spawning() {
    let w = World::new();
    let spawner = Spawner::ok(&w.state, Uuid::new_v4());

    let err = apply::spawn_into(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        None,
        Some(Uuid::new_v4()),
        None,
        "C:/tmp".to_string(),
    )
    .await
    .unwrap_err();

    assert!(err.contains("스폰 안 함"), "err={err}");
    assert_eq!(spawner.calls(), 0, "orphan 탭 방지 가드 = pre-spawn");
}

#[tokio::test]
async fn spawn_into_reports_live_agent_when_placement_fails() {
    let w = World::new();
    let label = w.popup();
    let foreign_tab = apply::list_tabs(&w.state, &label).unwrap().active;
    let agent = Uuid::new_v4();
    let spawner = Spawner::ok(&w.state, agent);

    let err = apply::spawn_into(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        Some(foreign_tab),
        None,
        None,
        "C:/tmp".to_string(),
    )
    .await
    .unwrap_err();

    assert_eq!(spawner.calls(), 1, "스폰은 이미 일어났다(하드 롤백 없음)");
    assert!(
        err.contains(&agent.to_string()),
        "생존 agent id 를 오류에 박아 invisible 에이전트를 막는다: err={err}"
    );
}

#[tokio::test]
async fn spawn_into_propagates_spawn_failure_untouched() {
    let w = World::new();
    let spawner = Spawner::failing(&w.state);

    let err = apply::spawn_into(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        None,
        None,
        None,
        "C:/tmp".to_string(),
    )
    .await
    .unwrap_err();

    assert_eq!(err, "spawn 실패: 데몬 거절");
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .tabs
            .len(),
        1,
        "스폰 실패면 탭도 안 만든다"
    );
    assert_eq!(w.resyncs(), 0);
}

/// ★아는 낱말의 정책 거절은 데몬을 한 번 다녀와 배치 없이 그대로 돌아온다★(ADR-0279) — 셸은 그 낱말을
/// 그물만 지나 포트로 보내고, 포트의 `Err` 를 문구 그대로 돌려주며 레이아웃을 안 건드린다.
/// `spawn_into_propagates_spawn_failure_untouched` 는 낱말이 없어 「낱말이 데몬까지 갔다가 거절되어
/// 돌아온다」를 못 잰다.
/// 문구는 데몬 정책 거절의 꼴을 흉내 낸 것이다 — 셸은 그 글을 읽지 않으므로 무엇이든 같다.
#[tokio::test]
async fn spawn_into_passes_a_daemon_refusal_through_without_placing() {
    let w = World::new();
    let refusal = "backend 'codex' 는 아는 낱말이지만 이 표면으로는 지금 만들지 않는다 — 시험용 사유. 스폰 안 함.";
    let spawner = Spawner::new(&w.state, Err(refusal.to_string()));

    let err = apply::spawn_into(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        None,
        None,
        Some("codex".to_string()),
        "C:/tmp".to_string(),
    )
    .await
    .unwrap_err();

    assert_eq!(spawner.calls(), 1, "셸은 정책을 안 보고 데몬에 묻는다");
    assert_eq!(
        &*spawner.backends.lock().unwrap(),
        &[Some(AgentBackendKind::Codex)],
        "고른 낱말이 포트까지 갔다"
    );
    assert_eq!(err, refusal, "데몬 문구가 그대로 돌아온다");
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .tabs
            .len(),
        1,
        "거절이면 탭도 안 만든다"
    );
    assert_eq!(w.resyncs(), 0);
}

// ── set_split_ratio (ADR-0227 — 레이아웃 알림은 Applied 일 때만) ──────────────

impl World {
    // main 활성 탭의 첫 칸을 좌우로 나눈 세계 — 반환 = (탭, 그 분할).
    fn split_view(&self) -> (Uuid, Uuid) {
        let view = self.main_active();
        let slot = self.empty_slot(view);
        apply::split_slot(
            &self.state,
            &self.subs,
            &self.ev,
            view,
            slot,
            SplitDir::LeftRight,
        )
        .unwrap();
        (view, self.snapshot(view).split_rects[0].split_id)
    }

    fn last_layout(&self) -> ViewSnapshot {
        self.ev
            .layout
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("레이아웃 알림")
    }
}

#[test]
fn set_split_ratio_applied_notifies_the_layout_once_with_the_new_ratio_and_rects() {
    let w = World::new();
    let (view, split) = w.split_view();
    let (layout_before, tabs_before, resyncs_before) =
        (w.layout_events(), w.tab_events(), w.resyncs());

    let got = apply::set_split_ratio(&w.state, &w.ev, view, split, 0.3).unwrap();

    assert_eq!(got.outcome, SplitRatioOutcome::Applied);
    assert_eq!(got.ratio, 0.3);
    assert_eq!(
        w.layout_events(),
        layout_before + 1,
        "레이아웃 통지 정확히 1"
    );
    assert_eq!(w.tab_events(), tabs_before, "탭 통지 0");
    assert_eq!(w.resyncs(), resyncs_before, "라우팅 불변 — 재동기 없음");

    let snap = w.last_layout();
    assert_eq!(snap.view_id, view);
    assert_eq!(
        got.version, snap.version,
        "반환 version == 통지된 스냅샷 version"
    );
    let LayoutNode::Split { ratio, .. } = &snap.layout else {
        panic!("분할 트리여야 한다: {:?}", snap.layout)
    };
    assert_eq!(*ratio, 0.3);
    let rect = snap
        .split_rects
        .iter()
        .find(|r| r.split_id == split)
        .expect("분할 사각형");
    assert_eq!(rect.at, 0.3, "사각형도 새 비율로 계산된다");
    let left = snap
        .slot_rects
        .iter()
        .find(|r| r.x0 == 0.0)
        .expect("왼쪽 칸");
    assert_eq!(left.x1, 0.3, "a = 왼쪽 칸이 30%");
}

#[test]
fn set_split_ratio_unchanged_or_too_small_notifies_nothing() {
    let w = World::new();
    let (view, split) = w.split_view();
    let events_before = (w.layout_events(), w.tab_events());
    let version = w.snapshot(view).version;

    let same = apply::set_split_ratio(&w.state, &w.ev, view, split, 0.5).unwrap();
    assert_eq!(same.outcome, SplitRatioOutcome::Unchanged);
    assert_eq!(same.ratio, 0.5);

    // 분할 폭 L = 300px < 2 × 200px — px 범위가 비었다.
    apply::report_window_canvas(&w.state, MAIN_WINDOW_LABEL, 300, 300).unwrap();
    let mut big_min = one_px_border();
    big_min.min_pane_px = 200;
    apply::report_ui_metrics(&w.state, MAIN_WINDOW_LABEL, big_min).unwrap();
    let small = apply::set_split_ratio(&w.state, &w.ev, view, split, 0.3).unwrap();
    assert_eq!(small.outcome, SplitRatioOutcome::TooSmall);
    assert_eq!(small.ratio, 0.5, "지금 값 그대로");

    assert_eq!(
        (w.layout_events(), w.tab_events()),
        events_before,
        "무변경 결말은 아무것도 안 알린다"
    );
    assert_eq!(w.snapshot(view).version, version, "version 불변");
}

#[test]
fn set_split_ratio_unknown_split_is_err_without_notify() {
    let w = World::new();
    let (view, split) = w.split_view();
    let events_before = (w.layout_events(), w.tab_events());

    let err = apply::set_split_ratio(&w.state, &w.ev, view, Uuid::new_v4(), 0.3).unwrap_err();
    assert!(err.contains("split 없음"), "err={err}");
    let err = apply::set_split_ratio(&w.state, &w.ev, view, split, f64::NAN).unwrap_err();
    assert!(err.contains("비율 거절"), "err={err}");

    assert_eq!((w.layout_events(), w.tab_events()), events_before);
}

/// 알림을 받는 그 자리에서 다른 쓰기를 끼워 넣는 포트 — 락을 놓은 뒤 전역 카운터를 다시 읽는 구현이면
/// 돌려주는 version 이 이 끼어든 쓰기의 값으로 어긋난다.
struct Meddler {
    inner: Recorder,
    state: LayoutState,
}

impl LayoutEvents for Meddler {
    fn layout_updated(&self, snapshot: &ViewSnapshot) {
        self.inner.layout_updated(snapshot);
        let mut mgr = self.state.0.lock().unwrap();
        mgr.rename_tab(snapshot.view_id, "끼어든 쓰기".to_string())
            .unwrap();
    }

    fn window_tabs_updated(&self, tabs: &WindowTabsPayload) {
        self.inner.window_tabs_updated(tabs);
    }
}

#[test]
fn set_split_ratio_returns_the_notified_snapshots_version_not_a_later_one() {
    let w = World::new();
    let (view, split) = w.split_view();
    let meddler = Meddler {
        inner: Recorder::new(&w.state, Journal::default()),
        state: w.state.clone(),
    };

    let got = apply::set_split_ratio(&w.state, &meddler, view, split, 0.3).unwrap();

    let notified = meddler
        .inner
        .layout
        .lock()
        .unwrap()
        .last()
        .cloned()
        .expect("알림 1");
    assert_eq!(got.version, notified.version);
    assert!(
        w.snapshot(view).version > got.version,
        "전제: 끼어든 쓰기가 전역 카운터를 올렸다"
    );
}

// 쓰기 하나가 알림을 하나 이상 냈다 — 그 차례를 비운다.
fn assert_notified(w: &World, write: &str) {
    assert!(!w.ev.take_order().is_empty(), "{write}: 알림이 있어야 한다");
}

#[tokio::test]
async fn every_notifying_write_notifies() {
    let w = World::new();
    let main = w.main_active();
    let slot = w.empty_slot(main);

    let tab = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    assert_notified(&w, "create_tab");
    apply::switch_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, main).unwrap();
    assert_notified(&w, "switch_tab");
    let added =
        apply::split_slot(&w.state, &w.subs, &w.ev, main, slot, SplitDir::LeftRight).unwrap();
    assert_notified(&w, "split_slot");
    apply::focus_slot(&w.state, &w.ev, main, added).unwrap();
    assert_notified(&w, "focus_slot");
    let split = w.snapshot(main).split_rects[0].split_id;
    let ratio = apply::set_split_ratio(&w.state, &w.ev, main, split, 0.3).unwrap();
    assert_eq!(ratio.outcome, SplitRatioOutcome::Applied);
    assert_notified(&w, "set_split_ratio");
    apply::rename_tab(&w.state, &w.ev, main, "이름".to_string()).unwrap();
    assert_notified(&w, "rename_tab");
    apply::assign_agent(
        &w.state,
        &w.subs,
        &w.ev,
        main,
        slot,
        Uuid::new_v4().to_string(),
    )
    .unwrap();
    assert_notified(&w, "assign_agent");
    apply::set_slot_content(
        &w.state,
        &w.subs,
        &w.ev,
        main,
        added,
        SlotContent::AgentList,
    )
    .unwrap();
    assert_notified(&w, "set_slot_content");
    apply::set_usage_slot(&w.state, &w.subs, &w.ev, main, added, Some(false), None).unwrap();
    assert_notified(&w, "set_usage_slot");
    apply::close_slot(&w.state, &w.subs, &w.ev, main, added).unwrap();
    assert_notified(&w, "close_slot");
    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, MAIN_WINDOW_LABEL, tab).unwrap();
    assert_notified(&w, "close_tab");
    apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, main, slot, None,
    )
    .unwrap();
    assert_notified(&w, "move_slot_to_window");
    let spawner = Spawner::ok(&w.state, Uuid::new_v4());
    apply::spawn_into(
        &w.state,
        &w.subs,
        &w.ev,
        &spawner,
        MAIN_WINDOW_LABEL,
        None,
        None,
        None,
        "C:/tmp".to_string(),
    )
    .await
    .unwrap();
    assert_notified(&w, "spawn_into");
}

#[test]
fn writes_that_change_nothing_notify_nothing() {
    let w = World::new();
    let (view, split) = w.split_view();
    let label = w.popup();
    let popup_tab = apply::list_tabs(&w.state, &label).unwrap().active;
    w.ev.take_order();

    apply::create_tab(&w.state, &w.subs, &w.ev, "no-such", None).unwrap_err();
    apply::rename_tab(&w.state, &w.ev, Uuid::new_v4(), "x".to_string()).unwrap_err();
    apply::focus_slot(&w.state, &w.ev, view, Uuid::new_v4()).unwrap_err();
    let same = apply::set_split_ratio(&w.state, &w.ev, view, split, 0.5).unwrap();
    assert_eq!(same.outcome, SplitRatioOutcome::Unchanged);
    apply::move_slot_to_window(
        &w.state,
        &w.subs,
        &w.ev,
        &w.host,
        &w.labels,
        view,
        Uuid::new_v4(),
        None,
    )
    .unwrap_err();
    // 마지막 탭을 닫아 창째 사라짐 — 알릴 창이 없다.
    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, &label, popup_tab).unwrap();

    assert_eq!(w.ev.take_order(), Vec::<&str>::new());
}

// ── 측정 보고 2종 (ADR-0227) ─────────────────────────────────────────────────
//
// 두 함수는 알림·재동기 포트를 아예 안 받으므로 「통지 0」은 시그니처가 먼저 지킨다. 여기서 재는 것은 그
// 나머지 — version 불변과, 성공·실패 어느 쪽이든 형제 명령의 알림이 딸려 나가지 않는다는 것이다.

fn one_px_border() -> UiMetrics {
    UiMetrics {
        frame_insets: Insets {
            t: 1.0,
            r: 1.0,
            b: 1.0,
            l: 1.0,
        },
        min_pane_px: 30,
    }
}

#[test]
fn canvas_and_metrics_reports_notify_nothing_and_keep_version() {
    let w = World::new();
    let before = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
        .unwrap()
        .version;

    apply::report_window_canvas(&w.state, MAIN_WINDOW_LABEL, 1200, 800).unwrap();
    apply::report_ui_metrics(&w.state, MAIN_WINDOW_LABEL, one_px_border()).unwrap();

    assert_eq!(w.layout_events(), 0, "레이아웃 통지 0");
    assert_eq!(w.tab_events(), 0, "탭 통지 0");
    assert_eq!(w.resyncs(), 0, "라우팅 불변 — 재동기 없음");
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .version,
        before,
        "측정 보고는 version 을 올리지 않는다"
    );

    // 저장은 됐다 — 셸 계산이 그 값을 읽는다.
    let view = w.main_active();
    let slot = w.slots(view)[0];
    let px = w
        .state
        .0
        .lock()
        .unwrap()
        .slot_px(view, slot)
        .unwrap()
        .expect("캔버스·지표 둘 다 보고됨");
    assert_eq!(
        px.frame,
        PxRect {
            x0: 0,
            y0: 0,
            x1: 1200,
            y1: 800
        }
    );
}

#[test]
fn rejected_reports_notify_nothing_and_keep_version() {
    let w = World::new();
    let before = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
        .unwrap()
        .version;

    let err = apply::report_window_canvas(&w.state, "no-such", 1200, 800).unwrap_err();
    assert!(err.contains("window 없음"), "err={err}");
    let err = apply::report_ui_metrics(&w.state, "no-such", one_px_border()).unwrap_err();
    assert!(err.contains("window 없음"), "err={err}");
    let mut bad = one_px_border();
    bad.frame_insets.l = f64::NAN;
    let err = apply::report_ui_metrics(&w.state, MAIN_WINDOW_LABEL, bad).unwrap_err();
    assert!(err.contains("ui 지표 거절"), "err={err}");
    // 0 크기는 오류가 아니라 무시다.
    apply::report_window_canvas(&w.state, MAIN_WINDOW_LABEL, 0, 800).unwrap();

    assert_eq!(w.layout_events() + w.tab_events(), 0);
    assert_eq!(w.resyncs(), 0);
    assert_eq!(
        apply::list_tabs(&w.state, MAIN_WINDOW_LABEL)
            .unwrap()
            .version,
        before
    );
}

// ── read-only 4종 ────────────────────────────────────────────────────────────

#[test]
fn get_view_unknown_is_err() {
    let w = World::new();
    let err = apply::get_view(&w.state, Uuid::new_v4()).unwrap_err();
    assert!(err.contains("view 없음"), "err={err}");
}

#[test]
fn list_tabs_returns_tabs_active_and_version() {
    let w = World::new();
    let before = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();
    apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let after = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();

    assert_eq!(after.label, MAIN_WINDOW_LABEL);
    assert_eq!(after.tabs.len(), before.tabs.len() + 1);
    assert!(after.version > before.version, "변경마다 version 증가");
}

#[test]
fn list_tabs_unknown_window_is_err() {
    let w = World::new();
    let err = apply::list_tabs(&w.state, "no-such").unwrap_err();
    assert!(err.contains("window 없음"), "err={err}");
}

#[test]
fn list_windows_lists_main_and_runtime_windows() {
    let w = World::new();
    assert_eq!(
        apply::list_windows(&w.state).unwrap(),
        vec![MAIN_WINDOW_LABEL.to_string()]
    );
    let label = w.popup();
    let mut got = apply::list_windows(&w.state).unwrap();
    got.sort();
    assert_eq!(got, vec![MAIN_WINDOW_LABEL.to_string(), label]);
}

#[test]
fn resolve_spatial_finds_neighbor_of_focused_slot() {
    let w = World::new();
    let view = w.main_active();
    let left = w.empty_slot(view);
    let right =
        apply::split_slot(&w.state, &w.subs, &w.ev, view, left, SplitDir::LeftRight).unwrap();
    assert_eq!(w.snapshot(view).focused_slot_id, Some(right));

    let got = apply::resolve_spatial(&w.state, "left", None, Some(view)).unwrap();
    assert_eq!(got, Some(left));
}

#[test]
fn resolve_spatial_without_view_uses_window_active_tab() {
    let w = World::new();
    let view = w.main_active();
    let by_view = apply::resolve_spatial(&w.state, "top-left", None, Some(view)).unwrap();

    // window 미지정 → main 의 활성 탭.
    let by_default = apply::resolve_spatial(&w.state, "top-left", None, None).unwrap();
    let by_label =
        apply::resolve_spatial(&w.state, "top-left", Some(MAIN_WINDOW_LABEL), None).unwrap();

    assert_eq!(by_default, by_view);
    assert_eq!(by_label, by_view);
    assert!(by_view.is_some());
}

#[test]
fn resolve_spatial_unknown_token_is_err() {
    let w = World::new();
    let err = apply::resolve_spatial(&w.state, "sideways", None, None).unwrap_err();
    assert!(err.contains("알 수 없는 공간 토큰"), "err={err}");
}

#[test]
fn resolve_spatial_unknown_view_is_err() {
    let w = World::new();
    let err = apply::resolve_spatial(&w.state, "left", None, Some(Uuid::new_v4())).unwrap_err();
    assert!(err.contains("view 없음"), "err={err}");
}

#[test]
fn resolve_spatial_unknown_window_is_err() {
    let w = World::new();
    let err = apply::resolve_spatial(&w.state, "left", Some("no-such"), None).unwrap_err();
    assert!(err.contains("window 없음"), "err={err}");
}

// ── 조회는 방금 쓴 값을 본다(read-your-writes — TRD S20 §7) ──────────────────

#[test]
fn reads_observe_writes_immediately() {
    let w = World::new();
    let tab = apply::create_tab(
        &w.state,
        &w.subs,
        &w.ev,
        MAIN_WINDOW_LABEL,
        Some("T".into()),
    )
    .unwrap();
    let label = w.popup();
    let slot = w.empty_slot(tab);
    let agent = Uuid::new_v4();
    apply::assign_agent(&w.state, &w.subs, &w.ev, tab, slot, agent.to_string()).unwrap();
    apply::rename_tab(&w.state, &w.ev, tab, "R".to_string()).unwrap();
    apply::focus_slot(&w.state, &w.ev, tab, slot).unwrap();

    let snap = apply::get_view(&w.state, tab).unwrap();
    assert_eq!(snap.focused_slot_id, Some(slot));
    assert_eq!(
        tree::find_slot(&snap.layout, slot),
        Some(&SlotContent::Agent {
            agent_id: agent.to_string()
        })
    );
    let tabs = apply::list_tabs(&w.state, MAIN_WINDOW_LABEL).unwrap();
    assert_eq!(tabs.active, tab);
    assert_eq!(
        tabs.tabs.iter().find(|t| t.id == tab).map(|t| &t.name),
        Some(&"R".to_string())
    );
    assert!(apply::list_windows(&w.state).unwrap().contains(&label));
    assert_eq!(
        apply::resolve_spatial(&w.state, "right", None, Some(tab)).unwrap(),
        None,
        "새 탭은 단일 슬롯 — 오른쪽 이웃 없음"
    );
}

// ── dock_tab (TRD S21-slot-docking §3-3) ──────────────────────
//
// 여기서 안 재는 것: 새 창 자리가 어느 모니터에도 안 걸치면 기본 자리로 가는 규칙은 창 포트의 Tauri 구현 몫이라
// `state::placement` 의 `resolve_place` 단위 시험(`lib_unit`)이 잰다. 새 창이 붙기 전에 닫혀 묘비가 선 갈래 · 재사용
// 옮기기 중에 창이 닫힌 갈래는 닫기 요청의 모델 몫이 crate 안 함수라 `commands::popout` 의 시험(`lib_unit`)이 잰다.

// ADR-0057 ①–④ — 소유 쌍이 양쪽에서 맞고 · View 마다 소유 창 하나 · 활성은 그 창의 탭 · main 탭 ≥ 1.
fn assert_invariants(w: &World) {
    let mgr = w.state.0.lock().unwrap();
    let mut tabs = 0;
    for (label, wt) in &mgr.windows {
        assert!(
            wt.tabs.contains(&wt.active),
            "③ {label} 의 활성이 그 창 탭이 아니다"
        );
        for view in &wt.tabs {
            assert_eq!(mgr.view_owner.get(view), Some(label), "① 탭 {view}");
            assert!(mgr.views.contains_key(view), "① 탭 {view} 의 View 가 없다");
        }
        tabs += wt.tabs.len();
    }
    assert_eq!(mgr.views.len(), tabs, "② 창 없는 View 나 두 창의 탭");
    assert_eq!(mgr.view_owner.len(), tabs, "① 남은 소유 쌍");
    assert!(
        mgr.windows
            .get(MAIN_WINDOW_LABEL)
            .is_some_and(|w| !w.tabs.is_empty()),
        "④ main 탭 ≥ 1"
    );
}

const AT: WindowPlace = WindowPlace {
    pos: Some((300, 200)),
    size: None,
};

fn bounds(x: f64, y: f64, w: f64, h: f64) -> Option<WindowBounds> {
    WindowBounds::new(x, y, w, h)
}

// 알림마다 창이 아직 안 닫혔나 본다 — 빈 출처 팝아웃 닫기가 알림 묶음 뒤인지 재는 눈.
struct ClosesAfter<'a> {
    inner: &'a Recorder,
    host: &'a Host,
}

impl ClosesAfter<'_> {
    fn not_closed_yet(&self) {
        assert!(
            self.host.closed.lock().unwrap().is_empty(),
            "빈 출처 팝아웃은 알림을 다 낸 뒤에 닫는다"
        );
    }
}

impl LayoutEvents for ClosesAfter<'_> {
    fn layout_updated(&self, snapshot: &ViewSnapshot) {
        self.not_closed_yet();
        self.inner.layout_updated(snapshot);
    }

    fn window_tabs_updated(&self, tabs: &WindowTabsPayload) {
        self.not_closed_yet();
        self.inner.window_tabs_updated(tabs);
    }
}

impl World {
    fn version(&self) -> u64 {
        self.state.0.lock().unwrap().version
    }

    fn layout_of(&self, view: Uuid) -> LayoutNode {
        self.snapshot(view).layout
    }

    fn owner(&self, view: Uuid) -> Option<String> {
        self.state.0.lock().unwrap().owner_of(view).cloned()
    }

    fn tabs_of(&self, label: &str) -> Vec<Uuid> {
        apply::list_tabs(&self.state, label)
            .map(|t| t.tabs.iter().map(|m| m.id).collect())
            .unwrap_or_default()
    }

    fn window_exists(&self, label: &str) -> bool {
        self.state.0.lock().unwrap().windows.contains_key(label)
    }

    // 빈 탭 `n` 개짜리 팝아웃 — 반환 = (label, 탭 순서). 마지막 탭이 활성이다.
    fn popup_with(&self, n: usize) -> (String, Vec<Uuid>) {
        let label = self.popup();
        let mut tabs = vec![apply::list_tabs(&self.state, &label).unwrap().active];
        for _ in 1..n {
            tabs.push(apply::create_tab(&self.state, &self.subs, &self.ev, &label, None).unwrap());
        }
        (label, tabs)
    }

    // 창의 모델 자리를 적는다 — 운영의 `Moved` · `Resized` 처리기와 같은 길.
    fn observe(&self, label: &str, maximized: bool, bounds: Option<WindowBounds>) {
        self.state
            .0
            .lock()
            .unwrap()
            .observe_window_placement(
                label,
                WindowPlacement {
                    minimized: false,
                    maximized,
                    bounds,
                },
            )
            .unwrap();
    }

    // 준비 쓰기가 남긴 기록을 비운다 — 뒤의 단언은 도킹 한 번만 본다. ★연 창 목록(`opened`)은 남긴다★ — 이 가짜의
    // `is_open` · 되돌리기가 그것으로 창이 떠 있는지 본다.
    fn forget(&self) {
        self.ev.take_order();
        self.ev.layout.lock().unwrap().clear();
        self.ev.tabs.lock().unwrap().clear();
        let h = &self.host;
        h.closed.lock().unwrap().clear();
        h.placed.lock().unwrap().clear();
        h.opened_at.lock().unwrap().clear();
        h.moves.lock().unwrap().clear();
        h.steps.lock().unwrap().clear();
        self.journal.take();
    }

    fn dock(&self, view: Uuid, to: DockDest, focus: FocusPolicy) -> Result<DockOutcome, DockError> {
        self.dock_via(&self.ev, view, to, focus)
    }

    fn dock_via(
        &self,
        events: &dyn LayoutEvents,
        view: Uuid,
        to: DockDest,
        focus: FocusPolicy,
    ) -> Result<DockOutcome, DockError> {
        apply::dock_tab(
            &self.state,
            &self.subs,
            events,
            &self.host,
            &self.labels,
            DockRequest {
                view,
                to,
                focus,
                mv: DockMove::default(),
            },
        )
    }

    fn notified_views(&self) -> Vec<Uuid> {
        self.ev
            .layout
            .lock()
            .unwrap()
            .iter()
            .map(|s| s.view_id)
            .collect()
    }

    fn notified_windows(&self) -> Vec<String> {
        self.ev
            .tabs
            .lock()
            .unwrap()
            .iter()
            .map(|t| t.label.clone())
            .collect()
    }

    fn notified_tabs_of(&self, label: &str) -> WindowTabsPayload {
        self.ev
            .tabs
            .lock()
            .unwrap()
            .iter()
            .find(|t| t.label == label)
            .cloned()
            .unwrap_or_else(|| panic!("{label} 탭 알림"))
    }

    // 낸 스냅숏 · 탭 목록이 전부 지금 `version` 이다 — 변경과 같은 임계 구역에서 떴다.
    fn assert_notified_at_current_version(&self) {
        let version = self.version();
        for s in self.ev.layout.lock().unwrap().iter() {
            assert_eq!(s.version, version, "스냅숏 {}", s.view_id);
        }
        for t in self.ev.tabs.lock().unwrap().iter() {
            assert_eq!(t.version, version, "탭 목록 {}", t.label);
        }
    }
}

// ── dock_tab: 이음새 · 탭바 ──

#[test]
fn dock_tab_seam_moves_the_tab_under_one_lock_and_notifies_after_it() {
    let w = World::new();
    let target = w.main_active();
    let (label, tabs) = w.popup_with(2);
    let source = tabs[1];
    let slot = w.empty_slot(source);
    let agent = Uuid::new_v4();
    apply::assign_agent(&w.state, &w.subs, &w.ev, source, slot, agent.to_string()).unwrap();
    let resyncs = w.resyncs();
    w.forget();

    let out = w
        .dock(
            source,
            DockDest::Seam {
                target,
                anchor: DockAnchor::Outer,
                side: DockSide::Left,
                share: DOCK_NEW_SHARE,
            },
            FocusPolicy::Reveal,
        )
        .unwrap();

    assert_eq!(
        out,
        DockOutcome {
            window: MAIN_WINDOW_LABEL.to_string(),
            slot_ids: vec![slot],
            emptied: None,
            main_refilled: None,
            reused: false,
            prewarmed: false,
        }
    );
    assert_eq!(w.resyncs(), resyncs + 1, "재동기 한 번 — 락 안(프로브)");
    assert_eq!(
        w.subs.router.targets(agent).to_vec(),
        [MAIN_WINDOW_LABEL.to_string()],
        "옮긴 에이전트의 출력은 대상 창으로"
    );
    assert!(w.unsubscribed().is_empty(), "구독은 끊기지 않는다");
    assert!(w.slots(target).contains(&slot), "대상 탭에 끼웠다");
    assert_eq!(w.owner(source), None, "출처 View 는 지워진다");
    assert_eq!(
        w.notified_views(),
        [target, tabs[0]],
        "대상 배치 + 출처 창의 새 활성 배치"
    );
    assert_eq!(w.notified_windows(), [MAIN_WINDOW_LABEL, label.as_str()]);
    assert_eq!(w.notified_tabs_of(&label).active, tabs[0]);
    w.assert_notified_at_current_version();
    assert_notified(&w, "dock_tab(seam)");
    assert!(w.host.closed.lock().unwrap().is_empty());
    assert_invariants(&w);
}

#[test]
fn dock_tab_tab_bar_moves_the_tab_and_notifies_both_windows() {
    let w = World::new();
    let main_tab = w.main_active();
    let (label, tabs) = w.popup_with(2);
    let source = tabs[1];
    let resyncs = w.resyncs();
    let spare = w.prewarm();
    let asked = w.claims_asked();
    w.forget();

    let out = w
        .dock(
            source,
            DockDest::TabBar {
                window: MAIN_WINDOW_LABEL.to_string(),
                index: Some(0),
            },
            FocusPolicy::Reveal,
        )
        .unwrap();

    w.assert_spare_not_asked(&spare, asked);
    assert!(!out.prewarmed);
    assert_eq!(out.window, MAIN_WINDOW_LABEL);
    assert_eq!(out.emptied, None);
    assert_eq!(w.resyncs(), resyncs + 1);
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [source, main_tab]);
    assert_eq!(w.owner(source).as_deref(), Some(MAIN_WINDOW_LABEL));
    let main = w.notified_tabs_of(MAIN_WINDOW_LABEL);
    assert_eq!(main.active, source, "Reveal — 옮긴 탭이 보인다");
    assert_eq!(
        main.tabs.iter().map(|m| m.id).collect::<Vec<_>>(),
        [source, main_tab]
    );
    assert_eq!(w.notified_tabs_of(&label).active, tabs[0]);
    assert_eq!(w.notified_views(), [source, tabs[0]]);
    w.assert_notified_at_current_version();
    assert_notified(&w, "dock_tab(tab_bar)");
    assert_invariants(&w);
}

#[test]
fn dock_tab_closes_an_emptied_source_popout_once_after_its_notifications() {
    let w = World::new();
    let target = w.main_active();
    let (label, tabs) = w.popup_with(1);
    let source = tabs[0];
    w.forget();
    let events = ClosesAfter {
        inner: &w.ev,
        host: &w.host,
    };

    let out = w
        .dock_via(
            &events,
            source,
            DockDest::Seam {
                target,
                anchor: DockAnchor::Outer,
                side: DockSide::Right,
                share: DOCK_NEW_SHARE,
            },
            FocusPolicy::Reveal,
        )
        .unwrap();

    assert_eq!(
        out.emptied,
        Some(label.clone()),
        "닫아도 지운 항목은 돌려준다"
    );
    assert_eq!(&*w.host.closed.lock().unwrap(), &[label.clone()], "한 번");
    assert!(!w.window_exists(&label), "모델에서 먼저 지웠다");
    assert_eq!(
        w.notified_windows(),
        [MAIN_WINDOW_LABEL],
        "지운 창의 탭 목록은 없다"
    );
    assert_notified(&w, "dock_tab(빈 출처)");
    assert_invariants(&w);
}

#[test]
fn dock_tab_refusals_change_nothing_and_notify_nothing() {
    let w = World::new();
    let (_, tabs) = w.popup_with(2);
    let source = tabs[1];
    let bogus = Uuid::new_v4();
    let cases = [
        (
            source,
            DockDest::Seam {
                target: source,
                anchor: DockAnchor::Outer,
                side: DockSide::Top,
                share: DOCK_NEW_SHARE,
            },
            DockError::SelfTarget,
        ),
        (
            source,
            DockDest::TabBar {
                window: "no-such".into(),
                index: None,
            },
            DockError::WindowNotFound("no-such".into()),
        ),
        (
            bogus,
            DockDest::NewWindow { at: AT },
            DockError::TabNotFound(bogus),
        ),
    ];
    let version = w.version();
    let resyncs = w.resyncs();
    w.forget();

    for (view, to, want) in cases {
        assert_eq!(w.dock(view, to, FocusPolicy::Reveal), Err(want));
    }

    assert_eq!(w.version(), version);
    assert_eq!(w.resyncs(), resyncs, "실패 = 재동기 없음");
    assert_eq!(w.ev.take_order(), Vec::<&str>::new(), "알림 0");
    assert!(
        w.host.opened_at.lock().unwrap().is_empty(),
        "새 창도 안 연다"
    );
    assert!(w.host.closed.lock().unwrap().is_empty());
    assert_invariants(&w);
}

#[test]
fn dock_tab_onto_the_same_place_is_ok_and_silent() {
    let w = World::new();
    let (label, tabs) = w.popup_with(2);
    let version = w.version();
    let resyncs = w.resyncs();
    w.forget();

    let out = w
        .dock(
            tabs[1],
            DockDest::TabBar {
                window: label.clone(),
                index: Some(1),
            },
            FocusPolicy::Keep,
        )
        .unwrap();

    assert_eq!(out.window, label);
    assert_eq!(w.version(), version);
    assert_eq!(w.resyncs(), resyncs);
    assert_eq!(
        w.ev.take_order(),
        Vec::<&str>::new(),
        "무변경 결말은 알리지 않는다"
    );
}

#[test]
fn dock_tab_of_mains_last_tab_announces_the_fresh_empty_tab() {
    // 탭바(다른 창) · 새 창 — 둘 다 main 에 빈 탭이 생기고 그 id 를 돌려주며 그 배치 · 탭 목록이 알림에 실린다.
    for mode in ["tab_bar", "new_window"] {
        let w = World::new();
        let only = w.main_active();
        let (label, _) = w.popup_with(1);
        w.forget();
        let to = match mode {
            "tab_bar" => DockDest::TabBar {
                window: label.clone(),
                index: None,
            },
            _ => DockDest::NewWindow { at: AT },
        };

        let out = w.dock(only, to, FocusPolicy::Keep).unwrap();

        let main = w.tabs_of(MAIN_WINDOW_LABEL);
        assert_eq!(main.len(), 1, "{mode}: main 에 빈 탭 하나");
        let fresh = main[0];
        assert_ne!(fresh, only);
        assert_eq!(out.main_refilled, Some(fresh), "{mode}");
        let payload = w.notified_tabs_of(MAIN_WINDOW_LABEL);
        assert_eq!(payload.active, fresh, "빈 탭이 main 의 활성");
        assert_eq!(
            payload.tabs.iter().map(|m| m.id).collect::<Vec<_>>(),
            [fresh]
        );
        assert!(
            w.notified_views().contains(&fresh),
            "빈 탭의 배치도 싣는다 — 탭 id 만 받으면 그릴 배치가 없다"
        );
        w.assert_notified_at_current_version();
        assert_notified(&w, "dock_tab(main 마지막 탭)");
        assert_invariants(&w);
    }
}

// ── dock_tab: 다음 활성 · 붙는 몫(ADR-0295) ──

impl World {
    fn dock_req(&self, req: DockRequest) -> Result<DockOutcome, DockError> {
        apply::dock_tab(
            &self.state,
            &self.subs,
            &self.ev,
            &self.host,
            &self.labels,
            req,
        )
    }
}

#[test]
fn dock_tab_hands_the_source_window_to_the_given_successor() {
    // 팝아웃 [t0, t1, t2] · 활성 t1 — 승계라면 오른쪽 이웃 t2 다.
    for (pick, want) in [(Some(0), 0), (None, 2)] {
        let w = World::new();
        let (label, tabs) = w.popup_with(3);
        apply::switch_tab(&w.state, &w.subs, &w.ev, &label, tabs[1]).unwrap();
        w.forget();

        w.dock_req(DockRequest {
            view: tabs[1],
            to: DockDest::TabBar {
                window: MAIN_WINDOW_LABEL.into(),
                index: None,
            },
            focus: FocusPolicy::Reveal,
            mv: DockMove {
                successor: pick.map(|i: usize| tabs[i]),
                ..DockMove::default()
            },
        })
        .unwrap();

        assert_eq!(w.notified_tabs_of(&label).active, tabs[want], "{pick:?}");
        assert!(
            w.notified_views().contains(&tabs[want]),
            "{pick:?}: 새 활성의 배치도 싣는다"
        );
        assert_notified(&w, "dock_tab(다음 활성)");
        assert_invariants(&w);
    }
}

#[test]
fn dock_tab_seam_wraps_the_target_with_the_requested_share() {
    let w = World::new();
    let target = w.main_active();
    let (_, tabs) = w.popup_with(2);

    w.dock(
        tabs[1],
        DockDest::Seam {
            target,
            anchor: DockAnchor::Outer,
            side: DockSide::Right,
            share: 0.25,
        },
        FocusPolicy::Keep,
    )
    .unwrap();

    match w.layout_of(target) {
        LayoutNode::Split { ratio, .. } => assert_eq!(ratio, 0.75, "새것 몫 1/4 · 오른쪽"),
        other => panic!("새 분할이어야: {other:?}"),
    }
}

#[test]
fn dock_tab_seam_with_a_share_outside_its_range_changes_nothing() {
    let w = World::new();
    let target = w.main_active();
    let (_, tabs) = w.popup_with(2);
    let version = w.version();
    let resyncs = w.resyncs();
    w.forget();

    let out = w.dock(
        tabs[1],
        DockDest::Seam {
            target,
            anchor: DockAnchor::Outer,
            side: DockSide::Left,
            share: f64::NAN,
        },
        FocusPolicy::Reveal,
    );

    assert_eq!(out, Err(DockError::InvalidShare("NaN".into())));
    assert_eq!(w.version(), version);
    assert_eq!(w.resyncs(), resyncs);
    assert_eq!(w.ev.take_order(), Vec::<&str>::new());
    assert_invariants(&w);
}

// ── dock_tab: 새 창 ──

#[test]
fn dock_tab_new_window_opens_then_attaches_and_records_the_placement() {
    let w = World::new();
    let first = w.main_active();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let resyncs = w.resyncs();
    w.forget();

    let out = w
        .dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal)
        .unwrap();

    let label = out.window.clone();
    assert!(!out.reused);
    assert!(!out.prewarmed);
    assert_eq!(out.emptied, None);
    assert_eq!(
        &*w.host.opened_at.lock().unwrap(),
        &[(label.clone(), AT, true)],
        "main 출처 = 크기 없음(팝아웃 기본)"
    );
    assert_eq!(&*w.host.placed.lock().unwrap(), &[label.clone()]);
    assert!(w.host.closed.lock().unwrap().is_empty());
    assert_eq!(w.tabs_of(&label), [source]);
    assert_eq!(w.owner(source).as_deref(), Some(label.as_str()));
    assert_eq!(w.resyncs(), resyncs + 1);
    assert_eq!(w.notified_views(), [source, first]);
    assert_eq!(w.notified_windows(), [label.as_str(), MAIN_WINDOW_LABEL]);
    // TRD S21-slot-docking §8-1 ⑬ — `open_at` → `dock` → 알림 → `record_placement`.
    assert_eq!(
        w.journal.take(),
        [
            format!("open_at:{label}"),
            "layout".to_string(),
            "layout".to_string(),
            format!("tabs:{label}"),
            format!("tabs:{MAIN_WINDOW_LABEL}"),
            format!("place:{label}"),
        ]
    );
    w.assert_notified_at_current_version();
    assert_notified(&w, "dock_tab(새 창)");
    assert_invariants(&w);
}

#[test]
fn dock_tab_new_window_activates_only_when_revealing() {
    let w = World::new();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    w.forget();

    let out = w
        .dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Keep)
        .unwrap();

    assert_eq!(
        &*w.host.opened_at.lock().unwrap(),
        &[(out.window, AT, false)],
        "Keep = 포커스를 가져가지 않는다"
    );
    assert_eq!(
        w.host.claims_asked.load(Ordering::Relaxed),
        1,
        "Keep 이어도 예열 창을 묻는다(TRD §3-9 I4)"
    );
    assert!(!out.prewarmed);
}

#[test]
fn dock_tab_new_window_takes_the_source_popouts_normal_size_unless_given_one() {
    let w = World::new();
    let (label, tabs) = w.popup_with(3);
    w.observe(&label, false, bounds(50.0, 60.0, 640.0, 480.0));
    w.forget();

    w.dock(tabs[2], DockDest::NewWindow { at: AT }, FocusPolicy::Reveal)
        .unwrap();
    let given = WindowPlace {
        pos: AT.pos,
        size: Some((400.0, 300.0)),
    };
    w.dock(
        tabs[1],
        DockDest::NewWindow { at: given },
        FocusPolicy::Reveal,
    )
    .unwrap();

    let sizes: Vec<_> = w
        .host
        .opened_at
        .lock()
        .unwrap()
        .iter()
        .map(|(_, at, _)| (at.pos, at.size))
        .collect();
    assert_eq!(
        sizes,
        [
            (AT.pos, Some((640.0, 480.0))),
            (AT.pos, Some((400.0, 300.0)))
        ]
    );
}

#[test]
fn dock_tab_new_window_open_failure_changes_nothing() {
    let w = World::failing_host();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let version = w.version();
    let resyncs = w.resyncs();
    w.forget();

    let out = w.dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal);

    assert!(matches!(out, Err(DockError::WindowOpen(_))), "{out:?}");
    assert_eq!(w.version(), version);
    assert_eq!(w.resyncs(), resyncs);
    assert_eq!(w.owner(source).as_deref(), Some(MAIN_WINDOW_LABEL));
    assert_eq!(w.ev.take_order(), Vec::<&str>::new());
    assert!(
        w.host.closed.lock().unwrap().is_empty(),
        "못 연 창은 닫을 것도 없다"
    );
    assert!(w.host.placed.lock().unwrap().is_empty());
    assert_invariants(&w);
}

#[test]
fn dock_tab_new_window_closes_the_opened_window_when_phase_c_refuses() {
    let w = World::new();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    w.forget();
    // 창을 여는 사이 다른 경로가 출처 탭을 닫는다.
    *w.host.during_open.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        state
            .0
            .lock()
            .unwrap()
            .close_tab(MAIN_WINDOW_LABEL, source)
            .unwrap();
    }));
    let resyncs = w.resyncs();

    let out = w.dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal);

    assert_eq!(out, Err(DockError::TabNotFound(source)));
    let opened: Vec<String> = w
        .host
        .opened_at
        .lock()
        .unwrap()
        .iter()
        .map(|(label, _, _)| label.clone())
        .collect();
    assert_eq!(opened.len(), 1);
    assert_eq!(
        &*w.host.closed.lock().unwrap(),
        &opened,
        "방금 연 창을 거둔다"
    );
    assert!(!w.window_exists(&opened[0]));
    assert_eq!(w.owner(source), None, "닫힌 탭은 새 창에 붙지 않는다");
    assert!(w.host.placed.lock().unwrap().is_empty());
    assert_eq!(w.resyncs(), resyncs);
    assert_eq!(w.ev.take_order(), Vec::<&str>::new());
    assert_invariants(&w);
}

// ── dock_tab: 새 창 — 예열 창(TRD S21-slot-docking §8-1 ⑬ · ADR-0299) ──

// 예열 차례 `claim_spare` → `open_at` → `dock` → 알림 → `committed` → `record_placement`.
#[test]
fn dock_tab_new_window_opens_the_prewarmed_window_and_commits_after_the_notifications() {
    let w = World::new();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let spare = w.prewarm();
    w.forget();

    let out = w
        .dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal)
        .unwrap();

    assert_eq!(out.window, spare, "맡긴 창의 label 로 연다");
    assert!(out.prewarmed && !out.reused);
    assert_eq!(
        &*w.host.opened_at.lock().unwrap(),
        &[(spare.clone(), AT, true)]
    );
    assert_eq!(
        w.journal.take(),
        [
            format!("claim:{spare}"),
            format!("open_at:{spare}"),
            "layout".to_string(),
            "layout".to_string(),
            format!("tabs:{spare}"),
            format!("tabs:{MAIN_WINDOW_LABEL}"),
            format!("committed:{spare}"),
            format!("place:{spare}"),
        ]
    );
    assert_eq!(w.tabs_of(&spare), [source]);
    assert!(!w.claim_outstanding());
    assert_invariants(&w);
}

// `Keep` 의 새 창도 예열 창을 쓴다 — 활성 안 함은 맡긴 창의 `open_at` 이 받는다(TRD §3-9 I4).
#[test]
fn dock_tab_new_window_uses_the_prewarmed_window_without_focus_too() {
    let w = World::new();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let spare = w.prewarm();
    w.forget();

    let out = w
        .dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Keep)
        .unwrap();

    assert!(out.prewarmed);
    assert_eq!(&*w.host.opened_at.lock().unwrap(), &[(spare, AT, false)]);
}

// 맡긴 창의 `open_at` 이 `Err` 면 창 포트가 그 창을 거뒀다 — `WindowOpen` 이고 확정 · 되물림은 없다.
#[test]
fn dock_tab_new_window_whose_prewarmed_window_fails_to_open_changes_nothing() {
    let w = World::failing_host();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let version = w.version();
    let spare = w.prewarm();
    w.forget();

    let out = w.dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal);

    assert!(matches!(out, Err(DockError::WindowOpen(_))), "{out:?}");
    assert_eq!(
        w.journal.take(),
        [format!("claim:{spare}"), format!("open_at:{spare}")]
    );
    assert_eq!(w.version(), version);
    assert!(!w.claim_outstanding(), "창 포트가 거뒀다");
}

// 붙이기 전 판정이 바뀌면(여는 사이 출처 탭이 닫힘) 맡긴 창은 닫지 않고 되물린다.
#[test]
fn dock_tab_new_window_refused_at_the_attach_gives_the_prewarmed_window_back() {
    let w = World::new();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let spare = w.prewarm();
    w.forget();
    *w.host.during_open.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        state
            .0
            .lock()
            .unwrap()
            .close_tab(MAIN_WINDOW_LABEL, source)
            .unwrap();
    }));

    let out = w.dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal);

    assert_eq!(out, Err(DockError::TabNotFound(source)));
    assert_eq!(
        w.journal.take(),
        [
            format!("claim:{spare}"),
            format!("open_at:{spare}"),
            format!("release:{spare}"),
        ],
        "닫기 · 알림 · 자리 기록 없음"
    );
    assert_eq!(w.owner(source), None);
    assert_invariants(&w);
}

// 알리는 사이(첫 탭 알림) · 확정 전에 그 창이 모델에서 빠져도 확정은 맡김표를 그대로 넘긴다(TRD §3-9 · I27 · I31).
#[test]
fn dock_tab_new_window_commits_the_claim_even_if_the_window_left_the_model_before_it() {
    let w = World::new();
    let source = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let spare = w.prewarm();
    let gone = spare.clone();
    *w.ev.on_tabs.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
        state.0.lock().unwrap().close_window(&gone).unwrap();
    }));
    w.host.vanished.lock().unwrap().push(spare.clone());
    w.forget();

    let out = w
        .dock(source, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal)
        .unwrap();

    assert!(out.prewarmed);
    let journal = w.journal.take();
    assert_eq!(
        journal[journal.len() - 2..],
        [format!("committed:{spare}"), format!("place:{spare}")]
    );
    assert!(!w.window_exists(&spare));
    assert!(
        w.host.closed.lock().unwrap().is_empty(),
        "적용 서비스는 닫지 않는다"
    );
}

// ── dock_tab: 재사용 ──

impl World {
    // 탭 하나뿐인 팝아웃 — 새 창이 아니라 그 창을 옮긴다(`plan_new_window`). 보통 자리도 적어 둔다. 반환 = (label, 탭).
    fn lone_popout(&self) -> (String, Uuid) {
        let (label, tabs) = self.popup_with(1);
        self.observe(&label, false, bounds(40.0, 30.0, 800.0, 600.0));
        (label, tabs[0])
    }
}

#[test]
fn dock_tab_reuse_moves_the_source_window_once_and_keeps_the_model() {
    let w = World::new();
    let (label, view) = w.lone_popout();
    let slot = w.empty_slot(view);
    let version = w.version();
    let resyncs = w.resyncs();
    let spare = w.prewarm();
    let asked = w.claims_asked();
    w.forget();

    let out = w
        .dock(view, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal)
        .unwrap();

    w.assert_spare_not_asked(&spare, asked);
    assert_eq!(
        out,
        DockOutcome {
            window: label.clone(),
            slot_ids: vec![slot],
            emptied: None,
            main_refilled: None,
            reused: true,
            prewarmed: false,
        }
    );
    assert_eq!(&*w.host.moves.lock().unwrap(), &[(label.clone(), AT)]);
    assert!(w.host.opened_at.lock().unwrap().is_empty(), "새 창 없음");
    assert_eq!(&*w.host.placed.lock().unwrap(), &[label.clone()], "한 번");
    assert_eq!(w.version(), version, "모델 변경 0");
    assert_eq!(w.resyncs(), resyncs);
    assert_eq!(w.ev.take_order(), Vec::<&str>::new(), "알릴 것이 없다");
    assert_eq!(w.owner(view).as_deref(), Some(label.as_str()));
    assert_invariants(&w);
}

#[test]
fn dock_tab_a_lone_popout_without_a_normal_place_is_reused_too() {
    let w = World::new();
    let (label, tabs) = w.popup_with(1);
    let view = tabs[0];
    assert_eq!(
        w.state
            .0
            .lock()
            .unwrap()
            .window_attrs(&label)
            .unwrap()
            .bounds,
        None,
        "전제: 보통 자리를 아직 못 봤다"
    );
    let version = w.version();
    w.forget();

    let out = w
        .dock(view, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal)
        .unwrap();

    assert!(out.reused);
    assert_eq!(out.window, label);
    assert_eq!(out.emptied, None);
    assert_eq!(&*w.host.moves.lock().unwrap(), &[(label.clone(), AT)]);
    assert!(w.host.opened_at.lock().unwrap().is_empty(), "새 창 없음");
    assert!(w.host.closed.lock().unwrap().is_empty());
    assert_eq!(&*w.host.placed.lock().unwrap(), &[label.clone()]);
    assert_eq!(w.version(), version);
    assert_eq!(w.ev.take_order(), Vec::<&str>::new());
    assert_invariants(&w);
}

#[test]
fn dock_tab_reuse_finishes_the_drop_even_if_the_source_changes_during_the_move() {
    // 사용자 결정(2026-10-16) — 끄기가 서면 놓은 자리에 둔다. 옮기는 사이의 쓰기는 다시 판정하지 않고 되돌리지도 않는다:
    // 그 쓰기가 남긴 모델 그대로 끝난다. (무엇, 끼어든 쓰기, 그 창이 모델에 남나)
    type Meddle = fn(&mut ViewManager, &str, Uuid, Uuid);
    let cases: [(&str, Meddle, bool); 3] = [
        (
            "출처 탭에 슬롯 더함",
            |mgr, _, view, slot| {
                mgr.split_slot(view, slot, SplitDir::TopBottom).unwrap();
            },
            true,
        ),
        (
            "그 팝아웃에 탭 더함",
            |mgr, label, _, _| {
                mgr.create_tab(label, None).unwrap();
            },
            true,
        ),
        (
            "출처 탭 닫음(창째 사라짐)",
            |mgr, label, view, _| {
                mgr.close_tab(label, view).unwrap();
            },
            false,
        ),
    ];
    for (what, meddle, kept) in cases {
        let w = World::new();
        let (label, view) = w.lone_popout();
        let slot = w.empty_slot(view);
        w.forget();
        let hooked = label.clone();
        *w.host.during_move.lock().unwrap() = Some(Box::new(move |state: &LayoutState| {
            meddle(&mut state.0.lock().unwrap(), &hooked, view, slot);
        }));

        let out = w
            .dock(view, DockDest::NewWindow { at: AT }, FocusPolicy::Reveal)
            .unwrap_or_else(|e| panic!("{what}: {e}"));

        assert!(out.reused, "{what}");
        assert_eq!(out.window, label, "{what}");
        assert_eq!(w.host.moves.lock().unwrap().len(), 1, "{what}");
        assert!(w.host.opened_at.lock().unwrap().is_empty(), "{what}");
        assert!(
            w.host.closed.lock().unwrap().is_empty(),
            "{what}: 거두지 않는다"
        );
        assert_eq!(w.window_exists(&label), kept, "{what}");
        let placed = w.host.placed.lock().unwrap().clone();
        assert_eq!(
            placed,
            if kept { vec![label.clone()] } else { vec![] },
            "{what}: 모델에 남은 창만 자리를 적는다"
        );
        assert_eq!(w.ev.take_order(), Vec::<&str>::new(), "{what}");
        assert_invariants(&w);
    }
}

#[test]
fn dock_tab_reuse_move_failure_is_window_open_and_undoes_nothing() {
    let at = WindowPlace {
        pos: AT.pos,
        size: Some((500.0, 400.0)),
    };
    for (failing, called) in [
        (MoveStep::Position, vec![MoveStep::Position]),
        (MoveStep::Size, vec![MoveStep::Position, MoveStep::Size]),
    ] {
        let w = World::new();
        let (label, view) = w.lone_popout();
        let version = w.version();
        w.forget();
        w.host.fail_step(failing);
        let captured = Captured::default();

        let out = tracing::subscriber::with_default(captured.clone(), || {
            w.dock(view, DockDest::NewWindow { at }, FocusPolicy::Reveal)
        });

        assert!(
            matches!(out, Err(DockError::WindowOpen(_))),
            "{failing:?}: {out:?}"
        );
        let lines = captured.0.lock().unwrap().clone();
        let ends: Vec<&Line> = lines
            .iter()
            .filter(|l| l.message.contains("출처 창을 못 옮겼다"))
            .collect();
        assert_eq!(ends.len(), 1, "{failing:?}: 끝 줄 하나 — {lines:?}");
        assert_eq!(ends[0].level, tracing::Level::WARN, "{failing:?}");
        let step = format!("{failing:?}");
        assert_eq!(ends[0].field("step"), Some(step.as_str()));
        assert_eq!(ends[0].field("label"), Some(label.as_str()));
        assert_eq!(w.host.steps(), called, "{failing:?}: 첫 실패에서 멈춘다");
        assert_eq!(
            w.host.moves.lock().unwrap().len(),
            1,
            "{failing:?}: 다시 옮기지 않는다"
        );
        assert!(w.host.placed.lock().unwrap().is_empty());
        assert!(w.host.closed.lock().unwrap().is_empty());
        assert_eq!(w.version(), version);
        assert_eq!(w.owner(view).as_deref(), Some(label.as_str()));
        assert_invariants(&w);
    }
}

// ── dock_tab: 끝 로그 ──

#[derive(Debug, Clone)]
struct Line {
    level: tracing::Level,
    message: String,
    fields: Vec<(String, String)>,
}

/// 이 스레드에서 낸 이벤트를 전부 모은다(`settings` 시험의 같은 이름 가짜와 같은 꼴). 만들 때 [`ASK_ALWAYS`] 를 세운다.
#[derive(Clone)]
struct Captured(std::sync::Arc<Mutex<Vec<Line>>>);

impl Default for Captured {
    fn default() -> Self {
        LazyLock::force(&ASK_ALWAYS);
        Self(std::sync::Arc::default())
    }
}

// tracing-core 0.1.36: 마지막 구독자 등록 때 살아 있던 구독자가 하나뿐이었으면 callsite 의 첫 등록이 **그 스레드의** 기본 구독자에게만 관심을 묻는다
//   (`Dispatchers::rebuilder` → `JustOne`). capture 밖 시험 스레드가 「도킹 적용」 같은 공용 자리를 capture 도중 처음 때리면
//   「관심 없음」이 전역에 박혀 그 capture 가 빈손이 된다(실측 — 6 병렬 120 회 중 38 회). 이것을 바이너리 내내 붙들어 두면 첫
//   등록이 살아 있는 구독자 전부에게 묻고, 이것이 늘 「때마다 묻기」라 「관심 없음」이 박히지 않는다. 남는 틈 = 이것을 처음 세우는
//   그 순간의 같은 경주 하나(미검 — 새면 증상은 위와 같다).
static ASK_ALWAYS: LazyLock<tracing::Dispatch> =
    LazyLock::new(|| tracing::Dispatch::new(AskAlways));

/// 아무것도 받지 않고 모든 callsite 에 「때마다 묻기」만 답한다 — [`ASK_ALWAYS`].
struct AskAlways;

impl tracing::Subscriber for AskAlways {
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        false
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, _: &tracing::Event<'_>) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

impl Line {
    fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }
}

struct Fields<'a>(&'a mut Line);

impl tracing::field::Visit for Fields<'_> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0
            .fields
            .push((field.name().to_string(), value.to_string()));
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let text = format!("{value:?}");
        if field.name() == "message" {
            self.0.message = text;
        } else {
            self.0.fields.push((field.name().to_string(), text));
        }
    }
}

impl tracing::Subscriber for Captured {
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::sometimes()
    }
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut line = Line {
            level: *event.metadata().level(),
            message: String::new(),
            fields: Vec::new(),
        };
        event.record(&mut Fields(&mut line));
        self.0.lock().unwrap().push(line);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[test]
fn dock_tab_onto_the_same_place_logs_that_nothing_changed() {
    let w = World::new();
    let (label, tabs) = w.popup_with(2);
    let captured = Captured::default();

    tracing::subscriber::with_default(captured.clone(), || {
        w.dock(
            tabs[1],
            DockDest::TabBar {
                window: label.clone(),
                index: Some(1),
            },
            FocusPolicy::Keep,
        )
        .unwrap();
        w.dock(
            tabs[1],
            DockDest::TabBar {
                window: label.clone(),
                index: Some(0),
            },
            FocusPolicy::Keep,
        )
        .unwrap();
    });

    let lines = captured.0.lock().unwrap().clone();
    let applied: Vec<(tracing::Level, Option<&str>)> = lines
        .iter()
        .filter(|l| l.message.contains("도킹 적용"))
        .map(|l| (l.level, l.field("changed")))
        .collect();
    assert_eq!(
        applied,
        [
            (tracing::Level::INFO, Some("false")),
            (tracing::Level::INFO, None)
        ],
        "무변경 결말만 그것을 적는다"
    );
}

// ── 놓기 명령 — 놓을 때 대상을 다시 찾는다(ADR-0297 · TRD S21-slot-docking §3-3 · §8-1 ⑦) ─────────────────────
//
// 놓기 명령(`tab_drop`)은 웹 스토어로 판정한 것을 싣는다 — 그 스토어가 모델보다 늦어도 놓기가 가리킨 것(끄는 탭 · 대상
// 탭 · 대상 칸 · 탭바 창)이 지금 있으면 같은 `dock_tab` 이 지금 상태에 맞춰 적용하고, 없으면 거절한다. 요청은 운영과
// 같은 순수 판정(`tab_drop::plan::plan`)으로 짓는다.

const FLAT_FRAME: SourceFrame = SourceFrame {
    insets: Insets {
        t: 0.0,
        r: 0.0,
        b: 0.0,
        l: 0.0,
    },
    scale: 1.0,
};

fn dropped_at(x: i32, y: i32) -> InputNow {
    InputNow {
        cursor: Some(PhysPoint { x, y }),
        primary_down: false,
        escape_down: false,
        top: TopAt::Unknown,
        scale: 1.0,
    }
}

impl World {
    fn active_of(&self, label: &str) -> Uuid {
        apply::list_tabs(&self.state, label).unwrap().active
    }

    // 놓기 명령 하나를 판정해 적용한다 — 운영의 `tab_drop` 과 같은 길(입력은 우리 웹뷰의 drop).
    fn tab_drop(&self, drop: &TabDrop, caller: &str) -> Result<DockOutcome, DockError> {
        match drop_plan::plan(drop, caller, &dropped_at(500, 400), &FLAT_FRAME) {
            Plan::Dock(req) => self.dock_req(req),
            other => panic!("도킹 요청이어야: {other:?}"),
        }
    }

    fn tab_bar_drop(&self, view: Uuid, from: &str, initial: Option<Uuid>, index: u32) -> TabDrop {
        TabDrop {
            drag: Uuid::new_v4(),
            view,
            from: from.into(),
            initial,
            to: DropTo::TabBar { index },
        }
    }

    // 이음새 놓기 — `target` 의 `anchor` 오른쪽 반.
    fn seam_drop(
        &self,
        view: Uuid,
        from: &str,
        initial: Option<Uuid>,
        target: Uuid,
        anchor: DropAnchor,
    ) -> TabDrop {
        TabDrop {
            drag: Uuid::new_v4(),
            view,
            from: from.into(),
            initial,
            to: DropTo::Seam {
                target,
                anchor,
                side: DockSide::Right,
                share: 0.5,
            },
        }
    }

    fn new_window_drop(&self, view: Uuid, from: &str) -> TabDrop {
        TabDrop {
            drag: Uuid::new_v4(),
            view,
            from: from.into(),
            initial: None,
            to: DropTo::NewWindow {
                by: NewWindowBy::Drop,
                size: None,
                grab: PointCss { x: 0.0, y: 0.0 },
            },
        }
    }

    fn assert_refused(&self, got: Result<DockOutcome, DockError>, want: DockError, what: &str) {
        let version = self.version();
        assert_eq!(got, Err(want), "{what}");
        assert_eq!(self.version(), version, "{what}: 모델 그대로");
    }
}

// main = [m0, t] · 팝아웃 = [p] — `t` 를 main 에서 끌기 시작한 뒤 LLM 이 `t` 를 팝아웃으로 옮겼다(D9).
#[test]
fn a_drop_whose_tab_left_its_source_window_moves_it_from_where_it_is_now() {
    let setup = || {
        let w = World::new();
        let m0 = w.main_active();
        let t = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
        let popout = w.popup();
        let p = w.active_of(&popout);
        w.dock(
            t,
            DockDest::TabBar {
                window: popout.clone(),
                index: None,
            },
            FocusPolicy::Keep,
        )
        .unwrap();
        w.forget();
        (w, m0, t, popout, p)
    };

    let (w, m0, t, popout, p) = setup();
    let out = w
        .tab_drop(
            &w.tab_bar_drop(t, MAIN_WINDOW_LABEL, Some(m0), 0),
            MAIN_WINDOW_LABEL,
        )
        .expect("탭바");
    assert_eq!(out.window, MAIN_WINDOW_LABEL);
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [t, m0]);
    assert_eq!(w.tabs_of(&popout), [p], "지금 창(팝아웃)에서 뺐다");
    assert_invariants(&w);

    let (w, _, t, popout, p) = setup();
    let out = w
        .tab_drop(
            &w.seam_drop(t, MAIN_WINDOW_LABEL, None, p, DropAnchor::Outer),
            &popout,
        )
        .expect("이음새");
    assert_eq!(out.window, popout);
    assert_eq!(w.tabs_of(&popout), [p]);
    assert_eq!(w.owner(t), None, "t 는 p 에 붙어 사라졌다");
    assert_invariants(&w);

    let (w, _, t, popout, p) = setup();
    let out = w
        .tab_drop(&w.new_window_drop(t, MAIN_WINDOW_LABEL), &popout)
        .expect("새 창");
    assert!(!out.reused, "팝아웃에 탭이 둘이라 새 창");
    assert_eq!(w.owner(t).as_deref(), Some(out.window.as_str()));
    assert_eq!(w.tabs_of(&popout), [p]);
    assert_eq!(w.host.opened_at.lock().unwrap().len(), 1);
    assert_invariants(&w);
}

// `initial` 은 끌기를 시작한 창(main)의 선택이다 — 끄는 탭이 그사이 간 창(팝아웃)은 `initial` 이 거기 있어도 셸 승계를 따른다.
#[test]
fn a_drop_whose_tab_left_its_source_window_leaves_that_window_to_the_shell_succession() {
    let w = World::new();
    let i = w.main_active();
    let t = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let drop = w.tab_bar_drop(t, MAIN_WINDOW_LABEL, Some(i), 0);
    let popout = w.popup();
    let p = w.active_of(&popout);
    for (tab, index) in [(i, Some(0)), (t, None)] {
        w.dock(
            tab,
            DockDest::TabBar {
                window: popout.clone(),
                index,
            },
            FocusPolicy::Reveal,
        )
        .unwrap();
    }
    assert_eq!(w.tabs_of(&popout), [i, p, t], "전제");
    assert_eq!(w.active_of(&popout), t, "전제");
    w.tab_drop(&drop, MAIN_WINDOW_LABEL).expect("받는다");
    assert_eq!(w.tabs_of(&popout), [i, p]);
    assert_eq!(
        w.active_of(&popout),
        p,
        "승계(왼쪽 이웃) — initial 이 아니다"
    );
    assert_eq!(w.active_of(MAIN_WINDOW_LABEL), t);
    assert_invariants(&w);
}

#[test]
fn a_drop_whose_tab_is_gone_is_refused_before_opening_anything() {
    let w = World::new();
    let m0 = w.main_active();
    let t = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let drops = [
        w.tab_bar_drop(t, MAIN_WINDOW_LABEL, Some(m0), 0),
        w.seam_drop(t, MAIN_WINDOW_LABEL, Some(m0), m0, DropAnchor::Outer),
        w.new_window_drop(t, MAIN_WINDOW_LABEL),
    ];
    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, MAIN_WINDOW_LABEL, t).unwrap();
    w.forget();
    for drop in &drops {
        w.assert_refused(
            w.tab_drop(drop, MAIN_WINDOW_LABEL),
            DockError::TabNotFound(t),
            &format!("{:?}", drop.to),
        );
    }
    assert!(
        w.host.opened_at.lock().unwrap().is_empty(),
        "창을 열기 전에 거절"
    );
    assert!(w.host.moves.lock().unwrap().is_empty());
    assert_eq!(w.ev.take_order(), Vec::<&str>::new(), "알림 없음");
}

#[test]
fn a_tab_bar_drop_lands_in_the_tab_list_as_it_is_now_and_clamps_to_its_end() {
    // 판정 뒤 탭이 늘었다 — 자리는 지금 목록에서 센다.
    let w = World::new();
    let m0 = w.main_active();
    let a = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let (popout, tabs) = w.popup_with(2);
    let t = tabs[1];
    let drop = w.tab_bar_drop(t, &popout, Some(tabs[0]), 1);
    let c = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    w.tab_drop(&drop, MAIN_WINDOW_LABEL).expect("받는다");
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [m0, t, a, c]);
    assert_eq!(w.active_of(MAIN_WINDOW_LABEL), t, "놓은 탭이 보인다");
    assert_invariants(&w);

    // 판정 뒤 탭이 줄었다 — 넘친 자리는 지금 목록의 끝 자리다.
    let w = World::new();
    let m0 = w.main_active();
    let a = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let b = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let (popout, tabs) = w.popup_with(2);
    let t = tabs[1];
    let drop = w.tab_bar_drop(t, &popout, Some(tabs[0]), 3);
    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, MAIN_WINDOW_LABEL, b).unwrap();
    w.tab_drop(&drop, MAIN_WINDOW_LABEL).expect("받는다");
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [m0, a, t]);
    assert_eq!(w.tabs_of(&popout), [tabs[0]]);
    assert_invariants(&w);

    // 같은 창 — 끄는 탭을 뺀 지금 목록에서 센다.
    let w = World::new();
    let m0 = w.main_active();
    let a = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let t = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let b = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let drop = w.tab_bar_drop(t, MAIN_WINDOW_LABEL, Some(m0), 3);
    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, MAIN_WINDOW_LABEL, a).unwrap();
    w.tab_drop(&drop, MAIN_WINDOW_LABEL).expect("받는다");
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [m0, b, t]);
    assert_invariants(&w);
}

#[test]
fn a_tab_bar_drop_into_a_window_that_closed_is_refused() {
    let w = World::new();
    let t = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let popout = w.popup();
    let drop = w.tab_bar_drop(t, MAIN_WINDOW_LABEL, None, 0);
    apply::close_window(&w.state, &w.subs, &w.host, &popout).unwrap();
    w.forget();
    w.assert_refused(
        w.tab_drop(&drop, &popout),
        DockError::WindowNotFound(popout.clone()),
        "닫힌 창",
    );
    assert_eq!(w.owner(t).as_deref(), Some(MAIN_WINDOW_LABEL));
}

#[test]
fn a_seam_drop_lands_in_the_target_layout_as_it_is_now() {
    // 대상 = 팝아웃의 탭 q · 끄는 탭 = main 의 t.
    let setup = || {
        let w = World::new();
        let t = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
        let popout = w.popup();
        let q = w.active_of(&popout);
        let s = w.empty_slot(q);
        (w, t, popout, q, s)
    };

    // 기준 칸이 그사이 나뉘었다 — 그 칸이 남았으니 그 칸에 붙는다.
    let (w, t, popout, q, s) = setup();
    let moved = w.slots(t);
    let drop = w.seam_drop(t, MAIN_WINDOW_LABEL, None, q, DropAnchor::Slot { id: s });
    let n = apply::split_slot(&w.state, &w.subs, &w.ev, q, s, SplitDir::TopBottom).unwrap();
    apply::rename_tab(&w.state, &w.ev, q, "새 이름".into()).unwrap();
    let out = w.tab_drop(&drop, &popout).expect("받는다");
    assert_eq!(out.window, popout);
    let slots = w.slots(q);
    assert!(
        [s, n].iter().chain(&moved).all(|x| slots.contains(x)),
        "나뉜 두 칸 + 붙은 칸: {slots:?}"
    );
    assert_eq!(slots.len(), 2 + moved.len());
    assert_eq!(w.owner(t), None);
    assert_invariants(&w);

    // 바깥 변 — 그사이 나뉜 대상 탭 전체에 붙는다.
    let (w, t, popout, q, s) = setup();
    let drop = w.seam_drop(t, MAIN_WINDOW_LABEL, None, q, DropAnchor::Outer);
    apply::split_slot(&w.state, &w.subs, &w.ev, q, s, SplitDir::LeftRight).unwrap();
    assert!(w.tab_drop(&drop, &popout).is_ok());
    assert_invariants(&w);

    // 기준 칸이 그사이 사라졌다 — 거절.
    let (w, t, popout, q, s) = setup();
    let n = apply::split_slot(&w.state, &w.subs, &w.ev, q, s, SplitDir::TopBottom).unwrap();
    let drop = w.seam_drop(t, MAIN_WINDOW_LABEL, None, q, DropAnchor::Slot { id: n });
    apply::close_slot(&w.state, &w.subs, &w.ev, q, n).unwrap();
    w.assert_refused(
        w.tab_drop(&drop, &popout),
        DockError::SlotNotFound(n),
        "기준 칸 없음",
    );

    // 대상 탭이 그사이 사라졌다 — 거절.
    let (w, t, popout, q, _) = setup();
    let other = apply::create_tab(&w.state, &w.subs, &w.ev, &popout, None).unwrap();
    let drop = w.seam_drop(t, MAIN_WINDOW_LABEL, None, q, DropAnchor::Outer);
    apply::close_tab(&w.state, &w.subs, &w.ev, &w.host, &popout, q).unwrap();
    w.assert_refused(
        w.tab_drop(&drop, &popout),
        DockError::TabNotFound(q),
        "대상 탭 없음",
    );
    assert_eq!(w.tabs_of(&popout), [other]);
}

#[test]
fn a_seam_drop_onto_a_tab_that_moved_to_another_window_docks_where_that_tab_is_now() {
    let w = World::new();
    let t = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let q = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    let drop = w.seam_drop(t, MAIN_WINDOW_LABEL, None, q, DropAnchor::Outer);
    let popout = w.popup();
    w.dock(
        q,
        DockDest::TabBar {
            window: popout.clone(),
            index: None,
        },
        FocusPolicy::Keep,
    )
    .unwrap();
    let out = w.tab_drop(&drop, MAIN_WINDOW_LABEL).expect("받는다");
    assert_eq!(out.window, popout);
    assert_eq!(w.active_of(&popout), q, "대상 탭이 보인다");
    assert_eq!(w.owner(t), None);
    assert_invariants(&w);
}

// main = [i, d] — d 를 누르면(활성 = d) 끄는 동안 남은 탭 i 를 보이고, 사용자는 i 의 이음새에 놓는다(같은 창).
#[test]
fn a_same_window_seam_applies_whichever_tab_the_window_shows_now() {
    let setup = || {
        let w = World::new();
        let i = w.main_active();
        let d = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
        assert_eq!(w.active_of(MAIN_WINDOW_LABEL), d, "누른 탭이 활성");
        (w, i, d)
    };

    // 놓기가 먼저 닿음 — 활성 = d.
    let (w, i, d) = setup();
    let drop = w.seam_drop(d, MAIN_WINDOW_LABEL, Some(i), i, DropAnchor::Outer);
    assert!(w.tab_drop(&drop, MAIN_WINDOW_LABEL).is_ok());
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [i]);
    assert_invariants(&w);

    // 출처의 원래 선택 되돌리기(`switch_tab_if(initial)`)가 먼저 닿음 — 활성 = i.
    let (w, i, d) = setup();
    let drop = w.seam_drop(d, MAIN_WINDOW_LABEL, Some(i), i, DropAnchor::Outer);
    apply::switch_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, i).unwrap();
    let out = w.tab_drop(&drop, MAIN_WINDOW_LABEL).unwrap();
    assert_eq!(out.window, MAIN_WINDOW_LABEL);
    assert_eq!(
        w.tabs_of(MAIN_WINDOW_LABEL),
        [i],
        "d 가 i 의 이음새에 붙었다"
    );

    // LLM 이 셋째 탭을 보였다 — 그래도 붙고, 대상 탭이 보인다.
    let (w, i, d) = setup();
    let x = apply::create_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, None).unwrap();
    apply::switch_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, d).unwrap();
    let drop = w.seam_drop(d, MAIN_WINDOW_LABEL, Some(i), i, DropAnchor::Outer);
    apply::switch_tab(&w.state, &w.subs, &w.ev, MAIN_WINDOW_LABEL, x).unwrap();
    assert!(w.tab_drop(&drop, MAIN_WINDOW_LABEL).is_ok());
    assert_eq!(w.tabs_of(MAIN_WINDOW_LABEL), [i, x]);
    assert_eq!(w.active_of(MAIN_WINDOW_LABEL), i);
    assert_invariants(&w);
}

#[test]
fn a_lone_popout_new_window_drop_reuses_the_window_and_keeps_its_size() {
    let w = World::new();
    let (label, view) = w.lone_popout();
    let version = w.version();
    w.forget();
    let drop = TabDrop {
        drag: Uuid::new_v4(),
        view,
        from: label.clone(),
        initial: Some(view),
        to: DropTo::NewWindow {
            by: NewWindowBy::Drop,
            size: None,
            grab: PointCss { x: 30.0, y: 12.0 },
        },
    };
    let out = w.tab_drop(&drop, &label).unwrap();
    assert!(out.reused);
    assert_eq!(out.window, label);
    assert_eq!(
        &*w.host.moves.lock().unwrap(),
        &[(
            label.clone(),
            WindowPlace {
                pos: Some((470, 388)),
                size: None,
            }
        )],
        "끈 자리로 옮기고 크기는 고르지 않는다"
    );
    assert!(w.host.opened_at.lock().unwrap().is_empty(), "새 창 없음");
    assert_eq!(w.version(), version, "모델 변경 0");
    assert_invariants(&w);
}
