# TRD — WezTerm-style window > tab > slot (layout B, Phase 1 = tabs)

> Status: **draft (written after all user decisions were locked)** · 2026-07-09
> Canonical PRD = `PRD.md` in the same folder. This document fills the "spec gaps to fix before the TRD" that its §10-3 required, and **fixes the implementation interface**, laying down the confirmed user decisions (D-1 C·D-2·D-3·D-6·D-7·D-8) and the ADRs (0056·0046·0006·0035) as the facts it faces.
> Discovery chain: `docs/README.md` → layout B → PRD.md → this document.
> Next steps: this TRD passes `/review trd` → record the tab ownership model ADR → `/implement critical` (staging §10).

---

## 0. One line · scope

Change the backend layout authority (`ViewManager`) so that a window owns a **tab list** (= several `View`s in code) and switches among them. Phase 1 = **tab model + tab bar UI + per-window independent routing + LLM control commands**. Drag between windows (D-4) and save/restore (D-5) are out of scope.

**Terminology (fixed in PRD §1):** the user's "tab" = **View** in code. The user's "window" = **OS webview window** (main + runtime popups). This document writes `View` (code) and "tab" (concept) side by side.

---

## 1. Confirmed decisions (no reopening — evidence and rejected alternatives in PRD §4/§10 + follow-up ADRs)

| # | Decision | Gist |
|---|---|---|
| **D-1 = C** | Owner-index hybrid | `views` (global pool) + `view_owner` (View→window, unique) + `windows[label].{tabs,active}`. A (ref-list = ownership) and B (global-pool sharing) rejected. |
| **D-2 = yes** | Main is also a regular window | Remove the global `active_view_id`. main/popup share the same code path. |
| **D-3** | Window-close rules | **main = always at least 1 tab** (an empty tab stays even when the last is closed) · **popup = closing the last tab also closes the window**. In both cases the **agent (daemon process) survives** (§5 hands-and-feet / brain separation). |
| **D-6 = yes** | Empty `create_window` | Add a command that creates an empty new window (1 empty tab) with no agent. |
| **D-7 = fold in** | Placement-designated spawn | As the **last slice** of Phase 1, a thin `spawn_into` wrapper (assembles spawn + create_tab + assign). Added on top after the tab model is complete. |
| **D-8 = Ctrl+Tab only** | Keyboard navigation | Phase 1 minimum = only Ctrl+Tab cycling of tabs within a window. Other keymaps later. |
| **D-4/D-5 = follow-up** | Drag · save/restore | Out of scope. Only make D-1 use a global UUID `ViewId` so it can be added on later. |

**Governing ADRs (mandatory — no reinterpretation):**
- **ADR-0056** Tab rendering = keep-alive: keep the inactive tab's xterm instance alive (output keeps accumulating), switching is instant and lossless (no replay needed). WebGL seats only for visible slots.
- **ADR-0046** View-direct replay: the routing table is label-agnostic, the wire fires only Unsubscribe (1→0), and Subscribe (0→1) is formed by the frontend `request_replay` alone. Slot progress (`lastDeliveredSeq`) is independent per view.
- **ADR-0006** Lock discipline: `ViewManager` mutation + `OutputRouter.rebuild` + delta enqueue = a single critical section (std Mutex), no external await/emit while holding the lock.
- **ADR-0035** Layout authority = src-tauri. (This TRD **flips** the `active_view_id` = main-only part → a follow-up ADR supersedes this section.)

---

## 2. New data model (`src-tauri/src/layout/manager.rs`)

### 2-1. Types (before → after)

```rust
// ── BEFORE ──────────────────────────────────────────────
struct ViewManager {
    views: Vec<View>,                     // 전역 View 벡터 (선형 탐색)
    active_view_id: Uuid,                 // 전역 활성 뷰 1개 (메인 전용)
    window_bindings: HashMap<String, Uuid>, // label → view_id (팝업/보조창 바인딩)
    version: u64,
}

// ── AFTER (D-1 C) ───────────────────────────────────────
type ViewId = Uuid;
type WindowLabel = String;

struct ViewManager {
    views: HashMap<ViewId, View>,            // 전역 View 풀 (id lookup)
    view_owner: HashMap<ViewId, WindowLabel>,// View → 소유 창 (유니크 소유 강제)
    windows: HashMap<WindowLabel, WindowTabs>,// 창 → 탭 목록 + 활성
    version: u64,                            // 기존 유지 (stale-emit 방어, PopoutPage race)
}

struct WindowTabs {
    tabs: Vec<ViewId>,   // 탭 순서 (좌→우)
    active: ViewId,      // 이 창의 활성 탭
}
```

- The `View` itself (id/name/layout tree/focused_slot), the `tree.rs` pure ops and `types.rs` are **unchanged** (slot-split logic is reused). Only the **ownership/active layer** of "which View is which-numbered tab of which window" changes.
- The `agent-tree` window is **outside** this model (§3-2). There is no `"agent-tree"` key in `windows`.

- **Keep `view_owner` as a cached reverse index (not duplicated state, G11).** The owning window could be derived by scanning `windows[L].tabs` (O(n) is trivial at ≤24 scale), but view-id-keyed commands (`assign_agent`/`split_slot`/`close_slot`) must find the owning window in **O(1)** (they have to fire events at that window), and above all it pins **unique ownership as a type-level invariant** (`HashMap<ViewId, WindowLabel>` = exactly 1 window per View). A scan-derived approach can keep this invariant only with a runtime assert, so it is not adopted. Consistency is kept by invariant 1 (two-way consistency) — updates are always done as the `windows[L].tabs` ↔ `view_owner[v]` pair.

### 2-2. Invariants (★load-bearing — pinned in code with `// ADR-` anchors★)

1. **Two-way consistency:** `view_owner[v] == L` ⟺ `windows[L].tabs.contains(v)`. (No updating only one side — always update as a pair.)
2. **Unique ownership:** every `v ∈ views` has exactly 1 entry in `view_owner`. **One View cannot belong to two windows.** (§10-3 item 5 = removing the current `output_router.rs:156` allowance of "one View in two windows".)
3. **Active membership:** `windows[L].active ∈ windows[L].tabs` always.
4. **Main has at least 1 tab + main is non-closable:** `windows["main"].tabs.len() >= 1` is invariant (D-3). **The main window itself does not close** — the `lib.rs:142` CloseRequested arm handles the `"main"` label only with `prevent_close`+`hide` (tray-resident, ADR-0026/0029) and leaves no Destroyed. Therefore `close_window("main")` is **forbidden** (when the last-tab branch of §5-2 `close_tab` is main, it falls through only to forced creation of an empty tab instead of `close_window`) — the command layer rejects `close_window` targeting `main`. (G11)
5. **Multiple agent references allowed:** the same `agent_id` string can be assigned to slots of two different Views (→ two windows see the same agent, independent progress, ADR-0046). This is a **different** matter from "one View in two windows" (forbidden by invariant 2).

### 2-3. Boot initialization

```
views       = { v0: View::new() }
view_owner  = { v0: "main" }
windows     = { "main": WindowTabs { tabs: [v0], active: v0 } }
```
`agent-tree` does not go into `windows` (a config window, renders /tree).

---

## 3. Migration (§10-3 item 1)

### 3-1. There is no data migration
Save/restore (D-5) does not exist yet, so only **runtime state** exists → no on-disk schema migration is needed. "Migration" = a pure **code refactor** (in-memory model + boot init + consumers).

### 3-2. The `agent-tree` window (special window — not a target)
- Measured: `agent-tree` is a config window (`tauri.conf.json` `url: index.html#/tree`, `visible:false`) that draws **`TreePage`** (the agent tree). It does **not actually go into** the runtime `window_bindings` (the `"agent-tree"` insert in the `output_router.rs` test is only an example of the general mechanism; there is no real insert in the boot code).
- **Conclusion: stays outside the tab model.** No tab bar. The `main`/`agent-tree` branch in the `lib.rs` Destroyed arm and the `is_popup_label` prefix decision also stay as they are.
- **`agent-tree` does not mount the slot canvas (`WindowLayout`, §7)** — it draws `TreePage` (the agent tree) as is. So it is not a target of the §7 unified path (`WindowLayout(label)`), and since it is outside `windows` there is no tab routing either. (Check at implementation: re-confirm that `TreePage` does not use `ViewLayoutRenderer` — if it does, it is promoted to a D-2 unification target.)

### 3-3. Popup windows + the `?view=` URL (§10-3 item 1 core)
- **Currently:** a popup (`slot-popup-N`) draws a **fixed single View** via the `#/popup?view=<id>` URL (`PopoutPage`, no tab switching). `pop_out_slot` put the window→View entry in `window_bindings`.
- **After:** a popup is also a **regular window that has tabs** (D-2, PRD §5 "popups also have tabs"). Popup window = `windows["slot-popup-N"] { tabs:[v], active:v }`.
  - **URL key switch: `?view=<id>` → `?window=<label>`.** The popup page now draws "this window's active tab" rather than a "fixed view" and shows its own tab bar. (The window label is the SSoT, the active tab comes from the backend `windows[label].active`.)
  - `pop_out_slot` → `move_slot_to_window` (§8): when detaching a slot, the new popup window starts with 1 tab holding that slot's agent.
  - **The path by which a popup learns "its own active tab" (G3 — required, the `?window=` key switch alone is not enough).** The popup output Channel subscribes by `window.label()`, not the URL (`agent.rs:143`) → even if the URL key becomes `?window=`, the frontend has no way to know the active tab on its own. So on mount `WindowLayout(label)` (§7) fixes the active tab with **① an initial pull** = `list_tabs(label)` (returns `{tabs, active, version}`, §6), and **② listening to `window:tabs-updated{label,tabs,active,version}`** swaps and re-renders the active tab on `switch_tab`/`create_tab`/`close_tab` (only when it matches its own `label`). Without it the popup draws only a fixed tab and D-2 "popups also have tabs" regresses.
  - **A stale `?view=` popup stranded by a dev reload = accept + document (decision, G3).** Popups are **runtime-only** (D-5 save/restore is out of scope), so a popup that comes back up with the old `?view=` URL after a dev HMR/manual reload and shows an error/blank screen because the window was already cleaned up is **accepted**. A `?window=`∥`?view=` dual fallback is **not added** (do not lay persistent restore logic on runtime-only state — scope pollution). If a reloaded popup cannot find a valid label, it only shows a "이 팝업 창을 닫으세요" (close this popup window) notice.

### 3-4. Cleaning up the `active_view_id` consumers (frontend — complete inventory, G7)
- The global `active_view_id` in `viewStore.ts` → replaced by **per-window active**. `useCurrentViewId()` resolves it as "which window this webview is → that window's active tab" (main = `windows["main"].active`, popup = the active of the `?window=` label).
- **Swap every existing consumer that reads the global `activeViewId` to `useCurrentViewId()` (per window) — if one is missed, popups/auxiliary windows touch the wrong main view:**
  - `viewStore.ts:273` `resolveDefaultViewId()` — currently `readViewIdFromHash() ?? activeViewId`. Remove the `?view=` hash parsing and change it to fall back to "this webview window's active tab" (§7 `useCurrentViewId`).
  - `SlotContextMenu.tsx:31/34` — `activeViewId` + `viewIdOverride ?? activeViewId`. `WindowLayout` (§7) passes its own window's active tab down as `viewIdOverride`, so this component stays window-unaware (it receives the per-window active only as a prop).
  - `AgentTree.tsx:209` — `vs.activeViewId` (the focus-slot placement coordinate). agent-tree is outside the tab model, so the rule below applies.
- **Handling `useCurrentViewId()` in the `agent-tree` window (G7 — outside-the-model fallback):** agent-tree is not in `windows`, so it has **no** per-window active tab (undefined). agent-tree's "place in the focus slot" (`AgentTree.tsx:209`) places onto the **main window's active tab** because its own window has no slot canvas — `useCurrentViewId()` falls back to `windows["main"].active` in the agent-tree context (a special case: agent-tree is a config window that manipulates main). If main active cannot be obtained, the same error as before: "no active view/focus slot".
- **Remove** the "bound-view filter + Fix 2B active guard" logic of `list_views`/`view_metas` — with unique ownership `windows[L].tabs` is the tab list itself (no filter needed). Also replace `close_view`'s "hand active over to an unbound view" logic with the §5-2 rule.
- **`applyViewListUpdated` field rename (re-review nit):** the old `ViewListPayload {views, active_view_id}` (`layout.rs:56`) → the new `list_tabs`/`window:tabs-updated` payload is `{tabs, active, version}`. Update the frontend `applyViewListUpdated` (`viewStore.ts:187`) to rename `active_view_id`→`active` and accept `version` (if missed, the active tab does not attach). The `get_view(viewId)` (id lookup) contract is unchanged.

---

## 4. Routing / replay (§10-3 item 2 — consistent with ADR-0046/0056)

### 4-1. New `OutputRouter.rebuild` logic (`output_router.rs`)
```
by_agent: HashMap<AgentKey, Vec<WindowLabel>> = {}
for (label, wt) in &windows {
    for vid in &wt.tabs {                 // ★활성뿐 아니라 모든 탭★ (ADR-0056 keep-alive)
        collect_agents(views[vid].layout, &[label], &mut by_agent);
    }
}
// dedup/sort → ArcSwap store → prev와 diff (§4-3)
```
- **Core change (G5 — routing reversal):** **completely remove** the `if view.id == mgr.active_view_id { windows.push(MAIN_WINDOW_LABEL) }` branch of the current logic (`output_router.rs:159`) plus the `window_bindings` scan, and replace it with "walk all tabs of each window". The `active_view_id` and `MAIN_WINDOW_LABEL` constants are **wholly excluded** from the `by_agent` computation (the active flag no longer enters routing). With unique ownership a View belongs to only one window → no double counting.
- **Hidden tabs are routed too** — the backend counterpart of ADR-0056 "output keeps accumulating". Zero frame loss on switching → no replay needed (hot path).

### 4-2. Routing ≠ seat — the two axes are orthogonal (G6 correction)
**Routing (backend) and the WebGL seat (frontend) are different resources, so do not conflate them:**
- **Routing** = flowing frames into the xterm *buffer*. Hidden tabs also keep **receiving frames and piling them into the buffer** (xterm instance kept, §4-1 "walk all tabs"). There is no upper limit on the number of routing targets (the buffer uses only memory).
- **WebGL seat** = the GPU context held by the frontend `WebglAddon` (scarce, measured 16). A hidden tab **disposes** its `WebglAddon` (returns the seat, ADR-0056) → seat consumption = **visible slots only**.
- So even if routing covers "all tabs", seats do not grow. The real invariant (**simultaneously visible slots ≤ 16**) lives in the **frontend's domain = §7**, not in routing.

**Visible-slot limit guard (ADR-0056, for D-6 unlimited windows):**
- `create_window` (D-6) can open windows without limit, so visible slots can exceed 16. But there is no data loss — the ADR-0056 `onContextLoss→DOM` fallback (`TerminalSlot.tsx`) **gracefully degrades** a slot that could not get a seat to DOM rendering (only rendering is degraded, routing/buffer stay the same) → 0 frame loss.
- **ADR-0056 limit guard:** design limit = 3 windows × 2 tabs × 4 slots (12 visible ≤ 16). A layout exceeding this limit (raising the window/tab/slot maximum) **requires re-examining ADR-0056** — a count note in the command layer: when `create_window` increases the window count, log when nearing this limit (not a hard block), and a layout over it triggers re-examination. (Details in §7.)

### 4-3. Subscribe/Unsubscribe delta (ADR-0046 invariant)
- Condition for keeping an agent subscription = **that agent's slot exists in any tab of any window.** rebuild diffs against `prev`:
  - `to_unsubscribe` (1→0, last exposure gone) → fire Unsubscribe to the daemon over the wire (leak cleanup).
  - `to_subscribe` (0→1) → **do not fire over the wire.** The frontend `request_replay` forms it alone (ADR-0046 BLOCK-1).
- `close_tab`/`switch_tab` change the exposure set, so rebuild is required. `switch_tab` is keep-alive so the **exposure set is unchanged** (all tabs are already subscribed) → usually a no-op delta, but rebuild is called by contract (only the active marker changes).

### 4-4. Same agent in two windows = independent progress (ADR-0046)
By invariant 5 the same `agent_id` can be assigned to two Views. Replay is view-direct (per-slot `lastDeliveredSeq`), so the seq cursors of the two Views are independent → scroll/progress of the two windows is independent. The backend routes to both window labels.

---

## 5. Concurrency contract + state machine (§10-3 items 3·4, ADR-0006)

### 5-1. Critical section (ADR-0006 kept)
Every tab/window mutation command follows a **single pattern**:
```
lock(ViewManager)
  → 모델 변경(views/view_owner/windows) + 불변식 유지
  → OutputRouter.rebuild(&mgr) (delta 산출)
  → send_subscription_delta(delta.to_unsubscribe 만) try_send (동기 non-blocking, 락 안)  // ★ADR-0046 BLOCK-1★
unlock
  → emit(창별 이벤트) · (async 창 빌드는 언락 후)
```
- Concurrent calls of several commands are serialized by the ViewManager Mutex → no rebuild interleaving (current contract kept). No webview build, network or emit while holding the lock.
- **Wire firing is only `delta.to_unsubscribe` (1→0 cleanup) (G11 — ADR-0046 BLOCK-1).** `delta.to_subscribe` (0→1) is only computed and **not sent over the wire** — subscription formation (replay) is by the view-driven `request_replay` alone. If a coder re-adds an eager Subscribe for convenience, this invariant breaks (a BLOCK-1 regression).

### 5-2. `close_tab` state machine (D-3 confirmed)
```
close_tab(L, v):
  assert view_owner[v] == L
  remove v from windows[L].tabs, views, view_owner    // ← view 1개 드롭(불변식 1 쌍 갱신)
  if closed v == windows[L].active:
      windows[L].active = 인접 탭(오른쪽 우선, 없으면 왼쪽)   // 탭 남아있을 때
  if windows[L].tabs.is_empty():
      if L == "main":  create_tab("main")  // 빈 탭 1개 강제 (불변식 4, main non-closable)
      else:            close_window(L)      // 팝업: 창도 닫힘, 에이전트 생존 (D-3 자가닫힘)
  rebuild + emit(window:tabs-updated{L,...})
```
- Do not kill the agent (§5 separation).

#### Self-close signal = "this window has 0 tabs = window closes" (G2 — double-fire reconciliation)
- **Signal redefinition (D-3):** the trigger of popup self-close is **"this window's `tabs` becomes 0 = the window closes"** — it is **not** the disappearance of a single view (`view:closed`). `view:closed{id}` (removal of one View) is only a tab-bar update signal, not a window close (conflating the two kills the window even when a non-last tab is closed).
- **Double-fire reconciliation (regression prevention — `layout.rs:49` "re-entrancy risk"):** `close_tab(popup, last)` closes the window only through the single path `close_window(L)` in the backend → OS Destroyed → the `lib.rs` Destroyed arm does the residual cleanup with `cleanup_popup_window` (§5-2/G1). **Do not fire a separate `view:closed`→`PopoutPage.close()` to the frontend** (the old path tried to kill the window twice through `close_window`+`view:closed` double firing and collapsed in a re-entrant chain). That is, window close = the backend `close_window` single source, and the frontend only handles closing its own window idempotently (no-op if it is already closing) when it receives `window:tabs-updated{tabs:[]}` (0 tabs).
- **`view:closed` fully retired (G2 re-review remainder — merely renaming does not actually make the double-fire disappear):** for the above reconciliation to hold, the old path must be removed **end to end**: ① **remove the `view:closed` emit** from `close_view`/`close_tab` (no longer fired), ② **remove the `view:closed`→`getCurrentWindow().close()` listener** of `PopoutPage` (`PopoutPage.tsx:66-72`) — self-close is replaced by the single signal `window:tabs-updated{tabs:[]}` (§7-1). The tab removal itself is already visible through the reduced `tabs[]` of `window:tabs-updated`, so `view:closed` is **redundant**. ③ the `view:closed{own}`→`close()` assertion in `PopoutPage.test.tsx` is the old contract, so **delete/invert** it (§8 stage 4 — in parallel with the G5 test inversion). If left, when `close_tab(비-마지막)` fires `view:closed` a stale `PopoutPage.close()` pops up and re-enters.
- **Cleanup path = the `cleanup_popup_window` multi-tab cleanup below (G1):** the residual cleanup for a popup window closing (self-close · titlebar · forced Destroyed, all of them) is dropping **all of `windows[L].tabs`** by iteration + removing `view_owner` + 1 rebuild + the Unsubscribe delta.

#### `cleanup_popup_window` multi-tab cleanup (G1 — ★concrete leak fix, top priority★)
- **Current bug:** `cleanup_popup_window` (`popout.rs:269`) cleans up **only the one** `window_bindings.get(label)` → if a multi-tab popup is closed by titlebar/forced Destroyed, the remaining tabs' Views **stay** in `views`/`view_owner` + the daemon subscription is **not Unsubscribed** (leak).
- **After (new model):** the Destroyed cleanup **iterates over all of** `windows[label].tabs`, drops each View from `views` + `view_owner`, removes the label entry from `windows` → **one** `rebuild` (at the end) → fires to the daemon the `to_unsubscribe` of that rebuild delta (agents remaining in no window). Do not call rebuild per tab; drop everything and then once (one lock section). **Mandatory** in the headless test (§8 stage 1) — not deferred to §8 stage 6 (GUI).

### 5-3. `move_slot_to_window` / pop-out mid-flight rollback (§10-3 item 4)
Return type = **`{ window: WindowLabel, tab: ViewId }`** (G4 — not a bare `Result`. The caller knows the window and tab it moved to). Keep and extend the current `pop_out_slot` 2-phase, but **defer the tab insertion for an existing-window target to phase C**:
```
phase A (lock): 소스 슬롯 에이전트 read → 대상 View 생성(임시, 아직 어느 창 tabs 에도 안 넣음) → assign → rebuild(pre-build)
phase B (unlock): WebviewWindowBuilder (async — 데드락 회피 위해 반드시 언락)
                  · 새 창 타깃: 여기서 창을 빌드 · 기존 창 타깃: 빌드 없음(창 이미 존재), 존재만 확인
phase C (lock): ┌ 새 창: 새 label 로 windows 엔트리 생성 + tab 삽입(create_tab 상당)
                ├ 기존 창: ★view_owner[?]/windows[to_window] 재검증 후★ 그 창 tabs 에 삽입 — 여기서 처음 삽입(phase A 아님)
                └ 공통: 소스 슬롯 close → rebuild + emit(양 창 window:tabs-updated)
rollback: phase B 실패 or phase C 재검증 실패 시 phase A 임시 View 원복(close_view 상당) + 소스 유지
```
- **★Preventing orphans for an existing-window target (G4 core)★:** defer the existing-window tabs insertion to **phase C, not phase A**. If it were pre-inserted in phase A, an orphan tab would remain when the target window disappears / is concurrently closed by close_window during phase B (unlocked async) (the current popup code handles only new windows, so this class did not exist). In phase C, **re-verify that `to_window` still exists** (`windows.contains_key(to_window)`) and insert only then — if absent, do not insert and roll back. (Only the source slot detach stays 2-phase as before = reserved in phase A, closed in phase C.)
- **Re-verification anchor:** right before inserting into an existing window, require that `windows[to_window]` exists, and after insertion that `view_owner[tab] == to_window` (pair update of invariants 1·2). The new window label comes from `PopupCounter` (monotonic, no reuse), so there is no reuse collision (§6/G8: `create_window` uses the same counter/prefix).
- **The same agent in two tabs = allowed (invariant 5, G4 — coder must not over-guard):** moving into a window where another tab is already viewing the same `agent_id` is also fine (two tabs on the same agent, independent progress, ADR-0046). Do not put a **spurious dedup** into the move path (reject/skip if that window already has that agent) — it is a **different** matter from "one View in two windows" (forbidden by invariant 2).

---

## 6. Command surface (§5 LLM-first control — invariant)

Every tab/window operation is exposed as a command (the LLM = the main operating actor, a human click = the same command). Existing view-language → **renamed to tab-language** (consistent with the user/LLM-facing surface, PRD scenario terms).

| command | Signature | Behavior | Existing counterpart |
|---|---|---|---|
| `create_window` | `() -> WindowLabel` | An empty new window (1 empty tab) + webview build. label = **reuse the `PopupCounter`/`slot-popup-*` prefix** (G8). (D-6) | new |
| `create_tab` | `(window, name?) -> ViewId` | Add a new empty-slot tab to that window and activate it. | `create_view` (global) |
| `switch_tab` | `(window, view) -> Result` | Replace only that window's active (other windows unchanged). | `switch_view` (global active) |
| `close_tab` | `(window, view) -> Result` | §5-2 state machine. | `close_view` |
| `close_window` | `(window) -> Result` | Close the whole window. **`"main"` forbidden** (invariant 4/§2-2 — main is hide only, G11). | new |
| `move_slot_to_window` | `(from_view, slot, to_window?) -> {window, tab}` | Move a slot's agent to a new tab of another window (a new window if none). §5-3 (phase-C insertion · re-verification, G4). | `pop_out_slot` (extended) |
| `split_slot`/`close_slot`/`assign_agent` | `(view, …)` | Operations inside a View (view_id is globally unique so the signature stays; the owning window is derived in O(1) from `view_owner`). | same |
| `list_windows` / `list_tabs` | `() ` / `(window) -> {tabs: ViewMeta[], active, version}` | Read-only for boot/tab-bar initialization. `list_tabs` **includes `version`** (G10, stale defense). | `list_views` (restructured) |
| `spawn_into` | `(window, tab?, slot?, backend, cwd, …) -> AgentId` | **D-7**: spawn (daemon) → create_tab if no tab → slot assignment (policy ↓). On failure the agent survives and is reported (no hard rollback). | new (composite) |

- **`create_window` label reuse (G8):** `create_window` (D-6) does not make a separate label scheme and **uses the `PopupCounter` + `slot-popup-*` prefix as is**. Reason: the `lib.rs:157` Destroyed cleanup gate is hooked on `is_popup_label` (`popout.rs:309`, prefix matching) — with a different label, `cleanup_popup_window` does not run when the window closes and routing/subscription/Channel leak. (The label's meaning widens from "popup" to "runtime window", so only the constant's comment is updated — the prefix value is unchanged.)
- **`spawn_into` slot policy (G9 — preventing coder guesswork):**
  - **`slot` unspecified:** assign to the target tab's **empty root slot** (if the tab was newly made with create_tab, it already has 1 empty slot → there). If the tab is also unspecified, make a new tab with create_tab and put it in its empty root slot.
  - **`slot` specified · empty:** assign to that slot.
  - **`slot` specified · occupied (already has an agent):** **error** (no overwriting) — the caller first makes an empty slot with `split_slot` and retries (no automatic split/replace = avoids destructive guessing).
  - **Failure visibility (§5 separation):** if the spawn succeeded but the assignment step fails (occupied slot, etc.), **the agent is alive in the daemon** and can be re-attached through `list_agents` (hard rollback = no kill). Report to the caller "배정 실패(에이전트 <id> 는 살아있음)" (assignment failed, agent <id> is alive) → prevents an invisible agent.

- **Events:** the per-window tab update = `window:tabs-updated { label, tabs: ViewMeta[], active, version }` (replaces the current global `view:list-updated`, window-scoped). **Attach `version` (G10):** carry the global `version` (§2 kept) that rises on every `windows`/`views` change so the frontend discards stale emits (consistent with the PopoutPage race mirror discipline). `layout:updated` (view snapshot) is kept. **The `view:closed` event is retired end to end (G2)** — no longer emitted and the `PopoutPage` listener is removed too (§5-2). Tab removal shows through the reduced `tabs[]` of `window:tabs-updated`, and popup window self-close is handled by `window:tabs-updated{tabs:[]}` (0 tabs) + backend `close_window`→Destroyed→`cleanup_popup_window` (§5-2/G2).
- **The renaming is an internal→external surface change, so it is a reporting item** (same behavior, only the names become tab-language). Update the old-name call sites (frontend · docs) in bulk.

---

## 7. Frontend interface (`src/`)

### 7-1. A single `WindowLayout(label)` path — main·popup unified (G7 — D-2 "same code path")
- **New `WindowLayout(label)` component:** a single component that draws one window's tab bar + active-tab slot canvas. **Both the main window and popup windows mount this** (each with its own `label`) → removes the old split of "`AppLayout` renders the global active vs `PopoutPage` renders a fixed view" (a D-2 violation).
  - main: `WindowLayout("main")` — window chrome such as Sidebar/DiffPanel/StatusBar is wrapped by `AppLayout`, and only the slot area is replaced with `WindowLayout("main")`.
  - popup: `WindowLayout(readWindowFromHash())` (§3-3 `?window=<label>`) — `PopoutPage` shrinks to a shell that thinly wraps this component.
  - **`agent-tree` is outside this path (§3-2 unchanged):** it draws `TreePage` (the agent tree) as is — it is not a slot canvas so it does not use `WindowLayout`.
- **`WindowLayout` behavior:** on mount, an initial pull of `list_tabs(label)` (§6, `{tabs,active,version}`) + listen to `window:tabs-updated{label,...}` (only its own label). **Mount all of** `windows[label].tabs`, **show only the active one** (ADR-0056 keep-alive: hidden tabs `display:none`, xterm kept · WebglAddon disposed). On receiving 0 tabs, self-close its own window (§5-2/G2, idempotent).

### 7-2. `useCurrentViewId()` + tab bar + seat limit
- **`TabBar` (new):** top of each window (inside WindowLayout). Renders `windows[label].{tabs,active}` + `[+]` (create_tab) + a per-tab close (close_tab). shadcn tab style (purely internal). Position/style = decided by the main session (reported).
- **`viewStore`:** global active → per-window state. `useCurrentViewId()` = the active tab of this webview's window (main = `windows["main"].active`, popup = the active of the `?window=` label, **agent-tree = fallback to `windows["main"].active`** — the §3-4/G7 special case). `SlotContextMenu` uses only the `viewIdOverride` (= that window's active tab) that `WindowLayout` passes down, and the reference to the global `activeViewId` is removed (§3-4).
- **★The visible-slot seat limit is canonical here (§4-2/G6)★:** simultaneously visible slots ≤ 16 (WebGL seats measured). The excess is gracefully degraded to DOM rendering by the `onContextLoss→DOM` fallback of `TerminalSlot` (ADR-0056, **already exists** — the place to attach a `// ADR-0056` anchor) → 0 data loss. A layout exceeding the design limit (3 windows × 2 tabs × 4 slots = 12 visible) triggers an ADR-0056 re-examination (§4-2).
- **Ctrl+Tab (D-8):** cycling tabs of the focused window = `switch_tab(currentWindow, next)`. Frontend key handler → command (the same path as a human click).

---

## 8. Implementation staging (module-boundary DDD · ADR-0012 isolation — `/implement critical`)

Each stage = tests first (or together) + cumulative regression. Core/layout is verified headless, outside the Tauri seam.

1. **Model (`manager.rs`)** — new struct + invariants 1~5 + `WindowTabs`. Unit tests (headless): create/switch/close_tab state machine (§5-2), preventing unique-ownership violation, main minimum 1 tab, **rejecting `close_window("main")`** (G11). **★G1 mandatory headless test★: forced cleanup of a multi-tab popup** — put 2+ Views in `windows[label].tabs`, run the path equivalent to `cleanup_popup_window`, and assert that all Views are removed from `views`+`view_owner` and that the rebuild delta produces an Unsubscribe for every remaining agent (0 left over). `tree.rs` unchanged.
2. **Routing (`output_router.rs`)** — the new rebuild logic (§4-1) + delta diff (§4-3). **★G5 — invert the existing test contract★:** remove the `active_view_id` routing branch (`output_router.rs:159`) and **delete/invert** `diff_switch_view_changes_visible_set` (`output_router.rs:557`) — under keep-alive `switch_tab` does not change the exposure set so the delta is a **no-op**, and the old "switch → A leaves and B enters" assertion flatly contradicts it (leaving the old test or the old active branch breaks keep-alive). Replace it with a test confirming that the `active` flag is wholly excluded from `by_agent`. Update the existing `agent-tree` example test → to the new model (all-tab routing · same agent in two windows · close→unsubscribe).
3. **Commands + concurrency (`commands/layout.rs`·`popout.rs`)** — the §6 surface + the §5 critical section + `move_slot_to_window` rollback (§5-3) + per-window events.
4. **Frontend (`src/`)** — the single `WindowLayout(label)` (§7-1) + `TabBar` + per-window `viewStore` + tabifying `AppLayout`/`PopoutPage` · keep-alive + Ctrl+Tab. **★G2 — retire `view:closed`★:** remove the emit and the `PopoutPage` listener + delete/invert the `view:closed`→close assertion in `PopoutPage.test.tsx` (in parallel with the stage 2 G5 test inversion). **Field rename (re-review nit):** `applyViewListUpdated` (`viewStore.ts:187`) `active_view_id`→`active` + accept `version`.
5. **`spawn_into` (D-7)** — a thin composite command (on top of stages 1~4). Failure-tolerant (agent survives, reported).
6. **GUI measurement (§9)** — cdp + EnumWindows E2E.

---

## 9. Acceptance criteria (PRD §6 + D-7)

1. A **tab bar** in each of the main and popup windows, showing that window's tab list and active tab.
2. Tab click/`switch_tab` → switches **only that window** (other windows unchanged).
3. `create_tab`/`[+]` → adds a new empty-slot tab to that window and activates it.
4. `close_tab` → state machine (§5-2): main last = an empty tab stays, popup last = the window closes, **the agent survives**.
5. Routing: each window receives the output of the agents of all its own tabs, lossless on switching (ADR-0056). Independent progress when two windows have the same agent (ADR-0046).
6. GUI measurement (cdp+EnumWindows): E2E of creating and switching 2 main tabs + switching a popup tab.
7. Ctrl+Tab cycling within a window (D-8).
8. **(D-7)** `spawn_into(창,탭?,슬롯?)` spawns and places in one shot → the agent appears in the slot of the designated window's tab.
9. Every operation comes with a command path (§5). Major decisions as ADRs.

---

## 10. ADRs · open items

- **Required ADR (record before implementation):** tab ownership model = **D-1 C (unique ownership + per-window tabs)** — rejected A (ref-list = ownership) and B (global pool), basis (validated by WezTerm/VS Code · absence of a sharing requirement). This ADR **supersedes the `active_view_id` = main-only section of ADR-0035** (removes the global active). The routing "receive all tabs" derives from ADR-0056, so describe it in this ADR together or reference it.
- **Purely internal (decided and reported by the main session, not a decision):** command renaming (view→tab language) · `TabBar` position/style/[+] · the `window:tabs-updated` event name · the direction of adjacent-tab succession (right first) · keeping the `view_owner` cached reverse index (§2/G11).
- **Check at implementation:** re-confirm in §3-2 that `TreePage` does not use the slot canvas · regression of the existing popup open path under the `?view=`→`?window=` switch · the dev-reload stale popup notice screen (§3-3/G3).
- **Follow-up docs (unrelated to implementation):** the PRD §6.5 wording "receive only the active tab" is **superseded** by ADR-0056 (all-tab routing) → update the PRD wording as a follow-up with `/review doc` (G11). This TRD §4-1 is canonical.
- **Out of scope (follow-up):** D-4 (drag between windows) · D-5 (save/restore, merges into roadmap D-7, popup persistence included here) · structured inbox (Phase 3).

---

## 11. Adversarial review reflected (/review trd deep — 2026-07-09)

Verdict: **all 3 reviewers FIX** (Designer blind=Codex · Architect-breaker doc-aware=Opus · migration/concurrency specialist doc-aware=Opus). No BLOCK — the **direction (D-1 C · keep-alive routing · 2-phase rollback) is validated**. No head-on conflict (the three viewpoints complement each other). With many FIX items, the **draft needs a major revision → after reflecting, re-review, then `/implement`**. Updated items below (duplicates removed, evidence points included).

### G1. Popup multi-tab cleanup leak (★concrete leak — top priority) [BLOCK-grade basis]
Basis: the current `cleanup_popup_window` cleans up only the single `window_bindings.get(label)` (`popout.rs:269`). On a multi-tab popup **forced close** (titlebar/Destroyed, not close_tab), the remaining tabs' Views **stay** in `views`/`view_owner` + the daemon subscription is **not Unsubscribed**.
→ Reflected in body §5-2 (cleanup multi-tab iteration) + §8 stage 1 (headless test mandatory).

### G2. Popup self-close double execution · re-entrancy
Basis: `close_tab(popup, last)` → §5-2 `close_window(L)` (→Destroyed→cleanup) **AND** `view:closed`→`PopoutPage.close()` double fire (regression of `layout.rs:49` "re-entrancy risk").
→ Reflected in body §5-2 ("self-close signal = 0 tabs = window closes" reconciliation, idempotent, **`view:closed` retired end to end**) + §6 + §8 stage 4 (listener · test delete/invert). **[Re-review remaining FIX applied]** — in the re-review it was judged PARTIAL because "the retirement is only a rename and the removal of the actual emit/listener/test is unspecified" → made explicit the removal of the `view:closed` emit · the `PopoutPage` listener · the `PopoutPage.test.tsx` assertion (+ the `list_tabs` field rename nit = §3-4).

### G3. No path for a popup to learn "its own active tab" (D-2 regression)
Basis: the popup output Channel subscribes by `window.label()` (independent of the URL, `agent.rs:143`) → **switching only the URL key from `?view=` to `?window=` is not enough** (the popup cannot know the active tab by itself).
→ Reflected in body §3-3 (initial pull `list_tabs` + `window:tabs-updated` listen) + §7-1 (`WindowLayout` behavior). **Decision:** a dev-reload stale popup = **accept + document** (runtime-only · D-5 out of scope — §3-3), dual fallback not adopted.

### G4. move_slot_to_window rollback — existing-window target orphan
Basis: if §5-3 phase A pre-inserts `create_tab` into the **existing window's** tabs, then when the target window disappears / is concurrently closed with close_tab during phase B (unlocked async build) there is an orphan tab (the current popup code handles only new windows so this class did not exist).
→ Reflected in body §5-3 (existing-window insertion deferred to phase C + `view_owner`/`windows[to_window]` re-verification + `{window,tab}` return + same agent in two tabs invariant 5 allowed / dedup forbidden) + §6 (return type).

### G5. Routing reversal = flips the existing test contract
Basis: the current rebuild routes only `active_view_id` to main (`output_router.rs:159`) → the exposure set changes on switching (the test `diff_switch_view_changes_visible_set`, `:557`, asserts it). Once it becomes keep-alive (all-tab routing), **switch = no-op delta**.
→ Reflected in body §4-1 (remove the active branch) + §8 stage 2 (test delete/invert made explicit, confirm the active flag is excluded).

### G6. WebGL seat limit guard (D-6 unlimited windows)
Basis: §4-2 conflated routing↔seat (routing = xterm buffer, seat = frontend WebglAddon dispose). The real invariant (**visible slots ≤16**) is §7's domain. `create_window` (D-6) = unlimited windows → visible slots can exceed 16. Mitigation already exists: the ADR-0056 `onContextLoss→DOM` fallback degrades gracefully.
→ Reflected in body §4-2 (routing ≠ seat orthogonality correction + limit is §7's domain + graceful degrade + create_window count note) + §7-2 (seat limit canonical · ADR-0056 re-examination trigger).

### G7. Frontend migration inventory + main/popup unification
Basis: `active_view_id` consumers missed — `AgentTree.tsx:209` (`vs.activeViewId`) · `SlotContextMenu.tsx:31/34` (`viewIdOverride ?? activeViewId`) · `resolveDefaultViewId` (`viewStore.ts:273`). The agent-tree window is outside the model (`useCurrentViewId()` undefined). The AppLayout/PopoutPage split is out of step with D-2 "same path".
→ Reflected in body §3-4 (complete inventory + agent-tree `useCurrentViewId` main fallback) + §7-1 (single `WindowLayout(label)`, agent-tree excluded) + §7-2 (`useCurrentViewId`).

### G8. create_window label = reuse the popup prefix/counter
Basis: `is_popup_label` (`popout.rs:309`, `slot-popup-*` prefix) is the Destroyed cleanup gate (`lib.rs:157`). If `create_window` (D-6) used a different label scheme, cleanup would be skipped → leak.
→ Reflected in body §6 (`create_window` label = reuse `PopupCounter`/prefix).

### G9. spawn_into slot policy + failure visibility
Basis: §6 `spawn_into` with no slot / an occupied slot leaves focus/split/replace/error undecided → coder guesswork.
→ Reflected in body §6 (slot policy: unspecified → empty root slot / occupied → error · caller splits / on failure re-attach through `list_agents`).

### G10. Attach version to events
Basis: `window:tabs-updated`/`list_tabs` have no `version` so stale defense is impossible (must be consistent with keeping §2 `version` · the PopoutPage race mirror).
→ Reflected in body §6 (attach `version` to `window:tabs-updated`·`list_tabs`).

### G11. nits/notes
Basis · decisions:
- the `§5-1` pseudo "**delta.to_unsubscribe only**" (ADR-0046 BLOCK-1, prevents re-adding an eager Subscribe) → reflected in body §5-1.
- **invariant4** `close_window("main")` forbidden (main non-closable = hide only, `lib.rs:142`) → reflected in body §2-2 invariant4 + §6 (`close_window` forbids `"main"`).
- **`view_owner` decision (BAKE):** **keep** as a cached reverse-index (O(1) owner lookup + type-enforced unique ownership) — scan derivation not adopted → reflected in body §2.
- **PRD §6.5 "receive only the active tab" → superseded by ADR-0056 (all tabs)** → the PRD wording update is a follow-up with `/review doc` (§10). This TRD §4-1 is canonical.

### Re-review condition
Re-review after reflecting G1~G8 (the triggered risk = multi-tab cleanup/rollback/routing reversal, so at least `light` (1 Adversary doc-aware) or more, `full` if there is capacity). After PASS, record the tab ownership model ADR → `/implement critical`.
