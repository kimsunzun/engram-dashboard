# How claude · codex recognize and name SKILL.md (2026-10-20)

| | |
|---|---|
| **Status** | Investigation complete · decision pending (input for S25 preset library's skill recognition · naming · collision rules) |
| **Method** | 2 pages of official docs (claude skills · plugins-reference) · source (codex tag `rust-v0.156.1` = `b412ff32` · for claude, the bundled JS inside the installed `claude.exe` was read closely as strings) · measurement (the same 26 fake skills fed to both CLIs — 「방법」 below). **0 real model calls** (claude = a local fake API server · codex = `skills/list` · `debug prompt-input`) |
| **Target versions** | Claude Code **2.1.284** · codex-cli **0.156.1** · Windows 11 (26200) |
| **Questions** | (1) File-name case (2) Depth (bundle folder · skill within a skill) (3) Required frontmatter fields · when it is broken (4) The name that becomes the identity · constraints (5) Name overlap (6) Junctions · links (7) Minimal shape of the claude plugin path · name (8) Other special files in a skill folder |
| **Earlier reports** | [`external-skill-dir-loading-claude-codex-2026-10-10.md`](external-skill-dir-loading-claude-codex-2026-10-10.md) (discovery location · `--plugin-dir` · codex folder) · [`codex-instruction-channels-2026-10-20.md`](codex-instruction-channels-2026-10-20.md) (fake server · trap proxy method) |
| **Confidence legend** | certain = measured (or measured + source cross-check) · likely = source or docs only · uncertain = not tested |

## Conclusion

1. **The two CLIs recognize a 「스킬」 (skill) differently — a lenient claude, and a strict, deeply scanning codex.**
   - **claude** opens **only one layer** of `<뿌리>/<폴더>/SKILL.md` by joining the path. So case is **an OS rule** (on Windows `skill.md` is also loaded), and it loads the skill even when the frontmatter is missing or broken.
   - **codex** walks under the root **recursively up to 6 layers** and picks up only files whose name is **exactly `SKILL.md`** (on Windows too). If the frontmatter · `description` is missing, the YAML is broken, or a BOM is attached, it **leaves that skill out** and raises it only in the error list.
2. **The identity of the name differs too.**
   - **claude:** the identity is the **folder name**. For a plugin it takes the shape `<플러그인>:<폴더 이름의 [^A-Za-z0-9_-] 를 - 로>`. The frontmatter `name` is attached only as a display name and an alias.
   - **codex:** the identity is the **frontmatter `name`** (whitespace folded), and if absent, the **actual folder name after following the junction**. There is a 64-character cap. If an ancestor of the actual folder has a plugin manifest, the `<플러그인>:` prefix is attached.
3. **Neither side blocks name overlap.**
   - **claude:** merges only the same **actual file**. If two identical display names are called by name, **the first in sort order** runs **without an error**.
   - **codex:** merges only the same **canonical path**. If names overlap, **both are visible to the model** — only the `$이름` mention is ambiguous and does not pick.
4. **Engram's volatile plugin path (TRD §1-5) works as is — measured, certain.** Three conditions to keep:
   - **No manifest needed** — `skills\` alone under `--plugin-dir <휘발>\engram` makes one plugin.
   - **Name** — the name is `engram:<정션 이름>`. The TRD's 「정션 따라가기 미측」 (following junctions untested) was confirmed by this measurement.
   - **What not to put** — do not put `skills\SKILL.md`. Only that one becomes a skill and the children are ignored.
5. **There are five places that diverge from Engram's rules** (「Engram 규칙과 어긋나는 곳」 below).
   - `skill.md` recognition — codex silently discards it.
   - A skill inside an attached skill — leaks in only to codex.
   - No frontmatter format check — codex silently drops it.
   - No name character · length check — claude changes the name, and some names cannot be Windows folder names.
   - Case · substitution of the collision key — claude's internal names collide.

## Answers by question

| # | claude 2.1.284 | Basis · confidence | codex 0.156.1 | Basis · confidence |
|---|---|---|---|---|
| **1 File name** | Opens by attaching `SKILL.md` to the folder path → **OS file system rule**. Measured on Windows: both `skill.md` · `Skill.md` are loaded (plugin `engram:c-lower` · `engram:c-mixed`, project `p-lower`). The docs do not state case (the examples are all uppercase) | Measured + bundle (stat · readFile after `join(dir,"SKILL.md")`) · **certain** | The file name obtained from the walk is **byte-compared with `== "SKILL.md"`** (`ext/skills/src/loader/discovery.rs:176`) → exact match regardless of OS. On Windows `skill.md` · `Skill.md` are **in neither the list nor the errors** | Measured + source · **certain** |
| **2 Depth** | **One layer only**: `<뿌리>/<폴더>/SKILL.md` (common to plugin `skills/` · personal · project). Under a bundle folder (`c-group/c-nested`) · a skill inside a skill (`c-outer/c-inner`) · anything deeper is **not loaded** (debug log 「Loaded 23 skills」 = 26 items − three). Exception: if `SKILL.md` is directly inside plugin `skills/`, **only that one** is a skill (source) · in a project, a subfolder's `.claude/skills` is also read when files in that folder are touched (docs) | Measured + bundle · **certain** | **Recursive**: up to 6 layers · 2000 folders · 20000 entries (`loader/mod.rs:31-32` · `discovery.rs:49`). Every `SKILL.md` is a skill — `c-nested` · `c-inner` · `c-deep6` loaded, `c-deep7` not loaded. **Skips under dot (.) folders** (no `.c-hidden`). Only the plugin's Agent Plugin root is in direct-child mode | Measured + source · **certain** |
| **3 Frontmatter** | All optional. No frontmatter → loaded as is · broken YAML → loaded 「필드 없이」 (without fields) (warning log) · BOM allowed · CRLF allowed. If there is no description, the first line of the body is used as the description (project `p-nofm` = 「Heading」). ★But a skill **whose description was not written directly on the plugin path** (`c-nofm` · `c-nodesc` · `c-badyaml`) is only in init `slash_commands` and **not in the skill list sent to the model** (calling it by name through the Skill tool works — `engram:c-nofm` measured). `claude plugin validate`: broken YAML = error · no frontmatter/description = warning | Measured · **certain** (the reason it drops out of the model list could not be pinned in code = likely) | **Required**: first line `---` · closing `---` · a non-empty block · YAML parsing (a line whose value contains `: ` is fixed once with quotes and re-read) · `description` non-empty · `name` ≤ 64 characters (`skills/src/parser.rs`). If violated, **that skill drops out** and goes on the `errors` of `skills/list` (TUI = 「Skipped loading N skill(s) due to invalid SKILL.md files.」). ★**If a BOM is attached it drops out as 「missing YAML frontmatter」**★ (`c-bom`). `name` is optional | Measured + source · **certain** |
| **4 Name** | **Identity (internal name · model list name) = folder name.** Plugin = `<플러그인>:` + the `[^A-Za-z0-9_-]` of the folder name changed to `-` (`.c-hidden` → `engram:-c-hidden`). Frontmatter `name` = the `/` menu display name and an alias — the Skill tool accepts both `engram:c-fm-other` · bare `c-fm-other`. A bare folder name (`c-dir-mismatch`) is rejected (「Did you mean engram:c-dir-mismatch?」). If the frontmatter name differs, the model list shows the folder side (debug 「Skill prompt: showing "engram:c-dir-mismatch" (userFacingName="engram:c-fm-other")」). **No length cap** (65 characters loaded). Characters: per the docs, rejects control characters · zero-width · bidi · `<>` · whitespace other than a single-cell space. Spaces · dots · Korean · `:` are loaded as a frontmatter name (`c-weird` · `c-colon`) | Measured + bundle + docs · **certain** | **Identity = frontmatter `name`** (whitespace folded). If absent or an empty string, the **normalized (junction-followed) actual folder name** (`loader/host.rs:394` — `c-junc-alias` → `c-jtarget` · `engram-1234abcd-c-noname` → `c-noname`). **≤ 64 characters** (65 = error). **No character restriction** (spaces · dots · uppercase · Korean · `:` measured). ★**If an ancestor of the actual folder has `.codex-plugin/plugin.json` · `.claude-plugin/plugin.json` · `.cursor-plugin/plugin.json`, `<manifest name>:<이름>`**★ (`myplug:c-ps` measured · `loader/namespace.rs`). A `$이름` mention reads on only `[A-Za-z0-9_:-]`, so a name containing a space cannot be called by mention (`skills/src/mentions.rs:226`) | Measured + source · **certain** (mention = source only) |
| **5 Name overlap** | **The same actual file (realpath)** only once — among plugins (`engram2:c-exact` · `engram2:other-dirname` skipping measured), and if it overlaps with a personal skill folder, the plugin side is skipped (source). If they are different files, all are loaded: personal > project — with the same name only personal (`c-pers` measured · docs) · plugins coexist, separated by prefix. Two with the same frontmatter name within one plugin (`c-dupA` · `c-dupB`) have different folder names, so **both are loaded**, and if called by the overlapping display name `engram:c-dup`, **the first in sort order (c-dupA)** runs **without an error** | Measured + bundle · **certain** | Merges into one **only the same canonical `SKILL.md` path** — the root that comes first wins (order = `$CODEX_HOME/skills` · `~/.agents/skills` · system · plugin · extraRoots · project `.agents/skills` — `host_roots.rs` · `host_merge.rs:232`). **Name overlap is listed in full with no error and no priority, and both appear in the model prompt** (three `c-dup` entries · two lines in the prompt measured). If called by name like `$c-dup`, the count ≠ 1, so it **does not pick** (`skills/src/selection.rs`) | Measured (list · prompt) + source (mention) · **certain** |
| **6 Junctions · links** | Follows — if a folder entry is a directory or a link, it is accepted. Name = **link name**, 「Base directory」 = link path, duplicate decision = realpath | Measured · **certain** | User · project · admin scopes follow directory links · junctions (only the system scope ignores them) · the same actual folder is walked only once. Name · prefix = **based on the actual path**, the path shown in the prompt is the link path (`r2/engram-1234abcd-c-exact/SKILL.md`). **`SKILL.md`, which is a file symbolic link, drops out of the walk** (`walk` of `exec-server/src/local_file_system.rs`) | Junction = measured · **certain** / file link = source only · **likely** (no permission to create one, so not measured) |
| **7 claude plugin** | **No manifest needed** — if `X\skills\` exists under `--plugin-dir X`, 「is one plugin: skills at its top marks it」 (debug log). Plugin name = the **X folder name** (init `plugins` = `engram` · source `engram@inline`). Minimal shape = `X\skills\<이름>\SKILL.md`. Pitfalls: (1) if `X\skills\SKILL.md` exists, only that one is a skill (bundle) (2) if `X` lacks a marker like `skills\`, it is seen as 「플러그인 묶음」 and each child becomes a plugin (`--help`) (3) **`claude plugin validate` fails if there is no manifest** (「No manifest found in directory」) — the runtime is fine (4) it does not follow links (「components are read without following symlinks」) — the actual path must be checked separately (5) plugin name rules (no spaces · `@` · `:` · path separators · `claude-` · `anthropic-` prefixes reserved — docs). `engram` is safe | Measured + docs + bundle · **certain** | Not applicable (JSON = `skills/extraRoots/set` host root · terminal = working folder `.agents/skills` — earlier report). But **codex also reads `.claude-plugin/plugin.json` as a plugin marker** (4) | — |
| **8 Special files** | `.claude-plugin/plugin.json` inside a personal · project skill folder → loaded as a `<이름>@skills-dir` plugin (docs) · reserved folder names `synced` · `anthropic-skills` (docs) · substitutions such as `${CLAUDE_SKILL_DIR}` in the body · `CLAUDE.md` at the plugin root is not read (docs). The rest are ordinary files the model reads when needed | Docs + bundle · **likely** (not measured this time) | `agents/openai.yaml` — display info · dependent tools · policy (`allow_implicit_invocation: false` = removed from implicit-invocation targets · `products` restriction — `skills/src/model.rs`) · the three plugin manifests of ancestors (the prefix in 4) · dot folders ignored. The rest are ordinary files | Source · **likely** |

### Results by fixture (measured)

claude = `--plugin-dir <휘발>\engram` (a junction to the library per entry — exactly the TRD §1-5 shape) · codex = `skills/list` after `skills/extraRoots/set [lib]`.

| Fixture | Content | claude | codex |
|---|---|---|---|
| `c-exact` | Baseline | `engram:c-exact` | `c-exact` |
| `c-lower` · `c-mixed` | `skill.md` · `Skill.md` | Loaded | **None** (no error either) |
| `c-group/c-nested` | Under a bundle folder | **None** | `c-nested` |
| `c-outer` + `c-outer/c-inner` | A skill inside a skill | `c-outer` only | **Both** |
| `c-deep6` · `c-deep7` | 6 layers · 7 layers | None · none | Present · none |
| `.c-hidden` | Dot folder | `engram:-c-hidden` | None |
| `c-noname` · `c-emptyname` | No `name` · `name: ""` | Folder name | Folder name |
| `c-nodesc` · `c-nofm` · `c-badyaml` | No description · no frontmatter · broken YAML | Loaded · **not in the model list** | **Dropped as an error** |
| `c-bom` · `c-crlf` | BOM · CRLF | Normal · normal | **Dropped as an error** · normal |
| `c-dir-mismatch` | `name: c-fm-other` | `engram:c-dir-mismatch` (display `engram:c-fm-other`) | `c-fm-other` |
| `c-weird` · `c-colon` | `Weird Name.v2 한글` · `ns:c-colon` | `engram:c-weird` · `engram:c-colon` (display is the frontmatter name) | Frontmatter as is |
| `c-long64` · `c-long` | 64 characters · 65 characters | Both loaded | Loaded · **error** |
| `c-dupA` · `c-dupB` | Same `name: c-dup` | Both (separated by folder name) | Both `c-dup` |
| `c-caseA` · `c-caseB` | `C-Case` · `c-case` | Both | Both |
| `c-junc-alias` | Junction → an unnamed `c-jtarget` | `engram:c-junc-alias` | **`c-jtarget`** |
| `c-junc-plug` | Junction → `c-ps` under `myplug\.claude-plugin` | `engram:c-junc-plug` (display `engram:c-ps`) | **`myplug:c-ps`** |

Side measurement — claude project `.claude/skills`:

- `skill.md` loaded · under a bundle folder not loaded.
- The frontmatter `name: p-fmname` of `p-dirname` works only as an alias.
- Personal `c-pers` shadows project `c-pers`.

Side measurement — codex working folder `.agents/skills`:

- The Engram-shaped junction `engram-1234abcd-c-exact` has the same actual path as the extraRoots side, so it **merged into one line**.
- Two with the same name (`w-samename` · `w-samename2`) appeared as two lines in the prompt.
- ★Even the claude sync folder `synced/<uuid>/…` under the real `~/.agents/skills` (on this PC a junction to the shared skill folder) is picked up recursively, and **`docs` · `pdf` · `pptx` and so on appear in the prompt twice each**. claude reserves that folder name and skips it.

## Where it diverges from Engram's rules (Engram 규칙과 어긋나는 곳)

Comparison target = the skill row of TRD §1-2 「종류 판정」 (kind decision) · §1-3 5 collision · the §1-5 skill translation row. The gist of the current rules:

- A folder with `SKILL.md` directly inside = a skill (OS case rule · on Windows `skill.md` too).
- Reference = folder path.
- Name = frontmatter `name`, or the folder name if absent.
- Same name, different path = collision (red).

| # | Engram rule | claude | codex | What happens | Recommendation |
|---|---|---|---|---|---|
| **A** | `SKILL.md` lookup = OS rule (on Windows `skill.md` too — user 2026-10-17) | Matches | **Diverges** — exactly `SKILL.md` regardless of OS | A `skill.md` skill in a Windows library is visible on screen and claude loads it, but **codex drops it without a single error line** | **Narrow recognition to exactly `SKILL.md`** — it is the only name both CLIs load on every OS. It overturns the 2026-10-17 user decision, so **user confirmation is needed**. Alternative = keep recognition and, when attached to a codex-backend agent, raise a problem line (「codex 는 `SKILL.md` 만 읽는다」 (codex reads only the exact name SKILL.md)) |
| **B** | A skill inside a skill = a separate skill (if attached, that one folder) | Matches — one junction = one skill (the inside is not scanned) | **Diverges** — loads together every `SKILL.md` up to 6 layers under the attached folder (dot folders excluded) | **An inner skill that was not attached is loaded only in codex.** Even if names overlap, Engram's collision check cannot see it | **Expand the inner skills too** into the codex backend's 「실제로 받는 것」 (`library::resolve`) (what it actually receives), and run the collision check on that list too (since they really are loaded). If an inner skill is attached separately, it has the same actual path, so codex merges it into one — which matches Engram's 「같은 경로 = 처음 한 번」 (the same path = only the first time) |
| **C** | Does not look at frontmatter format (reads only the name) | Lenient — but a skill on the plugin path that wrote no description **does not appear in the model list** | **Strict** — violating the frontmatter block · YAML · `description` · no BOM · `name` ≤ 64 makes it **drop out** (error list · one TUI warning line) | A skill that is fine in the library is absent in codex, and in claude the model does not know it | During the scan, **validate `SKILL.md` by codex's rules** and put that skill's line in an 「오류」 (error) state (reason + line). Whether to make it block ② as an error (common to both backends · or codex only) is **a user decision** — claude's model cannot see it either, so common is consistent |
| **D** | Name = frontmatter `name`, or the folder name if absent | The identity is the **name of the junction folder Engram made** with `[^A-Za-z0-9_-]`→`-` substituted. The frontmatter `name` is a display name · alias | Same — but (1) whitespace is folded and an empty string counts as absent (2) the folder name is **the actual folder after following the junction** (3) if there is a manifest ancestor, the `<ns>:` prefix | A name like `Weird Name.v2 한글` is changed to `engram:Weird-Name-v2---` in claude. Names that cannot be Windows folder names, like `ns:x` (`:` `?` `*` `"` `<` `>` `|` `\` `/`), **cannot even create the junction itself**. If the library holds someone else's plugin folder, only the codex name becomes `ns:이름` | Name = frontmatter `name` trimmed · whitespace folded, and if empty, the **actual folder name** (the target if a junction). **Add a name check**: 1–64 characters · outside `[a-z0-9-]` (Anthropic's recommended shape — lowercase · digits · hyphen) is 「오류」 (error). If an ancestor of the actual folder has one of the three plugin manifests, a problem line 「codex 에서는 `<ns>:<이름>`」 (in codex it is the namespace-prefixed name) (or run the collision check on that name) |
| **E** | Collision = the same name (exactly the same) · different path | The junction names collide on Windows by **case-insensitivity** (`C-Case` · `c-case` cannot be placed in one folder), and names that become equal after substitution (`a b` · `a-b` · any two two-character Korean names) become **the same internal name** | Allows name overlap (both loaded · only the mention is ambiguous). The mention count is also counted in lowercase | With the current key, `Foo` · `foo` pass, but the volatile folder cannot be laid down | **Collision key = `lower(이름)` with `[^a-z0-9_-]` replaced by `-`.** If D's name check is added, the key reduces to 「소문자 이름」 (lowercase name) |

Side notes (not a rule change · good to know)

- **The volatile plugin shape was confirmed** — without a manifest `engram:<정션 이름>` · follows the junction · Base directory = junction path.
  - Do not put `skills\SKILL.md`.
  - Do not use `claude plugin validate` as a gate — if there is no manifest it fails and does not follow links. To use it, a line `.claude-plugin\plugin.json` `{"name":"engram"}` is needed.
- **If the user's global folder already points to the same actual skill, 「붙임」 (attaching) has no effect.**
  - claude skips the plugin side and it shows only under the bare name (a personal skill).
  - codex merges it into one because the path is the same.
  - On this PC, `~/.claude/skills` · `~/.agents/skills` both lead to the shared skill folder. If the library holds that folder, that skill is always on, and does not drop out even if detached.
- **Since codex does not block even when names overlap, the Engram collision check, as far as codex is concerned, is a matter of 「모델이 둘을 본다」 (the model sees both), not 「Engram 이 깔 수 없다」 (Engram cannot lay it down).** claude cannot lay it down because Engram's volatile folder is per name (the reason of TRD §1-3 5 as is).

## Method (방법) — reproduction essentials

The scripts are session scratch (`…\scratchpad\skill-recog\`) and do not remain. Only the essentials are kept.

- **Fixtures:** in one folder imitating the library, I made the 26 items of the table above. The junction targets were placed outside that folder.
  - File symbolic links could not be created because there was no `mklink` permission.
- **codex source:** I read closely a sparse clone of tag `rust-v0.156.1`.
  - `ext/skills/src/loader/{mod,discovery,host,namespace,host_merge}.rs` · `ext/skills/src/host_roots.rs`
  - `skills/src/{parser,selection,name_counts,mentions,model}.rs`
  - `exec-server/src/local_file_system.rs` (walk) · `utils/plugins/src/plugin_namespace.rs` · `exec-server-protocol/src/protocol.rs` (the three manifest paths)
- **codex measurement:**
  - Started `codex app-server` in the scratch `CODEX_HOME` (name `hc`).
  - Called `skills/list` once before and once after setting extraRoots. The `skills` · `errors` of the response are the basis for the decision.
  - The rendering of the working folder `.agents/skills` was seen with `codex debug prompt-input` — it emits the model input offline.
  - The trap proxy (`HTTP(S)_PROXY` → log then 403) blocked one connection attempt each to `chatgpt.com` · `github.com`. There was no model request.
- **claude docs:** <https://code.claude.com/docs/en/skills> (「How a skill gets its command name」 · rejected characters in names · broken YAML = loaded without fields · overlap table) · <https://code.claude.com/docs/en/plugins-reference> (manifest optional · plugin name = `--plugin-dir` folder name · `skills/` = `<name>/SKILL.md` one each · name rules).
- **claude bundle:** from `claude.exe` (246 MB · a single bun executable) I picked out and read the strings around `"SKILL.md"`.
  - Personal · project loader = after `readdir`, stat · readFile of `join(dir, 이름, "SKILL.md")`.
  - Plugin loader = the same shape, internal name is `${플러그인}:${폴더 이름.replace(/[^a-zA-Z0-9_-]/g,"-")}`.
  - Display name = frontmatter `name` (with plugin prefix attached) · alias = the frontmatter name without `:` · realpath duplicate skipping.
  - Obfuscated function names change between builds, so they are not quoted.
- **claude measurement:**
  - Execution environment:
    - `CLAUDE_CONFIG_DIR` = scratch · `ANTHROPIC_BASE_URL` = a local fake Messages API (records **the request body only** · headers not recorded · fake key) · `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`
    - The parent session's environment variables (`CLAUDECODE` · messaging socket etc.) were removed.
    - `-p --output-format stream-json --verbose --no-session-persistence --debug-file` · two `--plugin-dir`
  - Round 1 = the init event (`skills` · `slash_commands` · `plugins`) + the skill list in the request body + the debug log.
  - Round 2 = the fake server sent back **10 Skill tool calls** (name candidates) as its first answer. Which names resolve was measured from the `tool_result` (success/「Unknown skill」) carried in the next request and the injected 「Base directory」.
  - `claude plugin validate --json` 4 times (all offline).
  - **0 real model calls.**
- **Real-config unchanged check:**
  - `~/.codex` — session files 1589 · `history.jsonl` 150 lines · `session_index.jsonl` 513 lines · `config.toml` hash · `skills` list, same before and after.
  - `~/.claude` — the `settings.json` · `skills` · `plugins` hashes are the same.
  - `~/.claude.json` · `~/.claude/projects` · `~/.codex/*.sqlite-wal` were written by other sessions in the meantime. 0 occurrences of the scratch path string in all files changed since the test.

## Limits

- **Case-sensitive file systems (Linux · macOS case-sensitive volumes) not measured.**
  - Per source, claude follows that OS's open rules, so there only `SKILL.md` would be loaded (likely).
  - codex is an exact match regardless of OS (certain from source).
- **File symbolic link `SKILL.md` not measured** (permission). codex drops it per source, and claude's stat follows the link so it would be loaded (uncertain).
- **Which skill a real model picks was not measured.** claude's name resolution went through the real CLI code path, but the calls were made up by the fake server. The ambiguity handling of the codex `$이름` mention was seen in source only.
- **The rule by which a skill without a description drops out of the model list on claude's plugin path** was measured in only three cases, and the code condition could not be pinned.
- **claude personal · project path junctions**, `.claude-plugin` (`@skills-dir`) inside the skill folder, dynamic discovery of subfolder `.claude/skills`, the effect of `agents/openai.yaml` — not measured (docs · source).
- **codex interactive TUI not measured** — `skills/list` and `debug prompt-input` use the same loader (source).
- Version-bound: Claude Code 2.1.284 · codex-cli 0.156.1. claude is read from bundle strings, so it can easily change in the next version.
