# ADR-0031: Review system — two-party adversarial review by opus + Codex (web consult discarded, stage-specific roles)

- 상태: 확정 (2026-06-22, basis: the methodology research in `docs/research/` + agreement in this session)
- 관련: CLAUDE.md 「구현 실행 규약」 · `.claude/skills/review/references/flow.md §2` (operating role table) · `docs/research/review-pipeline-design-draft.md` (detailed design) · `docs/research/review-methodology-research-2026-06-22.md` (basis)

## 맥락
Codex (GPT CLI, `mcp__codex__codex`) was introduced. Until then, cross-checking and review of major designs was done with a **web consult** (the same prompt to three models, GPT · Gemini · Claude-opus → blind judge → correctness-merge) and with fable as the first-choice LLD reviewer. Since Codex was now available, we re-examined "what is the right way to set up review".

## 결정
The web consult is discarded. Review of non-trivial code changes and major designs is unified as a **two-party adversarial review by opus + Codex**.
- **Structure (fixed):** every review pair is an **Advocate (defends and strengthens) vs Adversary (attacks, opposes)** — a devil's advocacy / dialectical pair. Do not invent one on the spot.
- **Specialization (fixed once per stage):** PRD = User/Tester · TRD = Designer/Architect-breaker · code = correctness/breaker · document cleanup = cut-advocate/load-bearing guardian. (Operating table = the review skill's `flow.md §2`, details = the design document)
- **Model mapping:** roles that need context (ADRs, invariants) = opus (doc-aware), fresh blind roles = Codex.
- **Verdict:** PASS/FIX/BLOCK (no scoring); the order and labels of aggregation do not matter. **Disagreement → the main session must not decide arbitrarily; it goes to the user.**
- **effort:** main session xhigh / coders and reviewers high (Codex medium by default, high only for changes where concurrency or lifetime is critical).
- sonnet = lower-tier coder only (not a judge), haiku = excluded from the gate.

## 거부한 대안
- **Keep the web three-family consult (GPT + Gemini + Claude blind judge)** — input diversity (three families) is larger, but **the judging node stays Claude to the end** (both the blind judge and the synthesis are Claude opus), so it *structurally cannot catch* the very bias we care about (the main session = Claude judges a Claude-style answer to be correct). Giving a non-Claude judge (Codex) teeth is more effective on that axis. As a side point, the operating cost is also large (web-runner panel · browser login · ~12 minutes of polling · fragile orch). What discarding loses is only *one family of input diversity, Gemini* — the consult was not catching the bias anyway.
- **A single reviewer (fable/opus only)** — no cross-family → own blind spots. No adversarial pair, no diversity benefit.
- **Generic Advocate/Adversary only (no stage specialization)** — stage-specific roles catch defects more widely. "Fixed once per stage ≠ invented on the spot each time", so fatigue does not grow; generic is only the fallback for an unspecified artifact.
- **Anonymization as the main bias safeguard** — we do not do it. A coarse rubric (PASS/FIX/BLOCK) + checklists come first; anonymization is secondary.

## 근거
All methodology and empirical basis lives in `docs/research/review-methodology-research-2026-06-22.md` (design details: `review-pipeline-design-draft.md`). This ADR holds only the decision and the rejected alternatives — it does not duplicate the basis (single source).

## 영향 / 불변식
- Every non-trivial code change goes through this two-party review as a gate (no skipping). Coder and reviewer are separate (the main session orchestrates).
- **On disagreement the user has the final say** — if the main session (a Claude family) decides arbitrarily, the anti-bias design breaks.
- When a new stage type is added, its specialized roles are fixed *once* (added to the review skill's `flow.md §2` table), not invented on the spot.
- **Unverified (experimental option):** the code-stage "Codex blind / opus doc-aware" asymmetry is a hypothesis with no empirical basis — it is not mandatory until its effect is measured.
