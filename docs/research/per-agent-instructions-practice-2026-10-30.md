# Per-agent instructions practice — how to split common defaults from claude/codex-only parts (2026-10-30)

- **Status:** `/research` light — collector 1 · main grounding spot-check (opened the frontmatter of the rulesync repository's rule and skill files directly with `gh api`) · no adversarial review (light). Awaiting decision — S25 「지시서 · 스킬도 공통 / claude / codex 로 가르기」 (user proposal 2026-10-30).
- **Use:** Put a 「공통 / claude / codex」 marker on each line of the instructions · skills inside a preset, show three groups on screen · at launch, common first and then that backend's own — does this proposal match practice.
- **Confidence legend:** certain = primary document · repository checked directly · likely = primary document summary · unknown = not found.

## Conclusion

1. **「공통 기본 + 에이전트별 예외」 is an existing shape.** The direct peer rulesync puts `targets` in the frontmatter of rule files and skill files — `["*"]` = all tools · `["claudecode"]` = only that tool (certain — checked repository `.rulesync/rules/coding-guidelines.md` · `.rulesync/skills/approve/SKILL.md` directly). **The same device is used for skills too.**
2. **Where the marker is attached differs per peer.** rulesync = file frontmatter (the file declares itself) · Roo Code = folder (`.roo/rules/` common · `.roo/rules-{모드}/` mode-specific) · ruler = only a per-agent setting, with no per-content marker. **No precedent was found for putting the marker on the bundle side, such as a preset.** No precedent was found for a file name suffix (`guide.codex.md`) or for dividing fields within one file either.
3. **Merging is almost always concatenation, not overwriting.** The only one that documents the order is Roo, and it is 「전용 먼저 → 공통 나중」 (the opposite of our proposal).
4. Cursor · Continue · GitHub Copilot split not by agent but by **applicability condition** (file path glob · always apply, etc.) — there is no concept of a target agent.

## Per peer

| Peer | Where the marker goes | Merging · order | For skills too? |
|---|---|---|---|
| **rulesync** | file frontmatter `targets: ["*"]` / `["claudecode"]` (certain) | unknown — each tool gets its own generated file | yes — `targets` also in the `SKILL.md` frontmatter (certain) |
| **ruler** | none — concatenates all `.md` in `.ruler/` and only turns on and off with `ruler.toml` `[agents.X]` (likely) | concatenation · root `AGENTS.md` first | propagates `.ruler/skills/` to each agent · per-skill target specification not found (experimental feature) |
| **Roo Code** | folder — `.roo/rules/` · `.roo/rules-{모드}/` (certain) | concatenation · mode-specific first → common after · the same folder is in name order (certain) | rules only |
| **GitHub Copilot** | frontmatter `applyTo` (path glob) · `excludeAgent` distinguishes surfaces inside Copilot (certain) | concatenation · personal > repository > organization | unknown |
| **Cursor** | only the application timing by frontmatter (always · glob · description · manual) (certain) | Team → Project → User | not applicable |
| **Continue.dev** | only the application condition by frontmatter (certain) | name order · order via the `01-` prefix | not applicable |

Sources:
- rulesync — https://github.com/dyoshikawa/rulesync (`.rulesync/rules/coding-guidelines.md` frontmatter `root: false` · `targets: ["*"]` · `.rulesync/skills/approve/SKILL.md` frontmatter `targets: - "*"` — main checked directly with `gh api repos/dyoshikawa/rulesync/contents/…` 2026-10-30. This chapter is not on the first page of the README · documentation site)
- ruler — https://github.com/intellectronica/ruler
- Roo Code — https://roocodeinc.github.io/Roo-Code/features/custom-instructions
- GitHub Copilot — https://docs.github.com/en/copilot/how-tos/configure-custom-instructions/add-repository-instructions
- Cursor — https://cursor.com/docs/context/rules
- Continue.dev — https://docs.continue.dev/customize/deep-dives/rules

## For us

| | (가) marker on the preset line (proposed) | (나) marker in the file frontmatter (rulesync shape) |
|---|---|---|
| Precedent | not found — a marker on the bundle side | rulesync (direct peer) |
| Same file different per preset | works — common in one preset · codex-only in another preset | does not work — the file decides once |
| Files of a received bundle (another root · G25) | not touched | the frontmatter of someone else's file must be edited → conflict on update |
| The problem of the frontmatter reaching the model | none | the frontmatter of the instructions `.md` must be stripped when it is loaded · how claude · codex handle an unknown key added to `SKILL.md` is unchecked |
| Order | common first → only-specific after (proposed) — opposite to Roo | the same choice remains |

## Gaps · what could not be confirmed

- rulesync's merge order for common · only-specific · the default when `targets` is omitted — not found in the documentation.
- A file name suffix · precedent for dividing fields within one file · other tools that generate CLAUDE.md and AGENTS.md together — not searched (cannot assert that none exist).
- Cline's rules folder — not checked directly.
