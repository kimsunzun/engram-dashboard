//! ADR-0133 decision 3 integration test — the gate that refuses mail by credential.
//!
//! ★This is the only enforcement point★: nothing on the agent side blocks mail — the CLI sends a sensible mail call as is. This test hits HTTP directly, a caller that bypasses the CLI, and it must still be refused.
//!
//! ★No claude needed · deterministic★: no real agent is launched. The mail routes leave the messaging slot empty so the handler gives 503 — that 503 proves "it passed the middleware" (a refusal is 200 + a bounce envelope, so they are distinguishable).

use std::sync::{Arc, Mutex};

use engram_dashboard_agent::backend::{accepts_mcp_config, uses_mail, writes_mcp_config_file};
use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::profile::{
    AgentCommand, AgentOutputFormat, AgentProfile, ProfileRegistry, ProfileStore, StoreError,
};
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, ControlChannelNeeds, NoopControlChannel,
    StatusSink,
};
use engram_dashboard_daemon::command_delivery::{BusSweeper, CommandBus};
use engram_dashboard_daemon::control::commands::{
    make_daemon_table, NoInputLeases, NoLibrary, NoUsageLimits,
};
use engram_dashboard_daemon::control::help::HelpSource;
use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server_with_help, start_mcp_server_without_help, CommandTableSlot, ManagerSlot,
    McpServerHandle, MessagingSlot, RosterBroadcastSlot,
};
use engram_dashboard_daemon::control::registry::ControlRegistry;
use engram_dashboard_daemon::control::DaemonControlChannel;

/// The code the daemon carries when it refuses mail. The daemon constant is crate-private, so it is written here as the wire value — a change to the string turns this test red on purpose (the code string is a wire contract the CLI reads).
const MAIL_NOT_ALLOWED: &str = "MAIL_NOT_ALLOWED";

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

struct Fixture {
    base: String,
    /// The credential an MCP-capable spawn receives (mail refused).
    mcp_token: String,
    /// The credential a non-MCP spawn receives (mail allowed).
    cli_token: String,
    /// The one copy the server actually reads — names to be relayed are planted through this handle.
    bus: CommandBus,
    /// ★Must be held★ — if dropped, the reaper empties and closes the seat table (`CommandDeliveries::drain`) and every later relay is bounced as 「shutting down」. This file was green in that state because its tests called only the daemon's own names and never went through the seat table — hence `a_relayed_name_still_reaches_its_owner_through_this_fixture` below.
    _relay_sweeper: BusSweeper,
    _handle: McpServerHandle,
    /// The help body folder injected into the server ([`KNOWN_HELP`]) — deleted when the fixture ends.
    help_dir: std::path::PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.help_dir);
    }
}

/// The help body the fixture injects — all six sections, so the source accepts it (the header before the first section is discarded).
///
/// ★The repository body file is not used★: expected values would change with every screen-text edit. With a known body, 「whichever credential, it gets this screen verbatim」 can be asserted directly.
const KNOWN_HELP: &str = "# test header — discarded\n\
## root\n{tool} index screen\n\
## mail\n{tool} mail screen\n\
## agent\n{tool} agent screen\n\
## window\n{tool} window screen\n\
## settings\n{tool} settings screen\n\
## library\n{tool} library screen\n";

/// A folder with [`KNOWN_HELP`] in the release shape (`<folder>\prompts\engram-help.md`).
fn known_help_dir(tag: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("engram-mail-gate-help-{tag}-{}", AgentId::new_v4()));
    let prompts = dir.join("prompts");
    std::fs::create_dir_all(&prompts).expect("prompts folder");
    std::fs::write(prompts.join("engram-help.md"), KNOWN_HELP).expect("help body");
    dir
}

/// The owner that actually answers the names it put up — the job the dashboard does for window · tab · slot commands.
struct AnsweringSink(engram_dashboard_daemon::command_delivery::CommandDeliveries);

impl engram_dashboard_daemon::conn_port::FrameSink for AnsweringSink {
    fn try_send(
        &self,
        frame: engram_dashboard_daemon::conn_port::Frame,
    ) -> Result<(), engram_dashboard_daemon::conn_port::FrameError> {
        let engram_dashboard_daemon::conn_port::Frame::Text(text) = frame else {
            return Ok(());
        };
        if let Ok(engram_dashboard_protocol::AgentEvent::CommandRequest { envelope }) =
            serde_json::from_str(&text)
        {
            // The envelope field is raw JSON (ADR-0272) — the receiving owner unpacks it.
            let envelope: engram_dashboard_command::CommandEnvelope =
                serde_json::from_str(envelope.get())
                    .expect("an envelope the daemon wrapped unpacks");
            self.0.complete(engram_dashboard_command::CommandReply::ok(
                envelope.request_id,
                serde_json::json!({ "ran": envelope.name }),
            ));
        }
        Ok(())
    }

    fn send(
        &self,
        frame: engram_dashboard_daemon::conn_port::Frame,
    ) -> futures_util::future::BoxFuture<
        '_,
        Result<(), engram_dashboard_daemon::conn_port::FrameError>,
    > {
        Box::pin(std::future::ready(self.try_send(frame)))
    }
}

impl Fixture {
    /// An attached client puts up its own names and answers.
    fn register_answering_owner(
        &self,
        conn_id: engram_dashboard_daemon::conn_port::ConnId,
        name: &str,
    ) {
        let sink: Arc<dyn engram_dashboard_daemon::conn_port::FrameSink> =
            Arc::new(AnsweringSink(self.bus.deliveries().clone()));
        self.bus.roster().attach(conn_id, &sink);
        self.bus
            .roster()
            .register(
                conn_id,
                vec![engram_dashboard_command::CommandDecl {
                    name: name.to_string(),
                    help: "{}".to_string(),
                    agent_args: Vec::new(),
                    deadline: engram_dashboard_command::DeadlineClass::Ordinary,
                }],
            )
            .expect("registration of the attached connection");
    }

    async fn post(&self, token: &str, route: &str, body: serde_json::Value) -> (u16, String) {
        let resp = reqwest::Client::new()
            .post(format!("{}{route}", self.base))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {token}"))
            .json(&body)
            .send()
            .await
            .expect("http request");
        let status = resp.status().as_u16();
        (status, resp.text().await.unwrap_or_default())
    }
}

/// A mail refusal — only when the response is 200 + a validated bounce envelope + that code.
fn is_mail_rejection(status: u16, body: &str) -> bool {
    if status != 200 {
        return false;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    v.get("status").and_then(|s| s.as_str()) == Some("error")
        && v.get("code").and_then(|c| c.as_str()) == Some(MAIL_NOT_ALLOWED)
}

async fn fixture(tag: &str) -> Fixture {
    let registry = Arc::new(ControlRegistry::new());
    let manager_slot = Arc::new(ManagerSlot::new());
    let broadcast_slot = Arc::new(RosterBroadcastSlot::new());
    let command_slot = Arc::new(CommandTableSlot::new());
    // ★The fixture must hold the reaper★ — tied to `_relay_sweeper` and returned from the function, the seat table closes at once (`CommandDeliveries::drain`) and every later relay becomes 「shutting down」.
    // ★Attach the stage-1 table to the same copy as the slot below — do not revert to `without_commands()`★: the decider would recognize `agent.*` but the bus would not, so control-route assertions would measure a path absent in production. Same adapter as the production assembly (`lib.rs`).
    let (relay_bus, relay_sweeper) = CommandBus::new(
        engram_dashboard_daemon::command_roster::CommandRoster::new(),
        engram_dashboard_daemon::command_delivery::CommandDeliveries::new(),
        Arc::new(
            engram_dashboard_daemon::control::commands::DaemonLocalCommands::new(
                command_slot.clone(),
            ),
        ),
    );
    let help_dir = known_help_dir(tag);
    let handle = start_mcp_server_with_help(
        registry.clone(),
        manager_slot.clone(),
        // ★Left empty★: the mail handler gives 503 so "it passed the gate" is distinguishable from a refusal.
        Arc::new(MessagingSlot::new()),
        command_slot.clone(),
        relay_bus.clone(),
        // ADR-0285: a known body on the same injection board as production — the screen is fixed wherever the test exe is.
        HelpSource::new(help_dir.clone()),
    )
    .await
    .unwrap_or_else(|e| panic!("start mcp server({tag}): {e}"));

    let control: Arc<dyn ControlChannel> = Arc::new(NoopControlChannel);
    let manager = Arc::new(AgentManager::new_with_control(
        Arc::new(NoopSink),
        Arc::new(ProfileRegistry::new(Arc::new(MemProfileStore::default()))),
        Arc::new(SessionTracker::new(
            TrackerConfig {
                enabled: false,
                poll_interval: std::time::Duration::from_secs(1),
            },
            Arc::new(|_, _| {}),
        )),
        control,
    ));
    manager_slot.set(manager.clone());
    // ★The control route exercises the table (ADR-0155)★: left empty it gives 503 and the 「control is open to everyone」 assertion would measure missing wiring instead of the gate.
    command_slot.set(Arc::new(make_daemon_table(
        manager,
        broadcast_slot,
        Arc::new(NoInputLeases),
        Arc::new(NoUsageLimits),
        Arc::new(NoLibrary),
    )));

    // ★The decision is planted by hand — this fixture alone is not enough★: if `provision` derived it wrongly, the planted value would stay the same and everything stay green. That axis is covered on the production path by `a_credential_minted_by_the_real_provision_path_is_refused_end_to_end`. Planting here isolates per-route gate behavior from the derivation.
    let mcp_token = format!("mcp-cred-{tag}");
    let cli_token = format!("cli-cred-{tag}");
    registry.issue(AgentId::new_v4(), 0, mcp_token.clone(), false);
    registry.issue(AgentId::new_v4(), 0, cli_token.clone(), true);

    let base = handle
        .url
        .strip_suffix("/mcp")
        .expect("mcp url suffix")
        .to_string();
    Fixture {
        base,
        mcp_token,
        cli_token,
        bus: relay_bus,
        _relay_sweeper: relay_sweeper,
        _handle: handle,
        help_dir,
    }
}

/// All mail routes — send and query split on the same axis (block one side and the ledger is read through the other).
const MAIL_ROUTES: [(&str, &str); 3] = [
    ("/control/send", r#"{"to":"bob","body":"hi"}"#),
    ("/control/messages", "{}"),
    ("/control/messages", r#"{"id":"m-1"}"#),
];

#[tokio::test]
async fn an_mcp_capable_credential_is_refused_on_every_mail_route() {
    let f = fixture("refuse").await;
    for (route, body) in MAIL_ROUTES {
        let payload: serde_json::Value = serde_json::from_str(body).expect("fixture body");
        let (status, text) = f.post(&f.mcp_token, route, payload).await;
        assert!(
            is_mail_rejection(status, &text),
            "{route} must be refused with this credential: {status} {text}"
        );
    }
}

#[tokio::test]
async fn a_non_mcp_credential_still_reaches_the_mail_handlers() {
    let f = fixture("allow").await;
    for (route, body) in MAIL_ROUTES {
        let payload: serde_json::Value = serde_json::from_str(body).expect("fixture body");
        let (status, text) = f.post(&f.cli_token, route, payload).await;
        assert!(
            !is_mail_rejection(status, &text),
            "{route} must not be refused with this credential: {status} {text}"
        );
        assert_eq!(
            status, 503,
            "proof it reached the handler — the messaging slot is empty, so 503 ({route}): {text}"
        );
    }
}

/// ★Control is open to everyone (ADR-0132 decision 5)★ — even a mail-blocked credential gets a normal response on the control routes. Without this, a regression widening the gate to the whole router passes silently.
#[tokio::test]
async fn the_same_credential_passes_on_the_agent_control_route() {
    let f = fixture("control").await;
    for token in [&f.mcp_token, &f.cli_token] {
        let (status, text) = f
            .post(token, "/control/agent", serde_json::json!({"verb":"list"}))
            .await;
        assert_eq!(status, 200, "the control route passes: {text}");
        let v: serde_json::Value = serde_json::from_str(&text).expect("JSON response");
        assert!(
            v.get("agents").is_some(),
            "must be a roster response: {text}"
        );
        assert!(!is_mail_rejection(status, &text));
    }
}

/// ★Discovery and full-name calls are control plane too★ — a credential that cannot write letters but cannot learn what it can call would leave an agent on that backend ignorant of its tools. That is why the two routes are classified 「not mail」; an assertion must exercise the real middleware, not just the classification function.
///
/// ★Not only "not a refusal"★: that stays green if the route disappears — also check the response is that route's real contract (discovery = the `commands` array, a call = the roster payload).
#[tokio::test]
async fn the_same_credential_passes_on_the_catalog_routes() {
    let f = fixture("catalog").await;
    for token in [&f.mcp_token, &f.cli_token] {
        let (status, text) = f
            .post(token, "/control/commands", serde_json::json!({}))
            .await;
        assert_eq!(status, 200, "discovery passes: {text}");
        assert!(!is_mail_rejection(status, &text));
        let v: serde_json::Value = serde_json::from_str(&text).expect("JSON response");
        assert!(
            v.get("commands")
                .and_then(|c| c.as_array())
                .is_some_and(|c| !c.is_empty()),
            "must be the discovery list: {text}"
        );

        let (status, text) = f
            .post(
                token,
                "/control/call",
                serde_json::json!({"name":"agent.list"}),
            )
            .await;
        assert_eq!(status, 200, "the full-name call passes: {text}");
        assert!(!is_mail_rejection(status, &text));
        let v: serde_json::Value = serde_json::from_str(&text).expect("JSON response");
        assert!(
            v.get("agents").is_some(),
            "must be a roster response: {text}"
        );
    }
}

/// ★The help screen is control plane too★ — a mail-blocked credential must read the mail screen to learn that channel's contract, and the screen is the same whoever asks (ADR-0220 decision 4). Classifying the route as mail would refuse the blocked credential; picking the screen by caller would split the two credentials' screens.
/// ★Not only "not a refusal"★: check that the injected body arrives verbatim — a missing route or a source not reaching the server diverges here.
// ADR-0285
#[tokio::test]
async fn both_credentials_read_the_same_help_screen() {
    use engram_dashboard_agent::types::{
        CLI_EXE_NAME, CLI_GROUP_MAIL, CLI_HELP_ROUTE, CLI_HELP_SCREEN_KEY, CLI_HELP_TOPIC_KEY,
    };
    let f = fixture("help").await;
    for token in [&f.mcp_token, &f.cli_token] {
        for (word, want) in [
            (None, format!("{CLI_EXE_NAME} index screen\n")),
            (
                Some(CLI_GROUP_MAIL),
                format!("{CLI_EXE_NAME} {CLI_GROUP_MAIL} screen\n"),
            ),
        ] {
            let (status, text) = f
                .post(
                    token,
                    CLI_HELP_ROUTE,
                    serde_json::json!({ CLI_HELP_TOPIC_KEY: word }),
                )
                .await;
            assert_eq!(status, 200, "help passes ({word:?}): {text}");
            assert!(!is_mail_rejection(status, &text), "{word:?}: {text}");
            let v: serde_json::Value = serde_json::from_str(&text).expect("JSON response");
            assert_eq!(
                v[CLI_HELP_SCREEN_KEY].as_str(),
                Some(want.as_str()),
                "the screen of the injected body must arrive verbatim ({word:?}): {text}"
            );
        }
    }
}

/// ★Pins that this fixture's seat table is open★
///
/// The siblings above call only `agent.list` (the daemon's own name), answered from the decider's own table, so they never go through the seat table — everything stayed green with a dropped reaper and a closed table. Only running someone else's name through the relay observes that axis.
/// ★Measures the fixture's health, not the gate★ — so it runs with one mail-open credential. A closed table returns `OUTCOME_UNKNOWN` (「shutting down」).
#[tokio::test]
async fn a_relayed_name_still_reaches_its_owner_through_this_fixture() {
    let f = fixture("relay-open").await;
    f.register_answering_owner(1, "tab.create");

    let (status, text) = f
        .post(
            &f.cli_token,
            "/control/call",
            serde_json::json!({ "name": "tab.create", "args": { "window": "main" } }),
        )
        .await;

    assert_eq!(status, 200, "answers with an envelope: {text}");
    let v: serde_json::Value = serde_json::from_str(&text).expect("JSON response");
    assert_eq!(
        v["ran"], "tab.create",
        "the outcome the owner gave must come back as is — if the seat table is closed, this is a bounce: {text}"
    );
}

/// The MCP route is not subject to this gate — using mail over MCP is exactly that credential's channel.
///
/// ★"Not a refusal" alone stays green if the route disappears★: so check it passed the gate and the route is really attached. rmcp's status for a request without a handshake wobbles across version bumps, so narrow to three things: ① not our refusal envelope ② not a missing route (404) ③ not an auth failure (401) — otherwise the request passed auth and the gate and reached rmcp.
#[tokio::test]
async fn the_mcp_route_is_not_touched_by_the_mail_gate() {
    let f = fixture("mcp").await;
    let (status, text) = f
        .post(
            &f.mcp_token,
            "/mcp",
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        )
        .await;
    assert!(
        !is_mail_rejection(status, &text),
        "the MCP route must not be caught by the mail gate: {status} {text}"
    );
    assert_ne!(
        status, 404,
        "the MCP route is not attached: {status} {text}"
    );
    assert_ne!(
        status, 401,
        "a valid token but cut off at auth: {status} {text}"
    );
    // Control group — same credential and server, the mail route is refused (only the path separates the two).
    let (mail_status, mail_text) = f
        .post(
            &f.mcp_token,
            "/control/send",
            serde_json::json!({"to":"bob","body":"hi"}),
        )
        .await;
    assert!(
        is_mail_rejection(mail_status, &mail_text),
        "control group: the same credential's mail route is refused: {mail_status} {mail_text}"
    );
}

/// ★`/control/...` outside the list is pinned in two directions★: ① a mail-blocked credential gets a refusal (a classification omission does not silently open an entrance — fail-closed) ② a mail-open credential gets 404 (the path is not hidden by the gate but not routed). Without ②, a later real route would stay green looking only at ①.
#[tokio::test]
async fn an_unlisted_control_path_is_refused_when_blocked_and_unrouted_otherwise() {
    let f = fixture("unlisted").await;
    for path in ["/control/whatever", "/control/send/extra", "/control"] {
        let (status, text) = f.post(&f.mcp_token, path, serde_json::json!({})).await;
        assert!(
            is_mail_rejection(status, &text),
            "a control path outside the list must be refused for the blocked credential ({path}): {status} {text}"
        );
        let (status, text) = f.post(&f.cli_token, path, serde_json::json!({})).await;
        assert_eq!(
            status, 404,
            "that path must not actually be routed ({path}): {text}"
        );
    }
    // Outside the control plane is not folded — still 404 for the blocked credential. `/controlfoo` and the like only look similar by name (segment boundary).
    for path in ["/nope", "/controlfoo", "/control-x"] {
        let (status, text) = f.post(&f.mcp_token, path, serde_json::json!({})).await;
        assert_eq!(
            status, 404,
            "the gate is not widened beyond the control namespace ({path}): {text}"
        );
    }
}

/// ★End-to-end with no hand-planted decision★: the fixtures above put the decision directly into `issue`, so a `provision` regression issuing every claude credential as mail-allowed would stay green. This test goes along the production path — the real `DaemonControlChannel::provision` derives the decision and stamps the registry, and the token is checked against a real HTTP mail request.
#[tokio::test]
async fn a_credential_minted_by_the_real_provision_path_is_refused_end_to_end() {
    let registry = Arc::new(ControlRegistry::new());
    let manager_slot = Arc::new(ManagerSlot::new());
    // ★Hold on to the reaper★ — if dropped, the seat table closes on the spot and every round trip after that is bounced.
    let (relay_bus, _relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server_without_help(
        registry.clone(),
        manager_slot.clone(),
        Arc::new(MessagingSlot::new()),
        Arc::new(CommandTableSlot::new()),
        relay_bus,
    )
    .await
    .expect("start mcp server");
    let base = handle
        .url
        .strip_suffix("/mcp")
        .expect("mcp url suffix")
        .to_string();

    let data_dir = std::env::temp_dir().join(format!("engram-mail-gate-e2e-{}", AgentId::new_v4()));
    let channel = DaemonControlChannel::new(
        registry.clone(),
        handle.url.clone(),
        engram_dashboard_daemon::control::mcp_config::McpDir::new(data_dir.join("mcp-config")),
        Some(std::path::PathBuf::from("C:/app/engram.exe")),
    );

    // Pass the claude backend's real capability as is — a literal here would make the decision a test copy again. Both axes are read from the backend dispatch (ADR-0133 — two inputs).
    let command = AgentCommand::Claude {
        extra_args: vec![],
        output_format: AgentOutputFormat::StreamJson,
    };
    let needs = ControlChannelNeeds {
        accepts_mcp_config: accepts_mcp_config(&command),
        writes_mcp_config_file: writes_mcp_config_file(&command),
        uses_mail: uses_mail(&command),
    };
    assert!(
        needs.accepts_mcp_config,
        "claude must be MCP-capable (premise)"
    );
    assert!(
        needs.uses_mail,
        "claude must be inside the mail plane (premise)"
    );
    let ep = channel
        .provision(AgentId::new_v4(), 0, needs)
        .expect("provision ok")
        .expect("endpoint");

    let resp = reqwest::Client::new()
        .post(format!("{base}/control/send"))
        .header("Authorization", format!("Bearer {}", ep.token))
        .json(&serde_json::json!({"to":"bob","body":"hi"}))
        .send()
        .await
        .expect("http request");
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    assert!(
        is_mail_rejection(status, &text),
        "a claude credential issued by the real provision must be refused on HTTP mail: {status} {text}"
    );
    // Also check the endpoint's record came from the same decision (one derivation point — ADR-0133 decision 2).
    assert!(
        !ep.mail_allowed,
        "the endpoint record must be disallowed too"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
}

/// ★A backend outside the mail plane gets no mail authorization even if it receives the control channel★ — reverts the state where only sending had opened as a side effect of `supports_control_channel`.
///
/// ★Two layers are measured together★: ① the credential (do the HTTP mail routes refuse it) ② what the endpoint
///   carries (the mail authorization and the send grants). Measuring one layer lets 「refuses, yet grants」 through —
///   that mismatch is the shape of the send freeze ADR-0099 measured.
#[tokio::test]
async fn a_backend_outside_the_mail_plane_gets_control_but_no_mail() {
    let registry = Arc::new(ControlRegistry::new());
    let manager_slot = Arc::new(ManagerSlot::new());
    let (relay_bus, _sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    let handle = start_mcp_server_without_help(
        registry.clone(),
        manager_slot.clone(),
        Arc::new(MessagingSlot::new()),
        Arc::new(CommandTableSlot::new()),
        relay_bus,
    )
    .await
    .expect("start mcp server");
    let base = handle
        .url
        .strip_suffix("/mcp")
        .expect("mcp url suffix")
        .to_string();

    let data_dir =
        std::env::temp_dir().join(format!("engram-mail-gate-nomail-{}", AgentId::new_v4()));
    let channel = DaemonControlChannel::new(
        registry.clone(),
        handle.url.clone(),
        engram_dashboard_daemon::control::mcp_config::McpDir::new(data_dir.join("mcp-config")),
        Some(std::path::PathBuf::from("C:/app/engram.exe")),
    );

    // ★The axis is set up synthetically — today no backend declares `uses_mail=false`★ (codex was the last; ADR-0209's MCP wiring made it true). Reading a real backend would measure a spawn inside the mail plane while only its name says outside.
    //   ★Do not delete this axis anyway★: the daemon's derivation and gate show together only in this combination, and
    //   the net must already stand when the next backend arrives with it.
    // ADR-0209
    let needs = ControlChannelNeeds {
        accepts_mcp_config: false,
        writes_mcp_config_file: false,
        uses_mail: false,
    };

    let ep = channel
        .provision(AgentId::new_v4(), 0, needs)
        .expect("the control channel must be issued (control verbs are open to everyone)")
        .expect("endpoint");

    // ① credential — refused on all mail routes.
    for (route, body) in MAIL_ROUTES {
        let payload: serde_json::Value = serde_json::from_str(body).expect("fixture body");
        let resp = reqwest::Client::new()
            .post(format!("{base}{route}"))
            .header("Authorization", format!("Bearer {}", ep.token))
            .json(&payload)
            .send()
            .await
            .expect("http request");
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        assert!(
            is_mail_rejection(status, &text),
            "{route} must be refused with this credential: {status} {text}"
        );
    }

    // ② endpoint — no mail · zero send grants. 「No authorization, and the record says so」.
    assert!(
        !ep.mail_allowed,
        "no authorization was given but the record says allowed"
    );
    assert!(
        ep.grants.is_empty(),
        "there is no send entrance but a grant was carried: {:?}",
        ep.grants
    );
    // ③ the control CLI entrance is received as is — this field closes mail only (ADR-0132 decision 5).
    assert!(ep.send_exe.is_some(), "the control CLI wiring must stay");
    assert!(!ep.token.is_empty(), "the control token must be issued");

    let _ = std::fs::remove_dir_all(&data_dir);
}

/// ★codex's mail goes out over MCP and the CLI mirror is closed★ — measures that ADR-0209's 「MCP is canonical, CLI is a mirror」 stands in the real declaration.
///
/// ★Reading the real backend declaration is the point — a literal makes the decision a test copy★:
///   the two fields this piece touched (`accepts_mcp_config` · `uses_mail`) must move **together** for both
///   assertions below to stand. Revert one and one of them quietly falls — revert `uses_mail` and the grant is gone
///   (the tool in hand with no grant); revert `accepts_mcp_config` and the CLI mirror is authorized open.
// ADR-0209
#[test]
fn codex_mails_over_mcp_and_the_cli_mirror_closes() {
    use engram_dashboard_agent::types::ToolGrant;
    use engram_dashboard_daemon::control::mcp_config::MCP_SERVER_NAME;
    use engram_dashboard_daemon::control::mcp_server::SEND_MESSAGE_TOOL;

    let data_dir =
        std::env::temp_dir().join(format!("engram-mail-gate-codex-{}", AgentId::new_v4()));
    let channel = DaemonControlChannel::new(
        Arc::new(ControlRegistry::new()),
        "http://127.0.0.1:1/mcp".to_string(),
        engram_dashboard_daemon::control::mcp_config::McpDir::new(data_dir.join("mcp-config")),
        Some(std::path::PathBuf::from("C:/app/engram.exe")),
    );
    let command = AgentCommand::Codex {
        extra_args: vec![],
        output_format: AgentOutputFormat::Terminal,
    };
    let needs = ControlChannelNeeds {
        accepts_mcp_config: accepts_mcp_config(&command),
        writes_mcp_config_file: writes_mcp_config_file(&command),
        uses_mail: uses_mail(&command),
    };
    assert!(needs.uses_mail, "premise: codex is inside the mail plane");
    assert!(
        needs.accepts_mcp_config,
        "premise: the daemon reads codex as the MCP mail branch"
    );

    let ep = channel
        .provision(AgentId::new_v4(), 0, needs)
        .expect("provision ok")
        .expect("endpoint");

    assert!(
        !ep.mail_allowed,
        "it is the MCP mail branch but the CLI mirror was authorized — both channels opened at once"
    );
    assert_eq!(
        ep.grants,
        vec![ToolGrant::Mcp {
            server: MCP_SERVER_NAME.to_string(),
            tool: SEND_MESSAGE_TOOL.to_string(),
        }],
        "the send entrance grant must be only MCP send_message"
    );
    let _ = std::fs::remove_dir_all(&data_dir);
}

/// ★Even without the `engram` executable, a spawn outside the mail plane is not cut off★ — the pair fail-closed guards
/// (the daemon-given CLI mail authorization `mail_allowed` · [Cli] grant ↔ the executable to call) does not exist for that spawn.
/// ★Neither side of that pair is priming★ — a non-MCP spawn learns nothing yet still gets the
/// authorization, so the pair and the verdict stand as they are (ADR-0209 · `control/mod.rs`). Cutting this branch too
/// would abort, in that install, every spawn of a backend that has nothing to do with mail.
#[test]
fn a_backend_outside_the_mail_plane_survives_a_missing_cli_binary() {
    let registry = Arc::new(ControlRegistry::new());
    let data_dir =
        std::env::temp_dir().join(format!("engram-mail-gate-nocli-{}", AgentId::new_v4()));
    let channel = DaemonControlChannel::new(
        registry,
        "http://127.0.0.1:1/mcp".to_string(),
        engram_dashboard_daemon::control::mcp_config::McpDir::new(data_dir.join("mcp-config")),
        None, // ← a daemon that could not find the CLI executable
    );

    let no_mail = ControlChannelNeeds {
        accepts_mcp_config: false,
        writes_mcp_config_file: false,
        uses_mail: false,
    };
    assert!(
        channel.provision(AgentId::new_v4(), 0, no_mail).is_ok(),
        "a spawn outside the mail plane must not be stopped by the missing CLI"
    );

    // The combination where the pair holds (uses mail but no CLI) is still cut off — that discipline stays.
    let cli_mail = ControlChannelNeeds {
        accepts_mcp_config: false,
        writes_mcp_config_file: false,
        uses_mail: true,
    };
    assert!(
        channel.provision(AgentId::new_v4(), 0, cli_mail).is_err(),
        "if it uses CLI mail but has no executable, it must be fail-closed"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
}
