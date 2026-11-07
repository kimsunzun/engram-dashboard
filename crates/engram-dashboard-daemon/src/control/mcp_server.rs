//! Daemon MCP Streamable HTTP server — the entrance of the control channel that spawned claude agents attach to via mcp-config.
//!
//! ★Owns★: nests the rmcp `StreamableHttpService` (a Tower service) at axum `/mcp` and puts bearer auth
//!   middleware in front of it. The CLI plain-text routes ride the same server, port and middleware — the **canonical
//!   list of those is `ControlRoute`** and it is not recounted here (a list kept here goes stale when a route is added).
//!   Tool surface = `engram_ping` · `eg_send` · `eg_messages`.
//!   ★Control verbs are not exposed as MCP tools★ (ADR-0132 decision = CLI) — tool schemas are permanently
//!   resident in every agent context, while control is low-frequency. So `/control/agent` has no matching `#[tool]`.
//!
//! ★OAuth metadata is not advertised (load-bearing, #59467)★: StreamableHttpService does not create
//!   `.well-known/*` routes, and we do not add any. When a server advertises OAuth metadata, claude ignores the static Authorization
//!   header (claude-code #59467); with no advertising route, the static Bearer is sent as is (ADR-0086 §rationale).
//!
//! tauri import 0 (daemon crate).

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
// ★Caller strings that go into log fields pass through this function — not the neighboring `agent::preview`★:
//   the canonical account of that split is the [`super::agent::preview`] doc (「로그에 쓰지 말 것」, do not use in logs) and the [`sanitize_for_log`] doc.
use crate::connection_core::sanitize_for_log;

/// The axum path the MCP server is mounted at. The mcp-config url also points at this path (`http://127.0.0.1:<port>/mcp`).
///
/// ★keep-in-sync(M5)★: `ControlEndpoint.url` is the MCP route with this path attached. When CLI entrance injection derives the CLI
///   base URL (ENGRAM_CONTROL_URL), it **strips this literal suffix ("/mcp") as a string** —
///   `crates/engram-dashboard-agent/src/backend/mod.rs` (strip_suffix("/mcp") in `inject_cli_entrance`).
///   If this value changes, the strip literal there must change too — the build does not enforce it, and if they drift the base derivation
///   goes wrong and the CLI silently gets a 404.
///   ★That place is now **shared by all backends**★ — on a mismatch the CLI of codex spawns dies too, not only claude's.
const MCP_PATH: &str = "/mcp";

/// CLI entrance (ADR-0086 step 2) — the plain JSON route that `engram mail send` POSTs to. The CLI assembles this path onto the base URL
/// (ENGRAM_CONTROL_URL).
const CONTROL_SEND_PATH: &str = "/control/send";

/// CLI query entrance (D · canonical surface = ADR-0132) — the route that `engram mail status <id>` / `engram mail pending`
/// POST to.
/// ★Why POST (not GET)★: it must go through the same bearer middleware, and the middleware cuts a **GET without a session id with 400**
///   (session operation rule — bearer_auth step 1.5). The query uses no session, so it takes the same session-less
///   POST form as send (auth rules are not split per path, which would create two disciplines).
const CONTROL_MESSAGES_PATH: &str = "/control/messages";

/// CLI control entrance (ADR-0132 decision 6) — the route that `engram agent <동사>` POSTs to. The verb comes in the
/// **body** (`{verb: …}`), not the path: the CLI's HTTP client is a single POST hand-assembled with std only, so adding a
/// path per verb would make that client carry a path table, and the route constants would grow with the verb count.
/// ★Auth is the same middleware as `/control/send`★ — it is a session-less POST, so the discipline in the `CONTROL_MESSAGES_PATH` comment above
///   applies as is (auth rules are not split per path, which would create two disciplines).
/// ★Control is open to everyone (ADR-0132 decision 5)★ — a spawned agent reaches this route regardless of backend.
///   Only mail is split by credential (`ControlRoute::is_mail`).
const CONTROL_AGENT_PATH: &str = "/control/agent";

/// CLI discovery entrance — gives the daemon's own table and the attached clients' registry as **one list** (`control::catalog`).
/// ★Read-only★ — it changes no state.
/// ★Why POST (not GET)★: exactly the discipline in the `CONTROL_MESSAGES_PATH` comment above (the middleware cuts a session-less GET
///   with 400). Auth rules are not split per path, which would create two disciplines.
/// ★Credentials with mail blocked can reach it★ — discovery is the control plane, and even a backend that cannot write letters needs to know
///   what it can call (`ControlRoute::is_mail`).
/// ★Why `pub(super)`★: the recovery hint in the neighboring `catalog` puts this path in its wording, and its test runs this constant through
///   a comparison — it is the only way to stop the wording from pointing at an address that does not exist.
pub(super) const CONTROL_COMMANDS_PATH: &str = "/control/commands";

/// CLI generic call entrance — calls a command by its **full name** (`{name, args}`).
///
/// ★Not an alias of `/control/agent`★: that one carries the name of one family and bears that family's vocabulary (`{verb}`) and
///   per-family recovery hints. If one address had two meanings, the closed route enumeration would lose its power to force classification.
/// ★The check is **the same function** as the sibling's★ (`commands::call_daemon_command`) — not building a second entrance
///   without the check was the only hard constraint kept when adding this route.
const CONTROL_CALL_PATH: &str = "/control/call";

/// CLI help screen entrance — `engram help [낱말]` POSTs one screen word and the daemon produces the screen
/// (`control::help`). ★The path is not written out again here★ — the CLI and this server see one agent constant (the
/// five siblings are still matched by hand).
/// ★Credentials with mail blocked can reach it★ — reading a mail screen is not mail, and the screen is the same for everyone
///   (`ControlRoute::is_mail`).
// ADR-0285
const CONTROL_HELP_PATH: &str = engram_dashboard_agent::types::CLI_HELP_ROUTE;

/// Namespace prefix of control-plane paths — **the basis for folding a path with no classification into fail-closed**.
///
/// ★Compare on a **path segment boundary** (a bare `starts_with` is forbidden)★: a string-prefix check alone would also fold names unrelated to this namespace such as `/controlfoo` and
///   `/control-x`, and a future route at that spot would be rejected for no reason. The targets are only `/control` itself and
///   everything under `/control/`.
// ADR-0133
const CONTROL_PATH_PREFIX: &str = "/control";

/// All the CLI plain-text routes the daemon opens (the `/mcp` nest is owned by rmcp, so it is not here).
///
/// ★The **single list** for router assembly and mail classification★: `start_mcp_server_with_help` walks this list and adds the routes.
///   A route that enters the list is forced to be classified **at compile time** by the exhaustive match in `is_mail` below.
/// ★A control path outside the list is folded into mail (`mail_gated_path`)★ — the compiler cannot stop the mistake of hand-adding a `.route()`
///   after the iteration, but for a path created that way **a missing classification means rejection**, so it is not
///   silently opened. So keeping the `.route()` call site to this one iteration is still the rule, but when that rule is broken,
///   the default is the safe side.
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

    /// Whether this route is **mail** — the axis the mail allow/deny rejection (ADR-0133 decision 3) hangs on.
    ///
    /// ★Do not add a `_` arm★: the moment a catch-all exists, a new route is silently classified as "not mail"
    /// (= everyone passes). That mistake gives no signal at runtime.
    ///
    /// ★What this exhaustive match forces is **routes** only★ — `/control/call` selects a command by the **name in the
    ///   body**, not by the path, so this axis cannot close that one. What closes that axis is [`mail_by_name`], and
    ///   the call route invokes it in its own place.
    ///   ★Do not revert on the grounds 「오늘 데몬 표가 `agent.*` 만 쥐고 있어 도달 불가」 ("today the daemon table holds only agent.*, so it is unreachable") — that sentence is now
    ///   false★: the reachable set is the daemon table **∪ the registry** (ADR-0160), and the registry's names are set by the dashboard
    ///   (`src-tauri/src/layout/commands.rs`). The person who names those will never read this comment, so it is not an
    ///   axis that can be left to prose.
    // ADR-0133
    // ADR-0160
    const fn is_mail(self) -> bool {
        match self {
            Self::Send | Self::Messages => true,
            // Control plane — discovery and calls are not mail. Even a credential that cannot write letters needs to know what it can
            //   call, and must be able to call it (ADR-0132 decision 5 — control is open to everyone).
            Self::Agent | Self::Commands | Self::Call => false,
            // Reading a screen is not mail — even a credential with mail blocked must read the mail screen to learn that channel's
            //   contract (ADR-0220 decision 4 — the screen is the same for everyone).
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
/// ★A missing classification folds into **rejection** (fail-closed)★: a route in the list follows its own classification, and a control path
///   **outside** the list (`/control…`) is treated as mail. The very purpose of this gate is rejection, so when someone adds a route and
///   forgets the classification, the default must be "blocked" — the other way round, that mistake would open an
///   entrance at runtime with no signal at all.
/// ★`/mcp` and paths outside control are not targets★: MCP is that credential's **own channel** (ADR-0128 decision 1),
///   and the rest are paths this server does not open in the first place.
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

/// The bounce code for when this credential **cannot use this HTTP/CLI mail entrance** (ADR-0133 decision 3).
///
/// ★Read the scope the name claims narrowly — it is not "mail forbidden"★: a spawn that gets this code (= an MCP-capable
///   backend) **writes mail normally** through `eg_send` on `/mcp`. The design is that the channel is split only by backend capability
///   with no runtime switching (ADR-0128 decision 1), and this gate closes **the entrance that must stay
///   closed** in that design. Do not read "can bypass via MCP" as "a hole in the gate" from the name alone.
const MAIL_NOT_ALLOWED_CODE: &str = "MAIL_NOT_ALLOWED";

/// ★Do not announce an alternative channel (ADR-0133 decision 3)★: the moment this says "use this instead", the gate that the two priming files
///   sealed so they never even mention each other's channel (the real gate of ADR-0128) is bypassed through this response path.
///   So the wording goes only as far as "this credential cannot write mail" and carries no name of the other channel or tool.
///   It tells only that retrying is pointless, which prevents infinite retries.
const MAIL_NOT_ALLOWED_HINT: &str =
    "This credential is not allowed to use mail. Retrying will not change that.";

/// Whether this **command name** is mail — the mail axis for a route that selects a command by body, not path (`/control/call`).
///
/// ★Why close it with one name★: the set that route can reach is the daemon's own table **∪ the registry** (ADR-0160), and
/// the registry-side names are set outside this crate (the dashboard adds its own commands). So this axis includes names that **cannot be known even by
/// reading all the code here**, and 「오늘은 그런 이름이 없다」 ("there is no such name today") cannot be the grounds.
/// ★The family separator belongs to the command-name convention (`<계열>.<동사>`)★ — if the family is `mail`, it is mail. Do not
/// use a prefix comparison: `mailbox.x` is not in the mail family but a prefix check catches it. The canonical source of that convention is the command table, and the reason this string
/// was not lifted into a shared constant is that the CLI also carries its own (`engram-dashboard-cli/src/bin/engram.rs`) — before
/// making a third copy, where to put that constant must be decided first.
/// ★Today no name makes this decision true (measured)★ — the daemon table has only `agent.*`, and the names the dashboard adds
/// have no `mail.*` either. So this is not plugging a hole but a **guardrail**: whichever side `mail.*` comes in from,
/// this line is already standing that day.
// ADR-0133
// ADR-0160
fn mail_by_name(name: &str) -> bool {
    name.split_once('.')
        .is_some_and(|(group, _)| group == engram_dashboard_agent::types::CLI_GROUP_MAIL)
}

/// ★Late-injection slot for the manager (breaking the cycle)★: daemon startup brings the MCP server up **first** (because it must create the DaemonControlChannel that
/// issues mcp-config with that URL) and wires the AgentManager afterwards — so at server start
/// the manager does not exist yet. So the server is handed an empty slot, filled with `set` right after the manager is assembled.
/// Request handling (send) runs after the accept loop, so it is always filled by then (set completes before any agent attaches).
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

/// ★Late-injection slot for MessagingService (breaking the cycle — C1)★: same shape as ManagerSlot. MessagingService
/// wraps AgentManager (DeliveryPort), so it can only be created **after** the manager is assembled.
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
    /// It also calls the flush lane of messaging_host (`run_flush_lane`), so it is exposed inside the crate.
    pub(crate) fn get(
        &self,
    ) -> Option<&Arc<engram_dashboard_messaging::service::MessagingService>> {
        self.inner.get()
    }
}

/// ★Late-injection slot for the registry-notification fan-out (ADR-0132)★: same shape as ManagerSlot. The fan-out is
/// derived from the connection registry, and that registry is assembled **after** the MCP server (lib.rs `run()` 5c → 6).
///
/// ★In some assemblies it is normal for this to be empty★: smoke bins and isolation harnesses have no clients to attach. If it is
///   empty in a production daemon, a registry changed by a control verb is not reflected on screen (see `agent::RosterBroadcast`).
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
    /// The command table's notification adapter reads it too — that side looks at it **on every call** (`commands` module).
    pub(crate) fn get(&self) -> Option<&Arc<dyn super::agent::RosterBroadcast>> {
        self.inner.get()
    }
}

/// ★Late-injection slot for the daemon command table (ADR-0155)★: same shape as ManagerSlot. The table holds the manager, so it is only created
/// **after** the manager is assembled, yet the slot that will hold that manager must exist before the MCP server.
///
/// ★Even if the table comes late, registry notification is not late★ — what the table holds is not the fan-out value but the slot above
/// (`commands::make_daemon_table`).
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
    /// The `/control/agent` adapter reads it on every request — if empty, that route returns 503 (it is a wiring-order fault, not
    ///   a request-format or auth problem, so it is not 4xx).
    pub(crate) fn get(&self) -> Option<&Arc<super::commands::DaemonTable>> {
        self.inner.get()
    }
}

/// MCP session identification header name (rmcp/spec standard, lowercase).
const SESSION_ID_HEADER: &str = "mcp-session-id";

/// ★The `eg_send` MCP tool name = **single source (ADR-0094)**★. The `#[tool]` method name below is the tool name rmcp
///   exposes in `tools/list`, and this const is the **canonical source** of that name — the ADR-0094 send-permission
///   grant uses it as the tool when building the `mcp__{server}__{tool}` pattern (DaemonControlChannel.provision).
///   Knowledge of claude grammar (`mcp__..`) lives only in backend/claude/ — this const provides just the name (ADR-0004/0094).
pub const SEND_MESSAGE_TOOL: &str = "eg_send";

/// ★The `eg_messages` MCP tool name (D · spec §6)★ — same discipline as `SEND_MESSAGE_TOOL`.
///   ★Not a grant target (intentional)★: the ADR-0094 decision is that pre-authorization covers **only the send entrance**
///   (control/mod.rs `build_grants`), so the query tool is not put in the grant list.
pub const MESSAGES_TOOL: &str = "eg_messages";

pub struct McpServerHandle {
    /// Endpoint URL to embed in mcp-config (e.g. `http://127.0.0.1:54321/mcp`).
    pub url: String,
    /// Shutdown signal — cancelling it cleans up the accept loop + active sessions.
    cancel: CancellationToken,
    /// The axum::serve task handle (awaited by join on shutdown — prevents test leaks). ★Option★: in a type with Drop,
    ///   it is taken out with take so `shutdown(self)` can move the handle and await it (a type with the Drop trait cannot
    ///   have a field partially moved out — round-2 F5).
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
    /// ★drop-on-error airtight (round-2 F5)★: if the handle is simply dropped without `shutdown().await` (e.g. a **later** startup step such as the
    ///   daemon.json write fails after the MCP server starts, and an error return drops this handle),
    ///   the detached serve task may not receive the cancel signal and keep running. Firing the cancel
    ///   token in Drop makes sure the serve task (graceful_shutdown observes the cancel) exits
    ///   however the drop happens. On the normal shutdown path `shutdown()` has already cancelled, so the cancel here
    ///   is an idempotent no-op.
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// ★Identity verification = middleware (FIX 12)★: the auth middleware verifies identity and plants it in the request extensions (BoundIdentity),
///   and session↔identity binding, pinning and cleanup are all done by the middleware (registry access via State) — the handler only reads
///   the identity from extensions.
#[derive(Clone)]
pub struct EngramMcpHandler {
    tool_router: ToolRouter<Self>,
    manager: Arc<ManagerSlot>,
    /// The **same Arc** shared with the middleware — a second registry is not created.
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

    /// When a connected agent calls this tool, it proves that the session binding works end to end (acceptance observation point).
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

    /// ★★This method name (`eg_send`) must equal the `SEND_MESSAGE_TOOL` constant (ADR-0094 single
    ///   source)★★. The rmcp `#[tool]` macro uses the **method name** as is as the `tools/list` tool name (2.2.0
    ///   tool.rs: `name = attribute.name.unwrap_or_else(|| fn_ident.to_string())`, and the macro attr accepts only a String
    ///   **literal**, so a const cannot be injected — verified), so there is no way to attach the const that is the canonical name here at compile time.
    ///   The runtime test `tools_list_exposes_send_message_tool` ties the two places together.
    ///   ⚠️ To rename this method, change the `SEND_MESSAGE_TOOL` const too — otherwise the grant points at a tool
    ///   that does not exist, the send entrance is silently blocked, and only the test catches it.
    // ADR-0086 / ADR-0094 (single-source coupling)
    #[tool(
        description = "팀원 에이전트에게 메시지를 보내고, 받은 요청에 답한다. 이것은 `engram` 중개의 \
        도구이고 하네스에 내장된 `SendMessage` 가 아니다 — 그쪽은 이 팀에서 막혀 있어 부르면 권한 \
        거절로 끝난다.\n\
        인자는 각 필드 설명이 가지고, 결과를 읽는 법과 그 밖의 규칙은 `engram help mail` 이 가진다."
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
            // ★F6 instrumentation (error, M4)★: a branch absent from the normal flow — the manager slot is not filled yet (wiring-order fault).
            //   There is no safe fallback (the message cannot be delivered) → error, not warn (logging-conventions: error =
            //   "a human must see this" = a wiring defect). It signals a defect that requires fixing the daemon wiring order.
            tracing::error!(
                entrance = "mcp",
                "제어 채널 send 불가 — manager 슬롯 미설정(배선 순서 이상, ADR-0086 F6)"
            );
            return Err(ErrorData::internal_error(
                "control channel not ready (manager not wired)",
                None,
            ));
        };
        let Some(messaging) = self.messaging.get() else {
            tracing::error!(
                entrance = "mcp",
                "제어 채널 send 불가 — messaging 슬롯 미설정(배선 순서 이상, C1)"
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
        // ★Blocking boundary (C4 review fix D · load-bearing)★: `inject` inside `handle_send` is a **blocking write**
        //   to the child's stdin. For a group broadcast, one request bears that write **serially** once per member, so calling it directly from this
        //   async handler lets one blocked pipe hold a whole tokio worker thread and cause head-of-line blocking for **other requests**
        //   stacked on that thread (runtime workers number only as many as cores). So it is moved to the blocking
        //   pool — a single send does the same write, so it gets the same treatment (paths are not split, which would
        //   create two disciplines).
        //   ★Same discipline as the flush lane★: taking the batch write off the request-handling thread (service.rs FlushTrigger).
        //
        // ★The price = **at-least-once delivery** (round-3 fix 7 · intended design, needs documenting)★: a `spawn_blocking`
        //   closure **cannot be aborted**. If the caller cancels the request midway (HTTP connection drop · MCP client exit ·
        //   timeout), this `.await` disappears but **the blocking task runs to the end** — so delivery and the ledger commit
        //   still happen and **only the response** is lost. If the sender retries, the daemon sees it as a new send with a new `msg_id`
        //   (there is no idempotency key), so the same content is delivered/broadcast once more.
        //   ★Why leave it this way★: the opposite choice (cancel delivery on cancel) is impossible — bytes already written to the child's stdin
        //   cannot be taken back, and "check for cancellation before writing" is yet another TOCTOU. So we choose **duplicate > loss**:
        //   a duplicate is a visible fact the receiving LLM can read and judge, while a loss is a silent failure nobody knows about
        //   (the same direction as ADR-0103 "no silent loss").
        //   ★Future extension point★: accept a sender-chosen **idempotency key** (the same value on retry) and fold duplicate sends in the ledger.
        //   For now `msg_id` is daemon-generated, so it cannot play that role (a new value on every retry).
        let (manager, registry, messaging) =
            (manager.clone(), self.registry.clone(), messaging.clone());
        let result = tokio::task::spawn_blocking(move || {
            handle_send(&manager, &registry, &messaging, Entrance::Mcp, cmd)
        })
        .await
        .map_err(|e| {
            tracing::error!(entrance = "mcp", "제어 채널 send 태스크 실패(패닉): {e}");
            ErrorData::internal_error("send task failed", None)
        })?;
        let json = serde_json::to_string(&result.to_json()).unwrap_or_default();
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    #[tool(
        description = "Engram 중개에 메시지 상태를 묻는다. 행을 읽는 법은 `engram help mail` 이 가진다."
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
                "messages 조회 불가 — manager/messaging 슬롯 미설정(배선 순서 이상)"
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

/// Bearer auth middleware (ADR-0086) — verifies the token **before** the MCP handshake and pins session↔identity.
///
/// Check order:
///   1. `Bearer <token>` in Authorization → registry.validate. On failure 401 (inner not called = no handshake created).
///   1.2. Mail allow/deny (ADR-0133 decision 3) — if the request **path** is a mail route but this credential's mail is blocked,
///      bounce it here. `/mcp` and control routes are not caught by this check (`ControlRoute::is_mail`).
///   1.5. Mcp-Session-Id format/required-ness → 400. This check guarantees the binding check's "a session op always
///      resolves to a binding" below (not relying on rmcp's internal 4xx behavior).
///   2. Session binding check (FIX 7 + round-2 F1) — match = pass / identity mismatch = **403** / no binding = **404**.
///      initialize is skipped because it has no session id yet (the session is created in the response). On DELETE, after identity
///      is confirmed, the session binding is pruned (FIX 8/F6).
///   3. Plant the verified identity in the request extensions and pass to inner (StreamableHttpService) → rmcp passes it
///      to the tool as `http::request::Parts` (the official custom-extension pattern).
///   4. If the response has a new Mcp-Session-Id (initialize succeeded), pin the identity to the session **only once** with `bind_session_if_absent`.
///      A failure (duplicate/dead) skips the binding (a duplicate is harmless, a dead one gets 401/403 on the next request).
///
/// ★Why 401/403 in the middleware (before the handshake)★: rmcp has no built-in auth (the official auth pattern = axum
///   middleware). Deferring verification into the handshake would let even a wrong token create a session — blocking here guarantees "rejection comes before any
///   MCP session state change" (acceptance).
async fn bearer_auth<B>(
    State(registry): State<Arc<ControlRegistry>>,
    request: Request<B>,
    next: Next,
) -> Response
where
    B: Send + 'static,
    Request<B>: Into<Request<axum::body::Body>>,
{
    // ★The strictness of the "Bearer " prefix is intentional (FIX 13)★: this header is a value the daemon **authored directly** in mcp-config
    //   (claude sends it as is), so its format is fixed — there is no reason for lenient parsing such as case variants or extra whitespace
    //   (it is not a general-purpose server). Only the exact `"Bearer "` prefix is allowed.
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

    // ★Mail rejection is here, not in the handler (ADR-0133 decision 3)★: this middleware wraps the **whole** router, so
    //   even when mail routes grow they are covered in one place — with a check per handler, a new handler would miss the check.
    //   The decision input is a single fact about the credential fixed at issue time, so neither a manager lookup nor body parsing is needed.
    // ★This is the only enforcement point★: nothing on the agent side blocks mail — the CLI sends a sensible
    //   mail call as is. Remove this check and mail is open.
    // ★What it blocks is only this HTTP/CLI mail surface★: an MCP-capable agent keeps writing mail through `eg_send` on `/mcp`
    //   (ADR-0128 decision 1 — the channel is split only by capability). This rejection is not "mail forbidden" but "this entrance is not
    //   your channel".
    // ★Why warn (not info)★: the default log level is warn, so with info an operator chasing "why can't this agent send
    //   letters" would find **no trace at all**. The re-review trigger of ADR-0133 (observing the behavior of an agent that got only rejections)
    //   also fires only if this line is visible. It is set to the same level as the sibling rejections (401/403/404/400).
    // ★This surface has **two** mail axes — this line is the path axis★: a route that selects a command by body is not
    //   classified by path, so it makes the same decision and the same log in its own place (the name axis in `control_call_handler`).
    // ADR-0133
    if !binding.mail_allowed && mail_gated_path(request.uri().path()) {
        tracing::warn!(
            agent = %identity.agent_id,
            epoch = identity.epoch,
            "우편 요청 거절 — 이 자격증명은 CLI/HTTP 우편 입구의 것이 아니다(ADR-0133)"
        );
        return mail_not_allowed();
    }

    // ★The existing session id the request carries (if any)★ — follow-up requests after initialize (tools/call · GET · DELETE)
    //   carry Mcp-Session-Id as a header. Identity pinning is checked with this value (the initial initialize has none).
    // ★malformed ≠ absent (Codex LOW)★: if the header is **present but** to_str() fails (non-UTF-8, etc.), it must not be folded to None —
    //   folding to None makes a session-carrying request be mistaken for "sessionless", so it skips the binding check below and
    //   passes through to inner (rmcp) (boundary bypass). Present-but-malformed is a client error, so it is cut with 400 before
    //   reaching the binding check (not an identity/auth problem, so 400 rather than 401/403, with an empty body). Only a header that is truly
    //   **absent** is treated as "sessionless" (the initialize path).
    let method = request.method().clone();
    let req_session_id = match request.headers().get(SESSION_ID_HEADER) {
        None => None,
        Some(v) => match v.to_str() {
            Ok(s) => Some(s.to_string()),
            Err(_) => {
                tracing::warn!(
                    "제어 채널 malformed Mcp-Session-Id 헤더 거부(400, ADR-0086 Codex LOW)"
                );
                return bad_request();
            }
        },
    };

    // ★Session operations (GET/DELETE) must resolve to a session binding (security lens)★: GET (SSE stream) and DELETE
    //   (teardown) are **operations on an existing session**, so they must carry a session id. A GET/DELETE without a session id
    //   cannot end in a binding, so it is not passed to inner but cut here with 400 (the "no inner reach without a
    //   binding" boundary integrity is guaranteed in the middleware, not by relying on rmcp's internal 4xx behavior). A POST without a session id is
    //   the exception — that is the initialize path (the session is created in the response, so having no session id yet is normal).
    if req_session_id.is_none() && (method == Method::GET || method == Method::DELETE) {
        tracing::warn!(
            method = %method,
            "제어 채널 세션 operation 무-세션id 거부(400, ADR-0086 — session op 는 바인딩으로 resolve 돼야)"
        );
        return bad_request();
    }

    if let Some(sid) = &req_session_id {
        match registry.identity_for_session(sid) {
            Some(bound) if bound == identity => {}
            Some(_) => {
                // The session id is also a caller-chosen header value, so it goes through [`sanitize_for_log`] (the same forgery axis).
                //   The HTTP parser already filters CR/LF, but tabs and non-ASCII obs-text pass and the length is not
                //   ours either — here we decide the width as well.
                tracing::warn!(
                    session = %sanitize_for_log(sid),
                    "제어 채널 cross-token 세션 탈취 거부(403, ADR-0086 FIX 7)"
                );
                return forbidden();
            }
            // ★orphaned-session rejection (round-2 F1)★: a session id is carried but the daemon has **no** binding.
            //   It used to pass this through to inner (rmcp) and let rmcp return 404, but that path has a fatal window:
            //   agent A opens session S and is bound, then a revoke (kill) prunes **only the binding**, while the rmcp-side
            //   session S may still be alive. If agent B with a valid token then presents S, the middleware
            //   passes it through as is and B attaches to A's orphaned session worker (session hijack). Now **every request
            //   carrying a session id without a binding is rejected**, closing that window — a session alive on the rmcp side but unknown to the daemon
            //   becomes an unreachable orphan. This also makes the DELETE-prune order fail-safe.
            //   A truly-unknown id got rmcp's 404 before too, so the status code a normal client sees
            //   does not change (no impact on the happy path).
            None => {
                tracing::warn!(
                    session = %sanitize_for_log(sid),
                    "제어 채널 orphaned/unknown 세션 거부(404, ADR-0086 F1)"
                );
                return not_found();
            }
        }
        // ★unbind-before-inner order choice (round-2 F6)★: the daemon binding is removed **before** inner (rmcp) actually
        //   closes the session. With F1 (reject session-carrying requests without a binding) in place, this order
        //   is fail-safe: even if the inner close fails for some reason after the unbind and the rmcp-side session remains, the daemon
        //   binding is already gone, so by F1 that session becomes an **unreachable orphan** —
        //   i.e. the state "binding removed but session worker alive" does not open a security window. Conversely,
        //   with unbind-after-close the prune hangs on the close succeeding, and if the close fails the binding remains, risking
        //   unbounded growth and stale bindings. It is also not an arbitrary prune, since it runs after the identity check (the match above).
        if method == Method::DELETE {
            registry.unbind_session(sid);
        }
    }

    let mut request: Request<axum::body::Body> = request.into();
    request.extensions_mut().insert(identity);
    // ★The permission bit is carried **separately** from the identity★: `BoundIdentity` is the identity key for session pinning, so if permission is mixed in,
    //   an authorization problem is disguised as a false session-hijack alarm (see that type's doc). Even so, there is one place
    //   where the handler must see this bit — the mail axis of a route that selects a command by body ([`mail_by_name`]) cannot be closed by the route classification above.
    request.extensions_mut().insert(binding);

    let response = next.run(request).await;

    // ★Pass along the same token string used for validation (FIX 7 + round-2 F2)★: bind compares locally whether that token is still this agent's
    //   current credential, filtering out a revoke/re-issue in the validate→bind window.
    if let Some(session_id) = response
        .headers()
        .get(SESSION_ID_HEADER)
        .and_then(|v| v.to_str().ok())
    {
        registry.bind_session_if_absent(session_id, identity, &token);
    }
    response
}

/// 401 response (empty body). WWW-Authenticate is deliberately not included (it is a static Bearer, so no challenge is needed).
fn unauthorized() -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .body(axum::body::Body::empty())
        .expect("valid 401 response")
}

/// 403 response (empty body) — rejects cross-token session hijack (FIX 7). The token itself is valid (hence not 401) but has no
/// permission to access this session (a session pinned to another identity).
fn forbidden() -> Response {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .body(axum::body::Body::empty())
        .expect("valid 403 response")
}

/// 404 response (empty body) — rejects an orphaned/unknown session (round-2 F1). A request carrying a session id that has no daemon binding.
/// The token is valid, but this session does not exist from the daemon's authorization view (it is not established that it belongs to another identity
/// either, so 404 rather than 403). The body is left empty so no session or identity information leaks.
fn not_found() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(axum::body::Body::empty())
        .expect("valid 404 response")
}

/// Mail rejection response (ADR-0133 decision 3).
///
/// ★Why 200 + a bounce envelope (not an empty 403)★: the contract of mail routes is "always 200 + JSON", and the CLI decides by
///   **exit code**, not stdout shape — this envelope is a verified bounce shape, so it folds as is into the existing three-way split's
///   **failure (1)** (no new class of outcome is created). With an empty 403 the CLI would print only a blank line
///   and the caller would have no way to know why it failed.
/// ★The body wording is canonical in `MAIL_NOT_ALLOWED_HINT`★ — it does not announce an alternative channel.
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

/// 400 response (empty body) — client request-format error (ADR-0086). It is a request-format problem, not an identity/auth problem,
/// so 400 rather than 401/403/404. The body is left empty so no information leaks.
fn bad_request() -> Response {
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .body(axum::body::Body::empty())
        .expect("valid 400 response")
}

// ── CLI entrance (/control/send) ────────────────────────────────────────────────────────

/// `/control/send` request body. `{to, body, request?, reply_by?, reply_to?}` — no from field (identity only from the token).
/// The C3 args are all optional, so it is **wire-compatible** with the old `{to, body}` body (omitted = notice).
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

/// ★Recipient tokenization of the CLI entrance (pure function — review C3)★: in a shell the only way to express a list in `engram mail send --to a,b` is
/// a comma, so it is split once **at this entrance only**. That the splitting rule **differs per entrance is intended** (the MCP-side rule is
/// in the `ToField` comment).
fn cli_recipient_tokens(to: ToField) -> Vec<String> {
    to.into_tokens()
        .into_iter()
        .flat_map(|t| t.split(',').map(|p| p.to_string()).collect::<Vec<_>>())
        .collect()
}

/// `/control/send` route State — shares the **same Arc** as the MCP factory (a second registry is not created).
#[derive(Clone)]
struct ControlSendState {
    manager: Arc<ManagerSlot>,
    registry: Arc<ControlRegistry>,
    messaging: Arc<MessagingSlot>,
}

/// Always 200 + JSON body (success and corrective errors both ride on an open request — the CLI parses the JSON to decide the exit code).
async fn control_send_handler(
    axum::extract::State(state): axum::extract::State<ControlSendState>,
    identity: Option<axum::Extension<BoundIdentity>>,
    body: Option<Json<SendRequest>>,
) -> Response {
    let Some(axum::Extension(from)) = identity else {
        // The middleware did not plant an identity = an auth-path fault. Defensive 401 (unreachable in the normal flow).
        return unauthorized();
    };
    let Some(Json(req)) = body else {
        return bad_request();
    };
    let Some(manager) = state.manager.get() else {
        tracing::error!(
            entrance = "cli",
            "제어 채널 send 불가 — manager 슬롯 미설정(배선 순서 이상, ADR-0086 F6)"
        );
        return service_unavailable();
    };
    let Some(messaging) = state.messaging.get() else {
        tracing::error!(
            entrance = "cli",
            "제어 채널 send 불가 — messaging 슬롯 미설정(배선 순서 이상, C1)"
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
        // JoinError = blocking task panic (absent from the normal flow). The "always 200 + JSON" contract is the contract for **when the handler
        //   produced an answer**, so this path, which has no answer at all, is split off to 500 so the CLI does not
        //   misread it as success (no new wire error code is invented — the spec §6 vocabulary is fixed).
        tracing::error!(entrance = "cli", "제어 채널 send 태스크 실패(패닉)");
        return Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(axum::body::Body::empty())
            .expect("valid 500 response");
    };
    Json(result.to_json()).into_response()
}

/// `/control/messages` request body (D) — `{id?}`. No identity field (derived from the token — the same invariant as send).
#[derive(Debug, Default, serde::Deserialize)]
struct MessagesRequest {
    #[serde(default)]
    id: Option<String>,
}

/// ★Empty body allowed★: `pending` (the no-argument query) has no field to send. The CLI sends `{}`, but even if there is no body at all or
///   it fails to parse, it is **folded into a no-argument query** — a query has no side effects, so leniency is safe, and a 400 here would
///   keep the CLI from telling "no argument given" from "request broken", so self-correction would spin in vain (send is the opposite, 400 —
///   it has required fields and has side effects).
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
            "messages 조회 불가 — manager/messaging 슬롯 미설정(배선 순서 이상)"
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
/// ★It does not hold the manager or fan-out slots★: **the table** holds both (`commands::make_daemon_table`). Holding
///   them again here would create a second path to the same real object, and if notifications go out from two places, the split of per-verb
///   notification counts (wake-up 0 · change 1) breaks. It does not hold registry or messaging either — control verbs use identity only
///   for authorization and have no sender derivation.
#[derive(Clone)]
struct ControlAgentState {
    commands: Arc<CommandTableSlot>,
}

/// Always 200 + JSON body (success and bounce alike) — the same contract as `/control/messages`, so the CLI's query decider
/// is used as is.
///
/// ★Identity is used only for authorization★: "me" does not appear in control verbs (no sender derivation). Still, the **existence** of the
///   identity the middleware planted is required — a call without a token must not reach here (defensive 401).
/// ★Blocking boundary★: `handle_agent` below is blocking (see its function doc — resume polling ~3 s + profile disk
///   save). Calling it directly from an async handler ties up one tokio worker thread for that time and blocks **other requests**
///   stacked on the same thread (the same reason the send route moved the child stdin write to the blocking pool).
async fn control_agent_handler(
    axum::extract::State(state): axum::extract::State<ControlAgentState>,
    identity: Option<axum::Extension<BoundIdentity>>,
    body: axum::body::Bytes,
) -> Response {
    if identity.is_none() {
        return unauthorized();
    }
    // ★Why the `Json<…>` extractor is not used★: when deserialization fails, that extractor returns an **empty 400** —
    //   mistyped values, duplicate keys, non-object bodies and broken JSON all land there, and the caller (an LLM) retries without knowing
    //   what to fix. Deserializing directly lets serde's reason ride in the envelope as is,
    //   and then this route returns an empty body only for auth failure and size overrun.
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
            "제어 동사 처리 불가 — 명령 표 슬롯 미설정(배선 순서 이상, ADR-0155)"
        );
        return service_unavailable();
    };
    let table = table.clone();
    let Ok(result) =
        tokio::task::spawn_blocking(move || super::agent::handle_agent(&table, req)).await
    else {
        // JoinError = blocking task panic. There is no answer at all, so it is outside the "always 200 + JSON" contract — it is split off to 500
        //   so the CLI does not misread it as success (the same discipline as the send route). ★That this contract holds **only in
        //   hook-less test builds** is canonical in the `internal_error` comment★.
        tracing::error!(entrance = "cli", "제어 동사 태스크 실패(패닉)");
        return internal_error();
    };
    Json(result.to_json()).into_response()
}

// ── CLI discovery and generic call entrance (/control/commands · /control/call) ──────────────────────

/// State of the two catalog routes — holds as is the **two sources** that discovery merges (`control::catalog::merge`).
///
/// ★The registry is a **handle**, not a slot★: the table (`CommandTableSlot`) comes into being after the manager and needs late injection, but
///   the registry is a shared `Clone` handle that the assembly creates **before** the server (`lib.rs` run() · accept loop). Adding
///   one more late-injection slot would add the branch 「비어 있으면 반쪽 목록」 ("if empty, a half list") for no reason.
/// ★Do not create a second registry★ — what is held here must be **the one copy** that connections add their registrations to. If it holds a copy,
///   discovery returns an empty list forever, and that symptom has no error and no log.
#[derive(Clone)]
struct ControlCatalogState {
    commands: Arc<CommandTableSlot>,
    /// Holds the registry + the seat table + the first-stage table as one value — ★it must be **the one copy** the socket entrance holds★
    /// (ADR-0160). If it holds a copy, discovery returns an empty list and the relay uses its own seat table.
    bus: crate::command_delivery::CommandBus,
}

/// Discovery list — success and bounce are both 200 + JSON, and only the case with no answer at all is 500 (the scope of that branch is in the
/// [`internal_error`] comment). ★An empty table slot is not 503★: in the same state the WS list gives
/// 「명부 절반」 (half the registry), so a 503 here would have two surfaces say different things about the same state (the canonical account of that choice =
/// `catalog::handle_list` doc).
///
/// ★What `spawn_blocking` fixed and what it did not — do not assume★
///   - **Fixed = executor hazard.** `CommandRoster::entries` takes a blocking mutex, so calling it from an async worker
///     **parks** that worker instead of yielding. Now the blocking pool bears that wait. It also matches the treatment of the sibling
///     (`control_agent_handler`) — there is no reason to take the same lock on a worker on only one side.
///   - **Not fixed = lock hold time.** `entries` still clones the entire registry (cap 4096 ×
///     help 4 KiB) **while holding** the mutex, and that mutex is shared by command delivery (`OwnerLookupSource::lookup`) and connection cleanup.
///     So a child running this route **still contends with delivery** — what changed is **who waits**, not how long
///     the lock is held. That cloning discipline is owned by the registry module, so it cannot be reduced here (the
///     「알려진 비용」 (known cost) in the `command_roster` header).
/// ★`decls()` rebuilds `help` on every call (the tools crate's `CommandTable::decls`) — leave it as is★:
///   the daemon's own table has only 5 entries today, so this response's cost is dominated by the registry side, and memoization is owned by that crate,
///   so it is not a place to touch here. If the table grows, measure it over there.
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
        tracing::error!(entrance = "cli", "발견 목록 태스크 실패(패닉)");
        return internal_error();
    };
    Json(result.into_json()).into_response()
}

/// Full-name call — always 200 + JSON (success and bounce alike), the same contract as `/control/agent`.
///
/// ★The reasons for not using the `Json<…>` extractor and for the `spawn_blocking` boundary are both the same as the sibling's (`control_agent_handler`)★ —
///   that comment is canonical. The two routes run the same table through the same function, so the treatment must be the same.
///
/// ★This route **may be delayed until the deadline** (ADR-0160 impact section)★: another party's name is relayed to its owner, and if the owner
///   does not answer, the delivery deadline (`CommandDeliveries::DEFAULT_DEADLINE`) yields `TIMEOUT`. The daemon's own names
///   finish on the spot as before, so they do not bear this delay.
/// ★`request_id` **may be sent by the caller**★ (ADR-0161 decision 1) — if not sent, the decider issues one
///   (`catalog::handle_call`). That split is not recreated here.
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
    // ★The mail axis is closed **by name**★ — this route selects a command by body, not path, so the middleware's
    //   route classification does not reach here (that classification doc · [`mail_by_name`]). It decides before taking anything needed for execution,
    //   and before telling which source the name belongs to — the answer must be the same whether it is the daemon table or the registry.
    // ★The rejection is logged at the **same level and with the same fields** as the sibling★ (the path-axis gate in `bearer_auth`): without that, a credential
    //   with mail blocked **groping around by name would leave no trace on the server at all**, and the ADR-0133 re-review
    //   trigger (observing the behavior of an agent that got only rejections) would lose its material on this axis too.
    // ★The name goes through [`sanitize_for_log`] — do not use `agent::preview`★: this line is printed **before validation**,
    //   so the name is a completely arbitrary string, and that function only truncates length and lets control characters through. One
    //   newline splits a log line in two and the second line looks like an entry we printed (a forged entry — that function's doc describes this
    //   failure mode). Being `warn`, it really lands in the file at the default level, which is what makes this axis real.
    // ADR-0133
    if !binding.mail_allowed && mail_by_name(&req.name) {
        tracing::warn!(
            agent = %caller.agent_id,
            epoch = caller.epoch,
            command = %sanitize_for_log(&req.name),
            "우편 요청 거절 — 이 자격증명은 CLI/HTTP 우편 입구의 것이 아니다(이름 축, ADR-0133)"
        );
        return mail_not_allowed();
    }
    // ★This check is a **wiring alarm**, not a routing authority★: the authority is the single `bus.locals()` below (so that decision and
    //   delivery see the same fact — that paragraph in `catalog::handle_call`), and what this line guards is only this route's contract that
    //   「표를 늦게 꽂는 조립에서 이름을 보기도 전에 크게 드러난다」 (in an assembly that plugs the table in late, it surfaces loudly before the name is even looked at) (the canonical account of that treatment =
    //   the two-route comparison test in `tests/control_agent.rs`). ★The assembly must pass this slot and the bus's first-stage table as the **same
    //   one copy** for this alarm to point at the real state★ — the production assembly does so (`lib.rs`).
    if state.commands.get().is_none() {
        tracing::error!(
            entrance = "cli",
            "전체 이름 호출 불가 — 명령 표 슬롯 미설정(배선 순서 이상, ADR-0155)"
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
        tracing::error!(entrance = "cli", "전체 이름 호출 태스크 실패(패닉)");
        return internal_error();
    };
    let result = match outcome {
        super::catalog::CallOutcome::Answered(result) => result,
        super::catalog::CallOutcome::Relay {
            name,
            args,
            request_id,
        } => {
            // ★Record who ordered the command★: a relayed name actually moves the dashboard's windows, tabs and slots
            //   (the layout commands in `src-tauri`), so it must be able to answer 「내 창을 누가 닫았나」 (who closed my window?). The material for that
            //   **has already arrived** at this gate (`BoundIdentity`), but until now it was used only for the existence check and thrown away.
            //   ★The key is not logged★ — the identity is derived from the key but is not itself a credential
            //   (logging convention 「보안」, security).
            // ★Why warn (not info)★: what this line holds is a **fact that cannot be recreated anywhere after the event has passed**
            //   — **which credential** ordered a user-visible change. The result of the operation remains on screen and in
            //   state queries, but the owner is recorded nowhere, so even raising the level later never reveals the owner of an operation
            //   that already happened. And this route has decided to treat 「에이전트가 사람 화면을 움직였다」 (an agent moved the human's screen) not as everyday noise
            //   but as **an event to be seen** — the same judgment is already embedded in this file's sibling gate
            //   (the `warn` clause of the mail rejection — `bearer_auth`).
            //   ★Do not use 「기본 레벨이 warn 이라 info 는 안 찍힌다」 (the default level is warn, so info is not printed) as the reason★ — the logging convention
            //   explicitly forbids that argument (raising the level of a normal notice because of the default filter). The two sentences above are the reason.
            //   ★So one relay produces this one warn line★ — the success line on the outcome side stays at `info` (that comment).
            //   ★Writing this exception into the logging convention doc has not been done yet (open — outside this round's scope).★
            //   The arrival rate is within this gate's natural cap (connections per request · `MAX_RELAYED_IN_FLIGHT`).
            // ★The entrance label uses the constant★ — this call goes through `CommandBus::invoke`, so it must equal the label that side
            //   records. `"cli"` belongs to **direct execution that does not go through relay** (that constant's doc), so it is not this one's.
            // ★The name goes through [`sanitize_for_log`]★ — the reason is the same as the mail-rejection line above (a caller-chosen
            //   string goes into the log as `warn`). The delivery side uses the same function (the first line of
            //   `command_delivery::deliver`) — so that one value is not written in different shapes in two layers.
            // ★The wording does not say 「중계했다」 (relayed) — this line is **before** the round trip★: the `invoke` after this, if it hit the concurrency
            //   cap, runs nothing and bounces (`MAX_RELAYED_IN_FLIGHT`), so writing 「중계한다」 (relaying) would
            //   leave a line recording something that did not happen on that branch. It states only the fact, and the paired lines give the outcome —
            //   `CommandBus::invoke` emits one success/failure line, and the cap rejection emits one line from `RelaySlot::admit`
            //   (all three carry the same `entrance` · `request_id`, tying them into one round trip).
            // ★Even so, do not defer this line until **after** the round trip★: attribution cannot be recreated anywhere once the event has passed
            //   (reason above), so if the daemon dies mid-round-trip, a deferred line is never written at all.
            tracing::warn!(
                entrance = crate::command_delivery::ENTRANCE_CONTROL,
                agent = %caller.agent_id,
                epoch = caller.epoch,
                command = %sanitize_for_log(&name),
                %request_id,
                "에이전트가 남의 명령을 불렀다 — 이 데몬이 그 주인 앞으로 접수했다"
            );
            let reply = bus.invoke(request_id, name, args).await;
            super::catalog::relayed(reply)
        }
    };
    Json(result.into_json()).into_response()
}

// ── CLI help screen entrance (/control/help) ───────────────────────────────────────────────

/// help screen — success and bounce are both 200 + JSON, and only the case with no answer at all is 500 (the scope of that branch is in the
/// [`internal_error`] comment). The body contract · failure policy are owned by `control::help`.
///
/// ★Identity is required only to exist★: the screen is the same no matter who asks, so identity is not used to choose the screen. A call without a token must not
///   reach here, so only a defensive 401 is placed (same as the sibling `control_agent_handler`).
/// ★Read inside `spawn_blocking`★: the source reads the body file on every request (no cache), so reading it
///   on an async worker would block other requests stacked on that worker during the disk wait.
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
        tracing::error!(entrance = "cli", "help 화면 태스크 실패(패닉)");
        return internal_error();
    };
    Json(result.into_json()).into_response()
}

/// 500 response (empty body) — the case where the blocking task panics and there is **no answer at all**.
///
/// ★This branch cannot be reached in a production process (known scope)★: a panic in a blocking task is not a `JoinError` but
///   **process termination** (hook — ADR-0292). The command execution that task drives is outside the containment marker (ADR-0293). So
///   "panic → 500" is a contract that holds only in hook-less test builds,
///   and in production this spot guards nothing. It is still not deleted because the branch is real in that build, and without it
///   a request with no answer would hang in that build.
fn internal_error() -> Response {
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .body(axum::body::Body::empty())
        .expect("valid 500 response")
}

/// 503 response (empty body) — slot not set (wiring-order fault). Not a request-format or auth problem, so it is not 4xx.
fn service_unavailable() -> Response {
    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .body(axum::body::Body::empty())
        .expect("valid 503 response")
}

/// The variant brought up without a help source — `/control/help` answers every request with `INTERNAL` (`help source not configured on this
/// server`) and leaves one warn line at startup. The meaning of the args is the same as [`start_mcp_server_with_help`].
///
/// ★This is for tests · harnesses that do not call help — if the production assembly (`lib.rs`) calls this variant, help is always a bounce★.
///   The name was left to say 「help 없음」 (no help) so that such a call stands out in review.
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

/// The registry must be the **same Arc** shared by the auth middleware (verification) and provision (issuing).
///
/// ★Local only + DNS rebinding defense★: bind is 127.0.0.1:0 (OS-assigned port). StreamableHttpServerConfig
///   allows only local Hosts with the default allowed_hosts=[localhost,127.0.0.1,::1] (rmcp default). stateful_mode=true (default),
///   so the session is kept by Mcp-Session-Id.
pub async fn start_mcp_server_with_help(
    registry: Arc<ControlRegistry>,
    manager: Arc<ManagerSlot>,
    messaging: Arc<MessagingSlot>,
    // ★The fan-out slot does not come here★: only the command table reads it (`commands::make_daemon_table`),
    //   and if the server held a second copy, **nobody would tell you even if the assembly passed two different slots** — the symptom is the tree showing
    //   the old registry forever, with no error and no log. Taking it out of the args removes that fork.
    commands: Arc<CommandTableSlot>,
    // ★The command bus comes **by value**★: it is a shared `Clone` handle and the assembly can create it before this server
    //   (`lib.rs` was changed to do so), so no late-injection slot is needed. Half of the discovery list and the relay
    //   delivery come from here, so the assembly must pass **the one copy the accept loop holds** — if it creates and passes a new one,
    //   discovery always yields only the daemon's own names and the relay uses its own seat table (ADR-0160).
    bus: crate::command_delivery::CommandBus,
    // ★The help source is built by the assembly★ — if the server built it inside from the install location, a test could not
    //   inject a known body, and the screen would be tied to the test exe's location · `CARGO_TARGET_DIR`.
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

    // ★nest_service★: StreamableHttpService is a Tower service, so it sits on the axum router as is.
    // ★Body limit = RequestBodyLimitLayer (round-2 F4)★: request bodies on the local control channel are small — a limit is stated explicitly so
    //   malicious/runaway bodies cannot swallow memory. axum `DefaultBodyLimit` applies only to **extractors** (Json/Bytes, etc.),
    //   and rmcp `StreamableHttpService` consumes the raw body directly (not via an extractor), so that limit does not
    //   work. `RequestBodyLimitLayer` wraps the body itself and enforces the limit on every downstream consumer (including rmcp),
    //   cutting with 413 on overrun. 1MB is enough for initialize/tools/call and send POST payloads.
    // ★Layer order★: below, the stack is body-limit → auth → route from outer to inner (in an axum layer, the one written later is
    //   outer). Putting body-limit outermost applies the limit before whichever of auth or inner reads the body.
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
    // ★Add routes by walking the list (`ControlRoute::ALL`) — do not revert to a builder chain★: a new route must
    //   enter the list to be served, and once it does, the exhaustive match in `is_mail` forces mail classification
    //   at compile time (ADR-0133). A chain bypasses that enforcement.
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
            tracing::warn!("MCP axum serve 종료: {e}");
        }
    });

    tracing::info!(
        port = addr.port(),
        path = MCP_PATH,
        "MCP 서버 시작(ADR-0086)"
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

    /// ★This function is the body of this test★: adding a variant **breaks compilation** here, forcing the classification to be written by hand.
    ///   The point is to state it **separately** from `ControlRoute::is_mail`, so it does not call that — if the two diverge,
    ///   the assertions below go red.
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
                "{route:?}({}) 의 우편 분류가 갈렸다",
                route.path()
            );
            assert_eq!(
                ControlRoute::from_path(route.path()),
                Some(route),
                "명단에 있으면 경로로 되찾을 수 있어야: {route:?}"
            );
        }
        // The router is assembled by walking this list — if the length shrinks, a route has silently disappeared.
        assert_eq!(ControlRoute::ALL.len(), 6);
        assert_eq!(
            ControlRoute::ALL.iter().filter(|r| r.is_mail()).count(),
            2,
            "우편 라우트는 발송·조회 둘 — 제어 평면이 늘어도 이 수는 안 는다"
        );
    }

    /// `/mcp` and paths outside control are not gate targets (MCP is that credential's own channel).
    #[test]
    fn the_mcp_route_is_outside_the_mail_gate() {
        for path in [MCP_PATH, "/mcp/", "/mcp/whatever", "/", "/health"] {
            assert!(
                !mail_gated_path(path),
                "제어 평면 밖 경로에 우편 게이트가 걸리면 안 된다: {path}"
            );
        }
    }

    /// ★A control path with a missing classification folds toward rejection★: if this direction were reversed, the mistake of forgetting to register in the list when adding a route
    ///   would **open a mail entrance with no signal at all**.
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
                "전제: 명단 밖 경로여야 이 단언이 의미를 가진다({path})"
            );
            assert!(
                mail_gated_path(path),
                "명단 밖 제어 경로는 우편으로 접혀야(fail-closed): {path}"
            );
        }
        // A route in the list follows its own classification as is (the folding does not overwrite the classification).
        for open in [
            CONTROL_AGENT_PATH,
            CONTROL_COMMANDS_PATH,
            CONTROL_CALL_PATH,
            CONTROL_HELP_PATH,
        ] {
            assert!(!mail_gated_path(open), "제어 평면은 전원 개방: {open}");
        }
        for route in ControlRoute::ALL {
            assert_eq!(mail_gated_path(route.path()), route.is_mail(), "{route:?}");
        }
        // ★Segment boundary★: if paths that merely look similar but are not in this namespace were folded, a future
        //   route at that spot would be rejected for no reason (the failure mode of a bare string-prefix comparison).
        for path in [
            "/controlfoo",
            "/control-x",
            "/controls/send",
            "/CONTROL/send",
        ] {
            assert!(
                !mail_gated_path(path),
                "제어 네임스페이스 밖 이름을 접으면 안 된다: {path}"
            );
        }
    }

    /// ★The rejection response does not announce an alternative channel (ADR-0133 decision 3)★: announcing the other channel here would let the mutual-exclusion seal of
    ///   the two priming files be bypassed through this path.
    ///
    /// ★Read the body made by the real constructor (`mail_not_allowed`) — do not reassemble the same JSON here★:
    ///   if a copy were inspected, this test would stay green even if the constructor added a field or changed the wording.
    #[tokio::test]
    async fn the_mail_rejection_names_no_other_channel() {
        let resp = mail_not_allowed();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "우편 라우트의 '항상 200 + JSON' 계약을 따른다(CLI 는 exit code 로 판정)"
        );
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .expect("거절 body");
        let body = String::from_utf8(bytes.to_vec()).expect("UTF-8 body");
        let lowered = body.to_lowercase();
        for forbidden in [SEND_MESSAGE_TOOL, MESSAGES_TOOL, "mcp", "tool", "server"] {
            assert!(
                !lowered.contains(&forbidden.to_lowercase()),
                "거절 응답이 상대 채널을 가리키면 안 된다({forbidden}): {body}"
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

    /// ★A caller string that goes verbatim into a log line printed **before** validation becomes log forgery★
    ///
    /// This route's mail rejection leaves a `warn` having only looked at the name and verified nothing — so
    /// that name is a completely arbitrary string. One newline splits a log line in two and the second line looks like an entry we printed,
    /// and since the default level is warn, that line **really lands in the production log file**.
    /// ★What is measured is not 「함수를 불렀나」 (was the function called) but 「그 필드에 원문이 갔나」 (did the verbatim text reach that field)★ — so it runs the real handler
    /// and reads the field text of the log event. Reverting to a length-only truncating function (`agent::preview`) turns this red.
    #[test]
    fn a_control_character_in_the_called_name_never_reaches_the_log_field_verbatim() {
        // A newline + a forged entry + **a tail tag that must be pushed out of the width** (two properties measured with one value).
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
            assert_eq!(resp.status(), StatusCode::OK, "우편 거절은 200 + 반려 봉투");
            sweeper.stop().await;
        });

        let logged = lines
            .iter()
            .find(|line| line.contains("command="))
            .unwrap_or_else(|| panic!("거절 줄이 남아야 한다(계측 의무): {lines:?}"));
        assert!(
            !logged.contains('\n'),
            "로그 필드가 줄을 쪼갤 수 있으면 위조 항목이 생긴다: {logged:?}"
        );
        assert!(
            logged.contains("\\n"),
            "제어문자는 escape 된 모양으로 남아야(값을 지우는 것이 아니다): {logged:?}"
        );
        assert!(
            !logged.contains("TAILMARK"),
            "로그 한 줄의 폭을 상대가 정하게 하면 안 된다(폭 상한): {logged:?}"
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
            "스키마에 to/body: {s}"
        );
        assert!(
            !s.contains("\"from\""),
            "eg_send 스키마에 from 필드가 없어야: {s}"
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
        let one: ToField = serde_json::from_str(r#""bob""#).expect("string 형태");
        assert_eq!(one.into_tokens(), vec!["bob".to_string()]);
        let many: ToField = serde_json::from_str(r#"["bob","@all"]"#).expect("array 형태");
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
            "라우터에 '{SEND_MESSAGE_TOOL}' 툴이 등록돼 있어야(const ↔ #[tool] 메서드명 일치 강제)"
        );
    }

    /// ★The pin moved over (the front half of `control/priming.rs::production_priming_files_pin_taught_channels`)★:
    ///   it used to check whether the priming file carries the word `send_message` — because at that time that was the
    ///   **only teaching surface** for MCP-capable spawns. As priming shrank to a one-line pointer, that surface
    ///   moved to this description (ADR-0126 decision 1's "an entrance teaches only one" is unchanged), so the pin came along too.
    ///
    /// ★The name alone is not enough★: what `tools_list_exposes_send_message_tool` checks is only the routing name,
    ///   so it stays green even if the description is empty. If the description disappears, all the agent keeps is the name and the JSON schema,
    ///   and it runs **without learning how to call it from anywhere** — priming no longer backs that spot,
    ///   so this is the one place that catches that silence.
    ///
    /// ★The **tool-argument spelling** of the reply contract (snake_case) is checked here too (ADR-0103 decision 2/3)★: envelope recognition
    ///   (including `<notice>`) is owned by the `engram help mail` screen (`control/help.rs`), and this side owns only its own entrance's
    ///   argument spelling. Splitting one contract per surface is the shape of ADR-0126 decision 1.
    ///
    /// ★The two assertions below existed only in the old instructions and were revived on this surface — do not delete them★:
    ///   ① do not infer the other party's state from `pending`; query with `eg_messages` (one of the two ADR-0211 decision 2 named
    ///      explicitly. An explanation that pins the cause on a single one flatly violates that prohibition).
    ///   ② tell it apart from the similarly named harness built-in `SendMessage` (ADR-0106 — deny is blocked structurally, but
    ///      if the agent does not know that a misfire ends in a permission denial, it reports to the owner **on its own judgment that the mail
    ///      channel is broken**. ★The old reason 「지시서가 시킨 채널 고장 보고」 (the channel-failure report the instructions ordered) is no longer true★ — the four
    ///      behavior-rule lines that ordered that report were withdrawn from priming (`control/priming.rs` comment), and priming now
    ///      says nothing about this misfire. So this description is the only guidance surface).
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
            .unwrap_or_else(|| panic!("tools/list 에 '{SEND_MESSAGE_TOOL}' 이 있어야"));
        let desc = tool.description.as_deref().unwrap_or_else(|| {
            panic!("'{SEND_MESSAGE_TOOL}' 에 설명문이 있어야(이게 교육 표면이다)")
        });
        // ★The home of the argument contract moved (2026-09-23)★: the description used to re-explain things one line per field; that was withdrawn
        //   and left to the **per-field descriptions of the input schema** — it was a duplicate carrying the same contract in two copies, and the schema side is more
        //   detailed and attached at the very place where the model fills in arguments. The contract did not disappear but gathered into one home,
        //   so this pin looks at that home too. If the schema is empty, the agent receives only field names and cannot learn their meaning.
        // ★Check whether **meaning was carried**, not field names★: schemars prints field names into `properties` as they are even if
        //   all doc comments are erased. Looking at names alone cannot catch the failure condition above, 「스키마가 비면 에이전트는 칸 이름만 받고 뜻을 못
        //   배운다」 (if the schema is empty, the agent receives only field names and cannot learn their meaning) (an adversarial review caught it — 2026-09-23).
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
            .expect("입력 스키마에 properties 가 있어야");
        for arg in ["to", "body", "reply_to", "reply_by"] {
            let slot = props
                .get(arg)
                .unwrap_or_else(|| panic!("입력 스키마가 {arg} 칸을 실어야: {props:?}"));
            assert!(
                carries_description(slot),
                "{arg} 칸에 설명이 실려야(인자 계약의 집이 여기다 — 설명문에서 걷어낸 것을 스키마가 진다): {slot}"
            );
        }
        // ★The ① above (do not infer the other party's state from `pending`; query) moved to the `help mail` screen★ —
        //   ADR-0211 decision 2 remains valid, and what moved is the surface, not the contract. The pin in that place is owned by
        //   `the_mail_screen_teaches_the_reply_contract` in `control/help.rs`. Here we check only **whether it points at** that screen —
        //   if the pointer breaks, the whole out-of-argument contract loses its place to go.
        assert!(
            desc.contains("engram help mail"),
            "설명문이 나머지 규칙을 어디서 읽는지 가리켜야: {desc}"
        );
        assert!(
            desc.contains("SendMessage"),
            "이름이 닮은 하네스 내장 툴과 자기를 갈라 가르쳐야(오발이 권한 거부로 끝난다는 것을 모르면 \
             허위 '채널 고장' 보고가 올라간다 — ADR-0106): {desc}"
        );
    }

    #[test]
    fn tools_list_exposes_the_query_tools() {
        // If the const ↔ #[tool] method name drift apart, the tool name priming teaches and the actually exposed name diverge, and the query
        //   entrance silently ceases to exist.
        let router = EngramMcpHandler::tool_router();
        assert!(router.has_route(MESSAGES_TOOL), "'{MESSAGES_TOOL}' 툴 등록");
    }

    #[test]
    fn query_tool_schemas_build_and_carry_no_identity_field() {
        let m =
            serde_json::to_string(&schemars::schema_for!(MessagesArgs)).expect("messages schema");
        assert!(m.contains("\"id\""), "messages 스키마에 id: {m}");
        assert!(
            !m.contains("\"from\"") && !m.contains("required"),
            "messages 인자는 전부 선택 + from 없음: {m}"
        );
    }

    #[tokio::test]
    async fn server_starts_and_reports_local_url() {
        let reg = Arc::new(ControlRegistry::new());
        // ★The sweeper must be held★ — if it is dropped, the seat table closes on the spot and every round trip after that is bounced.
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
            "로컬 엔드포인트 URL: {}",
            handle.url
        );
        handle.shutdown().await;
    }

    // ── round-2 F5 ──────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn dropping_handle_cancels_serve_task() {
        let reg = Arc::new(ControlRegistry::new());
        // ★The sweeper must be held★ — if it is dropped, the seat table closes on the spot and every round trip after that is bounced.
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
        assert!(!watch.is_cancelled(), "start 직후엔 cancel 안 됨");
        drop(handle);
        assert!(
            watch.is_cancelled(),
            "핸들 drop 시 cancel 토큰이 발화돼 detached serve 태스크가 종료돼야(F5)"
        );
    }
}
