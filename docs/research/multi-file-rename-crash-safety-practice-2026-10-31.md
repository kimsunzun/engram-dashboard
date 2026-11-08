# Crash-safety practice for renames spanning multiple files (2026-10-31)

- **Status:** `/research` light — collector 1 · main grounding spot-check (read the Unreal redirector official document and the Obsidian forum source text directly) · no adversarial review (light). Awaiting decision — whether to keep the S25 file-operation journal (`library-ops\` · flow ④ · finish at boot).
- **Background:** Current design = plan → write the journal → rename the file → fix the preset files that reference it → fix the agent profiles → delete the journal · if the process dies midway, the next boot finishes the remaining steps using the journal. The user's doubt (2026-10-31): 「이거 다른 데서도 안 할 것 같은데」.
- **Confidence legend:** certain = primary document · source text checked directly · likely = summary · user report · uncertain = community summary · unknown = not found.

## Conclusion

1. **No tool was found that protects a multi-file rename with a journal (intent record).** The user's guess is right (it is 「못 찾았다」, not 「없다」).
2. **Practice splits three ways.**
   - **No protection** — if interrupted midway, the references stay half-fixed and the user fixes them by hand (Obsidian · Lightroom folder move).
   - **Avoided by design** — references are not rewritten (Unreal redirectors · Unity GUID).
   - **Applied at once in memory + undo** — VS Code text edit batch · JetBrains undo · Local History. This is not crash recovery of disk state.
3. There is a report that for Obsidian, renaming a note with 600 links takes over 20 seconds, and if you rename again or close the app in between, links pointing to the old name, the new name and the second new name are left mixed (certain — forum source text).

## Per peer

| Peer | Method | If it dies midway | Confidence |
|---|---|---|---|
| **Obsidian** | renames, then fixes links in turn · no mention of a transaction · journal | links left mixed · the user's job | certain (forum report source text) · implementation document not found |
| **VS Code** | a `WorkspaceEdit` holding only text edits is all or nothing · if file creation · deletion · rename are mixed, they are applied in order and stop at the place that failed, and the earlier ones are not undone | protection against process death not found | likely (summary of an API documentation republication) |
| **JetBrains** | undo · Local History · on conflict, undo everything and show a preview | unknown | uncertain |
| **Unreal** | leaves a redirector at the old path so references are not rewritten right away · later 「Fix Up Redirectors」 resaves the referencing packages and deletes the redirector when all are done | thanks to the redirector, references do not break even in an intermediate state · rewriting can be postponed and the work can be redone | certain (official documentation source text) |
| **Unity** | the reference is the GUID in `.meta`, not the path — no need to rewrite referencing files on rename | not applicable | uncertain (general knowledge · not verified this time) |
| **Lightroom Classic** | the catalog is SQLite · no evidence that disk operations and catalog updates are protected together | a report that some subfolders disappeared from both sides while moving a folder inside the tool · after moving in Explorer, recommends 「Find Missing Folder」 | likely (user report) |

Sources:
- Obsidian — https://forum.obsidian.md/t/obsidian-note-renaming-bug-causing-broken-links-and-delayed-updates/59355 (main checked directly 2026-10-31: 「renaming of moc takes over 20 seconds to complete」 · 「the structure is broken, leaving links point to moc, moc-new-name, and moc-second-new-name」) · https://forum.obsidian.md/t/internal-links-not-updating-after-renaming-files-on-ipad-no-sync-no-plugins/101595
- VS Code — https://vscode-api.js.org/classes/vscode.WorkspaceEdit.html
- JetBrains — https://intellij-support.jetbrains.com/hc/en-us/community/posts/4408685170834/comments/4408703669650
- Unreal — https://dev.epicgames.com/documentation/en-us/unreal-engine/asset-redirectors-in-unreal-engine (main checked directly 2026-10-31: 「When you move or rename an asset … it leaves a Redirector in the asset's old location」 · 「This will resave all packages that point to the redirector, and will delete the redirector if it was able to resave all things referencing it」)
- Lightroom — https://community.adobe.com/questions-675/move-folder-from-1-ssd-to-another-within-lrc-2-of-3-subfolders-disappeared-984126

## For us

| | (가) keep the journal | (나) no journal — only decide the order (Obsidian shape) | (다) redirector (Unreal shape) |
|---|---|---|---|
| Precedent | not found | Obsidian · Lightroom | Unreal |
| After a crash | finish at the next boot | the lines pointing to the old path remain as 「찾을 수 없음」 → **a single relink fixes every preset · agent pointing to that old path** (R19 (a)) | the guide file at the old path keeps the references connected |
| Cost | a journal module · rerun at boot · steps designed to give the same result when done twice | 0 — recovers with the existing relink | guide files appear in the user's folder (visible in Explorer · git) · later cleanup work |

## Gaps · what could not be confirmed

- The implementation sources of Obsidian · VS Code · JetBrains were not read.
- The Unity GUID item was not verified this time.
- 「여러 단계 파일 작업」 tools such as git · package managers · Dropbox were not covered.
