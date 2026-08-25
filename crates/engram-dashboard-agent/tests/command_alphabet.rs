//! 선언 매크로의 **허용 타입 알파벳**을 실제로 태워 본다(TRD §2-2 3 의 미확인 항목).
//!
//! `agent.*` 가 쓰지 않는 갈래(정수·실수·bool·enum·`Option<Vec<T>>`)까지 여기서 덮는다 — 실무가 알파벳을
//! 넘는지는 선언해 보기 전엔 알 수 없고, 넘는 순간 컴파일이 멈춰야 한다.
//! ★여기 선언된 이름은 명령이 아니라 하네스 픽스처다★(`fixture.*` — 어느 표에도 꽂히지 않는다).

use engram_dashboard_command::declare_commands;

declare_commands! {
    catalog_version: 7;

    /// 중첩 struct — 부모 스키마에 인라인으로 펼쳐진다.
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

    /// 알파벳 전량.
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
    } errors [INVALID_ARGUMENT, UNSUPPORTED];
}

fn args_schema() -> serde_json::Value {
    serde_json::from_str(AlphabetArgs::SPEC.args_schema).expect("args 스키마는 JSON")
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

/// ★정수 칸은 실제 범위를 광고한다★ — 상한이 없으면 `18446744073709551616` 이 광고상 허용인데
/// 역직렬화는 반려한다(그 값을 보고 인자를 채우는 호출자에게 거짓말이다). 64비트 경계는 배정도로 읽는
/// 검증기에서 한 칸 반올림되지만, 그건 「없음」보다 좁다.
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

/// 선언한 struct 는 실제 Rust 타입이고 wire 왕복이 된다 — 스키마만 있고 타입이 없으면 소용없다.
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
    let parsed: AlphabetArgs = serde_json::from_value(value.clone()).expect("역직렬화");
    assert_eq!(parsed.mode, Mode::Slow);
    assert_eq!(serde_json::to_value(&parsed).expect("직렬화"), value);
}

/// `Option` 필드는 아예 빠져도 된다 — LLM 호출자가 안 쓰는 칸을 채우지 않아도 되게.
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
    let parsed: AlphabetArgs = serde_json::from_value(value).expect("역직렬화");
    assert!(parsed.maybe.is_none());
    assert!(parsed.maybe_many.is_none());
}

#[test]
fn catalog_version_is_per_block() {
    assert_eq!(CATALOG_VERSION, 7);
    assert_eq!(COMMAND_SPECS.len(), 1);
    assert_eq!(AlphabetArgs::SPEC.since, 3);
}
