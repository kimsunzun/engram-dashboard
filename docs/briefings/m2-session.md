# 모듈 2 — pty/session.rs 브리핑 (담당: dco23, Opus)

발신: ed12 (매니저)
근거: `docs/backend-lld-stage1.md` §4 (PtySession 구조체), §7 (subscribe/replay), §10 (동시성).
**반드시 §4·§7·§10 원문을 읽고 따른다.** 이 모듈이 백엔드에서 가장 동시성 민감하다.

## 목표

`src-tauri/src/pty/session.rs`:
1. `PtySession` 구조체 (§4)
2. `ReplayBuffer` — **types.rs에서 이 파일로 이동** (dr26 리뷰 일탈 1건 해결: LLD §1/§4가 session.rs 소속으로 명시). types.rs에서는 제거하고, types.rs가 ReplayBuffer를 참조하던 부분이 있으면 정리.
3. `impl PtySession` 의 `subscribe` / `unsubscribe` (§7)
4. `PtySession::new(...)` 생성자 (manager의 spawn_agent가 호출할 형태)

drain thread 로직(drain_loop)과 spawn/kill은 **이 파일이 아니다** (drain.rs / manager.rs). 여기선 자료구조 + subscribe/unsubscribe만.

## PtySession 구조체 (§4 그대로)

```rust
pub struct PtySession {
    pub id:   AgentId,
    pub cwd:  PathBuf,
    pub master: Mutex<Option<Box<dyn MasterPty + Send>>>,  // Option 필수: kill시 take()→ConPTY종료→reader EOF (spike 검증)
    pub writer: Mutex<Box<dyn Write + Send>>,
    pub child:  Mutex<Box<dyn Child + Send + Sync>>,
    pub status: Mutex<AgentStatus>,
    pub cols: AtomicU16,
    pub rows: AtomicU16,
    pub subscribers: Mutex<Vec<Arc<dyn OutputSink>>>,
    pub replay: Mutex<ReplayBuffer>,
    pub seq: AtomicU64,
    pub shutdown: AtomicBool,
    pub drain_handle:  Mutex<Option<std::thread::JoinHandle<()>>>,
    pub drain_done_rx: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    #[cfg(windows)]
    pub job_handle: crate::pty::platform::JobObjectHandle,
}
```

**필드별 별도 Mutex인 이유**(§4 line 245)를 구조체 위 doc comment로 설명: drain이 replay/subscribers만 잠그는 동안 write_stdin은 writer만 잠가 교착 없이 병행 가능.

## subscribe (§7 — C4 핵심, 절대 준수)

```
subscribe(&self, sink: Arc<dyn OutputSink>) -> SinkId:
    let sink_id = sink.sink_id();
    // (C4) subscribers lock 보유 중 replay 전송 — replay→live 순서 역전 원천 차단
    let mut subscribers_guard = self.subscribers.lock();
    subscribers_guard.push(sink.clone());               // (A) live 구독 먼저 등록
    let snapshot = self.replay.lock().snapshot();        // subscribers 보유 중 replay 취득 (유일 허용 예외)
    for chunk in snapshot {
        // PtyChunk → PtyEvent(base64) 변환 후 sink.send
        sink.send(event)?;  // 실패해도 일단 등록은 유지/정리 정책은 §7 따름
    }
    drop(subscribers_guard);   // lock 해제 → drain 재개
    sink_id
```

- **락 순서 예외 명시 주석**: "subscribe 함수만 subscribers→replay 두 lock 동시 취득. drain thread는 절대 두 lock 동시 보유 금지" (§10 규칙3 예외).
- seq 연속성: replay snapshot의 seq와 이후 live chunk의 seq가 끊기지 않아야 함. 프론트가 seq로 dedup한다.

## unsubscribe (§7)

```
unsubscribe(&self, sink_id: SinkId):
    self.subscribers.lock().retain(|s| s.sink_id() != sink_id);
```

## 불변 규칙 (리뷰 필수)

- tauri import 0개. (JobObjectHandle은 crate::pty::platform 경유)
- `master` 는 반드시 `Mutex<Option<...>>` — kill path에서 take() 한다.
- 락 보유 중 `sink.send()` 호출하는 곳은 **subscribe의 replay 전송뿐** (C4 의도된 예외). drain은 절대 금지 — 그건 drain.rs 몫.
- Mutex는 std::sync::Mutex 사용 (LLD가 parking_lot 명시 안 하면 std). lock().unwrap() 또는 expect로 poison 처리.
- 모든 unsafe 없음(이 파일엔 unsafe 불필요).

## 코드 품질

- 동시성 결정마다 *왜* 한국어 주석 (특히 C4, 필드별 Mutex, 락 순서 예외).
- **완료 보고 전 `cargo fmt` 필수** → `cargo fmt --check` 통과.
- `cargo build` 통과 (types.rs에서 ReplayBuffer 빠지므로 types.rs도 같이 빌드되는지 확인).

## 보고

`orch 12 "⟁dco23 session.rs 완료 — PtySession+ReplayBuffer이동+subscribe(C4)/unsubscribe, fmt/build OK"`
막히면 30분 내 중간보고. 동시성 판단 애매하면 ed12에 질문.
