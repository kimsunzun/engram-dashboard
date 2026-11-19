//! ADR-0132 piece 2 integration test — the `/control/agent` route (agent control verbs).
//!
//! No claude needed: real processes are launched with the shell backend (every runner has one). The one exception is `spawn --cwd` (create and launch), which creates a claude agent, so startup success depends on the machine — that test asserts only what does not vary (see its comment).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use engram_dashboard_agent::launch::{
    BlockReason, LaunchBlocked, LaunchKit, LaunchKitSource, LaunchPlan, LaunchPorts,
    NoopLaunchKits, PrepareError, Reason,
};
use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::manager::MAX_ROSTER_SIZE;
use engram_dashboard_agent::profile::{
    AgentCommand, AgentProfile, ProfileRegistry, ProfileStore, StoreError,
};
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, NoopControlChannel, StatusSink,
    CLI_CONTROL_READ_TIMEOUT_SECS,
};
use engram_dashboard_command::{CommandEnvelope, CommandReply};
use engram_dashboard_daemon::command_delivery::{BusSweeper, CommandBus, CommandDeliveries};
use engram_dashboard_daemon::command_roster::CommandRoster;
use engram_dashboard_daemon::conn_port::{ConnId, Frame, FrameError, FrameSink};
use engram_dashboard_daemon::control::agent::RosterBroadcast;
use engram_dashboard_daemon::control::commands::{
    make_daemon_table, NoInputLeases, NoLibrary, NoUsageLimits,
};
use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server_without_help, CommandTableSlot, ManagerSlot, McpServerHandle, MessagingSlot,
    RosterBroadcastSlot,
};
use engram_dashboard_daemon::control::registry::ControlRegistry;
use futures_util::future::BoxFuture;

// ── harness ────────────────────────────────────────────────────────────────────────

/// An outlet that discards frames — an owner that registers a name but never answers (only whether the relay reaches is checked).
struct DiscardingSink;

impl FrameSink for DiscardingSink {
    fn try_send(&self, _frame: Frame) -> Result<(), FrameError> {
        Ok(())
    }

    fn send(&self, _frame: Frame) -> BoxFuture<'_, Result<(), FrameError>> {
        Box::pin(async { Ok(()) })
    }
}

/// A client that really answers the names it put on — what the dashboard does for window/tab/slot commands.
///
/// It reads the envelope back and attaches the outcome to the slot table as a real client does (WS `CommandOutcome` → `CommandDeliveries::complete`); a shortcut would measure this double, not the relay.
struct AnsweringSink {
    deliveries: CommandDeliveries,
    /// The envelope the owner actually received — where to check that names and arguments crossed unchanged.
    seen: Arc<Mutex<Vec<CommandEnvelope>>>,
}

impl FrameSink for AnsweringSink {
    fn try_send(&self, frame: Frame) -> Result<(), FrameError> {
        let Frame::Text(text) = frame else {
            return Ok(());
        };
        if let Ok(engram_dashboard_protocol::AgentEvent::CommandRequest { envelope }) =
            serde_json::from_str(&text)
        {
            // The envelope field is raw JSON (ADR-0272) — the receiving owner unpacks it.
            let envelope: CommandEnvelope = serde_json::from_str(envelope.get())
                .expect("the envelope the daemon wrapped can be unpacked");
            let reply = CommandReply::ok(
                envelope.request_id,
                serde_json::json!({ "ran": envelope.name, "with": envelope.args }),
            );
            self.seen.lock().expect("poisoned").push(envelope);
            self.deliveries.complete(reply);
        }
        Ok(())
    }

    fn send(&self, frame: Frame) -> BoxFuture<'_, Result<(), FrameError>> {
        Box::pin(std::future::ready(self.try_send(frame)))
    }
}

/// Differs from [`AnsweringSink`] in one thing — the outcome is attached later by another task, not on this thread.
///
/// That is the measured axis: the sibling calls `complete` inside `try_send`, so the answer exists before the delivery lies down in the reply slot and the "lie down, then wake up" branch never runs. A production owner is another process, so the answer always comes later (a WS frame round trip).
struct ParkedAnsweringSink {
    deliveries: CommandDeliveries,
    /// Answers this long after receiving the envelope — slack so the delivery has surely lain down.
    after: Duration,
}

impl FrameSink for ParkedAnsweringSink {
    fn try_send(&self, frame: Frame) -> Result<(), FrameError> {
        let Frame::Text(text) = frame else {
            return Ok(());
        };
        if let Ok(engram_dashboard_protocol::AgentEvent::CommandRequest { envelope }) =
            serde_json::from_str(&text)
        {
            // The envelope field is raw JSON (ADR-0272) — the receiving owner unpacks it.
            let envelope: CommandEnvelope = serde_json::from_str(envelope.get())
                .expect("the envelope the daemon wrapped can be unpacked");
            let deliveries = self.deliveries.clone();
            let after = self.after;
            tokio::spawn(async move {
                tokio::time::sleep(after).await;
                deliveries.complete(CommandReply::ok(
                    envelope.request_id,
                    serde_json::json!({ "ran": envelope.name, "late": true }),
                ));
            });
        }
        Ok(())
    }

    fn send(&self, frame: Frame) -> BoxFuture<'_, Result<(), FrameError>> {
        Box::pin(std::future::ready(self.try_send(frame)))
    }
}

struct NoopSink;
impl StatusSink for NoopSink {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

#[derive(Default)]
struct MemProfileStore {
    saved: Mutex<Vec<AgentProfile>>,
}
impl ProfileStore for MemProfileStore {
    fn save(&self, profiles: &[AgentProfile]) -> Result<(), StoreError> {
        *self.saved.lock().expect("poisoned") = profiles.to_vec();
        Ok(())
    }
    fn load(&self) -> Vec<AgentProfile> {
        self.saved.lock().expect("poisoned").clone()
    }
}

/// Roster notification observer — counts which verbs trigger a screen refresh.
#[derive(Default)]
struct CountingBroadcast {
    calls: Mutex<usize>,
}
impl CountingBroadcast {
    fn count(&self) -> usize {
        *self.calls.lock().expect("poisoned")
    }
}
impl RosterBroadcast for CountingBroadcast {
    fn roster_changed(&self) {
        *self.calls.lock().expect("poisoned") += 1;
    }
}

struct Fixture {
    manager: Arc<AgentManager>,
    base: String,
    token: String,
    /// Held so more keys can be issued — an axis varying per credential (mail permission) needs two or more.
    registry: Arc<ControlRegistry>,
    broadcast: Arc<CountingBroadcast>,
    /// The one copy the server actually reads: names put on through this handle check that discovery and calls see the bus the assembly passed (not a copy).
    bus: CommandBus,
    /// Must be held — if dropped, the collector empties and closes the slot table on its way out (`CommandDeliveries::drain`) and every later call is refused as "shutting down".
    _sweeper: BusSweeper,
    _handle: McpServerHandle,
}

impl Fixture {
    async fn post(&self, body: serde_json::Value) -> (reqwest::StatusCode, serde_json::Value) {
        self.post_with(Some(self.token.clone()), body).await
    }

    async fn post_with(
        &self,
        bearer: Option<String>,
        body: serde_json::Value,
    ) -> (reqwest::StatusCode, serde_json::Value) {
        self.post_to("/control/agent", bearer, Some(body)).await
    }

    /// A sibling that names the path — for the two catalog routes. `body` absent = a POST with no body.
    async fn post_to(
        &self,
        path: &str,
        bearer: Option<String>,
        body: Option<serde_json::Value>,
    ) -> (reqwest::StatusCode, serde_json::Value) {
        let mut req = reqwest::Client::new()
            .post(format!("{}{path}", self.base))
            .header("Content-Type", "application/json");
        if let Some(b) = body {
            req = req.json(&b);
        }
        if let Some(b) = bearer {
            req = req.header("Authorization", format!("Bearer {b}"));
        }
        let resp = req.send().await.expect("http request");
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let json = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    /// Call by full name (`/control/call`).
    async fn call(&self, body: serde_json::Value) -> (reqwest::StatusCode, serde_json::Value) {
        self.post_to("/control/call", Some(self.token.clone()), Some(body))
            .await
    }

    /// Discovery list (`/control/commands`) — carries no body: answering an empty POST is this route's contract.
    async fn catalog(&self) -> (reqwest::StatusCode, serde_json::Value) {
        self.post_to("/control/commands", Some(self.token.clone()), None)
            .await
    }

    /// Builds the state where an attached client put its own name on — the dashboard's path for windows/tabs/slots. This owner only receives envelopes and does not answer.
    fn register_client_command(&self, conn_id: ConnId, name: &str, help: &str) {
        self.attach_owner(conn_id, name, help, Arc::new(DiscardingSink));
    }

    /// An owner that really answers the names put on — the return is the envelopes that owner received.
    fn register_answering_client(
        &self,
        conn_id: ConnId,
        name: &str,
        help: &str,
    ) -> Arc<Mutex<Vec<CommandEnvelope>>> {
        let seen: Arc<Mutex<Vec<CommandEnvelope>>> = Arc::default();
        self.attach_owner(
            conn_id,
            name,
            help,
            Arc::new(AnsweringSink {
                deliveries: self.bus.deliveries().clone(),
                seen: seen.clone(),
            }),
        );
        seen
    }

    fn attach_owner(&self, conn_id: ConnId, name: &str, help: &str, sink: Arc<dyn FrameSink>) {
        self.bus.roster().attach(conn_id, &sink);
        self.bus
            .roster()
            .register(
                conn_id,
                vec![engram_dashboard_command::CommandDecl {
                    name: name.to_string(),
                    help: help.to_string(),
                    agent_args: Vec::new(),
                    deadline: engram_dashboard_command::DeadlineClass::Ordinary,
                }],
            )
            .expect("registration of the attached connection");
    }

    /// An owner that answers later — the outcome attaches after the delivery has lain down in the reply slot ([`ParkedAnsweringSink`]).
    fn register_parked_answering_client(&self, conn_id: ConnId, name: &str, after: Duration) {
        self.attach_owner(
            conn_id,
            name,
            "{}",
            Arc::new(ParkedAnsweringSink {
                deliveries: self.bus.deliveries().clone(),
                after,
            }),
        );
    }

    /// Calls but demands the answer within the caller's budget, because a real client cuts off at it.
    ///
    /// The same number carries a different meaning, knowingly: the real CLI puts `CLI_CONTROL_READ_TIMEOUT_SECS` into `set_read_timeout` (a run of consecutive silence; canonical: that constant's doc on the agent side), while `reqwest`'s `.timeout()` here is the total budget. Today the route sends nothing before the answer,
    /// so they coincide, and the total being stricter cannot give a false green. That equivalence leans on a route property: if it starts streaming chunks or progress notices, the real CLI would not cut off but this test would — then fix this helper (measure a silence run), not the daemon deadline.
    async fn call_within(
        &self,
        budget: Duration,
        body: serde_json::Value,
    ) -> (reqwest::StatusCode, serde_json::Value) {
        let resp = reqwest::Client::builder()
            .timeout(budget)
            .build()
            .expect("client")
            .post(format!("{}/control/call", self.base))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&body)
            .send()
            .await
            .expect("the daemon's answer must arrive within the caller's budget");
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or_default())
    }

    /// Leaves no zombie PTY — a test that launched a real process must always call this at the end.
    fn shutdown_agents(&self) {
        self.manager.shutdown_all();
    }
}

async fn fixture(tag: &str) -> Fixture {
    fixture_with_table(tag, true).await
}

/// `with_table = false` reproduces an assembly where the table is not yet plugged in (the 503 contract); otherwise the table is plugged in as in production.
async fn fixture_with_table(tag: &str, with_table: bool) -> Fixture {
    fixture_assembled(tag, with_table, None).await
}

/// Builds the launch-kit source of a fixture from its profile registry (a race needs to change it mid-check).
type KitsOf = Box<dyn FnOnce(Arc<ProfileRegistry>) -> Arc<dyn LaunchKitSource>>;

/// `kits` = the manager's launch-kit source (`None` = the Noop default, today's launch).
async fn fixture_assembled(tag: &str, with_table: bool, kits: Option<KitsOf>) -> Fixture {
    let registry = Arc::new(ControlRegistry::new());
    let manager_slot = Arc::new(ManagerSlot::new());
    let broadcast = Arc::new(CountingBroadcast::default());
    let broadcast_slot = Arc::new(RosterBroadcastSlot::new());
    broadcast_slot.set(broadcast.clone());
    let command_slot = Arc::new(CommandTableSlot::new());
    // The fixture holds the one copy the assembly passed: if the server made its own, the discovery and relay tests below would turn red (in production that mismatch gives no signal).
    // The stage-1 table is attached as the slot, with the production assembly's shape (`lib.rs`), so this file really measures "the daemon's own name ends on the spot and only others' names are relayed".
    // The deadline watcher comes paired with the bus — without it a call meeting a non-answering owner would hang.
    let (bus, sweeper) = CommandBus::new(
        CommandRoster::new(),
        CommandDeliveries::new(),
        Arc::new(
            engram_dashboard_daemon::control::commands::DaemonLocalCommands::new(
                command_slot.clone(),
            ),
        ),
    );
    let handle = start_mcp_server_without_help(
        registry.clone(),
        manager_slot.clone(),
        Arc::new(MessagingSlot::new()),
        command_slot.clone(),
        bus.clone(),
    )
    .await
    .unwrap_or_else(|e| panic!("start mcp server({tag}): {e}"));

    let control: Arc<dyn ControlChannel> = Arc::new(NoopControlChannel);
    let profiles = Arc::new(ProfileRegistry::new(Arc::new(MemProfileStore::default())));
    let mut manager = AgentManager::new_with_control(
        Arc::new(NoopSink),
        profiles.clone(),
        Arc::new(SessionTracker::new(
            TrackerConfig {
                enabled: false,
                poll_interval: std::time::Duration::from_secs(1),
            },
            Arc::new(|_, _| {}),
        )),
        control,
    );
    if let Some(kits) = kits {
        manager = manager.with_launch_ports(LaunchPorts {
            kits: kits(profiles),
            ..LaunchPorts::noop()
        });
    }
    let manager = Arc::new(manager);
    manager_slot.set(manager.clone());
    if with_table {
        // This table is what the route actually runs (ADR-0155) — uses the same assembly function as the production assembly (`lib.rs`).
        command_slot.set(Arc::new(make_daemon_table(
            manager.clone(),
            broadcast_slot.clone(),
            Arc::new(NoInputLeases),
            Arc::new(NoUsageLimits),
            Arc::new(NoLibrary),
        )));
    }

    // Caller identity — control verbs use identity only for authorization (nothing derived from the sender), so any agent identity is valid; an id not in the roster also pins "control does not presuppose its own existence".
    let token = format!("test-token-{tag}");
    registry.issue(AgentId::new_v4(), 0, token.clone(), true);

    let base = handle
        .url
        .strip_suffix("/mcp")
        .expect("mcp url suffix")
        .to_string();
    Fixture {
        manager,
        base,
        token,
        registry,
        broadcast,
        bus,
        _sweeper: sweeper,
        _handle: handle,
    }
}

/// Registers a shell-backend agent in the roster — launching it for real needs no claude.
fn seed_shell_agent(manager: &Arc<AgentManager>, name: &str) -> AgentId {
    let cwd = std::env::temp_dir();
    let mut profile = AgentProfile::new(
        name.to_string(),
        AgentCommand::Shell {
            program: engram_dashboard_platform::shell::default_shell().to_string(),
            args: vec![],
        },
        cwd,
        vec![],
        false,
    );
    profile.display_name = Some(name.to_string());
    manager
        .try_create_agent(profile)
        .expect("registration succeeded")
        .id
}

fn agents_in(list: &serde_json::Value) -> Vec<(String, String)> {
    list["agents"]
        .as_array()
        .expect("agents array")
        .iter()
        .map(|a| {
            (
                a["name"].as_str().unwrap_or_default().to_string(),
                a["state"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

// ── auth ──────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_agent_route_refuses_calls_without_a_valid_token() {
    let f = fixture("auth").await;
    let list = serde_json::json!({ "verb": "list" });
    for bearer in [None, Some("not-a-real-token".to_string())] {
        let (status, _) = f.post_with(bearer.clone(), list.clone()).await;
        assert_eq!(
            status,
            reqwest::StatusCode::UNAUTHORIZED,
            "no token / an invalid token is 401 ({bearer:?})"
        );
    }
    let (status, body) = f.post(list).await;
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "a valid token passes: {body}"
    );
}

// ── happy path per verb ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_reports_both_live_and_sleeping_agents() {
    let f = fixture("list").await;
    seed_shell_agent(&f.manager, "sleepy");
    let live = seed_shell_agent(&f.manager, "awake");
    let profile = f.manager.agent_snapshot(live).expect("snapshot");
    f.manager
        .activate_profile(&profile, engram_dashboard_agent::profile::SpawnMode::Fresh)
        .expect("shell spawn");

    let (status, body) = f.post(serde_json::json!({ "verb": "list" })).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let mut rows = agents_in(&body);
    rows.sort();
    assert_eq!(
        rows,
        vec![
            ("awake".to_string(), "live".to_string()),
            ("sleepy".to_string(), "sleeping".to_string()),
        ],
        "the roster returns live and dormant ones together: {body}"
    );
    // A lookup does not change the roster, so there is no screen refresh either.
    assert_eq!(f.broadcast.count(), 0, "list does not notify");
    f.shutdown_agents();
}

#[tokio::test]
async fn new_registers_a_sleeping_agent_and_refreshes_the_clients() {
    let f = fixture("new").await;
    let cwd = std::env::temp_dir().to_string_lossy().to_string();
    let (status, body) = f
        .post(serde_json::json!({ "verb": "new", "cwd": cwd, "name": "fresh-one", "backend": "Claude" }))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    // The success body is flat — the return is declared per command so it is not wrapped one more level (ADR-0155).
    assert_eq!(body["name"], "fresh-one", "{body}");
    assert_eq!(body["state"], "sleeping", "{body}");
    assert!(
        body["agent_id"].as_str().is_some_and(|s| !s.is_empty()),
        "the id must be returned so a name collision can still be pinned down: {body}"
    );
    assert_eq!(
        f.broadcast.count(),
        1,
        "a roster change refreshes the clients"
    );

    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    assert_eq!(
        agents_in(&list),
        vec![("fresh-one".to_string(), "sleeping".to_string())]
    );
}

#[tokio::test]
async fn new_falls_back_to_the_folder_name_and_appends_a_number_when_it_is_taken() {
    let f = fixture("new-suffix").await;
    let dir = std::env::temp_dir().join(format!("engram-agent-route-{}", AgentId::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let base = dir
        .file_name()
        .and_then(|s| s.to_str())
        .expect("basename")
        .to_string();
    let cwd = dir.to_string_lossy().to_string();

    let (_, first) = f
        .post(serde_json::json!({ "verb": "new", "cwd": cwd, "backend": "Claude" }))
        .await;
    assert_eq!(
        first["name"], base,
        "with no name given it is the folder name: {first}"
    );
    let (_, second) = f
        .post(serde_json::json!({ "verb": "new", "cwd": cwd, "backend": "Claude" }))
        .await;
    // Returns the confirmed name, not the requested one (ADR-0120/0123) — otherwise it diverges from the screen and the address.
    assert_eq!(
        second["name"],
        format!("{base}(1)"),
        "on a collision a number is attached and the response carries that value: {second}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn spawn_wakes_an_agent_that_already_exists() {
    let f = fixture("spawn-wake").await;
    seed_shell_agent(&f.manager, "worker");

    let (status, body) = f
        .post(serde_json::json!({ "verb": "spawn", "target": "worker" }))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["name"], "worker", "{body}");
    assert_eq!(body["state"], "live", "{body}");
    assert_eq!(body["created"], false, "an existing one was woken: {body}");

    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    assert_eq!(
        agents_in(&list),
        vec![("worker".to_string(), "live".to_string())]
    );
    // Waking notifies too — the roster's composition stays the same but that entry's "last failure" changes (activation success = clear / failure = record, ADR-0172), and that axis flows only through the profile list, so the manager's live/dead transition (`agent_list_updated`) does not reach the screen.
    //   Without it an LLM wake does not show on screen while a human click (WS `SpawnProfile`) does, diverging the two handles (CLAUDE.md 「LLM-우선 제어」). // hangul-ok
    // ADR-0172
    assert_eq!(
        f.broadcast.count(),
        1,
        "the activation outcome changes the entry, so it notifies"
    );
    f.shutdown_agents();
}

/// The state a mutating verb reports and the state `list` reports cannot diverge — before, the mutating verb hard-coded `"live"`, so an agent that died right after waking could be live by `spawn` and dormant by `list`. The shell backend stays alive, so the comparison is deterministic.
#[tokio::test]
async fn the_state_a_mutation_reports_is_the_state_the_roster_reports() {
    let f = fixture("state-honesty").await;
    seed_shell_agent(&f.manager, "worker");

    let (_, spawned) = f
        .post(serde_json::json!({ "verb": "spawn", "target": "worker" }))
        .await;
    let claimed = spawned["state"].as_str().unwrap_or_default().to_string();
    assert_eq!(claimed, "live", "a live shell agent: {spawned}");

    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    let listed = agents_in(&list)
        .into_iter()
        .find(|(n, _)| n == "worker")
        .expect("worker row")
        .1;
    assert_eq!(
        claimed, listed,
        "two verbs of the same route must not give different answers about the same agent: {spawned} vs {list}"
    );
    f.shutdown_agents();
}

/// Create-and-start, checking strictly only what does not depend on the machine: one registration · one roster
/// announcement, after the activation (one `roster_changed` = one profile-list + agent-list frame pair) · the row in
/// the roster · an answer whose state is a word of the vocabulary. Whether the start succeeds depends on claude being
/// installed (this creates a claude agent), so it is not asserted here — the shell-backend tests above pin the start.
// ADR-0348
#[tokio::test]
async fn spawn_with_cwd_registers_once_and_announces_once_after_the_start() {
    let f = fixture("spawn-create").await;
    let cwd = std::env::temp_dir().to_string_lossy().to_string();
    let (status, body) = f
        .post(serde_json::json!({ "verb": "spawn", "cwd": cwd, "name": "newborn" }))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    // Once, after the activation, whether the start succeeded or not — the failure branch needs it most (the
    //   last failure this frame carries is all the screen shows then). No frame before the activation (TRD S25 step
    //   20 M3).
    // ADR-0172
    assert_eq!(
        f.broadcast.count(),
        1,
        "one announcement after the activation, whatever the start did: {body}"
    );

    match body["status"].as_str() {
        Some("error") => {
            // A startup failure is neither an argument problem nor a missing target — a failure with no argument to fix, so INTERNAL (TRD §4-⑦).
            assert_eq!(body["code"], "INTERNAL", "{body}");
            let hint = body["hint"].as_str().unwrap_or_default();
            assert!(
                hint.contains("newborn") && hint.contains("created"),
                "must say it was created but did not come up: {body}"
            );
        }
        _ => {
            assert_eq!(body["name"], "newborn", "{body}");
            assert_eq!(body["created"], true, "{body}");
            let state = body["state"].as_str().unwrap_or_default();
            assert!(
                state == "live" || state == "sleeping",
                "must not invent a value outside the status vocabulary: {body}"
            );
        }
    }

    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    let names: Vec<String> = agents_in(&list).into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        vec!["newborn".to_string()],
        "the registration remains: {list}"
    );
    f.shutdown_agents();
}

// ── targeting rules ─────────────────────────────────────────────────────────────────────

/// Confirms the miss while the target really exists: an earlier version renamed first and then asserted the miss, passing even with a case-insensitive / prefix-matching resolver (against an empty roster nothing matches anything).
#[tokio::test]
async fn near_miss_tokens_do_not_resolve_while_the_real_agent_is_still_there() {
    let f = fixture("targeting-exact").await;
    seed_shell_agent(&f.manager, "worker");

    for miss in ["work", "WORKER", "Worker", "worker ", " worker", "worker2"] {
        let (_, body) = f
            .post(serde_json::json!({ "verb": "rename", "target": miss, "name": "hijacked" }))
            .await;
        // If the targeting points at nobody it is NOT_FOUND — the code vocabulary does not diverge per family (TRD §4-⑦).
        assert_eq!(
            body["code"], "NOT_FOUND",
            "'{miss}' must not be resolved as 'worker': {body}"
        );
    }
    // Control group — the exact token goes through; without it the assertions above are indistinguishable from "the roster is empty".
    let (_, body) = f
        .post(
            serde_json::json!({ "verb": "rename", "target": "worker", "name": "renamed-exactly" }),
        )
        .await;
    assert_eq!(body["outcome"], "renamed", "{body}");
    assert_eq!(body["name"], "renamed-exactly", "{body}");
    // The missed calls changed nothing (only the one success was notified).
    assert_eq!(f.broadcast.count(), 1);
}

#[tokio::test]
async fn an_id_beats_a_name_that_looks_like_that_id() {
    let f = fixture("targeting-id").await;
    let real = seed_shell_agent(&f.manager, "worker");
    // An agent whose name is **another's id string** is also in the roster.
    let impostor = seed_shell_agent(&f.manager, &real.to_string());

    let (_, body) = f
        .post(serde_json::json!({ "verb": "rename", "target": real.to_string(), "name": "renamed-by-id" }))
        .await;
    assert_eq!(body["outcome"], "renamed", "{body}");
    assert_eq!(
        body["agent_id"],
        real.to_string(),
        "targeting by id goes to the agent with that id: {body}"
    );

    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    let impostor_row = list["agents"]
        .as_array()
        .expect("array")
        .iter()
        .find(|a| a["id"] == impostor.to_string())
        .expect("impostor row")
        .clone();
    assert_eq!(
        impostor_row["name"],
        real.to_string(),
        "the impostor agent was not touched: {list}"
    );
}

#[tokio::test]
async fn move_reparents_and_none_detaches_back_to_the_top_level() {
    let f = fixture("move").await;
    seed_shell_agent(&f.manager, "lead");
    let child = seed_shell_agent(&f.manager, "helper");

    let (status, body) = f
        .post(serde_json::json!({ "verb": "move", "target": "helper", "parent": "lead" }))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    assert_eq!(body["name"], "helper", "{body}");
    assert!(
        body["parent"].as_str().is_some(),
        "the new parent id: {body}"
    );
    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    let row = list["agents"]
        .as_array()
        .expect("array")
        .iter()
        .find(|a| a["name"] == "helper")
        .expect("helper row")
        .clone();
    assert_eq!(row["parent"], child_parent(&list, "lead"), "{list}");

    let (_, detached) = f
        .post(serde_json::json!({ "verb": "move", "target": "helper", "parent": null }))
        .await;
    assert!(detached["parent"].is_null(), "detach to root: {detached}");
    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    let row = list["agents"]
        .as_array()
        .expect("array")
        .iter()
        .find(|a| a["name"] == "helper")
        .expect("helper row")
        .clone();
    assert!(row["parent"].is_null(), "{list}");
    assert_eq!(
        f.broadcast.count(),
        2,
        "2 hierarchy changes = 2 screen refreshes"
    );
    let _ = child;
}

fn child_parent(list: &serde_json::Value, parent_name: &str) -> serde_json::Value {
    list["agents"]
        .as_array()
        .expect("array")
        .iter()
        .find(|a| a["name"] == parent_name)
        .expect("parent row")["id"]
        .clone()
}

#[tokio::test]
async fn move_refuses_an_impossible_parent_instead_of_silently_doing_nothing() {
    let f = fixture("move-reject").await;
    seed_shell_agent(&f.manager, "solo");
    let (status, body) = f
        .post(serde_json::json!({ "verb": "move", "target": "solo", "parent": "solo" }))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    // What the tree structure refused is not an argument error but a state conflict — CONFLICT (TRD §4-⑦).
    assert_eq!(body["code"], "CONFLICT", "{body}");
    assert_eq!(
        f.broadcast.count(),
        0,
        "a refusal does not trigger a screen refresh"
    );
}

// ── the four outcomes of rename ────────────────────────────────────────────────────────────────

/// The four outcomes must be distinguishable: confirmed rename · no change · target missing · name space exhausted.
///
/// Exhaustion (`RenameOutcome::Exhausted`) cannot be produced here — the manager yields it only when all of 1..=u32::MAX of that family is taken (4.2 billion roster entries). Its wire form is stated only by the code (`CONFLICT`); the other three are pinned.
#[tokio::test]
async fn rename_surfaces_each_of_its_four_outcomes_distinctly() {
    let f = fixture("rename").await;
    seed_shell_agent(&f.manager, "alpha");
    seed_shell_agent(&f.manager, "beta");

    // ① Confirmed rename — exactly the requested name.
    let (_, body) = f
        .post(serde_json::json!({ "verb": "rename", "target": "alpha", "name": "gamma" }))
        .await;
    assert_eq!(body["outcome"], "renamed", "{body}");
    assert_eq!(body["name"], "gamma", "{body}");

    // 2) Confirmed rename — the name collided and a number was attached. The requested name must not be returned (it diverges from the screen and address).
    let (_, body) = f
        .post(serde_json::json!({ "verb": "rename", "target": "gamma", "name": "beta" }))
        .await;
    assert_eq!(body["outcome"], "renamed", "{body}");
    assert_eq!(body["name"], "beta(1)", "the confirmed name: {body}");

    // 3) No change — it already holds a name of that family. A success, but a different fact from 1) and 2).
    let (_, body) = f
        .post(serde_json::json!({ "verb": "rename", "target": "beta(1)", "name": "beta" }))
        .await;
    assert_eq!(body["outcome"], "unchanged", "{body}");
    assert_eq!(body["name"], "beta(1)", "{body}");

    // ④ Target missing.
    let (_, body) = f
        .post(serde_json::json!({ "verb": "rename", "target": "ghost", "name": "whatever" }))
        .await;
    // The target is missing — NOT_FOUND (TRD §4-⑦).
    assert_eq!(body["code"], "NOT_FOUND", "{body}");
    assert!(
        body["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("exactly"),
        "must tell the exact-match rule: {body}"
    );
}

// ── targeting rules ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_duplicate_name_is_refused_rather_than_guessed() {
    let f = fixture("ambiguous").await;
    // Bypasses uniqueness (ADR-0120) to reproduce a state the normal path cannot make (2 same names).
    for _ in 0..2 {
        let mut p = AgentProfile::new(
            "twin".to_string(),
            AgentCommand::Shell {
                program: engram_dashboard_platform::shell::default_shell().to_string(),
                args: vec![],
            },
            std::env::temp_dir(),
            vec![],
            false,
        );
        p.display_name = Some("twin".to_string());
        f.manager.seed_agent_bypassing_uniqueness(p);
    }
    let (status, body) = f
        .post(serde_json::json!({ "verb": "rename", "target": "twin", "name": "x" }))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    // Two same names are a **different fact from absence** — absence is NOT_FOUND, this splits to CONFLICT (TRD §4-⑦).
    assert_eq!(body["code"], "CONFLICT", "{body}");
    assert!(
        body["hint"].as_str().unwrap_or_default().contains("id"),
        "must tell the escape hatch (targeting by id): {body}"
    );
    assert_eq!(f.broadcast.count(), 0, "if ambiguous, nothing is changed");
}

// ── argument rejection (daemon side) ─────────────────────────────────────────────────────────────

/// Even if the body breaks the JSON contract it answers with an envelope, not an empty 400: the caller is an LLM and the reason must be carried for self-correction (the `Json<…>` extractor would have given an empty 400 in all four cases).
#[tokio::test]
async fn a_body_that_is_not_a_command_object_still_gets_a_reason() {
    let f = fixture("bad-body").await;
    let client = reqwest::Client::new();
    for raw in [
        r#"{"verb": 5}"#,                   // type mismatch
        r#"{"verb":"list","verb":"move"}"#, // duplicate verb
        // A duplicate argument field is caught at the same place: taking the later value would turn an attach with `parent:"lead"` into a detach to root — no basis to pick either, so none is picked (ADR-0157).
        r#"{"verb":"move","target":"helper","parent":"lead","parent":null}"#,
        r#"["list"]"#,       // not an object
        r#"{"verb":"list""#, // broken JSON
        "",                  // empty body
    ] {
        let resp = client
            .post(format!("{}/control/agent", f.base))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", f.token))
            .body(raw)
            .send()
            .await
            .expect("http request");
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        assert_eq!(
            status,
            reqwest::StatusCode::OK,
            "answers with an envelope: {raw} → {text}"
        );
        let v: serde_json::Value = serde_json::from_str(&text).expect("response json");
        // If the body is not a command object it is an argument error — this entrance's codes go out in a single vocabulary (TRD §4-⑦).
        assert_eq!(v["code"], "INVALID_ARGUMENT", "{raw} → {text}");
        assert!(
            !v["hint"].as_str().unwrap_or_default().is_empty(),
            "with an empty reason there is no self-correction: {raw} → {text}"
        );
    }
    assert_eq!(f.broadcast.count(), 0);
}

#[tokio::test]
async fn the_daemon_refuses_malformed_verbs_on_its_own() {
    // What the CLI already blocks is looked at again by the daemon: there is no guarantee the CLI is the only caller of this route.
    let f = fixture("bad-args").await;
    // The two vocabularies diverge (TRD §4-②): not knowing the name needs the verb found again; knowing it but mismatching arguments needs only the arguments fixed.
    let cases: [(serde_json::Value, &str); 6] = [
        (serde_json::json!({}), "UNKNOWN_COMMAND"),
        (serde_json::json!({ "verb": "explode" }), "UNKNOWN_COMMAND"),
        (
            serde_json::json!({ "verb": "spawn", "target": "a", "cwd": "C:/x" }),
            "INVALID_ARGUMENT",
        ),
        (serde_json::json!({ "verb": "spawn" }), "INVALID_ARGUMENT"),
        (serde_json::json!({ "verb": "new" }), "INVALID_ARGUMENT"),
        (
            serde_json::json!({ "verb": "rename", "target": "a" }),
            "INVALID_ARGUMENT",
        ),
    ];
    for (body, want) in cases {
        let (status, resp) = f.post(body.clone()).await;
        assert_eq!(
            status,
            reqwest::StatusCode::OK,
            "a rejection is also 200 + JSON: {body}"
        );
        assert_eq!(resp["code"], want, "{body} → {resp}");
        assert!(
            resp["hint"].as_str().unwrap_or_default().contains("help"),
            "must tell the self-correction path: {resp}"
        );
    }
    assert_eq!(f.broadcast.count(), 0);
}

/// Absent · blank · explicit null are different requests — the caller may not be our CLI, so it is hit directly with a raw body (going through the CLI would let its defenses mask this decision).
#[tokio::test]
async fn move_distinguishes_an_absent_parent_a_blank_parent_and_an_explicit_detach() {
    let f = fixture("move-tristate").await;
    seed_shell_agent(&f.manager, "lead");
    seed_shell_agent(&f.manager, "helper");
    f.post(serde_json::json!({ "verb": "move", "target": "helper", "parent": "lead" }))
        .await;
    let under_lead = |list: &serde_json::Value| -> bool {
        list["agents"]
            .as_array()
            .expect("array")
            .iter()
            .find(|a| a["name"] == "helper")
            .expect("helper row")["parent"]
            .is_string()
    };
    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    assert!(under_lead(&list), "premise: helper is under lead: {list}");
    let after_setup = f.broadcast.count();

    // 1) Field absent — reading it as "no parent given so detach" would make one typo a hierarchy release.
    let (_, absent) = f
        .post(serde_json::json!({ "verb": "move", "target": "helper" }))
        .await;
    assert_eq!(absent["code"], "INVALID_ARGUMENT", "{absent}");
    // 2) Blank value — a shell's unset variable (`--parent "$UNSET"`) arrives in this shape.
    for blank in ["", "   "] {
        let (_, body) = f
            .post(serde_json::json!({ "verb": "move", "target": "helper", "parent": blank }))
            .await;
        assert_eq!(body["code"], "INVALID_ARGUMENT", "blank '{blank}': {body}");
    }
    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    assert!(
        under_lead(&list),
        "a rejected request must not have touched the hierarchy: {list}"
    );
    assert_eq!(
        f.broadcast.count(),
        after_setup,
        "a rejection does not notify"
    );

    // 3) Explicit null — only this is "detach to root".
    let (_, detached) = f
        .post(serde_json::json!({ "verb": "move", "target": "helper", "parent": null }))
        .await;
    assert!(detached["parent"].is_null(), "{detached}");
    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    assert!(!under_lead(&list), "now it is root: {list}");
}

/// Keeps one typo from becoming a different action — both unknown fields and fields the verb does not use are rejected.
#[tokio::test]
async fn unknown_and_irrelevant_fields_are_refused_instead_of_being_dropped() {
    let f = fixture("stray-fields").await;
    seed_shell_agent(&f.manager, "helper");
    seed_shell_agent(&f.manager, "lead");
    f.post(serde_json::json!({ "verb": "move", "target": "helper", "parent": "lead" }))
        .await;
    let before = f.broadcast.count();

    let cases: [(serde_json::Value, &str); 5] = [
        // The `parnet` typo — letting it through would read as "no parent" and release the hierarchy.
        (
            serde_json::json!({ "verb": "move", "target": "helper", "parnet": "lead" }),
            "parnet",
        ),
        (serde_json::json!({ "verb": "list", "wat": 1 }), "wat"),
        // A field the verb does not use — ignoring it would keep the caller from seeing that the argument vanished.
        (
            serde_json::json!({ "verb": "spawn", "target": "helper", "name": "x" }),
            "name",
        ),
        (
            serde_json::json!({ "verb": "list", "target": "helper" }),
            "target",
        ),
        (
            serde_json::json!({ "verb": "rename", "target": "helper", "name": "x", "cwd": "C:/x" }),
            "cwd",
        ),
    ];
    for (body, culprit) in cases {
        let (status, resp) = f.post(body.clone()).await;
        assert_eq!(
            status,
            reqwest::StatusCode::OK,
            "a rejection is also 200 + JSON: {body}"
        );
        assert_eq!(resp["code"], "INVALID_ARGUMENT", "{body} → {resp}");
        assert!(
            resp["hint"].as_str().unwrap_or_default().contains(culprit),
            "must point at which field is the problem ({culprit}): {resp}"
        );
    }
    assert_eq!(
        f.broadcast.count(),
        before,
        "a rejected request changes nothing"
    );
    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    let helper = list["agents"]
        .as_array()
        .expect("array")
        .iter()
        .find(|a| a["name"] == "helper")
        .expect("helper row")
        .clone();
    assert!(
        helper["parent"].is_string(),
        "the typo request must not have released the hierarchy: {list}"
    );
}

/// Also checks that the rejection list is derived from the declaration (ADR-0157): a hand-written allow-list would pass if it only pointed at the wrong field, but silently falls behind when verbs grow and lets unknown fields through. Carrying all declared fields shows the declaration is the source and lets the caller fix it itself.
#[tokio::test]
async fn an_unknown_field_is_refused_with_the_declared_argument_list() {
    let f = fixture("declared-args").await;
    seed_shell_agent(&f.manager, "helper");

    let (status, resp) = f
        .post(serde_json::json!({ "verb": "rename", "target": "helper", "nmae": "x" }))
        .await;
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "a rejection is also 200 + JSON: {resp}"
    );
    assert_eq!(resp["code"], "INVALID_ARGUMENT", "{resp}");
    let hint = resp["hint"].as_str().unwrap_or_default();
    assert!(
        hint.contains("nmae"),
        "must point at the wrong field: {resp}"
    );
    for declared in ["target", "name"] {
        assert!(
            hint.contains(declared),
            "all declared fields must be carried ({declared}): {resp}"
        );
    }
    assert_eq!(f.broadcast.count(), 0, "a rejection changes nothing");
}

/// An unknown verb is answered with an envelope — neither a missing route (404) nor the server blowing up (500).
///
/// Not knowing the name and mismatching the arguments are told apart by code (TRD §4-②).
#[tokio::test]
async fn an_unknown_verb_answers_with_the_error_envelope_not_a_404() {
    let f = fixture("unknown-verb").await;
    let (status, resp) = f.post(serde_json::json!({ "verb": "explode" })).await;

    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "answers with an envelope: {resp}"
    );
    assert_eq!(resp["status"], "error", "{resp}");
    assert_eq!(resp["code"], "UNKNOWN_COMMAND", "{resp}");
    let hint = resp["hint"].as_str().unwrap_or_default();
    assert!(hint.contains("explode"), "must point at which verb: {resp}");
    assert!(
        hint.contains("help"),
        "must tell the self-correction path: {resp}"
    );
}

/// An assembly where the table is not plugged in is 503 — the problem is the wiring order, not the request format or auth, so a 4xx would send the caller to fix arguments (nothing to fix). The status code carries that split.
#[tokio::test]
async fn the_route_reports_a_missing_command_table_as_unavailable() {
    let f = fixture_with_table("no-table", false).await;
    let (status, body) = f.post(serde_json::json!({ "verb": "list" })).await;
    assert_eq!(
        status,
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "table not set is 503: {body}"
    );
}

/// The refusal-reason list must correspond to the real refusal conditions: if "the side being moved is already a parent" (fires in a 2-level tree) is not in the wording, the refused caller cannot tell what to do next.
#[tokio::test]
async fn a_refused_move_names_the_rule_that_actually_fired() {
    let f = fixture("move-rules").await;
    seed_shell_agent(&f.manager, "a");
    seed_shell_agent(&f.manager, "b");
    seed_shell_agent(&f.manager, "c");
    // Put b under a — now a is a parent with a child.
    let (_, ok) = f
        .post(serde_json::json!({ "verb": "move", "target": "b", "parent": "a" }))
        .await;
    assert!(ok["parent"].is_string(), "{ok}");

    // Putting a, which has a child, under c = 3 levels so it is refused (the tree allows 1 level of nesting).
    let (_, refused) = f
        .post(serde_json::json!({ "verb": "move", "target": "a", "parent": "c" }))
        .await;
    assert_eq!(refused["code"], "CONFLICT", "{refused}");
    let hint = refused["hint"].as_str().unwrap_or_default();
    assert!(
        hint.contains("children"),
        "the condition that actually fired (the side being moved is already a parent) must be in the wording: {hint}"
    );
    // The case where the parent is already someone else's child must be answered by the same wording.
    let (_, refused) = f
        .post(serde_json::json!({ "verb": "move", "target": "c", "parent": "b" }))
        .await;
    assert_eq!(refused["code"], "CONFLICT", "{refused}");
    assert!(
        refused["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("top-level"),
        "the parent-eligibility condition must be in the wording too: {refused}"
    );
}

/// Runaway backstop (user decision) — a single count check. It blocks exactly at the boundary, and non-increasing verbs (lookup, rename) keep working after blocking, or recovery itself would be impossible.
///
/// The cap itself is enforced by the core (`AgentManager`, at the registration commit point inside the name assignment gate); checked here is only how this route translates that refusal: a code distinct from name space exhaustion and a hint stating its nature. That the cap applies to every entrance and does not leak under concurrent registration is the core unit tests'.
#[tokio::test]
async fn creating_agents_stops_at_the_runaway_ceiling() {
    let f = fixture("ceiling").await;
    let cwd = std::env::temp_dir().to_string_lossy().to_string();
    // Fill up to just below the cap (uniqueness-bypass seam — the normal path makes name assignment O(n²)).
    for i in 0..(MAX_ROSTER_SIZE - 1) {
        let mut p = AgentProfile::new(
            format!("filler-{i}"),
            AgentCommand::Shell {
                program: engram_dashboard_platform::shell::default_shell().to_string(),
                args: vec![],
            },
            std::env::temp_dir(),
            vec![],
            false,
        );
        p.display_name = Some(format!("filler-{i}"));
        f.manager.seed_agent_bypassing_uniqueness(p);
    }
    assert_eq!(
        f.manager.roster().len(),
        MAX_ROSTER_SIZE - 1,
        "fill premise"
    );

    // The last slot passes — also checks the boundary blocks at "greater or equal", not "less than".
    let (_, last) = f
        .post(serde_json::json!({ "verb": "new", "cwd": cwd, "name": "last-one", "backend": "Claude" }))
        .await;
    assert_eq!(last["name"], "last-one", "{last}");
    assert_eq!(f.manager.roster().len(), MAX_ROSTER_SIZE);

    for body in [
        serde_json::json!({ "verb": "new", "cwd": cwd, "name": "one-too-many", "backend": "Claude" }),
        serde_json::json!({ "verb": "spawn", "cwd": cwd, "name": "one-too-many" }),
    ] {
        let (status, resp) = f.post(body.clone()).await;
        assert_eq!(status, reqwest::StatusCode::OK);
        // The cap means "free up a slot", not "fix the arguments" — CONFLICT (TRD §4-⑦).
        assert_eq!(resp["code"], "CONFLICT", "{body} → {resp}");
        let hint = resp["hint"].as_str().unwrap_or_default();
        assert!(
            hint.contains("safety ceiling") && hint.contains("not a product limit"),
            "must state the nature of the cap (blocks the temptation to tune it): {hint}"
        );
    }
    assert_eq!(
        f.manager.roster().len(),
        MAX_ROSTER_SIZE,
        "it does not grow after blocking"
    );

    // Even at the cap non-increasing verbs keep working (the recovery path must stay alive).
    let (_, renamed) = f
        .post(serde_json::json!({ "verb": "rename", "target": "last-one", "name": "still-works" }))
        .await;
    assert_eq!(renamed["outcome"], "renamed", "{renamed}");
}

// ── discovery list and full-name call (`/control/commands` · `/control/call`) ────────────────

/// Every control-plane route sits behind the same middleware — auth is on the whole router so adding routes needs no separate wiring. The property is exercised for real (a per-handler check would leave this test the only net the day a new route forgets it).
#[tokio::test]
async fn the_catalog_routes_refuse_calls_without_a_valid_token() {
    let f = fixture("catalog-auth").await;
    let call_body = serde_json::json!({ "name": "agent.list" });

    for bearer in [None, Some("not-a-real-token".to_string())] {
        let (listing, _) = f.post_to("/control/commands", bearer.clone(), None).await;
        assert_eq!(
            listing,
            reqwest::StatusCode::UNAUTHORIZED,
            "discovery also requires a token ({bearer:?})"
        );
        let (invoke, _) = f
            .post_to("/control/call", bearer.clone(), Some(call_body.clone()))
            .await;
        assert_eq!(
            invoke,
            reqwest::StatusCode::UNAUTHORIZED,
            "a call also requires a token ({bearer:?})"
        );
    }

    let (listing, body) = f.catalog().await;
    assert_eq!(
        listing,
        reqwest::StatusCode::OK,
        "a valid token passes: {body}"
    );
    let (invoke, body) = f.call(call_body).await;
    assert_eq!(
        invoke,
        reqwest::StatusCode::OK,
        "a valid token passes: {body}"
    );
}

/// The list merges two sources — the daemon's own table and names put on by an attached client; if either is missing, an LLM asking learns something present is absent.
///
/// `help` must be byte for byte — also checks the daemon does not open a non-JSON value that was put on.
#[tokio::test]
async fn the_listing_carries_both_sources_with_the_help_bytes_intact() {
    let f = fixture("catalog-list").await;
    let opaque = "not json at all — arbitrary bytes {[(";
    f.register_client_command(1, "tab.create", opaque);

    let (status, body) = f.catalog().await;
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    let rows = body["commands"].as_array().expect("commands array").clone();
    let names: Vec<&str> = rows
        .iter()
        .map(|r| r["name"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        names,
        vec![
            "agent.cancelQueuedInput",
            "agent.interrupt",
            "agent.list",
            "agent.listQueuedInputs",
            "agent.move",
            "agent.new",
            "agent.rename",
            "agent.resolve",
            "agent.spawn",
            "library.folders",
            "library.list",
            "library.newFile",
            "library.newFolder",
            "library.read",
            "library.refresh",
            "library.relink",
            "library.setRoot",
            "library.status",
            "preset.add",
            "preset.check",
            "preset.get",
            "preset.previewFolder",
            "preset.remove",
            "preset.reorder",
            "preset.save",
            "preset.setBackend",
            "tab.create",
            "usage.get",
            "usage.refresh"
        ],
        "the two sources merge into one list in name order: {body}"
    );
    let client_row = rows
        .iter()
        .find(|r| r["name"] == "tab.create")
        .expect("client row");
    assert_eq!(
        client_row["help"], opaque,
        "help is neither opened nor trimmed: {body}"
    );
    // The daemon's own name's help is carried too — a names-only list cannot fit arguments and is not discovery.
    let own_row = rows
        .iter()
        .find(|r| r["name"] == "agent.rename")
        .expect("daemon row");
    assert!(
        own_row["help"]
            .as_str()
            .is_some_and(|h| h.contains("target")),
        "the daemon command's argument spec must be carried: {body}"
    );
    // Discovery is read-only — a screen refresh on pulling the list would make one poll keep shaking the tree.
    assert_eq!(f.broadcast.count(), 0, "discovery changes nothing");
}

/// The list's reachability field cannot diverge from what the call route does — call every listed name as is and compare; a "callable" field with an unreachable answer (`UNSUPPORTED`) turns this red.
///
/// Both sources are carried: the daemon's own name ends on the spot and others' names are relayed to the owner — both reachable since ADR-0160, confirmed through the route.
/// No arguments, which keeps the comparison side-effect free: a daemon command is refused for missing arguments and another's name is handed straight back by the answering owner.
#[tokio::test]
async fn the_listing_flag_predicts_what_the_call_route_actually_does() {
    let f = fixture("catalog-flag").await;
    f.register_answering_client(1, "tab.create", "{}");

    let (_, listing) = f.catalog().await;
    let rows = listing["commands"].as_array().expect("array").clone();
    assert!(
        rows.len() > 1,
        "premise: both sources are carried: {listing}"
    );
    assert!(
        rows.iter().any(|r| r["name"] == "tab.create"),
        "premise: another's name is carried too: {listing}"
    );

    for row in rows {
        let name = row["name"].as_str().expect("name").to_string();
        let callable = row["callable"]
            .as_bool()
            .expect("the reachability field must exist");
        let (status, answer) = f.call(serde_json::json!({ "name": name })).await;
        assert_eq!(status, reqwest::StatusCode::OK, "{name} → {answer}");
        // This one line is structurally true — not a regression catcher: the reachability field is built from `mine ∪ registered` and the row is the merge of those (`catalog::handle_list`), so a `false` row cannot be made today, and no negative control can be attached (the `UNKNOWN_COMMAND` axis below has one).
        //   Kept because the day a third source (a name known only by declaration, no owner) enters the merge this line first diverges on that row and is the only place to report it. Do not delete it, and do not read its green as "reachability computation verified".
        assert!(callable, "a name the list said is unreachable: {name}");
        // This assertion catches the call route not recognizing that name: the list merges two sources (`catalog::merge`) and the call looks up the same two again; a diverged name comes back as `UNKNOWN_COMMAND`. The negative control below shows it can really fail — without it this is indistinguishable from "asserting a constant".
        assert_ne!(
            answer["code"], "UNKNOWN_COMMAND",
            "the call route does not know a name the list taught: {name} → {answer}"
        );
        // The unreachable answer is blocked too — the named regression: before ADR-0160 this route returned `UNSUPPORTED` ("the owner is not me") for another's name; if revived, the assertion above would be green while the reachability field is false, so that code is denied separately.
        assert_ne!(
            answer["code"], "UNSUPPORTED",
            "an unreachable answer came for a name the list said is callable: {name} → {answer}"
        );
    }

    // Negative control — a name not in the list must be `UNKNOWN_COMMAND`; if red, the loop above is green whatever is put in and guards nothing.
    let (status, answer) = f
        .call(serde_json::json!({ "name": "nobody.declares.this" }))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{answer}");
    assert_eq!(
        answer["code"], "UNKNOWN_COMMAND",
        "a name not in the list must be an unknown command: {answer}"
    );
}

/// Even in the designed worst case, material to fix it must come back alive.
///
/// The argument check's rejection wording puts the caller's fields first and all declared fields after; its worst case is already capped by the tool crate — eight wrong fields (`MAX_NAMED_UNKNOWN`) each quoted up to 128 bytes (`MAX_QUOTED_INPUT_BYTES`), and showing all eight is the net's value.
/// That worst case is made through the real route to see the adapter's truncation does not eat the tail (what to fix it with). Do not lower it to a convenient size — the boundary would not be stepped on.
#[tokio::test]
async fn the_worst_designed_argument_refusal_still_carries_the_declared_fields() {
    let f = fixture("catalog-worst-args").await;
    seed_shell_agent(&f.manager, "helper");

    // Room so the quoted notation exceeds the cap — the form that even gets the truncation tag is the worst case.
    let mut args = serde_json::Map::new();
    for i in 0..8 {
        args.insert(format!("{i}{}", "n".repeat(200)), serde_json::json!(1));
    }
    args.insert("target".to_string(), serde_json::json!("helper"));

    let (status, resp) = f
        .call(serde_json::json!({ "name": "agent.rename", "args": args }))
        .await;

    assert_eq!(status, reqwest::StatusCode::OK, "{resp}");
    assert_eq!(resp["code"], "INVALID_ARGUMENT", "{resp}");
    let hint = resp["hint"].as_str().unwrap_or_default();
    // Premise — all eight counted and each name cut at the cap (evidence this is the worst case).
    assert_eq!(
        hint.matches("(truncated, input was").count(),
        8,
        "premise: the eight wrong fields must each have been cut at the cap: {hint}"
    );
    // Main point — the tail is alive.
    assert!(
        hint.contains("declared arguments"),
        "the declared field list was cut off whole: {hint}"
    );
    for declared in ["target", "name"] {
        assert!(
            hint.contains(declared),
            "the declared field ({declared}) is missing: {hint}"
        );
    }
    assert_eq!(f.broadcast.count(), 0, "a rejection changes nothing");
}

/// The body cap is on the whole router — exercises for real that adding routes needs no separate wiring (including the discovery route whose handler does not read the body).
#[tokio::test]
async fn the_catalog_routes_refuse_a_body_past_the_limit() {
    let f = fixture("catalog-413").await;
    let client = reqwest::Client::new();
    let over = "x".repeat(1024 * 1024);

    for path in ["/control/commands", "/control/call"] {
        let sent = client
            .post(format!("{}{path}", f.base))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", f.token))
            // If the cap does not apply this body creates one agent — the roster comparison below catches it.
            .body(format!(
                r#"{{"name":"agent.new","args":{{"cwd":"C:/","name":"past-the-limit","backend":"Claude"}},"pad":"{over}"}}"#
            ))
            .send()
            .await;

        // The two outcomes are a transport-layer fact (measured): the cap layer looks only at Content-Length, writes 413 and drops the connection; if the client is still uploading, Windows folds the socket into an RST and hyper raises `ConnectionAborted` (os error 10053) instead of a response.
        //   Either way the body never reached the handler; without the cap neither occurs and a 200 comes back.
        match sent {
            Ok(resp) => assert_eq!(
                resp.status(),
                reqwest::StatusCode::PAYLOAD_TOO_LARGE,
                "{path} accepted a body beyond the cap"
            ),
            Err(e) => assert!(
                e.is_request(),
                "it must have been cut while sending the request ({path}): {e:?}"
            ),
        }
    }

    // Status alone cannot tell "emitted 413 yet executed anyway".
    let (_, list) = f.post(serde_json::json!({ "verb": "list" })).await;
    assert_eq!(
        agents_in(&list),
        vec![],
        "a body beyond the cap was executed: {list}"
    );
    assert_eq!(f.broadcast.count(), 0);
}

/// Runs the same table as the sibling route — what is made here must show in the `/control/agent` roster for the two entrances to have operated the same daemon. Also checks a no-argument command need not carry `args`.
#[tokio::test]
async fn a_full_name_call_runs_a_daemon_owned_command() {
    let f = fixture("catalog-call").await;
    let cwd = std::env::temp_dir().to_string_lossy().to_string();

    let (status, body) = f
        .call(serde_json::json!({
            "name": "agent.new",
            "args": { "cwd": cwd, "name": "by-full-name", "backend": "Claude" }
        }))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    assert_eq!(body["name"], "by-full-name", "{body}");
    assert_eq!(body["state"], "sleeping", "{body}");
    assert_eq!(
        f.broadcast.count(),
        1,
        "a roster change refreshes the clients"
    );

    // `args` absent = no arguments.
    let (status, listed) = f.call(serde_json::json!({ "name": "agent.list" })).await;
    assert_eq!(status, reqwest::StatusCode::OK, "{listed}");
    assert_eq!(
        agents_in(&listed),
        vec![("by-full-name".to_string(), "sleeping".to_string())]
    );

    // The sibling route sees the same thing — evidence that the two entrances ran the same table.
    let (_, sibling) = f.post(serde_json::json!({ "verb": "list" })).await;
    assert_eq!(agents_in(&sibling), agents_in(&listed), "{sibling}");
}

/// The check must be the same function as the sibling's: if only one side checks, a typo blocked there passes through the other as a different action (ADR-0157). The same typo goes into both entrances and the same code and targeting must come out.
#[tokio::test]
async fn a_full_name_call_refuses_a_bad_argument_the_way_its_sibling_does() {
    let f = fixture("catalog-args").await;
    seed_shell_agent(&f.manager, "helper");

    let (_, sibling) = f
        .post(serde_json::json!({ "verb": "rename", "target": "helper", "nmae": "x" }))
        .await;
    let (status, mine) = f
        .call(serde_json::json!({
            "name": "agent.rename",
            "args": { "target": "helper", "nmae": "x" }
        }))
        .await;

    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "a rejection is also 200 + JSON: {mine}"
    );
    assert_eq!(mine["code"], sibling["code"], "{mine} vs {sibling}");
    assert_eq!(mine["code"], "INVALID_ARGUMENT", "{mine}");
    let hint = mine["hint"].as_str().unwrap_or_default();
    assert!(
        hint.contains("nmae"),
        "must point at the wrong field: {mine}"
    );
    for declared in ["target", "name"] {
        assert!(
            hint.contains(declared),
            "all declared fields must be carried ({declared}): {mine}"
        );
    }
    assert_eq!(f.broadcast.count(), 0, "a rejection changes nothing");
}

/// A name taught by discovery is really delivered to its owner and the answer comes back (ADR-0160 · completion condition 1).
///
/// The end-to-end path: HTTP request → daemon table miss → roster lookup → envelope to the owner's frame outlet → the owner's outcome → the same HTTP response; any broken link turns red. Arguments cross as is — another's name is neither checked nor touched.
#[tokio::test]
async fn a_client_owned_name_is_relayed_to_its_owner_and_its_answer_comes_back() {
    let f = fixture("catalog-relay").await;
    let seen = f.register_answering_client(1, "tab.create", "{}");
    let (_, listing) = f.catalog().await;
    assert!(
        listing["commands"]
            .as_array()
            .expect("array")
            .iter()
            .any(|r| r["name"] == "tab.create"),
        "premise: discovery taught that name: {listing}"
    );

    let (status, owned) = f
        .call(serde_json::json!({ "name": "tab.create", "args": { "window": "main" } }))
        .await;
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "answers with an envelope: {owned}"
    );
    assert_eq!(
        owned["ran"], "tab.create",
        "the result the owner made must be the response body as is: {owned}"
    );
    assert_eq!(owned["with"]["window"], "main", "{owned}");

    let envelopes = seen.lock().expect("poisoned");
    assert_eq!(envelopes.len(), 1, "one envelope");
    assert_eq!(envelopes[0].name, "tab.create");
    assert_eq!(
        envelopes[0].args["window"], "main",
        "arguments are not touched"
    );
    assert_eq!(
        envelopes[0].owner.as_str(),
        "conn-1",
        "the destination owner must be overwritten by the roster lookup result"
    );
}

/// A name nobody declared is still an "unknown command" (completion condition 2) — if this branch quietly changed to another code while attaching the relay, a caller with a typo could not find where to fix it.
#[tokio::test]
async fn a_name_nobody_declares_is_still_an_unknown_command() {
    let f = fixture("catalog-unknown").await;
    f.register_answering_client(1, "tab.create", "{}");

    let (status, nowhere) = f.call(serde_json::json!({ "name": "nope.nope" })).await;
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "answers with an envelope: {nowhere}"
    );
    assert_eq!(nowhere["code"], "UNKNOWN_COMMAND", "{nowhere}");
    let hint = nowhere["hint"].as_str().unwrap_or_default();
    assert!(
        hint.contains("nope.nope"),
        "must point at which name: {nowhere}"
    );
    assert!(
        hint.contains("/control/commands"),
        "the recovery path must be carried: {nowhere}"
    );
}

/// If the owner does not answer, the deadline answers — and before the caller cuts off (completion condition 3 · ADR-0161 decision 3).
///
/// The slowness is the substance: "the deadline really releases this route" runs the production deadline as is; pushing the clock by hand would measure the delivery unit and not whether the control route carries that answer as a response (that unit branch is `command_delivery`'s own tests').
/// Do not measure with a client without a time limit: it would be green however long it takes. It really was so, and with both deadlines at 10 s the CLI deterministically cut off first — the user saw "could not reach the daemon" instead of `TIMEOUT`. The cap below is the real caller's (`CLI_CONTROL_READ_TIMEOUT_SECS`); exceeding it makes `send` fail.
/// Red here may be runner load: this waits the daemon deadline (7 s) in real time against the client cap (10 s) — a 3 s margin — while the local regression runs `--test-threads=4` with suites launching real processes. Judge in order: 1) rerun and see if it reproduces 2) whether a transport error came instead of `TIMEOUT`
/// (then the deadline relation really flipped — ADR-0161 decision 3) 3) else load. Do not raise the values — the relation of the two numbers is what is measured.
#[tokio::test]
async fn an_owner_that_never_answers_ends_in_a_timeout_within_the_callers_budget() {
    let f = fixture("catalog-deadline").await;
    // An owner that does not answer — receives the envelope and gives no outcome.
    f.register_client_command(1, "tab.create", "{}");

    let (status, answer) = f
        .call_within(
            Duration::from_secs(CLI_CONTROL_READ_TIMEOUT_SECS),
            serde_json::json!({ "name": "tab.create", "args": { "window": "main" } }),
        )
        .await;

    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "answers with an envelope: {answer}"
    );
    assert_eq!(answer["code"], "TIMEOUT", "{answer}");
    assert!(
        !answer["hint"].as_str().unwrap_or_default().is_empty(),
        "the reason must be carried: {answer}"
    );
}

/// The branch where the answer wakes a lying-down delivery — a spot the sibling e2e test cannot measure.
///
/// The sibling's owner attaches the outcome inside the call stack that sends the envelope, so the answer exists before the delivery lies down in the reply slot; a production owner is another process and the answer necessarily comes later, and some things break only there: the reply outlet must stay alive past the round trip
/// (a roster entry reaped first by cancellation would leave the answer without a destination), and the `oneshot` receiver must not have dropped by then.
#[tokio::test]
async fn an_answer_that_arrives_while_the_delivery_is_parked_still_reaches_the_caller() {
    let f = fixture("catalog-parked").await;
    // The slack makes "the delivery has surely lain down", not a value competing with the deadline (far shorter).
    f.register_parked_answering_client(1, "tab.create", Duration::from_millis(250));

    let (status, answer) = f
        .call_within(
            Duration::from_secs(CLI_CONTROL_READ_TIMEOUT_SECS),
            serde_json::json!({ "name": "tab.create", "args": { "window": "main" } }),
        )
        .await;

    assert_eq!(status, reqwest::StatusCode::OK, "{answer}");
    assert_eq!(
        answer["late"], true,
        "the late-arriving outcome must be this call's response: {answer}"
    );
    assert_eq!(answer["ran"], "tab.create", "{answer}");
    assert_eq!(
        f.bus.deliveries().in_flight(),
        0,
        "once the round trip is over the slot is released"
    );
}

/// The number the caller gave is carried in the envelope as is (ADR-0161 decision 1) — a number changed midway would make a re-query a stranger, the slot table would lose the material to see the overlap, and the same operation would be applied twice.
///
/// Do not read the guarantee wider: this entrance catches only a re-query while the round trip is open; one after completion is applied again (canonical: `CallRequest::request_id` of `control::catalog` · pending). This measures only the number's preservation.
#[tokio::test]
async fn a_caller_supplied_request_id_rides_all_the_way_into_the_envelope() {
    let f = fixture("catalog-caller-id").await;
    let seen = f.register_answering_client(1, "tab.create", "{}");
    let mine = engram_dashboard_command::RequestId::new();

    let (status, answer) = f
        .call(serde_json::json!({
            "name": "tab.create",
            "args": { "window": "main" },
            "request_id": mine.to_string(),
        }))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{answer}");

    let envelopes = seen.lock().expect("poisoned");
    assert_eq!(envelopes.len(), 1, "one envelope: {envelopes:?}");
    assert_eq!(
        envelopes[0].request_id, mine,
        "the number the caller gave must go to the owner as is"
    );
}

/// The value comes from outside, so the grammar is validated — and the rejection points at that field, not "the whole body is wrong".
#[tokio::test]
async fn a_malformed_caller_request_id_is_refused_by_name() {
    let f = fixture("catalog-bad-caller-id").await;
    f.register_client_command(1, "tab.create", "{}");

    let (status, answer) = f
        .call(serde_json::json!({ "name": "tab.create", "request_id": "not-a-uuid" }))
        .await;

    assert_eq!(status, reqwest::StatusCode::OK, "{answer}");
    assert_eq!(answer["code"], "INVALID_ARGUMENT", "{answer}");
    assert!(
        answer["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("request_id"),
        "must say which field is wrong: {answer}"
    );
}

/// The mail axis is closed by name too — this route picks a command by the body's name, not the path, so the middleware's route classification does not reach here.
///
/// Today `mail.*` is in no source, so this measures whether the guardrail stands: whether that name enters the daemon table (planned) or the dashboard puts it on the roster, one name decides, so it stands that day. A mail-capable credential must not get this rejection on the same name — without that comparison this is green for "reject every request".
#[tokio::test]
async fn a_mail_name_is_refused_by_name_on_the_call_route() {
    let f = fixture("catalog-mail-name").await;
    let barred = "test-token-no-mail".to_string();
    f.registry
        .issue(AgentId::new_v4(), 0, barred.clone(), false);

    let body = serde_json::json!({ "name": "mail.send", "args": { "to": "someone" } });
    let (status, answer) = f
        .post_to("/control/call", Some(barred.clone()), Some(body.clone()))
        .await;
    assert_eq!(status, reqwest::StatusCode::OK, "{answer}");
    assert_eq!(answer["code"], "MAIL_NOT_ALLOWED", "{answer}");

    // A name outside the mail family passes as is with the same credential (answers as an unknown name).
    let (_, answer) = f
        .post_to(
            "/control/call",
            Some(barred),
            Some(serde_json::json!({ "name": "mailbox.open" })),
        )
        .await;
    assert_eq!(
        answer["code"], "UNKNOWN_COMMAND",
        "a name that merely shares a prefix must not be blocked: {answer}"
    );

    // A mail-capable credential does not get this rejection on that name — the decision really diverges on the credential axis.
    let (_, answer) = f.call(body).await;
    assert_eq!(
        answer["code"], "UNKNOWN_COMMAND",
        "a mail-capable key does not go through the name gate: {answer}"
    );
}

/// A body that is not a command-call object answers with an envelope, not an empty 400 — same discipline as the sibling route (the caller is an LLM and the reason must be carried).
#[tokio::test]
async fn a_call_body_that_is_not_a_command_object_still_gets_a_reason() {
    let f = fixture("catalog-bad-body").await;
    let client = reqwest::Client::new();
    for raw in [
        r#"{"name": 5}"#,
        r#"{"name":"agent.list","name":"agent.new"}"#,
        r#"{"name":"agent.move","args":{"parent":"lead","parent":null}}"#,
        r#"["agent.list"]"#,
        r#"{"name":"agent.list""#,
        "",
    ] {
        let resp = client
            .post(format!("{}/control/call", f.base))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", f.token))
            .body(raw)
            .send()
            .await
            .expect("http request");
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        assert_eq!(
            status,
            reqwest::StatusCode::OK,
            "answers with an envelope: {raw} → {text}"
        );
        let v: serde_json::Value = serde_json::from_str(&text).expect("response json");
        assert_eq!(v["code"], "INVALID_ARGUMENT", "{raw} → {text}");
        assert!(
            !v["hint"].as_str().unwrap_or_default().is_empty(),
            "with an empty reason there is no self-correction: {raw} → {text}"
        );
    }
    assert_eq!(f.broadcast.count(), 0);
}

/// In an assembly where the table is not plugged in the two routes differ: a call has no table to run so it is 503, while discovery returns the roster half as is — the answer the WS list gives in the same state, so a 503 would make two surfaces say different things about one state.
///
/// ## The price = the list's reachability field lies in this state
///
/// Discovery emits the roster's names as `callable: true` (reachability depends only on the roster — `catalog::merge`), but the call route is 503 before it sees the name (the table-slot check of `mcp_server::control_call_handler` precedes `handle_call`), so in this window a name the list taught gets "that door does not exist right now".
/// The assertion below pins that window: as prose only, the next edit would take "the list and execution always agree" as a premise (the sibling `the_listing_flag_predicts_what_the_call_route_actually_does` stands on it).
/// Not closed because closing means "no table = empty table, still run the roster branch", which makes a wiring-order fault quiet — now it shows loudly as one error! line and a 503. The assembled servers fill the slot before accepting connections (daemon `run()`), so production does not hit this window; changing it is a contract change and needs a decision first.
#[tokio::test]
async fn without_the_command_table_the_call_is_unavailable_but_discovery_still_answers() {
    let f = fixture_with_table("catalog-no-table", false).await;
    f.register_client_command(1, "tab.create", "{}");

    let (status, body) = f.call(serde_json::json!({ "name": "agent.list" })).await;
    assert_eq!(
        status,
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "table not set is 503: {body}"
    );

    let (status, listing) = f.catalog().await;
    assert_eq!(status, reqwest::StatusCode::OK, "{listing}");
    let rows = listing["commands"].as_array().expect("array");
    let names: Vec<&str> = rows
        .iter()
        .map(|r| r["name"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(names, vec!["tab.create"], "the roster half comes out as is");

    // A nail for a known mismatch — discovery says callable, and calling that name is 503.
    // This line itself is structurally true (a `false` reachability row cannot be made today — `catalog::handle_list`); what counts is the 503 right below — the pair nails down that a window where list and execution diverge really exists.
    assert_eq!(rows[0]["callable"], true, "{listing}");
    let (status, body) = f.call(serde_json::json!({ "name": "tab.create" })).await;
    assert_eq!(
        status,
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "in this window even a roster name is 503 — the price in the doc above: {body}"
    );
}

// ── targeting rules ↔ mail entrance cross-check ─────────────────────────────────────────────────

/// Checks the two entrances' targeting interpretation is the same by really running the mail entrance.
///
/// The comparison target is the mail kernel's `resolve_token` (private), so the same roster is attached to the public path (`MessagingService::handle_send`) where it is the only resolver and the outcome read: delivered = resolved to that recipient, `RECIPIENT_NOT_FOUND`/`RECIPIENT_AMBIGUOUS` = absent/ambiguous.
/// The control side calls the resolver the entrance actually runs: the `agent.*` verbs resolve through the single `agent::commands::resolve_in` (the table `/control/agent` calls), run directly here — a copy would make only the copy green.
/// Covered: the matching rules (exact match · id first · same-name refusal). Not covered: the resolvers' domains differ (mail sees only the live roster, control also dormant agents), so the fixture is all live and only rules are compared (aligning domains is not this slice's decision).
mod resolver_alignment {
    use super::*;
    use engram_dashboard_agent::commands::{resolve_in, AgentRosterRow};
    use engram_dashboard_command::ErrorCode;
    use engram_dashboard_messaging::envelope::{DeliveryObservation, Entrance, EnvelopeFormat};
    use engram_dashboard_messaging::service::{
        AddressingSources, ControlPlanePort, DeliveryPort, FailCode, InjectReceipt, LiveAgent,
        MessagingService, SendMeta, SendStatus,
    };
    use engram_dashboard_messaging::{PeerId, SenderIdentity};

    /// One fixture line = (id, name).
    type Fixture = Vec<(PeerId, String)>;

    struct FixturePort {
        roster: Fixture,
        injected: Mutex<Vec<PeerId>>,
    }

    impl DeliveryPort for FixturePort {
        fn inject(&self, to_id: PeerId, bytes: &[u8]) -> Result<InjectReceipt, String> {
            self.injected.lock().expect("poisoned").push(to_id);
            Ok(InjectReceipt {
                bytes_requested: bytes.len(),
                bytes_written: bytes.len(),
                msg_uuid: uuid::Uuid::new_v4(),
                epoch: 0,
            })
        }
        fn live_agents(&self) -> Vec<LiveAgent> {
            self.roster
                .iter()
                .map(|(id, name)| LiveAgent {
                    id: *id,
                    name: name.clone(),
                    epoch: 0,
                    turn_signal: false,
                })
                .collect()
        }
        fn addressing_sources(&self) -> AddressingSources {
            AddressingSources {
                roster: self.live_agents(),
                // Empty the dormant layer to align the two resolvers' domains (module comment above).
                dormant: vec![],
            }
        }
        fn is_agent_live(&self, id: PeerId) -> bool {
            self.roster.iter().any(|(rid, _)| *rid == id)
        }
    }

    struct FixtureControlPlane;
    impl ControlPlanePort for FixtureControlPlane {
        fn envelope_format(&self) -> EnvelopeFormat {
            EnvelopeFormat::default()
        }
        fn record_delivery(&self, _obs: DeliveryObservation) {}
    }

    /// The comparison vocabulary for matching the two entrances' outcomes — no crate's contract, lives only here; the resolvers' return types differ (a delivery row vs a typed error), so a common axis is needed.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Resolution {
        Found(PeerId),
        NotFound,
        Ambiguous,
    }

    /// How the mail entrance resolved that token — converted into the comparison vocabulary above.
    fn mail_resolution(fixture: &Fixture, token: &str) -> Resolution {
        let port = Arc::new(FixturePort {
            roster: fixture.clone(),
            injected: Mutex::new(vec![]),
        });
        let service = MessagingService::new(port.clone(), Arc::new(FixtureControlPlane));
        // The sender is an identity outside the roster — keeps its own targeting out of the result.
        let from = SenderIdentity {
            peer_id: uuid::Uuid::new_v4(),
            epoch: 0,
        };
        let rows = service
            .handle_send(
                &format!("m-{}", uuid::Uuid::new_v4().simple()),
                from,
                "cross-check-sender",
                &[token.to_string()],
                "body",
                Entrance::Cli,
                &SendMeta::default(),
            )
            .expect("only cases that are not a send-unit rejection are used as fixtures");
        let row = rows.first().expect("one row per recipient");
        match (row.status, row.code) {
            (SendStatus::Delivered, _) => {
                let injected = port.injected.lock().expect("poisoned");
                Resolution::Found(*injected.first().expect("the delivered recipient id"))
            }
            (SendStatus::Failed, Some(FailCode::RecipientAmbiguous)) => Resolution::Ambiguous,
            (SendStatus::Failed, Some(FailCode::RecipientNotFound)) => Resolution::NotFound,
            other => panic!("an outcome this fixture cannot produce: {other:?} ({token})"),
        }
    }

    /// How the control entrance resolved that token — runs the very function the table's verbs call. The outcome is read by typed code: absent = `NOT_FOUND`, two or more same names = `CONFLICT`.
    fn control_resolution(fixture: &Fixture, token: &str) -> Resolution {
        let roster: Vec<AgentRosterRow> = fixture
            .iter()
            .map(|(id, name)| AgentRosterRow {
                id: *id,
                canonical_name: name.clone(),
                live: None,
                cwd: String::new(),
                parent: None,
            })
            .collect();
        match resolve_in(&roster, token) {
            Ok(found) => Resolution::Found(found.id),
            Err(e) if e.code() == ErrorCode::NotFound => Resolution::NotFound,
            Err(e) if e.code() == ErrorCode::Conflict => Resolution::Ambiguous,
            Err(e) => panic!("an outcome this resolver cannot produce: {e:?} ({token})"),
        }
    }

    #[test]
    fn control_and_mail_resolve_the_same_tokens_the_same_way() {
        let alice = uuid::Uuid::new_v4();
        let bob = uuid::Uuid::new_v4();
        let twin_a = uuid::Uuid::new_v4();
        let twin_b = uuid::Uuid::new_v4();
        // A name that looks like a UUID: it carries bob's id as its name — it must not hijack targeting by id.
        let impostor = uuid::Uuid::new_v4();
        let fixture: Fixture = vec![
            (alice, "alice".to_string()),
            (bob, "bob".to_string()),
            (twin_a, "twin".to_string()),
            (twin_b, "twin".to_string()),
            (impostor, bob.to_string()),
        ];

        let tokens = vec![
            "alice".to_string(),  // exact name match
            bob.to_string(),      // exact id match (= also another's name)
            alice.to_string(),    // exact id match (no name collision)
            "twin".to_string(),   // 2 same names
            "nobody".to_string(), // absent
            "ALICE".to_string(),  // case — a lenient resolver would diverge here
            "ali".to_string(),    // prefix — if a unique-prefix match existed it would diverge here
            // id spelling — every spelling `Uuid` reads is an id (ADR-0309 · TRD S29 §4 B7).
            alice.to_string().to_uppercase(),
            alice.braced().to_string(),
            alice.simple().to_string(),
            alice.urn().to_string(),
            bob.to_string().to_uppercase(), // an id differing only in spelling from another's name (impostor) is still an id first
        ];

        let mut agreed = 0;
        for token in &tokens {
            let control = control_resolution(&fixture, token);
            let mail = mail_resolution(&fixture, token);
            assert_eq!(
                control, mail,
                "the targeting interpretation of the two entrances diverged (token={token}): control={control:?} mail={mail:?}"
            );
            agreed += 1;
        }
        assert_eq!(agreed, tokens.len(), "all tokens were compared");

        // The one mismatch left knowingly — token preprocessing: the mail entrance trims the token before comparing (rescuing the whitespace left after the CLI splits `--to a, b` on the comma), while this route has no splitting so it does not trim. Both sides are asserted so neither direction flips by accident — adding a trim to `resolve_in` or deleting mail's trim turns this red.
        let padded = " alice ";
        assert_eq!(
            control_resolution(&fixture, padded),
            Resolution::NotFound,
            "the control route promises exact match — it does not correct padding"
        );
        assert_eq!(
            mail_resolution(&fixture, padded),
            Resolution::Found(alice),
        "the mail entrance rescues the whitespace left by comma splitting (do not delete that trim)"
        );

        // Pins that the comparison did not pass vacuously — all three outcomes must really have occurred.
        assert_eq!(
            control_resolution(&fixture, "alice"),
            Resolution::Found(alice)
        );
        assert_eq!(
            control_resolution(&fixture, &bob.to_string()),
            Resolution::Found(bob),
            "an id beats a name that resembles a UUID"
        );
        assert_eq!(
            control_resolution(&fixture, &alice.braced().to_string()),
            Resolution::Found(alice),
            "an id that is not canonical in spelling was also resolved as an id"
        );
        assert_eq!(control_resolution(&fixture, "twin"), Resolution::Ambiguous);
        assert_eq!(control_resolution(&fixture, "nobody"), Resolution::NotFound);
    }
}

// ── Launch refusals on this route (TRD S25 step 16) ──

/// Blocks every launch with the reasons it holds.
struct BlockingKits(LaunchBlocked);

impl LaunchKitSource for BlockingKits {
    fn check(&self, _profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked> {
        Err(self.0.clone())
    }
    fn prepare(
        &self,
        plan: LaunchPlan,
        id: AgentId,
        epoch: u32,
    ) -> Result<LaunchKit, PrepareError> {
        NoopLaunchKits.prepare(plan, id, epoch)
    }
    fn release(&self, _id: AgentId, _epoch: u32) {}
}

/// Changes the agent's env in the roster during every check, so the launch inputs never settle — a race.
struct RacingKits(Arc<ProfileRegistry>);

impl LaunchKitSource for RacingKits {
    fn check(&self, profile: &AgentProfile) -> Result<LaunchPlan, LaunchBlocked> {
        let mut changed = profile.clone();
        changed
            .env
            .push(("ENGRAM_RACE".into(), uuid::Uuid::new_v4().to_string()));
        self.0.try_upsert(changed).expect("roster write");
        NoopLaunchKits.check(profile)
    }
    fn prepare(
        &self,
        plan: LaunchPlan,
        id: AgentId,
        epoch: u32,
    ) -> Result<LaunchKit, PrepareError> {
        NoopLaunchKits.prepare(plan, id, epoch)
    }
    fn release(&self, _id: AgentId, _epoch: u32) {}
}

/// A launch refused before any process is a `CONFLICT` whose message already says the next step (fix the listed
/// items / try again), so this route adds no roster tail — "see the roster" would send the caller looking for a
/// wrong target. A block carries its reasons in `args`; nothing starts either way.
#[tokio::test]
async fn a_refused_launch_is_a_conflict_without_a_roster_tail() {
    let blocked = LaunchBlocked {
        reasons: vec![BlockReason::Missing {
            item: "rules/a.md".into(),
            via: vec!["team.yaml".into()],
            reason: Reason {
                code: "not-found".into(),
                args: serde_json::json!({ "path": "rules/a.md" }),
                line: Some(3),
            },
        }],
    };
    let blocking = blocked.clone();
    let cases: [(&str, KitsOf, Option<serde_json::Value>, &str); 2] = [
        (
            "launch-blocked",
            Box::new(move |_| Arc::new(BlockingKits(blocking)) as Arc<dyn LaunchKitSource>),
            Some(serde_json::Value::Object(blocked.to_args())),
            "fix the listed items, then start it again.",
        ),
        (
            "launch-raced",
            Box::new(|profiles| Arc::new(RacingKits(profiles)) as Arc<dyn LaunchKitSource>),
            None,
            "try again.",
        ),
    ];
    for (tag, kits, args, ending) in cases {
        let f = fixture_assembled(tag, true, Some(kits)).await;
        seed_shell_agent(&f.manager, "worker");

        let (status, body) = f
            .post(serde_json::json!({ "verb": "spawn", "target": "worker" }))
            .await;
        assert_eq!(status, reqwest::StatusCode::OK, "{body}");
        assert_eq!(body["status"], "error", "{body}");
        assert_eq!(body["code"], "CONFLICT", "{body}");
        assert_eq!(body.get("args").cloned(), args, "{body}");
        let hint = body["hint"].as_str().unwrap_or_default();
        assert!(
            hint.starts_with("launch refused for agent 'worker'") && hint.ends_with(ending),
            "{body}"
        );
        assert!(!hint.contains("agent list"), "no roster tail: {body}");
        assert!(
            f.manager.list_agents().is_empty(),
            "nothing started: {body}"
        );
        f.shutdown_agents();
    }
}
