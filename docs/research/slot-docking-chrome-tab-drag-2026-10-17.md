# Investigation — slot docking with Chrome-style tab dragging (inside the tab bar = live reordering · outside = detaching)

- **Status:** Draft — before the adversarial review (medium · design-decision mode). Awaiting decision (user).
- **Date:** 2026-10-17
- **Trigger:** User demand 「크롬처럼 해」 (do it like Chrome) (2026-10-17). The current implementation (TRD 5.6 · step 7, uncommitted) shows a 「떼어진 표시」 (detached indicator) (header folded) plus a vertical bar at the insertion point from the very start of the drag, while in Chrome the tab follows the cursor inside the tab bar and neighboring tabs move aside, and the tab is detached only once it leaves the tab bar.
- **Method:** 4 collectors (direct peers · OS drags the shell starts · web-side techniques · direct source reading as a supplement) → main grounding (2 direct checks of load-bearing sources) → one cross-family adversarial review.
- **Confidence legend:** certain = primary source + independent cross-confirmation · likely = one primary source (supported by grounding) · uncertain = unchecked · inference.
- **Earlier investigations:** [slot-docking-survey-2026-10-15.md](slot-docking-survey-2026-10-15.md) · [slot-docking-drag-mechanism-2026-10-16.md](slot-docking-drag-mechanism-2026-10-16.md) (HTML5 drag and drop adopted · crabnebula drag plugin · shell polling drag rejected).

## Conclusion

1. **Chrome's own web-technology tab bar (the touch-mode WebUI tab strip) does this with the same means as ours** — HTML5 drag and drop alone carries one gesture to the end. Inside the tab bar, on every `dragover` it **moves the actual tab element in the DOM** to that position (`placeTabElement`) to show live reordering, and when it leaves the tab bar (`dragleave`) it hides that element (`setDraggedOut(true)`); moving between windows and opening a new window are handled by the browser's native drag, which carries the tab id (a custom data format). The order change is committed only on `drop`, and if the drag ends without a drop, it returns to the original position. — **Likely** (checked directly against the source · tag 120.0.6099.0 · that folder was later deleted from main — the reason for the deletion is unchecked). Source: `chrome/browser/resources/tab_strip/drag_manager.ts` (L66 · L122 · L193 · L302 setDragImage · L321 setDraggedOut · L496-500 HTML5 listeners) — https://chromium.googlesource.com/chromium/src/+/refs/tags/120.0.6099.0/chrome/browser/resources/tab_strip/drag_manager.ts
2. **No web-based precedent was found that does pointer-based reordering and then switches to an OS drag outside the tab bar** — uncertain (not evidence of absence, but not found). On the web side alone there is no API to start an OS drag in the middle of a gesture (HTML5 `dragstart` fires only from a native gesture · once it starts, pointer events are cut off after `pointercancel`) — likely.
3. **There is no off-the-shelf way for the shell to start an OS drag on our behalf** — Electron `webContents.startDrag` takes file paths only (`file`/`files` + `icon`) — certain (checked directly against the docs). The crabnebula `drag` crate does not support the `Data` kind on Windows (files only) — likely. Building it ourselves, a route that does `DoDragDrop` from an STA thread with an IDataObject carrying Chromium's custom data format (`"Chromium Web Custom MIME Data Format"` · pickle) is open in principle, but whether it works in the middle of a gesture while the webview holds the mouse, and whether WebView2 accepts a pickle made by another process, is **unchecked** (needs a spike). On macOS there is a precedent where Electron does `beginDraggingSession` with a synthetic NSEvent — likely.
4. **VS Code is not Chrome-style** — HTML5 drag and drop · only a CSS border (an insertion marker) on the neighboring tab · tabs do not move · detaching is decided at `dragend` by 「어느 창 위도 아니었나」 (was it over no window at all) — likely (source `multiEditorTabsControl.ts` L1297 · L1441-1456 · L1371-1405 · `dnd.ts` L785).

## Options

| | Approach | Looks like Chrome? | Cost and risk | macOS | Reuse of existing code (6a-7) |
|---|---|---|---|---|---|
| **A** | **Keep HTML5 alone · Chrome WebUI tab bar shape** — inside its own tab bar it does not fold the header, and on every `dragover` it moves that header to the insertion point (neighboring tabs slide by CSS) · when it leaves the tab bar, then the 「떼어진 표시」 (detached indicator) · when it comes back in, it reverts · dropping on its own tab bar changes the order to that position | Live movement inside the tab bar = yes · while dragging, the OS drag image follows the cursor even inside the tab bar (Chrome WebUI is the same) | Small to medium · frontend (TabBar · dockDrag's own-tab-bar handling) + shell decision (own-tab-bar drop · fix the defect of detaching while stopped) · all existing means | Same (web standard) | Nearly all — other-window targets · guides · shell ledger · executor unchanged |
| **B** | Pointer-based reordering + the shell starts an OS drag the moment it leaves the tab bar (custom IDataObject · Chromium custom data pickle · STA thread · macOS synthetic NSEvent) | Closest to Chrome (no OS image inside the tab bar) | Large · three unchecked risks (starting mid-gesture · accepting an external pickle · event loop blocked — the earlier investigation rejected the plugin for the same reason) · a new purpose-specific function in platform (ADR-0266 procedure) | Implemented separately | Other-window target · shell ledger reused · source side · the start path is new |
| **C** | HTML5 alone · transparent drag image + an image we draw only inside our window | Inside our window, yes | Small · outside the window (desktop · other apps · gaps between windows) the drag image is not visible at all — a step back from PRD R2 (drag image) | Same | Nearly all |
| D | As it is now (insertion bar) | No | 0 | — | — |

- **Recommendation = A.** It is the way Chrome itself solved the same problem with web technology, and since it uses the same means as ours (HTML5), nothing built so far is thrown away. The one remaining difference is 「탭바 안에서도 OS 끌기 그림이 따라온다」 (the OS drag image follows even inside the tab bar), and that is the same as the Chrome WebUI tab bar.
- **What to check for A (measured):** whether moving the dragged element in the DOM during the drag breaks the drag (Chrome WebUI does it, and in our S1 ④ the drag continued at 306/307 even when the header was folded after the start — it breaks only when the header disappears at the pressed point right after `dragstart` · so do the move after the start signal) · whether the slide animation (FLIP) stutters at 60/s `dragover`.

## Rejection candidates (material for the ADR rejected alternatives)

- **B now** — three unchecked risks · a new OS function · the earlier investigation rejected the same axis (the shell drives the drag). If A meets the user demand, there is no reason to open it.
- **C** — the drag image outside the window disappears, which is a step back from PRD R2.
- **The Electron `startDrag` shape** — it is files only, so it cannot carry a tab id (we are on Tauri, so it does not exist for us in the first place).
- **`Data` in crabnebula `drag`** — not supported on Windows.

## Gaps and limits

- The direct peer survey is thin — Hyper · Theia · Min · Notion · Discord · Slack · Figma · Arc not reached. Among web-based apps, the only one confirmed from source as Chrome-style is the Chromium WebUI tab bar.
- The native call path of the Chromium WebUI tab bar's 「아무 창 밖에 놓기 → 새 창」 (drop outside any window → new window) is unchecked (no caller of `DropTabsInNewBrowser` found) · the slide CSS is unchecked · the reason that folder was deleted from main is unchecked.
- B's three risks cannot be closed by literature — if chosen, a spike comes first.
