//! The table of commands the daemon can execute directly — the only place real dependencies come in (rule T-1).
//!
//! Three surfaces call this table through the one [`call_daemon_command`], so none skips the entrance check (ADR-0157): the family route
//! (`/control/agent`, `agent.rs`) · the full-name route (`/control/call`, `catalog.rs`) · command bus delivery stage 1 (`command_delivery`).
//! If the table is not plugged into the slot, the two routes are 503 and delivery is a stage-1 miss.
//! Declarations are not here: the contracts of `agent.*`·`usage.*`·`library.*` belong to agent (ADR-0155 decision 1); this file plugs the
//! daemon's real things (manager · roster notification fanout · usage service · library service) into them. Daemon-own commands (`mail.*`)
//! would add their declaration block here.
// ADR-0155

use std::sync::Arc;

use crate::conn_port::ConnId;
use engram_dashboard_agent::commands::{
    make_table, resolve_in, AgentBackend, AgentCommandHost, AgentRosterRow, LibraryCommandHost,
    LibraryFoldersOk, LibraryKind, LibraryListOk, LibraryNewFileKind, LibraryRefreshOk,
    LibraryRelinkOk, LibraryStatusOk, RefreshErrorRow, RosterChanged, UsageCommandHost,
    UsageVendorRow, INPUT_AFFECTING,
};
use engram_dashboard_agent::launch::{escape_controls, Reason};
use engram_dashboard_agent::library_commands::{LibraryReadView, PortClass, PortRefusal};
use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_agent::types::AgentId;
use engram_dashboard_asset_library::format::read::ITEM_PATH_INVALID;
use engram_dashboard_asset_library::path::LibPath;
use engram_dashboard_asset_library::LIBRARY_UNAVAILABLE;
use engram_dashboard_command::{
    CommandDecl, CommandError, CommandFuture, CommandSpec, CommandTable, DeadlineClass, Effect,
    ErrorCode,
};
use futures_util::FutureExt as _;

use crate::command_delivery::LocalCommands;
use crate::connection_core::INPUT_LOCKED_REFUSAL;

use super::mcp_server::{CommandTableSlot, RosterBroadcastSlot};

/// Plugs the daemon's real things into the `agent.*`·`usage.*` table.
///
/// The blocking contract comes along: handler bodies write to disk under the profile lock and poll for resume early exit (agent `make_table`
/// doc), so call only through [`call_daemon_command`], on the blocking pool.
/// `broadcast` is a slot, not a value (load-bearing): the roster notification fanout is derived from the connection registry and assembled after
/// this table, so a value taken now would freeze "no notification" for the process lifetime — the tree showing the old roster, with no error and no
/// log (agent `RosterChanged` port doc). The slot defers the read to call time. This closes only the assembly order: a request arriving before
/// the slot fills still answers ok without notifying, while an assembly fills it late.
/// `lease` must be the one copy the WS connections hold (production: the `MultiViewState` given to the accept loop; `clone()` shares state).
/// A new `MultiViewState::new()` is wrong — an empty table makes bus and CLI input-affecting commands silently ignore the lease.
/// `usage` must be the service the connection layer subscribes and ⟳s on (production: `DaemonWiring.usage`), or the bus and the screen read
/// different books. Non-production assemblies plug in [`NoUsageLimits`].
/// `library` must be the service `manager` launches through (production: `DaemonWiring.library`), or the bus edits one
/// library and launches read another. Assemblies without one plug in [`NoLibrary`].
// ADR-0155
// ADR-0231
// ADR-0386
pub fn make_daemon_table(
    manager: Arc<AgentManager>,
    broadcast: Arc<RosterBroadcastSlot>,
    lease: Arc<dyn InputLease>,
    usage: Arc<dyn UsageCommandHost>,
    library: Arc<dyn LibraryCommandHost>,
) -> DaemonTable {
    DaemonTable {
        table: make_table(
            manager.clone(),
            Arc::new(LateRosterBroadcast(broadcast)),
            usage,
            library,
        ),
        roster: manager,
        lease,
    }
}

/// The daemon command table — the `agent.*` table plus the material for checking input-affecting commands ([`INPUT_AFFECTING`]) (roster · lease port) as one value.
///
/// One value, since separate plugging would allow a table with an empty lease port, skipping the lease without a signal; all three enter the slot together.
/// No door takes out the inner [`CommandTable`]: a second entrance skipping [`call_daemon_command`] would drop both checks (ADR-0157 arguments · ADR-0231 lease).
// ADR-0231
pub struct DaemonTable {
    table: CommandTable,
    /// The roster that resolves target tokens — the host the verb bodies use, so the checked agent = the executing agent.
    roster: Arc<dyn AgentCommandHost>,
    lease: Arc<dyn InputLease>,
}

impl DaemonTable {
    pub fn specs(&self) -> impl Iterator<Item = &'static CommandSpec> + '_ {
        self.table.specs()
    }

    pub fn decls(&self) -> Vec<CommandDecl> {
        self.table.decls()
    }
}

/// The input lease port — may a command that moves a live agent's input reach that agent.
///
/// `caller` = the calling connection; `None` = a connectionless caller (the two CLI routes), not a lease holder — passes if nobody holds it,
/// refused otherwise. Must be the same decision as WS `WriteStdin`·`Interrupt`·`CancelQueuedInput` (production: `MultiViewState`). The lock ends within the call.
// ADR-0231
pub trait InputLease: Send + Sync {
    fn permits(&self, agent_id: AgentId, caller: Option<ConnId>) -> bool;
}

/// The lease port of assemblies accepting no WS connections (smoke bins · control route harnesses) — always passes.
/// Do not plug it where the WS accept loop runs: bus and CLI input-affecting commands would ignore the viewer's lease.
// ADR-0231
pub struct NoInputLeases;

impl InputLease for NoInputLeases {
    fn permits(&self, _agent_id: AgentId, _caller: Option<ConnId>) -> bool {
        true
    }
}

/// The usage port of assemblies without a usage service (smoke bins · harnesses): `usage.*` answers `NOT_FOUND` by value.
/// Do not plug it into production (it plugs the connection layer's service — [`make_daemon_table`]).
pub struct NoUsageLimits;

impl UsageCommandHost for NoUsageLimits {
    fn get(&self, _backend: &AgentBackend) -> Option<UsageVendorRow> {
        None
    }

    fn refresh(&self, _backend: &AgentBackend) -> Option<UsageVendorRow> {
        None
    }
}

/// The library port of assemblies without a library service (the test server · smoke bins · harnesses): every
/// `library.*` command answers `CONFLICT` `library-unavailable`, except `library.refresh`, which answers the report
/// a refresh that could not scan gives (one `errors` row with no path) — a refresh refuses only a `folder` that is not
/// a library path (`INVALID_ARGUMENT` `item-path-invalid`, as the real service does).
/// Do not plug it into production (it plugs the service the manager launches through — [`make_daemon_table`]).
// ADR-0386
pub struct NoLibrary;

/// Why [`NoLibrary`] answers nothing.
const NO_LIBRARY: &str = "this daemon runs no library service";

impl NoLibrary {
    fn refused<T>() -> Result<T, PortRefusal> {
        Err(PortRefusal::Refused {
            class: PortClass::Unavailable,
            reason: Reason {
                code: LIBRARY_UNAVAILABLE.to_owned(),
                args: serde_json::json!({ "message": NO_LIBRARY }),
                line: None,
            },
        })
    }
}

impl LibraryCommandHost for NoLibrary {
    fn status(&self) -> Result<LibraryStatusOk, PortRefusal> {
        Self::refused()
    }
    fn set_root(&self, _path: &str, _create: bool) -> Result<LibraryStatusOk, PortRefusal> {
        Self::refused()
    }
    fn folders(&self) -> Result<LibraryFoldersOk, PortRefusal> {
        Self::refused()
    }
    fn refresh(&self, folder: Option<&str>) -> Result<LibraryRefreshOk, PortRefusal> {
        if let Some(text) = folder {
            if let Err(why) = LibPath::parse(text) {
                return Err(PortRefusal::Refused {
                    class: PortClass::InvalidArgument,
                    reason: Reason {
                        code: ITEM_PATH_INVALID.to_owned(),
                        args: serde_json::json!({ "path": escape_controls(text), "why": why.to_string() }),
                        line: None,
                    },
                });
            }
        }
        Ok(LibraryRefreshOk {
            changed: 0,
            problems: Vec::new(),
            no_kind: Vec::new(),
            skipped_links: Vec::new(),
            errors: vec![RefreshErrorRow {
                path: None,
                why: NO_LIBRARY.to_owned(),
            }],
        })
    }
    fn list(
        &self,
        _folder: Option<&str>,
        _query: Option<&str>,
        _kinds: &[LibraryKind],
        _builtins: bool,
    ) -> Result<LibraryListOk, PortRefusal> {
        Self::refused()
    }
    fn read(&self, _path: &str) -> Result<LibraryReadView, PortRefusal> {
        Self::refused()
    }
    fn new_folder(
        &self,
        _parent: Option<&str>,
        _name: &str,
        _root_generation: Option<u64>,
    ) -> Result<String, PortRefusal> {
        Self::refused()
    }
    fn new_file(
        &self,
        _folder: Option<&str>,
        _name: &str,
        _kind: LibraryNewFileKind,
        _root_generation: Option<u64>,
    ) -> Result<String, PortRefusal> {
        Self::refused()
    }
    fn relink(
        &self,
        _from: &str,
        _to: &str,
        _root_generation: Option<u64>,
    ) -> Result<LibraryRelinkOk, PortRefusal> {
        Self::refused()
    }
}

/// The only place that calls this table — the three surfaces come in here.
///
/// `None` = not a name of this table; the caller decides: the family route says "unknown verb", the full-name route asks the roster to tell "unknown
/// name" from "someone else's name" (`catalog::handle_call`), delivery moves to its next stage.
///
/// The entrance check is this one (ADR-0157): what a person or LLM just typed arrives here, so undeclared fields and missing required fields are
/// named and bounced (a silent `parnet` typo turns `move … --parent lead` into detaching to root). Do not add a path calling [`CommandTable::call`]
/// directly — that surface would run unchecked, the failure ADR-0157 blocks.
/// Someone else's names are not checked: no declaration to compare against, so `check_args` lets them through (additive evolution across hops — TRD §4-③).
///
/// Blocking (caller contract): handlers poll ~3 s for an early exit in resume mode, and rename/hierarchy moves save to disk under the profile
/// lock (agent `make_table` doc). Call on the blocking pool: the two routes via `spawn_blocking` in `control_agent_handler` · `control_call_handler`
/// (`mcp_server`), delivery via `command_delivery::spawn_local`.
/// No name contains panics (ADR-0293): the table call (argument coercion · future creation) and body driving are outside the containment mark, so
/// in production the hook logs and ends the daemon (test builds without the hook fold a body panic to `INTERNAL`). Containing a name again is a
/// new decision with two conditions: the body must have no effect (a contained panic folds to `INTERNAL`; with effects it must be
/// `OUTCOME_UNKNOWN` · `command_delivery::fold_local`), and do not split by declared effect (`usage.get` is `Read` yet changes the shared usage book).
/// `entrance` is a log label: a contract violation below (an async handler) shows as one error reply, so the log must say which surface.
/// The input lease check is this one too (ADR-0231): [`INPUT_AFFECTING`] commands go through `admit_input` after the argument check; placing it
/// elsewhere (`DaemonLocalCommands::run`) would skip `/control/agent`. `caller` = the calling connection (bus `Some(origin)`, CLI routes `None`).
// ADR-0155
// ADR-0157
// ADR-0231
// ADR-0293
pub fn call_daemon_command(
    table: &DaemonTable,
    name: &str,
    args: &mut serde_json::Value,
    entrance: &'static str,
    caller: Option<ConnId>,
) -> Option<Result<serde_json::Value, CommandError>> {
    // Names not in the table are split off before the check: `check_args` lets unknown names through, so reversing the order would pass someone else's name as "no argument problem" all the way down.
    if !table.table.contains(name) {
        return None;
    }
    if let Err(rejection) = table.table.check_args(name, args) {
        return Some(Err(rejection));
    }
    if let Err(refusal) = admit_input(table, name, args, caller) {
        return Some(Err(refusal));
    }
    // Passed `contains` above and the table is immutable during this call, so `None` is not reached.
    let future = table.table.call(name, args)?;
    Some(drive_to_completion(future, name, entrance))
}

/// The lease check of input-affecting commands — on pass, replaces `args.target` with the resolved id.
///
/// Resolve `target` once (same [`resolve_in`] · roster as the verb body) → check the lease with that id → put that id in the body. Do not drop the
/// last step: if the body re-resolved the name, a rename in between would make the checked and executing agents differ (the resolver matches ids exactly before names).
/// A resolution failure (missing `NOT_FOUND` · same name `CONFLICT`) is returned as is — flowing into the body would run an agent with the same name created meanwhile, unchecked.
/// Refusal = `CONFLICT` + the same text as WS ([`INPUT_LOCKED_REFUSAL`]); connectionless entrances (`caller == None`) append [`CONNECTIONLESS_LEASE_TAIL`], the head unchanged.
/// A `target` that is not a string or is whitespace-only passes on untouched: the body bounces it by value (type mismatch · empty value) pointing at the field to fix; resolving here would blur a blank into "no such agent".
/// Check-then-act residue: another connection can take the lease between check and body — the same window as WS (`check_input` then the manager call).
// ADR-0231
fn admit_input(
    table: &DaemonTable,
    name: &str,
    args: &mut serde_json::Value,
    caller: Option<ConnId>,
) -> Result<(), CommandError> {
    if !INPUT_AFFECTING.contains(&name) {
        return Ok(());
    }
    let Some(token) = args
        .get("target")
        .and_then(serde_json::Value::as_str)
        .filter(|token| !token.trim().is_empty())
    else {
        return Ok(());
    };
    let agent = resolve_in(&table.roster.roster(), token)?;
    if !table.lease.permits(agent.id, caller) {
        // A new lease-only error code must also be added to the "releasing failures" of `command_delivery::retains_the_id`, or a retry with the same number after the lease is released is swallowed as `ALREADY_APPLIED` (today `CONFLICT` releases).
        let refusal = match caller {
            Some(_) => INPUT_LOCKED_REFUSAL.to_string(),
            None => format!("{INPUT_LOCKED_REFUSAL}{CONNECTIONLESS_LEASE_TAIL}"),
        };
        return Err(CommandError::of(ErrorCode::Conflict, refusal));
    }
    args["target"] = serde_json::Value::String(agent.id.to_string());
    Ok(())
}

/// The lease refusal tail of connectionless entrances (the two CLI routes): WS's "acquire first" needs a connection, so this says who holds it and what to wait for.
// ADR-0231
const CONNECTIONLESS_LEASE_TAIL: &str =
    " — a connected viewer holds this agent's input; retry after it releases";

/// Finishes the future the table gave on this thread.
///
/// No executor: all handlers are `blocking_handler`, so the body runs to the end on the first poll (a tool-crate contract), and the caller is already on the blocking pool.
/// If the contract breaks (an async handler gets in, first poll `Pending`) it is exposed as `OUTCOME_UNKNOWN`, not a made-up answer. Not `INTERNAL`
/// (`retry: never` = certainly failed at this hop): the first poll may have applied part of the work, so the certainty is "unknown" and a retry must
/// carry the same request_id (TRD §4-④; the tool crate's delivery panic uses the same code).
/// Instrumented because the type does not enforce the contract and the symptom is one error reply.
fn drive_to_completion(
    future: CommandFuture,
    name: &str,
    entrance: &'static str,
) -> Result<serde_json::Value, CommandError> {
    future.now_or_never().unwrap_or_else(|| {
        tracing::error!(
            entrance,
            command = name,
            "the command did not finish on the first poll — this entrance drives only blocking handlers (an async handler got into the table)"
        );
        Err(CommandError::of(
            ErrorCode::OutcomeUnknown,
            format!(
                "'{name}' did not finish on its first poll — this entrance only drives blocking handlers, so part of it may already have been applied"
            ),
        ))
    })
}

/// The daemon adapter of delivery stage 1 — reads the table slot when called ([`LocalCommands`] doc). An empty slot is "not my command" (smoke bins · early wiring): stage 1 misses and the name rides the roster.
pub struct DaemonLocalCommands(Arc<CommandTableSlot>);

impl DaemonLocalCommands {
    pub fn new(table: Arc<CommandTableSlot>) -> Self {
        Self(table)
    }
}

impl LocalCommands for DaemonLocalCommands {
    /// `claim` and `run` look at the same immutable table (the slot is a `OnceLock`), so "claimed as mine, then empty-handed" cannot happen — the basis on
    /// which the port made that state a contract violation ([`LocalCommands::claim`]); another implementation must keep it.
    ///
    /// The `effect` passed on is correctness material, not an advertising field: delivery decides from it whether to hold the number against re-execution
    /// (`retains_the_id` in `command_delivery`), so a verb declared `Read` whose body changes state silently loses double-application protection. This
    /// cannot be caught here; whoever adds a verb must write `#[effect(...)]` as a contract.
    fn claim(&self, name: &str) -> Option<(Effect, DeadlineClass)> {
        self.0
            .get()?
            .specs()
            .find(|spec| spec.name == name)
            .map(|spec| (spec.effect, spec.deadline))
    }

    /// The entrance label and caller are received and passed on, not made up: two entrances call this adapter — socket dispatch (`bus`, the calling
    /// connection) and `/control/call` (`cli`, no connection — `catalog::handle_call`). A hardcoded label would merge their accidents under one tag, and a
    /// hardcoded caller would make the lease decision entrance-independent (test: `the_entrance_label_is_forwarded_not_hardcoded`).
    fn run(
        &self,
        name: &str,
        args: &mut serde_json::Value,
        entrance: &'static str,
        caller: Option<ConnId>,
    ) -> Option<Result<serde_json::Value, CommandError>> {
        call_daemon_command(self.0.get()?, name, args, entrance, caller)
    }

    fn decls(&self) -> Vec<CommandDecl> {
        self.0.get().map(|table| table.decls()).unwrap_or_default()
    }

    /// The roster of the host where verb bodies and the lease check (`admit_input`) resolve targets; empty until the table is plugged in — the fact that this daemon knows no agents then.
    // ADR-0308
    fn agent_roster(&self) -> Vec<AgentRosterRow> {
        self.0
            .get()
            .map(|table| table.roster.roster())
            .unwrap_or_default()
    }
}

/// The daemon adapter of the roster notification port — reads the slot when called (reason: [`make_daemon_table`] doc). An empty slot is a skipped notification; assemblies with no client exist ([`RosterBroadcastSlot`]).
struct LateRosterBroadcast(Arc<RosterBroadcastSlot>);

impl RosterChanged for LateRosterBroadcast {
    fn roster_changed(&self) {
        match self.0.get() {
            Some(broadcast) => broadcast.roster_changed(),
            None => tracing::debug!(
                "roster change notification skipped — fanout port not set (an assembly where no client can attach)"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use engram_dashboard_agent::profile::{
        AgentProfile, ProfileRegistry, ProfileStore, StoreError,
    };
    use engram_dashboard_agent::session_tracker::{SessionTracker, TrackerConfig};
    use engram_dashboard_agent::types::{
        AgentId, AgentInfo, AgentStatus, StatusSink, CLI_AGENT_VERBS,
    };
    use engram_dashboard_command::CommandError;
    use serde_json::json;

    use super::super::agent::RosterBroadcast;
    use super::*;
    use crate::usage_service::clock::OsUsageClock;
    use crate::usage_service::fakes::{self, ProbeGate};

    #[derive(Default)]
    struct MemProfileStore {
        saved: Mutex<Vec<AgentProfile>>,
    }
    impl ProfileStore for MemProfileStore {
        fn save(&self, profiles: &[AgentProfile]) -> Result<(), StoreError> {
            *self.saved.lock().expect("store poisoned") = profiles.to_vec();
            Ok(())
        }
        fn load(&self) -> Vec<AgentProfile> {
            self.saved.lock().expect("store poisoned").clone()
        }
    }

    struct NoopSink;
    impl StatusSink for NoopSink {
        fn status_changed(&self, _id: AgentId, _status: AgentStatus, _epoch: u32) {}
        fn agent_list_updated(&self, _agents: Vec<AgentInfo>) {}
    }

    #[derive(Default)]
    struct CountingBroadcast {
        calls: Mutex<usize>,
    }
    impl CountingBroadcast {
        fn calls(&self) -> usize {
            *self.calls.lock().expect("counter poisoned")
        }
    }
    impl RosterBroadcast for CountingBroadcast {
        fn roster_changed(&self) {
            *self.calls.lock().expect("counter poisoned") += 1;
        }
    }

    /// A manager with neither disk nor PTY — real processes in the table harness would break T-1.
    fn manager() -> Arc<AgentManager> {
        Arc::new(AgentManager::new(
            Arc::new(NoopSink),
            Arc::new(ProfileRegistry::new(Arc::new(MemProfileStore::default()))),
            Arc::new(SessionTracker::new(
                TrackerConfig {
                    enabled: false,
                    poll_interval: Duration::from_secs(1),
                },
                Arc::new(|_, _| {}),
            )),
        ))
    }

    fn call(
        table: &DaemonTable,
        name: &str,
        mut args: serde_json::Value,
    ) -> Result<serde_json::Value, CommandError> {
        call_daemon_command(table, name, &mut args, "test", None).expect("a name in the table")
    }

    /// A verb that adds one roster entry — the cheapest place to hang a notification (no launch, no backend).
    fn register_one(table: &DaemonTable) {
        call(
            table,
            "agent.new",
            json!({ "cwd": "C:/work/probe", "backend": "Claude" }),
        )
        .expect("registration succeeded");
    }

    /// The expected value is derived from the CLI verb list — hand-written, it would pass the old list when agent gains verbs; a verb without a declaration stays uncallable by the CLI.
    /// Names without a CLI verb are only those listed below: the two pending-list verbs, interrupt (`agent.interrupt` — ADR-0237) and
    /// multi-word resolution (`agent.resolve` — ADR-0307) are outside `CLI_AGENT_VERBS`, called via the bus and generic calls (`engram <name> …`). A
    /// table name not in the list grew without deciding on a CLI verb, so this stops. `usage.*` and `library.*` are not the `agent` family and never
    /// reach `/control/agent` (the library commands are reached by full name — ADR-0386).
    // ADR-0231
    #[test]
    fn the_daemon_table_holds_every_cli_agent_verb() {
        const BUS_ONLY: [&str; 15] = [
            "agent.cancelQueuedInput",
            "agent.interrupt",
            "agent.listQueuedInputs",
            "agent.resolve",
            "library.folders",
            "library.list",
            "library.newFile",
            "library.newFolder",
            "library.read",
            "library.refresh",
            "library.relink",
            "library.setRoot",
            "library.status",
            "usage.get",
            "usage.refresh",
        ];
        let table = make_daemon_table(
            manager(),
            Arc::new(RosterBroadcastSlot::new()),
            Arc::new(NoInputLeases),
            Arc::new(NoUsageLimits),
            Arc::new(NoLibrary),
        );

        let mut held: Vec<&str> = table.specs().map(|spec| spec.name).collect();
        held.sort_unstable();
        let mut wanted: Vec<String> = CLI_AGENT_VERBS
            .iter()
            .map(|verb| format!("agent.{verb}"))
            .chain(BUS_ONLY.iter().map(|name| name.to_string()))
            .collect();
        wanted.sort();
        assert_eq!(
            held, wanted,
            "the table's names must be CLI verbs + bus-only names (those written in the list)"
        );
    }

    /// Assemblies with no client to attach exist (smoke bins · harnesses) — a roster-changing verb panicking there would stop the daemon from starting.
    #[test]
    fn a_missing_broadcast_slot_does_not_panic_the_table() {
        let table = make_daemon_table(
            manager(),
            Arc::new(RosterBroadcastSlot::new()),
            Arc::new(NoInputLeases),
            Arc::new(NoUsageLimits),
            Arc::new(NoLibrary),
        );

        register_one(&table);

        let out = call(&table, "agent.list", json!({})).expect("query succeeded");
        assert_eq!(
            out["agents"].as_array().map(Vec::len),
            Some(1),
            "the registration itself finishes even without a notification: {out}"
        );
    }

    // ── the common entrance of the two surfaces ──────────────────────────────────────────────────────

    fn table_slot() -> Arc<CommandTableSlot> {
        let slot = Arc::new(CommandTableSlot::new());
        slot.set(Arc::new(make_daemon_table(
            manager(),
            Arc::new(RosterBroadcastSlot::new()),
            Arc::new(NoInputLeases),
            Arc::new(NoUsageLimits),
            Arc::new(NoLibrary),
        )));
        slot
    }

    /// The two surfaces get the same check — if only one checked, a typo blocked there would pass through the other as a different action (ADR-0157). Same arguments into both, same bounce expected.
    #[test]
    fn the_control_route_and_the_bus_share_one_argument_check() {
        let slot = table_slot();
        let bus = DaemonLocalCommands::new(slot.clone());
        let table = slot.get().expect("table").clone();
        let typo = json!({ "cwdd": "C:/work/x" });

        let direct = call_daemon_command(&table, "agent.new", &mut typo.clone(), "cli", None)
            .expect("a name of this table")
            .expect_err("an unknown field is bounced");
        let over_bus = bus
            .run("agent.new", &mut typo.clone(), "bus", Some(2))
            .expect("a name of this table")
            .expect_err("an unknown field is bounced");

        assert_eq!(direct.code(), ErrorCode::InvalidArgument);
        assert_eq!(over_bus.code(), direct.code());
        assert_eq!(
            over_bus.message(),
            direct.message(),
            "same check, same wording"
        );
        assert!(direct.message().contains("cwdd"), "{}", direct.message());
    }

    /// Someone else's names are returned untouched — touching arguments on a miss would hand the real owner half-made arguments (unknown fields cross hops as is — TRD §4-③).
    #[test]
    fn a_name_this_daemon_does_not_hold_is_a_miss_that_leaves_the_arguments_alone() {
        let bus = DaemonLocalCommands::new(table_slot());
        let mut args = json!({ "window": "main", "unknown": 1 });

        assert!(bus.claim("tab.create").is_none());
        assert!(bus.run("tab.create", &mut args, "bus", Some(2)).is_none());
        assert_eq!(args, json!({ "window": "main", "unknown": 1 }));
    }

    /// With the table not plugged in yet (smoke bins · early wiring) stage 1 is a miss as a whole — neither panic nor empty reply.
    #[test]
    fn an_unfilled_table_slot_is_a_stage_one_miss() {
        let bus = DaemonLocalCommands::new(Arc::new(CommandTableSlot::new()));

        assert!(bus.claim("agent.list").is_none());
        assert!(bus
            .run("agent.list", &mut json!({}), "bus", Some(2))
            .is_none());
        assert!(bus.decls().is_empty());
        assert!(
            bus.agent_roster().is_empty(),
            "with no table there are no known agents either"
        );
    }

    /// The roster resolving agent reference fields of someone else's commands = the roster where verbs resolve targets (ADR-0308 · ADR-0132); diverging, one name resolves to different agents in bus relay and `agent.*`.
    #[test]
    fn agent_references_resolve_against_the_verbs_own_roster() {
        let slot = table_slot();
        let bus = DaemonLocalCommands::new(slot.clone());
        register_one(slot.get().expect("table"));

        let rows = bus.agent_roster();
        assert_eq!(rows.len(), 1, "the agent just registered is visible");
        let listed = call(slot.get().expect("table"), "agent.list", json!({})).expect("list");
        assert_eq!(
            listed["agents"][0]["id"],
            rows[0].id.to_string(),
            "the same rows as the roster the verb sees"
        );
    }

    /// The discovery list carries what is really plugged into the table — advertising a declared-only name would pass delivery stage 1 and return "unknown command".
    #[test]
    fn the_advertised_names_are_the_ones_the_table_actually_holds() {
        let slot = table_slot();
        let bus = DaemonLocalCommands::new(slot.clone());

        let mut advertised: Vec<String> = bus.decls().into_iter().map(|d| d.name).collect();
        advertised.sort();
        let mut held: Vec<String> = slot
            .get()
            .expect("table")
            .specs()
            .map(|spec| spec.name.to_string())
            .collect();
        held.sort();
        assert_eq!(advertised, held);
        assert!(advertised.iter().all(|name| bus.claim(name).is_some()));
    }

    // ── entrance label forwarding ─────────────────────────────────────────────────────

    /// A handler whose first poll does not finish — breaks the blocking contract on purpose: only that branch lets `entrance` reach an observable place (the `error!` of
    /// [`drive_to_completion`]); a hardcoded label would give no signal otherwise.
    struct NeverFinishes;

    impl engram_dashboard_command::CommandHandler for NeverFinishes {
        fn call(&self, _args: serde_json::Value) -> engram_dashboard_command::CommandFuture {
            Box::pin(std::future::pending())
        }
    }

    /// A test-only declaration — `CommandTable::insert` accepts only names this crate declared.
    static PENDING_SPEC: engram_dashboard_command::CommandSpec =
        engram_dashboard_command::CommandSpec {
            name: "fixture.pending",
            effect: Effect::Write,
            since: 1,
            summary: "a test handler that does not finish on the first poll",
            args_schema: "{}",
            ok_schema: "{}",
            errors: &[],
            args_type: "FixturePendingArgs",
            ok_type: "FixturePendingOk",
            agent_args: &[],
            deadline: engram_dashboard_command::DeadlineClass::Ordinary,
        };

    static PENDING_DECLARED: &[&engram_dashboard_command::CommandSpec] = &[&PENDING_SPEC];

    fn pending_table_slot() -> Arc<CommandTableSlot> {
        let mut table = CommandTable::new(PENDING_DECLARED);
        table
            .insert(PENDING_SPEC.name, Arc::new(NeverFinishes))
            .expect("insert the test handler");
        let slot = Arc::new(CommandTableSlot::new());
        slot.set(Arc::new(DaemonTable {
            table,
            roster: manager(),
            lease: Arc::new(NoInputLeases),
        }));
        slot
    }

    /// The adapter passes on the entrance label it received — no hardcoded constant.
    ///
    /// Sibling tests pass the label as an argument and look only at the result, so a `run` hardcoding `"bus"` would pass them. Here several values go in and arrive
    /// as is in the log field (production has one value today — `LocalCommands::run` doc — so there is no other observation surface).
    /// The contract-violation branch is the only place the label comes out; it also pins the code `OUTCOME_UNKNOWN` (`INTERNAL` would diverge the re-query convention).
    #[test]
    fn the_entrance_label_is_forwarded_not_hardcoded() {
        for entrance in ["bus", "control", "a-third-door"] {
            let (code, lines) = crate::log_capture::capture_loud(|| {
                let adapter = DaemonLocalCommands::new(pending_table_slot());
                adapter
                    .run(PENDING_SPEC.name, &mut json!({}), entrance, None)
                    .expect("a name of this table")
                    .expect_err("if the first poll does not finish, it does not make up an outcome")
                    .code()
            });

            assert_eq!(code, ErrorCode::OutcomeUnknown);
            let wanted = format!("entrance=\"{entrance}\"");
            assert!(
                lines.iter().any(|line| line.contains(&wanted)),
                "the received label must be written as is ({entrance}): {lines:?}"
            );
        }
    }

    /// Pins that assembly order cannot kill notifications: the table is made with the slot empty and filled later; an adapter capturing the value at assembly
    /// would give 0. That regression is signalless at runtime (only the tree shows the old roster).
    #[test]
    fn the_notifier_reads_the_slot_when_called_not_when_the_table_is_built() {
        let slot = Arc::new(RosterBroadcastSlot::new());
        let table = make_daemon_table(
            manager(),
            slot.clone(),
            Arc::new(NoInputLeases),
            Arc::new(NoUsageLimits),
            Arc::new(NoLibrary),
        );

        let broadcast = Arc::new(CountingBroadcast::default());
        slot.set(broadcast.clone());
        register_one(&table);

        assert_eq!(
            broadcast.calls(),
            1,
            "a fanout filled after the table assembly must get the notification too"
        );
    }

    // ── input lease check (ADR-0231) ─────────────────────────────────────────────────

    const CANCEL: &str = "agent.cancelQueuedInput";

    /// A port answering as if one connection held one agent's lease — records the (agent, caller) asked.
    struct HeldBy {
        agent: AgentId,
        holder: ConnId,
        asked: Mutex<Vec<(AgentId, Option<ConnId>)>>,
    }

    impl HeldBy {
        fn new(agent: AgentId, holder: ConnId) -> Arc<Self> {
            Arc::new(Self {
                agent,
                holder,
                asked: Mutex::new(Vec::new()),
            })
        }

        fn asked(&self) -> Vec<(AgentId, Option<ConnId>)> {
            self.asked.lock().expect("lease poisoned").clone()
        }
    }

    impl InputLease for HeldBy {
        fn permits(&self, agent_id: AgentId, caller: Option<ConnId>) -> bool {
            self.asked
                .lock()
                .expect("lease poisoned")
                .push((agent_id, caller));
            agent_id != self.agent || caller == Some(self.holder)
        }
    }

    /// Stands in the cancel verb's body and records the arguments received — what the check passes to the body.
    struct RecordsArgs(Arc<Mutex<Vec<serde_json::Value>>>);

    impl engram_dashboard_command::CommandHandler for RecordsArgs {
        fn call(&self, args: serde_json::Value) -> CommandFuture {
            self.0.lock().expect("recorder poisoned").push(args);
            Box::pin(std::future::ready(Ok(json!({ "outcome": "requested" }))))
        }
    }

    /// A manager with `names` registered in the roster, and their ids — not launched (the check looks only at the roster).
    fn manager_with(names: &[&str]) -> (Arc<AgentManager>, Vec<AgentId>) {
        let manager = manager();
        let real = make_daemon_table(
            manager.clone(),
            Arc::new(RosterBroadcastSlot::new()),
            Arc::new(NoInputLeases),
            Arc::new(NoUsageLimits),
            Arc::new(NoLibrary),
        );
        let ids = names
            .iter()
            .map(|name| {
                let ok = call(
                    &real,
                    "agent.new",
                    json!({ "cwd": format!("C:/work/{name}"), "name": name, "backend": "Claude" }),
                )
                .expect("registration succeeded");
                ok["agent_id"]
                    .as_str()
                    .and_then(|id| id.parse().ok())
                    .expect("id")
            })
            .collect();
        (manager, ids)
    }

    /// A table with a recording handler in the cancel verb's place — the declaration is agent's real one (real schema for the argument check); the roster is the real manager (same resolver view as production).
    fn gated_slot(
        manager: Arc<AgentManager>,
        lease: Arc<dyn InputLease>,
    ) -> (Arc<CommandTableSlot>, Arc<Mutex<Vec<serde_json::Value>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut table = CommandTable::new(engram_dashboard_agent::commands::COMMAND_SPECS);
        table
            .insert(CANCEL, Arc::new(RecordsArgs(seen.clone())))
            .expect("declared name");
        let slot = Arc::new(CommandTableSlot::new());
        slot.set(Arc::new(DaemonTable {
            table,
            roster: manager,
            lease,
        }));
        (slot, seen)
    }

    fn cancel_args(target: &str) -> serde_json::Value {
        json!({ "target": target, "input_id": "q1" })
    }

    fn command_args(value: serde_json::Value) -> super::super::agent::CommandArgs {
        serde_json::from_value(value).expect("argument object")
    }

    /// A control route's envelope → (code, wording). On success, `("OK", payload)`.
    fn flatten(result: super::super::ingress::ControlQueryResult) -> (String, String) {
        match result {
            super::super::ingress::ControlQueryResult::Ok(payload) => {
                ("OK".to_string(), payload.to_string())
            }
            super::super::ingress::ControlQueryResult::Error { code, hint, .. } => {
                (code.to_string(), hint)
            }
        }
    }

    /// Issues the same cancel through three entrances — (code, wording) in order (bus · `/control/call` · `/control/agent`).
    fn cancel_through_three_entrances(
        slot: &Arc<CommandTableSlot>,
        target: &str,
        bus_caller: ConnId,
    ) -> Vec<(String, String)> {
        let bus = DaemonLocalCommands::new(slot.clone());
        let over_bus = match bus
            .run(CANCEL, &mut cancel_args(target), "bus", Some(bus_caller))
            .expect("a name of this table")
        {
            Ok(payload) => ("OK".to_string(), payload.to_string()),
            Err(e) => (e.code().as_str().to_string(), e.message().to_string()),
        };
        let over_call = match super::super::catalog::handle_call(
            &bus,
            &crate::command_roster::CommandRoster::new(),
            super::super::catalog::CallRequest {
                name: CANCEL.to_string(),
                args: command_args(cancel_args(target)),
                request_id: None,
            },
        ) {
            super::super::catalog::CallOutcome::Answered(result) => flatten(result),
            super::super::catalog::CallOutcome::Relay { .. } => {
                panic!("the daemon's own names are not relayed")
            }
        };
        let over_agent = flatten(super::super::agent::handle_agent(
            slot.get().expect("table"),
            super::super::agent::AgentRequest {
                verb: "cancelQueuedInput".to_string(),
                args: command_args(cancel_args(target)),
            },
        ));
        vec![over_bus, over_call, over_agent]
    }

    /// The three entrances refuse a non-holder the same way, since the check is one common entrance; one letting through would let someone else shake the viewer's input. The bus carries the calling
    /// connection, the two connectionless routes `None` — `/control/call` is trial 14's path (TRD §7-1).
    #[test]
    fn the_three_entrances_refuse_a_non_holder_alike() {
        let (manager, ids) = manager_with(&["alpha"]);
        let alpha = ids[0];
        let lease = HeldBy::new(alpha, 7);
        let (slot, seen) = gated_slot(manager, lease.clone());

        let answers = cancel_through_three_entrances(&slot, "alpha", 2);

        for (code, text) in &answers {
            assert_eq!(code, "CONFLICT", "{answers:?}");
            assert!(
                text.starts_with(INPUT_LOCKED_REFUSAL),
                "the same refusal wording as WS: {answers:?}"
            );
        }
        // The bus (with a connection) gets the WS text as is; the two connectionless ones get a tail saying what to wait for, without roster guidance.
        assert_eq!(answers[0].1, INPUT_LOCKED_REFUSAL, "{answers:?}");
        for (_, text) in &answers[1..] {
            assert!(text.contains(CONNECTIONLESS_LEASE_TAIL), "{answers:?}");
            assert!(!text.contains("to see the roster"), "{answers:?}");
        }
        assert!(
            seen.lock().expect("recorder poisoned").is_empty(),
            "a refused cancel does not reach the body"
        );
        assert_eq!(
            lease.asked(),
            vec![(alpha, Some(2)), (alpha, None), (alpha, None)],
            "the lease is checked once each with the resolved id · bus = the calling connection · the two CLI routes = None"
        );
    }

    /// The body receives the checked id, not the name — re-resolving would let a rename diverge checked and cancelled agents. With nobody holding the lease a `None` caller passes, and the holder passes with its own connection.
    #[test]
    fn the_verb_body_receives_the_resolved_id_from_every_entrance() {
        let (manager, ids) = manager_with(&["alpha", "beta"]);
        let (alpha, beta) = (ids[0], ids[1]);
        let (slot, seen) = gated_slot(manager, HeldBy::new(beta, 7));

        let answers = cancel_through_three_entrances(&slot, "alpha", 2);
        assert!(
            answers.iter().all(|(code, _)| code == "OK"),
            "nobody holds this agent's lease: {answers:?}"
        );
        let by_holder = DaemonLocalCommands::new(slot.clone())
            .run(CANCEL, &mut cancel_args("beta"), "bus", Some(7))
            .expect("a name of this table");
        assert!(by_holder.is_ok(), "the holder passes: {by_holder:?}");

        let seen = seen.lock().expect("recorder poisoned");
        let targets: Vec<&str> = seen
            .iter()
            .map(|args| args["target"].as_str().expect("string target"))
            .collect();
        let (alpha, beta) = (alpha.to_string(), beta.to_string());
        assert_eq!(
            targets,
            vec![
                alpha.as_str(),
                alpha.as_str(),
                alpha.as_str(),
                beta.as_str()
            ]
        );
        assert!(
            seen.iter().all(|args| args["input_id"] == "q1"),
            "other fields are left untouched: {seen:?}"
        );
    }

    /// A target that does not resolve ends before the lease check with the body's answer (NOT_FOUND) — flowing into the body would run a same-named agent created meanwhile, unchecked.
    #[test]
    fn an_unresolved_target_is_answered_before_the_lease_is_consulted() {
        let (manager, ids) = manager_with(&["alpha"]);
        let lease = HeldBy::new(ids[0], 7);
        let (slot, seen) = gated_slot(manager, lease.clone());

        let err = DaemonLocalCommands::new(slot)
            .run(CANCEL, &mut cancel_args("ghost"), "bus", Some(2))
            .expect("a name of this table")
            .expect_err("missing agent");

        assert_eq!(err.code(), ErrorCode::NotFound, "{}", err.message());
        assert!(lease.asked().is_empty(), "does not look at the lease");
        assert!(seen.lock().expect("recorder poisoned").is_empty());
    }

    /// A non-string or whitespace-only `target` is bounced by value by the verb: with the real table, the check passes it on untouched and the real body executes nothing (resolving in the check would blur a blank into "no such agent").
    #[test]
    fn a_blank_or_non_string_target_is_left_for_the_verb_to_refuse() {
        let (manager, ids) = manager_with(&["alpha"]);
        let lease = HeldBy::new(ids[0], 7);
        let table = make_daemon_table(
            manager,
            Arc::new(RosterBroadcastSlot::new()),
            lease.clone(),
            Arc::new(NoUsageLimits),
            Arc::new(NoLibrary),
        );

        for target in [json!("   "), json!(5)] {
            let err = call_daemon_command(
                &table,
                CANCEL,
                &mut json!({ "target": target, "input_id": "q1" }),
                "bus",
                Some(2),
            )
            .expect("a name of this table")
            .expect_err("bounce");
            assert_eq!(
                err.code(),
                ErrorCode::InvalidArgument,
                "{target}: {}",
                err.message()
            );
        }
        assert!(
            lease.asked().is_empty(),
            "the check does not resolve these two"
        );
    }

    /// Queries and other verbs do not look at the lease (list is a read; roster verbs do not move input): a non-holder's query gets the verb's own answer (a sleeping agent = NOT_FOUND), not a lease refusal.
    #[test]
    fn listing_and_roster_verbs_do_not_consult_the_lease() {
        let (manager, ids) = manager_with(&["alpha"]);
        let lease = HeldBy::new(ids[0], 7);
        let table = make_daemon_table(
            manager,
            Arc::new(RosterBroadcastSlot::new()),
            lease.clone(),
            Arc::new(NoUsageLimits),
            Arc::new(NoLibrary),
        );

        let listed = call_daemon_command(
            &table,
            "agent.listQueuedInputs",
            &mut json!({ "target": "alpha" }),
            "cli",
            None,
        )
        .expect("a name of this table")
        .expect_err("a sleeping agent holds no list");
        assert_eq!(listed.code(), ErrorCode::NotFound, "{}", listed.message());
        call_daemon_command(
            &table,
            "agent.rename",
            &mut json!({ "target": "alpha", "name": "gamma" }),
            "cli",
            None,
        )
        .expect("a name of this table")
        .expect("roster verbs are unrelated to the lease");

        assert!(lease.asked().is_empty(), "{:?}", lease.asked());
    }

    /// The door through which the shell resolves agent names reaches delivery stage 1 (TRD S28 §5-1): the shell sends `agent.resolve` in a bus envelope and the daemon answers from the production roster; unplugged, the shell would get "unknown command" and mistake it for an old daemon.
    // ADR-0307
    #[test]
    fn the_bus_answers_agent_resolve_from_the_roster() {
        let (manager, ids) = manager_with(&["alpha", "beta"]);
        let slot = Arc::new(CommandTableSlot::new());
        slot.set(Arc::new(make_daemon_table(
            manager,
            Arc::new(RosterBroadcastSlot::new()),
            Arc::new(NoInputLeases),
            Arc::new(NoUsageLimits),
            Arc::new(NoLibrary),
        )));
        let bus = DaemonLocalCommands::new(slot);

        assert_eq!(
            bus.claim("agent.resolve"),
            Some((Effect::Read, DeadlineClass::Ordinary))
        );
        // ADR-0369: the seat of the daemon's own launch verb is sized from its declaration.
        assert_eq!(
            bus.claim("agent.spawn"),
            Some((Effect::Write, DeadlineClass::Launch))
        );
        let out = bus
            .run(
                "agent.resolve",
                &mut json!({ "targets": ["beta", ids[0].to_string()] }),
                "bus",
                Some(2),
            )
            .expect("a name of this table")
            .expect("both resolve");
        assert_eq!(
            out,
            json!({ "agents": [
                { "target": "beta", "id": ids[1].to_string(), "name": "beta", "live": false },
                { "target": ids[0].to_string(), "id": ids[0].to_string(), "name": "alpha", "live": false },
            ]})
        );
    }

    /// The check relies on input-affecting commands having a required string `target`: if optional, a request missing it would reach the body and, once the body picks a default target, execute without the lease. A name not in the table is guarded by nothing.
    #[test]
    fn every_input_affecting_command_is_held_and_takes_a_required_string_target() {
        let table = make_daemon_table(
            manager(),
            Arc::new(RosterBroadcastSlot::new()),
            Arc::new(NoInputLeases),
            Arc::new(NoUsageLimits),
            Arc::new(NoLibrary),
        );
        for name in INPUT_AFFECTING {
            let spec = table
                .specs()
                .find(|spec| spec.name == *name)
                .unwrap_or_else(|| panic!("an input-affecting command not in the table: {name}"));
            let schema: serde_json::Value =
                serde_json::from_str(spec.args_schema).expect("declaration schema");
            assert!(
                schema["required"]
                    .as_array()
                    .is_some_and(|required| required.iter().any(|f| f == "target")),
                "{name}: target must be required — {schema}"
            );
            assert_eq!(
                schema["properties"]["target"]["type"], "string",
                "{name}: target must be a string — {schema}"
            );
        }
    }

    // ── usage (`usage.*` — TRD S21 usage-limit-slot §1-4 「bus」) ─────────────────────────────────────

    fn usage_table(usage: Arc<dyn UsageCommandHost>) -> DaemonTable {
        make_daemon_table(
            manager(),
            Arc::new(RosterBroadcastSlot::new()),
            Arc::new(NoInputLeases),
            usage,
            Arc::new(NoLibrary),
        )
    }

    /// Signals a blocked query came in — if never, it fails at the 10 s deadline instead of hanging.
    fn wait_entered(gate: &mut ProbeGate) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            match gate.entered.try_recv() {
                Ok(()) => return,
                Err(tokio::sync::mpsc::error::TryRecvError::Empty)
                    if std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("the query must start: {e:?}"),
            }
        }
    }

    /// A waiting bus request answers with a row, not `OUTCOME_UNKNOWN` (TRD §3 #17): the common entrance drives only handlers finishing on the first poll ([`drive_to_completion`]); while blocked there is no
    /// answer (it waits on the calling thread), once released the value comes as `Fresh`. Also checks the bus word (`Claude`) reaches the prober key's cell.
    #[test]
    fn a_waiting_bus_request_answers_a_row_through_the_common_entrance() {
        let real = engram_dashboard_agent::backend::usage_probe_for("claude").expect("prober");
        let (probe, mut gate) = fakes::gated_probe(real, 37.5, 1_900_000_000);
        // The wait cap is long: asserting "no answer yet" would fail falsely if the production cap (5 s) ended first on a slow runner.
        let service = fakes::service_with_wait(
            vec![probe],
            Arc::new(OsUsageClock::new()),
            Duration::from_secs(60),
        );
        let table = Arc::new(usage_table(service));
        let (answered, answer) = std::sync::mpsc::channel();
        let caller = Arc::clone(&table);
        // Imitates a blocking pool thread — the entrance is called from a thread outside the runtime.
        let worker = std::thread::spawn(move || {
            let mut args = json!({ "backend": "Claude" });
            let outcome = call_daemon_command(&caller, "usage.get", &mut args, "bus", Some(1))
                .expect("a name of this table");
            let _ = answered.send(outcome);
        });

        wait_entered(&mut gate);
        assert!(
            answer.recv_timeout(Duration::from_millis(200)).is_err(),
            "the answer waits for the query end"
        );
        gate.release.send(()).expect("blocked query");
        let row = answer
            .recv_timeout(Duration::from_secs(10))
            .expect("the answer comes after the query end")
            .expect("a row, not OUTCOME_UNKNOWN");
        worker.join().expect("calling thread");

        assert_eq!(row["backend"], "Claude");
        assert_eq!(
            row["served"], "Fresh",
            "it is the value the waited query brought: {row}"
        );
        assert_eq!(row["in_flight"], false);
        assert_eq!(row["state"], "Ready");
        assert_eq!(row["windows"][0]["window"], "FiveHour");
        assert_eq!(row["windows"][0]["used_pct"], 37.5);
        assert_eq!(row["windows"][0]["left_pct"], 62);
    }

    /// An assembly without a library service answers every `library.*` command `CONFLICT` `library-unavailable` by
    /// value, and `library.refresh` the report of a refresh that could not scan — one `errors` row with no path.
    // ADR-0386
    #[test]
    fn a_table_without_a_library_answers_library_unavailable_and_an_empty_refresh() {
        let table = make_daemon_table(
            manager(),
            Arc::new(RosterBroadcastSlot::new()),
            Arc::new(NoInputLeases),
            Arc::new(NoUsageLimits),
            Arc::new(NoLibrary),
        );
        let refusing = [
            ("library.status", json!({})),
            ("library.setRoot", json!({ "path": "C:/lib" })),
            ("library.folders", json!({})),
            ("library.list", json!({})),
            ("library.read", json!({ "path": "a.md" })),
            ("library.newFolder", json!({ "name": "x" })),
            (
                "library.newFile",
                json!({ "name": "x", "kind": "Markdown" }),
            ),
            ("library.relink", json!({ "from": "a.md", "to": "b.md" })),
        ];
        for (name, args) in refusing {
            let err = call(&table, name, args).expect_err("no library");
            assert_eq!(err.code(), ErrorCode::Conflict, "{name}");
            assert_eq!(
                err.args().and_then(|args| args.get("code")),
                Some(&json!(LIBRARY_UNAVAILABLE)),
                "{name}"
            );
        }
        let empty = json!({
            "changed": 0, "problems": [], "no_kind": [], "skipped_links": [],
            "errors": [{ "path": null, "why": NO_LIBRARY }],
        });
        for args in [json!({}), json!({ "folder": "rules/team" })] {
            let report = call(&table, "library.refresh", args.clone()).expect("a report");
            assert_eq!(report, empty, "{args}");
        }
    }

    /// Without a library, a refresh `folder` that is not a library path is still refused as the real service refuses
    /// it — `INVALID_ARGUMENT` `item-path-invalid`, not an empty report.
    // ADR-0386
    #[test]
    fn a_table_without_a_library_refuses_an_unparsable_refresh_folder() {
        let table = make_daemon_table(
            manager(),
            Arc::new(RosterBroadcastSlot::new()),
            Arc::new(NoInputLeases),
            Arc::new(NoUsageLimits),
            Arc::new(NoLibrary),
        );
        for folder in ["../x", "/abs", "a:b", "rules/"] {
            let err = call(&table, "library.refresh", json!({ "folder": folder }))
                .expect_err("not a library path");
            assert_eq!(err.code(), ErrorCode::InvalidArgument, "{folder}");
            assert_eq!(
                err.args().and_then(|args| args.get("code")),
                Some(&json!(ITEM_PATH_INVALID)),
                "{folder}"
            );
        }
    }

    /// An assembly without a usage service fails `usage.*` by value (no panic) — the same code as the same absence on wire ⟳.
    #[test]
    fn a_table_without_usage_limits_answers_not_found_by_value() {
        let table = usage_table(Arc::new(NoUsageLimits));
        for name in ["usage.get", "usage.refresh"] {
            let err =
                call(&table, name, json!({ "backend": "Claude" })).expect_err("there is no cell");
            assert_eq!(err.code(), ErrorCode::NotFound, "{name}");
            assert!(
                err.message()
                    .contains("this daemon keeps no usage limits for backend 'Claude'"),
                "{name}: {err}"
            );
        }
    }

    /// Every name's body runs outside the containment mark (ADR-0293), including the two queries ADR-0292 used to contain (`agent.list` · `agent.listQueuedInputs`) — containing them again turns this red.
    /// Without the hook the body net folds a body panic to `INTERNAL` (production: the hook ends the daemon); the body is the same `blocking_handler` as production.
    // ADR-0293
    #[test]
    fn no_command_body_runs_under_the_containment_mark() {
        let mut table = CommandTable::new(engram_dashboard_agent::commands::COMMAND_SPECS);
        for name in ["agent.listQueuedInputs", "usage.get", "agent.rename"] {
            table
                .insert(
                    name,
                    engram_dashboard_command::blocking_handler(|_: serde_json::Value| {
                        Ok::<_, CommandError>(engram_dashboard_base::panic::is_contained())
                    }),
                )
                .expect("declared name");
        }
        table
            .insert(
                "agent.list",
                engram_dashboard_command::blocking_handler(
                    |_: serde_json::Value| -> Result<bool, CommandError> {
                        panic!(
                            "body panic · marked {}",
                            engram_dashboard_base::panic::is_contained()
                        )
                    },
                ),
            )
            .expect("declared name");
        let table = DaemonTable {
            table,
            roster: manager(),
            lease: Arc::new(NoInputLeases),
        };

        for (name, mut args) in [
            ("agent.listQueuedInputs", json!({ "target": "alpha" })),
            ("usage.get", json!({ "backend": "Claude" })),
            ("agent.rename", json!({ "target": "alpha", "name": "beta" })),
        ] {
            let contained = call_daemon_command(&table, name, &mut args, "test", None)
                .expect("a name of this table")
                .expect("the body's answer");
            assert_eq!(contained, json!(false), "{name}");
        }

        let err = engram_dashboard_command::testing::with_quiet_panic_hook(|| {
            call_daemon_command(&table, "agent.list", &mut json!({}), "test", None)
                .expect("a name of this table")
                .expect_err("body panic")
        });
        assert_eq!(err.code(), ErrorCode::Internal);
        assert_eq!(
            err.message(),
            "command handler panicked: body panic · marked false"
        );
    }
}
