# ADR-0092: S17 receive contract — priming (external MD seam · system-prompt injection at spawn) + 1:1 first (multi-receive abstraction) + happy-path-first — correcting the cause in ADR-0090/0091

- 상태: 확정 (2026-07-20, basis: Stage 2 pilot confound reinterpretation + user design decision) · 부분 폐기 by ADR-0215 (the backend reads the priming content)
- 관련: ADR-0086(dual entrance · immediate push · mailbox — canonical source for the sending half) · ADR-0087(envelope/name rules) · ADR-0090(pilot run — this ADR corrects its finding) · ADR-0091(acceptance-before-saturation order — conclusion valid, cause corrected) · Amended by ADR-0215 (the backend reads the priming content)

## 맥락
The Stage 2 pilot (ADR-0090) was read as "a policy-compliant claude isolates inter-agent messages as injection → the feature is blocked". But the user's remark revealed this was a **confound**: (a) the pilot's probe body was a **textbook injection/exfiltration shape**: "remember the codeword and later report the sender, id and codeword", and (b) the receiving agent had **no guidance at all (priming) that "you belong to a team and such messages are legitimate".** Normally orchestration works fine (spawn prompt = trusted channel). That is, **the feature was not blocked; the receiving half (the receive contract) was undesigned.** The sending half (dual entrance · immediate push · `wrap_message`) is already implemented.

To establish the happy path, a receive contract is needed — this is the design target now. The order in ADR-0091 that "acceptance comes before saturation measurement" is valid, but its cause is corrected from "a policy hard block" to "absence of priming".

## 결정
1. **Priming = an external MD seam (no hardcoding).** Do not embed the priming content in code as a string. At spawn the loader reads a shared markdown at a fixed path (`prompts/agent-priming.md`, repo · version controlled) and **injects it into the system prompt** (the daemon reads and inserts it — not the agent reading the file itself). The code has **only the path + the loader seam**. The current implementation = a shared-file loader, **the future implementation = a per-agent prompt injection/skill registration system** (absorbed by swapping the loader, "the road is cleared in advance"). The interim version **injects unconditionally into everyone**.
2. **1:1 first, multi-receive abstraction.** The final topology = group chat + mail (1:1 · multi-target · broadcast). Implement **1:1 first**, but let delivery flow through a "recipient list" (currently length 1) so groups and broadcast sit on the seam (on top of the group reserved of ADR-0086). It is a low-risk, long-horizon seam, so lay it down now (CLAUDE.md §0).
3. **Format choice = a comparison spike, decision axis acceptance > tokens.** Pick by running real send/receive tests on a few candidate envelope formats. **First priority = acceptance · processing quality, second priority = token efficiency.** Since priming is already laid down, it is not fighting injection but comparing readability on the happy path.
4. **happy-path-first (restructure the ladder).** Establish the normal acceptance baseline first, and abnormal cases such as saturation · injection · exact-once come after — **defer if large, implement in parallel if small**. Restructure the adversarial-first ladder of ADR-0088 from this view.
   - **Acceptance criterion:** the receiving agent naturally recognizes it without treating it as injection and reflects/reacts to it in context (not an immediate "received" ack).

## 거부한 대안
- **Hardcoding the priming text in code** — cannot be swapped for a future injection system, and every content edit needs a rebuild. → external MD seam.
- **The agent reads the MD file by itself** — needs a separate action + differs in kind from the future injection system (system-prompt injection), so it would have to be torn up later. → the daemon reads it and injects it into the system prompt.
- **A 1:1-only implementation (ignoring multi)** — reworks the delivery path when groups and broadcast are introduced. → recipient-list abstraction.
- **Deciding the format by forgery resistance** — happy-path acceptance comes first. Forgery defense = escaping is deferred to Stage 4 (ADR-0088/0091).
- **Finalizing the pilot finding as "the feature is blocked"** — an over-conclusion that did not reflect the confound (an injection-shaped probe + absent priming). The receive contract is the real blank.

## 근거
- **Pilot confound reinterpretation (user remark 2026-07-20):** orchestration works normally (spawn task = trusted channel), but the pilot threw a mid-stream third-party injection-shaped probe at an agent without priming — isolation is the normal reaction. A natural message + priming is unverified.
- **User design decision (2026-07-20):** topology (1:1 first · assume multi) · priming (external MD · inject into everyone · seam) · format choice (acceptance > tokens) · a bird's-eye-view approval gate right before code.

## 영향 / 불변식
- **No hardcoding of the priming content** — always an external MD. The loader is a seam (swappable for the future injection system).
- **Keep the delivery path = the recipient-list abstraction** — even 1:1 flows as a list (length 1). Narrowing to a single recipient regresses groups.
- **No full-scale start on abnormal cases (saturation · injection) before the happy-path baseline passes** — do not dig into failure modes without a success mode.
- **Format decision axis = acceptance > tokens** (not forgery resistance).
- **If the priming acceptance measurement is blocked by an instruction applied to the account, use an account with no instructions** — account instructions contaminate the acceptance decision (keeps ADR-0091's account dependence).
- **Get the user's approval of the bird's-eye view before starting code implementation** — this session's user rule.
