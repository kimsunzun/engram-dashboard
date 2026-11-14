# Design-doc lifecycle after implementation — freeze, update, or in between (and what an LLM-agent workflow needs)

- Status: **decided 2026-11-13 — option A** (reconcile, then freeze; link, do not copy; reference docs updated in the same commit; a fundamental design problem found mid-step = revise the whole TRD and re-review). User: 「생각해보니 그게 맞는것 같은데?」 · 「어쨋든 왠만하면 안고치는데 근본적으로 문제있으면 trd 자체를 전체적으로 한번 갱신한다는거아냐」. First applied to S25 step 8; skill changes recorded in engram-implement `feedback.md` (2026-11-13). Moving the current facts out of the S25 TRDs into `docs/reference/` is deferred to a separate session.
- Method: `/engram-research` medium — two collectors (branch A: established proposal processes · branch B: AI-coding-agent spec workflows), main grounding (raw re-check of the load-bearing quotes marked ✔raw), codex review (see 「Review」).
- Starts from: `documentation-architecture-research-2026-06-24.md` (design doc = "snapshot on approval"), which `docs/handbook/documentation-system.md` §3 adopted ("design/research become a snapshot after approval and freeze"). This report adds what that one did not cover: where deviations and hand-overs go, and the agent-workflow angle.
- Trigger: S25 steps 7 and 17 — the doc pass after each step (adding 「as landed」 blocks to the child TRD, copying them into the parent TRD, step-log, ADR anchors) cost about as much review as the code (step 17: two doc-review rounds vs two code-review rounds).
- Confidence legend: 확실 (two independent primary sources or raw-verified primary rule) · 가능성 높음 (one primary source, grounded) · 불확실.

## 1. Findings

### 1-1. Proposal processes freeze the document once it is resolved — 확실
- **Python PEP 1** ✔raw: "In general, PEPs are no longer substantially modified after they have reached the Accepted, Final, Rejected or Superseded state. Once resolution is reached, a PEP is considered a historical document rather than a living specification." (https://peps.python.org/pep-0001/)
- **Rust RFCs** ✔raw: "In general, once accepted, RFCs should not be substantially changed. Only very minor changes should be submitted as amendments. More substantial changes should be new RFCs, with a note added to the original RFC." (https://github.com/rust-lang/rfcs/blob/master/README.md)
- **Kubernetes KEPs** (collector-relayed): status `implemented` = "The KEP has been implemented and is no longer actively changed."; `replaced` points at the new KEP with `superseded-by`. (KEP-0000 process README) — 가능성 높음.
- **ADRs** (collector-relayed): Fowler — "Once an ADR is accepted, it should never be reopened or changed - instead it should be superseded."; Nygard — reversed decisions are kept and marked superseded. — 가능성 높음 (matches this repo's ADR practice).

### 1-2. The middle ground: update during implementation, freeze at "final" — 가능성 높음
- **PEP 1** ✔raw: "If changes based on implementation experience and user feedback are made to Standards track PEPs while in the Provisional or (with SC approval) Accepted state, they should be noted in the PEP, such that the PEP accurately describes the implementation at the point where it is marked Final." — changes are recorded while the PEP is Provisional/Accepted so that it is accurate when marked Final, after which it is historical. PEP 1 does not say the corrections are made in one pass at the end; doing them once per step is this repo's option (section 3), not PEP's rule.
- **Google design docs (Malte Ubl)** ✔raw: "If the designed system hasn't shipped yet, then definitely update the doc. In practice we humans are bad at updating documents, and for other practical reasons changes are often isolated into new documents." (https://www.industrialempathy.com/posts/design-docs-at-google/)
- **KEP template** (collector-relayed, paraphrase): one KEP per feature for its whole lifecycle, edited as details emerge, with an 「Implementation History」 section for milestones. — 불확실 on the exact rule.

### 1-3. The as-built truth lives somewhere else — 확실 for PEP, 가능성 높음 for Rust
- **PEP 1** ✔raw: "Formal documentation of the expected behavior should be maintained elsewhere, such as the Language Reference … the Library Reference … or the PyPA Specifications".
- **Rust** (collector-relayed; the stabilization guide page did not open for main): stabilization needs "a complete PR to the Reference describing the feature", and the stabilization report lists "The main design decisions and deviations since the RFC was accepted". Every accepted RFC has a tracking issue (RFC README ✔raw: "Every accepted RFC has an associated issue tracking its implementation").
- **Go** (collector-relayed): once accepted, "the issue is repurposed to track the work of implementing the proposal".

### 1-4. Follow-ups are named sections or new documents — 가능성 높음
- Rust RFC template (collector-relayed): 「Unresolved questions」 ("What parts of the design do you expect to resolve through the implementation of this feature before stabilization?") and 「Future possibilities」.
- Substantial later change = a new RFC/KEP/ADR that points back (1-1).

### 1-5. AI-coding-agent workflows: no settled practice; the reported failure is duplicated, repetitive spec text — 가능성 높음
- **Three levels** (Böckeler, martinfowler.com, 2025-10-15, collector-relayed): spec-first (spec discarded or left to drift after the task) · spec-anchored (kept and updated for evolution) · spec-as-source (only the spec is edited). She found Spec Kit's output "repetitive" and preferred "to review code than all these markdown files". An academic overview (arXiv 2602.00180) adds: spec-anchored "teams must commit to updating specs whenever behavior changes", kept aligned by tests derived from the spec; risks = specification rot, over-specification ("essentially becoming pseudo-code").
- **Anthropic's long-running-agent harness** (2025-11-26, collector-relayed): a frozen feature list where agents may change only the `passes` field ("It is unacceptable to remove or edit tests…"), a progress file, and the git log; a fresh session reads "the git logs and progress files to get up to speed".
- **Claude Code best practices** ✔raw: write the spec to `SPEC.md`, then "start a fresh session to build from it, so the build begins with a clean context and a written definition of done". CLAUDE.md should hold only what Claude cannot work out from the code (collector-relayed wording).
- **Spec Kit** (vendor doc, collector-relayed) claims specs are the living source with code "continuously regenerated", and keeps plans "high-level and readable" with detail pushed to `implementation-details/`. **Kiro** (collector-relayed summary): specs "designed for continuous refinement", one spec per feature, "Sync Files" to reconcile tasks with work done.
- **OpenSpec** ✔raw (https://github.com/Fission-AI/OpenSpec, `docs/editing-changes.md`, `docs/getting-started.md`) — a reconcile-then-archive workflow: each change has its own folder with delta specs; when the code is right and the spec stale, "Update the delta spec (and tasks, if relevant) to describe the behavior you actually shipped. The spec should match reality before you archive"; `/opsx:verify` lists where artifacts and code diverge; "at archive time, your specs become the truth of record"; on archive the change's specs merge into `specs/` ("The source of truth. These specs describe how your system currently behaves") and the change folder moves to `changes/archive/` for audit history. — 확실 (raw-read).
- Apart from OpenSpec, no primary source read gives a dedicated "what actually landed" section — 불확실 (not a complete-corpus search).

### 1-6. Duplication — 가능성 높음
- arc42 tip 4-4 (collector-relayed): "Avoid redundancy, don't repeat information from views or concepts" — link instead. This repo's own rule already says the same: `documentation-system.md` §3 「SSoT — link, do not copy. Duplication = stale cost.」
- None of the proposal processes states an explicit no-copy rule for design docs; they avoid it structurally (one proposal + one reference + one tracking issue). — 불확실 as an absence claim.

## 2. What this repo does today (fact, from the tree)
- Child TRD per big step (e.g. `docs/process/S25-preset-library/trd-asset-library.md`) is written before coding, then after landing it gets 「as landed」 rewrites of its sections, a §12 hand-over list and a §13 review record.
- The parent TRD (`trd.md`) repeats the landed facts in a 「Step N landed」 block and copies the hand-overs into each later step's row (「Handed over by step N」).
- `docs/process/step-log.md` gets a narrative entry; ADRs get written/linked; code gets `// ADR-NNNN` anchors.
- The same fact (e.g. "priming moved to `builtin/instructions/priming.md`") therefore sits in the commit message, the child TRD, the parent TRD's step row, its 「landed」 block, a hand-over paragraph, the step-log, and ADR-0368 — and each copy had to be reviewed.

## 3. Mapping to this repo (main's inference — one confidence level lower than the findings)

The question is two independent choices (review finding 3):
1. **What happens to a step's change document (the child TRD) after the step lands** — freeze at approval · reconcile to what shipped, then freeze · keep it living.
2. **Copy or link** — independent of 1; the sources (arc42, PEP/Rust's single reference, OpenSpec's merge) and this repo's own SSoT rule all say link. Every option below assumes link: the parent TRD gets one status line + link, hand-overs are written once in the receiving step's own section, step-log is one line.

And a third thing every option must name (review finding 5): **the maintained source a future brief reads** — PEP's "maintained elsewhere", OpenSpec's `specs/`, Chromium's component docs vs historical design links. In this repo that is `docs/reference/*` (architecture, conventions), ADRs and the code; a frozen TRD is history plus the hand-over list for the next step.

| Option (choice 1) | After a step lands | Precedent | Costs / risk |
|---|---|---|---|
| **A. Reconcile, then freeze** (recommended) | Before closing the step, edit the child TRD in place only where the shipped design differs (decisions, interfaces, values) — no separate 「as landed」 narrative — and mark it Implemented with the commits; then it is history. Durable truths (module boundaries, contracts, rules) are promoted into the maintained source (`docs/reference/*`, ADRs). | PEP accuracy-at-Final (1-2), OpenSpec reconcile-before-archive + merge into `specs/` (1-5), Ubl "update if not shipped" (1-2) | One reconcile pass per step; needs a clear rule for what is promoted (otherwise detail stays only in code/commits — review finding 5) |
| **B. Freeze at approval** | TRD untouched after approval; deviations only in commit messages and step-log | PEP/RFC/KEP after resolution (1-1). (Anthropic's harness freezes *requirements/tests*, not the design — it does not support freezing a technical design; review finding 4) | A later step's brief can cite a design that did not ship; a fresh coder is misled unless the main session corrects the brief by hand |
| **C. Living child TRD** (kept current for good) | Keep editing the child TRD whenever the code changes, as the maintained design doc | Böckeler's spec-anchored (1-5), NASA-style as-built documentation (review finding 6) | Two maintained sources (TRD and `docs/reference/*`) unless one is retired; a lean living TRD with links is cheaper than today but still adds an update duty to every later change |

Today's practice (section 2) is closest to C **plus copying**; most of its cost is the copying, which every option drops.

Why A (inference): the coders and reviewers are fresh subagents briefed from the TRD (Claude Code's SPEC.md pattern, 1-5), so the brief for the *next* step must not cite a design that did not ship (against B); and the repo already has a maintained source for current truth (`docs/reference/*`, ADRs — PEP's "maintained elsewhere"), so keeping the TRD living as well (C) duplicates it. A keeps one maintained source and one honest historical record. Not established by the sources: that A is cheaper than a lean C in practice — no measurement exists (section 5).

## 4. Review
codex (gpt-6.1-sol, effort high), verdict **FIX**, six findings — all applied:
1. Overclaim — PEP 1 does not say "once, at the end" (not-found) → 1-2 reworded; "once per step" is this repo's option.
2. Omission — OpenSpec reconcile-then-archive (verified) → added to 1-5 (main raw-read the two docs).
3. Logic gap — freeze-vs-maintain and link-vs-copy were bundled; a lean single-source living spec was missing → section 3 split into two choices; option C reframed as a lean living TRD.
4. **Refuted** — Anthropic's harness freezes functional requirements/tests, not technical design → removed as support for B.
5. Logic gap — A must name the maintained source future briefs read → section 3 「maintained source」 paragraph and option A's promotion rule.
6. Omission — maintained as-built design (NASA SWE-078) and generated reference (Sphinx autodoc) → noted under option C; generated reference already exists here for wire types (ts-rs bindings).
The reviewer rechecked PEP, Rust RFC/stabilization, Google, KEP, ADR, Go, Claude Code and Spec Kit sources independently; its summary: "A remains plausible; its superiority is not yet established" — reflected in the last sentence of section 3.

## 5. Gaps
- Not opened by anyone: Martraire 「Living Documentation」, Write the Docs on drift, adr.github.io, Tessl docs, OpenAI Codex docs, Cursor official docs (forum snippets only), Go `design/` post-acceptance practice, kep.yaml field rules.
- Rust stabilization-guide quotes are collector-relayed (the page did not open for main).
- No quantitative data on the cost of keeping specs in sync (only anecdote and one vendor page).
