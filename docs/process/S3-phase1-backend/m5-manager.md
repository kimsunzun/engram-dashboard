# Module 5 — pty/manager.rs briefing (owner: dco23, Opus)

From: ed12 (manager)
Source: `docs/backend-lld-stage1.md` §5 (PtyManager), §6 (kill sequence), §10 (lock order), the resource ownership table.
**The last module of Phase 1. It is the coupling point of all modules.** It ties session/drain/windows/types together.

## Struct (§5)

```rust
pub struct PtyManager {
    sessions: Arc<RwLock<HashMap<AgentId, Arc<PtySession>>>>,
    status_sink: Arc<dyn StatusSink>,   // C1: Tauri AppHandle 아님. trait 주입(테스트 시 Noop)
}
```

## Methods (§5 signatures as written)

new / spawn_agent / subscribe / unsubscribe / write_stdin / resize / kill_agent / list_agents / get_snapshot / shutdown_all

### spawn_agent(&self, cwd: &Path) -> Result<AgentInfo, PtyError>

```
1. native_pty_system().openpty(PtySize{rows,cols,..})   // 기본 24x80
2. CommandBuilder("claude" 또는 설정된 셸), cwd 지정 → pair.slave.spawn_command() → child
   (지금은 검증용으로 cmd.exe/pwsh 등 받아도 됨 — 인자화는 추후. LLD 기본 동작 우선)
3. #[cfg(windows)] JobObjectHandle::new() → assign(child.process_id())   // spike/windows.rs 그대로
4. reader = pair.master.try_clone_reader()   // ★ master를 session에 넣기 전에 reader 확보 ★
5. PtySession::new(PtySessionInit{ id, cwd, master: pair.master, writer: pair.master.take_writer(), child, cols, rows, job_handle, ... })
   → Arc<PtySession>
6. (done_tx, done_rx) = std::sync::mpsc::channel()
   session.drain_done_rx.lock() = Some(done_rx)
7. handle = spawn_drain_thread(session.clone(), reader, self.status_sink.clone(), done_tx)
   session.drain_handle.lock() = Some(handle)
8. sessions.write().insert(id, session)
9. self.status_sink.agent_list_updated(self.list_agents())   // 목록 갱신 알림
10. return AgentInfo
```

### kill_agent(&self, agent_id) -> Result<(), PtyError>  ★§6 six steps as written★

```
0. sessions.read() 로 Arc<PtySession> clone, read lock 즉시 해제   (§10 규칙1)
1. session.shutdown.store(true, Release)
2. session.child.lock().kill()
3. session.child.lock().wait()                       // reap, 좀비 방지
4. #[cfg(windows)] session.job_handle.terminate(1)   // 손자까지 전멸 → ConPTY slave 해제
5. session.master.lock().take()                      // C3: master drop → ClosePseudoConsole → reader EOF
6. session.drain_done_rx.lock().take()
       .and_then(|rx| rx.recv_timeout(Duration::from_secs(5)).ok())
   // timeout이면 drain_handle detach (drop) — Arc 참조 끊기면 자연 정리, leak 아님
7. sessions.write().remove(&agent_id)
8. self.status_sink.agent_list_updated(self.list_agents())
```

> **Division of responsibility for status notifications (no duplicate calls):**
> - Individual state transitions (`status_changed`, Killed/Exited) are called **by the drain thread alone** (in transition). kill_agent does not call it.
> - List changes (`agent_list_updated`) are called **by the manager** at the end of spawn/kill.

### The rest

- **subscribe/unsubscribe**: look up the session with sessions.read() and delegate to `session.subscribe(sink)` / `session.unsubscribe(sink_id)`. (The C4 logic is already in session.rs)
- **write_stdin**: look up the session and `session.writer.lock().write_all(data)`.
- **resize**: look up the session, resize on `session.master.lock()` + update the `session.cols/rows` atomics.
- **list_agents**: iterate sessions.read() → Vec<AgentInfo> (a snapshot of each session's status/cols/rows).
- **get_snapshot**: session.replay.lock().snapshot().
- **shutdown_all**: call kill_agent on every agent_id (or an equivalent cleanup). The app exit path.

## Invariants (mandatory for review)

- **Lock order §10 rule 1**: the `sessions` RwLock is for lookup only — clone the Arc and **release immediately**, then acquire the session's inner locks. Never acquire a session inner lock while holding the sessions lock.
- Never change the order of kill_agent's six steps (the order verified by the spike). In particular master.take() comes at 5, after 4 (Job terminate).
- Zero tauri imports. (status_sink is a trait, not AppHandle)
- Errors: a nonexistent agent_id → PtyError. Lock poison → expect (fail-fast, D-3 policy).

## Code quality

- A Korean comment on *why* at each step of spawn_agent/kill_agent (especially when the reader is obtained, where master.take is, the split of notifications).
- **Run cargo fmt before finishing** → fmt --check passes. cargo build passes (most dead_code warnings should be gone now).

## Report

`orch 12 "⟁dco23 manager.rs 완료 — spawn/kill(6단계)/subscribe위임/resize/shutdown_all, fmt/build OK, dead_code 경고 N건"`
If the PtySessionInit fields of PtySession::new do not match the session.rs definition, adjust them to match (same author, so keep it consistent). If stuck, send an interim report within 30 minutes.
