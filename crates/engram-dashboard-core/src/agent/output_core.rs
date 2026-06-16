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

use crate::agent::types::{
    AgentId, AgentStatus, OutputChunk, OutputEvent, OutputFrame, OutputSink, ReplayKind, SinkId,
    StatusSink, SubscribeOutcome, TerminalReason,
};

/// finalize 1회 종료 hook(ADR-0019). spawn_session 이 {id,epoch,intent,shutting_down,reaper_tx}
/// 를 캡처한 클로저를 주입하고, finalize 승자 경로에서 정확히 1회 호출된다.
type OnTerminalHook = Box<dyn Fn(TerminalReason) + Send + Sync>;

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

    // ── finalize 1회 hook (ADR-0019 reaper) ───────────────────
    /// finalize 승자(finalized.swap 통과) 경로에서 정확히 1회 호출되는 종료 hook.
    /// spawn_session 이 {id, epoch, intent, shutting_down, reaper_tx} 를 캡처한 클로저를
    /// 주입한다 — 그 안에서 intent·shutting_down 을 **그 순간 snapshot** 해 ReapMsg 를 송신한다.
    /// transport 는 이 의미를 모른다(transport 는 그냥 core.finish 만 부른다).
    /// 단위테스트는 OutputCore::new 만 쓰고 hook 을 주입하지 않으므로 Option(None=no-op).
    on_terminal: Mutex<Option<OnTerminalHook>>,
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
            on_terminal: Mutex::new(None),
        }
    }

    /// finalize-시점 hook 주입 — spawn_session 이 sessions 맵 등록 전에 1회 호출한다.
    /// 클로저는 finalized.swap 승자 경로에서만(=정확히 1회) 불린다. ★race 방지★:
    /// 클로저 내부에서 intent·shutting_down 을 그 순간 snapshot 해 ReapMsg 를 빌드·송신해야 한다.
    pub fn set_on_terminal(&self, hook: OnTerminalHook) {
        *self.on_terminal.lock().expect("on_terminal poisoned") = Some(hook);
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
        // single_match: variant-agnostic 의도(미래 variant 자동 무시)라 의도적으로 match 유지.
        #[allow(unreachable_patterns, clippy::single_match)]
        match event {
            OutputEvent::TerminalBytes(bytes) => {
                // 3. seq 발급 (C2: 즉시 send — partial batch 정체 없음).
                let seq = self.seq.fetch_add(1, Ordering::Relaxed);
                let data = bytes;

                // 4. ★replay 저장 먼저★ — brief lock. **순서 중요(gap 방지)**: replay.push가
                //    fanout보다 먼저여야, subscribe가 이 사이에 끼어들어도 새 sink는 replay에서
                //    이 seq를 받는다(최악 dup, 프론트 seq dedup이 흡수). 역순이면 gap 발생.
                //    raw를 replay에 1회 clone(N=1 구독 기준 구 base64 String clone과 실질 동률 — 구
                //    emit은 replay move(0) + sink마다 String clone, 신 emit은 replay clone 1 + borrow fanout).
                self.replay
                    .lock()
                    .expect("replay poisoned")
                    .push(OutputChunk {
                        seq,
                        data: data.clone(),
                    });

                // 5. ★불변식 1★ subscribers를 clone으로 스냅샷 뜨고 즉시 lock 해제 → lock 밖에서 send.
                //    send는 blocking/try_send 가능. lock을 쥔 채 send하면 subscribe/다른 send와 교착·정체.
                //    raw OutputFrame(borrow)을 넘긴다 — base64/wire 인코딩은 sink 책임(코어 transport-agnostic).
                let frame = OutputFrame {
                    agent_id: self.id,
                    epoch: self.epoch,
                    seq,
                    data: &data,
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

        // status lock 해제 후 외부 호출(§10: status lock 보유 중 외부호출 금지).
        self.status_sink
            .status_changed(self.id, new_status, self.epoch);

        // ★ADR-0019 reaper hook★: finalize 승자 경로에서 정확히 1회. status_sink 통지·done_tx·
        //   join_pump 동작은 위에서 그대로 보존하고 send 만 얹는다. 클로저 내부에서 intent·
        //   shutting_down 을 **그 순간** snapshot 해 ReapMsg 를 송신한다(reap 시점 live read 금지).
        //   on_terminal lock 은 짧게 잡고 즉시 clone 없이 호출 — 다른 core lock 미보유 구간이라 안전.
        //   (단위테스트·hook 미주입 세션은 None → no-op.)
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
        // raw OutputFrame(borrow) 전달 — 인코딩은 sink 책임.
        for chunk in &snapshot {
            let frame = OutputFrame {
                agent_id: self.id,
                epoch: self.epoch,
                seq: chunk.seq,
                data: &chunk.data,
            };
            let _ = sink.send(frame);
        }

        // lock 해제 → emit 재개. (명시적 drop으로 lock 보유 구간을 분명히 표시)
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
        // C4: subscribers lock 보유 시작 — emit live send 와 직렬화.
        let mut subscribers_guard = self.subscribers.lock().expect("subscribers poisoned");
        subscribers_guard.push(sink.clone());
        // subscribers 보유 중 replay 스냅샷(규칙3 유일 허용 예외, subscribe 와 동일).
        let snapshot = {
            let replay_guard = self.replay.lock().expect("replay poisoned");
            replay_guard.snapshot()
        };

        let oldest = snapshot.first().map(|c| c.seq).unwrap_or(0);
        let latest = snapshot.last().map(|c| c.seq).unwrap_or(0);

        let (kind, start_idx) = match after_seq {
            // epoch 불일치이거나 after_seq 미지정 → 전체 replay(안전 기본값).
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

        // 보낼 게 있으면 첫 seq, 없으면 "다음 live seq" 추정(after_seq+1 또는 latest+1).
        // ★replay 전송 전에 미리 계산★ — on_ready 가 정확한 outcome 을 받아야 TOCTOU 가 제거된다.
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

        // ★불변식 2 + TOCTOU 제거의 핵심★: replay frame 들을 sink 로 보내기 **직전**에,
        //   여전히 subscribers lock 을 보유한 채 on_ready 를 1회 호출한다. 데몬이 이 안에서
        //   SubscribeAck(control)를 conn_tx 에 try_send 하면, 그 enqueue 가 아래 replay 의
        //   conn_tx try_send(binary) 보다 반드시 먼저 일어난다(단일 writer FIFO → Ack→replay 순서).
        //   동시에 Ack 필드는 이 단일 스냅샷에서 나온 outcome 으로 채워지므로 A/B 두 스냅샷
        //   불일치(M-A)가 원천 제거된다.
        on_ready(&outcome);

        for chunk in to_send {
            let frame = OutputFrame {
                agent_id: self.id,
                epoch: self.epoch,
                seq: chunk.seq,
                data: &chunk.data,
            };
            let _ = sink.send(frame);
        }
        drop(subscribers_guard);

        outcome
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

/// 늦게 붙는 창을 위한 PTY 출력 ring buffer — 상한 2MB **그리고** event 수 상한.
/// ★event 수 상한 이유(S12 consult, GPT 단독 catch): byte 상한만 있으면 1바이트 청크가
/// 폭주할 때 event 수가 수백만으로 불어, 신규 구독자가 replay를 받을 때 bounded mpsc를
/// 즉시 가득 채워 매 재연결이 slow-consumer로 끊기는 영구 루프가 생긴다. 둘 중 하나라도
/// 초과하면 앞부터 evict. (불변식: max_events ≤ 데몬 WS 송신 큐 cap − control_slack.)
pub struct ReplayBuffer {
    chunks: VecDeque<OutputChunk>,
    total_bytes: usize,
    max_bytes: usize,
    max_events: usize,
}

impl ReplayBuffer {
    pub fn new() -> Self {
        Self {
            chunks: VecDeque::new(),
            total_bytes: 0,
            max_bytes: 2 * 1024 * 1024,
            // 4096: 데몬 WS 송신 큐 cap(예 4608) − control_slack(512) 이하로 잡아
            // replay만으로 신규 구독자 큐가 넘치지 않게 한다.
            max_events: 4096,
        }
    }

    pub fn push(&mut self, chunk: OutputChunk) {
        self.total_bytes += chunk.data.len();
        self.chunks.push_back(chunk);
        // byte 상한 OR event 수 상한 둘 중 하나라도 넘으면 앞부터 제거.
        while self.total_bytes > self.max_bytes || self.chunks.len() > self.max_events {
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

    /// 받은 출력을 (seq, raw data)로 순서대로 수집하는 mock OutputSink.
    /// raw 경계화 검증: base64 아닌 raw 바이트가 그대로 오는지 확인.
    struct MockSink {
        id: SinkId,
        events: Mutex<Vec<(u64, Vec<u8>)>>,
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
            // raw 바이트를 복사 보관(테스트 검증용).
            self.events
                .lock()
                .unwrap()
                .push((frame.seq, frame.data.to_vec()));
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

    use crate::agent::types::AgentInfo;

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
        // ★raw 경계화 검증: sink가 base64 아닌 raw 바이트를 받았는지.
        {
            let ev = sink.events.lock().unwrap();
            assert_eq!(ev[0].1, b"hello");
            assert_eq!(ev[1].1, b"world");
        }
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
    fn replay_buffer_caps_event_count() {
        // 1바이트 청크 폭주 → byte 상한(2MB)엔 한참 못 미치지만 event 수 상한(4096)에 걸려야 함.
        let mut rb = ReplayBuffer::new();
        for seq in 0..5000u64 {
            rb.push(OutputChunk {
                seq,
                data: vec![b'x'],
            });
        }
        let snap = rb.snapshot();
        // 정확히 max_events(4096)로 cap.
        assert_eq!(snap.len(), 4096);
        // 가장 오래된 것부터 evict → 남은 첫 seq = 5000-4096 = 904.
        assert_eq!(snap.first().unwrap().seq, 904);
        assert_eq!(snap.last().unwrap().seq, 4999);
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
    fn subscribe_from_resume_sends_only_tail() {
        let core = new_core(MockStatusSink::new());
        for i in 0..5u8 {
            core.emit(OutputEvent::TerminalBytes(vec![b'a' + i]));
        }
        let sink = MockSink::new();
        let out = core.subscribe_from(sink.clone(), Some(2), true, |_| {});

        // after_seq=2 → seq>2 인 [3,4] 만 전송.
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

        // after_seq=10 < oldest(904) → Truncated, oldest 부터 전체.
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

        // epoch 불일치 → after_seq 무시하고 전체.
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

        // 이후 live emit(seq3) → gap 없이 sink 가 받음(C4: 구독이 lock 보유 중 끝나 역전 없음).
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
        // send 가 처음 불릴 때 replay_started 를 true 로 세우는 sink.
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
            // (1) 콜백 시점엔 아직 어떤 frame 도 sink 로 안 나갔다.
            assert!(
                !started.load(Ordering::SeqCst),
                "on_ready 는 replay 전송 전에 호출돼야 함"
            );
            cc.fetch_add(1, Ordering::SeqCst);
            *seen_cb.lock().unwrap() = Some(*outcome);
        });

        // (2) 정확히 1회 호출.
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
        // replay 가 실제로 전송됐는지(after_seq=1 → seq 2 전송) → started true.
        assert!(replay_started.load(Ordering::SeqCst));
        // (3) 콜백이 본 outcome == 반환 outcome.
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
