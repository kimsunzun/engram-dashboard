# Engram Dashboard — Backend LLD Stage 1 (structure finalized)

**Author:** engram-dashboard (ed12), 2026-06-11  
**Basis:** `backend-architecture-final.md` + GPT/Gemini review feedback  
**Stage:** Stage 1 — structure finalized without code. Signature/table/pseudocode level.  
**Next:** three-way verification (fable ✅ / Gemini in progress / GPT in progress) → finalize → Stage 2 module-by-module code

---

## fable review feedback (2026-06-11) — 4 Critical + Major fixes

| Item | Decision |
|---|---|
| C1 pty/ isolation contradiction | Add `StatusSink` trait, remove `PtyManager.app_handle` |
| C2 batching: partial batch stall | Adopt **immediate send (option b)** — send what each read returned right away, delegate batching to the xterm.js side |
| C3 ConPTY kill→EOF not guaranteed | Change kill order: `child.kill() → child.wait() → master.take()` → EOF → join |
| C4 replay→live order inversion | Adopt **send replay while holding the subscribers lock (option a)** |
| M1 external Mutex on AppState | Change to `Arc<PtyManager>` (internal synchronization is enough) |
| M2 no unsubscribe | `subscribe` returns `SinkId`, add `unsubscribe_agent_output` command |
| M3 Cargo.toml errors | Fix tauri-build placement, add thiserror/uuid serde |
| M4 PtyEvent wire format | **JSON + base64 string** as a middle ground, switch to raw IPC after measurement |
| M5 status transition race | Single `transition()` function, clarify Failed/Exited criteria |
| M6 JoinHandle location | Add `PtySession.drain_handle` field |
| Minor (certain) | Arc\<dyn OutputSink\>, JobObjectHandle wrapper, fix windows_subsystem placement |
| Gemini new | Add TerminateJobObject to the kill sequence (ensures grandchild processes are covered) |
| GPT G-1 | JoinHandle timeout → completion channel (drain_done_rx) |
| GPT G-2 | Pin tauri 2.5→2.4 (Channel silent failure issue) |
| GPT G-3 | Confirmed that immediate send removes the final batch flush problem at the root |

---

## 0. Non-goals

Outside the scope of this LLD:

- Tauri frontend integration code (TerminalSlot.tsx etc.) — Stage 2 and later
- Auto-start/restart policy for the Claude Code process — decide later
- PTY session persistence (restoring sessions after an app restart) — decide later
- Authentication/API key management — decide later
- Linux support — current target is win32 only

---

## 1. Module map + dependency direction

```
src-tauri/src/
├── lib.rs                        # Tauri 앱 빌더, AppState 등록, command 등록
├── commands/
│   ├── mod.rs
│   ├── agent.rs                  # spawn_agent, kill_agent, get_agents
│   └── pty.rs                    # subscribe_agent_output, write_stdin, resize_pty, get_agent_snapshot
├── pty/
│   ├── mod.rs                    # pub re-export
│   ├── types.rs                  # AgentId, AgentStatus, PtyEvent, PtyChunk, AgentInfo, PtyError, OutputSink
│   ├── manager.rs                # PtyManager (no Tauri import)
│   ├── session.rs                # PtySession, ReplayBuffer (no Tauri import)
│   ├── drain.rs                  # drain thread 로직 (no Tauri import)
│   └── platform/
│       ├── mod.rs
│       └── windows.rs            # Job Object, ConPTY 우회 (#[cfg(windows)])
└── logging/
    ├── mod.rs                    # LogConfig, init_logging, set_log_level
    └── masking.rs                # API 키 마스킹 tracing layer
```

**Dependency direction (one-way, reverse imports forbidden):**

```
lib.rs
  └→ commands/          (Tauri 의존)
       └→ pty/           (Tauri 의존 없음 — 핵심 격리)
            └→ logging/
  └→ logging/

  pty/ ──────────────────────────── NO Tauri import
  commands/ ─────────────────────── Tauri + pty 연결층 (thin wrapper only)
```

**Core isolation principle:**  
No file under `pty/` may import the `tauri` crate.  
Abstract through the `OutputSink` trait → a mock sink in tests, a Channel sink in production.

**[Module verification point]** Enforce with a CI lint (grep) that `pty/` has no `tauri::` import.

---

## 2. Pinned dependency versions (Cargo.toml)

```toml
[dependencies]
tauri              = { version = "2.4",  features = ["protocol-asset"] }
# G-2: Tauri 2.5.0 Channel silent failure 이슈 (GitHub #13721, #13266)
# 2.4.x로 고정. 2.5.x 업그레이드 시 Windows Channel delivery 반드시 실측 검증
portable-pty       = { version = "0.8.1" }   # 최신 안정 버전 + smoke test 후 업그레이드 검토
uuid               = { version = "1.8",  features = ["v4", "serde"] }   # serde feature 필수
thiserror          = { version = "1.0" }                                 # M3: 누락 추가
tracing            = { version = "0.1" }
tracing-subscriber = { version = "0.3", features = ["env-filter", "reload"] }
serde              = { version = "1",    features = ["derive"] }
serde_json         = { version = "1" }
tokio              = { version = "1",    features = ["rt-multi-thread"] }
base64             = { version = "0.22" }   # M4: PtyEvent wire format (JSON+base64)

[target.'cfg(windows)'.dependencies]
windows            = { version = "0.58", features = ["Win32_System_JobObjects",
                                                      "Win32_Foundation",
                                                      "Win32_System_Threading"] }

[build-dependencies]                    # M3: tauri-build는 build-dependencies 섹션으로
tauri-build        = { version = "2.0" }

[dev-dependencies]
tokio-test = "0.4"
```

**Note:** `portable-pty 0.8.1` — the source of the "0.9.x Windows garbage issue" is unconfirmed (fable M3 finding).  
If the smoke test passes, consider upgrading to the latest version. The baseline is `tests/smoke_windows.rs`.

---

## 3. Core type definitions (`pty/types.rs`)

```rust
pub type AgentId = uuid::Uuid;

// C1(frontend): internally-tagged → wire: {"type":"Running"}, {"type":"Exited","code":0}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type")]
pub enum AgentStatus {
    // Starting 제거됨 (§9 기준 — spawn_command 성공 시 즉시 Running)
    Running,
    Exiting,
    Exited   { code: Option<i32> },
    Failed   { message: String },
    Killed,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PtyChunk {
    pub seq:  u64,
    pub data: Vec<u8>,       // 바이너리 그대로 (String 변환 없음 — UTF-8 쪼개짐 방지)
}

// 구버전 PtyEvent(chunk: PtyChunk) 삭제 — 아래 M4 버전으로 통일됨

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentInfo {
    pub id:     AgentId,
    pub cwd:    String,
    pub status: AgentStatus,
    pub cols:   u16,
    pub rows:   u16,
}

#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("agent not found: {0}")]   NotFound(AgentId),
    #[error("spawn failed: {0}")]      SpawnFailed(String),
    #[error("write failed: {0}")]      WriteFailed(String),
    #[error("cwd outside workspace")]  CwdDenied,
    #[error("io error: {0}")]          Io(#[from] std::io::Error),
}

/// OutputSink trait — Tauri 의존 없이 출력 전달 추상화
/// 프로덕션: tauri::ipc::Channel<PtyEvent> impl
/// 테스트:   std::sync::mpsc::Sender<PtyEvent> impl
pub trait OutputSink: Send + Sync + 'static {
    fn send(&self, event: PtyEvent) -> Result<(), SinkError>;
    fn sink_id(&self) -> SinkId;
}

/// C1 수정: StatusSink trait — pty/가 Tauri AppHandle 없이 상태 변경 알림 전달
pub trait StatusSink: Send + Sync + 'static {
    fn status_changed(&self, id: AgentId, status: AgentStatus);
    fn agent_list_updated(&self, agents: Vec<AgentInfo>);
}

pub type SinkId = uuid::Uuid;

#[derive(Debug)]
pub struct SinkError;  // send 실패 → 구독자 제거 트리거

/// M4: PtyEvent wire format — JSON + base64 (중간 절충)
/// data: Vec<u8>를 base64 인코딩하여 JSON string으로 전달
/// → 숫자 배열([27,91,...]) 대비 크기 유리, 프론트에서 atob() 또는 Uint8Array 변환
/// TODO: 실측 후 raw IPC(tauri InvokeResponseBody::Raw) 전환 검토
#[derive(Debug, Clone, serde::Serialize)]
pub struct PtyEvent {
    pub agent_id: AgentId,
    pub seq:      u64,
    pub data_b64: String,    // base64(Vec<u8>)
}
```

---

## 4. PtySession struct (`pty/session.rs`)

```rust
pub struct PtySession {
    // ── 불변 (생성 후 변경 없음) ──────────────────────────────
    pub id:       AgentId,
    pub cwd:      PathBuf,

    // ── PTY I/O (각각 독립 lock) ──────────────────────────────
    pub master:   Mutex<Option<Box<dyn MasterPty + Send>>>, // C3: Option — kill 시 take()로 drop → EOF
    pub writer:   Mutex<Box<dyn Write + Send>>,             // stdin write 전용
    pub child:    Mutex<Box<dyn portable_pty::Child + Send + Sync>>, // kill/wait

    // ── 상태 (독립 lock) ──────────────────────────────────────
    pub status:   Mutex<AgentStatus>,
    pub cols:     AtomicU16,
    pub rows:     AtomicU16,

    // ── 출력 구독 (독립 lock) ─────────────────────────────────
    pub subscribers: Mutex<Vec<Arc<dyn OutputSink>>>,    // Minor: Box→Arc (clone 가능)

    // ── Replay buffer (독립 lock) ─────────────────────────────
    pub replay:   Mutex<ReplayBuffer>,

    // ── drain thread 제어 ─────────────────────────────────────
    pub seq:       AtomicU64,
    pub shutdown:  AtomicBool,
    pub drain_handle: Mutex<Option<JoinHandle<()>>>,    // M6: JoinHandle 보관
    pub drain_done_rx: Mutex<Option<std::sync::mpsc::Receiver<()>>>, // G-1: timeout 완료 신호

    // ── Windows 전용 ──────────────────────────────────────────
    #[cfg(windows)]
    pub job_handle: JobObjectHandle,                      // Minor: wrapper 타입 (Drop 구현)
}

pub struct ReplayBuffer {
    chunks:   VecDeque<PtyChunk>,
    total_bytes: usize,
    max_bytes:   usize,    // 기본 2MB
}

impl ReplayBuffer {
    pub fn push(&mut self, chunk: PtyChunk) { ... }        // 초과 시 앞부터 제거
    pub fn snapshot(&self) -> Vec<PtyChunk> { ... }        // 전체 복사
}
```

**Why PtySession internals are split into a separate Mutex per field:**  
While the drain thread locks only replay/subscribers, write_stdin can lock only the writer, so they run in parallel without deadlock.  
With a single Mutex for the whole session, stdin would be blocked during drain and drain would be blocked during stdin.

---

## 5. PtyManager (`pty/manager.rs`)

```rust
pub struct PtyManager {
    sessions:     Arc<RwLock<HashMap<AgentId, Arc<PtySession>>>>,
    status_sink:  Arc<dyn StatusSink>,  // C1: AppHandle 대신 StatusSink trait 주입
    // commands 층에서 AppHandle 기반 impl 주입. 테스트 시 NoopStatusSink 주입.
}

impl PtyManager {
    pub fn new(status_sink: Arc<dyn StatusSink>) -> Self;

    /// PTY spawn + drain thread 시작
    pub fn spawn_agent(&self, cwd: &Path) -> Result<AgentInfo, PtyError>;

    /// 구독자 등록 + replay 전송 (순서 보장) → SinkId 반환 (unsubscribe용)
    pub fn subscribe(&self, agent_id: AgentId, sink: Arc<dyn OutputSink>)
        -> Result<SinkId, PtyError>;

    /// 명시적 구독 해제 (창 닫힘 시 effect cleanup에서 호출)
    pub fn unsubscribe(&self, agent_id: AgentId, sink_id: SinkId)
        -> Result<(), PtyError>;

    /// PTY stdin write
    pub fn write_stdin(&self, agent_id: AgentId, data: &[u8])
        -> Result<(), PtyError>;

    /// PTY cols/rows 변경
    pub fn resize(&self, agent_id: AgentId, cols: u16, rows: u16)
        -> Result<(), PtyError>;

    /// 에이전트 종료 (drain thread 정리 포함)
    pub fn kill_agent(&self, agent_id: AgentId)
        -> Result<(), PtyError>;

    /// 전체 목록
    pub fn list_agents(&self) -> Vec<AgentInfo>;

    /// replay 스냅샷 조회
    pub fn get_snapshot(&self, agent_id: AgentId)
        -> Result<Vec<PtyChunk>, PtyError>;

    /// 앱 종료 시 전체 정리
    pub fn shutdown_all(&self);
}
```

**RwLock vs Mutex:**  
In the sessions map, reads (lookups) are far more frequent than writes (add/remove) → `RwLock`.  
Read locks can be shared; only the write lock is exclusive. This minimizes latency on the lookup path.

---

## 6. drain thread (`pty/drain.rs`)

```rust
pub fn spawn_drain_thread(
    session: Arc<PtySession>,
    reader: Box<dyn Read + Send>,        // master.try_clone_reader() — spawn 직후 호출
    status_sink: Arc<dyn StatusSink>,    // D-4: 종료 시 status_changed 호출용 (PtySession엔 없으므로 주입)
    done_tx: std::sync::mpsc::Sender<()>,// D-4: G-1 완료 신호 (session.drain_done_rx와 짝)
) -> std::thread::JoinHandle<()>;
// D-4 갱신: 원안 2인자(session, reader)에서 4인자로. §4 PtySession에 status_sink/done_tx 필드가
// 없어(LLD 자체 모순) drain이 종료 알림+완료 신호를 보내려면 주입 필요. manager.spawn_agent가 연결.
```

### drain thread pseudocode (C2 fix: immediate send, reflects the C3 shutdown sequence)

```
fn drain_loop(session, reader):
    buf = [0u8; 4096]

    loop:
        // 1. blocking read (여기서 대기 — read 자체가 자연 배칭)
        match reader.read(&mut buf):
            Ok(0) | Err(_) => break   // EOF(master drop) or Err(broken pipe) → 종료
            Ok(n)          =>
                data = buf[..n].to_vec()

        // 2. shutdown 확인 (EOF로 먼저 깨지므로 보조 역할)
        if session.shutdown.load(Relaxed): break

        // 3. seq 발급 + 즉시 send (C2: partial batch 정체 없음)
        seq = session.seq.fetch_add(1, Relaxed)
        data_b64 = base64::encode(&data)
        event = PtyEvent { agent_id: session.id, seq, data_b64 }

        // 4. replay buffer 저장 (brief lock)
        session.replay.lock().push(PtyChunk { seq, data })

        // 5. subscriber 스냅샷 + send (lock 밖)
        sinks: Vec<Arc<dyn OutputSink>> = session.subscribers.lock().clone()
        dead_ids = []
        for sink in sinks:
            if sink.send(event.clone()).is_err():
                dead_ids.push(sink.sink_id())

        // 6. 죽은 구독자 제거
        if !dead_ids.is_empty():
            session.subscribers.lock()
                .retain(|s| !dead_ids.contains(&s.sink_id()))

    // G-3: loop 탈출 전 마지막 batch flush (C2 즉시 send 방식이라 해당 없으나 명시)
    // (즉시 send 방식은 batch 없으므로 유실 없음 — 확인용 주석)

    // loop 탈출 → 단일 transition 함수 (M5: race 방지)
    let exit_code = session.child.lock().try_wait()...
    transition(&session, if killed { Event::Killed } else { Event::Exited(code) })
    session.status_sink.status_changed(session.id, ...)

    // G-1: 완료 신호 전송 (kill_agent의 recv_timeout이 수신)
    let _ = drain_done_tx.send(())
```

### drain thread shutdown mechanism (C3 fix: master drop is mandatory)

**Problem:** On ConPTY there are cases where `ReadFile` does not return after `child.kill()`.  
The reader is released only once `ClosePseudoConsole` (= master drop) is called.

**Revised kill_agent order:**

```
1. session.shutdown.store(true, Release)
2. session.child.lock().kill()           // 직속 자식 종료 요청
3. session.child.lock().wait()           // reap (좀비 방지)
4. #[cfg(windows)]
   TerminateJobObject(session.job_handle.0, 1)  // Gemini 신규: 손자 프로세스까지 전멸
   // → Job 내 모든 프로세스 종료 → ConPTY slave 핸들 해제 보장
5. session.master.lock().take()          // C3: master drop → ClosePseudoConsole
                                         // → reader.read() Err(BrokenPipe) 반환
6. // G-1: std JoinHandle에 timeout 없음 → completion channel로 대기
   session.drain_done_rx.lock().take()
       .and_then(|rx| rx.recv_timeout(Duration::from_secs(5)).ok())
   // timeout 시: drain_handle drop (detach) — Arc<PtySession> 참조 유지되므로 leak 아님
   // 단 detach 시 해당 세션 제거 → Arc ref count 감소 → 자연 정리됨
```

**Why TerminateJobObject is needed (Gemini's finding):**  
`child.kill()` terminates only the direct child (e.g. cmd.exe). If a grandchild process (a build script etc.) holds the ConPTY slave handle, the ConPTY pipe does not close and `reader.read()` blocks forever.  
`TerminateJobObject` terminates the whole Job → all slave handles are released → ConPTY ends → the reader is released.

**[drain verification point]** First Stage 2 spike: confirm on a real Windows machine that join completes immediately after kill, without a timeout.

---

## 7. subscribe + replay → live switch seq integrity

```
subscribe(agent_id, sink: Arc<dyn OutputSink>) → SinkId:
    session = sessions.read()[agent_id]

    // C4 수정 (option a): subscribers lock 보유 중 replay 전송
    // → drain의 live send와 replay send가 같은 lock으로 직렬화됨 → 순서 역전 불가
    // 단: drain이 step 5에서 subscribers lock을 잡으려 할 때 대기 (일회성, 허용)
    subscribers_guard = session.subscribers.lock()

    subscribers_guard.push(sink.clone())              // (A) live 구독 등록
    sink_id = sink.sink_id()
    replay_chunks = session.replay.lock().snapshot()  // (B) replay 스냅샷

    // replay 전송 (subscribers lock 보유 중 — C4 핵심)
    for chunk in replay_chunks:
        event = PtyEvent { agent_id, seq: chunk.seq, data_b64: base64(chunk.data) }
        sink.send(event)   // 실패해도 무시 (막 등록된 sink라 unlikely)

    drop(subscribers_guard)   // lock 해제 → drain resume
    return sink_id

unsubscribe(agent_id, sink_id):
    session = sessions.read()[agent_id]
    session.subscribers.lock().retain(|s| s.sink_id() != sink_id)
```

**Explicit exception to lock order rule 3:** subscribe may acquire the `replay` lock while holding the `subscribers` lock (the subscribe function only). The drain thread must never hold both locks at the same time.

**[replay verification point]** Test seq continuity + ordering when a late attach happens during high-speed output.

---

## 8. Tauri Commands layer (`commands/`)

The commands layer is a **thin wrapper with no business logic**. It only calls PtyManager and converts Tauri types.

```rust
// commands/agent.rs

#[tauri::command]
pub async fn spawn_agent(
    state: tauri::State<'_, AppState>,
    cwd: String,
) -> Result<AgentInfo, String>;

#[tauri::command]
pub async fn kill_agent(
    state: tauri::State<'_, AppState>,
    agent_id: String,   // UUID string
) -> Result<(), String>;

#[tauri::command]
pub async fn get_agents(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AgentInfo>, String>;
```

```rust
// commands/pty.rs

#[tauri::command]
pub async fn subscribe_agent_output(
    state:    tauri::State<'_, AppState>,
    agent_id: String,
    channel:  tauri::ipc::Channel<PtyEvent>,
) -> Result<SinkId, String>;    // M2: SinkId 반환 → 프론트 effect cleanup에서 unsubscribe

#[tauri::command]
pub async fn unsubscribe_agent_output(  // M2 추가
    state:    tauri::State<'_, AppState>,
    agent_id: String,
    sink_id:  String,
) -> Result<(), String>;

#[tauri::command]
pub async fn write_stdin(
    state:    tauri::State<'_, AppState>,
    agent_id: String,
    data:     Vec<u8>,    // String 아닌 바이너리
) -> Result<(), String>;

#[tauri::command]
pub async fn resize_pty(
    state:    tauri::State<'_, AppState>,
    agent_id: String,
    cols:     u16,
    rows:     u16,
) -> Result<(), String>;

#[tauri::command]
pub async fn get_agent_snapshot(
    state:    tauri::State<'_, AppState>,
    agent_id: String,
) -> Result<Vec<PtyChunk>, String>;
```

```rust
// lib.rs — AppState

pub struct AppState {
    pub manager: Arc<PtyManager>,   // M1: 외부 Mutex 제거 (PtyManager 내부 동기화 충분)
}
```

---

## 9. AgentStatus state machine transition table

| Current state | Transition → next state | Trigger | Performed by | Resources to clean up |
|---|---|---|---|---|
| Current state | Transition → | Trigger | Performed by | Resources to clean up |
|---|---|---|---|---|
| (none) | → `Running` | `spawn_command` Ok (Starting removed — simplification) | spawn_agent | — |
| (none) | → `Failed` | spawn error | spawn_agent | master, child handles |
| `Running` | → `Exiting` | `kill_agent` call | kill_agent (manager) | — |
| `Running` | → `Exited{code}` | drain read EOF + code | drain thread (transition function) | drain join |
| `Exiting` | → `Killed` | drain shutdown complete | drain thread (transition function) | writer, master, drain_handle |
| `Exited` | (terminal) | — | — | writer, master, drain_handle |
| `Failed` | (terminal) | — | — | (already cleaned up) |
| `Killed` | (terminal) | — | — | (already cleaned up) |

**M5 fix:**
- `Starting` removed — `Running` immediately when spawn_command succeeds
- `Failed` = spawn failure / internal error only. child exit code ≠ 0 → `Exited { code }` (the frontend shows abnormal exit based on the code)
- Terminal transitions (Killed/Exited/Failed) go through the single `transition(session, event)` function — after fetching the child exit code first, it checks the `shutdown` (Acquire) flag **inside the status lock** and decides Killed/Exited with a terminal guard (an already-terminal state is not overwritten). (Implementation: drain.rs)
- **Division of notification responsibility (R-1, no duplicate calls):**
  - The transitional `Exiting` transition and notification = handled by **kill_agent (manager)**. Inside the status lock it sets Exiting if the state is not terminal, and calls `status_changed(Exiting)` **after releasing the lock** (complies with the §10 rule against external calls while holding a lock).
  - Terminal (`Killed`/`Exited`/`Failed`) transitions and notifications = **drain thread alone**.
- The drain's terminal guard safely overwrites kill_agent's Exiting. When a natural EOF races with it, the guard preserves Exited and skips Exiting.

**Note (frontend, T-4):** kill's Exiting notification and the drain's terminal notification can occur concurrently outside the lock → the frontend must not decide terminal state from `status_changed` alone. Decide from `agent_list_updated` (removal from the list).

---

## 10. Concurrency spec

### Lock list and acquisition order rules

| Lock | Type | Held for | External calls allowed while held |
|---|---|---|---|
| `PtyManager.sessions` | `RwLock` | released right after lookup completes | released right after Arc clone |
| `PtySession.subscribers` | `Mutex` | released right after snapshot copy | **forbidden** — send outside the lock |
| `PtySession.replay` | `Mutex` | released right after push/snapshot | **forbidden** |
| `PtySession.writer` | `Mutex` | released right after write completes | blocking I/O allowed (its own only) |
| `PtySession.master` | `Mutex` | released right after resize completes | blocking I/O allowed (its own only) |
| `PtySession.child` | `Mutex` | released right after kill/wait | blocking allowed |
| `PtySession.status` | `Mutex` | released right after read/write | **forbidden** |

**Lock acquisition order rules (deadlock prevention):**

```
규칙 1: sessions read/write lock 보유 중 session 내부 lock 획득 금지.
         → Arc clone 후 sessions lock 해제, 그 다음 session lock 획득.

규칙 2: session 내부 lock 간 획득 순서 (동시에 두 개 이상 필요 시):
         subscribers → replay  (항상 이 순서)
         (subscribe 함수만 두 개 동시 취득, 나머지는 단독 취득)

규칙 3: drain thread가 channel.send 시 어떤 session lock도 보유하지 않음.

규칙 4 (poison 정책, D-3): Mutex poison은 fail-fast — `.lock().expect(...)`로 패닉.
         복구 시도 안 함. drain 등에서 패닉 시 후속 접근은 연쇄 패닉으로 앱 종료(의도된 정책).
```

### Thread list

| Thread | Spawned by | Lifetime | join/detach |
|---|---|---|---|
| Tauri main | OS/Tauri | whole app | — |
| Tokio runtime workers | Tauri | whole app | — |
| drain thread (one per agent) | `PtyManager.spawn_agent` | until PTY EOF | on `kill_agent`, completion channel recv_timeout(5s) |

### Channel topology

| Channel | Direction | Type | When full |
|---|---|---|---|
| `OutputSink.send` (drain→frontend) | drain → Tauri Channel → WebView | Tauri IPC Channel | returns Err → subscriber removed |
| Tauri Event | Rust → all windows | Tauri Event (low frequency) | may be dropped (low-frequency notifications) |

---

## 11. Resource lifetime table

| Resource | Created at | Owner | Release trigger | Released by |
|---|---|---|---|---|
| PTY master | `spawn_agent` | `PtySession.master` | session removed after drain ends | PtySession drop |
| child process | `spawn_agent` | `PtySession.child` | `kill_agent` → child.kill() | kill_agent caller |
| drain thread | `spawn_agent` | `PtyManager` (JoinHandle) | child exit → EOF → loop exit | join() in kill_agent |
| PTY reader | drain thread (moved in) | drain thread stack | drain loop exit | automatic (out of scope) |
| writer | `PtySession.writer` | PtySession | session removed | PtySession drop |
| Channel subscription | `subscribe_agent_output` | `PtySession.subscribers` Vec | send failure detected or window closed | drain thread (cleanup) or unsubscribe |
| Windows Job Object | `spawn_agent` (#[cfg(windows)]) | `PtySession.job_handle` | PtySession drop | CloseHandle (drop impl) |
| ReplayBuffer | `spawn_agent` | `PtySession.replay` | session removed | PtySession drop |

---

## 12. Three shutdown path walkthroughs

### (a) Abnormal child exit

```
Claude Code가 예외로 종료 (exit code ≠ 0)
  → PTY stdout EOF
  → drain thread: reader.read() → Ok(0)
  → drain loop break
  → child.lock().try_wait() → ExitStatus 확인
  → session.status = Exited { code } or Failed { message }
  → app_handle.emit("agent-status-changed", ...)
  → drain thread 종료
  → subscribers의 모든 sink에 마지막 flush (남은 batch 전송)
  → subscribers 목록 clear
```

### (b) Cleaning up all PTYs on whole-app exit

```
Tauri on_window_event(CloseRequested) 또는 tauri::AppHandle::exit()
  → PtyManager.shutdown_all() 호출
  → 모든 session에 대해 병렬로:
      session.shutdown.store(true)
      session.child.lock().kill()
      drain_handle.join(timeout=5s)
      (타임아웃 시: drain_handle.detach(), log warning)
  → Windows: Job Object drop → KillOnJobClose 연쇄 정리
  → sessions 맵 clear
```

### (c) Subscriber cleanup on window close/reload

```
프론트엔드 창 닫힘 (Tauri WebviewWindow closed)
  → Tauri Channel 객체 drop (GC)
  → 다음 drain 사이클에서 sink.send() → SinkError
  → dead_ids에 추가
  → subscribers.lock().retain(|s| !dead_ids.contains(&s.id()))
  → PTY 자체는 계속 실행 (다른 창이 subscribe 중일 수 있음)
  → 모든 구독자 없어져도 PTY는 계속 실행, replay buffer만 유지
```

---

## 13. Windows-specific section (`pty/platform/windows.rs`)

```rust
#[cfg(windows)]
pub struct JobObjectHandle(HANDLE);

#[cfg(windows)]
impl JobObjectHandle {
    /// Job Object 생성 + KILL_ON_JOB_CLOSE 설정
    pub fn new() -> std::io::Result<Self>;
    /// process id를 Job에 편입 (OpenProcess → AssignProcessToJobObject)
    pub fn assign(&self, process_id: u32) -> std::io::Result<()>;
    /// Job 내 전 프로세스 강제 종료 (kill 시퀀스 4단계)
    pub fn terminate(&self, exit_code: u32) -> std::io::Result<()>;
}

#[cfg(windows)]
impl Drop for JobObjectHandle {
    fn drop(&mut self) { unsafe { CloseHandle(self.0) }; }
}
```

> **D-2 update:** The original single function `create_and_assign(pid) -> Result<Self, PtyError>` is split into `new`/`assign`/`terminate` and changed to `io::Result` (better for error handling, and the kill sequence needs terminate). The spike measurement passed. win_err preserves the message via `io::Error::other`.

**Job Object creation flow:**
```
CreateJobObject(NULL, NULL)
  → SetInformationJobObject(handle, JobObjectExtendedLimitInformation,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
            BasicLimitInformation: { LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE }
        })
  → AssignProcessToJobObject(handle, child_process_handle)
```

**Additional Windows ConPTY notes:**
- With `portable-pty 0.8.1`, Windows uses `NativePtySystem::openpty()` → `ConPtySystem`
- Hide the console window in release builds: put `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` at the very top of `main.rs` (console/logs are kept in debug builds)
- Pass VT sequences to xterm.js raw — no parsing/conversion in Rust

**[Windows verification point]** A spawn → kill_agent cycle smoke test, including a forced kill from Task Manager.

---

## 14. Logging + testability (`logging/`)

### Log configuration

```rust
// logging/mod.rs

/// 앱 시작 시 1회 호출 (멱등). 기본 레벨 = RUST_LOG env 우선, 없으면 "warn"(= 평상시 OFF).
/// reload handle을 전역 OnceLock에 보관.
pub fn init_logging();

/// 런타임 레벨 토글. "trace|debug|info|warn|error|off". reload layer로 재설정.
pub fn set_log_level(level: &str) -> Result<(), String>;
```

> **D-1 update:** Relative to the original (ENGRAM_LOG · default INFO · `LogConfig{mask_api_keys}` · returns a handle · verbose-log feature),
> the code changed to RUST_LOG · default warn (= OFF, user requirement "릴리스 기본 OFF") · global OnceLock · no arguments.
> **`mask_api_keys` (API key masking) is not discarded — it is deferred (tracking T-1).** With the default OFF, PTY output does not currently flow into logs, so the risk is low. It must be reintroduced at the point where PTY content can be printed to logs via `set_log_level("debug")` (debug logging/headless) — a security item.

**How to turn logging on/off:**

| Method | Default | How to change |
|---|---|---|
| Environment variable | `ENGRAM_LOG=info` | `ENGRAM_LOG=debug`, `ENGRAM_LOG=off` |
| Build feature | `verbose-log` feature off | `--features verbose-log` |
| Runtime | INFO | `set_log_level()` via Tauri command (can be changed from the debug window) |

**In headless tests:** `ENGRAM_LOG=debug cargo test` → logs are printed for the whole run.

### Core isolation = headless testable

```
pty/ 에는 Tauri import 없음
  → cargo test 가능 (Tauri 런타임 불필요)
  → PtyManager 직접 생성 + MockSink 주입
  → 통합테스트: spawn_agent → write_stdin → read from MockSink → assert
```

```rust
// tests/pty_integration.rs (headless)

#[test]
fn test_spawn_and_output() {
    let manager = PtyManager::new();   // Tauri 없이 생성
    let info = manager.spawn_agent(Path::new("I:/Engram")).unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    manager.subscribe(info.id, Box::new(MpscSink::new(tx))).unwrap();

    // stdout 수신 확인
    let event = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(!event.chunk.data.is_empty());
}
```

**[Test verification point]** `cargo test --package engram-dashboard-backend` — passes without Tauri.

---

## 15. First-review feedback traceability table

| Finding | Source | LLD section where addressed |
|---|---|---|
| emit_all → Channel | GPT | §8 Commands, §6 drain |
| Add PtySession child handle | GPT | §4 PtySession |
| Minimize Mutex scope | GPT | §10 Concurrency, §4 |
| AgentStatus 6 stages | GPT | §9 State machine |
| xterm.js backpressure batching | GPT | §6 drain (batching condition) |
| Windows Job Object | Gemini | §13 Windows |
| Batching on the Rust side | Gemini | §6 drain |
| Vec\<u8\> / Uint8Array | Gemini | §3 PtyChunk |
| fix-path PATH inheritance | Gemini | §16 Decision-rationale (at implementation) |
| Replay attach cols/rows mismatch | Gemini | §7 subscribe (comment) |
| drain blocking read shutdown mechanism | fable | §6 drain shutdown |
| No send while holding a lock | fable | §10 Concurrency rule 3 |
| Detect subscriber send failure and remove | fable | §6 drain pseudocode step8 |
| replay→live seq integrity | fable | §7 |
| Windows-specific section | fable | §13 |
| Backend standalone testability | extra constraint 1 | §14, §1 Module map |
| Log on/off toggle | extra constraint 2 | §14 |
| Reflect constraints in module structure | extra constraint 3 | §1 Module map, §14 |

---

## 16. Decision — rationale — rejected alternatives

| Decision | Rationale | Rejected alternative |
|---|---|---|
| OutputSink trait abstraction | pty/ is isolated from Tauri → headless test possible | Direct dependency on Tauri Channel — cannot test without Tauri |
| Per-field Mutex in PtySession | Maximize concurrency — drain/stdin/resize do not block each other | Single Mutex\<PtySession\> — stdin blocked during drain |
| RwLock for the sessions map | Lookup frequency >> add/remove | Mutex — needless serialization of reads |
| Release the blocking read by ending the child | portable-pty reader has no read_timeout API | CancellationToken — needs an async runtime, mismatched with an OS thread |
| Pin portable-pty 0.8.1 | 0.9.x Windows garbage issue confirmed | Latest version — risky until the smoke test |
| subscribers+replay held together under lock | Blocks the replay→live gap at the root | Sequential locks — a gap can occur |
| drain thread is an OS thread (not async) | Matches portable-pty blocking I/O naturally | tokio::spawn — blocking read exhausts async workers |

---

## 17. Review completion summary (three-way verification answers reflected)

1. **drain thread shutdown** — EOF is not guaranteed. Fixed: added master.take() + TerminateJobObject. [§6]
2. **subscribers+replay held together under lock** — no deadlock. Only the subscribe function acquires both at once (documented as an exception). [§10]
3. **OutputSink + Tauri Channel** — can be wrapped. Pin tauri 2.4. Silent failure is possible even when send is Ok → needs spike measurement. [§2, G-2]
4. **RwLock starvation** — practically none. The drain does not access the sessions map directly. [§10]
5. **AppState Mutex** — removed. `Arc<PtyManager>` confirmed. [§8]
6. **Additional findings** — C1 (serde tag), TerminateJobObject, completion channel, pinning tauri 2.4, and the frontend AgentStatus wire mismatch are all reflected.

---

## 18. S9 revision (session restore) — delta against the Stage-1 finalized version

Basis: `../S9-session-restore/session-restore-lld.md` (§12) + `session-restore-code-plan.md` (§E·§H). The Stage-1 body is preserved; only the semantics that change because of session restore are collected here. Backend items (a)~(d) of the 7 fable findings.

### (a) §9 state machine — re-entry with the same AgentId allowed + epoch
- A restart (restore/restart) **replaces the entry in the sessions map with a new `PtySession` under the same AgentId**. That is, re-entry from `Exited`/`Killed`/`Failed` (terminal) → `Running` is allowed on the manager path only (the drain stays one-way: the terminal guard is kept).
- At **every point** where the map is replaced, increment `epoch` by 1 with `ProfileRegistry::bump_epoch(id)` (restart + **including fresh fallback respawn** — H-1.5). `epoch` rides in the `AgentInfo`/`agent-status-changed` payload and becomes the deterministic trigger for the frontend to resubscribe.
- Added transition row:

  | Current state | Transition → | Trigger | Performed by | Note |
  |---|---|---|---|---|
  | `Exited`/`Killed`/`Failed` | → `Running` (new session) | `restore_all`/`restart_agent` | manager | new PtySession replaces the map entry + epoch++ |

### (b) §10 thread list — restart-only task
- The drain thread **does not respawn directly** after a terminal transition (to avoid entangling it with its own lifetime). The drain only sends a signal to the manager, and a **separate restart task** performs the ladder (resume→fresh→stop), backoff and respawn. However, restart automation is **gated** (after the core is stable) — in the core GO stage only `restore_all` (once at boot, in the background) is implemented.
- Added thread list rows:

  | Thread | Spawned by | Lifetime | Note |
  |---|---|---|---|
  | session-tracker (single) | `SessionTracker::start` | whole app | polls the sid of every agent, holds a stop handle. best-effort |
  | restart task | manager (gated) | while handling a restart | outside the core GO scope |

### (c) §7/§10 stale unsubscribe is harmless
- Right after a restart, an **old sinkId unsubscribe sent by the frontend may arrive at the new session**. `subscribers.retain(|s| s.sink_id() != sink_id)` passes without error even when nothing is found, so it is harmless (already designed that way — stated explicitly only).

### (d) §5/§8 StatusSink extension — restore-result + epoch
- Add a `restore_result(report: RestoreReport)` notification path to `StatusSink` and emit it as an `agent-restore-result` event.
- Add an `epoch: u32` field to `AgentInfo` and `AgentStatusChanged` (the frontend resubscribe trigger — paired with the frontend LLD §4 revision).
- The spawn entry point changes to `spawn_agent(profile, mode)` (a sole cwd argument → profile-based). cwd is normalized with `dunce::canonicalize` before saving.
