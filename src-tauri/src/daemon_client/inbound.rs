//! Daemon → shell inbound command receiver — applies a received envelope outside the connection task.
//!
//! Why this file exists (ADR-0155 decision 4 · ADR-0081 「relay application runs outside the actor (non-blocking)」): if the connection task `.await`s the envelope inline, a composite command (`agent.spawnInto` — its handler calls `DaemonClient::send_command().await` from inside) cannot fetch its own answer, which is resolved only in the same connection task's read loop, now waiting for this handler (self-deadlock). [`InboundReceiver::on_command`] therefore pushes onto a queue and returns at once, and the execution runs in a separate task launched by [`TaskSpawner`].
//!
//! The delivery rule is the same 3 steps at every hop (`engram_dashboard_command::route` — ADR-0155 decision 3): is it in my table → is it in the registry → error. The shell's step 2 is the webview's part ([`ViewCommandPort`]) — a name not in the table goes down to that window, and if not there either, goes out as `UNKNOWN_COMMAND`.
//!
//! ## Wiring
//! The caller is the connection driver (`connection.rs` `accept_inbound`, for a daemon `CommandRequest`); the wire frames are `AgentEvent::CommandRequest` (daemon → shell) and `AgentCommand::CommandOutcome` (shell → daemon). The shell's own names go onto the daemon's roster on every connect (`connection.rs` `register_commands`, packet = [`registration_command`]). The sending side exists too — the daemon finds the owner in its registry and writes the envelope onto that connection (`engram_dashboard_daemon::command_delivery` · ADR-0154); a name not in the shell table goes one hop further down to the webview ([`ViewCommandPort`] · real = `crate::view_commands`).
//!
//! The opposite direction does not reach yet: the reply of the daemon→shell round trip (`AgentEvent::CommandReply`) is not separated by the webview's `handleEvent` (`src/api/protocolClient.ts`), so the promise of a webview-issued command does not resolve (source of truth: the `AgentEvent::CommandReply` comment of `engram_dashboard_protocol`) — a separate bridge from this module's daemon→shell direction.
//!
//! Verified in `tests/layout_commands.rs` (drives the receiver, the outcome writer and the registration packet without a connection; its residue list is canonical) and `connection/tests.rs` (the driver's arm on a memory network).
// ADR-0155
// ADR-0081

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use engram_dashboard_command::{
    route, CommandDecl, CommandEnvelope, CommandError, CommandLink, CommandReply, CommandTable,
    ErrorCode, InboundCommands, OwnerLookup, OwnerLookupSource, ReplySink,
};
use engram_dashboard_protocol::{AgentCommand, RequestId};

use super::protocol_state;

pub type BoxedTask = Pin<Box<dyn Future<Output = ()> + Send>>;

/// The seam for where the application runs. Injected because "does not block the connection task" is caught not by counting but only by when it runs — the harness sees whether `on_command` had already returned while the fake held the task (ADR-0012). An implementation may drop the task (runtime shutdown, etc.) — the reply is then emitted by the `Drop` of [`ReplySink`].
pub trait TaskSpawner: Send + Sync {
    fn spawn(&self, task: BoxedTask);
}

/// Production implementation — launches on the very runtime the connection task runs on: no new runtime, since the application doing a daemon round trip (`agent.spawnInto`) must interleave with the socket task of the same runtime.
pub struct RuntimeSpawner(pub tokio::runtime::Handle);

impl TaskSpawner for RuntimeSpawner {
    fn spawn(&self, task: BoxedTask) {
        self.0.spawn(task);
    }
}

/// Where the shell hands a name not in its own table — today one webview (hop ③ of TRD §3-8). Three endpoints are bundled in one trait because all read the same state (which window reported what); injected separately, the assembly would have to guard each time that the registry and link are the same real object. Injected for testability: the real object reaches `AppHandle::emit_to` and cannot be stood up without a window (ADR-0012); real = `crate::view_commands::ViewCommandBridge`.
/// [`ViewCommandPort::declarations`] must not carry names held by the shell table — delivery would not diverge (`route` looks at the table first), but one more uncallable name would appear in the registration packet. The filter lives on the implementation side (the reserved-name filter of `view_commands`); filtering again here would put the same decision in two places that could use different criteria (declared / plugged in).
pub trait ViewCommandPort: Send + Sync {
    fn declarations(&self) -> Vec<CommandDecl>;
    fn lookup(&self, name: &str) -> OwnerLookup;
    fn dispatch(&self, env: CommandEnvelope) -> Pin<Box<dyn Future<Output = CommandReply> + Send>>;
}

/// An assembly without a webview — step 2 is always a miss, so an unknown name goes out as `UNKNOWN_COMMAND`.
struct NoViewCommands;

impl ViewCommandPort for NoViewCommands {
    fn declarations(&self) -> Vec<CommandDecl> {
        Vec::new()
    }

    fn lookup(&self, _name: &str) -> OwnerLookup {
        OwnerLookup::Unknown
    }

    /// The `lookup` above is always `Unknown`, so this is not reached — answering with a value rather than a panic is still a contract (a command handler does not blow up and die — TRD §4-⑨).
    fn dispatch(&self, env: CommandEnvelope) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
        let request_id = env.request_id;
        let name = env.name;
        Box::pin(async move {
            CommandReply::err(
                request_id,
                CommandError::of(
                    ErrorCode::Unsupported,
                    format!("this client cannot forward '{name}' — it has no onward command link"),
                ),
            )
        })
    }
}

struct ViewHop(Arc<dyn ViewCommandPort>);

impl OwnerLookupSource for ViewHop {
    fn lookup(&self, name: &str) -> OwnerLookup {
        self.0.lookup(name)
    }
}

impl CommandLink for ViewHop {
    fn send(&self, env: CommandEnvelope) -> Pin<Box<dyn Future<Output = CommandReply> + Send>> {
        self.0.dispatch(env)
    }
}

struct Hop {
    table: CommandTable,
    view: ViewHop,
}

/// A receiver that delivers a received envelope to its own table.
pub struct InboundReceiver {
    hop: Arc<Hop>,
    spawn: Arc<dyn TaskSpawner>,
    catalog_version: u32,
}

impl InboundReceiver {
    /// `catalog_version` = the generation of the declaration block that built the table (`CATALOG_VERSION`). The registration packet carries it for diagnostics (the receiving side comparing it with its own number and rejecting would be wrong — TRD §4-①). Stands without the webview's part — delivers only the shell's own table.
    pub fn new(table: CommandTable, spawn: Arc<dyn TaskSpawner>, catalog_version: u32) -> Self {
        Self::with_view(table, spawn, catalog_version, Arc::new(NoViewCommands))
    }

    pub fn with_view(
        table: CommandTable,
        spawn: Arc<dyn TaskSpawner>,
        catalog_version: u32,
        view: Arc<dyn ViewCommandPort>,
    ) -> Self {
        InboundReceiver {
            hop: Arc::new(Hop {
                table,
                view: ViewHop(view),
            }),
            spawn,
            catalog_version,
        }
    }

    /// What goes on the registration packet — advertises what is actually plugged into the table, not the declaration (a registered uncallable name would bounce an envelope the daemon delivered back as `UNKNOWN_COMMAND`). The webview's part is appended after — to the daemon the two layers are indistinguishable and need not be (the 2-hop delivery splits them again in the shell — TRD §3-7 clause 2). Re-read every time: the webview comes up late regardless of the socket, so a once-built cached list would resend the old one on reconnect.
    pub fn declarations(&self) -> Vec<CommandDecl> {
        let mut decls = self.hop.table.decls();
        decls.extend(self.hop.view.0.declarations());
        decls
    }

    pub fn catalog_version(&self) -> u32 {
        self.catalog_version
    }

    /// The single entry point the connection loop calls — receives the envelope and returns the reply through `deliver`. It does not take the correlation key as an argument: it takes `request_id` directly from the envelope and builds the reply slot, so the caller has no way to send another request's key (which would attach an answer to someone else's round trip). `deliver` is called exactly once — whether an answer was given (`send`) or the task vanished without one (the `Drop` of [`ReplySink`] emits an error reply), so the caller need not handle "the answer may not come". `deliver` runs on the apply task, not the driver's actor — it writes through a registry clone on the connection the request came on (`link::route::send_command_outcome`).
    pub fn accept(
        &self,
        env: CommandEnvelope,
        deliver: impl FnOnce(CommandReply) + Send + 'static,
    ) {
        let reply = ReplySink::new(env.request_id, deliver);
        self.on_command(env, reply);
    }
}

impl InboundCommands for InboundReceiver {
    /// All that is done here is one queue push — the connection read loop waits for nothing inside it; reverting to inline execution revives the self-deadlock of the header.
    fn on_command(&self, env: CommandEnvelope, reply: ReplySink) {
        let hop = Arc::clone(&self.hop);
        self.spawn.spawn(Box::pin(async move {
            let answered = route(&hop.table, &hop.view, &hop.view, env).await;
            reply.send(answered.outcome);
        }));
    }
}

/// The receiver place that is filled late — a slot because of a cycle: the table's spawn port holds `DaemonClient` (`agent.spawnInto` is a daemon round trip) and that client's connection task holds this receiver, so assembly order is client → table → receiver → fill the slot, and the connection task may find it empty when it comes up (an envelope then gets `None` from `get` and the call site emits an error reply — not discarded quietly). Same form as the daemon-side `CommandTableSlot`. Only the first winner wins — a second `set` is ignored; a swap path would make it unknowable which table is running (a reconnect does not rebuild the table, it only resends the registration).
pub struct InboundSlot(OnceLock<Arc<InboundReceiver>>);

impl InboundSlot {
    pub fn new() -> Self {
        InboundSlot(OnceLock::new())
    }

    pub fn set(&self, receiver: Arc<InboundReceiver>) {
        if self.0.set(receiver).is_err() {
            tracing::warn!(
                "the inbound receiver is already plugged in — ignoring the second install"
            );
        }
    }

    pub fn get(&self) -> Option<&Arc<InboundReceiver>> {
        self.0.get()
    }
}

impl Default for InboundSlot {
    fn default() -> Self {
        Self::new()
    }
}

// ── Registering its own names ────────────────────────────────────────────────────────
// The `owner` of the registration packet is an advertisement: the daemon does not use it but derives the owner token from the connection the packet came on (protocol `AgentCommand::RegisterCommands` doc · daemon `note_claimed_owner`), so whatever is written here the registry's owner does not change — its only use is a human recognizing the connection in the daemon log. Do not imitate the daemon's token prefix (`conn-`): the daemon would see a registration that wrote someone else's owner token and warn (the prefix branch of `note_claimed_owner`).
// Do not promote this value to a client-made identifier (ADR-0150 decision 1): such an identifier must be unique per connection but this literal is fixed, so two shells would use the same value, and since the daemon registry overwrites registration last-wins per name and removes on disconnect per owner (ADR-0150 decision 3), two shells would steal and erase each other's names. When introducing an identifier create a new per-connection value; do not edit this constant.
const SHELL_OWNER_ADVERT: &str = "engram-dashboard-shell";

/// The advertisement string shared by the registration and delta packets — do not build it in two places: diverging, two packets of the same shell look like strangers in the daemon log (the registry's owner decision is independent of this value).
pub fn shell_owner_advert() -> engram_dashboard_protocol::OwnerToken {
    engram_dashboard_protocol::OwnerToken::new(SHELL_OWNER_ADVERT)
}

/// The registration packet this client sends — `None` when there is nothing to register or the declarations do not fit the raw slot (error log · ADR-0272). `pub` and apart from the send because sending needs a connection (queued by the driver on every connect) while what is sent (advertised names · catalog version · owner string) does not: a harness asserts the very packet that goes out (`tests/layout_commands.rs`); a hand-rebuilt packet would stay green when the two drift.
pub fn registration_command(receiver: &InboundReceiver) -> Option<AgentCommand> {
    let decls = receiver.declarations();
    if decls.is_empty() {
        return None;
    }
    let decls = match protocol_state::decls_to_wire(&decls) {
        Ok(decls) => decls,
        Err(e) => {
            tracing::error!(
                names = decls.len(),
                "could not wrap the registration entry list into a raw-text field — not registering on this connection: {e}"
            );
            return None;
        }
    };
    Some(AgentCommand::RegisterCommands {
        owner: shell_owner_advert(),
        decls,
        catalog_version: receiver.catalog_version(),
        request_id: RequestId::new(),
    })
}
