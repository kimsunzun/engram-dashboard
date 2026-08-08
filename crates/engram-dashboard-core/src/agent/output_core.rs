//! OutputCore — 에이전트 1개의 출력 측 핵심 상태(seq/replay/subscribers/status)와
//! 그 위의 동작(emit/finish/subscribe/...)을 transport·session에서 분리한 공용 struct.
//!
//! 왜 분리하는가: 출력 fanout·종료 전이·구독은 PTY든 API든 transport 종류와 무관하게
//! 동일하다. transport는 바이트·이벤트를 만들어 `emit`/`finish`로 넘기기만 하면 된다.
//!
//! transport(pump)가 emit/finish로 출력·종료를 넘기고, manager/AgentSession이 subscribe/
//! status/snapshot으로 조회한다.
//!
//! tauri import 0. S9 drain/session의 락 규율·불변식을 글자 그대로 보존한다.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::agent::backend::TurnClassifier;
use crate::agent::turn::{TurnObservations, TurnSignal};
use crate::agent::types::{
    AgentId, AgentStatus, OutputChunk, OutputEvent, OutputFrame, OutputPayload, OutputSink,
    ReplayKind, SinkId, StatusSink, SubscribeOutcome, TerminalReason,
};

type OnTerminalHook = Box<dyn Fn(TerminalReason) + Send + Sync>;

/// 에이전트 1개의 출력 측 핵심 상태. 필드별 독립 Mutex(session.rs 모듈 주석의 분리 동기와 동일):
/// emit이 replay/subscribers lock만 짧게 잡는 동안 다른 경로(status 등)와 교착 없이 병행 가능.
pub struct OutputCore {
    // ── 불변 (생성 후 변경 없음) ──────────────────────────────
    id: AgentId,
    epoch: u32,

    // ── 출력 시퀀스 / 상태 ────────────────────────────────────
    seq: AtomicU64,
    status: Mutex<AgentStatus>,
    finalized: AtomicBool,

    // ── 출력 구독 ─────────────────────────────────────────────
    subscribers: Mutex<Vec<Arc<dyn OutputSink>>>,

    // ── Replay buffer ─────────────────────────────────────────
    replay: Mutex<Ring>,

    // ── 상태 알림 ─────────────────────────────────────────────
    status_sink: Arc<dyn StatusSink>,

    // ── pump thread 제어 ──────────────────────────────────────
    drain_handle: Mutex<Option<JoinHandle<()>>>,
    drain_done_rx: Mutex<Option<Receiver<()>>>,

    // ── finalize 1회 hook (ADR-0019 reaper) ───────────────────
    /// 단위테스트는 OutputCore::new 만 쓰고 hook 을 주입하지 않으므로 Option(None=no-op).
    on_terminal: Mutex<Option<OnTerminalHook>>,

    // ── ADR-0113 턴 관측 ──────────────────────────────────────
    /// 생성 후 불변이라 hot path 에 원자 load 도 락도 없다.
    turn: TurnWiring,
}

/// 턴 관측 배선 한 벌(ADR-0113) — 표(어디에 쌓나)와 분류자(무엇이 신호인가)는 항상 같이 꽂힌다.
/// 둘을 따로 두면 "표는 있는데 분류자가 없는" 반쪽 상태가 생긴다.
///
/// ★`OutputCore::new` 의 **필수 인자**인 게 이 타입의 요점이다(ADR-0002 §0 — 싸고 오래 가는 경계는
///   지금 깐다)★: 사후 주입(setter)이면 새 spawn 경로가 그걸 빠뜨려도 컴파일이 통과하고, 그 에이전트는
///   **영구히 idle 로 보고돼** 턴 중에 메일이 꽂힌다 — 에러도 로그도 없이. 인자로 만들면 그 실수가
///   컴파일 에러가 된다. 관측이 필요 없는 조립은 `detached()` 로 **명시**한다(빠뜨림과 구분된다).
pub struct TurnWiring {
    table: Arc<TurnObservations>,
    classify: TurnClassifier,
}

impl TurnWiring {
    /// 운영 배선 — 매니저의 공용 표 + 그 세션 백엔드의 분류자(`backend::turn_classifier`).
    pub fn new(table: Arc<TurnObservations>, classify: TurnClassifier) -> Self {
        Self { table, classify }
    }

    /// ★관측을 쓰지 않는 조립 전용(하네스·독립 smoke)★ — 아무도 읽지 않는 자기 표 + 침묵 분류자.
    ///   신호를 하나도 만들지 않으므로 관측 소비자 관점에서 "그 에이전트는 관측된 적 없음" 과 같다.
    ///
    /// ★운영 spawn 경로는 절대 쓰지 않는다★: 이걸 쓰면 그 에이전트의 턴 게이트가 조용히 꺼진다
    ///   (= 턴 중 주입). 운영은 반드시 `new(manager 표, backend 분류자)` 를 쓴다 — 이름이 다른 이유가
    ///   그것이다. ★그 구분을 강제하는 장치는 **없다**★: `use tauri` 격리나 메시징 격리와 달리 이 호출을
    ///   막는 grep 게이트가 없으므로, 이건 컴파일러가 아니라 규약이 지키는 경계다(운영 호출이 0 인지는
    ///   리뷰가 본다).
    ///
    /// ★왜 `test-harness` feature 로 잠그지 않았나(기록된 트레이드오프)★: 그렇게 하면 core 자체 통합
    ///   테스트(`tests/*.rs`)가 lib 을 **외부 crate 로** 링크하는데 그 기능이 꺼져 있어, core 에도
    ///   self-dev-dependency 를 얹어야 한다. 그 패턴은 이 워크스페이스에 이미 있다(daemon Cargo.toml 의
    ///   ADR-0088 트릭) — 그러니 불가능해서가 아니다. 걸리는 건 그 파일이 함께 문서화한 비용이다:
    ///   cargo 는 한 호출 안에서 같은 crate 의 feature 를 **합집합**하므로, `--all-targets` 로 lib 과
    ///   테스트 타깃을 같이 빌드하면 그 기능이 공유 lib 빌드로 유니피케이션돼 **배포 산출물에 하네스
    ///   표면이 박힌다**. 그 위험을 지금 core 까지 넓히는 값이, "이름이 다른 생성자" 가 이미 주는 방어보다
    ///   크다고 봤다. 잠그기로 결정이 바뀌면 그 릴리스 빌드 규칙(`--all-targets` 금지)이 core 에도 걸린다.
    #[doc(hidden)]
    pub fn detached() -> Self {
        Self {
            table: Arc::new(TurnObservations::new()),
            classify: crate::agent::backend::no_turn_signals,
        }
    }
}

impl OutputCore {
    pub fn new(
        id: AgentId,
        epoch: u32,
        status_sink: Arc<dyn StatusSink>,
        turn: TurnWiring,
    ) -> Self {
        Self {
            id,
            epoch,
            seq: AtomicU64::new(0),
            status: Mutex::new(AgentStatus::Running),
            finalized: AtomicBool::new(false),
            subscribers: Mutex::new(Vec::new()),
            replay: Mutex::new(Ring::new()),
            status_sink,
            drain_handle: Mutex::new(None),
            drain_done_rx: Mutex::new(None),
            on_terminal: Mutex::new(None),
            turn,
        }
    }

    pub fn id(&self) -> AgentId {
        self.id
    }

    /// finalize-시점 hook 주입 — spawn_session 이 sessions 맵 등록 전에 1회 호출한다.
    /// 클로저는 finalized.swap 승자 경로에서만(=정확히 1회) 불린다. ★race 방지★:
    /// 클로저 내부에서 intent·shutting_down 을 그 순간 snapshot 해 ReapMsg 를 빌드·송신해야 한다.
    pub fn set_on_terminal(&self, hook: OnTerminalHook) {
        *self.on_terminal.lock().expect("on_terminal poisoned") = Some(hook);
    }

    /// ADR-0079: resume 스폰 시 `.jsonl` transcript 에서 복원한 과거 이벤트를 replay 버퍼에 **seed**한다.
    ///
    /// ★seed-before-publish 불변식(ADR-0079, load-bearing — seed before publish: closes
    ///   empty-ring-replay + seq-interleave window, cross-family review 2026-07-13)★: manager.
    ///   spawn_session 이 이 core 를 **sessions 맵에 insert 하기 전에**(= 세션이 관측 가능해지기 전에)
    ///   호출한다. 구독·emit 경로 모두 sessions 맵 조회(get_session)를 거치므로, insert 전이면 다른
    ///   스레드가 이 core 에 닿을 수 없다 → seed 도중 재접속 구독이 빈 Ring 을 replay 하거나(seed 는
    ///   fanout 안 함 → 과거 영구 유실), 동시 emit 이 seq 를 뒤섞는(Ring 순서 [0,2,1]) 윈도가 원천 차단된다.
    ///   insert 전이니 당연히 pump(라이브 emit) 시작 전이기도 하다 — 그래서 아직 구독자·라이브 이벤트가 없다.
    ///
    /// ★fanout 없음(seed 시점 특성)★: emit 과 달리 subscribers 로 send 하지 않는다 — 구독자가 아직
    ///   없으므로 무의미하고, 있더라도 seed 는 "버퍼 사전 적재"라 replay 경로(subscribe→replay)로만
    ///   전달돼야 한다(라이브 fanout 이 아니다). 그래서 replay lock 만 짧게 잡아 Ring 에 push 하고 seq 만
    ///   전진시킨다(ADR-0006 락 규율 유지 — subscribers lock 미취득).
    ///
    /// ★seq 연속성★: seed 한 이벤트마다 emit 과 동일하게 `seq.fetch_add(1)` 로 seq 를 발급한다. 그래서
    ///   seed 뒤 첫 라이브 emit 의 seq 가 seed 마지막 seq+1 로 자연히 이어진다(gap·중복 없음).
    ///   finalize·status 는 건드리지 않는다(ADR-0005 — seed 는 종료 전이가 아니다).
    ///
    /// ★턴 관측도 건드리지 않는다 — 되살리지 말 것(ADR-0113)★: transcript 는 **지나간 기록**이라
    ///   턴 중간에 끊긴 것일 수 있고(killed incarnation), 그러면 진행 신호로 끝나고 종료 신호가 없다. 그걸
    ///   관측으로 먹이면 새 화신이 "턴 중" 으로 찍히는데 그 턴의 종료는 **영원히 오지 않는다** — 그 상태를
    ///   기다리는 소비자(우편 파킹 등)가 깨울 수 없이 멈춘다. 관측은 라이브 emit 에서만 시작한다.
    pub fn seed(&self, events: Vec<OutputEvent>) {
        let mut replay = self.replay.lock().expect("replay poisoned");
        for event in events {
            let seq = self.seq.fetch_add(1, Ordering::Relaxed);
            let cost_bytes = estimate_cost_bytes(&event);
            replay.push(StoredOutput {
                seq,
                event,
                cost_bytes,
            });
        }
    }

    /// **payload-generic (S15 B4)** — 모든 OutputEvent variant(콘솔 바이트 + 구조화)를 받아
    /// Ring 에 저장하고 payload-generic 으로 fanout 한다(ADR-0002 출력 종류 비가정).
    ///
    /// ★핵심 불변식 (ADR-0006 §10 규칙3 — payload 만 바뀌지 락 구조는 불변)★
    /// - `sink.send()` 호출 시 어떤 lock도 보유하지 않는다. subscribers를 clone으로
    ///   스냅샷 뜨고 lock을 즉시 해제한 뒤, 복사본을 돌며 send.
    /// - replay lock과 subscribers lock을 동시에 보유하지 않는다(각각 짧게).
    ///   두 lock 동시 취득은 subscribe 함수 단독 예외이며 emit은 절대 금지.
    pub fn emit(&self, event: OutputEvent) {
        let cost_bytes = estimate_cost_bytes(&event);

        // 3~4. ★seq 발급 + replay push 를 replay 락 안에서 원자적으로★ — brief lock(락 순서 1단계,
        //    ADR-0006). **순서 중요(gap 방지)**: replay.push 가 fanout 보다 먼저여야, subscribe 가
        //    이 사이에 끼어들어도 새 sink 는 replay 에서 이 seq 를 받는다(최악 dup, 프론트 seq dedup 이
        //    흡수). 역순이면 gap 발생.
        //
        // ★ADR-0079: seq 발급을 replay 락 안에서 push 직전에 한다(왜 락 밖이면 안 되나)★
        //    pump 스레드(transport stdio/pty)와 write_input 의 synthetic user-echo(session.rs)가
        //    동시에 emit 을 호출한다. seq 를 락 밖에서 fetch_add 하면 두 caller 가 N/N+1 을 발급받고도
        //    락 진입 순서가 뒤집혀 ring 에 N+1 을 N 보다 먼저 push 할 수 있다 → ring 이 seq 로 정렬
        //    깨짐. subscribe_from 은 `partition_point(|c| c.seq <= s)`(seq 오름차순 전제)로 replay
        //    slice 를 자르므로, ring 비단조면 replay 슬라이싱이 무너진다. 발급+push 를 같은 락 구간에
        //    묶어 원자화하면 락 획득 순서가 곧 seq 순서 = ring 항상 단조. (cross-family review 2026-07-13
        //    발견 — seed() 도 동일하게 락 안에서 발급하므로 두 경로가 일치한다.)
        //    Ordering::Relaxed 유지: 락이 순서(happens-before)를 제공하므로 atomic 은 유일성만 담당.
        let seq;
        {
            let mut replay = self.replay.lock().expect("replay poisoned");
            seq = self.seq.fetch_add(1, Ordering::Relaxed);
            replay.push(StoredOutput {
                seq,
                event: event.clone(),
                cost_bytes,
            });
        }

        // ★ADR-0113 턴 관측★: 표 갱신을 **fanout·통지보다 먼저** 한다. 통지를 받은 소비자가 곧바로
        //   표를 조회하므로(도어벨→flush 등) 순서가 뒤집히면 그 조회가 갱신 전 값을 본다.
        //   락 규율: 표 갱신은 자기 락 하나만 짧게 잡고(core 락 미보유), 통지는 그 락을 놓은 뒤 한다.
        if let Some(signal) = (self.turn.classify)(&event) {
            // ★신호에 **출력 순서(seq)** 를 실어 보낸다★: emit 호출자는 둘이라(pump · 입력 에코를 낸
            //   주입 스레드) 두 emit 이 병행하면 표 적용 순서가 발행 순서와 뒤집힐 수 있다. seq 는 replay
            //   락 안에서 발급돼 **출력의 정본 순서**이므로, 표가 그걸로 늦은 신호를 걸러낸다(turn.rs).
            self.turn.table.observe(self.id, self.epoch, seq, signal);
            // ★종료 후 지각 emit 이 유령 항목을 되살리지 못하게(load-bearing)★: 주입 스레드가 transport
            //   write 에 막혀 있는 동안 pump 가 EOF→`finish` 를 지나 표를 비울 수 있고, 그 뒤 깨어난 에코가
            //   **같은 epoch** 으로 항목을 다시 만든다(더 작은 epoch 만 버리는 표 쪽 규칙으론 못 막는다).
            //   그 항목은 종료 신호가 영영 오지 않아 아무도 못 지운다.
            //   ★무엇이 순서를 만드나 = **표의 뮤텍스**(이 인자를 지우지 말 것)★: 그 락이 우리 insert 와
            //   `finish` 의 forget 을 **전순서**로 놓는다. forget 이 먼저인 순서에서는 forget 의 unlock
            //   (release)이 우리 lock(acquire)과 synchronizes-with 하므로, forget 앞의 `finalized` swap 이
            //   우리 load 보다 happens-before → load 는 false 를 읽을 수 없다. 반대 순서면 우리 insert 가
            //   먼저이므로 뒤따르는 forget 이 그걸 지운다. 어느 쪽이든 유령이 남지 않는다.
            //   ★그래서 이 load 의 `Acquire` 가 근거가 아니다★ — `Relaxed` 여도 결론은 같고, 순서를 나르는
            //   것은 뮤텍스다. 표 갱신을 락 밖(lock-free)으로 "최적화" 하면 이 증명이 조용히 무너진다.
            if self.finalized.load(Ordering::Acquire) {
                self.turn.table.forget(self.id, self.epoch);
            }
            // ★통지는 epoch·finalize·seq 게이트를 걸지 않는다(위 표 갱신과 의도적으로 비대칭)★: 표는
            //   **상태**라 죽은/옛 화신이 쓰면 산 화신의 사실이 오염되지만, 도어벨은 **일회성 자극**이라
            //   잉여는 빈 큐 no-op 으로 흡수되고(소비자가 실행 시점에 재검증) 누락은 대기를 만든다
            //   ("누락 < 잉여" — StatusSink::turn_ended 계약).
            if signal == TurnSignal::Ended {
                self.status_sink.turn_ended(self.id, self.epoch);
            }
        }

        let payload = match &event {
            OutputEvent::TerminalBytes(v) => OutputPayload::Bytes(v),
            other => OutputPayload::Event(other),
        };

        // 5. ★불변식 1(ADR-0006 §10 규칙3)★ send 는 blocking/try_send 가능하므로 lock 을 쥔 채
        //    send 하면 subscribe/다른 send 와 교착·정체.
        let frame = OutputFrame {
            agent_id: self.id,
            epoch: self.epoch,
            seq,
            payload,
        };
        let sinks = self
            .subscribers
            .lock()
            .expect("subscribers poisoned")
            .clone();

        let mut dead = Vec::new();
        for sink in sinks {
            if sink.send(frame).is_err() {
                dead.push(sink.sink_id());
            }
        }

        if !dead.is_empty() {
            self.subscribers
                .lock()
                .expect("subscribers poisoned")
                .retain(|s| !dead.contains(&s.sink_id()));
        }
    }

    /// 종료 전이 — pump가 루프 탈출 후 1회 호출. finalize 정확히 1회 게이트로 중복 호출을 흡수한다.
    ///
    /// terminal 알림 주체는 pump(=여기) 단독. reason→AgentStatus 매핑은 impl-spec 표 그대로.
    pub fn finish(&self, reason: TerminalReason) {
        if self.finalized.swap(true, Ordering::AcqRel) {
            return;
        }

        // AgentStatus 변형 추가 금지(impl-spec 표).
        // ★reason 은 reaper hook 에도 넘겨야 하므로 매핑 전에 clone 해 둔다(소비 전 보존).
        let new_status = match reason.clone() {
            TerminalReason::Exited { code } => AgentStatus::Exited { code },
            TerminalReason::Killed => AgentStatus::Killed,
            TerminalReason::Interrupted => AgentStatus::Killed,
            TerminalReason::StreamClosed => AgentStatus::Exited { code: None },
            TerminalReason::Cancelled => AgentStatus::Killed,
            TerminalReason::Error(s) => AgentStatus::Failed { message: s },
        };

        {
            let mut status = self.status.lock().expect("status poisoned");
            *status = new_status.clone();
        }

        // ★ADR-0113: 이 화신의 턴 관측을 여기서 거둔다★ — finalize 승자 경로라 **정확히 1회**고(ADR-0005),
        //   terminal 로 넘어간 화신은 더 이상 사실을 만들지 않는다. 안 지우면 턴 중에 죽은 에이전트가
        //   "턴 중" 으로 남아 그 앞에서 기다리던 소비자(우편 파킹 등)가 영영 풀리지 않는다.
        //   ★왜 여기고 reaper 가 아닌가★: reaper 는 이 finish 가 보낸 ReapMsg 로만 깨어나므로 **늘 뒤**고,
        //   무엇보다 `finalized` 플래그와 같은 지점이어야 emit 쪽 지각 삽입과의 경쟁이 순서로 닫힌다
        //   (emit 의 finalize 재확인 주석이 그 인과의 다른 반쪽). epoch 일치 검사는 forget 이 한다.
        // ADR-0113
        self.turn.table.forget(self.id, self.epoch);

        // status lock 해제 후 외부 호출(§10: status lock 보유 중 외부호출 금지).
        self.status_sink
            .status_changed(self.id, new_status, self.epoch);

        // ★ADR-0019 reaper hook★: on_terminal lock 은 짧게 잡고 즉시 clone 없이 호출 — 다른 core
        //   lock 미보유 구간이라 안전.
        if let Some(hook) = self
            .on_terminal
            .lock()
            .expect("on_terminal poisoned")
            .as_ref()
        {
            hook(reason);
        }
    }

    /// 과도기 Exiting 전이 — manager kill 0.5단계용. Exiting 알림 주체가 이 경로.
    /// 이미 finalize됐거나 terminal이면 false(덮어쓰지 않음). Running 등 비-terminal일 때만
    /// Exiting 기록 + status_changed(Exiting) 발행 후 true.
    ///
    /// **보호 범위(정확히):** 아래 finalized/terminal 검사가 보호하는 것은 **status 필드 값**이다
    /// (terminal이 Exiting으로 덮여 고착되는 것 방지). status_sink로 나가는 **알림 순서**는
    /// 보호하지 않는다 — finish의 status_changed와 lock 밖에서 경합할 수 있다.
    /// 단, 정상 kill 경로는 manager가 enter_exiting() 완주 후에야 transport.shutdown()을 호출하므로
    /// (그제서야 pump가 깨어 finish), Exiting 알림이 terminal 알림보다 먼저 완주해 순서가 보장된다.
    /// 알림 역전 창은 "프로세스 자연 종료가 kill과 동시에 겹치는 순간"뿐이며, 이는 S9 원본
    /// (manager.kill_agent의 Exiting 발행 vs drain.transition의 terminal 발행)에도 동일하게 존재하는
    /// 기존 동작이다. 프론트는 terminal 판정을 status_changed가 아니라 agent-list-updated(목록)로
    /// 하므로(CLAUDE.md 핵심 불변식) 늦은 Exiting을 받아도 고착되지 않는다 — 설계상 완화됨.
    pub fn enter_exiting(&self) -> bool {
        // 이미 종료 처리됐으면 Exiting을 쓰지 않는다(빠른 경로 — 실제 status 필드 보호는 아래 lock 구간).
        if self.finalized.load(Ordering::Acquire) {
            return false;
        }

        {
            let mut status = self.status.lock().expect("status poisoned");
            if matches!(
                *status,
                AgentStatus::Exited { .. } | AgentStatus::Killed | AgentStatus::Failed { .. }
            ) {
                return false;
            }
            *status = AgentStatus::Exiting;
        }

        // status lock 해제 후 외부 호출.
        self.status_sink
            .status_changed(self.id, AgentStatus::Exiting, self.epoch);
        true
    }

    /// pump 종료 대기 — kill 6단계. 수신측이 이미 사라졌어도(타임아웃 후 detach) 무시 가능하도록
    /// 결과를 버린다.
    pub fn join_pump(&self, timeout: Duration) {
        let rx = self
            .drain_done_rx
            .lock()
            .expect("drain_done_rx poisoned")
            .take();
        if let Some(rx) = rx {
            let _ = rx.recv_timeout(timeout);
        }
    }

    /// pump 핸들/done_rx 적재 — transport.start(stage 3)가 pump 스레드를 띄운 뒤 호출.
    pub fn attach_pump(&self, handle: JoinHandle<()>, done_rx: Receiver<()>) {
        *self.drain_handle.lock().expect("drain_handle poisoned") = Some(handle);
        *self.drain_done_rx.lock().expect("drain_done_rx poisoned") = Some(done_rx);
    }

    /// 구독자 등록 + replay 전송. SinkId 반환(unsubscribe용).
    ///
    /// **C4 (LLD §7, 절대 준수):** subscribers lock을 보유한 채로 replay를 전송한다.
    /// 이렇게 하면 emit의 live send와 이 replay send가 같은 subscribers lock으로
    /// 직렬화되어 replay→live 순서 역전이 원천 차단된다. emit은 step 5에서 subscribers
    /// lock을 잡으려다 잠깐 대기하지만, replay 전송은 일회성이라 허용된다.
    ///
    /// **락 순서 규칙 3 예외 (LLD §10):** subscribe 함수만 subscribers→replay 두 lock을
    /// 동시에 취득한다(항상 이 순서). emit은 두 lock 동시 보유 절대 금지.
    pub fn subscribe(&self, sink: Arc<dyn OutputSink>) -> SinkId {
        let sink_id = sink.sink_id();

        let mut subscribers_guard = self.subscribers.lock().expect("subscribers poisoned");
        subscribers_guard.push(sink.clone());

        let snapshot = {
            let replay_guard = self.replay.lock().expect("replay poisoned");
            replay_guard.snapshot()
        };

        // snapshot의 seq와 이후 live chunk의 seq가 끊기지 않아 프론트가 seq로 dedup/정렬 가능.
        // 막 등록된 sink라 send 실패는 unlikely → 무시(§7).
        for stored in &snapshot {
            let payload = match &stored.event {
                OutputEvent::TerminalBytes(v) => OutputPayload::Bytes(v),
                other => OutputPayload::Event(other),
            };
            let frame = OutputFrame {
                agent_id: self.id,
                epoch: self.epoch,
                seq: stored.seq,
                payload,
            };
            let _ = sink.send(frame);
        }

        drop(subscribers_guard);

        sink_id
    }

    /// after_seq/epoch 기반 선택적 replay 구독. `subscribe`의 C4 패턴(subscribers lock 보유 중
    /// replay 전송)을 그대로 따르되, 보낼 범위를 분기한다.
    ///
    /// 분기:
    /// - epoch 불일치(epoch_matches=false) 또는 after_seq=None → FromOldest(전체).
    /// - epoch 일치 & after_seq=Some(s):
    ///     - 버퍼 비었으면 → Resumed(전송 0).
    ///     - s < oldest → Truncated(oldest 부터 전체).
    ///     - s >= oldest → Resumed(seq>s 인 tail 만).
    ///
    /// `on_ready`: 분기·메타(oldest/latest/kind/replay_from)가 확정된 뒤 **replay 를 sink 로
    ///   전송하기 직전**에 1회 호출된다. 데몬이 이 안에서 SubscribeAck 를 먼저 큐잉해
    ///   "Ack→replay binary" FIFO 순서(불변식 2)를 보장하면서도, Ack 필드를 이 단일 스냅샷
    ///   기준 outcome 으로 채워 TOCTOU(스냅샷 A/B 불일치)를 제거한다. **on_ready 안에서 블로킹 금지**
    ///   (subscribers lock 보유 중 호출 — non-blocking try_send 만).
    pub fn subscribe_from(
        &self,
        sink: Arc<dyn OutputSink>,
        after_seq: Option<u64>,
        epoch_matches: bool,
        on_ready: impl FnOnce(&SubscribeOutcome),
    ) -> SubscribeOutcome {
        let sink_id = sink.sink_id();
        let mut subscribers_guard = self.subscribers.lock().expect("subscribers poisoned");
        subscribers_guard.push(sink.clone());
        let snapshot = {
            let replay_guard = self.replay.lock().expect("replay poisoned");
            replay_guard.snapshot()
        };

        let oldest = snapshot.first().map(|c| c.seq).unwrap_or(0);
        let latest = snapshot.last().map(|c| c.seq).unwrap_or(0);

        let (kind, start_idx) = match after_seq {
            _ if !epoch_matches => (ReplayKind::FromOldest, 0usize),
            None => (ReplayKind::FromOldest, 0usize),
            Some(s) => {
                if snapshot.is_empty() {
                    (ReplayKind::Resumed, 0usize)
                } else if s < oldest {
                    (ReplayKind::Truncated, 0usize)
                } else {
                    // seq<=s 인 prefix 를 건너뛴다(seq 연속 보장 → partition_point 안전).
                    let idx = snapshot.partition_point(|c| c.seq <= s);
                    (ReplayKind::Resumed, idx)
                }
            }
        };

        let to_send = &snapshot[start_idx..];

        let replay_from = to_send
            .first()
            .map(|c| c.seq)
            .unwrap_or_else(|| match after_seq {
                Some(s) => s.saturating_add(1),
                None => latest.saturating_add(1),
            });

        let outcome = SubscribeOutcome {
            kind,
            sink_id,
            oldest_seq: oldest,
            latest_seq: latest,
            replay_from,
            replayed: to_send.len(),
        };

        on_ready(&outcome);

        for stored in to_send {
            let payload = match &stored.event {
                OutputEvent::TerminalBytes(v) => OutputPayload::Bytes(v),
                other => OutputPayload::Event(other),
            };
            let frame = OutputFrame {
                agent_id: self.id,
                epoch: self.epoch,
                seq: stored.seq,
                payload,
            };
            let _ = sink.send(frame);
        }
        drop(subscribers_guard);

        outcome
    }

    pub fn unsubscribe(&self, sink_id: SinkId) {
        self.subscribers
            .lock()
            .expect("subscribers poisoned")
            .retain(|s| s.sink_id() != sink_id);
    }

    /// replay 스냅샷 — 늦게 붙는 창의 초기 복원용(get_snapshot → wire SnapshotChunk 경로).
    ///
    /// ★계약 유지(B5)★: 이 게터는 여전히 `Vec<OutputChunk>`(바이트 전용 wire 미러)를 반환한다 —
    /// 호출부(manager.get_snapshot → daemon snapshot_chunk_to_wire)가 이 형태에 의존한다. Ring 은
    /// payload-generic(StoredOutput) 이지만, 이 경로의 wire 타입(OutputChunk{seq,data})은 아직 바이트
    /// 전용이라 **TerminalBytes 만** 변환한다. 구조화 이벤트의 snapshot wire 매핑은 B7(daemon adapter)
    /// 몫이라 여기선 스킵한다 — 현재 구조화 이벤트 생산자가 없어(B3 미배선) 런타임 유실 없음.
    pub fn snapshot(&self) -> Vec<OutputChunk> {
        self.replay
            .lock()
            .expect("replay poisoned")
            .snapshot()
            .into_iter()
            .filter_map(|s| match s.event {
                OutputEvent::TerminalBytes(data) => Some(OutputChunk { seq: s.seq, data }),
                // ★무음 유실 관측 훅★: subscribe_from replay 경로는 payload-generic 이라 정상 전달돼
                // 두 복원 경로가 비대칭 — B3 배선 순간 무음 유실이 되므로 drop 을 warn 으로 관측
                // 가능하게 남긴다.
                ref other => {
                    // variant 태그만 로그(payload 내용은 로그에 싣지 않음 — 민감/대용량 회피).
                    let kind = match other {
                        OutputEvent::TerminalBytes(_) => "TerminalBytes", // 위 arm 이 처리 — 도달 안 함
                        OutputEvent::TextDelta { .. } => "TextDelta",
                        OutputEvent::ToolCall { .. } => "ToolCall",
                        OutputEvent::Usage { .. } => "Usage",
                        OutputEvent::MessageDone { .. } => "MessageDone",
                        OutputEvent::Error(_) => "Error",
                        OutputEvent::Structured { .. } => "Structured",
                    };
                    tracing::warn!(
                        seq = s.seq,
                        kind,
                        "structured event dropped from wire snapshot (B7 미배선 — get_snapshot 경로 무음 유실)"
                    );
                    None
                }
            })
            .collect()
    }

    pub fn status(&self) -> AgentStatus {
        self.status.lock().expect("status poisoned").clone()
    }
}

// ── S15 B4: payload-generic replay 버퍼 ───────────────────────────────────────────
//
// ADR-0002: 출력 종류 비가정 — 버퍼가 바이트/이벤트를 차별하지 않는다.

#[derive(Debug, Clone)]
pub struct StoredOutput {
    pub seq: u64,
    pub event: OutputEvent,
    /// eviction 바이트 예산 산정용 근사 크기(정확한 wire 크기 아님 — 아래 estimate_cost_bytes 참조).
    pub cost_bytes: usize,
}

/// OutputEvent 의 **eviction 예산용** 크기 근사.
///
/// ★왜 근사인가(TRD 핵심)★: core 는 직렬화를 못 한다(ADR-0003 — wire 변환은 daemon adapter 몫).
/// 그래서 정확한 wire 바이트 수를 계산할 수 없다. 하지만 큰 `args_json`(도구 인자)·`json`(Structured)
/// 이벤트가 "건수 1" 로만 세지면 max_bytes(2MB) 상한을 우회해 버퍼가 무한정 커진다. 이를 막으려
/// **payload 문자열 필드들의 바이트 길이 합**을 구조적으로 근사해 예산에 반영한다. 이 값은 eviction
/// 판단 전용이며 정확한 직렬화 크기가 아니다(태그·구분자·escape 오버헤드 무시).
fn estimate_cost_bytes(event: &OutputEvent) -> usize {
    match event {
        OutputEvent::TerminalBytes(v) => v.len(),
        OutputEvent::TextDelta {
            text,
            turn_id,
            message_id,
        } => text.len() + opt_len(turn_id) + opt_len(message_id),
        OutputEvent::ToolCall {
            name,
            args_json,
            id,
            turn_id,
            message_id,
        } => name.len() + args_json.len() + opt_len(id) + opt_len(turn_id) + opt_len(message_id),
        // Usage 는 고정 크기 수치 필드 — turn_id 문자열만 반영(u64 두 개는 무시).
        OutputEvent::Usage { turn_id, .. } => opt_len(turn_id),
        OutputEvent::MessageDone {
            turn_id,
            message_id,
        } => opt_len(turn_id) + opt_len(message_id),
        OutputEvent::Error(s) => s.len(),
        OutputEvent::Structured { kind, json } => kind.len() + json.len(),
    }
}

fn opt_len(s: &Option<String>) -> usize {
    s.as_deref().map(str::len).unwrap_or(0)
}

/// 늦게 붙는 창을 위한 출력 replay ring buffer — 상한 2MB **그리고** event 수 상한.
/// StoredOutput 전용 구체 타입으로 둔다(제네릭 `Ring<T>` 는 이 프로젝트에 다른 저장 대상이 없어
/// 과함 — 단순한 쪽).
///
/// ★event 수 상한 이유(S12 consult, GPT 단독 catch): byte 상한만 있으면 1바이트 청크가
/// 폭주할 때 event 수가 수백만으로 불어, 신규 구독자가 replay를 받을 때 bounded mpsc를
/// 즉시 가득 채워 매 재연결이 slow-consumer로 끊기는 영구 루프가 생긴다. 둘 중 하나라도
/// 초과하면 앞부터 evict.
pub struct Ring {
    items: VecDeque<StoredOutput>,
    total_bytes: usize,
    max_bytes: usize,
    max_events: usize,
}

impl Ring {
    pub fn new() -> Self {
        Self {
            items: VecDeque::new(),
            total_bytes: 0,
            max_bytes: 2 * 1024 * 1024,
            // 4096: 데몬 WS 송신 큐 cap(예 4608) − control_slack(512) 이하로 잡아
            // replay만으로 신규 구독자 큐가 넘치지 않게 한다.
            max_events: 4096,
        }
    }

    /// StoredOutput 을 뒤에 추가하고, 이중 상한(cost_bytes 합 OR 건수) 초과 시 앞부터 evict.
    /// total_bytes 는 push 마다 cost_bytes 로 누적, evict 마다 차감해 항상 items 합과 일치한다.
    pub fn push(&mut self, item: StoredOutput) {
        self.total_bytes += item.cost_bytes;
        self.items.push_back(item);
        // ★최신 1건 보존 불변식(len() > 1 가드)★: 방금 push 한 단일 이벤트의 cost_bytes 가
        // max_bytes(2MB)를 홀로 초과하면(예: 큰 args_json/Structured.json), len() > 1 가드가 없을 때
        // eviction 루프가 그 최신 이벤트까지 pop_front 로 빼내 버퍼가 비어 버린다 → 늦게 붙는 구독자가
        // **최신 seq 를 통째로 놓친다**. 그래서 오래된 것만 evict 하고 마지막 1건은 항상 남긴다.
        // 트레이드오프: 단일 이벤트가 예산을 넘으면 메모리는 "가장 큰 단일 이벤트 크기"까지 초과할 수
        // 있으나(byte 상한 일시 위반), replay 정합(늦은 구독자가 늘 최신을 본다) > 엄격 byte 상한.
        while self.items.len() > 1
            && (self.total_bytes > self.max_bytes || self.items.len() > self.max_events)
        {
            if let Some(oldest) = self.items.pop_front() {
                self.total_bytes -= oldest.cost_bytes;
            } else {
                break;
            }
        }
    }

    /// 현재 버퍼 전체를 clone 해 반환(seq 오름차순). 호출부가 after_seq 필터는 partition_point 로.
    /// 호출부(subscribe)가 lock 밖에서 borrow 하려면 소유 스냅샷이 필요하다(락 보유 시간 최소화).
    pub fn snapshot(&self) -> Vec<StoredOutput> {
        self.items.iter().cloned().collect()
    }
}

impl Default for Ring {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 받은 출력을 (seq, bytes, is_event)로 순서대로 수집하는 mock OutputSink.
    struct MockSink {
        id: SinkId,
        events: Mutex<Vec<(u64, Vec<u8>, bool)>>,
    }

    impl MockSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                id: uuid::Uuid::new_v4(),
                events: Mutex::new(Vec::new()),
            })
        }

        fn seqs(&self) -> Vec<u64> {
            self.events.lock().unwrap().iter().map(|e| e.0).collect()
        }

        fn len(&self) -> usize {
            self.events.lock().unwrap().len()
        }
    }

    impl OutputSink for MockSink {
        fn send(&self, frame: OutputFrame<'_>) -> Result<(), crate::agent::types::SinkError> {
            let (bytes, is_event) = match frame.payload {
                OutputPayload::Bytes(b) => (b.to_vec(), false),
                OutputPayload::Event(e) => (format!("{e:?}").into_bytes(), true),
            };
            self.events
                .lock()
                .unwrap()
                .push((frame.seq, bytes, is_event));
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    struct MockStatusSink {
        statuses: Mutex<Vec<AgentStatus>>,
        turn_ends: Mutex<Vec<(AgentId, u32)>>,
    }

    impl MockStatusSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                statuses: Mutex::new(Vec::new()),
                turn_ends: Mutex::new(Vec::new()),
            })
        }

        fn statuses(&self) -> Vec<AgentStatus> {
            self.statuses.lock().unwrap().clone()
        }

        fn turn_ends(&self) -> Vec<(AgentId, u32)> {
            self.turn_ends.lock().unwrap().clone()
        }
    }

    impl StatusSink for MockStatusSink {
        fn status_changed(&self, _id: AgentId, status: AgentStatus, _epoch: u32) {
            self.statuses.lock().unwrap().push(status);
        }
        fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
        fn turn_ended(&self, id: AgentId, epoch: u32) {
            self.turn_ends.lock().unwrap().push((id, epoch));
        }
    }

    use crate::agent::types::AgentInfo;

    fn new_core(status_sink: Arc<dyn StatusSink>) -> OutputCore {
        OutputCore::new(uuid::Uuid::new_v4(), 0, status_sink, TurnWiring::detached())
    }

    // ── ADR-0113: emit → 턴 관측 표 + 턴 종료 push ───────────────────────────────────

    /// 표 + claude 분류자를 꽂은 core(운영 spawn_session 배선과 동형 — 같은 dispatch 로 뽑는다).
    fn core_with_turns(
        status_sink: Arc<dyn StatusSink>,
        epoch: u32,
    ) -> (OutputCore, Arc<TurnObservations>, AgentId) {
        use crate::agent::profile::{AgentCommand, ClaudeOutputFormat};
        let id = uuid::Uuid::new_v4();
        let turns = Arc::new(TurnObservations::new());
        let core = OutputCore::new(
            id,
            epoch,
            status_sink,
            TurnWiring::new(
                turns.clone(),
                crate::agent::backend::turn_classifier(&AgentCommand::Claude {
                    extra_args: vec![],
                    output_format: ClaudeOutputFormat::StreamJson,
                }),
            ),
        );
        (core, turns, id)
    }

    fn delta() -> OutputEvent {
        OutputEvent::TextDelta {
            text: "x".into(),
            turn_id: None,
            message_id: None,
        }
    }

    fn message_done() -> OutputEvent {
        OutputEvent::MessageDone {
            turn_id: None,
            message_id: None,
        }
    }

    #[test]
    fn emit_records_turn_signals_into_the_shared_table() {
        let (core, turns, id) = core_with_turns(MockStatusSink::new(), 7);
        core.emit(delta());
        assert!(turns.is_in_turn(id, 7), "진행 신호 → 턴 중");
        core.emit(message_done());
        assert!(!turns.is_in_turn(id, 7), "종료 신호 → 턴 아님");
    }

    #[test]
    fn emit_of_non_turn_events_leaves_the_table_untouched() {
        let (core, turns, id) = core_with_turns(MockStatusSink::new(), 0);
        core.emit(OutputEvent::TerminalBytes(b"raw".to_vec()));
        core.emit(OutputEvent::Usage {
            input_tokens: 1,
            output_tokens: 2,
            turn_id: None,
        });
        core.emit(OutputEvent::Error("stream hiccup".into()));
        assert_eq!(
            turns.get(id, 0),
            None,
            "턴 신호가 아니면 항목을 만들지 않는다"
        );
    }

    #[test]
    fn every_message_done_pushes_a_turn_end_notification() {
        let sink = MockStatusSink::new();
        let (core, _turns, id) = core_with_turns(sink.clone(), 2);
        core.emit(delta());
        core.emit(message_done());
        core.emit(message_done());
        assert_eq!(sink.turn_ends(), vec![(id, 2), (id, 2)]);
    }

    #[test]
    fn progress_events_do_not_push_turn_end() {
        let sink = MockStatusSink::new();
        let (core, _turns, _id) = core_with_turns(sink.clone(), 0);
        core.emit(delta());
        core.emit(OutputEvent::Structured {
            kind: "user".into(),
            json: "{}".into(),
        });
        assert!(sink.turn_ends().is_empty());
    }

    #[test]
    fn a_late_echo_after_finish_cannot_resurrect_the_entry() {
        let (core, turns, id) = core_with_turns(MockStatusSink::new(), 0);
        core.emit(delta());
        assert!(turns.is_in_turn(id, 0), "전제: 턴 중으로 관측됨");

        core.finish(TerminalReason::Exited { code: Some(0) });
        assert_eq!(turns.get(id, 0), None, "finish 가 자기 항목을 거둔다");

        // 막혀 있던 주입 스레드가 이제야 에코를 낸다(같은 epoch).
        core.emit(OutputEvent::Structured {
            kind: "user".into(),
            json: "{}".into(),
        });
        assert_eq!(
            turns.get(id, 0),
            None,
            "종료된 화신은 항목을 되살리지 못한다"
        );
        assert!(turns.in_turn_snapshot().is_empty());
    }

    #[test]
    fn a_dead_incarnations_late_echo_cannot_delete_the_live_ones_observation() {
        use crate::agent::profile::{AgentCommand, ClaudeOutputFormat};
        let id = uuid::Uuid::new_v4();
        let turns = Arc::new(TurnObservations::new());
        let classify = crate::agent::backend::turn_classifier(&AgentCommand::Claude {
            extra_args: vec![],
            output_format: ClaudeOutputFormat::StreamJson,
        });
        let wiring = |t: &Arc<TurnObservations>| TurnWiring::new(t.clone(), classify);

        // 앞선 화신(epoch 0) — 한동안 돌다 종료한다.
        let dead = OutputCore::new(id, 0, MockStatusSink::new(), wiring(&turns));
        for _ in 0..5 {
            dead.emit(delta());
        }
        dead.finish(TerminalReason::Killed);
        assert_eq!(turns.get(id, 0), None, "종료가 자기 항목을 거둔다(전제)");

        // 새 화신(epoch 1) — 같은 AgentId, 새 core 라 seq 는 0 부터 다시 센다.
        let live = OutputCore::new(id, 1, MockStatusSink::new(), wiring(&turns));
        live.emit(delta());
        assert!(turns.is_in_turn(id, 1), "산 화신이 턴 중으로 관측됨(전제)");

        // 죽은 화신의 주입 스레드가 이제야 깨어 에코를 낸다 — seq 는 산 화신의 것보다 **크다**.
        dead.emit(OutputEvent::Structured {
            kind: "user".into(),
            json: "{}".into(),
        });
        assert!(
            turns.is_in_turn(id, 1),
            "죽은 화신의 지각 emit 이 산 화신의 관측을 덮거나 지우면 안 된다"
        );
        assert_eq!(
            turns.get(id, 0),
            None,
            "죽은 화신의 항목이 되살아나지도 않는다"
        );
    }

    #[test]
    fn a_backend_without_a_turn_mapping_never_records_a_signal() {
        use crate::agent::profile::AgentCommand;
        let id = uuid::Uuid::new_v4();
        let turns = Arc::new(TurnObservations::new());
        let core = OutputCore::new(
            id,
            0,
            MockStatusSink::new(),
            TurnWiring::new(
                turns.clone(),
                crate::agent::backend::turn_classifier(&AgentCommand::Shell {
                    program: "cmd.exe".into(),
                    args: vec![],
                }),
            ),
        );
        core.emit(delta());
        core.emit(OutputEvent::Structured {
            kind: "session_meta".into(),
            json: "{}".into(),
        });
        core.emit(message_done());
        assert_eq!(turns.get(id, 0), None, "분류자가 침묵하면 관측도 없다");
    }

    #[test]
    fn an_unwired_core_records_nothing_at_all() {
        let sink = MockStatusSink::new();
        let core = new_core(sink.clone());
        core.emit(delta());
        core.emit(message_done());
        assert!(sink.turn_ends().is_empty(), "배선 없으면 통지도 없다");
    }

    #[test]
    fn seed_never_bootstraps_turn_observation_from_a_transcript() {
        let sink = MockStatusSink::new();
        let (core, turns, id) = core_with_turns(sink.clone(), 0);
        core.seed(vec![delta(), message_done(), delta()]);
        assert_eq!(turns.get(id, 0), None, "seed 는 관측이 아니다");
        assert!(sink.turn_ends().is_empty(), "seed 는 통지도 내지 않는다");
        // 라이브 emit 부터 관측이 시작된다(seed 가 관측을 막아 버리는 것도 아니다).
        core.emit(delta());
        assert!(turns.is_in_turn(id, 0));
    }

    #[test]
    fn emit_increments_seq_and_fans_out() {
        let core = new_core(MockStatusSink::new());
        let sink = MockSink::new();
        core.subscribe(sink.clone());

        core.emit(OutputEvent::TerminalBytes(b"hello".to_vec()));
        core.emit(OutputEvent::TerminalBytes(b"world".to_vec()));

        assert_eq!(sink.seqs(), vec![0, 1]);
        assert_eq!(sink.len(), 2);
        {
            let ev = sink.events.lock().unwrap();
            assert_eq!(ev[0].1, b"hello");
            assert_eq!(ev[1].1, b"world");
            assert!(!ev[0].2, "TerminalBytes 는 OutputPayload::Bytes 로 와야 함");
            assert!(!ev[1].2);
        }
        let snap = core.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].seq, 0);
        assert_eq!(snap[1].seq, 1);
        assert_eq!(snap[0].data, b"hello");
        assert_eq!(snap[1].data, b"world");
    }

    #[test]
    fn emit_structured_event_fans_out_as_event_and_replays() {
        let core = new_core(MockStatusSink::new());

        let live = MockSink::new();
        core.subscribe(live.clone());
        core.emit(OutputEvent::TextDelta {
            text: "hi".into(),
            turn_id: None,
            message_id: None,
        });
        core.emit(OutputEvent::TerminalBytes(b"raw".to_vec()));

        {
            let ev = live.events.lock().unwrap();
            assert_eq!(ev.len(), 2);
            assert!(
                ev[0].2,
                "구조화 이벤트는 OutputPayload::Event 로 fanout 돼야 함"
            );
            assert!(
                String::from_utf8_lossy(&ev[0].1).contains("TextDelta"),
                "Event payload 가 해당 이벤트를 담아야 함"
            );
            assert!(!ev[1].2, "TerminalBytes 는 Bytes 로 fanout");
            assert_eq!(ev[1].1, b"raw");
        }

        let late = MockSink::new();
        core.subscribe(late.clone());
        {
            let ev = late.events.lock().unwrap();
            assert_eq!(ev.len(), 2);
            assert_eq!(ev[0].0, 0);
            assert!(ev[0].2, "replay 된 구조화 이벤트도 Event payload");
            assert_eq!(ev[1].0, 1);
            assert!(!ev[1].2);
            assert_eq!(ev[1].1, b"raw");
        }
    }

    #[test]
    fn seed_pushes_to_ring_in_order_without_fanout_then_live_seq_continues() {
        let core = new_core(MockStatusSink::new());

        // (1) 구독자 없는 상태에서 과거 이벤트 3건 seed(=resume 시 .jsonl 복원분 흉내).
        core.seed(vec![
            OutputEvent::Structured {
                kind: "user".into(),
                json: r#"{"type":"text","text":"과거 질문"}"#.into(),
            },
            OutputEvent::TextDelta {
                text: "과거 답변".into(),
                turn_id: None,
                message_id: None,
            },
            OutputEvent::MessageDone {
                turn_id: None,
                message_id: None,
            },
        ]);

        // Ring 에 seq 0,1,2 로 순서대로 적재됐는지(snapshot 은 TerminalBytes 만 변환하므로 seq 검증은
        // 아래 늦은 구독자 replay 로 한다 — snapshot() 은 구조화 이벤트를 스킵).
        let late = MockSink::new();
        core.subscribe(late.clone());
        assert_eq!(
            late.seqs(),
            vec![0, 1, 2],
            "seed 한 과거 3건이 Ring 에 seq 0,1,2 로 순서대로 있어야 하고 replay 로 전달됨"
        );

        core.emit(OutputEvent::TextDelta {
            text: "새 라이브 토큰".into(),
            turn_id: None,
            message_id: None,
        });
        assert_eq!(
            late.seqs(),
            vec![0, 1, 2, 3],
            "seed 뒤 라이브 emit 의 seq 가 seed 마지막+1 로 이어져야 함(gap·중복 없음)"
        );
    }

    #[test]
    fn seed_does_not_fanout_to_existing_subscriber() {
        let core = new_core(MockStatusSink::new());
        // (비정상 순서지만 방어 검증용) 구독자를 먼저 붙인 뒤 seed 한다.
        let sink = MockSink::new();
        core.subscribe(sink.clone());

        core.seed(vec![OutputEvent::TextDelta {
            text: "seed".into(),
            turn_id: None,
            message_id: None,
        }]);

        assert_eq!(sink.len(), 0, "seed 는 기존 구독자로 fanout 하지 않아야 함");

        core.emit(OutputEvent::TextDelta {
            text: "live".into(),
            turn_id: None,
            message_id: None,
        });
        assert_eq!(
            sink.seqs(),
            vec![1],
            "라이브 emit 만 fanout, seq 는 seed 다음"
        );
    }

    #[test]
    fn subscribe_replays_then_lives_without_seq_gap() {
        let core = new_core(MockStatusSink::new());

        // 구독 전에 2건 emit → replay에만 쌓임.
        core.emit(OutputEvent::TerminalBytes(b"a".to_vec()));
        core.emit(OutputEvent::TerminalBytes(b"b".to_vec()));

        let sink = MockSink::new();
        core.subscribe(sink.clone());
        assert_eq!(sink.seqs(), vec![0, 1]);

        core.emit(OutputEvent::TerminalBytes(b"c".to_vec()));
        assert_eq!(sink.seqs(), vec![0, 1, 2]);
    }

    // ── S15 B4 Ring 단위테스트(격리) ──────────────────────────────────────────────

    fn stored(seq: u64, event: OutputEvent) -> StoredOutput {
        let cost_bytes = estimate_cost_bytes(&event);
        StoredOutput {
            seq,
            event,
            cost_bytes,
        }
    }

    #[test]
    fn ring_push_snapshot_preserves_order_and_seq() {
        let mut ring = Ring::new();
        ring.push(stored(0, OutputEvent::TerminalBytes(b"a".to_vec())));
        ring.push(stored(1, OutputEvent::TerminalBytes(b"bb".to_vec())));
        ring.push(stored(2, OutputEvent::Error("boom".into())));

        let snap = ring.snapshot();
        assert_eq!(
            snap.iter().map(|s| s.seq).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert!(matches!(&snap[0].event, OutputEvent::TerminalBytes(v) if v == b"a"));
        assert!(matches!(&snap[2].event, OutputEvent::Error(s) if s == "boom"));
    }

    #[test]
    fn ring_evicts_on_byte_budget_independent_of_count() {
        let mut ring = Ring::new();
        // 각 ~1MB args_json 이벤트 3건 → cost 합 ~3MB > 2MB → 가장 오래된 것 evict.
        let big = "x".repeat(1024 * 1024);
        ring.push(stored(
            0,
            OutputEvent::ToolCall {
                name: "t".into(),
                args_json: big.clone(),
                id: None,
                turn_id: None,
                message_id: None,
            },
        ));
        ring.push(stored(
            1,
            OutputEvent::ToolCall {
                name: "t".into(),
                args_json: big.clone(),
                id: None,
                turn_id: None,
                message_id: None,
            },
        ));
        ring.push(stored(
            2,
            OutputEvent::ToolCall {
                name: "t".into(),
                args_json: big.clone(),
                id: None,
                turn_id: None,
                message_id: None,
            },
        ));
        let snap = ring.snapshot();
        assert!(
            snap.len() < 3,
            "cost_bytes 합이 2MB 초과 시 건수 상한과 무관하게 evict 돼야 함(len={})",
            snap.len()
        );
        assert_eq!(snap.last().unwrap().seq, 2);
    }

    #[test]
    fn ring_evicts_on_event_count_cap() {
        // 작은 이벤트 5000건 → byte 예산엔 한참 못 미치지만 건수 상한(4096)에 걸려 cap.
        let mut ring = Ring::new();
        for seq in 0..5000u64 {
            ring.push(stored(seq, OutputEvent::TerminalBytes(vec![b'x'])));
        }
        let snap = ring.snapshot();
        assert_eq!(snap.len(), 4096);
        // 가장 오래된 것부터 evict → 남은 첫 seq = 5000-4096 = 904.
        assert_eq!(snap.first().unwrap().seq, 904);
        assert_eq!(snap.last().unwrap().seq, 4999);
    }

    #[test]
    fn ring_preserves_latest_when_single_event_exceeds_byte_budget() {
        // FIX-A: 방금 push 한 단일 이벤트의 cost_bytes 가 max_bytes(2MB)를 홀로 초과해도,
        // 최신 1건 보존 불변식(len() > 1 가드)에 의해 그 이벤트는 replay 버퍼에 남아야 한다.
        let mut ring = Ring::new();
        let huge = "y".repeat(3 * 1024 * 1024); // > 2MB (max_bytes)
        ring.push(stored(
            42,
            OutputEvent::Structured {
                kind: "big".into(),
                json: huge,
            },
        ));
        let snap = ring.snapshot();
        assert_eq!(snap.len(), 1, "단일 초과 이벤트라도 최신 1건은 보존돼야 함");
        assert_eq!(snap[0].seq, 42, "보존된 이벤트는 방금 push 한 최신 seq");
    }

    #[test]
    fn ring_evicts_only_old_when_latest_exceeds_budget() {
        // FIX-A: 오래된 작은 이벤트들 + 예산을 홀로 초과하는 큰 최신 이벤트.
        let mut ring = Ring::new();
        ring.push(stored(0, OutputEvent::TerminalBytes(b"old0".to_vec())));
        ring.push(stored(1, OutputEvent::TerminalBytes(b"old1".to_vec())));
        let huge = "z".repeat(3 * 1024 * 1024); // > 2MB → 이것만으로 예산 초과
        ring.push(stored(
            2,
            OutputEvent::Structured {
                kind: "big".into(),
                json: huge,
            },
        ));
        let snap = ring.snapshot();
        assert_eq!(snap.len(), 1, "오래된 것만 빠지고 최신 1건 남아야 함");
        assert_eq!(snap[0].seq, 2, "남은 것은 최신 이벤트");
    }

    #[test]
    fn ring_cost_bytes_reflects_terminal_and_structured() {
        // TerminalBytes → v.len().
        assert_eq!(
            estimate_cost_bytes(&OutputEvent::TerminalBytes(vec![0u8; 100])),
            100
        );
        // 구조화(ToolCall) → name + args_json + optional 문자열 합.
        let cost = estimate_cost_bytes(&OutputEvent::ToolCall {
            name: "read".into(),           // 4
            args_json: "{\"p\":1}".into(), // 7
            id: Some("abc".into()),        // 3
            turn_id: None,
            message_id: None,
        });
        assert_eq!(cost, 4 + 7 + 3);
        // TextDelta → text + optional.
        let cost2 = estimate_cost_bytes(&OutputEvent::TextDelta {
            text: "hello".into(),       // 5
            turn_id: Some("t1".into()), // 2
            message_id: None,
        });
        assert_eq!(cost2, 5 + 2);
    }

    #[test]
    fn finish_finalizes_exactly_once() {
        let status_sink = MockStatusSink::new();
        let core = new_core(status_sink.clone());

        core.finish(TerminalReason::Killed);
        core.finish(TerminalReason::Killed);

        let statuses = status_sink.statuses();
        assert_eq!(statuses.len(), 1);
        assert!(matches!(statuses[0], AgentStatus::Killed));
        assert!(matches!(core.status(), AgentStatus::Killed));
    }

    /// ADR-0019 reaper hook(on_terminal) 1회 보장: finalize 승자 경로(finalized.swap 통과)에서
    /// hook 이 정확히 1회 호출되고, 중복 finish(swap 패자)에서는 0회임을 단언한다.
    #[test]
    fn on_terminal_hook_fires_exactly_once() {
        let core = new_core(MockStatusSink::new());
        let calls = Arc::new(AtomicU64::new(0));

        let c = calls.clone();
        core.set_on_terminal(Box::new(move |_reason: TerminalReason| {
            c.fetch_add(1, Ordering::SeqCst);
        }));

        core.finish(TerminalReason::Exited { code: Some(0) });
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "finalize 승자 경로에서 on_terminal hook 이 정확히 1회 호출돼야 함"
        );

        core.finish(TerminalReason::Killed);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "중복 finish(finalize 패자)에서 on_terminal hook 이 다시 호출됨(1회 위반)"
        );
    }

    #[test]
    fn subscribe_from_resume_sends_only_tail() {
        let core = new_core(MockStatusSink::new());
        for i in 0..5u8 {
            core.emit(OutputEvent::TerminalBytes(vec![b'a' + i]));
        }
        let sink = MockSink::new();
        let out = core.subscribe_from(sink.clone(), Some(2), true, |_| {});

        assert_eq!(sink.seqs(), vec![3, 4]);
        assert_eq!(out.kind, ReplayKind::Resumed);
        assert_eq!(out.replayed, 2);
        assert_eq!(out.replay_from, 3);
    }

    #[test]
    fn subscribe_from_truncated_when_after_below_oldest() {
        let core = new_core(MockStatusSink::new());
        // 1바이트 청크 5000개 emit → event 상한(4096) 초과로 oldest=904 까지 evict.
        for _ in 0..5000u64 {
            core.emit(OutputEvent::TerminalBytes(vec![b'x']));
        }
        let sink = MockSink::new();
        let out = core.subscribe_from(sink.clone(), Some(10), true, |_| {});

        assert_eq!(out.kind, ReplayKind::Truncated);
        assert_eq!(out.oldest_seq, 904);
        assert_eq!(sink.seqs().first().copied(), Some(904));
        assert_eq!(out.replay_from, 904);
    }

    #[test]
    fn subscribe_from_epoch_mismatch_is_from_oldest() {
        let core = new_core(MockStatusSink::new());
        for i in 0..3u8 {
            core.emit(OutputEvent::TerminalBytes(vec![b'a' + i]));
        }
        let sink = MockSink::new();
        let out = core.subscribe_from(sink.clone(), Some(1), false, |_| {});

        assert_eq!(out.kind, ReplayKind::FromOldest);
        assert_eq!(sink.seqs(), vec![0, 1, 2]);
        assert_eq!(out.replay_from, 0);
    }

    #[test]
    fn subscribe_from_caught_up_sends_nothing() {
        let core = new_core(MockStatusSink::new());
        for i in 0..3u8 {
            core.emit(OutputEvent::TerminalBytes(vec![b'a' + i]));
        }
        let sink = MockSink::new();
        // after_seq=2(=latest) → 보낼 tail 없음.
        let out = core.subscribe_from(sink.clone(), Some(2), true, |_| {});
        assert_eq!(out.kind, ReplayKind::Resumed);
        assert_eq!(out.replayed, 0);
        assert_eq!(out.replay_from, 3);
        assert_eq!(sink.len(), 0);

        core.emit(OutputEvent::TerminalBytes(b"d".to_vec()));
        assert_eq!(sink.seqs(), vec![3]);
    }

    #[test]
    fn subscribe_from_none_after_seq_is_from_oldest() {
        let core = new_core(MockStatusSink::new());
        for i in 0..3u8 {
            core.emit(OutputEvent::TerminalBytes(vec![b'a' + i]));
        }
        let sink = MockSink::new();
        let out = core.subscribe_from(sink.clone(), None, true, |_| {});

        assert_eq!(out.kind, ReplayKind::FromOldest);
        assert_eq!(sink.seqs(), vec![0, 1, 2]);
        assert_eq!(out.replay_from, 0);
    }

    /// M-A fix 검증: on_ready 콜백이 (1) replay 전송 **전**에, (2) 정확히 1회 호출되고,
    /// (3) 콜백이 받는 outcome 이 반환 outcome 과 동일(단일 스냅샷 기준)임을 확인.
    #[test]
    fn subscribe_from_calls_on_ready_before_replay() {
        struct OrderSink {
            id: SinkId,
            replay_started: Arc<AtomicBool>,
        }
        impl OutputSink for OrderSink {
            fn send(&self, _frame: OutputFrame<'_>) -> Result<(), crate::agent::types::SinkError> {
                self.replay_started.store(true, Ordering::SeqCst);
                Ok(())
            }
            fn sink_id(&self) -> SinkId {
                self.id
            }
        }

        let core = new_core(MockStatusSink::new());
        for i in 0..3u8 {
            core.emit(OutputEvent::TerminalBytes(vec![b'a' + i]));
        }

        let replay_started = Arc::new(AtomicBool::new(false));
        let sink = Arc::new(OrderSink {
            id: uuid::Uuid::new_v4(),
            replay_started: replay_started.clone(),
        });

        let call_count = Arc::new(AtomicU64::new(0));
        // 콜백이 본 outcome 을 캡처해 반환 outcome 과 비교(SubscribeOutcome 는 Copy).
        let seen: Arc<Mutex<Option<SubscribeOutcome>>> = Arc::new(Mutex::new(None));

        let cc = call_count.clone();
        let started = replay_started.clone();
        let seen_cb = seen.clone();
        let out = core.subscribe_from(sink, Some(1), true, move |outcome| {
            assert!(
                !started.load(Ordering::SeqCst),
                "on_ready 는 replay 전송 전에 호출돼야 함"
            );
            cc.fetch_add(1, Ordering::SeqCst);
            *seen_cb.lock().unwrap() = Some(*outcome);
        });

        assert_eq!(call_count.load(Ordering::SeqCst), 1);
        // replay 가 실제로 전송됐는지(after_seq=1 → seq 2 전송) → started true.
        assert!(replay_started.load(Ordering::SeqCst));
        let seen = seen.lock().unwrap().expect("콜백이 호출됨");
        assert_eq!(seen.kind, out.kind);
        assert_eq!(seen.oldest_seq, out.oldest_seq);
        assert_eq!(seen.latest_seq, out.latest_seq);
        assert_eq!(seen.replay_from, out.replay_from);
        assert_eq!(out.kind, ReplayKind::Resumed);
    }

    #[test]
    fn enter_exiting_true_when_running_false_after_terminal() {
        let status_sink = MockStatusSink::new();
        let core = new_core(status_sink.clone());

        assert!(core.enter_exiting());
        assert!(matches!(core.status(), AgentStatus::Exiting));
        assert!(matches!(
            status_sink.statuses().last().unwrap(),
            AgentStatus::Exiting
        ));

        core.finish(TerminalReason::Exited { code: Some(0) });
        assert!(!core.enter_exiting());
    }

    /// ADR-0079 회귀 방지: 동시 emit 하에서도 replay ring 이 seq 오름차순(단조)을 유지하는지.
    ///
    /// ★왜 이 테스트가 성립하나(hermetic)★: pump 스레드와 write_input synthetic echo 가 동시에 emit 을
    ///   부르는 실제 상황을 여러 스레드의 emit 루프로 재현한다. 확률적이지만 스레드×반복이 크면
    ///   발급/push 역전 창을 거의 확실히 밟아 회귀를 잡는다(플래키하지 않게 반복 수를 넉넉히 잡음).
    #[test]
    fn concurrent_emit_keeps_replay_ring_monotonic() {
        let core = Arc::new(new_core(MockStatusSink::new()));
        const THREADS: u64 = 4;
        const PER_THREAD: u64 = 500; // 4*500 = 2000 < max_events(4096) → eviction 없음.

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let core = core.clone();
                std::thread::spawn(move || {
                    for _ in 0..PER_THREAD {
                        // 작은 payload — 2MB max_bytes 도 안 건드림. emit 이 유일한 seq 소비자.
                        core.emit(OutputEvent::TerminalBytes(vec![b'x']));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("emit thread panicked");
        }

        let stored = core.replay.lock().expect("replay poisoned").snapshot();
        let seqs: Vec<u64> = stored.iter().map(|s| s.seq).collect();
        assert_eq!(
            seqs.len() as u64,
            THREADS * PER_THREAD,
            "eviction 없이 전량 저장돼야(검사 완전성)"
        );
        assert!(
            seqs.windows(2).all(|w| w[0] < w[1]),
            "replay ring 은 seq 로 엄격 오름차순이어야 한다(동시 emit 원자성): {seqs:?}"
        );
    }
}
