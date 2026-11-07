# ADR-0032: Comment convention — two layers (narrowed inline + load-bearing overview header) + gradual expansion of ADR anchors

- 상태: 확정 (2026-06-24, basis: `docs/research/code-commenting-conventions-research-2026-06-23.md` + opus/Codex adversarial review)
- 관련: CLAUDE.md `## 컨벤션` · `docs/reference/commenting-conventions.md` (living canon) · `docs/research/code-commenting-conventions-research-2026-06-23.md` (basis)

## 맥락
With agent-driven development, the user no longer reads all the code directly, so the user hypothesized that "comments must be more detailed so that looking at just that part gives an instant understanding". We researched whether this is right and how industry commenting practice changes in the agent era (consult/PRD stage — research → options → user decision). Until this decision, engram's comment principles existed only in the CLAUDE.md `## 컨벤션` section; there was no authoritative document or ADR dedicated to comments.

## 결정
**Option B adopted.** We write down a two-layer comment convention:
- **(1) Inline = narrowed to why/intent/invariant/load-bearing** (keeps the existing engram convention).
- **(2) Load-bearing files get an overview header (`//!`), recommended (soft)** — the header summarizes role, responsibilities, key invariants, entry points, and "load-bearing meaning that the signature does not show". It is not a gate but gradual boy-scout work (consistent with "a gradual recommendation (soft), not a hard guardrail" below).
- **(3) Gradually expand `// ADR-NNNN` anchors** — decision anchors on load-bearing code, starting with new and modified code.

The body of the evolving convention (practice rules, examples) goes in `docs/reference/commenting-conventions.md` (living canon), and this ADR keeps only "why B" (no duplicated basis — precedent ADR-0031).

It is a **gradual recommendation (soft)**, not a hard guardrail. Both rot cleanup and header expansion are done boy-scout style (on the side when touching a file) — the research pointed out "documents = soft context; bulk cleanup risks sampling bias".

## 거부한 대안
- **Option A (minimal — keep the convention implicit, only gradual reinforcement)** — cost and rot risk are minimal, but **because the standard is implicit, the next session misses the "understand at a glance" quality of the overview.** Even though engram already follows ~85%, without writing it down the omission keeps recurring in new files.
- **Option C (strong — living-doc tooling with NL-outline-style automatic updates)** — structurally blocks rot and is the most agent-friendly, but **high cost and uncertain (over-engineering at this stage).** It is a high-cost, uncertain area rather than low-risk, long-term, so by our decision criteria it counts as "shell only". It is also uninvestigated whether living-doc tooling is realistic in a Rust/Tauri environment (research §5 gap). If it becomes necessary, a separate prior-art study + ADR.

## 근거
All methodology and empirical basis lives in `docs/research/code-commenting-conventions-research-2026-06-23.md` (single source — the deep-research content is not copy-pasted into the ADR body, precedent ADR-0031). In short:
- **The user hypothesis = conditionally supported.** The naive form ("more detailed overall") is refuted — the redundancy and hallucination backfire of LLM-generated verbose comments is empirically shown. The refined form ("narrow the inline, a separate overview layer for 'understand at a glance', automatic updates, no replacing human review") is supported.
- Good practice = not "reduce comments" but "narrow responsibility" (four layers). It happens to match the existing engram convention by ~85%.

## 영향 / 불변식
- The CLAUDE.md `## 컨벤션` section points to the canon (`docs/reference/commenting-conventions.md`) — the body stays in the canon and is not copied (rot prevention, single source).
- It is a gradual recommendation (soft), not a gate — it does not force a bulk change to the code. Adding overview headers to existing code in bulk is not part of this decision (boy-scout only).
- **Location provisional:** placing the canon in `docs/reference/` is provisional. When the document process (the formal role of the reference slot) is established, the canon's location is readjusted, and this ADR and the CLAUDE.md router link are updated together.
