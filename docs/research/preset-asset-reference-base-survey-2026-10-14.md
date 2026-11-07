# The base a bundle uses to point to files that ship with it — file-relative · root-relative · ID

- **Status:** light · used to choose the base for the paths by which a preset points to assets (skills · instructions · settings) · **decision (user, 2026-10-15) = absolute path — relative to the folder the user designates as the library · no scope restriction**
- **Date:** 2026-10-14
- **Method:** research light. 1 collector · main-session grounding (checked the original text of Claude Code plugins · Docker Compose · Cargo) · no adversarial review.
- **Trigger:** a preset is usually exchanged as one set folder together with its skills (decision memo). Even when a received set folder is placed as a subfolder of my library, the preset must still find the skills it received with it.

## Conclusions

1. **For a bundle that ships together, 「묶음 자리 기준 상대 경로」 (a path relative to the bundle's location) is the standard.**
   - **Claude Code plugins** — "Every component path in a manifest is relative to the plugin root and must start with `./`." · "Every component path must resolve inside the plugin root and must exist." A path that leaves the root (`..`) is rejected (`path escapes plugin directory`). The base is not the folder of the manifest file itself but the **plugin root** (the folder that contains `.claude-plugin/`). **Likely** (official docs, original text — checked by the main session).
   - **Docker Compose `include`** — "Relative paths in Compose files being referred by `include` are resolved relative to their own Compose file path, not based on the local project's directory." **Likely** (official docs, original text — checked by the main session).
   - **Cargo path dependencies** — "found in the `hello_utils` folder, relative to the `Cargo.toml` file it's written in." Path dependencies accept only relative paths, which makes subprojects hard to move; this is the reason RFC 3529 (path-base names) came about (collector). **Likely** (official docs, original text — checked by the main session).
   - Common to all: wherever the bundle folder is placed as a whole, the references inside it stay connected. Moving a single file outside the bundle on its own breaks it.
2. **Engines that hold a project-wide asset DB use root paths or IDs.**
   - **Godot** — `res://` (relative to the project root) + `uid://` (ID · the path is a fallback). Moving inside the editor makes references follow; moving with an outside file manager and leaving the `.uid` behind breaks them. **Uncertain** (collector · some third-party posts).
   - **Unity** — points only by the GUID in `.meta`. If `.meta` is lost, a new GUID is created and every reference breaks. Even when the folder is taken to another project as a whole, references stay connected if the `.meta` comes along. **Uncertain** (collector · old manual).
   - A root path (`res://`) does not fit when the bundle folder is placed somewhere else.

## Carrying this over to Engram (session interpretation — the decision is the user's)

- The flow of exchanging a set folder is the same as plugins · Compose · Cargo (relative to the bundle's location).
- ★**Engram-specific circumstance:** plugins forbid references outside the root, but an Engram preset can point **both** to a skill inside the set **and** to a **library-shared skill**. If only file-relative paths are written, the shared skill takes the form `../../skills/review`, and moving the set breaks that line.
- ID schemes (Unity · Godot) need an identity file attached to every file and break easily when moved from outside — in Engram, moving from outside (Explorer · agent shell) is the intended path, so they do not fit (interpretation).

## Limits

- Because it is light, there is no adversarial review. npm `file:` · VS Code extensions · Unreal paths · HTML's `/` root path were not collected.
- The main session did not check the primary sources for Godot · Unity.

## Addendum (2026-10-15) — lightweight tools only, engines excluded

User's remark: engines (Unity · Godot · Unreal) are not comparison targets because their reference exceptions are heavy. Only the same category (agent · prompt-configuration bundles) was re-examined (1 collector · 2 original-text checks by the main session).

- **Pointing to shared items by name rather than by path is the mainstream.** Paths are used only for files inside the bundle itself.
  - **Claude Code plugin dependencies** — `"dependencies": ["audit-logger", { "name": "secrets-vault", "version": "~2.1.0" }]` · a bare name is looked up in the same marketplace, and `name@marketplace` points to another one. ★「Bundle plugins for a team」 = a plugin holding only a name and a dependency list — the same shape as our preset★. If a dependency is missing: install · `/reload-plugins` · auto-update installs the missing dependency, and if it cannot be filled, that plugin does not load and shows an error (`Requires "secrets-vault@your-marketplace" ~2.1.0, installed 3.0.0`). **Likely** (official docs, original text — checked by the main session).
  - **Continue.dev** — `uses: sanity/sanity-opinionated` (hub `owner/name`) · local is `uses: file://…`. The docs do not say what happens when it is missing. **Likely** (official docs — checked by the main session).
  - **Goose recipes** — a sub-recipe is `path:` (relative to the recipe file · or absolute) · sharing = passing files · a recipe folder · a GitHub repository · a deep link. **Uncertain** (via a summarizer).
  - **VS Code profiles** — extensions by extension ID · export/import (file · gist). **Uncertain** (search results only).
- **Engines (reference only):** Unity package export (with dependencies) · Unreal Migrate (copies dependencies · the Content folder is required) · Godot has no export, only copying + a convention of fixed places such as `res://addons/<이름>/`. In all three, the receiving side does not rewrite paths (collector · some official docs).
- **Carrying this over to Engram (interpretation):** besides paths (absolute · relative), a **name reference** is the third option. It does not break wherever it is moved inside the library (the burden of a move feature · of relinking shrinks), and when given to someone else it connects if the receiving side has the same name, and otherwise shows 「찾을 수 없음」 (not found). The cost = names inside the library must not collide (a collision rule, or a distinction such as the bundle name before the name, `owner/name`, is needed).

## Addendum 2 (2026-10-15) — lightweight browser-style tools · plugins and engines excluded

User's remark: plugins are not comparison targets because their name scope is clear, and engines because their ID scheme is heavy — look at lightweight tools where files point to each other inside a folder the user manages (1 collector · 1 original-text check by the main session · the REAPER wiki is a JS-verification page and could not be read).

- **Note apps** — **Obsidian**: `[[Three laws of motion]]` (name) · when a folder is attached, "Folder paths start at the vault root and use forward slashes" → **a path relative to the top of the vault**. "Obsidian can automatically update internal links in your vault when you rename a file." (it can be turned off in settings). The help does not cover a move from outside. **Likely** (official help, original text — checked by the main session). The setting 「새 링크 형식(가장 짧은 경로 · 상대 · 절대)」 (new link format: shortest path · relative · absolute) was not checked. **Foam · Dendron** — name links · renaming inside the app fixes the links (**uncertain** — search snippets).
- **Media tools** — **REAPER**: files inside the project folder are relative, those outside use the full path · when saving, 「모든 미디어를 프로젝트 폴더로 옮기기」 (move all media into the project folder) · if one cannot be found, it asks at load time (**uncertain** — search summary). **Ableton Live**: 「Collect All and Save」 = gather the samples in use into the set folder · missing samples are searched for · specified manually (**uncertain** — forum). **Lightroom Classic**: stores the original's location as a path · organizing inside the app edits the catalog · when moved from outside, the folder gets a 「?」 → 「Find Missing Folder」 to specify the new location (**uncertain** — third-party blog). **M3U playlists**: relative entries are relative to that file · absolute entries break when the folder · drive changes (**uncertain**).
- **Common shape (interpretation):** ① when moved inside the app, the app fixes the references ② when moved from outside, a 「없음」 (missing) marker + the person specifies again · searches (documentation for automatic name matching was not found) ③ when giving to someone else, copy the folder as a whole, or 「쓰는 것 모아 담기」 (gather what is in use) (Ableton · REAPER).
- **Carrying this over to Engram (interpretation):** a path relative to the top of the library (Obsidian's top-of-vault path · Lightroom) + moving inside the app fixes it + a move from outside gets 「찾을 수 없음 · 다시 잇기」 (not found · relink) + gather-and-pack when giving (later) — this matches the common shape of this category. Engram's 「이름으로 자동 다시 잇기」 (automatic relinking by name) goes one step further than this category.
