# ADR-0035: Layout authority = the src-tauri client (daemon UI-agnostic, editor model)

- 상태: 확정 (2026-06-27, basis: `/research` deep report + user decision) · 부분 폐기 by ADR-0169 (the clause that the §5 control surface is __engramLayout)
- 관련: CLAUDE.md §5 (LLM control) · 「아키텍처 원칙」 throughout · ADR-0029 (daemon = single owner of data) · ADR-0004 (backend isolation) · ADR-0006 (lock order) · ADR-0011 (agentClient control surface) · ADR-0036 (transport relay — the topology counterpart of this decision) · `docs/research/multi-window-layout-authority-topology-research-2026-06-27.md` · step-log S14 · Amended by ADR-0057 (ViewManager internal model: global active_view_id (main-only) + window_bindings → per-window active + view_owner/windows tab ownership) · Amended by ADR-0169 (the clause that the §5 control surface is __engramLayout)

## 맥락
S14 multi-page layout — extending a single slot tree into multiple Views (tab switching + independent splits in popup windows). The key open question: **where to put the single source of truth for layout state (Views · split tree · slot→agent bindings)** — (a) the agent daemon / (b) src-tauri Rust / (c) each window's JS. rev.1 and rev.2 used JS authority (c) and got BLOCK twice in `/review trd` — the cause was a split-brain: in Tauri each window has an isolated JS context (no shared memory), so the moment a popup appears the main and popup stores become separate instances. rev.3 leaned toward daemon authority (a), but it was reconsidered over the layer-mixing dispute of "should the daemon know about the UI".

## 결정
**Layout authority = src-tauri Rust.** Put `Arc<Mutex<ViewManager>>` (views · active_view_id · window_bindings) in `AppState`. The daemon **knows nothing about Views** (only agents — hosting · I/O · policy). Each window's JS is a **pure renderer**: it mirrors the `layout:updated`/`view:list-updated` emits from src-tauri, and sends layout commands to src-tauri via invoke. OS window lifecycle (creating/closing popups) is also owned by src-tauri (the daemon cannot create Tauri windows). §5 LLM control = `window.__engramLayout` exposes the same handles as a human click, as an invoke wrapper.

## 거부한 대안
- **(a) The daemon owns layout authority** — in engram **agents and slots are decoupled**: even after `close_view` the agent does not die (the daemon survives) and can be reassigned to a slot. So a slot/View is a *presentation layer* on top of agents, not the session itself. tmux/Wezterm Mux keep layout on the server because there **pane == PTY** (layout = session) — engram lacks that premise. So we belong not to the tmux model but to the **editor model** (VS Code/Zed: files and language servers = shared backend, the editor grid = client-local). Coupling the UI to the agent host would also violate the spirit of ADR-0004 backend isolation. (Research report §constraint-fit table.)
- **(c) Each window's JS is the authority** — each Tauri window has an isolated JS context → the popup store and the main store are separate → close/dispatch cleans up only one side: a split-brain. Demonstrated by the two BLOCKs on rev.1 and rev.2.

## 근거
`/research` deep (three Claude Sonnet branches + two Codex BLIND cross-checks → opus adversarial verification, full convergence on the factual dimension). Two practices diverge: **terminal multiplexer = server authority / GUI editor = client-local**, and both families gave the same diagnosis of *why they diverge* (is the durable object a server-side PTY / does that window draw the UI grid). engram's **agent/slot decoupling** (its own design already settled — ADR-0016/0017, the close_view policy) is the deciding criterion → editor model → client (src-tauri) authority. ADR-0029 "daemon = single owner of data" forbids double-hosting *agent data*; it does not say to put layout into the daemon, and layout is a new client concern singly owned by src-tauri, so it is not double ownership (no violation).

## 영향 / 불변식
- `ViewManager` is owned by src-tauri `AppState`, as `Arc<Mutex<>>` (concurrent access from the invoke thread pool). **Emit after releasing the lock** (ADR-0006 lock order — no holding the lock during emit).
- View, slot, and layout types do not leak into the daemon crate/protocol (keeping the daemon UI-agnostic — aim for `rg "View\|Layout\|Slot"` to find 0 in the daemon crate).
- The frontend mirrors via Tauri listen on `layout:updated`/`view:list-updated` (a separate channel from the WS agentClient — separation of concerns). The authority is not JS → split-brain is impossible.
- `slotId` = UUID (globally unique across windows).
- §5: `window.__engramLayout.*` invoke wrapper = the same control surface for LLM and human. The frontend stays pure I/O.
- The topology (window ↔ src-tauri ↔ daemon connections) is specified by ADR-0036.
