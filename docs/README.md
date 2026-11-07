# Engram Dashboard — Docs Index

A native dashboard that manages Claude agents on one screen.
Tauri v2 + React + xterm.js (frontend) / Rust + portable-pty (backend).

> **Read this file first before doing any documentation work.** It lists which documents exist and where new content goes.

## If you are new — reading order

This is **jump-style consumption**, not a read-through — narrow down to the area you are touching and read only what you need.

> **If you need the big picture first** → `reference/architecture.md` (modules · connection map · runtime invariants).

1. `../CLAUDE.md` — development order · branch/commit · design principles · modules (always loaded).
2. This `docs/README.md` — the hub for status and structure.
3. Narrow down the area you are touching — the `decisions/README.md` index + the `// ADR-NNNN` anchors in code (`rg "ADR-"`).
4. Why → `decisions/` — ADRs (decision + rejected alternatives).
5. When/what → `process/step-log.md` — the timeline.
6. Evidence → `research/` — prior research.
7. Canon/conventions → `reference/` — the code-synchronized canon.

## Document kinds

| Folder/file | What it is |
|---|---|
| `process/SN-name/` | "How we have built it" — **a folder is a step** (self-evident from the name alone). The real list is `ls docs/process/`, the flow is `step-log.md`. *Do not copy the step list here by hand — a copy drifts (rot).* |
| `process/step-log.md` | ★ Timeline — when, what, how. **The single source for the step flow.** |
| `decisions/` | "Why we decided this way" — decisions + rejected alternatives (ADRs). Accumulates permanently. |
| `tracking.md` | Open deferred items (T-). The "re-introduction point" is the trigger. When one is resolved, remove it, and if anything points to its number, move it to `tracking-archive.md` as one line. |
| `tracking-archive.md` | Only those items resolved or closed in `tracking.md` that other documents or code point to by number, one line each. Full text = git history. |
| `todo.md` | **What is not done yet** — the **index** of the to-do list. One line per item (item · status · pointer); the detail lives in `todo/<이름>.md`. Four states = ready to start · waiting on trigger · owned · confirmed only. Delete an item when it is resolved. ★Transitional (user decision 2026-09-30 — `tracking.md` T-37): it is to be discarded — pile new items up in `tracking.md` as T-, not here★ |
| `research/` | Prior research before a step starts (investigation, comparison, open questions). |
| `refactoring/` | Refactoring material — **measured boundaries** for splitting/merging, structural-debt measurements, plans and order. **Split out of `research/` so it can be skimmed in one place** (user remark 2026-08-23: it gets buried in the research pile and cannot be found). Browsing surface and boundary rules = that folder's `README.md`. |
| `reference/` | Code-synchronized canon (evolving canon — edited in place). The real list = `ls docs/reference/` (comment, logging and debugging conventions, etc.). *Do not enumerate by hand (rot).* |
| `reference/structure/` | **Detailed structure** by topic — reusable parts, **one file = one topic** (briefings · decks · TRDs point here and do not copy it). ★Not a bird's-eye view★ — the whole picture is held by `reference/architecture.md`. The real list = `ls docs/reference/structure/`. |
| `handbook/` | **System manuals** for documents and process. `documentation-system.md` (development flow ↔ document mapping · invariants · automation map) · `branch-commit.md` (getting `S<n>` at topic start · integration procedure). The big picture of "where and why the documents are placed". |

## Where new content goes

- You made a design **decision** (+ discarded alternatives) → an ADR in `decisions/` (read the related ADRs before working)
- Something to handle **later**, not now, or an open question → `tracking.md` (T-/D-)
- **What was done, when** → `step-log.md`
- **Research** before a step starts → `research/`
- **Refactoring** boundary measurements · structural-debt measurements · plans → `refactoring/` (not a decision — if you decide, record it in `decisions/` and link from there. 「나중에 다룬다」 (handle later) goes to `tracking.md` as a T-)
- **Canon (evolving canon)** synchronized with code → `reference/` (edited in place, not an ADR — if an ADR is the *why*, canon is the *practice convention*)
- Starting a new feature **design** → a new folder `process/SN-name/` (read the related `research/` and ADRs first)

**★ No orphans:** when you create a new document, link it from at least one of the paths above, `tracking.md`, or a code anchor (`// see …`). If you do not, the next session cannot find it.

## Progress status

Do not maintain this in duplicate in this file (to prevent rot). Single sources:
- **When/what (timeline):** `process/step-log.md` ★ always latest
- **Why (decisions, rejected alternatives):** `decisions/`
- **Deferred (T-/D-):** `tracking.md`
