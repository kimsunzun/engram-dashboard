# Code commenting practice research — comments in the agent era (detailed record)

> **Status:** **Adopted: option B (ADR-0032, canon `docs/reference/commenting-conventions.md`). Code is applied gradually (boy-scout).** Stages 1, 1.5 and 2 complete.
> **Phase:** PRD/consult (survey → options → user decision) adoption complete. Code is gradual boy-scout.
> **Method:** Claude (Sonnet) 4 + Codex 4 **symmetric independent surveys per track** (Stage 1) → Opus adversarial verification 2 (Stage 1.5) → Opus synthesis and cross-check (Stage 2).
> **Date:** 2026-06-23 · **plan:** `~/.claude/plans/goofy-conjuring-walrus.md`
> **Confidence notation:** (certain) primary source · multiple sources converge · (likely) circumstantial · single source · (uncertain) unconfirmed/presumed.

---

## 0. Why (Context)

**User question/hypothesis:** With agent-style development the user no longer reads all the code directly, so the hypothesis is that "comments must be written in more detail so the user can look at just that part and **understand it at a glance**". We check whether this holds, and how industry commenting practice changes in the agent era.

**Diagnosis of engram's current state (prior Explore):**
- The concurrency · kill · race core (`output_core.rs`/`reaper.rs`/`session.rs`) is good, with a comment density of 22~28%. With 36 `// ADR-NNNN` anchors, the file headers (`//!`), and Korean "why" comments, compliance with the CLAUDE.md convention is ~85%.
- Weaknesses: the ADR anchors are partial (relative to the load-bearing points), and the internal branches of `session_tracker.rs` (resolve/poll/PID shim) and some large tests leave a first-time reader lost.
- There is no comment-only ADR. The principle lives only in the CLAUDE.md "컨벤션" section (why-only · no comments on the self-evident · plant load-bearing intent · the `ENGRAM_DATA_DIR` accidental deletion case).

The thread of this research's conclusion → organize **options + trade-offs** on how to refine engram's comment convention, for the user's decision (no arbitrary adoption).

---

## 1. Stage 1 — empirical survey (Claude × Codex cross-check)

> Survey design: tracks 1 and 2 = fact split (one per family), track 3 = skeptic vs. advocate confrontation (two per family). The two families surveyed independently so they could not see each other → cross-checked in §1.D.

### A. Track 1 — taxonomy of traditional commenting practice

The two families **converged almost completely**. Core taxonomy:

- **The why-not-what principle (certain)** — code shows *how* and comments state *why* (intent · background · constraints). Sources: Atwood "Code Tells You How, Comments Tell You Why"; Google C++ Style Guide (prefers self-documenting names + requires tricky/non-obvious and "why chose this implementation").
- **The self-documenting code debate (certain)** — "code is the documentation" but ≠ "code alone is the documentation". Martin Fowler "CodeAsDocumentation": code can be the primary documentation, but supplementary documentation is needed. Extreme self-documentation backfires with over-abstraction and broken flow.
- **The "comments are an excuse for failure" line (certain)** — *Clean Code* (Martin) "Don't comment bad code—rewrite it". But this is **a warning aimed at excessive `what` comments**, and does not mean to erase even API contracts · ADRs · invariants · concurrency constraints · historical reasons (dangerous if misread).
- **comment rot / staleness (certain)** — the compiler does not force comments to be updated, so they diverge from the code. The response is not minimizing comments but placing them near the code + testable examples (doctest) + update pressure from lint and review. Source: arxiv 2403.00251 (CoCC).
- **literate programming (certain)** — Knuth (1984): a program is a communication to a human reader. weave/tangle. Modern successors: Jupyter, Quarto, doctest. Revived in the LLM era as NL Outline (§B).
- **doc-comment systems (certain)** — rustdoc (`///`, `//!`, first-line summary · Markdown · doctest), JSDoc (`@param`/`@returns`, linked to TS `checkJs`), Python docstring (PEP 257, runtime `__doc__`), Javadoc (IDE warnings · effectively enforced), Doxygen (C/C++ multi-language). Common trend: reduce type descriptions that duplicate the signature and focus on contract · examples · panics/errors/safety.
- **ADR (certain)** — Nygard (2011): Title/Status/Context/Decision/Consequences. The core is **the rejected alternatives + the reasons**. Version-controlled in the repo next to the code. Turning even trivial decisions into ADRs is noise.
- **Diátaxis (certain)** — Procida: separates 4 types: tutorial/how-to/reference/explanation. It is not a code-comment taxonomy but the information architecture of user docs, yet its diagnosis that one document mixing several purposes degrades quality also applies to comments.
- **Comment kind classification (certain)** — by purpose (explanation/license/metadata/TODO·FIXME/debug/directive/documentation); position-based rules are easy to lint but "good content" is hard to judge automatically. TODO/FIXME is abandoned unless linked to an issue tracker.

**Codex's synthesis framing (likely, useful):** good practice is not "reduce comments" but **"narrow the responsibility of comments"** — `what` is carried by names/types/structure/tests, and `why·contract·edge case·decision rationale·user task` are separated into 4 layers: **local comment → doc-comment → ADR → Diátaxis docs**. (This layer model is the skeleton of the §3.1 recommendation)

### B. Track 2 — changes in the agent/LLM era (2024~2026)

The two families converge strongly. The center of gravity **moves from "code explanation" to "intent/context/guardrail shared by humans + AI agents"**.

- **(i) The environment where humans review AI code:**
  - ~42% of committed code is AI-assisted (Stack Overflow 2025 Survey), yet trust in AI went from 43% (2024) to 33% (2025). (certain)
  - **verification debt** emerges: 96% of developers do not fully trust the correctness of AI code, and a considerable share say reviewing AI code takes longer than a colleague's code (Sonar). (likely)
  - The review burden shifts from "writing" to "verifying · understanding". Salesforce Engineering (2026): with production code up 30% from AI, they redesigned review as "intent reconstruction". **The diff alone cannot tell the intent.** (certain)
  - Addy Osmani: the PR minimum standard is "Intent in 1-2 sentences" — if you cannot explain it, you cannot commit. (likely)
  - **NL Outlines (certain, accepted at FSE 2025)** — arxiv 2408.04820: a **separate layer**, distinct from docstrings (contract) and inline comments (scattered rationale) = the implementation strategy as function/block-level NL summaries. About half the size of the code, "very helpful" 63%, and two-way automatic synchronization overcomes staleness.
- **(ii) Docs/comments that AI consumes:**
  - **The agent instruction file ecosystem (certain)** — `AGENTS.md` (60k+ repos, 30+ agents), `CLAUDE.md`, `.cursor/rules`, `.kiro/steering/`. Princeton measurement: with an AGENTS.md, runtime down 28.6% · tokens down 16.6%.
  - **Warning (certain):** an LLM auto-generated AGENTS.md backfires by duplicating the README (success rate down 2% · cost up 23%). **A 50-line pinpoint > a 1000-line catch-all**. Most of what exceeds 500 lines is ignored.
  - **llms.txt (certain adoption / uncertain effect)** — proposed 2024-09, 840k+ sites (Stripe · Cloudflare · Anthropic). But there is no official confirmation that the major LLMs use it as a ranking signal.
  - **spec-driven development (likely)** — "the spec is the prompt". GitHub Spec Kit, AWS Kiro (Vibe↔Spec). Karpathy also pivoted: the vibe coding era is over, toward "agentic engineering". But "spec vs. code — which is the source of truth" has no agreement.
  - **Docs = soft context, not enforcement (certain)** — Claude docs: `CLAUDE.md` is context, not enforced configuration. Actual blocking needs a **hard guardrail** such as hooks/CI/CODEOWNERS. (directly tied to engram's §5 LLM-first control design)
  - **Inline `// ADR-NNNN` anchors (likely)** — spreading as an agent guardrail. But arxiv 2602.04445: there is a gap — ADRs capture only architecture decisions and miss *implementation-level decisions* (far more numerous and the most easily lost).

### C. Track 3 — skeptic vs. advocate (confrontation)

| | Skeptic | Pro (advocate) |
|---|---|---|
| Core proposition | Only code is the executed truth. Comments are unverified and rot | Code preserves only *how*; only comments preserve *why · rationale · invariant* |
| Strongest authority | Clean Code (Martin), Kernighan&Pike | Ousterhout *A Philosophy of Software Design* |
| Empirical weapons | eye-tracking (comments have no effect on comprehension) · large-scale comment rot studies · link decay | comprehension cost 50~70% · preventing deletion of load-bearing code · ABF comments improve LLM bug fixing |
| Alternative | Good naming · small functions · the type system · tests as documentation | comments-first (a design tool) · designNotes · NL outline |
| The agent era | AI-generated comments are redundant · hallucination · verification debt → use tests/types/static analysis instead | The less a human wrote the code, the more intent comments are the only context channel · the review baseline |

Both positions **agree that "`what` comments are bad"** — the real dispute is "should even *why* be cut (skeptic) vs. is *why* essential and should grow (advocate)". The key empirical claims on this fault line are adversarially verified in §2.

### D. Claude ↔ Codex cross-check table (convergence/divergence)

| Track | Convergence (both families agree) | Divergence/complement |
|---|---|---|
| 1 Traditional taxonomy | The 8 taxonomy items · doc-comment systems · ADR · Diátaxis all match | Only Codex states the "narrow the responsibility" 4-layer model (reinforcement, not divergence) |
| 2 The agent era | The move to intent · AGENTS.md/CLAUDE.md/llms.txt · spec-driven · NL Outline match | Claude = stresses NL Outline · Princeton figures / Codex = stresses "docs = soft context, a hard guardrail is needed" |
| 3a Skeptic | rot · self-documenting · Clean Code · types>comments · tests as docs match | Only Codex adds link decay (arxiv 1901.07440) · the SLR on "difficulty of evaluating comment quality" |
| 3b Advocate | Ousterhout · comprehension cost · load-bearing · literate · NL outline match | Only Codex adds ABF (arxiv 2601.23059) "comments make LLM bug fixing ~3x" |

**Conclusion: no substantive conflict between the two families (convergence).** All the differences are *complements* that one side covered at greater length. → The cross-check raises the confidence in the landscape (certain). However, the two empirical claims Codex presented alone (ABF as the opposite side of eye-tracking, link decay) and the skeptics' strongest weapon are single-source, so they need adversarial verification in §2.

---

## 2. Stage 1.5 — adversarial verification (2 Opus, web primary sources re-checked)

### Skeptic empirical claims — **weaker than the summary**

- **Claim 1 (eye-tracking, "comments have no effect on comprehension") — verdict: partly true / the summary is exaggerated.**
  - Real: Abdelsalam et al., *Empirical Software Engineering* 2025, DOI 10.1007/s10664-025-10721-2.
  - Actual: **n=20 students**, 12 simple Java snippets of ~20 lines, comments present or absent as a binary (**no** what/why distinction). Only **1 of 12** snippets showed a significant effect of comments on correctness. But for time (efficiency), 6/12 were significant (in some, comments *hindered*).
  - The author's own conclusion: *"the efficacy of comments is highly contextual"* — in some cases they help, in others they hinder. → This is **not** "comments are useless". The "readability vs comprehension" dichotomy itself does not match what the paper measured (the paper measured visual attention · reading linearity). The authors explicitly caution about generalizing from a student sample and simple code.
  - The proposition skeptics can honestly cite: "*under these narrow conditions* the presence of comments did not significantly raise the correct-answer rate (11/12 not significant)" — and no further. (certain)
- **Claim 2 (comment rot proven at large scale) — verdict: fact confirmed, but the conclusion drawn is a leap.**
  - Sources and figures are accurate: Wen et al. 2019 (ICPC, 1,500 Java projects, most do not co-evolve), CoCC arxiv 2403.00251 (detector), 9.6M Links arxiv 1901.07440 (~10% dead links). The *existence* of rot is solid (certain).
  - **The leap:** "rot exists → do not write detailed comments" exceeds the data. The same data equally supports "update comments together with the code / sync them automatically" (the actual direction of follow-up research = detection · auto-repair). Rot is a function of *missed updates*, not of comment *length*.
- **Synthesis:** The skeptics are solid on the data up to "comments have costs and failure modes", but at the last step of "so cut them / they do not help understanding", both claims exceed the evidence.

### Advocate empirical claim + the user's hypothesis — **direction supported, shape corrected**

- **Claim 1 (comments make LLM bug fixing ~3x) — verdict: conditionally supported.**
  - Real: Vitale et al., arxiv 2601.23059. The "3x" is for a fine-tuned **CodeT5+ 220M** on 2011–2017 Java, 3.26%→9.80%. **GPT-4.1 goes 6.11%→8.63% = ~1.4x** (a small gain on large models). Oracle leakage (comments generated from the *fixed* code) — the authors state that it is not applicable in practice.
  - **The solid qualitative conclusion:** of the 5 comment categories, **"How (implementation intent/mechanism)" is strongest and "What" is weakest** (common to both models). (certain)
- **Claim 2 (the user's hypothesis) — verdict: conditionally supported. The naive form is refuted, the refined form is supported.**
  - (a) The industry consensus = **not** "more/more detailed" but "narrow the responsibility" (toward why/intent/invariant). The backfire of LLM-generated comments (redundant · hallucination) is demonstrated (arxiv 2605.13280, 2406.14836) → "detailing across the board" is self-destructive. → **The naive hypothesis is refuted.**
  - (b) What helps "understand at a glance" is not verbose inline but a **function/file/module overview · NL outline** (a separate layer) + **automatic updating (living doc)** to stop rot (arxiv 2408.04820).
  - (c) The increased comprehension burden is solid (verification debt converges across many sources). But METR (arxiv 2507.09089, perceived +20% / measured -19%) is limited by n=16 and early-2025 models — not direct evidence of an "18-month wall" (likely). **Decisive:** Osmani (paraphrased) — comments and specs alone cannot solve comprehension debt … in the end someone has to review. → The hypothesis's implicit premise ("if only the comments are detailed, no need to read") is **refuted**.
- **The refined proposition (the form the evidence supports):** *"In agent development, raise reliability by **narrowing** inline comments to why/invariant/load-bearing, and separate the 'understand at a glance' into an automatically updated overview/outline **separate layer** — but this does **not replace** human review."* — which happens to match engram CLAUDE.md's existing convention ("no comments on the self-evident, go deep only on load-bearing").

---

## 3. Stage 2 — synthesis

### 3.1 Practice taxonomy — the 4-layer model of "narrowing the responsibility"

| Layer | Form | What to put in | What to avoid |
|---|---|---|---|
| Local inside code | `//`, `/* */` | Non-obvious reasons · invariants · sentinels · concurrency/lifetime · workaround rationale | A line-by-line paraphrase of code · dead code · stale TODOs |
| API doc-comment | rustdoc `///`/`//!`, JSDoc, docstring | Usage · contract · params/return meaning · errors/panics/safety · examples · deprecation | Type descriptions that duplicate the signature · template filler |
| Design decision | ADR (+ code `// ADR-NNNN` anchors) | context · decision · **rejected alternatives** · consequences · status · supersede | Meeting minutes · trivial implementation choices |
| User/agent docs | The 4 Diátaxis types · AGENTS.md/CLAUDE.md | Separating learning/task/lookup/understanding · agent invariants · prohibitions · verification procedures | Mixing intro + procedure + API + philosophy in one document |

**+ A new layer for the agent era (proposal):** a file/module **overview (understand at a glance)** — summarize the role · responsibilities · key invariants · entry points in the header (a strengthened version of engram's current `//!` headers), updated together with code changes where possible.

### 3.2 The flow of change in the agent era (one-line summary)

Comments and docs evolve from "supplementary explanation of code" into **"intent infrastructure"** (human = grok AI code · the review baseline / AI = injected repo rules · intent · prohibitions · verification). But docs are soft context, so they need a hard guardrail (tests · CI · hooks) alongside. (certain)

### 3.3 Verdict on the user's hypothesis

**Conditionally supported.** The direction (since humans read less, invest more in comments) is right. The shape is not "more detailed across the board" but **(1) narrow inline comments to why/intent/invariant/load-bearing, (2) a separate overview/outline layer carries the 'understand at a glance', (3) prevent rot with automatic updating, (4) do not assume comments replace human review**.

### 3.4 Recommendation for engram — options (user decision)

engram's current convention already matches the evidence-supported shape at ~85% (not a coincidence but a good stance). The gaps are ① uneven quality of the file overview "understand at a glance" ② partial ADR anchors ③ too few comments on internal branches such as session_tracker.rs.

- **Option A (minimal):** leave the convention text as is, and only gradually reinforce the files diagnosed as weak (ADR anchors · internal why comments). No ADR/CLAUDE.md change.
  - Trade-off: minimal cost · minimal rot risk / the standard stays implicit, so the next session may miss "overview quality".
- **Option B (middle, recommended):** write a **2-layer comment convention** into the CLAUDE.md "컨벤션" section — (1) inline = why/intent/invariant/load-bearing (existing) (2) **new: load-bearing files must have an "understand at a glance" overview header** (role · responsibilities · invariants · entry points) + gradually expand `// ADR-NNNN` anchors. If it is a major decision, ADR-00xx.
  - Trade-off: meets the user's "understand at a glance" need directly in the evidence-supported shape / headers can also rot → a discipline of "update together with code changes" is needed.
- **Option C (strong):** B + aim at NL-outline-style structured overviews · automatic updating (living doc) tooling (e.g., a notice to update the header when a change is detected).
  - Trade-off: stops rot structurally · the most agent-friendly / high tooling cost, with a risk of over-engineering at this stage (a high-cost-uncertain area, not low-risk-long-term).

Recommendation: **B**. It meets the user's "understand at a glance" in the exact shape the evidence supports (separating the overview layer + narrowing inline), while avoiding C's tooling cost and rot risk. If adoption is a major decision, record it as an ADR.

---

## 4. Application example (proposal — not applied to code)

> This is the research stage, so nothing was applied to code. Below is an **example** showing what option B's "understand at a glance overview header" looks like in practice (applied on adoption).

**Important finding:** **19 of the 22 core files already have a `//!` header** → this codebase already follows option B ~faithfully. Even `session_tracker.rs`, which the prior diagnosis saw as "weak", is actually exemplary (header · branch why · PID shim/degraded/lock order planted), so it needs no touching. The gap is only the 3 files without headers (`logging/mod.rs` · `types.rs` · `agent/mod.rs`) + the partial ADR anchors.

**Example target:** `crates/engram-dashboard-core/src/logging/mod.rs` — load-bearing (security `mask_secrets`, tracking T-1/D-6), yet line 1 starts with `use ...`, giving a first-time reader zero orientation. The per-item `///` is good, but only the module overview (`//!`) is missing.

**Before:** The first line = `use std::sync::OnceLock;`. "What responsibility this file carries", and especially the non-obvious invariant that **"masking is not automatic but the caller's responsibility"**, is not visible at a glance → the next session risks adding PTY text to production logs and forgetting `mask_secrets` (= the same class as the CLAUDE.md `ENGRAM_DATA_DIR` accidental deletion incident).

**After (if such a `//!` header is attached):**

```rust
//! logging — tracing-subscriber 전역 설정 + 로그 비밀값 마스킹.
//!
//! ## 두 책임
//! - 로그 초기화·레벨 제어: init_logging(부팅 1회, 멱등) → RELOAD_HANDLE → set_log_level.
//! - 비밀값 마스킹: mask_secrets가 API 키·Bearer 토큰을 ***로 치환(T-1).
//!
//! ## load-bearing (시그니처만 봐선 안 보이는 의미)
//! - 마스킹은 *호출자* 책임 — 자동 적용이 아니다. 이 모듈은 mask_secrets를 제공만 한다.
//!   (tracking T-1/D-6 — production PTY 로그 추가 시 필수)
//! - 마스킹은 best-effort. AWS Secret Key·generic api_key= 는 못 잡는다. "통과 = 비밀 0" 아님.
//! - 전역 상태(OnceLock)라 init·regex 컴파일은 정확히 1회. 중복 init/set은 no-op.
```

**Improvement:** On line 1 a first-time reader immediately grasps ① the two responsibilities (log control / secret masking) ② the load-bearing invariants (masking = the caller's responsibility · best-effort · global, once). It is not redundant comment (noise) but planting meaning that the signature does not show — consistent with why-not-what · load-bearing. This is the concrete shape of option B. (Actual application · QA after the adoption decision.)

---

## 5. Gaps (what this survey could not fill)

- **Feasibility of NL-outline automatic updating for engram code** — whether living doc tooling is realistic in a Rust/Tauri environment is not investigated (a separate prior-art survey is needed if option C is adopted).
- **The optimal length · format of the "understand at a glance" overview** — the 50-line pinpoint principle is for AGENTS.md, and the appropriate length of a code file header is not measured.
- **Korean-language practice** — the survey targets English-language OSS · literature. Studies of the effect of Korean-language comments themselves are unconfirmed.

## 6. Limits (overall)

- The skeptics' key empirical evidence (eye-tracking) is a single venue · small sample. The advocates' key empirical evidence (ABF 3x) has limited practical extrapolation due to oracle leakage — both are cited narrowed in §2.
- Agent-era trends (2024~2026) shift quickly — spec-vs-code source of truth, the real effect of llms.txt, and the optimal granularity of AGENTS.md are all without agreement (uncertain).
- Both families are web-survey based — the comparison against full primary papers is limited to the 4 claims in §2.

## 7. Key sources

- Atwood "Code Tells You How…": https://blog.codinghorror.com/code-tells-you-how-comments-tell-you-why/
- Fowler "CodeAsDocumentation": https://martinfowler.com/bliki/CodeAsDocumentation.html
- Ousterhout *A Philosophy of Software Design*: https://web.stanford.edu/~ouster/cgi-bin/book.php
- Knuth Literate Programming: https://www-cs-faculty.stanford.edu/~knuth/lp.html
- ADR (Nygard / adr.github.io): https://adr.github.io/ , https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions
- Diátaxis: https://diataxis.fr/
- rustdoc: https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html
- NL Outlines (FSE 2025): https://arxiv.org/abs/2408.04820
- AGENTS.md: https://agents.md/ · Codex AGENTS.md: https://developers.openai.com/codex/guides/agents-md
- llms.txt: https://llmstxt.org/
- Spec-driven: https://kiro.dev/docs/specs/ , https://arxiv.org/abs/2602.00180
- eye-tracking (verified): https://www.se.cs.uni-saarland.de/publications/docs/APB+25.pdf (DOI 10.1007/s10664-025-10721-2)
- comment rot: Wen 2019 https://dl.acm.org/doi/10.1109/ICPC.2019.00019 · CoCC https://arxiv.org/abs/2403.00251 · link decay https://arxiv.org/abs/1901.07440
- ABF comment effect (verified): https://arxiv.org/abs/2601.23059
- comprehension debt: https://addyosmani.com/blog/comprehension-debt/ · METR https://arxiv.org/abs/2507.09089 · Sonar verification gap
- LLM comment backfire: https://arxiv.org/html/2605.13280v1 , https://arxiv.org/abs/2406.14836
