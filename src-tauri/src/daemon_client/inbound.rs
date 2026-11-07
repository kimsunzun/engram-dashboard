//! Daemon → shell inbound command receiver — applies a received envelope **outside the connection task**.
//!
//! ★The reason for this file's existence is that one line★ (ADR-0155 decision 4 · ADR-0081 「relay 적용은 액터 밖(비블로킹)」):
//! if the connection task `.await`s the envelope inline, a composite command (`agent.spawnInto` — its handler calls
//! `DaemonClient::send_command().await` from inside itself) **cannot fetch its own answer**. That answer is resolved only in the same connection
//! task's read loop, and that loop is now waiting for this handler (self-deadlock).
//! [`InboundReceiver::on_command`] therefore **pushes onto a queue and returns immediately**, and the actual execution runs in a separate task
//! launched by [`TaskSpawner`].
//!
//! The delivery rule uses the same 3 steps at every hop (`engram_dashboard_command::route` — ADR-0155 decision 3): is it in my
//! table → is it in the registry → error. The shell's step 2 is **the webview's share** ([`ViewCommandPort`]) — a name not in the table goes down to
//! that window, and if it is not there either, it goes out as `UNKNOWN_COMMAND`.
//!
//! ## Wiring
//! The caller is the connection driver (`connection.rs` `accept_inbound`, for a daemon `CommandRequest`); the wire
//! frames are `AgentEvent::CommandRequest` (daemon → shell) and `AgentCommand::CommandOutcome` (shell → daemon). The
//! shell's own names go onto the daemon's roster on every connect (`connection.rs` `register_commands`, packet =
//! [`registration_command`]).
//! The sending side exists now too — the daemon finds the owner in its own registry and writes the envelope onto that connection
//! (`engram_dashboard_daemon::command_delivery` · ADR-0154). A name not in the shell table goes one hop further down from here
//! and reaches the webview ([`ViewCommandPort`] · the real thing = `crate::view_commands`).
//!
//! ★But **the opposite direction does not reach yet**★: the reply of the daemon→shell round trip (`AgentEvent::CommandReply`) is not yet separated by the webview's
//! `handleEvent` (`src/api/protocolClient.ts`), so the promise of a command issued by the webview does not resolve
//! (the source of truth for that fact = the `AgentEvent::CommandReply` comment of `engram_dashboard_protocol`). It is a separate bridge from the direction this module runs
//! (daemon→shell).
//!
//! ★Where it is verified★: `tests/layout_commands.rs` drives the receiver, the outcome writer and the registration
//! packet without a connection (its residue list is canonical); the driver's suite (`connection/tests.rs`) drives the
//! driver's arm on a memory network.
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

/// One task — it has no output (the reply is carried by [`ReplySink`]).
pub type BoxedTask = Pin<Box<dyn Future<Output = ()> + Send>>;

/// The seam for **where the application runs**.
///
/// ★Why it is injected★: 「연결 태스크를 안 막는다」 (does not block the connection task) is not caught by counting but only by **when it runs**. The harness directly sees
/// whether `on_command` had already returned while the fake implementation held the task (ADR-0012).
/// An implementation **may drop the task** (runtime shutdown, etc.) — in that case the reply is emitted by the `Drop` of [`ReplySink`].
pub trait TaskSpawner: Send + Sync {
    fn spawn(&self, task: BoxedTask);
}

/// Production implementation — launches on **the very runtime** the connection task runs on.
///
/// The point is not to create a new runtime: for the application to do a daemon round trip (`agent.spawnInto`), it must
/// interleave with the socket task of the same runtime.
pub struct RuntimeSpawner(pub tokio::runtime::Handle);

impl TaskSpawner for RuntimeSpawner {
    fn spawn(&self, task: BoxedTask) {
        self.0.spawn(task);
    }
}

/// Where the shell hands **a name not in its own table** — today that place is one webview (hop ③ of TRD §3-8).
///
/// ★Why three endpoints are bundled in one trait★: all three read the same one state (which window reported what). If the registry and the
/// link were injected separately, the assembly part would have to guard every time that the two are the same real object.
/// ★Why it is injected★: the real object reaches `AppHandle::emit_to` and cannot be stood up without a window (ADR-0012). The real object is
/// `crate::view_commands::ViewCommandBridge`.
/// ★[`ViewCommandPort::declarations`] must not carry names held by the shell table★ — even if it does, delivery does not diverge
/// (`route` looks at the table first), but instead **one more name that cannot be called appears in the registration packet**. The place to filter is the implementation side
/// (the reserved-name filter of `view_commands`) — if filtered once more here, the same decision lives in two places, and the moment the two use different
/// criteria (declared / plugged in), it becomes impossible to tell which one is real.
pub trait ViewCommandPort: Send + Sync {
    /// The webview's share to put on the registration packet.
    fn declarations(&self) -> Vec<CommandDecl>;
    /// 「이 이름을 지금 웹뷰가 받을 수 있나」 (can the webview receive this name now) — step 2 of the 3-step delivery.
    fn lookup(&self, name: &str) -> OwnerLookup;
    /// Send the envelope down to the webview and wait for the reply.
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

    /// ★The `lookup` above is always `Unknown`, so this is not reached★ — even so, answering with a value rather than a panic is
    /// a contract (a command handler does not blow up and die — TRD §4-⑨).
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

/// Provides the two endpoints `route` requires (`OwnerLookupSource`·`CommandLink`) from one port.
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

/// Everything this hop uses for delivery — it must move into a task, so it is bundled in one `Arc`.
struct Hop {
    table: CommandTable,
    /// Steps 2·3 of the 3-step delivery — provides the registry (which name is the webview's) and its link together.
    view: ViewHop,
}

/// A receiver that delivers a received envelope to its own table.
pub struct InboundReceiver {
    hop: Arc<Hop>,
    spawn: Arc<dyn TaskSpawner>,
    catalog_version: u32,
}

impl InboundReceiver {
    /// `catalog_version` = the generation of the declaration block that built the table (`CATALOG_VERSION`). The registration packet carries it for diagnostics
    /// (it is wrong if the receiving side compares it with its own number and rejects — TRD §4-①).
    ///
    /// Stands without the webview's share — delivers only the shell's own table.
    pub fn new(table: CommandTable, spawn: Arc<dyn TaskSpawner>, catalog_version: u32) -> Self {
        Self::with_view(table, spawn, catalog_version, Arc::new(NoViewCommands))
    }

    /// An assembly that carries the webview's share too — used by the production path (`DaemonClient::install_command_table`).
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

    /// What goes on the registration packet — ★it advertises **what is actually plugged into the table**, not the declaration★ (if a name that cannot be called is registered,
    /// an envelope the daemon delivered bounces back as `UNKNOWN_COMMAND`).
    ///
    /// The webview's share is appended after — to the daemon's eyes the two layers are indistinguishable, and need not be (the 2-hop delivery splits them again
    /// in the shell — TRD §3-7 clause 2). ★So this list is **re-read every time**★: the webview comes up late regardless of the socket,
    /// so if it were built once and cached, a reconnect would send the old list again.
    pub fn declarations(&self) -> Vec<CommandDecl> {
        let mut decls = self.hop.table.decls();
        decls.extend(self.hop.view.0.declarations());
        decls
    }

    pub fn catalog_version(&self) -> u32 {
        self.catalog_version
    }

    /// The **single entry point** the connection loop calls — receives the envelope and returns the reply through `deliver`.
    ///
    /// ★The point is that it does not take the correlation key as an argument★: it takes `request_id` **directly** from the envelope and builds the reply
    /// slot, so the caller has no way to send another request's key (that mistake would attach an answer to someone else's round trip).
    /// `deliver` is called **exactly once** — whether an answer was given (`send`) or the task vanished without one (the `Drop` of
    /// [`ReplySink`] emits an error reply). So the caller does not handle 「답이 안 올 수도 있다」 (the answer may not come).
    /// ★`deliver` runs on the apply task, not on the connection driver's actor★ — it writes through a registry clone
    /// on the connection the request came on (`link::route::send_command_outcome`).
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
    /// ★All that is done here is one queue push★ — the caller (the connection read loop) waits for nothing inside this function. If it is reverted
    /// to inline execution, the self-deadlock of the header above revives as it was.
    fn on_command(&self, env: CommandEnvelope, reply: ReplySink) {
        let hop = Arc::clone(&self.hop);
        self.spawn.spawn(Box::pin(async move {
            let answered = route(&hop.table, &hop.view, &hop.view, env).await;
            reply.send(answered.outcome);
        }));
    }
}

/// The receiver place that is filled late.
///
/// ★Why a slot — because of a cycle★: the table's spawn port holds `DaemonClient` (`agent.spawnInto` is a daemon round trip)
/// and that client's connection task holds this receiver. So the assembly order is only **client → table → receiver → fill the
/// slot**, and the connection task may find the slot empty when it comes up (an envelope arriving then gets `None` from `get` below and the
/// call site emits an error reply — it is not quietly discarded). It has the same form as the daemon-side `CommandTableSlot`.
/// ★Only the first winner wins★ — a second `set` is ignored. If a path to swap the table were made, at that moment it would be impossible to know
/// which table is running (a reconnect does not rebuild the table — it only resends the registration).
pub struct InboundSlot(OnceLock<Arc<InboundReceiver>>);

impl InboundSlot {
    pub fn new() -> Self {
        InboundSlot(OnceLock::new())
    }

    pub fn set(&self, receiver: Arc<InboundReceiver>) {
        if self.0.set(receiver).is_err() {
            tracing::warn!("인바운드 수신기가 이미 꽂혀 있다 — 두 번째 설치는 무시한다");
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
//
// ★The `owner` of the registration packet is an **advertisement**★ — the daemon does not use this value but derives the owner token from **the connection that packet
//   came on** (protocol `AgentCommand::RegisterCommands` doc · daemon `note_claimed_owner`). So whatever is
//   written here, the registry's owner does not change — its only use is for a human to recognize who this connection is in the daemon log.
// ★Do not imitate the daemon's token prefix (`conn-`)★ — if that form is written, the daemon sees it as 「남의 주인 토큰을 적은 등록」 (a registration that wrote someone else's owner token) and
//   leaves a warning (the prefix branch of `note_claimed_owner`).
//
// ★★Do not promote this value to a 「클라이언트가 만든 식별자」 (client-made identifier) (ADR-0150 decision 1)★★ — that identifier must be **unique per
//   connection** but this literal is fixed, so two shells use the same value. The daemon registry overwrites registration last-wins per name and removes on disconnect
//   per owner (ADR-0150 decision 3), so that promotion would become behavior where **two shells steal each other's names and erase each other**.
//   When introducing an identifier, do not edit this constant but create a new value per connection.
const SHELL_OWNER_ADVERT: &str = "engram-dashboard-shell";

/// The advertisement string shared by the registration and delta packets — ★do not build it separately in two places★. If they diverge, two packets of the same
/// shell look like strangers in the daemon log (the registry's owner decision is independent of this value — see the comment above).
pub fn shell_owner_advert() -> engram_dashboard_protocol::OwnerToken {
    engram_dashboard_protocol::OwnerToken::new(SHELL_OWNER_ADVERT)
}

/// The registration packet this client sends — `None` when there is nothing to register or the declarations do not fit
/// the raw slot (error log · ADR-0272).
///
/// ★Why `pub` and apart from the send★: sending needs a connection (the driver queues it on every connect), but
/// **what is sent** (the advertised names · the catalog version · the owner string) does not. Apart, a harness asserts
/// the very packet that goes out (`tests/layout_commands.rs`) — a packet rebuilt by hand would stay green when the two
/// drift.
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
                "등록 항목 목록을 원문 칸으로 싸지 못했다 — 이 연결에서는 등록하지 않는다: {e}"
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
