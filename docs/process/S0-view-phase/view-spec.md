# Engram Dashboard — View Spec v2 (for GPT review)

> How to have GPT review it: upload this file as an attachment. Do not paste the contents directly into the chat window.

## Project overview

**Engram Dashboard**: a desktop app that manages several Claude Code AI agents at once.
Agents are currently managed with WezTerm terminal panels; this replaces that with a dedicated desktop app.

**Current stage**: before connecting the backend (Rust PTY), finishing the frontend View with dummy data.

---

## Tech stack (View only)

| Layer | Choice | Note |
|---|---|---|
| App shell | Tauri v2 | Rust backend + Edge WebView2 |
| UI | React 19 + TypeScript + Vite | |
| Style | TailwindCSS v4 + Radix UI | not shadcn — use Radix directly |
| State | Zustand | manages dummy data |
| Terminal view | @xterm/xterm + **own wrapper** | written ourselves because of the react-xtermjs GPL license problem |
| Panel split | allotment | VS Code derived |
| Agent tree | react-arborist | |
| Diff view | @monaco-editor/react DiffEditor | local bundle mandatory |
| Font | CSS-variable based | ThemeManager single entry point |

---

## Overall layout

```
┌─────────────┬──────────────────────────────────┐
│ Agent Tree  │  Slot 1        │  Slot 2         │
│ (사이드바)  ├────────────────┼─────────────────│
│             │  Slot 3        │  Slot 4         │
├─────────────┴──────────────────────────────────┤
│ 알림/이벤트 패널  │  Status Bar                 │
└─────────────────────────────────────────────────┘
```

- Sidebar (agent tree): collapse/expand, can be detached into a separate window
- Main area: N slots (dynamic split)
- Bottom: notification/event panel + status bar

---

## Agent tree

**Data model**:
```ts
agents = [
  { id: '1', name: '비서', type: '비서', status: 'running', cost: '$0.12' },
  { id: '2', name: '코더-1', type: '코더', status: 'idle', cost: '$0.21' },
  { id: '3', name: '코더-2', type: '코더', status: 'running', cost: '$0.08' },
]
groups = [
  { id: 'g1', name: '코딩룰', members: ['1', '2', '3'] },
]
```

**Tree display rules**:
```
AGENTS
── 비서  ●  $0.12
▼ 코더
   ── 코더-1  ◌  $0.21
   ── 코더-2  ●  $0.08

GROUPS
▼ 코딩룰
   비서 · 코더-1 · 코더-2
```
- Same type, 1 agent: flat / 2 or more: grouped
- State icons: running (green) / idle (gray) / error (red)
- Parent node aggregate badge (state visible even when the tree is closed)
- Token/cost toggle display
- Right-click context menu: Start / Stop / Kill (dummy)

**Slot switching**: **double-click** an agent → the focus slot switches to that agent
(single-click was not adopted because it risks conflicting with the right-click menu; double-click was adopted)

---

## Slot system

- **Slot** = a display container (separate from the PTY process)
- **Focus slot**: the last clicked slot = active (border highlight)
- Double-click on the agent tree → switches the focus slot
- Agent name: absolute overlay at the bottom right of xterm.js (no header bar)
- 1 instance: name only / 2 or more: ▼ dropdown

**Slot controls (right-click menu)**:
- Split horizontal/vertical
- Pop-out detach → a separate Tauri window
- Agent switch
- Close

**Docking**:
- Merge pop-out windows with each other
- Dock back into the main window

---

## Terminal — xterm.js

- Write our own wrapper (replaces the `react-xtermjs` GPL)
- Test with dummy ANSI output
- **LinkProvider**: click interaction for file paths/URLs/diffs
- **resize debounce**: allotment drag → ResizeObserver → debounce → FitAddon.fit()
- Virtual scrollback (only visible rows rendered in the DOM)

---

## Theme system

```css
:root[data-theme="dark"]  { --bg: #0a0a0a; --text: #e0e0e0; --border: #333; }
:root[data-theme="light"] { --bg: #f5f5f5; --text: #1a1a1a; --border: #ccc; }
:root[data-theme="e-ink"] { --bg: #ffffff; --text: #000000; --border: #000; }
```
- Per-window theme switching
- Managed through the **ThemeManager** single entry point
- Hot reload (applied immediately without restart)
- Blunt high-contrast — no shadows/gradients

---

## Font system

```css
/* 패밀리 */
--font-ui: 'JetBrains Mono', monospace;
--font-terminal: 'Cascadia Code', monospace;
--font-code: 'Fira Code', monospace;
--font-claude-prose: 'Inter', sans-serif;
--font-claude-code: 'JetBrains Mono', monospace;
--font-claude-path: 'Cascadia Code', monospace;
--font-claude-header: 'Inter', sans-serif;

/* 사이즈 */
--font-size-ui: 13px;
--font-size-terminal: 14px;
--font-size-code: 13px;

/* 행간 */
--line-height-ui: 1.4;
--line-height-terminal: 1.2;
```
- Replaceable from the settings JSON, applied immediately on change

---

## Notification/event panel

- Gathers the events coming from agents in one place
- Event kinds: approval request / task complete / error occurred / user-specified notification
- Approvals should normally be absent — the main purpose is notification
- Location: bottom (toggleable within the layout)

---

## Monaco DiffEditor

- Local bundle (`loader.config({ monaco })`)
- Shows a dummy diff
- Accept / Revert buttons (enabled after onDidUpdateDiff)
- Bottom panel toggle

---

## Status bar

- Number of running agents / error presence / total token sum
- Fixed at the bottom

---

## Review requests

Changes relative to v1:
- react-xtermjs → replaced by our own wrapper
- Slot switching single-click → double-click
- Notification/event panel added
- font-size/line-height CSS variables added
- ThemeManager single entry point
- resize debounce
- Start/Stop/Kill dummy controls added

Review requests:
1. Were the v1 findings properly reflected in this v2?
2. Is there anything else we missed?
3. Are there React 19 + Monaco Editor version compatibility issues? Which combinations need version pinning?
4. Known problems with the allotment + own xterm.js wrapper combination?
5. Overall, is it possible to build a practical AI agent management tool with this design?
