//! 연결 lifecycle 락 — generation 가드의 TOCTOU 차단 단일 출처 (S14 모듈① T2, ADR-0036).
//!
//! ## 왜 이 파일이 따로 있나 (load-bearing — 동시성 치명)
//! generation 가드(openGen 씨앗, Fix B)의 1차 구현은 `generation: AtomicU64` 하나만 atomic 으로
//! 두고, "내가 current 인가"(load) 와 그에 딸린 공유 상태 변경(watch `state_tx.send` / `cmd_tx`
//! 저장)을 **분리된 두 연산**으로 했다. SeqCst 는 atomic 하나의 전역 순서만 보장할 뿐, **체크 +
//! 변경을 하나로 묶지 못한다**. 그래서:
//!
//! ```text
//!   stale task:  load() == my_gen  ── true (아직 current)
//!                ── 여기서 preempt ──
//!   다른 스레드:  close()/connect() 가 generation bump + state 갈아끼움
//!   stale task:  state_tx.send(...) ── stale 인데 current 의 상태를 clobber!
//! ```
//!
//! tokio multi-thread 에서 `close()`(동기, 임의 스레드)가 연결 task(워커 스레드)와 진짜 병행하므로
//! reachable 한 TOCTOU 다. SeqCst 만으로는 못 막는다(Codex blind 적출, 메인 확인).
//!
//! ## 해법 — "체크 + 변경" 을 한 락 아래 원자화
//! `generation`(plain u64 로 강등) · `cmd_tx`(Option<Sender>) · watch `state_tx` **를 하나의
//! `Mutex<Lifecycle>` 아래로** 통합한다. 가드된 모든 전이는 이 모듈의 메서드 한 곳을 통과한다:
//!   - `bump_and_capture()` — 세대를 올리고 새 my_gen 을 캡처(connect/ensure 진입).
//!   - `publish_if_current(my_gen, state)` — 락 잡고 `gen == my_gen` 비교 → 맞을 때만 watch send.
//!   - `store_cmd_if_current(my_gen, tx)` — 락 잡고 비교 → 맞을 때만 cmd_tx 저장(좀비 sender 차단).
//!   - `close()` — 락 잡고 bump + cmd_tx=None + Down send(셋 다 원자).
//! 비교와 변경이 같은 critical section 안이라, 그 사이 다른 스레드가 세대를 못 바꾼다 → clobber 불가.
//!
//! ## ★ADR-0006 불변식 — 락을 .await across 보유 금지★
//! 이 락의 critical section 은 **순수 동기 코드만** 담는다. watch `send`·cmd_tx 교체·u64 비교/증가는
//! 전부 동기라 락 안에서 OK. 소켓 `sink.close().await`·`stream.next().await`·task `spawn` 등 await 는
//! 반드시 락 해제 후(메서드가 반환해 guard 가 drop 된 뒤) 호출한다. 이 모듈의 어떤 메서드도 내부에서
//! `.await` 를 하지 않는다(전부 `&self` 동기 메서드) — 그래서 호출자가 락을 await 너머로 들 수 없다.
//!
//! ## loom 도입 가능성
//! 결정론적 인터리빙 검증(loom)은 이 TOCTOU 류 결함의 정석 도구다. 현재는 무게(loom 전용 atomic/sync
//! 추상화 도입 + std 동시 유지) 때문에 스트레스 반복 테스트(tests.rs `*_stress`)로 확률적 회귀 검출만
//! 깐다. lifecycle 을 loom 의 `loom::sync::Mutex` 로 추상화하면(cfg(loom) feature) 결정론적으로 이
//! 락의 원자성을 증명할 수 있다 — 동시성 표면이 더 커지는 T4(재연결·백오프) 합류 시 재검토 가치 높음.

use std::sync::Mutex;

use tokio::sync::{mpsc, watch};

use super::connection::ConnectionCommand;
use super::ConnectionState;

/// generation 가드의 단일 진실원. `Arc<Lifecycle>` 로 DaemonClient·연결 task 가 공유한다.
///
/// ★불변식★: `generation`/`cmd_tx`/`state_tx(전이)` 의 모든 가드된 접근은 `inner`(Mutex) 한 락
/// 아래서 일어난다 — "내가 current 인가" 판정과 그에 딸린 변경이 같은 critical section 이라 원자적이다.
/// `state_rx.borrow()` 빠른 읽기(`DaemonClient::state`)만 락 밖(watch 자체 동기화)이다.
pub(crate) struct Lifecycle {
    inner: Mutex<LifecycleInner>,
}

struct LifecycleInner {
    /// 연결 세대 카운터. 이전 AtomicU64 를 락 안 plain u64 로 강등(단일 출처화) — 비교+증가가
    /// 이제 락 안 동기 연산이라 atomic 필요 없음. bump 는 connect/ensure 진입(`bump_and_capture`)과
    /// close(`close`) 에서만 일어난다.
    generation: u64,
    /// 현재 활성 연결 task 로 가는 명령 채널. None = 연결 task 없음(초기/close 후 / stale 미저장).
    /// ★단일 task 소유★: invoke 는 여기로 ConnectionCommand 만 보내고, 처리는 연결 task 단독(T6).
    cmd_tx: Option<mpsc::Sender<ConnectionCommand>>,
    /// 상태 전이 송신자. **가드된 전이는 반드시 이 락 아래서** 보낸다 — 락 밖에서 보내면 다시
    /// TOCTOU 가 열린다(체크는 락 안, 변경은 락 밖 = 분리). watch send 는 동기라 락 안에서 OK.
    state_tx: watch::Sender<ConnectionState>,
}

impl Lifecycle {
    /// 초기 상태 Down 으로 생성. `state_rx` 는 호출자(DaemonClient)가 빠른 읽기용으로 보관한다.
    pub(crate) fn new() -> (Self, watch::Receiver<ConnectionState>) {
        let (state_tx, state_rx) = watch::channel(ConnectionState::Down);
        (
            Self {
                inner: Mutex::new(LifecycleInner {
                    generation: 0,
                    cmd_tx: None,
                    state_tx,
                }),
            },
            state_rx,
        )
    }

    /// 세대를 올리고 새 my_gen 을 돌려준다(connect/ensure 진입). 선택적으로 같은 락 아래서
    /// `set_state` 전이도 발행한다 — 진입의 "bump + Connecting" 을 한 critical section 으로 묶어,
    /// bump 직후 다른 스레드가 끼어 세대를 또 올리는 창에서도 *내가 올린 세대로* 일관되게 행동한다.
    ///
    /// ★동기★: u64 증가 + watch send 둘 다 동기 → 락 안에서 원자. await 없음.
    pub(crate) fn bump_and_capture(&self, set_state: Option<ConnectionState>) -> u64 {
        let mut g = self.inner.lock().expect("lifecycle poisoned");
        g.generation += 1;
        let my_gen = g.generation;
        if let Some(state) = set_state {
            // 방금 내가 올린 세대를 들고 있는 동안의 전이라 항상 유효(이 락 안에서 누구도 못 바꿈).
            let _ = g.state_tx.send(state);
        }
        my_gen
    }

    /// ★가드된 전이★: 락 잡고 `generation == my_gen` 일 때만 watch 상태를 발행한다. stale(밀려난
    /// 세대)이면 아무것도 안 한다 → current 연결의 상태를 clobber 하지 않는다. 비교와 send 가 같은
    /// critical section 이라, 그 사이 다른 스레드가 세대를 못 바꾼다(TOCTOU 차단).
    ///
    /// 반환: 실제로 발행했으면(=내가 current) true. 호출자가 후속(ready 보고 등) 분기에 쓴다.
    /// ★동기★: watch send 는 동기 → await 없음.
    pub(crate) fn publish_if_current(&self, my_gen: u64, state: ConnectionState) -> bool {
        let g = self.inner.lock().expect("lifecycle poisoned");
        if g.generation == my_gen {
            let _ = g.state_tx.send(state);
            true
        } else {
            false
        }
    }

    /// ★가드된 cmd_tx 저장★: 락 잡고 current 일 때만 sender 를 저장한다. stale 이면 저장하지 않고
    /// false 를 돌려준다 → 호출자가 sender 를 drop(연결 task 의 cmd_rx EOF → 정리)하게 한다. 좀비
    /// sender 부활을 비교+저장 원자화로 차단한다.
    /// ★동기★: Option 교체 → await 없음.
    pub(crate) fn store_cmd_if_current(
        &self,
        my_gen: u64,
        tx: mpsc::Sender<ConnectionCommand>,
    ) -> bool {
        let mut g = self.inner.lock().expect("lifecycle poisoned");
        if g.generation == my_gen {
            g.cmd_tx = Some(tx);
            true
        } else {
            false
        }
    }

    /// 명시 종료(close). 락 잡고 (a)세대 bump (b)cmd_tx=None (c)Down 발행 **을 한 원자 단위로** 한다.
    /// 셋이 같은 critical section 이라, bump 와 Down 사이에 stale task 가 끼어 Connected 를 발행할 수
    /// 없다(끼더라도 그 publish_if_current 는 이미 올라간 세대를 보고 삼킨다). 이 Down 은 close 자신의
    /// 의도라 항상 유효.
    /// ★동기★: bump + Option 교체 + watch send → await 없음.
    pub(crate) fn close(&self) {
        let mut g = self.inner.lock().expect("lifecycle poisoned");
        g.generation += 1;
        g.cmd_tx = None;
        let _ = g.state_tx.send(ConnectionState::Down);
    }

    /// 현재 세대 스냅샷(테스트용 — 가드 판정의 단위 검증). 운영 코드는 my_gen 캡처값으로 비교한다.
    #[cfg(test)]
    pub(crate) fn current_generation(&self) -> u64 {
        self.inner.lock().expect("lifecycle poisoned").generation
    }
}
