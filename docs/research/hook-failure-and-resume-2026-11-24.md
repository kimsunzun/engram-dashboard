# Step 22 measurements, round 3: Claude bad-hook containment, Codex resume hooks

Date: 2026-11-24. Claude Code 2.1.330, codex-cli 0.171.0 (`codex.exe` from the npm install, spawned directly). Windows 11.
Tags: **[measured]** = observed in this session; **[source]** = tool output or docs text quoted by the tool itself; **[inferred]** = my reading, not verified.

Method: same tricks as rounds 1 and 2. Claude: `claude -p "say ok" --model haiku --output-format stream-json --verbose --include-hook-events` from an empty cwd (`%TEMP%\engram-hook-m3-1795511292953\work`), `CLAUDECODE` unset, stdin from /dev/null. Codex: `codex app-server` with the dead provider on the server line (`-c model_provider="dead" -c model_providers.dead={name="dead",base_url="http://127.0.0.1:9/v1",wire_api="responses",request_max_retries=0,stream_max_retries=0}`), driven by a Node JSON-RPC script (`initialize` with `experimentalApi:true`). Hook script `node "<tmp>/hooks/mark.js" <TAG>` appends `{tag, hook_event_name, source, ENGRAM_M3}` (read from the hook's stdin JSON and env) to `markers/<TAG>.txt`.

## Summary table

| # | Question | Result | Evidence | Tag |
|---|---|---|---|---|
| M1a | `--settings` file with a valid SessionStart hook, `env.ENGRAM_M3="1"`, and a UserPromptSubmit group holding one bad handler (`"timeout": "30"` as a string) plus a valid sibling | **Only the bad handler is dropped.** The rest of the file loads: SessionStart hook A ran, the valid sibling in the *same group* (Asib) ran, `ENGRAM_M3=1` reached both hook processes. No stderr warning, no stream-json event, exit 0. The only trace is in the debug log: `[ERROR] Invalid setting skipped without dialog (automated session): <path>\settings-a.json: hooks.UserPromptSubmit.0.hooks.0: Invalid command hook (timeout: Invalid input: expected number, received string); entry ignored.` | 2 runs (m1a, m1a2 with `--debug-file`). Markers A=1, Asib=1, Abad absent; hook_started: SessionStart x3 (A + user + codex plugin), UserPromptSubmit x2 (Asib + user). | measured |
| M1a' | Interactive mode | The log text "skipped **without dialog (automated session)**" implies an interactive session shows a dialog for the invalid setting. | wording of the debug line | inferred |
| M1b | Same bad handler in a `--plugin-dir` plugin's `hooks/hooks.json`, plus a separate valid `--settings` file with marker-B hook | **The damage stays inside that one handler.** Plugin loaded (`engram-m3-probe@inline` in init.plugins), plugin SessionStart P ran, plugin sibling Psib ran, settings hook B ran, user + codex-plugin hooks ran. Silent on stderr/stream. Debug log: `[WARN] Plugin engram-m3-probe: hooks.UserPromptSubmit.0.hooks.0: Invalid command hook (timeout: Invalid input: expected number, received string); entry ignored`. | run m1b: markers P=1, Psib=1, B=1, Pbad absent; SessionStart x4 (P, B, user, codex plugin), UserPromptSubmit x2 (Psib, user) | measured |
| M1b' | Can the bad entry be caught before launch? | **Yes, token-free:** `claude plugin validate <dir>` prints `hooks: hooks.UserPromptSubmit.0.hooks.0: Invalid command hook (timeout: Invalid input: expected number, received string); entry ignored at runtime` as a *warning* and still ends "Validation passed with warnings" (exit 0). It lists the surviving hooks under "Provides". There is no equivalent validator for a `--settings` file. | validate output | measured (no settings validator: inferred from `--help`, not exhaustively checked) |
| M1c | Bad hook in `--settings` vs the user's own settings hook | Not run as a separate case: M1a already shows the user's `~/.claude/settings.json` SessionStart and UserPromptSubmit hooks still firing next to the damaged `--settings` file. | m1a hook counts | measured |
| M2a | Codex: persistent thread started with `config.hooks` {SessionStart + state} then a first turn | SessionStart fired on the first `turn/start` with `source:"startup"`. `hooks/list {threadId}` = sessionFlags, key `C:\<session-flags>\config.toml:session_start:0:0`, **trusted**, self-computed hash = codex's `currentHash`. A rollout file was written (thread not ephemeral). | marker R #1 `src:"startup"`; hook/started + hook/completed (`source:"sessionFlags"`) | measured |
| M2b | New app-server process, `thread/resume {threadId, config: same hooks + state}`, a turn | **SessionStart fires with `source:"resume"`, and again only on `turn/start`, not on `thread/resume`.** `hooks/list {threadId}` shows the hook **trusted**. | marker R #2 `src:"resume"`. Timeline: resume req 681 ms, idle until turn/start 5647 ms, hook/started 6382 ms, hook/completed 8407 ms. No marker during the 5 s between resume and turn/start. | measured |
| M2c | New process, `thread/resume {threadId}` **without** config, a turn | **Hooks absent.** `hooks/list {threadId}` → `hooks: []`, no warnings, no errors; no hook notification; no marker. The thread config is **not** persisted with the thread; it must be re-sent on every resume. | marker count stayed 2; rollout has 0 matches for `mark.js`, `session-flags`, `SessionStart` | measured |

## Consequences for Engram [inferred]

- Claude: a malformed library hook costs only itself; it does not knock out the rest of the assembled `--settings` or plugin. But it fails **silently** in `-p`/automated runs (debug log only) and probably with a dialog interactively. Engram should validate hook entries itself before launch (schema check matching Claude's zod rules: `timeout` number, etc.), or run `claude plugin validate` on the assembled plugin folder and surface its warnings, because the user will not see anything at runtime.
- Codex: hooks given through `thread/start.config` live only in that process's thread state. On every `thread/resume` (new process, same thread id) Engram must send the same `config.hooks` (events + state) again; otherwise the resumed thread runs with no Engram hooks and nothing says so. When sent, SessionStart runs with `source:"resume"`, so a hook that should run only on fresh starts needs `matcher:"startup"` (that changes the hash: matcher is in the hashed object).

## Raw evidence

### M1a (settings-a.json)
```
{
 "env": {
  "ENGRAM_M3": "1"
 },
 "hooks": {
  "SessionStart": [
   {
    "hooks": [
     {
      "type": "command",
      "command": "node \"<tmp>/hooks/mark.js\" A"
     }
    ]
   }
  ],
  "UserPromptSubmit": [
   {
    "hooks": [
     {
      "type": "command",
      "command": "node \"<tmp>/hooks/mark.js\" Abad",
      "timeout": "30"
     },
     {
      "type": "command",
      "command": "node \"<tmp>/hooks/mark.js\" Asib"
     }
    ]
   }
  ]
 }
}
```
- stdout non-JSON lines (both runs): only `Warning: claude.ai MCP servers blocked by enterprise policy: claude.ai Claude Docs, claude.ai Google Drive` (unrelated).
- markers (m1a2): `{"tag":"A","ev":"SessionStart","src":"startup","ENGRAM_M3":"1"}`, `{"tag":"Asib","ev":"UserPromptSubmit","src":"","ENGRAM_M3":"1"}`.
- hook_started (both runs): SessionStart:startup x3, UserPromptSubmit x2, Stop x2. All hook_response outcome success.
- debug line 104 (m1a2): `[ERROR] Invalid setting skipped without dialog (automated session): C:\Users\<u>\AppData\Local\Temp\engram-hook-m3-1795511292953\settings-a.json: hooks.UserPromptSubmit.0.hooks.0: Invalid command hook (timeout: Invalid input: expected number, received string); entry ignored.`

### M1b (plugin hooks.json = same shape with tags P / Pbad / Psib; settings-b.json = SessionStart B)
- init.plugins: `engram-m3-probe@inline, codex@openai-codex, cc-plugin-agents-md@builtin, cc-plugin-telemetry@builtin, cc-plugin-privacy@builtin, cc-plugin-plugin-authoring@builtin`
- hook_started: SessionStart:startup x4, UserPromptSubmit x2, Stop x2.
- markers: P (SessionStart startup), Psib (UserPromptSubmit), B (SessionStart startup). `ENGRAM_M3` null (not set in this run, as expected).
- debug: line 49 `[WARN] Plugin engram-m3-probe: hooks.UserPromptSubmit.0.hooks.0: Invalid command hook (…); entry ignored`, line 154 `Hooks: SessionStart:startup [node "…/mark.js" P] (plugin engram-m3-probe@inline) finished with status 0 (819ms)`, line 192 same for Psib.

Cost: 3 haiku runs (of the 4 allowed): $0.00386 + $0.00225 + $0.00036 ≈ $0.0065.

### M2 (codex)
Hook config sent (thread/start and resume-config):
```
{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"node \"<tmp>/hooks/mark.js\" R"}]}],
          "state":{"C:\\<session-flags>\\config.toml:session_start:0:0":{"trusted_hash":"sha256:8655ac2a8ec9d1a16d6f7e5a7637ccd3baf424ce895fb7c1e644d5cde3e3ef3d"}}}}
```
Other thread params: `cwd=<tmp>/w2, approvalPolicy:"never", sandbox:"read-only"` (no `ephemeral`).

| Process | Request | hooks/list {threadId} | Marker written | Hook notes |
|---|---|---|---|---|
| 1 | thread/start + config | 1 hook, sessionFlags, trusted, hash = self | `src:"startup"` (after turn/start) | hook/started + hook/completed, source sessionFlags |
| 2 | thread/resume + same config | 1 hook, sessionFlags, trusted | `src:"resume"` (after turn/start) | same |
| 3 | thread/resume, no config | `hooks: []`, warnings [], errors [] | none | none |

- thread id `01a20cd2-3909-7232-b1ed-93ae9c62a1b9`; thread/start response path `C:\Users\<u>\.codex\sessions\2026\11\24\rollout-2026-11-24T18-10-35-01a20cd2-3909-7232-b1ed-93ae9c62a1b9.jsonl`; resume responses returned the same file as a `\\?\C:\…` path, with 1 and then 2 prior turns.
- Each turn ended with `error {"message":"Reconnecting... waiting for network", willRetry:true}` notes (dead provider) and no `turn/completed` within 25 s; the driver then closed stdin and app-server exited 0. No model request succeeded.
- thread/resume emitted `deprecationNotice: "Full-history hydration is deprecated for paginated threads; use \`excludeTurns: true\`…"` and `thread/goal/cleared`.
- Rollout: 15 lines / 164,820 B after process 1, 27 lines / 172,156 B at the end. `session_meta.originator` = "probe" (the clientInfo name). 0 occurrences of the hook command or the session-flags key in it.

## Side effects and cleanup [measured]

- `~/.codex/config.toml` backed up as `config.toml.engram-m3-bak` before the runs; `cmp` at the end: **identical**. Backup recycled.
- **Left in place (session files, as instructed to report):**
  - `~/.codex/sessions/2026/11/24/rollout-2026-11-24T18-10-35-01a20cd2-3909-7232-b1ed-93ae9c62a1b9.jsonl` (172 KB). Not in `session_index.jsonl` (0 matches), but the thread is presumably also recorded in `state_5.sqlite` / `thread_history_1.sqlite` (both modified; the Codex desktop app may list it).
  - `~/.codex/tmp/arg0-windows/codex-arg06LLYTa/{.lock,apply_patch.bat,applypatch.bat}`
  - `~/.claude/projects/C--Users-<u>-AppData-Local-Temp-engram-hook-m3-1795511292953-work/` with 3 `.jsonl` (`9c7acbc8-…`, `a13dd0ac-…`, `f2cc36e1-…`), plus `~/.claude/session-env/<id>/` dirs.
- Other `~/.codex` files modified during the window (Codex desktop app + daemon from 2026-11-23 were running, so some are theirs): `.sandbox/sandbox.2026-11-24.log`, `cache/codex_apps_server_info/8672…json`, `cache/codex_apps_tools/8672…json`, `goals_1.sqlite-shm`, `logs_2.sqlite(-wal)`, `memories_1.sqlite-shm/-wal`, `models_cache.json`, `state_5.sqlite-shm/-wal`, `thread_history_1.sqlite(-shm/-wal)`.
- Recycled: the scratch folder `%TEMP%\engram-hook-m3-1795511292953`, `~/.claude/plugins/data/engram-m3-probe-inline/`, the config backup.
- No orphan processes (the 3 codex.exe and 5 node.exe left all started 2026-11-23). No credentials read or copied. No ~/.claude settings changed.
