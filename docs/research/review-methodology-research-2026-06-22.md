# Review/verification methodology research (multi-stage · multi-reviewer)

- Written: 2026-06-22
- Source: deep-research harness (5-angle fan-out → 26 sources fetched → 115 claims extracted → top 25 adversarially verified → 19 confirmed / 6 killed)
- Purpose: a survey of **methods that raise review quality through structure**, not "which model to use" — per-stage perspectives, blinding, mitigating LLM-judge bias, multi-agent patterns.
- Caution: each finding carries a **confidence level** and its **qualifiers**. Do not cite a single number as a universal law.

---

## 1. Verified findings (confirmed)

### F1. Perspective/lens-based review = Perspective-Based Reading (PBR) — [certain]
A review design that assigns each reviewer a **different stakeholder viewpoint** (tester / developer / user) and a **dedicated reading scenario** (user = use-case, tester = equivalence partitioning, designer = structured analysis). Premise: different perspectives find **non-overlapping defects**, and their union widens coverage.
- Basis: verbatim from Basili's original paper (peer-reviewed) — "members of a review team read a document from a particular perspective… combination of different perspectives provides better coverage than the same number of readers using their usual technique." (3-0)
- Sources: cs.umd.edu/~mvz/handouts/emp_pbr.pdf · link.springer.com/article/10.1007/BF00368702 · springer 978-3-642-29044-2_13

### F2. The PBR team effect is empirically demonstrated — [certain, but with large qualifiers]
One-person-per-perspective teams had statistically significantly higher defect coverage than unstructured review (1995 run: general document p=0.0007, NASA p=0.0390). Mechanism = non-overlapping coverage across perspectives.
- **Qualifier:** these "teams" are **SIMULATED teams** built by permutation after the experiment (not real collaborating teams). At the individual level the results are mixed (general p=0.0019 significant / NASA p=0.4755 not significant). The NASA documents had large overlap between perspectives, so the team gain was weak.

### F3. PBR vs CBR (checklist) — no universal winner, context-dependent — [certain]
- One controlled experiment (59 people, UML/OO design): individual detection rate PBR ~69% / CBR ~70% (nearly identical). CBR had lower cost-per-defect and was ahead in a 3-person virtual team. Conversely, Laitenberger (2001) found PBR ahead.
- "Are the Perspectives Really Different?" (Lanubile & Visaggio): **no significant difference** in detection rate, time, or coverage among the three perspectives — a refutation-style test of PBR's differentiation premise.
- **The largest factor in defect variance was the specification document itself, not the reading technique.**
- Implication: use perspective assignment, but do not over-trust "perspective means superior".
- Sources: academia.edu/53008777 · thescipub.com/pdf/jcssp.2017.470.495.pdf · springer 978-3-642-29044-2_13

### F4. Double-blind reduces prestige (anchoring) bias *moderately* — [certain]
A natural experiment on 5,027 ICLR papers (2017 single-blind → 2018 double-blind). Scores of the group of top-cited authors dropped significantly. But the effect is small, **not large enough to change acceptance rates**.
- Source: asistdl.onlinelibrary.wiley.com/doi/full/10.1002/asi.24582 (Sun et al., JASIST 2022, arXiv:2101.02701)

### F5. A side effect of blinding = lower agreement (reliability) — [likely]
Once author identity was hidden, **the variance of ratings among reviewers of the same paper rose significantly** (loss of context/shared cues).
- **Not unanimous:** the Bornmann et al. (PLOS One) meta-analysis finds a null effect of blinding on inter-rater reliability. → Treat it as "evidence from one large CS conference study".

### F6. ★Rubric/scale design is a stronger debiasing tool than blinding★ — [likely]
Same ICLR dataset: **changing the rating scale from 10 points to 4 points (coarser) reduced prestige bias about 4 times more than double-blind did** (the effect of prestige on acceptance ~3% → ~1.7%).
- **Qualifier:** observational study of a single venue, the two interventions were in different years (temporal confounding).
- Implication (the core of our design): **a coarse verdict scale (PASS/FIX/BLOCK) + a well-built checklist** is a higher-priority bias control than anonymization.

### F7. LLM self-preference (preferring its own model) is not a universal law — [likely]
The sign flips from model to model (one study: Flash +0.56 vs Pro −0.22). *Negative* self-preference is also reported in several models.
- **Qualifier:** the main source is a single-author, non-peer-reviewed preprint (arXiv:2604.23178), sample size unknown → low trust in the quantitative values. But the "non-universality" is robust across multiple sources.
- Implication: "the main session is Claude, so it always favors Claude" is **not established**. Do not design with self-preference as a constant.

### F8. Multi-judge panels (PoLL): diversity is the active ingredient, not headcount — [certain]
A small panel of **diverse** models has higher agreement with humans, lower intra-model bias, and is 7 times cheaper than a single large judge (Cohere PoLL, arXiv:2404.18796). An ensemble improves agreement with humans (kappa) over individual judges (arXiv:2408.09235).
- **Addendum:** the 2026 follow-up "Nine Judges, Two Effective Votes" (arXiv:2605.29800) — **a naive panel of correlated (same-family) judges has little value; diversity is essential**.
- Implication: opus+opus (same family) is weak, and **opus+Codex (different families) is the meaningful pair**.

---

## 2. Rejected claims (do not adopt — failed adversarial verification)

| Claim | Votes | Note |
|---|---|---|
| PBR is superior to CBR in inspection *time* | 1-2 | insufficient basis for a time-efficiency claim |
| PBR-CBR is a uniform yield↔cost trade-off | 0-3 | a simple trade-off formula does not hold |
| giving a reference answer removes self-preference | 1-2 | reference-guided does not remove the bias |
| **position (order) bias is negligible (≤0.04)** | 0-3 | **order bias is real → swapping answer positions is valid** |
| combining all mitigations (swap+CoT+rubric) gives the maximum gain | 0-3 | insufficient basis for a combined-maximum-gain claim |
| evidence that a diverse-family panel *directly* mitigates self-preference | 1-2 | do not attribute the panel's advantage to self-preference debiasing |

---

## 3. Gaps (what this survey could not fill)

- **Asymmetric reviewers (one context-blind, one doc/invariant-aware)** — no explicit empirical study or practical pattern **was found.** This is our hypothesis and unverified.
- **Comparison of the effect of multi-agent review patterns (generator-critic, self-refine/Reflexion, multi-agent debate, red-team/blue-team, devil's advocate)** — not included in this verification set (unresolved).
- **Whether PBR perspective differentiation is valid at the code-review stage** — the verification focused on requirements/design documents. Mapping to the code stage is extrapolation.
- Whether the debiasing from rating-scale coarsening transfers to the verbosity/position bias of LLM judges — no evidence.

---

## 4. Limits (overall)

- The inspection literature (PBR/CBR) is from 1996~2017 — a mature, stable field, so the vintage alone is not disqualifying, but **conclusions split depending on the unit of measurement (individual vs team) and the artifact type (requirements vs design vs code).**
- The blinding-related conclusions (F5·F6) depend on **a single observational study of one venue (ICLR)**. F5 conflicts with a meta-analysis.
- The main source for the non-universality of LLM judges (F7) is a weak preprint.
- Mostly evidence from **human inspection and academic peer review** extrapolated to LLM code review — not direct empirical evidence on LLM code review.

---

## 5. Implications for our design (summary)

1. **Bias control priority 1 = per-stage checklist + coarse verdict scale (PASS/FIX/BLOCK)** — stronger than blinding (F6).
2. **The active ingredient of multiple reviewers = family diversity** — opus+Codex is meaningful, opus+opus is weak (F8).
3. **Perspective (lens) assignment** — two different viewpoints per stage (F1/F2). But do not over-trust "perspective means superior" (F3).
4. **Blinding is optional** — only at stages with high anchoring risk (divergence), aware of the reliability cost. Mark asymmetric blind/doc-aware as an **unverified bonus** (gap).
5. **Self-preference is not a constant** — the solid backstop is not blinding but **diversity + a coarse rubric + a human (final decision)** (F7).
6. **Order bias is real** — when aggregating review results, be order-independent and anonymize labels (shown by the converse in the rejection table).
