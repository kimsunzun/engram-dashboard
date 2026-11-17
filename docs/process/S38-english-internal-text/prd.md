# S38 PRD — English-only internal text

- Status: edition 1 (2026-11-18) · approved 2026-11-18 in chat (plan accepted, then 「자율로 쭉 진행해」) · input = `docs/process/S35-i18next-backend/log-localization-notes-2026-11-08.md` · previous pass = `docs/process/S33-english-translation/plan.md` · inventory (2026-11-18, master `36a055da`) = scratch scan, numbers below.

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
  - In docs: user quotes in 「」, fenced code blocks and backtick spans quoted verbatim (S33 rules), ADR lint tokens (`- 상태:` and its vocabulary, `- 관련:`, the five ADR section headings) — parsed by `adr.mjs` of the held `engram-adr` skill.
- Hold (user, 2026-11-18: 「engram help 그냥 놔두고, 너 스킬같은것도 놔두고 … 일단은 보류」): `prompts/engram-help.md`, `prompts/agent-priming.md`, `CLAUDE.md`, everything under `.claude/`.
- Non-goals:
  - Korean file names (309 ADRs, 2 `launch/*.bat`) — every inbound link moves with them; a later script-only batch.
  - A settings screen or new languages.
  - Rewording or restructuring beyond translation (except `docs/reference/logging-conventions.md`, whose rule changes).

## 3. Requirements

- R1. No Hangul remains in tracked files outside the keep and hold lists above.
- R2. Logs are English; the logging convention says so (`docs/reference/logging-conventions.md` — "message = one line in Korean" becomes English).
- R3. Text the shell shows on screen as a whole message (connection band, refusal reasons built in `src-tauri`) moves into the shell's bilingual table and follows `ui.language`. Error causes from crates stay English and appear inside a translated frame (`… : {err}`), as S35 already does.
- R4. LLM-facing text in code is English; the generated command catalog (`commands.schema.json`, ts bindings) is regenerated from it.
- R5. The placeholder marker (`PLACEHOLDER_ERROR_MESSAGE` and its mirror in `src/api/wsFrame.ts`) changes on both sides together; the screen still shows its translated text.
- R6. Tests that match product text change in the same commit as that text; no test is dropped (test-name comparison before/after).
- R7. Translation keeps meaning: negation, conditions, invariants, ADR anchors, identifiers, paths, links, numbers unchanged. Terms follow `docs/process/S33-english-translation/glossary.md` (new terms are added there).
- R8. A check fails CI when Hangul appears outside the keep and hold lists, and can run locally.

## 4. Acceptance criteria

- AC1. The Hangul check passes on the branch head; it fails on a planted Korean comment in a crate and on a planted Korean log line in `src/`.
- AC2. CI three jobs green on the merged SHA (cargo workspace tests, vitest, tsc + gates).
- AC3. A debug run of the app writes an English daemon log and app log (`<data>/logs/`), first lines read.
- AC4. With `ui.language = en` the screen shows no Korean on the connection band and agent list error lines (forced daemon-missing case); with `ko` the same case shows Korean.
- AC5. `engram` command catalog summaries (`commands.schema.json`) are English; `prompts/engram-help.md` and `prompts/agent-priming.md` are byte-identical to master.
- AC6. Test count before/after: no test lost per crate and per vitest file.

## 5. Open decisions — main defaults (user delegated: 「자율로 쭉 진행해」)

- D1. Screen errors (R3): whole messages owned by the shell → shell table (en + ko); crate causes → English inside the translated frame. Alternative (error codes with every cause translated, Home Assistant style) costs a key per error across crates — not now.
- D2. Frontend tests stay pinned to `ko` (they guard the Korean table); only names and messages change.
- D3. Placeholder marker: replace on both sides, no dual-accept — app and daemon of different builds are already refused at handshake.
- D4. Translation commits go into `.git-blame-ignore-revs` (S33 rule), per unit.
- D5. Single merge to master at the end; the user syncs other branches (「그냥 푸쉬하면 내가 알아서 동기화 하라고 전달함」).

## 6. Risks

- Merge conflicts with open branches (preset library, multi-daemon, others): hundreds of files touched. The user syncs them after the push.
- Meaning drift in ~45k translated lines. Mitigation: mechanical token compare on comment-only changes, side-by-side review per unit.
- Tests that match product text across crates (16 cross-crate lines) break if the two sides are translated differently — one integration pass owns them.
