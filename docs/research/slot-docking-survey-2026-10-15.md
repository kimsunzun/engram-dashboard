# Slot docking — dragging a window to attach it to another window's layout (peers · technical feasibility · size rule)

- 상태: Adversarial review folded in · **size rule decided (user 2026-10-15)** — whether beside a slot or on the outer border, one rule: **new = 1/3, existing = 2/3**. No exception rules such as a px minimum, a cap, or neighboring panes yielding (pure ratio — Unreal also uses only a ratio with no px rule · minimum 0). Beside a slot = only the target slot is split and the other slots stay as they are · outer border = the whole existing layout shrinks to 2/3 while keeping its inner ratios. With a small target the result is also small, which is the same attitude as the current split rule (ADR-0227 decision 9 — small panes allowed · only width 0 blocked). The rough path: 30% → percent + a base px (Unity-style A) → the problem that a fixed 200px band becomes 0 in a small window → settled on a pure 1/3. A · B · C of §8 are closed by this decision (the common items remain valid).
- Method: `/research` medium · design-decision mode. 3 collectors (peer UI conventions · Tauri technical feasibility · the state of our code) → main grounding (direct checks against the source text) → one cross-family adversarial review
- Date: 2026-10-15
- Tracking: T-63 (slot docking — `docs/tracking.md` on the `v0.3.3/chore/misc` branch, not reflected in master)
- Confidence legend: certain (independent cross-confirmation) · likely (basis supported · single source) · uncertain · contested

## The shape the user decided (2026-10-15 conversation)

- Docking unit = **the whole window**. To move one slot, first take it out into a separate window with the existing popout, then attach that window.
- When a window is dragged over a target slot, a **4-way guide of up · down · left · right** appears on that slot, and the slot is split toward the side dropped on and the window is inserted.
- The **outer border of the window** also has cells, and dropping there attaches it long along that side of the whole layout (e.g. bottom = a strip across the full width at the bottom).
- The core issue the user picked = **how to decide the size when docking**.

## 1. Size rule — peers (core)

| Peer | Docking beside a slot (inside) | Docking on the outer border | Preview | Basis · confidence |
|---|---|---|---|---|
| **KDDockWidgets** | When a floating window is dropped whole (`addMultiSplitter`), `DefaultSizeMode::FairButFloor` — **it gives an equal share, but if what was dragged in is smaller than that, it uses that smaller size** | the same drop path (the same rule) | unchecked | `src/core/DropArea.cpp:461,466-467` · `src/KDDockWidgets.h:147-149` checked against the source text — **likely** · ★the peer closest to our shape of attaching a floating window whole★ |
| Qt Advanced Docking System | splits the target area **in half** — `Size = (TargetAreaSize - handleWidth) / 2` (same direction) · `TargetAreaSize / 2` (other direction). ★conditional★ — for a floating container holding several areas, it can skip the halving and preserve the internal arrangement (review finding · not checked by main) | no size given, the splitter default (actual value unchecked) | area mode = half of the target · container mode = 1/3 ratio | `src/DockContainerWidget.cpp:705,738,833,844` checked against the source text — **likely** |
| FlexLayout (caplin) | **in half** — `moveNode.setWeight(this.getWeight() / 2); this.setWeight(this.getWeight() / 2)` | unchecked | half area of the target | `src/model/TabSetNode.ts:539-540` checked against the source text — **likely** |
| rc-dock | if the same direction, **new panel 30%** (`newPanel.size = box.size * 0.3; box.size *= 0.7`) · if another direction, 120 : 280 (= 30%) | the root (dockbox) also 30% | unchecked | `src/Algorithm.ts:283-284,296-298,310-311,325-326` checked against the source text — **likely** |
| Lumino (JupyterLab) | half of the target (by preview) | **new panel ≈ 38%** — after normalizing the existing root to a sum of 1, it adds a new panel sizer `GOLDEN_RATIO = 0.618` and normalizes again → 0.618 / 1.618 ≈ 0.382 | the root-cell preview = panel height × 0.618 (may differ from the actual insertion ratio — unchecked) | `packages/widgets/src/docklayout.ts:926-928,1489` checked against the source text — **likely** |
| VS Code (editor group) | preview 50% | no outer cell (based on a summary — uncertain) | half area of the result | `editorDropTarget.ts` summary — uncertain |
| Dockview | inserts with `Sizing.Distribute` (behavior unchecked) | has outer cells (`edges`) · size unchecked | adjusted with the `size` option · thin border if small | docs — uncertain |

- **The convention for inner docking size does not converge on one** — it splits among halves (a single-area Qt ADS · FlexLayout · VS Code · Lumino preview) · new panel 30% (rc-dock) · an equal share but the dragged-in size if smaller (KDDockWidgets).
- **There is a peer that reflects the size of the dragged-in window** — KDDockWidgets. The draft's 「찾지 못했다」 (could not find one) was disproved by the review.
- **Peers that remember size** — AvalonDock stores `DockWidth`/`DockHeight` and `FloatingWidth`/`FloatingHeight` separately per window, and JetBrains has a 「도구 창마다 크기 기억」 (remember size per tool window) setting. But whether this means 「다시 붙일 때 그 크기를 쓴다」 (use that size when re-attaching) was **not checked** (storing and reusing are different things — review finding).
- **Outer border docking also splits** — Lumino new panel about 38% · rc-dock 30% · KDDockWidgets equal share (the dragged-in size if smaller) · Qt ADS preview 1/3, actual size the splitter default (unchecked). No peer using halves was found.
- **Minimum size handling** — no explicit handling at drop time was found in the code checked (Dockview = left to the `minimumWidth`/`minimumHeight` of the grid leaf · FlexLayout = weight floor 1).
- **Neighboring slots** — Qt ADS restores the existing size of the outer splitter, so other areas are unaffected. Lumino renormalizes by ratio.

### Our current state

- Split ratio constant `SPLIT_RATIO = 0.5` (`src-tauri/src/layout/tree.rs:23`) — `slot.split` is always half now, and a split node has a `ratio` field so ratios other than half can be expressed (`src-tauri/src/layout/types.rs:80-100`). The ratio-adjusting command `split.setRatio` also already exists.

## 2. Guide cell placement — peers

- **Four inner directions + the center** is common. Center = **insert as a tab** into that area (Qt ADS · FlexLayout · Lumino · VS Code · Dockview). Our slots have no tabs (tabs are per window — ADR-0057), so the center cell cannot be carried over as is.
- **Outer border cells**: in Lumino (`root-top/left/right/bottom`) · Qt ADS (container compass) · Dockview (`edges`). VS Code not confirmed. The same as the shape the user wanted.
- **Preview**: four or more paint the result area translucently before the drop.
- **Command API**: Lumino `addWidget(widget, {mode: 'split-left', ref})` · FlexLayout `MOVE_NODE` (target + position) · Dockview `addPanel` (reference + direction) — the 「기준 + 방향」 (reference + direction) shape is common, so it fits LLM commands as is (CLAUDE.md 「LLM-우선 제어」 (LLM-first control)).

## 3. Precedents for 「OS 창을 끌어 붙이기」 (dragging an OS window to attach it)

- **KDDockWidgets** — drops a floating window whole into the target layout and deletes the floating window (`scheduleDeleteLater` after `addMultiSplitter` — `DropArea.cpp:466-469`). The same as the shape we decided — likely.
- **AvalonDock (WPF)** — when a floating OS window is dragged, it updates the guides to the cursor position on every `WM_MOVING` and drops on `WM_EXITSIZEMOVE`. The guides are a separate top-level overlay window. — source `LayoutFloatingWindowControl.cs` · collector report — likely
- **Chrome tab detaching · merging** — the detached window runs the native move loop (`SC_MOVE`) and detects the drop with a mouse hook, and finds its own window under the cursor excluding the window being dragged. — chromium source · likely
- **Qt ADS floating widgets** — a real top-level window that can be dropped whole.
- **Visual Studio** — a guide diamond appears when a window is dragged (docs). Size rule unchecked.
- **VS Code auxiliary windows** — there is no way to drag a window back to attach it (it returns by dragging the tab or closing the window — secondary source).
- Among Tauri apps and libraries, a precedent for native window re-docking was **not found**.

## 4. Technical feasibility — Tauri v2 + WebView2 (Windows)

Verdict: **it works, with conditions** — leave the native move loop as it is and have the backend drive the docking (the same shape as AvalonDock · Chrome).

- **「창 내용을 옮긴다」 (Moving the window content) = a model move** — slot content is data of the backend layout model (`SlotContent`), and the existing popout already moves the model and draws a new window (`apply::move_slot_to_window`). So docking too is not moving a webview (Tauri `Webview::reparent`) but inserting the floating window's slot tree into the target view tree and closing the floating window (the review pointed out that this choice should be stated explicitly).
- **Title-bar dragging and `start_dragging` are the same OS move loop** — tao `drag_window` sends `WM_NCLBUTTONDOWN` (HTCAPTION) after `ReleaseCapture` (checked against `tao-0.35.3/src/platform_impl/windows/window.rs:539,546`) — certain (source).
- **Tauri has no move start or end events** — `WindowEvent` = Resized · Moved · CloseRequested · Destroyed · Focused · ScaleFactorChanged · DragDrop (file drop) · ThemeChanged (checked against `tauri-runtime-2.11.3/src/window.rs`). tao uses `WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE` only as internal markers (`event_loop.rs:1028,1036`). So to learn the end of the drag, `WM_MOVING`/`WM_EXITSIZEMOVE` has to be received through an HWND subclass (`SetWindowSubclass`) — under this repository's rule that is inside the OS-layer crate `platform` (ADR-0266). Electron provides a `moved` (end) event.
- **Cursor position** — `AppHandle::cursor_position()` (Windows = `GetCursorPos`) exists, and calling `GetCursorPos` directly inside the subclass avoids a round trip.
- **While being dragged, the target window cannot receive pointer events (likely)** → for the guides, the backend tells the target webview 「이 창 · 이 칸에 가이드」 (guide on this window · this cell) and it draws in its DOM.
- **Decision under the cursor** — `WindowFromPoint` returns the window being dragged, so walk our own window list in z-order and exclude the dragged window (Chrome does the same). Our model already has the per-window canvas size and slot pixel rectangles (`set_window_canvas` · `slot_px` of `ViewManager`) — the material for the decision exists.
- **Esc cancel** — the native loop returns to the original position on Esc, but `WM_EXITSIZEMOVE` still arrives → cancel and drop have to be told apart (Chrome = keyboard hook).
- **The guides are covered by the dragged window** — if drawn inside the target webview, the dragged window covers it. Either make the dragged window translucent (VS-style — unchecked) or use a separate overlay window (transparent · click-through — WebView2 has a transparent window issue).

### What only a spike can resolve

1. Does emit to the target webview flow smoothly during the modal move loop (guide delay)
2. Is there no conflict if our subclass is placed on top of the subclass tao already installed · does the `WM_LBUTTONUP` that tao synthesizes after `start_dragging` not become a stray click
3. Telling Esc cancel apart
4. The slot rectangle decision when each monitor has a different DPI
5. Guide visibility (translucent window vs overlay window)
6. When the OS setting 「끄는 동안 창 내용 표시」 (show window contents while dragging) is off
7. A subclass can be installed and removed only on the UI thread that created the window — subclassing across threads is forbidden (`SetWindowSubclass` docs · review finding). The release order before the window disappears also has to be decided.

## 5. State of our code (what exists and what does not)

- **Exists**: a binary split tree per view (`Slot | Split{dir, ratio, a, b}`) · `slot.split` · `slot.close` · `split.setRatio` · `slot.popout{to_window}`, which moves one slot to a **new tab** of another window (bus · reachable by LLM) · popout window = native title bar · window move/size event hookup (for recording placement).
- **Does not exist**: an operation 「대상 슬롯 옆 특정 방향에 내용(서브트리)을 끼워 넣기」 (insert content (a subtree) beside the target slot in a specific direction) (`split_in_tree` always puts the original slot in `a` and an empty new slot in `b` — no direction or content argument) · an operation 「루트 전체를 감싸 바깥 변에 붙이기」 (wrap the whole root and attach to an outer side) · an operation that moves a view / a multi-slot subtree between windows · drag · guide code · HWND use.
- The original concept document also wrote down VS-style docking (`docs/process/S0-view-phase/requirements.md:98-104`). Moving tabs between windows was left as 「후속」 (follow-up) in ADR-0057 and does not exist yet.
- The renderer TRD rejected dockview · golden-layout · Lumino · FlexLayout because they 「자기 모델을 소유해서」 (own their own model) (`docs/process/S21-flat-split-renderer/trd.md:422`) — they are targets for borrowing patterns, not for adopting the library.

## 5b. Minimum size — when attaching makes it too small (additional investigation 2026-10-15)

1 collector (direct source reading) → main source check (tmux · dockview · KDDockWidgets · VS Code) → cross-family review.

| Shape | Peers | Basis · confidence |
|---|---|---|
| **(b)+(c) accept it, borrow from neighbors, and stop at the minimum** — if short, cut the overflowing content (overflow hidden · scroll) or enlarge the window | KDDockWidgets (minimum 80×90 px · if the root is short, enlarge the window) · dockview (group minimum 100×100 px · cut if it overflows) · VS Code editor group (minimum 220×70 px · SplitView scrolls if it overflows) · Lumino (「모자라면 전부 최소로」 (if short, everything to the minimum)) · Qt ADS · AvalonDock (minimum 25 px — estimate) | KDDW `src/core/layouting/Item.cpp:59` · dockview `dockviewGroupPanel.ts:21-22` · VS Code `browser/parts/editor/editor.ts:29` checked against the source text — likely · Qt ADS · AvalonDock uncertain |
| **(d) refuse** — if the two sides fall short of the minimum, do not split and notify | tmux (「no space for a new pane」 · minimum 1 cell) · Zellij (under 2×5 cells 「CAN'T SPLIT!」) · WezTerm (「No space for split!」 only when it would be 0 cells) | tmux `layout.c:1694` checked against the source text — likely · Zellij · WezTerm from the collector's source reading |
| **(b) conditional** — borrow if the neighbors have room, and leave it as is if the sum of minimums does not fit (leaving it below the minimum) | golden-layout (minimum width 10 px · `respectMinItemSize`) | `src/ts/items/row-or-column.ts` — the collector and the review both described the same behavior — likely |
| **(e) no handling** — it always splits and can become very small | sway (the minimum only blocks manual resizing) · i3 · Hyprland (estimate) · FlexLayout default (minimum 1 px) | collector's source reading — likely to uncertain |
| **(a) hide that cell in the guide** — for a cell where the two sides would not fit after dropping, do not offer the cell for that direction | deepseek-harness `ui-dockkit` (「An edge zone is only offered where the split it would make is allowed: within the pane budget and with room for two halves」) · those that give only a hook = dockview `canDisplayOverlay` · KDDockWidgets indicator-hiding callback (by default no size check) | `packages/client/ui-dockkit/src/components/DockSurface.tsx` checked against the source text (found by the review · main checked) — likely · single source |
| **(d) refusal on the GUI side** | Dock (Avalonia) `PreventSizeConflicts` — refuses docking if sizes of fixed-size tools conflict (review report · not checked by main — the wording was not found on the docs page) | uncertain |

- GUI docking libraries mostly **accept it and borrow from neighbors**. Refusal is the mainstream on the terminal multiplexer side, and one precedent of blocking by hiding the cell before the drop in a GUI was confirmed (deepseek-harness `ui-dockkit`).
- **Our current rule** — **a split is allowed** that makes a pane under 30px, and the only thing blocked is the extreme where one side's width becomes 0 (ADR-0227 decision 9 · R11 — user 「0이되는 극단적인 상황만 막으면 될것같아 일단은」 (I think we only need to block the extreme case where it becomes 0, for now)). The 30px pane minimum applies only to dragging the divider (ADR-0260).

## 6. Gaps

- Unchecked peers: golden-layout · Unreal's size and cell rules.
- Whether AvalonDock · JetBrains · Visual Studio actually use the stored size when re-docking.
- The actual behavior of Dockview `Sizing.Distribute` · the actual size of Qt ADS container docking.

## Adversarial review result

codex (cross-family) once · effort high · web search on — verdict **FIX**. The main re-checked the counter-evidence sources against the source text (KDDockWidgets · rc-dock).

| Finding | Handling |
|---|---|
| 「안쪽 도킹은 반반이 관례」 (Inner docking is halves by convention) is an overstatement — rc-dock 30% | Folded in (checked against the source text) — corrected to 「관례가 하나로 모이지 않는다」 (the convention does not converge on one) |
| Disproof of 「끌고 온 크기를 쓰는 피어 없음」 (no peer uses the dragged-in size) — KDDockWidgets `FairButFloor` | Folded in (checked against the source text) — added to the table, marked as the closest peer |
| Qt ADS halves are conditional (for a multi-area container, the internal arrangement is preserved) | Folded in — marked as not checked against the source text by main |
| Missing size-memory alternatives (AvalonDock · JetBrains · VS) | Folded in — only the storing fact is written, and whether it is reused is left as a gap |
| The meaning of 「창 내용을 옮긴다」 (moving the window content) (webview reparent vs model move) is unclear | Folded in — stated to be a model move (the same path as the existing popout) |
| Missing `SetWindowSubclass` thread constraint | Folded in — spike list 7 |

### Review of 5b (codex once · verdict FIX — folded in)

| Finding | Handling |
|---|---|
| Missing precedent of the (a) shape — deepseek-harness `ui-dockkit` | Folded in — confirmed against the source text |
| Classifying golden-layout as (e) — it is conditional handling | Folded in — as a separate row |
| Missing size refusal on the GUI side — Dock (Avalonia) `PreventSizeConflicts` | Folded in — marked as not checked by main |

The confirmed side (the review could not knock it down): FlexLayout halves · Lumino root about 38% · tao `drag_window` → `WM_NCLBUTTONDOWN` · Tauri `WindowEvent` has no move start/end.

## 7. General convention survey (2026-10-15 third round — game engines · authoring tools · IDEs · professional toolkits)

2 collectors (engines · authoring tools / IDEs · toolkits — direct source reading) → main source check (Unity `SplitView.cs` · Eclipse `SplitDropAgent2.java` · JetBrains `WindowInfoImpl.kt`) → cross-family review. User instruction: 「특정 것만 보고 다시 묻지 말고 성숙한 앱 전반을 조합해 최종안 몇 개로」 (do not look at only specific ones and ask again; combine mature apps in general into a few final options).

| Peer | Size beside a slot | Outer border size | Small target | Guide · center | Preview | Unit · cancel | Basis · confidence |
|---|---|---|---|---|---|---|---|
| **Unity Editor** | **1/3 of the target, but minimum 100px** | **fixed 200px band** (band decision 70px) | neighboring panes each yield down to their minimum | band at the target edge (1/3 · max 300px) · center = insert into the tab row | the exact result rectangle | one window (tab) · Esc cancel | `Editor/Mono/GUI/SplitView.cs:14-18,315-316,371-372` checked against the source text — likely |
| **Unreal (Slate)** | same weight as the target → usually halves | weight 1.0 (equal to the existing children) | minimum 0 (no handling) | band = 0.3 of the target (5~150px) · cross + outer cells · center = tab | transformed to half of the target (may differ from the actual) | one tab · Esc cancel (likely) | the collector read the engine source (`SDockingTabStack.cpp` · `SDockingArea.cpp` · `SDockingCross.cpp`) — not checked by main · likely |
| Blender | decided by cursor position (snaps to 0.5 · Ctrl = 1/8 steps) | — | drops the direction cells that cannot fit (under about 58px = no side cell) | center = swap content | the applied ratio as is (likely) | one area · Esc · right-click cancel | `screen_ops.cc` · manual — collector |
| Adobe (Photoshop · AE) | unchecked | unchecked | unchecked | edge = beside · group cell = tab | unchecked | whole by group handle · **holding Ctrl moves without attaching** | help search fragments — uncertain |
| Visual Studio | unchecked | unchecked | unchecked | diamond guide + shading | shading (whether exact is unchecked) | **dragging with Ctrl moves without attaching** · Ctrl+double-click = to the last docked place | MS docs — certain (docs wording) |
| **JetBrains** | half of the side | **default weight 0.33** · if 「도구 창마다 크기 기억」 (remember size per tool window) is on, that size when re-attached | unchecked | side stripes | — | one tool window · cancel = stop dragging | `WindowInfoImpl.kt:35,87,89` checked against the source text · `ToolWindowManagerImpl.kt` — likely |
| **Eclipse 4** | 0.5 | **0.33** (outer band 35px) | unchecked | no compass · edge bands | ratio shown | — | `SplitDropAgent2.java:37,237,240` checked against the source text — likely |
| AvalonDock | preview half | preview = `min(선호 크기, 절반)` (min(preferred size, half)) | minimum 25px | compass + outer | general half | the floating window whole (internal splits kept) · **reuses the last docked size when re-attaching** (likely) | source — collector |
| Qt ADS · KDDockWidgets | halves · equal (the dragged-in size if smaller) | preview 1/3 · equal | shrinks · minimum 80×90 + enlarge the window | compass + outer | general · **exact (KDDW)** | the floating window whole (internal splits kept) · Esc (ADS) | §1 · §5b |

**What converges**
- The outer border makes **the new one small** — around 1/3 (Eclipse 0.33 · JetBrains 0.33 · ADS preview 1/3 · Lumino 38% · rc-dock 30%) or fixed px (Unity 200px).
- Beside a slot it splits — halves (Unreal · Qt ADS · Eclipse · FlexLayout) vs small (Unity 1/3 · minimum 100px · rc-dock 30%).
- **「퍼센트, 기준보다 작으면 기준 px」 (Percent, but the base px if smaller than the base) is exactly what Unity does** (1/3 of the target, minimum 100px).
- Toolkits that attach a floating window whole (Qt ADS · AvalonDock · KDDW) **insert the window's internal split arrangement as it is**. Engines (Unreal · Unity) move only one tab at a time.
- Cancel = **Esc** (Unreal · Unity · Blender · Qt ADS). **Dragging with Ctrl moves without attaching** (Visual Studio · Adobe).
- Command API = the 「기준 + 방향 (+ 비율)」 (reference + direction (+ ratio)) shape is common (Eclipse `insert(el, relTo, where, ratio)` · KDDW `addDockWidget(location, relativeTo)` · Unreal layout builder).
- Small target — no handling (Unreal) · neighbors yield (Unity) · drop the cell (Blender) · enlarge the window (KDDW). No GUI peer refuses.

## 8. Final options (combination — awaiting user choice)

**Common to all options** (what the peers converge on · what is already decided)
- Unit = the whole floating window. The internal split arrangement of the window is inserted as it is (Qt ADS · AvalonDock · KDDW). If there are several tabs, the arrangement of the visible tab is inserted and the other tabs stay in the floating window (a matter of our structure — no peer precedent).
- Guide = bands at the up · down · left · right edges of the target slot + bands on the outer border of the window. There is no center cell (our slots have no tabs).
- Preview = the **exact rectangle** that will result on drop (Unity · KDDW).
- Cancel = Esc. **Dragging with Ctrl moves the window without attaching** (Visual Studio · Adobe).
- The space needed is always made by pushing the existing ones — it is a split that fills the window completely, so there is no overflow (user decision).
- LLM command = 「창 · 기준 슬롯(또는 바깥 변) · 방향」 (window · reference slot (or outer side) · direction) (the common shape of Eclipse · KDDW · Unreal).
- Optional addition — **returning to the last docked place** (Visual Studio Ctrl+double-click — switching between floating ↔ the last docked place). It can be used to return a popped-out slot to its original place.
- Note: Eclipse has turned off the outer top band (only left · right · bottom) — whether to provide all four outer sides is decided at implementation.

| | **A. Unity-style** | **B. Halves + outer 1/3** — halves beside a slot = Unreal · Qt ADS · Eclipse partial stack · outer 1/3 = Eclipse (`0.33`) · JetBrains default weight | **C. Remember size** — JetBrains (optional setting) · AvalonDock (likely from the source) |
|---|---|---|---|
| Beside a slot | goal = 1/3 of the target, minimum 100px (Unity's drop rectangle — the actual size can differ because of the neighbors' minimums) | half of the target | the size that window last had when attached · A rule if first time |
| Outer border | fixed 200px band | 1/3 of the window | the last size · A rule if first time |
| Small target | if the target cannot reach the minimum, the neighboring panes of the same split each yield down to their minimum (Unity) | no handling (the current split rule as is — only width 0 blocked · ADR-0227) | same as A |
| Pros | the same as the 「퍼센트 + 기준 px」 (percent + base px) the user mentioned · the existing layout is disturbed the least | simplest · the same halves as the current `slot.split` | when you pop out and return, it comes back to the original size |
| Cons | three numbers (1/3 · 100px · 200px) must be decided | attaching beside a slot shrinks the target a lot (conflicts with the 「새것을 작게」 (make the new one small) the user chose) | the size must be stored per window · popout windows are created ad hoc, so a definition is needed for what 「그 창의 마지막 크기」 (that window's last size) is |

### Review of 7 · 8 (codex once · verdict FIX — folded in)

| Finding | Handling |
|---|---|
| Unity's 100px is a drop rectangle, not a guarantee of the final size | Folded in — A changed to 「목표」 (goal) |
| The basis of C is overstated — JetBrains is an optional setting, and for AvalonDock only the stored-state docs exist | Folded in — lowered the C basis markings (AvalonDock reuse is the collector's source reading · not checked by main) |
| B's outer 1/3 attributed to Unreal | Folded in — limited to Eclipse · JetBrains |
| Eclipse outer top band turned off | Folded in — note line |
| Which neighbor yields is undecided | Folded in — 「같은 분할의 옆 칸」 (the neighboring panes of the same split) in A |
| Missing return to the last docked place | Folded in — added to the common options |

## 9. Usable libraries (2026-10-15 fourth round)

1 collector → main check (AgentCat PR #36 · drag-rs repository facts) → cross-family review.

**Verdict: there is no library that does the core — only borrow the patterns and use the `windows` crate directly inside `platform`.** All candidates either have their own layout model or start the drag in the DOM (not a native title-bar move).

| Candidate | What it gives | Fit | Verdict |
|---|---|---|---|
| `windows` crate (already a `platform` dependency) | `SetWindowSubclass` · `DefSubclassProc` · `GetCursorPos` | fits as is (OS code inside `platform` — ADR-0266) | **use it** |
| tao (Tauri window layer · Apache-2.0) | uses `WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE` only inside and does not emit them as events · synthetic `WM_LBUTTONUP` at the end | our subclass must only look at messages and pass them on with `DefSubclassProc`, or tao's markers break | reference only |
| AgentCat PR #36 (Tauri app · merged 2026-08-01) | receives `WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE` through window subclassing and uses the native move loop as the canonical drag lifetime · outside Windows falls back to the webview pointerup | a real Tauri precedent of the same shape (4 stars · no license marking) | borrow the pattern |
| crabnebula `drag-rs` · `tauri-plugin-drag-as-window` (MIT/Apache-2.0 · active) | starts an OLE drag from a DOM element and the target webview receives `DragDropEvent::Over` (with position) | the drag starts in the DOM — not a title-bar drag. An alternative if the design starts the drag from a tab handle | not applicable to the current design |
| Tauri `startDragging()` · `WindowEvent::Moved` · cursor position API | the same native move loop · position during the move | no end event | use as plumbing |
| Atlassian pragmatic-drag-and-drop (Apache-2.0 · active) | nearest-edge computation (about 10 lines) · drop indicator parts | it moves only by pointer events — a webview does not receive pointer events during a native move | pattern only |
| dnd-kit · react-dnd | dragging inside a document | same as above | not applicable |
| dockview · FlexLayout · golden-layout | multiple windows (`window.open` popouts) · docking guides | owns its own model (the renderer TRD already rejected it) · the guides are entangled with the model · whether `window.open` popouts run in Tauri/WebView2 is unchecked | not applicable |

- No Tauri docking plugin was found.
- **Gaps** — detecting the move end on macOS · the WebView2 behavior of dockview popouts · the body of the AgentCat code (only the description was read) · Tauri issue #10767 (window dragging flips focus and swallows mouse events — may get tangled with the guides · not investigated).

### Review of 9 (codex once · verdict FIX — conclusion kept, supplement folded in)

| Finding | Handling |
|---|---|
| Tauri's own runtime has an implementation of `SetWindowSubclass`/`DefSubclassProc` + `WM_NCDESTROY` cleanup — the closest reference | Folded in — confirmed by a local check of `tauri-runtime-wry/src/undecorated_resizing.rs:88-89,189,252,260`. **First-priority implementation reference** |
| `drag-rs` also has `dragBack()` (return to the window) | Folded in — it is a DOM drag path, so the verdict stays |
| dockview documented popout re-attaching · position events · Tauri-specific handling | Folded in — it owns the model so the verdict stays; the WebView2 behavior is still unverified |
| winit 0.31 added a cross-window drag source and drop/cancel events (triggered by the re-docking request issue #3229) | Folded in — it is data transfer and not tracking of title-bar moves, and Tauri sits on tao, so it does not apply directly |
| Subclass wrappers `michiu` (only windows it made itself) · `winsafe` (exposes unsafe) | Folded in — no substitute, use the `windows` crate directly |
