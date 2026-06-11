# GPT 검토 결과: backend-architecture-draft

**검토 일시:** 2026-06-08

---

먼저 전제만 짚으면, 제가 이 대화에서 실제 모델을 GPT-4o로 전환한다고 보장할 수는 없습니다. 대신 2026-06-08 기준 최신 문서와 이슈를 확인해서 설계 검토 형태로 답변하겠습니다.

## 총평

현재 초안은 MVP로는 충분히 가능한 구조입니다. 다만 실전형으로 보려면 핵심 수정점이 있습니다.

가장 중요한 변경은 PTY 출력 스트리밍을 Tauri event `emit_all` 중심으로 설계하지 말고, **Tauri v2 Channel 또는 별도 subscription/broadcast 계층 중심으로 바꾸는 것**입니다. Tauri 공식 문서도 event는 작은 데이터/멀티 프로듀서-컨슈머용이고, 저지연·고처리량 스트리밍에는 적합하지 않으며, 스트리밍 데이터에는 Channel을 권장한다고 설명합니다.

---

## 1. portable-pty가 Tauri v2 + Windows 환경에서 실전 사용 가능한가? 알려진 이슈?

**결론: 사용 가능하다. 다만 Windows는 반드시 ConPTY 특성을 감안해야 한다.**

portable-pty 자체는 크로스플랫폼 PTY API를 제공하고, Windows처럼 여러 PTY 구현이 있을 수 있는 환경에서 런타임 선택이 가능하도록 설계된 crate입니다. 공식 docs.rs 예제도 `native_pty_system()`, `openpty()`, `spawn_command()`, `try_clone_reader()`, `take_writer()` 흐름을 보여주고 있어 현재 초안의 방향과 잘 맞습니다.

Windows에서는 사실상 ConPTY 기반으로 보게 됩니다. Microsoft의 CreatePseudoConsole 문서는 최소 지원 클라이언트를 Windows 10 October 2018 Update, version 1809로 명시하고, 입력/출력 스트림이 UTF-8 텍스트와 Virtual Terminal Sequence가 섞인 형태라고 설명합니다. 즉 xterm.js 같은 ANSI/VT 처리 가능한 터미널 에뮬레이터와 붙이는 방향은 맞습니다.

**주의할 알려진 이슈:**
- portable-pty 0.9.0에서 Windows pty.read 결과가 "garbage"처럼 보인다는 GitHub issue가 열려 있음
- 별도로 Tauri build에서 Windows에 cmd 창이 뜬다는 보고도 있음
- 두 이슈 모두 "무조건 사용 불가"라기보다는 Windows ConPTY, crate 버전, subsystem 설정, console window 숨김 처리, VT 시퀀스 해석 문제를 검증해야 한다는 신호

**권장안:**
- Windows 10 1809 미만은 지원 대상에서 제외하거나 명확히 에러 처리
- portable-pty 버전은 고정하고, Windows 실기기에서 cmd, PowerShell, Git Bash, Claude Code 각각 smoke test
- Rust 앱 릴리즈 빌드에서 콘솔 창이 뜨면 `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` 적용 여부 확인
- ConPTY는 synchronous I/O 특성이 있어 deadlock 방지를 위해 입출력 채널을 별도 thread에서 계속 drain하는 구조가 안전합니다. Microsoft도 pseudoconsole 통신 채널을 개별 thread에서 처리하라고 경고합니다.

---

## 2. PTY 읽기 스레드에서 emit_all 하는 방식이 멀티 창 fanout에 적절한가? 더 나은 패턴?

**결론: MVP에서는 가능하지만, 실전 구조로는 부적절합니다.**

Tauri v2 기준으로는 용어도 조금 바꾸는 게 좋습니다. v2의 Emitter API는 `emit`, `emit_to`, `emit_filter`를 제공합니다. `emit`은 모든 target으로 보내고, `emit_to`는 특정 target, `emit_filter`는 조건에 맞는 target으로 보냅니다.

하지만 PTY output은 "작은 알림"이 아니라 **고빈도 스트리밍 데이터**입니다. Tauri 공식 문서는 event system이 low-latency/high-throughput 용도가 아니고, event payload가 JSON 문자열이라 큰 메시지에 적합하지 않으며, 스트리밍에는 **Channel**이 더 적합하다고 설명합니다.

**더 나은 패턴:**

```
PTY read thread
  -> bounded internal channel / broadcast hub
  -> per-window 또는 per-terminal subscription
  -> Tauri Channel로 frontend에 전달
```

즉 command를 이렇게 나누는 편이 좋습니다.

```
spawn_agent(cwd) -> agent_id
subscribe_agent_output(agent_id, Channel<PtyEvent>)
write_stdin(agent_id, data)
resize_pty(agent_id, cols, rows)
kill_agent(agent_id)
```

이렇게 하면 멀티 창 fanout도 명확해집니다. 각 window가 특정 agent에 subscribe하고, backend는 해당 agent의 subscriber 목록에만 전송합니다. "모든 창에 뿌리고 frontend에서 알아서 버려라" 방식보다 CPU, IPC, listener lifecycle 관리 면에서 낫습니다.

다만 `agent_status_changed`, `agent_list_updated` 같은 저빈도 상태 이벤트는 Tauri event로 유지해도 좋습니다.

---

## 3. 이벤트 이름 pty_output:{agent_id} 동적 채널 방식 vs 단일 채널 + payload 필터링

**결론: event를 쓴다면 단일 이벤트 + payload 필터링이 더 낫고, 최선은 Channel 기반 subscription입니다.**

`pty_output:{agent_id}` 자체는 문법상 가능합니다. Tauri event name은 alphanumeric, `-`, `/`, `:`, `_` 문자를 허용하므로 UUID의 hyphen과 colon 구분자는 문제가 되지 않습니다.

하지만 설계 관점에서는 동적 event name을 많이 만드는 방식보다 아래가 더 관리하기 쉽습니다.

```typescript
type PtyOutputEvent = {
  agentId: string;
  seq: number;
  data: string; // 또는 bytes/base64
};
```

event를 꼭 쓴다면:
- event name: `pty-output`
- payload: `{ agentId, seq, data }`

이유:
- listener 등록/해제가 단순합니다.
- 로그/디버깅/테스트가 쉽습니다.
- 동적 event name 누수 가능성이 줄어듭니다.
- 여러 agent를 한 UI에서 동시에 보여줄 때 aggregation이 쉽습니다.
- Tauri v2의 고처리량 권장 방향과도 더 잘 맞습니다.

**정리:**
- 최선: `subscribe_agent_output(agent_id, Channel<PtyEvent>)`
- 차선: 단일 event `"pty-output"` + `{ agentId, data }`
- 비권장: `"pty_output:{agent_id}"` 동적 event를 대량 생성

---

## 4. xterm.js - Tauri v2 연동 시 알려진 pitfall?

**가장 큰 pitfall은 backpressure입니다.** xterm.js의 `write()`는 non-blocking이고 내부 버퍼에 쌓은 뒤 다음 event loop에서 처리합니다. xterm.js 문서는 빠른 producer가 xterm.js를 압도하면 UI가 느려지거나 입력 반응이 떨어지고, 입력 버퍼가 커질 수 있으며, 버퍼 한도를 넘는 데이터가 버려질 수 있다고 설명합니다.

따라서 PTY reader에서 읽은 데이터를 즉시 무한 emit/send하지 말고, backend 또는 frontend 중 한 곳에서 반드시 batching/throttling을 둬야 합니다.

**권장 패턴:**
```
PTY reader
  -> 4KB~32KB chunk
  -> 8~16ms 단위 batch
  -> Channel send
  -> frontend queue
  -> term.write(chunk, callback)
```

**두 번째 pitfall은 resize 동기화입니다.** xterm.js 화면 크기만 바꾸고 PTY 크기를 안 바꾸면 shell/Claude Code 쪽은 여전히 옛 cols/rows로 생각합니다.

```
ResizeObserver
  -> debounce
  -> fitAddon.fit()
  -> invoke("resize_pty", { agentId, cols: term.cols, rows: term.rows })
```

**세 번째 pitfall은 listener cleanup입니다.** React/Vue/Svelte 컴포넌트가 unmount될 때 Tauri listener를 해제하지 않으면 같은 PTY output이 여러 번 찍히는 문제가 생깁니다.

**추가 주의점:**
- `term.onData()`에서 받은 입력은 그대로 PTY stdin으로 보내고, frontend에서 직접 echo하지 않는 편이 좋습니다.
- Windows ConPTY는 VT sequence가 섞여 나오므로, 출력 문자열을 "깨진 텍스트"로 오판하지 말고 xterm.js에 그대로 넘겨야 합니다.
- hidden tab/container 상태에서 `fit()`을 호출하면 cols/rows가 0 또는 이상한 값이 될 수 있으므로, terminal이 실제로 visible 된 뒤 fit 해야 합니다.
- 대량 paste는 한 번에 write_stdin 하지 말고 chunking하는 게 안전합니다.
- frontend에서 ANSI escape를 sanitize하려고 직접 파싱하지 말고 xterm.js에 맡기는 편이 낫습니다.

---

## 5. 이 구조에서 놓친 것, 더 개선할 설계 포인트?

### 5-1. PtySession에 child handle이 빠져 있습니다

현재 구조에는 `master`, `writer`는 있지만 child process handle이 없습니다. `portable-pty` 문서상 `Child`는 PTY 안에서 spawn된 child process를 나타내며 wait 또는 terminate에 사용할 수 있는 handle입니다. kill, exit status, zombie 방지, 정상 종료 감지를 하려면 session에 child handle 또는 killer handle을 보관해야 합니다.

**개선 예시:**

```rust
pub struct PtySession {
    pub id: AgentId,
    pub cwd: PathBuf,
    pub writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub master: Box<dyn MasterPty + Send>,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
    pub status: AgentStatus,
    pub subscribers: Vec<Subscriber>,
}
```

### 5-2. 전역 Mutex\<HashMap\>을 오래 잡으면 안 됩니다

`write_stdin`, `resize_pty`, `kill_agent`에서 `sessions.lock()`을 잡은 상태로 blocking I/O를 하면 다른 agent 작업까지 막힐 수 있습니다.

**권장 구조:**
- sessions lock은 agent lookup까지만
- 각 session 내부에 writer lock 별도 보유
- blocking I/O는 전역 manager lock 밖에서 수행

### 5-3. 상태 모델이 더 촘촘해야 합니다

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

특히 Claude Code는 인증, 권한 prompt, 네트워크 실패, cwd invalid, binary not found 같은 실패가 많을 수 있습니다. `Running/Stopped`만 있으면 UI에서 원인 표현이 어렵습니다.

### 5-4. replay buffer가 필요합니다

멀티 창 구조에서는 새 창이 agent에 나중에 attach할 수 있습니다. 그때 과거 output이 없으면 빈 터미널에 이후 출력만 보입니다.

각 agent마다 다음 중 하나를 두는 게 좋습니다:
- raw output ring buffer
- xterm serialize snapshot
- 최근 N KB output + seq number

최소 MVP로는 `VecDeque<PtyChunk>` 형태의 최근 output buffer를 두고, `subscribe_agent_output` 시점에 replay 후 live stream으로 전환하면 됩니다.

### 5-5. Claude Code 자체의 session 모델과 충돌하지 않게 해야 합니다

초안은 "에이전트 = Claude Code 세션 = PTY 프로세스 1개"로 정의했는데, 최신 Claude Code CLI 자체에도 `claude agents`, `claude attach`, `claude logs`, `claude daemon`, `claude --bg` 같은 background/session 관리 명령이 있습니다.

따라서 제품 방향을 둘 중 하나로 명확히 해야 합니다:

- **A안**: Engram Dashboard가 직접 PTY 프로세스를 관리한다. agent_id는 Engram 내부 UUID.
- **B안**: Claude Code의 background session/daemon 모델을 적극 활용한다. agent_id는 Claude Code session id와 매핑.

현재 초안은 A안입니다. A안도 가능하지만, Claude Code가 자체 background agent를 제공하는 상황에서는 나중에 "Engram agent"와 "Claude Code session"의 개념이 어긋날 수 있습니다.

### 5-6. 보안/권한 설계가 필요합니다

`spawn_agent(cwd: String)`은 위험한 진입점입니다. 최소한 다음을 해야 합니다:

- cwd canonicalize
- 허용 workspace root 밖이면 거부
- 실행 binary는 사용자가 임의 입력하지 못하게 고정
- environment allowlist/denylist
- 로그에 token/API key가 남지 않게 masking
- kill_agent/write_stdin 권한은 agent owner/window 기준으로 확인

---

## 최종 권장 구조

```
Tauri Commands
- spawn_agent(cwd) -> AgentInfo
- subscribe_agent_output(agent_id, on_event: Channel<PtyEvent>)
- write_stdin(agent_id, data)
- resize_pty(agent_id, cols, rows)
- kill_agent(agent_id)
- get_agents()
- get_agent_snapshot(agent_id)

Tauri Events
- agent-status-changed
- agent-list-updated

Internal
- PtyManager
  - sessions: Mutex<HashMap<AgentId, Arc<PtySession>>>
- PtySession
  - writer lock
  - child handle
  - status
  - subscribers
  - replay ring buffer
  - seq counter
```

**판정:** PTY 선택과 큰 방향은 맞지만 **`emit_all` 기반 스트리밍만은 실전 전에 바꾸는 게 좋습니다.** 상태 이벤트는 Tauri event, PTY output은 Channel 또는 subscription hub로 분리하는 구조가 가장 안전합니다.
