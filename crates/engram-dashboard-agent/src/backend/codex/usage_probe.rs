//! Active usage query for codex — starts a short-lived `codex app-server --stdio`, exchanges `initialize` → `initialized` →
//! `account/rateLimits/read`, and decodes the response into a [`UsageObservation`]. Callers see [`CODEX_USAGE_PROBE`] only as a [`UsageProbe`].
//!
//! ★No model turn★ — no thread is opened (no `thread/start`). After the read response (or a refusal) it closes stdin and ends
//!   ([`finish_after_answer`]).
//! ★Does not use the conversation path's request machinery (`transport`)★ — its wait table and writer thread are tied to that path's state;
//!   here requests are written one at a time and the response is read on one thread.
//! ★The response's `accountId` is not read at all — it is nowhere in logs · errors · observations · the raw-text field★ (TRD §1-3): upstream
//!   strings (response · RPC error text · stderr) and serde error text (it can repeat the value) never go into logs or errors. Exceptions:
//!   short display names (plan · bucket name) and the RPC `error.message` (raw-text-only [`UsageDetail::upstream`]; `data` is not put in).
//! Shape source = codex-cli 0.156.1's JSON Schema (`GetAccountRateLimitsResponse` doc in `protocol`).
// ADR-0004

use std::path::Path;
use std::time::{Duration, Instant};

use engram_dashboard_platform::shell::console_command;
use serde::Serialize;
use serde_json::Value;

use super::protocol::{
    classify, error_response_line, method, notification_line, request_line, ClientInfo,
    GetAccountRateLimitsParams, GetAccountRateLimitsResponse, Inbound, InitializeParams,
    RateLimitSnapshot, RequestId, RpcError, CLIENT_NAME, METHOD_NOT_FOUND,
};
use super::usage::{names_default_bucket, windows_by_duration, DEFAULT_LIMIT_ID, USAGE_VENDOR};
use super::{APP_SERVER_STDIO_FLAG, APP_SERVER_SUBCOMMAND, CODEX_PROGRAM};
use crate::usage::{
    display_text, finish_after_answer, has_word, ProbeChild, ProbeCommand, ProbeEnv, ProbeError,
    ProbeFailure, ScopedWindowObs, ScratchDir, UpstreamText, UsageDetail, UsageObservation,
    UsagePolicy, UsageProbe, UsageSource, UsageVendorKey,
};

/// Minimum interval between automatic queries (same value as Claude).
const CODEX_COOLDOWN: Duration = Duration::from_secs(15 * 60);

/// The time limit of one query — prevents hangs. Measured total 0.74–0.94 s (0.156.1); `initialize` was once ~9.7 s right after a daemon
/// restart (cause unknown — TRD §6 #5).
const CODEX_PROBE_TIMEOUT: Duration = Duration::from_secs(30);

const INITIALIZE_ID: i64 = 1;
const READ_ID: i64 = 2;

/// The text put in when refusing a server request — it goes to the peer's log.
const REFUSAL_MESSAGE: &str = "the engram-dashboard usage query does not handle server requests";

/// Character cap for external-string fields (plan · bucket name) — they are display names, so anything longer is truncated.
const PLAN_MAX_CHARS: usize = 32;
const LABEL_MAX_CHARS: usize = 48;
/// Cap on the per-model window list. Today's response has one non-default bucket.
const MODEL_SCOPED_MAX: usize = 16;

/// Response classification words ([`UsageDetail::kind`]) — they go out on the wire and the screen shows them without translation.
const KIND_RPC_ERROR: &str = "rpc_error";

pub(crate) struct CodexUsageProbe;

pub(crate) static CODEX_USAGE_PROBE: CodexUsageProbe = CodexUsageProbe;

impl UsageProbe for CodexUsageProbe {
    fn key(&self) -> UsageVendorKey {
        USAGE_VENDOR
    }

    fn policy(&self) -> UsagePolicy {
        UsagePolicy {
            cooldown: CODEX_COOLDOWN,
            timeout: CODEX_PROBE_TIMEOUT,
        }
    }

    fn query(&self, env: &ProbeEnv<'_>) -> Result<UsageObservation, ProbeFailure> {
        env.require_time_left()?;
        // ★Declaration order decides drop order — the folder before the child★: Windows deletes the working folder only after the child is
        //   dropped.
        let scratch = ScratchDir::create(env.scratch_root)?;
        let mut child = env
            .spawner
            .spawn(&probe_command(scratch.path()), env.deadline)?;
        let answer = converse(child.as_mut(), env.deadline)?;
        finish_after_answer(child, env.deadline);
        observation_from_result(answer?)
    }
}

/// The env is inherited from the daemon as is (TRD §3 #9 — the default login). No conversation arguments (`--cd` · policy · `-c`) — no
///   thread is opened.
fn probe_command(scratch: &Path) -> ProbeCommand {
    let (program, args) = console_command(
        CODEX_PROGRAM,
        vec![
            APP_SERVER_SUBCOMMAND.to_owned(),
            APP_SERVER_STDIO_FLAG.to_owned(),
        ],
    );
    ProbeCommand {
        program,
        args,
        cwd: scratch.to_path_buf(),
        env_set: Vec::new(),
        env_remove: Vec::new(),
    }
}

/// The server's answer to one request — `result` on success, its classification ([`classify_rpc_error`]) on an error response. Distinct from the
/// outer `Result` error (no answer): once answered, the child gets a grace period to end.
type Answer = Result<Value, ProbeFailure>;

/// Handshake → read request → its answer; each request waits for its response before the next write. A refused handshake is the answer.
fn converse(child: &mut dyn ProbeChild, deadline: Instant) -> Result<Answer, ProbeError> {
    let initialize = InitializeParams {
        client_info: ClientInfo {
            name: CLIENT_NAME.to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        capabilities: None,
    };
    let initialize_line = request_text(INITIALIZE_ID, method::INITIALIZE, &initialize)?;
    // If the wrapping shell could not find the target and already exited, the first write fails first — returning at once would miss the exit
    //   status that carries 「not installed」. Hold the error, read to EOF, tell apart by the exit.
    let write_error = match send(child, &initialize_line, deadline) {
        Ok(()) => None,
        Err(err @ ProbeError::Io(_)) => Some(err),
        Err(err) => return Err(err),
    };
    if let Err(rejected) = await_result(child, INITIALIZE_ID, deadline, write_error)? {
        return Ok(Err(rejected));
    }
    send(child, &notification_line(method::INITIALIZED), deadline)?;
    send(
        child,
        &request_text(
            READ_ID,
            method::ACCOUNT_RATE_LIMITS_READ,
            &GetAccountRateLimitsParams::default(),
        )?,
        deadline,
    )?;
    await_result(child, READ_ID, deadline, None)
}

fn request_text<P: Serialize>(id: i64, method: &str, params: &P) -> Result<String, ProbeError> {
    request_line(&RequestId::Num(id), method, params).map_err(|_| {
        ProbeError::Io("could not build the codex usage query request line".to_owned())
    })
}

/// Both the `protocol` line tool and [`ProbeChild::write_line`] attach the newline — unstripped, an extra empty line goes out.
fn send(child: &mut dyn ProbeChild, line: &str, deadline: Instant) -> Result<(), ProbeError> {
    child.write_line(line.strip_suffix('\n').unwrap_or(line), deadline)
}

/// Read until the response for our request `id` — an error response is an answer too (classified and returned).
///
/// - A server **request** gets a refusal response — unanswered, the peer may stall.
/// - Notifications · other ids · lines unreadable as an envelope are discarded.
/// - ★A line with our id that is not a valid envelope (both success and failure, or a broken error body) is `Parse` at once★ — waiting will
///   not bring another response. A non-JSON line has no readable id and is discarded (→ time limit).
/// - `write_error` = the I/O error of that request's failed write — returned if it ends without a response and is not 「not installed」.
fn await_result(
    child: &mut dyn ProbeChild,
    id: i64,
    deadline: Instant,
    write_error: Option<ProbeError>,
) -> Result<Answer, ProbeError> {
    let ours = RequestId::Num(id);
    let mut read_error = write_error;
    loop {
        let line = match child.read_line(deadline) {
            Ok(Some(line)) => line,
            Ok(None) => return Err(ended_without_response(child, deadline, read_error)),
            // Either a single line that hit the cap or a read failure — in the former the stream continues to the next line, in the latter
            //   EOF comes soon.
            Err(err @ ProbeError::Io(_)) => {
                read_error = Some(err);
                continue;
            }
            Err(err) => return Err(err),
        };
        match classify(&line) {
            Ok(Inbound::Response { id, result }) if id == ours => return Ok(Ok(result)),
            Ok(Inbound::Error { id, error }) if id == ours => {
                return Ok(Err(classify_rpc_error(&error)))
            }
            Ok(Inbound::Request { id, .. }) => refuse(child, &id, deadline)?,
            Ok(_) => {}
            Err(_) if is_reply_to(&line, id) => {
                return Err(parse_error(
                    "the response line to our request does not read as an envelope",
                ));
            }
            Err(_) => {}
        }
    }
}

/// Is it a line with no `method`, whose `id` is `id`, and that carries at least one of `result` · `error`.
fn is_reply_to(line: &str, id: i64) -> bool {
    let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(line) else {
        return false;
    };
    !obj.contains_key("method")
        && obj.get("id").and_then(Value::as_i64) == Some(id)
        && (obj.contains_key("result") || obj.contains_key("error"))
}

/// ★Do not fake success★ — a success response to an approval request is an automatic approval (same spot in `transport`). The request content is
/// not written down (upstream-decided string).
fn refuse(child: &mut dyn ProbeChild, id: &RequestId, deadline: Instant) -> Result<(), ProbeError> {
    tracing::debug!("codex usage query: refusing a server request");
    send(
        child,
        &error_response_line(id, METHOD_NOT_FOUND, REFUSAL_MESSAGE),
        deadline,
    )
}

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
    // The stderr content is not put in — what gets masked is only token shapes, and account identifiers · paths may remain. Only the line
    //   count is written.
    tracing::debug!(
        code = ?exit.code(),
        stderr_lines = child.stderr_tail().len(),
        "codex usage query: ended without a response"
    );
    if let Some(err) = read_error {
        return err;
    }
    ProbeError::Upstream(match exit.code() {
        Some(code) => format!("codex ended without a usage response (exit code {code})"),
        None => "codex ended without a usage response (no exit code)".to_owned(),
    })
}

/// ★This phrase table is a guess★ — codex's not-logged-in error text has not been seen (TRD §6 #5 — the stage-3 real smoke collects it).
///   If wrong, it falls into `Upstream` (= query failure). The limit text is deliberately not split into [`ProbeError::RateLimited`] (TRD
/// §3 #7). Evidence is the same for every classification — [`KIND_RPC_ERROR`] + `error.code` + raw `error.message` (none if only whitespace).
fn classify_rpc_error(error: &RpcError) -> ProbeFailure {
    const UNAUTHENTICATED: [&str; 10] = [
        "authenticat",
        "unauthenticated",
        "unauthorized",
        "not logged in",
        "logged out",
        "log in",
        "login",
        "log out",
        "sign in",
        "signed out",
    ];
    let text = error.message.to_ascii_lowercase();
    let classified = if UNAUTHENTICATED.iter().any(|w| has_word(&text, w, false))
        || has_word(&text, "401", true)
    {
        ProbeError::Unauthenticated
    } else {
        ProbeError::Upstream(format!(
            "codex refused the usage query with an error (code {})",
            error.code
        ))
    };
    tracing::debug!(kind = %classified, "codex usage query: error response");
    ProbeFailure::with_detail(
        classified,
        UsageDetail {
            kind: KIND_RPC_ERROR,
            code: Some(error.code),
            upstream: UpstreamText::non_blank(&error.message),
        },
    )
}

/// The read response's `result` → observation.
///
/// - ★Default bucket = [`DEFAULT_LIMIT_ID`] in `rateLimitsByLimitId` first★ — the top-level `rateLimits` is a 「backward-compatible
///   single-bucket view」 that can mirror another bucket; use it only when the map lacks the default and its `limitId` names the default
///   ([`names_default_bucket`]).
/// - ★Default bucket not found = `Parse`★ — reading it as 「no windows」 would let other-bucket-only responses succeed and wipe every held value;
///   on failure the receiving side keeps its values. Also `Parse` if `result` is not an object.
/// - Windows by duration ([`windows_by_duration`], sibling `usage`). `None` field = not carried (the receiving side keeps it).
/// - Other map buckets → per-model windows ([`model_scoped_windows`]); `planType` → plan. ★If the map was read, per-model windows are `Some`
///   even when empty★ — `None` would leave a vanished model window forever. No map (top level only) → other buckets unknown → `None`.
fn observation_from_result(result: Value) -> Result<UsageObservation, ProbeFailure> {
    let response: GetAccountRateLimitsResponse = serde_json::from_value(result)
        .map_err(|_| parse_error("the response result is not an object"))?;
    let buckets = response.rate_limits_by_limit_id;
    let top_level = response.rate_limits.as_ref();
    let default = buckets
        .iter()
        .flatten()
        .find(|(id, _)| id == DEFAULT_LIMIT_ID)
        .map(|(_, snapshot)| snapshot)
        .or_else(|| top_level.filter(|snapshot| names_default_bucket(snapshot.limit_id.as_ref())))
        .ok_or_else(|| parse_error("there is no snapshot of the default bucket"))?;
    let (five_hour, weekly) = windows_by_duration(default);
    let plan = [Some(default), top_level]
        .into_iter()
        .flatten()
        .filter_map(|snapshot| snapshot.plan_type.as_deref())
        .find_map(|plan| display_text(plan, PLAN_MAX_CHARS));
    Ok(UsageObservation {
        vendor: USAGE_VENDOR,
        five_hour,
        weekly,
        model_scoped: buckets.as_deref().map(model_scoped_windows),
        plan,
        source: UsageSource::Active,
        limits_unavailable: None,
    })
}

fn parse_error(what: &str) -> ProbeError {
    ProbeError::Parse(format!("codex usage response: {what}"))
}

/// Each non-default bucket's one **weekly** window (D8) — name = `limitName`, or the bucket id if unusable. Buckets with no weekly window or no
/// usable name are left out. Same name (ASCII case-insensitive): the first stays (bucket id order).
fn model_scoped_windows(buckets: &[(String, RateLimitSnapshot)]) -> Vec<ScopedWindowObs> {
    let mut windows: Vec<ScopedWindowObs> = Vec::new();
    for (id, snapshot) in buckets {
        if windows.len() >= MODEL_SCOPED_MAX {
            break;
        }
        if id == DEFAULT_LIMIT_ID {
            continue;
        }
        let label = snapshot
            .limit_name
            .as_deref()
            .and_then(|name| display_text(name, LABEL_MAX_CHARS))
            .or_else(|| display_text(id, LABEL_MAX_CHARS));
        let (Some(label), (_, Some(window))) = (label, windows_by_duration(snapshot)) else {
            continue;
        };
        if windows.iter().any(|w| w.label.eq_ignore_ascii_case(&label)) {
            continue;
        }
        windows.push(ScopedWindowObs { label, window });
    }
    windows
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use crate::usage::testing::{ChildScript, ScriptedSpawner, TempRoot};
    use crate::usage::{ExitInfo, ProbeSpawner, WindowObs};

    const SHORT: Duration = Duration::from_millis(300);
    const ROOMY: Duration = Duration::from_secs(4);

    const FIVE_HOUR_RESET: i64 = 1_790_424_706;
    const WEEKLY_RESET: i64 = 1_790_739_378;
    const MODEL_RESET: i64 = 1_791_018_755;

    /// A fake account identifier — this value must not be visible in any result.
    const ACCOUNT_ID: &str = "acct-SECRET-7f3a91";

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
        CODEX_USAGE_PROBE.query(&env)
    }

    fn run(script: ChildScript) -> (Result<UsageObservation, ProbeFailure>, ScriptedSpawner) {
        let root = TempRoot::new("codex-probe");
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
            "the temp folder remained after the query: {left:?}"
        );
    }

    fn init_response() -> String {
        json!({
            "id": 1,
            "result": {
                "codexHome": "C:\\Users\\someone\\.codex",
                "platformFamily": "windows",
                "platformOs": "windows",
                "userAgent": "engram-dashboard/0.0.0 (test)",
            },
        })
        .to_string()
    }

    fn read_response(result: Value) -> String {
        json!({ "id": 2, "result": result }).to_string()
    }

    fn answering(result: Value) -> ChildScript {
        ChildScript::new()
            .line(init_response())
            .line(read_response(result))
    }

    fn window_json(used: i64, mins: i64, resets: i64) -> Value {
        json!({ "usedPercent": used, "windowDurationMins": mins, "resetsAt": resets })
    }

    fn codex_bucket() -> Value {
        json!({
            "limitId": "codex",
            "limitName": null,
            "planType": "plus",
            "primary": window_json(37, 300, FIVE_HOUR_RESET),
            "secondary": window_json(58, 10080, WEEKLY_RESET),
            "credits": null,
        })
    }

    /// The measured shape of research doc §4-1 (numbers are that measurement · identifiers are fake) + the remaining fields of the schema.
    fn model_bucket() -> Value {
        json!({
            "limitId": "base_model_inference",
            "limitName": "gpt-reserve",
            "normalModelSlug": "gpt-5.6-luna",
            "planType": "plus",
            "primary": window_json(0, 10080, MODEL_RESET),
            "secondary": null,
        })
    }

    fn typical_result() -> Value {
        json!({
            "accountId": ACCOUNT_ID,
            "rateLimits": codex_bucket(),
            "rateLimitsByLimitId": { "codex": codex_bucket(), "base_model_inference": model_bucket() },
            "rateLimitResetCredits": { "availableCount": 0, "credits": null },
            "ordinaryUsageAllowed": true,
        })
    }

    fn window(used_pct: Option<f64>, resets_at: Option<i64>) -> Option<WindowObs> {
        Some(WindowObs {
            used_pct,
            resets_at,
        })
    }

    fn model_window(used_pct: f64) -> WindowObs {
        WindowObs {
            used_pct: Some(used_pct),
            resets_at: Some(MODEL_RESET),
        }
    }

    fn labels(obs: &UsageObservation) -> Vec<String> {
        obs.model_scoped
            .iter()
            .flatten()
            .map(|w| w.label.clone())
            .collect()
    }

    fn observe(result: Value) -> Result<UsageObservation, ProbeFailure> {
        observation_from_result(result)
    }

    fn is_bare_parse(result: &Result<UsageObservation, ProbeFailure>) -> bool {
        matches!(
            result,
            Err(ProbeFailure {
                error: ProbeError::Parse(_),
                detail: None,
            })
        )
    }

    // ── Policy ──

    #[test]
    fn policy_and_key_are_the_codex_values() {
        assert_eq!(CODEX_USAGE_PROBE.key(), USAGE_VENDOR);
        assert_eq!(CODEX_USAGE_PROBE.key().as_str(), "codex");
        assert_eq!(
            CODEX_USAGE_PROBE.policy(),
            UsagePolicy {
                cooldown: Duration::from_secs(900),
                timeout: Duration::from_secs(30),
            }
        );
    }

    // ── Interpretation ──

    #[test]
    fn success_reads_the_codex_bucket_and_the_other_buckets_weekly_window() {
        let (result, spawner) = run(answering(typical_result()));
        let obs = result.expect("query");
        assert_eq!(obs.vendor, USAGE_VENDOR);
        assert_eq!(obs.source, UsageSource::Active);
        assert_eq!(obs.five_hour, window(Some(37.0), Some(FIVE_HOUR_RESET)));
        assert_eq!(obs.weekly, window(Some(58.0), Some(WEEKLY_RESET)));
        assert_eq!(
            obs.model_scoped,
            Some(vec![ScopedWindowObs {
                label: "gpt-reserve".to_owned(),
                window: model_window(0.0),
            }])
        );
        assert_eq!(obs.plan.as_deref(), Some("plus"));
        assert!(
            spawner.stdin_closed(),
            "did not close stdin after the response"
        );
        assert!(
            spawner.child_dropped(),
            "did not kill the child after the response"
        );
        assert!(!spawner.cut_at_deadline());
    }

    /// A child that does not end even after stdin is closed after the response — it is interrupted after the grace period, but the query
    ///   succeeds.
    #[test]
    fn a_child_that_lingers_after_the_answer_is_cut_but_the_query_succeeds() {
        let (result, spawner) = run(answering(typical_result()).hang_after_lines());
        assert_eq!(result.expect("query").plan.as_deref(), Some("plus"));
        assert!(spawner.stdin_closed());
        assert!(
            spawner.cut_at_deadline(),
            "should have cut after the grace period"
        );
        assert!(spawner.child_dropped());
    }

    /// If the deadline has already passed, neither a temp folder nor a child is created.
    #[test]
    fn a_passed_deadline_times_out_before_anything_is_created() {
        let root = TempRoot::new("codex-probe-late");
        let spawner = ScriptedSpawner::new(answering(typical_result()));
        assert_eq!(
            run_with_deadline(&spawner, &root, Duration::ZERO),
            Err(ProbeError::Timeout.into())
        );
        assert!(spawner.commands().is_empty(), "attempted a launch");
        assert_scratch_gone(&root);
    }

    /// As in the measurement of research doc §4-1, if the map has no default bucket we use the top level, and the map's buckets become
    ///   per-model windows.
    #[test]
    fn without_the_codex_bucket_in_the_map_the_top_level_view_is_used() {
        let mut top_level = codex_bucket();
        top_level["limitId"] = Value::Null;
        for by_limit_id in [
            json!({ "base_model_inference": model_bucket() }),
            json!(null),
            json!({}),
        ] {
            let has_model = by_limit_id.get("base_model_inference").is_some();
            let obs =
                observe(json!({ "rateLimits": top_level, "rateLimitsByLimitId": by_limit_id }))
                    .expect("observation");
            assert_eq!(obs.five_hour, window(Some(37.0), Some(FIVE_HOUR_RESET)));
            assert_eq!(obs.weekly, window(Some(58.0), Some(WEEKLY_RESET)));
            assert_eq!(obs.plan.as_deref(), Some("plus"));
            assert_eq!(
                labels(&obs),
                if has_model {
                    vec!["gpt-reserve".to_owned()]
                } else {
                    vec![]
                }
            );
            assert_eq!(
                obs.model_scoped.is_some(),
                by_limit_id.is_object(),
                "if the map was read, it is carried even when empty: {by_limit_id}"
            );
        }
        let mut bare = codex_bucket();
        bare.as_object_mut().expect("object").remove("limitId");
        let obs = observe(json!({ "rateLimits": bare })).expect("observation");
        assert_eq!(obs.weekly, window(Some(58.0), Some(WEEKLY_RESET)));
        assert_eq!(obs.model_scoped, None);
    }

    /// ★Even if the top level mirrors another bucket, the map's default bucket wins★ — the top-level value must not go into the account window.
    #[test]
    fn the_map_wins_over_a_top_level_view_of_another_limit() {
        let obs = observe(json!({
            "rateLimits": model_bucket(),
            "rateLimitsByLimitId": { "codex": codex_bucket(), "base_model_inference": model_bucket() },
        }))
        .expect("observation");
        assert_eq!(obs.five_hour, window(Some(37.0), Some(FIVE_HOUR_RESET)));
        assert_eq!(obs.weekly, window(Some(58.0), Some(WEEKLY_RESET)));
        assert_eq!(labels(&obs), ["gpt-reserve"]);

        // The map winning holds even when the map's window is empty — we do not fill it from the top level.
        let mut empty = codex_bucket();
        empty["primary"] = Value::Null;
        empty["secondary"] = Value::Null;
        let obs = observe(json!({
            "rateLimits": codex_bucket(),
            "rateLimitsByLimitId": { "codex": empty },
        }))
        .expect("observation");
        assert_eq!(obs.five_hour, None);
        assert_eq!(obs.weekly, None);
    }

    #[test]
    fn no_default_bucket_anywhere_is_a_parse_error() {
        for result in [
            json!({ "rateLimits": model_bucket() }),
            json!({ "rateLimits": model_bucket(), "rateLimitsByLimitId": { "base_model_inference": model_bucket() } }),
            json!({ "rateLimitsByLimitId": { "base_model_inference": model_bucket() } }),
            json!({ "rateLimits": null }),
            json!({}),
            json!({ "rateLimits": { "limitId": "Codex", "primary": window_json(1, 300, FIVE_HOUR_RESET) } }),
            json!({ "rateLimits": { "limitId": 5, "primary": window_json(1, 300, FIVE_HOUR_RESET) } }),
        ] {
            let got = observe(result.clone());
            assert!(is_bare_parse(&got), "{result}: {got:?}");
        }
    }

    #[test]
    fn null_windows_are_absent_in_an_active_observation() {
        let obs = observe(json!({
            "rateLimits": { "limitId": "codex", "planType": "free", "primary": null, "secondary": null },
        }))
        .expect("observation");
        assert_eq!(obs.five_hour, None);
        assert_eq!(obs.weekly, None);
        assert_eq!(obs.model_scoped, None);
        assert_eq!(obs.plan.as_deref(), Some("free"));
        assert_eq!(obs.source, UsageSource::Active);
    }

    #[test]
    fn unknown_or_missing_durations_are_dropped() {
        let bucket = |primary: Value, secondary: Value| {
            observe(json!({ "rateLimits": { "primary": primary, "secondary": secondary } }))
                .expect("observation")
        };
        let obs = bucket(
            window_json(40, 43_200, 5),
            window_json(7, 10080, WEEKLY_RESET),
        );
        assert_eq!(
            obs.five_hour, None,
            "monthly window — not one of our fields"
        );
        assert_eq!(obs.weekly, window(Some(7.0), Some(WEEKLY_RESET)));

        let obs = bucket(
            json!({ "usedPercent": 40, "resetsAt": FIVE_HOUR_RESET }),
            json!(null),
        );
        assert_eq!(
            obs.five_hour, None,
            "without a length, do not guess from the slot name"
        );
        assert_eq!(obs.weekly, None);

        let obs = observe(json!({
            "rateLimits": codex_bucket(),
            "rateLimitsByLimitId": {
                "five_hour_only": { "primary": window_json(3, 300, FIVE_HOUR_RESET) },
                "monthly": { "primary": window_json(3, 43_200, FIVE_HOUR_RESET) },
            },
        }))
        .expect("observation");
        assert_eq!(obs.model_scoped, Some(vec![]));
    }

    /// ★At no layer does an array get read in field order and turn into a plausible fake window★.
    #[test]
    fn arrays_where_objects_are_expected_never_become_windows() {
        let array_bucket = json!([
            "codex",
            [37, 300, FIVE_HOUR_RESET],
            [58, 10080, WEEKLY_RESET]
        ]);
        let parse_cases = [
            json!([{ "rateLimits": codex_bucket() }]),
            json!({ "rateLimits": array_bucket }),
            json!({ "rateLimitsByLimitId": { "codex": array_bucket } }),
            json!({ "rateLimits": array_bucket, "rateLimitsByLimitId": [["codex", codex_bucket()]] }),
        ];
        for result in parse_cases {
            let got = observe(result.clone());
            assert!(is_bare_parse(&got), "{result}: {got:?}");
        }

        for by_limit_id in [
            json!([["codex", codex_bucket()]]),
            json!({ "codex": array_bucket }),
            json!({ "codex": "codex" }),
            json!(7),
        ] {
            let obs = observe(
                json!({ "rateLimits": codex_bucket(), "rateLimitsByLimitId": by_limit_id }),
            )
            .expect("observation");
            assert_eq!(
                obs.weekly,
                window(Some(58.0), Some(WEEKLY_RESET)),
                "{by_limit_id}"
            );
            assert_eq!(
                obs.model_scoped,
                by_limit_id.is_object().then(Vec::new),
                "{by_limit_id}"
            );
        }

        let obs = observe(json!({
            "rateLimits": {
                "limitId": "codex",
                "primary": [37, 300, FIVE_HOUR_RESET],
                "secondary": window_json(58, 10080, WEEKLY_RESET),
            },
            "rateLimitsByLimitId": {
                "base_model_inference": ["gpt-reserve", [0, 10080, MODEL_RESET]],
                "other": { "limitName": "other", "primary": [0, 10080, MODEL_RESET] },
            },
        }))
        .expect("observation");
        assert_eq!(obs.five_hour, None);
        assert_eq!(obs.weekly, window(Some(58.0), Some(WEEKLY_RESET)));
        assert_eq!(obs.model_scoped, Some(vec![]));
    }

    #[test]
    fn model_scoped_labels_are_named_trimmed_deduplicated_and_capped() {
        let weekly = |used: i64| window_json(used, 10080, MODEL_RESET);
        let obs = observe(json!({
            "rateLimits": codex_bucket(),
            "rateLimitsByLimitId": {
                "codex": codex_bucket(),
                "b_no_name": { "primary": weekly(1) },
                "a_named": { "limitName": "  Reserve  ", "secondary": weekly(2) },
                "c_bad_name": { "limitName": "bad\u{7}name", "primary": weekly(3) },
                "d_dup": { "limitName": "reserve", "primary": weekly(4) },
                "e_number_name": { "limitName": 5, "primary": weekly(5) },
                "f_no_window": { "limitName": "empty", "primary": null },
            },
        }))
        .expect("observation");
        assert_eq!(
            labels(&obs),
            ["Reserve", "b_no_name", "c_bad_name", "e_number_name"],
            "bucket id order · id when there is no name · for the same name, the one that came first"
        );
        let scoped = obs.model_scoped.expect("per-model windows");
        assert_eq!(scoped[0].window, model_window(2.0));
        assert_eq!(scoped[2].window, model_window(3.0));

        let many: serde_json::Map<String, Value> = (0..40)
            .map(|i| (format!("bucket-{i:02}"), json!({ "primary": weekly(i) })))
            .collect();
        let obs = observe(json!({ "rateLimits": codex_bucket(), "rateLimitsByLimitId": many }))
            .expect("observation");
        assert_eq!(
            obs.model_scoped.expect("per-model windows").len(),
            MODEL_SCOPED_MAX
        );

        let obs = observe(json!({
            "rateLimits": codex_bucket(),
            "rateLimitsByLimitId": { "long": { "limitName": "가".repeat(500), "primary": weekly(1) } }, // hangul-ok: multi-byte char for character-unit truncation
        }))
        .expect("observation");
        let scoped = obs.model_scoped.expect("per-model windows");
        assert_eq!(
            scoped[0].label.chars().count(),
            LABEL_MAX_CHARS,
            "cuts by character"
        );
    }

    #[test]
    fn the_plan_comes_from_the_default_bucket_then_the_top_level() {
        let plan = |default_plan: Value, top_plan: Value| {
            let mut default = codex_bucket();
            default["planType"] = default_plan;
            let mut top = model_bucket();
            top["planType"] = top_plan;
            observe(json!({ "rateLimits": top, "rateLimitsByLimitId": { "codex": default } }))
                .expect("observation")
                .plan
        };
        assert_eq!(plan(json!("pro"), json!("plus")).as_deref(), Some("pro"));
        assert_eq!(plan(json!(null), json!("plus")).as_deref(), Some("plus"));
        assert_eq!(plan(json!(" team\n"), json!(null)).as_deref(), Some("team"));
        assert_eq!(
            plan(json!("x".repeat(10_000)), json!(null)).map(|p| p.chars().count()),
            Some(PLAN_MAX_CHARS)
        );
        for bad in [
            json!(5),
            json!(""),
            json!("pl\nus"),
            json!({ "tier": "pro" }),
            json!(null),
        ] {
            assert_eq!(plan(bad.clone(), json!(null)), None, "{bad}");
        }
    }

    // ── Errors ──

    fn kind(err: &ProbeError) -> &'static str {
        match err {
            ProbeError::Unauthenticated => "auth",
            ProbeError::Upstream(_) => "upstream",
            other => panic!("outside the classification: {other:?}"),
        }
    }

    fn rpc_error(message: &str) -> RpcError {
        RpcError {
            code: -32600,
            message: message.to_owned(),
            data: None,
        }
    }

    #[test]
    fn rpc_error_text_classification_table() {
        let cases = [
            (
                "codex account authentication required to read rate limits",
                "auth",
            ),
            (
                "chatgpt authentication required to read rate limits",
                "auth",
            ),
            (
                "failed to fetch codex rate limits: unexpected status 401 Unauthorized",
                "auth",
            ),
            ("Not logged in. Run `codex login`", "auth"),
            ("status=401", "auth"),
            ("please sign in again", "auth"),
            (
                "failed to fetch codex rate limits: 429 Too Many Requests",
                "upstream",
            ),
            ("internal error", "upstream"),
            ("", "upstream"),
            ("request req_14010a failed", "upstream"),
            ("catalog index failed", "upstream"),
            ("bloglogin", "upstream"),
        ];
        for (text, expected) in cases {
            assert_eq!(
                kind(&classify_rpc_error(&rpc_error(text)).error),
                expected,
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_blank_rpc_message_carries_the_code_without_upstream() {
        for blank in ["", "  ", "   "] {
            assert_eq!(
                classify_rpc_error(&rpc_error(blank))
                    .detail
                    .and_then(|d| d.upstream),
                None,
                "{blank:?}"
            );
            let failure = classify_rpc_error(&rpc_error(blank));
            let detail = failure.detail.expect("detail");
            assert_eq!(detail.kind, KIND_RPC_ERROR, "{blank:?}");
            assert_eq!(detail.code, Some(-32600), "{blank:?}");
            assert_eq!(detail.upstream, None, "{blank:?}");
        }
    }

    /// The read request's and the handshake's error responses classify the same way — evidence = [`KIND_RPC_ERROR`] + code + raw `message`
    ///   (token shapes masked · no `data`), and the raw text is not visible in `Display` · `Debug`.
    #[test]
    fn rpc_errors_carry_the_code_and_message_only_in_the_detail() {
        const MARKER: &str = "someone@example.com";
        const TOKEN: &str = "sk-proj-abcdefghijklmnopqrstuvwxyz";
        let error_line = |id: i64, code: i64, message: String| {
            json!({
                "id": id,
                "error": { "code": code, "message": message, "data": { "accountId": ACCOUNT_ID } },
            })
            .to_string()
        };
        let cases = [
            (
                2,
                -32600,
                format!("codex account authentication required ({MARKER} {TOKEN})"),
                "auth",
                format!("codex account authentication required ({MARKER} ***)"),
            ),
            (
                2,
                -32603,
                format!("failed to fetch codex rate limits for {MARKER}"),
                "upstream",
                format!("failed to fetch codex rate limits for {MARKER}"),
            ),
            (
                1,
                -32000,
                format!("boom {MARKER}"),
                "upstream",
                format!("boom {MARKER}"),
            ),
        ];
        for (id, code, message, expected, upstream) in cases {
            let mut script = ChildScript::new();
            if id == 2 {
                script = script.line(init_response());
            }
            let (result, spawner) = run(script.line(error_line(id, code, message)));
            let err = result.expect_err("error response");
            assert_eq!(kind(&err.error), expected);
            let detail = err.detail.as_ref().expect("detail");
            assert_eq!(detail.kind, KIND_RPC_ERROR);
            assert_eq!(detail.code, Some(code));
            assert_eq!(
                detail.upstream.as_ref().map(UpstreamText::as_str),
                Some(upstream.as_str())
            );
            let shown = format!("{err} {err:?}");
            assert!(
                !shown.contains(MARKER)
                    && !shown.contains("sk-proj")
                    && !shown.contains(ACCOUNT_ID),
                "{shown}"
            );
            assert!(shown.contains(KIND_RPC_ERROR), "{shown}");
            assert!(
                spawner.stdin_closed(),
                "a refusal is an answer too — it gets the grace period"
            );
            assert!(spawner.child_dropped());
            if id == 1 {
                assert_eq!(
                    spawner.written().len(),
                    1,
                    "if the handshake fails, the read request is not sent"
                );
            }
        }
    }

    // ── Conversation ──

    #[test]
    fn the_dialogue_is_initialize_then_initialized_then_one_read() {
        let script = ChildScript::new().line(init_response()).reply(|written| {
            assert_eq!(
                written.len(),
                3,
                "lines written before the read response = two handshake lines + the read: {written:?}"
            );
            read_response(typical_result())
        });
        let (result, spawner) = run(script);
        result.expect("query");
        let written = spawner.written();
        let lines: Vec<Value> = written
            .iter()
            .map(|line| {
                assert!(
                    !line.ends_with('\n'),
                    "the trailing newline goes out twice: {line:?}"
                );
                serde_json::from_str(line).expect("JSON")
            })
            .collect();
        assert_eq!(
            lines,
            [
                json!({
                    "id": 1,
                    "method": "initialize",
                    "params": { "clientInfo": { "name": "engram-dashboard", "version": env!("CARGO_PKG_VERSION") } },
                }),
                json!({ "method": "initialized" }),
                json!({ "id": 2, "method": "account/rateLimits/read", "params": {} }),
            ]
        );
    }

    #[test]
    fn server_requests_are_refused_and_the_dialogue_continues() {
        let script = ChildScript::new()
            .line(r#"{"id":7,"method":"item/tool/requestUserInput","params":{}}"#)
            .line(init_response())
            .line(r#"{"id":"srv-1","method":"item/commandExecution/requestApproval","params":{"x":1}}"#)
            .line(read_response(typical_result()));
        let (result, spawner) = run(script);
        assert_eq!(result.expect("query").plan.as_deref(), Some("plus"));
        let written: Vec<Value> = spawner
            .written()
            .iter()
            .map(|line| serde_json::from_str(line).expect("JSON"))
            .collect();
        assert_eq!(written.len(), 5, "{written:?}");
        let refusal = |id: Value| json!({ "id": id, "error": { "code": METHOD_NOT_FOUND, "message": REFUSAL_MESSAGE } });
        assert_eq!(
            written[1],
            refusal(json!(7)),
            "carries the received id as is"
        );
        assert_eq!(written[2]["method"], "initialized");
        assert_eq!(written[3]["method"], "account/rateLimits/read");
        assert_eq!(written[4], refusal(json!("srv-1")));
    }

    #[test]
    fn notifications_other_ids_and_junk_lines_are_ignored() {
        let script = ChildScript::new()
            .line(r#"{"method":"account/rateLimits/updated","params":{"rateLimits":{"limitId":"codex"}}}"#)
            .line(read_response(json!({ "rateLimits": model_bucket() })))
            .line("Warning: not json")
            .line("[1,2,3]")
            .line(r#"{"id":9,"result":{},"error":{"code":1,"message":"x"}}"#)
            .line(init_response())
            .line(init_response())
            .line(r#"{"id":"2","result":{"rateLimits":{"limitId":"base_model_inference"}}}"#)
            .line(r#"{"id":3,"error":{"code":-1,"message":"not logged in"}}"#)
            .line(r#"{"method":"thread/started","params":{},"emittedAtMs":1}"#)
            .line(read_response(typical_result()));
        let (result, _) = run(script);
        let obs = result.expect("query");
        assert_eq!(obs.weekly, window(Some(58.0), Some(WEEKLY_RESET)));
    }

    #[test]
    fn a_broken_reply_to_our_request_is_a_parse_error_without_waiting() {
        for broken in [
            r#"{"id":2,"result":{},"error":{"code":1,"message":"x"}}"#,
            r#"{"id":2,"error":"just text"}"#,
            r#"{"id":2,"error":{"message":"no code"}}"#,
        ] {
            let root = TempRoot::new("codex-probe-broken");
            let spawner = ScriptedSpawner::new(
                ChildScript::new()
                    .line(init_response())
                    .line(broken)
                    .hang_after_lines(),
            );
            let result = run_with_deadline(&spawner, &root, SHORT);
            assert!(is_bare_parse(&result), "{broken}: {result:?}");
            assert!(
                !spawner.cut_at_deadline(),
                "{broken}: waited until the deadline"
            );
            assert_scratch_gone(&root);
        }
    }

    /// A child that emits a read error first — the real spawner's 「a single line over the cap」 shape (a scripted stand-in cannot emit an
    ///   error line).
    struct IoFirstSpawner {
        then_answer: bool,
    }

    struct IoFirstChild {
        lines: VecDeque<String>,
        io_sent: bool,
    }

    impl ProbeSpawner for IoFirstSpawner {
        fn spawn(
            &self,
            _cmd: &ProbeCommand,
            _deadline: Instant,
        ) -> Result<Box<dyn ProbeChild>, ProbeError> {
            let lines = if self.then_answer {
                VecDeque::from([init_response(), read_response(typical_result())])
            } else {
                VecDeque::new()
            };
            Ok(Box::new(IoFirstChild {
                lines,
                io_sent: false,
            }))
        }
    }

    impl ProbeChild for IoFirstChild {
        fn write_line(&mut self, _line: &str, _deadline: Instant) -> Result<(), ProbeError> {
            Ok(())
        }

        fn close_stdin(&mut self) {}

        fn read_line(&mut self, _deadline: Instant) -> Result<Option<String>, ProbeError> {
            if !self.io_sent {
                self.io_sent = true;
                return Err(ProbeError::Io("a single line exceeded the cap".to_owned()));
            }
            Ok(self.lines.pop_front())
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
            let root = TempRoot::new("codex-probe-io");
            let spawner = IoFirstSpawner { then_answer };
            let env = ProbeEnv {
                spawner: &spawner,
                deadline: Instant::now() + ROOMY,
                scratch_root: root.path(),
            };
            let result = CODEX_USAGE_PROBE.query(&env);
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
        for script in [
            ChildScript::new().hang_after_lines(),
            ChildScript::new().line(init_response()).hang_after_lines(),
        ] {
            let root = TempRoot::new("codex-probe-silent");
            let spawner = ScriptedSpawner::new(script);
            let started = Instant::now();
            let result = run_with_deadline(&spawner, &root, SHORT);
            assert_eq!(result, Err(ProbeError::Timeout.into()));
            assert!(started.elapsed() >= SHORT, "returned before the deadline");
            assert!(spawner.cut_at_deadline());
            assert!(spawner.child_dropped());
            assert_scratch_gone(&root);
        }
    }

    #[test]
    fn a_child_that_never_reads_stdin_times_out() {
        let root = TempRoot::new("codex-probe-blocked");
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

        let stderr = format!("Error: account {ACCOUNT_ID} is not allowed");
        let (result, _) = run(ChildScript::new()
            .line(init_response())
            .exit_code(1)
            .stderr_tail(&[stderr.as_str()]));
        match result {
            Err(ProbeFailure {
                error: ProbeError::Upstream(text),
                detail: None,
            }) => {
                assert!(text.contains("exit code 1"), "{text}");
                assert!(!text.contains(ACCOUNT_ID), "carried stderr: {text}");
            }
            other => panic!("{other:?}"),
        }

        let root = TempRoot::new("codex-probe-linger");
        let spawner = ScriptedSpawner::new(ChildScript::new().never_exit());
        assert_eq!(
            run_with_deadline(&spawner, &root, SHORT),
            Err(ProbeError::Timeout.into())
        );
        assert!(spawner.cut_at_deadline());
        assert_scratch_gone(&root);
    }

    /// ★A failed first write does not hide 「not installed」★ — even if the shell already ended and the handshake write fails, we tell apart
    ///   by the exit.
    #[test]
    fn a_failed_handshake_write_still_reads_the_exit() {
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

    #[test]
    fn a_spawn_failure_passes_through_and_cleans_up() {
        let root = TempRoot::new("codex-probe-missing");
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
    fn the_command_is_the_app_server_in_its_own_scratch_dir_with_the_daemon_env() {
        let root = TempRoot::new("codex-probe-command");
        let seen_dir: Arc<Mutex<Option<PathBuf>>> = Arc::default();
        let seen = seen_dir.clone();
        let scan_root = root.path().to_path_buf();
        let spawner =
            ScriptedSpawner::new(ChildScript::new().line(init_response()).reply(move |_| {
                let dir = fs::read_dir(&scan_root)
                    .expect("root")
                    .flatten()
                    .map(|e| e.path())
                    .next()
                    .expect("temp folder");
                *seen.lock().expect("lock") = Some(dir);
                read_response(typical_result())
            }));
        run_with_deadline(&spawner, &root, ROOMY).expect("query");

        let commands = spawner.commands();
        let [cmd] = commands.as_slice() else {
            panic!("one child: {commands:?}")
        };
        let dir = seen_dir
            .lock()
            .expect("lock")
            .take()
            .expect("folder during the conversation");
        assert_eq!(
            cmd.cwd, dir,
            "working folder = the temp folder of that query"
        );
        assert!(cmd.cwd.starts_with(root.path()));
        assert!(
            !cmd.cwd.exists(),
            "the temp folder remained after the query"
        );

        let (program, args) =
            console_command("codex", vec!["app-server".to_owned(), "--stdio".to_owned()]);
        assert_eq!(cmd.program, program);
        assert_eq!(cmd.args, args);
        assert!(
            cmd.env_set.is_empty(),
            "env is inherited from the daemon as is"
        );
        assert!(
            cmd.env_remove.is_empty(),
            "env is inherited from the daemon as is"
        );
    }

    // ── Account identifier ──

    /// ★The response's `accountId` is not visible in the observation or in the text of any failure★.
    #[test]
    fn the_account_id_never_reaches_an_observation_or_an_error() {
        let obs = observe(typical_result()).expect("observation");
        assert!(!format!("{obs:?}").contains(ACCOUNT_ID), "{obs:?}");

        let failing = [
            json!({ "accountId": ACCOUNT_ID, "rateLimits": model_bucket() }),
            json!({ "accountId": ACCOUNT_ID, "rateLimits": [ACCOUNT_ID] }),
            json!({ "accountId": ACCOUNT_ID }),
            json!([ACCOUNT_ID]),
            json!(ACCOUNT_ID),
        ];
        for result in failing {
            let err = observe(result.clone()).expect_err("failure");
            let shown = format!("{err} {err:?}");
            assert!(!shown.contains(ACCOUNT_ID), "{result}: {shown}");
            assert_eq!(
                err.detail, None,
                "does not carry the response in the raw-text field: {result}"
            );
        }

        // Across the whole conversation too — even if it arrives in any field outside the display fields put into the observation (plan ·
        //   bucket name).
        let mut result = typical_result();
        result["rateLimits"]["accountId"] = json!(ACCOUNT_ID);
        result["rateLimitsByLimitId"]["codex"]["accountId"] = json!(ACCOUNT_ID);
        result["rateLimitsByLimitId"]["codex"]["normalModelSlug"] = json!(ACCOUNT_ID);
        let (got, _) = run(answering(result));
        let obs = got.expect("query");
        assert!(!format!("{obs:?}").contains(ACCOUNT_ID), "{obs:?}");
    }

    // ── No panic ──

    /// No panic whatever arrives in whatever spot — the result is an observation or `Parse`.
    #[test]
    fn hostile_results_never_panic() {
        let weird = [
            json!(null),
            json!(true),
            json!(0),
            json!(-1),
            json!(1e308),
            json!(-1e308),
            json!(18_446_744_073_709_551_615_u64),
            json!(i64::MIN),
            json!(0.5),
            json!(""),
            json!("x".repeat(5_000)),
            json!("\u{0}\u{7f}\u{202e}"),
            json!([]),
            json!([1, 2, 3]),
            json!({}),
            json!({ "a": { "b": [] } }),
        ];
        let base = typical_result();
        let paths: [&[&str]; 13] = [
            &[],
            &["rateLimits"],
            &["rateLimitsByLimitId"],
            &["rateLimitsByLimitId", "codex"],
            &["rateLimitsByLimitId", "base_model_inference"],
            &["rateLimitsByLimitId", "codex", "primary"],
            &["rateLimitsByLimitId", "codex", "limitId"],
            &["rateLimitsByLimitId", "codex", "planType"],
            &["rateLimitsByLimitId", "codex", "primary", "usedPercent"],
            &[
                "rateLimitsByLimitId",
                "codex",
                "primary",
                "windowDurationMins",
            ],
            &["rateLimitsByLimitId", "codex", "secondary", "resetsAt"],
            &["rateLimitsByLimitId", "base_model_inference", "limitName"],
            &[
                "rateLimitsByLimitId",
                "base_model_inference",
                "primary",
                "usedPercent",
            ],
        ];
        for path in paths {
            for value in &weird {
                let mut result = base.clone();
                match path.split_last() {
                    None => result = value.clone(),
                    Some((last, parents)) => {
                        let mut at = &mut result;
                        for key in parents {
                            at = &mut at[*key];
                        }
                        at[*last] = value.clone();
                    }
                }
                match observe(result) {
                    Ok(obs) => {
                        for w in [obs.five_hour, obs.weekly].into_iter().flatten() {
                            assert!(
                                w.used_pct.is_none_or(|p| (0.0..=100.0).contains(&p)),
                                "{path:?}: {w:?}"
                            );
                        }
                    }
                    Err(ProbeFailure {
                        error: ProbeError::Parse(_),
                        detail: None,
                    }) => {}
                    Err(other) => panic!("{path:?} {value}: {other:?}"),
                }
            }
        }

        let mut script = ChildScript::new();
        for junk in [
            "",
            "{",
            "null",
            r#"{"id":null,"result":{}}"#,
            r#"{"id":{"n":2},"result":{}}"#,
            r#"{"id":2.0,"result":{}}"#,
            r#"{"method":7,"id":2}"#,
            r#"{"method":"x","id":[2]}"#,
            "\u{feff}{\"id\":1}",
        ] {
            script = script.line(junk);
        }
        let (result, _) = run(script
            .line(init_response())
            .line(read_response(typical_result())));
        assert!(result.is_ok(), "{result:?}");
    }
}
