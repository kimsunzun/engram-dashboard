# ★ When to validate asset · settings files — everything on scan · on pick · on change · on a separate press — light survey

| Item | Value |
|---|---|
| Status | survey done · decision pending (user) |
| Date | 2026-10-17 |
| Method | `/research` light — 1 collector (about 11 searches · fetches) · 1 main grounding spot-check (Unreal Data Validation doc WebFetch) · no adversarial review |
| Question | When validating the library's settings · preset files by syntax + schema and putting ⚠/⛔ icons on the list, must we validate everything on every scan (cost) — which is the practice among validating on pick · only changed files · on a separate press |
| Earlier measurement | `docs/research/settings-asset-validation-spike-2026-10-17.md` |

## Conclusion

- **No one validates everything on every scan and attaches marks — the default is bit by bit**: validate on open · on edit · on save (changed ones only) and remember the result; the whole project is run by a **separately pressed command** or CI. (likely)
- **Unreal Data Validation** — 「on asset save (enabled by default), and through menu options in the editor and Content Browser」 + CIS · results go to 「Message Log in the editor, and in the console in CIS」 · no mention of icons shown in the Content Browser. (close to certain — main checked the original)
- **Attaching list marks to files that were never opened is rare** — where it exists, background analysis results are accumulated (JetBrains whole-project analysis — optional · yaml-language-server `workspace` mode — optional). (likely · VS Code explorer marks unconfirmed)
- **Cost is not an obstacle** — compile the schema once and reuse it many times (ajv docs: compiling is slow and running is fast). Rust `jsonschema` figures not looked at. (pattern = certain · figures = unconfirmed)

## Findings

| Tool | When | Scope | List marks for unopened files | Source | Confidence |
|---|---|---|---|---|---|
| VS Code + yaml-language-server | on open · on edit · diagnostic mode `openFilesOnly` / `workspace` | default is open files · the whole workspace is optional | unconfirmed | https://github.com/redhat-developer/vscode-yaml | mode = likely |
| JetBrains | open files live · 「Inspect Code」 = a separate press · whole-project analysis = optional | open files / separate / optional | Project Errors by default covers only files that have been opened | https://blog.jetbrains.com/idea/2020/08/working-with-code-problems-in-intellij-idea/ | likely |
| Unreal Data Validation | on save (default on) · menu 「Validate Assets」 · command line (CI) | what was saved / what was picked / everything | none (not mentioned in the docs) | https://dev.epicgames.com/documentation/en-us/unreal-engine/data-validation-in-unreal-engine (main checked) | close to certain |
| Unity | import when a file changes · errors go to the console | changed files | no basis | https://docs.unity3d.com/Packages/com.unity.asset-manager-for-unity@1.7/manual/import-assets.html | uncertain |
| Godot | scan when changed · import · broken dependencies on scene open (「Fix Dependencies」) | changed files / the open scene | no basis | (third-party article) | uncertain |
| ajv | compile once · run many times | — | — | https://ajv.js.org/guide/getting-started | pattern = certain |

## What this gives our design

- Syntax (yaml reading): the scan opens the file anyway to read `kind` — a syntax error found then gets ⛔ for free.
- Schema content validation is done **on pick (when a card is opened) + when a changed file is re-read**, and the result is remembered as (path · modified time · size) · list icons show only remembered results (no icon before validation) · whole-library validation is a separately pressed command (human = menu · LLM = the same command).
- The pre-launch checks (claude doctor · the codex model list) stay as they are, separately.

## Gaps

- Whether the VS Code explorer · problems pane shows unopened files · JetBrains project view marks · Unity · Godot marks · Obsidian whole validation — no primary source.
