# External events during a drag — practice survey (cancel · defer · leave it alone + re-verify at drop)

- **Status:** final (medium · design-decision mode · codex adversarial review, 3 FIX items incorporated) · this report does not itself decide — it is for grasping practice (user instruction 2026-10-20 「한다는게 아니라 관행을 일단 보라고」). The decision the user chose after this survey = **ADR-0297** (leave it alone + look up again at drop)
- **Date:** 2026-10-20
- **Method:** three tracks (A direct peers · B means of cancelling an in-progress OS drag · C the real risk of IPC during a drag and the basis of our invariant) collected → main grounding → codex cross-family adversarial review
- **Confidence legend:** certain (independently cross-confirmed) · likely (grounds support, single source) · uncertain · contested
- **Trigger:** the U3 code review (2026-10-20) caught calls that leak outside 「끄는 동안 셸 호출 0」 (zero shell calls during a drag) (ADR-0296 · TRD edition 7 §2-2) (window-frame lookup · canvas-size report). The user's question — if an event occurs during a drag, wouldn't cancelling the drag eliminate all issues (gist — not the original text) · 「끌기 때문에 메인에 있는걸 멈추면 모든 예외사항들을 추가해야되서」 (original text).

## Conclusion (summary)

1. **The mainstream of practice is 「그냥 둠 + 놓을 때 id 로 다시 찾아 없으면 무동작」** (leave it alone + look it up again by id at drop, no-op if missing) (the exception = dockview — if not found it throws instead of doing nothing). Among the peers surveyed, we found no global block that wholly blocks or defers model · layout · window-size changes during a drag. There are local exceptions — Chromium blocks tab-bar editing during a drag (a narrow block) and defers closing the empty source window of a touch drag until the drag ends, and golden-layout detaches the item from the tree at drag start so others cannot touch it (uncertain — inference) (table A). (likely — five peers have the same form in source · only dockview differs in handling 「없으면」 (when missing))
2. **A peer that ends a drag midway ends it only when the dragged item or its tab bar (container) changes — Chromium = the dragged tab is removed · a tab is added during a window drag (forced completion) / Lumino = a tab is added to or removed from that tab bar, or its orientation changes (abort). No peer ends one because of unrelated events such as IPC or window size.** It does so only in implementations that have their own drag loop. The way of ending differs — the Chromium tab bar does a **forced completion** (`EndDrag(kComplete)` — not reverted · cancel `kCancel` is a different path that reverts via `RevertDrag`), the Lumino tab bar does **abort + restoring the position** (`_releaseMouse`). The reason is 「복잡한 특수 경우를 피하려고」 (to avoid complex special cases). (likely)
3. **Deferral is rare and local** — not a global queue, but only specific risks (destroying the window holding the drag · losing capture) are deferred (Chromium). **We could not find a peer that blocks saves · IPC during a drag** (not found ≠ none).
4. **Our 「끄는 동안 셸 호출 0」 is a design choice, not a technical constraint** — the grounds are the old engine's synchronization defects (ADR-0296:8 · :37), and our spike measured that invoke · emit are normal during a drag (a small sample). (likely)
5. **There is no way for web code to cancel an in-progress HTML5 drag. There is no supported way from the shell (Tauri/wry window hosting) either.** Esc injection is unchecked (a spike is needed). (web side = likely · shell-side Esc = uncertain)

## A. Direct peers — external events during a drag

| Peer | Form | In response to what | Basis | Confidence |
|---|---|---|---|---|
| **Chromium tab bar** (`TabDragController`) | Mixed | The dragged tab is about to be removed → ends the drag **as complete** (`EndDrag(kComplete)` — not reverted · 「End the drag before we remove a tab that's being dragged, to avoid complex special cases」) · someone adds a tab during a detached-window drag → ends as complete (「not worth the complexity」) · another tab removed · replaced → only tracked (drag continues) · window size · position change → let it pass and decide again · closing the empty source window of a touch drag → deferred until the drag ends · tab-bar editable = false during a drag (narrow block) | Local copy `.claude/handoff/attachments/chromium-src-2026-10-18/tab_drag_controller.cc:732 · :742 · :2117-2128` · `tab_strip.cc:1399` (main checked against the original text) · two local exceptions = blocking tab-bar editing `IsTabStripEditable` `tab_strip.cc:1298-1301` · deferring the close of the empty source window of a touch drag `tab_drag_controller.cc:651-669 · :1620-1623` (same copy · lines checked 2026-10-20) | Likely |
| **VS Code** (editor · group DnD) | Re-verify at drop | Looks up the source group again by id and does nothing if missing · a move between groups checks `canMove` again at drop | `src/vs/workbench/browser/parts/editor/editorDropTarget.ts` · `editorGroupView.ts` (main) | Likely (an editor closed during a drag = uncertain) |
| **dockview** | Re-verify at drop (throws — the only peer different from the mainstream's 「무동작」 (no-op)) | Looks up the group · panel by id and throws an exception if missing · the host can block with `onWillDrop` | `packages/dockview-core/src/dockview/dockviewComponent.ts` (master) | Likely |
| **FlexLayout** | Re-verify at drop (no-op) | `applyMoveNode` looks up by id and does nothing if missing or the type is wrong | `src/model/Model.ts` | Likely |
| **Zed** | Re-verify at drop (no-op) | 「// Tab was closed during drag」 → `return` | `crates/workspace/src/workspace.rs:12257` (main · main checked against the original text) | Likely |
| **Lumino (JupyterLab · Theia)** | Tab bar = abort · dock = re-verify at drop | Before that tab bar changes, 「Release the mouse before making any changes」 → abort the drag — orientation change (`orientation` setter `:332`) · tab added (`:431`) · removed (`:523`) · all cleared (`clearTabs` `:558`) · a dock drag re-evaluates the target at drop | `packages/widgets/src/tabbar.ts:332 · :431 · :523 · :558` (main · main checked against the original text · `:558` = added by the doc review 2026-10-20) · `dockpanel.ts` | Likely |
| **golden-layout v2** | Detach at start · cache at drop | At drag start detaches the item from the tree so others cannot touch it · the areas are counted once at start | `src/ts/controls/drag-proxy.ts` | Uncertain (the outcome is inference) |
| **Firefox tab** | Null check at drop + stale-drag watch | If the drag session is gone or the button is released, force-ends with `endDragSession(false)` | `browser/components/tabbrowser/content/drag-and-drop.js` (main) | Uncertain (behavior when the dragged tab is removed = unknown) |

- **Could not judge:** JetBrains · Obsidian (closed source) · Electron multi-window apps such as Tabby/Hyper · Zed tab dragging between windows.
- **Pattern:** the drag payload is mostly an id · handle, so it is looked up again at drop, and if stale it becomes a no-op on its own.

## B. Means of cancelling an in-progress HTML5 drag (Chromium · WebView2 · Windows)

| Means | Does it cancel | Basis | Confidence |
|---|---|---|---|
| Remove · hide the source element (after the start) | No — it is looked at only once right after dragstart | Blink `mouse_event_manager.cc` · `drag_controller.cc` | Likely |
| `dropEffect = 'none'` · not accepting dragover | Partial — only the drop fails, the drag continues until the button is released · dragend `none` | HTML standard DnD processing model | Likely |
| JS synthetic Esc | No — an untrusted event does not reach the OLE loop | `ui/base/dragdrop/drag_source_win.cc` | Likely (inference) |
| `drag` event preventDefault | In Chromium, no — the return value is not looked at (「we don't care if event handler cancels default behavior」) · **the standard leaves room to read cancellation as aborting the drag** (review finding — the collector read it as 「작업을 none 으로」 (set the operation to none)) | Blink `event_handler.cc` comment · HTML standard DnD | Chromium behavior = likely · standard interpretation = contested |
| Inject Esc from the shell (SendInput · PostMessage) | **Unknown** — Windows ole32 is closed source · it may need keyboard focus | Only the Wine reference implementation (not Windows grounds) | Uncertain — a spike is needed |
| WebView2 `CompositionController5.DragStarting` | **Not** a means of cancelling an in-progress drag — it is a route where the host intercepts (`Handled`) WebView2's **first** `DoDragDrop` and runs it itself, and cancelling afterward is done by the host's own `IDropSource` (`DRAGDROP_S_CANCEL`) (a different mechanism). Composition hosting only · wry uses window hosting so it cannot use it | MS Learn · `IDropSource::QueryContinueDrag` · wry-0.55.1 `src/webview2/mod.rs` | Likely |
| Synthetic mouse release | It ends as a drop, not a cancel (+ if dragover rejects, the result is 「놓기 없음」 (no drop)) | `drag_source_win.cc` | Uncertain |

- Note: in our measurement (TRD §9-2 · S14), even a quick Esc typed by a human was missed 1 time in 5.

## C. The real risk of IPC during a drag · the basis of our invariant

- **Our basis (primary source):** ADR-0296:8 — shell ↔ webview boundary defects recurred in the old Chrome-style engine (cause confirmed 1 · unconfirmed 2). 「공통 뿌리 = 한 끌기를 두 프로세스가 판정하고 끄는 동안 상태 · 기하를 IPC 로 맞춤」 (common root = two processes decide one drag and sync state · geometry by IPC during the drag). ADR-0296:37 — 「끌기 도중 셸 호출 0 … 되살리면 이 결정의 이유(경계 동기화 결함)가 돌아온다」 (zero shell calls during a drag … if revived, the reason for this decision (boundary sync defects) returns). TRD §4-7 deferred `resizePty` because of that invariant (at the time of the survey) (it did not claim a fix for the size sync itself · lifted by ADR-0297). **Nature = a design choice + inherited from the old engine.**
- **Our counter-measurement:** `docs/research/slot-docking-drag-mechanism-2026-10-16.md:36` — during a drag, invoke round trip p50 2.2 · max about 50 ms · emit normal · no stalls (Tauri 2.11.3 · WebView2 155 · RDP · 22 runs — a small sample). DoDragDrop runs on the WebView2 browser process thread (not the Tauri main thread).
- **External:** Chromium deliberately runs tasks inside the DoDragDrop loop (`ScopedAllowApplicationTasksInNativeNestedLoop` in `StartDragging` of `web_contents_view_aura.cc`). We found no report that WebView2 messages · Tauri invoke break during a drag. **The only weak reports are about window manipulation** — focusing another window during a drag · a window first shown during a drag cannot be the drop target (one blog).

## Options in light of our constraints (comparison before the decision — the table at the time of the survey)

→ Decision = **ADR-0297** (leave it alone + look up again at drop · user decision 2026-10-20). The 「우리에게」 (For us) column below records the code at the time of the survey (U1's strict rejection · U3's deferral) and differs from the current code.

| Option | Precedent in practice | For us | Cost |
|---|---|---|---|
| **Leave it alone + re-verify at drop** | Mainstream (VS Code · dockview (throws) · FlexLayout · Zed · Lumino dock) | The shell already rejects at drop through preconditions (`TargetChanged` · `SourceChanged` · `TabNotFound`) — it is half this form already | 「끄는 동안 셸 호출 0」 must be narrowed to 「끌기가 상태를 셸과 맞추지 않는다」 (the drag does not sync state with the shell) (unrelated calls pass through). The deferral exceptions (terminal size · window frame · canvas) can be lifted |
| **End the drag only when the dragged item or its tab bar changes** | Chromium tab bar (forced completion) · Lumino tab bar (abort + revert) | When the dragged tab is closed by the LLM etc. | There is no cancel means (B) — Esc injection is unchecked |
| **Deferral (the design at the time of the survey)** | Only local precedent (Chromium window destruction) | An exception must be added for every event (the two new gaps the review caught are the proof) | An omission is itself an invariant violation |
| **Cancel on unrelated events** | No precedent | — | No cancel means · not in practice |

## Limits · gaps

- Peer sources are mostly a single point in time on the main branch and line numbers go stale. Chromium is a local copy (revision unknown).
- ★The Chromium grounds (table A · adversarial review 1 · grounding) come from reading a copy that exists only on this PC★ — `.claude/handoff/attachments/chromium-src-2026-10-18/` is not tracked by git so it does not exist on other PCs. Line numbers are also relative to that copy.
- 「끄는 동안 IPC 를 막는 피어 없음」 (no peer blocks IPC during a drag) means we did not find one.
- The effect of shell Esc injection · window manipulation during a drag is unchecked.

## Adversarial review (codex · 2026-10-20 · FIX)

1. **Writing Chromium as a 「취소」 (cancel) precedent = a wrong detail** — `EndDrag(kComplete)` is a forced completion (cancel is `kCancel` → `RevertDrag`). Main confirmed it with the local copy `tab_drag_controller.cc:727-746 · :2114 · :2173` and fixed it.
2. **Writing 「취소 안 됨」 (not cancelled) for `drag` preventDefault on standards grounds = a misattribution** — the standard and the Chromium implementation were written separately (the standard interpretation is left as contested — irrelevant to our conclusion: Chromium ignores it).
3. **Writing `DragStarting` as 「예」 (yes) for a means of cancelling in progress = a logic gap** — fixed to the route that intercepts the first drag.
- The reviewer's own exploration: peers that freeze IPC · saves during a drag · a trustworthy SendInput/PostMessage cancel path · WebView2 postMessage failure during a drag — no primary grounds found for any of the three. The conditional re-entrancy risk in the WebView2 threading docs (when a message handler opens a nested loop) does not mean failure of IPC in general during a drag.
- grounding (main): Chromium `tab_drag_controller.cc:732 · :742 · :2128` · `tab_strip.cc:1399` · Zed `workspace.rs:12257` · Lumino `tabbar.ts:332 · :431 · :523` checked against the original text — supported.
