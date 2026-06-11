# Engram Dashboard — Rust 백엔드 설계 초안

## 목적

Tauri v2 앱에서 실제 Claude Code 프로세스를 PTY로 실행하고,
터미널 출력을 xterm.js에 스트리밍하는 백엔드 구조 설계.

## 기술 선택

| 항목 | 선택 | 이유 |
|---|---|---|
| PTY | `portable-pty` crate | Windows/Mac/Linux 크로스플랫폼, 진짜 PTY (ANSI 완전 지원) |
| 프레임워크 | Tauri v2 | 이미 사용 중 |
| 상태 관리 | Rust `Arc<Mutex<HashMap>>` | 멀티 PTY 세션 관리 |
| 창→백엔드 | Tauri invoke | 명령 (spawn/kill/resize/write) |
| 백엔드→창 | Tauri emit | 스트리밍 출력, 상태 변경 알림 |

---

## 에이전트 모델

- **에이전트 = Claude Code 세션 = PTY 프로세스 1개**
- 같은 폴더에 여러 세션 가능 → 트리에서 폴더가 부모, 세션이 자식
- 에이전트 ID (UUID) ↔ PTY 핸들 1:1 바인딩

---

## Rust 구조

### PTY Manager (`src-tauri/src/pty_manager.rs`)

```rust
pub struct PtySession {
    pub id: String,          // 에이전트 UUID
    pub master: Box<dyn MasterPty>,
    pub writer: Box<dyn Write>,
    pub cwd: String,
    pub status: AgentStatus, // Running | Idle | Error | Exited
}

pub struct PtyManager {
    sessions: Arc<Mutex<HashMap<String, PtySession>>>,
}
```

### Tauri Commands (invoke)

| 커맨드 | 인자 | 동작 |
|---|---|---|
| `spawn_agent` | `cwd: String` | PTY 생성, Claude Code 실행, agent_id 반환 |
| `kill_agent` | `agent_id: String` | PTY 종료 |
| `write_stdin` | `agent_id, data: String` | PTY stdin에 키입력 전달 |
| `resize_pty` | `agent_id, cols, rows: u16` | PTY 크기 변경 |
| `get_agents` | - | 전체 세션 목록 반환 |

### Tauri Events (emit)

| 이벤트 | payload | 수신자 |
|---|---|---|
| `pty_output:{agent_id}` | `String` (ANSI) | 해당 에이전트 구독 중인 모든 창 |
| `agent_status_changed` | `{ id, status }` | 모든 창 (AgentTree 업데이트) |
| `agent_list_updated` | `Vec<AgentInfo>` | 모든 창 |

---

## 멀티 창 fanout

메인 창 + 팝업 창이 동시에 같은 PTY 출력 받는 방법:

- PTY 읽기 스레드가 루프에서 출력 읽음
- `app_handle.emit_all("pty_output:{id}", chunk)` 로 전체 창 broadcast
- 각 창은 필요한 agent_id만 구독, 나머지 무시

---

## 프론트엔드 변경

### agentStore.ts
```ts
// 더미 데이터 제거
fetchAgents: async () => {
  const agents = await invoke('get_agents')
  set({ agents })
},
spawnAgent: async (cwd: string) => {
  const id = await invoke('spawn_agent', { cwd })
  // agent_list_updated 이벤트로 자동 갱신
},
```

### TerminalSlot.tsx
```ts
// 더미 write 제거
useEffect(() => {
  const unlisten = await listen(`pty_output:${agentId}`, (e) => {
    terminal.write(e.payload)
  })
  return unlisten
}, [agentId])

// 키입력 전달
terminal.onData((data) => {
  invoke('write_stdin', { agentId, data })
})
```

---

## 데이터 흐름

```
사용자 키입력
  → xterm.js onData
  → invoke('write_stdin')
  → Rust PTY stdin
  → Claude Code 처리
  → PTY stdout
  → Rust 읽기 스레드
  → emit_all('pty_output:{id}')
  → 구독 중인 모든 창 xterm.js.write()
```

---

## 구현 순서 (제안)

1. Rust PTY spawn + 기본 읽기 루프
2. `get_agents` / `spawn_agent` / `kill_agent` invoke
3. `pty_output` event → xterm.js 연결
4. `write_stdin` + `resize_pty`
5. `agent_status_changed` 이벤트 → AgentTree 실시간 업데이트

---

## 검토 요청 사항

1. `portable-pty`가 Tauri v2 + Windows 환경에서 실전 사용 가능한가? 알려진 이슈?
2. PTY 읽기 스레드에서 `emit_all` 하는 방식이 멀티 창 fanout에 적절한가? 더 나은 패턴?
3. 이벤트 이름 `pty_output:{agent_id}` 동적 채널 방식 vs 단일 채널 + payload 필터링 — 어느 쪽이 Tauri v2에서 더 권장?
4. xterm.js ↔ Tauri v2 연동 시 알려진 pitfall?
5. 이 구조에서 놓친 것, 더 개선할 설계 포인트?
