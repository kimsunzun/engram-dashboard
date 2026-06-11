# Engram Dashboard — 백엔드 LLD Stage 1 (구조 확정본)

**작성:** engram-dashboard (ed12), 2026-06-11  
**기반:** `backend-architecture-final.md` + GPT/Gemini 검토 반영  
**단계:** 1단계 — 코드 없이 구조 확정. 시그니처/표/의사코드 수준.  
**다음:** 3자 검증(fable ✅ / Gemini 진행 중 / GPT 진행 중) → 확정 → 2단계 모듈별 코드

---

## fable 검토 반영 (2026-06-11) — Critical 4건 + Major 수정

| 항목 | 결정 |
|---|---|
| C1 pty/ 격리 모순 | `StatusSink` trait 추가, `PtyManager.app_handle` 제거 |
| C2 배칭 partial batch 정체 | **즉시 send 방식(option b)** 채택 — read 반환분 즉시 전송, xterm.js 단 배칭 위임 |
| C3 ConPTY kill→EOF 미보장 | kill 순서 변경: `child.kill() → child.wait() → master.take()` → EOF → join |
| C4 replay→live 순서 역전 | **subscribers lock 보유 중 replay 전송(option a)** 채택 |
| M1 AppState 외부 Mutex | `Arc<PtyManager>` 로 변경 (내부 동기화 충분) |
| M2 unsubscribe 부재 | `subscribe` → `SinkId` 반환, `unsubscribe_agent_output` command 추가 |
| M3 Cargo.toml 오류 | tauri-build 위치 수정, thiserror/uuid serde 추가 |
| M4 PtyEvent wire format | **JSON + base64 문자열** 중간 절충, 실측 후 raw IPC 전환 |
| M5 상태 전이 race | `transition()` 단일 함수, Failed/Exited 기준 정리 |
| M6 JoinHandle 위치 | `PtySession.drain_handle` 필드 추가 |
| Minor (확실) | Arc\<dyn OutputSink\>, JobObjectHandle wrapper, windows_subsystem 위치 fix |
| Gemini 신규 | TerminateJobObject kill 시퀀스에 추가 (손자 프로세스 보장) |
| GPT G-1 | JoinHandle timeout → completion channel (drain_done_rx) |
| GPT G-2 | tauri 2.5→2.4 고정 (Channel silent failure 이슈) |
| GPT G-3 | 즉시 send 방식으로 final batch flush 문제 원천 차단 확인 |

---

## 0. 비목표 (Non-goals)

이 LLD 범위 밖:

- Tauri 프론트엔드 연동 코드 (TerminalSlot.tsx 등) — 2단계 이후
- Claude Code 프로세스 자동 시작/재시작 정책 — 추후 결정
- PTY 세션 영속화 (앱 재시작 후 세션 복원) — 추후 결정
- 인증/API 키 관리 — 추후 결정
- Linux 지원 — 현재 타깃 win32 only

---

## 1. 모듈 맵 + 의존 방향

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

**의존 방향 (단방향, 역방향 import 금지):**

```
lib.rs
  └→ commands/          (Tauri 의존)
       └→ pty/           (Tauri 의존 없음 — 핵심 격리)
            └→ logging/
  └→ logging/

  pty/ ──────────────────────────── NO Tauri import
  commands/ ─────────────────────── Tauri + pty 연결층 (thin wrapper only)
```

**핵심 격리 원칙:**  
`pty/` 하위 모든 파일은 `tauri` crate import 금지.  
`OutputSink` trait으로 추상화 → 테스트 시 mock sink, 프로덕션 시 Channel sink.

**[모듈 검증 포인트]** `pty/` 에 `tauri::` import 없음을 CI lint(grep)로 강제.

---

## 2. 의존성 버전 고정 (Cargo.toml)

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

**주의:** `portable-pty 0.8.1` — "0.9.x Windows garbage 이슈" 출처 미확인 (fable M3 지적).  
smoke test 통과하면 최신 버전으로 업그레이드 검토. `tests/smoke_windows.rs` 기준.

---

## 3. 핵심 타입 정의 (`pty/types.rs`)

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

## 4. PtySession 구조체 (`pty/session.rs`)

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

**왜 PtySession 내부를 필드별 별도 Mutex로 분리하는가:**  
drain thread가 replay/subscribers lock만 잠그는 동안 write_stdin이 writer lock만 잠글 수 있어 교착 없이 병행 가능.  
전체 세션 단일 Mutex면 drain 중 stdin 차단, stdin 중 drain 차단 발생.

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
sessions 맵은 read(조회)가 write(추가/삭제)보다 훨씬 빈번 → `RwLock`.  
read lock은 공유 가능, write lock만 배타적. 조회 경로의 지연 최소화.

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

### drain thread 의사코드 (C2 수정: 즉시 send, C3 종료 시퀀스 반영)

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

### drain thread 종료 메커니즘 (C3 수정: master drop 필수)

**문제:** ConPTY에서 `child.kill()` 후 `ReadFile`이 반환 안 되는 케이스 존재.  
`ClosePseudoConsole`(= master drop)이 호출돼야 reader가 해제됨.

**수정된 kill_agent 순서:**

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

**왜 TerminateJobObject가 필요한가 (Gemini 지적):**  
`child.kill()`은 직속 자식(예: cmd.exe)만 종료. 손자 프로세스(빌드 스크립트 등)가 ConPTY slave 핸들을 쥐고 있으면 ConPTY 파이프가 닫히지 않아 `reader.read()` 가 영원히 블로킹됨.  
`TerminateJobObject`로 Job 전체 종료 → 모든 slave 핸들 해제 → ConPTY 종료 → reader 해제.

**[drain 검증 포인트]** 2단계 첫 스파이크: kill 후 join이 타임아웃 없이 즉시 완료되는지 Windows 실기기 확인.

---

## 7. subscribe + replay → live 전환 seq 무결성

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

**락 순서 규칙 3 명시적 예외:** subscribe는 `subscribers` lock 보유 중 `replay` lock 취득 가능 (subscribe 함수 단독). drain thread는 절대 두 lock 동시 보유 금지.

**[replay 검증 포인트]** 고속 출력 중 후발 attach 시 seq 연속성 + 순서 테스트.

---

## 8. Tauri Commands 레이어 (`commands/`)

commands 층은 **비즈니스 로직 없는 thin wrapper**. PtyManager 호출 + Tauri 타입 변환만.

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

## 9. AgentStatus 상태머신 전이표

| 현재 상태 | 전이 → 다음 상태 | 트리거 | 수행 주체 | 정리 자원 |
|---|---|---|---|---|
| 현재 상태 | 전이 → | 트리거 | 수행 주체 | 정리 자원 |
|---|---|---|---|---|
| (없음) | → `Running` | `spawn_command` Ok (Starting 제거 — 단순화) | spawn_agent | — |
| (없음) | → `Failed` | spawn 오류 | spawn_agent | master, child 핸들 |
| `Running` | → `Exiting` | `kill_agent` 호출 | kill_agent (manager) | — |
| `Running` | → `Exited{code}` | drain read EOF + code | drain thread (transition 함수) | drain join |
| `Exiting` | → `Killed` | drain 종료 완료 | drain thread (transition 함수) | writer, master, drain_handle |
| `Exited` | (terminal) | — | — | writer, master, drain_handle |
| `Failed` | (terminal) | — | — | (이미 정리됨) |
| `Killed` | (terminal) | — | — | (이미 정리됨) |

**M5 수정:**
- `Starting` 제거 — spawn_command 성공 시 즉시 `Running`
- `Failed` = spawn 실패 / 내부 오류 전용. child exit code ≠ 0 → `Exited { code }` (프론트가 code로 비정상 표시)
- terminal 전이(Killed/Exited/Failed)는 `transition(session, event)` 단일 함수 경유 — child exit code 선취득 후 **status lock 안에서** `shutdown`(Acquire) 플래그 체크 + terminal 가드(이미 terminal이면 미덮어쓰기)로 Killed/Exited 판정. (구현: drain.rs)
- **알림 책임 분담 (R-1, 중복 호출 금지):**
  - 과도기 `Exiting` 전이·알림 = **kill_agent (manager)** 가 담당. status lock 안에서 terminal 아니면 Exiting 설정, **lock 해제 후** `status_changed(Exiting)` 호출(§10 lock 보유 중 외부 호출 금지 준수).
  - terminal(`Killed`/`Exited`/`Failed`) 전이·알림 = **drain thread 단독**.
- drain의 terminal 가드가 kill_agent의 Exiting을 안전하게 덮어씀. 자연 EOF 경합 시 가드가 Exited 보존 + Exiting 생략.

**주의(프론트, T-4):** kill의 Exiting 알림과 drain의 terminal 알림이 lock 밖 동시 발생 가능 → 프론트는 `status_changed`만으로 terminal 판정 금지. `agent_list_updated`(목록 제거)로 판정.

---

## 10. 동시성 명세

### 락 목록 및 획득 순서 규칙

| 락 | 타입 | 보유 기간 | 보유 중 외부 호출 허용 여부 |
|---|---|---|---|
| `PtyManager.sessions` | `RwLock` | lookup 완료 즉시 해제 | Arc clone 후 즉시 해제 |
| `PtySession.subscribers` | `Mutex` | snapshot 복사 후 즉시 해제 | **금지** — lock 밖에서 send |
| `PtySession.replay` | `Mutex` | push/snapshot 후 즉시 해제 | **금지** |
| `PtySession.writer` | `Mutex` | write 완료 후 즉시 해제 | blocking I/O 허용 (자신만) |
| `PtySession.master` | `Mutex` | resize 완료 후 즉시 해제 | blocking I/O 허용 (자신만) |
| `PtySession.child` | `Mutex` | kill/wait 후 즉시 해제 | blocking 허용 |
| `PtySession.status` | `Mutex` | read/write 후 즉시 해제 | **금지** |

**락 획득 순서 규칙 (데드락 방지):**

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

### 스레드 목록

| 스레드 | spawn 주체 | 수명 | join/detach |
|---|---|---|---|
| Tauri main | OS/Tauri | 앱 전체 | — |
| Tokio runtime workers | Tauri | 앱 전체 | — |
| drain thread (에이전트당 1개) | `PtyManager.spawn_agent` | PTY EOF까지 | `kill_agent` 시 completion channel recv_timeout(5s) |

### 채널 토폴로지

| 채널 | 방향 | 타입 | 가득 찬 경우 |
|---|---|---|---|
| `OutputSink.send` (drain→frontend) | drain → Tauri Channel → WebView | Tauri IPC Channel | Err 반환 → subscriber 제거 |
| Tauri Event | Rust → 모든 창 | Tauri Event (저빈도) | 드롭 가능 (저빈도 알림) |

---

## 11. 자원 수명 표

| 자원 | 생성 시점 | 소유자 | 해제 트리거 | 해제 주체 |
|---|---|---|---|---|
| PTY master | `spawn_agent` | `PtySession.master` | drain 종료 후 session 제거 | PtySession drop |
| child process | `spawn_agent` | `PtySession.child` | `kill_agent` → child.kill() | kill_agent 호출자 |
| drain thread | `spawn_agent` | `PtyManager` (JoinHandle) | child 종료 → EOF → loop 탈출 | join() in kill_agent |
| PTY reader | drain thread (moved in) | drain thread 스택 | drain loop 탈출 | 자동 (out of scope) |
| writer | `PtySession.writer` | PtySession | session 제거 | PtySession drop |
| Channel 구독 | `subscribe_agent_output` | `PtySession.subscribers` Vec | send 실패 감지 or 창 닫힘 | drain thread (정리) or unsubscribe |
| Windows Job Object | `spawn_agent` (#[cfg(windows)]) | `PtySession.job_handle` | PtySession drop | CloseHandle (drop impl) |
| ReplayBuffer | `spawn_agent` | `PtySession.replay` | session 제거 | PtySession drop |

---

## 12. 종료 경로 워크스루 3종

### (a) child 비정상 종료

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

### (b) 앱 전체 종료 시 PTY 전체 정리

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

### (c) 창 닫힘/reload 시 구독자 정리

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

## 13. Windows 전용 절 (`pty/platform/windows.rs`)

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

> **D-2 갱신:** 원안 `create_and_assign(pid) -> Result<Self, PtyError>` 단일 함수에서 `new`/`assign`/`terminate` 분리 + `io::Result`로 변경(에러 처리 유리, kill 시퀀스가 terminate 필요). spike 실측 통과. win_err는 `io::Error::other`로 메시지 보존.

**Job Object 생성 흐름:**
```
CreateJobObject(NULL, NULL)
  → SetInformationJobObject(handle, JobObjectExtendedLimitInformation,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
            BasicLimitInformation: { LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE }
        })
  → AssignProcessToJobObject(handle, child_process_handle)
```

**Windows ConPTY 추가 주의:**
- `portable-pty 0.8.1` 기준 Windows는 `NativePtySystem::openpty()` → `ConPtySystem` 사용
- 릴리즈 빌드 콘솔 창 숨김: `main.rs` 최상단에 `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` (debug 빌드에서 콘솔/로그 유지)
- VT sequence는 xterm.js에 raw 전달 — Rust에서 파싱/변환 금지

**[Windows 검증 포인트]** spawn → kill_agent 사이클을 작업관리자 강제종료 포함 smoke test.

---

## 14. 로깅 + 테스트 가능성 (`logging/`)

### 로그 설정

```rust
// logging/mod.rs

/// 앱 시작 시 1회 호출 (멱등). 기본 레벨 = RUST_LOG env 우선, 없으면 "warn"(= 평상시 OFF).
/// reload handle을 전역 OnceLock에 보관.
pub fn init_logging();

/// 런타임 레벨 토글. "trace|debug|info|warn|error|off". reload layer로 재설정.
pub fn set_log_level(level: &str) -> Result<(), String>;
```

> **D-1 갱신:** 원안(ENGRAM_LOG · 기본 INFO · `LogConfig{mask_api_keys}` · handle 반환 · verbose-log feature)에서
> 코드는 RUST_LOG · 기본 warn(=OFF, 사용자 요구 "릴리스 기본 OFF") · 전역 OnceLock · 인자 없음으로 변경.
> **`mask_api_keys`(API 키 마스킹)는 폐기 아님 — 보류(tracking T-1).** 기본 OFF라 현재 PTY 출력이 로그로 흐르지 않아 위험 낮음. `set_log_level("debug")`로 PTY 내용이 로그에 찍힐 수 있는 시점(debug 로깅/headless)에 재도입 필수 — 보안 항목.

**로그 on/off 방식:**

| 방식 | 기본값 | 변경 방법 |
|---|---|---|
| 환경변수 | `ENGRAM_LOG=info` | `ENGRAM_LOG=debug`, `ENGRAM_LOG=off` |
| 빌드 feature | `verbose-log` feature off | `--features verbose-log` |
| 런타임 | INFO | `set_log_level()` via Tauri command (디버그 창에서 변경 가능) |

**headless 테스트 시:** `ENGRAM_LOG=debug cargo test` → 전 구간 로그 출력.

### 코어 격리 = headless 테스트 가능

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

**[테스트 검증 포인트]** `cargo test --package engram-dashboard-backend` — Tauri 없이 통과.

---

## 15. 1차 리뷰 반영 추적표

| 지적 사항 | 출처 | LLD 반영 절 |
|---|---|---|
| emit_all → Channel | GPT | §8 Commands, §6 drain |
| PtySession child handle 추가 | GPT | §4 PtySession |
| Mutex 범위 최소화 | GPT | §10 동시성, §4 |
| AgentStatus 6단계 | GPT | §9 상태머신 |
| xterm.js backpressure batching | GPT | §6 drain (배칭 조건) |
| Windows Job Object | Gemini | §13 Windows |
| Rust 단 배칭 | Gemini | §6 drain |
| Vec\<u8\> / Uint8Array | Gemini | §3 PtyChunk |
| fix-path PATH 상속 | Gemini | §16 결정-근거 (구현 시) |
| Replay attach cols/rows 불일치 | Gemini | §7 subscribe (주석) |
| drain blocking read 종료 메커니즘 | fable | §6 drain 종료 |
| lock 보유 중 send 금지 | fable | §10 동시성 규칙3 |
| subscriber send실패 감지 제거 | fable | §6 drain 의사코드 step8 |
| replay→live seq 무결성 | fable | §7 |
| Windows 전용 절 | fable | §13 |
| 백엔드 단독 테스트 가능성 | 추가제약1 | §14, §1 모듈맵 |
| 로그 on/off 토글 | 추가제약2 | §14 |
| 모듈 구조에 제약 반영 | 추가제약3 | §1 모듈맵, §14 |

---

## 16. 결정 — 근거 — 기각한 대안

| 결정 | 근거 | 기각한 대안 |
|---|---|---|
| OutputSink trait 추상화 | pty/ Tauri 격리 → headless test 가능 | Tauri Channel 직접 의존 — Tauri 없이 테스트 불가 |
| PtySession 필드별 Mutex | 병행성 최대화 — drain/stdin/resize 서로 안 막힘 | 단일 Mutex\<PtySession\> — drain 중 stdin 차단 |
| RwLock for sessions 맵 | 조회 빈도 >> 추가/삭제 | Mutex — 불필요한 read 직렬화 |
| child 종료로 blocking read 해제 | portable-pty reader에 read_timeout API 없음 | CancellationToken — async 런타임 필요, OS thread와 불일치 |
| portable-pty 0.8.1 고정 | 0.9.x Windows garbage issue 확인됨 | 최신 버전 — smoke test 전까지 위험 |
| subscribers+replay 동시 lock | replay→live gap 원천 차단 | 순차 lock — gap 발생 가능 |
| drain thread OS thread (not async) | portable-pty blocking I/O와 자연스럽게 일치 | tokio::spawn — blocking read가 async worker 고갈 유발 |

---

## 17. 검토 완료 요약 (3자 검증 답변 반영)

1. **drain thread 종료** — EOF 보장 안 됨. 수정됨: master.take() + TerminateJobObject 추가. [§6]
2. **subscribers+replay 동시 lock** — 데드락 없음. subscribe 함수만 동시 취득(예외로 문서화). [§10]
3. **OutputSink + Tauri Channel** — 래핑 가능. tauri 2.4 고정. send Ok여도 silent failure 가능 → 스파이크 실측 필요. [§2, G-2]
4. **RwLock starvation** — 사실상 없음. drain이 sessions 맵 직접 접근 안 함. [§10]
5. **AppState Mutex** — 제거됨. `Arc<PtyManager>` 확정. [§8]
6. **추가 발견** — C1(serde tag), TerminateJobObject, completion channel, tauri 2.4 고정, frontend AgentStatus wire 불일치 모두 반영.
