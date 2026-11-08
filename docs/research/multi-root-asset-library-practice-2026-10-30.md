# Multi-library-root practice — how to hold and combine asset bundles from several sources (2026-10-30)

- **Status:** `/research` light — collector 1 · main grounding spot-check (read the Claude Code skills documentation and the Obsidian forum directly) · no adversarial review (light). Awaiting decision — the 「루트 간 참조 허용?」 of S25 decision memo G25 (multi-root).
- **Use:** Decide which way the S25 asset library goes: option 1 (multiple roots + references across roots allowed · 「찾을 수 없음 · 다시 연결」 when broken) or option 2 (one root + bundle clones under it).
- **Confidence legend:** certain = primary document checked directly · likely = primary document summary · search excerpt · forum · unknown = not found.

## Conclusion

1. **Registering several roots is the practice.** Four of the five peers surveyed (Claude Code · codex · Blender · Ableton) accept several roots. None forces gathering under one folder. The exception, Obsidian, has one root per vault and deliberately keeps vaults apart.
2. **The problem of overlapping names is solved with a prefix before the name.** Claude Code = plugin skills always `/플러그인:스킬` · additional-folder skills `/폴더:스킬` only when they overlap. Tools that find by path (Blender · Ableton) use 「어느 라이브러리의 어느 경로」, so there is no overlap problem.
3. **Only path-based tools (Blender · Ableton) make references across roots** — and both provide a **broken-reference recovery tool (relink)** and an **escape hatch of 「프로젝트 안으로 복사해 모으기」** together. The AI skill systems (Claude Code · codex) have no documented device for pointing at another root by name (they work around it with junctions · symbolic links).
4. **For received bundles, 「따로 둔 사본 + 판 단위 갱신」 is common.** Claude Code plugin = a copy in the cache · a new copy only when the version changes. What happens if the user edits that copy is not in the documentation (unknown).

## Per peer

| Peer | Several roots? | References across roots | When a root is removed | Update of received bundles · user edits |
|---|---|---|---|---|
| **Claude Code skills · plugins** | yes — enterprise · personal `~/.claude/skills` · project · subfolder · `--add-dir` additional folder · plugin · claude.ai account (certain) | none by name (unknown). Symbolic links between plugins inside a marketplace → content is copied at install time, skipped if it points outside the marketplace (certain) | removing a marketplace = removing its plugins. A plugin dropped from the list stays and is marked 「not found」 (certain) · deleting a skill folder root = unknown | cache copy · updated only when the version changes · auto-update off by default · a local-path marketplace uses the real files directly (certain) · whether cache edits are preserved = unknown |
| **codex CLI skills** | yes — repo `.agents/skills` · user `~/.agents/skills` · admin · built-in (likely) | unknown · follows symbolic links | unknown | unknown · does not merge the same name and shows both |
| **Blender asset library** | yes — a list of names + paths in preferences · an import method per library (Link = read-only reference · Append = copy) (certain) | yes — Link references another `.blend` and changes in the source follow (certain) | does not lose the reference thanks to placeholder data, and reconnects to the new path with 「Library Relocate」 (likely — old-version documentation excerpt) | Link = fix at the source · Append = my copy (does not follow the source) (certain) |
| **Ableton Live** | yes — browser 「Places」: User Library · Packs · user folders (likely) | yes — a set references outside files by path · 「Collect All and Save」 copies them into the project (certain) | warning in the status bar + find by hand in File Manager · automatic search (certain) | unknown |
| **Obsidian** | one root per vault · several vaults are isolated from each other | wikilinks · backlinks cannot cross vaults — only remains as a feature request · workaround = `obsidian://` URI · open a parent folder as the vault (likely — forum) | unknown | not applicable |

Sources:
- Claude Code skill locations · same-name priority · prefix — https://code.claude.com/docs/en/skills (main checked directly 2026-10-30)
- Claude Code plugins · marketplace — https://code.claude.com/docs/en/plugin-marketplaces · https://code.claude.com/docs/en/plugins/host-marketplace.md
- codex skills — https://learn.chatgpt.com/docs/build-skills (redirected from developers.openai.com/codex/skills · read as a summary)
- Blender — https://docs.blender.org/manual/en/latest/editors/preferences/file_paths.html (the body is thin, so supplemented with a search of 4.x documentation copies)
- Ableton — https://www.ableton.com/en/manual/managing-files-and-sets/
- Obsidian — https://forum.obsidian.md/t/support-for-linking-between-separate-vaults/100628 (main checked directly — a feature request in the 「Feature archive」 category · a reply recommends a URI)

## For us — option 1 vs option 2

| | Option 1: multiple roots + references across roots allowed | Option 2: one root + bundle clones under it |
|---|---|---|
| Peer basis | the same shape as path-based tools (Blender · Ableton) — both provide relink and a copy escape hatch together. **We already have both** (「찾을 수 없음 · 다시 연결」 R19 · copy P6) | the same shape as Obsidian's workaround (open a parent folder as one) |
| In-place folders | works — register `~/.claude/skills` · another drive · a shared folder without moving it | does not work — it must be moved or cloned into the root |
| Cost | root name before the path (peer practice = prefix) · move record · trash per root · move rules between roots | 0 — the current design (R7) as is |
| Breakage | if a root is removed, references from other roots become 「찾을 수 없음」 — the user's job | occurs only inside the root |

## Gaps · what could not be confirmed

- The Blender · Ableton official documents were read only as summaries · search excerpts (secondary).
- VS Code · npm workspaces were not covered.
- Claude Code: whether edits to a cached plugin survive an update · whether a warning appears when a skill root is deleted — unknown.
- codex: same-name priority · update · removal behavior — unknown.
- No AI skill peer was found that points at another root by name (Claude Code's plugin prefix · link copy rules are the closest).
