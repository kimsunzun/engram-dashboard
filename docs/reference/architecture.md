# Architecture

What each module does, who it talks to and over what, and the runtime invariants. Extracted from the current code — if the invariants and the code disagree, do not quietly fix the invariants to match the code. Work out whether the code is wrong (a regression) or the intent changed, and report it. Canonical member list = the root `Cargo.toml`; canonical boundary and isolation gate of each crate = the header of that crate's `src/lib.rs`; canonical per-file responsibility = that file's header and the `// ADR-NNNN` anchor. Design principles are in `CLAUDE.md` 「설계 원칙」.

## Connection diagram

```text
[앱 프로세스 engram-dashboard.exe]
  프론트(React, 창마다 한 벌)
    │  Tauri invoke · event · Channel
    ▼
  셸(src-tauri)
    │  WS 127.0.0.1 — 연결 1개, daemon.json 으로 찾는다
    │  (데몬 띄우기 = WMI 로 Job 밖에서 · 끄기 = StopDaemon 프레임)
    ▼
[데몬 프로세스 engram-dashboard-daemon.exe]
  net(WS 행) · agent(런타임) · 명령 버스 · messaging(우편) · 제어 평면(HTTP)
    │  PTY(터미널 모드) · stdio(JSON 모드)
    ▼
[에이전트 N — claude · codex · 셸]
    │  engram.exe(CLI) · MCP 툴 → HTTP /control/* · /mcp (Bearer 토큰)
    └──▶ 데몬 제어 평면
```

Crate dependencies (arrow = depends on, the production graph):

```text
src-tauri ─▶ protocol · command · base · platform · transport(ws)      (agent 는 의존하지 않는다)
daemon    ─▶ agent · protocol · net(server) · messaging · command · base · platform
cli       ─▶ agent · command
agent     ─▶ command · base · platform
net       ─▶ protocol · platform   (platform only behind the server feature)
base · platform · command · messaging · transport · protocol ─▶ 워크스페이스 의존 0
```

## Modules

**Three processes.** Shell (app) · daemon · agent. The daemon owns the agents, so agents stay alive when the app is turned off and on.

- **Frontend (`src/`)** — only renders the screen. Communication with agents and the daemon goes through the shell over a single `agentClient` (`ProtocolClient` + `TauriTransport`) — commands = invoke (`forward_daemon_command` and others), output = Tauri Channel, broadcast = Tauri event. For state the shell owns (layout · window · settings · restore), the frontend invokes shell commands directly. The control surface = the `window.__engramCmd` registry (`src/commands/`), and a bus command comes down from the shell to the webview and rides the same `registry.run`.
- **Shell (`src-tauri`)** — the daemon's resident client. It holds the window · tray · layout (the authority for windows · tabs · slots) · shell settings · screen state persistence · finding/launching/shutting down the daemon (`discovery`). It does not own agents. It talks to the daemon over one WS connection (`daemon_client` — one however many windows there are) and relays output frames to a Channel per window (`output_router`). It is registered on the daemon command bus as the owner of the layout commands (`window/tab/slot`). The only window is main, and pop-outs are created at run time.
- **daemon** — the agent host that owns `AgentManager`, and the assembly room. It accepts shell connections in the WS accept loop, runs the command bus registry and delivery (`command_roster` · `command_delivery`), and plugs the real adapters into the messaging kernel (`messaging_host`). The control plane (`control`) = one HTTP server carrying MCP (`/mcp`) and the CLI route (`/control/*`) — an agent's identity comes only from the Bearer token. Data folder layout (`data_dir`) · usage query (`usage_service`) are here too.
- **agent** — the agent runtime (Tauri import 0). Session lifetime · output pump · `transport` (PTY · stdio) · `backend` (claude · codex · shell · a gemini stub — backend-specific knowledge lives only here) · turn observation · waiting input · profile persistence · `agent.*` command declarations. Output and state go out only through the `OutputSink`/`StatusSink` traits.
- **cli** — `engram.exe`. A spawned agent attaches to the daemon control route over HTTP with `ENGRAM_TOKEN` · `ENGRAM_CONTROL_URL`. It only authenticates and delivers, and the daemon also produces the help screen. bin name = agent `CLI_EXE_NAME`.
- **protocol** — the shell ↔ daemon wire contract (messages · output binary frame codec · ts-rs bindings). No domain logic.
- **command** — command bus tooling (envelope · error · declaration macro · table · routing). There are zero commands here, and a command is declared next to its producer (`agent.*` = agent, `mail.*` = daemon, `window/tab/slot` = shell, webview commands = frontend).
- **net** — the daemon's network layer (WS upgrade · Origin · token handshake · connection lifetime · single writer · keepalive · fan-out · single-instance guard · `daemon.json`). Everything sits behind the `server` feature; without it only `auth` remains, a re-export of protocol's handshake frame. Only the daemon depends on net — the shell's edge was cut by ADR-0310 decision 9.
- **messaging** — the mail kernel between agents (delivery · parking · bounced · ledger). Zero workspace dependencies; the only seam is the port traits the kernel owns, and the daemon holds the real adapters.
- **base** — the bottom foundation leaf crate. General-purpose helpers with zero domain knowledge (data file read decision · atomic write · logging · clock seam · panic containment and so on). The canonical entry condition = the header of its `lib.rs`.
- **platform** — the OS layer leaf crate. Operational code that varies by OS (process · Job Object · launching · window · input · file lock) is placed only here.
- **transport** — reusable transport lib (ADR-0177). The shell depends on it (`ws` feature) for its daemon link `daemon_client/link/` (stage 3 3-2 · ADR-0310); the shell's daemon connection runs on it in every build (driver = `daemon_client/connection.rs`). The daemon attaches in 3-3.

## Runtime invariants (런타임 불변식)

The 「리뷰」 column = what a reviewer checks in a change that touches that area. ★ = check for any agent·daemon runtime change, regardless of area.

| # | Invariant | Review (리뷰) |
|---|---|---|
| I1 | **kill causality (2 verbs):** `transport.shutdown()` (child.kill+wait → group owner `GroupOwner::terminate` → master drop) → `core.join_pump(5s)`. master drop → reader EOF → pump break → `core.finish`. The first line of `kill_agent` is `session.begin_retire()` — it only sets the retirement field and does not withdraw resources, so the causality above is unchanged. (ADR-0001 · ADR-0262) | kill · transport changes |
| I2 | **finalize once:** `OutputCore.finalized.swap(AcqRel)` — terminal transition · notification exactly once (pump alone). (ADR-0005) | ★ |
| I3 | **Lock order:** the sessions RwLock is released right after the Arc clone. No external call while holding the status lock. emit clones subscribers and then sends without a lock (「락 없이 send」). Input path = `input_order → replay → {QueuedInputs, InputsPendingTable}` — the registry · table are leaves. The pump · emit · `finish` do not take `input_order`. codex link = `Announcer.order → {상태 락, replay → …}`. The intended exceptions to that send without a lock are two (emit holding the session `input_order` · codex `order`), and the reason there is no deadlock is the trait contract that `OutputSink::send` does not block. The two leaf locks (`GateCell.state` · `Recorder.inner`) of claude leftover cleanup (`backend/claude/leftover.rs`) are not held overlapping, and while holding one no other lock · log · emit · OS call is made (exception = a monotonic clock read · one `TerminateProcess`). (ADR-0006 · ADR-0231 · ADR-0234 · ADR-0262) | ★ |
| I4 | **State notification division:** transitional `Exiting` = manager (`enter_exiting`), terminal (`Killed`/`Exited`/`Failed`) = pump alone. The frontend does not decide terminal from `status_changed` but from `agent-list-updated`. (ADR-0005) | ★ |
| I5 | **replay→live:** replay is sent while holding the subscribers lock. The frontend corrects the delivery inversion of the live fan-out with seq continuity (only `마지막+1` is delivered · anything at or below is dropped · what follows a hole is held back). So the sink and the shell relay never skip a seq — an event that could not be carried · an unknown tag = placeholder `Error` (`placeholder_error_frame`) · a frame whose seq cannot be read = cut the connection and replay everything. (ADR-0231) | output path · shell relay · `protocolClient` changes |
| I6 | **Incarnation marker (field name `epoch`):** a new 32-bit random number per incarnation — comparison is match/mismatch only (no deriving order). `agents.json` disk serde alone skips on read + writes a `0` placeholder, and it is not widened to the wire (the `0` on the wire is a legitimate value). The trigger for re-attaching is an observation of the authoritative registry, and the marker is not put in the subscription effect deps. (ADR-0163 · ADR-0164) | ★ |
| I7 | **Ownership split:** transport = master/writer/child/shutdown/group · core = subscribers/replay/seq/status/finalized + the waiting input registry · waiting list table · seal · session = id/cwd/epoch/cols/rows + first-submit latch · `mid_turn` · `delivery_ack` · `input_order`. The only way to change the waiting input registry is the one reduction the core feeds inside the replay lock — the session only reads the same `Arc`. The codex link (`TransportOwned`) holds its own input state under a single state lock and emits list events itself (canonical = `State` of `backend/codex/transport.rs`). The group owner (`GroupOwner`) is one per group, not `Clone` — that is as far as the type enforces. That the link owns the agent group is kept by placement (the group of a usage query child is held by that query's `TreeRoot` — `usage/process.rs`). (ADR-0231 · ADR-0234 · ADR-0235 · ADR-0262 · ADR-0275) | ★ |
| I8 | **Turn observation cleanup = two points only:** the forget of `finish` + the finalize recheck of `emit`. Do not add a third caller — if it is missed, a dead agent stays as 「턴 중」 and mail is blocked up to the 30-minute cap. The stop after an error `last_end_failed` is also withdrawn at these two points, and is filtered not by the progress cursor `last_seq` but by its own cursors (`last_end_seq` · `error_seq`). The 30-minute cap is not attached to `inputs_pending` · `last_end_failed`. The disconnect marker `Structured{kind:"interrupted"}` is not counted as progress by the claude classifier. (ADR-0127 · ADR-0231 · ADR-0238 · ADR-0243) | turn classification · `output_core` changes |
| I9 | **Waiting input state = one stream of ring events (`StructuredEvent::QueuedInput`):** the daemon registry and the frontend accumulator use the same reduction rule and both consume the golden `queued_input_golden.json` — if the rule is changed, the golden and the TS edition are changed in the same change. The only waiting input event that enters turn observation is claude `Delivered`, and for codex all of them are `None`. The user speech bubble that the codex link synthesizes and the boundary after a late receipt are emitted with `emit_without_turn_observation` — changing them to a normal emit makes mail blocked or flowing. (ADR-0231 · ADR-0235 · ADR-0241) | waiting input · codex link changes |
| I10 | **Registration order:** the sessions insert comes before the pump start — if reversed, a session that exits immediately is not reaped (the reaper test catches it). (ADR-0019) | spawn path changes |
| I11 | **Session id persistence = one point, the first-submit latch, by compare-and-swap:** the backend session id is persisted to the profile only when the incarnation's first-submit latch commits (it is not an overwrite `observe_session_id`). Do not revive other record paths (issue at spawn and persist, etc.) — a 0-turn id would be persisted and the next activation would fail. Exception = claude `/clear` tracking. In codex JSON mode the link commits with `FirstTurnSink` from the echo of the peer's first user message (it is not persisted before the first turn). A save failure is recorded in the in-memory registry and marked dirty — the next successful save carries it. (ADR-0226 · ADR-0233 · ADR-0291) | session id · profile persistence changes |

### Session restore · activation (세션 복원 · 활성화)

- Restore relies on the single backend session id stored in the profile — who issues it is up to the backend (claude = we issue it with `--session-id` · codex = received from the thread creation response). (ADR-0008 · ADR-0185)
- Activation is resume only. On failure it does not fall back to a new conversation; it ends in the observed exit state and records the failure in the profile. (ADR-0082 · ADR-0172)
- The activation entries (bus `wake_existing` · WS `SpawnProfile`) and boot restore call the same decision `backend::can_resume_profile`. The explicit WS `resume=true` request does not go through that decision.
- The published `Failed` is only `TerminalReason::Error` (pump panic) — the end point of a resume failure is usually `Exited`, and the failure stands in the activation result (`RestoreOutcome::Failed`) and the profile 「마지막 실패」 axis.
- Per-backend session table = `docs/reference/structure/agent-backend.md`.

## Frontend rules

- Agent · daemon communication passes through a single `agentClient`. Only the state the shell owns (layout · window · settings · restore) uses Tauri invoke/listen directly. `WsTransport` is not used in production (only tests import it).
- The lifetime of subscriptions (callbacks) is owned by `eventBus` through the abstract subscription of `agentClient`, and the side that attaches Tauri `listen` directly owns its own disposer. Do not count the places that attach — find them: `rg "from '@tauri-apps/api/event'" src/`.
- Subscription effect deps = `[viewId, agentId]` (excluding the incarnation marker — I6). Before subscribing `terminal.reset()`.
- The hold caps `HELD_MAX_FRAMES` · `HELD_MAX_BYTES` (`src/api/protocolClient.ts`) are set smaller than the daemon ring caps `REPLAY_MAX_EVENTS` · `REPLAY_MAX_BYTES` — if larger, overflow recovery always ends in truncation. On overflow it goes to `startBuffering`. (ADR-0231)
- The replay boundary = the gen fence success marker (the marker carries the replay head, and flush starts from `max(마지막+1, 머리)`).
- The replay request is made after that window's output Channel is registered (`awaitOutputChannel`) — the shell silently skips the share of an unregistered window.
- The structured view re-checks the waiting list against the daemon at every `'live'` (`ListQueuedInputs` → applied after being delivered up to the snapshot `as_of_seq` · an old answer is dropped by the check generation). (ADR-0231)
- Detaching a Channel is `delete channel.onmessage` (not assigning null) and is a no-op in real Tauri — the frames in the re-registration gap rely on that no-op, so do not make the detach real.
- Terminal resize is debounced at 50ms.

## Windows

- The static window is main only — declared in `src-tauri/tauri.conf.json` but `"create": false`, so the shell creates it at boot at the saved position.
- Pop-outs are created at run time (`/popup` route · `PopoutPage` — it owns the tabs, ADR-0057). label = `slot-popup-<실행 표식>-<n>`, so it changes with every run — what points to the same window across restarts is the persistent id. (ADR-0302)
- All code that creates a window passes through `WebviewEnv::finish` (`src-tauri/src/webview_env.rs` — webview data folder · browser arguments · background · OS frame). On an OS where the app draws the window buttons (currently Windows) the OS frame is withdrawn (`decorations(false)` · `shadow(true)`) and a web title bar stands in its place. (ADR-0054 · ADR-0283)
- The window buttons call registry commands (invoke `window_control`) — the same service as the bus window state commands. OS window title · minimum window size = the shell `chrome` module. (ADR-0300 · ADR-0301 · ADR-0302 · ADR-0303)

## Screen state persistence

- The layout is persisted to `<데이터 루트>\shell\state\state.json`. Boot after a normal exit restores without asking, and after an abnormal exit it copies it aside as `state.crash.json` and starts with the default screen.
- The answer on whether to restore = bus `restore.status` / `restore.answer` · a person = the main window modal. Canonical = `docs/process/S21-storage/trd.md` §6.

## Status

- The regression net is still tied to Windows — the CI runner (the build · test job of `.github/workflows/ci.yml`) and the run scripts are Windows only, and some real-process tests are `#![cfg(windows)]` for the whole file.
