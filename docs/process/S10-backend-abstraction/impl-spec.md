# S10 구현 스펙 (impl-spec) — 코더 공통 참조

`agent-transport-design.md`의 개념을 **구체 Rust 시그니처**로 확정한 문서. 모든 단계 코더가 이걸 기준으로 코딩한다. 설계 충돌 시 `agent-transport-design.md`(권위) > 이 문서 > 코더 재량.

오케스트레이터(상위 claude)가 derive. 목표: **회귀 0**. 검증된 S9 PTY 코드를 아래 구조로 재편하되 동작·불변식은 그대로.

## 목표 모듈 레이아웃 (`src-tauri/src/pty/`)

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

기존 `drain.rs`, `claude.rs`는 위로 흡수되며 제거(또는 re-export). `pty/mod.rs`는 새 모듈 선언으로 갱신.

## 소유권 분할 (절대 준수 — fable 저수준 취합 §2)

- **transport(PtyTransport)**: master / writer / child(Arc<Mutex>) / shutdown flag(Arc<AtomicBool>) / job_handle / reader(start 전까지 보관) / cols·rows? → **cols/rows는 AgentSession이 보유**(아래 참조).
- **core(OutputCore)**: subscribers / replay / seq / status / finalized flag / status_sink / drain_handle / drain_done_rx / id / epoch.
- **session(AgentSession)**: id / cwd / epoch / cols / rows(atomic) / core(Arc) / transport(Box).

## 신규 타입 (types.rs)

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

`PtyChunk` → **`OutputChunk`** 개명. **직렬화 필드명은 그대로(`seq`,`data`)** 유지(wire 호환). `get_snapshot` 반환도 `Vec<OutputChunk>`.

`PtyEvent`(프론트 wire, base64)·`OutputSink`(send(PtyEvent))·`StatusSink`·`PtyError`·`SinkError`·`AgentStatus`·`AgentInfo`는 **이름 유지**(프론트 결합 최소화). PtyError 변형 부족 시 `Unsupported(&str)` 추가 허용(ApiTransport용).

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

메서드(불변식: drain.rs/session.rs 현 락 규율 그대로):
- `new(id, epoch, status_sink) -> Self` — status Running, seq 0.
- `emit(&self, event: OutputEvent)` — **variant-agnostic**. `TerminalBytes(bytes)`: seq.fetch_add → OutputChunk push(replay, brief lock) → subscribers clone 스냅샷 후 lock 해제 → lock 밖에서 PtyEvent(base64) fanout → dead sink 짧게 retain. 그 외 variant `_ => {}`. **불변식 1·2(drain.rs) 그대로**.
- `finish(&self, reason: TerminalReason)` — `if finalized.swap(true, AcqRel) { return; }`. reason→AgentStatus 매핑 후 status 기록 + `status_sink.status_changed(id, status, epoch)`. **terminal 알림 주체 = pump(=여기), 현 drain transition 대체.**
- `enter_exiting(&self) -> bool` — finalized면 false. status가 terminal이면 false. 아니면 Exiting 기록 + `status_changed(Exiting)` 후 true. **(manager kill 0.5단계용)**
- `join_pump(&self, timeout: Duration)` — drain_done_rx.take() 후 recv_timeout. (kill 6단계)
- `attach_pump(&self, handle: JoinHandle<()>, done_rx: Receiver<()>)` — transport.start가 호출해 핸들/rx 적재.
- `subscribe(&self, sink) -> SinkId` / `unsubscribe(&self, sink_id)` — 현 session.rs C4 로직 그대로 이식.
- `snapshot(&self) -> Vec<OutputChunk>` — replay 스냅샷.
- `status(&self) -> AgentStatus` — clone 반환.

**reason→AgentStatus 매핑(AgentStatus 변형 추가 금지):**
`Exited{code}`→`Exited{code}` · `Killed`→`Killed` · `Interrupted`→`Killed` · `StreamClosed`→`Exited{code:None}` · `Cancelled`→`Killed` · `Error(s)`→`Failed{message:s}`.

stage 2 단위 테스트: emit seq 증가/replay 누적 · finish 1회(2번 호출해도 1번만 status 변경) · enter_exiting가 terminal 후엔 false.

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

- `open(spec: &CommandSpec, size: (u16,u16)) -> Result<(PtyTransport, Option<u32> /*child_pid*/), PtyError>` — 현 manager.spawn_session의 1~5단계(openpty/spawn/job/clone_reader/take_writer). reader는 PtyTransport.reader에 보관. **pump는 아직 안 띄움.**
- `start(&self, core)` — reader.take() → pump 스레드 spawn(아래) → `core.attach_pump(handle, done_rx)`.
- pump 스레드 = 현 drain_loop + transition 흡수:
  - 캡처: reader(move) · core(Arc) · child(Arc clone) · shutdown(Arc clone) · done_tx.
  - loop: `reader.read` → Ok(0)/Err break, Ok(n) → `core.emit(OutputEvent::TerminalBytes(buf[..n].to_vec()))`. read 후 `if shutdown.load(Relaxed) break`.
  - 탈출 후 reason 산출: child exit code를 status lock 없이 `child.lock().try_wait()`로 취득 → `if shutdown.load(Acquire) { Killed } else { Exited{code} }` → `core.finish(reason)` → `done_tx.send(())`.
- `send_input(Raw(bytes))` — writer.write_all+flush (현 write_stdin).
- `resize(cols,rows)` — master.resize. (cols/rows atomic 저장은 AgentSession이 함)
- `interrupt()` — writer로 `0x03` 주입(현 send_input 경로 재사용).
- `shutdown()` — **멱등**: shutdown.store(true,Release) → child.kill+wait → (win)job.terminate(1) → master.take(). (현 kill 1~5단계, drain 대기 제외).
- `capabilities()` — 콘솔 값: input.raw=true, output.terminal_bytes=true, control{resize:true,interrupt:true,cancel:false,graceful_shutdown:false}, session{resume:true,snapshot:false,cwd_env:true}, 나머지 false.

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
- `kill(&self, timeout)` → `transport.shutdown(); self.core.join_pump(timeout);` **(= 합성, 인과 보존)**.
- `enter_exiting(&self) -> bool` → core.enter_exiting().
- `capabilities(&self)` → transport.capabilities().
- `subscribe`/`unsubscribe`/`snapshot`/`status` → core 위임.
- `core(&self) -> &Arc<OutputCore>` — manager.spawn에서 transport.start에 넘기기 위함(또는 new 내부에서 처리).
- start 호출 위치: AgentSession::new가 `transport.start(core.clone())`를 부르거나, manager가 new 후 부른다 → **manager가 spawn_session에서 명시 호출**(테스트 가시성).

## AgentBackend (backend/) — stage 4

```rust
pub trait AgentBackend: Send + Sync {
    fn needs_session(&self) -> bool;
    fn build_spec(&self, command: &AgentCommand, mode: SpawnMode,
                  session_id: Option<Uuid>, cwd: PathBuf, env: Vec<(String,String)>) -> CommandSpec;
}
```
- `claude.rs ClaudeBackend` — 현 claude.rs build_command 로직. `--session-id`/`--resume`. needs_session=true.
- `shell.rs ShellBackend` — program/args 그대로. needs_session=false.
- `mod.rs` 디스패치:
  ```rust
  pub fn needs_session(c: &AgentCommand) -> bool { backend_for(c).needs_session() }
  pub fn build_command_spec(c, mode, sid, cwd, env) -> CommandSpec { backend_for(c).build_spec(...) }
  fn backend_for(c: &AgentCommand) -> &'static dyn AgentBackend {
      match c { Claude{..} => &ClaudeBackend, Shell{..} => &ShellBackend }
  }
  ```
- 현 claude.rs의 단위 테스트는 backend/claude.rs로 이동(시그니처 맞춰 갱신).
- **codex.rs/gemini.rs(tail)**: CodexBackend/GeminiBackend struct + build_spec(best-guess 플래그 + `// TODO: CLI spike로 플래그 확정` 표식). AgentCommand에 variant 추가 안 함(라우팅 X) — 구조만. 자체 단위 테스트로 build_spec 검증.

## AgentManager (manager.rs) — stage 6

PtyManager→AgentManager 개명. `sessions: RwLock<HashMap<AgentId, Arc<AgentSession>>>`. 나머지 필드 동일(status_sink/profiles/tracker).

`spawn_agent` 흐름 변경:
1. 이중 spawn 가드(동일) · profiles.upsert · cwd canonicalize(동일).
2. `let needs = backend::needs_session(&profile.command); let sid = if needs { profiles.ensure_session_id(id) } else { None };`
3. `let spec = backend::build_command_spec(&profile.command, mode, sid, cwd, profile.env.clone());`
4. `spawn_session(id, spec, epoch)` → `(Arc<AgentSession>, Option<u32>)`:
   - `let (transport, child_pid) = PtyTransport::open(&spec, (DEFAULT_COLS,DEFAULT_ROWS))?;`
   - `let core = Arc::new(OutputCore::new(id, epoch, self.status_sink.clone()));`
   - `let transport: Box<dyn AgentTransport> = Box::new(transport);`
   - `transport.start(core.clone());`
   - `let session = Arc::new(AgentSession::new(id, spec.cwd.clone(), epoch, DEFAULT_COLS, DEFAULT_ROWS, core, transport));`
   - sessions.write().insert. return.
5. tracker.watch(동일) · 로그 · agent_info · agent_list_updated.

`kill_agent`:
- get_session → `session.enter_exiting()` true면 별도 알림 없음(enter_exiting가 이미 status_changed(Exiting) 발행) → `session.kill(Duration::from_secs(5))` → sessions.remove → tracker.unwatch → agent_list_updated.

`remove_session`(fallback용): tracker.unwatch → sessions.remove(Arc 회수) → `session.kill(5s)`(silent, Exiting 알림 없음). pump의 finish(Killed)는 정상 발행되고 join으로 소진(현 C-1 동작 동일).

`early_terminal_status`: session.status()(=core.status()) 사용.
`list_agents`/`agent_info`: cols/rows는 session.cols/rows, status는 session.status().
`get_snapshot` → session.snapshot().
`write_stdin`→`write_input`, `resize`, 신규 `interrupt(id)` 위임.
`shutdown_all` 동일(tracker.stop + scope kill_agent).

**불변식 체크리스트(회귀 금지):**
- kill 자원 순서 child→job→master(PtyTransport.shutdown) + master drop→pump EOF→core.finish→join_pump.
- §10 락: sessions clone 후 해제 → session 내부 접근. status lock 보유 중 외부호출 금지. emit fanout 시 subscribers lock 미보유.
- finalize 1회(core.finalized). epoch 재구독(epoch 보존). best-effort tracker 불변.
- 과도기 Exiting=manager(enter_exiting 트리거), terminal=pump 단독(core.finish).

## stage 7 — ApiTransport (transport/api.rs)
AgentTransport 구현, 전부 `Err(PtyError::Unsupported(...))`/`unimplemented!`/no-op. start=no-op. capabilities 전부 false. **manager 라우팅 추가 안 함**(껍데기 존재만).

## stage 8 — commands/lib/프론트
- commands: `write_stdin`→유지 또는 `send_input`(둘 다 가능, 우선 기존 유지+신규 `interrupt_agent` 추가). `interrupt_agent(agent_id)` 커맨드. lib.rs invoke_handler 등록.
- AgentInfo에 `capabilities: Capabilities` 필드 추가 → agent_info에서 session.capabilities() 실어줌.
- TS 미러(api/types.ts): Capabilities 인터페이스 + AgentInfo.capabilities 추가. (OutputEvent/InputEvent는 내부라 TS 미러 불필요. OutputChunk wire 필드 동일이라 영향 없음.)

## QA 게이트(매 단계 후 오케스트레이터 인라인)
`cargo test --lib` (≥19) + `cargo run --example headless`(Running→Exiting→Killed, remaining=0) + `cargo fmt` + `rg "use tauri" src/pty/`(→0). 통과해야 다음 단계·커밋.
</content>
</invoke>
