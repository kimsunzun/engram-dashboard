# Preset folder entry — what claude and codex load natively from an extra folder, and what we must compose (2026-11-04)

| | |
|---|---|
| **Status** | Research done · cross-family review FIX applied · **decided 2026-11-04 → ADR-0341** (claude native `--add-dir` + env var · codex fills only what it lacks · terminal codex omits and notifies once per agent · missing folder blocks) |
| **Method** | `/engram-research` medium · 3 collectors (claude measurement · codex measurement + source · peer docs survey) · main grounding (raw docs re-read for the load-bearing claude claims, `codex --help` re-run) · cross-family review (see "Review") |
| **Versions** | Claude Code **2.1.284** · codex-cli **0.163.0** (source tag `rust-v0.163.0` = `83086188`) · Windows 11 |
| **Prior reports** | [`external-skill-dir-loading-claude-codex-2026-10-10.md`](external-skill-dir-loading-claude-codex-2026-10-10.md) · [`skill-md-recognition-claude-codex-2026-10-20.md`](skill-md-recognition-claude-codex-2026-10-20.md) · [`codex-instruction-channels-2026-10-20.md`](codex-instruction-channels-2026-10-20.md) — their conclusions still hold; this report adds the non-skill items, codex 0.163.0, and peers |
| **Confidence** | certain = measured (or measured + doc/source) · likely = one doc or source only · unknown |

## The feature (user, 2026-11-04)

A preset can name one or more **folders** (any path, e.g. a company-rules folder and a personal-rules folder). For each folder:
1. At launch it is attached automatically as an extra directory (`claude --add-dir`, codex equivalent).
2. Its contents are read the way the backend reads a project folder at startup (claude: `CLAUDE.md` · `.claude/rules` · `.claude/skills` …; codex: `AGENTS.md` · `.agents/skills` …) and attached to the agent.
3. The path is stored in the preset.

The user's earlier "no outside content" rule (PRD R7 · ADR-0312 decision 3) targets per-item links that break when the outside file moves; it does not forbid a folder entry (user 2026-11-04). The ADR wording needs an amendment.

## Conclusions

1. **claude does most of it natively — with one env var.** `--add-dir <folder>` alone loads the folder's `.claude/skills`, `.claude/agents`, `.claude/commands`. Adding `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1` also loads `CLAUDE.md`, `.claude/CLAUDE.md`, `CLAUDE.local.md` and `.claude/rules/**/*.md` without `paths:`. — certain (measured + docs).
2. **claude never loads, from an added folder:** `paths:`-scoped rules · subfolder `CLAUDE.md` (no on-demand loading outside cwd) · `AGENTS.md` · settings `env`/`hooks`/`permissions` (only `enabledPlugins`, `extraKnownMarketplaces`) · `.mcp.json` (likely) · output styles · `@` imports (not resolved in `-p` unless the cwd project already approved external imports — that case unmeasured). — certain except where marked.
3. **`permissions.additionalDirectories` in settings is file access only** — loads nothing, env var or not. Only the flag form loads configuration. — certain (doc verbatim + measured).
4. **codex attaches only skills (plus plugin MCP via one experimental field) from an extra folder, and only in JSON mode.**
   - `--add-dir` / `writable_roots` / `thread/start.runtimeWorkspaceRoots` = sandbox write access + the folder listed to the model; no `AGENTS.md`, no skills, no `.codex/config.toml`. — certain (real turn + source).
   - `skills/extraRoots/set` (stable, process-wide) = `SKILL.md` only, walked from the root itself, **dot-folders pruned** → `<folder>/.agents/skills` and `<folder>/.codex/skills` are skipped unless we pass those subfolders as the root. — certain.
   - **Present on 0.163.0, absent from our 0.156.1 notes (versions not diffed):** `thread/start.selectedCapabilityRoots` (experimental) = **per-thread** skill root, dot-folders included, bounded walk (depth 6 · 2000 dirs · 20000 entries · byte caps), plugin manifests under it also add MCP servers. Not a `thread/resume`/`thread/fork` parameter, but **resume restores the thread's saved roots** — source `core/src/session/mod.rs:650-665` ("Resume restores this thread's saved roots") + upstream test `app-server/tests/suite/v2/selected_capability_stack.rs:288,426` (resume without the parameter). — certain for start; likely for resume (source + test, not measured here).
   - Terminal mode: no arg / `-c` key / env var that **adds** a skill root or an `AGENTS.md` source. `CODEX_HOME` swaps the whole home (global `AGENTS.md`, `skills/`, but also auth and sessions — prior report finding 11), so it replaces rather than adds. — certain.
   - No native second project root for `AGENTS.md`. — likely (source only).
5. **Our side must compose:** for codex both modes — the folder's `AGENTS.override.md` > `AGENTS.md` (per codex rules) into our instruction channel; for codex terminal — skills have no path (same gap as library skills today, ADR-0337); for claude — nothing for the native set, plus a decision on the never-loaded items in (2).
6. **Peers persist such paths as a list in a config file, not as copies.** opencode `instructions`, Gemini `includeDirectories` (+ `context.loadMemoryFromIncludeDirectories`), Aider `read:`, paseo `additionalDirectories` per agent provider option. Org rules elsewhere come from fixed system paths or server-side admin settings. In the dashboard peers (vibe-kanban, orca, paseo) an `--add-dir` keyword grep found only pass-through — no rules-folder preset was found in the inspected paths (not a feature audit). — certain for opencode/Aider/paseo; likely for Gemini (snippet).
7. **Missing folder: only Gemini documents it** — missing `context.includeDirectories` entries are skipped with a warning (https://geminicli.com/docs/reference/configuration/#context — review finding, original not re-read by main). Others are silent. Blocking vs skipping is our own policy choice. — likely.
8. **claude also has `--plugin-dir`** (repeatable, session-local; already our S25 skill route, ADR-0316). It loads skills, agents, hooks, MCP, output styles from a plugin layout (`<dir>/skills` …) but no plugin-root `CLAUDE.md` — a bounded alternative only if the folder is laid out as a plugin. — certain (`claude --help` + plugins reference).

## Findings — claude (2.1.284)

| Item in added folder | `--add-dir` loads it? | Condition | Confidence |
|---|---|---|---|
| `CLAUDE.md`, `.claude/CLAUDE.md` | only with env var | value `1`/`true`/`yes`/`on` (bundled JS); still loads under `--setting-sources user` (measured — contradicts the doc's "project source" wording) | certain |
| `CLAUDE.local.md` | only with env var + `local` source | | certain |
| `.claude/rules/**/*.md` without `paths:` | only with env var | recursive | certain |
| rules with `paths:` | never | loaded with `conditionalRule:false`; on-demand walk is cwd-only | certain |
| `.claude/skills/<n>/SKILL.md` | yes | needs `project` setting source; off under `--safe-mode` / `strictPluginOnlyCustomization` | certain |
| `.claude/agents/*.md`, `.claude/commands/*.md` | yes | same source condition; **skipped under `--bare`** (skills still load); interactive sessions may need a trust entry for an untrusted folder (doc) | certain |
| `.claude/settings*.json` | only `enabledPlugins`, `extraKnownMarketplaces` | `env`, hooks, `outputStyle` ignored (measured) | certain (ignored) / likely (the two keys) |
| `.mcp.json` | no | measurement inconclusive (cwd baseline also missing in `-p`) | likely |
| subfolder `CLAUDE.md` | no | measured: reading `<dir>/sub/x.ts` did not load `<dir>/sub/CLAUDE.md` | certain |
| `AGENTS.md` | no | doc table: "Their `AGENTS.md` doesn't load" | certain |

**Order and collisions.** Memory text: managed → `~/.claude` → cwd ancestors (root first) → added folders in flag order; concatenated, nothing overrides. Name collisions on 2.1.284 drop the loser silently: skills = personal > project > added, first added folder wins; commands = project > added, first added wins; agents = project > added > personal, **last** added wins. — certain (measured R6–R9). The current skills docs (https://code.claude.com/docs/en/skills#run-a-same-named-skill-from-an-added-directory) describe a `/<prefix>:<skill>` form for a clashing added-folder skill and give v2.1.309 as the minimum (review finding; main re-read the prefix sentences, version number not re-located). The installed 2.1.284 drops the loser instead — upgrading claude changes this.

**Sources (verbatim, raw `.md` re-read by main 2026-11-04):**
- https://code.claude.com/docs/en/permissions.md — "Directories listed in `permissions.additionalDirectories` in a settings file grant file access only and don't load any of the configuration below." Table row: "CLAUDE.md files, `.claude/rules/`, and `CLAUDE.local.md` | Only when `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1` is set."
- https://code.claude.com/docs/en/memory.md — "To also load memory files from additional directories, set the `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD` environment variable" · "Directories you add with `--add-dir` while … is set | Their `CLAUDE.md` loads | Their `AGENTS.md` doesn't load".
- Measurements: 12 haiku `-p` runs with marker files and an `InstructionsLoaded` hook logger passed via `--settings` (no `~/.claude` edits).

## Findings — codex (0.163.0)

| Mechanism | Mode | Loads from the folder | Confidence |
|---|---|---|---|
| `--add-dir <DIR>` | terminal (TUI, `exec`) | sandbox write + `<workspace_roots><root>` line; ignored with a warning in read-only sandbox | certain |
| `-c sandbox_workspace_write.writable_roots` | both | same | certain |
| `thread/start.runtimeWorkspaceRoots` (exp.) | JSON | same | certain |
| `skills/extraRoots/set` | JSON | `SKILL.md` only, ≤6 levels, dot-folders pruned, process-wide | certain |
| `thread/start.selectedCapabilityRoots` (exp.) | JSON | per-thread, every `SKILL.md` incl. dot-folders, plugin manifests' MCP | certain (start + real turn) |
| folder as `cwd` | both | full project scan — but then it is the project, not an extra | certain |
| any skill / `AGENTS.md` source | terminal | none | certain |

**Startup scan from cwd (what our parser emulates for codex):** project root = nearest `.git` (configurable markers); for each folder root→cwd the first of `AGENTS.override.md` > `AGENTS.md` > fallback names, concatenated, total cap 32 KiB; skipped only when the project is explicitly untrusted. Skills from `.agents/skills` and `.codex/skills` root→cwd (even untrusted). `.codex/config.toml` applies only when trusted. — certain (prompt-input probes) / likely (cap, from source).

**Side notes:** the 0.163.0 TUI may run on a shared app-server daemon (`--no-daemon`) — a process-wide `extraRoots` must never reach a shared daemon. `codex debug prompt-input` and `codex exec` disagreed on applying a project `.codex/config.toml` under an ancestor-trusted path — cause unknown.

**Sources:** `codex --help` ("`--add-dir <DIR>` Additional directories that should be writable alongside the primary workspace" — re-run by main) · generated schemas (`codex app-server generate-json-schema [--experimental]`) · source `core/src/config/mod.rs:3538-3541,3664-3680` (workspace roots) · `core/src/agents_md.rs:58-122` · `ext/skills/src/host_roots.rs:48-71` · `exec-server/src/capability_discovery.rs:21-29,110-120` · `app-server-protocol/src/protocol/v2/thread.rs:83,153` · zero-model probes A/A2/B + 2 real turns (B8, C).

## Findings — peers

| Peer | Mechanism | Persistence | Confidence |
|---|---|---|---|
| Claude Code | `--add-dir` (+ env var); managed `CLAUDE.md` at a fixed system path; `@import`; rules symlinks | flag = per session; settings list = access only | certain |
| codex | `AGENTS.md` hierarchy, `CODEX_HOME` | no extra-rules-folder option found | likely |
| Gemini CLI | `includeDirectories` + `context.loadMemoryFromIncludeDirectories` | settings file | likely (snippet) |
| opencode | `instructions` array (files, globs, URLs) | `opencode.json` (global or project) | certain |
| Aider | `--read` / `read:` | `.aider.conf.yml` | certain (flag) |
| Cline · Continue · Windsurf/Devin · Cursor · Copilot | fixed global dir or server-side team/org rules | home dir or vendor service | certain–likely |
| paseo · vibe-kanban · orca | pass `--add-dir` / `additionalDirectories` through | per-agent option (paseo) | certain (source grep) |

## Implications for S25 (main's inference — one level lower confidence than the facts above)

- **Two shapes are possible.** (A) *native first*: claude gets `--add-dir` + the env var and loads its native set itself; we compose only what a backend lacks (codex `AGENTS.md`; codex JSON skills via `selectedCapabilityRoots` or `extraRoots` on `.agents/skills`; terminal codex = a launch block like library skills under ADR-0337). (B) *compose everything*: we parse the folder ourselves and translate into the S25 materials (merged instruction file, plugin-dir junction, merged settings) for both backends; the folder is still passed as `--add-dir` for file access.
- (A) gives the subset claude itself loads from an added folder (Conclusion 1 — not the full project set: no `paths:` rules, nested memory, hooks, MCP) and follows CLI rule changes, but the env var is process-wide: it would also load `CLAUDE.md` from any other `--add-dir` the agent gets. (B) gives one consistent preview and one code path, but has to re-implement and track each backend's discovery rules.
- The "folder scan" preview the user asked for needs our own parser in both shapes — in (A) it only predicts, in (B) it also produces the materials.
- Missing folder: S25 already blocks launch when an attached item is missing (PRD E4 ①). Applying the same rule is the consistent default.
- codex JSON skills: `selectedCapabilityRoots` (experimental, per thread, restored on resume per source) or `extraRoots` (stable; process-wide, and our JSON agents each own their app-server process, so per agent — likely: `backend/codex/mod.rs` spawns `codex app-server --stdio` per agent (`APP_SERVER_SUBCOMMAND`); not traced through the transport).

## Gaps

- claude `@` imports after the cwd project approved external imports — unmeasured.
- claude `.mcp.json` from an added folder — inconclusive.
- codex `selectedCapabilityRoots` across resume (source + upstream test say restored) and compaction — not measured here.
- codex TUI via `--remote` to a pre-configured app-server — unmeasured.
- Interactive claude sessions (all claude probes were `-p`) — trust prompts for agents/commands from an untrusted added folder unmeasured.
- Gemini `includeDirectories` memory loading — original not checked.

## Review

Cross-family review — codex (GPT, `--effort high`, independence level 2–3) via `codex:codex-rescue`, 2026-11-04. Verdict **FIX**, 10 findings, all applied above:

| Finding | Label | Applied |
|---|---|---|
| `selectedCapabilityRoots` survives resume (source `session/mod.rs` + upstream test) — the "resume needs `extraRoots`" inference was wrong | refuted · high | main re-read the source lines; Conclusion 4 + Implications fixed |
| `CODEX_HOME` is an env var that moves global `AGENTS.md` / `skills/` | refuted · med | Conclusion 4 narrowed to "adds"; replacement caveat kept |
| claude `--plugin-dir` omitted | refuted · med | Conclusion 8 |
| native-first "matches a project folder exactly" | refuted · med | Implications narrowed to the loaded subset |
| codex selected roots also add plugin MCP; discovery is bounded | refuted · med | Conclusion 4 |
| Gemini documents missing include dirs (skip + warning) | refuted · med | Conclusion 7 (original not re-read by main) |
| claude ≥ 2.1.309 collision prefix is in the skills docs | refuted · low | collision note fixed |
| `--bare` skips added-folder agents/commands | refuted · med | claude table |
| dashboard peers: "none has" → "none found in inspected paths" | not-found · med | Conclusion 6 |
| "new in 0.163.0" unsupported (versions not diffed) | not-found · low | Conclusion 4 wording |

Spot-checked as supported by the reviewer: the claude env-var gate and settings-only access (docs + 2.1.284 loader), `conditionalRule:false` / cwd-only nested memory (loader), codex writable roots not feeding `AGENTS.md` discovery (`config/mod.rs:3669`, `agents_md.rs:68`), `extraRoots` process-wide with hidden-dir pruning, the codex `AGENTS.md` hierarchy and trust gate, opencode / Aider persistence.
