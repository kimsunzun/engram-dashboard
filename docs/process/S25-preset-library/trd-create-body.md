# TRD — One creation body for "create a new agent for a folder and start it" (S25 step 20)

| Item | Value |
|---|---|
| **Status** | **Draft 1 — 2026-11-06 — canonical ADR-0348** (`docs/decisions/0348-creating-and-starting-a-new-agent-is-one-body-in-the-agent-crate-ws-spawnbycwd-and-bus-agentspawn-cwd-are-its-adapters.md`). ADR-0348 is the what/why (it stands in for a PRD). Not yet reviewed. |
| **Line pointers** | HEAD `28ecd79a` (paths and line numbers below were read at this commit) |
| **Parent TRD** | [`trd.md`](trd.md) (S25 preset library) — this file is one new step of it (§1) |

## 1. Scope and relation to the S25 TRD

**What this step does.** It folds the two daemon bodies that create a new agent for a folder and start it — WS `SpawnByCwd` (`crates/engram-dashboard-daemon/src/connection_core.rs:1577-1650`) and bus `agent.spawn {cwd}` (`create_and_start`, `crates/engram-dashboard-agent/src/commands.rs:1211-1290`) — into one body in the agent crate. The two entrances become adapters. It also stops the shell from folding the `SpawnByCwd` reply, so `agent.spawnInto` answers with `notices` and `args.reasons`. No new verb, no change to the shell ↔ daemon hop order (ADR-0279 decision 2 stands).

**Place in the S25 order.** New step **20**, run after step 16 (landed) and before §5-2 step 5 (user decision 2026-11-06, ADR-0348). Commit titles `S25: refactor(<scope>): … (step 20 P<n>)`.

**Absorbed from step 10a** (`trd.md` §5-2 10a and step 16's "Deferred" line):
- The pre-create launch check of `agent.spawn --cwd` (§1-5 (가) 1 ② — the `TODO(TRD S25 §5-2 10a)` at `commands.rs:1208`): a block before the add answers `CONFLICT` + `args.reasons` with nothing created.
- Rewriting the `agent.spawn` summary ("registered before its launch is checked", `commands.rs:268-269`).

**Stays in step 10a:** every library · preset · agent-preset command declaration, `engram help library`, and `agent.new {presets}` validation before `create_agent` (a different verb — it registers without starting).

**Absorbed from the step 11 handover** (`trd.md:905`):
- (a) partly — "give `SpawnByCwd` a recording path": the body takes a `NoticeDelivery` argument, so `SpawnByCwd` can record once a WS consumer shows notices.
- (b) partly — widening `daemon_client/protocol_state.rs` `classify_reply` so `CommandFailure::Daemon` keeps `launch_blocked` (shared with (c)).
- (c) fully — shell bus `agent.spawnInto` passes `notices` and the structured block reasons.

**Stays in step 11:** flipping WS activations to `NoticeDelivery::Delivered` (WS `SpawnByCwd` stays `Withheld` here — ADR-0348 decision 3); moving `Spawn` from `Ack` to `Spawned`; the Tauri carrier `commands/agent.rs` `forward_to_daemon` and the webview reading the widened replies; the Tauri `spawn_into` command's return type (stays `Result<String, String>` here).

**Not in scope:** request-id retention for shell-owned (delegated) commands (§2 premise 3); a caller axis on the spawn packet (ADR-0279 decision 4); a `backend` field on bus `agent.spawn` (ADR-0348 alternative (c)).

## 2. Premises checked against code

Each premise the design leans on, with the verdict at `28ecd79a`.

| # | Premise | Verdict | Evidence |
|---|---|---|---|
| 1 | The Fresh branch of `activate_profile_reporting` accepts a profile that is not in the roster | **True** | `manager.rs:1916-1990`: the in-flight check and the live-session lookup do not need a roster entry; Fresh goes to `spawn_fresh_settled` (`:2177`) → `spawn_agent_watching_link` (`:1427`) → `ensure_registered` (`:1324-1346`), which runs `check_before_create`, the capacity check and name assignment, then inserts (`inserted = true` skips `register_for_spawn`). The premise noted in the `connection_core.rs:1609-1614` comment ("`activate_profile` needs a stored entry") was already lifted by step 4 (§1-5 (가) 0). |
| 2 | `set_last_failure` on an unknown id is a no-op | **True** | `profile.rs:841-856` — the `_ => false` arm. So a pre-add refusal that reaches `note_spawn_result` (`manager.rs:2133`) records nothing. |
| 3 | `agent.spawnInto` has a duplicate-on-retry hole that an `INTERNAL` answer would close by holding the request id | **False — no hole that the code fixes** | `retains_the_id` (`command_delivery.rs:1444`) is called only on the daemon-local branch (`:1895`, `:1920`). A delegated round trip (a shell-owned command such as `agent.spawnInto`) always frees its seat after the reply frame (`release`, `:888-897`). So for `agent.spawnInto`, `INTERNAL` vs `CONFLICT` changes the meaning of the answer, not id retention; a same-id retry after either re-runs. ADR-0348 「영향」's "so the shell keeps the request id (ADR-0122)" is inaccurate — main to correct the ADR. The text of the created-asleep answer is what stops a duplicate ("start it again by that name instead of creating another"). |
| 4 | A new `ok` field needs a catalog version bump | **Fold into the open generation** | The rule is "a declaration change bumps `CATALOG_VERSION`" (`command/src/macros.rs:31`). Shell `catalog_version: 22` (`layout/commands.rs:153`) and agent `catalog_version: 9` (`commands.rs:58`) were both set by S25 step 1 (`d2c80862`, 2026-11-01), which no tag contains (latest tag `v0.4.0`). Precedent: agent v9 note "The same unreleased generation: the `agent.spawn` answer gains `notices` (TRD S25 step 16)". So add a note line to each, no bump. Bindings regenerate. |
| 5 | Link backstop vs reply caps | **Numbers** | `LINK_RESOLUTION_BACKSTOP` = **15 s** (`manager.rs:81`). Every bus round trip — the local `agent.spawn` and the relayed `agent.spawnInto` alike — has a seat deadline `DEFAULT_DEADLINE` = **7 s** (`command_delivery.rs:462`, applied to every seat at `:664`), sized under the CLI's **10 s** read timeout (`CLI_CONTROL_READ_TIMEOUT_SECS`, `agent/src/types.rs:521`). The webview's direct WS commands (`protocolClient.spawnAgent`) go through `forward_daemon_command` with a **30 s** cap (`src-tauri/src/commands/agent.rs:277`). `DaemonSpawner` itself awaits `send_command` with **no** cap (`commands/layout.rs:107-131`). So after this step a codex app-server create through `agent.spawnInto` that takes over ~7 s to settle answers `TIMEOUT` to the LLM while the agent is still created and placed. Bus `agent.spawn {target}` on a codex app-server agent already has this exposure today. Measured in R8. |
| 6 | Today WS drops the codex link verdict | **True** | `spawn_agent_reporting` (`manager.rs:2043-2051`) discards the `LinkWatch`. `spawn_command_by_cwd` (`connection_core.rs:677`) uses `NEW_AGENT_OUTPUT_FORMAT` = `StreamJson` (`commands.rs:693`), and codex `StreamJson` is app-server, which `declares_link` (`backend/codex/mod.rs:707`). The `spawn_agent` doc claim "the codex that gets there today has a terminal output format" (`manager.rs` ~1390) is out of date. No test fixture plays a refusing app-server (`manager.rs:4810-4813`); R7 is the first live measure. |
| 7 | A screen gesture creates through `SpawnByCwd` | **False — registry commands only** | No menu reaches `SpawnByCwd`: `slot.createAgentHere` left the slot menu (ADR-0067, `src/commands/slotContentCommands.ts:94`); tree create is `CreateProfile` + `SpawnProfile` (`src/commands/agentCommands.ts:20-37`), which this step does not touch. The WS entrances are the webview registry commands `agent.spawn` (`agentCommands.ts:69-92`), `slot.createAgentHere`, `agent.spawnInto` (`tabCommands.ts:338` → Tauri `spawn_into` → `DaemonSpawner`), and the bus `agent.spawnInto`. In ADR-0348 decision 6, "screen path" means these. |
| 8 | A missing folder is refused before the add | **False for `cwd`** | Nothing checks that `cwd` exists before `ensure_registered` inserts: `register` only fixes the spelling (`commands.rs:1356`), Noop `check` reads no paths, and the stdio spawn sets `current_dir` (`transport/spawn.rs:195`). A missing `cwd` is a failure **after** the add (`SpawnFailed`, asleep agent marked), for both entrances, today and after this step. The "missing folder" in ADR-0348's refusal list fits the step-8 block ① (a preset folder entry), not `cwd`. TRD keeps today's behaviour (M4). |
| 9 | A launch block is reachable today with real inputs | **False** | The daemon plugs no launch-kit source (`daemon/src/lib.rs:271` `TODO(TRD S25 §5-2 8)`), so the manager uses `NoopLaunchKits` (`launch.rs:507-523`), which never blocks. Policy refusals are not reachable either: `LLM_BACKEND_POLICY` opens both claude and codex. See U1. |
| 10 | Roster notify on WS `SpawnByCwd` today | **None** | No `broadcast_profile_list` in the arm (the arm at `connection_core.rs:1577-1650` vs the `SpawnProfile` arm at `:1392`). |

## 3. Module boundaries and signatures

Layers: agent manager (owns activation) → agent `commands` (the one body + bus adapter) → daemon `connection_core` (WS adapter) → shell (passes the reply through). The agent crate stays free of Tauri and transport (ADR-0003); the shell still does not depend on the agent crate (ADR-0270). Code anchor `// ADR-0348` on each item below.

### 3-1. Manager entry — `crates/engram-dashboard-agent/src/manager.rs`

```rust
/// A create that did not end in a started agent. `created` = this call added the agent to the roster
/// (it stays there asleep; `last_failure` is set when the failure reached a launch).
pub struct NotStarted { pub error: PtyError, pub created: bool }

/// Create-and-start for a profile whose id is not in the roster: the Fresh activation path
/// (`activate_profile_reporting` → `spawn_fresh_settled`) — pre-create check, add, prelaunch, link verdict,
/// last-failure record, notice delivery all come from that path.
pub fn create_and_activate(
    &self,
    profile: &AgentProfile,
    delivery: NoticeDelivery,
) -> Result<Activated, NotStarted>
```

Body:
1. If `self.profiles.get(profile.id)` is `Some`, return `NotStarted { error: PtyError::Unsupported("…a new agent needs a new id — wake an existing one instead"), created: false }`. This guards misuse; callers always pass a fresh `AgentProfile::new` uuid.
2. Call `self.activate_profile_reporting(profile, SpawnMode::Fresh, delivery)`.
3. On `Err(error)`, `created = self.profiles.get(profile.id).is_some()` (§5).

Placement: put it where no existing `squashed_production_body` window moves (check the split markers in `manager.rs` tests, e.g. `:4778`, `:4838`, `:4872`, `:4909`, `:7129`). Extend `both_fresh_entrances_settle_the_link_before_reporting_success` (`:4817`) with a pin that `create_and_activate` calls `activate_profile_reporting(` with `SpawnMode::Fresh`.

**Removed in P2:** `spawn_agent_reporting` (`:2043`) loses its only production consumer, so it goes, together with its test in `tests/activation.rs:1462` and the doc sentences that name it (`manager.rs` ~1390, `deliver_notices` doc).

### 3-2. The one body — `crates/engram-dashboard-agent/src/commands.rs`

```rust
/// What the one creation body needs from its entrance.
pub struct NewAgent {
    pub cwd: String,
    pub name: Option<String>,
    pub backend: AgentBackend,
    /// `None` = create · `Some(reason)` = refuse. Production = `llm_creation_refusal`.
    pub policy: fn(&str) -> Option<&'static str>,
    pub delivery: NoticeDelivery,
}

pub enum CreateRefused {
    /// The backend policy refused; nothing was registered and no last failure was written.
    Policy { reason: &'static str },
    /// Refused or failed before the add: store refusal, roster ceiling, name space, launch block or race
    /// before the add. Nothing was registered.
    NotCreated(PtyError),
    /// The agent was registered and is asleep; `error` is why it did not start.
    CreatedAsleep { id: AgentId, name: String, error: PtyError },
}

/// ★The one body (ADR-0348)★ — every entrance that creates-and-starts calls this.
pub fn create_and_start(
    host: &dyn AgentCommandHost,
    notify: &dyn RosterChanged,
    req: NewAgent,
) -> Result<Activated, CreateRefused>
```

Body, in order:
1. `(req.policy)(&backend_word(&req.backend))` → `Some(reason)` ⇒ `Err(Policy)`. The host is not called.
2. `cwd = normalize_spelling(&req.cwd)`.
3. `AgentProfile::new(cwd, backend_command(req.backend, NEW_AGENT_OUTPUT_FORMAT), PathBuf::from(cwd), vec![], false)` with `display_name = req.name`. This is the one home of the default output format (replaces `spawn_command_by_cwd`).
4. `host.create_and_activate(&profile, req.delivery)`.
5. If `Ok`, or `Err` with `created == true`, call `notify.roster_changed()` once. No notify when nothing was registered.
6. On `Err` with `created`, read the registered name via `host.agent_snapshot(id)` (`canonical_name_when_live()`; fall back to the id string) ⇒ `CreatedAsleep`. Otherwise ⇒ `NotCreated(error)`.

Shared wording, so both entrances say the same thing (moved out of the closure in today's `create_and_start`):

```rust
/// "agent '<name>' (<id>) was created but did not start: <cause> — it is registered and asleep, so <next>
/// instead of creating another"
pub fn created_but_not_started(name: &str, id: AgentId, error: &PtyError) -> String
```

- `cause`: `Store` → `store_error_cause`; `LaunchBlocked` → `launch_blocked_reason`; `LaunchRaced` → `LAUNCH_RACED_CAUSE`; other → `to_string()`.
- `next`: for a block, "fix the listed items first, then start it by that name"; otherwise `AGAIN` ("start it again by that name (in a moment)").
- The wording is today's (`commands.rs:1253-1287`).

### 3-3. `AgentCommandHost` addition — `commands.rs:495-530` (trait) · `:614` (manager impl) · `:2134` (`FakeHost`) · `:3216` (`RefuseOnStart`)

```rust
/// Create-and-start a profile not yet in the roster — [`AgentManager::create_and_activate`].
fn create_and_activate(&self, profile: &AgentProfile, delivery: NoticeDelivery)
    -> Result<Activated, NotStarted>;
```

It returns `Activated` (full `AgentInfo` + notices) because the WS reply needs `agent_info_to_wire(&AgentInfo)`. The bus maps it to `StartedAgent`. The existing `create_agent` (used by `verb_new`) and `activate_profile` (used by `wake_existing`) stay. The test fakes gain the method; their rules stay identical to the real ones (ADR-0155 note at the trait).

### 3-4. Bus adapter — `verb_spawn` cwd arm, `commands.rs:1138`

```rust
(None, Some(cwd)) => create_and_start(host, notify, NewAgent {
    cwd: cwd.to_string(), name: name.map(str::to_string),
    backend: AgentBackend::Claude,                  // fixed on purpose (2026-09-22 rule, ADR-0348 decision 3)
    policy: llm_creation_refusal, delivery: NoticeDelivery::Delivered,
}).map(|a| started_payload(a.into(), true)).map_err(spawn_create_refusal)
```

`spawn_create_refusal: CreateRefused -> CommandError` maps per §4-2. The register error mapping (`commands.rs:1366-1391`) moves into a shared `fn create_error(e: PtyError) -> CommandError` used by both `register` (for `agent.new`) and the `NotCreated` arm. The old private `create_and_start` and the second `roster_changed()` go away.

### 3-5. WS adapter — `SpawnByCwd` arm, `connection_core.rs:1577-1650`

1. `let Some(kind) = backend else { reply(MISSING_BACKEND) }` — unchanged.
2. Exhaustive `match kind { WireBackendKind::Claude => AgentBackend::Claude, WireBackendKind::Codex => AgentBackend::Codex }` — no wildcard, so a new wire word stops compilation here (the reason `spawn_command_by_cwd` kept its `match`).
3. `tokio::task::spawn_blocking(move || create_and_start(&*mgr, &roster, NewAgent { cwd, name: None, backend, policy: self.llm_policy, delivery: NoticeDelivery::Withheld }))`.
   - `roster` is the existing `RosterFanout` (`connection_core.rs:2455`), cloned into the closure, with a new `impl RosterChanged for RosterFanout` next to its `RosterBroadcast` impl (profile list then agent list — the same two frames as the bus notify).
4. Map the result per §4-1.
5. Delete `spawn_command_by_cwd` and the asymmetry comment.
6. Keep `by_cwd_refusal`'s wording (it gains a `reason` parameter in place of the policy call) and `MISSING_BACKEND`.
7. A `JoinError` maps to `activation_error(… PtyError::SpawnFailed("spawn task failed: …"))` as today.

### 3-6. Shell reply pass-through — `src-tauri`

- `daemon_client/protocol_state.rs:40` — `CommandFailure::Daemon { message: String, launch_blocked: Option<serde_json::Value> }`.
  - `classify_reply` (`:61-65`) copies the field.
  - `Display` is unchanged, so `send_command`'s string stays byte-identical.
  - The 8 existing `Daemon { message }` matches (`commands/view_bus.rs`, `daemon_client/{connection,mod,refusal,tests}.rs`) gain `..`.
- `layout/apply.rs:247` — `AgentSpawner::spawn_by_cwd` → `Result<SpawnedAgent, SpawnRefusal>`.
  - `pub struct SpawnedAgent { pub id: String, pub notices: Vec<serde_json::Value> }`.
  - `pub struct SpawnRefusal { pub message: String, pub launch_blocked: Option<serde_json::Value> }`.
  - `spawn_into` / `spawn_into_placed` / `spawn_into_at` and `SpawnAtError::Failed` carry them.
  - The placement outcome's `out` becomes `SpawnedAgent`.
- `commands/layout.rs:103-131` — `DaemonSpawner` calls `send_command_with_origin`.
  - `Ok(Spawned { agent, notices })` ⇒ `SpawnedAgent { id: agent.id, notices: notices.unwrap_or_default() }`.
  - `Err(Daemon { message, launch_blocked })` ⇒ `SpawnRefusal { message, launch_blocked }`.
  - `Err(Local(m))` ⇒ `SpawnRefusal { message: m, launch_blocked: None }`.
  - Tauri `spawn_into` (`:567`) keeps `Result<String, String>` (id / message).
- `layout/commands.rs` — `agent.spawnInto`:
  - `AgentSpawnIntoOk` gains `notices: Vec<LaunchNoticeRow>`.
  - `LaunchNoticeRow { code: String, args: Json, line: Option<u32>, text: String }` is declared in the shell block, the same shape as the agent block's (`commands.rs:224-233`).
  - `spawn_not_applied` (`:2920`) takes `SpawnRefusal`: the code-prefix mapping is unchanged, plus `.with_args(launch_blocked)` when present.
  - Summary text updated (§4-3).
  - Catalog stays 22, with a note line.
- `prompts/engram-help.md` — line 69 (bus `--cwd` pre-create block = `CONFLICT`, nothing created) and line 171 (`agent.spawnInto` answer carries `notices`; a block carries `args.reasons`; a failure after the add is `INTERNAL` and the agent stays asleep).

## 4. Answer shapes per entrance

### 4-1. WS `SpawnByCwd` (`AgentEvent`)

| Outcome | Reply |
|---|---|
| started | `Spawned { request_id, agent: agent_info_to_wire(&info), notices: notices_to_wire(&notices) }` — `notices` absent when empty (byte-identical to step 16); not marked seen (`Withheld`) |
| `backend` absent | `Error { message: MISSING_BACKEND, launch_blocked: None }` (unchanged) |
| `Policy { reason }` | `Error { message: by_cwd_refusal text, launch_blocked: None }` (unchanged wording) |
| `NotCreated(e)` | `activation_error(conn_id, "SpawnByCwd", "the agent was not created or started", request_id, e)` (unchanged): store ⇒ `"CONFLICT: …"` / `"INTERNAL: …"` prefix (`store_error_text`); launch block ⇒ `launch_blocked: {"reasons":[…]}`; other ⇒ plain text |
| `CreatedAsleep { id, name, error }` | `Error { message: "INTERNAL: " + created_but_not_started(name, id, &error), launch_blocked: block ? Some(to_args()) : None }` — **new**: the asleep agent is named; on the bus `agent.spawnInto` the prefix turns into code `INTERNAL` (`spawn_not_applied` — `INTERNAL` is a common error, `command/src/spec.rs:60`); the webview registry commands see the text raw (front unchanged until step 11) |

### 4-2. Bus `agent.spawn {cwd}` (`AgentSpawnOk` / `CommandError`)

| Outcome | Answer |
|---|---|
| started | `{agent_id, name, state, created: true, notices: [...]}` — notices delivered and recorded (unchanged) |
| `Policy { reason }` | `INVALID_ARGUMENT`, `verb_new` policy wording (unreachable today — both backends open) |
| `NotCreated(LaunchBlocked)` | **new**: `CONFLICT`, `"{LAUNCH_REFUSAL} for a new agent in '<cwd>': <reasons>; nothing was created — fix the listed items, then try again"`, `args = blocked.to_args()` (`{"reasons":[…]}`) — the request id is released (`retains_the_id`), so the fixed retry runs |
| `NotCreated(LaunchRaced)` | **new**: `CONFLICT` with `RetryMode::AfterCondition`, "…nothing was created — try again" |
| `NotCreated(other)` | today's `register` mapping: `RosterFull` / `Unsupported` ⇒ `CONFLICT`; `CwdDenied` ⇒ `INVALID_ARGUMENT`; `Store` ⇒ `store_command_error` (refusal `CONFLICT`, write failure `INTERNAL`); else `INTERNAL` |
| `CreatedAsleep { id, name, error }` | `INTERNAL`, `created_but_not_started(…)`, `args = {agent_id}` (+ `reasons` for a block) — unchanged; the id stays held (daemon-local command) |

Summary of `agent.spawn` (`commands.rs:264-269`) becomes: "With `cwd`, a launch blocked before the agent is added answers `CONFLICT` with `args.reasons` and creates nothing; a start failure after the add is `INTERNAL` with `args.agent_id` (plus `args.reasons` when blocked) — start that agent by target, do not create another." Bindings (`crates/engram-dashboard-agent/bindings/`) regenerate; catalog stays 9 with a note line.

### 4-3. Shell bus `agent.spawnInto`

| Outcome | Answer |
|---|---|
| started | `{agent_id, layout, notices: [...]}` — `notices` = the daemon's rows `{code, args, line, text}` (`[]` when none; a row that fails to decode is dropped with a `warn` naming it) |
| daemon `Error` with `launch_blocked` | `CONFLICT` (prefix-less block text ⇒ `not_applied`), message = daemon text, `args = launch_blocked` (`{"reasons":[…]}`) |
| daemon `Error` `"INTERNAL: …"` (created-asleep or store write failure) | `INTERNAL`, prefix stripped (`spawn_not_applied`), plus `args.reasons` for a block after the add |
| other daemon / local refusal | unchanged (`spawn_not_applied` / `spawn_at_error`) |

Summary addition: "Answer adds `notices`. A blocked launch creates nothing and is `CONFLICT` with `args.reasons`. If the agent was created but did not start, the code is `INTERNAL` and the message names the asleep agent — start it by name, do not spawn again." Shell bindings (`src-tauri/bindings/`) regenerate; catalog stays 22 with a note line.

## 5. Concurrency

- **Lock order** is unchanged: `create_and_activate` holds no lock of its own. `ensure_registered` already runs `check_before_create` before taking the name-allocation gate (asset-library lock → manager lock, `manager.rs:1330-1335`). The created probe (`profiles.get`) is one short registry read after activation has returned, with no lock held. `notify.roster_changed()` runs after the manager call.
- **Blocking**: both adapters keep the body off async workers — WS in `spawn_blocking` (as today, `connection_core.rs:1621`; pin test `the_by_cwd_spawn_hands_the_blocking_call_to_the_pool` (`:2775`) is rewritten to look for `create_and_start(` inside the `spawn_blocking` window); bus in `blocking_handler` (`make_table`). The body can now wait up to the 15 s link backstop on WS (it never did before).
- **Created probe after `Err`**: a delete landing between the add and the probe would report `NotCreated` for an agent that was created and then deleted. The end state is still what the answer says (nothing left). The id is a fresh uuid that no caller holds before the answer, so the window needs someone acting on a roster listing. Accepted.
- **Existing-id refusal**: `create_and_activate` refuses an id already in the roster before activating. Without it, `ensure_registered` would return `inserted = false` and the call would silently become a wake with a merge of the caller's copy.
- **Reservation**: a concurrent `agent.setWorkspace` cannot take the new id's `SpawnReservation` before the add, because no caller knows the id yet. The §1-5 (가) 1 ② race named in `trd.md:252` (`setWorkspace` winning between `create_agent` and `activate_profile`) disappears for this body, since the add happens inside the reservation.
- **Roster frames**: the bus loses its pre-activation `roster_changed` (M3). WS gains one after activation. For codex app-server the row appears when the handshake settles (≤ 15 s); for claude (no link axis) right after the process starts.

## 6. Test plan

QA binding rows (`.claude/skill-bindings/qa.md` 「돌릴 것 표」): **Rust source** ⇒ full regression; **agent lifecycle (spawn)** ⇒ full regression + live run **on · GUI**. Shell IPC shape unchanged for the webview (Tauri `spawn_into` keeps its type); the shell bus answer shape changes ⇒ covered by the same live run. claude/codex lanes: no touched path is in a lane list (`backend/**`, `daemon/src/control/**` untouched) ⇒ lanes only by the CLI-version rule.

### 6-1. Automated, per piece

**P1 — agent crate** (`cargo test -p engram-dashboard-agent -- --test-threads=4`)
- Change (`commands.rs` tests):
  - `spawn_by_cwd_creates_then_starts` (`:2563`) — one `create_and_activate` call with `Fresh` / `Delivered`, normalized `cwd`, `display_name = name`, command `backend_command(Claude, NEW_AGENT_OUTPUT_FORMAT)`.
  - `a_created_agent_that_fails_to_start_is_not_rolled_back` (`:2584`).
  - `a_start_failure_after_the_create_stays_internal_and_names_the_created_agent` (`:2674`).
  - `the_spawn_answer_carries_the_launch_notices_as_rows` (`:2731`).
  - `a_create_and_start_whose_create_the_store_refuses_starts_nothing_and_announces_nothing` (`:3184`) — now `NotCreated(Store)`, no notify.
  - `a_created_agent_whose_start_the_store_refuses_stays_internal_with_the_recovery_hint` (`:3213`).
  - `FakeHost` (`:2134`) and `RefuseOnStart` (`:3216`) gain `create_and_activate`.
- New (`commands.rs`):
  - a block before the add ⇒ `CONFLICT` + `args.reasons`, `LAUNCH_REFUSAL` prefix, no notify, host saw no add;
  - a race before the add ⇒ `CONFLICT` retry;
  - policy refusal ⇒ host never called (injected policy closing `claude`);
  - one notify on success and on created-asleep, none on `NotCreated`;
  - the body builds the same default mode for both backends (replaces the daemon's `by_cwd_fills_one_shared_default_mode_for_every_backend`, `connection_core.rs:5416`);
  - `created_but_not_started` wording per error kind.
- New (`tests/activation.rs`, real child processes, existing `noticing_manager` / `long_lived_profile` fixtures):
  - `create_and_activate` of an unregistered profile registers, starts, returns and records delivered notices;
  - a block before the add leaves no roster row and no last failure;
  - a block after the add (a kit that passes `check_before_create` and blocks `prelaunch`) leaves an asleep row with `last_failure = Other` and `created = true`;
  - a missing `cwd` leaves an asleep row with `last_failure = SpawnFailed`;
  - an id already in the roster is refused with nothing started.
- `manager.rs` pin: extend `both_fresh_entrances_settle_the_link_before_reporting_success` (`:4817`).
- `crates/engram-dashboard-daemon/tests/control_agent.rs:610` `spawn_with_cwd_registers_once_and_announces_both_steps` — now one announcement; rename and assert one `ProfileListUpdated` + `AgentListUpdated` pair. (A daemon test file — run in P1 so P1 ends green.)

**P2 — daemon WS adapter** (`cargo test -p engram-dashboard-daemon -- --test-threads=4`, plus agent again for the removed function)
- Change:
  - `the_by_cwd_spawn_hands_the_blocking_call_to_the_pool` (`:2775`);
  - `spawn_by_cwd_without_a_backend_is_refused` (`:5152`, unchanged behaviour);
  - `spawn_by_cwd_refuses_a_backend_the_policy_closes` (`:5272`, policy now inside the body — still via `with_llm_policy`);
  - `a_ws_spawn_by_cwd_the_store_refuses_answers_conflict_and_registers_nothing` (`:6573`, unchanged answer);
  - `a_blocked_ws_activation_answers_with_its_reasons_and_starts_nothing` (`:7127`, the `SpawnByCwd` leg = pre-add block, unchanged).
- Remove: `by_cwd_fills_one_shared_default_mode_for_every_backend` (`:5416`, moved to P1); agent `tests/activation.rs:1462` `spawn_agent_reporting_carries_the_notices_of_a_start_and_never_records` (function removed).
- New (`connection_core.rs` tests, real manager, no process — a two-phase blocking kit):
  - a WS create blocked after the add answers `Error` with `"INTERNAL: agent '…' (…) was created but did not start"` + `launch_blocked`, the row exists with `last_failure = Other`, and `ProfileListUpdated` was broadcast;
  - a WS create success broadcasts the roster once and records no notice fingerprint (`Withheld`).
- `tests/ws_e2e.rs` `case40_ws_spawn_by_cwd` (`:2056`) must stay green unchanged.

**P3 — shell** (`cargo test -p engram-dashboard --test layout_apply`, `--test layout_commands`, `--test daemon_client_pending`, `--test lib_unit`)
- Change:
  - fake spawners in `src-tauri/tests/layout_apply.rs` (5 sites), `src-tauri/tests/layout_commands.rs` (3), `daemon_client/tests.rs` (3 `spawn_by_cwd` + the `send_command_resolves_err_on_matching_error` family for `Daemon { .. }`);
  - `layout/mod.rs` export.
- New:
  - `classify_reply` keeps `launch_blocked`;
  - `agent.spawnInto` success carries `notices` rows (and `[]` when the wire field is absent);
  - a block becomes `CONFLICT` + `args.reasons`;
  - `"INTERNAL: …"` becomes `INTERNAL` with the prefix stripped;
  - the `DaemonSpawner` arm for `Spawned` keeps `notices` (a real-socket test in the `OwnedSubs` style if one exists for the spawner; otherwise unit-level).
- Bindings sync gate (qa-verify 「생성물 동기」) for all three binding folders.

Full regression after each piece (qa-verify 「전체 회귀」). Mutation spot checks (qa 「변이 확인」):
- revert the WS arm to `spawn_agent_reporting` ⇒ P2's created-asleep test fails;
- drop `.with_args` in `spawn_not_applied` ⇒ P3's block test fails;
- skip the policy call ⇒ P1's policy test fails.

### 6-2. Live run (mandatory — ADR-0348 decision 7)

Setup = qa 「GUI 실측」 as written:
- build the debug client (`node scripts/build-client-shell.mjs`), daemon and CLI;
- launch the daemon first with `ENGRAM_DATA_DIR=$QA_TMP/data`, then vite, then the app with CDP 9223;
- folders are created under `$QA_TMP` with slashes;
- `ID` below = the agent id an answer returns.

Agent-list probe (GUI):

```js
node scripts/cdp.mjs eval "(() => { const r = document.querySelector('[data-agent-row=\"ID\"]'); return r && { shape: r.querySelector('[data-agent-glyph-shape]').dataset.agentGlyphShape, title: r.title, text: r.innerText } })()"
```

Expected values:
- running = `shape: "filled-circle"`;
- marked = `shape: "blocked"`;
- the marked `title` is the failure line: `SpawnFailed` = `에이전트를 띄우지 못했습니다 — 다시 시도해 보세요`, `Other` = `활성화에 실패했습니다 — 다시 시도해 보세요` (`src/i18n/ko.ts:111-118`).

CLI host (for the CLI scenes): the `engram` CLI needs `ENGRAM_TOKEN` / `ENGRAM_CONTROL_URL`, which only a spawned agent has (`cli/src/bin/engram.rs:240-255`). So:
1. Create a Terminal claude host: `window.__ENGRAM_AGENT__.createClaudeProfile('cli-host', '<QA_TMP>/host', [], [], false, 'Terminal')`, then `spawnProfile(id, false)`, then put it in a slot (`layout.setSlotContent`).
2. Answer claude's folder-trust prompt with `writeStdin(id, '\r')` if it shows.
3. Run commands through claude's `!` bash mode (no model turn): `writeStdin(id, encode('!"$ENGRAM_CLI_EXE" <command>'))`, then `writeStdin(id, encode('\r'))`.
4. Read the output with `run('slot.domMode.enable', { slotId })` and the slot's `innerText`.
5. Fallback if `!` mode does not run: ask the host to run the exact command (one model turn) and read the `tool_result` from `~/.claude/projects/<slug>/<session>.jsonl` (qa-verify 「에이전트 입출력」).

The host is a `SpawnProfile` agent — not under test. No claude login ⇒ the CLI scenes are PARTIAL (no means), not FAIL.

| # | Scenario | GUI | Exact action | Expected observable |
|---|---|---|---|---|
| R1 | Screen create, claude, success (webview registry → Tauri `spawn_into` → `SpawnByCwd`) | yes | `node scripts/cdp.mjs eval "window.__engramCmd.run('agent.spawnInto', { cwd: '<QA_TMP>/w1', backend: 'claude' })"` | returns a uuid string; probe on it within 3 s: `shape: "filled-circle"`, row text `w1` (name from the folder); the row appears without a manual refresh (`ProfileListUpdated` broadcast — new) |
| R2 | Webview `agent.spawn` (direct WS `SpawnByCwd`), claude, success | yes | `eval "window.__engramCmd.run('agent.spawn', { cwd: '<QA_TMP>/w2' })"` | resolves to AgentInfo JSON with `"name":"w2"`, `"status":"Running"`; probe `filled-circle` |
| R3a | Bus `agent.spawnInto` without the CLI (daemon relay → shell handler) | yes | `eval "(async () => window.__TAURI__.core.invoke('forward_daemon_command', { cmd: { Command: { envelope: { name: 'agent.spawnInto', request_id: crypto.randomUUID(), owner: '', proto_ver: 1, args: { cwd: '<QA_TMP>/w3a', backend: 'claude' } } } } }))()"` | reply payload ok with `agent_id`, `layout` rows, and **`notices: []`** (field present — new); probe `filled-circle` |
| R3b | CLI `agent.spawnInto`, claude, success incl. notices | yes (host + probe) | in the host: `!"$ENGRAM_CLI_EXE" agent.spawnInto --cwd <QA_TMP>/w3b --backend claude` | stdout JSON `{"agent_id":"<uuid>","layout":[…],"notices":[]}`, exit 0 (append `; echo rc=$?` — expect `rc=0`); probe `filled-circle`. Non-empty notices are not producible live today (no production code emits a notice — step 16 record); they are covered by P2/P3 tests |
| R4 | Launch block, live | — | **Not reachable with real inputs today** (§2 premise 9) | see U1. If U1 = (a): daemon relaunched with the debug-only marker env; `agent.spawnInto --cwd <QA_TMP>/block-me --backend claude` ⇒ CLI exit 1, body `{"status":"error","code":"CONFLICT","hint":"…","args":{"reasons":[…]}}`; no row named `block-me`; then bus `agent.spawn --cwd <QA_TMP>/block-me` ⇒ same `CONFLICT` + `args.reasons`, "nothing was created", no row. If U1 = (b): this row moves to step 8's acceptance (a preset folder entry pointing at a missing folder ⇒ block ①), with the same expectations |
| R5a | Bus `agent.spawn --cwd`, success | yes (host + probe) | host: `!"$ENGRAM_CLI_EXE" agent.spawn --cwd <QA_TMP>/w5 --name r5` | stdout `{"agent_id":"<uuid>","name":"r5","state":"live","created":true,"notices":[]}`, `rc=0`; probe `filled-circle`, text `r5` |
| R5b | Bus `agent.spawn --cwd`, failure after the add | yes (host + probe) | host: `!"$ENGRAM_CLI_EXE" agent.spawn --cwd <QA_TMP>/missing-r5` (folder not created) | `rc=1`; body `code:"INTERNAL"`, hint starts `agent 'missing-r5' (<uuid>) was created but did not start:` and contains the OS cause (expected `os error 267` — confirm), ends `…so start it again by that name (in a moment) instead of creating another`; `args.agent_id` = that uuid; probe `shape: "blocked"`, title = SpawnFailed line. Recovery: `mkdir <QA_TMP>/missing-r5`, then `!"$ENGRAM_CLI_EXE" agent.spawn --target missing-r5` ⇒ `rc=0`, `"created":false`, probe back to `filled-circle` (mark cleared — ADR-0172) |
| R6 | Screen create failure after the add leaves an asleep agent with the mark (new behaviour, ADR-0348 decision 6) | yes | `eval "window.__engramCmd.run('agent.spawnInto', { cwd: '<QA_TMP>/missing-r6', backend: 'claude' }).catch(e => String(e))"` | resolves to an error string containing `was created but did not start`; the agent list shows a row `missing-r6` with `shape: "blocked"`, title = SpawnFailed line (before this step: no mark). Bus form `forward_daemon_command` with `agent.spawnInto` + the same cwd ⇒ error code `INTERNAL`, prefix stripped |
| R7 | codex app-server create whose handshake really fails ⇒ failure + ban glyph | yes | stop the daemon (GUI cleanup line for the daemon PID). Write `<QA_TMP>/fakecodex/codex.cmd` = `@exit /b 1` (the codex backend runs `cmd.exe /c codex app-server …` via `PATH`, `backend/codex/mod.rs:182, 996`). Relaunch the daemon with `-EnvVars 'ENGRAM_DATA_DIR=<QA_TMP>/data','PATH=<QA_TMP win path>\fakecodex;<original PATH>'` (original from `powershell -NoProfile -Command "$env:Path"`). Then `eval "window.__engramCmd.run('agent.spawnInto', { cwd: '<QA_TMP>/w7', backend: 'codex' }).catch(e => String(e))"` | error string containing `was created but did not start` and the link cause; row `w7` with `shape: "blocked"`, title = `Other` line (link failure ⇒ `AgentFailureKind::Other`, `manager.rs` `spawn_fresh_settled`); no live codex session left (`list_agents` via `invoke` shows it not `Running`). Before this step: answered as started, then the row drops to `square` with no mark. Variant R7b (optional — records the cap risk): `codex.cmd` = `@ping -n 60 127.0.0.1 >nul` (never answers) ⇒ the webview call fails after ~15 s (backstop); the bus `agent.spawnInto` form answers `TIMEOUT` at ~7 s while the row later turns `blocked` |
| R8 | codex app-server create success, timing | yes (+ host for CLI) | restore the real daemon env (relaunch without the PATH override). `eval "(async () => { const t = performance.now(); const id = await window.__engramCmd.run('agent.spawnInto', { cwd: '<QA_TMP>/w8', backend: 'codex' }); return { id, ms: Math.round(performance.now() - t) } })()"`; then host: `!time "$ENGRAM_CLI_EXE" agent.spawnInto --cwd <QA_TMP>/w8b --backend codex` | uuid + `ms`; probe `filled-circle`; CLI `rc=0` with `real` well under 7 s. Record both durations in the QA report. If either is ≥ 5 s, raise the cap risk (§7 R1) to the user before commit |

Live-run evidence: answers verbatim (JSON / error strings), probe outputs, daemon log lines for the `SpawnByCwd` arm (`$QA_TMP/data/logs/`), and one screenshot (`node scripts/cdp.mjs shot $QA_TMP/r6.png`) showing the R6/R7 ban glyph rows. GUI cleanup per qa 「GUI 뒷정리」, including the grandchild scan (T-53). Optional contrast: run R6 and R7 once on the pre-change build (`28ecd79a`) to record the "before" (no mark / answered as started).

## 7. Implementation units

Sequential, each ending green (build + the piece's tests + full regression before its commit). They are sequential because they share files (P1 and P2 both touch `manager.rs` and the agent tests; P1 and P3 both touch `prompts/engram-help.md`).

| Piece | Files | Rough size | Ends green because |
|---|---|---|---|
| **P1** agent body + bus adapter | `agent/src/manager.rs` (`create_and_activate`, `NotStarted`, pin), `agent/src/commands.rs` (`NewAgent`, `CreateRefused`, `create_and_start`, `created_but_not_started`, `create_error`, host method + fakes, `verb_spawn` arm, summary + v9 note), `agent/tests/activation.rs`, `daemon/tests/control_agent.rs`, `agent/bindings/*` (regen), `prompts/engram-help.md` (line 69) | prod +200 / −90, tests +300 | WS still on `spawn_agent_reporting` (untouched); the bus uses the body |
| **P2** daemon WS adapter | `daemon/src/connection_core.rs` (arm, `RosterFanout: RosterChanged`, delete `spawn_command_by_cwd`, tests), `agent/src/manager.rs` (delete `spawn_agent_reporting`, docs), `agent/tests/activation.rs` (delete its test) | prod +70 / −120, tests ±150 | both entrances on the body; nothing else calls the removed function |
| **P3** shell pass-through | `src-tauri/src/daemon_client/protocol_state.rs`, `daemon_client/{connection,mod,refusal,tests}.rs`, `commands/view_bus.rs`, `commands/layout.rs`, `layout/apply.rs`, `layout/mod.rs`, `layout/commands.rs` (+22 note), `src-tauri/tests/layout_{apply,commands}.rs`, `src-tauri/bindings/*` (regen), `prompts/engram-help.md` (line 171 + 69 tail) | prod +150, tests +150 | the wire fields exist since step 16 P4; the webview Tauri type is unchanged |

Then `/engram-review code` per piece (or once over P1–P3), `/engram-qa` with the live run of §6-2 after P3, then the docs:
- `trd.md` §5-2 gets a step-20 entry; 10a and the step-11 handover each get an "absorbed into step 20" line;
- step-log;
- ADR-0348 erratum for §2 premises 3 and 8 (main);
- the ADR-0279 decision-3 amendment link is already in ADR-0348.

Code comments to fix in passing: `connection_core.rs` asymmetry comment (deleted); `manager.rs` `spawn_agent` doc (~1390, the codex-terminal claim); `create_and_start` 2026-09-22 comment (the policy now applies — ADR-0348 decision 4); the step-16 "Deferred" note in `trd.md` (pre-create check landed here).

## 8. Decisions, open items, risks

### 8-1. Decisions main made (within ADR-0348)

- **M1** Step number 20, after 16 and before 5; three commits P1–P3.
- **M2** Names: `AgentManager::create_and_activate` · `NotStarted { error, created }` · `commands::create_and_start` (pub, the widened body) · `NewAgent` · `CreateRefused` · `created_but_not_started`.
- **M3** Drop the bus's pre-activation roster frame: one notify after activation, on both entrances. Keeping it would need a callback threaded through `spawn_agent_watching_link`, which is not cheap. Visible effect: a bus-created codex row appears when its handshake settles (≤ 15 s).
- **M4** A missing `cwd` stays a failure after the add (asleep + `SpawnFailed` mark) — today's bus behaviour, now also WS. Refusing it before the add would be a new behaviour, not part of ADR-0348.
- **M5** Delete `spawn_agent_reporting` and its test — no production consumer is left (ADR-0348 invariant: one body).
- **M6** The WS created-asleep reply = `"INTERNAL: "` + the shared bus wording. The webview registry commands show the prefix raw until step 11.
- **M7** `agent.spawnInto` `notices` is a required `Vec` (`[]` when none), the same row shape as bus `agent.spawn`. Catalogs stay 9 / 22 (open generations) with note lines.
- **M8** Tauri `spawn_into` keeps `Result<String, String>` (the webview half is step 11's).
- **M9** A policy refusal on bus `agent.spawn --cwd` answers `INVALID_ARGUMENT` with `verb_new`'s wording. It is unreachable today.
- **M10** The shell forwards the daemon's `launch_blocked` object as `args` verbatim — no re-shaping in the shell, because the shell may not know the agent types (ADR-0270).

### 8-2. Open items for the user

- **U1 — live launch block (R4).** None is reachable with real inputs until step 8 plugs the real launch check (§2 premise 9).
  - (a) **Recommended:** a debug-build-only daemon hook. For example, under `#[cfg(debug_assertions)]`, an env var `ENGRAM_QA_BLOCK_CWD=<marker>` wraps `NoopLaunchKits` with a kit that blocks (reason ① `missing`) when `profile.cwd` contains the marker. About 30 lines in `daemon/src/lib.rs`, absent from release builds, removed when step 8 lands. It lets R4 run end-to-end now (daemon → shell → CLI `args.reasons`), which is the part of this step that has never run live.
  - (b) Defer R4 to step 8's acceptance (a preset folder entry on a missing folder ⇒ block ①). Until then cover the block only with the in-process tests (P1/P2/P3 — real WS and real manager, no live CLI).
  - **Decided by main (2026-11-06): (a)** — an internal test hook under the user's delegation for this step (「이부분 너가 알아서 쭉 진행」 · real-run requirement).

### 8-3. Risks

- **R1 Reply caps.** WS creates now wait for the codex handshake (≤ 15 s backstop), but the bus seat deadline is 7 s (CLI 10 s). A slow handshake answers `TIMEOUT` to `agent.spawnInto` while the agent is created and placed — the LLM may then create another. Today's bus codex wake has the same exposure. R8 measures; a fix (a longer deadline for spawn verbs, or answering before the verdict) is a separate decision.
- **R2 Retry duplicates on `agent.spawnInto`.** Delegated seats never hold ids (§2 premise 3), so a same-id retry after any failure re-runs. Mitigated only by the created-asleep text naming the agent.
- **R3 The created probe** can misreport `NotCreated` if a delete lands between the add and the probe (§5). The end state is still truthful.
- **R4 Source-pin tests** break by design (`the_by_cwd_spawn_hands_the_blocking_call_to_the_pool`, `squashed_production_body` windows); review must check they were rewritten to pin the new shape, not deleted.
- **R5 Codex link-failure path** has no automated fixture (`manager.rs:4810-4813`); R7's fake `codex.cmd` is the only execution evidence. A `codex.cmd` that exits may be classified by the transport as an early exit rather than a link failure — either must end in `blocked`; the exact kind (`Other` vs `SpawnFailed`) is recorded, not asserted.
- **R6 Webview error text** for created-asleep failures gains the `INTERNAL: ` prefix and the longer wording (registry commands only — no menu path).
- **R7 Missing-cwd cause text** is OS-localized (`os error 267` expected); R5b asserts the stable parts only.

## 9. Review record

(empty)
