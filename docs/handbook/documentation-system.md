# Engram Documentation System (documentation-system)

- Status: **living** (git is the history — no change-history section inside the file) · Update: whoever touches it, boy-scout
- This document = a map of "**where and how the engram documents fit into the development flow**". Before doing document or design work, read `../README.md` (the hub) and this document.
- Basis: `../research/documentation-architecture-research-2026-06-24.md` (industry practice survey).

## 1. The development flow is the spine

The **definition** of the whole development flow is in `../../CLAUDE.md` 「개발 순서」 and the `/engram-implement` skill (PRD→TRD→review→coding→code review→QA→commit). **It is not rewritten here (SSoT).** This document is a *layer that puts documents on top of* that flow.

## 2. By stage — output documents · gates · records

| Stage | Output documents | Verification gate | Record |
|---|---|---|---|
| **Research** | `research/` report (+`status:`) | cross-verification inside research | commit message |
| **Decision (PRD)** | PRD (`process/S<n>-<슬러그>/prd.md`) + **ADR** (why + rejected alternatives) | user approval | commit message + `/engram-adr` |
| **Design (TRD)** | TRD (`process/S<n>-<슬러그>/trd.md`) + design ADR | `/engram-review` (PRD+TRD) → user confirmation | commit message + `/engram-adr` |
| **Implementation** | code + tests + `// ADR-` anchors | `/engram-review code` → `/engram-qa` | commit message |

- **An ADR is not dedicated to one stage** — write one whenever a major decision is made (in both PRD and TRD).
- The **commit message** (`S<n>:` title) records *when/what* at every stage, review and QA results included. Timeline = git log + each work folder's `README.md` (one status line per TRD). `process/step-log.md` is frozen.
- **reference** (`reference/`) holds current behavior — a sentence a code change makes wrong is fixed in the same commit as that code.
- **Handoff** (`/engram-handoff`) is a one-off session handover tool — **outside the document architecture, leaves no record.**

## 3. Invariants (keep only these and the rest stays flexible)

- **SSoT** — link, do not copy. Duplication = stale cost.
- **No orphans** — a new document must connect to at least one link in the discovery chain (`README.md` · `tracking.md` · code anchor).
- **soft + hard** — pair documents (intent and reason, soft) with tools (lint/test/CI, hard). Pin invariants down in the hard form too.
- **Lifetime** — immutable and cumulative (ADR) vs living (README · reference, git is the history). Research becomes a snapshot after approval and freezes.
  - Step docs (PRD · TRD) are workspaces. At the step's last commit only the TRD lines that differ from the code (names · values · locations) are fixed and a 「구현됨」 line is added — then the TRD is frozen for good.
  - Reference md is edited in the same commit as the code change that makes a sentence wrong.
  - Basis: `../research/design-doc-lifecycle-after-implementation-2026-11-13.md` · `../research/living-reference-dual-audience-structure-2026-11-14.md`.

## 4. Automation map (what is automatic, what is manual)

| Node | Tool | Status |
|---|---|---|
| Research | `/engram-research` | ✅ |
| ADR (numbering · two-way links · index · lint) | `/engram-adr` (global skill — scripts built in, binding = `.claude/skill-bindings/adr.md`) | ✅ |
| Stage gate | `/engram-review` (stage argument) · `/engram-qa` | ✅ |
| Timeline (commit titles · work-folder `README.md` status lines) | (manual) | manual |
| Document lint/CI (orphans · links · freshness) | — | later (hard reinforcement) |

The skills (research/implement/review/qa/adr) are already **placed at the output and gate spots** of the flow. The only blank is the timeline (manual).

## 5. Where to put things / what documents exist

Type, placement and "where does new content go" have **`../README.md` as the canonical source** — do not copy-paste them here (SSoT). This document covers only the *flow mapping*; for placement rules see the hub.
