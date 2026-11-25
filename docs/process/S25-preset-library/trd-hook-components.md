# TRD — S25 step 22: hook components (`S25: feat(asset-library,agent): hook components`)

| Item | Value |
|---|---|
| **Status** | **Draft 7 (2026-11-24) — reviewed deep, PASS (2026-11-25)** — decisions and boundaries only. User decisions U1–U5 (§9.1); open items O2–O8 (§9.3). Canonical ADRs: ADR-0359 (no schema check — Engram checks only what its assembly breaks) · ADR-0367 (asset-library is backend-blind; per-backend choices are port answers) · ADR-0334 (resolution in asset-library, flag translation in backend `materialize`) · ADR-0316 · ADR-0328 (claude: one merged `--settings`) · ADR-0341 (preset sections, launch notices once per agent — one exception here, §3.3) · ADR-0374 (a missing item is skipped with a notice) · ADR-0353 (codex fail-closed before any write) · ADR-0216 (decision 1: no silent overriding; the session-id hook stays removed — amended here, §10) · ADR-0210 (superseded; why per-definition trust failed before) · ADR-0339 · ADR-0386 (preset / library command surface) · ADR-0307 · ADR-0155. |
| **Parent TRD** | [`trd.md`](trd.md) §5-3 item 22 (`trd.md` line 990, spelled `22\.`) · §1-7 · the end-of-server-stage stop after item 22. |
| **Pattern** | [`trd-file-commands.md`](trd-file-commands.md) (step 10b) for form; skills are the template for a hook kind at every layer (scan → preset section → resolve → plan → materialize). |
| **Facts** | Claude Code 2.1.330, codex-cli 0.171.0 (source at `rust-v0.171.0`): [`hook-loading-claude-codex-2026-11-24.md`](../../research/hook-loading-claude-codex-2026-11-24.md) · [`codex-hook-trust-per-run-2026-11-24.md`](../../research/codex-hook-trust-per-run-2026-11-24.md) (「measure-2」: JSON route, terminal route, hash, override replace rules, budget, guard) · [`hook-failure-and-resume-2026-11-24.md`](../../research/hook-failure-and-resume-2026-11-24.md) (「measure-3」: claude bad-hook containment, codex resume) · [`hook-trust-layers-2026-11-24.md`](../../research/hook-trust-layers-2026-11-24.md) (「measure-4」: M-A `experimentalApi` not needed, M-B session-layer state overrides per key and field, M-C terminal behaviour of untrusted hooks) · codebase facts [`hook-components-codebase-2026-11-24.md`](../../research/hook-components-codebase-2026-11-24.md), checked against the code at `7528e4ae` (§11). The hash rules of §2.8 were re-read from the codex source (`hooks/src/engine/discovery.rs`, `protocol/src/hooks.rs`, `config/src/fingerprint.rs`). |
| **Reference files to fix** | **docs** — `docs/reference/architecture.md` (its asset-library line 「reads, checks and resolves the instruction · skill · settings · preset files」 gains hook) · `docs/reference/structure/asset-library.md` (Modules: `format/hooks.rs`; the hook kind in the scan and in the file operations; `LaunchPlan.hooks`; the section list; `library.folders` hook marker) · `docs/reference/structure/agent-backend.md` (claude hook layers; codex hook assembly, thread config, trust entries, the post-open check and the run notices; an ADR-0216 note in 「Session id recovery」) · `prompts/engram-help.md` (lines 22 and 258 kind enumerations, the kind table, the example preset, the tag rule 「instructions, skills and hooks」, line 301 folders marker, line 302 `--kinds` list, the section words of `preset.reorder` / `save` / `check` and 「hooks omitted = they stay」, line 341 lone marker (if P4), the hooks paragraph of §4) · `builtin/instructions/priming.md` root line (「Instruction · skill · hook · settings · preset files」). **code docs** — asset-library `src/lib.rs` header · agent `launch.rs` (`LaunchPlan`, `LaunchArtifacts`, `notice_text`) · `backend/settings_check.rs` (`ModeLoads`, `NoticeCodes`, `CommandLine.config_args`, `command_line_fits`) · `transport/mod.rs` `LinkResolution` · `manager.rs` `Activated` / `LaunchNotices` · codex `transport.rs` `handshake` (「The order is the contract」) and `AppServerOpen` · codex `launch.rs` module and `materialize` docs · codex `settings.rs` header (hash and key rules from 0.171.0) · codex `cmdline.rs` module doc · codex `protocol.rs` thread params · codex `mod.rs` ADR-0216 note, `hook_override_value` doc and the `passthrough_overrides_key` block · claude `launch.rs` settings-file doc. |
| **Line-pointer basis** | Code at `7528e4ae` (step 10b Implemented). Code is cited by file + symbol. |

## 0. Scope

**What and why.** A hook becomes a library component like a skill: a folder holding a hook definition (`hooks.json`) and its scripts, listed in a preset's new `hooks:` section, handed to claude and codex the way each loads hooks. Engram assembles and hands over; the CLIs apply their own rules (ADR-0359). User 2026-11-23: 「없는것보다는 있는게 좋겠지? 유저가 하드코딩하기 딱 좋은게 훅인데」 · 「기존 인프라에 그대로 끼고 우리쪽은 거치고 클로드 코덱스쪽으로 에러를 넘기는거라서 많이 수정은 안한다고 생각되는데 어쨋든 쭉 진행하면 될듯」. Today a claude hook can already pass through a settings preset's raw text (ADR-0328); codex raw `-c hooks.*` lines pass too but are skipped silently (untrusted).

**PRD relation.** Parent PRD [`prd.md`](prd.md). Item 22 (user, 2026-11-23) adds a fifth asset kind beside instruction · skill · settings · preset and follows the PRD's rules for kinds (backend tags — G29, order per preset — G51, the problem list, notices, ADR-0374 skipping). **Server stage**: no screen. It runs after 10b and closes the server stage (then the stop and the 「서버 단계 수용」 pass of parent §4).

**In** — asset-library: the hook kind (scan, catalog, file operations), the `hooks:` section, the shape check, resolve, `LaunchPlan.hooks`. agent, neutral: `HookRef`, the placeholder rewrite and its path rule, port fields, the run-notice path from transport to caller, the `hooks` section word and `Hook` / `HookFile` kinds on the existing commands. claude backend: hook layers in the merged `--settings`. codex backend: the one hook assembly (components + folded raw lines + carried state entries), terminal lines, JSON thread config, per-run trust, the post-open check, the refusal of pass-through hooks lines (U5). Help, priming, ADRs.

**Out** — the screen (client stage, §8); a preset-editor budget warning (user: the refusal text is the guidance); any schema check of events, matchers or handler fields (ADR-0359); a hook-creation verb (an LLM writes files with its own tools and runs `library.refresh`, as for skills); Engram's own past codex session-id hook (U3); the bypass flag; any write to `~/.codex` or `~/.claude`; Windows shell differences between backends (M6); a per-codex-version hash probe (L17 — considered, not taken, §9.2 T9); new wire messages; the catalog stays 9.

## 1. Boundaries and ownership

- **All per-backend handling lives in agent `backend/claude` and `backend/codex`; asset-library stays backend-blind** (ADR-0367 · ADR-0334 · CLAUDE.md 「백엔드 확장」). That covers the claude settings layer, every codex line, the codex TOML reading, trust entries, hash, guard, budget and the post-open check. asset-library sees hooks only as neutral `HookRef`s, neutral agent functions and port answers. Enforced by the §8-3 gate of `trd-asset-library.md` (`rg -i '\b(claude|codex)\b'` over `crates/engram-dashboard-asset-library/src` — no branch hits), re-run in every unit that touches asset-library.
- **asset-library** owns the component format: finds hook folders, reads and shape-checks `hooks.json` at scan (§3.1), resolves the section with tags, skips a missing hook with a notice (ADR-0374), hands `HookRef`s to the plan. Whether a backend · mode loads hooks is `ModeLoads.hooks`; its omission notice code is `NoticeCodes.hooks_omitted`.
- **agent, neutral (a new `backend/hooks.rs`)** owns `HookRef`, the placeholder rewrite and the placeholder path rule (§2.4) — one place both backends call.
- **platform** gains two generic functions (ADR-0266): `platform::fs::forward_slash_text(&Path) -> String` (the path with `/` separators on Windows, unchanged elsewhere) and `platform::os::windows_paths() -> bool` (the host uses Windows path rules). Codex's synthetic path string and the `commandWindows` choice stay in `backend/codex` and read the predicate (ADR-0004).
- **backend/claude** turns hooks into one more layer of the merged settings (§2.5).
- **backend/codex** owns the **hook assembly**: one function in `cmdline.rs` (named here `hook_assembly`) that takes the raw `-c` lines **with their origins** (§2.3), the pass-through arguments, the plan's hooks and the mode, and returns the non-hook lines, the hook lines or thread-config object, the trust entries, the notices and the refusals — each notice and refusal naming the hook or the settings asset (and line) it came from. `command_line_fits` (check, through the port) and `materialize` both call it, so what `check` counted is what goes on the line (§2.3). It writes only what codex reads: codex's 14 events (`HookEventsToml` at `rust-v0.171.0`, a constant in `backend/codex` pinned by the lane), a group's `matcher` and `hooks`, and the handler fields of §2.8.
- **codex transport (`transport.rs`)** sends the thread config on `thread/start` and `thread/resume` and runs the post-open check inside the handshake; **`transport/mod.rs`** (`LinkResolution::Ready` carries notices) and **`manager.rs`** (`deliver_notices` appends them to the reply's notices) carry the verdict (§3.3).
- **daemon · CLI · shell · frontend**: regenerated bindings and help only.

## 2. Data shapes

### 2.1 Hook folder (library)

```
team/hooks/notify/            ← the hook item (its name = the folder name)
  hooks.json                  ← the marker and the definition
  notify.js · guard.js        ← scripts, run in place
```

- **Marker** — a file named `hooks.json` directly in a folder below a mount's top makes that folder a hook (`ItemKind::Hook`, and `ItemKind::HookFile` for the file, as `Skill` / `SkillFile`). The name compares by the OS rule (platform `same_path`), as `SKILL.md`. A `hooks.json` directly in a mount's top makes no hook and stays unlisted.
- **Precedence is a folder-level rule** — a folder holding both `SKILL.md` and `hooks.json` is a skill; its `hooks.json` is an ordinary file. The per-file `FileKind::of` cannot decide this alone, so the scan, `ops.rs` `reclassify` and `reread_file` resolve the folder's kind from both marker names (one function in `catalog.rs`); a `SKILL.md` added to a hook folder turns it into a skill, removed turns it back.
- Everything else in the folder classifies as today. Nested hook folders are their own items.
- `hooks.json` = the claude/codex shared shape `{"description"?: …, "hooks": {"<Event>": [{"matcher"?: …, "hooks": [{"type": "command", "command": …, …}]}]}}`. Engram carries only the `hooks` object.
- **Folder items are `Skill | Hook`** wherever skill-only code treats a marker folder as one item (traced in `asset-library/src`): `catalog.rs` `apply_scan` (the absence anchor — `ItemKind::Skill` → own path — and `skill_file_key` become marker-generic); `ops.rs` `reclassify` (a gained or lost marker changes the kind), `follow_moves` (re-reads the marker of a moved folder item, so a renamed hook keeps its kind and name), `refuse_into_itself` (a hook folder is refused as a move or copy target inside itself); `lib.rs` `folders()` marks hook folders. Each place is listed in P1. **Not in P1**: `ops.rs` `Target.skill_folder` / `lone_skill_file` / `leaves_skill` stay skill-only — they are ADR-0339's lone-marker mechanism and drive `skill-unconfirmed`; generalizing them is O4 (P4, with its own code).
- **Wire** — agent `LibraryKind` gains `Hook` and `HookFile` (PascalCase, as the wire spells kinds — help line 302); `library.list --kinds HookFile` matches a hook folder's `hooks.json` row, never `Hook` (the ADR-0386 decision 4 rule for `SkillFile`). `library.folders` rows gain `hook: bool` beside `skill: bool` (`commands.rs` `FolderRow`), so the LLM path sees what the client stage will mark.
- The lone-marker confirmation (renaming `hooks.json` or moving it out of a referenced hook folder) is O4.

### 2.2 Preset section

```yaml
kind: preset
version: 1
description: Web team rules, review skill and hooks
skills:
  - team/skills/review
hooks:
  - team/hooks/notify
  - {path: team/hooks/format-on-edit, backend: claude}
```

- `ListName::Hooks` (key `hooks`), after `Skills` in `ListName::ALL`; `takes_tags` = true (M5). An entry naming another kind answers `wrong-kind`.
- `PresetSection` gains `hooks`; `preset.reorder --section hooks`; views and drafts carry the list. `preset.save` / `preset.check` gain an optional `hooks`: **omitted = the file's hooks stay** (save) and are judged as they are (check) — the `folders` precedent, so a caller following the old help cannot wipe the section.
- **Order** — plan order = preset expansion order: a preset's common entries, then its entries for the agent's backend (G51), presets in attach order. The same hook folder reached twice is carried once, at its first position.
- **Missing** — a hook entry whose folder (or `hooks.json`) is not there is skipped and named in the launch notice, as any missing item (ADR-0374 decisions 1–3, ADR-0386 decision 11). A hook that is there but cannot be read, or fails the shape check, blocks (problem ②, ADR-0374 decision 4).

### 2.3 Plan, artifacts and the one counting seam

- `HookRef` = `path` (library reference) · `dir` (absolute hook folder) · `definition` (the parsed `hooks` object, not rewritten). `LaunchPlan.hooks: Vec<HookRef>` in plan order; empty in a Noop plan. The check's `stamp` covers each `hooks.json` read (`PrepareError::Stale` on change).
- Hooks are **not copied** into the volatile folder; scripts run in place (as skills).
- **`CommandLine`** (port input) gains `hooks: &[HookRef]` and `config_origins` — one neutral origin per argument of `config_args`, at the same index (asset-library cannot know how a backend pairs its arguments): the settings asset's library path and the field's line. asset-library builds `config_origins` in the same loop that builds `config_args` (which skips refused values — `filter_map(… .ok())`), so origins and pairs cannot drift. `RawSetting` (plan) gains `source` (the settings asset's library path) so `materialize` has the same origins. `CommandLine.config_args` stays what `command_line_args` returned for the raw values — **all** lines, hooks lines included (doc fixed accordingly). `command_line_args` (codex) keeps returning a hooks line unchanged and refuses one it cannot fold with `codex-hooks-unreadable {index, line}`; that lands as the **settings asset's** problem ② at the field's line through `check_settings`, as other raw-line refusals do.
- **One seam** — `command_line_fits` and `materialize` call `hook_assembly` (§1) with `config_args`, `config_origins`, `extra_args`, `hooks` and the mode. It splits the hooks lines out of `config_args`, applies codex's last-wins rule over all of them, folds the effective result (§2.9), adds the components, builds trust entries, and returns the line pieces. **Terminal** counts the non-hook lines plus the built hook lines and state line; **JSON** counts only the non-hook lines (the hooks leave the app-server line for the thread config, which has no length limit) — no over-count. **"Engram assembles hooks" is decided on the written result, not the inputs**: Engram writes hook output only when at least one group (a component's or a folded raw group, after leaving-off) or one carried state entry (§2.9) would be written. With nothing written: no `hook_args`, no `hook_config` (never an empty `config.hooks` in JSON mode — it would still wipe the app-server line's `hooks` table) — that is all 「nothing written」 means. The U5 refusal does not depend on what is written; it is decided on the inputs (§2.6). **Raw settings hooks lines are always consumed** (`hooks.<Event>=…`, `hooks={…}` and raw `hooks.state`): the assembly folds, carries or leaves each one off with a notice, and never re-emits one as a raw `-c` line or in thread config, whether or not anything is written — so a raw handler Engram left off (e.g. a settings asset whose only hook is a `Stop` → `mcp_tool` handler) never reaches codex. A settings asset with no hooks lines is untouched as before. So a component or a raw line whose handlers are all left off (e.g. only claude events) writes nothing; its `codex-hooks-left-off` notices are still raised.
- `LaunchArtifacts`: `config_args` now means 「the raw settings' lines minus their hooks lines」 — always, whether or not hook output is written (T3 changed accordingly); new codex fields `hook_args: Vec<String>` (terminal, placed right after `config_args`, before the MCP attachment and `developer_instructions`) and `hook_config: Option<Value>` (JSON, §2.7). claude: `settings_file` is written when there are raw settings **or** hooks.
- **`command_line_fits` answers notices** — its `Ok` becomes `Ok(Vec<Notice>)` (backend-neutral values asset-library adds to the plan's notices without reading them): the codex assembly's `codex-raw-hooks-trusted` and `codex-hooks-left-off` (§3.3). claude answers `Ok(vec![])`. asset-library's note loop calls `fits` up to three times (named form, count form, without a note — `launch.rs`); **only the notices of the call whose result is kept** are added.
- **Refusal attribution** (neutral — asset-library reads only these fields): a refusal whose `args` name a `hook` (library path) is carried on that hook's entry; one whose `args` name a `source` (settings asset path, with `line`) — a folded raw group's line, a carried state entry the guard refuses — on that settings asset at its line; anything else (the budget, the U5 pass-through refusal) on `command-line:<backend>` as today.

### 2.4 Script paths — the placeholder (M2)

- A command may use `${CLAUDE_PLUGIN_ROOT}` or the unbraced `$CLAUDE_PLUGIN_ROOT` (followed by a character that cannot continue a variable name, or the end). Engram replaces both in **every string value** of the definition with the hook folder's absolute path from `platform::fs::forward_slash_text` (`D:/Library/hooks/notify` — the spelling claude itself substitutes on Windows; works in Git Bash, PowerShell and cmd; half the cost on the codex line).
- **Path rule** — when a definition uses the placeholder, a hook folder path holding a backtick anywhere, or any `$` that is not the last character of a path component (a component = the text between two `/` of the forward-slash path, or after the last one; off Windows also `"`, `\` or a control character), is refused: problem ② `hook-path-unsafe {hook, path}`. Inside the double quotes the help recommends, Git Bash expands `$(…)`, `${…}`, `$name`, backticks and the special parameters (`$$` = its PID, `$?`, `$!`, `$1` …), and PowerShell 7 expands `$(…)` / `$name`; the codex hash would be computed over the substituted text. A `$` that ends a component (followed by `/` or the end of the path) stays literal in both shells, so a hidden-share library (`\\srv\team$\library` → `//srv/team$/…`) passes, while `$$`, `$?`, `$1`, `$name`, `${` and `$(` are refused (CLAUDE.md 「최소한만 막기」). The rule lives in the rewrite function itself (`backend/hooks.rs` refuses to rewrite an unsafe path), and asset-library calls the same rule at check (the characters per OS through platform). A placeholder written in single quotes or unquoted is the author's risk (ADR proposal 1).
- **Not supported** — `CLAUDE_PLUGIN_DATA` (in settings hooks claude leaves it unset or another plugin's folder — hook-loading A2; codex sets no plugin variables for session-flags hooks), `$CLAUDE_PROJECT_DIR` passes through as written. So a Claude plugin's hook folder works as is **only** when it uses `CLAUDE_PLUGIN_ROOT` alone; help says so.
- The path is the library path — stable across launches — so a codex hash does not change per launch.

### 2.5 claude — one more settings layer (M3, O2)

`settings.json` = `merge(raw settings in attach order, then one layer {"hooks": <rewritten definition>} per hook in plan order)` with the ADR-0328 merge:

```json
{
  "allowedMcpServers": [{"serverName": "engram"}],
  "hooks": {
    "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "node \"D:/Library/hooks/notify/guard.js\"", "timeout": 30}]}],
    "SessionStart": [{"hooks": [{"type": "command", "command": "node \"D:/Library/hooks/notify/notify.js\" start"}]}]
  }
}
```

A settings preset's own `hooks` groups come first in each event, the components' after; identical groups collapse. A wrong-typed field drops only that handler, silently (measure-3 M1a; §3.1). Both claude modes load hooks.

### 2.6 codex terminal — lines (M4, M7)

After the non-hook `config_args`: one line per event holding every group of that event, then **one** `hooks.state` line (codex replaces a repeated `-c` path — measure-2 M4). Events in sorted order:

```
-c hooks.PreToolUse=[{"hooks"=[{"command"='node "D:/Library/hooks/notify/guard.js"',"timeout"=30,"type"='command'}],"matcher"='Bash'}]
-c hooks.SessionStart=[{"hooks"=[{"command"='node "D:/Library/hooks/notify/notify.js" start',"type"='command'}]}]
-c hooks.state={"C:\\\u003csession-flags\u003e\\config.toml:pre_tool_use:0:0"={"enabled"=true,"trusted_hash"='sha256:…'},"C:\\\u003csession-flags\u003e\\config.toml:session_start:0:0"={"enabled"=true,"trusted_hash"='sha256:…'}}
```


- **TOML writer** (in `cmdline.rs`, no `toml` crate for writing): every key inside a value is a quoted basic key (`"command"`) — never bare, so no key can close a table or start a comment; strings as literals `'…'` unless they hold `'` or a control character, then basic strings with escapes; a control character is never raw (cmd cuts the line at a real LF); integers; booleans; arrays; inline tables. Because only what codex reads is written (§1) and every written field is type-checked first (§2.8), no `null` or out-of-range value reaches the writer — a claude-only field holding one is simply not written.
- **Events** — only codex's 14 events are written (`PreModelRequest` · `PostModelRequest` · `PreToolUse` · `PermissionRequest` · `PostToolUse` · `PreCompact` · `PostCompact` · `SessionStart` · `SessionEnd` · `UserPromptSubmit` · `SubagentStart` · `SubagentStop` · `Stop` · `Interrupt`, exact spelling); codex ignores any other name (`HookEventsToml` denies no unknown field). A handler of another event (e.g. claude's `Notification`) is left off with `codex-hooks-left-off {hook | source, why: "event"}` — no line, no entry, no expected key. These names are bare keys, so the path needs no quoting and no event-name refusal exists.
- **Guard** — every final line passes `text_is_cli_safe` + an even `"` count (as `cmdline::config_args`). The trust key holds `<` `>`, so it is a basic key with `\u003c` / `\u003e` (measured through `cmd.exe /c codex`, measure-2 M6). A hook whose line the guard refuses (e.g. `%USERPROFILE%`, `| tee`, `> out`, an odd quote count) is problem ② `codex-hook-line-refused {hook | source, why}` on that hook's entry (a folded raw group: on the settings asset at its line, §2.3); `Err` in `materialize` (ADR-0353).
- **`enabled = true` in every entry** — neutralises a stale user-layer `enabled = false` on the same session-flags key (R3): the session layer overrides per key and field (measure-4 M-B); ≈ 14 units per entry.
- **Budget** — `cmdline::fits`, 8191 with the shim margin. Today's line with the built-in priming ≈ 2542 units (measure-2 M5, a replica — P3 pins the Rust number, P5 updates it for the priming line); a plain hook ≈ 330 units for the first and ≈ 280 for each more on one event (measure-2's 293 / 244 plus the quoted keys and `enabled = true` — an estimate; P3 pins real numbers), so about 18 typical hooks fit. Over budget = the existing fail-closed refusal (ADR-0353), no truncation.
- **Budget policy (M7, user 2026-11-24)** — no proactive warning; (1) with hook lines on the line, the problem ② text reads 「codex terminal command line limit exceeded — reduce hooks or use JSON mode」 (`args` say hook lines are counted); without hooks today's text; (2) one help line on the terminal-mode limit (§4); no preset-editor warning (the refusal is the guidance; JSON has no limit).
- **Only trusted hooks go on the line** (F7) — a handler Engram cannot trust (§2.8 「Which handlers」) is left off, with notice `codex-hooks-left-off {hook | source, why}`; a left-off raw handler's settings line is not re-sent either (§2.3). **Indexes** = positions in the arrays Engram writes: a group that leaving-off empties is not written, and handlers left off are not written; codex assigns `group_index` / `handler_index` over what it receives before its own skips (`discovery.rs`), so the two counts agree. So codex's startup review screen never shows for Engram's own lines, and never writes `~/.codex/config.toml` — measure-4 M-C: `exec` skips an untrusted hook silently, while the TUI shows a blocking picker whose 「Trust all」 writes session-flags keys into the real `~/.codex/config.toml`; hence every hook on the terminal line has its state entry on the same line. A hash drift (a codex release changing the rules) is not detectable at launch in terminal mode (T9); help says: if codex shows a hook review screen for Engram hooks, do not trust there — update Engram.
- **Pass-through hooks lines — refused when the launch has hook inputs (U5, user decision 2026-11-25)** — when a codex launch has any hook component attached for codex or any settings asset with a hooks line (`hooks.<Event>=…`, `hooks={…}`, `hooks.state=…`) — regardless of whether Engram ends up writing anything — a pass-through argument in the agent's `extra_args` that sets `hooks` or `hooks.*` refuses the launch: problem ② `codex-passthrough-hooks {arg}` on `command-line:codex`, text 「move this hooks line into a hook component or a codex settings asset」. **Detection reads `extra_args` the way clap reads codex's `-c` / `--config`** (`utils/cli/src/config_override.rs`): the value is the next element after `-c` or `--config`, the rest after `--config=`, or the rest after `-c` with one leading `=` stripped (covers `-c=k=v` and `-ck=v`); nothing after `--` is read. The key (the value before its first `=`) is trimmed and refused when its first `.`-segment is `hooks` (segments untrimmed, as codex splits them). With neither, pass-through hooks lines pass untouched as today. The trigger is on the inputs, not the written result, so no interplay remains: a pass-through line never meets a consumed settings hooks line (e.g. a pass-through `hooks.state={K={enabled=true}}` would otherwise survive when a later settings `hooks.state={}` is consumed and nothing is written). Reason: the hook lines go after `extra_args`, so Engram's lines would replace them (ADR-0216 decision 1 — no silent overriding), a pass-through event Engram does not write would stay on the line untrusted (terminal: codex's blocking picker and its write to `~/.codex/config.toml`), and the JSON thread config wipes them all. The alternative (fold every spelling into one override sequence) is O6.

### 2.7 codex JSON — thread config (M4)

When Engram writes hook output (§2.3), `thread/start` **and `thread/resume`** carry `config: {"hooks": <object>}` — otherwise no `config` at all, never an empty `hooks` object — one `hooks` key (events + `state`; never a dotted `"hooks.…"` key beside it — codex applies request keys in hash-map order):

```json
"config": {"hooks": {
  "PreToolUse":   [{"matcher": "Bash", "hooks": [{"type": "command", "command": "node \"D:/Library/hooks/notify/guard.js\"", "timeout": 30}]}],
  "SessionStart": [{"hooks": [{"type": "command", "command": "node \"D:/Library/hooks/notify/notify.js\" start"}]}],
  "state": {
    "C:\\<session-flags>\\config.toml:pre_tool_use:0:0":  {"enabled": true, "trusted_hash": "sha256:…"},
    "C:\\<session-flags>\\config.toml:session_start:0:0": {"enabled": true, "trusted_hash": "sha256:…"}
  }
}}
```

- The object replaces the whole `hooks` table of the app-server line (measure-2 M1 a2): the raw settings hooks lines are folded into it (§2.9) and never go on the app-server line, also when no object is sent (§2.3); a pass-through hooks line is refused whenever the launch has hook inputs, so none reaches the app-server line beside Engram's hooks (U5, §2.6).
- **Resume (measure-3 M2)** — with the same config the resumed thread's hook is trusted and SessionStart fires with `source: "resume"` on the first `turn/start`; without it the thread has no hooks (not saved with the thread). So Engram sends it on every resume; a fresh-only hook uses `matcher: "startup"` (passed through; help mentions it).
- `ThreadStartParams` / `ThreadResumeParams` gain an optional `config` (absent = byte-identical).
- **`experimentalApi` — not needed (measure-4 M-A)**: `thread/start` / `thread/resume` `config.hooks` and `hooks/list {threadId}` work without it (`config` carries no experimental flag). Engram's handshake stays as is; the lane goes through it (§5).

### 2.8 Trust entries and the hash (U1, U2, M4)

- **Key** = `<session-flags source>:<event label>:<group index>:<handler index>`, indexes 0-based in the arrays Engram writes (§2.6 「Indexes」). Source spelling: `C:\<session-flags>\config.toml` when `platform::os::windows_paths()` (codex hard-codes `C:\`), `/<session-flags>/config.toml` otherwise (codex source; pinned by the lane). Event label = codex's `hook_event_key_label` snake form (`SessionStart` → `session_start`), defined only for the 14 events.
- **Hash** — codex's algorithm (`discovery.rs` normalization → `hook_hash` → `fingerprint.rs` `version_for_toml`), applied per `type: "command"` handler:
  - `command` = `commandWindows` (also read under its alias `command_windows`) when set and `windows_paths()`, else `command`; `commandWindows` is dropped;
  - `timeout` always present: `SessionEnd` / `Interrupt` → (given or 1) clamped to 1..3; every other event → (given or 600), at least 1 (so `0` → `1`);
  - `async` always present, **as configured** (default false) — codex runs an async `SessionEnd` hook synchronously but hashes the configured value (the review's 「SessionEnd async → false」 is the run, not the hash — checked in `discovery.rs`);
  - `statusMessage` only if set;
  - `additionalContextLimit` only for `PreToolUse` · `PostToolUse` · `SessionStart` · `UserPromptSubmit` · `SubagentStart`, and only if set and not 2500;
  - every other handler field is dropped (codex reads only the listed fields);
  - **matcher**: kept as written for `PreToolUse` · `PermissionRequest` · `PostToolUse` · `SessionStart` · `SessionEnd` · `SubagentStart` · `SubagentStop` · `PreCompact` · `PostCompact`; **dropped** for `PreModelRequest` · `PostModelRequest` · `UserPromptSubmit` · `Stop` · `Interrupt` (`matcher_pattern_for_event`);
  - then `{event_name, matcher?, hooks: [handler]}` with all object keys sorted, compact JSON, sha256, `"sha256:" + lowercase hex`.
  The agent crate gains `sha2` (already locked at 0.10.9 through asset-library — no new package; reported). Live fixtures in the lane cover each rule (§5).
- **Which handlers get an entry** — `type: "command"` with a string `command` that is not blank (codex skips a blank one) and fields Engram can normalize: string `commandWindows` / `command_windows`, integer `timeout` ≥ 0, boolean `async`, string `statusMessage`, non-negative integer `additionalContextLimit`; and in a group whose `matcher` is absent or a string. A group with a non-string `matcher` is left off whole; any other handler — `prompt`, `agent`, an unnormalizable field — is left off with `codex-hooks-left-off {hook | source, why}` (§2.6). Reason for the strictness: on a type error codex drops the **whole** session-flags `hooks` table (`load_toml_hooks_from_layer` warns and returns none), so one bad handler would silence every Engram hook of that run. **`mcp_tool` is left off by choice** (a scope cut): codex normalizes and hashes it too (`server`, `tool`, `input`, `timeout`, `statusMessage`), but that path was not measured.
- **Per run, nothing persisted** (U2) — `~/.codex/config.toml` is never written (QA compares it). No bypass flag, no review prompt (U1).
- **Accepted consequence (F4, held for the ADR)** — the hash is recomputed from the current `hooks.json` at every launch, so any edit — a teammate on a shared mount, a sync, an LLM writing files or calling `preset.save` — is trusted at the next launch without review. This is what 「우리건데?」 means here: trusted = attached by anyone with library and command access, agents included; content changes are not re-reviewed (the same as claude, which has no hook trust at all).
- **ADR-0216 tests** — `the_terminal_spawn_no_longer_registers_the_session_start_hook` and its two siblings keep asserting that a spawn **with no hook output written** (§2.3) carries no hook (doc: 「Engram's own session-id hook stays removed — ADR-0216, U3」); a new test asserts a component's line carries only the user's command.

### 2.9 codex settings assets with raw hooks lines — merged (U4)

- **What is read** — in each codex raw settings value, a line whose `-c` path (whole key trimmed, then split on `.`, segments case-sensitive and untrimmed — as codex's `parse_overrides` / `apply_toml_override` do) is `hooks` (a table) or `hooks.<Event>` (an array of groups) or `hooks.state` (a table). A deeper path (`hooks.<Event>.<more>`, `hooks.state.<x>`) cannot be folded → `codex-hooks-unreadable`. The codex backend parses the values with `toml` (only there — §1). Each line keeps its origin (settings asset, line) through the assembly.
- **Raw lines first follow codex's own rule (F2)** — over **all** settings lines in attach order, then file order: the last write to a path wins, and a `hooks` table line replaces the whole table (its `state` member included). The effective result is what codex alone would have read. (Pass-through lines do not enter this sequence — they are refused whenever a settings asset has a hooks line or a hook component is attached, U5.)
- **Then merge with the components (U4)** — per event: the effective raw groups first, then the hook components' groups (plan order) — the same order as claude's merge (§2.5).
- **State entries (F1, measure-4 M-B)** — the session-flags `hooks.state` does not wipe the user layer; it overrides **per key and per field** (an entry for a user-layer key with `enabled = true` re-enables a hook the user disabled). So Engram **generates** trust entries only for the session-flags keys it assembled, never for another source. Other-source entries appear in Engram's state line only when the **effective** settings result (after the last-wins rule above — so an entry a later line cancelled is not revived) contains them: they are **carried verbatim** (through the guard and the budget), because Engram's one session state line replaces the user's raw session line (last wins inside the session layer). A carried entry alone (no group written) still counts as hook output (§2.3). Keys are **trimmed before classifying**, as codex trims state keys (`config_rules.rs`): an entry equal to a session-flags key after trimming is dropped (positions change in the combined list) with a warn, and Engram never emits two keys equal after trimming. The lane checks that a user-layer trusted hook still runs, a user-layer `enabled = false` hook stays off, and a carried entry keeps its effect with Engram's state present (§5).
- **Trust** — folded groups are trusted like component groups (U1). A raw hooks line that is skipped silently today **runs** after this step; the launch notice `codex-raw-hooks-trusted {settings: [{source, digest}]}` names each settings asset whose hooks Engram trusted, with a digest of its folded groups — so the notice fingerprint changes when an already-named asset's trusted set grows (a raw line is hidden inside a settings asset, unlike a component listed in the preset). `notice_fingerprint` (ADR-0341, agent `launch.rs`) already hashes each notice's code plus its args, so the digest changes the fingerprint as is — the fingerprint function is not edited. `preset.check` cannot name it — a preset draft has no agent, hence no backend (F3 partly).
- **Always consumed** — every raw settings hooks line (`hooks.<Event>`, `hooks`, `hooks.state`) leaves `config_args` and is folded, carried or left off with a notice; none is re-emitted raw, whether or not Engram writes hook output (§2.3). A settings asset with no hooks lines is untouched.
- **Paths** — no placeholder rewrite (a settings asset has no folder).
- **Dependency** — agent regains `toml` (removed by ADR-0359 with `raw_to_json`): version 1.1 (locked 1.1.2 — no new package), parse-only features (exact list checked against the locked crate at coding). Reported per CLAUDE.md.

## 3. Checks, blocks and notices

### 3.1 asset-library — the shape it reads (ADR-0359)

- **Where** — at scan, like settings: `hooks.json` is read into `ItemContent::Hook {name, file_name, folder, text, verdict}`; a shape failure is an item problem (`gate.rs` `item_problems`), so `library.list` shows the hook with its problem and `library.read` shows the text; resolve carries it as problem ② on every entry that reaches the hook.
- **What** — UTF-8 JSON (a BOM is dropped); a top-level object whose `hooks` is an object; each event value an array; each group an object whose `hooks` is an array of objects. Failure = `hook-shape-invalid {path, why}`. Not checked: event names, matchers, handler types and fields, script existence (each CLI's rules).
- **Why it carries the weight (measure-3 M1)** — claude contains a bad handler (only that handler is dropped, in `--settings` and in a plugin alike) but silently in automated runs (only the debug log line 「Invalid setting skipped without dialog (automated session)」; interactive probably a dialog — inferred). `claude plugin validate` checks plugin folders only; there is no settings-file equivalent (inferred). So Engram refuses before launch only what breaks its own assembly; a wrong field in one handler is the CLI's rule, named in help.

### 3.2 Problems (block ②) by layer

| Code | Raised by | Carried on | When |
|---|---|---|---|
| `hook-shape-invalid {path, why}` | asset-library (scan) | the hook entry | §3.1 |
| `hook-path-unsafe {hook, path}` | neutral agent rule (check) | the hook entry | §2.4 |
| `codex-hook-line-refused {hook \| source, why}` | codex `hook_assembly` (terminal) | the hook entry, or the settings asset at its line (§2.3) | guard refuses the hook's line |
| `codex-passthrough-hooks {arg}` (U5) | codex `hook_assembly` | `command-line:codex` | a pass-through hooks argument while a hook component is attached for codex or a settings asset has a hooks line, written output or not (§2.6) |
| `codex-hooks-unreadable {index, line}` | codex `command_line_args` | the settings asset, at its field line | a raw hooks line that does not parse or fold |
| the existing budget code, M7 text | codex `command_line_fits` | `command-line:codex` | over 8191 with hook lines |

`materialize` repeats the codex refusals before any write (ADR-0353).

### 3.3 Notices

| Code | Raised by | Delivery |
|---|---|---|
| `hooks_omitted` — interim codes `claude-hooks-omitted` / `codex-hooks-omitted` (P1 only; `None` from P2 / P3) | asset-library from `ModeLoads.hooks = false` | plan notice (ADR-0341 once per agent) |
| `codex-raw-hooks-trusted {settings}` | codex `command_line_fits` | plan notice |
| `codex-hooks-left-off {hook \| source, why}` | codex `command_line_fits` | plan notice — `why` = `event` (not one of codex's 14) · `type` · `field` · `matcher` |
| `codex-hooks-untrusted {hooks, why}` (JSON) | codex transport, after the thread opens | **run notice** — every launch it occurs, outside the fingerprint (exception to ADR-0341 decision 5) |

Each codex code gets its own wording in `notice_text` (`own_wording`) — the generic 「intended behavior」 sentence is false for them.

**The post-open check and its path to the caller (L2, F8)**

- `AppServerOpen` gains the expected entries `(key, hash, hook path)`.
- After the thread opens (start or resume), inside `HANDSHAKE_BUDGET` and **before** the link reports ready (so before the first `turn/start`, where SessionStart fires), the transport asks `hooks/list {threadId}`. Every expected key must be listed with `trustStatus: "trusted"` **and** `enabled: true`; any `sessionFlags` hook listed trusted that Engram did not write (a persisted user-layer entry, a bypass from a raw line) is named too. Errors — an older codex without the method, a timeout — fold into the verdict; **a failed `hooks/list` never fails the handshake**.
- `LinkResolution::Ready` carries `notices: Vec<Notice>` (`transport/mod.rs`). Through `EarlyVerdict::Ready`, `spawn_fresh_settled` / `resume_no_fallback` (`manager.rs`) hand them to the launch result, and `deliver_notices` **appends them to the returned `notices`** — the one reader the bus reply (`StartedAgent` → `started_payload`) and the WS `Spawned` event already use, so nothing changes downstream. The fingerprint record stays plan-only, so they survive an unchanged fingerprint and also reach a `NoticeDelivery::Withheld` caller's log. Boot restore (`Entry::Internal`) only logs them (O7).
- Terminal mode has no RPC; drift shows in the lane (T9).

## 4. LLM path (CLAUDE.md 「LLM-우선 제어」 · M8)

No new command. `preset.save` / `preset.check` `--hooks <entries>` (omitted = they stay), `preset.reorder --section hooks`, views carry `hooks`, `library.list --kinds Hook,HookFile`, `library.folders` rows `hook`. Bindings and `commands.schema.json` regenerate; `tests/command_declarations.rs` counts six section words.

`engram help library` gains (lines per the reference row): `hook` in the kind lists and table (「a folder holding hooks.json」), the example's `hooks:`, the tag rule, the save/check omission rule, and a hooks paragraph — the shape; `${CLAUDE_PLUGIN_ROOT}` (or `$CLAUDE_PLUGIN_ROOT`) is the hook folder, `CLAUDE_PLUGIN_DATA` is not supported; a hook folder path with a backtick, or a `$` that is not the last character of a path component, is refused when the placeholder is used, and a placeholder in single quotes or unquoted is the author's risk; write `node "${CLAUDE_PLUGIN_ROOT}/x.js"` (claude runs hooks in Git Bash, codex in PowerShell on Windows); codex runs only `type: command` hooks from Engram, trusted per run, and any edit to a hook is trusted at the next launch; codex settings hooks lines are merged and trusted too; when a codex agent has a hook component or a codex settings asset with hooks lines, a pass-through `-c hooks…` argument (any `-c` / `--config` spelling) refuses the launch — move it into a hook component or a codex settings asset (U5); a pass-through `--disable hooks` turns off every hook on that codex agent, Engram's included (the user's own choice); codex ignores events other than its 14, so such a handler is left off for codex; a command with `% & ^ | < >` cannot go on the codex terminal line; **the terminal-mode limit** (about 18 typical hooks; over it the launch is refused — reduce hooks or use JSON mode); SessionStart also runs on resume (`matcher: "startup"` for fresh starts only); claude silently skips a handler with a wrong field type; tag a hook that uses one backend's fields; if codex shows a hook review screen for Engram hooks, do not trust there — update Engram; after writing files with your own tools run `library.refresh`. English only; no UI text.

## 5. Test plan

`qa.md` 「돌릴 것 표」: **Rust 소스** (every unit) → full regression. **Lanes** — P1 and P4 (`asset-library/src/**`) both lanes; P2 (`backend/claude/**`, `platform/src/**`) claude, and both if `platform/src/shell.rs` changes; P3 (`backend/codex/**`) codex; P5 (`builtin/**` priming) both lanes, and P5 updates the pinned Rust total of P3. **GUI** — on for P3 (codex handshake = the 「… transport」 row and `manager.rs` spawn path); scene H1 in P5. Help and bindings changed commands → 「바뀐 명령만 문자 그대로 한 번」 (H1). Mutation on rows marked †.

| What | How | Unit |
|---|---|---|
| Scan, kind, precedence | `hooks.json` → `Hook` + `HookFile`; OS-rule spelling; at a mount's top → nothing; with `SKILL.md` → skill †; adding / removing `SKILL.md` or `hooks.json` flips the kind without a rescan (`reclassify` / `reread_file`) †; `--kinds HookFile` matching; `library.folders` `hook` | P1 |
| File operations | rename a referenced hook folder → references follow, kind and name kept (`follow_moves`) †; move a hook folder into itself refused †; merge target with a hook folder; delete / restore a hook folder | P1 |
| Section | round trip with tags; `wrong-kind`; `preset.reorder` / `save` / `check` with `hooks`; `save` without `hooks` keeps them †; relink rewrites a hook reference; six section words | P1 |
| Resolve | `hook-shape-invalid` at scan and on the entry †; a missing hook skipped with the missing notice, an unreadable one blocks †; dedup; tag filter; plan order; stamp; `hooks_omitted` (fake backend); the ADR-0367 gate | P1 |
| Placeholder | braced and unbraced rewritten, forward slashes, other variables untouched, identity off Windows; `hook-path-unsafe` only when the placeholder is used: `//srv/team$/x` passes; `notify$$`, `$?`, `$name`, `${x}`, `$(x)` and a backtick are refused, one fixture each †; the rewrite function itself refuses an unsafe path † | P2 |
| claude merge | golden `settings.json` (raw hooks first, components after, identical groups once); `settings_file` with hooks and no raw settings; Noop argv golden unchanged | P2 |
| codex hash | goldens for every §2.8 rule — `command_windows` alias, `timeout: 0` → 1, `SessionEnd` / `Interrupt` default and clamp, async `SessionEnd` hashed as configured, matcher dropped for `UserPromptSubmit` / `Stop`, `additionalContextLimit` per event and default, unknown fields dropped † | P3 |
| codex assembly | one `hook_assembly`: check count = `materialize` count with a folded raw line, in both modes; JSON counts no hook lines †; only the kept `fits` call's notices added †; key spelling, quoted keys always, `enabled = true`; one state line; event order; only the 14 events written, a `Notification` handler left off with `why: "event"` and no expected key †; a non-string `matcher` leaves the group off, a non-string `commandWindows` the handler †; a `null` in a claude-only field is not written and blocks nothing †; `codex-hook-line-refused` on the hook entry †; a group whose only handler is `mcp_tool`, followed by a trusted group → the emptied group not written, the next key's index matches `hooks/list` †; **written-result rule**: a component whose handlers are all left off and no carried state entry → no `hook_args`, no JSON `config`, no raw hooks line re-emitted (a pass-through hooks argument is still refused — U5 is decided on inputs) †; a carried state entry alone → the state line / thread config is written †; budget M7 text †; Rust total pinned | P3 |
| codex raw folding | codex last-wins over all settings lines first (a later `hooks={…}` wipes an earlier `hooks.SessionStart`) † then components; **cancellation**: asset A `hooks.state={K={enabled=true}}`, later asset B `hooks.state={}` (or `hooks={Stop=[…]}`) → nothing carried for K †; non-session-flags state entries carried, session-flags ones dropped with a warn; a key with a stray space equal to a session-flags key after trimming is dropped †; refusals and notices name the settings asset and line (provenance) †; `codex-hooks-unreadable {index, line}` on the settings asset; **raw-only, all filtered**: no component, one settings asset whose only hooks line is `hooks.Stop=[{hooks=[{type='mcp_tool',…}]}]`, no carried state → `codex-hooks-left-off {source, why}`, no `hook_args`, no JSON `config`, and the raw line absent from the final argv in both modes †; a settings asset with no hooks lines → its lines unchanged †; `codex-raw-hooks-trusted` with the digest — a new line in a named asset changes the notice fingerprint (unchanged fingerprint function) † | P3 |
| Pass-through (U5) | with a hook component attached for codex or a settings asset with a hooks line, a pass-through hooks argument → `codex-passthrough-hooks` on `command-line:codex`, one test per spelling †: `-c hooks.Stop=…` (next element), `--config hooks.Stop=…` (next element), `--config=hooks.Stop=…`, `-c=hooks.Stop=…`, `-chooks.Stop=…`, a key with surrounding spaces (`-c " hooks.Stop =…"`), `hooks={…}`; `-c features.x=1` and a `hooks.Stop=…` element after `--` pass †; refused also when nothing is written: a component whose handlers are all left off †, and the **no-output cancellation** — pass-through `hooks.state={K={enabled=true}}` + settings `hooks.state={}` → refused †; with no hook component and no settings hooks line it passes unchanged † | P3 |
| codex JSON + run notices | start and resume carry `config.hooks` (absent when nothing is written — never an empty `hooks`); `hooks/list` verdicts — untrusted, `enabled: false`, foreign trusted session-flags hook → notice †; a failed `hooks/list` keeps `Ready` †; the notice delivered once per launch and **survives an unchanged fingerprint** †; boot restore only logs; ADR-0216 tests green | P3 |
| codex live (lane, no tokens) | in `tests/codex_launch_live.rs`, **through Engram's own transport and `AppServerOpen`**: (a) JSON start and resume with Engram's config → `hooks/list {threadId}` keys, `currentHash` = Engram's for a fixture per hash rule, all trusted and enabled; (b) app-server with Engram's terminal lines → `hooks/list {cwds}` the same; (c) a user-layer state fixture in a scratch `CODEX_HOME`: a trusted user hook stays listed trusted, an `enabled = false` one stays off, with Engram's state present, and a carried raw entry keeps its effect (F1, measure-4 M-B); (d) Engram's handshake as is, without `experimentalApi` (measure-4 M-A) | P3 |
| claude live (lane) | `launch_kits_live` `claude_`: a hook component's marker written | P2 |
| Scene H1 (GUI 실측) | isolated daemon; hook `team/hooks/marker` (SessionStart + UserPromptSubmit) via `engram preset.save --hooks`; claude terminal, claude JSON, codex terminal, codex JSON — one prompt each → a marker per backend · mode; codex terminal: no review screen, no bypass warning; **restart both codex agents** (JSON resume, terminal `codex resume <id>` with the hook lines) → marker again; `~/.codex/config.toml` byte-identical; a codex settings asset with `hooks.Stop=[…]` + the component → both run, trusted, `codex-raw-hooks-trusted` shown; a hook command with `|` → `codex-hook-line-refused`; budget overflow → M7 text; claude terminal (interactive) — no prompt for the settings hooks (O5 (1)) | P5 |

## 6. Implementation units

Sequential — P1–P3 share `launch.rs` / `settings_check.rs` and the fakes; P4 shares `ops.rs` / `catalog.rs` with P1. Each unit carries its implementors and test doubles; green at each end. (Units are **P1–P5** so they do not read as user decisions U1–U4.)

| Unit | Content | Lines |
|---|---|---|
| P1 | asset-library kind, folder-level precedence, `Skill \| Hook` folder items in catalog and file operations (§2.1 list; `Target.skill_folder` stays skill-only), `library.folders` `hook`, section, shape check at scan, resolve with ADR-0374 skipping; agent `HookRef`, `LaunchPlan.hooks`, `ModeLoads.hooks` (claude and codex `false` for now — interim notice, never a silent drop), `NoticeCodes.hooks_omitted`, `CommandLine.hooks` and `config_origins`, `RawSetting.source`, `command_line_fits` → `Ok(Vec<Notice>)` (empty for both), the refusal attribution by `hook` / `source`, only the kept `fits` call's notices; `PresetSection::Hooks`, `LibraryKind::Hook` / `HookFile`, args (omission rule), bindings; section help lines. | ~1,800–2,300 |
| P2 | `backend/hooks.rs` (rewrite, path rule), the two platform functions, claude hook layers, claude `loads.hooks = true`, claude lane test. | ~450–650 |
| P3 | codex `hook_assembly` (hash, key, TOML writer, the 14-event list, typed field checks, raw folding with `toml` and codex last-wins over all settings lines, state carry with trimmed keys, left-off and index rule, provenance, refusals, budget, U5 pass-through refusal), `command_line_args` `codex-hooks-unreadable`, `hook_args` / `hook_config`, thread params `config`, `AppServerOpen` expected entries, post-open check, `LinkResolution::Ready` notices, `deliver_notices` appending them, `notice_text` wordings, codex `loads.hooks = true`, `sha2`, ADR-0216 test docs, codex lane tests. | ~1,800–2,400 |
| P4 | O4 included (main, 2026-11-25): the lone-marker confirmation for `hooks.json` (`hook-unconfirmed`; help line 341). | ~300–500 |
| P5 | Help paragraph and priming root line (pinned total updated), reference files and code docs (the row), scene H1. | ~250 |

## 7. Interim state

Between P1 and P2 / P3 an attached hook is carried by neither backend; the interim omission notice names it, so no commit drops a hook silently. P2 and P3 flip their backend's answer in the commit that carries the hooks.

## 8. Hand-over to the client stage

The preset edit area gains a `hooks` field on the same `preset.save` / `check` (step 13); the library tree marks hook folders from `library.folders` `hook`; the problem list shows the §3.2 codes; the launch report shows the §3.3 notices, the run notice included. LLM path = §4. Written once more in the client step's TRD at the settling commit.

## 9. Decisions

### 9.1 User decisions (2026-11-24)

- **U1 — Codex hook trust: Engram trusts only the hooks it attaches.** Attaching a hook to a preset is the user's approval — 「우리건데?」. Not the bypass flag (which also trusts every other unreviewed hook and warns on every run); not a review prompt. Convention: Zed codex-acp, Orca and context-mode trust only their own injected hooks — a fixed set they ship, which they re-check by hash; Engram's hooks are editable library content and the hash is recomputed per run, so an edit is trusted at the next launch (§2.8, accepted consequence — F4); atuin: 「the user asked for atuin's hooks by installing them」; Warp and hapi use the bypass flag for unattended runs.
- **U2 — Trust per run, in the session-flags layer**, never in `~/.codex/config.toml` — 「-c 가 그럼 더 좋다는거아냐? 유동적이고」. Keys are positional, so a persisted entry would be overwritten by the next agent's combination; per-run keeps agents independent and leaves nothing behind. Risk: undocumented but source-honoured (`config_rules.rs`); no other tool found using it (R2).
- **U3 — Engram's own past codex hook is not revived**, nothing added to the built-in preset — 「ㅇㅇ 그대로가」.
- **U4 — A codex settings asset's raw hooks lines are merged into the combined list** (§2.9) — 「합치기가 맞고」. Options offered: (a) refuse with guidance — ★ main's recommendation; (b) merge; (c) ours win + notice. The user chose (b), **갈림** from the recommendation. Consequences: raw hooks lines are always consumed by the assembly, never re-sent raw (§2.3); TOML reading only in the codex backend, agent regains `toml` (ADR-0359 amendment); raw hooks lines are trusted and run where they were skipped before (named by `codex-raw-hooks-trusted`); claude already merges (O2); no collision refusal. Main's refinements (round 1), **to be shown to the user at the next report**: among the raw settings lines codex's own last-wins rule applies first, then the result merges with the components (F2 — replaces draft 1's 「raw lines concatenated」); non-session-flags state entries of the effective result are carried (F1).
- **U5 — Pass-through codex hooks lines while the launch has hook inputs: refuse with guidance** (user 2026-11-25: 「A로 해서 코딩 들어가.」). Options offered: (A) refuse the launch with guidance — ★ main's recommendation; (B) fold every spelling into one override sequence with the settings lines (O6). The user chose (A), **같음** with the recommendation. Rule (§2.6): `codex-passthrough-hooks` — move the line into a hook component or a codex settings asset — for any `-c` / `--config` spelling as clap reads it, whenever a hook component is attached for codex or a settings asset has a hooks line, written output or not; with neither they pass as today.

### 9.2 Main's decisions (defaults, reported)

- **M1** A hook component = a library folder holding `hooks.json` plus scripts, recognised by that marker (§2.1). Rejected: per-backend definition files in one folder.
- **M2** Script paths through the placeholder, rewritten to the hook folder for both backends (§2.4), with the path rule. No schema check.
- **M3** claude route = the merged `--settings` `hooks` key (ADR-0328). Rejected: `hooks/hooks.json` in the `--plugin-dir` folder (two mechanisms). measure-3 M1: a bad handler drops only itself in both routes (O6 closed).
- **M4** codex route = terminal lines + one state line; JSON = one `hooks` object in the thread config on start and resume; hash computed by Engram; the JSON post-open check with run notices; escaped trust key.
- **M5** Section `hooks` with backend tags; preset order.
- **M6** Windows shell difference passed through; help recommends `node "<script>"`.
- **M7** Terminal budget and its user-facing policy as §2.6.
- **M8** Screen with the client stage; this step's LLM path = existing commands (§4).
- **M9** asset-library backend-blind; per-backend handling only in `backend/claude` / `backend/codex` (§1).
- **TRD-level choices**: T1 config on resume (measured); T2 hooks not copied; T3 `config_args` in artifacts = raw lines minus hooks lines, in `CommandLine` = all lines (§2.3); T4 write only what codex reads (14 events, typed fields) and trust only normalizable command handlers, the rest left off with a notice in both modes; `mcp_tool` left off as a scope cut (§2.8); T5 the post-open notice is a run notice outside the fingerprint (§3.3); T6 interim `false` answers (§7); T7 the shape check at scan as Engram's component format; T8 `enabled = true` in every trust entry (R3); **T9 no per-codex-version hash probe** (L17): it would catch a hash drift in terminal mode at run time for ≈ 350 ms once per codex binary, but needs a codex-binary identity, a process spawn on the check path and a cache that follows codex's self-update; the codex lane already re-runs on every installed codex version change (qa.md lanes rule ②) and JSON mode has the run-time check; the residual terminal drift is R2 with the help line of §2.6.

### 9.3 Open items (recommendation each)

- **O1 — decided (U4).**
- **O2 — claude settings `hooks` + components.** Merge (ADR-0328) — close as 「merge」.
- **O3 — Which modes load hooks.** All four; `hooks_omitted` for a mode that answers `false`.
- **O4 — The lone marker.** Whole-folder operations are P1. Left: renaming `hooks.json` or moving it out of a referenced hook folder makes the hook vanish — references then read as missing and the launch **skips** it with a notice (ADR-0374), not silent but easy to miss. Recommend **include (P4)**: the `SKILL.md` confirmation (ADR-0339, 「skill.md가 없으면 스킬 자체가없어지는거잖아. 그럼 참조를 빼야되는거 아님?」) with its own code `hook-unconfirmed`. Alternative: `docs/tracking.md`. **Decided by main (2026-11-25): include — P4 runs** (shown to the user in the TRD summary as a main decision; ADR-0420 decision 14).
- **O5 — Left unmeasured.** (1) claude interactive launch with `--settings` hooks (scene H1); (2) claude pass-through `--bare` / `--safe-mode` / `disableAllHooks` (risk); (3) measured (resume); (4) terminal drift — lane only (T9); (5) user-layer state on session-flags keys — mitigated by T8, checked in the lane (c); (6) measured (measure-4): M-A — `experimentalApi` not needed; M-B — session state overrides per key and field; M-C — terminal: `exec` skips untrusted hooks silently, the TUI shows a blocking picker whose 「Trust all」 writes session-flags keys into the real `~/.codex/config.toml`.
- **O6 — not chosen (U5 = A): fold pass-through hooks lines.** Recognise every codex spelling for the `hooks` prefix, put pass-through lines into one override sequence with the settings lines (pass-through first, as they come first on the line), fold the effective result, trust it like U4's raw lines, carry its state the same way, refuse an unparsable pass-through value on `command-line:codex`. Cost: one more input class to the seam and its tests; benefit: no refusal for users who keep hooks in profile arguments.
- **O7 — (hold, for the user / later) boot-restored codex JSON agents only log the post-open verdict** (risk N6). The 「foreign trusted session-flags hook」 verdict is the only run-time detector of a hook riding Engram's trust; restored agents can run mail-driven turns unwatched. Options: log it at warn level (cheap), or surface it in the restore status / problem list.
- **O8 — (hold, optional) detect codex's hook review screen in the terminal pump** (risk N8): its fixed text (「Hooks need review …」, `tui/src/startup_hooks_review.rs`) could raise the 「do not trust there — update Engram」 notice at run time, turning R2's terminal residual visible with no probe and no cache.

## 10. ADR proposals (numbers by `/engram-adr`)

1. **New — hook components and codex per-run trust**: the component and its folder-level precedence, the section and the omission rule, the placeholder and its path rule (refused inside the rewrite function; single-quoted or unquoted placeholders are the author's risk), the claude route, the codex assembly (one seam with origins, only what codex reads, raw folding with codex last-wins first, state carry with trimmed keys, left-off and the index rule), per-run trust with `enabled = true` and no persisted state, the run notice, the shape check as Engram's format, M7, U4, U5 (refuse with guidance), and **the accepted consequence of F4** (any edit is trusted at the next launch).
2. **Amend ADR-0216** — codex hooks return as user components and merged settings hooks with per-run trust; decision 1 applied to them (pass-through hooks lines refused while a hook component or a settings hooks line is present — U5); 「do not revive the session-id hook」 stays; the no-hook tests read 「with no hook output written」.
3. **Amend ADR-0328** (and ADR-0316 decision 3) — one settings layer per hook component after the raw settings.
4. **Amend ADR-0353** — the fail-closed list gains the hook line refusals, the budget with hook lines (M7 text), `codex-hooks-unreadable` and `codex-passthrough-hooks` (U5).
5. **Amend ADR-0359** — agent regains `toml` (codex backend only, parse); still no schema check.
6. **Amend ADR-0341 decision 5** — the codex post-open notice is shown on every launch it occurs (outside the fingerprint).
7. **Amend ADR-0386 decision 4** — `LibraryKind` gains `Hook` / `HookFile`; `--kinds` matches a hook's `hooks.json` row only by `HookFile`; `LibraryFolderRow` (the `library.folders` row) gains `hook`.
8. If O4 = include — the lone-marker rule covers `hooks.json` (ADR-0339 lineage).

## 11. Input claims corrected

- **C1** The brief's JSON route named only `thread/start`; resume needs the config too — measured (measure-3 M2).
- **C2** Parent item 22's trust choices (per-hash storage or bypass) — replaced by per-run session-flags trust (U1, U2).
- **C3** 「a volatile script path would need re-approval every launch」 — moot (per-run trust; stable library paths).
- **C4** ADR-0359's older 「a bad `--settings` object is discarded whole」 does not hold for hooks on 2.1.330 (measure-3 M1).
- **C5** measure-2 M5's 2542 is a replica; P3 pins the Rust total.
- **C6** `codebase.md`'s `LaunchArtifacts` field list is partial; nothing depends on it.
- **C7** No skill-creation verb exists either; no hook-creation verb.
- **C8** Draft 1 said whole-folder file operations 「come free」 — skill-only code in `catalog.rs` / `ops.rs` / `lib.rs` must learn the hook kind (§2.1).
- **C9** Draft 1's 「missing hook blocks」 (O4) contradicted ADR-0374 — a missing hook is skipped with a notice.
- **C10** Review C2 listed 「SessionEnd async → false」 for the hash; in `discovery.rs` the hashed config keeps the configured `async` (only the run is forced synchronous). The other review items (`command_windows` alias, `timeout: 0` → 1) hold; re-reading added the matcher drop for five events and the `additionalContextLimit` event rule (§2.8).
- **C11** Draft 1's `architecture.md` condition (「if the handshake order it states changes」) was wrong: it states no handshake but does enumerate the kinds.

## 12. Risks

- **R1** claude drops a wrong-typed handler silently (only that handler); Engram does not check handler fields; help names it.
- **R2** Per-run trust relies on source-honoured codex behaviour; a release can change it or the hash rules. JSON: run notice; terminal: the lane on every codex version (T9) and the help line — until fixed, codex's review screen may come back for terminal agents with hooks (the ADR-0216 cost).
- **R3** A user-layer state entry on a session-flags key — mitigated by `enabled = true` (T8), lane (c).
- **R4** Fields only one backend knows may break the other backend's config; help says tag such hooks.
- **R5** Shell semantics differ on Windows (Git Bash vs PowerShell 7); a bash-only command fails under codex.
- **R6** Terminal budget ≈ 18 typical hooks; no warning before a launch — the refusal text or `preset.check` tells.
- **R7** Scripts run in place; edits affect running agents and are trusted at the next launch (F4).
- **R8** Handlers Engram cannot trust are left off for codex (notice) — an `mcp_tool` hook never runs through Engram on codex.
- **R9** The post-open `hooks/list` adds one round trip to codex JSON launches with hooks.
- **R10** Bindings and help-screen counts grow.
- **R11** Raw codex hooks lines that were inert start running once this step ships; named by `codex-raw-hooks-trusted` (with a per-asset digest, so a grown set is named again) and the help line; a release note.
- **R12** If a user's own raw `hooks.state` carries an other-source entry, Engram carries it verbatim, so its effect on the user's hook (enable or disable, per key and field — measure-4 M-B) is exactly what the user wrote; Engram never generates such entries.
- **R13** A pass-through `--disable hooks` (codex's feature toggle, equal to `-c features.hooks=false`) is not a hooks line and passes U5; it turns off every hook, Engram's included — the user's own choice (like O5 (2)). JSON mode names it through the run notice; terminal mode is silent; help says so.

## 13. Review record

PRD+TRD review: 2026-11-25 · deep (lead, risk axis: trust and execution boundary, Codex gpt-6.1-sol xhigh) · PASS · reviewed draft at 56c6a273
