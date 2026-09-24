//! 세션 id **첫 제출 래치** — 화신 하나에 하나.
//!
//! 입력 둘이 **둘 다** 일어난 첫 순간에 commit 포트를 한 번 부른다. 순서는 상관없다.
//! - [`SessionIdLatch::offer`] — 이 화신의 세션 id 가 알려졌다(backend·통로는 [`SessionIdLatch::offer_sink`] 로 받는다).
//! - [`SessionIdLatch::note_submission`] — 이 화신의 첫 사용자 턴이 상대에게 **나가기 직전**이다.
//!
//! 제출 없이 화신이 끝나면 포트는 한 번도 안 불린다 — 「저장된 id 가 있다 ⟺ 이어받을 대화가 있다」가
//! 여기서 선다.
//!
//! ## 불변식
//! - ★포트는 화신당 많아야 한 번 불린다. 한 번 부르기 시작했으면(성공·거절·패닉 무엇으로 끝나든) 다시
//!   부르지 않는다★. 그래서 `committed` 를 포트 호출 **전에** 세운다. 포트는 반환이 없어 결과가 래치에
//!   돌아오지 않고, 같은 입력이면 답도 같으므로 재시도할 것이 없다.
//! - offer 는 화신당 많아야 한 번이다(기록 포트 [`SessionIdSink`] 의 계약). 계약 밖의 둘째 offer 는 언제
//!   오든 버린다 — commit 전이어도 첫 값이 이긴다.
//! - ★빠른 길의 표식(`settled`)은 첫 commit 이 **돌아온 뒤에만** 선다 — 「제출이 있었다」로 세우지 말 것★:
//!   두 제출자가 다른 스레드일 때(우편 flush · 연결의 사용자 입력) 둘째가 첫째의 commit 이 끝나기 전에 빠른
//!   길로 빠져 첫 턴이 영속보다 먼저 나간다. settled 전의 제출은 전부 뮤텍스를 잡고 진행 중인 commit 뒤에
//!   줄을 선다.
//! - 래치 뮤텍스를 쥔 채 하는 외부 호출은 commit 포트 하나뿐이다(로그도 놓은 뒤에 쓴다). 락 순서 =
//!   `래치 → 포트의 expected 칸 → profiles → store write_lock` 단방향 — 래치를 잡는 호출자는 다른 락을 쥐지
//!   않은 채로 부른다.
//! - ★`catch_unwind` 를 두지 않는다★: 릴리스는 `panic = "abort"` 라 아무것도 잡지 않는 죽은 코드다. unwind
//!   빌드(개발·시험)에서 포트가 패닉하면 뮤텍스에 독이 들고, 뒤이은 호출은 독을 걷어 내고(`into_inner`)
//!   `committed` 를 보고 통과한다 — 포트를 다시 부르지 않는다.
//!
//! backend·transport·profile 을 모른다 — uuid 해석과 「이 화신이 시작 때 본 값」 추적은 commit 포트 안
//! (조립점) 몫이다.
// ADR-0226

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::backend::SessionIdSink;
use crate::types::AgentId;

pub(crate) struct SessionIdLatch {
    /// 로그 필드 전용.
    agent: AgentId,
    /// 로그 필드 전용 — 화신 가드는 commit 포트가 진다.
    epoch: u32,
    commit: SessionIdSink,
    settled: AtomicBool,
    state: Mutex<LatchState>,
}

#[derive(Default)]
struct LatchState {
    /// 받았지만 제출이 아직 없어 commit 을 보류한 id.
    pending: Option<String>,
    submitted: bool,
    committed: bool,
}

enum OfferOutcome {
    Held,
    Committed,
    Dropped,
}

impl SessionIdLatch {
    pub(crate) fn new(agent: AgentId, epoch: u32, commit: SessionIdSink) -> Arc<Self> {
        Arc::new(Self {
            agent,
            epoch,
            commit,
            settled: AtomicBool::new(false),
            state: Mutex::new(LatchState::default()),
        })
    }

    /// backend·통로에 건네는 기록 포트 — 부르면 [`SessionIdLatch::offer`] 다.
    pub(crate) fn offer_sink(self: &Arc<Self>) -> SessionIdSink {
        let latch = Arc::clone(self);
        Arc::new(move |raw: &str| latch.offer(raw))
    }

    pub(crate) fn offer(&self, raw: &str) {
        let mut state = self.lock_state();
        let outcome = if state.committed || state.pending.is_some() {
            OfferOutcome::Dropped
        } else if state.submitted {
            self.commit_locked(&mut state, raw);
            OfferOutcome::Committed
        } else {
            state.pending = Some(raw.to_owned());
            OfferOutcome::Held
        };
        drop(state);
        match outcome {
            // ★이 줄을 지우지 말 것★: 영속 뒤에는 commit 포트의 반영 로그가 남지만, 0턴 화신에는 이 줄
            //   말고 「id 가 왔다」를 알리는 것이 없다 — 현장 진단과 GUI 실측이 기다리는 사건이다.
            //   id 값은 싣지 않는다.
            OfferOutcome::Held => tracing::info!(
                agent = %self.agent,
                epoch = self.epoch,
                "세션 id 를 받았다 — 첫 제출 전이라 영속을 보류한다"
            ),
            OfferOutcome::Dropped => tracing::debug!(
                agent = %self.agent,
                epoch = self.epoch,
                "이 화신의 둘째 세션 id 를 버린다 — 기록 포트 계약(화신당 한 번) 밖의 호출"
            ),
            OfferOutcome::Committed => {}
        }
    }

    /// 첫 턴이 나가기 **직전**에 부른다. id 를 이미 받았으면 그 자리에서 commit 하고, commit 이 돌아온 뒤에
    /// 돌아온다 — 호출자는 이 호출이 돌아온 **뒤에** 턴을 보내야 「첫 턴 전에 영속」이 선다.
    pub(crate) fn note_submission(&self) {
        if self.settled.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.lock_state();
        if state.committed {
            // 앞선 commit 이 돌아왔거나 패닉으로 끝났다 — 어느 쪽이든 다시 부르지 않는다.
            self.settled.store(true, Ordering::Release);
            return;
        }
        state.submitted = true;
        if let Some(raw) = state.pending.take() {
            self.commit_locked(&mut state, &raw);
        }
    }

    fn commit_locked(&self, state: &mut LatchState, raw: &str) {
        state.committed = true;
        (self.commit)(raw);
        self.settled.store(true, Ordering::Release);
    }

    fn lock_state(&self) -> MutexGuard<'_, LatchState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{mpsc, Barrier};
    use std::thread;
    use std::time::Duration;

    /// 스레드를 쓰는 항목의 모든 기다림에 거는 상한 — ★매달림 방지이지 정확성 문턱이 아니다★. 올바른
    /// 래치는 밀리초 안에 끝나므로 이 값에 닿는 것은 회귀로 영영 안 돌아오는 경우뿐이고, 그때 시험이
    /// 매달리는 대신 실패하게 한다.
    const HANG_GUARD: Duration = Duration::from_secs(10);

    fn recording_port() -> (SessionIdSink, Arc<Mutex<Vec<String>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = calls.clone();
        let port: SessionIdSink =
            Arc::new(move |raw: &str| seen.lock().unwrap().push(raw.to_owned()));
        (port, calls)
    }

    fn latch_with(port: SessionIdSink) -> Arc<SessionIdLatch> {
        SessionIdLatch::new(AgentId::new_v4(), 7, port)
    }

    // ── 한 스레드 순서 ──

    /// ① — 수령 포트(`offer_sink`)를 거쳐도 같은 규칙이다.
    #[test]
    fn an_offer_then_a_submission_commits_the_offered_id_once() {
        let (port, calls) = recording_port();
        let latch = latch_with(port);

        (latch.offer_sink())("sid-1");
        assert!(
            calls.lock().unwrap().is_empty(),
            "제출 전에는 부르지 않는다"
        );
        latch.note_submission();

        assert_eq!(calls.lock().unwrap().as_slice(), ["sid-1"]);
    }

    /// ② — 제출이 먼저 오면 뒤이은 offer 가 그 자리에서 commit 한다.
    #[test]
    fn a_submission_then_an_offer_commits_at_the_offer() {
        let (port, calls) = recording_port();
        let latch = latch_with(port);

        latch.note_submission();
        assert!(calls.lock().unwrap().is_empty());
        latch.offer("sid-1");

        assert_eq!(calls.lock().unwrap().as_slice(), ["sid-1"]);
    }

    /// ③ — 제출 없이 끝난 화신은 아무것도 영속하지 않는다.
    #[test]
    fn an_offer_without_a_submission_commits_nothing() {
        let (port, calls) = recording_port();
        let latch = latch_with(port);

        latch.offer("sid-1");

        assert!(calls.lock().unwrap().is_empty());
        assert!(!latch.settled.load(Ordering::Acquire));
    }

    /// ④
    #[test]
    fn repeated_submissions_commit_once() {
        let (port, calls) = recording_port();
        let latch = latch_with(port);

        latch.note_submission();
        latch.note_submission();
        latch.offer("sid-1");
        for _ in 0..3 {
            latch.note_submission();
        }

        assert_eq!(calls.lock().unwrap().as_slice(), ["sid-1"]);
    }

    /// ⑤ — 「포트는 화신당 많아야 한 번」의 성공 갈래. commit 뒤의 둘째 offer 는 계약 밖이라 버린다.
    #[test]
    fn an_offer_after_a_commit_does_not_call_the_port() {
        let (port, calls) = recording_port();
        let latch = latch_with(port);

        latch.offer("sid-1");
        latch.note_submission();
        latch.offer("sid-2");
        latch.note_submission();

        assert_eq!(calls.lock().unwrap().as_slice(), ["sid-1"]);
    }

    /// ⑪ — commit 전에 온 둘째 offer 도 버린다. 첫 값이 이긴다.
    #[test]
    fn a_second_offer_before_the_commit_is_dropped_and_the_first_wins() {
        let (port, calls) = recording_port();
        let latch = latch_with(port);

        latch.offer("sid-1");
        latch.offer("sid-2");
        latch.note_submission();

        assert_eq!(calls.lock().unwrap().as_slice(), ["sid-1"]);
    }

    /// ⑧ — offer 가 없는 동안(핸드셰이크 중 · 락 회수 전)은 commit 할 것이 없어 빠른 길이 안 선다.
    #[test]
    fn submissions_without_an_offer_never_settle() {
        let (port, calls) = recording_port();
        let latch = latch_with(port);

        for _ in 0..5 {
            latch.note_submission();
        }

        assert!(calls.lock().unwrap().is_empty());
        assert!(!latch.settled.load(Ordering::Acquire));
    }

    /// ⑩ — 「포트는 화신당 많아야 한 번」의 거절 갈래. 포트가 아무것도 적지 않고 돌아와도 래치에는 성공과
    ///   구별되지 않으며(반환이 없다), 빠른 길이 서고 다시 부르지 않는다.
    #[test]
    fn a_commit_that_records_nothing_still_settles_and_is_not_retried() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let refusing: SessionIdSink = Arc::new(move |_raw: &str| {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        let latch = latch_with(refusing);

        latch.offer("sid-1");
        latch.note_submission();
        assert!(latch.settled.load(Ordering::Acquire));
        latch.note_submission();
        latch.offer("sid-2");
        latch.note_submission();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// ⑨ — 「포트는 화신당 많아야 한 번」의 패닉 갈래. ★unwind 빌드에서만 도는 항목이다★ — 릴리스는
    ///   `panic = "abort"` 라 이 갈래 자체가 없다. 뮤텍스에 독이 들어도 뒤이은 호출이 멈추지 않고 포트를
    ///   다시 부르지 않는다.
    #[cfg(panic = "unwind")]
    #[test]
    fn a_commit_that_panics_is_never_called_again() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let panicking: SessionIdSink = Arc::new(move |_raw: &str| {
            counted.fetch_add(1, Ordering::SeqCst);
            panic!("시험대: commit 포트가 패닉한다");
        });
        let latch = latch_with(panicking);

        latch.offer("sid-1");
        let unwound =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| latch.note_submission()));
        assert!(unwound.is_err(), "패닉은 그 호출을 타고 올라간다");
        assert!(latch.state.is_poisoned(), "전제: 뮤텍스에 독이 들었다");

        latch.note_submission();
        latch.offer("sid-2");
        latch.note_submission();

        assert_eq!(calls.load(Ordering::SeqCst), 1, "포트를 다시 불렀다");
        assert!(
            latch.settled.load(Ordering::Acquire),
            "독을 걷어 낸 뒤 빠른 길이 서야 한다"
        );
    }

    // ── 스레드 사이 ──

    /// ⑥ — offer 하나와 제출자 둘이 동시에 달려도 정확히 한 번.
    #[test]
    fn a_concurrent_offer_and_submissions_commit_exactly_once() {
        for round in 0..200 {
            let (port, calls) = recording_port();
            let latch = latch_with(port);
            let start = Arc::new(Barrier::new(3));
            let (done_tx, done_rx) = mpsc::channel::<()>();
            let spawn = |act: fn(&SessionIdLatch)| {
                let latch = latch.clone();
                let start = start.clone();
                let done_tx = done_tx.clone();
                thread::spawn(move || {
                    start.wait();
                    act(&latch);
                    done_tx.send(()).expect("시험 본체");
                })
            };
            // 띄우는 순서를 판마다 돌린다 — 장벽을 마지막에 친 스레드가 먼저 달리는 경향이 있어, 고정
            //   순서면 offer 가 늘 같은 자리에 서고 「offer 가 먼저」 갈래를 거의 안 탄다(실측).
            let mut acts: [fn(&SessionIdLatch); 3] = [
                |l| l.offer("sid-1"),
                |l| l.note_submission(),
                |l| l.note_submission(),
            ];
            acts.rotate_left(round % 3);
            let handles = acts.map(spawn);
            for _ in &handles {
                done_rx.recv_timeout(HANG_GUARD).unwrap_or_else(|e| {
                    panic!("{round}번째 판에서 시험 스레드가 돌아오지 않는다: {e}")
                });
            }
            for h in handles {
                h.join().expect("시험 스레드");
            }
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                ["sid-1"],
                "{round}번째 판에서 commit 횟수가 1 이 아니다"
            );
        }
    }

    /// ⑦ ★빠른 길 경합의 회귀망★ — commit 이 진행 중일 때 다른 스레드의 제출은 그 commit 이 **돌아오기
    ///   전에** 돌아오지 않는다. 돌아오면 그 스레드의 턴이 영속보다 먼저 나간다.
    ///
    /// commit 포트를 채널로 붙잡아 둔다. 둘째 제출자가 붙잡힌 동안 돌아오지 않는지를 한 구간 동안 본다 —
    ///   ★그 구간은 회귀를 잡는 창일 뿐이다★: 올바른 래치는 구간 길이와 무관하게 통과하고, 제출 표식으로
    ///   빠른 길을 세우는 회귀는 마이크로초 안에 돌아와 걸린다.
    #[test]
    fn a_submission_racing_an_in_flight_commit_waits_for_it_to_return() {
        let (entered_tx, entered_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let events = Arc::new(Mutex::new(Vec::<&'static str>::new()));
        let port_events = events.clone();
        let held: SessionIdSink = Arc::new(move |_raw: &str| {
            entered_tx.send(()).expect("시험 본체가 기다린다");
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(HANG_GUARD)
                .expect("시험 본체가 포트를 풀어 주지 않는다");
            port_events.lock().unwrap().push("commit 반환");
        });
        let latch = latch_with(held);
        latch.offer("sid-1");

        let (first_done_tx, first_done_rx) = mpsc::channel::<()>();
        let first = {
            let latch = latch.clone();
            thread::spawn(move || {
                latch.note_submission();
                first_done_tx.send(()).expect("시험 본체");
            })
        };
        entered_rx.recv_timeout(HANG_GUARD).expect(
            "첫 제출이 commit 포트에 들어가지 않는다 — offer 뒤의 제출이 commit 을 안 불렀다",
        );

        let (returned_tx, returned_rx) = mpsc::channel::<()>();
        let second = {
            let latch = latch.clone();
            let events = events.clone();
            thread::spawn(move || {
                latch.note_submission();
                events.lock().unwrap().push("둘째 제출 반환");
                returned_tx.send(()).expect("시험 본체");
            })
        };
        assert!(
            matches!(
                returned_rx.recv_timeout(Duration::from_millis(200)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ),
            "commit 이 붙잡힌 동안 둘째 제출이 돌아왔다 — 첫 턴이 영속 전에 나간다"
        );

        release_tx.send(()).expect("포트");
        first_done_rx
            .recv_timeout(HANG_GUARD)
            .expect("포트를 풀었는데 첫 제출자가 돌아오지 않는다");
        returned_rx
            .recv_timeout(HANG_GUARD)
            .expect("commit 이 돌아왔는데 둘째 제출자가 돌아오지 않는다");
        first.join().expect("첫 제출자");
        second.join().expect("둘째 제출자");
        assert_eq!(
            events.lock().unwrap().as_slice(),
            ["commit 반환", "둘째 제출 반환"]
        );
    }
}
