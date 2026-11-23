# S25 — preset library

Agent presets: a library folder of instructions, skills, settings and preset files, read, checked and resolved by the `asset-library` crate and plugged into the daemon as the manager's launch-material source. Every claude and codex agent launches with the built-in preset 「기본」 (`builtin\`); library presets, the library commands and the screen come in later steps.

- Canonical ADRs: ADR-0311 (crate edges) · ADR-0327 · ADR-0328 · ADR-0337 · ADR-0348 · ADR-0360 · ADR-0361 · ADR-0368 · ADR-0369 · ADR-0370 · ADR-0371 (index: `docs/decisions/README.md`).
- Tracking: T-104 · T-105 · T-106 · T-107 · T-109 · T-110 · T-111.

| File | What | Status |
|---|---|---|
| `prd.md` | PRD | see its status line |
| `trd.md` | parent TRD — one item per step; step status lives in the items | in progress |
| `trd-create-body.md` | step 20 — create and start as one body | landed |
| `trd-codex-translation.md` | step 6 — codex launch materials | landed |
| `trd-asset-library.md` | step 7 — the asset-library crate | Implemented |
| `trd-daemon-wiring.md` | step 8 — plug the real launch materials into the daemon | Implemented |
| `trd-library-edit.md` | step 9a — root, edits, create, relink | Implemented |
| `trd-library-commands.md` | step 10a — library, preset and agent-preset commands, help, priming | Implemented |
| `trd-old-priming-removal.md` | remove the old priming path and the settings fragment (closes `trd.md` §2-4 item 2 — ADR-0370) | Implemented |
| `trd-launch-skip-missing.md` | T-112 — a launch skips what is absent and blocks on what exists but cannot be read (ADR-0374 · ADR-0387) | Implemented |
| `trd-library-mounts.md` | step 21 — one virtual root of named mounts (ADR-0396 · ADR-0398) | Implemented |
| `trd-file-operations.md` | step 9b — file operations, per-mount trash, emptying by direct removal (ADR-0408 · ADR-0409) | reviewed |
| `briefing.html` · `trd-asset-library.html` | Korean explainers (made on request; the md files are canonical) | — |
