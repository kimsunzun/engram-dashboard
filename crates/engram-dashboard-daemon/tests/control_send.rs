//! ADR-0086 step 2 integration test — dual-entrance A→B message send (the eg_send MCP tool + the /control/send HTTP route).
//!
//! The relay is observed with a live json agent really spawned: the input echo that write_input emits synchronously right after send_input succeeds is caught with an
//! OutputSink. It is published before the claude round trip, so it is deterministic regardless of claude latency or authentication.
//!
//! The real-claude axis does not run in CI (the `--skip` list of `ci.yml` — no claude on the runner), so locally an observation failure is a failure, not a pass: passing it
//! through would leave nowhere that checks this axis (user decision 2026-10-02 · T-46). Exception: `c1_park_then_spawn_auto_delivers` is `#[ignore]` (reason = that attribute).
//! `require_claude` cannot catch claude's absence — the spawn succeeds and the process dies later, so the assertions after it fail.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::persistence::{FilePresetStore, FileProfileStore};
use engram_dashboard_agent::preset::PresetRegistry;
use engram_dashboard_agent::profile::{
    AgentCommand, AgentOutputFormat, AgentProfile, ProfileRegistry, SpawnMode,
};
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, OutputEvent, OutputFrame, OutputPayload,
    OutputSink, SinkError, SinkId, StatusSink,
};
use engram_dashboard_base::testing::wait_until;

use engram_dashboard_daemon::control::mcp_server::{
    start_mcp_server_without_help, CommandTableSlot, ManagerSlot, McpServerHandle, MessagingSlot,
};
use engram_dashboard_daemon::control::registry::ControlRegistry;
use engram_dashboard_daemon::control::DaemonControlChannel;
use engram_dashboard_messaging::service::MessagingService;

struct NoopSink;
impl StatusSink for NoopSink {
    fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

/// Where the relay collector is tied to the process lifetime.
///
/// If dropped, the collector empties and closes the slot table on its way out (`CommandDeliveries::drain`), and every later relay on that server is refused as "shutting down" — tied as a local inside `wire()` that
/// happens the moment the function returns (this file was in that state for a long time; tests were green because they call only paths that skip the slot table).
/// No extra axis in the return tuple: this harness has over 20 call sites and none reads it, and it already reclaims at test end = process end (as the flush worker handle detach below).
static RELAY_SWEEPERS: Mutex<Vec<engram_dashboard_daemon::command_delivery::BusSweeper>> =
    Mutex::new(Vec::new());

fn hold_for_the_process(sweeper: engram_dashboard_daemon::command_delivery::BusSweeper) {
    RELAY_SWEEPERS
        .lock()
        .expect("relay sweepers poisoned")
        .push(sweeper);
}

struct EventCapture {
    id: SinkId,
    seen: Arc<Mutex<Vec<String>>>,
}
impl OutputSink for EventCapture {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        if let OutputPayload::Event(OutputEvent::Structured { json, .. }) = frame.payload {
            self.seen.lock().unwrap().push(json.clone());
        }
        Ok(())
    }
    fn sink_id(&self) -> SinkId {
        self.id
    }
}

/// Fails rather than skipping: cargo swallows stdout, so a skip leaves only "ok" and hides the unverified as green; local is the only place that verifies this axis (module header).
/// It catches only "the agent is not on the list" — claude's absence itself cannot be told apart.
fn require_claude(test: &str) -> ! {
    panic!(
        "[{test}]: the claude (stream-json) agent is not on the list (claude may be absent or authentication failed) — \
         this axis does not run in CI, so it is reported as a failure locally."
    );
}

/// Wiring that mirrors the assembly order of production `run()`.
async fn wire(
    tag: &str,
) -> (
    Arc<AgentManager>,
    Arc<ControlRegistry>,
    String,
    std::path::PathBuf,
    McpServerHandle,
    Arc<MessagingService>,
    Arc<engram_dashboard_messaging::busy::BusyPolicy>,
) {
    let registry = Arc::new(ControlRegistry::new());
    let slot = Arc::new(ManagerSlot::new());
    let messaging_slot = Arc::new(MessagingSlot::new());
    // The collector must be held — as a local the slot table closes the moment this function returns ([`hold_for_the_process`]).
    let (relay_bus, relay_sweeper) =
        engram_dashboard_daemon::command_delivery::CommandBus::without_commands();
    hold_for_the_process(relay_sweeper);
    let handle = start_mcp_server_without_help(
        registry.clone(),
        slot.clone(),
        messaging_slot.clone(),
        // This file calls no control verb — emptying the command table makes only that route 503.
        Arc::new(CommandTableSlot::new()),
        relay_bus,
    )
    .await
    .expect("start mcp server");
    let url = handle.url.clone();
    let data_dir = std::env::temp_dir().join(format!("engram-cli-{tag}-{}", AgentId::new_v4()));

    let control: Arc<dyn ControlChannel> = Arc::new(DaemonControlChannel::new(
        registry.clone(),
        url.clone(),
        engram_dashboard_daemon::control::mcp_config::McpDir::new(data_dir.join("mcp-config")),
        None, // send_exe: the relay tests need no CLI path (direct HTTP/MCP calls).
        // ADR-0092: the existing relay tests are priming-independent — Noop is byte-identical to today's behavior.
        Arc::new(engram_dashboard_daemon::control::priming::NoopPrimingProvider),
    ));

    let (flush_tx, flush_rx) =
        tokio::sync::mpsc::unbounded_channel::<engram_dashboard_daemon::messaging_host::FlushMsg>();
    let idle_coalescer = Arc::new(engram_dashboard_daemon::messaging_host::IdleCoalescer::new());
    let sink: Arc<dyn StatusSink> = Arc::new(
        engram_dashboard_daemon::messaging_host::MessagingFlushSink::new_test(
            Box::new(NoopSink),
            flush_tx.clone(),
            idle_coalescer.clone(),
        ),
    );
    let profiles = Arc::new(ProfileRegistry::new(Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-cli-prof-{tag}-{}", AgentId::new_v4())),
    ))));
    let presets = Arc::new(PresetRegistry::new(Arc::new(FilePresetStore::new(
        std::env::temp_dir().join(format!("engram-cli-preset-{tag}-{}", AgentId::new_v4())),
    ))));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = Arc::new(AgentManager::new_with_control(
        sink, profiles, presets, tracker, control,
    ));
    slot.set(manager.clone());
    let idle_notifier = Arc::new(
        engram_dashboard_daemon::messaging_host::ChannelIdleNotifier::new(
            flush_tx,
            idle_coalescer.clone(),
        ),
    );
    let busy = Arc::new(
        engram_dashboard_daemon::messaging_host::busy_gate_for_manager(
            manager.clone(),
            idle_notifier.clone(),
        ),
    );
    let messaging = Arc::new(
        engram_dashboard_daemon::messaging_host::messaging_for_manager_gated(
            manager.clone(),
            registry.clone(),
            busy.clone(),
        )
        .with_flush_trigger(idle_notifier),
    );
    messaging_slot.set(messaging.clone());
    // This harness detaches the worker handle (reclaimed at test end = process end; production shutdown is brought down by lib.rs with a belt).
    drop(engram_dashboard_daemon::messaging_host::spawn_flush_worker(
        flush_rx,
        engram_dashboard_daemon::messaging_host::FlushWiring {
            messaging: messaging_slot.clone(),
            idle: idle_coalescer,
        },
    ));

    let base = url.strip_suffix("/mcp").unwrap_or(&url).to_string();
    (manager, registry, base, data_dir, handle, messaging, busy)
}

async fn post_send(
    base: &str,
    bearer: Option<&str>,
    to: &str,
    body: &str,
) -> (reqwest::StatusCode, String) {
    let client = reqwest::Client::new();
    let mut req = client
        .post(format!("{base}/control/send"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "to": to, "body": body }));
    if let Some(b) = bearer {
        req = req.header("Authorization", format!("Bearer {b}"));
    }
    let resp = req.send().await.expect("http request");
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

fn spawn_json_agent(
    manager: &Arc<AgentManager>,
    registry: &Arc<ControlRegistry>,
    name: &str,
) -> Option<(AgentInfo, String)> {
    let mut profile = AgentProfile::new(
        name.to_string(),
        AgentCommand::Claude {
            extra_args: vec![],
            output_format: AgentOutputFormat::StreamJson,
        },
        std::path::PathBuf::from("."),
        vec![],
        false,
    );
    // ADR-0101 (WYSIWYA): the canonical name used by routing/roster is display_name ?? cwd basename. The test targets by `name`, so it is planted in display_name to make "visible name = address" hold (the basename of cwd="." is ".").
    profile.display_name = Some(name.to_string());
    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .ok()?
        .into_started()?;
    if !wait_until(Duration::from_secs(5), || {
        manager.list_agents().iter().any(|a| a.id == info.id)
    }) {
        return None;
    }
    // The registry has no API to look up an issued token, so the token provision issued for this (id, epoch) cannot be taken out — a sender token is issued separately and planted (relay is the same as long as the sender identity matches).
    let token = format!("sender-tok-{}", info.id);
    registry.issue(info.id, info.epoch, token.clone(), true);
    Some((info, token))
}

// ── /control/send: auth ────────────────────────────────────────────────────────────
#[tokio::test]
async fn control_send_missing_token_is_401() {
    let (_m, _r, base, data_dir, handle, _messaging, _busy) = wire("auth-missing").await;
    let (status, _body) = post_send(&base, None, "bob", "hi").await;
    assert_eq!(
        status,
        reqwest::StatusCode::UNAUTHORIZED,
        "no token /control/send → 401"
    );
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

#[tokio::test]
async fn control_send_wrong_token_is_401() {
    let (_m, _r, base, data_dir, handle, _messaging, _busy) = wire("auth-wrong").await;
    let (status, _body) = post_send(&base, Some("bogus-token"), "bob", "hi").await;
    assert_eq!(
        status,
        reqwest::StatusCode::UNAUTHORIZED,
        "unknown token /control/send → 401"
    );
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── /control/send: corrective errors (no recipient · group · oversize) — a valid token is needed ────────────────────────
#[tokio::test]
async fn control_send_corrective_errors() {
    let (_m, registry, base, data_dir, handle, _messaging, _busy) = wire("corrective").await;
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "valid-sender".to_string(), true);

    let (status, body) = post_send(&base, Some("valid-sender"), "nobody", "hi").await;
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "acceptance is also 200 + JSON"
    );
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert!(
        v.get("status").is_none(),
        "a success response has no top-level status (spec §6): {body}"
    );
    assert_eq!(
        v["results"][0]["status"], "failed",
        "★ADR-0111 decision 1★ a nonexistent recipient is an **entrance refusal = a per-recipient failure row** (the old absent-parking is abolished): {body}"
    );
    assert!(
        v["results"][0]["hint"].is_string(),
        "a failure row must carry code + hint (spec §6)"
    );
    assert_eq!(v["results"][0]["code"], "RECIPIENT_NOT_FOUND", "{body}");

    // An `@` address error = an address-space error → the whole send unit is refused (ADR-0114 decision 3). User-defined groups were removed (ADR-0111 decision 4), so every `@name` other than `@all` is `GROUP_NOT_FOUND` — a different layer from a missing name (a row failure): the sender must fix the address and resend.
    let (_s, body) = post_send(&base, Some("valid-sender"), "@team", "hi").await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["code"], "GROUP_NOT_FOUND",
        "an unregistered @ address: {body}"
    );

    let big = "x".repeat(64 * 1024 + 1);
    let (_s, body) = post_send(&base, Some("valid-sender"), "nobody", &big).await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["code"], "BODY_TOO_LARGE", "an oversize body: {body}");

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── /control/send: a live shell session is not a recipient (user decision 2026-08-17) ─────────────────────────
// An envelope reaching a shell is run as a command, not read; the body is LLM free text, so `&`, `|` or `;` would parse as a separate command. So it is removed from the list wholesale instead of dropping the submit bytes.
// Not a regression to ADR-0116 decision 1 (that rejected "cannot deliver because there is no turn signal"; a terminal claude still receives): this is the "what the input is interpreted as" axis. The seal of the capability ≠ membership axis is
//   `roster_includes_a_terminal_agent_without_a_turn_signal_no_claude`.
// Omitting the gate is not verified here (mutation-measured): all defenses are in the kernel (`messaging/src/service.rs`) — busy half = `a_live_agent_without_a_turn_signal_is_injected_with_no_gate`, queue backlog half = `inject_failure_parks_pending_without_a_turn_signal`.
#[tokio::test]
async fn control_send_shell_recipient_is_not_a_mail_recipient() {
    let (manager, registry, base, data_dir, handle, messaging, _busy) =
        wire("no-turn-signal").await;
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "valid-sender".to_string(), true);

    let mut profile = AgentProfile::new(
        "sheller".to_string(),
        AgentCommand::Shell {
            program: engram_dashboard_platform::shell::default_shell().to_string(),
            args: vec![],
        },
        std::path::PathBuf::from("."),
        vec![],
        false,
    );
    profile.display_name = Some("sheller".to_string());
    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("shell spawn")
        .into_started()
        .expect("this call really launches (not a duplicate request)");
    assert!(wait_until(Duration::from_secs(3), || manager
        .list_agents()
        .iter()
        .any(|a| a.id == info.id)));

    let (_s, body) = post_send(&base, Some("valid-sender"), "sheller", "hi").await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["results"][0]["status"], "failed",
        "even a live shell is not a recipient of the letter: {body}"
    );
    assert_eq!(
        v["results"][0]["code"], "RECIPIENT_NOT_FOUND",
        "the same outcome as a name not on the list — the sender fixes the address and sends again: {body}"
    );
    assert_eq!(
        messaging.parked_total_for_test(),
        0,
        "it must not be parked either (letters would pile up for a name that never wakes): {body}"
    );

    manager.kill_agent(info.id).ok();
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── roster predicate seal: a terminal-state session leaves the roster even if it stays in the map — ADR-0116 decision 1 ──────
// Needed (mutation-measured, review fix D9-a): deleting the status condition of `messaging_host::is_live` left all 412 daemon tests green. With the 4th amendment that condition is the only membership gate (capability moved to the timing axis), so it is sealed at the real adapter level.
#[tokio::test]
async fn roster_excludes_a_terminal_session_still_in_the_map() {
    use engram_dashboard_messaging::service::DeliveryPort;

    let (manager, _registry, _base, data_dir, handle, messaging, _busy) = wire("dead-roster").await;
    let dead = obs_seam::insert_terminal_seam_recipient(&manager, "corpse");
    let port = engram_dashboard_daemon::messaging_host::ManagerDeliveryPort::new(manager.clone());

    assert!(
        manager.list_agents().iter().any(|a| a.id == dead),
        "the injected session must stay in the map (the premise of this test)"
    );
    assert!(
        !matches!(
            manager
                .list_agents()
                .into_iter()
                .find(|a| a.id == dead)
                .expect("it is in the map")
                .status,
            AgentStatus::Running | AgentStatus::Exiting
        ),
        "the state must be terminal (the premise of this test)"
    );

    assert!(
        !port.live_agents().iter().any(|a| a.id == dead),
        "★D9-a★ a terminal session got mixed into the roster (the status predicate was deleted)"
    );
    let sources = port.addressing_sources();
    assert!(
        !sources.roster.iter().any(|a| a.id == dead),
        "the entry-decision source must use the same predicate: {sources:?}"
    );
    assert!(
        !port.is_agent_live(dead),
        "the delete-cleanup gate must use the same predicate too (if a corpse is seen as live the cleanup never runs)"
    );
    let rows = messaging
        .handle_send(
            "m-dead",
            engram_dashboard_messaging::SenderIdentity {
                peer_id: AgentId::new_v4(),
                epoch: 0,
            },
            "outsider",
            &["corpse".to_string()],
            "hi",
            engram_dashboard_messaging::envelope::Entrance::Cli,
            &engram_dashboard_messaging::service::SendMeta::default(),
        )
        .expect("row response");
    assert_eq!(
        rows[0].code,
        Some(engram_dashboard_messaging::service::FailCode::RecipientNotFound),
        "a corpse is not a recipient: {rows:?}"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── relay happy path: sending to a json agent shows the wrapped line as a stdin input echo ────────────────
#[tokio::test]
async fn control_send_relays_wrapped_line_to_json_agent() {
    let (manager, registry, base, data_dir, handle, _messaging, _busy) = wire("relay").await;

    let Some((b_info, _b_tok)) = spawn_json_agent(&manager, &registry, "bee") else {
        require_claude("control_send_relays_wrapped_line_to_json_agent");
    };

    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = Arc::new(EventCapture {
        id: SinkId::new_v4(),
        seen: seen.clone(),
    });
    manager.subscribe(b_info.id, sink).expect("subscribe B");

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "relay-sender".to_string(), true);

    let (status, body) = post_send(&base, Some("relay-sender"), "bee", "ping-body-XYZ").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).expect("json ACK");
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "delivery succeeded: {body}"
    );
    assert_eq!(
        v["results"][0]["to"], "bee",
        "the resolved recipient name is attached"
    );
    assert!(v["id"].is_string(), "msg-id is attached");

    // Exact-match assertion (no loose substring — review finding): the `j` the observation sink catches is the whole JSON of the user echo (`{"type":"text","text":"<envelope>","uuid":"X"}`). A substring `contains` let truncation or corruption appended after `</message>` pass, so `j` is parsed, the `text` field extracted and compared
    //   exactly with the expected envelope — framing is outside the field, only the envelope's own integrity is looked at.
    let sender_display = &sender.to_string()[..8];
    let expected_envelope = obs_seam::expected_default_envelope(sender_display, "ping-body-XYZ");
    let observed = wait_until(Duration::from_secs(3), || {
        seen.lock().unwrap().iter().any(|j| {
            serde_json::from_str::<serde_json::Value>(j)
                .ok()
                .and_then(|v| v["text"].as_str().map(|t| t == expected_envelope))
                .unwrap_or(false)
        })
    });
    assert!(
        observed,
        "the relay injected the wrapped line into B's stdin (the text field of the input echo = an exact match of the expected envelope): expect={:?} seen={:?}",
        expected_envelope,
        seen.lock().unwrap()
    );

    manager.kill_agent(b_info.id).ok();
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── a message is delivered even from a discarded sender (liveness is not a gate · a record-only observation) — isolated by calling handle_send directly ──
// User decision 2026-07-19: a message's validity holds by authentication at composition time (entrance auth); a later death or rotation (token revoked) does not invalidate it — "send the result and exit" (last-words) is a core multi-agent pattern and consistent with future mailbox commit semantics.
//   is_identity_live does not block delivery and only leaves a forensic log.
// handle_send directly, not HTTP: the middleware (bearer_auth) validates first and blocks a revoke with 401 before the commit point, and a mid-flight injection between revoke and send cannot be made deterministic in one synchronous request. So: make the sender
//   live, revoke just before the relay, call handle_send → observe delivered. A reachable recipient needs a json claude spawn (fails if claude is absent).
#[tokio::test]
async fn control_send_revoked_sender_still_delivers_observation() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("revoked-delivers").await;

    let Some((b_info, _b_tok)) = spawn_json_agent(&manager, &registry, "target-b") else {
        require_claude("control_send_revoked_sender_still_delivers_observation");
    };

    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = Arc::new(EventCapture {
        id: SinkId::new_v4(),
        seen: seen.clone(),
    });
    manager.subscribe(b_info.id, sink).expect("subscribe B");

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "sender-tok".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };
    registry.revoke(sender, 0);

    let cmd = ControlCommand {
        from,
        to: vec!["target-b".to_string()],
        body: "revoked-but-DELIVERED".to_string(),
        contract: Default::default(),
    };
    let result = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "delivered even from a discarded sender (liveness is not a gate, user decision): {v}"
    );
    assert_eq!(
        v["results"][0]["to"], "target-b",
        "the resolved recipient name is attached"
    );
    assert!(v["id"].is_string(), "msg-id is attached");

    // Anchor assertion (no loose substring): `j` is the whole JSON of the user echo, so a `"` inside the envelope is `\"` — `"text":"<message from=\"sender\">` pins the sender at the envelope start (a render padding the sender fails).
    let sender_display = &sender.to_string()[..8];
    let anchored_envelope =
        format!(r#""text":"<message from=\"{sender_display}\">revoked-but-DELIVERED</message>"#);
    let delivered = wait_until(Duration::from_secs(3), || {
        seen.lock()
            .unwrap()
            .iter()
            .any(|j| j.contains(&anchored_envelope))
    });
    assert!(
        delivered,
        "the wrapped line must be injected into B's stdin even from a discarded sender (delivered): {:?}",
        seen.lock().unwrap()
    );

    manager.kill_agent(b_info.id).ok();
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── ADR-0088 Stage 0: delivery-boundary observation record — retrieved by an in-proc sink (no detached log scraping) ──
// The production daemon runs detached, so log scraping is a do-not (ADR-0088 HARD CONSTRAINT); a DeliveryObserver installed on the registry with handle_send called directly asserts the record without logs.
struct DeliveryCapture {
    seen: Arc<Mutex<Vec<engram_dashboard_messaging::envelope::DeliveryObservation>>>,
}
impl engram_dashboard_messaging::envelope::DeliveryObserver for DeliveryCapture {
    fn observe(&self, obs: engram_dashboard_messaging::envelope::DeliveryObservation) {
        self.seen.lock().unwrap().push(obs);
    }
}

// ── ADR-0088 (FIX-3/FIX-4): a session seam that drives delivery-boundary observation without the claude binary ──────────────
// The e2e tests above need a live claude spawn, tying the core assertion to the binary (FIX-4). `AgentManager::insert_test_session` plants a session that imitates a structured=true carrier where we decide whether write succeeds or fails, measuring both handle_send branches without claude.
mod obs_seam {
    use std::sync::atomic::AtomicU8;
    use std::sync::{Arc, Mutex};

    use engram_dashboard_agent::backend::InputEncoder;
    use engram_dashboard_agent::manager::AgentManager;
    use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
    use engram_dashboard_agent::session::AgentSession;
    use engram_dashboard_agent::transport::AgentTransport;
    use engram_dashboard_agent::types::{
        AgentId, AgentStatus, BackendCaps, ControlCaps, InputCaps, InputEvent, ModelCaps,
        OutputCaps, PtyError, SessionCaps, StatusSink, TransportCaps,
    };

    struct NoopStatus;
    impl StatusSink for NoopStatus {
        fn status_changed(&self, _id: AgentId, _s: AgentStatus, _e: u32) {}
        fn agent_list_updated(&self, _a: Vec<engram_dashboard_agent::types::AgentInfo>) {}
    }

    /// `structured`: does this carrier emit structured output (= a turn signal). Default true (a json claude stand-in); false is a terminal claude stand-in — whether that class stays on the list is the regression axis of ADR-0116 decisions 1·7.
    struct SeamTransport {
        fail: bool,
        structured: bool,
        captured: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl AgentTransport for SeamTransport {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            if self.fail {
                return Err(PtyError::WriteFailed("seam: recipient stdin closed".into()));
            }
            let InputEvent::Raw(bytes) = input;
            self.captured.lock().unwrap().push(bytes);
            Ok(())
        }
        fn resize(&self, _c: u16, _r: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            TransportCaps {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: false,
                    structured: self.structured,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: false,
                    interrupt: false,
                    cancel: false,
                    graceful_shutdown: false,
                },
            }
        }
    }

    fn backend_caps() -> BackendCaps {
        BackendCaps {
            session: SessionCaps {
                resume: true,
                snapshot: false,
                cwd_env: true,
            },
            model: ModelCaps {
                select: false,
                temperature: false,
                max_tokens: false,
            },
        }
    }

    pub fn insert_seam_recipient(
        manager: &Arc<AgentManager>,
        fail: bool,
    ) -> (AgentId, Arc<Mutex<Vec<Vec<u8>>>>) {
        let id = AgentId::new_v4();
        let name = id.to_string()[..8].to_string();
        insert_seam_recipient_named(manager, fail, id, &name)
    }

    /// Not made by a real termination: the reaper removes the session from the map at once, so "in list_agents yet status terminal" cannot be reproduced with a real session.
    pub fn insert_terminal_seam_recipient(manager: &Arc<AgentManager>, name: &str) -> AgentId {
        let id = AgentId::new_v4();
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        // Calls the terminal transition (pump's sole ownership — ADR-0005) directly: this session has no pump, so no finalize competitor. Result = it stays in the map with only the status terminal (Killed).
        core.finish(engram_dashboard_agent::types::TerminalReason::Killed);
        let session = Arc::new(AgentSession::new(
            id,
            std::path::PathBuf::from(format!("seam-root/{name}")),
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            backend_caps(),
            InputEncoder::ClaudeStreamJson,
            true,
            core,
            Box::new(SeamTransport {
                fail: false,
                structured: true,
                captured: Arc::new(Mutex::new(Vec::new())),
            }),
        ));
        manager.insert_test_session(session);
        id
    }

    /// For the many-same-names scenario (D review B1) — specifies the AgentId and the visible name separately; two sessions with one name show two same-name roster entries, and the sender can point at one only by the exact AgentId.
    pub fn insert_seam_recipient_named(
        manager: &Arc<AgentManager>,
        fail: bool,
        id: AgentId,
        name: &str,
    ) -> (AgentId, Arc<Mutex<Vec<Vec<u8>>>>) {
        // A core with no observation wiring — most tests must not ride the idle gate (the injection's user echo would turn the recipient mid-turn and park the next send).
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        let (agent, captured, _core) = insert_seam_with_core(
            manager,
            fail,
            id,
            name,
            core,
            InputEncoder::ClaudeStreamJson,
            true,
        );
        (agent, captured)
    }

    /// Specifies the recipient's carrier kind — contrasts a terminal claude stand-in (`Raw` + no turn signal) with a json claude stand-in (`ClaudeStreamJson` + a turn signal) in one test (other helpers are fixed to json).
    /// The two axes are tied in one encoder because they move together in reality; taking them separately could create a nonexistent combination.
    pub fn insert_seam_recipient_with_encoder(
        manager: &Arc<AgentManager>,
        encoder: InputEncoder,
    ) -> (AgentId, Arc<Mutex<Vec<Vec<u8>>>>) {
        let id = AgentId::new_v4();
        let name = id.to_string()[..8].to_string();
        let core = Arc::new(OutputCore::new(
            id,
            0,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        let structured = encoder == InputEncoder::ClaudeStreamJson;
        let (agent, captured, _core) =
            insert_seam_with_core(manager, false, id, &name, core, encoder, structured);
        (agent, captured)
    }

    pub fn insert_observed_seam_recipient(
        manager: &Arc<AgentManager>,
        fail: bool,
    ) -> (AgentId, Arc<Mutex<Vec<Vec<u8>>>>, Arc<OutputCore>) {
        let id = AgentId::new_v4();
        let name = id.to_string()[..8].to_string();
        // Pull the classifier via the same dispatch as production spawn — the seam session imitates a json-mode claude carrier.
        let core = manager.wired_test_core(
            id,
            0,
            engram_dashboard_agent::backend::turn_classifier(
                &engram_dashboard_agent::profile::AgentCommand::Claude {
                    extra_args: vec![],
                    output_format: engram_dashboard_agent::profile::AgentOutputFormat::StreamJson,
                },
            ),
        );
        insert_seam_with_core(
            manager,
            fail,
            id,
            &name,
            core,
            InputEncoder::ClaudeStreamJson,
            true,
        )
    }

    fn insert_seam_with_core(
        manager: &Arc<AgentManager>,
        fail: bool,
        id: AgentId,
        name: &str,
        core: Arc<OutputCore>,
        encoder: InputEncoder,
        structured: bool,
    ) -> (AgentId, Arc<Mutex<Vec<Vec<u8>>>>, Arc<OutputCore>) {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let core_out = core.clone();
        // ADR-0101 (WYSIWYA): canonical name = display_name ?? basename(session.cwd), and a seam session has no profile, so the cwd basename is its addressable name. The test targets by fallback_name(id)=id[:8], so the cwd basename is matched to id[:8] (the old cwd="." had basename "." — untargetable, same-name collisions).
        let cwd = std::path::PathBuf::from(format!("seam-root/{name}"));
        let session = Arc::new(AgentSession::new(
            id,
            cwd,
            0,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            backend_caps(),
            encoder,
            // The seam recipient is a claude stand-in so mail eligibility is true — the shell axis is covered by the real spawn test (`messaging_host::tests::a_live_shell_is_excluded_from_the_mail_roster_but_still_counts_as_live`).
            true,
            core,
            Box::new(SeamTransport {
                fail,
                structured,
                captured: captured.clone(),
            }),
        ));
        manager.insert_test_session(session);
        (id, captured, core_out)
    }

    pub fn last_written(captured: &Arc<Mutex<Vec<Vec<u8>>>>) -> Vec<u8> {
        captured.lock().unwrap().last().cloned().unwrap_or_default()
    }

    pub fn all_written(captured: &Arc<Mutex<Vec<Vec<u8>>>>) -> Vec<Vec<u8>> {
        captured.lock().unwrap().clone()
    }

    pub fn fallback_name(id: AgentId) -> String {
        id.to_string()[..8].to_string()
    }

    /// Rebuilding the expected envelope (ADR-0103 — production default = xml): the test rebuilds the daemon's default-format envelope to account byte-exactly — plain `<message from="{sender}">{body}</message>` (no attributes — current-scope increment A).
    /// This helper does no XML escaping, so it matches only fixtures whose sender/body have no `<`/`>`/`&`/`"` (all current call sites).
    pub fn expected_default_envelope(sender: &str, body: &str) -> String {
        format!("<message from=\"{sender}\">{body}</message>")
    }
}

// ── ADR-0088 (FIX-4): the delivery-observation core assertions without claude — a successful relay to a seam recipient ──────────────
#[tokio::test]
async fn control_send_delivery_observation_via_seam_no_claude() {
    use engram_dashboard_agent::backend::InputEncoder;
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("obs-seam-ok").await;

    let (b_id, captured) = obs_seam::insert_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "seam-ok-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let body = "안녕-msg-α"; // two Hangul chars (6B) + "-msg-"(5B) + α(2B) = 13B. hangul-ok
    let cmd = ControlCommand {
        from,
        to: vec![to_name.clone()],
        body: body.to_string(),
        contract: Default::default(),
    };
    let result = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "seam successful delivery ACK: {v}"
    );
    let ack_id = v["id"].as_str().expect("msg-id is attached").to_string();

    let obs = {
        let g = seen.lock().unwrap();
        assert_eq!(
            g.len(),
            1,
            "1 successful relay → 1 observation record: {:?}",
            *g
        );
        g[0].clone()
    };

    assert_eq!(
        obs.msg_id, ack_id,
        "record msg_id = ACK id (correlation axis 1)"
    );
    assert!(
        obs.msg_uuid.is_some(),
        "a successful delivery must carry msg_uuid (correlation axis 2)"
    );

    let sender_name = obs_seam::fallback_name(sender);
    let expected_wrapped = obs_seam::expected_default_envelope(&sender_name, body);
    let expected_bytes = expected_wrapped.len();
    assert_eq!(
        obs.bytes_requested, expected_bytes,
        "request bytes = the exact UTF-8 byte count of the envelope (crossing multibyte): got={} expect={} wrapped={:?}",
        obs.bytes_requested, expected_bytes, expected_wrapped
    );
    assert_eq!(
        obs.bytes_written,
        Some(obs.bytes_requested),
        "by-construction copy (bytes_written = request) — not short-write detection"
    );
    assert!(obs.error.is_none(), "a successful delivery has error None");
    assert!(obs.is_delivered(), "is_delivered() = true");
    assert_eq!(obs.to_id, b_id, "the record's recipient AgentId");
    assert_eq!(
        obs.to_name, to_name,
        "the record's recipient name (fallback)"
    );
    assert_eq!(
        obs.from,
        from.into(),
        "the record's sender identity (token-derived)"
    );

    // Layer-crossing (exact bytes): the bytes the session received through write = encoder(envelope, msg_uuid). An XML envelope's `"` is escaped to `\"` in the stream-json encoding, so a raw envelope-string substring comparison fails — the expected line is rebuilt with the encoder and asserted byte-exactly (multibyte integrity + handoff truncation/corruption).
    let written = obs_seam::last_written(&captured);
    let msg_uuid = obs.msg_uuid.expect("a successful delivery msg_uuid");
    let expected_line =
        InputEncoder::ClaudeStreamJson.encode(expected_wrapped.as_bytes(), msg_uuid);
    assert_eq!(
        written,
        expected_line,
        "the stream-json line the session received must match the expected encoded envelope byte-exactly: {}",
        String::from_utf8_lossy(&written)
    );

    manager.kill_agent(b_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── ADR-0116 decisions 1·7 seal: a terminal agent without a turn signal stays on the list ─────────────────────────
// Blocked regression: reviving `.filter(|a| a.capabilities.output.structured)` in the adapter's output function makes every terminal claude silently unable to receive letters (no parking trigger either; expiry by the 24h TTL). That class cannot be spawned without a real claude binary, so the same shape (Raw input + no structured output) is made through the session injection seam.
// A different axis from the shell exclusion (`reads_messages`): capability vs "what the input is interpreted as" — one function has two filters, so both need coverage (the shell axis = the messaging_host real spawn test).
#[tokio::test]
async fn roster_includes_a_terminal_agent_without_a_turn_signal_no_claude() {
    use engram_dashboard_agent::backend::InputEncoder;
    use engram_dashboard_daemon::messaging_host::ManagerDeliveryPort;
    use engram_dashboard_messaging::service::DeliveryPort;

    let (manager, _registry, _base, data_dir, handle, _messaging, _busy) =
        wire("no-signal-roster").await;
    let port = ManagerDeliveryPort::new(manager.clone());

    let (id, _captured) = obs_seam::insert_seam_recipient_with_encoder(&manager, InputEncoder::Raw);

    let entry = port
        .live_agents()
        .into_iter()
        .find(|a| a.id == id)
        .expect("it must be on the delivery list even without a turn signal (capability is not a membership axis)");
    assert!(
        !entry.turn_signal,
        "that fact shows up only as turn_signal=false (= an immediate-injection target with no gate): {entry:?}"
    );
    let sources = port.addressing_sources();
    assert!(
        sources.roster.iter().any(|a| a.id == id && !a.turn_signal),
        "the entry-decision source must use the same predicate (filtering one side only blocks that class wholesale): {sources:?}"
    );

    manager.kill_agent(id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── to a terminal (TUI) recipient the submit (CR) goes out as a separate write once more ───────────────────────────
// Defect recurrence axis = the write boundary: the envelope bytes reached the PTY before, but the claude TUI kept them in its input box without starting a turn (measured 2026-08-17 — one write of `body+CR` is not submitted, two writes are), so it asserts "were the writes split in two", not "did the bytes go".
// Recipients are sent one at a time: with two a `to` attribute is attached and the expected string is hard to rebuild; one at a time both can be pinned by a byte-exact match, catching anything added.
#[tokio::test]
async fn control_send_adds_a_separate_submit_write_only_for_terminal_recipients_no_claude() {
    use engram_dashboard_agent::backend::InputEncoder;
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("submit-split").await;

    let (term_id, term_captured) =
        obs_seam::insert_seam_recipient_with_encoder(&manager, InputEncoder::Raw);
    let (json_id, json_captured) =
        obs_seam::insert_seam_recipient_with_encoder(&manager, InputEncoder::ClaudeStreamJson);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "submit-split-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };
    let body = "hi";
    let envelope = obs_seam::expected_default_envelope(&obs_seam::fallback_name(sender), body);

    let send_to = |id| {
        let v = handle_send(
            &manager,
            &registry,
            &messaging,
            Entrance::Cli,
            ControlCommand {
                from,
                to: vec![obs_seam::fallback_name(id)],
                body: body.to_string(),
                contract: Default::default(),
            },
        )
        .to_json();
        assert_eq!(v["results"][0]["status"], "delivered", "{v}");
    };
    send_to(term_id);
    send_to(json_id);

    assert_eq!(
        obs_seam::all_written(&term_captured),
        vec![envelope.as_bytes().to_vec(), b"\r".to_vec()],
        "terminal recipient = the envelope as is once + the submit (CR) alone once, **two separate writes**"
    );

    // For json, rebuild the expected line from that delivery's msg_uuid and check a byte-exact match — a leaked submit write or appended characters break here.
    let json_uuid = seen
        .lock()
        .unwrap()
        .iter()
        .find(|o| o.to_id == json_id)
        .and_then(|o| o.msg_uuid)
        .expect("the observation record + msg_uuid of the json delivery");
    assert_eq!(
        obs_seam::all_written(&json_captured),
        vec![InputEncoder::ClaudeStreamJson.encode(envelope.as_bytes(), json_uuid)],
        "a json recipient must get exactly one encoder output as before (the trailing \\n already submits)"
    );

    manager.kill_agent(term_id).ok();
    manager.kill_agent(json_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── ADR-0088 extension (review F1/F2): DeliveryObservation.in_reply_to — derived only from structured meta ───────────
// reply_to need not point at an open request: strict matching is used only when closing the ledger contract (NoMatch is the normal path — `close_reply_contract` in service.rs) and the message is delivered as is, so the observation axis is checked independently with an arbitrary id.
#[tokio::test]
async fn control_send_reply_to_carries_structured_in_reply_to_no_claude() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, SendContract};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("obs-reply-to").await;

    let (b_id, _captured) = obs_seam::insert_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "obs-reply-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let cmd = ControlCommand {
        from,
        to: vec![to_name.clone()],
        body: "Done coding, tests pass".to_string(),
        contract: SendContract {
            request: false,
            reply_by: None,
            reply_to: Some("m-7f3k9q2d".to_string()),
        },
    };
    let result = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "seam successful delivery ACK: {v}"
    );

    let obs = {
        let g = seen.lock().unwrap();
        assert_eq!(
            g.len(),
            1,
            "1 successful relay → 1 observation record: {:?}",
            *g
        );
        g[0].clone()
    };
    assert_eq!(
        obs.in_reply_to.as_deref(),
        Some("m-7f3k9q2d"),
        "the observation record of a reply send must carry the SendMeta.reply_to value as is (structured derivation, F1)"
    );

    manager.kill_agent(b_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

#[tokio::test]
async fn control_send_plain_send_has_no_in_reply_to_no_claude() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("obs-plain-no-reply").await;

    let (b_id, _captured) = obs_seam::insert_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "obs-plain-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let cmd = ControlCommand {
        from,
        to: vec![to_name.clone()],
        // F2 forgery pin: the body deliberately carries a fake in-reply-to attribute string — harmless text would still pass if the old substring parser (deleted in F1) came back. Only a forged attribute in the body pins the security invariant "in_reply_to is derived structurally from SendMeta.reply_to, not by re-parsing the body".
        body: r#"Done. See in-reply-to="m-forged1""#.to_string(),
        contract: Default::default(),
    };
    let result = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "seam successful delivery ACK: {v}"
    );

    let obs = {
        let g = seen.lock().unwrap();
        assert_eq!(
            g.len(),
            1,
            "1 successful relay → 1 observation record: {:?}",
            *g
        );
        g[0].clone()
    };
    assert_eq!(
        obs.in_reply_to, None,
        "a plain notice send must have in_reply_to None even if the body carries a forged in-reply-to attribute (structured derivation, not re-parsing)"
    );

    manager.kill_agent(b_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── ADR-0088 (FIX-2): observation sink panic isolation — delivery/ACK unaffected (the immediate-push invariant) ───────────────
// If the ACK were lost just because observation was on, the sender would retry → duplicate delivery; this guards that regression.
#[tokio::test]
async fn control_send_observer_panic_does_not_break_delivery_or_ack() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::{DeliveryObservation, DeliveryObserver, Entrance};

    struct PanicObserver;
    impl DeliveryObserver for PanicObserver {
        fn observe(&self, _obs: DeliveryObservation) {
            panic!("seam: observer boom (intended panic — must be isolated)");
        }
    }

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("obs-panic").await;
    let (b_id, _captured) = obs_seam::insert_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    registry.set_delivery_observer(Arc::new(PanicObserver));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "panic-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let cmd = ControlCommand {
        from,
        to: vec![to_name],
        body: "trigger-panic-observer".to_string(),
        contract: Default::default(),
    };
    // If the panic is not isolated in record_delivery, the test dies here by unwinding.
    let result = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "even with an observer panic, delivery/ACK must be normal (Enqueued): {v}"
    );

    manager.kill_agent(b_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── ADR-0088 (FIX-3): relay write failure → the observation record does not swallow the failure as a success ────────────────
#[tokio::test]
async fn control_send_delivery_failure_observation_records_error_not_success() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("obs-fail").await;

    // ★fail=true★: makes the relay write an Err to force the Err branch of handle_send.
    let (b_id, _captured) = obs_seam::insert_seam_recipient(&manager, true);
    let to_name = obs_seam::fallback_name(b_id);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "fail-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let body = "this-delivery-will-fail";
    let cmd = ControlCommand {
        from,
        to: vec![to_name.clone()],
        body: body.to_string(),
        contract: Default::default(),
    };
    let result = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "pending",
        "a write failure switches to parking (pending), not a refusal (spec §5): {v}"
    );

    let obs = {
        let g = seen.lock().unwrap();
        assert_eq!(
            g.len(),
            1,
            "1 failed relay → 1 observation record: {:?}",
            *g
        );
        g[0].clone()
    };

    assert!(
        obs.error.is_some(),
        "a failed delivery must be error=Some (not swallowed as a success): {obs:?}"
    );
    assert_eq!(obs.bytes_written, None, "on failure bytes_written=None");
    assert_eq!(
        obs.msg_uuid, None,
        "on failure msg_uuid=None (no write happened)"
    );
    assert!(!obs.is_delivered(), "a failure is is_delivered()==false");
    assert!(
        obs.bytes_requested > body.len(),
        "a failure record must also carry the request bytes (envelope size): req={} body={}",
        obs.bytes_requested,
        body.len()
    );
    assert_eq!(obs.to_id, b_id, "the failure record's recipient AgentId");
    assert_eq!(
        obs.from,
        from.into(),
        "the failure record's sender identity"
    );

    manager.kill_agent(b_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

#[tokio::test]
async fn control_send_delivery_observation_records_bytes_and_correlated_ids() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("delivery-obs").await;

    let Some((b_info, _b_tok)) = spawn_json_agent(&manager, &registry, "obs-target") else {
        require_claude("control_send_delivery_observation_records_bytes_and_correlated_ids");
    };

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "obs-sender-tok".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let body = "observe-me-안녕-α"; // 11 ASCII chars + 2 Hangul chars (6B) + '-'(1B) + α(2B). hangul-ok
    let cmd = ControlCommand {
        from,
        to: vec!["obs-target".to_string()],
        body: body.to_string(),
        contract: Default::default(),
    };
    let result = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "delivery success ACK: {v}"
    );
    let ack_id = v["id"].as_str().expect("msg-id is attached").to_string();

    let obs = {
        let g = seen.lock().unwrap();
        assert_eq!(
            g.len(),
            1,
            "1 successful relay → 1 observation record: {:?}",
            *g
        );
        g[0].clone()
    };

    assert_eq!(
        obs.msg_id, ack_id,
        "the record msg_id must equal the ACK id (correlation axis 1)"
    );
    assert!(
        obs.msg_uuid.is_some(),
        "a successful delivery must carry the correlated msg_uuid (correlation axis 2)"
    );
    let sender_name = sender.to_string()[..8].to_string();
    let expected_wrapped = obs_seam::expected_default_envelope(&sender_name, body);
    assert_eq!(
        obs.bytes_requested,
        expected_wrapped.len(),
        "request bytes = the exact UTF-8 byte count of the envelope (crossing multibyte): got={} wrapped={:?}",
        obs.bytes_requested,
        expected_wrapped
    );
    assert_eq!(
        obs.bytes_written,
        Some(obs.bytes_requested),
        "by-construction copy (bytes_written = request) — not short-write detection"
    );
    assert!(obs.error.is_none(), "a successful delivery has error None");
    assert!(
        obs.is_delivered(),
        "is_delivered() = true (transfer complete)"
    );
    assert_eq!(obs.to_id, b_info.id, "the record's recipient AgentId");
    assert_eq!(obs.to_name, "obs-target", "the record's recipient name");
    assert_eq!(
        obs.from,
        from.into(),
        "the record's sender identity (token-derived)"
    );

    manager.kill_agent(b_info.id).ok();
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ═══════════════════════════════════════════════════════════════════════════════════════════
// ADR-0088 Stage 1 — delivery correctness oracle (deterministic · seam-based, no real claude needed)
// ═══════════════════════════════════════════════════════════════════════════════════════════
// Seam observation scope = handle_send → registry → submit_stdin_observed → session.submit_input_observed (envelope assembly · encoder) → SeamTransport.send_input inclusive. The physical layer below (production `StdioTransport::send_input`'s
//   `stdin.lock()` + `write_all`/`flush`) is bypassed — SeamTransport receives a complete Vec and captures it atomically with `push`.
// If production behavior violates the oracle, the test is not weakened but left failing and reported as a FINDING (no masking).

/// ── ADR-0088 Stage 1 oracle 1: concurrent entrance exact-once + N distinct bodies delivered intact (seam handoff) ──
/// Does not prove (coverage gaps):
///   (1) no interleaving of physical OS-pipe bytes — that serialization is the `stdin.lock()` layer this seam bypasses, so erasing it is not caught here. ▶ `physical_pipe_concurrent_sends_no_interleave` in the agent crate's `tests/stdio_physical_pipe.rs`.
///   (2) correctness inside the encoder — actual and expected use the same encoder, so an encoder defect (e.g. wrap_user_turn dropping the newline) pollutes both. ▶ golden unit test `wrap_user_turn_exact_line_and_newline_terminated` in `backend/claude/`.
#[tokio::test]
async fn stage1_concurrent_sends_exact_once_distinct_bodies_intact_at_seam() {
    use engram_dashboard_agent::backend::InputEncoder;
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;
    use std::sync::Barrier;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("stage1-concurrency").await;

    // One seam recipient (fail=false → the success path).
    let (b_id, captured) = obs_seam::insert_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    // All threads send with the same identity — piling onto 1 recipient is the point.
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "stage1-conc-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    const N: usize = 100;
    // The markers must have no special characters so the envelope string enters the captured line as a subsequence without JSON escape; the idx zero-pad prevents false subsequence matches (1 ⊂ 10).
    let markers: Vec<String> = (0..N).map(|i| format!("BODY-{i:04}")).collect();

    // Barrier (entrance alignment): gathers the N threads just before handle_send so early threads cannot finish before late ones spawn. Limit: it aligns entry only and cannot force overlap inside handle_send (a single core/scheduler can still serialize it).
    let barrier = Arc::new(Barrier::new(N));

    let mut handles = Vec::with_capacity(N);
    for marker in &markers {
        let manager = manager.clone();
        let registry = registry.clone();
        let messaging = messaging.clone();
        let to = to_name.clone();
        let body = marker.clone();
        let barrier = barrier.clone();
        // handle_send is sync(&Arc<..>) — fired near-simultaneously from OS threads (not tokio tasks, for real parallelism).
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            let cmd = ControlCommand {
                from,
                to: vec![to],
                body,
                contract: Default::default(),
            };
            let result = handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd);
            let v = result.to_json();
            assert!(
                v.get("results").is_some(),
                "concurrent firing is also each accepted (results): {v}"
            );
            v["id"].as_str().expect("msg-id").to_string()
        }));
    }
    let ack_ids: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    // Do not promote "all `delivered`" to an assertion: a send that stepped back in an overlapping drain answers `pending` (ADR-0125) and goes out later via the winner's batch or a re-rung doorbell. The axis is no loss or duplication regardless of order + envelope byte integrity.
    for _ in 0..600 {
        if seen.lock().unwrap().len() >= N && messaging.parked_len(b_id) == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let obs_records = { seen.lock().unwrap().clone() };
    assert_eq!(
        obs_records.len(),
        N,
        "N concurrent firings → exactly N observation records (no loss/duplication at the concurrent entrance)"
    );
    let distinct_obs: std::collections::HashSet<&String> =
        obs_records.iter().map(|o| &o.msg_id).collect();
    assert_eq!(
        distinct_obs.len(),
        N,
        "all observed msg_ids distinct (no duplicate delivery)"
    );
    let distinct_ack: std::collections::HashSet<&String> = ack_ids.iter().collect();
    assert_eq!(distinct_ack.len(), N, "all ACK ids distinct");

    let sender_name = obs_seam::fallback_name(sender);
    let writes = obs_seam::all_written(&captured);
    assert_eq!(
        writes.len(),
        N,
        "captured write count == N (each send_input is 1 complete envelope — no truncation/merging)"
    );
    let by_uuid: std::collections::HashMap<
        uuid::Uuid,
        &engram_dashboard_messaging::envelope::DeliveryObservation,
    > = obs_records
        .iter()
        .filter_map(|o| o.msg_uuid.map(|u| (u, o)))
        .collect();
    assert_eq!(
        by_uuid.len(),
        N,
        "a unique msg_uuid per success record (no correlation-key collision)"
    );

    let mut matched_uuids: std::collections::HashSet<uuid::Uuid> = std::collections::HashSet::new();
    let mut received_bodies: Vec<String> = Vec::with_capacity(N);
    for (i, w) in writes.iter().enumerate() {
        let s = std::str::from_utf8(w)
            .unwrap_or_else(|e| panic!("write[{i}] is not intact UTF-8: {e}"));
        let line_json: serde_json::Value = serde_json::from_str(s.trim_end()).unwrap_or_else(|e| {
            panic!("write[{i}] is not an intact stream-json line (merge/truncation suspected): {e} in {s:?}")
        });
        let line_uuid: uuid::Uuid = line_json["uuid"]
            .as_str()
            .and_then(|u| u.parse().ok())
            .unwrap_or_else(|| panic!("write[{i}] has no top-level uuid: {s:?}"));
        let obs = by_uuid.get(&line_uuid).unwrap_or_else(|| {
            panic!("write[{i}]'s msg_uuid={line_uuid} has no corresponding observation record (a ghost write)")
        });
        assert!(
            matched_uuids.insert(line_uuid),
            "write[{i}]'s msg_uuid={line_uuid} was captured twice (a duplicate write)"
        );
        let hits: Vec<&String> = markers.iter().filter(|m| s.contains(m.as_str())).collect();
        assert_eq!(
            hits.len(),
            1,
            "write[{i}] must carry exactly 1 envelope marker (no merging at the seam level) — observed: {hits:?}"
        );
        let body = hits[0];
        received_bodies.push(body.clone());
        // Exact-byte reconstruction: this line made with the same encoder and msg_uuid is the bytes the session passed to send_input — exact-eq holds as a handoff oracle.
        let wrapped = obs_seam::expected_default_envelope(&sender_name, body);
        let expected_line = InputEncoder::ClaudeStreamJson.encode(wrapped.as_bytes(), line_uuid);
        assert_eq!(
            w, &expected_line,
            "write[{i}] must match the expected encoded envelope byte-exactly (detects session→transport handoff truncation/corruption/merging — not encoder-internal correctness): body={body} msg_id={}",
            obs.msg_id
        );
    }
    assert_eq!(
        matched_uuids.len(),
        N,
        "all N msg_uuids delivered by exactly 1 write (exact-once, multiset equality)"
    );

    // Blocks a substitution bug (FIX-1): the exact-bytes reconstruction extracts the body from the write itself, so "every message → the same body" would pass self-consistently (each write carries BODY-0000, reconstructed as BODY-0000). A direct comparison with the set of sent markers closes the hole.
    {
        let mut sent_sorted: Vec<String> = markers.clone();
        sent_sorted.sort();
        let distinct_sent: std::collections::HashSet<&String> = sent_sorted.iter().collect();
        assert_eq!(
            distinct_sent.len(),
            N,
            "test premise: all sent markers are distinct"
        );
        let mut received_sorted = received_bodies.clone();
        received_sorted.sort();
        assert_eq!(
            received_sorted, sent_sorted,
            "the received body multiset must exactly match the N distinct bodies sent (each once — no loss, duplication or substitution). \
             With a substitution bug (every message → the same body) this equality breaks (one body N times, the rest 0 times). \
             sent={sent_sorted:?} received={received_sorted:?}"
        );
    }

    manager.kill_agent(b_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// ── ADR-0088 Stage 1 oracle 2: body size boundary (MAX_BODY_BYTES = 64 KiB), bytes vs chars ──────────
/// The captured write is compared directly (FIX-2): `bytes_requested` is the envelope copy value before encoding and cannot catch truncation or corruption at the session→transport handoff.
#[tokio::test]
async fn stage1_body_size_boundary_bytes_not_chars() {
    use engram_dashboard_agent::backend::InputEncoder;
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    const MAX: usize = 64 * 1024; // = MAX_BODY_BYTES (an ingress constant — mirrored here; if the value drifts the assertions below catch it).

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("stage1-boundary").await;

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "stage1-boundary-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    // Each case plants a fresh seam recipient + a fresh observer to avoid state accumulation.
    async fn send_once(
        manager: &Arc<AgentManager>,
        registry: &Arc<ControlRegistry>,
        messaging: &Arc<MessagingService>,
        from: BoundIdentity,
        body: String,
    ) -> (
        serde_json::Value,
        Option<engram_dashboard_messaging::envelope::DeliveryObservation>,
        Vec<u8>,
        usize,   // the expected byte length of the envelope (wrap_message)
        Vec<u8>, // the expected encoded stream-json line (rebuilt on success, an empty Vec on failure)
        AgentId,
    ) {
        let (b_id, captured) = obs_seam::insert_seam_recipient(manager, false);
        let to_name = obs_seam::fallback_name(b_id);
        let seen = Arc::new(Mutex::new(Vec::new()));
        registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

        let cmd = ControlCommand {
            from,
            to: vec![to_name.clone()],
            body: body.clone(),
            contract: Default::default(),
        };
        let result = handle_send(manager, registry, messaging, Entrance::Cli, cmd);
        let v = result.to_json();
        let obs = seen.lock().unwrap().first().cloned();
        let written = obs_seam::last_written(&captured);
        // The MAX gate is based on the body: 64 KiB is the body length so it passes the gate, and the envelope wrapper rides on top.
        let sender_name = obs_seam::fallback_name(from.agent_id);
        let expected_env_bytes = obs_seam::expected_default_envelope(&sender_name, &body).len();
        let expected_line = match obs.as_ref().and_then(|o| o.msg_uuid) {
            Some(uuid) => {
                let wrapped = obs_seam::expected_default_envelope(&sender_name, &body);
                InputEncoder::ClaudeStreamJson.encode(wrapped.as_bytes(), uuid)
            }
            None => Vec::new(),
        };
        (v, obs, written, expected_env_bytes, expected_line, b_id)
    }

    // ── (1) exactly 64 KiB (boundary included) → delivered ──────────────────────────────────────────────
    let body_eq = "x".repeat(MAX);
    assert_eq!(body_eq.len(), MAX, "test premise: exactly 64 KiB");
    let (v, obs, written, env_bytes, expected_line, b_id) =
        send_once(&manager, &registry, &messaging, from, body_eq.clone()).await;
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "exactly 64 KiB must be delivered (≤ the cap): {v}"
    );
    let obs = obs.expect("a successful delivery has an observation record");
    assert_eq!(
        obs.bytes_requested, env_bytes,
        "request bytes = the exact byte length of the envelope"
    );
    assert!(obs.is_delivered(), "exactly 64 KiB is is_delivered()");
    assert_eq!(
        written, expected_line,
        "the seam capture must be the exact encoded bytes of the 64 KiB envelope (detects session→transport handoff truncation/corruption)"
    );
    manager.kill_agent(b_id).ok();

    // ── (2) 64 KiB − 1 → delivered ───────────────────────────────────────────────────────────
    let body_lt = "x".repeat(MAX - 1);
    let (v, obs, written, env_bytes, expected_line, b_id) =
        send_once(&manager, &registry, &messaging, from, body_lt).await;
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "64 KiB−1 must be delivered: {v}"
    );
    assert_eq!(
        obs.expect("observation").bytes_requested,
        env_bytes,
        "64 KiB−1: request bytes = the exact envelope length"
    );
    assert_eq!(
        written, expected_line,
        "64 KiB−1: the captured write matches the expected encoded envelope byte-exactly (detects truncation)"
    );
    manager.kill_agent(b_id).ok();

    // ── (3) 64 KiB + 1 → BODY_TOO_LARGE, no write attempt (capture 0) ────────────────────────────
    let body_gt = "x".repeat(MAX + 1);
    let (b_id, captured) = obs_seam::insert_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);
    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));
    let result = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![to_name],
            body: body_gt,
            contract: Default::default(),
        },
    );
    let v = result.to_json();
    assert_eq!(v["status"], "error", "64 KiB+1 is refused: {v}");
    assert_eq!(
        v["code"], "BODY_TOO_LARGE",
        "exceeding is BODY_TOO_LARGE: {v}"
    );
    assert!(
        obs_seam::all_written(&captured).is_empty(),
        "exceeding the cap must not even attempt a write (capture 0 — the bytes never reach the recipient)"
    );
    assert!(
        seen.lock().unwrap().is_empty(),
        "exceeding the cap must leave no delivery observation record either (the relay is not entered)"
    );
    manager.kill_agent(b_id).ok();

    // ── (4) multibyte: char count < 64Ki but bytes > 64Ki → BODY_TOO_LARGE (proves the cap is bytes) ──────
    // '가' (U+AC00) = 3 bytes in UTF-8. (MAX/3 + 1) chars → the char count is ~21846 (≪ 64Ki chars) but the bytes are > MAX. // hangul-ok
    let char_count = MAX / 3 + 1;
    let body_mb_over = "가".repeat(char_count); // hangul-ok
    assert!(
        body_mb_over.chars().count() < MAX,
        "multibyte premise: the char count ({}) is under 64Ki",
        body_mb_over.chars().count()
    );
    assert!(
        body_mb_over.len() > MAX,
        "multibyte premise: the byte count ({}) is over 64Ki",
        body_mb_over.len()
    );
    let (b_id, captured) = obs_seam::insert_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);
    let result = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![to_name],
            body: body_mb_over,
            contract: Default::default(),
        },
    );
    let v = result.to_json();
    assert_eq!(
        v["code"], "BODY_TOO_LARGE",
        "a multibyte body's cap is also measured in **bytes**, not chars (it must be refused though chars<64Ki): {v}"
    );
    assert!(
        obs_seam::all_written(&captured).is_empty(),
        "a multibyte excess also makes no write attempt"
    );
    manager.kill_agent(b_id).ok();

    // ── (5) multibyte straddle: the char count is the same but the bytes are just under the boundary → delivered (confirms the boundary is in bytes) ──
    let char_count_ok = MAX / 3; // floor → bytes = char_count_ok*3 ≤ MAX
    let body_mb_ok = "가".repeat(char_count_ok); // hangul-ok
    assert!(
        body_mb_ok.len() <= MAX,
        "straddle premise: bytes ({}) ≤ 64Ki",
        body_mb_ok.len()
    );
    let (v, obs, written, env_bytes, expected_line, b_id) =
        send_once(&manager, &registry, &messaging, from, body_mb_ok).await;
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "a multibyte body with bytes ≤ 64Ki must be delivered (the boundary is judged in bytes): {v}"
    );
    assert_eq!(
        obs.expect("observation").bytes_requested,
        env_bytes,
        "multibyte straddle: request bytes = the exact UTF-8 length of the envelope (not the char count)"
    );
    assert_eq!(
        written, expected_line,
        "multibyte straddle: the captured write matches the expected encoded envelope byte-exactly (handoff intact)"
    );
    manager.kill_agent(b_id).ok();

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// ── ADR-0088 Stage 1 oracle 3(a) → ADR-0111 update: recipient absent → **a failure row** + no delivery observation ──────────
#[tokio::test]
async fn stage1_lifecycle_recipient_absent_is_a_failed_row_with_no_observation() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("stage1-absent").await;

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "stage1-absent-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let result = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec!["no-such-agent".to_string()],
            body: "hi".to_string(),
            contract: Default::default(),
        },
    );
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "failed",
        "★ADR-0111 decision 1★ an absent recipient is a failure row: {v}"
    );
    assert!(
        seen.lock().unwrap().is_empty(),
        "parking does not inject so there is no delivery observation record (no phantom delivery)"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── S18 messaging v1 acceptance scenario (spec §7): busy parking → turn end → automatic delivery (real claude) ──────────────
// multi_thread runtime: `wait_until` is synchronous polling with std::thread::sleep, so on a current-thread runtime the spinning test task leaves the flush worker no gap to be polled. (Async polling would work on the default runtime, but this helper is shared by many sync tests, so it is isolated by runtime flavor.)
#[ignore = "no input is given so the busy window never comes and the core assertion is skipped every time — the held-mail delivery axis is deterministically covered by c2_busy_recipient_parks_then_batch_flushes_on_turn_end (2026-10-02 · T-46)"]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c1_park_then_spawn_auto_delivers() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, busy) = wire("c1-park-spawn").await;

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c1-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    // ── 1) launch a real agent first ────────────────────────────────────────────────────
    // Setup changed with ADR-0111 decision 1: absence became an entrance refusal (a failure row), so the old "send to an unlaunched name to park, spawn later" path is gone. This test guards the wiring "parked mail is delivered automatically at the flush trigger", so it uses the only remaining parking reason (busy = a turn in progress); the observed target (idle diff → flush worker → injection → observation) is unchanged.
    // Uncovered axis (review C1): the roster appearance diff → flush wiring (MessagingFlushSink watching agent_list_updated and opening that name's queue) is covered by no integration-path test — only both ends are unit-covered (the daemon's `flush_sink_appears_*`, the kernel's `flush_on_appearance_*` call `flush_for` directly). A daemon-level test making the appearance diff by "parking to a live recipient → replacing that agent's epoch (reactivation)" is backlog.
    let target_name = "late-recv";
    let Some((info, _tok)) = spawn_json_agent(&manager, &registry, target_name) else {
        require_claude("c1_park_then_spawn_auto_delivers");
    };

    // ── 2) catch the moment that agent is **mid-turn** (the priming response) ──────────────────────────
    let saw_busy = wait_until(Duration::from_secs(20), || {
        busy.is_busy(info.id, info.epoch)
    });
    if !saw_busy {
        eprintln!(
            "SKIP c1_park_then_spawn_auto_delivers: could not observe the busy window right after spawn (real claude timing) — \
             the busy parking→flush axis is deterministically covered by c2_busy_recipient_parks_then_batch_flushes_on_turn_end"
        );
        manager.kill_agent(info.id).ok();
        let _ = std::fs::remove_dir_all(&data_dir);
        handle.shutdown().await;
        return;
    }

    // ── 3) a send mid-turn → parking (pending). There is no injection so there is no delivery observation either ─────────────────────
    let result = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![target_name.to_string()],
            body: "parked-until-idle".to_string(),
            contract: Default::default(),
        },
    );
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "pending",
        "arriving mid-turn is parked (spec §5 branch 3): {v}"
    );
    assert!(
        seen.lock().unwrap().is_empty(),
        "no delivery observation at parking time (no injection)"
    );

    // ── 4) when the turn ends the idle trigger → flush lane → automatic delivery ─────────────────────────────────
    let delivered = wait_until(Duration::from_secs(30), || {
        seen.lock().unwrap().iter().any(
            |o: &engram_dashboard_messaging::envelope::DeliveryObservation| {
                o.to_name == target_name && o.from.peer_id == sender && o.is_delivered()
            },
        )
    });
    assert!(
        delivered,
        "at turn end (idle transition) the parked item must be delivered automatically (flush → inject → observe): {:?}",
        seen.lock().unwrap()
    );

    manager.kill_agent(info.id).ok();
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── S18 messaging v1 C2 acceptance scenario (spec §7 "stronger batch verification"): arrive while busy → not injected → batch flush at turn end ──
// Proves the whole idle gate span is connected: core.emit (classification) → turn observation table → MessagingService gate (parking) → IdleNotifier → flush channel → flush worker → flush_for_agent → injection.
// Deterministic without claude: turn events are emitted directly into the recipient core, independent of a real claude turn's timing.
// multi_thread runtime: as the c1 test.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_busy_recipient_parks_then_batch_flushes_on_turn_end() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;
    use engram_dashboard_messaging::ledger::DeliveryStatus;

    let (manager, registry, _base, data_dir, handle, messaging, busy) = wire("c2-idle-gate").await;

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c2-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let (b_id, captured, core) = obs_seam::insert_observed_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    // core.emit is the same entry point production's pump uses, so the whole span rides as is.
    let feed = |ev: OutputEvent| core.emit(ev);

    // ── 1) turn start observation (assistant delta) → busy ────────────────────────────────────
    feed(OutputEvent::TextDelta {
        text: "thinking...".to_string(),
        turn_id: None,
        message_id: None,
    });
    assert!(
        busy.is_busy(b_id, 0),
        "after observing a turn event the gate must show busy (emit → core table → gate wiring)"
    );

    // ── 2) 2 sends mid-turn → both pending (injection 0) ───────────────────────────────────────
    let mut ids = Vec::new();
    for body in ["first", "second"] {
        let v = handle_send(
            &manager,
            &registry,
            &messaging,
            Entrance::Cli,
            ControlCommand {
                from,
                to: vec![to_name.clone()],
                body: body.to_string(),
                contract: Default::default(),
            },
        )
        .to_json();
        assert_eq!(
            v["results"][0]["status"], "pending",
            "arriving mid-turn is parked (pending) — no pre-injection into the CLI stdin (ADR-0104): {v}"
        );
        ids.push(v["id"].as_str().expect("msg id").to_string());
    }
    assert!(
        obs_seam::all_written(&captured).is_empty(),
        "nothing is written to the recipient stdin mid-turn"
    );
    assert!(
        seen.lock().unwrap().is_empty(),
        "there is no injection so there is no delivery observation either (no phantom delivered)"
    );
    assert_eq!(messaging.parked_len(b_id), 2, "2 items wait in the mailbox");
    for id in &ids {
        assert_eq!(messaging.ledger_statuses(id), vec![DeliveryStatus::Pending]);
    }

    // ── 3) turn end (MessageDone) → idle trigger → batch flush ─────────────────────────────
    // The idle assertion is made while holding the recipient stdin (flaky regression — CI 2026-10-06): the MessageDone doorbell launches the flush on another thread, and the claude classifier counts that first injection's input-time user echo (`Structured`) as progress, so the recipient is
    //   mid-turn again at once — intended (injection = a new turn; the `drain_queue` mid-batch re-check prohibition). Asking idle after the doorbell races the flush, and that idle cannot be waited for (it does not return until the next MessageDone or the turn-cap fail-open).
    //   Holding the seam's record lock makes the flush stand at `send_input` — the session emits the echo after `send_input` succeeds (`AgentSession::write_now`), so the busy decision here reflects only the MessageDone.
    {
        let _stdin = captured.lock().unwrap();
        feed(OutputEvent::MessageDone {
            turn_id: None,
            message_id: None,
        });
        assert!(!busy.is_busy(b_id, 0), "MessageDone → idle");
    }
    let flushed = wait_until(Duration::from_secs(10), || {
        obs_seam::all_written(&captured).len() == 2
    });
    let written = obs_seam::all_written(&captured);
    assert!(
        flushed,
        "at turn end the parked items must be injected **as one batch** (idle gate flush): actual {} items",
        written.len()
    );
    let first = String::from_utf8_lossy(&written[0]).to_string();
    let second = String::from_utf8_lossy(&written[1]).to_string();
    assert!(
        first.contains("first") && !first.contains("second"),
        "the first batch injection = the oldest message (an individual envelope): {first}"
    );
    assert!(
        second.contains("second"),
        "the second batch injection = the next message: {second}"
    );
    // The ledger becomes delivered only at the actual injection moment (ADR-0104).
    for id in &ids {
        assert_eq!(
            messaging.ledger_statuses(id),
            vec![DeliveryStatus::Delivered],
            "pending→delivered transition after the flush injection"
        );
    }
    assert_eq!(
        messaging.parked_len(b_id),
        0,
        "the queue is emptied after the flush"
    );
    assert_eq!(
        seen.lock()
            .unwrap()
            .iter()
            .filter(|o| o.to_name == to_name && o.is_delivered())
            .count(),
        2,
        "2 delivery observations (each item of the batch)"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── C2 review fix 1: do not bootstrap busy from a resumed transcript ──────────────────────────────
// Blocked regression (critical): a resume spawn seeds the transcript into the ring (core ADR-0079 seed-before-pump); a transcript cut mid-turn ends with a progress signal and no end signal, and feeding it as an observation marks (id, epoch) mid-turn with an end that never comes — every send to it parks until the TTL (a false-busy that cannot wake = delivery stops).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_a_resumed_transcript_never_bootstraps_busy() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, busy) = wire("c2-seed").await;

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c2-seed-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let (b_id, captured, core) = obs_seam::insert_observed_seam_recipient(&manager, false);
    let to_name = obs_seam::fallback_name(b_id);

    // The seed fixture = the transcript of a dead incarnation ("a past that ended mid-turn" — only a progress signal, no end signal).
    core.seed(vec![OutputEvent::TextDelta {
        text: "past mid-turn".to_string(),
        turn_id: None,
        message_id: None,
    }]);

    assert!(
        !busy.is_busy(b_id, 0),
        "if busy were bootstrapped from a resumed transcript that busy could not be woken (delivery stops until the TTL)"
    );

    let written_before = obs_seam::all_written(&captured).len();
    let v = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![to_name.clone()],
            body: "hello".to_string(),
            contract: Default::default(),
        },
    )
    .to_json();
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "with no bootstrap it is delivered immediately: {v}"
    );
    assert_eq!(messaging.parked_len(b_id), 0, "parking 0");

    assert!(
        obs_seam::all_written(&captured).len() > written_before,
        "the injection must have actually happened (premise)"
    );
    assert!(
        busy.is_busy(b_id, 0),
        "a live emit is observed — pins that excluding the seed does not mean 'sees nothing'"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── C2: a recipient that died mid-turn leaves no phantom busy (the whole daemon span, no claude needed) ─────────────────
// Daemon level because unit tests see only the core: whether the adapter and gate (manager table → ManagerTurnFacts → BusyPolicy) see the same table as the core is caught only here.
// Blocked regression: if the "mid-turn" of an incarnation that died mid-turn remains, (1) parking to that name is never released and (2) the cap sweep rings the doorbell to the dead agent every 60 seconds for the whole process lifetime.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_a_recipient_that_dies_mid_turn_leaves_no_busy_ghost() {
    use engram_dashboard_agent::types::TerminalReason;

    let (manager, _registry, _base, data_dir, handle, _messaging, busy) = wire("c2-ghost").await;
    let (b_id, _captured, core) = obs_seam::insert_observed_seam_recipient(&manager, false);

    core.emit(OutputEvent::TextDelta {
        text: "thinking...".to_string(),
        turn_id: None,
        message_id: None,
    });
    assert!(
        busy.is_busy(b_id, 0),
        "premise: the gate sees it as mid-turn"
    );

    // An abnormal exit with no turn-end signal = exactly the shape the cap sweep originally handled.
    core.finish(TerminalReason::Killed);
    assert!(
        !busy.is_busy(b_id, 0),
        "a terminated incarnation must vanish from the gate immediately (without waiting for the 30-minute cap)"
    );

    // The injection thread that was blocked emits the input echo late = a phantom revive attempt (same epoch).
    core.emit(OutputEvent::Structured {
        kind: "user".to_string(),
        json: "{}".to_string(),
    });
    assert!(
        !busy.is_busy(b_id, 0),
        "a late echo after termination must not revive the phantom busy"
    );
    assert!(
        manager
            .turns()
            .in_turn_snapshot()
            .iter()
            .all(|(id, _, _)| *id != b_id),
        "it must not remain in the list the cap sweep scans either (prevents repeated doorbells to a dead agent)"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── C2 real-path axis (claude-gated): is a send during a real claude turn parked ────────────────────────────────
// Adds: busy observation from events produced by the real claude decoder (not a synthetic emit) — measures whether the "capability proxy (structured = turn events exist)" really holds.
// Asymmetric assertions: "a send mid-turn → pending" is a hard assert; delivery after the turn ends needs claude to emit a `result` line (authentication · network · model latency) and is reported as a failure if it does not arrive within 90s (module header).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c2_live_mid_turn_send_parks_and_delivers_after_turn_end() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, busy) = wire("c2-live").await;

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c2-live-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let target = "c2-live-recv";
    let Some((info, _tok)) = spawn_json_agent(&manager, &registry, target) else {
        require_claude("c2_live_mid_turn_send_parks_and_delivers_after_turn_end");
    };

    // Before input claude is quiet — system/init is passed by the decoder. This assertion alone is weak (indistinguishable from "no gate at all"); the evidence that the observation wiring is alive is the `is_busy` hard assert after the first injection.
    assert!(
        wait_until(Duration::from_secs(5), || !busy
            .is_busy(info.id, info.epoch)),
        "before input the recipient must be observed as idle (no turn events)"
    );

    let v1 = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![target.to_string()],
            body: "say OK".to_string(),
            contract: Default::default(),
        },
    )
    .to_json();
    assert_eq!(
        v1["results"][0]["status"], "delivered",
        "an immediate injection to an idle recipient: {v1}"
    );
    assert!(
        busy.is_busy(info.id, info.epoch),
        "injection = turn start — the input-time user echo (Structured) must enter the table through the real emit path"
    );

    let v2 = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![target.to_string()],
            body: "and then say DONE".to_string(),
            contract: Default::default(),
        },
    )
    .to_json();
    assert_eq!(
        v2["results"][0]["status"], "pending",
        "arriving during a real claude turn is parked: {v2}"
    );
    let msg2 = v2["id"].as_str().expect("msg id").to_string();
    assert_eq!(
        messaging.parked_len(info.id),
        1,
        "the item that arrived mid-turn waits"
    );

    let delivered = wait_until(Duration::from_secs(90), || {
        seen.lock().unwrap().iter().any(
            |o: &engram_dashboard_messaging::envelope::DeliveryObservation| {
                o.msg_id == msg2 && o.is_delivered()
            },
        )
    });
    assert!(
        delivered,
        "[c2_live] a parked delivery was not observed within 90s after the turn ended (result) (parked={}) — \
         a turn-event observation/idle flush regression or real claude response latency",
        messaging.parked_len(info.id)
    );
    assert_eq!(
        messaging.parked_len(info.id),
        0,
        "the queue is emptied by the turn-end flush"
    );

    manager.kill_agent(info.id).ok();
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// ── ADR-0088 Stage 1 oracle 4 (write failure): the failure observation shape — a single failure record (no partial/duplicate observation) ──
/// Does not prove (coverage gaps): partial delivery where a real OS write emits an Err after writing a prefix — in this seam send_input returns an Err wholesale before the push (atomic all-or-nothing), so "only a prefix written, then fails" never arises. ▶ `physical_pipe_partial_write_then_err_surfaces_as_err` in the agent crate's `tests/stdio_physical_pipe.rs`.
#[tokio::test]
async fn stage1_lifecycle_write_error_single_failure_no_partial_dup() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("stage1-write-err").await;

    // fail=true → a live recipient on the roster but send_input is an Err.
    let (b_id, captured) = obs_seam::insert_seam_recipient(&manager, true);
    let to_name = obs_seam::fallback_name(b_id);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "stage1-write-err-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let result = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![to_name.clone()],
            body: "will-fail-once".to_string(),
            contract: Default::default(),
        },
    );
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "pending",
        "a write failure is parking (pending): {v}"
    );

    let g = seen.lock().unwrap();
    assert_eq!(
        g.len(),
        1,
        "a failure also gives exactly 1 observation record (no partial/duplicate): {:?}",
        *g
    );
    let obs = &g[0];
    assert!(obs.error.is_some(), "failure = error Some");
    assert_eq!(
        obs.bytes_written, None,
        "failure = bytes_written None (no success field leaks)"
    );
    assert_eq!(obs.msg_uuid, None, "failure = msg_uuid None");
    assert_eq!(
        obs.to_epoch, None,
        "failure = to_epoch None (no complete write → no landed incarnation to attest, no success field leaks)"
    );
    assert!(!obs.is_delivered(), "failure = !is_delivered()");
    assert!(
        obs_seam::all_written(&captured).is_empty(),
        "on a write failure no bytes must be stuck into the recipient (capture 0)"
    );

    drop(g);
    manager.kill_agent(b_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// ── ADR-0088 Stage 1 oracle 5 (epoch): sequential incarnation replacement semantics — delivered to the current incarnation ──────
/// Design intent: mail is addressed to the logical agent (a stable address), so no epoch pinning (ADR-0086 §F5).
#[tokio::test]
async fn stage1_lifecycle_epoch_rotation_delivers_to_current_incarnation() {
    use engram_dashboard_agent::backend::InputEncoder;
    use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
    use engram_dashboard_agent::session::AgentSession;
    use engram_dashboard_agent::transport::AgentTransport;
    use engram_dashboard_agent::types::{
        AgentId as CoreAgentId, AgentStatus, BackendCaps, ControlCaps, InputCaps, InputEvent,
        ModelCaps, OutputCaps, PtyError, SessionCaps, StatusSink, TransportCaps,
    };
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;
    use std::sync::atomic::AtomicU8;

    // Local seam transport — as obs_seam's, but different capture buffers per epoch are needed to tell incarnations apart, so it is inline (same AgentId, different buffer).
    struct NoopStatus;
    impl StatusSink for NoopStatus {
        fn status_changed(&self, _id: CoreAgentId, _s: AgentStatus, _e: u32) {}
        fn agent_list_updated(&self, _a: Vec<engram_dashboard_agent::types::AgentInfo>) {}
    }
    struct EpochSeam {
        captured: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl AgentTransport for EpochSeam {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            let InputEvent::Raw(bytes) = input;
            self.captured.lock().unwrap().push(bytes);
            Ok(())
        }
        fn resize(&self, _c: u16, _r: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            TransportCaps {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: false,
                    structured: true,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: false,
                    interrupt: false,
                    cancel: false,
                    graceful_shutdown: false,
                },
            }
        }
    }
    fn backend_caps() -> BackendCaps {
        BackendCaps {
            session: SessionCaps {
                resume: true,
                snapshot: false,
                cwd_env: true,
            },
            model: ModelCaps {
                select: false,
                temperature: false,
                max_tokens: false,
            },
        }
    }
    fn insert_epoch(
        manager: &Arc<AgentManager>,
        id: CoreAgentId,
        epoch: u32,
    ) -> Arc<Mutex<Vec<Vec<u8>>>> {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let core = Arc::new(OutputCore::new(
            id,
            epoch,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        let cwd = std::path::PathBuf::from(format!("seam-root/{}", &id.to_string()[..8]));
        let session = Arc::new(AgentSession::new(
            id,
            cwd,
            epoch,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            backend_caps(),
            InputEncoder::ClaudeStreamJson,
            true,
            core,
            Box::new(EpochSeam {
                captured: captured.clone(),
            }),
        ));
        manager.insert_test_session(session);
        captured
    }

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("stage1-epoch").await;

    let id = CoreAgentId::new_v4();
    let to_name = obs_seam::fallback_name(id);

    let old_buf = insert_epoch(&manager, id, 0);
    // Replacement injection with the same AgentId = a restart (new incarnation marker) — `insert_test_session` overwrites the same id, so only B (marker 1) remains. The order of the two marker values has no meaning (random — `AgentProfile::epoch`).
    let new_buf = insert_epoch(&manager, id, 1);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "stage1-epoch-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let result = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![to_name],
            body: "to-current-incarnation".to_string(),
            contract: Default::default(),
        },
    );
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "must be delivered to the replaced current incarnation (no loss, ADR-0086 §F5): {v}"
    );

    let g = seen.lock().unwrap();
    assert_eq!(
        g.len(),
        1,
        "1 logical message → 1 observation record (no wrong-epoch double delivery): {:?}",
        *g
    );
    assert_eq!(
        g[0].to_id, id,
        "the record's recipient = that stable AgentId"
    );
    assert_eq!(
        g[0].to_epoch,
        Some(1),
        "the record's to_epoch = the landed current incarnation (epoch 1) — a direct assertion"
    );
    drop(g);

    assert_eq!(
        new_buf.lock().unwrap().len(),
        1,
        "the current incarnation (epoch 1) must receive the bytes"
    );
    assert!(
        old_buf.lock().unwrap().is_empty(),
        "the replaced old incarnation (epoch 0) must not receive bytes (no wrong-epoch delivery)"
    );
    assert!(
        String::from_utf8_lossy(&new_buf.lock().unwrap()[0]).contains("to-current-incarnation"),
        "the envelope body must be fully contained in the current incarnation buffer"
    );

    manager.kill_agent(id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// ── ADR-0088 Stage 1 oracle (mid-flight epoch race): reproducing the deterministic resolve↔write race ──────────────
/// Proves: if right after handle_send resolves the recipient (a list_agents snapshot) and right before injecting, the same AgentId is replaced by an incarnation with a new marker (a restart), the write lands on the incarnation (marker 1) at write-resolution time, not the old one (marker 0) resolve saw.
/// (Strictly, one more concurrent replacement after get_session may land bytes on the incarnation that became current in between — to_epoch still carries the incarnation actually landed on, so record-self-sufficiency holds.) This is a true race between resolve↔write, not a sequential replacement (oracle 5): the production yield-seam
/// (ControlRegistry::set_mid_send_hook, feature=test-harness) fires a hook right before the write and intervenes deterministically. ADR-0086 §F5 = design-accepted: landing on the new incarnation is correct (no loss); proven directly by the record's `to_epoch == Some(1)`.
///
/// Does not prove: the real StdioTransport/real claude do not intervene (EpochSeam only captures the write), and scheduling races other than that injected point (e.g. reachability↔write, physical OS-pipe interleaving) are not covered.
#[tokio::test]
async fn stage1_lifecycle_mid_flight_epoch_race_lands_on_new_incarnation_deterministic() {
    use engram_dashboard_agent::backend::InputEncoder;
    use engram_dashboard_agent::output_core::{OutputCore, TurnWiring};
    use engram_dashboard_agent::session::AgentSession;
    use engram_dashboard_agent::transport::AgentTransport;
    use engram_dashboard_agent::types::{
        AgentId as CoreAgentId, AgentStatus, BackendCaps, ControlCaps, InputCaps, InputEvent,
        ModelCaps, OutputCaps, PtyError, SessionCaps, StatusSink, TransportCaps,
    };
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

    struct NoopStatus;
    impl StatusSink for NoopStatus {
        fn status_changed(&self, _id: CoreAgentId, _s: AgentStatus, _e: u32) {}
        fn agent_list_updated(&self, _a: Vec<engram_dashboard_agent::types::AgentInfo>) {}
    }
    struct EpochSeam {
        captured: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl AgentTransport for EpochSeam {
        fn start(&self, _core: Arc<OutputCore>) {}
        fn send_input(&self, input: InputEvent) -> Result<(), PtyError> {
            let InputEvent::Raw(bytes) = input;
            self.captured.lock().unwrap().push(bytes);
            Ok(())
        }
        fn resize(&self, _c: u16, _r: u16) -> Result<(), PtyError> {
            Ok(())
        }
        fn interrupt(&self) -> Result<(), PtyError> {
            Ok(())
        }
        fn shutdown(&self) {}
        fn capabilities(&self) -> TransportCaps {
            TransportCaps {
                input: InputCaps {
                    raw: true,
                    message: false,
                    attachment: false,
                },
                output: OutputCaps {
                    terminal_bytes: false,
                    structured: true,
                    markdown: false,
                    tool_events: false,
                    usage: false,
                },
                control: ControlCaps {
                    resize: false,
                    interrupt: false,
                    cancel: false,
                    graceful_shutdown: false,
                },
            }
        }
    }
    fn backend_caps() -> BackendCaps {
        BackendCaps {
            session: SessionCaps {
                resume: true,
                snapshot: false,
                cwd_env: true,
            },
            model: ModelCaps {
                select: false,
                temperature: false,
                max_tokens: false,
            },
        }
    }
    fn insert_epoch(
        manager: &Arc<AgentManager>,
        id: CoreAgentId,
        epoch: u32,
    ) -> Arc<Mutex<Vec<Vec<u8>>>> {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let core = Arc::new(OutputCore::new(
            id,
            epoch,
            Arc::new(NoopStatus),
            TurnWiring::detached(),
        ));
        let cwd = std::path::PathBuf::from(format!("seam-root/{}", &id.to_string()[..8]));
        let session = Arc::new(AgentSession::new(
            id,
            cwd,
            epoch,
            80,
            24,
            Arc::new(AtomicU8::new(0)),
            backend_caps(),
            InputEncoder::ClaudeStreamJson,
            true,
            core,
            Box::new(EpochSeam {
                captured: captured.clone(),
            }),
        ));
        manager.insert_test_session(session);
        captured
    }

    let (manager, registry, _base, data_dir, handle, messaging, _busy) =
        wire("stage1-midflight").await;

    let id = CoreAgentId::new_v4();
    let to_name = obs_seam::fallback_name(id);

    let old_buf = insert_epoch(&manager, id, 0);

    let seen = Arc::new(Mutex::new(Vec::new()));
    registry.set_delivery_observer(Arc::new(DeliveryCapture { seen: seen.clone() }));

    let new_buf_slot: Arc<Mutex<Option<Arc<Mutex<Vec<Vec<u8>>>>>>> = Arc::new(Mutex::new(None));
    let rotated = Arc::new(AtomicBool::new(false));
    // Cutting the Arc cycle: the registry stores the hook (a closure) and is transitively owned by the manager-side wiring; a hook holding the manager strongly forms a cycle, so an assertion panic before cleanup (set_mid_send_hook(None)) would leak the manager and reaper thread for the process lifetime. It is held as a Weak and upgraded when fired.
    let mgr_weak = Arc::downgrade(&manager);
    let slot_for_hook = new_buf_slot.clone();
    let rotated_for_hook = rotated.clone();
    registry.set_mid_send_hook(Some(Arc::new(move || {
        // compare_exchange: rotate exactly once (guards re-firing).
        if rotated_for_hook
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let Some(mgr_for_hook) = mgr_weak.upgrade() else {
            return;
        };
        let new_buf = insert_epoch(&mgr_for_hook, id, 1);
        *slot_for_hook.lock().unwrap() = Some(new_buf);
    })));

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "stage1-midflight-sender".to_string(), true);
    let from = BoundIdentity {
        agent_id: sender,
        epoch: 0,
    };

    let result = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from,
            to: vec![to_name],
            body: "mid-flight-race-body".to_string(),
            contract: Default::default(),
        },
    );
    let v = result.to_json();
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "must be delivered to the current incarnation even after a mid-flight replacement (no loss, ADR-0086 §F5): {v}"
    );
    let ack_id = v["id"].as_str().expect("msg-id is attached").to_string();

    let obs = {
        let g = seen.lock().unwrap();
        assert_eq!(
            g.len(),
            1,
            "1 logical message → 1 observation record (no mid-flight double delivery): {:?}",
            *g
        );
        g[0].clone()
    };
    assert!(obs.is_delivered(), "is_delivered() = true (fully accepted)");
    assert_eq!(obs.to_id, id, "the record's recipient = the stable AgentId");
    assert_eq!(
        obs.to_epoch,
        Some(1),
        "the write lands on the replaced current incarnation (epoch 1) — not the resolve-time one (epoch 0). to_epoch={:?}",
        obs.to_epoch
    );
    assert_eq!(
        obs.msg_id, ack_id,
        "the record msg_id = ACK id (correlation axis 1)"
    );
    assert!(
        obs.msg_uuid.is_some(),
        "a successful delivery must carry msg_uuid (correlation axis 2)"
    );

    let new_buf = new_buf_slot
        .lock()
        .unwrap()
        .clone()
        .expect("the hook must have replace-injected the epoch 1 incarnation");
    assert_eq!(
        new_buf.lock().unwrap().len(),
        1,
        "the current incarnation (epoch 1) must receive the bytes"
    );
    assert!(
        old_buf.lock().unwrap().is_empty(),
        "the old incarnation (epoch 0) the resolve saw must not receive bytes (a mid-flight landing goes to current)"
    );
    assert!(
        String::from_utf8_lossy(&new_buf.lock().unwrap()[0]).contains("mid-flight-race-body"),
        "the envelope body must be fully contained in the current incarnation buffer"
    );

    registry.set_mid_send_hook(None);
    manager.kill_agent(id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// The rmcp client builds a reqwest 0.13 client, and the daemon's graph turns on reqwest's `rustls-no-provider` (through transport's
///   `iroh`), so building one without an installed rustls provider panics. Installs ring's for the process; a later call answers `Err`
///   (already installed), which is the state wanted.
fn install_tls_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

// ── MCP eg_send tool: happy path + corrective errors (rmcp client) ─────────────────────────────
#[tokio::test]
async fn mcp_send_message_tool_happy_and_error() {
    use rmcp::model::CallToolRequestParams;
    use rmcp::transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    };
    use rmcp::ServiceExt;

    let (manager, registry, _base, data_dir, handle, _messaging, _busy) = wire("mcp-tool").await;

    let Some((b_info, _b_tok)) = spawn_json_agent(&manager, &registry, "recv") else {
        require_claude("mcp_send_message_tool_happy_and_error");
    };

    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "mcp-sender-tok".to_string(), true);

    install_tls_provider();
    let config = StreamableHttpClientTransportConfig::with_uri(handle.url.clone())
        .auth_header("mcp-sender-tok");
    let transport = StreamableHttpClientTransport::from_config(config);
    let client = ().serve(transport).await.expect("MCP handshake");

    let tools = client.list_all_tools().await.expect("list tools");
    assert!(
        tools.iter().any(|t| t.name == "eg_send"),
        "eg_send in tools: {:?}",
        tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>()
    );

    let mut params = CallToolRequestParams::default();
    params.name = "eg_send".into();
    params.arguments = Some(
        serde_json::json!({ "to": "recv", "body": "mcp-hello" })
            .as_object()
            .unwrap()
            .clone(),
    );
    let result = client.call_tool(params).await.expect("call eg_send");
    let text = result
        .content
        .iter()
        .find_map(|c| c.as_text().map(|t| t.text.clone()))
        .expect("text content");
    let v: serde_json::Value = serde_json::from_str(&text).expect("ACK json");
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "MCP send happy path: {text}"
    );
    assert_eq!(v["results"][0]["to"], "recv");

    let mut params = CallToolRequestParams::default();
    params.name = "eg_send".into();
    params.arguments = Some(
        serde_json::json!({ "to": "ghost", "body": "x" })
            .as_object()
            .unwrap()
            .clone(),
    );
    let result = client.call_tool(params).await.expect("call eg_send err");
    let text = result
        .content
        .iter()
        .find_map(|c| c.as_text().map(|t| t.text.clone()))
        .expect("text content");
    let v: serde_json::Value = serde_json::from_str(&text).expect("ack json");
    assert_eq!(
        v["results"][0]["status"], "failed",
        "★ADR-0111 decision 1★ a nonexistent recipient is a failure row at the MCP entrance too: {text}"
    );

    let _ = client.cancel().await;
    manager.kill_agent(b_info.id).ok();
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── C3 (ADR-0103): reply contract — request→reply→replied transition + overdue notice (seam, no claude needed) ────────
//
// Seam because contract transitions and notice injection verify plumbing, and depending on model responses is flaky; a structured-carrier seam recipient is plugged in and the injected bytes accounted directly.
// Deterministic clock: sweep takes `now` as an argument (purity invariant), so a past-deadline time is pushed in by hand with no real waiting.

async fn post_send_json(
    base: &str,
    bearer: Option<&str>,
    body: serde_json::Value,
) -> (reqwest::StatusCode, String) {
    post_control(base, "/control/send", bearer, body).await
}

async fn post_control(
    base: &str,
    route: &str,
    bearer: Option<&str>,
    body: serde_json::Value,
) -> (reqwest::StatusCode, String) {
    let client = reqwest::Client::new();
    let mut req = client
        .post(format!("{base}{route}"))
        .header("Content-Type", "application/json")
        .json(&body);
    if let Some(b) = bearer {
        req = req.header("Authorization", format!("Bearer {b}"));
    }
    let resp = req.send().await.expect("http request");
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    (status, text)
}

#[tokio::test]
async fn c3_request_reply_roundtrip_transitions_ledger_to_replied() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, SendContract};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;
    use engram_dashboard_messaging::ledger::DeliveryStatus;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("c3-roundtrip").await;

    // A = the requester (both sender and reply recipient), B = the replier. Both are structured seams (reachable).
    let (a_id, a_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let (b_id, b_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let a_name = obs_seam::fallback_name(a_id);
    let b_name = obs_seam::fallback_name(b_id);
    registry.issue(a_id, 0, "c3-a".to_string(), true);
    registry.issue(b_id, 0, "c3-b".to_string(), true);
    let from_a = BoundIdentity {
        agent_id: a_id,
        epoch: 0,
    };
    let from_b = BoundIdentity {
        agent_id: b_id,
        epoch: 0,
    };

    // 1) A → B: request (+ deadline). The envelope must carry id/type/reply-by so that B knows the id to reply with.
    let ack = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Mcp,
        ControlCommand {
            from: from_a,
            to: vec![b_name.clone()],
            body: "Write the code and reply".to_string(),
            contract: SendContract {
                request: true,
                reply_by: Some("10m".to_string()),
                reply_to: None,
            },
        },
    )
    .to_json();
    assert_eq!(
        ack["results"][0]["status"], "delivered",
        "request ACK: {ack}"
    );
    let req_id = ack["id"].as_str().expect("msg id").to_string();
    assert!(
        req_id.starts_with("m-") && req_id.len() == 10,
        "the wire id is a short base36 contract (spec §1): {req_id}"
    );
    assert_eq!(
        messaging.open_request_count(),
        1,
        "the contract must open in the ledger (spec §3 step 2)"
    );

    // The bytes B actually received must contain the request attributes (inside the stream-json encoding, so a substring check).
    let b_line = String::from_utf8_lossy(&obs_seam::last_written(&b_captured)).to_string();
    for needle in ["type=", "request", &req_id, "reply-by", "10m"] {
        assert!(
            b_line.contains(needle),
            "the envelope B received must contain '{needle}': {b_line}"
        );
    }

    // 2) B → A: reply with that id. Strict matching closes the contract and moves the history to Replied.
    let ack = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Cli,
        ControlCommand {
            from: from_b,
            to: vec![a_name.clone()],
            body: "Done coding, tests pass".to_string(),
            contract: SendContract {
                request: false,
                reply_by: None,
                reply_to: Some(req_id.clone()),
            },
        },
    )
    .to_json();
    assert_eq!(ack["results"][0]["status"], "delivered", "reply ACK: {ack}");
    assert!(
        ack.get("status").is_none(),
        "the reply response shape must be the same as a notice (no new field): {ack}"
    );

    assert_eq!(
        messaging.ledger_statuses(&req_id),
        vec![DeliveryStatus::Replied],
        "the request record transitions to replied (spec §3 step 3)"
    );
    assert_eq!(
        messaging.open_request_count(),
        0,
        "the contract must be closed"
    );

    let a_line = String::from_utf8_lossy(&obs_seam::last_written(&a_captured)).to_string();
    assert!(
        a_line.contains("in-reply-to") && a_line.contains(&req_id),
        "the reply envelope A received must carry in-reply-to: {a_line}"
    );

    // 3) Even after the deadline passes there is no notice (a replied contract is not a due target).
    messaging.sweep(Instant::now() + Duration::from_secs(3600));
    let a_line = String::from_utf8_lossy(&obs_seam::last_written(&a_captured)).to_string();
    assert!(
        !a_line.contains("<notice>"),
        "a replied contract must have no timeout notice: {a_line}"
    );

    manager.kill_agent(a_id).ok();
    manager.kill_agent(b_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn c3_reply_by_timeout_injects_notice_to_the_sender() {
    use engram_dashboard_daemon::control::ingress::{handle_send, ControlCommand, SendContract};
    use engram_dashboard_daemon::control::registry::BoundIdentity;
    use engram_dashboard_messaging::envelope::Entrance;

    let (manager, registry, _base, data_dir, handle, messaging, _busy) = wire("c3-timeout").await;

    // A = the requester (live · reachable — receives the notice). B = an accepted recipient (live · reachable) that does not answer.
    // Setup changed with ADR-0111 decision 1: absence is now a failure row and no contract opens either (no request without tracking); the overdue notice presupposes a contract opened to an accepted recipient, so a live recipient is used and only the missing reply is kept.
    let (a_id, a_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let (b_id, _b_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let silent_worker = obs_seam::fallback_name(b_id);
    registry.issue(a_id, 0, "c3-timeout-a".to_string(), true);
    let from_a = BoundIdentity {
        agent_id: a_id,
        epoch: 0,
    };

    let ack = handle_send(
        &manager,
        &registry,
        &messaging,
        Entrance::Mcp,
        ControlCommand {
            from: from_a,
            to: vec![silent_worker.clone()],
            body: "Do it".to_string(),
            contract: SendContract {
                request: true,
                reply_by: Some("1m".to_string()),
                reply_to: None,
            },
        },
    )
    .to_json();
    assert_eq!(
        ack["results"][0]["status"], "delivered",
        "delivered to an accepted recipient — that recipient's contract opens (spec §3): {ack}"
    );
    let req_id = ack["id"].as_str().expect("msg id").to_string();
    assert_eq!(
        messaging.open_request_count(),
        1,
        "1 contract (1 recipient)"
    );
    assert!(
        obs_seam::all_written(&a_captured).is_empty(),
        "nothing has been injected into A (the requester) yet"
    );
    let _ = b_id;

    messaging.sweep(Instant::now() + Duration::from_secs(61));

    assert!(
        wait_until(Duration::from_secs(5), || !obs_seam::all_written(
            &a_captured
        )
        .is_empty()),
        "the flush lane must inject the notice into A"
    );
    let a_line = String::from_utf8_lossy(&obs_seam::last_written(&a_captured)).to_string();
    assert!(
        a_line.contains("<notice>"),
        "the overdue notice is a <notice> tag (no from = not a reply target): {a_line}"
    );
    for needle in [req_id.as_str(), "1m", silent_worker.as_str(), "[engram]"] {
        assert!(
            a_line.contains(needle),
            "the notice wording must contain '{needle}' (spec §1 template): {a_line}"
        );
    }
    assert!(
        !a_line.contains("<message"),
        "the notice must not leak as <message> (mistaken as repliable): {a_line}"
    );

    // An asynchronous lane, so "does not grow" is meaningful only after waiting a moment (checking at once could misjudge not-yet-arrived as not arrived — false-green).
    let before = obs_seam::all_written(&a_captured).len();
    messaging.sweep(Instant::now() + Duration::from_secs(120));
    assert!(
        !wait_until(Duration::from_millis(500), || obs_seam::all_written(
            &a_captured
        )
        .len()
            > before),
        "the notice is exactly once (the second sweep sends nothing)"
    );

    manager.kill_agent(a_id).ok();
    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

#[tokio::test]
async fn c3_invalid_contract_args_are_rejected_identically_at_the_cli_entrance() {
    // The rejection code/shape of the two entrances must be the same (entrance-agnostic — ADR-0086); checked through the HTTP entrance.
    let (_m, registry, base, data_dir, handle, _messaging, _busy) = wire("c3-args").await;
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c3-args-sender".to_string(), true);
    let tok = Some("c3-args-sender");

    // Mutually exclusive.
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": "nobody", "body": "x", "request": true, "reply_to": "m-1" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["code"], "INVALID_SEND_ARGS", "request+reply_to: {body}");
    assert!(v["hint"].is_string(), "the corrective hint is attached");

    // `reply_to` takes exactly 1 recipient — blocked at the notation stage (spec §3 item 7-① · ADR-0111). 1) multiple recipients + reply_to.
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": ["a", "b"], "body": "x", "reply_to": "m-1" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["code"], "INVALID_SEND_ARGS",
        "reply_to + multiple recipients: {body}"
    );
    //   2) with an `@` token — refused even if the expansion is 1 person (an answer split by roster state could not be learned).
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": ["@all"], "body": "x", "reply_to": "m-1" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["code"], "INVALID_SEND_ARGS",
        "reply_to + an @ address is refused at the notation stage (before expansion): {body}"
    );
    //   3) control — 1 recipient + reply_to passes (a row-only failure because of absence).
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": ["nobody"], "body": "x", "reply_to": "m-1" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert!(
        v.get("code").is_none(),
        "a single-recipient reply is not an argument refusal: {body}"
    );
    assert_eq!(v["results"][0]["status"], "failed", "{body}");

    // An empty recipient list = INVALID_SEND_ARGS (spec §6) — both an empty array and an empty string.
    for empty in [serde_json::json!([]), serde_json::json!("")] {
        let (_s, body) =
            post_send_json(&base, tok, serde_json::json!({ "to": empty, "body": "x" })).await;
        let v: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(
            v["code"], "INVALID_SEND_ARGS",
            "an empty recipient list: {body}"
        );
    }

    // reply_by alone.
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": "nobody", "body": "x", "reply_by": "10m" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["code"], "INVALID_SEND_ARGS", "reply_by alone: {body}");

    // Notation error.
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": "nobody", "body": "x", "request": true, "reply_by": "ten min" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["code"], "INVALID_SEND_ARGS",
        "a duration notation error: {body}"
    );

    // A deadline under 1 minute (review fix 7).
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": "nobody", "body": "x", "request": true, "reply_by": "30s" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["code"], "INVALID_SEND_ARGS",
        "a deadline under 1 minute: {body}"
    );
    assert!(
        v["hint"].as_str().unwrap_or_default().contains("1-minute"),
        "the hint must state the lower bound: {body}"
    );
    // Control: exactly 1 minute (in seconds notation) is accepted.
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": "nobody", "body": "x", "request": true, "reply_by": "60s" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["results"][0]["status"], "failed",
        "60s is a valid notation (the row fails because the recipient is absent but it is not INVALID_SEND_ARGS): {body}"
    );

    // An `@` address request is now allowed (ADR-0111 decision 5 — the old "v1 permanent ban" abolished): the refusal below is "there is no address `@coders`" (address-space error = whole refusal, ADR-0114 decision 3), not "because it is a request".
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": "@coders", "body": "x", "request": true }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    // `GROUP_REQUEST_UNSUPPORTED` was abolished (ADR-0111 decision 5); the only remaining refusal is the address-space error — user-defined groups are gone, so `@coders` does not exist.
    assert_eq!(
        v["code"], "GROUP_NOT_FOUND",
        "a nonexistent @ address: {body}"
    );

    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": "@coders", "body": "x" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["code"], "GROUP_NOT_FOUND",
        "a group notice (unregistered): {body}"
    );

    // The old `{to, body}` body must still work (notice wire compatibility).
    let (status, body) = post_send(&base, tok, "nobody", "hi").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["results"][0]["status"], "failed",
        "notice regression (a row-only failure because the recipient is absent — not an argument refusal): {body}"
    );
    let id = v["id"].as_str().expect("id");
    assert!(id.starts_with("m-") && id.len() == 10, "id format: {id}");

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// C3 review fix 1: the contract fields (request/reply_to) are XML-envelope only — refused in the colon format.
//   Integration, because the decision logic is covered by the ingress unit tests in both branches (format · template env); this checks the real entrance rides that decision (a runtime format switch leads to a send refusal).
#[tokio::test]
async fn c3_contract_fields_are_rejected_while_the_colon_envelope_is_active() {
    use engram_dashboard_messaging::envelope::EnvelopeFormat;

    let (_m, registry, base, data_dir, handle, _messaging, _busy) = wire("c3-colon").await;
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c3-colon-sender".to_string(), true);
    let tok = Some("c3-colon-sender");

    registry.set_envelope_format(EnvelopeFormat::Colon);

    for payload in [
        serde_json::json!({ "to": "nobody", "body": "x", "request": true, "reply_by": "10m" }),
        serde_json::json!({ "to": "nobody", "body": "x", "reply_to": "m-7f3k" }),
    ] {
        let (_s, body) = post_send_json(&base, tok, payload.clone()).await;
        let v: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(
            v["code"], "INVALID_SEND_ARGS",
            "contract fields refused in the colon envelope ({payload}): {body}"
        );
        assert!(
            v["hint"]
                .as_str()
                .unwrap_or_default()
                .contains("XML envelope"),
            "the corrective hint must state the reason: {body}"
        );
    }

    // Control ①: even in the same colon format a **notice** is accepted normally (existing behavior unchanged).
    let (status, body) = post_send(&base, tok, "nobody", "hi").await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["results"][0]["status"], "failed",
        "a notice is not caught by the format refusal (a row-only failure because the recipient is absent): {body}"
    );

    // Control ②: reverting to xml accepts the contract send again (the refusal was tied only to the format).
    registry.set_envelope_format(EnvelopeFormat::Xml);
    let (_s, body) = post_send_json(
        &base,
        tok,
        serde_json::json!({ "to": "nobody", "body": "x", "request": true, "reply_by": "10m" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["results"][0]["status"], "failed",
        "reverting to xml accepts the contract send (a row-only failure because the recipient is absent — not a format refusal): {body}"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// Extracts only the logical envelope text (`message.content[0].text`) from a line the stream-json carrier wrote to stdin.
///
/// Parsing, not string contains: the carrier wraps the envelope as a JSON string and escapes `"` (`to=\"@all\"`), so looking for `to="@all"` in raw bytes fails even when right; the envelope golden must look at the envelope itself, so one layer is peeled first.
fn stream_json_text(written: &[u8]) -> String {
    let line = String::from_utf8_lossy(written);
    let v: serde_json::Value = serde_json::from_str(&line)
        .unwrap_or_else(|e| panic!("stream-json line parse failed ({e}): {line}"));
    v["message"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no envelope text in the stream-json line: {line}"))
        .to_string()
}

// ── S18 messaging v1 C4 acceptance scenario (spec §4·§6): group fan-out rides the real entrance (HTTP /control/send) ──
// Where it differs from the unit tests: service.rs unit tests cover the fan-out logic; here the entrance wiring — an `@` address descends to the fan-out branch at the HTTP entrance, the per-member result comes out as the spec §6 `results[]` (one line per member) JSON, and a `to="@…"` envelope is written to the real recipient stdin.

#[tokio::test]
async fn c4_all_group_fans_out_to_live_agents_and_excludes_the_sender() {
    let (manager, registry, base, data_dir, handle, _messaging, _busy) = wire("c4-all").await;

    // The sender must also be a live structured agent to demonstrate "excluding itself" (someone in the roster to exclude).
    let (sender_id, sender_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let (a_id, a_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let (b_id, b_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let sender_name = obs_seam::fallback_name(sender_id);
    let a_name = obs_seam::fallback_name(a_id);
    let b_name = obs_seam::fallback_name(b_id);
    registry.issue(sender_id, 0, "c4-all-token".to_string(), true);

    let (status, body) = post_send(
        &base,
        Some("c4-all-token"),
        "@all",
        "Everyone, wait for the rebase",
    )
    .await;
    assert_eq!(
        status,
        reqwest::StatusCode::OK,
        "a broadcast acceptance is also 200: {body}"
    );
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert!(
        v.get("status").is_none(),
        "a success response has no top-level status (spec §6): {body}"
    );

    let results = v["results"].as_array().expect("results array").clone();
    // Also asserts the order (C4 review fix H): `got` is not sorted — production `ManagerDeliveryPort` emits the roster in (name, id) ascending order, so `@all` result order must be name-sorted. Sorting here would hide the determinism (green even in HashMap order) and miss a different injection order per run.
    let got: Vec<(String, String)> = results
        .iter()
        .map(|r| {
            (
                r["to"].as_str().unwrap_or_default().to_string(),
                r["status"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let mut want = vec![
        (a_name.clone(), "delivered".to_string()),
        (b_name.clone(), "delivered".to_string()),
    ];
    want.sort();
    assert_eq!(
        got, want,
        "@all = all live − the sender, each delivered, **in name ascending order** (a deterministic roster — fix H): {body}"
    );
    assert!(
        !results
            .iter()
            .any(|r| r["to"].as_str() == Some(sender_name.as_str())),
        "the sender itself is not on the @all list (no echo of one's own broadcast): {body}"
    );

    for (captured, who) in [(&a_captured, &a_name), (&b_captured, &b_name)] {
        let written = obs_seam::all_written(captured);
        assert_eq!(written.len(), 1, "exactly 1 injection to {who}");
        assert_eq!(
            stream_json_text(&written[0]),
            format!(
                r#"<message from="{sender_name}" to="@all">Everyone, wait for the rebase</message>"#
            ),
            "the broadcast envelope golden — the to attribute must be carried ({who})"
        );
    }
    assert!(
        obs_seam::all_written(&sender_captured).is_empty(),
        "nothing is written to the sender's own stdin"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// Real-entrance test because this was an inconsistency between decisions: the group branch looks at the raw `to` with `starts_with('@')`, group-name normalization at the trimmed value, so `" @all"` flowed to a single send and became "no agent of that name → absent parking" — a `pending` success to the sender, yet nobody receives it and it expires at the TTL (a silent loss behind one space). Only riding the entrance proves both decisions see the same string.
#[tokio::test]
async fn c4_leading_whitespace_in_the_destination_does_not_change_routing() {
    let (manager, registry, base, data_dir, handle, messaging, _busy) = wire("c4-trim").await;

    let (a_id, a_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let a_name = obs_seam::fallback_name(a_id);
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "c4-trim-token".to_string(), true);
    let tok = Some("c4-trim-token");

    // 1) `" @all"` — even with a space it goes to the group branch (per-member accounting, not a single send).
    let (status, body) = post_send(&base, tok, " @all", "Space broadcast").await;
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["results"][0]["to"], a_name.as_str(),
        "results are **member names** — evidence it was routed as a group (if it were a single send ' @all' would be carried as is): {body}"
    );
    assert_eq!(v["results"][0]["status"], "delivered", "{body}");
    assert_eq!(
        messaging.parked_total_for_test(),
        0,
        "no absent parking to a space name (prevents silent loss)"
    );
    assert!(
        !stream_json_text(&obs_seam::last_written(&a_captured)).contains("to="),
        "with 1 accepted recipient there is no to attribute (a letter received alone): {:?}",
        stream_json_text(&obs_seam::last_written(&a_captured))
    );

    // 2) `" <name>"` — the trim of resolution order 1 applies to every element (spec §5 · ADR-0111). The old version left the single-send address byte for byte (round-3 fix 4) so a padded target ended in absent parking; with absent parking abolished that outcome would be a failure row, not a silent loss, and the sender gains nothing.
    //    The spec states the trim in the resolution order, so it is trimmed at the daemon single point and a padded target is delivered to that agent. The response `to` returns the sender's notation (the trimmed token) as is (WYSIWYA — ADR-0101).
    let padded = format!(" {a_name}");
    let (status, body) = post_send(&base, tok, &padded, "Space 1:1").await;
    assert_eq!(status, reqwest::StatusCode::OK, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["results"][0]["status"], "delivered",
        "surrounding whitespace is trimmed at resolution order ① and delivered to that agent (spec §5): {body}"
    );
    assert_eq!(
        v["results"][0]["to"],
        a_name.as_str(),
        "the result `to` is the trimmed token (the sender's notation) as is: {body}"
    );
    assert_eq!(
        messaging.parked_total_for_test(),
        0,
        "no phantom queue is created for a space name"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

// ── D (spec §6): lookup entrance — `messages` gives the same JSON at both the MCP and CLI entrances ──────────────────────
//
// Integration because that the two entrances call the same common handler is this increment's core contract (ADR-0086 entrance-agnostic), proven only by riding the wiring (a unit test sees one handler): a real daemon (MCP server + auth middleware + MessagingService) is started and the two paths' JSON compared for sameness.
// No claude needed: the lookup does not touch child stdin (read only).

#[tokio::test]
async fn d_messages_reports_delivery_state_by_id_and_the_callers_open_items() {
    let (manager, registry, base, data_dir, handle, _messaging, _busy) = wire("d-messages").await;

    let (recv_id, _recv_captured) = obs_seam::insert_seam_recipient(&manager, false);
    let recv_name = obs_seam::fallback_name(recv_id);
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "d-msg-tok".to_string(), true);
    let tok = Some("d-msg-tok");
    let sender_name = obs_seam::fallback_name(sender);

    // ① send a notice → look up its status by that id.
    let (_s, body) = post_send(&base, tok, &recv_name, "hello").await;
    let sent: serde_json::Value = serde_json::from_str(&body).expect("json");
    let msg_id = sent["id"].as_str().expect("id").to_string();

    let (status, body) = post_control(
        &base,
        "/control/messages",
        tok,
        serde_json::json!({ "id": msg_id }),
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["id"], msg_id.as_str());
    assert_eq!(v["from"], sender_name.as_str(), "the sender label: {body}");
    assert_eq!(
        v["awaiting_reply"], false,
        "a notice is not awaiting a reply"
    );
    assert_eq!(v["rows"].as_array().map(|a| a.len()), Some(1));
    assert_eq!(v["rows"][0]["to"], recv_name.as_str());
    assert_eq!(v["rows"][0]["status"], "delivered");
    assert!(
        v["rows"][0]["age_secs"].is_number(),
        "elapsed seconds are attached: {body}"
    );

    // ② a nonexistent id → MESSAGE_NOT_FOUND (the refusal shape is the same as for send).
    let (_s, body) = post_control(
        &base,
        "/control/messages",
        tok,
        serde_json::json!({ "id": "m-nope1234" }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["status"], "error");
    assert_eq!(v["code"], "MESSAGE_NOT_FOUND", "{body}");

    // 3) no argument = my open items. 1 parked item is caught as outbound_pending.
    // Setup changed with ADR-0111 decision 1: with absent parking abolished the only path making an open item is spec §5 branch 3 — an injection (write) failure parking (a seam where the recipient is live and reachable but the stdin write fails). The observed target (direction tags and fields of the open list) is unchanged.
    let (stuck_id, _stuck_captured) = obs_seam::insert_seam_recipient(&manager, true);
    let stuck_name = obs_seam::fallback_name(stuck_id);
    let (_s, _b) = post_send(&base, tok, &stuck_name, "parked").await;
    let (_s, body) = post_control(&base, "/control/messages", tok, serde_json::json!({})).await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["me"],
        sender_name.as_str(),
        "'me' is the token identity: {body}"
    );
    let open = v["open"].as_array().expect("open array");
    assert_eq!(
        open.len(),
        1,
        "1 open item (a delivered notice is not open): {body}"
    );
    assert_eq!(open[0]["direction"], "outbound_pending");
    assert_eq!(open[0]["to"], stuck_name.as_str());
    assert!(
        open[0].get("reply_by").is_none(),
        "a notice line has no contract axis"
    );

    // 4) asking with another identity shows only its own (another's open items do not leak — identity is token-derived).
    let other = AgentId::new_v4();
    registry.issue(other, 0, "d-msg-other".to_string(), true);
    let (_s, body) = post_control(
        &base,
        "/control/messages",
        Some("d-msg-other"),
        serde_json::json!({}),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["open"],
        serde_json::json!([]),
        "another's open items are not visible: {body}"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

#[tokio::test]
async fn d_messages_shows_the_reply_debt_on_both_sides_of_a_request() {
    let (manager, registry, base, data_dir, handle, _messaging, _busy) = wire("d-req-debt").await;

    let (recv_id, _c) = obs_seam::insert_seam_recipient(&manager, false);
    let recv_name = obs_seam::fallback_name(recv_id);
    // The recipient identity token — asks from that side's view (what I must answer).
    registry.issue(recv_id, 0, "d-recv-tok".to_string(), true);
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "d-send-tok".to_string(), true);

    let (_s, body) = post_send_json(
        &base,
        Some("d-send-tok"),
        serde_json::json!({ "to": recv_name, "body": "Do it", "request": true, "reply_by": "10m" }),
    )
    .await;
    let sent: serde_json::Value = serde_json::from_str(&body).expect("json");
    let req_id = sent["id"].as_str().expect("id").to_string();

    // The sender's view: awaiting reply (awaiting_their_reply) + the original deadline notation.
    let (_s, body) = post_control(
        &base,
        "/control/messages",
        Some("d-send-tok"),
        serde_json::json!({}),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    let open = v["open"].as_array().expect("array");
    assert_eq!(open.len(), 1, "{body}");
    assert_eq!(open[0]["direction"], "awaiting_their_reply");
    assert_eq!(open[0]["id"], req_id.as_str());
    assert_eq!(
        open[0]["reply_by"], "10m",
        "the original deadline notation as is: {body}"
    );
    assert_eq!(open[0]["timed_out"], false);

    // The recipient's view: my turn to answer (reply_owed_by_me) — the same contract with the opposite tag.
    let (_s, body) = post_control(
        &base,
        "/control/messages",
        Some("d-recv-tok"),
        serde_json::json!({}),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    let open = v["open"].as_array().expect("array");
    assert_eq!(open.len(), 1, "{body}");
    assert_eq!(open[0]["direction"], "reply_owed_by_me", "{body}");
    assert_eq!(open[0]["id"], req_id.as_str());

    // An id lookup tells the same fact by awaiting_reply.
    let (_s, body) = post_control(
        &base,
        "/control/messages",
        Some("d-send-tok"),
        serde_json::json!({ "id": req_id }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(v["awaiting_reply"], true, "{body}");

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

async fn call_mcp_tool(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    use rmcp::model::CallToolRequestParams;
    let mut params = CallToolRequestParams::default();
    params.name = name.to_string().into();
    params.arguments = Some(args.as_object().expect("object").clone());
    let result = client.call_tool(params).await.expect("call tool");
    let text = result
        .content
        .iter()
        .find_map(|c| c.as_text().map(|t| t.text.clone()))
        .expect("text content");
    serde_json::from_str(&text).expect("tool json")
}

/// The same JSON at both entrances (spec §6) — directly compares whether the MCP tool's and the CLI route's responses are byte-isomorphic; if they diverge, only one entrance's LLM learns a different contract (the same family as the send freeze).
#[tokio::test]
async fn d_mcp_and_cli_entrances_return_identical_json_for_messages_and_group() {
    use rmcp::transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    };
    use rmcp::ServiceExt;

    let (manager, registry, base, data_dir, handle, _messaging, _busy) = wire("d-parity").await;
    let caller = AgentId::new_v4();
    registry.issue(caller, 0, "d-parity-tok".to_string(), true);
    let tok = Some("d-parity-tok");

    install_tls_provider();
    let config = StreamableHttpClientTransportConfig::with_uri(handle.url.clone())
        .auth_header("d-parity-tok");
    let transport = StreamableHttpClientTransport::from_config(config);
    let client = ().serve(transport).await.expect("MCP handshake");

    // The tools/list exposed names = the same values the priming teaches.
    let tools = client.list_all_tools().await.expect("list tools");
    let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    for want in ["eg_send", "eg_messages"] {
        assert!(
            names.contains(&want.to_string()),
            "{want} in tools: {names:?}"
        );
    }
    // Removal of the `group` tool (ADR-0111 decision 4 · ADR-0112 decision 1) — if it remains, the priming teaches a nonexistent tool.
    assert!(
        !names.contains(&"group".to_string()),
        "the group tool must be removed: {names:?}"
    );

    // 1) eg_messages with no argument — same token so "me" is the same and the result the same (an empty open list).
    let via_mcp = call_mcp_tool(&client, "eg_messages", serde_json::json!({})).await;
    let (_s, body) = post_control(&base, "/control/messages", tok, serde_json::json!({})).await;
    let via_cli: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        via_mcp, via_cli,
        "the open lookup must be **byte-isomorphic** at both entrances"
    );
    assert_eq!(via_mcp["open"], serde_json::json!([]));

    // 2) a lookup refusal is also the same shape and code.
    let via_mcp = call_mcp_tool(
        &client,
        "eg_messages",
        serde_json::json!({ "id": "m-nope1234" }),
    )
    .await;
    let (_s, body) = post_control(
        &base,
        "/control/messages",
        tok,
        serde_json::json!({ "id": "m-nope1234" }),
    )
    .await;
    let via_cli: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        via_mcp, via_cli,
        "the refusal JSON is also the same at both entrances"
    );
    assert_eq!(via_mcp["code"], "MESSAGE_NOT_FOUND");

    // 3) send response parity (ADR-0111 multi-recipient) — MCP points at the same two recipients with an array, the CLI with a comma string. Notation differs per entrance (comma splitting is CLI-only) but is normalized at the daemon single point, so `results[]` must be byte-isomorphic (compared excluding `id`).
    let (x_id, _x) = obs_seam::insert_seam_recipient(&manager, false);
    let (y_id, _y) = obs_seam::insert_seam_recipient(&manager, false);
    let (x, y) = (obs_seam::fallback_name(x_id), obs_seam::fallback_name(y_id));
    let via_mcp = call_mcp_tool(
        &client,
        "eg_send",
        serde_json::json!({ "to": [x.clone(), y.clone()], "body": "parity" }),
    )
    .await;
    let (_s, body) = post_send(&base, tok, &format!("{x},{y}"), "parity").await;
    let via_cli: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        via_mcp["results"], via_cli["results"],
        "the multi-recipient send response must be the same at both entrances: mcp={via_mcp} cli={via_cli}"
    );
    assert_eq!(
        via_mcp["results"].as_array().map(|a| a.len()),
        Some(2),
        "1 row per recipient: {via_mcp}"
    );

    // 4) an MCP array element is not split on commas (spec §6) — `"a,b"` is one such name. The same string via the CLI is split into two recipients, so the two entrances' results must deliberately differ.
    let split_free = call_mcp_tool(
        &client,
        "eg_send",
        serde_json::json!({ "to": [format!("{x},{y}")], "body": "no-split" }),
    )
    .await;
    let rows = split_free["results"].as_array().expect("results");
    assert_eq!(rows.len(), 1, "an array element is one name: {split_free}");
    assert_eq!(rows[0]["status"], "failed");
    assert_eq!(
        rows[0]["code"], "RECIPIENT_NOT_FOUND",
        "there is no agent named 'x,y' (evidence of no double splitting): {split_free}"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// D review B1 — a real-entrance regression net: with two live agents of the same name, the reply obligation of a request made with the exact AgentId is not wrongly attached to the twin. Identity derives from two tokens (each AgentId), so this axis is proven only by the real authentication path (unit tests see only service function arguments).
#[tokio::test]
async fn d_messages_does_not_misassign_a_reply_obligation_to_a_same_named_twin() {
    let (manager, registry, base, data_dir, handle, _messaging, _busy) = wire("d-twin").await;

    // Two live agents with the same visible name — only then does the misattribution of name-only attribution show.
    let twin_a = AgentId::new_v4();
    let twin_b = AgentId::new_v4();
    let (_a, _cap_a) = obs_seam::insert_seam_recipient_named(&manager, false, twin_a, "worker");
    let (_b, _cap_b) = obs_seam::insert_seam_recipient_named(&manager, false, twin_b, "worker");
    registry.issue(twin_a, 0, "d-twin-a".to_string(), true);
    registry.issue(twin_b, 0, "d-twin-b".to_string(), true);
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "d-twin-send".to_string(), true);

    // exact AgentId request to A only.
    let (_s, body) = post_send_json(
        &base,
        Some("d-twin-send"),
        serde_json::json!({ "to": twin_a.to_string(), "body": "Do it", "request": true, "reply_by": "10m" }),
    )
    .await;
    let sent: serde_json::Value = serde_json::from_str(&body).expect("json");
    let req_id = sent["id"].as_str().expect("id").to_string();

    // A's view: my turn to answer.
    let (_s, body) = post_control(
        &base,
        "/control/messages",
        Some("d-twin-a"),
        serde_json::json!({}),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    let a_open = v["open"].as_array().expect("array");
    assert_eq!(
        a_open.len(),
        1,
        "the targeted A carries the obligation: {body}"
    );
    assert_eq!(a_open[0]["direction"], "reply_owed_by_me");
    assert_eq!(a_open[0]["id"], req_id.as_str());

    // B's view: it never received it, so there is nothing (the old implementation returned A's obligation here).
    let (_s, body) = post_control(
        &base,
        "/control/messages",
        Some("d-twin-b"),
        serde_json::json!({}),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        v["open"],
        serde_json::json!([]),
        "an obligation must not attach to a twin that never received it (B1): {body}"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}

/// D review B2 — the response carries whether the lookup asserts completeness.
#[tokio::test]
async fn d_messages_declares_whether_the_row_list_is_complete() {
    let (manager, registry, base, data_dir, handle, _messaging, _busy) = wire("d-trunc").await;
    let (recv_id, _c) = obs_seam::insert_seam_recipient(&manager, false);
    let recv_name = obs_seam::fallback_name(recv_id);
    let sender = AgentId::new_v4();
    registry.issue(sender, 0, "d-trunc-tok".to_string(), true);
    let tok = Some("d-trunc-tok");

    let (_s, body) = post_send(&base, tok, &recv_name, "hello").await;
    let sent: serde_json::Value = serde_json::from_str(&body).expect("json");
    let msg_id = sent["id"].as_str().expect("id").to_string();

    let (_s, body) = post_control(
        &base,
        "/control/messages",
        tok,
        serde_json::json!({ "id": msg_id }),
    )
    .await;
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    // The ring has not been pushed yet = certainly all of it. The field is always carried (absence must not be misread as completeness).
    assert_eq!(
        v["may_be_truncated"], false,
        "completeness is asserted actively: {body}"
    );
    assert!(
        v.get("hint").is_none(),
        "when complete no warning sentence is attached: {body}"
    );

    let _ = std::fs::remove_dir_all(&data_dir);
    handle.shutdown().await;
}
