# Step 22 measurements, round 2: codex hook overrides (JSON mode, terminal mode, hash, merge, budget, characters)

Date: 2026-11-24. codex-cli 0.171.0 (`codex.exe` from the npm install, and once through `cmd.exe /c codex` = the npm shim). Source read at tag `rust-v0.171.0` (scratch clone). Windows 11, hooks ran under PowerShell 7 (the session shell, as in round 1).
Tags: **[measured]** = observed in this session; **[source]** = codex source at rust-v0.171.0 or this repo's code; **[inferred]** = my reading, not verified.

Method: no model tokens were spent. All runs used a dead provider (`-c model_provider="dead" -c model_providers.dead={name="dead",base_url="http://127.0.0.1:9/v1",wire_api="responses",request_max_retries=0,stream_max_retries=0}`). With retries at 0, `codex exec` exits by itself (exit 1) right after the hooks run. App-server threads used `ephemeral: true`; exec used `--ephemeral`. The hook was `node "<tmp>/hooks/mark.js" <TAG>`, which appends a line to `markers/<TAG>.txt`. Scratch folder: `%TEMP%\engram-hook-m2-1795509763033` (recycled at the end). Driver: a Node script talking JSON-RPC lines to `codex app-server` over stdio (`initialize` with `capabilities.experimentalApi=true`, then `initialized`).

## Summary table

| # | Question | Result | Evidence | Confidence |
|---|---|---|---|---|
| M1 | JSON mode: which order makes a hook from thread/start config run? | **Works with no extra step: put both the event list and `state` in one `config.hooks` object on thread/start (order b), using a self-computed hash.** Order a1 also works (hooks as `-c` on the app-server line + thread config key `"hooks.state"`). Order a2 fails: a thread config key `"hooks"` **replaces** the whole `-c hooks` table, which leaves 0 hooks. Order c works (thread 1 untrusted → read hash via `hooks/list {threadId}` → thread 2 with state), but it is not needed. **SessionStart fires on the first `turn/start`, not on `thread/start`.** Thread-config hooks show up as `source: "sessionFlags"`, sourcePath `C:\<session-flags>\config.toml`, key `…:session_start:0:0`, only via `hooks/list {threadId}`. `hooks/list {cwds}` never shows them. No state = skipped silently (no warning, no hook notification). | measured runs a1, a2, b, c and a no-state control. Notes: `hook/started` + `hook/completed` (`source: sessionFlags`). Source: `config_manager.rs` L679-695 | high (measured) |
| M2 | Terminal mode: `-c hooks` + `-c hooks.state` with no bypass | **Runs, no warning.** In a 3-handler run with trust given for only `0:0` and `1:0`, exactly those 2 ran and `0:1` did not. This was through `cmd.exe /c codex` (npm shim). Key = `C:\<session-flags>\config.toml:<event_snake>:<groupIdx>:<handlerIdx>`, 0-based. | measured: M2a markers G0H0=1, G0H1=0, G1H0=1, with "hook: SessionStart … Completed" ×2. M2b (direct exe, trust all 3) = 3 markers. `hooks/list` keys `:0:0 :0:1 :1:0` and `user_prompt_submit:0:0`. Source: `hooks/src/lib.rs` `hook_key`, `discovery.rs` L473-486 (`C:\` is hard-coded on Windows) | high |
| M3 | Self-computed hash = codex's? Cost of asking codex | **Match in 8 of 8 cases**: plain, matcher+timeout, statusMessage+async, commandWindows, plus the M1/M2 hashes that were then accepted as trusted. Canonical form below. Throwaway app-server spawn → initialize → hooks/list → stdin EOF exit takes **330 / 349 / 368 ms** (warm). The first run in the session took 723 ms to exit. | measured (`hooks/list` currentHash == local sha256). Source: `discovery.rs` `hook_hash` L983-1006, `config/src/fingerprint.rs` `version_for_toml` L54-66 | high |
| M4 | Two override sources for one event | **The last one replaces; there is no merge.** `-c hooks.SessionStart=[A] -c hooks.SessionStart=[B]` → only B. A preset `hooks={SessionStart=[P],Stop=[PS]}` then ours `hooks.SessionStart=[E]` → E + PS. Ours then a preset `hooks={Stop=[PS]}` → only PS (ours is wiped). Different events on separate lines → both kept. `-c hooks.state={K1…} -c hooks.state={K2…}` → only K2 is trusted, so all state entries must go in **one** line. | measured via `hooks/list`. Source: `config/src/overrides.rs` `apply_toml_override` (`table.insert` at the last segment; it merges tables only under `features.*`) | high |
| M5 | Budget on the cmd line | Today's terminal codex line with the built-in priming = **2542** units (upper bound). That leaves **5649** of 8191. With the 10,000-item launch note it is 2649, leaving 5542. Per hook (definition + state entry, `node "D:/Library/hooks/notify/notify.js"`, guard-safe escaped key): **first hook 293, each further hook on the same event 244.** With matcher+timeout: 324 / 275. One hook per distinct event: 302 / ~270. **Fits: 22 plain (20 with matcher/timeout) on one event, 20 (18) spread over events.** codex itself has **no cap** on `-c` count or length. | Computed from `cmdline.rs` `needed`/`units`/`arg_cost` and the test's inputs, replicated in JS (not printed by Rust). Source for codex: `utils/cli/src/config_override.rs` (`ArgAction::Append`, no limit), `overrides.rs` | high for the arithmetic. medium that the replica equals Rust's number exactly |
| M6 | cmd-unsafe characters / guard | A typical hook definition line passes the guard (no `% & ^ \| < >`, even `"` count when the path is double-quoted inside a TOML literal `'…'`). **But the trust line does not, as written by codex:** the key contains `<session-flags>`, and `text_is_cli_safe` refuses `<` `>`. Fix that works: write the key as a TOML basic string with `\u003c` / `\u003e` escapes, `"C:\\\u003csession-flags\u003e\\config.toml:session_start:0:0"`. That passes the guard and ran through `cmd.exe /c codex` (M2a). Paths or commands with `&`, `%`, `\|`, `>` are refused. Spaces, backslashes and `'` in the path are fine (`'` only if the command is written as a TOML basic string). | measured (M2a through cmd). The guard was replicated in JS from `platform/src/shell.rs` L47/L70 + `cmdline.rs` `config_args` | high |

## M1 — JSON mode details [measured]

Each scenario: a fresh `codex app-server` (dead provider), then `hooks/list {cwds}`, `thread/start {cwd, ephemeral:true, approvalPolicy:"never", sandbox:"read-only", config}`, a 3 s wait, a marker check, `hooks/list {threadId}`, `turn/start` ("hi"), a 6 s wait, a marker check, and `hooks/list {cwds}`.

| Scenario | app-server `-c` | thread/start `config` | marker after thread/start | marker after turn/start | hooks/list {threadId} |
|---|---|---|---|---|---|
| a1 | `hooks.SessionStart=[…M1a1]` | `{"hooks.state": {K: {trusted_hash: H}}}` | 0 | **1** | sessionFlags, key K, **trusted** |
| a2 | `hooks.SessionStart=[…M1a2]` | `{"hooks": {"state": {K: …}}}` | 0 | 0 | **no hooks** (the table was replaced) |
| b | none | `{"hooks": {"SessionStart": [{hooks:[{type:"command",command:…}]}], "state": {K: {trusted_hash: H}}}}` | 0 | **1** | sessionFlags, key K, **trusted** |
| c thread 1 | none | `{"hooks": {"SessionStart": [...]}}` | 0 | 0 | sessionFlags, key K, **untrusted**, hash shown |
| c thread 2 (same server) | none | `{"hooks": {"SessionStart": [...], "state": {...}}}` | 0 | **1** | **trusted** |
| control | none | hooks only, no state | 0 | 0 | untrusted (silent: no hook notification, no warning) |

K = `C:\<session-flags>\config.toml:session_start:0:0`.

- In b, the hash read from a throwaway app-server launched with the same hook as `-c` and the self-computed hash were identical: `sha256:9278f51e…48b4`.
- `hooks/list {cwds}` before and after thread/start shows only the app-server's own config (`listBefore` in b and c = no hooks; in a1, `listAfter` still says `untrusted`). The thread config only exists per thread.
- Notification excerpt (a1): `hook/started {"threadId":"01a20cb9-…","turnId":"01a20cba-…","run":{"id":"session-start:0:C:\\<session-flags>\\config.toml","eventName":"sessionStart","handlerType":"command","executionMode":"sync","scope":"thread","sourcePath":"C:\\<session-flags>\\config.toml","source":"sessionFlags",…}}`, then `hook/completed … "status":"completed" … "durationMs":2884`. (The 2.9 s is node started under PowerShell 7.)
- Source caveat **[source: app-server/src/config_manager.rs L687-695]**: thread config is a `HashMap<String, JsonValue>` that is appended after the app-server's `-c` overrides in **HashMap order**. Keys where one is a prefix of another (`"hooks"` and `"hooks.state"` in the same request) therefore apply in a nondeterministic order. Use one `"hooks"` object (as in b), or only non-overlapping dotted keys (`"hooks.SessionStart"`, `"hooks.state"`). **[inferred]** A `"hooks"` key in thread config also wipes any `hooks.*` a settings preset put on the app-server line (a2 shows the mechanism). Per-event dotted keys wipe only that event.

## M2 — terminal details [measured]

M2a, via `cmd.exe /c codex exec --ephemeral --skip-git-repo-check -C <work> -s read-only <dead> -c <hookArg> -c <stateArg> hi` (Node spawn quoting = MSVC rule):
```
hookArg  = hooks.SessionStart=[{hooks=[{type="command",command='node "<tmp>/hooks/mark.js" M2aG0H0'},{type="command",command='node "<tmp>/hooks/mark.js" M2aG0H1'}]},{matcher="startup",hooks=[{type="command",command='node "<tmp>/hooks/mark.js" M2aG1H0'}]}]
stateArg = hooks.state={"C:\\\u003csession-flags\u003e\\config.toml:session_start:0:0"={trusted_hash="sha256:a4c2a8b1…001f"},"C:\\\u003csession-flags\u003e\\config.toml:session_start:1:0"={trusted_hash="sha256:55990145…0afa"}}
output   : OpenAI Codex v0.171.0 … provider: dead … user hi / hook: SessionStart / hook: SessionStart / hook: SessionStart Completed / hook: SessionStart Completed / ERROR: Reconnecting... waiting for network
markers  : M2aG0H0=1  M2aG0H1=0  M2aG1H0=1      warning lines: none
```
M2b: direct `codex.exe exec` with the raw TOML-literal key `'C:\<session-flags>\config.toml:session_start:0:1'` and all 3 trusted. Result: 3 markers, 3 × "hook: SessionStart Completed", no warning.

## M3 — hash [measured + source]

Algorithm (`discovery.rs` `hook_hash` → `fingerprint.rs` `version_for_toml`): take the **normalized** handler, put it in `{event_name: <snake label>, matcher?: <as written, only if present>, hooks: [handler]}`, convert to TOML, then to JSON, sort all object keys, serialize compactly (`serde_json::to_vec`), and sha256. The result is `"sha256:" + lowercase hex`.
Normalization for a `command` handler (`discovery.rs` L654-727):
- `command` = `commandWindows` if it is set (on Windows), else `command`. `commandWindows` itself is dropped. **So on Windows the hash covers the Windows command.**
- `timeout` is always present: default 600; SessionEnd/Interrupt default 1, clamped to 1..3.
- `async` is always present (bool).
- `statusMessage` only if set. `additionalContextLimit` only if set and not equal to the default.

Canonical strings that matched codex (the path is shortened here; the hash was over the full string):
```
{"event_name":"session_start","hooks":[{"async":false,"command":"node \"C:/…/mark.js\" V0","timeout":600,"type":"command"}]}
   codex sha256:3f9355e3…b8ba == self
{"event_name":"session_start","hooks":[{"async":false,"command":"node \"C:/…/mark.js\" V1","timeout":30,"type":"command"}],"matcher":"startup"}
   codex sha256:88b535be…8341 == self
{"event_name":"session_start","hooks":[{"async":true,"command":"node \"C:/…/mark.js\" V2","statusMessage":"Notifying","timeout":600,"type":"command"}]}
   codex sha256:c0d5b85f…54b9 == self
(commandWindows='node … V3', command="echo unix") -> {"event_name":"session_start","hooks":[{"async":false,"command":"node \"C:/…/mark.js\" V3","timeout":600,"type":"command"}]}
   codex sha256:a4582879…981a == self
```
Timing of a throwaway app-server: `[{init 258, list 353, exit 368 ms}, {240, 334, 349}, {227, 314, 330}]` (warm cache). Self-computing costs nothing and matched in every case, so asking codex is not needed. **[inferred]**

## M4 — override merge [measured via hooks/list]

| `-c` lines (in order) | sessionFlags hooks listed |
|---|---|
| `hooks.SessionStart=[A]`, `hooks.SessionStart=[B]` | `session_start:0:0` = B only |
| `hooks={SessionStart=[P],Stop=[PS]}`, `hooks.SessionStart=[E]` | E (`session_start:0:0`), PS (`stop:0:0`) |
| `hooks.Stop=[PS]`, `hooks.SessionStart=[E]` | E, PS |
| `hooks.SessionStart=[E]`, `hooks={Stop=[PS]}` | PS only (ours wiped) |
| `hooks.SessionStart=[A]`, `hooks.Stop=[S]`, `hooks.state={K_ss}`, `hooks.state={K_stop}` | A **untrusted**, S trusted |
| same, but one `hooks.state={K_ss, K_stop}` | A trusted, S trusted |

Consequence **[inferred]**: Engram must emit, after every preset `-c` line, one `hooks.<Event>` line per event that holds all groups for that event (a preset's own groups for that event included, if they should survive), plus exactly one `hooks.state` line. Indexes in the keys are the positions in that final merged array. The `.`-split of `-c` paths (`overrides.rs`) means a single state key cannot be addressed as `hooks.state.<key>`, because the key contains `config.toml`.

## M5 — command-line budget [source, computed]

Count rule from `cmdline.rs`: per argument, UTF-16 length + 1 per `"` and `\` + 3. The wrapper is 262 + 1 + 2 + 1 + 5 + 300 (shim margin) = 571. Terminal args are counted as in the test `the_shipped_priming_fits_the_codex_terminal_command_line`: cwd `C:\Users\someone\work\project`, no preset `-c` lines, MCP URL at its 128 bound.

| Piece | Units |
|---|---|
| wrapper | 571 |
| `resume <uuid> --cd <cwd> -s workspace-write -a on-request -c <mcp at bound>` | 369 |
| priming (`-c` + `developer_instructions='…'`, 1594 chars; the text is LF-only, 21 non-ASCII chars) | 1602 |
| **total today** | **2542** (room 5649) |
| with the count note for 10,000 items | 2649 (room 5542) |

Per-hook cost. Each line also costs `-c` = 5. The state key uses the guard-safe escaped form, 161 units per entry including the hash:
- group `{hooks=[{type="command",command='node "D:/Library/hooks/notify/notify.js"'}]}` = 81 units. With `matcher="Bash"` + `timeout=30` = 109.
- Same event, N hooks (one `hooks.SessionStart` line + one `hooks.state` line): first 293, then +244 each. With matcher/timeout: 324, then +275.
- Different event per hook (one line per event + a shared state line): first 302, then +269..272. With matcher/timeout: 330, then +297..300.
- **How many fit in 5649:** 22 (same event, plain), 20 (same event, matcher+timeout), 20 (one per event, plain), 18 (one per event, matcher+timeout). Real lines are shorter where the library path is shorter. Backslash paths cost double per `\`. Preset `-c` lines, extra args and appended instruction presets come out of the same room.
- codex-side limits **[source]**: `CliConfigOverrides.raw_overrides` is a `Vec<String>` with `ArgAction::Append` and no count or length check. `parse_overrides` and `build_cli_overrides_layer` loop with no limit. The only limit is the OS: cmd 8191 here (32767 for a direct CreateProcess).

## M6 — characters [measured + source]

Guard = `text_is_cli_safe` (refuses `% & ^ | < >` and control characters) + an even number of `"` (`cmdline.rs` `config_args`).

| Line | Guard |
|---|---|
| the M2 hook line (3 handlers, double-quoted path inside `'…'`) | pass (14 `"`) |
| state line with the escaped key `"C:\\\u003csession-flags\u003e\\config.toml:…"` | pass (8 `"`). **Ran through cmd (M2a)** |
| state line with the raw key `'C:\<session-flags>\config.toml:…'` | **refused** (`<` `>`) |
| path with a space `'node "C:/Users/a b/…/notify.js"'` | pass |
| backslash path `'node "D:\Library\…\notify.js"'` | pass (costs more) |
| `'node "D:/R&D/…"'` / `'… \| tee log'` / `'node "%USERPROFILE%/x.js"'` / `'… > out.txt'` | refused |
| path with `'`, written as a basic string `command="node \"D:/Bob's/x.js\""` | pass (6 `"`) |

TOML spelling: a single-quoted literal `command='node "<abs path>"'` carries the double quotes unchanged, and codex received them intact (the hooks/list `command` field showed `node "C:/…/mark.js" TAG`). A literal cannot contain `'`. In that case use a basic string with `\"`, keeping the quote count even.

## Side effects and cleanup [measured]

- `~/.codex/config.toml` was backed up as `config.toml.engram-m2-bak` before the runs and compared at the end with `cmp`: **identical**. The backup was recycled afterwards.
- No session rollout under `~/.codex/sessions`. No `hooks.json` and no `.hook-trust/` were created.
- Files under `~/.codex` modified during the session (the Codex desktop app and its daemon from 2026-11-23 were running at the same time, so some of these are theirs):
  - `state_5.sqlite` and `-shm`
  - `goals_1.sqlite-wal` and `-shm`
  - `memories_1.sqlite-wal` and `-shm`
  - `logs_2.sqlite` and `-wal`
  - `.sandbox/sandbox.2026-11-24.log`
  - `models_cache.json`
  - `cache/codex_apps_tools/8672…json`
  - `cache/codex_apps_server_info/8672…json`
- **New** files from my two `codex exec` runs (left in place, not deleted):
  - `~/.codex/tmp/arg0-windows/codex-arg09U45xh/{.lock,apply_patch.bat,applypatch.bat}`
  - `~/.codex/tmp/arg0-windows/codex-arg0YJ10hm/{.lock,apply_patch.bat,applypatch.bat}`
- No orphan processes: the only codex/node processes left started on 2026-11-23.
- No credentials were read or copied.
