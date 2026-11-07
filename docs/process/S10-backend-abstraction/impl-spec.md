# S10 implementation spec (impl-spec) — shared reference for coders

A document that fixes the concepts of `agent-transport-design.md` as **concrete Rust signatures**. Coders of every stage code against this. On a design conflict: `agent-transport-design.md` (authoritative) > this document > coder discretion.

Derived by the orchestrator (the upper claude). Goal: **zero regressions**. Restructure the verified S9 PTY code into the structure below, keeping behavior and invariants as they are.

## Target module layout (`src-tauri/src/pty/`)

```
types.rs           # 기존 타입 + 신규 중립 타입(OutputEvent/InputEvent/TerminalReason/Capabilities/CommandSpec/OutputChunk)
output_core.rs     # NEW: OutputCore — seq/replay/subscribers/status/finalize + emit/finish/join_pump/subscribe
session.rs         # AgentSession (구체 struct) = OutputCore + Box<dyn AgentTransport> 합성
manager.rs         # AgentManager (PtyManager 개명) — Arc<AgentSession> 보유
transport/
  mod.rs           # trait AgentTransport
  pty.rs           # PtyTransport (master/writer/child/shutdown/job + pump 스레드) ← 현 drain.rs 흡수
  api.rs           # ApiTransport 껍데기 (stage 7)
backend/
  mod.rs           # dispatch (build_command_spec / needs_session) + trait AgentBackend
  claude.rs        # ClaudeBackend (현 claude.rs) — CommandSpec 산출
  shell.rs         # ShellBackend
  codex.rs         # CodexBackend stub (tail)
  gemini.rs        # GeminiBackend stub (tail)
profile.rs         # 거의 불변 (AgentCommand 등)
session_tracker.rs # 불변
platform/windows.rs# 불변
```

The existing `drain.rs` and `claude.rs` are absorbed into the above and removed (or re-exported). `pty/mod.rs` is updated with the new module declarations.

## Ownership split (must follow — fable low-level consolidation §2)

- **transport (PtyTransport)**: master / writer / child (Arc<Mutex>) / shutdown flag (Arc<AtomicBool>) / job_handle / reader (held until start) / cols·rows? → **cols/rows are held by AgentSession** (see below).
- **core (OutputCore)**: subscribers / replay / seq / status / finalized flag / status_sink / drain_handle / drain_done_rx / id / epoch.
- **session (AgentSession)**: id / cwd / epoch / cols / rows (atomic) / core (Arc) / transport (Box).

## New types (types.rs)

```rust
/// pump→core 내부 출력 이벤트. 확장 가능 enum. core는 variant-agnostic(_ => ignore).
#[derive(Debug, Clone)]
pub enum OutputEvent {
    TerminalBytes(Vec<u8>),   // 콘솔 — 지금 유일 variant
    // 후일: TextDelta(String) / MessageDone / Usage{..} / ToolCall{..} / Error(String)
}

/// session→transport 입력 이벤트. 확장 가능 enum.
#[derive(Debug, Clone)]
pub enum InputEvent {
    Raw(Vec<u8>),             // PTY 키 입력 바이트
    // 후일: Message(String) / Reconfigure{..}
}

/// transport가 산출하는 종료 사유(flat). core가 AgentStatus로 매핑(finalize 1회).
/// ※ raw lib error(reqwest/nix) 직접 노출 금지 — 도메인 문자열로.
#[derive(Debug, Clone)]
pub enum TerminalReason {
    Exited { code: Option<i32> },
    Killed,
    Interrupted,
    StreamClosed,
    Cancelled,
    Error(String),
}

/// transport에 주입하는 중립 실행 명세. backend가 산출. PtyTransport는 claude/codex를 모름.
#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: std::path::PathBuf,
}

/// 영역별 capability (bool 폭증 금지). 콘솔 값으로 채움. 직렬화(프론트 공유, snake_case).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Capabilities {
    pub input: InputCaps,
    pub output: OutputCaps,
    pub control: ControlCaps,
    pub session: SessionCaps,
    pub model: ModelCaps,
}
#[derive(Debug, Clone, serde::Serialize)] pub struct InputCaps   { pub raw: bool, pub message: bool, pub attachment: bool }
#[derive(Debug, Clone, serde::Serialize)] pub struct OutputCaps  { pub terminal_bytes: bool, pub markdown: bool, pub tool_events: bool, pub usage: bool }
#[derive(Debug, Clone, serde::Serialize)] pub struct ControlCaps { pub resize: bool, pub interrupt: bool, pub cancel: bool, pub graceful_shutdown: bool }
#[derive(Debug, Clone, serde::Serialize)] pub struct SessionCaps { pub resume: bool, pub snapshot: bool, pub cwd_env: bool }
#[derive(Debug, Clone, serde::Serialize)] pub struct ModelCaps   { pub select: bool, pub temperature: bool, pub max_tokens: bool }
```

`PtyChunk` → renamed **`OutputChunk`**. **Keep the serialized field names as they are (`seq`,`data`)** (wire compatibility). The `get_snapshot` return is also `Vec<OutputChunk>`.

`PtyEvent` (frontend wire, base64) · `OutputSink` (send(PtyEvent)) · `StatusSink` · `PtyError` · `SinkError` · `AgentStatus` · `AgentInfo` **keep their names** (minimize frontend coupling). If PtyError lacks variants, adding `Unsupported(&str)` is allowed (for ApiTransport).

## OutputCore (output_core.rs) — stage 2

```rust
pub struct OutputCore {
    id: AgentId,
    epoch: u32,
    seq: AtomicU64,
    status: Mutex<AgentStatus>,
    finalized: AtomicBool,            // finish 정확히 1회 게이트
    subscribers: Mutex<Vec<Arc<dyn OutputSink>>>,
    replay: Mutex<ReplayBuffer>,      // ReplayBuffer는 여기로 이동
    status_sink: Arc<dyn StatusSink>,
    drain_handle: Mutex<Option<JoinHandle<()>>>,
    drain_done_rx: Mutex<Option<Receiver<()>>>,
}
```

Methods (invariant: keep the current lock discipline of drain.rs/session.rs as is):
- `new(id, epoch, status_sink) -> Self` — status Running, seq 0.
- `emit(&self, event: OutputEvent)` — **variant-agnostic**. `TerminalBytes(bytes)`: seq.fetch_add → push OutputChunk (replay, brief lock) → take a clone snapshot of subscribers and release the lock → fan out PtyEvent (base64) outside the lock → briefly retain to drop dead sinks. Other variants `_ => {}`. **Invariants 1·2 (drain.rs) kept as is**.
- `finish(&self, reason: TerminalReason)` — `if finalized.swap(true, AcqRel) { return; }`. Map reason→AgentStatus, then record status + `status_sink.status_changed(id, status, epoch)`. **The terminal notification emitter = the pump (= here), replacing the current drain transition.**
- `enter_exiting(&self) -> bool` — false if finalized. False if status is already terminal. Otherwise record Exiting + `status_changed(Exiting)` and return true. **(for manager kill step 0.5)**
- `join_pump(&self, timeout: Duration)` — after drain_done_rx.take(), recv_timeout. (kill step 6)
- `attach_pump(&self, handle: JoinHandle<()>, done_rx: Receiver<()>)` — called by transport.start to store the handle/rx.
- `subscribe(&self, sink) -> SinkId` / `unsubscribe(&self, sink_id)` — port the current session.rs C4 logic as is.
- `snapshot(&self) -> Vec<OutputChunk>` — replay snapshot.
- `status(&self) -> AgentStatus` — returns a clone.

**reason→AgentStatus mapping (no adding AgentStatus variants):**
`Exited{code}`→`Exited{code}` · `Killed`→`Killed` · `Interrupted`→`Killed` · `StreamClosed`→`Exited{code:None}` · `Cancelled`→`Killed` · `Error(s)`→`Failed{message:s}`.

Stage 2 unit tests: emit seq increments/replay accumulates · finish once (even if called twice, status changes only once) · enter_exiting is false after terminal.

## trait AgentTransport (transport/mod.rs) — stage 3

```rust
pub trait AgentTransport: Send + Sync {
    /// 출력 pump/stream 기동 → core 연결. spawn 직후 1회 호출. (design "start" 동사)
    /// PtyTransport: 보관해둔 reader를 take해 pump 스레드 spawn, core.attach_pump 호출.
    fn start(&self, core: Arc<OutputCore>);
    fn send_input(&self, input: InputEvent) -> Result<(), PtyError>;
    fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError>;
    /// ≠kill. PTY=0x03 주입 / API=cancel. PTY 구현: send_input(Raw(vec![0x03])).
    fn interrupt(&self) -> Result<(), PtyError>;
    /// 자원 강제 종료(멱등). PtyTransport: shutdown flag set → child.kill+wait → job.terminate → master.take(drop).
    /// 반환 전 master drop 보장(pump read가 EOF로 깸). pump 종료 대기는 여기서 안 함(core.join_pump).
    fn shutdown(&self);
    fn capabilities(&self) -> Capabilities;
    // reconfigure: 단계화 — 지금 trait에 안 넣음(필요 시 default Err). API 때 추가.
}
```

## PtyTransport (transport/pty.rs) — stage 3

```rust
pub struct PtyTransport {
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    shutdown: Arc<AtomicBool>,
    reader: Mutex<Option<Box<dyn Read + Send>>>,   // start()에서 take
    #[cfg(windows)] job_handle: JobObjectHandle,
}
```

- `open(spec: &CommandSpec, size: (u16,u16)) -> Result<(PtyTransport, Option<u32> /*child_pid*/), PtyError>` — steps 1~5 of the current manager.spawn_session (openpty/spawn/job/clone_reader/take_writer). The reader is held in PtyTransport.reader. **The pump is not started yet.**
- `start(&self, core)` — reader.take() → spawn the pump thread (below) → `core.attach_pump(handle, done_rx)`.
- pump thread = the current drain_loop + transition absorbed:
  - Captures: reader (move) · core (Arc) · child (Arc clone) · shutdown (Arc clone) · done_tx.
  - loop: `reader.read` → Ok(0)/Err break, Ok(n) → `core.emit(OutputEvent::TerminalBytes(buf[..n].to_vec()))`. After a read, `if shutdown.load(Relaxed) break`.
  - After exiting, produce the reason: get the child exit code via `child.lock().try_wait()` without the status lock → `if shutdown.load(Acquire) { Killed } else { Exited{code} }` → `core.finish(reason)` → `done_tx.send(())`.
- `send_input(Raw(bytes))` — writer.write_all+flush (the current write_stdin).
- `resize(cols,rows)` — master.resize. (Storing cols/rows atomically is done by AgentSession)
- `interrupt()` — inject `0x03` via the writer (reuse the current send_input path).
- `shutdown()` — **idempotent**: shutdown.store(true,Release) → child.kill+wait → (win) job.terminate(1) → master.take(). (The current kill steps 1~5, excluding the drain wait).
- `capabilities()` — console values: input.raw=true, output.terminal_bytes=true, control{resize:true,interrupt:true,cancel:false,graceful_shutdown:false}, session{resume:true,snapshot:false,cwd_env:true}, the rest false.

## AgentSession (session.rs) — stage 5

```rust
pub struct AgentSession {
    pub id: AgentId,
    pub cwd: PathBuf,
    pub epoch: u32,
    pub cols: AtomicU16,
    pub rows: AtomicU16,
    core: Arc<OutputCore>,
    transport: Box<dyn AgentTransport>,
}
```
- `new(id, cwd, epoch, cols, rows, core: Arc<OutputCore>, transport: Box<dyn AgentTransport>)`.
- `write_input(&self, bytes: &[u8])` → `transport.send_input(InputEvent::Raw(bytes.to_vec()))`.
- `resize(&self, cols, rows)` → `transport.resize(cols,rows)?; self.cols.store; self.rows.store;`.
- `interrupt(&self)` → `transport.interrupt()`.
- `kill(&self, timeout)` → `transport.shutdown(); self.core.join_pump(timeout);` **(= composition, causality preserved)**.
- `enter_exiting(&self) -> bool` → core.enter_exiting().
- `capabilities(&self)` → transport.capabilities().
- `subscribe`/`unsubscribe`/`snapshot`/`status` → delegate to core.
- `core(&self) -> &Arc<OutputCore>` — so that manager.spawn can pass it to transport.start (or handle it inside new).
- Where start is called: AgentSession::new calls `transport.start(core.clone())`, or the manager calls it after new → **the manager calls it explicitly in spawn_session** (test visibility).

## AgentBackend (backend/) — stage 4

```rust
pub trait AgentBackend: Send + Sync {
    fn needs_session(&self) -> bool;
    fn build_spec(&self, command: &AgentCommand, mode: SpawnMode,
                  session_id: Option<Uuid>, cwd: PathBuf, env: Vec<(String,String)>) -> CommandSpec;
}
```
- `claude.rs ClaudeBackend` — the current claude.rs build_command logic. `--session-id`/`--resume`. needs_session=true.
- `shell.rs ShellBackend` — program/args as is. needs_session=false.
- `mod.rs` dispatch:
  ```rust
  pub fn needs_session(c: &AgentCommand) -> bool { backend_for(c).needs_session() }
  pub fn build_command_spec(c, mode, sid, cwd, env) -> CommandSpec { backend_for(c).build_spec(...) }
  fn backend_for(c: &AgentCommand) -> &'static dyn AgentBackend {
      match c { Claude{..} => &ClaudeBackend, Shell{..} => &ShellBackend }
  }
  ```
- The unit tests of the current claude.rs move to backend/claude.rs (updated to match the signatures).
- **codex.rs/gemini.rs (tail)**: CodexBackend/GeminiBackend structs + build_spec (best-guess flags + a `// TODO: CLI spike로 플래그 확정` marker). Do not add variants to AgentCommand (no routing) — structure only. Verify build_spec with their own unit tests.

## AgentManager (manager.rs) — stage 6

PtyManager renamed to AgentManager. `sessions: RwLock<HashMap<AgentId, Arc<AgentSession>>>`. The other fields are the same (status_sink/profiles/tracker).

`spawn_agent` flow changes:
1. Double-spawn guard (same) · profiles.upsert · cwd canonicalize (same).
2. `let needs = backend::needs_session(&profile.command); let sid = if needs { profiles.ensure_session_id(id) } else { None };`
3. `let spec = backend::build_command_spec(&profile.command, mode, sid, cwd, profile.env.clone());`
4. `spawn_session(id, spec, epoch)` → `(Arc<AgentSession>, Option<u32>)`:
   - `let (transport, child_pid) = PtyTransport::open(&spec, (DEFAULT_COLS,DEFAULT_ROWS))?;`
   - `let core = Arc::new(OutputCore::new(id, epoch, self.status_sink.clone()));`
   - `let transport: Box<dyn AgentTransport> = Box::new(transport);`
   - `transport.start(core.clone());`
   - `let session = Arc::new(AgentSession::new(id, spec.cwd.clone(), epoch, DEFAULT_COLS, DEFAULT_ROWS, core, transport));`
   - sessions.write().insert. return.
5. tracker.watch (same) · log · agent_info · agent_list_updated.

`kill_agent`:
- get_session → if `session.enter_exiting()` is true, no separate notification (enter_exiting already emitted status_changed(Exiting)) → `session.kill(Duration::from_secs(5))` → sessions.remove → tracker.unwatch → agent_list_updated.

`remove_session` (for fallback): tracker.unwatch → sessions.remove (reclaim the Arc) → `session.kill(5s)` (silent, no Exiting notification). The pump's finish(Killed) is emitted normally and drained by the join (same as the current C-1 behavior).

`early_terminal_status`: uses session.status() (=core.status()).
`list_agents`/`agent_info`: cols/rows from session.cols/rows, status from session.status().
`get_snapshot` → session.snapshot().
`write_stdin`→`write_input`, `resize`, and the new `interrupt(id)` delegate.
`shutdown_all` same (tracker.stop + scope kill_agent).

**Invariant checklist (no regression):**
- kill resource order child→job→master (PtyTransport.shutdown) + master drop→pump EOF→core.finish→join_pump.
- §10 locks: release after sessions clone → then access session internals. No external calls while holding the status lock. No subscribers lock held during emit fanout.
- Finalize once (core.finalized). epoch resubscribe (epoch preserved). best-effort tracker unchanged.
- Transitional Exiting = manager (enter_exiting trigger), terminal = pump alone (core.finish).

## stage 7 — ApiTransport (transport/api.rs)
Implements AgentTransport, all `Err(PtyError::Unsupported(...))`/`unimplemented!`/no-op. start=no-op. capabilities all false. **Do not add manager routing** (the shell merely exists).

## stage 8 — commands/lib/frontend
- commands: `write_stdin`→keep or `send_input` (both possible; first keep the existing one + add a new `interrupt_agent`). The `interrupt_agent(agent_id)` command. Register in the lib.rs invoke_handler.
- Add a `capabilities: Capabilities` field to AgentInfo → agent_info carries session.capabilities().
- TS mirror (api/types.ts): add the Capabilities interface + AgentInfo.capabilities. (OutputEvent/InputEvent are internal so no TS mirror is needed. OutputChunk wire fields are the same so no impact.)

## QA gate (inline by the orchestrator after every stage)
`cargo test --lib` (≥19) + `cargo run --example headless` (Running→Exiting→Killed, remaining=0) + `cargo fmt` + `rg "use tauri" src/pty/` (→0). It must pass before the next stage and commit.
</content>
</invoke>
