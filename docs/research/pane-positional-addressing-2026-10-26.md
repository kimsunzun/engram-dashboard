# How Panes Are Addressed by Position in a Split-Tree Layout — 2026-10-26

- **Status:** light summary · no decision — an input to T-65 (LLM layout commands). Earlier survey = `llm-command-design-query-vs-action-2026-10-26.md`.
- **Method:** `/research` light (the user specified 「간단하게」) · subclass engram · 1 collector (7 searches) · main grounding (tmux · ace-window · niri source text checked). No adversarial review (light).
- **Question:** the user's proposal 「왼쪽부터 x1 y2 식 좌표」 — in a split tree that is not a grid, how do other tools address a pane by position?

## Conclusion

**Among the tools we looked at, none addresses a pane in a split tree by 「열 N · 행 M」 coordinates.** They use four things instead — ① corner · edge · direction tokens ② an ordinal (creation order or reading order) ③ a person looks at numbers or letters drawn on the screen and picks ④ names · marks. Confidence = likely (a sample of 4 tools · source text confirmed for 2).

| # | Claim | Source | Grounding | Confidence |
|---|---|---|---|---|
| 1 | A tmux pane number is not a position but the **creation order** — "Panes are numbered beginning from zero in the order they are created." | [tmux(1)](https://man7.org/linux/man-pages/man1/tmux.1.html) | supported (main checked) | likely |
| 2 | tmux spatial tokens = four corners (`{top-left}` …) · four edges (`{top}` · `{bottom}` · `{left}` leftmost · `{right}`) · four neighbors relative to the current pane (`{up-of}` · `{left-of}` …) · `{next}`/`{previous}` (by number) · `{marked}`. There are no column/row coordinates. **Almost the same shape as our tokens (4 corners + 4 directions)**, and tmux has four more edges. | same place | supported (main checked) | likely |
| 3 | tmux `display-panes` draws a number on each pane and lets you pick with the 0~9 keys or a click (a way where a person looks and picks · up to 10 panes). | same place | supported (main checked) | likely |
| 4 | ace-window (Emacs) draws a letter on each window and assigns them in **top→bottom, left→right order** — if you remember the layout, you can pick without looking at the letters. The closest precedent for assigning a deterministic positional ordinal to an irregular layout. | [ace-window](https://github.com/abo-abo/ace-window) | README sentence supported (main checked) · ★the doc review (2026-10-26) pointed out that in the source `aw-window<` compares the left edge before the top (= column-first) — not checked by main★ | uncertain |
| 5 | i3 addresses by directional move (「그 방향의 가장 가까운 컨테이너」) · marks (`mark`) · criteria, and has no ordinal. | [i3 user guide](https://i3wm.org/docs/userguide.html) | collector summary | likely |
| 6 | WezTerm `PaneSelect` draws 1–2 letters on each pane and a person types them (including move · swap modes). A script cannot use it unless it knows which letter is which pane. | [WezTerm PaneSelect](https://wezterm.org/config/lua/keyassignment/PaneSelect.html) | collector summary | likely |
| 7 | The 「N 번째 열 / N 번째 편집기 그룹」 of niri · VS Code is **unconfirmed** (the collector's memory · the niri key-binding wiki main saw had only directional actions such as `focus-column-left/right`). niri is not a split tree but a 「열의 줄」 model, so column numbers are well defined (inference). | — | unconfirmed | uncertain |

- **Which of several neighbors to choose (the tie rule for directional moves) was in none of the tmux · i3 · WezTerm docs** (we did not read the source). We fixed it as 「가장 많이 맞닿은 칸」 (`neighbor_in_dir` in `src-tauri/src/layout/spatial.rs`).

## If we carry this to ours (inference · not a decision)

- **Column/row coordinates have no precedent, and we found no one who solved the problem of the 「행」 being ambiguous in a tree.** Other tools fill that gap with an ordinal + an on-screen display.
- Options that come out of the precedents:
  1. **A reading-order ordinal** (ace-window) — top→bottom, left→right. Our code already has it (the center-point sort `ordinal` in `spatial.rs` · it does not accept words). The header comment of that module notes that in an asymmetric split, a 「한 열」 can be broken up.
  2. **A person and the LLM use the on-screen number together** (tmux `display-panes`) — the same shape as the window number marker (ADR-0300).
  3. **Add four edge tokens** (tmux `{top}` · `{left}` …) — widens expressiveness cheaply.
  4. If it is still unknown, query — return the current ordinals and let the LLM choose.

## Gaps

- niri · VS Code · Hyprland · Zellij · PaperWM unconfirmed.
- The tie rule for directional moves — not in the docs, source not read.
- Whether the tmux `display-panes` number equals the pane number — source text not confirmed.
