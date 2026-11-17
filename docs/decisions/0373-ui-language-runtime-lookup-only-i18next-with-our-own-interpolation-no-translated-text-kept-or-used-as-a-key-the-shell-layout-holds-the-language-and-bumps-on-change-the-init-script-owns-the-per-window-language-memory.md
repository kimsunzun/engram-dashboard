# ADR-0373: UI language runtime: lookup-only i18next with our own interpolation, no translated text kept or used as a key, the shell layout holds the language and bumps on change, the init script owns the per-window language memory

- 상태: 확정 (2026-11-17, 근거: S35 TRD edition 2.3, PRD+TRD deep review PASS on f7868b2a — main-agent design decisions)
- 관련: ADR-0372 (English source, Korean selectable, live switch) · ADR-0349 (i18next behind `t()`) · ADR-0301 (shell builds tab names; a write that changes an effective name bumps the version) · ADR-0265 (shell settings, theme pushed live) · `docs/process/S35-i18next-backend/trd.md` · `docs/reference/structure/i18n.md`

## 맥락
ADR-0372 makes the UI language switchable at run time with no restart, in every window, the tray, the refusal dialog, shell-made tab names and OS window titles. The S35 design review (2026-11-17) found the places where a live switch silently fails: translated text captured once or used as an identity, layout notifications dropped as not newer, a shell-wide language racing between hooks and tests, and a boot path that never read the setting.

## 결정
1. **Lookup-only i18next.** `t()` resolves the key through a private, synchronously initialized i18next instance with interpolation skipped, then fills `{word}` tokens with our own `interpolate`. A key that is not exactly `ns.key`, or whose resolved resource value is not a string, returns the key unchanged.
2. **No translated text is kept or used as a key.** Labels are resolved where they are shown: command and submenu titles are lazy functions; container identity, React keys, dismiss keys and edit baselines are built from untranslated inputs; stored errors keep their inputs and are translated at render.
3. **Re-render from the root.** `App` and every `memo` boundary whose subtree renders `t()` text read the language through `useLanguage()` (`src/i18n/react.ts`); `src/i18n/index.ts` imports neither React nor Tauri.
4. **The shell layout holds the language.** `ViewManager` has the language as a field, set from the effective `ui.language` before the boot swap and kept by `adopt_restored`. A change that alters an effective auto tab name bumps the layout version and announces the affected windows (the `update_agent_names` pattern), as ADR-0301 invariant 2 requires. `ui_text` functions take the language explicitly; there is no process-global language.
5. **One serialized hook, off the main thread.** `ui_language_changed` runs under one mutex, reads the current effective setting (last stored value wins whatever order hooks run in), changes the layout under its lock, releases it, announces, then calls tray `set_text`.
6. **The init script owns the per-window language memory.** It plants `<html lang>` from `sessionStorage['engram.lang.' + label]` when that is `en` or `ko`, else from the effective setting; `src/i18n/langMemory.ts` (prefix `LANG_KEY_PREFIX`) writes it after a value is applied. The prefix is pinned by a shell test.
7. **`is_word` is a seed filter over both languages.** Auto vs named stays `TabEntry.named`; `is_word` only keeps shell words of either language out of the agent-name ledger seed.

## 거부한 대안
- **i18next's own interpolation** — its matcher `\{(.+?)\}` trims names and accepts `{-x}` / `{x,fmt}`, so `{한글}`, `{first-name}`, `{ name }` get substituted; reproduced with a probe against i18next 27.0.0 (2026-11-08), breaking today's pinned behaviour in `index.test.ts`.
- **`react-i18next` hooks at every call site** — call-site changes across about 36 files for the same re-render that a root subscription gives.
- **A process-global language in the shell** — `effective_tab_name` is called from layout, rows and state conversion code; a global makes parallel Rust tests share it, and hooks that run outside the settings locks can apply an older value last (review 2026-11-17).
- **Re-announcing through `apply::sync_titles`** — it does not bump the version, so `viewStore.ts` (`payload.version <= prev.version`) and `chrome/book.rs` (`accept_title`) drop the payload (code).
- **A language-neutral marker for auto tab names in the state file** — it already exists as `TabEntry.named`; nothing to change (code).
- **Reading the language memory inside `src/i18n/index.ts`** — the per-window key needs the window label from Tauri, which would pull `@tauri-apps/api/window` into every module that imports `t` (36 modules) and the tests that mock them (code).

## 근거
- S35 TRD edition 2.3 and its deep review (lead Claude, risk-axis Claude "captured or persisted translated text", cross-family GPT; three rounds; PASS on f7868b2a).
- Code paths checked by the reviewers: `viewStore.ts` `applyWindowTabsUpdated`, `chrome/book.rs` `accept_title`, `ViewManager::update_agent_names`, `apply::agent_names_updated` / `sync_titles`, `boot_plugin.rs` swap, `ViewManager::adopt_restored`, `convert.rs` `restored_name` / `agent_name_seed`, `webview_env.rs` `theme_script` and its prefix pin test, tauri 2.11.3 `set_text` (waits for the main thread), `settings_set` / bus verbs on `spawn_blocking`.

## 영향 / 불변식
- A new UI string goes through `t()` at the point of display; storing a `t()` result, or using one as a key, id, sort key or comparison baseline, is a defect.
- A new `memo` boundary over `t()` text reads `useLanguage()`.
- A shell change to tab-name words or their language must keep the version bump; a new `ViewManager` constructor or swap path must set the language from the setting.
- The language hook must not run on the main thread and must not hold the layout lock while calling tray APIs.
- `LANG_KEY_PREFIX` and the init script's prefix stay equal (pinned test).
