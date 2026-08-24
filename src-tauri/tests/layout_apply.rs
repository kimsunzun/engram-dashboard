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
//! ★이 파일은 워크스페이스 회귀에 안 실린다★ — 그 명령이 `--exclude engram-dashboard` 로 이 패키지를
//! 통째로 뺀다. 그래서 CI가 **이 타깃만 따로 부르는 전용 스텝**을 갖는다(`.github/workflows/ci.yml`) —
//! 그 스텝을 지우면 셸 조각 커버리지가 도로 0이 된다.
//!
//! 창 label 발급은 **실 `PopupCounter`**(프로브 껍데기만 덧씌운다), 구독 재동기는 **실 `OutputRouter`** 를
//! 쓴다 — 가짜로 흉내내면 label 단조성·라우팅 계약을 검증하는 게 아니라 가짜를 검증한다. 나머지 세
//! 포트(알림·OS 창·스폰)는 기록용 가짜다.
//!
//! ## ★락 위치 프로브(이 하네스의 핵심)★
//! 이 리팩터가 지키려는 불변식은 "어느 포트가 락 **안**에서 불리고 어느 포트가 **밖**에서 불리나"인데,
//! 호출을 세는 것만으로는 그게 안 잡힌다 — 위치를 옮겨도 개수가 같다. 그래서 각 가짜 포트가 호출된
//! **그 자리에서** `try_lock` 으로 락 보유 여부를 직접 본다:
//! - `LayoutEvents`·`WindowHost`·`LabelSource`·`AgentSpawner` → `is_ok()`(락 **밖**이어야 한다).
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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use uuid::Uuid;

use engram_dashboard_lib::commands::popout::PopupCounter;
use engram_dashboard_lib::layout::apply::{
    self, AgentSpawner, LabelSource, LayoutEvents, SubscriptionSync, WindowHost, WindowTabsPayload,
};
use engram_dashboard_lib::layout::{
    tree, LayoutState, SlotContent, SplitDir, ViewManager, ViewSnapshot, MAIN_WINDOW_LABEL,
};
use engram_dashboard_lib::output_router::OutputRouter;

// ── 락 위치 프로브 ───────────────────────────────────────────────────────────

// 같은 `Arc<Mutex<ViewManager>>` 를 공유해 호출 시점의 락 보유 여부를 본다(헤더 §락 위치 프로브).
struct Probe(LayoutState);

impl Probe {
    fn new(state: &LayoutState) -> Self {
        Probe(state.clone())
    }

    fn assert_outside(&self, who: &str) {
        assert!(
            self.0 .0.try_lock().is_ok(),
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

struct Recorder {
    probe: Probe,
    layout: Mutex<Vec<ViewSnapshot>>,
    tabs: Mutex<Vec<WindowTabsPayload>>,
}

impl Recorder {
    fn new(state: &LayoutState) -> Self {
        Recorder {
            probe: Probe::new(state),
            layout: Mutex::new(Vec::new()),
            tabs: Mutex::new(Vec::new()),
        }
    }
}

impl LayoutEvents for Recorder {
    fn layout_updated(&self, snapshot: &ViewSnapshot) {
        self.probe.assert_outside("LayoutEvents::layout_updated");
        self.layout.lock().unwrap().push(snapshot.clone());
    }

    fn window_tabs_updated(&self, tabs: &WindowTabsPayload) {
        self.probe
            .assert_outside("LayoutEvents::window_tabs_updated");
        self.tabs.lock().unwrap().push(tabs.clone());
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

struct Host {
    probe: Probe,
    fail_open: bool,
    opened: Mutex<Vec<String>>,
    closed: Mutex<Vec<String>>,
}

impl Host {
    fn new(state: &LayoutState, fail_open: bool) -> Self {
        Host {
            probe: Probe::new(state),
            fail_open,
            opened: Mutex::new(Vec::new()),
            closed: Mutex::new(Vec::new()),
        }
    }
}

impl WindowHost for Host {
    fn open(&self, label: &str) -> Result<(), String> {
        self.probe.assert_outside("WindowHost::open");
        self.opened.lock().unwrap().push(label.to_string());
        if self.fail_open {
            return Err("창 생성 실패(테스트)".to_string());
        }
        Ok(())
    }

    fn close(&self, label: &str) {
        self.probe.assert_outside("WindowHost::close");
        self.closed.lock().unwrap().push(label.to_string());
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
    ) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>> {
        Box::pin(async move {
            // 락 안에서 await 하면 명령 future 가 Send 를 잃어 컴파일도 안 되지만, 그 벽이 서 있는지를
            // 여기서도 본다(형제 포트와 같은 프로브 — 벽이 하나면 조용히 무너진다).
            self.probe.assert_outside("AgentSpawner::spawn_by_cwd");
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.cwds.lock().unwrap().push(cwd);
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
        World {
            subs: Subs::new(&state),
            ev: Recorder::new(&state),
            host: Host::new(&state, fail_open),
            labels: Labels {
                probe: Probe::new(&state),
                inner: PopupCounter::default(),
            },
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
    assert!(apply::list_windows(&w.state).unwrap().contains(&label));
    assert_eq!(
        apply::list_tabs(&w.state, &label).unwrap().tabs.len(),
        1,
        "빈 탭 1개"
    );
    // 성공 경로는 프론트에 아무것도 안 쏜다(창 mount 시 list_tabs pull — §3-3).
    assert_eq!(w.layout_events() + w.tab_events(), 0);
}

#[test]
fn create_window_rolls_back_model_when_open_fails() {
    let w = World::failing_host();
    let err = apply::create_window(&w.state, &w.subs, &w.host, &w.labels).unwrap_err();
    assert!(err.contains("창 생성 실패"), "err={err}");

    let label = w.host.opened.lock().unwrap()[0].clone();
    assert!(
        !apply::list_windows(&w.state).unwrap().contains(&label),
        "빌드 실패 시 모델 롤백 — 유령 창 금지"
    );
    assert_eq!(w.resyncs(), 2, "생성·롤백 각 1회");
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

    apply::focus_slot(&w.state, &w.ev, view, target).unwrap();

    assert_eq!(w.snapshot(view).focused_slot_id, Some(target));
    assert_eq!(w.layout_events(), 1);
    assert_eq!(w.tab_events(), 1);
    assert_eq!(w.resyncs(), 0, "포커스는 출력 라우팅을 안 바꾼다");
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

// ── move_slot_to_window ──────────────────────────────────────────────────────

impl World {
    // agent 하나가 든 슬롯 + 그것을 담은 View — pop-out 의 전제(빈 슬롯은 거부되므로).
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
    let (view, slot, _) = w.view_with_filled_slot();

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
    let tabs = apply::list_tabs(&w.state, &target).unwrap();
    assert_eq!(tabs.tabs.len(), tabs_before + 1);
    assert_eq!(tabs.active, moved.tab);
}

#[test]
fn move_slot_to_window_refuses_an_empty_slot() {
    let w = World::new();
    let view = w.main_active();
    let slot = w.empty_slot(view);

    let err = apply::move_slot_to_window(
        &w.state, &w.subs, &w.ev, &w.host, &w.labels, view, slot, None,
    )
    .unwrap_err();

    assert!(err.contains("빈 슬롯"), "err={err}");
    assert!(w.host.opened.lock().unwrap().is_empty(), "창도 안 연다");
    assert!(w.slots(view).contains(&slot), "부분변경 금지");
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

#[tokio::test]
async fn spawn_into_rejects_explicit_backend_before_spawning() {
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
        Some("claude".to_string()),
        "C:/tmp".to_string(),
    )
    .await
    .unwrap_err();

    assert!(err.contains("스폰 안 함"), "err={err}");
    assert_eq!(spawner.calls(), 0, "★스폰 전에 거부★ — ADR-0058");
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
    let slots = w.slots(view);
    let focused = w.snapshot(view).focused_slot_id.unwrap();
    let other = *slots
        .iter()
        .find(|s| **s != focused)
        .expect("main 은 2슬롯");

    let right = apply::resolve_spatial(&w.state, "right", None, Some(view)).unwrap();
    assert_eq!(right, Some(other));
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
