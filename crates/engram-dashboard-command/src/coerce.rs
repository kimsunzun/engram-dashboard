//! Coercion between the advertised schema and serde — runs once, **right before the table calls** (TRD §2-4 ② · §4-③).
//!
//! ★A property of the contract, not of the adapter★: coercion inside `blocking_handler` would let a handler implementing
//!   [`CommandHandler`] directly (the `await`ing shell commands of Step 3) advertise the same
//! `{"type":"integer"}` and accept input differently. So the material is the **declared schema** and it runs in one place,
//!   [`crate::CommandTable::call`].
//!
//! There are two coercions — `1.0` arriving in an integer field ([`integral_numbers_to_integers`]) and a different letter case
//! arriving in an enum field ([`enum_words_to_declared_spelling`]). Both rules use only **what the declaration said that field is** as
//!   their material.
//!
//! [`CommandHandler`]: crate::CommandHandler

use serde_json::Value;

/// The limit up to which f64 holds an integer exactly.
const EXACT: f64 = 9_007_199_254_740_992.0;

/// Rewrites a real number with no fractional part (`1.0`) in an integer field into integer notation — **where the advertised schema
///   matches what is actually accepted**.
///
/// ★It cannot be narrowed on the schema side★: in JSON's number model `1.0` and `1` are **the same value** and `{"type":"integer"}`
///   includes numbers with fractional part 0 (since draft 6; the derived file states its dialect via `$schema`),
/// so no keyword expresses "no decimal point". But serde rejects an f64 in an integer field, so a caller that filled the arguments from
///   the schema (an LLM or JS — `1.0` is common) would be rejected.
///
/// ★Only fields **the declaration said are integers**★ are fixed — real-number fields and fields absent from the declaration are left
///   alone, so `-0.0` folds to `0` in an integer field (no signed zero) and stays in a real-number field (sign observable).
/// ★Nothing outside 2^53 is touched★ — f64 cannot hold an integer exactly there, so the rewrite itself changes the value; deserialization
///   rejects it.
///
/// ★Known limit (not fixed)★: a value with a fractional part inside `[2^52, 2^53)` is **already broken before it gets here** (JSON→f64
///   rounds to an integer), so no threshold can recover "the caller sent a decimal".
/// The schema keywords read are only `properties`, `items`, `anyOf`, `type` (not `$ref`, `oneOf`, `allOf`, `prefixItems`) because the
///   macro emits only those; if it emits more, this must grow with it.
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

/// Every schema node that can stand at this position — itself plus the `anyOf` branches (`Option<T>` is emitted as
///   `{"anyOf":[T,{"type":"null"}]}`, so without expanding, optional fields would leak out of the coercion).
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
/// ★Why lenient★: a word typed by a person or LLM against a PascalCase declaration (`Claude`, `Codex`) is naturally `claude`; reading them
///   as different values would reject a word it knows.
/// ★Only fields **the declaration said are enums**★ are rewritten — the case of free-string fields (paths, names, ids) is part of the value.
/// ★Two cases left alone★: **already exactly equal** to the declared spelling, and **two or more** candidates differing only in case (same
///   decision as not touching a field where integer and real overlap).
/// ★Unknown words are not rejected here★ — deserialization rejects them listing the expected words; imitating that here would put one rule
///   in two places.
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

/// The one enum word that can stand at this position and differs from `given` **only in letter case** — `None` if there is none or it is
///   ambiguous.
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
        .expect("fixture schema")
    }

    fn coerced(mut args: Value) -> Value {
        integral_numbers_to_integers(&schema(), &mut args);
        args
    }

    #[test]
    fn an_integral_float_becomes_an_integer_only_in_an_integer_field() {
        let out = coerced(json!({ "count": 2.0, "ratio": 1.0, "maybe": 3.0 }));

        assert_eq!(out["count"], json!(2));
        assert_eq!(out["ratio"], json!(1.0), "a float field stays a float");
        assert_eq!(
            out["maybe"],
            json!(3),
            "an optional field (anyOf) is adjusted too"
        );
    }

    /// ★Knowing the type lets us tell `-0.0` apart★ — integers have no signed zero, so in an integer field it is `0`, and
    /// in a real-number field the sign is observable, so we leave it.
    #[test]
    fn negative_zero_folds_for_an_integer_field_and_survives_for_a_number_field() {
        let out = coerced(json!({ "count": -0.0, "ratio": -0.0 }));

        assert_eq!(out["count"], json!(0));
        assert_eq!(
            serde_json::to_string(&out["ratio"]).expect("serialize"),
            "-0.0",
            "the sign of a float field is observed"
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
        assert_eq!(
            out["count"],
            json!(2.5),
            "with a fractional part it is not an integer"
        );

        let out = coerced(json!({ "count": 1e300 }));
        assert_eq!(
            out["count"],
            json!(1e300),
            "outside 2^53, moving it changes the value"
        );
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
        .expect("fixture schema")
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
            "an optional field (anyOf) is moved too"
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
                .expect("schema");
        let mut value = json!("claude");
        enum_words_to_declared_spelling(&ambiguous, &mut value);

        assert_eq!(value, json!("claude"));
    }

    /// ★If two meanings overlap, do not touch★ — folding to an integer would change the value of the branch that would read it as a real number.
    #[test]
    fn a_field_that_accepts_both_integer_and_number_is_left_alone() {
        let both: Value =
            serde_json::from_str(r#"{"anyOf":[{"type":"integer"},{"type":"number"}]}"#)
                .expect("schema");
        let mut value = json!(1.0);
        integral_numbers_to_integers(&both, &mut value);

        assert_eq!(value, json!(1.0));
    }
}
