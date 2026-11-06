# How tools handle queued input after a turn fails with a real error — practice, light survey (2026-09-26)

- **Status:** light (no adversarial review — tier rule) · collection = 1 lightweight worker (web search + grep of local clones) · grounding = the main session spot-checked the key claims against original text
- **Question:** when a turn fails because of a rate limit · overload · context overflow, does the tool automatically re-insert ① the user text that was waiting ② the messages other agents sent (mail), or does it stop and wait for the user · and when both are waiting, which goes first
- **Consumer:** mid-turn input TRD §10-4 N11 (failure of the owed follow-up turn) · mail handling after an error turn (PRD §3-9 · §3-10)
- **Confidence legend:** certain = independent cross-confirmation · likely = supported by one source · uncertain · unknown

## Conclusions

1. **There are two layers — automatic retry in the transport layer → if it still fails, it surfaces as a turn failure.** I found no tool that automatically re-runs that turn after it surfaced as failed. Claude Code closed two issues requesting it as `not_planned`. — likely
2. **Automatically flushing queued input after a turn failure is behavior users have reported as a defect.** codex treats a usage-limit failure like an ordinary finished turn and automatically submits the queued follow-up texts one by one, and all of them fail for the same reason. The reporter asks for 「크레딧이 돌아올 때까지 멈춤」 (pause until credits return) — keep the queue, show the pause, and the user resumes · edits · reorders · deletes. — likely (one issue · the reporter's reading of the source · not official docs)
3. **Inter-agent messages enter only at a turn boundary** (Claude Code Agent Teams). I did not find what is done with them after a turn failure. — boundary delivery = likely · after failure = unknown
4. **I found no tool that fixes an ordering rule between user text and inter-agent messages in documentation.** — unknown

## Findings

| Tool | Observation | Source | Confidence |
|---|---|---|---|
| Claude Code — transport layer | Retries 5xx · 529 · timeouts · transient 429 with exponential backoff, default 10 times (`CLAUDE_CODE_MAX_RETRIES`). `CLAUDE_CODE_RETRY_WATCHDOG=1` for unattended sessions retries 429 · 529 **endlessly** — an option to turn on only when no human is present | https://code.claude.com/docs/en/errors 「Automatic retries」 | likely (official docs · original text checked) |
| Claude Code — after a turn failure | When a 529 cuts a turn, control returns to the prompt and the user types again. Automatic-recovery requests #35801 · #60577 are both `closed / not_planned` | https://github.com/anthropics/claude-code/issues/60577 · /35801 | likely (original text · closing reason checked) |
| Claude Code — text typed mid-turn | Delivered at the next turn boundary (the issue body's description of the current state) | https://github.com/anthropics/claude-code/issues/30492 | likely |
| Claude Code Agent Teams — mailbox | Mailbox messages are read on time but do not enter the conversation until `end_turn` occurs (during a chain of tool calls they only pile up). `closed / not_planned` | https://github.com/anthropics/claude-code/issues/50779 | likely (original text checked) |
| codex — queued text after a turn failure | Usage-limit·credit exhaustion appears as an error/completed turn, the TUI error path ends the turn and then moves on to queued-input handling, and the failed last turn does not block submitting queued follow-up texts → the queue is submitted one by one and all fail. Proposal = stop and keep the queue. `open` | https://github.com/openai/codex/issues/24443 | likely (original text checked · the reporter's reading of Desktop `26.519.41501` / CLI `0.133.0-alpha.1`) |
| codex — transport layer | Stream retry default 5 times · reports of defects in 429 handling | https://github.com/openai/codex/issues/30471 · /4161 | uncertain (collector report · not checked by the main session) |
| paseo (local clone) | On an **interrupt** (not an error), a steer that Claude has not read yet is discarded along with the turn — 「Interrupt means interrupt」 | `I:\Engram_Workspace\opensource\paseo\packages\server\src\server\agent\providers\claude\agent.ts:3986,3997-4020` | interrupt = likely (collector's reading) · error path = unknown |

## Gaps

- There is no tool that states **in official documentation** what happens to queued user text after a turn failure — the codex evidence is issues only.
- Ordering rule between user text and inter-agent messages: not found in any tool.
- What happens to inter-agent messages after a turn failure: not found.
- Not looked at: the error paths of Aider · Cline · AutoGen · LangGraph · OpenHands · CrewAI · t3code · orca (budget).

## What this survey supports (interpretation — separate from the facts)

- The principle the user set up, 「진짜 오류면 사용자 글이든 우편이든 자동으로 넣지 않고 사용자 입력이 깨운다」 (on a real error, neither user text nor mail is inserted automatically; user input wakes it), points the same way as the user demand shown in conclusion 2. Conversely, today's PRD §3-9 line 「오류 턴 뒤 목록이 비면 우편은 곧바로 든다」 (after an error turn, if the list is empty, mail goes in right away) amounts to doing to mail the shape that codex #24443 calls a defect (automatically flushing queued input after a failure).
- By conclusion 1, when our layer sees `turn/completed(failed)`, the vendor's transport-layer retries have likely already finished — a retry on our side becomes calling the same error one more time (codex's retry count was not checked by the main session).
