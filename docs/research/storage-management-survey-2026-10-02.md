# Storage management structure survey — screen-state restore · user settings · backend storage (2026-10-02)

- **Status:** survey complete · PRD-level decisions complete (user decisions 2026-10-02 — 「0. 결정」 (0. Decisions) below) · TRD = `docs/process/S21-storage/trd.md`

## 0. Decisions (user decisions 2026-10-02 · conversation)

- **Data root = keep portable** (release `<exe>\data\` · development `<워크트리>\.engram-dev\`). The installed form (Program Files + AppData) is to be revisited after maturity or when security requires it.
- **Development data root name = `.engram-dev`** (formerly `.engram-data` — user decision 2026-10-02 · reverses decision 2 of ADR-0136 · ADR-0264). The practice = release name + `-dev` (herdr `herdr-dev` · orca `orca-dev`). `engram` is left alone so that it does not overlap in repository root, search and ignore rules, and it also matches the development identifier `com.engram.dashboard.dev`. The name is not changed — `.engram-dev` starts fresh (「옛 데이터」 (Old data) below).
- **Folder structure = component first, kind second:** `daemon\{state,run}\` · `shell\{config,state,run}\` (run only when it arises) · `webview\` · `logs\`. Rule = `config\` and `state\` are kept, and `run\` may be thrown away. Files that hold a token (`daemon.json` · `mcp-config\`) go in `daemon\run\`.
  - **Folder names = industry standard** (user decision 2026-10-02) — systemd `ConfigurationDirectory=` · `StateDirectory=` · `RuntimeDirectory=` · XDG `CONFIG_HOME` · `STATE_HOME` · `RUNTIME_DIR` (https://specifications.freedesktop.org/basedir/latest/ · systemd.exec man page). Criterion for the split: **config = preferences** (「어떻게」 (how) — if deleted, it returns to defaults) · **state = what the program remembers** (「무엇이 있나 · 어땠나」 (what exists · how it was) — the agent registry belongs here even though the user made it. Same as Docker's `/var/lib`). The daemon has no config yet (future tuning values → `daemon\config\`). The old name `store` was dropped because it is not a standard term.
  - `daemon\state\` = `agents.json` · `presets.json` · `usage_rejects.json` · `daemon\run\` = `daemon.json` · `mcp-config\` · `usage-probe\`. ~~Existing files are moved once at first boot.~~ → overturned by 「옛 데이터」 below.
- **Old data is dropped, not moved** (user decision 2026-10-02 — 「아직 데이터 정립 안 됐으니」 (since the data is not settled yet) · ADR-0264 decisions 5 · 6). New code neither reads, moves nor deletes the old flat files (release `<exe>\data\*`) and the development `.engram-data`. There is no first-boot migration, no completion marker and no lock at the old location, and the lock is the single `daemon\run\daemon.json`. Old and new binaries do not share files, so they do not interfere (that old data is invisible to the new binary is intended).
- **Webview data goes to `webview\`** (now `%LOCALAPPDATA%\<identifier>` — resolves the violation of the portable principle · resolves localStorage sharing between worktrees).
- **Settings = `shell\config\settings.json`** — values decided by people and LLMs, only what was changed from the default. Keys are namespaced (`theme.default` · `chat.style.*` …). Access is through only four generic commands (`settings.get` · `set` · `reset` · `schema`) — a setting is added by registering one line in the schema. The path by which an LLM edited `ui-settings.json` directly is abolished, and the theme section and priming of `prompts/engram-help.md` are fixed together. Only one place in the storage layer knows the file (so even if `chat.*` and others are split into a separate file later, the call sites need no change). Chat style moves from localStorage to here.
- **Screen state = `shell\state\state.json`** (name = Windows Terminal practice · `session` is avoided because it collides with agent sessions) — window → tab → split tree → slot → `content{kind, agentId / 옵션}`. An id per window that persists across restarts. Version envelope + unknown kinds allowed (ADR-0060).
- **Restore policy = always restore, ask only after an abnormal exit** (the Chrome way) — a `cleanExit` marker · Windows shutdown/logoff notifications are treated as a normal exit.
- **A theme changed on screen is also saved** (reverses the 「화면 변경 미저장」 (screen changes not saved) of ADR-0167) — global default = `settings.json` · **per-window theme = that window's entry in `state.json`** (its lifetime is the same as the window, so no boot sweep is needed).
- **Themes are referenced by name** — the built-in set (dark · light · e-ink) stays in CSS. User themes later go in `shell\config\themes\<이름>.json` (`extends` + the changed colors). Whether to unify the built-ins as JSON too is left to when theme presets start (outside this scope).
- **Slots:** inserted content = `state.json` · settings common to each slot kind = `shell\config\slots\<종류>.json` (only for the kinds that need it). A slot whose target agent is gone keeps its placement and is restored in a 「대상 없음」 (no target) waiting state.
- **The preset system is not in this scope** — only the principle that the definition used when launching an agent is on the daemon side. For the existing `presets.json`, only its place is set to `daemon\state\` (the old file is not moved), and no new preset structure is built (2026-10-02).
- **Logs = keep the top-level `logs\`** (2026-10-02) — each process has its own file. If the daemon and client split onto different machines, each machine's `logs\` keeps only its own kind.
- **Out of scope (separate items):** the problem of worktrees sharing the development Vite port (1420 fixed · reused) · work-content history (T-14).
- **Follow-up decisions of TRD edition 4 (user decisions 2026-10-02 — details = TRD §6-0 · §14-6 · §14-7):**
  - **Save scope** — kept: popout windows · tabs (order · active · name) · split tree (structure · direction · ratio) · slot content · focus · window position · size · maximized · usage-slot check · theme (global = settings · per-window = state) · chat style (settings). Not kept: tool group · thinking line · expanded waiting list · scroll pin · list selection · input draft · tray hidden.
  - ~~**The slot view mode (terminal/chat) is also kept** — move the renderer override, which lived only in frontend memory, to a shell-owned slot value (separate from the agent output format — ADR-0078).~~ → overturned (user decision 2026-10-07 · TRD §14-20 · T-51).
  - **Old values are not handled (U1)** — released builds (v0.1.0–v0.3.2) exist, but the only user is the user himself, so there is no user data to protect (user judgment 2026-10-02). `ui-settings.json` is not moved, and the shell just stops reading it. The chat-style localStorage values are not imported either. `settings.json` starts from the defaults.
  - **Chat style is kept light with only 11 keys for now (U2)** — `chat.style.*` is a temporary namespace (it will be replaced by the chat-area rewrite · plugin batch).
  - ~~**An abnormal-exit snapshot is not overwritten; the latest 3 are kept on rotation** — session default (awaiting user confirmation · TRD §10 F17). The design is rewritten and re-reviewed before P3 starts.~~ → withdrawn by D1 of edition 5 below (it was not a user decision but a default the session made up).
- **Follow-up decisions of TRD edition 5 (user decisions D1–D3 2026-10-02 — details = TRD §6-5 · §6-7 · §14-9 · precedents = `docs/research/crash-session-snapshot-precedents-2026-10-02.md`):**
  - **D1 — 「최근 3 개 회전 보관」 (keep the latest 3 on rotation) withdrawn.** It had no source and there is no precedent (Firefox's 3 is an upgrade backup · mature apps keep one generation).
  - **D2 — crash copy = a single `shell\state\state.crash.json`.** The boot after an abnormal exit copies `state.json` aside to the copy once before writing it, starts with the default screen, and asks. Restore = delete the copy only after that state has been written to `state.json` · decline = delete · if it dies again before an answer, the earlier copy is not overwritten and it asks again · an unusable file is copied aside as `.corrupt-<ms>` like settings, and it does not ask. ★→ The name to copy aside was changed by ADR-0274 — one fixed name `.corrupt` · overwritten next time (user decision F21 · TRD §14-12)★
  - **D3 — always ask after a crash (the Chromium way).** Rejected = the Firefox way (restore silently on the first crash, ask only on consecutive crashes) — dominant practice · simple · prevents crash loops.
- **Follow-up decisions of TRD edition 6 (user decisions D4–D8 2026-10-02 — details = TRD §6-5 · §6-7 · §14-10):** principle = 「그런 것까지 가정하면 끝도 없음」 (there is no end if we assume even that) — the simplest design that fills in the decisions.
  - **D4 — 「복원할까요?」 (restore it?) is a modal inside the main window webview.** The app cannot be used until it is answered · a person and an LLM answer with the same `restore.answer`. It is neither a banner nor an OS dialog (「무시하고 진행하면 이걸 또 저장해야 되냐 … 복잡하잖아」 (if we ignore it and proceed, do we have to save this too … that's complicated)).
  - **D5 — always ask after an abnormal exit · no exception for the default shape** (comparison logic is unneeded complexity).
  - **D6 — only an answer releases the copy.** Even if the app exits normally (including a tray exit) while the modal is up, the copy is kept and the next boot asks again.
  - **D7 — a `--hidden` boot is not handled separately** (the modal is inside the hidden main).
  - **F15 — the development build also always asks, with no exception** (「나중에 짜증나면 요청」 (ask for it later if it gets annoying) — TRD §10 · §14-11 N2).
  - **D8 — write failures (including the run marker) are not handled separately** — log it, and the next save writes again (`docs/research/crash-session-snapshot-precedents-2026-10-02.md` 「덧붙임」 (Addendum) — all four apps are the same).
- **Method:** research medium · design-decision mode. 3 tracks (peer apps of the same kind · Tauri/WebView2 mechanism · settings/state separation patterns) + 1 current-state inventory. Main grounding (checked against local clones and cargo registry sources) + one cross-family (codex) adversarial review (verdict BLOCK → corrections reflected below).
- **Scope:** work-content history (conversation and output records) is excluded — the T-14 track.
- **User criterion:** 「업계 표준이 우선」 (industry standard comes first) (2026-10-02). We adopt general software practice too, not limited to agent tools.
- **Confidence:** certain = primary source + independent cross-confirmation · likely = primary source alone · uncertain = unsupported.

## 1. Current state (inventory · confirmed by reading the source, not a run measurement)

There are three stores.

| | Location | Axis along which they split | What it holds |
|---|---|---|---|
| Daemon data | debug = worktree root `.engram-data` · release = `<exe>/data` (`discovery/src/lib.rs:84` · ADR-0134/0136) | worktree · dev/release | `agents.json` · `presets.json` · `daemon.json` (lock + discovery) · `usage_rejects.json` · `mcp-config/` · `usage-probe/` · daemon log |
| Shell data | the **same folder** as the daemon | same as the daemon | `ui-settings.json` (theme — the agent writes it, and the shell sweeps out only the dead window entries at boot · ADR-0166/0167) · app log. **Layout, windows and tabs are memory only** (`src-tauri/src/layout/mod.rs:26-35`) |
| Webview data | `%LOCALAPPDATA%\<identifier>` that Tauri forces through the identifier (`tauri-2.11.3/src/manager/webview.rs:534-545` — checked) | dev/release (identifier) · origin (port) — **does not split by worktree** | one chat style in localStorage (`src/store/chatStyleStore.ts`) |

- In release, **the portable principle (ADR-0134 「폴더를 지우면 흔적 없음」 (delete the folder and no trace remains)) is already broken by the webview data** — the webview folder goes to `%LOCALAPPDATA%`.
- The dev app brings up Vite `localhost:1420` (`strictPort`), and the run script reuses a Vite that is already up — the dev apps of two worktrees can see the same frontend code and the same origin (inferred from reading the config · not measured).
- Known defects: T-22 (an `agents.json` save failure is reported as success) · T-33 (one unknown backend kind makes the whole file be treated as corrupt).
- Coupling: the boot sweep of ADR-0167 presupposes 「레이아웃이 복원되지 않는다」 (the layout is not restored). ADR-0060 requires a version envelope · an `Unknown` variant · migration for layout persistence.

## 2. Findings

### 2-1. Separate settings and state by purpose — likely to certain
- The **settings** that people edit and the **state** that machines write (window arrangement · open tabs) are put in different stores: VS Code (`settings.json` ↔ `storage.json`·`state.vscdb` — https://code.visualstudio.com/docs/editor/settings) · Windows Terminal (`settings.json` ↔ `state.json` — `ApplicationState.cpp:16-17`) · JetBrains (`.idea/*.xml` ↔ `workspace.xml` — https://www.jetbrains.com/help/idea/configure-project-settings.html) · Zed (`settings.json` ↔ `db.sqlite`) · herdr (`config.toml` ↔ `session.json`, clone checked).
- ★**Correction (adversarial review):** 「두 파일 이름·두 위치」 (two file names · two locations) is not a universal rule. What is universal is the **separation by purpose** (Zed's state is SQLite·Local, and the Electron `userData` default is Roaming — https://www.electronjs.org/docs/latest/api/app).

### 2-2. Screen state is written to a file by the native side — certain (general software) · counterexamples exist (web-first agent tools)
- Written by native/server: VS Code (main-process `windowsState`) · Windows Terminal · herdr · zellij (`session-layout.kdl`) · orca (`workspaceSession` of the main-process `orca-data.json`).
- Written to the frontend web storage: t3code (`t3code:ui-state:v1` — `apps/web/src/uiStateStore.ts:6`, clone checked) · paseo (`workspace-layout-state` — `workspace-layout-store.ts:1724`, clone checked). Both are web-first structures.
- The side that matches our principle (LLM-first control · layout authority = shell, ADR-0035/0057) is the native side.

### 2-3. Restore is off by default + turned on in settings — likely
- The default of Windows Terminal's `firstWindowPreference` is `defaultProfile` (start fresh), and restoring the saved layout is turned on by the user (https://learn.microsoft.com/en-us/windows/terminal/customize-settings/startup). Chrome too has 「이어서 열기」 (Continue where you left off) as a setting, and after an abnormal exit it asks instead of restoring silently.
- Window restore needs a **window identity** — Terminal saves the window name together with the arrangement and tabs. How to handle closed windows · vanished agents · duplicate popouts has to be decided before the schema.

### 2-4. Location — the default is the OS standard folder, and portable is an option tied to the distribution form — likely
- Default: VS Code · Zed (`%LOCALAPPDATA%\Zed`) · herdr (settings `%APPDATA%` · state `%LOCALAPPDATA%`) · orca (`%APPDATA%\orca`).
- Portable: VS Code turns it on **only in the ZIP distribution** when a `data/` folder is placed (https://code.visualstudio.com/docs/editor/portable). Windows Terminal, in a **separate portable distribution**, uses a `.portable` marker → `settings/` next to the exe (https://learn.microsoft.com/en-us/windows/terminal/distributions). We did not find any agent-orchestrator peer that has a portable mode.
- ★**Correction (adversarial review):** fixing 「설정 = Roaming, 상태 = Local」 (settings = Roaming, state = Local) as the standard is an overstatement. The MS guidance is that Roaming = small preference values that should follow the user between devices, and Local = device-specific data (https://learn.microsoft.com/en-us/uwp/api/Windows.Storage.ApplicationData). Terminal also keeps `settings.json` in Local.
- ★**Correction (adversarial review):** that peers use the OS folder is not by itself grounds for changing our portable distribution — our distribution is a single ZIP, and VS Code's ZIP distribution supports portable.

### 2-5. dev/release separation = a different root name at startup + environment variables — certain
- herdr `herdr-dev` (`src/config/io.rs:22`, clone checked) · orca `orca-dev` (`configure-process.ts:153-184`, clone checked) · t3code `dev/` ↔ `userdata/` (`apps/server/src/config.ts:132`, clone checked) · Zed `0-<channel>`. Re-pointing through environment variables is common (T3CODE_HOME · PASEO_HOME · ORCA_USER_DATA_PATH · Zed `--user-data-dir`).
- **No peer isolates per worktree** — orca writes in a comment that its dev worktrees share `orca-dev`. Our `.engram-data` is more isolated than the peers. If we move to the OS standard folder, this isolation disappears without an environment variable (adversarial review finding).

### 2-6. Tauri mechanism — certain (checked against the source)
- Our versions: tauri 2.11.3 · wry 0.55.1 · webview2-com 0.38.2 (`Cargo.lock`).
- Per-window webview folder: `WebviewWindowBuilder::data_directory(PathBuf)` exists (`tauri-2.11.3/src/webview/webview_window.rs:1024`). The config key `dataDirectory` accepts only relative paths.
- Webviews with different environment options (`additionalBrowserArgs` etc.) must use different webview folders (MS — https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/user-data-folder). The popout uses `additional_browser_args` (`popout.rs:117`).
- The webview folder holds not only the cache but also cookies · permissions · **localStorage** · IndexedDB (same MS document). If the folder is moved, the chat-style value stays in the old folder. That folder cannot be deleted while a session is alive.
- ★**Fact · version correction raised by the adversarial review:** Tauri has an **app-directory bulk override setting `app > appDirectoriesOverride`** — it moves the app paths, plugins and the **default webview folder** together (https://v2.tauri.app/reference/config/#appdirectoriesoverride). The review said 「2.10 에 들어왔다」 (it came in with 2.10), but **it is not in our 2.11.3 source** (`tauri-2.11.3/src/path/desktop.rs:238-260` — it only appends the identifier). The release where this feature first appeared is **tauri 2.12.0** (tauri-utils 2.10.0) (`gh api repos/tauri-apps/tauri/releases` body search). → To use it, a **Tauri 2.12 upgrade must come first**. There is also a Tauri warning that paths relative to the executable fail in non-writable locations such as Program Files.
- Official plugins: `tauri-plugin-window-state` saves only size · position · maximized · visible · decorations · fullscreen, to `app_config_dir/.window-state.json` (the window label is the key). `tauri-plugin-store` is JSON relative to AppData. Neither is in the dependencies now. If the override setting above exists, both follow that root.

### 2-7. File robustness — likely
- Do not discard the whole file because one item is bad — Zed resets the whole settings to defaults if one value in `settings.json` is wrong (zed#12389 · #9989). Our T-33 is the same class. Parse per item, and do not overwrite a file that failed to parse.
- Atomic write: the daemon uses tmp → fsync → rename. ★**Correction (adversarial review):** on Windows this is not a complete specification — we must decide whether to use `ReplaceFileW` (including its backup behavior) and how to recover on failure (https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew).
- Save timing: saving only once at exit is weak against crashes and forced exits — Zed saves periodically with throttling, and Firefox keeps a live recovery file + rotating backups. Frequently changing values are taken out of the layout blob (zed#64823).
- When tools and people edit the same settings file: editing that preserves comments and changes only one property (`jsonc-parser`) · file-watch reload. For us, 「모든 쓰기를 같은 명령 표면으로 보내 쓰는 쪽을 하나로」 (send all writes to the same command surface so that there is a single writing side) is the natural one (inference — no source).

## 3. Decision axes and options

| # | Axis | Options | Standard basis |
|---|---|---|---|
| 1 | **Data root location** | (a) OS standard folder by default + a portable marker puts it next to the exe · (b) keep portable as the default + put the webview folder inside it too · (c) OS standard folder only | VS Code ZIP · Windows Terminal = shape (a) · our current = close to (b) |
| 2 | Screen-state ownership | the shell writes a state file vs frontend localStorage | general software = native. Frontend is only for web-first tools |
| 3 | Restore policy | off by default + turned on in settings vs always restore · ask after a crash | Windows Terminal · Chrome = off by default/settings |
| 4 | Settings file | settings (edited by people·LLMs) and state (written by machines) in different files · move chat style from localStorage to the settings file | separation by purpose (2-1) |
| 5 | Worktree isolation | keep (a root per worktree) vs share one dev root | peers share. Our isolation is stronger |
| 6 | Daemon data robustness | fix T-22 · T-33 · Windows atomic write specification | 2-7 |

- **1 determines 4 · 5:** with (a) or (c), worktree isolation has to be kept separately through an environment variable, and moving the webview folder is solved all at once by the override setting of Tauri 2.12. With (b), the current structure is kept and only the webview folder is moved per window through `data_directory` (possible in 2.11.3).

## 4. Rejected candidates (candidates for the ADR's rejected alternatives)

- **Layout in frontend localStorage** — conflicts with LLM-first control (the shell and LLM cannot read it), and it is tied to the origin and webview folder, so it is lost when dev/release or the port changes. Only the web-first peers (t3code·paseo) take this path.
- **`tauri-plugin-window-state` as is** — it saves only window geometry and does not know tabs · slots · splits. In 2.11.3 its location is fixed to `app_config_dir`, so it cannot follow the portable or worktree root (with the 2.12 override setting it does).
- **Everything in one file** — orca had to split out a side file because of frequently changing data.
- **SQLite now** — the state to save is small. The storage engine is decided in the T-14 track.

## 5. Adversarial review result (codex · verdict BLOCK)

Findings addressed: Roaming/Local overstatement · logic gap in the move from portable to the OS folder · localStorage lives in the webview folder · 「흔적 없음」 (no trace) overstatement (environment variables and policy can re-point the webview folder, and the old folder is not deleted automatically) · missing restore policy · missing window identity · logic gap in worktree isolation · Windows atomic write specification · write permission of paths relative to the executable.

**Partial rebuttal:** 「`appDirectoriesOverride` 가 2.10 에 들어와 2.11.3 에 있다」 (appDirectoriesOverride came in with 2.10 and is in 2.11.3) — the feature is real, but it is not in our 2.11.3 source (`path/desktop.rs:238-260`). The release where it first appeared is tauri 2.12.0. → We wrote it as 「업그레이드 선행」 (upgrade first).

## 6. Limits · gaps

- WezTerm · Warp · superset · Obsidian · electron-store were not examined.
- We could not find a primary source for version-envelope and migration practice.
- We could not find a source for crash-loop isolation (a mechanism that does not restore again a layout that died while being restored).
- The internal format of the webview folder (LevelDB) and the subfolder name (`EBWebView`) were not checked.
- The actual behavior when two windows use different webview folders in one process was not checked.
- The cost of the Tauri 2.12 upgrade was not measured.
