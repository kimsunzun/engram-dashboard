//! Daemon MCP Streamable HTTP server — the entrance of the control channel that spawned claude agents attach to via mcp-config.
//!
//! Nests the rmcp `StreamableHttpService` (a Tower service) at axum `/mcp` behind bearer auth middleware. The CLI plain-text routes ride the same server, port and
//! middleware — their canonical list is `ControlRoute` (not recounted here). Tool surface = `engram_ping` · `eg_send` · `eg_messages`.
//! Control verbs are not MCP tools (ADR-0132 decision = CLI): tool schemas are resident in every agent context while control is low-frequency, so `/control/agent` has no `#[tool]`.
//!
//! OAuth metadata is not advertised (load-bearing, #59467): StreamableHttpService creates no `.well-known/*` routes and we add none — when a server advertises
//! OAuth metadata, claude ignores the static Authorization header (claude-code #59467); without it the static Bearer is sent as is (ADR-0086 §rationale).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use engram_dashboard_agent::manager::AgentManager;
use http::{Method, Request, StatusCode};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ServerCapabilities, ServerInfo};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{schemars, tool, tool_handler, tool_router, ErrorData, RoleServer, ServerHandler};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use engram_dashboard_messaging::envelope::Entrance; // ADR-0110: the entrance label is owned by the kernel crate.

use super::ingress::{handle_messages, handle_send, ControlCommand, SendContract};
use super::registry::{BoundIdentity, ControlRegistry};
// Caller strings going into log fields pass through this function, not `agent::preview`: the split is canonical in the [`super::agent::preview`] and [`sanitize_for_log`] docs.
use crate::connection_core::sanitize_for_log;

/// The axum path the MCP server is mounted at; the mcp-config url points at it (`http://127.0.0.1:<port>/mcp`).
///
/// keep-in-sync (M5): `ControlEndpoint.url` is this path attached to the MCP route, and CLI entrance injection derives the CLI base URL (ENGRAM_CONTROL_URL) by stripping this
/// literal suffix as a string — `crates/engram-dashboard-agent/src/backend/mod.rs` (strip_suffix("/mcp") in `inject_cli_entrance`), shared by all backends.
/// Changing this value requires changing that literal; the build does not enforce it, and on drift the base derivation goes wrong and the CLI silently gets a 404 (codex spawns too, not only claude).
const MCP_PATH: &str = "/mcp";

/// CLI entrance (ADR-0086 step 2) — the plain JSON route `engram mail send` POSTs to; the CLI assembles this path onto the base URL (ENGRAM_CONTROL_URL).
const CONTROL_SEND_PATH: &str = "/control/send";

/// CLI query entrance (D · canonical surface = ADR-0132) — `engram mail status <id>` / `engram mail pending` POST here.
/// POST, not GET: it must pass the same bearer middleware, which cuts a GET without a session id with 400 (bearer_auth step 1.5). The query uses no session, so it takes the session-less POST form as send; auth rules are not split per path.
const CONTROL_MESSAGES_PATH: &str = "/control/messages";

/// CLI control entrance (ADR-0132 decision 6) — `engram agent <verb>` POSTs here. The verb is in the body (`{verb: …}`), not the path: the CLI's HTTP client is one std-only POST, and a path per verb would need a path table in it.
/// Auth is the same middleware as `/control/send` (session-less POST; see `CONTROL_MESSAGES_PATH`).
/// Control is open to everyone (ADR-0132 decision 5): a spawned agent reaches it regardless of backend; only mail is split by credential (`ControlRoute::is_mail`).
const CONTROL_AGENT_PATH: &str = "/control/agent";

/// CLI discovery entrance — the daemon's own table and the attached clients' registry as one list (`control::catalog`). Read-only.
/// POST for the reason in `CONTROL_MESSAGES_PATH`. Credentials with mail blocked can reach it: discovery is control plane, and a backend that cannot write letters still needs to know what it can call (`ControlRoute::is_mail`).
/// `pub(super)`: the recovery hint in `catalog` puts this path in its wording and its test compares this constant, the only way to stop the wording pointing at a nonexistent address.
pub(super) const CONTROL_COMMANDS_PATH: &str = "/control/commands";

/// CLI generic call entrance — calls a command by its full name (`{name, args}`).
///
/// Not an alias of `/control/agent` (that bears one family's `{verb}` vocabulary and per-family hints); one address with two meanings would weaken the closed route enumeration's forced classification.
/// The check is the same function as the sibling's (`commands::call_daemon_command`) — a second entrance without the check was the one hard constraint when adding this route.
const CONTROL_CALL_PATH: &str = "/control/call";

/// CLI help screen entrance — `engram help [word]` POSTs one screen word and the daemon produces the screen (`control::help`).
/// The path is not written out again here — the CLI and this server see one agent constant. Credentials with mail blocked can reach it: reading a mail screen is not mail, and the screen is the same for everyone (`ControlRoute::is_mail`).
// ADR-0285
const CONTROL_HELP_PATH: &str = engram_dashboard_agent::types::CLI_HELP_ROUTE;

/// Namespace prefix of control-plane paths — the basis for folding an unclassified path into fail-closed.
///
/// Compare on a path segment boundary (bare `starts_with` is forbidden): a string prefix would also fold `/controlfoo` and `/control-x`, rejecting a future route there for no reason. Targets: `/control` itself and everything under `/control/`.
// ADR-0133
const CONTROL_PATH_PREFIX: &str = "/control";

/// All the CLI plain-text routes the daemon opens (the `/mcp` nest is owned by rmcp).
///
/// The single list for router assembly and mail classification: `start_mcp_server_with_help` walks it, and a route entering it is forced to be classified at compile time by the exhaustive match in `is_mail`.
/// A control path outside the list folds into mail (`mail_gated_path`): the compiler cannot stop a hand-added `.route()` after the iteration, but a missing classification means rejection, not silent opening. Keep the `.route()` call site to this one iteration; when broken, the default is the safe side.
// ADR-0133
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlRoute {
    Send,
    Messages,
    Agent,
    Commands,
    Call,
    Help,
}

impl ControlRoute {
    const ALL: [ControlRoute; 6] = [
        Self::Send,
        Self::Messages,
        Self::Agent,
        Self::Commands,
        Self::Call,
        Self::Help,
    ];

    const fn path(self) -> &'static str {
        match self {
            Self::Send => CONTROL_SEND_PATH,
            Self::Messages => CONTROL_MESSAGES_PATH,
            Self::Agent => CONTROL_AGENT_PATH,
            Self::Commands => CONTROL_COMMANDS_PATH,
            Self::Call => CONTROL_CALL_PATH,
            Self::Help => CONTROL_HELP_PATH,
        }
    }

    /// Whether this route is mail — the axis the mail allow/deny rejection (ADR-0133 decision 3) hangs on.
    ///
    /// Do not add a `_` arm: a catch-all would silently classify a new route as "not mail" (= everyone passes), with no runtime signal.
    /// The exhaustive match forces routes only — `/control/call` selects a command by the name in the body, so this axis cannot close it; [`mail_by_name`] does, invoked in the call route.
    /// Do not revert on the grounds "today the daemon table holds only `agent.*`, so it is unreachable" — false: the reachable set is the daemon table ∪ the registry (ADR-0160), whose names the dashboard sets
    /// (`src-tauri/src/layout/commands.rs`); whoever names them will never read this comment, so it cannot be left to prose.
    // ADR-0133
    // ADR-0160
    const fn is_mail(self) -> bool {
        match self {
            Self::Send | Self::Messages => true,
            // Control plane — discovery and calls are not mail: a credential that cannot write letters still needs to know what it can call and be able to call it (ADR-0132 decision 5).
            Self::Agent | Self::Commands | Self::Call => false,
            // Reading a screen is not mail — a mail-blocked credential must read the mail screen to learn that channel's contract (ADR-0220 decision 4).
            // ADR-0285
            Self::Help => false,
        }
    }

    /// A path outside the list (e.g. `/mcp` and below) is `None`.
    fn from_path(path: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.path() == path)
    }
}

/// Whether the mail-rejection gate applies to this request path (ADR-0133 decision 3).
///
/// A missing classification folds into rejection (fail-closed): a listed route follows its own classification, a control path outside the list (`/control…`) is treated as mail. The gate's purpose is rejection, so a forgotten classification
/// must default to "blocked" — otherwise it opens an entrance at runtime with no signal.
/// `/mcp` and paths outside control are not targets: MCP is that credential's own channel (ADR-0128 decision 1), and the rest this server does not open.
// ADR-0133
fn mail_gated_path(path: &str) -> bool {
    match ControlRoute::from_path(path) {
        Some(route) => route.is_mail(),
        None => {
            path == CONTROL_PATH_PREFIX
                || path
                    .strip_prefix(CONTROL_PATH_PREFIX)
                    .is_some_and(|rest| rest.starts_with('/'))
        }
    }
}

/// The bounce code for a credential that cannot use this HTTP/CLI mail entrance (ADR-0133 decision 3).
///
/// Read the name's scope narrowly — it is not "mail forbidden": a spawn getting this code (an MCP-capable backend) writes mail normally through `eg_send` on `/mcp`. The channel is split only by backend capability with no runtime switching
/// (ADR-0128 decision 1), and this gate closes the entrance that must stay closed in that design; "can bypass via MCP" is not a hole.
const MAIL_NOT_ALLOWED_CODE: &str = "MAIL_NOT_ALLOWED";

/// Do not announce an alternative channel (ADR-0133 decision 3): saying "use this instead" would bypass the gate the two priming files sealed by never mentioning each other's channel (the real gate of ADR-0128).
/// So the wording says only "this credential cannot write mail", names no other channel or tool, and tells that retrying is pointless (preventing infinite retries).
const MAIL_NOT_ALLOWED_HINT: &str =
    "This credential is not allowed to use mail. Retrying will not change that.";

/// Whether this command name is mail — the mail axis for a route selecting a command by body (`/control/call`).
///
/// The reachable set is the daemon table ∪ the registry (ADR-0160), whose names are set outside this crate (the dashboard adds its own), so this axis includes names unknowable from this code and "no such name today" cannot be the grounds.
/// The family separator is the command-name convention (`<family>.<verb>`): family `mail` = mail. No prefix comparison — `mailbox.x` is not mail but a prefix check catches it. The convention's source is the command table; it is not a shared
/// constant because the CLI carries its own copy too (`engram-dashboard-cli/src/bin/engram.rs`) — decide where a constant lives before making a third copy.
/// Today no name makes this true (measured: the daemon table has only `agent.*`, the dashboard names no `mail.*`), so this is a guardrail, not a hole-plug: whichever side `mail.*` comes from, this line stands that day.
// ADR-0133
// ADR-0160
fn mail_by_name(name: &str) -> bool {
    name.split_once('.')
        .is_some_and(|(group, _)| group == engram_dashboard_agent::types::CLI_GROUP_MAIL)
}

/// Late-injection slot for the manager (breaking the cycle): daemon startup brings the MCP server up first (it must create the DaemonControlChannel issuing mcp-config with that URL) and wires the AgentManager afterwards, so the server gets
/// an empty slot, filled with `set` right after the manager is assembled. Request handling (send) runs after the accept loop, so it is always filled by then.
#[derive(Default)]
pub struct ManagerSlot {
    inner: std::sync::OnceLock<Arc<AgentManager>>,
}

impl ManagerSlot {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&self, manager: Arc<AgentManager>) {
        let _ = self.inner.set(manager);
    }
    fn get(&self) -> Option<&Arc<AgentManager>> {
        self.inner.get()
    }
}

/// Late-injection slot for MessagingService (C1): same shape as ManagerSlot; MessagingService wraps AgentManager (DeliveryPort), so it can only be created after the manager.
#[derive(Default)]
pub struct MessagingSlot {
    inner: std::sync::OnceLock<Arc<engram_dashboard_messaging::service::MessagingService>>,
}

impl MessagingSlot {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&self, svc: Arc<engram_dashboard_messaging::service::MessagingService>) {
        let _ = self.inner.set(svc);
    }
    /// Also called by messaging_host's flush lane (`run_flush_lane`), so it is exposed inside the crate.
    pub(crate) fn get(
        &self,
    ) -> Option<&Arc<engram_dashboard_messaging::service::MessagingService>> {
        self.inner.get()
    }
}

/// Late-injection slot for the registry-notification fan-out (ADR-0132): same shape as ManagerSlot; the fan-out derives from the connection registry, assembled after the MCP server (lib.rs `run()` 5c → 6).
///
/// Empty is normal in some assemblies (smoke bins and harnesses have no clients to attach); empty in a production daemon means a registry changed by a control verb is not reflected on screen (`agent::RosterBroadcast`).
#[derive(Default)]
pub struct RosterBroadcastSlot {
    inner: std::sync::OnceLock<Arc<dyn super::agent::RosterBroadcast>>,
}

impl RosterBroadcastSlot {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&self, broadcast: Arc<dyn super::agent::RosterBroadcast>) {
        let _ = self.inner.set(broadcast);
    }
    /// The command table's notification adapter reads it too — on every call (`commands` module).
    pub(crate) fn get(&self) -> Option<&Arc<dyn super::agent::RosterBroadcast>> {
        self.inner.get()
    }
}

/// Late-injection slot for the daemon command table (ADR-0155): same shape as ManagerSlot; the table holds the manager so it is created after it, yet the slot holding that manager must exist before the MCP server.
/// A late table does not make registry notification late: the table holds the fan-out slot above, not its value (`commands::make_daemon_table`).
#[derive(Default)]
pub struct CommandTableSlot {
    inner: std::sync::OnceLock<Arc<super::commands::DaemonTable>>,
}

impl CommandTableSlot {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&self, table: Arc<super::commands::DaemonTable>) {
        let _ = self.inner.set(table);
    }
    /// The `/control/agent` adapter reads it on every request; empty → 503 (a wiring-order fault, not a request-format or auth problem, so not 4xx).
    pub(crate) fn get(&self) -> Option<&Arc<super::commands::DaemonTable>> {
        self.inner.get()
    }
}

/// MCP session identification header name (rmcp/spec standard, lowercase).
const SESSION_ID_HEADER: &str = "mcp-session-id";

/// The `eg_send` MCP tool name = single source (ADR-0094). The `#[tool]` method name below is the name rmcp exposes in `tools/list`; this const is its canonical source — the ADR-0094 send-permission grant uses it
/// to build the `mcp__{server}__{tool}` pattern (DaemonControlChannel.provision). Knowledge of claude grammar (`mcp__..`) lives only in backend/claude/ (ADR-0004/0094).
pub const SEND_MESSAGE_TOOL: &str = "eg_send";

/// The `eg_messages` MCP tool name (D · spec §6) — same discipline as `SEND_MESSAGE_TOOL`. Not a grant target (intentional): ADR-0094 pre-authorizes only the send entrance (control/mod.rs `build_grants`).
pub const MESSAGES_TOOL: &str = "eg_messages";

pub struct McpServerHandle {
    /// Endpoint URL to embed in mcp-config (e.g. `http://127.0.0.1:54321/mcp`).
    pub url: String,
    /// Shutdown signal — cancelling it cleans up the accept loop + active sessions.
    cancel: CancellationToken,
    /// The axum::serve task handle (awaited on shutdown — prevents test leaks). `Option` so that in a type with Drop it is taken with `take` and `shutdown(self)` can move and await it (a Drop type cannot have a field partially moved out — round-2 F5).
    serve_handle: Option<tokio::task::JoinHandle<()>>,
}

impl McpServerHandle {
    pub async fn shutdown(mut self) {
        self.cancel.cancel();
        if let Some(h) = self.serve_handle.take() {
            let _ = h.await;
        }
    }
}

impl Drop for McpServerHandle {
    /// drop-on-error airtight (round-2 F5): if the handle is dropped without `shutdown().await` (e.g. a later startup step such as the daemon.json write fails and an error return drops it), the detached serve task may keep running.
    /// Firing the cancel token in Drop makes the serve task (graceful_shutdown observes cancel) exit however the drop happens; after a normal `shutdown()` it is an idempotent no-op.
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Identity verification = middleware (FIX 12): the auth middleware verifies identity and plants it in the request extensions (BoundIdentity), and session↔identity binding, pinning and cleanup are all done there (registry via State) — the handler only reads the identity.
#[derive(Clone)]
pub struct EngramMcpHandler {
    tool_router: ToolRouter<Self>,
    manager: Arc<ManagerSlot>,
    /// The same Arc shared with the middleware — no second registry.
    registry: Arc<ControlRegistry>,
    messaging: Arc<MessagingSlot>,
}

#[tool_router]
impl EngramMcpHandler {
    pub fn new(
        manager: Arc<ManagerSlot>,
        registry: Arc<ControlRegistry>,
        messaging: Arc<MessagingSlot>,
    ) -> Self {
        Self {
            tool_router: Self::tool_router(),
            manager,
            registry,
            messaging,
        }
    }

    /// Calling this tool proves the session binding works end to end (acceptance observation point).
    #[tool(description = "Diagnostic ping — returns pong and the caller's bound agent identity")]
    async fn engram_ping(
        &self,
        _params: Parameters<PingArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let identity = ctx
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<BoundIdentity>().copied());
        match identity {
            Some(BoundIdentity { agent_id, epoch }) => {
                Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                    "pong agent={agent_id} epoch={epoch}"
                ))]))
            }
            None => Err(ErrorData::invalid_request(
                "no bound identity in request context (auth middleware should have set it)",
                None,
            )),
        }
    }

    /// This method name (`eg_send`) must equal the `SEND_MESSAGE_TOOL` constant (ADR-0094 single source). The rmcp `#[tool]` macro uses the method name as the `tools/list` name (2.2.0
    /// tool.rs: `name = attribute.name.unwrap_or_else(|| fn_ident.to_string())`) and accepts only a String literal, so a const cannot be injected (verified); the runtime test
    /// `tools_list_exposes_send_message_tool` ties the two together.
    /// To rename it, change the `SEND_MESSAGE_TOOL` const too — otherwise the grant points at a nonexistent tool and the send entrance is silently blocked, caught only by the test.
    // ADR-0086 / ADR-0094 (single-source coupling)
    #[tool(
        description = "Sends a message to a teammate agent and answers requests it received. This is a tool of the \
        `engram` broker, not the harness's built-in `SendMessage` — that one is blocked in this team, and calling it ends in a permission \
        refusal.\n\
        The arguments are described by each field's description; how to read the result and the other rules are in `engram help mail`."
    )]
    async fn eg_send(
        &self,
        params: Parameters<SendArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(from) = ctx
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<BoundIdentity>().copied())
        else {
            return Err(ErrorData::invalid_request(
                "no bound identity in request context (auth middleware should have set it)",
                None,
            ));
        };

        let Some(manager) = self.manager.get() else {
            // F6 instrumentation (error, M4): the manager slot is not filled yet (wiring-order fault). No safe fallback (the message cannot be delivered) → error, not warn (logging-conventions: error = a human must see this = a wiring defect).
            tracing::error!(
                entrance = "mcp",
                "control channel send unavailable — manager slot not set (wiring order anomaly, ADR-0086 F6)"
            );
            return Err(ErrorData::internal_error(
                "control channel not ready (manager not wired)",
                None,
            ));
        };
        let Some(messaging) = self.messaging.get() else {
            tracing::error!(
                entrance = "mcp",
                "control channel send unavailable — messaging slot not set (wiring order anomaly, C1)"
            );
            return Err(ErrorData::internal_error(
                "control channel not ready (messaging not wired)",
                None,
            ));
        };
        let Parameters(SendArgs {
            to,
            body,
            request,
            reply_by,
            reply_to,
        }) = params;
        let cmd = ControlCommand {
            from,
            to: to.into_tokens(),
            body,
            contract: SendContract {
                request: request.unwrap_or(false),
                reply_by,
                reply_to,
            },
        };
        // Blocking boundary (C4 review fix D · load-bearing): `inject` inside `handle_send` is a blocking write to the child's stdin, borne serially once per member of a group broadcast. Called directly from this async handler, one blocked
        //   pipe would hold a whole tokio worker (workers = cores) and head-of-line block other requests on it. So it goes to the blocking pool — a single send does the same write and gets the same treatment (no two disciplines),
        //   as the flush lane (service.rs FlushTrigger).
        //
        // The price = at-least-once delivery (round-3 fix 7, intended): a `spawn_blocking` closure cannot be aborted. If the caller cancels midway (HTTP drop · MCP client exit · timeout), this `.await` disappears but the blocking task runs to the end,
        //   so delivery and the ledger commit still happen and only the response is lost; a retry is a new send with a new `msg_id` (no idempotency key), delivering the same content again.
        //   The opposite (cancel delivery on cancel) is impossible — bytes already written to child stdin cannot be taken back, and "check cancellation before writing" is another TOCTOU. So duplicate > loss: a duplicate is a visible
        //   fact the receiving LLM can judge, a loss is silent (the direction of ADR-0103 "no silent loss").
        //   Future: accept a sender-chosen idempotency key (same value on retry) and fold duplicates in the ledger; `msg_id` is daemon-generated and cannot play that role.
        let (manager, registry, messaging) =
            (manager.clone(), self.registry.clone(), messaging.clone());
        let result = tokio::task::spawn_blocking(move || {
            handle_send(&manager, &registry, &messaging, Entrance::Mcp, cmd)
        })
        .await
        .map_err(|e| {
            tracing::error!(
                entrance = "mcp",
                "control channel send task failed (panic): {e}"
            );
            ErrorData::internal_error("send task failed", None)
        })?;
        let json = serde_json::to_string(&result.to_json()).unwrap_or_default();
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Asks the Engram broker about message states. How to read the rows is in `engram help mail`."
    )]
    async fn eg_messages(
        &self,
        params: Parameters<MessagesArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(from) = ctx
            .extensions
            .get::<http::request::Parts>()
            .and_then(|parts| parts.extensions.get::<BoundIdentity>().copied())
        else {
            return Err(ErrorData::invalid_request(
                "no bound identity in request context (auth middleware should have set it)",
                None,
            ));
        };
        let (Some(manager), Some(messaging)) = (self.manager.get(), self.messaging.get()) else {
            tracing::error!(
                entrance = "mcp",
                "messages query unavailable — manager/messaging slot not set (wiring order anomaly)"
            );
            return Err(ErrorData::internal_error("control channel not ready", None));
        };
        let Parameters(MessagesArgs { id }) = params;
        let result = handle_messages(manager, messaging, from, id.as_deref());
        let json = serde_json::to_string(&result.to_json()).unwrap_or_default();
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }
}

/// Tool args — ping takes no args (empty struct). The input schema is generated automatically by schemars (re-exported by rmcp).
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct PingArgs {}

/// Plural form of `to` (spec §6 — ADR-0111 multiple recipients): **either one string or an array of strings**.
///
/// A single string is still valid (backward compatible), so existing calls and priming work unchanged. Each element is
///   an agent name, an agent id, or an `@` address, and the kinds **may be mixed**.
/// Elements of an MCP array are **never split on commas** (spec §6): the array structure already gives the boundary, so there is
///   no second splitting — an element `"a,b"` is treated as **one name** of that form (and if it does not exist, you get a
///   `RECIPIENT_NOT_FOUND` row). Comma splitting is a rule only of the CLI entrance (`/control/send`).
/// Because of `#[serde(untagged)]`, the input schema comes out as `anyOf[string, array<string>]`, so you can see from the schema
///   alone that both forms are valid.
// ADR-0111
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum ToField {
    /// One recipient (agent name, agent id, or `@` address).
    One(String),
    /// Several recipients (each element is an agent name, agent id, or `@` address).
    Many(Vec<String>),
}

impl ToField {
    pub fn into_tokens(self) -> Vec<String> {
        match self {
            ToField::One(s) => vec![s],
            ToField::Many(v) => v,
        }
    }
}

/// Arguments of `eg_send` — recipient selection (`to`) + body (`body`) + reply contract (C3, all optional). ★No from field★:
/// the sender is derived only from the session identity (payload from is forbidden — ADR-0086 invariant). The input schema is generated automatically by schemars.
///
/// ★No type-string argument (structural — ADR-0103 invariant)★: `type` is not accepted as a string; only `request: bool` exists —
///   so an agent has no surface at all to smuggle in `type="notice"` (a daemon-only tag).
/// ★Doc comments = tool schema descriptions★: schemars carries these comments as property descriptions (the contract the receiving LLM reads).
// ADR-0103 (C3 — spec §6 eg_send { to, body, request?, reply_by?, reply_to? })
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct SendArgs {
    /// Who to send to: one teammate, or a list. Each entry is an agent name, an exact agent id, or a
    /// group address: "@here" (everyone live except you) or "@all" (every agent in the team tree
    /// except you, including ones that are not running — their copy is delivered when they come
    /// back). You can mix them, e.g. ["@here", "qa-bravo"].
    pub to: ToField,
    /// Message body (text).
    pub body: String,
    /// Set true when you need answers back: the broker tracks this message as awaiting a reply.
    /// With several recipients this opens ONE INDEPENDENT reply contract per recipient — each of
    /// them owes you their own answer, one of them replying does not close the others, and each
    /// silent one gets its own deadline notice. Mutually exclusive with reply_to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<bool>,
    /// Reply deadline for a request, as an integer + unit: "5m", "10m", "1h" (minimum 1 minute —
    /// deadlines are checked once a minute, so anything shorter is rejected; "60s" is accepted).
    /// Only valid together with request. On timeout the broker notifies YOU (the sender), not the recipient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_by: Option<String>,
    /// The id of the request you are answering (the `id` attribute on the message you received).
    /// A reply goes to EXACTLY ONE recipient — the agent that sent the request; passing several
    /// recipients (or any "@" address) with reply_to is rejected. Mutually exclusive with request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
}

/// Arguments of `eg_messages` (D · spec §6 `eg_messages { id? }`). All optional — calling with no arguments lists your own pending messages.
/// ★No identity field★: "me" comes only from the session identity (you cannot view someone else's pending items via payload — ADR-0086 invariant).
#[derive(Debug, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
pub struct MessagesArgs {
    /// A message id to inspect (e.g. "m-7f3k9q2d"). Omit to list your own open items instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

// router = self.tool_router — actually reading the stored field avoids dead_code, and the router is not
// rebuilt per handler (the factory calls new() per session, so building the router once into a field is more efficient).
#[tool_handler(router = self.tool_router)]
impl ServerHandler for EngramMcpHandler {
    fn get_info(&self) -> ServerInfo {
        // ServerInfo (= InitializeResult) is #[non_exhaustive], so a struct literal is impossible → use the ctor chain.
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Engram daemon control channel (ADR-0086). Available tools: engram_ping, eg_send, eg_messages.",
        )
    }
}

/// Bearer auth middleware (ADR-0086) — verifies the token before the MCP handshake and pins session↔identity.
///
/// Check order:
///   1. `Bearer <token>` in Authorization → registry.validate; failure 401 (inner not called = no handshake created).
///   1.2. Mail allow/deny (ADR-0133 decision 3) — a mail route with a mail-blocked credential is bounced here; `/mcp` and control routes are not caught (`ControlRoute::is_mail`).
///   1.5. Mcp-Session-Id format/required-ness → 400; this guarantees step 2's "a session op always resolves to a binding" without relying on rmcp's internal 4xx behavior.
///   2. Session binding check (FIX 7 + round-2 F1) — match = pass / identity mismatch = 403 / no binding = 404. initialize is skipped (no session id yet; the session is created in the response). On DELETE, after identity is confirmed, the binding is pruned (FIX 8/F6).
///   3. Plant the verified identity in the request extensions and pass to inner (StreamableHttpService); rmcp passes it to the tool as `http::request::Parts` (the official custom-extension pattern).
///   4. If the response has a new Mcp-Session-Id (initialize succeeded), pin the identity once with `bind_session_if_absent`; a failure (duplicate/dead) skips the binding (a duplicate is harmless, a dead one gets 401/403 next).
///
/// 401/403 live in the middleware (before the handshake): rmcp has no built-in auth (the official pattern = axum middleware), and deferring verification into the handshake would let even a wrong token create a session — blocking here guarantees rejection precedes any MCP session state change.
async fn bearer_auth<B>(
    State(registry): State<Arc<ControlRegistry>>,
    request: Request<B>,
    next: Next,
) -> Response
where
    B: Send + 'static,
    Request<B>: Into<Request<axum::body::Body>>,
{
    // The strict "Bearer " prefix is intentional (FIX 13): the daemon authored this header itself in mcp-config (claude sends it as is), so its format is fixed — no lenient parsing of case variants or extra whitespace.
    let token = request
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string());

    let Some(token) = token else {
        return unauthorized();
    };
    let Some(binding) = registry.validate(&token) else {
        return unauthorized();
    };
    let identity = binding.identity;

    // Mail rejection is here, not in the handler (ADR-0133 decision 3): this middleware wraps the whole router, so growing mail routes are covered in one place (a per-handler check would be missed by a new handler). The decision input is one fact fixed at credential issue time — no manager lookup or body parsing.
    // This is the only enforcement point: nothing on the agent side blocks mail (the CLI sends a sensible mail call as is). Remove this check and mail is open.
    // It blocks only this HTTP/CLI mail surface: an MCP-capable agent keeps writing mail through `eg_send` on `/mcp` (ADR-0128 decision 1). The rejection is "this entrance is not your channel", not "mail forbidden".
    // warn, not info: the default log level is warn, so with info an operator chasing "why can't this agent send letters" would find no trace, and the ADR-0133 re-review trigger (observing an agent that got only rejections) fires only if this line is visible; same level as the sibling rejections (401/403/404/400).
    // This surface has two mail axes — this is the path axis; a route selecting a command by body makes the same decision and log in its own place (the name axis in `control_call_handler`).
    // ADR-0133
    if !binding.mail_allowed && mail_gated_path(request.uri().path()) {
        tracing::warn!(
            agent = %identity.agent_id,
            epoch = identity.epoch,
            "mail request refused — this credential is not for the CLI/HTTP mail entrance (ADR-0133)"
        );
        return mail_not_allowed();
    }

    // The existing session id the request carries (if any): follow-ups after initialize (tools/call · GET · DELETE) carry Mcp-Session-Id; identity pinning is checked with it (initialize has none).
    // malformed ≠ absent (Codex LOW): a header present but failing to_str() (non-UTF-8) must not fold to None — a session-carrying request would be mistaken for sessionless, skip the binding check and reach rmcp (boundary bypass). Present-but-malformed is
    //   a client error, cut with 400 (empty body; not an identity/auth problem) before the binding check. Only a truly absent header is "sessionless" (the initialize path).
    let method = request.method().clone();
    let req_session_id = match request.headers().get(SESSION_ID_HEADER) {
        None => None,
        Some(v) => match v.to_str() {
            Ok(s) => Some(s.to_string()),
            Err(_) => {
                tracing::warn!(
                    "control channel malformed Mcp-Session-Id header refused (400, ADR-0086 Codex LOW)"
                );
                return bad_request();
            }
        },
    };

    // Session operations (GET/DELETE) must resolve to a session binding (security lens): GET (SSE stream) and DELETE (teardown) act on an existing session and must carry a session id; without one they cannot end in a binding, so they are cut here with 400 instead of
    //   relying on rmcp's internal 4xx ("no inner reach without a binding"). A POST without a session id is the initialize path (the session is created in the response).
    if req_session_id.is_none() && (method == Method::GET || method == Method::DELETE) {
        tracing::warn!(
            method = %method,
            "control channel session operation without session id refused (400, ADR-0086 — session ops must resolve through the binding)"
        );
        return bad_request();
    }

    if let Some(sid) = &req_session_id {
        match registry.identity_for_session(sid) {
            Some(bound) if bound == identity => {}
            Some(_) => {
                // The session id is also a caller-chosen header value, so it goes through [`sanitize_for_log`] (same forgery axis): the HTTP parser filters CR/LF but tabs and non-ASCII obs-text pass and the length is not ours — decide the width here too.
                tracing::warn!(
                    session = %sanitize_for_log(sid),
                    "control channel cross-token session takeover refused (403, ADR-0086 FIX 7)"
                );
                return forbidden();
            }
            // Orphaned-session rejection (round-2 F1): a session id is carried but the daemon has no binding. It used to pass to rmcp for a 404, but that has a fatal window: agent A opens session S and is bound, a revoke (kill) prunes only the binding while
            //   rmcp's session S may live; agent B with a valid token presenting S would attach to A's orphaned session worker (session hijack). Now every request carrying a session id without a binding is rejected, so a session alive in rmcp but unknown to the daemon is an
            //   unreachable orphan (also making the DELETE-prune order fail-safe). A truly unknown id got rmcp's 404 before too, so a normal client sees no change.
            None => {
                tracing::warn!(
                    session = %sanitize_for_log(sid),
                    "control channel orphaned/unknown session refused (404, ADR-0086 F1)"
                );
                return not_found();
            }
        }
        // unbind-before-inner order (round-2 F6): the daemon binding is removed before inner (rmcp) closes the session. With F1 in place this is fail-safe: if the inner close fails after the unbind, the leftover rmcp session is an unreachable orphan, no security window.
        //   Unbind-after-close would hang the prune on the close succeeding and risk unbounded growth and stale bindings on failure. It runs after the identity check (the match above), so it is not an arbitrary prune.
        if method == Method::DELETE {
            registry.unbind_session(sid);
        }
    }

    let mut request: Request<axum::body::Body> = request.into();
    request.extensions_mut().insert(identity);
    // The permission bit is carried separately from the identity: `BoundIdentity` is the session-pinning key, and mixing permission in would disguise an authorization problem as a false session-hijack alarm (see that type's doc).
    //   The handler must still see this bit in one place — the mail axis of a body-selected route ([`mail_by_name`]) cannot be closed by route classification.
    request.extensions_mut().insert(binding);

    let response = next.run(request).await;

    // Pass the same token string used for validation (FIX 7 + round-2 F2): bind compares locally whether that token is still this agent's current credential, filtering a revoke/re-issue in the validate→bind window.
    if let Some(session_id) = response
        .headers()
        .get(SESSION_ID_HEADER)
        .and_then(|v| v.to_str().ok())
    {
        registry.bind_session_if_absent(session_id, identity, &token);
    }
    response
}

/// 401 response (empty body). WWW-Authenticate is deliberately omitted (static Bearer, no challenge needed).
fn unauthorized() -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(axum::body::Body::empty())
        .expect("valid 401 response")
}

/// 403 response (empty body) — rejects cross-token session hijack (FIX 7): the token is valid (hence not 401) but not permitted this session (pinned to another identity).
fn forbidden() -> Response {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .body(axum::body::Body::empty())
        .expect("valid 403 response")
}

/// 404 response (empty body) — rejects an orphaned/unknown session (round-2 F1): a request carrying a session id with no daemon binding. The token is valid but the session does not exist from the daemon's authorization view (not established to belong to another identity, so not 403); the body is empty so no session or identity information leaks.
fn not_found() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(axum::body::Body::empty())
        .expect("valid 404 response")
}

/// Mail rejection response (ADR-0133 decision 3).
///
/// 200 + a bounce envelope, not an empty 403: mail routes' contract is "always 200 + JSON" and the CLI decides by exit code, not stdout shape — this verified bounce shape folds into the existing three-way split's failure (1) (no new outcome class);
/// an empty 403 would print only a blank line. Body wording is canonical in `MAIL_NOT_ALLOWED_HINT` (it does not announce an alternative channel).
// ADR-0133
fn mail_not_allowed() -> Response {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "error",
            "code": MAIL_NOT_ALLOWED_CODE,
            "hint": MAIL_NOT_ALLOWED_HINT,
        })),
    )
        .into_response()
}

/// 400 response (empty body) — a request-format error (ADR-0086), not identity/auth, hence not 401/403/404; empty so no information leaks.
fn bad_request() -> Response {
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .body(axum::body::Body::empty())
        .expect("valid 400 response")
}

// ── CLI entrance (/control/send) ────────────────────────────────────────────────────────

/// `/control/send` request body `{to, body, request?, reply_by?, reply_to?}` — no from (identity only from the token). The C3 args are optional, so wire-compatible with the old `{to, body}` (omitted = notice).
#[derive(Debug, serde::Deserialize)]
struct SendRequest {
    /// Recipients — one string (a comma list is allowed) or an array.
    to: ToField,
    body: String,
    #[serde(default)]
    request: Option<bool>,
    #[serde(default)]
    reply_by: Option<String>,
    #[serde(default)]
    reply_to: Option<String>,
}

/// Recipient tokenization of the CLI entrance (pure function — review C3): `engram mail send --to a,b` can only express a list as a comma, so it is split once at this entrance only. The splitting rule differs per entrance on purpose (MCP side: `ToField` comment).
fn cli_recipient_tokens(to: ToField) -> Vec<String> {
    to.into_tokens()
        .into_iter()
        .flat_map(|t| t.split(',').map(|p| p.to_string()).collect::<Vec<_>>())
        .collect()
}

/// `/control/send` route State — the same Arc as the MCP factory (no second registry).
#[derive(Clone)]
struct ControlSendState {
    manager: Arc<ManagerSlot>,
    registry: Arc<ControlRegistry>,
    messaging: Arc<MessagingSlot>,
}

/// Always 200 + JSON body (success and corrective errors both ride an open request — the CLI parses the JSON for the exit code).
async fn control_send_handler(
    axum::extract::State(state): axum::extract::State<ControlSendState>,
    identity: Option<axum::Extension<BoundIdentity>>,
    body: Option<Json<SendRequest>>,
) -> Response {
    let Some(axum::Extension(from)) = identity else {
        // The middleware planted no identity = an auth-path fault; defensive 401 (unreachable normally).
        return unauthorized();
    };
    let Some(Json(req)) = body else {
        return bad_request();
    };
    let Some(manager) = state.manager.get() else {
        tracing::error!(
            entrance = "cli",
            "control channel send unavailable — manager slot not set (wiring order anomaly, ADR-0086 F6)"
        );
        return service_unavailable();
    };
    let Some(messaging) = state.messaging.get() else {
        tracing::error!(
            entrance = "cli",
            "control channel send unavailable — messaging slot not set (wiring order anomaly, C1)"
        );
        return service_unavailable();
    };
    let cmd = ControlCommand {
        from,
        to: cli_recipient_tokens(req.to),
        body: req.body,
        contract: SendContract {
            request: req.request.unwrap_or(false),
            reply_by: req.reply_by,
            reply_to: req.reply_to,
        },
    };
    let (manager, registry, messaging) =
        (manager.clone(), state.registry.clone(), messaging.clone());
    let Ok(result) = tokio::task::spawn_blocking(move || {
        handle_send(&manager, &registry, &messaging, Entrance::Cli, cmd)
    })
    .await
    else {
        // JoinError = blocking task panic (absent normally). "Always 200 + JSON" covers when the handler produced an answer; with no answer at all this goes to 500 so the CLI does not misread it as success (no new wire error code — spec §6 vocabulary is fixed).
        tracing::error!(entrance = "cli", "control channel send task failed (panic)");
        return Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(axum::body::Body::empty())
            .expect("valid 500 response");
    };
    Json(result.to_json()).into_response()
}

/// `/control/messages` request body (D) — `{id?}`. No identity field (derived from the token, as send).
#[derive(Debug, Default, serde::Deserialize)]
struct MessagesRequest {
    #[serde(default)]
    id: Option<String>,
}

/// Empty body allowed: `pending` (the no-argument query) has no field. A missing or unparsable body folds into a no-argument query — a query has no side effects so leniency is safe, and a 400 would keep the CLI from telling "no argument" from "request broken" (send is the opposite: required fields and side effects → 400).
async fn control_messages_handler(
    axum::extract::State(state): axum::extract::State<ControlSendState>,
    identity: Option<axum::Extension<BoundIdentity>>,
    body: Option<Json<MessagesRequest>>,
) -> Response {
    let Some(axum::Extension(from)) = identity else {
        return unauthorized();
    };
    let (Some(manager), Some(messaging)) = (state.manager.get(), state.messaging.get()) else {
        tracing::error!(
            entrance = "cli",
            "messages query unavailable — manager/messaging slot not set (wiring order anomaly)"
        );
        return service_unavailable();
    };
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let result = handle_messages(manager, messaging, from, req.id.as_deref());
    Json(result.to_json()).into_response()
}

// ── CLI control entrance (/control/agent) ──────────────────────────────────────────────

/// `/control/agent` route State — only one command-table slot (ADR-0155).
///
/// It does not hold the manager or fan-out slots: the table holds both (`commands::make_daemon_table`); holding them again would create a second path to the same object and break per-verb notification counts (wake-up 0 · change 1) if notifications go out from two places. No registry or messaging either — control verbs use identity only for authorization (no sender derivation).
#[derive(Clone)]
struct ControlAgentState {
    commands: Arc<CommandTableSlot>,
}

/// Always 200 + JSON body (success and bounce alike) — the same contract as `/control/messages`, so the CLI's query decider applies as is.
///
/// Identity is used only for authorization ("me" does not appear in control verbs), but its existence is required — a call without a token must not reach here (defensive 401).
/// Blocking boundary: `handle_agent` is blocking (resume polling ~3 s + profile disk save); calling it from an async handler would tie up a tokio worker and block other requests on that thread (same reason send moved the child stdin write to the blocking pool).
async fn control_agent_handler(
    axum::extract::State(state): axum::extract::State<ControlAgentState>,
    identity: Option<axum::Extension<BoundIdentity>>,
    body: axum::body::Bytes,
) -> Response {
    if identity.is_none() {
        return unauthorized();
    }
    // Why not the `Json<…>` extractor: on deserialization failure it returns an empty 400 (mistyped values, duplicate keys, non-object bodies, broken JSON), and the caller (an LLM) retries without knowing what to fix. Deserializing directly lets serde's reason ride in the envelope; the route returns an empty body only for auth failure and size overrun.
    let req: super::agent::AgentRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return Json(
                super::agent::malformed_body(&e.to_string(), &String::from_utf8_lossy(&body))
                    .to_json(),
            )
            .into_response()
        }
    };
    let Some(table) = state.commands.get() else {
        tracing::error!(
            entrance = "cli",
            "control verb processing unavailable — command table slot not set (wiring order anomaly, ADR-0155)"
        );
        return service_unavailable();
    };
    let table = table.clone();
    let Ok(result) =
        tokio::task::spawn_blocking(move || super::agent::handle_agent(&table, req)).await
    else {
        // JoinError = blocking task panic: no answer at all, outside "always 200 + JSON" → 500 so the CLI does not misread it as success (as the send route). That this holds only in hook-less test builds is canonical in the `internal_error` comment.
        tracing::error!(entrance = "cli", "control verb task failed (panic)");
        return internal_error();
    };
    Json(result.to_json()).into_response()
}

// ── CLI discovery and generic call entrance (/control/commands · /control/call) ──────────────────────

/// State of the two catalog routes — holds the two sources discovery merges (`control::catalog::merge`).
///
/// The registry is a handle, not a slot: the table (`CommandTableSlot`) comes after the manager and needs late injection, but the registry is a shared `Clone` handle the assembly creates before the server (`lib.rs` run() · accept loop); another late-injection slot would add a pointless "if empty, half list" branch.
/// Do not create a second registry: what is held must be the one copy connections register into, or discovery returns an empty list forever with no error and no log.
#[derive(Clone)]
struct ControlCatalogState {
    commands: Arc<CommandTableSlot>,
    /// Holds the registry + the seat table + the first-stage table as one value — it must be the one copy the socket entrance holds (ADR-0160); a copy gives an empty discovery list and a relay with its own seat table.
    bus: crate::command_delivery::CommandBus,
}

/// Discovery list — success and bounce are both 200 + JSON; only no answer at all is 500 (scope: [`internal_error`] comment). An empty table slot is not 503: in the same state the WS list gives "half the roster", so a 503 would make two surfaces say different things (canonical: `catalog::handle_list` doc).
///
/// What `spawn_blocking` fixed and did not — do not assume:
///   - Fixed = executor hazard. `CommandRoster::entries` takes a blocking mutex, which parks an async worker instead of yielding; the blocking pool now bears the wait, matching the sibling (`control_agent_handler`).
///   - Not fixed = lock hold time. `entries` still clones the whole registry (cap 4096 × help 4 KiB) while holding the mutex shared with command delivery (`OwnerLookupSource::lookup`) and connection cleanup, so this route still contends with delivery — what changed is who waits.
///     The cloning discipline is the registry module's (the "known cost" in the `command_roster` header).
/// `decls()` rebuilds `help` on every call (tools crate `CommandTable::decls`) — leave it: the own table has 5 entries, the cost is dominated by the registry side, and memoization belongs to that crate; if the table grows, measure there.
async fn control_commands_handler(
    axum::extract::State(state): axum::extract::State<ControlCatalogState>,
    identity: Option<axum::Extension<BoundIdentity>>,
) -> Response {
    if identity.is_none() {
        return unauthorized();
    }
    let Ok(result) = tokio::task::spawn_blocking(move || {
        let mine = state
            .commands
            .get()
            .map(|table| table.decls())
            .unwrap_or_default();
        let registered = state.bus.roster().entries();
        super::catalog::handle_list(mine, registered)
    })
    .await
    else {
        tracing::error!(entrance = "cli", "discovery list task failed (panic)");
        return internal_error();
    };
    Json(result.into_json()).into_response()
}

/// Full-name call — always 200 + JSON (success and bounce alike), the same contract as `/control/agent`; the `Json<…>` extractor and `spawn_blocking` reasoning is canonical in `control_agent_handler` (same table, same function, same treatment).
///
/// This route may be delayed until the deadline (ADR-0160 impact section): another party's name is relayed to its owner and an unanswering owner yields `TIMEOUT` at `CommandDeliveries::DEFAULT_DEADLINE`; the daemon's own names finish on the spot.
/// `request_id` may be sent by the caller (ADR-0161 decision 1); otherwise the decider issues one (`catalog::handle_call`) — that split is not recreated here.
async fn control_call_handler(
    axum::extract::State(state): axum::extract::State<ControlCatalogState>,
    binding: Option<axum::Extension<super::registry::TokenBinding>>,
    body: axum::body::Bytes,
) -> Response {
    let Some(axum::Extension(binding)) = binding else {
        return unauthorized();
    };
    let caller = binding.identity;
    let req: super::catalog::CallRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return Json(
                super::catalog::malformed_body(&e.to_string(), &String::from_utf8_lossy(&body))
                    .to_json(),
            )
            .into_response()
        }
    };
    // The mail axis is closed by name: this route selects a command by body, so the middleware's route classification does not reach here ([`mail_by_name`]). It decides before taking anything needed for execution and before telling which source the name belongs to — daemon table and registry must answer the same.
    // The rejection is logged at the same level and fields as the sibling (path-axis gate in `bearer_auth`): otherwise a mail-blocked credential groping by name would leave no server trace, and the ADR-0133 re-review trigger would lose its material on this axis.
    // The name goes through [`sanitize_for_log`], not `agent::preview`: printed before validation it is an arbitrary string, and `preview` only truncates and lets control characters through — one newline forges a second log entry; as `warn` it really lands in the file at the default level.
    // ADR-0133
    if !binding.mail_allowed && mail_by_name(&req.name) {
        tracing::warn!(
            agent = %caller.agent_id,
            epoch = caller.epoch,
            command = %sanitize_for_log(&req.name),
            "mail request refused — this credential is not for the CLI/HTTP mail entrance (name axis, ADR-0133)"
        );
        return mail_not_allowed();
    }
    // This check is a wiring alarm, not a routing authority: the authority is the single `bus.locals()` below (decision and delivery see the same fact — `catalog::handle_call`); this line only surfaces loudly, before the name is looked at, an assembly plugging the table in late
    //   (canonical: the two-route comparison test in `tests/control_agent.rs`). The assembly must pass this slot and the bus's first-stage table as the same one copy for the alarm to point at the real state — the production assembly does (`lib.rs`).
    if state.commands.get().is_none() {
        tracing::error!(
            entrance = "cli",
            "full-name call unavailable — command table slot not set (wiring order anomaly, ADR-0155)"
        );
        return service_unavailable();
    }
    let bus = state.bus.clone();
    let locals = Arc::clone(bus.locals());
    let registered = bus.roster().clone();
    let Ok(outcome) = tokio::task::spawn_blocking(move || {
        super::catalog::handle_call(locals.as_ref(), &registered, req)
    })
    .await
    else {
        tracing::error!(entrance = "cli", "full-name call task failed (panic)");
        return internal_error();
    };
    let result = match outcome {
        super::catalog::CallOutcome::Answered(result) => result,
        super::catalog::CallOutcome::Relay {
            name,
            args,
            request_id,
        } => {
            // Record who ordered the command: a relayed name moves the dashboard's windows, tabs and slots (layout commands in `src-tauri`), so "who closed my window?" must be answerable. The material (`BoundIdentity`) already arrives at this gate but was only used for the existence check.
            // The key is not logged — the identity derives from the key but is not a credential (logging convention 「Security」).
            // warn, not info: this line holds a fact that cannot be recreated after the event — which credential ordered a user-visible change; the result stays on screen but the owner is recorded nowhere, so raising the level later never reveals it. This route treats "an agent moved the human's screen" as an event to be seen,
            //   the same judgment as the mail-rejection `warn` in `bearer_auth`. Do not use "the default level is warn, so info is not printed" as the reason — the logging convention forbids that argument. One relay produces this one warn line; the success line on the outcome side stays `info`.
            //   Writing this exception into the logging convention doc is not done yet (open — outside this round).
            //   Arrival rate is within this gate's natural cap (connections per request · `MAX_RELAYED_IN_FLIGHT`).
            // The entrance label uses the constant: this call goes through `CommandBus::invoke`, so it must equal the label that side records; `"cli"` belongs to direct execution that does not go through relay (that constant's doc).
            // The name goes through [`sanitize_for_log`] (same reason as the mail-rejection line; the delivery side uses it too — first line of `command_delivery::deliver` — so one value is not written in two shapes).
            // The wording does not say "relayed" — this line is before the round trip: `invoke` after it may hit the concurrency cap (`MAX_RELAYED_IN_FLIGHT`) and run nothing, so "relaying" would record something that did not happen. The paired lines give the outcome
            //   (`CommandBus::invoke` one success/failure line; the cap rejection one line from `RelaySlot::admit`; all share `entrance` · `request_id`, tying one round trip).
            // Do not defer this line until after the round trip: attribution cannot be recreated once the event has passed, and a daemon dying mid-round-trip would never write it.
            tracing::warn!(
                entrance = crate::command_delivery::ENTRANCE_CONTROL,
                agent = %caller.agent_id,
                epoch = caller.epoch,
                command = %sanitize_for_log(&name),
                %request_id,
                "an agent called someone else's command — this daemon accepted it addressed to that owner"
            );
            let reply = bus.invoke(request_id, name, args).await;
            super::catalog::relayed(reply)
        }
    };
    Json(result.into_json()).into_response()
}

// ── CLI help screen entrance (/control/help) ───────────────────────────────────────────────

/// help screen — success and bounce are both 200 + JSON; only no answer at all is 500 (scope: [`internal_error`] comment). Body contract · failure policy: `control::help`.
///
/// Identity is required only to exist: the screen is the same for everyone and identity does not choose it; a call without a token must not reach here (defensive 401, as `control_agent_handler`).
/// Read inside `spawn_blocking`: the source reads the body file on every request (no cache), and reading on an async worker would block other requests there during the disk wait.
// ADR-0285
async fn control_help_handler(
    axum::extract::State(help): axum::extract::State<Arc<super::help::HelpSource>>,
    identity: Option<axum::Extension<BoundIdentity>>,
    body: axum::body::Bytes,
) -> Response {
    if identity.is_none() {
        return unauthorized();
    }
    let Ok(result) = tokio::task::spawn_blocking(move || help.answer(&body)).await else {
        tracing::error!(entrance = "cli", "help screen task failed (panic)");
        return internal_error();
    };
    Json(result.into_json()).into_response()
}

/// 500 response (empty body) — the blocking task panicked and there is no answer at all.
///
/// Unreachable in a production process (known scope): a blocking-task panic is process termination, not a `JoinError` (hook — ADR-0292), and the command execution the task drives is outside the containment marker (ADR-0293). So "panic → 500" holds only in hook-less test builds and guards nothing in production;
/// it stays because the branch is real in that build, where a request with no answer would otherwise hang.
fn internal_error() -> Response {
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .body(axum::body::Body::empty())
        .expect("valid 500 response")
}

/// 503 response (empty body) — slot not set (wiring-order fault), not a request-format or auth problem, so not 4xx.
fn service_unavailable() -> Response {
    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .body(axum::body::Body::empty())
        .expect("valid 503 response")
}

/// The variant brought up without a help source — `/control/help` answers every request with `INTERNAL` (`help source not configured on this server`) and leaves one startup warn. Args as in [`start_mcp_server_with_help`].
///
/// For tests · harnesses not calling help — if the production assembly (`lib.rs`) calls this variant, help is always a bounce; the name says "no help" so such a call stands out in review.
// ADR-0285
pub async fn start_mcp_server_without_help(
    registry: Arc<ControlRegistry>,
    manager: Arc<ManagerSlot>,
    messaging: Arc<MessagingSlot>,
    commands: Arc<CommandTableSlot>,
    bus: crate::command_delivery::CommandBus,
) -> std::io::Result<McpServerHandle> {
    start_mcp_server_with_help(
        registry,
        manager,
        messaging,
        commands,
        bus,
        super::help::HelpSource::not_configured(),
    )
    .await
}

/// The registry must be the same Arc shared by the auth middleware (verification) and provision (issuing).
///
/// Local only + DNS rebinding defense: bind is 127.0.0.1:0 (OS-assigned port); StreamableHttpServerConfig allows only local Hosts with the default allowed_hosts=[localhost,127.0.0.1,::1] (rmcp default); stateful_mode=true (default), so the session is kept by Mcp-Session-Id.
pub async fn start_mcp_server_with_help(
    registry: Arc<ControlRegistry>,
    manager: Arc<ManagerSlot>,
    messaging: Arc<MessagingSlot>,
    // The fan-out slot does not come here: only the command table reads it (`commands::make_daemon_table`); a second copy held by the server would hide an assembly passing two different slots — the tree showing the old registry forever, no error, no log.
    commands: Arc<CommandTableSlot>,
    // The command bus comes by value: a shared `Clone` handle the assembly can create before this server (`lib.rs`), so no late-injection slot. Half of discovery and the relay delivery come from here, so the assembly must pass the one copy the accept loop holds —
    //   a new one would make discovery show only the daemon's own names and the relay use its own seat table (ADR-0160).
    bus: crate::command_delivery::CommandBus,
    // The help source is built by the assembly: built inside from the install location, a test could not inject a known body and the screen would be tied to the test exe's location · `CARGO_TARGET_DIR`.
    // ADR-0285
    help: super::help::HelpSource,
) -> std::io::Result<McpServerHandle> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr: SocketAddr = listener.local_addr()?;
    let url = format!("http://127.0.0.1:{}{}", addr.port(), MCP_PATH);

    let cancel = CancellationToken::new();

    // StreamableHttpServerConfig is #[non_exhaustive], so a struct literal is impossible → Default + builder methods.
    let config =
        StreamableHttpServerConfig::default().with_cancellation_token(cancel.child_token());
    let factory_manager = manager.clone();
    let factory_registry = registry.clone();
    let factory_messaging = messaging.clone();
    let mcp_service = StreamableHttpService::new(
        move || {
            Ok(EngramMcpHandler::new(
                factory_manager.clone(),
                factory_registry.clone(),
                factory_messaging.clone(),
            ))
        },
        Arc::new(LocalSessionManager::default()),
        config,
    );

    // nest_service: StreamableHttpService is a Tower service and sits on the axum router as is.
    // Body limit = RequestBodyLimitLayer (round-2 F4): bodies on the local control channel are small; the limit is stated explicitly so malicious/runaway bodies cannot swallow memory. axum `DefaultBodyLimit` applies only to extractors (Json/Bytes),
    //   while rmcp `StreamableHttpService` consumes the raw body directly, so it does not work; `RequestBodyLimitLayer` wraps the body itself and cuts with 413 on overrun, covering rmcp too. 1MB suffices for initialize/tools/call and send POSTs.
    // Layer order: body-limit → auth → route from outer to inner (in axum the later-written layer is outer), so the limit applies before auth or inner reads the body.
    const MAX_BODY_BYTES: usize = 1024 * 1024;
    let send_state = ControlSendState {
        manager: manager.clone(),
        registry: registry.clone(),
        messaging: messaging.clone(),
    };
    let catalog_state = ControlCatalogState {
        commands: commands.clone(),
        bus,
    };
    let agent_state = ControlAgentState { commands };
    help.log_startup();
    let help = Arc::new(help);
    // Add routes by walking the list (`ControlRoute::ALL`) — do not revert to a builder chain: a new route must enter the list to be served, and then the exhaustive match in `is_mail` forces mail classification at compile time (ADR-0133); a chain bypasses it.
    let mut app = axum::Router::new().nest_service(MCP_PATH, mcp_service);
    for route in ControlRoute::ALL {
        app = match route {
            ControlRoute::Send => app.route(
                route.path(),
                axum::routing::post(control_send_handler).with_state(send_state.clone()),
            ),
            ControlRoute::Messages => app.route(
                route.path(),
                axum::routing::post(control_messages_handler).with_state(send_state.clone()),
            ),
            ControlRoute::Agent => app.route(
                route.path(),
                axum::routing::post(control_agent_handler).with_state(agent_state.clone()),
            ),
            ControlRoute::Commands => app.route(
                route.path(),
                axum::routing::post(control_commands_handler).with_state(catalog_state.clone()),
            ),
            ControlRoute::Call => app.route(
                route.path(),
                axum::routing::post(control_call_handler).with_state(catalog_state.clone()),
            ),
            ControlRoute::Help => app.route(
                route.path(),
                axum::routing::post(control_help_handler).with_state(help.clone()),
            ),
        };
    }
    let app = app
        .layer(axum::middleware::from_fn_with_state(
            registry.clone(),
            bearer_auth,
        ))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            MAX_BODY_BYTES,
        ));

    let serve_cancel = cancel.clone();
    let serve_handle = tokio::spawn(async move {
        let server = axum::serve(listener, app.into_make_service());
        let graceful = server.with_graceful_shutdown(async move {
            serve_cancel.cancelled().await;
        });
        if let Err(e) = graceful.await {
            tracing::warn!("MCP axum serve ended: {e}");
        }
    });

    tracing::info!(
        port = addr.port(),
        path = MCP_PATH,
        "MCP server started (ADR-0086)"
    );
    Ok(McpServerHandle {
        url,
        cancel,
        serve_handle: Some(serve_handle),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ADR-0133: route classification — a new route does not silently leak as "not mail" ──────────────────

    /// This function is the body of this test: adding a variant breaks compilation here, forcing the classification to be written by hand. It is stated separately from `ControlRoute::is_mail` and does not call it — if the two diverge the assertions go red.
    fn expected_is_mail(route: ControlRoute) -> bool {
        match route {
            ControlRoute::Send => true,
            ControlRoute::Messages => true,
            ControlRoute::Agent => false,
            ControlRoute::Commands => false,
            ControlRoute::Call => false,
            ControlRoute::Help => false,
        }
    }

    #[test]
    fn every_registered_control_route_is_explicitly_classified() {
        for route in ControlRoute::ALL {
            assert_eq!(
                route.is_mail(),
                expected_is_mail(route),
                "the mail classification of {route:?}({}) diverged",
                route.path()
            );
            assert_eq!(
                ControlRoute::from_path(route.path()),
                Some(route),
                "if it is on the list, it must be recoverable by path: {route:?}"
            );
        }
        // The router is assembled by walking this list — if the length shrinks, a route has silently disappeared.
        assert_eq!(ControlRoute::ALL.len(), 6);
        assert_eq!(
            ControlRoute::ALL.iter().filter(|r| r.is_mail()).count(),
            2,
            "mail routes are two, send and query — this number does not grow even if the control plane grows"
        );
    }

    /// `/mcp` and paths outside control are not gate targets (MCP is that credential's own channel).
    #[test]
    fn the_mcp_route_is_outside_the_mail_gate() {
        for path in [MCP_PATH, "/mcp/", "/mcp/whatever", "/", "/health"] {
            assert!(
                !mail_gated_path(path),
                "a mail gate must not apply to a path outside the control plane: {path}"
            );
        }
    }

    /// A control path with a missing classification folds toward rejection: reversed, forgetting to register a new route would open a mail entrance with no signal.
    #[test]
    fn an_unclassified_control_path_folds_to_the_mail_gate() {
        for path in [
            "/control/whatever",
            "/control/send/extra",
            "/control",
            "/control/",
        ] {
            assert_eq!(
                ControlRoute::from_path(path),
                None,
                "premise: this assertion means something only for a path outside the list ({path})"
            );
            assert!(
                mail_gated_path(path),
                "a control path outside the list must fold into mail (fail-closed): {path}"
            );
        }
        // A route in the list follows its own classification as is (the folding does not overwrite the classification).
        for open in [
            CONTROL_AGENT_PATH,
            CONTROL_COMMANDS_PATH,
            CONTROL_CALL_PATH,
            CONTROL_HELP_PATH,
        ] {
            assert!(
                !mail_gated_path(open),
                "the control plane is open to everyone: {open}"
            );
        }
        for route in ControlRoute::ALL {
            assert_eq!(mail_gated_path(route.path()), route.is_mail(), "{route:?}");
        }
        // Segment boundary: folding merely similar paths would reject a future route there for no reason (the failure mode of a bare string-prefix comparison).
        for path in [
            "/controlfoo",
            "/control-x",
            "/controls/send",
            "/CONTROL/send",
        ] {
            assert!(
                !mail_gated_path(path),
                "a name outside the control namespace must not be folded: {path}"
            );
        }
    }

    /// The rejection response does not announce an alternative channel (ADR-0133 decision 3): doing so would let the mutual-exclusion seal of the two priming files be bypassed through this path.
    ///
    /// Read the body from the real constructor (`mail_not_allowed`), not a reassembled copy: a copy would stay green if the constructor added a field or changed the wording.
    #[tokio::test]
    async fn the_mail_rejection_names_no_other_channel() {
        let resp = mail_not_allowed();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "follows the mail route's 'always 200 + JSON' contract (the CLI decides by exit code)"
        );
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .expect("refusal body");
        let body = String::from_utf8(bytes.to_vec()).expect("UTF-8 body");
        let lowered = body.to_lowercase();
        for forbidden in [SEND_MESSAGE_TOOL, MESSAGES_TOOL, "mcp", "tool", "server"] {
            assert!(
                !lowered.contains(&forbidden.to_lowercase()),
                "the refusal response must not point at the other channel ({forbidden}): {body}"
            );
        }
        // The bounce shape folds as is into the CLI's existing three-way split (no new class of outcome is created).
        let v: serde_json::Value = serde_json::from_str(&body).expect("JSON");
        assert_eq!(v["status"], "error");
        assert!(v["code"].as_str().is_some_and(|c| !c.is_empty()));
        assert!(v["hint"].as_str().is_some_and(|h| !h.is_empty()));
    }

    // ── Log forgery prevention — a caller-chosen string does not go into a log field verbatim ──────────────

    use crate::log_capture::capture_loud_async as capture_loud;

    /// A caller string going verbatim into a log line printed before validation becomes log forgery.
    ///
    /// This route's mail rejection leaves a `warn` having verified nothing, so the name is arbitrary; one newline forges a second entry and, the default level being warn, it really lands in the production log file.
    /// Measured is "did the verbatim text reach that field", not "was the function called": it runs the real handler and reads the log event's field text; reverting to a length-only function (`agent::preview`) turns this red.
    #[test]
    fn a_control_character_in_the_called_name_never_reaches_the_log_field_verbatim() {
        // A newline + a forged entry + a tail tag that must be pushed out of the width (two properties with one value).
        let forged = "mail.probe\n2026-08-19T00:00:00Z  WARN forged entry padded out past the field width TAILMARK";
        let lines = capture_loud(async move {
            let (bus, sweeper) = crate::command_delivery::CommandBus::without_commands();
            let state = ControlCatalogState {
                commands: empty_commands_slot(),
                bus,
            };
            let binding = axum::Extension(crate::control::registry::TokenBinding {
                identity: BoundIdentity {
                    agent_id: engram_dashboard_agent::types::AgentId::new_v4(),
                    epoch: 0,
                },
                mail_allowed: false,
            });
            let body = serde_json::json!({ "name": forged }).to_string();
            let resp = control_call_handler(
                axum::extract::State(state),
                Some(binding),
                axum::body::Bytes::from(body),
            )
            .await;
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "a mail refusal is 200 + a bounce envelope"
            );
            sweeper.stop().await;
        });

        let logged = lines
            .iter()
            .find(|line| line.contains("command="))
            .unwrap_or_else(|| {
                panic!("the refusal line must remain (instrumentation duty): {lines:?}")
            });
        assert!(
            !logged.contains('\n'),
            "if a log field can split the line, forged entries appear: {logged:?}"
        );
        assert!(
            logged.contains("\\n"),
            "control characters must remain in escaped form (not erasing the value): {logged:?}"
        );
        assert!(
            !logged.contains("TAILMARK"),
            "the width of one log line must not be decided by the other party (width cap): {logged:?}"
        );
    }

    #[test]
    fn ping_args_schema_builds() {
        let schema = schemars::schema_for!(PingArgs);
        let _ = serde_json::to_string(&schema).expect("serialize schema");
    }

    /// Empty manager slot (for server-lifetime tests that do not call send — they verify only start/drop).
    fn empty_slot() -> Arc<ManagerSlot> {
        Arc::new(ManagerSlot::new())
    }

    /// Empty messaging slot (same as above — for start/drop tests, send not called).
    fn empty_messaging_slot() -> Arc<MessagingSlot> {
        Arc::new(MessagingSlot::new())
    }

    /// Empty command-table slot — this file's tests do not call control verbs (if they did, that route would be 503).
    fn empty_commands_slot() -> Arc<CommandTableSlot> {
        Arc::new(CommandTableSlot::new())
    }

    #[test]
    fn send_args_schema_builds() {
        let schema = schemars::schema_for!(SendArgs);
        let s = serde_json::to_string(&schema).expect("serialize schema");
        assert!(
            s.contains("\"to\"") && s.contains("\"body\""),
            "schema has to/body: {s}"
        );
        assert!(
            !s.contains("\"from\""),
            "the eg_send schema must not have a from field: {s}"
        );
    }

    #[test]
    fn to_field_tokenizes_per_entrance_without_double_splitting() {
        let owned = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<String>>();
        // ── MCP entrance ──
        assert_eq!(
            ToField::One("a,b".to_string()).into_tokens(),
            owned(&["a,b"])
        );
        assert_eq!(
            ToField::Many(owned(&["a,b", "@all"])).into_tokens(),
            owned(&["a,b", "@all"])
        );
        // ── CLI entrance ──
        assert_eq!(
            cli_recipient_tokens(ToField::One("a,b".to_string())),
            owned(&["a", "b"])
        );
        assert_eq!(
            cli_recipient_tokens(ToField::Many(owned(&["a,b", "@all"]))),
            owned(&["a", "b", "@all"])
        );
        assert_ne!(
            ToField::One("a,b".to_string()).into_tokens(),
            cli_recipient_tokens(ToField::One("a,b".to_string()))
        );
    }

    #[test]
    fn to_field_deserializes_both_string_and_array_shapes() {
        let one: ToField = serde_json::from_str(r#""bob""#).expect("string form");
        assert_eq!(one.into_tokens(), vec!["bob".to_string()]);
        let many: ToField = serde_json::from_str(r#"["bob","@all"]"#).expect("array form");
        assert_eq!(
            many.into_tokens(),
            vec!["bob".to_string(), "@all".to_string()]
        );
    }

    #[test]
    fn tools_list_exposes_send_message_tool() {
        let router = EngramMcpHandler::tool_router();
        assert!(
            router.has_route(SEND_MESSAGE_TOOL),
            "the '{SEND_MESSAGE_TOOL}' tool must be registered in the router (enforces const ↔ #[tool] method name match)"
        );
    }

    /// The pin moved over (front half of `control/priming.rs::production_priming_files_pin_taught_channels`): it checked that the priming file carries `send_message`, then the only teaching surface for MCP-capable spawns; as priming shrank to a one-line pointer
    /// that surface moved to this description (ADR-0126 decision 1, an entrance teaches only one), and the pin came along.
    ///
    /// The name alone is not enough: `tools_list_exposes_send_message_tool` checks only the routing name and stays green with an empty description, leaving the agent with a name and JSON schema and nowhere to learn how to call it — this is the one place that catches that silence.
    /// The tool-argument spelling of the reply contract (snake_case) is checked here too (ADR-0103 decision 2/3): envelope recognition (including `<notice>`) belongs to the `engram help mail` screen (`control/help.rs`), this side owns only its own entrance's spelling (one contract per surface, ADR-0126 decision 1).
    /// Do not delete the two assertions below — they existed only in the old instructions and were revived here:
    ///   1. do not infer the other party's state from `pending`; query with `eg_messages` (ADR-0211 decision 2; an explanation pinning the cause on a single one violates that prohibition).
    ///   2. tell it apart from the similarly named harness built-in `SendMessage` (ADR-0106 — deny is blocked structurally, but an agent not knowing a misfire ends in a permission denial would report on its own judgment that the mail channel is broken). The old reason "the channel-failure report the instructions ordered" is
    ///      no longer true (those four behavior-rule lines were withdrawn from priming — `control/priming.rs` comment); this description is now the only guidance surface.
    // ADR-0103
    // ADR-0106
    // ADR-0126
    // ADR-0128
    // ADR-0211
    #[test]
    fn the_send_message_entry_teaches_its_own_call() {
        let tools = EngramMcpHandler::tool_router().list_all();
        let tool = tools
            .iter()
            .find(|t| t.name == SEND_MESSAGE_TOOL)
            .unwrap_or_else(|| panic!("tools/list must have '{SEND_MESSAGE_TOOL}'"));
        let desc = tool.description.as_deref().unwrap_or_else(|| {
            panic!("'{SEND_MESSAGE_TOOL}' must have a description (this is the teaching surface)")
        });
        // The home of the argument contract moved (2026-09-23): the description no longer re-explains per field (a duplicate of the schema's per-field descriptions, which are more detailed and sit where the model fills in arguments), so this pin looks at that home too; with an empty schema the agent receives only field names.
        // Check that meaning was carried, not field names: schemars prints field names into `properties` even with all doc comments erased, so names alone cannot catch that failure (caught by an adversarial review — 2026-09-23).
        fn carries_description(v: &serde_json::Value) -> bool {
            match v {
                serde_json::Value::Object(m) => {
                    m.get("description")
                        .and_then(|d| d.as_str())
                        .is_some_and(|s| !s.trim().is_empty())
                        || m.values().any(carries_description)
                }
                serde_json::Value::Array(a) => a.iter().any(carries_description),
                _ => false,
            }
        }
        let props = tool
            .input_schema
            .get("properties")
            .and_then(|v| v.as_object())
            .expect("the input schema must have properties");
        for arg in ["to", "body", "reply_to", "reply_by"] {
            let slot = props.get(arg).unwrap_or_else(|| {
                panic!("the input schema must carry the {arg} field: {props:?}")
            });
            assert!(
                carries_description(slot),
                "the {arg} field must carry a description (the home of the argument contract is here — the schema carries what was stripped from the description): {slot}"
            );
        }
        // The ① above (do not infer the other party's state from `pending`; query) moved to the `help mail` screen — ADR-0211 decision 2 still holds, only the surface moved; its pin is `the_mail_screen_teaches_the_reply_contract` in `control/help.rs`.
        //   Here we check only that it points at that screen — if the pointer breaks, the whole out-of-argument contract loses its place to go.
        assert!(
            desc.contains("engram help mail"),
            "the description must point to where the remaining rules are read: {desc}"
        );
        assert!(
            desc.contains("SendMessage"),
            "it must teach itself apart from the similarly named harness built-in tool (if it does not know a misfire ends in a permission refusal, \
             a false 'channel broken' report goes up — ADR-0106): {desc}"
        );
    }

    #[test]
    fn tools_list_exposes_the_query_tools() {
        // If the const ↔ #[tool] method name drift, the tool name priming teaches and the exposed name diverge and the query entrance silently ceases to exist.
        let router = EngramMcpHandler::tool_router();
        assert!(
            router.has_route(MESSAGES_TOOL),
            "'{MESSAGES_TOOL}' tool registered"
        );
    }

    #[test]
    fn query_tool_schemas_build_and_carry_no_identity_field() {
        let m =
            serde_json::to_string(&schemars::schema_for!(MessagesArgs)).expect("messages schema");
        assert!(m.contains("\"id\""), "messages schema has id: {m}");
        assert!(
            !m.contains("\"from\"") && !m.contains("required"),
            "messages arguments are all optional + no from: {m}"
        );
    }

    #[tokio::test]
    async fn server_starts_and_reports_local_url() {
        let reg = Arc::new(ControlRegistry::new());
        // The sweeper must be held — if dropped, the seat table closes at once and every round trip after is bounced.
        let (relay_bus, _relay_sweeper) = crate::command_delivery::CommandBus::without_commands();
        let handle = start_mcp_server_without_help(
            reg,
            empty_slot(),
            empty_messaging_slot(),
            empty_commands_slot(),
            relay_bus,
        )
        .await
        .expect("start mcp server");
        assert!(
            handle.url.starts_with("http://127.0.0.1:") && handle.url.ends_with("/mcp"),
            "local endpoint URL: {}",
            handle.url
        );
        handle.shutdown().await;
    }

    // ── round-2 F5 ──────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn dropping_handle_cancels_serve_task() {
        let reg = Arc::new(ControlRegistry::new());
        // The sweeper must be held — if dropped, the seat table closes at once and every round trip after is bounced.
        let (relay_bus, _relay_sweeper) = crate::command_delivery::CommandBus::without_commands();
        let handle = start_mcp_server_without_help(
            reg,
            empty_slot(),
            empty_messaging_slot(),
            empty_commands_slot(),
            relay_bus,
        )
        .await
        .expect("start mcp server");
        let watch = handle.cancel.clone();
        assert!(!watch.is_cancelled(), "not cancelled right after start");
        drop(handle);
        assert!(
            watch.is_cancelled(),
            "on handle drop the cancel token must fire and the detached serve task must end (F5)"
        );
    }
}
