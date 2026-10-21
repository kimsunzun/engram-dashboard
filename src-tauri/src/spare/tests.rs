//! 예열 창 상태 기계 시험(TRD S21-slot-docking §8-1 ⑫) — 가짜 첫 화면 문 · 번호 발급기 · 확정 포트 · 짓기로 잰다.
//! 시한은 주입한다 — 실제 1 s 를 기다리는 시험은 없다(시한 시험만 몇 ms).

use super::*;

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{OnceLock, Weak};
use std::thread;

use pool::Slot;

use crate::layout::geometry::Insets;
use crate::layout::manager::ViewId;
use crate::layout::CanvasPx;

// 옳은 시험은 닿지 않는 기다림 상한 — 깨우기가 빠지면 이만큼 기다린 뒤 빨개진다. ★깨움은 `LONG / 2` 로 잰다★ —
//   Windows 의 조건 변수 시한은 몇 ms 일찍 풀려, `< LONG` 은 깨우기를 뺀 변이도 통과시켰다(실측 2026-10-21).
const LONG: Duration = Duration::from_secs(10);
const SAFETY: Duration = Duration::from_millis(7);

// ── 가짜 첫 화면 문 · 번호 발급기 · 확정 포트 ──

#[derive(Debug, Clone, PartialEq)]
enum GateCall {
    Hold(String),
    Cancel(String),
    Reveal {
        label: String,
        generation: u32,
        activate: bool,
        held: bool,
    },
}

// 올림 걸기 · 거두기는 풀 락 안, 올리기는 풀 락 밖이어야 한다 — 부를 때마다 잰다(어긋나면 그 자리에서 빨갛다).
#[derive(Default)]
struct FakeGate {
    spare: OnceLock<Weak<SpareState>>,
    calls: Mutex<Vec<GateCall>>,
    held: Mutex<BTreeMap<String, Option<(u32, bool)>>>,
}

impl FakeGate {
    fn pool_locked(&self) -> bool {
        let spare = self.spare.get().and_then(Weak::upgrade).expect("풀이 산다");
        let locked = spare.pool.try_lock().is_err();
        locked
    }

    fn calls(&self) -> Vec<GateCall> {
        self.calls.lock().unwrap().clone()
    }

    fn is_held(&self, label: &str) -> bool {
        self.held.lock().unwrap().contains_key(label)
    }

    // ③ 이 올린 항목 — 채택 번호 · 활성화.
    fn raised(&self, label: &str) -> Option<(u32, bool)> {
        self.held.lock().unwrap().get(label).copied().flatten()
    }

    // UI 숨기기 — 풀 밖의 부르는 쪽이 술어와 상관없이 그 label 의 올림을 거둔다.
    fn hide(&self, label: &str) {
        self.held.lock().unwrap().remove(label);
    }
}

impl PaintGate for FakeGate {
    fn hold_unarmed(&self, label: &str) {
        assert!(self.pool_locked(), "올림은 맡김과 같은 풀 락 안에서 건다");
        self.held.lock().unwrap().insert(label.to_owned(), None);
        self.calls
            .lock()
            .unwrap()
            .push(GateCall::Hold(label.to_owned()));
    }

    fn cancel(&self, label: &str) {
        assert!(
            self.pool_locked(),
            "올림은 상태가 바뀌는 풀 락 안에서 거둔다"
        );
        self.held.lock().unwrap().remove(label);
        self.calls
            .lock()
            .unwrap()
            .push(GateCall::Cancel(label.to_owned()));
    }

    fn reveal_if_held(&self, label: &str, generation: u32, activate: bool) -> bool {
        assert!(!self.pool_locked(), "③ 은 풀 락 밖이다");
        let held = match self.held.lock().unwrap().get_mut(label) {
            Some(entry) => {
                *entry = Some((generation, activate));
                true
            }
            None => false,
        };
        self.calls.lock().unwrap().push(GateCall::Reveal {
            label: label.to_owned(),
            generation,
            activate,
            held,
        });
        held
    }
}

#[derive(Default)]
struct Labels(AtomicU32);

impl Labels {
    fn issued(&self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

impl LabelSource for Labels {
    fn next_label(&self) -> String {
        format!("slot-popup-{}", self.0.fetch_add(1, Ordering::SeqCst) + 1)
    }

    fn tab_name(&self, label: &str) -> String {
        label.to_owned()
    }
}

#[derive(Debug, Clone, PartialEq)]
enum PortCall {
    Adopted { label: String, generation: u32 },
    Deadline { label: String, after: Duration },
    Destroy(String),
}

struct FakePort {
    spare: Arc<SpareState>,
    layout: LayoutState,
    calls: Mutex<Vec<PortCall>>,
    destroy: Mutex<VecDeque<Result<(), String>>>,
}

impl FakePort {
    fn new(spare: &Arc<SpareState>, layout: &LayoutState) -> Self {
        Self {
            spare: spare.clone(),
            layout: layout.clone(),
            calls: Mutex::default(),
            destroy: Mutex::default(),
        }
    }

    fn destroy_results(self, results: Vec<Result<(), String>>) -> Self {
        *self.destroy.lock().unwrap() = results.into();
        self
    }

    fn calls(&self) -> Vec<PortCall> {
        self.calls.lock().unwrap().clone()
    }

    fn outside_locks(&self, what: &str) {
        assert!(self.spare.pool.try_lock().is_ok(), "{what} — 풀 락 밖");
        assert!(self.layout.0.try_lock().is_ok(), "{what} — 레이아웃 락 밖");
    }
}

impl CommitPort for FakePort {
    fn adopted(&self, label: &str, generation: u32) {
        self.outside_locks("채택 사건");
        self.calls.lock().unwrap().push(PortCall::Adopted {
            label: label.to_owned(),
            generation,
        });
    }

    fn reveal_deadline(&self, label: &str, after: Duration) {
        self.outside_locks("안전 시한");
        self.calls.lock().unwrap().push(PortCall::Deadline {
            label: label.to_owned(),
            after,
        });
    }

    fn destroy(&self, label: &str) -> Result<(), String> {
        self.outside_locks("닫기");
        self.calls
            .lock()
            .unwrap()
            .push(PortCall::Destroy(label.to_owned()));
        self.destroy.lock().unwrap().pop_front().unwrap_or(Ok(()))
    }
}

// ── 차림 ──

fn fresh_with(build_wait: Duration) -> (Arc<SpareState>, Arc<FakeGate>) {
    let gate = Arc::new(FakeGate::default());
    let spare = Arc::new(SpareState::new(SpareSeams {
        gate: gate.clone(),
        build_wait,
        reveal_safety: SAFETY,
    }));
    gate.spare.set(Arc::downgrade(&spare)).unwrap();
    (spare, gate)
}

fn fresh() -> (Arc<SpareState>, Arc<FakeGate>) {
    fresh_with(LONG)
}

fn built(spare: &SpareState, labels: &Labels) -> String {
    match spare.build(labels, |_| Ok(())) {
        SpareBuild::Built { label } => label,
        other => panic!("지어야 한다: {other:?}"),
    }
}

fn claimed(spare: &SpareState, labels: &Labels) -> SpareClaim {
    built(spare, labels);
    spare.claim().claim.expect("지어진 창을 맡긴다")
}

// 맡긴 창의 새 창 항목을 모델에 넣는다(놓기 · 팝아웃이 확정 전에 하는 일).
fn model_with(label: &str) -> (LayoutState, ViewId) {
    let layout = LayoutState::new();
    let view = layout.0.lock().unwrap().create_window(label).unwrap();
    (layout, view)
}

fn model_measures(layout: &LayoutState, view: ViewId) -> Option<(CanvasPx, UiMetrics)> {
    layout.0.lock().unwrap().px_context(view).unwrap()
}

fn metrics(min_pane_px: u32) -> UiMetrics {
    UiMetrics {
        frame_insets: Insets {
            t: 1.0,
            r: 1.0,
            b: 1.0,
            l: 1.0,
        },
        min_pane_px,
    }
}

fn adopt(spare: &Arc<SpareState>, claim: &SpareClaim) -> CommitOutcome {
    let (layout, _) = model_with(&claim.label);
    spare.commit(&layout, claim, &FakePort::new(spare, &layout))
}

// 짓기 단계 안에서 `go` 를 기다리는 짓기 — 그동안 칸은 `Building` 이다.
struct HeldBuild {
    go: mpsc::Sender<()>,
    done: thread::JoinHandle<SpareBuild>,
    label: String,
}

impl HeldBuild {
    fn start(spare: &Arc<SpareState>, labels: &Arc<Labels>, result: Result<(), String>) -> Self {
        let (go, wait) = mpsc::channel::<()>();
        let (started, entered) = mpsc::channel::<String>();
        let done = {
            let spare = spare.clone();
            let labels = labels.clone();
            thread::spawn(move || {
                spare.build(&*labels, |label| {
                    started.send(label.to_owned()).unwrap();
                    wait.recv().unwrap();
                    result
                })
            })
        };
        let label = entered.recv_timeout(LONG).expect("짓기 단계에 들었다");
        Self { go, done, label }
    }

    fn finish(self) -> SpareBuild {
        self.go.send(()).unwrap();
        self.done.join().unwrap()
    }
}

fn claim_in_background(spare: &Arc<SpareState>) -> thread::JoinHandle<ClaimReply> {
    let spare = spare.clone();
    thread::spawn(move || spare.claim())
}

fn wait_for_waiters(spare: &SpareState, n: u32) {
    let until = Instant::now() + LONG;
    while spare.pool().waiting() != n {
        assert!(Instant::now() < until, "맡기기 {n} 개가 서지 않았다");
        thread::yield_now();
    }
}

// ── 맡기기(I2 · I16 · I17) ──

#[test]
fn a_built_spare_is_handed_out_once_with_a_receive_only_hold() {
    let (spare, gate) = fresh();
    let label = built(&spare, &Labels::default());
    assert!(
        gate.calls().is_empty(),
        "맡기지 않은 예열 창에는 올림이 없다"
    );
    let reply = spare.claim();
    assert_eq!(
        reply.claim.as_ref().map(|c| c.label.as_str()),
        Some(label.as_str())
    );
    assert!(!reply.build);
    assert_eq!(reply.waited, None);
    assert_eq!(gate.calls(), [GateCall::Hold(label.clone())]);
    assert!(spare.is_spare(&label), "확정 전에는 풀의 창이다");
    let second = spare.claim();
    assert_eq!((second.claim, second.build), (None, false));
}

#[test]
fn an_empty_pool_hands_out_nothing_and_asks_for_a_build_unless_exiting() {
    let (spare, _) = fresh();
    let reply = spare.claim();
    assert_eq!((reply.claim, reply.build), (None, true));
    spare.begin_exit();
    let reply = spare.claim();
    assert_eq!((reply.claim, reply.build), (None, false));
}

#[test]
fn exiting_hands_out_nothing_even_with_a_built_spare() {
    let (spare, gate) = fresh();
    built(&spare, &Labels::default());
    spare.begin_exit();
    let reply = spare.claim();
    assert_eq!((reply.claim, reply.build), (None, false));
    assert!(gate.calls().is_empty());
}

#[test]
fn a_claim_waits_for_a_building_spare_and_takes_it() {
    let (spare, gate) = fresh();
    let labels = Arc::new(Labels::default());
    let building = HeldBuild::start(&spare, &labels, Ok(()));
    let label = building.label.clone();
    assert!(spare.is_spare(&label));
    let claimer = claim_in_background(&spare);
    wait_for_waiters(&spare, 1);
    assert!(gate.calls().is_empty(), "짓는 중인 창에는 올림이 없다");
    assert_eq!(
        building.finish(),
        SpareBuild::Built {
            label: label.clone()
        }
    );
    let reply = claimer.join().unwrap();
    assert_eq!(reply.claim.map(|c| c.label), Some(label));
    assert!(reply.waited.is_some());
}

#[test]
fn a_claim_gives_up_on_a_building_spare_at_the_wait_limit() {
    let (spare, _) = fresh_with(Duration::from_millis(20));
    let labels = Arc::new(Labels::default());
    let building = HeldBuild::start(&spare, &labels, Ok(()));
    let reply = spare.claim();
    assert_eq!(
        (reply.claim, reply.build),
        (None, false),
        "짓기가 이미 돈다"
    );
    assert!(reply.waited.unwrap() >= Duration::from_millis(20));
    assert_eq!(spare.pool().waiting(), 0);
    assert!(matches!(building.finish(), SpareBuild::Built { .. }));
    assert!(spare.claim().claim.is_some(), "지어진 뒤에는 맡긴다");
}

#[test]
fn two_claims_waiting_on_one_build_get_it_exactly_once() {
    let (spare, gate) = fresh();
    let labels = Arc::new(Labels::default());
    let building = HeldBuild::start(&spare, &labels, Ok(()));
    let a = claim_in_background(&spare);
    let b = claim_in_background(&spare);
    wait_for_waiters(&spare, 2);
    building.finish();
    let replies = [a.join().unwrap(), b.join().unwrap()];
    assert_eq!(replies.iter().filter(|r| r.claim.is_some()).count(), 1);
    assert!(
        replies.iter().all(|r| !r.build),
        "남은 쪽은 맡긴 창을 만나 짓지 않는다"
    );
    assert_eq!(gate.calls().len(), 1, "올림도 하나");
}

#[test]
fn a_failed_build_wakes_a_waiting_claim_before_the_limit() {
    let (spare, _) = fresh();
    let labels = Arc::new(Labels::default());
    let building = HeldBuild::start(&spare, &labels, Err("짓기 실패".to_owned()));
    let claimer = claim_in_background(&spare);
    wait_for_waiters(&spare, 1);
    assert!(matches!(building.finish(), SpareBuild::Failed { .. }));
    let reply = claimer.join().unwrap();
    assert_eq!(reply.claim, None);
    assert!(reply.build, "비었으니 다시 짓는다(I16)");
    assert!(
        reply.waited.unwrap() < LONG / 2,
        "시한이 아니라 깨우기로 끝났다"
    );
}

#[test]
fn a_building_spare_lost_while_a_claim_waits_wakes_it_before_the_limit() {
    let (spare, _) = fresh();
    let labels = Arc::new(Labels::default());
    let building = HeldBuild::start(&spare, &labels, Ok(()));
    let claimer = claim_in_background(&spare);
    wait_for_waiters(&spare, 1);
    assert!(spare.on_destroyed(&building.label), "다시 짓는다");
    let reply = claimer.join().unwrap();
    assert_eq!(reply.claim, None);
    assert!(
        reply.waited.unwrap() < LONG / 2,
        "시한이 아니라 깨우기로 끝났다"
    );
    let label = building.label.clone();
    assert_eq!(building.finish(), SpareBuild::Vanished { label });
}

#[test]
fn exiting_wakes_a_waiting_claim() {
    let (spare, _) = fresh();
    let labels = Arc::new(Labels::default());
    let building = HeldBuild::start(&spare, &labels, Ok(()));
    let claimer = claim_in_background(&spare);
    wait_for_waiters(&spare, 1);
    spare.begin_exit();
    let reply = claimer.join().unwrap();
    assert_eq!((reply.claim, reply.build), (None, false));
    assert!(
        reply.waited.unwrap() < LONG / 2,
        "시한이 아니라 깨우기로 끝났다"
    );
    building.finish();
}

// 기다리던 짓기 A 가 사라지거나 실패하고, 깨운 맡기기가 락을 다시 잡기 전에 다른 짓기 B 가 자리를 잡았다(짓는 중 ·
//   지어짐). 그 사이를 풀 락으로 고정한다 — 깬 맡기기는 이 락을 놓은 뒤에야 칸을 다시 본다.
#[test]
fn a_claim_whose_awaited_build_ended_does_not_follow_its_replacement() {
    for (ending, replacement_built) in [
        ("destroyed", false),
        ("destroyed", true),
        ("failed", false),
        ("failed", true),
    ] {
        let (spare, _) = fresh();
        let labels = Arc::new(Labels::default());
        let a = HeldBuild::start(&spare, &labels, Ok(()));
        let claimer = claim_in_background(&spare);
        wait_for_waiters(&spare, 1);
        {
            let mut pool = spare.pool();
            let ended = match ending {
                "destroyed" => matches!(
                    pool.destroyed(&a.label, Instant::now()),
                    Gone::Removed { .. }
                ),
                _ => pool.build_failed(&a.label),
            };
            assert!(ended);
            pool.reserve().unwrap();
            assert!(pool.assign("slot-popup-b".to_owned()));
            if replacement_built {
                assert!(pool.mark_built("slot-popup-b"));
            }
            spare.changed.notify_all();
        }
        let reply = claimer.join().unwrap();
        let case = format!("{ending} · B 지어짐 = {replacement_built}");
        assert_eq!(reply.claim, None, "{case} — B 를 맡지 않는다");
        assert!(!reply.build, "{case} — B 가 이미 칸에 있다");
        assert!(
            reply.waited.unwrap() < LONG / 2,
            "{case} — B 를 따라 기다리지 않는다"
        );
        a.finish();
    }
}

#[test]
fn a_building_spare_dropped_for_a_listen_failure_wakes_a_waiting_claim() {
    let (spare, _) = fresh();
    let labels = Arc::new(Labels::default());
    let building = HeldBuild::start(&spare, &labels, Ok(()));
    let claimer = claim_in_background(&spare);
    wait_for_waiters(&spare, 1);
    assert!(matches!(
        spare.on_listen(&building.label, false),
        Listen::Removed { claimed: false, .. }
    ));
    let reply = claimer.join().unwrap();
    assert_eq!(reply.claim, None);
    assert!(
        reply.waited.unwrap() < LONG / 2,
        "시한이 아니라 깨우기로 끝났다"
    );
    building.finish();
}

// ── 짓기(I13 · I25) ──

#[test]
fn one_build_at_a_time_and_a_refused_build_takes_no_number() {
    let (spare, _) = fresh();
    let labels = Arc::new(Labels::default());
    let building = HeldBuild::start(&spare, &labels, Ok(()));
    assert_eq!(
        spare.build(&*labels, |_| panic!("거절된 짓기는 짓지 않는다")),
        SpareBuild::Refused(NoBuild::Occupied)
    );
    assert_eq!(labels.issued(), 1, "거절된 짓기는 번호를 받지 않는다");
    building.finish();
    assert_eq!(
        spare.build(&*labels, |_| panic!("지어진 창이 있다")),
        SpareBuild::Refused(NoBuild::Occupied)
    );
    assert_eq!(labels.issued(), 1);
}

#[test]
fn a_second_claim_after_commit_step_one_neither_takes_nor_builds() {
    let (spare, _) = fresh();
    let labels = Labels::default();
    let claim = claimed(&spare, &labels);
    let (layout, _) = model_with(&claim.label);
    let step = spare.commit_locked(&mut layout.0.lock().unwrap(), &claim);
    let reply = spare.claim();
    assert_eq!(
        (reply.claim, reply.build),
        (None, false),
        "보임을 기다린다(I25)"
    );
    assert_eq!(
        spare.build(&labels, |_| panic!("보임 완료가 짓는다")),
        SpareBuild::Refused(NoBuild::AwaitingReveal)
    );
    assert_eq!(labels.issued(), 1);
    let _ = step.finish(&spare, &FakePort::new(&spare, &layout));
}

#[test]
fn the_rebuild_after_a_reveal_happens_once_per_label() {
    type Done = fn(&SpareState, &str) -> bool;
    let done: [(&str, Done); 3] = [
        ("shown", SpareState::on_shown),
        ("overdue", SpareState::on_reveal_overdue),
        ("destroyed", SpareState::on_destroyed),
    ];
    for (first, run_first) in done {
        let (spare, _) = fresh();
        let labels = Labels::default();
        let claim = claimed(&spare, &labels);
        adopt(&spare, &claim);
        assert!(run_first(&spare, &claim.label), "{first} 이 먼저 와 짓는다");
        for (later, run_later) in done {
            assert!(
                !run_later(&spare, &claim.label),
                "{first} 뒤의 {later} 는 무동작"
            );
        }
        assert_eq!(spare.reveal_generation(&claim.label), None);
        assert!(matches!(
            spare.build(&labels, |_| Ok(())),
            SpareBuild::Built { .. }
        ));
    }
}

#[test]
fn a_reveal_build_and_an_empty_claim_build_overlapping_make_one_spare() {
    let (spare, _) = fresh();
    let labels = Arc::new(Labels::default());
    let claim = claimed(&spare, &labels);
    adopt(&spare, &claim);
    assert!(spare.on_shown(&claim.label));
    assert!(spare.claim().build, "비었고 보임 기다림도 없다");
    let first = HeldBuild::start(&spare, &labels, Ok(()));
    assert_eq!(
        spare.build(&*labels, |_| panic!("하나만 선다")),
        SpareBuild::Refused(NoBuild::Occupied)
    );
    first.finish();
    assert_eq!(labels.issued(), 2, "맡긴 창 하나 + 다시 지은 하나");
}

#[test]
fn a_completion_for_another_label_leaves_the_pending_reveal_alone() {
    let (spare, _) = fresh();
    let claim = claimed(&spare, &Labels::default());
    adopt(&spare, &claim);
    type Done = fn(&SpareState, &str) -> bool;
    let done: [Done; 3] = [
        SpareState::on_shown,
        SpareState::on_reveal_overdue,
        SpareState::on_destroyed,
    ];
    for run in done {
        assert!(!run(&spare, "slot-popup-9"));
        assert_eq!(
            spare.reveal_generation(&claim.label),
            Some(claim.generation)
        );
    }
}

#[test]
fn a_reveal_completion_while_exiting_clears_without_building() {
    let (spare, _) = fresh();
    let claim = claimed(&spare, &Labels::default());
    adopt(&spare, &claim);
    spare.begin_exit();
    assert!(!spare.on_shown(&claim.label));
    assert_eq!(spare.awaiting_reveal(), None);
}

// ── 되물림 · 맡김표(I27) ──

#[test]
fn a_release_with_the_current_ticket_returns_the_spare_hidden_and_claimable() {
    let (spare, gate) = fresh();
    let first = claimed(&spare, &Labels::default());
    assert!(spare.release(&first));
    assert!(matches!(spare.pool().slot(), Slot::Built { label, .. } if *label == first.label));
    assert!(spare.is_spare(&first.label), "숨긴 채 풀에 남는다");
    let second = spare.claim().claim.expect("다시 맡긴다");
    assert_eq!(second.label, first.label);
    assert_ne!(second.generation, first.generation);
    assert_eq!(
        gate.calls(),
        [
            GateCall::Hold(first.label.clone()),
            GateCall::Cancel(first.label.clone()),
            GateCall::Hold(first.label.clone()),
        ]
    );
    assert!(
        gate.is_held(&first.label),
        "앞 되물림의 거둠이 다시 맡긴 올림을 지우지 않는다"
    );
}

#[test]
fn a_late_release_or_commit_from_an_older_ticket_is_ignored() {
    let (spare, gate) = fresh();
    let a = claimed(&spare, &Labels::default());
    assert!(spare.release(&a));
    let b = spare.claim().claim.expect("B 가 다시 맡는다");
    let (layout, _) = model_with(&a.label);
    let port = FakePort::new(&spare, &layout);
    let calls_before = gate.calls();
    assert!(!spare.release(&a), "A 의 늦은 되물림");
    assert_eq!(
        spare.commit(&layout, &a, &port),
        CommitOutcome::Stale,
        "A 의 늦은 확정"
    );
    assert_eq!(gate.calls(), calls_before, "B 의 올림을 건드리지 않는다");
    assert!(gate.is_held(&b.label));
    assert!(port.calls().is_empty());
    assert!(matches!(
        spare.pool().slot(),
        Slot::Claimed { generation, .. } if *generation == b.generation
    ));
    assert_eq!(
        spare.commit(&layout, &b, &port),
        CommitOutcome::Adopted { revealed: true }
    );
}

#[test]
fn a_commit_with_a_ticket_of_another_generation_does_nothing() {
    let (spare, gate) = fresh();
    let claim = claimed(&spare, &Labels::default());
    let (layout, _) = model_with(&claim.label);
    let port = FakePort::new(&spare, &layout);
    let forged = SpareClaim {
        generation: claim.generation.wrapping_add(1),
        ..claim.clone()
    };
    assert_eq!(spare.commit(&layout, &forged, &port), CommitOutcome::Stale);
    assert!(!spare.release(&forged));
    assert!(port.calls().is_empty());
    assert!(gate.is_held(&claim.label));
    assert!(spare.is_spare(&claim.label));
}

// ── 열기(I4 · I20) ──

#[test]
fn a_claimed_label_is_opened_in_place_and_a_lost_one_is_refused() {
    let (spare, gate) = fresh();
    let claim = claimed(&spare, &Labels::default());
    assert_eq!(spare.opening("slot-popup-99", true), Opening::Fresh);
    assert_eq!(spare.opening(&claim.label, true), Opening::Claimed);
    assert!(
        spare.on_destroyed(&claim.label),
        "맡긴 창이 사라지면 다시 짓는다"
    );
    assert_eq!(
        gate.calls().last(),
        Some(&GateCall::Cancel(claim.label.clone()))
    );
    assert_eq!(
        spare.opening(&claim.label, true),
        Opening::Unavailable,
        "사라진 맡긴 창의 label 로 새로 짓지 않는다"
    );
}

#[test]
fn the_activate_choice_of_the_open_reaches_the_reveal() {
    for activate in [false, true] {
        let (spare, gate) = fresh();
        let claim = claimed(&spare, &Labels::default());
        assert_eq!(spare.opening(&claim.label, activate), Opening::Claimed);
        adopt(&spare, &claim);
        assert_eq!(
            gate.raised(&claim.label),
            Some((claim.generation, activate))
        );
    }
}

// ── 확정(I6 · I21 · I26 · I31) ──

#[test]
fn commit_carries_the_measures_then_reveals_then_announces_outside_both_locks() {
    let (spare, gate) = fresh();
    let label = built(&spare, &Labels::default());
    let (layout, view) = model_with(&label);
    spare.report_canvas(&layout, &label, 800, 600).unwrap();
    spare.report_metrics(&layout, &label, metrics(50)).unwrap();
    assert_eq!(
        model_measures(&layout, view),
        None,
        "확정 전 보고는 풀에 적힌다"
    );
    let claim = spare.claim().claim.unwrap();
    let port = FakePort::new(&spare, &layout);
    assert_eq!(
        spare.commit(&layout, &claim, &port),
        CommitOutcome::Adopted { revealed: true }
    );
    assert_eq!(
        model_measures(&layout, view),
        Some((CanvasPx { w: 800, h: 600 }, metrics(50)))
    );
    assert_eq!(
        gate.calls().last(),
        Some(&GateCall::Reveal {
            label: label.clone(),
            generation: claim.generation,
            activate: true,
            held: true,
        })
    );
    assert_eq!(
        port.calls(),
        [
            PortCall::Adopted {
                label: label.clone(),
                generation: claim.generation
            },
            PortCall::Deadline {
                label: label.clone(),
                after: SAFETY
            },
        ]
    );
    assert!(!spare.is_spare(&label), "이제 모델 창이다");
    assert_eq!(spare.reveal_generation(&label), Some(claim.generation));
}

#[test]
fn a_report_during_the_commit_lands_on_top_and_one_before_it_is_carried() {
    let (spare, _) = fresh();
    let label = built(&spare, &Labels::default());
    let (layout, view) = model_with(&label);
    spare.report_canvas(&layout, &label, 800, 600).unwrap();
    spare.report_metrics(&layout, &label, metrics(50)).unwrap();
    let claim = spare.claim().claim.unwrap();

    let mut mgr = layout.0.lock().unwrap();
    let step = spare.commit_locked(&mut mgr, &claim);
    // 확정의 레이아웃 락 구간 안 — 풀은 그 label 을 더 받지 않고, 지금 닿는 보고는 이 락을 기다린다.
    assert_eq!(
        spare.record_measure(&label, Measure::Canvas { w: 1, h: 1 }),
        None
    );
    let reporter = {
        let (spare, layout, label) = (spare.clone(), layout.clone(), label.clone());
        thread::spawn(move || {
            spare.report_canvas(&layout, &label, 1024, 768).unwrap();
            spare.report_metrics(&layout, &label, metrics(60)).unwrap();
        })
    };
    assert_eq!(
        mgr.px_context(view).unwrap(),
        Some((CanvasPx { w: 800, h: 600 }, metrics(50))),
        "확정 전에 닿은 보고 = 확정이 옮긴 값"
    );
    drop(mgr);
    // ③ 의 락 밖 검사가 보고 스레드의 락을 잘못 잡지 않게 그 스레드를 먼저 끝낸다.
    reporter.join().unwrap();
    let _ = step.finish(&spare, &FakePort::new(&spare, &layout));
    assert_eq!(
        model_measures(&layout, view),
        Some((CanvasPx { w: 1024, h: 768 }, metrics(60))),
        "확정 도중 닿은 보고가 옛 값에 덮이지 않는다"
    );
}

#[test]
fn commit_for_a_label_gone_from_the_model_closes_it_outside_both_locks() {
    let (spare, gate) = fresh();
    let labels = Labels::default();
    let claim = claimed(&spare, &labels);
    let layout = LayoutState::new();
    let port = FakePort::new(&spare, &layout);
    assert_eq!(spare.commit(&layout, &claim, &port), CommitOutcome::Closed);
    assert_eq!(port.calls(), [PortCall::Destroy(claim.label.clone())]);
    assert_eq!(
        gate.calls().last(),
        Some(&GateCall::Cancel(claim.label.clone()))
    );
    assert_eq!(spare.awaiting_reveal(), None);
    assert!(!spare.release(&claim), "돌려받지 않는다");
    assert!(
        spare.is_spare(&claim.label),
        "그 Destroyed 가 뺄 때까지 풀의 창이다"
    );
    let reply = spare.claim();
    assert_eq!(
        (reply.claim, reply.build),
        (None, false),
        "그동안 아무도 맡지 못한다"
    );
    assert!(
        spare.on_destroyed(&claim.label),
        "그 Destroyed 가 다시 짓는다"
    );
    assert!(matches!(
        spare.build(&labels, |_| Ok(())),
        SpareBuild::Built { .. }
    ));
}

#[test]
fn a_close_that_fails_is_tried_once_more_and_a_second_failure_frees_the_slot() {
    let fail = || Err::<(), _>("못 닫음".to_owned());
    for (results, outcome, still_spare) in [
        (vec![fail(), Ok(())], CommitOutcome::Closed, true),
        (vec![fail(), fail()], CommitOutcome::CloseFailed, false),
    ] {
        let (spare, _) = fresh();
        let claim = claimed(&spare, &Labels::default());
        let layout = LayoutState::new();
        let port = FakePort::new(&spare, &layout).destroy_results(results);
        assert_eq!(spare.commit(&layout, &claim, &port), outcome);
        assert_eq!(
            port.calls(),
            [
                PortCall::Destroy(claim.label.clone()),
                PortCall::Destroy(claim.label.clone())
            ]
        );
        assert_eq!(spare.is_spare(&claim.label), still_spare);
        assert_eq!(
            spare.claim().build,
            !still_spare,
            "비었으면 다음 맡기기가 짓는다"
        );
    }
}

// ── 숨기기 · 트레이 보이기(I17) ──

#[test]
fn a_hide_between_claim_and_reveal_keeps_the_window_hidden() {
    for hide_after_step_one in [false, true] {
        let (spare, gate) = fresh();
        let claim = claimed(&spare, &Labels::default());
        let (layout, _) = model_with(&claim.label);
        if !hide_after_step_one {
            gate.hide(&claim.label);
        }
        let step = spare.commit_locked(&mut layout.0.lock().unwrap(), &claim);
        if hide_after_step_one {
            gate.hide(&claim.label);
        }
        assert_eq!(
            step.finish(&spare, &FakePort::new(&spare, &layout)),
            CommitOutcome::Adopted { revealed: false },
            "① 뒤 숨기기 = {hide_after_step_one}"
        );
        assert_eq!(gate.raised(&claim.label), None, "올리지 않았다");
        assert_eq!(
            spare.reveal_generation(&claim.label),
            Some(claim.generation),
            "안전 시한이 다시 짓기를 띄운다"
        );
    }
}

#[test]
fn a_tray_show_rehangs_the_hold_of_a_claimed_window_only() {
    let (spare, gate) = fresh();
    let label = built(&spare, &Labels::default());
    assert_eq!(spare.on_tray_show(&label), TrayShow::Untouched);
    assert!(gate.calls().is_empty());
    let claim = spare.claim().claim.unwrap();
    gate.hide(&label);
    assert_eq!(spare.on_tray_show(&label), TrayShow::Rehold);
    assert!(gate.is_held(&label));
    assert_eq!(
        adopt(&spare, &claim),
        CommitOutcome::Adopted { revealed: true }
    );
    assert_eq!(spare.on_tray_show(&label), TrayShow::Adopted);
}

// ── 듣기 실패(I28) ──

#[test]
fn a_window_that_failed_to_listen_leaves_the_pool_without_an_immediate_rebuild() {
    let (spare, _) = fresh();
    let label = built(&spare, &Labels::default());
    assert_eq!(
        spare.on_listen(&label, false),
        Listen::Removed {
            claimed: false,
            disabled_now: false
        }
    );
    assert!(!spare.is_spare(&label));
    assert!(
        !spare.on_destroyed(&label),
        "먼저 뺐으므로 그 Destroyed 는 짓지 않는다"
    );
    assert!(spare.claim().build, "다음 맡기기의 빈 갈래가 짓는다");
}

#[test]
fn three_listen_failures_in_a_row_turn_the_pool_off() {
    let (spare, _) = fresh();
    let labels = Labels::default();
    for nth in 1..=3 {
        let label = built(&spare, &labels);
        assert_eq!(
            spare.on_listen(&label, false),
            Listen::Removed {
                claimed: false,
                disabled_now: nth == 3
            }
        );
    }
    let reply = spare.claim();
    assert_eq!((reply.claim, reply.build), (None, false));
    assert_eq!(
        spare.build(&labels, |_| panic!("꺼졌다")),
        SpareBuild::Refused(NoBuild::Disabled)
    );
    assert_eq!(labels.issued(), 3);
}

#[test]
fn a_window_that_listens_resets_the_failure_count() {
    let (spare, _) = fresh();
    let labels = Labels::default();
    for _ in 0..2 {
        let label = built(&spare, &labels);
        spare.on_listen(&label, false);
    }
    let label = built(&spare, &labels);
    assert_eq!(spare.on_listen(&label, true), Listen::Kept);
    assert!(spare.is_spare(&label));
    assert!(matches!(
        spare.on_listen(&label, false),
        Listen::Removed {
            disabled_now: false,
            ..
        }
    ));
    let label = built(&spare, &labels);
    assert!(matches!(
        spare.on_listen(&label, false),
        Listen::Removed {
            disabled_now: false,
            ..
        }
    ));
    built(&spare, &labels);
    assert!(
        spare.claim().claim.is_some(),
        "셈이 0 에서 다시 세어 꺼지지 않았다"
    );
}

#[test]
fn a_listen_failure_of_a_claimed_window_drops_its_hold_and_its_label() {
    let (spare, gate) = fresh();
    let claim = claimed(&spare, &Labels::default());
    assert_eq!(
        spare.on_listen(&claim.label, false),
        Listen::Removed {
            claimed: true,
            disabled_now: false
        }
    );
    assert!(!gate.is_held(&claim.label));
    assert_eq!(spare.opening(&claim.label, true), Opening::Unavailable);
    assert_eq!(adopt(&spare, &claim), CommitOutcome::Stale);
}

#[test]
fn a_window_outside_the_pool_is_not_counted() {
    let (spare, _) = fresh();
    for _ in 0..3 {
        assert_eq!(spare.on_listen("main", false), Listen::NotPool);
    }
    built(&spare, &Labels::default());
    assert!(spare.claim().claim.is_some());
}

// 듣기 결과는 칸의 창인지 먼저 본다 — 풀 밖 창의 「들었다」가 셈을 되돌리면 잇단 실패가 끝내 풀을 끄지 못한다.
#[test]
fn a_listen_success_from_a_window_outside_the_pool_does_not_reset_the_count() {
    let (spare, _) = fresh();
    let labels = Labels::default();
    for _ in 0..2 {
        let label = built(&spare, &labels);
        spare.on_listen(&label, false);
    }
    assert_eq!(spare.on_listen("main", true), Listen::NotPool);
    let label = built(&spare, &labels);
    assert_eq!(
        spare.on_listen(&label, false),
        Listen::Removed {
            claimed: false,
            disabled_now: true
        }
    );
}

// ── 역할 · 채택 번호(I26) ──

#[test]
fn the_adoption_number_is_reported_only_while_the_reveal_is_pending() {
    let (spare, _) = fresh();
    let claim = claimed(&spare, &Labels::default());
    assert_eq!(spare.window_role(&claim.label, || true), WindowRole::Spare);
    assert_eq!(spare.reveal_generation(&claim.label), None);
    adopt(&spare, &claim);
    assert_eq!(spare.window_role(&claim.label, || true), WindowRole::Model);
    assert_eq!(
        spare.reveal_generation(&claim.label),
        Some(claim.generation)
    );
    assert_eq!(
        spare.awaiting_reveal(),
        Some((claim.label.clone(), claim.generation))
    );
    assert_eq!(spare.reveal_generation("slot-popup-99"), None);
    spare.on_shown(&claim.label);
    assert_eq!(spare.reveal_generation(&claim.label), None);
}

// ── 뜻밖의 `Destroyed` ──

#[test]
fn a_spare_lost_in_any_state_is_rebuilt_unless_exiting() {
    for exiting in [false, true] {
        let stop = |spare: &SpareState| {
            if exiting {
                spare.begin_exit();
            }
        };
        // 지어짐
        let (spare, _) = fresh();
        let label = built(&spare, &Labels::default());
        stop(&spare);
        assert_eq!(spare.on_destroyed(&label), !exiting);
        assert!(!spare.is_spare(&label));
        // 맡김
        let (spare, _) = fresh();
        let claim = claimed(&spare, &Labels::default());
        stop(&spare);
        assert_eq!(spare.on_destroyed(&claim.label), !exiting);
        // 닫는 중
        let (spare, _) = fresh();
        let claim = claimed(&spare, &Labels::default());
        let layout = LayoutState::new();
        let _ = spare.commit(&layout, &claim, &FakePort::new(&spare, &layout));
        stop(&spare);
        assert_eq!(spare.on_destroyed(&claim.label), !exiting);
        // 짓는 중
        let (spare, _) = fresh();
        let labels = Arc::new(Labels::default());
        let building = HeldBuild::start(&spare, &labels, Ok(()));
        stop(&spare);
        assert_eq!(spare.on_destroyed(&building.label), !exiting);
        building.finish();
    }
    let (spare, _) = fresh();
    assert!(!spare.on_destroyed("main"), "풀과 무관한 창");
}

// ── 창 역할 · 재기 받기 ──

#[test]
fn the_exit_mark_stays_once_set() {
    let (spare, _) = fresh();
    assert!(!spare.exiting());
    spare.begin_exit();
    spare.begin_exit();
    assert!(spare.exiting());
}

#[test]
fn a_pool_window_is_spare_without_asking_the_model() {
    let (spare, _) = fresh();
    let label = built(&spare, &Labels::default());
    let role = spare.window_role(&label, || {
        panic!("풀의 창은 모델을 묻지 않는다");
    });
    assert_eq!(role, WindowRole::Spare);
}

#[test]
fn a_window_outside_the_pool_is_model_or_unknown() {
    let (spare, _) = fresh();
    assert_eq!(spare.window_role("main", || true), WindowRole::Model);
    assert_eq!(
        spare.window_role("slot-popup-9", || false),
        WindowRole::Unknown
    );
}

#[test]
fn the_model_question_runs_with_the_pool_unlocked() {
    let (spare, _) = fresh();
    let role = spare.window_role("main", || {
        assert!(spare.pool.try_lock().is_ok(), "풀 락을 쥔 채 묻지 않는다");
        true
    });
    assert_eq!(role, WindowRole::Model);
}

#[test]
fn a_report_for_a_window_outside_the_pool_goes_to_the_model() {
    let (spare, _) = fresh();
    let (layout, view) = model_with("slot-popup-7");
    spare
        .report_canvas(&layout, "slot-popup-7", 640, 480)
        .unwrap();
    spare
        .report_metrics(&layout, "slot-popup-7", metrics(40))
        .unwrap();
    assert_eq!(
        model_measures(&layout, view),
        Some((CanvasPx { w: 640, h: 480 }, metrics(40)))
    );
    assert!(spare
        .report_canvas(&layout, "slot-popup-8", 640, 480)
        .is_err());
}
