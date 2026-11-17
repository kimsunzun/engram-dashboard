# S35 PRD — UI language: English source, Korean on request (i18next behind `t()`)

- Status: edition 2 (2026-11-17) · approved 2026-11-17 (no objection to edition 2 or the D1–D5 defaults; user on labels: 「한국 영어 테이블 둘다 있고 한글 테이블에 영어 넣으면 되잖아」) · decisions = ADR-0349 (amended by the edition-2 ADR) · research = `docs/research/i18n-oss-library-reevaluation-2026-11-07.md` · reference = `docs/reference/structure/i18n.md`
- Edition 1 (approved 2026-11-08, Korean only, lookup swap only) is replaced by the user decisions of 2026-11-08 and 2026-11-17: the UI source becomes English, Korean is selectable, the switch applies immediately, the language is set through LLM commands only.

## 1. What · why

The app's UI text becomes English by default, and Korean becomes a translation the user can switch to by telling an LLM (no settings screen yet). The switch applies at once everywhere the user reads UI text: the React UI in every window, the tray menu, the native refusal dialog, the shell-computed tab names and the OS window titles. Inside `t()`, i18next replaces the home-made lookup.

Why: the user wants English as the app's base language (internal text — logs, errors, LLM-facing text — goes English-only in the next topic) with Korean available on request, built on a mainstream library instead of growing the home-made layer for language switching.

## 2. Scope / non-goals

- In scope:
  - Frontend tables: `en.ts` (new, the typed source) and `ko.ts` (kept, same keys).
  - `t()` backend: i18next for lookup; our own `{word}` interpolation stays.
  - One settings key for the UI language, read and changed through the existing LLM settings commands.
  - Live switch in the frontend: re-render on change, command and submenu titles resolved when used, no stored translated strings.
  - Rust shell visible strings in both languages, following the key: tray labels, refusal dialog, auto tab-name words, OS window titles.
  - Screen strings that bypass `t()` today (`(경로 없음)`, the chat view's `Error` / `Wait` / `arguments`, the dropped-output placeholder message). All of them go through the tables. In Korean, the error header and the input caption are translated; status labels (`Wait`) keep their English text in `ko.ts` too (user, 2026-11-17: 「상태? 같은건 영어가 좋을것 같음 … 나중에 확인해보면서 조정해도 되니」 — this differs from the surveyed convention, where Roo Code, Open WebUI and LibreChat translate "Running"/"Thinking"); tuning later is a `ko.ts` value edit only. Tool names coming from an agent (`Bash`, `read_file`) stay raw identifiers in both languages (same convention: not locale keys in those apps).
  - Tests: frontend tests pinned to Korean, new tests for English and for the switch; Rust tests that assert Korean labels updated.
  - Docs: CLAUDE.md 「언어」, the i18n reference, reference sentences that quote Korean UI labels.
- Non-goals:
  - A settings screen or any UI control for the language.
  - OS-locale detection.
  - Languages other than English and Korean; plurals.
  - Logs, thrown error text, LLM-facing text (settings `desc`, command `help`) — next topic (English only).
  - Test names and test messages — the topic after that.
  - LLM read/override of labels (ADR-0069 §5).

## 3. Requirements

- R1 — `en.ts` is the typed source of keys and placeholders. `ko.ts` must have exactly the same keys and the same placeholder set per key; a mismatch is a compile error.
- R2 — The public `t()` signature and types stay; call sites change only where a translated string is captured once (module-load titles, stored strings).
- R3 — Interpolation parity with today: our own `{word}` interpolation runs after an i18next lookup (all of today's `index.test.ts` behaviour holds).
- R4 — No HTML escaping of values.
- R5 — A settings key selects the UI language: values `en` | `ko`, default `en`, changed through `settings.set` / `settings.reset` (and read through `settings.get` / `settings.schema`). No new command.
- R6 — Live switch: after a change, every open window, the slot right-click menu, the tray menu, tab names and OS window titles show the new language without a restart. Component state (terminal contents, typed input, scroll) is kept.
- R7 — Command and submenu titles are resolved when shown; submenu identity does not depend on translated text.
- R8 — The refusal dialog uses the language in effect when it is shown.
- R9 — State files: auto tab names saved in either language, and today's Korean-only files, are still recognized as auto names on load.
- R10 — A missing key still returns the key text (today's behaviour).
- R11 — Only `src/i18n/` imports i18next.

## 4. Acceptance criteria

- AC1 — `npx vitest run` and the Rust test suites are green.
- AC2 — `tsc` green; a key missing from `ko.ts`, or a key whose placeholders differ, fails the type check (compile-time test).
- AC3 — With a fresh settings file the app shows English: slot right-click menu, empty-slot text, connection band, tray menu, tab names, OS window titles.
- AC4 — `settings.set` of the language key to `ko` (through the CLI or an LLM): the main window and an open popout, the slot right-click menu, the tray menu, tab names and window titles switch to Korean without a restart, and an open terminal keeps its contents. `settings.reset` of the key returns everything to English.
- AC5 — After an app restart the chosen language is kept.
- AC6 — A state file with Korean auto tab names (saved before this change) loads with those tabs still auto-named, shown in the current language.
- AC7 — Setting the key to an unknown value (e.g. `fr`) is refused and the answer lists the allowed values.

## 5. Open decisions (main defaults)

- D1 — Key name: `ui.language` (default) vs `language` vs `locale`. Existing keys are dotted by area (`theme.default`, `chat.style.*`).
- D2 — How the frontend re-renders: the root re-renders on a language change and the whole tree follows (default; no new dependency) vs `react-i18next` hooks in every component (call-site changes across about 36 files).
- D3 — Tests: pin the frontend test run to Korean through a vitest setup file, so the existing Korean assertions stay, plus new English and switch tests (default) vs rewrite the Korean assertions to English.
- D4 — Rust shell strings: a small en/ko table inside the shell (default) vs reading the frontend tables (couples the shell build to frontend source).
- D5 — Dropped-output placeholder: the protocol keeps its fixed text as a marker and the frontend shows the translated text when it sees that marker (default) vs translating in the daemon (the daemon does not know the UI language).

## 6. Risks

- A translated string captured somewhere and not refreshed shows the old language after a switch — guarded by the switch tests and the reference rule "do not store `t()` results".
- State-file compatibility for auto tab names (R9, AC6).
- Another branch that adds keys only to `ko.ts` fails the type check after it pulls this change — intended; the fix is to add the English text.
- A Rust test reads `src/i18n/ko.ts` as source text — it moves with the table change.
- Bundle size +~15 kB gzip for i18next — acceptable for a desktop app.
