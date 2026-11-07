# ADR-0034: Documentation architecture — a development-flow-centered frame + turning CLAUDE.md into a router

- 상태: 확정 (2026-06-26, basis: documentation-architecture-research)
- 관련: `docs/handbook/documentation-system.md` · `docs/README.md` · CLAUDE.md · `docs/research/documentation-architecture-research-2026-06-24.md` · ADR-0032 (comment convention)

## 맥락
As the kinds of documents in docs/ grew (README · decisions · process/step-log · research · reference), the picture of "how the whole documentation system is woven into one set" became scattered across CLAUDE.md and docs/README. CLAUDE.md was ~12k tokens (far above the 20~80 lines recommended for always-load), so the always-loading cost was large. We needed a big picture that sees documents not as individual types but as **one set of the development flow**.

## 결정
- **Flow-centered frame** — put a frame that maps documents to the development flow (research → ADR → TRD → code) **by stage: outputs / gates / records** in `docs/handbook/documentation-system.md`. The flow *definition* links to CLAUDE.md (not duplicated, SSoT).
- **Four invariants** — SSoT (link, do not copy) · no orphans · soft (documents) + hard (tools) in pairs · lifespan (immutable = ADR · step-log / living = README · reference).
- **CLAUDE.md = router** — detailed explanations and summaries go to canonical pointers (review table → review skill, reference implementation → ADR-0013/14, matrix → ADR-0002/30, the why of the module map → code). Rules, key invariants, build commands, and §5 LLM-first are kept (238 → 206 lines, a conservative slim-down).
- **Handoff is outside the documentation architecture** — `.ccb/` is a one-off session handover tool, not a permanent record node.

## 거부한 대안
- **Doc-type classification only (a table per type)** — the flow of work is not visible, so "where do I put a new document" stays confusing. Switched to flow-centered.
- **Keep the details in CLAUDE.md** — the always-load cost goes up + only one side gets updated, so it rots. Routerizing delegates the details to the canonical documents.
- **Move even the key invariants to reference (aggressive slim-down, option B)** — they are a safety net that prevents regressions, so they stay in always-load (conservative option A). B is reconsidered once anchor coverage is sufficient.
- **Rename step-log → CHANGELOG** — CHANGELOG (Keep a Changelog) is for releases and versions, so its meaning does not match step-log, which is an internal work timeline. Keep step-log.
- **Pin the handoff as a document node** — it is one-off, so a permanent record is unnecessary.

## 근거
- **documentation-architecture-research (2026-06-24)** — Diátaxis · docs-as-code · SSoT · orphan prevention (build failure) · GitLab/MS handbook patterns · the "keep always-load short" recommendation converge across three families (Claude ×2 + Codex) (zero substantive conflicts).

## 영향 / 불변식
- A new document is mapped to a stage of the frame + connected to the discovery chain (README · tracking · code anchors) (no orphans).
- CLAUDE.md stays a router — do not grow the details back into its body (canonical pointers).
- **Not applied (future):** further slimming of CLAUDE.md (option B) · document CI (orphan/link/freshness as hard gates).
