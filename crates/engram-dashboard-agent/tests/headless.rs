//! ② Isolation integration test — asserts the whole AgentManager flow with a real shell spawn.
//!
//! A real OS process (default shell), light single spawn without named-mutex/global contention, so it runs by default under `cargo test -p
//!   engram-dashboard-agent`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::persistence::FileProfileStore;
use engram_dashboard_agent::profile::{AgentCommand, AgentProfile, ProfileRegistry, SpawnMode};
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::types::{
    AgentId, AgentInfo, AgentStatus, ControlChannel, ControlChannelNeeds, ControlEndpoint,
    InputOrigin, OutputFrame, OutputPayload, OutputSink, ProvisionError, SinkError, SinkId,
    StatusSink,
};
use engram_dashboard_base::testing::wait_until;
use engram_dashboard_platform::shell::default_shell;

// ── RecordingSink ────────────────────────────────────────────────────────────

#[derive(Clone)]
struct RecordingSink {
    id: SinkId,
    output: Arc<Mutex<Vec<u8>>>,
    statuses: Arc<Mutex<Vec<AgentStatus>>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            output: Arc::new(Mutex::new(Vec::new())),
            statuses: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn output_len(&self) -> usize {
        self.output.lock().unwrap().len()
    }

    fn output_contains(&self, needle: &str) -> bool {
        let buf = self.output.lock().unwrap();
        let text = String::from_utf8_lossy(&buf);
        text.contains(needle)
    }

    fn statuses(&self) -> Vec<AgentStatus> {
        self.statuses.lock().unwrap().clone()
    }
}

impl OutputSink for RecordingSink {
    fn send(&self, frame: OutputFrame<'_>) -> Result<(), SinkError> {
        if let OutputPayload::Bytes(b) = frame.payload {
            self.output.lock().unwrap().extend_from_slice(b);
        }
        Ok(())
    }

    fn sink_id(&self) -> SinkId {
        self.id
    }
}

impl StatusSink for RecordingSink {
    fn status_changed(&self, _id: AgentId, status: AgentStatus, _epoch: u32) {
        self.statuses.lock().unwrap().push(status);
    }

    fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
}

// ── FIX 6: control channel provision race guard — concurrent spawns of the same id do not trample each other ──────────
//
// Verifies the agent's reservation (SpawnReservation) causality in isolation, without a real DaemonControlChannel (ADR-0012 seam).
#[derive(Clone, Default)]
struct CountingControl {
    live: Arc<Mutex<std::collections::HashSet<(AgentId, u32)>>>,
    provisions: Arc<std::sync::atomic::AtomicUsize>,
    seen_flags: Arc<Mutex<Vec<bool>>>,
}

impl ControlChannel for CountingControl {
    fn provision(
        &self,
        id: AgentId,
        epoch: u32,
        needs: ControlChannelNeeds,
    ) -> Result<Option<ControlEndpoint>, ProvisionError> {
        self.provisions
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.seen_flags
            .lock()
            .unwrap()
            .push(needs.accepts_mcp_config);
        self.live.lock().unwrap().insert((id, epoch));
        Ok(Some(ControlEndpoint {
            agent_id: id,
            epoch,
            url: "http://127.0.0.1:1/mcp".into(),
            token: format!("tok-{id}-{epoch}"),
            // ADR-0099: config_path is an Option — this test double does not verify the MCP channel physics, so None.
            config_path: None,
            send_exe: None,
            // ADR-0094
            grants: vec![],
            // ADR-0133: computing mail authorization is out of concern too — it only imitates the daemon's computation rule (the
            //   ingredients are two).
            mail_allowed: needs.uses_mail && !needs.accepts_mcp_config,
        }))
    }
    fn revoke(&self, id: AgentId, epoch: u32) {
        self.live.lock().unwrap().remove(&(id, epoch));
    }
}

impl CountingControl {
    fn live_count(&self) -> usize {
        self.live.lock().unwrap().len()
    }
}

#[test]
fn concurrent_same_id_spawn_does_not_clobber() {
    // Verified here = the exactly-one-wins causality of SpawnReservation itself; the provision race of control-channel-consuming backends
    //   is not covered here (the same guard covers it).
    let control = CountingControl::default();
    let control_dyn: Arc<dyn ControlChannel> = Arc::new(control.clone());
    let status_dyn: Arc<dyn StatusSink> = Arc::new(RecordingSink::new());
    let store = Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-test-race-{}", Uuid::new_v4())),
    ));
    let profiles = Arc::new(ProfileRegistry::new(store));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = Arc::new(AgentManager::new_with_control(
        status_dyn,
        profiles,
        tracker,
        control_dyn,
    ));

    // A long-lived shell profile (same id) — so it can be observed before a spawn that succeeded exits immediately and the reaper erases
    //   the token.
    let profile = AgentProfile::new(
        "race".into(),
        AgentCommand::Shell {
            program: default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        false,
    );

    const N: usize = 8;
    let barrier = Arc::new(std::sync::Barrier::new(N));
    let ok_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    std::thread::scope(|s| {
        for _ in 0..N {
            let manager = manager.clone();
            let profile = profile.clone();
            let barrier = barrier.clone();
            let ok_count = ok_count.clone();
            s.spawn(move || {
                barrier.wait();
                // ★The counted axis is 「did it launch」, not 「was it not an error」★: duplicate requests are outcomes (moot), not errors, so
                //   everyone gets Ok (ADR-0172). Only one side made the incarnation — counting it is what this guard verifies.
                if manager
                    .spawn_agent(&profile, SpawnMode::Fresh)
                    .expect("a duplicate request is not an error")
                    .started()
                {
                    ok_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            });
        }
    });

    assert_eq!(
        ok_count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "among concurrent spawns of the same id exactly one must actually launch (reservation guard)"
    );
    assert_eq!(
        manager.list_agents().len(),
        1,
        "exactly 1 in the session map (not trampling the other's session)"
    );
    assert_eq!(
        control.provisions.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "shell has supports_control_channel=false, so provision is not called (F3)"
    );
    assert_eq!(
        control.live_count(),
        0,
        "the shell path makes no live control channel token (registry untouched)"
    );

    let id = manager.list_agents()[0].id;
    manager.kill_agent(id).expect("kill ok");
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
}

// ── round-2 F3: backend-conditional provisioning (agent seam) ──────────────────────────────
// A backend that does not consume the control channel (shell) **never calls provision**, so a shell spawn succeeds even with an
// always-failing ControlChannel. A consuming backend (claude) fails closed (aborts the spawn) on a provision Err. Both causalities are
//   verified at the agent level (seam).
#[derive(Clone, Default)]
struct FailingControl {
    calls: Arc<std::sync::atomic::AtomicUsize>,
    seen_flags: Arc<Mutex<Vec<bool>>>,
}
impl ControlChannel for FailingControl {
    fn provision(
        &self,
        _id: AgentId,
        _epoch: u32,
        needs: ControlChannelNeeds,
    ) -> Result<Option<ControlEndpoint>, ProvisionError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.seen_flags
            .lock()
            .unwrap()
            .push(needs.accepts_mcp_config);
        Err(ProvisionError("injected".into()))
    }
    fn revoke(&self, _id: AgentId, _epoch: u32) {}
}

fn make_manager_with(control: Arc<dyn ControlChannel>) -> Arc<AgentManager> {
    let status_dyn: Arc<dyn StatusSink> = Arc::new(RecordingSink::new());
    let store = Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-test-f3-{}", Uuid::new_v4())),
    ));
    let profiles = Arc::new(ProfileRegistry::new(store));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    Arc::new(AgentManager::new_with_control(
        status_dyn, profiles, tracker, control,
    ))
}

#[test]
fn shell_spawn_ignores_failing_control_channel() {
    let control = FailingControl::default();
    let manager = make_manager_with(Arc::new(control.clone()));
    let profile = AgentProfile::new(
        "f3-shell".into(),
        AgentCommand::Shell {
            program: default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    let res = manager.spawn_agent(&profile, SpawnMode::Fresh);
    assert!(
        res.is_ok(),
        "the shell does not call provision, so the spawn succeeds (F3)"
    );
    assert_eq!(
        control.calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the shell path must not call provision (F3 backend-conditional)"
    );
    assert!(
        control.seen_flags.lock().unwrap().is_empty(),
        "shell → provision not called, so there must be no accepts_mcp_config flag passed"
    );
    let id = manager.list_agents()[0].id;
    manager.kill_agent(id).expect("kill ok");
    let _ = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
}

#[test]
fn claude_spawn_fails_closed_on_provision_error() {
    // ★No claude binary needed★: the provision Err returns early with `?`, so it never reaches an actual process spawn.
    let control = FailingControl::default();
    let manager = make_manager_with(Arc::new(control.clone()));
    let profile = AgentProfile::new(
        "f3-claude".into(),
        AgentCommand::Claude {
            extra_args: vec![],
            output_format: engram_dashboard_agent::profile::AgentOutputFormat::Terminal,
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    let res = manager.spawn_agent(&profile, SpawnMode::Fresh);
    assert!(res.is_err(), "claude fails closed on a provision Err (F3)");
    assert_eq!(
        control.calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the claude path must call provision exactly once"
    );
    // ADR-0099 (FIX 5): backend::accepts_mcp_config returns claude=true and that value arrives through the plumbing.
    assert_eq!(
        *control.seen_flags.lock().unwrap(),
        vec![true],
        "a claude spawn must pass accepts_mcp_config=true to provision (MCP-capable)"
    );
    assert!(
        manager.list_agents().is_empty(),
        "a fail-closed spawn must not register a session"
    );
}

#[test]
fn manager_spawn_write_resize_kill() {
    let status_sink = RecordingSink::new();
    let status_dyn: Arc<dyn StatusSink> = Arc::new(status_sink.clone());

    let store = Arc::new(FileProfileStore::new(
        std::env::temp_dir().join(format!("engram-test-headless-{}", Uuid::new_v4())),
    ));
    let profiles = Arc::new(ProfileRegistry::new(store));
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig {
            enabled: false,
            poll_interval: Duration::from_secs(1),
        },
        Arc::new(|_, _| {}),
    ));
    let manager = AgentManager::new(status_dyn, profiles, tracker);

    let profile = AgentProfile::new(
        "headless".into(),
        AgentCommand::Shell {
            program: default_shell().to_string(),
            args: vec![],
        },
        PathBuf::from("."),
        vec![],
        false,
    );
    let info = manager
        .spawn_agent(&profile, SpawnMode::Fresh)
        .expect("spawn failed")
        .into_started()
        .expect("this call actually launches (not a duplicate request)");

    assert_eq!(
        manager.list_agents().len(),
        1,
        "there must be 1 agent after spawn"
    );

    let out_sink = RecordingSink::new();
    let _sid = manager
        .subscribe(info.id, Arc::new(out_sink.clone()))
        .expect("subscribe failed");

    let got_output = wait_until(Duration::from_secs(2), || out_sink.output_len() > 0);
    assert!(
        got_output,
        "did not receive the initial PTY output within 2s"
    );

    manager
        .write_stdin(info.id, b"echo headless-test\r\n", InputOrigin::User)
        .expect("write_stdin failed");
    let echoed = wait_until(Duration::from_secs(3), || {
        out_sink.output_contains("headless-test")
    });
    assert!(
        echoed,
        "the echo input was not reflected in the PTY output (headless-test not received)"
    );

    manager.resize(info.id, 100, 30).expect("resize failed");

    let kill_started = Instant::now();
    manager.kill_agent(info.id).expect("kill_agent failed");
    let removed = wait_until(Duration::from_secs(5), || manager.list_agents().is_empty());
    let kill_elapsed = kill_started.elapsed();

    assert!(
        removed,
        "the session remains after kill — reaper map removal failed"
    );
    assert!(
        kill_elapsed < Duration::from_secs(5),
        "kill→reap did not finish within 5s (suspected hang): {kill_elapsed:?}"
    );

    // The terminal notification is the pump's alone (ADR-0005), so it can be slightly asynchronous, hence polling.
    let reached_killed = wait_until(Duration::from_secs(2), || {
        matches!(status_sink.statuses().last(), Some(AgentStatus::Killed))
    });
    let seq = status_sink.statuses();
    assert!(
        reached_killed,
        "status did not reach the terminal Killed — transition record: {seq:?}"
    );
    assert!(
        seq.iter().any(|s| matches!(s, AgentStatus::Exiting)),
        "the kill transition has no transitional Exiting: {seq:?}"
    );
}
