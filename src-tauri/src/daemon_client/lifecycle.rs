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
//! 동시성 표면이 더 커지는 T4(재연결·백오프) 합류 시 재검토 가치 높음.

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
    /// ★재연결 취소 신호(T4 — in-flight 취소 결함 수정, ADR-0038 OSS 정석)★. generation 이 bump 될
    /// 때마다(connect/ensure 승계 진입 · close) **이 watch 에 새 generation 값을 send** 한다. 진행 중인
    /// 재연결 task 가 await(백오프 sleep · read_live · connect_async · 핸드셰이크)를 이 watch 의 `changed()`
    /// 와 `select!` 로 경쟁시켜, 취소가 켜지면 **소켓을 열기 전에 즉시 탈출**한다(close/승계 후 stale
    /// task 가 소켓을 열고 Auth(token)를 서버로 보내는 창을 닫는다 — Codex 적출). watch 를 고른 이유:
    /// (a) cancel-safe(select! 의 다른 arm 이 이기면 changed() 는 부작용 없이 버려짐) (b) **마지막 값을
    /// 보존**해 늦게 구독한 reader 도 borrow 로 *현재 generation 값 자체*는 읽을 수 있다(Notify 는 값이
    /// 없어 "현재 무엇인지"를 못 본다). ★정직(nit)★: 단, `changed()` 가 보는 것은 watch 도 **구독 이후
    /// send 뿐**이다 — 구독 전 send 는 watch 도 changed() 로 회수 못 한다(Notify 와 이 점은 같다). 그래서
    /// 재연결 task 는 connected 직후 곧바로 구독해 그 이후 send 를 빠짐없이 봐야 한다(cancel_subscribe 주석).
    /// watch 의 이점은 "마지막 값 보존"(b)이지 "구독 전 send 회수"가 아니다 — 작업 지시 "Notify 금지" 근거는
    /// (a)+(b)다. ★generation 과 한 락 아래 두는 이유★: bump 와 cancel send 가 같은 critical section 이라,
    /// "세대 올림 ↔ 취소 신호" 사이에 stale task 가 끼어 옛 세대로 소켓을 못 연다.
    cancel_tx: watch::Sender<u64>,
    /// ★closedByUser 가드(T4 — wsTransport `closedByUser` 대응)★. 사용자가 명시 close() 했는가.
    /// true 면 재연결 루프가 즉시 멈춘다(끊김으로 재연결하지 않음) — 명령/재연결이 데몬을 respawn 하면
    /// 안 된다는 ADR-0021 의 task-lifetime 판(꺼진 채 유지, 복구는 명시 connect 로만). connect/ensure
    /// 진입(`bump_and_capture`)이 false 로 되돌려 다시 살아날 수 있게 한다(wsTransport start() 와 동형).
    /// ★generation 과 한 락 아래 두는 이유★: "내가 current 인가 + 사용자가 닫았나" 를 재연결 루프가
    /// 한 번에 원자로 읽어야(`reconnect_guard`), bump 직후 close 가 끼는 창에서 stale 재연결을 못 한다.
    closed_by_user: bool,
}

/// 재연결 루프 1틱의 가드 판정(원자 스냅샷). 재연결 task 가 매 백오프/시도 전에 이걸로 "계속할지"를
/// 결정한다 — `generation`(내가 아직 current 인가)과 `closed_by_user`(사용자가 닫았나)를 **한 락
/// 아래서 함께** 읽어, 둘을 분리 조회하는 사이 close()/새 connect 가 끼는 TOCTOU 를 닫는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReconnectVerdict {
    /// 내가 current + 사용자가 안 닫음 → 재연결 시도/백오프 진행.
    Proceed,
    /// stale(더 새 connect/close 가 세대를 올림) 또는 사용자 close → 재연결 중단(조용히 종료).
    Stop,
}

impl Lifecycle {
    /// 초기 상태 Down 으로 생성. `state_rx` 는 호출자(DaemonClient)가 빠른 읽기용으로 보관한다.
    pub(crate) fn new() -> (Self, watch::Receiver<ConnectionState>) {
        let (state_tx, state_rx) = watch::channel(ConnectionState::Down);
        // 초기 cancel epoch = 0(= 초기 generation). bump/close 가 generation 을 올릴 때마다 같은 값을 send.
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

    /// 세대를 올리고 새 my_gen 을 돌려준다(connect/ensure 진입). 선택적으로 같은 락 아래서
    /// `set_state` 전이도 발행한다 — 진입의 "bump + Connecting" 을 한 critical section 으로 묶어,
    /// bump 직후 다른 스레드가 끼어 세대를 또 올리는 창에서도 *내가 올린 세대로* 일관되게 행동한다.
    ///
    /// ★closedByUser 해제(T4)★: 명시 connect/ensure 진입은 사용자가 다시 살리려는 의도이므로 같은 락
    /// 아래서 `closed_by_user=false` 로 되돌린다(wsTransport start() 의 `closedByUser=false` 와 동형) —
    /// 이전 close 로 멈춘 재연결을 부활시킬 수 있게. bump 와 한 원자라 "닫힘 해제 + 새 세대 캡처"가 쪼개져
    /// 그 사이 stale 재연결이 끼는 일이 없다.
    ///
    /// ★stale cmd_tx 정리(T4 — Codex FIX lifecycle:124)★: 승계가 일어나면(세대 bump) 옛 연결의 cmd_tx 는
    /// 더 이상 유효하지 않으므로 **같은 락 안에서 None 으로 비운다**. 이걸 안 하면 새 connect 핸드셰이크가
    /// 끝나(새 cmd_tx 를 store_cmd_if_current 로 덮어쓰)기 전까지 옛(stale) 명령채널이 lifecycle 에 살아
    /// 있어, 그 창에 들어온 invoke 가 *죽어가는 옛 연결* 로 명령을 보낼 수 있다("stale 명령채널" 잔존).
    /// bump 와 한 원자라 정리와 세대 올림이 쪼개지지 않는다. Sender(옛 cmd_tx)를 여기서 drop 하면 옛 연결
    /// task 의 cmd_rx 가 EOF → main_loop 가 Closed 로 종료(재연결 안 함) → 옛 소켓 정리.
    ///
    /// ★동기★: u64 증가 + bool 대입 + Option 교체 + watch send 모두 동기 → 락 안에서 원자. await 없음.
    pub(crate) fn bump_and_capture(&self, set_state: Option<ConnectionState>) -> u64 {
        let mut g = self.inner.lock().expect("lifecycle poisoned");
        g.generation += 1;
        let my_gen = g.generation;
        // 명시 진입이므로 사용자 close 가드 해제(다시 살아날 수 있게).
        g.closed_by_user = false;
        // 승계 = 옛 cmd_tx 무효화. 새 연결이 store_cmd_if_current 로 자기 것을 넣을 때까지 None 유지.
        g.cmd_tx = None;
        // ★재연결 취소 송신(T4)★: 세대를 올린 = 승계가 일어난 것이므로, 진행 중인 옛 세대 재연결 task 의
        //   await 를 즉시 깨워 stale 임을 알린다(그 task 가 소켓을 열기 전에 select! 로 탈출). bump 와 같은
        //   락 안이라 "세대 올림 ↔ 취소 신호"가 원자 — 그 사이 옛 task 가 끼어 옛 세대로 진행할 수 없다.
        let _ = g.cancel_tx.send(g.generation);
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

    /// 명시 종료(close). 락 잡고 (a)세대 bump (b)cmd_tx=None (c)closed_by_user=true (d)Down 발행 **을
    /// 한 원자 단위로** 한다. 넷이 같은 critical section 이라, bump 와 Down 사이에 stale task 가 끼어
    /// Connected 를 발행할 수 없다(끼더라도 그 publish_if_current 는 이미 올라간 세대를 보고 삼킨다).
    /// 이 Down 은 close 자신의 의도라 항상 유효.
    ///
    /// ★closed_by_user=true(T4)★: 진행 중 재연결 task 가 다음 `reconnect_guard()` 에서 Stop 을 보고
    /// 즉시 멈춘다(끊김 재연결 금지 — wsTransport `close()` 의 `closedByUser=true` 와 동형). bump 로 인한
    /// stale 화만으론 "끊김→재연결 루프가 새 my_gen 으로 다시 진입" 같은 경로를 못 막을 수 있어, 의도
    /// 플래그를 함께 둬 명시 종료를 영구히 식별한다.
    /// ★동기★: bump + Option 교체 + bool 대입 + watch send → await 없음.
    pub(crate) fn close(&self) {
        let mut g = self.inner.lock().expect("lifecycle poisoned");
        g.generation += 1;
        g.cmd_tx = None;
        g.closed_by_user = true;
        // ★재연결 취소 송신(T4)★: close 는 generation bump + closed_by_user 둘 다 켜지만, 진행 중인 재연결
        //   task 가 *await 중*이면 다음 reconnect_guard 동기 체크에 닿기 전까지 그 await(예: connect_async)가
        //   완료돼 소켓이 열린다. cancel watch 를 같은 락 안에서 send 해 그 await 를 select! 로 즉시 깨운다 —
        //   close 후 stale task 가 소켓 open + Auth 전송하는 창을 닫는 1차 방어선(generation 가드는 2차).
        let _ = g.cancel_tx.send(g.generation);
        let _ = g.state_tx.send(ConnectionState::Down);
    }

    /// ★재연결 루프 1틱 가드(T4)★: 재연결 task 가 매 백오프/시도 전에 호출한다. `generation == my_gen`
    /// (내가 아직 current 인가)과 `!closed_by_user`(사용자가 안 닫았나)를 **한 락 아래서 함께** 읽어
    /// 원자 판정을 돌려준다. 둘을 분리 조회하면(generation 따로, closed 따로) 그 사이 close()/새 connect
    /// 가 끼어 stale task 가 "둘 다 옛 스냅샷"으로 재연결을 강행하는 TOCTOU 가 열린다 — 한 critical
    /// section 으로 묶어 닫는다.
    ///
    /// 반환 Proceed = 내가 current + 안 닫힘 → 시도/백오프 계속. Stop = stale 이거나 사용자 close →
    /// 재연결 중단(task 가 조용히 종료). ★동기★: 비교 2개 → await 없음.
    pub(crate) fn reconnect_guard(&self, my_gen: u64) -> ReconnectVerdict {
        let g = self.inner.lock().expect("lifecycle poisoned");
        if g.generation == my_gen && !g.closed_by_user {
            ReconnectVerdict::Proceed
        } else {
            ReconnectVerdict::Stop
        }
    }

    /// ★재연결 취소 구독(T4 — in-flight 취소)★. 진행 중인 재연결 task 가 이 receiver 를 들고 매 await 를
    /// `select!` 의 한 arm(`cancel_rx.changed()`)으로 경쟁시킨다. close()/승계 connect 가 cancel_tx 에 새
    /// generation 을 send 하면 그 await 가 즉시 깨어나, task 는 `reconnect_guard(my_gen)` 로 재확인 후
    /// Stop 이면 **소켓을 열지 않고** 탈출한다. ★cancel-safe★: select! 의 다른 arm 이 이기면 changed()
    /// 는 부작용 없이 폐기된다(watch 의 cancel-safety).
    ///
    /// ★구독 타이밍 정직 표기(nit 정정)★: tokio `watch::Receiver` 는 **구독(subscribe) 이후의 send 만**
    /// `changed()` 로 본다 — 구독 *전*에 이미 일어난 send 는 못 본다(구독 시 현재값을 "seen" 으로 마킹).
    /// 이전 주석의 "구독 직후 이미 올라간 epoch 도 첫 changed() 가 잡는다"는 *사실과 다르다*. 그래서
    /// 호출 순서가 load-bearing 이다: 재연결 task 는 **connected 직후(= my_gen 이 current 로 확정된 시점)
    /// 곧바로 구독**해야 한다(run_connection 이 connected_lifetime 진입 전에 cancel_subscribe 호출). 그
    /// 구독 이후의 모든 bump/close send 를 빠짐없이 본다. Notify 대신 watch 를 고른 진짜 이유는 "마지막
    /// 값 보존"(늦게 구독해도 *현재 generation 값 자체*는 borrow 로 읽힘)이지, "구독 전 send 를 changed()
    /// 로 회수"가 아니다 — 후자는 watch 도 못 한다.
    pub(crate) fn cancel_subscribe(&self) -> watch::Receiver<u64> {
        self.inner
            .lock()
            .expect("lifecycle poisoned")
            .cancel_tx
            .subscribe()
    }

    /// ★현재 활성 연결의 cmd_tx 핸들(T6a — send_command 진입점)★. 락 잡고 현재 저장된 cmd_tx 를
    /// clone 해 돌려준다(없으면 None = 연결 task 없음/끊김). `mpsc::Sender::clone` 은 동기·경량이라
    /// 락 안에서 OK(ADR-0006 — await 없음). 호출자는 반환된 Sender 로 **락 밖에서** `send().await` 한다
    /// (Sender 는 cmd_rx 와 독립 채널이라, 이 락을 쥔 채 send 하지 않는다 → 락 across await 없음).
    ///
    /// ★stale 송신 차단★: bump_and_capture/close 가 cmd_tx 를 None 으로 비우므로(승계·종료), 이 clone 은
    /// 항상 "현재 current 연결" 의 채널이다. 승계 직후 옛 cmd_tx 로 명령이 새는 일이 없다(lifecycle 정합).
    pub(crate) fn current_cmd_tx(&self) -> Option<mpsc::Sender<ConnectionCommand>> {
        self.inner
            .lock()
            .expect("lifecycle poisoned")
            .cmd_tx
            .clone()
    }

    /// 현재 closed_by_user 스냅샷(테스트용 — close 가드의 단위 검증).
    #[cfg(test)]
    pub(crate) fn is_closed_by_user(&self) -> bool {
        self.inner
            .lock()
            .expect("lifecycle poisoned")
            .closed_by_user
    }

    /// 현재 세대 스냅샷(테스트용 — 가드 판정의 단위 검증). 운영 코드는 my_gen 캡처값으로 비교한다.
    #[cfg(test)]
    pub(crate) fn current_generation(&self) -> u64 {
        self.inner.lock().expect("lifecycle poisoned").generation
    }

    /// 저장된 cmd_tx 의 식별자(테스트 전용 — 좀비 sender 차단의 *상태 불변* 관찰점). cmd_tx 가 private 이라
    /// 반환 bool 만으로는 "stale 저장이 기존 current sender 를 *덮지 않았다*"를 증명 못 한다 — 저장된 sender
    /// 의 동일성을 비교할 핸들이 필요하다. Sender 자체는 Eq 가 없고 운영 코드가 식별자를 들 이유가 없으므로,
    /// `same_channel` 비교용 clone 을 테스트에만 노출한다(None=미저장). 운영 경로엔 이 접근자가 없다.
    #[cfg(test)]
    pub(crate) fn cmd_tx_snapshot(&self) -> Option<mpsc::Sender<ConnectionCommand>> {
        self.inner
            .lock()
            .expect("lifecycle poisoned")
            .cmd_tx
            .clone()
    }
}
