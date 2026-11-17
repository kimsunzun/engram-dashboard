# S35 TRD — UI language: English source, Korean on request (i18next behind `t()`)

- Status: edition 2 (2026-11-17) · canonical ADR = ADR-0372 (amends ADR-0349) · PRD = `prd.md` edition 2 (approved 2026-11-17)
- Reference this step changes: `docs/reference/structure/i18n.md` (made from the pre-S35 code in `46504260`; rewritten to the new behaviour in the commits below). Also edited where a sentence turns wrong: `docs/reference/structure/agent-backend.md` (quoted Korean labels), the `crates/engram-dashboard-protocol/src/lib.rs` header (quoted labels), the `src-tauri/src/layout/tab_name.rs` header, CLAUDE.md 「언어」.
- Edition 1 (Korean-only lookup swap) and its review record are history; the 2026-11-08 deep-review findings that still apply are folded in (§3, §4).

## 1. Scope and relation to the PRD

Implements PRD R1–R11 / AC1–AC7 with the D1–D5 defaults: key `ui.language`; the root re-renders on a change (no `react-i18next`); frontend tests pinned to Korean; a small en/ko table inside the shell; the dropped-output placeholder translated in the frontend.

English labels use sentence-style capitalization ("Close tab"), the Microsoft Writing Style Guide rule for button labels, menu items, dialog titles and tab labels (learn.microsoft.com/style-guide/capitalization). Placeholders keep the same `{word}` names as the Korean value. Status labels keep English text in `ko.ts` (PRD §2).

## 2. Module boundary

Frontend (`src/i18n/`):
- `en.ts` — the source table (`as const`, deep-frozen). Its type is the key and placeholder contract (`StringKey`, `Placeholders`).
- `ko.ts` — typed against `en.ts`: same keys, and per key the same placeholder set (a type-level check; a missing key, an extra key or a different placeholder set is a compile error). Deep-frozen.
- `index.ts` — the only importer of i18next. One private instance (`createInstance(...)` then `.init(opts)`, synchronous). Exports `t` (unchanged signature), `getLanguage()`, `setLanguage(lang)`, `subscribeLanguage(listener)` and a React hook `useLanguage()` (built on the subscription with `useSyncExternalStore`). `Lang = 'en' | 'ko'`.

Frontend wiring:
- Boot language: the shell plants `document.documentElement.lang` when it creates the window, through the same path that plants `data-theme` today (see the doc of `applyBootTheme` in `src/theme/bootTheme.ts`). `index.ts` reads it at module evaluation; absent or unknown → `en`.
- Live changes: a subscriber installed in `src/main.tsx` next to `installChatStyleApplier()` listens to `settingsClient` for `ui.language` and calls `setLanguage`.
- Re-render: `App` reads `useLanguage()`, so a change re-renders the tree under it (no remount — component state stays). Every `memo` component that renders `t()` text also reads `useLanguage()` (today: `LayoutLeaf`, its inner `SlotBody`).
- Rule (goes into the reference): no code keeps a `t()` result across renders or calls. Labels are resolved where they are shown.

Shell (`src-tauri`):
- Settings: new `SETTINGS` entry `ui.language`, `Kind::Choice(&["en", "ko"])`, default `"en"`, English `desc`. `SettingsEvents` gets a key-specific hook `ui_language_changed()` called from `set_and_notify` / `reset_and_notify` outside the locks, exactly like `theme_default_changed()`.
- New module `src-tauri/src/ui_text.rs` — `Lang`, the current language (read from the settings service at boot, updated by the hook), and one function per visible string or string family: tray labels, refusal title / leads / body frame, tab-name words. Callers ask it for text instead of holding Korean literals.
- Tray: the four item handles are kept in managed state (like `AutostartCheck`); on the hook, each gets `set_text` with the new label. The tooltip stays "Engram".
- Refusal dialog: `refusal_text` takes the language in effect when the box is shown.
- Tab names and window titles: `word()` takes the language. On the hook, the shell re-derives tab names and OS window titles for every window through the existing path a tab rename uses to reach the tab strip and `window_title()`. If no single path does both, the coder reports before adding one.
- State files: `is_word` recognizes a word in any language (`en`, `ko`), so files saved in either language — and today's Korean files — still mark auto names. The file format does not change. Rejected: saving a language-neutral marker (a state-format change and a migration for no visible gain).

Protocol / chat:
- The dropped-output placeholder keeps its fixed text in `crates/engram-dashboard-protocol/src/placeholder.rs` and its byte-equal mirror in `src/api/wsFrame.ts`; it is a marker. The chat view renders `t(<placeholder key>)` when an error item's message equals the marker, otherwise the message as is.

## 3. i18next init (version 27.x; names confirmed against the installed `typescript/options.d.ts`)

| Option | Value | Why |
|---|---|---|
| instance | `createInstance(opts)` then `.init()` at module evaluation | private, synchronous; `createInstance` alone does not init (review 2026-11-08) |
| `lng` | boot language (§2) | |
| `fallbackLng` | `false` | `ko` has every key by type; a miss stays loud |
| `initAsync` | `false` | inline resources, sync init |
| `resources` | `{ en: { translation: <copy of en> }, ko: { translation: <copy of ko> } }` | the tables are frozen; i18next gets copies |
| `keySeparator` / `nsSeparator` | `'.'` / `false` | keys are `ns.key` |
| `skipInterpolation` (per call) | `true` | lookup only — our own `{word}` interpolation runs after (R3; i18next's matcher `\{(.+?)\}` trims names and accepts `{-x}` / `{x,fmt}`, reproduced 2026-11-08) |
| nesting | off | values come from user data (tab names) |
| `returnNull` / `returnEmptyString` | `false` / `true` | an empty value is returned as `''`, not turned into a key fallback that `exists()` does not catch |

`setLanguage` calls `changeLanguage` (synchronous with inline resources), then notifies subscribers once.

## 4. `t()` adapter

- Missing key: `exists(key)` false → return the key unchanged, uninterpolated (today's behaviour).
- Lookup through the instance with interpolation skipped; then today's `interpolate` (`WORD_PLACEHOLDER`) fills `{word}` tokens. Params never reach i18next options, so `count`, `lng`, `ns`, `context`, `defaultValue` in params have no special meaning.
- Return type stays `string`.

## 5. Command titles and other stored strings

- `Command.title` becomes a lazy label: `title: () => t('…')`. The same for the helpers that take a title today (`registerSplit`, `registerWindowControl`, `registerToggle`, `registerConnectionCommands`) and for submenu container titles in `registerSlotMenu`. About 52 module-load sites in 10 files under `src/commands/` (counted 2026-11-17; the coder recounts first and reports a difference).
- `buildSlotMenu` / `resolveRunnable` and `registry.list()` call the label when they build their output, so `window.__engramCmd.list()` returns titles in the current language.
- Submenu containers get a stable `id`; `itemKey` uses `container:<id>` instead of the translated title. Sort tie-breaks use the id.
- `AgentList` keeps the error inputs (key and params, or the raw error) in `errorById` and translates at render.
- `connection.status` keeps returning `bandText()` computed at call time, in the current language.

## 6. Strings moved into the tables

- `PATH_NAME_PLACEHOLDER` (`src/util/basename.ts`) — becomes a key; callers call `t()` at render.
- Chat view: the error row header and the `arguments` fallback (`StructuredTextView.tsx`), the wait label (`WaitRow.tsx`) — keys; `ko.ts` keeps `Wait` (and other status labels) in English.
- The dropped-output placeholder (§2).
- Translating the ~204 existing values plus the new keys into English is part of unit U1.

## 7. Tests

Frontend:
- `vitest.config.ts` gets a `setupFiles` entry that calls `setLanguage('ko')` before each test file, so the existing Korean assertions stay. Tests that need English set it and restore it.
- `src/i18n/index.test.ts`: every existing assertion holds under `ko`; the malformed-brace scan runs over both tables; new cases — English is the default without a planted `lang`; a planted `lang="ko"` boots Korean; `setLanguage` changes `t()` output and notifies once; params named `count` / `lng` are plain values; `$t(...)` in a value stays literal; `<b>&` is not escaped; an empty value returns `''`; a module-scope `t()` works right after import (sync init).
- Compile-time (`@ts-expect-error`): a `ko` table missing a key, with an extra key, or with a different placeholder set fails; existing wrong-key / missing-param cases stay.
- Switch tests: after `setLanguage`, a rendered `SlotBody` (memo) shows the new text; the slot right-click menu shows translated command and container titles; `list()` titles follow the language; container identity and order are the same in both languages; an `AgentList` error shown before the switch is shown in the new language after it.

Shell (Rust):
- `ui_text`: every string exists in both languages; default `en`.
- Settings: the new key's choices, default and refusal of an unknown value; the key-count test moves from 12 to 13; `ui_language_changed` fires on set and reset of that key only.
- Tray / refusal: the existing Korean assertions become per-language assertions.
- Tab names: `is_word` accepts words of both languages; a Korean auto name from an old file stays auto.
- `foreign_word_is_the_one_the_frontend_shows` checks the shell's `en` and `ko` words against `slot.foreignContent` in `en.ts` and `ko.ts` (it reads the source text, so it pins those files' paths and the line form).

## 8. Test plan (qa)

- Frontend rows of the qa binding: `npx vitest run`, `tsc --noEmit` / `npm run build` (AC1, AC2).
- Shell rows: `cargo test` for `src-tauri` and the protocol crate (AC1).
- GUI scene (AC3–AC6), real app: fresh settings file → English (slot right-click menu, empty-slot text, connection band, tray, tab names, window titles); `engram` CLI `settings.set ui.language ko` with a popout open and a terminal running → everything Korean without a restart, terminal contents kept; app restart keeps Korean; `settings.reset ui.language` → English; a state file saved with Korean auto tab names loads them as auto names; `settings.set ui.language fr` refused with the allowed values (AC7).

## 9. Implementation units

Order and overlap: U1 → U2 touch the same frontend files and run in sequence. U3 (Rust) touches no frontend file and can run in parallel with U1/U2. The contract between them: key `ui.language`, values `en` | `ko`, default `en`; `<html lang>` planted at window creation; `slot.foreignContent` text equal on both sides.

- U1 — Frontend tables and backend: `en.ts`, `ko.ts` typing, `index.ts` on i18next with `getLanguage` / `setLanguage` / `subscribeLanguage` / `useLanguage`, vitest setup pin, `index.test.ts`, the i18next dependency. Ends green with the app in English (no switch wiring yet).
- U2 — Frontend live switch: boot language from `<html lang>`, settings subscriber, `App` / memo re-render, lazy titles and container ids, `AgentList` errors, strings moved into the tables (§6), switch tests. Reference `i18n.md` rewritten to the new behaviour in this unit's commit.
- U3 — Shell: settings key and hook, `ui_text`, tray `set_text`, refusal text, tab words and `is_word`, re-derive names and titles on change, plant `<html lang>` at window creation, Rust tests; header comments in `tab_name.rs` and the protocol `lib.rs`, `agent-backend.md` labels.
- Last commit of the topic: CLAUDE.md 「언어」 (app screen text = English source, Korean translation selectable) and the `Implemented` line.

## 10. Decisions

- User (PRD edition 2): English source, Korean selectable, one LLM-only key, default `en`, live switch, shell strings in scope, status labels English in `ko.ts`, D1–D5 defaults (ADR-0372).
- Main: lookup-only i18next with our own interpolation (closes the 2026-11-08 hold H1 — user confirmed 「ㅇㅇ 그렇게 하면 될것같은데?」); boot language via `<html lang>` planted like `data-theme`; `is_word` over both languages instead of a neutral marker; lazy title thunks and container ids; `ui_language_changed` as a key-specific hook like the theme one, not a generic listener list.

## 11. Risks

- A translated string kept somewhere and missed by the recount shows the old language after a switch — the switch tests and the reference rule; the GUI scene checks the visible surfaces.
- An element tree reused across renders (elements stored in state or refs) would not re-render on `App`'s change — coder reports any such place found.
- Re-deriving tab names and window titles may need a broadcast the shell does not have yet (§2) — reported before adding one.
- Another branch that adds keys only to `ko.ts` fails the type check after it pulls this change — intended.

## 12. Review record

- Edition 1: 2026-11-08 deep review FIX (findings folded into §3–§4 above).
