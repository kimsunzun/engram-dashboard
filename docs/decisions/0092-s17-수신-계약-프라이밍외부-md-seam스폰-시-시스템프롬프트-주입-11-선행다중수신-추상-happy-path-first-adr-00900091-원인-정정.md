# ADR-0092: S17 receive contract — priming (external MD seam · injected into the system prompt at spawn) + 1:1 first (multi-recipient abstraction) + happy-path-first — correcting the cause in ADR-0090/0091

- 상태: 확정 (2026-07-20, basis: reinterpreting the Stage 2 pilot confound + the user's design decision) · 부분 폐기 by ADR-0215 (the backend reads the priming content)
- 관련: ADR-0086(dual entrance · immediate push · mailbox — the canonical source for the sending half) · ADR-0087(envelope/name rules) · ADR-0090(pilot execution — this ADR corrects its finding) · ADR-0091(acceptance-before-saturation order — the conclusion stays valid, the cause is corrected) · Amended by ADR-0215 (the backend reads the priming content)

## 맥락
The Stage 2 pilot (ADR-0090) was read as "a policy-compliant claude isolates an agent-to-agent message as an injection → the feature is blocked". But the user's remark showed this was a **confound**: (a) the pilot's probe body was a **textbook injection/exfiltration shape** ("remember the codeword, and later report the sender, id and codeword"), and (b) the receiving agent had **no notice at all (priming) that "you belong to a team and messages like this are legitimate".** Orchestration normally works (the spawn prompt = a trusted channel). That is, **the feature was not blocked; the receiving half (the receive contract) was simply undesigned**. The sending half (dual entrance · immediate push · `wrap_message`) is already implemented.

Standing up the happy path needs a receive contract — that is what we design now. The order in ADR-0091, "acceptance precedes saturation measurement", stays valid, but its cause is corrected from "a hard policy block" to "no priming".

## 결정
1. **Priming = an external MD seam (no hardcoding).** The priming content is not embedded in code as a string. A shared markdown file at a fixed path (`prompts/agent-priming.md`, in the repo, version-controlled) is read by a loader at spawn and **injected into the system prompt** (the daemon reads it and puts it in — the agent does not read the file itself). The code holds only the **path + the loader seam**. The current implementation = a shared-file loader, the **future implementation = a per-agent prompt-injection/skill-registration system** (absorbed by swapping the loader, "keep the road open"). The interim version **injects it into everyone unconditionally**.
2. **1:1 first, multi-recipient abstraction.** The final topology = group chat + mail (1:1 · multi-target · broadcast). Implement **1:1 first**, but let delivery flow as a "recipient list" (currently length 1) so groups and broadcast sit on top of the seam (on top of ADR-0086's reserved group). It is a low-risk, long-lived seam, so we lay it down now (CLAUDE.md §0).
3. **Format choice = a comparison spike, decision axis acceptance > tokens.** Pick by testing real send/receive on a few candidate envelope formats. **Priority 1 = acceptance/processing quality, priority 2 = token efficiency.** With priming in place, this is not fighting injection; it is a readability comparison on the happy path.
4. **happy-path-first (restructuring the ladder).** Stand up the normal-acceptance baseline first, and handle abnormal cases (saturation, injection, exact-once, etc.) after it — **if large, later; if small, implement in parallel**. ADR-0088's adversarial-first ladder is restructured from this viewpoint.
   - **Acceptance criterion:** the receiving agent naturally recognizes the message without treating it as an injection and reflects/reacts to it in context (not an immediate "received" ack).

## 거부한 대안
- **Hardcoding the priming text in code** — cannot be swapped for the future injection system, and every content edit needs a rebuild. → external MD seam.
- **The agent reads the MD file itself** — needs a separate action, and it is a different grain from the future injection system (system-prompt injection), so it would have to be torn out later. → the daemon reads it and injects it into the system prompt.
- **A 1:1-only implementation (ignoring multi)** — introducing groups or broadcast would rework the delivery path. → recipient-list abstraction.
- **Deciding the format by forgery resistance** — happy-path acceptance comes first. Forgery defense = escaping is deferred to Stage 4 (ADR-0088/0091).
- **Fixing the pilot finding as "feature blocked"** — an over-conclusion that ignored the confound (an injection-shaped probe + no priming). The receive contract is the real gap.

## 근거
- **Reinterpreting the pilot confound (user remark 2026-07-20):** orchestration works normally (the spawn task = a trusted channel), yet the pilot threw a mid-stream third-party injection-shaped probe at an agent with no priming — the isolation was a normal reaction. A natural message + priming is unverified.
- **User design decision (2026-07-20):** topology (1:1 first · multi assumed) · priming (external MD · injected into everyone · seam) · format choice (acceptance > tokens) · an approval gate on the bird's-eye view just before code.

## 영향 / 불변식
- **No hardcoding of the priming content** — always an external MD. The loader is a seam (swappable for a future injection system).
- **The delivery path keeps the recipient-list abstraction** — even 1:1 flows as a list (length 1). Narrowing to a single recipient would be a group regression.
- **No serious start on abnormal cases (saturation, injection) before the happy-path baseline passes** — do not dig into failure modes without a success mode.
- **Format decision axis = acceptance > tokens** (not forgery resistance).
- **If measuring priming acceptance is blocked by an instruction applied to the account, use an account without the instruction** — account instructions contaminate the acceptance decision (ADR-0091's account dependency stays).
- **Get the user's approval of the bird's-eye view before starting the code implementation** — a user rule in this session.
