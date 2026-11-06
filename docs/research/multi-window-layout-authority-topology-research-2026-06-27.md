# Research — Multi-Window Layout Authority Location & Connection Topology (2026-06-27)

**Status:** done (deep · design-decision mode)
**Method:** Claude (Sonnet) 3-track BLIND fan-out + Codex 2 BLIND independent cross-checks → Opus aggregator cross-comparison + adversarial verification
**Confidence legend:** certain / likely / uncertain
**Trigger:** S14 multi-page layout F1 (ViewManager ownership) — the 3 existing research reports covered only "layout state synchronization" and **left the authority location + connection topology uncovered** (a gap). This report fills that gap.

---

## Decision questions

- **Q-A (layout authority):** Where does the single source of truth for Views, the split/pane tree and the slot→agent mapping live: (a) the agent daemon / (b) src-tauri Rust / (c) each window's JS?
- **Q-B (connection topology):** Each window with a direct WS to the daemon, or src-tauri holding a single WS and multiplexing/relaying to the windows?

---

## Key findings (Claude ↔ Codex fully converged)

### F-1. Terminal multiplexers = **the server (daemon) is the layout authority** — certain
tmux ("all its state in a single main process, the server"), Zellij (the server's `Screen` owns tab→pane), WezTerm Mux (a central singleton owns Window→Tab→Pane), GNU screen (the daemon saves/restores the region layout), mosh (the server is the authority on terminal screen state, the client has a synchronized copy + predictive echo). The client = a thin renderer/input. **Basis: detach/reattach, simultaneous views from several clients and crash isolation hold only on the premise that "only the server knows the state."**
- **Codex addition (likely):** WezTerm is owned by **the GUI process for the local domain, and by the mux server for Unix/SSH/TLS remote domains** — the authority location splits by domain. That is, *the moment it becomes remote, server authority is forced*.

### F-2. GUI editors = **layout is local to the window/project** — certain
VS Code (EditorPart is window-local, SharedProcess centralizes only file watching, terminals and extensions, not layout), Zed (Workspace/PaneGroup are window-local, App merely owns memory and is not an architectural authority), JetBrains (a workspace.xml per project, a single JVM). Electron's "main = single source of truth" is the authority on **business state (files, sessions, settings)**, not on the *layout grid*. Real-time layout synchronization across windows is absent or rare (persistence goes through shared storage keyed by workspace/project).

### F-3. **Divergence of the two families and the reason** — the same diagnosis from both families, meta-convergence
- Multiplexers: the durable objects are **server-side PTY, pane and screen state** → server authority enables detach/multi-client/bandwidth-aware rendering.
- Editors: the editor grid is **UI state that the window draws and interacts with** → local. The center holds only lifecycle, files and services.
- **Implication:** "which family are we" decides the answer.

### F-4. Chromium browser process = the authority on tab/window placement — certain
`Browser` + `TabStripModel` own the tab collection, the order and "tear off tab into new window" (`DetachWebContentsAt`). The renderer is a content view and has no say over which window/tab it belongs to. browser→renderers = a single authority + Mojo fan-out.

### F-5. Wayland compositor = the single authority on window placement — certain
xdg_toplevel has no API for setting an absolute position at all. A client submits only a `wl_surface` over a single socket, and the compositor decides the final placement (security, multi-monitor flexibility). (X11 differs — a client can request a position and the WM intercepts it.)

### F-6. LSP = multiplexing many documents over a single connection — certain
The editor↔server uses a single transport (stdio/socket) and multiplexes the didOpen/didChange of all documents, told apart by URI. It does not create a connection per document. (`lspmux` exists separately = by default one client ↔ one server.)

---

## Cross-verification table

| Claim | Claude | Codex | Decision |
|---|---|---|---|
| Terminal multiplexer = server authority | certain | high | **converged** (Codex added the WezTerm local/remote domain nuance) |
| GUI editor = window-local layout | certain | high | **converged** |
| Divergence of the two families + the reason | stated | stated | **converged (same diagnosis)** |
| Chromium browser process placement authority | certain | high | **converged** |
| Wayland compositor single authority | certain | high | **converged** |
| LSP single-connection multiplexing | certain | high | **converged** |
| **Q-A recommendation: engram → daemon authority** | (not asked) | **high** | Codex alone + Opus synthesis agree |
| **Q-B recommendation: src-tauri single-connection relay** | (not asked) | medium-high | Opus: consistent in the long run but large in scope → split off |

**Zero cross-family disagreements on facts.** Even applying the unanimity caveat (suspicion of a shared bias), primary sources (source code, man pages, official docs) back it, so the chance of hallucination is low.

---

## Constraint-fit table — Q-A (engram constraints × options)

| Constraint | (a) daemon authority | (b) src-tauri authority | (c) each window's JS |
|---|---|---|---|
| ADR-0029 "the daemon = the single owner of data, no in-proc double ownership" | ✅ consistent | ❌ **revives app-process state ownership** (what ADR-0029 killed) | ❌ |
| §5 LLM-first control (the brain = the daemon side controls the layout) | ✅ co-located | △ needs a back channel (daemon→src-tauri) | △ |
| Remote daemon / detach·reattach (the workspace follows you) | ✅ tmux-style, free | ❌ the layout does not follow remote | ❌ |
| The practice of our family (= terminal multiplexers, the ADR-0013 anchor) | ✅ matches | ❌ assumes the editor family | ❌ |
| Reuse the existing WS broadcast path | ✅ (`onLayoutUpdated` is isomorphic to `onProfileListUpdated`) | △ a second Tauri emit channel | — |
| Avoid JS split-brain from window isolation | ✅ | ✅ | ❌❌ (the cause of the 2 BLOCKs in rev.1 and rev.2) |
| Scope/risk | medium (add ViewManager + protocol to the daemon, **transport unchanged**) | low (fast) · **a structural regression** | — |

**Conclusion (close to certain): Q-A = (a) daemon authority.** Triple consistency: the practice of our family (terminal multiplexers) + §5 + ADR-0029 "single owner". (b) is a regression that revives the double ownership ADR-0029 removed.

---

## Q-B (connection topology) — long-term consistency vs the current scope

- **Practice (LSP · Chromium · Codex medium-high):** src-tauri holds a single connection and multiplexes/relays to the windows = auth, reconnection, ordering and session de-duplication + a single LLM control surface + remote credentials/TLS in one place in Rust.
- **Counterargument (Codex's self-rebuttal):** a direct connection per window is better for **failure isolation + a direct path for high-bandwidth terminal output**. A relay adds a daemon→Rust→WebView hop + a copy of PTY bytes.
- **Conflict with current code:** today `wsTransport.ts` has a **direct WS from each window to the daemon** (the ADR-0020 transport seam). Going to the Q-B relay means redesigning the transport = beyond the S14 scope.
- **Decision:** Q-B (full relay) is **split off into a separate decision/ADR.** S14 = daemon authority (Q-A=a) + **keep the current direct-per-window connection** — the daemon broadcasts the layout over each window's existing WS (F2/F3 resolve automatically). Unifying through a relay comes later.

---

## Rejection candidates → ADR rejected alternatives (move to the S14 ViewManager ADR)
- **(b) src-tauri owns the layout authority:** revives ADR-0029's double ownership + a §5 back channel + does not follow remote. (Handoff plan A = this → rejected.)
- **(c) each window's JS as authority:** split-brain from window isolation (demonstrated by the 2 BLOCKs in rev.1 and rev.2).
- **(Q-B) full src-tauri relay now:** consistent in the long run, but it redesigns the transport (ADR-0020), so it is beyond the S14 scope — in a later ADR.

---

## Gaps / limits
- The WezTerm nuance "local domain = GUI-owned" does not apply to us (we always attach to the daemon) — we are always the remote type (= server authority).
- How Zed/JetBrains synchronize *the same project* across several windows is left unsettled as "uncertain" by both families (no effect on our decision).
- The high-bandwidth output overhead of the Q-B relay is **unquantified before measurement** — to be decided in a later ADR on profiling evidence.

## Sources (primary first)
tmux man7 · tmux Control Mode wiki · Zellij DeepWiki (client-server · layout) · issue#4253 · WezTerm multiplexing official · mux/src/lib.rs · GNU screen manual (Layout/Displays) · mosh.org techinfo · VS Code Source-Code-Organization · layout.ts · Sandbox blog · Zed workspace.rs · project.rs · GPUI ownership · JetBrains Remote Dev · Electron process-model/ipc · Chromium multi-process · TabStripModel · Wayland Protocol Book · xdg-shell · LSP 3.18 spec · overview · lspmux.
