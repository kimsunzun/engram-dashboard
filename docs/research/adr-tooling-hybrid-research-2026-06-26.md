# ADR automation: industry practice for the "script + a spoonful of judgment" hybrid (research)

- 상태: done (2026-06-26) · depth: research skill **medium**
- Method: Claude (Sonnet) 2 branches (supersede / hybrid architecture) + Codex 1, **BLIND independent collection** → opus cross-check and adversarial verification. (The numbering and index branches were collected in the 6-25 round.)
- Purpose: redesign the engram `adr` skill as a "deterministic script + LLM judgment" hybrid. Look for answers to the partial-discard, lint and numbering holes that `/review trd full` caught.
- Confidence legend: certain / likely / uncertain

## Core conclusions

1. **The machine-vs-judgment boundary is consistent across the industry** (certain). Script = clerical work (numbering, file creation, templates, index regeneration, laying down supersede links, format lint). Human/LLM = reasoning work (body prose, deciding full vs partial discard, interpreting status, the "is this ADR-worthy?" trigger). The same from 2015 adr-tools to 2026 adr-kit.
2. **adr-tools (npryce) is the standard thin hybrid reference** (certain). A few hundred lines of bash. `adr new -s N` = supersede, automatic in both directions; `-l "N:Amends:Amended by"` = an arbitrary two-way relation.
3. **Partial discard (partial supersede) has no standard tool support** (certain). For a full discard the default is to change the status to `superseded`. For a partial one: ① the adr-tools `-l Amends/Amended by` two-way link convention ② a dated note in the body (living-doc, joelparkerhenderson) — **both keep the old ADR's status** and only add a link/note.
   - → **The answer to our hole:** partial discard = keep the status + two-way `Amends`/`Amended by` links (+ a clause qualifier). Do not overwrite the whole status with 'discarded'.
4. **Two-way links are the core, and automating them is the value** (certain). Done by hand, one side gets forgotten → the script laying down both sides is the reason adr-tools exists (same as our review finding).
5. **Index/TOC is a separate concern, of the regenerate kind** (certain). adr-log and pyadr `generate-toc` scan the bodies and regenerate the index. adr-tools itself does not build an index.
   - → **Regenerating** the index from the bodies blocks "index↔body drift" and "title drift" at the source (body = single source). Consistent with our invariant "the status source of truth = the body header".
6. **Status vocabulary standard** = proposed/accepted/deprecated/superseded (+rejected) (certain). superseded = replaced by a better decision / deprecated = dropped with no replacement.
7. **The pattern of putting an LLM into ADRs appeared in 2024–2026, but it is not yet built into the mainstream** (likely). Verified patterns = Equal Experts "kernel of truth" (a human supplies the core → LLM draft → AI critique → human fact-check and approval), Codex CLI governance, rvdbreemen/adr-kit (MCP). Common warning: **the LLM hallucinates Context/Consequences** (invents APIs that do not exist) → human verification is required.
8. **CI/hook lint is still forming and is not mainstream** (certain). The archgate, adr-kit and Codex CLI patterns are all new in 2025–2026.

## Cross-check table (Claude ↔ Codex)

| Claim | Claude | Codex | Decision |
|---|---|---|---|
| The machine/judgment boundary is consistent | ✓ | ✓ | converged · certain |
| adr-tools = thin reference | ✓ | ✓ | converged · certain |
| Partial discard is not supported by any standard, and Amends links are the only practical option | ✓ | ✓ | converged · certain |
| Two-way automation exists only in adr-tools | ✓ | ✓ | converged · certain |
| index regeneration = a separate tool | ✓ | ✓ | converged · certain |
| LLM pattern emerging · not mainstream | ✓ | ✓ | converged · likely |
| EventCatalog structured `amends` field | ✓ (unconfirmed) | — | one side only · likely |
| dotnet-adr exists | confirmed in round 1 | could not confirm | weak · uncertain |

## Design implications (engram adr hybrid)

**`scripts/adr.mjs` (deterministic):**
- `new` — numbering (max+1 over the body files, rescanned right before writing) + template file creation + index regeneration
- `supersede` — full: old ADR status→`폐기 (Superseded by N)` + new ADR `Supersedes M` (two-way) / **partial: keep the old status + two-way link `Amended by N` ↔ new `Amends M`**
- `index` — scan the body H1 and status and **regenerate** the README index (blocks drift at the source)
- `lint` — compare the status *vocabulary only* / duplicate and missing numbers / supersede two-way consistency / orphan code anchors `// ADR-` (code paths only, `docs/` excluded)

**`adr` skill (thin, judgment):** take the user's decision → **decide full vs partial discard** → tidy the body prose (kernel→template) → call the script → report the result. Rejected alternatives and rationale come from the user (no hallucination · fact-check).

→ This design resolves the 6 holes `/review` caught (partial discard · lint false positives · numbering · rg noise · title drift · status source of truth) **structurally**.

## Gaps and limits
- **Unanimity caution:** both families anchor strongly on adr-tools — it is the oldest and best documented, so it may be over-sampled. But "no standard for partial discard" is solid, since many independent tools confirm it.
- The schema of EventCatalog's structured `amends` field was not checked (room for further investigation).
- Many of the effect figures in the LLM-ADR studies are not disclosed in the abstracts (quantitatively uncertain).

## Sources
- adr-tools(npryce): https://github.com/npryce/adr-tools (`src/adr-new` source checked directly)
- adr-log: https://github.com/adr/adr-log · pyadr: https://github.com/opinionated-digital-center/pyadr
- MADR: https://adr.github.io/madr/ · log4brains: https://github.com/thomvaill/log4brains
- joelparkerhenderson ADR collection: https://github.com/joelparkerhenderson/architecture-decision-record
- Backstage ADR: https://backstage.io/docs/architecture-decisions/ · EventCatalog: https://www.eventcatalog.dev/blog/introducing-adrs
- archgate/cli: https://github.com/archgate/cli · rvdbreemen/adr-kit: https://github.com/rvdbreemen/adr-kit · kschlt/adr-kit
- Codex CLI ADR governance: https://codex.danielvaughan.com/2026/04/28/codex-cli-architecture-decision-records-adr-automated-governance/
- Equal Experts (generative AI + ADR): https://www.equalexperts.com/blog/our-thinking/accelerating-architectural-decision-records-adrs-with-generative-ai/
- Papers: arXiv 2403.01709 (Can LLMs Generate ADDs) · arXiv 2504.08207 (DRAFT-ing ADDs) · AWS Prescriptive Guidance (ADR process)
