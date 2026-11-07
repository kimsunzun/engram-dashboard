# ★ Where to put the text excerpt and the rendered preview — light investigation

| Item | Value |
|---|---|
| Status | Investigation complete · decision = the card shows only the preview · the excerpt only in one-line places (list · search results · add-search) (follows practice — user 2026-10-17 「관행 찾아봐」 (look up the practice)) |
| Date | 2026-10-17 |
| Method | `/research` light — one collector (about 8 searches · no fetch) · one main grounding spot check (Outlook help WebFetch) · no adversarial review |
| Question | If the instruction detail card shows both a text excerpt (2 lines …) and a rendered markdown preview, the same opening appears twice — where do other apps put the two |

## Conclusion

- **An excerpt (1-3 lines of plain text) is the secondary text of a list or result row, and the content is shown by a separate preview or reading pane.** Outlook = list row 「Message Preview」 of 1-3 lines + the whole message in the reading pane — source text: 「Message Preview, where you can choose to display between one and three lines of each message in the message list」 · 「When you select a message in the message list, you can see the message in the Reading Pane」. (likely — main checked the source text · single source)
- **No example was found that puts an excerpt and rendered content together in one pane** — not found ≠ does not exist. (uncertain)
- Explorer shows only one of the details pane (metadata) and the preview pane (content) · Material list 「supporting text」 = an optional secondary line (up to 1-3 lines). (likely — secondary summaries)

## Findings

| App | Excerpt place | Rendered content place | Both in one place? | Source | Confidence |
|---|---|---|---|---|---|
| Outlook | list row 1-3 lines (default 1) | reading pane | no evidence | https://support.microsoft.com/outlook/use-and-configure-the-reading-pane-to-preview-messages-in-outlook (main checked) | likely |
| Explorer | details pane = metadata only | preview pane | no — one at a time | https://learn.microsoft.com/en-us/answers/a/7363238 | likely (secondary) |
| Finder · Spotlight | result row = name + metadata | Quick Look (Space) | no evidence | https://sealsystems.net/go/preview-spotlight-results-in-ventura-using-quick-look/ | likely (secondary) |
| Obsidian | search result = a few lines around the match | community Quick Preview (hold Alt) — a separate action | no | https://community.obsidian.md/plugins/quick-preview | uncertain |
| Unreal global asset picker | no description line · on hover, info at the bottom of the window | thumbnail | no | https://docs.unrealengine.com/4.26/en-US/Basics/UI/GlobalAssetPicker | likely |
| Material list | supporting text 1-3 lines (optional) | not applicable | — | https://m2.material.io/go/design-lists | likely (search summary) |
| Apple Notes · Bear · Gmail · VS Code · Raycast · Notion · Unity · HIG · Fluent | — | — | — | — | abstained |

## What this gives our design

- Instruction detail card (right pane · edit pane 👁 card) = rendered preview only.
- Excerpt = only where a one-line secondary text is attached — the 「이름 · 한 줄」 (name · one line) of the preset 「+ 담기」 (+ add) search result row (skill = `SKILL.md` description · instruction = excerpt). The rules for making the excerpt stay exactly as in `docs/research/markdown-plaintext-snippet-survey-2026-10-17.md`.

## Gaps

- Only one primary page was fetched, Outlook — the rest are search summaries.
- No active search was made for a counterexample (both in one pane).
