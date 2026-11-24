# TRD — S25 step 22: hook components (`S25: feat(asset-library,agent): hook components`)

| Item | Value |
|---|---|
| **Status** | **Draft 1 (2026-11-24)** — decisions and boundaries only; not reviewed. User decisions U1–U4 taken 2026-11-24 (§9.1). Open items O2–O5 carry a recommendation each (§9.3). Round 2 (2026-11-24): O6 closed by measurement (M3 kept), T1 measured, the measurement notes moved to `docs/research/`. Round 3 (2026-11-24): O1 decided by the user as U4 (merge); the client-stage budget warning dropped; the per-backend boundary stated (§1). Canonical ADRs: ADR-0359 (no schema check — Engram checks only what its assembly breaks) · ADR-0367 (asset-library is backend-blind; per-backend choices are port answers) · ADR-0334 (resolution in asset-library, flag translation in backend `materialize`) · ADR-0316 · ADR-0328 (claude: one merged `--settings`) · ADR-0341 (preset sections, launch notices once per agent) · ADR-0353 (codex fail-closed before any write) · ADR-0216 (the codex session-id hook stays removed; amended here — §10) · ADR-0210 (superseded; why per-definition trust failed before) · ADR-0217 · ADR-0386 (preset / library command surface) · ADR-0307 · ADR-0155. |
| **Parent TRD** | [`trd.md`](trd.md) §5-3 item 22 · §1-7 (command surface — the screen and the LLM call the same commands) · the end-of-server-stage stop after item 22. |
| **Pattern** | [`trd-file-commands.md`](trd-file-commands.md) (step 10b) for form; skills are the template for a hook kind at every layer (scan → preset section → resolve → plan → materialize). |
| **Facts** | Measurements of 2026-11-24 on Claude Code 2.1.330 and codex-cli 0.171.0 (source at `rust-v0.171.0`): [`hook-loading-claude-codex-2026-11-24.md`](../../research/hook-loading-claude-codex-2026-11-24.md) (hook loading in both CLIs) · [`codex-hook-trust-per-run-2026-11-24.md`](../../research/codex-hook-trust-per-run-2026-11-24.md) (codex M1–M6: JSON route, terminal route, hash, override replace rules, budget, guard) · [`hook-failure-and-resume-2026-11-24.md`](../../research/hook-failure-and-resume-2026-11-24.md) (claude bad-hook containment, codex resume). Codebase facts ([`hook-components-codebase-2026-11-24.md`](../../research/hook-components-codebase-2026-11-24.md)) were checked against the code at `7528e4ae` (§11). Below, 「measure-2」 = the codex trust file and 「measure-3」 = the failure-and-resume file. |
| **Reference files to fix** | `docs/reference/structure/asset-library.md` (Modules: `format/hooks.rs`; the hook kind in the scan; `LaunchPlan.hooks`; the section list) · `docs/reference/structure/agent-backend.md` (claude: hook layers in the merged settings; codex: hook lines, thread config, trust entries, the post-open `hooks/list` check; an ADR-0216 note in 「Session id recovery」) · `docs/reference/architecture.md` only if the handshake order it states changes (one request after the thread opens) · `prompts/engram-help.md` (kind table, example preset, tag rule 「instructions, skills and hooks」, section words in `preset.reorder` / `save` / `check`, a hooks paragraph with the terminal-limit line — §4) · `builtin/instructions/priming.md` root line (「Instruction · skill · hook · settings · preset files」 — codex priming budget re-checked) · asset-library `src/lib.rs` header · agent `src/launch.rs` docs of `LaunchPlan` / `LaunchArtifacts` · `backend/settings_check.rs` (`ModeLoads`, `NoticeCodes`, `CommandLine`) · `backend/codex/mod.rs` comment at the ADR-0216 note and the doc of `hook_override_value` (§2.8) · `backend/codex/protocol.rs` thread params docs. |
| **Line-pointer basis** | Code at `7528e4ae` (step 10b Implemented). Code is cited by file + symbol. |

## 0. Scope

**What and why.** A hook becomes a library component like a skill: a folder holding a hook definition (`hooks.json`) and its scripts, listed in a preset's new `hooks:` section, handed to claude and codex the way each loads hooks. Engram assembles and hands over; the CLIs apply their own rules (ADR-0359). User 2026-11-23: 「없는것보다는 있는게 좋겠지? 유저가 하드코딩하기 딱 좋은게 훅인데」 · 「기존 인프라에 그대로 끼고 우리쪽은 거치고 클로드 코덱스쪽으로 에러를 넘기는거라서 많이 수정은 안한다고 생각되는데 어쨋든 쭉 진행하면 될듯」. Today a claude hook can already pass through a settings preset's raw text (ADR-0328), with its script outside the library; codex raw `-c hooks.*` lines pass too but are skipped silently (untrusted).

**PRD relation.** Parent PRD [`prd.md`](prd.md) (preset + library). Item 22 was inserted by the user on 2026-11-23 after the PRD's requirement list; it adds a fifth asset kind beside instruction · skill · settings · preset and follows the PRD's rules for kinds (backend tags — G29, order per preset — G51, the problem list, launch blocks, notices). This step is **server stage**: no screen. It runs after 10b and closes the server stage (then the stop and the 「서버 단계 수용」 pass of parent §4).

**In**

- asset-library: the hook kind in the scan (folder + marker `hooks.json`), the `hooks:` preset section (reader, writer, edit, relink, references, tags), the shape check of a definition, resolve and `LaunchPlan.hooks`, the stamp.
- agent, neutral: `HookRef`, `LaunchPlan.hooks`, the placeholder rewrite, `ModeLoads.hooks` and its omission notice code, `CommandLine.hooks`, the `hooks` section word on the existing `preset.*` / `library.*` commands (bindings regenerated).
- claude backend: hook layers in the one merged `--settings`.
- codex backend: terminal `-c` lines, JSON thread config, per-run trust entries with an Engram-computed hash, the post-open check in JSON mode, folding a codex settings asset's raw hooks lines into the combined list (U4), the budget count and its refusal text.
- Help and priming lines; ADRs (§10).

**Out** — the screen and its LLM path for editing the hooks section (client stage, hand-over §8); a preset-editor budget warning (dropped by the user — the refusal text is the guidance, §2.6); any schema check of hook events, matchers or handler fields (ADR-0359); a hook-creation verb (as for skills, an LLM writes the files with its own tools and calls `library.refresh`); Engram's own past codex session-id hook (U3 — not revived, nothing added to the built-in preset); the bypass flag; any write to `~/.codex/config.toml` or `~/.claude/settings.json`; Windows shell differences between the backends (M6 — passed through); new wire messages (`PROTOCOL_VERSION` unchanged); the catalog stays 9 (same unreleased generation as 10a, 10b and step 21).

## 1. Boundaries and ownership

- **All per-backend handling lives in agent `backend/claude` and `backend/codex`; asset-library stays backend-blind** (ADR-0367 · ADR-0334 · CLAUDE.md 「백엔드 확장」). That covers the claude settings layer, every codex line, the codex TOML reading of raw hooks lines (U4), trust entries, hash, guard, budget, and the post-open check; asset-library sees hooks only as backend-neutral `HookRef`s and port answers. Enforced by the existing §8-3 gate of the child TRD `trd-asset-library.md` (`rg -i '\b(claude|codex)\b'` over `crates/engram-dashboard-asset-library/src` — no branch hits), re-run in every unit that touches asset-library.
- **asset-library owns the component format and stays backend-blind** (ADR-0367, M9). It finds hook folders, reads `hooks.json`, checks the shape Engram reads (§3.1), resolves the section with tags, and hands `HookRef`s to the plan. It never names a backend, never rewrites a command and never knows the placeholder. Whether a backend · mode loads hooks is the port's answer (`ModeLoads.hooks`); a mode that answers `false` gets the port's notice code (`NoticeCodes.hooks_omitted`). Gate: the existing §8-3 check (`rg -i '\b(claude|codex)\b'` — no branch hits in asset-library).
- **agent, neutral (`launch.rs` or a new `backend/hooks.rs`)** owns `HookRef` and the placeholder rewrite (§2.4) — the one place both backends call, so claude and codex see the same rewritten definition.
- **backend/claude** turns hooks into one more layer of the merged settings (§2.5). Nothing on the command line changes.
- **backend/codex** owns everything codex-specific: the event label, the session-flags key, the hash, the TOML inline spelling, the line guard and budget, the thread config, the reading and folding of raw hooks lines from codex settings assets (U4, §2.9), and the post-open check (§2.6–§2.9). `check` (through the port) and `materialize` build the lines with **one function** in `cmdline.rs`, as the priming does — what `check` counted is what goes on the line.
- **codex transport** sends the thread config on `thread/start` and `thread/resume` and runs the post-open check; its verdict reaches the launch's notices through the spawn result (§3.3).
- **daemon · CLI · shell · frontend**: no change beyond regenerated bindings and help text.

## 2. Data shapes

### 2.1 Hook folder (library)

```
team/hooks/notify/            ← the hook item (its name = the folder name)
  hooks.json                  ← the marker and the definition
  notify.js · guard.js        ← scripts, run in place
```

- **Marker** — a file named `hooks.json` directly in a folder below a mount's top makes that folder a hook (`ItemKind::Hook`, with `ItemKind::HookFile` for the file, as `Skill` / `SkillFile`). The name compares by the OS rule (platform `same_path` — on Windows `Hooks.JSON` is one), as `SKILL.md`. A `hooks.json` directly in a mount's top makes no hook (a mount's top is not an item) and stays unlisted (`.json` is no kind).
- **One folder, one kind** — a folder holding both `SKILL.md` and `hooks.json` is a skill; its `hooks.json` is an ordinary file. Existing classifications do not move.
- Everything else in the folder classifies as today (a `.md` beside the scripts is an instruction item, as in a skill folder). Nested hook folders are their own items, as nested skills.
- `hooks.json` = the claude/codex shared shape: `{"description"?: …, "hooks": {"<Event>": [{"matcher"?: …, "hooks": [{"type": "command", "command": …, …}]}]}}`. Engram carries only the `hooks` object; other top-level keys stay in the file.
- `library.list` kinds gain `hook` and `hook_file` (agent `LibraryKind`). Moves, renames, copies and deletes of a whole hook folder work through the existing generic paths once the kind maps to its list (`edit.rs` `list_of` → `ListName::Hooks`); the lone-marker rule is O4.

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

- `ListName::Hooks` (key `hooks`), placed after `Skills` in `ListName::ALL` (the writer's order for a missing key); `takes_tags` = true (M5). Entries are library paths to hook folders; an entry naming another kind answers the existing `wrong-kind`.
- `PresetSection` (agent) gains `hooks`; `preset.save` / `preset.check` gain `hooks?`; `preset.reorder` takes `--section hooks`; the preset view and draft carry the list (§4).
- **Order** — plan order = preset expansion order, as instructions and skills: a preset's common entries, then its entries for the agent's backend (G51), presets in attach order. The same hook folder reached twice is carried once, at its first position.

### 2.3 Plan and artifacts

- `HookRef` = `path` (the library reference, for messages and notices) · `dir` (absolute hook folder) · `definition` (the parsed `hooks` object, not rewritten). `LaunchPlan.hooks: Vec<HookRef>` in plan order; empty in a Noop plan. The check's `stamp` covers each `hooks.json` read, so a definition changed between `check` and `prepare` is `PrepareError::Stale` as for other sources.
- Hooks are **not copied** into the volatile folder: scripts run in place from the library folder (as skills, through junctions today). Editing a script while an agent runs changes what the next hook run does — accepted, same as skills.
- `LaunchArtifacts` gains, for codex only: `hook_args: Vec<String>` (terminal — the `-c` pairs of §2.6, placed right after `config_args`, before the MCP attachment and `developer_instructions`) and `hook_config: Option<Value>` (JSON — the object of §2.7). `config_args` keeps meaning 「exactly what the raw settings add」. claude needs no new field: `settings_file` is now written when there are raw settings **or** hooks.
- `CommandLine` (port input) gains `hooks: &[HookRef]`, so codex terminal counts and guards the hook lines at `check` (block ②) exactly as `materialize` will build them.

### 2.4 Script paths — the placeholder (M2)

- A command may use `${CLAUDE_PLUGIN_ROOT}` (exactly this spelling). For both backends Engram replaces it in **every string value** of the definition with the hook folder's absolute path written with forward slashes (`D:/Library/hooks/notify`) — the spelling claude itself substitutes on Windows; it works in Git Bash, PowerShell and cmd, does not depend on bash's backslash rules (a UNC `\\server` would lose a backslash inside bash double quotes), and costs half as much on the codex line (each `\` counts twice). The separator change goes through a platform function (ADR-0266); it is the identity off Windows.
- Nothing else is rewritten (`${CLAUDE_PLUGIN_DATA}`, `$CLAUDE_PROJECT_DIR` pass through as written). A Claude plugin's hook folder works as it is.
- The path is the library path — stable across launches, not the volatile folder — so a codex hash does not change per launch (the ADR-0216 failure mode came from a per-release path).

### 2.5 claude — one more settings layer (M3, O2)

`settings.json` = `merge(raw settings in attach order, then one layer {"hooks": <rewritten definition>} per hook in plan order)` with the existing ADR-0328 merge (objects recursive, lists concatenated, equal values once). Result for the example (the built-in 「기본」 plus one hook, other keys left out):

```json
{
  "allowedMcpServers": [{"serverName": "engram"}],
  "hooks": {
    "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "node \"D:/Library/hooks/notify/guard.js\"", "timeout": 30}]}],
    "SessionStart": [{"hooks": [{"type": "command", "command": "node \"D:/Library/hooks/notify/notify.js\" start"}]}]
  }
}
```

A wrong-typed field in one hook drops only that handler (measure-3 M1a — silent; §3.1), so one bad component does not take down the other settings. A settings preset's own `hooks` groups come first in each event, the components' after; identical groups collapse to one (claude also runs an identical handler from several settings sources once). No collision rule is needed for claude (O2). Both claude modes load hooks (`ModeLoads.hooks` = true).

### 2.6 codex terminal — lines (M4, M7)

Per event one line holding every group of that event (all hooks, plan order), then **one** `hooks.state` line holding every trust entry — codex replaces, never merges, a repeated `-c` path (measure-2 M4: a second `hooks.state` line wipes the first's entries). Events in a fixed (sorted) order. For the example (mount `team` at `D:\Library`):

```
-c hooks.PreToolUse=[{matcher='Bash',hooks=[{type='command',command='node "D:/Library/hooks/notify/guard.js"',timeout=30}]}]
-c hooks.SessionStart=[{hooks=[{type='command',command='node "D:/Library/hooks/notify/notify.js" start'}]}]
-c hooks.state={"C:\\\u003csession-flags\u003e\\config.toml:pre_tool_use:0:0"={trusted_hash="sha256:…"},"C:\\\u003csession-flags\u003e\\config.toml:session_start:0:0"={trusted_hash="sha256:…"}}
```

- **TOML spelling** — a small inline writer in `cmdline.rs` (no `toml` crate): strings as literals `'…'` unless they hold `'` or a control character, then as basic strings with escapes; a control character is never raw (cmd cuts the line at a real LF — `backend/codex/mod.rs`). Numbers, booleans, arrays and objects as inline TOML.
- **Guard** — every hook line passes the existing guard (`text_is_cli_safe` + an even `"` count, as `cmdline::config_args`). The trust key holds `<` `>`, which the guard refuses, so it is written as a basic string with `\u003c` / `\u003e` (measured through `cmd.exe /c codex`, measure-2 M6). A command with `% & ^ | < >` (e.g. `%USERPROFILE%`, `| tee`, `> out`) is refused: problem ② at `check`, `Err` in `materialize` (ADR-0353 style).
- **Budget** — counted with the existing rule (`cmdline::fits`, 8191 with the shim margin). Today's line with the built-in priming ≈ 2542 units (measure-2 M5, a replica of the Rust count — U3 pins the Rust number in a test); a plain hook costs ≈ 293 for the first and ≈ 244 for each more on one event, so about 20 typical hooks fit. Over budget = the existing fail-closed refusal (ADR-0353) — no truncation, no dropping.
- **Budget policy toward the user (main 2026-11-24, M7)** — no proactive warning in this step. (1) The over-budget refusal names the cause and the remedy: when hook lines are on the line, the problem ② text reads 「codex terminal command line limit exceeded — reduce hooks or use JSON mode」 (the refusal's `args` also say that hook lines are counted, so an LLM can act on it); a line over budget without hooks keeps today's text. (2) One line in the hooks part of `engram help library` states the terminal-mode limit (§4). No preset-editor warning (user 2026-11-24): the refusal text with cause and remedy is the guidance, and JSON mode has no such limit. `preset.check` still reports the same over-budget problem, as for any problem ②.

### 2.7 codex JSON — thread config (M4)

`thread/start` **and `thread/resume`** carry `config: {"hooks": <object>}` — one `hooks` key holding the events and `state` (measure-2 M1 order b; never `"hooks"` together with a dotted `"hooks.…"` key in one request — codex applies request keys in hash-map order). JSON needs no escaping of the key's angle brackets:

```json
"config": {"hooks": {
  "PreToolUse":   [{"matcher": "Bash", "hooks": [{"type": "command", "command": "node \"D:/Library/hooks/notify/guard.js\"", "timeout": 30}]}],
  "SessionStart": [{"hooks": [{"type": "command", "command": "node \"D:/Library/hooks/notify/notify.js\" start"}]}],
  "state": {
    "C:\\<session-flags>\\config.toml:pre_tool_use:0:0":  {"trusted_hash": "sha256:…"},
    "C:\\<session-flags>\\config.toml:session_start:0:0": {"trusted_hash": "sha256:…"}
  }
}}
```

The object replaces the whole `hooks` table the app-server line holds (measure-2 M1 a2) — so a settings asset's raw hooks lines are folded into it rather than left on the app-server line (U4, §2.9). SessionStart fires on the first `turn/start`, not on `thread/start` (measured). **Resume (measure-3 M2, measured)**: `thread/resume` in a new process with the same `config.hooks` (events + state) shows the hook `trusted` in `hooks/list {threadId}` and fires SessionStart with `source: "resume"` on the first `turn/start`; `thread/resume` **without** the config has no hooks at all (`hooks: []`, no warning) — the thread config is not saved with the thread, so Engram sends it on every resume. SessionStart therefore runs on resumes too; a hook meant for fresh starts only uses `matcher: "startup"` (the matcher is in the hashed object, so it changes the hash — passed through as written; help mentions it). The protocol structs `ThreadStartParams` / `ThreadResumeParams` gain an optional `config` (absent = byte-identical to today).

### 2.8 Trust entries and the hash (U1, U2, M4)

- **Key** = `<session-flags source>:<event label>:<group index>:<handler index>`, indexes 0-based positions in the **final** combined array of that event. The source spelling is codex's synthetic path (`C:\<session-flags>\config.toml` on Windows — codex hard-codes `C:\`); its OS choice goes through a platform function (ADR-0266), and the live codex lane test pins it against `hooks/list`. The event label is the snake form of the event name (`SessionStart` → `session_start`, `PreToolUse` → `pre_tool_use`).
- **Hash** (codex's algorithm, measure-2 M3 — 8 of 8 matched): normalize the handler — `command` = `commandWindows` when set on Windows (OS choice through platform), `commandWindows` dropped; `timeout` always present (default 600; `SessionEnd` / `Interrupt` default 1, clamped 1..3); `async` always present (default false); `statusMessage` only if set; `additionalContextLimit` only if set and not the default — then `{event_name, matcher?, hooks:[handler]}` with every object's keys sorted explicitly, compact JSON, sha256, `"sha256:" + lowercase hex`. The agent crate gains the `sha2` dependency (already in the workspace lock through asset-library — no new package; reported per CLAUDE.md).
- **Which handlers get an entry** — only `type: "command"` (the measured normalization). Other handler types get none: codex skips them (JSON: named by the post-open check, §3.3; terminal: codex's own startup review screen shows them). Help says codex runs Engram hooks of type `command`.
- **Per run, nothing persisted** (U2) — the entries live in the session-flags layer of that process only; `~/.codex/config.toml` is never written (QA compares it before and after). No bypass flag, no review prompt (U1).
- **ADR-0216 tests** — `backend/codex/mod.rs` `the_terminal_spawn_no_longer_registers_the_session_start_hook` and its two siblings keep asserting that a spawn without hook components carries no hook; their doc changes from 「must always be None」 to 「no hook without a hook component — Engram's own session-id hook stays removed (ADR-0216, U3)」, and a new test asserts that a component's line carries only the user's command.

### 2.9 codex settings assets with raw hooks lines — merged (U4)

- **What is read** — in a codex raw settings value (the list of `key=value` lines), a line whose key is `hooks` (a table of events) or `hooks.<Event>` (an array of groups). The codex backend parses its value as TOML (only there — §1) and takes the groups per event. Such lines leave `config_args` (terminal and the JSON app-server line alike) and join the combined hook list, so `config_args` becomes 「what the raw settings add, minus their hooks lines」. A raw `hooks.state` line is dropped: Engram writes the one state line itself, and positions change in the combined list anyway.
- **Order** — per event: the settings assets' groups first (settings attach order, lines in file order), then the hook components' (plan order) — the same order as claude's merge (§2.5), so one rule for both backends. Several raw lines for one event are concatenated (merged, not replaced as codex alone would — the user's word is merge).
- **Trust** — folded groups are trusted exactly like component groups (U1: attaching the settings asset to a preset is the user's approval). Consequence: a raw hooks line that today is skipped silently (untrusted) **runs** after this step — even with no hook component attached.
- **Paths** — no placeholder rewrite (a settings asset has no folder); the command goes as written.
- **What breaks the assembly** — a hooks line whose value does not parse as TOML or does not have the shape of §3.1 (a table of event arrays / an array of group tables), or a deeper path (`hooks.<Event>.<more>`) that cannot be folded: problem ② `codex-hooks-unreadable` `{preset, line}` at `check`, `Err` in `materialize` (ADR-0353). Everything else passes through (ADR-0359).
- **Dependency** — agent regains `toml` (ADR-0359 decision 「영향」 removed it from agent together with `raw_to_json`); `toml` is already in `Cargo.lock` (three versions), so no new package if the version matches one of them. A crate dependency change — reported per CLAUDE.md.

## 3. Checks, blocks and notices

### 3.1 What asset-library checks (ADR-0359 — only what assembly breaks)

The shape Engram reads, nothing else: UTF-8 JSON (a BOM is dropped — Engram re-serializes); a top-level object whose `hooks` is an object; each event value an array; each group an object whose `hooks` is an array of objects. Failure = an error on that entry (problem ②, code `hook-shape-invalid` `{path, why}`) — it blocks the launch, shows in `preset.check` and the problem list, and never reaches a CLI. Not checked: event names, matchers, handler types and fields, timeouts, script existence — each CLI handles them by its own rules (an unknown event can make codex refuse the config; that is codex's error, passed through).

**Why this check carries the weight (measure-3 M1).** claude contains a bad handler: a wrong-typed field (`"timeout": "30"`) in `--settings` drops **only that handler** — the rest of the file, its `env` and the sibling handler in the same group still load; the same containment holds in a `--plugin-dir` plugin. But the drop is **silent** in an automated session (no stderr, no stream event, exit 0 — only the claude debug log line 「Invalid setting skipped without dialog (automated session)」; an interactive session probably shows a dialog — inferred from that wording). So what Engram must still catch before launch is what would break its own assembly (the structure above, refused per entry); a wrong field inside one handler costs only that handler and is the CLI's rule (ADR-0359), noted in help. `claude plugin validate <dir>` reports such entries token-free for a plugin folder, but there is no equivalent for a settings file (inferred from `--help`), so it offers no pre-launch check for this route.

### 3.2 What the backends check (port answers)

| Answer | claude | codex terminal | codex JSON |
|---|---|---|---|
| `ModeLoads.hooks` | true (both modes) | true | true |
| hook line guard · budget (`command_line_fits` with `CommandLine.hooks`) | — | problem ② (the line guard's code, or a new `codex-hook-unsafe-char`; over budget with hooks → the M7 text of §2.6) | — (JSON body) |
| raw hooks lines in a settings asset (U4) | merged by ADR-0328 (O2) | folded into the combined list (§2.9); unreadable line → problem ② `codex-hooks-unreadable` | same |

`materialize` repeats the codex refusals before any write (ADR-0353); an `Err` there is a failed spawn, as today.

### 3.3 Notices

- `hooks_omitted` — a backend · mode answering `ModeLoads.hooks = false` names the hooks it does not load (no such mode today; tested with the fake third backend; also the interim state between units, §7).
- `codex-hooks-untrusted` (codex JSON) — after the thread opens, the transport asks `hooks/list {threadId}`; every key Engram trusted must be listed `trusted`. A key not trusted, a handler Engram could not trust (non-`command`), or a failed `hooks/list` (an older codex) gives this notice with the hook paths; it never fails the spawn (M4 — no silent skip, no new failure). It shares the handshake budget. The verdict travels out of the spawn with the launch's notices (the `LaunchNotices` the manager builds after the spawn), so it reaches the caller once like the plan's notices; boot restore only logs it (as today). It is outside the notice fingerprint (ADR-0341 dedups the plan's omission set; this is a per-run anomaly).
- Terminal mode has no RPC to ask; a hash drift after a codex update shows in the codex lane test (§5), not at run time (O5).

## 4. LLM path (CLAUDE.md 「LLM-우선 제어」 · M8)

No new command. The existing commands gain the section and the kind: `preset.save` / `preset.check` `--hooks <entries>`, `preset.reorder --section hooks`, `preset.get` and the views carry `hooks`, `library.list --kinds hook`; the `agent.*` preset verbs are unchanged (they attach presets). Bindings and `commands.schema.json` regenerate (CI sync gate); `tests/command_declarations.rs` counts six section words.

`engram help library` gains: `hook` in the kind table (「a folder holding hooks.json」), the example preset's `hooks:` list, the tag rule (「instructions, skills and hooks」), and a hooks paragraph — the shape; `${CLAUDE_PLUGIN_ROOT}` is rewritten to the hook folder; write commands as `node "${CLAUDE_PLUGIN_ROOT}/x.js"` because on Windows claude runs hooks in Git Bash and codex in PowerShell (M6); codex runs only `type: command` hooks from Engram and trusts them per run; a command with `% & ^ | < >` cannot go on the codex terminal line; **one line on the terminal-mode limit** (M7 (2) — codex terminal carries hooks on its command line, which has a fixed length limit, about 20 typical hooks; over it the launch is refused — reduce hooks or use JSON mode); SessionStart also runs when a conversation resumes (`source: "resume"`) — use `matcher: "startup"` for fresh starts only; a handler with a wrong field type is skipped silently by claude (only that handler); a field only one backend knows belongs in a hook tagged for that backend; after writing files with your own tools run `library.refresh`. All English; no UI text, so no Korean key (ADR-0372 does not apply).

## 5. Test plan

`.claude/skill-bindings/qa.md` 「돌릴 것 표」: **Rust 소스** (every unit) → full regression. **Lanes** — U1 touches `crates/engram-dashboard-asset-library/src/**` (both lanes); U2 `backend/claude/**` (claude); U3 `backend/codex/**` (codex) and possibly `platform/src/shell.rs` (both). **GUI** — on for U3 (the codex app-server handshake = the 「에이전트 수명 · 출력 경로 … transport」 row); off for U1, U2, U4; the closing scene H1 in U5. `prompts/engram-help.md` and the bindings change commands → 「바뀐 명령만 문자 그대로 한 번」 (scene H1 covers it). Mutation checks per qa 「변이 확인」 on rows marked †.

| What | How | Unit |
|---|---|---|
| Scan and kind | `catalog.rs` tests: `hooks.json` in a folder → `Hook` + `HookFile`; OS-rule spelling; at a mount's top → nothing; with `SKILL.md` → still a skill †; `library.list --kinds hook` | U1 |
| Section | reader / writer round trip with tags; `wrong-kind`; `backend-tag-not-allowed` no longer for hooks; `preset.reorder` / `save` / `check` with `hooks`; relink and move rewrite a hook reference; `command_declarations` six words | U1 |
| Resolve | shape errors → `hook-shape-invalid` †; dedup at first position; tag filter; plan order (common, then backend-tagged); the stamp catches a changed `hooks.json`; `hooks_omitted` with the fake backend answering false; the ADR-0367 gate (`rg`) | U1 |
| Placeholder | every string rewritten, forward slashes, other variables untouched, identity off Windows (platform test) † | U2 |
| claude merge | golden `settings.json`: raw settings' hooks first, components after, identical groups once; `settings_file` written with hooks and no raw settings; Noop argv golden unchanged (`tests/launch_argv_golden.rs`) | U2 |
| codex lines | hash goldens (plain, matcher + timeout, statusMessage + async, commandWindows, SessionEnd default timeout) †; key spelling, escaped and raw †; TOML writer (literal vs basic, control escapes, even quotes); one `hooks.state` line; event order fixed; guard refusals → problem ② at `check` and `Err` in `materialize` †; the budget count at `check` equals `materialize`'s; the Rust total of today's line pinned; the over-budget text with hooks names cause and remedy (M7), without hooks unchanged †; raw hooks folding (U4): `hooks.<Event>=` and `hooks={…}` lines leave `config_args` and join the combined list settings-first †, a raw `hooks.state` dropped, folded groups get trust entries †, an unparsable line → `codex-hooks-unreadable` | U3 |
| codex JSON | `thread/start` and `thread/resume` carry `config.hooks` (absent without hooks — byte-identical params); `hooks/list` verdicts → notice / none; a failed `hooks/list` → notice, the spawn succeeds; ADR-0216 tests still green | U3 |
| codex live (lane, `#[ignore]`, no tokens) | in `tests/codex_launch_live.rs`: (a) `codex app-server` + `thread/start` with Engram's `config` → `hooks/list {threadId}`: keys and `currentHash` equal Engram's and every hook `trusted`; (b) `codex app-server` with Engram's terminal lines as `-c` → `hooks/list {cwds}` the same — a codex change to the hash, key or override rules turns the lane red | U3 |
| claude live (lane) | `launch_kits_live` `claude_`: a preset with a hook component → the hook's marker file is written (`-p`, haiku) | U2 |
| Scene H1 (GUI 실측) | isolated daemon; a library hook `team/hooks/marker` (`SessionStart` + `UserPromptSubmit`, `node "${CLAUDE_PLUGIN_ROOT}/mark.js" <tag>`) attached with `engram preset.save --hooks …`; launch claude terminal, claude JSON, codex terminal, codex JSON, one prompt each → a marker line per backend · mode; codex terminal shows no review screen and no bypass warning; restart the codex JSON agent (resume) → marker again; `~/.codex/config.toml` byte-identical before and after; a codex settings asset with `hooks.Stop=[…]` plus the component → both hooks run in both codex modes, trusted (U4); a hook command with `|` → problem ② on codex terminal; enough hooks to pass the budget → the refusal text of M7 | U5 (after U1–U4) |

## 6. Implementation units

Sequential — U1–U3 share `launch.rs` / `settings_check.rs` and the fakes; a new port field breaks every implementor, so each unit carries its implementors and test doubles. Green at each end. Sizes include tests (about half).

| Unit | Content | Lines |
|---|---|---|
| U1 | asset-library kind + section + resolve; agent `HookRef`, `LaunchPlan.hooks`, `ModeLoads.hooks`, `NoticeCodes.hooks_omitted`, `CommandLine.hooks` (claude and codex answer `loads.hooks = false` for now, so an attached hook is **named as omitted, never dropped silently** until its backend unit); `PresetSection::Hooks`, `LibraryKind::Hook` / `HookFile`, command args, bindings and schema; the help lines of §4 that name the section (the help-screen tests); tests. | ~1,400–1,800 |
| U2 | Placeholder rewrite (neutral) + platform slash spelling; claude hook layers in the merged settings; claude `loads.hooks = true`; claude lane test. | ~400–600 |
| U3 | codex: hash, key, TOML writer, terminal lines, guard and budget through `command_line_fits` with the M7 refusal text, raw hooks folding with the TOML reader and the `toml` dependency (U4), `hook_args` / `hook_config`, thread params `config`, post-open `hooks/list` check and notice, codex `loads.hooks = true`, `sha2`, ADR-0216 test docs; codex live lane tests. | ~1,200–1,600 |
| U4 | Only if O4 = include: the lone-marker confirmation for `hooks.json` in `ops.rs` (the `SKILL.md` rule generalized; code `hook-unconfirmed`; help line). | ~300–500 |
| U5 | The hooks help paragraph (with the terminal-limit line) and the priming root line, reference files and headers (the 「Reference files to fix」 row), scene H1. | ~200 |

## 7. Interim state

Between U1 and U2 / U3 an attached hook is carried by neither backend; the omission notice names it (U1's `false` answers), so no launch drops a hook silently at any commit. U2 and U3 flip their backend's answer in the same commit that carries the hooks.

## 8. Hand-over to the client stage

- The preset edit area gains a `hooks` field (rows, backend tag, reorder) on the same `preset.save` / `check` (parent §1-10 · step 13); the library tree shows hook folders like skill folders (marked `hook`); the problem list shows `hook-shape-invalid`, `codex-hooks-unreadable` and the hook line refusals; the launch report shows `hooks_omitted` and `codex-hooks-untrusted`. Their LLM path is the commands of §4 — no new command.
- Written once more in the client step's TRD at the settling commit.

## 9. Decisions

### 9.1 User decisions (2026-11-24)

- **U1 — Codex hook trust: Engram trusts only the hooks it attaches.** Attaching a hook to a preset is the user's approval — 「우리건데?」 (no need to ask for our own hooks). Not the bypass flag (`--dangerously-bypass-hook-trust` / `bypass_hook_trust`, which also trusts every other unreviewed hook and warns on every run); not a review prompt. Convention: Zed codex-acp, Orca and context-mode trust only their own injected hooks with a hash re-check; atuin: 「the user asked for atuin's hooks by installing them」; Warp and hapi use the bypass flag for unattended runs.
- **U2 — The trust goes per run, in the session-flags layer** (terminal: one `-c hooks.state=…` line; JSON: inside the thread config), never into `~/.codex/config.toml`. 「-c 가 그럼 더 좋다는거아냐? 유동적이고」. Reason: Engram combines hooks per agent and the session-flags keys are positional (`<session-flags>:<event>:<group>:<handler>`), so a persisted entry would be overwritten by the next agent's combination; per-run trust keeps agents independent and leaves nothing behind. Zed and Orca persist (config / `config/batchWrite`) because they inject one fixed set. Risk: undocumented but source-honoured (`config_rules.rs` reads `hooks.state` from the SessionFlags layer); no other tool found using it (§12 R2).
- **U3 — Engram's own past codex hook is not revived** (the SessionStart session-id hook of ADR-0208 / ADR-0210, removed by ADR-0216), and nothing is added to the built-in preset — 「ㅇㅇ 그대로가」.
- **U4 — A codex settings asset's raw hooks lines are merged into the combined hook list** (§2.9) — 「합치기가 맞고」. Problem: the last `-c` for a path replaces (measure-2 M4) and in JSON mode the thread-config `hooks` object wipes the app-server line's `hooks.*` (M1 a2), so raw hooks lines and components would erase each other. Options offered: (a) refuse the launch with guidance (move the raw hooks into a hook component) — ★ main's recommendation; (b) merge — Engram reads the raw lines and folds them in; (c) ours win + a notice. The user chose (b), **갈림** from the recommendation. Consequences: TOML reading lives only in the codex backend and agent regains the `toml` dependency (§2.9; ADR-0359 amendment point — §10); raw hooks lines are trusted like components and run where they were skipped silently before; claude already merges through ADR-0328 (O2 stands); the `codex-hooks-collide` refusal of draft 1 is gone.

### 9.2 Main's decisions (defaults, reported)

- **M1** A hook component = a library folder holding `hooks.json` (the shared claude/codex shape) plus its scripts, recognised by that marker like `SKILL.md` (rules in §2.1: OS-rule name, below a mount's top, `SKILL.md` wins in a shared folder). Rejected: per-backend definition files in one folder (two sources of truth for one hook; backend tags already target one backend).
- **M2** Script paths through `${CLAUDE_PLUGIN_ROOT}`, rewritten by Engram to the hook folder for both backends (§2.4 — forward slashes is this TRD's spelling choice). No schema check (ADR-0359).
- **M3** claude route = the merged `settings.json` `hooks` key through the existing one `--settings` (ADR-0328). Rejected: `hooks/hooks.json` in the `--plugin-dir` folder (works, measured; `CLAUDE_PLUGIN_ROOT` would point at the volatile plugin folder, not the hook folder; two mechanisms). Measured 2026-11-24 (measure-3 M1, closes O6): a wrong-typed handler in `--settings` drops only that handler — other settings, `env` and sibling hooks survive — the same containment as in a plugin, so the merged route does not couple one bad hook with the rest of the settings; the drop is silent (§3.1).
- **M4** codex route = terminal: one `-c hooks.<Event>=[…]` line per event + one `-c hooks.state={…}` line after the preset `-c` lines; JSON: one `hooks` object (events + state) in the thread config. Hash computed by Engram with codex's algorithm, no probe app-server (a throwaway app-server costs ≈ 330–370 ms per launch, measure-2 M3); a codex lane test compares with `hooks/list` on the installed codex. JSON mode checks `hooks/list {threadId}` after the thread opens and gives a notice if ours are not trusted. Trust key escaped `\u003c` / `\u003e` for the cmd guard.
- **M5** Preset section `hooks` with `backend:` tags like skills; order = preset order.
- **M6** The Windows shell difference (claude Git Bash, codex PowerShell 7) is passed through; help recommends `node "<script>"`.
- **M7** Terminal budget as §2.6: today about 2542 / 8191 units, about 20 typical hooks fit; over budget = the existing fail-closed refusal (ADR-0353), no truncation. User-facing policy (main 2026-11-24): no proactive warning; (1) the refusal text names cause and remedy (「codex terminal command line limit exceeded — reduce hooks or use JSON mode」) — this step; (2) one line on the terminal-mode limit in the hooks part of `engram help library` — this step. No preset-editor warning (user 2026-11-24 — the refusal text is the guidance; JSON mode has no limit).
- **M8** Screen and its LLM path for the hooks field come with the client stage (§8); this step's LLM path = the existing `preset.*` / `library.*` commands gain the section (§4).
- **M9** asset-library stays backend-blind (ADR-0367): `ModeLoads.hooks` and `NoticeCodes.hooks_omitted` are port answers; per-backend layout only in each `materialize`.
- **TRD-level choices** (this draft): T1 thread config also on `thread/resume` — measured: with it the hook is trusted and fires (`source: "resume"`), without it the resumed thread has no hooks (measure-3 M2; §11 C1); T2 hooks not copied into the volatile folder (§2.3); T3 `hook_args` / `hook_config` as fields of their own, `config_args` unchanged in meaning; T4 trust entries only for `type: "command"` (§2.8); T5 the post-open notice travels with the launch notices and stays outside the fingerprint (§3.3); T6 U1 answers `loads.hooks = false` for both backends until their unit (§7); T7 the shape check is part of Engram's component format (asset-library `format/hooks.rs`), like the preset reader — not a backend schema check.

### 9.3 Open items (recommendation each)

- **O1 — decided by the user (U4, §9.1): merge.**
- **O2 — claude: settings preset `hooks` + hook components.** Merge (ADR-0328 concatenation) is fine: claude's hook sources are additive by design, and identical groups collapse as claude collapses identical handlers across settings sources (cli-hooks A1). Recommend: no rule — close as 「merge」.
- **O3 — Which modes load hooks.** claude both modes; codex terminal and JSON (terminal within the budget). Recommend as written; `hooks_omitted` exists for a backend · mode that answers `false` (none today) and for the interim state (§7).
- **O4 — File operations with references on a hook folder.** Whole-folder move / rename / copy / delete with reference handling come free once the kind maps to its list (U1). Left: renaming `hooks.json` or moving it out of a referenced hook folder makes the hook vanish (references then read as not found — a visible launch block, not silent). Recommend **include (U4)**: the same confirmation as the lone `SKILL.md` (ADR-0339 — the user's rule that a reference follows whether the agent still recognises the item, 「skill.md가 없으면 스킬 자체가없어지는거잖아. 그럼 참조를 빼야되는거 아님?」, applies unchanged), with its own code `hook-unconfirmed` (10b's codes are final, so `skill-unconfirmed` is not widened). Alternative: defer to `docs/tracking.md`.
- **O5 — Left unmeasured.** (1) claude interactive (TTY) launch with `--settings` hooks — no prompt documented; (2) claude pass-through `--bare` / `--safe-mode` / `disableAllHooks` switch hooks off (the user's own choice; the preview-style risk of parent §6-3); (3) — measured since (codex `thread/resume` with the config: trusted, fires; without it: no hooks — §2.7, T1); (4) codex terminal has no RPC check — a drift shows only in the lane; (5) a user-layer `hooks.state` entry on a session-flags key (e.g. an old `enabled = false` from a TUI review) may override ours. Recommend: (1) in scene H1, and scene H1 re-checks resume through Engram's own transport; (2) and (5) recorded as risks; (4) accepted with the lane test.
- **O6 — closed 2026-11-24 (measure-3 M1).** Draft 1 asked whether one malformed hook in the merged `--settings` drops every setting (ADR-0359's older measurement). Measured on claude 2.1.330: a wrong-typed handler drops only that handler, in `--settings` and in a plugin alike; other settings, `env` and sibling hooks survive. M3 kept; the silent drop is answered by Engram's shape check (§3.1, T7) and a help line (R1).

## 10. ADR proposals (numbers by `/engram-adr`)

1. **New — hook components and codex per-run trust**: the component (folder + `hooks.json`, §2.1), the section with tags and order, the placeholder rewrite, the claude route (M3 — merged `--settings`; bad-handler containment measured), the codex lines / thread config, per-run session-flags trust computed by Engram with no bypass and no persisted state (U1, U2), the post-open notice, the shape check as Engram's format (consistent with ADR-0359), the M7 budget policy, the raw-hooks merge and its trust (U4).
2. **Amend ADR-0216** — codex hooks come back in another form: user hook components with per-run trust; its rule 「do not revive the session-id hook」 stays (U3); the no-hook tests narrow to 「no hook without a component」.
3. **Amend ADR-0328** (and ADR-0316 decision 3) — the merged settings gain one layer per hook component after the raw settings.
4. **Amend ADR-0353** — the fail-closed list gains: a hook line the guard refuses, the line over budget with hook lines (with the M7 text), and a codex raw hooks line Engram cannot fold (`codex-hooks-unreadable`, U4).
5. **Amend ADR-0359** (its 「영향」 bullet that removed agent's `toml` with `raw_to_json`) — agent regains `toml`, used only by the codex backend to read raw hooks lines for the merge (U4); still no schema check.
6. If O4 = include — a line in ADR-0339's lineage (the lone-marker rule covers `hooks.json`; code `hook-unconfirmed`).

## 11. Input claims corrected (brief and facts → code at `7528e4ae`)

- **C1** The brief's JSON route names only `thread/start`. A codex JSON agent that resumes opens its thread with `thread/resume` (`transport.rs` `handshake`, `ThreadOpen::Resume`), and the thread config is per thread — without the config on resume a resumed agent runs with no hooks. This TRD carries it on both (T1); measured since (measure-3 M2): with the config the resumed thread's hook is trusted and fires with `source: "resume"`, without it the thread has no hooks.
- **C2** Parent item 22 lists the codex trust choices as 「stored per definition hash, or `--dangerously-bypass-hook-trust`」 — replaced by per-run session-flags trust (U1, U2).
- **C3** `codebase.md`: 「a volatile script path would need re-approval every launch; a stable path trusts once」 — moot: trust is per run, and script paths are library paths (stable) after the rewrite.
- **C4** Draft 1 raised ADR-0359's older measurement (claude discards a whole bad `--settings` object) against M3 (O6). measure-3 M1 on claude 2.1.330 shows a wrong-typed hook handler drops only that handler; for hooks the older claim does not hold. O6 closed, M3 kept.
- **C5** measure-2 M5's 2542 units is a JavaScript replica of `cmdline.rs`'s count, not a number Rust printed (its own caveat); U3 pins the Rust total in a test.
- **C6** `codebase.md` lists `LaunchArtifacts {skill_roots, settings_file, add_dirs, env, config_args}` — the struct also has `args`, `instructions_file`, `instructions_override`, `instructions_text`, `instructions_changed`; no design depends on the shorter list.
- **C7** `library.newFile` makes only `Markdown` · `Settings` · `Preset` — there is no skill-creation verb either; this step adds no hook-creation verb (§0 Out).
- Checked true: `FileKind::of` / `SKILL_FILE` OS-rule naming, `ListName` (five lists, `takes_tags` = instructions and skills), `ModeLoads {instructions, skills}`, `NoticeCodes {omitted, folder_omitted, same_name}`, `merge` / `merge_two` / `without_repeats`, the cmd guard characters, the ADR-0216 no-hook tests (`hook_override_value`), `SKILL_UNCONFIRMED` / `lone_skill_file` in `ops.rs`, the shim-margin wrapper of 571 units.

## 12. Risks

- **R1** A hook handler with a wrong field type is dropped by claude silently (only that handler — measure-3 M1); Engram does not check handler fields (ADR-0359), so the user learns it only from claude's debug log or, interactively, probably a dialog. Help names the rule.
- **R2** Per-run trust relies on undocumented, source-honoured codex behaviour (`hooks.state` read from the SessionFlags layer — `config_rules.rs`); a codex release can change it, the hash, the key spelling or the override replace rule. Mitigated by the live codex lane (§5) and, in JSON mode, the run-time `hooks/list` notice; terminal mode would show codex's startup review screen (visible, blocking until answered — the ADR-0216 cost) until fixed.
- **R3** A user-layer `hooks.state` entry on a session-flags key (left by an earlier TUI review) may disable or override ours (O5 (5)).
- **R4** Fields only one backend knows (`commandWindows`, claude's `shell` / exec-form `args`, claude-only events) may break the other backend's config — passed through (ADR-0359); help tells users to tag such hooks.
- **R5** Shell semantics differ per backend on Windows (Git Bash vs PowerShell 7 — measured); a bash-only command fails under codex (M6).
- **R6** codex terminal budget: about 20 typical hooks, fewer with long paths and preset `-c` lines; over budget refuses the launch with the M7 text; there is no warning before a launch (user 2026-11-24) — the user, an LLM or the CLI learns it from the refusal text (cause and remedy) or from `preset.check`.
- **R11** U4 makes raw codex hooks lines that were skipped silently (untrusted) start running, trusted per run, once this step ships; a user who kept such lines inert is surprised. Release note and help line.
- **R7** Scripts run in place from the library; editing them affects running agents; a hook folder on an unavailable mount blocks the launch like any missing item.
- **R8** Non-`command` handlers are not trusted for codex: JSON names them in a notice; terminal shows codex's review screen at startup.
- **R9** The post-open `hooks/list` adds one round trip to every codex JSON launch with hooks (inside the handshake budget).
- **R10** Bindings grow (section word, kinds, `hooks?` args) — review cost; the help-screen tests change counts.

## 13. Review record

(empty — not reviewed)
