# Multi-Agent Orchestration Architecture — OSS Survey + Option Set (material for a design decision)

> **Status:** research material (not a decision). A collection of option sets, trade-offs and rejection candidates for the next session to use in the PRD/TRD decision. No arbitrary adoption — the user holds the decision.
> **Method:** `/research design-decision deep` — 5 main-track collectors in parallel (sonnet, by-candidate per layer) → main grounding → Codex (cross-family blind) adversarial review. Anchor = ADR-0014 (proposed).
> **Date:** 2026-07-12 · **Author:** main orchestrator (Opus)
> **Confidence legend:** certain (primary/official docs checked directly) · likely (many secondary sources support it, or partly unverified) · uncertain (unverified · estimated).
> **Degradation label:** deep's *cross-family parallel collection* was not run (single-family main-track collection + the Codex adversarial review's omission lens as a partial backstop). This topic is omission-critical (an uncovered candidate could swing the conclusion), so the possibility of omissions is stated in the §8 limits.
> **Adversarial review applied (2026-07-12):** the Codex (cross-family blind, effort high) review = BLOCK on the first edition, 12 findings (3 high), all with counter-evidence. Applied in this v2 — added the native baseline (A0) · added a process-tree isolation axis · corrected the attribution of "Claude Code Workflow" (not public OSS) · the nuance of Temporal's local dev server · corrected the confident-wrong deadlock/ACK claim in §5 · corrected the internal contradiction in the §6 order. The direction (the option set) is kept; details and grounds strengthened.

---

## 0. TL;DR (option summary — the decision is in §6)

- engram already has **half of the orchestration base**: reaper (death detection and classification) + epoch (safe resubscription) + the S9 restore ladder (resume→fresh→Failed) + persistence (reserved fields `RestartPolicy`/`restart_count`/`failed_reason`) + the WS event bus (fanout) + the command registry skeleton (the §5 entry point). What truly has to be built new is **the runtime auto-restart execution path · inter-agent addressing/messaging · a task graph · the orchestrator brain (§5 LLM)**.
- Conclusion on mature OSS: **the honest baseline for supervision (Layer A) = implement a supervisor state machine directly on tokio + the existing PTY/reaper, without a framework (A0)** — Erlang/OTP and Ractor are for *borrowing patterns*, not for adoption (OS process ≠ in-VM actor). On top of this, the axis of **process-tree isolation** (Unix process group/cgroups · Windows Job Object) is key, and engram already has a Job Object (ADR-0001). **Durable execution engines (Layer B, Temporal/Restate/Inngest) are excessive at the current scale** (a separate runtime + a pre-1.0 Rust SDK) — borrow only the local "task journal" pattern. **For coordination (Layer C), the central deterministic model "code owns the flow, the LLM does each step" is consistent with §5** (the nearest working reference = Claude Code's own Workflow/subagent tool — a built-in feature, not public OSS; see the attribution note in §4 below). **Communication (Layer D, A2A) is excessive for a single local daemon** — the existing sink/bus/protocol seams + a bounded `tokio::mpsc` suffice (but mind the §5 reliability limits).
- Three candidate big directions (§6): **(1) supervision first, minimal increment** (OnCrash auto-restart on top of the reaper + reserved fields) · **(2) a central orchestrator (Workflow-style)** · **(3) messaging first (inter-agent pipes)**. They are not mutually exclusive — it is a question of order.

---

## 1. Survey axes & engram constraints (decision criteria)

**Constraints (invariant — CLAUDE.md):** ① swappability (an implementation on top of an abstract interface, swappable) ② §5 LLM-first control (every feature is a handle the LLM calls, the human UI is secondary, the frontend = pure I/O) ③ crate boundaries (core = zero tauri imports · daemon = owns AgentManager, hosts agents · protocol = the wire contract) ④ small and local (not a distributed cluster) ⑤ Rust core. An agent = an OS process (PTY claude/codex), not an in-VM object.

**Layers (ADR-0014 structure):** A supervision and fault tolerance · B durable execution · C coordination frameworks (borrow patterns) · D inter-agent communication. + E grounding in engram's existing assets.

---

## 2. Layer A — Supervision and restart (A0 native baseline · Erlang/OTP · Ractor · Actix)

**Key finding:** engram's death detection already has the equivalent of an OTP monitor (`reaper.rs` pump EOF → `done_tx` → classification). What to borrow is the *restart policy and the meltdown threshold*, not an actor runtime. **The honest answer is A0** — write the supervisor directly on tokio + the existing PTY/reaper without a framework, with OTP/Ractor as the source of the *patterns* to put in that supervisor.

| Candidate | Core mechanism | Restart model | Maturity/license | Fit for engram |
|---|---|---|---|---|
| **A0 — tokio + hand-rolled supervisor (baseline)** | Implement a supervisor state machine directly on `tokio::process` (child wait · `kill_on_drop`) + `JoinSet` + a bounded `mpsc` + engram's reaper/epoch/S9 ladder | Code the OTP patterns directly (mapping below). Zero framework abstraction | engram already has most of it (portable-pty · reaper · epoch), Rust std/tokio (MIT) | **★ Default adoption candidate.** Since no framework can supervise a PTY OS process directly (all assume in-VM actors), we would write the supervisor ourselves anyway — OTP/Ractor only supply patterns |
| **Erlang/OTP supervision tree** | link/monitor + supervisor behaviour, let-it-crash, process isolation | 4 strategies (one_for_one/one_for_all/rest_for_one/simple_one_for_one) + intensity/period meltdown → escalation to the parent. Restart types permanent/transient/temporary | 30+ years in production, Apache-2.0 | **Pattern priority 1 (borrow).** intensity/period → the meltdown threshold, transient → give up after consecutive fresh-fallback failures, one_for_one as the default, rest_for_one → a future pipeline chain |
| **Ractor (Rust)** (+ ractor-supervisor) | **core ractor already provides supervision relations, links, failure propagation and `Actor::handle_supervisor_evt`** (the core supervises — it is not unsupervised). The separate `ractor-supervisor` layers *reusable restart strategies* on top | ractor-supervisor: OneForOne/OneForAll/RestForOne + `ChildSpec{restart,backoff_fn,reset_after}` + meltdown (max_restarts/max_window). **DynamicSupervisor** is isomorphic to engram's dynamic spawn/kill | ractor v0.15.x (2.1k★, used by Meta, MIT). supervisor crate v0.1.x — young | **Borrow (direct adoption on hold).** Port the `backoff_fn:(count)->Option<Duration>` + DynamicSupervisor · meltdown concepts into the A0 supervisor. An actor = a tokio task ≠ an OS process, so using it directly complicates the ADR-0001 kill causality |
| **Actix** | `Supervised` trait + `Supervisor<A>` watching a single actor | No strategy enum · no restart-intensity/backoff (infinite restart by default) | v0.13.5 (2024) · infrequent releases (maintenance continues · not abandoned), MIT/Apache | **Not recommended.** *The ground for rejection = `Supervisor` has no restart-intensity/backoff policy* (a verifiable API fact), not the maintenance state. It watches a single actor, so an N-agent tree needs manual wrapping |

**Process-tree isolation (a key axis of Layer A — underrated in the first edition, per Codex):** engram agents are OS processes, so "supervising direct children" differs from "isolating the whole descendant tree." Killing a tokio child **does not give portable tree isolation** — Unix needs process groups/`setsid`/cgroups, and Windows needs a **Job Object (kill-on-job-close)**. **engram already has a Windows Job Object** (`TerminateJobObject` — the ADR-0001 kill causality). So the Layer A requirement = ① Windows = Job Object (present) ② Unix = process group isolation (needed if claude/codex fork grandchild processes — checking the current implementation is recommended). OS supervisors such as systemd/supervisord are effective for restarting the daemon itself and cgroup isolation, but adoption is rejected over portability (Windows is the main target); their *isolation model*, however, is absorbed into the requirement above.

**engram mapping (OTP/Ractor patterns to port into A0):** intensity/period → `restart_count` + a time window → Failed terminal · `Transient` → give up after consecutive S9 fresh-fallback failures · `backoff_fn` → exponential backoff before the epoch swap (250ms→×2→30s cap) · DynamicSupervisor → runtime agent management · `one_for_one` as the default. Confidence: OTP = certain, that Ractor core has supervision = certain, version numbers = likely (re-check on adoption).

**Pitfalls/defenses:** restart storm (if the intensity is lenient) · the blast radius of one_for_all (engram uses one_for_one) · immediate restart without backoff (always use exponential backoff) · OS children orphaned on a supervisor meltdown/daemon crash (defended by tree isolation — unlike BEAM it is not automatic; the Job Object/process group plays that role).

---

## 3. Layer B — Durable execution (Temporal · Restate · Inngest)

**Key finding:** **all three engines are excessive at engram's current scale** — common reasons ① a separate server process is required (cannot be embedded) ② all Rust SDKs are pre-1.0/alpha (a long-term dependency risk) ③ they target distributed service orchestration, so the purpose does not match local small-scale PTY management.

| Candidate | Mechanism | Infrastructure weight | Rust SDK | Justification for engram |
|---|---|---|---|---|
| **Temporal** | Event sourcing + deterministic replay (compares history when a worker restarts, skips completed steps) | Heavy in production (4 server kinds + Cassandra/PG + optional ES). **But a local dev server exists as a single process + embedded storage** (`temporal server start-dev`) — "always heavy" holds only for production HA. Enforced determinism spreads widely | Rust SDK Public Preview (pre-1.0 — but pre-1.0 ≠ alpha) | **Unnecessary.** Even with the dev server, enforced determinism + the burden of the workflow-engine concept are excessive for local PTY management |
| **Restate** | Journaling + snapshots (code does not run on a journal hit, no enforced determinism) | Medium (a single Rust binary, no external DB). But **cannot be embedded** — the runtime sits between the client and the service endpoint (transport also supports streaming/Lambda besides HTTP callbacks, but it is not an in-process library) | v0.10 pre-1.0, MIT | **Unnecessary (for now).** The strongest candidate if a real durable need arises |
| **Inngest** | HTTP-callback step memoization | An external server + HTTP workers | Alpha (axum only) | **Unsuitable.** For serverless/webhooks |

**Minimal pattern to borrow — the "local task journal":** record each orchestration step append-only locally (SQLite/NDJSON) as `JournalEntry{task_id, step_id, status, result, ts}` → on daemon restart, skip completed steps and resume unfinished ones. An added layer on top of the existing `agents.json` persistence layer. No enforced determinism (the Restate journal pattern). **When a real engine becomes necessary:** when multi-agent DAGs (A→B→C · conditional branches · saga rollback) must be guaranteed to progress across daemon/client restarts and remote/high load comes in — re-evaluate Restate then (after the SDK reaches 1.0). Confidence: engine characteristics = certain, engram fit = likely.

---

## 4. Layer C — Coordination frameworks (borrow patterns, Python-centric)

**Key finding:** the axis is *who holds the coordination decision* — code (deterministic) ↔ LLM (flexible · non-deterministic). For engram §5 (the daemon = a deterministic host, the LLM = a brain that issues handles), a **code-owned central orchestrator** is consistent. The nearest working reference = **Claude Code's own Workflow/subagent tool**.

> **Attribution note (Codex's finding):** "Claude Code Workflow" is **not a public OSS framework but a built-in feature of Claude Code (Anthropic)** — this project actually runs on it, so it is a first-hand observation, but an outside reader cannot reproduce it by version or repo. The fan-out cap figures below (16 concurrent / 1000 total / nesting 1-depth) are based on that tool's own spec and are not publicly citable values. So the ★ rating in this row is a *design judgment*, not a verified benchmark. If a public citation is needed, use the Anthropic Claude Code subagents / Agent SDK docs as the basis.

| Model | Coordination decision | Determinism | Fan-out cap | Fit for engram |
|---|---|---|---|---|
| **Claude Code Workflow/Task** (built-in tool · non-OSS) | **the script (code)**, the LLM only does each step | highest | per the tool spec 16 concurrent / 1000 total / nesting 1-depth (not publicly citable) | **best fit by judgment** — the daemon = the script, agent processes = subagents (★ is a judgment, not a verified benchmark) |
| **LangGraph** | routing function (code + LLM) | high | no explicit cap (the graph structure is the cap, a cycle needs END) | ★★★★ — borrow conditional routing + checkpoints as a supplement |
| **CrewAI** Sequential | fixed order (code) | high | max_iter | ★★★ — only the role-declaration idiom |
| **AG2/AutoGen** SelectorGroupChat | LLM | medium | max_turns | ★★ — only the termination-marker pattern |
| **OpenAI Agents SDK** (Swarm's successor) | LLM (handoff = full transfer of control) | low | unclear (Guardrail is the line of defense) | ★★ — only Guardrail's parallel validation; handoff conflicts |

**Patterns recommended for immediate borrowing (from Claude Code Workflow):** ① **Schema-validated structured output** (validation and retry at the tool layer — consistent with protocol crate responses) ② **Adversarial verify** (fan out N skeptics per finding — `/review` already implements it, extend to orchestration QA) ③ **Loop-until-dry + global dedup** (the restart/fresh-fallback chain) ④ **A 1-depth nesting cap** (prevents a fork bomb when the daemon spawns subagents). **Borrowing not recommended:** peer handoff (the LLM decides the next agent) = conflicts with the daemon's deterministic routing. Confidence: Claude Code figures and patterns = certain, other frameworks' coordination models = certain · some cap figures = uncertain.

**Anti-patterns (a warning common to all frameworks):** a peer groupchat free-for-all (error amplification) · a missing cycle END (infinite loop) · state explosion (bloated shared state) · missing max_turns/max_iter (cost explosion).

---

## 5. Layer D — Inter-agent communication (A2A · actor messaging · pub-sub)

**Key finding:** inter-agent messaging is a concern engram **does not have yet** (agents do not talk to each other now). A2A is excessive locally — the existing seams + bounded channels suffice.

- **A2A (Google/Linux Foundation):** for cross-vendor · organization-boundary interoperability (agent card `/.well-known`, the Task state machine, multiple protocol bindings). **For a single local daemon, the fit is judged excessive** (signing · agent cards · transport overhead bring no gain to an internal daemon — "overkill" is a fit judgment, not an established fact, and A2A's value for boundaries/interoperability is acknowledged separately). But the **Task lifecycle state machine pattern** (contextId + taskId + explicit FAILED/CANCELED) is already largely reflected in engram's `AgentStatus`/`Disposition`/`epoch`. Revisit if collaboration with external organizations' agents becomes a product requirement. Apache-2.0. Confidence: likely (secondary sources, spec text not parsed).
- **Actor messaging mechanisms:** delivery = at-most-once (Akka official · local sends can also be lost) · ordering = FIFO only per same sender-receiver pair · backpressure = a bounded mailbox. For exactly-once, you need ACK + a unique ID + idempotency. engram's `OutputSink` fanout is already isomorphic to a "mailbox receiver".
- **pub-sub/bus:** engram's event bus (ADR-0028 single-push) is a degenerate pub-sub with one topic. `AgentCommand::Subscribe{agent_id,epoch,after_seq}` already holds the logical contract of "subscribe to a per-agent topic".

**engram mapping (borrow):** when inter-agent messaging is needed → `AgentManager` = a named registry + `WriteStdin` = the routing means, an "agent-to-agent bridge `OutputSink`" (A's output → B's stdin, the core does not know the receiver · keeps ADR-0003). **The key risk (distinguish the reliability levels — Codex's finding):** a bounded `tokio::mpsc` gives **backpressure only**, not an end-to-end delivery guarantee. An ACK at the queue boundary proves only *queue acceptance*; it cannot prove PTY write success, the agent's parsing or task completion. Truly reliable messaging holds only with ① framing ② **receiver-level** message ID/ACK ③ retry rules ④ idempotency or persistence. **No unbounded channels** (OOM — tokio #4321). seq dedup + epoch defend ordering. Confidence: tokio/actor mechanisms = certain.

**Pitfalls/defenses:** **deadlock (A↔B circular wait) is not defended by supervision (Codex's correction)** — supervision detects and restarts *termination*; it cannot detect or resolve live processes waiting on each other forever. What is needed is timeouts · cancellation · a bounded request lifetime · cycle detection. · unbounded queue OOM · message storm (if an agent spawns agents, N² fanout — harmless now because it is small-scale, but fan-out control is needed once an orchestrator pattern is added) · silent drop (data loss if SinkError is not reported upward).

---

## 6. Big-direction option set (not mutually exclusive — a question of order) — **decision = the user**

Three entry directions on top of engram's existing assets (§7). Each = the minimal increment · rejected alternatives · prerequisites.

### Option 1 — Supervision first (minimal increment): runtime auto-restart (recommended starting point)
- **What:** turn the reserved fields (`RestartPolicy::OnCrash`/`restart_count`/`failed_reason`) into real behavior. After the reaper's `KeepDisableAutoRestore` disposition, if OnCrash then restart after backoff (borrow OTP intensity/period + transient). The unfulfilled part of ADR-0019 §후속2.
- **Borrow:** OTP restart types · the meltdown threshold · ractor-supervisor `backoff_fn`. **To build new:** an auto-restart task (the ladder resume→fresh→Failed already exists), a meltdown counter, a `SetRestartPolicy` command (§5).
- **Rejected alternatives:** adopting Ractor directly (OS-process mismatch → ADR-0001 complexity) · Actix (thin supervision).
- **Prerequisite:** none (the core is in a stable state). **The smallest and easiest to reverse.**

### Option 2 — Central orchestrator (Workflow-style): task delegation and graph
- **What:** a deterministic orchestrator inside the daemon where "code owns the flow, the LLM does each step" (the Claude Code Workflow model). Combine SpawnProfile/WriteStdin/subscriptions to run a task graph and collect the results. The §5 LLM brain coordinates through command handles.
- **Borrow:** Workflow's schema-validated output · adversarial verify · loop-until-dry · the 1-depth cap. **To build new:** task graph/DAG state, "returning a task result" (today there is only termination and no result channel), an internal channel from child output → the orchestrator.
- **Rejected alternatives:** peer handoff (Swarm) · groupchat (AG2) = non-determinism · error amplification → conflicts with the daemon model.
- **Prerequisite:** none — **can start right away with the existing SpawnProfile/WriteStdin/subscription APIs** (Option 3 is not a prerequisite). It is sturdier if Option 1 (supervision) exists. If task state needs to persist, borrow the §3 local journal.

### Option 3 — Messaging first: inter-agent pipes
- **What:** a `SendMessage{from,to,payload}` variant in `AgentCommand` + an agent-to-agent bridge sink. A's output becomes B's input.
- **Borrow:** the actor named registry · bounded mpsc + ACK · the A2A Task state machine concept (not full adoption). **To build new:** an addressing/routing layer · a message schema.
- **Rejected alternatives:** full A2A adoption (local overkill) · unbounded channels (OOM).
- **Prerequisite:** none, but the value is large only together with an orchestrator (Option 2) (messaging without coordination has few uses).

**Order suggestion (material — the decision is the user's · Codex's correction applied):** **1 → 2 → (3 if needed)**. The first edition's "1→3→2" was internally contradictory — the central orchestrator (Option 2) can command agents **directly** through the existing spawn/stdin/output APIs, so general peer messaging (Option 3) need not come first (putting Option 3 first would in fact trigger premature protocol and reliability work). Add Option 3 when a direct agent-to-agent pipe becomes a product requirement. However, per the §5 principle, ship an LLM command handle **together with** each step ("feature first, control later" = a violation).

---

## 7. Map of engram's existing assets (build-on vs new) — code grounding

**BUILD ON (exists):**
- Supervision: `reaper.rs` (the single consumer of death classification) + `epoch` (resubscription safety, `manager.rs:195`) + the S9 ladder `restore_one`/`fallback_fresh` (`manager.rs:358-423`) + the reserved fields `RestartPolicy`/`restart_count`/`failed_reason` (`profile.rs:159-169`) + `TerminationIntent`/`Disposition`.
- Messaging: the WS `ConnRegistry` fanout (`ws.rs`) + the extensible enums `AgentCommand`/`AgentEvent` (`protocol/messages.rs`) + injecting arbitrary subscribers into `OutputSink` + `WriteStdin`.
- Delegation: `SpawnProfile`/`SpawnByCwd` + `AgentProfile{cwd,extra_args,env}` + the command registry `window.__engramCmd` (§5 entry, the ADR-0022/0055 skeleton).

**New (no home):** inter-agent addressing · a task graph/DAG (no persistence) · the orchestrator brain (today a human is the orchestrator) · runtime auto-restart execution · an inter-agent message protocol · persistence of orchestration state · per-agent heartbeat (only termination EOF, no periodic probe).

**§5 LLM-control gap:** the current LLM paths = `window.__engramCmd.{list,run}` (skeleton) + `invoke` (AgentCommand) + a direct WS connection. **Missing handles:** `agent.send_message` · task-graph manipulation · `SetRestartPolicy` (today only `SetProfileAutoRestore`) · supervisor policy settings. ADR-0022/0055 point toward absorbing them, but work has not started.

**Related existing documents (to avoid duplicate research — must check):** `docs/research/control-surface-and-fleet.md` (fleet control surface) · `docs/research/llm-control-surface-message-command-scope-2026-06-28.md` (message system scope draft) · ADR-0022 (command registry direction) · 0028 (event bus) · 0019 (reaper · auto-restart gate) · 0055 (command registry confirmed).

---

## 8. Issues · limits · what was not covered (honest labels)

- **Cross-family parallel collection was not run** — a partial backstop from 5 single-family main-track collectors + the Codex adversarial review (omission lens, active web_search probing). Codex's verdict on probing the candidates uncovered in §8:
  - **Ray · Orleans (virtual actor) · NATS** — **do not overturn** the local-daemon recommendation (they only add a distributed runtime/broker boundary). Confirmed out of scope.
  - **systemd/supervisord/s6 (OS process supervisors)** — adoption is rejected over portability (Windows), but their *process-tree isolation model* is absorbed into §2 (underrated in the first edition → reflected in v2).
  - **Native tokio baseline (A0)** — missing in the first edition (high). Added explicitly to the Layer A table in v2 — since no framework can supervise a PTY OS process directly, A0 is the real default adoption candidate.
  - **tmux/zellij session multiplexing** — matters only if owning persistent terminal sessions and re-attaching becomes a *product requirement* (not a requirement now). Not examined in depth.
- **Version numbers and SDK maturity** are time-sensitive (likely) — must be re-checked at the time of adoption (library versions = a confident-wrong risk category).
- **A2A delivery guarantees and Restate architecture details** rest on secondary sources (source text not parsed) — not load-bearing, but check the source text before adopting.
- **Unverified details:** the internals of ractor-supervisor `RestForOne` · the exact strictness of Inngest exactly-once · the current state of engram's process-group isolation when a Unix-side grandchild process forks (§2 — checking the code is recommended).
- **Adversarial review residue:** Codex's 3 high and 5 medium findings in the first-edition BLOCK are applied and corrected in v2. The review could not overturn the direction (the option set · using existing assets · borrowing patterns rather than frameworks) (no counter-evidence) → the direction is **likely**, and individual technical details are mostly certain, excluding the unverified items above.

---

## 9. Follow-up (material → decision path)
- The user picks a §6 option direction → update that direction's TRD + ADR-0014 to **confirmed** (or a follow-up ADR) and record the rejected alternatives (§6) in the ADR.
- Option 1 (supervision) is the minimal increment and independent, so it is the pilot candidate. If chosen, implement ADR-0019 §후속2 (OnCrash) with `/implement`.
- Of the uncovered items (§8), tmux/zellij and OS process supervisors are worth a short supplementary survey before starting Option 1.
