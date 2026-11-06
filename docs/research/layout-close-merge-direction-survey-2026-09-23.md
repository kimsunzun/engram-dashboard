# Who absorbs the space when a slot is closed in a split layout — peer survey

- 상태: 확정 research · **decision = keep the current behavior (A — sibling absorption, the direction is decided by the split order · user decision 2026-09-24)** · Date: 2026-09-23 · Intensity: medium · design-decision mode
- Method: main-family (Claude) 3 collector tracks (terminals · multiplexers / tiling WMs · editors / layout UI libraries) + main grounding (local clone tmux · zellij code, 3 issue bodies via the GitHub API, source-text cross-check of VS Code `grid.ts` · the Emacs manual) + one cross-family (codex, effort high, web search) adversarial review — verdict REFUTE-PARTIAL, all 6 findings incorporated (2 missing peers · 1 misattribution · 2 overstatements · 1 version detail)
- Confidence legend: certain = 2 or more independent sources or direct check of local code / likely = supported by a single source / uncertain = unsupported · inference

## Question

When one slot in a 2×2 grid is closed, our app "always merges horizontally" (user report). What do other tools do, and what should we go with.

## Our current state (code checked)

- The layout = a **binary split tree** (`LayoutNode::Split{dir, ratio, a, b}`). Closing = **replace the closed slot's parent Split with the sibling node** (`src-tauri/src/layout/tree.rs:101` `close_in_tree`). It does not look at geometry.
- So the absorption direction is decided by **the split order that built the grid**. In a grid built by splitting top/bottom (top_bottom) first and then splitting each row left/right, `TB{LR{1,2}, LR{3,4}}`, the horizontal neighbor absorbs whichever cell is closed. If it is split left/right first, the vertical neighbor absorbs (reproduced with an audit probe — certain).
- The menu's "horizontal split" is `top_bottom` in the ADR-0140 vocabulary. Building in the order "horizontal split → vertical split of each cell" always gives a horizontal merge.

## Findings

### F1. The dominant pattern of tree-based tools = sibling absorption. The direction is decided by the split history — likely

Tools that keep the layout as a tree give the closed cell's space to **a sibling inside the same parent**. They do not look at the on-screen position (geometry). Most of the tools surveyed are of this family. But it is not "almost all" — there are a geometry-based family (F3) and a list-rearranging family (F3b) (adversarial review finding).

- Binary tree + the sibling takes the whole parent rectangle: WezTerm (`mux/src/tab.rs` `unsplit_leaf`), Windows Terminal (`Pane.cpp` `_CloseChild`), Ghostty (`split_tree.zig`), kitty Splits (`splits.py`), JetBrains (`EditorWindow.kt` `removeFromSplitter`), Terminator, Tilix, bspwm (`tree.c` `unlink_node`), Hyprland dwindle, herdr (`src/layout.rs` `remove_pane`), react-mosaic ≤v6.
- n-ary tree + to a sibling inside the same container: tmux, Vim/Neovim, Emacs, i3/sway, the VS Code editor grid, dockview, FlexLayout, Golden Layout, Lumino, KDDockWidgets.
- Local check: tmux (clone 5eaf557, 2026-06-14) `layout.c:541` "Merge the space into the previous or next cell" — the previous sibling, or the next sibling if it is first. 3.5a is the same. **The current master has changed to prefer the next sibling** (the adversarial review supplied a source link — not confirmed by main). Either way it is a tree sibling.

**Our behavior matches this dominant pattern.** The "always horizontal" the user saw is not a defect but the result of this pattern + the split order of our grid.

### F2. Distribution rules in an n-ary tree come in five kinds — likely

1. One adjacent sibling takes everything (tmux, Emacs default, VS Code `auto` when sizes are uneven, KDDockWidgets gives half to each side)
2. Proportional to current size (i3, FlexLayout, Golden Layout, Emacs `window-combination-resize`)
3. Even additive (react-mosaic v7, Lumino)
4. Equalize everything (Vim `equalalways` on by default, dockview, VS Code `distribute`)
5. A binary tree has only one sibling so there is no choice

We have a binary tree, so we fall under 5. The size-distribution policy is an axis separate from the merge direction.

### F3. The geometry-based family = Zellij · Sublime Origami. Zellij's horizontal-merge priority is fixed, and users complain — certain

- Zellij has no tree and uses flat geometry. When closing a regular pane it looks for a neighbor whose edge matches in the order **left → right → up → down** (checked in local clone bf8d23a `tiled_pane_grid.rs:1277-1287`). For a regular pane in a 2×2 it is a horizontal merge regardless of the split order. **But a stacked pane takes a separate path, and a fixed-size pane falls into a full re-layout after removal** — "always" is limited to regular panes (adversarial review finding).
- The Sublime Text Origami plugin's `destroy_current_pane` also picks an **adjacent pane whose edge matches**, with no tree, and has it absorbed (a grid-cell model — the adversarial review supplied a source link, consistent with the collector's report).
- Complaints about that fixed priority are open: zellij #1772 "Select pane orientation priority" (open) — after splitting left/right first and then splitting each pane top/bottom and closing, it says "zellij will prioritize horizontal over vertical", and requests a choice option or remembering the open/close order (body checked via the GitHub API). Discussion #2419 also expects it to return to the previous state (back-translated paraphrase; collector report, body not checked).

That is, **the same complaint as our user's also comes up in a geometry-based tool.** Fixing "horizontal first" by geometry is not a solution; it only changes the direction of the complaint.

### F3b. The list-rearranging family — neighbors do not absorb, the whole is redone — likely

Layouts that hold windows in **list order**, such as Qtile Matrix and awesome tile, do not decide who absorbs the closed cell. They recompute the whole layout from the remaining window list (the adversarial review supplied a source link, not confirmed by main). The xmonad · dwm family looks like the same kind but was not surveyed. The premise differs from our model, which handles a tree the user splits freely.

### F4. "Restore to the state before the split" is a stated design principle — certain

- Emacs Lisp manual "Recombining Windows": if the new window is deleted after a split, it "reestablishes the layout ... as it existed before the splitting" (gnu.org source text checked). Mechanism = `window-combination-limit`.
- The Vim `win_altframe` comment: the reason for switching which window receives depending on `splitbelow`/`splitright` = "opening a window and then immediately closing it will preserve the initial window layout" (collector report, Vim · Neovim source).
- VS Code #187431: a user reported as a bug that after "split editor right", closing the middle group merges **with the opposite side, not the side it was split from** (issue body checked via the GitHub API). Evidence that the user expectation is "back to before the split". **But its fix (PR #188898) is not a mechanism that remembers split history** — it changed the reference cell for size distribution to the index-adjacent cell (the adversarial review pointed this out from the diff). This case is not read as precedent for a "history restoration mechanism".

Binary tree + sibling absorption **automatically satisfies this principle when splits were made one at a time** (closing returns to just before that split). What breaks it: ① a grid made at once, like a preset ② when the user wants a merge in a direction different from "the order they split".

### F5. Few features let the user choose which neighbor absorbs on closing — likely

- tmux #1499: a flag to `kill-pane` that specifies the absorbing window was requested, and the maintainer said "Adding a flag to kill-pane is OK by me" and then it was closed (title · state checked via the GitHub API). The local clone's `kill-pane` has no such flag (collector report — usage `[-a] [-f filter] [-t]`).
- Sublime Text Origami plugin `destroy_pane(direction)`: the direction points to **the pane to be deleted**, and the current pane widens into that place. That is, not "close this pane and the other side absorbs" but **"delete the other pane and I absorb"** (corrected from the source by the adversarial review — the collector's report wrote the meaning the opposite way). That the direction choice is in the user's hands is the same.
- The argument of VS Code `removeView(view, sizing)` chooses the **size distribution**, not the absorbing neighbor (checked at `grid.ts:450`).

## Our options (fit to constraints)

Constraints: layout authority = backend (ADR-0035) · binary tree · every feature controllable by an LLM (CLAUDE.md 「LLM-우선 제어」) · drag resizing is not returned to the backend (ADR-0063).

| Option | What | Peers | Fit | Cost |
|---|---|---|---|---|
| A. Keep current | Sibling absorption. Direction = split order | All of the dominant pattern | High. The tree model unchanged | 0 |
| B. Fixed geometry priority | A neighbor whose edge matches, in a fixed order | Zellij, Origami (`destroy_current_pane`) | Low. A binary tree needs reconstruction logic, and the same complaint exists in Zellij (#1772) | Medium |
| C. Directional merge | The user picks the direction. The default close keeps A. Two forms: C1 "close this pane and the ←/↑ neighbor absorbs", C2 "delete the ← neighbor and I absorb" (Origami `destroy_pane`) | tmux request (accepted, then not shipped), Origami | Medium. Natural as an LLM command argument. But if the absorbing neighbor is not a tree sibling, the tree has to be reconstructed, only where the edges match | Medium to high |
| D. Equalize after closing | A size policy (separate from direction) | Vim default, dockview | Does not solve the direction problem | Low |
| E. Full re-layout | Redo the grid from the list of remaining cells | Qtile Matrix, awesome tile | Low. Throws away the shape the user split freely | Medium |

- Rejection candidates (→ ADR rejected-alternative candidates): **B** — the same direction complaint is open in a geometry-family peer (Zellij). A fixed priority only changes the direction of the complaint. **E** — abandons the premise of our model, a freely split tree.

## Issues · limits

- The tmux direction detail differs by version: 3.5a · local clone (2026-06) = the previous sibling, the latest master one collector saw = "prefers the next" (unconfirmed). It does not affect the conclusion (a tree sibling).
- The size-distribution details of iTerm2 · Konsole · QSplitter are unconfirmed. The group-close behavior of Sublime proper is unconfirmed.
- No academic or guide literature on UX principles was found.
- No precedent code was found for C's tree reconstruction (converting a 2×2 whose edges match between left/right first ↔ top/bottom first). Design needed.
