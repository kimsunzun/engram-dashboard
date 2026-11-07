# ★ Viewing the content of a referenced item without leaving the edit pane — light survey

| Item | Value |
|---|---|
| Status | survey done · decision pending (user) |
| Date | 2026-10-17 |
| Method | `/research` light — 1 collector (about 10 searches · fetches) · 2 main-session grounding spot checks (VS Code docs · Obsidian docs via WebFetch) · no adversarial review |
| Question | When a preset is chosen, the right side becomes the edit pane. To keep seeing the content of the `.md` that an instruction line inside it points to, without leaving the edit pane — what do other tools do (user idea: on click, a tooltip to the left instead of the right) |

## Conclusions

- **The most common form = a preview that attaches and appears briefly (hover · peek)** — it shows the referenced content beside that spot (height cap · scroll), and 「계속 보기」 (keep viewing) is a separate extra step (pin · open in a real editor · to a tool window). (likely)
- **How it is raised:** light preview = mouse hover (noisy while editing, so Ctrl/Cmd + hover — Obsidian edit view · VS Code) · heavy preview = a key (VS Code Alt+F12 · JetBrains Ctrl+Q · macOS Quick Look Space). (certain — Obsidian · VS Code primary sources checked)
- **「다른 줄을 눌러도 열린 채로 내용이 바뀐다」 was seen only in windows that follow the selection** — macOS Quick Look (the floating window changes with the selection) · the JetBrains documentation tool window (tab pinning). (likely)
- **Placement:** attached right beside the raised row (VS Code embeds it in the editor — pushing content down), or a separately floating window (Quick Look). We saw no precedent that raises it on 「늘 정해진 한쪽(목록 위)」 (uncertain — not found ≠ none). However, our edit pane is at the right edge of the screen, so **if attached beside, it flips to the left for lack of space** — this is how an ordinary attached popup behaves (inference).
- ★ The collector said 「줄 아래 펼치기는 어디에도 없다」, but **VS Code Peek is exactly that form** — 「we embed the result inline」 (primary source checked). That summary sentence is downgraded.

## Findings

| Tool | Raise | What | Placement | Length · scroll | Pin · follow | Close | Source | Confidence |
|---|---|---|---|---|---|---|---|---|
| Obsidian Page preview | file explorer · search · backlinks = hover · edit view = Ctrl/Cmd + hover · a setting always requires the modifier key | floating preview | beside the link (no direction documented) | not documented | core = not documented · community Hover Editor = pin · drag · resize · several | not documented | https://obsidian.md/help/plugins/page-preview (main checked) · https://community.obsidian.md/plugins/obsidian-hover-editor | raising = certain · the rest = uncertain |
| VS Code Peek | Alt+F12 · Shift+F12 · right-click | **a peek window embedded in the editor** (pushes the text below down) | below that line | a small editable editor + result list | stays open while moving between references · `editor.stablePeek` = does not close by itself · clicking the file name · double-click = to the outer editor | Esc · double-click | https://code.visualstudio.com/docs/editing/editingevolved (main checked) | close to certain (a single primary source, so the tag is likely) |
| VS Code hover | hover · Ctrl + hover = declaration preview | card | beside the symbol | unconfirmed | unconfirmed | unconfirmed | same | raising = likely |
| JetBrains Quick Documentation | hover (can be turned off) · Ctrl+Q | popup or documentation tool window | beside the element | unconfirmed | to the tool window · pin the documentation tab | not documented | https://www.jetbrains.com/help/idea/viewing-reference-information.html | likely |
| macOS Quick Look | Space · Cmd+Y on the selected item | separate floating window (always on top) | free | full · resizable | **changes with the selection** | Space · Esc | https://macmost.com/how-to-use-mac-quick-look-in-5-minutes.html | likely (secondary source) |
| Unreal Content Browser | hover | info + thumbnail tooltip | beside the cursor · a defect where it overflows and does not appear at the bottom-right edge of the screen | not content | none | move the mouse away | https://issues.unrealengine.com/issue/UE-328263 | likely |
| Unity Inspector reference field | click = flashes that asset in Project · double-click = open | no preview | — | — | — | — | https://docs.unity.cn/6000.0/Documentation/Manual/UsingTheInspector.html | likely |
| Notion · Figma | — | — | — | — | — | — | — | abstained |

## grounding

- VS Code Peek — primary source: 「we embed the result inline」 · edit while moving between references with it open · closes with Esc · double-click, and `editor.stablePeek` turns that off · clicking the file name · double-click = the outer editor. **Supported.**
- Obsidian — primary source: hover (explorer · search · backlinks) · the edit view uses Ctrl/Cmd + hover · a setting that requires the modifier key. Pin · scroll · close are not in the docs. **Supported (raising only).**

## What this gives our design

- For viewing content from an edit pane instruction line, the combination of precedents is **a preview attached beside the line + a form where, once 「고정」 (pinned), the content changes even if another line is chosen** (Quick Look's selection following). Because the edit pane is at the right edge of the screen, attaching beside it **opens to the left (above the list)** — the same place as the user's idea.
- The way to raise it is kept separate so it does not overlap with the click that chooses a line (click = choosing the slot to insert into · R46) — a preview button on the line · Space.

## Gaps

- Not confirmed: scroll · close · direction of the Obsidian core preview · pinning of the VS Code hover card · JetBrains Ctrl+Shift+I · Notion · Figma.
- The flip at the screen edge was seen in only one Unreal defect.
