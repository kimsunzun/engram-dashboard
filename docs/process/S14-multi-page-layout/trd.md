# TRD — S14: multi-page layout (rev.5)

**Status:** review reflected — awaiting coder (rev.4 `/review trd full` FIX reflected + phasing discarded)
**Written:** 2026-06-27
**Basis ADRs:** ADR-0035 (layout authority = src-tauri) · ADR-0036 (transport relay unification — straight to the target structure with no phasing)
**Basis research:** `docs/research/multi-window-layout-authority-topology-research-2026-06-27.md` (deep, Claude+Codex cross) and 2 others
**Review:** rev.4 = Architect-breaker (opus) + Designer (Codex) both **FIX, no BLOCK** (judged the architecture sound). This rev.5 reflects the FIX + the user decision (do it properly without legacy → phasing discarded).

---

## Architecture direction (confirmed — ADR-0035/0036, no phasing)

**src-tauri = a single daemon client + protocol owner + output router + layout authority.** The daemon = agents only (View-agnostic). A window = a pure renderer (a thin agentClient on TauriTransport). **The legacy per-window connection is discarded — not kept even temporarily.**

```
[데몬]  에이전트만 (PTY·출력·정책). View·창 모름
   ▲ 단일 WS (에이전트 명령/출력)
[src-tauri Rust = 허브]
   · DaemonClient   — 단일 연결·재연결·resubscribe·epoch·seq dedup·routing (TS에서 이전)
   · ViewManager    — 레이아웃 권위(Views·split 트리·slot→agent·window_bindings)
   · OutputRouter   — agentId → ViewManager snapshot 조회 → 해당 창에만 fan-out
   ▼ Tauri IPC: invoke(커맨드) / emit(레이아웃) / Channel(라우팅된 출력)
[main창] [popup창] [tree창]  순수 렌더러 (창마다 JS 격리 → 권위는 src-tauri Rust)
```

Criterion (ADR-0035): engram **decouples** agents/slots (an agent survives close_view and can be reassigned) → slot = display layer → editor model (client-local layout). Topology (ADR-0036): single-connection multiplexing (LSP · Chromium practice) — per-window connections are legacy, not practice.

---

## ★Implementation risk gate (stated first)★

**Relocating the transport layer (DaemonClient's connection · reconnect · resubscribe · epoch · seq dedup from TS→Rust) is this project's concurrency-critical area.** The zombie-socket guard · generation token · epoch race · replay→live order that the current `wsTransport.ts`/`protocolClient.ts` block must be preserved in Rust **without reproducing the races** (invariants: ADR-0001 kill causality · 0005 finalize once · 0006 lock order · 0007 epoch resubscribe). → **This module is TDD-enforced + `/review code deep` (concurrency trigger).** The gate is not writing it fast but writing it correctly.

---

## Rust-side design (`src-tauri/src/`)

### ViewManager — layout authority (ADR-0035)

```rust
// AppState 소유: Arc<Mutex<ViewManager>>  (invoke 스레드풀 동시접근 → 락)
pub struct ViewManager {
    pub views: Vec<View>,
    pub active_view_id: Uuid,                    // = 메인 창 탭 선택(아래 의미론 참조)
    pub window_bindings: HashMap<String, Uuid>,  // window_label → view_id
}
pub struct View { pub id: Uuid, pub name: String, pub layout: LayoutNode, pub focused_slot_id: Option<Uuid> }

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNode {
    Slot  { id: Uuid, agent_id: Option<String> },   // agent_id = 데몬 에이전트 "참조"(소유 아님)
    Split { dir: SplitDir, ratio: f32, a: Box<LayoutNode>, b: Box<LayoutNode> },
}
```

**Hard contracts (review FIX — Codex F2):**
- `ratio`: 0.0~1.0 **clamped** (out-of-range input rejected/clamped). Default 0.5 on split.
- **root slot close:** closing a View's last slot → that View is in an empty state (empty screen + a centered `+`). Closing even the last View is the same (empty screen + `+` = `create_view`).
- **focus fallback:** if the slot `focused_slot_id` pointed to disappears → fall back to the first slot of the tree (None if none).
- **invalid id:** a non-existent `view_id`/`slot_id` → **no-op + return an error** (no panic/partial change).
- **agent_id is not validated (FIX — opus F7):** `assign_agent` stores only the reference string. It does **not** ask the daemon whether it really exists → 0 DaemonClient calls while holding the ViewManager lock (ADR-0006). (Unassigned/dead agents are shown at render time.)

**switch_view multi-window semantics (FIX — Codex F1):**
- `active_view_id` = **the main window's active tab**. Popup · tree windows are bound to a **fixed View** via `window_bindings` (render only their own view, no tab switching).
- `switch_view(view_id)` = changes the main window's active tab. No effect on popups.

### DaemonClient — protocol owner (ADR-0036, ★concurrency-critical)
- A **single WS** connection to the daemon + reconnect + **subscription union tracking** + resubscribe (on reconnect · epoch change) + seq dedup + epoch guard. Moves the current TS semantics to Rust.
- A window's spawn/kill/write/resize/subscribe goes invoke → DaemonClient → daemon.

### OutputRouter — output routing (FIX — opus F6: avoid locks on the hot path)
- Receive a daemon output frame → `agentId` → "the View containing the slot that hosts that agent → the windows watching that View" → deliver only to those windows' Channels.
- **The routing table is a lock-free snapshot refreshed when the ViewManager changes** (do not lock the ViewManager Mutex per frame — compliance with ADR-0006 "no send while holding a lock").

### invoke handlers
- **Layout:** `create_view(name?)→Uuid` · `close_view(view_id)` · `switch_view(view_id)` · `split_slot(view_id,slot_id,dir)→Uuid` · `close_slot(view_id,slot_id)` · `assign_agent(view_id,slot_id,agent_id)` · `open_view_in_popup(view_id)→String` · `close_popup(window_label)` · `get_view(view_id)→ViewSnapshot` (including version, race below).
- **Agent:** `spawn`/`kill`/`write`/`resize` → via DaemonClient.
- **Invariant (ADR-0006):** modify under the ViewManager lock → emit **after releasing the lock**. No external calls (DaemonClient/emit) while holding the lock.

### emit / Channel
```
layout:updated     { view_id, layout, focused_slot_id, version }   // version = 팝업 race용
view:list-updated  { views: ViewMeta[], active_view_id }
(per-창 Channel)    라우팅된 에이전트 출력
```

### Popup (ADR-0036)
- `open_view_in_popup`: duplicate guard (`get_webview_window`) → `WebviewWindow::builder(/popup?viewId=X)` → `window_bindings.insert`.
- **race (FIX — opus F4):** on popup mount `get_view(viewId)` → keep the response's `version` → then register listen → discard emits arriving via listen at or below the kept version (closes the window between pull ↔ listen).
- `on_window_event` CloseRequested → `close_popup` (clean up window_bindings) + explicit Tauri #15583 unlisten.
- **The tree window is also a target of the same dynamic creation** (FIX — opus F5: also replace `window.open('#/tree')` in `Sidebar.tsx`).

---

## React-side changes

### Remove
- **Delete** `src/store/slotStore.ts` (JS layout authority = the cause of split-brain).

### New
- `src/store/viewStore.ts` — emit mirror (views · activeViewId · currentLayout).
- `src/store/layoutTypes.ts` — LayoutNode · SplitDir · ViewMeta (ts-rs mirror) + pure render helpers.
- `src/components/layout/ViewTabBar.tsx` — tabs + `+` (including the empty state) → create/switch/close_view invoke.
- `src/api/tauriTransport.ts` — `Transport` implementation (src-tauri IPC carrier). `agentClient` = the `ProtocolClient` interface (ADR-0011) kept, the brain is Rust → this carrier is thin.

### Modify (file paths corrected — FIX opus F5)
- `src/store/eventBus.ts` — add Tauri `listen("layout:updated"/"view:list-updated")` (a channel separate from WS). **Remove the existing `{dispatch}` key of `__engramLayout`** and replace it with invoke wrappers (if not removed the LLM calls the deleted store → crash).
- `src/api/clientFactory.ts` — carrier `WsTransport` → `TauriTransport`.
- `src/components/layout/AppLayout.tsx` — ViewTabBar + viewStore subscription.
- `src/components/layout/LayoutRenderer.tsx` — **(omission corrected)** the body of the recursive LayoutNode render. Subscribes to viewStore.
- `src/components/slot/SlotPane.tsx` **and** `src/components/layout/SlotPane.tsx` — remove the slotStore dependency from **both** (the coder confirms which is the live path, then cleans up). ⚠️ dashboard1 coordination.
- `src/components/slot/SlotContextMenu.tsx` — remove `window.open` → `open_view_in_popup` invoke. ⚠️ dashboard1 coordination.
- `src/components/.../Sidebar.tsx` — tree `window.open` → dynamic window invoke.
- `src/components/agent/AgentTree.tsx` — dispatch → `assign_agent` invoke.
- `src/pages/PopupPage.tsx` — remove the direct `useSlotStore` read (currently the proof of split-brain) → `get_view(viewId)` pull (version) → listen.
- `tauri.conf.json` — remove the static `slot-popup` window → dynamic creation.

### slotId: `number` → **UUID** (globally unique across windows).

---

## LLM control surface (§5 — ADR-0035)
`window.__engramLayout = { splitSlot, closeSlot, assignAgent, createView, switchView, closeView, openPopup }` — each an invoke wrapper. Wherever it is called from in the multi-window setup it reaches the same src-tauri AppState ViewManager (opus F3 confirmed). The frontend stays pure I/O.

---

## Policy
- **close_view:** collect the agentIds but **do not kill the agents** (the daemon survives, AgentTree shows "미배정"). "Kill on close" is a follow-up parameter.
- **Message lock/gating:** policy enforce = **the daemon** (the single choke point). The src-tauri relay only carries, the frontend only displays (ADR-0036).
- **Swappability guard (FIX — Codex F3):** do not leak routing · window concepts into `agentClient` · the protocol crate. 0 View/Layout/Slot types in the daemon crate (`rg` gate).

---

## Acceptance criteria
1. Tab bar View add/switch/close → src-tauri ViewManager change → emit → re-render of all windows. Closing the last View = empty screen + `+`.
2. A popup renders that View independently + splits. By the `get_view` version pull→listen, no loss or duplication of the initial emit. On close, window_bindings cleanup + #15583 unlisten.
3. `window.__engramLayout.*` works in main · popup. (Confirm the removal of the existing dispatch key.)
4. slotId is a globally unique UUID. `ratio` 0~1, invalid id no-op.
5. N windows work over a **single daemon connection** + 1 subscription per agent + the OutputRouter does not send output to non-displaying windows (lock-free snapshot).
6. 0 View/Layout/Slot types in the daemon crate (`rg` gate) — UI-agnostic.
7. **Transport-relocation concurrency invariants preserved** (ADR-0001/0005/0006/0007) — regression asserted via TDD, `/review code deep` passed.
8. `cargo test` (root) + `cargo build` + `npm test` + `npx tsc --noEmit` pass + `scripts/cdp.mjs` measurement.

---

## Open items
- **dashboard1 coordination:** common modification of SlotPane (2 places) · SlotContextMenu — ping before touching (agreed).
- **resize path (dashboard1 Task1):** after unification resize also goes through src-tauri (invoke→DaemonClient). Task1 (80x24) is fixed on the current path and only the carrier is replaced at unification (interface-unchanged carry-forward). **Multi-view resize policy** (the same agent in windows of different sizes → tmux window-size style smallest/largest) = decided in src-tauri since it owns the layout.
- **OutputRouter snapshot refresh mechanism:** regenerate the routing snapshot when the ViewManager changes — the exact data structure is a coder spike.
- **Persistence (future):** ViewManager restart restore. ★If the persistence location mixes with the daemon data dir (ADR-0024), there is a risk of reviving ADR-0029 (daemon = single owner of data) (opus F1) → when deciding persistence, fix the location boundary in an ADR.
