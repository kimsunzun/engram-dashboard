# ADR-0092: S17 receiving contract — priming (external MD seam, system-prompt injection at spawn) + 1:1 first (multi-recipient abstraction) + happy-path-first — corrects the cause in ADR-0090/0091

- 상태: 확정 (2026-07-20, basis: reinterpretation of the Stage 2 pilot confound + user design decision) · 부분 폐기 by ADR-0215 (the backend reads the priming content) · 부분 폐기 by ADR-0327 (Decision 1, the priming source path)
- 관련: ADR-0086 (dual entrance, immediate push, mailbox — the canonical source for the sending half) · ADR-0087 (envelope/name rules) · ADR-0090 (pilot run — this ADR corrects its finding) · ADR-0091 (acceptance-before-saturation order — conclusion stands, cause corrected) · Amended by ADR-0215 (the backend reads the priming content) · Amended by ADR-0327 (Decision 1, the priming source path)

## 맥락
The Stage 2 pilot (ADR-0090) was read as "a policy-compliant claude isolates inter-agent messages as injection → the feature is blocked". But the user pointed out that this was a **confound**: (a) the pilot's probe body had a **textbook injection/exfiltration shape** ("remember the codeword and later report the sender, id and codeword"), and (b) the receiving agent had **no guidance at all (priming) saying "you belong to a team and messages like this are legitimate".** Orchestration normally works (the spawn prompt = a trusted channel). That is, **the feature was not blocked; the receiving half (the receiving contract) was undesigned.** The sending half (dual entrance, immediate push, `wrap_message`) is already implemented.

To set up the happy path we need a receiving contract — that is what we are designing now. The order in ADR-0091, "acceptance comes before saturation measurement", stands, but its cause is corrected from "a hard policy block" to "absence of priming".

## 결정
1. **Priming = an external MD seam (no hardcoding).** The priming content is not embedded in code as a string. A shared markdown file at a fixed path (`prompts/agent-priming.md`, in the repo, version-controlled) is read by a loader at spawn and **injected into the system prompt** (the daemon reads and inserts it — the agent does not read the file by itself). The code holds only **the path + the loader seam**. The current implementation = a shared-file loader, the **future implementation = a per-agent prompt injection/skill registration system** (absorbed by swapping the loader; "keep the road open"). The interim version **injects for everyone unconditionally**.
2. **1:1 first, multi-recipient abstraction.** The final topology = group chat + mail (1:1, multi-target, broadcast). Implement **1:1 first**, but let delivery flow as a "recipient list" (currently length 1) so that group and broadcast can be layered on the seam (on top of the group reserved in ADR-0086). It is a low-risk, long-lived seam, so we lay it down now (CLAUDE.md §0).
3. **Format choice = a comparison spike, decision axis acceptance > tokens.** Pick among a few candidate envelope formats by testing real send/receive. **First priority = acceptance and processing quality, second priority = token efficiency.** Priming is already in place, so we are not fighting injection but comparing readability on the happy path.
4. **Happy-path-first (restructure the ladder).** Set up the normal-acceptance baseline first, and handle abnormal cases such as saturation, injection and exactly-once after it — **if large, later; if small, implement in parallel**. This restructures the adversarial-first ladder of ADR-0088 from this viewpoint.
   - **Acceptance criterion:** the receiving agent notices the message naturally, without treating it as injection, and reflects or reacts to it in context (not an immediate "received" ack).

## 거부한 대안
- **Hardcoding the priming text in code** — cannot be swapped for the future injection system, and every content edit needs a rebuild. → external MD seam.
- **The agent reads the MD file by itself** — needs a separate action, and differs in kind from the future injection system (system-prompt injection), so it would have to be torn out later. → the daemon reads it and injects it into the system prompt.
- **A 1:1-only implementation (ignoring multi-recipient)** — the delivery path must be reworked when group and broadcast are introduced. → recipient-list abstraction.
- **Deciding the format by forgery resistance** — happy-path acceptance comes first. Forgery defense = escaping is deferred to Stage 4 (ADR-0088/0091).
- **Settling the pilot finding as "the feature is blocked"** — an over-conclusion that did not account for the confound (an injection-shaped probe + no priming). The receiving contract is the real gap.

## 근거
- **Reinterpretation of the pilot confound (user remark 2026-07-20):** orchestration works normally (the spawn task = a trusted channel), but the pilot threw a mid-stream third-party injection-shaped probe at an agent with no priming — isolating it is the normal reaction. A natural message + priming is unverified.
- **User design decision (2026-07-20):** topology (1:1 first, assume multi-recipient) · priming (external MD, inject for everyone, seam) · format choice (acceptance > tokens) · an approval gate on the bird's-eye view right before code.

## 영향 / 불변식
- **Hardcoding the priming content is forbidden** — always an external MD. The loader is a seam (replaceable by the future injection system).
- **Keep the delivery path as a recipient-list abstraction** — even 1:1 flows as a list (length 1). Narrowing it to a single recipient causes a regression for group.
- **No serious start on abnormal cases (saturation, injection) before the happy-path baseline passes** — do not dig into failure modes without a success mode.
- **Format decision axis = acceptance > tokens** (not forgery resistance).
- **If the measured priming acceptance is blocked by instructions applied to the account, use an account without instructions** — account instructions contaminate the acceptance decision (ADR-0091 account dependence stands).
- **Get the user's approval of the bird's-eye view before starting code implementation** — a user rule for this session.
