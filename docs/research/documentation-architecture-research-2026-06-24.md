# Documentation architecture research for the AI era (detailed record)

> **Status:** survey done (Claude 2 tracks + Codex 1 cross-check). **A recommendation for applying to engram is presented — awaiting the user's decision.** Writing the unified guide is a separate matter.
> **Stage:** consulting/research (survey → recommendation → user decision). Not an implementation.
> **Method:** Claude (Sonnet) 2 tracks of independent BLIND surveys (① doc-type/IA/meta-guides ② AI efficiency) + Codex 1 independent cross-check (whole) → opus cross-comparison and synthesis. (= the `research` skill pattern.)
> **Date:** 2026-06-24
> **Confidence labels:** (certain) converged by many / primary source · (likely) circumstantial · single source · (uncertain) unconfirmed.

---

## 0. Why (Context)

engram's docs/ uses several document kinds (README/index · decisions=ADR · process=step-log · tracking.md · research/ · reference/ reserved). We investigated whether the whole can be aligned to **industry standards + efficiency in the AI era** and whether we can write a unified guide saying "this is our documentation system." (Comment conventions and ADR placement have separate research already completed.)

---

## 1. Full map of doc-types + immutable vs living (3 tracks converge · certain)

| Type | Purpose/audience | Lifespan | Placement |
|---|---|---|---|
| README / index | "what · why · how to start", the entry point | Living | repo root + `docs/README.md` portal |
| **ADR** | context, decision and consequences of a hard-to-reverse decision | **Immutable (append/supersede)** | `docs/decisions/NNNN-*.md` |
| Coding convention | recurring code rules · review criteria | Living | `docs/handbook/` or `reference/style/` |
| Reference (canon) | API · settings · contracts | Living (synchronized with code) | `docs/reference/`, generated from code/schema where possible |
| Design doc | pre-implementation design · trade-offs | Snapshot on approval | `docs/designs/` |
| Research/spike | unsettled investigation · experiments | Archive after freeze | `docs/research/` (+ a `status:` label) |
| Changelog | change history | **append-only** | root `CHANGELOG.md` (or step-log) |
| Tracking | undecided items · TODO | Very living | an issue tracker (SoR) recommended, only an index in docs |
| Runbook | operations · incident procedures | Living | `docs/runbooks/` |
| Handbook | team operations · definition of the documentation system | Living | `docs/handbook/` |

**Spelling out the boundaries (likely):** ADR · changelog = append / design = approved snapshot / reference · runbook · convention · handbook · README = living. For living documents, git history is the history (no change-history section inside the file), and freshness goes in `last-reviewed:` · `status:`.

## 2. Information architecture (certain)
- Use **Diátaxis** (tutorial/how-to/reference/explanation) as the base IA. ADR · design · research · runbook are supplemented separately as "engineering artifacts" (they do not fit Diátaxis cleanly).
- **docs-as-code**: docs also get VCS · PR review · CI (link/orphan/lint) · owner. GitLab splits top-level by audience (`doc/user` · `doc/development`).
- **SSoT**: "link, do not copy" — duplication = the cost of going stale.
- **Orphan prevention**: `docs/index` + folder READMEs + link graph + **orphan check as a build failure** (Read the Docs). Isomorphic to engram's "no orphans" convention.

## 3. Unified guide (handbook) meta-patterns (certain)
- Mature organizations treat **documents like product/code** (owner · source control · review · freshness). Designate a canonical source (Google SWE Book).
- **A single giant file < folders + split markdown + a landing index.** A monolithic wiki that mixes purposes fails.
- Cases: **GitLab Handbook** (handbook-first, MR-based change = process control plane, SSoT, CODEOWNERS) · Google **g3doc** (docs next to code) + **eng-practices** (separate) · **Microsoft Engineering Playbook** (MkDocs multiple sections + the meta-rule "know, follow and fix the playbook").
- Recommended location of the guide itself: **`docs/handbook/documentation-system.md`** ("this is our documentation system"), linked from `docs/README.md` · `AGENTS.md`.

## 4. Efficiency in the AI era (certain, partly single source)
- **Agent instruction files:** `AGENTS.md` (a README for agents, standardized by the Linux Foundation, 60k+ repos) · `CLAUDE.md` (org/user/project/local hierarchy, nearest-file-wins, directory lazy-load).
- **Keep always-load short:** Anthropic recommends CLAUDE.md ≤200 lines, in practice 20~80 lines felt. From about 80 lines recall drops, and a long file = context squatting · lower compliance. Only "commands · prohibitions · standard locations · tests · links", with details through **on-demand links/path-scoped rules**. (likely — vendor docs + measurement reports)
- **auto-gen backfires (likely):** an LLM-generated AGENTS.md gives -0.5~2% success rate · +20% cost (ETH Zurich arXiv 2602.11988), and the more detailed it is, the more exploration steps increase. **The minimum amount written by hand** is best.
- **Pairing soft vs hard guardrails:** documents (soft, intent · reason) + lint/typecheck/test/CI/hooks/CODEOWNERS (hard, enforcement). Memory is context, not enforcement (stated explicitly in the Claude docs). Invariants must also be made hard.
- **spec-driven:** requirements→design→task→implementation reduces agent guessing (the 2025~2026 trend, likely).
- **llms.txt:** effective for public docs/websites, but **for an internal codebase AGENTS.md + a docs index is more direct**. No official support from the major LLMs.

## 5. Cross-verification table (Claude ↔ Codex)

| Item | Converged | Diverged/complemented |
|---|---|---|
| doc-type map · immutable/living | Fully consistent | Codex added CHANGELOG (Keep a Changelog) · "tracking = the issue tracker as SoR" |
| Diátaxis · docs-as-code · SSoT · orphan prevention | Consistent | Codex added orphan = build failure (Read the Docs) |
| Meta-guide (handbook) pattern | Consistent (GitLab/Google) | Codex added the MS Engineering Playbook · the location `handbook/documentation-system.md` |
| AI efficiency (short always-load · soft/hard · llms.txt) | Strongly consistent | Claude = ETH auto-gen backfire · 200/500 thresholds / Codex = the 20~80 line practical value |

**Conclusion: zero real conflicts between the two families (converged).** All differences are complements that one side covered at greater length → confidence in the landscape is high (certain). However, "the auto-gen backfire figures" and "recall drop at 80 lines" are single-source, so (likely).

## 6. Recommendations for applying to engram (against the current system)

engram's current docs/ = README (portal) · decisions/ (ADR) · process/ (step-log) · tracking.md · research/ · reference/ (reserved, empty). **Already roughly aligned to the standard structure.** Gaps and recommendations:

- **(Recommendation A) Consider slimming CLAUDE.md** — currently **~12k tokens (≈300+ lines)**, far exceeding the always-load recommendation (≤200, practical 20~80). The cost of always loading rises. Move details (the body of architecture invariants · module map · session restore etc.) out to on-demand documents and make CLAUDE.md a **thin router** (commands · prohibitions · standard locations · a one-line core invariant + links). Note: a large change, so a separate decision.
- **(Recommendation B) Create a unified documentation guide** — "this is the engram documentation system" is now scattered across CLAUDE.md + docs/README.md. Put doc-type placement · immutable/living · always-load vs on-demand · no orphans in one place, in `docs/handbook/documentation-system.md` (or an expanded docs/README.md). (The results of this research are the raw material for that draft.)
- **(Recommendation C) Fill reference/** — make the first resident of the folder that is only reserved a living canon (the comment convention etc.).
- **(Recommendation D) Minimal doc CI set** — markdown lint · link check · **orphan check (build failure)** · generated reference freshness. Right now "no orphans" is only a convention (soft) → reinforce it as hard (CI).

## 7. Points of contention (user judgment)
- **tracking.md vs issue tracker:** Codex recommends keeping tracking in the issue tracker (SoR) and only an index in docs (long-lived tracking in docs = risk of going stale). engram deliberately uses `docs/tracking.md` as a living backlog (T-/D-). → For solo/small scale, keeping docs/tracking is simpler; reconsider when the team grows. **User decision.**
- **CHANGELOG vs step-log:** Codex recommends a root CHANGELOG (Keep a Changelog). engram's step-log (when/what) effectively plays that role → a separate CHANGELOG seems unnecessary. But if "release notes for users/operators" become needed, separate them.

## 8. Gaps / limits
- **No established frontmatter standard (type/audience/status/last-reviewed)** — it can be introduced but there is no single industry standard (uncertain).
- The auto-gen backfire and the line-count threshold are single-source (likely) — never trust them absolutely before measuring in engram.
- Actually writing the unified guide and slimming CLAUDE.md are outside this research's scope (recommendations only).

## 9. Main sources
- Diátaxis https://diataxis.fr/ · docs-as-code https://www.writethedocs.org/guide/docs-as-code/
- GitLab Handbook https://handbook.gitlab.com/handbook/about/handbook-usage/ · Google SWE Book ch10 https://abseil.io/resources/swe-book/html/ch10.html · MS Engineering Playbook https://microsoft.github.io/code-with-engineering-playbook/
- ADR https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions · Keep a Changelog https://keepachangelog.com/en/1.1.0/
- AGENTS.md https://agents.md/ · CLAUDE.md memory https://code.claude.com/docs/en/memory · auto-gen backfire arXiv 2602.11988 · spec-driven arXiv 2604.05278 · llms.txt https://llmstxt.org/ · Read the Docs orphan https://docs.readthedocs.com/
