//! 선언 골든 — 이름 집합과 오류 집합이 조용히 갈리지 않게 못 박는다(TRD §7 「선언 파생」).

use engram_dashboard_agent::commands::{
    AgentCancelQueuedInputArgs, AgentListArgs, AgentListQueuedInputsArgs, AgentMoveArgs,
    AgentNewArgs, AgentRenameArgs, AgentSpawnArgs, COMMAND_SPECS, INPUT_AFFECTING,
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
            "agent.list",
            "agent.listQueuedInputs",
            "agent.move",
            "agent.new",
            "agent.rename",
            "agent.spawn"
        ]
    );
}

/// 조회는 v1 목록에 **실린다**(ADR-0156 — 가정 B 채택). 빼면 발견이 절반에서 멈춘다.
#[test]
fn the_read_verb_is_in_the_v1_list() {
    let list = spec_of("agent.list").expect("agent.list 는 링크된 선언에 있다");
    assert_eq!(list.effect, Effect::Read);
    assert!(
        COMMAND_SPECS
            .iter()
            .filter(|s| s.effect == Effect::Write)
            .count()
            == 5
    );
}

/// 대기 목록 두 동사(ADR-0231) — 조회는 `Read`, 취소는 `Write` 이고 둘 다 카탈로그 5 에 들어왔다.
#[test]
fn the_queued_input_verbs_are_declared_with_their_effect_and_generation() {
    let list = spec_of("agent.listQueuedInputs").expect("선언");
    assert_eq!(list.effect, Effect::Read);
    assert_eq!(list.since, 5);
    let cancel = spec_of("agent.cancelQueuedInput").expect("선언");
    assert_eq!(cancel.effect, Effect::Write);
    assert_eq!(cancel.since, 5);
    assert_eq!(engram_dashboard_agent::commands::CATALOG_VERSION, 5);
}

/// ★입력을 움직이는 명령 목록은 이 crate 의 `Write` 선언만 담는다★ — 없는 이름이나 조회가 끼면 공통 입구의
/// 임대 검사가 엉뚱한 명령을 막거나 막아야 할 명령을 놓친다.
#[test]
fn every_input_affecting_name_is_a_declared_write_verb() {
    assert!(!INPUT_AFFECTING.is_empty());
    for name in INPUT_AFFECTING {
        let spec = COMMAND_SPECS
            .iter()
            .find(|s| s.name == *name)
            .unwrap_or_else(|| panic!("{name} 은 이 블록의 선언이어야 한다"));
        assert_eq!(spec.effect, Effect::Write, "{name}");
    }
    assert!(INPUT_AFFECTING.contains(&"agent.cancelQueuedInput"));
    assert!(!INPUT_AFFECTING.contains(&"agent.listQueuedInputs"));
}

#[test]
fn error_sets_are_golden() {
    let declared = |name: &str| {
        spec_of(name)
            .unwrap_or_else(|| panic!("{name} 선언"))
            .errors
            .to_vec()
    };
    assert_eq!(declared("agent.list"), vec![]);
    assert_eq!(
        declared("agent.spawn"),
        vec![ErrorCode::NotFound, ErrorCode::Conflict]
    );
    // `preset` 지목이 빗나가면 대상 부재다 — 그래서 만들기 동사도 NOT_FOUND 를 광고한다.
    assert_eq!(
        declared("agent.new"),
        vec![ErrorCode::NotFound, ErrorCode::Conflict]
    );
    assert_eq!(
        declared("agent.rename"),
        vec![ErrorCode::NotFound, ErrorCode::Conflict]
    );
    assert_eq!(
        declared("agent.move"),
        vec![ErrorCode::NotFound, ErrorCode::Conflict]
    );
    // 지목이 빗나가면 NOT_FOUND · 동명 둘이면 CONFLICT(`resolve_in`) — 두 동사도 `target` 을 푼다.
    assert_eq!(
        declared("agent.listQueuedInputs"),
        vec![ErrorCode::NotFound, ErrorCode::Conflict]
    );
    assert_eq!(
        declared("agent.cancelQueuedInput"),
        vec![ErrorCode::NotFound, ErrorCode::Conflict]
    );
}

/// ★광고된 집합은 실제로 날 수 있는 것 전부여야 한다★ — 인자 반려와 내부 실패는 어느 명령에서나 난다.
#[test]
fn every_command_advertises_the_errors_the_table_can_produce() {
    for spec in COMMAND_SPECS {
        let advertised = spec.advertised_errors();
        for common in COMMON_ERRORS {
            assert!(
                advertised.contains(common),
                "{}: {common} 이 광고에 없다",
                spec.name
            );
        }
    }
}

/// 같은 이름이 두 번 링크되면 어느 계약이 이기는지 알 수 없다 — 조회가 패닉하기 전에 여기서 잡는다.
#[test]
fn no_command_name_is_declared_twice_in_this_binary() {
    assert!(
        duplicate_command_names().is_empty(),
        "중복 선언: {:?}",
        duplicate_command_names()
    );
}

/// 매크로가 컴파일 타임에 못 잡는 것(raw identifier · 깨진 스키마)을 여기서 잡는다.
#[test]
fn every_declaration_passes_the_schema_lint() {
    for spec in COMMAND_SPECS {
        lint_spec(spec).expect("선언 lint");
    }
}

/// ★스키마가 「없어도 된다」고 광고한 필드만 실제로 생략 가능해야 한다★(ADR-0156 — 광고가 곧 사용법이다).
///
/// 광고를 목록끼리 견주면 **광고가 틀렸을 때 둘 다 같이 틀린다** — 그래서 광고 옆에 실제 역직렬화를
/// 세우고, 칸을 하나씩 빼면서 「required 라고 적힌 것만 빠질 때 터지는지」를 본다.
#[test]
fn required_matches_what_deserialization_actually_demands() {
    fn parses<T: serde::de::DeserializeOwned>(args: serde_json::Value) -> bool {
        serde_json::from_value::<T>(args).is_ok()
    }

    let cases: &[(&str, fn(serde_json::Value) -> bool, serde_json::Value)] = &[
        ("agent.list", parses::<AgentListArgs>, json!({})),
        (
            "agent.spawn",
            parses::<AgentSpawnArgs>,
            json!({ "target": "alpha", "cwd": "C:/work/x", "name": "beta" }),
        ),
        (
            "agent.new",
            parses::<AgentNewArgs>,
            // 상호배타인 두 칸을 함께 싣는다 — 이 대조가 재는 것은 **역직렬화**이고, 조합 제약은 핸들러가
            //   본다(`agent.spawn` 의 target/cwd 와 같은 자리).
            json!({
                "cwd": "C:/work/x",
                "preset": "bookmark-1",
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
    ];

    for (name, parse, full) in cases {
        let schema: serde_json::Value =
            serde_json::from_str(spec_of(name).expect("선언").args_schema).expect("args 스키마");
        let required: Vec<&str> = schema["required"]
            .as_array()
            .expect("required 배열")
            .iter()
            .map(|v| v.as_str().expect("문자열"))
            .collect();
        let fields: Vec<String> = schema["properties"]
            .as_object()
            .expect("properties 객체")
            .keys()
            .cloned()
            .collect();

        assert!(parse(full.clone()), "{name}: 전량은 역직렬화된다");

        for field in fields {
            let mut without = full.as_object().expect("객체").clone();
            without.remove(&field);
            let parsed = parse(serde_json::Value::Object(without));
            assert_eq!(
                parsed,
                !required.contains(&field.as_str()),
                "{name}: `{field}` 를 빼면 광고(required={required:?})와 역직렬화가 갈린다"
            );
        }
    }
}

/// ★대화상자 없이 등록하는 길을 **광고가** 나른다★(ADR-0156 — 광고가 곧 사용법이다).
///
/// 이 명령을 부르는 주체는 LLM 이고, 그가 보는 것은 이 스키마뿐이다: 폴더를 두 길로 정할 수 있다는 것
/// (`cwd`·`preset`)과 나머지 두 칸이 **닫힌 어휘**라는 것이 거기 실려야 자기 인자를 스스로 고른다.
/// ★폴더를 정하는 두 칸은 어느 쪽도 혼자서는 필수가 아니다★ — 「정확히 하나」는 칸이 아니라 **조합**이라
/// 스키마가 표현하지 못한다(그 판정은 핸들러가 지고, 반려 문구가 두 칸을 함께 짚는다).
/// ★반면 `backend` 는 혼자서 필수다(2026-09-08)★ — 조용한 claude 기본값을 걷은 결과이고, 그 사실이
/// `required` 에 실려야 호출자가 부르기 전에 안다. 옛 계약(`required` 가 비어 있음)은 그 기본값 위에
/// 서 있었다.
#[test]
fn the_create_verb_advertises_both_ways_to_pick_a_folder_and_its_closed_vocabularies() {
    let schema: serde_json::Value =
        serde_json::from_str(spec_of("agent.new").expect("선언").args_schema).expect("args 스키마");
    let properties = schema["properties"].as_object().expect("properties 객체");

    for field in ["cwd", "preset", "name", "output_format", "backend"] {
        assert!(properties.contains_key(field), "{field} 칸이 광고에 없다");
    }
    let required: Vec<&str> = schema["required"]
        .as_array()
        .expect("required 배열")
        .iter()
        .map(|value| value.as_str().expect("문자열"))
        .collect();
    assert_eq!(
        required,
        vec!["backend"],
        "혼자서 필수인 칸은 `backend` 하나다: {schema}"
    );

    // 닫힌 어휘는 **값 목록**으로 실린다 — 이름만 실리면 호출자가 무엇을 넣을지 스스로 못 고른다.
    // ★두 모양을 다 읽는다★ — 필수 칸은 `{"enum":[…]}`, 생략 가능한 칸은 `{"anyOf":[{"enum":[…]},…]}` 다.
    let vocabulary = |field: &str| -> Vec<String> {
        let property = &properties[field];
        let branches = match property.get("anyOf") {
            Some(any_of) => any_of.as_array().expect("anyOf 는 배열").clone(),
            None => vec![property.clone()],
        };
        branches
            .iter()
            .filter_map(|branch| branch.get("enum"))
            .flat_map(|values| values.as_array().expect("enum 배열").clone())
            .map(|value| value.as_str().expect("문자열").to_string())
            .collect()
    };
    assert_eq!(vocabulary("output_format"), vec!["Terminal", "StreamJson"]);
    assert_eq!(
        vocabulary("backend"),
        vec!["Claude", "Codex"],
        "통과하는 값과 광고가 같아야 한다 — 광고가 정책보다 좁으면 호출자는 되는 것을 안 된다고 읽고, \
         넓으면 반려당할 값을 고른다"
    );
}

/// ★`null` 은 「루트로 떼라」는 지시이고 부재는 반려다★ — 이 셋이 갈려야 오타 필드 하나가 조용히
/// 계층 해제로 실행되지 않는다(TRD §2-2 알파벳 표).
#[test]
fn an_absent_option_option_field_is_refused_while_null_is_a_value() {
    let detach: AgentMoveArgs =
        serde_json::from_value(json!({ "target": "alpha", "parent": null })).expect("null 은 값");
    assert_eq!(detach.parent, Some(None));

    let reparent: AgentMoveArgs =
        serde_json::from_value(json!({ "target": "alpha", "parent": "beta" })).expect("값");
    assert_eq!(reparent.parent, Some(Some("beta".to_string())));

    let absent = serde_json::from_value::<AgentMoveArgs>(json!({ "target": "alpha" }))
        .expect_err("부재는 역직렬화가 반려한다");
    assert!(absent.to_string().contains("parent"), "{absent}");
}

/// 링커 수집이 **다른 crate 의 선언까지** 훑는지 — 이 테스트 바이너리에 링크된 것은 agent 의 선언뿐이다.
#[test]
fn linker_collection_sees_the_declaring_crate() {
    let mut collected: Vec<&str> = command_specs().map(|s| s.name).collect();
    collected.sort_unstable();
    let mut declared: Vec<&str> = COMMAND_SPECS.iter().map(|s| s.name).collect();
    declared.sort_unstable();
    assert_eq!(collected, declared);
}

/// 파생 파일의 원소와 등록 패킷의 `help` 가 **같은 한 출처**에서 나온다(ADR-0156).
#[test]
fn every_schema_item_is_valid_json_with_the_agreed_keys() {
    for spec in COMMAND_SPECS {
        let item: serde_json::Value =
            serde_json::from_str(&spec_item_json(spec)).expect("항목은 JSON 하나");
        for key in ["name", "effect", "since", "summary", "args", "ok", "errors"] {
            assert!(item.get(key).is_some(), "{}: {key} 칸이 없다", spec.name);
        }
        assert!(
            !item["summary"].as_str().unwrap_or_default().is_empty(),
            "{}: summary 가 비었다",
            spec.name
        );
    }
}
