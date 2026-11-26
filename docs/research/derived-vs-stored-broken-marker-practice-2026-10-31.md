# Broken-reference marker — compute on view, or compute on change and store (practice survey · light)

- **Status:** light · 2026-10-31 · collector 1 (worker-scout) · 2 main grounding spot-checks (Foam `graph.ts` incremental-processing comment · `document-decorator.ts` `resolveLink` at draw time — both supported by the source text). No adversarial review (light).
- **Trigger:** S25 — the user asked 「폴더를 이리저리 옮기는데 매번 저 검사를 한다고?」 (2026-10-31) about the design (P8) that computes the 「찾을 수 없음」 of the visible presets from the in-memory index each time the user moves from one library folder to another. The immediately preceding survey = `broken-reference-check-timing-practice-2026-10-31.md` (when to look at the disk).
- **Confidence legend:** certain = primary source (source code) checked · likely = single source · uncertain = estimate · no source.

## Conclusion

- **A split shape is common**, neither pure 「볼 때 계산」 nor pure 「항목마다 깨짐 표시를 저장」:
  - **What is stored = the index of relations between files** (who points at whom · backlinks · a map of unresolved links) — updated **incrementally** on add, change and delete (a full rebuild is for the first time and for emergencies).
  - **The marker beside an item = looked up from that index and computed on view** — no separate 「깨짐」 field is kept.
- **Computing from the in-memory index on every view is normal and treated as cheap** (Foam = `resolveLink` for each link visible on screen at draw time · Unity Inspector = resolves the reference at draw time and shows 「Missing」 if it is gone).
- **What is avoided is rescanning or rebuilding everything on each change** (the Foam comment states this explicitly).
- A shape that stores the marker per item appears **where deciding needs disk I/O** (Nautilus — when reading a directory, and when a watcher reports, it follows the link and keeps the result in the file info). It does not appear where an in-memory lookup is enough.

→ Applied to S25: the current design (at first scan, build the relation tables ① file list + references · ② reverse table, and fix them incrementally on Engram changes · when a folder is viewed, look up only the visible presets in that table and show them) is **the same shape as Foam**.

## Per tool

| Tool | What it stores | When it shows | Confidence · source |
|---|---|---|---|
| Foam (VS Code note extension · open source) | a graph — links · backlinks · `placeholders` (unresolved links) Map. Updated incrementally on each add, change, delete (delete = only re-resolve the ones that pointed at that note). A full rebuild = first time and emergencies | editor decoration = when the active editor changes · when the text changes (deferred), `resolveLink` → `isPlaceholder()` for each link on screen | certain (main checked the source text — `packages/foam-core/src/model/graph.ts` 21 · 87–104 · `packages/foam-vscode/src/vscode/features/editing/document-decorator.ts` 45–46 · 110–114) |
| Obsidian | the `resolvedLinks` and `unresolvedLinks` map of `MetadataCache` — updated whenever a file changes (`resolve` · `resolved` events) | how the editor decides the 「풀리지 않음」 appearance = unknown because the source is closed | map = likely (https://docs.obsidian.md/Reference/TypeScript+API/MetadataCache) · drawing = uncertain |
| VS Code · language servers | diagnostics (「모듈을 찾을 수 없음」) — push (`publishDiagnostics`: recheck on open and on change and send → the client stores) · LSP 3.17 pull (`textDocument/diagnostic` — only documents the client can see · the client chooses when to compute) | draws the stored diagnostics · trend = lazy for what is visible + invalidation | likely |
| GNOME Files (Nautilus) | per-file info (the kind after following the link) — when reading a directory · when a watcher reports | 「깨진 심볼릭 링크」 = comes straight from the stored kind (no stat again at draw time) | certain (`src/nautilus-file.c` about 6921 · 2453 — checked by the collector) |
| Unity Inspector | serialized reference id | at draw time, if the id points at nothing, 「Missing」 — no marker stored separately | likely (issue tracker · community) |
| Unreal Asset Registry · Explorer · Finder | — | — | not surveyed · uncertain |

## Gaps

- The drawing side of Obsidian is closed, so it is unconfirmed.
- Unreal · Windows Explorer (.lnk) · Finder (alias) not surveyed.
- Logseq · Dendron · Zotero not surveyed.
