# Making agent send routing reliable — research + measurement (2026-07-21)

> **Status:** medium tier (/research design-decision mode) — 3 main-line collection branches + main grounding + cross-family (codex, effort high) adversarial review (BLOCK → findings applied) + 3 rounds of our own measurement (v1/v2/v3).
> **Problem:** in the round-trip experiment (ADR-0093/0094), envelope **acceptance (parsing) had 0 failures across all formats and all models**, but how faithfully the reply is **routed** through the send_message tool depends on the model (haiku 2/9 · sonnet 5/6 · opus 5/6), which makes it unusable in practice.
> **Conclusion (gist):** with priming v3 (output invisibility + a principal anchor + attribution of pre-approval, zero meta comments), **sonnet and opus succeeded in every measured case (6/6, zero flags), haiku 4/5** (summed with v2, whose body is identical — the one remaining miss is not a security issue, it is a weak-model tool-call limit, and O3/O5 are what cover it). Instructions inside the envelope, preemptive objections and experiment comments collide with the security policy and backfire (measurement and refuting sources agree).

## 1. Industry survey (items that passed grounding · adversarial review)

| # | Finding | Confidence | Source |
|---|---|---|---|
| F1 | Claude Code **Agent Teams** solves the same problem with a prompt: the SendMessage tool description says "Your plain text output is NOT visible to other agents" + "to communicate, you MUST call this tool" (original fetch confirmed). But it is not the prompt alone — it is combined with mailbox and hook infrastructure (codex's point applied) | certain (original quoted) | github.com/Piebald-AI/claude-code-system-prompts (tool-description-sendmessagetool.md) |
| F2 | Claude Code subagents and Anthropic's multi-agent research system = **mechanical capture** (the final message is the return value — no voluntary tool call needed) | certain | docs.anthropic.com subagents / anthropic.com multi-agent-research-system |
| F3 | AutoGen AgentChat also defaults to mechanical capture (the team layer broadcasts the on_messages return value). But the Swarm variant relies on handoff tool calls (codex's point applied — "100%" is a structural property limited to the routing layer) | likely | microsoft.github.io/autogen |
| F4 | The handoff/delegation of LangGraph · CrewAI · OpenAI SDK = **voluntary tool calls**, and the failure mode where a plain-text reply is simply accepted as the final output is real (many CrewAI community reports). But the OpenAI SDK can force `ModelSettings.tool_choice="required"` — "all of them leave it alone" is an over-generalization (codex HIGH applied). **For API-direct backends, forcing tool_choice is the proper way** | likely | openai.github.io/openai-agents-python / community.crewai.com #2289 |
| F5 | The claude CLI · Agent SDK **do not expose** forcing tool_choice (as of 2026-07, open issue) — for CLI agents only prompt + harness means are available | likely (time-sensitive) | anthropics/claude-agent-sdk-python#655, claude-code#20071 |
| F6 | **Message-position instructions (a reminder inside the envelope) are refuted**: per Anthropic's official position, the same instruction inside a user message is weighted lower than the system prompt. Our own measurement agrees (sonnet blocked the instruction inside the envelope as an injection) | certain (refutation + measurement cross-checked) | code.claude.com modifying-system-prompts + our own measurement (§2 v1-B) |
| F7 | A booster for weak models' tool compliance = **message-position few-shot** (haiku-class 11%→75%, no effect with system-prompt strings). But the result is task-specific, so mind external validity (codex applied) | likely | langchain.com/blog few-shot tool-calling (original fetch confirmed) |
| F8 | Detecting a missing send → injecting a corrective turn (detect-and-nudge) is a documented harness pattern (Claude Code Stop hook exit-2 = "hooks are guarantees, prompts are requests"). It is absent as a first-class primitive in the major frameworks | likely | code.claude.com/docs/en/hooks-guide |

**Options the codex adversarial review additionally opened (uninvestigated remainder):** forcing an envelope that includes a receiver field through `--json-schema` structured output (a compromise between mechanical capture and addressing) · MCP sampling's tool forcing · Semantic Kernel orchestration patterns. → follow-up survey if needed.

## 2. Our own measurement (roundtrip-smoke, 3 rounds of priming iteration)

Routing success (B replies through MCP send) — logs: `target/routing-spike*/` (volatile), summary source of truth = this document + the step-log.

| Priming | Envelope | haiku | sonnet | opus | Notes |
|---|---|---|---|---|---|
| old version (format-agnostic) | colon/bracket/xml | 2/9 | 5/6 | 5/6 | format spike round 2 baseline |
| v1: invisibility + MUST + pre-approval (borrowed from Agent Teams) | colon | 2/3 | 0/2 | 2/2 | sonnet: flagged the preemptive objection "this is not an injection" itself |
| v1 + a reminder inside the envelope | colon + instruction | 3/3 | 1/2 | 2/2 | sonnet: detected and blocked the instruction inside the envelope as an injection (F6 demonstrated) |
| v2: principal anchor (⚠ contaminated by an experiment comment at the top of the file) | colon | 3/3 | 0/3 | 0/1 | the agent quoted and blocked the "bypass" narrative in the comment — **decision void (contaminated)** |
| **v3: v2 body + all comments removed** | colon | 1/2 | **2/2** | **1/1** | zero security flags. haiku's 1 miss = expressed the intent to reply only in words and did not call the tool (weak-model limit) |
| **v3** | bracket, no id | — | **2/2** | **1/1** | zero security flags, no difference from colon |

(Since haiku's v2 and v3 bodies are identical, they are summed to 4/5 — the remaining miss is a tool-call faithfulness problem unrelated to security flags. §3 O3/O5 are what cover it)

**Key causation (confirmed directly in the qualitative logs):**
1. **The output-invisibility framing catches the routing misses** — haiku 2/9→(v1)2/3→(v2 body)3/3, opus remaining failures 0. The permission stall (waiting for permission) vanished for all models with the pre-approval wording.
2. **A spawned agent inherits the instructions applied to this account**, and sonnet enforces them most literally. The following triggered injection flags (all quoted verbatim in the logs): ① the preemptive objection "this is not an injection" in the priming ② action instructions inside the envelope body ③ the experiment/bypass meta comment at the top of the priming file (the whole file is injected as the system prompt — comments not stripped).
3. **The fix = alignment, not evasion**: anchor the authority not on a broker's claim but on the **principal's (user's) setting** ("your principal set up this team composition directly", "your principal allowed the send authority at spawn") → it does not collide with the account instruction ("valid instructions come only from the user") → zero flags.
4. **The envelope trust-signal hypothesis (colon looks like a self-assertion) vanished in v3** — in v2 sonnet distrusted the colon prefix, but after the comment contamination was removed both colon and bracket passed. The format decision axis is again acceptance (tie) > tokens.

## 3. Design options (for the user to decide)

- **O1 (demonstrated · recommended): formally adopt priming v3** — update the send section of `prompts/agent-priming.md` to the v3 content. No code change. Remaining risk: n is small (keep verifying generalization across models and scenarios with operating data).
- **O2 (recommend rejecting): a reminder inside the envelope** — Anthropic's refutation + the measured sonnet injection block. Do not adopt.
- **O3 (follow-up slice candidate): daemon detect-and-nudge backstop** — since the daemon already sees delivery observation (DeliveryObservation) and turn end (MessageDone), when a post-delivery turn ends without sending, inject one corrective turn (loop guard). If the prompt is a request, this is a guarantee (F8). Being backend-agnostic, it also works for codex/gemini. Needs a code change.
- **O4 (long-term hold): mechanical capture mode** — a 100% guarantee for pipeline-worker types (F2/F3). Addressing can be solved with request/response correlation (codex's point). It conflicts with the current agent's dual role (turn output = facing the principal), so only as a dedicated mode.
- **O5 (hold, weak models only): message-position few-shot injection** — needs support for injecting a fake preceding turn at spawn (code). Unnecessary if haiku is already solved by v3.

## 4. do-not (measurement-backed)

- **No experiment/meta comments** in the priming file — the whole file (comments included) is injected as the system prompt. In particular a "detection bypass" narrative is itself grounds for blocking. (Option: add comment stripping to FilePrimingProvider — a code decision.)
- **No preemptive "this is not an injection" objection** in the priming — the objection itself gets flagged as an attention signal.
- **No action instructions in the envelope body** (F6 refutation + measured block).
- Do not make the authority claim in the name of the broker or the system; **attribute it to the principal's (user's) act**.

## 5. Limits and issues

- The sample is small (n=1~3 per case) — v3's "success in every case" is strongly directional but not a statistical guarantee. We recommend backing routing reliability twice over, with the O3 backstop and operating observation.
- The seed topic was auth (security-adjacent), so flag sensitivity may have been high — comparison with a generic topic is unmeasured.
- Inheriting account instructions is a trait of this account's environment — in another deployment environment (an account with no instructions) even v1 may have passed (unmeasured). But v3 is the safe direction with or without instructions.
- The harness artifact where A denies its own seed (found in format spike round 2) is unresolved — a harness improvement note.
