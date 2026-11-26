//! `/control/agent` entrance (ADR-0132 decision 6) — an adapter delivering CLI verbs to the command table (ADR-0155).
//!
//! Takes the `{verb, …}` the CLI (`engram agent …`) POSTed and hands `agent.<verb>` to `commands::call_daemon_command` (the same function as bus delivery). Verb bodies and
//! checks are owned by agent (the declaration's owner) and that common entrance; this file adds only (a) `{verb}` → command name (b) recovery guidance in this surface's vocabulary
//! (c) table outcome → wire envelope. The 503 for an unplugged table is the HTTP adapter's job (`mcp_server`).
//!
//! This route is open to everyone (ADR-0132 decision 5): a spawned agent gets the `engram` wiring regardless of backend, and that wiring lays down the mail CLI too (one
//! executable), so mail is refused by the daemon by credential (ADR-0133 — the only enforcement point is the auth middleware in `mcp_server.rs`). Do not put a mail-axis check on this route again.
//! The absence of `kill`·`rm` is not unimplemented — `CLI_AGENT_VERBS` comment is the source (ADR-0122 unresolved).
//!
//! Input discipline has two layers (ADR-0157):
//!   - Unknown fields and missing required fields are refused by the common entrance (`call_daemon_command`). What a person or LLM just typed arrives here, so "unknown field = typo" is safe;
//!     a silent `parnet` typo turns `move … --parent lead` into detaching to root. The list is derived from the declaration (`CommandTable::check_args`), not kept by hand — a copy
//!     would diverge with every added verb and silently let things through.
//!   - Blank values and the absent/`null` distinction are looked at by the table side (agent `commands`): which fields differ per verb, and the decision must live with the verb body.
//!
//! The target token is not trimmed: this family promises exact match (`"worker "` is not `worker`). The rule lives only in agent (`agent::commands::resolve_in`); this entrance
//! hands the token over untouched. Rule match with the mail entrance is guarded by the cross-check in `tests/control_agent.rs`.
// ADR-0155
// ADR-0157

use engram_dashboard_agent::commands::LAUNCH_REFUSAL;
use engram_dashboard_agent::types::{CLI_EXE_NAME, CLI_GROUP_AGENT};
use engram_dashboard_command::{CommandError, ErrorCode};

use crate::connection_core::INPUT_LOCKED_REFUSAL;

use super::ingress::ControlQueryResult;

/// The outlet (port) that tells every attached client the roster changed.
///
/// A port: the real notification pushes `ProfileListUpdated` · `AgentListUpdated` by all-connection fanout (why both: `connection_core::RosterFanout` doc), assembled by `connection_core`.
/// The consumer here owns only a narrow trait and the assembly gives the real adapter (ADR-0110 port discipline; ADR-0236 dropped ADR-0130's leaf property that first set this up).
/// Without it, an agent renaming itself or launching a sibling leaves the dashboard tree on the old roster until an unrelated event (silent stale).
/// The caller is the table: the verb that changed an entry is responsible through notification (agent `commands::RosterChanged`); this entrance makes none.
/// Call count is decided by the verb, not "one verb, one notification": the criterion is whether what the profile list carries changed. Activation changes the entry's
/// "last failure" (ADR-0172), so wake-up notifies once and create+launch twice (after registration and after activation — current even on an in-between failure).
/// Do not revive the old "wake-up 0 times" split on the basis of this doc.
// ADR-0172
// ADR-0132
// ADR-0130
pub trait RosterBroadcast: Send + Sync {
    /// Must be non-blocking — the fanout only does try_send to per-connection queues.
    fn roster_changed(&self);
}

/// `/control/agent` request body — only `verb` is this entrance's vocabulary; the rest is all command arguments.
///
/// Arguments are not taken as a type (load-bearing): the declaration (agent) holds the argument contract, and a second struct would be a second vocabulary that silently drops a field
/// the declaration gains. Sent raw, the unknown-field decision (ADR-0157) and value interpretation happen in the declaration's one place.
/// A class of deserialization failures remains (honest scope): a wrong `verb` type (`{"verb": 5}`), duplicate keys, a non-object body, broken JSON. So they too are not an empty 400, the adapter deserializes
/// directly and carries serde's reason in the envelope (`mcp_server::control_agent_handler`); the only empty-body cases are auth failure and request size overrun.
#[derive(Debug, Default, serde::Deserialize)]
pub struct AgentRequest {
    /// Required yet `default`: failing deserialization would give an empty 400 that does not say what to fix; an empty string lets dispatch give an "unknown verb" bounce and the caller can go to help.
    #[serde(default)]
    pub verb: String,
    #[serde(flatten)]
    pub args: CommandArgs,
}

/// All command argument fields — a key carried twice is bounced.
///
/// Last-value-wins would turn one typo into a different action: in `{"parent":"lead","parent":null}` a request meant to attach silently becomes detaching to root — the form ADR-0157 blocks.
/// JSON does not define duplicate-key meaning, so any pick is baseless, and not silently making a baseless choice is this entrance's discipline.
/// `serde_json::Map` cannot be used as is: its deserialization folds duplicates into overwrites.
/// Only the top level is looked at: objects inside an argument object (`{"target":{"parent":1,"parent":2}}`) are swallowed by `serde_json::Value` and the last value wins. Declared `agent.*`
/// arguments are all strings today, so that dies at the type check, but a command declaring an object value would bring the same accident one level down — then this must go depth-first
/// (the entrance check has the same depth limit — `CommandTable::check_args`).
#[derive(Debug, Default)]
pub struct CommandArgs(serde_json::Map<String, serde_json::Value>);

impl CommandArgs {
    /// The neighboring `catalog` full-name call uses the same duplicate-key discipline; look at them together.
    pub(super) fn into_value(self) -> serde_json::Value {
        serde_json::Value::Object(self.0)
    }
}

impl std::ops::Deref for CommandArgs {
    type Target = serde_json::Map<String, serde_json::Value>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'de> serde::Deserialize<'de> for CommandArgs {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = CommandArgs;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON object of command arguments")
            }

            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<CommandArgs, M::Error> {
                let mut args = serde_json::Map::new();
                while let Some((key, value)) = map.next_entry::<String, serde_json::Value>()? {
                    if args.insert(key.clone(), value).is_some() {
                        // The wording matches serde's duplicate field bounce, so a `verb` duplicate and an argument duplicate do not look like different accidents.
                        // The key is carried short: this wording leads the envelope's `hint` and the recovery guidance follows; the whole key would push it to the far end, unreadable for a person or an LLM.
                        return Err(serde::de::Error::custom(format!(
                            "duplicate field `{}`",
                            preview(&key)
                        )));
                    }
                }
                Ok(CommandArgs(args))
            }
        }

        de.deserialize_map(Visitor)
    }
}

/// The common handler of control verbs — the only entry point the HTTP adapter calls.
///
/// Blocking (caller contract): `agent.*` handlers poll ~3 s for an early exit in resume mode, and rename/hierarchy moves save to disk under the profile lock (agent `make_table` doc);
/// call on the blocking pool (the adapter `mcp_server::control_agent_handler` uses `spawn_blocking`).
/// Roster notification is not done here — the port plugged in at table assembly calls it per verb ([`RosterBroadcast`]).
pub fn handle_agent(table: &super::commands::DaemonTable, req: AgentRequest) -> ControlQueryResult {
    // The CLI surface and the catalog name differ by one dot↔space (TRD §2-1), so no hand-kept per-verb table.
    let name = format!("{CLI_GROUP_AGENT}.{}", req.verb);
    let mut args = req.args.into_value();

    // Checking and execution go through the neighboring `commands` common entrance: calling the table directly would give a different check than bus delivery, keeping ADR-0157 on only one entrance.
    // No connection, so the caller is `None` = not an input lease holder (ADR-0231).
    match super::commands::call_daemon_command(table, &name, &mut args, "cli", None) {
        None => unknown_verb(&req.verb),
        Some(Ok(payload)) => ControlQueryResult::Ok(payload),
        Some(Err(e)) => refused(e),
    }
}

/// The table's failure → wire envelope.
///
/// The code goes out as the typed vocabulary (TRD §4-⑦): per-entrance domain strings would name the same failure differently per surface and tie machine branching to those names.
/// The self-correction path is attached by the adapter: the table and tool crate do not know the calling surface (no CLI vocabulary in the wording); without it the LLM retries with the same arguments.
/// One exception, the input lease refusal (`CONFLICT` but not a target problem): the roster fixes nothing and the next step (again after release) is in the refusal wording itself
/// (connectionless tail of `commands::admit_input`); roster guidance would send the caller after a nonexistent namesake.
/// A launch refusal (`agent.spawn` blocked or raced — [`LAUNCH_REFUSAL`]) is the same kind of `CONFLICT`: the fix is
///   in the listed library items, or a plain retry for a race, and the message already says which. Keyed on the
///   message prefix on purpose — a deliberate exception to "never branch on the message" (command `error.rs`):
///   a race carries no `args`, and the prefix is a constant our own table writes before any caller text.
// ADR-0231
fn refused(e: CommandError) -> ControlQueryResult {
    let carries_its_next_step = e.message().starts_with(INPUT_LOCKED_REFUSAL)
        || (e.code() == ErrorCode::Conflict && e.message().starts_with(LAUNCH_REFUSAL));
    let hint = if carries_its_next_step {
        format!("{}.", e.message())
    } else {
        format!("{} — {}.", e.message(), recovery_for(e.code()))
    };
    ControlQueryResult::Error {
        code: e.code().as_str(),
        hint,
        args: e.args().cloned(),
    }
}

/// The next thing to do differs per code.
///
/// One wording for everything is a lie: "look at the argument spec" on a failure with no fixable argument (target missing · internal) sends the caller after a nonexistent typo; no path at all stops self-correction.
/// What is attached is always one command that can be typed — a sentence alone makes the LLM type that sentence as a command.
fn recovery_for(code: ErrorCode) -> String {
    match code {
        // What can be carried is the answer.
        ErrorCode::InvalidArgument | ErrorCode::UnknownCommand => {
            format!("run `{CLI_EXE_NAME} help {CLI_GROUP_AGENT}` for this group's verbs and fields")
        }
        // Not an argument syntax problem — the target is chosen by what is in the roster now (missing · same name · cap).
        ErrorCode::NotFound | ErrorCode::Conflict => {
            format!("run `{CLI_EXE_NAME} {CLI_GROUP_AGENT} {AGENT_LIST_VERB}` to see the roster")
        }
        // No argument to fix — the next step is to check what actually remains (e.g. something created but not launched).
        _ => format!(
            "run `{CLI_EXE_NAME} {CLI_GROUP_AGENT} {AGENT_LIST_VERB}` to see what actually exists now"
        ),
    }
}

/// The query verb the recovery guidance suggests. Which verb is a choice; that it exists is not — the test below checks it against the family verb list (else the guidance makes them type a nonexistent command).
const AGENT_LIST_VERB: &str = "list";

fn unknown_verb(verb: &str) -> ControlQueryResult {
    ControlQueryResult::Error {
        code: ErrorCode::UnknownCommand.as_str(),
        hint: format!(
            "unknown verb '{verb}' — {}.",
            recovery_for(ErrorCode::UnknownCommand)
        ),
        args: None,
    }
}

/// The part (in characters) **one fragment** of the wording may take.
const PREVIEW_CHARS: usize = 80;

/// Shrinks a caller-given string to one fragment of the wording — when it cuts, it says so.
///
/// Guards only its own fragment: the caller string must not eat the whole wording, so the recovery guidance after it stays inside; unsaid cutting would make the caller fix a place that is fine.
/// There is no cap on the whole bounce wording (a known open condition): production points carrying caller strings without this function exist on this and neighboring routes and go out at full length.
/// Do not write "every wording gets cut somewhere" here — it was false four versions in a row.
///
/// Do not use in log fields — this looks only at length, not shape. Control characters are not escaped, so one name with a newline splits a log line and the later line looks like a
/// printed entry (a string mimicking timestamp · level fits in 80 characters). Adding escaping here is not the answer: the output is wording returned to the caller, which must look verbatim.
/// So the split is by destination (user-facing wording = here; log fields = `connection_core::sanitize_for_log`); `command_delivery` already uses that side (first line of `deliver`).
/// It broke once: a bounce log before validation in `/control/call` carried an arbitrary string as is.
/// One opposite exception, left knowingly: the registration bounce wording uses the log sanitizer though it is a response (`connection_core::ConnectionCore::refuse_names_i_answer`, reasons there) — the
/// receiver is a program whose logs the value flows into, with no verbatim comparison axis. So the axis is not "response or log" but "is a value a person typed shown back to a person".
pub(super) fn preview(text: &str) -> String {
    preview_within(text, PREVIEW_CHARS)
}

/// Shrinks the middle, keeping both ends — for carrying one whole wording, not a fragment.
///
/// [`preview`] (head only) must not be used here: the table's bounce puts what the caller typed first and the next thing to do (all declared fields) last (`CommandTable::check_args`); head-only cuts exactly
/// that, so the caller hears its field is wrong and never sees a correct one.
/// A cap is still needed because not every wording caps its own input (`commands::drive_to_completion` puts the caller's command name in verbatim). Says so when it cuts, like the sibling.
pub(super) fn preview_within(text: &str, chars: usize) -> String {
    let total = text.chars().count();
    if total <= chars {
        return text.to_string();
    }
    let head_chars = chars / 2;
    let head: String = text.chars().take(head_chars).collect();
    let tail: String = text.chars().skip(total - (chars - head_chars)).collect();
    format!("{head}…(truncated, {} bytes)…{tail}", text.len())
}

/// The bounce when the body failed the JSON contract — called by the adapter (`control_agent_handler`).
///
/// No empty-body 400: the callers are self-correcting LLMs, so serde's wording (field type · duplicate keys · syntax position) is carried as is.
/// The raw body is shrunk with [`preview`]: returning the whole body is not the entrance's job, and eating the front would push the guidance to the far end.
/// `reason` is not cut here (known open condition): serde wording is usually short but can grow with caller strings and then goes out in full; cutting an already short wording would cut at the boundary and hide the problem.
pub fn malformed_body(reason: &str, raw: &str) -> ControlQueryResult {
    ControlQueryResult::Error {
        code: ErrorCode::InvalidArgument.as_str(),
        hint: format!(
            "the request body is not a valid {CLI_GROUP_AGENT} command object: {reason} — it must be a JSON object like {{\"verb\":\"list\"}}; got: {} — {}.",
            preview(raw),
            recovery_for(ErrorCode::InvalidArgument)
        ),
        args: None,
    }
}

#[cfg(test)]
mod tests {
    use engram_dashboard_agent::types::CLI_AGENT_VERBS;

    use super::*;

    fn parse(body: serde_json::Value) -> AgentRequest {
        serde_json::from_value(body).expect("request deserialization")
    }

    fn error_of(result: ControlQueryResult) -> (&'static str, String) {
        match result {
            ControlQueryResult::Error { code, hint, .. } => (code, hint),
            other => panic!("must be a bounce: {other:?}"),
        }
    }

    // ── request → command ──────────────────────────────────────────────────────────────

    /// Only `verb` is this entrance's vocabulary — the rest, known or unknown, is all carried as arguments for the table; picking only known fields would make unknown ones evaporate and break ADR-0157's refusal.
    #[test]
    fn every_field_but_the_verb_is_carried_through_as_command_arguments() {
        let req = parse(serde_json::json!({
            "verb": "move", "target": "a", "parent": null, "parnet": "lead"
        }));
        assert_eq!(req.verb, "move");
        assert_eq!(
            *req.args,
            *serde_json::json!({ "target": "a", "parent": null, "parnet": "lead" })
                .as_object()
                .expect("object")
        );
    }

    /// A field carried twice picks neither value: in `{"parent":"lead","parent":null}` the later value would turn attach into detaching to root. Same discipline for `verb` and argument fields — blocking one side lets the accident in through the other.
    #[test]
    fn a_duplicate_key_is_refused_for_the_verb_and_for_arguments_alike() {
        for body in [
            r#"{"verb":"list","verb":"move"}"#,
            r#"{"verb":"move","target":"a","target":"b"}"#,
            r#"{"verb":"move","target":"a","parent":"lead","parent":null}"#,
        ] {
            let refusal = serde_json::from_str::<AgentRequest>(body).map(|_| ());
            assert!(refusal.is_err(), "a duplicate key must be bounced: {body}");
        }
        // Several fields with different names are normal (the duplicate decision does not spread to "bounce if there are several fields").
        let ok = parse(serde_json::json!({ "verb": "move", "target": "a", "parent": null }));
        assert_eq!(ok.args.len(), 2);
    }

    /// The bounce wording does not grow with the request — carrying the key as is would make a 500 KB key produce a 200 response that big, paid by the receiver. The cut must be said too.
    #[test]
    fn a_huge_duplicate_key_is_reported_within_a_bound() {
        let huge = "k".repeat(500_000);
        let body = format!(r#"{{"verb":"move","{huge}":1,"{huge}":2}}"#);

        let reason = serde_json::from_str::<AgentRequest>(&body)
            .expect_err("duplicate key")
            .to_string();
        let (_, hint) = error_of(malformed_body(&reason, &body));

        assert!(hint.contains("duplicate field"), "{hint}");
        assert!(hint.contains("truncated"), "says it cut: {hint}");
        // Measured ~434 B — the number below is slack, not a contract (raising `PREVIEW_CHARS` to 250 still passes); the contract is independence from request size, established by the per-fragment [`preview`].
        assert!(
            hint.len() < 1024,
            "the wording grew with the request: {} bytes",
            hint.len()
        );
    }

    /// The depth the duplicate decision reaches is only the top level (known boundary): duplicates in a nested object are swallowed by `serde_json::Value`, last value wins. Declared arguments are strings today (dies at the type check); a command declaring an object value would bring the accident back one level down.
    #[test]
    fn duplicate_detection_is_top_level_only() {
        let nested = parse(serde_json::json!({ "verb": "move", "target": "a" }));
        assert_eq!(
            nested.args.len(),
            1,
            "premise: the top-level field is carried as is"
        );

        let req: AgentRequest =
            serde_json::from_str(r#"{"verb":"move","target":{"parent":1,"parent":2}}"#)
                .expect("a nested duplicate is not caught here (a known boundary)");
        assert_eq!(req.args["target"]["parent"], 2);
    }

    // ── bounce envelope ────────────────────────────────────────────────────────────────

    /// A bounce always goes out with one command that can be typed — the table and tool crate do not know this surface, so the guidance is attached only here; a sentence alone is typed as a command by the LLM.
    #[test]
    fn every_rejection_carries_the_typed_code_and_a_runnable_command() {
        let cases = [
            CommandError::invalid_argument("bad field"),
            CommandError::not_found("no agent called 'ghost'"),
            CommandError::of(ErrorCode::Conflict, "more than one agent"),
            CommandError::internal("was created but did not start"),
        ];
        for error in cases {
            let (code, hint) = error_of(refused(error.clone()));
            assert_eq!(code, error.code().as_str());
            assert!(hint.contains(error.message()), "{hint}");
            assert!(
                hint.contains(&format!("`{CLI_EXE_NAME} ")),
                "there is no command that can be typed: {hint}"
            );
        }
    }

    /// The code's arguments ride to the `engram agent …` body untouched; the hint keeps its recovery line.
    #[test]
    fn a_rejection_carries_the_codes_args_next_to_the_hint() {
        let args = serde_json::json!({"reasons": [{"code": "missing", "item": "a/b"}]});
        let body = refused(
            CommandError::of(ErrorCode::Conflict, "launch blocked")
                .with_args(args.as_object().expect("object").clone()),
        )
        .into_json();

        assert_eq!(body["code"], "CONFLICT");
        assert_eq!(body["args"], args);
        assert!(
            body["hint"]
                .as_str()
                .is_some_and(|hint| hint.contains(&format!("`{CLI_EXE_NAME} "))),
            "{body}"
        );
    }

    /// The input lease refusal carries no roster guidance — `CONFLICT` but not a wrongly chosen target; roster guidance would send the caller after a nonexistent namesake. The next step is in the refusal wording.
    #[test]
    fn a_lease_refusal_does_not_point_at_the_roster() {
        let message = format!("{INPUT_LOCKED_REFUSAL} — tail");
        let (code, hint) = error_of(refused(CommandError::of(
            ErrorCode::Conflict,
            message.clone(),
        )));
        assert_eq!(code, "CONFLICT");
        assert_eq!(hint, format!("{message}."));
        // A target failure (same name) with the same code still points to the roster.
        let (_, ambiguous) = error_of(refused(CommandError::of(
            ErrorCode::Conflict,
            "more than one agent",
        )));
        assert!(
            ambiguous.contains(&format!("{CLI_GROUP_AGENT} {AGENT_LIST_VERB}")),
            "{ambiguous}"
        );
    }

    /// The next thing to do differs per code — "look at the argument spec" on a failure with no argument to fix sends the caller after a nonexistent typo, and the spec on a failed target pick keeps them from thinking of the roster.
    #[test]
    fn the_suggested_recovery_matches_what_the_caller_can_actually_fix() {
        let help = format!("`{CLI_EXE_NAME} help {CLI_GROUP_AGENT}`");
        let roster = format!("`{CLI_EXE_NAME} {CLI_GROUP_AGENT} {AGENT_LIST_VERB}`");

        for argument_fault in [ErrorCode::InvalidArgument, ErrorCode::UnknownCommand] {
            assert!(
                recovery_for(argument_fault).contains(&help),
                "{argument_fault:?}"
            );
        }
        for targeting_fault in [ErrorCode::NotFound, ErrorCode::Conflict] {
            assert!(
                recovery_for(targeting_fault).contains(&roster),
                "{targeting_fault:?}"
            );
        }
        // A failure with no argument to fix (internal · unknown) has a next step too — look at what actually remains.
        for opaque_fault in [ErrorCode::Internal, ErrorCode::OutcomeUnknown] {
            assert!(
                recovery_for(opaque_fault).contains(&roster),
                "{opaque_fault:?}"
            );
        }
    }

    /// The suggested verb must exist — guiding to a nonexistent verb turns self-correction into a bounce.
    #[test]
    fn the_suggested_roster_verb_is_one_this_group_actually_has() {
        assert!(
            CLI_AGENT_VERBS.contains(&AGENT_LIST_VERB),
            "{AGENT_LIST_VERB} is not in the family verb list: {CLI_AGENT_VERBS:?}"
        );
    }

    #[test]
    fn an_unknown_verb_names_the_verb_and_points_at_the_group_help() {
        let (code, hint) = error_of(unknown_verb("explode"));
        assert_eq!(code, "UNKNOWN_COMMAND");
        assert!(hint.contains("explode"), "{hint}");
        assert!(hint.contains("help"), "{hint}");
    }
}
