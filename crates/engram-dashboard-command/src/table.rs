//! Per-process handler table — 「내 표」 (my table) (TRD §3-3).

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
    /// The declaration's schema text (`args` or `ok`) is not JSON — it can be neither advertised nor used to fit
    /// arguments. ★Both fields are checked together★ — if either breaks, the registration packet's `help` breaks whole.
    /// (`lint_spec` catches it first in the declaration crate's tests, but assembly does not run carrying it either.)
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

/// The table of commands one process (strictly, one assembly unit) **can run directly**.
///
/// ★It is not a global static★ — `make_table(deps)` builds it at assembly and handlers are injected then (rule T-1).
/// So a harness can plug in fake dependencies and assert handlers without a process (ADR-0012).
// ADR-0155
pub struct CommandTable {
    declared: &'static [&'static CommandSpec],
    entries: BTreeMap<&'static str, Entry>,
}

impl CommandTable {
    /// `declared` = **the set the crate building this table declared** (`COMMAND_SPECS`, made by the declaration macro).
    /// It is not the whole binary ([`crate::command_specs`]) — putting another crate's names into my table must be blocked too.
    ///
    /// ★If the same name is declared twice, it panics here★ — blow up before assembly. A duplicate is a value
    /// decided by the build and does not change at runtime, and if left, the table and the advertised schema would point at different declarations.
    pub fn new(declared: &'static [&'static CommandSpec]) -> Self {
        for (i, spec) in declared.iter().enumerate() {
            if declared[i + 1..]
                .iter()
                .any(|other| other.name == spec.name)
            {
                panic!(
                    "command declared more than once in this crate: {} — 두 선언이 같은 이름을 쥐면 어느 계약이 이기는지 알 수 없다",
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
        // ★The table does not use `ok_schema`, but we check it here too★: since `spec_item_json` attaches it into `help`
        //   **as is** (`spec.rs`), a single broken `ok` makes the registration packet's `help` as a whole not JSON.
        //   Since it is right for the daemon to store that as an opaque string (ADR-0156), the breakage would only surface in front of the LLM or CLI reading that
        //   string — assembly is the last checkpoint. We do not use the value, so we discard it.
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

    /// Runs one name **by the advertised contract** — if it is not in the table, `None`, and `args` remains untouched
    /// (the next step, the registry, must carry it as is).
    ///
    /// ★The point is that there is no door that hands out the handler★: only if fitting arguments to the declared schema happens in this
    /// one place does 「광고한 것을 실제로 받는다」 (what is advertised is actually accepted) become a property of **the whole table**, not of each adapter
    /// ([`crate::coerce`]). So a handler that implements [`CommandHandler`] directly gets the same coercion.
    /// The net that folds body panics is not here — the caller ([`crate::route`]) covers both entry points (`call` · first poll)
    /// together.
    pub fn call(&self, name: &str, args: &mut serde_json::Value) -> Option<CommandFuture> {
        let entry = self.entries.get(name)?;
        crate::coerce::integral_numbers_to_integers(&entry.args_schema, args);
        crate::coerce::enum_words_to_declared_spelling(&entry.args_schema, args);
        Some(entry.handler.call(args.take()))
    }

    /// Checks **whether the arguments match the declaration** before the call — rejects unknown fields and missing required fields by naming them.
    ///
    /// ★The only place that calls it is the surface where what a person or LLM just typed comes in (ADR-0157)★. If it were called in inter-hop wiring,
    /// a new field sent by a caller on a newer version would hard-fail an old owner and additive evolution would die (TRD §4-③) —
    /// [`crate::route`] not calling it is the concrete form of that decision, and wedging it in there would break it.
    ///
    /// The rejection message carries **the wrong fields and the whole set of declared fields** together — the net is only worth something if the caller can fix it
    /// itself. ★Do not put the vocabulary of the calling surface in the message★: this crate does not know what that surface is, and
    /// its own guidance is added by the adapter through [`CommandError::set_message`].
    ///
    /// Three things pass through (places with no material to check):
    /// - **A name not in this table** — that contract is held by another, so there is no declaration here to compare against. ★Nobody behind it
    ///   checks on our behalf★: if the registry knows the name, delivery **forwards the envelope as is**, and the receiving hop's
    ///   [`crate::route`]→[`CommandTable::call`] does not screen either (that is the decision in the paragraph just above). So
    ///   if a person or LLM types another owner's name with a mistyped field, that field is caught nowhere, deserialization silently
    ///   drops it, and success is reported — the very failure ADR-0157 meant to block lives on one hop further along (the layering
    ///   of that decision is by **origin**, not by who owns the name).
    ///   ★This hole remains a duty of the entry adapter **pair** — the wiring wave decides which side★: the typing-side
    ///   adapter does not hold another's declaration (the registry's shape is an opaque string to the daemon — ADR-0156), and the owner-side
    ///   adapter cannot tell from the envelope alone whether it is what a person or LLM just typed. Which side gets that material
    ///   is a decision item, so this crate does not close it.
    /// - **A declaration that does not carry `properties`** — rejecting without knowing the allowed set would block perfectly good arguments.
    /// - **Below the top level** — this is a comparison against the declaration's property set, not a schema validator. A typo inside a nested object
    ///   passes, and that spot is caught by deserialization (if it is a declared field the type does not match, otherwise it is ignored).
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
        // Look at unknown fields first — a typo is caught together as 「모르는 칸 하나 + 빠진 필수 칸 하나」 (one unknown field + one missing required field), but what the caller
        //   should fix is **the very field it typed**.
        // ★Count **all** wrong fields★: if only one is pointed out, the caller fixes it, resends, and is rejected again, repeatedly
        //   (field order is alphabetical, so it is not 「먼저 친 것」 (the one typed first) either). Showing three typos at once is what the net is worth.
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

/// The allowed set the rejection message carries — not truncated. With a truncated list the caller cannot tell whether the name it failed to pick is
/// on the truncated side, so it cannot fix it itself (these names are **our declaration**, so the size is in our hands —
/// the string the caller gave is truncated by [`quoted_input`]).
fn declared_names(args_schema: &serde_json::Value) -> String {
    let Some(properties) = args_schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
    else {
        // A declaration that does not carry the allowed set — that case passes through (`check_args`), so what arrives here is only the case caught as
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
/// ★Count them all but do not carry them all★: the number of fields can grow as large as the request body (thousands), and without a cap the rejection
/// message grows by that much — and that cost is paid by the receiving side, not the typing side (same argument as [`quoted_input`]). If exceeded,
/// it **says how many more there are** (if it did not, the caller would read the list as everything and could not fix the remaining fields).
/// ★The eight named are **not** 「호출자가 먼저 친 여덟」 (the eight the caller typed first)★ — the argument map is a sorted data structure so they are the first
/// eight alphabetically, and that order was not made by the caller. To get 「먼저 친 것부터」 (from the one typed first), the map would have to be changed
/// to an insertion-order-preserving form, and that is a decision outside this crate (the test below pins this choice).
const MAX_NAMED_UNKNOWN: usize = 8;

/// The size cap (bytes) of one name's **literal** (`"…"` — including the opening and closing quotes and the ellipsis).
///
/// ★Without it, one 100 MiB field name would make a message of that size★ — that message is carried in the error reply onto the wire,
/// and the cost of making it is paid by the receiving side, not the side that typed the arguments. The value is just enough for a person to recognize their own typo.
/// ★What this number **measures and does not measure**★: it measures the literal, and the piece that actually goes into the message adds the truncation
/// marker (`(truncated, input was N bytes)` — a bit over 30 B) on top, reaching up to **~170 B**. If you need a count for the whole piece,
/// use that sum — do not copy this constant over into that place.
/// ★It is not a cap set in order to shrink things★: for input whose escaped notation is 129~158 B, the truncated form is actually
/// **longer** than the original notation (because the tag is attached). What this cap guards is not the average size but **the finiteness of the tail**.
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

/// Trims a name the caller gave so it can be quoted in the message — when it truncates it **says it truncated** (otherwise the caller
/// would read the truncated name as the name it typed and go fix a perfectly good field).
///
/// ★The name is wrapped in Rust string-literal notation★: if wrapped in single quotes, **one** field `a', 'b` would be printed as `'a', 'b'`
/// and read as **two wrong fields** — the caller would go fix fields that do not even exist. `{:?}` escapes quotes, backslashes and control
/// characters and removes that ambiguity.
/// ★A truncated name also **closes its quote** (load-bearing)★: if it did not, in a message joining several names the quote pairing
/// would go out of step and the very ambiguity just removed would come back (a piece with no closing quote + the next name's opening quote would
/// read as a pair).
/// ★The cap is applied **after escaping**★: if truncated first, one control character would swell to six bytes as `\u{1b}` and the message
/// would grow up to six times the cap — the cap would vary 「입력 종류에 따라」 (by input kind) and the number written in the docs would become false.
/// ★Do not truncate in the middle of an escape sequence★: it accumulates character by character and stops at the budget, so no piece
/// with only a lone `\` is produced — such a piece has broken notation and would violate the 「리터럴 표기」 (literal notation) promise above.
/// ★The truncation marker states **what the number counts**★: what is visible is the escaped notation while the number is original bytes, so without a label
/// 30 control characters (180 characters in notation) would read as 「30 바이트를 잘랐다」 (truncated 30 bytes) and the two would look inconsistent.
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
        // ★The body runs inside the future (not inside `call`)★: if `call` finished the work and returned a future holding only the result,
        //   `timeout(d, handler.call(args))` would **never fire** — the deadline (TRD §4-⑥)
        //   cannot stand on this shape. Before polling, cancellation really is cancellation.
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
/// ★Blocking contract (for the caller to keep)★: the body **runs to the end on the first poll**. If the assembly code polls it on an async runtime
/// thread, it blocks that thread — the agent control verbs write to disk while holding the profile lock and
/// poll for early resume exit (the same contract in daemon `control/commands.rs`), so it must be called behind `spawn_blocking`.
/// Argument deserialization failure = `INVALID_ARGUMENT` · body panic = `INTERNAL` (it does not end without a reply).
/// ★The coercion between schema and serde is not here★ — [`CommandTable::call`] does it by looking at the declaration, so putting it
/// back into this adapter would give the two places different rules.
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
        summary: "  두 칸 들여쓴 요약  ",
        args_schema: "{\"type\":\"object\",\"properties\":{}}",
        ok_schema: "{\"type\":\"object\",\"properties\":{\"pong\":{\"type\":\"boolean\"}}}",
        errors: &[ErrorCode::Internal],
        args_type: "PingArgs",
        ok_type: "PingOk",
        agent_args: &[],
    };

    /// A declaration holding both integer and real fields — checks here that the table fits arguments by looking at the schema.
    static NUMBERS: CommandSpec = CommandSpec {
        name: "fixture.numbers",
        effect: Effect::Write,
        since: 1,
        summary: "수치 칸",
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
    };
    /// A declaration holding both required and omittable fields — checks here that the entry screening tells the two apart.
    static ENTRANCE: CommandSpec = CommandSpec {
        name: "fixture.entrance",
        effect: Effect::Write,
        since: 1,
        summary: "입구 검문 픽스처",
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
        table.insert("fixture.ping", handler()).expect("첫 삽입");
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
            summary: "같은 이름, 다른 계약",
            args_schema: "{\"type\":\"object\",\"properties\":{}}",
            ok_schema: "{\"type\":\"object\",\"properties\":{}}",
            errors: &[],
            args_type: "OtherPingArgs",
            ok_type: "OtherPingOk",
            agent_args: &[],
        };
        static COLLIDING: &[&CommandSpec] = &[&PING, &OTHER_PING];

        let outcome = crate::testing::with_quiet_panic_hook(|| {
            std::panic::catch_unwind(|| CommandTable::new(COLLIDING))
        });

        assert!(outcome.is_err(), "중복 선언은 표 생성에서 터진다");
    }

    #[test]
    fn unknown_lookup_is_none() {
        let mut table = CommandTable::new(DECLARED);
        assert!(!table.contains("fixture.ping"), "아직 안 꽂혔다");
        assert!(table.call("fixture.ping", &mut json!({})).is_none());

        table.insert("fixture.ping", handler()).expect("삽입");
        assert!(!table.contains("nope"));
        let mut args = json!({ "kept": 1 });
        assert!(table.call("nope", &mut args).is_none());
        assert_eq!(args, json!({ "kept": 1 }), "못 찾으면 인자를 안 가져간다");
    }

    /// ★Assembly does not run carrying a declaration it cannot advertise★ — if the schema text is not JSON, the registration packet's
    /// `help` breaks too and there is no material to fit arguments either.
    #[test]
    fn a_declaration_whose_schema_is_not_json_is_refused_at_assembly() {
        static BROKEN: CommandSpec = CommandSpec {
            name: "fixture.broken",
            effect: Effect::Read,
            since: 1,
            summary: "깨진 스키마",
            args_schema: "{not json",
            ok_schema: "{\"type\":\"object\",\"properties\":{}}",
            errors: &[],
            args_type: "BrokenArgs",
            ok_type: "BrokenOk",
            agent_args: &[],
        };
        static WITH_BROKEN: &[&CommandSpec] = &[&BROKEN];

        let mut table = CommandTable::new(WITH_BROKEN);
        assert_eq!(
            table.insert("fixture.broken", handler()),
            Err(TableError::InvalidSchema("fixture.broken"))
        );
        assert!(table.is_empty());
    }

    /// ★The same holds when `ok` is broken★ — the table does not use `ok_schema`, but `spec_item_json` attaches it into `help` as is,
    /// so looking only at the argument schema would let **a registration packet carrying a broken `help` through**. The daemon is right to store it as an opaque
    /// string (ADR-0156), so by then there is no place left to catch it.
    #[test]
    fn a_declaration_whose_ok_schema_is_not_json_is_refused_at_assembly() {
        static BROKEN_OK: CommandSpec = CommandSpec {
            name: "fixture.broken-ok",
            effect: Effect::Read,
            since: 1,
            summary: "인자는 멀쩡하고 반환이 깨졌다",
            args_schema: "{\"type\":\"object\",\"properties\":{}}",
            ok_schema: "{not json",
            errors: &[],
            args_type: "BrokenOkArgs",
            ok_type: "BrokenOkOk",
            agent_args: &[],
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
        table.insert("fixture.ping", handler()).expect("삽입");

        let decls = table.decls();
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "fixture.ping");
        assert_eq!(decls[0].help, crate::spec_item_json(&PING));
        // It must have **the same origin** as the elements of the derived file (ADR-0156).
        let parsed: serde_json::Value = serde_json::from_str(&decls[0].help).expect("help 는 JSON");
        assert_eq!(parsed["name"], "fixture.ping");
        assert_eq!(parsed["effect"], "Read");
        assert_eq!(parsed["summary"], "두 칸 들여쓴 요약");
        assert_eq!(parsed["ok"]["properties"]["pong"]["type"], "boolean");
        assert_eq!(parsed["errors"][0], "INTERNAL");
        // The common errors the table adds are in the advertisement (they are not in the declaration).
        assert_eq!(parsed["errors"][1], "INVALID_ARGUMENT");
    }

    /// ★Confirms in bytes the claim that the disk file and the registration packet have **the same single origin**★ (ADR-0156).
    ///
    /// If we compared the two paths against each other, it would be an identity that calls the same function twice, so **it passes even if both drift together** — so
    /// we put a fixed expected byte string in the middle.
    #[test]
    fn the_schema_file_element_is_byte_identical_to_help() {
        const EXPECTED: &str = concat!(
            r#"{"name":"fixture.ping","effect":"Read","since":1,"summary":"두 칸 들여쓴 요약","#,
            r#""args":{"type":"object","properties":{}},"#,
            r#""ok":{"type":"object","properties":{"pong":{"type":"boolean"}}},"#,
            r#""errors":["INTERNAL","INVALID_ARGUMENT"]}"#
        );

        let mut table = CommandTable::new(DECLARED);
        table.insert("fixture.ping", handler()).expect("삽입");
        let help = table.decls().remove(0).help;

        assert_eq!(help, EXPECTED, "등록 패킷의 help");
        assert!(
            crate::catalog_json(1, DECLARED).contains(EXPECTED),
            "파생 파일의 원소"
        );
    }

    /// ★The marking goes out in a separate field, not in `help`★ (ADR-0308) — the daemon does not open `help`.
    #[test]
    fn decls_carry_the_agent_reference_marks_beside_the_help() {
        static MARKED: CommandSpec = CommandSpec {
            name: "fixture.marked",
            effect: Effect::Write,
            since: 1,
            summary: "표시 픽스처",
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
        };
        static MARKED_DECLARED: &[&CommandSpec] = &[&MARKED, &PING];

        let mut table = CommandTable::new(MARKED_DECLARED);
        table.insert("fixture.marked", handler()).expect("삽입");
        table.insert("fixture.ping", handler()).expect("삽입");

        let decls = table.decls();
        let marked = decls
            .iter()
            .find(|d| d.name == "fixture.marked")
            .expect("표시한 명령");
        assert_eq!(marked.agent_args, ["target", "after"], "선언 순서 그대로");
        assert_eq!(
            marked.help,
            crate::spec_item_json(&MARKED),
            "help 는 표시와 무관하다"
        );
        assert!(!marked.help.contains("agent_args"));
        let plain = decls
            .iter()
            .find(|d| d.name == "fixture.ping")
            .expect("표시 없는 명령");
        assert!(plain.agent_args.is_empty());
    }

    #[test]
    fn bad_arguments_become_invalid_argument() {
        let table_handler = handler();
        let outcome = block_on(table_handler.call(json!({ "unexpected": [1, 2] })));
        // Unknown fields are ignored (we do not attach deny_unknown_fields — additive rule) and it succeeds.
        assert!(outcome.is_ok());

        let outcome = block_on(table_handler.call(json!("not an object")));
        assert_eq!(
            outcome.expect_err("객체가 아니면 반려").code(),
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

    /// ★A handler that does not go through the adapter★ — if the coercion were a property of `blocking_handler`, this handler would advertise the same schema
    /// and accept input differently. So we build the coercion test on this shape (the `await`ing commands of Step 3 are
    /// this shape).
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
            .expect("선언된 이름");
        table
    }

    fn call_numbers(mut args: serde_json::Value) -> Result<serde_json::Value, CommandError> {
        let table = numbers_table();
        let future = table
            .call("fixture.numbers", &mut args)
            .expect("표에 있는 이름");
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
        .expect("스키마가 광고한 대로 받는다");

        assert_eq!(outcome["count"], 2);
        assert_eq!(outcome["depth"], -3);
        assert_eq!(outcome["ratio"], 1.0, "실수 칸은 실수로 남는다");
        assert_eq!(outcome["size"], 4, "중첩·배열 안까지 같다");
    }

    #[test]
    fn a_fractional_value_is_still_refused_for_an_integer_field() {
        let outcome = call_numbers(json!({
            "count": 2.5, "depth": 0, "ratio": 1.0, "rows": [],
        }));

        assert_eq!(
            outcome.expect_err("소수부가 있으면 정수가 아니다").code(),
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
            outcome.expect_err("정수 칸에 담을 수 없다").code(),
            ErrorCode::InvalidArgument
        );
    }

    // ── Entry screening (ADR-0157) ──────────────────────────────────────────────────────────────────

    fn entrance_table() -> CommandTable {
        let mut table = CommandTable::new(DECLARED);
        table
            .insert("fixture.entrance", handler())
            .expect("선언된 이름");
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
            .expect_err("선언에 없는 칸");

        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("target"),
            "틀린 칸을 지목한다: {}",
            err.message()
        );
        assert!(
            err.message().contains("cwd") && err.message().contains("label"),
            "허용 집합을 함께 낸다: {}",
            err.message()
        );
    }

    /// ★If there are several wrong fields, all of them come out★ — if only one is pointed out, the caller repeats reject, fix, retry as many
    /// times as there are typos. Field order is alphabetical, so there is no meaning of 「처음 친 칸」 (the field typed first) either.
    #[test]
    fn every_undeclared_argument_field_is_named_not_just_the_first() {
        let err = entrance_table()
            .check_args(
                "fixture.entrance",
                &json!({ "cwd": "C:/x", "target": "alpha", "zzz": 1, "aaa": 2 }),
            )
            .expect_err("선언에 없는 칸 셋");

        for offender in ["target", "zzz", "aaa"] {
            assert!(
                err.message().contains(offender),
                "{offender} 이 빠졌다: {}",
                err.message()
            );
        }
    }

    /// A rejection message carrying `count` fields absent from the declaration (each `key_bytes` bytes).
    ///
    /// ★Insert in **reverse alphabetical order**★: if inserted ascending, insertion order and alphabetical order coincide, so even if the map changes
    /// to an insertion-order-preserving form (`preserve_order`) the 「사전순 앞 여덟」 (first eight alphabetically) assertion still passes — and nobody would
    /// know the property that assertion meant to measure is gone.
    fn flood_refusal(count: usize, key_bytes: usize) -> String {
        flood_refusal_of(count, key_bytes, "x")
    }

    /// `fill` = the character that fills the name — kept separate so we can measure whether printable characters and escaped characters fall under the same cap
    /// (escape expansion is an axis that can make the cap differ by kind).
    fn flood_refusal_of(count: usize, key_bytes: usize, fill: &str) -> String {
        let mut given = serde_json::Map::new();
        given.insert("cwd".to_string(), json!("C:/x"));
        for i in (0..count).rev() {
            given.insert(format!("stray{i:04}{}", fill.repeat(key_bytes)), json!(1));
        }
        entrance_table()
            .check_args("fixture.entrance", &serde_json::Value::Object(given))
            .expect_err("선언에 없는 칸 다수")
            .message()
            .to_string()
    }

    /// If the count cap is exceeded it says **how many more there are** — if the caller thinks the list is everything it cannot fix the remaining fields.
    #[test]
    fn a_flood_of_unknown_fields_is_capped_and_says_how_many_are_left() {
        let message = flood_refusal(40, 0);

        assert!(
            message.contains("(+32 more)"),
            "남은 개수를 말한다: {message}"
        );
        assert!(
            message.contains("stray0000"),
            "이름 지어진 칸이 실제로 실린다: {message}"
        );
    }

    /// ★The message length is **unrelated** to the request size★ — the property to keep is not 「몇 바이트 이하」 (at most so many bytes) but 「입력이 커져도
    /// 안 자란다」 (does not grow as the input grows). The cap on one name is held by [`quoted_input`] and the cap on the number of names by [`MAX_NAMED_UNKNOWN`], and
    /// only with both standing does this property stand (with only one, it grows along the other axis).
    ///
    /// The measured worst case is **about 1.4 KB** (eight names × quote cap 128B + truncation marker + declared set), and an escaped
    /// name gives the same value (since the cap applies **after** escaping) — still, do not read that number as a contract. The contract is
    /// the first assertion below (even if the input grows, the message grows only by the number of digits).
    #[test]
    fn the_refusal_message_does_not_grow_with_the_body() {
        let small = flood_refusal(40, 1 << 10);
        let huge = flood_refusal(4_000, 1 << 14);

        // The width allowed to grow = **the number of digits only**: the 「N 바이트」 (N bytes) notation of the eight names and the number in 「+N more」 get longer.
        //   The margin of 32 is sized to cover that growth in digits (even if the input is 1000 times larger, decimal digits grow by only a few).
        assert!(
            huge.len() <= small.len() + 32,
            "칸 수 100배·이름 크기 16배인데 문구가 자릿수 이상으로 자랐다: {} → {} 바이트",
            small.len(),
            huge.len()
        );
        // Escaped names fall under the same cap — this is the axis that made the number in the docs false in the previous two editions.
        let escaped = flood_refusal_of(40, 1 << 10, "\u{1b}");
        for (label, measured) in [("printable", small.len()), ("escaped", escaped.len())] {
            assert!(
                measured < 2048,
                "{label}: 실측 최악(~1.4 KB) 근처를 벗어났다 — 상한 둘 중 하나가 풀렸다: {measured} 바이트"
            );
        }
    }

    /// ★The eight named are **the first eight alphabetically**, and the **order carried is also alphabetical**★ — they are not the eight the caller
    /// typed first (the argument map is a sorted data structure). The fixture inserts in **reverse** alphabetical order, so if the map changes to an insertion-order-
    /// preserving form this turns red.
    /// ★Letting the names be truncated is the point★: only a combination where the name cap and the count cap apply **together** checks that the two do not
    /// mask each other (with only short names, the truncation path never runs once in this combination).
    #[test]
    fn the_named_subset_is_the_alphabetically_first_ones_in_order() {
        let message = flood_refusal(40, 1 << 8);

        let at = |name: &str| message.find(name);
        // Looking only at both ends would pass even if the middle six got mixed up — we look at all eight positions.
        let places: Vec<usize> = (0..MAX_NAMED_UNKNOWN)
            .map(|i| at(&format!("stray{i:04}")).expect("이름 지어진 여덟"))
            .collect();
        assert!(
            places.windows(2).all(|pair| pair[0] < pair[1]),
            "사전순 차례로 이어 붙인다: {message}"
        );
        assert!(
            at("stray0008").is_none(),
            "아홉째부터는 개수로만 센다: {message}"
        );
        assert!(
            at("stray0039").is_none(),
            "삽입 순서(역순 첫 칸)가 아니라 사전순으로 고른다: {message}"
        );
        assert!(
            message.contains("truncated"),
            "이 조합에서 이름 상한도 함께 걸린다: {message}"
        );
    }

    /// Splits a truncated piece into `"본문…"` + truncation marker — measuring what the body holds needs a boundary.
    fn literal_and_body(rendered: &str) -> (&str, &str) {
        let marker = rendered
            .find("(truncated")
            .expect("잘림 표시가 있어야 한다");
        let literal = &rendered[..marker];
        let body = literal
            .trim_start_matches('"')
            .trim_end_matches('"')
            .trim_end_matches('…');
        (literal, body)
    }

    /// ★A truncated name must also be a **closed string literal**★ — if in a message joining several names the quote pairing goes
    /// out of step, the ambiguity `{:?}` was meant to remove (one field read as two) comes right back.
    ///
    /// ★Three things are measured, and all three are needed★: ① whether the literal **size** keeps the cap (if the budget forgets the room for the opening and closing quotes,
    /// this catches it) ② whether the body has no **unescaped quote** (if the truncation branch forgets to escape,
    /// this catches it — which is why the fixture name contains a quote) ③ whether it was not truncated in the middle of an escape.
    /// If any one of the three is missing, the defects that the old versions of this function actually produced pass through.
    #[test]
    fn a_truncated_name_is_still_a_closed_string_literal() {
        let key = format!("a\"b{}", "c".repeat(300));
        let rendered = quoted_input(&key);
        let (literal, body) = literal_and_body(&rendered);

        assert!(rendered.starts_with('"'), "여는 따옴표: {rendered}");
        assert!(
            rendered.contains("…\"(truncated"),
            "닫는 따옴표가 잘림 표시 **앞**에 온다: {rendered}"
        );
        assert!(
            literal.len() <= MAX_QUOTED_INPUT_BYTES,
            "리터럴이 상한을 넘었다(따옴표·말줄임표 자리를 예산에서 안 뺐다): {} 바이트 — {rendered}",
            literal.len()
        );
        assert!(
            rendered.contains(&format!("input was {} bytes", key.len())),
            "무엇을 센 수인지 밝힌다: {rendered}"
        );

        // ★Every quote in the body **must be escaped**★ — if even one is raw, that spot is read as a field boundary
        //   (counting alone would count `\"` as one too and miss this defect).
        let mut escaped = false;
        for ch in body.chars() {
            match ch {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => panic!("본문에 이스케이프 안 된 따옴표가 있다: {rendered}"),
                _ => {}
            }
        }

        // ★Checks **whether the last escape of the body is intact**★: `\u{1b}` is six characters so there are five places to be cut in the middle, and of those only 「`\` 바로 뒤」 (right after the backslash) is
        //   caught by backslash parity. The other four (`\u`·`\u{`·`\u{1`·
        //   `\u{1b`) pass the parity check as is, so we check whether everything from the last `\` to the end is a **closed sequence**.
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
                "본문이 미완성 이스케이프로 끝났다(시퀀스 한가운데서 잘렸다): {control}"
            );
        }
        assert!(
            control_literal.len() <= MAX_QUOTED_INPUT_BYTES,
            "제어문자 이름의 리터럴이 상한을 넘었다: {} 바이트 — {control}",
            control_literal.len()
        );
        assert!(
            control.contains("input was 200 bytes"),
            "표기는 이스케이프된 것이고 수는 원본이라는 것을 라벨이 밝힌다: {control}"
        );
    }

    /// ★The cap must also be unrelated to the input **kind**★: if truncated before escaping, one control character swells to six bytes,
    /// and the same cap becomes 128 B for printable characters and six times that for control characters — the number written in the docs
    /// becomes false at that moment.
    #[test]
    fn an_escape_heavy_key_obeys_the_same_bound_as_a_printable_one() {
        let refusal_len = |key: String| {
            let mut given = serde_json::Map::new();
            given.insert(key, json!(1));
            entrance_table()
                .check_args("fixture.entrance", &serde_json::Value::Object(given))
                .expect_err("선언에 없는 칸")
                .message()
                .len()
        };

        let printable = refusal_len("k".repeat(4096));
        let escaped = refusal_len("\u{1b}".repeat(4096));

        assert!(
            escaped <= printable + 8,
            "제어문자 이름이 인쇄 가능 이름보다 긴 문구를 만들었다: {printable} → {escaped} 바이트"
        );
    }

    /// ★It is caught before the handler runs★ — the screening is a pure up-front decision, so it gives the same answer whether or not there is a body, and even if the body blows up
    /// (a handler that blows up asserts that).
    #[test]
    fn a_missing_required_field_is_refused_before_the_handler() {
        let mut table = CommandTable::new(DECLARED);
        table
            .insert(
                "fixture.entrance",
                blocking_handler(|_: NoArgs| -> Result<serde_json::Value, CommandError> {
                    panic!("검문이 통과시키면 안 되는 인자다")
                }),
            )
            .expect("선언된 이름");

        let err = table
            .check_args("fixture.entrance", &json!({ "label": "alpha" }))
            .expect_err("필수 칸이 빠졌다");

        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("cwd"),
            "빠진 칸을 지목한다: {}",
            err.message()
        );
    }

    #[test]
    fn a_declared_optional_field_may_be_absent() {
        let table = entrance_table();

        table
            .check_args("fixture.entrance", &json!({ "cwd": "C:/x" }))
            .expect("생략 가능한 칸은 없어도 된다");
        table
            .check_args(
                "fixture.entrance",
                &json!({ "cwd": "C:/x", "label": "alpha" }),
            )
            .expect("실어도 된다");
    }

    /// ★The rejection **always** carries the allowed set★ — a caller that received only 「객체가 아니다」 (not an object) would have to try again
    /// without knowing what to carry (giving material to fix it itself is this screening's contract).
    #[test]
    fn a_non_object_refusal_also_carries_the_declared_set() {
        let err = entrance_table()
            .check_args("fixture.entrance", &json!("C:/x"))
            .expect_err("객체가 아니다");

        assert_eq!(err.code(), ErrorCode::InvalidArgument);
        assert!(
            err.message().contains("cwd") && err.message().contains("label"),
            "허용 집합을 함께 낸다: {}",
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
            .expect_err("선언에 없는 칸");

        // The measured size of a single-name message is ~230 bytes (quote cap 128 + truncation marker + declared set) — the numbers below are
        //   that measurement plus margin, not a contract. The contract is 「입력 크기와 무관」 (unrelated to input size) and
        //   `the_refusal_message_does_not_grow_with_the_body` measures it.
        assert!(
            err.message().len() < 512,
            "1 MiB 이름 하나가 그만한 문구를 만들었다: {} 바이트",
            err.message().len()
        );
        assert!(
            err.message().contains("truncated"),
            "잘랐다고 말한다: {}",
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
            let message = refusal.expect_err("반려").message().to_lowercase();
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
                    "호출 표면의 어휘가 샜다({vocabulary}): {message}"
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
            .expect("남의 이름은 통과한다");
    }

    /// ★The net is in the table too★ — even on a path that calls a handler directly outside routing (harness · assembly code), a panic
    /// must not swallow the reply. Without this assertion, `route`'s outer net would mask a defect at this spot.
    #[test]
    fn a_panicking_body_is_folded_into_an_error_without_route() {
        let panicking = blocking_handler(|_: NoArgs| -> Result<serde_json::Value, CommandError> {
            panic!("handler blew up")
        });

        let outcome = crate::testing::with_quiet_panic_hook(|| block_on(panicking.call(json!({}))));

        let err = outcome.expect_err("패닉은 오류가 된다");
        assert_eq!(err.code(), ErrorCode::Internal);
        assert!(err.message().contains("handler blew up"), "사유가 실린다");
    }
}
