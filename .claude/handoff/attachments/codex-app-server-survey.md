# Synthesis for adversarial review — can `codex app-server` be our Phase 2 codex backend?

## Who is asking
A Rust desktop app: Tauri shell + a headless daemon that owns agent processes. It already hosts Claude Code two ways: a PTY terminal mode, and a stream-json structured chat mode. Codex is currently wired as a PTY-only backend (Phase 1: launch and render a TUI, no resume, no structured chat). Phase 2 would give codex resume + structured chat + message delivery.

## Our binding constraints (from the project's own rules)
- **C1 Backend isolation.** All backend-specific knowledge lives in one crate (`backend`); the manager only dispatches and the transport must not know which backend it serves.
- **C2 Session restore.** "Restore correctness depends only on a session id **we control**. Vendor tracking files are best-effort and must not be the foundation of a feature." Today claude satisfies this literally: we mint a UUID and pass `--session-id`, then `--resume`.
- **C3 Core isolation.** The agent core knows nothing about Tauri or the transport; output/status flow only through sink traits.
- **C4 Kill causality (invariant, 2 verbs).** `transport.shutdown()` (child.kill+wait → TerminateJobObject → master drop) then `core.join_pump(5s)`. master drop → reader EOF → pump break → finish → done_tx.
- **C5 finalize exactly once**, pump only.
- **C6** Windows is the only supported platform in practice.
- **C7** The repo already uses `ts-rs` to generate TS bindings from Rust protocol types, with a CI gate that regenerates and diffs them.

## Findings

### F1 — The protocol is JSON-RPC-*shaped* NDJSON, not JSON-RPC 2.0. 확실
No `jsonrpc` field is sent or expected; upstream `rpc.rs` says so in a comment, the generated schema omits the property, and a live handshake response omitted it. One JSON object per line. Requests carry optional W3C `trace`.
Grounding: I ran a bounded stdio handshake myself and captured the bytes (below, F2).

### F2 — It runs on Windows over stdio. 확실 (captured bytes, my own run)
`printf '{"id":1,"method":"initialize","params":{...}}' | codex app-server` returned:
`{"id":1,"result":{"userAgent":"engram-spike/0.153.4 (Windows 10.0.26200; x86_64) ...","codexHome":"C:\\Users\\kimsunzun\\.codex","platformFamily":"windows","platformOs":"windows"}}`
and exited 0 on stdin EOF. Transports on Windows are stdio and ws only; unix sockets and the managed daemon lifecycle are Unix-only in 0.153.4 (`codex app-server daemon version` → "only supported on Unix platforms", executed).

### F3 — The server assigns the thread id. A client cannot supply one. 확실
`ThreadStartParams` has 27 properties, none of them an id, threadId, sessionId, name or title; all optional. `Thread.id` is documented "Codex-generated thread IDs are UUIDv7". `Thread` derives Serialize only.
**Decisive corroboration:** issue openai/codex#15767 asked for exactly this (`session_id` on `ThreadStartParams` + a `--session-id` flag), with an implementation offered, and was closed by an OpenAI maintainer as declined ("hasn't received enough upvotes").
So the claude pattern is not reproducible and is not coming.

### F4 — But the id arrives in the creation response, over the protocol. 확실
`thread/start` → `ThreadStartResponse { thread: Thread, ... }` and `Thread.id` is that id. `Thread` also carries a separate `sessionId` ("session id shared by threads that belong to the same session tree"; forks keep the root's session id; docs say read `thread.sessionId` rather than deriving it).
This is a third category, distinct from both "we dictate the id" and "we recover it afterwards from vendor files or the screen".

### F5 — Thread names exist but cannot serve as a key. 확실
`thread/name/set { threadId, name }` — after creation only, and `codex resume` accepts a name ("UUIDs take precedence if it parses"). But names are **not unique**: the local `state_5.sqlite` `threads` table has no UNIQUE constraint on `name`, and duplicates were observed both there and in `session_index.jsonl`.

### F6 — Reattach works within one app-server process; across processes it is broken. 확실 / 가능성 높음
`thread/resume` doc-comment: "If thread_id identifies a running thread, app-server **rejoins** that thread." Subscriptions are per-connection sets (`connection_ids: HashSet<ConnectionId>` per thread), so multiple clients can attach. On disconnect the thread survives; the server keeps it loaded 30 minutes with no subscribers, then emits `thread/status/changed → notLoaded` and `thread/closed`. Cross-process is unsafe: issue #33241 reports two app-server processes holding writable fds on the same rollout JSONL and histories merging after restart; #25914 reports `thread not found` / `no active turn to steer` across processes. (Issue reports, no maintainer confirmation → 가능성 높음.)

### F7 — Persistence: `state_5.sqlite` is the queryable index and it has cwd. 확실
`threads(id PK, rollout_path, created_at, updated_at, cwd NOT NULL, title, name, ...)` with cwd indexes; rollout JSONL under `~/.codex/sessions/<Y>/<M>/<D>/rollout-<ts>-<uuid>.jsonl` is the content source of truth; `thread_history_1.sqlite` is a derived projection; `session_index.jsonl` is a 3-field name-lookup side file (id, thread_name, updated_at) and is drifty. Over the wire, `ThreadListParams.cwd` filters by exact cwd match; stored form on Windows is UNC-extended (`\\?\I:\...`) so normalization is required.

### F8 — Structured events are a superset of what stream-json gives us. 확실
83 server→client notifications, identical with and without the experimental opt-in. Deltas: `item/agentMessage/delta`, `item/reasoning/textDelta`, `item/reasoning/summaryTextDelta`, `item/commandExecution/outputDelta`. Complete blocks via `item/started`/`item/completed` carrying a 20-variant `ThreadItem` union. Turn lifecycle `turn/started`/`turn/completed` with `TurnStatus = completed|interrupted|failed|inProgress`. `thread/status/changed` carries `ThreadActiveFlag = waitingOnApproval|waitingOnUserInput`. Token usage via `thread/tokenUsage/updated`.

### F9 — Approvals are server→client **requests** we must answer. 확실
`item/commandExecution/requestApproval`, `item/fileChange/requestApproval`, `item/permissions/requestApproval`, `item/tool/requestUserInput`, `mcpServer/elicitation/request`. The command payload carries `availableDecisions: Array<CommandExecutionApprovalDecision>` — the server tells the client which buttons to render. **Our client must therefore also be a JSON-RPC server**; if we do not answer, the agent stalls. This is bidirectional, not a one-way event feed.

### F10 — Input: `turn/start` (idle) and `turn/steer` (mid-turn, with `expectedTurnId` precondition) are both in the stable surface. `thread/queue/*` is experimental-gated. `turn/interrupt { threadId, turnId }` is the Esc equivalent and is turn-scoped (thread survives, turn lands `interrupted`). 확실

### F11 — There is **no shutdown method** and **no protocol version**. 확실
Teardown is transport-level: close stdin (observed: exit 0). Negotiation is capability flags only (`experimentalApi`, `optOutNotificationMethods`, `requestAttestation`, open `extensions` map) — `InitializeParams`/`InitializeResponse` carry no version on either side. So a breaking change cannot be detected at handshake; it surfaces at runtime.

### F12 — Churn is real, dated, and absorbed by peers. 확실
v1 is frozen (4 legacy methods remain); AGENTS.md says all new API goes in v2. Peers carry dated compat gates: paseo has `// COMPAT(codexLegacyCollabAgentToolCall): Codex <0.143 emits this shape ... remove after 2027-01-09`, version floors (`CODEX_GOALS_MIN_VERSION [0,128,0]`), and dual handlers (`tool/requestUserInput` alongside `item/tool/requestUserInput`). Vibe Kanban migrated `newConversation` → `thread_start`/`thread_fork` between v0.0.161 and main. Version probing is table stakes.
A `deprecationNotice` notification method exists; some types carry inline `@deprecated`/`[UNSTABLE]`.

### F13 — Peer survey: app-server is the de-facto choice for multi-backend hosts. 확실
- **paseo** (daemon owns agents, multi-client attach — our exact architecture) drives codex **exclusively** through `codex app-server` with piped stdio: a 7,230-line provider, 14 codex test files, a `fake-app-server.ts` seam. Its architecture doc names Codex = "Codex AppServer" while routing every other non-claude backend through a generic ACP adapter — i.e. codex deliberately got a bespoke first-class client. `codex exec` appears once, as a TODO.
- **Vibe Kanban** (Rust) spawns `codex app-server` and uses the official `codex_app_server_protocol` crate.
- **Codexia** (Rust + Tauri — closest structural peer) spawns `codex app-server` with piped stdio.
- **orca** runs the TUI in a PTY for execution but uses app-server as a narrow side-channel, and its source explains a **migration toward** app-server: it used to reimplement codex's hook `trusted_hash`, "which drifted from the real one across Codex releases (#7896, #7110, #8699)", so it now lets codex be the only authority via the sanctioned RPCs.
- **No peer drives codex through `codex exec --json` as a primary backend. No peer adopted app-server and retreated.** (Absence in this sample, not proof.)
- `codex mcp-server` is **officially deprecated in favor of app-server**.
- Counter-signal: OpenAI labels app-server experimental and steers CI/automation to the Codex SDK — but that disclaimer names the command and the **WebSocket** transport specifically; every peer above uses stdio.

### F14 — The PTY route's cost is visible in peer source. 확실
orca: keystroke injection collides with the TUI trust menu ("the paste either selects an arbitrary option or quits the session"); approvals are all-or-nothing (`--dangerously-bypass-approvals-and-sandbox`); structured chat is produced not from the PTY but by tailing `~/.codex/sessions/**/rollout-*.jsonl`. herdr ships remotely-updatable versioned screen manifests because TUI churn cannot be version-gated, and its changelog shows detection breaking on reordered spinners and changed status text.

### F15 — Windows-specific peer lesson. 확실
paseo changelog: "Windows: daemon no longer crashes when Codex emits non-JSON output — Localized stdout lines from the Codex CLI are now ignored instead of taking down the daemon worker (#866)". Treat as a hard requirement: never assume every stdout line is JSON.

## Candidate approaches

**A. app-server over stdio, our daemon owns the child, one process per agent.**
Matches how the daemon owns PTYs today. Gets F8/F9/F10 in full. Requires: a bidirectional JSON-RPC client (we must answer server requests), storing the codex-assigned `thread.id` at creation, version probing, generated bindings + CI diff (F12), tolerant line parsing (F15).

**B. app-server over stdio, one shared child hosting many threads.**
Fewer processes; F6 says multi-thread multiplexing is well-supported (every notification carries `threadId`). But one crash takes down all codex agents, and it conflicts with the already-decided "one process per conversation" process model.

**C. Keep PTY, add transcript-file tailing for structured chat (the orca pattern).**
No new protocol. But F14 shows the costs, and it makes vendor files load-bearing, which C2 explicitly forbids.

**D. Embed codex-rs as Cargo git deps (the Zed `codex-acp` pattern).**
In-process, no subprocess protocol. But that repo is archived, and it hard-pins to codex's internal crates at a git tag — worse churn exposure than the protocol, not better.

## The real decision
Not "is it possible" — it is, and peers prove it. The decision is what to do about **C2**: codex cannot give us a client-controlled session id, and the request for one was declined upstream. Either C2 gains an explicit second branch ("server-minted, protocol-delivered, stored by us at creation" — which is not the guessing C2 was written to forbid), or codex is exempted from C2, or Phase 2 does not happen.

## Known gaps (not yet closed)
- No live `thread/start` round-trip was run; F3/F4 rest on the generated schema + upstream source + the declined issue, not on captured bytes of a created thread.
- Whether two connections on one daemon can both *drive* turns (not just observe) is unresolved.
- `codex exec --json` uses a **different, flatter event vocabulary** (dot-form `item.started` vs slash-form `item/started`); it was not extracted.
- Codexia's resume mechanism was not verified.
- Local install is 0.153.4; upstream source read was `main`. Silent version drift is possible.

---

# 적대 리뷰 결과 (cross-family · codex CLI · effort high) — 판정 **BLOCK**

★**위 본문은 이 리뷰를 반영하지 않은 상태다 — 아래 지적을 먼저 읽고 본문을 읽을 것.**★
BLOCK 은 「app-server 가 불가능하다」가 아니라 **위 보고서의 논증이 막혔다**는 뜻이다.

## BLOCKER 2건

**B1 — 최종 삼지선다가 거짓이고, 「프로토콜로 받으면 통제한 것」은 합리화다.**
빠뜨린 **네 번째 안**: 우리가 우리 id(E)를 만들고 `E → codex thread id(T)` 매핑을 **`turn/start` 를 허용하기 전에** 디스크에 확정한다. 그러면 C2 를 손대지 않고도 성립한다. 깨지는 시나리오 = codex 가 T 를 만든 뒤 우리가 E→T 를 커밋하기 전에 죽으면, E 만으로는 복원 불가. 이슈 #15767 본문 자체가 이 매핑 계층을 언급한다.
→ 네 번째 안을 설계 후보로 올리고 크래시 복구를 명세하든지, C2 를 명시적으로 완화하든지 택일.

**B2 — F6 의 「프로세스가 다르면 재부착이 깨진다」는 과장이고 세 가지를 뭉갰다.**
갈라야 하는 셋 = ① **cold resume**(껐다 켜고 저장된 스레드 잇기 — 0.153.4 README 가 지원한다고 문서화) ② **동시 소유 거부**(#33241 은 두 프로세스가 *동시에* 같은 rollout 에 쓴 사고, 0.144.2) ③ **live stdio 재부착 불가**(#25914 는 Desktop 의 *활성* 턴 인수인계). **순차 재시작 후 resume 이 실패한다는 증거는 어디에도 없다.** 30분 unload 규칙도 stdio EOF 가 아니라 `thread/unsubscribe` 뒤에 걸린다.
→ 셋으로 쪼개고 「깨진다」의 확실 태그를 뗄 것. **우리 용례(껐다 켜고 잇기)는 지원되는 쪽이다.**

## HIGH 6건 (요약)

3. **`sessionId` 의미를 잘못 적었다** — 0.153.4 README 의 `thread/fork` 예시는 새 스레드의 `id` 와 `sessionId` 가 **둘 다 새 루트**다. 「fork 는 원래 루트의 session id 를 유지한다」는 서술과 어긋난다. → **resume 키는 `thread.id` 를 쓰고**, `sessionId` 로 계보를 추론하지 말 것.
4. **#15767 결말을 과대해석했다** — 「upvote 부족」으로 닫힌 것은 **설계 거부도 로드맵 약속도 아니다**. → 「0.153.4 에는 클라이언트 지정 id 가 없다」까지만 말하고 「앞으로도 안 온다」는 예측을 뺄 것.
5. **피어 표본이 편향됐다** — `codex exec --json` 으로 resume 까지 붙인 프로젝트가 실재한다(OpenDesign `nexu-io/open-design`, `nucel-dev/agent-sdk`). OpenAI 비대화형 문서는 스크립트·CI 에 `exec` 를 권한다. → **`exec --json` 과 공식 SDK 를 평가 후보로 올리고**, 생태계 주장을 조사 표본으로 한정할 것.
6. **실험적 딱지 범위를 좁혀 읽었다** — 현행 공식 문서는 「app-server **명령**과 WebSocket 전송」이라 적어 stdio 도 포함된다. 0.153.4 README 만 WebSocket 으로 좁게 적혀 있다 = **문서 간 불일치**이지 stdio 면제가 아니다.
7. **수명 복구 명세가 통째로 빠졌다** — 이슈 #40766: Windows 에서 MCP OAuth 만료가 app-server 를 죽여 rollout 이 잘리고 resume 이 streaming 에서 멈춤. → 프로세스·연결·스레드·턴의 **상태기계를 각각** 정의하고, EOF 시 대기 중 RPC 를 실패시키고 활성 턴을 미확정으로 표시한 뒤 재시도 전에 조정할 것. 로그인·MCP·서브에이전트 복구도 명세.
8. **와이어 동시성 위험이 빠졌다** — 0.153.4 README 가 bounded queue 와 `-32001` 과부하 응답을 문서화하고, #22393 이 Windows 큐 포화를 보인다. 구체 실패 = 우리와 서버가 동시에 request id `0` 을 보내는데 대기표가 하나면 승인 응답이 엉뚱한 데로 간다. → **읽기는 항상 비우고**, 하류 큐는 유계로, stdin 쓰기는 하나로 직렬화, **인바운드/아웃바운드 id 공간 분리**, 승인 생명주기와 재시도 규칙 명시.

## 리뷰가 확인해 준 것 (반박 없음)
Windows stdio 기동 · 구조화 이벤트 목록 · 승인이 서버→클라 요청이라는 것 · `turn/start`/`turn/steer`/`turn/interrupt` 가 stable 이라는 것 · 프로토콜 버전 부재.

## 재생성 명령 (스키마·타입 — 서버 안 띄움)
```
codex app-server generate-json-schema --experimental --out <dir>
codex app-server generate-ts          --experimental --out <dir>
```
