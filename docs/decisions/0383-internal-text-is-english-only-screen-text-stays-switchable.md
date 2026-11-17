# ADR-0383: Internal text is English-only; screen text stays switchable

- 상태: 확정 (2026-11-18, basis: user rule 2026-11-18 + S38 PRD/TRD review PASS)
- 관련: ADR-0372 · ADR-0373 · ADR-0342 · `docs/process/S38-english-internal-text/trd.md`

## 맥락
After S35 the app screen text is English-source with Korean selectable (ADR-0372, ADR-0373), but logs, panic/assert messages, internal errors, LLM-facing strings in code, test names and messages, and most crate comments were still Korean (S33 left string literals out by design and did not finish crate comments). The user set the rule 「내가 닿는거라면 모를까 닿지 않는건 영어」 (2026-11-18): what the user reads on the app screen stays switchable, everything else is English. Held by the user: `prompts/engram-help.md`, `prompts/agent-priming.md`, `CLAUDE.md`, `.claude/**`.

## 결정
1. Text that does not reach the app screen is English: comments, logs, panic/assert messages, error strings, LLM-facing strings in code (command `///` summaries, tool descriptions, settings `desc`), test names and messages, script output, docs prose.
2. Error causes are English wherever they are built, including errors the shell builds. They keep travelling as plain strings and the screen shows them inside its translated head (connection band `causeText`, agent list `rowErrorText`, chat error header). In Korean mode the head is Korean and the cause English. No wire or IPC shape changes.
3. `CATALOG_VERSION` is not bumped for a translation-only change of command summaries — the rule is "bump when the declaration changes" and a translated summary is not a declaration change.
4. The placeholder marker (`placeholder_message!` in `crates/engram-dashboard-protocol/src/placeholder.rs`, mirror in `src/api/wsFrame.ts`) is English. The frontend comparison also accepts the old Korean marker until the next `PROTOCOL_VERSION` bump.
5. A Hangul check (`scripts/check-hangul.mjs`, CI `gates`) fails on Hangul in code and config outside an allowlist: `src/i18n/ko.ts` outside comments; `src-tauri/src/ui_text.rs` string literals on `Lang::Ko` lines; frontend test files outside comments, test titles and `console.` lines; recorded `fixtures/*.jsonl`; lines or blocks marked `hangul-ok`; Korean file-name references. Docs are not checked.
6. Kept Korean: the Korean UI tables and the tests that assert them (frontend tests stay pinned to `ko`), intentional Korean fixtures, user quotes in 「」, Korean file names, ADR lint tokens and the ADR index (`docs/decisions/README.md`), Korean HTML reports under `docs/`, the S33 glossary.
7. Each translation unit commits its package's regenerated generated files (ts-rs bindings, `commands.schema.json`, `src-tauri/gen/schemas/capabilities.json`).

## 거부한 대안
- Shell-built screen errors moved into the shell table (`ui_text.rs`) — the text crosses IPC as a `String` and the frontend stores and shows it raw (`commands/discovery.rs` → `protocolClient.ts` `connError` → `connectionBand.ts` `causeText`), so a translated string would keep the old language after a live switch, against ADR-0373 (found by both reviewers).
- Per-error codes with every cause translated at render (Home Assistant style) — a code + params seam through IPC and the frontend for each of ~430 error strings; the user accepted "head translated, cause English" with the plan (2026-11-18).
- No dual-accept of the placeholder marker — the handshake checks only `PROTOCOL_VERSION` and a pre-S38 daemon can keep running across an app update (ADR-0342), so the old Korean marker would show raw.
- A `PROTOCOL_VERSION` bump for the marker — forces a daemon restart on update for a cosmetic case.
- Checking docs too — the held `adr.mjs` writes Korean into the ADR index on every ADR write, Korean HTML reports are user-read, and docs-only pushes do not run CI (path filter, ADR-0232).

## 근거
- User rule and hold list, 2026-11-18 (chat). OSS survey: logs and developer-facing errors stay in one language (Mattermost, OpenStack dropped log translation; Home Assistant, GitLab localize user-facing errors only) — `docs/process/S35-i18next-backend/log-localization-notes-2026-11-08.md`.
- S38 PRD+TRD review, full (Claude + Codex), PASS after three rounds.

## 영향 / 불변식
- New Korean in code or config fails CI `gates` unless it is in the allowlist or marked `hangul-ok`; intentional Korean must carry the marker.
- Screen text that must follow `ui.language` goes through `en.ts`/`ko.ts` or `ui_text.rs`; error causes do not — a new error meant to be read whole on screen needs a table entry looked up at render, not a translated string.
- Remove the old-marker accept in `StructuredTextView.tsx` when `PROTOCOL_VERSION` is bumped next.
- `docs/reference/logging-conventions.md` §Format: log messages are one English line.
