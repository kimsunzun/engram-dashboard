# TRD — One creation body for "create a new agent for a folder and start it" (S25 step 20)

| Item | Value |
|---|---|
| **Status** | **Draft 3 — 2026-11-09 — canonical ADR-0348** (`docs/decisions/0348-creating-and-starting-a-new-agent-is-one-body-in-the-agent-crate-ws-spawnbycwd-and-bus-agentspawn-cwd-are-its-adapters.md`, errata `8cb86e55`, synced with this draft). ADR-0348 is the what/why (it stands in for a PRD). Draft 3 applies review round 2 (FIX). One user decision is pending: U2, re-opened (§8-2). U3 and U4 are decided. |
| **Line pointers** | HEAD `a70e3ecd` (after the master merge `907449da`). Every anchor below was re-read at this commit. |
| **Parent TRD** | [`trd.md`](trd.md) (S25 preset library) — this file is one new step of it (§1) |

## 1. Scope and relation to the S25 TRD

**What this step does.** It folds the two daemon bodies that create a new agent for a folder and start it into one body in the agent crate:
- WS `SpawnByCwd` (`crates/engram-dashboard-daemon/src/connection_core.rs:1693-1770`);
- bus `agent.spawn {cwd}` (`create_and_start`, `crates/engram-dashboard-agent/src/commands.rs:1211-1290`).

Both entrances become adapters. The shell also stops folding the `SpawnByCwd` reply. `agent.spawnInto` now answers with:
- `notices` — delivered, so recorded as shown;
- `args.reasons` on a launch block;
- `args.agent_id` when the agent was created but did not start (U3, decided 2026-11-09).

Two optional wire fields are added: `AgentCommand::SpawnByCwd.deliver_notices` and `AgentEvent::Error.created_agent_id` (§3-5, §4-1). No new verb, and no change to the shell ↔ daemon hop order (ADR-0279 decision 2 stands).

**Place in the S25 order.** New step **20**, run after step 16 (landed) and before §5-2 step 5 (user decision 2026-11-06, ADR-0348). Commit titles follow `S25: refactor(<scope>): … (step 20 P<n>)`.

**Absorbed from step 10a** (`trd.md` §5-2 10a, plus step 16's "Deferred" line):
- The pre-create launch check of `agent.spawn --cwd` (§1-5 (가) 1 ② — the `TODO(TRD S25 §5-2 10a)` at `commands.rs:1208`). A block before the add answers `CONFLICT` + `args.reasons`, with nothing created.
- Rewriting the `agent.spawn` summary ("registered before its launch is checked", `commands.rs:268-269`).

**Stays in step 10a:**
- every library, preset and agent-preset command declaration;
- `engram help library`;
- `agent.new {presets}` validation before `create_agent` (a different verb — it registers without starting).

**Absorbed from the step 11 handover** (`trd.md:905`):
- (a) partly — "give `SpawnByCwd` a recording path". The body takes a `NoticeDelivery`, and `SpawnByCwd.deliver_notices` lets a sender that shows notices ask for `Delivered`. The shell bus `agent.spawnInto` does so here.
- (b) partly — widening `daemon_client/protocol_state.rs` `classify_reply`, so `CommandFailure::Daemon` keeps `launch_blocked` (shared with (c)).
- (c) fully — shell bus `agent.spawnInto` passes `notices`, the structured block reasons and the created agent id.

**Stays in step 11:**
- the webview senders asking for delivery: `protocolClient.spawnAgent` and the Tauri `spawn_into` command keep `deliver_notices` false;
- moving `Spawn` from `Ack` to `Spawned`;
- the Tauri carrier `commands/agent.rs` `forward_to_daemon`, and the webview reading the widened replies;
- the Tauri `spawn_into` return type, which stays `Result<String, String>` here.

**Not in scope:**
- request-id retention for shell-owned (delegated) commands (§2 premise 3);
- a caller axis on the spawn packet (ADR-0279 decision 4 — `deliver_notices` is not one: it says what the sender does with notices, not who the caller is);
- a `backend` field on bus `agent.spawn` (ADR-0348 alternative (c));
- the claude early-exit limit — U4, decided as a known limit, tracked as T-81;
- a fix for the reply-cap `TIMEOUT` — U2, pending.

## 2. Premises checked against code

Verdicts at HEAD `a70e3ecd`, after the master merge `907449da` (S21 transport-attach, S33 translation). The agent crate anchors did not move in the merge. The daemon and shell anchors did; they are re-read below.

| # | Premise | Verdict | Evidence |
|---|---|---|---|
| 1 | The Fresh branch of `activate_profile_reporting` accepts a profile that is not in the roster | **True** | `manager.rs:1916-1990`: the in-flight check and the live-session lookup need no roster entry. Fresh goes to `spawn_fresh_settled` (`:2177`) → `spawn_agent_watching_link` (`:1427`) → `ensure_registered` (`:1324-1346`). That runs `check_before_create`, the capacity check and name assignment, then inserts (`inserted = true` skips `register_for_spawn`). The premise noted in the comment at `connection_core.rs:1725-1732` was already lifted by step 4 (§1-5 (가) 0). |
| 2 | `set_last_failure` on an unknown id is a no-op | **True** | `profile.rs:841-856` (the `_ => false` arm). A pre-add refusal that reaches `note_spawn_result` (`manager.rs:2133`) records nothing. |
| 3 | An `INTERNAL` answer on `agent.spawnInto` holds the request id | **False** (ADR-0348 corrected in `8cb86e55`) | `retains_the_id` (`command_delivery.rs:1450`) is called only on the daemon-local branch (`:1901`, `:1926`). A delegated round trip (a shell-owned command) frees its seat after the reply frame (`release`, `:890`). On `agent.spawnInto`, `INTERNAL` vs `CONFLICT` changes only the meaning; a same-id retry re-runs either way. Two things steer a caller away from a duplicate: `args.agent_id` (U3 — names the asleep agent to wake) and the created-asleep wording. |
| 4 | A new `ok` field or command summary needs a catalog version bump | **Fold into the open generation** | The rule is "a declaration change bumps `CATALOG_VERSION`" (`command/src/macros.rs:31`). Shell 22 (`layout/commands.rs:153`) and agent 9 (`commands.rs:58`) were set by S25 step 1 (`d2c80862`); no tag contains that commit (latest `v0.4.0`). Precedent: agent v9's note "The same unreleased generation … gains `notices`". So: note lines, no bump. Shell command declarations produce **no** TS bindings (`src-tauri/bindings/` has none); the agent's produce `crates/engram-dashboard-agent/bindings/commands.schema.json`. |
| 5 | Codex create wait vs reply caps | **Numbers** | See the breakdown below this table. |
| 6 | Today WS drops the codex link verdict | **True** | `spawn_agent_reporting` (`manager.rs:2043-2051`) discards the `LinkWatch`. `spawn_command_by_cwd` (`connection_core.rs:731`) uses `NEW_AGENT_OUTPUT_FORMAT` = `StreamJson` (`commands.rs:693`), and codex `StreamJson` is app-server, which `declares_link` (`backend/codex/mod.rs:707`). The `spawn_agent` doc claim (`manager.rs` ~1390) is out of date. No test fixture plays a refusing app-server (`manager.rs:4810-4813`), so R7c is the first live measure. |
| 7 | A screen gesture creates through `SpawnByCwd` | **False — registry commands only** | `slot.createAgentHere` left the slot menu (ADR-0067, `src/commands/slotContentCommands.ts:94`). Tree create is `CreateProfile` + `SpawnProfile` (`src/commands/agentCommands.ts:20-37`), untouched here. The WS entrances are: webview registry `agent.spawn` (`agentCommands.ts:69-92`), `slot.createAgentHere`, `agent.spawnInto` (`tabCommands.ts:338` → Tauri `spawn_into` → `DaemonSpawner`), and bus `agent.spawnInto` (→ `OwnedSpawner` → `DaemonSpawner`). |
| 8 | A missing `cwd` is refused before the add | **False** (ADR-0348 corrected in `8cb86e55`) | No `cwd` existence check exists before `ensure_registered` inserts (`register` only fixes the spelling, `commands.rs:1356`; Noop `check` reads no paths; the stdio spawn sets `current_dir`, `transport/spawn.rs:195`). A missing `cwd` is a failure after the add (`SpawnFailed` mark), on both entrances. Kept (M4). |
| 9 | A launch block is reachable today with real inputs | **False** | The daemon plugs no launch-kit source (`daemon/src/lib.rs:271` `TODO(TRD S25 §5-2 8)`), so `NoopLaunchKits` (`launch.rs:507-523`) never blocks. Policy refusals are unreachable too (`LLM_BACKEND_POLICY` opens claude and codex). Hence the debug-only hook (M12, §3-7). |
| 10 | Roster notify on WS `SpawnByCwd` today | **None** | No `broadcast_profile_list` in the arm (contrast the `SpawnProfile` arm, `:1508`). |
| 11 | Adding the two optional wire fields needs a `PROTOCOL_VERSION` bump | **No** | See the breakdown below this table. |
| 12 | A user kill during the codex handshake records a last failure | **No** | `spawn_fresh_settled`'s `EarlyVerdict::Terminal { status: Killed }` arm logs and does not record ("a user stop is not an activation failure"). The caller still gets `Err(SpawnFailed("…(Killed)"))` (`manager.rs:2247-2273`). Any other end before the link settles (`Terminal` other, `LinkFailed`, `Diagnosed`) records `Other` or the diagnosed kind. |

Premise 5 in detail:
- **Normal failure ceiling:** the codex handshake budget `HANDSHAKE_BUDGET` = **10 s** (`backend/codex/transport.rs:229`, link deadline at `:2499`).
- **Blocked-write backstop:** `LINK_RESOLUTION_BACKSTOP` = **15 s** (`manager.rs:81`). It only bites when a write blocks.
- **Worst case:** about **20 s**, including the teardown kill after a failed link.
- **Bus seat deadline:** every bus round trip has `DEFAULT_DEADLINE` = **7 s** (`command_delivery.rs:464`, set per seat at `:666`). That covers both the local `agent.spawn` and the relayed `agent.spawnInto`. It sits under the CLI's **10 s** read timeout (`agent/src/types.rs:521`).
- **Shell round-trip cap (after S21):** every shell → daemon round trip goes through the transport `Peer::request_in` (`daemon_client/connection.rs:515`). It is bounded by the link policy's `REQUEST_TIMEOUT` = **30 s** (`connection.rs:201-211` — "one value for every round trip"; the transport crate's default 8 s is overridden). That cap covers:
  - `DaemonSpawner` (Tauri `spawn_into` and bus `agent.spawnInto`);
  - the webview's `forward_to_daemon` path (`commands/agent.rs:218`).

  So draft 2's "no cap" for `DaemonSpawner` no longer holds. A timeout comes back as `CommandFailure::Local(reply_timeout_text)` (`link/errors.rs:50`).
- **Consequence:** a codex `agent.spawnInto` that fails slowly, or succeeds slowly, answers `TIMEOUT` at the 7 s bus seat deadline while the agent is created (and, on success, placed). The 30 s shell cap is never reached first. The ≈ 20 s worst case also stays under 30 s for the webview senders.
- **A healthy codex can be slow too.** `backend/codex/usage_probe.rs:38-39` records a healthy `initialize` observed at ≈ **9.7 s** right after a daemon restart (cause unknown; normal total 0.74–0.94 s). So the `TIMEOUT` is not limited to hung codexes, and the 10 s `HANDSHAKE_BUDGET` itself may be tight. This is U2, re-opened (§8-2).

Premise 11 in detail — no `PROTOCOL_VERSION` bump for either field:
- The wire enums take no `deny_unknown_fields` (`protocol/src/messages.rs:322`, `:458`), so both fields are additive in both directions.
- `SpawnByCwd.deliver_notices`:
  - an earlier v8 daemon with a new shell drops the key and answers as today (`Withheld`), so the set is shown again on the next delivering activation — harmless;
  - a new daemon with an old shell reads absent as `false`.
- `Error.created_agent_id`:
  - an old shell drops the key and still has the asleep agent named in the message text;
  - a new shell talking to an earlier v8 daemon gets no field and answers as today, without `args.agent_id`.
- v8 itself is unreleased (set by S25 step 1; no tag contains it).
- Add one line for both fields to the v8 doc in `protocol/src/lib.rs`, next to the step-16 line (`:143-146`).
- `crates/engram-dashboard-protocol/bindings/AgentCommand.ts` and `AgentEvent.ts` regenerate.

## 3. Module boundaries and signatures

The layers are:
1. agent manager — owns activation;
2. agent `commands` — the one body and the bus adapter;
3. protocol — two optional fields;
4. daemon `connection_core` — the WS adapter, plus the debug hook in `lib.rs`;
5. shell — the reply pass-through and the delivery request.

The agent crate stays free of Tauri and transport (ADR-0003), and the shell still does not depend on the agent crate (ADR-0270). Each item below gets the code anchor `// ADR-0348`.

### 3-1. Manager entry — `crates/engram-dashboard-agent/src/manager.rs`

```rust
/// A create that did not end in a started agent. `created` = this call added the agent to the roster; it
/// stays there asleep. Every failure after the add records last failure (ADR-0172) — except a user kill
/// during the link handshake (`spawn_fresh_settled`'s `Terminal { Killed }` arm).
pub struct NotStarted { pub error: PtyError, pub created: bool }

/// Create-and-start for a profile whose id is not in the roster — the Fresh activation path
/// (`activate_profile_reporting` → `spawn_fresh_settled`). The pre-create check, add, prelaunch, link
/// verdict, last-failure record and notice delivery all come from that path.
pub fn create_and_activate(
    &self,
    profile: &AgentProfile,
    delivery: NoticeDelivery,
) -> Result<Activated, NotStarted>
```

Body:
1. If `self.profiles.get(profile.id)` is `Some`, return `NotStarted { error: PtyError::Unsupported("…a new agent needs a new id — wake an existing one instead"), created: false }`. Callers always pass a fresh `AgentProfile::new` uuid, so this only guards against misuse.
2. Call `self.activate_profile_reporting(profile, SpawnMode::Fresh, delivery)`.
3. On `Err(error)`, set `created = self.profiles.get(profile.id).is_some()` (see §5).

Placement: put it where no existing `squashed_production_body` window moves. Check the split markers in the `manager.rs` tests (`:4778`, `:4838`, `:4872`, `:4909`, `:7129`). Extend `both_fresh_entrances_settle_the_link_before_reporting_success` (`:4817`) with a pin that `create_and_activate` calls `activate_profile_reporting(` with `SpawnMode::Fresh`.

**Removed in P2:**
- `spawn_agent_reporting` (`:2043`), which has no production consumer left;
- its test, `tests/activation.rs:1462`;
- the doc sentences that name it (`manager.rs` ~1390 and the `deliver_notices` doc).

### 3-2. The one body — `crates/engram-dashboard-agent/src/commands.rs`

```rust
pub struct NewAgent {
    pub cwd: String,
    pub name: Option<String>,
    pub backend: AgentBackend,
    /// `None` = create · `Some(reason)` = refuse. Production = `llm_creation_refusal`.
    pub policy: fn(&str) -> Option<&'static str>,
    pub delivery: NoticeDelivery,
}

pub enum CreateRefused {
    /// The backend policy refused — nothing registered, no last failure.
    Policy { reason: &'static str },
    /// Refused or failed before the add — nothing registered. In practice: a store refusal or write
    /// failure, the roster ceiling, name space, or a launch block (`check_before_create` returns only
    /// `LaunchBlocked`).
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
1. Call `(req.policy)(&backend_word(&req.backend))`. `Some(reason)` ⇒ `Err(Policy)`, and the host is not called.
   - **The policy receives the declared spelling** — `backend_word` serializes `AgentBackend`, giving `"Claude"` or `"Codex"` (`commands.rs:801-806`). That is the same spelling `verb_new` passes.
   - The production policy compares ignoring case (`llm_creation_refusal`, `commands.rs:791-796`).
   - WS today asks with the lowercase wire word (`wire_backend_word`, `connection_core.rs:692`). That function stays only for the WS refusal text (§3-5).
2. `cwd = normalize_spelling(&req.cwd)`.
3. Build the profile: `AgentProfile::new(cwd, backend_command(req.backend, NEW_AGENT_OUTPUT_FORMAT), PathBuf::from(cwd), vec![], false)` with `display_name = req.name`. This is the one home of the default output format; it replaces `spawn_command_by_cwd`.
4. Call `host.create_and_activate(&profile, req.delivery)`.
5. If that returned `Ok`, or `Err` with `created`, call `notify.roster_changed()` once. Do not notify when nothing was registered.
6. On `Err` with `created`, read the registered name via `host.agent_snapshot(id)` (`canonical_name_when_live()`, falling back to the id string) ⇒ `CreatedAsleep`. Otherwise ⇒ `NotCreated(error)`.

Shared wording, so both entrances say the same thing (today's text, `commands.rs:1253-1287`, moved out of the closure):

```rust
/// "agent '<name>' (<id>) was created but did not start: <cause> — it is registered and asleep, so <next>
/// instead of creating another"
pub fn created_but_not_started(name: &str, id: AgentId, error: &PtyError) -> String
```

- `cause`: `Store` → `store_error_cause`; `LaunchBlocked` → `launch_blocked_reason`; `LaunchRaced` → `LAUNCH_RACED_CAUSE`; anything else → `to_string()`.
- `next`: a block → "fix the listed items first, then start it by that name"; anything else → `AGAIN`.

### 3-3. `AgentCommandHost` addition

Sites: `commands.rs:495-530` (trait), `:614` (manager impl), `:2134` (`FakeHost`), `:3216` (`RefuseOnStart`).

```rust
fn create_and_activate(&self, profile: &AgentProfile, delivery: NoticeDelivery)
    -> Result<Activated, NotStarted>;
```

It returns `Activated` (full `AgentInfo` plus notices) because the WS reply needs `agent_info_to_wire(&AgentInfo)`; the bus maps it to `StartedAgent`. `create_agent` (used by `verb_new`) and `activate_profile` (used by `wake_existing`) stay as they are.

### 3-4. Bus adapter — `verb_spawn` cwd arm, `commands.rs:1138`

```rust
(None, Some(cwd)) => create_and_start(host, notify, NewAgent {
    cwd: cwd.to_string(), name: name.map(str::to_string),
    backend: AgentBackend::Claude,               // fixed on purpose (2026-09-22 rule, ADR-0348 decision 3)
    policy: llm_creation_refusal, delivery: NoticeDelivery::Delivered,
}).map(|a| started_payload(a.into(), true))
  .map_err(|refused| spawn_create_refusal(refused, cwd, &backend_word(&AgentBackend::Claude)))
```

- `spawn_create_refusal(refused, cwd, word) -> CommandError` maps per §4-2. `cwd` names the folder in the block text, and `word` names the backend in the policy text.
- The register error mapping (`commands.rs:1366-1391`) moves into a shared `fn create_error(e: PtyError) -> CommandError`, used by `register` (for `agent.new`) and by the `NotCreated` arm.
- The old private `create_and_start` and its second `roster_changed()` go away.

### 3-5. WS adapter and wire fields — `protocol/src/messages.rs:121-126, :695-708` · `connection_core.rs:1693-1770`

**Wire 1.** `AgentCommand::SpawnByCwd` gains:

```rust
/// The sender shows the reply's `notices` to a reader, so the daemon may mark the set as shown
/// (`NoticeDelivery::Delivered`). Absent = `false` = `Withheld`.
#[serde(default, skip_serializing_if = "std::ops::Not::not")]
deliver_notices: bool,
```

- The TS binding must be optional (`deliver_notices?: boolean`): `#[ts(optional)]` if the ts-rs version accepts it on `bool`, otherwise `Option<bool>` with `None` = false.
- `false` serializes to today's bytes.
- No `PROTOCOL_VERSION` move (§2 premise 11).

**Wire 2.** `AgentEvent::Error` (`messages.rs:695-708`) gains, after `launch_blocked`:

```rust
/// Present only when an activation that creates (`SpawnByCwd`) added the agent to the roster and it did not
/// start — the agent stays registered and asleep under this id; wake it instead of creating another. The
/// shell bus `agent.spawnInto` carries it as `args.agent_id`, the same shape as bus `agent.spawn`.
// TRD S25 step 20 (U3)
#[serde(default, skip_serializing_if = "Option::is_none")]
#[ts(optional)]
created_agent_id: Option<AgentId>,
```

- `AgentId` is the protocol id type (`protocol/src/ids.rs`, already imported at `messages.rs:11`).
- `None` serializes to today's bytes.
- Only the WS adapter's created-asleep arm sets it (§4-1). Every other `Error` construction passes `None`.

A new struct field forces every `AgentEvent::Error { … }` construction — and every pattern written without `..` — to name it. These are the sites step 16 touched for `launch_blocked`, re-read at `a70e3ecd`. All go in **P2**, so P2 ends green:

| Crate / file | Constructions (line) | Note |
|---|---|---|
| `protocol/src/messages.rs` | `:1487` (test) | plus new serde tests |
| `daemon/src/connection_core.rs` | `:1106` (`activation_error`), `:1433`, `:2229`, `:2273`, `:2305`, `:2687` | plus the test pattern at `:7756` (written without `..`) |
| `daemon/src/agent_conn.rs` | `:360`, `:369`, `:390`, `:436`, `:618`, `:864` | |
| `src-tauri/src/daemon_client/protocol_state.rs` | `:536`, `:545`, `:587` (tests) | |
| `src-tauri/src/daemon_client/connection/tests.rs` | `:444`, `:521`, `:1899`, `:2853`, `:3222` | |
| `src-tauri/src/daemon_client/link/handshake.rs` | `:211`, `:229`, `:250`, `:267` | |
| `src-tauri/src/daemon_client/link/dialer.rs` | `:456` | |
| `src-tauri/src/daemon_client/link/route.rs` | `:1493` | |

That is 27 constructions plus 1 pattern. `tests/ws_e2e.rs` and `commands/layout.rs` match `Error` with `..` only, so they are untouched. Bindings: `crates/engram-dashboard-protocol/bindings/AgentEvent.ts` regenerates.

**Senders:**

| Sender | Value | Where |
|---|---|---|
| shell bus `agent.spawnInto` | **true** | `DaemonSpawner` gains a `deliver_notices: bool` field; `OwnedSpawner` (the bus table's spawner, `commands/layout.rs:229-247, :275`) builds it with `true` |
| Tauri `spawn_into` (webview registry `agent.spawnInto`) | false | `commands/layout.rs:586` builds `DaemonSpawner { client, deliver_notices: false }` |
| webview `protocolClient.spawnAgent` (front `agent.spawn`, `slot.createAgentHere`) | absent (false) | `src/api/protocolClient.ts:980-982` unchanged |
| Rust test literals | false | `connection_core.rs:2803, :5759, :5881, :7174, :7749`; `tests/ws_e2e.rs:2069` |

**Arm.**
1. `let Some(kind) = backend else { reply(MISSING_BACKEND) }` (unchanged).
2. Map the backend with an exhaustive `match kind { WireBackendKind::Claude => AgentBackend::Claude, WireBackendKind::Codex => AgentBackend::Codex }`. No wildcard: a new wire word must fail to compile here.
3. `let delivery = delivery_for(deliver_notices);`, where `fn delivery_for(bool) -> NoticeDelivery` is `true` ⇒ `Delivered`, `false` ⇒ `Withheld`.
4. Copy what the closure needs out of `self` before the `move`. `ConnectionCore` is not `'static`, and `llm_policy` is a `fn` pointer (`Copy`):
   - `let policy = self.llm_policy;`
   - `let roster = RosterFanout::new(self.fanout.clone(), manager.clone());` — `fanout` is the core's `Arc<dyn FrameFanout>` (`:1278`). The core holds no `RosterFanout`, and it is not `Clone`, so build a fresh one.
5. Add `impl RosterChanged for RosterFanout` next to its `RosterBroadcast` impl (`:2635`). It sends profile list then agent list (the same two frames as the bus notify). Amend `RosterFanout`'s doc (`:2612-2623`; struct at `:2624`): "`DaemonWiring::roster_broadcast` is the only production construction point" (`:2619`) becomes "… and the WS `SpawnByCwd` arm, from the core's own `fanout` + `manager` (the same assembly)".
6. Run `tokio::task::spawn_blocking(move || create_and_start(&*mgr, &roster, NewAgent { cwd, name: None, backend, policy, delivery }))`.
7. Map the result per §4-1. The created-asleep arm builds `Error { request_id: Some(request_id), message, launch_blocked, created_agent_id: Some(id) }` directly — not through `activation_error`, which stays `created_agent_id: None` for every activation verb.
8. Delete `spawn_command_by_cwd` (`:731`) and the asymmetry comment (`:1725-1732`).
9. `by_cwd_refusal` (`:762`) keeps its wording; it takes `(kind, reason)` instead of calling the policy.
10. A `JoinError` maps to `activation_error(… PtyError::SpawnFailed("spawn task failed: …"))`, as today.
11. Update the `llm_policy` doc (`:1294-1300`: "asked before `spawn_agent_reporting`" → "passed to the one creation body").

### 3-6. Shell pass-through — `src-tauri`

**`daemon_client/protocol_state.rs:261-270`:**
- `CommandFailure::Daemon` becomes `{ message: String, launch_blocked: Option<serde_json::Value>, created_agent_id: Option<AgentId> }`.
- `classify_reply` (`:283`, called from `connection.rs:516`) copies both fields from `AgentEvent::Error` (`:285`).
- `Display` (`:269`) is unchanged, so `send_command`'s string stays byte-identical.
- Sites to touch (6):
  - `commands/view_bus.rs:127`;
  - `daemon_client/connection/tests.rs:525`;
  - `protocol_state.rs:262` (decl), `:269`, `:285`, `:590`.

  Patterns gain `..`. (Draft 2's `daemon_client/tests.rs` was deleted by the merge.)

**`layout/apply.rs`:**
- `AgentSpawner::spawn_by_cwd` (`:247`) returns `Result<SpawnedAgent, SpawnRefusal>`, with:
  - `pub struct SpawnedAgent { pub id: String, pub notices: Vec<serde_json::Value> }`
  - `pub struct SpawnRefusal { pub message: String, pub launch_blocked: Option<serde_json::Value>, pub created_agent_id: Option<AgentId> }`
- `spawn_into` (`:824`), `spawn_into_placed` (`:853`), `spawn_into_at` (`:943`) and `spawn_then_place` (`:994`) carry `SpawnRefusal` as their error type throughout.
- Local errors (pre-spawn validation, placement) are `SpawnRefusal { message, launch_blocked: None, created_agent_id: None }`.
- `SpawnAtError::Failed` carries `SpawnRefusal`.
- `Wrote<String>` becomes `Wrote<SpawnedAgent>`.
- A placement failure after a successful spawn keeps the spawn's `notices` (M11 — they were delivered, so they are recorded as shown).

**`AgentSpawner` impls (5):**
- `DaemonSpawner` (`commands/layout.rs:106`);
- `OwnedSpawner` (`:233`);
- `NoSpawner` (`daemon_client/test_support.rs:258-260`);
- `Spawner` (`tests/layout_apply.rs:447`);
- `DaemonSpawner` fake (`tests/layout_commands.rs:317`).

**`commands/layout.rs:103-131` — `DaemonSpawner { client, deliver_notices }`** calls `send_command_with_origin` with `SpawnByCwd { …, deliver_notices }`:
- `Ok(Spawned { agent, notices })` ⇒ `SpawnedAgent { id, notices: notices.unwrap_or_default() }`;
- `Err(Daemon { message, launch_blocked, created_agent_id })` ⇒ `SpawnRefusal { message, launch_blocked, created_agent_id }`;
- `Err(Local(m))` ⇒ `SpawnRefusal { message: m, launch_blocked: None, created_agent_id: None }`. A transport timeout (30 s, §2 premise 5) arrives here.

The Tauri `spawn_into` (`:567`) keeps `Result<String, String>`.

**`layout/commands.rs` — `agent.spawnInto`:**
- `AgentSpawnIntoOk` gains `notices: Vec<LaunchNoticeRow>`.
- `LaunchNoticeRow { code: String, args: Json, line: Option<u32>, text: String }` is declared in the shell block, in the same shape as the agent block's (`agent commands.rs:224-233`).
- `spawn_not_applied` (`:2920`) takes `SpawnRefusal`:
  - the code-prefix mapping is unchanged;
  - `args` = the `launch_blocked` object (its `reasons`) when present;
  - plus `"agent_id": <created_agent_id>` when present;
  - both keys together for a block after the add.

  This is the same shape as bus `agent.spawn`'s created-asleep answer (`args = {agent_id, reasons?}`).
- The placement-failure answer carries `args.notices` when the spawn returned any (M11).
- The summary is updated (§4-3).
- Catalog stays 22 with a note line. No TS bindings.

**`prompts/engram-help.md`:**
- line 69: the `--cwd` pre-create block is `CONFLICT` with nothing created;
- line 171: the `agent.spawnInto` answer carries `notices`; a block creates nothing and carries `args.reasons`; an `INTERNAL` with `args.agent_id` means the agent was created but did not start — it is asleep and not placed, so wake that id (`agent.spawn --target <agent_id>`) and place it with `layout.setSlotContent --assign`, instead of spawning again.

### 3-7. Debug-only launch-block hook — `crates/engram-dashboard-daemon/src/lib.rs` (M12, owned by P2)

At the `lib.rs:271` TODO, under `#[cfg(debug_assertions)]` only, the daemon reads two env vars. If either is set, it plugs a `QaBlockKits` wrapper over `NoopLaunchKits` through `with_launch_ports`:
- `ENGRAM_QA_BLOCK_CWD=<marker>` — `check` returns a `LaunchBlocked` with one reason ① `missing` (item = the cwd) whenever `profile.cwd` contains the marker. That blocks at `check_before_create`, so nothing is created.
- `ENGRAM_QA_BLOCK_AFTER_ADD_CWD=<marker>` — for a matching cwd, the first `check` for a profile id passes and the second (in `prelaunch`) blocks. That gives a block after the add: an asleep agent with an `Other` mark.

Rules:
- Release builds never contain it.
- It is logged at `warn` when active.
- One test (`lib.rs`, `#[cfg(all(test, debug_assertions))]`): a kit built from the two markers blocks the matching cwd on the right call, and passes a non-matching cwd. The kit is built from a constructor, not from the process env.
- **Removal is handed to step 8** (§7-1).

## 4. Answer shapes per entrance

### 4-1. WS `SpawnByCwd` (`AgentEvent`)

| Outcome | Reply |
|---|---|
| started | `Spawned { request_id, agent: agent_info_to_wire(&info), notices: notices_to_wire(&notices) }`. `notices` is absent when empty (step-16 bytes). It is marked as shown only when `deliver_notices` was `true`. |
| `backend` absent | `Error { message: MISSING_BACKEND, launch_blocked: None }` (unchanged) |
| `Policy { reason }` | `Error { message: by_cwd_refusal(kind, reason), launch_blocked: None }` (unchanged wording, lowercase wire word) |
| `NotCreated(e)` | `activation_error(conn_id, "SpawnByCwd", "the agent was not created or started", request_id, e)` (unchanged). A store error gets the `"CONFLICT: …"` / `"INTERNAL: …"` prefix (`store_error_text`); a launch block gets `launch_blocked: {"reasons":[…]}`; anything else is plain text. |
| `CreatedAsleep { id, name, error }` | `Error { message: "INTERNAL: " + created_but_not_started(name, id, &error), launch_blocked: block ? Some(to_args()) : None, created_agent_id: Some(id) }` (new). On bus `agent.spawnInto` the prefix becomes code `INTERNAL` (`spawn_not_applied`; `INTERNAL` is a common error, `command/src/spec.rs:60`), and `created_agent_id` becomes `args.agent_id`. The webview registry commands see the text raw until step 11 (they ignore the field). |

Every row other than `CreatedAsleep` carries `created_agent_id: None` (absent on the wire).

### 4-2. Bus `agent.spawn {cwd}` (`AgentSpawnOk` / `CommandError`)

| Outcome | Answer |
|---|---|
| started | `{agent_id, name, state, created: true, notices: [...]}` — delivered and recorded (unchanged) |
| `Policy { reason }` | `INVALID_ARGUMENT`, `verb_new` policy wording with `word` (unreachable today — both backends open) |
| `NotCreated(LaunchBlocked)` | New: `CONFLICT`, text `"{LAUNCH_REFUSAL} for a new agent in '<cwd>': <reasons>; nothing was created — fix the listed items, then try again"`, `args = blocked.to_args()`. The request id is released (`retains_the_id`), so the fixed retry runs. |
| `NotCreated(LaunchRaced)` | **Defensive.** `CONFLICT` with `RetryMode::AfterCondition`, "…nothing was created — try again". `check_before_create` returns only `LaunchBlocked` (`prelaunch.rs:28-33`); a real race arises only after the add (it surfaces as `CreatedAsleep`). This arm keeps the mapping total. |
| `NotCreated(other)` | today's `register` mapping: `RosterFull` / `Unsupported` ⇒ `CONFLICT`; `CwdDenied` ⇒ `INVALID_ARGUMENT`; `Store` ⇒ `store_command_error` (refusal `CONFLICT`, write failure `INTERNAL`); anything else ⇒ `INTERNAL` |
| `CreatedAsleep { id, name, error }` | `INTERNAL`, `created_but_not_started(…)`, `args = {agent_id}` (+ `reasons` for a block) — unchanged; the id stays held (daemon-local command, `retains_the_id`) |

The `agent.spawn` summary (`commands.rs:264-269`) becomes: "With `cwd`, a launch blocked before the agent is added answers `CONFLICT` with `args.reasons` and creates nothing; a start failure after the add is `INTERNAL` with `args.agent_id` (plus `args.reasons` when blocked) — start that agent by target, do not create another." `commands.schema.json` regenerates; the catalog stays 9 with a note line.

### 4-3. Shell bus `agent.spawnInto`

| Outcome | Answer |
|---|---|
| started | `{agent_id, layout, notices: [...]}`. `notices` = the daemon's rows `{code, args, line, text}`; `[]` when none. A row that fails to decode is dropped, with a `warn` naming it. The set is recorded as shown (`deliver_notices: true`). |
| daemon `Error` with `launch_blocked`, before the add | `CONFLICT` (the block text has no prefix ⇒ `not_applied`), message = daemon text, `args = launch_blocked` (`{"reasons":[…]}`) |
| daemon `Error` `"INTERNAL: …"` with `created_agent_id` (created-asleep — R4b, R5b-style, R6 bus form) | `INTERNAL`, prefix stripped (`spawn_not_applied`), `args = {agent_id}` — plus `reasons` for a block after the add (R4b). The message names the agent too. |
| daemon `Error` `"INTERNAL: …"` without `created_agent_id` | `INTERNAL`, prefix stripped, no `args`. A store write failure before the add: nothing was created. |
| placement failed after the spawn | as today (`CONFLICT`, "배치 실패(…) — 에이전트 <id> 는 살아있음"), plus `args.notices` when present (M11) |
| other daemon / local refusal | unchanged (`spawn_not_applied` / `spawn_at_error`) |

Summary addition: "Answer adds `notices`. A blocked launch creates nothing and is `CONFLICT` with `args.reasons`. An `INTERNAL` with `args.agent_id` (plus `args.reasons` when blocked) means the agent was created but did not start — it is asleep and not placed: wake that id (`agent.spawn --target`), then place it with `layout.setSlotContent --assign`; do not spawn again." The catalog stays 22 with a note line.

## 5. Concurrency

- **Lock order is unchanged.** `create_and_activate` holds no lock of its own. `ensure_registered` already runs `check_before_create` before the name-allocation gate (asset-library lock → manager lock, `manager.rs:1330-1335`). The created check is one short registry read after activation returns, with no lock held. `notify.roster_changed()` runs after the manager call.
- **Blocking.** Both adapters keep the body off async workers:
  - WS runs it in `spawn_blocking` (as today, `connection_core.rs:1738`). The pin `the_by_cwd_spawn_hands_the_blocking_call_to_the_pool` (`:2945`) is rewritten to find `create_and_start(` inside the `spawn_blocking` window.
  - Bus runs it in `blocking_handler` (`make_table`).
  - On WS the body now waits for the link. A failed codex handshake normally takes ≤ 10 s (`HANDSHAKE_BUDGET`). The worst case is about 20 s including teardown, against a 15 s backstop that applies only to a blocked write. A healthy one was once seen at ≈ 9.7 s (§2 premise 5). All of these are under the shell's 30 s round-trip cap, but can exceed the bus's 7 s (U2).
- **The policy now runs on the blocking-pool thread** (inside the body), no longer on the dispatch thread. `spawn_by_cwd_refuses_a_backend_the_policy_closes` (`:5863`) keeps its closed word in a `thread_local!` (`:5864`; "the handler calls the policy synchronously inside dispatch"). That would read empty on the pool thread. The moved test therefore keeps the word in a process-global (`static … Mutex<String>`) and compares ignoring case (§3-2).
- **Created check after `Err`.** If a delete lands between the add and the check, an agent that was created then deleted is reported as `NotCreated`. The end state is still what the answer says. Nobody knows the fresh uuid before the answer, except through a roster listing. Accepted.
- **Existing-id refusal.** Without it, `ensure_registered` would return `inserted = false` and the call would silently become a wake that merges the caller's copy.
- **Reservation.** The new id's `SpawnReservation` is taken before the add, so the `agent.setWorkspace` race named in `trd.md:252` (it can win between `create_agent` and `activate_profile`) disappears for this body.
- **Roster frames.** The bus loses its pre-activation `roster_changed` (M3); WS gains one after activation. Bus `--cwd` is claude-only (no link axis), so its row appears right after the process starts. Codex rows come only through WS, and appear when the handshake settles (≤ 10 s normally).

## 6. Test plan

QA binding rows (`.claude/skill-bindings/qa.md` 「돌릴 것 표」):
- **Rust source** ⇒ full regression.
- **Frontend** (`src/api/protocolClient.test.ts`) ⇒ full regression + live run, GUI on.
- **Agent lifecycle (spawn)** ⇒ full regression + live run, GUI on.
- claude / codex lanes: no touched path is in a lane list, so lanes run only under the CLI-version rule.

### 6-1. Automated, per piece

**P1 — agent crate** (`cargo test -p engram-dashboard-agent -- --test-threads=4`)

Change (`commands.rs` tests):
- `spawn_by_cwd_creates_then_starts` (`:2563`) — one `create_and_activate` call with `Fresh` / `Delivered`, normalized `cwd`, `display_name = name`, command `backend_command(Claude, NEW_AGENT_OUTPUT_FORMAT)`.
- `a_created_agent_that_fails_to_start_is_not_rolled_back` (`:2584`).
- `a_start_failure_after_the_create_stays_internal_and_names_the_created_agent` (`:2674`).
- `the_spawn_answer_carries_the_launch_notices_as_rows` (`:2731`).
- `a_create_and_start_whose_create_the_store_refuses_starts_nothing_and_announces_nothing` (`:3184`) — now `NotCreated(Store)`, no notify.
- `a_created_agent_whose_start_the_store_refuses_stays_internal_with_the_recovery_hint` (`:3213`).
- `FakeHost` (`:2134`) and `RefuseOnStart` (`:3216`) gain `create_and_activate`.

New (`commands.rs`):
- a block before the add ⇒ `CONFLICT` + `args.reasons`, `LAUNCH_REFUSAL` prefix, no notify, no add;
- *defensive* — the `NotCreated(LaunchRaced)` mapping ⇒ `CONFLICT` retry (the fake host returns it; not reachable from the real manager);
- policy refusal ⇒ host never called (injected policy closing `"Claude"`, compared ignoring case);
- the body asks the policy with the declared spelling;
- one notify on success and on created-asleep, none on `NotCreated`;
- the body builds the same default mode for both backends (replaces the daemon's `by_cwd_fills_one_shared_default_mode_for_every_backend`, `connection_core.rs:6007`);
- `created_but_not_started` wording per error kind.

New (`tests/activation.rs` — real child processes, existing `noticing_manager` / `long_lived_profile` / `recorded` fixtures, `:150`, `:1229`, `:1253`):
- `create_and_activate` of an unregistered profile with `Delivered` registers, starts, returns the notices and records the fingerprint.
- **Regression for review finding 1:** a `Delivered` create, then kill and reap, then a `Delivered` wake (`activate_profile_reporting(.., Fresh, Delivered)`) returns **no** notices.
- **Withheld (replaces draft 1's impossible daemon test):** a `Withheld` create leaves the fingerprint `None`, and a later `Delivered` wake returns the set.
- A block before the add leaves no roster row and no last failure.
- A block after the add (a kit that passes `check_before_create` and blocks `prelaunch`) leaves an asleep row with `last_failure = Other` and `created = true`.
- A missing `cwd` leaves an asleep row with `last_failure = SpawnFailed`.
- An id already in the roster is refused, with nothing started.

Also in P1:
- `manager.rs` pin: extend `:4817`.
- `crates/engram-dashboard-daemon/tests/control_agent.rs:616` `spawn_with_cwd_registers_once_and_announces_both_steps` — now one announcement. Rename it and assert one `ProfileListUpdated` + `AgentListUpdated` pair.

**P2 — protocol + daemon WS adapter + debug hook** (`cargo test -p engram-dashboard-protocol`; `cargo test -p engram-dashboard-daemon -- --test-threads=4`; agent again for the removed function)

Protocol (`messages.rs` tests), compatibility both ways for both fields:
- `SpawnByCwd`:
  - an old frame without the key decodes with `deliver_notices == false`;
  - `false` serializes without the key (old bytes);
  - `true` round-trips.
- `Error`:
  - an old frame without `created_agent_id` decodes as `None`;
  - `None` serializes without the key (old bytes — the step-16 test at `:1462` keeps passing);
  - `Some(id)` round-trips next to `launch_blocked`;
  - a frame carrying `created_agent_id` decodes in a reader that predates it. Model this as deserializing into a local copy of the step-16 `Error` shape — the enums carry no `deny_unknown_fields`, so the key is dropped.

Change (P2 also carries every mechanical `created_agent_id: None` literal of the §3-5 table, shell files included, so the workspace builds):
- `connection_core.rs` tests:
  - the five `SpawnByCwd` literals gain `deliver_notices: false`;
  - `the_by_cwd_spawn_hands_the_blocking_call_to_the_pool` (`:2945`);
  - `spawn_by_cwd_without_a_backend_is_refused` (`:5743`, unchanged behaviour);
  - `spawn_by_cwd_refuses_a_backend_the_policy_closes` (`:5863`) — process-global closed word, compared ignoring case (§5); still through `with_llm_policy`;
  - `a_ws_spawn_by_cwd_the_store_refuses_answers_conflict_and_registers_nothing` (`:7164`, unchanged answer; asserts `created_agent_id == None`);
  - `a_blocked_ws_activation_answers_with_its_reasons_and_starts_nothing` (`:7723`) — its `SpawnByCwd` leg is a pre-add block, unchanged; its pattern at `:7756` gains the field and asserts `None` (nothing was created).

Remove:
- `by_cwd_fills_one_shared_default_mode_for_every_backend` (`:6007`, moved to P1);
- agent `tests/activation.rs:1462` `spawn_agent_reporting_carries_the_notices_of_a_start_and_never_records` (its function is removed).

New (daemon):
- `delivery_for(true) == Delivered`, `delivery_for(false) == Withheld`;
- source pin: the `SpawnByCwd` arm passes `delivery_for(deliver_notices)` into `NewAgent` (no literal `NoticeDelivery::` in the arm);
- **post-add vs pre-add**:
  - a WS create blocked after the add (two-phase blocking kit, real manager, no process) answers `Error` with `"INTERNAL: agent '…' (…) was created but did not start"`, `launch_blocked`, and `created_agent_id == Some(<the roster row's id>)`. The row exists with `last_failure = Other`, and `ProfileListUpdated` was broadcast once.
  - a WS create blocked before the add answers with `created_agent_id == None` and no row.
- source pin: `activation_error` builds `created_agent_id: None`; only the `SpawnByCwd` created-asleep arm sets `Some`;
- the §3-7 hook test.

`tests/ws_e2e.rs` `case40_ws_spawn_by_cwd` (`:2062`, literal `:2069`) gains `deliver_notices` and must stay green.

**P3 — shell + webview pin** (`cargo test -p engram-dashboard --test layout_apply`, `--test layout_commands`, `--test daemon_client_pending`, `--test lib_unit`; `npm test`; `npx tsc --noEmit`)

Change:
- the five `AgentSpawner` impls (§3-6);
- the 6 `CommandFailure::Daemon` sites;
- the `layout/mod.rs` export.

New:
- `classify_reply` keeps `launch_blocked` and `created_agent_id` (`protocol_state.rs` tests);
- `agent.spawnInto` success carries `notices` rows, and `[]` when the wire field is absent;
- a pre-add block becomes `CONFLICT` + `args.reasons`, with no `args.agent_id`;
- a created-asleep refusal (`"INTERNAL: …"` + `created_agent_id`) becomes `INTERNAL` with the prefix stripped and `args.agent_id`. For a block after the add, `args` has both `agent_id` and `reasons`;
- an `"INTERNAL: …"` without `created_agent_id` becomes `INTERNAL` with no `args`;
- `DaemonSpawner` maps `Daemon { created_agent_id }` into `SpawnRefusal` (fake-reply unit test in `daemon_client/connection/tests.rs`, harness from `test_support.rs`);
- a placement failure after a spawn with notices carries `args.notices`;
- source pins in `commands/layout.rs`: the Tauri `spawn_into` builds `DaemonSpawner { …, deliver_notices: false }`, and `OwnedSpawner` builds it with `true`;
- `src/api/protocolClient.test.ts:154` additionally asserts that the sent `SpawnByCwd` has no `deliver_notices` key.

Gates and checks:
- Bindings sync gate (qa-verify 「생성물 동기」): the expected changes are in `crates/engram-dashboard-agent/bindings/commands.schema.json` (P1) and `crates/engram-dashboard-protocol/bindings/AgentCommand.ts` + `AgentEvent.ts` (P2). `src-tauri/bindings/` gets nothing new.
- Full regression after each piece (qa-verify 「전체 회귀」).
- Mutation checks (qa 「변이 확인」), each of which must turn a test red:
  - revert the WS arm to `spawn_agent_reporting` ⇒ P2's created-asleep test;
  - hard-code `Withheld` in the WS arm ⇒ P2's pin;
  - drop `.with_args` in `spawn_not_applied` ⇒ P3's block test;
  - drop the `agent_id` key in `spawn_not_applied` ⇒ P3's created-asleep test;
  - set `created_agent_id: None` in the WS created-asleep arm ⇒ P2's post-add test;
  - skip the policy call ⇒ P1's policy test.

### 6-2. Live run (mandatory — ADR-0348 decision 7)

**Setup** follows qa 「GUI 실측」 as written:
- build the debug client (`node scripts/build-client-shell.mjs`), the daemon and the CLI;
- launch the daemon first with `ENGRAM_DATA_DIR=$QA_TMP/data`, plus `ENGRAM_QA_BLOCK_CWD=qa-block-me` and `ENGRAM_QA_BLOCK_AFTER_ADD_CWD=qa-block-late` (debug build — §3-7);
- then vite, then the app with CDP 9223;
- folders live under `$QA_TMP`, written with slashes;
- `ID` below = the agent id an answer returns.

**Before-run (mandatory for R6 and R7c).** Swap only the daemon binary:
1. `git worktree add --detach "$QA_TMP/base" "$BASE"` (see 2 for `BASE`).
2. Build `cargo build -p engram-dashboard-daemon --target-dir "$QA_TMP/target-base"` in it (qa 「기준점 사본」 rules). The base is **the branch head P1 is built on** — the post-merge commit just before P1's first commit, recorded as `BASE=$(git rev-parse <first P1 commit>^)`. Not `28ecd79a`: that predates the master merge (`907449da`, S21 transport-attach), so its daemon would differ from the new one in more than this step.
3. Stop the daemon, relaunch that exe with the same env and data folder, and keep the new client running.
4. Run R6 and R7c and record the "before" answers and probes.
5. Relaunch the new daemon.

The old daemon ignores `deliver_notices` and never sends `created_agent_id` (§2 premise 11), so the new client is safe against it.

**Agent-list probe** (GUI):

```js
node scripts/cdp.mjs eval "(() => { const r = document.querySelector('[data-agent-row=\"ID\"]'); return r && { shape: r.querySelector('[data-agent-glyph-shape]').dataset.agentGlyphShape, title: r.title, text: r.innerText } })()"
```

Probe values:
- running = `"filled-circle"`;
- stopped = `"square"`;
- marked = `"blocked"`, with the title giving the failure line (`src/i18n/ko.ts:111-118`): `SpawnFailed` = `에이전트를 띄우지 못했습니다 — 다시 시도해 보세요`, `Other` = `활성화에 실패했습니다 — 다시 시도해 보세요`.

**Bus calls — two routes:**
- **CLI route.** The `engram` CLI needs `ENGRAM_TOKEN` / `ENGRAM_CONTROL_URL`, which only a spawned agent has (`cli/src/bin/engram.rs:240-255`). So:
  1. Create a Terminal claude host: `window.__ENGRAM_AGENT__.createClaudeProfile('cli-host', '<QA_TMP>/host', [], [], false, 'Terminal')`, then `spawnProfile(id, false)`, then place it with `layout.setSlotContent`.
  2. Answer the folder-trust prompt with `writeStdin(id, '\r')` if it shows.
  3. Run each command through claude's `!` bash mode (no model turn): `writeStdin(id, encode('!"$ENGRAM_CLI_EXE" <command>; echo rc=$?'))`, then `'\r'`.
  4. Read the output with `slot.domMode.enable` and the slot's `innerText`.
  5. If `!` mode does not run, ask the host to run the exact command in one model turn, and read the `tool_result` from `~/.claude/projects/<slug>/<session>.jsonl`.
  6. With no claude login, the CLI route is PARTIAL (no means).
- **Token-free route** (qa-verify.md:118), used for every bus scene as a fallback and always for R3a:

  ```js
  node scripts/cdp.mjs eval "(async () => window.__TAURI__.core.invoke('forward_daemon_command', { cmd: { Command: { envelope: { name: '<command>', request_id: crypto.randomUUID(), owner: '', proto_ver: 1, args: { … } } } } }))()"
  ```

  It checks the same daemon routing and handler. It does not check the CLI's exit code or control body.

**Scenarios.** All scenes need the GUI for the probe; the CLI forms also need the host.

R1 — screen create, claude, success (webview registry → Tauri `spawn_into` → `SpawnByCwd`, `deliver_notices` false):
- Action: `node scripts/cdp.mjs eval "window.__engramCmd.run('agent.spawnInto', { cwd: '<QA_TMP>/w1', backend: 'claude' })"`.
- Expected:
  - returns a uuid string;
  - probe within 3 s: `"filled-circle"`, row text `w1`;
  - the row appears without a manual refresh (new `ProfileListUpdated`).

R2 — webview `agent.spawn` (direct WS `SpawnByCwd`), claude, success:
- Action: `eval "window.__engramCmd.run('agent.spawn', { cwd: '<QA_TMP>/w2' })"`.
- Expected: AgentInfo JSON with `"name":"w2"` and `"status":"Running"`; probe `"filled-circle"`.

R3a — bus `agent.spawnInto` without the CLI (daemon relay → shell handler):
- Action: token-free route with `name: 'agent.spawnInto'`, `args: { cwd: '<QA_TMP>/w3a', backend: 'claude' }`.
- Expected:
  - the reply is OK, with `agent_id`, `layout` rows and **`notices: []`** (the field is present — new);
  - probe `"filled-circle"`.

R3b — CLI `agent.spawnInto`, claude, success including notices:
- Action (host): `agent.spawnInto --cwd <QA_TMP>/w3b --backend claude`.
- Expected:
  - stdout `{"agent_id":"<uuid>","layout":[…],"notices":[]}` and `rc=0`;
  - probe `"filled-circle"`.
- Non-empty notices cannot be produced live (no production code emits one — step 16 record). The automated tests cover them, including the record-once regression.

R4 — live launch block before the add (hook M12):
- Action, two parts:
  1. host or token-free: `agent.spawnInto --cwd <QA_TMP>/qa-block-me-1 --backend claude`;
  2. bus `agent.spawn --cwd <QA_TMP>/qa-block-me-2`.
- Expected:
  - part 1: CLI `rc=1`, body `{"status":"error","code":"CONFLICT","hint":"…","args":{"reasons":[{"kind":"missing",…}]}}`;
  - part 2: `CONFLICT`, hint starting `launch refused for a new agent in '…qa-block-me-2'` and containing "nothing was created", with `args.reasons`;
  - no row named `qa-block-me-*`;
  - daemon log has the hook's `warn`.

R4b — *optional* — block after the add (second marker):
- Action: bus `agent.spawn --cwd <QA_TMP>/qa-block-late-1`, then `agent.spawnInto --cwd <QA_TMP>/qa-block-late-2 --backend claude`.
- Expected:
  - each answers `INTERNAL`, with hint `agent 'qa-block-late-N' (<uuid>) was created but did not start: …fix the listed items first…`;
  - `args` = `{agent_id: <uuid>, reasons: [...]}` on **both** — bus `agent.spawn`, and `agent.spawnInto` through `Error.created_agent_id` (U3);
  - probe on that uuid: `"blocked"` with the `Other` title.

R5a — bus `agent.spawn --cwd`, success:
- Action (host or token-free): `agent.spawn --cwd <QA_TMP>/w5 --name r5`.
- Expected: `{"agent_id":"<uuid>","name":"r5","state":"live","created":true,"notices":[]}`, `rc=0`; probe `"filled-circle"`, text `r5`.

R5b — bus `agent.spawn --cwd`, failure after the add:
- Action (host or token-free): `agent.spawn --cwd <QA_TMP>/missing-r5` (folder not created).
- Expected:
  - `rc=1`, `code:"INTERNAL"`;
  - hint starts `agent 'missing-r5' (<uuid>) was created but did not start:`, contains the OS cause (expected `os error 267`), and ends `…instead of creating another`;
  - `args.agent_id` = that uuid;
  - probe `"blocked"` with the SpawnFailed title.
- Recovery:
  1. `mkdir <QA_TMP>/missing-r5`.
  2. `agent.spawn --target missing-r5` ⇒ `rc=0`, `"created":false`.
  3. Probe ⇒ `"filled-circle"` (mark cleared — ADR-0172).

R6 — screen create failure after the add leaves an asleep agent with the mark (ADR-0348 decision 6). Run it on the before-run daemon as well.
- Action: `eval "window.__engramCmd.run('agent.spawnInto', { cwd: '<QA_TMP>/missing-r6', backend: 'claude' }).catch(e => String(e))"`.
- Expected:
  - an error string containing `was created but did not start`;
  - row `missing-r6`, `"blocked"`, SpawnFailed title;
  - token-free bus form `agent.spawnInto`, same cwd suffix `-b` ⇒ `INTERNAL`, prefix stripped, `args.agent_id` = the id of row `missing-r6-b`, no `args.reasons`; the agent is not placed (no slot holds it).
- Before-run: an error without the created-asleep wording and without `args.agent_id`, and the row (if shown) has no mark.

R7c — codex app-server create whose handshake really fails, with teardown observed (mandatory; run on the before-run daemon as well).
- Setup:
  1. Write `<QA_TMP>/fake-app-server.js` with the Write tool. It reads newline-delimited JSON from stdin. In mode `error` it answers the `initialize` request (`{"jsonrpc":"2.0","id":<its id>,"error":{"code":-32600,"message":"qa: refused"}}`). In mode `silent` it answers nothing. In both modes it stays alive (`setInterval`).
  2. Write `<QA_TMP>/fake-error/codex.cmd` = `@node "%~dp0..\fake-app-server.js" error %*` and `<QA_TMP>/fake-silent/codex.cmd` = `@node "%~dp0..\fake-app-server.js" silent %*`. `%*` forwards codex's arguments, so the agent's fake carries `--cd <scene cwd>` on its command line. The codex backend runs `cmd.exe /c codex app-server …` resolved through `PATH` (`backend/codex/mod.rs:182, :996`).
  3. Stop the daemon and relaunch it with `-EnvVars` adding `PATH=<QA_TMP win>\fake-error;<original PATH>` (original from `powershell -NoProfile -Command "$env:Path"`).
  4. **Keep the codex usage probe out of the scan.** The probe also launches `cmd /c codex app-server --stdio` (`backend/codex/usage_probe.rs:90-98`), so with the fake on PATH a probe can start a fake too. Two guards:
     - no slot shows codex usage during R7c–R7d (`layout.get` — no `Usage` slot with `show_codex`), so the shell sends no codex usage interest;
     - the scan matches only command lines containing both `fake-app-server.js` **and** the scene's cwd (`w7c`, `w7s`, `w7d`). A probe's command line has `--stdio` and no `--cd`.
- Action: `eval "(async () => { const t = performance.now(); const r = await window.__engramCmd.run('agent.spawnInto', { cwd: '<QA_TMP>/w7c', backend: 'codex' }).catch(e => String(e)); return { r, ms: Math.round(performance.now() - t) } })()"`.
- Then repeat with `fake-silent` on PATH, cwd `w7s`.
- Expected, error mode:
  - `r` contains `was created but did not start`, and `ms` is short (well under 10 s);
  - row `"blocked"` with the **`Other`** title;
  - grandchild scan (qa 「GUI 뒷정리」, `QA_TAG`, filtered as in setup step 4) finds **no** fake for `w7c`.
- Expected, silent mode: the same, with `ms` ≈ 10 s (`HANDSHAKE_BUDGET`).
- Before-run: `r` is a uuid (answered as started), the row is not marked, and the scan **finds** the fake still running.

R7 — *extra* — fake that exits:
- Action: `fake-exit/codex.cmd` = `@exit /b 1`, same action.
- Expected:
  - created-asleep error;
  - `"blocked"`, `Other` title (`Terminal` exit before the link settles ⇒ `Other`, `manager.rs:2247-2266`).
- Cheap, but it cannot tell teardown apart (the process dies by itself) — R7c is the proof.

R7b — *optional* — cap behaviour on the bus (U2):
- Action: `fake-silent` on PATH; token-free `agent.spawnInto` with cwd `w7b`.
- Expected:
  - the reply is `TIMEOUT` at ≈ 7 s (seat deadline);
  - the row turns `"blocked"` at ≈ 10 s.
- This records the U2 behaviour for the user.

R7d — kill during the handshake window:
- Action:
  1. `fake-silent` on PATH.
  2. `eval "window.__p = window.__engramCmd.run('agent.spawnInto', { cwd: '<QA_TMP>/w7d', backend: 'codex' }).catch(e => String(e)); 1"`.
  3. Within 10 s, find the id through token-free `agent.list` (the row whose cwd ends `w7d` — it is registered before the spawn), then `eval "window.__engramCmd.run('agent.kill', { agentId: 'ID' })"`.
  4. Then `eval "window.__p"`.
- Expected:
  - the created-asleep error, with cause `…(Killed)`;
  - the error carries `created_agent_id`, so the bus form would carry `args.agent_id`;
  - probe `"square"`, **no** `"blocked"` mark (a user kill is not an activation failure — §2 premise 12);
  - the filtered scan finds no fake for `w7d`.

R8 — codex app-server create success, timing:
- Action:
  1. Relaunch the daemon without the PATH override.
  2. `eval "(async () => { const t = performance.now(); const id = await window.__engramCmd.run('agent.spawnInto', { cwd: '<QA_TMP>/w8', backend: 'codex' }); return { id, ms: Math.round(performance.now() - t) } })()"`.
  3. Host: `time "$ENGRAM_CLI_EXE" agent.spawnInto --cwd <QA_TMP>/w8b --backend codex`.
- Expected:
  - a uuid and `ms`; probe `"filled-circle"`;
  - CLI `rc=0`, with `real` recorded;
  - record both durations, and attach them to U2 (§8-2) — they are the data for the pending decision. Restart the daemon once and repeat step 3 immediately to probe the ≈ 9.7 s cold `initialize` (§2 premise 5). If the CLI form answers `TIMEOUT` while the row is `"filled-circle"`, record that as the U2 symptom, not as a FAIL.

**Evidence.** Keep all of the following in the QA report:
- the answers verbatim;
- probe outputs;
- the grandchild-scan output for R7c and R7d (after the answer, on both builds);
- daemon log lines for the `SpawnByCwd` arm and the hook (`$QA_TMP/data/logs/`);
- screenshots `node scripts/cdp.mjs shot $QA_TMP/r6.png` and `$QA_TMP/r7c.png`;
- the before-run answers.

Cleanup follows qa 「GUI 뒷정리」, including the grandchild scan (T-53) and removing `$QA_TMP/base` (qa 「뒷정리」).

## 7. Implementation units

The pieces are sequential, each ending green (build, the piece's tests, then full regression before its commit). They are sequential because they share files:
- P1 and P2 both touch `manager.rs` and `tests/activation.rs`;
- P2 and P3 both touch `src-tauri/src/commands/layout.rs`, `protocol_state.rs` and `connection/tests.rs`;
- P1 and P3 both touch `prompts/engram-help.md`.

**P1 — agent body + bus adapter.** Rough size: prod +200 / −90, tests +350.
- Files:
  - `agent/src/manager.rs` (`create_and_activate`, `NotStarted`, pin);
  - `agent/src/commands.rs` (`NewAgent`, `CreateRefused`, `create_and_start`, `created_but_not_started`, `create_error`, `spawn_create_refusal`, the host method and fakes, the `verb_spawn` arm, the summary and the v9 note);
  - `agent/tests/activation.rs`;
  - `daemon/tests/control_agent.rs`;
  - `agent/bindings/commands.schema.json` (regenerated);
  - `prompts/engram-help.md` (line 69).
- Ends green because WS stays on `spawn_agent_reporting` (untouched) while the bus uses the body.

**P2 — protocol fields + WS adapter + debug hook.** Rough size: prod +150 / −120, tests ±250, plus about 28 one-line literal edits.
- Files:
  - `protocol/src/messages.rs` (`deliver_notices`, `created_agent_id`, tests);
  - `protocol/src/lib.rs` (v8 doc line);
  - `protocol/bindings/AgentCommand.ts`, `AgentEvent.ts` (regenerated);
  - `daemon/src/connection_core.rs` (the arm, `delivery_for`, the created-asleep `Error`, `RosterFanout: RosterChanged` and its doc, the `llm_policy` doc, deleting `spawn_command_by_cwd`, the `Error` literals, tests);
  - `daemon/src/agent_conn.rs` (6 `Error` literals);
  - `daemon/src/lib.rs` (§3-7 hook and test);
  - `daemon/tests/ws_e2e.rs`;
  - `agent/src/manager.rs` (deleting `spawn_agent_reporting` and its docs);
  - `agent/tests/activation.rs` (deleting its test);
  - shell, mechanical only, to keep the workspace compiling:
    - `src-tauri/src/commands/layout.rs:115` (`deliver_notices: false`);
    - `created_agent_id: None` in the `Error` literals of `daemon_client/protocol_state.rs`, `daemon_client/connection/tests.rs` and `daemon_client/link/{handshake,dialer,route}.rs` (§3-5 table).
- Ends green because both entrances are on the body, nothing calls the removed function, the shell still sends `false`, and it does not read `created_agent_id` yet.

**P3 — shell pass-through + delivery request.** Rough size: prod +190, tests +200.
- Files:
  - `src-tauri/src/daemon_client/protocol_state.rs` (`CommandFailure::Daemon` fields, `classify_reply`, tests);
  - `daemon_client/connection.rs` (unchanged call at `:516`; only if a pattern needs `..`) and `daemon_client/connection/tests.rs` (`:525` pattern, the `DaemonSpawner` mapping test);
  - `daemon_client/test_support.rs` (`NoSpawner`);
  - `commands/view_bus.rs` (`:127` pattern);
  - `commands/layout.rs` (`DaemonSpawner { deliver_notices }`, `OwnedSpawner` true, Tauri `spawn_into` false, pins);
  - `layout/apply.rs`, `layout/mod.rs`;
  - `layout/commands.rs` (`notices`, `LaunchNoticeRow`, `args.reasons` + `args.agent_id`, placement `args.notices`, summary, 22 note);
  - `src-tauri/tests/layout_{apply,commands}.rs`;
  - `src/api/protocolClient.test.ts`;
  - `prompts/engram-help.md` (line 171 + the line-69 tail).
- Ends green because the wire fields exist since step 16 P4 (plus P2's two), and the webview Tauri type is unchanged.

After the pieces:
- `/engram-review code` per piece (or once over P1–P3);
- `/engram-qa` with the §6-2 live run after P3;
- then the doc pass:
  - parent `trd.md` §5-2 gets a step-20 entry, with "absorbed into step 20" lines at 10a and the step-11 handover;
  - parent `trd.md` ~`:390` (the `agent.spawn` row) and ~`:529` (the "until 10a" parenthesis) are updated;
  - step-log.

Code comments to fix in passing:
- the `connection_core.rs` asymmetry comment (deleted);
- the `manager.rs` `spawn_agent` doc (~1390, the codex-terminal claim);
- the `create_and_start` 2026-09-22 comment (the policy now applies — ADR-0348 decision 4);
- the `llm_policy` doc (`connection_core.rs:1294-1300`);
- the `RosterFanout` doc (`:2612-2623`);
- the step-16 "Deferred" note in `trd.md`.

### 7-1. Hand-over to the parent TRD (main copies these in)

- **Step 8:** remove the debug-only `QaBlockKits` hook (`daemon/src/lib.rs`, the `ENGRAM_QA_BLOCK_CWD` and `ENGRAM_QA_BLOCK_AFTER_ADD_CWD` env vars, its test) in the same commit that plugs the real `LaunchKitSource` at `lib.rs:271`. Then re-run this TRD's R4 / R4b with a real trigger (a preset folder entry on a missing folder ⇒ block ①, before and after the add).
- **Step 11:** the webview senders (`protocolClient.spawnAgent`, Tauri `spawn_into`) set `deliver_notices: true` once the webview shows `Spawned.notices`. Until then they stay false.

## 8. Decisions, open items, risks

### 8-1. Decisions main made (within ADR-0348)

- **M1** Step 20, after 16 and before 5; three commits P1–P3.
- **M2** Names: `AgentManager::create_and_activate`, `NotStarted { error, created }`, `commands::create_and_start` (pub — the widened body), `NewAgent`, `CreateRefused`, `created_but_not_started`, `delivery_for`.
- **M3** Drop the bus's pre-activation roster frame: one notify after activation, on both entrances. Keeping it would need a callback threaded through `spawn_agent_watching_link`. The visible cost is nil for bus `--cwd` (claude, no link axis).
- **M4** A missing `cwd` stays a failure after the add (asleep, `SpawnFailed` mark) — today's bus behaviour, now on WS too.
- **M5** Delete `spawn_agent_reporting` and its test.
- **M6** The WS created-asleep reply is `"INTERNAL: "` + the shared bus wording. The webview registry commands show the prefix raw until step 11.
- **M7** `agent.spawnInto` `notices` is a required `Vec` (`[]` when none), in the bus `agent.spawn` row shape. Catalogs stay at 9 / 22 with note lines.
- **M8** The Tauri `spawn_into` keeps `Result<String, String>` (webview half = step 11).
- **M9** A policy refusal on bus `agent.spawn --cwd` answers `INVALID_ARGUMENT` with `verb_new`'s wording. Unreachable today.
- **M10** The shell forwards the daemon's `launch_blocked` object as `args` verbatim (ADR-0270 — the shell does not know the agent types).
- **M11** A placement failure after a delivered spawn carries the spawn's `notices` in `args.notices`. Otherwise a set recorded as shown would never be shown.
- **M12** The debug-only launch-block hook (§3-7) is owned by P2, and its removal is handed to step 8 (§7-1). Adopted by main from draft 1's U1 (a).
- **M13** `SpawnByCwd.deliver_notices` (optional, default false): shell bus `agent.spawnInto` sends `true`; the webview senders stay false until step 11. No `PROTOCOL_VERSION` move (§2 premise 11). Main decision, from review finding 1.
- **M14** U3 (a), as specified here:
  - `AgentEvent::Error.created_agent_id: Option<AgentId>` (optional, absent when `None`), set only by the WS created-asleep arm;
  - it flows through `CommandFailure::Daemon` → `SpawnRefusal` → `spawn_not_applied` into `args.agent_id`, next to `args.reasons`;
  - no `PROTOCOL_VERSION` move (§2 premise 11).

  The name says *created*, so a reader cannot take it for "the agent this error is about" on other verbs.

### 8-2. User decisions — one pending (U2), two decided (U3, U4)

- **U2 — `TIMEOUT` on codex `agent.spawnInto`** (review H1). **Re-opened 2026-11-09 — pending user.** Design hooks are left neutral; this TRD changes no deadline.
  - *Earlier decision:* (b) accept (user 2026-11-09, 「추천으로 가」). Its basis was "a healthy codex answers `initialize` in ≈ 130 ms and a refusal comes back at once (`codex/transport.rs` `HANDSHAKE_BUDGET` doc), so only a silently hung codex exceeds 7 s".
  - *Why re-opened (review round 2):* `backend/codex/usage_probe.rs:38-39` records a **healthy** `initialize` observed at ≈ 9.7 s right after a daemon restart (cause unknown; normal 0.74–0.94 s). So:
    - a healthy codex `agent.spawnInto` can answer `TIMEOUT` (bus seat 7 s, CLI read 10 s) while the agent is running and placed — the LLM may then spawn again;
    - the 10 s `HANDSHAKE_BUDGET` itself may be tight: a 9.7 s start is within 0.3 s of being torn down as a failure.
  - *Known gap tied to U2 (notices):* on a slow success the daemon records the notice set as shown (`Delivered`, M13), while the bus caller receives `TIMEOUT` with no notices. That breaks the parent TRD rule "the fingerprint is recorded only when a success reply reaches a caller" (§1-5 「Notices once per agent」). No live effect until step 8, because no production code emits notices today. Option (a) removes it for spawns that settle inside the new deadline. Under (b) it stays a recorded gap.
  - *Options:*
    - (a) a longer deadline for spawn verbs — the daemon seat for `agent.spawn` / `agent.spawnInto` plus the CLI read timeout, which must stay above the seat (`fits_caller_silence_window`, `command_delivery.rs`);
    - (b) accept `TIMEOUT` as it is.
  - *Data for the decision:* R7b (silent fake on the bus) and R8 (healthy codex timings, including one cold start right after a daemon restart).
- **U3 — `agent.spawnInto` created-asleep answer carries `args.agent_id`** (review H2). **Decided 2026-11-09 by the user: (a)** (「1번 a」, main's recommendation). Specified as M14 (§3-5 Wire 2, §3-6, §4-1, §4-3, §6-1). Same answer shape as bus `agent.spawn`; the LLM can wake that id and attach it instead of creating a duplicate.
- **U4 — a claude that exits right after launch is answered "started" with no mark** (review H3). **Decided 2026-11-09 by the user: (a)** (「너 추천으로 해」) — a known limit. `cmd.exe /c claude` starts, and claude has no link verdict (ADR-0201 left claude Fresh early exit out of scope). ADR-0348 decision 6 states the limit; tracked as `docs/tracking.md` T-81. Not touched here.
- **T-76 closes with this step** — 「즉석 by-cwd codex 의 연결 실패에 주인이 없다」 is the WS link-verdict gap this step fixes (R7c proves it); delete the T-76 row and section when step 20 lands (T-71 rule).

### 8-3. Risks

- **R1 Reply caps** — see U2 (pending), including the notices gap. Timing: 10 s `HANDSHAKE_BUDGET` is the normal failure ceiling, and the 15 s backstop applies only to a blocked write. Worst case ≈ 20 s with teardown — under the shell's 30 s round-trip cap, over the bus's 7 s seat.
- **R2 Retry duplicates on `agent.spawnInto`.** Delegated seats never hold ids (§2 premise 3), so a same-id retry after any failure re-runs. Mitigated by `args.agent_id` (U3/M14 — the caller can wake the asleep agent by id) and the created-asleep wording. A caller that ignores both still creates a duplicate.
- **R3 Created check race** (§5). The end state is still truthful.
- **R4 Source-pin tests break by design** (`connection_core.rs:2945`, the `squashed_production_body` windows, the policy test's thread-local at `:5864`). Review must confirm they were rewritten to pin the new shape, not deleted.
- **R5 Codex link failure** has no automated fixture. R7c (persistent fake) is the only execution evidence of the teardown, and it is mandatory, with a before-run contrast.
- **R6 Webview error text** for created-asleep failures gains the `INTERNAL: ` prefix and the longer wording (registry commands only).
- **R7 The missing-cwd cause text** is OS-localized (`os error 267` expected); R5b asserts only the stable parts.
- **R8 The fake app-server's framing** assumes newline-delimited JSON-RPC on stdin. If the error-mode answer is not read as a refusal, the silent mode still proves the timeout teardown. Record which happened.
- **R9 About 28 mechanical `Error` literal edits in P2** (§3-5 table). They are spread over daemon and shell crates; a missed one is a compile error, not a silent bug.

## 9. Review record

- 2026-11-06 — draft 1 `/engram-review trd deep` (2 Claude reviewers + Codex): **FIX**. Five MUST, the recommended items and the batched wording were applied in draft 2. H1–H3 went to the user as U2–U4. One finding was refined rather than taken verbatim: "every failure after the add records last failure" now excludes a user kill during the handshake (§2 premise 12).
- 2026-11-09 — round 2 delta review (Claude lead + Codex): round-1 findings 1–5 **FIXED**; new **FIX** → draft 3. Applied:
  - U3 fully specified (`Error.created_agent_id`, M14);
  - ADR-0348 synced (decisions 3 and 5, 영향);
  - the U2 ⇄ M13 notices gap recorded;
  - before-run base = the head P1 is built on;
  - R7c/R7d scan filtered against the codex usage probe;
  - the shell round-trip cap pinned at 30 s (S21 `REQUEST_TIMEOUT`);
  - all anchors re-read after the master merge `907449da`.

  U2 re-opened (pending user). Re-review pending.
