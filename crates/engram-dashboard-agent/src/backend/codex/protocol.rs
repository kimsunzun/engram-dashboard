//! codex app-server wire types — the envelope · method names · the payloads we send and receive.
//!
//! ★Source and regeneration (do not infer again)★: field names · required-ness · enum values were read from the JSON Schema codex-cli
//!   **0.154.0** emitted — ★except the usage limit
//!   section (`AccountRateLimitsUpdatedNotification`·`GetAccountRateLimits*` and their types), read from the **0.156.1** schema★.
//!   Regenerate with
//!   `codex app-server generate-json-schema --out <dir>` (266 per-type files under `v2/`); to check upstream drift, regenerate and compare
//!   — do not read this file.
//!
//! ★The envelope is not JSON-RPC 2.0★ — `jsonrpc` appears 0 times in the schema. Do not send it or require it.
//!
//! ★Inbound structs carry **only the fields we read**★ (serde ignores unknown fields; unread required fields only add failure points).
//!   **Outbound** carries every field the schema marks
//!   required, or the server rejects it.
//!
//! ★`deny_unknown_fields` is attached nowhere — a correctness requirement★ (measured 0.154.0): real responses carry fields **not in the
//!   schema** (the `thread/start` response alone:
//!   `environments`·`runtimeWorkspaceRoots`·`extra`·`canAcceptDirectInput`·`daybreakEnabled`·`multiAgentMode`·`activePermissionProfile`·`historyMode`); rejecting them fails a normal response.
//!
//! ★No crate-wide `rename_all`★ — casing is mixed within the protocol (`input_text` beside `inputText`, `approved_for_session` beside
//!   `acceptForSession`);
//!   `rename_all` is attached **only to types whose fields were all confirmed camelCase**.
//!
//! No I/O in this file — string in, type out.

// chunk 1 sets up only the types and the translator, no wiring; chunk 2 adds the consumer of the surface below and removes this allow.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Envelope ──────────────────────────────────────────────────────────────────────

/// JSON-RPC id. The schema is `anyOf[string, i64]`, string first (`RequestId.json`).
///
/// ★An id the server issued must be returned **character for character**★ — not normalized to a number. i64 is intentional (`format:
///   int64`; a JS-side peer may send an id above 2^53).
///
/// ★No variant may serialize as `null` — the invariant of this type★ (measured 0.154.0): `id: null` makes the server's untagged envelope
///   union **reclassify the line as a notification and
/// drop it silently** (no error, nothing on stderr). No `Option`, no Null variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum RequestId {
    Str(String),
    Num(i64),
}

/// `JSONRPCErrorError` — the body of the error envelope.
///
/// ★`code` is an opaque i64★: the schema has no enum · const · min/max and `-32xxx` appears **not once** (`-32001` — grep 0 hits).
///   Overload · rate limit come in the `error`
/// **notification**'s `codexErrorInfo` (`serverOverloaded`·`rateLimitExceeded`), not this envelope.
/// ★Whether a code like `-32001` arrives at runtime is unverified★ — no code branches on the number; observe a real server first if a
///   specific code needs handling.
///
/// Measured 0.154.0: an unknown method answers `-32600` and `data` **did not come** (still optional — the schema allows it). ★`message`
///   can reach 4KB★ (the unknown-method body lists all 160
/// valid methods); cut it before a log · the screen.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RpcError {
    pub(crate) code: i64,
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) data: Option<Value>,
}

/// The four envelope shapes of one line coming in from the server.
///
/// ★Discrimination is by **presence of fields**, never the id value★: server-issued ids are returned as is, so the two id spaces overlap
///   (TRD §4-4).
/// Not `#[serde(untagged)]` — the four shapes' required sets overlap and none forbids extra fields, so order would decide meaning
///   (`JSONRPCMessage` has that shape).
#[derive(Debug, Clone)]
pub(crate) enum Inbound {
    /// `method` + `id` — the server asks us. ★If not answered, that agent freezes permanently★.
    Request {
        id: RequestId,
        method: String,
        params: Option<Value>,
    },
    /// `method` only — nobody waits for an answer. This is all the translator receives.
    ///
    /// ★A real notification also carries `emittedAtMs` beside `method`·`params`★ (measured 0.154.0 — 12/12 notifications, 0/8 responses;
    ///   not in the schema). **Deliberately not carried**:
    /// translation here does not depend on order; read that key from the original line if needed (discrimination holds with it present).
    Notification {
        method: String,
        params: Option<Value>,
    },
    /// `id` + `result` — the answer to a request we sent.
    Response { id: RequestId, result: Value },
    /// `id` + `error` — the failure of a request we sent.
    Error { id: RequestId, error: RpcError },
}

/// The reason [`classify`] could not read a line as any of the four envelopes.
///
/// ★All non-fatal★ — the caller logs the line and drops it; the stream is not cut (TRD §4-6).
#[derive(Debug, thiserror::Error)]
pub(crate) enum ParseError {
    #[error("not JSON: {0}")]
    NotJson(serde_json::Error),
    #[error("the JSON top level is not an object")]
    NotObject,
    #[error("`method` is not a string")]
    MethodNotString,
    #[error("`id` is neither a string nor an i64")]
    BadId,
    #[error("`error` does not have the {{code,message}} shape: {0}")]
    BadError(serde_json::Error),
    #[error("the `{0}` envelope has no `id`")]
    MissingId(&'static str),
    /// `result` and `error` came together in one line — claims success and failure at once; ★if the field that appears first won, a
    ///   failure would be read as a success★.
    #[error("`result` and `error` are together in one envelope")]
    ConflictingEnvelope,
    #[error("none of the four envelope shapes — method, result and error are all missing")]
    UnknownShape,
}

/// Read one **line** as one envelope. Line splitting is the caller's job.
///
/// The discrimination order is the contract: `method` present → request/notification (then `id` splits the two); only when absent are
///   `result`·`error` looked at.
/// ★A line with `method` and `id` is always a request★ — reading it as a notification would drop a request that must be answered.
///
/// ★Discrimination holds with unknown sibling fields★ — real notifications always carry `emittedAtMs` (measured 0.154.0) and upstream may
///   add more; "extra field → none of the four shapes"
/// would drop normal lines whole.
///
/// A non-JSON line, or JSON fitting none of the four shapes, is an **error** — neither a panic nor a quiet drop.
///
/// ★Caller requirement — put a deadline on every request sent★ (measured 0.154.0): the server **gives no answer to an envelope it could
///   not decode, even with an `id`** (`{"id":90}` left
/// only a stderr log; stdout stayed silent forever). Answers come only for *parameter* errors after the envelope was read, so a ticket
///   that "waits until the answer comes" will hang.
///
/// ★Known limitation — duplicate keys are not filtered★: `{"id":1,"id":2,"result":{}}` reads `id=2` (serde_json takes the last value) and
///   the response is **delivered to the wrong waiter**
/// (the original request is never released). Filtering needs a map-free visitor — not done; unverified, and no real server has been seen
///   to emit such a line.
pub(crate) fn classify(line: &str) -> Result<Inbound, ParseError> {
    let value: Value = serde_json::from_str(line).map_err(ParseError::NotJson)?;
    let obj = value.as_object().ok_or(ParseError::NotObject)?;

    // ★Check before the method branch★ — the contradiction is independent of the envelope kind; a branch picking `result` first would read
    //   a failure as a success.
    if obj.contains_key("result") && obj.contains_key("error") {
        return Err(ParseError::ConflictingEnvelope);
    }

    if let Some(raw_method) = obj.get("method") {
        let method = raw_method
            .as_str()
            .ok_or(ParseError::MethodNotString)?
            .to_string();
        let params = obj.get("params").cloned();
        return match obj.get("id") {
            Some(raw_id) => Ok(Inbound::Request {
                id: parse_id(raw_id)?,
                method,
                params,
            }),
            None => Ok(Inbound::Notification { method, params }),
        };
    }

    if let Some(result) = obj.get("result") {
        let raw_id = obj.get("id").ok_or(ParseError::MissingId("result"))?;
        return Ok(Inbound::Response {
            id: parse_id(raw_id)?,
            result: result.clone(),
        });
    }

    if let Some(raw_error) = obj.get("error") {
        let raw_id = obj.get("id").ok_or(ParseError::MissingId("error"))?;
        let error: RpcError =
            serde_json::from_value(raw_error.clone()).map_err(ParseError::BadError)?;
        return Ok(Inbound::Error {
            id: parse_id(raw_id)?,
            error,
        });
    }

    Err(ParseError::UnknownShape)
}

fn parse_id(raw: &Value) -> Result<RequestId, ParseError> {
    match raw {
        Value::String(s) => Ok(RequestId::Str(s.clone())),
        Value::Number(n) => n.as_i64().map(RequestId::Num).ok_or(ParseError::BadId),
        _ => Err(ParseError::BadId),
    }
}

// ── Method names ───────────────────────────────────────────────────────────────

/// Wire method names. ★No version prefix★ — `v1/`·`v2/` in the schema tree are only source directories; methods go out as bare names like
///   `thread/start`.
pub(crate) mod method {
    // Us → server requests
    pub(crate) const INITIALIZE: &str = "initialize";
    pub(crate) const THREAD_START: &str = "thread/start";
    pub(crate) const THREAD_RESUME: &str = "thread/resume";
    pub(crate) const TURN_START: &str = "turn/start";
    /// Put one input into a running turn — it does not open a turn (ADR-0231).
    pub(crate) const TURN_STEER: &str = "turn/steer";
    pub(crate) const TURN_INTERRUPT: &str = "turn/interrupt";

    /// Receive the past items of a resumed thread **as pages** (ADR-0203).
    ///
    /// ★The twin `thread/turns/list` is not used★ — we draw items, not turns, and that one wraps the same item in one more turn envelope.
    pub(crate) const THREAD_ITEMS_LIST: &str = "thread/items/list";

    /// Us → server notification. ★The only notification a client can send, and it has no params field★ (`ClientNotification.json`).
    /// ★Sending it is not a duty★ (measured 0.154.0: `thread/start` succeeded either way); it is harmless, so we match the handshake shape.
    pub(crate) const INITIALIZED: &str = "initialized";

    // Among server → us notifications, the ones the translator knows by name
    pub(crate) const ITEM_AGENT_MESSAGE_DELTA: &str = "item/agentMessage/delta";
    pub(crate) const ITEM_STARTED: &str = "item/started";
    pub(crate) const ITEM_COMPLETED: &str = "item/completed";
    pub(crate) const THREAD_TOKEN_USAGE_UPDATED: &str = "thread/tokenUsage/updated";
    pub(crate) const ERROR: &str = "error";
    pub(crate) const DEPRECATION_NOTICE: &str = "deprecationNotice";

    /// ★Carried into a usage observation, not a screen event★ — account-level state must not go through per-agent replay (`decoder` arm ·
    ///   interpretation = this folder's `usage`).
    pub(crate) const ACCOUNT_RATE_LIMITS_UPDATED: &str = "account/rateLimits/updated";

    /// Us → server request — only one active usage query is sent (this folder's `usage_probe`). The conversation path does not send this.
    pub(crate) const ACCOUNT_RATE_LIMITS_READ: &str = "account/rateLimits/read";

    /// ★This one name has two consumers★ — the translator carries it into a turn boundary (`decoder`) and the path feeds it to the
    ///   queue-release state machine (`transport`); each reads the same line.
    pub(crate) const TURN_COMPLETED: &str = "turn/completed";

    // Among server → us requests, the ones the path knows by name. ★Refusal is the same for any request★; these two additionally remember
    //   the refused item and attribute that item's
    // end to our refusal (ADR-0241).
    pub(crate) const ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL: &str =
        "item/commandExecution/requestApproval";
    pub(crate) const ITEM_FILE_CHANGE_REQUEST_APPROVAL: &str = "item/fileChange/requestApproval";
}

/// All four values of `TurnStatus` (schema 0.154.0 `definitions.TurnStatus` — a closed `enum`).
///
/// ★Kept as string constants, not a Rust `enum`★: a closed type would fail the **whole** `turn/completed` line the day upstream adds a
///   fifth value, the turn boundary would vanish and
/// the screen's waiting indicator would spin forever — the defect these constants fix.
pub(crate) mod turn_status {
    pub(crate) const COMPLETED: &str = "completed";
    pub(crate) const INTERRUPTED: &str = "interrupted";
    pub(crate) const FAILED: &str = "failed";
    pub(crate) const IN_PROGRESS: &str = "inProgress";

    /// For telling grades apart — a value **not** here is upstream drift; one that is here but not carried into an outcome (today
    ///   [`IN_PROGRESS`] alone) is 「meaning not yet known」. Either way
    /// the translation is still done.
    pub(crate) const ALL: &[&str] = &[COMPLETED, INTERRUPTED, FAILED, IN_PROGRESS];
}

/// The code carried when refusing an inbound **request** we do not know.
///
/// ★The schema fixes no code range★ (`JSONRPCErrorError.code` is an unconstrained i64). We define the envelope side, so we borrow the
///   JSON-RPC 2.0 convention's "method not found", whose
/// meaning a log reader knows.
/// ★The app-server reads this answer to an approval request as 「approval request failed」★ (measured codex-cli 0.156.1 · fixture
///   `refuse_u2a`): it closes the item right away without
/// executing it (command = `failed` · file change = `declined`) and the turn continues to `completed` with no `error` notification. The
///   reaction to a non-approval request is unverified.
/// Either way better than dropping, which freezes that agent permanently (TRD §6-2).
///
/// For reference, **codex itself returns `-32600` for an unknown method** (measured 0.154.0); ours is -32601 for its conventional meaning.
pub(crate) const METHOD_NOT_FOUND: i64 = -32601;

// ── Serialization helpers ───────────────────────────────────────────────────────────────

/// Build one request line. ★The returned string is a **complete line including the newline**★ — appending `\n` again sends an extra empty line.
///
/// A `params` serialization failure is an error (our types cannot fail, but it is generic).
pub(crate) fn request_line<P: Serialize>(
    id: &RequestId,
    method: &str,
    params: &P,
) -> Result<String, serde_json::Error> {
    let mut obj = serde_json::Map::new();
    obj.insert("id".to_string(), serde_json::to_value(id)?);
    obj.insert("method".to_string(), Value::String(method.to_string()));
    obj.insert("params".to_string(), serde_json::to_value(params)?);
    Ok(format!("{}\n", Value::Object(obj)))
}

/// One notification line without params (newline included). `initialized` is the only consumer.
pub(crate) fn notification_line(method: &str) -> String {
    format!("{}\n", serde_json::json!({ "method": method }))
}

/// One error response line that refuses a server request (newline included).
///
/// ★`id` is carried exactly as received★ — it is the server's waiting-ticket key; a normalized id is never released.
/// ★Success must not be disguised★ — a success response to an approval request becomes an automatic approval and bypasses the approval
///   policy (TRD §6-2).
pub(crate) fn error_response_line(id: &RequestId, code: i64, message: &str) -> String {
    format!(
        "{}\n",
        serde_json::json!({
            "id": id,
            "error": { "code": code, "message": message },
        })
    )
}

// ── initialize ────────────────────────────────────────────────────────────────

/// The client name carried in `initialize` (shared by the conversation path · usage query). The peer writes this value into its own log ·
///   `user_agent`.
pub(crate) const CLIENT_NAME: &str = "engram-dashboard";

/// ★Can be sent only once★ (measured 0.154.0) — a second `initialize` returns `-32600` `"Already initialized"`. The caller must keep a
///   retry path from sending it again.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InitializeParams {
    pub(crate) client_info: ClientInfo,
    // ★Not carrying `capabilities` is intentional★ — every field is optional and the defaults are what we want. In particular
    //   `requestAttestation` defaults to false, so the unanswerable
    //   `attestation/generate` request (no refusal shape at all) does not come; enabling it needs code that can bear the consequence.
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ClientInfo {
    pub(crate) name: String,
    pub(crate) version: String,
    // `title` is optional so it is not carried.
}

/// ★No protocol version here★ — `protocolVersion` appears 0 times in the schema; `initialize` negotiates a capability set, which we do not
///   send either.
/// Upstream drift is visible only through `user_agent` and `Thread.cliVersion`, so they are recorded once at startup (TRD §4-7 item 1).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InitializeResponse {
    pub(crate) codex_home: String,
    pub(crate) platform_family: String,
    pub(crate) platform_os: String,
    pub(crate) user_agent: String,
}

// ── thread/start · thread/resume ──────────────────────────────────────────────

/// Approval policy. The schema is `oneOf[3 strings, GranularAskForApproval]` but ★we send only the string branch★ (the granular shape is
///   not copied). The value spelling is
/// kebab-case; the protocol's other approval enums are camelCase.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) enum AskForApproval {
    #[serde(rename = "untrusted")]
    Untrusted,
    #[serde(rename = "on-request")]
    OnRequest,
    #[serde(rename = "never")]
    Never,
}

/// The type of the `sandbox` field of `thread/start`·`thread/resume`.
///
/// ★Do not confuse it with the same-named differently typed fields★ — `TurnStartParams.sandboxPolicy` and `ThreadStartResponse.sandbox`
///   are a structured union (`SandboxPolicy`), not this
/// string enum; the response's `sandbox` cannot be put back into this field.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) enum SandboxMode {
    #[serde(rename = "read-only")]
    ReadOnly,
    #[serde(rename = "workspace-write")]
    WorkspaceWrite,
    #[serde(rename = "danger-full-access")]
    DangerFullAccess,
}

/// ★The schema has no `required` array — `{}` is valid params★. Every field is `Option` + `skip_serializing_if`, or codex gets an explicit
///   `null` where it expected absence.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadStartParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) approval_policy: Option<AskForApproval>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sandbox: Option<SandboxMode>,
    /// Instructions **appended after** codex's default instructions. Absent = nothing appended.
    ///
    /// ★Appending, not replacing★ — codex's default instructions remain (measured 0.155.0: the body stands as a `developer`-role message
    ///   at `input[0]`, default prompt intact).
    /// ★Carries the same thing as terminal mode's `-c developer_instructions=…` but **not the same path**★: this is a JSON body — no
    ///   command-line length cap, no `%VAR%` substitution, no
    ///   line-break truncation — so the value goes **untouched**; do not bring that side's line-break conversion · character guard here
    ///   (it blocks no risk and only ruins the document).
    // ADR-0215
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) developer_instructions: Option<String>,
}

/// ★There is **no** `developerInstructions` counterpart here — the absence is the decision★: whether the field exists in this request's
///   schema is unmeasured, and an unknown field on JSON-RPC
/// params means a 「rejected handshake」 that kills the resume spawn. So **a resumed app-server thread gets no priming** (terminal mode is
///   argv and is carried on resume too; the gap is this
/// branch only). Add it if measurement confirms the field.
// ADR-0215
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadResumeParams {
    pub(crate) thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) approval_policy: Option<AskForApproval>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sandbox: Option<SandboxMode>,
    /// If true, `thread.turns` is not filled in — upstream marks full-history hydration as deprecated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) exclude_turns: Option<bool>,
}

/// The **second request** of the handshake — open a new thread, or resume a saved one.
///
/// ★The one place that chooses is [`crate::backend::AgentBackend::open_spawn`] in `backend/codex/mod.rs`★ — the path emits what it
///   received; a path deciding again from its own state would make
/// two deciders (the shape ADR-0191 withdrew from path selection).
// ADR-0185
pub(crate) enum ThreadOpen {
    Start(ThreadStartParams),
    Resume(ThreadResumeParams),
}

/// ★The thread id lives here — it is `thread.id`, not the response top level★.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ThreadStartResponse {
    pub(crate) thread: Thread,
}

/// The `thread/resume` response — the `thread/start` response plus two more cursors.
///
/// ★One cursor is read — the **entry point** of screen restore★ (ADR-0203). The schema (0.154.0) describes it: "Pass this as `cursor` to
///   `thread/items/list` with
/// `sortDirection: \"desc\"`. The first page includes the item identified by the cursor."
/// ★`default` because the schema is so★ (not in `required`, `default: null`): absent = no history to go back to, **not a failure**
///   (restore skipped).
/// ★The `thread/start` response has no such field★ — a new conversation has nothing to restore.
/// ★The other cursor (`turnsBackwardsCursor`) is still not read★ — see [`method::THREAD_ITEMS_LIST`] for why its twin is unused.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadResumeResponse {
    pub(crate) thread: Thread,
    #[serde(default)]
    pub(crate) items_backwards_cursor: Option<String>,
}

/// Which direction to walk the page — both values of the schema's closed `enum` (0.154.0 `SortDirection`).
///
/// ★[`SortDirection::Asc`] is unused today — written anyway★: this is an **outgoing** field (values cannot be invented), so a
///   direction-reversing change adds no new constant.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) enum SortDirection {
    #[serde(rename = "asc")]
    Asc,
    #[serde(rename = "desc")]
    Desc,
}

/// `thread/items/list` request. ★Only `threadId` is required★ (schema 0.154.0); the rest are `Option` + `skip_serializing_if` (same reason
///   as [`ThreadStartParams`]).
/// ★`turnId` (narrows to one turn) is not copied★ — we walk the whole thread from the end.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadItemsListParams {
    pub(crate) thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sort_direction: Option<SortDirection>,
}

/// One page.
///
/// ★`nextCursor` absent or `null` = nothing more in that direction★ — the schema says so and measurement agrees (`null` on the last page,
///   2026-09-16, real 0.154.0, a 28-item thread = 25+3).
/// ★`backwardsCursor` (for reversing direction) is not copied★ — we walk one direction only.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadItemsListResponse {
    pub(crate) data: Vec<ThreadItemEntry>,
    #[serde(default)]
    pub(crate) next_cursor: Option<String>,
}

/// One item carried in a page + the turn it belongs to.
///
/// ★`item` is the **same type** as that of [`ItemNotification`] — the ground of ADR-0203★: the vendor schema's item definition hash is
///   identical for the `item/completed` notification ·
/// resume response · page response (19 variants), so we only peel the envelope instead of writing a new translator. [`Value`] for the same
///   reason as there: one unknown variant must not kill
/// the **whole** page.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadItemEntry {
    pub(crate) turn_id: String,
    pub(crate) item: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Thread {
    /// Issued by codex — ★the caller cannot decide it★. A UUIDv7 string, and `thread.sessionId` holds the same value (measured 0.154.0);
    ///   at `result.thread.id`, not the response top level.
    pub(crate) id: String,
    /// ★Required in the schema but absence is allowed★ — failing the whole thread start over one diagnostic value is worse; it is the
    ///   value that tells upstream drift after the fact.
    #[serde(default)]
    pub(crate) cli_version: Option<String>,
}

// ── turn/start · turn/interrupt ───────────────────────────────────────────────

/// One piece of turn input. The schema has 7 variants (text·image·localImage·audio·localAudio·skill·mention) but ★we send only `text`★ and
///   did not copy the rest.
///
/// ★Not carrying `text_elements` is intentional★ — optional with default `[]`, and the only snake_case spelling in this camelCase type;
///   write it with that spelling if needed.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub(crate) enum UserInput {
    #[serde(rename = "text")]
    Text { text: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnStartParams {
    pub(crate) thread_id: String,
    pub(crate) input: Vec<UserInput>,
    /// Our id that comes back as `clientId` on the echo (`userMessage`) of the first input (since 0.140.0 — measured 0.156.1).
    /// ★`None` → the field is not carried★ — envelopes of inputs without an identifier and of incarnations below the minimum codex version
    ///   are byte-for-byte as before this field existed.
    // ADR-0231
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) client_user_message_id: Option<String>,
}

/// `turn/steer` — put one input into a running turn. The success response `{turnId}` is **acceptance**, not a receipt (the receipt is the
///   echo carrying `clientId` — measured 0.156.1);
/// the body is not read.
///
/// ★`expectedTurnId` is a vendor-required precondition★ — an ended turn or mismatched id returns an error (`-32600 "no active turn to
///   steer"`), so a late steer never mixes into the next turn.
/// ★`clientUserMessageId` is deliberately not an `Option`★ — optional in the schema, but steer is sent only for entries with an identifier
///   from an incarnation meeting the minimum codex version
///   (without one the handed-over text cannot be matched to the echo).
// ADR-0231
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnSteerParams {
    pub(crate) thread_id: String,
    pub(crate) expected_turn_id: String,
    pub(crate) client_user_message_id: String,
    pub(crate) input: Vec<UserInput>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct TurnStartResponse {
    pub(crate) turn: Turn,
}

/// Turn — ★reads the `turn/start` **response** only, not the `turn/completed` notification★: that `turn` is the same schema type, but the
///   translator scans it directly as a [`Value`]
/// (reason: the `turn/completed` spot of [`crate::backend::codex::decoder`] — one off field must not stop the turn boundary standing).
/// Only the id for `turn/interrupt` is needed, so the other fields
///   (`status`·`items`·`error`·`startedAt`·`completedAt`·`durationMs`·`itemsView`) are not copied.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Turn {
    pub(crate) id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnInterruptParams {
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
}

/// The result of `turn/interrupt` — ★a truly empty object (neither properties nor required in the schema)★. We **only receive** it (our
///   only emitted response is the refusal error).
///
/// A braced struct is intentional — a unit struct serializes as `null`, not `{}`. No line goes out this way now; it is pinned so the shape
///   does not change quietly if this type is ever
/// carried outbound (the regression net measures that serialization).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct TurnInterruptResponse {}

// ── params of the notifications we translate ────────────────────────────────────────────────────

/// `item/agentMessage/delta` — the assistant body stream. All four fields are required and we read three of them.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentMessageDeltaNotification {
    pub(crate) turn_id: String,
    pub(crate) item_id: String,
    pub(crate) delta: String,
}

/// The common read fields of `item/started` · `item/completed`.
///
/// ★The two differ only in `startedAtMs` vs `completedAtMs`; the `item` union is the same★ — one type, branched by `method`.
/// `item` is a [`Value`], not a type, intentionally: an internally-tagged enum meeting an unknown `type` fails **the whole line**, and
///   this union has 19 variants and keeps growing.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ItemNotification {
    pub(crate) turn_id: String,
    pub(crate) item: Value,
}

/// `thread/tokenUsage/updated`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadTokenUsageUpdatedNotification {
    pub(crate) turn_id: String,
    pub(crate) token_usage: ThreadTokenUsage,
}

/// ★The schema does not explain what `last` and `total` count★ (no description, both required). Read by name: `last` = the model request
///   that just ended, `total` = cumulative; the translator
/// uses `last` (reason: the usage translation spot of [`crate::backend::codex::decoder`]).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ThreadTokenUsage {
    pub(crate) last: TokenUsageBreakdown,
}

/// ★Our `Usage` vocabulary has only two fields, so the rest is lost★
///   (`cachedInputTokens`·`reasoningOutputTokens`·`totalTokens`·`cacheWriteInputTokens`); growing it is a wire change, out of scope.
///
/// ★Signed `i64` because the schema declares `format: int64`★ — `u64` would fail **the whole notification** on one negative or
///   out-of-range value and lose that turn's `Usage`. Our vocabulary
/// field is `u64`, so a negative is clamped to 0 at the translation spot (its comment is canonical).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TokenUsageBreakdown {
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
}

/// `error` notification.
///
/// ★`threadId`·`turnId`·`willRetry` are required in the schema but absence is allowed★ — dropping an undecodable error whole is the worst
///   outcome in this protocol; pass it as long as the error
/// body is present.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ErrorNotification {
    pub(crate) error: TurnError,
    #[serde(default)]
    pub(crate) will_retry: bool,
}

/// ★Two fields carry this type★ — `error` of the `error` notification, and `Turn.error` (schema: 「Only populated when the Turn's status is
///   failed」, 0.154.0). A failed turn's reason is read by this
/// one type, and one place makes the screen string.
///
/// ★`codexErrorInfo` is a [`Value`] intentionally★ — a union mixing 13 string enums and 5 wrapper objects (`oneOf` members 6), of which we
///   use one human-readable label. It holds
/// `serverOverloaded`·`rateLimitExceeded` — the schema's only backpressure vocabulary, which comes via **this notification**, not a
///   numeric error code.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TurnError {
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) additional_details: Option<String>,
    #[serde(default)]
    pub(crate) codex_error_info: Option<Value>,
    #[serde(default)]
    pub(crate) misalignment: Option<MisalignmentErrorDetails>,
}

/// Detail for a turn blocked by a model-alignment violation. ★`message` alone cannot tell "why it was blocked" or "what to do"★ — these
///   three carry that.
///
/// The schema has no `required` array — all three fields are optional and nullable.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MisalignmentErrorDetails {
    /// An already localized explanation comes (schema description: "A substantive localized explanation").
    #[serde(default)]
    pub(crate) detailed_explanation: Option<String>,
    /// ★Not a closed enum★ — the schema leaves it open ("clients must accept categories added by Responses"), so it is received as a string.
    #[serde(default)]
    pub(crate) error_type: Option<String>,
    /// If we decide to continue, the **sentence to submit as-is as the next turn input** — given by codex, not invented by us.
    #[serde(default)]
    pub(crate) steer: Option<MisalignmentSteer>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MisalignmentSteer {
    pub(crate) message: String,
}

// ── params of the usage limit notification ─────────────────────────────────────────────────

/// `account/rateLimits/updated`. ★This section was read from the schema of **codex-cli 0.156.1**, not 0.154.0★
///   (`v2/AccountRateLimitsUpdatedNotification.json`; regeneration: this file's header).
///
/// ★One notification = a sparse update of **one** limit bucket★, told apart by [`RateLimitSnapshot::limit_id`]. Schema description: 「merge
///   into the most recent account/rateLimits/read response —
/// an empty value does not erase the earlier value」. Arrives mid-turn (`docs/process/S21-codex-backend/trd-phase2a.md` L7); no carried
///   value has ever been captured.
///
/// ★Params and `rateLimits` that are not JSON **objects** fail deserialization ([`object_only`])★ — unlike the two field-lenient types
///   below, the line is dropped whole: with a different overall
///   shape there is no field to save.
#[derive(Debug, Clone)]
pub(crate) struct AccountRateLimitsUpdatedNotification {
    pub(crate) rate_limits: RateLimitSnapshot,
}

impl<'de> Deserialize<'de> for AccountRateLimitsUpdatedNotification {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Params {
            #[serde(deserialize_with = "object_only")]
            rate_limits: RateLimitSnapshot,
        }
        let params: Params = object_only(de)?;
        Ok(Self {
            rate_limits: params.rate_limits,
        })
    }
}

/// A snapshot of one limit bucket (schema `RateLimitSnapshot` — all fields optional · nullable).
///
/// ★Each field fails separately★ — a wrong-typed field becomes `None` alone and the snapshot survives ([`lenient`]·[`lenient_object`]);
///   one field's shape change must not lose the bucket's other values.
/// ★Only `limit_id` is a raw [`Value`] intentionally★ — it says 「which bucket」, and a wrong type folded into `None` would be
///   indistinguishable from absence (read as the default bucket). The consumer interprets.
/// ★This type would read even a JSON array in field order★ — the object requirement is carried by the [`object_only`] of the carrying side
///   (the notification · [`GetAccountRateLimitsResponse`]);
///   a new read path must pass that gate too, or a plausible fake window appears.
/// `limit_name`·`plan_type` are read only by the query response (pickup carries no plan · per-model windows — this folder's `usage`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RateLimitSnapshot {
    #[serde(default)]
    pub(crate) limit_id: Option<Value>,
    #[serde(default, deserialize_with = "lenient")]
    pub(crate) limit_name: Option<String>,
    #[serde(default, deserialize_with = "lenient_object")]
    pub(crate) primary: Option<RateLimitWindow>,
    #[serde(default, deserialize_with = "lenient_object")]
    pub(crate) secondary: Option<RateLimitWindow>,
    /// The schema is a closed enum (`PlanType`) but received as a string — when upstream adds a value, only this field needs reading.
    #[serde(default, deserialize_with = "lenient")]
    pub(crate) plan_type: Option<String>,
}

/// One limit window (schema `RateLimitWindow`: `usedPercent` int32 (required) · `windowDurationMins` int64 · `resetsAt` int64 (epoch
///   seconds) — no range constraints; the consumer decides range).
///
/// ★`primary`/`secondary` are slot names, not 5-hour/weekly★ (measured — the consumer tells them apart by `window_duration_mins`).
/// ★`used_percent` as `f64` is intentional★ — the schema says integer but there is no reason to reject a fraction; the reset time accepts
///   integers only (no float → integer conversion of external values).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RateLimitWindow {
    #[serde(default, deserialize_with = "lenient")]
    pub(crate) used_percent: Option<f64>,
    #[serde(default, deserialize_with = "lenient")]
    pub(crate) window_duration_mins: Option<i64>,
    #[serde(default, deserialize_with = "lenient")]
    pub(crate) resets_at: Option<i64>,
}

// ── account/rateLimits/read ───────────────────────────────────────────────────

/// The params of `account/rateLimits/read` — ★no field carried (`{}`)★. The schema (0.156.1 `v2/NullableGetAccountRateLimitsParams.json`)
///   accepts `null` or an all-optional object; the probe
/// measured with `{}` (`.claude/handoff/attachments/codex-usage-probe.mjs`).
/// ★`excludeResetCreditDetails` is not carried to keep the measured shape★ — its description (「for background polling — skips the reset
///   credit detail query」) fits, but a request with it is unmeasured.
///
/// A braced struct for the same reason as [`TurnInterruptResponse`] — a unit struct serializes as `null`.
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct GetAccountRateLimitsParams {}

/// The `account/rateLimits/read` response (schema 0.156.1 `v2/GetAccountRateLimitsResponse.json`) — two fields read.
///
/// ★Not carrying `accountId` is a decision★ — it sits at the response top level; unread, it cannot leak into logs · error strings (TRD
///   §1-3). If needed (account key — TRD §3 #33), decide its exit first.
/// ★Snapshot slots accept objects only ([`object_only`])★ — a non-object response fails deserialization; a non-object snapshot slot
///   (`rateLimits` · each map entry) is treated as absent for **that slot only**.
#[derive(Debug, Clone)]
pub(crate) struct GetAccountRateLimitsResponse {
    /// Schema: "Backward-compatible single-bucket view" — ★not guaranteed to be the default bucket★. `None` = absent, `null`, or not an object.
    pub(crate) rate_limits: Option<RateLimitSnapshot>,
    /// `rateLimitsByLimitId` — bucket id → snapshot, **sorted by id** (JSON map order varies with serde_json features). `None` = absent,
    ///   `null`, or not an object.
    pub(crate) rate_limits_by_limit_id: Option<Vec<(String, RateLimitSnapshot)>>,
}

impl<'de> Deserialize<'de> for GetAccountRateLimitsResponse {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Raw {
            #[serde(default)]
            rate_limits: Option<Value>,
            #[serde(default)]
            rate_limits_by_limit_id: Option<Value>,
        }
        let raw: Raw = object_only(de)?;
        let snapshot = |value: Value| object_only::<_, RateLimitSnapshot>(value).ok();
        let by_limit_id = match raw.rate_limits_by_limit_id {
            Some(Value::Object(map)) => {
                let mut buckets: Vec<(String, RateLimitSnapshot)> = map
                    .into_iter()
                    .filter_map(|(id, value)| Some((id, snapshot(value)?)))
                    .collect();
                buckets.sort_by(|a, b| a.0.cmp(&b.0));
                Some(buckets)
            }
            _ => None,
        };
        Ok(Self {
            rate_limits: raw.rate_limits.and_then(snapshot),
            rate_limits_by_limit_id: by_limit_id,
        })
    }
}

/// Read one field, and if its shape is wrong (including `null`) put **that field only** as `None`. A missing field is handled by
///   `#[serde(default)]`.
fn lenient<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Value::deserialize(de)?;
    Ok(serde_json::from_value(value).ok())
}

/// Same as [`lenient`] but accepts only JSON **objects**.
/// ★Do not remove the object check★ — serde struct deserialization also accepts arrays, filling in field order; an array at a window slot
///   would read as utilization · window length · reset and become a
///   plausible fake window.
fn lenient_object<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Value::deserialize(de)?;
    Ok(if value.is_object() {
        serde_json::from_value(value).ok()
    } else {
        None
    })
}

/// Read only JSON **objects**; anything else is an error. Same ground as [`lenient_object`] — an array is read in field order and becomes
///   a plausible fake value (params `[{…snapshot…}]` · a snapshot
/// `["codex", {…window…}, {…window…}]`).
fn object_only<'de, D, T>(de: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Value::deserialize(de)?;
    if !value.is_object() {
        return Err(serde::de::Error::custom("not a JSON object"));
    }
    serde_json::from_value(value).map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
        v.get(key)
            .unwrap_or_else(|| panic!("field `{key}` is missing: {v}"))
    }

    // ── Envelope discrimination ────────────────────────────────────────────────────────────

    #[test]
    fn notification_has_method_and_no_id() {
        let line = r#"{"method":"item/agentMessage/delta","params":{"delta":"hi"}}"#;
        match classify(line).expect("must read as a notification") {
            Inbound::Notification { method, params } => {
                assert_eq!(method, "item/agentMessage/delta");
                assert_eq!(field(&params.unwrap(), "delta"), "hi");
            }
            other => panic!("not a notification: {other:?}"),
        }
    }

    #[test]
    fn method_plus_id_is_a_request_never_a_notification() {
        let line = r#"{"id":7,"method":"item/fileChange/requestApproval","params":{}}"#;
        match classify(line).expect("must read as a request") {
            Inbound::Request { id, method, .. } => {
                assert_eq!(id, RequestId::Num(7));
                assert_eq!(method, "item/fileChange/requestApproval");
            }
            other => panic!("not a request: {other:?}"),
        }
    }

    #[test]
    fn id_plus_result_is_a_response_never_a_request() {
        let line = r#"{"id":"abc","result":{"thread":{"id":"t-1"}}}"#;
        match classify(line).expect("must read as a response") {
            Inbound::Response { id, result } => {
                assert_eq!(id, RequestId::Str("abc".to_string()));
                assert_eq!(field(field(&result, "thread"), "id"), "t-1");
            }
            other => panic!("not a response: {other:?}"),
        }
    }

    #[test]
    fn id_plus_error_is_an_error_envelope() {
        let line = r#"{"id":3,"error":{"code":-1,"message":"nope"}}"#;
        match classify(line).expect("must read as an error") {
            Inbound::Error { id, error } => {
                assert_eq!(id, RequestId::Num(3));
                assert_eq!(error.code, -1);
                assert_eq!(error.message, "nope");
                assert!(error.data.is_none());
            }
            other => panic!("not an error envelope: {other:?}"),
        }
    }

    #[test]
    fn string_ids_survive_verbatim() {
        let line = r#"{"id":"req-42","method":"x","params":null}"#;
        match classify(line).unwrap() {
            Inbound::Request { id, .. } => assert_eq!(id, RequestId::Str("req-42".to_string())),
            other => panic!("not a request: {other:?}"),
        }
    }

    #[test]
    fn non_json_is_an_error_not_a_panic() {
        assert!(matches!(
            classify("Warning: something on stderr"),
            Err(ParseError::NotJson(_))
        ));
    }

    #[test]
    fn json_that_is_not_an_object_is_an_error() {
        assert!(matches!(classify("[1,2,3]"), Err(ParseError::NotObject)));
        assert!(matches!(classify("\"bare\""), Err(ParseError::NotObject)));
    }

    #[test]
    fn json_object_fitting_no_envelope_is_an_error() {
        assert!(matches!(
            classify(r#"{"hello":"world"}"#),
            Err(ParseError::UnknownShape)
        ));
    }

    #[test]
    fn result_and_error_together_is_an_error_not_a_response() {
        assert!(matches!(
            classify(r#"{"id":7,"result":{},"error":{"code":-1,"message":"failed"}}"#),
            Err(ParseError::ConflictingEnvelope)
        ));
        // Even if method is present, the contradiction wins — it does not depend on branch order.
        assert!(matches!(
            classify(r#"{"method":"x","result":{},"error":{"code":-1,"message":"f"}}"#),
            Err(ParseError::ConflictingEnvelope)
        ));
    }

    #[test]
    fn result_without_id_is_an_error() {
        assert!(matches!(
            classify(r#"{"result":{}}"#),
            Err(ParseError::MissingId("result"))
        ));
    }

    #[test]
    fn non_string_method_is_an_error() {
        assert!(matches!(
            classify(r#"{"method":42,"id":1}"#),
            Err(ParseError::MethodNotString)
        ));
    }

    #[test]
    fn non_scalar_id_is_an_error() {
        assert!(matches!(
            classify(r#"{"method":"x","id":{"nested":true}}"#),
            Err(ParseError::BadId)
        ));
    }

    /// Measured 0.154.0 — notification envelopes always carry `emittedAtMs`; wobbling discrimination on the extra sibling field would drop
    ///   normal notifications whole.
    #[test]
    fn an_extra_sibling_key_does_not_break_classification() {
        let line =
            r#"{"method":"item/started","params":{"turnId":"u"},"emittedAtMs":1764000000000}"#;
        match classify(line).expect("must read as a notification") {
            Inbound::Notification { method, .. } => assert_eq!(method, "item/started"),
            other => panic!("not a notification: {other:?}"),
        }
    }

    /// Measured 0.154.0 — responses carry several fields not in the schema; rejecting unknown fields would fail a normal response.
    #[test]
    fn unknown_fields_on_a_response_are_tolerated() {
        let v = serde_json::json!({
            "thread": {"id": "t-1", "historyMode": "compact", "extra": {"a": 1}},
            "environments": [], "runtimeWorkspaceRoots": [], "canAcceptDirectInput": true,
            "daybreakEnabled": false, "multiAgentMode": "off", "activePermissionProfile": null
        });
        let r: ThreadStartResponse = serde_json::from_value(v).unwrap();
        assert_eq!(r.thread.id, "t-1");
    }

    #[test]
    fn missing_params_is_not_an_error() {
        match classify(r#"{"method":"thread/closed"}"#).unwrap() {
            Inbound::Notification { params, .. } => assert!(params.is_none()),
            other => panic!("not a notification: {other:?}"),
        }
    }

    // ── Serialization helpers ──────────────────────────────────────────────────────────

    #[test]
    fn request_line_is_one_newline_terminated_line_that_classifies_back() {
        let params = TurnInterruptParams {
            thread_id: "t-1".to_string(),
            turn_id: "u-9".to_string(),
        };
        let line = request_line(&RequestId::Num(5), method::TURN_INTERRUPT, &params).unwrap();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        match classify(line.trim_end()).unwrap() {
            Inbound::Request { id, method, params } => {
                assert_eq!(id, RequestId::Num(5));
                assert_eq!(method, "turn/interrupt");
                let p = params.unwrap();
                assert_eq!(field(&p, "threadId"), "t-1");
                assert_eq!(field(&p, "turnId"), "u-9");
            }
            other => panic!("not a request: {other:?}"),
        }
    }

    #[test]
    fn request_line_carries_no_jsonrpc_field() {
        let line = request_line(
            &RequestId::Str("a".to_string()),
            method::TURN_INTERRUPT,
            &TurnInterruptParams {
                thread_id: "t".to_string(),
                turn_id: "u".to_string(),
            },
        )
        .unwrap();
        assert!(!line.contains("jsonrpc"), "{line}");
    }

    #[test]
    fn notification_line_has_method_only() {
        let line = notification_line(method::INITIALIZED);
        assert!(line.ends_with('\n'));
        let v: Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(field(&v, "method"), "initialized");
        assert!(v.get("params").is_none());
        assert!(v.get("id").is_none());
    }

    #[test]
    fn error_response_line_echoes_the_id_verbatim() {
        let line = error_response_line(
            &RequestId::Str("srv-1".to_string()),
            METHOD_NOT_FOUND,
            "unsupported",
        );
        assert!(line.ends_with('\n'));
        match classify(line.trim_end()).unwrap() {
            Inbound::Error { id, error } => {
                assert_eq!(id, RequestId::Str("srv-1".to_string()));
                assert_eq!(error.code, -32601);
                assert_eq!(error.message, "unsupported");
            }
            other => panic!("not an error envelope: {other:?}"),
        }
    }

    /// Measured 0.154.0 — a request with `id: null` is **reclassified as a notification and silently dropped** (no error, no stderr). Our
    ///   id must never become `null` by any path.
    #[test]
    fn ids_never_serialize_as_null() {
        for id in [RequestId::Str(String::new()), RequestId::Num(0)] {
            assert!(!serde_json::to_value(&id).unwrap().is_null());
            let req = request_line(&id, method::INITIALIZE, &serde_json::json!({})).unwrap();
            let v: Value = serde_json::from_str(req.trim_end()).unwrap();
            assert!(!field(&v, "id").is_null(), "{req}");
            let err = error_response_line(&id, METHOD_NOT_FOUND, "m");
            let v: Value = serde_json::from_str(err.trim_end()).unwrap();
            assert!(!field(&v, "id").is_null(), "{err}");
        }
    }

    #[test]
    fn error_response_line_keeps_numeric_ids_numeric() {
        let line = error_response_line(&RequestId::Num(9_007_199_254_740_993), 1, "m");
        let v: Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(field(&v, "id").as_i64(), Some(9_007_199_254_740_993));
    }

    // ── Wire spelling of outbound params ─────────────────────────────────────

    #[test]
    fn initialize_params_field_names() {
        let v = serde_json::to_value(InitializeParams {
            client_info: ClientInfo {
                name: "engram-dashboard".to_string(),
                version: "0.1.0".to_string(),
            },
        })
        .unwrap();
        let info = field(&v, "clientInfo");
        assert_eq!(field(info, "name"), "engram-dashboard");
        assert_eq!(field(info, "version"), "0.1.0");
        assert!(v.get("capabilities").is_none());
    }

    #[test]
    fn thread_start_params_omit_absent_fields_entirely() {
        let v = serde_json::to_value(ThreadStartParams::default()).unwrap();
        assert_eq!(v, serde_json::json!({}));
    }

    #[test]
    fn thread_start_params_field_names_and_enum_spellings() {
        let v = serde_json::to_value(ThreadStartParams {
            cwd: Some("C:/w".to_string()),
            approval_policy: Some(AskForApproval::Never),
            sandbox: Some(SandboxMode::WorkspaceWrite),
            developer_instructions: Some("be brief".to_string()),
        })
        .unwrap();
        assert_eq!(field(&v, "cwd"), "C:/w");
        assert_eq!(field(&v, "approvalPolicy"), "never");
        assert_eq!(field(&v, "sandbox"), "workspace-write");
        // ★Going out as camelCase is the contract★ — snake_case leaks as an unknown field codex silently drops (symptom: 「priming has no
        //   effect」, no error).
        assert_eq!(field(&v, "developerInstructions"), "be brief");
    }

    #[test]
    fn approval_and_sandbox_enums_use_the_schema_spellings() {
        assert_eq!(
            serde_json::to_value(AskForApproval::OnRequest).unwrap(),
            "on-request"
        );
        assert_eq!(
            serde_json::to_value(AskForApproval::Untrusted).unwrap(),
            "untrusted"
        );
        assert_eq!(
            serde_json::to_value(SandboxMode::ReadOnly).unwrap(),
            "read-only"
        );
        assert_eq!(
            serde_json::to_value(SandboxMode::DangerFullAccess).unwrap(),
            "danger-full-access"
        );
    }

    #[test]
    fn thread_resume_params_field_names() {
        let v = serde_json::to_value(ThreadResumeParams {
            thread_id: "t-1".to_string(),
            cwd: None,
            approval_policy: None,
            sandbox: None,
            exclude_turns: Some(true),
        })
        .unwrap();
        assert_eq!(field(&v, "threadId"), "t-1");
        assert_eq!(field(&v, "excludeTurns"), true);
        assert!(v.get("cwd").is_none());
        assert!(v.get("approvalPolicy").is_none());
        assert!(v.get("sandbox").is_none());
    }

    #[test]
    fn turn_start_params_field_names_and_input_tagging() {
        let v = serde_json::to_value(TurnStartParams {
            thread_id: "t-1".to_string(),
            input: vec![UserInput::Text {
                text: "hello".to_string(),
            }],
            client_user_message_id: None,
        })
        .unwrap();
        assert_eq!(field(&v, "threadId"), "t-1");
        let item = &field(&v, "input").as_array().unwrap()[0];
        assert_eq!(field(item, "type"), "text");
        assert_eq!(field(item, "text"), "hello");
        assert!(
            v.get("clientUserMessageId").is_none(),
            "a field appeared on an envelope without an identifier: {v}"
        );
    }

    // ADR-0231
    #[test]
    fn turn_start_params_carry_the_client_message_id_under_its_wire_name() {
        let v = serde_json::to_value(TurnStartParams {
            thread_id: "t-1".to_string(),
            input: vec![UserInput::Text {
                text: "hello".to_string(),
            }],
            client_user_message_id: Some("u-1".to_string()),
        })
        .unwrap();
        assert_eq!(field(&v, "clientUserMessageId"), "u-1");
    }

    // ADR-0231
    #[test]
    fn turn_steer_params_field_names() {
        let v = serde_json::to_value(TurnSteerParams {
            thread_id: "t-1".to_string(),
            expected_turn_id: "u-9".to_string(),
            client_user_message_id: "c-1".to_string(),
            input: vec![UserInput::Text {
                text: "more".to_string(),
            }],
        })
        .unwrap();
        assert_eq!(field(&v, "threadId"), "t-1");
        assert_eq!(field(&v, "expectedTurnId"), "u-9");
        assert_eq!(field(&v, "clientUserMessageId"), "c-1");
        let item = &field(&v, "input").as_array().unwrap()[0];
        assert_eq!(field(item, "type"), "text");
        assert_eq!(field(item, "text"), "more");
        assert_eq!(
            v.as_object().unwrap().len(),
            4,
            "an unknown field was carried: {v}"
        );
    }

    #[test]
    fn turn_interrupt_params_field_names() {
        let v = serde_json::to_value(TurnInterruptParams {
            thread_id: "t-1".to_string(),
            turn_id: "u-1".to_string(),
        })
        .unwrap();
        assert_eq!(field(&v, "threadId"), "t-1");
        assert_eq!(field(&v, "turnId"), "u-1");
    }

    #[test]
    fn turn_interrupt_response_serializes_as_an_empty_object_not_null() {
        let s = serde_json::to_string(&TurnInterruptResponse::default()).unwrap();
        assert_eq!(s, "{}");
    }

    // ── Inbound payload ─────────────────────────────────────────────────────

    #[test]
    fn thread_start_response_reads_the_nested_thread_id() {
        let v = serde_json::json!({
            "thread": {"id": "0199-7ab", "sessionId": "0199-7ab", "cliVersion": "0.154.0",
                       "cwd": "C:/w", "createdAt": 1, "updatedAt": 2, "ephemeral": false,
                       "modelProvider": "openai", "preview": "", "projectId": null,
                       "source": {}, "status": {"type": "idle"}, "turns": []},
            "cwd": "C:/w", "model": "gpt-5", "modelProvider": "openai",
            "approvalPolicy": "never", "approvalsReviewer": "user",
            "sandbox": {"type": "dangerFullAccess"}
        });
        let r: ThreadStartResponse = serde_json::from_value(v).unwrap();
        assert_eq!(r.thread.id, "0199-7ab");
        assert_eq!(r.thread.cli_version.as_deref(), Some("0.154.0"));
    }

    #[test]
    fn thread_survives_a_missing_cli_version() {
        let t: Thread = serde_json::from_value(serde_json::json!({"id": "x"})).unwrap();
        assert_eq!(t.id, "x");
        assert!(t.cli_version.is_none());
    }

    #[test]
    fn turn_start_response_reads_the_nested_turn_id() {
        let v = serde_json::json!({"turn": {"id": "u-7", "items": [], "status": "inProgress"}});
        let r: TurnStartResponse = serde_json::from_value(v).unwrap();
        assert_eq!(r.turn.id, "u-7");
    }

    #[test]
    fn initialize_response_reads_all_four_required_fields() {
        let v = serde_json::json!({
            "codexHome": "C:/Users/x/.codex", "platformFamily": "windows",
            "platformOs": "windows", "userAgent": "codex/0.154.0"
        });
        let r: InitializeResponse = serde_json::from_value(v).unwrap();
        assert_eq!(r.user_agent, "codex/0.154.0");
        assert_eq!(r.platform_os, "windows");
        assert_eq!(r.platform_family, "windows");
        assert_eq!(r.codex_home, "C:/Users/x/.codex");
    }

    #[test]
    fn error_notification_reads_with_only_the_message_present() {
        let n: ErrorNotification =
            serde_json::from_value(serde_json::json!({"error": {"message": "boom"}})).unwrap();
        assert_eq!(n.error.message, "boom");
        assert!(!n.will_retry);
        assert!(n.error.codex_error_info.is_none());
    }

    /// If a field the schema declares as `int64` is narrowed to `u64`, this notification vanishes whole.
    #[test]
    fn token_counts_accept_the_full_declared_int64_range() {
        let n: ThreadTokenUsageUpdatedNotification = serde_json::from_value(serde_json::json!({
            "threadId": "t", "turnId": "u",
            "tokenUsage": {
                "last": {"inputTokens": -1, "cachedInputTokens": 0, "outputTokens": 9_007_199_254_740_993i64,
                         "reasoningOutputTokens": 0, "totalTokens": 0},
                "total": {"inputTokens": 0, "cachedInputTokens": 0, "outputTokens": 0,
                          "reasoningOutputTokens": 0, "totalTokens": 0}
            }
        }))
        .unwrap();
        assert_eq!(n.token_usage.last.input_tokens, -1);
        assert_eq!(n.token_usage.last.output_tokens, 9_007_199_254_740_993);
    }

    #[test]
    fn misalignment_details_survive_the_read() {
        let n: ErrorNotification = serde_json::from_value(serde_json::json!({
            "error": {
                "message": "blocked",
                "misalignment": {
                    "detailedExplanation": "this request is outside the policy",
                    "errorType": "somethingBrandNew",
                    "steer": {"message": "ask it differently"}
                }
            }
        }))
        .unwrap();
        let m = n.error.misalignment.expect("misalignment must be read");
        assert_eq!(
            m.detailed_explanation.as_deref(),
            Some("this request is outside the policy")
        );
        assert_eq!(m.error_type.as_deref(), Some("somethingBrandNew"));
        assert_eq!(m.steer.unwrap().message, "ask it differently");
    }

    #[test]
    fn misalignment_is_optional_and_its_members_are_too() {
        let n: ErrorNotification = serde_json::from_value(serde_json::json!({
            "error": {"message": "m", "misalignment": {}}
        }))
        .unwrap();
        let m = n.error.misalignment.unwrap();
        assert!(m.detailed_explanation.is_none() && m.error_type.is_none() && m.steer.is_none());
    }

    #[test]
    fn error_notification_keeps_the_codex_error_label() {
        let n: ErrorNotification = serde_json::from_value(serde_json::json!({
            "threadId": "t", "turnId": "u", "willRetry": true,
            "error": {"message": "overloaded", "codexErrorInfo": "serverOverloaded"}
        }))
        .unwrap();
        assert!(n.will_retry);
        assert_eq!(
            n.error.codex_error_info.unwrap().as_str(),
            Some("serverOverloaded")
        );
    }
}
