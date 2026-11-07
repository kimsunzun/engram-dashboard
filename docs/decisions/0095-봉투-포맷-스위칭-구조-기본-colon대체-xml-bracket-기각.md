# ADR-0095: Envelope format switching structure — colon by default, xml as the alternative (bracket rejected)

- 상태: 확정 (2026-07-21, basis: user decision — after 3 spike measurements + a sonnet stress-scale-up tie confirmation)
- 관련: ADR-0086(single wrap point — the placeholder was a clause saying "a follow-up spike will replace it", and this ADR carries that out) · ADR-0092(decision axis acceptance > tokens) · ADR-0093(decision 4 format spike) · `crates/engram-dashboard-daemon/src/control/ingress.rs::wrap_message` · step-log S17 (format spike rounds 1 and 2, routing trust, rich-metadata readability) · Amended by ADR-0096 (where the envelope format switch is stored and how it is exposed (decision 5))

## 맥락

In an agent-to-agent message, the sender submits it without form (tool parameters: recipient, body), and the daemon assembles the envelope at the single point `wrap_message` and drives it into the recipient's stdin as text (ADR-0086). The envelope is a **surface for the receiving model to read only** that no machine parses (delivery addressing is already done by the tool parameters). Which envelope shape is best was deferred by ADR-0093 decision 4 to a follow-up spike, and the measurement is now finished.

## 결정

1. **The envelope format is a switching structure, not a single fixed one** — swap formats by template selection inside `wrap_message` (the send and receive code paths unchanged).
2. **General default = colon** `{sender}: {body}` (e.g. `alice: auth 모듈 진행 상황 알려줄래?`).
3. **Alternative (switch target) = xml** `<message from="{sender}">{body}</message>` — consider switching when a weak model is deployed, when another backend (gemini/local, etc.) shows anomalies, or when options are extended (group/cc/thread).
4. The adoption condition, "a high-end model gives colon = xml even under stress (rich metadata)", was met by measurement: sonnet rich-colon 6/6 = rich-xml 6/6 (routing · sender identification · flags 0 · A-side metadata leak 1/6 each, symmetric).
5. Where the switch is stored and how it is exposed (global setting / per recipient / by presence of metadata) is carried over as an implementation fork — decided by the user in the implementation slice.

## 거부한 대안

- **Keep bracket `[message from {sender} id:{id}] {body}` (the existing placeholder)** — halfway by measurement: no token gain over colon, and no extension insurance over xml either (in haiku rich-metadata it collapsed to 0/2 along with colon — only xml survived 2/2).
- **Expose the uuid (msg_id) in the envelope** — not needed for acceptance (spike round 1: every candidate was accepted on the sender label alone) and the largest overhead (36B). msg_id itself stays for internal delivery dedup (removed only from the envelope).
- **An in-envelope behavior instruction (e.g. a "reply with send_message" reminder)** — refuted by the official Anthropic docs (an instruction inside a user message weighs less than the system prompt), and in measurement sonnet detected it as an injection and blocked it.
- **The sender (an LLM) generates the envelope directly** — spends output tokens (several times costlier than input) on boilerplate + generation errors such as broken tags. Keep the daemon's deterministic assembly (zero error · input tokens only).
- **A single fixed format (no switch)** — for unmeasured models such as gemini/local there is no guarantee that colon holds, so it cannot be finalized. Switching is the insurance.

## 근거

- **Acceptance (parsing)**: 0 failures in every format — across format spike rounds 1 and 2 (24 samples) + rich-metadata (18 samples), sender and content were all parsed correctly (sender misidentification 0 · security flags 0).
- **Tokens**: fixed overhead colon 2B < bracket 16B (+uuid 56B) < xml 27B (+attributes) — acceptance is tied, so on the decision axis (ADR-0092 acceptance > tokens) colon wins.
- **Extension insurance**: the haiku (weak model) on rich metadata gave colon · bracket 0/4 vs xml 2/2 — only a structured envelope keeps the tool-action signal (directional evidence, n=2~3 per cell). This is why xml stays as the switch target.
- **Industry**: mature frameworks structure the transport layer (no envelope needed), and where text injection is needed (the Claude Code harness) Anthropic uses XML tags — colon is a human chat convention (the most frequent in training data), which justifies it as the minimal form.
- **Limits (honestly)**: all measurements were on Korean conversation · n per cell is small — to be re-verified after the agent-to-agent switch to English (backlog). Haiku-class models are not going to be deployed anyway, so the default decision is unaffected.

## 영향 / 불변식

- **The single wrap point invariant (ADR-0086) holds** — switching is just template selection inside `wrap_message`; envelope assembly must not become two places.
- The `ENGRAM_WRAP_FORMAT` env **stays as a spike-only seam** — it is not an operational switch. The formal switch (settings-based) is a follow-up implementation slice.
- The code change that replaces the default template with colon + aligning the envelope description in the priming go together in the implementation slice (coder → review → QA gate).
- When a new model/backend is deployed, measure colon routing and acceptance with a smoke, and consider switching to xml on anomalies (the switching structure lowers that cost to the level of template selection).
