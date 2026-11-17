# S38 PRD — English-only internal text

- Status: edition 2 (2026-11-18; edition 1 changed after the PRD+TRD review: R3, R5, R6, R8, AC1, AC4, keep list, §5) · approved 2026-11-18 in chat (plan accepted, then 「자율로 쭉 진행해」) · input = `docs/process/S35-i18next-backend/log-localization-notes-2026-11-08.md` · previous pass = `docs/process/S33-english-translation/plan.md` · inventory (2026-11-18, master `36a055da`) = scratch scan, numbers below.

## 1. What · why

Every piece of text that does not reach the user on the app screen becomes English: logs, panic/assert messages, internal error strings, LLM-facing strings embedded in code, test names and messages, code comments, script output, and the docs prose that S33 left behind. Text that reaches the screen stays switchable through the S35 tables (English source, Korean translation).

Why: the user's rule — 「내가 닿는거라면 모를까 닿지 않는건 영어」 (2026-11-18). Logs and developer-facing errors stay in one language in the surveyed OSS (Mattermost, OpenStack dropped log translation; Home Assistant, GitLab localize only user-facing errors). S33 translated part of the comments and docs but left string literals out by design and missed most crate comments.

## 2. Scope / non-goals

Inventory (lines containing Hangul, outside hold):

| Kind | Lines |
|---|---:|
| Code comments (S33 left ~24.5k, 19.6k in crates) | 24,555 |
| Logs (`tracing`, `println!`/`eprintln!`, `console.*`) + script output | 1,008 |
| panic/expect/assert in product code | 136 |
| Error strings + `Display` impls | 433 |
| LLM-facing text in code (`#[tool(description)]`, command `summary`/`description`, settings `desc`, `///` inside `declare_commands!`) | 432 |
| Test names · assertion messages · test data | 14,381 |
| Tests matching product Korean text (change with the product) | 212 |
| Other strings (Cargo `description`, capability `description`, misc) | ~330 |
| Docs prose (`docs/**`, long + short lines) | ~11,400 |

- In scope: all of the above, in every tracked file outside the hold list.
- Keep Korean (intended):
  - `src/i18n/ko.ts` and the Korean column of `src-tauri/src/ui_text.rs`.
  - Frontend tests that assert Korean UI text (vitest runs in `ko` on purpose, `src/i18n/vitest.setup.ts`) — their assertion values stay; their test names and messages become English.
  - Fixtures that exercise Korean input or rendering (IME, Hangul width, bilingual word filters), recorded transcripts (`fixtures/*.jsonl`).
  - In docs: user quotes in 「」, fenced code blocks and backtick spans quoted verbatim (S33 rules), ADR lint tokens (`- 상태:` and its vocabulary, `- 관련:`, the five ADR section headings) and the ADR index in `docs/decisions/README.md` — parsed and written by `adr.mjs` of the held `engram-adr` skill; the S33 glossary (`docs/process/S33-english-translation/glossary.md`); Korean HTML reports under `docs/` (the user reads them).
  - Rust tests that assert the Korean column of `ui_text.rs`.
- Hold (user, 2026-11-18: 「engram help 그냥 놔두고, 너 스킬같은것도 놔두고 … 일단은 보류」): `prompts/engram-help.md`, `prompts/agent-priming.md`, `CLAUDE.md`, everything under `.claude/`.
- Non-goals:
  - Korean file names (309 ADRs, 2 `launch/*.bat`) — every inbound link moves with them; a later script-only batch.
  - A settings screen or new languages.
  - Rewording or restructuring beyond translation (except `docs/reference/logging-conventions.md`, whose rule changes).

## 3. Requirements

- R1. No Hangul remains in tracked files outside the keep and hold lists above.
- R2. Logs are English; the logging convention says so (`docs/reference/logging-conventions.md` — "message = one line in Korean" becomes English).
- R3. Error text is English wherever it is built, including errors the shell builds and the frontend shows (connection band, agent list error lines, chat error body). The screen keeps showing them inside its translated head, as S35 already does — in Korean mode the head is Korean and the cause English (the default accepted with the plan, 2026-11-18).
- R4. LLM-facing text in code is English; the generated command catalog (`commands.schema.json`, ts bindings) is regenerated from it.
- R5. The placeholder marker (`PLACEHOLDER_ERROR_MESSAGE` and its mirror in `src/api/wsFrame.ts`) changes on both sides together; the screen still shows its translated text, also for the old Korean marker from a pre-S38 daemon still running.
- R6. Tests that match product text change in the same commit as that text; no test is dropped (test count per test file before/after).
- R7. Translation keeps meaning: negation, conditions, invariants, ADR anchors, identifiers, paths, links, numbers unchanged. Terms follow `docs/process/S33-english-translation/glossary.md` (new terms are added there).
- R8. A check fails CI when Hangul appears in code or config outside the keep list, and can run locally. Docs are not checked (translated once here).

## 4. Acceptance criteria

- AC1. The Hangul check passes on the branch head; it fails on a planted Korean comment in a crate and on a planted Korean log line in `src/`.
- AC2. CI three jobs green on the merged SHA (cargo workspace tests, vitest, tsc + gates).
- AC3. A debug run of the app writes an English daemon log and app log (`<data>/logs/`), first lines read.
- AC4. Forced daemon-missing case on the connection band: with `ui.language = en` no Korean in our own text; with `ko` a Korean head and an English cause; after a live switch no old-language text remains (OS-localized `io::Error` text excluded).
- AC5. `engram` command catalog summaries (`commands.schema.json`) are English; `prompts/engram-help.md` and `prompts/agent-priming.md` are byte-identical to master.
- AC6. Test count before/after: no test lost per crate and per vitest file.

## 5. Open decisions — main defaults (user delegated: 「자율로 쭉 진행해」)

Decisions D1–D8 are in the TRD §7. The ones the user sees:
- Screen errors: English cause inside the translated head (TRD D1).
- Korean HTML reports under `docs/` stay Korean (TRD D7) — asked 2026-11-18, pending.
- Build script output (`launch/*.bat` → `scripts/build-release.ps1`) becomes English like other developer tools — asked 2026-11-18, pending.
- Single merge to master at the end; the user syncs other branches (「그냥 푸쉬하면 내가 알아서 동기화 하라고 전달함」).

## 6. Risks

- Merge conflicts with open branches (preset library, multi-daemon, others): hundreds of files touched. The user syncs them after the push.
- Meaning drift in ~45k translated lines. Mitigation: mechanical token compare on comment-only changes, side-by-side review per unit.
- Tests that match product text across packages (16 cross-crate lines) break if the two sides are translated differently — the dependent package's unit starts after the producer's English has landed (TRD §5 waves).
