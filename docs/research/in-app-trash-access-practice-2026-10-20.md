# Where to open the in-app trash · restoring · emptying (2026-10-20)

| Item | Value |
|---|---|
| **Status** | Research complete · **decided 2026-10-20** — a fixed row 「🗑 휴지통」 at the bottom of the left pane (Notion style · not a 「…」 menu) → view the trash · restore to the original path · no automatic emptying (user: 「그렇게 하고」 · 「일단 유저가 비우게 나중에 필요하면 추가하고」 — PRD R18 · TRD §6-5) |
| **Method** | `/research` light — collector 1 (search snippets only · no direct reading of source texts) + main grounding (checked the Notion · Apple Photos help source texts) |
| **Question** | For an app-managed trash (not the OS trash): ① where to open it (fixed in the sidebar · a menu · none) ② where to restore · name collisions ③ emptying · confirming permanent deletion ④ automatic deletion |

## Conclusion

1. **A fixed sidebar item is the convention** (likely — two source texts checked): Notion 「You'll see Trash at the bottom of your sidebar」 · Apple Photos 「Click Recently Deleted in the sidebar」 · Google Drive Trash under the left nav · Apple Notes 「최근 삭제된 항목」 in the folder list · Joplin 3.0 Trash in the notebook list. **None of the apps surveyed put it only inside a menu.** Exceptions = Notion mobile (inside the sidebar `•••` menu — because of space) · Obsidian (no core screen — opened through the explorer or the Trash Explorer plugin · an inconvenience case).
2. **Restore = to the original place** (likely): the Obsidian plugin returns files to the original path and creates missing parent folders · the Windows Recycle Bin renames on a name collision and restores (snippets only).
3. **Automatic deletion after 30 days is common** (likely — Notion · Apple Photos source text 「30 days」 · Drive · Notes from snippets). Emptying · permanent deletion goes through a confirmation (Drive 「Delete forever」).

## Cases

| Case | Where it opens | Restore | Empty · permanent delete | Auto delete | Source | Confidence |
|---|---|---|---|---|---|---|
| Notion | Trash at the very bottom of the sidebar (mobile = `•••` menu) | Open it and restore · search · filter | Open it and permanently delete | 30 days | https://www.notion.com/help/duplicate-delete-and-restore-content | Certain (main checked the source text) |
| Apple Photos | Recently Deleted in the sidebar (15.3 and below = inside Utilities) | Select and Recover | — | 30 days | https://support.apple.com/en-us/guide/photos/delete-photos-videos-recover-deleted-pht3bfab2f96/10.0/mac/15.0 | Certain (main checked the source text) |
| Apple Notes | 「최근 삭제된 항목」 in the folder list | Recover | — | 30 days | https://support.apple.com/en-us/guide/icloud/mm2f42f05cb9 | Likely (snippet) |
| Google Drive | Trash under the left nav | — | Empty trash → confirm 「Delete forever」 | 30 days | News article (official docs not checked) | Likely |
| Joplin 3.0+ | Trash in the notebook list | Unknown | Right-click Empty trash | N-day option | https://discourse.joplinapp.org/t/trash-can/34057 | Likely |
| Obsidian | No core screen (`.trash` through the explorer · plugin) | Plugin = original path · creates missing folders | Plugin | No core feature | https://community.obsidian.md/plugins/obsidian-trash-explorer | Likely (snippet) |
| Windows Recycle Bin | Fixed on the desktop · in the explorer | Original place · renames on collision | Confirm on emptying | Capacity-based (unverified) | Snippet | Likely |

## Gaps

- OneDrive · Dropbox · Figma · Logseq · Bear · Evernote · VS Code · Unity · Unreal · JetBrains Local History · macOS 「Put Back」 were not looked at.
- What to do when restoring a folder whose original parent folder is gone — not confirmed anywhere except the Obsidian plugin (creates it anew).
- Engram context: emptying is by a human only (R18 · ADR candidate 28). Whether the app emptying itself after 30 days fits that decision is a separate matter to decide.
