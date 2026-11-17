# S38 TRD — English-only internal text

- Status: edition 1 · 2026-11-18 · canonical ADR = the S38 ADR written after review (decisions D1–D6 below) · PRD = `prd.md`.

## 1. Scope and PRD relation

Implements PRD R1–R8. Keep / hold lists are the PRD's (§2). Reference files changed: `docs/reference/logging-conventions.md` (rule rewrite), `docs/reference/structure/i18n.md` (shell table gains the moved screen errors), lib.rs head comments of each crate (translated).

## 2. Translation rules (every unit)

- Translate Korean in: comments (incl. `///`/`//!` doc comments), log messages, panic/expect/assert/unreachable messages, error strings and `Display` impls, LLM-facing descriptions, test names, assertion messages, test data that is not intentional Korean, script output, YAML/TOML/PS1/bat comments, Cargo/capability `description`.
- Keep verbatim: identifiers, paths, ADR numbers, links, numbers, backtick spans, user quotes in 「」, format placeholders (`{e}`, `{0}`, `%s`), structured log fields (`agent = %id`). Do not reword English that is already there.
- Meaning first: negation, conditions, invariants, ★emphasis markers★ and `**bold**` survive. Terms follow `docs/process/S33-english-translation/glossary.md`; a unit that needs a new term adds a row (U9 merges glossary rows).
- Logs follow the new convention (`logging-conventions.md` §Format after U9): one English line saying what happened; identifiers stay structured fields.
- Keep lists: `src/i18n/ko.ts`, Korean column of `src-tauri/src/ui_text.rs`, assertion values of frontend tests that check Korean UI text, intentional Korean fixtures (IME / width / bilingual filters), `fixtures/*.jsonl` recorded transcripts.

## 3. Boundaries and data shape

- **Screen errors (D1).** Whole messages the shell builds and shows (`DiscoveryError` and helpers in `src-tauri/src/discovery/mod.rs`, `HandshakeError` in `daemon_client/connection.rs`, refusal texts in `daemon_client/link/route.rs`, `daemon_client/mod.rs` and `link/errors.rs` texts that reach the agent list) get entries in `src-tauri/src/ui_text.rs` (en + ko) and are rendered with the current `ui.language` where they become a user-visible string. If an error type is also logged or returned to an LLM, its `Display` is English and the screen path looks up the table by variant — the error value does not carry translated text (ADR-0373: no kept translated text). Crate-side causes (`profile.rs` store refusals, codex `headline()`, `commands/agent.rs` parse failures) become English and appear inside the existing translated frame.
- **Command catalog (R4).** `///` lines inside `declare_commands!` are the command summaries. After translation regenerate `commands.schema.json` and the ts bindings with the project's generator (the sync gate in CI names it). `CATALOG_VERSION` is bumped only if its rule says summary text is part of the versioned shape — the unit reads the rule next to the constant and reports.
- **Placeholder marker (D3).** `placeholder_message!` in `crates/engram-dashboard-protocol/src/placeholder.rs` and its byte mirror in `src/api/wsFrame.ts` change together to an English marker; the identity test stays.
- **Tests matching product text.** A test owned by a unit that matches text owned by another unit is left as is by that unit and listed in its report; U9 fixes all of them after the units land, against the landed English.

## 4. Hangul check (R8, D5)

- `scripts/check-hangul.mjs` (node, no deps): scans `git ls-files`, fails on Hangul (U+AC00–D7A3, U+3130–318F) outside an allowlist kept in the script:
  - paths: `src/i18n/ko.ts`, `src-tauri/src/ui_text.rs`, `**/fixtures/*.jsonl`, `prompts/**`, `CLAUDE.md`, `.claude/**`, Korean file names (path text itself);
  - frontend test files (`src/**/*.test.ts(x)`): Hangul allowed only inside string/template literals (assertion values), not in comments or test titles;
  - an inline marker for intentional Korean fixtures elsewhere: a trailing `// hangul-ok: <reason>` (or `# hangul-ok:`) on the line;
  - `docs/**` md/html: Hangul allowed only inside 「」, backtick spans, fenced code blocks, link targets, and the ADR lint tokens (`- 상태:` line, `- 관련:` label, the five ADR section headings).
- Wired as a step in the CI `gates` job and runnable as `node scripts/check-hangul.mjs`. Docs-only pushes do not run CI (path filter) — accepted; the next code push or `workflow_dispatch` catches docs.

## 5. Implementation units

Units do not share files. Each runs in its own git worktree from the same base commit; main applies each unit's diff onto the topic branch and commits it as one commit (`S38: chore(i18n-…): …`). Each unit ends with: its crate/package builds, its own tests green, a test-name list before/after with no loss, a comment-only token check (for files whose changes are comment-only: the comment-stripped token stream is unchanged).

| Unit | Files | ~Lines |
|---|---|---:|
| U1 | crates `protocol`, `base`, `command`, `platform`, `transport`, `messaging`, `cli` (incl. placeholder marker + `wsFrame.ts` mirror) | 5.7k |
| U2 | `crates/engram-dashboard-agent/src/backend/codex/**` | 4.3k |
| U3 | rest of `crates/engram-dashboard-agent/src/**` (incl. `declare_commands!` summaries) | ~6k |
| U4 | `crates/engram-dashboard-agent/tests/**` + `build.rs` | ~4k |
| U5 | `crates/engram-dashboard-daemon/src/**` | ~5k |
| U6 | `crates/engram-dashboard-daemon/tests/**` | ~4k |
| U7 | `src-tauri/**` except `ui_text.rs` Korean column (incl. D1 table entries) | 6.3k |
| U8 | `src/**` except `ko.ts` (+ `wsFrame.ts` left to U1), `index.html`, root config | 5.9k |
| U8s | `scripts/**`, `.github/**`, `launch/**`, root files; `check-hangul.mjs` + CI step | 1.0k |
| D1–D3 | docs prose: D1 `docs/process/step-log.md` + `docs/tracking.md` + `docs/todo/**`; D2 `docs/process/**` rest; D3 `docs/research/**`, `docs/decisions/**` prose, `docs/reference/**` (not `logging-conventions.md`), `docs/handbook/**`, other `docs/**` | 11.4k |
| U9 | integration: regenerate bindings/schema, cross-unit test matches, `logging-conventions.md` rewrite, glossary rows merge, `.git-blame-ignore-revs`, run `check-hangul.mjs` to zero, ADR | — |

Order: U1–U8s and D1–D3 in parallel waves (at most 4 concurrent worktrees for build cost); U9 last. Generated files (`bindings/*.ts`, `commands.schema.json`) are touched only by U9's regeneration.

## 6. Test plan

- Per unit: build + that package's tests; test-name before/after; comment-only token compare.
- Review per unit: `/engram-review code` — side-by-side meaning check of source vs translation (negation, conditions, invariants) plus a check that no non-comment code changed beyond strings.
- QA after U9 (qa binding table: Rust source + frontend rows → full regression + GUI):
  - CI three jobs locally (cargo workspace, vitest, tsc + gates incl. the new Hangul step).
  - AC1: `check-hangul.mjs` clean; planted Korean comment in a crate and planted Korean `console.warn` in `src/` both fail it.
  - AC3: debug run, read the first lines of the daemon and app logs.
  - AC4: GUI — daemon-missing case on the connection band in `en` (no Korean) and `ko` (Korean).
  - AC5: `git diff master -- prompts/` empty; schema summaries English.
  - AC6: test counts per crate / vitest file vs master.

## 7. Decisions

- D1 screen errors: shell table for shell-owned whole messages; English causes inside the translated frame. Rejected: per-error codes translated everywhere (cost: a key per error across crates).
- D2 frontend tests stay in `ko`.
- D3 placeholder marker replaced on both sides, no dual-accept (mixed builds are refused at handshake).
- D4 `.git-blame-ignore-revs` gets each unit commit.
- D5 Hangul check with allowlist + inline `hangul-ok:` marker, CI `gates` step.
- D6 Korean file names and ADR lint tokens stay (held `adr.mjs` parses them; renames are a later batch).

## 8. Risks

- Parallel worktrees: one full cargo build each — at most 4 at once.
- A unit translating text another unit's test matches: handled only in U9; units must not "fix" the other side.
- Meaning drift: review per unit; mechanical token check for comment-only files.

## Review record

(none yet)
