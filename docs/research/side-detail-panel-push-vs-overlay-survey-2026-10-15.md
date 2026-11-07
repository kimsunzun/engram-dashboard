# Side detail panel — overlay it, or insert a pane (practice summary)

- **Status:** light · awaiting decision (pre-preset-PRD idea — when pressing an instruction or skill row in the preset edit pane shows its detail in a side panel, how to make room without breaking out of the slot)
- **Date:** 2026-10-15
- **Method:** research light. One collector (4 searches · search-summary level — no source fetch) · main grounding (1 JetBrains source text) · no adversarial review (light).

## Conclusions

1. **The most common shape = a pinned pane (insert it and narrow the neighbor).** Windows Explorer preview pane and details pane, Outlook reading pane, Jira board detail panel, JetBrains 「Dock Pinned」. **Likely** (JetBrains = source text · the rest = summaries of community posts and issue trackers).
   - JetBrains source text: Dock Pinned = 「always visible along with the editor and other pinned tool windows」.
2. **Overlaying is used for a quick look.** JetBrains Undock (「covers a part of the editor … when active」) · Float (「floating on top of the main window」) · Notion side peek (whether it overlays or pushes aside is unchecked). **Likely** (JetBrains source text).
3. **Mature apps let the user choose.** Notion = Side peek · Center peek · Full page (「Open pages in」 per database) · JetBrains five modes · Outlook right · bottom · off. **Likely** (JetBrains source text · Notion · Outlook summaries).
4. **Width = a draggable border is the standard.** Remembering the width is confirmed only for Explorer (a registry value — community) · Jira maximum 1680px · double-click auto fit (issue tracker). No documented rule for minimum width or auto-collapse when narrow was found. **Uncertain.**
5. **A shape that inserts a pane and widens the outside (the container) is not in the sample.** **Gap.**

## Carried over to Engram (candidate — not a decision)

- **Insert** a detail pane inside the library slot **and narrow the other panes** (pinned) · drag the border to adjust the width · remember the width. Switching to an overlay inside the slot when the slot is too narrow is the **collector's inference**, not a peer precedent.

## grounding

| Claim | Source | Verdict |
|---|---|---|
| JetBrains five-mode wording | https://www.jetbrains.com/help/idea/viewing-modes.html (source text quoted via the summarizer) | supported |
| Notion 「Open pages in」 Side/Center/Full | https://www.notion.com/releases/2022-07-20 (search summary) | unchecked |
| Jira border drag · 1680px · double-click | https://jira.atlassian.com/browse/JRACLOUD-98347 · JRACLOUD-79715 · JSWCLOUD-8778 (search summary) | unchecked |
| Explorer width registry | https://www.elevenforum.com/t/reset-details-and-preview-pane-width-to-default-in-file-explorer-in-windows-11.9352/ (community) | unchecked |
| Outlook right · bottom · off | https://www.ablebits.com/office-addins-blog/outlook-reading-pane-change-show-hide/ (third party) | unchecked |

## Gaps

- Not looked at: Linear · Asana · GitHub Projects (official) · Finder · VS Code secondary side bar · Figma · Unreal · Obsidian.
