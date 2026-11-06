# Practice when deleting in a location where the recycle bin is unavailable (2026-10-19)

| Item | Value |
|---|---|
| **Status** | survey complete · **decided 2026-10-19** — always our own trash: deletion goes to a hidden `.trash` inside the library folder regardless of location · the OS recycle bin is not used · emptying is by a person only (no `engram` or bus command) (user 「ㅇㅇ 우리 trash 폴더로해」 (yes, use our trash folder) · 「휴지통으로 가되 휴지통 비우기를 llm으로 쏘지 못하게 막자는거지」 (it goes to the trash, but we block emptying the trash from being fired by an llm) — PRD R18). So conclusion 4 below (the silent permanent-delete risk of `IFileOperation`) no longer applies to our implementation |
| **Method** | `/research` light — 1 collector (6 searches · 6 pages) + main grounding (Obsidian settings — search result of a copy of the official help) |
| **Question** | On network drives · volumes without a recycle bin · recycle bin over capacity, does the app ① use an app-specific trash ② ask about permanent deletion ③ refuse |

## Conclusion

1. **The precedent of an app-specific trash = Obsidian** (likely): the setting 「Deleted files」 is chosen by the user in advance among three — system trash (default) · Obsidian trash (a hidden `.trash` folder inside the vault) · permanent delete. `.trash` is emptied separately from the system trash, and the main app has no auto-empty or restore screen, but there are community plugins (Empty Trash · Trash Explorer) (so we presume the main app has none). When the system trash fails, it does not switch to `.trash` by itself (forum — a case of permanent deletion on Linux).
2. **The precedent of asking about permanent deletion on failure = VS Code** (certain — admin answer): if the move to the trash fails, it asks 「영구 삭제할까요」 (permanently delete?) in a modal · if `files.enableTrash` is turned off, 「Delete Permanently」. Since it takes the form of asking a person, an agent call has no person to answer.
3. **Windows Explorer**: deleting at a network location = permanent delete + confirmation (likely — no official document found · forum).
4. ★**The Windows API can fall through to permanent delete instead of the recycle bin** (certain — Microsoft docs): `FOF_WANTNUKEWARNING` of `IFileOperation` = 「재활용 대신 파괴될 때 경고」 (back-translated: warn when destroyed instead of recycled) · `FOFX_RECYCLEONDELETE` = 「영구 삭제 대신 휴지통으로」 (back-translated: to the recycle bin instead of permanent deletion). The Rust `trash` crate appears to call with `FOF_NO_UI | FOF_ALLOWUNDO | FOF_WANTNUKEWARNING` (likely — read in a summary), so because there is no UI the warning is buried and there is a risk of **silent permanent deletion** (uncertain — must be re-checked on a real SMB share). Our implementation (TRD — platform trash · `IFileOperation`) must itself catch a fall-through to permanent delete and refuse.
5. Unreal asset deletion is recovered not by the OS recycle bin but by source control (uncertain — snippet). JetBrains was not examined.

## Peers

| Peer | When the trash is unavailable | App trash · location | Emptying | Source | Confidence |
|---|---|---|---|---|---|
| Obsidian | the pre-chosen method as is (no automatic substitution) | yes · hidden `.trash` inside the vault | no automatic · plugin | https://huggingface.co/spaces/anpigon/obsidian-qa-bot/blob/e272fa2f2c33a981360ec7e29f42e93f75fd504f/docs/obsidian-help/Files%20and%20folders/Manage%20notes.md · https://forum.obsidian.md/t/obsidian-permanent-deletes-files-system-trash/46409 | likely |
| VS Code | modal 「영구 삭제할까요」 | none | — | https://github.com/microsoft/vscode/issues/14638 | certain (modal) |
| Windows Explorer (network) | permanent delete + confirmation | none | — | no official | likely |
| Win32 `IFileOperation` | can destroy instead of recycling · has a warning flag | none | — | https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-setoperationflags | certain |
| Electron `shell.trashItem` | rejects on error · network behavior not documented | none | — | https://www.electronjs.org/docs/latest/api/shell | certain (docs) · network uncertain |
| Rust `trash` crate | `IFileOperation` · no separate network handling | none | — | https://raw.githubusercontent.com/Byron/trash-rs/master/src/windows.rs | likely (flags) · failure behavior uncertain |
| Unreal | recovered by source control | none | — | https://issues.unrealengine.com/issue/UE-201031 | uncertain |

## For us

- Options: A if the system trash is unavailable, use a hidden `.trash` inside the library (using Obsidian's second method as the fallback path — there is no precedent of switching by itself) · B always refuse · C permanent delete + a strong confirmation (the VS Code shape — does not fit agent calls). The user decides.
- Either way the implementation has to catch 「휴지통 대신 영구 삭제」 (permanent delete instead of the recycle bin) — a measurement on a real SMB share · a large file is the item to check before implementation.

## Limits

- light — no adversarial review. Obsidian was checked through a copy of the official help (a third-party repository) and a forum. JetBrains was not examined.
