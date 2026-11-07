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
//! 보내는 쪽도 이제 있다 — 데몬이 자기 명부에서 주인을 찾아 그 연결로 봉투를 쓴다
//! (`engram_dashboard_daemon::command_delivery` · ADR-0154). 셸 표에 없는 이름은 여기서 한 홉 더 내려가
//! 웹뷰에 닿는다([`ViewCommandPort`] · 실물 = `crate::view_commands`).
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
    /// ★상관 키를 인자로 받지 않는 것이 요점이다★: `request_id` 를 봉투에서 **직접** 꺼내 답장 자리를
    /// 만들므로, 부르는 쪽이 다른 요청의 키를 실어 보낼 방법이 없다(그 실수는 남의 왕복에 답을 붙인다).
    /// `deliver` 는 **정확히 한 번** 불린다 — 답을 냈든(`send`), 태스크가 답 없이 사라졌든([`ReplySink`] 의
    /// `Drop` 이 오류 답장을 낸다). 그래서 부르는 쪽은 「답이 안 올 수도 있다」를 다루지 않는다.
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

// ── 자기 이름 등록 ────────────────────────────────────────────────────────────
//
// ★등록 패킷의 `owner` 는 **광고**다★ — 데몬은 이 값을 쓰지 않고 **그 패킷이 온 연결**에서 주인 토큰을
//   파생한다(protocol `AgentCommand::RegisterCommands` doc · daemon `note_claimed_owner`). 그래서 여기 무엇을
//   적어도 명부의 주인은 안 바뀐다 — 쓸모는 데몬 로그에서 이 연결이 누구인지 사람이 알아보는 것뿐이다.
// ★데몬의 토큰 접두(`conn-`)를 흉내내지 않는다★ — 그 형태를 적으면 데몬이 「남의 주인 토큰을 적은 등록」으로
//   보고 경고를 남긴다(`note_claimed_owner` 의 접두 분기).
//
// ★★이 값을 「클라이언트가 만든 식별자」로 승격하지 말 것(ADR-0150 결정 1)★★ — 그 식별자는 **연결마다
//   유일**해야 하는데 이 리터럴은 고정이라 두 셸이 같은 값을 쓴다. 데몬 명부는 등록을 이름 단위 last-wins 로
//   덮고 연결 해제를 주인 단위로 제거하므로(ADR-0150 결정 3), 그 승격은 **두 셸이 서로의 이름을 빼앗고 서로를
//   지우는** 동작이 된다. 식별자를 들일 때는 이 상수를 고치는 것이 아니라 연결마다 새 값을 만들어야 한다.
const SHELL_OWNER_ADVERT: &str = "engram-dashboard-shell";

/// 등록·차분 패킷이 함께 쓰는 광고 문자열 — ★두 곳에서 따로 짓지 않는다★. 갈리면 데몬 로그에서 같은
/// 셸의 두 패킷이 남남으로 보인다(명부의 주인 판정은 이 값과 무관하다 — 위 주석).
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
