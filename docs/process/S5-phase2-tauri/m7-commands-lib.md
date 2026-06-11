# 모듈 7 — commands/ + lib.rs 브리핑 (담당: dcs24, Sonnet)

발신: ed12 (매니저)
근거: backend-lld-stage1.md §8(commands), AppState(§8 끝), frontend-integration-lld.md(event 이름).
목적: **Phase 2 — Tauri 연결 계층.** PTY 코어(pty/)를 Tauri에 노출한다. 이게 백엔드 마지막 wiring.
선행: dco23이 channel_spike 임시코드 정리 완료 상태에서 시작(깨끗한 lib.rs).

## 구성

```
commands/
  ├── mod.rs        # pub use
  ├── agent.rs      # spawn_agent, kill_agent, get_agents
  └── pty.rs        # subscribe/unsubscribe_agent_output, write_stdin, resize_pty, get_agent_snapshot
lib.rs              # AppState, TauriStatusSink, ChannelOutputSink, setup, invoke_handler
```

## 1. commands/ — thin wrapper (§8 그대로, 비즈니스 로직 0)

§8 시그니처 정확히 따른다. 전부 `async`, `Result<_, String>`(에러는 `e.to_string()`). agent_id는 String→Uuid 파싱(`Uuid::parse_str`, 실패 시 Err).

```rust
#[tauri::command]
pub async fn spawn_agent(state: State<'_, AppState>, cwd: String) -> Result<AgentInfo, String> {
    state.manager.spawn_agent(Path::new(&cwd)).map_err(|e| e.to_string())
}
// kill_agent/get_agents/write_stdin(data: Vec<u8>)/resize_pty/get_agent_snapshot 동일 패턴
```

`subscribe_agent_output`만 특별 — Channel을 OutputSink로 래핑:
```rust
#[tauri::command]
pub async fn subscribe_agent_output(
    state: State<'_, AppState>, agent_id: String, channel: tauri::ipc::Channel<PtyEvent>,
) -> Result<SinkId, String> {
    let id = Uuid::parse_str(&agent_id).map_err(|e| e.to_string())?;
    let sink = Arc::new(ChannelOutputSink::new(channel));   // 새 SinkId 내부 생성
    state.manager.subscribe(id, sink).map_err(|e| e.to_string())
}
```

## 2. lib.rs — 핵심 연결부

### AppState
```rust
pub struct AppState { pub manager: Arc<PtyManager> }   // 외부 Mutex 없음(M1)
```

### ChannelOutputSink (OutputSink의 Tauri 구현) — pty/ 밖이라 tauri import OK
```rust
pub struct ChannelOutputSink { id: SinkId, channel: tauri::ipc::Channel<PtyEvent> }
impl ChannelOutputSink {
    pub fn new(channel: Channel<PtyEvent>) -> Self { Self { id: Uuid::new_v4(), channel } }
}
impl OutputSink for ChannelOutputSink {
    fn send(&self, event: PtyEvent) -> Result<(), SinkError> {
        self.channel.send(event).map_err(|_| SinkError::Closed)   // send 실패 → drain이 dead 제거
    }
    fn sink_id(&self) -> SinkId { self.id }
}
```

### TauriStatusSink (StatusSink의 Tauri 구현) — AppHandle로 event emit
```rust
pub struct TauriStatusSink { app: tauri::AppHandle }
impl StatusSink for TauriStatusSink {
    fn status_changed(&self, id: AgentId, status: AgentStatus) {
        // frontend-integration-lld.md event 이름 준수
        let _ = self.app.emit("agent-status-changed", AgentStatusChanged { id, status });
    }
    fn agent_list_updated(&self, agents: Vec<AgentInfo>) {
        let _ = self.app.emit("agent-list-updated", agents);
    }
}
```
> emit 실패는 무시(로그만) — 창이 닫히는 중일 수 있음. 패닉 금지.
> event 페이로드 구조체(AgentStatusChanged 등)는 serde Serialize. 프론트 타입과 일치시킬 것.

### setup (manager 생성 타이밍 주의)
TauriStatusSink는 AppHandle이 필요하므로 **manager를 setup 안에서 생성**:
```rust
.setup(|app| {
    logging::init_logging();                                  // 기본 warn(OFF)
    let status_sink = Arc::new(TauriStatusSink { app: app.handle().clone() });
    let manager = Arc::new(PtyManager::new(status_sink));
    app.manage(AppState { manager });
    Ok(())
})
.invoke_handler(tauri::generate_handler![
    spawn_agent, kill_agent, get_agents,
    subscribe_agent_output, unsubscribe_agent_output,
    write_stdin, resize_pty, get_agent_snapshot,
])
```

## 불변 규칙 (리뷰 필수)

- `pty/` 는 **건드리지 않는다**. tauri import는 commands/lib.rs에만.
- commands는 thin — 파싱/위임/에러변환만, 로직 0.
- emit/channel.send 실패는 패닉 금지(무시+로그).
- dead_code 경고가 이제 대부분 사라져야 함(manager/session 등이 command 통해 used).

## 검증 & 보고

- `cargo fmt --check` + `cargo build` (경고 대폭 감소 확인) + `grep 'use tauri' src/pty/` → 0건 유지.
- `npm run tauri dev` 떠서 빌드/기동 정상인지(아직 프론트 연결은 Phase3라 호출은 안 됨, 컴파일·기동만).
- 보고: `orch 12 "⟁dcs24 commands+lib.rs 완료 — 8 command 등록/TauriStatusSink/ChannelOutputSink/setup, fmt/build OK, dead_code N건"`

막히면 30분 내 중간보고. setup의 manager 생성 타이밍(AppHandle 의존)이 까다로우면 질문.
