# Cross-root move practice — move or copy, and do references follow (2026-10-30)

- **Status:** `/research` light — collector 1 · main grounding spot-check (read the Eagle support document directly) · no adversarial review (light). Awaiting decision — after G25 in the S25 decision memo, 「루트 사이 파일 옮기기」.
- **Use:** In the multi-root setup (G25) of the S25 asset library, when moving a file from root A to root B, decide between (가) copy only and (나) also allow move (references follow).
- **Confidence legend:** certain = primary document checked directly · likely = primary document summary · community post · unknown = not found.

## Conclusion

1. **Asset-management tools that keep several libraries only copy between libraries.** The Eagle support document says directly that 「다른 라이브러리에 추가」 supports copying only (certain — it gives no reason). For Blender too, the only path in the docs is 「Copy Asset Bundle」 (copy) (likely). The user deletes the original by hand.
2. **A catalog tool (Lightroom Classic) allows moves across drives, but references follow only when the move is made inside the tool.** If moved outside the tool (in Explorer), the reference is lost as 「?」 and reconnected with 「Find/Update Folder Location」 (likely).
3. **Windows Explorer defaults to move on the same drive and copy on a different drive.** Shift = force move · Ctrl = force copy (likely — checked via Microsoft blog and Q&A summaries; widely known behavior).
4. **No peer documents how it handles a cross-drive move that fails midway.** Only the general copy-then-delete approach is presumed.

## Per peer

| Peer | Cross-root move | Do references follow | Cross-drive failure handling |
|---|---|---|---|
| **Eagle** (several libraries) | copy only — 「Add to Other Library」 (certain) · delete the original by hand | if the item's unique id is the same, it is treated as the same file and not imported again · otherwise unknown | unknown |
| **EagleFiler** | drag and drop copies · move is planned (likely — forum) | unknown | unknown |
| **Blender asset library** | copy only per the docs — 「Copy Asset Bundle」 (likely) | unknown | unknown |
| **Lightroom Classic** | allowed — even across drives in the in-tool folder panel (likely) | follows if moved inside the tool · lost if moved outside, then reconnect (likely) | unknown |
| **Windows Explorer** | same drive = move · different drive = copy by default · changed with Shift/Ctrl (likely) | unknown | unknown |
| **VS Code multi-root** | only says moving between roots is possible · could not find whether the default is move or copy (uncertain) | depends on the language extension's rename handling · not checked | unknown |
| **Obsidian** | no built-in feature · community plugins (Trans Vault = both copy and move) (likely) | auto-updates only inside the vault · the plugin fixes references only when both ends are moved together (likely) | unknown |

Sources:
- Eagle — https://eagle.cool/support/article/why-is-there-no-move-option-when-adding-files-to-other-libraries (main checked directly 2026-10-30: 「currently only supports copying files and does not provide a move file option」) · https://en.eagle.cool/article/1304-adding-images-to-other-libraries
- EagleFiler — https://forum.c-command.com/t/moving-not-copying-items-between-libraries/11852
- Blender — https://docs.blender.org/manual/nb/latest/editors/asset_browser.html (read via search summary)
- Lightroom Classic — https://lightroomkillertips.com/?p=14231 · https://www.lightroomqueen.com/find-move-rename-catalog/ (not Adobe documentation)
- Windows Explorer — https://devblogs.microsoft.com/oldnewthing/?p=37323 · https://learn.microsoft.com/en-us/answers/a/2790390 (read via summary)
- VS Code — https://code.visualstudio.com/docs/editing/workspaces/multi-root-workspaces
- Obsidian — https://community.obsidian.md/plugins/transvault · https://community.obsidian.md/plugins/vault-transfer · https://forum.obsidian.md/t/moving-notes-from-one-vault-to-another/2040

## For us

| | (가) between roots = copy only | (나) allow move · references follow | (다) Explorer shape — same drive = move · different drive = copy · Shift = move |
|---|---|---|---|
| Peers | Eagle · Blender (direct peers — several libraries) | Lightroom (catalog) | Windows Explorer — P5 set move and conflict by Explorer's rules |
| Cost | 0 — copy (P6) as is | a cross-drive move = copy then delete via a journal · no precedent for failure handling | bears the whole cost of (나) (Shift opens up move) |
| References | if the original is deleted, 「찾을 수 없음 · 다시 연결」 — the user's job | follow | follow if it is a move; the original stays if it is a copy |

## Gaps · what could not be confirmed

- No peer documents handling of a mid-way failure in a cross-drive move.
- Adobe official documents, the full Blender asset browser document, and the VS Code issue tracker were not read. VS Code's default drag behavior was not measured.
- Adobe Bridge · Unity/Unreal content browsers · Steam 「설치 폴더 옮기기」 (a peer for cross-drive moves) were not covered.
