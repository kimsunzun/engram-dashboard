# ★ A long 「참조」 (references) list in the detail card — display practice — light survey

| Item | Value |
|---|---|
| Status | survey complete · awaiting decision (user) |
| Date | 2026-10-17 |
| Method | `/research` light — 1 collector (about 10 searches · no fetches) · 1 main grounding spot check (WebFetch of the NSW design system 「Show more」) · no adversarial review |
| Question | When the 「참조」 (references) in an item's detail card (the presets · agents that reference that item — name links · for indirect ones 「(<프리셋> 통해)」 (via <preset>)) grows past 20 — does the design of a group header · only 5 · expanding in place with 「N개 더 보기」 (show N more) match practice |

## Conclusions

- **The count is always shown up front, and a long list is sent to a separate full view** — GitHub 「Used by N」 → the dependents page · the npm Dependents tab · the Unreal Reference Viewer (a separate window · copy the list · create a collection) · Unity (the results come out filtered in another window). (likely — search-summary level)
- **The only in-place example is the Obsidian backlinks — the unit is not 「N개 더」 (N more) but a group folded and unfolded per file** (count shown). (likely — changelog)
- **Design systems discourage expanding a truncated list with 「더 보기」 (show more)** — NSW: do not use it for a 「truncated list reveal」; use an accordion (several titled groups) · load more · pagination. (close to certain — main checked the source text · single source) · Siemens iX · Oracle = only the few important ones are shown and 「모두 보기」 (show all) is a new screen. (likely)
- **No place was seen that shows an indirect reference inline** — Unreal · Unity separate them by graph depth or a separate 「의존」 (dependency) action. Our 「(… 통해)」 (via …) has no precedent. (uncertain — not found ≠ does not exist)

## Findings

| Tool | Grouping | Count | First few | More → | Indirect shown | Source | Confidence |
|---|---|---|---|---|---|---|---|
| GitHub Used by | no (side-column avatars) · on the dependents page, filter repositories/packages | 「Used by N」 | a few avatars | a separate Dependents page (paginated) | no | https://docs.github.com/en/code-security/how-tos/secure-your-supply-chain/secure-your-dependencies/exploring-the-dependencies-of-a-repository | likely |
| Unreal Reference Viewer | graph (referencing side · depending side) | not found | — | a separate window · copy the list · create a collection · Asset Audit CSV | graph depth | https://dev.epicgames.com/documentation/en-us/unreal-engine/reference-viewer-in-unreal-engine | likely |
| Unity | no · a right-click action filters the Hierarchy/Project | not found | — | results in another window | no | https://docs.unity3d.com/Manual/search-ref-prop.html | likely |
| Obsidian backlinks | per file · fold and unfold | total count | all shown · the user folds per file | fold and unfold in place | no (unlinked mentions are a separate section) | https://obsidian.md/changelog/2020-08-12-desktop-v0.8.4/ | certain (changelog) |
| VS Code references | per file | 「N references」 CodeLens | — | in-editor peek list · references view | no | — | uncertain |
| Confluence (third-party app) | tab | 「8 links」 | count only | popup | no | https://marketplace.atlassian.com/apps/1223325/incoming-links-for-confluence | likely (not core) |
| npm Dependents | no | count on the tab | — | a separate tab · pages | direct only | — | uncertain |
| NSW design system | — | — | — | 「Show more」 is only for one short chunk of supplementary text · not used for expanding a truncated list or for load more → accordion · load more · pagination | — | https://designsystem.nsw.gov.au/components/show-more/ (main checked) | close to certain |
| Notion | — | — | — | — | — | — | abstain |

## What it gives our design

- The 「5개 + N개 더 보기」 (5 + show N more) in-place expansion is out of line with practice (the form NSW discourages).
- Two forms that fit the precedents: ① the Obsidian form — **fold and unfold group headers with counts**, like 「프리셋 3」 (presets 3) · 「에이전트 17」 (agents 17) (accordion) ② the GitHub · Unreal form — the card shows the count · a few items, and 「모두 보기」 goes to a **separate full view**.

## Gaps

- For no tool was it confirmed how many are shown first · no Notion material · no official source for VS Code references view · npm · the Unreal asset detail's Referencers unchecked.
