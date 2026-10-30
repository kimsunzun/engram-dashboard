# OS(HTML5) tab drag spike — progress

Env: Win11 26200, RDP session, primary 1920x1080 @125% (DPR 1.25), secondary 3200x1800 @200% at (-3200,-305).
Spike build of slot-docking 1c5ad58b + throwaway patch (spike.patch): identifier com.engram.dashboard.spikeosdrag
(user's dev app holds the dev single-instance mutex), vite :1431, CDP :9241, own data dir + own daemon (fake streaming
`claude.cmd` on the daemon PATH for S13). Real input = SendInput (S1In kit). User was concurrently active (Chrome in front)
— runs may contain stray user input; anomalies noted per item.

Layout: main outer (0,0) 1150x900 [TOPMOST during runs], tabs View 1 (split L|R, R = streaming terminal), A2, A3, A4.
Popout slot-popup-2 outer (1200,100) 700x600, tabs View 1, P2. Bare desktop at ~(1650,950). Chrome under the gap x≈1175.

- [contaminated] S14 pilot (k=0, first run): user moved the physical mouse during the run (press landed at 1237,176 vs intended 1240,171) and the main window had been moved by the user between probe and run (my TOPMOST main covered their Chrome). Discarded; re-run below. Change: no persistent TOPMOST; per-trial focus + "top window under the press point is mine" check; per-op cursor-deviation check flags contaminated trials automatically (|cur - intended| > 2 px).

## S14 drag start — verified (N=5, 0 contaminated)
- Click without move on an inactive tab: activates it 5/5, no dragstart (seq pointerdown,mousedown,pointerup,click).
- Double-click on a tab: rename input opens 2/2, no dragstart.
- Drag threshold = Chromium/OS native: dragstart fired on the 4 px (physical; 3.2 CSS px @125%) move, 5/5; 5–6 ms after that move. Not configurable to PRD's 10 px (dragstart can only be cancelled, not deferred).
- Side finding: in the threshold trials Esc (30 ms tap, no mouse motion) cancelled the drag 4/5 (dragend none 6.9 ms after Esc; shell read lbutton=down esc=down at dragend); 1/5 the Esc was MISSED and the later release dropped (dragend move). -> quantify in S4.

## S1 same-window reorder — verified with real input (2 runs x N=5) — see results.jsonl
- dragover inside the source strip while moving: 108–122/s (p50 gap 6.7–7.6 ms, p95 21–31 ms, max 47 ms) with SendInput steps every 12 ms; when the mouse is still: 13–16/s (~63 ms timer). Handler time ≤0.5 ms.
- Insertion index from dragover clientX vs head midpoints tracked the path every trial (e.g. 1→2→3→4→3→2→3→4); drop delivered in 9/10 at the strip with the index, getData(custom MIME) ok, text/plain "" , text/uri-list "", types = [application/x-engram-tab, chromium/x-drag-id] only. Release→drop 7–10 ms; dragend dropEffect "move".
- 1/10 (run 1 k=3) press landed 28 px off (user motion) -> no drop, dragend none. Contaminated, not counted.
- rAF frame gaps during the drag: p50 31.2 ms, max 31.8 ms, 0 gaps >50 ms in all 10 (≈32 fps — RDP-paced compositor; no stall). Insertion marker drawn from dragover: no extra frames lost.
- Harness note: the per-op cursor check flagged most trials, but the pattern is deterministic (e.g. every trial `G 509 → cur 517`): GetCursorPos read right after the last SendInput of a glide lags by 1 step *during an OLE drag* (not seen outside a drag). Only the press-time deviations (run 1 k=0, k=3) were real external motion.

## STOP real input (coordinator order, after S1 run 2)
Real-input trials that ran before the stop: S14 pilot (contaminated, discarded), S14 x5 + dblclick x2, S1 run1 x5, S1 run2 x5, plus a cursor-stability probe (cursor parked at 600,500 for 3 s). No real input after this line. Input-driving processes checked: none alive.
Everything below = CDP in-page simulation (no OS cursor), code reading, logs.

## Real-input evidence re-read from S14/S1 raw logs (shell read at dragend)
- Shell `GetCursorPos` read right after dragend (invoke issued 0–0.1 ms after the dragend handler) = the exact release point (physical px) 10/10 (5 drops + 5 Esc ends, inside our window). dragend screenX/Y are DIP (e.g. 567,44 for phys 709,55 @1.25).
- Esc vs drop discriminator (real input, inside our window): Esc ends had lbutton=DOWN and esc=DOWN at the read 4/4 (30 ms later: esc up, lbutton still down); drops had lbutton=up esc=up 5/5. Esc→dragend 6–8 ms. But 1 Esc of 5 was missed by the OS drag loop (S14 k=4) and the later release became a drop (move).

## S2 cross-window — CDP simulation only (OLE delivery untested in this run)
- Input.setInterceptDrags + page mouse events -> dragstart on source, intercepted DragData items = [application/x-engram-tab] only, operations mask 16 (move). Threshold in the simulated path: dragstart at 4 CSS px (not at 3).
- dispatchDragEvent into the other window's page: dragenter types = [application/x-engram-tab]; insertion index tracked (main->pop "21", pop->main "4321"); drop getData = {"view","label"} JSON, text/plain "", uri "", files 0. 6/6 both directions. Handler ≤0.3 ms.
- Source dragend does NOT fire under CDP interception (artifact) — real OLE dragend/dropEffect across windows = untested here (prior spike 2026-10-16: move 12/12).

## S6 dock-band preview — CDP simulation (dragover at ~60/s injected; OS dragover rate measured in S1 real = 108–122/s moving, ~16/s still)
- Popout P2 dragged over main View 1 (split 453x655 CSS each, right = streaming terminal) 3 runs + over A3 (one slot 906x655) 2 runs: 235/123 dragovers per sweep, 10% band decision matched geometry 100% (0 wrong of 1179), handler p50 0–0.1 ms, max 0.2 ms. Drop in the right band delivered {zone band, side right, slot id, data}.
- Overlay (fixed div moved from dragover) visible in a CDP screenshot mid-drag (shots/S6-band.png: left-half preview on the left slot while the terminal streams). rAF gaps p50 31.2 ms, 0–1 gap >50 ms (the one = during the screenshot capture). No flicker evidence (overlay only re-styled when the zone changes is not even needed — re-styling every dragover cost ≤0.2 ms).
- S13 (stream vs no stream): identical frame stats (p50 31.2 / max 31.8) and handler time; real-input S1 trials with View 1 active (terminal streaming ~500 lines/s visible) also showed the same dragover rate and 0 frame gaps >50 ms.

## S12 same window — CDP simulation
- There is ONE tab strip per window in this app (no per-pane strips), so "another pane's strip" does not exist. Measured instead: drag main's inactive tab A2 over main's own canvas. Press activates A2 (rule R2a), so the canvas under the drag is A2's OWN layout (1 slot) — bands computed correctly (0 wrong / 246) and drop delivered (band right of A2's own slot), but the other tab's layout (View 1) is not reachable in the same window. Chrome-style showed the previously active tab after detach (R15c/AC14/AC41); with OS drag nothing detaches -> needs a web-local "show previous tab while dragging" or rule change.

## S10 OS file drag — CDP simulation (DragData.files) only; real Explorer OLE drag untested
- main and popout: types [Files], fileDropGuard sets dropEffect none -> no drop event, no navigation (href unchanged, no beforeunload) 2/2. Guard only reacts to Files/text/uri-list, so tab drags (custom MIME only) are untouched (S1 real drops delivered).

## S11 — code-verified + partial runtime
- All three window builders (state/placement.rs from_config for main, commands/popout.rs runtime popouts + spare, commands/dock.rs overlay) go through WebviewEnv::finish -> attach() -> .no_drop_handler() (webview_env.rs). Real OLE HTML5 events arrived in main (S1 real). Real OLE into a popout: untested in this run (prior spike 2026-10-16: runtime window with disable_drag_drop_handler received dragenter/over/drop).

## S8 single-tab window / S9 main's last tab — CDP simulation, events only
- S8: new popout slot-popup-3 with 1 tab: dragstart fires normally (tabCount 1), drop on main strip delivered with data. S9: main reduced to 1 tab (View 1), drag to popout strip: dragstart (tabCount 1), drop delivered idx 2.
- An HTML5 drag cannot move the window during the drag (the OS drag loop owns the mouse; only the drag image follows). The visible rule "the whole window follows the cursor" therefore needs a different gesture (e.g. cancel the HTML5 drag and call startDragging/move loop) — that path then gets no dragover in other windows (no bands/attach). Real-input check of startDragging from dragstart: untested.

## S7 drag image — one real capture before the stop (shots/S1-strip-mid.png)
- Label image (35.8x25.6 CSS = ~45x32 phys, setDragImage offset -10,-10) is drawn by the OS next to the cursor inside the strip, semi-transparent dark box with the tab name; our insertion marker visible at the same time. Dotted-box cursor, image over desktop/other apps, mixed-DPI scaling: untested (needs real input; CopyFromScreen does not capture the cursor).

## Untested — needs real input (manual run later)
S2 OLE cross-window (dragend dropEffect at source, popout receipt), S3 desktop drop (dragend none timing incl. snap-back animation, cursor read after it), S4 Esc vs outside drop (N≥10; Esc missed 1/5 already seen; Esc held 100 ms vs 30 ms; Esc over desktop), S5 drop onto Notepad/Explorer/browser (no text/file leak — intercepted DragData shows only the custom MIME, so leak is unlikely), S7 visuals (dotted box, other apps, DPI), S8 startDragging variant, S10 real Explorer file drag, mixed DPI (secondary 3200x1800 @200% exists on this PC).
