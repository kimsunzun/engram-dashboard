//! Per-process handler table — 「my table」 (TRD §3-3).

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use crate::{CommandDecl, CommandError, CommandSpec, ErrorCode};

pub type CommandFuture =
    Pin<Box<dyn Future<Output = Result<serde_json::Value, CommandError>> + Send>>;

pub trait CommandHandler: Send + Sync {
    fn call(&self, args: serde_json::Value) -> CommandFuture;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableError {
    /// The same name came in twice — we do not silently overwrite.
    Duplicate(&'static str),
    /// A name not in the declaration set of the crate that made this table — blocks a second vocabulary from arising here.
    NotDeclared(&'static str),
    /// The declaration's schema text (`args` or `ok`) is not JSON — it can be neither advertised nor used to fit arguments.
    /// ★Both fields are checked together★ — either breaking breaks the registration packet's `help` whole. (`lint_spec` catches it first
    ///   in the declaration crate's tests.)
    InvalidSchema(&'static str),
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate(name) => write!(f, "command already in this table: {name}"),
            Self::NotDeclared(name) => {
                write!(f, "command not declared by this crate: {name}")
            }
            Self::InvalidSchema(name) => {
                write!(f, "declared args/ok schema is not JSON: {name}")
            }
        }
    }
}

impl std::error::Error for TableError {}

struct Entry {
    spec: &'static CommandSpec,
    /// The declared argument schema — parsed **once at assembly**. This is so no parser sits on the delivery path.
    args_schema: serde_json::Value,
    handler: Arc<dyn CommandHandler>,
}

/// The table of commands one process (one assembly unit) **can run directly**.
///
/// ★Not a global static★ — `make_table(deps)` builds it at assembly and injects handlers (rule T-1), so a harness can plug in fake
///   dependencies (ADR-0012).
// ADR-0155
pub struct CommandTable {
    declared: &'static [&'static CommandSpec],
    entries: BTreeMap<&'static str, Entry>,
}

impl CommandTable {
    /// `declared` = **the set the crate building this table declared** (`COMMAND_SPECS`, from the declaration macro), not the whole binary
    ///   ([`crate::command_specs`]) — another crate's names must be blocked too.
    ///
    /// ★A name declared twice panics here★ — blow up before assembly: the duplicate is fixed by the build, and left alone the table and
    ///   the advertised schema would point at different declarations.
    pub fn new(declared: &'static [&'static CommandSpec]) -> Self {
        for (i, spec) in declared.iter().enumerate() {
            if declared[i + 1..]
                .iter()
                .any(|other| other.name == spec.name)
            {
                panic!(
                    "command declared more than once in this crate: {} — when two declarations hold the same name, it is unknown which contract wins",
                    spec.name
                );
            }
        }
        Self {
            declared,
            entries: BTreeMap::new(),
        }
    }

    pub fn insert(
        &mut self,
        name: &'static str,
        handler: Arc<dyn CommandHandler>,
    ) -> Result<(), TableError> {
        let Some(spec) = crate::spec::find_unique(self.declared.iter().copied(), name) else {
            return Err(TableError::NotDeclared(name));
        };
        if self.entries.contains_key(name) {
            return Err(TableError::Duplicate(name));
        }
        let Ok(args_schema) = serde_json::from_str::<serde_json::Value>(spec.args_schema) else {
            return Err(TableError::InvalidSchema(name));
        };
        // ★The table does not use `ok_schema`, but checks it too★: `spec_item_json` attaches it into `help` **as is** (`spec.rs`), so one
        //   broken `ok` makes the registration packet's `help` not JSON;
        //   the daemon stores it as an opaque string (ADR-0156), so assembly is the last checkpoint. The value is discarded.
        if serde_json::from_str::<serde::de::IgnoredAny>(spec.ok_schema).is_err() {
            return Err(TableError::InvalidSchema(name));
        }
        self.entries.insert(
            name,
            Entry {
                spec,
                args_schema,
                handler,
            },
        );
        Ok(())
    }

    /// Runs one name **by the advertised contract** — not in the table → `None`, `args` untouched (the registry, the next step, carries it
    ///   as is).
    ///
    /// ★There is no door that hands out the handler★: fitting arguments to the declared schema in this one place makes "what is advertised
    ///   is accepted" a property of **the whole table**
    /// ([`crate::coerce`]), so a handler implementing [`CommandHandler`] directly gets the same coercion. The net that folds body panics
    ///   is the caller's ([`crate::route`], covering both `call` and first poll).
    pub fn call(&self, name: &str, args: &mut serde_json::Value) -> Option<CommandFuture> {
        let entry = self.entries.get(name)?;
        crate::coerce::integral_numbers_to_integers(&entry.args_schema, args);
        crate::coerce::enum_words_to_declared_spelling(&entry.args_schema, args);
        Some(entry.handler.call(args.take()))
    }

    /// Checks **whether the arguments match the declaration** before the call — rejects unknown fields and missing required fields by
    ///   naming them.
    ///
    /// ★Called only on the surface where what a person or LLM just typed comes in (ADR-0157)★: in inter-hop wiring a new field from a
    ///   newer caller would hard-fail an old owner and kill additive evolution (TRD §4-③) —
    /// [`crate::route`] not calling it is the concrete form of that decision.
    ///
    /// The rejection message carries **the wrong fields and the whole declared set** so the caller can fix it itself. ★No vocabulary of
    ///   the calling surface in the message★: this crate does not know it;
    /// the adapter adds guidance via [`CommandError::set_message`].
    ///
    /// Three things pass through (no material to check):
    /// - **A name not in this table** — its contract is held by another. ★Nobody behind it checks for us★: delivery forwards the envelope
    ///   as is and the receiving hop's [`crate::route`]→[`CommandTable::call`] does not screen,
    ///   so another owner's name with a mistyped field is caught nowhere (deserialization silently drops it) — the failure ADR-0157 meant
    ///   to block, one hop later (that decision layers by **origin**, not by name owner).
    ///   ★A duty of the entry adapter **pair**; the wiring wave decides which side★: the typing-side adapter holds no other's declaration
    ///   (opaque to the daemon — ADR-0156), the owner-side adapter cannot tell from the envelope
    ///   whether it was typed. Not closed here.
    /// - **A declaration without `properties`** — rejecting without the allowed set would block good arguments.
    /// - **Below the top level** — this compares against the property set, not a schema validator; a nested typo passes and is caught by
    ///   deserialization (type mismatch) or ignored.
    // ADR-0157
    pub fn check_args(&self, name: &str, args: &serde_json::Value) -> Result<(), CommandError> {
        let Some(entry) = self.entries.get(name) else {
            return Ok(());
        };
        let serde_json::Value::Object(given) = args else {
            return Err(CommandError::invalid_argument(format!(
                "arguments for '{name}' must be an object — declared arguments: {}",
                declared_names(&entry.args_schema)
            )));
        };
        let Some(declared) = entry
            .args_schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
        else {
            return Ok(());
        };
        // Unknown fields first — what the caller should fix is **the field it typed**, not the missing required field a typo causes.
        // ★Count **all** wrong fields★: naming one makes the caller fix, resend and be rejected again (field order is alphabetical, not
        //   "typed first"); showing three typos at once is the net's worth.
        let unknown: Vec<&String> = given
            .keys()
            .filter(|key| !declared.contains_key(key.as_str()))
            .collect();
        if !unknown.is_empty() {
            return Err(CommandError::invalid_argument(format!(
                "not an argument of '{name}': {} — declared arguments: {}",
                quoted_list(&unknown),
                declared_names(&entry.args_schema)
            )));
        }
        if let Some(serde_json::Value::Array(required)) = entry.args_schema.get("required") {
            for wanted in required.iter().filter_map(serde_json::Value::as_str) {
                if !given.contains_key(wanted) {
                    return Err(CommandError::invalid_argument(format!(
                        "'{name}' requires '{wanted}' — declared arguments: {}",
                        declared_names(&entry.args_schema)
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// The list to register with the daemon — carries the name and the **shape** together (ADR-0156).
    pub fn decls(&self) -> Vec<CommandDecl> {
        self.entries
            .values()
            .map(|e| CommandDecl {
                name: e.spec.name.to_string(),
                help: crate::spec_item_json(e.spec),
                agent_args: e
                    .spec
                    .agent_args
                    .iter()
                    .map(|arg| arg.to_string())
                    .collect(),
                deadline: e.spec.deadline,
            })
            .collect()
    }

    pub fn specs(&self) -> impl Iterator<Item = &'static CommandSpec> + '_ {
        self.entries.values().map(|e| e.spec)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The allowed set the rejection message carries — not truncated: with a truncated list the caller cannot tell whether the name it failed
///   to pick is on the cut side
/// (these names are **our declaration**; the caller's string is truncated by [`quoted_input`]).
fn declared_names(args_schema: &serde_json::Value) -> String {
    let Some(properties) = args_schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
    else {
        // A declaration that does not carry the allowed set — that case passes through (`check_args`), so what arrives here is only the
        //   case caught as
        //   a non-object argument.
        return "(not declared)".to_string();
    };
    if properties.is_empty() {
        return "(none)".to_string();
    }
    properties
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The upper bound on the **number of wrong fields** named in one message.
///
/// ★Count them all but do not carry them all★: the field count can grow as large as the request body, and the receiving side pays for the
///   message (as [`quoted_input`]). Beyond the cap it
/// **says how many more there are**, so the caller does not read the list as everything.
/// ★The eight named are the first eight **alphabetically**, not "typed first"★ — the argument map is sorted; insertion order would need an
///   insertion-order-preserving map, a decision outside this crate (the test below pins this).
const MAX_NAMED_UNKNOWN: usize = 8;

/// The size cap (bytes) of one name's **literal** (`"…"` — including the quotes and ellipsis).
///
/// ★Without it one 100 MiB field name makes a message of that size★ — carried in the error reply onto the wire, paid by the receiving
///   side. The value is just enough for a person to recognize their typo.
/// ★Measures the literal only★: the truncation marker (`(truncated, input was N bytes)`, a bit over 30 B) is on top, up to **~170 B** per
///   piece — use that sum for a whole-piece count, do not reuse this constant.
/// ★Not a cap to shrink things★: for input whose escaped notation is 129~158 B the truncated form is **longer** than the original; what it
///   guards is **the finiteness of the tail**.
const MAX_QUOTED_INPUT_BYTES: usize = 128;

/// Puts the wrong field names into the message — each name is subject to the [`quoted_input`] cap, and the count to [`MAX_NAMED_UNKNOWN`].
fn quoted_list(keys: &[&String]) -> String {
    let named: Vec<String> = keys
        .iter()
        .take(MAX_NAMED_UNKNOWN)
        .map(|key| quoted_input(key))
        .collect();
    let listed = named.join(", ");
    match keys.len().checked_sub(MAX_NAMED_UNKNOWN) {
        Some(rest) if rest > 0 => format!("{listed} (+{rest} more)"),
        _ => listed,
    }
}

/// Trims a name the caller gave so it can be quoted in the message — a truncation **says it truncated** (else the caller would fix a
///   perfectly good field).
///
/// ★The name is wrapped in Rust string-literal notation★: single quotes would print **one** field `a', 'b` as `'a', 'b'` and read as
///   **two** wrong fields; `{:?}` escapes quotes, backslashes and control characters.
/// ★A truncated name also **closes its quote** (load-bearing)★: otherwise in a message joining several names the quote pairing goes out of
///   step and the ambiguity returns.
/// ★The cap is applied **after escaping**★: truncating first lets one control character swell to six bytes (`\u{1b}`), so the message
///   would grow up to six times the cap by input kind and the documented number would be false.
/// ★Do not truncate in the middle of an escape sequence★: it accumulates character by character and stops at the budget, so no lone `\`
///   piece appears (broken notation).
/// ★The marker states **what the number counts**★: what is visible is the escaped notation but the number is original bytes — unlabeled,
///   30 control characters (180 characters shown) would read as "truncated 30 bytes".
fn quoted_input(text: &str) -> String {
    let quoted = format!("{text:?}");
    if quoted.len() <= MAX_QUOTED_INPUT_BYTES {
        return quoted;
    }
    // Reserve in advance the room for the opening quote, closing quote and ellipsis.
    let budget = MAX_QUOTED_INPUT_BYTES.saturating_sub('"'.len_utf8() * 2 + '…'.len_utf8());
    let mut shown = String::with_capacity(budget);
    for ch in text.chars() {
        let piece = ch.escape_debug().to_string();
        if shown.len() + piece.len() > budget {
            break;
        }
        shown.push_str(&piece);
    }
    format!("\"{shown}…\"(truncated, input was {} bytes)", text.len())
}

struct Blocking<F, A, O> {
    run: Arc<F>,
    _types: PhantomData<fn(A) -> O>,
}

impl<F, A, O> CommandHandler for Blocking<F, A, O>
where
    F: Fn(A) -> Result<O, CommandError> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    fn call(&self, args: serde_json::Value) -> CommandFuture {
        // ★The body runs inside the future (not inside `call`)★: if `call` did the work and returned a future holding only the result,
        //   `timeout(d, handler.call(args))` would **never fire** — the deadline
        //   (TRD §4-⑥) cannot stand on that shape. Before polling, cancellation really is cancellation.
        let run = Arc::clone(&self.run);
        Box::pin(async move { crate::route::guard_panic(|| Self::apply(&run, args)) })
    }
}

impl<F, A, O> Blocking<F, A, O>
where
    F: Fn(A) -> Result<O, CommandError> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    fn apply(run: &F, args: serde_json::Value) -> Result<serde_json::Value, CommandError> {
        let parsed: A = serde_json::from_value(args)
            .map_err(|e| CommandError::invalid_argument(e.to_string()))?;
        let ok = run(parsed)?;
        serde_json::to_value(ok).map_err(|e| CommandError::of(ErrorCode::Internal, e.to_string()))
    }
}

/// Wraps a synchronous (blocking) body as a handler — argument deserialization and return serialization are done here, once.
///
/// ★Blocking contract (for the caller)★: the body **runs to the end on the first poll**; polled on an async runtime thread it blocks that
///   thread (the agent control verbs write to disk under the profile lock and poll
/// for early resume exit — same contract in daemon `control/commands.rs`), so call it behind `spawn_blocking`. Deserialization failure =
///   `INVALID_ARGUMENT` · body panic = `INTERNAL` (never ends without a reply).
/// ★The schema↔serde coercion is not here★ — [`CommandTable::call`] does it from the declaration; putting it here would give two places
///   different rules.
pub fn blocking_handler<A, O, F>(run: F) -> Arc<dyn CommandHandler>
where
    F: Fn(A) -> Result<O, CommandError> + Send + Sync + 'static,
    A: serde::de::DeserializeOwned + Send + 'static,
    O: serde::Serialize + Send + 'static,
{
    Arc::new(Blocking {
        run: Arc::new(run),
        _types: PhantomData,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::testing::block_on;
    use crate::Effect;

    static PING: CommandSpec = CommandSpec {
        name: "fixture.ping",
        effect: Effect::Read,
        since: 1,
        summary: "  summary indented by two spaces  ",
        args_schema: "{\"type\":\"object\",\"properties\":{}}",
        ok_schema: "{\"type\":\"object\",\"properties\":{\"pong\":{\"type\":\"boolean\"}}}",
        errors: &[ErrorCode::Internal],
        args_type: "PingArgs",
        ok_type: "PingOk",
        agent_args: &[],
        deadline: crate::DeadlineClass::Ordinary,
    };

    /// A declaration holding both integer and real fields — checks here that the table fits arguments by looking at the schema.
    static NUMBERS: CommandSpec = CommandSpec {
        name: "fixture.numbers",
        effect: Effect::Write,
        since: 1,
        summary: "numeric fields",
        args_schema: concat!(
            r#"{"type":"object","properties":{"count":{"type":"integer","minimum":0},"#,
            r#""depth":{"type":"integer"},"ratio":{"type":"number"},"#,
            r#""rows":{"type":"array","items":{"type":"object","properties":{"#,
            r#""size":{"type":"integer"}},"required":["size"]}}},"#,
            r#""required":["count","depth","ratio","rows"]}"#
        ),
        ok_schema: "{\"type\":\"object\",\"properties\":{}}",
        errors: &[],
        args_type: "NumbersArgs",
        ok_type: "NumbersOk",
        agent_args: &[],
        deadline: crate::DeadlineClass::Ordinary,
    };
    /// A declaration holding both required and omittable fields — checks here that the entry screening tells the two apart.
    static ENTRANCE: CommandSpec = CommandSpec {
        name: "fixture.entrance",
        effect: Effect::Write,
        since: 1,
        summary: "entrance check fixture",
        args_schema: concat!(
            r#"{"type":"object","properties":{"cwd":{"type":"string"},"#,
            r#""label":{"anyOf":[{"type":"string"},{"type":"null"}]}},"#,
            r#""required":["cwd"]}"#
        ),
        ok_schema: "{\"type\":\"object\",\"properties\":{}}",
        errors: &[],
        args_type: "EntranceArgs",
        ok_type: "EntranceOk",
        agent_args: &[],
        deadline: crate::DeadlineClass::Ordinary,
    };
    static DECLARED: &[&CommandSpec] = &[&PING, &NUMBERS, &ENTRANCE];

    #[derive(serde::Deserialize)]
    struct NoArgs {}

    fn handler() -> Arc<dyn CommandHandler> {
        blocking_handler(|_: NoArgs| Ok(json!({ "pong": true })))
    }

    #[test]
    fn duplicate_name_is_refused_not_overwritten() {
        let mut table = CommandTable::new(DECLARED);
        table
            .insert("fixture.ping", handler())
            .expect("first insert");
        assert_eq!(
            table.insert("fixture.ping", handler()),
            Err(TableError::Duplicate("fixture.ping"))
        );
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn name_outside_this_crates_declarations_is_refused() {
        let mut table = CommandTable::new(DECLARED);
        assert_eq!(
            table.insert("tab.create", handler()),
            Err(TableError::NotDeclared("tab.create"))
        );
        assert!(table.is_empty());
    }

    /// ★Do not silently pick one★ — two declarations of the same name mean there are two contracts.
    #[test]
    fn a_name_declared_twice_fails_the_table_instead_of_picking_one() {
        static OTHER_PING: CommandSpec = CommandSpec {
            name: "fixture.ping",
            effect: Effect::Write,
            since: 2,
            summary: "same name, different contract",
            args_schema: "{\"type\":\"object\",\"properties\":{}}",
            ok_schema: "{\"type\":\"object\",\"properties\":{}}",
            errors: &[],
            args_type: "OtherPingArgs",
            ok_type: "OtherPingOk",
            agent_args: &[],
            deadline: crate::DeadlineClass::Ordinary,
        };
        static COLLIDING: &[&CommandSpec] = &[&PING, &OTHER_PING];

        let outcome = crate::testing::with_quiet_panic_hook(|| {
            std::panic::catch_unwind(|| CommandTable::new(COLLIDING))
        });

        assert!(
            outcome.is_err(),
            "a duplicate declaration blows up at table creation"
        );
    }

    #[test]
    fn unknown_lookup_is_none() {
        let mut table = CommandTable::new(DECLARED);
        assert!(!table.contains("fixture.ping"), "not plugged in yet");
        assert!(table.call("fixture.ping", &mut json!({})).is_none());

        table.insert("fixture.ping", handler()).expect("insert");
        assert!(!table.contains("nope"));
        let mut args = json!({ "kept": 1 });
        assert!(table.call("nope", &mut args).is_none());
        assert_eq!(
            args,
            json!({ "kept": 1 }),
            "if not found, it does not take the arguments"
        );
    }

    /// ★Assembly does not run carrying a declaration it cannot advertise★ — if the schema text is not JSON, the registration packet's
    /// `help` breaks too and there is no material to fit arguments either.
    #[test]
    fn a_declaration_whose_schema_is_not_json_is_refused_at_assembly() {
        static BROKEN: CommandSpec = CommandSpec {
            name: "fixture.broken",
            effect: Effect::Read,
            since: 1,
            summary: "broken schema",
            args_schema: "{not json",
            ok_schema: "{\"type\":\"object\",\"properties\":{}}",
            errors: &[],
            args_type: "BrokenArgs",
            ok_type: "BrokenOk",
            agent_args: &[],
            deadline: crate::DeadlineClass::Ordinary,
        };
        static WITH_BROKEN: &[&CommandSpec] = &[&BROKEN];

        let mut table = CommandTable::new(WITH_BROKEN);
        assert_eq!(
            table.insert("fixture.broken", handler()),
            Err(TableError::InvalidSchema("fixture.broken"))
        );
        assert!(table.is_empty());
    }

    /// ★The same holds when `ok` is broken★ — `spec_item_json` attaches `ok_schema` into `help` as is, so checking only the argument
    ///   schema would let **a packet with a broken `help` through**;
    /// the daemon stores it as an opaque string (ADR-0156), so nothing catches it later.
    #[test]
    fn a_declaration_whose_ok_schema_is_not_json_is_refused_at_assembly() {
        static BROKEN_OK: CommandSpec = CommandSpec {
            name: "fixture.broken-ok",
            effect: Effect::Read,
            since: 1,
            summary: "arguments are fine and the return is broken",
            args_schema: "{\"type\":\"object\",\"properties\":{}}",
            ok_schema: "{not json",
            errors: &[],
            args_type: "BrokenOkArgs",
            ok_type: "BrokenOkOk",
            agent_args: &[],
            deadline: crate::DeadlineClass::Ordinary,
        };
        static WITH_BROKEN_OK: &[&CommandSpec] = &[&BROKEN_OK];

        let mut table = CommandTable::new(WITH_BROKEN_OK);
        assert_eq!(
            table.insert("fixture.broken-ok", handler()),
            Err(TableError::InvalidSchema("fixture.broken-ok"))
        );
        assert!(table.is_empty());
        // The `help` that would have gone out had this declaration passed — it is not JSON.
        assert!(
            serde_json::from_str::<serde_json::Value>(&crate::spec_item_json(&BROKEN_OK)).is_err()
        );
    }

    #[test]
    fn decls_carry_the_schema_item_as_help() {
        let mut table = CommandTable::new(DECLARED);
        table.insert("fixture.ping", handler()).expect("insert");

        let decls = table.decls();
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "fixture.ping");
        assert_eq!(decls[0].help, crate::spec_item_json(&PING));
        // It must have **the same origin** as the elements of the derived file (ADR-0156).
        let parsed: serde_json::Value = serde_json::from_str(&decls[0].help).expect("help is JSON");
        assert_eq!(parsed["name"], "fixture.ping");
        assert_eq!(parsed["effect"], "Read");
        assert_eq!(parsed["summary"], "summary indented by two spaces");
        assert_eq!(parsed["ok"]["properties"]["pong"]["type"], "boolean");
        assert_eq!(parsed["errors"][0], "INTERNAL");
        // The common errors the table adds are in the advertisement (they are not in the declaration).
        assert_eq!(parsed["errors"][1], "INVALID_ARGUMENT");
    }

    /// ★Confirms in bytes that the disk file and the registration packet have **the same single origin**★ (ADR-0156). Comparing the two
    ///   paths against each other would call the same function twice
    /// and **pass even if both drift together**, so a fixed expected byte string sits in the middle.
    #[test]
    fn the_schema_file_element_is_byte_identical_to_help() {
        const EXPECTED: &str = concat!(
            r#"{"name":"fixture.ping","effect":"Read","since":1,"summary":"summary indented by two spaces","#,
            r#""args":{"type":"object","properties":{}},"#,
            r#""ok":{"type":"object","properties":{"pong":{"type":"boolean"}}},"#,
            r#""errors":["INTERNAL","INVALID_ARGUMENT"]}"#
        );

        let mut table = CommandTable::new(DECLARED);
        table.insert("fixture.ping", handler()).expect("insert");
        let help = table.decls().remove(0).help;

        assert_eq!(help, EXPECTED, "help of the registration packet");
        assert!(
            crate::catalog_json(1, DECLARED).contains(EXPECTED),
            "element of the derived file"
        );
    }

    /// ★The marking goes out in a separate field, not in `help`★ (ADR-0308) — the daemon does not open `help`.
    #[test]
    fn decls_carry_the_agent_reference_marks_beside_the_help() {
        static MARKED: CommandSpec = CommandSpec {
            name: "fixture.marked",
            effect: Effect::Write,
            since: 1,
            summary: "mark fixture",
            args_schema: concat!(
                r#"{"type":"object","properties":{"target":{"type":"string"},"#,
                r#""after":{"anyOf":[{"type":"string"},{"type":"null"}]}},"#,
                r#""required":["target"]}"#
            ),
            ok_schema: "{\"type\":\"object\",\"properties\":{}}",
            errors: &[],
            args_type: "MarkedArgs",
            ok_type: "MarkedOk",
            agent_args: &["target", "after"],
            deadline: crate::DeadlineClass::Ordinary,
        };
        static MARKED_DECLARED: &[&CommandSpec] = &[&MARKED, &PING];

        let mut table = CommandTable::new(MARKED_DECLARED);
        table.insert("fixture.marked", handler()).expect("insert");
        table.insert("fixture.ping", handler()).expect("insert");

        let decls = table.decls();
        let marked = decls
            .iter()
            .find(|d| d.name == "fixture.marked")
            .expect("marked command");
        assert_eq!(
            marked.agent_args,
            ["target", "after"],
            "in declaration order"
        );
        assert_eq!(
            marked.help,
            crate::spec_item_json(&MARKED),
            "help is unrelated to the marks"
        );
        assert!(!marked.help.contains("agent_args"));
        let plain = decls
            .iter()
            .find(|d| d.name == "fixture.ping")
            .expect("unmarked command");
        assert!(plain.agent_args.is_empty());
    }

    /// ★The seat and the client's wait read one declaration★ (ADR-0369): for every registered decl, the class field the
    /// daemon sizes the seat by equals the class its `help` states, which the CLI reads.
    #[test]
    fn every_decl_carries_the_class_its_help_states() {
        static SPAWN: CommandSpec = CommandSpec {
            name: "fixture.spawn",
            effect: Effect::Write,
            since: 1,
            summary: "launch fixture",
            args_schema: "{\"type\":\"object\",\"properties\":{}}",
            ok_schema: "{\"type\":\"object\",\"properties\":{}}",
            errors: &[],
            args_type: "SpawnArgs",
            ok_type: "SpawnOk",
            agent_args: &[],
            deadline: crate::DeadlineClass::Launch,
        };
        static WITH_LAUNCH: &[&CommandSpec] = &[&SPAWN, &PING];

        let mut table = CommandTable::new(WITH_LAUNCH);
        table.insert("fixture.spawn", handler()).expect("insert");
        table.insert("fixture.ping", handler()).expect("insert");

        let decls = table.decls();
        assert_eq!(decls.len(), 2);
        for decl in &decls {
            let help: serde_json::Value = serde_json::from_str(&decl.help).expect("help is JSON");
            assert_eq!(
                decl.deadline,
                crate::DeadlineClass::of_schema_entry(&help),
                "{}",
                decl.name
            );
        }
        let class_of = |name: &str| {
            decls
                .iter()
                .find(|d| d.name == name)
                .map(|d| d.deadline)
                .expect("registered")
        };
        assert_eq!(class_of("fixture.spawn"), crate::DeadlineClass::Launch);
        assert_eq!(class_of("fixture.ping"), crate::DeadlineClass::Ordinary);
    }

    #[test]
    fn bad_arguments_become_invalid_argument() {
        let table_handler = handler();
        let outcome = block_on(table_handler.call(json!({ "unexpected": [1, 2] })));
        // Unknown fields are ignored (we do not attach deny_unknown_fields — additive rule) and it succeeds.
        assert!(outcome.is_ok());

        let outcome = block_on(table_handler.call(json!("not an object")));
        assert_eq!(
            outcome.expect_err("bounced if not an object").code(),
            ErrorCode::InvalidArgument
        );
    }

    #[derive(serde::Deserialize)]
    struct Numbers {
        count: u32,
        depth: i64,
        ratio: f64,
        rows: Vec<Inner>,
    }

    #[derive(serde::Deserialize)]
    struct Inner {
        size: u32,
    }

    /// ★A handler that does not go through the adapter★ — were coercion a property of `blocking_handler`, this handler would advertise the
    ///   same schema and accept input differently,
    /// so the coercion test is built on this shape (Step 3's `await`ing commands are this shape).
    struct RawNumbers;
    impl CommandHandler for RawNumbers {
        fn call(&self, args: serde_json::Value) -> CommandFuture {
            Box::pin(async move {
                let parsed: Numbers = serde_json::from_value(args)
                    .map_err(|e| CommandError::invalid_argument(e.to_string()))?;
                Ok(json!({
                    "count": parsed.count,
                    "depth": parsed.depth,
                    "ratio": parsed.ratio,
                    "size": parsed.rows.first().map(|r| r.size),
                }))
            })
        }
    }

    fn numbers_table() -> CommandTable {
        let mut table = CommandTable::new(DECLARED);
        table
            .insert("fixture.numbers", Arc::new(RawNumbers))
            .expect("declared name");
        table
    }

    fn call_numbers(mut args: serde_json::Value) -> Result<serde_json::Value, CommandError> {
        let table = numbers_table();
        let future = table
            .call("fixture.numbers", &mut args)
            .expect("name in the table");
        block_on(future)
    }

    /// ★What the advertised schema accepts is actually accepted★ — `{"type":"integer"}` includes `1.0` (JSON's number
    /// model has no distinction between `1` and `1.0`) yet serde rejects an f64 in an integer field. The table fills that gap.
    #[test]
    fn an_integral_float_is_accepted_the_way_the_schema_says() {
        let outcome = call_numbers(json!({
            "count": 2.0,
            "depth": -3.0,
            "ratio": 1.0,
            "rows": [{ "size": 4.0 }],
        }))
        .expect("accepts as the schema advertised");

        assert_eq!(outcome["count"], 2);
        assert_eq!(outcome["depth"], -3);
        assert_eq!(outcome["ratio"], 1.0, "a float field stays a float");
        assert_eq!(outcome["size"], 4, "the same down inside nesting·arrays");
    }

    #[test]
    fn a_fractional_value_is_still_refused_for_an_integer_field() {
        let outcome = call_numbers(json!({
            "count": 2.5, "depth": 0, "ratio": 1.0, "rows": [],
        }));

        assert_eq!(
            outcome
                .expect_err("with a fractional part it is not an integer")
                .code(),
            ErrorCode::InvalidArgument
        );
    }

    /// ★Outside 2^53 f64 cannot hold an integer exactly★ — the rewrite itself would change the value, so we leave it as is.
    #[test]
    fn a_float_beyond_exact_integer_range_is_left_alone() {
        let outcome = call_numbers(json!({
            "count": 0, "depth": 1e300, "ratio": 1.0, "rows": [],
        }));

        assert_eq!(
            outcome.expect_err("cannot fit in an integer field").code(),
            ErrorCode::InvalidArgument
        );
    }

    // ── Entry screening (ADR-0157) ──────────────────────────────────────────────────────────────────

    fn entrance_table() -> CommandTable {
        let mut table = CommandTable::new(DECLARED);
        table
            .insert("fixture.entrance", handler())
            .expect("declared name");
        table
    }

    /// ★If an unknown field is silently ignored, **success is reported**★ — the caller does not know that something different from what it
    /// instructed happened (the reason ADR-0157 set up this net). So it rejects, but also gives **material to fix it itself**.
    #[test]
    fn an_undeclared_argument_field_is_named_and_refused() {
        let err = entrance_table()
            .check_args(
                "fixture.entrance",
                &json!({ "cwd": "C:/x", "target": "alpha" }),
            )
            .expect_err("field not in the declaration");

        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("target"),
            "points at the wrong field: {}",
            err.message()
        );
        assert!(
            err.message().contains("cwd") && err.message().contains("label"),
            "emits the allowed set too: {}",
            err.message()
        );
    }

    /// ★If there are several wrong fields, all of them come out★ — if only one is pointed out, the caller repeats reject, fix, retry as many
    /// times as there are typos. Field order is alphabetical, so there is no meaning of 「the field typed first」 either.
    #[test]
    fn every_undeclared_argument_field_is_named_not_just_the_first() {
        let err = entrance_table()
            .check_args(
                "fixture.entrance",
                &json!({ "cwd": "C:/x", "target": "alpha", "zzz": 1, "aaa": 2 }),
            )
            .expect_err("three fields not in the declaration");

        for offender in ["target", "zzz", "aaa"] {
            assert!(
                err.message().contains(offender),
                "{offender} is missing: {}",
                err.message()
            );
        }
    }

    /// A rejection message carrying `count` fields absent from the declaration (each `key_bytes` bytes).
    ///
    /// ★Insert in **reverse alphabetical order**★: ascending would make insertion and alphabetical order coincide, so a switch to an
    ///   insertion-order map (`preserve_order`) would still pass the
    /// "first eight alphabetically" assertion unnoticed.
    fn flood_refusal(count: usize, key_bytes: usize) -> String {
        flood_refusal_of(count, key_bytes, "x")
    }

    /// `fill` = the character that fills the name — kept separate so we can measure whether printable characters and escaped characters
    ///   fall under the same cap
    /// (escape expansion is an axis that can make the cap differ by kind).
    fn flood_refusal_of(count: usize, key_bytes: usize, fill: &str) -> String {
        let mut given = serde_json::Map::new();
        given.insert("cwd".to_string(), json!("C:/x"));
        for i in (0..count).rev() {
            given.insert(format!("stray{i:04}{}", fill.repeat(key_bytes)), json!(1));
        }
        entrance_table()
            .check_args("fixture.entrance", &serde_json::Value::Object(given))
            .expect_err("many fields not in the declaration")
            .message()
            .to_string()
    }

    /// If the count cap is exceeded it says **how many more there are** — if the caller thinks the list is everything it cannot fix the
    ///   remaining fields.
    #[test]
    fn a_flood_of_unknown_fields_is_capped_and_says_how_many_are_left() {
        let message = flood_refusal(40, 0);

        assert!(
            message.contains("(+32 more)"),
            "says the remaining count: {message}"
        );
        assert!(
            message.contains("stray0000"),
            "the named fields are actually carried: {message}"
        );
    }

    /// ★The message length is **unrelated** to the request size★ — the property is not "at most N bytes" but "does not grow as the input
    ///   grows"; the per-name cap ([`quoted_input`]) and the count cap
    /// ([`MAX_NAMED_UNKNOWN`]) are both needed (one alone grows along the other axis).
    ///
    /// The measured worst case is **about 1.4 KB** (eight names × quote cap 128B + marker + declared set), the same for escaped names (the
    ///   cap applies **after** escaping) — not a contract; the contract is the first assertion below.
    #[test]
    fn the_refusal_message_does_not_grow_with_the_body() {
        let small = flood_refusal(40, 1 << 10);
        let huge = flood_refusal(4_000, 1 << 14);

        // The width allowed to grow = **the number of digits only** (the "N bytes" notation of the eight names and the "+N more"); the
        //   margin of 32 covers that.
        assert!(
            huge.len() <= small.len() + 32,
            "100x the field count·16x the name size, yet the message grew beyond the digits: {} → {} bytes",
            small.len(),
            huge.len()
        );
        // Escaped names fall under the same cap — this is the axis that made the number in the docs false in the previous two editions.
        let escaped = flood_refusal_of(40, 1 << 10, "\u{1b}");
        for (label, measured) in [("printable", small.len()), ("escaped", escaped.len())] {
            assert!(
                measured < 2048,
                "{label}: strayed from near the measured worst case (~1.4 KB) — one of the two caps came loose: {measured} bytes"
            );
        }
    }

    /// ★The eight named are **the first eight alphabetically**, in **alphabetical order**★ — not the eight typed first (the map is
    ///   sorted). The fixture inserts in **reverse** order,
    /// so a switch to an insertion-order map turns this red.
    /// ★Letting the names be truncated is the point★: only name cap and count cap applying **together** checks that they do not mask each other.
    #[test]
    fn the_named_subset_is_the_alphabetically_first_ones_in_order() {
        let message = flood_refusal(40, 1 << 8);

        let at = |name: &str| message.find(name);
        // Looking only at both ends would pass even if the middle six got mixed up — we look at all eight positions.
        let places: Vec<usize> = (0..MAX_NAMED_UNKNOWN)
            .map(|i| at(&format!("stray{i:04}")).expect("the eight named"))
            .collect();
        assert!(
            places.windows(2).all(|pair| pair[0] < pair[1]),
            "joins them in alphabetical order: {message}"
        );
        assert!(
            at("stray0008").is_none(),
            "from the ninth on, counted only by number: {message}"
        );
        assert!(
            at("stray0039").is_none(),
            "picks alphabetically, not by insertion order (the first field in reverse): {message}"
        );
        assert!(
            message.contains("truncated"),
            "in this combination the name cap also applies: {message}"
        );
    }

    /// Splits a truncated piece into `"body…"` + truncation marker — measuring what the body holds needs a boundary.
    fn literal_and_body(rendered: &str) -> (&str, &str) {
        let marker = rendered
            .find("(truncated")
            .expect("there must be a truncation marker");
        let literal = &rendered[..marker];
        let body = literal
            .trim_start_matches('"')
            .trim_end_matches('"')
            .trim_end_matches('…');
        (literal, body)
    }

    /// ★A truncated name must also be a **closed string literal**★ — else in a joined message the quote pairing goes out of step and the
    ///   one-field-read-as-two ambiguity `{:?}` removed returns.
    ///
    /// ★Three things are measured, all needed★: ① the literal **size** keeps the cap (catches a budget that forgets the quotes) ② no
    ///   **unescaped quote** in the body
    /// (catches a truncation branch that forgets to escape — hence the quote in the fixture name) ③ not truncated in the middle of an
    ///   escape. Missing any one lets defects of the old versions through.
    #[test]
    fn a_truncated_name_is_still_a_closed_string_literal() {
        let key = format!("a\"b{}", "c".repeat(300));
        let rendered = quoted_input(&key);
        let (literal, body) = literal_and_body(&rendered);

        assert!(rendered.starts_with('"'), "opening quote: {rendered}");
        assert!(
            rendered.contains("…\"(truncated"),
            "the closing quote comes **before** the truncation marker: {rendered}"
        );
        assert!(
            literal.len() <= MAX_QUOTED_INPUT_BYTES,
            "the literal exceeded the cap (the quote·ellipsis room was not taken out of the budget): {} bytes — {rendered}",
            literal.len()
        );
        assert!(
            rendered.contains(&format!("input was {} bytes", key.len())),
            "states what the number counts: {rendered}"
        );

        // ★Every quote in the body **must be escaped**★ — if even one is raw, that spot is read as a field boundary
        //   (counting alone would count `\"` as one too and miss this defect).
        let mut escaped = false;
        for ch in body.chars() {
            match ch {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => panic!("the body has an unescaped quote: {rendered}"),
                _ => {}
            }
        }

        // ★Checks **whether the last escape of the body is intact**★: `\u{1b}` has five mid-cut places and only "right after the
        //   backslash" is caught by backslash parity,
        //   so check that everything from the last `\` to the end is a **closed sequence**.
        let control = quoted_input(&"\u{1b}".repeat(200));
        let (control_literal, control_body) = literal_and_body(&control);
        if let Some(last) = control_body.rfind('\\') {
            let tail = &control_body[last..];
            let complete = match tail.chars().nth(1) {
                // `\u{…}` means one character only if it has the closing brace.
                Some('u') => tail.contains('}'),
                // A one-character escape (`\n`·`\\`·`\"` etc.) is intact if that one character is attached.
                Some(_) => true,
                // It ended with a backslash — a piece that escapes nothing.
                None => false,
            };
            assert!(
                complete,
                "the body ended in an incomplete escape (cut in the middle of a sequence): {control}"
            );
        }
        assert!(
            control_literal.len() <= MAX_QUOTED_INPUT_BYTES,
            "the literal of a control-character name exceeded the cap: {} bytes — {control}",
            control_literal.len()
        );
        assert!(
            control.contains("input was 200 bytes"),
            "the label states that the notation is escaped and the number is of the original: {control}"
        );
    }

    /// ★The cap must also be unrelated to the input **kind**★: truncating before escaping makes the same cap 128 B for printable and six
    ///   times that for control characters.
    #[test]
    fn an_escape_heavy_key_obeys_the_same_bound_as_a_printable_one() {
        let refusal_len = |key: String| {
            let mut given = serde_json::Map::new();
            given.insert(key, json!(1));
            entrance_table()
                .check_args("fixture.entrance", &serde_json::Value::Object(given))
                .expect_err("field not in the declaration")
                .message()
                .len()
        };

        let printable = refusal_len("k".repeat(4096));
        let escaped = refusal_len("\u{1b}".repeat(4096));

        assert!(
            escaped <= printable + 8,
            "a control-character name made a longer message than a printable name: {printable} → {escaped} bytes"
        );
    }

    /// ★It is caught before the handler runs★ — the screening is a pure up-front decision, so it gives the same answer whether or not
    ///   there is a body, and even if the body blows up
    /// (a handler that blows up asserts that).
    #[test]
    fn a_missing_required_field_is_refused_before_the_handler() {
        let mut table = CommandTable::new(DECLARED);
        table
            .insert(
                "fixture.entrance",
                blocking_handler(|_: NoArgs| -> Result<serde_json::Value, CommandError> {
                    panic!("arguments the check must not let through")
                }),
            )
            .expect("declared name");

        let err = table
            .check_args("fixture.entrance", &json!({ "label": "alpha" }))
            .expect_err("a required field is missing");

        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("cwd"),
            "points at the missing field: {}",
            err.message()
        );
    }

    #[test]
    fn a_declared_optional_field_may_be_absent() {
        let table = entrance_table();

        table
            .check_args("fixture.entrance", &json!({ "cwd": "C:/x" }))
            .expect("an omittable field may be absent");
        table
            .check_args(
                "fixture.entrance",
                &json!({ "cwd": "C:/x", "label": "alpha" }),
            )
            .expect("may be carried");
    }

    /// ★The rejection **always** carries the allowed set★ — a caller that received only 「not an object」 would have to try again
    /// without knowing what to carry (giving material to fix it itself is this screening's contract).
    #[test]
    fn a_non_object_refusal_also_carries_the_declared_set() {
        let err = entrance_table()
            .check_args("fixture.entrance", &json!("C:/x"))
            .expect_err("not an object");

        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("cwd") && err.message().contains("label"),
            "emits the allowed set too: {}",
            err.message()
        );
    }

    /// ★The rejection message quotes back the string the caller gave★ — without a cap, one huge field name would make a message of that size,
    /// and it would be carried in the error reply onto the wire.
    #[test]
    fn a_huge_unknown_key_is_quoted_within_a_bound() {
        let huge = "k".repeat(1 << 20);
        let mut given = serde_json::Map::new();
        given.insert("cwd".to_string(), json!("C:/x"));
        given.insert(huge, json!(1));

        let err = entrance_table()
            .check_args("fixture.entrance", &serde_json::Value::Object(given))
            .expect_err("field not in the declaration");

        // The single-name message is ~230 bytes (quote cap 128 + marker + declared set) — the numbers below are that plus margin, not a
        //   contract;
        //   the contract is "unrelated to input size" (`the_refusal_message_does_not_grow_with_the_body`).
        assert!(
            err.message().len() < 512,
            "a single 1 MiB name made a message that large: {} bytes",
            err.message().len()
        );
        assert!(
            err.message().contains("truncated"),
            "says it truncated: {}",
            err.message()
        );
    }

    /// ★The message must not learn the calling surface★ — this crate does not know which surface its rejection appears on. The per-surface
    /// guidance is added by the adapter through [`CommandError::set_message`], so if we invent it here the two pieces of guidance diverge.
    #[test]
    fn check_args_does_not_know_any_cli_vocabulary() {
        let table = entrance_table();
        let refusals = [
            table.check_args(
                "fixture.entrance",
                &json!({ "cwd": "C:/x", "target": "alpha" }),
            ),
            table.check_args("fixture.entrance", &json!({})),
            table.check_args("fixture.entrance", &json!("not an object")),
        ];

        for refusal in refusals {
            let message = refusal.expect_err("bounced").message().to_lowercase();
            for vocabulary in [
                "engram",
                "--",
                "flag",
                "cli",
                "command line",
                "usage",
                "help",
            ] {
                assert!(
                    !message.contains(vocabulary),
                    "vocabulary of the calling surface leaked ({vocabulary}): {message}"
                );
            }
        }
    }

    /// ★Another owner's name is not decided★ — that contract is not in this table. If rejected here, the envelope headed for the registry would
    /// die at the entry and two-stage delivery (TRD §3-8) would not hold.
    #[test]
    fn a_name_this_table_does_not_hold_is_left_to_the_next_step() {
        entrance_table()
            .check_args("tab.create", &json!({ "anything": 1 }))
            .expect("another's name passes");
    }

    /// ★The net is in the table too★ — even on a path that calls a handler directly outside routing (harness · assembly code), a panic
    /// must not swallow the reply. Without this assertion, `route`'s outer net would mask a defect at this spot.
    #[test]
    fn a_panicking_body_is_folded_into_an_error_without_route() {
        let panicking = blocking_handler(|_: NoArgs| -> Result<serde_json::Value, CommandError> {
            panic!("handler blew up")
        });

        let outcome = crate::testing::with_quiet_panic_hook(|| block_on(panicking.call(json!({}))));

        let err = outcome.expect_err("a panic becomes an error");
        assert_eq!(err.code(), ErrorCode::Internal);
        assert!(
            err.message().contains("handler blew up"),
            "the reason is carried"
        );
    }
}
