# Debugging and defect-fixing conventions (engram-dashboard)

**Status:** working convention (settled doctrine — an edit-in-place canon, not an ADR). The *why* + the rejected alternatives = ADR-0038. Evidence case = `docs/research/toctou-concurrency-test-verification-research-2026-06-28.md`. When you fix a load-bearing defect, this document is the single source.

> In one line: **for a non-trivial technical defect, do not tune it alone with magic numbers; first research how OSS solves this class of problem, then fix it at the root.** The *defect-fixing* sibling of the rule for new *features*, OSS first (`/engram-research` design research).

**Non-trivial technical defect** = an area such as concurrency/race · timing/flaky · perf · memory · architecture, where *mature OSS has a standard answer and the naive fix is often wrong*.

## Trigger (either one is enough)

This is a self-awareness guard (it cannot be enforced by a hook) — it is caught by *an action you can notice in the moment*, not by *category*:

1. **Suppressing the symptom** — you try to "get past" a failure/flaky test/slowness by adjusting a magic number, a timing constant, a threshold or a retry count (erasing only the symptom without understanding the root cause).
2. **Repeated guessing** — a guess-based fix for a non-trivial technical defect has already failed once, and you still try to keep going solo.

**Excluded:** trivial bugs with an obvious cause, typos, deterministic fixes / project-specific business logic with no OSS precedent.

## When it triggers

Stop, research how OSS (mature libraries, primary documentation) solves this class of problem, and **turn the root-cause solutions into options** (if it is big, present it to the user · write an ADR). Carry out the research under the **existing research-delegation rule** (global-rules "위임 우선", formerly "컨텍스트 위생" — subagent / `/engram-research`). Even if the result is "the current approach is right" — it is *confirmed*, so it differs from a solo guess.

---
Discovery = one line under 「설계」 in the code-conventions binding (`.claude/skill-bindings/code-conventions.md`) (it is carried in coder and reviewer briefs) → this document. **Firing** = the point where `/engram-qa` catches a flaky, timing or perf failure (qa binding 「Rust 테스트 판정」). The canonical anti-pattern case (S14 flaky `5ms→50ms` solo tuning ↔ `/research` root-cause fix) is in ADR-0038 + the research report above.
