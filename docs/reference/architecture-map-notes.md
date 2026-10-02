# Engram 아키텍처 지도 — AI용 원자료

> **독자 = 세션(모델).** 사람용 정제본 = [`architecture-map.html`](architecture-map.html) — 장 번호를 공유한다.
>
> **기준 = master `a226f63` · 2026-09-26.** 아래 `file:line`은 이 커밋 기준이다. 코드가 움직여 어긋나면 `git show a226f63:<경로>`로 연다.
>
> **갱신 규칙(임시):** 이 파일을 고치면 같은 장의 HTML도 같은 턴에 고친다. 이 규칙의 적립(문서 규약·ADR-0197과의 관계)은 사용자 결정 대기다. ADR-0197은 「HTML = 그 시점 스냅숏, 갱신 안 함 · 그림 소스 = mermaid 하나」를 정했는데 이 문서 쌍은 둘 다 따르지 않는다(사용자 지시 「임시로 작업해봐」, 2026-09-26).
>
> **사람용 형식 — 사용자가 통과시킨 모양(2026-09-26):** 「적당히 정보를 선별해서 잘된것같음. 이미지 큰거 한장하고 단순하게 폴더구조 설명한게」. 장마다 **큰 그림 한 장 + 단순한 구조 표 + 짧은 관측 목록**을 기본으로 하고(★**다크 모드 전용** — 사용자 2026-09-26: 「일단 다크 나중에 라이트 필요하면 말할게」★), 세부(줄 수·코드 위치·간선)는 이 파일에 둔다. 룰 적립 때 재료. ★**사람용 = 도메인과 관계를 보여 주는 그림 한 장이면 된다 — 모듈 막대 · 의존 행렬 · 분리·합치기 후보 목록은 사람용에 넣지 말 것**★(사용자 2026-09-26: 「그냥 도메인이 어떤것들이 있고 어떤관계로 되는지 그림으로 보여주면 되는데 이건 내가 못볼 문서잖아」 — 그 판은 세부를 사람용에 다 올렸다가 되돌렸다). HTML 장 번호는 이 파일 장 번호와 더는 1:1이 아니다(HTML 1장 = 프로세스 · 2장 = 도메인과 관계).
>
> **원자료 파일:** 파일·줄 범위별 도메인 태그 = [`architecture-map-data/tags.json`](architecture-map-data/tags.json) · Rust 간선 = `architecture-map-data/edges-rust.json` · 프론트 간선 = `architecture-map-data/edges-front.json` · HTML 그림 생성기 = `architecture-map-data/tools/`(`gen_bars.py`는 tags.json에서 막대를 다시 굽고, `gen_graph.py`의 숫자는 3장에서 손으로 옮긴 것). 코드가 움직이면 태그부터 다시 매긴다.
>
> **옛 `architecture-overview.md`는 참고하지 않는다**(사용자: 「만들어놓고 안봤는데 내가 볼 용도는 아님」). 처리는 **보류**(사용자 2026-09-26: 「일단 건들지마. 나중에 정리하자」). 정리할 때 걸리는 것: 그 문서의 「세션 복원 / 활성화」 절은 ADR-0186이 정본으로 지정했고, 살아 있는 인용 7곳(`docs/README.md` 2 · `README.md` 1 · `reference/structure/agent-backend.md` 2 · `agent/src/manager.rs:2370` 주석 1 · `docs/todo/` 2 — 그중 정본 인용은 agent-backend·manager 둘)이 있다. 세션이 제안한 안 = 그 절만 `reference/structure/`로 옮기고 나머지 삭제(미승인).

## 1장 조감도 원자료

**수집:** 서브에이전트 4명이 읽기 전용으로 모았다(프로세스·통신선 / 셸·데몬 내부 / 하위 crate / 프론트). **메인이 직접 대조한 주장**(전부 일치): 셸→`net` = `AuthFrame` 1개 · 셸→`agent` = 기호 3개 · discovery→`net` = `AuthFrame` 1개 · `bin/engram.rs`의 데몬 lib 참조 0 · `protocol/src/domain.rs` 머리글의 미러 선언 · `connection_core::sanitize_*` 사용처. 나머지는 수집자 보고의 확신도 태그를 그대로 따른다.

### 1.0 도메인 아홉 개 — 기능 기준(사용자 합의 2026-09-26)

- **키(태깅 JSON과 같음):** `AGENT` 에이전트 · `MAIL` 우편 · `CMD` 커맨드 · `AUTH` 인증 · `COMM` 통신 · `DAEMON_LIFE` 데몬 수명 · `SCREEN` 화면 · `RENDER` 렌더링(프론트만) · `BASE` 바닥 · (`ROOT` 조립 — 도메인 아님).
- **태깅 규칙:**
  - 통신 = 순수 배관만(프레이밍 · 코덱 · 단일 writer · keepalive · 재연결 · 요청/응답 짝짓기 · 프레임 팬아웃 · 프레임 포트 · IPC 전달 · HTTP 서버 기계부). **받은 메시지를 해석해 기능을 부르는 코드는 그 기능**(= 기능 쪽 어댑터).
  - 기능별 명령 선언(`layout/commands.rs` · `agent/src/commands.rs` · 프론트 `*Commands.ts`)은 그 기능. 버스 자체만 커맨드.
  - 섞인 파일 = 두 도메인 이상이 각각 대략 15% 또는 80줄(프론트 40줄)을 넘음 → 함수·impl 묶음 단위 줄 범위로 분할(경계 ±10~20줄).
- **폐기한 첫 초안(같은 날):** 렌더링 · 화면 배치 · 명령 · 연결 · 에이전트 실행 · 메시징 + 바닥. 기능 기준(에이전트 실행·메시징·화면 배치·렌더링)과 기술 층 기준(연결·명령)이 섞여, 받은 요청을 기능에 넘기는 어댑터 코드가 전부 「연결」로 모이면서 연결이 모든 도메인을 부르는 허브처럼 보였다(착시). 사용자 지적: 「도메인 단위가 명령, 연결? 어떤 기준의 도메인이야? 데몬 기준으로 우편시스템, 에이전트 시스템, 로깅, 커맨드, 등등이 큰 카테고리 아님?」 + 「인증도 있고」 → 기능 기준 재태깅. 초안 기준 의존 원자료 = 부록 A.
- **방법(사용자 합의):** 도메인 선 → 모듈마다 도메인 표시 → 한 모듈에 여러 도메인 = 분리 후보 · 한 도메인이 여러 모듈/crate = 합치기 후보 → 나눈 뒤 의존 방향은 도메인 연관관계로.

### 1.1 프로세스와 통신선

#### 도식

```
 User
  │ launches (scripts/run-*.bat / installer)
  ▼
┌──────────────────────────── engram-dashboard.exe (Tauri shell, src-tauri) ─────────────────────────┐
│  WebView2 windows: main (static) · agent-tree (static, hidden, #/tree) · popout (runtime-created)  │
│     ▲ invoke (tauri::command, 40+ cmds)      ▼ app.emit / emit_to (events)                         │
│     ▲ subscribe_output → ipc::Channel<Response> per window label (binary output frames)            │
│  Rust side: daemon_client (WS client) · layout (in-memory) · output_router · tray                  │
└──────┬──────────────────────────────────────────────┬──────────────────────────────────────────────┘
       │ WMI Win32_Process.Create (discovery::ensure_daemon)  │ WebSocket ws://127.0.0.1:<port> (token auth)
       │ + reads daemon.json (port/token/pid)                 │ text JSON = AgentCommand/AgentEvent; binary = codec frames
       ▼                                                      ▼
┌──────────────────── engram-dashboard-daemon.exe (owns AgentManager) ─────────────────────┐
│  Listener A: TcpListener 127.0.0.1:0 → WS (net crate) ── shell clients                   │
│  Listener B: TcpListener 127.0.0.1:0 → axum HTTP: /mcp (rmcp streamable HTTP) + /control/*│
│  Writes daemon.json (single-instance lock = same file), agents.json, presets.json, logs,  │
│  mcp-config/<id>-<epoch>.json|.settings.json                                              │
└───┬─────────────────────────────┬────────────────────────────────┬───────────────────────┘
    │ ConPTY (portable-pty)       │ stdio pipes NDJSON (양방향)    │ stdio pipes bidi JSON-RPC
    │ claude / codex TUI / shell  │ claude --stream-json           │ codex app-server --stdio
    │ / gemini                    │                                │
    ▼  (all children in a KILL_ON_JOB_CLOSE Job Object)            ▼
┌──────────── agent CLI processes ────────────┐
│ env: ENGRAM_TOKEN, ENGRAM_CONTROL_URL, ENGRAM_EXE; claude also gets --mcp-config/--settings files;
│ codex gets -c mcp_servers.engram={url,bearer env}                                            │
└──┬──────────────────────────────┬───────────┘
   │ MCP over HTTP (Bearer)       │ agent shells out to `engram` CLI
   │ → daemon Listener B /mcp     ▼
   │                        ┌──── engram.exe (control CLI) ────┐
   │                        │ HTTP POST <base>/control/<route> │ → daemon Listener B (Bearer ENGRAM_TOKEN)
   └────────────────────────┴──────────────────────────────────┘
Side: daemon reads ~/.claude/{sessions,projects/*.jsonl}, ~/.codex (thread-writer-locks) + Restart Manager.
```

#### 실행 파일

| 바이너리 | crate / 경로 | 용도 | 링크하는 워크스페이스 crate | 확신 |
|---|---|---|---|---|
| `engram-dashboard.exe` | src-tauri, `src-tauri/Cargo.toml:32-34` (lib `engram_dashboard_lib`) | Tauri 클라이언트 셸: 창·트레이·discovery·데몬 WS 클라이언트·인메모리 레이아웃 | base, agent, command, protocol, discovery, net (`src-tauri/Cargo.toml:61-86`). daemon crate 는 **dev-dep 전용**(`:100-104`) | 확실 |
| `engram-dashboard-daemon.exe` | `crates/engram-dashboard-daemon/src/main.rs` (`Cargo.toml:29-31`) | 에이전트 호스트. `lib::run()` 만 부르는 얇은 진입점. 릴리즈 = windows 서브시스템, 디버그 = 콘솔 앱(`main.rs:9`) | agent, base, protocol, discovery, messaging, net[server], command (`daemon/Cargo.toml:79-98`) | 확실 |
| `engram.exe` | `daemon/src/bin/engram.rs` (`Cargo.toml:37-39`) | 스폰된 에이전트가 셸에서 부르는 제어 평면 CLI(`engram mail…`, `agent…`, `<full.name> --k v`). bin 이름 = agent `CLI_EXE_NAME` 와 일치해야 함 | 데몬과 같은 패키지. 헤더상 std TcpStream + 손조립 HTTP. 실제 crate 사용 범위는 미확인 | 가능성 높음 |
| `saturation-pilot`, `priming-smoke`, `roundtrip-smoke`, `drain-latency` | `daemon/src/bin/*` (`Cargo.toml:46-76`) | 실험·측정 드라이버. `required-features = ["test-harness"]` 라 릴리즈 빌드는 컴파일하지 않음 | daemon lib | 확실 |
| (없음) | `engram-dashboard-transport` | 새 재사용 전송 lib(ADR-0177). 이 crate 를 의존하는 Cargo.toml 이 워크스페이스에 아직 없음 | — | 확실(의존자 미발견) |

#### 통신선

| From → To | 메커니즘 | 흐르는 것 | 타입 소재 | 포인터 | 확신 |
|---|---|---|---|---|---|
| WebView → 셸 | Tauri `invoke` | 데몬 수명(discover/start/stop/ensure/connect/close), `forward_daemon_command`, 레이아웃(tab/window/slot/split/assign/spawn_into/move_slot_to_window), agent_spawn/kill/interrupt/write_stdin/resize, set_envelope_format, subscribe_output, request_replay, report_view_commands, report_command_outcome, UI 설정·메트릭, autostart, show/hide/quit | 셸 command + protocol ts-rs 바인딩 | `src-tauri/src/lib.rs:210-256` | 확실 |
| 셸 → WebView (출력) | `tauri::ipc::Channel<Response>`, 창 label 당 하나(`subscribe_output` 로 등록). replay 는 `request_replay` 로 따로 유발(ADR-0046) | 에이전트 출력 프레임(binary) | — | `src-tauri/src/output_channel.rs:24`, `commands/agent.rs:156-165` | 확실 |
| 셸 → WebView (상태) | `app.emit` / `emit_to` | `agent-list-updated`, `status-changed`, `restore-result`, `profile-list-updated`, `preset-list-updated`, `daemon-connection-state`, `daemon-status-changed`, `layout:updated`, `window:tabs-updated`, `ui:settings-updated`, `command:request` | protocol domain 타입 | `daemon_client/events.rs:107-139`, `commands/layout.rs:29-59`, `commands/settings.rs:28,124`, `view_commands.rs:50,179`, `tray/actions.rs:151` | 확실 |
| 셸 → WebView popout | 런타임 `WebviewWindowBuilder`(route `/popup`) | 창 생성 | — | `commands/popout.rs:112` | 확실 |
| 셸 → 데몬 (기동) | `discovery::ensure_daemon`: daemon.json 읽기 → 생존 판정(pid + 시작시각) → 아니면 WMI `Win32_Process.Create` 후 폴링. WMI 는 env 주입 불가라 토큰은 daemon.json 으로만 전달 | spawn | protocol `DaemonInfo`(`protocol/src/discovery.rs:19`) | `discovery/src/lib.rs:932,1105-1153`. 호출부: `src-tauri/src/daemon_client/mod.rs:95`, `tray/mod.rs:206`, `commands/discovery.rs:191` | 확실 |
| 셸 ↔ 데몬 | WebSocket(tokio-tungstenite `connect_async`) → 127.0.0.1:port, Auth→Hello 핸드셰이크, 백오프 재연결 | text JSON: `AgentCommand`(Spawn/Kill/Interrupt/WriteStdin/Resize/Subscribe/…Profile/Preset/Snapshot/StopDaemon/RegisterCommands/UpdateCommands/ListCommands/CommandOutcome) · `AgentEvent`(Hello/Ack/SubscribeAck/Output/ReplayComplete/StatusChanged/AgentListUpdated/…/CommandRequest/CommandReply/Error). binary: codec 프레임 `[tag:1][agent_id:16][epoch:4 BE][seq:8 BE][payload]`, tag 0 = VT 바이트, tag 1 = 구조화 이벤트 | `engram-dashboard-protocol`(`messages.rs:42,362`, `codec.rs:3-14`) | `src-tauri/src/daemon_client/connection.rs:39,410,503`, `daemon/src/agent_conn.rs:1-11` | 확실 |
| 데몬 → 셸 → WebView (명령 버스) | 데몬 WS `CommandRequest` → 셸 `emit_to("command:request")` → 프론트 `window.__engramCmd` → `report_command_outcome` → WS `CommandOutcome` | UI 소유 명령(window/theme…), LLM/CLI 가 호출 | protocol + command crate | `view_commands.rs:179`, `messages.rs:275-333,537-580` | 가능성 높음(라인 단위 전 경로 추적 안 함) |
| 데몬 → 에이전트 | ConPTY, portable-pty `CommandBuilder` + `spawn_command` | VT 바이트 양방향, resize | agent `transport/pty.rs` | `agent/src/transport/pty.rs:83,129`. `TransportShape` 은 backend 가 고름(기본 Pty, `backend/mod.rs:387-391,830-843`) | 확실 |
| 데몬 → claude (stream-json 모드) | stdio 파이프 NDJSON(`StdioNdjson`) | JSON 이벤트 출력, 인코딩된 입력 | agent transport/stdio | `transport/stdio.rs:79`, `backend/claude/mod.rs:372-377` | 확실 |
| 데몬 ↔ codex (app-server 모드) | stdio 양방향 JSON-RPC(`codex app-server --stdio`, `StdioBidiJson`) | thread/start, turn/start, turn/completed, codex 쪽 요청 | agent backend/codex | `backend/codex/transport.rs:598-617`, `codex/mod.rs:461-462,1016-1021` | 확실 |
| 에이전트 → 데몬 | MCP streamable HTTP `http://127.0.0.1:<portB>/mcp`, bearer 인증 미들웨어 | 우편 툴(eg_send 등) | daemon control/mcp_server | `daemon/src/control/mcp_server.rs:1252-1320`. codex 부착: `codex/mod.rs:434-455` | 확실 |
| 에이전트 셸 → engram.exe → 데몬 | 프로세스 exec(PATH/ENGRAM_EXE) 후 HTTP POST `<ENGRAM_CONTROL_URL>/control/{send,messages,agent,commands,call}` + `Bearer ENGRAM_TOKEN` | 우편·에이전트 제어·전체 이름 명령 호출 | JSON. 응답 shape 은 S18 spec §6(파일 헤더 기준) | `bin/engram.rs:69-70,159-160,450-461,678`. env 주입: `agent/src/backend/mod.rs:90-102`. 라우트: `mcp_server.rs:1303-1325` | 확실 |
| 셸 → 데몬 (종료) | WS `StopDaemon`, 또는 daemon.json pid kill(discovery). 자식은 Job Object 로 함께 종료 | — | — | `discovery/src/lib.rs:688-747` | 가능성 높음 |

#### 디스크 상태

data_dir: 디버그 = repo 루트 `.engram-data`, 릴리즈 = exe 경로 기준, env override 있음(`discovery/src/lib.rs:84-110`).

| 파일 | 소유(쓰는 쪽) | 읽는 쪽 | 포인터 | 확신 |
|---|---|---|---|---|
| `daemon.json`(pid, port, WS token, 시작시각, 프로토콜 버전) | 데몬. 단일 인스턴스 잠금으로 열어 둔 채 유지(ADR-0135), bind 후 기록 | 셸(discovery 경유). 클라이언트는 지우지 않음(ADR-0134) | `net/src/portfile.rs:33`, `net/src/instance.rs`, `daemon/src/lib.rs:448-490`, `discovery/src/lib.rs:21,659` | 확실 |
| `agents.json`(+`.tmp`, 손상 시 백업) | 데몬(agent `persistence`, tmp+rename 원자 쓰기) | 데몬 부팅 | `agent/src/persistence/mod.rs:22-23,69-90` | 확실 |
| `presets.json` | 데몬 | 데몬 | `agent/src/persistence/presets.rs:19-20` | 확실 |
| `mcp-config/<id>-<epoch>.json`, `.settings.json`(`allowedMcpServers:[engram]`) | 데몬. provision 때 생성, revoke 때 삭제, 부팅 스윕 | claude CLI(`--mcp-config` / `--settings`) | `daemon/src/control/mcp_config.rs:31-51,92,116,147,186`, `lib.rs`(스윕 호출) | 확실 |
| `logs/<daemon\|app>-*.log`(종류당 10개 보존, 폴백 `%TEMP%/engram-dashboard/logs`) | 데몬·셸 각자 자기 파일 | 사람 | `base/src/logging/mod.rs:80-96`, `daemon/src/lib.rs:411` | 확실 |
| `ui-settings.json` | **외부 에이전트가 편집**. 셸은 읽고, 죽은 창 항목을 지우는 부팅 스윕에서만 씀(ADR-0167) | 셸 | `src-tauri/src/ui_settings.rs:1-20,48,705-711` | 확실 |
| 레이아웃·탭·창 | 없음. 인메모리 전용(셸 재시작 시 초기화) | — | CLAUDE.md §5, 코드 재확인 안 함 | 가능성 높음 |
| 프로필(`AgentProfile`) | agents.json 안으로 추정(미확인) | — | — | 불확실 |

#### 주경로 밖 사이드 채널

- 데몬이 claude CLI 자체 파일을 읽음: `~/.claude/sessions`(세션 파일 추적), `~/.claude/projects/<slug>/<sid>.jsonl`(트랜스크립트·resume 이벤트). `backend/claude/session_file.rs:79-80,147`, `backend/claude/mod.rs:999-1014`. 확실.
- 데몬이 codex home(`CODEX_HOME` 또는 `~/.codex`) → `thread-writer-locks` 를 읽고, 파일을 쥔 프로세스를 Windows Restart Manager 로 조회해 codex 세션 id 를 회수(ADR-0218). `backend/codex/thread_lock.rs:39-43,217-232`, agent Cargo.toml 의 windows feature `Win32_System_RestartManager`. 가능성 높음(RM 호출부는 안 열어 봄).
- 외부 에이전트가 `ui-settings.json` 을 씀. 셸은 `ui.refresh` 때 반영하고, 파일 watcher 는 발견 못 함. 가능성 높음.
- 데몬 → 자식 env: `ENGRAM_TOKEN`, `ENGRAM_CONTROL_URL`, `ENGRAM_EXE`(데몬 부팅 때 설정, `daemon/src/lib.rs:95-102,420`). 확실.
- 데몬 자기 시작시각을 daemon.json 에 기록하고, discovery 는 PID + 시작시각으로 생존 판정(`base::platform`). 확실.

#### 모름

- `engram.exe` 를 실제로 실행하는 프로세스는 에이전트의 셸. 바이너리 해석 경로(PATH vs `ENGRAM_EXE`)는 추적 안 함.
- gemini·shell backend 가 PTY 외 모양을 쓰는 경우가 있는지 확인 안 함(기본값은 Pty).
- 프론트가 운영에서 데몬 WS 에 직접 붙는지: CLAUDE.md 는 `WsTransport` = 테스트 전용, 운영 = `TauriTransport` 라 함. `src/` 에서 재확인 안 함.
- 프로필 저장 위치.
- 셸이 연결마다 데몬에 명령 표를 등록(`RegisterCommands`)하는지: 가능성 높음, 추적 안 함.
- `engram-dashboard-transport` 는 아직 소비자가 없음.

### 1.2 셸(src-tauri)과 데몬 crate 내부

**핵심 5개**
1. **셸 → `net` 간선은 타입 하나뿐이다.** `engram_dashboard_net::auth::AuthFrame`을 `src-tauri/src/daemon_client/connection.rs:24`에서만 쓰고, 첫 핸드셰이크 프레임을 직렬화하는 데 쓴다(`:387`). 그 밖엔 테스트 사본 `daemon_client/tests.rs:45`뿐이다. net feature는 켜지 않는다(`src-tauri/Cargo.toml:74-88`). [확실]
2. **셸 → `agent` 간선은 3개 파일의 심볼 3개다. 에이전트 호스팅이 아니다.** [확실]
   - `agent::commands::COMMAND_SPECS` — `view_commands.rs:204`. `reserved_names()`가 데몬 명령 이름과 셸 `layout::commands::COMMAND_SPECS`를 합쳐 웹뷰가 겹치는 이름을 등록하지 못하게 한다.
   - `agent::commands::llm_creation_refusal` — `layout/apply.rs:36,521`. `gate_backend`가 `spawn_into`에 LLM 백엔드 생성 정책을 적용한다.
   - `agent::commands::normalize_cwd` — `layout/commands.rs:35,844`. `verb_spawn_into`가 cwd 철자를 `agent.new`와 맞춘다.
   - `src-tauri/Cargo.toml:62` 주석은 "`COMMAND_SPECS` 하나만 남았다"고 하지만 **낡았다 — 실제로는 3개다.** [확실]
   - 테스트는 `AgentNewArgs`·`LLM_BACKEND_POLICY`를 더 쓴다(`tests/layout_apply.rs:37`). dev-dep `engram-dashboard-daemon`도 테스트 빌드에 `agent`를 끌고 온다(`Cargo.toml:66,102`).
3. **데몬 내부 순환.** 테스트 외 코드에서 `connection_core`·`command_delivery`·`command_roster`·`control`이 서로 얽힌 한 덩어리다. [확실]
   - 상당 부분은 유틸 쏠림이다. `sanitize_for_log`·`sanitize_within`·`event_json`이 `connection_core.rs:820,1913,1919`에 있고, `command_roster:42`·`command_delivery:108`·`control/mcp_server:41`·`control/registry:246`·`control/catalog:357`이 이를 import한다.
4. **메시징 호스트 코드가 데몬 여러 모듈에 흩어져 있다.** [확실]
   - `messaging_host.rs`는 스스로 "호스트 어댑터 + 조립실"이라 하지만 `MessagingSlot`은 `control::mcp_server`에 정의돼 있다.
   - `MessagingSlot`을 `connection_core:856`·`agent_conn:443`·`messaging_host:730-857`이 쓴다.
   - `control/ingress.rs`에 `engram_dashboard_messaging` 참조 14개, `connection_core`에 7개.
5. **셸 순환: `commands` ↔ `tray`.** `commands/tray.rs:6`이 `crate::tray::actions`를, `tray/actions.rs:19`·`tray/core.rs:218`이 `crate::commands::popout::is_popup_label`을 쓴다. 팝업 label 소유가 Tauri 어댑터 층(`commands/popout`)에 있고 `tray`와 `lib.rs`의 `on_window_event`가 그것을 소비한다. [확실]

---

#### 셸 `src-tauri` (lib `engram_dashboard_lib`)

##### 1. 모듈 인벤토리
줄 수 = "비테스트 / 테스트". 테스트 = `mod tests` 줄부터 파일 끝, 또는 별도 `tests.rs`.

| 모듈 | 파일 | 줄 | 책임 |
|---|---|---|---|
| `lib.rs` | 1 | 266/0 | 조립 루트: 플러그인, `setup`(로깅·managed state·`DaemonClient`·명령 표 설치·트레이), `on_window_event` 팝업 정리, invoke 명령 47개의 `generate_handler!` |
| `main.rs` | 1 | 8 | `run()` 호출 |
| `commands/` | 9 | ~1613/~59 | `#[tauri::command]` 47개 **전부**: agent 9, autostart 2, discovery 9, layout 20, popout 1, settings 1, tray 3, view_bus 2. 얇은 Tauri 어댑터. `popout.rs`는 런타임 창 생성·파괴(OS 창 포트 어댑터)도 한다. `layout.rs`는 `command_ports`(레이아웃 포트의 Tauri 어댑터)도 만들고 `AgentCommand::SpawnByCwd`를 보낸다(`:96`) |
| `daemon_client/` | 8 | ~4055/~5265 | 데몬 WS 연결 1개. `mod.rs` 925 = `DaemonClient` API. `connection.rs` 1822 = 액터·핸드셰이크·재연결. `lifecycle.rs` 367 = generation 락. `protocol_state.rs` 133/318 = epoch 가드·pending 매칭. `replay_flight.rs` 403/646 = 순수 replay 상태기계. `inbound.rs` 253 = 데몬이 셸로 배달한 명령 수신기(`ViewCommandPort` trait). `events.rs` 152 = emit seam. `tests.rs` 4289 |
| `layout/` | 8 | 3545/3273 | 레이아웃 SSOT(ADR-0035): `manager.rs` 813/1792(`ViewManager`), `apply.rs` 837(전송 중립 적용 서비스 + 포트), `commands.rs` 945(`window`/`tab`/`slot` 버스 명령 선언과 본문), 순수 모듈 `tree` 283/478·`spatial` 236/567·`geometry` 185/436·`types` 203·`mod` 43. `layout/`에 `tauri` import 없음, grep 확인 [확실] |
| `output_router.rs` | 1 | 242/664 | `ViewManager`에서 파생한 agent_id → 창 label 라우팅 표(순수) |
| `output_channel.rs` | 1 | 66 | 창별 `Channel` 레지스트리와 send 헬퍼 |
| `ui_settings.rs` | 1 | 780/0 | `ui-settings.json` 읽기, 창별 설정 배달, 부팅 때 죽은 창 항목 정리(순수, 파일 I/O는 주입) |
| `view_commands.rs` | 1 | 802/0 | 웹뷰 몫 명령의 다리: 등록 패킷·예약 이름 필터·답장 상관. `ViewCommandPort` 구현, `tauri` import |
| `tray/` | 3 | 634/212 | 트레이: `core`(순수), `actions`(show/hide/quit 부수효과), `mod`(Tauri 배선·데몬 관찰자) |

`src-tauri/tests/`는 통합 타깃 4개(6230줄). `lib_unit`은 `tests/`에 파일이 없고 `src/**` 단위 테스트를 도는 `[[test]]` 타깃이다 [가능성 높음].

##### 2. 내부 간선 (주석 제외 `crate::` 사용)
- **`lib`** → 전부.
- **`commands`** → `daemon_client`, `layout`, `ui_settings`, `output_router`, `output_channel`, `view_commands`, `tray`, `discovery`(재수출).
- **`daemon_client`** → `output_router`, `output_channel`. `commands`·`layout`·`view_commands` 참조는 doc 주석뿐이라 `inbound`는 자기 포트 trait에 의존한다.
- **`view_commands`** → `daemon_client::inbound::ViewCommandPort`, `layout`.
- **`output_router`** → `layout::manager`, `layout::types`.
- **`output_channel`** → `output_router::WindowLabel`.
- **`layout`** → `ui_settings`(`UiSettingsRefresh`, `ThemeSource` — `layout/commands.rs:46,811`).
- **`tray`** → `commands::popout`, `layout::MAIN_WINDOW_LABEL`, `discovery`.
- **`ui_settings`** → 없음.
- **순환:** `commands` ↔ `tray` 하나뿐 [확실].

##### 3. crate 간 사용 (비테스트 `src`)
- **`protocol`** — 13개 파일:
  - 심볼: `AgentCommand`(`Subscribe`·`Unsubscribe` 포함), `AgentEvent`, `AgentId`, `RequestId`, `DaemonInfo`, `AgentBackendKind`, `PROTOCOL_VERSION`, `SubscribeAction`, `command_request_id`.
  - 주로 `daemon_client/*`와 `commands/{agent,discovery,layout,view_bus}`. 그 밖에 `layout/apply.rs:37`, `layout/commands.rs`.
- **`command`** — `view_commands.rs`, `layout/commands.rs`, `daemon_client/{connection,inbound,mod}`, `commands/view_bus`:
  - 심볼: `declare_commands!`, `blocking_handler`, `CommandTable`, `CommandHandler`, `CommandFuture`, `CommandError`, `ErrorCode`, `CommandReply`, `CommandEnvelope`, `CommandDecl`, `OwnerToken`, `RequestId`, `route`, `spec_item_json`.
  - 이유: 셸이 `window`/`tab`/`slot` 명령을 선언·소유하고(ADR-0155) 데몬이 배달한 봉투를 라우팅한다.
- **`discovery`** — `crate::discovery`로 재수출(`lib.rs:8`):
  - `daemon_client/mod.rs`에서 `ensure_daemon`·`read_live_daemon`·`default_data_dir`.
  - `commands/discovery.rs`에서 `locate_daemon_exe`.
  - `tray/`에서 참조 11개.
  - `ui_settings`·`lib.rs`에서 `default_data_dir`.
- **`base`**: `logging`(`lib.rs:18`에서 초기화), `logging::mask_secrets`(`ui_settings.rs`).
- **`agent`**·**`net`**: 핵심 1~2 참조.

##### 4. 책임 태그
- **조립 루트:** `lib`
- **Tauri IPC 어댑터 층 / 제어 표면:** `commands`
- **데몬 연결(전송·핸드셰이크·재연결·replay):** `daemon_client`
- **레이아웃·창·탭 모델:** `layout`. `apply`/`commands`를 통해 에이전트 스폰 정책도 일부 진다
- **창으로의 출력 팬아웃:** `output_router`, `output_channel`
- **영속(읽기 전용 UI 설정):** `ui_settings`
- **명령 버스 배선:**
  - 웹뷰 쪽: `view_commands`, `commands/view_bus`
  - 셸 쪽: `layout/commands`, `daemon_client/inbound`
- **OS 통합:** `tray`, `commands/autostart`, `commands/popout`(창 생성)

##### 5. 핫스팟
- **`layout`이 에이전트 생성 정책을 진다.**
  - `layout/apply.rs:512-525`: `gate_backend`가 `llm_creation_refusal`을 쓴다.
  - `layout/commands.rs:837-844`: `verb_spawn_into`가 `normalize_cwd`를 쓴다.
  - `commands/layout.rs:96`: `AgentCommand::SpawnByCwd`를 만든다.
  - 즉 "슬롯에 스폰"이 레이아웃·Tauri 어댑터·agent crate를 가로지른다.
- **명령 버스 배관이 4개 모듈에 흩어져 있다:** `layout/commands`(선언), `view_commands`(웹뷰 다리, tauri 결합), `daemon_client/inbound`(수신기), `commands/view_bus`(어댑터). `lib.rs:78-93`이 이들을 엮는다.
- **팝업·창 수명이 흩어져 있다.**
  - label 발급·창 빌드·정리는 `commands/popout`(어댑터 층)에 있다.
  - `lib.rs` `on_window_event`가 state 4개를 들고 `cleanup_popup_window`를 부른다.
  - `tray`가 `is_popup_label`에 의존한다.
  - 포트 추상은 `layout/apply`가 소유한다.
- **`daemon_client`가 창 라우팅을 안다.** `connection.rs:49-50`이 `OutputRouter`·`WindowChannelRegistry`를 import해, 연결 액터가 출력을 창으로 팬아웃하는 일까지 한다.
- **`commands/`가 서로 무관한 8개 영역을 묶는다.** 평평한 `pub use *` 재수출 하나로 노출한다(`commands/mod.rs:11-18`).

---

#### 데몬 `crates/engram-dashboard-daemon`

##### 1. 모듈 인벤토리 (비테스트 / 테스트)

| 모듈 | 줄 | 책임 |
|---|---|---|
| `lib.rs` | 978/58 | 조립: `run()`(`:403`) — data dir, 로깅, `build_daemon_wiring`, accept loop(`:307`), panic hook, 토큰 생성, `engram`/send exe 찾기. `start_test_server*`(`:791-833`)도 여기 있다 — cfg 게이트 없음, 셸 dev-dep이 쓴다 |
| `main.rs` | 16 | `run()` 호출 |
| `connection_core.rs` | 2032/3004 | 전송 중립 dispatch(`ConnectionCore`, `ConnectionSession`, `MultiViewState`). core→wire 변환기(`:544-820`), 로그 sanitizer(`:1913-1919`), `RosterFanout` 리스 브로드캐스트(`:1935-1960`), `hello`/`agent_list` 이벤트도 여기 있다 |
| `command_delivery.rs` | 2385/2767 | 명령 버스 배달(`CommandDeliveries`, `CommandBus`, `deliver`, `LocalCommands`, sweeper) |
| `command_roster.rs` | 452/610 | 공유 명령 주인 명부 |
| `agent_conn.rs` | 517/653 | 에이전트 시스템을 net 프레임 포트에 꽂는 어댑터(`FrameOutboundSink`) |
| `status_fanout.rs` | 78/213 | 상태 → wire 팬아웃(`DaemonStatusSink`) |
| `messaging_host.rs` | 891/772 | 메시징 커널의 호스트 어댑터 + 조립 |
| `control/`(9개 파일) | 4202/3439 | 스폰 에이전트용 제어 평면: `mcp_server` 1359(MCP HTTP), `ingress` 617(MCP/CLI 듀얼 파이프라인), `catalog` 465, `registry` 392(bearer 토큰), `priming` 361, `mod` 307(`DaemonControlChannel`), `agent` 289, `mcp_config` 221(에이전트별 스폰 파일), `commands` 191(데몬 로컬 명령 표) |
| `experiment/`(6개 파일) | 1371/987 | 포화 파일럿 순수 로직. `feature="test-harness"`에서만 컴파일(`lib.rs:17`) |
| `log_capture.rs`, `test_doubles.rs` | 103, 79 | `#[cfg(test)]` 전용 |
| `bin/engram.rs` | 2791/2655 | `engram` 제어 평면 CLI. **데몬 lib을 쓰지 않는다(참조 0).** `agent::types`(`:117`)·`command::RequestId`(`:177`)·`discovery::find_install_root`(`:329`)만 쓴다 |
| `bin/{saturation_pilot 1746, roundtrip_smoke 1528, drain_latency 666, priming_smoke 393}` | — | 실험·스모크 드라이버. `required-features = ["test-harness"]`(`Cargo.toml:46-76`) |

`tests/`: 파일 7개, 12001줄.

##### 2. 내부 간선 (비테스트·주석 제외)
- **`agent_conn`** → `command_delivery`, `command_roster`, `connection_core`, `control`(`registry`, `mcp_server::MessagingSlot`), `status_fanout`.
- **`connection_core`** → `command_delivery`(`:64,1653`), `command_roster`(`:65`), `control`(`registry :66`, `MessagingSlot :856`, `catalog::merge :1621`, `agent::RosterBroadcast :1960`).
- **`command_delivery`** → `command_roster`, `connection_core`(`event_json`, `sanitize_*`).
- **`command_roster`** → `connection_core`(`sanitize_for_log`).
- **`control`** → `command_delivery`(`LocalCommands`, `CommandBus`, `ENTRANCE_CONTROL`), `connection_core`(`sanitize_for_log`).
- **`messaging_host`** → `control`(`registry`, `MessagingSlot`), `status_fanout`, `connection_core`.
- **`status_fanout`** → `connection_core`.
- **순환:**
  - `connection_core` ↔ `command_delivery`
  - `connection_core` ↔ `command_roster`
  - `connection_core` ↔ `control`
  - `control` ↔ `command_delivery`
  - 잎: `status_fanout`, `experiment`. [확실]

##### 3. crate 간 사용 (수치 = 참조 줄 수)
- **`agent`**(가장 무겁다 — `connection_core` 42, `lib` 17, `messaging_host` 17, `control/mod` 9):
  - 모듈: `types`, `profile`, `preset`, `manager`(`AgentManager`), `session_tracker`, `persistence`, `backend`, `commands`(`NEW_AGENT_OUTPUT_FORMAT` 등), `turn`, `transport`, `session`, `output_core`, `name`, `failure`.
- **`net`**:
  - `frame_port` — `agent_conn`, `command_delivery`, `command_roster`, `connection_core`, `status_fanout`, `test_doubles`.
  - `ws`·`portfile`·`instance`·`auth` — `lib.rs`에서만.
- **`command`** — `connection_core` 18, `command_delivery` 10, `control/commands` 8, `command_roster` 4, 그 밖에 `catalog`/`agent`/`mcp_server`/`agent_conn`.
- **`messaging`**(`service`, `envelope`, `busy`) — `control/ingress` 14, `messaging_host` 9, `connection_core` 7, `mcp_server` 4, `registry` 2.
- **`protocol`** — `connection_core` 21, `agent_conn` 10, 그 밖에 `command_delivery`, `catalog`, `status_fanout`, `lib`.
- **`discovery`**: `default_data_dir`·`ensure_data_dir_writable`(`lib`), `find_install_root`(`control/priming`, `bin/engram`).
- **`base`**: `logging`·`platform`, `lib.rs`에서만.

##### 4. 책임 태그
- **데몬 조립 / 프로세스 수명:** `lib`, `main`
- **연결 dispatch + wire 변환 + 로그 유틸:** `connection_core`
- **명령 버스:** `command_delivery`, `command_roster`, `control/{commands,catalog,agent}`
- **net 프레임 포트 어댑터:** `agent_conn`, `status_fanout`
- **메시징 호스트:** `messaging_host`, `control/{ingress,mcp_server,registry}`
- **제어 평면(MCP/HTTP·토큰·프라이밍·스폰 설정):** `control`
- **제어 CLI:** `bin/engram`
- **실험:** `experiment`, bin 4개

##### 5. 핫스팟
- **`connection_core`(5036줄)가 5가지 관심사를 섞는다:** dispatch, core↔wire 변환, 로그 sanitize, 리스 브로드캐스트, 이벤트 빌더. 유틸이 핵심 3의 순환을 만든다.
- **`control/`이 3개 영역을 섞는다:** 명령 버스 입구(`agent`, `catalog`, `commands`), 메시징 전송(`mcp_server`, `ingress`), 스폰 준비(`mcp_config`, `priming`, `registry`).
- **메시징 책임이 흩어져 있다:** `messaging_host`, `control/mcp_server`(`MessagingSlot`), `control/ingress`, `connection_core`.
- **`bin/engram`(5446줄)은 데몬 crate 안의 독립 클라이언트다.** lib 코드를 전혀 공유하지 않고 하위 crate 3개에만 의존한다.
- **테스트 비계가 공개 API에 실려 나간다.** `experiment`와 테스트 서버 헬퍼(`start_test_server*`)가 lib 표면에 있다. `experiment`는 feature 게이트가 있고 테스트 서버 헬퍼는 없다.

---

#### 모름
- 정확한 `protocol` 심볼 수. 중괄호로 묶은 `use`를 다 펼치지 않아 근사치다. [불확실]
- 테스트 줄 수는 첫 `mod tests` 이후 전부를 테스트로 셌다. 그 위에 흩어진 `#[cfg(test)]` 항목은 빼지 않았다. [가능성 높음 — 오차 ~5% 이내]
- `connection_core`의 큰 테스트 블록(`:2033`부터) 뒤에 비테스트 항목이 있는지. 확인 안 함.
- `src-tauri/Cargo.toml`에서 `lib_unit`이 어떻게 정의돼 있는지(`[[test]] path`). 읽지 않았다.

### 1.3 하위 crate 내부

측정 방법: 줄 수는 `wc -l`. prod/test 분할은 파일의 첫 최상위 `#[cfg(test)]` 줄 기준이라 근사치다(prod 구획 안에도 `cfg(test)` 항목이 있는 파일이 있다 — 예 `backend/codex/transport.rs:520`). 모듈 간선은 주석 줄을 뺀 `crate::X` 참조로 셌고, `super::` 경로는 messaging 외에는 세지 않았다.

**핵심 5가지**

1. **agent가 압도적으로 크고, 세 곳에 몰려 있다**(확실). crate 전체 40,916줄 중 `backend/` 20,026(prod 약 7.7k), `manager.rs` 6,205(prod 2,967), `commands.rs` 2,733(prod 1,024). codex만 14,415줄(`backend/codex/*`). 그중 `transport.rs`(5,497, prod 2,640)는 `AgentTransport` 완전 구현인데 `agent/transport/`가 아니라 `backend/` 안에 산다 — transport seam 구현체가 두 디렉터리로 갈려 있다(`backend/codex/transport.rs:1-6`).
2. **agent 내부에 prod 순환이 실재한다**(확실).
   - backend ↔ output_core: `backend/codex/transport.rs:170`이 `OutputCore`를 import하고, `output_core.rs:19`가 `backend::TurnClassifier`를 import한다.
   - types ↔ profile: `types.rs:884`의 `StatusSink::restore_result(profile::RestoreReport)` 기본 메서드, 그리고 `profile.rs:18`의 `types::AgentId`.
   - 겉보기 순환 몇 개는 doc 주석·테스트 전용이라 실제 간선이 아니다: backend→manager(`codex/transport.rs:5412` 테스트 단언만), transport→manager/backend(doc 링크만), manager→commands(주석만).
3. **agent와 protocol이 같은 도메인 타입을 따로 정의한다 — 두 crate 사이 간선은 없다**(확실). `protocol/domain.rs:1`이 스스로 "`agent::types` / `agent::profile` 미러"라고 적는다. 양쪽에 같은 이름으로 정의된 것: AgentStatus, AgentInfo, AgentProfile, AgentFailureKind, Capabilities, Preset, RestoreReport, AgentOutputFormat, RestartPolicy, AgentId, PresetId, TurnOutcome.
   - `AgentCommand`는 crate마다 뜻이 다르다 — agent = spawn 종류, protocol = 클→데몬 요청 봉투(`protocol/lib.rs` 헤더).
   - 변환 코드는 이 범위 밖이다. `daemon/src/connection_core.rs`일 가능성 높음 — `engram_dashboard_agent::` 참조 42개이고 변환 함수 검색에 걸린 유일한 파일이다.
4. **discovery가 net에서 쓰는 것은 심볼 1개뿐이고, portfile 읽기를 따로 한다**(확실).
   - 쓰는 것은 `net::auth::AuthFrame` 하나(`discovery/src/lib.rs:18`).
   - 자체 `DAEMON_FILE="daemon.json"` 상수(`discovery:21`)와 자체 `FileReader`(`discovery:1016-1028`, `DaemonInfo::parse` 경유)를 둔다. net에도 같은 상수(`net/portfile.rs:33`)와 `portfile::read`(`net/portfile.rs:63`)가 있다.
   - stop 명령용 동기 tungstenite WS 클라이언트도 자체로 갖는다(`TungsteniteStopSender`, 약 `discovery:819-870`).
   - 결과적으로 WS 클라이언트 로직이 discovery, 새 transport crate, 그리고 src-tauri 데몬 클라이언트(추정 — 범위 밖)에 따로 있다.
5. **transport crate(9,047줄)는 소비자가 0이고, 기존 코드와 범위가 겹친다**(소비자 0 = 확실 / 겹침 = 해석). 명시 범위(`transport/lib.rs:1-6`)는 프레이밍, 요청/응답 상관, keepalive, 쓰기 시한, 재연결, 팬아웃이다. 겹치는 곳:
   - `net/ws.rs`: keepalive, 단일 writer, 팬아웃
   - `agent/backend/codex/{transport,protocol}.rs`: JSON-RPC id 계수기 + 대기표(`codex/transport.rs:3`)

   자기 `tests/`만 이 crate를 import한다.

#### agent (40,916줄 · 의존: base, command)

**목적·게이트.** Tauri import 0인 에이전트 코어. 데몬이 공유한다(`lib.rs:1-6`). 출력·상태는 `types::{OutputSink, StatusSink}` trait으로만 흐른다. 게이트: `rg "^\s*use tauri" src/` → 0줄.

**모듈**(prod/test 줄 수 · 책임):
- `backend/` 20,026 — 백엔드 고유 지식(claude/codex/gemini/shell). `AgentBackend` trait(`backend/mod.rs:206`)과 dispatch 자유 함수 약 20개(`:690-983` — `build_command_spec`, `open_spawn`, `turn_classifier`, `input_encoder`, `output_decoder`, `session_id_source` 등). 플랫폼 중립 CLI 래퍼 `console_command`(`:44`)와 `inject_cli_entrance`(`:90`)도 여기 있다.
  - `claude/` = mod 1137/2240 + `session_file.rs`.
  - `codex/` = `mod` 1174/2484 · `transport` 2640/2857(app-server stdio transport + Job Object) · `decoder` 1195/1659 · `protocol` 686/475(JSON-RPC wire) · `thread_lock` 473/772(파일 락 기반 세션 id 회수, ADR-0218).
  - `gemini/` prod 143 — **도달 불가 stub**. `AgentCommand`에 Gemini variant가 없다(`gemini/mod.rs:7-8`).
  - `shell/` 88.
- `manager.rs` 2967/3238 — 수명 오케스트레이터(`AgentManager`). 14개 모듈로 간선을 뻗는다.
- `commands.rs` 1024/1709 — 명령 버스 선언 `agent.*`(`declare_commands!` 블록 1개). 어댑터 층.
- `output_core.rs` 742/1206 — seq, replay, subscribers, status, finalize.
- `session.rs` 490/869 — OutputCore + `dyn AgentTransport` 합성.
- `transport/` 약 1.9k — `AgentTransport`·`OutputDecoder` seam(`mod.rs:34,51`) + `pty.rs` 462, `stdio.rs` 449, `input_queue.rs` 293, `api.rs`(빈 `ApiTransport`).
- `profile.rs` 846/1244 — 프로필 SSOT + spawn 종류 enum `AgentCommand`(Claude/Shell/Codex, `profile.rs:46-71`).
- `types.rs` 902/55 — **잡동사니**: id, AgentStatus, Output/InputEvent, reap 메시지, spawn용 `CommandSpec`, ControlEndpoint/ControlChannel trait, Capabilities 일가, OutputChunk/PtyEvent, AgentInfo, PtyError, Subscribe*, sink trait.
- `persistence/` 157+129 — agents.json·presets 원자 저장.
- `preset.rs` 138 — cwd 북마크.
- `name.rs` 92 — canonical 표시명.
- `reaper.rs` 229 — 종료 분류(단일 소비자).
- `turn.rs` 255 — (에이전트, epoch)별 턴 표.
- `session_tracker.rs` 206 — 세션 id drift 러너.
- `session_id_latch.rs` 139 — 첫 제출 래치(ADR-0226, `pub(crate)`).
- `failure.rs` 38 — 실패 종류 어휘.
- `platform/` 약 500 — Job Object(`windows.rs`), `file_holders`(Restart Manager), `process_tree`. 소비자가 하나씩이라 agent에 남았다(`platform/mod.rs:1-5`).

**간선**(prod, 주석 제외):
- backend → failure, output_core, platform, profile, session_tracker, transport(18), turn, types(21)
- manager → backend, failure, name, output_core, persistence, preset, profile(21), reaper, session, session_id_latch, session_tracker, transport, turn, types(15)
- session → backend(9), output_core, profile, session_id_latch, transport, types
- output_core → backend(5), profile, turn, types
- transport → output_core(9), platform, types
- commands → backend, manager, preset, profile, types
- session_id_latch → backend(`SessionIdSink`), types
- profile → failure, name, types
- types → profile
- reaper → name, profile, session, types
- persistence → preset, profile
- turn, name, session_tracker → types

순환: backend↔output_core, types↔profile.

**crate 간 사용.**
- base: `platform::process_creation_time` ×7, `pid_alive` ×4, `pid_alive_with_start_time` ×1, `child_pids` ×1, `logging::mask_secrets` ×3.
- command: prod는 `commands.rs:14` 한 곳뿐(`blocking_handler, declare_commands, CommandError, CommandTable, ErrorCode`). 테스트 전용 = `command::testing::{block_on, with_quiet_panic_hook}`.

**핫스팟.**
- 이름 충돌: `agent::types::CommandSpec`(프로세스 spawn 명세, `types.rs:186`) vs `command::CommandSpec`(버스 명세)(확실).
- backend 이름이 backend/ 밖으로 샌다: `commands.rs`의 `Claude`/`Codex` 백엔드 enum과 "claude"/"codex" 문자열(`:30-31,190-221`), `profile.rs`의 enum(확실).
- `AgentTransport` 구현이 backend/codex에 산다(확실).
- `types.rs`가 sink 계약·제어 채널·capabilities·spawn 명세를 섞는다(확실).
- manager가 거의 모든 모듈에 의존한다(확실).
- `agent/platform` vs `base/platform`은 소비자 수 기준으로 의도적으로 갈랐다(ADR-0175).

#### messaging (17,203줄 · 의존: 없음)

**목적·게이트.** 데몬이 사는 동안 에이전트 간 메시지를 확실히 배달하는 커널. 워크스페이스 의존 0(컴파일러가 강제). 호스트와는 포트로만 만난다: `DeliveryPort`, `ControlPlanePort`, `FlushTrigger`(`service.rs:295,309,415`)와 `TurnFacts`, `IdleNotifier`, `BusyGate`(`busy.rs:63-105`). 순수성: `mailbox`·`ledger`·`groups`는 tokio와 `Instant::now`를 쓰지 않는다(시계 주입)(`lib.rs:1-25`).

**모듈**(prod/test): `service.rs` 3896/7182(오케스트레이터 + 포트 + 뷰) · `ledger.rs` 1408/1799(메시지 상태 원장) · `mailbox.rs` 659/681 · `envelope.rs` 288/399 · `groups.rs` 249/173 · `busy.rs` 250/166(busy/턴 게이트). `PeerId = uuid::Uuid`와 `SenderIdentity{peer_id, epoch}`는 lib.rs에 있다.

**간선.** service → busy, envelope, groups, ledger, mailbox. 잎 모듈은 대체로 간선이 없다(`super::` 경유).

**핫스팟.** `service.rs` 11,078줄 — 이 범위에서 가장 큰 단일 파일(확실). 소비자는 데몬 하나(`daemon/src/messaging_host.rs`, 어댑터일 가능성 높음).

#### net (3,599줄 · 의존: base, protocol — 둘 다 `server` feature 뒤 optional)

**목적·게이트.** 소켓 수락 이후의 데몬 네트워크 행: WS 업그레이드, Origin 검사, 토큰 인증, 단일 writer, keepalive, 팬아웃, `frame_port` 계약, 단일 인스턴스 가드, portfile. `features.default=[]`라 기본 빌드는 `auth`만 남는다 — discovery·src-tauri용(`lib.rs:1-15`, `Cargo.toml:31-37`). accept loop는 아직 데몬에 있다(이사 보류, ADR-0130). 에이전트 어휘를 모른다. 게이트: 소스 정규식, base 심볼 allowlist = 정확히 2, `agent` 참조 0, cargo-tree 상한 = 3.

**모듈**(prod/test): `ws.rs` 889/1350(WS 서버) · `instance.rs` 626(daemon.json 단일 인스턴스 가드) · `portfile.rs` 89/143(daemon.json 읽기/쓰기/stale 판정, `:28`에서 `protocol::DaemonInfo` 재수출) · `frame_port.rs` 242(불투명 프레임 포트 trait) · `auth.rs` 40/78(`AuthFrame`).

**간선.** ws → auth, frame_port. instance → portfile.

**crate 간 사용.** base: `platform::current_process_start_time` ×2, `pid_alive_with_start_time` ×1. protocol: `DaemonInfo` ×1뿐.

#### discovery (2,889줄 = prod 1,320 + test 1,569, lib.rs 단일 · 의존: base, protocol, net)

**목적.** 데몬을 찾고, 없으면 WMI로 띄운 뒤 port와 token을 회수한다. 부수효과는 trait 뒤에 있다(`PidLiveness`, `DaemonReader`, `Spawner`, `Clock`, `ProcessKiller`, `StopSender`, `:411-752`). 그래서 `ensure_daemon`은 순수 오케스트레이션이다(`lib.rs:1-14`).

**내용**(태그: discovery/spawn, data-dir 정책, platform):
- data-dir 결정: `default_data_dir`, `release_data_dir`, 쓰기 가능 probe, `find_install_root`(`:84-373`)
- `ensure_daemon` / `ensure_with`(`:475, :932`)
- status/stop(`:613-797`)
- exe 위치 탐색(`:980`)
- `cfg(windows)` WMI COM spawn(`:1069-1314`)

**crate 간 사용.**
- net: `auth::AuthFrame`(×1)
- protocol: `AgentCommand`(stop 명령), `DaemonInfo`, `RequestId`, `PROTOCOL_VERSION`
- base: `pid_alive_with_start_time`

**핫스팟.** data-dir 정책(여러 곳이 쓰는 관심사)이 discovery에 산다(확실). portfile 읽기·WS 클라이언트 중복은 핵심 4번 참조.

#### protocol (2,801줄 · 의존: command)

**목적.** 프론트·src-tauri·데몬 사이 wire 계약. 타입 + serde + codec만 담고, Tauri와 도메인 로직은 없다(`lib.rs:1-15`). `PROTOCOL_VERSION`도 여기 있다.

**모듈**(prod/test): `messages.rs` 870/1052(클→데몬 `AgentCommand`, `AgentEvent`) · `domain.rs` 431/85(미러 타입) · `codec.rs` 94(binary 출력 프레임) · `discovery.rs` 51/74(`DaemonInfo`) · `ids.rs` 40.

**간선.** messages → domain, ids. codec·domain → ids.

**command에 의존하는 이유.** wire variant가 버스 타입을 싣기 때문이다. `messages.rs:4`가 `CommandDecl`, `CommandEnvelope`, `CommandReply`, `OwnerToken`을 import하고 variant `:277-337`, `:541-584`에서 쓴다. 나머지 참조는 테스트다. `:556`의 `engram_dashboard_daemon`은 주석일 뿐 의존이 아니다.

#### command (4,991줄 · 의존: 없음)

**목적·게이트.** **명령 0개**인 명령 버스 도구: 봉투, 오류 어휘, `declare_commands!`, 표, 명부, 3단계 route, `CommandSpec`만 담는 `inventory` 링커 수집(`lib.rs:1-30`). 게이트: 워크스페이스 path 의존 0, crate 안 `declare_commands!` 호출 0.

**모듈**(prod/test): `table.rs` 377/728 · `roster.rs` 381/609 · `error.rs` 404/266 · `spec.rs` 307/137 · `macros.rs` 353 · `route.rs` 193/404 · `coerce.rs` 184/147 · `link.rs` 93/89(전송 seam + 인바운드) · `envelope.rs` 102/26 · `testing.rs`(feature 뒤).

**간선.** table → coerce, route, spec. link·route·table은 테스트에서만 testing을 쓴다.

**이 범위의 소비자.** agent(`commands.rs`), protocol.

#### base (1,074줄 · 의존: 없음)

잎 crate, 입주자 2개: `logging/` 443/250(tracing 초기화, 파일 로그, `mask_secrets`) · `platform.rs` 230/97(pid liveness, 시작시각, `child_pids`). 입주자끼리 서로 참조하지 않는다. 게이트 3개는 `lib.rs:21-50`. 소비자: agent, net, discovery(확실).

#### transport (9,047줄 · 의존: 없음 · 간략)

재사용 전송 lib(ADR-0177). 소비자의 패킷 어휘는 제네릭 `Wire`, 링크와 시계는 `dyn`이다.
- **순수 층:** `machine` 440, `pending` 226, `stream` 380. 게이트 ③이 이 파일들에서 tokio/std::net을 막는다.
- **구동 층:** `peer` 1428/1651, `registry` 215, `event`, `policy`, `ws` 어댑터(`ws` feature 뒤), `testing`(`test-support` 뒤).

간선은 peer/registry 쪽으로 올라간다. 순수 파일은 frame/link/policy에만 의존한다. **소비자 0.**

#### 모름
- agent↔protocol 타입 변환의 정확한 위치: `daemon/src/connection_core.rs`일 가능성 높음, 미검증.
- transport crate가 `net/ws.rs`, codex JSON-RPC 배관, discovery stop 클라이언트 중 무엇을 대체할 의도인지: 모름(ADR-0177 미열람).
- agent 하위 모듈의 `super::` 경로에 숨은 실제 간선: 세지 않음.
- `ApiTransport`의 역할·상태(stub 이상인지): 불확실.

### 1.4 프론트(src/)

#### 핵심 5
1. **백엔드 통로가 단일하지 않다.** 정식 통로 = 싱글턴 `agentClient` = `ProtocolClient(new TauriTransport())`(`src/api/clientFactory.ts:22-27`). 그 옆에서 store·commands·layout·theme 모듈이 백엔드 권위 표면(레이아웃·탭·창·UI 설정·command 다리)에 Tauri `invoke`/`listen`을 직접 건다. (확실 · 상세 = 「백엔드 통로」)
2. **폴더 단위 import 순환이 있다.** components↔commands · components↔store · store↔commands.
   - store→commands = `store/eventBus.ts:6`(registry)
   - store→components = `store/viewStore.ts:43`(`components/slot/renderMode`)
   - commands→components = `commands/renderModeCommands.ts:21`(같은 `renderMode`)
   - 순환 셋 중 둘이 잎 파일 `components/slot/renderMode.ts` 하나를 지난다 — 하위 층에 있어야 할 파일이다. (확실)
3. **components가 가장 크고 백엔드도 직접 부른다.** 비테스트 7153줄 / 테스트 12251줄.
   - `api/` 반입 29건 = `layoutTypes` 13 · `clientFactory` 10 · `types` 9 · `agentClient` 4 · `wsFrame` 3
   - `components/layout/*`가 `invoke`를 직접 부른다(`get_view`·`list_tabs`·`report_ui_metrics`·`report_window_canvas`)
   - 같은 곳이 `window:tabs-updated`를 `listen`한다(`WindowLayout.tsx:11-12,71,83,124` · `useSplitDrag.ts:281` · `uiMetricsReport.ts:49` · `windowCanvasReport.ts:139`). (확실)
4. **세 창이 같은 SPA를 띄우고 창마다 App 부팅 전체를 돈다.** `App.tsx`의 HashRouter 라우트 셋 = `/` → `AppLayout`(main) · `/tree` → `TreePage`(agent-tree) · `/popup?window=<label>` → `PopoutPage`. 부팅 전체 = 데몬 ensure · eventBus · keybindings · view command bridge · agents/profiles/presets 조회(`App.tsx:30-68`). (확실)
5. **살아 있는 전역 핸들 4개.**
   - `__engramCmd` = 정식 command 레지스트리(`store/eventBus.ts:86`)
   - 버스 밖 곁문 셋 = `__ENGRAM_AGENT__` · `__ENGRAM_DAEMON__`(`api/clientFactory.ts:29,31`) · `__engram`(DEV 전용 · theme/agent/chatStyle store 노출 · `main.tsx:30`). (확실)

#### 폴더 (TS/TSX 줄 수, 비테스트 / 테스트)
| 폴더 | 비테스트 | 테스트 | 책임 |
|---|---|---|---|
| api | 3000 | 3486 | 백엔드 클라이언트: `AgentClient` 인터페이스(`agentClient.ts`) · `ProtocolClient` · `Transport` seam(`transport.ts:37`) · `TauriTransport`(운영) · `WsTransport`(운영 import 없음 — 테스트·텍스트 언급뿐, 가능성 높음) · `daemonControl` · `layoutTypes`/`types` DTO · `wsFrame` 태그 · `decodeBase64` |
| commands | 1696 | 2153 | 제어 표면: `registry` · `dispatch` · `contributions`(부수효과 등록 매니페스트 — `App.tsx:12`에서 import) · `keybindings` · `viewCommandBridge`(셸↔registry) · 도메인별 command 파일(agent·preset·slot·slotContent·tab·renderMode) · `slotMenu` · `enumArg` |
| components | 7153 | 12251 | 렌더링(하위 폴더는 아래 행) |
| ├ layout | 2187 | 4861 | 창·탭·분할 렌더: `AppLayout` · `WindowLayout` · `ViewLayoutRenderer` · `LayoutLeaf` · `Splitter` · `TabBar` · `useSplitDrag` · `ConnectionNotice` · 백엔드로 보내는 UI 수치·캔버스 보고 · `testing/rects.ts` |
| ├ slot | 3516 | 5551 | 슬롯 내용: `TerminalSlot`(xterm) · `RichSlot` · `DomSlot` · `StructuredTextView` · `structuredAccumulator` · `PresetPalette` · `AgentMonitoringPicker` · `SlotContextMenu` · `renderMode` · 마스코트 · `chat/`(Markdown·ChatRow·ThoughtRow·WaitRow …) |
| ├ agent | 1172 | 1560 | 에이전트 트리: `AgentList`(react-arborist) · `mergeTreeNodes` · `failureKinds` · `selectOpenTarget` · `statusGlyph` |
| ├ ui | 278 | 279 | 프리미티브: scroll-area · `LoadingPanel` · `RootErrorBoundary` · `nativeScrollActivity` |
| └ diff | 0 | 0 | 빈 디렉터리 (확실) |
| store | 945 | 928 | Zustand 상태: `agentStore` · `viewStore`(백엔드 `ViewManager` 미러, invoke/listen도 직접) · `eventBus`(구독 소유 + `__engramCmd` 설치) · `themeStore` · `chatStyleStore` · `monitoringPickerStore` |
| i18n | 311 | 178 | `t()` + 한국어 문자열(`ko.ts`) |
| theme | 156 | 343 | `ThemeManager`(`data-theme` 설정) · `uiSettings`(디스크 기반 UI 설정, invoke/listen 경유) |
| util | 83 | 370 | `basename` · `retryInvoke`(`retryAsync`) |
| lib | 7 | 0 | shadcn `cn()`만(`lib/utils.ts`) |
| pages | 79 | 67 | `TreePage` · `PopoutPage`(얇은 래퍼) |
| lab | 93 | 0 | `lab/terminal/TerminalView.tsx` = 격리 xterm resize 실험. 어디서도 import하지 않는다(죽었거나 격리 — 확실). 주석이 가리키는 `lab/richslot/*`는 이제 없다 |
| styles | CSS만 | — | `theme.css`(CSS 변수 테마) · `font.css` |
| 루트 | 150 | 219 | `main.tsx`(첫 페인트 전 테마·채팅 스타일, DEV `__engram`) · `App.tsx`(라우터 + 부팅) · `themes.ts` · `index.css`(+ 테스트) |

#### 백엔드 통로
**A. 프로토콜 클라이언트(정식)**
- `clientFactory.getAgentClient()` → 싱글턴 `ProtocolClient`(`protocolClient.ts:132`) over `TauriTransport`(`clientFactory.ts:22-27`). 같은 자리에서 `DaemonDaemonControl`을 만들고 전역 둘을 노출한다.
- `TauriTransport` invoke:
  - `subscribe_output`(Tauri `Channel` 동반) `tauriTransport.ts:352`
  - `forward_daemon_command` :442
  - `request_replay` :459
  - `daemon_connection_state` :162
  - `daemon_close` :487
- `TauriTransport` listen: `agent-list-updated` :196 · `status-changed` :204 · `restore-result` :219 · `profile-list-updated` :229 · `preset-list-updated` :237 · `daemon-connection-state` :257
- `daemonControl.ts` invoke: `daemon_start` :61 · `daemon_status` :101/:113 · `daemon_stop` :109
- `wsTransport.ts` invoke: `discover_daemon` · `read_daemon_info`(:218 · :230 · :552). 테스트·직결 흔적만 쓴다(가능성 높음).

**B. 프로토콜 클라이언트 밖 직접 `invoke`(백엔드 권위 표면)**
- `store/viewStore.ts:186-225`:
  - 탭: `create_tab` · `close_tab` · `switch_tab` · `rename_tab`
  - 창: `create_window` · `close_window` · `move_slot_to_window`
  - 슬롯: `split_slot` · `set_split_ratio` · `focus_slot` · `close_slot` · `assign_agent` · `set_slot_content`
  - 조회: `list_tabs` :419 · `get_view` :425
- `commands/slotCommands.ts:128` `resolve_spatial` · `commands/tabCommands.ts:214` `spawn_into`
- `commands/viewCommandBridge.ts:23,25` `report_view_commands` · `report_command_outcome`
- `theme/uiSettings.ts:30` `get_ui_settings`
- components/layout: 핵심 5 ③의 직접 호출들
- `@tauri-apps/plugin-dialog` `open`: `agentCommands.ts:5` · `presetCommands.ts:6` · `slotContentCommands.ts:6`
- `getCurrentWindow`: `viewStore.ts:29` · `viewCommandBridge.ts:16` · `uiSettings.ts:17`

**C. `listen` 등록 주체** (`rg "from '@tauri-apps/api/event'"` → 5파일)
- `api/tauriTransport.ts:28`(위 에이전트 이벤트)
- `store/viewStore.ts:384,387`(`layout:updated` · `window:tabs-updated`)
- `components/layout/WindowLayout.tsx:71`(`window:tabs-updated`, 자기 label만 거른다)
- `commands/viewCommandBridge.ts:116`(`command:request`)
- `theme/uiSettings.ts:24`(`ui:settings-updated`)

**D. command 레지스트리**
- `window.__engramCmd = {list, run}`를 `store/eventBus.ts:86`이 `commands/registry`에서 설치한다.
- 레지스트리는 `App.tsx:12`의 부수효과 import `commands/contributions`가 채운다.
- 핸들러는 store 액션이나 invoke로 라우팅할 뿐, 레지스트리가 상태를 갖지 않는다(`eventBus.ts:80-82` 주석).
- `viewCommandBridge`가 레지스트리를 셸에 보고하고, 셸이 내려보낸 command 요청을 실행한다.

**E. 기타**
- `components/slot/structuredAccumulator.ts:18-19`가 ts-rs 바인딩을 `../../../crates/engram-dashboard-protocol/bindings/`에서 직접 import한다. 폴더 간선 스크립트가 이것을 `components -> ..`로 셌다. (확실)
- `api -> ..` 간선 13건도 같은 원인일 것이다(가능성 높음 — 건별 미확인).

#### 창 → 라우트 → 페이지
- **main**: 정적 창(`src-tauri/tauri.conf.json:16`, url 없음 → `/`). `AppLayout` → `WindowLayout label=MAIN_WINDOW_LABEL`(`AppLayout.tsx:18`). (확실)
- **agent-tree**: 정적 창, 숨김, url `index.html#/tree`(`tauri.conf.json:24-26`). `TreePage` = `AgentList` + `ConnectionNotice`(`pages/TreePage.tsx:1-2`). (확실)
- **popout**: 런타임에 `WebviewWindowBuilder`가 만든다(`src-tauri/src/commands/popout.rs:112`, URL은 `window_url(label)`). 계기 = `create_window`(`commands/layout.rs:237`). 라우트 `/popup?window=<label>` → `PopoutPage` → `WindowLayout label`(main과 같은 컴포넌트) + `ConnectionNotice`(`pages/PopoutPage.tsx:4,22-29`). (확실)

#### 폴더 간 의존 (비테스트 import 수)
- ROOT → api · commands(2) · components(3) · pages(2) · store(5) · theme(2) · util
- pages → components(4) · store(1)
- components → api(29) · store(18) · i18n(15) · util(8) · lib(5) · commands(3) · crates 바인딩(2)
- commands → store(7) · api(6) · i18n(6) · components(1)
- store → api(3) · commands(1) · components(1) · util(1)
- theme → store(2) · util(1)
- api → util(1) · `..`(13, 「백엔드 통로」 E)

**순환·층 위반 표시**
- **components ↔ commands**: `LayoutLeaf.tsx:20` · `SlotContextMenu.tsx:10-11`이 `slotMenu`/`dispatch`를 import하고, `renderModeCommands.ts:21`이 `components/slot/renderMode`를 import한다.
- **components ↔ store**: `viewStore.ts:43`이 `components/slot/renderMode`를 import한다.
- **store ↔ commands**: `eventBus.ts:6`이 `commands/registry`를 import하고, commands는 store를 7건 import한다.
- **components → api/wsFrame**(`DomSlot.tsx:22` · `RichSlot.tsx:34` · `TerminalSlot.tsx:7`): 프레임 태그 상수가 클라이언트 추상 너머로 새어 나온다. IPC 호출은 아니고 가벼운 누수다.
- **components/layout이 store도 `agentClient`도 거치지 않고 raw `invoke`를 부른다.** 「컴포넌트·스토어는 agentClient에만 의존」 규칙 위반인지는 해석에 달렸다 — 레이아웃은 백엔드 권위 표면이고, CLAUDE.md는 거기서 직접 `listen`은 허용하지만 컴포넌트의 `invoke`는 명시적으로 허용하지 않는다. (불확실)
- api 층은 store·components를 import하지 않는다. (확실)

#### 책임 태그
| 폴더 | 태그 |
|---|---|
| api | 백엔드 클라이언트 · transport seam · wire DTO |
| commands | 제어 표면(LLM·키보드·메뉴) · 셸 command 다리 |
| components/layout | 렌더링 · 창/탭/분할 뷰 · 레이아웃 직접 백엔드 IPC |
| components/slot | 렌더링(터미널·리치·챗) · 출력 구독 소비자 |
| components/agent | 렌더링(트리) |
| components/ui | UI 프리미티브 |
| store | Zustand 상태 · 이벤트 구독 소유(`eventBus`) · 백엔드 레이아웃 상태 미러(`viewStore`) · 전역 command 핸들 호스트 |
| theme | 테마 · 디스크 기반 UI 설정 동기화 |
| i18n | 현지화 |
| util / lib | 헬퍼(retry·basename·`cn`) |
| pages | 라우트 진입 래퍼 |
| lab | 실험·격리(미사용) |
| styles | CSS 변수 테마·폰트 |

#### 모름
- `api -> ..` 간선 13건의 정확한 대상. `crates/*/bindings`의 ts-rs 바인딩으로 추정하지만 미확인이다.
- `WsTransport`의 런타임 사용 여부. 운영 코드 import는 찾지 못했다(테스트 전용일 가능성 높음).
- 창마다 부팅 전체(데몬 ensure·프로필·프리셋)를 도는 것이 의도인지 중복인지. 조사하지 않았다.
- import 수는 `from '...'` 정규식 스크립트로 셌다. 동적 `import()`와 부수효과 전용 import(`import './x'`)는 세지 않았다(예: `App.tsx:12`의 contributions).

## 2장 모듈 구성 원자료

**수집:** 서브에이전트 3명이 1.0 규칙으로 파일·줄 범위마다 도메인을 매겼다(데몬·셸 / 하위 crate / 프론트, 2026-09-26). 결과 = `architecture-map-data/tags.json`(216개 파일).

### 2.0 메인 요약 — HTML 2장의 근거

- **집계:** 모듈 × 도메인 줄 수는 `tools/gen_bars.py`가 tags.json을 묶어 계산한다(하위 crate는 crate 통째, 데몬·셸은 최상위 모듈, 프론트는 폴더). 막대의 「도메인 N」 = 80줄 이상 또는 15% 이상인 기능 도메인 수(기타 제외).
- **도메인별 총 줄 수(비테스트):** AGENT 24,165 · COMM 12,938 · CMD 10,231 · MAIL 9,189 · SCREEN 9,020 · RENDER 5,707 · DAEMON_LIFE 2,680 · AUTH 803 · BASE 1,261 · ROOT 1,784.
- **HTML 번호 ↔ 근거 절:** 분리 후보 ❶ control · ❷ bin/engram · ❸ connection_core · ❹ 셸 daemon_client · ❻ tray → 2.1 / ❺ net → 2.2 / ❻ 프론트 api → 2.3. 합치기 후보 ❼ 데몬 수명 · ❽ 통신 중복 · ❿ 커맨드 → 2.2(+2.1) / ❾ 인증(토큰 생성 둘 = 데몬 `lib.rs` `generate_token` · `control/mod.rs` `gen_token`) → 2.1 / ⓫ 프론트 에이전트 → 2.3.

### 2.1 데몬·셸 crate 태깅

범위 = `crates/engram-dashboard-daemon/src`(`src/bin/engram.rs` 포함. `experiment/`·테스트 하네스 전용 bin 넷(`saturation_pilot`·`roundtrip_smoke`·`drain_latency`·`priming_smoke`)·`test_doubles.rs`·`log_capture.rs` 제외) + `src-tauri/src`. 기준 커밋 = master `a226f63`. 파일별·구간별 태그 원본은 태그 JSON에 있다(경로 = 문서 머리). 여기엔 집계와 판단만 둔다.

#### 방법

- **비테스트 줄 수** = 최상위 `#[cfg(test)]` 바로 뒤에 `mod …`가 오는 첫 지점 앞까지. 파일 앞머리의 `#[cfg(test)] use` 줄은 경계로 치지 않는다. `tests.rs` 파일은 뺐다.
- **태깅 단위** = 파일당 태그 하나. 도메인이 둘 이상이고 각각 크면(파일의 약 15% 초과 또는 80줄 초과) `MIXED`로 두고 fn/impl 묶음 단위 구간으로 갈랐다. 51개 파일 중 15개가 `MIXED`.
- **근거** = fn/impl 윤곽 + 파일 헤더 + 핵심 본문(dispatch arm 이름 등). 모든 줄을 읽지는 않았다. **구간 경계는 ±10–20줄 오차가 있다(가능성 높음, 정확하지 않음).**

#### 모듈×도메인 표

단위 = 비테스트 줄 수(근사). DLIFE = DAEMON_LIFE.

| module | AGENT | MAIL | CMD | AUTH | COMM | DLIFE | SCREEN | BASE | ROOT |
|---|---|---|---|---|---|---|---|---|---|
| daemon::agent_conn | 516 | | | | | | | | |
| daemon::bin (`engram.rs`) | 444 | 379 | 1332 | | 181 | | | | 454 |
| daemon::command_delivery | | | 2385 | | | | | | |
| daemon::command_roster | | | 451 | | | | | | |
| daemon::connection_core | 1317 | | 269 | | 298 | 44 | | 22 | 81 |
| daemon::control | 1238 | 1123 | 830 | 557 | 359 | | | | 86 |
| daemon::lib | 76 | | | 29 | | 83 | | 54 | 735 |
| daemon::messaging_host | | 890 | | | | | | | |
| daemon::status_fanout | 77 | | | | | | | | |
| daemon::main | | | | | | | | | 16 |
| shell::commands | 308 | | 131 | | 52 | 177 | 925 | | 18 |
| shell::daemon_client | 691 | | 480 | 11 | 2801 | 81 | | | |
| shell::layout | | | | | | | 3541 | | |
| shell::output_channel/router | | | | | | | 307 | | |
| shell::ui_settings | | | | | | | 780 | | |
| shell::view_commands | | | 802 | | | | | | |
| shell::tray | | | | | | 306 | 305 | 21 | |
| shell::lib/main | | | | | | | | | 274 |
| **TOTAL** | 4667 | 2392 | 6680 | 597 | 3691 | 691 | 5858 | 97 | 1664 |

요점:

- **`daemon::control`이 가장 섞였다.** 기능 도메인 다섯을 전부 든다(AGENT 1238 · MAIL 1123 · CMD 830 · AUTH 557 · COMM 359, ROOT 86). 이름이 기능이 아니라 입구 표면이다.
- **`connection_core.rs`(2031줄)은 AGENT 중심이고 CMD 꼬리가 붙어 있다.** CMD 약 270줄 = dispatch arm(RegisterCommands·UpdateCommands·ListCommands·Command·CommandOutcome, 약 170줄) + `refuse_names_i_answer` + `note_claimed_owner`. 그 밖에 StopDaemon arm(DAEMON_LIFE, 44줄)과 COMM seam 약 300줄(OutboundSink·inbound lane·연결별 session)이 있다.
- **셸 `daemon_client/connection.rs`(1822줄)은 COMM이지만 CMD·AGENT를 싣고 있다.** CMD 약 230줄(shell owner advert·outcome sink·accept inbound·자기 명령 등록·reject foreign command) · AGENT 약 215줄(`apply_replay_event`·subscribe 거절·broadcast→`DaemonEvents`). `main_loop`(889–1227)는 replay flight 부기가 전송 코드에 엮여 있어 줄 구간으로는 못 가른다.
- **CLI `bin/engram.rs`(2790줄)는 한 파일에 도메인 다섯이다.** CMD catalog/invoke/help 약 1332 · AGENT 파싱·검증 444 · MAIL send/messages 379 · 손으로 짠 HTTP 클라이언트(COMM) 181 · ROOT 454. 기능별 분리 후보로 가장 선명하다.
- **tray는 SCREEN과 DAEMON_LIFE가 반씩이다.** `tray/mod.rs`·`actions.rs`·`core.rs`에 걸쳐 show/hide/menu = SCREEN 약 305, 데몬 liveness/start/stop/autostart = DAEMON_LIFE 약 306.

#### 분리 후보

- **`control/mcp_server.rs`**: MAIL 약 414 · AUTH 약 258 · COMM 약 296 · CMD 176 · ROOT 86 · AGENT 64. HTTP 호스트·auth 미들웨어·기능별 핸들러가 한 파일이다. 핸들러를 각 기능 옆으로 옮기면 라우터만 COMM으로 남는다.
- **`control/` 모듈**: AGENT 38% / MAIL 34% / CMD 25% / AUTH 17%(비테스트 약 3190줄 기준, `mcp_server`의 COMM/ROOT 제외). `mcp_config.rs`·`priming.rs`·`mod.rs` provision = AGENT, `ingress.rs` = MAIL, `catalog.rs`·`commands.rs` = CMD, `registry.rs` = AUTH+MAIL.
- **`control/registry.rs`**: AUTH 76%(토큰·identity, 1–298) / MAIL 24%(delivery observer·envelope format·mid-send hook, 299–391). 우편 배달 관측이 토큰 레지스트리에 얹혀 있다.
- **`connection_core.rs`**: AGENT 65% / COMM 15% / CMD 13% / DAEMON_LIFE 2%. CMD arm과 owner token 처리는 명령 버스 소속이다.
- **`bin/engram.rs`**: CMD 48% / ROOT 16% / AGENT 16% / MAIL 14% / COMM 6%. CLI 바이너리 하나에 기능별 파서·검증기가 모여 있다.
- **daemon `lib.rs`**: ROOT 75%. 그 안에 AUTH 토큰 생성, DAEMON_LIFE 단일 인스턴스 가드·portfile, AGENT exe 위치 찾기, BASE panic hook이 섞여 있고 약 190줄짜리 테스트 서버도 여기 산다.
- **셸 `daemon_client/connection.rs`**: COMM 약 75% / CMD 약 13% / AGENT 약 12%.
- **셸 `daemon_client/mod.rs`**: COMM 91% / DAEMON_LIFE 9%(discovery seam + ensure). 경계선이다.
- **셸 `daemon_client/protocol_state.rs`**: COMM 44% / AGENT 56%(구독 epoch 판정). 작다.
- **셸 `commands/discovery.rs`**: DAEMON_LIFE 75% / COMM 25%(connect/close/state invoke).
- **셸 `tray/*`**: SCREEN 약 50% / DAEMON_LIFE 약 50%.
- **확인했고 섞이지 않은 것(단일 도메인)**: `messaging_host.rs`(MAIL) · `view_commands.rs`(CMD) · `layout/apply.rs`·`layout/commands.rs`(SCREEN. `spawn_into`/`parse_backend` 약 97줄이 에이전트 스폰 포트를 쓰지만 SCREEN으로 둔다) · `commands/layout.rs`(SCREEN, 에이전트 스폰 포트 어댑터 약 60줄 포함) · `control/mod.rs`(AGENT provisioning. AUTH 약 15줄·MAIL grant 약 55줄이 안에 있다) · `agent_conn.rs`(AGENT 어댑터).

#### 흩어진 도메인

슬라이스 안에서 모듈 셋 이상에 퍼진 도메인:

- **CMD**: `command_delivery` · `command_roster` · `connection_core` · `control`(`catalog`/`commands`/`mcp_server`) · `bin` · 셸 `daemon_client`(`connection`/`inbound`) · `view_commands` · `commands/view_bus`. 모듈 8개. 셸 쪽 거처만 셋이다(`daemon_client/inbound`, `view_commands`, `daemon_client/connection`의 등록부).
- **AGENT**: `agent_conn` · `connection_core` · `status_fanout` · `control`(`mod`/`agent`/`mcp_config`/`priming`) · daemon `lib`(exe 위치) · `bin` · 셸 `daemon_client`(`replay_flight`/`protocol_state`/`connection`) · `commands/agent`.
- **AUTH**: daemon `lib`(WS 토큰) · `control/registry` + `mcp_server` 미들웨어 · `control/mod`(에이전트별 토큰 생성) · 셸 `connection`(auth frame). **토큰 생성 코드가 두 벌이다** — daemon `lib.rs`의 `generate_token`, `control/mod.rs`의 `gen_token`.
- **DAEMON_LIFE**: daemon `lib` · `connection_core`(StopDaemon) · 셸 `commands`(`discovery`/`autostart`) · `daemon_client/mod` · `tray`.
- **MAIL**: `messaging_host` · `control`(`ingress`/`registry`/`mcp_server`/`priming`의 텍스트 검사) · `bin`.
- **COMM**: `connection_core` · `control/mcp_server` · `bin`(HTTP 클라이언트) · 셸 `daemon_client` · `commands/discovery`.

#### 불확실 태그

형식 = 대상 — 후보 둘(고른 쪽 먼저) — 이유.

- **daemon `lib.rs` 95–170**(`set_engram_exe_env`/`locate_send_exe`) — AGENT vs DAEMON_LIFE — 스폰된 에이전트가 우편·제어에 쓰는 `engram` CLI exe를 찾는다. 스폰 provisioning으로 보고 AGENT를 골랐다.
- **daemon `lib.rs` 307–402** `run_accept_loop` — ROOT vs COMM — net accept loop에 `ConnectionCore`·bus를 조립한다.
- **`agent_conn.rs` 전체** — AGENT vs COMM — 규칙상 기능 쪽 어댑터지만 factory(435–516)와 FrameSink 인코딩은 배관에 가깝다.
- **`connection_core.rs` 284–363** `ConnectionSession` — COMM vs AGENT — 연결별 subs/viewport(에이전트 쪽)와 CMD·포화 래치를 함께 든다.
- **`control/agent.rs`** — AGENT vs CMD — `/control/agent` CLI 동사를 명령 표로 넘기는 어댑터이고, `agent.*` 동사의 주인은 agent다.
- **`bin/engram.rs` 139–383** help 텍스트 — CMD vs ROOT — CLI help가 모든 그룹을 덮는다.
- **셸 `commands/agent.rs`** — AGENT vs COMM — 얇은 invoke 빌더다. 규칙은 전달만 하는 invoke 핸들러를 COMM으로 친다. `subscribe_output`은 실은 창 채널 등록(SCREEN/COMM)이다.
- **셸 `daemon_client/events.rs`** — COMM vs AGENT — agent 목록·상태·profile 이벤트를 Tauri emit으로 넘기기만 한다.
- **셸 `daemon_client/replay_flight.rs`** — AGENT vs COMM — replay는 출력 코어 의미론이지만 여기서는 클라이언트 쪽 요청 부기다.
- **셸 `output_channel.rs`** — SCREEN vs COMM — 창별 Tauri 채널로 바이트를 민다.
- **`control/mcp_server.rs` 225–310** late-binding slot — ROOT vs 각 slot의 도메인.
- **`control/ingress.rs` 440–509** `ControlQueryResult` — MAIL vs COMM — `agent.rs`·`catalog.rs`도 쓰는 범용 결과 봉투다.
- **`connection_core.rs` CMD 구간(1546–1717)** — 경계 = 가능성 높음. arm 이름만 읽었고 본문은 다 읽지 않았다.

### 2.2 하위 crate 태깅

대상은 `agent`·`messaging`·`net`·`discovery`·`protocol`·`command`·`base`·`transport` crate의 `src/`이다. 파일·구간별 태그 원본은 태그 JSON에 있다(경로는 문서 헤더 참조). 여기에는 집계와 판단만 둔다.

#### 방법

- 비테스트 줄 = 첫 top-level `#[cfg(test)]` 앞의 줄. 모듈 헤더, `rg` 아웃라인, 표적 읽기로 태깅했다. 파일을 통째로 정독하지 않았다.
- test-support 모듈 2개는 제외했다: `transport/src/testing.rs`(704줄, `cfg(any(test, feature="test-support"))`), `command/src/testing.rs`(80줄).
- `net/instance.rs`는 318줄로 셌다. 319줄부터 `cfg(all(test, …))` 테스트다.
- MIXED 판정 기준 = 두 도메인 이상이 각각 파일의 약 15% 이상 또는 80줄 이상. 결과는 81개 항목이고 그중 MIXED 파일은 4개다(`agent/types.rs`·`net/ws.rs`·`discovery/lib.rs`·`protocol/messages.rs`).
- 도메인별 합계(비테스트 줄, 근사):

| 도메인 | 줄 수 |
|---|---|
| AGENT | 18,387 |
| COMM | 6,994 |
| MAIL | 6,797 |
| CMD | 2,740 |
| DAEMON_LIFE | 1,605 |
| BASE | 1,133 |
| AUTH | 206 |

핵심 관찰 다섯:

1. **하위 crate는 대부분 단일 도메인이다.** 도메인 분리가 필요한 crate는 없다. 문제는 섞임이 아니라 크기다(확실). 도메인 안에서 쪼갤 대상:
   - `messaging/service.rs` 3,895줄
   - `agent/manager.rs` 2,966줄
   - `agent/backend/codex/transport.rs` 2,639줄
   - `transport/peer.rs` 1,427줄
2. **`net` crate가 세 도메인을 품는다:** COMM 1,105, DAEMON_LIFE 406(`instance.rs`·`portfile.rs`), AUTH 206(`auth.rs` + `ws.rs` 264-430의 Origin 검사·토큰 핸드셰이크). daemon.json 가드·portfile은 소켓과 무관하다. crate 헤더가 네트워크 행 소관으로 적어서 거기 있을 뿐이고, discovery 쪽이 제자리다(가능성 높음).
3. **DAEMON_LIFE가 crate 셋에 흩어져 있다:** `discovery` 1,149, `net::instance`·`net::portfile` 406, `protocol::discovery` 50. daemon.json은 protocol이 정의하고 net이 쓰며, 읽기는 두 곳이다(discovery 자체 `FileReader` 1016-1032, `net::portfile::read`). 가장 강한 병합 후보(확실).
4. **요청/응답 대조가 최소 세 번 구현돼 있다:**
   - `transport::pending`(225) — 범용
   - `agent::backend::codex::transport`의 자체 `Pending`/`request_blocking`(426-527, 742-810 부근 약 170줄)
   - `protocol::messages`의 request-id 헬퍼(769-869)

   별도로 `discovery/lib.rs` 735-904는 stop 명령 하나만 보내는 수제 blocking tungstenite WS 클라이언트다. `transport` crate 기능과 중복이다(가능성 높음). 다만 transport crate가 blocking 호출자를 받을 수 있는지는 불확실하다.
5. **`protocol::domain`(430)과 `protocol::messages` 대부분은 agent 타입의 wire 미러다.** 헤더가 그렇다고 밝히고, `agent`가 `protocol`에 의존하지 않게 하려고 병합을 금한다. COMM으로 태깅했지만 내용은 AGENT 형태의 wire 타입 약 1,140줄이다. `messages.rs`에는 CMD wire variant 157줄도 있다.

#### 모듈×도메인 표

| module | AGENT | MAIL | CMD | AUTH | COMM | DAEMON_LIFE | BASE |
|---|---|---|---|---|---|---|---|
| agent::backend (claude/codex/gemini/shell/mod) | 8745 | | | | | | |
| agent::manager | 2966 | | | | | | |
| agent::transport (pty/stdio/api/input_queue) | 1404 | | | | | | |
| agent::commands (cmd adapter) | 1023 | | | | | | |
| agent::types | 768 | | 133 | | | | |
| agent::profile | 845 | | | | | | |
| agent::output_core | 741 | | | | | | |
| agent::session / session_id_latch / session_tracker | 489 / 138 / 205 | | | | | | |
| agent::persistence / preset / name / reaper / turn / failure / lib | 284 / 137 / 91 / 228 / 254 / 38 / 31 | | | | | | |
| agent::platform | | | | | | | 408 |
| messaging::service | | 3895 | | | | | |
| messaging::ledger / mailbox / envelope / busy / groups / lib | | 1407 / 658 / 287 / 249 / 248 / 53 | | | | | |
| command::* (10 files) | | | 2450 | | | | |
| net::ws | | | | 167 | 721 | | |
| net::auth | | | | 39 | | | |
| net::frame_port / lib | | | | | 242 / 142 | | |
| net::instance / portfile | | | | | | 318 / 88 | |
| discovery::lib | | | | | 170 | 1149 | |
| protocol::messages | | | 157 | | 712 | | |
| protocol::domain / codec / ids / lib | | | | | 430 / 94 / 40 / 104 | | |
| protocol::discovery | | | | | | 50 | |
| transport::* (13 files, peer 1427) | | | | | 4339 | | |
| base::logging / platform / lib | | | | | | | 442 / 229 / 54 |

#### 분리 후보

- **`net` crate:** COMM 1,105(65%) / DAEMON_LIFE 406(24%) / AUTH 206(12%).
  - `instance.rs`·`portfile.rs`를 DAEMON_LIFE 거처(discovery 리더·`protocol::discovery` 옆)로 옮긴다.
  - AUTH 핸드셰이크 분리도 가능하다. 다만 `handle_connection` 안(264-430)에 끼어 있어 함수 내부를 잘라야 한다.
- **`discovery/lib.rs`:** DAEMON_LIFE 87% / COMM 13%(735-904, blocking WS stop 클라이언트). 이 클라이언트는 transport crate 클라이언트로 바꾸거나 밖으로 뺀다. 데이터 폴더 해석(1-387)과 데몬 기동·정지도 같은 DAEMON_LIFE 안에서 파일로 가를 수 있다.
- **`protocol::messages`:** COMM/AGENT 미러 82% / CMD 18%(275-353, 519-596). 명령 버스 wire variant는 `protocol::command` 같은 별도 모듈로 뺄 수 있다.
- **`agent::types`:** AGENT 85% / CMD 15%. 213-345는 CLI 표면 어휘다(`CLI_EXE_NAME`, `CLI_GROUP_*`, `CLI_*_VERBS/FLAGS`, `TOKEN_ENV`, `CLI_EXE_ENV`, `AGENT_STATE_*`, `RENAME_OUTCOME_*`). 소비자는 engram CLI, 프라이밍, 명령 버스다. 나머지도 잡동사니지만 전부 AGENT라서 세 구간으로 자연스럽게 나뉜다.
  - status/events/reap(1-212)
  - 제어 채널 provision + capabilities(346-660)
  - output/sink 타입(661-901)
- **크기 과대 단일 도메인 파일(내부 분할만):** `messaging::service`, `agent::manager`(impl 블록 725-2955), `codex::transport`, `transport::peer`, `messaging::ledger`.

#### 흩어진 도메인

- **DAEMON_LIFE:** crate 셋(discovery, net, protocol).
- **COMM:** 4개 crate, 5곳 — transport crate, `net`(ws/frame_port), `protocol`, `discovery` stop 클라이언트, `agent`(codex/transport JSON-RPC pending, COMM 성격). 요청/응답 pending 로직이 중복이다(위 관찰 4).
- **AUTH:** `net::auth` + `net::ws` 264-430, `discovery` stop 클라이언트 안의 AuthFrame 조립(~10줄), `agent::types`의 `TOKEN_ENV` 상수. crate는 셋이지만 코드량은 적다.
- **CMD:** command crate(2,450) + `protocol::messages`(157) + `agent::types` CLI 어휘(133) = crate 셋. 기능 쪽 어댑터 `agent::commands`는 AGENT로 태깅했다.
- **BASE:** `base` crate + `agent::platform`(408). `agent::backend::console_command`(44-89)도 BASE 성격의 OS 헬퍼다.

#### 불확실 태그

- **`agent/types.rs` 213-345 — CMD vs MAIL/AGENT.** `engram` CLI의 verb·flag·env 어휘다. mail verb는 MAIL 성격이고 `TOKEN_ENV`는 AUTH 성격이라 어느 키에도 깔끔히 맞지 않는다.
- **`agent/platform/*` — BASE vs AGENT.** 내용은 도메인 무관 OS 질의(Job Object, Restart Manager 파일 점유자, 프로세스 트리)라 BASE로 태깅했다. `agent`에 있는 이유는 소비자가 둘 미만이라 base 입주 조건(ADR-0175)을 못 채워서다. "누가 쓰나"를 축으로 삼으면 AGENT다.
- **`agent/backend/mod.rs` `inject_cli_entrance`(90-200) — AGENT vs AUTH.** AGENT로 태깅했다. endpoint가 준 토큰·URL·exe를 env·PATH에 넣는 스폰 provision이고, 토큰을 생성·검증하지 않고 불투명하게 나르기만 한다. `ControlEndpoint`/`ControlChannel` 타입(`types.rs` 346-660)도 같은 판단이다. 토큰 발급은 이 조각 밖(daemon)이다.
- **`agent/backend/codex/transport.rs` — AGENT vs COMM(약 170줄의 JSON-RPC 대조).** 자식 stdio 위의 codex app-server 전용이라 AGENT로 유지했다.
- **`protocol/domain.rs`와 `messages.rs` 미러 구간 — COMM vs AGENT.** 지시대로 COMM으로 태깅했다. 기능상으로는 AGENT wire 어댑터다.
- **`protocol/discovery.rs` — DAEMON_LIFE vs COMM.** WS wire가 아니라 daemon.json 계약이다.
- **`net/ws.rs` AUTH 구간 경계 — 근사치.** 핸드셰이크는 `handle_connection` 앞부분(314-약 430)이고 나머지는 연결 수명 관리다.
- **`discovery/lib.rs` stop 클라이언트 — COMM vs DAEMON_LIFE.** 목적은 데몬 정지지만 코드는 WS 클라이언트 기계장치다.

### 2.3 프론트 태깅

#### 방법

- 대상은 `src/` 아래 비테스트 `.ts`/`.tsx` 전부다. `*.test.*`·`lab/`·CSS·`vite-env.d.ts`는 뺐다. 기준 커밋 = master `a226f63`.
- 파일마다 기능 도메인 하나를 붙였다: ROOT·AGENT·CMD·COMM·DAEMON_LIFE·SCREEN·RENDER·BASE.
- 한 파일에 두 도메인 이상이 실질 분량(대략 15% 초과 또는 40줄 초과)으로 섞여 있으면 MIXED로 두고 줄 범위별로 도메인을 붙였다.
- 기능 자신의 command 파일은 CMD가 아니라 그 기능 도메인으로 태깅했다(비고 "cmd adapter"). `agentCommands`·`presetCommands` → AGENT, `tabCommands`·`slotCommands`·`slotContentCommands` → SCREEN, `renderModeCommands` → RENDER.
- 파일·범위별 태그는 tags JSON에 있다(경로는 문서 헤더). 여기 옮기지 않는다.
- **MAIL은 없다**: 에이전트 간 메시징 UI가 src에 없다(mail/inbox grep 0건).
- **AUTH도 없다**: 토큰·신원 코드가 없다. `token`은 두 곳뿐이다 — `DaemonInfo.token` 타입(`daemonControl.ts`)과 `wsTransport.ts`.
- `wsTransport.ts`는 테스트·레거시 전용이다. 운영 경로는 `clientFactory`가 `TauriTransport`로 고정한다.

#### 폴더×도메인 표

비테스트 줄 수이고, MIXED 파일은 범위별로 나눠 셌다.

| folder | COMM | AGENT | DAEMON_LIFE | SCREEN | RENDER | CMD | BASE | ROOT |
|---|---|---|---|---|---|---|---|---|
| (root) | | | | | 29 (themes.ts) | | | 120 |
| api | 2194 | 461 | 324 | 21 | | | | |
| commands | | 286 | | 500 | 126 | 784 | | |
| components/agent | | 107 | | 60 | 1005 | | | |
| components/layout | | | 60 | 1910 | 217 | | | |
| components/slot | | 62 | | | 3454 | | | |
| components/ui | | | | | 278 | | | |
| i18n / pages | | | | | 311 / 79 | | | |
| store | | 195 | | 533 | 190 | 27 | | |
| theme | | | | 138 | 18 | | | |
| util / lib | 59 | | | | | | 24 / 7 | |
| **합계** | 2253 | 1111 | 384 | 3162 | 5707 | 811 | 31 | 120 |

#### 분리 후보

MIXED 파일 8개다.

- **`api/protocolClient.ts` (1038줄)**
  - COMM = 1-857, 1021-1038. replay/구독 상태기계, wire 이벤트 demux, subscribeOutput, close.
  - AGENT = 858-889(에이전트 동사), 896-1020(profile/preset CRUD + 이벤트 리스너). 약 157줄이다.
  - DAEMON_LIFE = 890-895(StopDaemon).
- **`api/tauriTransport.ts` (502줄)**
  - DAEMON_LIFE = 102-172(연결 상태 반영 + self-heal pull), 355-429(ensureReady/start → `daemon_connect`/`daemon_ensure`), 470-495(close → `daemon_close`). 약 170줄이다.
  - 나머지는 COMM이다.
  - 확신도 = 가능성 높음. connectionState가 Transport 계약의 일부라서 COMM/DAEMON_LIFE 경계는 판단이다.
- **`api/agentClient.ts` (273줄)**
  - COMM = 1-170(연결·출력 구독 계약).
  - AGENT = 171-273. 단 207-208의 stopDaemon 계약은 DAEMON_LIFE다.
- **`api/clientFactory.ts` (87줄)**: COMM = 1-57. DAEMON_LIFE = 58-86(`getDaemonControl`, `bootstrapDaemonIfNeeded`).
- **`store/eventBus.ts` (206줄)**
  - AGENT = 1-63, 150-206. 이벤트를 agentStore로 흘리고, profile/preset refresh와 재연결 resync를 한다.
  - CMD = 64-90(`__engramCmd` 전역 설치, 약 27줄).
  - SCREEN = 91-149(레이아웃 구독, main window init, 약 59줄).
  - 이름과 달리 전송 버스가 아니라 앱 전역 이벤트 조립점이다.
- **`components/layout/LayoutLeaf.tsx` (328줄)**
  - SCREEN = 1-111(frame, rect).
  - RENDER = 112-328. `SlotBody`가 Terminal/Rich/Dom slot, AgentList, PresetPalette 중 무엇을 그릴지 고른다.
- **`components/agent/AgentList.tsx` (691줄)**
  - RENDER = 1-167, 301-691.
  - AGENT = 168-193, 220-300(spawnProfile, kill, delete, reparent, rename).
  - SCREEN = 194-219(포커스 slot에 에이전트 열기).
- **`components/slot/PresetPalette.tsx` (340줄)**: AGENT = 86-147(preset delete/rename 호출). 나머지는 RENDER다.

임계 미만이라 단일 태그에 비고만 단 파일:

- `viewStore`: renderModeOverride 약 45줄이 RENDER 성격이다.
- `TerminalSlot`: writeStdin/resizePty를 인라인으로 호출한다.
- `slotContentCommands`: spawnAgent/killAgent를 호출한다.
- `wsTransport`: discover_daemon/read_daemon_info를 호출한다.

#### 흩어진 도메인

- **AGENT (1111줄, 6개 폴더)**
  - api: `types`, 그리고 `agentClient`·`protocolClient`의 AGENT 범위.
  - commands: `agentCommands`, `presetCommands`.
  - store: `agentStore`, `eventBus`.
  - components: agent·slot의 액션 범위(AgentList, PresetPalette), TerminalSlot의 인라인 I/O.
  - ROOT: App의 부팅 시 `getAgents`.
  - `features/agent` 같은 거처가 없다.
- **DAEMON_LIFE**: `api/daemonControl`, tauriTransport 범위 3개, clientFactory 범위 1개, `components/layout/ConnectionNotice.tsx`. ConnectionNotice는 layout 폴더에 있지만 DAEMON_LIFE 배너다.
- **SCREEN**: components/layout, `store/viewStore`, `store/monitoringPickerStore`, commands의 tab/slot/slotContent, `api/layoutTypes`, `theme/uiSettings`, `components/agent/selectOpenTarget`.
- **CMD**: 대부분 commands/(버스 본체)에 있고, eventBus의 `__engramCmd` 설치가 따로 떨어져 있다.
- **테마**: theme/, `store/themeStore`, `store/chatStyleStore`, 루트 `themes.ts`로 갈려 있다.

#### 불확실 태그

- `theme/uiSettings.ts`: "UI settings sync" 규칙대로 SCREEN으로 태깅했지만, 내용은 테마 적용뿐이라 RENDER일 수 있다.
- `store/monitoringPickerStore.ts`: SCREEN과 RENDER 사이.
- `components/agent/selectOpenTarget.ts`: SCREEN과 AGENT 사이.
- `store/viewStore.ts`: renderModeOverride 부분이 RENDER일 수 있다.
- `api/protocolClient.ts` 629-756(handleEvent): callback으로 demux만 하므로 COMM으로 태깅했지만, AGENT 이벤트를 전부 이름으로 다룬다.
- tauriTransport의 COMM/DAEMON_LIFE 경계는 판단이다. 리스너 블록(COMM으로 태깅)이 `daemon-connection-state`도 등록한다.
- `api/types.ts`: AGENT로 태깅했다. 내용은 wire DTO다.
- `util/retryInvoke.ts`: COMM으로 태깅했지만, 주 사용처는 SCREEN과 theme 코드다. BASE도 가능하다.
- `ConnectionNotice.tsx`: DAEMON_LIFE로 태깅했다. pages와 AppLayout이 import한다.
- eventBus 범위 경계는 근사다(±3줄).

## 3장 도메인 연관관계 원자료

**수집:** Rust = 서브에이전트 1명이 tags.json으로 import 수준 결합을 다시 셌다(`edges-rust.json`). 프론트 = 태깅한 수집자가 같이 셌다(`edges-front.json`). 숫자는 결합 지표이지 호출 횟수가 아니다.

### 3.0 메인 해석 — HTML 3장의 근거

- **그림에 그린 쌍** = 한 방향이라도 4 이상(8쌍). 숫자 = 3.1의 usage 귀속 행렬.
- **❶** AGENT→COMM 69 = 기능 쪽 wire 어댑터(`connection_core` 27 · `agent_conn` 25 · 셸 `commands/agent` 10)가 wire 타입을 씀 — 자연 방향. 역방향 10 = 셸 `daemon_client/connection.rs` → `protocol_state.rs` 46-119(AGENT 태그) · 데몬 `connection_core` 통신 범위 → `AgentManager` 등.
- **❷** AGENT→CMD 39 중 26 = `agent/src/types.rs` 213-345(CLI 어휘 상수, CMD 태그 — 태거도 불확실 표시). 재태깅하면 ≈13. 같은 범위가 MAIL→CMD 3 · AUTH→CMD 1도 만든다.
- **❸** CMD↔COMM 17/7 · **❹** COMM↔SCREEN 6/8(전부 셸 내부) · **❺** `control/registry.rs` 경유 AUTH·MAIL·CMD 얽힘 · **❻** 13쌍 중 9쌍이 얇음(한쪽 ≤3) · **❼** 프론트 SCREEN↔RENDER 정적 양방향, 나머지 순환은 등록·타입 import로만 닫힘.
- **귀속 방식 민감도:** AGENT→COMM · AGENT→CMD · BASE 잎은 useline 귀속에서도 유지된다. MAIL/AUTH/COMM의 작은 칸은 방식에 따라 흔들린다(3.1 「useline과의 차이」).

### 3.1 Rust 도메인 의존

master `a226f63` 기준 실측이다. 입력은 `map-lower.json` + `map-daemon-shell.json`(132 파일)이고 스크립트는 scratchpad `dom_dep2.py`(`dom_dep.py` 사본을 고친 것)다. 원자료: `dd2_edges.json`(교차 도메인 엣지 259건 — 필드 src·line·path·sdom·tdom·how·tfile·item·srcmode) · `dd2_misc.json`(미해결·ROOT 당김) · `out_usage.txt` · `out_useline.txt`.

#### 방법

- **지표** = 서로 다른 (소스 파일, 참조 항목 경로) 쌍. 같은 도메인 안의 쌍(922건)은 버리고 ROOT 소스는 따로 센다.
- **전처리**: 주석·문자열 리터럴·`#[cfg(test)]` 블록을 지우되 줄 번호는 보존한다. `use` 트리와 인라인 경로(`crate::`·`super::`·`self::`·`engram_dashboard_*::`)를 모은다.
- **소스 도메인 = usage attribution(주 방식)**. `use`로 들여온 이름은 `use` 줄이 아니라 **그 이름이 실제로 쓰인 모든 줄**의 range로 귀속한다. `use` 줄로 귀속하면 import 대부분이 헤더 range로 쏠리기 때문이다(예: `connection_core.rs` 1-79 = "AGENT header, imports"). 모듈 import(`use x::m;` 뒤 `m::Item`)는 `m::Item`으로 펼친다. glob·`_`·미사용 import(trait import 등 4건)는 `use` 줄로 폴백한다. 그래서 한 쌍이 여러 소스 도메인에 셀 수 있다. srcmode 분포: usage 210 · inline 37 · usage-mod 8 · useline-unused 4.
- **타깃 도메인** = 참조 항목의 **정의 줄**이 든 range. 정의는 해석된 파일에서 `fn|struct|enum|trait|type|const|static|mod|union|macro_rules!`를 이름으로 찾고, `pub use` 재수출을 따라간다(깊이 6까지). 정의를 못 찾으면 대상 파일의 태그로 폴백한다(MIXED면 가장 큰 도메인).
- **폴백: 259건 중 5건.** 전부 텍스트 정의가 없는 매크로 생성 항목이다(`declare_commands!`가 만드는 `COMMAND_SPECS` ×3 — `agent/commands.rs`·`T/layout/commands.rs` / `CATALOG_VERSION` ×2). 쌍별로는 AGENT→CMD 1 · CMD→AGENT 2 · SCREEN→CMD 1 · CMD→SCREEN 1이다. **미해결 0.**
- **도중에 고친 해석 버그 둘**
  - Windows 파일시스템이 대소문자를 가리지 않아 `engram_dashboard_command::Roster`가 `roster.rs`로 해석됐다. 파일 존재 확인을 대소문자 정확 비교로 바꿨다.
  - 상대 경로 `pub use mod::X` 재수출(`protocol/lib.rs`·`command/lib.rs`)과 `pub use engram_dashboard_discovery as discovery`(tauri `lib.rs`)를 따라가지 못해 약 100건이 폴백하고 있었다. 이제 따라간다.

#### 행렬 (행 = from, 열 = to, usage attribution)

| from\to | AGENT | MAIL | CMD | AUTH | COMM | DAEMON_LIFE | SCREEN | BASE | (ROOT) |
|---|---|---|---|---|---|---|---|---|---|
| AGENT | - | 2 | 39 | 2 | 69 | 1 | 3 | 12 | 2 |
| MAIL | 11 | - | 3 | 5 | 0 | 0 | 0 | 1 | 1 |
| CMD | 10 | 1 | - | 2 | 17 | 1 | 2 | 5 | 2 |
| AUTH | 1 | 2 | 1 | - | 2 | 1 | 0 | 2 | 0 |
| COMM | 10 | 0 | 7 | 2 | - | 4 | 6 | 0 | 0 |
| DAEMON_LIFE | 1 | 0 | 0 | 0 | 4 | - | 1 | 4 | 0 |
| SCREEN | 2 | 0 | 7 | 0 | 8 | 2 | - | 1 | 0 |
| BASE | 0 | 0 | 0 | 0 | 0 | 0 | 0 | - | 0 |

(ROOT) 열은 타깃이 ROOT로 태깅된 range라는 뜻이다: `agent_conn`→daemon `lib.rs` 루트 2 · `control/commands` 2 · `messaging_host` 1.

- **AGENT→COMM 69가 지배적이고 역방향 COMM→AGENT는 10이다.** 타깃: `protocol/domain.rs` 20 · `protocol/messages.rs` 16(AgentCommand ×6, AgentEvent ×5) · `connection_core.rs`의 COMM range 12 · `net/frame_port.rs` 11(FrameFanout, ConnId) · `protocol/ids.rs` 6 · `protocol/codec.rs` 2 · tauri `daemon_client/events.rs`·`mod.rs` 각 1. 에이전트 쪽 wire 어댑터가 wire 타입에 기대는 모양이라 방향은 예상대로다.
- **BASE는 깨끗한 잎이다.** 행이 전부 0이고 나머지 모든 도메인이 당긴다.

#### useline과의 차이

`python dom_dep2.py useline`(`use` 줄 귀속)은 엣지 247건, 같은 도메인 792건이다. 폴백은 fallback-module 7 · fallback-nodef 5다. 크게 달라지는 칸:

- MAIL→CMD 3 → 0
- COMM→MAIL 0 → 5
- COMM→AGENT 10 → 9, COMM→CMD 7 → 10, COMM→AUTH 2 → 4
- CMD→COMM 17 → 11
- AUTH→MAIL 2 → 4, AUTH→COMM 2 → 0
- DAEMON_LIFE→SCREEN 1 → 3

AGENT→COMM, AGENT→CMD, BASE 잎 결론은 두 방식에서 **견고하다**. MAIL/AUTH/COMM의 작은 칸은 **견고하지 않다**.

#### 순환

BASE를 뺀 13개 쌍이 양방향이다. 그중 9개는 **가는 순환**(한 방향 ≤ 3건)이다.

**굵은 순환** (괄호 안 = 작은 쪽 방향의 근거)
- **AGENT↔COMM 69/10.** COMM→AGENT: `connection_core.rs:114/287/2026`(AgentManager, OutputSink, SinkId, AgentId) · tauri `daemon_client/connection.rs:608-616/1037/1204`(AGENT로 태깅된 `protocol_state.rs` 46-119의 decide_epoch, EpochDecision, SubState, PendingMap).
- **AGENT↔CMD 39/10** (아래 「태깅이 만든 왜곡」 참고). CMD→AGENT: `control/commands.rs` 45/48/180(make_table, RosterChanged, AgentManager) · `control/catalog.rs` 171/287/392(`control/agent.rs`의 CommandArgs, preview, preview_within) · `command_delivery.rs:1102/2111`(`connection_core`의 AGENT range에 있는 event_json · CATALOG_VERSION) · `T/daemon_client/connection.rs:1734` · `T/view_commands.rs:204`.
- **CMD↔COMM 17/7.** COMM→CMD: `connection_core.rs:347-348`(CommandRoster, CommandBus) · `protocol/ids.rs:36`(command RequestId) · `mcp_server.rs:1254`(CommandTable) · tauri `connection.rs:214/259`·`mod.rs:335`(InboundSlot, OwnerToken, CommandReply).
- **COMM↔SCREEN 6/8** — 전부 src-tauri 안이다. COMM→SCREEN: `T/daemon_client/connection.rs:252-253/960-961`, `mod.rs:159/162`(OutputRouter, WindowChannelRegistry, send_to_windows, WindowLabel). 역방향: `T/commands/layout.rs`·`popout.rs`가 DaemonClient와 protocol 타입을 쓴다.
- **COMM↔DAEMON_LIFE 4/4** (문턱 바로 위). COMM→DL: DaemonInfo(`discovery/lib.rs:757`, tauri `connection.rs:237`, `mod.rs:490`) · DaemonDiscovery(`connection.rs:240`). DL→COMM: `connection_core.rs:1181`(AgentCommand) · `discovery/lib.rs:446`(PROTOCOL_VERSION) · `T/tray/mod.rs:217/248`(send_stop, StopOutcome).

**가는 순환**
- **AGENT↔MAIL 2/11.** AGENT→MAIL: `connection_core.rs:1534`(messaging EnvelopeFormat) · `control/agent.rs:156`(ingress::ControlQueryResult).
- **AGENT↔AUTH 2/1.** `agent_conn.rs:441`(ControlRegistry) · `mcp_server.rs:996`(BoundIdentity). 역방향: `registry.rs:25`(AgentId).
- **AGENT↔DAEMON_LIFE 1/1.** `priming.rs:276`(discovery::find_install_root). 역방향: `connection_core.rs:1193`(AgentStatus).
- **AGENT↔SCREEN 3/2.** `T/commands/agent.rs:157`(WindowChannelRegistry) · `connection.rs:1431/1433`(OutputRouter, WindowLabel). 역방향: `T/layout/apply.rs:521`(llm_creation_refusal) · `T/layout/commands.rs:844`(normalize_cwd).
- **CMD↔MAIL 1/3 — 태깅 산물.** `catalog.rs:103`(ControlQueryResult). 역방향: `bin/engram.rs` 934/1222/1223(CMD로 태깅된 `types.rs` range의 CLI_MAIL_* 상수).
- **CMD↔SCREEN 2/7.** `T/view_commands.rs:206`(layout::commands::COMMAND_SPECS — 매크로 생성, 폴백) · `:287`(MAIN_WINDOW_LABEL). 역방향: `T/layout/commands.rs` 7건(command crate의 table·error·declare_commands).
- **AUTH↔MAIL 2/5.** `registry.rs:56`(SenderIdentity) · `:95`(DeliveryObserver). 역방향: `ingress.rs:29/288`, `mcp_server.rs:364/393`, `messaging_host.rs:225`(registry 타입).
- **AUTH↔CMD 1/2 — 태깅 산물.** `mcp_server.rs:217`(CLI_GROUP_MAIL). 역방향: `mcp_server.rs:1072/1106`(BoundIdentity, TokenBinding).
- **AUTH↔COMM 2/2.** `net/ws.rs:318`(ConnectionHandlerFactory) · tauri `connection.rs:389`(PROTOCOL_VERSION). 역방향: `mcp_server.rs:1243`(ControlRegistry) · `discovery/lib.rs:810`(net::auth::AuthFrame).
- **DAEMON_LIFE↔SCREEN 1/2** — 전부 tauri tray 안이다. `T/tray/actions.rs:181`(TrayIcons). 역방향: `actions.rs:95`, `ui_settings.rs:236`(default_data_dir).

**AUTH/MAIL/CMD 엉킴은 전부 `daemon/control/registry.rs`를 지난다.** ControlRegistry·BoundIdentity·TokenBinding을 MAIL(5)·CMD(2)·AGENT(2)·COMM(1)이 당긴다. 그 파일의 299-391이 MAIL로 태깅돼 있어, 같은 파일의 AUTH 코드가 messaging 타입을 당기는 것도 교차 엣지가 된다. `mcp_server.rs`는 range가 여러 도메인으로 번갈아 태깅돼 AUTH↔CMD, AUTH↔MAIL, AGENT↔AUTH의 양쪽에 모두 나온다.

#### 쌍별 상위 소스 모듈 (쌍 수)

(`T/` = `src-tauri/src/`, 그 밖은 crate 접두를 뺀 경로)

- AGENT→COMM 69: `daemon/connection_core` 27 · `daemon/agent_conn` 25 · `T/commands/agent` 10
- AGENT→CMD 39: `agent/commands` 9 · `daemon/bin/engram` 7 · `daemon/control/agent` 6
- AGENT→BASE 12: `agent/backend/codex/thread_lock` 5 · `codex/transport` 2 · `agent/transport/stdio` 2
- AGENT→SCREEN 3: `T/daemon_client/connection` 2 · `T/commands/agent` 1
- AGENT→MAIL 2: `control/agent` 1 · `connection_core` 1
- AGENT→AUTH 2: `agent_conn` 1 · `control/mcp_server` 1
- AGENT→DAEMON_LIFE 1: `control/priming`
- MAIL→AGENT 11: `daemon/messaging_host` 9 · `control/ingress` 2
- MAIL→AUTH 5: `control/ingress` 2 · `control/mcp_server` 2 · `messaging_host` 1
- MAIL→CMD 3: `bin/engram` 3
- MAIL→BASE 1: `messaging_host`
- CMD→COMM 17: `daemon/command_delivery` 5 · `T/commands/view_bus` 3 · `T/daemon_client/connection` 3
- CMD→AGENT 10: `control/commands` 3 · `control/catalog` 3 · `command_delivery` 2
- CMD→BASE 5: `command_delivery` 2 · `command_roster` 1 · `control/mcp_server` 1
- CMD→AUTH 2: `control/mcp_server` 2
- CMD→SCREEN 2: `T/view_commands` 2
- CMD→MAIL 1: `control/catalog`
- CMD→DAEMON_LIFE 1: `bin/engram`
- AUTH→MAIL 2: `control/registry` 2
- AUTH→BASE 2: `registry` 1 · `mcp_server` 1
- AUTH→COMM 2: `T/daemon_client/connection` 1 · `net/ws` 1
- AUTH→AGENT 1: `registry`
- AUTH→CMD 1: `mcp_server`
- AUTH→DAEMON_LIFE 1: `T/daemon_client/connection`
- COMM→AGENT 10: `T/daemon_client/connection` 6 · `daemon/connection_core` 4
- COMM→CMD 7: `connection_core` 2 · `T/daemon_client/connection` 2 · `control/mcp_server` 1
- COMM→SCREEN 6: `T/daemon_client/connection` 4 · `T/daemon_client/mod` 2
- COMM→DAEMON_LIFE 4: `T/daemon_client/connection` 2 · `T/daemon_client/mod` 1 · `discovery/lib` 1
- COMM→AUTH 2: `control/mcp_server` 1 · `discovery/lib` 1
- DAEMON_LIFE→COMM 4: `T/tray/mod` 2 · `connection_core` 1 · `discovery/lib` 1
- DAEMON_LIFE→BASE 4: `daemon/lib` 2 · `discovery/lib` 1 · `net/portfile` 1
- DAEMON_LIFE→AGENT 1: `connection_core`
- DAEMON_LIFE→SCREEN 1: `T/tray/actions`
- SCREEN→COMM 8: `T/commands/layout` 5 · `T/tray/actions` 1 · `T/commands/popout` 1
- SCREEN→CMD 7: `T/layout/commands` 7
- SCREEN→DAEMON_LIFE 2: `T/ui_settings` 1 · `T/tray/actions` 1
- SCREEN→AGENT 2: `T/layout/apply` 1 · `T/layout/commands` 1
- SCREEN→BASE 1: `T/ui_settings`

#### ROOT가 당기는 것 (행렬에서 제외, 쌍 수)

- AGENT 16 — daemon `lib.rs` · `connection_core` ROOT range · `mcp_server`
- SCREEN 15 — 전부 src-tauri `lib.rs`
- CMD 9 — `bin/engram` · `connection_core` · `mcp_server` · tauri `lib.rs`
- COMM 8 — daemon `lib.rs` · `connection_core` · tauri `lib.rs`
- DAEMON_LIFE 7 — daemon `lib.rs`·`main.rs` · tauri `lib.rs`
- BASE 3 — daemon `lib.rs` · tauri `lib.rs`
- AUTH 2 — `connection_core` · daemon `lib.rs`
- MAIL 1 — `mcp_server`

#### 태깅이 만든 왜곡 (스크립트가 아니라 지도에서 온다)

- **`agent/types.rs` 213-345가 CMD로 태깅돼 있다.** CLI 어휘 상수 range이고 지도 스스로 "uncertain vs MAIL/AGENT"라 적어 둔 곳이다. AGENT→CMD 39건 중 26건이 여기를 가리킨다(CLI_EXE_NAME ×5, MCP_SERVER_NAME, TOKEN_ENV, CLI_EXE_ENV, AGENT_STATE_LIVE 등). 실제 명령 버스 의존은 13건이다(command crate error·table·lib 9 · `command_delivery` 2 · `command_roster` 1 · `control/commands` 1). 이 range를 AGENT로 바꾸면 AGENT→CMD는 약 13으로 떨어진다. 같은 range가 MAIL→CMD 3(CLI_MAIL_FLAGS·CLI_MAIL_VERBS·CLI_GROUP_MAIL)과 AUTH→CMD 1(CLI_GROUP_MAIL)도 만든다. **CMD↔MAIL, AUTH↔CMD 순환은 이 태그 때문에만 존재한다.**
- **`agent/platform/*`가 agent crate 안에 있는데 BASE로 태깅돼 있다.** AGENT→BASE 12건 중 5건이 여기다(JobObjectHandle ×3, file_holders, process_tree). 나머지 7건은 `base/logging`(mask_secrets ×3)과 `base/platform`을 가리킨다.
- **tauri `protocol_state.rs` 46-119가 AGENT로 태깅돼 있다.** COMM→AGENT 중 tauri 쪽 6건이 이것이다.
- **`control/registry.rs`가 AUTH(1-298)와 MAIL(299-391)로 갈려 있다.** 그래서 AUTH↔MAIL 엣지가 파일 하나 안에서 생긴다.
- 가는 순환들은 이런 태그에 민감하다.

#### 한계

- 정의 매칭은 정규식이다. 파일에서 그 이름의 첫 정의를 잡고, 메서드와 variant는 부모 타입으로 해석하고, cfg로 갈린 동명 정의는 첫 것을 쓴다. **가능성 높음** — 엣지별로 검증하지는 않았다.
- usage 귀속은 단어 경계로 이름을 맞춘다. 로컬 이름이 필드나 변수와 겹치면 과대 귀속될 수 있다. 크기는 **모른다**(작을 것으로 추정).
- 지도에 없는 파일은 무시한다. 매핑된 132 파일 안에서 태그가 없는 소스는 없었다.
- 매크로 생성 항목 5건은 정의 줄 대신 파일 태그로 귀속했다(위 폴백).

### 3.2 프론트 도메인 의존

#### 귀속 방법

- **세는 단위**는 import 문이다(심볼 수가 아니다). 정적·type-only·side-effect-only로 나눠 센다. 동적 import는 0건이다.
- **도구**: 스크래치패드 `fe_domains.cjs`(원본 `deps.cjs`를 복사해 수정). 태그는 tags JSON(경로는 문서 헤더), import별 귀속 결과는 `edges-frontend.json`의 `fdHow`/`tdHow`에 있다.
- **출발(from) 쪽 = first-use 휴리스틱**
  - "import 문을 담은 범위" 규칙은 쓰지 않았다. import가 전부 파일 맨 위라 그 규칙이면 전부 첫 범위로 간다.
  - 대신 MIXED 파일에서는 import한 로컬 이름이 주석이 아닌 곳에서 처음 쓰인 줄의 범위로 귀속했다.
  - 수동 보정 1건: `store/eventBus.ts`의 5행 import(`agentClient`)는 AGENT, 6행(`registry` list/run)은 CMD로 고정했다. 휴리스틱이 잡음을 탔기 때문이다.
- **도착(to) 쪽**
  - MIXED 대상은 심볼이 정의된 범위로 귀속했다.
  - 정의를 못 찾으면 그 파일의 우세 도메인으로 fallback했다. 해당 1건은 틀렸다: `ViewLayoutRenderer.tsx:12`→`LayoutLeaf` default export가 RENDER로 갔지만, 실제로는 SCREEN 부분이다.

#### 행렬

행 = from, 칸 = 정적/type-only/side-effect-only 수.

| from\to | ROOT | AGENT | CMD | COMM | DAEMON | SCREEN | RENDER | BASE |
|---|---|---|---|---|---|---|---|---|
| ROOT | - | 3/0/0 | 3/0/1* | 2/0/0 | 1/0/0 | 2/0/0 | 7/0/0 | - |
| AGENT | - | - | 5/0/0 | 5/0/0 | - | - | 2/0/0 | 1/0/0 |
| CMD | - | 0/0/2 | - | - | - | 0/1/3 | 0/0/1 | - |
| COMM | - | 0/1/0 | - | - | 1/0/0 | - | - | - |
| DAEMON | - | - | - | 1/1/0 | - | - | - | - |
| SCREEN | - | - | 6/0/0 | 7/0/0 | 1/0/0 | - | 9/1/0 | - |
| RENDER | - | 7/7/0 | 4/1/0 | 6/4/0 | 2/0/0 | 6/0/0 | - | 8/0/0 |

\* ROOT→CMD에는 artifact 1건이 들어 있다. App의 `initEventBus` import가 CMD로 잡혔는데, 함수가 64행(CMD 범위)에서 시작하기 때문이다. 실제로는 AGENT·SCREEN·CMD에 걸친다. 빼면 이 칸은 2/0/1이다.

#### 순환

**정적 import로 양방향인 진짜 순환은 SCREEN↔RENDER 하나다.**

- SCREEN→RENDER: `WindowLayout.tsx:31`→`AgentMonitoringPicker`, `viewStore.ts:43`→`renderMode`, `uiSettings.ts:21`→`themeManager`.
- RENDER→SCREEN: `LayoutLeaf.tsx:23`(SlotBody)→`uiMetricsReport`, `renderModeCommands.ts:22`→`viewStore`, `PopoutPage.tsx:23`→`WindowLayout`.

나머지 순환은 CMD가 끼어 `contributions.ts`의 side-effect import로만 닫히거나, 한쪽이 type-only다. 쌍마다 가장 작은 근거:

- AGENT↔CMD
  - `agentCommands.ts:13`→`registry`(register), 정적.
  - `contributions.ts:13`→`agentCommands`, side-effect.
- AGENT↔COMM
  - `agentCommands.ts:9`→`clientFactory`(agentClient), 정적.
  - `protocolClient.ts:32`→`types`, type-only.
- COMM↔DAEMON_LIFE
  - `clientFactory.ts:10`→`daemonControl`, 정적.
  - `daemonControl.ts:13`→`agentClient`는 type-only이고, `ConnectionNotice.tsx:19`→`clientFactory`는 정적이다.
- SCREEN↔CMD
  - `slotCommands.ts:17`→`registry`, 정적.
  - `contributions.ts:9`→`slotCommands`는 side-effect, `slotMenu.ts:8`→`layoutTypes`는 type-only다.
- RENDER↔CMD
  - `SlotContextMenu.tsx:10`→`dispatch`, 정적.
  - `contributions.ts:11`→`renderModeCommands`, side-effect.
- AGENT↔RENDER
  - `agentCommands.ts:7`→`i18n`(t), 정적.
  - `LayoutLeaf.tsx:10`→`agentStore`, 정적. 기능 command 파일의 i18n 의존이 이 순환을 만든다.

#### 백엔드 도달 표

| FE 도메인 (파일) | invoke / event | BE 도메인 |
|---|---|---|
| SCREEN (viewStore) | create_tab, close_tab, switch_tab, create_window, close_window, split_slot, set_split_ratio, rename_tab, focus_slot, close_slot, set_slot_content, move_slot_to_window, list_tabs, get_view; event `layout:updated`, `window:tabs-updated` | SCREEN |
| SCREEN (viewStore) | assign_agent | SCREEN (slot↔agent 바인딩) |
| SCREEN (WindowLayout, useSplitDrag) | window:tabs-updated, list_tabs, get_view | SCREEN |
| SCREEN (uiMetricsReport, windowCanvasReport) | report_ui_metrics, report_window_canvas | SCREEN |
| SCREEN (uiSettings) | get_ui_settings, event `ui:settings-updated` | SCREEN |
| SCREEN (slotCommands) | resolve_spatial | CMD |
| SCREEN (tabCommands) | spawn_into | SCREEN+AGENT |
| SCREEN (slotContentCommands) | agentClient.spawnAgent/killAgent → forward_daemon_command | COMM, AGENT 운반 |
| CMD (viewCommandBridge) | report_view_commands, report_command_outcome, event `command:request` | CMD |
| COMM (tauriTransport) | forward_daemon_command | COMM. AGENT 동사(SpawnByCwd, Kill, Interrupt, WriteStdin, Resize, ListAgents, GetSnapshot, profile/preset CRUD)와 DAEMON_LIFE(StopDaemon)를 운반 |
| COMM (tauriTransport) | subscribe_output, request_replay | COMM, RENDER slot 출력 운반 |
| COMM (tauriTransport) | event agent-list-updated, status-changed, restore-result, profile-list-updated, preset-list-updated | COMM, AGENT 운반 |
| DAEMON_LIFE (tauriTransport 범위) | daemon_connection_state, daemon_connect, daemon_ensure, daemon_close | DAEMON_LIFE |
| COMM (tauriTransport 리스너 블록, COMM으로 태깅) | event `daemon-connection-state` (257행) | DAEMON_LIFE |
| DAEMON_LIFE (daemonControl) | daemon_start, daemon_status, daemon_stop | DAEMON_LIFE |
| COMM (wsTransport, 테스트 전용) | discover_daemon, read_daemon_info | DAEMON_LIFE |

client를 거치는 간접 도달:

- **AGENT**: agentCommands, presetCommands, AgentList, PresetPalette, eventBus가 `agentClient`를 거쳐 forward_daemon_command로 간다.
- **RENDER**: Terminal/Rich/DomSlot이 subscribeOutput을 거쳐 subscribe_output·request_replay에 닿는다. TerminalSlot은 writeStdin/resizePty도 보낸다.
- **DAEMON_LIFE stop**: `DaemonControl.stop`은 `window.__ENGRAM_DAEMON__` 전역으로만 닿는다. UI 호출처는 0이다.

#### 한계

- **메서드 단위 도달은 import 분석에 안 보인다.** `agentClient`가 COMM 심볼이라 UI 코드의 `agentClient.killAgent` 같은 호출은 X→COMM으로만 잡힌다. 실제 AGENT 의존은 백엔드 도달 표로 보완해야 한다.
- **first-use 휴리스틱에는 잡음이 있다.** 흔한 이름(`list`·`run`)이 오탐된다. 수동 보정 1건, 오귀속 fallback 1건(`LayoutLeaf` default), `initEventBus` artifact 1건이 있다.
- **같은 파일 안 범위 사이 의존은 세지 않았다.** 예: protocolClient AGENT 범위 → COMM 범위.
- **src 밖 생성 바인딩 import 15건은 행렬에서 뺐다.** `src-tauri/bindings`와 protocol `bindings`이고, EXT로 표시했다.
- MIXED 범위 경계가 근사라서 경계 부근 귀속은 ±몇 줄 차이로 바뀔 수 있다.

## 부록 A — 첫 도메인 초안 기준 의존(폐기 2026-09-26)

> 1.0 「폐기한 첫 초안」 기준으로 센 것이다. 도메인 경계가 바뀌어 숫자·순환·관측 전부 무효 — 「연결이 허브」가 착시였음을 보여 주는 기록으로만 남긴다. HTML에서는 지웠다.

**수집:** 서브에이전트 2명(Rust · 프론트)이 1장 도메인 초안을 모듈·파일 단위 매핑으로 박아 import 수준 결합을 셌다(2026-09-26). 매핑은 각 절 머리에 있다. 숫자는 결합 지표이지 호출 횟수가 아니다.

### A.0 메인 해석(폐기된 초안 기준)

- **❶ 연결 = 허브, 그러나 대부분 배관 일이 아님:** N→L 35 중 27이 셸 `commands/{layout,settings,tray}.rs`(화면 배치의 IPC 어댑터를 N으로 매핑한 탓 — 2.1 misfit 1). N→A 61 중 25, N→C 46 중 12가 데몬 `connection_core.rs`(에이전트 타입 wire 변환 · AgentCommand 배정 정책 · `RosterBroadcast` impl — 2.1 misfit 2). 즉 매핑을 고치면 허브 크기는 줄지만, `connection_core`가 여러 도메인을 안은 사실은 남는다.
- **❷ Rust 순환 6:** L↔N · L↔C · C↔N · C↔A · N↔A · N↔M. 얇은 둘 = N↔A(A→N 1: `control/priming.rs:276` → `discovery::find_install_root`) · L↔C(C→L 2: `view_commands.rs` → `layout::MAIN_WINDOW_LABEL`, `layout::commands::COMMAND_SPECS`). 로그 유틸 몫(6)은 어떤 순환도 단독으로 만들지 않는다.
- **❸ C↔N crate 단위:** `protocol` → `command`(wire 변형에 `CommandEnvelope`·`CommandReply`·`OwnerToken`·`CommandDecl`, `RequestId`) · 데몬 `command_delivery`/`command_roster` → `net::frame_port`.
- **❹ A·M 깨끗:** A 바깥 = C 5(`agent/src/commands.rs`의 선언 도구) + B 7 + N 1(❷). M 바깥 = A 10 + N 5(+1 유틸).
- **❺ 자리 어긋남:** CLI 어휘 상수가 `agent::types`(소비 = N `bin/engram.rs` ×12 · C) · `tray/*`가 데몬 기동·정지(N 일) · `output_router`/`output_channel`을 N `daemon_client`가 씀 · `ControlQueryResult`가 M(`control/ingress.rs`)에 정의되고 C가 소비(C→M의 유일한 원천).
- **❻~❽ 프론트:** R↔L 최대 순환(`LayoutLeaf.tsx`가 슬롯 전부 마운트 · 슬롯이 `viewStore.assignAgent` · `viewStore.ts:43` → `components/slot/renderMode`) · N의 바깥 선 전부 `store/eventBus.ts`(→A `:7` · →C `:6` · →L `:8`) · L이 `invoke` 직접 호출(8개 파일 — `api/` 우회).
- **그림에서 뺀 것:** 바닥(모두 → B, B → 없음) · 가는 한 방향 L→A 2 · C→M 2 — 표에만.

### A.1 Rust 도메인 의존(초안 매핑)

기준 트리 = master `a226f63`. 핵심 사실 다섯:
1. 바깥으로 나가는 의존이 가장 많은 도메인은 N(연결)이다(N→A 61 · N→C 46 · N→L 35 · N→M 11 · N→B 2). 대부분 세 곳에서 나온다 — daemon `connection_core.rs`(A 25 · C 12), `control/mcp_server.rs`(C 14 · M 6), src-tauri IPC 핸들러 `commands/{layout,settings,tray}.rs`(L 27). (수치 확실)
2. 유틸 중력을 빼도 도메인 순환이 여섯 개 남는다: L↔N · L↔C · C↔N · C↔A · N↔A · N↔M(L↔A는 없다). 이 중 둘은 가늘다 — N↔A는 A→N 참조 1건(`control/priming.rs:276`)에, C↔L은 C→L 참조 2건(`view_commands.rs`)에 걸려 있다. (확실)
3. 유틸 중력은 작다 — 합계 6건(C→N 5 · M→N 1). 유틸 때문에만 생기는 순환은 없다. (확실)
4. 명령 어댑터가 실제 간선의 출처다. `agent/src/commands.rs`가 A→C 5건 전부, `layout/commands.rs`가 L→C 7건 전부와 L→A 1건(`normalize_cwd`)을 만든다. A의 정책이 L로도 새어 든다(`layout/apply.rs` → `agent::commands::llm_creation_refusal`). (확실)
5. crate 수준 사실 셋:
   - `engram-dashboard-transport`는 **테스트 밖 소비자가 0**이다(의존하는 Cargo.toml이 없고 자기 `tests/`만 참조한다).
   - `protocol` → `command`는 crate 수준의 N→C 간선이다.
   - `agent` → `command`는 A→C다. (확실)

#### 모듈 → 도메인 매핑 (이 조사에서 권위로 쓴 것)
- **L 화면 배치:** src-tauri `layout/*`(`layout/commands.rs` = L의 명령 어댑터) · `output_router.rs` · `output_channel.rs` · `ui_settings.rs` · `tray/*` · `commands/popout.rs`
- **C 명령:**
  - crate `command` 전체
  - src-tauri `view_commands.rs` · `commands/view_bus.rs` · `daemon_client/inbound.rs`
  - daemon `command_delivery.rs` · `command_roster.rs` · `control/{commands,catalog,agent}.rs`
- **N 연결:**
  - crate `protocol` · `transport` · `net` · `discovery` 전체
  - src-tauri `daemon_client/*`(inbound 제외) · `commands/*`(popout·view_bus 제외)
  - daemon `connection_core.rs` · `agent_conn.rs` · `status_fanout.rs` · `control/{mcp_server,registry,mod}.rs` · `bin/engram.rs`
- **A 에이전트 실행:** crate `agent` 전체(`agent/src/commands.rs` = A의 명령 어댑터) · daemon `control/{mcp_config,priming}.rs`
- **M 메시징:** crate `messaging` 전체 · daemon `messaging_host.rs` · `control/ingress.rs`
- **B 바닥:** crate `base`
- **조립실(행렬에서 제외):** src-tauri `lib.rs`·`main.rs` · daemon `lib.rs`·`main.rs`
- **완전 제외:** 테스트 코드(`#[cfg(test)]`·`tests.rs`·`tests/`) · daemon `experiment/` · 테스트 하네스 bin(`saturation_pilot`·`roundtrip_smoke`·`drain_latency`·`priming_smoke`) · `test_doubles.rs` · `log_capture.rs`
- **유틸 중력:** daemon `connection_core.rs`의 로그 정리 유틸(`sanitize_for_log`·`sanitize_within`·`event_json`) 때문에만 생긴 참조는 따로 센다.

#### 측정 방법
- **단위:** 수치는 (원천 파일, 참조 항목 경로)의 서로 다른 쌍의 개수다.
- **원천:** 펼친 `use` 트리 + 인라인 한정 경로(`crate::`/`super::`/`self::`/`engram_dashboard_*::`). 주석·문자열·`#[cfg(...test...)]` 항목(test·test-harness·test-support)을 걷은 뒤에 센다.
- **범위:** 같은 도메인 안 참조는 버린다. 134개 파일, 매핑 안 된 파일 0.
- **수동 보정:** `mod.rs` 안의 상대 자식 모듈 `use`(예: `use inbound::X`)는 스크립트가 못 잡아 손으로 더했다.
- **읽는 법:** 결합 지표이지 호출 횟수가 아니다.

#### 행렬 (행 = 출발 · 열 = 도착 · "(+Nu)" = 유틸 중력)
| from\to | L | C | N | A | M | B |
|---|---|---|---|---|---|---|
| L | – | 7 | 14 | 2 | 0 | 1 |
| C | 2 | – | 14 (+5u) | 8 | 2 | 0 |
| N | 35¹ | 46² | – | 61³ | 11 | 2 |
| A | 0 | 5 | 1 | – | 0 | 7 |
| M | 0 | 0 | 5 (+1u) | 10 | – | 0 |
| B | 0 | 0 | 0 | 0 | 0 | – |

- ¹ 스크립트 34 + 1: `commands/mod.rs:15` `pub use popout::*`
- ² 스크립트 44 + 2: `commands/mod.rs:18` `pub use view_bus::*` · `daemon_client/mod.rs:45` `use inbound::InboundSlot`
- ³ 스크립트 59 + 2: `control/mod.rs:22,24` (`mcp_config::MCP_SERVER_NAME`, `priming::PrimingProvider`)

B는 순수 잎이다. (확실)

유틸 중력 6건의 내역:
- C→N 5: `command_delivery.rs` ×3 · `command_roster.rs` ×1 · `control/catalog.rs` ×1
- M→N 1: `messaging_host.rs` ×1

유틸을 빼도 C→N은 14로 남는다.

#### 순환 (유틸 중력 제외, 방향별 최소 근거)
- **L↔N**
  - L→N 14: `tray/mod.rs` → `crate::discovery::{ensure_daemon, send_stop, locate_daemon_exe…}` · `commands/popout.rs` → `daemon_client::DaemonClient`
  - N→L 35: `commands/layout.rs` → `layout::LayoutState` 외 17
  - commands/* 핸들러를 L로 옮겨도 순환은 남는다. 남는 참조 = `daemon_client/{connection,mod}.rs` → `output_router::OutputRouter`·`output_channel::WindowChannelRegistry`, `commands/agent.rs` → `output_channel`
- **L↔C**
  - L→C 7: `layout/commands.rs` → `engram_dashboard_command::declare_commands`
  - C→L 2: `view_commands.rs` → `layout::commands::COMMAND_SPECS`
- **C↔N**
  - C→N 14: `command_delivery.rs` → `engram_dashboard_net::frame_port::FrameSink` · `commands/view_bus.rs` → `DaemonClient`
  - N→C 46: `protocol/src/ids.rs` → `engram_dashboard_command::RequestId`(crate 수준) · `connection_core.rs` → `command_delivery::deliver`
- **C↔A**
  - C→A 8: `control/commands.rs` → `agent::manager::AgentManager`·`agent::commands::make_table`
  - A→C 5: `agent/src/commands.rs` → `declare_commands`(컴파일 시점 crate 의존)
- **N↔A**
  - N→A 61
  - A→N 1: `control/priming.rs:276` `engram_dashboard_discovery::find_install_root`가 이 순환의 유일한 근거다
- **N↔M**
  - N→M 11: `control/registry.rs` → `messaging::envelope::DeliveryObserver` · `control/mcp_server.rs` → `super::ingress::handle_send`
  - M→N 5: `control/ingress.rs` → `super::registry::ControlRegistry` · `messaging_host.rs` → `control::mcp_server::MessagingSlot`·`status_fanout::DaemonStatusSink`
- **순환 아님:** C→M 2(역방향 0) · M→A 10(역방향 0) · L→A 2(역방향 0) · 모든 도메인→B

#### 간선별 근거 (흐르는 것)
- **L→C 7:** 전부 `layout/commands.rs`(L의 명령 어댑터) — `CommandError` · `CommandFuture` · `CommandHandler` · `CommandTable` · `ErrorCode` · `blocking_handler` · `declare_commands`(타입 + 매크로)
- **L→N 14:**
  - `tray/mod.rs`·`tray/actions.rs`: discovery 함수(`daemon_status` · `default_data_dir` · `send_stop` · `ensure_daemon` · `locate_daemon_exe` · `StopOutcome`)
  - `commands/popout.rs`: `commands::layout::{RouterSubs, TauriEvents}` · `DaemonClient`
  - `layout/apply.rs`: `protocol::AgentBackendKind`
  - `ui_settings.rs`: `discovery::default_data_dir`
- **L→A 2:**
  - `layout/apply.rs` → `agent::commands::llm_creation_refusal`
  - `layout/commands.rs` → `agent::commands::normalize_cwd` — L의 어댑터가 A의 어댑터를 부른다(표시)
- **L→B 1:** `ui_settings.rs` → `base::logging::mask_secrets`
- **C→L 2:** `view_commands.rs` → `layout::MAIN_WINDOW_LABEL`·`layout::commands::COMMAND_SPECS`(카탈로그 취합)
- **C→A 8:**
  - `command_delivery.rs`: `agent::commands::CATALOG_VERSION` · `types::CLI_CONTROL_READ_TIMEOUT_SECS`
  - `control/agent.rs`: `CLI_EXE_NAME` · `CLI_GROUP_AGENT`
  - `control/commands.rs`: `RosterChanged`(trait impl :180) · `make_table` · `AgentManager`
  - `view_commands.rs`: `agent::commands::COMMAND_SPECS`
- **C→M 2:** `control/agent.rs`·`control/catalog.rs` → `super::ingress::ControlQueryResult`(공유 결과 타입)
- **C→N 14 (+5u):**
  - `net::frame_port::{ConnId, Frame, FrameError, FrameSink}` — `command_delivery.rs`·`command_roster.rs`
  - `protocol::{AgentEvent, CommandListEntry, AgentCommand, RequestId}`
  - `control/commands.rs` → `mcp_server::{CommandTableSlot, RosterBroadcastSlot}`
  - `commands/view_bus.rs` → `DaemonClient` · `connection::shell_owner_advert`
- **N→L 35:**
  - `commands/layout.rs` 18: layout 타입 · `apply` · `layout::commands::LayoutPorts` · `output_router` · `commands::popout::{PopupCounter, TauriWindowHost}`
  - `commands/settings.rs` 8: `ui_settings::*` 함수
  - `commands/tray.rs` 1: `tray::actions`
  - `commands/agent.rs` 1: `output_channel::WindowChannelRegistry`
  - `daemon_client/connection.rs` 4 · `daemon_client/mod.rs` 2: `OutputRouter` · `WindowLabel` · `WindowChannelRegistry`
  - 글롭 재수출 1
- **N→C 46:**
  - `connection_core.rs` 12: `command_delivery::{CommandDeliveries, LocalCommands, OutcomeLanding, deliver, ENTRANCE_SOCKET}` · `CommandRoster` · `control::catalog::merge` · `command::{CommandDecl, CommandError, ErrorCode, OwnerToken}`. C의 `control::agent::RosterBroadcast`를 `RosterFanout`에 impl한다(`connection_core.rs:1960`)
  - `control/mcp_server.rs` 14: `agent::handle_agent` · `catalog::{handle_call, handle_list, relayed}` · `CommandBus`로 디스패치
  - `agent_conn.rs` 3
  - `protocol` 5(crate 수준 — `protocol/src/messages.rs`: `CommandEnvelope` · `CommandReply` · `OwnerToken` · `CommandDecl`; `protocol/src/ids.rs`: `RequestId`)
  - `daemon_client/connection.rs` 8: `CommandEnvelope` · `OwnerToken` · `inbound::{InboundReceiver, InboundSlot}`
  - `daemon_client/mod.rs` 2 · `bin/engram.rs` 1 · 글롭 재수출 1
- **N→A 61:**
  - `connection_core.rs` 25: `AgentManager` · `profile::*` · `preset::Preset` · `backend::can_resume_profile` · `types::*` 싱크·이벤트 · `failure::AgentFailureKind`
  - `bin/engram.rs` 12: `CLI_*` 어휘 상수
  - `control/mod.rs` 7+2: `ControlChannel` trait impl :154 · `ProvisionError` · `ToolGrant` + 상대 `mcp_config`/`priming`
  - `agent_conn.rs` 7: `OutputSink` impl :67
  - `status_fanout.rs` 5: `StatusSink` impl :47
  - `control/mcp_server.rs` 2 · `control/registry.rs` 1
- **N→M 11:**
  - `control/mcp_server.rs` 6: `MessagingService` · `Entrance` · `ingress::{ControlCommand, SendContract, handle_messages, handle_send}`
  - `control/registry.rs` 4: `SenderIdentity` · `DeliveryObserver` 등
  - `connection_core.rs` 1: `EnvelopeFormat`
- **N→B 2:** `discovery/src/lib.rs`·`net/src/portfile.rs` → `base::platform::pid_alive_with_start_time`
- **A→C 5:** `agent/src/commands.rs`(A의 명령 어댑터 · 표시)
- **A→N 1:** priming → `discovery`
- **A→B 7:**
  - `mask_secrets` ×3: codex `decoder.rs` · codex `transport.rs` · `stdio.rs`
  - `platform::{process_creation_time, pid_alive_with_start_time, child_pids}`
- **M→A 10:**
  - `messaging_host.rs` 8: `AgentManager` · `TurnObservations` · `types::*` · `StatusSink` impl :862
  - `control/ingress.rs` 2
- **M→N 5 (+1u):**
  - `control/ingress.rs` → `registry::{BoundIdentity, ControlRegistry}`
  - `messaging_host.rs` → `control::mcp_server::MessagingSlot` · `ControlRegistry` · `status_fanout::DaemonStatusSink`
  - M이 N의 타입에 메시징 포트를 impl한다(`messaging_host.rs:225` `impl ControlPlanePort for ControlRegistry`)

#### 조립실 (행렬 제외)
- **daemon `lib.rs`** — B를 포함해 여섯 도메인 전부를 끌어온다.
  - 한정 경로: A 14(`AgentManager` · `persistence::File*Store` · `Preset/ProfileRegistry` · `SessionTracker` · `NoopControlChannel`) · N 15(`net::{instance, portfile, ws, frame_port}` · `discovery::default_data_dir` · `PROTOCOL_VERSION`) · B 2
  - 상대 경로 C: `command_delivery::{CommandBus, CommandDeliveries, BusSweeper}` · `CommandRoster` · `control::commands::make_daemon_table` · `control::agent::RosterBroadcast`
  - 상대 경로 N: `mcp_server::*Slot` · `start_mcp_server` · `ControlRegistry` · `RosterFanout` · `MultiViewState` · `AgentConnections`
  - 상대 경로 A: `control::priming::FilePrimingProvider` · `mcp_config::sweep_stale_configs`
  - 상대 경로 M: `messaging_host::{FlushWiring, ChannelIdleNotifier, FlushMsg}`
- **daemon `main.rs`:** `engram_dashboard_daemon::run`만 부른다.
- **src-tauri `lib.rs`:**
  - L 13: `LayoutState` · `layout::commands::make_table`/`CATALOG_VERSION` · `OutputRouter` · `WindowChannelRegistry` · popout 헬퍼 · tray `show/hide_main_ui`
  - N 6: `DaemonClient::new_real_with_owned_runtime` · `commands::layout::command_ports` · `commands::settings::sweep_dead_window_entries` · discovery
  - C 3: `ViewCommandBridge` · `TauriViewDispatch` · `hidden_window_labels`
  - B 1: logging
  - `generate_handler![commands::…]`의 상대 `commands::` 참조 약 56개는 스크립트가 세지 않았다.
- **src-tauri `main.rs`:** `engram_dashboard_lib::run`만 부른다.

#### 여러 crate에 걸친 도메인
| 도메인 | crate |
|---|---|
| N | 6개 — protocol · transport · net · discovery · daemon · src-tauri |
| C | 3개 — command · daemon · src-tauri |
| A | 2개 — agent · daemon(`mcp_config`·`priming`) |
| M | 2개 — messaging · daemon(`messaging_host`·`ingress`) |
| L | src-tauri 하나 |
| B | base 하나 |

#### 매핑이 안 맞는 곳
1. **`src-tauri/src/commands/{layout,settings,tray}.rs`(N으로 매핑)는 L의 Tauri IPC 어댑터다.** N→L 35건 중 27건이 여기서 나오고, import에 연결 로직이 없다. (가능성 높음)
2. **daemon `connection_core.rs`(5,036줄, N)는 세 도메인이 섞여 있다.** (가능성 높음)
   - A→wire 변환: `agent_info_to_wire` :544 · `core_report_to_wire` :807 · `output_event_to_wire` :739
   - AgentCommand 디스패치 정책: `inbound_lane` :191 · `dispatch_order` :228
   - C trait impl: `RosterBroadcast` :1960
   - 로그 유틸: :820 · :1913 · :1919
3. **`agent::types`가 CLI 어휘(`CLI_GROUP_MAIL` · `CLI_MAIL_VERBS` · `RENAME_OUTCOME_*` · `CLI_EXE_NAME` …)를 쥐고 있다.** 소비자는 N(`bin/engram.rs` ×12 · `mcp_server` · `control/mod.rs`)과 C(`control/agent.rs` · `command_delivery`)다. 내용상 A가 아니라 N 또는 C다. (가능성 높음)
4. **`tray/*`(L)가 데몬 수명 관리를 한다**(`ensure_daemon` · `send_stop` · `locate_daemon_exe`). 이는 N의 내용이다. (가능성 높음, 일부)
5. **`control/registry.rs`(N)는 에이전트 신원·토큰 레지스트리이면서 메시징 역할도 한다.** 메시징 `DeliveryObserver`를 impl하고, M이 이 타입에 `ControlPlanePort`를 impl한다. M 또는 A일 수 있다. (불확실)
6. **`control/mod.rs`(N)의 주 내용은 A의 `ControlChannel` 포트 어댑터다**(`DaemonControlChannel`, :154). (가능성 높음)
7. **`ControlQueryResult`는 M의 `control/ingress.rs`에 정의돼 있지만 C가 쓴다**(`control/agent.rs`·`control/catalog.rs`). 공유 타입이 잘못 놓였고, C→M의 유일한 출처다. (가능성 높음)
8. **`output_router.rs`·`output_channel.rs`(L)를 N의 연결 계층이 쓴다**(`daemon_client/connection.rs`). 이 둘을 옮기면 어댑터가 아닌 N→L 참조가 사라진다. (정말 L인지 불확실)
9. **`control/priming.rs`(A)가 A→N의 유일한 간선이다**(`find_install_root`). (확실)

#### 모름 / 한계
- 임포트한 타입의 메서드 호출과 추론된 타입 사용은 세지 않았다. 수치는 import 수준의 결합이다.
- `lib.rs`·`mod.rs` 밖의 상대 경로(`crate::` 없는 것)는 전수 확인하지 않았다. 드물다고 보지만 확인 안 됨.
- `test-harness`·`test-support` cfg 항목을 테스트 코드로 보고 뺀 것은 내 판단이다.
- 항목 수준 재수출 사슬은 따라가지 않았다. 경로에 나타난 crate로 셌다(예: `daemon::KeepaliveConfig` → N). src-tauri의 `pub use engram_dashboard_discovery as discovery`만 N으로 풀었다.

### A.2 프론트 도메인 의존(초안 매핑)

기준 트리 = master `a226f63`. 정적 분석만 했다(테스트 미실행).

**파일 → 도메인 매핑(주어진 것 그대로 적용):**
- 렌더링 R = `components/{slot,agent,ui}` · `pages` · `theme/ThemeManager*` · `themes.ts` · `i18n` · `lib` · `store/{themeStore,chatStyleStore}`
- 화면 배치 L = `components/layout` · `store/{viewStore,monitoringPickerStore}` · `theme/uiSettings`
- 명령 C = `commands/{registry,dispatch,contributions,keybindings,viewCommandBridge,slotMenu,enumArg}`
- 명령 어댑터(표기 `*`): `commands/{agentCommands,presetCommands}` → A* · `commands/{tabCommands,slotCommands,slotContentCommands}` → L* · `commands/renderModeCommands` → R*
- 연결 N = `api/*` · `util/retryInvoke` · `store/eventBus`(섞인 파일)
- 에이전트 실행 A = `store/agentStore`
- 바닥 B = `util/basename`
- 조립실(행렬 제외) = `main.tsx` · `App.tsx`
- 분석 제외 = `*.test.*` · `lab/` · CSS
- GEN = `src/` 밖의 생성 바인딩(매핑에 없는 것 — 섞인 파일 절 참조)

#### 요점

1. **N 은 허브다. N 이 걸린 순환은 전부 `store/eventBus.ts` 파일 하나를 지난다.** N 에서 다른 프론트 도메인으로 나가는 간선은 이 파일뿐이다.
   - →A `store/eventBus.ts:7` (useAgentStore)
   - →C `:6` (registry list/run — `:86` 에서 `__engramCmd` 설치)
   - →L `:8` (initMainWindowFromBackend, subscribeViewEvents)
   - eventBus 를 N 에서 떼어 내면 N 의 대외 간선은 0 이 된다. 남는 것은 `api/layoutTypes.ts` → GEN 13(type)뿐이다. (확실)
2. **실제로 가장 큰 순환은 R↔L 이다.**
   - L→R 17건 중 11건이 `components/layout/LayoutLeaf.tsx:11-22` 에 몰려 있다 — 모든 슬롯 컴포넌트를 마운트하는 자리다.
   - R→L 7건 = `pages/PopoutPage.tsx:22-26` 이 WindowLayout 을 마운트 · AgentList·AgentMonitoringPicker 가 `viewStore.assignAgent` 호출.
   - 스토어가 컴포넌트 폴더에 기대는 L→R 도 둘 있다 = `store/viewStore.ts:43` → `components/slot/renderMode` · `theme/uiSettings.ts:19,21` → themeStore·ThemeManager. (확실)
3. **레이아웃 백엔드(L)는 `api/` 를 거치지 않는다 — L 파일들이 `invoke`/`listen` 을 직접 부른다.** 직접 부르는 곳 = `store/viewStore.ts` · `components/layout/*` 4개 · `theme/uiSettings.ts` · `commands/{tabCommands,slotCommands}.ts`. `api/` 를 타는 것은 데몬 경로(A/N 백엔드)뿐이다(`agentClient` → `forward_daemon_command`). (확실)
4. **C → 어댑터 간선은 side-effect import 뿐이다.**
   - `commands/contributions.ts:8-13` 이 어댑터 6개를 side-effect import 한다.
   - 어댑터는 전부 `registry`/`slotMenu`/`enumArg` 를 static import 한다 → 등록 순환이다.
   - 어댑터를 동사 도메인에 접으면 A↔C · L↔C · R↔C 순환이 된다. (확실)
5. **A(agentStore)는 거의 순수한 싱크다.** 밖으로 나가는 간선은 `api/types.ts` type import 하나뿐이다(`store/agentStore.ts:3`). 들어오는 쪽 = eventBus(N)가 쓰고 · 조립실 · `commands/agentCommands` 가 읽는다. (확실)

#### 측정 방법

- 대상 = `src/` 아래 테스트가 아닌 `.ts/.tsx` 83개. 제외 = `lab/` · `*.test.*` · `.d.ts` · CSS.
- node 스크립트가 `import` · `export … from` · `import()` 를 파싱했다. 상대경로와 `@/` 별칭(`tsconfig` `paths`, `vite` alias)을 해석했다.
- 세는 단위 = import **문장** 수(심볼 수 아님).
- 종류를 나눠 셌다: static / type / side-effect / dynamic.
- Tauri 호출 지점은 `invoke(`/`listen(` 을 grep 했고, 이름이 상수로 들어간 것은 그 상수를 되짚어 풀었다.

#### 행렬(원본·접은 것)

**원본** (행 = 출발 · 괄호 = 자기 도메인 안 간선 · A*/L*/R* = 명령 어댑터 · GEN = 생성 바인딩):

| from\to | R | L | C | N | A | B | A* | L* | R* | GEN |
|---|---|---|---|---|---|---|---|---|---|---|
| R | (49) | 7 | 2 (1 type) | 22 (12 type) | 6 | 4 | – | – | – | 2 type |
| L | 17 | (20) | 1 | 17 (10 type) | 1 | – | – | – | – | – |
| C | – | – | (7) | 1 type | – | – | 2 side | 3 side | 1 side | – |
| N | – | 1 | 1 | (18) | 1 | – | – | – | – | 13 type |
| A | – | – | – | 1 type | – | – | – | – | – | – |
| A* | 2 | – | 5 | 4 (1 type) | 1 | – | – | – | – | – |
| L* | 3 | 4 | 6 | 3 (static 1·type 1·dynamic 1) | – | – | – | – | – | – |
| R* | 2 | 1 | 2 | – | – | – | – | – | – | – |
| B | – | – | – | – | – | – | – | – | – | – |

**접은 것** (어댑터를 동사 도메인에 합침 · s = side-effect · t = type):

| from\to | R | L | C | N | A | B |
|---|---|---|---|---|---|---|
| R | – | 8 | 4 | 22 | 6 | 4 |
| L | 20 | – | 7 | 20 | 1 | – |
| C | 1s | 3s | – | 1t | 2s | – |
| N | – | 1 | 1 | – | 1 | – |
| A | 2 | – | 5 | 5 | – | – |

- side-effect import = C 안에 6건(`contributions.ts`) + 조립실 1건(`App.tsx:13`).
- dynamic import = 1건. 실제로는 type 전용 인라인이다 — `commands/tabCommands.ts:196` `import('../api/layoutTypes').SlotContent`.

#### 순환

양방향마다 가장 작은 근거 하나씩 적는다.

- **R↔L** (실제 런타임 순환)
  - R→L = `pages/TreePage.tsx:2` → ConnectionNotice
  - L→R = `theme/uiSettings.ts:21` → themeManager · `store/viewStore.ts:43` → renderMode
- **L↔N** (실제 순환)
  - L→N = `store/viewStore.ts:44` → retryAsync
  - N→L = `eventBus.ts:8` → viewStore
- **C↔N**
  - N→C = `eventBus.ts:6` → registry (런타임)
  - C→N = `commands/slotMenu.ts:8` — type 전용(SlotContent)
  - 한쪽이 type 전용인 순환이다.
- **A↔N**
  - N→A = `eventBus.ts:7` (런타임)
  - A→N = `agentStore.ts:3` — type 전용
- **R↔C**
  - R→C = `components/slot/SlotContextMenu.tsx:10` → fireAndForget
  - C→R = R* 경유뿐(`contributions.ts:11`, side-effect). 원본 C→R 은 0 이다.
- **L↔C**
  - L→C = `components/layout/LayoutLeaf.tsx:20` → buildSlotMenu
  - C→L = L* 경유뿐(side-effect)
- **A↔C** (접은 행렬에서만)
  - A*→C = `agentCommands.ts:13` → register
  - C→A* = `contributions.ts:13` (side-effect)
- **순환 없음** = B(순수 잎). A 는 eventBus 를 빼면 싱크다.

#### 간선별 근거

도메인 사이 간선만, 대표 지점만 적는다.

- **R→N 22건** — `components/slot/TerminalSlot.tsx:6-8`: agentClient · FRAME_TAG_TERMINAL_BYTES · OutputSubscription.
  - `DomSlot`·`RichSlot` 도 같은 모양이다.
  - `AgentList.tsx:27` · `PresetPalette.tsx:17` 은 eventBus 의 refresh* 를 import 한다.
  - 22건 중 12건은 `api/types`·`layoutTypes` type import 다.
- **R→A 6건** — useAgentStore: `AgentList` · `AgentMonitoringPicker` · `DomSlot` · `PresetPalette` · `RichSlot` · `TerminalSlot`.
- **R→B 4건** — basename: `AgentList.tsx:28` 외 slot 파일 3개.
- **R→GEN 2건** — `components/slot/structuredAccumulator.ts:18-19` → protocol crate 바인딩(StructuredEvent, TurnOutcome).
- **L→N 17건** — 10건이 layoutTypes type import 다.
  - 나머지 = retryAsync(`useSplitDrag.ts:22` · `WindowLayout.tsx:21` · `uiSettings.ts:20` 등) · agentClient(`ConnectionNotice.tsx:19`).
- **L→A 1건** — `LayoutLeaf.tsx:10`.
- **L*→L 4건** — viewStore/monitoringPickerStore: `slotCommands.ts:15` · `slotContentCommands.ts:10-11` · `tabCommands.ts:12`.
- **A*→N** — `agentCommands.ts:9,11` (agentClient, refreshProfiles) · `presetCommands.ts:9`.
- **어댑터 전부 →R** — i18n `t`.
- **R*→L** — `renderModeCommands.ts:22` → viewStore setRenderMode. 프론트 전용 상태로 보인다. (가능성 높음)

#### 백엔드 도달 표

| 프론트 도메인 | 백엔드 도메인 | 이름 | 지점 |
|---|---|---|---|
| N | N | invoke: `forward_daemon_command` · `subscribe_output` · `request_replay` · `daemon_connection_state` · `daemon_connect`/`daemon_ensure` · `daemon_close` · `daemon_start`/`daemon_status`/`daemon_stop` · `discover_daemon` · `read_daemon_info`<br>listen: `daemon-connection-state` | `api/tauriTransport.ts:162,257,352,414,442,459,487` · `api/daemonControl.ts:61,101,109` · `api/wsTransport.ts:218,230` |
| N | A | listen: `agent-list-updated` · `status-changed` · `restore-result` · `profile-list-updated` · `preset-list-updated` | `api/tauriTransport.ts:196-237` |
| N (eventBus) | A (터널 경유) | ListAgents · ListProfiles · ListPresets + on* 구독 | `store/eventBus.ts` |
| R | A (N 터널 `forward_daemon_command` 경유) | WriteStdin(`DomSlot`, `TerminalSlot:388`) · Resize(`TerminalSlot:124…`) · spawnProfile/reparentProfile/deleteProfile(`AgentList.tsx:178,258`) | wire 명령 정의 = `api/protocolClient.ts:858-984` |
| R | N (api 경유) | subscribeOutput → `subscribe_output` + `request_replay` | `TerminalSlot.tsx:278` · `DomSlot.tsx:121` · `RichSlot.tsx:169` |
| R | L (viewStore 경유) | `assign_agent` | `AgentList.tsx:212` · `AgentMonitoringPicker.tsx:75` → `viewStore.ts:211` |
| L | L | invoke: `create_tab` · `close_tab` · `switch_tab` · `create_window` · `close_window` · `split_slot` · `set_split_ratio` · `rename_tab` · `focus_slot` · `close_slot` · `assign_agent` · `set_slot_content` · `move_slot_to_window` · `list_tabs` · `get_view`<br>listen: `layout:updated` · `window:tabs-updated` | `store/viewStore.ts:186-225,384-425` |
| L | L | invoke: `report_ui_metrics` · `get_view` · `report_window_canvas` · `list_tabs`<br>listen: `window:tabs-updated` | `components/layout/uiMetricsReport.ts:49` · `useSplitDrag.ts:281` · `windowCanvasReport.ts:139` · `WindowLayout.tsx:71,83,124` |
| L | L | invoke: `get_ui_settings`<br>listen: `ui:settings-updated` | `theme/uiSettings.ts:30,24,125` |
| C | C | invoke: `report_view_commands` · `report_command_outcome`<br>listen: `command:request` | `commands/viewCommandBridge.ts:21-25,64,116,136` |
| L* | C | invoke: `resolve_spatial` | `commands/slotCommands.ts:128` |
| L* | C + A + L | invoke: `spawn_into` | `commands/tabCommands.ts:214` |
| L* | L (viewStore 경유) · A (agentClient 경유) | L = split · close · focus · move · setSlotContent<br>A = SpawnByCwd · Kill | `slotContentCommands.ts` · `slotCommands.ts` |
| A* | A (N 터널 경유) | SpawnByCwd · create*Profile · renameProfile · create/delete/rename/listPresets | `agentCommands.ts` · `presetCommands.ts` |
| A (agentStore) | 없음 | – | – |
| 조립실 | N · A | bootstrapDaemonIfNeeded(`daemon_start`…) · getAgents | `App.tsx:43` |

Tauri 플러그인 `plugin-dialog open` 은 OS 파일 선택 창이라 백엔드 도메인이 아니다. 쓰는 곳 = `agentCommands.ts:5` · `presetCommands.ts:6` · `slotContentCommands.ts:6`.

#### 조립실

- **`main.tsx`**
  - import = App · themeStore · ThemeManager · chatStyleStore · agentStore
  - `main.tsx:30` 에서 dev 핸들 `window.__engram` 을 설치한다.
- **`App.tsx`**
  - A = agentStore
  - C = contributions(side-effect) · installKeybindings · installViewCommandBridge
  - L = uiSettings · AppLayout
  - N = eventBus init/refresh · clientFactory(agentClient, bootstrapDaemonIfNeeded) · retryAsync
  - R = TreePage · PopoutPage · nativeScrollActivity · RootErrorBoundary
- **조립실 fan-out** = A 2 · C 3 · L 2 · N 3 · R 7.

#### 섞인 파일

- **`src/` 밖의 생성 바인딩 — 매핑에 없다.**
  - `api/layoutTypes.ts:5-21` → `src-tauri/bindings/` 13개 type. 레이아웃 모양이라 사실상 N 안에 있는 L 의 wire 타입이다.
  - `structuredAccumulator.ts:18-19` → protocol crate 바인딩.
  - 둘 다 GEN 으로 따로 셌다. (확실)
- **`store/eventBus.ts` — 섞인 파일이다.** 한 파일이 네 가지를 한다: N 구독 소유 · A 쓰기 · L 뷰 구독 초기화 · C 의 LLM 표면(`__engramCmd`, `:86`) 설치. (확실)
- **`api/clientFactory.ts:29,31`** — 전역 핸들 `__ENGRAM_AGENT__` · `__ENGRAM_DAEMON__` 도 설치한다(N). (확실)
- **`invoke` 를 직접 부르는 컴포넌트** — `components/layout/{uiMetricsReport,useSplitDrag,windowCanvasReport,WindowLayout}`.
  - 간선은 L → L 백엔드이고 N 을 우회한다.
  - L 에 귀속시키자는 것은 해석이다.
- **C/A 백엔드에 닿는 L 어댑터** — `commands/slotCommands.ts`(L*) → C 백엔드 `resolve_spatial` · `tabCommands.ts`(L*) → `spawn_into`(C·A·L 에 걸친다). (확실)
- **컴포넌트 폴더에 사는 공유 상태** — `components/slot/renderMode.ts`(R) 를 `store/viewStore.ts`(L)와 R* 어댑터가 import 한다. 사실상 R 에 사는 공유 도메인 상태다(해석).
- **L 로 센 테스트 헬퍼** — `components/layout/testing/rects.ts` 는 이름이 `*.test.*` 가 아니어서 L 로 셌다(N 으로 type 간선 1). (확실)
- **주석에만 있는 언급** — `components/slot/richBranding.ts` 는 agentClient 를 주석(`:56`)에서만 언급하고 실제 import 는 없다. (확실)

#### 모름

- `viewStore.setRenderMode`·dom-mode 액션이 백엔드에 영속되는지 — 프론트 전용일 가능성 높음, 검증 안 함.
- `RichSlot` 의 입력 경로 — writeStdin 을 찾지 못했다(읽기 전용일 가능성 높음), 검증 안 함.
- 정규식 파서는 인라인 `import('…')` 타입 참조를 찾은 1건 말고는 못 본다. 수치는 문장 수이지 심볼 수가 아니다.
- 스크립트·원본 간선 덤프(스크래치): `…\scratchpad\deps.cjs` · `edges.json`.

