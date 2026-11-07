//! Coercion between the advertised schema and serde — runs once, **right before the table calls** (TRD §2-4 ② · §4-③).
//!
//! ★This is a property of the contract, not of the adapter★: if the coercion lived inside `blocking_handler`, a handler that implements
//! [`CommandHandler`] directly (the `await`ing shell commands of Step 3) would advertise the same `{"type":"integer"}` and yet
//! accept input differently. So the coercion material is the **declared schema**, and it runs in one place, [`crate::CommandTable::call`].
//!
//! There are two coercions — `1.0` arriving in an integer field ([`integral_numbers_to_integers`]) and a different letter case
//! arriving in an enum field ([`enum_words_to_declared_spelling`]). Both rules use only **what the declaration said that field is** as their material.
//!
//! [`CommandHandler`]: crate::CommandHandler

use serde_json::Value;

/// The limit up to which f64 holds an integer exactly.
const EXACT: f64 = 9_007_199_254_740_992.0;

/// Rewrites a real number with no fractional part (`1.0`) in an integer field into integer notation — this is **the point
/// where the advertised schema matches what is actually accepted**.
///
/// ★It cannot be narrowed on the schema side★: in JSON's number model `1.0` and `1` are **the same value**,
/// and `{"type":"integer"}` includes 「소수부가 0 인 수」 (numbers whose fractional part is 0) (since draft 6 — the derived file
/// states its dialect through `$schema`). So no keyword expresses 「소수점을 찍지 마라」 (do not write a decimal point). But serde
/// rejects an f64 arriving in an integer field, so if we leave it alone, a caller that filled the arguments from the schema
/// (an LLM or JS — writing an integer as `1.0` is common) sends what was advertised and gets rejected.
///
/// ★The only fields we fix are **those the declaration said are integers**★ — real-number fields (`{"type":"number"}`) and
/// fields absent from the declaration are left alone. So `-0.0` folds to `0` in an integer field (integers have no signed zero) and stays
/// as is in a real-number field (there the sign is observable).
/// ★Nothing outside 2^53 is touched★ — there f64 cannot hold an integer exactly, so the rewrite itself changes the value.
/// For an integer field we leave it as is and let deserialization reject it.
///
/// ★Known limit (not fixed)★: a value with a fractional part inside `[2^52, 2^53)` is **already broken
/// before it gets here** — JSON→f64 parsing cannot represent a fractional part in that range and rounds to an integer, so
/// whatever threshold this function picks, it cannot recover 「호출자가 소수를 보냈다」 (the caller sent a decimal). Also, the schema keywords this function reads
/// are only four: `properties`, `items`, `anyOf`, `type` (it cannot follow `$ref`, `oneOf`, `allOf`, `prefixItems`) —
/// because the declaration macro emits only those four; if the macro starts emitting more, this must grow with it.
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

/// Every schema node that can stand at this position — itself plus the `anyOf` branches.
///
/// `Option<T>` is emitted as `{"anyOf":[T,{"type":"null"}]}` (declaration macro), so without expanding the branches, optional fields
/// would all leak out of the coercion.
fn for_each_alternative(schema: &Value, visit: &mut impl FnMut(&Value)) {
    visit(schema);
    if let Some(Value::Array(branches)) = schema.get("anyOf") {
        for branch in branches {
            for_each_alternative(branch, visit);
        }
    }
}

/// Whether any branch accepts this type. ★If even one branch is `number`, do not coerce★ — changing the value
/// in a field where two meanings overlap means we decide which way it will be read.
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

/// Rewrites a word arriving in an enum field **into the declared spelling** — only when the letter case alone differs.
///
/// ★Why be lenient★: what fills this field is a word typed by hand by a person or LLM, and if the declaration is
/// PascalCase like `Claude`, `Codex`, typing `claude` is natural (and the reverse is the same). Reading those two as different values
/// would reject 「아는 낱말」 (a word it knows) for a caller who filled it from the schema.
/// ★The only fields we rewrite are **those the declaration said are enums**★ — free-string fields are left alone. The letter case of
/// such a field is part of the value (paths, names, ids).
/// ★Two cases we do not rewrite★: if it is **already exactly equal** to the declared spelling, leave it (nothing to rewrite), and if
/// there are **two or more** candidates differing only in letter case, leave it — we do not decide here which way
/// to read it (the same decision as not touching a field where integer and real overlap).
/// ★Unknown words are not rejected here★ — if left alone, deserialization rejects them listing the expected words. If this layer
/// imitated that, the same rule would live in two places.
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

/// The one enum word that can stand at this position and differs from `given` **only in letter case** — `None` if there is none or it is ambiguous.
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

    /// ★Knowing the type lets us tell `-0.0` apart★ — integers have no signed zero, so in an integer field it is `0`, and
    /// in a real-number field the sign is observable, so we leave it.
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

    /// Fields absent from the declaration are left alone — there is no basis for changing a value whose reading is unknown.
    #[test]
    fn a_field_the_schema_does_not_declare_is_left_alone() {
        let out = coerced(json!({ "unknown": 7.0 }));
        assert_eq!(out["unknown"], json!(7.0));
    }

    // ── Spelling of enum words ────────────────────────────────────────────────────────────────────

    /// Exactly the shape the declaration macro emits for an enum field (`{"enum":[…]}` · an optional field is wrapped in `anyOf`).
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

    /// The letter case of a non-enum field is part of the value — paths, names and ids fall here.
    #[test]
    fn a_free_string_field_and_an_undeclared_field_keep_their_casing() {
        let out = respelled(json!({ "name": "Worker", "unknown": "Claude" }));

        assert_eq!(out["name"], json!("Worker"));
        assert_eq!(out["unknown"], json!("Claude"));
    }

    /// Unknown words are left alone — it is deserialization that produces the rejection message.
    #[test]
    fn a_word_the_enum_does_not_declare_is_left_alone() {
        let out = respelled(json!({ "backend": "codx" }));
        assert_eq!(out["backend"], json!("codx"));
    }

    /// ★If two candidates differ only in letter case, do not touch★ — this layer would decide which way to read it.
    #[test]
    fn a_word_that_two_declared_spellings_could_be_is_left_alone() {
        let ambiguous: Value =
            serde_json::from_str(r#"{"anyOf":[{"enum":["Claude"]},{"enum":["CLAUDE"]}]}"#)
                .expect("스키마");
        let mut value = json!("claude");
        enum_words_to_declared_spelling(&ambiguous, &mut value);

        assert_eq!(value, json!("claude"));
    }

    /// ★If two meanings overlap, do not touch★ — folding to an integer would change the value of the branch that would read it as a real number.
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
