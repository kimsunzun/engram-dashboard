//! 광고한 스키마와 serde 사이의 조정 — **표가 부르기 직전에** 한 번 돈다(TRD §2-4 ② · §4-③).
//!
//! ★어댑터의 성질이 아니라 계약의 성질이다★: 조정을 `blocking_handler` 안에 두면 [`CommandHandler`] 를
//! 직접 구현한 핸들러(Step 3 의 `await` 하는 셸 명령들)는 같은 `{"type":"integer"}` 를 광고하면서 다르게
//! 받는다. 그래서 조정 재료를 **선언된 스키마**로 두고 [`crate::CommandTable::call`] 한 곳에서 돈다.
//!
//! 조정은 둘이다 — 정수 칸에 온 `1.0`([`integral_numbers_to_integers`])과 열거 칸에 온 다른 대소문자
//! ([`enum_words_to_declared_spelling`]). 두 규칙 다 **선언이 그 칸을 무엇이라 말했는가**만 재료로 쓴다.
//!
//! [`CommandHandler`]: crate::CommandHandler

use serde_json::Value;

/// f64 가 정수를 정확히 담는 한계.
const EXACT: f64 = 9_007_199_254_740_992.0;

/// 정수 칸에 온 소수부 없는 실수(`1.0`)를 정수 표기로 옮긴다 — **광고한 스키마가 실제로 받는 것과
/// 같아지는 자리**다.
///
/// ★스키마 쪽으로는 좁힐 수 없다★: JSON 의 수치 모델에서 `1.0` 과 `1` 은 **같은 값**이고,
/// `{"type":"integer"}` 는 「소수부가 0 인 수」를 포함한다(draft 6 이후 — 파생 파일이 `$schema` 로 방언을
/// 밝힌다). 즉 「소수점을 찍지 마라」를 표현하는 키워드가 없다. 그런데 serde 는 정수 칸에 f64 가 오면
/// 반려하므로, 손대지 않으면 스키마를 보고 인자를 채운 호출자(LLM·JS — 정수를 `1.0` 으로 찍는 일이
/// 흔하다)가 광고대로 보내고 거절당한다.
///
/// ★고치는 자리는 **선언이 정수라고 말한 칸**뿐이다★ — 실수 칸(`{"type":"number"}`)과 선언에 없는 칸은
/// 손대지 않는다. 그래서 `-0.0` 이 정수 칸에서는 `0` 으로 접히고(정수에 부호 있는 0 이 없다) 실수 칸에는
/// 그대로 남는다(거기서는 부호가 관측된다).
/// ★2^53 밖은 건드리지 않는다★ — 거기서는 f64 가 정수를 정확히 담지 못해 옮기는 것 자체가 값을 바꾼다.
/// 정수 칸이면 그대로 두어 역직렬화가 반려하게 둔다.
///
/// ★알려진 한계(고치지 않는다)★: `[2^52, 2^53)` 안의 소수부 있는 값은 **여기 오기 전에 이미 망가져
/// 있다** — JSON→f64 파싱이 그 구간에서 소수부를 표현하지 못해 정수로 반올림하므로, 이 함수가 어떤
/// 문턱을 골라도 「호출자가 소수를 보냈다」를 되살릴 수 없다. 그리고 이 함수가 읽는 스키마 키워드는
/// `properties`·`items`·`anyOf`·`type` 넷뿐이다(`$ref`·`oneOf`·`allOf`·`prefixItems` 는 못 따라간다) —
/// 선언 매크로가 그 넷만 찍기 때문이고, 매크로가 더 찍게 되면 여기도 함께 늘려야 한다.
pub(crate) fn integral_numbers_to_integers(schema: &Value, value: &mut Value) {
    match value {
        Value::Number(_) => {
            if accepts(schema, "integer") && !accepts(schema, "number") {
                fold_to_integer(value);
            }
        }
        Value::Object(fields) => {
            for_each_alternative(schema, &mut |node| {
                let Some(props) = node.get("properties").and_then(Value::as_object) else {
                    return;
                };
                for (name, field) in fields.iter_mut() {
                    if let Some(field_schema) = props.get(name) {
                        integral_numbers_to_integers(field_schema, field);
                    }
                }
            });
        }
        Value::Array(items) => {
            for_each_alternative(schema, &mut |node| {
                let Some(item_schema) = node.get("items") else {
                    return;
                };
                for item in items.iter_mut() {
                    integral_numbers_to_integers(item_schema, item);
                }
            });
        }
        _ => {}
    }
}

/// 이 자리에 설 수 있는 스키마 마디 전부 — 자기 자신 + `anyOf` 갈래들.
///
/// `Option<T>` 는 `{"anyOf":[T,{"type":"null"}]}` 로 찍히므로(선언 매크로) 갈래를 안 펴면 선택 필드가
/// 전부 조정 밖으로 샌다.
fn for_each_alternative(schema: &Value, visit: &mut impl FnMut(&Value)) {
    visit(schema);
    if let Some(Value::Array(branches)) = schema.get("anyOf") {
        for branch in branches {
            for_each_alternative(branch, visit);
        }
    }
}

/// 갈래 중 하나라도 이 타입을 받나. ★한 갈래라도 `number` 면 조정하지 않는다★ — 두 뜻이 겹친 칸에서
/// 값을 바꾸면 어느 쪽으로 읽힐지를 우리가 정해 버린다.
fn accepts(schema: &Value, wanted: &str) -> bool {
    let mut found = false;
    for_each_alternative(schema, &mut |node| match node.get("type") {
        Some(Value::String(declared)) => found |= declared == wanted,
        Some(Value::Array(declared)) => {
            found |= declared.iter().any(|t| t.as_str() == Some(wanted));
        }
        _ => {}
    });
    found
}

/// 열거 칸에 온 낱말을 **선언된 철자로** 옮긴다 — 대소문자만 다른 경우다.
///
/// ★왜 관용하나★: 이 칸을 채우는 것은 사람·LLM 이 손으로 친 낱말이고, 선언이 `Claude`·`Codex` 처럼
/// PascalCase 면 `claude` 로 치는 쪽이 자연스럽다(그 반대도 같다). 그 둘을 다른 값으로 읽으면 스키마를
/// 보고 채운 호출자가 「아는 낱말」을 거절당한다.
/// ★옮기는 자리는 **선언이 열거라고 말한 칸**뿐이다★ — 자유 문자열 칸은 손대지 않는다. 그 칸의 대소문자는
/// 값의 일부다(경로·이름·id).
/// ★안 옮기는 둘★: 선언된 철자와 **이미 정확히 같으면** 그대로 두고(옮길 것이 없다), 대소문자만 다른
/// 후보가 **둘 이상**이면 그대로 둔다 — 어느 쪽으로 읽을지를 여기서 정해 버리지 않는다(정수·실수가 겹친
/// 칸을 안 건드리는 것과 같은 판정).
/// ★모르는 낱말은 여기서 반려하지 않는다★ — 그대로 두면 역직렬화가 기대 낱말을 나열해 거절한다. 이 층이
/// 흉내 내면 같은 규칙이 두 곳에 산다.
pub(crate) fn enum_words_to_declared_spelling(schema: &Value, value: &mut Value) {
    match value {
        Value::String(given) => {
            if let Some(declared) = declared_spelling(schema, given) {
                *given = declared;
            }
        }
        Value::Object(fields) => {
            for_each_alternative(schema, &mut |node| {
                let Some(props) = node.get("properties").and_then(Value::as_object) else {
                    return;
                };
                for (name, field) in fields.iter_mut() {
                    if let Some(field_schema) = props.get(name) {
                        enum_words_to_declared_spelling(field_schema, field);
                    }
                }
            });
        }
        Value::Array(items) => {
            for_each_alternative(schema, &mut |node| {
                let Some(item_schema) = node.get("items") else {
                    return;
                };
                for item in items.iter_mut() {
                    enum_words_to_declared_spelling(item_schema, item);
                }
            });
        }
        _ => {}
    }
}

/// 이 자리에 설 수 있는 열거 낱말 중 `given` 과 **대소문자만 다른** 하나 — 없거나 애매하면 `None`.
fn declared_spelling(schema: &Value, given: &str) -> Option<String> {
    let mut exact = false;
    let mut folded: Option<String> = None;
    let mut ambiguous = false;
    for_each_alternative(schema, &mut |node| {
        let Some(Value::Array(words)) = node.get("enum") else {
            return;
        };
        for word in words.iter().filter_map(Value::as_str) {
            if word == given {
                exact = true;
            } else if word.eq_ignore_ascii_case(given) {
                match &folded {
                    None => folded = Some(word.to_string()),
                    Some(seen) if seen == word => {}
                    Some(_) => ambiguous = true,
                }
            }
        }
    });
    if exact || ambiguous {
        return None;
    }
    folded
}

fn fold_to_integer(value: &mut Value) {
    let Value::Number(number) = value else {
        return;
    };
    if !number.is_f64() {
        return;
    }
    let Some(as_float) = number.as_f64() else {
        return;
    };
    if as_float.is_finite() && as_float.fract() == 0.0 && as_float.abs() <= EXACT {
        *number = serde_json::Number::from(as_float as i64);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn schema() -> Value {
        serde_json::from_str(
            r#"{"type":"object","properties":{
                 "count":{"type":"integer","minimum":0},
                 "ratio":{"type":"number"},
                 "maybe":{"anyOf":[{"type":"integer"},{"type":"null"}]},
                 "rows":{"type":"array","items":{"type":"object","properties":{
                    "size":{"type":"integer"}}}}}}"#,
        )
        .expect("픽스처 스키마")
    }

    fn coerced(mut args: Value) -> Value {
        integral_numbers_to_integers(&schema(), &mut args);
        args
    }

    #[test]
    fn an_integral_float_becomes_an_integer_only_in_an_integer_field() {
        let out = coerced(json!({ "count": 2.0, "ratio": 1.0, "maybe": 3.0 }));

        assert_eq!(out["count"], json!(2));
        assert_eq!(out["ratio"], json!(1.0), "실수 칸은 실수로 남는다");
        assert_eq!(out["maybe"], json!(3), "선택 칸(anyOf)도 조정된다");
    }

    /// ★타입을 알면 `-0.0` 을 가를 수 있다★ — 정수에 부호 있는 0 이 없으므로 정수 칸에서는 `0` 이고,
    /// 실수 칸에서는 부호가 관측되므로 그대로 둔다.
    #[test]
    fn negative_zero_folds_for_an_integer_field_and_survives_for_a_number_field() {
        let out = coerced(json!({ "count": -0.0, "ratio": -0.0 }));

        assert_eq!(out["count"], json!(0));
        assert_eq!(
            serde_json::to_string(&out["ratio"]).expect("직렬화"),
            "-0.0",
            "실수 칸의 부호는 관측된다"
        );
    }

    #[test]
    fn nested_objects_and_arrays_are_reached() {
        let out = coerced(json!({ "rows": [{ "size": 4.0 }] }));
        assert_eq!(out["rows"][0]["size"], json!(4));
    }

    #[test]
    fn a_fractional_value_and_a_value_beyond_exact_range_are_left_alone() {
        let out = coerced(json!({ "count": 2.5 }));
        assert_eq!(out["count"], json!(2.5), "소수부가 있으면 정수가 아니다");

        let out = coerced(json!({ "count": 1e300 }));
        assert_eq!(out["count"], json!(1e300), "2^53 밖은 옮기면 값이 바뀐다");
    }

    /// 선언에 없는 칸은 손대지 않는다 — 무엇으로 읽힐지 모르는 값을 바꿀 근거가 없다.
    #[test]
    fn a_field_the_schema_does_not_declare_is_left_alone() {
        let out = coerced(json!({ "unknown": 7.0 }));
        assert_eq!(out["unknown"], json!(7.0));
    }

    // ── 열거 낱말의 철자 ────────────────────────────────────────────────────────────────────

    /// 선언 매크로가 열거 칸에 찍는 모양 그대로다(`{"enum":[…]}` · 선택 칸은 `anyOf` 로 감싼다).
    fn word_schema() -> Value {
        serde_json::from_str(
            r#"{"type":"object","properties":{
                 "backend":{"enum":["Claude","Codex"]},
                 "format":{"anyOf":[{"enum":["Terminal","StreamJson"]},{"type":"null"}]},
                 "name":{"type":"string"},
                 "rows":{"type":"array","items":{"enum":["Claude"]}}}}"#,
        )
        .expect("픽스처 스키마")
    }

    fn respelled(mut args: Value) -> Value {
        enum_words_to_declared_spelling(&word_schema(), &mut args);
        args
    }

    #[test]
    fn an_enum_word_typed_in_another_casing_becomes_the_declared_spelling() {
        let out = respelled(json!({ "backend": "codex", "format": "streamjson" }));

        assert_eq!(out["backend"], json!("Codex"));
        assert_eq!(
            out["format"],
            json!("StreamJson"),
            "선택 칸(anyOf)도 옮긴다"
        );

        let out = respelled(json!({ "backend": "CLAUDE" }));
        assert_eq!(out["backend"], json!("Claude"));
    }

    #[test]
    fn nested_enum_words_are_reached() {
        let out = respelled(json!({ "rows": ["claude"] }));
        assert_eq!(out["rows"][0], json!("Claude"));
    }

    /// 열거가 아닌 칸의 대소문자는 값의 일부다 — 경로·이름·id 가 여기 든다.
    #[test]
    fn a_free_string_field_and_an_undeclared_field_keep_their_casing() {
        let out = respelled(json!({ "name": "Worker", "unknown": "Claude" }));

        assert_eq!(out["name"], json!("Worker"));
        assert_eq!(out["unknown"], json!("Claude"));
    }

    /// 모르는 낱말은 그대로 둔다 — 반려 문구를 내는 것은 역직렬화 쪽이다.
    #[test]
    fn a_word_the_enum_does_not_declare_is_left_alone() {
        let out = respelled(json!({ "backend": "codx" }));
        assert_eq!(out["backend"], json!("codx"));
    }

    /// ★대소문자만 다른 후보가 둘이면 안 건드린다★ — 어느 쪽으로 읽을지를 이 층이 정해 버린다.
    #[test]
    fn a_word_that_two_declared_spellings_could_be_is_left_alone() {
        let ambiguous: Value =
            serde_json::from_str(r#"{"anyOf":[{"enum":["Claude"]},{"enum":["CLAUDE"]}]}"#)
                .expect("스키마");
        let mut value = json!("claude");
        enum_words_to_declared_spelling(&ambiguous, &mut value);

        assert_eq!(value, json!("claude"));
    }

    /// ★두 뜻이 겹치면 안 건드린다★ — 정수로 접으면 실수로 읽을 갈래의 값을 우리가 바꿔 버린다.
    #[test]
    fn a_field_that_accepts_both_integer_and_number_is_left_alone() {
        let both: Value =
            serde_json::from_str(r#"{"anyOf":[{"type":"integer"},{"type":"number"}]}"#)
                .expect("스키마");
        let mut value = json!(1.0);
        integral_numbers_to_integers(&both, &mut value);

        assert_eq!(value, json!(1.0));
    }
}
