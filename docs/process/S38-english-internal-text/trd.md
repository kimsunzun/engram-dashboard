# S38 TRD — English-only internal text

- Status: edition 2.1 · 2026-11-18 · canonical ADR = the S38 ADR written after review (decisions D1–D8 below) · PRD = `prd.md`.
- Edition 2 replaces edition 1 after the PRD+TRD review (full): screen errors become English causes inside the translated head (no shell table entries); the placeholder accepts the old marker too; the Hangul check covers code and config only; units follow packages.

## 1. Scope and PRD relation

Implements PRD R1–R8. Keep / hold lists are the PRD's (§2). Reference files changed: `docs/reference/logging-conventions.md` (rule rewrite), `docs/reference/structure/i18n.md` (the line on thrown error text: causes are English), lib.rs head comments of each crate (translated).

## 2. Translation rules (every unit)

- Translate Korean in: comments (incl. `///`/`//!` doc comments), log messages, panic/expect/assert/unreachable messages, error strings and `Display` impls, LLM-facing descriptions, test names, assertion messages, test data that is not intentional Korean, script output, YAML/TOML/PS1/bat comments, Cargo/capability `description`.
- Translation only. Do not restructure, delete or "improve" comments or code (this overrides the code-conventions binding rule that aligns comments of a touched function). Already-English text stays as is.
- Keep verbatim: identifiers, paths, ADR numbers, links, numbers, backtick spans, user quotes in 「」, format placeholders (`{e}`, `{0}`, `%s`), structured log fields (`agent = %id`).
- Meaning first: negation, conditions, invariants, ★emphasis markers★ and `**bold**` survive. Terms follow `docs/process/S33-english-translation/glossary.md`. Units do not edit the glossary; they report new terms and U9 adds the rows.
- Logs: one English line saying what happened; identifiers stay structured fields (the new `logging-conventions.md` §Format, written by U9).
- Intentional Korean that stays (PRD keep list) gets a marker where the Hangul check needs one (§4): trailing `// hangul-ok` (or `# hangul-ok`) on the line, or `hangul-ok-begin` / `hangul-ok-end` comment lines around a multi-line block.

## 3. Boundaries

- **Screen errors (D1).** Every error string becomes English, including the ones the shell builds (`src-tauri/src/discovery/mod.rs` `DiscoveryError` and helpers, `daemon_client/connection.rs` `HandshakeError`, `daemon_client/link/route.rs` refusal texts, `daemon_client/mod.rs` constants, `link/errors.rs`, `commands/agent.rs`) and crate causes (`profile.rs` refusals, codex `headline()` and queue-full text). They keep travelling as plain strings; the frontend keeps showing them inside its translated head (`connectionBand.ts` `causeText`, `AgentList` `rowErrorText`, the chat error header). No wire or IPC shape changes. In Korean mode the head is Korean and the cause is English — the default the user accepted with the plan (2026-11-18). The frontend's own translated `connection.refusedCause` / `versionCause` stay the table entries for refusals.
- **Command catalog (D2).** `///` summaries inside `declare_commands!` become English. `CATALOG_VERSION` is not bumped: the rule is "bump when the declaration changes" (`crates/engram-dashboard-command/src/macros.rs`), the value is diagnostic only, and a translation-only summary change is not a declaration change. The unit owning the declarations commits the regenerated `commands.schema.json` and bindings.
- **Generated bindings (D3).** Each unit runs its package's tests or build, which regenerate that package's generated files (`crates/engram-dashboard-agent/bindings`, `crates/engram-dashboard-protocol/bindings`, `src-tauri/bindings`, `commands.schema.json`, `src-tauri/gen/schemas/capabilities.json` from capability `description`); the unit includes the regenerated files in its diff so the CI sync gate stays green.
- **Placeholder marker (D4).** `placeholder_message!` in `crates/engram-dashboard-protocol/src/placeholder.rs` and its byte mirror in `src/api/wsFrame.ts` change to an English marker. The frontend comparison (`StructuredTextView.tsx` `errorMessageText`) also accepts the old Korean marker (one `hangul-ok` line, removed at the next `PROTOCOL_VERSION` bump): the handshake checks only `PROTOCOL_VERSION`, and a pre-S38 daemon can keep running across an app update (ADR-0342), so an old daemon would otherwise show the raw Korean marker.
- **Tests that match product text.** Product and tests of one package are in one unit, so the unit changes both together (PRD R6). A test in another package that matches the producer's text: if that package's unit runs in a later wave, the producer unit edits that test line too (same commit as the text); if it runs in the same wave, the producer reports it and main commits the two units together as one commit with the line fixed.

## 4. Hangul check (D5)

- `scripts/check-hangul.mjs` (node, no deps), run as `node scripts/check-hangul.mjs`, step in the CI `gates` job with `shell: bash`.
- Files: `git ls-files -z`, text files only (skip files with a NUL byte or a binary extension), and only code and config — `docs/**`, `*.md`, `*.html` outside `src/`, `prompts/**`, `CLAUDE.md`, `.claude/**` are not checked (docs are translated once by this topic; Korean HTML reports and the held ADR tooling's index stay Korean).
- Hangul ranges: U+1100–11FF, U+3130–318F, U+A960–A97F, U+AC00–D7A3, U+D7B0–D7FF, U+FFA0–FFDC.
- Allowed:
  - `src/i18n/ko.ts`: anywhere except comments.
  - `src-tauri/src/ui_text.rs`: inside string literals on lines containing `Lang::Ko` (the Korean arms); other Korean lines there need a marker.
  - Frontend test files (`src/**/*.test.ts(x)`): anywhere except comments, test titles (`describe`/`it`/`test` first argument) and lines with `console.`.
  - `**/fixtures/*.jsonl` (recorded transcripts).
  - Lines carrying `hangul-ok`, and blocks between `hangul-ok-begin` / `hangul-ok-end`.
  - The path text itself of Korean file names (references to them in code, with `/` or `\`).
- After `cargo fmt`, re-run the check: a wrapped statement can separate a trailing marker from its literal — use block markers there.
- Accepted residual: Korean inside a frontend test's assertion diagnostic or a multi-line `console.*` argument in a test file passes the check; review covers it.
- Output: `file:line: text` per hit, exit 1 if any. The script has its own small test (planted hits in a temp dir: comment in `.rs`, `console.warn` in `src/*.ts`, allowed ko.ts value, allowed marker line).

## 5. Implementation units and waves

Each unit runs in its own git worktree based on the topic branch at the time the unit starts; main applies the diff and commits it. Each unit ends with: its package builds; its package's tests green; test count per test file before/after with no loss; for comment-only files, the comment-stripped token stream unchanged.

| Unit | Files | ~Lines |
|---|---|---:|
| U1 | crates `protocol`, `base`, `command`, `platform`, `transport`, `messaging`, `cli` (incl. marker in `placeholder.rs` + `src/api/wsFrame.ts` mirror + `wsFrame.test.ts`; the old-marker accept in `StructuredTextView.tsx` + its test moved here from U5 after review) | 5.7k |
| U2 | `crates/engram-dashboard-agent/**` (src incl. codex backend, tests, examples, `Cargo.toml`, bindings, `commands.schema.json`) — split in two coders by directory (`src/backend/codex/**` vs the rest) only for editing, tested and committed together | 14k |
| U3 | `crates/engram-dashboard-daemon/**` | 8.8k |
| U4 | `src-tauri/**` (keeps the Korean column of `ui_text.rs` and Rust tests asserting it, marked; rewords the `link/errors.rs` head claim that the texts stay byte-identical to the old shell texts) | 6.3k |
| U5 | `src/**` except U1's `src/api/wsFrame.ts` and `src/api/wsFrame.test.ts` (in `ko.ts` only comments); `index.html`, `vite.config.*`, `vitest.config.*`, `tsconfig*.json`, `package.json`; (the marker dual-accept in `StructuredTextView.tsx` landed with U1 after its review) | 5.9k |
| U6 | `scripts/**`, `.github/**`, `launch/**`, `_wip/**` (translate text dumps too), other root files not in U5; `check-hangul.mjs` (scans `git ls-files -z` by default, or explicit paths / a root given as arguments) + its test under `src/` (vitest only collects `src/**`; planted Hangul built from `\u` escapes) | 1.0k |
| W1 | docs: `docs/process/step-log.md`, `docs/tracking.md`, `docs/todo/**` | |
| W2a | docs: `docs/process/S21-codex-backend/**`, `docs/process/S21-crate-boundaries/**` (md only) | |
| W2b | docs: `docs/process/**` rest (md only), except `S33-english-translation/glossary.md` and `S38-english-internal-text/**` | |
| W3 | docs: `docs/research/**`, `docs/decisions/**` prose (not lint tokens; `docs/decisions/README.md` untouched — the held ADR tooling cites and writes it), `docs/reference/**` except `logging-conventions.md` and `structure/i18n.md`, `docs/handbook/**`, other `docs/**` md | |
| U9 | integration: glossary rows (harmonize duplicates), `logging-conventions.md` rewrite, `structure/i18n.md` line, `.git-blame-ignore-revs`, `check-hangul.mjs` to zero, one-time docs scan (§6 AC7), full regression, ADR | — |

Waves (at most 4 building worktrees at once; docs units do not build):
- Wave A: U1, U2, W1, W2a, W2b, W3.
- Wave B (base has U1, U2): U3, U4, U5, U6. (U4 waits for U1: `src-tauri/src/state/codec.rs` tests match text from `crates/engram-dashboard-base/src/file.rs` — U1 edits those test lines.)
- U9 last.

Commits: one per unit. The CI step of the Hangul check lands after U9 brought the check to zero (U6 delivers the script and its test; U9 commits the CI step). After W3, U9 runs `adr.mjs index --write` as its own commit (W3 translates `- 관련:` clauses the index cells are built from). `.git-blame-ignore-revs` gets the docs commits (W1–W3) and unit commits that change only comments and strings; the marker dual-accept, `check-hangul.mjs` + CI step and the logging rule are separate commits and do not go there.

## 6. Test plan

- Per unit: build + that package's tests (Rust: qa binding module commands; frontend: vitest + tsc); U1 also runs `src/api/wsFrame.test.ts`; test count per file; comment-only token compare.
- Review per unit: `/engram-review code` — side-by-side meaning check (negation, conditions, invariants) and a check that no code changed beyond strings, markers and regenerated files.
- QA after U9 (qa binding: Rust source + frontend rows → full regression + GUI):
  - CI three jobs locally, including the new Hangul step.
  - AC1: `check-hangul.mjs` clean; the planted cases fail it.
  - AC3: debug run, first lines of the daemon and app logs.
  - AC4: GUI — daemon-missing case on the connection band: `en` shows no Korean in our own text; `ko` shows the Korean head with an English cause; a live switch leaves no old-language text.
  - AC5: `git diff master -- prompts/` empty; schema summaries English.
  - AC6: test counts per crate / vitest file vs master.
  - AC7: one-time markdown scan over every tracked `.md` outside the hold list (`.s38-review/s38_docs.py` rules, copied into the repo as `scripts/` is not its place — U9 runs it from the topic folder) — remaining Hangul only inside 「」, backticks, code fences, links, ADR lint tokens, `docs/decisions/README.md`, the glossary and S38's own docs.

## 7. Decisions

- D1 screen errors: English causes inside the translated head; no wire change. Rejected: shell table entries (the text crosses IPC as a string and would keep the old language after a live switch — ADR-0373); per-error codes translated at render (a code + params seam through IPC and the frontend for each error — not now).
- D2 `CATALOG_VERSION` not bumped for translated summaries.
- D3 each unit commits its package's regenerated bindings.
- D4 English marker, frontend accepts the old Korean marker too. Rejected: `PROTOCOL_VERSION` bump (forces a daemon restart on update for a cosmetic case).
- D5 Hangul check over code and config with an allowlist and `hangul-ok` markers; docs not checked.
- D6 Korean file names, ADR lint tokens and the ADR index stay (the held `adr.mjs` parses and writes them); renames are a later batch.
- D7 Korean HTML reports under `docs/` stay (the user reads them).
- D8 frontend tests stay in `ko`; only titles and messages change.

## 8. Risks

- Parallel worktrees: one full cargo build each — at most 4 at once.
- Meaning drift: review per unit; token compare for comment-only files.
- OS-localized `io::Error` text inside causes stays in the OS language (out of scope; AC4 covers our own text).

## Review record

- 2026-11-18 · full (Claude + Codex) · PASS · reviewed edition 2.1 at `1d9b5459`; the round-3 wording items applied after it.
