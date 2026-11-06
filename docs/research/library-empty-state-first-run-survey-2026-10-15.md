# When no library folder has been chosen yet — empty-screen guidance (practice summary)

- **Status:** light · awaiting decision (an earlier idea before the preset PRD — user decision 2026-10-15: the library slot appears in its usual form even at first, only the tree is empty, and guidance is given so the user can notice it by clicking)
- **Date:** 2026-10-15
- **Method:** research light. Collector 1 (4 searches · 2 fetches) · main grounding (Obsidian source text) · no adversarial review.

## Conclusion

1. **Placement = inside that pane's empty content area (not a strip).** The VS Code explorer shows a 「welcome view」 inside the empty explorer — one sentence + 「Open Folder」 · 「Open Recent」. Source string (source · via a summarizer): 「You have not yet opened a folder.」. **Likely.** Obsidian · Lightroom use a blocking chooser window.
2. **Buttons = 「기존 폴더 열기」 (open an existing folder) + (if the app can create the repository) 「새로 만들기」 (create new).** Obsidian vault management: 「Create new vault」 (name · Browse · Create) · 「Open folder as vault」 (Open) — **likely** (main checked the source text). Lightroom: create a new catalog · choose another catalog (community — uncertain).
3. **Only apps that 「소유하는」 (own) the repository suggest a default path.** The Calibre first-run wizard proposes 「Calibre Library」 under the home folder (forum — uncertain). VS Code, which only displays folders, does not suggest one.
4. **The practice when the designated folder has disappeared was not confirmed.** Lightroom shows a catalog chooser window when it cannot open the last catalog (community — uncertain). **Gap.**

## If moved to Engram (candidates)

- VS Code-style guidance inside the empty tree pane — one sentence + [open folder] (existing folder) + [create new] (create it at a pre-filled location, `library\` inside the distribution — the Obsidian 「Create new vault」 · Calibre default-path form).
- If the folder disappears, 「…를 찾을 수 없습니다」 in the same place + the same buttons — practice not confirmed (session proposal).

## grounding

| Claim | Source | Verdict |
|---|---|---|
| Obsidian Create new vault (Create · Vault name · Browse) · Open folder as vault (Open) | https://obsidian.md/help/manage-vaults (main · source text quoted via a summarizer) | Supported |
| VS Code 「You have not yet opened a folder.」 · Open Folder · Open Recent | https://raw.githubusercontent.com/microsoft/vscode/main/src/vs/workbench/contrib/files/browser/explorerViewlet.ts (collector · via a summarizer) | Unconfirmed (main did not cross-check) |
| Calibre default 「Calibre Library」 · Lightroom chooser window | Forum · community | Uncertain |
