# Slot docking — investigation of the means for dragging between windows (2026-10-16)

- **Status:** Conclusion = **use HTML5 DnD (built into the browser) as the base** — user decision 2026-10-16 (「내장 기능을 최대한 · 기존 것 교체」 (built-in features as much as possible · replace the existing) · drag image = the Windows default look).
- **Method:** /research medium — 3 collection tracks (crabnebula drag plugin source check · web-based peers · feasibility limited to Tauri) + main grounding + **measured spike**. ★The cross-family adversarial review was not run — the claim that splits the conclusion (whether the shell main thread gets blocked) was decided by measurement instead of literature, which replaced it (degradation label: medium — adversarial review omitted)★.
- **Confidence:** certain = confirmed by source or measurement · likely = single source · uncertain · unknown.
- Earlier investigation = `docs/research/slot-docking-survey-2026-10-15.md` §9 (the design then was title-bar dragging — this investigation looked at it again on a tab-drag basis).

## 1. crabnebula `tauri-plugin-drag-as-window` / `drag-rs` — not used (certain)

- It runs `DoDragDrop` on the Tauri **main thread** (`run_on_main_thread` in `crates/tauri-plugin-drag-as-window/src/commands.rs`). wry's drop target carries position events to that main loop through `proxy.send_event` (`tauri-runtime-wry-2.11.3/src/lib.rs:4888-4895`) — they are blocked during the drag and flood in after the drop (open issue #77). Live guides are impossible.
- Esc = fixed to cancel (`crates/drag/src/platform_impl/windows/mod.rs:72-80` `fescapepressed → DRAGDROP_S_CANCEL`) · no position during the drag (`GetCursorPos` once at the end) · what it carries is an actual temp file (dropping onto Explorer copies it) · no drag image size control · an open bug #94 where the app freezes because the drag does not end. Main checked the above three lines against the source.
- Version 2.1.1 (2026-05-01) · Apache-2.0 OR MIT · 121★.

## 2. Peers — the browser's built-in HTML5 DnD is the mainstream

| Peer | Means | Basis · confidence |
|---|---|---|
| VS Code (Electron) — dragging an editor tab to another window · detaching to a new window | HTML5 DnD · outside-window decision = at `dragend`, 「어느 창도 마지막에 dragover 를 못 봤다」 (no window saw a dragover last) (`isWindowDraggedOver` · shared via `BroadcastDataChannel`) → ask the host for the cursor position (`getCursorScreenPoint`) → new window | `src/vs/workbench/browser/parts/editor/editorTabsControl.ts:474-505` · `src/vs/workbench/browser/dnd.ts:700,757` — **main received and checked it directly (certain)** |
| Windows Terminal 1.18+ tab detaching | OLE drag and drop | spec commit · likely |
| Chromium tab detaching | native move loop where the window follows | commit 269f665 · certain |
| dockview (3.4k★) Tauri demo | HTML5 DnD · `disable_drag_drop_handler()` on every window | measured only on Linux (WebKitGTK) · certain (demo README · source) |
| tauri-explorer · block/buzz-app PR #102 · base-studio-code | direct pointer tracking (buzz = Rust decides the window from screen coordinates + an always-on-top click-through ghost window — the same shape as the earlier design) | all three said 「HTML5 는 창 밖으로 못 나간다」 (HTML5 cannot leave the window), but each was with the wry handler on · macOS · unmeasured — weak as evidence |
| Unreal Editor | its own UI framework (Slate) does dragging, windows, and docking directly | the model of the earlier design |

## 3. Tauri v2 + WebView2 feasibility

- With `dragDropEnabled=false` (config) or `disable_drag_drop_handler()` (builder), wry does not attach its own `IDropTarget`, so the WebView2 default drop remains (`wry-0.55.1/src/webview2/drag_drop.rs:45-75` · `mod.rs:150-157` · Tauri docs 「Windows 에서 HTML5 DnD 를 쓰려면 꺼야 한다」 (to use HTML5 DnD on Windows it has to be turned off) (back-translated) `tauri-2.11.3/src/webview/mod.rs:971-976`). Certain.
- ★Our code: main is already off (`src-tauri/tauri.conf.json:21`) · the runtime popout (`src-tauri/src/commands/popout.rs:150` `WebviewWindowBuilder::new`) does not turn it off — zero calls to `disable_drag_drop_handler` in the repository★ (main checked). HTML5 DnD is blocked in the popout → one added line frees it.
- What turning it off loses = Tauri `DragDropEvent` (our code does not use it — grep 0) · wry's `AllowExternalDrop(false)` → dropping an OS file onto a page that has it turned off lets the page navigate, so a global `preventDefault` may be needed (unchecked).
- Chromium drag image limits: element image 8192px per side · `<img>` 200×200 DIP (non-Mac) · default alpha 0.75 — Blink source (likely).

## 4. Measured spike (certain — measured · details = `.claude/handoff/attachments/spike-html5dnd-2026-10-16.md`)

Environment: tauri 2.11.3 · WebView2 Edg/155 · RDP session · DPR 1.25 · 22 drags.

- **The shell main thread is not blocked** — `DoDragDrop` runs on the WebView2 browser-process UI thread (confirmed by capture and ownership of the drag image window). `run_on_main_thread` delay max 4.1ms (usually 1.4) · emit arrival p99 11 · max 33ms · invoke round trip p50 2.2 · max ~50ms.
- Target window dragover 60/s (while moving) · ~15/s (while stopped) · coordinates exact (`inner_position + client×scale` = physical cursor) · custom MIME 12/12 · dragend fires in every case (drop move · outside window none · Esc none) · reading the shell key state right after dragend tells Esc from a drop outside the window · it continues even if the source window is hidden during the drag.
- The drag image = the Windows shell look (translucent · feathered edges) · it disappears over other apps (not a drop target).
- In places that are no webview (gaps · desktop) there are no JS events — if needed the shell reads the cursor (the main thread is free).

## 5. Conclusion and impact

- Use HTML5 DnD as the base — the engine takes over cursor tracking · window decision · the following image · button reading. The shell polling loop · topmost-window decision · badge window · most OS functions drop out (TRD edition 5).
- Steps 1 · 2 · 3 · 5 already built (tree operations · docking model · closing window · decision) are on the drop and apply side, so they stay. The step 6 state machine is to be replaced.
- Platform: the method is a web standard, so platform-specific OS code shrinks. Cross-window behavior on macOS (WKWebView) is unknown — measure separately when porting.

## 6. Limits and gaps

- Unchecked: dropping on the real Explorer desktop · file drop (handler off) · mixed DPI · multiple monitors · other WebView2 environments · console (non-RDP) session · small sample (hide 2 · outside-window Esc 3).
- Telling Esc from a drop outside the window relies on the key state right after dragend (watch the timing).
- A broad survey of web-based peers (Theia · Obsidian · terminal apps) was narrowed to Tauri only at the user's instruction and was not run.
