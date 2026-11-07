# Module 3 — pty/drain.rs briefing (owner: dco23, Opus)

From: ed12 (manager)
Source: `docs/backend-lld-stage1.md` §6 (drain thread), §9 (state machine), §10 (concurrency).
**Implement the §6 pseudocode as written.** A violation of this module's invariants leads directly to deadlock or loss.

## Goal

`src-tauri/src/pty/drain.rs`:
1. `spawn_drain_thread(...)` — create an OS thread (std::thread, not tokio)
2. `drain_loop(...)` — the read→send loop
3. State transition + completion signal when drain ends

## Signature (argument addendum — the LLD §6 pseudocode shows only the reader, but this is what is actually needed)

```rust
pub fn spawn_drain_thread(
    session: Arc<PtySession>,
    reader: Box<dyn Read + Send>,        // master.try_clone_reader() 결과
    status_sink: Arc<dyn StatusSink>,    // 종료 시 status_changed 호출용 (PtySession엔 없음, manager가 보유)
    done_tx: std::sync::mpsc::Sender<()>,// G-1 완료 신호 (session.drain_done_rx와 짝)
) -> std::thread::JoinHandle<()>;
```

> This fits the structure where session.new() leaves drain_handle/done_rx as None and manager injects them afterwards.
> Manager's spawn_agent: (tx,rx)=channel() → inject rx into session.drain_done_rx → spawn_drain_thread(session, reader, status_sink, tx) → inject the returned handle into session.drain_handle.
> This wiring is done in manager.rs (module 5). drain.rs provides only the signature above.

## drain_loop (§6 pseudocode — as written)

```
buf = [0u8; 4096]
loop {
    // 1. blocking read (read 자체가 자연 배칭)
    n = match reader.read(&mut buf) {
        Ok(0) | Err(_) => break,        // EOF(master drop) or Err → 종료
        Ok(n) => n,
    };
    // 2. shutdown 보조 확인 (EOF로 먼저 깨지는 게 보통)
    if session.shutdown.load(Relaxed) { break; }
    // 3. seq 발급 + 즉시 send (C2: partial batch 정체 없음)
    let seq = session.seq.fetch_add(1, Relaxed);
    let data = buf[..n].to_vec();
    let event = PtyEvent { agent_id: session.id, seq, data_b64: base64(&data) };
    // 4. replay 저장 (brief lock — 즉시 해제)
    session.replay.lock().push(PtyChunk { seq, data });
    // 5. subscriber 스냅샷 후 lock 밖 send  ★불변식★
    let sinks = session.subscribers.lock().clone();   // clone 후 즉시 lock 해제
    let mut dead = vec![];
    for sink in sinks {
        if sink.send(event.clone()).is_err() { dead.push(sink.sink_id()); }
    }
    // 6. 죽은 구독자 제거
    if !dead.is_empty() {
        session.subscribers.lock().retain(|s| !dead.contains(&s.sink_id()));
    }
}
// 루프 탈출 → 상태 전이(단일 함수, M5 race 방지) + 알림
// child.try_wait()로 exit code 판별, shutdown flag면 Killed, 아니면 Exited{code}
transition(&session, ...);
status_sink.status_changed(session.id, <new status>);
// G-1: 완료 신호 (kill_agent의 recv_timeout(5s)가 수신)
let _ = done_tx.send(());
```

## ★Core invariants (mandatory for review)★

1. **Hold no lock when calling `sink.send()`.** Snapshot subscribers with `clone()`, release the lock, then iterate the copy and send. (§10 rule 3)
2. **Never hold `replay.lock()` and `subscribers.lock()` at the same time** — take each briefly and release immediately. (Only the subscribe function holds both locks; drain never does.)
3. The exit path runs only once — whether the loop broke on EOF, Err or shutdown, the transition goes through a single function and is idempotent.

## State transition (transition)

See the §9 state machine. If the shutdown flag is set, `Killed`; otherwise `Exited{code}` from the child exit code.
Where the transition function lives is dco23's call (a session impl or a helper inside drain). But status is a Mutex<AgentStatus>, so lock it and then transition.

## Rules and quality

- Zero tauri imports, zero unsafe. Use std::thread::spawn.
- base64 uses the base64 crate (Engine::encode). `use base64::Engine`.
- A Korean comment on *why* for each concurrency invariant. Especially the reason for "send outside the lock".
- **Run cargo fmt before finishing** → cargo fmt --check passes. cargo build passes.

## Report

`orch 12 "⟁dco23 drain.rs 완료 — drain_loop(lock밖send)+종료전이+done_tx, fmt/build OK"`
Note: session.rs is under dr26 review. If a blocker that changes the session structure comes up, ed12 will tell you — in that case part of drain may be adjusted. If stuck, send an interim report within 30 minutes.
