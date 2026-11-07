# Library slot — how to show several source folders (peer survey)

- **Status:** version with the adversarial review (BLOCK) applied · awaiting decision (a sketch from before the preset PRD — re-examination of the screen 1 library slot)
- **Date:** 2026-10-14
- **Method:** research medium · design-decision mode. 3 collection tracks (AI agent tools · game engines/DCC · general file libraries) · main grounding (「grounding」 below) · one cross-family adversarial review (「적대 리뷰」 (adversarial review) below).
- **Confidence legend:** certain = source text checked + independent cross-confirmation · likely = source text checked (single source) or partly supported · uncertain = from memory · source text unchecked.
- **Earlier survey:** `docs/research/agent-preset-components-survey-2026-10-11.md` (components · UI pattern survey) — this document fills that document's gap on the 「라이브러리 목록 모양」 (shape of the library list).

## What is asked

The user will have many instructions · skills and will organize them in their own folder structure. In that case:

1. In the middle of the library, is it right to lay the list out **flat**, or should it show a **folder tree**, or should it have a **classification unrelated to folders** (catalog · collection · tag)?
2. When there are several source folders, how are they merged for display?
3. Is there a way to casually link a single file outside the registered roots? If so, how is portability between PCs handled?
4. How are roots registered?

## Conclusions (top level)

1. **Within the sample, DCC · file tools that handle large libraries keep both 「파일이 놓인 곳(물리 폴더)」 (where the file sits (physical folder)) and 「찾아보는 길(가상 분류)」 (the way to browse (virtual classification)).** The closest real thing is the **Unreal Content Browser** — in the left Sources panel there are a project folder hierarchy tree and a Collections list together (plugin · engine content folders can also be turned on), and the middle Asset View is filtered by picking several folders with Ctrl/Shift. Others: Lightroom (Folders panel ↔ Collections) · foobar2000 (switching between folder-structure view ↔ artist/album view) · Obsidian (folders ↔ nested tags) · Substance 3D Designer (a filter can have several conditions and one resource shows up in several filters — not exclusive). **Blender is the opposite end** — its left tree is not the disk folders but a **catalog tree independent of file location** (an asset goes in only one catalog). The sample is not wide enough to generalize this as 「주류」 (mainstream). **Likely** (Unreal · Blender · Substance filters have source text checked · Lightroom · foobar2000 · parts of Obsidian are from snippets/forums/memory).
2. **On the AI-tool side, the collection only confirmed 「디스크 위치 = 분류」 (disk location = classification), and it cannot be asserted that there is no virtual classification.** What was confirmed: Claude Code loads by scope (Enterprise · Personal · Project · Nested · `--add-dir` · Plugin) and separates name collisions by prefix (`/plugin:skill` · `/apps/web:deploy`) and priority — these are name-collision rules, not evidence of list-screen conventions. **The adversarial review raised counterexample candidates** (not checked by main — 「적대 리뷰」 below): the TypingMind prompt library (tags · group visibility) · the JetBrains AI Assistant Prompt Library · the Cursor Rules management UI · the Zed Skills manager. They are not the same as file-based skills, but the statement 「AI 도구엔 라이브러리 선례가 없다」 (AI tools have no library precedent) is withdrawn. **Uncertain.**
3. **For showing several roots, both tracks are seen.** ① A tree per root (VS Code multi-root — a per-root explorer tree · search results also grouped per root) ② merged into one pool (Houdini search path · Espanso · music-library watch folders). Unreal places the roots (project · plugin · engine) as top-level nodes inside one folder tree. There is no basis for stating a general convention on how the source is shown when merging. **Likely** (①) · **uncertain** (convention for showing the source).
4. **The surest precedent for 「루트 밖 파일 하나 링크」 (linking a single file outside the roots) is the Zotero linked file** — it remembers only the original location (no sync · groups · mobile), and **only files under the base folder (Linked Attachment Base Directory)** are stored as relative paths, so re-specifying the base folder on each PC makes them match. A link **outside** the base folder stays an absolute path, so it has no portability. When a base folder is specified, it offers to find broken absolute-path links and re-link them all at once. Claude Code instructs attaching outside skills with a **symbolic link** (if two paths point to the same target, it is loaded only once). The adversarial review also cited **Adobe Bridge Collections** (drag in a file from any drive and keep it as a reference · repairing missing files) — the source text returned 403, so main did not check it. **Certain** (Zotero · Claude Code source text) · **uncertain** (Bridge). → Treating a one-off link outside the roots as **allowed but an item without portability** is closer to the precedents.
5. **Root registration is 「설정의 이름 + 경로 목록」 (a settings name + a path list) or 「폴더 안 매니페스트 + 추가 명령」 (a manifest inside the folder + an add command).** VS Code `.code-workspace` `{name, path}` (relative paths possible) · the foobar2000 Music Folders list · Blender Preferences (from memory) · Houdini `HOUDINI_PATH` · Unity `file:` relative/absolute · Claude Code `marketplace.json` + `claude plugin marketplace add`. Portability is secured by **a path relative to the root + a per-PC root location** (Zotero · Unity · the Claude marketplace — relative to the marketplace root · `..` forbidden). The Blender UUID is an id that points to a **catalog**, so it is not an example of root-location portability (the draft's misuse — review finding). **Likely.**
6. **A missing item is not hidden but shown, and re-linked per folder.** Lightroom: a `?` icon on a missing folder · right-click 「Find Missing Folder」 · re-linking per folder is recommended over per file. **Likely** (search snippets · parts of the source text pages returned 403).

## If carried over to Engram (interpretation — the decision is the user's)

The library slot in the current briefing v13 is 「왼쪽 = 소스 폴더 체크박스 · 가운데 = 평평한 목록(검색 + 칩 + 종류)」 (left = source-folder checkboxes · middle = a flat list (search + chips + kind)). In light of conclusions 1 · 3, the gap is that **the middle list shows no folder structure at all**. Options:

| Option | Shape | Precedent | Pros | Cons |
|---|---|---|---|---|
| **A. Keep the flat list** | as now · folders only as a path display | most AI tools (Claude Code etc.) | simplest · search-centered | once there are more than a few dozen files, the folder organization the user made is thrown away |
| **B. A folder tree per source** | the middle is a per-root tree like the VS Code explorer | VS Code multi-root | the user's organization is shown as it is | cross-cutting filters such as search · 「쓰는 중」 (in use) overlap awkwardly with the tree · it gets long with many roots |
| **C. Left = folder-tree navigation, middle = results of the chosen folder** | unfolding a left source folder gives a subfolder tree · the items of the chosen folder (several possible) go in the middle list · search · chips unchanged | **Unreal Sources panel** (folder tree · multi-select → Asset View) · Lightroom Folders panel · foobar2000 folder-structure view | folder organization and search live together · keeps the current screen structure almost intact | the left column is narrow if it gets deep · whether subfolders of a chosen folder are shown is not even in the Unreal docs, so we must decide |
| **D. C + virtual classification (tags · collections)** | C plus a tag · collection zone on the left | **Unreal** (Collections in the same panel) · Lightroom Collections · Substance filters (non-exclusive) · Blender catalog (a virtual tree instead of folders) · Obsidian nested tags | grouping on an axis different from folders (merges with briefing T-1) | the most to build · the decision on where tags are stored (T-2) follows |

- **D's classification axes are two** — a tree where an item goes in only one place (Blender catalog — exclusive like a file system) and tags · collections where an item goes in several places (Substance filters · Lightroom · Unreal). Briefing T-1 (nested tags) is the latter.
- One-off link outside the roots: following conclusion 4, **allowing 「바깥 링크」 (outside link) as a first-class item but marking it as without portability** resembles Zotero. Putting one 「바깥 링크」 group in the left column fits directly into the C · D structure. ★A source folder playing the role of the Zotero base folder holds only for **files inside the source folder** — outside links do not benefit from it (a logic gap in the draft — review finding).
- Missing: following conclusion 6, put a per-source-folder 「폴더 다시 찾기」 (find the folder again) in the left column's right-click.

## grounding (main · source text comparison)

| Claim | Source | Verdict |
|---|---|---|
| Blender catalogs are independent of the file layout on disk · an asset goes in only one catalog · definition file `blender_assets.cats.txt` (`UUID:경로:단순이름`) · reason for separation = free movement between files | https://developer.blender.org/docs/features/asset_system/backend/asset_catalogs/ | supported |
| Zotero linked files under the base folder are stored as relative paths · base folder specified per PC · offers automatic re-linking of broken absolute paths | https://www.zotero.org/support/preferences/advanced | supported |
| Claude Code skill-location table · Enterprise > Personal > Project priority · symbolic links allowed (same target once) · plugin `/plugin:skill` · nested `/apps/web:deploy` · `--add-dir` | https://code.claude.com/docs/en/skills | supported |
| VS Code per-root explorer tree · `.code-workspace` folders `{name, path}` relative paths | https://code.visualstudio.com/docs/editing/workspaces/multi-root-workspaces | collector checked source text · not re-checked by main |
| Lightroom `?` icon · Find Missing Folder | https://helpx.adobe.com/lightroom-classic/desktop/manage-catalogs-and-files/locate-missing-photos.html | search snippet only — partly supported |
| foobar2000 folder-structure view ↔ artist/album view | https://hydrogenaudio.org/index.php?msg=717049 (forum) | partly supported (not official docs) |
| Unity `file:` relative (relative to Packages)/absolute · slashes recommended | https://docs.unity3d.com/Manual/upm-localpath.html | collector checked source text · not re-checked by main |
| Houdini `HOUDINI_PATH/otls` auto-load · embedding in the scene discouraged | https://www.sidefx.com/docs/houdini/assets/install.html | collector checked source text · not re-checked by main |
| Unreal Sources panel = project folder hierarchy + Collections list · plugin/engine folders shown by selection · Ctrl/Shift multi-folder selection | https://dev.epicgames.com/documentation/unreal-engine/sources-panel-reference-in-unreal-engine?lang=en-US | supported (found by the review · main checked) |
| Substance Designer filter = several conditions · one resource in several filters · not exclusive · saved in the project file | https://experienceleague.adobe.com/en/docs/substance-3d-designer/using/workspace/library/managing-custom-content-and-filters | partly supported (several paths · aliases are not on that page) |
| Claude marketplace `source` relative path (relative to the marketplace root · `..` forbidden) | https://code.claude.com/docs/en/plugin-marketplaces | collector checked source text · not re-checked by main |

## Adversarial review

codex (cross-family · effort high · level 2 · web search), once. Verdict **BLOCK** — on the draft. It was applied as below.

| Finding | Severity | Applied |
|---|---|---|
| 「AI 도구엔 가상 분류 · 라이브러리 선례가 없다」 (AI tools have no virtual-classification · library precedent) is an overstatement — TypingMind prompt library (tags) · JetBrains Prompt Library · Cursor Rules · Zed Skills | high | conclusion 2 changed to 「단정 못 함 · 철회」 (cannot be asserted · withdrawn). **The four counterexamples are not checked by main** (reviewer's claim) |
| Blender used as a precedent for C (physical folder tree) — Blender's left tree is a catalog (independent of disk) | high | removed from C and moved to D (virtual classification) |
| Unreal was put as 「못 읽음」 (could not be read) — the Sources panel doc shows folder tree + Collections + multi-select → Asset View | high | **main checked the source text** (https://dev.epicgames.com/documentation/unreal-engine/sources-panel-reference-in-unreal-engine?lang=en-US) — raised to the representative precedent in conclusions 1 · C · D. Whether subfolders are included · whether collections are static/dynamic is not in that doc |
| Substance Designer was left uninvestigated — several watch paths · virtual filters · path aliases | high | **main source-text check is partial** — only 「필터는 조건 여럿 · 리소스 하나가 여러 필터에 · 배타 아님 · 프로젝트 파일에 저장」 (a filter has several conditions · one resource in several filters · not exclusive · saved in the project file) was confirmed (https://experienceleague.adobe.com/en/docs/substance-3d-designer/using/workspace/library/managing-custom-content-and-filters). The procedure for adding several paths · path aliases are not on that page → left unchecked |
| Zotero was made the sole precedent — Adobe Bridge Collections also does outside-file references + repairing missing files | medium | added to conclusion 4 as the reviewer's claim · source text returned 403, so **unchecked** |
| A logic gap that 「소스 폴더 = Zotero 기준 폴더」 (source folder = Zotero base folder) makes outside links portable too — Zotero relative paths are only for files **under** the base folder | high | stated explicitly in conclusion 4 · the carry-over section |
| The Blender UUID was used as a root-location portability example — it is a catalog id | medium | withdrawn in conclusion 5 |
| 「혼합이 주류」 (mixed is mainstream) · 「병합하면 출처 접두어 · 열」 (when merged, a source prefix · column) are generalizations beyond the sample | medium | wording in conclusions 1 · 3 lowered to 「표본 안에서」 (within the sample) · 「불확실」 (uncertain) |

- Reviewer grounding spot-check: the Blender catalog · Zotero base folder · Claude Code skill location are all **supported** — with the caveat that the first two had their use wrong, as in the two rows above.
- No re-review was run after the changes (medium single pass).

## Limits · gaps

- **Not a single screen capture was seen** — the 「평평한가 · 트리인가」 (flat or tree) verdicts are mostly inferences drawn from document descriptions. In particular, for AI tools, 「폴더 트리를 안 보여 준다」 (they do not show a folder tree) means 「보여 준다는 증거를 못 찾았다」 (no evidence that they do was found), not a confirmed absence.
- **Could not be read:** Cursor (redirect) · the Zed Rules Library (404 — per the reviewer, changed to Skills) · Substance 3D Painter resource paths · aliases (403) · the Adobe Folders panel (403) · Adobe Bridge Collections (403) · MusicBee (402).
- **Not investigated:** Windsurf · JetBrains AI Assistant · Roo · dedicated prompt-manager apps · Godot · Krita · Maya · Figma/Adobe CC Libraries · Logseq · Calibre · Eagle · Raycast/Alfred.
- No material was found that states, as a user complaint, the **reason** for keeping a separate virtual classification — the confirmed basis is one Blender design document.
- Follow-up candidates: Substance path aliases (the `your_assets://` kind — a portability candidate · from memory) · how Unreal Collections are stored · Blender library registration and relative paths.
