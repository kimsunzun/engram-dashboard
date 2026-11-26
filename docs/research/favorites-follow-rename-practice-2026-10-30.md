# How favorites · collections follow renames · moves — practice (2026-10-30)

- **Status:** `/research` light — 1 collector · main grounding spot check (VS Code Bookmarks maintainer's answer confirmed directly · the Blender catalog docs were opened directly but only the table of contents came back, so unconfirmed) · no adversarial review (light). Decision pending — whether to have the S25 move record (`library-moves.json` · M10).
- **Use:** to make the collection the shell stores follow moves · renames that happen inside the daemon, does the current design fit practice, where the daemon keeps a 「옛 경로 → 새 경로」 record (the latest 1,000 entries · numbered) and the shell pulls 「N 번 뒤로 바뀐 것」.
- **Classification:** (A) fixed id — refer by an id that does not change when the name · location changes · (B) the side that changes fixes the references directly · (C) a move record · change feed that the other side follows by reading · (D) it breaks and the user relinks · (E) other.

## Conclusion

1. **The practice is (A) a fixed id or (B) the changing side fixing directly.** If the OS or catalog gives an id, refer by it (Lightroom · Eagle · Blender · macOS bookmarks · Windows shortcuts), and if the reference is in the hands of the changing side, fix it on the spot.
2. **We found no peer that uses (C) 「상한 있는 이동 기록을 다른 저장소가 당겨 읽기」 (a bounded move record that another store pulls and reads).** The closest is the VS Code Bookmarks extension receiving a rename notification inside the same process and fixing it (a push · not a record).
3. **Changes made outside the app (Explorer) are mostly not followed anywhere** — the exceptions are only the OS level that uses file system ids (macOS bookmarks · NTFS link tracking), and the rest fall to (D) relinking.
4. So the current design (the shell holds the collection and the daemon keeps the move record) is an **uncommon shape**. There is one condition that justifies it — 「참조를 들고 있는 쪽(셸)이 바꾸는 쪽(데몬)과 떨어져 있고, 평범한 파일이라 고정 id 가 없다」 (the side holding the references (the shell) is separate from the side that changes them (the daemon), and since they are ordinary files there is no fixed id). If the references are moved to the daemon side it becomes (B) and no record is needed.

## By peer

| Peer | Method | If changed outside the app | Confidence |
|---|---|---|---|
| **Lightroom Classic collections** | (A) — a collection points to the catalog's photo record, not a path · the catalog holds the path separately and fixes it on an in-app move | 「없음」 (none) mark → 「Find Missing Folder」 · if even the file name changed, relink one by one (D) | likely |
| **Eagle** | (A) — a `<ITEM_ID>.info` folder per item · identity is the id, not the path · folder membership is metadata | Unknown | likely |
| **Blender asset catalogs** | (A) — an asset holds a catalog UUID · renaming · moving a catalog fixes only one file, `blender_assets.cats.txt` (`{UUID}:{경로}:{이름}`) | A catalog is a text-file edit, so safe · the asset file path itself is not tracked | likely (the collector confirmed via a translated doc · main's direct check failed) |
| **macOS bookmark data · aliases** | (A) + fallback — finds the file after a move · rename by file system id · if told it is stale, the app saves it again | Follows (the id is in the file system) | certain (bookmark data) |
| **Windows `.lnk` + distributed link tracking** | (A) + guess search — NTFS object id + volume id · on failure, searches in several steps by old path · name · creation time · size | Follows if NTFS | certain |
| **VS Code Bookmarks extension** | (E) in-process notification — if done by VS Code's rename, it fixes automatically | 「셸 · 다른 도구로 바꾸는 것은 지원 안 함」 (changing it from a shell · another tool is not supported) | certain (maintainer's answer — main confirmed directly) |
| **Obsidian bookmarks** | Probably (B) — path in `.obsidian/bookmarks.json` · could not find docs on whether it fixes on rename | Unknown | uncertain |
| **JetBrains bookmarks** | Probably (B) — file URL in `workspace/<id>.xml` · could not find rename behavior | Unknown | uncertain |

Sources:
- Lightroom — https://blog.nobledesktop.com/learn/lightroom/relinking-missing-files-folders · https://lightroomkillertips.com/when-photos-go-missing/
- Eagle — https://api.eagle.cool/llms-full.txt
- Blender — https://docs.blender.org/manual/en/latest/files/asset_libraries/catalogs.html
- macOS — https://developer.apple.com/documentation/foundation/nsurl/bookmarkdata(options:includingresourcevaluesforkeys:relativeto:).md · https://indiestack.com/?p=514
- Windows — https://learn.microsoft.com/en-ie/Windows/Win32/fileio/distributed-link-tracking-and-object-identifiers · https://msdn.microsoft.com/it-it/windows/desktop/bb774952
- VS Code Bookmarks — https://github.com/alefragnani/vscode-bookmarks/discussions/593 (main confirmed directly 2026-10-30: 「If you rename the file inside VS Code … the bookmarks will be automatically updated」 · 「Renaming the file outside VS Code, shell or other tools/commands, isn't supported」)
- Obsidian — https://forum.obsidian.md/t/new-bookmarks-should-overwrite-old-bookmarks-if-they-have-the-same-name/109648 (file location only)
- JetBrains — https://intellij-support.jetbrains.com/hc/zh-cn/community/posts/360009712499/comments/360002241419 (storage shape only)

## For us

| | (가) Now — the shell holds collections · the daemon keeps the move record (C) | (나) The daemon holds the collection (B) | (다) The daemon issues fixed ids (A) |
|---|---|---|---|
| Precedent | Not found | The same shape as the daemon fixing presets · agents · practice | Lightroom · Eagle shape |
| Move record | Needed (latest 1,000 · on overflow 「찾을 수 없음」 (not found)) | Not needed | Not needed — instead the daemon persists an id → path table |
| LLM | Collection commands reach it only while the shell is attached | They are daemon commands, so they reach it even without the UI (G20 · G22 「LLM 이 기본」 (the LLM is the default)) | Depends on where the collection lives |
| Changes outside the app | Not followed | Not followed | Not followed (ordinary files have no id) |

## Gaps · what could not be confirmed

- Rename behavior of Obsidian · JetBrains bookmarks — not confirmed.
- Eagle's moves outside the app · inside smart folders — not found.
- Windows Quick Access pinning · the actual mechanism of the Finder sidebar — not checked.
- A precedent of a sync · catalog tool that hands out 「커서 N 뒤로 바뀐 것」 (changes since cursor N) with a cap — not found (git's rename detection is not a record but after-the-fact similarity judgment — knowledge-based, not searched).
