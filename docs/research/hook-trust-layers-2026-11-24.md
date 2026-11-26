# Step 22 measurements, round 4: experimentalApi gate, user-layer hook state vs session flags, exec with untrusted hooks

Date: 2026-11-24. codex-cli 0.171.0 (`codex.exe` from the npm install, spawned directly). Source read at tag `rust-v0.171.0` (scratch clone). Windows 11.
Tags: **[measured]** = observed in this session; **[source]** = codex source at rust-v0.171.0; **[inferred]** = my reading, not verified.

Method: the same no-token tricks as rounds 2 and 3. Dead provider on every launch (`-c model_provider="dead" -c model_providers.dead={name="dead",base_url="http://127.0.0.1:9/v1",wire_api="responses",request_max_retries=0,stream_max_retries=0}`). Every codex process ran with `CODEX_HOME` set to a **temp folder with no `auth.json`** (`%TEMP%\engram-hook-m4-1795513992406\homeA|homeB|homeC`). The real `~/.codex` was never used as a home, and no credentials were read or copied. Hook = `node "<tmp>/hooks/mark.cjs" <TAG>`, which appends `{tag, hook_event_name, source}` to `markers/<TAG>.txt`. App-server driver = a Node JSON-RPC-over-stdio script. **`initialize` was sent with no `capabilities` object at all** (so no `experimentalApi`) in every app-server run of this round. After `turn/start`, the driver waits until every `hook/started` has a matching `hook/completed`.

## Summary table

| # | Question | Result | Evidence | Tag |
|---|---|---|---|---|
| M-A1 | Gate check: is the experimental gate active on a connection with no `experimentalApi`? | Yes. `thread/start {mockExperimentalField:"x"}` → JSON-RPC error `-32600`, message **`thread/start.mockExperimentalField requires experimentalApi capability`**. | driver response. Source: `message_processor.rs` L1046-1050, `experimental_api.rs` L30-31 (`"{reason} requires experimentalApi capability"`) | measured + source |
| M-A2 | `thread/start params.config` hooks (events + state) **without** experimentalApi | **Accepted, and the hook applies and fires.** `thread/start` returned ok. `hooks/list {threadId}` shows key `C:\<session-flags>\config.toml:session_start:0:0`, `source: sessionFlags`, `enabled: true`, **trusted** (the self-computed hash was accepted). On the first `turn/start`: `hook/started` + `hook/completed` (source sessionFlags), and marker MA has `src:"startup"`. | run A, thread `01a20cfa-…3793` (non-ephemeral, rollout in the temp home). `ThreadStartParams.config` has no `#[experimental]` attribute (`v2/thread.rs` L102-103). | measured + source |
| M-A3 | `hooks/list {threadId}` without experimentalApi | **Works.** Same response as with the flag. `HooksListParams` has no experimental fields, and `HooksList` is not marked experimental. | run A. Source: `v2/plugin.rs` L53-60, `common.rs` L983-987 | measured + source |
| M-A4 | `thread/resume {threadId, config: same hooks + state}` in a **new** app-server process without experimentalApi | **Accepted. The hook is trusted and fires** on the first `turn/start` with `src:"resume"`. `hooks/list {threadId}` after resume shows the same trusted sessionFlags entry. | run A (2nd process): marker MA = `["startup","resume"]`, one started/completed pair. Source: `ThreadResumeParams.config` is not experimental (`v2/thread.rs` L423-424) | measured + source |
| M-B1 | Terminal `codex exec`. User layer (`$CODEX_HOME/config.toml` + `hooks.json`): H1 trusted, H2 trusted + `enabled=false`, H3 (hooks.json) trusted. Engram-style `-c hooks.SessionStart=[E]` + `-c hooks.state={<session key only>}` | **H1 and H3 run, H2 stays off, E runs.** A session-flags `hooks.state` table does **not** wipe the user layer's state. | markers after B1: BE1, H1, H3 (no H2). 3 × "hook: SessionStart … Completed" | measured |
| M-B2 | Same, but the session `hooks.state` line also holds `'<H2 key>'={enabled=true}` (no hash) | **H2 is re-enabled and runs.** The session layer overrides the user layer **per key, field by field**: H2's `trusted_hash` still comes from the user layer, and `enabled=true` comes from the session layer. | markers after B2: BE2, H1, H2, H3. 4 × Completed. Source: `hooks/src/config_rules.rs` `hook_states_from_stack` L15-60 (reads only the User and SessionFlags layers, low → high; a later layer's value overwrites a field only when it is set). Unit tests `…respects_layer_precedence` / `…merges_fields_across_layers` | measured + source |
| M-B3 | App-server: `thread/start config.hooks` = {SessionStart:[E], state:{session key}} (no experimentalApi, ephemeral) | Same as B1. `hooks/list {threadId}` lists 4 hooks: H3 user trusted enabled, H1 user trusted enabled, **H2 user trusted `enabled:false`**, E sessionFlags trusted. The turn ran 3 hooks (started/completed for hooks.json, config.toml and session flags). H2 did not run. | run Bapp/BAS | measured |
| M-B4 | App-server, state also holds `{<H2 key>: {enabled:true}}` | Same as B2. `hooks/list {threadId}` shows **H2 `enabled:true`**, trusted. The turn ran 4 hooks, H2 included. | run Bapp/BAS2 | measured |
| M-C1 | `codex exec` with an untrusted session-flags hook (`-c hooks.SessionStart=[MC]`, no state, no bypass) | **Skipped silently.** No marker, no "hook:" line, no warning about hooks. stdout/stderr = the banner, `user hi`, then `ERROR: Reconnecting...` from the dead provider. | run C: marker MC absent. Full output saved and checked | measured |
| M-C2 | Interactive TUI with the same untrusted hook | **Shows a blocking startup consent picker** before the session starts. The exception is `--dangerously-bypass-hook-trust` on a launch that is not a persistent resume. The choices: "Review hooks", "Trust all and continue" (needs explicit confirmation), "Continue without trusting (hooks won't run)". **"Trust all" writes `hooks.state.<key> = {trusted_hash}` into the user's `~/.codex/config.toml`** (`config/batchWrite`, `keyPath:"hooks.state"`, `Upsert`, `filePath: None`). For a session-flags hook, that would persist a `C:\<session-flags>\config.toml:…` key into the real config. | Source: `tui/src/startup_hooks_review.rs` L70-82 (`maybe_run_startup_hooks_review`), L271-276 (choices), L150-170 (trust all → `write_hook_trusts`), L283-293 (`review_is_needed` = !bypass && count(Untrusted or Modified) > 0). `tui/src/hooks_rpc.rs` L57-62, L64-94. `tui/src/lib.rs` L2147 (`bypass_hook_trust_for_startup_review = config.bypass_hook_trust && !is_persistent_resume`), L2204 (the picker's list comes from `hooks/list` for the **cwd**), L2246-2258. The picker also runs after a directory-trust change: `tui/src/app/resume_config.rs` L320-355 | source. Not run (interactive) |
| M-C3 | Would the TUI picker see Engram's session-flags hooks at all? | Probably yes. The TUI asks its own app-server for `hooks/list {cwds:[cwd]}`, and round 2 (a1) measured that `-c` hooks on an app-server command line do appear in `hooks/list {cwds}`. If Engram also sends the matching `hooks.state` line, every hook is Trusted, `review_needed_count` = 0, and no picker appears. | round 2 a1 + source above | inferred |

## Details

### M-A (homeA: empty config, no auth) [measured]

```
initialize {clientInfo:{name:"probe-m4",version:"0"}}            -> ok   (no capabilities field)
thread/start {cwd, mockExperimentalField:"x"}                     -> error -32600 "thread/start.mockExperimentalField requires experimentalApi capability"
hooks/list {cwds:[work]}                                          -> hooks: []
thread/start {cwd, approvalPolicy:"never", sandbox:"read-only",
  config:{hooks:{SessionStart:[{hooks:[{type:"command",command:'node "<tmp>/hooks/mark.cjs" MA'}]}],
                 state:{"C:\\<session-flags>\\config.toml:session_start:0:0":{trusted_hash:"sha256:f3f051a5…5a9f"}}}}}
                                                                  -> ok, thread 01a20cfa-ea55-7961-b3f8-3b808e0b3793
hooks/list {threadId}                                             -> [{key:"C:\<session-flags>\config.toml:session_start:0:0", source:"sessionFlags", enabled:true, trustStatus:"trusted", currentHash = self-computed}]
turn/start                                                        -> hook/started + hook/completed (sessionFlags); marker MA src "startup"
--- new process, same homeA, no capabilities ---
thread/resume {threadId, cwd, approvalPolicy, sandbox, config: same} -> ok
hooks/list {threadId}                                             -> same entry, trusted
turn/start                                                        -> hook/started + hook/completed; marker MA src "resume"
```
In the first attempt of run A, the driver closed the app-server 9 s after `turn/start`. On resume, the hook had started but not yet completed (node under PowerShell takes about 3 s), so the marker was missing. The driver was changed to wait for `hook/completed`, and the full rerun above is the result. None of the fields Engram sends (`cwd`, `approvalPolicy:"never"`, `sandbox`, `ephemeral`, `config`) is gated. **[source]** `approvalPolicy` is `#[experimental(nested)]`: only its granular variant is gated (test `config_granular_approval_policy_is_marked_experimental`), and `"never"` passed.

### M-B (homeB) [measured]

User layer as written (`homeB/config.toml` with two groups, plus `homeB/hooks.json` with one group):
```
[[hooks.SessionStart]]            # group 0 -> H1   key <homeB>\config.toml:session_start:0:0
[[hooks.SessionStart]]            # group 1 -> H2   key <homeB>\config.toml:session_start:1:0
hooks.json SessionStart group 0   #         -> H3   key <homeB>\hooks.json:session_start:0:0
[hooks.state.'<H1 key>'] trusted_hash = "sha256:cc849b03…6182"
[hooks.state.'<H2 key>'] trusted_hash = "sha256:405d8c4d…cfe6"  enabled = false
[hooks.state.'<H3 key>'] trusted_hash = "sha256:1e183b93…63f1"
```
- Before the state was added, `hooks/list {cwds}` showed all three as `source:"user"`, untrusted. The self-computed hashes (round-2 canonical form) equal codex's `currentHash` for all three. After the state was added: H1 and H3 trusted and enabled, H2 trusted and **enabled:false**.
- A user-layer hook's key = the real file path + `:<event_snake>:<group>:<handler>`. `hooks.json` and `config.toml` are indexed separately, each from 0.
- Side warning (from my setup, not something Engram does): having both files in one layer gives `warning: loading hooks from both …\hooks.json and …\config.toml; prefer a single representation for this layer` (exec prints it, and `hooks/list` returns it in `warnings`).

exec lines. These spawned `codex.exe` directly, so the raw literal key works. Through `cmd`, use the escaped form for `<` and `>` from round 2 (M6).
```
B1: -c hooks.SessionStart=[{hooks=[{type="command",command='node "<tmp>/hooks/mark.cjs" BE1'}]}]
    -c hooks.state={'C:\<session-flags>\config.toml:session_start:0:0'={trusted_hash="sha256:…"}}
    -> hook: SessionStart x3, Completed x3; markers +BE1 +H1 +H3 (H2 none)
B2: -c hooks.SessionStart=[{hooks=[{type="command",command='… BE2'}]}]
    -c hooks.state={'C:\<session-flags>\config.toml:session_start:0:0'={trusted_hash="sha256:…"},'<homeB>\config.toml:session_start:1:0'={enabled=true}}
    -> hook: SessionStart x4, Completed x4; markers +BE2 +H1 +H2 +H3
```
App-server (a fresh process per thread, `ephemeral:true`, no capabilities):
- BAS: 3 hooks ran (H3, H1, session). `hooks/list {threadId}` showed H2 `enabled:false`.
- BAS2 (state plus `{<H2 key>:{enabled:true}}`): 4 hooks ran, and `hooks/list` showed H2 `enabled:true`.

Consequences **[inferred]**:
- Engram's single `hooks.state` line, holding only Engram's keys, is safe for the user's own hooks. Their trust and disable flags in `~/.codex/config.toml` stay as they are, because state is merged key by key across layers rather than replaced as a whole table. Round 2's M4 "last one replaces" rule applies only **within** the session-flags layer.
- The flip side: whatever Engram writes into the session `hooks.state` under a **user** key beats the user's choice for that session. `enabled=true` turns back on a hook the user turned off, and a `trusted_hash` would trust a hook the user never trusted. Engram should only ever emit `C:\<session-flags>\…` keys.
- State is read only from the `User` and `SessionFlags` layers (`config_rules.rs` L24-28). A project `.codex/config.toml` `hooks.state` is ignored. Managed hooks ignore state (`discovery.rs` L1030-1037). **[source]**

### M-C [measured + source]

- exec with `homeC` (empty), `-c hooks.SessionStart=[MC]`, no state. Output lines: the PATH-alias warning (temp CODEX_HOME), `Reading additional input from stdin...`, the banner (`provider: dead`, `approval: never`, `sandbox: read-only`), `user` / `hi`, `ERROR: Reconnecting... waiting for network` ×4. No `hook:` line, no trust warning, no marker. This matches round 2's finding for app-server: no state = skipped silently.
- TUI (source only), details in M-C2. An interactive `codex` with an untrusted hook stops at a consent picker. Its "Trust all" saves the trust in the user's config file. For session-flags keys, that leaves `C:\<session-flags>\config.toml:…` entries in `~/.codex/config.toml`, and they would trust any later launch that uses the same hook text. So Engram's terminal launch should always send the state line (no picker, nothing written) and not rely on the picker. **[inferred]**

### Observations off the main questions [measured]

- With a temp `CODEX_HOME` (no auth), `codex exec` did **not** exit by itself after the dead-provider error. In round 2, with the real home, it exited with code 1. All three exec runs here were killed by the driver's 60 s timeout, after their hooks had completed. I did not look into the cause (missing auth, or a different retry path). It does not affect the hook results.
- A temp `CODEX_HOME` under `%TEMP%` prints `WARNING: proceeding, even though we could not create PATH aliases: Refusing to create helper binaries under temporary dir …`.

## Side effects and cleanup [measured]

- `~/.codex/config.toml` was backed up as `config.toml.engram-m4-bak` before the runs and never edited. `cmp` at the end: **IDENTICAL**. The backup was recycled.
- No codex process of mine used `~/.codex`: every spawn had `CODEX_HOME` set to a temp home. M-A's non-ephemeral thread wrote its rollout and sqlite files into `homeA`, which was recycled with the scratch folder.
- Recycled: `%TEMP%\engram-hook-m4-1795513992406` (homes, markers, outputs) and the driver folder `%TEMP%\engram-hook-m4-drv-1795513992406`.
- No orphan processes. The codex.exe and node.exe processes still running all started on 2026-11-23, before this round.
