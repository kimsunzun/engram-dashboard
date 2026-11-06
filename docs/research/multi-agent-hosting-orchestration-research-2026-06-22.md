# Multi-Agent Hosting and Orchestration Research (Detailed Record)

> **Status:** Stages 1, 1.5, 2 and 3 complete. Options presented — **awaiting user decision**.
> **Phase:** PRD/consult (options → user decision). Not implementation.
> **Method:** 5 sonnet Explore + 1 Codex parallel survey (Stage 1) → 3 parallel adversarial verifications (Stage 1.5) → Opus synthesis (Stage 2) → Codex + opus two-reviewer review (Stage 3).
> **Date:** 2026-06-22 · **plan:** `~/.claude/plans/jaunty-spinning-moth.md`
> **Confidence labels:** (certain) backed by official docs or code · (likely) backed by circumstantial evidence · (uncertain) unconfirmed or guessed.

---

## 0. Why (Context)

engram runs one agent = one `claude` CLI process (PTY/portable-pty, spawned under the daemon) = **process-per-agent**. Spawning N agents costs RAM for N CLI exes. The user's questions: ① can "one session/process hosting several agents (in-exe)" reduce memory, ② in-exe/out-exe combinations, ③ which algorithms to borrow from existing orchestrators. The main line → lay out options, trade-offs and missed alternatives for the user's decision.

---

## 1. Stage 1 — Empirical Survey (raw data)

### A. Claude first-party multi-agent mechanisms

**A-1. Claude Code Task subagents — separate OS processes (process-per-agent), fully isolated (certain)**
- Actual spawn command (confirmed in issue #19045): `claude --output-format stream-json --verbose --input-format stream-json --model <m> --resume <SESSION-ID> --disallowedTools ...`. The parent session creates the child `claude` with `spawn()`.
- Concurrency: the official docs say "sequential", but when a single response makes several Task calls, children are created in rapid succession. There is no official `maxParallelAgents` (feature request #15487). One reported case created 24 processes within 2 minutes.
- Memory: orphan processes (not terminated after the parent exits) take ~400MB each.
- Context: reads the existing session via `--resume <sid>`, but execution is fully isolated. There is no real-time sharing between parent and child.
- Control: the LLM calls the Task tool → the runtime spawns. There is no external API for direct control.

**A-2. Claude Agent SDK (TS/Py) — CLI subprocess wrapper, a single SDK process drives N sessions (certain)**
- Internals: `query()` spawns the Claude Code CLI binary as a subprocess (the native binary is bundled as an optional dependency). **It is not a direct API call.**
- N sessions: the caller runs several `query()` calls in parallel → each is an independent subprocess. (11+ concurrent calls triggered a `MaxListenersExceededWarning` bug → fixed, which means many concurrent sessions are a supported design)
- Three ways to resume a session: `resume: "<uuid>"` (a specific past session), `continue: true` (pick up the most recent), `forkSession: true` + `resume` (fork a past session → a new sid is issued = a branch).
- Session ID injection: `query({ options: { sessionId: "custom-uuid" } })`.
- **Pre-warming:** the `startup()` API initializes the subprocess ahead of time → removes first-query latency. `WarmQuery` handle, `AsyncDisposable` (automatic release).
- Session storage: JSONL by default (`.claude/sessions/`). A `sessionStore` adapter connects an external backend (DB). `persistSession: false` disables storage.
- File checkpointing (experimental): `rewindFiles(userMessageId)` rolls files back to the point of a given message.
- Streaming input: `prompt: AsyncIterable<SDKUserMessage>` → a message stream during the session. `setModel()`/`setPermissionMode()` can be called mid-session.
- Memory: one subprocess per concurrent session — the SDK is also process-per-agent. The SDK process itself is a lightweight orchestrator.

**A-3. Agent View / Teams / multi-agent UI**
- Agent View: visualizes subagent run status (running/done/failed). (uncertain: official screenshots not confirmed)
- **Teams mode:** each teammate = a **separate Claude Code instance** (certain). Session IDs + tmux pane IDs are stored in `~/.claude/teams/{team}/config.json`. Split-pane is a *display method*; it is not a single process.
- claude.ai: no separate multi-agent feature (single conversation). Multi-agent belongs to Claude Code/SDK.

**A-4. Headless/programmatic control**
- `claude -p "prompt"` is non-interactive and one-shot (every call is an independent process). `--output-format stream-json` (JSONL) / `text`.
- `--session-id <uuid>` (we control the sid — same as engram S9), `--resume <sid>`, `--input-format stream-json` (multi-turn JSON over stdin). Exit codes 0/1/2.

**A conclusion:** Anthropic first-party (Task + SDK) is **process-per-agent in every case**. First party does not officially offer "N agents in a single process" → engram's current design = the same layer as Anthropic's own architecture. What the SDK adds = a process-management abstraction (pre-warming, fork, sessionStore, AsyncDisposable) — a reference to port into engram.
Sources: code.claude.com/docs/en/agent-sdk/typescript, .../agent-sdk/subagents, .../agent-teams, claude.com/blog/building-agents-with-the-claude-agent-sdk, github.com/anthropics/claude-code/issues/{4182,19045,15487}, docs.anthropic.com/.../sdk-sessions

### B. Third-party Claude agent managers/UIs

| Tool | Process model | Isolation | Memory | Persistence/resume | Stack |
|---|---|---|---|---|---|
| **claude-squad** (smtg-ai) | 1 tmux session + 1 CLI per agent | git worktree 1:1 | no savings | pause/resume with the `r` key (details uncertain) | Go, tmux, gh |
| **Crystal** (stravu→Nimbalyst) | node-pty, 1 CLI per agent, no tmux | worktree 1:1 (`worktreeManager.ts`) | **lazy init** (spawn only when opened) + stops rendering inactive views; still 1 process/agent | **SQLite** (sessions/outputs/messages) → scrollback restored after restart | Electron+React19+Zustand+XTerm+SQLite |
| **Conductor** (conductor.build) | 1 CLI per agent, runs its own bundle | workspace = branch + terminal (worktree presumed, uncertain) | no savings | uncertain | macOS only, internals not public |
| **vibe-kanban** (BloopAI) | spawns the CLI subprocess with `command-group`, 1/agent | worktree 1:1 | no savings | uncertain | Rust 50% + TS 46%, React |
| **ccmanager** (kbwo) | PTY, 1 CLI per agent, avoids tmux | UI to create/delete/merge worktrees, copies session data | no savings | copies conversation history between worktrees | TypeScript |

- **Common (certain):** no tool fundamentally solves RAM — all are process-per-agent. The CLI itself is an independent process, so an external manager cannot multiplex it. RAM can be solved only at the **agent runtime layer**, not in the client (manager).
- Partial mitigation: Crystal lazy init (avoids unnecessary eager spawns).
- Candidates for engram to borrow: lazy spawn, SQLite session persistence (the current replay buffer is memory-only), ccmanager's 4-stage state machine + hooks (§5), automatic worktree creation/cleanup.
Sources: github.com/{smtg-ai/claude-squad, stravu/crystal, kbwo/ccmanager, BloopAI/vibe-kanban}, conductor.build

### C1. Named tools the user pointed to

**Gastown** (github.com/gastownhall/gastown, Steve Yegge, 2025) — runs many coding agents (20~30) in parallel on the same codebase.
- Coordination = hierarchical supervisors: **Mayor** (splits and assigns work) · **Polecat** (worker; the session is ephemeral but identity and history persist) · **Witness** (per-rig health monitor — detects stalls → nudge/handoff) · **Deacon** (cross-rig watchdog daemon).
- Hosting: process-per-agent. Context: **Hooks** (worktree state) · **Beads** (git issue data) · **Seance** (search and recover previous sessions → no context loss after restart).
- To borrow: **an external health monitor + context-carrying restart (Witness)** = a direct counterpart of engram's reaper/StatusSink.

**multi-agent-coordinator** — not an official product name (uncertain). Candidates: ① issue #20095 "Conference Room Mode" (a router distributes input to several sessions; feature request) ② the Managed Agents multi-agent API (an orchestrator spawns subagents via Task, parallel in isolated contexts, threads persist). The most likely reading = a Task-based orchestrator-worker.

**claude-flow → ruflo** (github.com/ruvnet/ruflo, renamed over a trademark, Reuven Cohen, v3.5) — a meta-harness of 100+ specialized agents.
- Coordination = **Queen-worker swarm**: the Queen manages topology and consensus; role-based agents (coder/tester/reviewer/architect/security), stream-json chaining, dependency-graph parallel execution.
- Hosting: coordinates Claude Code instances on top of MCP + 27 hooks, process-per-agent. No fault tolerance is stated (uncertain).
- Memory: **AgentDB** (shared vector DB) + SONA + ReasoningBank.
- To borrow: capability-based dynamic role routing.
Sources: github.com/gastownhall/gastown, steve-yegge.medium.com/welcome-to-gas-town, github.com/anthropics/claude-code/issues/20095, platform.claude.com/docs/en/managed-agents/multi-agent, github.com/ruvnet/ruflo

### C2. General orchestration frameworks (focus: durability and resume)

| Framework | Coordination model | Durability/resume | Context sharing | Pattern to borrow |
|---|---|---|---|---|
| **LangGraph** | StateGraph (nodes/edges), Supervisor node | **super-step checkpoints**: a snapshot per `thread_id` (SQLite/PG), automatic resume under the same id after a crash, time-travel | shared state via TypedDict + reducer | `thread_id=AgentSession.id` |
| **Temporal** | Workflow (deterministic) + Activity (external I/O), Child Workflow | **event-history replay**: skips completed Activities → resumes at the interruption point | Workflow-local + Signal/Query | the boundary "everything that touches the outside is an Activity" |
| **Restate** | service handlers (HTTP) + Virtual Object (per-key state) | journal replay (event sourcing), **single binary + built-in KV (RocksDB)** | per-key KV in Virtual Objects | single-binary sidecar |
| **AutoGen (AG2)** | GroupChat conversation, speaker selection | weak (in-memory history), external checkpointer plugin (uncertain) | conversation history | UserProxy = a separated orchestrator role |
| **CrewAI** | Agent + Task + Crew, Sequential/Hierarchical | weak (partly in Flows), production tends to migrate to LangGraph | task output chaining | role-based agent spec |
| **MS Agent Framework** | Orchestrator-Agent (AutoGen + SK), v1.0 GA 2026-04 | none built in, delegated to external (Cosmos/Durable Task) | message stream + shared memory | agent type classification |
| **A2A protocol** | agent = network endpoint (HTTP + JSON-RPC + SSE) | the protocol itself has no durability, only Task state | Task multipart messages | **Agent Card** (advertises capability JSON) |

- **Recommended Top 3 (engram's pain = cannot resume):** ① Restate/Temporal replay (wrap session spawn as an Activity → resume; **Restate first** — a single-binary sidecar, lowest operating burden for a desktop-native app; a Rust SDK exists, maturity uncertain) ② LangGraph thread_id = session id checkpoints (one SQLite file, easy to implement directly in Rust) ③ standardize the capability matrix with A2A Agent Cards.
Sources: docs.langchain.com/.../persistence, docs.temporal.io/workflow-execution, spheron.network/blog(temporal-restate), devblogs.microsoft.com/agent-framework, arxiv.org/pdf/2504.16736 (A2A survey), latenode.com (LangGraph vs AutoGen vs CrewAI)

### C3. Rust-fit supervision (OTP/actor)

- **OTP restart strategies:** `one_for_one` (only the dead one — AI agents are independent, so this is the **default**) / `one_for_all` (if one dies, all restart) / `rest_for_one` (the dead one and those after it — pipeline dependencies).
- **Restart Intensity `{MaxR, MaxT}`** (the key thing to borrow): more than MaxR restarts within MaxT seconds → the supervisor itself exits = escalation. It defines a crash loop with numbers. "Let it crash" (do not try to recover, kill and restart). Supervisor tree hierarchy.
- **Ractor vs plain-tokio decision: plain-tokio + a thin wrapper is recommended.** Ractor (2.1k★, v0.15) `ractor_supervisor` is at an early stage, does not support supervising external OS processes, and there is an impedance mismatch between in-process actors and PTY processes. The tokio alternatives (task-supervisor · Taskvisor · **ProcessKit-rs** (for external processes only)) already implement backoff and crash-loop handling. → **Porting the OTP concepts directly into AgentManager** pays off compared with the dependency. Re-evaluate at the point of a real actor mesh (remote distribution).
- engram mapping: AgentManager = root supervisor, AgentSession = child spec, reaper = monitor + exit classification, epoch = the child's restart slot. Recommended values: MaxR=5/MaxT=120s, backoff 1s×2 capped at 60s, jitter ±20%.
Sources: erlang.org/doc/apps/stdlib/supervisor.html, github.com/slawlor/ractor, docs.rs/ractor-supervisor, github.com/ZelAnton/ProcessKit-rs, users.rust-lang.org (Taskvisor)

### Hosting-axis matrix (Codex inference — FACT/REASONING/UNCERTAIN labels)

| Model | RAM | Fault isolation | Security/sandbox | Implementation complexity |
|---|---|---|---|---|
| **A. process-per-agent** (current) | highest (a per-process runtime tax) | strong (1 crash = 1 agent) | strongest (OS process boundary) | lowest for a CLI (natural) |
| **B. in-process multi** | lowest (daemon + tasks, per-agent = conversation state) | weak (a panic, deadlock or event-loop starvation affects everything) | weak (shared address space and credentials) | medium to high (cancellation, task supervision, shared-state discipline) |
| **C. hybrid worker pool** | medium (K agents/worker, tax spread out) | medium (1 worker crash = K agents) | medium (per worker/trust tier) | highest (worker protocol, routing, partial failure) |

- **Key insight (confirmed, refined):** "the backend's execution substrate decides the minimum hosting boundary." A PTY CLI = the process boundary is forced / an HTTP API = no boundary (async task) / a daemonized CLI (JSON-RPC, LSP-style) = a worker pool is possible. Threads are valid only for library-style agents. Even tmux cannot merge CLIs.
- **Memory scale (UNCERTAIN, estimate):** idle async task ~10KB~hundreds of KB / in-process + conversation state ~0.5~10+MB / **idle Node-style AI CLI ~80~300+MB RSS** / active CLI ~200MB~1GB+. → 20 idle: process-per-agent several GB vs in-process API tens to hundreds of MB.
- **Codex recommendation:** introduce `capabilities.hosting`, keep the CLI as process-per-agent (an honest boundary), **go in-process starting with the API (codex_api) (the largest gain)**, a worker pool is for the future. Implications for AgentManager: ① separate AgentHandle from ProcessHandle ② make supervision hosting-aware ③ layer kill (cancel/terminate/close_session) ④ normalize output (terminal_stream vs structured_events) ⑤ start pooling with the API/SDK, not the PTY CLI.

---

## 2. Stage 1.5 — A Disputed Premise, Corrected with Data (parallel adversarial verification)

The original Stage 1 summary **over-generalized** to "a CLI cannot host several agents in one session." The parallel verification (Advocate/Adversary/Codex) corrected it:

**Corrected claim (Codex decision: the original conclusion was over-stated):** process-per-agent is forced **only when "the CLI binary exposes a single session and has no multiplexed entry point."** Only merging two already-running CLI processes after the fact is impossible (an OS fact). **On an API/SDK substrate, one process runs N agent loops (async tasks)** — demonstrated:

| Approach | Same process? | Mechanism |
|---|---|---|
| Claude Agent SDK (`agents` param) | yes (likely) | async `Agent` tool calls inside a single process, isolation is per context window |
| **Claude Code Agent Teams** | **no (certain)** | a separate OS process per CLI instance, separate tmux panes |
| AutoGen v0.4 | yes (certain) | a single Python process, asyncio event loop |
| CrewAI | yes (certain) | Agent objects + LLM API calls inside a single process |
| LangGraph | yes (certain) | graph nodes (functions/async tasks) inside a single process |

**Terminology correction (the cause of the confusion — "session" blurred the two):**
- **Agent Teams = multiple sessions / multiple processes** (certain; "each teammate is a separate Claude instance", session ID + tmux pane). → Same kind as engram's current model, not a target for memory savings.
- **Subagents = a single session** ("work within a single session"). SDK `agents` = in-process objects (likely). Whether the CLI Task path uses a subprocess is **uncertain (not confirmed officially)**.

**Boundaries (Adversary data):**
- Impossible to merge after the fact = an OS fact (memory/fd/signals are owned per PID, there is no merge API).
- If a CLI exposes a daemon/mux mode, several sessions in a single process are possible — real examples: **tmux server** (a session = a structure inside the server), **Zellij**, **lspmux/karellen-lsp-mcp** (LSP multi-session daemons). Whether the claude CLI supports such a mode is **uncertain**.
- Cost of co-hosting: fault isolation (Chrome site isolation splits processes to isolate renderer crashes, for an OS sandbox, and to block Spectre), but the Erlang VM isolates thousands of lightweight processes inside a single OS process (runtime isolation is possible without OS isolation, but memory/sandbox isolation is not).
- **The narrowest correct claim:** "process-per-agent is forced only when ① OS-level sandbox/credential/memory isolation between agents is required or ② the CLI gives no multiplexed entry point. If the CLI has a daemon mode (tmux/LSP), or if trusted same-user agents are isolated with an async runtime, hosting several agents in a single process is possible."

**Conclusion:** the *memory* gain of "several agents in one session" exists **only on an API substrate**. The CLI (claude_console) is honestly process-per-agent.
Sources: code.claude.com/docs/en/{agent-teams, agent-sdk/subagents}, microsoft.github.io/autogen, tao-of-tmux(server), chromium process-model & site-isolation, codeberg.org/p2502/lspmux, github.com/karellen/karellen-lsp-mcp, moldstud.com (Erlang isolation)

---

## 3. Mapping to engram's Current Seams (code checked — review corrections applied)

- `Capabilities { input, output, control, session, model } = compose(TransportCaps, BackendCaps)` (domain.rs / types.rs:141). **A 2-source merge**: transport = input/output/control, backend = session/model. → The **transport substrate decides** the hosting model, so a field inside `TransportCaps` is right (making it a new top-level dimension would mean a compose signature change + the wire struct + ts-rs + a PROTOCOL_VERSION bump). `ControlCaps` *has* `cancel`/`graceful_shutdown` fields, but they are *not set* for the API.
- `AgentTransport` (start/send_input/resize/interrupt/shutdown/capabilities) = responsible for resource lifetime. **`transport/api.rs` is a no-op shell** (`pub struct ApiTransport;` stateless, all caps=false, cancel=false confirmed). **Only the trait boundary exists — the implementation is greenfield** (HTTP client, stream task, cancel token and session state are all new). It is not "filling in a stub."
- The kill causality of 2 verbs (ADR-0001: shutdown → master drop → reader EOF → pump break → core.finish → done_tx, then join_pump 5s) **assumes a PTY**. An in-process API has **no master fd** → it **needs a redesign** of who the pump is, what produces the done signal, and whether the synchronous 5s wait in `join_pump` is legitimate on a shared tokio runtime.
- Fault isolation: the reaper (catch_unwind per pump, reaper.rs) assumes a **per-process/per-thread fault domain**. With a shared async-task runtime, one task's panic/deadlock can stall every pump and the reaper through executor starvation — current supervision does not defend against this.

---

## 4. Stage 2 — Fused Options (for the user's decision, phased)

Memory (substrate) and control (orchestration) are separate axes. In-process has a large isolation/finalize retrofit cost, so the options are presented in phases.

**Option 0 — hosting unchanged + practical guards** _(lowest-cost baseline, near-term)_
A maxParallelAgents cap + queue (supported by issue #15487) + lazy spawn (Crystal) + a warm pool (SDK `startup`). Gain: *practically* eases a memory blowup without touching the architecture. Cost: the fundamental RAM per agent stays. The other options must justify their extra cost against this one.

**Option 1 — API backend in-process (hosting = a TransportCaps field)**
Put the hosting model in a field inside `TransportCaps` (not a new dimension — consistent with the compose 2-source and ADR-0002). Implement the `codex_api` transport anew as an async task. Gain: API agents could take ~MB per task → many concurrent low-memory agents (**figures not measured · UNCERTAIN**; features, auth and session semantics may differ from the CLI). **Unresolved retrofit:** with no master fd, the ADR-0001 kill causality, finalize-once and join_pump do not fit → redefine cancel → synthetic terminal → core.finish + the done producer. Isolation weakens from per-process to per-task. Rejected alternative: "the CLI also in-process" → impossible on the OS.

**Option 1b — engram's own out-of-process API worker pool** _(review proposal — isolation-preserving)_
A separate `engram-worker` binary hosts K API agents (the daemon already spawns processes). Gain: the RAM gain + **process isolation kept** (avoids Option 1's finalize/starvation problems), and it can be built now without vendor dependence. Cost: IPC/worker protocol, rescheduling, partial failure. Rejected alternative: "in-process single process" → loses isolation.

**Option 2 — cwd team orchestration + lifecycle control** _(a separate axis from RAM, §5)_
A supervisor routes logical "teams" grouped by cwd and shares context (Gastown Witness/Seance). **A team of mixed CLI+API heterogeneous backends** (unified status/output/kill/restart). Team lifecycle as LLM handles: start/pause-all/cancel-all/kill-all/restart-failed/restart-team/drain/archive. Gain: the UX of "controlling several agents in the same folder from one place" + LLM-first. **Does not reduce RAM** → must be combined with Option 1/1b to cover memory too. Rejected alternative: "a cwd team = one CLI process" → claude has no multiplexed sessions (uncertain).

**Option 3 — vendor daemon-mux hosting** _(future gate, non-action)_
When claude/codex expose a daemon or multiplexed session, K agents/worker. Reserve only the enum (low risk · §0), defer implementation. Vendor support uncertain.

### Cross-cutting borrowings (corrections applied)
- **supervision:** the reaper `RestartRecord` (OTP restart-intensity {MaxR,MaxT} + backoff) is a decision that **reopens the "runtime auto-restart" that ADR-0019 retired** — it is not the activation of a reserved field (the ADR-0016 reservation is limited to boot restore/guard/Failed, profile.rs:55). Adopting it needs an ADR that reverses ADR-0019.
- **resumability (the "from scratch" pain):** S9 `--session-id` = isomorphic to LangGraph `thread_id`. Persist output replay in SQLite (Crystal) to restore on cold boot.
- **capability exposure:** let the LLM query the capability matrix in A2A Agent Card format (§5). §5 requires even "the LLM can call every UI action" → every team lifecycle verb must become a tool/command schema.

### Forks to decide (user)
1. **Memory approach:** Option 0 (guards only) / Option 1 (in-process, simple · weak isolation · large retrofit) / Option 1b (out-of-proc pool, isolation kept · IPC cost).
2. **Team/control (Option 2)** now or later.
3. Whether to do **supervision auto-restart** (= reopening ADR-0019).

---

## 5. Stage 3 — Two-Reviewer Adversarial Review (done: both FIX, converged — no escalation needed)

- **Codex (User/completeness lens) FIX:** mixed CLI+API teams, team lifecycle, crash/resume and the depth of §5 were thin / promote Option 0 / memory figures UNCERTAIN / no phased recommendation for "what first next week?". → Applied.
- **opus (Tester/breaker lens, doc-aware) FIX (code checked):** ① the kill causality/finalize/join_pump retrofit was "only named" (manager.rs:462-483, reaper.rs:178-198, session.rs) ② the weakening of per-task isolation was not reflected (the reaper's catch_unwind does not defend against executor starvation) ③ "filling in a stub" was an overstatement — api.rs is stateless, caps=false ④ HostingCaps is not a new dimension but a TransportCaps field (compose 2-source, types.rs:141) ⑤ a missing alternative = engram's own out-of-proc worker pool (Option 1b) ⑥ the RAM figures' UNCERTAIN was not propagated ⑦ RestartRecord = reopening ADR-0019 (profile.rs:55). → All applied.
- **No disagreement** → no user escalation needed.

### Remaining UNCERTAIN (needs measurement or confirmation)
- The real memory of codex_api (measured including stream/session/tool state).
- Whether the CLI Task-tool path uses a subprocess internally (not confirmed officially).
- Whether claude/codex support a daemon-mux mode.

---

## 6. Consolidated Sources

- **Anthropic:** code.claude.com/docs/en/{agent-sdk/typescript, agent-sdk/subagents, agent-teams}, claude.com/blog/building-agents-with-the-claude-agent-sdk, docs.anthropic.com/.../sdk-sessions, platform.claude.com/docs/en/managed-agents/multi-agent, github.com/anthropics/claude-code/issues/{4182,15487,19045,20095}
- **Third-party managers:** github.com/{smtg-ai/claude-squad, stravu/crystal, kbwo/ccmanager, BloopAI/vibe-kanban}, conductor.build
- **Named tools:** github.com/gastownhall/gastown, steve-yegge.medium.com/welcome-to-gas-town, github.com/ruvnet/ruflo
- **Frameworks:** docs.langchain.com/.../persistence, docs.temporal.io/workflow-execution, spheron.network/blog(temporal-restate), devblogs.microsoft.com/agent-framework, arxiv.org/pdf/2504.16736
- **Rust supervision:** erlang.org/doc/apps/stdlib/supervisor.html, github.com/slawlor/ractor, docs.rs/ractor-supervisor, github.com/ZelAnton/ProcessKit-rs
- **Hosting boundary:** tao-of-tmux(server), chromium process-model & site-isolation, codeberg.org/p2502/lspmux, moldstud.com (Erlang isolation)

---

## 7. Follow-up

- **Location/discovery:** lives in `docs/research/` — consult it when work starts on its topic (hosting · orchestra · memory). Not listed in tracking (user decision 2026-06-24: it is *reference material*, not an action-pending item). Same line of reference: T-9 (claude pooling) · `control-surface-and-fleet.md`.
- **After the user decides:** each major choice → an ADR (with rejected alternatives). Adopting supervision auto-restart needs an **ADR that reverses ADR-0019**. Adopting hosting = a TransportCaps field extends ADR-0002/0030.
- **Next-session TODO (not incorporated):** "organize the orchestra view into 5 axes" (topology · agent lifetime · brain (the controlling party) · context sharing · supervision) — derived in conversation but not reflected in the doc body. Make it a separate section next time.
