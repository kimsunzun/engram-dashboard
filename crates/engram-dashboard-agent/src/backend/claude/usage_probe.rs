//! claude's active usage query — sends one `get_usage` control request line to a short-lived `claude -p` (stream-json) and
//! decodes the response into [`UsageObservation`]. Callers see [`CLAUDE_USAGE_PROBE`] only as [`UsageProbe`] (ADR-0248).
//!
//! ★A request with no model call — not a single user message line is written★: the only thing going to stdin is one control request line.
//!   stdin is kept open until the response arrives (closing it may end the CLI before the response is received). Once the response arrives, stdin is closed,
//!   it waits briefly, then ends ([`finish_after_answer`]).
//! ★The response shape is an experimental feature★ (starting with the SDK method name `…_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET`) —
//!   measurements and sources = `docs/research/claude-usage-query-method-survey-2026-09-27.md` §3·§7 (claude 2.1.280). So
//!   if one field is wrong, only that field is dropped, and it is [`ProbeError::Parse`] only when the skeleton of the response is broken.
//! ★Response and error texts are not put in logs or error strings★ — account information may be in them. What is carried is only sentences and figures
//!   we wrote. The upstream original text goes only into the dedicated [`UsageDetail::upstream`] field, and even then only the values of named fields
//!   (the `error` of an error response · the two limit fields `rate_limits_available`·`rate_limits` · the `subscription_type` of 「no limit info」).
// ADR-0004

use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use chrono::DateTime;
use engram_dashboard_platform::shell::console_command;
use serde_json::{json, Map, Value};
use uuid::Uuid;

use super::usage::USAGE_VENDOR;
use super::{CLAUDE_PROGRAM, NO_AUTOUPDATE_ENV};
use crate::usage::{
    display_text, finish_after_answer, has_word, resets_at_from_epoch_secs, used_pct_from_percent,
    ProbeChild, ProbeCommand, ProbeEnv, ProbeError, ProbeFailure, ScopedWindowObs, ScratchDir,
    UpstreamText, UsageDetail, UsageObservation, UsagePolicy, UsageProbe, UsageSource,
    UsageVendorKey, WindowObs,
};

/// Minimum interval between automatic queries (user decision 2026-09-27 — the same value as Codex).
const CLAUDE_COOLDOWN: Duration = Duration::from_secs(15 * 60);

/// Time limit of one query. Measured 1.2–1.4 s (claude 2.1.280 · probe) — a stall guard that amply covers slow startup and network.
const CLAUDE_PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// The core arguments that **must be the same** as the stream-json mode of an agent spawn — in that mode the CLI requires this combination
/// (without `--verbose` it dies immediately — see that line's comment in `super::ClaudeBackend::build_spec`). Not derived from the spawn arguments but
/// written separately — the spawn side mixes in permission, session and control-channel arguments, so deriving would leak those changes into the query.
/// A test catches the two copies drifting apart.
const STREAM_JSON_CORE: [&str; 7] = [
    "-p",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--replay-user-messages",
    "--verbose",
];

const SETTINGS_FILE: &str = "settings.json";
const MCP_CONFIG_FILE: &str = "mcp.json";
/// Turns off all hooks — so hooks from user and project settings do not run in the query process.
const SETTINGS_JSON: &str = r#"{"disableAllHooks":true}"#;
/// No MCP servers — paired with `--strict-mcp-config`, so other MCP settings are not read.
const MCP_CONFIG_JSON: &str = r#"{"mcpServers":{}}"#;

/// What is kept while stripping keys starting with `CLAUDE` from the daemon env — the two pointing at the default login. Stripping these too
/// could query with a login other than the daemon's default account (TRD §3 #9).
const KEPT_CLAUDE_ENV: [&str; 2] = ["CLAUDE_CONFIG_DIR", "CLAUDE_CODE_OAUTH_TOKEN"];
const STRIPPED_ENV_PREFIX: &str = "CLAUDE";

/// env added to the query process — the first four are exactly the combination the probe (`get-usage-probe.mjs`) measured (`MAX_THINKING_TOKENS` is
/// the same value as an agent spawn's stream-json mode). ★What each of the four turns off was not checked beyond its name★.
/// The last one, [`NO_AUTOUPDATE_ENV`], is outside the probe combination — added for the same reason as an agent spawn (this five-item combination has not been run against a real claude).
const PROBE_ENV: [(&str, &str); 5] = [
    ("MAX_THINKING_TOKENS", "8000"),
    ("ENABLE_CLAUDEAI_MCP_SERVERS", "false"),
    ("CLAUDE_CODE_AUTO_CONNECT_IDE", "0"),
    ("CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL", "1"),
    NO_AUTOUPDATE_ENV,
];

/// Character cap for fields holding external strings (plan · model name) — they are display names, so longer ones are truncated.
const PLAN_MAX_CHARS: usize = 32;
const LABEL_MAX_CHARS: usize = 48;
/// Cap on the per-model window list. Today's responses have one or two.
const MODEL_SCOPED_MAX: usize = 16;

/// Named per-model weekly window fields (fixed fields of the SDK type) and the display names attached to them. Unlike `model_scoped[]` entries, the response does not give
/// a name, so we attach one.
const NAMED_MODEL_WINDOWS: [(&str, &str); 3] = [
    ("seven_day_opus", "Opus"),
    ("seven_day_sonnet", "Sonnet"),
    ("seven_day_oauth_apps", "OAuth apps"),
];

/// Response category words ([`UsageDetail::kind`]) — go out on the wire and are shown on screen without translation.
const KIND_RATE_LIMITS_NULL: &str = "rate_limits_null";
const KIND_CLAUDE_ERROR: &str = "claude_error";
const KIND_LIMITS_UNAVAILABLE: &str = "limits_unavailable";

/// claude usage prober. Callers use the singleton [`CLAUDE_USAGE_PROBE`].
pub(crate) struct ClaudeUsageProbe;

pub(crate) static CLAUDE_USAGE_PROBE: ClaudeUsageProbe = ClaudeUsageProbe;

impl UsageProbe for ClaudeUsageProbe {
    fn key(&self) -> UsageVendorKey {
        USAGE_VENDOR
    }

    fn policy(&self) -> UsagePolicy {
        UsagePolicy {
            cooldown: CLAUDE_COOLDOWN,
            timeout: CLAUDE_PROBE_TIMEOUT,
        }
    }

    fn query(&self, env: &ProbeEnv<'_>) -> Result<UsageObservation, ProbeFailure> {
        query_with(env, std::env::vars_os().map(|(key, _)| key))
    }
}

/// `daemon_env_keys` = all keys of the daemon process env (production = process env · test = stand-in).
fn query_with(
    env: &ProbeEnv<'_>,
    daemon_env_keys: impl IntoIterator<Item = OsString>,
) -> Result<UsageObservation, ProbeFailure> {
    env.require_time_left()?;
    // ★Declaration order sets drop order — the folder comes before the child★: the child must be dropped first (tree kill + wait for exit) so that
    //   Windows can delete that working folder and the settings files.
    let scratch = ScratchDir::create(env.scratch_root)?;
    let command = probe_command(scratch.path(), daemon_env_keys)?;
    let request_id = Uuid::new_v4().to_string();
    let mut child = env.spawner.spawn(&command, env.deadline)?;
    // If the wrapping shell could not find the target and already ended, the write fails first — returning right away would miss the exit
    //   status carrying 「not installed」. Holding the error, it reads until EOF and then decides by the exit.
    let write_error = match child.write_line(&request_line(&request_id), env.deadline) {
        Ok(()) => None,
        Err(err @ ProbeError::Io(_)) => Some(err),
        Err(err) => return Err(err.into()),
    };
    let response = await_response(child.as_mut(), &request_id, env.deadline, write_error)?;
    // ★Not killed right away★ — when stdin is closed claude ends by itself, but it takes about 1 second, during which it writes the global settings file
    //   (`.claude.json`) (measured 2026-09-27 · 2.1.280 · two logged-in queries = 1060 and 862 ms from closing to ending, exit
    //   code 0, and both times the modification time changed in between). Killing right after the answer left `.claude.json.lock`·
    //   `.claude.json.tmp.<pid>.*` in the settings folder (same day · logged-out query). The grace period covers that 1 second — past it, it is cut off.
    finish_after_answer(child, env.deadline);
    observation_from_response(&response)
}

/// Writes the two settings files to `scratch` and builds the query command that launches from that folder.
///
/// Why the two settings are passed as file paths rather than inline JSON = on Windows it goes through one layer of `cmd.exe /c`, and its quoting
/// rules silently break chunks of quotes and braces (the same situation as the `--settings` comment of the agent spawn).
fn probe_command(
    scratch: &Path,
    daemon_env_keys: impl IntoIterator<Item = OsString>,
) -> Result<ProbeCommand, ProbeError> {
    let settings = scratch.join(SETTINGS_FILE);
    let mcp_config = scratch.join(MCP_CONFIG_FILE);
    write_config(&settings, SETTINGS_JSON)?;
    write_config(&mcp_config, MCP_CONFIG_JSON)?;

    let mut args: Vec<String> = STREAM_JSON_CORE.iter().map(|a| (*a).to_owned()).collect();
    args.push("--no-session-persistence".to_owned());
    args.push("--settings".to_owned());
    args.push(path_arg(&settings)?);
    args.push("--strict-mcp-config".to_owned());
    // ★`--mcp-config` goes last★ — that flag takes multiple values (2.1.280 `--help`: `<configs...>`) and absorbs any bare
    //   value that follows as a config too.
    args.push("--mcp-config".to_owned());
    args.push(path_arg(&mcp_config)?);
    // `--permission-mode bypassPermissions` is deliberately absent — there is no model turn, so there is nothing to ask permission for.
    let (program, args) = console_command(CLAUDE_PROGRAM, args);

    Ok(ProbeCommand {
        program,
        args,
        cwd: scratch.to_path_buf(),
        env_set: PROBE_ENV
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
        env_remove: stripped_env_keys(daemon_env_keys),
    })
}

/// Keys to strip from the daemon env — keys starting with `CLAUDE` (ASCII case-insensitive) other than [`KEPT_CLAUDE_ENV`].
/// ★Non-UTF-8 keys cannot be stripped★ — [`ProbeCommand::env_remove`] is a string, so it cannot point at such a key.
fn stripped_env_keys(daemon_env_keys: impl IntoIterator<Item = OsString>) -> Vec<String> {
    daemon_env_keys
        .into_iter()
        .filter_map(|key| key.into_string().ok())
        .filter(|key| {
            key.get(..STRIPPED_ENV_PREFIX.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(STRIPPED_ENV_PREFIX))
        })
        .filter(|key| {
            !KEPT_CLAUDE_ENV
                .iter()
                .any(|kept| key.eq_ignore_ascii_case(kept))
        })
        .collect()
}

fn write_config(path: &Path, content: &str) -> Result<(), ProbeError> {
    fs::write(path, content)
        .map_err(|e| ProbeError::Io(format!("could not write the query settings file: {e}")))
}

/// Refuses non-Unicode paths — a lossily converted path points at a different (nonexistent) file.
fn path_arg(path: &Path) -> Result<String, ProbeError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| ProbeError::Io("the query scratch folder path is not Unicode".to_owned()))
}

/// `skip_behaviors` = for callers that need only the plan limits — skips scanning 7 days of usage history (SDK type comment).
fn request_line(request_id: &str) -> String {
    json!({
        "type": "control_request",
        "request_id": request_id,
        "request": { "subtype": "get_usage", "skip_behaviors": true },
    })
    .to_string()
}

/// Reads until the response line of our request arrives — other lines (init notices · responses to other requests · non-JSON lines) are discarded.
/// `write_error` = the I/O error from a failed request write — returned if it ends without a response and is not 「not installed」 either.
fn await_response(
    child: &mut dyn ProbeChild,
    request_id: &str,
    deadline: Instant,
    write_error: Option<ProbeError>,
) -> Result<Value, ProbeError> {
    let mut read_error = write_error;
    loop {
        let line = match child.read_line(deadline) {
            Ok(Some(line)) => line,
            Ok(None) => return Err(ended_without_response(child, deadline, read_error)),
            // Either a single line that hit the cap or a read failure — in the former case the stream continues to the next line, in the latter EOF comes soon.
            Err(err @ ProbeError::Io(_)) => {
                read_error = Some(err);
                continue;
            }
            Err(err) => return Err(err),
        };
        match serde_json::from_str::<Value>(&line) {
            Ok(value) if is_response_to(&value, request_id) => return Ok(value),
            Ok(_) => {}
            // If it is our response but not JSON (non-finite number literals etc.), no other response will come by waiting — do not drag on until the time limit.
            // The request id is a uuid we drew, so it cannot appear in another line by chance.
            Err(_) if line.contains(request_id) && line.contains("control_response") => {
                return Err(ProbeError::Parse(
                    "the claude usage response line is not JSON".to_owned(),
                ));
            }
            Err(_) => {}
        }
    }
}

/// Response line = `{"type":"control_response","response":{"subtype":…,"request_id":…}}` (probe measurement). The request id is
/// inside `response` — it is accepted even if it comes at the top level of the envelope.
fn is_response_to(line: &Value, request_id: &str) -> bool {
    let id_matches = |holder: Option<&Value>| {
        holder
            .and_then(|h| h.get("request_id"))
            .and_then(Value::as_str)
            == Some(request_id)
    };
    line.get("type").and_then(Value::as_str) == Some("control_response")
        && (id_matches(line.get("response")) || id_matches(Some(line)))
}

/// stdout closed without a response — 「not installed」 is decided from the exit status.
fn ended_without_response(
    child: &mut dyn ProbeChild,
    deadline: Instant,
    read_error: Option<ProbeError>,
) -> ProbeError {
    let exit = match child.wait_exit(deadline) {
        Ok(exit) => exit,
        Err(err) => return err,
    };
    if exit.looks_not_installed() {
        return ProbeError::NotInstalled;
    }
    // stderr content is not carried — only token shapes are masked, and account and path may remain. Only the line count is noted.
    tracing::debug!(
        code = ?exit.code(),
        stderr_lines = child.stderr_tail().len(),
        "claude usage query: ended without a response"
    );
    if let Some(err) = read_error {
        return err;
    }
    ProbeError::Upstream(match exit.code() {
        Some(code) => format!("claude ended without a usage response (exit code {code})"),
        None => "claude ended without a usage response (no exit code)".to_owned(),
    })
}

/// The response line of our request → an observation.
///
/// - `response.subtype == "error"` → [`classify_error`] · any other unknown subtype → `Parse`.
/// - On success `response.response` is the usage: `rate_limits` · within it `five_hour`/`seven_day`
///   (`{utilization 0–100, resets_at ISO 8601}`) · per-model weekly windows · `subscription_type` → plan.
/// - The meaning of the two fields is set by the response schema description of CLI 2.1.280: `rate_limits_available` = "`false` if plan limits do not apply
///   (API key · Bedrock · Vertex · a token without profile permission)", `rate_limits` = "`null` if the CLI could not fetch it".
///   So `null` alone is 「query failure」, and only an explicit `false` is 「this account has no limits」.
/// - `rate_limits_available` is bool `false` → 「no limit info」 (`limits_unavailable` — grounds =
///   [`limits_unavailable_detail`]). All windows are `None`, and `rate_limits` is not read whatever its shape (even if missing) —
///   plan is read as usual.
/// - Otherwise (`true` · no field · not a bool — a non-bool counts as absent), if `rate_limits` is an explicit `null`, it is
///   `Upstream` (= query failure). The receiver keeps the value it held. ★Even when the CLI-side query gets a 429, it comes not as an error subtype
///   but as this `null` (from reading the CLI code)★ — so it cannot be told apart as `RateLimited` and is folded into `Upstream`.
///   The limit-text table of [`classify_error`] is kept for when another version answers with an error subtype. ★It is a failure without upstream
///   error text, so the original = the two fields received as is★ ([`KIND_RATE_LIMITS_NULL`] — if the `rate_limits_available` field is missing, that field is left out).
/// - ★Otherwise, if the `rate_limits` field is missing or is neither an object nor `null`, it is `Parse`★ — reading that as 「no windows」 would let a response whose
///   shape changed come in as a success and erase all the values held. On failure the receiver keeps the value. The grounds are only the category.
/// - A `None` field = not carried (the receiver keeps the value it held). A window where neither field can be read is also folded into `None`.
///   ★Per-model windows are `Some` even when empty, as long as the `rate_limits` object was read★ — folding into `None` would leave a vanished model window on the receiving
///   side forever.
fn observation_from_response(line: &Value) -> Result<UsageObservation, ProbeFailure> {
    let response = line
        .get("response")
        .and_then(Value::as_object)
        .ok_or_else(|| parse_error("the response envelope has no response object"))?;
    match response.get("subtype").and_then(Value::as_str) {
        Some("success") => {}
        Some("error") => return Err(classify_error(response.get("error"))),
        _ => return Err(parse_error("unknown subtype in the response").into()),
    }
    let usage = response
        .get("response")
        .and_then(Value::as_object)
        .ok_or_else(|| parse_error("the success response has no usage object"))?;
    let flag = usage.get("rate_limits_available");
    let limits_unavailable =
        (flag.and_then(Value::as_bool) == Some(false)).then(|| limits_unavailable_detail(usage));
    let (five_hour, weekly, model_scoped) = if limits_unavailable.is_some() {
        // Even if `false` comes with values in `rate_limits`, they are not read — an observation of 「no info」 carrying windows would be a contradiction.
        (None, None, None)
    } else {
        match usage.get("rate_limits") {
            Some(Value::Null) => {
                let received = match flag {
                    Some(flag) => format!("rate_limits_available: {flag}, rate_limits: null"),
                    None => "rate_limits: null".to_owned(),
                };
                return Err(ProbeFailure::with_detail(
                    ProbeError::Upstream(
                        "claude could not fetch usage (rate_limits is null)".to_owned(),
                    ),
                    UsageDetail {
                        kind: KIND_RATE_LIMITS_NULL,
                        code: None,
                        upstream: Some(UpstreamText::new(&received)),
                    },
                ));
            }
            Some(Value::Object(limits)) => (
                active_window(limits.get("five_hour")),
                active_window(limits.get("seven_day")),
                Some(model_scoped_windows(limits)),
            ),
            Some(_) => return Err(parse_error("rate_limits is neither an object nor null").into()),
            None => return Err(parse_error("the usage object has no rate_limits field").into()),
        }
    };
    let plan = usage
        .get("subscription_type")
        .and_then(Value::as_str)
        .and_then(|plan| display_text(plan, PLAN_MAX_CHARS));
    Ok(UsageObservation {
        vendor: USAGE_VENDOR,
        five_hour,
        weekly,
        model_scoped,
        plan,
        source: UsageSource::Active,
        limits_unavailable,
    })
}

fn parse_error(what: &str) -> ProbeError {
    ProbeError::Parse(format!("claude usage response: {what}"))
}

/// Grounds of 「no limit info」 — original = the decided field and `subscription_type` as received (including `null` · a missing field is left out).
/// The plan field is carried along as a clue to the cause (a logged-out CLI answers with `null` — measured).
fn limits_unavailable_detail(usage: &Map<String, Value>) -> UsageDetail {
    let received = match usage.get("subscription_type") {
        Some(plan) => format!("rate_limits_available: false, subscription_type: {plan}"),
        None => "rate_limits_available: false".to_owned(),
    };
    UsageDetail {
        kind: KIND_LIMITS_UNAVAILABLE,
        code: None,
        upstream: Some(UpstreamText::new(&received)),
    }
}

/// One window object — `utilization` is already 0–100, `resets_at` is an ISO 8601 (RFC 3339) string. A wrong field drops only that field.
fn active_window(value: Option<&Value>) -> Option<WindowObs> {
    let fields = value?.as_object()?;
    let window = WindowObs {
        used_pct: fields
            .get("utilization")
            .and_then(Value::as_f64)
            .and_then(used_pct_from_percent),
        resets_at: fields
            .get("resets_at")
            .and_then(Value::as_str)
            .and_then(epoch_secs_from_iso),
    };
    (window.used_pct.is_some() || window.resets_at.is_some()).then_some(window)
}

/// Times without a time zone are not accepted — converting a value whose time zone is unknown to epoch would draw a wrong reset.
fn epoch_secs_from_iso(text: &str) -> Option<i64> {
    let at = DateTime::parse_from_rfc3339(text).ok()?;
    resets_at_from_epoch_secs(at.timestamp())
}

/// Per-model weekly windows — after `model_scoped[]` (`{display_name, utilization, resets_at}`), the named fields that have values
/// ([`NAMED_MODEL_WINDOWS`]). For the same name (ASCII case-insensitive) only the first one stays.
/// ★The `model_scoped` entry shape is not in the SDK type★ — it is a field the CLI carries ahead of the type, so only entries whose name is a string are accepted.
fn model_scoped_windows(limits: &Map<String, Value>) -> Vec<ScopedWindowObs> {
    let listed = limits
        .get("model_scoped")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let label = entry
                .get("display_name")
                .and_then(Value::as_str)
                .and_then(|name| display_text(name, LABEL_MAX_CHARS))?;
            Some((label, active_window(Some(entry))?))
        });
    let named = NAMED_MODEL_WINDOWS
        .iter()
        .filter_map(|(key, label)| Some(((*label).to_owned(), active_window(limits.get(*key))?)));

    let mut windows: Vec<ScopedWindowObs> = Vec::new();
    for (label, window) in listed.chain(named) {
        if windows.len() >= MODEL_SCOPED_MAX {
            break;
        }
        if windows.iter().any(|w| w.label.eq_ignore_ascii_case(&label)) {
            continue;
        }
        windows.push(ScopedWindowObs { label, window });
    }
    windows
}

/// Error response → category + grounds ([`KIND_CLAUDE_ERROR`] · original = the `error` field value). If `error` is not a string, its JSON notation is taken as is
/// as the text — for both the category and the original. If `error` is missing, `null` or only whitespace, there is no original either.
fn classify_error(error: Option<&Value>) -> ProbeFailure {
    let text = match error {
        Some(Value::String(text)) => Some(text.clone()),
        None | Some(Value::Null) => None,
        Some(other) => Some(other.to_string()),
    };
    let classified = classify_error_text(&text.as_deref().unwrap_or_default().to_ascii_lowercase());
    tracing::debug!(kind = %classified, "claude usage query: error response");
    ProbeFailure::with_detail(
        classified,
        UsageDetail {
            kind: KIND_CLAUDE_ERROR,
            code: None,
            upstream: text.as_deref().and_then(UpstreamText::non_blank),
        },
    )
}

/// ★This text table is a guess★ — the real texts of limit and logout errors have not been seen yet (TRD §6 #4). Even if it is wrong, where it lands is
///   `Upstream` (= query failure). ★Not reading unknown errors as limits is intended★ — showing 「refused」 would mislead the user
///   and block refresh. The `Upstream` text is fixed — the upstream text is not repeated.
/// `text` = the error text folded to lowercase.
fn classify_error_text(text: &str) -> ProbeError {
    const UNSUPPORTED: &str = "not supported in this context";
    const RATE_LIMITED: [&str; 5] = [
        "rate limit",
        "rate_limit",
        "rate-limit",
        "ratelimit",
        "too many requests",
    ];
    // ★Do not bring back a bare `oauth`★ — a 5xx carrying the query endpoint path (`/api/oauth/usage`) would read as a login refusal.
    const UNAUTHENTICATED: [&str; 10] = [
        "not logged in",
        "logged out",
        "log in",
        "login",
        "unauthorized",
        "unauthenticated",
        "authenticat",
        "oauth token",
        "oauth_token",
        "invalid api key",
    ];
    if text.contains(UNSUPPORTED) {
        return ProbeError::Unsupported;
    }
    if RATE_LIMITED.iter().any(|w| has_word(text, w, false)) || has_word(text, "429", true) {
        return ProbeError::RateLimited { retry_after: None };
    }
    if UNAUTHENTICATED.iter().any(|w| has_word(text, w, false)) || has_word(text, "401", true) {
        return ProbeError::Unauthenticated;
    }
    ProbeError::Upstream("claude refused the usage query with an unclassified error".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use crate::backend::claude::ClaudeBackend;
    use crate::backend::AgentBackend;
    use crate::profile::{AgentCommand, AgentOutputFormat, SpawnMode};
    use crate::usage::testing::{ChildScript, ScriptedSpawner, TempRoot};
    use crate::usage::{ExitInfo, ProbeSpawner};

    const SHORT: Duration = Duration::from_millis(300);
    const ROOMY: Duration = Duration::from_secs(4);

    /// 2026-09-27T05:00:00Z · 2026-10-01T09:00:00Z.
    const FIVE_HOUR_RESET: i64 = 1_790_485_200;
    const WEEKLY_RESET: i64 = 1_790_845_200;

    /// Keys of the stand-in daemon env — a mix of ones to strip, ones to keep and unrelated ones.
    const DAEMON_KEYS: [&str; 10] = [
        "CLAUDECODE",
        "CLAUDE_CODE_ENTRYPOINT",
        "claude_lower_case",
        "CLAUDE_CONFIG_DIR",
        "Claude_Code_OAuth_Token",
        "CLAUDE_CODE_AUTO_CONNECT_IDE",
        "PATH",
        "ANTHROPIC_API_KEY",
        "XCLAUDE",
        "CLAUD",
    ];

    fn daemon_keys() -> Vec<OsString> {
        DAEMON_KEYS.iter().map(OsString::from).collect()
    }

    fn run_with_deadline(
        spawner: &ScriptedSpawner,
        root: &TempRoot,
        within: Duration,
    ) -> Result<UsageObservation, ProbeFailure> {
        let env = ProbeEnv {
            spawner,
            deadline: Instant::now() + within,
            scratch_root: root.path(),
        };
        query_with(&env, daemon_keys())
    }

    fn run(script: ChildScript) -> (Result<UsageObservation, ProbeFailure>, ScriptedSpawner) {
        let root = TempRoot::new("claude-probe");
        let spawner = ScriptedSpawner::new(script);
        let result = run_with_deadline(&spawner, &root, ROOMY);
        assert_scratch_gone(&root);
        (result, spawner)
    }

    fn assert_scratch_gone(root: &TempRoot) {
        let left: Vec<_> = fs::read_dir(root.path())
            .expect("test root")
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert!(
            left.is_empty(),
            "the scratch folder remained after the query: {left:?}"
        );
    }

    fn request_id_of(written: &[String]) -> String {
        let first = written
            .first()
            .expect("the prober writes the request line first");
        let line: Value = serde_json::from_str(first).expect("the request line is JSON");
        line["request_id"]
            .as_str()
            .expect("the request id is a string")
            .to_owned()
    }

    fn success_line(written: &[String], usage: Value) -> String {
        json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": request_id_of(written), "response": usage },
        })
        .to_string()
    }

    fn error_line(written: &[String], error: Value) -> String {
        json!({
            "type": "control_response",
            "response": { "subtype": "error", "request_id": request_id_of(written), "error": error },
        })
        .to_string()
    }

    fn answering(usage: Value) -> ChildScript {
        ChildScript::new().reply(move |written| success_line(written, usage))
    }

    /// The shape the probe saw (figures are fake) — two windows · plan · a model window with only a key and a null value · a field not in the type.
    fn typical_usage() -> Value {
        json!({
            "subscription_type": "max",
            "rate_limits_available": true,
            "rate_limits": {
                "five_hour": { "utilization": 42.5, "resets_at": "2026-09-27T05:00:00.123456+00:00" },
                "seven_day": { "utilization": 13, "resets_at": "2026-10-01T09:00:00Z" },
                "seven_day_opus": null,
                "seven_day_sonnet": null,
                "seven_day_oauth_apps": null,
                "model_scoped": [],
                "extra_usage": { "is_enabled": false, "monthly_limit": null },
            },
            "session": { "cost": 0 },
        })
    }

    fn window(used_pct: Option<f64>, resets_at: Option<i64>) -> Option<WindowObs> {
        Some(WindowObs {
            used_pct,
            resets_at,
        })
    }

    fn observe(usage: Value) -> Result<UsageObservation, ProbeFailure> {
        observation_from_response(&json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": "r", "response": usage },
        }))
    }

    fn rate_limits(limits: Value) -> Result<UsageObservation, ProbeFailure> {
        observe(json!({ "rate_limits": limits }))
    }

    /// Whether it is a `Parse` whose only grounds are the category.
    fn is_bare_parse(result: &Result<UsageObservation, ProbeFailure>) -> bool {
        matches!(
            result,
            Err(ProbeFailure {
                error: ProbeError::Parse(_),
                detail: None,
            })
        )
    }

    fn upstream_of(detail: &UsageDetail) -> Option<&str> {
        detail.upstream.as_ref().map(UpstreamText::as_str)
    }

    // ── Policy ──

    #[test]
    fn policy_and_key_are_the_claude_values() {
        assert_eq!(CLAUDE_USAGE_PROBE.key(), USAGE_VENDOR);
        assert_eq!(CLAUDE_USAGE_PROBE.key().as_str(), "claude");
        assert_eq!(
            CLAUDE_USAGE_PROBE.policy(),
            UsagePolicy {
                cooldown: Duration::from_secs(900),
                timeout: Duration::from_secs(15),
            }
        );
    }

    // ── Interpretation ──

    #[test]
    fn success_carries_both_windows_and_the_plan() {
        let (result, spawner) = run(answering(typical_usage()));
        let obs = result.expect("query");
        assert_eq!(obs.vendor, USAGE_VENDOR);
        assert_eq!(obs.source, UsageSource::Active);
        assert_eq!(obs.five_hour, window(Some(42.5), Some(FIVE_HOUR_RESET)));
        assert_eq!(obs.weekly, window(Some(13.0), Some(WEEKLY_RESET)));
        assert_eq!(
            obs.model_scoped,
            Some(vec![]),
            "null model window · empty list = the per-model windows are empty (not absent)"
        );
        assert_eq!(obs.plan.as_deref(), Some("max"));
        assert!(
            spawner.stdin_closed(),
            "did not close stdin after receiving the response"
        );
        assert!(
            spawner.child_dropped(),
            "did not kill the child after receiving the response"
        );
        assert!(!spawner.cut_at_deadline());
    }

    /// A child that does not end even when stdin is closed after the response — cut off after the grace period, but the query succeeds.
    #[test]
    fn a_child_that_lingers_after_the_answer_is_cut_but_the_query_succeeds() {
        let (result, spawner) = run(answering(typical_usage()).hang_after_lines());
        assert_eq!(result.expect("query").plan.as_deref(), Some("max"));
        assert!(spawner.stdin_closed());
        assert!(
            spawner.cut_at_deadline(),
            "should have been cut off after the grace period"
        );
        assert!(spawner.child_dropped());
    }

    /// If the deadline has already passed, neither the scratch folder nor the child is created.
    #[test]
    fn a_passed_deadline_times_out_before_anything_is_created() {
        let root = TempRoot::new("claude-probe-late");
        let spawner = ScriptedSpawner::new(answering(typical_usage()));
        assert_eq!(
            run_with_deadline(&spawner, &root, Duration::ZERO),
            Err(ProbeError::Timeout.into())
        );
        assert!(spawner.commands().is_empty(), "attempted to start");
        assert_scratch_gone(&root);
    }

    fn assert_no_windows(obs: &UsageObservation) {
        assert_eq!(obs.five_hour, None);
        assert_eq!(obs.weekly, None);
        assert_eq!(obs.model_scoped, None);
        assert_eq!(obs.source, UsageSource::Active);
    }

    /// `rate_limits: null` failure — grounds = [`KIND_RATE_LIMITS_NULL`] + the two fields as received (`upstream`).
    fn assert_fetch_failed(
        result: Result<UsageObservation, ProbeFailure>,
        upstream: &str,
        why: &str,
    ) {
        match result {
            Err(ProbeFailure {
                error: ProbeError::Upstream(_),
                detail: Some(detail),
            }) => {
                assert_eq!(detail.kind, KIND_RATE_LIMITS_NULL, "{why}");
                assert_eq!(detail.code, None, "{why}");
                assert_eq!(upstream_of(&detail), Some(upstream), "{why}");
            }
            other => panic!("{why}: {other:?}"),
        }
    }

    /// 「no limit info」 — grounds = [`KIND_LIMITS_UNAVAILABLE`] + the decided field and plan field as received (`upstream`).
    fn assert_unavailable(obs: &UsageObservation, upstream: &str) {
        let detail = obs.limits_unavailable.as_ref().expect("no limit info");
        assert_eq!(detail.kind, KIND_LIMITS_UNAVAILABLE);
        assert_eq!(detail.code, None);
        assert_eq!(upstream_of(detail), Some(upstream));
        assert_no_windows(obs);
    }

    /// The shape a logged-out CLI answers with (measured).
    #[test]
    fn logged_out_shape_is_limits_unavailable_without_a_plan() {
        let obs = observe(json!({
            "subscription_type": null,
            "rate_limits_available": false,
            "rate_limits": null,
        }))
        .expect("observation");
        assert_unavailable(
            &obs,
            "rate_limits_available: false, subscription_type: null",
        );
        assert_eq!(obs.plan, None);
    }

    /// A token without profile permission — the plan is known but limits do not apply.
    #[test]
    fn unavailable_with_a_plan_still_reads_the_plan() {
        let obs = observe(json!({
            "subscription_type": "max",
            "rate_limits_available": false,
            "rate_limits": null,
        }))
        .expect("observation");
        assert_unavailable(
            &obs,
            r#"rate_limits_available: false, subscription_type: "max""#,
        );
        assert_eq!(obs.plan.as_deref(), Some("max"));
    }

    /// The original field does not exceed the cap either — however long the received plan is.
    #[test]
    fn a_huge_plan_is_cut_in_the_unavailable_detail() {
        let obs = observe(json!({
            "subscription_type": "가".repeat(10_000), // hangul-ok
            "rate_limits_available": false,
        }))
        .expect("observation");
        let detail = obs.limits_unavailable.expect("no limit info");
        let upstream = upstream_of(&detail).expect("original");
        assert_eq!(upstream.chars().count(), 200);
        // hangul-ok-begin
        assert!(upstream.starts_with("rate_limits_available: false, subscription_type: \"가"));
        // hangul-ok-end
    }

    #[test]
    fn unavailable_ignores_windows_carried_in_rate_limits() {
        let mut usage = typical_usage();
        usage["rate_limits_available"] = json!(false);
        let obs = observe(usage).expect("observation");
        assert_unavailable(
            &obs,
            r#"rate_limits_available: false, subscription_type: "max""#,
        );
        assert_eq!(obs.plan.as_deref(), Some("max"));
    }

    #[test]
    fn unavailable_does_not_read_rate_limits_of_any_shape() {
        let without_limits = json!({ "subscription_type": "max", "rate_limits_available": false });
        let obs = observe(without_limits).expect("observation");
        assert_unavailable(
            &obs,
            r#"rate_limits_available: false, subscription_type: "max""#,
        );

        for limits in [json!("none"), json!([]), json!(0)] {
            let obs = observe(json!({ "rate_limits_available": false, "rate_limits": limits }))
                .expect("observation");
            assert_unavailable(&obs, "rate_limits_available: false");
        }
    }

    #[test]
    fn null_rate_limits_without_a_false_flag_is_a_fetch_failure() {
        assert_fetch_failed(
            observe(json!({ "rate_limits_available": true, "rate_limits": null })),
            "rate_limits_available: true, rate_limits: null",
            "true",
        );
        assert_fetch_failed(
            observe(json!({ "subscription_type": "max", "rate_limits": null })),
            "rate_limits: null",
            "no field",
        );
    }

    /// Only the grounds word goes into the log text of a failure (`Display`) — the two received fields are only in the original field.
    #[test]
    fn a_fetch_failure_shows_only_the_kind_in_its_display() {
        let failure = observe(json!({ "rate_limits_available": true, "rate_limits": null }))
            .expect_err("query failure");
        let shown = failure.to_string();
        assert!(shown.contains(KIND_RATE_LIMITS_NULL), "{shown}");
        assert!(!shown.contains("rate_limits_available"), "{shown}");
    }

    #[test]
    fn available_with_windows_is_not_unavailable() {
        let obs = observe(typical_usage()).expect("observation");
        assert_eq!(obs.limits_unavailable, None);
        assert_eq!(obs.five_hour, window(Some(42.5), Some(FIVE_HOUR_RESET)));
        assert_eq!(obs.weekly, window(Some(13.0), Some(WEEKLY_RESET)));
    }

    /// A non-bool `rate_limits_available` is the same as no field — `rate_limits` decides.
    #[test]
    fn a_non_bool_flag_is_read_as_missing() {
        for flag in [
            json!("false"),
            json!(0),
            json!(1),
            json!({}),
            json!([false]),
        ] {
            let mut usage = typical_usage();
            usage["rate_limits_available"] = flag.clone();
            let obs = observe(usage).expect("observation");
            assert_eq!(obs.limits_unavailable, None, "{flag}");
            assert_eq!(
                obs.five_hour,
                window(Some(42.5), Some(FIVE_HOUR_RESET)),
                "{flag}"
            );

            assert_fetch_failed(
                observe(json!({ "rate_limits_available": flag.clone(), "rate_limits": null })),
                &format!("rate_limits_available: {flag}, rate_limits: null"),
                &flag.to_string(),
            );

            for missing_or_bad in [
                json!({ "rate_limits_available": flag.clone() }),
                json!({ "rate_limits_available": flag.clone(), "rate_limits": "none" }),
            ] {
                assert!(is_bare_parse(&observe(missing_or_bad)), "{flag}");
            }
        }
    }

    #[test]
    fn model_scoped_list_then_named_windows_without_duplicates() {
        let obs = rate_limits(json!({
            "five_hour": { "utilization": 1, "resets_at": null },
            "seven_day_opus": { "utilization": 3, "resets_at": null },
            "seven_day_sonnet": { "utilization": 55.25, "resets_at": "2026-10-01T09:00:00Z" },
            "seven_day_oauth_apps": null,
            "model_scoped": [
                { "display_name": "Fable", "utilization": 73, "resets_at": "2026-10-01T09:00:00Z" },
                { "display_name": "Ghost", "utilization": null, "resets_at": null },
                { "display_name": "  opus  ", "utilization": 9, "resets_at": null },
                { "utilization": 50 },
                { "display_name": 7, "utilization": 50 },
                "not an entry",
            ],
        }))
        .expect("observation");
        let scoped = obs.model_scoped.expect("per-model windows");
        let labels: Vec<&str> = scoped.iter().map(|w| w.label.as_str()).collect();
        assert_eq!(
            labels,
            ["Fable", "opus", "Sonnet"],
            "a duplicate Opus: the list side wins"
        );
        assert_eq!(
            scoped[0].window,
            WindowObs {
                used_pct: Some(73.0),
                resets_at: Some(WEEKLY_RESET)
            }
        );
        assert_eq!(
            scoped[1].window,
            WindowObs {
                used_pct: Some(9.0),
                resets_at: None
            }
        );
        assert_eq!(
            scoped[2].window,
            WindowObs {
                used_pct: Some(55.25),
                resets_at: Some(WEEKLY_RESET)
            }
        );
        assert_eq!(obs.five_hour, window(Some(1.0), None));
        assert_eq!(obs.weekly, None, "a missing window = none");
    }

    #[test]
    fn model_scoped_list_is_capped() {
        let entries: Vec<Value> = (0..40)
            .map(|i| json!({ "display_name": format!("model-{i}"), "utilization": i }))
            .collect();
        let obs = rate_limits(json!({ "model_scoped": entries })).expect("observation");
        assert_eq!(
            obs.model_scoped.expect("per-model windows").len(),
            MODEL_SCOPED_MAX
        );
    }

    #[test]
    fn iso_reset_times_parse_with_offsets_and_bad_ones_drop_only_that_field() {
        let reset = |text: Value| {
            rate_limits(json!({ "five_hour": { "utilization": 20, "resets_at": text } }))
                .expect("observation")
                .five_hour
                .expect("utilization remains, so the window exists")
                .resets_at
        };
        assert_eq!(reset(json!("2026-09-27T05:00:00Z")), Some(FIVE_HOUR_RESET));
        assert_eq!(
            reset(json!("2026-09-27T14:00:00+09:00")),
            Some(FIVE_HOUR_RESET)
        );
        assert_eq!(
            reset(json!("2026-09-27T05:00:00.999999999Z")),
            Some(FIVE_HOUR_RESET)
        );
        assert_eq!(reset(json!("9999-12-31T23:59:59Z")), Some(253_402_300_799));
        for bad in [
            json!("2026-09-27T05:00:00"),
            json!("2026-09-27"),
            json!("2026-13-45T00:00:00Z"),
            json!("next tuesday"),
            json!(""),
            json!("1969-12-31T23:59:59Z"),
            json!("1970-01-01T00:00:00Z"),
            json!(1_790_485_200),
            json!(null),
            json!(["2026-09-27T05:00:00Z"]),
        ] {
            assert_eq!(reset(bad.clone()), None, "{bad}");
        }
    }

    #[test]
    fn broken_envelopes_are_parse_errors() {
        let envelope = |response: Value| {
            observation_from_response(&json!({ "type": "control_response", "response": response }))
        };
        let cases = [
            observation_from_response(&json!({ "type": "control_response", "request_id": "r" })),
            observation_from_response(
                &json!({ "type": "control_response", "response": "success" }),
            ),
            envelope(json!({ "request_id": "r" })),
            envelope(json!({ "subtype": "partial", "request_id": "r" })),
            envelope(json!({ "subtype": 1, "request_id": "r" })),
            envelope(json!({ "subtype": "success", "request_id": "r" })),
            envelope(json!({ "subtype": "success", "request_id": "r", "response": [1] })),
            observe(json!({ "rate_limits_available": true })),
            observe(json!({ "subscription_type": "max" })),
            observe(json!({ "rate_limits": "none" })),
            observe(json!({ "rate_limits": [] })),
            observe(json!({ "rate_limits": 0 })),
        ];
        for (i, case) in cases.into_iter().enumerate() {
            assert!(is_bare_parse(&case), "{i}: {case:?}");
        }
    }

    /// However one field is wrong, it does not panic and drops only that field.
    #[test]
    fn hostile_fields_never_panic_and_drop_only_that_field() {
        let window_of = |w: Value| {
            rate_limits(json!({ "five_hour": w }))
                .expect("observation")
                .five_hour
        };
        assert_eq!(
            window_of(json!({ "utilization": -5, "resets_at": null })),
            window(Some(0.0), None)
        );
        assert_eq!(
            window_of(json!({ "utilization": 130 })),
            window(Some(100.0), None)
        );
        assert_eq!(
            window_of(json!({ "utilization": 1e308 })),
            window(Some(100.0), None)
        );
        assert_eq!(
            window_of(json!({ "utilization": -1e308 })),
            window(Some(0.0), None)
        );
        assert_eq!(
            window_of(json!({ "utilization": 18_446_744_073_709_551_615_u64 })),
            window(Some(100.0), None)
        );
        assert_eq!(
            window_of(json!({ "utilization": 0.55 })),
            window(Some(0.55), None),
            "already 0–100"
        );
        assert_eq!(
            window_of(json!({ "utilization": "42", "resets_at": 5 })),
            None
        );
        assert_eq!(
            window_of(json!({ "utilization": null, "resets_at": null })),
            None
        );
        assert_eq!(window_of(json!({})), None);
        assert_eq!(window_of(json!([42])), None);
        assert_eq!(window_of(json!("42%")), None);
        assert_eq!(window_of(json!(null)), None);

        for limits in [
            json!({ "model_scoped": { "display_name": "Fable", "utilization": 1 } }),
            json!({ "model_scoped": "Fable" }),
            json!({ "model_scoped": [null, 1, [], {}] }),
            json!({ "model_scoped": [{ "display_name": "", "utilization": 1 }] }),
            json!({ "model_scoped": [{ "display_name": "   ", "utilization": 1 }] }),
            json!({ "model_scoped": [{ "display_name": "bad\u{7}name", "utilization": 1 }] }),
            json!({ "seven_day_opus": "3%" , "seven_day_sonnet": [], "seven_day_oauth_apps": 0 }),
        ] {
            let obs = rate_limits(limits.clone()).expect("observation");
            assert_eq!(obs.model_scoped, Some(vec![]), "{limits}");
        }

        let long = "x".repeat(10_000);
        let obs = observe(json!({
            "subscription_type": long,
            "rate_limits": { "model_scoped": [{ "display_name": "가".repeat(500), "utilization": 1 }] }, // hangul-ok
        }))
        .expect("observation");
        assert_eq!(obs.plan.map(|p| p.chars().count()), Some(PLAN_MAX_CHARS));
        let scoped = obs.model_scoped.expect("per-model windows");
        assert_eq!(
            scoped[0].label.chars().count(),
            LABEL_MAX_CHARS,
            "cut by character"
        );

        for plan in [
            json!(5),
            json!(""),
            json!(null),
            json!("ma\nx"),
            json!({ "tier": "max" }),
        ] {
            let obs = observe(json!({ "subscription_type": plan, "rate_limits_available": false }))
                .expect("observation");
            assert_eq!(obs.plan, None, "{plan}");
        }
        let obs =
            observe(json!({ "subscription_type": " team\n", "rate_limits_available": false }))
                .expect("observation");
        assert_eq!(obs.plan.as_deref(), Some("team"));
    }

    const NON_JSON_RESPONSE: &str = r#"{"type":"control_response","response":{"subtype":"success","request_id":"REQUEST_ID","response":{"rate_limits":{"five_hour":{"utilization":NUMBER}}}}}"#;

    /// Our response carrying a non-JSON number (non-finite literal · out-of-range exponent) is `Parse` without waiting until the time limit.
    #[test]
    fn a_non_json_response_line_is_a_parse_error_without_waiting() {
        for number in ["NaN", "Infinity", "1e400"] {
            let script = ChildScript::new()
                .reply(move |written| {
                    NON_JSON_RESPONSE
                        .replace("REQUEST_ID", &request_id_of(written))
                        .replace("NUMBER", number)
                })
                .hang_after_lines();
            let root = TempRoot::new("claude-probe-nan");
            let spawner = ScriptedSpawner::new(script);
            let result = run_with_deadline(&spawner, &root, SHORT);
            assert!(is_bare_parse(&result), "{number}: {result:?}");
            assert!(
                !spawner.cut_at_deadline(),
                "{number}: waited until the time limit"
            );
            assert_scratch_gone(&root);
        }
    }

    // ── Error classification ──

    fn kind(err: &ProbeError) -> &'static str {
        match err {
            ProbeError::RateLimited { retry_after: None } => "rate",
            ProbeError::Unauthenticated => "auth",
            ProbeError::Unsupported => "unsupported",
            ProbeError::Upstream(_) => "upstream",
            other => panic!("outside the categories: {other:?}"),
        }
    }

    #[test]
    fn error_text_classification_table() {
        let cases = [
            ("get_usage is not supported in this context", "unsupported"),
            ("rate limit exceeded", "rate"),
            ("rate_limit_error", "rate"),
            ("request was rate-limited", "rate"),
            ("too many requests", "rate"),
            ("api error: 429 {\"type\":\"error\"}", "rate"),
            ("status=429", "rate"),
            ("not logged in · please run /login", "auth"),
            ("oauth token has expired", "auth"),
            ("invalid claude_code_oauth_token", "auth"),
            ("authentication_error", "auth"),
            ("invalid api key", "auth"),
            ("api error: 401", "auth"),
            // Even when limit text comes with login guidance, it is a limit.
            ("rate limited — log in again later", "rate"),
            ("", "upstream"),
            ("unauthenticated", "auth"),
            ("request req_14290a failed", "upstream"),
            ("internal server error 4010", "upstream"),
            ("not supported", "upstream"),
            // The same fragment inside another word is not the text.
            ("moderate limits apply", "upstream"),
            ("catalog index failed", "upstream"),
            ("bloglogin", "upstream"),
            // The `oauth` of the query endpoint path is not login text.
            (
                "api error: 500 internal server error (get /api/oauth/usage)",
                "upstream",
            ),
            (
                "502 bad gateway from https://api.example.test/api/oauth/usage",
                "upstream",
            ),
            ("/api/oauth/token returned 503", "upstream"),
        ];
        for (text, expected) in cases {
            assert_eq!(kind(&classify_error_text(text)), expected, "{text:?}");
        }
    }

    /// ★Upstream text goes only into the original field★ — for all four categories the grounds = [`KIND_CLAUDE_ERROR`] + the `error` field value (token shapes masked), and
    ///   it does not show in the failure's `Display`·`Debug`.
    #[test]
    fn error_responses_carry_the_upstream_text_only_in_the_detail() {
        let secret = "someone@example.com org-1234";
        let token = "sk-ant-abcdefghijklmnopqrstuvwxyz0123";
        for (error, expected, upstream) in [
            (
                json!(format!("Rate limit reached for {secret} {token}")),
                "rate",
                Some(format!("Rate limit reached for {secret} ***")),
            ),
            (
                json!(format!("Not logged in as {secret}")),
                "auth",
                Some(format!("Not logged in as {secret}")),
            ),
            (
                json!("get_usage is not supported in this context"),
                "unsupported",
                Some("get_usage is not supported in this context".to_owned()),
            ),
            (
                json!(format!("boom {secret}")),
                "upstream",
                Some(format!("boom {secret}")),
            ),
            (
                json!({ "message": format!("OAuth token expired for {secret}") }),
                "auth",
                Some(format!(
                    r#"{{"message":"OAuth token expired for {secret}"}}"#
                )),
            ),
            (json!(null), "upstream", None),
            (json!(""), "upstream", None),
            (json!("   "), "upstream", None),
        ] {
            let (result, spawner) =
                run(ChildScript::new().reply(move |written| error_line(written, error)));
            let err = result.expect_err("error response");
            assert_eq!(kind(&err.error), expected);
            let detail = err.detail.as_ref().expect("grounds");
            assert_eq!(detail.kind, KIND_CLAUDE_ERROR);
            assert_eq!(detail.code, None);
            assert_eq!(upstream_of(detail), upstream.as_deref());
            let shown = format!("{err} {err:?}");
            assert!(
                !shown.contains("example.com")
                    && !shown.contains("org-1234")
                    && !shown.contains("sk-ant"),
                "{shown}"
            );
            assert!(shown.contains(KIND_CLAUDE_ERROR), "{shown}");
            assert!(
                spawner.stdin_closed(),
                "an error response is also an answer — the grace period is given"
            );
            assert!(spawner.child_dropped());
        }
    }

    // ── Conversation ──

    #[test]
    fn the_request_is_one_get_usage_control_line_with_a_fresh_uuid() {
        let (result, spawner) = run(answering(typical_usage()));
        result.expect("query");
        let written = spawner.written();
        assert_eq!(
            written.len(),
            1,
            "only one control request line is written (no user message): {written:?}"
        );
        let line: Value = serde_json::from_str(&written[0]).expect("JSON");
        let id = line["request_id"].as_str().expect("request id");
        assert_eq!(Uuid::parse_str(id).expect("uuid").get_version_num(), 4);
        assert_eq!(
            line,
            json!({
                "type": "control_request",
                "request_id": id,
                "request": { "subtype": "get_usage", "skip_behaviors": true },
            })
        );
    }

    #[test]
    fn other_lines_and_other_request_ids_are_ignored() {
        let script = ChildScript::new()
            .line(r#"{"type":"system","subtype":"init"}"#)
            .line("plain text that is not json")
            .line(r#"{"type":"control_response","response":{"subtype":"error","request_id":"someone-else","error":"rate limit"}}"#)
            .line(r#"{"type":"control_request","request_id":"x","request":{"subtype":"can_use_tool"}}"#)
            .line(r#"[1,2,3]"#)
            .reply(|written| {
                // A line carrying our id but not a control_response is also discarded.
                json!({ "type": "user", "request_id": request_id_of(written) }).to_string()
            })
            .reply(|written| success_line(written, typical_usage()));
        let (result, _) = run(script);
        assert_eq!(result.expect("query").plan.as_deref(), Some("max"));
    }

    #[test]
    fn a_top_level_request_id_is_accepted() {
        let script = ChildScript::new().reply(|written| {
            json!({
                "type": "control_response",
                "request_id": request_id_of(written),
                "response": { "subtype": "success", "response": typical_usage() },
            })
            .to_string()
        });
        let (result, _) = run(script);
        assert!(result.is_ok(), "{result:?}");
    }

    /// A child that first produces one read error — the shape of the real spawner's 「one line over the cap」 (the script stand-in cannot produce an error line).
    struct IoFirstSpawner {
        then_answer: bool,
    }

    struct IoFirstChild {
        request: String,
        io_sent: bool,
        then_answer: bool,
    }

    impl ProbeSpawner for IoFirstSpawner {
        fn spawn(
            &self,
            _cmd: &ProbeCommand,
            _deadline: Instant,
        ) -> Result<Box<dyn ProbeChild>, ProbeError> {
            Ok(Box::new(IoFirstChild {
                request: String::new(),
                io_sent: false,
                then_answer: self.then_answer,
            }))
        }
    }

    impl ProbeChild for IoFirstChild {
        fn write_line(&mut self, line: &str, _deadline: Instant) -> Result<(), ProbeError> {
            self.request = line.to_owned();
            Ok(())
        }

        fn close_stdin(&mut self) {}

        fn read_line(&mut self, _deadline: Instant) -> Result<Option<String>, ProbeError> {
            if !self.io_sent {
                self.io_sent = true;
                return Err(ProbeError::Io("one line exceeded the cap".to_owned()));
            }
            if std::mem::take(&mut self.then_answer) {
                let written = std::slice::from_ref(&self.request);
                return Ok(Some(success_line(written, typical_usage())));
            }
            Ok(None)
        }

        fn wait_exit(&mut self, _deadline: Instant) -> Result<ExitInfo, ProbeError> {
            Ok(ExitInfo::new(Some(0), false))
        }

        fn wait_exit_code(&mut self, _deadline: Instant) -> Result<Option<i32>, ProbeError> {
            Ok(Some(0))
        }

        fn stderr_tail(&self) -> Vec<String> {
            Vec::new()
        }
    }

    #[test]
    fn a_read_error_is_skipped_and_reported_only_if_no_response_follows() {
        for (then_answer, expect_ok) in [(true, true), (false, false)] {
            let root = TempRoot::new("claude-probe-io");
            let spawner = IoFirstSpawner { then_answer };
            let env = ProbeEnv {
                spawner: &spawner,
                deadline: Instant::now() + ROOMY,
                scratch_root: root.path(),
            };
            let result = query_with(&env, daemon_keys());
            if expect_ok {
                assert!(result.is_ok(), "{result:?}");
            } else {
                assert!(
                    matches!(
                        result,
                        Err(ProbeFailure {
                            error: ProbeError::Io(_),
                            detail: None
                        })
                    ),
                    "{result:?}"
                );
            }
            assert_scratch_gone(&root);
        }
    }

    #[test]
    fn a_silent_child_times_out_and_is_cut() {
        let root = TempRoot::new("claude-probe-silent");
        let spawner = ScriptedSpawner::new(
            ChildScript::new()
                .line(r#"{"type":"system","subtype":"init"}"#)
                .hang_after_lines(),
        );
        let started = Instant::now();
        let result = run_with_deadline(&spawner, &root, SHORT);
        assert_eq!(result, Err(ProbeError::Timeout.into()));
        assert!(started.elapsed() >= SHORT, "returned before the deadline");
        assert!(spawner.cut_at_deadline());
        assert!(spawner.child_dropped());
        assert_scratch_gone(&root);
    }

    #[test]
    fn a_child_that_never_reads_stdin_times_out() {
        let root = TempRoot::new("claude-probe-blocked");
        let spawner = ScriptedSpawner::new(ChildScript::new().block_writes());
        assert_eq!(
            run_with_deadline(&spawner, &root, SHORT),
            Err(ProbeError::Timeout.into())
        );
        assert!(spawner.cut_at_deadline());
        assert!(spawner.written().is_empty());
        assert_scratch_gone(&root);
    }

    #[test]
    fn eof_without_a_response_is_classified_by_the_exit() {
        let (result, spawner) = run(ChildScript::new().exit_not_installed().stderr_tail(&["x"]));
        assert_eq!(result, Err(ProbeError::NotInstalled.into()));
        assert!(spawner.child_dropped());

        let (result, _) = run(ChildScript::new()
            .line(r#"{"type":"system"}"#)
            .exit_code(1)
            .stderr_tail(&["Error: someone@example.com is not allowed"]));
        match result {
            Err(ProbeFailure {
                error: ProbeError::Upstream(text),
                detail: None,
            }) => {
                assert!(text.contains("exit code 1"), "{text}");
                assert!(!text.contains("example.com"), "carried stderr: {text}");
            }
            other => panic!("{other:?}"),
        }

        let root = TempRoot::new("claude-probe-linger");
        let spawner = ScriptedSpawner::new(ChildScript::new().never_exit());
        assert_eq!(
            run_with_deadline(&spawner, &root, SHORT),
            Err(ProbeError::Timeout.into())
        );
        assert!(spawner.cut_at_deadline());
        assert_scratch_gone(&root);
    }

    /// ★A failed request write does not mask 「not installed」★ — even if the wrapping shell has already ended and the write fails, it decides by the exit.
    ///   If it is a failure on the installed side, it is the write error as is.
    #[test]
    fn a_failed_request_write_still_reads_the_exit() {
        let (result, spawner) = run(ChildScript::new().fail_writes().exit_not_installed());
        assert_eq!(result, Err(ProbeError::NotInstalled.into()));
        assert!(spawner.written().is_empty());

        let (result, _) = run(ChildScript::new().fail_writes().exit_code(1));
        assert!(
            matches!(
                result,
                Err(ProbeFailure {
                    error: ProbeError::Io(_),
                    detail: None
                })
            ),
            "{result:?}"
        );
    }

    /// A failure of the spawner seam comes up without grounds (`?`) — the receiver fills in the category word.
    #[test]
    fn a_spawn_failure_passes_through_and_cleans_up() {
        let root = TempRoot::new("claude-probe-missing");
        let spawner = ScriptedSpawner::failing(ProbeError::NotInstalled);
        assert_eq!(
            run_with_deadline(&spawner, &root, ROOMY),
            Err(ProbeFailure {
                error: ProbeError::NotInstalled,
                detail: None,
            })
        );
        assert_eq!(spawner.commands().len(), 1);
        assert_scratch_gone(&root);
    }

    // ── Launch spec ──

    #[test]
    fn the_command_runs_in_its_own_scratch_dir_with_hooks_and_mcp_off() {
        let root = TempRoot::new("claude-probe-command");
        let seen_files: Arc<Mutex<Option<(PathBuf, String, String)>>> = Arc::default();
        let seen = seen_files.clone();
        let scan_root = root.path().to_path_buf();
        let spawner = ScriptedSpawner::new(ChildScript::new().reply(move |written| {
            // While the child is up — the settings files must be in place.
            let dir = fs::read_dir(&scan_root)
                .expect("root")
                .flatten()
                .map(|e| e.path())
                .next()
                .expect("scratch folder");
            let settings = fs::read_to_string(dir.join(SETTINGS_FILE)).expect("settings file");
            let mcp = fs::read_to_string(dir.join(MCP_CONFIG_FILE)).expect("MCP settings file");
            *seen.lock().expect("lock") = Some((dir, settings, mcp));
            success_line(written, typical_usage())
        }));
        run_with_deadline(&spawner, &root, ROOMY).expect("query");

        let commands = spawner.commands();
        let [cmd] = commands.as_slice() else {
            panic!("one child: {commands:?}")
        };
        let (dir, settings, mcp) = seen_files
            .lock()
            .expect("lock")
            .take()
            .expect("file during the conversation");
        assert_eq!(cmd.cwd, dir, "working folder = that query's scratch folder");
        assert!(cmd.cwd.starts_with(root.path()));
        assert!(
            !cmd.cwd.exists(),
            "the scratch folder remained after the query"
        );
        assert_eq!(
            serde_json::from_str::<Value>(&settings).expect("JSON"),
            json!({ "disableAllHooks": true })
        );
        assert_eq!(
            serde_json::from_str::<Value>(&mcp).expect("JSON"),
            json!({ "mcpServers": {} })
        );

        let path = |name: &str| dir.join(name).to_str().expect("Unicode").to_owned();
        let expected_args: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--replay-user-messages",
            "--verbose",
            "--no-session-persistence",
            "--settings",
            &path(SETTINGS_FILE),
            "--strict-mcp-config",
            "--mcp-config",
            &path(MCP_CONFIG_FILE),
        ]
        .iter()
        .map(|a| (*a).to_owned())
        .collect();
        let (program, args) = console_command("claude", expected_args);
        assert_eq!(cmd.program, program);
        assert_eq!(cmd.args, args);
        assert!(
            !cmd.args
                .iter()
                .any(|a| a.contains("bypassPermissions") || a == "--permission-mode"),
            "the query has no permission-bypass argument: {:?}",
            cmd.args
        );

        assert_eq!(
            cmd.env_remove,
            [
                "CLAUDECODE",
                "CLAUDE_CODE_ENTRYPOINT",
                "claude_lower_case",
                "CLAUDE_CODE_AUTO_CONNECT_IDE"
            ],
            "strips only CLAUDE* and keeps the two login keys (case-insensitive)"
        );
        let set: Vec<(&str, &str)> = cmd
            .env_set
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            set,
            [
                ("MAX_THINKING_TOKENS", "8000"),
                ("ENABLE_CLAUDEAI_MCP_SERVERS", "false"),
                ("CLAUDE_CODE_AUTO_CONNECT_IDE", "0"),
                ("CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL", "1"),
                ("DISABLE_AUTOUPDATER", "1"),
            ]
        );
    }

    #[test]
    fn non_utf8_env_keys_are_skipped() {
        #[cfg(windows)]
        let odd = {
            use std::os::windows::ffi::OsStringExt;
            OsString::from_wide(&[0x43, 0x4C, 0x41, 0x55, 0x44, 0x45, 0xD800])
        };
        #[cfg(unix)]
        let odd = {
            use std::os::unix::ffi::OsStringExt;
            OsString::from_vec(b"CLAUDE\xFF".to_vec())
        };
        assert_eq!(
            stripped_env_keys([odd, OsString::from("CLAUDE_X")]),
            ["CLAUDE_X"]
        );
    }

    /// The production path (`query`) reads the daemon process env — whatever is there, only `CLAUDE*` is stripped and the two login keys remain.
    #[test]
    fn query_strips_only_claude_keys_from_the_process_env() {
        let root = TempRoot::new("claude-probe-process-env");
        let spawner = ScriptedSpawner::new(answering(typical_usage()));
        let env = ProbeEnv {
            spawner: &spawner,
            deadline: Instant::now() + ROOMY,
            scratch_root: root.path(),
        };
        CLAUDE_USAGE_PROBE.query(&env).expect("query");
        let expected = stripped_env_keys(std::env::vars_os().map(|(key, _)| key));
        let cmd = spawner.commands().pop().expect("command");
        assert_eq!(cmd.env_remove, expected);
        for key in &cmd.env_remove {
            assert!(key.to_ascii_uppercase().starts_with("CLAUDE"), "{key}");
            assert!(
                !KEPT_CLAUDE_ENV.iter().any(|k| key.eq_ignore_ascii_case(k)),
                "{key}"
            );
        }
    }

    /// ★The two copies of the stream-json core arguments must be the same★ — if the agent spawn changes that combination (meaning the CLI requirement changed),
    ///   this test breaks so the query side gets fixed too. The query arguments are not derived from the spawn.
    #[test]
    fn stream_json_core_args_match_the_agent_spawn() {
        let spawn = ClaudeBackend.build_spec(
            &AgentCommand::Claude {
                extra_args: vec![],
                output_format: AgentOutputFormat::StreamJson,
            },
            SpawnMode::Fresh,
            None,
            None,
            PathBuf::from("."),
            vec![],
            None,
        );
        assert!(
            spawn
                .args
                .windows(STREAM_JSON_CORE.len())
                .any(|w| w == STREAM_JSON_CORE),
            "the spawn's stream-json core arguments drifted from the query side: {:?}",
            spawn.args
        );
        let root = TempRoot::new("claude-probe-parity");
        let probe = probe_command(root.path(), Vec::new()).expect("command");
        assert!(probe
            .args
            .windows(STREAM_JSON_CORE.len())
            .any(|w| w == STREAM_JSON_CORE));
        assert_eq!(
            probe.program, spawn.program,
            "goes through the same shell wrapping"
        );
        let thinking = |env: &[(String, String)]| {
            env.iter()
                .find(|(k, _)| k == "MAX_THINKING_TOKENS")
                .map(|(_, v)| v.clone())
        };
        assert_eq!(thinking(&probe.env_set), thinking(&spawn.env));
    }
}
