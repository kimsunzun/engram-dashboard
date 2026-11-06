# Focus rules in split layouts — peer survey on focus after closing and on control panels

- 상태: 확정 research · **decision = C (previous content pane → first empty pane → none) + refusing focus on control panes — user decision 2026-09-24, ADR-0224** · Date: 2026-09-24 · Intensity: medium · design-decision mode
- Method: main-family (Claude) 3 collector tracks (terminals · multiplexers · tiling WMs / editors · IDEs · docking libraries / control panels and the work-target pane) + the third track run once more with Sonnet (pilot — the result comparison goes to the research skill's feedback) + main grounding (local clone tmux · zellij code, source-text cross-check of VS Code `editorPart.ts` · Zed `workspace.rs` · Emacs `window.el`) + one cross-family (codex, effort high, web search) adversarial review — verdict REFUTE-PARTIAL, all 6 findings incorporated (3 omissions · 2 logic gaps · 1 missing source)
- Confidence legend: certain = 2 or more independent sources or direct check of code / likely = supported by a single source / uncertain = unsupported · inference

## Questions

1. When the focused pane is closed, where does focus go.
2. Can a control panel such as the tree or sidebar become the "work-target pane" (the pane that open · create goes into).

## Our current state (code checked)

- There is one focus pointer per view (`focused_slot_id`, owned by the backend — ADR-0035).
- Click focus is accepted only by content panes (`empty`/`agent`) — the frontend skips control panes (`agent_list`/`preset_palette`) (ADR-0073).
- The tree's 「열기」 goes to the focused pane if the focus is a content pane, otherwise to the first empty pane, and if there is none shows guidance (`src/components/agent/selectOpenTarget.ts`).
- Closing the focused pane moves focus to the tree's **first pane** (`src-tauri/src/layout/manager.rs` `fixup_focus`). If the first pane is the tree, the focus ring shows on the tree (a leftover appearance issue that ADR-0073 put on hold).
- The LLM's explicit focus (`slot.focus`) is accepted by control panes too.

## Findings

### F1. Focus after closing — there is no common industry rule, there are four families — certain

1. **The previously focused pane (MRU / focus history)** — most editors and multiplexers.
   - tmux: the head of the per-window previous-pane stack, and if empty, the previous pane → the next pane (local clone 5eaf557 `window.c` `window_lost_pane` — checked).
   - zellij: among the remaining panes, the one whose last active time is latest (local clone bf8d23a `tiled_panes/mod.rs` `move_clients_out_of_pane` — checked).
   - VS Code editor group: the previously active group `MOST_RECENTLY_ACTIVE[1]` (`editorPart.ts` — checked).
   - Emacs: the default of `delete-window-choose-selected` is `'mru` (`window.el` — checked). The only case where `'pos` (position) · `nil` (first window) can also be chosen by a setting.
   - kitty (group history deque) · bspwm (per-desktop history) · herdr (one previously focused pane) · Hyprland `focus_on_close=2` (collector report).
   - i3 · sway: not a global MRU but **MRU within the nearest surviving ancestor** (hierarchical — collector report).
2. **The sibling that absorbed the space / tree order** — Vim · Neovim splits (the absorbing window becomes current), JetBrains editor split (the first window among the siblings), Windows Terminal (the absorbing sibling → its first leaf), WezTerm · Ghostty · iTerm2 (the earlier pane in tree order, otherwise the later pane) (collector report).
3. **The window under the pointer (cursor)** — Hyprland `input:focus_on_close=1` (official option doc — flagged as an omission by the adversarial review). A mouse-centric WM family.
4. **The first pane, or no successor** — web docking libraries. dockview (the first remaining group — `baseComponentGridview.ts` `doRemoveGroup` · `dockviewComponent.ts` `activateFallbackGroupIfRemoved`, collector's source reading · the official docs say only that there may be no active group — likely) · FlexLayout (the active tabset disappears) · Golden Layout (focus is released) (collector report). **Our current behavior is in this family.**

- **If a pane without focus is closed, focus stays as it is** — every implementation checked picks a successor only when 「닫힌 것 == 활성」 (closed one == active) (collector report, confirmed with tmux code).
- **Every MRU implementation falls back to a position rule when the history is empty** (previous → next, or the first leaf).
- **User complaints** — Neovim #20455 「창을 닫으면 직전 창으로 가게 해 달라」 (an MRU expectation against the absorbing-sibling rule — collector report). Ghostty discussion #8831 「분할을 닫으면 포커스가 아무 데도 없는 경우」.

### F2. Tools that keep panels separate exclude control panels from work targets — they split the pointer in two — likely

★**It is not that 「어느 도구도 패널을 작업 대상으로 삼지 않는다」**★ (no tool treats a panel as a work target) — there is a counterexample family (adversarial review finding + Sonnet pilot collection): **a model where the browser is an ordinary buffer and opening replaces its place** — Vim netrw default (the selected file replaces the browser window), oil.nvim, neo-tree `window.position = current`. ★**This model has already been rejected in our app**★ — ADR-0073 treated the tree pane being covered by the agent screen on 「열기」 as a defect and blocked it.

Tools that keep panels separate hold **keyboard focus** (anywhere, including the tree) and the **work-target pane** (the pane that open goes into) separately. The work-target pane **moves only when focus goes to a content pane** — clicking the tree leaves it as it is, and the tree's 「열기」 goes to that pane.

- VS Code: `activeGroup` = "An active group is the default location for new editors to open". The explorer is not a group (collector report, docs).
- Zed: focus also goes to dock panels, but open goes to `last_active_center_pane` (the center pane focused last) (`workspace.rs` — checked).
- JetBrains: `FileEditorManagerEx.currentWindow` — a tool window is not an editor window (collector report).

Where the tree is placed **inside an ordinary split**, it is blocked with an eligibility filter — closest to our shape.

- NERDTree: the previous window if it is 「쓸 수 있으면」 (usable), otherwise the first usable window, and if none, a new split.
- nvim-tree: by default a window picker, and with `eject` it does not open into the tree window. Terminals · help, etc. are excluded from the candidates.
- neo-tree: `open_files_in_last_window`, `open_files_do_not_replace_types = { "terminal", … }`.
- Neovim `winfixbuf` · VS Code 「잠긴 그룹」: a content pane too can be locked so that it is not a target without explicit designation.
- Emacs: a side window is dedicated by default and is excluded from cycling by `no-other-window`.

- **A ring that marks only the work target is rare** — VS Code colors the active group's tab differently (a theme token — the meaning detail unconfirmed). tmux's 「표시된 칸(marked pane)」 is the closest precedent (collector report).

## Our options (fit to constraints)

Constraints: focus is backend authority (ADR-0035) · the LLM moves the same handle (「LLM-우선 제어」) · 「열기」 **changes** the pane's content (the agent process lives but that pane's screen changes, and if it is the preserved screen of a dead agent it disappears — ADR-0148).

| Option | Focus after closing | Control panes | Peers | Cost | Risk |
|---|---|---|---|---|---|
| A. Keep current | The first pane (may be the tree) | Only clicks excluded, LLM-explicit allowed | Web docking libraries | 0 | The focus ring shows on the tree + then 「열기」 goes to the first empty pane (or guidance if none), not the pane the user was working in — it is not purely appearance |
| B. The first **empty** pane → if none, no focus | Empty panes only | The backend excludes them too (refuses explicit focus) | Same grain as nvim-tree's eligibility filter | Low | If there is no empty pane focus disappears and directional moves (`left`, etc.) lose their reference point |
| C. MRU (previous content pane) → if none, the first empty pane → none | The pane used just before | Not entered into the history + refuses explicit focus | tmux · zellij · VS Code · Emacs default | Medium (keeping · cleaning up per-view history) | If the previous pane is an agent pane, 「열기」 changes that pane's screen — the same shape as the regression reverted today. ★It cannot be seen as the same as VS Code★ — VS Code's 「열기」 **adds** a tab to the group (and when closing a group it also moves the editors into another group), while we **change** the pane's content |
| D. Two pointers (keyboard focus / work target) | The work target follows rule C | Keyboard focus can be on the tree too, the work target is content only | VS Code · Zed · JetBrains | High (wire · frontend · the whole LLM surface) | Inherits C's risk as is + the surfaces double so the LLM description · ring display must be redone |

- **Peers that keep panels separate agree that 「제어 칸은 작업 대상이 아니다」** (a control pane is not a work target) (the browser-replacing model was already rejected by ADR-0073) — our current behavior, where the LLM's explicit focus is accepted by control panes, departs from that consensus.
- Rejection candidate (→ ADR rejected-alternative candidate): **the absorbing-sibling rule** — our tree may have a tree pane as the sibling, so an eligibility filter is needed anyway, and in that family (Neovim) there are complaints wanting MRU.

## Issues · limits

- The tmux `window_lost_pane` line number differs by collector (local clone 845 vs master 1138) — a version difference. The rule is the same.
- The layout-specific meaning of the Hyprland default (0 = 「다음 후보」), kitty's edge case of removing an inactive group, and whether focus moves when an unfocused pane is closed in Ghostty GTK are unconfirmed.
- Sublime Text · react-mosaic could not be surveyed (abstained). For the file-browser family only netrw · oil.nvim · neo-tree were looked at (the two the adversarial review added are not confirmed by main).
- All behavior is based on reading code · docs, with no actual run observation.
