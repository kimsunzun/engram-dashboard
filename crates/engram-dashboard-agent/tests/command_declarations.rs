//! Declaration golden — pins the name set and the error set so they do not silently diverge (TRD §7 「declaration derivation」).

use engram_dashboard_agent::commands::{
    AgentCancelQueuedInputArgs, AgentInterruptArgs, AgentListArgs, AgentListQueuedInputsArgs,
    AgentMoveArgs, AgentNewArgs, AgentRenameArgs, AgentResolveArgs, AgentSpawnArgs, UsageGetArgs,
    UsageRefreshArgs, COMMAND_SPECS, INPUT_AFFECTING,
};
use engram_dashboard_command::{
    command_specs, duplicate_command_names, lint_spec, spec_item_json, spec_of, Effect, ErrorCode,
    COMMON_ERRORS,
};
use serde_json::json;

#[test]
fn declared_names_are_the_cli_verbs() {
    let mut names: Vec<&str> = COMMAND_SPECS.iter().map(|s| s.name).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "agent.cancelQueuedInput",
            "agent.interrupt",
            "agent.list",
            "agent.listQueuedInputs",
            "agent.move",
            "agent.new",
            "agent.rename",
            "agent.resolve",
            "agent.spawn",
            "usage.get",
            "usage.refresh"
        ]
    );
}

/// Queries **are carried** in the v1 list (ADR-0156 — assumption B adopted). Without them discovery stops halfway.
#[test]
fn the_read_verb_is_in_the_v1_list() {
    let list = spec_of("agent.list").expect("agent.list is in the linked declarations");
    assert_eq!(list.effect, Effect::Read);
    assert!(
        COMMAND_SPECS
            .iter()
            .filter(|s| s.effect == Effect::Write)
            .count()
            == 7 // two were added in v6 — `agent.interrupt` · `usage.refresh`
    );
}

/// The two waiting-list verbs (ADR-0231) — the query is `Read`, the cancel is `Write`, and both came into catalog 5.
#[test]
fn the_queued_input_verbs_are_declared_with_their_effect_and_generation() {
    let list = spec_of("agent.listQueuedInputs").expect("declaration");
    assert_eq!(list.effect, Effect::Read);
    assert_eq!(list.since, 5);
    let cancel = spec_of("agent.cancelQueuedInput").expect("declaration");
    assert_eq!(cancel.effect, Effect::Write);
    assert_eq!(cancel.since, 5);
    let interrupt = spec_of("agent.interrupt").expect("declaration");
    assert_eq!(interrupt.effect, Effect::Write);
    assert_eq!(interrupt.since, 6);
}

/// The two usage verbs (TRD S21 usage-limit-slot §1-6) — the query is `Read`, the forced refresh is `Write`; both in catalog 6.
/// ★`usage.refresh` being `Write` is a contract★ — delivery holds the number by that value (a resend with the same number does not launch
/// the query again), so the result log after the deadline carries that payload and the original is stripped there (daemon `command_delivery`).
#[test]
fn the_usage_verbs_are_declared_with_their_effect_and_generation() {
    let get = spec_of("usage.get").expect("declaration");
    assert_eq!(get.effect, Effect::Read);
    assert_eq!(get.since, 6);
    let refresh = spec_of("usage.refresh").expect("declaration");
    assert_eq!(refresh.effect, Effect::Write);
    assert_eq!(refresh.since, 6);
}

/// The query resolving several words at once (TRD S28 §6) — it is `Read` and came into catalog 8.
// ADR-0307
#[test]
fn the_resolve_verb_is_a_read_declared_in_generation_8() {
    let resolve = spec_of("agent.resolve").expect("declaration");
    assert_eq!(resolve.effect, Effect::Read);
    assert_eq!(resolve.since, 8);
}

/// The generation is 9 now — `agent.new`'s `preset` field and `store.presets` in the `agent.list` reply were removed (TRD
/// S25-preset-library §5-2 1). It is bumped by hand when a declaration changes, so the value is pinned here.
#[test]
fn the_catalog_generation_is_9() {
    assert_eq!(engram_dashboard_agent::commands::CATALOG_VERSION, 9);
}

/// ★The input-moving command list holds only this crate's `Write` declarations★ — a missing name or a query slipping in would make the
/// common entry's lease check block the wrong command or miss one it should block.
#[test]
fn every_input_affecting_name_is_a_declared_write_verb() {
    assert!(!INPUT_AFFECTING.is_empty());
    for name in INPUT_AFFECTING {
        let spec = COMMAND_SPECS
            .iter()
            .find(|s| s.name == *name)
            .unwrap_or_else(|| panic!("{name} must be a declaration of this block"));
        assert_eq!(spec.effect, Effect::Write, "{name}");
    }
    assert!(INPUT_AFFECTING.contains(&"agent.cancelQueuedInput"));
    assert!(INPUT_AFFECTING.contains(&"agent.interrupt"));
    assert!(!INPUT_AFFECTING.contains(&"agent.listQueuedInputs"));
}

#[test]
fn error_sets_are_golden() {
    let declared = |name: &str| {
        spec_of(name)
            .unwrap_or_else(|| panic!("{name} declaration"))
            .errors
            .to_vec()
    };
    assert_eq!(declared("agent.list"), vec![]);
    // If even one word misses, that word's code — absent = NOT_FOUND · same name = CONFLICT (`resolve_in`).
    assert_eq!(
        declared("agent.resolve"),
        vec![ErrorCode::NotFound, ErrorCode::Conflict]
    );
    assert_eq!(
        declared("agent.spawn"),
        vec![ErrorCode::NotFound, ErrorCode::Conflict]
    );
    // No field points at a target, so there is no absent target — the create verb does not advertise NOT_FOUND.
    assert_eq!(declared("agent.new"), vec![ErrorCode::Conflict]);
    assert_eq!(
        declared("agent.rename"),
        vec![ErrorCode::NotFound, ErrorCode::Conflict]
    );
    assert_eq!(
        declared("agent.move"),
        vec![ErrorCode::NotFound, ErrorCode::Conflict]
    );
    // If the pointing misses NOT_FOUND · two of the same name CONFLICT (`resolve_in`) — the two verbs also resolve `target`.
    assert_eq!(
        declared("agent.listQueuedInputs"),
        vec![ErrorCode::NotFound, ErrorCode::Conflict]
    );
    assert_eq!(
        declared("agent.cancelQueuedInput"),
        vec![ErrorCode::NotFound, ErrorCode::Conflict]
    );
    assert_eq!(
        declared("agent.interrupt"),
        vec![ErrorCode::NotFound, ErrorCode::Conflict]
    );
    // A backend without a field = NOT_FOUND (the same code as wire ⟳).
    assert_eq!(declared("usage.get"), vec![ErrorCode::NotFound]);
    assert_eq!(declared("usage.refresh"), vec![ErrorCode::NotFound]);
}

/// ★The advertised set must be everything that can actually happen★ — argument rejection and internal failure happen in any command.
#[test]
fn every_command_advertises_the_errors_the_table_can_produce() {
    for spec in COMMAND_SPECS {
        let advertised = spec.advertised_errors();
        for common in COMMON_ERRORS {
            assert!(
                advertised.contains(common),
                "{}: {common} is not in the advertisement",
                spec.name
            );
        }
    }
}

#[test]
fn no_command_name_is_declared_twice_in_this_binary() {
    assert!(
        duplicate_command_names().is_empty(),
        "duplicate declaration: {:?}",
        duplicate_command_names()
    );
}

#[test]
fn every_declaration_passes_the_schema_lint() {
    for spec in COMMAND_SPECS {
        lint_spec(spec).expect("declaration lint");
    }
}

/// ★Only fields the schema advertised as 「may be absent」 must actually be omittable★ (ADR-0156 — the advertisement is the usage).
///
/// Comparing advertisement lists with each other makes **both wrong together if the advertisement is wrong** — so real deserialization is
/// set beside it, and fields are removed one at a time to see 「whether it fails only when a required field is missing」.
#[test]
fn required_matches_what_deserialization_actually_demands() {
    fn parses<T: serde::de::DeserializeOwned>(args: serde_json::Value) -> bool {
        serde_json::from_value::<T>(args).is_ok()
    }

    let cases: &[(&str, fn(serde_json::Value) -> bool, serde_json::Value)] = &[
        ("agent.list", parses::<AgentListArgs>, json!({})),
        (
            "agent.resolve",
            parses::<AgentResolveArgs>,
            json!({ "targets": ["alpha"] }),
        ),
        (
            "agent.spawn",
            parses::<AgentSpawnArgs>,
            json!({ "target": "alpha", "cwd": "C:/work/x", "name": "beta" }),
        ),
        (
            "agent.new",
            parses::<AgentNewArgs>,
            json!({
                "cwd": "C:/work/x",
                "name": "beta",
                "output_format": "StreamJson",
                "backend": "Claude",
            }),
        ),
        (
            "agent.rename",
            parses::<AgentRenameArgs>,
            json!({ "target": "alpha", "name": "beta" }),
        ),
        (
            "agent.move",
            parses::<AgentMoveArgs>,
            json!({ "target": "alpha", "parent": null }),
        ),
        (
            "agent.listQueuedInputs",
            parses::<AgentListQueuedInputsArgs>,
            json!({ "target": "alpha" }),
        ),
        (
            "agent.cancelQueuedInput",
            parses::<AgentCancelQueuedInputArgs>,
            json!({ "target": "alpha", "input_id": "q1" }),
        ),
        (
            "agent.interrupt",
            parses::<AgentInterruptArgs>,
            json!({ "target": "alpha" }),
        ),
        (
            "usage.get",
            parses::<UsageGetArgs>,
            json!({ "backend": "Claude" }),
        ),
        (
            "usage.refresh",
            parses::<UsageRefreshArgs>,
            json!({ "backend": "Codex" }),
        ),
    ];

    for (name, parse, full) in cases {
        let schema: serde_json::Value =
            serde_json::from_str(spec_of(name).expect("declaration").args_schema)
                .expect("args schema");
        let required: Vec<&str> = schema["required"]
            .as_array()
            .expect("required array")
            .iter()
            .map(|v| v.as_str().expect("string"))
            .collect();
        let fields: Vec<String> = schema["properties"]
            .as_object()
            .expect("properties object")
            .keys()
            .cloned()
            .collect();

        assert!(parse(full.clone()), "{name}: the full set deserializes");

        for field in fields {
            let mut without = full.as_object().expect("object").clone();
            without.remove(&field);
            let parsed = parse(serde_json::Value::Object(without));
            assert_eq!(
                parsed,
                !required.contains(&field.as_str()),
                "{name}: removing `{field}` splits the advertisement (required={required:?}) from deserialization"
            );
        }
    }
}

/// ★**The advertisement** carries the way to register without a dialog★ (ADR-0156 — the advertisement is the usage).
///
/// The caller is the LLM and sees only this schema: it must carry that the working folder (`cwd`) and the backend are required and
/// that the two fields are **closed vocabularies**, for it to choose its own arguments.
/// ★`backend` is required (2026-09-08)★ — the silent claude default was removed, and `required` must carry that so the caller knows
///   before calling.
#[test]
fn the_create_verb_advertises_its_required_fields_and_closed_vocabularies() {
    let schema: serde_json::Value =
        serde_json::from_str(spec_of("agent.new").expect("declaration").args_schema)
            .expect("args schema");
    let properties = schema["properties"].as_object().expect("properties object");

    let mut fields: Vec<&str> = properties.keys().map(String::as_str).collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        vec!["backend", "cwd", "name", "output_format"],
        "advertised fields: {schema}"
    );
    let required: Vec<&str> = schema["required"]
        .as_array()
        .expect("required array")
        .iter()
        .map(|value| value.as_str().expect("string"))
        .collect();
    assert_eq!(
        required,
        vec!["cwd", "backend"],
        "the required fields are `cwd` · `backend`: {schema}"
    );

    // A closed vocabulary is carried as **a value list** — a bare name would not let the caller choose what to put.
    // ★Both shapes are read★ — required = `{"enum":[…]}`, omittable = `{"anyOf":[{"enum":[…]},…]}`.
    let vocabulary = |field: &str| -> Vec<String> {
        let property = &properties[field];
        let branches = match property.get("anyOf") {
            Some(any_of) => any_of.as_array().expect("anyOf is an array").clone(),
            None => vec![property.clone()],
        };
        branches
            .iter()
            .filter_map(|branch| branch.get("enum"))
            .flat_map(|values| values.as_array().expect("enum array").clone())
            .map(|value| value.as_str().expect("string").to_string())
            .collect()
    };
    assert_eq!(vocabulary("output_format"), vec!["Terminal", "StreamJson"]);
    assert_eq!(
        vocabulary("backend"),
        vec!["Claude", "Codex"],
        "the passing values and the advertisement must be the same — if the advertisement is narrower than the policy, the caller reads what works as not working, \
         and if wider, it chooses values that will be rejected"
    );
}

/// ★`null` means 「detach to root」 and absence is a rejection★ — only with these three split does a single typo field not silently run as a
/// hierarchy release (the alphabet table of TRD §2-2).
#[test]
fn an_absent_option_option_field_is_refused_while_null_is_a_value() {
    let detach: AgentMoveArgs =
        serde_json::from_value(json!({ "target": "alpha", "parent": null }))
            .expect("null is a value");
    assert_eq!(detach.parent, Some(None));

    let reparent: AgentMoveArgs =
        serde_json::from_value(json!({ "target": "alpha", "parent": "beta" })).expect("value");
    assert_eq!(reparent.parent, Some(Some("beta".to_string())));

    let absent = serde_json::from_value::<AgentMoveArgs>(json!({ "target": "alpha" }))
        .expect_err("absence is rejected by deserialization");
    assert!(absent.to_string().contains("parent"), "{absent}");
}

/// Whether the linker collection scans **declarations of other crates too** — only the agent's declarations are linked into this test binary.
#[test]
fn linker_collection_sees_the_declaring_crate() {
    let mut collected: Vec<&str> = command_specs().map(|s| s.name).collect();
    collected.sort_unstable();
    let mut declared: Vec<&str> = COMMAND_SPECS.iter().map(|s| s.name).collect();
    declared.sort_unstable();
    assert_eq!(collected, declared);
}

/// The elements of the derived file and the `help` of the registration packet come from **the same one source** (ADR-0156).
#[test]
fn every_schema_item_is_valid_json_with_the_agreed_keys() {
    for spec in COMMAND_SPECS {
        let item: serde_json::Value =
            serde_json::from_str(&spec_item_json(spec)).expect("an item is one JSON");
        for key in ["name", "effect", "since", "summary", "args", "ok", "errors"] {
            assert!(
                item.get(key).is_some(),
                "{}: the {key} field is missing",
                spec.name
            );
        }
        assert!(
            !item["summary"].as_str().unwrap_or_default().is_empty(),
            "{}: summary is empty",
            spec.name
        );
    }
}
