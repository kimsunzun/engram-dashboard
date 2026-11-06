# Module 2 — pty/session.rs briefing (owner: dco23, Opus)

From: ed12 (manager)
Source: `docs/backend-lld-stage1.md` §4 (PtySession struct), §7 (subscribe/replay), §10 (concurrency).
**Read the original §4, §7 and §10 and follow them.** This module is the most concurrency-sensitive part of the backend.

## Goal

`src-tauri/src/pty/session.rs`:
1. The `PtySession` struct (§4)
2. `ReplayBuffer` — **move it from types.rs to this file** (resolves one deviation found in the dr26 review: LLD §1/§4 state that it belongs to session.rs). Remove it from types.rs, and clean up anything in types.rs that referenced ReplayBuffer.
3. `subscribe` / `unsubscribe` of `impl PtySession` (§7)
4. The `PtySession::new(...)` constructor (in the form manager's spawn_agent will call)

The drain thread logic (drain_loop) and spawn/kill are **not this file** (drain.rs / manager.rs). Here: only the data structures + subscribe/unsubscribe.

## PtySession struct (§4 as written)

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

**Explain the reason for a separate Mutex per field** (§4 line 245) in a doc comment above the struct: while drain locks only replay/subscribers, write_stdin locks only writer, so they run in parallel without deadlock.

## subscribe (§7 — the core of C4, follow strictly)

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

- **A comment stating the lock-order exception**: "only the subscribe function acquires both locks, subscribers→replay, at the same time. The drain thread must never hold the two locks at the same time" (§10 rule 3 exception).
- seq continuity: the seq of the replay snapshot and the seq of the live chunks after it must not have a gap. The frontend dedups by seq.

## unsubscribe (§7)

```
unsubscribe(&self, sink_id: SinkId):
    self.subscribers.lock().retain(|s| s.sink_id() != sink_id);
```

## Invariants (mandatory for review)

- Zero tauri imports. (JobObjectHandle goes through crate::pty::platform)
- `master` must be `Mutex<Option<...>>` — the kill path does take() on it.
- The **only** place that calls `sink.send()` while holding a lock is subscribe's replay send (the intended C4 exception). Drain must never do this — that is drain.rs's job.
- Use std::sync::Mutex (std unless the LLD names parking_lot). Handle poison with lock().unwrap() or expect.
- No unsafe at all (this file needs no unsafe).

## Code quality

- A Korean comment on *why* for every concurrency decision (especially C4, the per-field Mutex, the lock-order exception).
- **Run `cargo fmt` before the completion report** → `cargo fmt --check` passes.
- `cargo build` passes (ReplayBuffer leaves types.rs, so check that types.rs builds together with it).

## Report

`orch 12 "⟁dco23 session.rs 완료 — PtySession+ReplayBuffer이동+subscribe(C4)/unsubscribe, fmt/build OK"`
If stuck, send an interim report within 30 minutes. If a concurrency call is ambiguous, ask ed12.
