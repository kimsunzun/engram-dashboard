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
//! `Mutex<Lifecycle>` 아래로** 통합한다. 가드된 모든 전이는 이 모듈의 메서드 한 곳을 통과한다.
//! 비교와 변경이 같은 critical section 안이라, 그 사이 다른 스레드가 세대를 못 바꾼다 → clobber 불가.
//!
//! ## ★ADR-0006 불변식 — 락을 .await across 보유 금지★
//! 이 락의 critical section 은 **순수 동기 코드만** 담는다. watch `send`·cmd_tx 교체·u64 비교/증가는
//! 전부 동기라 락 안에서 OK. 소켓 `sink.close().await`·`stream.next().await`·task `spawn` 등 await 는
//! 반드시 락 해제 후(메서드가 반환해 guard 가 drop 된 뒤) 호출한다. 이 모듈의 어떤 메서드도 내부에서
//! `.await` 를 하지 않는다(전부 `&self` 동기 메서드) — 그래서 호출자가 락을 await 너머로 들 수 없다.
//!
//! ## 계측 위치 (관찰성)
//! 이 모듈의 메서드는 가드 판정 결과(bool)를 **호출자**에게 돌려주고, stale 폐기·전이 로그는
//! 호출자(connection.rs run_connection / mod.rs start_connection·close)가 my_gen·맥락과 함께
//! 남긴다 — flat event(컨벤션 §형식, span 미사용)를 유지하고 같은 가드 발동을 lifecycle/호출자
//! 양쪽에서 이중 로깅하지 않으려는 의도다. 그래서 이 파일 자체엔 tracing 호출이 없다.
//!
//! ## loom 도입 가능성
//! 결정론적 인터리빙 검증(loom)은 이 TOCTOU 류 결함의 정석 도구다. 현재는 ① 결정론적 단위 테스트
//! (tests.rs `guard_*`)로 가드의 *논리 계약*(stale→거부, current→허용)을 검증하고 ② 실 소켓 race 의
//! 통합 wiring 은 single-shot 결정론 회귀 테스트가 커버한다. 다만 비교+변경의 *원자성*(동시 스레드에서
//! 진짜 안 깨짐) 자체의 결정론 증명은 아직 없다 — 그건 무게(loom 전용 atomic/sync 추상화 도입 + std 동시
//! 유지) 때문에 보류 중이다(저ROI 판단:
//! docs/research/toctou-concurrency-test-verification-research-2026-06-28.md). lifecycle 을 loom 의
//! `loom::sync::Mutex` 로 추상화하면(cfg(loom) feature) 이 락의 원자성을 결정론적으로 증명할 수 있다 —
//! 재연결·백오프·in-flight 취소(T4)가 합류해 동시성 표면이 이미 커졌으므로 재검토 가치가 높다.

use std::sync::Mutex;

use tokio::sync::{mpsc, watch};

use super::connection::ConnectionCommand;
use super::ConnectionState;

// `inner` 락 밖에서 일어나는 유일한 접근 = `state_rx.borrow()` 빠른 읽기(`DaemonClient::state` —
// watch 자체 동기화).
pub(crate) struct Lifecycle {
    inner: Mutex<LifecycleInner>,
}

struct LifecycleInner {
    // plain u64 — 비교+증가가 이 락 안 동기 연산이라 atomic 이 필요 없다.
    generation: u64,
    // ★단일 task 소유★: invoke 는 여기로 ConnectionCommand 만 보내고, 처리는 연결 task 단독(T6).
    cmd_tx: Option<mpsc::Sender<ConnectionCommand>>,
    state_tx: watch::Sender<ConnectionState>,
    /// ★재연결 취소 신호(T4 — in-flight 취소 결함 수정)★. generation bump(승계 connect/ensure ·
    /// close)마다 새 generation 값을 send 해, 진행 중인 재연결 task 의 await 를 즉시 깨운다 —
    /// close/승계 후 stale task 가 소켓을 점유·통신하는 창을 닫는 게 목적이다(Codex 적출).
    /// ★Notify 가 아니라 watch★: (a) cancel-safe(select! 의 다른 arm 이 이기면 changed() 는 부작용
    /// 없이 버려짐) (b) **마지막 값을 보존**해 늦게 구독한 reader 도 borrow 로 *현재 generation 값
    /// 자체*는 읽을 수 있다(Notify 는 값이 없어 "현재 무엇인지"를 못 본다).
    cancel_tx: watch::Sender<u64>,
    // ★closedByUser 가드(T4 — wsTransport `closedByUser` 대응)★. true 면 재연결 루프가 즉시 멈춘다
    // (끊김으로 재연결하지 않음) — 명령/재연결이 데몬을 respawn 하면 안 된다는 ADR-0021 의
    // task-lifetime 판(꺼진 채 유지, 복구는 명시 connect 로만).
    closed_by_user: bool,
}

// 재연결 루프 1틱의 가드 판정(원자 스냅샷).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReconnectVerdict {
    // 내가 current + 사용자가 안 닫음 → 재연결 시도/백오프 진행.
    Proceed,
    // stale(더 새 connect/close 가 세대를 올림) 또는 사용자 close → 재연결 중단(조용히 종료).
    Stop,
}

impl Lifecycle {
    pub(crate) fn new() -> (Self, watch::Receiver<ConnectionState>) {
        let (state_tx, state_rx) = watch::channel(ConnectionState::Down);
        let (cancel_tx, _cancel_rx) = watch::channel(0u64);
        (
            Self {
                inner: Mutex::new(LifecycleInner {
                    generation: 0,
                    cmd_tx: None,
                    state_tx,
                    cancel_tx,
                    closed_by_user: false,
                }),
            },
            state_rx,
        )
    }

    // 세대를 올리고 새 my_gen 을 돌려준다(connect/ensure 진입). 선택적으로 같은 락 아래서
    // `set_state` 전이도 발행한다 — 진입의 "bump + Connecting" 을 한 critical section 으로 묶어,
    // bump 직후 다른 스레드가 끼어 세대를 또 올리는 창에서도 *내가 올린 세대로* 일관되게 행동한다.
    //
    // ★closedByUser 해제(T4)★: 명시 connect/ensure 진입은 사용자가 다시 살리려는 의도이므로 같은 락
    // 아래서 `closed_by_user=false` 로 되돌린다(wsTransport start() 의 `closedByUser=false` 와 동형) —
    // 이전 close 로 멈춘 재연결을 부활시킬 수 있게.
    //
    // ★stale cmd_tx 정리(T4 — Codex FIX)★: 승계가 일어나면 옛 연결의 cmd_tx 는 더 이상 유효하지
    // 않으므로 **같은 락 안에서 None 으로 비운다**. 이걸 안 하면 새 connect 핸드셰이크가 끝나(새
    // cmd_tx 를 store_cmd_if_current 로 덮어쓰)기 전까지 옛(stale) 명령채널이 살아 있어, 그 창에
    // 들어온 invoke 가 *죽어가는 옛 연결* 로 명령을 보낼 수 있다. Sender(옛 cmd_tx)를 여기서 drop
    // 하면 옛 연결 task 의 cmd_rx 가 EOF → main_loop 가 Closed 로 종료(재연결 안 함) → 옛 소켓 정리.
    pub(crate) fn bump_and_capture(&self, set_state: Option<ConnectionState>) -> u64 {
        let mut g = self.inner.lock().expect("lifecycle poisoned");
        g.generation += 1;
        let my_gen = g.generation;
        g.closed_by_user = false;
        g.cmd_tx = None;
        let _ = g.cancel_tx.send(g.generation);
        if let Some(state) = set_state {
            let _ = g.state_tx.send(state);
        }
        my_gen
    }

    // ★가드된 전이★: `generation == my_gen` 일 때만 watch 상태를 발행한다. stale(밀려난 세대)이면
    // 아무것도 안 해 current 연결의 상태를 clobber 하지 않는다. 반환 true = 내가 current 라 발행했다.
    pub(crate) fn publish_if_current(&self, my_gen: u64, state: ConnectionState) -> bool {
        let g = self.inner.lock().expect("lifecycle poisoned");
        if g.generation == my_gen {
            let _ = g.state_tx.send(state);
            true
        } else {
            false
        }
    }

    // ★판정만 하는 형제 — 발행하지 않는다★: `generation == my_gen` 인지만 보고 돌려준다.
    //
    // ★왜 `publish_if_current` 로 대신하면 안 되나★: 전이를 **이미 발행한** 자리(`close_locked` 의 Down)가
    // 락 밖 발화 직전에 세대만 재확인할 때, 그것으로 대신하면 같은 종료가 watch 에 **두 번** 실린다.
    // tokio `watch` 는 값이 같아도 버전을 올리므로(값 비교는 `send_if_modified` 몫), 두 send 사이에 깨어
    // 있던 구독자는 종료 한 번을 `changed()` 두 번으로 본다. 이 자리에 남아 있는 물음은 「발행할까」가
    // 아니라 「발화할까」뿐이므로, 그 물음만 답하는 연산을 따로 둔다.
    //
    // ★불변식★: 락은 이 호출 안에서 잡았다 푼다 — 반환 뒤의 발화는 락 밖 외부 호출이다(ADR-0006).
    // 비교는 일치/불일치만(ADR-0163 — 대소로 "더 새 것" 을 유도하지 않는다).
    pub(crate) fn is_current(&self, my_gen: u64) -> bool {
        self.inner.lock().expect("lifecycle poisoned").generation == my_gen
    }

    // ★가드된 cmd_tx 저장★: current 일 때만 sender 를 저장한다. stale 이면 저장하지 않고 false 를
    // 돌려준다 → 호출자가 sender 를 drop(연결 task 의 cmd_rx EOF → 정리)해야 한다.
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

    // 명시 종료(close). 세대 bump·cmd_tx 비움·closed_by_user·Down 발행이 한 락 안이라, bump 와 Down
    // 사이에 stale task 가 끼어 Connected 를 발행할 수 없다(끼더라도 그 publish_if_current 는 이미
    // 올라간 세대를 보고 삼킨다). 이 Down 은 close 자신의 의도라 가드 없이 항상 유효.
    //
    // ★closed_by_user=true(T4)★: 진행 중 재연결 task 가 다음 `reconnect_guard()` 에서 Stop 을 보고
    // 즉시 멈춘다(끊김 재연결 금지 — wsTransport `close()` 의 `closedByUser=true` 와 동형). bump 로 인한
    // stale 화만으론 "끊김→재연결 루프가 새 my_gen 으로 다시 진입" 같은 경로를 못 막을 수 있어, 의도
    // 플래그를 함께 둬 명시 종료를 영구히 식별한다.
    //
    // ## ★반환값 = 이 close 가 세운 세대★
    // 이 모듈은 프론트 발화 포트를 **들지 않는다** — 그 호출은 락 보유 중 외부 호출이 되어 ADR-0006 을
    // 깬다(헤더 「락을 .await across 보유 금지」와 같은 이유로, 이 락 안에서는 우리 코드만 돈다).
    // 그래서 발화는 호출자가 이 함수가 반환한 **뒤에**(= guard 가 풀린 뒤에) 낸다. 다만 그 발화에는
    // 위 watch 전이와 달리 가드가 필요하다: 반환 직후~발화 사이에 승계 connect 가 세대를 올리면 화면의
    // 주인은 더 새 세대이고, 그때 뒤늦은 `down` 은 그 주인의 `connected` 를 덮어쓴다. 그 재확인
    // ([`Self::is_current`])의 키가 이 반환값이다.
    //
    // ★재확인은 **판정**이지 발행이 아니다★ — `Down` 은 위 한 락 안에서 이미 watch 에 실렸다. 그 자리에
    // `publish_if_current` 를 쓰면 한 번의 close 가 watch 에 두 장으로 실린다(그 사유의 정본 =
    // [`Self::is_current`]).
    pub(crate) fn close(&self) -> u64 {
        Self::close_locked(&mut self.inner.lock().expect("lifecycle poisoned"))
    }

    // `DaemonClient` 의 `Drop` 전용 — ★**이 함수는** 절대 패닉하지 않는다★.
    //
    // `Drop` 안의 패닉은 unwinding 중이면 프로세스를 abort 시킨다. 이 락의 critical section 은 순수 동기
    // 코드뿐이라(위 헤더) 오염될 일이 사실상 없지만, "사실상 없다"에 abort 를 걸지는 않는다 — 오염된
    // 락은 조용히 포기한다. 그 경우엔 이미 다른 패닉이 진행 중이고, 연결 task 는 런타임 종료가 거둔다.
    //
    // ★그 보증이 덮는 것은 `Drop` 경로의 **절반**뿐이다★ — 나머지 절반은 이 호출이 끝난 **뒤에 이어지는
    // 필드 drop** 이고, 거기에는 전용 tokio 런타임(`DaemonClient::_owned_rt`)이 들어 있다. 런타임을 async
    // 컨텍스트 안에서 drop 하면 tokio 가 패닉하는데, 이 함수는 그쪽을 **막지 못한다**(막을 자리가 아니다).
    // 그 절반은 `DaemonClient` 의 `Drop` 본문이 런타임을 직접 꺼내 `shutdown_background` 로 접어 막는다 —
    // 설명의 정본은 그 자리 주석이다. 여기서 "Drop 경로가 패닉하지 않는다"를 통째로 읽지 말 것.
    //
    // ★이 경로는 세대를 **돌려주지 않는다**(명시 `close` 와 갈리는 지점)★ — 락이 오염되면 전이 자체가
    // 일어나지 않으므로 돌려줄 세대가 없고, 없는 것을 있는 척 돌려주면 호출자가 *안 일어난* 전이를 화면에
    // 알린다. 그 갈림의 결말(Drop 경로는 발화하지 않는다)은 [`super::DaemonClient::close_on_drop`] 이
    // 소유한다 — 여기 다시 적지 않는다.
    pub(crate) fn close_best_effort(&self) {
        if let Ok(mut g) = self.inner.lock() {
            Self::close_locked(&mut g);
        }
    }

    // close 의 본체 — 두 진입점(명시 close / Drop)이 **같은 전이**를 하도록 한 곳에 둔다. 갈라 적으면
    // 한쪽만 cancel 을 쏘거나 한쪽만 closed_by_user 를 세우는 어긋남이 조용히 생긴다.
    // 반환 = 이 전이가 세운 세대(락 밖 발화 가드의 키 — `close` 주석).
    fn close_locked(g: &mut LifecycleInner) -> u64 {
        g.generation += 1;
        g.cmd_tx = None;
        g.closed_by_user = true;
        // ★재연결 취소 송신(T4)★: bump + closed_by_user 만으로는, 재연결 task 가 *await 중*이면 다음
        //   reconnect_guard 동기 체크에 닿기 전에 그 await(예: connect_async)가 완료돼 소켓이 열린다.
        let _ = g.cancel_tx.send(g.generation);
        let _ = g.state_tx.send(ConnectionState::Down);
        g.generation
    }

    // ★재연결 루프 1틱 가드(T4)★: 재연결 task 가 매 백오프/시도 전에 호출한다. `generation == my_gen`
    // 과 `!closed_by_user` 를 **한 락 아래서 함께** 읽는다 — 둘을 분리 조회하면 그 사이 close()/새
    // connect 가 끼어 stale task 가 "둘 다 옛 스냅샷"으로 재연결을 강행하는 TOCTOU 가 열린다.
    pub(crate) fn reconnect_guard(&self, my_gen: u64) -> ReconnectVerdict {
        let g = self.inner.lock().expect("lifecycle poisoned");
        if g.generation == my_gen && !g.closed_by_user {
            ReconnectVerdict::Proceed
        } else {
            ReconnectVerdict::Stop
        }
    }

    /// ★구독 타이밍이 load-bearing★: tokio `watch::Receiver` 는 **구독 이후의 send 만** `changed()` 로
    /// 본다(구독 시 현재값을 "seen" 으로 마킹) — 구독 *전* send 는 회수 못 한다. 그래서 재연결 task 는
    /// **connected 직후(= my_gen 이 current 로 확정된 시점) 곧바로** 구독해야 한다.
    pub(crate) fn cancel_subscribe(&self) -> watch::Receiver<u64> {
        self.inner
            .lock()
            .expect("lifecycle poisoned")
            .cancel_tx
            .subscribe()
    }

    // ★현재 활성 연결의 cmd_tx 핸들(T6a — send_command 진입점)★. 저장된 cmd_tx 를 clone 해 돌려준다
    // (None = 연결 task 없음/끊김). `mpsc::Sender::clone` 은 동기·경량이라 락 안에서 OK — 호출자는
    // 반환된 Sender 로 **락 밖에서** `send().await` 한다(Sender 는 lifecycle 락과 독립).
    //
    // ★stale 송신 차단★: bump_and_capture/close 가 cmd_tx 를 None 으로 비우므로(승계·종료), 이 clone 은
    // 항상 current 연결의 채널이다.
    pub(crate) fn current_cmd_tx(&self) -> Option<mpsc::Sender<ConnectionCommand>> {
        self.inner
            .lock()
            .expect("lifecycle poisoned")
            .cmd_tx
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn is_closed_by_user(&self) -> bool {
        self.inner
            .lock()
            .expect("lifecycle poisoned")
            .closed_by_user
    }

    #[cfg(test)]
    pub(crate) fn current_generation(&self) -> u64 {
        self.inner.lock().expect("lifecycle poisoned").generation
    }

    // 테스트 전용 — 좀비 sender 차단의 *상태 불변* 관찰점. `store_cmd_if_current` 의 반환 bool 만으로는
    // "stale 저장이 기존 current sender 를 *덮지 않았다*"를 증명 못 한다(cmd_tx 가 private, Sender 는
    // Eq 없음) — `same_channel` 비교용 clone 을 테스트에만 노출한다.
    #[cfg(test)]
    pub(crate) fn cmd_tx_snapshot(&self) -> Option<mpsc::Sender<ConnectionCommand>> {
        self.inner
            .lock()
            .expect("lifecycle poisoned")
            .cmd_tx
            .clone()
    }
}
