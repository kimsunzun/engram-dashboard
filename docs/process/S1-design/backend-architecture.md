# Engram Dashboard — Rust Backend Design, Final

**Written:** 2026-06-08  
**Based on:** draft + GPT review reflected

---

## Technology choices

| Item | Choice | Reason |
|---|---|---|
| PTY | `portable-pty` crate | Cross-platform (Windows ConPTY / Unix PTY), a real PTY |
| Framework | Tauri v2 | Already in use |
| PTY output delivery | Tauri v2 **Channel** | Suited to high-frequency streaming (emit_all not recommended) |
| Low-frequency state notification | Tauri **Event** | agent list/status changes, etc. |
| Session management | `Arc<Mutex<HashMap>>` | Manages multiple PTYs at once |

---

## Agent model

- **Agent = Claude Code session = 1 PTY process**
- Multiple sessions are possible in the same folder → in the tree, folder (parent) / session (child leaf)
- `AgentId` (UUID) ↔ `PtySession` 1:1

---

## Rust core structures

### AgentStatus

```rust
pub enum AgentStatus {
    Starting,
    Running,
    Exiting,
    Exited { code: Option<i32> },
    Failed { message: String },
    Killed,
}
```

### PtySession

```rust
pub struct PtySession {
    pub id: AgentId,                                          // UUID
    pub cwd: PathBuf,
    pub master: Box<dyn MasterPty + Send>,                    // PTY 마스터 (크기 변경)
    pub child: Box<dyn portable_pty::Child + Send + Sync>,    // 자식 프로세스 (kill/wait)
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,            // stdin 전용 lock
    pub status: AgentStatus,
    pub subscribers: Vec<tauri::ipc::Channel<PtyEvent>>,      // 구독 중인 창 목록
    pub replay_buffer: VecDeque<PtyChunk>,                    // 후발 창 attach용 최근 출력
    pub seq: u64,                                             // 청크 순서 번호
}
```

### PtyManager

```rust
pub struct PtyManager {
    sessions: Arc<Mutex<HashMap<AgentId, Arc<Mutex<PtySession>>>>>,
}
```

**Mutex strategy:**
- The global `sessions` lock → only up to the lookup (released immediately after cloning the Arc)
- The `writer` lock inside each session → dedicated to stdin I/O
- Blocking I/O is performed outside the global lock (to avoid blocking other agents)

---

## Tauri Commands (invoke)

| Command | Arguments | Returns | Behavior |
|---|---|---|---|
| `spawn_agent` | `cwd: String` | `AgentInfo` | Create the PTY + run Claude Code |
| `subscribe_agent_output` | `agent_id`, `channel: Channel<PtyEvent>` | - | Register the PTY output subscription + send the replay |
| `write_stdin` | `agent_id`, `data: String` | - | Deliver to PTY stdin |
| `resize_pty` | `agent_id`, `cols: u16`, `rows: u16` | - | Change the PTY size |
| `kill_agent` | `agent_id` | - | Terminate the PTY + child process |
| `get_agents` | - | `Vec<AgentInfo>` | Full session list |
| `get_agent_snapshot` | `agent_id` | `Vec<PtyChunk>` | Query the replay buffer |

## Tauri Events (emit) — low-frequency only

| Event | payload | Purpose |
|---|---|---|
| `agent-status-changed` | `{ id, status }` | Update the AgentTree in all windows |
| `agent-list-updated` | `Vec<AgentInfo>` | When the agent list changes |

---

## PTY output flow (Channel-based)

```
Claude Code 실행
  → PTY stdout
  → Rust drain thread (별도 thread, 항상 read loop)
  → 4KB~32KB chunk + seq 번호 부여
  → replay_buffer에 저장 (ring buffer, 최근 N KB)
  → session.subscribers 순회
  → channel.send(PtyEvent { agent_id, seq, data })
  → 각 창 frontend Channel callback
  → xterm.js write(data) (8~16ms batch)
```

**Why Channel:**
- A Tauri event is JSON serialization + broadcast to all windows → unsuited to high-frequency streaming
- A Channel delivers directly to a specific subscriber, binary possible

---

## Multi-window fanout

```
메인 창 ──── subscribe_agent_output(agentA, ch1) ──┐
팝업 창 ──── subscribe_agent_output(agentA, ch2) ──┼─→ PtySession.subscribers = [ch1, ch2]
                                                   └─→ drain thread → 둘 다에 send
```

Each window explicitly subscribes to the agent it wants. When a window closes, the channel is dropped → the subscriber is removed automatically.

---

## Late-window attach (replay)

```
새 팝업 창이 agentA에 subscribe_agent_output 호출
  → Rust: replay_buffer 순서대로 channel.send (과거 출력 재생)
  → 이후 live stream으로 자연스럽게 이어짐
```

---

## Frontend change points

### agentStore.ts

```ts
fetchAgents: async () => {
  const agents = await invoke('get_agents')
  set({ agents })
},
spawnAgent: async (cwd: string) => {
  const info = await invoke('spawn_agent', { cwd })
  // agent-list-updated 이벤트로 자동 갱신
},
```

### TerminalSlot.tsx

```ts
useEffect(() => {
  let channel: Channel<PtyEvent> | null = null
  invoke('subscribe_agent_output', {
    agentId,
    channel: new Channel<PtyEvent>((event) => {
      terminal.write(event.data)  // xterm.js에 직접 전달
    })
  })
  return () => { /* channel GC시 subscriber 자동 제거 */ }
}, [agentId])

terminal.onData((data) => {
  invoke('write_stdin', { agentId, data })
})
```

### resize sync

```ts
const resizeObserver = new ResizeObserver(debounce(() => {
  fitAddon.fit()
  invoke('resize_pty', { agentId, cols: terminal.cols, rows: terminal.rows })
}, 50))
```

---

## Windows ConPTY notes

- Windows 10 1809 (October 2018 Update) or later is required
- After pinning the `portable-pty` version, a smoke test on a real Windows machine is mandatory
- A drain thread is mandatory (Microsoft ConPTY recommendation — I/O on a separate thread)
- Pass VT sequences to xterm.js as they are (no direct parsing)
- Hide the console window in release builds: `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`

---

## Security

- `spawn_agent`: canonicalize cwd + reject outside the workspace root
- Fixed executable binary (the user cannot specify an arbitrary one)
- Mask API keys in logs

---

## Implementation order

1. Rust PTY spawn + drain thread + Channel-based subscription
2. `get_agents` / `spawn_agent` / `kill_agent`
3. `subscribe_agent_output` → connect the xterm.js Channel
4. `write_stdin` + `resize_pty`
5. `agent-status-changed` event → real-time AgentTree update
6. replay buffer (late attach)

---

## Review requests

1. Is `portable-pty` usable in practice with Tauri v2 + Windows? Known issues?
2. Tauri v2 Channel-based PTY streaming — any pitfalls missed in this design?
3. Improvement points for the drain thread + replay buffer + subscriber management structure?
4. Cautions for connecting xterm.js ↔ Tauri v2 Channel?
5. Anything missed in the overall structure, or design points that can be improved further?
