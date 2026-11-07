# Backend Stage 2 Coding Briefing

**Audience:** coder agent (new session)  
**Written by:** ed12, 2026-06-11

---

## Files to read (in this order)

1. `docs/backend-lld-stage1.md` — **the design contract. This is the reference.**
2. `CLAUDE.md` — current project state
3. This file

---

## What you do

Implement the Rust backend under `src-tauri/`, module by module.  
Turn the LLD's signatures, types and pseudocode into real code. **Do not deviate from the LLD.**

---

## Implementation order (follow this order)

### 1. `src-tauri/Cargo.toml` setup
Exactly the LLD §2 dependencies. Never change a version on your own.

### 2. `src-tauri/src/pty/types.rs`
All of LLD §3. `AgentId`, `AgentStatus`, `PtyChunk`, `PtyEvent`, `AgentInfo`, `PtyError`, the `OutputSink` trait, the `StatusSink` trait.

### 3. `src-tauri/src/pty/session.rs`
LLD §4. `PtySession`, `ReplayBuffer`. Follow the per-field Mutex structure exactly.

### 4. `src-tauri/src/pty/drain.rs`
LLD §6. Drain thread + immediate send + exit sequence.  
**Note:** include the `completion channel` (`std::sync::mpsc`) drain_done_tx/rx.

### 5. `src-tauri/src/pty/platform/windows.rs`
LLD §13. The `JobObjectHandle` wrapper + `create_and_assign`.

### 6. `src-tauri/src/pty/manager.rs`
LLD §5. `PtyManager` — `sessions: Arc<RwLock<...>>`, `status_sink: Arc<dyn StatusSink>`.  
`spawn_agent`, `subscribe`, `unsubscribe`, `write_stdin`, `resize`, `kill_agent`, `list_agents`, `get_snapshot`, `shutdown_all`.

### 7. `src-tauri/src/logging/mod.rs`
LLD §14. `init_logging`, `set_log_level`. env `ENGRAM_LOG`.

### 8. `src-tauri/src/commands/agent.rs` + `pty.rs`
LLD §8. Thin wrappers only. No business logic.  
`subscribe_agent_output` must return a `SinkId`.  
Add `unsubscribe_agent_output`.

### 9. `src-tauri/src/lib.rs`
`AppState { manager: Arc<PtyManager> }`.  
The Tauri Channel-based `OutputSink` impl goes here (commands layer).  
The Tauri AppHandle-based `StatusSink` impl goes here.

---

## Code-writing rules

- **Many comments.** Especially on concurrency (why this order, why this lock).
- Above each function, mark the LLD reference: `// LLD §6 drain loop`
- Never violate the lock acquisition order rule (see LLD §10).
- Never put `use tauri::` in files under `pty/`.
- Isolate `#[cfg(windows)]` branches in the platform module.

---

## Verification points (check after each module is done)

| Module | Verification |
|---|---|
| types.rs | `cargo check` passes |
| session.rs | `cargo check` passes |
| drain.rs | headless test: spawn → receive output → kill → join completes immediately |
| manager.rs | headless test: PtyManager::new(MockStatusSink) can be constructed directly |
| commands/ | `cargo build` passes |
| Whole | `npm run tauri dev` → confirm Claude Code actually runs |

---

## First spike of stage 2 (before writing code)

As a temporary file `src-tauri/src/pty/spike.rs`:
```rust
// Windows 실측 검증
// 1. portable-pty spawn → stdout 수신 → child.kill() → master.take() → join 즉시 완료?
// 2. 완료 시간 측정 (5초 이내여야 함)
```
If it passes, proceed to the real implementation. If it fails, report to ed12 (manager).

---

## After completion

Each time you finish a module, report `orch 12 "⟁ed12 [모듈명] 완료"`.  
After everything is done, confirm the real behavior with `npm run tauri dev`.
