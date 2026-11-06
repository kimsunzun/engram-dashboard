# Research — OSS survey of agent messaging (traditional → agentic) + a rough decision on merging

**Status:** design-decision mode. cross-family (Claude Sonnet fan-out 2 + Codex blind) + opus adversarial verification. **medium.**
**Date:** 2026-06-28 · author: dashboard1 (wip/a1) · **reference** (adoption = the owner's PRD decision).
**Confidence legend:** certain / likely / uncertain.

> Purpose: how do messages get delivered between agents/components — survey **from traditional messaging (MQ · actor · channel) to agent flow (A2A · orchestration)** and give a final recommendation. + a rough decision on "do we merge it into one bus with the command bus".

## PART A — Traditional messaging

| Candidate | Mechanism | Delivery guarantee | engram fit | License/maturity |
|---|---|---|---|---|
| **tokio channels** (mpsc/broadcast/watch) | in-proc async channels | mpsc=backpressure (bounded)/broadcast=Lagged loss/watch=latest value | ★ **best for local in-proc, zero dependencies, already in use**. Cannot cross a process boundary | MIT/very high |
| **Ractor** (Rust actor) | actors on Tokio, mailbox (4 priority lanes), supervision, PG routing, optional cluster | at-most-once (estimate · uncertain) | ★ OTP-style supervision transplant · remote path. But **default unbounded mailbox = no backpressure** (watch the high-frequency PTY output) | MIT/v0.15 · active |
| Actix | its own Arbiter, Handler<M>, bounded mailbox | at-most-once | best local performance, but **no remote · development stalled · friction with Tokio** | MIT/Apache · stalled |
| **NATS/JetStream** | subject pub/sub, JetStream = persistence · replay · ack | core at-most-once / JS at-least-once · exactly-once-like | ★ when scaling to remote. For today's local setup an **external server = over-spec**. A transport seam swap candidate | Apache-2.0/CNCF · high |
| Redis pub/sub | channel pub/sub | at-most-once (Streams strengthens) | ✗ external server · **license change (RSAL/SSPL/AGPL)** · not designed for messaging | caution/high |
| MQTT | topic pub/sub, QoS 0/1/2 | QoS selectable | △ QoS is an advantage but the IoT abstraction is awkward · thin Rust ecosystem | Apache/MIT · medium |
| RabbitMQ/AMQP | exchange→queue routing, ack | at-least-once | ✗ requires an Erlang server · heavy | MPL-2.0/very high |
| ZeroMQ | brokerless sockets (inproc/ipc/tcp) | at-most-once | △ lowest local latency but no persistence · no guarantees · Rust-native maturity uncertain | LGPL (crate MIT)/medium |

## PART B — Agentic (LLM multi-agent)

| Candidate | Mechanism | engram fit | Maturity |
|---|---|---|---|
| **A2A protocol** | Agent Card · task · message · artifact · streaming, JSON-RPC/gRPC/REST | ★ for an **external interoperability boundary adapter**. Unsuitable for internal PTY routing | LF project/new (likely) |
| **LangGraph** | state graph · supervisor · Command handoff · persistence · interrupt | ★ reference for the internal topology (supervisor + handoff + shared state). Python-first → across a process/API boundary | OSS/mature (agent ecosystem) |
| **OpenAI Agents SDK / Swarm** | handoff as tool · typed input · history filter · tracing · guardrail | ★ reference for the handoff model (handoff/routing). Python/TS-first | MIT/current (Swarm is experimental) |
| MS Agent Framework/AutoGen | conversational message · group chat · workflow | for reference only (.NET/Python). Early v1.0 = maturity short | MS/new to mature, mixed |
| CrewAI | role/task/crew · delegation | reference for the mental model. Python-first | OSS/medium |

**★ What agentic messaging requires beyond traditional messaging (certain):** a typed semantic envelope (task · message · artifact · tool_call · observation), capability discovery, identity/role/authority, conversation/task lifecycle, correlation ID, context/memory policy, artifact/file references, **human approval gate**, cancellation/interrupt, handoff/routing policy, audit/trace/eval metadata, **idempotent replay** (LLM actions are non-deterministic). → That is, a structure that puts an **"agentic layer"** on top of traditional messaging.

## Cross-check table (Claude ↔ Codex)

| Claim | Claude | Codex | Decision |
|---|---|---|---|
| For now, tokio channels in-proc (behind a transport seam trait) | ✓ | ✓ `Transport` trait + delivery classes (ephemeral/durable/state_latest/command_rpc) | **converged · certain** |
| Remote/persistent future target = NATS JetStream | ✓ | ✓ | **converged · certain** |
| Exclude Redis/RabbitMQ/MQTT (beyond current needs/license) | ✓ | ✓ | **converged · certain** |
| PTY agent = Ractor-style supervision | ✓ (watch backpressure) | ✓ (a small layer rather than a deep adoption) | converged · likely |
| A2A = external boundary only, not the internal bus (yet) | ✓ | ✓ | **converged · certain** |
| The agentic layer separate, on top of the transport | ✓ (supervisor + handoff) | ✓ `AgentId/Capability/Task/Message/ArtifactRef/Handoff/Approval/RunTrace` | **converged · certain** |

**No disagreement.** Codex made the envelope types, delivery classes and append-only log more concrete.

## ★ Merge vs separate (rough cross-note — the key point the owner asked)

**Decision: logically separate (control plane / data plane), the physical transport may be shared.** (Codex explicit, Claude consistent — likely)
- The **command bus** (control plane) needs authorization · priority · cancellation · a bounded schema · the meaning "do not confuse with chat".
- **Agent messages** (data/message plane) need replay · context · artifacts · loose evolution.
- The received wisdom of mature distributed design = separate the control plane from the data plane. But early on both may sit on the **same physical transport (WS/Tokio/later NATS)** — the separation is at the *logical/schema* level.
- Adversarial verification: "for a single-machine dashboard, is one bus simpler?" → true early on, but the auth · cancellation · schema-confusion risks are large, so splitting at the seam fits low-risk over-engineering (§0). **Keep them separate.**

## Final recommendation (engram messaging)

1. **Now:** in-proc = **tokio channels** placed **behind a `Transport` (or the existing sink/transport) seam trait**, with **delivery classes explicit** (ephemeral/durable/state_latest/command_rpc). Important agent events go to an **append-only local log**.
2. **Agent model:** PTY agents as **supervised actors** (the Ractor-style concept; a small layer rather than heavily adopting Actix). Use a **bounded mailbox alongside** (backpressure for high-frequency PTY output).
3. **Agentic layer (on top of the transport):** the types `AgentId·Capability·Task·Message·ArtifactRef·Handoff·Approval·RunTrace` + a supervisor/handoff topology (transplant the LangGraph · OpenAI SDK patterns). **No free peer discussion** (an orchestrator is mandatory).
4. **External boundary:** track the A2A shape and use it as an adapter (external interoperability). Not as the internal bus yet.
5. **When remote is needed:** swap the transport seam to **NATS JetStream**. (Consistent with ADR-0020 transport semantics · ADR-0028 single-push)
6. **Logically separate from the command bus** (the cross-note above).

## Rejected candidates → ADR rejected-alternative candidates

Redis (license · design mismatch) · RabbitMQ/AMQP (Erlang server · heavy) · MQTT (awkward IoT abstraction) · Actix (no remote · stalled) · ZeroMQ (no persistence/guarantees · Rust maturity uncertain) · A2A as the *internal* bus (still new · unsuitable for internal PTY).

## Gaps and limits

- Unanimity ≠ correct: both families converge strongly on "tokio now + NATS later + actor supervision + a separate agentic layer" — these are well-established Rust/distributed patterns, so the risk is low, but we acknowledge the possibility of a shared bias.
- Ractor's at-most-once delivery guarantee = **not stated in the docs (uncertain)** — needs measurement if adopted.
- Most agentic frameworks are Python/TS-first — a Rust port borrows patterns, it is not a direct dependency.
- A2A and MS Agent Framework are new — maturity changes need tracking.

## Sources

tokio sync https://docs.rs/tokio/latest/tokio/sync/ · Ractor https://github.com/slawlor/ractor · https://docs.rs/ractor/ · Actix https://docs.rs/actix/ · NATS JetStream https://docs.nats.io/nats-concepts/jetstream · Redis pubsub/license https://redis.io/docs/latest/develop/pubsub/ · MQTT https://mqtt.org/ · RabbitMQ confirms https://www.rabbitmq.com/docs/confirms · A2A https://a2a-protocol.org/latest/specification/ · LangGraph https://github.com/langchain-ai/langgraph · OpenAI Agents SDK handoffs https://openai.github.io/openai-agents-python/handoffs/ · MS Agent Framework https://learn.microsoft.com/en-us/agent-framework/ · CrewAI https://docs.crewai.com/en/concepts/collaboration · actor library comparison https://tqwewe.com/blog/comparing-rust-actor-libraries/
