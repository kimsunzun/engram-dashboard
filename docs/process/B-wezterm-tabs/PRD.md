# PRD — WezTerm-style window > tab > slot (layout B, Brick 2+)

> Status: **draft + adversarial review done (waiting on user decision)** · 2026-07-09 · autonomous session. `/review prd`: direction OK · BLOCK because of spec gaps → strengthened in §10 and the decision items expanded (the user reads up to §10 and picks).
> This document is a PRD that lays out options and trade-offs so **the user chooses** (no arbitrary adoption). Major decisions are recorded as ADRs after the choice.
> Discovery chain: `docs/README.md` → layout B track (Brick 1 = commit 297796e, popup = bebbf66) → this document.

## 0. One line

Handle several agents **like WezTerm**: **several windows (popups) → a tab strip in each window → split slots inside each tab.** Today "window" and "slot" work, but the **"per-window tab"** layer between them is missing. We fill it.

## 1. Concept model (direction confirmed by the user)

```
창(Window/팝업)  ── WezTerm 창          [됨: 런타임 팝업 bebbf66]
  └ 탭(Tab = 코드의 "View")             [갭: 탭 줄 UI + 창별 소유]
      └ 슬롯(Slot = pane, 분할)          [됨: split_slot]
```

- **Window** = an OS window (main + runtime popups). Several by purpose, such as a "code agent window" and a "personal work window".
- **Tab** = one set of slot arrangement (= `View` in code). Keep several tabs inside one window and switch between them.
- **Slot** = one cell inside a tab (one agent, or split again).

**Fixed terminology (to avoid confusion):** the user's "tab" = **View** in code. The user's "window (A/B)" = **OS window (popup)**. From here on this document writes it as **tab (=View)**.

## 2. Current state — done vs gap (measured by investigation)

| Layer | State | Evidence |
|---|---|---|
| Several windows (runtime popup) | ✅ done | `pop_out_slot` (bebbf66) |
| Slot split | ✅ done | `split_slot`/`close_slot` (manager.rs) |
| Tab (View) create/switch **behavior** | ✅ backend done | `create_view`/`switch_view` (manager.rs:168,234) |
| **Tab strip UI** | ❌ none | 0 tab bar components in the frontend |
| **Per-window tab ownership** | ❌ none | ViewManager = one global `active_view_id` + `window_bindings` (popup fixed to 1 view) |

**Core gap:** today there is **one global active view** (main) + **popup = fixed to 1 view**, so the concept of "a window has a tab list and switches inside it" does not exist at all. Building that is the body of this work.

## 3. OSS evidence (design survey — delegated to /research)

| App | Where tabs live | Window↔tab | Move between windows |
|---|---|---|---|
| **WezTerm** | MuxWindow (the window) owns its own tab list (mux is the global registry) | 1:1 | possible (per pane) |
| **VS Code** | EditorGroup (per window) owns | strict per-window | possible (drag) |
| tmux | the server owns everything, a session references windows by winlink | M:N shared | possible |
| zellij | the session's Screen owns the tab BTreeMap · **active tab is per client** | 1:1 | unconfirmed |

**Recommendation:** **per-window tab ownership + global UUID tab IDs**. It is the model validated by WezTerm and VS Code, and since there is currently no advanced requirement like "expose the same tab in two windows at once", the complexity of a global-pool (sharing, GC, syncing in 2 places) is unnecessary. If the tab ID is a global UUID, it can be extended gradually later to inter-window move/global-pool.

**Sources:** the official wezterm.mux API and DeepWiki source, VS Code custom-layout docs, tmux/zellij DeepWiki. (Full URLs = this session's research log.)

## 4. ★Core design decisions (user decision needed)★

### D-1. Tab ownership model — where to put the tab list

- **(A) Per-window ownership [recommended]** — each window (main included) holds `{ tabs: Vec<ViewId>, active: ViewId }`. The View itself (slot tree) stays in the global `views: Vec<View>` pool as now, but **which tab belongs to which window · that window's active tab** is owned per window. (= WezTerm: global mux registry + MuxWindow owns the tab list.)
  - Pros: clear ownership, cleaning up the tabs that belong to a window when it closes is natural, self-contained storage per window, `{window, tab}` coordinates are intuitive for LLM control.
  - Against the rejected alternative: a global-pool is over-engineering for the current use case.
- **(B) Global pool + mapping** — `tabs: HashMap<TabId, Tab>` + `windows: {tab_ids, active}`. Advantageous for tab sharing and workspace switching but GC and syncing are complex. **Rejected for now, follow-up if needed.**

> **Recommendation = A.** Replace the current `active_view_id: Uuid` + `window_bindings: HashMap<label, view_id>` with `windows: HashMap<WindowLabel, WindowTabs { tabs: Vec<ViewId>, active: ViewId }>`. Main ("main") has the same structure, with no special treatment.

### D-2. Unify the main window as a "regular window that has tabs"?

- **(Yes) [recommended]** — main is also just one `WindowTabs`. Remove the `active_view_id` special treatment → main/popup share the same code path (ADR-0035 simplification).
- (No) — main keeps the global active, only popups have tabs. → keeps two branching paths (complex, not recommended).

### D-3. When a window closes, what about its tabs (views)?

- **(Recommended)** When a window closes, all of its tabs (views) close. **But the agents (daemon processes) do not die** — the §5 hands-and-feet / brain separation (only the display surface disappears). Extend the current `cleanup_popup_window` (Destroyed cleanup) to multi-tab.
- Alternative: reparent the tabs of the closing window to main (prevents loss). → WezTerm avoids this with "0 tabs → window closes automatically". **Default = closes, reparent is an optional follow-up.**

### D-4. Moving a tab between windows (drag) — in scope this time?

- **(Recommended) Out of scope this time** — the user said "later" earlier. But **design D-1 with global UUID tab IDs** only so it can be added on later. (The WezTerm `move_to_new_window` pattern.)

### D-5. Save/restore tab/window arrangement — in scope this time?

- **(Recommended) Out of scope this time** — merges with roadmap D-7 (frontend localStorage, deferred until after daemonization). For now only runtime state.

> **Purely internal (the main session decides and reports, not a decision):** tab strip UI position (top of each window) · style (shadcn tabs) · [+] button · routing (rebuild OutputRouter based on "the window's active tab") · events (per-window `view:list-updated`).

## 5. Scenarios / edge cases (BDD style — for acceptance checking)

- **Switching among several tabs:** the main window has 2 tabs (code agent · personal work) → clicking a tab changes that window's canvas to the corresponding view. The LLM's `switch_tab(window, view)` does the same.
- **Popups also have tabs:** a popup window also has a tab strip, and tabs can be added/switched inside that window (today fixed to 1 view → extended).
- **Closing a window:** when a popup window closes, that window's tabs disappear, but those agents stay alive in the daemon (can be attached again from another window).
- **Single-window edge:** when there is only 1 window, "tab to a new window" defaults to creating a new window (no move target).
- **Closing the last tab:** if the last tab of a window is closed? → (linked to decision D-3) the window also closes or an empty tab stays. Recommendation = keep one empty tab (main always has at least 1 tab).
- **LLM control (§5 invariant):** all of the above are exposed as commands — `create_tab(window)`, `switch_tab(window, view)`, `close_tab(window, view)`, `move_slot_to_window` (extending the existing pop_out). A human click = the same command.

## 6. Acceptance criteria (Phase 1 = tabs)

1. A **tab strip** appears in each of the main and popup windows, and shows that window's tab list and active tab.
2. Tab click/`switch_tab` → only **that window** changes to the corresponding view (other windows unchanged — independent per window).
3. `create_tab`/[+] → a new tab (empty slot view) is added to that window and activated.
4. `close_tab` → that tab closes, the agent survives.
5. Routing: each window receives the agent output of **its own active tab** (independent progress per window, ADR-0046 kept).
6. Measured on the GUI (cdp+EnumWindows): end to end, create and switch 2 main tabs + switch a popup tab.
7. The LLM control path comes with a command (§5). Major decisions as ADRs.

## 7. Follow-up Phase sketch (not this time — next in order)

### Phase 2 — Make the main window's left agent spawn a "slot"

- **Current:** the Sidebar (AgentTree + spawn) is a **fixed panel** outside the slots (frontend local state). The slot system is backend-authoritative.
- **Goal:** make that spawn/tree area **one kind of slot content** → incorporated into the layout (movable, poppable, LLM-controllable).
- **Underlying decision item (existing roadmap gap):** the **slot content kind model** — what a slot holds = agent terminal / agent tree / (later diff·api view). This merges with the "mode system" (terminal/claude/codex/api). Add a type `content: AgentTerminal(id) | AgentTree | …` to the slot. **A separate PRD is needed** (when Phase 2 starts).

### Phase 3 — Mini orchestration (messages only)

- **Goal (user: "just the message"):** one agent sends one line of text to another agent.
- **Minimal proposal:** the `send_message(to_agent, text)` command = inject text into the target agent's stdin (reuse the existing `write_input`). Address = agent id/name.
- **Decision items:** address scheme (id vs human name) · stdin injection vs a structured inbox · the surface the LLM calls. Start with **minimum = stdin injection**, structured as a follow-up.
- **One-cycle demo (user goal):** spawn agents A/B/C/D by command → some into a new window (tab) → A messages C → confirm arrival.

## 8. Open decision list (waiting on the user's pick)

- **D-1** tab ownership = A (per-window) / B (global pool) — recommend **A**
- **D-2** unify main as a regular window = yes / no — recommend **yes**
- **D-3** tabs when a window closes = close (agents survive) / reparent — recommend **close**
- **D-4** tab move between windows = this time / follow-up — recommend **follow-up (only prepare the ID)**
- **D-5** save/restore = this time / follow-up — recommend **follow-up (merge with D-7)**
- **Scope:** implement only Phase 1 (tabs) this time / 2 and 3 together — recommend **Phase 1 first** (2 and 3 each get their own PRD)

## 9. Next steps (process)

1. The user picks D-1~D-5 above + scope → major ones as ADRs.
2. TRD (fix the ViewManager model change · OutputRouter · events · frontend tab bar interface).
3. Implementation = `/implement` (coder → review → qa, including GUI measurement).

---

## 10. Adversarial review reflected (/review prd — 2026-07-09)

Verdict: **User lens (Opus) = FIX(5) · Tester lens (Codex) = BLOCK(8).** Both accept the direction (per-window tabs, the D-1 A family) — the reason for BLOCK is not "wrong direction" but **spec gaps that must be fixed before the TRD** (model invariants · migration · edge state machine · routing/replay · concurrency). Autonomous-mode conservative aggregation = **BLOCK stays** (entering implementation is held until the user decides). The PRD is strengthened below.

### 10-1. D-1 re-organized — ownership invariant + a missed option 3 (C)
- ★Invariant to be fixed★: **does one View (tab) belong to only one window?** The current router allows one View to be routed to both main-active and popup-bound at the same time (`output_router.rs:156`) — the new model must state forbid/allow explicitly. **Recommendation: unique ownership** (one View = one tab of one window), sharing as a follow-up.
- **(C) Owner-index hybrid [proposed by Codex — strong candidate]:** `views: HashMap<ViewId,View>` + `view_owner: HashMap<ViewId, WindowLabel>` + `windows[label].tabs: Vec<ViewId>`. Keeps the global lookup + enforces unique ownership through the type — removes the pretense that "ref-list = ownership" (the conceptual blur of A). **→ A vs C re-examined: C is cleaner (recommendation raised).**

### 10-2. Additional decisions (surfaced by the review — user picks)
- **D-6. Empty-new-window creation command** — today the only window creation path is `pop_out_slot` (detaching a slot holding an agent); there is no "open an empty new window". Essential for the demo ("spawn C in a new window") → `create_window` (1 empty tab). **Recommendation: add.**
- **D-7. Placement-designated spawn** — spawn and placement (`assign_agent`) are separate, so "spawn D in a new tab of window B" is multi-step. The real demo flow = designate placement → spawn → window/tab/slot placement → routing, which needs an acceptance item that ties it into one flow. **Recommendation: fold into the Phase 1 acceptance criteria.**
- **D-8. Keyboard navigation (Ctrl+Tab etc.)** — a core WezTerm UX but the PRD is silent. Because §5 is LLM-first it can be deferred, but **it is a user-facing trade-off so decide explicitly.** Recommendation: only Phase 1 minimum (Ctrl+Tab switching) or a follow-up — user's pick.
- **D-3 confirmed (state machine)** — "close window" and "close last tab" are out of step in 3 places (D-3 vs §5 vs the `close_view` code `manager.rs:186` = forced creation of an empty view = the keep-the-tab side). State the rules by window kind: **main = always at least 1 tab** (an empty tab stays even if the last is closed) · **popup = closing the last tab also closes the window** (WezTerm style, recommended). Put the result of `close_tab(window, last)` into the acceptance criteria.

### 10-3. Specs that the TRD must fix (the gaps behind the BLOCK)
1. **`window_bindings` migration** — move the current `active_view_id`+`window_bindings` (popup fixed to 1 view, hidden from main-metas `manager.rs:74`) to the new `windows` model. Includes handling existing popups and the `PopoutPage ?view=` URL (if not done: orphans/duplicates/exposure).
2. **Routing/replay semantics on tab switch (ADR-0046 sensitive)** — does a hidden tab stop receiving frames? Does remounting the active tab request a full replay? If two windows watch the same agent, is progress independent? Must be consistent with ADR-0046 (view-direct replay · slot `lastDeliveredSeq` · wire Subscribe forbidden). **Promoted from "purely internal" — user-facing + ADR-sensitive.**
3. **Concurrency contract** — serialization and no-op/error results when `create_tab`/`switch_tab`/`close_tab`/`pop_out` are called concurrently. The current router relies on ViewManager's serialized mutation (ADR-0006), keep that.
4. **pop-out mid-flight rollback** — prevent orphans/duplicates when the target window closing/cleanup overlaps with installing the new tab list during window creation.

### 10-4. Phase order reconsidered (User lens FIX-3)
Phase 1 (tab bar) alone does not reach the demo ("A→C message") at all — the payoff is Phase 3 (≈trivial, reusing the existing `write_input`). **Option: a thin vertical slice** = 2 slots (already done) + `send_message` (Phase 3 minimum) first → the orchestration demo before the tab bar. **User's pick: Phase 1 first vs pull send_message forward.**
