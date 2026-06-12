//! OutputCore — 에이전트 1개의 출력 측 핵심 상태(seq/replay/subscribers/status)와
//! 그 위의 동작(emit/finish/subscribe/...)을 transport·session에서 분리한 공용 struct.
//!
//! 왜 분리하는가: 출력 fanout·종료 전이·구독은 PTY든 API든 transport 종류와 무관하게
//! 동일하다. transport는 바이트·이벤트를 만들어 `emit`/`finish`로 넘기기만 하면 된다.
//!
//! 이 단계(stage 2)에선 아직 PtySession/manager에 배선되지 않는다. 독립 모듈로 완성한다.
//!
//! tauri import 0. drain.rs/session.rs의 락 규율·불변식을 글자 그대로 보존한다.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use base64::Engine as _;

use crate::pty::types::{
    AgentId, AgentStatus, OutputChunk, OutputEvent, OutputSink, PtyEvent, SinkId, StatusSink,
    TerminalReason,
};

/// 에이전트 1개의 출력 측 핵심 상태. 필드별 독립 Mutex(session.rs 모듈 주석의 분리 동기와 동일):
/// emit이 replay/subscribers lock만 짧게 잡는 동안 다른 경로(status 등)와 교착 없이 병행 가능.
pub struct OutputCore {
    // ── 불변 (생성 후 변경 없음) ──────────────────────────────
    id: AgentId,
    /// 이 세션 인스턴스의 epoch. status_changed에 동봉해 프론트가 stale terminal 알림을
    /// epoch 불일치로 버릴 수 있게 한다(S9 §18-d).
    epoch: u32,

    // ── 출력 시퀀스 / 상태 ────────────────────────────────────
    seq: AtomicU64,
    status: Mutex<AgentStatus>,
    /// finish 정확히 1회 게이트.
    finalized: AtomicBool,

    // ── 출력 구독 (독립 lock) ─────────────────────────────────
    subscribers: Mutex<Vec<Arc<dyn OutputSink>>>,

    // ── Replay buffer (독립 lock) ─────────────────────────────
    replay: Mutex<ReplayBuffer>,

    // ── 상태 알림 ─────────────────────────────────────────────
    status_sink: Arc<dyn StatusSink>,

    // ── pump thread 제어 (transport.start가 attach_pump로 적재) ─
    drain_handle: Mutex<Option<JoinHandle<()>>>,
    drain_done_rx: Mutex<Option<Receiver<()>>>,
}

impl OutputCore {
    /// 새 core 생성. status는 Running, seq 0, finalized false. pump 핸들은 None
    /// — transport.start가 attach_pump로 채운다(stage 3).
    pub fn new(id: AgentId, epoch: u32, status_sink: Arc<dyn StatusSink>) -> Self {
        Self {
            id,
            epoch,
            seq: AtomicU64::new(0),
            status: Mutex::new(AgentStatus::Running),
            finalized: AtomicBool::new(false),
            subscribers: Mutex::new(Vec::new()),
            replay: Mutex::new(ReplayBuffer::new()),
            status_sink,
            drain_handle: Mutex::new(None),
            drain_done_rx: Mutex::new(None),
        }
    }

    /// transport(pump)가 만든 출력 이벤트를 받아 replay 저장 + 구독자 fanout.
    /// **variant-agnostic** — 콘솔은 TerminalBytes만 처리하고, 미래 variant는 `_ => {}`로 무시.
    ///
    /// ★핵심 불변식 (drain.rs §10 규칙3)★
    /// - `sink.send()` 호출 시 어떤 lock도 보유하지 않는다. subscribers를 clone으로
    ///   스냅샷 뜨고 lock을 즉시 해제한 뒤, 복사본을 돌며 send.
    /// - replay lock과 subscribers lock을 동시에 보유하지 않는다(각각 짧게).
    ///   두 lock 동시 취득은 subscribe 함수 단독 예외이며 emit은 절대 금지.
    pub fn emit(&self, event: OutputEvent) {
        // 미래 variant(TextDelta/Usage/...)는 무시. 현재 enum이 단일 variant라 `_` 팔이
        // 도달 불가하지만, variant 추가 시 자동으로 무시 동작이 되도록 의도적으로 둔다.
        #[allow(unreachable_patterns)]
        match event {
            OutputEvent::TerminalBytes(bytes) => {
                // 3. seq 발급 + 이벤트 구성 (C2: 즉시 send — partial batch 정체 없음).
                let seq = self.seq.fetch_add(1, Ordering::Relaxed);
                let data = bytes;
                let event = PtyEvent {
                    agent_id: self.id,
                    seq,
                    data_b64: base64::engine::general_purpose::STANDARD.encode(&data),
                };

                // 4. replay 저장 — brief lock. 여기서 subscribers lock은 잡지 않는다(불변식 2).
                self.replay
                    .lock()
                    .expect("replay poisoned")
                    .push(OutputChunk { seq, data });

                // 5. ★불변식 1★ subscribers를 clone으로 스냅샷 뜨고 즉시 lock 해제 → lock 밖에서 send.
                //    send는 blocking 가능(IPC). lock을 쥔 채 send하면 subscribe/다른 send와 교착·정체.
                let sinks = self
                    .subscribers
                    .lock()
                    .expect("subscribers poisoned")
                    .clone();

                let mut dead = Vec::new();
                for sink in sinks {
                    if sink.send(event.clone()).is_err() {
                        dead.push(sink.sink_id());
                    }
                }

                // 6. 죽은 구독자 제거 — 다시 짧게 lock. (clone 시점 이후 새로 붙은 sink는 건드리지 않음)
                if !dead.is_empty() {
                    self.subscribers
                        .lock()
                        .expect("subscribers poisoned")
                        .retain(|s| !dead.contains(&s.sink_id()));
                }
            }
            _ => {}
        }
    }

    /// 종료 전이 — pump가 루프 탈출 후 1회 호출. finalize 정확히 1회 게이트로 중복 호출을 흡수한다.
    /// (drain.rs transition + spawn_drain_thread의 status_changed를 대체.)
    ///
    /// terminal 알림 주체는 pump(=여기) 단독. reason→AgentStatus 매핑은 impl-spec 표 그대로.
    pub fn finish(&self, reason: TerminalReason) {
        // finalize 1회: 이미 종료 처리됐으면 즉시 반환(idempotent).
        if self.finalized.swap(true, Ordering::AcqRel) {
            return;
        }

        // reason→AgentStatus 매핑(AgentStatus 변형 추가 금지, impl-spec 표):
        // Interrupted/Cancelled→Killed, StreamClosed→Exited{None}, Error(s)→Failed, 나머지 직역.
        let new_status = match reason {
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

        // status lock 해제 후 외부 호출(§10: status lock 보유 중 외부호출 금지).
        self.status_sink
            .status_changed(self.id, new_status, self.epoch);
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

    /// pump 종료 대기 — kill 6단계. done_rx를 take해 recv_timeout. 수신측이 이미 사라졌어도
    /// (타임아웃 후 detach) 무시 가능하도록 결과를 버린다.
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

        // (C4) subscribers lock 보유 시작 — drop 전까지 emit의 live send와 직렬화된다.
        let mut subscribers_guard = self.subscribers.lock().expect("subscribers poisoned");

        // (A) live 구독을 먼저 등록 → 이후 도착하는 live chunk는 이 sink에도 전달됨.
        subscribers_guard.push(sink.clone());

        // (B) subscribers 보유 중 replay 스냅샷 취득 (규칙 3의 유일한 허용 예외).
        let snapshot = {
            let replay_guard = self.replay.lock().expect("replay poisoned");
            replay_guard.snapshot()
        };

        // replay 전송 — snapshot의 seq와 이후 live chunk의 seq가 끊기지 않아 프론트가
        // seq로 dedup/정렬 가능. 막 등록된 sink라 send 실패는 unlikely → 무시(§7).
        for chunk in snapshot {
            let event = PtyEvent {
                agent_id: self.id,
                seq: chunk.seq,
                data_b64: base64::engine::general_purpose::STANDARD.encode(&chunk.data),
            };
            let _ = sink.send(event);
        }

        // lock 해제 → emit 재개. (명시적 drop으로 lock 보유 구간을 분명히 표시)
        drop(subscribers_guard);

        sink_id
    }

    /// 구독 해제 (창 닫힘 시 cleanup에서 호출). 해당 sink_id만 제거.
    pub fn unsubscribe(&self, sink_id: SinkId) {
        self.subscribers
            .lock()
            .expect("subscribers poisoned")
            .retain(|s| s.sink_id() != sink_id);
    }

    /// replay 스냅샷 — 늦게 붙는 창의 초기 복원용.
    pub fn snapshot(&self) -> Vec<OutputChunk> {
        self.replay.lock().expect("replay poisoned").snapshot()
    }

    /// 현재 상태 clone 반환.
    pub fn status(&self) -> AgentStatus {
        self.status.lock().expect("status poisoned").clone()
    }
}

/// 늦게 붙는 창을 위한 PTY 출력 ring buffer — 상한 2MB, 초과 시 앞부터 제거.
/// (장기 소속이 output_core.rs — session.rs에서 이동. PtySession.replay가 stage 3까지 이걸 import.)
pub struct ReplayBuffer {
    chunks: VecDeque<OutputChunk>,
    total_bytes: usize,
    max_bytes: usize,
}

impl ReplayBuffer {
    pub fn new() -> Self {
        Self {
            chunks: VecDeque::new(),
            total_bytes: 0,
            max_bytes: 2 * 1024 * 1024,
        }
    }

    pub fn push(&mut self, chunk: OutputChunk) {
        self.total_bytes += chunk.data.len();
        self.chunks.push_back(chunk);
        while self.total_bytes > self.max_bytes {
            if let Some(oldest) = self.chunks.pop_front() {
                self.total_bytes -= oldest.data.len();
            } else {
                break;
            }
        }
    }

    pub fn snapshot(&self) -> Vec<OutputChunk> {
        self.chunks.iter().cloned().collect()
    }
}

impl Default for ReplayBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 받은 PtyEvent를 순서대로 수집하는 mock OutputSink.
    struct MockSink {
        id: SinkId,
        events: Mutex<Vec<PtyEvent>>,
    }

    impl MockSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                id: uuid::Uuid::new_v4(),
                events: Mutex::new(Vec::new()),
            })
        }

        fn seqs(&self) -> Vec<u64> {
            self.events.lock().unwrap().iter().map(|e| e.seq).collect()
        }

        fn len(&self) -> usize {
            self.events.lock().unwrap().len()
        }
    }

    impl OutputSink for MockSink {
        fn send(&self, event: PtyEvent) -> Result<(), crate::pty::types::SinkError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }
        fn sink_id(&self) -> SinkId {
            self.id
        }
    }

    /// 받은 status 변경을 순서대로 수집하는 mock StatusSink.
    struct MockStatusSink {
        statuses: Mutex<Vec<AgentStatus>>,
    }

    impl MockStatusSink {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                statuses: Mutex::new(Vec::new()),
            })
        }

        fn statuses(&self) -> Vec<AgentStatus> {
            self.statuses.lock().unwrap().clone()
        }
    }

    impl StatusSink for MockStatusSink {
        fn status_changed(&self, _id: AgentId, status: AgentStatus, _epoch: u32) {
            self.statuses.lock().unwrap().push(status);
        }
        fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
    }

    use crate::pty::types::AgentInfo;

    fn new_core(status_sink: Arc<dyn StatusSink>) -> OutputCore {
        OutputCore::new(uuid::Uuid::new_v4(), 0, status_sink)
    }

    #[test]
    fn emit_increments_seq_and_fans_out() {
        let core = new_core(MockStatusSink::new());
        let sink = MockSink::new();
        core.subscribe(sink.clone());

        core.emit(OutputEvent::TerminalBytes(b"hello".to_vec()));
        core.emit(OutputEvent::TerminalBytes(b"world".to_vec()));

        // seq 0,1로 증가.
        assert_eq!(sink.seqs(), vec![0, 1]);
        // 구독자에 2건 전달.
        assert_eq!(sink.len(), 2);
        // replay에 2건 누적.
        let snap = core.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].seq, 0);
        assert_eq!(snap[1].seq, 1);
        assert_eq!(snap[0].data, b"hello");
        assert_eq!(snap[1].data, b"world");
    }

    #[test]
    fn subscribe_replays_then_lives_without_seq_gap() {
        let core = new_core(MockStatusSink::new());

        // 구독 전에 2건 emit → replay에만 쌓임.
        core.emit(OutputEvent::TerminalBytes(b"a".to_vec()));
        core.emit(OutputEvent::TerminalBytes(b"b".to_vec()));

        // 늦게 붙는 sink → replay 스냅샷이 먼저 전달돼야 함.
        let sink = MockSink::new();
        core.subscribe(sink.clone());
        assert_eq!(sink.seqs(), vec![0, 1]);

        // 이후 live emit → seq 끊김 없이 이어짐.
        core.emit(OutputEvent::TerminalBytes(b"c".to_vec()));
        assert_eq!(sink.seqs(), vec![0, 1, 2]);
    }

    #[test]
    fn finish_finalizes_exactly_once() {
        let status_sink = MockStatusSink::new();
        let core = new_core(status_sink.clone());

        core.finish(TerminalReason::Killed);
        core.finish(TerminalReason::Killed);

        // 2번 호출해도 status_sink에는 Killed 1회만.
        let statuses = status_sink.statuses();
        assert_eq!(statuses.len(), 1);
        assert!(matches!(statuses[0], AgentStatus::Killed));
        assert!(matches!(core.status(), AgentStatus::Killed));
    }

    #[test]
    fn enter_exiting_true_when_running_false_after_terminal() {
        let status_sink = MockStatusSink::new();
        let core = new_core(status_sink.clone());

        // Running 상태 → true + Exiting 알림.
        assert!(core.enter_exiting());
        assert!(matches!(core.status(), AgentStatus::Exiting));
        assert!(matches!(
            status_sink.statuses().last().unwrap(),
            AgentStatus::Exiting
        ));

        // terminal로 전이 후 → false(덮어쓰지 않음).
        core.finish(TerminalReason::Exited { code: Some(0) });
        assert!(!core.enter_exiting());
    }
}
