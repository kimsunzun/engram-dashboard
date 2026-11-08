# ADR-0348: Creating and starting a new agent is one body in the agent crate — WS SpawnByCwd and bus agent.spawn --cwd are its adapters

- 상태: 확정 (2026-11-06, 근거: user delegation 2026-11-06 「이부분 너가 알아서 쭉 진행」 after main recommended "unify before step 5" and "screen-path failures get the failure mark" · code reading 2026-11-06 at `2dfd33e0` (main + two scouts))
- 관련: Amends ADR-0279 (결정 3) · ADR-0172(last failure is recorded where the failure happens) · ADR-0173(failure glyph) · ADR-0004(backend knowledge lives in the agent crate) · ADR-0003(agent crate knows no Tauri/transport) · ADR-0270(shell does not depend on the agent crate) · ADR-0122(tree delete ends the agent — why a duplicate asleep agent is costly) · ADR-0341(launch blocks and notices) · `crates/engram-dashboard-daemon/src/connection_core.rs`(`SpawnByCwd` arm · `spawn_command_by_cwd`) · `crates/engram-dashboard-agent/src/commands.rs`(`verb_spawn` · `create_and_start`) · `crates/engram-dashboard-agent/src/manager.rs`(`spawn_agent_reporting` · `activate_profile_reporting` · `spawn_fresh_settled`) · `src-tauri/src/commands/layout.rs`(`DaemonSpawner`) · `src-tauri/src/layout/apply.rs`(`spawn_into_placed`) · `docs/process/S25-preset-library/trd.md`(steps 10a · 11)

## 맥락

The daemon has two separate implementations of "create a new agent for a folder and start it":

- **WS `SpawnByCwd` arm** (`connection_core.rs`). Callers: the shell `DaemonSpawner` (Tauri `spawn_into` and shell bus verb `agent.spawnInto`) and the webview `protocolClient.spawnAgent` (front `agent.spawn`, `slot.createAgentHere`). It builds an ad-hoc profile and calls `manager.spawn_agent_reporting`.
- **Bus `agent.spawn {cwd}`** → `create_and_start` (`agent/src/commands.rs`). It registers, then calls `activate_profile` → `activate_profile_reporting` → `spawn_fresh_settled`.

Both end in `AgentManager::spawn_agent_watching_link`, but they split one level above it, and the split has drifted:

| | WS `SpawnByCwd` | bus `create_and_start` |
|---|---|---|
| Backend | caller's choice, required | fixed Claude (by design — `agent.new` is the backend-choosing door) |
| LLM backend policy | checked | not checked |
| Launch block | checked before the add; nothing left | add first, then INTERNAL + `agent_id` + reasons; asleep agent left (pre-create check deferred to TRD step 10a) |
| Caller-given name · cwd spelling normalization | no | yes |
| Roster notify | none | yes |
| Last failure (ADR-0172) | never recorded | recorded |
| Notices fingerprint | not recorded | recorded |
| Codex link verdict | **dropped** — `spawn_agent_reporting` discards the `LinkWatch` | settled in `spawn_fresh_settled` |

The last row is a defect: a codex app-server agent created through the screen or `agent.spawnInto` path is answered as started even when its handshake fails, and the failure is neither torn down nor recorded (code reading; not reproduced at runtime). The `connection_core.rs` arm already carries a comment admitting the asymmetry ("when aligning, look at both").

Separately, the shell folds the daemon reply: `send_command` turns `AgentEvent::Error` into a plain string and `DaemonSpawner` takes only `agent.id` from `Spawned`, so `launch_blocked` reasons and `notices` (wired in step 16 P4) never reach `agent.spawnInto`'s answer.

The user asked whether this structure is normal. The hop order (shell checks the target slot → daemon spawns → shell places) is sound; the duplicated, drifting creation body is not.

## 결정

1. **One creation body.** "Create a new agent for a folder and start it" is one function in the agent crate (the widened `create_and_start`), over one manager entry that runs the Fresh activation path (`activate_profile_reporting` → `spawn_fresh_settled`) for a not-yet-registered profile. Link verdict, last-failure record, notice delivery and launch-block reasons come from that one path. `spawn_command_by_cwd` (backend → launch command mapping in the daemon) moves into the agent crate with it.
2. **Every create gets the same treatment.** Backend policy check before anything is registered · launch-block check before the add (absorbs TRD step 10a's pre-create check) · cwd spelling normalization · roster notify after activation · last failure recorded when a registered agent fails to start · codex link verdict settled before answering.
3. **Entrances are adapters and keep only what is theirs.**
   - WS `SpawnByCwd`: caller-chosen backend, `MISSING_BACKEND` refusal, reply = `Spawned { agent, notices }` / `Error` (with `launch_blocked`, and `created_agent_id` when the agent was added but did not start). Notice delivery is the sender's: `SpawnByCwd.deliver_notices: true` ⇒ `Delivered` (marked seen), absent/false ⇒ `Withheld`. The shell bus `agent.spawnInto` sends true — it returns the notices to the LLM. The webview senders (`protocolClient.spawnAgent`, Tauri `spawn_into`) stay `Withheld` until a webview consumer shows them (TRD step 11). (Amended 2026-11-09 — TRD step 20 draft 3, M13 · M14.)
   - Bus `agent.spawn {cwd}`: backend stays fixed to Claude (`agent.new` remains the only bus door that chooses a backend), notices `Delivered`, answer = `AgentSpawnOk` / `CommandError`.
4. **Policy doors (replaces ADR-0279 decision 3).** The daemon checks the backend policy at two places: `agent.new` (`verb_new`) and the one creation body. The policy's home is still `LLM_BACKEND_POLICY`. ADR-0279 decisions 1, 2 and 4 stand (the wall is still before anything is registered; shell slot-spawn path, round trips and placement order unchanged; the door still does not tell callers apart).
5. **Shell stops discarding the reply.** `agent.spawnInto` answers with the daemon's `notices` on success, `args.reasons` on a launch block, and `args.agent_id` when the agent was created but did not start (the same shape as bus `agent.spawn`). Two optional wire fields carry this, both additive under `PROTOCOL_VERSION` 8: `AgentCommand::SpawnByCwd.deliver_notices` and `AgentEvent::Error.created_agent_id`. Hop order unchanged; no new verb; the shell still does not depend on the agent crate. (Amended 2026-11-09 — TRD step 20 draft 3; U3 decided (a) by the user.)
6. **Screen-path failures get the failure mark** (user decision 2026-11-06, delegated). A `SpawnByCwd` create that registers the agent but fails to start it now records last failure, so the agent list shows the ban glyph and the failure-kind tooltip, as the bus path already does. (Erratum 2026-11-06: no menu gesture uses `SpawnByCwd` today — `slot.createAgentHere` left the menu (ADR-0067) and tree create goes through `CreateProfile` + `SpawnProfile`. The callers are the webview registry commands (front `agent.spawn`, `slot.createAgentHere`), shell bus `agent.spawnInto` and Tauri `spawn_into`.) Known limit (user decision 2026-11-09): a claude that exits right after launch is still answered "started" with no mark — `cmd.exe /c claude` starts and claude has no link verdict (ADR-0201 left claude Fresh early exit out of scope); tracked as `docs/tracking.md` T-81.
7. **Completion requires a real run.** Unit tests are not enough for this change: verification includes a live daemon with real claude/codex processes and the GUI (screen create, `agent.spawnInto` from the CLI, bus `agent.spawn --cwd`), checking the answers and the agent list (user: 「실테스트 필요할듯. 지금 귀찮아서 너무 대충 넘기는게 많아」).

## 거부한 대안

- **(a) Keep both bodies; only fix the shell's reply folding.** Rejected — code: the WS body drops the codex link verdict (`spawn_agent_reporting` discards the `LinkWatch`), skips the last-failure record and roster notify, and the table above shows the two bodies already diverged on eight axes. Fixing one row at a time keeps the drift.
- **(b) Shell spawns through `agent.new` + `agent.spawn {target}` over the bus.** Rejected — still holds from ADR-0279 alternative (a): two round trips, an asleep orphan if the second fails, args JSON built by hand in the shell, and the webview `SpawnByCwd` paths would remain a second body.
- **(c) Add a `backend` argument to bus `agent.spawn` and have the shell call the bus.** Rejected — code: the shell has no bus-call producer today (no `AgentCommand::Command` sender in `src-tauri/src`) and would need a `CommandReply` fold helper and a fit under its 30 s reply cap; and it reverses the 2026-09-22 rule in `create_and_start` that `agent.new` is the one backend-choosing bus door. The webview `SpawnByCwd` paths would still need the body anyway.
- **(d) Put the unified body in the daemon crate.** Rejected — code/ADR: backend → launch-command knowledge belongs in the agent crate (ADR-0004), the activation path it must reuse lives in the agent `AgentManager`, and the bus entrance already lives in the agent crate. The daemon keeps only the WS adapter.

## 근거

- **Decision source** — main recommended both points (unify before TRD step 5; screen failures get the mark); the user delegated: 「이부분 너가 알아서 쭉 진행」 (2026-11-06). The real-run requirement is the user's (decision 7).
- **Code reading (at `2dfd33e0`)** — `spawn_agent_reporting` calls `spawn_agent_watching_link` and discards the watch (`let (outcome, _, launched)`); `spawn_fresh_settled` settles it via `link_activation_verdict`. `create_and_start` fixes Claude on purpose (comment dated 2026-09-22). `DaemonSpawner::spawn_by_cwd` maps `Spawned` to `agent.id` only; `send_command` folds `Error` to `Err(message)`.
- **Design scout (2026-11-06)** — reported that the Fresh branch of `activate_profile_reporting` already accepts an unregistered profile (`ensure_registered` runs the pre-create check, capacity check, name assignment, then inserts) and that `set_last_failure` on an unknown id is a no-op. To be confirmed in the TRD before coding.
- **Previously rejected new combined daemon verb (ADR-0279 alternative (b))** is not reopened: this decision adds no verb; it merges bodies behind the existing entrances.

## 영향 / 불변식

- ★**Invariant: one creation body.**★ A new entrance that creates-and-starts must call it; do not add a third body. The WS and bus adapters must not re-implement policy, launch-block, link-verdict or failure-record logic.
- **Policy before registration** (ADR-0279 invariant) still holds — inside the one body.
- **A refusal (policy · launch block before the add) records no last failure and leaves nothing registered.** A failure after the add leaves an asleep agent with last failure set; the WS answer says so with an `INTERNAL:` prefix so callers read it as created-but-not-started. A missing `cwd` is such a failure after the add (unchanged bus behaviour, now also WS). (Erratum 2026-11-06: an earlier wording said the prefix makes the shell keep the request id — it does not; relayed shell-owned commands free their id after any reply, `command_delivery.rs` `retains_the_id`.)
- **Notice delivery is per sender** — a sender that shows the notices asks for `Delivered` (`SpawnByCwd.deliver_notices`; today the shell bus `agent.spawnInto`); the webview senders stay `Withheld` until TRD step 11 gives them a consumer. Marking a set seen without showing it is forbidden. (Known gap, tied to the pending reply-cap decision TRD U2: a codex `agent.spawnInto` that succeeds after the 7 s bus deadline records the set while the caller gets `TIMEOUT` — no live effect until notices exist, TRD step 8.)
- **Visible changes** — screen-path failures now show the ban glyph; WS codex creates answer only after the handshake settles (normal failure ceiling = codex `HANDSHAKE_BUDGET` 10 s; the 15 s link backstop applies only to a blocked write; worst case ≈ 20 s with teardown — while a bus round trip has a 7 s deadline, the CLI a 10 s read timeout and every shell round trip a 30 s cap; measured in the TRD live run; TRD U2 re-opened 2026-11-09 because a healthy codex `initialize` was once observed at ≈ 9.7 s); bus creates appear in the tree when activation settles (the early roster frame is dropped — TRD M3).
- **TRD** — a new S25 step before step 5. It absorbs step 10a's pre-create check and step 11's "give `SpawnByCwd` a recording path" and shell `agent.spawnInto` items; the webview carrier stays with step 11.
- **Tests that pin today's split break by design** — manager source-pin tests on `spawn_agent_reporting`, `create_and_start` INTERNAL/block tests, daemon `SpawnByCwd` policy tests (move to the agent crate with the body).
- **Code anchors = `// ADR-0348`** on the unified body, the manager entry, both adapters, and the shell reply pass-through.
