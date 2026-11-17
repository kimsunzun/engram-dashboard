# S38 TRD — English-only internal text

- Status: edition 2 · 2026-11-18 · canonical ADR = the S38 ADR written after review (decisions D1–D8 below) · PRD = `prd.md`.
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
- **Command catalog (D2).** `///` summaries inside `declare_commands!` become English. `CATALOG_VERSION` is not bumped: it marks vocabulary changes (`crates/engram-dashboard-command/src/macros.rs`), is diagnostic only, and a translation changes no vocabulary. The unit owning the declarations commits the regenerated `commands.schema.json` and bindings.
- **Generated bindings (D3).** Each unit runs its package's tests, which regenerate that package's ts-rs bindings (`crates/engram-dashboard-agent/bindings`, `crates/engram-dashboard-protocol/bindings`, `src-tauri/bindings`, `commands.schema.json`); the unit includes the regenerated files in its diff so the CI sync gate stays green.
- **Placeholder marker (D4).** `placeholder_message!` in `crates/engram-dashboard-protocol/src/placeholder.rs` and its byte mirror in `src/api/wsFrame.ts` change to an English marker. The frontend comparison (`StructuredTextView.tsx` `errorMessageText`) also accepts the old Korean marker (one `hangul-ok` line): the handshake checks only `PROTOCOL_VERSION`, and a pre-S38 daemon can keep running across an app update (ADR-0342), so an old daemon would otherwise show the raw Korean marker.
- **Tests that match product text.** Product and tests of one package are in one unit, so the unit changes both together (PRD R6). Matches across packages (daemon tests on agent text, shell or frontend tests on another package's text) are handled by sequencing: the dependent unit starts from a base that already has the producer's English (§5 waves).

## 4. Hangul check (D5)

- `scripts/check-hangul.mjs` (node, no deps), run as `node scripts/check-hangul.mjs`, step in the CI `gates` job with `shell: bash`.
- Files: `git ls-files -z`, text files only (skip files with a NUL byte or a binary extension), and only code and config — `docs/**`, `*.md`, `*.html` outside `src/`, `prompts/**`, `CLAUDE.md`, `.claude/**` are not checked (docs are translated once by this topic; Korean HTML reports and the held ADR tooling's index stay Korean).
- Hangul ranges: U+1100–11FF, U+3130–318F, U+A960–A97F, U+AC00–D7A3, U+D7B0–D7FF, U+FFA0–FFDC.
- Allowed:
  - `src/i18n/ko.ts`: anywhere except comments.
  - `src-tauri/src/ui_text.rs`: inside string literals only.
  - Frontend test files (`src/**/*.test.ts(x)`): anywhere except comments, test titles (`describe`/`it`/`test` first argument) and lines with `console.`.
  - `**/fixtures/*.jsonl` (recorded transcripts).
  - Lines carrying `hangul-ok`, and blocks between `hangul-ok-begin` / `hangul-ok-end`.
  - The path text itself of Korean file names (references to them in code, with `/` or `\`).
- Output: `file:line: text` per hit, exit 1 if any. The script has its own small test (planted hits in a temp dir: comment in `.rs`, `console.warn` in `src/*.ts`, allowed ko.ts value, allowed marker line).

## 5. Implementation units and waves

Each unit runs in its own git worktree based on the topic branch at the time the unit starts; main applies the diff and commits it. Each unit ends with: its package builds; its package's tests green; test count per test file before/after with no loss; for comment-only files, the comment-stripped token stream unchanged.

| Unit | Files | ~Lines |
|---|---|---:|
| U1 | crates `protocol`, `base`, `command`, `platform`, `transport`, `messaging`, `cli` (incl. marker in `placeholder.rs` + `src/api/wsFrame.ts` mirror + `wsFrame.test.ts`) | 5.7k |
| U2 | `crates/engram-dashboard-agent/**` (src incl. codex backend, tests, examples, `Cargo.toml`, bindings, `commands.schema.json`) — split in two coders by directory (`src/backend/codex/**` vs the rest) only for editing, tested and committed together | 14k |
| U3 | `crates/engram-dashboard-daemon/**` | 8.8k |
| U4 | `src-tauri/**` (keeps the Korean column of `ui_text.rs` and Rust tests asserting it, marked) | 6.3k |
| U5 | `src/**` except `ko.ts` and U1's `wsFrame.ts`; `index.html`, root config; marker dual-accept in `StructuredTextView.tsx` | 5.9k |
| U6 | `scripts/**`, `.github/**`, `launch/**`, `_wip/**`, root files; `check-hangul.mjs` + CI step | 1.0k |
| W1 | docs: `docs/process/step-log.md`, `docs/tracking.md`, `docs/todo/**` | |
| W2 | docs: `docs/process/**` rest, except `*.html` and `S33-english-translation/glossary.md` | |
| W3 | docs: `docs/research/**`, `docs/decisions/**` prose (not lint tokens, not `README.md` index), `docs/reference/**` except `logging-conventions.md`, `docs/handbook/**`, other `docs/**` md | |
| U9 | integration: glossary rows, `logging-conventions.md` rewrite, `i18n.md` line, `check-hangul.mjs` to zero, full regression, ADR | — |

Waves (at most 4 building worktrees at once; docs units do not build):
- Wave A: U1, U2, U4, W1–W3.
- Wave B (base has U1, U2): U3, U5, U6.
- U9 last.

Commits: one per unit. `.git-blame-ignore-revs` gets the docs commits (W1–W3) and unit commits that change only comments and strings; the marker dual-accept, `check-hangul.mjs` + CI step and the logging rule are separate commits and do not go there.

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

(none yet)
