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
//! The call site is the `Message::Text` branch of `connection.rs` (that file's `accept_inbound`), and the wire
//! frames that carry the envelope are also in place — `AgentEvent::CommandRequest` (daemon→shell) and `AgentCommand::CommandOutcome` (shell→daemon).
//! The shell putting its own names onto the daemon registry also goes out on every (re)connect (`register_own_commands`).
//! The sending side exists now too — the daemon finds the owner in its own registry and writes the envelope to that connection
//! (`engram_dashboard_daemon::command_delivery` · ADR-0154). A name not in the shell table goes one hop further down here
//! and reaches the webview ([`ViewCommandPort`] · real implementation = `crate::view_commands`).
//!
//! ★But **the opposite direction does not reach yet**★: the reply of the daemon→shell round trip (`AgentEvent::CommandReply`) is not yet separated by the webview's
//! `handleEvent` (`src/api/protocolClient.ts`), so the promise of a command issued by the webview does not resolve
//! (the source of truth for that fact = the `AgentEvent::CommandReply` comment of `engram_dashboard_protocol`). It is a separate bridge from the direction this module runs
//! (daemon→shell).
//!
//! ★Where verification stands★: the connection select loop **requires a real socket** — only that branch selection is left uncovered here,
//! and everything below it (slot lookup · outcome assembly · channel delivery · registration packet contents) is covered with real code by a harness without a socket
//! — the source of truth for the remaining list is the header of `tests/layout_commands.rs`.
//! ★But that requirement is **a cost, not an impossibility**★: `src/daemon_client/tests.rs` of the same package
//! sets up a loopback WS and actually runs that loop. What is bought here is the determinism and speed obtained without a socket.
//! ★Before 2026-08-24 this place said 「어떤 하네스로도 세울 수 없다」★ — that was true, but the
//! reason was not the socket but **a short-circuit that did not launch the connection task at all without an `AppHandle`**. When that short-circuit
//! was removed (source of truth = the `start_connection` 「단락 이야기」 of `mod.rs`), the sentence went stale along with it.
//!
//! ★Beware of name collision★: `connection.rs` has a **different** `CommandReply` (`oneshot::Sender`). So that file
//! aliases the envelope's reply type as `BusReply` — do not just bring both names into one scope.
// ADR-0155
// ADR-0081

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use engram_dashboard_command::{
    route, CommandDecl, CommandEnvelope, CommandError, CommandLink, CommandReply, CommandTable,
    ErrorCode, InboundCommands, OwnerLookup, OwnerLookupSource, ReplySink,
};

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
    /// ★The point is not taking the correlation key as an argument★: `request_id` is taken **directly** from the envelope to make the reply slot,
    /// so the caller has no way to send a different request's key (that mistake would attach the answer to someone else's round trip).
    /// `deliver` is called **exactly once** — whether an answer was given (`send`) or the task vanished without one (the `Drop` of [`ReplySink`]
    /// emits an error reply). So the caller does not handle 「답이 안 올 수도 있다」 (the answer may not come).
    /// ★`deliver` is called from the application task — not the connection task★. To write to the socket it must go through the channel
    /// that hands over to that task (`ConnectionCommand`) (single writer convention — `connection.rs` header 「동시성」).
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
