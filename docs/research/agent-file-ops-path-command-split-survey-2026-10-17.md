# ★ Agent file operations on the library — precedents for 「경로가 바뀌면 명령, 아니면 직접 + refresh」 (if the path changes, a command; otherwise write directly + refresh) — light survey

| Item | Value |
|---|---|
| Status | survey done · decision pending (user) |
| Date | 2026-10-17 |
| Method | `/research` light — 1 collector (about 10 searches · fetches) · 2 main grounding spot-checks (Godot original text · Obsidian forum WebFetch) · no adversarial review |
| Question | A rule that tells the agent: 「만들기 · 내용 고치기 = 파일 직접 쓰고 `library.refresh`(오류를 돌려줌) · 옮기기 · 이름 바꾸기 · 지우기 = `library.move` · `rename` · `delete` 명령(참조를 같은 조작 안에서 고침)」 — is there a precedent for this, and if not, what is the closest shape |

## Conclusion

- **The 「경로가 바뀌는 조작은 앱 명령으로」 (operations that change the path go through an app command) half is common.** Several places expose a reference-fixing move/rename as a tool (Unity MCP `assets-move` · Unreal MCP `rename_asset` · Obsidian MCP `move_note` · `ts-refactor-mcp`, which opens the TS language server · Dendron · Foam), and the reason is always the same: 「셸 `mv` 는 참조를 끊는다」 (a shell mv breaks references). (Likely — mostly MCP listings and READMEs; we did not read the code.)
- **We did not find anywhere that documents our combined rule (content written directly + an explicit refresh that returns errors, path operations forced through commands).** (Uncertain — about 10 searches, not exhaustive.) The closest shape is Unity MCP `assets-move` — the command moves, collects per-item errors, then calls `AssetDatabase.Refresh`. Writing content directly to disk and letting refresh/import read it afterwards is also Unity's usual shape.
- **None of them 「금지」 (forbids) moving directly.** Choosing the tool is a recommendation, and things moved from outside are revived through identity sidecars (Godot `.uid` · Unity `.meta` GUID) or a cleanup pass (Unreal redirector). Obsidian accepts that links break when you move from outside. We saw no one that re-finds by content.

## Findings

| Tool | Done directly | Done through a command · API | Moved · renamed from outside | Source | Confidence |
|---|---|---|---|---|---|
| Obsidian | note content | only an in-app move fixes links · third-party MCP `move_note` (claims to update links) | **links break** — 「The links only update if the file is moved within Obsidian」 · a scriptable move command remains a feature request | https://forum.obsidian.md/t/move-vault-files-by-obsidian-command-line-option-or-url-to-allow-scriptable-link-updates/6776 (main checked) · https://glama.ai/mcp/servers/diegorv/another-bloated-obsidian-mcp-server/tools/move_note | breakage = likely (main checked) · MCP = uncertain (listing level) |
| Unity | content directly on disk | `AssetDatabase.MoveAsset` · Unity MCP `assets-move` (MoveAsset per pair · per-item errors · Refresh at the end) | `.meta` GUID sidecar — moving it along keeps references | https://docs.unity3d.com/460/Documentation/ScriptReference/AssetDatabase.MoveAsset.html · https://glama.ai/mcp/servers/butterlatte-zhang/unity-ai-bridge/tools/assets-move | MCP = likely · sidecar = uncertain (from memory — original not read) |
| Unreal | inside the editor | rename · move → a Redirector at the old location · Fixup / `-fixupredirects` re-saves the references · Unreal MCP `rename_asset` (claims to update references) | moving in the file explorer is discouraged (forum — answer not read) | https://dev.epicgames.com/documentation/unreal-engine/asset-redirectors-in-unreal-engine · https://glama.ai/mcp/servers/aadeshrao123/Unreal-MCP/tools/rename_asset | redirector = likely · moving from outside = uncertain |
| Godot 4.4+ | text files directly | moving in the editor's FileSystem dock fixes references | **`.uid` sidecar** — 「when you move a script or shader outside Godot, you should also move the `.uid` file along with it」 · the path is saved too, and if the path is unchanged it is resolved by that (fallback) | https://godotengine.org/article/uid-changes-coming-to-godot-4-4/ (main checked) | close to certain — primary source · main checked the original (single source, so the tag is likely) |
| VS Code · agent tools | the agent often uses a shell `mv` | third-party `ts-refactor-mcp` (opens the language server's 「파일 옮기면 import 고치기」 (fix imports when a file moves) to the agent) · `cclsp` (`rename_symbol`) · a request on the Cursor forum to 「VS Code 내장 rename 을 열어 달라」 (expose VS Code's built-in rename) | `mv` breaking imports is where the problem starts | https://cdn.jsdelivr.net/npm/ts-refactor-mcp@0.1.2/README.md · https://forum.cursor.com/t/executing-vscodes-built-in-file-rename-functionality/129697 | MCP = likely · 「1차 도구 없음」 (no first-party tool) = uncertain (not found ≠ does not exist) |
| Foam · Dendron | note content | Foam `renameNote` · Dendron Rename Note / Refactor Hierarchy — update backlinks | not covered (within what was read) | https://wiki.dendron.so/notes/2gHesjl7XP5Q9QNJWRv7E | likely |
| Logseq | — | only an in-app rename | unconfirmed | https://discuss.logseq.com/t/is-it-possible-to-change-assets-name-without-breaking-the-existing-notes/6207 | uncertain |
| JetBrains | — | — | — | — | abstained (not looked at) |

## grounding

- Godot `.uid` — checked three sentences of the original (creation · moving along · path fallback). **Supported.**
- Obsidian — forum original: moving from outside breaks links and only an in-app move fixes them · a scriptable move command remains a request. **Supported.** ★The collector cited this thread as the source of 「Obsidian CLI `move` 가 링크를 고친다」 (the Obsidian CLI move fixes links); that is **not supported** — the thread says no such command exists. We did not check the official CLI's current capabilities (downgraded).

## What this gives our design

- Taking path operations as commands (the PRD's `library.move` · `rename` · `delete`) is the shape the industry chose for the same reason.
- 「내용은 직접 + refresh 가 오류를 돌려준다」 (content directly + refresh returns the errors) is the part we add. Unity's MoveAsset → Refresh flow is the closest.
- The collector's inference (no source): if, like Godot, we **store identity together with the path**, things moved from outside can also be revived at `library.refresh`. Our 「다시 잇기」 (same kind · same name · exactly one) is the weak version of that.

## Gaps

- JetBrains not looked at · the built-in rename behavior of Claude Code / Copilot / Cursor was not confirmed from primary docs · the LSP `workspace/willRenameFiles` spec was not fetched.
- MCP server capabilities are at the listing (glama.ai) · README level and we did not read the code.
