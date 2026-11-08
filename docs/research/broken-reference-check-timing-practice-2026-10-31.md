# When is the broken-reference marker computed — on view, on refresh, or on use (practice survey · light)

- **Status:** light · 2026-10-31 · collector 1 (worker-scout-sonnet · 4 searches · mostly based on search summaries) · 2 main grounding spot-checks (Blender = supported · Obsidian update timing = not in that document → downgraded to uncertain). No adversarial review (light). **The evidence is thin** — almost no primary source states the timing in a sentence.
- **Trigger:** S25 — the user asked 「F5 나 실행할 때만 하면 되지 않나」 (2026-10-31) about the design (P8) that computes the 「찾을 수 없음」 of the library list · tree ⛔ every time a folder is viewed (against the in-memory list).
- **Confidence legend:** certain = primary source checked · likely = single source · uncertain = estimate · no source.

## Conclusion

- All three approaches exist:
  1. **Shown immediately on view from an index (memory) + rechecked on use** — tools that hold a reference index (Obsidian's metadata cache · IDEs and the like, by guess — no primary source for the timing · uncertain).
  2. **Only on an explicit check command** — Blender 「Report Missing Files」 (certain · manual command) · Lightroom 「Find All Missing Photographs」 (the list updates only when that command is rerun — likely).
  3. **Surfaces as a failure on open or on use** — tools whose project file holds only path strings (Premiere 「Link Media」 = when the project is opened · Ableton = when the set is loaded — likely).
- 「새로 고침이나 쓸 때만」 (2+3) is **the common shape for file-based tools**. Tools that hold an index mostly use 1 (a guess).
- No sample confirmed a watcher for outside changes — Blender, Lightroom and Ableton use manual commands.
- No primary source states 「보이는 항목만 vs 전체」 (Lightroom's advice to expand folders only hints at 「보이는 것만」).

## Per tool

| Tool | Timing | Scope | Outside changes | Confidence · source |
|---|---|---|---|---|
| Blender | explicit command 「Report Missing Files」 (no warning means nothing is missing) · 「Find Missing Files」 (pick a folder, recursive search · recovers what it finds) · otherwise on load (estimate) | whole file | manual | command = certain (main checked the source text — https://docs.blender.org/manual/en/3.4/interface/window_system/topbar.html) · on load = uncertain |
| Lightroom Classic | thumbnail 「!」 badge (when it checks is unknown) · 「Missing Photographs」 list updates only when Find All Missing Photographs is rerun | unknown | no watcher · resync | update command = likely · timing = uncertain (https://helpx.adobe.com/au/lightroom-classic/desktop/manage-catalogs-and-files/locate-missing-photos.html) |
| Premiere | when the project is opened 「Link Media」 | whole project (scan on open — guessed from requests to turn it off) | unknown | likely (Premiere Elements docs — https://helpx.adobe.com/ca/premiere-elements/desktop/importing-and-adding-media/offline-files.html) |
| Ableton Live | missing/offline when the set is loaded · ad-hoc list via 「Manage Files › View Files」 | whole set | unknown | likely (search summary — https://help.ableton.com/hc/en-us/articles/209070829) |
| Obsidian | `unresolvedLinks` in the metadata cache (a map of the whole vault) | whole vault | unknown | cache exists = certain (https://docs.obsidian.md/Reference/TypeScript+API/MetadataCache/unresolvedLinks) · update timing = uncertain (not in that document — main checked) |

## Gaps

- Unity · Unreal · DaVinci Resolve · Zotero · IDEs were not examined.
- Almost no full pages were opened (based on search summaries).
- Not confirmed whether index-based tools follow outside changes with a watcher.
