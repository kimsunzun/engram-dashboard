# Split divider and size policy — peer survey on who owns the dragged size · minimum size · keyboard · DOM order

- Status: confirmed survey · **decision = D1~D6 + R11 — user decision 2026-09-24, ADR-0227** · date: 2026-09-24 · depth: medium · design-decision mode
- Method: 3 main-family (Claude) collector tracks (owner of the dragged size · command · double-click / minimum size · window shrink · keyboard / DOM order · fractional pixels) + main grounding (checked against the originals of local-clone code: herdr `065ef9d`, tmux, orca `df7460af`, paseo) + one cross-family (codex, effort high, web search) adversarial review — verdict **REFUTE**, all 12 findings addressed (1 logic error · 2 scope omissions · 1 concurrency omission · 2 wrong generalizations · 2 overstatements · 2 missing counterexamples · 1 misattribution · 1 missing caveat). Where the conclusion changed: the cost of D1-B (F1) · D5 (F2) · D6 (F6)
- Confidence legend: certain = 2 or more independent sources, or direct code check / likely = supported by a single source / uncertain = unsupported or inferred
- Prior survey: `docs/research/split-layout-library-survey-2026-09-24.md` (the decision to implement the flat renderer ourselves). This document is the basis for the user decisions D1~D6 of that implementation.

## Questions

When we implement the flat renderer (a list of leaves + positions computed from the tree) ourselves:

- D1. Where do we keep the pane size changed by dragging (screen vs backend tree) · can it also be changed by a command
- D2. Minimum pane size and its propagation in nested splits
- D3. Adjusting the divider by keyboard
- D4. When the window becomes smaller than the minimum sum
- D5. Double-click on the divider
- D6. DOM order of the leaves · fractional-pixel boundaries and the terminal

## Our current state (code checked)

- The split tree (`id`·`dir`·`ratio`·two children) is owned by the shell backend, and the frontend draws the snapshot (ADR-0035). Every split is born with ratio 0.5, and there is no command that changes the ratio (`src-tauri/src/layout/commands.rs:544-612` — TRD-draft investigation).
- Dragged sizes are not written back to the backend (the comment in `src/components/layout/ViewLayoutRenderer.tsx` and the 2026-07-10 entry of `docs/process/step-log.md` put this outside the scope of ADR-0063, but the body of ADR-0063 has no such clause — surfaced by the ADR-0227 document review). The 「영향」 (Impact) of ADR-0068 states that once a resize command exists, the backend must enforce a minimum pane size, but the backend does not know pixels.
- Layout changes reach the screen only through backend events, and there is no optimistic update (`src/store/viewStore.ts` — review finding). The snapshot version guard only drops **late-arriving events**; it does not decide which of two writes that arrive at the same time wins.
- In the flat renderer the ratio is directly an input to the pane position computation, so a changed backend ratio is reflected by a re-render alone — no separate imperative resize path is needed, unlike allotment (design premise — prior survey F5).
- Design principle (CLAUDE.md 「LLM-우선 제어」 (LLM-first control)): what the UI does must also be doable by an LLM through the same backend command, and the frontend does not own control.

## Findings

### F1. Owner of the dragged size — three families, and the peers with the same model as ours are on the 「백엔드 명령」 (backend command) side — certain (herdr·tmux) / likely (the rest)

- **Backend-command family**
  - **herdr** (binary split + direction + ratio — the same model as ours, the server owns the layout): while the client drags the divider, it sends `layout.set_split_ratio {tab_id, path, ratio}` every 33ms — **the same API method that scripts use** (`src/client/shell/mouse.rs:1118-1143` — original checked). The server checks that the value is finite, **clamps the ratio to 0.1~0.9**, and writes it into the tree (`src/layout.rs:275-277`); on each accepted write it schedules a session save and emits a `layout_updated` event carrying the whole layout (`src/app/api/layouts.rs:221-251` — original checked). Session restore brings the ratio back (`src/persist/restore.rs:826-835` — collector report).
  - **tmux**: the default binding that drags a border with the mouse is itself a command — `bind -n MouseDrag1Border { resize-pane -M }` (`key-bindings.c:470` — original checked). The size lives in the server's layout cell.
- **Mixed family — the screen owns it, and the server follows to match**: WezTerm manipulates splits in the GUI, and the mux server rebuilds the split sizes (kept on reconnect) — review report (`mux/src/tab.rs`), not checked against the original.
- **Family where the screen owns it and copies it into persistence**: Wave (frontend layout model → confirmed at the end of the drag → lazily saved to a backend object) · orca (the DOM `style.flex` is the source of truth, read out at serialization time — `layout-serialization.ts:98-118`) · paseo (client store `sizes[]`) · VS Code (view hierarchy, serialized into the workspace state). On this side we did not find a size command that scripts could call (VS Code's relative commands are the exception).
- **Shape of size commands**: relative increments are common — VS Code `workbench.action.increaseViewSize`/`decreaseViewSize` · WezTerm `wezterm cli adjust-pane-size <방향> --amount <셀>` · kitty `kitten @ resize-window` · herdr `herdr pane resize --direction --amount`. **Absolute sizes exist too** — tmux `resize-pane -x/-y` sets width and height as absolute values (including percent relative to the window), and the default binding uses `-x50% -y50%` (`cmd-resize-pane.c:39` · `key-bindings.c:409` — review report). A command that sets the ratio by split **path** was found only in herdr.
- **★A ratio clamp does not guarantee a minimum pane size (review finding — it overturns the draft's conclusion)★**: herdr clamps each split separately to 0.1~0.9 and rounds the rectangles, but does not enforce a pane minimum (`src/layout.rs:691` rectangle computation). A 100px pane can become 10px, and when 0.1 splits overlap, a descendant shrinks down to 1px. That is, herdr did **not solve** the pixel problem of ADR-0068; it **did not enforce** it. paseo also applies the ratio 0.1 (`packages/app/src/stores/workspace-layout-constants.ts:1` — original checked) only per pair.
- We found no design document that explicitly discusses 「화면 소유 vs 모델 소유」 (screen ownership vs model ownership) — the classification above is read from the shape of the code.

### F2. Double-click on the divider — no consensus — likely

- orca: resets both sides to half and half (`flex 1 1 0%`) (`pane-divider-drag.ts:262-274` — original checked).
- VS Code: divider double-click differs by context — it sizes to fit the content or shrinks the sidebar to its minimum (release notes·layout docs — review report). ★#123953, which the draft cited as evidence, is a **feature request** asking for a half-and-half reset, not evidence of an implementation★.
- Zed: spreads the space evenly (search summary — uncertain). herdr: we did not find double-click handling for the pane split divider (it exists only on the sidebar divider).
- We did not find anyone who resets to a saved default ratio (our current behavior = allotment `preferredSize` → reset to the backend ratio).

### F3. Minimum pane size — most check 「직계 두 칸만」 (only the two direct panes), and few roll lower-level minimums up — certain

| Tool | Value | What it checks | Source |
|---|---|---|---|
| orca | 50px, effective `min(50, 전체/2)` | the two panes on either side of the divider | `pane-divider-drag.ts:15, 244-248` (original checked) |
| Wave | 40px, if violated the move is **rejected** (not clamped) | the two nodes before and after | `layoutModel.ts` (collector report) |
| paseo | ratio 0.1, pair minimum `min(0.1, 짝/2)` | the adjacent pair | the constants file above (original checked) · `resize-handle-sizes.ts` |
| herdr | ratio 0.1~0.9, no pixel or cell minimum | that one node only | `src/layout.rs:276` (original checked) |
| zellij | 5 cells | the pane that shrinks | `tab/mod.rs:154-155` (collector report) |
| tmux | 1 cell (+ border) · **propagates** — sum for children in the same direction, maximum for the other direction | the whole subtree | `layout.c:421-470` (original checked) |
| VS Code gridview | branch minimum = sum along the axis · maximum on the cross axis | the whole subtree | splitview/gridview (via summary — likely) |
| Unity SplitView | merges child minimums recursively (sum along the axis · maximum on the cross axis) | the whole subtree | `Editor/Mono/GUI/SplitView.cs` (review report) |
| allotment | 30px default | its own level | `modern.mjs:963` |

- In the direct-only approach, dragging an ancestor divider can squeeze a deep descendant below its minimum. tmux splits the reduction among the children in the same direction one cell at a time in turn, and skips a child that has reached its minimum (`layout.c:476-521` — collector report).

### F4. When the window is smaller than the minimum sum — within what we surveyed, no one hides panes — likely

- tmux: does not shrink the layout below the minimum and draws it clipped beyond the window (comment at `layout.c:604-629`). VS Code splitview: clamps each pane to its minimum and shows a scrollbar. orca·paseo: lower the minimum to `짝/2`, so panes shrink below the minimum (orca comment 「음수 flex 를 막으려고」 (to prevent a negative flex) — original checked). paseo restore: if `칸 수 × 0.1 > 1`, it falls back to an even distribution.
- ★zellij·Wave·herdr·WezTerm were not surveyed — do not generalize to 「아무도 숨기지 않는다」 (nobody hides them)★.

### F5. Keyboard adjustment — a standard draft exists, but terminal peers do it with a 「포커스 칸 리사이즈 명령」 (focused-pane resize command), not through the divider — likely

- Standard: WAI-ARIA APG 「Window Splitter」 — a focusable `role="separator"` · `aria-valuenow/min/max` · arrows to move, Enter to collapse/expand, Home/End (optional), F6 to cycle panes (optional). ★APG itself states that this is a pattern with **review not completed**★ (review report). react-resizable-panels implements it (arrows 5% — secondary source).
- Terminal peers (orca·Wave·paseo·herdr·tmux·zellij) do not give the divider keyboard focus. paseo only attaches `role="separator"` and has no key handling (`resize-handle.tsx:261-271` — collector report). herdr has a **command** that finds the nearest split in the arrow direction from the focused pane and adds ± to its ratio (`src/layout.rs:279-304` — collector report).
- 「크기 명령이 있으면 키보드 접근성이 충족된다」 (if a size command exists, keyboard accessibility is satisfied) is a claim that has to be shown separately in our UI — it is only a peer practice, not a proof of equivalence (review finding).
- Drag-to-collapse (snap) exists only in general-purpose UI libraries (VS Code splitview·allotment·react-resizable-panels), and not in terminal peers.

### F6. DOM order and fractional pixels

- **DOM order — terminal state preservation and accessibility trade off against each other — certain**: react-mosaic is in key order (independent of layout), and states itself that, at that cost, Tab and screen-reader order follow the key rather than the screen (`MosaicRoot.tsx:57-63` — original checked). Wave TileLayout is in tree-path order (= screen order), so nodes move when the layout changes (collector report). **The side effects of a DOM move are real**: orca writes that when a terminal container is moved, the browser clears `scrollTop`, and near the limit Chromium kills the oldest WebGL context without `contextlost`, and it does a two-step restore and WebGL re-attach (`pane-split-scroll.ts:83-93` — original checked). An ordinary move also loses focus (Chrome 133+ `moveBefore()` preserves it — Chrome blog). However, **a drag that only changes sizes moves no node under any order policy** — a move happens only when the tree shape changes.
  - Summary: id order **never moves terminal nodes**, so it blocks the side effects above at the source. The cost is that focus and screen-reader order differ from screen order, and that has to be filled by separate means such as `tabindex`, `aria-flowto` and a directional focus command.
- **Fractional pixels — likely**: VS Code splitview and dockview accumulate sizes as integer px with `Math.round` and attach `left/top` in px (via summary). orca rounds the measured value used for fit decisions so that 「소수 떨림이 리사이즈로 읽히지 않게」 (fractional jitter is not read as a resize) (`pane-fit-client-size.ts:1-13` — original checked). FitAddon truncates the computed size to an integer and rounds the cell count down (installed `@xterm/addon-fit`) — a fractional size is silently cut off. CSS `round()` is supported in Chrome/Edge 125+ (caniuse — WebView2 follows the Edge version). ★xterm.js #3584, which the draft cited as evidence, is a report that 「세로 리사이즈 중 너비가 바뀐다」 (the width changes during a vertical resize), so it is not evidence of fractional boundary → column wobble (review finding — removed)★. **There is no evidence that directly links a half-pixel pane position to blurry WebGL text — that is for measurement.**

## Options (fit to constraints)

Constraints: ⓐ LLM-first control (what the UI does goes through the same backend command) ⓑ the backend is the source of truth for the tree ⓒ scope of this replacement ⓓ peer basis

| Decision | Option | ⓐ | ⓑ | ⓒ | ⓓ |
|---|---|---|---|---|---|
| D1 | A. screen-only override (keyed by split id) | ✕ (status quo) | ✕ screen and backend ratios diverge | ○ small | Wave·orca·paseo·VS Code |
| D1 | B. drag → backend ratio command (herdr shape) | ○ | ○ | △ design requirements below | herdr·tmux |
| D2 | only the two direct panes (flat minimum) | — | — | ○ | most (orca·Wave·paseo·herdr·allotment) |
| D2 | roll lower-level minimums up (sum·maximum) | — | — | △ algorithm | tmux·VS Code·Unity |
| D3 | no divider keyboard · size command (LLM · later key binding) | ○ | — | ○ | terminal peers |
| D3 | APG divider focus | — | — | △ | react-resizable-panels |
| D5 | half and half | — | — | ○ | orca |
| D5 | reset to the backend ratio (current) | — | — | ○ | none |
| D5 | no double-click behavior | — | — | ○ | herdr (pane divider) |
| D6 | id order + no order correction | — | — | ○ | react-mosaic |
| D6 | screen order | — | — | △ DOM-move side effects | Wave |

### What must be decided together when choosing D1-B (review finding — the draft's 「명령 하나 + 비율 제한」 (one command + ratio clamp) was an underestimate)

1. **Preview and commit** — during the drag the screen shows a local preview, and the backend is written once at the end of the drag (Wave shape) or by throttling (herdr 33ms). Drawing only from backend round trips may make the drag look choppy (round-trip latency · number of events · the terminal fit and PTY resize cost of every update are not yet measured).
2. **Minimum size has two layers** — the pixel minimum is enforced during the drag by the screen, which knows pixels, and the backend applies only the ratio clamp. Then a ratio that an LLM sets by command can squeeze a deep pane below its minimum — decide whether to allow that (herdr) or have the backend block it further based on pane counts.
3. **Concurrent writes** — when a user drag and an LLM command overlap on the same split, the last write in backend arrival order wins (default). A reconciliation rule is needed between the preview during the drag and the incoming backend value.
4. **Undo** — the layout has no undo. The problem of grouping a drag into one unit does not arise this time (if the commit is once at the end of the drag).
5. **Popout · multiple windows** — a split belongs to one view and a view is owned by one window, so addressing by `view + split id` is enough (inference).

## Open issues and limits

- Every finding of the review verdict REFUTE carried evidence, so all were addressed — no contested items remain.
- Parts of Wave·VS Code·Zed·WezTerm·Unity·react-mosaic were read only through web summaries or reviewer reports (not checked against the originals).
- The window-shrink behavior of zellij·Wave·herdr·WezTerm was not surveyed.
- WebGL text blur at half-pixel positions and the drag feel of D1-B are for GUI measurement.
