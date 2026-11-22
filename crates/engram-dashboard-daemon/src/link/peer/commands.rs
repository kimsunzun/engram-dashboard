//! The `peer.*` commands — the daemon's own command table, declared beside its producer and joined into the daemon table behind
//!   `control::commands::call_daemon_command` (the only entrance: argument check, then [`PeerCommands::call`] — this module's approval gate,
//!   then the handler). The table is private: no other path reaches a handler.
//!
//! Who may call what (ADR-0394 decision 1), decided from the typed caller — never an argument, never the entrance label — by one rule per
//!   name ([`agent_policy`]; a name without a rule is refused for agents):
//! - open to agents: `peer.list`, `peer.remove` (it narrows trust), `peer.edit` of an address;
//! - gated: `peer.add`, `peer.setNetwork`, `peer.regenerateKey`, `peer.edit` with a new name — an agent's call records a pending request
//!   and is answered at once ("requested"); a human's call applies at once;
//! - human-only: `peer.approve` / `peer.deny` — the answers to requests, including the system questions (accept the trusted list, adopt —
//!   whose third answer is `peer.approve --start_fresh true`).
//!
//! The first `peer.*` call of a PC without an identity makes it (ADR-0389 d3: once, on first need), whoever calls — this PC's own key
//!   widens no trust — so `peer.list` shows an ID to pair with.
//!
//! ★Every handler finishes on its first poll★ (the daemon table drives blocking handlers only): the trust operations run on the caller's
//!   blocking-pool thread and never wait for a link — endpoint and link results show in `peer.list`. An agent's gated call never waits
//!   for the human either.
//!
//! The answers are for an LLM caller (`docs/reference/llm-command-conventions.md`): one query for the whole area (`peer.list`, filter by
//!   one PC), a PC named by `--name` or `--id` (separate arguments), refusals that say what to do next, and `actions` in `peer.list`
//!   naming every state that needs a human.

// ADR-0394, ADR-0390, ADR-0389, ADR-0307

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use engram_dashboard_command::{
    blocking_handler, declare_commands, CommandDecl, CommandError, CommandFuture, CommandSpec,
    CommandTable, ErrorCode,
};

use super::approvals::{
    apply, quote, Answered, Applied, Approvals, Change, Decision, Kind, NetworkPatch, NoRequest,
    PendingRow, Refusal, Requested, TrustOps, DENY_COOLDOWN, MAX_PENDING,
};
pub(crate) use super::approvals::{PeerCaller, Requester};
use super::book::{Online, Warning};
use super::id::PeerId;
use super::interfaces::{cached_interface_ips, my_addresses};
use super::runtime::{EndpointError, EndpointView, LinkSettings, PeerSnapshot};
use super::store::{PeerEdit, PeerError, PeerStatus};
use super::trusted::{check_name, Helper, Permission, TrustedPeer};
use super::wire::DuplicateKind;

declare_commands! {
    // v1: the first daemon-declared table (ADR-0394).
    catalog_version: 1;

    /// This PC's network settings.
    struct PeerNetworkRow {
        /// The fixed listen port; absent = the build's default (`listen_port` says which).
        port: Option<u16>,
        listen_port: u16,
        /// default | none | a URL.
        relay: String,
        /// default | none | a URL.
        address_lookup: String,
        mdns: bool,
    }

    /// One trusted PC.
    struct PeerPcRow {
        id: String,
        name: String,
        address: Option<String>,
        /// online | offline | refused | version_mismatch | duplicate_identity | endpoint_error.
        online: String,
        warning: Option<String>,
    }

    /// A PC refused at the accept gate this run (not on the trusted list).
    struct PeerKnockRow {
        id: String,
        count: u64,
        last_at_ms: i64,
    }

    /// One request waiting for a human's answer.
    struct PeerRequestRow {
        request: String,
        /// add | setNetwork | regenerateKey | rename | acceptList | adopt.
        kind: String,
        summary: String,
        /// The requesting agent — absent for a question the peer state raised.
        agent_id: Option<String>,
        agent_name: Option<String>,
        /// The PC whose mail most recently reached that agent (a hint).
        origin: Option<String>,
        asked_at_ms: i64,
        expires_in_s: Option<u64>,
    }

    /// The peer area in one query — this PC's ID and direct addresses, the network settings, the endpoint, every trusted PC with its
    /// link state and warnings, refused PCs, requests waiting for a human, and the actions a human must take. `--name` / `--id` narrow it
    /// to one PC.
    #[effect(Read)]
    #[since(1)]
    "peer.list" => args PeerListArgs {
        name: Option<String>,
        id: Option<String>,
    } -> ok PeerListOk {
        /// ready | not_started | no_identity | key_unreadable | key_transient | adopt_or_fresh | identity_in_use | list_refused |
        /// list_changed_outside | list_corrupt | unavailable.
        state: String,
        reason: Option<String>,
        id: Option<String>,
        addresses: Vec<String>,
        network: Option<PeerNetworkRow>,
        /// off | up | error.
        endpoint: String,
        endpoint_error: Option<String>,
        pcs: Vec<PeerPcRow>,
        knocks: Vec<PeerKnockRow>,
        requests: Vec<PeerRequestRow>,
        actions: Vec<String>,
    } errors [NOT_FOUND];

    /// Pairs a PC by its ID. An agent's call only requests it — a human approves.
    #[effect(Write)]
    #[since(1)]
    "peer.add" => args PeerAddArgs {
        id: String,
        name: String,
        address: Option<String>,
    } -> ok PeerAddOk {
        /// applied | requested.
        outcome: String,
        request: Option<String>,
        message: String,
    } errors [CONFLICT];

    /// Changes a PC's direct address (`--address ip:port` or `none`) and / or its name (`--new_name` — an agent's rename only requests it).
    #[effect(Write)]
    #[since(1)]
    "peer.edit" => args PeerEditArgs {
        name: Option<String>,
        id: Option<String>,
        new_name: Option<String>,
        address: Option<String>,
    } -> ok PeerEditOk {
        /// applied | requested.
        outcome: String,
        request: Option<String>,
        message: String,
    } errors [NOT_FOUND, CONFLICT];

    /// Removes a PC from the trusted list at once (its links close); a refused save is reported and the removal holds for this run.
    #[effect(Write)]
    #[since(1)]
    "peer.remove" => args PeerRemoveArgs {
        name: Option<String>,
        id: Option<String>,
    } -> ok PeerRemoveOk {
        id: String,
        name: String,
        saved: bool,
        message: String,
    } errors [NOT_FOUND, CONFLICT];

    /// Changes how this PC reaches others; absent fields keep their value. An agent's call only requests it.
    #[effect(Write)]
    #[since(1)]
    "peer.setNetwork" => args PeerSetNetworkArgs {
        /// A UDP port or `default`.
        port: Option<String>,
        /// default | none | an http(s) URL.
        relay: Option<String>,
        /// default | none | an http(s) URL.
        address_lookup: Option<String>,
        mdns: Option<bool>,
    } -> ok PeerSetNetworkOk {
        /// applied | requested.
        outcome: String,
        request: Option<String>,
        message: String,
    } errors [CONFLICT];

    /// Makes a new identity key for this PC (every other PC must pair again) — or, while the adopt question is open, a fresh identity
    /// instead of adopting. An agent's call only requests it.
    #[effect(Write)]
    #[since(1)]
    "peer.regenerateKey" => args PeerRegenerateKeyArgs {}
        -> ok PeerRegenerateKeyOk {
        /// applied | requested.
        outcome: String,
        request: Option<String>,
        message: String,
        /// This PC's ID after the change.
        id: Option<String>,
    } errors [CONFLICT];

    /// Approves a waiting request — a human only. `--start_fresh true` answers the adopt request with a fresh identity instead.
    #[effect(Write)]
    #[since(1)]
    "peer.approve" => args PeerApproveArgs {
        request: String,
        start_fresh: Option<bool>,
    } -> ok PeerApproveOk {
        /// applied.
        outcome: String,
        message: String,
        /// This PC's ID when the approved change made a new one.
        id: Option<String>,
    } errors [NOT_FOUND, CONFLICT];

    /// Denies a waiting request — a human only. `--stop_asking true` also refuses that agent's requests until the daemon restarts.
    #[effect(Write)]
    #[since(1)]
    "peer.deny" => args PeerDenyArgs {
        request: String,
        stop_asking: Option<bool>,
    } -> ok PeerDenyOk {
        /// denied.
        outcome: String,
        message: String,
    } errors [NOT_FOUND, CONFLICT];
}

const LIST: &str = "peer.list";
const ADD: &str = "peer.add";
const EDIT: &str = "peer.edit";
const REMOVE: &str = "peer.remove";
const SET_NETWORK: &str = "peer.setNetwork";
const REGENERATE_KEY: &str = "peer.regenerateKey";
const APPROVE: &str = "peer.approve";
const DENY: &str = "peer.deny";

/// The answer to an agent calling a human-only command.
const HUMAN_ONLY: &str = "only a human answers a request — the user approves or denies it in the Engram app or the dialog that shows it; agents cannot (peer.list shows what waits)";
const NOT_RUNNING: &str = "the peer link is not running on this daemon (it is starting, or it failed to start — see the daemon log); nothing was changed";

/// What an agent may do with a `peer.*` name (ADR-0394 decision 1). ★Every declared name has one★ — a name without one is refused for
///   agents (fail closed), and a test checks every declaration is classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentPolicy {
    /// Runs for agents.
    Open,
    /// Runs for agents unless it renames (`--new_name`), which is gated.
    OpenUnlessRename,
    /// An agent's call records a pending request.
    Gated,
    /// Agents are refused.
    HumanOnly,
}

fn agent_policy(name: &str) -> Option<AgentPolicy> {
    match name {
        LIST | REMOVE => Some(AgentPolicy::Open),
        EDIT => Some(AgentPolicy::OpenUnlessRename),
        ADD | SET_NETWORK | REGENERATE_KEY => Some(AgentPolicy::Gated),
        APPROVE | DENY => Some(AgentPolicy::HumanOnly),
        _ => None,
    }
}

/// What a call came to.
pub(crate) enum PeerCall {
    /// Answered without a handler — an agent's gated call became a pending request.
    Answered(serde_json::Value),
    /// The handler's future — blocking, finished on its first poll by the caller.
    Run(CommandFuture),
}

/// The `peer.*` table and its approval gate. ★The table is not handed out★: the only way to run a handler is [`PeerCommands::call`],
///   which applies the gate first.
pub(crate) struct PeerCommands {
    table: CommandTable,
    approvals: Arc<Approvals>,
}

impl PeerCommands {
    pub(crate) fn new(approvals: Arc<Approvals>) -> Self {
        let mut table = CommandTable::new(COMMAND_SPECS);
        let a = approvals.clone();
        // Blow up at assembly: an insert refusal is a build-decided value (the agent table's rule).
        table
            .insert(
                LIST,
                blocking_handler(move |args: PeerListArgs| list(&a, args)),
            )
            .expect("could not plug peer.list into the table");
        let a = approvals.clone();
        table
            .insert(
                ADD,
                blocking_handler(move |args: PeerAddArgs| {
                    let ops = running(&a)?;
                    let change = Change::Add(add_of(args, Some(&ops.snapshot()))?);
                    let message = applied_message(&apply(ops, &change).map_err(refused)?);
                    Ok(PeerAddOk {
                        outcome: "applied".to_string(),
                        request: None,
                        message,
                    })
                }),
            )
            .expect("could not plug peer.add into the table");
        let a = approvals.clone();
        table
            .insert(
                EDIT,
                blocking_handler(move |args: PeerEditArgs| {
                    let ops = running(&a)?;
                    let (id, edit) = edit_of(args, &ops.snapshot())?;
                    ops.edit(&id, edit).map_err(refused)?;
                    Ok(PeerEditOk {
                        outcome: "applied".to_string(),
                        request: None,
                        message: format!("changed PC {id} — peer.list shows it"),
                    })
                }),
            )
            .expect("could not plug peer.edit into the table");
        let a = approvals.clone();
        table
            .insert(
                REMOVE,
                blocking_handler(move |args: PeerRemoveArgs| {
                    let ops = running(&a)?;
                    let id = pick(&ops.snapshot(), args.name.as_deref(), args.id.as_deref())?;
                    let removal = ops.remove(&id).map_err(refused)?;
                    let (saved, message) = match removal.saved {
                        Ok(()) => (
                            true,
                            format!(
                                "removed PC {} — its links are closed and it is refused from now on",
                                quote(&removal.removed.name)
                            ),
                        ),
                        // ADR-0390 d4: the revocation holds for this run even though the file kept the PC.
                        Err(e) => (
                            false,
                            format!(
                                "removed PC {} for this run — its links are closed and it is refused — but the trusted list could not be saved ({e}), so it is listed again after a restart unless a later save succeeds",
                                quote(&removal.removed.name)
                            ),
                        ),
                    };
                    Ok(PeerRemoveOk {
                        id: removal.removed.id.to_text(),
                        name: removal.removed.name,
                        saved,
                        message,
                    })
                }),
            )
            .expect("could not plug peer.remove into the table");
        let a = approvals.clone();
        table
            .insert(
                SET_NETWORK,
                blocking_handler(move |args: PeerSetNetworkArgs| {
                    let ops = running(&a)?;
                    let patch = patch_of(args, &ops.snapshot())?;
                    let message =
                        applied_message(&apply(ops, &Change::SetNetwork(patch)).map_err(refused)?);
                    Ok(PeerSetNetworkOk {
                        outcome: "applied".to_string(),
                        request: None,
                        message,
                    })
                }),
            )
            .expect("could not plug peer.setNetwork into the table");
        let a = approvals.clone();
        table
            .insert(
                REGENERATE_KEY,
                blocking_handler(move |_: PeerRegenerateKeyArgs| {
                    let ops = running(&a)?;
                    let applied = apply(ops, &Change::RegenerateKey).map_err(refused)?;
                    Ok(PeerRegenerateKeyOk {
                        outcome: "applied".to_string(),
                        request: None,
                        message: applied_message(&applied),
                        id: new_id(&applied),
                    })
                }),
            )
            .expect("could not plug peer.regenerateKey into the table");
        let a = approvals.clone();
        table
            .insert(
                APPROVE,
                blocking_handler(move |args: PeerApproveArgs| {
                    let decision = if args.start_fresh == Some(true) {
                        Decision::StartFresh
                    } else {
                        Decision::Approve
                    };
                    match a.answer(args.request.trim(), decision) {
                        Ok(Answered::Applied(applied)) => Ok(PeerApproveOk {
                            outcome: "applied".to_string(),
                            message: applied_message(&applied),
                            id: new_id(&applied),
                        }),
                        Ok(Answered::Failed(e)) => {
                            let mut error = refused(e);
                            let message = format!(
                                "the request was approved and is used up, but applying it failed: {}",
                                error.message()
                            );
                            error.set_message(message);
                            Err(error)
                        }
                        Ok(Answered::Denied) => Err(CommandError::internal(
                            "an approval was recorded as a deny",
                        )),
                        Err(missing) => Err(no_request(&args.request, missing)),
                    }
                }),
            )
            .expect("could not plug peer.approve into the table");
        let a = approvals.clone();
        table
            .insert(
                DENY,
                blocking_handler(move |args: PeerDenyArgs| {
                    let decision = if args.stop_asking == Some(true) {
                        Decision::DenyAndStop
                    } else {
                        Decision::Deny
                    };
                    match a.answer(args.request.trim(), decision) {
                        Ok(_) => Ok(PeerDenyOk {
                            outcome: "denied".to_string(),
                            message: format!(
                                "denied request {} — nothing was changed",
                                quote(args.request.trim())
                            ),
                        }),
                        Err(missing) => Err(no_request(&args.request, missing)),
                    }
                }),
            )
            .expect("could not plug peer.deny into the table");
        Self { table, approvals }
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.table.contains(name)
    }

    pub(crate) fn specs(&self) -> impl Iterator<Item = &'static CommandSpec> + '_ {
        self.table.specs()
    }

    pub(crate) fn decls(&self) -> Vec<CommandDecl> {
        self.table.decls()
    }

    /// The declaration's argument check (ADR-0157) — the entrance runs it before [`PeerCommands::call`].
    pub(crate) fn check_args(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<(), CommandError> {
        self.table.check_args(name, args)
    }

    /// Runs `name` for `caller` (ADR-0394 decisions 1 · 2 · 4): this PC's identity is made first if it has none yet (any caller — its
    ///   own key widens no trust); then a human runs every command, an agent runs the open ones, is refused the human-only ones, and its
    ///   gated calls become a pending request answered at once. ★Blocking★ — on the blocking pool, like every handler here.
    pub(crate) fn call(
        &self,
        name: &str,
        args: &mut serde_json::Value,
        caller: &PeerCaller,
    ) -> Result<PeerCall, CommandError> {
        self.ensure_identity();
        if let PeerCaller::Agent(agent) = caller {
            if let Some(answer) = self.gate(name, args, agent)? {
                return Ok(PeerCall::Answered(answer));
            }
        }
        // Names reach here only after `contains`, and the table does not change.
        self.table
            .call(name, args)
            .map(PeerCall::Run)
            .ok_or_else(|| CommandError::internal(format!("'{name}' is not in the peer table")))
    }

    /// ADR-0389 d3: the key is made once, on first need — the first `peer.*` call.
    fn ensure_identity(&self) {
        let Some(ops) = self.approvals.ops() else {
            return;
        };
        if ops.snapshot().status != PeerStatus::NoIdentityYet {
            return;
        }
        match ops.ensure_identity() {
            Ok(id) => tracing::info!(pc = %id, "this PC's peer identity was created"),
            Err(e) => tracing::warn!("this PC's peer identity could not be created: {e}"),
        }
    }

    /// An agent's call: `None` = run the handler, `Some` = answered here (a pending request), `Err` = refused.
    fn gate(
        &self,
        name: &str,
        args: &serde_json::Value,
        agent: &Requester,
    ) -> Result<Option<serde_json::Value>, CommandError> {
        let (change, snapshot) = match agent_policy(name) {
            Some(AgentPolicy::Open) => return Ok(None),
            Some(AgentPolicy::OpenUnlessRename) => {
                let args: PeerEditArgs = parse(args)?;
                if args.new_name.is_none() {
                    return Ok(None);
                }
                let snapshot = running(&self.approvals)?.snapshot();
                let (id, edit) = edit_of(args, &snapshot)?;
                (Change::Edit { id, edit }, snapshot)
            }
            // One read of the state: the change is checked against it and the request is written from it.
            Some(AgentPolicy::Gated) => {
                let snapshot = running(&self.approvals)?.snapshot();
                let change = match name {
                    ADD => Change::Add(add_of(parse(args)?, Some(&snapshot))?),
                    SET_NETWORK => Change::SetNetwork(patch_of(parse(args)?, &snapshot)?),
                    REGENERATE_KEY => Change::RegenerateKey,
                    _ => return Err(unclassified(name, agent)),
                };
                (change, snapshot)
            }
            Some(AgentPolicy::HumanOnly) => {
                tracing::warn!(agent = %agent.id, command = name, "an agent tried to answer an approval request — refused");
                return Err(CommandError::of(ErrorCode::Conflict, HUMAN_ONLY));
            }
            None => return Err(unclassified(name, agent)),
        };
        let requested = self
            .approvals
            .request_seen(change, agent.clone(), Some(snapshot))
            .map_err(request_refused)?;
        requested_answer(name, &requested).map(Some)
    }
}

/// A name no agent rule covers — refused (fail closed).
fn unclassified(name: &str, agent: &Requester) -> CommandError {
    tracing::error!(agent = %agent.id, command = name, "a peer command has no agent rule — refused for agents");
    CommandError::of(
        ErrorCode::Conflict,
        format!("'{name}' is not open to agents — ask the user"),
    )
}

/// The trust operations, or the refusal while the peer link has not started.
fn running(approvals: &Approvals) -> Result<&dyn TrustOps, CommandError> {
    approvals
        .ops()
        .ok_or_else(|| CommandError::of(ErrorCode::Conflict, NOT_RUNNING))
}

fn parse<T: serde::de::DeserializeOwned>(args: &serde_json::Value) -> Result<T, CommandError> {
    serde_json::from_value(args.clone()).map_err(|e| CommandError::invalid_argument(e.to_string()))
}

/// A refusal of the peer state → the command error vocabulary.
fn refused(e: PeerError) -> CommandError {
    match e {
        PeerError::Invalid(why) => CommandError::invalid_argument(why),
        PeerError::NotFound(id) => CommandError::not_found(format!(
            "no trusted PC has the ID {id} — peer.list lists them"
        )),
        PeerError::Blocked(why) | PeerError::Conflict(why) | PeerError::NotOffered(why) => {
            CommandError::of(ErrorCode::Conflict, why)
        }
        PeerError::Io(why) => CommandError::internal(why),
    }
}

fn request_refused(refusal: Refusal) -> CommandError {
    let message = match refusal {
        Refusal::Stopped => "the user asked not to be asked again for this agent's trust changes until the daemon restarts — nothing was recorded; ask the user directly".to_string(),
        Refusal::CoolingDown(ago) => format!(
            "the user denied this kind of request from this agent {} min ago — nothing was recorded; do not ask again before {} min have passed, or ask the user directly",
            ago.as_secs() / 60,
            DENY_COOLDOWN.as_secs() / 60
        ),
        Refusal::Stopping => "the daemon is shutting down — nothing was recorded".to_string(),
        Refusal::NetworkUnknown => "the current network settings are not known — nothing was recorded; peer.list shows the state".to_string(),
        Refusal::Full => format!(
            "{MAX_PENDING} trust-change requests already wait for the user's approval — nothing was recorded; ask the user to answer them first (peer.list lists them)"
        ),
    };
    CommandError::of(ErrorCode::Conflict, message)
}

fn no_request(request: &str, missing: NoRequest) -> CommandError {
    match missing {
        NoRequest::Unknown => CommandError::not_found(format!(
            "no request {} is waiting — it was answered already, expired, or never made (peer.list lists the waiting ones)",
            quote(request.trim())
        )),
        NoRequest::NotRunning => CommandError::of(ErrorCode::Conflict, NOT_RUNNING),
        NoRequest::NotAdopt => CommandError::invalid_argument(
            "start_fresh answers only the adopt request — approve or deny this one; the request still waits",
        ),
    }
}

/// The answer to an agent's gated call (ADR-0394 decision 4), in the command's declared answer shape.
fn requested_answer(name: &str, requested: &Requested) -> Result<serde_json::Value, CommandError> {
    let message = if requested.existing {
        format!(
            "already requested — waiting for the user's approval (request {}); do not call again, peer.list shows it until the user answers",
            requested.id
        )
    } else {
        format!(
            "requested — waiting for the user's approval (request {}); nothing changes until the user approves it, and peer.list shows it until then",
            requested.id
        )
    };
    let (outcome, request) = ("requested".to_string(), Some(requested.id.clone()));
    let value = match name {
        ADD => serde_json::to_value(PeerAddOk {
            outcome,
            request,
            message,
        }),
        EDIT => serde_json::to_value(PeerEditOk {
            outcome,
            request,
            message,
        }),
        SET_NETWORK => serde_json::to_value(PeerSetNetworkOk {
            outcome,
            request,
            message,
        }),
        REGENERATE_KEY => serde_json::to_value(PeerRegenerateKeyOk {
            outcome,
            request,
            message,
            id: None,
        }),
        other => {
            return Err(CommandError::internal(format!(
                "'{other}' has no requested answer"
            )))
        }
    };
    value.map_err(|e| CommandError::internal(e.to_string()))
}

fn applied_message(applied: &Applied) -> String {
    match applied {
        Applied::Done => "applied — peer.list shows the state; link results come later".to_string(),
        Applied::Regenerated(regenerated) => {
            let old = regenerated
                .old
                .map(|old| format!(" (the old ID was {})", old.grouped()))
                .unwrap_or_default();
            format!(
                "this PC's ID is now {}{old}. Every other PC must remove this PC's old ID and pair with the new one. Peer links stay off until a human accepts the trusted list again (peer.list shows the request).",
                regenerated.id.grouped()
            )
        }
        Applied::Fresh(id) => format!(
            "this PC has a new identity of its own: {} — pair it with the other PCs (the other machine's identity was left as it is)",
            id.grouped()
        ),
        Applied::NetworkMoved(fields) => format!(
            "applied — note: {} changed after the request was made, and the request's values replaced them; peer.list shows the settings now",
            fields.join(", ")
        ),
    }
}

fn new_id(applied: &Applied) -> Option<String> {
    match applied {
        Applied::Done | Applied::NetworkMoved(_) => None,
        Applied::Regenerated(regenerated) => Some(regenerated.id.to_text()),
        Applied::Fresh(id) => Some(id.to_text()),
    }
}

fn parse_id(text: &str) -> Result<PeerId, CommandError> {
    PeerId::parse(text.trim()).map_err(|e| CommandError::invalid_argument(format!("id: {e}")))
}

fn parse_address(text: &str) -> Result<SocketAddr, CommandError> {
    text.trim().parse::<SocketAddr>().map_err(|_| {
        CommandError::invalid_argument(format!(
            "address: {} is not ip:port (e.g. 192.168.0.7:{})",
            quote(text),
            super::trusted::RELEASE_DEFAULT_PORT
        ))
    })
}

/// The trusted PC an argument pair names — exactly one of `name` (any case) and `id`.
fn pick(
    snapshot: &PeerSnapshot,
    name: Option<&str>,
    id: Option<&str>,
) -> Result<PeerId, CommandError> {
    let found = match (name, id) {
        (Some(_), Some(_)) | (None, None) => {
            return Err(CommandError::invalid_argument(
                "name the PC with exactly one of --name or --id",
            ))
        }
        (Some(name), None) => snapshot
            .peers
            .iter()
            .find(|row| row.peer.name.to_lowercase() == name.trim().to_lowercase()),
        (None, Some(id)) => {
            let id = parse_id(id)?;
            snapshot.peers.iter().find(|row| row.peer.id == id)
        }
    };
    found
        .map(|row| row.peer.id)
        .ok_or_else(|| not_listed(snapshot))
}

fn not_listed(snapshot: &PeerSnapshot) -> CommandError {
    let names: Vec<String> = snapshot
        .peers
        .iter()
        .map(|row| quote(&row.peer.name))
        .collect();
    let candidates = if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    };
    CommandError::not_found(format!(
        "no trusted PC is named that — candidates: {candidates}"
    ))
}

/// A new trusted PC from `peer.add`'s arguments, checked as far as the current state allows (the store checks again when it applies).
fn add_of(args: PeerAddArgs, snapshot: Option<&PeerSnapshot>) -> Result<TrustedPeer, CommandError> {
    let id = parse_id(&args.id)?;
    let name = args.name.trim().to_string();
    check_name(&name).map_err(|why| CommandError::invalid_argument(format!("name: {why}")))?;
    let address = args.address.as_deref().map(parse_address).transpose()?;
    if let Some(snapshot) = snapshot {
        if snapshot.id == Some(id) {
            return Err(CommandError::invalid_argument(
                "that is this PC's own ID — add the ID shown by peer.list on the other PC",
            ));
        }
        if let Some(row) = snapshot.peers.iter().find(|row| row.peer.id == id) {
            return Err(CommandError::of(
                ErrorCode::Conflict,
                format!("that PC is already trusted as {}", quote(&row.peer.name)),
            ));
        }
        if let Some(row) = snapshot
            .peers
            .iter()
            .find(|row| row.peer.name.to_lowercase() == name.to_lowercase())
        {
            return Err(CommandError::of(
                ErrorCode::Conflict,
                format!(
                    "the name {} is already used by PC {} — choose another name",
                    quote(&row.peer.name),
                    row.peer.id
                ),
            ));
        }
    }
    Ok(TrustedPeer {
        id,
        name,
        address,
        permission: Permission::Mail,
    })
}

/// The PC and the change `peer.edit` names.
fn edit_of(
    args: PeerEditArgs,
    snapshot: &PeerSnapshot,
) -> Result<(PeerId, PeerEdit), CommandError> {
    let id = pick(snapshot, args.name.as_deref(), args.id.as_deref())?;
    let name = match args.new_name {
        Some(name) => {
            let name = name.trim().to_string();
            check_name(&name)
                .map_err(|why| CommandError::invalid_argument(format!("new_name: {why}")))?;
            Some(name)
        }
        None => None,
    };
    let address = match args.address.as_deref().map(str::trim) {
        None => None,
        Some("none") => Some(None),
        Some(text) => Some(Some(parse_address(text)?)),
    };
    if name.is_none() && address.is_none() {
        return Err(CommandError::invalid_argument(
            "nothing to change — give --new_name and / or --address (ip:port, or none to clear it)",
        ));
    }
    Ok((id, PeerEdit { name, address }))
}

/// The fields `peer.setNetwork` names, checked: each value's form, and the settings they make with the current ones against the link
///   adapter (merged again with the settings of the moment it is applied).
fn patch_of(
    args: PeerSetNetworkArgs,
    snapshot: &PeerSnapshot,
) -> Result<NetworkPatch, CommandError> {
    let mut patch = NetworkPatch::default();
    if let Some(port) = args.port.as_deref().map(str::trim) {
        patch.port = Some(match port {
            "default" => None,
            text => match text.parse::<u16>() {
                Ok(port) if port != 0 => Some(port),
                _ => {
                    return Err(CommandError::invalid_argument(
                        "port: a UDP port from 1 to 65535, or default",
                    ))
                }
            },
        });
    }
    if let Some(relay) = args.relay.as_deref() {
        patch.relay = Some(
            Helper::parse(relay.trim())
                .map_err(|why| CommandError::invalid_argument(format!("relay: {why}")))?,
        );
    }
    if let Some(lookup) = args.address_lookup.as_deref() {
        patch.address_lookup = Some(
            Helper::parse(lookup.trim())
                .map_err(|why| CommandError::invalid_argument(format!("address_lookup: {why}")))?,
        );
    }
    patch.mdns = args.mdns;
    if patch.is_empty() {
        return Err(CommandError::invalid_argument(
            "nothing to change — give --port, --relay, --address_lookup and / or --mdns",
        ));
    }
    // Without the settings of this moment (the list file could not be read) the question could show no true "from" — refused.
    let Some(current) = &snapshot.network else {
        return Err(CommandError::of(
            ErrorCode::Conflict,
            format!(
                "the current network settings are not known ({}) — nothing was recorded or changed; peer.list shows the state",
                snapshot.status
            ),
        ));
    };
    let merged = patch.applied_to(current);
    merged.check_for_endpoint().map_err(|why| {
        CommandError::invalid_argument(format!("the link refuses that server: {why}"))
    })?;
    Ok(patch)
}

// ── peer.list ──

fn list(approvals: &Approvals, args: PeerListArgs) -> Result<PeerListOk, CommandError> {
    let pending = approvals.pending();
    let requests: Vec<PeerRequestRow> = pending.iter().map(request_row).collect();
    let Some(ops) = approvals.ops() else {
        return Ok(PeerListOk {
            state: "not_started".to_string(),
            reason: Some(NOT_RUNNING.to_string()),
            id: None,
            addresses: Vec::new(),
            network: None,
            endpoint: "off".to_string(),
            endpoint_error: None,
            pcs: Vec::new(),
            knocks: Vec::new(),
            requests,
            actions: Vec::new(),
        });
    };
    let snapshot = ops.snapshot();
    let only: Option<PeerId> = match (&args.name, &args.id) {
        (None, None) => None,
        (Some(_), Some(_)) => {
            return Err(CommandError::invalid_argument(
                "narrow to one PC with exactly one of --name or --id",
            ))
        }
        (Some(name), None) => Some(pick(&snapshot, Some(name), None)?),
        // An ID may name a trusted PC or one that only knocked.
        (None, Some(id)) => {
            let id = parse_id(id)?;
            if !snapshot.peers.iter().any(|row| row.peer.id == id)
                && !snapshot.knocks.iter().any(|knock| knock.id == id)
            {
                return Err(CommandError::not_found(format!(
                    "no trusted PC and no refused PC has the ID {id}"
                )));
            }
            Some(id)
        }
    };
    let actions = actions(&snapshot, &pending);
    let pcs = snapshot
        .peers
        .iter()
        .filter(|row| only.map_or(true, |id| row.peer.id == id))
        .map(|row| PeerPcRow {
            id: row.peer.id.to_text(),
            name: row.peer.name.clone(),
            address: row.peer.address.map(|a| a.to_string()),
            online: online_word(row.online).to_string(),
            warning: row.warning.map(warning_text),
        })
        .collect();
    let knocks = snapshot
        .knocks
        .iter()
        .filter(|knock| only.map_or(true, |id| knock.id == id))
        .map(|knock| PeerKnockRow {
            id: knock.id.to_text(),
            count: knock.count,
            last_at_ms: epoch_ms(knock.last_at),
        })
        .collect();
    let (endpoint, endpoint_error) = match &snapshot.endpoint {
        EndpointView::Off => ("off", None),
        EndpointView::Up { .. } => ("up", None),
        EndpointView::Error(e) => ("error", Some(endpoint_error_text(e))),
    };
    let addresses = match &snapshot.endpoint {
        EndpointView::Up { addrs } => my_addresses(addrs, &cached_interface_ips()),
        _ => Vec::new(),
    };
    Ok(PeerListOk {
        state: state_word(&snapshot.status).to_string(),
        reason: (snapshot.status != PeerStatus::Ready).then(|| snapshot.status.to_string()),
        id: snapshot.id.map(|id| id.to_text()),
        addresses,
        network: snapshot.network.as_ref().map(|network| PeerNetworkRow {
            port: network.port,
            listen_port: network.listen_port(LinkSettings::daemon().dev),
            relay: helper_word(&network.relay),
            address_lookup: helper_word(&network.address_lookup),
            mdns: network.mdns,
        }),
        endpoint: endpoint.to_string(),
        endpoint_error,
        pcs,
        knocks,
        requests,
        actions,
    })
}

fn request_row(row: &PendingRow) -> PeerRequestRow {
    PeerRequestRow {
        request: row.id.clone(),
        kind: row.kind.word().to_string(),
        summary: row.summary.clone(),
        agent_id: row.agent.as_ref().map(|agent| agent.id.clone()),
        agent_name: row.agent.as_ref().and_then(|agent| agent.name.clone()),
        origin: row.origin.clone(),
        asked_at_ms: epoch_ms(row.asked_at),
        expires_in_s: row.expires_in.map(|left| left.as_secs()),
    }
}

/// Every state that needs a human, with what to do. A question the user put off is said to be put off, not pointed at.
fn actions(snapshot: &PeerSnapshot, pending: &[PendingRow]) -> Vec<String> {
    let waiting = |kind: Kind| {
        pending
            .iter()
            .find(|row| row.kind == kind)
            .map(|row| row.id.clone())
    };
    let mut out = Vec::new();
    match &snapshot.status {
        PeerStatus::KeyUnreadable(_) => out.push("this PC's identity key cannot be used — a human makes a new one with peer.regenerateKey (every other PC must pair again)".to_string()),
        PeerStatus::AdoptOrFresh { .. } => out.push(match waiting(Kind::Adopt) {
            Some(id) => format!("this account's identity was found under another machine id — a human answers request {id}: adopt it (peer.approve), or start fresh (peer.approve --start_fresh true)"),
            None => "this account's identity was found under another machine id — the user put the question off for this run; it comes back at the next start, or a human starts fresh with peer.regenerateKey".to_string(),
        }),
        PeerStatus::ListChangedOutside | PeerStatus::ListCorrupt(_) => out.push(match waiting(Kind::AcceptList) {
            Some(id) => format!("peer links stay off until a human accepts the trusted list — request {id} (the app, or the dialog)"),
            None => "peer links stay off until a human accepts the trusted list — the user put the question off; it comes back within 10 minutes (or at the next start)".to_string(),
        }),
        PeerStatus::IdentityInUse => out.push("another Engram copy on this PC uses this identity — stop that copy; this one retries by itself".to_string()),
        PeerStatus::ListRefused(_) => out.push("the trusted list file cannot be written (reason) — fix or move it; peer changes answer CONFLICT until then".to_string()),
        _ => {}
    }
    match &snapshot.endpoint {
        EndpointView::Error(EndpointError::HelperRefused(_)) => out.push("a stored relay or address-lookup URL is refused — a human changes it with peer.setNetwork".to_string()),
        EndpointView::Error(EndpointError::Bind(_)) => out.push("the listen port cannot be bound (in use?) — retried every minute; peer.setNetwork --port changes it".to_string()),
        _ => {}
    }
    for row in &snapshot.peers {
        if row.warning.is_some() {
            out.push(format!(
                "PC {} sees this PC's identity running twice — look for a second running copy of Engram with this identity (a copied data folder, a cloned VM); if there is none, peer.regenerateKey makes a new one (every other PC must pair again)",
                quote(&row.peer.name)
            ));
        }
    }
    if !snapshot.knocks.is_empty() {
        out.push("PCs not on the trusted list tried to link (knocks) — a human pairs one only after its owner confirms its ID another way; an unknown ID is not a reason to add it".to_string());
    }
    let agent_waiting = pending.iter().filter(|row| row.agent.is_some()).count();
    if agent_waiting > 0 {
        out.push(format!(
            "{agent_waiting} request(s) from agents wait for a human's answer (the app or the dialog) — agents cannot answer them"
        ));
    }
    out
}

fn state_word(status: &PeerStatus) -> &'static str {
    match status {
        PeerStatus::Ready => "ready",
        PeerStatus::NoIdentityYet => "no_identity",
        PeerStatus::KeyUnreadable(_) => "key_unreadable",
        PeerStatus::KeyTransient(_) => "key_transient",
        PeerStatus::AdoptOrFresh { .. } => "adopt_or_fresh",
        PeerStatus::IdentityInUse => "identity_in_use",
        PeerStatus::ListRefused(_) => "list_refused",
        PeerStatus::ListChangedOutside => "list_changed_outside",
        PeerStatus::ListCorrupt(_) => "list_corrupt",
        PeerStatus::Unavailable(_) => "unavailable",
    }
}

fn online_word(online: Online) -> &'static str {
    match online {
        Online::Online => "online",
        Online::Offline => "offline",
        Online::Refused => "refused",
        Online::VersionMismatch => "version_mismatch",
        Online::DuplicateIdentity => "duplicate_identity",
        Online::EndpointError => "endpoint_error",
    }
}

fn warning_text(warning: Warning) -> String {
    let what = match warning.kind {
        DuplicateKind::Duplicate => "duplicate identity — two running copies of this PC's identity",
        DuplicateKind::Suspected => {
            "suspected duplicate identity — this PC's link to it keeps being replaced"
        }
    };
    if warning.reported {
        format!("{what} (reported by that PC)")
    } else {
        what.to_string()
    }
}

fn endpoint_error_text(error: &EndpointError) -> String {
    match error {
        EndpointError::HelperRefused(why) => {
            format!("a stored helper server URL is refused: {why}")
        }
        EndpointError::Bind(why) => format!("could not bind the listen port: {why}"),
    }
}

fn helper_word(helper: &Helper) -> String {
    match helper {
        Helper::Default => "default".to_string(),
        Helper::None => "none".to_string(),
        Helper::Url(url) => url.clone(),
    }
}

fn epoch_ms(at: std::time::SystemTime) -> i64 {
    at.duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
