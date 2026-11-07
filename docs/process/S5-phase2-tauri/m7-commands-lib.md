# Module 7 — commands/ + lib.rs briefing (owner: dcs24, Sonnet)

From: ed12 (manager)
Source: backend-lld-stage1.md §8 (commands), AppState (end of §8), frontend-integration-lld.md (event names).
Purpose: **Phase 2 — the Tauri connection layer.** Expose the PTY core (pty/) to Tauri. This is the last wiring of the backend.
Prerequisite: starts after dco23 has finished cleaning up the temporary channel_spike code (a clean lib.rs).

## Layout

```
commands/
  ├── mod.rs        # pub use
  ├── agent.rs      # spawn_agent, kill_agent, get_agents
  └── pty.rs        # subscribe/unsubscribe_agent_output, write_stdin, resize_pty, get_agent_snapshot
lib.rs              # AppState, TauriStatusSink, ChannelOutputSink, setup, invoke_handler
```

## 1. commands/ — thin wrappers (§8 as written, zero business logic)

Follow the §8 signatures exactly. All `async`, `Result<_, String>` (errors via `e.to_string()`). agent_id is parsed String→Uuid (`Uuid::parse_str`, Err on failure).

```rust
#[tauri::command]
pub async fn spawn_agent(state: State<'_, AppState>, cwd: String) -> Result<AgentInfo, String> {
    state.manager.spawn_agent(Path::new(&cwd)).map_err(|e| e.to_string())
}
// kill_agent/get_agents/write_stdin(data: Vec<u8>)/resize_pty/get_agent_snapshot 동일 패턴
```

Only `subscribe_agent_output` is special — it wraps the Channel as an OutputSink:
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

## 2. lib.rs — the core wiring

### AppState
```rust
pub struct AppState { pub manager: Arc<PtyManager> }   // 외부 Mutex 없음(M1)
```

### ChannelOutputSink (the Tauri implementation of OutputSink) — outside pty/, so tauri imports are OK
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

### TauriStatusSink (the Tauri implementation of StatusSink) — emits events through AppHandle
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
> Ignore emit failures (log only) — the window may be closing. No panics.
> The event payload structs (AgentStatusChanged etc.) are serde Serialize. Match them to the frontend types.

### setup (mind when the manager is created)
TauriStatusSink needs an AppHandle, so **create the manager inside setup**:
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

## Invariants (mandatory for review)

- **Do not touch** `pty/`. tauri imports only in commands/lib.rs.
- commands are thin — parsing/delegating/error conversion only, zero logic.
- No panic on emit/channel.send failure (ignore + log).
- Most dead_code warnings should now be gone (manager/session etc. are used through the commands).

## Verification and report

- `cargo fmt --check` + `cargo build` (confirm warnings drop sharply) + `grep 'use tauri' src/pty/` → stays at 0 hits.
- Bring up `npm run tauri dev` and check that build/startup is normal (the frontend connection is Phase 3, so nothing is called yet — only compile and startup).
- Report: `orch 12 "⟁dcs24 commands+lib.rs 완료 — 8 command 등록/TauriStatusSink/ChannelOutputSink/setup, fmt/build OK, dead_code N건"`

If stuck, send an interim report within 30 minutes. If the timing of manager creation in setup (the AppHandle dependency) is tricky, ask.
