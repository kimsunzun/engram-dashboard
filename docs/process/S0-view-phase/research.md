# Engram Dashboard — Technical Research Results

*2026-06-01, 7 subagents researching in parallel (206K tokens)*

---

## 1. UX patterns worth stealing

**AgentHub's FileWatcher → reactive tree**
Agent state is managed as files, chokidar detects changes → the Zustand store updates → Tree/Panel update automatically. No DB; the files are the source of truth. Fits naturally with Engram's `orchestra.md`-based architecture.

**Claude Squad's yolo/auto-accept mode**
A toggle for "run automatically without human confirmation". Paired with the Accept button of the Monaco DiffEditor at the bottom of the dashboard — with the toggle ON the diff is only displayed and accepted automatically; OFF is a manual review gate.

**VoltAgent's 4ms coalesce pattern**
Gather PTY stdout for another 4ms after the first byte arrives and write it to xterm at once. Bundles a burst into one chunk to prevent render frame drops.

**OpenHands' real-time accumulated_cost display**
A token/cost counter in each agent panel header. The overall total is also shown in the status bar.

**AgentsRoom's Dynamic Island-style floating status**
A global status widget that always floats on top (how many running / errors present). Implemented as a bar fixed at the very top of the dashboard.

**Composio AO's CI failure → automatically focus that agent**
On error detection, the tree node turns red and focus moves automatically to that panel. Triggered when the PTY onExit code is not 0.

---

## 2. Final tech stack recommendation

| Layer | Choice | Reason |
|---|---|---|
| Desktop wrapper | Tauri v2 | fixed |
| UI framework | React 19 + TypeScript | fixed |
| Build tool | Vite | Default of the official Tauri template |
| Styling | TailwindCSS v4 + shadcn/ui | Manage agent state colors with custom color tokens |
| State management | **Zustand** | 3KB, works naturally with the FileWatcher pattern, an official `@tauri-store/zustand` exists |
| Agent tree | **react-arborist** | Virtualization by default, `openAll/closeAll` API, MIT |
| Terminal | @xterm/xterm + react-xtermjs | Isolate multiple instances with the `key` prop |
| Panel split | **allotment** | Derived from the VS Code codebase, declarative dynamic panes, nested splits possible |
| Diff editor | @monaco-editor/react DiffEditor | fixed. loader.config local bundling is mandatory |
| PTY management | **portable-pty (Rust directly)** | Multiple instances + Windows ConPTY stability. tauri-plugin-pty supports a single instance only |
| Rust→TS types | **tauri-specta** | Auto-generates TS types for invoke signatures |
| Panel resize detection | ResizeObserver (direct) | Connects allotment onResize + fitAddon.fit() |

---

## 3. Expected implementation difficulty & risks

| Component | Difficulty | Main risk |
|---|---|---|
| Basic Tauri setup + capabilities | Low | A missing v2 ACL file blocks every invoke |
| PTY spawn (single) | Low | Missed Windows cwd backslash conversion |
| **PTY multi-instance management** | **High** | ConPTY simultaneous create/destroy race condition. Serializing with a global Mutex is mandatory |
| xterm.js React multi-instance | Medium | Strict Mode effect running twice, dispose timing |
| allotment dynamic panels | Low | FitAddon.fit() must be called again when a pane is added |
| react-arborist tree | Low | Watch the function deriving the data prop from the Zustand store |
| Monaco DiffEditor (inside Tauri) | Medium | CDN loading is blocked. Without local bundle setup, an empty screen offline |
| Monaco Accept/Revert | Medium | getLineChanges() is null right after mount. Callable only after onDidUpdateDiff |
| PTY → xterm streaming | Medium | Matching the Channel type, implementing the 4ms coalesce buffer |
| Full Windows build | Medium | WebView2, orphan processes without a ConPTY Job Object |

---

## 4. Starting order

**Phase 1 — Skeleton (1~2 days)**
1. `npm create tauri-app` → React + TypeScript
2. capabilities/default.json minimal permissions
3. Vite + TailwindCSS + shadcn/ui
4. Install tauri-specta, confirm type generation with a dummy invoke

**Phase 2 — Single PTY instance (2~3 days)**
5. `portable-pty` Cargo dependency
6. `pty_open` / `pty_write` / `pty_resize` / `pty_close` commands (single session)
7. Connect xterm.js, confirm Channel streaming
8. Confirm the Windows cwd backslash problem is solved

**Phase 3 — Multiple PTYs + panels (2~3 days)**
9. `HashMap<u32, Arc<Session>>` multi-session structure
10. Global ConPTY Mutex
11. allotment dynamic n-way split panels
12. Tie PTY spawn/close to panel add/remove

**Phase 4 — Agent tree (1~2 days)**
13. Zustand agentStore (id, name, status, ptyId)
14. react-arborist NodeRenderer per-state icons/colors
15. Rust `agent-status` emit → listen → store → tree update

**Phase 5 — Monaco DiffEditor (1~2 days)**
16. `loader.config({ monaco })` local bundle
17. `vite-plugin-monaco-editor` language worker specification
18. Accept/Revert buttons after `onDidUpdateDiff`

**Phase 6 — UX polish (1 day)**
19. Global status bar (running count, error presence)
20. yolo mode toggle (diff auto accept)
21. On abnormal exit, tree node turns red + focus moves

---

## 5. Easy-to-miss pitfalls

**Windows ConPTY race condition** ← the most dangerous
`portable-pty` known issue (#356) — the console breaks when openpty and drop overlap.
Serializing both spawn and drop with a global `static CONPTY_LIFECYCLE_LOCK: Mutex<()>` is mandatory.
Without it, the terminal breaks at random when 2 or more agents start at once.

**pty_close blocking**
On Windows `ClosePseudoConsole` blocks waiting for the conhost drain.
Session drop must run on a separate thread so the Tauri command thread is not blocked.
→ the `thread::spawn(move || drop(session))` pattern.

**cwd slashes**
`CreateProcessW` rejects a forward-slash cwd. `cwd.replace('/', "\\")` is mandatory in Rust.

**xterm.js Strict Mode effect running twice**
In development the effect runs twice → `terminal.open(container)` is called twice → the terminal breaks.
An early-return guard is mandatory when `termRef.current` exists.

**Monaco CDN blocking**
If `loader.config({ monaco })` is not run immediately at the App entry point, it tries a CDN request.
If the Tauri CSP blocks it, an empty screen and no error message.

**getLineChanges() null**
Calling it right in DiffEditor onMount gives null.
You get a valid result only if you subscribe to the `editor.onDidUpdateDiff(() => { ... })` event and call it after that.

**allotment + FitAddon integration**
A `window.resize` event does not fire on an allotment drag resize.
Attach `onVisibleChange` + `ResizeObserver` to each xterm container to sync cols/rows.
Without it, line wrapping goes out of alignment.

**Monaco bundle size**
2~5MB after gzip. `languageWorkers: ["editorWorkerService"]` alone makes the diff view work.
Including the TypeScript worker adds +40%.

**tauri-specta initial setup**
If added later, all existing invokes must be reworked. Set it up from the start.
