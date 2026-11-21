//! engram-dashboard-daemon — the library surface.
//!
//! Server assembly and the accept loop live here so `main.rs` (one line, `run()`) and the isolation harness (`tests/ws_e2e.rs`, via
//! `start_test_server()` behind the `test-support` feature, absent from production builds) share the same startup flow.
//!
//! Scope of this crate (ADR-0130): the application layer, assembly (`run()` decides the call order of the parts), and process
//! housekeeping — the single-instance guard (`instance`) and `daemon.json` (`portfile`). The accepting side runs on the transport crate
//! (`link`) and meets the upper layers through the connection port contract (`conn_port`).

pub mod agent_conn;
pub mod command_delivery;
pub mod command_roster;
pub mod conn_port;
pub mod connection_core;
pub mod control;
pub mod data_dir;
#[cfg(feature = "test-harness")]
pub mod experiment;
mod file_hooks;
pub mod identity;
pub mod instance;
mod link;
#[cfg(test)]
mod log_capture;
pub mod messaging_host;
#[cfg(test)]
mod panic_hook_tests;
pub mod portfile;
pub mod status_fanout;
#[cfg(test)]
mod test_doubles;
pub mod usage_service;

use std::path::PathBuf;
use std::sync::{Arc, Weak};

use engram_dashboard_agent::commands::LibraryCommandHost;
use engram_dashboard_agent::launch::{LaunchPorts, StdLaunchFs};
use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::persistence::FileProfileStore;
use engram_dashboard_agent::profile::{ProfileRegistry, ProfileStore};
use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
use engram_dashboard_agent::types::CLI_EXE_NAME;
use engram_dashboard_base::logging;
use engram_dashboard_protocol::PROTOCOL_VERSION;

use tokio::sync::watch;

use crate::conn_port::FrameFanout;
use connection_core::MultiViewState;
use data_dir::{ensure_data_dir_writable, DataLayout};
use engram_dashboard_agent::usage::{OsProbeSpawner, ProbeSpawner, UsageProbe};
use engram_dashboard_asset_library::backends::AgentBackends;
use engram_dashboard_asset_library::config::LibraryConfig;
use engram_dashboard_asset_library::folder::{
    BoundedIo, LaunchWaits, StdFolderOps, ThreadSpawn, IO_LIMIT, LIST_CAP,
};
use engram_dashboard_asset_library::store::FsStore;
use engram_dashboard_asset_library::{LibraryService, LibraryValues};
use link::server::{ConnRegistry, Listening};
use status_fanout::DaemonStatusSink;
use usage_service::clock::OsUsageClock;
use usage_service::observe::UsageObserveSink;
use usage_service::{OsProbeThreads, ProbeThreads, UsageParts, UsageService};

// The link's keepalive settings appear in this crate's public signature (`start_test_server_with_keepalive`), so the type is
//   re-exported, not the module; that signature is behind feature `test-support`, so the re-export is too (ADR-0286).
#[cfg(feature = "test-support")]
pub use link::server::KeepaliveConfig;
#[cfg(not(feature = "test-support"))]
use link::server::KeepaliveConfig;
// ADR-0129 0-4: the handshake frame (protocol `handshake::AuthFrame`) is not re-exported — it is absent from this crate's public
//   signatures, so one type would get two import paths; `tests/ws_e2e.rs` calls protocol directly.

// Do not assemble the path here: the file the daemon holds and the file it writes must be the same for the single instance to hold
//   (ADR-0135). The path comes from `data_dir::DataLayout` alone, written only through the held guard; this name is for log wording.
use crate::portfile::DAEMON_FILE;

// ── data dir / token ──────────────────────────────────────────────────────────────

/// Security: do not print the return value to logs (it is written only to daemon.json).
pub fn generate_token() -> Result<String, getrandom::Error> {
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf)?;
    Ok(engram_dashboard_base::text::hex_lower(&buf))
}

/// The root generation a run's library service starts at ([`LibraryStart::root_generation_base`]): random below 2^52,
/// so the base plus a run's root changes stays an exact JSON number and a generation kept from an earlier run does not
/// match. Should the OS source fail (the boot token draws from it first and stops the daemon then), std's per-process
/// random hash keys stand in — never the clock.
// ADR-0381
pub fn root_generation_base() -> u64 {
    const BELOW: u64 = 1 << 52;
    let mut buf = [0u8; 8];
    let drawn = match getrandom::getrandom(&mut buf) {
        Ok(()) => u64::from_le_bytes(buf),
        Err(e) => {
            tracing::warn!(
                "root generation base: OS random source failed; using std hash keys: {e}"
            );
            use std::hash::{BuildHasher, Hasher};
            std::collections::hash_map::RandomState::new()
                .build_hasher()
                .finish()
        }
    };
    drawn % BELOW
}

// ── control plane CLI location search (ADR-0086 step 2 · F1) ─────────────────────────────────
//
// An agent of a backend that cannot use MCP must call the control CLI (`CLI_EXE_NAME` + platform extension) from a shell, but the binary
// is not on PATH. So the daemon finds the absolute path among the siblings of its own exe folder (rule = platform `env::sibling_exe`, the
// same function as the first candidate of the shell `discovery` module's `locate_daemon_exe`; the three exes deploy together), and
// provision carries it as ControlEndpoint.send_exe. The backend injects it as ENGRAM_CLI_EXE·PATH into every spawn with a control
// endpoint — control verbs are open to everyone (ADR-0132 decision 5), not just mail.
//
// Best-effort (fail-open): if not found (a partial dev build), it returns None and the daemon keeps starting — an MCP-capable backend
// (claude) only loses the control CLI, and a non-MCP backend spawn has 0 mail entrances so provision blocks it fail-closed. A warn log records the cause.

/// Returns the path value without setting env (env injection is the backend's).
///
/// Do not delete it even with one caller: this value is the only mail entrance of non-MCP backends and the control entrance of every
/// spawn; without it non-MCP provision blocks spawns fail-closed. This stretch has no test (passing None at the call site keeps the suite
/// green), so this anchor is the only defense; the later stretch (provision→endpoint→spawn env) is caught by the daemon control tests
/// `provision_*_wires_the_*_cli_*`.
// ADR-0133
fn locate_send_exe() -> Option<PathBuf> {
    // The file name derives from the constant — a separate literal could diverge from the deployed executable and silently disable the CLI entrance.
    if let Some(send_exe) = engram_dashboard_platform::env::sibling_exe(CLI_EXE_NAME) {
        if send_exe.is_file() {
            tracing::info!(path = %send_exe.display(), "control plane CLI location settled (ADR-0086 F1)");
            return Some(send_exe);
        }
    }
    tracing::warn!(
        name = %engram_dashboard_platform::env::exe_file_name(CLI_EXE_NAME),
        "could not find the control plane CLI sibling exe — CLI entrance disabled (MCP entrance normal, ADR-0086 F1)"
    );
    None
}

// ── file log · panic hook ───────────────────────────────────────────────────────────

/// After this, every panic leaves a reason line (thread · sentence · location) in this log, and a panic outside the containment mark
/// (base `panic::contain`) ends the daemon process on the spot — only contained places (the three agent pumps · input writer drain) fold
/// into that place's failure surface. Bus command execution is not contained (ADR-0293). The hook ends things before unwinding, so
/// tokio does not contain a task panic in that task.
///
/// Even with no log place openable (`None`) the hook still ends things. An unwritable data folder stops startup (`ensure_data_dir_writable`),
/// so this case is: the folder is writable but both its `logs` and the `%TEMP%` fallback cannot be opened. Then the reason line goes only
/// to stdout (and the previous hook's stderr) — nowhere to remain for a shell-launched daemon (release has no console). No other fallback
/// (TRD S21-panic-policy §7).
// ADR-0292
fn init_logging_and_panic_hook(layout: &DataLayout) -> Option<PathBuf> {
    let log_file = logging::init_logging_with_file(&layout.logs_dir(), logging::LogKind::Daemon);
    engram_dashboard_base::panic::install_hook();
    log_file
}

// ── AgentManager wiring (mirror of src-tauri lib.rs setup) ───────────────────────────────

/// The daemon wiring the assembly carries as one lump — ties `AgentManager` with the very connection registry its status sink fans out to.
///
/// Why tie them (load-bearing): the manager carries `DaemonStatusSink` → `Arc<dyn FrameFanout>` inside it, so a manager made with
/// `build_daemon_wiring(A)` passed into `run_accept_loop(.., registry B)` compiles. Connections register in B while state · list · restore
/// notifications go to A and reach no client — an empty map is 0 iterations, with not even a failure log, and the frontend decides terminal by
/// list events (ADR-0005), so every agent looks alive forever. Lease/profile broadcasts vanish the same way.
/// A mismatched pair is representable at every place that takes both a parameter receiving the registry and a parameter carrying the
/// fanout, directly or transitively (`manager` → status sink; `handlers` → factory → `AgentConnections.fanout`). Enumerating paths always
/// missed one, so "it does not take the fanout type" is not a criterion. Today the rule catches the entry points outside the assembly
/// (`link::server::Server::new` · `AgentConnections::new` · `ConnectionCore::new`) and the `link::server` test harness; none inside the
/// assembly, which is why this struct exists. Hand-assembling it with someone else's registry is within one module, so the type cannot block it.
/// Those entry points are not fixed here: `Server::new` legitimately needs the registry and must not know how the factory was assembled.
/// The fix is slice 3's projection (`impl ConnRegistry { pub fn fanout(&self) -> Arc<dyn FrameFanout> }`), deferred by ADR-0130 — the
/// prescription for when it resumes, not planned work.
/// The usage service is the same pair: the service the `manager`'s status sink chain (`UsageObserveSink`) feeds and the one the connection
/// layer subscribes and ⟳s on must be one; otherwise observations pile up in one while subscribers get no sheet, quietly.
// ADR-0129
struct DaemonWiring {
    manager: Arc<AgentManager>,
    /// The map connections are registered in — the same as the map the status sink inside `manager` fans out to (`ConnRegistry` is an Arc inside).
    registry: ConnRegistry,
    /// The service the `manager`'s status sink chain feeds — the connection layer subscribes and ⟳s on the same one.
    usage: Arc<UsageService>,
    /// The library service plugged in as `manager`'s launch-kit source — the same one answers the library commands
    /// ([`DaemonWiring::library_commands`]) — with `manager` plugged into it as its profile-edit port. `None` = the
    /// Noop kits (the test server). Production also reads it for the first-start step and the boot scan
    /// ([`build_daemon_wiring`]).
    // ADR-0311
    library: Option<Arc<LibraryService>>,
}

impl DaemonWiring {
    /// The fanout adapter the control route (`/control/agent`) uses to announce roster changes (ADR-0132).
    ///
    /// Made here from the struct's own two fields: taking fanout and manager out separately would make the mismatched pair above representable again.
    fn roster_broadcast(&self) -> Arc<dyn control::agent::RosterBroadcast> {
        Arc::new(connection_core::RosterFanout::new(
            Arc::new(self.registry.clone()),
            self.manager.clone(),
        ))
    }

    /// The library port of the command table — the service `manager` launches through, so the bus edits the library
    /// the launches read; with no service (the test server), [`control::commands::NoLibrary`].
    // ADR-0386
    fn library_commands(&self) -> Arc<dyn LibraryCommandHost> {
        match &self.library {
            Some(library) => library.clone(),
            None => Arc::new(control::commands::NoLibrary),
        }
    }
}

fn build_daemon_wiring(
    layout: &DataLayout,
    control: Arc<dyn engram_dashboard_agent::types::ControlChannel>,
    flush_tx: tokio::sync::mpsc::UnboundedSender<messaging_host::FlushMsg>,
    idle_coalescer: Arc<messaging_host::IdleCoalescer>,
) -> DaemonWiring {
    let state_dir = layout.daemon_state_dir();
    let profile_store = Arc::new(FileProfileStore::new(state_dir.clone()));
    let scratch_root = layout.usage_probe_dir();
    // The safety of this sweep depends on timing: after taking the instance lock (no other daemon can be querying in the folder) and before any
    //   scheduler or query starts. Moved before the lock or after [`build_usage_service`], it could delete a live query's folder.
    engram_dashboard_agent::usage::sweep_stale_scratch(&scratch_root);
    // The launch folders left by an earlier daemon go on the same timing: after the instance lock, and before the
    //   library service is plugged in below (boot restore is off, so nothing launches before). Plugged first, a launch
    //   could have its fresh folder removed.
    let launch_root = layout.launch_dir();
    engram_dashboard_asset_library::launch::sweep_launch_dir(&launch_root);
    let start = LibraryStart {
        config: engram_dashboard_asset_library::config::load(&state_dir),
        default_root: layout.default_library_root(),
        data_root: layout.absolute_root(),
        daemon_id: identity::load_or_create(&state_dir),
        root_generation_base: root_generation_base(),
    };
    let wiring = build_daemon_wiring_with_store(
        profile_store,
        UsageInputs {
            probes: engram_dashboard_agent::backend::usage_probes().to_vec(),
            spawner: Arc::new(OsProbeSpawner),
            threads: Arc::new(OsProbeThreads),
            scratch_root,
        },
        Some(LaunchInputs {
            launch_root,
            builtin_dir: data_dir::find_builtin_dir(),
            start,
        }),
        control,
        flush_tx,
        idle_coalescer,
    );
    // After the service is built and before `run()` sets the command table, so no command meets a first start
    //   halfway; production only — the test server builds no service. The step logs its own failure or unsaved root.
    //   Then the boot scan starts on every start, whatever the first start answered, and is never waited for: the
    //   daemon does not walk the library before it is reachable (catalog readers wait for the walk at the entrance).
    // ADR-0375
    // ADR-0381
    // ADR-0382
    if let Some(library) = &wiring.library {
        let outcome = library.first_start();
        tracing::debug!(?outcome, "library first start");
        library.start_boot_scan();
    }
    wiring
}

/// What the launch kits take from outside the assembly — production passes it ([`build_daemon_wiring`]); the test
/// server passes none and keeps the Noop kits.
struct LaunchInputs {
    /// The volatile launch folder (`DataLayout::launch_dir`).
    launch_root: PathBuf,
    /// The built-in assets folder (`data_dir::find_builtin_dir`); `None` = none could be found.
    builtin_dir: Option<PathBuf>,
    start: LibraryStart,
}

/// The mount setting, the data folder, the identity and the root generation base a library service starts with
/// ([`LibraryValues::config`] · [`LibraryValues::default_root`] · [`LibraryValues::data_root`] ·
/// [`LibraryValues::daemon_id`] · [`LibraryValues::root_generation_base`]).
pub struct LibraryStart {
    /// `asset_library::config::load` over the daemon state folder — read before the service is built.
    pub config: LibraryConfig,
    /// `DataLayout::default_library_root`.
    pub default_root: PathBuf,
    /// `DataLayout::absolute_root`.
    pub data_root: PathBuf,
    /// `identity::load_or_create`.
    pub daemon_id: String,
    /// [`root_generation_base`], drawn once per run.
    pub root_generation_base: u64,
}

/// The home folder and environment a library service reads the native skill roots through
/// ([`LibraryValues::home`] · [`LibraryValues::env`]).
pub struct LibraryHost {
    pub home: Option<PathBuf>,
    pub env: Arc<dyn Fn(&str) -> Option<String> + Send + Sync>,
}

impl LibraryHost {
    /// The daemon process's own home folder and environment — what production launches read.
    pub fn process() -> Self {
        LibraryHost {
            home: engram_dashboard_platform::env::home_dir(),
            env: Arc::new(|name| std::env::var(name).ok()),
        }
    }
}

/// The library service the daemon launches through, over the production folder waits. `builtin_dir` = the built-in
/// assets folder (`None` = there is none), `launch_root` = the volatile launch folder, `start` = the root setting read
/// from `library.json` and the daemon identity. Building it does no disk work beyond the built-ins; the first-start
/// step is the caller's ([`build_daemon_wiring`]). The one builder for production wiring and the harness bin
/// `roundtrip-smoke`, so the two cannot drift.
// ADR-0311
// ADR-0360
// ADR-0368
// ADR-0370
// ADR-0375
pub fn build_library_service(
    registry: Arc<ProfileRegistry>,
    builtin_dir: Option<PathBuf>,
    launch_root: PathBuf,
    host: LibraryHost,
    start: LibraryStart,
) -> LibraryService {
    LibraryService::new(
        LibraryValues {
            registry,
            backends: Arc::new(AgentBackends),
            config: start.config,
            default_root: start.default_root,
            data_root: start.data_root,
            daemon_id: start.daemon_id,
            root_generation_base: start.root_generation_base,
            builtin_dir,
            launch_root,
            home: host.home,
            env: host.env,
        },
        Arc::new(FsStore),
        BoundedIo::new(
            IO_LIMIT,
            LaunchWaits::LAUNCH,
            LIST_CAP,
            Arc::new(StdFolderOps),
            Arc::new(ThreadSpawn),
        ),
    )
}

/// The parts the usage service receives from outside the assembly — production has the real ones ([`build_daemon_wiring`]), the test server 0 probers.
struct UsageInputs {
    probes: Vec<&'static dyn UsageProbe>,
    spawner: Arc<dyn ProbeSpawner>,
    threads: Arc<dyn ProbeThreads>,
    scratch_root: PathBuf,
}

/// The store-injecting form of build_daemon_wiring — tests slot in an in-memory store to isolate from disk/Embedded.
fn build_daemon_wiring_with_store(
    store: Arc<dyn ProfileStore>,
    usage_inputs: UsageInputs,
    launch_inputs: Option<LaunchInputs>,
    control: Arc<dyn engram_dashboard_agent::types::ControlChannel>,
    flush_tx: tokio::sync::mpsc::UnboundedSender<messaging_host::FlushMsg>,
    idle_coalescer: Arc<messaging_host::IdleCoalescer>,
) -> DaemonWiring {
    // ADR-0129: the only place in the production assembly that makes the connection registry and its fanout face — a concrete `ConnRegistry`
    //   used to make a pair mismatch impossible by type, so creation is gathered in one place to keep that guarantee (ADR-0129 decision 3).
    let registry = ConnRegistry::new();
    let fanout: Arc<dyn FrameFanout> = Arc::new(registry.clone());
    let usage = build_usage_service(usage_inputs);
    let status_sink = Arc::new(status_chain(
        fanout,
        usage.clone(),
        flush_tx,
        idle_coalescer,
    ));
    let profiles = Arc::new(ProfileRegistry::new(store));

    let profiles_cb = profiles.clone();
    let tracker = Arc::new(SessionTracker::new(
        TrackerConfig::default(),
        Arc::new(move |agent_id, new_sid| {
            // `None` in the incarnation marker place — this observer (`SessionTracker`) does not carry that axis.
            profiles_cb.observe_session_id(agent_id, None, new_sid);
        }),
    ));
    tracker.start();

    let manager = AgentManager::new_with_control(status_sink, profiles.clone(), tracker, control);
    let (manager, library) = match launch_inputs {
        Some(inputs) => {
            let library = Arc::new(build_library_service(
                profiles,
                inputs.builtin_dir,
                inputs.launch_root,
                LibraryHost::process(),
                inputs.start,
            ));
            let manager = manager.with_launch_ports(LaunchPorts {
                kits: library.clone(),
                fs: Arc::new(StdLaunchFs),
            });
            (manager, Some(library))
        }
        None => (manager, None),
    };
    let manager = Arc::new(manager);
    // ADR-0380: the manager goes into the service as its profile-edit port right here, before any command table can
    //   reach the service; weak, as the manager already holds the service as its launch-kit source.
    if let Some(library) = &library {
        let port: Weak<AgentManager> = Arc::downgrade(&manager);
        library.plug_profile_edit(port);
    }
    DaemonWiring {
        manager,
        registry,
        usage,
        library,
    }
}

/// Sets up the usage service and launches the scheduler thread.
///
/// The daemon starts even if the scheduler cannot launch: requests (⟳) · pickup · subscription replacement keep running; only time-driven work
/// (automatic queries · deadline emits) stops. The thread holds the service weakly and ends when it is dropped — ownership is carried by the
/// `manager`'s status sink chain and the connection layer.
fn build_usage_service(inputs: UsageInputs) -> Arc<UsageService> {
    let UsageInputs {
        probes,
        spawner,
        threads,
        scratch_root,
    } = inputs;
    let (usage, wakes) = UsageService::new(UsageParts {
        probes,
        spawner,
        scratch_root,
        threads,
        clock: Arc::new(OsUsageClock::new()),
        encoder: Arc::new(agent_conn::UsageEventEncoder),
    });
    match usage_service::schedule::spawn_scheduler(&usage, wakes) {
        Ok(_) => tracing::info!(thread = "usage-scheduler", "usage scheduler thread started"),
        Err(e) => tracing::warn!(
            error = %e,
            "could not launch the usage scheduler thread — running without automatic queries and deadline emits"
        ),
    }
    usage
}

/// The production status sink chain — `MessagingFlushSink(UsageObserveSink(DaemonStatusSink))`.
fn status_chain(
    fanout: Arc<dyn FrameFanout>,
    usage: Arc<UsageService>,
    flush_tx: tokio::sync::mpsc::UnboundedSender<messaging_host::FlushMsg>,
    idle_coalescer: Arc<messaging_host::IdleCoalescer>,
) -> messaging_host::MessagingFlushSink {
    messaging_host::MessagingFlushSink::new(
        UsageObserveSink::new(Box::new(DaemonStatusSink::new(fanout)), usage),
        flush_tx,
        idle_coalescer,
    )
}

// ── accept loop (shared by main + tests) ──────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn run_accept_loop(
    listening: Listening,
    wiring: DaemonWiring,
    multiview: MultiViewState,
    control_registry: Arc<control::registry::ControlRegistry>,
    messaging_slot: Arc<control::mcp_server::MessagingSlot>,
    // Command bus (ADR-0155/0156/0160) — the one copy binding the roster · seat table · stage-1 table. Not made here because the two control
    //   plane routes (discovery · call) use the same copy (`control::mcp_server::start_mcp_server_with_help`); separate copies would silently
    //   hide client-registered names from discovery and give relay another seat table.
    bus: command_delivery::CommandBus,
    // Expiry reaper — takes the one launched where the bus was made and stops it here (`CommandBus::new` produces it as a pair).
    sweeper: command_delivery::BusSweeper,
    expected_token: Arc<String>,
    shutdown_tx: watch::Sender<bool>,
    mut shutdown_rx: watch::Receiver<bool>,
    enable_ctrl_c: bool,
    keepalive: KeepaliveConfig,
) {
    // The fanout port is pulled from the very value the accept branch below registers, so "broadcast target ≠ registration target" cannot be
    //   written inside this function (ADR-0129); remaining representable places and the rule are in the `DaemonWiring` comment.
    let DaemonWiring {
        manager,
        registry,
        usage,
        library: _,
    } = wiring;
    let fanout: Arc<dyn FrameFanout> = Arc::new(registry.clone());
    let handlers: Arc<dyn crate::conn_port::ConnectionHandlerFactory> =
        Arc::new(agent_conn::AgentConnections::new(
            manager,
            multiview,
            fanout,
            control_registry,
            messaging_slot,
            bus.roster().clone(),
            bus.deliveries().clone(),
            Arc::clone(bus.locals()),
            usage,
            shutdown_tx,
        ));
    let server = link::server::Server::new(
        listening.clock(),
        registry,
        handlers,
        &expected_token,
        PROTOCOL_VERSION,
        keepalive,
    );

    let stop = async move {
        loop {
            tokio::select! {
                // StopDaemon command received — exit when the watch changes to true.
                res = shutdown_rx.changed() => {
                    match res {
                        Ok(()) if *shutdown_rx.borrow() => {
                            tracing::info!("shutdown signal (watch=true) received — leaving the accept loop");
                            break;
                        }
                        Ok(()) => {} // a change to false is ignored (does not happen now)
                        Err(_) => break, // all senders dropped — exit
                    }
                }
                _ = tokio::signal::ctrl_c(), if enable_ctrl_c => {
                    tracing::info!("Ctrl-C received — leaving the accept loop");
                    break;
                }
            }
        }
    };
    // Returns after every connection was closed with its queue flushed (the stop `Ack` · a last `Error`), or the flush deadline passed. The
    //   process ends soon after `run()` returns, so without this wait those frames could die with it (TRD trd-3 §3-0 ④).
    server.run(listening, stop).await;

    // ── command round trip cleanup (ADR-0154) ───────────────────────────────────────────────────
    // ★Waiting for the sweeper is what bounds the shutdown★: on its way out it answers every remaining seat
    //   (`CommandDeliveries::drain`) and those answers free the delivery tasks **at once**. Without the wait the seats
    //   and tasks live on after "stopped" is logged, until each seat's own deadline (7 s ordinary, up to 91 s for a
    //   launch — ADR-0360).
    // It comes here once regardless of the exit reason — three exit branches (StopDaemon · Ctrl-C · sender gone), and Ctrl-C does not touch the daemon shutdown watch.
    sweeper.stop().await;
}

// ── main body (production) ──────────────────────────────────────────────────────────────

/// The daemon body. `Err(code)` makes the caller (main) exit with that code; a normal exit (including already running) is Ok.
pub async fn run() -> Result<(), i32> {
    // 0) data_dir resolution comes before logging because the file log lives in that folder. It is pure (env lookup + path assembly), so no
    //    lines are lost; reversing it would send the startup failures of 1)~2) to stdout only, which goes nowhere in release.
    let layout = DataLayout::resolve();
    let data_dir = layout.root().to_path_buf();

    // 0.1) file log → panic hook.
    //    Masking is not included: init_logging does not hide keys; mask_secrets is only a helper and the caller applies it
    //    (docs/reference/logging-conventions.md).
    // ADR-0292
    let log_file = init_logging_and_panic_hook(&layout);

    // 1) data_dir creation + writability check.
    //    No fallback (ADR-0134 decision 4): an unwritable folder stops here; flowing elsewhere would give "I deleted the folder but the roster is alive".
    //    This line is all that remains of this failure: the file log is blocked in the same folder too, so the sink that fell back under `%TEMP%`
    //    receives it (base `logging` header). If even that fails, the owner is the client's pre-spawn check (ADR-0135) — the daemon has no screen.
    if let Err(e) = ensure_data_dir_writable(&data_dir) {
        // e already holds the folder path and the remedy (`data_dir::DataDirUnwritable`).
        tracing::error!(
            "cannot start the daemon because the data folder could not be prepared: {e}"
        );
        return Err(1);
    }
    if let Err(e) = layout.ensure_daemon_dirs() {
        tracing::error!(
            data_dir = %data_dir.display(),
            "cannot start because the daemon folders (daemon/state · daemon/run) could not be created: {e}"
        );
        return Err(1);
    }
    // ADR-0134 §Impact: the lock has no name, so this is the only means for a person to check which folder was taken.
    tracing::info!(
        data_dir = %data_dir.display(),
        log_file = ?log_file,
        "data folder decided"
    );

    // 2) Single instance guard = holding that data folder's connection file (`daemon\run\daemon.json`) (ADR-0135 decision 1 — the locked file
    //    and the file clients read are the same; the scope is the folder).
    //    The guard must live for the process lifetime (released on Drop = singleness broken). The handle obtained here writes the connection
    //    info in 8) — acquire → write is the invariant.
    //    Do not lump the three branches: duplicate (normal yield) · third-party obstruction · system error ask the user for different things.
    // ADR-0264
    use crate::instance::{AcquireError, Acquired};
    let daemon_path = layout.daemon_file();
    let mut guard = match crate::instance::acquire(&daemon_path) {
        Ok(Acquired::Held(g)) => g,
        Ok(Acquired::AlreadyRunning { pid }) => {
            // Which folder was held is the reason this line exists — without it "why does it not start" cannot be traced.
            tracing::info!(
                data_dir = %data_dir.display(),
                owner_pid = pid,
                "a daemon for this data folder is already running — exiting"
            );
            return Ok(());
        }
        // Do not conclude "not a duplicate daemon": if the first daemon got the lock and has not published yet (until 8), the diagnostic read sees an empty file and comes here.
        Err(e @ AcquireError::FileBusy { .. }) => {
            tracing::error!(
                lock_file = %daemon_path.display(),
                "cannot start because another program, or another daemon that has not published yet, holds {DAEMON_FILE} (check antivirus · indexers · backup): {e}"
            );
            return Err(1);
        }
        // A restart does not clear this state, so the wording must differ: FileBusy is "clears if you wait", this is "fix the permissions or extract elsewhere".
        Err(e @ AcquireError::AccessDenied { .. }) => {
            tracing::error!(
                lock_file = %daemon_path.display(),
                "cannot start because {DAEMON_FILE} cannot be written: {e}"
            );
            return Err(1);
        }
        Err(e) => {
            tracing::error!(
                lock_file = %daemon_path.display(),
                "failed to acquire the single instance guard: {e}"
            );
            return Err(1);
        }
    };

    // 2.5) Diagnostic log before overwriting the existing contents.
    //
    // PID liveness is no longer a veto (ADR-0134 decision 3): reaching here means we already hold this folder's file, proving no other daemon owns
    //   it. A PID check on top would make the daemon permanently unable to start when an unrelated process reused the old PID. Holding is
    //   the authority — do not revert.
    if let Some(prev) = crate::portfile::read(&daemon_path) {
        tracing::info!(
            pid = prev.pid,
            stale = crate::portfile::is_stale(&prev),
            "overwriting the existing daemon.json (the folder lock is ours)"
        );
    }

    // 3) bind → get the actual port.
    let listening = match Listening::bind_loopback() {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("WS listener bind failed: {e}");
            return Err(1);
        }
    };
    let port = listening.port();

    // 4) Generate the WS auth token.
    let token = match generate_token() {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("token generation failed: {e}");
            return Err(1);
        }
    };

    // ADR-0086: control channel token registry — a different concern from the daemon.json WS token in 4) (no mixing — ADR-0086 §Context).
    let control_registry = Arc::new(control::registry::ControlRegistry::new());

    // 5b) Multi-viewer negotiation state — shared by all connections.
    let multiview = MultiViewState::new();

    // 5b.5) Boot sweep (FIX 5). Must come before the MCP server and provision start: `control_registry` was just made empty, so every existing
    //   mcp-config now is a dead credential.
    let mcp_dir = control::mcp_config::McpDir::new(layout.mcp_config_dir());
    control::mcp_config::sweep_stale_configs(&mcp_dir);

    // 5c) Start the control channel MCP server.
    //     Fail-closed (FIX 1): a bind/start failure is fatal — keeping the daemon up with NoopControlChannel (the old behavior) would fake health
    //     without a control channel. It drops the made resources (WS listener · control_registry) and aborts startup with Err(1); agent
    //     orchestration is the foundation of §5 LLM-first control. Must come before any agent spawn.
    let manager_slot = Arc::new(control::mcp_server::ManagerSlot::new());
    let messaging_slot = Arc::new(control::mcp_server::MessagingSlot::new());
    let roster_broadcast_slot = Arc::new(control::mcp_server::RosterBroadcastSlot::new());
    let command_table_slot = Arc::new(control::mcp_server::CommandTableSlot::new());
    // Command bus (ADR-0155/0156/0160) — one copy of roster · seat table · stage-1 table. It comes before the MCP server: the two control routes
    //   use it (discovery the roster half, call the delivery machine). The same copy goes into the accept loop below — different copies would
    //   hide client-registered names from discovery only or keep the two entrances from seeing each other's round trips, with no runtime signal.
    // The stage-1 table goes in as a slot, not a value: the table is created after the manager assembly (stage 6), so it is empty now; capturing
    //   the value would freeze that empty table as "not my command" for the process lifetime, and `agent.*` would come back as unknown commands
    //   (that property of `command_delivery::LocalCommands`).
    // The reaper comes out paired with the bus: launched separately, an assembly deferring it to the accept loop becomes representable and control
    //   route calls in between hang with no eye on the deadline (`CommandBus::new`). The accept loop below stops it.
    let (command_bus, command_sweeper) = command_delivery::CommandBus::new(
        command_roster::CommandRoster::new(),
        command_delivery::CommandDeliveries::new(),
        Arc::new(control::commands::DaemonLocalCommands::new(
            command_table_slot.clone(),
        )),
    );
    let (flush_tx, flush_rx) = tokio::sync::mpsc::unbounded_channel::<messaging_host::FlushMsg>();
    let idle_coalescer = Arc::new(messaging_host::IdleCoalescer::new());

    // The handle must live for the process lifetime (drop = server shutdown).
    let (control, mut mcp_server_handle): (
        Arc<dyn engram_dashboard_agent::types::ControlChannel>,
        Option<control::mcp_server::McpServerHandle>,
    ) = match control::mcp_server::start_mcp_server_with_help(
        control_registry.clone(),
        manager_slot.clone(),
        messaging_slot.clone(),
        command_table_slot.clone(),
        command_bus.clone(),
        // ADR-0285: the help body source is anchored at the install location — it is built here so the server does not
        //   build it inside (a test passes a known body).
        control::help::HelpSource::from_install_root(),
    )
    .await
    {
        Ok(handle) => {
            let url = handle.url.clone();
            let send_exe = locate_send_exe();
            let channel = Arc::new(control::DaemonControlChannel::new(
                control_registry.clone(),
                url,
                mcp_dir,
                send_exe,
            ));
            (channel, Some(handle))
        }
        Err(e) => {
            // This is all of the fail-closed cleanup — the connection registry does not exist yet (stage 6, `build_daemon_wiring`).
            // The record has not been written to daemon.json yet (stage 8), but the acquire in 2) already opened (existing) or created (new folder) the
            //   file; dropping the guard leaves the old daemon's record as is, or a 0-byte file in a new folder. Both are overwritten by the next startup
            //   (the reader sees 0 bytes as "not ready yet").
            tracing::error!(
                "MCP server startup failed — the daemon is not started without the control channel (fail-closed): {e}"
            );
            drop(listening);
            return Err(1);
        }
    };

    // 6) AgentManager wiring.
    let wiring = build_daemon_wiring(&layout, control, flush_tx.clone(), idle_coalescer.clone());
    // `wiring` keeps holding the registry and hands it to the accept loop together — untying it would allow a mismatched pair (ADR-0129 `DaemonWiring`).
    let manager = wiring.manager.clone();
    manager_slot.set(manager.clone());
    // The wiring that lets attached clients see the roster the control route changed (ADR-0132). Without this line, `engram agent rename/new/move`
    //   succeeds yet the dashboard tree shows the old roster until an unrelated event (no error, no log).
    roster_broadcast_slot.set(wiring.roster_broadcast());
    // This table is the real entrance of control verbs (ADR-0155) — without it `/control/agent` is 503 on every request. It does not depend on
    //   the order relative to the fanout set above: the table holds the slot, read at notification time (`control::commands::make_daemon_table`).
    // The lease port is that `multiview` passed to the accept loop below (ADR-0231): `clone()` is a handle on the same shared state. A new
    //   `MultiViewState::new()` is wrong — an empty table would make bus · CLI cancels silently ignore the viewer's input lease.
    command_table_slot.set(Arc::new(control::commands::make_daemon_table(
        manager.clone(),
        roster_broadcast_slot.clone(),
        Arc::new(multiview.clone()),
        wiring.usage.clone(),
        wiring.library_commands(),
    )));

    // 6.4) idle gate assembly — turn observation is loaded directly by the core from the output pump, so nothing to wire here.
    let idle_notifier = Arc::new(messaging_host::ChannelIdleNotifier::new(
        flush_tx,
        idle_coalescer.clone(),
    ));
    let busy = Arc::new(messaging_host::busy_gate_for_manager(
        manager.clone(),
        idle_notifier.clone(),
    ));

    // 6.5) MessagingService assembly.
    let messaging = Arc::new(
        messaging_host::messaging_for_manager_gated(
            manager.clone(),
            control_registry.clone(),
            busy.clone(),
        )
        .with_flush_trigger(idle_notifier.clone()),
    );
    messaging_slot.set(messaging.clone());

    // 6.6) Maintenance sweep task (a long-lived tokio task for the daemon's lifetime).
    //    Period = 60s (internal choice): dense against the TTL (24h) so expiry is not delayed much, negligible load at a low message rate.
    //    An async task without `spawn_blocking`: neither sweep does a blocking write to child stdin (injection is the flush lane's job — service.rs
    //    `deliver_notice` comment), so abort takes effect at once.
    //    Coupling: the overdue decision's resolution is this period — ingress's `MIN_REPLY_BY_SECS` (1 minute) pairs with it; change both together.
    const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
    let sweep_messaging = messaging.clone();
    let sweep_busy = busy.clone();
    let sweep_task = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
        // Skip the immediate first tick (no parking right after boot).
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let now = std::time::Instant::now();
            // A panic in the tick body is outside the containment mark, so the panic hook of 0.1) ends the daemon (ADR-0292).
            sweep_messaging.sweep(now);
            sweep_busy.sweep_stale_busy(now);
        }
    });

    // 6.7) flush worker task (next to the sweep task — aborted on shutdown).
    let flush_worker = messaging_host::spawn_flush_worker(
        flush_rx,
        messaging_host::FlushWiring {
            messaging: messaging_slot.clone(),
            idle: idle_coalescer.clone(),
        },
    );

    // 7) Keep the token for auth comparison in an Arc (the token moves into daemon.json, so a shared copy is taken first).
    let expected_token = Arc::new(token.clone());

    // 8) Write daemon.json.
    let start_time = engram_dashboard_platform::process::current_process_start_time().unwrap_or(0);
    let info = crate::portfile::DaemonInfo {
        pid: std::process::id(),
        host: "127.0.0.1".to_string(),
        port,
        token: engram_dashboard_protocol::Secret::new(token),
        protocol_version: PROTOCOL_VERSION,
        start_time,
    };
    // Write with the handle obtained in 2) (ADR-0135): a temp file + rename is refused because we hold the file with delete sharing closed, and
    //   writing before acquisition would mix two daemons' writes.
    if let Err(e) = guard.publish(&info) {
        tracing::error!("{DAEMON_FILE} write failed: {e}");
        return Err(1);
    }
    tracing::info!(
        port,
        pid = info.pid,
        protocol_version = PROTOCOL_VERSION,
        path = %daemon_path.display(),
        "daemon started — {DAEMON_FILE} written"
    );

    // Do not revive a periodic job that republishes a vanished file (ADR-0135 decision 4): the file cannot be deleted while we hold it, and an app
    //   silently rewriting a file the user just deleted is reported as a bug in every precedent.

    // 9) Automatic boot resume default OFF (2026-07-09, user decision): `mgr.restore_all()`, which revived every auto_restore=true profile at boot,
    //   is not called. The auto_restore field · reaper disposition · restore_all() implementation stay; event-driven restore of specific agents will
    //   call restore_all() from an explicit command in the future. `handle` is a no-op kept for the abort/await contract below.
    //   (This stopgap reverses the ADR-0016 "boot restore" default — record it in an ADR when designing the proper opt-in.)
    let restore_handle = tokio::task::spawn_blocking(|| {});

    // 10) Shutdown signal channel (watch); the StopDaemon command triggers shutdown through it.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // 11) accept loop.
    tracing::info!("accept loop started (WS handling active)");
    run_accept_loop(
        listening,
        wiring,
        multiview,
        control_registry,
        messaging_slot.clone(),
        command_bus,
        command_sweeper,
        expected_token,
        shutdown_tx,
        shutdown_rx,
        true, // production: Ctrl-C graceful shutdown enabled
        KeepaliveConfig::default(),
    )
    .await;

    // 12) graceful shutdown.
    restore_handle.abort();
    let _ = restore_handle.await;

    // No remaining expiry processing: parking is in-memory, so process exit = state gone (spec §0 "no persistence"); the sweep does no blocking
    //    writes (6.6), so abort takes effect at once — unlike the flush worker.
    sweep_task.abort();
    let _ = sweep_task.await;

    // Shutdown order (BLOCK): agent cleanup (shutdown_all) comes before ending the flush worker. A live child that does not drain stdin keeps the
    //   flush's blocking write uncut by abort, so the reverse order would hang shutdown forever (`FlushWorkerHandles::shutdown`).
    let mgr = manager.clone();
    if let Err(e) = tokio::task::spawn_blocking(move || mgr.shutdown_all()).await {
        tracing::warn!("shutdown_all join failed: {e}");
    }

    flush_worker.shutdown().await;

    // ADR-0086: graceful shutdown of the control channel MCP server (after agent cleanup — remaining sessions are cleaned up with it).
    if let Some(handle) = mcp_server_handle.take() {
        handle.shutdown().await;
        tracing::info!("MCP server shutdown complete");
    }

    // daemon.json is left behind — the next boot ignores it by the stale decision.
    tracing::info!("daemon shutdown complete");
    Ok(())
}

// ── test server startup helpers ───────────────────────────────────────────────────────
// Every item in this section is behind the `test-support` feature (not shipped in production builds) — new items get the same cfg, or
//   unused-code warnings appear without the feature and test scaffolding leaks as public API.
// ADR-0286

/// The handle of a test server launched in-process. Tests must call `shutdown().await` at the end — drop only ends the accept loop and
/// does not clean up children (zombie PTYs).
///
/// The single instance guard · daemon.json are intentionally omitted (real-process-only); that path is verified with a real .exe by the
/// #[ignore]/harness of `tests/ws_e2e.rs`.
///
/// Exists only with the `test-support` feature — tests of other packages turn it on in their daemon dev dependency.
#[cfg(feature = "test-support")]
pub struct TestServerHandle {
    pub port: u16,
    pub token: String,
    pub manager: Arc<AgentManager>,
    accept_handle: tokio::task::JoinHandle<()>,
    shutdown_tx: watch::Sender<bool>,
    flush_worker: messaging_host::FlushWorkerHandles,
}

#[cfg(feature = "test-support")]
impl TestServerHandle {
    /// Brings the server down gracefully. The order kill all agents → end the flush worker is load-bearing (basis = the run() shutdown
    /// comment); it waits synchronously through shutdown_all to prevent zombie PTYs.
    pub async fn shutdown(self) {
        let _ = self.shutdown_tx.send(true);
        let _ = self.accept_handle.await;
        let mgr = self.manager.clone();
        let _ = tokio::task::spawn_blocking(move || mgr.shutdown_all()).await;
        self.flush_worker.shutdown().await;
    }

    /// Whether the daemon's shutdown trigger has fired — by a `StopDaemon` or by [`Self::shutdown`].
    pub fn shutdown_signalled(&self) -> bool {
        *self.shutdown_tx.borrow()
    }
}

#[cfg(feature = "test-support")]
pub async fn start_test_server() -> std::io::Result<TestServerHandle> {
    let store: Arc<dyn ProfileStore> = Arc::new(MemProfileStore::default());
    start_test_server_with_store(store).await
}

/// The keepalive-injecting form — keepalive (half-open detection) tests slot in short ping/idle values (production defaults would take tens of seconds).
#[cfg(feature = "test-support")]
pub async fn start_test_server_with_keepalive(
    keepalive: KeepaliveConfig,
) -> std::io::Result<TestServerHandle> {
    let store: Arc<dyn ProfileStore> = Arc::new(MemProfileStore::default());
    start_test_server_inner(store, keepalive).await
}

/// The store-injecting form — restore · persist tests slot in the store directly.
#[cfg(feature = "test-support")]
pub async fn start_test_server_with_store(
    store: Arc<dyn ProfileStore>,
) -> std::io::Result<TestServerHandle> {
    start_test_server_inner(store, KeepaliveConfig::default()).await
}

#[cfg(feature = "test-support")]
async fn start_test_server_inner(
    store: Arc<dyn ProfileStore>,
    keepalive: KeepaliveConfig,
) -> std::io::Result<TestServerHandle> {
    let listening = Listening::bind_loopback().map_err(std::io::Error::other)?;
    let port = listening.port();

    // getrandom::Error does not implement std::error::Error, so it is converted to a message and wrapped in io::Error.
    let token =
        generate_token().map_err(|e| std::io::Error::other(format!("token gen failed: {e}")))?;
    let expected_token = Arc::new(token.clone());

    let multiview = MultiViewState::new();
    // ADR-0096: the home of the envelope format global state used by WS dispatch (SetEnvelopeFormat). The test server wires no MCP server ·
    //   control channel (Noop below), so a new standalone registry serves only the accept loop — the same Arc, so the value dispatch wrote
    //   is observable for that server's lifetime (production shares control_registry).
    let control_registry = Arc::new(control::registry::ControlRegistry::new());
    // WS tests do not use the control channel → Noop (control channel integration tests are on the control::mcp_server side).
    let control: Arc<dyn engram_dashboard_agent::types::ControlChannel> =
        Arc::new(engram_dashboard_agent::types::NoopControlChannel);
    // WS tests do not verify send/flush, but the status sink wrapper requires the channel, so the messaging wiring is set up with it.
    let messaging_slot = Arc::new(control::mcp_server::MessagingSlot::new());
    let (flush_tx, flush_rx) = tokio::sync::mpsc::unbounded_channel::<messaging_host::FlushMsg>();
    let idle_coalescer = Arc::new(messaging_host::IdleCoalescer::new());
    // 0 probers — on this server no real CLI starts by any path (subscription · ⟳ have no cells, so no query can start). The connection-layer
    //   tests of the usage wire are in `agent_conn` with a service carrying fake probers.
    let wiring = build_daemon_wiring_with_store(
        store,
        UsageInputs {
            probes: Vec::new(),
            spawner: Arc::new(OsProbeSpawner),
            threads: Arc::new(OsProbeThreads),
            scratch_root: std::env::temp_dir(),
        },
        // ADR-0311: real kits stay out of the WS tests — tests that need them build `LibraryService` themselves.
        None,
        control,
        flush_tx.clone(),
        idle_coalescer.clone(),
    );
    let manager = wiring.manager.clone();
    // Plug the real entrance of control verbs in the same place as production (ADR-0155): this table is stage 1 of bus delivery and the discovery
    //   list's own part. Unplugged, `agent.*` would come back as "unknown command" only on this server, and the WS integration tests would read
    //   that as production behavior.
    let command_table_slot = Arc::new(control::mcp_server::CommandTableSlot::new());
    let roster_broadcast_slot = Arc::new(control::mcp_server::RosterBroadcastSlot::new());
    roster_broadcast_slot.set(wiring.roster_broadcast());
    // Lease port = a handle (`clone()`) of the same shared state as the accept loop — a new `MultiViewState::new()` is wrong (see production assembly).
    command_table_slot.set(Arc::new(control::commands::make_daemon_table(
        manager.clone(),
        roster_broadcast_slot,
        Arc::new(multiview.clone()),
        wiring.usage.clone(),
        wiring.library_commands(),
    )));
    // This server does not wire the MCP control plane (Noop above), so the accept loop is the only consumer of the bus; the assembly uses the same function as production.
    let (command_bus, command_sweeper) = command_delivery::CommandBus::new(
        command_roster::CommandRoster::new(),
        command_delivery::CommandDeliveries::new(),
        Arc::new(control::commands::DaemonLocalCommands::new(
            command_table_slot,
        )),
    );
    // Keep the wiring the same as production run() — no branch "with no gate only on the test server".
    let idle_notifier = Arc::new(messaging_host::ChannelIdleNotifier::new(
        flush_tx,
        idle_coalescer.clone(),
    ));
    let busy = Arc::new(messaging_host::busy_gate_for_manager(
        manager.clone(),
        idle_notifier.clone(),
    ));
    messaging_slot.set(Arc::new(
        messaging_host::messaging_for_manager_gated(
            manager.clone(),
            control_registry.clone(),
            busy.clone(),
        )
        .with_flush_trigger(idle_notifier),
    ));
    let flush_worker = messaging_host::spawn_flush_worker(
        flush_rx,
        messaging_host::FlushWiring {
            messaging: messaging_slot.clone(),
            idle: idle_coalescer,
        },
    );

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let accept_handle = {
        let shutdown_tx = shutdown_tx.clone();
        tokio::spawn(async move {
            run_accept_loop(
                listening,
                wiring,
                multiview,
                control_registry,
                messaging_slot,
                command_bus,
                command_sweeper,
                expected_token,
                shutdown_tx,
                shutdown_rx,
                false, // test: Ctrl-C not installed (shutdown only via watch)
                keepalive,
            )
            .await;
        })
    };

    Ok(TestServerHandle {
        port,
        token,
        manager,
        accept_handle,
        shutdown_tx,
        flush_worker,
    })
}

/// Stands in for production's `FileProfileStore` for test isolation (no disk/Embedded pollution).
#[cfg(feature = "test-support")]
#[derive(Default)]
struct MemProfileStore {
    saved: std::sync::Mutex<Vec<engram_dashboard_agent::profile::AgentProfile>>,
}

#[cfg(feature = "test-support")]
impl ProfileStore for MemProfileStore {
    fn save(
        &self,
        profiles: &[engram_dashboard_agent::profile::AgentProfile],
    ) -> Result<(), engram_dashboard_agent::profile::StoreError> {
        *self.saved.lock().expect("mem store poisoned") = profiles.to_vec();
        Ok(())
    }
    fn load(&self) -> Vec<engram_dashboard_agent::profile::AgentProfile> {
        self.saved.lock().expect("mem store poisoned").clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All three links are in the production status sink chain — usage observations go as far as the service and the subscribed connections (not
    /// all connections), state events as far as the fanout. Link order is tied by type in the constructors (`MessagingFlushSink::new` takes a `UsageObserveSink`).
    #[test]
    fn the_status_chain_feeds_the_usage_service_and_the_fanout() {
        use crate::conn_port::{Frame, FrameSink};
        use engram_dashboard_agent::types::{AgentStatus, StatusSink};
        use engram_dashboard_agent::usage::{UsageObservation, UsageSource, WindowObs};

        let real = engram_dashboard_agent::backend::usage_probes()[0];
        let (probe, _gate) = usage_service::fakes::gated_probe(real, 10.0, 2_000_000_000);
        let usage = usage_service::fakes::service(vec![probe], Arc::new(OsUsageClock::new()));
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Frame>(8);
        let frames: Arc<dyn FrameSink> = Arc::new(test_doubles::FakeFrameSink::new(tx));
        usage.attach(1, agent_conn::ConnUsageOutlet::new(frames));
        usage.replace_subscription(1, [real.key()].into());
        assert!(rx.try_recv().is_ok(), "the first sheet");

        let fanout = Arc::new(test_doubles::RecordingFanout::new());
        let (flush_tx, _flush_rx) = tokio::sync::mpsc::unbounded_channel();
        let chain = status_chain(
            fanout.clone(),
            usage,
            flush_tx,
            Arc::new(messaging_host::IdleCoalescer::new()),
        );

        chain.usage_observed(UsageObservation {
            vendor: real.key(),
            five_hour: Some(WindowObs {
                used_pct: Some(40.0),
                resets_at: None,
            }),
            weekly: None,
            model_scoped: None,
            plan: None,
            source: UsageSource::Passive,
            limits_unavailable: None,
        });
        let Ok(Frame::Text(text)) = rx.try_recv() else {
            panic!("the observation must reach the subscribed connection through the service");
        };
        assert!(text.contains("UsageLimitsUpdated"), "{text}");
        assert!(fanout.texts().is_empty(), "{:?}", fanout.texts());

        chain.status_changed(uuid::Uuid::new_v4(), AgentStatus::Killed, 1);
        assert!(fanout.sole_text().contains("StatusChanged"));
    }

    #[test]
    fn token_is_64_hex_chars() {
        let t = generate_token().expect("token gen");
        assert_eq!(t.len(), 64, "256-bit = 32B → 64 hex chars");
        assert!(
            t.chars().all(|c| c.is_ascii_hexdigit()),
            "must contain only hex characters"
        );
    }

    #[test]
    fn root_generation_bases_are_random_below_2_pow_52() {
        let bases: Vec<u64> = (0..4).map(|_| root_generation_base()).collect();
        assert!(bases.iter().all(|&base| base < 1 << 52), "{bases:?}");
        assert!(bases.windows(2).any(|pair| pair[0] != pair[1]), "{bases:?}");
    }

    #[test]
    fn tokens_are_unique() {
        let a = generate_token().unwrap();
        let b = generate_token().unwrap();
        assert_ne!(a, b);
    }

    /// The production text of `lib.rs` up to its test module.
    fn production_source() -> &'static str {
        include_str!("lib.rs")
            .split("mod tests {")
            .next()
            .expect("production part")
    }

    /// The production text of one function — from its signature to the next top-level `fn`.
    fn body_of(signature: &str) -> &'static str {
        let production = production_source();
        let start = production
            .find(signature)
            .unwrap_or_else(|| panic!("`{signature}` is not in the production part"));
        let rest = &production[start + signature.len()..];
        let end = rest.find("\nfn ").unwrap_or(rest.len());
        &rest[..end]
    }

    /// ★The boot sweep runs before the library service is plugged in★ — moved after it, a launch could have its fresh
    /// folder removed; no runtime test sees that (boot restore is off, so nothing launches at boot), only this one.
    #[test]
    fn the_boot_sweeps_before_the_plug() {
        let boot = body_of("fn build_daemon_wiring(");
        let sweep = boot
            .find("sweep_launch_dir(&launch_root)")
            .expect("the boot sweep of the launch folder");
        let plug = boot
            .find("build_daemon_wiring_with_store(")
            .expect("the assembly that plugs the library");
        assert!(sweep < plug, "the sweep runs after the plug");
        assert!(
            body_of("fn build_daemon_wiring_with_store(").contains(".with_launch_ports("),
            "the assembly no longer plugs the launch kits"
        );
    }

    /// ★The boot path never walks the library★ (ADR-0382): neither `build_daemon_wiring` nor the assembly it calls,
    /// `build_daemon_wiring_with_store`, calls a catalog reader (each would run or wait for the first scan before the
    /// daemon is reachable), and the boot scan starts right after the first start, unconditionally — not only when the
    /// first start published. `build_daemon_wiring` hard-codes the real store, so only the source shows this; the runtime
    /// side is the production-wiring tests below.
    #[test]
    fn the_boot_scan_starts_after_the_first_start_and_the_boot_path_reads_no_catalog() {
        let boot = body_of("fn build_daemon_wiring(");
        let assembly = body_of("fn build_daemon_wiring_with_store(");
        for reader in [
            ".first_scan_join(",
            ".refresh(",
            ".list(",
            ".folders(",
            ".read(",
            ".resolve_",
        ] {
            assert!(!boot.contains(reader), "the boot path calls `{reader}`");
            assert!(!assembly.contains(reader), "the assembly calls `{reader}`");
        }
        let first = boot.find(".first_start()").expect("the first-start step");
        let scan = boot.find(".start_boot_scan()").expect("the boot scan");
        assert!(first < scan, "the boot scan starts before the first start");
        let between = &boot[first..scan];
        for branch in ["if ", "match ", "else"] {
            assert!(
                !between.contains(branch),
                "the boot scan depends on the first start's outcome: {between}"
            );
        }
    }

    /// ★The manager is plugged into the library as its profile-edit port before any command table is set★
    /// (ADR-0380): the assembly plugs it right after it wraps the manager, and `run` builds the wiring before it sets
    /// the command table. Plugged after the table, an early agent-preset command would answer
    /// `profile-edit-unavailable`; no runtime test orders the two, only this one.
    #[test]
    fn the_profile_edit_port_is_plugged_before_the_command_table_is_set() {
        let assembly = body_of("fn build_daemon_wiring_with_store(");
        let wrapped = assembly
            .find("Arc::new(manager)")
            .expect("the assembly wraps the manager");
        let plugged = assembly
            .find(".plug_profile_edit(")
            .expect("the assembly plugs the profile-edit port");
        assert!(
            wrapped < plugged,
            "the port is plugged before the manager is wrapped"
        );
        let run = body_of("pub async fn run(");
        let wiring = run
            .find("build_daemon_wiring(")
            .expect("`run` builds the wiring");
        let table = run
            .find("command_table_slot.set(")
            .expect("`run` sets the command table");
        assert!(
            wiring < table,
            "the command table is set before the wiring is built"
        );
    }

    /// ★Both assemblies give the command table the wiring's library port★ (ADR-0386): production `run` and the test
    /// server pass `DaemonWiring::library_commands` — the kept service, or `NoLibrary` when there is none. A table
    /// built with another port would edit a library the launches do not read; only the source shows which port `run`
    /// passes (the runtime side is the test below).
    #[test]
    fn both_assemblies_give_the_command_table_the_wirings_library_port() {
        for signature in ["pub async fn run(", "async fn start_test_server_inner("] {
            let body = body_of(signature);
            let table = body
                .find("make_daemon_table(")
                .unwrap_or_else(|| panic!("`{signature}` builds the command table"));
            let call = &body[table..table + body[table..].find(")));").expect("the call ends")];
            assert!(
                call.contains("wiring.library_commands()"),
                "`{signature}` passes another library port: {call}"
            );
        }
    }

    /// ★The library port of the production wiring is the kept service★: a root set through the port is the root the
    /// service the manager launches through reports.
    #[test]
    fn the_production_library_port_is_the_kept_service() {
        let temp = std::env::temp_dir().join(format!("engram-wiring-{}", uuid::Uuid::new_v4()));
        let layout = DataLayout::new(&temp);
        let wiring = production_wiring(&layout);
        let library = wiring
            .library
            .clone()
            .expect("production plugs the library");
        wait_boot_scan(&library);
        let moved = temp.join("moved-library");

        let answer = wiring
            .library_commands()
            .set_root(&moved.display().to_string(), true)
            .expect("set root through the port");

        let kept = library.status();
        assert_eq!(kept.root.as_deref(), Some(moved.as_path()));
        assert_eq!(answer.root_generation, kept.root_generation);
        drop(wiring);
        let _ = std::fs::remove_dir_all(&temp);
    }

    /// The debug launch-block hook is gone with the real kits (its env vars included).
    #[test]
    fn the_debug_launch_block_hook_is_gone() {
        let production = production_source();
        for symbol in ["qa_block", "QaBlockKits", "ENGRAM_QA_BLOCK"] {
            assert!(!production.contains(symbol), "{symbol}");
        }
    }

    /// ★The production assembly plugs the real kits★ (WS tests run the Noop kits, so nothing else sees this): built
    /// over a temp data folder, it sweeps what an earlier daemon left in the launch folder, and the plugged service
    /// launches a plain claude agent with the built-in default preset (`engram/default`) from the repo `builtin\` — a real plan whose merged instructions file,
    /// in this launch's volatile folder, is the shipped priming. No process is spawned.
    #[test]
    fn the_production_wiring_plugs_the_library_with_the_builtin_priming() {
        use engram_dashboard_agent::launch::LaunchKitSource;
        use engram_dashboard_agent::profile::{AgentCommand, AgentOutputFormat, AgentProfile};

        let temp = std::env::temp_dir().join(format!("engram-wiring-{}", uuid::Uuid::new_v4()));
        let layout = DataLayout::new(&temp);
        let leftover = layout.launch_dir().join("stale-0");
        std::fs::create_dir_all(&leftover).unwrap();
        std::fs::write(leftover.join("instructions.md"), "old").unwrap();
        let work = temp.join("work");
        std::fs::create_dir_all(&work).unwrap();

        let wiring = production_wiring(&layout);
        let leftover_gone = !leftover.exists();
        let library = wiring
            .library
            .clone()
            .expect("production plugs the library");
        wait_boot_scan(&library);

        let profile = AgentProfile::new(
            "wired".into(),
            AgentCommand::Claude {
                extra_args: Vec::new(),
                output_format: AgentOutputFormat::StreamJson,
            },
            work.clone(),
            Vec::new(),
            false,
        );
        let scope = library.begin_launch();
        let plan = library
            .check_in(&scope, &profile)
            .expect("the built-in default preset passes");
        let kit = library
            .prepare_in(&scope, plan, profile.id, 1)
            .expect("prepared");
        let written = kit
            .instructions_file
            .as_deref()
            .map(std::fs::read_to_string)
            .transpose()
            .unwrap();
        let volatile = kit.volatile_dir.clone();
        library.release(profile.id, 1);
        wiring.manager.shutdown_all();
        let _ = std::fs::remove_dir_all(&temp);

        assert!(leftover_gone, "the boot sweep did not run");
        assert!(kit.plan.real && kit.plan.include_priming, "{:?}", kit.plan);
        assert_eq!(
            volatile,
            Some(layout.launch_dir().join(format!("{}-1", profile.id)))
        );
        let priming = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../builtin/instructions/priming.md"),
        )
        .unwrap();
        assert_eq!(
            written,
            Some(engram_dashboard_asset_library::launch::merge_instructions(
                Some(&priming),
                &[]
            ))
        );
        assert!(kit.instructions_hash.is_some());
    }

    /// ★The production assembly reads the root setting and the identity from the state folder and runs the first
    /// start★: over a fresh data folder the default place `<data>\library` is created, `library.json` holds the
    /// setting at its default value, the kept service publishes that root, and its status carries the id
    /// `identity.json` holds. A second assembly over the same folder keeps both, and starts at another root generation
    /// base: after the same number of root changes, the first run's generation is stale.
    #[test]
    fn the_production_wiring_runs_the_first_start_over_the_state_folder() {
        use engram_dashboard_asset_library::{RootSettingKind, RootStatus};

        let temp = std::env::temp_dir().join(format!("engram-wiring-{}", uuid::Uuid::new_v4()));
        let layout = DataLayout::new(&temp);
        let state = layout.daemon_state_dir();
        // `kept` = a generation from an earlier run, tried after one root change of this run (the first run's
        //   first start made one).
        let status_of = |layout: &DataLayout, kept: Option<u64>| {
            let wiring = production_wiring(layout);
            let library = wiring
                .library
                .as_ref()
                .expect("production plugs the library");
            // The first run's boot scan follows its first start; the second run's is a normal start's.
            wait_boot_scan(library);
            let kept_accepted = kept.map(|kept| {
                library
                    .set_root(&layout.default_library_root(), false)
                    .expect("re-set the same root");
                library
                    .change_at(Some(kept), |_, _, _| ((), Vec::new()))
                    .is_ok()
            });
            let status = library.status();
            wiring.manager.shutdown_all();
            (status, kept_accepted)
        };
        let (first, _) = status_of(&layout, None);
        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(state.join("library.json")).unwrap())
                .unwrap();
        let identity: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(state.join("identity.json")).unwrap())
                .unwrap();
        let library_made = temp.join("library").is_dir();
        let (second, kept_accepted) = status_of(&layout, Some(first.root_generation));
        let _ = std::fs::remove_dir_all(&temp);

        assert!(library_made);
        assert_eq!(
            config,
            serde_json::json!({ "schema_version": 2, "mounts": [{ "name": "library", "path": "default" }] })
        );
        assert_eq!(first.root, Some(temp.join("library")));
        assert_eq!(first.root, Some(layout.default_library_root()));
        assert_eq!(
            (first.state.clone(), first.root_setting),
            (RootStatus::Ok, Some(RootSettingKind::Default))
        );
        assert_eq!(
            identity["daemon_id"].as_str(),
            Some(first.daemon_id.as_str())
        );
        assert_ne!(second.root_generation, first.root_generation);
        assert_eq!(
            kept_accepted,
            Some(false),
            "a generation kept from the earlier run is stale"
        );
        assert_eq!(
            (second.root, second.daemon_id),
            (first.root, first.daemon_id),
            "the next start reads both back"
        );
    }

    /// ★The production library writes agent attachments through the very manager the wiring keeps★ (ADR-0380): with
    /// the cell empty the body would refuse `profile-edit-unavailable`, and a port over another registry would leave the
    /// wiring's roster unmoved. A Default-slot detach needs no library root.
    #[test]
    fn the_production_wiring_plugs_the_manager_as_the_profile_edit_port() {
        use engram_dashboard_agent::profile::{AgentCommand, AgentOutputFormat, AgentProfile};

        let temp = std::env::temp_dir().join(format!("engram-wiring-{}", uuid::Uuid::new_v4()));
        let layout = DataLayout::new(&temp);
        let work = temp.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let wiring = production_wiring(&layout);
        let library = wiring
            .library
            .as_ref()
            .expect("production plugs the library");
        wait_boot_scan(library);
        let created = wiring
            .manager
            .try_create_agent(AgentProfile::new(
                "port".into(),
                AgentCommand::Claude {
                    extra_args: Vec::new(),
                    output_format: AgentOutputFormat::StreamJson,
                },
                work,
                Vec::new(),
                false,
            ))
            .expect("create the agent");
        let before = wiring
            .manager
            .agent_snapshot(created.id)
            .map(|p| p.default_preset);
        let detached =
            library.detach_preset(created.id, &["engram/default".to_owned()], true, None);
        let after = wiring
            .manager
            .agent_snapshot(created.id)
            .map(|p| p.default_preset);
        wiring.manager.shutdown_all();
        let _ = std::fs::remove_dir_all(&temp);

        let view = detached.expect("the detach goes through the plugged port");
        assert!(!view.default_preset);
        assert_eq!((before, after), (Some(true), Some(false)));
    }

    /// Waits (at most 10 s, then fails) until the boot scan `build_daemon_wiring` started has applied the first scan —
    /// nothing else scans in these tests — so no detached walk outlives the test's temp folder.
    fn wait_boot_scan(library: &LibraryService) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while library.numbers().swap_ins == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "the boot scan never applied a scan"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// The production assembly over `layout`. ★Built inside a log capture★ — a boot log callsite is then hit only inside
    /// one ([`log_capture`] "Observation conditions" ①).
    fn production_wiring(layout: &DataLayout) -> DaemonWiring {
        let (flush_tx, _flush_rx) = tokio::sync::mpsc::unbounded_channel();
        let (wiring, _logged) = log_capture::capture_loud(|| {
            build_daemon_wiring(
                layout,
                Arc::new(engram_dashboard_agent::types::NoopControlChannel),
                flush_tx,
                Arc::new(messaging_host::IdleCoalescer::new()),
            )
        });
        wiring
    }

    /// ★The production manager launches through the very library service the wiring keeps★ (the Noop kits, or a
    /// second service, would pass every other non-ignored test): a claude agent with a library preset whose mount is
    /// not registered (a corrupt `library.json` leaves the mounts unknown) is not refused by the library — the preset
    /// is skipped (ADR-0387 · ADR-0400 decision 19) and the launch gets as far as the spawn, which fails on the working folder; once the kept service moves to a
    /// root whose copy of that preset is broken, the manager's refusal is the library's row for it. A cwd that does
    /// not exist — no process is spawned.
    #[test]
    fn the_production_manager_launches_through_the_kept_library_service() {
        use engram_dashboard_agent::launch::BlockReason;
        use engram_dashboard_agent::manager::NoticeDelivery;
        use engram_dashboard_agent::profile::{
            AgentCommand, AgentOutputFormat, AgentProfile, AttachedPreset,
        };

        let temp = std::env::temp_dir().join(format!("engram-wiring-{}", uuid::Uuid::new_v4()));
        let layout = DataLayout::new(&temp);
        let library_root = temp.join("library");
        std::fs::create_dir_all(&library_root).unwrap();
        std::fs::create_dir_all(layout.daemon_state_dir()).unwrap();
        std::fs::write(layout.daemon_state_dir().join("library.json"), "not json").unwrap();
        std::fs::write(library_root.join("P.yaml"), "kind: preset\nversion: [\n").unwrap();
        let wiring = production_wiring(&layout);
        let profile = || {
            let mut profile = AgentProfile::new(
                "wired-preset".into(),
                AgentCommand::Claude {
                    extra_args: Vec::new(),
                    output_format: AgentOutputFormat::StreamJson,
                },
                temp.join("no-such-folder"),
                Vec::new(),
                false,
            );
            profile.presets = vec![AttachedPreset {
                path: "library/P.yaml".to_string(),
            }];
            profile
        };

        let without_root = wiring
            .manager
            .create_and_activate(&profile(), NoticeDelivery::Withheld)
            .expect_err("the spawn fails on the working folder");
        let moved = wiring
            .library
            .as_ref()
            .map(|l| l.change_root(library_root.clone()));
        let refused = wiring
            .manager
            .create_and_activate(&profile(), NoticeDelivery::Withheld)
            .expect_err("the launch is refused");
        let listed = wiring.manager.list_agents().len();
        wiring.manager.shutdown_all();
        let _ = std::fs::remove_dir_all(&temp);

        // The spawn is the first step that needs the working folder (Windows answers `ERROR_DIRECTORY`, os error 267).
        assert!(
            matches!(
                without_root.error,
                engram_dashboard_agent::types::PtyError::SpawnFailed(_)
            ),
            "a preset in no known mount does not block (its items are skipped); the spawn fails on the missing working folder: {:?}",
            without_root.error
        );
        assert_eq!(listed, 0, "no process was spawned");
        assert!(
            matches!(moved, Some(Ok(_))),
            "the kept service did not move: {moved:?}"
        );
        assert!(!refused.created, "the refused launch was added");
        let with_root = match refused.error {
            engram_dashboard_agent::types::PtyError::LaunchBlocked(blocked) => blocked.reasons,
            other => panic!("not the library's block: {other:?}"),
        };
        assert!(
            with_root.iter().any(|reason| matches!(
                reason,
                BlockReason::Broken { item, reason, .. }
                    if item == "library/P.yaml" && reason.code == "yaml-syntax"
            )),
            "{with_root:?}"
        );
        assert!(
            !with_root
                .iter()
                .any(|reason| matches!(reason, BlockReason::LibraryUnavailable { .. })),
            "the manager does not see the kept service's root: {with_root:?}"
        );
    }
}
