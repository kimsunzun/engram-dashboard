//! 끌기 세션의 실행기(TRD S21-slot-docking §2-4) — 사건 줄 · 메인 스레드 비우기 · 시한 · 포인터 폴러. Tauri ·
//! platform 을 모른다 — 메인 스레드로 일을 올리는 길([`MainThread`])과 입력 장치([`InputSource`])는 어댑터가 채운다.
//!
//! - 줄([`Inbox`])은 아무 스레드나 복제해 쥐는 공유 손잡이다. 줄 자물쇠는 넣기 · 꺼내기 한 번씩만 쥐고 사건 처리
//!   동안 쥐지 않는다.
//! - 받는 쪽(세션 + 포트 — [`Driver`])은 [`Inbox::install`] 을 부른 스레드(메인)의 스레드 지역 칸에만 산다 — `Send`
//!   가 아니어도 되고(포트가 창 핸들을 쥐어도 된다), 다른 스레드에서 돌 길이 타입으로 막힌다.
//! - [`Inbox::post`] 하나가 입구다 — 받는 쪽을 세운 스레드에서 부르면 넣고 곧바로 비우고, 다른 스레드면 넣고
//!   [`MainThread`] 로 비우기를 올린다.
//! - 비우기는 사건을 하나씩 끝까지 처리한다. 처리 도중(포트 안 · 그 아래 OS 가 동기로 부른 알림 — 이동 루프 끝은
//!   `begin_move_loop` 나 루프 도는 창을 부수는 호출 안에서도 온다)에 넣은 사건은 줄 끝에 서고, 지금 사건이 끝난 뒤
//!   같은 비우기에서 처리된다. 처리 도중에 불린 비우기(재진입 · 중첩 메시지 펌프가 돌린 예약)는 아무것도 하지 않고
//!   돌아온다.
// ADR-0295

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use engram_dashboard_base::sync;

use super::ports::Ports;
use super::session::{Drag, Event};
use super::units::PhysPoint;

/// 포인터 폴러가 커서 · 주 버튼을 읽는 주기.
pub const POLL_PERIOD_MS: u64 = 8;

/// 메인 스레드로 올릴 일.
pub type Job = Box<dyn FnOnce() + Send>;

/// 메인 스레드에 일을 올리는 길 — 어댑터가 Tauri `AppHandle::run_on_main_thread` 로 채운다. 올린 일은 메인 스레드에서
/// 돈다 — 도는 차례는 약속하지 않는다(메인 스레드에서 올리면 그 자리에서 돈다 · 사건 차례는 줄이 지킨다). 올리지
/// 못하면(이벤트 루프가 닫혔다) 그 일은 버리고 어댑터가 로그를 남긴다 — 넣은 사건은 다음 비우기까지 줄에 남는다.
pub trait MainThread: Send + Sync + 'static {
    fn run(&self, job: Job);
}

/// 비우기가 사건을 하나씩 넘기는 받는 쪽 — 운영은 [`Driver`].
pub trait Handler {
    fn handle(&mut self, ev: Event);
}

/// 운영 받는 쪽 — 끌기 세션과 그 포트.
pub struct Driver<P> {
    drag: Drag,
    ports: P,
}

impl<P: Ports> Driver<P> {
    pub fn new(ports: P) -> Self {
        Self {
            drag: Drag::new(),
            ports,
        }
    }
}

impl<P: Ports> Handler for Driver<P> {
    fn handle(&mut self, ev: Event) {
        self.drag.handle(ev, &mut self.ports);
    }
}

/// 사건 줄 — 복제해 아무 스레드에서나 쥔다(Tauri 상태 · 명령 · 시한 · 폴러 · 포트). 받는 쪽은 하나이고 그것을 세운
/// 스레드에서만 처리된다(모듈 머리).
#[derive(Clone)]
pub struct Inbox {
    shared: Arc<Shared>,
}

struct Shared {
    queue: Mutex<VecDeque<Event>>,
    main: Box<dyn MainThread>,
}

impl Inbox {
    pub fn new(main: impl MainThread) -> Self {
        Self {
            shared: Arc::new(Shared {
                queue: Mutex::new(VecDeque::new()),
                main: Box::new(main),
            }),
        }
    }

    /// 지금 스레드(메인 — Tauri `setup`)에 이 줄의 받는 쪽을 세운다. 한 스레드에 하나다 — 다시 세우면 앞 것을 갈아
    /// 끼운다.
    pub fn install(&self, handler: impl Handler + 'static) {
        let exec: Rc<dyn Drain> = Rc::new(Executor {
            shared: Arc::clone(&self.shared),
            handler: RefCell::new(handler),
        });
        INSTALLED.with(|slot| *slot.borrow_mut() = Some(exec));
    }

    /// 사건을 줄 끝에 넣는다 — 받는 쪽을 세운 스레드면 곧바로 비우고(처리 도중이면 넣기만 — 지금 사건 뒤 같은
    /// 비우기에서 처리된다), 다른 스레드면 [`MainThread`] 로 비우기를 올린다.
    ///
    /// ★메인 스레드에서 비우기가 돌지 않을 때 부르면 세션이 이 호출 안에서 돈다★(창 사건 처리기 · 이동 루프 끝 알림) —
    /// 세션은 포트로 레이아웃 락 · 명부 락을 잡고 Tauri 를 부르므로, 레이아웃 락 · 명부 락 · Tauri 리스너 자물쇠를 쥔
    /// 채 부르지 않는다.
    // ADR-0295
    pub fn post(&self, ev: Event) {
        sync::lock(&self.shared.queue).push_back(ev);
        match installed_for(&self.shared) {
            Some(exec) => exec.drain(),
            None => {
                let inbox = self.clone();
                self.shared.main.run(Box::new(move || inbox.drain_here()));
            }
        }
    }

    fn drain_here(&self) {
        match installed_for(&self.shared) {
            Some(exec) => exec.drain(),
            None => tracing::debug!("이 스레드에 끌기 실행기가 없다 — 사건은 줄에 남는다"),
        }
    }
}

// 꺼내기를 함수로 둔 것은 줄 자물쇠가 처리 동안 살지 않게다 — `while let Some(ev) = sync::lock(..).pop_front()` 로
//   쓰면 가드가 고리 몸통 내내 살아, 포트 안에서 넣는 사건이 같은 스레드의 그 자물쇠에서 막힌다.
fn pop(shared: &Shared) -> Option<Event> {
    sync::lock(&shared.queue).pop_front()
}

struct Executor<H> {
    shared: Arc<Shared>,
    handler: RefCell<H>,
}

trait Drain {
    fn serves(&self, shared: &Arc<Shared>) -> bool;
    fn drain(&self);
}

impl<H: Handler> Drain for Executor<H> {
    fn serves(&self, shared: &Arc<Shared>) -> bool {
        Arc::ptr_eq(&self.shared, shared)
    }

    // 받는 쪽 빌림이 곧 「비우는 중」 표시다 — 처리 도중의 비우기는 빌리지 못해 그대로 돌아오고, 그동안 든 사건은
    //   바깥 고리가 꺼낸다.
    // ADR-0295
    fn drain(&self) {
        let Ok(mut handler) = self.handler.try_borrow_mut() else {
            return;
        };
        while let Some(ev) = pop(&self.shared) {
            handler.handle(ev);
        }
    }
}

thread_local! {
    static INSTALLED: RefCell<Option<Rc<dyn Drain>>> = const { RefCell::new(None) };
}

// 칸 빌림은 꺼내 오는 동안만이다 — 비우기는 그 뒤에 돈다.
fn installed_for(shared: &Arc<Shared>) -> Option<Rc<dyn Drain>> {
    INSTALLED.with(|slot| {
        slot.borrow()
            .as_ref()
            .filter(|exec| exec.serves(shared))
            .cloned()
    })
}

/// 시한 — 스레드 하나가 걸린 시한을 모두 쥐고 때가 되면 [`Inbox::post`] 로 넣는다. 앞 시한을 지우지 않는다(받는 쪽이
/// 번호로 가른다 — `Ports::arm_raise`). 이 값을 버리면 스레드가 그치고 아직 오지 않은 시한도 버린다.
pub struct Timer {
    arms: Sender<(Instant, Event)>,
}

impl Timer {
    pub fn new(inbox: Inbox) -> io::Result<Self> {
        let (arms, rx) = mpsc::channel();
        // 시한 하나에 스레드 하나를 띄우지 않는다 — 앞으로 올리기 시한은 이동 루프 동안 `Moved` 마다 걸린다.
        thread::Builder::new()
            .name("dock-timer".into())
            .spawn(move || run_timer(&rx, &inbox))?;
        Ok(Self { arms })
    }

    /// `after` 뒤 `ev` 를 줄에 넣는다 — 때는 부른 때부터 잰다.
    pub fn arm(&self, after: Duration, ev: Event) {
        if self.arms.send((Instant::now() + after, ev)).is_err() {
            tracing::debug!("시한 스레드가 없다 — 시한을 버린다");
        }
    }
}

// 걸린 시한을 때 순으로 쥔다(같은 때면 건 차례).
fn run_timer(rx: &Receiver<(Instant, Event)>, inbox: &Inbox) {
    let mut armed: Vec<(Instant, Event)> = Vec::new();
    loop {
        let got = match armed.first() {
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            Some((at, _)) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
        };
        match got {
            Ok((at, ev)) => {
                let i = armed.partition_point(|(a, _)| *a <= at);
                armed.insert(i, (at, ev));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();
        let due = armed.partition_point(|(at, _)| *at <= now);
        for (_, ev) in armed.drain(..due) {
            inbox.post(ev);
        }
    }
}

/// 폴러가 읽는 입력 장치 — 어댑터가 platform `input`(`cursor_pos` · `primary_button_down`)으로 채운다. 폴러 스레드에서
/// 불린다.
pub trait InputSource: Send + Sync + 'static {
    fn cursor(&self) -> PhysPoint;
    fn primary_down(&self) -> bool;
}

/// 포인터 폴러 — [`POLL_PERIOD_MS`] 마다 커서 · 주 버튼을 읽어 `Event::Pointer` 를 넣는다(넣는 규칙 =
/// `Ports::poll_start`). 도는 폴러는 하나다 — 새 시작 · 멈춤 · 이 값 버리기가 앞 폴러를 그치게 한다(그 스레드는 다음
/// 표본 전에 그친다 — 그 사이 넣은 옛 표본은 세션이 세대 · 단계로 버린다).
pub struct Poller {
    inbox: Inbox,
    input: Arc<dyn InputSource>,
    run: Arc<AtomicU64>,
}

impl Poller {
    pub fn new(inbox: Inbox, input: Arc<dyn InputSource>) -> Self {
        Self {
            inbox,
            input,
            run: Arc::new(AtomicU64::new(0)),
        }
    }

    /// `label` = 그 끌기를 쥔 창(로그 몫).
    // ADR-0295
    pub fn start(&self, label: &str, gen: u32) {
        let mine = self.run.fetch_add(1, Ordering::SeqCst) + 1;
        let run = Arc::clone(&self.run);
        let inbox = self.inbox.clone();
        let input = Arc::clone(&self.input);
        let spawned = thread::Builder::new()
            .name("dock-poller".into())
            .spawn(move || {
                let mut last = None;
                while run.load(Ordering::SeqCst) == mine {
                    let sample = (input.cursor(), input.primary_down());
                    if last != Some(sample) {
                        last = Some(sample);
                        inbox.post(Event::Pointer {
                            gen,
                            cursor: sample.0,
                            down: sample.1,
                        });
                    }
                    thread::sleep(Duration::from_millis(POLL_PERIOD_MS));
                }
            });
        match spawned {
            Ok(_) => tracing::debug!(window = %label, gen, "포인터 폴러 시작"),
            Err(e) => tracing::warn!(window = %label, gen, "포인터 폴러 스레드를 못 띄웠다: {e}"),
        }
    }

    pub fn stop(&self) {
        self.run.fetch_add(1, Ordering::SeqCst);
        tracing::debug!("포인터 폴러 멈춤");
    }
}

impl Drop for Poller {
    fn drop(&mut self) {
        self.run.fetch_add(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock::session::LoopEnd;

    // 메인 스레드 흉내 — 올린 일을 시험 스레드(받는 쪽을 세운 스레드)가 꺼내 돌린다.
    struct TestMain(Sender<Job>);

    impl MainThread for TestMain {
        fn run(&self, job: Job) {
            // 시험이 끝난 뒤 그치는 중인 폴러 · 시한 스레드가 올리는 일은 받을 쪽이 없다.
            self.0.send(job).ok();
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    enum Seen {
        Raise(u32),
        Moved(String),
        LoopEnded(u64, u32),
        Pointer(u32, PhysPoint, bool),
    }

    fn seen(ev: &Event) -> Seen {
        match ev {
            Event::RaiseDue { seq } => Seen::Raise(*seq),
            Event::Moved { label } => Seen::Moved(label.clone()),
            Event::LoopEnded {
                session, loop_no, ..
            } => Seen::LoopEnded(*session, *loop_no),
            Event::Pointer { gen, cursor, down } => Seen::Pointer(*gen, *cursor, *down),
            other => unreachable!("시험이 넣지 않는 사건: {other:?}"),
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    enum Step {
        In(Seen),
        Out(Seen),
        Note(&'static str),
    }

    struct Ctx {
        inbox: Inbox,
        jobs: Receiver<Job>,
        log: RefCell<Vec<Step>>,
    }

    impl Ctx {
        fn note(&self, note: &'static str) {
            self.log.borrow_mut().push(Step::Note(note));
        }

        fn steps(&self) -> Vec<Step> {
            self.log.borrow().clone()
        }

        fn ins(&self) -> Vec<Seen> {
            self.log
                .borrow()
                .iter()
                .filter_map(|s| match s {
                    Step::In(seen) => Some(seen.clone()),
                    _ => None,
                })
                .collect()
        }

        // 올라와 있는 일을 다 돌리고 그 수를 돌려준다.
        fn run_waiting_jobs(&self) -> usize {
            let mut ran = 0;
            for job in self.jobs.try_iter() {
                job();
                ran += 1;
            }
            ran
        }

        // 올라온 일을 돌리며 `until` 이 참이 되기를 기다린다 — 때를 넘기면 `false`.
        fn pump_until(&self, timeout: Duration, until: impl Fn(&Ctx) -> bool) -> bool {
            let deadline = Instant::now() + timeout;
            loop {
                if until(self) {
                    return true;
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return false;
                }
                if let Ok(job) = self.jobs.recv_timeout(left.min(Duration::from_millis(5))) {
                    job();
                }
            }
        }

        fn pump_for(&self, span: Duration) {
            self.pump_until(span, |_| false);
        }
    }

    type During = Box<dyn FnMut(&Seen, &Ctx)>;

    // 사건마다 들어감 · 나옴을 적고, 그 사이에 `during`(포트 호출 흉내)을 돌린다.
    struct Recorder {
        ctx: Rc<Ctx>,
        during: During,
    }

    impl Handler for Recorder {
        fn handle(&mut self, ev: Event) {
            let s = seen(&ev);
            self.ctx.log.borrow_mut().push(Step::In(s.clone()));
            (self.during)(&s, &self.ctx);
            self.ctx.log.borrow_mut().push(Step::Out(s));
        }
    }

    fn rig(during: impl FnMut(&Seen, &Ctx) + 'static) -> Rc<Ctx> {
        let (tx, jobs) = mpsc::channel();
        let ctx = Rc::new(Ctx {
            inbox: Inbox::new(TestMain(tx)),
            jobs,
            log: RefCell::new(Vec::new()),
        });
        ctx.inbox.install(Recorder {
            ctx: Rc::clone(&ctx),
            during: Box::new(during),
        });
        ctx
    }

    fn quiet() -> Rc<Ctx> {
        rig(|_, _| {})
    }

    fn post_from_another_thread(inbox: &Inbox, events: Vec<Event>) {
        let inbox = inbox.clone();
        thread::spawn(move || {
            for ev in events {
                inbox.post(ev);
            }
        })
        .join()
        .unwrap();
    }

    fn whole(s: Seen) -> [Step; 2] {
        [Step::In(s.clone()), Step::Out(s)]
    }

    const WAIT: Duration = Duration::from_secs(5);

    // ── 줄 · 비우기 ──

    #[test]
    fn a_post_from_another_thread_waits_for_the_main_thread_job() {
        let ctx = quiet();
        post_from_another_thread(&ctx.inbox, vec![Event::RaiseDue { seq: 1 }]);
        assert!(ctx.steps().is_empty(), "다른 스레드에서는 처리하지 않는다");
        let job = ctx.jobs.try_recv().expect("비우기를 올렸다");
        job();
        assert_eq!(ctx.steps(), whole(Seen::Raise(1)));
    }

    #[test]
    fn queued_events_run_one_at_a_time_in_arrival_order() {
        let ctx = quiet();
        post_from_another_thread(
            &ctx.inbox,
            (1..=3).map(|seq| Event::RaiseDue { seq }).collect(),
        );
        let first = ctx.jobs.try_recv().unwrap();
        first();
        let expected: Vec<Step> = (1..=3).flat_map(|seq| whole(Seen::Raise(seq))).collect();
        assert_eq!(ctx.steps(), expected, "첫 비우기가 셋을 차례로 끝까지");
        assert_eq!(ctx.run_waiting_jobs(), 2);
        assert_eq!(ctx.steps(), expected, "남은 예약은 빈 줄을 본다");
    }

    #[test]
    fn a_post_on_the_main_thread_drains_at_once() {
        let ctx = quiet();
        ctx.inbox.post(Event::RaiseDue { seq: 4 });
        assert_eq!(ctx.steps(), whole(Seen::Raise(4)));
        assert_eq!(ctx.run_waiting_jobs(), 0, "메인 스레드로 올리지 않는다");
    }

    #[test]
    fn an_event_posted_while_handling_runs_after_it_in_the_same_drain() {
        let ctx = rig(|s, ctx| {
            if *s == Seen::Raise(1) {
                ctx.inbox.post(Event::RaiseDue { seq: 2 });
                ctx.note("넣음");
            }
        });
        ctx.inbox.post(Event::RaiseDue { seq: 1 });
        let mut expected = vec![
            Step::In(Seen::Raise(1)),
            Step::Note("넣음"),
            Step::Out(Seen::Raise(1)),
        ];
        expected.extend(whole(Seen::Raise(2)));
        assert_eq!(ctx.steps(), expected);
        assert_eq!(
            ctx.run_waiting_jobs(),
            0,
            "같은 비우기 — 메인 스레드로 올리지 않는다"
        );
    }

    #[test]
    fn a_drain_called_while_handling_returns_at_once() {
        // 다른 스레드가 넣으며 올린 예약이 처리 도중 중첩 펌프에서 돈다(OS 모달 루프 흉내).
        let ctx = rig(|s, ctx| {
            if *s == Seen::Raise(1) {
                post_from_another_thread(&ctx.inbox, vec![Event::RaiseDue { seq: 9 }]);
                let job = ctx.jobs.try_recv().unwrap();
                job();
                ctx.note("중첩 비우기 돌아옴");
            }
        });
        ctx.inbox.post(Event::RaiseDue { seq: 1 });
        let mut expected = vec![
            Step::In(Seen::Raise(1)),
            Step::Note("중첩 비우기 돌아옴"),
            Step::Out(Seen::Raise(1)),
        ];
        expected.extend(whole(Seen::Raise(9)));
        assert_eq!(ctx.steps(), expected);
    }

    #[test]
    fn a_loop_end_delivered_inside_the_port_call_is_handled_after_it() {
        // `begin_move_loop` 안에서 platform 이 끝 알림을 동기로 부른다 — 어댑터의 알림 받이는 줄에 넣기만 한다.
        let ctx = rig(|s, ctx| {
            if *s == Seen::Moved("w".to_owned()) {
                let inbox = ctx.inbox.clone();
                let on_end = move |end: LoopEnd| {
                    inbox.post(Event::LoopEnded {
                        session: 7,
                        loop_no: 3,
                        end,
                        cursor: PhysPoint { x: 1, y: 2 },
                    });
                };
                on_end(LoopEnd::Destroyed);
                ctx.note("포트 돌아옴");
            }
        });
        ctx.inbox.post(Event::Moved {
            label: "w".to_owned(),
        });
        let mut expected = vec![
            Step::In(Seen::Moved("w".to_owned())),
            Step::Note("포트 돌아옴"),
            Step::Out(Seen::Moved("w".to_owned())),
        ];
        expected.extend(whole(Seen::LoopEnded(7, 3)));
        assert_eq!(ctx.steps(), expected);
        assert_eq!(ctx.run_waiting_jobs(), 0);
    }

    // ── 시한 ──

    #[test]
    fn an_armed_event_arrives_once_after_its_delay() {
        let ctx = quiet();
        let timer = Timer::new(ctx.inbox.clone()).unwrap();
        let armed = Instant::now();
        timer.arm(Duration::from_millis(40), Event::RaiseDue { seq: 5 });
        assert!(ctx.pump_until(WAIT, |c| !c.ins().is_empty()));
        assert!(
            armed.elapsed() >= Duration::from_millis(40),
            "때 전에 오지 않는다"
        );
        ctx.pump_for(Duration::from_millis(100));
        assert_eq!(ctx.ins(), [Seen::Raise(5)]);
    }

    #[test]
    fn timers_fire_in_deadline_order() {
        let ctx = quiet();
        let timer = Timer::new(ctx.inbox.clone()).unwrap();
        timer.arm(Duration::from_millis(80), Event::RaiseDue { seq: 1 });
        timer.arm(Duration::from_millis(20), Event::RaiseDue { seq: 2 });
        assert!(ctx.pump_until(WAIT, |c| c.ins().len() == 2));
        assert_eq!(ctx.ins(), [Seen::Raise(2), Seen::Raise(1)]);
    }

    // ── 폴러 ──

    struct FakeInput(Mutex<(PhysPoint, bool)>);

    impl FakeInput {
        fn set(&self, cursor: PhysPoint, down: bool) {
            *sync::lock(&self.0) = (cursor, down);
        }
    }

    impl InputSource for FakeInput {
        fn cursor(&self) -> PhysPoint {
            sync::lock(&self.0).0
        }

        fn primary_down(&self) -> bool {
            sync::lock(&self.0).1
        }
    }

    const A: PhysPoint = PhysPoint { x: 10, y: 20 };
    const B: PhysPoint = PhysPoint { x: 11, y: 20 };

    fn poller(ctx: &Ctx) -> (Poller, Arc<FakeInput>) {
        let input = Arc::new(FakeInput(Mutex::new((A, true))));
        let poller = Poller::new(
            ctx.inbox.clone(),
            Arc::clone(&input) as Arc<dyn InputSource>,
        );
        (poller, input)
    }

    // 앞 폴러가 그치는 데 넉넉한 때 — 주기의 몇십 배(바쁜 러너에서 스레드가 늦게 깨도).
    const SETTLE: Duration = Duration::from_millis(150);

    #[test]
    fn the_first_sample_arrives_without_a_change_and_then_only_changes() {
        let ctx = quiet();
        let (poller, input) = poller(&ctx);
        poller.start("w", 3);
        assert!(ctx.pump_until(WAIT, |c| c.ins().len() == 1));
        ctx.pump_for(SETTLE);
        assert_eq!(
            ctx.ins(),
            [Seen::Pointer(3, A, true)],
            "같은 표본은 다시 넣지 않는다"
        );

        input.set(B, true);
        assert!(ctx.pump_until(WAIT, |c| c.ins().len() == 2));
        input.set(B, false);
        assert!(ctx.pump_until(WAIT, |c| c.ins().len() == 3));
        ctx.pump_for(SETTLE);
        assert_eq!(
            ctx.ins(),
            [
                Seen::Pointer(3, A, true),
                Seen::Pointer(3, B, true),
                Seen::Pointer(3, B, false),
            ]
        );
    }

    #[test]
    fn a_stopped_poller_puts_nothing_more() {
        let ctx = quiet();
        let (poller, input) = poller(&ctx);
        poller.start("w", 1);
        assert!(ctx.pump_until(WAIT, |c| c.ins().len() == 1));
        poller.stop();
        ctx.pump_for(SETTLE);
        input.set(B, false);
        ctx.pump_for(SETTLE);
        assert_eq!(ctx.ins(), [Seen::Pointer(1, A, true)]);
    }

    #[test]
    fn a_new_start_retires_the_old_poller_and_delivers_its_own_first_sample() {
        let ctx = quiet();
        let (poller, input) = poller(&ctx);
        poller.start("w", 1);
        assert!(ctx.pump_until(WAIT, |c| c.ins().len() == 1));
        poller.start("w2", 2);
        assert!(
            ctx.pump_until(WAIT, |c| c.ins().len() == 2),
            "바뀌지 않았어도 새 폴러의 첫 표본은 온다"
        );
        ctx.pump_for(SETTLE);
        input.set(B, true);
        assert!(ctx.pump_until(WAIT, |c| c.ins().len() == 3));
        ctx.pump_for(SETTLE);
        assert_eq!(
            ctx.ins(),
            [
                Seen::Pointer(1, A, true),
                Seen::Pointer(2, A, true),
                Seen::Pointer(2, B, true),
            ],
            "앞 폴러는 바뀐 표본을 넣지 않는다"
        );
    }
}
