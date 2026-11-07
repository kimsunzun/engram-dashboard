# ★ Plain-text excerpt (snippet) of a markdown body — length · truncation practice — light survey

| Item | Value |
|---|---|
| Status | survey complete · awaiting decision (user) |
| Date | 2026-10-17 |
| Method | `/research` light — 1 collector (about 11 searches · fetches · read the `remove-markdown` source) · 1 main grounding spot check (WebFetch of the `strip-markdown` readme) · no adversarial review |
| Question | When the detail card of an instruction (`.md` — usually no front matter) shows the beginning of the body as plain text (markdown symbols · whitespace cleaned up) instead of a description, what is done when a line is too long or the body starts with a heading · code · table · long URL |

## Conclusions

- **Do not cut by line · paragraph.** Flatten the beginning of the body into one stream (line breaks → spaces), **cut by character count or line count (1–2 lines)**, and if it overflows use an ellipsis (…) or a fade. (likely — on the app side, at the level of help pages · user posts)
- **Where there is a heading, use the first line (first heading) as the title and the excerpt starts after it** — Apple Notes (first line = title · second line in the list = date + one line of the following text) · Bear. (likely · Apple's source text was not fetched)
- **The full text is opened to view** — no basis for in-place expansion · hover tooltip was found.
- **Symbol-stripping libraries** — `strip-markdown` (remark): `code` · `html` · `horizontalRule` · `table` · `toml` · `yaml` are removed **with their content**, and the rest become plain-text paragraphs · images become their alt text. (certain — main checked the source text) · `remove-markdown` (regex): strips only symbols and keeps heading · link text · code content · no length cap · no whitespace cleanup. (certain — the collector read the source)

## Findings

| App | What it strips | First heading | Length | Overflow | Full view | Source | Confidence |
|---|---|---|---|---|---|---|---|
| Apple Notes | no docs | first line = title | one line | no docs | open | https://support.apple.com/en-asia/guide/notes/not9474646a9/4.5/mac | likely (search summary) |
| Bear | not found | not found | a preview-size setting · a user post saying it is joined into one line without line breaks | not found | open | https://community.bear.app/t/display-new-lines-in-note-preview-list-view/14423 | uncertain |
| Obsidian search | not found | — (around the match) | 1 line around the match · 「Show more context」 = up to 5 lines | not found | open | https://obsidian.md/changelog/2020-08-12-desktop-v0.8.4/ | likely |
| Gmail | no docs | — | API `snippet` = 「a short part of the message text」 (length not stated) | unchecked | open | https://developer.google.com/gmail/api/v1/reference/users/messages | field = certain · length = abstain |
| GitHub repository description | no markdown processing | — | input cap of 350 characters | unchecked | unchecked | https://slategit.com/blog/github-repo-description-character-limit-what-fits | likely |
| Notion gallery 「Page content」 | no docs | no docs · first block · an image if it is an image | first block only | not found | open | https://www.notion.com/help/galleries | likely (search summary of a help-page copy) |
| Linear | not found | not found | Peek = the whole description · list-row preview not found | not found | Peek · open | https://linear.app/docs/peek | uncertain |
| `remove-markdown` | list markers · heading markers (text kept) · links (text kept) · emphasis · inline code (content kept) · code fences (content kept) · HTML · quote `>` · horizontal rules · footnotes | kept as text | none | — | — | https://github.com/stiang/remove-markdown | certain (source) |
| `strip-markdown` | code · HTML · horizontal rules · tables · toml · yaml = with content · the rest = plain text | plain-text paragraph | none | — | — | https://github.com/remarkjs/strip-markdown (main checked) | certain |
| Outlook | — | — | — | — | — | — | abstain |

## Collector synthesis (design inference without sources — not a measurement of practice)

1. Use the AST rather than regex (the `strip-markdown` form) — remove code blocks · tables · HTML · horizontal rules · front matter, and keep link text · image alt · inline code content · list text.
2. If the first block is a heading, start from the text after it (if it is only a heading, use the heading text) · if nothing is left after stripping, leave it empty or use the file name.
3. Join blocks with a single space and collapse runs of whitespace into one · a bare URL as only the host or about 40 characters · cut at about 150–200 characters on a word boundary and add … — the character cap first, the CSS line clamp after that (1 line in a list row · 2–3 lines in a detail card).
4. Open to see the whole (or a `title` hover) — no in-place expansion.

## Gaps

- Outlook · Bear · Apple Notes source text · Linear list rows · Obsidian Notes Explorer · GitHub issue list were not looked at.
- No app stated its symbol-stripping rules in docs — most app rows are inferred from help pages · user posts.
- Whether overflow is handled with an ellipsis or a fade was in no source.
