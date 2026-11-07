# Session restore snapshots after an abnormal exit — retention and rotation precedents (2026-10-02)

> Strength: light (1 collector · main-session spot grounding). Trigger: confirming the source of S21-storage TRD §10 F17 「최근 3 개 회전 보관(Firefox 류)」. Link: `docs/process/S21-storage/trd.md` §6-7 · §10 F17.
> **Conclusion: there is no precedent for 「크래시 스냅숏 N 개 회전」 (rotating N crash snapshots).** Mature apps keep only the live file + a copy one generation back (+ the last clean-exit copy). Firefox's 「3 개」 (3 items) applies **only to upgrade backups**.
> **Decision (2026-10-02, user decisions D1–D3):** F17 「최근 3 개 회전 보관」 withdrawn · crash copy = a single `shell\state\state.crash.json` · after a crash, always ask (Chromium style — rejected = Firefox style of quietly restoring the first crash). Reflected in TRD §6-5 · §6-7 · §14-9. **D8** (2026-10-02 — write failures are not handled separately) rests on the "Addendum" below — TRD §6-4 · §14-10.

## Comparison

| | Firefox | Chromium | VS Code (window state) | Windows Terminal |
|---|---|---|---|---|
| Files | clean-exit copy `sessionstore` · running `recovery` · one generation back `recovery.baklz4` · last clean copy `previous` · upgrade backups ≤3 | `Sessions/Session_<ts>` append-only log — current + 1 previous valid copy | one `windowsState` key in `storage.json` (no rotation) · SQLite has 1 `.backup` | one `state.json` (no rotation) |
| Detecting abnormal exit | CrashMonitor checkpoint (if absent, `state == "running"`) | `profile.exit_type` set to "Crashed" at start, "Normal" at exit | none | none |
| When broken | next candidate in order: clean → recovery → recovery.bak → previous(+upgrade) | from the newest, the first file whose 「초기 상태 표지」 (initial state marker) can be read | sqlite: swapped for `.backup` → empty DB | starts with empty state |
| Atomicity | temp + rename · backup is a move · no fsync | append-only + marker for validity | temp (`.vsctmp`) + rename | temp + rename · fsync unconfirmed |
| Asks? | the first crash is restored quietly; on consecutive crashes (more than `max_resumed_crashes=1`) · or a session older than 6 hours, a prompt screen appears | **no automatic restore** after a crash (prevents crash loops) — 「페이지 복원?」 (restore pages?) | quietly, per setting | quietly, per setting (1 second debounce · layout every 5 minutes + at exit) |

## What this gives our design

- **There is no basis for N-way rotation** — the citation 「Firefox 류 3 개」 (Firefox-style 3 items) is wrong (3 is the upgrade backups). For a crash snapshot, **one generation** matches the precedents.
- **Separate 「뻗었나」 (did it die?) from 「어느 파일이 멀쩡한가」 (which file is intact?)** — detection uses a separate marker (Chromium sets Crashed at start and clears it at clean exit), the file uses fixed-order candidates + validation.
- **Promote to backup only what is confirmed intact** — blind rotation pushes out the last intact snapshot in a crash loop (common to Firefox · VS Code).
- **Crash loop prevention** — on consecutive crashes, do not restore automatically; ask (Firefox once · Chromium always asks).
- Atomic writes are temp + rename; none of the three apps was confirmed to use fsync — a power cut is covered by the one-generation copy.

## Sources (main-session spot confirmed = ✔)

- Firefox `browser/components/sessionstore/SessionFile.sys.mjs` — file roles L82–150 · read order L165 ✔ — https://github.com/mozilla-firefox/firefox/blob/main/browser/components/sessionstore/SessionFile.sys.mjs
- Firefox `SessionWriter.sys.mjs` — backup only when recovery is intact L171–228 · upgrade backup cleanup L245–300
- Firefox `browser/app/profile/firefox.js` — `max_resumed_crashes = 1` ✔ · `upgradeBackup.maxUpgradeBackups = 3` ✔
- Firefox `SessionStartup.sys.mjs` L276–320 (crash detection) · `SessionStore.sys.mjs` `#needsRestorePage`
- Chromium `components/sessions/core/command_storage_backend.cc` · `chrome/browser/sessions/exit_type_service.cc` L194–205 · `startup_browser_creator_impl.cc`
- VS Code `src/vs/platform/windows/electron-main/windowsStateHandler.ts` · `src/vs/platform/state/node/stateService.ts` L141 · `src/vs/base/parts/storage/node/storage.ts` L183–315
- Windows Terminal `src/cascadia/TerminalSettingsModel/ApplicationState.cpp` L145–148 ✔ · `src/inc/til/io.h` · `src/cascadia/WindowsTerminal/WindowEmperor.cpp`

Confidence: each cell of the table was read from source by the collector (certain), while Chromium's 「실행 중 ≤3 파일」 (≤3 files while running) and the absence of fsync are inferences (likely). The collector's original text exists only in this session.

## Addendum — when the state file cannot be written (same day · light)

**Conclusion: all four apps just log and keep running.** None refuses to start or turns off session restore for that run, and none shows the user a notification. Retry is replaced by the next save cycle rewriting everything (only Chromium retries ReplaceFile 5 times, for Windows file locking). No one handles a failure to write the 「실행 중」 (running) marker separately either — Firefox records a clean exit even if the last write failed. The only place that shows the user a window is the **read side** (Chromium's dialog for a corrupted or unreadable profile).

- Windows Terminal `ApplicationState.cpp` L195 「Errors are only logged」 ✔
- VS Code `stateService.ts` `logService.error` in `doSave` L144 ✔
- Firefox `SessionFile.sys.mjs` write error → 「Could not write session state file」 + failure count tally (gecko-dev L450–492)
- Chromium `base/files/important_file_writer.cc` (retry 5 · 100 ms · warning log) · `chrome_pref_service_factory.cc` (profile error dialog = read errors only)
