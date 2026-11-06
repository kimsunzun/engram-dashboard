# Engram Dashboard — Requirements

Decisions that came out of the conversation, recorded in order.

---

## Tech stack

- Desktop: **Tauri v2** (Rust backend + the OS's built-in webview)
- Frontend: **React 19 + TypeScript**, build: **Vite**
- Style: **TailwindCSS v4 + shadcn/ui**
- State management: **Zustand** (official `@tauri-store/zustand` integration)
- Terminal rendering: **@xterm/xterm + react-xtermjs**
- Panel split: **allotment** (VS Code derived, dynamic panes)
- Agent tree: **react-arborist** (expand/collapse API built in)
- Code diff view: **@monaco-editor/react DiffEditor** (local bundle mandatory)
- PTY management: **portable-pty (Rust directly)** — tauri-plugin-pty was dropped because it supports only a single instance
- Rust↔TS types: **tauri-specta** (mandatory from the start — if added later, everything is reworked)
- Built from scratch, not a fork of CCB (Claude Code Bridge)

## UI structure

- Left sidebar: agent tree + state display (running / waiting / ...)
- Right main: the orchestrator agent terminal (xterm.js, large)
- When a sub-agent is created, a panel is added dynamically at the bottom right (n-way split)
- Bottom selection area: Monaco DiffEditor (code before/after change, Accept/Revert)

---

## Agent tree UI

- The tree can be collapsed/expanded
- Controls: provide the options "모두 펼치기" (expand all) and "하위만 펼치기" (expand only the children)
- Inactive agent: icon gray / active agent: active color
- If there are several agents of the same type, group them as a folder structure
  - Upper: agent type node (e.g. secretary agent)
  - Lower: instance 1, 2, 3 ...
- **Aggregated state of the parent node**: if some of the child instances are active, show a badge/half-fill on the parent icon → the state can be grasped even while the tree is collapsed
- **Display by number of instances**
  - 0: type node only (gray, flat) — shows that the agent type exists in the system
  - 1: type node = instance (flat, active color)
  - 2 or more: group instances 1, 2, 3... under the type node

---

## Terminal replacement

- The dashboard spawns the `claude` process directly as a PTY
- 1 xterm.js panel = 1 PTY = 1 `claude` process
- The xterm.js inside the dashboard fully replaces the role of the WezTerm panel
- In the end WezTerm is unnecessary — the dashboard runs as a standalone executable

## Background execution + focus switching (tmux style)

- PTY processes always run in the background in the Rust backend (50 of them is fine)
- Rust stores each PTY's output as a **scrollback buffer**
- The screen shows only the N panels the user selected
- When an agent gets focus: replay the buffer → then connect live streaming
- When focus is released: only the xterm.js connection is cut, the PTY keeps running
- Orchestration is fully in the background; the user picks and views only what they want to see

## Terminal output — virtual scrollback

- Use xterm.js's default virtual rendering (only visible rows are rendered in the DOM)
- Set an upper limit on the scrollback buffer per PTY (e.g. the most recent 10,000 lines) — memory-safe even with 50 agents
- Buffer replay on switching is also fast because it is virtual rendering

## Terminal output — click interaction (LinkProvider)

Register regex patterns with the xterm.js LinkProvider API → Tauri invoke on click:
- **File path** → open that folder/file in Explorer
- **diff block** → Monaco DiffEditor full-diff popup
- **URL** → open the browser
- **Error line** → jump to that line of that file
- Custom regexes can be added to match Claude output patterns

## Layout — slot structure

- **Slot** = a display container (a concept separate from the PTY process)
- Default N slots (e.g. 3) — any agent can be assigned/switched into each slot
- Slot header: current agent name + switch button (switching to another agent)
- Further splitting is possible inside each slot (nested split)
- The switching algorithm will be designed later

## Sidebar — agent management view

- Default: **tree view** (folder structure)
- Later option: **graph view** (connection map between agents/orchestration flow) — sidebar toggle
- When implementing the graph view, consider react-flow or d3-force

## Slot switching UI

- A **focus slot** concept — the slot last clicked/interacted with is the active one (border highlight)
- **Primary switch**: click a slot (focus) → click an agent in the tree → the focus slot switches to that agent
- **Secondary switch**: click the slot header directly → agent list popup → select (designate a specific slot without focus)
- The same feel as the VS Code editor group + file explorer combination

## Layout — Visual Studio style docking

- **Main window**: agent tree (fixed) + status bar (fixed) + default slot area
- **Floating window**: a slot torn off alone (same as detaching an editor tab in VS)
  - Supports both tab groups + split views
  - Floating windows can be merged with each other (drag docking)
  - Can be docked back into the main window
- The tree/status bar are fixed in main — only slots (terminals) are detachable

## Detachable scope of panels

- **Slot (terminal)**: detachable (default)
- **Agent tree**: detachable — stays in main as collapsed or fully hidden (multi-monitor support)
- **Status bar**: fixed in main

## Style & theme

- **Blunt high-contrast** style — no flashiness, clear lines, distinct boundaries
- Minimize animation (e-ink support + efficiency)
- No gradients/shadows — flat, sharp
- **Per-window theme** support (must be designed in from the start)
  - E-ink monitor window: white (light) theme
  - Regular monitor window: black (dark) theme
- CSS-variable-based theme system — hot-reload support (applied immediately without restart)
- Customization scope: the entire color token set (background, text, border, state colors, etc.)

---

## Open issues (stack review needed)

- shadcn/ui: its default style feels round and refined → may conflict with the blunt high-contrast style. After the requirements are fixed, decide whether to switch to using Radix UI directly.

## Splash screen (low priority)

- On app loading, the text "Engram" + a logo/subtext with an operations feel
- Omit it if there is no loading

## Dropdown display rules

- Agent tree: with 1 instance, show only the name flat with no expand arrow
- Slot header: if the agent has only 1 instance, show only the name with no ▼
- Show the dropdown only when there are 2 or more instances

## Per-agent token/cost display

- No separate API request — extract by parsing the Claude Code output from the PTY stream
- Parse target: the token/cost lines that Claude Code outputs (regex)
- Shown individually in the slot header, total summed in the status bar
- Risk: parsing may break if the Claude Code output format changes

- Location: next to each node of the agent tree + the slot header
- Toggleable (show/hide)

## Slot control method

- No buttons in the slot header — the header shows only the agent name (+ ▼ if 2 or more instances)
- **Right-click context menu**: split (horizontal/vertical), pop-out detach, agent switch, close slot
- **Keyboard shortcuts**: the above actions are reachable by shortcuts
- **Claude programmatic control**: the orchestrator can manipulate the layout directly with orch commands

## Slot agent name display

- No separate header bar
- A name overlay in the bottom-right corner of the terminal (xterm.js) (same feel as a WezTerm panel title)
- Laid over the xterm.js with absolute position
- Maximizes terminal space

## Agent spawn (later)

- Agents can be spawned directly from the dashboard
- Designate a specific slot/panel to bring up the desired claude process
- The orchestrator can also request a spawn programmatically

## Agent relationship model (important)

- Relationships between agents are **logical dependency relationships, not a folder/file structure**
- One agent can have **multiple parents** (a tree is impossible → DAG)
- How relationships are set:
  - Connected automatically by "who spawned it" at spawn time
  - Dependency relationships can be designated manually
- **Tree view**: show the hierarchy by choosing only one primary parent (default, easy to view)
- **Graph view**: show all of the actual DAG relationships (accurate but complex)
- Switch with the [Tree][Graph] toggle at the bottom of the sidebar

## Agent relationship model (confirmed)

- A method that separates an **agent list + group section**
  ```
  AGENTS
  ── 플래너 ●  $0.12
  ── 코더   ●  $0.21

  GROUPS
  ▼ 코딩룰
    플래너 · 코더 · 리뷰어
  ```
- An agent can belong to several groups (multiple membership)
- Data model: separate `agents[]` + `groups[]` → data survives UI changes
- Can later switch to tree/graph view (only the rendering is swapped)

---

## Items reflecting the GPT review (2026-06-07)

### Slot switching — changed to double-click (confirmed)
- Before: single-click on an agent tree → switch the focus slot
- Change: **double-click** → slot switch
- Reason: single-click risks mistakes + double-click is natural in combination with the right-click context menu

### Notification/event panel (added)
- A panel that gathers the events coming from agents in one place
- Included events: approval request, task complete, error occurred, user-specified notification
- Location: reserve space in the View (layout not fixed)
- Approvals should normally be absent — the main purpose is notification

### Agent Detail Panel (later)
- Detailed agent information (working files, token graph, command history, etc.)
- No problem for the current layout design
- On implementation: assign it to a slot or handle it as a separate popup

### Tech stack change
- `react-xtermjs` → **write our own xterm.js wrapper** (GPL license risk)
- Add a **debounce** to the resize event (allotment resize → FitAddon sync)

### Font/theme CSS variables added
- Added `--font-size-ui`, `--font-size-terminal`, `--font-size-code`
- Added `--line-height-ui`, `--line-height-terminal`
- Managed through the single ThemeManager entry point

### Agent controls (added as View dummies)
- Start / Stop / Kill buttons — included in the View in a dummy state
- Location: right-click context menu of an agent tree node

### State separation architecture (confirmed)
- `AgentSession`: actual agent state (tokens, status, work history) → a central store shared across the whole app
- `SlotView`: per-window layout (slot composition) → a per-window store
- `TerminalView`: xterm DOM instance → owned by the window, disposed when the window closes
- Effect: agent state is kept across pop-out detach/re-docking, only xterm is newly created and the buffer replayed

### Keyboard shortcuts (added as View dummies)
- `Ctrl+1~4`: switch slot focus
- `Ctrl+Shift+S`: split slot
- `Ctrl+Shift+P`: command palette (dummy)
- `Esc`: close panel
- Implemented with dummy handlers at the View stage, the real behavior connected later

### Review later
- LinkProvider security policy (file path whitelist, external URL confirm) — currently open without restriction, add it if it becomes inconvenient
- Broadcast Input: replaced by the orchestrator propagating to sub-agents
