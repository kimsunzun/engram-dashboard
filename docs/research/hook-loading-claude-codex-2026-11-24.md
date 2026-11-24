# CLI hook loading: Claude Code 2.1.330 and codex-cli 0.171.0

Date: 2026-11-24. Machine: Windows 11, Git for Windows installed (bash 5.2.37), PowerShell 7.5.5.
Confidence tags: **[measured]** = observed on this machine in this session; **[documented]** = official docs or the codex source at tag `rust-v0.171.0`; **[inferred]** = my reading, not verified.

## TL;DR for the Engram design

| | Claude Code | Codex |
|---|---|---|
| Route that matches what Engram already does | Put `hooks/hooks.json` (plus the scripts) inside the plugin folder Engram already passes with `--plugin-dir`. The command uses `${CLAUDE_PLUGIN_ROOT}`. **[measured]** | Pass the hook definition as config overrides: `-c hooks.<Event>=[...]` (terminal) or `thread/start.config.hooks` (app-server). These land in the **sessionFlags** layer. **[measured: -c / documented: app-server]** |
| Second route | `--settings <file|json>` with a `hooks` key. Scripts must be referenced by absolute path, because `CLAUDE_PLUGIN_ROOT` is not set for settings hooks. **[measured]** | A plugin hook (`hooks/hooks.json` in an *installed* plugin). Codex has no `--plugin-dir`-style session flag, so this needs a marketplace install, which persists. **[documented/inferred]** |
| Combining with the user's own hooks | Additive. User, project, local, `--settings` and plugin hooks all run. Identical handlers across settings files run once, and a plugin copy is kept separate. **[measured + documented]** | Additive across all layers (system, user, project, sessionFlags, plugins). **[documented: source]** |
| Trust gate | None for `--plugin-dir` or `--settings` hooks. They ran under `-p` with no prompt. **[measured]** Interactive: no trust step for these routes is documented. **[inferred]** | **Yes.** Every non-managed hook, sessionFlags ones included, must be trusted by its definition hash, or it is skipped *silently*. **[measured]** There are 3 ways past it (below). |
| Shell on Windows | Git Bash by default (`$0=/usr/bin/bash`). PowerShell if Git Bash is missing. Per-hook `"shell": "bash"|"powershell"`, or exec form with `args`. **[measured + documented]** | The session's shell. On this machine that was **PowerShell 7.5.5**, not cmd. It falls back to `%COMSPEC% /C` only when no session shell is known. A `commandWindows` override field exists. **[measured + documented: source]** |
| Referring to its own script | `"${CLAUDE_PLUGIN_ROOT}"/scripts/x.js` (plugin), or an absolute path (settings). `$CLAUDE_PROJECT_DIR` = the session cwd. **[measured]** | Absolute path (sessionFlags hooks get no root variable). Plugin hooks get `PLUGIN_ROOT`/`PLUGIN_DATA` plus `CLAUDE_PLUGIN_ROOT`/`CLAUDE_PLUGIN_DATA` as compatibility copies. **[documented: source]** |

Recommended shape for a library "hook" component, inferred from the above: a folder holding `hooks.json` (in the Claude/Codex shared event-map shape) and a `scripts/` subfolder.
- **Claude:** copy or merge it into the assembled plugin folder as `hooks/hooks.json` and `scripts/`, using `${CLAUDE_PLUGIN_ROOT}`.
- **Codex:** turn the same event map into `-c hooks.<Event>=[...]` (or `thread/start.config.hooks`), with `${...}` rewritten to the absolute path of the scripts. Then either pass a pre-trust `hooks.state` override, or use bypass.

The two CLIs' JSON schemas are almost identical (`{hooks:{Event:[{matcher?, hooks:[{type:"command", command, timeout?, statusMessage?, async?}]}]}}`). Codex even sets `CLAUDE_PLUGIN_ROOT` for plugin hooks "for OOTB compat". **[documented]**

---

## A. Claude Code 2.1.330

### A1. Loading routes and how they combine

- **Locations [documented]** (https://code.claude.com/docs/en/hooks, "Hook locations"):
  - `~/.claude/settings.json`, `.claude/settings.json` and `.claude/settings.local.json`
  - managed policy
  - plugin `hooks/hooks.json`
  - skill frontmatter and subagent frontmatter
  - `--settings <file-or-json>`: the "Command line" tier, above user/project/local and below managed (https://code.claude.com/docs/en/settings).
- **Merge, not replace [documented]:** "Hook entries merge across settings levels rather than replacing each other." "If you define the same handler in more than one settings file, it runs once. A plugin's or skill's copy of the same handler stays separate." Matching hooks run in parallel.
- **Merge, measured [measured]:** three `claude -p "say ok" --model haiku --output-format stream-json --verbose --include-hook-events` runs from an empty cwd under `%TEMP%\engram-hook-probe-*\work`.
  - Run 1, `--plugin-dir <probe plugin>`: SessionStart fired 4 handlers = 2 probe plugin + 1 user `~/.claude/settings.json` + 1 from the installed `codex@openai-codex` plugin. UserPromptSubmit fired 2 = probe plugin + user. The plugin showed in `init.plugins` as `engram-hook-probe@inline`. Markers written: yes.
  - Run 2, `--settings <file>` with the same hooks: 4 SessionStart (2 settings + user + codex plugin) and 2 UserPromptSubmit. Markers written: yes.
  - Run 3, `--plugin-dir` **and** inline `--settings '<json>'`: UserPromptSubmit fired 3 = plugin + inline settings + user. So plugin, `--settings` and user hooks all stack.
- **Plugin manifest [documented]** (https://code.claude.com/docs/en/plugins-reference):
  - `plugin.json` `hooks` takes a path, an inline object, or an array of either. It is "Loaded together with `hooks/hooks.json`" (it merges, it does not replace).
  - Component paths must start with `./` and stay inside the plugin root.
  - A hooks *file* needs the top-level `"hooks"` wrapper.
  - The manifest is optional. The name falls back to the directory name with `--plugin-dir`.
- **No trust prompt [measured]:** `--plugin-dir` and `--settings` hooks ran under `-p` with no prompt. **[documented]** `-p` skips the workspace trust dialog, and settings-file hooks are "Used" even in an untrusted folder under `-p` (permissions doc, "What runs before you trust a folder"). A `--plugin-dir` plugin is "On for the session unless the manifest sets `defaultEnabled: false` or a settings file sets `"<name>@inline": false`" (https://code.claude.com/docs/en/plugins/loading). Interactive mode: no extra trust step for `--plugin-dir` or `--settings` hooks is documented. **[inferred, not measured]**
- **Ways the user can switch these hooks off [documented]:**
  - `disableAllHooks` (effective value after precedence)
  - managed `allowManagedHooksOnly`
  - `--safe-mode`
  - `--bare`: skips "hooks (those defined in settings and by installed plugins)", yet the same help text lists `--settings`/`--plugin-dir` as the way to supply context under `--bare`. Whether `--settings`/`--plugin-dir` hooks run under `--bare` was **not measured**.
- **`--plugin-dir` loads in place [documented + measured]:** "the directory loads in place and is never copied". Measured `CLAUDE_PLUGIN_ROOT` = the source folder itself.

### A2. Path variables and Windows

- **Plugin hook process [measured]** (probe env dump):
  - `CLAUDE_PLUGIN_ROOT=C:/…/plugin` (forward slashes)
  - `CLAUDE_PLUGIN_DATA=C:/Users/<u>/.claude/plugins/data/engram-hook-probe-inline`
  - `CLAUDE_PROJECT_DIR=C:/…/work` (the session cwd)
  - `CLAUDE_ENV_FILE` (SessionStart only)
  - hook cwd = the session cwd
  - stdin JSON = `{session_id, transcript_path, cwd, hook_event_name, source}`
- **Settings hooks [measured]:** `CLAUDE_PLUGIN_ROOT` is **absent**, so use absolute paths. Oddity: `CLAUDE_PLUGIN_DATA` was present but pointed at the *codex* plugin's data dir. This looks like an env leak from another plugin, so do not rely on it in settings hooks.
- **Side effect [measured]:** `~/.claude/plugins/data/engram-hook-probe-inline/` was created on first reference. Docs: "created on first reference and kept across plugin updates". Every distinct inline plugin name will leave such a folder.
- **Shell [measured]:** the shell-form probe wrote `0=/usr/bin/bash bash=5.2.37(1)-release`, so it ran through Git Bash. **[documented]**:
  - "Shell form … Git Bash on Windows, or PowerShell when Git Bash isn't installed. Set the `shell` field to choose explicitly" (`"bash"|"powershell"`).
  - Exec form `{"command":"node","args":["${CLAUDE_PLUGIN_ROOT}/x.js"]}` skips the shell. On Windows the `command` must be a real `.exe` (`.cmd` shims fail).
  - In shell form, double-quote every placeholder. `claude plugin validate` warns about an unquoted one.
  - On Windows, substituted paths use forward slashes, except in exec form and PowerShell hooks, which keep backslashes.
  - `CLAUDE_CODE_GIT_BASH_PATH` overrides where Git Bash is found.
- **Recommendation [inferred]:**
  - For portable library hooks, prefer exec form `command: "node", args: ["${CLAUDE_PLUGIN_ROOT}/scripts/x.js"]`. It does not depend on Git Bash being present.
  - Do not put the `hooks/` directory itself behind a junction that points outside the plugin root. Docs reject "A symlink that leads outside the plugin" for component paths. Engram's current skill junctions evidently work, but I did not measure a junctioned `hooks/`.
  - Script paths inside a `command` string are not component paths and are not containment-checked. **[inferred]**

### A3. Measurement record [measured]

- Probe folder: `%TEMP%\engram-hook-probe-1795498357479\` with `plugin/.claude-plugin/plugin.json`, `plugin/hooks/hooks.json`, `plugin/scripts/mark.js` and `settings.json`. Recycled after the runs.
- Cost: 3 runs on `claude-haiku-5-5`, ≈ $0.0047 in total.
- Session records left (not deleted, as instructed): `~/.claude/projects/C--Users-<u>-AppData-Local-Temp-engram-hook-probe-1795498357479-work/` holds 3 `.jsonl` files (`11b60700-…`, `3d24625c-…`, `f2477955-…`).
- Also created: `~/.claude/session-env/<session-id>/` (CLAUDE_ENV_FILE) and `~/.claude/plugins/data/engram-hook-probe-inline/`. The latter was recycled (see cleanup).
- Nested launch: these were `claude -p` runs started from inside a Claude Code session, with `CLAUDECODE` unset. Engram's own launches are not nested.

---

## B. Codex CLI 0.171.0

### B1. Support, locations, events, execution

- **Supported and on by default.** `codex features list` → `hooks  stable  true`. User config also has `[features] hooks = true`. `plugin_hooks` shows as `removed` (the flag is gone and plugin hooks are always on). **[measured]**
- **Locations [documented]** (https://learn.chatgpt.com/docs/hooks, formerly developers.openai.com/codex/hooks, plus source):
  - `hooks.json` in system `%ProgramData%\OpenAI\Codex\`, user `$CODEX_HOME/hooks.json` (`~/.codex/hooks.json`), and trusted-project `.codex/hooks.json`.
  - Or inline `[hooks]` tables in each layer's `config.toml`. Both forms in one layer load together, with a warning.
  - Enabled plugins: `hooks/hooks.json` in the plugin root, or the `hooks` field in `.codex-plugin/plugin.json`. Here the manifest entry **replaces** the default file. This is the opposite of Claude.
  - Managed hooks: `requirements.toml`/MDM/cloud.
- **Source:**
  - Discovery iterates **every config layer**, including `SessionFlags` (-c / request overrides). `codex-rs/hooks/src/engine/discovery.rs` L157-221 and L1048 (`ConfigLayerSource::SessionFlags => (HookSource::SessionFlags, false)`, i.e. not managed).
  - Plugin hooks: `codex-rs/core-plugins/src/loader.rs` L75 `DEFAULT_HOOKS_CONFIG_FILE = "hooks/hooks.json"`.
- **Events [documented: `HookEventName.ts` / `protocol/src/hooks.rs`]:** PreModelRequest, PostModelRequest, PreToolUse, PermissionRequest, PostToolUse, PreCompact, PostCompact, SessionStart, SessionEnd, UserPromptSubmit, SubagentStart, SubagentStop, Stop, Interrupt.
- **Handler fields:**
  - `type: "command"` with `command`, `commandWindows`/`command_windows`, `timeout` (sec, default 600; SessionEnd/Interrupt default 1 s, max 3 s), `async`, `statusMessage`, `additionalContextLimit`
  - `type: "mcp_tool"`
  - `prompt` and `agent` exist in the schema but are not supported.
  - `hooks.json` top level: `{description?, hooks:{…}}` with `deny_unknown_fields` (`codex-rs/config/src/hook_config.rs`).
- **Execution [documented: `codex-rs/hooks/src/engine/command_runner.rs` L448-526, `core/src/session/mod.rs` L6196-6224]:**
  - The program is the session's shell (`shell.derive_exec_args`).
  - Only if no shell is known: `%COMSPEC% /C "<cmd>"` (fallback `cmd.exe`) on Windows, `$SHELL -lc` on Unix.
  - cwd = the session cwd. stdin = event JSON.
- **Measured on this machine [measured]:** `echo $PSVersionTable.PSVersion %COMSPEC%` printed `7 5 5` followed by the literal `%COMSPEC%`, so hooks ran in **PowerShell 7**, not cmd. Hook env = Codex's full inherited env. No CODEX_* variable was added for non-plugin hooks. stdin JSON = `{session_id, transcript_path (null with --ephemeral), cwd, hook_event_name, model, permission_mode, source}`.
- **Plugin hook env [documented: discovery.rs L312-320]:** `PLUGIN_ROOT`, `CLAUDE_PLUGIN_ROOT`, `PLUGIN_DATA`, `CLAUDE_PLUGIN_DATA`.
- **Implication [inferred]:** a library hook written for Git Bash (Claude's default) will **not** run unchanged under Codex on Windows, because Codex uses PowerShell here. Options:
  - Make the command a plain `node "<abs>/x.js"`. That works in both bash and PowerShell, and was **measured** in both.
  - Or supply `commandWindows`.

### B2. Trust

**Model [documented + source]:**
- "Before a non-managed hook can run, Codex requires you to review and trust the exact hook definition. Codex records trust against the hook's current hash, so new or changed hooks are marked for review and skipped until trusted."
- Managed hooks (system/MDM/cloud/requirements) are trusted by policy.

**Where trust is stored [documented: `codex-rs/config/src/hook_config.rs`, `codex-rs/hooks/src/config_rules.rs`]:**
- In config TOML: `[hooks.state."<key>"]` with `enabled`, `trusted_hash`, and `trusted_content_hash` (plugins).
- Read **only from the User layer (`~/.codex/config.toml`) and the SessionFlags layer**. Project/managed/plugin layers "do not get to write user hook state".
- Key format: `<sourcePath>:<event_snake>:<groupIdx>:<handlerIdx>`. For a plugin, the source part is `<pluginId>:<relpath>` (`declarations.rs` L35).
- Hash: sha256 over a normalized `{event_name, matcher, hooks:[handler]}` (`discovery.rs` L983-1000). It covers the **command string and all handler fields, not the script contents**.
- Plugin hooks also pin a content hash of the whole plugin folder (`engine/plugin_fingerprint.rs`). Any script edit forces a re-review. A legacy baseline lives in `~/.codex/.hook-trust/<sha>` (`engine/plugin_trust.rs`).
- `plugin_fingerprint.rs` uses `symlink_metadata` and returns `unsupported plugin entry type` for anything that is neither a file nor a dir. On Windows a junction or symlink inside a Codex plugin folder will therefore make fingerprinting fail, and the hook is skipped "failed to fingerprint plugin contents" unless trust is bypassed. **[inferred from source; not measured]**

**SessionFlags hooks, measured token-free via the app-server `hooks/list` [measured]:**
- Command: `codex app-server -c 'hooks.SessionStart=[…]' -c 'hooks.UserPromptSubmit=[…]'`, then `hooks/list {cwds:[work]}`.
- Each hook is listed with:
  - `source: "sessionFlags"`
  - `sourcePath: "C:\\<session-flags>\\config.toml"` (a synthetic, stable path)
  - `key: "C:\\<session-flags>\\config.toml:session_start:0:0"`
  - `currentHash: "sha256:82f7…"`
  - `trustStatus: "untrusted"` and `enabled: true`

**Three ways to make them run:**

1. **`--dangerously-bypass-hook-trust`** (TUI root, `exec`, other subcommands sharing `utils/cli/src/shared_options.rs` L61-64).
   - **[measured]** `codex exec` printed `warning: \`--dangerously-bypass-hook-trust\` is enabled. Enabled hooks may run without review for this invocation.` (twice), then `hook: SessionStart … Completed`, and the markers were written. The TUI shows the same text as a ⚠ startup warning (snapshot `tui/src/snapshots/codex_tui__app__tests__bypass_hook_trust_startup_warning.snap`).
   - It still respects `enabled=false`.
2. **App-server equivalent: `thread/start` (or `resume`/`fork`) `params.config.bypass_hook_trust = true`.** `codex-rs/app-server/src/config_manager.rs` L679-686 pops it from the request overrides into `typesafe_overrides.bypass_hook_trust`. `exec` itself forwards the flag this way (`exec/src/lib.rs` L1635-1639). **[documented: source; not measured]**
3. **Pre-trust per hook through the same session layer:** `-c "hooks.state={'C:\\<session-flags>\\config.toml:session_start:0:0'={trusted_hash=\"sha256:82f7…\"}, …}"`.
   - **[measured]** `hooks/list` flipped all three to `trustStatus: "trusted"`.
   - A `codex exec` run with that override and **no** bypass flag ran the hooks (markers written, no warning).
   - This uses the SessionFlags state layer that `config_rules.rs` deliberately honours.
   - It needs the hash. The hash can be read token-free from `hooks/list`, or computed the same way.
   - The hash changes whenever the command string changes, for example when an absolute script path differs per agent.

**Without any of these [measured]:** `codex exec` with the same `-c` hooks ran **no** hooks and printed **no** warning. They were skipped silently. The TUI instead shows a "Hooks need review … 1. Review hooks / 2. Trust all and continue / 3. Continue without trusting (hooks won't run)" prompt at startup (`tui/src/startup_hooks_review.rs`, snapshot `…startup_hooks_review_prompt.snap`). Choosing "trust" writes `hooks.state` into `~/.codex/config.toml`. **[documented: source; TUI not run]**

**Supplying hooks without files [documented + measured]:**
- `-c hooks.<Event>=[{matcher=…, hooks=[{type="command", command='…'}]}]` works and is **measured**.
- `thread/start` has a `config: {[key]: JsonValue}` param (`ThreadStartParams.ts`). `config_manager.rs` L687-695 merges request overrides into the same CLI-override list (it converts JSON to TOML). So `config: {"hooks": {"SessionStart": [...]}, "bypass_hook_trust": true}` should behave like `-c`. **[inferred from source; not measured]**
- `initialize` has no hook params. The `hooks/list` method exists. There are no hook write or trust RPCs. `config/value/write`/`config/batchWrite` could persist `hooks.state` to the user config. **[documented: generated schema; not used]**

### B3. Measurement record [measured]

- **Token cost: zero.**
  - `hooks/list` needs no model.
  - The 3 `codex exec` runs used a dead provider (`-c model_provider="dead" -c model_providers.dead={base_url="http://127.0.0.1:9/v1",…}`), so no model request ever succeeded.
  - SessionStart and UserPromptSubmit hooks fire *before* the first model request.
  - Each run was killed by my timeout while Codex was retrying the network. No orphan processes remained: the 3 `codex.exe` processes left are the user's desktop app and daemon from 2026-11-23.
- `--ephemeral` was used, and no rollout appeared under `~/.codex/sessions`.
- **Backup:** `~/.codex/config.toml` was copied to `config.toml.engram-probe-bak` first. After all runs, `cmp` showed them **identical** (nothing was written to config). The backup was recycled afterwards.
- **Files under `~/.codex` with mtime after the probe started:**
  - `.sandbox/sandbox.2026-11-24.log`
  - `logs_2.sqlite(-wal)`
  - `state_5.sqlite-shm/-wal`
  - `goals_1.sqlite-shm`
  - `memories_1.sqlite-shm/-wal`
  - `models_cache.json` (rewritten 05:39:40Z, valid content)
  - 5 files under `tmp/`/`cache/`

  The Codex desktop app was running concurrently, so some of these may be its writes. No `.hook-trust/` or `hooks.json` was created.
- No credentials were read or copied.

---

## C. How comparable tools package reusable hooks

1. **Claude Code plugins.** The official marketplace `anthropics/claude-plugins-official` has `plugins/hookify/hooks/hooks.json` + `pretooluse.py`/`posttooluse.py`/`stop.py`/`userpromptsubmit.py`, with commands like `python3 "${CLAUDE_PLUGIN_ROOT}/hooks/pretooluse.py"`. `plugins/ralph-loop/hooks/hooks.json` + `stop-hook.sh` uses `bash "${CLAUDE_PLUGIN_ROOT}/hooks/stop-hook.sh"`. `claude-security`, `security-guidance` and `code-modernization` follow the same pattern. The convention is that scripts live next to `hooks.json` inside the plugin and are addressed via `${CLAUDE_PLUGIN_ROOT}`. Locally, the installed `codex@openai-codex` Claude plugin does the same (`node "${CLAUDE_PLUGIN_ROOT}/scripts/session-lifecycle-hook.mjs"`). **[documented/measured]**
2. **Codex plugins.** These mirror the Claude layout: `hooks/hooks.json` in the plugin root, or `hooks` in `.codex-plugin/plugin.json`, plus `PLUGIN_ROOT`/`CLAUDE_PLUGIN_ROOT` env. Packaging rules: https://developers.openai.com/plugins/build/plugins#bundled-mcp-servers-and-lifecycle-hooks. The difference is that plugin hooks need review, and a content-hash change re-triggers review. **[documented]**
3. **Gemini CLI extensions.** `google-gemini/gemini-cli` `docs/extensions/reference.md`: "Define hooks in a `hooks/hooks.json` file within your extension directory … hooks are not defined in the `gemini-extension.json` manifest". Variables: `${extensionPath}`, `${workspacePath}`, `${/}`. **[documented]**
4. **Cursor.** https://cursor.com/docs/agent/hooks lists `.cursor/hooks.json` (project), `~/.cursor/hooks.json` (user), enterprise MDM paths, team hooks synced to `~/.cursor/managed/team_<id>/hooks/`, and plugin hooks. Plugin hooks run from the plugin install dir, `CURSOR_PLUGIN_ROOT`. Project hooks require a trusted workspace. No per-hook approval is documented. **[documented via fetch summary]**

The convention across all four: **one folder = `hooks/hooks.json` + the scripts beside it, addressed by a root-path variable the host injects.** That maps directly onto a library component that Engram drops into the Claude plugin folder. For Codex, the same folder can be referenced by absolute path through sessionFlags `-c` hooks. **[inferred]**

---

## Sources (codex, tag `rust-v0.171.0`, https://github.com/openai/codex/tree/rust-v0.171.0/)

- `codex-rs/hooks/src/engine/discovery.rs`: layer iteration, sessionFlags source, hash, trust gate L928-951, plugin env L312-320
- `codex-rs/hooks/src/config_rules.rs`: `hooks.state` read from User + SessionFlags only
- `codex-rs/config/src/hook_config.rs`: `HooksFile`, `HookStateToml {enabled, trusted_hash, trusted_content_hash}`
- `codex-rs/protocol/src/hooks.rs`: events and handler fields (`commandWindows`)
- `codex-rs/hooks/src/engine/command_runner.rs`: shell selection
- `codex-rs/core/src/session/mod.rs`: hook shell = session shell
- `codex-rs/hooks/src/engine/plugin_trust.rs`, `plugin_fingerprint.rs`: plugin content trust, `.hook-trust/`
- `codex-rs/core-plugins/src/loader.rs`: plugin `hooks/hooks.json` default
- `codex-rs/utils/cli/src/shared_options.rs`: `--dangerously-bypass-hook-trust`
- `codex-rs/core/src/config/mod.rs`: L1057-1060, L3595-3601 (warning text)
- `codex-rs/app-server/src/config_manager.rs`: L679-695 (`bypass_hook_trust` request override, request overrides merged as CLI overrides)
- `codex-rs/exec/tests/suite/hooks.rs`: bypass test
- `codex-rs/tui/src/startup_hooks_review.rs`
- App-server schema generated locally with `codex app-server generate-ts`: `HookMetadata`, `HookSource` (`sessionFlags`), `HookTrustStatus`, `HooksListParams`, `ThreadStartParams.config`

Claude docs: https://code.claude.com/docs/en/hooks, …/plugins-reference, …/plugins/loading, …/settings, …/permissions.

## Open items (not measured)

- Claude interactive (TTY) launch with `--plugin-dir`/`--settings` hooks: is there any prompt? None is documented.
- Claude `--bare` plus `--settings`/`--plugin-dir` hooks: do they run?
- Codex `thread/start.config.hooks` and `config.bypass_hook_trust` over the app-server: the source reading is strong, but there was no live call.
- Codex plugin folder containing a junction: fingerprint failure, inferred from source.
