# Multiple Libraries — Reference-Path Practice · Problems That Arise with Several Roots · Precedents with a Shape Like Ours

- **Status:** version with the adversarial review (FIX) applied · awaiting decision (a pre-preset-PRD idea — the user leaned toward 「첫 판부터 라이브러리 여러 개」, and asked to look at practice before deciding)
- **Date:** 2026-10-15
- **Method:** research medium · design-decision mode. 2 collection tracks (AI agent tools · asset/file browsers and dev tools) · main grounding (「grounding」 below) · one cross-family adversarial review (「적대 리뷰」 below).
- **Confidence legend:** certain = source text confirmed + independently cross-confirmed · likely = source text confirmed (single source) or partly supported · uncertain = from memory · based on a summary · source text not confirmed.
- **Earlier surveys:** `docs/research/library-multi-root-display-survey-2026-10-14.md` (how several roots are **shown and registered**) · `docs/research/preset-asset-reference-base-survey-2026-10-14.md` (reference base = decided as an absolute path from the very top of the library). This document covers only **how references are written when there are several roots · what breaks**.

## What is asked

The current decision is 「라이브러리 하나 · 프리셋 안 참조 = 라이브러리 맨 위부터의 경로(`스킬/리뷰/review`)」 (one library · a reference inside a preset = a path from the very top of the library). If we keep several libraries and let A's preset use B's skill:

1. How does one reference line say 「어느 라이브러리인가」 (which library) — what is the practice?
2. What problems arise because there are several roots, and how does each peer handle them?
3. Is there anything shaped like ours (several user-folder libraries + presets that bundle parts across roots)?

## Conclusions (top layer)

1. **The shape seen most often in this sample = 「루트 이름 + 그 루트 안 경로」 (root name + a path inside that root), and the root name and the disk location are written separately.** ★We do not go as far as calling it 「주류」 (mainstream)★ — adversarial review pointed out: VS Code's is a variable that expands in some settings and Claude Code's is a lookup by name in a registry, so the mechanisms differ too much to group all three as the practice of 「파일에 저장된 항목 참조」 (references to items stored in a file). Only Unreal directly matches the shape of a stored reference. The Obsidian URI takes 「볼트 id + 볼트 안 경로」 (vault id + a path inside the vault) (reviewer report · not confirmed by main — it is a URI for links, not the shape of a reference inside a note).
   - **Unreal** — registers the mount point name (`/Engine/` · `/PluginName/`) and the disk location (`ContentPath`) separately. A reference uses the name side (`/PluginName/...`). **Likely**, close to **certain** (API doc text).
   - **VS Code multi-root** — points to a sibling root by attaching the root folder name, like `${workspaceFolder:Client}`. The root name can differ from the disk folder name via the `name` property. **Likely** (variables doc text · the `name` property is a collector report).
   - **Claude Code plugin dependencies** — `"name@marketplace"` points to a plugin in another marketplace. The marketplace name is the identity, and a user registers only one marketplace per name. **Likely** (source text).
   - **Unity packages** — `Packages/<패키지 이름>/...`. **Uncertain** (collector report · not confirmed by main).
   - Result: if you move the disk folder, you only change the registered location and the reference stays the same. In exchange, **renaming a root breaks every reference that wrote that name.** The reviewer's way to avoid this — write a never-changing id in the reference and use the name only for on-screen display (Obsidian vault id · a Blender asset library name is for the screen, so changing it is fine — reviewer report · not confirmed by main). We still found no peer that documents 「이름을 바꾸면 참조를 일괄 고친다」 (when you rename, fix the references in bulk).
2. **Three other shapes, and they fit us less.**
   - **id + path fallback** (Godot `uid://` · Unity GUID) — the id sits beside or inside the file (Unity = a `.meta` beside every asset · Godot = a `.uid` beside scripts and shaders only, inside the file for scenes and resources, `.import` for imported resources — a detail the adversarial review corrected). When moving from outside, that file must move too. If the id breaks, Godot finds the file by the path written alongside and shows a warning. **Likely** (Godot source text). Already discarded as the 「무거운 엔진 모델」 (heavy engine model) (earlier survey).
   - **Lookup by name through a search path** (Houdini `HOUDINI_PATH` · Claude Code skill scope priority Enterprise > Personal > Project) — with the same name, the earlier one wins (shadowing). The Houdini side is **uncertain** (search snippet · memory).
   - **Path relative to the referencing file** (Blender — keeps the linked library path relative to the referencing `.blend`, and re-fixes it when saving under another name — the reviewer reported confirming it from source text · not confirmed by main). **Uncertain**, short of **likely**.
3. **Six problems that arise because there are several roots, and how peers handle them.**

   | Problem | Peer handling | Confidence |
   |---|---|---|
   | ① The same path exists in two roots | The root-name prefix separates them outright (Unreal · VS Code · Claude `name@marketplace`). Where there is no prefix, order shadows — Unreal makes overlapping mounts win by 「override chain 앞에 두어 이기게」 (putting them at the front of the override chain) · Claude Code skills use scope priority · plugin skills coexist with a `/plugin:skill` prefix | likely |
   | ② Can a reference pointing to another root be trusted | **Claude Code blocks it by default** — it does not install a dependency from another marketplace (exception if already installed and enabled). Reason in the source text: 「한 마켓플레이스가 사용자가 검토하지 않은 출처의 플러그인을 조용히 설치하지 못하게」 (back-translated: so that one marketplace cannot silently install plugins from sources the user has not reviewed). To lift it, add it to the allow list in the root-side `marketplace.json` | likely (source text) |
   | ③ A reference that depends on another root when handing over to someone else | The receiving side must register that root **under the same name** for it to resolve (Claude Code · claude.ai sync plugin description). Places with a separate bundling step: Ableton 「Collect All and Save」 (copies outside files into the project) · Blender Append (brings in as a copy) | likely (Claude source text) · uncertain (Ableton · Blender — summaries) |
   | ④ The root is gone (folder deleted · drive missing) | Claude Code: depends on the case — if a dependency written on the marketplace side is outside the allowance it **refuses the install itself**, and a dependency written in `plugin.json` installs but **the plugin does not load** · if the local copy is disabled, `is disabled — enable it or remove the dependency` (this sentence is from the local-testing section — the adversarial review pointed out the generalization). Lightroom: shows the missing folder + reconnects per folder with 「Find Missing Folder」 | likely (Claude source text) · uncertain (Lightroom — community) |
   | ⑤ Moving an item to another root | **We found no peer that automatically rewrites a stored reference such as YAML on a move between roots.** Close precedents — Blender 「Relocate」 (re-points a linked item to another library) · Adobe Bridge collections (bundle files from several drives; moving inside Bridge keeps collection membership · a move from outside gets a fix-up offered). A move within one project is fixed by the tool (Godot FileSystem dock · Unity Project window). Unreal leaves a redirector (a pointer from the old place → the new place) at the moved spot and cleans up in bulk later | likely (Godot · Unreal source text) · uncertain (Blender · Bridge — reviewer report, not confirmed by main) |
   | ⑥ A preset inside a preset crosses roots | Cycles (A→B→A) and chained dependencies must be decided. AgentsRoom documents chains of prompt links and cycle detection · Claude Code refuses to install when the same dependency requires different versions (`has conflicting version requirements`) | uncertain (AgentsRoom — reviewer report, main fetch failed) · likely (Claude source text) |

4. **We found nothing with exactly our shape (several user folders + presets that bundle across roots + a folder browser). However, the adversarial review found two close ones more** — which the collection tracks missed.
   - **Skills Manager** (`xingkongliang/skills-manager` · a React 19 + Rust desktop app — the same stack as ours) — 「여러 코딩 도구의 에이전트 스킬을 한 앱에서 관리」 (manage agent skills of several coding tools in one app). **It bundles skills into named presets** and turns them on together by clicking a preset in a workspace · applying a preset is 「한 번 복사이지 실시간 동기가 아니다」 · **any folder can be attached as a skill root, but that root is treated as a separate workspace that does not take part in the global preset sync** — that is, it looks like presets that cross roots are blocked. The docs do not say whether a preset points to a skill by name or by path. It is not a folder browser (a preset · library management screen). **Likely** (README source text — confirmed by main).
   - **AgentsRoom** (linked prompts) — a prompt pulls in other prompts in a chain and detects cycles, and **it deliberately keeps project links and global links separate** (reviewer report). **Uncertain** (the main fetch failed with a 413 — not confirmed).
   - Both are on the side of **blocking or separating** 「루트를 가로지른 묶음」 (bundles that cross roots) — the same direction as Claude Code's default refusal (②).
   - Two other close ones:
   - **Claude Code marketplace** — the same in that a bundle of parts depends across several sources. But it is a **registry + name** model, not a folder browser, and it blocks dependencies across sources by default.
   - **Unreal Content Browser** — the same in that it shows several roots (project · plugin · engine) in one tree and references them by root name. But it is a heavy engine.
   - Places not seen because they were outside the collection scope: Cursor · Zed · Aider · OpenHands · Gemini CLI extensions · prompt management tools · Obsidian (several vaults) · Zotero · REAPER. **「없다」 (there is none) is not an assertion; it means 「이번 표본에서 못 찾았다」 (we did not find one in this sample).**

## If we carry this into Engram (candidates — not a decision)

Putting the practice on top of our decisions (absolute path · moving inside the app fixes references · reconnecting by name) gives this shape. **Before the user's decision.**

- **Reference shape — three separator candidates** (user question on 2026-10-15, 「`@` 냐 `/` 냐」):
  - `업무/스킬/리뷰/review` — the library name in the first path segment (the Unreal `/PluginName/...` shape). **Session recommendation** — the route down the tree = the reference line · with one library the shape just gets one more segment. Cost = it is separated only by the rule 「첫 칸은 항상 라이브러리 이름」 (the first segment is always the library name).
  - `업무:스킬/리뷰/review` — the VS Code `${workspaceFolder:업무}` · Godot `res://` shape. Confusable with the Windows drive `C:`.
  - `review@업무` — the Claude Code `플러그인@마켓` shape. It is a registry-by-name model, so it carries no folder path — in a folder browser the same name is common and cannot be told apart (the same reason we dropped name references).
  - The mapping library name → disk folder is kept separately in settings (the Unreal · VS Code shape). There is also a way to write a never-changing id in the reference instead of the name (the Obsidian vault id shape), but then the human-readable YAML turns opaque.
- **Names must not overlap** (Claude Code's 「이름 하나에 하나」 (one per name)).
- **Moving the disk folder** only fixes the location in settings — references stay the same.
- **Renaming a library** makes the app fix the preset lines too — our 「옮기기 · 이름 바꾸기가 참조를 고친다」 (moving · renaming fixes references) decision widened to the library level. No peer has a precedent for this.
- **Moving across libraries** also makes the app fix references — again no precedent (⑤).
- **Open questions:** whether to simply allow references pointing to another library, or to mark · warn (②: peers block or separate — Claude Code default refusal · Skills Manager separate workspace) · cycles and chained dependencies when a preset inside a preset crosses libraries (⑥) · how to tell someone that a part depends on another library when handing over (③: 「내보내기」 (export) is already deferred to 「나중」 (later)) · how to show a preset when a library is missing (④).

## grounding (main external check)

| Claim | Source | Decision |
|---|---|---|
| Claude Code `"name@marketplace"` · default refusal of dependencies from another marketplace · reason · the allow list belongs only to the root marketplace · an already-enabled dependency is an exception · written in `plugin.json` it fails to load after install | https://code.claude.com/docs/en/plugin-dependencies.md (source text read directly) | supported |
| Claude Code disabled dependency → `is disabled — enable it or remove the dependency` | same doc (`--plugin-dir` section — in the local-copy context) | partly supported (a sentence in the local-copy context) |
| Claude Code one marketplace per name · plugin skills coexist as `/plugin-name:skill-name` · scope priority | collector quotes (marketplace-reference · skills docs) | unconfirmed (main did not check directly — the collector quotes are the source wording) |
| VS Code `${workspaceFolder:Client}` for a sibling root | https://code.visualstudio.com/docs/editor/variables-reference (quote via a summarizer) | supported |
| Unreal `RegisterMountPoint` — RootPath (`/Engine/` · `/PluginName/`) · ContentPath (disk) · placing at the front of the override chain wins on overlap | https://dev.epicgames.com/documentation/unreal-engine/API/Runtime/CoreUObject/FPackageName/RegisterMountPoint (quote via a summarizer) | supported |
| Godot UID + path fallback · a move inside the dock is fixed · a move from outside also moves `.uid` · a warning on an outside move | https://godotengine.org/article/uid-changes-coming-to-godot-4-4/ (quote via a summarizer) | supported |
| Unreal redirector · Unity `.meta` · Ableton · Lightroom · Blender · Houdini · Continue · Goose | collector reports (partly summaries · 403 · table of contents only) | unconfirmed — confidence lowered in the table above |

| Skills Manager — skill presets · any folder as a skill root · that root is outside the global preset sync · apply = copy once · React 19 + Rust | https://github.com/xingkongliang/skills-manager/blob/main/README.md (quote via a summarizer) | supported |
| AgentsRoom linked prompts · cycle detection · project/global link separation | https://agentsroom.dev/features/linked-prompts | uncertain (fetch 413 — not confirmed by main) |

## Adversarial review

codex (cross-family) · effort high · level 2 · web search on · decision **FIX**. Applied:

1. Overstated 「주류」 (mainstream) (high) → lowered conclusion 1 to 「이번 표본에서 가장 많이 보인 꼴」 (the shape seen most often in this sample) and noted the mechanism differences.
2. Missing close peers (high) → added Skills Manager (confirmed by main) · AgentsRoom (unconfirmed) to conclusion 4.
3. A limited rebuttal of 「루트 간 이동 고치기 선례 없음」 (no precedent for fixing references on a move across roots) (medium) → added Blender Relocate · Adobe Bridge collections to ⑤ and narrowed to 「YAML 저장 참조를 자동으로 고쳐 쓰는 선례는 없다」 (there is no precedent for automatically rewriting a stored YAML reference).
4. Missing alternative for renaming a root (medium) → added the id-reference + display-name route at the end of conclusion 1.
5. Generalization of a Claude error sentence (medium) → fixed ④ per case.
6. Godot `.uid` details (low) → fixed conclusion 2.
7. Missing from the problem list — cycles and chained dependencies (medium) → added ⑥.

Reviewer spot grounding: reported that Unreal · VS Code · Claude · Godot were supported, and that Blender relative paths and Ableton Collect All and Save were also supported by source text (not confirmed by main).

## Gaps and limits

- How to fix references in bulk when a root name is renamed · how to rewrite a stored reference such as YAML on a move between roots — no precedent in this sample.
- The reviewer's Obsidian vault id · Blender Relocate · Adobe Bridge · AgentsRoom were not confirmed from the source text by main.
- Outside the collection scope (the list in conclusion 4 above) — something shaped like ours may exist there.
- Only the table of contents of the Blender source text could be read · the Ableton help returned 403 · the relative-path base of Continue · Goose was not found in their docs.
