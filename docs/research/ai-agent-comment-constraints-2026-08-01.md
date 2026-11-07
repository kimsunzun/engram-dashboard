# Over-commenting by AI agents — causes · practical constraints · automatic checks (research medium)

**Status:** done · main grounding + cross-family adversarial review (Codex, effort high, web_search) applied · 2026-08-01
**Purpose:** the evidence base for revising the comment standard. The revision result lives in `~/.claude/skills/clean-comment/references/` (source of truth).
**Confidence:** certain (independent cross-confirmation) / likely (grounding-supported · single source) / uncertain (on hold)
**Adversarial review decision: BLOCK → all 12 items applied, and what follows is the corrected version.** The draft's three central negative claims were wrong.

---

## ★Conclusion first — what can and cannot be used as evidence★

**Not to be used:** "AI writes more comments than humans" is **not established wisdom.**
- Controlled comparison studies **exist** (the draft asserted "there are none" — an error), and their results point the opposite way:
  AICodeDetect compared comment density over about 24,000 Java/Python/OCaml samples and reports that **human Python is
  higher and AI samples are consistently lower**. [PDF](https://svijayakumar2.github.io/aicodegen.pdf) ·
  [Springer](https://link.springer.com/chapter/10.1007/978-3-032-20362-5_10) — *main grounding: extracting the PDF
  body failed so we could not check directly → kept as **uncertain**. But the draft's "none exist" claim is discarded.*
- The appendix of arXiv 2506.06069 also directly compares the comment-to-code ratio distribution of human APPS solutions vs Claude 3 Haiku/CodeLlama/GPT-3.5.
  [arXiv](https://arxiv.org/abs/2506.06069) — likely.
- The circulating "human 18.42% vs AI 100%" statistic → **discarded.** The original (ResearchGate 388091783) is inaccessible with a 403,
  and a search on the same topic yields the opposite statement (paraphrased: comment documentation is more comprehensive in human code).
- arXiv 2508.21634 (large-scale human vs AI) — checked directly: **it does not measure comments at all** (only defects ·
  vulnerabilities · complexity). It cannot be used as evidence. — certain.
- **The direction differs by model and prompt, and these studies generate a benchmark snippet once, so the setting differs from
  "an agent that edits a real codebase over many turns with no instructions".**

**What can be used (a narrowed proposition):** **"Claude-family coding agents over-produce redundant comments when uninstructed"**
- `anthropics/claude-code#65961` — "[MODEL] Claude verbose code comments by default — ignores
  instructions to stop". **open · no assignee · no maintainer response** (checked directly 2026-08-01, rechecked by Codex).
  Body: *"adds far too many code comments by default … mostly redundant, restating what the adjacent
  code already makes obvious"* / *"A clear, mandatory rule in `CLAUDE.md` does not reliably suppress
  it."* / *"Reinforcing the rule via the memory system does not stop it either."* — certain (quotes verified).
  **★Limit (found by the adversarial review)★:** this is an **anecdotal report from one user in one environment (Opus 4.8)**. There is no multi-model experiment ·
  sample · control group · maintainer confirmation. **Do not generalize it into a behavioral property of all agents.**
- Duplicate issue `#61305` — the reporter says it failed even with a stronger CLAUDE.md wording, "Default to writing zero comments…".
  **★Limit★:** this shows a *failure to follow instructions*, not *a weak rule content*. "A more specific
  prohibition list raises compliance" is an **unverified hypothesis** (found by the adversarial review — a logic gap in the draft).
- **Each project's own measurement** — this repo's sample: interface-file comments −58.5%, files already swept a further −20%,
  while byte-layout files −6.9% (a contrast showing that the standard discriminates).

---

## 1. Causes — only one has evidence

- **[likely] RLHF verbosity bias** — arXiv 2310.10076 *Verbosity Bias in Preference Labeling by
  LLMs*: quantitatively shows that GPT-4 prefers longer answers than humans do. **But it is a study of general response length
  and does not target code comments** — the transfer is by analogy. [arXiv](https://arxiv.org/abs/2310.10076)
- **[uncertain — downgraded from the draft]** arXiv 2402.13013 *Code Needs Comments*: comment-augmented **training data**
  raises HumanEval/MBPP performance. **Found by the adversarial review:** this paper does not measure whether the resulting model
  *emits more comments when generating*. The causation training-data density → output verbosity does not hold. [arXiv](https://arxiv.org/abs/2402.13013)
- **[abstain]** training-data skew toward tutorials · chain-of-thought leakage · uncertainty hedging · literal-mindedness about instructions —
  no verifying studies. CoT leakage is one practitioner observation (a #65961 comment: comments refer to the chat history and implementation "phase").

## 2. Practical rule sets — "why not what" is practically the standard

- **[certain] Convergence:** GitHub's official `github/awesome-copilot`
  [`self-explanatory-code-commenting.instructions.md`](https://github.com/github/awesome-copilot/blob/main/instructions/self-explanatory-code-commenting.instructions.md)
  — *"Write code that speaks for itself. Comment only when necessary to explain WHY, not WHAT."*
  **6 prohibited kinds stated explicitly: Obvious / Redundant / Outdated / Dead Code / Changelog / Divider.**
  The Changelog item = *"Skip historical notes like 'Modified by John on 2023-01-15'"* (rechecked by Codex).
  Allowed exceptions: complex business logic · the reason for an algorithm choice · regexes · API constraints · public API docs · the rationale for settings.
- **[likely]** A stronger extreme is also used — Boris Cherny (original author of Claude Code) *"Avoid code comments
  unless your are explicitly asked to add comments"* (tweet — WebFetch 402, only the snippet checked → **verbatim
  unverified**). SamHatoum CLAUDE.md *"Do not add comments."*
- **[corrected]** The draft's "at least 4 independent sources converge — certain" was an **over-claim** (adversarial review): only 3 were actually presented, and
  one was "gist-type" so it was not pinned down as a source → downgraded to **likely**.
- **[corrected]** The draft's "vendor official docs have no comment rule — certain" → **scope limited:** Anthropic
  `code.claude.com/docs/en/best-practices` and the AGENTS.md of `openai/codex` — **we checked these two and found none**.
  A universal negative over all vendors does not hold, and the GitHub (`awesome-copilot`) case in this same report is a counter-example.
- **2 hallucinations in collection caught by ourselves:** ① a search summary saying "Anthropic's official docs say 'do not explain obvious code'" →
  a full-text check of the original found **no such wording** ② a rule saying "move design decisions into the PR description" →
  a direct check of 2 candidate sources found **it does not exist**. We adopted neither.

## 3. Automatic checks — there is no mature "low-value comment" gate, but **a narrow gate that works does exist**

**[certain — draft corrected] A usable tool exists:**
- **`comment-checker`** — **a Claude Code `PostToolUse` hook.** Go + **tree-sitter AST parsing** (not heuristics or an
  LLM), checks on every `Write`/`Edit`/`MultiEdit`, and if a comment is outside the allowed patterns (linter directives · shebang · BDD markers) it rejects with **exit 2**
  → Claude fixes it itself on the next turn. 30+ languages. Install via npm/brew/go.
  [pkg.go.dev](https://pkg.go.dev/github.com/code-yeongyu/go-claude-code-comment-checker)
  **It cannot be used as is for our purpose** — its allow list is blunt and also cuts load-bearing WHY comments.
- **Dart Sentinel** — a `redundant_comment` rule + IDE diagnostics + CI JSON/ratchet. [pub.dev](https://pub.dev/packages/dart_sentinel)
- **CRAIC** — scores the **semantic redundancy** of method comments (not whether one exists). [arXiv](https://arxiv.org/abs/1806.04616)

**[certain] But there is no general-purpose, verified semantic gate:**
- Coverage tools that only count existence (`interrogate` · `docstr-coverage`) also pass `def f(): """f"""`.
- Accuracy of low-value-comment classifiers falls short — arXiv 2504.18956: Random Forest **69%** / GPT-4 34% → **55%** after augmentation.
- Code-comment inconsistency (stale) detection has F1 88~90 (C4RLLaMA ICSE'25 · CCISolver 89.54), but **all are research prototypes**,
  and no CI deployment case was found.
- **[corrected]** The reason Checkstyle excluded PMD `CommentSize` is **not "too much false positives"**
  (found by the adversarial review — that wording belongs to another rule, `DataflowAnomalyAnalysis`). The actual reason = **Checkstyle
  writes class comments as the xdoc source, so the comments are deliberately large.** *But the lesson itself, "cutting by line count catches legitimately large
  comments", is in fact better supported by this actual reason.*
- The Rust Clippy lint for commented-out code has been unresolved since issue #1348 in 2016 (maintainer, paraphrased: to tell whether it is code you
  effectively have to parse) — **main verification failed, and Codex could not confirm it directly either → uncertain.**
- The criticism of comment-to-code ratio as a vanity metric is widespread, and SonarQube also does not
  commonly use it as a default quality gate condition — likely.
- The pattern in which an LLM review bot's accumulating false positives → developers ignore/suppress the alerts → workflow trust collapses is
  described in common across several industry sources — likely.

---

## 4. Conclusions this evidence supports (as applied in the revision)

1. **Narrow the justification.** Do not write "AI writes a lot of comments" in the standard document. All we can use is
   ① an **unresolved · anecdotal report** about Claude-family uninstructed default behavior ② **our own measurement**.
2. **"why not what" alone does not differentiate** — it is already the standard, and the people who used it also report failures.
   The differentiation lies in the **specificity of the prohibited kinds**. **But that hypothesis is unverified** (found by the adversarial review).
3. **Machine enforcement only for "what needs no judgment".** Position · volume (whether a private function has a doc · the doc>code ratio per definition ·
   a `파일:라인` (file:line) inside a comment) can be decided objectively with tree-sitter. **If you make a machine do semantic decisions it dies** —
   Clippy unresolved for 10 years · the 69% classifier · trust collapse from accumulating false positives are the evidence.
4. **Keep the after-the-fact adversarial review.** This project's record: FIXes found in every round (7 and 6) — but
   **the claim that "it was a class that mechanical proof could not catch" does not hold without a comparison protocol** (found by the adversarial review).
   The honest wording = *the review found defects while all the mechanical gates we ran (comment-only diff proof · protected-token balance · build/test/fmt/isolation) had
   passed.*

## 5. Issues / limits

- The AICodeDetect original PDF was not read (extraction failed) → its figures · direction stay **uncertain**. We adjusted the §Conclusion to claim only "not established wisdom"
  so that the conclusion does not depend on this paper.
- The Boris Cherny quote verbatim unverified (402). The source context of the SamHatoum rule unverified.
- **For "do not embed line numbers · symbol names in comments" we found no precedent in the AI instruction corpus** — it exists only as human-facing OSS practice
  (file-path stale discussions on the Linux LKML · PostgreSQL mailing lists). Codex also found only near cases
  (Coolify replaced "brittle line number references" — but whether that was inside a source comment is unconfirmed).
  → **This item is newly created based on measurement.**
- "No transcribing deliberation" **has precedent** (a guide for agents states, in paraphrase, that long architecture explanations go in an ADR, not in the source)
  — the draft's "original contribution" claim was refuted by the adversarial review and withdrawn.
- The HN thread (id=49078710) failed to load with a 429. No official Anthropic material on this behavior.
- Google/Ousterhout/Kernighan originals verbatim not obtained (secondary summaries only).

## 6. Method notes

Main-line collection = 3 parallel branches (research collector · lead lightweight). Grounding = the main agent's external verification (every load-bearing claim —
`#65961` quote cross-check, refutation of the ResearchGate statistic, direct check of arXiv 2508.21634). Adversarial review = cross-family
(non-lead senior · effort high · web_search) once, **level 2~3**, decision BLOCK · 12 findings.
**The adversarial review refuted all three of the draft's central negative claims (no controlled study · no working gate · no precedent for the no-transcribing-deliberation rule)**
— had cross-family been left out, this report would have put three confidently stated errors straight into the standard.
