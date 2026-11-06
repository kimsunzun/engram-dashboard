# Engram verification pipeline design (proposal → partly applied)

- Written: 2026-06-22 · Status: **reflected in CLAUDE.md (2026-06-22)** + this document is the detailed reference. Recording as an ADR is undecided (§6).
- Basis: `docs/research/review-methodology-research-2026-06-22.md` (each item marked Fn)
- Premises (user decisions done): judge models = **opus + Codex only**, sonnet = lower-level coder only, web consult discarded, final decider = the user.

---

## 0. Principles (derived from the research)

1. **Bias is caught by a rubric, not by anonymization.** Per-stage checklist + coarse scale (PASS/FIX/BLOCK). No fine-grained scores. (F6)
2. **The strength of a two-reviewer setup comes from "different families".** opus + Codex. Two of the same family have weak value. (F8)
3. **The structure is a fixed dialectical pair = Advocate vs Adversary.** A reusable skeleton that is not improvised each time (devil's advocacy / dialectical inquiry). Task-agnostic + inherently adversarial.
4. **Performance comes from "specialized role fixes" per stage.** Rather than a generic pair, choosing *once, in advance* a role equipped with a dedicated technique for each stage type catches more defects (the core empirical finding of PBR). "A fixed role per stage ≠ improvising every time". (F1/F2)
5. **Blinding is optional.** ON only at stages with high anchoring risk (divergence). Aware of the cost: agreement drops and invariants get missed because context is lost. (F4/F5)
6. **Self-preference is not a constant → a human is the final backstop.** On disagreement the main session does not decide arbitrarily but goes to the user. (F7)
7. **Order bias is real → aggregation is independent of order and labels.** (the converse of the research rejection table)

---

## 1. Two-layer structure (performance without fatigue — this is the core)

- **Skeleton (fixed · permanent):** every review pair is **Advocate (support · strengthen) vs Adversary (attack · counter)**. The adversarial stance is always guaranteed.
- **Specialization (performance · fixed once per stage):** for each known stage, a *dedicated role + technique* is set in advance (§2 table). Once decided, it is only taken out and used afterwards — zero improvisation.
- **Fallback:** only a new artifact that was not set in advance gets the generic Advocate/Adversary.

**Model mapping principle (fixed default):** roles that need ADR/project context → **opus (doc-aware)**. Roles that benefit from a fresh blind judgment → **Codex (blind)**. (The code gate is an exception because both sides are adversarial — see the table note below.)

---

## 2. ★ Per-stage specialized role fix table ★ (not built every time — taken from here)

| Stage | Advocate (support · strengthen) | Adversary (attack · counter) | Blind | Checklist source |
|---|---|---|---|---|
| **PRD / requirements · divergence** | **User lens** (Codex) — advocate the real needs · completeness through use-cases | **Tester lens** (opus) — attack with equivalence/boundary · failure scenarios · **missed alternatives** | **ON** (hide the decision basis → block anchoring) | PBR: User (use-case) + Tester (equivalence-class) |
| **TRD / design** | **Designer lens** (Codex) — soundness of interface · structure · swappability · a simpler option | **Architect-breaker** (opus, doc-aware) — attack invariant violations · coupling · breaking existing ADRs · lifetime | **OFF** (opus = ADR auto-injected / for Codex, explicitly provide the bundle of relevant ADRs) | PBR: Designer + our invariants · seams · capability · swappability |
| **Code (gate)** | **Advocate for correctness · simplicity** — meets the target behavior · a simpler/clearer implementation | **Adversarial breaker** — attack race · lifetime · off-by-one · regression · security | **Asymmetric (experimental)** — Codex = code + contract only (fresh blind breaker) / opus = doc-aware (invariants) | ODC defect types + our invariants (kill · finalize · lock order · epoch · replay) |
| **Document cleanup** | **Cut-advocate** (Codex, blind) — cut more duplicates · dead references · padding | **Load-bearing guardian** (opus, doc-aware) — does a deletion drop an invariant · the "why" · an anti-pattern warning · a cross-reference | Codex = basis hidden (blind) / opus = code · ADR access | Deletion-safety check (preserve load-bearing meaning · cross-references) |
| **(Fallback) unspecified** | did it reach the goal · a better/more concise version · what is missing | what breaks · unwritten assumptions · worst input/race · what is silently violated | Adversary = doc-aware / Advocate = blind by default | (general role questions, no dedicated checklist) |

> **Code gate note:** code review is adversarial in essence, so both roles lean toward attack (opus = doc-aware breaker of invariants / Codex = blind breaker of edges · protocol). "Advocate (it is correct · simple)" is folded into a weak auxiliary check. This asymmetric blind is **our hypothesis with no research evidence** (§4) — an "option" until its effect is measured.

---

## 3. Common rules (all stages)

- **Verdict scale:** `PASS` / `FIX(항목 ≤N)` / `BLOCK`. No scoring. (F6)
- **Handling disagreement:** when Advocate and Adversary split (especially cut vs keep, adopt vs risk), the main session synthesizes but **must not decide arbitrarily → report the dispute to the user**. (F7)
- **Aggregation:** merge the two results with labels (A/B) anonymized and independent of order (blocks order bias).
- **effort:** main session = **xhigh** (the permanent ceiling; ultracode above it is session-only, and is workflow automation rather than higher effort), coder · reviewer = **high** (Codex defaults to medium, high only for fatal concurrency/lifetime changes). The main session, being the unguarded integration node, is given effort rather than inspection.
- **Coder:** opus (complex) / sonnet (simple). The main session must not implement directly (documents are an inline exception).
- **QA:** a measured gate separate from review — build/test + GUI (`scripts/cdp.mjs`). No change.

---

## 4. Honest marking (strength of evidence)

- **Solid (has evidence):** coarse scale + checklist first (F6) · family diversity (F8) · defect coverage of specialized roles (F1/F2) · reduction of anchoring by blinding at divergence (F4) · fixed Advocate/Adversary (devil's advocacy/dialectical inquiry — but it is strategic-decision research, not direct evidence on software review, so directional).
- **Weak/unverified:** **asymmetric blind/doc-aware** at the code stage (zero evidence, our hypothesis) · validity of PBR perspectives at the *code/document stage* (extrapolation) · "specialization is *always* better than generic" (the PBR studies also show "perspectives are not always different" — specialization is a directional advantage, not an absolute good).
- **Do not discard (converse evidence):** order bias is real → keep aggregation order-independent.
- **Rot warning:** the checklists in the specialized-fix table (invariants · ADR · ODC) must be updated when code/ADRs change. Fix the table in place, but check it periodically.

## 5. First application (dogfood)
Run the **CLAUDE.md wording cleanup** with the "document cleanup" row above: the main session writes a cut proposal → Advocate (Codex, blind, "cut more") + Adversary (opus, doc-aware, "protect load-bearing") in parallel → disputed items go to the user for a decision → edit only the approved ones.

## 6. Undecided (needs user ratification)
1. Whether to make the code-stage asymmetric blind "mandatory" or an "experimental option".
2. Fixing the concrete items of each stage's checklist (only sources are specified now).
3. Whether to record this design as an ADR (rejected alternatives = web 3-family consult, generic-only review) + additional reflection in CLAUDE.md.
