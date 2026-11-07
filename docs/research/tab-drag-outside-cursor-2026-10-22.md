# The not-allowed cursor when dragging a tab outside the window — is there a way to change it

- Status: confirmed — 3 adversarial-review FIX items addressed (2026-10-22) · decided in T-66 (user 2026-10-23): ④⑤ not done this time · B/C not taken — PRD `docs/process/S21-window-chrome/prd.md` R28
- Method: `/research` medium — 3 collectors (① WebView2 API · ② OS drag cursor · ③ precedents in other apps) · main grounding (local source spot checks) · cross-family adversarial review (codex)
- Confidence legend: certain · likely · uncertain · contested
- Trigger: user measurement (2026-10-22) — when a tab is dragged outside the window (desktop · another app), a 🚫 cursor appears. When it is dropped, the new window is created normally (TRD `docs/process/S21-slot-docking/trd.md` §8-5 M2 · M5).

## Conclusion

**We did not find a supported way to change the cursor while keeping the current structure (HTML5 drag — the owner of the drag = the WebView2 browser process · ADR-0296 / ADR-0297) (likely — this is not a proof of absence).** To change it, the ownership of the drag has to be brought to our side, and there are three ways, all large.

| Way | What | Cost | Confidence |
|---|---|---|---|
| A. WebView2 `DragStarting` | the host takes over the drag, runs `DoDragDrop` with its own `IDropSource`, and decides the cursor through `GiveFeedback` | **visual hosting (composition) mode only** — wry uses only the window-mode controller. Needs a wry fork or a self-made composition hosting (DComp · input · `IDropTarget` forwarding) + webview2-com 0.39 or later. There are open regression issues | certain (API) · likely (cost) |
| C. The shell calls `DoDragDrop` directly (the way the adversarial review found) | the web blocks the start of the HTML drag, and through a Tauri command the shell opens an OLE drag with its own `IDataObject` (private format only · empty data) + its own `IDropSource` — the cursor can be decided through `GiveFeedback`, and the label through the drag image helper (inference). No need for `DragStarting` or composition | **it reopens the first item of ADR-0296 「거부한 대안」 (Rejected alternatives) (Rust native OLE source — 「덜어내고」 (leaving it out) · not done in the 1st pass)** and also collides with 「끄는 동안 셸 명령 0」 (zero shell commands while dragging) (ADR-0297). The precedent `crabnebula-dev/drag-rs` opens a native drag from Tauri/wry, but its Windows implementation returns the default cursor and, even in the empty-data branch, exports the current folder as shell data — so as is it cannot meet the requirement and would have to be modified | likely |
| B. Chrome-style direct window move | do not use the OS drag; capture the mouse and move an actual window through the OS move loop | a redesign in the direction of the old engine that ADR-0296 rejected and U4c′ removed. It overlaps with T-66 (removing the title bar · dragging the last tab moves the window) | likely |
| ✕ Carry a harmless format | carry a format that the desktop accepts, to change the cursor | the formats the shell accepts (files · virtual objects · URLs, etc.) are actually consumed on drop — we did not find a suitable format that is not consumed (risk of violating M5 「새어 나감 없음」 (no leakage)) | likely |
| ✕ Swap the system cursor | `SetSystemCursor(OCR_NO)` changed while dragging and then restored | PC-wide — if the app dies, the cursor stays changed | certain (global) · likely (restore risk) |

## Basis

### ① The cursor is decided by the drag source

- `DoDragDrop` passes the DROPEFFECT returned by the target's `DragEnter/DragOver` to the source's `IDropSource::GiveFeedback`, and the source either changes the cursor or leaves it to the OLE default cursors with `DRAGDROP_S_USEDEFAULTCURSORS` — certain. https://learn.microsoft.com/en-us/windows/win32/api/oleidl/nf-oleidl-idropsource-givefeedback
- Chromium's `DragSourceWin::GiveFeedback` returns only `DRAGDROP_S_USEDEFAULTCURSORS` → if the target is NONE, the default 🚫 — certain. https://chromium.googlesource.com/chromium/src/+/HEAD/ui/base/dragdrop/drag_source_win.cc (we could not open the body of the browser-side `WebDragSource` — likely the same behavior)
- We did not find an official way for a process that is not the source to step into `GiveFeedback` — uncertain (not a proof of absence).
- There is no evidence that HTML5 `effectAllowed` / `dropEffect` changes the OS cursor outside the window — outside the window the response of the external target decides (inference · likely).

### ② There is no harmless format that the desktop accepts

- The formats that shell targets accept include `CF_HDROP` · `CFSTR_FILEDESCRIPTOR/FILECONTENTS` · `CFSTR_SHELLIDLIST` (both files and virtual objects) · URLs · printers · network resources, etc. — certain. https://learn.microsoft.com/en-us/windows/win32/shell/datascenarios · https://learn.microsoft.com/en-us/windows/win32/shell/clipboard#formats-for-transferring-virtual-objects — the documents above are examples, not the complete list of formats the Windows 11 desktop accepts (adversarial review finding). Even so, accepted formats are consumed on drop, so what we can say goes only as far as 「소비되지 않는 알맞은 형식은 찾지 못했다」 (we did not find a suitable format that is not consumed).
- `CFSTR_PREFERREDDROPEFFECT` is only an intent hint from the source and does not make the target accept — certain (the documentation describes no such effect).
- `DROPDESCRIPTION` is guidance text over the drag image, not a cursor, and the source has to call `IDragSourceHelper2::SetFlags` — certain. https://devblogs.microsoft.com/oldnewthing/20210513-00/?p=105212

### ③ WebView2 `DragStarting` (way A)

- `ICoreWebView2CompositionController5::add_DragStarting` — stable, Win32 SDK 1.0.3719.77 — certain. https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2compositioncontroller5 · spec https://github.com/MicrosoftEdge/WebView2Feedback/pull/4871
- Arguments: `AllowedDropEffects` · `Data` (the `IDataObject` that WebView2 hands over) · `Position` · `Handled` (if TRUE, WebView2 does not start the drag) · `GetDeferral` — certain. The official example does `DoDragDrop(args->Data, 자기 IDropSource, …)` and then `Handled=TRUE`. https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2dragstartingeventargs
- The window-mode `ICoreWebView2Controller` has no equivalent — likely.
- **wry is window mode only**: wry 0.55.1 `src/webview2/mod.rs:408,410` calls `CreateCoreWebView2ControllerWithOptions` / `CreateCoreWebView2Controller` and there is no composition controller creation (the main model checked in local source). The latest 0.57.0 is the same (collector check) — certain.
- In webview2-com-sys 0.38.2 (SDK 1.0.3650.58), which this repository locks, the `DragStarting` symbol count is 0 (main grep check) — certain. It exists in 0.39.1 (collector check).
- Open issues on the composition path: WebView2Feedback #5141 (control locked after a drag) · #5695 (composition — no input response after repeatedly dragging off screen · regression) — warning signs. ★#5613 is not composition; it is a drag issue in window mode + under the conditions `HostInputProcessing` · `AllowExternalDrop` (adversarial review correction) — it is not used as grounds for `DragStarting`.
- Unknown: the minimum runtime version · whether it also fires for selected-text and link drags · whether, outside the window, the host cursor takes precedence over the composition `CursorChanged`.

### ④ Precedents in other apps

- **Chrome / Edge**: do not use the OS drag for tab dragging — they use their own mouse capture + make the detached tab into an actual window and move it through the OS move loop (`RunWindowMoveLoop`) — likely (based on a commit message). https://chromium.googlesource.com/chromium/src/+/269f66586e5fdb7d7e0e5b7f2a4ef4e7d9c92023 · so the 🚫 seems not to appear, but not confirmed — uncertain.
- **VS Code (1.85~)**: the same shape as ours — HTML5 drag + an auxiliary window at the drop position on `dragend` — certain. https://github.com/microsoft/vscode/blob/main/src/vs/workbench/browser/parts/editor/multiEditorTabsControl.ts · the cursor outside the window was not confirmed — uncertain.
- The Electron author's `dnd-drag-hint` (the `dataTransfer.noFailureEffect` proposal) treats the OS failure indicator appearing in HTML5 detaching as a problem — likely. https://github.com/zcbenz/dnd-drag-hint
- We did not find a precedent of tab detaching in Tauri / WebView2.

## Relation to our structure

- ADR-0296 / ADR-0297 set 「끄는 동안 셸 명령 0」 and place the shell OLE source among the rejected alternatives — ways A · B · C all reopen that decision.
- For T-66 (removing the title bar) to do 「마지막 탭 · 탭바 빈 곳을 끌면 창 이동」 (dragging the last tab or an empty spot of the tab bar moves the window) (user 2026-10-22), a window move that captures the mouse is needed anyway — it is cheaper to look at way B together then.
- The user's remark the same day, 「탭 사이를 끌 때 도킹용 이름표(끌기 그림)가 나오는 게 어색하다」 (it feels awkward that a docking label (the drag image) appears when dragging between tabs), has the same root — an HTML5 drag image is decided once at the start and cannot be changed mid-drag. With way B or C we hold the drag image. It is decided together in T-66.

## Limits · gaps

- The cursor outside the window in Chrome · VS Code was not measured (one drag would settle it).
- No measurement of the body of `WebDragSource::GiveFeedback` (browser side) or of the minimum formats the desktop accepts.
- The implementation cost of way A (the scope of the wry fork) is an estimate.

## Adversarial review (codex · cross-family · FIX)

- **Omission — way C (the shell calls `DoDragDrop` directly)**: 「두 길」 (two ways) was a false dichotomy → added to the table above. Basis = the OLE `DoDragDrop` · `GiveFeedback` contract + `crabnebula-dev/drag-rs` (as is, does not meet the requirement).
- **Overstatement — 「셸은 파일류만 받는다」 (the shell accepts only file-like things)**: virtual objects · URLs, etc. are accepted too → lowered to 「소비되지 않는 알맞은 형식은 찾지 못했다」.
- **Misattribution — #5613**: a window-mode issue → removed from the composition grounds.
- Re-check passed: `GiveFeedback` decides the cursor · `ICoreWebView2CompositionController5::add_DragStarting` SDK 1.0.3719.77 · wry 0.55.1 / 0.57.0 window-mode controller (the builder has no composition switch) · Chrome tab drag = move loop (Edge was not checked separately).
- 「HTML 끌기를 유지한 채 쓸 지원된 덮어쓰기」 (a supported override to use while keeping the HTML drag) did not turn up in the reviewer's additional exploration either — it is not a proof of absence but 「찾지 못함」 (not found).
