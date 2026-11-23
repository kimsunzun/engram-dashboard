//! ControlIngress seam (ADR-0086 step 2) — the common pipeline of the dual entrance (MCP + CLI).
//!
//! The two entrances (MCP `eg_send` tool · `/control/send` HTTP route) each build a normalized `ControlCommand` and call the single `handle_send`; what is below it
//! (Validator·Relay·ACK) is entrance-agnostic, preventing per-entrance duplication and drift.
//!
//! Invariants (ADR-0086):
//!   - `from` (sender identity) is derived only from the token/session, never the payload — both adapters build the ControlCommand with the BoundIdentity (blocks impersonation).
//!   - ACK/error JSON has the same shape on both entrances (same code), as does the self-correction roster (RECIPIENT_* hint).
//!   - All syntax validation of send arguments is owned here; it finishes before delegating to the service.

use std::sync::Arc;
use std::time::Instant;

use engram_dashboard_agent::manager::AgentManager;
use engram_dashboard_messaging::envelope::{new_msg_id, Entrance, EnvelopeFormat};

use super::registry::{BoundIdentity, ControlRegistry};

/// The cap on the body string itself — separate from the MCP route's transport-layer cap (`mcp_server` `RequestBodyLimitLayer`).
const MAX_BODY_BYTES: usize = 64 * 1024;

/// The normalized control command (ADR-0086) both entrances build and hand to `handle_send`.
#[derive(Debug, Clone)]
pub struct ControlCommand {
    pub from: BoundIdentity,
    /// Each element = an agent name · an exact AgentId string · an `@`address (mixable). Comma splitting is CLI-entrance only and done by the adapter (MCP array elements are never split — spec §6).
    pub to: Vec<String>,
    /// Plain text (attachments · structuring are out of scope).
    pub body: String,
    pub contract: SendContract,
}

/// Reply-contract send arguments (C3 · spec §6 `eg_send { …, request?, reply_by?, reply_to? }`).
///
/// A separate struct since all three are optional and `Default` is "notification": flat in `ControlCommand`, every plain-send builder (tests, smoke bins) would write three fields.
/// Adapters carry raw values; validation is only in `validate_contract`.
// ADR-0103 (decisions 2/3 — request type + reply contract)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SendContract {
    /// `true` = reply required (an unreplied ledger entry + envelope `type="request"`). Multiple recipients and `@all` are allowed: one independent contract per recipient (ADR-0111 decision 5).
    pub request: bool,
    /// Reply deadline as a duration notation (`"5m"`/`"10m"`/`"1h"`, at least 1 minute). `request` only — alone it is bounced.
    pub reply_by: Option<String>,
    /// Which request this replies to (the original message id). Mutually exclusive with `request` (spec §6).
    pub reply_to: Option<String>,
}

/// Cap of the `reply_by` duration notation — 30 days; over is bounced.
///
/// Load-bearing for daemon stability: the ledger computes `created_at + reply_by` (`Instant + Duration`), which panics on overflow — without a cap one `--reply-by 99999999999999s`
/// could kill the sweep task. 30 days suffices for the in-memory stage (parking TTL 24h; a restart wipes the ledger).
const MAX_REPLY_BY_SECS: u64 = 30 * 24 * 60 * 60;

/// Floor of `reply_by` — 1 minute; under is bounced.
///
/// Load-bearing for contract honesty: the overdue decision runs only on the sweep period (60 s, lib.rs `SWEEP_INTERVAL`), so a `30s` promise would be noticed 60~120 s later.
/// The `s` unit stays accepted (`120s` is valid) — what is blocked is the value size, not the notation.
const MIN_REPLY_BY_SECS: u64 = 60;

/// The `reply_by` duration notation parser (spec §3) — strict: this value is written freely by the sending LLM, so a lenient guess (`"10"` as minutes) becomes a silent contract
/// error; when ambiguous, bounce and show the form in the hint so it self-corrects. `0` is blocked (overdue the instant it is sent — more likely a typo).
pub(crate) fn parse_reply_by(s: &str) -> Result<std::time::Duration, String> {
    let bytes = s.as_bytes();
    // Cuts by bytes, so multibyte characters are caught at the unit match.
    if bytes.len() < 2 {
        return Err(format!(
            "reply_by '{s}' is not a duration; use an integer with a unit like 5m, 10m, or 1h (minimum 1 minute)."
        ));
    }
    let (digits, unit) = bytes.split_at(bytes.len() - 1);
    let unit_secs: u64 = match unit[0] {
        b's' => 1,
        b'm' => 60,
        b'h' => 3600,
        _ => {
            return Err(format!(
                "reply_by '{s}' has an unknown unit; use s (seconds), m (minutes), or h (hours) — e.g. 10m."
            ))
        }
    };
    if !digits.iter().all(|c| c.is_ascii_digit()) {
        return Err(format!(
            "reply_by '{s}' must be a plain integer plus one unit (no spaces or signs) — e.g. 5m, 10m, 1h."
        ));
    }
    let n: u64 = std::str::from_utf8(digits)
        .expect("ascii digits")
        .parse()
        .map_err(|_| {
            format!("reply_by '{s}' is out of range; use at most 30d worth (e.g. 24h).")
        })?;
    if n == 0 {
        return Err(format!(
            "reply_by '{s}' must be greater than zero; drop the flag if you don't want a deadline."
        ));
    }
    let secs = n.checked_mul(unit_secs).ok_or_else(|| {
        format!("reply_by '{s}' is out of range; use at most 30d worth (e.g. 24h).")
    })?;
    if secs < MIN_REPLY_BY_SECS {
        return Err(format!(
            "reply_by '{s}' is shorter than the 1-minute minimum; deadlines are checked once a minute, so anything shorter cannot be honoured — use 1m or longer (60s is fine)."
        ));
    }
    if secs > MAX_REPLY_BY_SECS {
        return Err(format!(
            "reply_by '{s}' exceeds the 30-day maximum; use a shorter deadline (e.g. 24h)."
        ));
    }
    Ok(std::time::Duration::from_secs(secs))
}

/// Argument consistency validation — bounces at the first violation; every code is `INVALID_SEND_ARGS` (the caller attaches it).
pub(crate) fn validate_contract(
    c: &SendContract,
) -> Result<engram_dashboard_messaging::service::SendMeta, String> {
    if c.request && c.reply_to.is_some() {
        return Err(
            "request and reply_to are mutually exclusive: a message is either a new request or a reply to one. Send two messages if you need both."
                .to_string(),
        );
    }
    if c.reply_by.is_some() && !c.request {
        return Err(
            "reply_by is only meaningful with request=true (it is the deadline of the reply contract you are opening). Add request, or drop reply_by."
                .to_string(),
        );
    }
    let reply_by = match &c.reply_by {
        Some(raw) => Some(parse_reply_by(raw)?),
        None => None,
    };
    let reply_to = match &c.reply_to {
        Some(raw) => {
            // trim as entrance normalization (intentional, kept after review): the trimmed value is used as is for both the envelope render and the ledger's strict matching (one
            //   `SendMeta.reply_to`), so matching does not loosen; untrimmed, `" m-7f3k"` would silently miss as NoMatch.
            let t = raw.trim();
            if t.is_empty() {
                return Err(
                    "reply_to must be the id of the request you are answering (e.g. m-7f3k9q2d); it cannot be empty."
                        .to_string(),
                );
            }
            Some(t.to_string())
        }
        None => None,
    };
    Ok(engram_dashboard_messaging::service::SendMeta {
        request: c.request,
        reply_by_raw: c.reply_by.clone(),
        reply_by,
        reply_to,
        to_attr: None,
    })
}

/// `reply_to` notation rule (spec §3 item 7-①) — a reply is valid only with exactly 1 recipient.
///
/// Decided at the notation stage (load-bearing): an `@`token refuses even if it expands to 1 person — counting after expansion would make the same send pass or bounce depending on roster state, and the LLM could not learn the rule.
fn reply_to_has_single_recipient(recipients: &[String]) -> bool {
    recipients.len() == 1 && !recipients[0].starts_with('@')
}

/// Are the contract fields (request/reply_to) inexpressible on the current envelope render path (load-bearing)?
///
/// Bounced to refuse silent degradation: the colon render and the `ENGRAM_WRAP_FORMAT` template drop the id·type·reply-by·in-reply-to attributes, so a request there has a fixed ending —
/// the recipient never sees the id to reply with, the ledger demands strict matching (spec §2), and after the deadline a "no reply" notice always fires (a guaranteed false timeout); a reply
/// also loses `in-reply-to`.
/// Env is read here because the template is a global switch `wrap_message` checks first; the decision and the render must look at the same input.
pub(crate) fn contract_unsupported_by_envelope(
    meta: &engram_dashboard_messaging::service::SendMeta,
    format: EnvelopeFormat,
) -> Option<String> {
    if !(meta.request || meta.reply_to.is_some()) {
        return None;
    }
    let template_active = std::env::var("ENGRAM_WRAP_FORMAT").is_ok_and(|v| !v.is_empty());
    if format == EnvelopeFormat::Xml && !template_active {
        return None;
    }
    Some(
        "Reply contracts (request / reply_to) need the XML envelope: the envelope format in effect drops the id / type / reply-by / in-reply-to attributes, so the recipient could never reply and you would get a false timeout notice. Send this as a plain notification instead, or switch the envelope format back to xml."
            .to_string(),
    )
}

/// The send result for one recipient — a spec §6 `results[]` element (one row per recipient, not a group summary).
#[derive(Debug, Clone)]
pub struct SendResult {
    pub to: String,
    pub status: &'static str,
    pub code: Option<&'static str>,
    pub hint: Option<String>,
}

/// The control command processing result — `to_json` makes the wire JSON.
///
/// Parser branching (accepted asymmetry): a success has no top-level `status` and a bounce has no `results`; the receiver branches on whether `status == "error"` exists (shapes not unified — existing contract).
#[derive(Debug, Clone)]
pub enum ControlResult {
    Ok {
        id: String,
        results: Vec<SendResult>,
    },
    Error {
        code: &'static str,
        hint: String,
    },
}

impl ControlResult {
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            ControlResult::Ok { id, results } => {
                let arr: Vec<serde_json::Value> = results
                    .iter()
                    .map(|r| {
                        let mut obj = serde_json::json!({ "to": r.to, "status": r.status });
                        if let Some(c) = r.code {
                            obj["code"] = serde_json::Value::String(c.to_string());
                        }
                        if let Some(h) = &r.hint {
                            obj["hint"] = serde_json::Value::String(h.clone());
                        }
                        obj
                    })
                    .collect();
                serde_json::json!({ "id": id, "results": arr })
            }
            ControlResult::Error { code, hint } => serde_json::json!({
                "status": "error",
                "code": code,
                "hint": hint,
            }),
        }
    }

    /// A row failure is not a failure here: partial progress is normal (ADR-0111 decision 3), so "some did not get it" is for the sender to read in `results[]`; even if everyone failed it is accepted — only send-level bounces (argument errors · address space errors) are failures.
    pub fn is_accepted(&self) -> bool {
        matches!(self, ControlResult::Ok { .. })
    }
}

/// The dual-entrance common handler — what the two adapters (MCP tool · HTTP route) call, through [`handle_send_at`] with their arrival instant.
///
/// Check order (send-level bounce at the first failure):
///   0. Reply-contract argument consistency → `INVALID_SEND_ARGS`, before addresses: a pure syntax error must answer the same regardless of roster state.
///   0-b. Contract fields are XML-envelope only → `INVALID_SEND_ARGS`.
///   1. Normalize the recipient list — if nothing is left, `INVALID_SEND_ARGS`.
///   2. `reply_to` needs exactly 1 recipient → otherwise `INVALID_SEND_ARGS`.
///   3. body cap → `BODY_TOO_LARGE`.
///   4. Everything else is delegated to MessagingService; only address space errors (`GROUP_NOT_FOUND`/`GROUP_EMPTY`) and id collisions come back as send-level bounces.
///
/// Missing recipients are not filtered here (ADR-0111 decision 1): the service decides existence and same-name per recipient from one roster snapshot at send time; a second snapshot
/// brings back half-decisions (the old pre-check's TOCTOU), so the entrance does not look at the roster. Self-send is allowed (spec §4).
// ADR-0086
// ADR-0111
pub fn handle_send(
    manager: &Arc<AgentManager>,
    registry: &Arc<ControlRegistry>,
    messaging: &Arc<engram_dashboard_messaging::service::MessagingService>,
    entrance: Entrance,
    cmd: ControlCommand,
) -> ControlResult {
    handle_send_at(manager, registry, messaging, entrance, cmd, Instant::now())
}

/// [`handle_send`] for a request that arrived at `arrival` — rows for agents on other PCs wait for their outcome until
///   `arrival + CommandDeliveries::DEFAULT_DEADLINE` (the deadline itself, not a copy, so the caller's silence window still binds it), then
///   what has no outcome yet is answered unconfirmed (`MessagingService::await_remote_answer`). Blocking — the entrances call it on the
///   blocking pool with the instant their request came in.
// ADR-0393
pub fn handle_send_at(
    manager: &Arc<AgentManager>,
    registry: &Arc<ControlRegistry>,
    messaging: &Arc<engram_dashboard_messaging::service::MessagingService>,
    entrance: Entrance,
    cmd: ControlCommand,
    arrival: Instant,
) -> ControlResult {
    use engram_dashboard_messaging::service::{SendReject, SendStatus};

    let meta = match validate_contract(&cmd.contract) {
        Ok(m) => m,
        Err(hint) => {
            return ControlResult::Error {
                code: "INVALID_SEND_ARGS",
                hint,
            }
        }
    };

    if let Some(hint) = contract_unsupported_by_envelope(&meta, registry.envelope_format()) {
        return ControlResult::Error {
            code: "INVALID_SEND_ARGS",
            hint,
        };
    }

    // Trim, but no correction beyond that: the name is a byte-exact address namespace (WYSIWYA — ADR-0101).
    let recipients: Vec<String> = cmd
        .to
        .iter()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect();
    if recipients.is_empty() {
        return ControlResult::Error {
            code: "INVALID_SEND_ARGS",
            hint: "to must name at least one recipient — pass an agent name, an agent id, @here (everyone live except you), or @all (every agent in the tree, including ones that are not running, except you). A list is allowed."
                .to_string(),
        };
    }

    if meta.reply_to.is_some() && !reply_to_has_single_recipient(&recipients) {
        return ControlResult::Error {
            code: "INVALID_SEND_ARGS",
            hint: "A reply goes to exactly one recipient — the agent that sent the request. Drop the extra recipients (and any @address) and send the reply to that one agent; broadcast any follow-up as a separate message."
                .to_string(),
        };
    }

    if cmd.body.len() > MAX_BODY_BYTES {
        return ControlResult::Error {
            code: "BODY_TOO_LARGE",
            hint: format!(
                "Message body exceeds the {MAX_BODY_BYTES}-byte limit; shorten it and retry."
            ),
        };
    }

    let mut msg_id = new_msg_id();
    if !registry.is_identity_live(cmd.from) {
        // Do not carry body/token in this log (security).
        tracing::warn!(
            from = %cmd.from.agent_id,
            from_epoch = cmd.from.epoch,
            msg_id = %msg_id,
            entrance = entrance.as_str(),
            "control channel message send — the sender is no longer a live identity at relay time (valid by authentication at write time; not a gate, an observation for the record; ADR-0086 · user decision 2026-07-19)"
        );
    }

    let sender_name = sender_display_name(manager, cmd.from);

    #[cfg(feature = "test-harness")]
    registry.fire_mid_send_hook();

    let mut outcome = messaging.handle_send(
        &msg_id,
        cmd.from.into(),
        &sender_name,
        &recipients,
        &cmd.body,
        entrance,
        &meta,
    );
    if matches!(outcome, Err(SendReject::IdCollision)) {
        let collided = std::mem::replace(&mut msg_id, new_msg_id());
        tracing::error!(
            collided = %collided,
            replacement = %msg_id,
            entrance = entrance.as_str(),
            "message id collision — retrying once with a new id (ADR-0103 · a practically impossible path, so suspect the random/ledger wiring)"
        );
        outcome = messaging.handle_send(
            &msg_id,
            cmd.from.into(),
            &sender_name,
            &recipients,
            &cmd.body,
            entrance,
            &meta,
        );
    }

    if let Ok(rows) = &mut outcome {
        messaging.await_remote_answer(
            &msg_id,
            rows,
            arrival + crate::command_delivery::CommandDeliveries::DEFAULT_DEADLINE,
        );
    }

    match outcome {
        Ok(rows) => ControlResult::Ok {
            id: msg_id,
            results: rows
                .into_iter()
                .map(|r| SendResult {
                    to: r.to,
                    status: match r.status {
                        SendStatus::Delivered => "delivered",
                        SendStatus::Pending => "pending",
                        SendStatus::Failed => "failed",
                    },
                    code: r.code.map(|c| c.as_str()),
                    hint: r.hint,
                })
                .collect(),
        },
        Err(SendReject::GroupNotFound { name }) => ControlResult::Error {
            code: "GROUP_NOT_FOUND",
            hint: format!(
                "'{name}' is not an address the broker knows. The only group addresses are @here (everyone live except you) and @all (every agent in the tree, including ones that are not running, except you); everything else must be an agent name or agent id. Fix the address and send again."
            ),
        },
        Err(SendReject::GroupEmpty) => ControlResult::Error {
            code: "GROUP_EMPTY",
            hint: "That send resolved to no recipients at all — nothing was sent. For @here it means nobody other than you is live right now; for @all it means the team tree holds nobody but you. Name an agent directly (including yourself) if you meant to reach someone specific."
                .to_string(),
        },
        Err(SendReject::IdCollision) => ControlResult::Error {
            code: "INTERNAL_ID_COLLISION",
            hint: "The daemon could not allocate a unique message id; retry the send.".to_string(),
        },
    }
}

// ── query entrance (D · spec §6 `eg_messages`) ─────────────────────────────────────────────────────
//
// No spawn_blocking: this path has no blocking write to child stdin (no inject); the only lock is the messaging state with a short pure-data critical section (port calls are outside it).
// The reason `handle_send` uses the blocking pool (a blocked pipe) does not hold here.

/// The result of a query command — the error shape is the same as send (`ControlResult`, `{status:"error", code, hint}`) so the sending LLM and CLI read success/failure by one rule.
///
/// A different type from send: a send success has a fixed shape `{id, results[]}`, a query success differs per verb; one enum would grow a variant per verb and blur the contract, so
/// the success payload belongs to its maker and only the error axis is shared.
// PartialEq, not Eq: `serde_json::Value` holds f64.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlQueryResult {
    Ok(serde_json::Value),
    Error { code: &'static str, hint: String },
}

impl ControlQueryResult {
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            ControlQueryResult::Ok(v) => v.clone(),
            ControlQueryResult::Error { code, hint } => serde_json::json!({
                "status": "error",
                "code": code,
                "hint": hint,
            }),
        }
    }

    /// The consuming sibling that gives out the value — for large success payloads: [`ControlQueryResult::to_json`] deep-copies the whole payload on success, which costs CPU·memory per request when
    /// the payload reaches several MB (the command catalog). Callers with no need to reuse the envelope (all HTTP handlers) use this.
    pub fn into_json(self) -> serde_json::Value {
        match self {
            ControlQueryResult::Ok(v) => v,
            other => other.to_json(),
        }
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, ControlQueryResult::Ok(_))
    }
}

/// `messages { id? }` common handler (D · spec §6) — read-only: it only queries the ledger.
///
/// Response shape (contract, the same on both entrances):
///
/// `id` given = that message's delivery ledger. A multi-recipient or `@all` send is one row per recipient (1 msg_id : N delivery records):
/// ```json
/// { "id": "m-7f3k", "from": "alice", "awaiting_reply": false, "may_be_truncated": false,
///   "rows": [ { "to": "bob", "status": "delivered", "age_secs": 42, "updated_secs_ago": 40 } ] }
/// ```
/// - `status` = ledger vocabulary (`pending|delivered|replied|expired|failed|skipped`), not the send response vocabulary (`delivered|pending|failed`) — different axes joined by the spec §6 mapping table:
///   the response is an instant snapshot of how the send call ended, the query is how far that delivery record has gone now.
/// - `code`/`hint` appear only when present on the row — the reason of a row terminated after the fact (today only `RECIPIENT_DELETED`).
/// - `awaiting_reply` = this message is a request and no reply has come yet (always false for a notification).
/// - `may_be_truncated` = no guarantee `rows` is all of that message (the in-memory history ring may have pushed out the front rows); `false` means certainly all. Always carried:
///   carrying it only when true would make its absence read as completeness (10-person broadcast with 6 rows pushed out would read "went to only 4"). When `true`, `hint` is carried too.
/// - Unknown id → `{status:"error", code:"MESSAGE_NOT_FOUND", hint}`.
///
/// No `id` = the caller's open items (three branches in one list, oldest first):
/// ```json
/// { "me": "alice", "open": [
///     { "direction": "outbound_pending",     "id": "m-1", "from": "alice", "to": "ghost", "age_secs": 90 },
///     { "direction": "awaiting_their_reply", "id": "m-2", "from": "alice", "to": "bob",   "age_secs": 30,
///       "reply_by": "10m", "timed_out": false },
///     { "direction": "reply_owed_by_me",     "id": "m-3", "from": "carol", "to": "alice", "age_secs": 5 } ] }
/// ```
/// - `direction` = what this line is (a stable token); the three values have opposite things to do, so the tag is required.
/// - `reply_by`·`timed_out` appear only on request lines. With no open items, `open: []`.
///
/// Caller matching = id (ADR-0309): open items are picked by the agent id of the caller's identity (`open_items_for(from.agent_id, …)`); the display name
/// (`sender_display_name`, the same computation as the send envelope · WYSIWYA — ADR-0101) is only for output.
pub fn handle_messages(
    manager: &Arc<AgentManager>,
    messaging: &Arc<engram_dashboard_messaging::service::MessagingService>,
    from: BoundIdentity,
    id: Option<&str>,
) -> ControlQueryResult {
    let now = std::time::Instant::now();
    match id {
        Some(raw) => {
            let msg_id = raw.trim();
            if msg_id.is_empty() {
                return ControlQueryResult::Error {
                    code: "MESSAGE_NOT_FOUND",
                    hint: "Pass the message id you want to inspect (e.g. m-7f3k9q2d), or call this tool with no arguments to list your own open items.".to_string(),
                };
            }
            let Some(view) = messaging.message_state(msg_id, now) else {
                return ControlQueryResult::Error {
                    code: "MESSAGE_NOT_FOUND",
                    hint: format!(
                        "No message '{msg_id}' in the ledger. Ids look like m-7f3k9q2d; very old messages fall out of the in-memory history, and the ledger is cleared when the daemon restarts."
                    ),
                };
            };
            let rows: Vec<serde_json::Value> = view
                .rows
                .iter()
                .map(|r| {
                    let mut row = serde_json::json!({
                        "to": r.to,
                        "status": r.status,
                        "age_secs": r.age_secs,
                        "updated_secs_ago": r.updated_secs_ago,
                    });
                    // Carried only when present: `code: null` on a normal row would make every row read as having a reason.
                    if let Some(code) = r.code {
                        row["code"] = serde_json::Value::String(code.to_string());
                    }
                    if let Some(hint) = &r.hint {
                        row["hint"] = serde_json::Value::String(hint.clone());
                    }
                    row
                })
                .collect();
            let mut out = serde_json::json!({
                "id": view.id,
                "from": view.from,
                "awaiting_reply": view.awaiting_reply,
                "may_be_truncated": view.may_be_truncated,
                "rows": rows,
            });
            if view.may_be_truncated {
                out["hint"] = serde_json::Value::String(
                    "The in-memory ledger has rotated, so some delivery rows for this message may already be gone — treat the list below as partial, not as the full set of recipients.".to_string(),
                );
            }
            ControlQueryResult::Ok(out)
        }
        None => {
            let me = sender_display_name(manager, from);
            let open: Vec<serde_json::Value> = messaging
                .open_items_for(from.agent_id, now)
                .iter()
                .map(|i| {
                    let mut obj = serde_json::json!({
                        "direction": i.direction.as_str(),
                        "id": i.id,
                        "from": i.from,
                        "to": i.to,
                        "age_secs": i.age_secs,
                    });
                    if let Some(rb) = &i.reply_by {
                        obj["reply_by"] = serde_json::Value::String(rb.clone());
                    }
                    if i.reply_by.is_some() || i.timed_out {
                        obj["timed_out"] = serde_json::Value::Bool(i.timed_out);
                    }
                    obj
                })
                .collect();
            ControlQueryResult::Ok(serde_json::json!({ "me": me, "open": open }))
        }
    }
}

/// ADR-0101 (WYSIWYA): the envelope sender name must be byte-identical to the name the recipient sees in roster · tree · routing, or the recipient gets "A: hi" while A shows as another string and targeting misses.
fn sender_display_name(manager: &Arc<AgentManager>, from: BoundIdentity) -> String {
    if let Some(name) = manager.canonical_name(from.agent_id) {
        return name;
    }
    // The session was reaped (sender terminal). `profile.cwd` is raw and may differ from canonical, but there is no live session, so it is not a routing target either (display-only best-effort).
    // Do not change it to `canonical_name_when_live()` (behavior preservation): it skips fs when display_name exists, so cwd normalization diverges and the result differs.
    if let Some(p) = manager.agent_snapshot(from.agent_id) {
        return engram_dashboard_agent::name::canonical_name_or_id_fallback(
            p.display_name.as_deref(),
            &p.cwd.to_string_lossy(),
            from.agent_id,
        );
    }
    let s = from.agent_id.to_string();
    s[..8.min(s.len())].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_dashboard_agent::types::AgentId;

    fn ident(id: AgentId) -> BoundIdentity {
        BoundIdentity {
            agent_id: id,
            epoch: 0,
        }
    }
    use engram_dashboard_messaging::envelope::wrap_message;

    /// ENGRAM_WRAP_FORMAT is a process-global env — tests asserting set/remove · unset are serialized so one test's set does not trample another's "unset" assertion.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // ── ControlResult wire shape ──────────────────────────────
    #[test]
    fn ok_delivered_json_shape() {
        let r = ControlResult::Ok {
            id: "mid".to_string(),
            results: vec![SendResult {
                to: "bob".to_string(),
                status: "delivered",
                code: None,
                hint: None,
            }],
        };
        let v = r.to_json();
        assert_eq!(v["id"], "mid");
        assert_eq!(v["results"][0]["to"], "bob");
        assert_eq!(v["results"][0]["status"], "delivered");
        assert!(
            v["results"][0].get("hint").is_none(),
            "delivered omits the hint"
        );
        assert!(
            v.get("status").is_none(),
            "a success response has no top-level status (spec §6)"
        );
        assert!(r.is_accepted());
    }

    #[test]
    fn ok_pending_json_shape_includes_hint() {
        let r = ControlResult::Ok {
            id: "mid".to_string(),
            results: vec![SendResult {
                to: "ghost".to_string(),
                status: "pending",
                code: None,
                hint: Some("parked".to_string()),
            }],
        };
        let v = r.to_json();
        assert_eq!(v["results"][0]["status"], "pending");
        assert_eq!(v["results"][0]["hint"], "parked");
        assert!(
            r.is_accepted(),
            "pending is an accepted success too (not a bounce)"
        );
    }

    #[test]
    fn error_json_shape() {
        let r = ControlResult::Error {
            code: "GROUP_NOT_FOUND",
            hint: "h".to_string(),
        };
        let v = r.to_json();
        assert_eq!(v["status"], "error");
        assert_eq!(v["code"], "GROUP_NOT_FOUND");
        assert_eq!(v["hint"], "h");
        assert!(!r.is_accepted(), "a bounce is not an accepted success");
    }

    // ── ControlCommand normalization ──────────────
    #[test]
    fn control_command_carries_identity_not_payload_from() {
        let id = AgentId::new_v4();
        let cmd = ControlCommand {
            from: ident(id),
            to: vec!["bob".to_string()],
            body: "hi".to_string(),
            contract: Default::default(),
        };
        assert_eq!(cmd.from.agent_id, id);
    }

    // ── C3: reply_by duration notation parser ──────────────────────────────────────────────────────────

    #[test]
    fn reply_to_requires_exactly_one_non_group_recipient() {
        let one = |v: &[&str]| {
            reply_to_has_single_recipient(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>())
        };
        assert!(one(&["bob"]), "1 recipient = valid");
        assert!(!one(&["bob", "carol"]), "multiple recipients = bounce");
        assert!(
            !one(&["@all"]),
            "an @token alone is bounced too — blocked at the notation stage even if the expansion is 1 person (no nondeterminism)"
        );
        assert!(!one(&["@all", "bob"]), "mixing is bounced too");
        assert!(!one(&[]), "an empty list is not valid in the first place");
    }

    #[test]
    fn parse_reply_by_accepts_integer_plus_unit() {
        use std::time::Duration;
        assert_eq!(
            parse_reply_by("10m").expect("10m"),
            Duration::from_secs(600)
        );
        assert_eq!(parse_reply_by("1h").expect("1h"), Duration::from_secs(3600));
        assert_eq!(parse_reply_by("60s").expect("60s"), Duration::from_secs(60));
        assert_eq!(
            parse_reply_by("120s").expect("120s"),
            Duration::from_secs(120)
        );
        assert_eq!(
            parse_reply_by("720h").expect("720h = 30d"),
            Duration::from_secs(30 * 24 * 3600)
        );
    }

    #[test]
    fn parse_reply_by_rejects_below_one_minute_floor() {
        for bad in ["1s", "30s", "59s"] {
            let err = parse_reply_by(bad).expect_err("under 1 minute is bounced");
            assert!(
                err.contains("1-minute minimum"),
                "the hint must tell the floor: {err}"
            );
        }
        assert!(
            parse_reply_by("60s").is_ok(),
            "exactly 1 minute is accepted"
        );
        assert!(
            parse_reply_by("1m").is_ok(),
            "1m is the same value, so accepted"
        );
    }

    #[test]
    fn parse_reply_by_rejects_malformed_forms() {
        for bad in [
            "", "s", "m", "10", "10 m", " 10m", "10m ", "10M", "1H", "10x", "-5m", "1.5h", "1h30m",
            "0s", "0m", "0h", "십분", "10초", // hangul-ok
        ] {
            assert!(
                parse_reply_by(bad).is_err(),
                "'{bad}' must be bounced (strict parser)"
            );
        }
    }

    #[test]
    fn parse_reply_by_rejects_overflow_and_beyond_cap() {
        assert!(parse_reply_by("721h").is_err(), "over 30 days is bounced");
        assert!(
            parse_reply_by("18446744073709551615s").is_err(),
            "the u64 maximum is bounced at the cap too"
        );
        assert!(
            parse_reply_by("99999999999999999999s").is_err(),
            "u64 parse overflow is bounced too"
        );
        assert!(
            parse_reply_by("18446744073709551615h").is_err(),
            "multiplication overflow is bounced too"
        );
    }

    // ── C3 review fix 1: contract fields are XML-envelope only ─────────────────────────────────────────────

    fn req_meta() -> engram_dashboard_messaging::service::SendMeta {
        engram_dashboard_messaging::service::SendMeta {
            request: true,
            reply_by_raw: Some("10m".to_string()),
            reply_by: Some(std::time::Duration::from_secs(600)),
            reply_to: None,
            to_attr: None,
        }
    }
    fn reply_meta() -> engram_dashboard_messaging::service::SendMeta {
        engram_dashboard_messaging::service::SendMeta {
            request: false,
            reply_by_raw: None,
            reply_by: None,
            reply_to: Some("m-7f3k".to_string()),
            to_attr: None,
        }
    }

    #[test]
    fn contract_fields_are_rejected_under_colon_envelope() {
        let _g = ENV_LOCK.lock().unwrap();
        assert!(
            std::env::var("ENGRAM_WRAP_FORMAT").is_err(),
            "premise: the template env is unset"
        );
        for meta in [req_meta(), reply_meta()] {
            let hint = contract_unsupported_by_envelope(&meta, EnvelopeFormat::Colon)
                .expect("contract fields are bounced under colon");
            assert!(hint.contains("XML envelope"), "correction hint: {hint}");
        }
        assert_eq!(
            contract_unsupported_by_envelope(
                &engram_dashboard_messaging::service::SendMeta::default(),
                EnvelopeFormat::Colon
            ),
            None,
            "a notification is allowed regardless of format (existing behavior unchanged)"
        );
        assert_eq!(
            contract_unsupported_by_envelope(&req_meta(), EnvelopeFormat::Xml),
            None
        );
    }

    #[test]
    fn contract_fields_are_rejected_when_wrap_template_env_is_active() {
        let _g = ENV_LOCK.lock().unwrap();
        assert!(
            std::env::var("ENGRAM_WRAP_FORMAT").is_err(),
            "premise: no value left behind by another test"
        );
        std::env::set_var("ENGRAM_WRAP_FORMAT", "<{sender}#{id}> {body}");
        let under_template = contract_unsupported_by_envelope(&req_meta(), EnvelopeFormat::Xml);
        let plain_under_template = contract_unsupported_by_envelope(
            &engram_dashboard_messaging::service::SendMeta::default(),
            EnvelopeFormat::Xml,
        );
        std::env::set_var("ENGRAM_WRAP_FORMAT", "");
        let under_empty = contract_unsupported_by_envelope(&req_meta(), EnvelopeFormat::Xml);
        std::env::remove_var("ENGRAM_WRAP_FORMAT");

        assert!(
            under_template.is_some(),
            "with the template on, contract fields are bounced even under the xml format"
        );
        assert_eq!(
            plain_under_template, None,
            "a notification passes even under the template (the existing spike path unchanged)"
        );
        assert_eq!(under_empty, None, "empty value = treated as unset");
    }

    // ── C3: send argument consistency (spec §6 mutual exclusion) ──────────────────────────────────────────────────

    #[test]
    fn contract_default_is_plain_notice_message() {
        let m =
            validate_contract(&SendContract::default()).expect("a notification is always valid");
        assert!(!m.request);
        assert_eq!(m.reply_by, None);
        assert_eq!(m.reply_to, None);
    }

    #[test]
    fn contract_request_and_reply_to_together_is_rejected() {
        let c = SendContract {
            request: true,
            reply_by: None,
            reply_to: Some("m-7f3k".to_string()),
        };
        let e = validate_contract(&c).expect_err("mutually exclusive (spec §6)");
        assert!(
            e.contains("mutually exclusive"),
            "the hint must state the reason: {e}"
        );
    }

    #[test]
    fn contract_reply_by_without_request_is_rejected() {
        let c = SendContract {
            request: false,
            reply_by: Some("10m".to_string()),
            reply_to: None,
        };
        let e = validate_contract(&c).expect_err("reply_by is request-only");
        assert!(e.contains("request"), "the hint must state the fix: {e}");
    }

    #[test]
    fn contract_bad_reply_by_notation_is_rejected_with_form_hint() {
        let c = SendContract {
            request: true,
            reply_by: Some("ten minutes".to_string()),
            reply_to: None,
        };
        let e = validate_contract(&c).expect_err("notation error");
        assert!(e.contains("10m"), "example of an allowed form: {e}");
        // The hint gives only examples valid now (pair of review fix 7): `30s` left as an example after the floor would bounce a sender that followed it (a self-correction loop); an example must always pass.
        for example in ["30s", "1s", "59s"] {
            assert!(
                !e.contains(example),
                "the hint must not give a value that gets bounced as an example ('{example}'): {e}"
            );
        }
    }

    #[test]
    fn contract_empty_reply_to_is_rejected_and_whitespace_is_trimmed() {
        let empty = SendContract {
            request: false,
            reply_by: None,
            reply_to: Some("   ".to_string()),
        };
        assert!(
            validate_contract(&empty).is_err(),
            "a whitespace-only reply_to is bounced"
        );

        let padded = SendContract {
            request: false,
            reply_by: None,
            reply_to: Some("  m-7f3k  ".to_string()),
        };
        let m = validate_contract(&padded).expect("valid after trim");
        assert_eq!(
            m.reply_to.as_deref(),
            Some("m-7f3k"),
            "leading and trailing whitespace is trimmed"
        );
    }

    #[test]
    fn valid_request_contract_carries_both_raw_and_parsed_deadline() {
        let c = SendContract {
            request: true,
            reply_by: Some("10m".to_string()),
            reply_to: None,
        };
        let m = validate_contract(&c).expect("valid");
        assert!(m.request);
        assert_eq!(m.reply_by_raw.as_deref(), Some("10m"));
        assert_eq!(m.reply_by, Some(std::time::Duration::from_secs(600)));
    }

    // ── C3: envelope attribute assembly (exposure principle — spec §1) ────────────────────────────────────────────────

    #[test]
    fn request_envelope_exposes_id_type_reply_by_only() {
        let _g = ENV_LOCK.lock().unwrap();
        let m = validate_contract(&SendContract {
            request: true,
            reply_by: Some("10m".to_string()),
            reply_to: None,
        })
        .expect("valid");
        let w = wrap_message(
            "qa-alpha",
            "m-7f3k",
            "write the code and reply",
            EnvelopeFormat::Xml,
            &m.envelope_fields("m-7f3k"),
        );
        assert_eq!(
            w,
            r#"<message from="qa-alpha" id="m-7f3k" type="request" reply-by="10m">write the code and reply</message>"#
        );
    }

    #[test]
    fn reply_envelope_exposes_only_in_reply_to() {
        let _g = ENV_LOCK.lock().unwrap();
        let m = validate_contract(&SendContract {
            request: false,
            reply_by: None,
            reply_to: Some("m-7f3k".to_string()),
        })
        .expect("valid");
        let w = wrap_message(
            "qa-bravo",
            "m-other",
            "all written",
            EnvelopeFormat::Xml,
            &m.envelope_fields("m-other"),
        );
        assert_eq!(
            w, r#"<message from="qa-bravo" in-reply-to="m-7f3k">all written</message>"#,
            "a reply has no id/type (exposure principle)"
        );
    }

    #[test]
    fn plain_envelope_has_no_attributes() {
        let _g = ENV_LOCK.lock().unwrap();
        let m = validate_contract(&SendContract::default()).expect("valid");
        let w = wrap_message(
            "alice",
            "m-zz",
            "build finished",
            EnvelopeFormat::Xml,
            &m.envelope_fields("m-zz"),
        );
        assert_eq!(w, r#"<message from="alice">build finished</message>"#);
    }

    #[test]
    fn query_result_error_shape_matches_the_send_entrance() {
        let e = ControlQueryResult::Error {
            code: "GROUP_NOT_FOUND",
            hint: "h".to_string(),
        };
        assert_eq!(
            e.to_json(),
            serde_json::json!({"status":"error","code":"GROUP_NOT_FOUND","hint":"h"})
        );
        assert!(!e.is_ok());
        let ok = ControlQueryResult::Ok(serde_json::json!({"groups":["@all"]}));
        assert_eq!(ok.to_json(), serde_json::json!({"groups":["@all"]}));
        assert!(ok.is_ok());
    }
}
