# Engram Dashboard — Rust 백엔드 설계 최종본

**작성일:** 2026-06-08  
**기반:** draft + GPT 검토 반영

---

## 기술 선택

| 항목 | 선택 | 이유 |
|---|---|---|
| PTY | `portable-pty` crate | 크로스플랫폼 (Windows ConPTY / Unix PTY), 진짜 PTY |
| 프레임워크 | Tauri v2 | 이미 사용 중 |
| PTY 출력 전달 | Tauri v2 **Channel** | 고빈도 스트리밍에 적합 (emit_all 비권장) |
| 저빈도 상태 알림 | Tauri **Event** | agent list/status 변경 등 |
| 세션 관리 | `Arc<Mutex<HashMap>>` | 멀티 PTY 동시 관리 |

---

## 에이전트 모델

- **에이전트 = Claude Code 세션 = PTY 프로세스 1개**
- 같은 폴더에 여러 세션 가능 → 트리에서 폴더(부모) / 세션(자식 리프)
- `AgentId` (UUID) ↔ `PtySession` 1:1

---

## Rust 핵심 구조

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

**Mutex 전략:**
- 전역 `sessions` lock → lookup까지만 (clone Arc 후 즉시 해제)
- 각 session 내 `writer` lock → stdin I/O 전용
- blocking I/O는 전역 lock 밖에서 수행 (다른 agent 차단 방지)

---

## Tauri Commands (invoke)

| 커맨드 | 인자 | 반환 | 동작 |
|---|---|---|---|
| `spawn_agent` | `cwd: String` | `AgentInfo` | PTY 생성 + Claude Code 실행 |
| `subscribe_agent_output` | `agent_id`, `channel: Channel<PtyEvent>` | - | PTY 출력 구독 등록 + replay 전송 |
| `write_stdin` | `agent_id`, `data: String` | - | PTY stdin 전달 |
| `resize_pty` | `agent_id`, `cols: u16`, `rows: u16` | - | PTY 크기 변경 |
| `kill_agent` | `agent_id` | - | PTY + 자식 프로세스 종료 |
| `get_agents` | - | `Vec<AgentInfo>` | 전체 세션 목록 |
| `get_agent_snapshot` | `agent_id` | `Vec<PtyChunk>` | replay buffer 조회 |

## Tauri Events (emit) — 저빈도만

| 이벤트 | payload | 용도 |
|---|---|---|
| `agent-status-changed` | `{ id, status }` | 모든 창 AgentTree 업데이트 |
| `agent-list-updated` | `Vec<AgentInfo>` | 에이전트 목록 변경 시 |

---

## PTY 출력 흐름 (Channel 기반)

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

**왜 Channel인가:**
- Tauri event는 JSON 직렬화 + 전체 창 broadcast → 고빈도 스트리밍 부적합
- Channel은 특정 구독자에게 직접 전달, 바이너리 가능

---

## 멀티 창 fanout

```
메인 창 ──── subscribe_agent_output(agentA, ch1) ──┐
팝업 창 ──── subscribe_agent_output(agentA, ch2) ──┼─→ PtySession.subscribers = [ch1, ch2]
                                                   └─→ drain thread → 둘 다에 send
```

각 창은 원하는 agent를 명시적으로 구독. 창 닫힐 때 channel drop → subscriber 자동 제거.

---

## 후발 창 attach (replay)

```
새 팝업 창이 agentA에 subscribe_agent_output 호출
  → Rust: replay_buffer 순서대로 channel.send (과거 출력 재생)
  → 이후 live stream으로 자연스럽게 이어짐
```

---

## 프론트엔드 변경 포인트

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

### resize 동기화

```ts
const resizeObserver = new ResizeObserver(debounce(() => {
  fitAddon.fit()
  invoke('resize_pty', { agentId, cols: terminal.cols, rows: terminal.rows })
}, 50))
```

---

## Windows ConPTY 주의사항

- Windows 10 1809 (October 2018 Update) 이상 필수
- `portable-pty` 버전 고정 후 Windows 실기기 smoke test 필수
- drain thread 필수 (Microsoft ConPTY 권장 사항 — 별도 thread에서 I/O)
- VT sequence 그대로 xterm.js 전달 (직접 파싱 금지)
- 릴리즈 빌드 콘솔 창 숨김: `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`

---

## 보안

- `spawn_agent`: cwd canonicalize + workspace root 밖 거부
- 실행 binary 고정 (사용자 임의 지정 불가)
- 로그 API key masking

---

## 구현 순서

1. Rust PTY spawn + drain thread + Channel 기반 구독
2. `get_agents` / `spawn_agent` / `kill_agent`
3. `subscribe_agent_output` → xterm.js Channel 연결
4. `write_stdin` + `resize_pty`
5. `agent-status-changed` 이벤트 → AgentTree 실시간 업데이트
6. replay buffer (후발 attach)

---

## 검토 요청 사항

1. `portable-pty`가 Tauri v2 + Windows 환경에서 실전 사용 가능한가? 알려진 이슈?
2. Tauri v2 Channel 기반 PTY 스트리밍 — 이 설계에서 놓친 pitfall?
3. drain thread + replay buffer + subscriber 관리 구조의 개선 포인트?
4. xterm.js ↔ Tauri v2 Channel 연동 주의사항?
5. 전체 구조에서 놓친 것, 더 개선할 설계 포인트?
