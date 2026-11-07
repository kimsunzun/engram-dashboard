# TRD — Flat split renderer (replacing allotment) + writing split ratios back + the shell's window canvas geometry

- Status: **Confirmed — /review trd full 3 rounds (r1·r2 both reviewers' FIX applied, r3 doc-aware single-reviewer confirmation PASS) · user decisions D1~D6·R11 applied** · Date: 2026-09-24 · ADR = ADR-0227
- Implementation: **§6 steps 1~8 committed** — from `e401966` to `5e99268` (2026-09-24~25). The per-step gates are in each commit body, the round record is in `docs/process/step-log.md`. In step 9 (docs) this document was aligned to the implementation — places where the implementation differs from the design, or where it decided something the design left open, are marked **「구현:」**.
- ★**Line pointers are based on the code at design time (`473da52`) — only pointers inside a 「구현:」 place (including 「구현 —」 and 「구현(…)」) or carrying 「지금(9단계)」 are based on the tree at step 9**★. A bare 「지금」 or 「구현 메모」 outside such places is a statement from design time, so its pointers are also `473da52`. Later implementation and the master merge changed the files and shifted the lines, so open them with `git show 473da52:<경로>`.
- Research basis: `docs/research/split-layout-library-survey-2026-09-24.md` (renderer structure — option D) · `docs/research/split-resize-policy-survey-2026-09-24.md` (D1~D6 policy)
- Terms: **shell** = the client shell backend (the `ViewManager` in `src-tauri/src/layout/`). It is not the daemon. **pane** = slot (leaf), **splitter** = the drag handle on a split boundary.
- Scope: **inside one view**. Tabs are already keep-alive (`src/components/layout/WindowLayout.tsx:182-197`, ADR-0056), so they are not touched.

## 1. What the current renderer does — responsibilities that must survive

`src/components/layout/ViewLayoutRenderer.tsx` (325 lines) draws the tree as a **recursive component**. One slot node is one renderer instance.

| # | Responsibility | Location | New place |
|---|---|---|---|
| R1 | props `node`·`focusedSlotId`·`viewIdOverride` | `:26-37` | root renderer (§2e) |
| R2 | View coordinate of menu·focus = `viewIdOverride ?? useCurrentViewId()` | `:68-70` | leaf |
| R3 | render mode = `renderModeOverride[slotId] ?? defaultRenderMode(agent)` (the store key is already the slot id — `src/store/viewStore.ts:78`) | `:38`, `:75-80` | leaf |
| R4 | caps gate — do not mount a concrete renderer before AgentInfo arrives (ADR-0041/0044) | `:101-109` | leaf |
| R5 | ADR-0148 last-mount memory `lastMountRef` + the effect that records after commit and discards on mismatch | `:47-62`, `:82-97`, `:110-116` | leaf (survives by the slot id key — §2g) |
| R6 | three absence branches (connecting / view kept / does not exist — ADR-0148·0149) | `:117-127`, `:193-205` | leaf |
| R7 | content branches: agent→dom/rich/terminal · preset_palette · agent_list · empty (`+` icon, `pointer-events:none`) | `:192-241` | leaf |
| R8 | slot frame (1px border fixed · `position:relative` · overflow/centering depending on hasContent) | `:128-159` | leaf (position by absolute placement) |
| R9 | `data-slot-id` — cdp QA narrows by this (`.claude/skill-bindings/qa.md:262`) | `:161` | keep on the leaf wrapper |
| R10 | click-to-focus, content slots only (`isContentSlot`, ADR-0066), bubbling allowed | `:162-180` | leaf |
| R11 | right-click menu state (`useState` per slot) + `SlotContextMenu` (ADR-0064/0144) | `:67`, `:183-186`, `:243-254` | leaf |
| R12 | focus ring overlay (accent 40%, `zIndex:10`) | `:255-270` | leaf |
| R13 | per-slot error isolation `SlotErrorBoundary` (resetKey `${id}:${type}:${agent}:${mode}`) | `:188-191` | leaf |
| R14 | ADR-0140 direction mapping (`top_bottom` = top/bottom) — 「유일한 진실 경계」 | `:274-275`, `:313` | **shell geometry function** (§2a) |
| R15 | initial ratio sizing `defaultSizes` (ADR-0063) | `:276-291`, `:314` | the shell rectangle is the size |
| R16 | double-click reset (pane-a `preferredSize` integer %) | `:287-290`, `:316` | splitter → 50:50 command (D5) |
| R17 | split instance key `${id}:${dir}` (ADR-0223) · pane key fixed by position | `:292-308`, `:312`, `:316-319` | **gone** — the split id is used as the splitter key · preview key |
| R18 | what allotment did implicitly: minimum pane 30px (`node_modules/allotment/dist/modern.mjs:963`) · proportional distribution on window resize + minimum clamp + integer px rounding (`:659`) · splitter hit area 8px / hover line 4px (`dist/style.css:73-74`) · hover delay 300ms (`modern.mjs:436`) · double-click (`:466`). No keyboard adjustment | — | splitter · shell |

Other things that depend on allotment: `src/main.tsx:5-8` (CSS import) · `src/index.css:33-40` (rules overriding allotment variables with theme tokens) · `package.json:28` (+lock) · `src/components/layout/ViewLayoutRenderer.allotment.test.tsx` (437 lines — split replacement 4 `:179` · same split kept 3 `:243` · first frame 2 `:291` · error isolation 5 `:337`) · the stub in `ViewLayoutRenderer.test.tsx` (`:66-101`) and the split tests that rely on it (`:563-658`) · `src/components/slot/slotDaemonRestart.test.tsx:72-74` (mock) · `src/components/slot/TerminalSlot.tsx:301` (comment) · `CLAUDE.md` 「기술 스택」. There is no selector or cdp recipe that picks allotment classes (`.split-view`·`.sash`) (`rg "split-view|sash" src/ scripts/ .claude/skill-bindings/` → only comments and `index.css`).

## 2. Architecture

### 2a. Shell geometry function — the screen and the shell use the same rectangle

**What the shell has now is not enough.** The shell already computes a normalized rectangle per leaf (`src-tauri/src/layout/spatial.rs:57-110` `assign_rects`), but it cannot be used as is, for four reasons.

1. It is internal — `NormRect` is `pub(crate)` (`spatial.rs:33`) and the snapshot carries only neighbors·order (`src-tauri/src/layout/types.rs:113-116`, ADR-0068 decision 3).
2. Being in `{x,y,w,h}` form, there is no guarantee under nesting that neighboring boundaries are equal bit for bit (`b2.x + b2.w` may differ from the parent's right boundary in the last floating-point digits → a hairline gap on screen).
3. It contains the `[EPS,1-EPS]` clamp for adjacency decisions (`spatial.rs:78`).
4. It has no split (splitter) boxes.

**What is added:** a new pure module `src-tauri/src/layout/geometry.rs` (no dependency on Tauri or locks, ADR-0012 standalone tests).

- Input `&LayoutNode`, output `Vec<SlotRect>` + `Vec<SplitRect>`. Coordinates are in **boundary form** `x0,y0,x1,y1` (relative to the view, [0,1], `f64`).
- **A child copies the parent's boundaries and the only new value is the one split boundary** — `at = x0 + (x1 - x0) * ratio`. The `x1` of a and the `x0` of b are the same variable, so neighboring boundaries are equal bit for bit. It does not depend on depth.
- The ADR-0140 mapping is in this one place: `left_right` → x-axis split (a on the left), `top_bottom` → y-axis split (a on top).
- `SplitRect = { split_id, dir, x0,y0,x1,y1, at }` (this split's own box + the boundary coordinate).
- The ratio is clamped here too to `[RATIO_MIN, RATIO_MAX]` (§2c). The write path already guarantees it, but this is so that a value such as `0.0` that a test puts directly into the tree cannot produce a leaf of area 0. *Implementation:* a NaN passes through the clamp and spreads to all descendant coordinates, so it is read as a half split (0.5) (`boundary` in `geometry.rs`).
- `spatial.rs` drops its own rectangle computation and takes this function's boundaries as input (§2h). Inside the shell too, the geometry has a single source.
- **Pixel conversion is here too:** `px_edge(canvas: u32, e: f64) = (canvas as f64 * e).round()`. The width is **the difference of two rounded boundaries** (`round(W·x1) − round(W·x0)`) — the width is not rounded separately. So the px boundary of neighboring panes is always the same integer. There are no negatives, so the tie handling of Rust `round` (away from zero) and CSS `round(nearest)` (toward +∞) is the same.

### 2b. Snapshot extension

- Add `slot_rects: Vec<SlotRect>` · `split_rects: Vec<SplitRect>` · `ratio_min: f64` · `ratio_max: f64` to `ViewSnapshot` (filled by `snapshot()` at `manager.rs:140-152` · the struct is at `types.rs:112-141` now (step 9)). ts-rs newly produces `SlotRect.ts`·`SplitRect.ts` and changes `ViewSnapshot.ts`.
  - The reason both ratio limits are in the snapshot: so that the screen drag does not keep its own copy of the shell constants 0.1/0.9. It is simpler than a cross-language golden test.
- The frontend cache `CachedView` (`src/store/viewStore.ts:58-63`, filled at `:242-250`) holds both together. Today it does not even cache `slot_spatial`.
- This is a revision of ADR-0068 decision 3 (「좌표 노출 보류」) — normalized coordinates ride in the shell→screen snapshot. **Exposing coordinates·px through the LLM command surface is not in this scope** (§7 R10).

### 2c. Ratio command — humans and the LLM through the same apply service

- **Raise ratio storage from `f32` to `f64`** (`types.rs:78`).
  - Reason: a `0.3` received over the bus, passing through `f32`, comes back as `0.30000001192092896` and looks as if it had been clamped. With `f64` the received value comes back as is.
  - ts-rs exports both as `number`, so the body of `LayoutNode.ts` is unchanged (the generated-file sync gate checks it).
  - Tests with `f32` literals such as `tree.rs:269` change only the type.
- **Pure:** `tree::set_ratio_in_tree(node, split_id, ratio) -> bool`. Constants `RATIO_MIN = 0.1`·`RATIO_MAX = 0.9` (precedent: herdr `src/layout.rs:275-277`). The old `clamp_ratio` (`src-tauri/src/layout/tree.rs:13`, 0~1, called only by tests) is replaced with these constants.
  - **The finite-value check lives in the manager** (not only on the bus). A non-finite value is `LayoutError::InvalidRatio`, and the Tauri path goes through the same check.
- **Manager:** `ViewManager::set_split_ratio(view, split, ratio) -> Result<SplitRatioResult { ratio: f64, outcome }, LayoutError>`. Add `LayoutError::SplitNotFound`·`InvalidRatio` (`manager.rs:46-56`).
  - `outcome` is one of three.
    - `Applied` — the value changed and `bump_version` (`:187`) was done.
    - `Unchanged` — after the clamp the value equals the current one. No change · no notification.
    - `TooSmall` — the px range is empty, so nothing was touched. No change · no notification.
  - **The returned version is meaningful only for `Applied`** — that value is the version of the snapshot notified to this view. The manager version is global (`:150`) and the screen cache is per view (`src/store/viewStore.ts:240-241`), so if we made the screen wait for the global version when nothing changed, the preview would stay until a pull (§2f rule 2).
  - **That version is taken from the snapshot the apply service captures in the same critical section** — inside the lock: write → `snapshot(view)` → put that snapshot's `version` in the return value (the shape of `split_slot` at `apply.rs:311-321`). The global counter is not read again after the lock is released. If another write slips in between, it would not match the notified snapshot.
  - **Degeneration guard (representability rule):** if any leaf of the resulting tree would get area 0 in f64 geometry (`x0 >= x1` or `y0 >= y1`), do not write and return `outcome = TooSmall`. It recomputes the whole view with `geometry.rs` to check, O(number of leaves). It also blocks the path where an ancestor ratio change collapses a deep descendant.
    - *Implementation note (defensive path only):* only leaves that **newly** get area 0 are filtered. A leaf that already had area 0 before the write (it cannot arise through the write path — a value planted by a test, etc.) is left out of the comparison. Otherwise every `set_split_ratio` would be locked as `TooSmall` in a view with one such leaf.
    - *Implementation note:* so `TooSmall` now has two causes. ① the px range is empty ② one leaf collapses. The screen's drag range cannot know ② in advance. It occurs only in chains of 16 levels or more, so it is accepted — then the confirm response is `TooSmall` and, on release, the preview returns to the shell value (§2f rule 2).
- **The shell's px-minimum enforcement (first line of defense that keeps neighbors·order):** if it knows both the owning window's `canvas` and `metrics.min_pane_px` (§2d), it narrows once more after the ratio clamp.
  - `L` = canvas axis length × that split box's axis width (`geometry.rs`, f64). `m = min_pane_px`. Allowed range = `[max(0.1, m/L), min(0.9, 1 − m/L)]` — each of the split's two sides (the whole subtree) is at least `m` px.
  - The requested value is **clamped to the nearest allowed value** and the applied value is returned (it is not rejected — same as the drag path). If the range is empty (`L < 2m`), the current ratio is left as is and **`outcome = TooSmall`** is returned — distinguished from 「적용됨」 (the same condition as 「잠김」 on the drag path).
  - If either is unknown (before the report · right after a popout), only the ratio clamp is done.
  - The shell does not hard-code a px constant — `m` is the value the screen sent.
  - The screen drag also computes its range from **the same formula and the same input** (the integer canvas it reported × the snapshot box). So the confirmed value is not cut again in the shell, and nothing jumps on release (§2f).
- **Apply service:** `apply::set_split_ratio` — the same shape as `focus_slot` (`src-tauri/src/layout/apply.rs:348-366`). Output routing does not change, so it does not take `SubscriptionSync`. It notifies only the layout snapshot and not tabs (`tabs=None` in `notify` `:157-168`).
- **Human path:** `#[tauri::command] set_split_ratio(view_id, split_id, ratio) -> SplitRatioApplied { ratio: f64, outcome, version }` (the shape of `split_slot` `:321-342` in `src-tauri/src/commands/layout.rs`, registered in `generate_handler!` at `src-tauri/src/lib.rs:211`). The return type is exported through ts-rs (*implementation:* `SplitRatioApplied.ts` + the outcome enum `SplitRatioOutcome.ts`), and `#[ts(type = "number")]` is put on `version: u64` (precedent: `types.rs:136-140` now (step 9)).
- **LLM path (command bus):** add to the declaration block (`:71`) of `src-tauri/src/layout/commands.rs`.
  - `"split.setRatio" => args { view_id: String, split_id: String, ratio: f64 } -> ok { ratio: f64, outcome: RatioOutcome }` (`Applied`·`Unchanged`·`TooSmall` — an enum without fields, so the macro carries it).
    - *Implementation:* the enum name on the declaration-macro side is `RatioOutcome`. Only the type name differs from its twin inside the shell, `SplitRatioOutcome` (the Tauri reply); the variant spellings are the same (§4 「구현 중 메인이 정한 것」).
    - A non-finite value is `INVALID_ARGUMENT`. Out of range, it returns the clamped applied value (not an error).
    - The macro carries `f64` as `{"type":"number"}` (`crates/engram-dashboard-command/src/macros.rs:149`).
    - **Write 「ratio = a 쪽(왼쪽/위) 칸의 몫」 in the summary** (ADR-0140).
  - `"split.list" => args { view_id: String } -> ok { splits: Vec<SplitRow { split_id, dir: SplitDirection, ratio: f64, a_slots: Vec<String>, b_slots: Vec<String> }> }` — write the same ratio definition and 「a = 왼쪽/위」 in the summary.
    - **The LLM currently has no way to find a split id.** `get_view` is a recursive type so it is not on the bus (`commands.rs` header), and `slot.resolveSpatial` (`:266-272`) gives only slot ids.
    - Giving both slot lists lets it pick 「x 와 y 사이 구분선」.
  - `catalog_version` 7 → 8 (`:72` — raise it when a declaration changes, `:62-70`).
- **Concurrent writes:** the last write in shell arrival order wins. The shell serializes with one lock, so no extra mechanism is needed. Reconciliation on the screen side is §2f.
- **The same representability rule is applied to the split path.** Ratios 0.1~0.9 alone do not guarantee a positive area — if we keep splitting the child on the same side, the f64 boundary becomes equal to the parent's end (about 54 levels at 0.5, about 17 levels toward 0.9). `split_in_tree` (`tree.rs:57`) currently has no depth guard.
  - If the new boundary `at` does not fall **strictly** inside the target pane's box (`at <= x0 || at >= x1`, y for a top/bottom split), `ViewManager::split_slot` does not change the tree. It returns a new `LayoutError::SplitTooDeep`, which becomes `CONFLICT` on the bus and an `Err` string in Tauri. It does not panic.
    - *Implementation:* a pane that already has area 0 (it cannot arise through the write path — a planted tree) also gets `SplitTooDeep`, regardless of axis. If split along the other axis, even when the new boundary falls inside the pane, both new panes have area 0.
  - **A split that creates a pane smaller than `min_pane_px` is allowed** (R11 — user decision 2026-09-24). The only thing blocked is the representability rule above (`SplitTooDeep`). Even when the canvas is known, a split is not rejected on px grounds (§7 R11).

### 2d. Window canvas size and base metrics — no per-pane reports, no constants

- **Storage:** add `canvas: Option<CanvasPx { w: u32, h: u32 }>` · `metrics: Option<UiMetrics>` to `WindowTabs` (`src-tauri/src/layout/manager.rs:58-62`). The 3 constructors (`manager.rs:102,239,504`) start with `None`. When the window closes, the entry disappears whole. A popout window is a new entry, so it is `None` until its webview reports.
- **Neither report command takes the window label as an argument.** They take `tauri::Window` and use `window.label()` — precedent `report_view_commands` (`src-tauri/src/commands/view_bus.rs:24-26`: 「잘못 적힌 label 하나가 남의 창으로 간다 — Tauri 가 넣어 주는 Window 가 유일한 권위」).
- **Canvas report (only when it changes):** measure the tab content area of `WindowLayout` (`WindowLayout.tsx:182` — all tabs overlap inside it with `inset:0` `:189-193`) with a `ResizeObserver`. This area does not hide (what hides is the tab canvas).
  - That div renders only after `win` arrives (the early return at `:152-167`), so the observation is attached and detached with a **callback ref**, not `useEffect([])+useRef`.
  - In an environment without `ResizeObserver` (jsdom — vitest has no setupFiles), observation is skipped.
  - 100ms debounce → integer rounding → **do not send if `w == 0 || h == 0`** (a 0×0 RO callback does occur — `TerminalSlot.tsx:99-101`) → `report_window_canvas(w, h)` only when it differs from the last **successful** value.
  - On failure, the last value is not updated. RO fires only when the size changes, so instead of waiting for the next observation it resends by the **same bounded retry** (`src/util/retryInvoke.ts`). If a new size arrives in the meantime, the old retry is cancelled.
  - *Implementation:* what to send and when is decided by `src/components/layout/windowCanvasReport.ts` (only observation·debounce are in the hook in `WindowLayout.tsx`). The state is a **webview singleton** (module scope) — hook instances change even within the same webview (error-boundary redraw · HMR remount), so with per-instance state a new instance would not know about an in-flight request and would fire one more.
    - **One invoke at a time per webview.** If the shell handled two overlapping requests in reverse arrival order, a stale canvas would stay until the next resize (whether WebView2 delivers in issue order is not in the docs — unknown). If one is in flight, send after its response. Cost: if a response never comes, that window's reporting stops there.
    - **Keep only the latest value to send** — a new value overwrites the waiting value. If a new observation before the debounce differs from the value being sent or waiting, withdraw that retry·waiting value immediately.
    - A success response is recorded as the shell's current value even if it belongs to a cancelled job — with one at a time, response order is the shell's apply order.
    - When the value the shell accepted changes, subscribers are notified (`subscribeCanvasReport`) — the splitter's allowed range reads this (§2f).
  - Hidden tabs share the window canvas, so one per window is enough.
  - Boot · popout creation · webview reload are all a fresh `WindowLayout` mount, so the first measurement is the report.
- **Base metrics report (once per webview):** `report_ui_metrics(UiMetrics { frame_insets: Insets { t, r, b, l: f64 }, min_pane_px: u32 })`.
  - `frame_insets` = **measured**. The four border widths from `getComputedStyle` of the border element inside the pane frame (§2e). At 125% scaling, if Chromium snaps borders to device px, fractions can appear, so it is received as `f64`.
  - `min_pane_px` = the screen's **policy constant** `MIN_PANE_PX` (30) — it is not a measured value, so it is a separate field. The **only source of truth for this value is the screen**, and the shell receives and uses it (§2c).
  - Sent when the first leaf mounts. **The 「보냈음」 flag is set only after a success response**, and on failure it retries with bound (`src/util/retryInvoke.ts`). The flag is module level, so it is reset together with a webview reload.
  - **In-flight latch**: at boot N leaves mount at once, so one module-level in-flight promise is kept and the second and later leaves share it. The report goes out only once. If it ends in failure, the latch is released so the next leaf mount can try again.
  - *Implementation:* `src/components/layout/uiMetricsReport.ts`. When the leaf body (`SlotBody` — §4) mounts, it measures its own `[data-slot-border]`. If even one border width cannot be read as a finite number, it does not send and only logs an error (the shell would reject that value, so sending would only burn the retries).
  - The only fixed decoration attached to every pane now is the 1px frame border (`ViewLayoutRenderer.tsx:139`). There is no header line. Per-content inner padding (such as `4px 8px` at `DomSlot.tsx:203`) is not put into the metrics — it is a matter of the pane kind.
- **Shell-side defense:**
  - `report_window_canvas` ignores `w == 0 || h == 0` and keeps the last value.
  - `report_ui_metrics` checks the range, rejects (`Err`) values outside it, and keeps the last value — inset is finite·`0 ≤ v ≤ 64`, `min_pane_px` is `1 ≤ v ≤ 1000`.
- **Shell computation:** `ViewManager::slot_px(view, slot) -> Result<Option<SlotPx { frame: PxRect, content: RectF64 }>, LayoutError>`. `None` if the canvas is unknown. *Implementation:* it is also `None` if the metrics are unknown — `Some` only when **both** canvas·metrics are known (§4 「구현 중 메인이 정한 것」). A nonexistent view·slot is `Err` before that.
  - `frame` = the integer rectangle made by rounding the owning window's `canvas` × that pane's `SlotRect` by the §2a rule.
  - `content` = `frame` reduced by `frame_insets`. **Width·height do not go below 0 — they are `max(0, …)`** — there is no minimum window size (D4) and an ancestor drag can make a pane smaller than the frame border (D2).
  - **The splitter thickness is not subtracted** — the splitter is an overlay that takes no layout space (§2f).
  - The shell does not ask the screen.
  - The two reports are **not bus commands and neither raise the version nor notify** (they are measurement reports, not control — on the bus an LLM could forge the canvas). Precedent: `report_view_commands`, which each window reports once at boot (`src-tauri/src/commands/view_bus.rs:33`).
  - The only consumer now is the px-minimum enforcement of `split.setRatio` (§2c). `slot_px` itself belongs to future shell features (the user's reason — §3 D1).

### 2e. The flat renderer and rounding

- Under **one** root `div` (`position:absolute; inset:0; overflow:hidden`), draw `slot_rects` sorted **by slot id** as a list of `<LayoutLeaf key={slotId}>` (D6), and after that draw `split_rects` sorted **by split id** as `<Splitter key={splitId}>`.
  - A surviving leaf keeps its parent·key·relative order, so there is no remount and no DOM move. Only the position values change; a new leaf is inserted and a closed leaf is removed.
  - Why splitters are also in id order: in tree pre-order, an LLM restructure would change the order, and React would move a node holding a pointer capture, causing `lostpointercapture`.
- **Separate the pane frame from the border — the frame is exactly the rectangle and the border is clipped inside it.** Today the wrapper itself has a 1px border + `border-box` (`ViewLayoutRenderer.tsx:139,144`). Left as is, on a 0~1px pane the border exceeds the allocated width and overlaps the neighbor.
  - Outer **frame** = rectangle position·size + `overflow:hidden` + **`data-slot-id` + `onClick` (click-to-focus) + `onContextMenu`**. No border.
    - Why the events are on the frame: tests (`ViewLayoutRenderer.test.tsx:679,706,826` click · `:781,834` right-click) and cdp QA recipes fire events straight at `[data-slot-id]`. Inner clicks (including the icon `:699`) reach the frame by bubbling.
  - Inner **border element** = `data-slot-border` · `position:absolute; inset:0; border:1px; box-sizing:border-box; background`. The **overflow/centering depending on hasContent** that the wrapper did (`ViewLayoutRenderer.tsx:147-158`) comes here.
    - It holds the content·`+` icon·focus ring·menu. The metrics `frame_insets` are also measured from this element.
  - Even if the frame is 0~1px, the border element is clipped by the frame's `overflow:hidden`, so panes do not overlap each other. The shell's `frame` is exactly the same as the screen's frame.
  - Tests that looked at the border·centering on the frame **move their target to the inner element** (§5 — not deleted. If not moved, the `!== 'center'` assertion passes vacuously).
- **Position = round the shell rectangle in CSS.** Each leaf carries only the inline custom properties `--x0 --y0 --x1 --y1`, and the rules live in static CSS.
  - Default: `left: calc(var(--x0)*100%)` · `width: calc((var(--x1) - var(--x0))*100%)` (top/height likewise).
  - Inside `@supports (width: round(nearest, 1.5px, 1px))`: `left: round(nearest, calc(var(--x0)*100%), 1px)` · `width: calc(round(nearest, calc(var(--x1)*100%), 1px) - round(nearest, calc(var(--x0)*100%), 1px))`.
  - This shape is the same formula as the shell's `px_edge`. Neighboring panes use the same boundary formula, so gaps·overlaps are 0.
  - Why split with `@supports`: a declaration containing `var()` is not filtered out as invalid at parse time but becomes `unset` at computed-value time, so a fallback that writes the same property twice does not work.
- **Precision:** per boundary, shell px and screen px are **0 normally and 1 CSS px at worst**. A 1px mismatch arises in only one of three cases. ① the CSS size of the window canvas is not an integer (scale ≠ 1 — the shell holds the rounded integer) ② the boundary is near a .5 tie (Chromium's 1/64px layout-unit quantization) ③ `round()` is unsupported.
- **No transform·`contain`** — `SlotContextMenu` is `position:fixed` (`src/components/slot/SlotContextMenu.tsx:121,127`), so if a transform is applied to an ancestor, the coordinate system breaks.
- First paint: the position is the style of the render result itself. No measurement and no ResizeObserver is needed, and CSS also handles window resize in that same frame (D4 = pure proportional).
- props: **the tree prop name `node` is kept**, not to mention the name·path of `ViewLayoutRenderer`. The 51 call sites (`node=` in `ViewLayoutRenderer.test.tsx`) and `slotDaemonRestart.test.tsx` run without modification. *Implementation:* the prop name did stay as is — the split-branch tests were rewritten per §5, so the number of call sites changed, and that call in the latter file is now (step 9) at `:220` (shifted by deleting the allotment mock).
  - Optional props added: `slotRects`·`splitRects`·`ratioBounds`·`version`. `TabCanvas` (`WindowLayout.tsx:213-229`) passes them from the cache.
  - **If there are no rectangles and the root is a slot, draw the full box.** If there are no rectangles and the root is a split, log an error and draw nothing (it does not occur on the production path — the snapshot always carries them).
    - *Implementation:* rectangles are used only when the set of pane·split ids is **exactly** the same as the tree (no missing, no extra, no duplicate). If they disagree, handle it the same as when absent — if the root is a slot, the full box; if a split, draw nothing. Both log an error (a single pane with no rectangles at all is the only permitted input, so it is silent). Drawing with mismatched rectangles would drop panes and make splitters drag splits that are not in the tree.
    - *Implementation:* if the view coordinate or the ratio limits (`ratioBounds`) are unknown, lock the splitters — there is neither a place to confirm nor a range.

### 2f. Splitters — preview·confirm·reconcile

- **Shape:** an overlay centered on the split boundary `at`. Layout space 0, hit area 8px, visible line 1px `var(--border)`, `var(--accent)` while hovering·dragging (the same color as the current `index.css:37-40` mapping), `zIndex:20` (above the focus ring's 10, below the menu's 1000). `data-split-id`·`data-dir` are attached for cdp.
- **Drag:**
  - On `pointerdown`, compute the axis length `L` = **the integer canvas px this window reported to the shell** × the snapshot's split box width. It is the same input as the shell enforcement (§2c). The root `getBoundingClientRect()` is used only to capture the pointer origin. Before the canvas report, it uses `getBoundingClientRect()` instead.
    - *Implementation:* the range is computed per splitter **at render time**, not at `pointerdown` (`useSplitDrag.ts`). The canvas = the value this window reported to the shell and that **succeeded**, and since it subscribes to the report module (§2d), it recomputes when the report changes. Before the report it uses the root `getBoundingClientRect()` rounded to an integer (known limit: a change of root size while the renderer is not redrawn is not seen, and when the report arrives that is used). The box width is the pre-remap snapshot value — it must be the same input as the shell, bit for bit. `Splitter` reads the range at that moment each time the pointer moves, and cancels if that split gets locked during the drag or its direction·split id changes. The root rect is used once on press to capture the pointer origin·size.
  - The allowed range is `[max(ratio_min, 30/L), min(ratio_max, 1 − 30/L)]` — the same limits as the shell clamp (`ratio_min/max` of the snapshot — the screen does not copy 0.1/0.9), overlaid with the condition that each of the split's two sides (the whole subtree) is at least 30px (D2). If the range is empty (`L < 60`), that splitter does not move.
  - `pointermove` is coalesced with rAF (orca `pane-divider-drag.ts:251-253`). `setPointerCapture` and a window-level listener are installed together (Chromium loses the capture in an instant — orca `:97`). During the drag the `body` cursor is fixed and `user-select:none` is set. A click released without movement sends no command.
- **Preview computation = a linear remap of the shell rectangles** (the frontend does not get a second tree-geometry implementation). With the dragged split's box `[X0,X1]`, old boundary `B`, new boundary `B'`:
  - the a-side coordinate is `X0 + (x−X0)·(B'−X0)/(B−X0)`, the b-side is `X1 − (X1−x)·(X1−B')/(X1−B)`, and `x == B` becomes `B'` as is.
  - This is the same result as the shell recomputation that shrinks proportionally with descendant ratios fixed (differences in the last floating-point digits are overwritten by the confirmed snapshot). The splitters inside that subtree receive the same mapping.
- **Confirm:** call `set_split_ratio` **once** on `pointerup`. **Double-click** = 0.5 on the same command without a preview (D5).
- **Reconciliation rules** (preview state = `{ token, splitId, ratio, phase: 'drag' | 'commit', awaitVersion? }`, one per view):
  1. A snapshot arriving during `drag` reflects everything else. Only the dragged split keeps the preview value. If that split id disappears from the snapshot, cancel the drag (release capture · delete the preview · no command).
  2. On release, go to `commit`. Decide by the response's `outcome`.
     - `Applied` — put `version` in `awaitVersion` (it is the version of the snapshot notified to that view). The moment the cache version becomes at least `awaitVersion` (the response time or a later snapshot), discard the preview — after that, screen = shell value.
     - `Unchanged`·`TooSmall` — no snapshot will come, so discard the preview **immediately**. The screen returns to the cache (= the shell value).
  3. Before the response, or on a snapshot below `awaitVersion`, keep the preview (so it does not jump back to the old value).
  4. If the command fails, discard the preview (return to the shell value) + `console.error`.
  5. If a new drag starts before the earlier confirm, the new `token` overwrites it. An old response arriving late is ignored because its token differs.
  6. If the response has arrived but the `awaitVersion` snapshot does not arrive within 500ms (event loss), pull `get_view` once (the same pull as `WindowLayout.tsx:116`).
  - *Implementation — two decisions outside the rules (`useSplitDrag.ts`; the source of truth for rules 1~6 is `reducePreview` in the pure reducer `splitPreview.ts`):*
    - **If at the moment of release the preview is already not this drag's, send no command** — either a new drag on another splitter took it (two pointers — rule 5), or a snapshot deleted that split id and so it was cancelled (rule 1). The value shown on screen already belonged to that other drag (or was the shell value), so sending would confirm a value that was never visible. The decision does not wait for a render and is made on the state with the events already sent folded in.
    - The preview·confirm response·pull are bound to the view that started them. If the view changes within the same renderer, the preview and any gesture in progress are discarded, and the response·pull of a confirm started in the old view do not reach the new view.
  - As a result, if an LLM write arrives at the shell later than the user's confirm, the LLM value is seen; if earlier, the user's value wins — the last write in arrival order.
- Call wiring: add `setSplitRatio` to the existing invoke bundle of `viewStore` (next to `split` at `src/store/viewStore.ts:169`).
- It is not registered in the frontend command registry (`window.__engramCmd`) — the LLM path is the single bus command `split.setRatio`. We do not create a new precedent where the same id differs between two surfaces (see the `slot.popout` comment in `commands.rs`).

### 2g. Moving per-slot state

- Move everything the renderer instance held — `lastMountRef` (`ViewLayoutRenderer.tsx:62`) · menu coordinates (`:67`) · derived values·effects (`:72-97`) · the slot branches (`:99-273`) — to `LayoutLeaf` (key = slot id), **logic unchanged**.
- **Effect:** the preserved memory and the terminal instance survive after a split·close·promotion.
  - A split keeps the original slot as `a` and keeps its id (`src-tauri/src/layout/tree.rs:57-77`).
  - So the renderer comment 「알려진 한계 ①」 (`:57-60`) and the 「승격된 서브트리는 다시 마운트된다」 in the 「영향」 of ADR-0223 are resolved.
  - A popout is a new slot id·new webview, so it still remounts.

### 2h. Shell neighbors·order — not breaking even on very thin leaves

The px enforcement of §2c protects only **the two sides of the dragged split**. If an ancestor splitter is dragged, a deep leaf still shrinks proportionally (D2), and a leaf whose ratio product goes below 1e-4 can occur. So `spatial.rs` must be correct regardless of size.

- **Current handling (checked in the code):**
  - There is no panic path. The order is `partial_cmp(..).unwrap_or(Equal)` with a stable sort (`spatial.rs:212-222`), so it is deterministic. Corner selection also keeps the first one seen on a tie (`:305-317`). There is no division, and `ordinal_of[&i]` (`:241`) is safe because every index is in the order table.
  - But the adjacency decision uses an **absolute EPS** (`EPS = 1e-4` `:28`, `edge_eq` `:55`, `overlap > EPS` `:184`). On a leaf whose width·height is below EPS it is **silently wrong, without panicking.**
    - ① If that leaf is split again, the child overlap is exactly EPS and so is dropped (a neighbor is lost).
    - ② A pane beyond it is misjudged as adjacent by `edge_eq` and skips the thin leaf (the comment at `:66-77`, ADR-0068 「영향」 :43).
    - The same comment itself writes that the per-split `[EPS,1-EPS]` clamp (`:78`) is a product along the path and cannot prevent this.
- **What changes:**
  - `spatial.rs` takes the boundary-form f64 rectangles of `geometry.rs` as input (replacing `leaf_rects` `:115`).
  - **Adjacency becomes an exact comparison** (`me.x1 == other.x0`), and **overlap becomes `> 0.0`**. Reason: in a guillotine layout, the boundary where two panes touch is always one split's `at`, and `geometry.rs` copies that one value into both descendants (§2a). So **a true adjacency is equal bit for bit regardless of size and is never missed.**
  - False candidates remain in theory. Panes touching only at a corner have a mathematically zero overlap on the orthogonal axis, but if their two endpoints are computed separately along different split paths, the overlap can become positive by the rounding error. ★*Implementation:* that error is not bounded by 1 ulp★ — the deeper the path, the larger it can grow, to several ulps (code comment now (step 9) `spatial.rs:86-89`).
    - This candidate loses to the **rule that picks the candidate with the largest orthogonal-axis overlap** (now (step 9) `spatial.rs:108-113`). The opposite side of my edge is fully covered by leaves, so there is a true neighbor actually touching my edge, and normally that overlap is far larger than the error.
    - ★*Implementation:* this is not a guarantee★ — if my own pane is about to collapse to a few ulps in thickness, that margin disappears and ties·reversals can occur (the same comment). What the test pins is the single case of a 1 ulp mismatch (`one_ulp_corner_candidate_loses_to_true_neighbor` at `spatial.rs:625` — it first measures the premise, a 1 ulp difference, with `to_bits`).
  - Panes touching only at a corner (overlap 0) are not adjacent — the same as now.
  - The write path does not create a leaf of area 0 — ratio writes and splits are filtered by the f64 representability rule (§2c).
  - **Even so, if a leaf of area 0 comes in, handle it deterministically** (defense — a value planted directly in the tree by a test, etc.): that leaf has no neighbors and is no one's neighbor (it is skipped before the adjacency decision). The order is assigned by the rule below so it is not missing from the list. No panic.
    - *Implementation note:* the corner token (`corner_slot` in `resolve_spatial` — `spatial.rs:305-317`) also skips an area-0 leaf. It is the same rule as excluding neighbors. There is never a case where only area-0 leaves remain (a view always has a positive-area leaf).
- **Order:** compare center points with `f64::total_cmp` to make a total order even for NaN. Ties are kept deterministic by tree pre-order (stable sort).
- **What is removed:** the `EPS` constant · `edge_eq` · the `[EPS,1-EPS]` clamp. The existing spatial tests are the regression net. The degenerate-ratio test at `:515` is kept, changing only its target to 「geometry 클램프가 면적 0 잎을 막는다」.

## 3. User decisions (confirmed 2026-09-24)

| # | Decision | Discarded options · reason |
|---|---|---|
| D1 | Drag size is **written back to the shell.** During the drag the screen previews by split id, and on release it calls `split.setRatio` once. The shell clamps to 0.1~0.9, writes to the tree, raises the version and notifies as usual. The LLM uses the same command. Concurrent writes: the last write in shell arrival order. | Screen-only override — **user:** 「어차피 레이아웃을 셸로 관리하니 사이즈가 있으면 좋을테니 진행하던지」 (the neighbor computation in `spatial.rs` already uses ratios). Throttled sending during the drag — **user's criterion:** 「동기화 비용 vs 사용 횟수? 정도로 판정해야될듯」 (neither cost has been measured). |
| D1+ | The shell holds **px geometry**. Fixed UI metrics are measured and sent once at initialization, and the window canvas px is sent only when it changes and is kept in the window entry (`WindowTabs`). Pane px = canvas × normalized rectangle. The shell does not ask the UI. | (a) Reporting the measured size per pane on every change — **user:** 「아니아니 내말은 기준선이나 그런건 아주 최초에 한번 보내면 된다는 얘기였는데 레이아웃 계산 완전 기본정보」 (only the basic information once, the computation in the shell). (b) Hard-coded margin·offset constants on the shell side for canvas × rectangle — **user:** 「그 픽셀들은 View에서 가져와서 적립해야된다는거알지? 하드코딩하지말고.」. (c) Per-view·per-pane storage — **user:** 「어쨋든 window 클래스에 정보가 잇으면 되겠네」 (the tabs of one window share the canvas — `WindowLayout.tsx:182-197`). |
| D2 | Minimum = **each of the two sides (whole subtree) of the dragged split** is 30px. During the drag the screen enforces it. The shell cuts the ratio to 0.1~0.9 and, if it knows the canvas·metrics, also enforces the same 30px condition (§2c — the value is what the screen sent · ★this shell-side enforcement is an internal decision the review added, not the user's D2★). Dragging an outer splitter so that a deep pane goes below 30px is **accepted by the user**. Even so, the design keeps shell neighbors·order from breaking (§2h). | Propagating the sub-minimum through the whole tree (tmux·VS Code·Unity). **User:** accepted the main agent's reason for its recommendation — most peers look only at the two children of a split, and propagation needs a whole-tree algorithm. |
| D3 | No keyboard adjustment of splitters. Adjustment = mouse (human) + `split.setRatio` (AI). No relative increment/decrement command for now either. | APG splitter focus · relative increment/decrement command. **User:** 「사실 AI에게도 크게 필요한 기능은아닌데 어쨋든 키보드로는 필요없고 마우스 AI 가 되도록해. A안이 되는건가?」 (A = no extra command). |
| D4 | No minimum window size. Panes shrink proportionally to the end (screen = shell ratio). | A minimum window size. **User:** 「상관없긴한데 창 크기를 구지 막을 이유는 없다고 생각」. |
| D5 | Splitter double-click = **50:50** with the same command. | No action. **User:** 「필요하면 그냥 넣고. 넣는다고 손해는 아니라서」. |
| D6 | A flat list sorted by slot id. Existing pane containers do not move in the DOM (only position values change, new panes are inserted, closed panes are removed). **Accepted:** Tab·screen-reader order ≠ screen order. | Screen order — **user:** 「기존 컨테이너가 변경이 되지 않는게 좋지」 (side effects of DOM moves — orca `pane-split-scroll.ts:83-93` notes the `scrollTop` reset and early WebGL release, and the Chrome `moveBefore()` blog notes focus loss — policy survey F6). |
| R11 | A split that creates a pane smaller than `min_pane_px` is also allowed. The only thing blocked is the extreme where the width becomes unrepresentable (0) (`SplitTooDeep`). | Rejecting a split below the px minimum when the canvas is known — **deferred**. **User:** 「0이되는 극단적인 상황만 막으면 될것같아 일단은」 · acknowledged the concern: 「너무 작아서 유저가 해당칸을 조작 못하는것도 문제긴 하겠네. 적당히 막던지.」 (then chose to block only the extreme). Conditions for reopening · peer evidence = §7 R11. |

## 4. Internal choices I made (report)

- **Files:** shell `layout/geometry.rs` (new) · `tree.rs`·`manager.rs`·`apply.rs`·`types.rs`·`commands.rs`·`commands/layout.rs`·`lib.rs` (modified). Screen `src/components/layout/LayoutLeaf.tsx`·`Splitter.tsx`·`splitPreview.ts` (pure preview state·remap·range)·`ViewLayoutRenderer.tsx` (flat root) · canvas measurement is a hook inside `WindowLayout.tsx` · the position-rule CSS is in `src/index.css`.
  - *Implementation:* three more screen files appeared — `useSplitDrag.ts` (a hook that attaches token issuing·confirm call·pull timer·allowed range to the preview reducer) · `windowCanvasReport.ts` (canvas report state — §2d) · `uiMetricsReport.ts` (metrics report — §2d).
- **Names:** bus `split.setRatio`·`split.list` (object first — the same word order as `slot.split`) · Tauri `set_split_ratio`·`report_window_canvas`·`report_ui_metrics` (snake_case — precedent `split_slot`·`report_view_commands`).
- **Shell px minimum = clamp (not rejection)** — the applied value is returned. If the range is empty, `TooSmall` reports no change (§2c). It has the same shape as the drag path, so the LLM and humans see the same result.
- **Result signal = three `outcome`s (`Applied`·`Unchanged`·`TooSmall`)** — it distinguishes no-change from 「적용됨」, and the screen discards the preview immediately on no-change. The alternative of keeping a per-view version separately would have changed the manager structure, so it was not chosen.
- **Ratio storage `f64`** — the received value comes back as is. The binding body is unchanged.
- **The ratio limits ride in the snapshot** (`ratio_min/max`) — the screen does not copy constants.
- **Pane frame/border separation** (§2e) + the shell `content` width·height `max(0, …)` — frame = rectangle and no overlap even on 0~1px panes.
- **Report-command defense:**
  - No label argument (`tauri::Window`).
  - A 0-size canvas is not sent by the screen and is also ignored by the shell. Metrics are range-checked.
  - inset is `f64`. `min_pane_px` is a policy field kept apart from the measured inset.
  - 「보냈음」 is set only after success.
- **Canvas measurement = callback ref + RO-absence guard.** On failure it does not wait for RO and retries with bound. The metrics report goes once through the in-flight latch.
  - *Implementation:* the canvas report state is a **webview singleton** in `windowCanvasReport.ts` — only the latest value to send · one invoke at a time · only successful values are recorded · subscribers are notified on change (§2d).
- **Frame = events·`data-slot-id`, inner `data-slot-border` = border·centering·content.** Style assertions move their target to the inner element and a positive counterpart is added.
- **f64 representability rule** — a split is rejected with `SplitTooDeep`, and a ratio write is `TooSmall` if it **newly** creates an area-0 leaf. No px-based split rejection is done (R11 — user decision).
- **Returned version = the version of the snapshot taken inside the lock.**
- **Sorting:** leaves by slot id, splitters by split id.
- **Renderer prop name `node` kept.**
- **Constants:** `RATIO_MIN/MAX = 0.1/0.9` (shell — goes to the screen through the snapshot) · `MIN_PANE_PX = 30` (screen only — the shell receives it through the metrics) · metrics allowed range inset `0~64`·`min_pane_px` `1~1000` · splitter hit area 8px · line 1px · hover delay 300ms · splitter `zIndex 20` · canvas debounce 100ms · pull wait after confirm 500ms.
- **Coordinates:** `f64` boundary form. Shell `round` = CSS `round(nearest)` (the same for positive values).
- **`split.list`** was added because it is the LLM's only way to find a split id (scope addition).
- Leaf `React.memo` — a leaf whose box did not change during a preview is not re-rendered. The `key={node.id}` (`:219-224`) of the concrete renderer inside a leaf is left as is.
  - *Implementation:* the leaf was split in two (`src/components/layout/LayoutLeaf.tsx`). The outer **position frame** (`LayoutLeaf`, memo — `--x0..--y1`·`data-slot-id`·click focus·right-click menu state) and the **body** (`SlotBody`, memo — `:112` · ★it does not receive the rectangle★ · `[data-slot-border]`·content·focus ring·menu·error boundary·ADR-0148 memory). The remap returns rectangles outside the box as the same object, so those leaves do not redraw even their frame, and even for a leaf whose rectangle changed, only the frame is redrawn each drag frame (if the body received the rectangle, the terminal·markdown would be redrawn every frame too). The concrete renderer's `key={node.id}` is unchanged.
- **Decided by the main agent during implementation (not a user decision — report, 2026-09-25):**
  - **`slot_px` is `Some` only when both canvas and metrics are known** — giving it from the canvas alone would make `content` (frame − insets) a wrong value. A nonexistent view·slot is `Err` before that (§2d).
  - **The spelling of the ratio-write outcome is the same on both surfaces** — `RatioOutcome` of the bus `split.setRatio` (the declaration-macro enum) and `SplitRatioOutcome` of Tauri `set_split_ratio` carry the variant names `Applied`·`Unchanged`·`TooSmall` as they are. Only the type names differ (the same relation as `ThemeOrigin`↔`ThemeSource`) and the bus handler does the conversion. The spelling is matched now (step 9) by `both_surfaces_spell_the_split_ratio_outcome_the_same_way` at `src-tauri/tests/layout_commands.rs:1061`.
  - **A nonexistent view·split on the bus is `CONFLICT`** + the apply service's reason text — the one code that every apply failure in this table uses (the header 「적용 실패는 코드 하나로 나간다」 of `src-tauri/src/layout/commands.rs`). A malformed id is `INVALID_ARGUMENT`, and the rejection text for a malformed `split_id` points to `split.list`.

## 5. Test plan

**Shell (Rust)**

- `geometry.rs` (lib_unit):
  - Single slot = (0,0,1,1). The two ADR-0140 directions.
  - Neighbor boundaries `==` identical (including 3-level nesting). Split box·`at`.
  - Cross-agreement with the `spatial.rs` rectangles at 1e-6.
    - *Implementation:* ★it existed only in step 1 (`e401966`)★ — 1b (`3f8c4ef`) removed `spatial.rs`'s own rectangle computation, so the reference for the comparison was gone, and the test was removed with it. Now `spatial.rs` takes this module's boundaries as input, so there is no place for the two shell geometries to diverge. Measuring agreement between the shell formula and the screen formula is not this test's job — for the allowed range, `splitPreview.test.ts` matches it bit for bit against an oracle that ports the shell px clamp formula, and for agreement between the CSS-rounded boundary and the shell `px_edge`, jsdom has no layout, so only the GUI measurement (seam 0 — §7 R1) looked at it.
  - `px_edge` rounding · neighbor px boundaries identical · `content = frame − insets` · no canvas → `None` (*implementation:* this last one belongs to the manager `slot_px` test, and it is `None` if either canvas or metrics is missing).
  - **0px·1px frames**: in a tree where shrinking the canvas makes pane frames 0px·1px, the `content` width·height is 0 (not negative) and the neighboring frames' px boundaries do not overlap.
  - A tree value outside the ratio limits (`0.0`) is clamped.
- `spatial.rs` (lib_unit): keep all existing tests. Added:
  - In a tree where 0.1 is nested four levels on the same axis and then the ancestor is set to 0.1 so that a leaf width went below 1e-4, the left·right neighbors of the thin leaf are exact (no skipping·loss).
  - The order is deterministic (the same tree twice = the same result). No panic.
  - Panes touching only at a corner are not neighbors. Even if there is a false candidate whose corner endpoint is off by 1 ulp so the overlap became positive, the true neighbor is chosen.
  - An area-0 leaf planted directly in the tree: no neighbors · no one's neighbor · included in the order list · no panic · the same result when computed twice.
- `tree.rs`·`manager.rs` (lib_unit):
  - Ratio write: find by id · nonexistent id = `SplitNotFound`·no change · non-finite = `InvalidRatio` (in the manager) · 0.1/0.9 clamp · `0.3` comes back as exactly `0.3` (`f64`).
  - `outcome`: change = `Applied`·version +1 · same value = `Unchanged`·version unchanged · `L < 2m` = `TooSmall`·no change.
  - **Allow small splits (R11):** in a world with the canvas·metrics planted, a split whose two resulting panes are below `min_pane_px` (30) also succeeds. The tree changes and there is no error.
  - **Degenerate chain (kept):**
    - If the same-side child keeps being split at 0.5, `SplitTooDeep` occurs at the level just before the boundary becomes equal to the parent's end, and the tree is unchanged and there is no panic.
    - The 0.9-side chain is the same.
    - A request that changes an ancestor ratio so that one deep descendant becomes area 0 is `TooSmall`·no change.
    - In both cases every leaf of the resulting tree has positive area.
  - px minimum: if canvas·metrics exist, clamp to the `m` px boundary of the two sides and return the applied value. If the canvas or metrics is missing, only the ratio clamp.
  - Canvas storage: per window · a hidden tab also uses the same window canvas for `slot_px` · a new popout window is `None` · disappears with the window closing · a report leaves the version unchanged · **a 0-size report is ignored (last value kept)** · **an out-of-range metrics report is rejected (last value kept)**.
- `tests/layout_apply.rs`:
  - `Applied` notifies the layout exactly once (including the new ratio·rectangles), `Unchanged`·`TooSmall` zero times, tab notifications zero. Canvas·metrics reports notify zero.
  - **Returned version == the `version` of the notified snapshot** (captured in the same critical section — §2c).
- `tests/layout_commands.rs`:
  - Add the two names to the list golden (`:440-466`) · generation 8.
  - `split.setRatio` argument validation (bad uuid·non-finite = `INVALID_ARGUMENT`, out of range = the clamped value returned). In a world with the canvas·metrics planted, the LLM ratio is clamped to the px minimum and the `outcome` is carried.
  - The a/b slot lists of the `split.list` rows.
  - The a-side definition is in both commands' summaries.

**Screen (vitest)**

- `splitPreview.test.ts`:
  - Each of reconciliation rules 1~6. On a `Unchanged`·`TooSmall` response the preview is discarded immediately (no pull wait).
  - Remap: boundary `B → B'` exact · shared boundary kept · subtree splitters.
  - Range: two sides 30px · snapshot `ratio_min/max` · `L<60` lock · the same value as the shell formula on the same input (reported canvas × box).
- `Splitter.test.tsx` (root `getBoundingClientRect` mock):
  - down/move/up → exactly one invoke with the applied value.
  - A click without movement → 0 times. Double-click → 0.5 once.
  - cancel/blur → restore·0 times.
  - Whether jsdom supports PointerEvent·`setPointerCapture` is **unknown** → call them optionally and push with `fireEvent` coordinates.
- `ViewLayoutRenderer.flat.test.tsx` (replacing `.allotment.test.tsx`):
  - **No remount** — a per-slot-id mount-counter mock + DOM node identity before and after + a `MutationObserver` checks that surviving leaves are not detached.
    - Scenarios: close (`LR{x,TB{y,z}}` → y) · promotion (`LR{TB{x,y},z}` → z) · split (x → `LR{x,new}`) · direction change with the same split id · ratio-change snapshot · **an ADR-0148 preserved view is the same instance after promotion·split**.
  - Leaf·splitter DOM order = slot id·split id order respectively (the splitter nodes do not move even after an LLM restructure). `--x0..--y1` are set on the first render (without a global ResizeObserver). No transform. `data-slot-id`·`data-split-id`.
  - The pane frame has no border and the inner border element (`[data-slot-border]`) is inside the frame (the frame style is exactly the rectangle even for 0px·1px frame rectangles). The click·right-click handlers and `data-slot-id` are on the frame.
- `WindowLayout.test.tsx` (an RO stub in the same commit — vitest has no setupFiles and jsdom has no `ResizeObserver`. The existing mock `:37-49` swaps only the renderer):
  - Even if `win` arrives late, observation is attached by the callback ref.
  - Debounce → integer → `report_window_canvas` only on change · no resend of the same size · 0×0 not sent.
  - On failure it resends by bounded retry **even without an RO change**. If a new size arrives during a retry, the old retry is cancelled.
  - Metrics retry until success, once per webview after success. Even if N leaves mount simultaneously, one report through the in-flight latch. If it ends in failure, the latch is released and the next mount retries.
    - *Implementation:* the assertions of this line are in `src/components/layout/uiMetricsReport.test.ts`. The one on the leaf-mount side (it does not resend even when leaves increase) is in `ViewLayoutRenderer.flat.test.tsx`.
  - It does not throw in an environment without RO.
- *Implementation:* `src/components/layout/windowCanvasReport.test.ts` — subscription notification of the canvas report singleton (§2d) (only successful values · unsubscribe · test reset · a late success before the reset is ignored). Observation·debounce·retry are as in `WindowLayout.test.tsx` above.
- *Implementation:* `src/components/layout/useSplitDrag.test.tsx` — confirm·reconcile · per-splitter gesture token · pull after confirm (rule 6) · view switch · allowed range (reported canvas × snapshot box) · reference retention of the rectangles to draw.
- **Kept:** the slot branches (`:231-560`)·click focus (`:660-728`)·menu (`:730-`) of `ViewLayoutRenderer.test.tsx` — the prop name `node` as is, running with the root-slot full-box rule (§2e). Events fired at the frame (`[data-slot-id]`) reach as before (`:679,699,706,826` click · `:781,834` right-click).
- **Moving the target (not deleting)** — move the style assertions that looked at the frame to the inner `[data-slot-border]`. If not moved, the selector returns an empty array and throws, or `!== 'center'` passes vacuously.
  - `emptyIcons()` (`:191-193`, used in 7 places) selector `'[data-slot-id] > svg'` → `'[data-slot-border] > svg'`.
  - Border assertions `:263-264`·`:270-271` (`wrapper.style.border`) → the inner element. The focus ring overlay lookup (`:272`) is also relative to the inner element.
  - Centering-absence assertions `:282`·`:291`·`:304-305` (`justifyContent`·`alignItems`) → the inner element.
  - After moving, add one more **positive counterpart** — in an empty slot the inner element really is `center`. That prevents the case where an empty target still goes green.
- **Deletion·rewrite:**
  - Delete the stub `:66-101`.
  - Split branches `:563-586` (recursive two children·child caps) → rewrite with shell rectangle fixtures.
  - Delete `:587-599`. `:601-627` → no remount. `:629-658` → shell-rectangle based.
  - Delete `.allotment.test.tsx`. The 5 error-isolation tests (`:337-436`) draw a split tree, so remove the fake RO and move them **with rectangle fixtures attached**. The `'[data-slot-id="bad"] > svg'` (`:381`) in them also moves its target to `[data-slot-border]`.
    - The fixtures are made by a test-only helper `src/components/layout/testing/rects.ts`. It only needs to guarantee non-overlapping rectangles and does not assert that the values equal the shell's — geometric correctness belongs to the Rust `geometry.rs` tests.
  - Delete `slotDaemonRestart.test.tsx:72-74`.
  - Add a new pane to `viewStore.test.ts`·`WindowLayout.test.tsx`, which build snapshot literals.
- **GUI measurement (`/qa full`) is required** — jsdom has no layout. To check:
  - First frame of close·promotion · seams·text sharpness·column count at odd widths and 125% scaling.
  - Drag feel · no jump after confirm · the LLM `split.setRatio` taking effect · double-click.
  - WebGL seat · popout window canvas report · hidden-tab switching.
  - Measure `CSS.supports('width','round(nearest, 1.5px, 1px)')`.

## 6. Order of work (green at every commit)

| # | Layer | Content | Gates that apply |
|---|---|---|---|
| 0 | Docs | Record the ADR (§8) + link this TRD in step-log (no orphans — `docs/README.md:47`) | `/review doc` |
| 1 | Rust | `geometry.rs` + tests (no consumer). Read the ratio with `f64::from(ratio)` — it compiles whether the stored type is `f32` or `f64` | workspace regression · `--test lib_unit` · fmt |
| 1b | Rust | **Ratio storage `f32` → `f64`** (`types.rs:78`·`tree.rs` literals) + `spatial.rs` takes the `geometry.rs` boundaries as input and changes to exact adjacency·`total_cmp` order·deterministic handling of area-0 leaves · remove EPS · thin-leaf tests (§2h) — one commit. The `f32` EPS clamp at `spatial.rs:78` does not compile together with an `f64` ratio, so the two are not separated | lib_unit (all existing spatial tests) · `--test layout_apply` · binding sync (confirm the `LayoutNode.ts` body is unchanged) |
| 2 | Rust+TS | `slot_rects`·`split_rects`·`ratio_min/max` in the snapshot · new bindings `SlotRect.ts`·`SplitRect.ts` + regenerate `ViewSnapshot.ts` · re-export in `layoutTypes.ts` · `CachedView` storage · update snapshot-literal tests (same commit — they are required fields, so tsc breaks) | lib_unit (ts-rs export) · **CI binding sync gate** (`src-tauri/bindings/`, new files are caught by the `git add -N` path — commit the generated files) · `npm test`·`tsc` |
| 3 | Rust+TS | `WindowTabs.canvas/metrics` (3 constructors) · `report_window_canvas`·`report_ui_metrics` (`tauri::Window`, ignore 0 size, range check) · `slot_px` (content `max(0,…)`) · `WindowLayout` canvas measurement·report (callback ref, RO-absence guard) + **the `WindowLayout.test.tsx` RO stub in the same commit** — the ratio command's px enforcement reads this storage, so **before the command** | lib_unit · layout_apply · `npm test`·`tsc` |
| 4 | Rust | `set_ratio_in_tree` · `SplitNotFound`·`InvalidRatio`·`SplitTooDeep` · `set_split_ratio` (manager — finite check + ratio clamp + px-minimum enforcement + area-0 filtering + `outcome` · the apply service returns the version of the snapshot inside the lock · Tauri·`generate_handler`) · `split_slot` representability guard · bus `split.setRatio`·`split.list` (a-side definition in the summary) · generation 8 · `SplitRatioApplied.ts` (version fixed to `number`) | lib_unit · `--test layout_apply` · `--test layout_commands` (list golden) · binding sync |
| 5 | TS | **Only move** the slot branches to `LayoutLeaf.tsx` (the recursive allotment calls it at the leaf position) — behavior unchanged | `npm test`·`tsc` |
| 6 | TS | `splitPreview.ts` + `Splitter.tsx` + `viewStore.setSplitRatio` + tests (not wired) | `npm test`·`tsc` |
| 7 | TS | Replace the `ViewLayoutRenderer` body with the flat root (keep prop `node`) · pane frame/border separation · `TabCanvas` optional props · position CSS (`index.css` — the `src/index.css.test.ts` gate that reads the source text runs along) · metrics report (first leaf) · rewrite split tests (`:563-658`) · move the target of frame style assertions (`emptyIcons`·`:263-305`) + positive counterpart · test-only rectangle helper · new `.flat.test.tsx` · delete `.allotment.test.tsx` (move the 5 error-isolation tests) | `npm test`·`tsc` · **`/qa full`** |
| 8 | TS | Remove allotment: `main.tsx:5-8` · `index.css:33-40` · `package.json:28`+lock · `slotDaemonRestart.test.tsx:72-74` · the `TerminalSlot.tsx:301` comment | `npm test`·`tsc` · `/qa full` |
| 9 | Docs | The header stamps of ADR-0223·0068·0140·0168 are already in (ADR-0227 reservation `fad7a7f` + scope widening). **The body of ADR-0223 「영향」 is not fixed** — ADRs are accumulated, not overwritten (`docs/decisions/README.md:13`). The widened stamp (「결정 2 Allotment key와 영향의 승격 재마운트 조항」) covers that clause · the renderer's 「알려진 한계 ①」 comment and the wrong ADR-0063 citation at `ViewLayoutRenderer.tsx:291` disappear with the renderer replacement (7) · `CLAUDE.md` tech stack · the `qa.md:262` line pointer · the `types.rs:118-120` comment · **the `types.rs:73-75` `Split.id` doc comment** (「프론트는 렌더러 인스턴스를 이 값으로 가른다(`ViewLayoutRenderer`)」 → splitter·preview key) + the 「0.0~1.0 클램프」 at `:71` → 0.1~0.9 (the lines of these three are now (step 9)). The id comment is exported into `LayoutNode.ts`, so **regenerate the bindings and include them in the same commit** (lib_unit + CI binding sync gate) | `/review doc` · lib_unit · binding sync · since the stamps are already on this branch, the pre-merge condition is met — do not revert them |

- 1~4 do not change screen behavior (the new fields·commands are not used yet). 1b changes the shell's neighbor·order answers only on thin leaves (they become correct). 7 is the only commit that changes what users feel.
- 7 is the largest — replacing the renderer and rewriting two test files must be in one commit to be green. 2 ties the Rust·bindings·TS test literals into one commit.
- *Implementation:* the order is as in the table (5·6 ran review·QA together). The per-step commits are recorded by this implementation round's entry in `docs/process/step-log.md`.

## 7. Risks · open items

- **R1 Fractional px and xterm** — rounding (§2e) makes pane boundaries integer CSS px. What remains:
  - At scale ≠ 1, an integer CSS px is still fractional in device px, so text may blur — **we do not know whether it blurs** (policy survey F6).
  - Whether the installed WebView2 runtime supports `round()` is **unconfirmed** (Edge 125+ supports it — via caniuse). If unsupported, it falls back to % without rounding.
  - *Implementation — the answer from the step-7 GUI measurement (a PC at 125% scaling, 2026-09-25 · source = the body of `d014c0c`, the inset value·observations are from that round's handoff note `.claude/handoff/history/20260925-085850-평평한-렌더러-1-7단계-완료-다음은-8단계.md:26-28`):*
    - This WebView2 supports `round()` — the rounding path really runs.
    - Measured pane border inset = **0.8px** (125%) — the reason `frame_insets` is received as `f64` actually appeared.
    - Seams between panes **0** — odd widths · DSF 1.0/1.25/1.5 · native window resize driven by a helper (dragging the window edge by hand is the user's GUI check).
    - Observation (not a defect): when the root CSS size is fractional, the outer border overflows or falls short by 0.2~0.4 CSS px (§2e precision ①) · the splitter's line is placed at `at × 100%` without rounding and is off by about 0.4px at most (it is not visible since it is the same color).
    - ★What remains — whether text blurs and whether the column count is stable are **not measured**★. A real agent terminal pane is needed, so it was handed to the user's GUI check.
- **R2 PTY resize during drag** — the preview changes the pane size every frame, and `TerminalSlot.tsx:93-119` does `fit()` on every RO and the PTY send is debounced 50ms. It is the same kind as now. orca's 「드래그 중 PTY 리사이즈 보류」 (`pane-pty-resize-hold.ts`) was not put in. *Implementation:* the feel of PTY resize during a drag is **not measured** (a real agent is needed — the user's GUI check).
- **R3 Very thin leaves** — blocked in two layers. ① If `split.setRatio` knows the canvas·metrics, it enforces the px minimum on the two sides of the dragged split (§2c). ② Even so, thin leaves produced by an ancestor drag are handled correctly regardless of size by `spatial.rs` through exact adjacency·`total_cmp` order (§2h). What remains: an LLM ratio that comes in **before** the canvas·metrics report (right after boot · right after popout) is cut only to 0.1~0.9 — the screen's 30px minimum can be broken, but neighbors·order are correct thanks to ②.
- **R4 Preview stuck on event loss** — reconciliation rule 6 (pull) blocks it. If the pull also fails, the preview remains until the next snapshot. A no-change response (`Unchanged`·`TooSmall`) does not wait for a snapshot, so it does not enter this path.
- **R4b Pane smaller than its frame** — there is no minimum window size (D4) and an ancestor drag shrinks deep panes (D2), so a pane can become smaller than the border thickness. The frame/border separation (§2e) and the shell `content`'s `max(0, …)` (§2d) mean there is no overlap or negative size. A pane of that size just shows its content cut off.
- **R11 Splitting small panes — decided (user 2026-09-24)**
  - **Decision:** a split that creates a pane smaller than `min_pane_px` is **allowed.** The only thing blocked is the extreme where one side gets a representable width of 0 — the existing f64 representability rule (`SplitTooDeep`, §2c). The user: 「0이되는 극단적인 상황만 막으면 될것같아 일단은」.
  - **Deferred (not rejected):** 「캔버스를 알 때 `min_pane_px` 미만이면 분할 거절」. The user also acknowledged the concern — 「너무 작아서 유저가 해당칸을 조작 못하는것도 문제긴 하겠네. 적당히 막던지.」 — and then chose to block only the extreme.
  - The nature of the basis for rejection: **user preference (for now)**.
  - **Conditions for reopening:** when being unable to operate a small pane is reported as a real problem, or when an LLM is observed repeatedly splitting panes.
  - **Peer evidence to look at when reopening (checked in code by the main agent):**
    - tmux rejects a split with 「no space for a new pane」 when there is no room (`tmux/layout.c:1410`, `cmd-split-window.c:153`).
    - zellij rejects an explicit split when the pane would fall short of 2× the minimum size (`MIN_TERMINAL_WIDTH`·`MIN_TERMINAL_HEIGHT` = 5 — `zellij-server/src/tab/mod.rs:154-155`) (`can_split_pane_*` at `zellij-server/src/panes/tiled_panes/mod.rs:697-747` — confirmed in the local clone `bf8d23a`). Automatic placement (`tiled_pane_grid.rs:1374-1377`, `find_room_for_new_pane`) has a condition of the same kind but is not a basis for explicit splits.
- **R4c The canvas is a latest value with no version** — it can be late by the 100ms debounce. The current consumer (the px-minimum clamp) tolerates that error (the difference from ADR-0068 is in §8).
- **R5 Popout** — it is a new slot id·new webview, so remount cannot be avoided. The new window's canvas is `None` until its first report.
- **R6 Hidden tabs** — the canvas is measured from the window content area, which does not hide (§2d). The hidden-tab slot-hide detection (`TerminalSlot.tsx:94-108`) and the WebGL seat (visibility observation `:224`) are unchanged.
  - *Implementation (pre-existing observation — not created by this change):* the initial `fit()`+`resizePty` right after subscription (now (step 9) `TerminalSlot.tsx:304-305`) has no hidden guard (`offsetParent`) — three paths that make the same send do have it: the RO callback (`:99-101`)·the flush callback (`:285`)·reconnect (`:355`). The WebGL attach path (`:211-213`) also makes the same send, but filters by IntersectionObserver visibility (`:224-227`) instead of this guard. It was not reproduced and not touched.
- **R7 DOM order ≠ screen order** (accepted under D6).
- **R8 Performance** — the snapshot grows by O(N) rectangles, and one snapshot arrives per confirm (all leaves re-render, remount 0). The preview changes only the leaves of the dragged subtree. Not measured.
  - *Implementation (follow-up candidate):* the memo of the body (`SlotBody`) compares `node` by reference, but the cache holds the snapshot's tree as is, so on every snapshot every pane's body is redrawn (not on drag frames — §4). It could be reduced by comparing `node.id` + `content` values. This is from reading the code and has not been measured.
- **R9 Two surface spellings** — bus `split.setRatio` ↔ Tauri `set_split_ratio`. It is the same convention as the existing `slot.split` ↔ `split_slot`, so it is not a new divergence.
- **R10 Exposing px to the LLM** — the shell knows px but no command exports it to the bus (no external consumer). If it becomes necessary, putting it in the `split.list`·`slot.resolveSpatial` answers is the follow-up.
- **What we do not know** — jsdom PointerEvent support · the `getComputedStyle` border width value at 125% scaling · whether Chromium makes xterm lose WebGL when it detaches and reattaches it (avoided via D6 but not verified).
  - *Implementation:* the border width was answered (0.8px@125% — R1). jsdom pointer capture is still unknown — tests call it optionally and push with coordinates as planned. The WebGL seat is not measured (a real agent is needed).

## 8. ADR proposal

- **Recorded: ADR-0227** 「분할 렌더러를 평평한 칸 목록으로 직접 구현하고 구분선 드래그를 셸 비율 명령으로 되쓴다」. This section was that ADR's draft, and now the source of truth is the ADR body. Below only the correspondence is kept.
- **Decisions:**
  1. A custom flat renderer (leaf key = slot id · one parent · slot id order · shell rectangle + CSS rounding · splitter overlay).
  2. `split.setRatio` (clamp 0.1~0.9 + clamp to the two-sided px minimum when the canvas·metrics are known, once at drag end, last write in arrival order wins, double-click 50:50) + `split.list`.
  3. The window canvas px goes in the window entry, the measured metrics (frame margin·minimum pane px) are set once at initialization, and the shell computes pane px.
  4. `spatial.rs` takes the shell geometry boundaries and decides by exact adjacency — neighbors·order are correct even on thin leaves.
  5. D2~D6.
- **Rejected alternatives:**
  - Renderer (the option table of the library survey):
    - **A** allotment + ratio `defaultSizes` — remounting of the sibling subtree remains on close·promotion. The direction is read only at mount time, so a key remount is needed. Upstream has low activity (F2).
    - **B** react-resizable-panels v4 nested — mirroring the backend tree as is leaves remounts. If blocked with pass-through nodes, the React tree diverges from the backend and a separate `setLayout` sync must be written (F3).
    - **C** react-mosaic — pulls in five `react-dnd`-family packages (16.0.1, no release since 2022-06) and `lodash-es` etc., and needs a tree conversion (F5).
    - dockview·golden-layout·Lumino·FlexLayout (own their model·imperative) · Split.js·react-split (stagnant) (F6).
  - Size·geometry: the discarded options in the D1·D1+ rows (§3).
- **Old decisions being revised** (the stamp wording is the same in three places — the old ADR's status line · the old ADR's related line · ADR-0227's related line):
  - **ADR-0223** 「결정 2 Allotment key와 영향의 승격 재마운트 조항」 → discarded. Decision 1 (split id) is kept and used as the splitter·preview key. The body of 「영향」 is not fixed (accumulation principle — `docs/decisions/README.md:13`).
  - **ADR-0068** 「결정 3과 거부 대안 백엔드 투영 px와 영향의 실측 픽셀 및 최소 칸 조항」:
    - Decision 3 (coordinate exposure deferred — `docs/decisions/0068-*.md:21`) → revised toward carrying normalized coordinates in the snapshot and having the shell compute px.
    - The rejected alternative 「백엔드 투영 물리 px」 (`:26`) is **revived.** The reason for rejecting it then (modeling gutter·sash·min-size·collapse → duplicating the render engine·drift) does not hold in this design. The renderer draws the shell rectangles as they are, the splitter is a zero-space overlay, and the border is measured, not assumed.
    - `:28` (normalized ≠ actual render — because of sash·min-size·border) is resolved structurally for the same reason.
  - The 「실측 픽셀 추가 시 프론트 실측 rect 는 **versioned 관측값** — 창크기/render epoch 가 바뀌면 stale」 in the 「영향」 of ADR-0068 (`:41`, the same model as decision 3 `:21`) → **we deliberately deviate.** What this design receives from measurement is not a per-pane rect but one pair of canvas sizes per window (+ metrics sent once), and it keeps only the latest value with no version.
    - Reason: the layout authority (the rectangles) is still the shell tree, and the measurement is one scale factor that multiplies, so no dual authority arises.
    - The width by which it can be late is the 100ms debounce, and the current consumer (the px-minimum clamp) tolerates an error of that size (R4c).
    - Attaching a version is revisited when px is exposed to the LLM (R10).
  - The 「리사이즈 명령은 UX 최소 칸 크기 강제」 in the 「영향」 of ADR-0068 :43 → **REPLACED — not satisfied.**
    - The minimum-size enforcement applies only to the two sides of the dragged split. Deep panes below 30px are accepted (D2), and splits that create panes below 30px are also allowed (R11).
    - Defects ①② are fixed by exact adjacency (removing `EPS`·`edge_eq` — §2h).
  - **ADR-0140** 「영향의 allotment 매핑 진실 경계」 → the truth boundary moves to the direction mapping in the shell's `geometry.rs`. A `// ADR-0140` anchor is placed there.
  - **ADR-0168** 「결정 7 allotment sash 토큰」 → `index.css:33-40` disappears together with allotment. The new splitter uses the theme tokens `var(--border)`/`var(--accent)` directly.
  - **Drag write-back scope limit** → resolved by D1. The source is **not** ADR-0063 but `step-log.md:776` (that slice's unresolved item ①)·`:2506`·the parenthesis of ADR-0223 decision 2. The ADR-0063 citation in the renderer comment `ViewLayoutRenderer.tsx:291` was wrong (the ADR-0063 stamp was reverted).
- **Source of the rejection reasons:**
  - The rejection reasons of D1~D6·R11 are the verbatim user statements (quoted in the §3 table — 2026-09-24).
  - For renderer A it is 「일단 쭈그림만 막고 머지하고」, for B it is 「하긴 개념적으로 하위개념이 아닌데 트리로 되는게 이상한거지」 + survey F3.
  - C is the user's answer 「ㅇㅇ 이것 상관없이 쭉 진행하고, 직접 구현하는걸로 하자」 to the main agent's question, which recommended D and described C's cost as the stagnant `react-dnd` family — the description of reasons is survey F5.
  - The strength marking of each reason is in ADR-0227 「거부한 대안」, which is the source of truth.
