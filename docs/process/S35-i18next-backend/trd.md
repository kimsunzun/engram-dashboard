# S35 TRD — UI language: English source, Korean on request (i18next behind `t()`)

- Status: edition 2.2 (2026-11-17, deep-review rounds 1–2 applied) · canonical ADR = ADR-0372 (amends ADR-0349) · PRD = `prd.md` edition 2 (approved 2026-11-17)
- Reference this step changes: `docs/reference/structure/i18n.md` (made from the pre-S35 code in `46504260`; its `is_word` sentence and its inventory are corrected in the edition-2.1 commit, then rewritten to the new behaviour in U2). Also edited where a sentence turns wrong: `docs/reference/structure/agent-backend.md` (quoted Korean labels), the `crates/engram-dashboard-protocol/src/lib.rs` header (quoted labels), the `placeholder.rs` doc and the `src/api/wsFrame.ts` comment ("consumers do not branch on this value"), the `src-tauri/src/layout/tab_name.rs` header, the `USAGE_LOCALE` comment in `src/components/slot/usageFormat.ts`, CLAUDE.md 「언어」.
- Edition 1 (Korean-only lookup swap) and its review record are history; the 2026-11-08 deep-review findings that still apply are folded in (§3, §4).

## 1. Scope and relation to the PRD

Implements PRD R1–R11 / AC1–AC7 with the D1–D5 defaults: key `ui.language`; the root re-renders on a change (no `react-i18next`); frontend tests pinned to Korean; a small en/ko table inside the shell; the dropped-output placeholder translated in the frontend.

English labels use sentence-style capitalization ("Close tab"), the Microsoft Writing Style Guide rule for button labels, menu items, dialog titles and tab labels (learn.microsoft.com/style-guide/capitalization). Placeholders keep the same `{word}` names as the Korean value. Status labels: see §10 (open — H1).

## 2. Module boundary

Frontend (`src/i18n/`):
- `en.ts` — the source table (`as const`, deep-frozen). Its type is the key and placeholder contract (`StringKey`, `Placeholders`).
- `ko.ts` — typed against `en.ts`: same keys, and per key the same placeholder set (a type-level check; a missing key, an extra key or a different placeholder set is a compile error). Deep-frozen. Today it has 216 keys in 11 namespaces.
- `index.ts` — the only importer of i18next. One private instance. Exports `t` (unchanged signature), `getLanguage()`, `setLanguage(lang)`, `subscribeLanguage(listener)`. `Lang = 'en' | 'ko'`. `setLanguage` with the current value does nothing. It imports neither React nor Tauri.
- `react.ts` — `useLanguage()` (on `subscribeLanguage` with `useSyncExternalStore`). Kept apart so that importing `t` does not pull React in.

Frontend wiring:
- Boot language: `index.ts` reads only `<html lang>` (through `globalThis.document?.documentElement?.lang`, so node-environment test files still load it); absent or unknown → `en`. The shell's init script decides that value, exactly like `data-theme` (`theme_script` in `webview_env.rs`): the per-window memory in `sessionStorage` under the prefix `engram.lang.` + window label wins (so a reload — e.g. `RootErrorBoundary`'s reload button — does not flash the old language), else the effective `ui.language` at window creation.
- `setLanguage` updates `<html lang>` (guarded like the read). The per-window memory is written by a small helper outside `src/i18n` (shaped like `src/theme/themeMemory.ts`, guarded against a missing or throwing `sessionStorage`), called by the `main.tsx` subscriber after it applies a value.
- Live changes: a subscriber installed in `src/main.tsx` next to `installChatStyleApplier()` listens to `settingsClient` for `ui.language` and calls `setLanguage`. `settingsClient` replays the current value on subscribe and pulls after listening, so a stale boot value corrects itself.
- Re-render: `App` reads `useLanguage()`, so a change re-renders the tree under it (no remount — component state stays). Every `memo` boundary whose subtree renders `t()` text also reads `useLanguage()` (today: `LayoutLeaf` and its inner `SlotBody`).
- Rule (goes into the reference): no code keeps a `t()` result across renders or calls, and no translated text is used as a key, id or comparison baseline. Labels are resolved where they are shown.

Shell (`src-tauri`):
- Settings: new `SETTINGS` entry `ui.language`, `Kind::Choice(&["en", "ko"])`, default `"en"`, English `desc`. `SettingsEvents` gets a key-specific hook `ui_language_changed()` called from `set_and_notify` / `reset_and_notify` (including a prefix or full reset that covers the key) outside the settings locks, like `theme_default_changed()`.
- No process-global language. `ui_text.rs` holds `Lang` and pure functions that take a `Lang` explicitly: tray labels, refusal title / leads / body frame, tab-name words. The language the layout uses lives in `ViewManager` (a field changed under the layout lock); the tray and the refusal box read the effective setting when they need it.
- The hook body is serialized (one mutex) and reads the **current effective** `ui.language` from the settings service instead of trusting the value it was called for, so two near-simultaneous writes end with the last stored value applied everywhere, whatever order the hooks run in. Invariant: the hook never runs on the main thread (its callers — the `settings_set` / `settings_reset` commands and the bus verbs — already run off it). Order inside the hook: under the layout lock set the language, bump, collect labels → release the lock → announce → tray `set_text` (which waits for the main thread, so it is never called while holding the layout lock).
- Boot: the `ViewManager` language is set from the effective `ui.language` before the boot model is swapped in and before the boot `sync_titles`, so a restart in Korean shows Korean tab names and titles from the first frame. If the boot plugin runs before the settings service is loaded, the coder moves the value in at the swap point (the service is loaded in the builder) and reports how. `adopt_restored` keeps the live manager's language, not the restored one's.
- Tab names and OS window titles: under the layout lock the hook sets the `ViewManager` language; if any effective auto tab name changes, the manager bumps its version (ADR-0301 invariant 2 — a write that changes an effective name bumps the version) and returns the labels of the affected windows; then the hook announces them, following `update_agent_names` + `apply::agent_names_updated`. Without the bump both receivers drop the payload as not newer (`viewStore.ts` `applyWindowTabsUpdated`, `chrome/book.rs` `accept_title`). `apply::sync_titles` is not used for this — it does not bump.
- Tray: the four item handles are kept in managed state (like `AutostartCheck`) and looked up with `try_state` (a failed `build_tray` is tolerated at startup); the hook calls `set_text` on each. The tooltip stays "Engram".
- Refusal dialog: `refusal_text` takes the `Lang` in effect when the box is shown.
- State files: whether a tab name is auto or named is decided by `TabEntry.named` (`restored_name`), not by the text — this already holds in either language and the file format does not change. `is_word` only keeps a shell word from being seeded into the agent-name ledger (`agent_name_seed`, ADR-0301 decisions 3 and 5); it recognizes the words of both languages, so neither an old "에이전트" nor a new "Agent" is seeded.

Protocol / chat:
- The dropped-output placeholder keeps its fixed text in `crates/engram-dashboard-protocol/src/placeholder.rs` and its byte-equal mirror in `src/api/wsFrame.ts`; it becomes a marker. The chat view renders `t(<placeholder key>)` when an error item's message equals the marker, otherwise the message as is. The two "do not branch on this value" comments are rewritten.

## 3. i18next init (version 27.x)

i18next is not installed yet; U1 adds it and confirms every option name below against the installed `typescript/options.d.ts`, using the version's equivalent and reporting any difference.

| Option | Value | Why |
|---|---|---|
| instance | `createInstance()` then `.init(opts)` at module evaluation | private, synchronous; `createInstance` alone does not init (review 2026-11-08) |
| `lng` | boot language (§2) | |
| `fallbackLng` | `false` | `ko` has every key by type; a miss stays loud |
| `initAsync` | `false` | inline resources, sync init |
| `resources` | `{ en: { translation: <copy of en> }, ko: { translation: <copy of ko> } }` | the tables are frozen; i18next gets copies |
| `keySeparator` / `nsSeparator` | `'.'` / `false` | keys are `ns.key` |
| `skipInterpolation` (per call) | `true` | lookup only — our own `{word}` interpolation runs after (R3; i18next's matcher `\{(.+?)\}` trims names and accepts `{-x}` / `{x,fmt}`, reproduced 2026-11-08) |
| nesting | off | values come from user data (tab names) |
| `returnNull` / `returnEmptyString` / `returnObjects` | `false` / `true` / `false` | an empty value is `''`; with `returnObjects: false` a namespace comes back as i18next's "returned an object" warning string, so the §4 miss check looks at the resolved resource value, not at `t()`'s return |
| console notices | off if the version prints a support / info notice at init | otherwise every window and every test file logs it |

`setLanguage` calls `changeLanguage` (synchronous with inline resources), then notifies subscribers once.

## 4. `t()` adapter

- Miss: a key that is not exactly `ns.key`, or whose resolved value is not a string → return the key unchanged, uninterpolated (today's behaviour, also for a bare namespace like `'tab'` from a cast or a JS caller).
- Lookup through the instance with interpolation skipped; then today's `interpolate` (`WORD_PLACEHOLDER`) fills `{word}` tokens. Params never reach i18next options, so `count`, `lng`, `ns`, `context`, `defaultValue` in params have no special meaning.
- Return type stays `string`.

## 5. Captured, stored or key-forming translated strings

- Command titles become lazy labels: `title: () => t('…')`. Today about 58 module-load `t()` calls in 10 files under `src/commands/` (about 56 title lines by a machine count; `usageCommands.ts` `registerToggle` holds 3 calls and is registered twice; `registerConnectionCommands` calls `t()` inside). `registerSplit` and `registerWindowControl` take a title argument and take the lazy label instead; `registerToggle` and `registerConnectionCommands` build lazy labels inside. Folder-picker dialog titles are resolved at click time already. The coder recounts first and reports a difference.
- `buildSlotMenu` / `resolveRunnable` and `registry.list()` call the label when they build their output, so `window.__engramCmd.list()` returns titles in the current language. The title type check in `slotMenu.ts` changes from "is a string" to "is a function".
- Submenu containers get a stable `id`; it replaces the translated title in `itemKey`, in the resolved item `id` (`container:<id>`), in the React key and in `data-slot-menu-container`, and in sort tie-breaks. An open flyout survives a switch.
- React keys built from labels become ids: the row menu in `AgentList.tsx` and the list in `PresetPalette.tsx` (`key={item.label}`).
- `AgentList`: `errorById` keeps the error inputs (key and params, or the raw error) and translates at render; `commitEdit` compares the draft with the value captured when editing started, not with a freshly translated display name (so a switch during an edit does not save the placeholder text as a profile name).
- `ConnectionNotice`: the dismiss key is built from untranslated inputs (`kind`, the refused flag, the raw connection error); `bandView` returns a cause descriptor and the text is made at render. A dismissed band stays dismissed across a switch.
- LLM-facing text that follows the UI language until the next topic makes LLM-facing text English-only: `__engramCmd.list()` titles, the `connection.status` reply text (`bandText()` at call time), `layout.rows` `tab_name`, `tab.list` `name`. Recorded as a known exception to ADR-0372's "LLM-facing text does not follow this key".

## 6. Strings moved into the tables

- `PATH_NAME_PLACEHOLDER` (`src/util/basename.ts`) — becomes a key; callers call `t()` at render.
- Chat view: in `StructuredTextView.tsx` the error row header, the tool row's `Error` badge, its `In` / `Out` captions and the `arguments` fallback; the wait label in `WaitRow.tsx`.
- The dropped-output placeholder (§2).
- Date formatters made at module load with a fixed `ko-KR` locale — `SAVED_AT` in `RestoreModal.tsx`, `USAGE_LOCALE` / `CLOCK` in `usageFormat.ts` — follow the UI language (`en-US` / `ko-KR`), made per language when used, not at module load.
- Translating the existing values plus the new keys into English is part of unit U1.

## 7. Tests

Frontend:
- `vitest.config.ts` gets a `setupFiles` entry that calls `setLanguage('ko')` before each test file, so the existing Korean assertions stay; it must also load in `// @vitest-environment node` files. Tests that need English set it and restore it.
- `src/i18n/index.test.ts`: every existing assertion holds under `ko`; the malformed-brace scan runs over both tables; new cases — English is the default with nothing planted; a planted `lang="ko"` boots Korean; `index.ts` loads in a node-environment file; `setLanguage` changes `t()` output, updates `<html lang>`, notifies once, and does nothing for the same value; params named `count` / `lng` are plain values; `$t(...)` in a value stays literal; `<b>&` is not escaped; an empty value returns `''`; a bare namespace key returns the key; a module-scope `t()` works right after import (sync init).
- Compile-time (`@ts-expect-error`): a `ko` table missing a key, with an extra key, or with a different placeholder set fails; existing wrong-key / missing-param cases stay.
- Switch tests: after `setLanguage`, a rendered `SlotBody` (memo) shows the new text; the slot right-click menu shows translated command and container titles and an open flyout stays open; `list()` titles follow the language; container identity and order are the same in both languages; an `AgentList` error shown before the switch is shown in the new language after it; a dismissed connection band stays dismissed; a draft being edited in `AgentList` is not saved by a switch.
- Existing test files that assert `title: '…'` strings (about 72 places in 10 files) are updated for lazy titles.

Shell (Rust):
- `ui_text`: every string exists in both languages.
- Settings: the new key's choices, default and refusal of an unknown value; the key-count asserts (4 places: `registry.rs` and `settings/tests.rs`) move from 12 to 13; `ui_language_changed` fires on set, reset and a covering prefix reset of that key only.
- Hook ordering: two writes whose hooks run in reverse order end with the last stored value applied to the layout and the tray.
- Boot: a boot model built with `ui.language = ko` yields Korean auto tab names and window titles before any write; `adopt_restored` keeps the live language.
- Init script: the per-window language memory wins over the setting; the prefix `engram.lang.` is pinned against the frontend helper's constant (like the theme pin test next to `THEME_MEMORY_PREFIX`).
- Layout: a language change with an auto-named tab bumps the version and announces every affected window; a change with only named tabs does not bump; `is_word` keeps both "에이전트" and "Agent" out of the agent-name seed.
- Existing Rust tests that assert Korean auto names (`manager.rs`, `convert.rs`, `tests/layout_apply.rs`, `tests/layout_commands.rs`, `codec.rs`, `chrome/book.rs`) pass `Lang` explicitly; where Korean is the subject they keep Korean.
- Tray / refusal: the existing Korean assertions become per-language assertions.
- `foreign_word_is_the_one_the_frontend_shows` checks the shell's `en` and `ko` words against `slot.foreignContent` in `en.ts` and `ko.ts` (it reads the source text, so it pins those files' paths and the line form `foreignContent: '…'`; the English text must not contain `'`).
- Signatures stay inside the CI shell gate 2 form when `slot_label` gains a language argument.

## 8. Test plan (qa)

- Frontend rows of the qa binding: `npx vitest run`, `tsc --noEmit` / `npm run build` (AC1, AC2).
- Shell rows: `cargo test` for `src-tauri` and the protocol crate (AC1).
- GUI scene (AC3–AC6), real app: fresh settings file → English (slot right-click menu, empty-slot text, connection band, tray, tab names, window titles); `engram` CLI `settings.set ui.language ko` with a popout open and a terminal running → everything Korean without a restart, terminal contents kept, an open slot-menu flyout stays open; reload a window → Korean on first paint; app restart keeps Korean; `settings.reset ui.language` → English; a state file saved with Korean auto tab names loads them as auto names; `settings.set ui.language fr` refused with the allowed values (AC7).

## 9. Implementation units

Order: U1 first. Then U2 (frontend) and U3 (shell) in parallel — they touch no common file. Contract: key `ui.language`, values `en` | `ko`, default `en`; `<html lang>` planted at window creation by the init script, which reads the per-window memory `sessionStorage['engram.lang.' + label]` first; the frontend helper writes that key; `slot.foreignContent` = `Unknown content` in `en.ts` and `알 수 없는 내용` in `ko.ts` (equal to the shell's `Foreign` word in each language).

- U1 — Frontend tables and backend: `en.ts`, `ko.ts` typing, `index.ts` on i18next with `getLanguage` / `setLanguage` / `subscribeLanguage`, `react.ts`, vitest setup pin, `index.test.ts`, the i18next dependency. Ends green with the app in English (no switch wiring yet).
- U2 — Frontend live switch: `<html lang>` read, the memory-writing helper, settings subscriber, `App` / memo re-render, §5, §6, switch tests. Reference `i18n.md` rewritten to the new behaviour in this unit's commit.
- U3 — Shell: settings key and hook, `ui_text`, `ViewManager` language (boot value, `adopt_restored`) and version bump, tray `set_text`, refusal text, `is_word` over both languages, plant `<html lang>` and the theme-style memory at window creation, Rust tests; header and doc comments in `tab_name.rs`, `placeholder.rs`, the protocol `lib.rs`, `agent-backend.md` labels.
- Last commit of the topic: CLAUDE.md 「언어」 (app screen text = English source, Korean translation selectable) and the `Implemented` line.

## 10. Decisions

- User (PRD edition 2): English source, Korean selectable, one LLM-only key, default `en`, live switch, shell strings in scope, status labels English in `ko.ts`, D1–D5 defaults (ADR-0372).
- Main: lookup-only i18next with our own interpolation (closes the 2026-11-08 hold H1 — user confirmed 「ㅇㅇ 그렇게 하면 될것같은데?」); boot language from the per-window memory, then `<html lang>` planted like `data-theme`; no process-global language in the shell, the layout's language in `ViewManager` and a version bump when an auto name changes; the hook serialized and reading the latest stored value; `is_word` over both languages as a seed filter (auto vs named stays `TabEntry.named`); lazy title thunks and container ids; untranslated inputs for every key, id and comparison baseline; date formatters follow the UI language.
- Open — H1 (review 2026-11-17): which labels count as "status labels" kept English in `ko.ts`. Today's Korean values include `chat.interrupting` (중단하는 중…), `chat.toolGroupRunning` (진행 중…), `chat.toolDeclined` (거부됨), `connection.reconnecting` (연결 중…), `common.loading` (로딩 중…) and others. Main default: only the newly moved wait label (`Wait`) keeps English; the newly moved captions (`Error`, `In` / `Out`, `arguments`) are translated as PRD §2 says; existing Korean values stay Korean and are tuned later by editing `ko.ts`.

## 11. Risks

- A translated string kept somewhere and missed by the recount shows the old language after a switch — the switch tests, the reference rule, the GUI scene.
- An element tree reused across renders (elements stored in state or refs) would not re-render on `App`'s change — none found in the 2026-11-17 review; the coder reports any found.
- A real agent name equal to an English shell word ("Usage") is not seeded at boot and shows the word until the next roster arrives (one boot flicker); downgrading to an older build after English names were saved may seed "Agent" for a dead agent's tab (cosmetic).
- Another branch that adds keys only to `ko.ts` fails the type check after it pulls this change — intended.

## 12. Review record

- (no pass yet)
