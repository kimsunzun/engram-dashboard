# Messaging v1 over-implementation audit — peer survey (2026-07-28)

> **Origin**: the user's concern "the rules are too complex and it feels like only we are over-implementing" → a full re-investigation with `/research deep`.
> **Method**: 3 collection branches in parallel (15~25 searches each) → main grounding (9 direct verifications of key claims against the original sources) → cross-family (GPT·web_search) adversarial review (30 items raised, draft judged UNSOUND) → 4 re-verifications of the reviewer's counter-evidence → corrected synthesis.
> **Where the conclusion is used**: user decision 2026-07-28 — **"too much" concern withdrawn, v1 kept, switched to learning**. (§5 below)
> Confidence labels: ●certain (cross-confirmed) ◐likely ○uncertain.

## 1. Conclusion in one line

"Only we are complex" is only half right — **newcomer peers are each re-inventing the same parts (queue, timeout, state tracking, retirement) in 2025~26**, so the direction is not over-implementation. But the claim that each individual axis is "inevitable" does not hold (several are **choices** that come from product goals), and for the body of the perceived complexity (the internal precision machinery) simpler alternatives really exist.

## 2. Findings by branch

### A. LLM multi-agent frameworks (~25 surveyed)
- Reply deadline + timeout notification: absent in the major frameworks (AutoGen, LangGraph, CrewAI, OpenAI SDK, SK, MetaGPT, etc.). But **not absent in all** — the adversarial review raised counter-examples (see D below).
- Mailbox capacity cap/TTL: absent in the mainstream frameworks.
- Asynchronous delivery to a busy agent: more common than expected — AutoGen Core asyncio queue, MetaGPT message pool, Letta async send + delivery receipt, A2A push. Conversational frameworks (CrewAI, the handoff family) are synchronous turn-passing only.
- ● A2A spec (original text checked): has messageId/taskId/contextId + task state/history, **no deadline, TTL or queue-size field**.
- Sender identity authentication: none in the in-process family, only cross-organization protocols (A2A Bearer/mTLS).

### B. Same-constraint peers (agent = a separate process, a long-lived session)
- ● **Naive injection (stdin ignoring turns) is a failure the community itself documented**: "~70-80% reliable", "message clobbering" (agent-orchestrator#853, original text checked). Its redesign adopted 3-layer injection, epoch fencing and an append-only JSONL inbox — converging in the same direction as us.
- ● **Claude Code Agent Teams** (official docs checked): file mailbox + automatic delivery + TeammateIdle hook. Correlation id, deadline, groups (send one message per recipient), session persistence (sending to a ghost on resume) — all absent. A real incident where one corrupted entry blocked delivery of that whole mailbox → fixed in v2.1.207. **Evidence that the price of simplicity is real.**
- ● LangGraph Agent Server: `multitask_strategy` defaults to enqueue (per-recipient FIFO) — original text checked. (But it is a server endpoint default, not the core graph default — review point applied)
- ● Turn/idle detection is a spot with many real incidents: 2 GitHub Copilot SDK postmortems (a fixed 60s idle timeout killed in-progress sessions by mistake — a 6~8 day outage / the orchestrator's turn end mistaken for full completion ~30%). Exactly the same family of problem as our busy valve and the unverified nested-Task item.
- Offline delivery: fail-fast (AutoGen UndeliverableException) or avoiding the transport (AgentMail = email). No one sells "indefinite parking + automatic delivery on spawn" as first-class (Temporal is the non-LLM ceiling).

### C. Mature messaging infrastructure baseline
- Universal minimum set: per-channel FIFO-like ordering + a reply-correlation **pattern** (native field in half: email Message-ID/In-Reply-To, AMQP correlation_id, FIPA reply-with, Matrix relates_to) + a failure signal toward the sender (bounce/DSN/DLX/DLQ).
- Family split: **TTL and capacity cap = standard in the queue/broker family** (Postfix 5d, SQS 4d, Kafka 7d, AMQP max-length) vs **the actor family is deliberately unbounded** (Erlang mailbox unbounded — OOM risk documented).
- Deadline watching: infrastructure puts it on **plain sends** nowhere. Where it exists it is all an opt-in request-response wrapper (Erlang gen_server:call, Akka ask, NATS request/reply, gRPC deadline) — our reply_by is also opt-in, so it is structurally in this class.
- Envelope-level sender authentication: rare (XMPP JID stamp, Matrix signature, SQS SigV4). In SMTP, SPF/DKIM is a later bolt-on.

### D. Adversarial review (GPT · 30 items raised · draft UNSOUND) + re-verification of counter-evidence
The draft's universal negative sentences of "absent in all" were shot down by counter-examples. The result of directly re-verifying the counter-examples:
- ● **OpenClaw**: subagent async execution + `runTimeoutSeconds` + on overrun pushes a "timed out" status to the parent + run history + 60-minute auto-archive and stale-run retirement. (The claim of queue cap 20 and an overflow policy is ○unconfirmed — doc redirect)
- ● **AgentScope** (Alibaba): `timeout_seconds` on `agent_send` (default 30s, max 600s) + task state persistence and query. MsgHub dynamic group (a late participant does not receive past broadcasts = send-time membership).
- ● **MCP Agent Mail** (2k★): a mailbox MCP server for coding agents — threading, reply correlation, ack_required. (Quota/TTL enforcement ○not found — the reviewer also overstated)
- Nuance: the peers' timeouts are all of the form of a **delegated-task (parent→child run) timeout**. Our combination of **an absolute deadline on an arbitrary peer message + a reply-debt ledger + notification-debt retirement** is still not found — the reviewer's wording: "unusual but not demonstrated to be unique/necessary".
- Method corrections accepted: the FIPA "the sender watches" wording is overstated (the spec leaves the watching party unspecified) · the "modal solution" assertion has too small a sample · the ESSENTIAL judgment is a leap from lineage to necessity · the number of tools is not grounds to rebut complexity (the complexity is in the internal state machine).

## 3. Corrected per-axis judgment (3 categories)

| Category | Axis | Basis |
|---|---|---|
| **Close to inevitable** (same-constraint peers converge + failure demonstrated) | parking queue/FIFO / busy gate / bounded (any form) / sender identity stamp | ● naive injection failure documented + Agent Teams incident + all peers converge |
| **A choice that comes from product goals** (evidence does not force it) | the whole request contract / reply_by / groups / history ring depth / notice lane | ◐ justified if one grants the goals (external memory for a forgetful LLM, self-awakening, drift-free lookup) — the goal itself is the basis, not the peers |
| **Precision machinery** (a simpler alternative really exists) | mark-and-sweep retirement (alternative: bounce when the cap is reached) / in-flight accounting / busy 30-minute valve / notice +1 allowance | correctness cost — it was a real candidate for reduction, but kept by the §5 decision |

## 4. Limits
OpenClaw queue details ○ · MS Durable Agents ○unverified · no second adversarial review of the corrected report (the reflection of the first 30 items stands in for it) · Slack/Discord internals not public · academic survey tables (arxiv 2504.16736, etc.) not cross-checked.

## 5. User decision (2026-07-28)
- **"Too much" concern withdrawn — v1 kept as is**, no reduction.
- Instead, switch to a **user learning plan**: after studying the system deeply, the user will pick out anomalous spots directly.
- MCP Agent Mail = a direct peer → a reference-research-worthy item added to the backlog.

## Sources (key ones only — the full list per branch is in the collection log)
A2A spec: a2a-protocol.org/latest/specification · Agent Teams: code.claude.com/docs/en/agent-teams · agent-orchestrator#853: github.com/AgentWrapper/agent-orchestrator/issues/853 · LangGraph: docs.langchain.com/langsmith/agent-server-api/thread-runs/create-background-run · OpenClaw: docs.openclaw.ai/tools/subagents · AgentScope: java.agentscope.io/v1/en/docs/harness/subagent.html, doc.agentscope.io/tutorial/task_pipeline.html · MCP Agent Mail: github.com/Dicklesworthstone/mcp_agent_mail · Copilot SDK postmortem: github.com/github/gh-aw/issues/39310, github.com/github/copilot-sdk/issues/1275 · infrastructure: postfix.org·rabbitmq.com/docs/maxlength·docs.aws.amazon.com(SQS)·erlang.org·doc.akka.io·fipa.org/specs/fipa00061·xmpp.org/extensions/xep-0160·spec.matrix.org · NATS/gRPC/ROS2: docs.nats.io·grpc.io/docs/guides/deadlines·docs.ros.org(QoS)
