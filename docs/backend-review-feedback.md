# 백엔드 설계 외부 검토 결과

**검토일:** 2026-06-08  
**원본 설계:** `backend-architecture-draft.md`

---

## GPT 검토 요약

전문: `backend-architecture-gpt-review.md`

### 확정 변경 사항 (설계 수정 필요)

#### 1. emit_all → Channel로 교체 (핵심)

초안의 `emit_all("pty_output:{id}")` 방식은 PTY 고빈도 출력에 부적합.  
Tauri event는 JSON 직렬화 + 전체 창 broadcast로 저지연/고처리량에 맞지 않음.

**변경안:**
```
Tauri Commands
  spawn_agent(cwd) → AgentInfo
  subscribe_agent_output(agent_id, Channel<PtyEvent>)  ← 신규
  write_stdin(agent_id, data)
  resize_pty(agent_id, cols, rows)
  kill_agent(agent_id)
  get_agents()
  get_agent_snapshot(agent_id)  ← 신규 (replay용)

Tauri Events (저빈도만 유지)
  agent-status-changed
  agent-list-updated
```

멀티 창 fanout: 각 창이 `subscribe_agent_output`으로 명시적 구독. backend는 subscriber 목록에만 전송.

#### 2. PtySession 구조 보강

```rust
pub struct PtySession {
    pub id: AgentId,
    pub cwd: PathBuf,
    pub master: Box<dyn MasterPty + Send>,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,  // ← 추가 (kill/exit 감지)
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,           // ← 별도 lock
    pub status: AgentStatus,
    pub subscribers: Vec<Subscriber>,
    pub replay_buffer: VecDeque<PtyChunk>,                  // ← 추가 (후발 attach용)
    pub seq: u64,
}
```

**Mutex 범위**: 전역 `sessions` lock은 lookup까지만. blocking I/O는 lock 밖에서 수행.

#### 3. AgentStatus 세분화

```rust
enum AgentStatus {
    Starting,
    Running,
    Exiting,
    Exited { code: Option<i32> },
    Failed { message: String },
    Killed,
}
```

#### 4. Windows ConPTY 주의사항

- `portable-pty 0.9.x` Windows read garbage 이슈 있음 → 버전 고정 후 smoke test 필수
- ConPTY는 별도 thread에서 계속 drain 필요 (deadlock 방지)
- Rust 릴리즈 빌드: `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` 확인
- VT sequence 그대로 xterm.js에 전달 (직접 파싱 금지)

#### 5. xterm.js 연동 주의사항

- **backpressure**: PTY 출력을 즉시 무한 emit 금지. 4KB~32KB chunk + 8~16ms batch
- **resize 동기화**: `ResizeObserver → debounce → fitAddon.fit() → invoke("resize_pty")`
- **listener cleanup**: 컴포넌트 unmount 시 반드시 unlisten
- hidden 상태에서 `fit()` 호출 금지 (cols/rows = 0 발생)
- frontend에서 echo 직접 하지 말 것 (PTY 자체 echo 사용)

#### 6. replay buffer

새 창이 나중에 attach할 때 과거 출력 없으면 빈 터미널.  
각 agent마다 최근 N KB ring buffer 유지 → `subscribe_agent_output` 시 replay 후 live stream 전환.

#### 7. Claude Code session 모델 충돌 주의

Claude Code CLI 자체에 `--bg`, `claude agents`, `claude attach` 등 session 관리 기능이 있음.  
**현재 방향(A안)**: Engram이 직접 PTY 관리, agent_id는 Engram 내부 UUID.  
나중에 Claude Code background session과 개념 어긋날 수 있으니 인식하고 진행.

#### 8. spawn_agent 보안

- cwd canonicalize + workspace root 밖 거부
- 실행 binary 고정 (사용자 임의 입력 불가)
- 로그에 API key masking

---

## Gemini 검토 (Gemini 3.5 Thinking)

상태: **완료**  
전문: `backend-architecture-gemini-review.md`

### GPT와 겹치지 않는 추가 지적

#### G1. Windows Job Object — 자식 프로세스 좀비화

Tauri 앱 강제종료/크래시 시 Claude Code 하위 프로세스(Node.js 등)가 백그라운드에 살아남음.  
`kill_agent` 정상 경로에서는 안 죽는 케이스.

**해결:** PTY spawn 시 Windows Job Object 등록 + `KillOnJobClose` 플래그 설정.  
Unix: `prctl(PR_SET_PDEATHSIG, SIGKILL)` (부모 죽으면 자식도 SIGKILL).

#### G2. Rust 단 배칭 누락

설계안엔 프론트엔드 배칭(8~16ms)만 있음.  
drain thread가 수 바이트 단위로 `channel.send` 무차별 호출하면 IPC 파이프 과부하 가능.

**해결:** Rust drain thread에서도 5~10ms 버퍼링 or 4KB 누적 시 send.

#### G3. String 대신 Vec\<u8\> / Uint8Array

PTY 출력을 4KB 단위로 자르면 멀티바이트 UTF-8 또는 ANSI 이스케이프 시퀀스 중간이 잘릴 수 있음.  
`String::from_utf8_lossy` 변환 없이 바이너리 그대로 전송하면 회피 가능.  
xterm.js `terminal.write(Uint8Array)` 네이티브 지원.

#### G4. PATH 상속 문제 (fix-path)

개발 중엔 PTY 환경변수 잘 잡히지만, 프로덕션 패키지(.exe 더블클릭) 실행 시 사용자 PATH가 온전히 상속 안 됨.  
Claude Code 내부에서 node/npm 못 찾아 실패할 수 있음.

**해결:** `spawn_agent` 시 사용자 셸 환경변수 명시적 주입 (`fix-path` 크레이트 또는 직접 구현).

#### G5. 팝업 창 attach 시 cols/rows 불일치

팝업이 나중에 열릴 때 replay buffer 재생하면 메인 창과 cols/rows가 달라 줄바꿈 깨짐.

**해결:** `subscribe_agent_output` 호출 전 팝업이 먼저 `resize_pty`로 크기 전송하거나, Rust에서 세션의 현재 cols/rows 반환 → 팝업이 xterm.js 크기 맞춘 뒤 replay.

---

## 설계 초안 대비 주요 변경점

| 항목 | 초안 | 변경 후 |
|---|---|---|
| PTY 출력 전달 | `emit_all("pty_output:{id}")` | `Channel<PtyEvent>` per subscriber |
| 저빈도 상태 이벤트 | emit_all | Tauri event 유지 |
| PtySession | master + writer | + child + replay_buffer + subscriber list |
| Mutex 범위 | 전체 I/O | lookup까지만 |
| AgentStatus | Running/Idle/Error | Starting/Running/Exiting/Exited/Failed/Killed |
| 멀티 창 fanout | 전체 broadcast + frontend 필터 | 명시적 subscribe |
