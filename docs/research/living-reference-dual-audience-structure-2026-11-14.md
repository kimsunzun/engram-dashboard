# Living reference structure for two audiences — unit, promotion, harvest, and the Korean-HTML / English-md pair

- Status: **survey done — awaiting the user's decision** (structure options in section 3; recommendation = option A).
- Method: `/engram-research` medium, design-survey mode — three collectors (B1 reference unit + promotion + harvest · B2 dual-audience docs · B3 bilingual sync), main grounding (load-bearing quotes re-read in the raw page, marked ✔raw), codex review (section 4).
- Date: 2026-11-14.
- Starts from: `documentation-architecture-research-2026-06-24.md` (doc types, Diátaxis, SSoT, AGENTS.md sizing — not redone) and `design-doc-lifecycle-after-implementation-2026-11-13.md` (decided: reconcile the step TRD, then freeze it; current behavior lives in `docs/reference/`). This report answers what that decision left open: how the maintained reference set is structured, what goes into it, how TRD-time briefings roll into it, and how the two audiences are served.
- New variable from the user: two audiences — the user reads Korean, picture-first HTML; agents read dense English markdown.
- Confidence legend: 확실 (independent primary sources agree, or a primary rule read raw plus a second source) · 가능성 높음 (one primary source, grounded) · 불확실. Section 3 is main's inference — one level lower than the findings.

## 1. Findings

### 1-1. Unit of a maintained reference set: a whole-system map plus one file per capability/component — 확실
- **OpenSpec** ✔raw (`docs/concepts.md`): current behavior lives in `openspec/specs/<capability>/spec.md`; "A spec is a **behavior contract**, not an implementation plan." Collector-relayed: grouping by feature area, component or bounded context is allowed; "one spec per logical capability".
- **Kiro steering** ✔raw (https://kiro.dev/docs/steering/): `product.md` · `tech.md` · `structure.md` "are included in every interaction by default, forming the baseline"; further topic files load conditionally (`fileMatch`), manually or automatically.
- **arc42** (collector, summary): 12 fixed sections; the Building Block View is the system map, crosscutting concepts are per-topic sections.
- **Chromium** (collector, summary): a README.md in every directory "that merits a container directory" — unit = code component.
- **Rust Reference / Python Language & Library Reference**: unit = language concept / stdlib module (B1, summary).
- No size norm per file was found (abstain). The unit differs by tool (capability · concern · view · component); what is shared is *one always-read map + per-unit detail*.

### 1-2. What gets promoted: behavior others rely on, plus current structure — 가능성 높음
- **OpenSpec** ✔raw: "Observable behavior users or downstream systems rely on"; quick test: "If implementation can change without changing externally visible behavior, it likely does not belong in the spec." Internal names, library choices and step-by-step plans belong in `design.md` / `tasks.md`.
- **Rust Reference** ✔raw (stabilization dev-guide): new features "usually require an update to the Reference"; exception — areas the Reference does not document ("changes to the details of type inference do not require an update to the Reference").
- **Chromium** (collector, summary; reviewer re-read): a component README says "what that component is, how it is expected to be used and provides rough outline of the code's structure" — structural reference, outside OpenSpec's behavior-only test.
- **Martraire, Living Documentation** (secondary summary, Hilton): document knowledge that is long-lived, read by many, or high-value; skip what is readily recreatable.

### 1-3. Harvest: the change carries a delta; the implementer applies it at a gate; the design record freezes — 확실
- **OpenSpec** ✔raw: each change folder holds delta specs (`ADDED` / `MODIFIED` / `REMOVED` requirements); at archive they are applied to `openspec/specs/…` ("✓ Synced specs to openspec/specs/auth/spec.md") and the change folder moves to `changes/archive/<date>-<name>/`. Collector-relayed: `MODIFIED` copies the whole requirement block. The sync is skippable ✔raw (`docs/workflows.md`): "Sync skipped … Archive without syncing"; the CLI `openspec archive` "still validates, then applies any delta specs" — the merge is a default, not a hard gate.
- **Rust** ✔raw: the feature author writes the Reference PR — "This can be done at any time before stabilization, and it is usually better to prepare a PR early". Collector-relayed (snippet): lang-docs approval is required before the stabilization PR merges — 불확실.
- **Python PEP 1** (✔raw in the 11-13 report): a resolved PEP is "a historical document rather than a living specification"; expected behavior is "maintained elsewhere, such as the Language Reference".
- **Kubernetes KEP** (collector, summary): the release-signoff checklist requires user-facing docs in kubernetes/website — a rewrite for users, not a merge of KEP text.
- Spec Kit (`specs/NNN-feature/`) and Kiro per-feature specs showed no harvest step in the pages read — hypothesis, not a complete survey.

### 1-4. Two audiences: one authored source; views may differ per audience — 가능성 높음
- **Mintlify** ✔raw: auto-generated `llms.txt` / `llms-full.txt` / per-page `.md` from the authored docs; "Adding a custom file overrides the automatically generated file of the same name."
- **Vercel** (`Accept: text/markdown` on the same URL), **Stripe** (append `.md` to any docs URL), **Docusaurus plugins** (build-time `.md` / `llms-full.txt`) — collector, summary/snippet.
- In these markdown (or MDX) is the source. The views are not always the same text: **Mintlify `Visibility`** ✔raw (`/docs/components/visibility`) shows "different content to humans, AI agents … on the same page" — `for="humans"` renders on the site, `for="agents"` only in the `.md` view; interactive components (calculators, charts) get a static agent-side stand-in. Shape = one source file, shared facts, audience-specific blocks.
- **Spec + separate explainer per unit — W3C WCAG "Understanding" documents** ✔raw (`/WAI/WCAG22/Understanding/about`): "Informative explanations, not required to meet WCAG"; one per success criterion, to "better understand the intent", with "background information" and "techniques that are examples". The normative text stays in WCAG; the explainer adds intent and examples without becoming normative. (Found by the reviewer.)
- No precedent found for a bilingual pair (English spec + Korean explainer) with a drift check (absence = hypothesis).
- Practitioner guidance says write one doc that serves both readers — **Tian Pan** ✔raw: "The harder discipline is to write the primary docs so they serve both readers"; also Upsun, Passo (summary). All assume one language and one prose form; none covers a Korean visual explainer of an English spec.
- **Diagrams as code** — **Structurizr** ✔raw (`docs.structurizr.com/as-code`): many diagrams from a single model; C4-PlantUML, Kroki, Asciidoctor (collector). The text model is canonical; humans get rendered pictures, agents can read the text.
- Evidence that agents gain from diagram source over prose is thin: arXiv 2608.21747 (Canedo, 2026-08) ✔raw abstract — "On the strongest models (Sonnet 4.6, GPT-5), format barely matters (quality spread 0.17-0.92)"; it measures code generation from architecture specs, not reading reference docs.

### 1-5. Two-language sets: pin the source revision in the derived file, per page — 확실
- **MDN translated-content** ✔raw: "Add a new `l10n.sourceCommit` key, which contains the commit hash of the latest commit that modified the file." Adoption is partial and the update section of the guide is a placeholder (collector).
- **OpenTelemetry** ✔raw: `default_lang_commit` front matter per localized page; `drifted_from_default` marks a page drifted — "the page displays an 'outdated' banner, and the link checker skips" it; `npm run check:i18n` lists pages to update.
- **FSFE** (collector, opened): a `<version>` element compared between original and translation; outdated → yellow box.
- **Kubernetes** ✔raw: timestamps mislead — "A localized page may look recently updated because of a small local edit while still missing an important upstream change." Its 2026 blog: "AI assistance should support human review, not create a new stream of unverified content."
- **Regenerate instead of edit**: genaiscript continuous translation ✔raw (hash per markdown node, LLM-as-judge gate, skip when unsure); MDN's machine-translated Italian locale is regenerated on every English change and manual edits are overwritten (snippet — 불확실).
- The pin works for hand-maintained pages: OpenTelemetry's localized pages are written by contributors, and the pin + `check:i18n` + banner apply to them ✔raw. The pin needs a process that runs it — OpenTelemetry keeps drift status through its PR checks (reviewer; the housekeeping schedule was not re-read).
- No practice was found for tracking staleness of a *non-literal* derived page (summary, explainer) — hypothesis. The nearest pattern is the same source-revision pin.

## 2. What this repo has today (fact, from the tree)
- `docs/reference/architecture.md` = the whole map (modules, connections, runtime invariants); `docs/reference/structure/<topic>.md` = one file per topic ("one file = one topic", hub `docs/README.md`); convention files beside them.
- One pair already exists: `structure/agent-backend.md` (English, canonical detail) + `structure/agent-backend.html` (Korean, visual), and the HTML says the md is canonical and some sections live only there.
- `structure/session-path-ownership.md` is a dated code trace (2026-09-09) that calls itself not the source of truth — a snapshot inside the living folder.
- TRD briefings are Korean HTML in the step folder (`docs/process/<task>/…html`, explain binding). Implement binding: "md 가 바뀌면 HTML 도 같이 고친다" — a hand-sync rule with no check.
- Language rule (`CLAUDE.md`): `docs/` md in English; what the user reads (HTML reports) in Korean.

## 3. Mapping to this repo (main's inference — one confidence level lower)

### 3-1. Options for the pair

| Option | Shape | Precedent | Drift | Cost for this repo |
|---|---|---|---|---|
| **(A) md canonical + HTML as a non-normative explainer, pinned** (recommended) | Per topic `X.md` (English, the only place facts are stated) + `X.html` (Korean, written by `/engram-explain` from `X.md` + the ADRs it links; adds pictures, intent, examples — never a fact the md lacks). The HTML records the md commit it was written against. | WCAG + Understanding docs (normative text + per-unit explainer); MDN/OTel (source pin on hand-written pages) | Pin + check at step end lists HTMLs whose md moved; those are refreshed (regenerated or edited, then re-pinned) | One explainer refresh per changed topic per step; the "no new facts in HTML" rule must be reviewed |
| (B) md single source, HTML generated mechanically | `X.html` rendered from `X.md` (+ audience blocks, Mintlify `Visibility`) | Mintlify/Vercel/Stripe; MDN-it/genaiscript (regenerate, never edit) | None by construction | Korean picture-first teaching needs more than a rendering: either the md carries Korean human-only blocks (two languages in one agent file) or an LLM translates/explains — then it is (A) |
| (C) Paired, both carry facts | Same two files; facts edited in both | Today's `agent-backend` pair (its HTML already holds facts the md lacks) | A pin + check works here too (OTel), but it detects *staleness*, not *divergence* of facts | Every fact change edits two files in two languages |
| (D) Separate sets per item | Human set and agent set organised independently | AGENTS.md vs README — different content | Same facts in two independent shapes | Two maps, two indexes |

### 3-2. Recommended structure (main's inference)
1. **Unit** — keep the current shape: `architecture.md` = the one map every brief reads; `structure/<topic>.md` = one file per topic (a capability or subsystem other code relies on). The first step that introduces a topic creates its file.
2. **Two admission rules for a topic md** (reviewer finding 3 — OpenSpec's test alone would drop structure):
   - Behavior and contracts — what callers, other modules or the user rely on and can observe: interfaces, data shapes, file formats, rules, invariants. Gate = OpenSpec's quick test.
   - Current structure — what the component is, how it is used, its rough code layout and main types (Chromium README). Gate = "a coder starting the next step needs it and would otherwise read many files to rebuild it".
   - Out: dates, progress, review rounds (commit message), the "why" (ADR — linked, not copied), details the code shows directly.
3. **Pair per topic, same name, same folder** — `X.md` + `X.html`, like WCAG + Understanding. Facts live only in `X.md`. Diagrams that state structure are Mermaid in `X.md`, so agents read the same graph the HTML draws.
4. **The HTML is an explainer, not a second spec** — written from `X.md` + linked ADRs (for intent) + the frozen TRD briefings of the steps that touched the topic (for pictures). It may add intent, examples and pictures; it may not state a fact absent from `X.md`. Its head shows the md commit it was written against.
5. **Pin, check, refresh — at step end** — a small script lists pairs where `git log <pin>..HEAD -- X.md` is non-empty. It runs in the step-end pass (the same moment the TRD freezes); listed HTMLs are refreshed and re-pinned in that pass. Between step ends the HTML may lag by at most the current step — no live banner (reviewer finding 5: nothing would update one).
6. **Harvest from the TRD** — the TRD names the reference delta (which topic files gain or change which statements; OpenSpec delta). The coder applies it to the md in the same commit as the code; code review checks it. TRD briefing HTMLs freeze with the TRD; their pictures are material for the topic HTML refresh, not merged by hand.
7. **Snapshots leave the living folder** — dated traces like `structure/session-path-ownership.md` move to `docs/research/` (or are marked snapshot), so `docs/reference/` holds only current statements.
8. **Trial gate** (reviewer finding 4) — the first topic (S25 preset library) is the trial: the user judges whether the explainer teaches as well as a hand-made briefing. If not, keep (A)'s pin and "facts only in md" rule but have the HTML hand-directed by the user per refresh — the structure stays, only the writing mode changes.

### 3-3. Constraint fit

| Constraint | (A) | (B) | (C) | (D) |
|---|---|---|---|---|
| Agents read one canonical English source | ✔ | ✔ | △ HTML holds extra facts | ✗ |
| User reads Korean, picture-first, top-down | ✔ untested until the trial | △ rendering is not teaching | ✔ | ✔ |
| Low upkeep for a solo user with LLM writers | ✔ one fact write + one refresh per step | ✔ | ✗ two fact writes | ✗ |
| Drift detectable | ✔ staleness by pin; divergence by the no-new-facts rule | ✔ by construction | △ staleness only | ✗ |
| Fits existing `docs/reference/` layout | ✔ | ✔ | ✔ | ✗ |

Rejected for the ADR: (D) — same facts in two independent sets; (B) as a mechanical rendering — cannot produce a Korean picture-first explainer without an LLM step, which makes it (A); refreshing the HTML on every commit — the user reads at step end.

## 4. Review
codex (effort high), verdict **FIX**, seven findings — all applied (main re-read the counter-evidence of 1, 6, 7 in the raw page):
1. **Refuted** — "the views are the same text": Mintlify `Visibility` serves human-only and agent-only blocks from one source → 1-4 corrected; option (B) notes audience blocks.
2. **Refuted** — pins were credited only to the generated option; OpenTelemetry pins hand-written pages → 1-5 line added; 3-1 compares staleness vs divergence per option.
3. Scope mismatch — OpenSpec's behavior-only test would drop structural reference (Chromium READMEs keep classes and layout) → 3-2 item 2 split into two admission rules.
4. Unsupported — teaching quality of a generated explainer untested; explanation needs reasons → explainer inputs include linked ADRs (3-2 item 4); trial gate (item 8); constraint table marks it untested.
5. Missing mechanism — a stored pin does not update a published banner → banner dropped; check runs in the step-end pass (3-2 item 5).
6. Omission — W3C WCAG Understanding docs (spec + per-unit explainer) and Asciidoctor includes → WCAG added to 1-4 and used as option (A)'s precedent; includes not pursued (no shared-fragment need while facts live only in md).
7. **Refuted** — OpenSpec sync can be skipped at archive → 1-3 corrected.

## 5. Gaps
- Not opened: C4/Structurizr beyond the as-code page, Martraire's book, BMAD primary docs, Python/React/Rust translation workflows, Docusaurus discussion #9947, yari PR #8338.
- No source measures whether any drift mechanism works in practice; MDN's partial `sourceCommit` adoption is the only adoption data.
- No precedent for a hand-made or generated *explainer* (not a translation) paired with a spec; the pin-and-regenerate design for it is main's inference.
- Whether a generated Korean explainer is good enough for the user to learn from (vs today's hand-made briefings) is untested — the first topic is the trial.
