//! Declaration golden — pins the name set and the error set so they do not silently diverge (TRD §7 「declaration derivation」).

use engram_dashboard_agent::commands::{
    AgentAttachPresetArgs, AgentCancelQueuedInputArgs, AgentDetachPresetArgs, AgentInterruptArgs,
    AgentListArgs, AgentListQueuedInputsArgs, AgentMoveArgs, AgentNewArgs, AgentPresetsArgs,
    AgentRenameArgs, AgentReorderPresetsArgs, AgentResolveArgs, AgentSpawnArgs, LibraryFoldersArgs,
    LibraryListArgs, LibraryNewFileArgs, LibraryNewFolderArgs, LibraryReadArgs, LibraryRefreshArgs,
    LibraryRelinkArgs, LibrarySetRootArgs, LibraryStatusArgs, PresetAddArgs, PresetCheckArgs,
    PresetGetArgs, PresetPreviewFolderArgs, PresetRemoveArgs, PresetReorderArgs, PresetSaveArgs,
    PresetSetBackendArgs, UsageGetArgs, UsageRefreshArgs, COMMAND_SPECS, INPUT_AFFECTING,
};
use engram_dashboard_command::DeadlineClass;
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
            "agent.attachPreset",
            "agent.cancelQueuedInput",
            "agent.detachPreset",
            "agent.interrupt",
            "agent.list",
            "agent.listQueuedInputs",
            "agent.move",
            "agent.new",
            "agent.presets",
            "agent.rename",
            "agent.reorderPresets",
            "agent.resolve",
            "agent.spawn",
            "library.folders",
            "library.list",
            "library.newFile",
            "library.newFolder",
            "library.read",
            "library.refresh",
            "library.relink",
            "library.setRoot",
            "library.status",
            "preset.add",
            "preset.check",
            "preset.get",
            "preset.previewFolder",
            "preset.remove",
            "preset.reorder",
            "preset.save",
            "preset.setBackend",
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
            == 19 // two were added in v6 — `agent.interrupt` · `usage.refresh`; twelve in v9 — `library.setRoot` · `newFolder` · `newFile` · `relink`, `preset.add` · `remove` · `reorder` · `setBackend` · `save`, `agent.attachPreset` · `detachPreset` · `reorderPresets`
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

/// The nine library commands (ADR-0386) — all in catalog 9 and ordinary (no launch deadline); the four that change the
/// library or `library.json` are `Write`. ★`library.refresh` is `Read`★: it changes only the in-memory catalog and a
/// rerun is harmless, while a `Write` would make a same-id retry answer `ALREADY_APPLIED` with no report.
#[test]
fn the_library_verbs_are_declared_in_generation_9_with_their_effect() {
    let cases = [
        ("library.status", Effect::Read),
        ("library.setRoot", Effect::Write),
        ("library.folders", Effect::Read),
        ("library.refresh", Effect::Read),
        ("library.list", Effect::Read),
        ("library.read", Effect::Read),
        ("library.newFolder", Effect::Write),
        ("library.newFile", Effect::Write),
        ("library.relink", Effect::Write),
    ];
    for (name, effect) in cases {
        let spec = spec_of(name).unwrap_or_else(|| panic!("{name} declaration"));
        assert_eq!(spec.effect, effect, "{name}");
        assert_eq!(spec.since, 9, "{name}");
        assert_eq!(spec.deadline, DeadlineClass::Ordinary, "{name}");
    }
}

/// The eight preset commands (ADR-0386) — all in catalog 9 and ordinary; the five that rewrite a preset file are
/// `Write`, the view, the draft check and the folder preview are `Read`.
#[test]
fn the_preset_verbs_are_declared_in_generation_9_with_their_effect() {
    let cases = [
        ("preset.get", Effect::Read),
        ("preset.check", Effect::Read),
        ("preset.add", Effect::Write),
        ("preset.remove", Effect::Write),
        ("preset.reorder", Effect::Write),
        ("preset.setBackend", Effect::Write),
        ("preset.save", Effect::Write),
        ("preset.previewFolder", Effect::Read),
    ];
    for (name, effect) in cases {
        let spec = spec_of(name).unwrap_or_else(|| panic!("{name} declaration"));
        assert_eq!(spec.effect, effect, "{name}");
        assert_eq!(spec.since, 9, "{name}");
        assert_eq!(spec.deadline, DeadlineClass::Ordinary, "{name}");
    }
}

/// The four agent-preset commands (ADR-0386) — all in catalog 9 and ordinary; the view is `Read`, the three edits
/// `Write`.
#[test]
fn the_agent_preset_verbs_are_declared_in_generation_9_with_their_effect() {
    let cases = [
        ("agent.presets", Effect::Read),
        ("agent.attachPreset", Effect::Write),
        ("agent.detachPreset", Effect::Write),
        ("agent.reorderPresets", Effect::Write),
    ];
    for (name, effect) in cases {
        let spec = spec_of(name).unwrap_or_else(|| panic!("{name} declaration"));
        assert_eq!(spec.effect, effect, "{name}");
        assert_eq!(spec.since, 9, "{name}");
        assert_eq!(spec.deadline, DeadlineClass::Ordinary, "{name}");
    }
}

/// `preset.reorder` advertises the closed section vocabulary and `preset.previewFolder` the two modes.
#[test]
fn the_preset_section_and_preview_mode_are_closed_vocabularies() {
    let schema = |name: &str| -> serde_json::Value {
        serde_json::from_str(spec_of(name).expect("declaration").args_schema).expect("args schema")
    };
    assert_eq!(
        schema("preset.reorder")["properties"]["section"]["enum"],
        json!(["Presets", "Instructions", "Skills", "Settings", "Folders"])
    );
    assert_eq!(
        schema("preset.previewFolder")["properties"]["mode"]["anyOf"][0]["enum"],
        json!(["Terminal", "StreamJson"])
    );
}

/// The library list filter advertises the closed kind vocabulary — `SkillFile` is its own word beside `Skill`.
#[test]
fn the_library_list_advertises_the_five_kinds() {
    let schema: serde_json::Value =
        serde_json::from_str(spec_of("library.list").expect("declaration").args_schema)
            .expect("args schema");
    let kinds = &schema["properties"]["kinds"]["anyOf"][0]["items"]["enum"];
    assert_eq!(
        kinds,
        &json!(["Instruction", "Skill", "SkillFile", "Settings", "Preset"]),
        "{schema}"
    );
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
    // ADR-0379: `Unavailable` and `Conflict` refusals are CONFLICT; `library.status` refuses only from a daemon with
    //   no library; a refresh never refuses (a bad spelling is INVALID_ARGUMENT, from the table's set).
    for name in [
        "library.status",
        "library.setRoot",
        "library.folders",
        "library.list",
    ] {
        assert_eq!(declared(name), vec![ErrorCode::Conflict], "{name}");
    }
    assert_eq!(declared("library.refresh"), vec![]);
    // A preset view or edit answers NOT_FOUND for a missing preset file; the draft check and the folder preview never
    //   do (CONFLICT `library-unavailable` only — the preview only from a daemon that runs no library).
    for name in [
        "preset.get",
        "preset.add",
        "preset.remove",
        "preset.reorder",
        "preset.setBackend",
        "preset.save",
    ] {
        assert_eq!(
            declared(name),
            vec![ErrorCode::NotFound, ErrorCode::Conflict],
            "{name}"
        );
    }
    for name in ["preset.check", "preset.previewFolder"] {
        assert_eq!(declared(name), vec![ErrorCode::Conflict], "{name}");
    }
    // The agent-preset commands resolve `target` (absent = NOT_FOUND · same name = CONFLICT); their refusals are
    //   NOT_FOUND (the agent deleted meanwhile) and CONFLICT.
    for name in [
        "library.read",
        "library.newFolder",
        "library.newFile",
        "library.relink",
        "agent.presets",
        "agent.attachPreset",
        "agent.detachPreset",
        "agent.reorderPresets",
    ] {
        assert_eq!(
            declared(name),
            vec![ErrorCode::NotFound, ErrorCode::Conflict],
            "{name}"
        );
    }
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
                "presets": ["p.yaml"],
                "root_generation": 3,
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
        ("library.status", parses::<LibraryStatusArgs>, json!({})),
        (
            "library.setRoot",
            parses::<LibrarySetRootArgs>,
            json!({ "path": "C:/lib", "create": true }),
        ),
        ("library.folders", parses::<LibraryFoldersArgs>, json!({})),
        (
            "library.refresh",
            parses::<LibraryRefreshArgs>,
            json!({ "folder": "rules" }),
        ),
        (
            "library.list",
            parses::<LibraryListArgs>,
            json!({ "folder": "rules", "query": "x", "kinds": ["Preset"], "builtins": true }),
        ),
        (
            "library.read",
            parses::<LibraryReadArgs>,
            json!({ "path": "rules/a.md" }),
        ),
        (
            "library.newFolder",
            parses::<LibraryNewFolderArgs>,
            json!({ "parent": "rules", "name": "team", "root_generation": 3 }),
        ),
        (
            "library.newFile",
            parses::<LibraryNewFileArgs>,
            json!({ "folder": "rules", "name": "a", "kind": "Markdown", "root_generation": 3 }),
        ),
        (
            "library.relink",
            parses::<LibraryRelinkArgs>,
            json!({ "from": "a.md", "to": "b/a.md", "root_generation": 3 }),
        ),
        (
            "preset.get",
            parses::<PresetGetArgs>,
            json!({ "path": "p.yaml" }),
        ),
        (
            "preset.check",
            parses::<PresetCheckArgs>,
            json!({
                "path": "p.yaml", "presets": [], "instructions": [{ "path": "a.md", "backend": "codex" }],
                "skills": [], "settings": [], "folders": ["C:/f"],
            }),
        ),
        (
            "preset.add",
            parses::<PresetAddArgs>,
            json!({
                "path": "p.yaml", "items": ["a.md"], "backend": "codex", "folders": ["C:/f"],
                "after": "b.md", "root_generation": 3,
            }),
        ),
        (
            "preset.remove",
            parses::<PresetRemoveArgs>,
            json!({ "path": "p.yaml", "items": ["a.md"], "folders": ["C:/f"], "root_generation": 3 }),
        ),
        (
            "preset.reorder",
            parses::<PresetReorderArgs>,
            json!({
                "path": "p.yaml", "section": "Skills", "order": ["a", "b"], "base_rev": "r",
                "root_generation": 3,
            }),
        ),
        (
            "preset.setBackend",
            parses::<PresetSetBackendArgs>,
            json!({ "path": "p.yaml", "items": ["a.md"], "backend": "codex", "root_generation": 3 }),
        ),
        (
            "preset.save",
            parses::<PresetSaveArgs>,
            json!({
                "path": "p.yaml", "presets": [], "instructions": [], "skills": [], "settings": [],
                "folders": [], "base_rev": "r", "root_generation": 3,
            }),
        ),
        (
            "preset.previewFolder",
            parses::<PresetPreviewFolderArgs>,
            json!({ "folder": "C:/f", "backend": "codex", "mode": "Terminal" }),
        ),
        (
            "agent.presets",
            parses::<AgentPresetsArgs>,
            json!({ "target": "alpha" }),
        ),
        (
            "agent.attachPreset",
            parses::<AgentAttachPresetArgs>,
            json!({ "target": "alpha", "presets": ["p.yaml"], "after": "q.yaml", "root_generation": 3 }),
        ),
        (
            "agent.detachPreset",
            parses::<AgentDetachPresetArgs>,
            json!({ "target": "alpha", "presets": ["p.yaml"], "yes": true, "root_generation": 3 }),
        ),
        (
            "agent.reorderPresets",
            parses::<AgentReorderPresetsArgs>,
            json!({ "target": "alpha", "order": ["p.yaml"], "base_rev": "r", "root_generation": 3 }),
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
        vec![
            "backend",
            "cwd",
            "name",
            "output_format",
            "presets",
            "root_generation"
        ],
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
