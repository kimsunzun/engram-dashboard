//! Actually runs **the allowed type alphabet** of the declaration macro (an unconfirmed item of TRD §2-2 3).
//!
//! Covers branches `agent.*` does not use (integers · floats · bool · enum · `Option<Vec<T>>`) — whether real use exceeds the alphabet is
//! unknowable before declaring, and compilation must stop the moment it does.
//! ★The names declared here are harness fixtures, not commands★ (`fixture.*` — plugged into no table).

use engram_dashboard_command::declare_commands;

declare_commands! {
    catalog_version: 7;

    struct Sample {
        flag: bool,
        count: u32,
        offset: i64,
        ratio: f64,
        tags: Vec<String>,
    }

    enum Mode {
        Fast,
        Slow,
    }

    /// An answer row with a free-form payload — the shape a notice row (`code` + its `args`) takes.
    struct Note {
        code: String,
        args: Json,
    }

    #[effect(Write)]
    #[since(3)]
    "fixture.alphabet" => args AlphabetArgs {
        text: String,
        flag: bool,
        count: u32,
        offset: i64,
        big: u64,
        ratio: f64,
        maybe: Option<String>,
        many: Vec<String>,
        maybe_many: Option<Vec<String>>,
        nested: Sample,
        nested_many: Vec<Sample>,
        mode: Mode,
    } -> ok AlphabetOk {
        echoed: String,
        notes: Vec<Note>,
    } errors [INVALID_ARGUMENT, UNSUPPORTED];

    /// Agent reference marker (ADR-0308) — only the marked field names go out as `agent_args`, and the schema and type stay the same.
    #[effect(Write)]
    #[since(1)]
    "fixture.agentRefs" => args AgentRefsArgs {
        #[agent_ref]
        target: String,
        label: String,
        #[agent_ref]
        after: Option<String>,
        maybe: Option<String>,
        nested: Sample,
        reparent: Option<Option<String>>,
        many: Vec<String>,
        maybe_many: Option<Vec<String>>,
    } -> ok AgentRefsOk {
    } errors [NOT_FOUND];
}

fn args_schema() -> serde_json::Value {
    serde_json::from_str(AlphabetArgs::SPEC.args_schema).expect("the args schema is JSON")
}

#[test]
fn primitives_map_to_json_types() {
    let schema = args_schema();
    let props = &schema["properties"];
    assert_eq!(props["text"]["type"], "string");
    assert_eq!(props["flag"]["type"], "boolean");
    assert_eq!(props["count"]["type"], "integer");
    assert_eq!(props["offset"]["type"], "integer");
    assert_eq!(props["ratio"]["type"], "number");
}

/// ★Integer fields advertise their real range★ — without an upper bound, `18446744073709551616` would pass the advertisement but fail
/// deserialization (a lie to callers filling arguments by it). The 64-bit boundary is rounded by one step in validators reading double
///   precision, still narrower than 「none」.
#[test]
fn integer_fields_advertise_their_range() {
    let schema = args_schema();
    let props = &schema["properties"];

    assert_eq!(props["count"]["minimum"], 0);
    assert_eq!(props["count"]["maximum"], 4294967295u32);

    assert_eq!(props["offset"]["minimum"], i64::MIN);
    assert_eq!(props["offset"]["maximum"], i64::MAX);

    assert_eq!(props["big"]["minimum"], 0);
    assert_eq!(props["big"]["maximum"], u64::MAX);
}

#[test]
fn option_is_nullable_and_vec_is_an_array() {
    let schema = args_schema();
    let props = &schema["properties"];
    assert_eq!(props["maybe"]["anyOf"][0]["type"], "string");
    assert_eq!(props["maybe"]["anyOf"][1]["type"], "null");
    assert_eq!(props["many"]["type"], "array");
    assert_eq!(props["many"]["items"]["type"], "string");
    assert_eq!(props["maybe_many"]["anyOf"][0]["type"], "array");
    assert_eq!(props["maybe_many"]["anyOf"][1]["type"], "null");
}

#[test]
fn block_declared_types_are_inlined_not_referenced() {
    let schema = args_schema();
    let nested = &schema["properties"]["nested"];
    assert_eq!(nested["type"], "object");
    assert_eq!(nested["properties"]["ratio"]["type"], "number");
    assert_eq!(nested["properties"]["tags"]["items"]["type"], "string");

    let nested_many = &schema["properties"]["nested_many"];
    assert_eq!(
        nested_many["items"]["properties"]["count"]["type"],
        "integer"
    );

    assert_eq!(schema["properties"]["mode"]["enum"][0], "Fast");
    assert_eq!(schema["properties"]["mode"]["enum"][1], "Slow");
}

/// A declared struct is a real Rust type and round-trips over the wire — a schema without a type is useless.
#[test]
fn declared_types_round_trip_through_serde() {
    let value = serde_json::json!({
        "text": "hi",
        "flag": true,
        "count": 2,
        "offset": -3,
        "big": 7,
        "ratio": 0.5,
        "maybe": null,
        "many": ["a"],
        "maybe_many": null,
        "nested": { "flag": false, "count": 1, "offset": 0, "ratio": 1.0, "tags": [] },
        "nested_many": [],
        "mode": "Slow"
    });
    let parsed: AlphabetArgs = serde_json::from_value(value.clone()).expect("deserialize");
    assert_eq!(parsed.mode, Mode::Slow);
    assert_eq!(serde_json::to_value(&parsed).expect("serialize"), value);
}

/// `Option` fields may be omitted entirely — so LLM callers need not fill fields they do not use.
#[test]
fn absent_option_fields_are_none() {
    let value = serde_json::json!({
        "text": "hi",
        "flag": true,
        "count": 2,
        "offset": -3,
        "big": 7,
        "ratio": 0.5,
        "many": [],
        "nested": { "flag": false, "count": 1, "offset": 0, "ratio": 1.0, "tags": [] },
        "nested_many": [],
        "mode": "Fast"
    });
    let parsed: AlphabetArgs = serde_json::from_value(value).expect("deserialize");
    assert!(parsed.maybe.is_none());
    assert!(parsed.maybe_many.is_none());
}

// ── Free-form JSON field (`Json`) ───────────────────────────────────────────────────────

/// `Json` advertises "any value" and stays required — the schema must not promise an omission that
/// deserialization then refuses.
#[test]
fn a_json_field_advertises_any_value_and_stays_required() {
    let schema: serde_json::Value =
        serde_json::from_str(AlphabetArgs::SPEC.ok_schema).expect("ok schema is JSON");
    let row = &schema["properties"]["notes"]["items"];
    assert_eq!(row["properties"]["args"], serde_json::json!({}));
    assert_eq!(row["required"], serde_json::json!(["code", "args"]));
    engram_dashboard_command::lint_spec(&AlphabetArgs::SPEC).expect("lint");

    let missing = serde_json::json!({"echoed": "x", "notes": [{"code": "c"}]});
    assert!(serde_json::from_value::<AlphabetOk>(missing).is_err());
}

/// The payload goes out and comes back as given — nested objects and arrays included.
#[test]
fn a_json_field_round_trips_its_payload_verbatim() {
    let value = serde_json::json!({
        "echoed": "x",
        "notes": [
            {"code": "unsupported-line", "args": {"item": "a/b", "line": 3, "names": ["k", "v"]}},
            {"code": "empty", "args": {}}
        ]
    });
    let parsed: AlphabetOk = serde_json::from_value(value.clone()).expect("deserialize");
    assert_eq!(parsed.notes[0].args["names"][1], "v");
    assert_eq!(serde_json::to_value(&parsed).expect("serialize"), value);
}

/// The TS binding names the payload `unknown` — the reader narrows it by `code`.
#[test]
fn a_json_field_is_unknown_in_typescript() {
    use ts_rs::TS as _;

    let decl = Note::decl();
    assert!(decl.contains("args: unknown"), "{decl}");
}

#[test]
fn catalog_version_is_per_block() {
    assert_eq!(CATALOG_VERSION, 7);
    assert_eq!(COMMAND_SPECS.len(), 2);
    assert_eq!(AlphabetArgs::SPEC.since, 3);
}

// ── Agent reference marker (ADR-0308) ───────────────────────────────────────────────────────

#[test]
fn marked_fields_become_agent_args_in_declaration_order() {
    assert_eq!(AgentRefsArgs::SPEC.agent_args, ["target", "after"]);
    assert!(
        AlphabetArgs::SPEC.agent_args.is_empty(),
        "a declaration without markers gives an empty list"
    );
}

#[test]
fn the_mark_leaves_the_schema_and_the_type_alone() {
    let schema: serde_json::Value =
        serde_json::from_str(AgentRefsArgs::SPEC.args_schema).expect("the args schema is JSON");
    let props = &schema["properties"];
    assert_eq!(props["target"], props["label"]);
    assert_eq!(props["after"], props["maybe"]);
    assert_eq!(
        schema["required"],
        serde_json::json!(["target", "label", "nested", "reparent", "many"])
    );

    let value = serde_json::json!({
        "target": "lead",
        "label": "x",
        "after": "worker",
        "nested": { "flag": false, "count": 1, "offset": 0, "ratio": 1.0, "tags": [] },
        "reparent": null,
        "many": []
    });
    let parsed: AgentRefsArgs = serde_json::from_value(value).expect("deserialize");
    assert_eq!(parsed.target, "lead");
    assert_eq!(parsed.after.as_deref(), Some("worker"));
    assert_eq!(parsed.reparent, Some(None));
}
