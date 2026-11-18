//! The two control entrances that do not distinguish families — discovery (`/control/commands`) and full-name call (`/control/call`).
//!
//! Unlike `agent.rs` (the `{verb}` vocabulary of one family), this takes the name as is: no verb table, no family knowledge; checking and
//! execution go through the stage-1 port to `commands::call_daemon_command` (why that port: the [`handle_call`] paragraph). This file adds
//! (a) the merge rule folding two sources into one list, (b) recovery guidance in this surface's vocabulary, (c) table outcome → wire envelope.
//!
//! `help` is not opened (hard constraint): merge and list carry it byte for byte — no parsing, validation or branching (ADR-0156).
//!
//! The list's `callable` and the outcome of [`handle_call`] must come from the same fact (the only machine-branchable field); that fact is in [`handle_list`].
//!
//! Someone else's names go to the bus (ADR-0160): a name not in the daemon's own table with an owner attached becomes [`CallOutcome::Relay`]
//! for the caller to deliver — this file is blocking, the round trip is async.
// ADR-0155
// ADR-0156
// ADR-0160

use std::collections::BTreeSet;

use engram_dashboard_command::{
    CommandDecl, CommandError, CommandReply, ErrorCode, OwnerLookup, OwnerLookupSource, RequestId,
    RosterEntry,
};
use engram_dashboard_protocol::CommandListEntry;

use super::agent::{preview, preview_within, CommandArgs};
use super::ingress::ControlQueryResult;
use crate::command_delivery::LocalCommands;

/// The merge rule of the discovery list — this rule lives here only.
///
/// WS `ListCommands` and HTTP `/control/commands` both call it; a copy would make discovery lie to one side.
/// - Combine `mine` (the daemon's own table) and `registered` (attached owners' roster); without its own table `agent.*` would be delivered yet invisible.
/// - On the same name the daemon's wins — delivery is stage 1 own table, stage 2 roster (`command_delivery::deliver`); otherwise the caller would
///   fit arguments with someone else's `help` and be bounced by the daemon handler.
/// - Sort by name once at the end (the concatenation is not ordered).
///
/// The dedup covers a state unreachable today, but do not delete it: an overlap needs a registration arriving while the daemon table is empty
/// (`refuse_names_i_answer` looks at the table then), and the assembled servers fill the slot before accepting connections; a late-plugged assembly
/// is possible (the slot is late injection), and then this is the only net.
/// The return type is the WS projection: its `available` is true only in the WS sense (an owner is attached; a disconnected owner's names leave the
/// roster — ADR-0150 decision 3; field truth: `CommandListEntry`). The HTTP list does not re-emit that field — it says "can this entrance
/// execute it" (one more input, the daemon's own table), and one name with two meanings would mislead a branching caller ([`handle_list`]).
pub fn merge(mine: Vec<CommandDecl>, registered: Vec<RosterEntry>) -> Vec<CommandListEntry> {
    let mine: Vec<CommandListEntry> = mine
        .into_iter()
        .map(|decl| CommandListEntry {
            name: decl.name,
            help: decl.help,
            available: true,
        })
        .collect();
    let mut entries: Vec<CommandListEntry> = registered
        .into_iter()
        .filter(|entry| !mine.iter().any(|held| held.name == entry.name))
        .map(|entry| CommandListEntry {
            name: entry.name,
            help: entry.help,
            available: true,
        })
        .collect();
    entries.extend(mine);
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

/// The discovery response — `{ "commands": [ { "name", "help", "callable" }, … ] }`.
///
/// `callable` = this entrance can execute that name now (contract): own-table names run on the spot, others are relayed to their owner (ADR-0160).
/// It must be the only reachability field — carrying the WS `available` again would give one name two meanings per surface.
/// Inputs are the same two facts as [`handle_call`]: the name is in the daemon's own table (`mine` / `CommandTable::contains`) or the roster
/// answers an owner (`registered` / `OwnerLookupSource::lookup`); split inputs give different answers.
/// Today it is true on every [`merge`] row, but do not fold it into a constant: that is because both sources are reachable, and a future third
/// source (declared names without an owner) would then falsely advertise reachability.
///
/// Arguments are values, not slots, so this rides the same [`merge`] as WS; the read time is the caller's. Half lists are emitted too (an empty
/// table slot gives empty `mine`) — the same answer WS gives in that state; the assembled servers plug the table in before accepting connections.
pub fn handle_list(mine: Vec<CommandDecl>, registered: Vec<RosterEntry>) -> ControlQueryResult {
    // Only names are taken — `help` is not copied (not needed for this decision, and that blob is the largest thing in this list).
    let callable: BTreeSet<String> = mine
        .iter()
        .map(|decl| decl.name.clone())
        .chain(registered.iter().map(|entry| entry.name.clone()))
        .collect();
    // No `json!`: it expands non-literal places with `to_value(&expr)`, duplicating the `help` blob per row — at the roster cap (4096 × 4 KiB) the
    //   largest cost of this response. Hand assembly moves the Strings, and drops the `.unwrap()` `json!` hides (outside the containment mark that
    //   panic is a daemon exit — hook · ADR-0292).
    let commands: Vec<serde_json::Value> = merge(mine, registered)
        .into_iter()
        .map(|entry| {
            let callable = callable.contains(&entry.name);
            let mut row = serde_json::Map::with_capacity(3);
            row.insert("name".to_string(), serde_json::Value::String(entry.name));
            row.insert("help".to_string(), serde_json::Value::String(entry.help));
            row.insert("callable".to_string(), serde_json::Value::Bool(callable));
            serde_json::Value::Object(row)
        })
        .collect();
    let mut root = serde_json::Map::with_capacity(1);
    root.insert("commands".to_string(), serde_json::Value::Array(commands));
    ControlQueryResult::Ok(serde_json::Value::Object(root))
}

/// The `/control/call` request body — `{ "name": "<full name>", "args": { … } }`.
///
/// Arguments go inside `args`: `name` is also a real argument name (`agent.rename`), so spreading at the top (as `/control/agent` does with
/// only `verb`) would make that verb uncallable here. `args` absent = no arguments.
/// Duplicate keys are bounced at the top and inside `args` (no basis to pick one — ADR-0157; inner discipline: [`CommandArgs`]).
/// Unknown top-level fields are ignored (additive evolution across hops); a request putting arguments at the top does not pass silently — its
/// required fields are empty, so it is bounced with `INVALID_ARGUMENT` listing all declared fields.
/// `request_id` is optional — the caller's is used, else the daemon issues one (ADR-0161 decision 1). It exists because delivery's duplicate
/// prevention triggers only when the same number comes again; a retry must resend the same one. Old requests without it still work, without even the in-flight guarantee (ADR-0161 Impact).
///
/// ## The guarantee today holds only while the round trip is open (known gap)
///
/// A re-query overlapping an in-progress round trip is bounced by the seat table ([`Seat::Taken`](crate::command_delivery) ·
/// [`Seat::Conflict`](crate::command_delivery)) or merged; one after completion is applied again, because:
/// 1. a relayed name releases its seat as the reply frame goes out ([`CommandDeliveries::release`](crate::command_delivery));
///    [`SeatState::Retained`](crate::command_delivery) attaches only to the daemon's own stage-1 seats;
/// 2. the daemon's own names (`agent.*`) are answered on the spot by [`handle_call`], never reaching the seat table.
///
/// Do not write "a re-query after completion is blocked with `ALREADY_APPLIED`" — false here: there is no dedup store returning completed results
/// (`no_retry` in `command_delivery`), and holding seats after completion is unresolved (awaiting a user decision).
/// Only this HTTP body grows, not the envelope (ADR-0160 "envelope unchanged").
#[derive(Debug, Default)]
pub struct CallRequest {
    /// Required yet absence-tolerant, as in the neighbor: failing deserialization would bounce with no reason; an empty string gives "unknown command" and the caller can go to the discovery list.
    pub name: String,
    pub args: CommandArgs,
    /// The raw text of the caller's request number; `None` = not sent. Not parsed at deserialization: failing there would fold the body into
    /// [`malformed_body`] ("the JSON shape is wrong") when only this one field is wrong.
    pub request_id: Option<String>,
}

impl<'de> serde::Deserialize<'de> for CallRequest {
    /// Objects only — do not revert to `#[derive(Deserialize)]`: the derive also reads a JSON array as a field sequence, so `["agent.list"]`
    /// would execute `agent.list` (measured). The neighbor `AgentRequest` is safe because `#[serde(flatten)]` forces map-only deserialization.
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct Visitor;

        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = CallRequest;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON object like {\"name\":\"agent.list\",\"args\":{}}")
            }

            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<CallRequest, M::Error> {
                let mut name: Option<String> = None;
                let mut args: Option<CommandArgs> = None;
                let mut request_id: Option<String> = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "name" => {
                            if name.is_some() {
                                return Err(serde::de::Error::duplicate_field("name"));
                            }
                            name = Some(map.next_value()?);
                        }
                        "args" => {
                            if args.is_some() {
                                return Err(serde::de::Error::duplicate_field("args"));
                            }
                            args = Some(map.next_value()?);
                        }
                        CALLER_REQUEST_ID_FIELD => {
                            if request_id.is_some() {
                                return Err(serde::de::Error::duplicate_field(
                                    CALLER_REQUEST_ID_FIELD,
                                ));
                            }
                            request_id = Some(map.next_value()?);
                        }
                        _ => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(CallRequest {
                    name: name.unwrap_or_default(),
                    args: args.unwrap_or_default(),
                    request_id,
                })
            }
        }

        de.deserialize_map(Visitor)
    }
}

/// Did the full-name call finish on the spot, or must it be handed to the bus? The handing branch is an async round trip and cannot finish inside
/// this blocking function; as prose, a caller that does not hand over would be representable (dashboard commands silently `UNSUPPORTED` again).
// ADR-0160
#[derive(Debug)]
pub enum CallOutcome {
    /// This daemon answered (executed, bounced, or does not know the name) — goes out as is.
    Answered(ControlQueryResult),
    /// Someone else's name with an owner attached — the caller puts it on the delivery machine. Arguments pass as is: no declaration to compare
    /// against (tolerance is what additive evolution hangs on) and none touched (the miss contract of `call_daemon_command`).
    Relay {
        name: String,
        args: serde_json::Value,
        /// The number of this round trip — the caller's (ADR-0161 decision 1) or one issued here. Issuing is not deferred to the adapter, or one would
        /// have to read the adapter to know whether the validated value is used.
        request_id: RequestId,
    },
}

/// The field name where the caller puts its request number — deserialization and guidance wording must spell it the same.
const CALLER_REQUEST_ID_FIELD: &str = "request_id";

/// The raw text of the caller-given number → [`RequestId`].
///
/// Requiring UUID syntax is the length · charset validation (ADR-0161 decision 5): [`RequestId`] wraps one UUID (tool crate `envelope.rs`) and
/// another syntax would change the envelope's type, blocked by ADR-0160. So no arbitrary-length outside string becomes a seat table key.
/// The accepted shape is not only hyphenated 36 characters: `Uuid::parse_str` also takes simple · braced · urn, all bounded (urn longest, 45);
/// the guidance wording says so — "36 characters" would make the caller suspect a passing value.
/// The content is not checked (nil or non-v4 pass); no place may rely on it (`CommandDeliveries::complete` in `command_delivery`).
/// The bounce wording carries the raw text shrunk with [`preview`].
fn caller_request_id(raw: &str) -> Result<RequestId, ControlQueryResult> {
    uuid::Uuid::parse_str(raw).map(RequestId).map_err(|_| {
        ControlQueryResult::Error {
            code: ErrorCode::InvalidArgument.as_str(),
            hint: format!(
                "'{CALLER_REQUEST_ID_FIELD}' must be a UUID — hyphenated (8-4-4-4-12), 32 hex digits with no dashes, braced, or urn:uuid: form — so a retry can reuse it; got: {}",
                preview(raw)
            ),
        }
    })
}

/// Full-name call — the only entry point the HTTP adapter calls.
///
/// Blocking (caller contract): the table's handlers are all blocking, so call it on the blocking pool.
/// `registered` is read only on the miss branch — the roster lookup touches a shared lock, ending within that call (ADR-0006, why
/// [`OwnerLookupSource`] exists).
/// The owner can disconnect between lookup and delivery; that race cannot be closed here — delivery answers it in its own vocabulary (`route`'s
/// re-lookup · `OwnerLink::send`'s torn window), and carrying the lock into delivery would let one slow command stall registration · cleanup · other deliveries.
/// The caller number is decided before doing anything, or the contract would split by name (bad numbers passing for own names, bounced for others).
///
/// ## The stage-1 table is the one copy the bus holds — do not revert to `&CommandTable`
///
/// This decision (answer / relay) and delivery's ([`LocalCommands::claim`] — the seat's `local` marker and `CommandBus::invoke`'s relay refusal)
/// must be the same fact. A separate table argument would allow an adapter passing a different copy than the bus, with a quiet symptom: relay
/// for "not my name" while the bus bounces it as its own with `INTERNAL`, or a silent no-op refusal leaving a seat forever. As a port, the
/// adapter can only pass that one copy.
/// Checking and execution go through this port too (`commands::DaemonLocalCommands::run`, so the ADR-0157 check applies); do not add a path
/// calling [`CommandTable::call`] directly.
// ADR-0157
// ADR-0160
pub fn handle_call(
    locals: &dyn LocalCommands,
    registered: &dyn OwnerLookupSource,
    req: CallRequest,
) -> CallOutcome {
    let CallRequest {
        name,
        args,
        request_id,
    } = req;
    let mut args = args.into_value();
    let request_id = match request_id.as_deref().map(caller_request_id) {
        Some(Err(refusal)) => return CallOutcome::Answered(refusal),
        Some(Ok(id)) => id,
        None => RequestId::new(),
    };

    // The routing authority is `claim` alone ([`LocalCommands::claim`]): it decides the seat's `local` marker, so another predicate here would diverge from delivery.
    if locals.claim(&name).is_none() {
        return match registered.lookup(&name) {
            OwnerLookup::Available(_) => CallOutcome::Relay {
                name,
                args,
                request_id,
            },
            OwnerLookup::Unknown => CallOutcome::Answered(unknown_name(&name)),
        };
    }
    // No connection, so the caller is `None` = not an input lease holder (ADR-0231).
    CallOutcome::Answered(match locals.run(&name, &mut args, "cli", None) {
        Some(Ok(payload)) => ControlQueryResult::Ok(payload),
        Some(Err(e)) => refused(e),
        // A contract violation does not flow to the next stage: `claim` said mine, so relaying would be refused again by the bus (a meaningless round
        //   trip). Delivery exposes the same mismatch with the same code (`fold_local` in `command_delivery`).
        None => {
            tracing::error!(
                entrance = "cli",
                command = %crate::connection_core::sanitize_for_log(&name),
                "the stage-1 table claimed the name as its own and came up empty-handed — the routing authority (`claim`) and execution diverged"
            );
            refused(CommandError::of(
                ErrorCode::Internal,
                format!(
                    "this daemon claims '{}' but its own table produced no answer for it",
                    preview(&name)
                ),
            ))
        }
    })
}

/// The outcome delivery returned → wire envelope.
///
/// The success payload was made by the owner and is not opened (ADR-0081 "daemon opaque"). Failures ride [`refused`] like the other bounces —
/// split, the same code would carry different wording and guidance per place.
// ADR-0160
pub fn relayed(reply: CommandReply) -> ControlQueryResult {
    match reply.outcome {
        Ok(payload) => ControlQueryResult::Ok(payload),
        Err(e) => refused(e),
    }
}

/// The table's failure → wire envelope; the code goes out as the typed vocabulary (TRD §4-⑦).
///
/// Recovery guidance only on fixable failures: "look at the list" on a missing target · state conflict · internal failure sends the caller after
/// a nonexistent typo. This entrance does not know families, so it cannot suggest a per-family query verb; other codes carry only the table's wording.
/// The wording is shrunk keeping both ends: the argument bounce puts the caller's input first and all declared fields last, so head-only loses
/// the material to fix (layout and cap: [`MESSAGE_PREVIEW_CHARS`]).
fn refused(e: CommandError) -> ControlQueryResult {
    let message = preview_within(e.message(), MESSAGE_PREVIEW_CHARS);
    let hint = match e.code() {
        ErrorCode::InvalidArgument | ErrorCode::UnknownCommand => {
            format!("{message} — {CATALOG_RECOVERY}.")
        }
        _ => message,
    };
    ControlQueryResult::Error {
        code: e.code().as_str(),
        hint,
    }
}

/// The part (in characters) one whole bounce wording the table gave may take.
///
/// It must not cut the designed worst case: the argument check's worst wording is capped by the tool crate (128 B per name via `quoted_input`,
/// eight names `MAX_NAMED_UNKNOWN`, ~1.4 KB measured), and cutting below that loses the middle in the designed case.
/// The cap still exists because not every wording is so capped (`commands::drive_to_completion` puts the caller's command name in verbatim); it
/// bounds that tail, not the average. The number is slack above 1.4 KB, not a contract.
const MESSAGE_PREVIEW_CHARS: usize = 2048;

/// The only recovery path this entrance knows — only the discovery list knows callable names and their fields.
///
/// No made-up CLI verbs: the daemon owns the route, not CLI spelling, and no CLI verb renders this list yet.
/// Do not revert to `engram help`: that screen is static (`control/help.rs`), gives two family names and no field, so names outside `agent.*`
/// cannot appear — it would send exactly the cases this guidance exists for (unknown name · wrong field) somewhere that cannot answer.
/// ADR-0132 decision 4 (discovery only through help) is not the basis here: it chose a static family screen instead of listing commands, the opposite of this runtime list.
/// The path string must not diverge from the route constant (the test below checks).
// ADR-0156 (what carries the fields is the opaque `help` blob, and the surface that hands that blob down is this list)
const CATALOG_RECOVERY: &str =
    "list the command catalog (POST /control/commands) for every callable name and its fields";

/// Nobody declared that name — not in the daemon table and no attached owner.
///
/// Do not revive "someone else's name, unreachable" (`UNSUPPORTED`): that branch goes to delivery ([`CallOutcome::Relay`] · ADR-0160); reviving
/// it would make a name just learned from the list uncallable and the list's reachability field ([`handle_list`]) false.
/// A name whose owner just disconnected comes here too (ADR-0150 decision 3): no material to tell it from a name that never existed — an accepted loss.
fn unknown_name(name: &str) -> ControlQueryResult {
    ControlQueryResult::Error {
        code: ErrorCode::UnknownCommand.as_str(),
        hint: format!(
            "unknown command '{}' — neither this daemon nor any connected client declares that name; {CATALOG_RECOVERY}.",
            preview(name)
        ),
    }
}

/// The bounce when the body failed the JSON contract — called by the adapter.
///
/// No empty-body 400: the callers are self-correcting LLMs, so serde's wording (field type · duplicate keys · syntax position) goes out, with the raw body shrunk by [`preview`].
/// Do not attach [`CATALOG_RECOVERY`]: what is wrong is the body's JSON shape, not the name or fields — pointing at the catalog would make the
/// caller dig through the list while the envelope shape stays broken; this wording already holds the material to fix.
pub fn malformed_body(reason: &str, raw: &str) -> ControlQueryResult {
    ControlQueryResult::Error {
        code: ErrorCode::InvalidArgument.as_str(),
        hint: format!(
            "the request body is not a valid command call: {reason} — it must be a JSON object like {{\"name\":\"agent.list\",\"args\":{{}}}}; got: {}",
            preview(raw)
        ),
    }
}

#[cfg(test)]
mod tests {
    use crate::conn_port::ConnId;
    use engram_dashboard_command::{Effect, OwnerToken, RequestId};

    use super::*;
    use crate::command_delivery::NoLocalCommands;

    /// A stage-1 table answering one name as its own — imitates only the real one's property (`commands::DaemonLocalCommands`): `claim` and `run` look at the same list.
    ///
    /// The real `agent.*` table holds an `AgentManager` (disk · profile locks, `commands::make_daemon_table`); this file measures the decision
    /// (answer / relay / unknown), and pulling it in would shift the test to the assembly (ADR-0012).
    struct Holds(&'static str);

    impl LocalCommands for Holds {
        fn claim(&self, name: &str) -> Option<Effect> {
            (name == self.0).then_some(Effect::Read)
        }

        fn run(
            &self,
            name: &str,
            _args: &mut serde_json::Value,
            _entrance: &'static str,
            _caller: Option<ConnId>,
        ) -> Option<Result<serde_json::Value, CommandError>> {
            (name == self.0).then(|| Ok(serde_json::json!({ "ran": name })))
        }

        fn decls(&self) -> Vec<CommandDecl> {
            vec![decl(self.0, "{}")]
        }

        fn agent_roster(&self) -> Vec<engram_dashboard_agent::commands::AgentRosterRow> {
            Vec::new()
        }
    }

    fn decl(name: &str, help: &str) -> CommandDecl {
        CommandDecl {
            name: name.to_string(),
            help: help.to_string(),
            agent_args: Vec::new(),
        }
    }

    fn entry(name: &str, help: &str) -> RosterEntry {
        RosterEntry {
            name: name.to_string(),
            help: help.to_string(),
            owner: OwnerToken::new("conn-1"),
        }
    }

    fn names(entries: &[CommandListEntry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    /// Where the guidance points must exist and be able to answer: a const cannot be built with format, so the path is hand-written; this exercises the route constant to check.
    #[test]
    fn the_recovery_points_at_the_route_that_can_actually_answer_it() {
        assert!(
            CATALOG_RECOVERY.contains(super::super::mcp_server::CONTROL_COMMANDS_PATH),
            "the recovery guidance diverged from the discovery route: {CATALOG_RECOVERY}"
        );
    }

    #[test]
    fn the_merge_sorts_both_sources_into_one_list_and_the_daemon_wins_a_clash() {
        let merged = merge(
            vec![decl("agent.list", "mine"), decl("agent.new", "mine")],
            vec![entry("tab.create", "theirs"), entry("agent.list", "stolen")],
        );

        assert_eq!(
            names(&merged),
            vec!["agent.list", "agent.new", "tab.create"]
        );
        assert_eq!(
            merged
                .iter()
                .find(|e| e.name == "agent.list")
                .map(|e| e.help.as_str()),
            Some("mine"),
            "on overlap the daemon table's help remains"
        );
    }

    /// `help` is one opaque string field — even a non-JSON value must go out byte for byte.
    #[test]
    fn the_listing_carries_the_help_bytes_untouched() {
        let opaque = "not json at all — arbitrary bytes {[(";
        let result = handle_list(vec![decl("agent.list", opaque)], vec![]);

        let json = result.to_json();
        assert_eq!(json["commands"][0]["name"], "agent.list");
        assert_eq!(json["commands"][0]["help"], opaque);
    }

    /// [`handle_call`] recognizes every name the list taught and does not know names outside it.
    ///
    /// Asserting only `callable == true` would be indistinguishable from asserting a constant (true on every [`merge`] row today); the property is
    /// "list and execution come from the same two facts", seen only by running both on the same input. A negative control keeps the loop from being green for any implementation.
    #[test]
    fn every_name_the_listing_teaches_is_a_name_the_call_route_recognises() {
        let registered = Registered("tab.create");
        let json = handle_list(
            vec![decl("agent.list", "mine")],
            vec![entry("tab.create", "theirs")],
        )
        .to_json();
        let rows = json["commands"].as_array().expect("array");
        assert_eq!(rows.len(), 2, "premise: both sources were carried: {json}");

        // The stage-1 table is left empty — this measures recognition, not the outcome.
        let table = NoLocalCommands;
        for row in rows {
            let name = row["name"].as_str().expect("name");
            assert_eq!(row["callable"], true, "{json}");
            // `agent.list` is not in this empty table so it falls to the roster: this loop measures the roster-side half; the table-side half is the integration test with a real table.
            let known = !matches!(
                handle_call(&table, &registered, call_of(name)),
                CallOutcome::Answered(ControlQueryResult::Error { code, .. }) if code == "UNKNOWN_COMMAND"
            );
            assert!(
                known || name == "agent.list",
                "the call decision does not know a name the list taught: {name}"
            );
        }

        assert!(
            matches!(
                handle_call(&table, &registered, call_of("nobody.declares.this")),
                CallOutcome::Answered(ControlQueryResult::Error { ref code, .. }) if code == &"UNKNOWN_COMMAND"
            ),
            "negative control — a name not in the list must be an unknown command"
        );

        // The WS projection's field must not leak here — differing reachability rules would give it two meanings.
        assert!(
            rows.iter().all(|r| r["available"].is_null()),
            "WS's available was carried in this list: {json}"
        );
    }

    /// The caller-given number is used as is, and one that does not fit the syntax is bounced naming that field (ADR-0161 decisions 1·5).
    ///
    /// A new number here would keep delivery's duplicate prevention from ever triggering. How far that holds today: [`CallRequest::request_id`]
    /// (only while the round trip is open); this test measures only whether the number is used as is.
    #[test]
    fn a_caller_supplied_request_id_is_used_as_is_and_a_malformed_one_names_that_field() {
        let table = NoLocalCommands;
        let registered = Registered("tab.create");
        let mine = RequestId::new();

        let req = CallRequest {
            name: "tab.create".to_string(),
            request_id: Some(mine.to_string()),
            ..CallRequest::default()
        };
        match handle_call(&table, &registered, req) {
            CallOutcome::Relay { request_id, .. } => {
                assert_eq!(
                    request_id, mine,
                    "the caller-given number must be used as is"
                )
            }
            other => panic!("must be a relay: {other:?}"),
        }

        // If not sent, the daemon issues one — backward compatible (on that path no duplicate prevention triggers).
        match handle_call(&table, &registered, call_of("tab.create")) {
            CallOutcome::Relay { .. } => {}
            other => panic!("a request without a number must be accepted too: {other:?}"),
        }

        // A value from outside, so its syntax is validated — and that bounce must not say the whole body is wrong.
        for bad in ["", "not-a-uuid", &"9".repeat(4096)] {
            let req = CallRequest {
                name: "tab.create".to_string(),
                request_id: Some(bad.to_string()),
                ..CallRequest::default()
            };
            match handle_call(&table, &registered, req) {
                CallOutcome::Answered(ControlQueryResult::Error { code, hint }) => {
                    assert_eq!(code, "INVALID_ARGUMENT", "{hint}");
                    assert!(
                        hint.contains(CALLER_REQUEST_ID_FIELD),
                        "must say which field is wrong: {hint}"
                    );
                    assert!(
                        hint.len() < 4096,
                        "the wording grew with the request: {}",
                        hint.len()
                    );
                }
                other => panic!("must be a bounce: {other:?}"),
            }
        }
    }

    /// A wrong number gives the same answer regardless of the name — otherwise the contract would split by name.
    ///
    /// The stage-1 table must really hold that name: an empty table would drop `agent.list` into the relay branch too and never run the claimed axis.
    /// The sibling ([`a_caller_supplied_request_id_is_used_as_is_and_a_malformed_one_names_that_field`]) covers the relay branch with an empty table.
    #[test]
    fn a_malformed_request_id_is_refused_even_for_a_name_this_daemon_holds() {
        let req = CallRequest {
            name: "agent.list".to_string(),
            request_id: Some("nope".to_string()),
            ..CallRequest::default()
        };
        match handle_call(&Holds("agent.list"), &Registered("tab.create"), req) {
            CallOutcome::Answered(ControlQueryResult::Error { code, .. }) => {
                assert_eq!(code, "INVALID_ARGUMENT")
            }
            other => panic!("must be a bounce: {other:?}"),
        }
    }

    /// If the stage-1 table claims a name as its own and comes up empty-handed, it does not flow to relay: the caller would read a wiring defect it
    /// cannot fix as its own mistake ("unknown command", or a second bus refusal). Delivery exposes the same mismatch with the same code (`fold_local`).
    /// The real adapter cannot reach this state (`claim`·`run` share one `OnceLock` table), so a split-personality table is hand-built; the test's
    /// value is that any other implementation plugged in later must keep the property.
    #[test]
    fn a_table_that_claims_a_name_but_answers_nothing_is_not_relayed() {
        struct ClaimsButAnswersNothing;
        impl LocalCommands for ClaimsButAnswersNothing {
            fn claim(&self, _name: &str) -> Option<Effect> {
                Some(Effect::Write)
            }
            fn run(
                &self,
                _name: &str,
                _args: &mut serde_json::Value,
                _entrance: &'static str,
                _caller: Option<ConnId>,
            ) -> Option<Result<serde_json::Value, CommandError>> {
                None
            }
            fn decls(&self) -> Vec<CommandDecl> {
                Vec::new()
            }
            fn agent_roster(&self) -> Vec<engram_dashboard_agent::commands::AgentRosterRow> {
                Vec::new()
            }
        }

        // A name with an owner attached — if it flowed to relay, this would become `Relay`.
        match handle_call(
            &ClaimsButAnswersNothing,
            &Registered("tab.create"),
            call_of("tab.create"),
        ) {
            CallOutcome::Answered(ControlQueryResult::Error { code, hint }) => {
                assert_eq!(code, "INTERNAL", "{hint}");
                assert!(hint.contains("tab.create"), "must say which name: {hint}");
            }
            other => panic!("must be an INTERNAL bounce: {other:?}"),
        }
    }

    #[test]
    fn an_absent_args_object_is_the_same_as_an_empty_one() {
        let with: CallRequest =
            serde_json::from_str(r#"{"name":"agent.list","args":{}}"#).expect("deserialization");
        let without: CallRequest =
            serde_json::from_str(r#"{"name":"agent.list"}"#).expect("deserialization");
        assert_eq!(with.args.len(), without.args.len());
        assert_eq!(without.name, "agent.list");
    }

    /// `name` is top-level vocabulary and also a real argument name — spreading arguments at the top would make `agent.rename` uncallable here.
    #[test]
    fn a_command_argument_called_name_survives_alongside_the_command_name() {
        let req: CallRequest =
            serde_json::from_str(r#"{"name":"agent.rename","args":{"target":"a","name":"b"}}"#)
                .expect("deserialization");

        assert_eq!(req.name, "agent.rename");
        assert_eq!(req.args["name"], "b");
        assert_eq!(req.args["target"], "a");
    }

    /// An array body is not a call — derived deserialization would read `["agent.list"]` as a field sequence and execute it (measured); at an entrance with side effects that tolerance is an accident.
    #[test]
    fn a_json_array_is_not_a_command_call() {
        for body in [r#"["agent.list"]"#, r#"["agent.new",{"cwd":"C:/x"}]"#] {
            assert!(
                serde_json::from_str::<CallRequest>(body).is_err(),
                "a non-object body must be bounced: {body}"
            );
        }
    }

    /// Unknown top-level fields are ignored (additive evolution), but a request that lost its arguments does not silently succeed — the table's entrance check carries that; here only that fields do not leak into `args`.
    #[test]
    fn an_unknown_top_level_field_is_ignored_and_never_becomes_an_argument() {
        let req: CallRequest =
            serde_json::from_str(r#"{"name":"agent.move","target":"a","parent":"lead"}"#)
                .expect("deserialization");

        assert_eq!(req.name, "agent.move");
        assert!(
            req.args.is_empty(),
            "if a top-level field leaks into the arguments, the `args` discipline becomes meaningless: {:?}",
            *req.args
        );
    }

    /// Top-level fields only — duplicates inside `args` are owned and measured by [`CommandArgs`]; measuring them here would read as this file guarding a discipline it lacks.
    #[test]
    fn a_duplicate_name_is_refused() {
        assert!(
            serde_json::from_str::<CallRequest>(r#"{"name":"agent.list","name":"agent.new"}"#)
                .is_err()
        );
        assert!(serde_json::from_str::<CallRequest>(
            r#"{"name":"agent.list","args":{},"args":{}}"#
        )
        .is_err());
    }

    /// A roster that answers that one name has an owner attached.
    struct Registered(&'static str);

    impl OwnerLookupSource for Registered {
        fn lookup(&self, name: &str) -> OwnerLookup {
            if name == self.0 {
                OwnerLookup::Available(OwnerToken::new("conn-1"))
            } else {
                OwnerLookup::Unknown
            }
        }
    }

    fn call_of(name: &str) -> CallRequest {
        CallRequest {
            name: name.to_string(),
            ..CallRequest::default()
        }
    }

    /// The names the list taught go to the bus — folding them into a bounce would close the fifteen dashboard commands again (ADR-0160); a name nobody declared must stay "unknown command", or the caller keeps changing a real name.
    #[test]
    fn a_name_with_a_connected_owner_goes_to_the_bus_and_an_unknown_one_is_refused() {
        let table = NoLocalCommands;
        let registered = Registered("tab.create");

        match handle_call(&table, &registered, call_of("tab.create")) {
            CallOutcome::Relay { name, args, .. } => {
                assert_eq!(name, "tab.create");
                assert!(args.is_object(), "arguments are passed on as is: {args}");
            }
            CallOutcome::Answered(other) => {
                panic!("a name with an owner attached must be relayed: {other:?}")
            }
        }

        match handle_call(&table, &registered, call_of("nope.nope")) {
            CallOutcome::Answered(ControlQueryResult::Error { code, hint }) => {
                assert_eq!(code, "UNKNOWN_COMMAND");
                assert!(hint.contains("nope.nope"), "{hint}");
                assert!(hint.contains(CATALOG_RECOVERY), "{hint}");
            }
            other => panic!("an unknown name must be bounced: {other:?}"),
        }
    }

    /// A relayed outcome is made by the owner — the success payload is not opened, and failures ride the same function as other bounces (recovery guidance discipline applies).
    #[test]
    fn a_relayed_outcome_keeps_the_owners_answer() {
        let id = RequestId::new();

        let ok = relayed(CommandReply::ok(id, serde_json::json!({ "tab": "t-1" })));
        assert!(
            matches!(&ok, ControlQueryResult::Ok(v) if v["tab"] == "t-1"),
            "{ok:?}"
        );

        let failed = relayed(CommandReply::err(
            id,
            CommandError::invalid_argument("no such window 'ghost'"),
        ));
        match failed {
            ControlQueryResult::Error { code, hint } => {
                assert_eq!(code, "INVALID_ARGUMENT");
                assert!(hint.contains("ghost"), "{hint}");
                assert!(
                    hint.contains(CATALOG_RECOVERY),
                    "it is a fixable failure: {hint}"
                );
            }
            other => panic!("must be a bounce: {other:?}"),
        }
    }

    /// "Look at the list" on a failure with no argument to fix sends the caller after a nonexistent typo.
    #[test]
    fn the_recovery_is_attached_only_to_faults_the_catalog_can_fix() {
        let fixable = refused(CommandError::invalid_argument("bad field"));
        let unfixable = refused(CommandError::not_found("no agent called 'ghost'"));

        match (fixable, unfixable) {
            (
                ControlQueryResult::Error { hint: fixable, .. },
                ControlQueryResult::Error {
                    code,
                    hint: unfixable,
                },
            ) => {
                assert!(fixable.contains(CATALOG_RECOVERY), "{fixable}");
                assert!(fixable.contains("bad field"), "{fixable}");
                assert_eq!(code, "NOT_FOUND");
                assert!(!unfixable.contains(CATALOG_RECOVERY), "{unfixable}");
                assert!(unfixable.contains("ghost"), "{unfixable}");
            }
            other => panic!("both must be bounces: {other:?}"),
        }
    }

    /// The bounce wording does not grow with the request, whichever branch it goes out by: caller strings enter as the name (`unknown_name`) and inside
    /// the table's wording (`refused` — `check_args` puts the typed key in verbatim). Measuring one branch would leave the other with only the 1 MiB
    /// body cap and make this test read as a whole-file property.
    ///
    /// The number below is slack, not a contract — the contract is independence from request size, established by per-branch `preview_within`·`preview`.
    #[test]
    fn every_refusal_branch_is_reported_within_a_bound() {
        let huge = "k".repeat(500_000);
        let branches = [
            unknown_name(&huge),
            // A relayed failure also goes out as this surface's bounce — the owner's wording can be as large as the request.
            relayed(CommandReply::err(
                RequestId::new(),
                CommandError::of(ErrorCode::Conflict, format!("owner said: {huge}")),
            )),
            // The table's wording caps its own input (`quoted_input`), but some producers do not (`commands::drive_to_completion`) — measured in the shape that side arrives in.
            refused(CommandError::invalid_argument(format!(
                "not an argument of '{huge}': … — declared arguments: target, name"
            ))),
            refused(CommandError::not_found(format!("no agent called '{huge}'"))),
            malformed_body("expected a string", &huge),
        ];
        for branch in branches {
            match branch {
                ControlQueryResult::Error { code, hint } => {
                    assert!(hint.contains("truncated"), "says it cut ({code}): {hint}");
                    assert!(
                        hint.len() < 4096,
                        "the wording grew with the request ({code}): {} bytes",
                        hint.len()
                    );
                }
                other => panic!("must be a bounce: {other:?}"),
            }
        }
    }

    /// Cutting keeps both ends — the caller's input at the head, the next thing to do at the tail; head-only would lose that (the table's argument
    /// bounce attaches all declared fields after the wording).
    ///
    /// The input is a hypothetical worst case that passed no cap (the designed worst case is much smaller and uncut — measured on a real route by
    /// the `an_unknown_field_is_refused_with_the_declared_argument_list` family); this measures what remains when cut.
    #[test]
    fn truncation_keeps_both_ends_so_the_next_step_survives() {
        let refusal = refused(CommandError::invalid_argument(format!(
            "not an argument of 'agent.rename': \"{}\" — declared arguments: target, name",
            "n".repeat(500_000)
        )));

        match refusal {
            ControlQueryResult::Error { hint, .. } => {
                assert!(hint.contains("truncated"), "says it cut: {hint}");
                assert!(
                    hint.contains("not an argument of 'agent.rename'"),
                    "the head (what was wrong) must remain: {hint}"
                );
                for declared in ["declared arguments", "target", "name"] {
                    assert!(
                        hint.contains(declared),
                        "the tail (what to fix it with) was cut off ({declared}): {hint}"
                    );
                }
            }
            other => panic!("must be a bounce: {other:?}"),
        }
    }
}
