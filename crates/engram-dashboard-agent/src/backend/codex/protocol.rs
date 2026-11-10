//! codex app-server wire types — the envelope · method names · the payloads we send and receive.
//!
//! ★Source and how to regenerate (do not infer again)★: the field names · required-ness · enum values in this file were
//!   read from the JSON Schema that codex-cli **0.154.0** emitted — ★one exception = the usage limit
//!   section (`AccountRateLimitsUpdatedNotification`·`GetAccountRateLimits*` and the types under them) was read from the **0.156.1** schema★.
//!   The command to extract it again =
//!   `codex app-server generate-json-schema --out <dir>` — 266 per-type files
//!   land under `v2/`. To check whether upstream changed, do not read this file but extract again with that command and compare.
//!
//! ★The envelope is not JSON-RPC 2.0★ — the `jsonrpc` field appears 0 times in the whole schema. Do not send it and
//!   do not require it on receipt.
//!
//! ★Inbound structs carry **only the fields we read**★ — serde ignores unknown fields, so copying even the required fields
//!   we do not read gains nothing and only adds deserialization failure points. Conversely **outbound** carries every field
//!   the schema marks required — if missing, the server rejects it.
//!
//! ★`deny_unknown_fields` is attached nowhere — it is a correctness requirement, not taste★ (measured 0.154.0):
//!   real responses carry fields that are **not in the schema at all**. In the `thread/start` response alone, what was observed is
//!   `environments`·`runtimeWorkspaceRoots`·`extra`·`canAcceptDirectInput`·`daybreakEnabled`
//!   ·`multiAgentMode`·`activePermissionProfile`·`historyMode`. The moment unknown fields are rejected,
//!   a normal response becomes a decode failure.
//!
//! ★A crate-wide `rename_all` is not used★ — this protocol mixes casing within itself
//!   (`input_text` alongside `inputText`, `approved_for_session` alongside `acceptForSession`).
//!   `rename_all` is attached **only to types whose fields were all confirmed camelCase in the schema**.
//!
//! This file does no I/O — string in, type out.

// chunk 1 only sets up the types and the translator and does no wiring at all. The consumer of the surface below (the path implementation)
//   is made by chunk 2, and this allow is removed then.
#![allow(dead_code)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Envelope ──────────────────────────────────────────────────────────────────────

/// JSON-RPC id. The schema is `anyOf[string, i64]` and lists string first (`RequestId.json`).
///
/// ★An id the server issued must be returned **character for character**★ — so it is not normalized to a number.
/// Being i64 is also intentional: the schema is `format: int64`, so a JS-side peer may send an id above 2^53.
///
/// ★That there is no variant that can serialize as `null` is the invariant of this type★ (measured 0.154.0):
/// if `id: null` is sent, the server's untagged envelope union **reclassifies that line as a notification and quietly
/// drops it** — no error, not one line on stderr. Wrapping in `Option` or adding a Null variant
/// would revive that silence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum RequestId {
    Str(String),
    Num(i64),
}

/// `JSONRPCErrorError` — the body of the error envelope.
///
/// ★`code` is an opaque i64★: the schema has no enum · const · min/max, and the `-32xxx` range appears **not once**
/// (including `-32001` — grep 0 hits). Overload · rate limit come not in this envelope but in the `error`
/// **notification**'s `codexErrorInfo` (`serverOverloaded`·`rateLimitExceeded`).
/// ★Whether a code like `-32001` actually arrives at runtime is unverified★ — we put no code that branches on the number
/// here. If handling of a specific code becomes necessary, real-server observation comes first.
///
/// Measured 0.154.0: the answer to an unknown method is `-32600` and the `data` field **did not come** (we still make it
/// optional — the schema allows it). ★`message` can reach 4KB★ — the body of an unknown-method
/// error lists all 160 valid methods. Whoever carries this string to a log · the screen
/// must cut it.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RpcError {
    pub(crate) code: i64,
    pub(crate) message: String,
    #[serde(default)]
    pub(crate) data: Option<Value>,
}

/// The four envelope shapes of one line coming in from the server.
///
/// ★Discrimination is by **presence of fields** — in principle it cannot be told by the id value★: the id the server issued must be
/// returned as is, so the two id spaces are bound to overlap (TRD §4-4).
/// It is not made `#[serde(untagged)]` — the required sets of the four shapes overlap and none forbids extra fields, so
/// order would become a trap that decides meaning (`JSONRPCMessage` has that shape).
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
    /// ★A real notification envelope carries one more `emittedAtMs` alongside `method`·`params`★
    /// (measured 0.154.0 — present in 12/12 notifications and absent in 8/8 responses. It is a field not in the schema).
    /// Here we **deliberately do not carry** it — translation at this stage does not depend on order. If needed,
    /// read that key from the original line (discrimination stands as it is even with that field present).
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
/// ★All are non-fatal★ — the caller logs that one line and drops it, and does not cut the stream
/// (TRD §4-6: we do not die on a non-JSON stdout line).
#[derive(Debug, thiserror::Error)]
pub(crate) enum ParseError {
    #[error("JSON 이 아니다: {0}")]
    NotJson(serde_json::Error),
    #[error("JSON 최상위가 객체가 아니다")]
    NotObject,
    #[error("`method` 가 문자열이 아니다")]
    MethodNotString,
    #[error("`id` 가 문자열도 i64 도 아니다")]
    BadId,
    #[error("`error` 가 {{code,message}} 모양이 아니다: {0}")]
    BadError(serde_json::Error),
    #[error("`{0}` 봉투에 `id` 가 없다")]
    MissingId(&'static str),
    /// `result` and `error` came together in one line. It is a line claiming success and failure at once, so reading it either way may be wrong —
    /// ★if the field that appears first is let to win, a failure is read as a success★.
    #[error("`result` 와 `error` 가 한 봉투에 함께 있다")]
    ConflictingEnvelope,
    #[error("네 봉투 모양 중 어느 것도 아니다 — method·result·error 가 전부 없다")]
    UnknownShape,
}

/// Read one **line** as one envelope. Line splitting is the caller's job and this function does not handle newlines.
///
/// The discrimination order is the contract: if `method` is present it is a request/notification (then the presence of `id` splits the two),
/// and only when absent are `result`·`error` looked at. ★A line with `method` and `id` together is always a request★ —
/// reading it as a notification would drop a request that must be answered.
///
/// ★Discrimination stands even with unknown sibling fields★ — real notifications always come with `emittedAtMs`
/// (measured 0.154.0), and upstream may add fields not in the schema at any time. If read as "there is an extra field so it is none of the four
/// shapes", normal lines would be dropped whole.
///
/// A line that is not JSON, or is JSON but fits none of the four shapes, comes back as an **error** — neither a panic nor
/// a quiet drop.
///
/// ★One requirement on the caller — put a deadline on every request sent★ (measured 0.154.0): the server
/// **gives no answer to an envelope it could not decode, even if an `id` is carried** (for the one line `{"id":90}`
/// only a stderr log was left and stdout stayed silent forever). Answers come only for *parameter* errors after the envelope was read.
/// So a waiting ticket that "waits until the answer comes" will sooner or later hang forever.
///
/// ★Known limitation — duplicate keys are not filtered★: for a line like `{"id":1,"id":2,"result":{}}` serde_json
/// **takes the last value**. So if such a line comes we read `id=2`, and that response is **delivered to the
/// wrong waiter** (the original request is never released and someone else's request gets someone else's answer). To filter,
/// a dedicated visitor that does not go through a map is needed, so it is not done here — unverified, and we have
/// never seen a real server emit such a line.
pub(crate) fn classify(line: &str) -> Result<Inbound, ParseError> {
    let value: Value = serde_json::from_str(line).map_err(ParseError::NotJson)?;
    let obj = value.as_object().ok_or(ParseError::NotObject)?;

    // ★Look at it before the method branch★ — the contradiction is independent of the envelope kind, and if looked at later, the branch that picks `result` first
    //   reads a failure as a success.
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

/// Wire method names. ★There is no version prefix★ — `v1/`·`v2/` in the schema tree are merely directories for source organization,
/// and methods go out as bare names like `thread/start`.
pub(crate) mod method {
    // Us → server requests
    pub(crate) const INITIALIZE: &str = "initialize";
    pub(crate) const THREAD_START: &str = "thread/start";
    pub(crate) const THREAD_RESUME: &str = "thread/resume";
    /// Set the extra skill roots of this app-server **process** (measured 0.163.0 — M4, M9): another live process,
    /// even on the same `CODEX_HOME`, does not see them, and `thread/resume` does not restore them, so every process
    /// sets them again before its `thread/start` / `thread/resume`. The response is `{}`.
    // ADR-0337
    pub(crate) const SKILLS_EXTRA_ROOTS_SET: &str = "skills/extraRoots/set";
    pub(crate) const TURN_START: &str = "turn/start";
    /// Put one input into a running turn — it does not open a turn (ADR-0231).
    pub(crate) const TURN_STEER: &str = "turn/steer";
    pub(crate) const TURN_INTERRUPT: &str = "turn/interrupt";

    /// Receive the past items of a resumed thread **as pages** (ADR-0203).
    ///
    /// ★The twin `thread/turns/list` is not used★ — the unit we draw on the screen is the item, not the turn, and that one
    /// wraps the same item one more layer inside a turn envelope. It only adds one envelope to peel.
    pub(crate) const THREAD_ITEMS_LIST: &str = "thread/items/list";

    /// Us → server notification. ★The only notification a client can send is this one, and it has no params field
    /// at all★ (`ClientNotification.json`).
    /// ★Sending it is not a duty★ (measured 0.154.0 — in both the sent and unsent cases
    /// `thread/start` succeeded the same). Sending is harmless, so we keep it on the side of matching the handshake shape.
    pub(crate) const INITIALIZED: &str = "initialized";

    // Among server → us notifications, the ones the translator knows by name
    pub(crate) const ITEM_AGENT_MESSAGE_DELTA: &str = "item/agentMessage/delta";
    pub(crate) const ITEM_STARTED: &str = "item/started";
    pub(crate) const ITEM_COMPLETED: &str = "item/completed";
    pub(crate) const THREAD_TOKEN_USAGE_UPDATED: &str = "thread/tokenUsage/updated";
    pub(crate) const ERROR: &str = "error";
    pub(crate) const DEPRECATION_NOTICE: &str = "deprecationNotice";

    /// ★Carried into a usage observation, not a screen event★ — it is account-level state, so it must not go through per-agent replay
    /// (this arm of `decoder` · interpretation = this folder's `usage`).
    pub(crate) const ACCOUNT_RATE_LIMITS_UPDATED: &str = "account/rateLimits/updated";

    /// Us → server request — only one active usage query is sent (this folder's `usage_probe`). The conversation path does not send this.
    pub(crate) const ACCOUNT_RATE_LIMITS_READ: &str = "account/rateLimits/read";

    /// ★This one name has two consumers★ — the translator carries it into a turn boundary (`decoder`), and the path reads it as input to the queue-release
    /// state machine (`transport`). The two each look at the same line.
    pub(crate) const TURN_COMPLETED: &str = "turn/completed";

    // Among server → us requests, the ones the path knows by name. ★Refusal is done the same way for any request★ — on top of that these two remember the refused
    // item and attribute the end of that item to our refusal (ADR-0241).
    pub(crate) const ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL: &str =
        "item/commandExecution/requestApproval";
    pub(crate) const ITEM_FILE_CHANGE_REQUEST_APPROVAL: &str = "item/fileChange/requestApproval";
}

/// All four values of `TurnStatus` (schema 0.154.0 `definitions.TurnStatus` — a closed `enum`).
///
/// ★Kept as string constants and not received as a Rust `enum`★: if deserialized into a closed type, the day upstream adds a fifth value
/// the **whole** `turn/completed` line fails, and then the turn boundary vanishes entirely and the screen's waiting
/// indicator spins forever. That outcome is the very defect these constants fix.
pub(crate) mod turn_status {
    pub(crate) const COMPLETED: &str = "completed";
    pub(crate) const INTERRUPTED: &str = "interrupted";
    pub(crate) const FAILED: &str = "failed";
    pub(crate) const IN_PROGRESS: &str = "inProgress";

    /// For telling grades apart — a value **not** here is upstream drift, and a value that is here but not carried into an outcome
    /// (today [`IN_PROGRESS`] alone) is 「뜻을 아직 모른다」 (meaning not yet known). Either way the translation is still done.
    pub(crate) const ALL: &[&str] = &[COMPLETED, INTERRUPTED, FAILED, IN_PROGRESS];
}

/// The code carried when refusing an inbound **request** we do not know.
///
/// ★The schema does not fix a code range★ — `JSONRPCErrorError.code` is an unconstrained i64 and
/// `-32xxx` never appears in the schema. We are the side that defines the envelope, so we pick the value, borrowing
/// the JSON-RPC 2.0 convention's "method not found" (its meaning is clear to a reader).
/// ★The app-server reads this answer to an approval request as 「승인 요청 실패」 (approval request failed)★ (measured codex-cli 0.156.1 · fixture
/// `refuse_u2a`): it closes that item right away without executing it (command = `failed` · file change = `declined`), and the turn continues and
/// ends with `completed` — there is no `error` notification. The reaction to a non-approval request is unverified. Either way it is better than dropping:
/// if dropped, that agent freezes permanently (TRD §6-2).
///
/// For reference, **codex itself returns `-32600` for an unknown method** (measured 0.154.0) — that is, the peer
/// does not count "unknown method" separately. Still, our value is -32601, whose conventional meaning is clear:
/// the first reader of this number is the person who opens the log.
pub(crate) const METHOD_NOT_FOUND: i64 = -32601;

// ── Serialization helpers ───────────────────────────────────────────────────────────────

/// Build one request line. ★The returned string is a **complete line including the newline**★ — if the caller
/// also appends `\n`, one extra empty line goes out.
///
/// If `params` serialization fails it is an error (our types cannot fail, but it is generic so the contract leaves it open).
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
/// ★`id` is carried exactly as received★ — it is the server's waiting-ticket key, so if normalized that request is never released.
/// ★Success must not be disguised★ — if a success response is returned to an approval request it becomes an automatic approval and
/// bypasses the approval policy (TRD §6-2).
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

/// The client name carried in `initialize` (shared by the conversation path · usage query). The peer writes this value into its own log · `user_agent`.
pub(crate) const CLIENT_NAME: &str = "engram-dashboard";

/// ★It can be sent only once★ (measured 0.154.0) — a second `initialize` comes back as `-32600` `"Already
/// initialized"`. The responsibility to keep a retry path from sending this again lies with the caller.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InitializeParams {
    pub(crate) client_info: ClientInfo,
    /// `None` = the field is absent — every capability at its default. `Some` only on a spawn whose first turn carries
    /// `additionalContext`, which codex refuses (`-32600`) without `experimentalApi` (measured 0.163.0 — M3).
    /// ★Only `experimentalApi` is ever sent★ — `requestAttestation` must stay at its default false: it would bring the
    ///   `attestation/generate` request, which has no refusal shape at all. The opt-in also enables the server request
    ///   `currentTime/read` (refused `-32601` like every server request) and the `thread/readState/changed`
    ///   notification (not translated).
    // ADR-0337
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) capabilities: Option<InitializeCapabilities>,
}

/// The capability set of `initialize` — only the one field we send (see [`InitializeParams::capabilities`]).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InitializeCapabilities {
    pub(crate) experimental_api: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ClientInfo {
    pub(crate) name: String,
    pub(crate) version: String,
    // `title` is optional so it is not carried.
}

/// ★There is no protocol version here★ — `protocolVersion` appears 0 times in the whole schema.
/// What `initialize` negotiates is not a version but a capability set ([`InitializeParams::capabilities`]).
/// The only values that tell upstream drift after the fact are `user_agent` and `Thread.cliVersion`, so they are recorded once at
/// startup (TRD §4-7 item 1).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InitializeResponse {
    pub(crate) codex_home: String,
    pub(crate) platform_family: String,
    pub(crate) platform_os: String,
    pub(crate) user_agent: String,
}

// ── thread/start · thread/resume ──────────────────────────────────────────────

/// Approval policy. The schema is `oneOf[문자열 3 종, GranularAskForApproval]` but ★we send only the string branch★
/// — the granular side is a field we do not use, so we did not copy its shape.
/// Note that the value spelling is kebab-case: the other approval enums of the same protocol are camelCase.
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
/// ★Do not confuse it with the differently-typed same name★ — `TurnStartParams.sandboxPolicy` and
/// `ThreadStartResponse.sandbox` are a structured union (`SandboxPolicy`) and not this string enum.
/// The response's `sandbox` cannot be put back into this field as it is.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) enum SandboxMode {
    #[serde(rename = "read-only")]
    ReadOnly,
    #[serde(rename = "workspace-write")]
    WorkspaceWrite,
    #[serde(rename = "danger-full-access")]
    DangerFullAccess,
}

/// ★The schema has no `required` array at all — `{}` is valid params★. So every field must be
/// `Option` + `skip_serializing_if`: otherwise codex receives an explicit `null`
/// where it expected "absence".
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadStartParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) approval_policy: Option<AskForApproval>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sandbox: Option<SandboxMode>,
    /// Instructions **appended after** codex's default instructions. Absent = nothing is appended.
    ///
    /// ★It is appending, not replacing★ — whatever is put in this field, codex's own default instructions remain
    ///   (measured 0.155.0: the body stands in front as a `developer`-role message at `input[0]` and the default prompt is undamaged).
    /// ★It **carries the same thing as the terminal mode's `-c developer_instructions=…` but is not the same path**★:
    ///   this one is a JSON body, so there is no command-line length cap, no `%VAR%` substitution, no line-break truncation. So the value is carried
    ///   **untouched, as it is** — do not bring that side's line-break conversion · character guard over here (it would block no
    ///   risk while only ruining the document the agent reads).
    // ADR-0215
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) developer_instructions: Option<String>,
}

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
    /// The current full instructions text, on every resume. `None` = the field is absent.
    ///
    /// ★It does not reach the next turn★ (measured 0.163.0 — M8): codex holds it in the resumed process's session and
    ///   uses it only when a compaction in that process rebuilds the initial context. Without it that rebuild has no
    ///   instructions at all — so it is sent on every resume, changed or not; a change reaches the next turn only
    ///   through the first turn's `additionalContext` ([`TurnStartParams::additional_context`]).
    /// The field is in the stable schema from 0.155.0; the transport drops it for a codex below that or whose version
    ///   it cannot read.
    // ADR-0337
    // ADR-0354
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) developer_instructions: Option<String>,
}

/// The **second request** of the handshake — open a new thread, or resume a saved thread.
///
/// ★The one place that chooses is [`crate::backend::AgentBackend::open_spawn`] in `backend/codex/mod.rs`★ —
/// the path emits what it received as is. If the path looked at its own state and decided again, there would be two places that decide
/// (the same shape as what ADR-0191 withdrew from path selection).
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

/// The `thread/resume` response. Its field composition is the `thread/start` response with two more cursors attached.
///
/// ★One of those cursors is now read — it is the **entry point** of screen restore★ (ADR-0203). How to use it the schema
/// (0.154.0) writes directly in that field's description: "Pass this as `cursor` to `thread/items/list` with
/// `sortDirection: \"desc\"`. The first page includes the item identified by the cursor."
/// ★It is left `default` because the schema is so★ — it is not in the `required` list and has `default: null`.
/// Absent = there is no history to go back to, and it is **not a failure** (restore is skipped).
/// ★The `thread/start` response has no such field at all★ — so a new conversation has no material to take this path.
/// ★The other cursor (`turnsBackwardsCursor`) is still not read★ — [`method::THREAD_ITEMS_LIST`] above carries the
/// reason for not using that twin.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadResumeResponse {
    pub(crate) thread: Thread,
    #[serde(default)]
    pub(crate) items_backwards_cursor: Option<String>,
}

/// Which direction to walk the page — all two values of the schema's closed `enum` (0.154.0 `SortDirection`).
///
/// ★[`SortDirection::Asc`] is used by nobody today — we write it anyway★: this is an **outgoing** field so the value cannot be
/// invented, and with both written, a change that reverses direction does not create a new constant.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) enum SortDirection {
    #[serde(rename = "asc")]
    Asc,
    #[serde(rename = "desc")]
    Desc,
}

/// `thread/items/list` request. ★Only `threadId` is required★ (schema 0.154.0) — all the rest may be omitted, so they are left as
/// `Option` + `skip_serializing_if` (the same reason as [`ThreadStartParams`]: an explicit
/// `null` is not sent where absence is expected).
/// ★`turnId`, which narrows to one turn, was not copied★ — we walk the whole thread from the end.
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
/// ★If `nextCursor` is absent or `null`, there is nothing more to see in that direction★ — the schema says so, and in measurement too
/// `null` came on the last page (2026-09-16, real 0.154.0 · a 28-item thread ended as 25+3).
/// ★`backwardsCursor`, used when reversing direction, was not copied★ — we walk in one direction only.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadItemsListResponse {
    pub(crate) data: Vec<ThreadItemEntry>,
    #[serde(default)]
    pub(crate) next_cursor: Option<String>,
}

/// One item carried in a page + the turn it belongs to.
///
/// ★That `item` is the **same type** as that field of [`ItemNotification`] is the ground of ADR-0203★ — in the vendor
/// schema the item definition hash is the same for the `item/completed` notification · resume response · page response (19 variants).
/// So we do not write a new translator but only peel the envelope. The reason for taking [`Value`] is also the same as that side — one unknown variant
/// must not kill the **whole** page.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThreadItemEntry {
    pub(crate) turn_id: String,
    pub(crate) item: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Thread {
    /// Issued by codex — ★the caller cannot decide it★. The value is a UUIDv7 string and `thread.sessionId`
    /// holds the same value (measured 0.154.0). It sits at `result.thread.id`, not at the response top level.
    pub(crate) id: String,
    /// ★The schema marks it required but we allow absence★ — failing the whole thread start because one diagnostic value is gone is
    /// worse. It is the value that tells upstream drift after the fact.
    #[serde(default)]
    pub(crate) cli_version: Option<String>,
}

// ── turn/start · turn/interrupt ───────────────────────────────────────────────

/// One piece of turn input. The schema has 7 variants (text·image·localImage·audio·localAudio·skill·mention) but
/// ★what we send is `text` alone★, so we did not copy the rest.
///
/// ★Not carrying `text_elements` is also intentional★ — it is optional with default `[]`, and its spelling is the only snake_case
/// in this camelCase type. If needed it must be written with that spelling.
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
    /// ★If `None`, the field itself is not carried★ — the envelopes of inputs without an identifier and of incarnations below the minimum codex version are
    /// byte-for-byte the same as before this field existed.
    // ADR-0231
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) client_user_message_id: Option<String>,
    /// Context codex adds to this turn's model input as a developer message, tagged with the map key
    /// (`<engram-instructions>…</engram-instructions>`) — not shown in the echoed user message. `None` = the field is
    /// absent, byte-identical to the envelope before this field existed.
    /// ★Needs `experimentalApi` at `initialize`★ (`-32600` without it — measured 0.163.0, M3). It lives in the history
    ///   only, so every compaction drops it (M8).
    // ADR-0337
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) additional_context: Option<BTreeMap<String, AdditionalContext>>,
}

/// The `additionalContext` map key of the instructions-change context — codex shows it as the tag of the developer
/// message.
// ADR-0337
pub(crate) const INSTRUCTIONS_CONTEXT_KEY: &str = "engram-instructions";

/// One `additionalContext` entry. `kind` is always `application` — the one kind we send.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AdditionalContext {
    pub(crate) kind: &'static str,
    pub(crate) value: String,
}

impl AdditionalContext {
    pub(crate) fn application(value: String) -> Self {
        AdditionalContext {
            kind: "application",
            value,
        }
    }
}

/// `skills/extraRoots/set` — this process's extra skill roots, absolute paths (see [`method::SKILLS_EXTRA_ROOTS_SET`]).
/// The `{}` response is not read.
// ADR-0337
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkillsExtraRootsSetParams {
    pub(crate) extra_roots: Vec<String>,
}

/// `turn/steer` — put one input into a running turn. The success response is `{turnId}` and that is **acceptance**, not a receipt
/// (the receipt is the echo carrying `clientId` — measured 0.156.1). The response body is not read.
///
/// ★`expectedTurnId` is a precondition the vendor requires★ — if that turn has already ended or the id mismatches, an error
///   response (`-32600 "no active turn to steer"`) comes. So a steer that reaches an ended turn late does not mix into the
///   next turn.
/// ★Not making `clientUserMessageId` an `Option` is intentional★ — it is an optional field in the schema, but steer is sent only for entries that
///   have an identifier from an incarnation that meets the minimum codex version (without an identifier the handed-over text cannot be matched against the echo).
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

/// Turn — ★this type reads the `turn/start` **response**, and the `turn/completed` notification is not read with it★.
/// The `turn` of that notification is the same schema type, but the translator scans it directly as a [`Value`] (source of truth for the reason is
/// the `turn/completed` spot of [`crate::backend::codex::decoder`] — even if one field is off, the turn boundary must stand).
/// All that is needed here is the one id to put into `turn/interrupt`, so we did not copy the other fields (`status`·`items`·`error`
/// ·`startedAt`·`completedAt`·`durationMs`·`itemsView`).
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

/// The result of `turn/interrupt` — ★a truly empty object, with neither properties nor required in the schema★.
/// We **only receive** this (the only response we emit is the refusal error).
///
/// Writing it as a struct with braces is intentional — a unit struct serializes as `null`, not `{}`.
/// Right now no line goes out in that direction, but we pin it so the shape does not change quietly when this type is some day carried on an outgoing line
/// (the regression net measures that serialization).
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
/// ★The two notifications differ only in `startedAtMs` vs `completedAtMs`, and the `item` union is the same★ — so
/// we read with one type and branch by `method`.
/// Taking `item` as a [`Value`] and not as a type is intentional — an internally-tagged enum meeting an unknown
/// `type` makes **the whole line a deserialization failure**, and this union has 19 variants today and upstream keeps
/// growing it. One unknown variant must not kill the line.
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

/// ★The schema does not explain what `last` and `total` count★ — there is no description string and both are
/// required. We read by name alone, `last` = the model request that just ended, `total` = cumulative, and the translator
/// uses `last` (the reason is at the usage translation spot of [`crate::backend::codex::decoder`]).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ThreadTokenUsage {
    pub(crate) last: TokenUsageBreakdown,
}

/// ★Our `Usage` vocabulary has only two fields, so the rest is lost★ — they are `cachedInputTokens`
/// ·`reasoningOutputTokens`·`totalTokens`·`cacheWriteInputTokens`. Growing the vocabulary is
/// a wire change and outside the scope of this stage.
///
/// ★It is a signed `i64` because the schema declares `format: int64`★ — if narrowed to `u64`, a single
/// negative or out-of-range value makes **the whole notification fail deserialization** and that turn's `Usage` vanishes entirely.
/// Our vocabulary field is `u64`, so a negative is clamped to 0 at the translation spot (the comment there is the source of truth).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TokenUsageBreakdown {
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
}

/// `error` notification.
///
/// ★`threadId`·`turnId`·`willRetry` are marked required by the schema but absence is allowed here★ —
/// dropping an undecodable error whole is the worst outcome in this protocol, so
/// we let it through as long as the error body is present.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ErrorNotification {
    pub(crate) error: TurnError,
    #[serde(default)]
    pub(crate) will_retry: bool,
}

/// ★Two fields carry this type★ — the `error` of the `error` notification, and `Turn.error`. The schema writes on the latter
/// 「Only populated when the Turn's status is failed」 (0.154.0). So the reason of a failed turn is read by this one type,
/// and the place that makes the screen string is also one.
///
/// ★Taking `codexErrorInfo` as a [`Value`] is intentional★ — it is a union mixing 13 string enums and 5 wrapper objects
/// (= `oneOf` members 6), and all we use is one label a person can read. Inside it are
/// `serverOverloaded`·`rateLimitExceeded` — the schema's only backpressure vocabulary,
/// and it comes via **this notification**, not a numeric error code.
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

/// Detail for when a turn is blocked by a model-alignment violation. ★`message` alone cannot tell "why it was blocked" nor "what to
/// do"★ — these three carry those two.
///
/// The schema has no `required` array — all three fields are optional and all three are nullable.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MisalignmentErrorDetails {
    /// An already localized explanation comes (schema description: "A substantive localized explanation").
    #[serde(default)]
    pub(crate) detailed_explanation: Option<String>,
    /// ★Not a closed enum★ — the schema left it open with "clients must accept categories added by Responses",
    /// so it is received as a string as is.
    #[serde(default)]
    pub(crate) error_type: Option<String>,
    /// If we decide to continue, it is the **sentence to submit as-is as the next turn input** — not a sentence we invent,
    /// but one codex gave.
    #[serde(default)]
    pub(crate) steer: Option<MisalignmentSteer>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct MisalignmentSteer {
    pub(crate) message: String,
}

// ── params of the usage limit notification ─────────────────────────────────────────────────

/// `account/rateLimits/updated`. ★The fields of this section were read from the schema of **codex-cli 0.156.1**, not 0.154.0★
/// (`v2/AccountRateLimitsUpdatedNotification.json` — the regeneration command is in this file's header).
///
/// ★One notification = a sparse update of **one** limit bucket★ — buckets are told apart by [`RateLimitSnapshot::limit_id`]. The schema
/// description says 「가장 최근 `account/rateLimits/read` 응답에 병합하라 — 비어 온 값은 앞 값을 지우지 않는다」 (merge into the most recent account/rateLimits/read response — an empty value does not erase the earlier value).
/// It was measured that it arrives mid-turn (`docs/process/S21-codex-backend/trd-phase2a.md` L7), and no carried value has ever been captured.
///
/// ★If params and `rateLimits` are not JSON **objects**, it is a deserialization failure ([`object_only`])★ — unlike the two field-lenient types below,
///   here that line is dropped whole: if the whole notification or the whole snapshot has another shape, there is no field to save.
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
/// ★Each field fails separately★ — a field of the wrong type becomes `None` for that field alone and the snapshot survives ([`lenient`]·[`lenient_object`]).
/// If one field's shape change made the whole notification a deserialization failure, the bucket's remaining values would be lost too.
/// ★Only `limit_id` is taken as a raw [`Value`] intentionally★ — this field tells 「어느 버킷인가」 (which bucket it is), and if a wrong type were folded into
///   `None` it could not be told apart from absence (= the side read as the default bucket). The consumer interprets.
/// ★This type itself would read even a JSON array in field order★ — enforcing an object is carried by the [`object_only`] of the side that carries this type (the notification ·
///   [`GetAccountRateLimitsResponse`]). When this type is read on a new path, it must also pass that gate — if not, a
///   plausible fake window appears.
/// `limit_name`·`plan_type` are read only by the query response (pickup does not carry plan · per-model windows — this folder's `usage`).
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
    /// The schema is a closed enum (`PlanType`) but we receive it as a string — the day upstream adds a value, only this field needs to be read.
    #[serde(default, deserialize_with = "lenient")]
    pub(crate) plan_type: Option<String>,
}

/// One limit window (schema `RateLimitWindow`). Schema types = `usedPercent` int32 (required) · `windowDurationMins`
/// int64 · `resetsAt` int64 (epoch seconds) — none has a range constraint, so the consumer decides range.
///
/// ★`primary`/`secondary` are slot names, not 5-hour/weekly★ (measured — the consumer tells them apart by `window_duration_mins`).
/// ★Taking `used_percent` as `f64` is intentional★ — the schema says integer, but there is no reason to reject a fraction, and the field value is
///   `f64` anyway. The reset time, by contrast, accepts integers only (we do not do float → integer conversion on external values).
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

/// The params of `account/rateLimits/read` — ★no field is carried at all (`{}`)★. The schema (0.156.1
/// `v2/NullableGetAccountRateLimitsParams.json`) accepts `null` or an object whose fields are all optional, and the probe measured with `{}`
/// (`.claude/handoff/attachments/codex-usage-probe.mjs`).
/// ★Not carrying `excludeResetCreditDetails` is to keep the measured shape★ — the schema description is 「배경 폴링용 —
///   리셋 크레딧 상세 조회를 건너뛴다」 (for background polling — skips the reset credit detail query) and fits our use, but a request carrying that field has not been measured.
///
/// Being a struct with braces is for the same reason as [`TurnInterruptResponse`] — a unit struct serializes as `null`.
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct GetAccountRateLimitsParams {}

/// The `account/rateLimits/read` response (schema 0.156.1 `v2/GetAccountRateLimitsResponse.json`) — only two fields read.
///
/// ★Not carrying `accountId` is a decision★ — an account identifier comes at the response top level. If not read, there is no way for this value to leak into logs · error
///   strings starting from the type (TRD §1-3). If it becomes needed (account key — TRD §3 #33), decide that value's exit first.
/// ★Snapshot slots accept objects only ([`object_only`])★ — if the response itself is not an object it is a deserialization failure, and if a snapshot
///   slot (`rateLimits` · each map entry) is not an object, **only that slot** is treated as absent.
#[derive(Debug, Clone)]
pub(crate) struct GetAccountRateLimitsResponse {
    /// Schema description = "Backward-compatible single-bucket view" — ★no guarantee it is the default bucket★ (it may reflect another bucket).
    /// `None` = absent, `null`, or not an object.
    pub(crate) rate_limits: Option<RateLimitSnapshot>,
    /// `rateLimitsByLimitId` — bucket id → snapshot, **sorted by id** (JSON map order differs with each serde_json feature combination so
    /// we decide). `None` = the field is absent, `null`, or not an object.
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

/// Read one field, and if its shape is wrong (including `null`) put **that field only** as `None`. A missing field is handled by `#[serde(default)]`.
fn lenient<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Value::deserialize(de)?;
    Ok(serde_json::from_value(value).ok())
}

/// Same as [`lenient`] but accepts only JSON **objects**.
/// ★Do not remove the object check★ — struct deserialization made by serde also accepts arrays and fills in field order. If an array
///   comes at a window slot, those numbers are read as utilization · window length · reset in declaration order and become a plausible fake window.
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

/// Read only JSON **objects** and anything else is an error. The ground of the object check is the same as [`lenient_object`] — an array is read in field order
/// and becomes a plausible fake value (params that are `[{…스냅숏…}]` · a snapshot that is `["codex", {…창…}, {…창…}]`).
fn object_only<'de, D, T>(de: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Value::deserialize(de)?;
    if !value.is_object() {
        return Err(serde::de::Error::custom("JSON 객체가 아니다"));
    }
    serde_json::from_value(value).map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
        v.get(key)
            .unwrap_or_else(|| panic!("필드 `{key}` 가 없다: {v}"))
    }

    // ── Envelope discrimination ────────────────────────────────────────────────────────────

    #[test]
    fn notification_has_method_and_no_id() {
        let line = r#"{"method":"item/agentMessage/delta","params":{"delta":"hi"}}"#;
        match classify(line).expect("알림으로 읽혀야 한다") {
            Inbound::Notification { method, params } => {
                assert_eq!(method, "item/agentMessage/delta");
                assert_eq!(field(&params.unwrap(), "delta"), "hi");
            }
            other => panic!("알림이 아니다: {other:?}"),
        }
    }

    #[test]
    fn method_plus_id_is_a_request_never_a_notification() {
        let line = r#"{"id":7,"method":"item/fileChange/requestApproval","params":{}}"#;
        match classify(line).expect("요청으로 읽혀야 한다") {
            Inbound::Request { id, method, .. } => {
                assert_eq!(id, RequestId::Num(7));
                assert_eq!(method, "item/fileChange/requestApproval");
            }
            other => panic!("요청이 아니다: {other:?}"),
        }
    }

    #[test]
    fn id_plus_result_is_a_response_never_a_request() {
        let line = r#"{"id":"abc","result":{"thread":{"id":"t-1"}}}"#;
        match classify(line).expect("응답으로 읽혀야 한다") {
            Inbound::Response { id, result } => {
                assert_eq!(id, RequestId::Str("abc".to_string()));
                assert_eq!(field(field(&result, "thread"), "id"), "t-1");
            }
            other => panic!("응답이 아니다: {other:?}"),
        }
    }

    #[test]
    fn id_plus_error_is_an_error_envelope() {
        let line = r#"{"id":3,"error":{"code":-1,"message":"nope"}}"#;
        match classify(line).expect("오류로 읽혀야 한다") {
            Inbound::Error { id, error } => {
                assert_eq!(id, RequestId::Num(3));
                assert_eq!(error.code, -1);
                assert_eq!(error.message, "nope");
                assert!(error.data.is_none());
            }
            other => panic!("오류 봉투가 아니다: {other:?}"),
        }
    }

    #[test]
    fn string_ids_survive_verbatim() {
        let line = r#"{"id":"req-42","method":"x","params":null}"#;
        match classify(line).unwrap() {
            Inbound::Request { id, .. } => assert_eq!(id, RequestId::Str("req-42".to_string())),
            other => panic!("요청이 아니다: {other:?}"),
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

    /// A line claiming success and failure at once — picking one of the two to read can read a failure as a success.
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

    /// Measured 0.154.0 — notification envelopes always come with `emittedAtMs`. If discrimination wobbles because of the extra sibling field,
    /// normal notifications would be dropped whole.
    #[test]
    fn an_extra_sibling_key_does_not_break_classification() {
        let line =
            r#"{"method":"item/started","params":{"turnId":"u"},"emittedAtMs":1764000000000}"#;
        match classify(line).expect("알림으로 읽혀야 한다") {
            Inbound::Notification { method, .. } => assert_eq!(method, "item/started"),
            other => panic!("알림이 아니다: {other:?}"),
        }
    }

    /// Measured 0.154.0 — responses carry several fields not in the schema. If unknown fields are rejected, a normal response
    /// becomes a decode failure.
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
            other => panic!("알림이 아니다: {other:?}"),
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
            other => panic!("요청이 아니다: {other:?}"),
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
            other => panic!("오류 봉투가 아니다: {other:?}"),
        }
    }

    /// Measured 0.154.0 — a request with `id: null` is **reclassified as a notification by the server and quietly dropped** (no error and no
    /// stderr). So our id must never become `null` by any path.
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
            capabilities: None,
        })
        .unwrap();
        let info = field(&v, "clientInfo");
        assert_eq!(field(info, "name"), "engram-dashboard");
        assert_eq!(field(info, "version"), "0.1.0");
        assert!(v.get("capabilities").is_none());
    }

    /// The opt-in a spawn with a first-turn context sends — `experimentalApi` and nothing else (`requestAttestation`
    /// stays absent, so at its default false).
    // ADR-0337
    #[test]
    fn initialize_params_carry_only_the_experimental_api_opt_in() {
        let line = request_line(
            &RequestId::Num(0),
            method::INITIALIZE,
            &InitializeParams {
                client_info: ClientInfo {
                    name: "engram-dashboard".to_string(),
                    version: "0.1.0".to_string(),
                },
                capabilities: Some(InitializeCapabilities {
                    experimental_api: true,
                }),
            },
        )
        .unwrap();
        let v: Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(
            field(&v, "params")["capabilities"],
            serde_json::json!({"experimentalApi": true})
        );
    }

    // ADR-0337
    #[test]
    fn skills_extra_roots_set_params_field_names() {
        let v = serde_json::to_value(SkillsExtraRootsSetParams {
            extra_roots: vec![
                "C:/v/engram/skills".to_string(),
                "C:/f/.agents/skills".to_string(),
            ],
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({"extraRoots": ["C:/v/engram/skills", "C:/f/.agents/skills"]})
        );
        assert_eq!(method::SKILLS_EXTRA_ROOTS_SET, "skills/extraRoots/set");
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
        // ★Going out as camelCase is the contract★ — if it leaks out as snake_case, codex reads it as an unknown field and quietly
        //   drops it, and the one symptom is 「프라이밍이 안 먹는다」 (priming has no effect) (there is no error).
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
            developer_instructions: None,
        })
        .unwrap();
        assert_eq!(field(&v, "threadId"), "t-1");
        assert_eq!(field(&v, "excludeTurns"), true);
        assert!(v.get("cwd").is_none());
        assert!(v.get("approvalPolicy").is_none());
        assert!(v.get("sandbox").is_none());
        assert!(v.get("developerInstructions").is_none());
    }

    /// M8 — the resume carries the instructions under the same wire name as `thread/start`.
    // ADR-0337
    #[test]
    fn thread_resume_params_carry_the_instructions_under_their_wire_name() {
        let v = serde_json::to_value(ThreadResumeParams {
            thread_id: "t-1".to_string(),
            cwd: None,
            approval_policy: None,
            sandbox: None,
            exclude_turns: Some(true),
            developer_instructions: Some("be brief\n".to_string()),
        })
        .unwrap();
        assert_eq!(field(&v, "developerInstructions"), "be brief\n");
    }

    #[test]
    fn turn_start_params_field_names_and_input_tagging() {
        let v = serde_json::to_value(TurnStartParams {
            thread_id: "t-1".to_string(),
            input: vec![UserInput::Text {
                text: "hello".to_string(),
            }],
            client_user_message_id: None,
            additional_context: None,
        })
        .unwrap();
        assert_eq!(field(&v, "threadId"), "t-1");
        let item = &field(&v, "input").as_array().unwrap()[0];
        assert_eq!(field(item, "type"), "text");
        assert_eq!(field(item, "text"), "hello");
        assert!(
            v.get("clientUserMessageId").is_none(),
            "식별자 없는 봉투에 칸이 생겼다: {v}"
        );
        assert_eq!(
            v.as_object().unwrap().len(),
            2,
            "the envelope without an identifier and without a context grew a field: {v}"
        );
    }

    // ADR-0337
    #[test]
    fn turn_start_params_carry_the_instructions_context_under_its_wire_shape() {
        let v = serde_json::to_value(TurnStartParams {
            thread_id: "t-1".to_string(),
            input: vec![UserInput::Text {
                text: "hello".to_string(),
            }],
            client_user_message_id: None,
            additional_context: Some(BTreeMap::from([(
                INSTRUCTIONS_CONTEXT_KEY.to_string(),
                AdditionalContext::application("new text".to_string()),
            )])),
        })
        .unwrap();
        assert_eq!(
            field(&v, "additionalContext"),
            &serde_json::json!({"engram-instructions": {"kind": "application", "value": "new text"}})
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
            additional_context: None,
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
        assert_eq!(v.as_object().unwrap().len(), 4, "모르는 칸이 실렸다: {v}");
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
                    "detailedExplanation": "이 요청은 정책을 벗어난다",
                    "errorType": "somethingBrandNew",
                    "steer": {"message": "다르게 물어보세요"}
                }
            }
        }))
        .unwrap();
        let m = n.error.misalignment.expect("misalignment 가 읽혀야 한다");
        assert_eq!(
            m.detailed_explanation.as_deref(),
            Some("이 요청은 정책을 벗어난다")
        );
        assert_eq!(m.error_type.as_deref(), Some("somethingBrandNew"));
        assert_eq!(m.steer.unwrap().message, "다르게 물어보세요");
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
