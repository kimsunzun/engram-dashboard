//! 정적 계약 — 선언에서 파생되는 것 전부(TRD §3-1 · §2-4).

use std::fmt::Write as _;

use crate::ErrorCode;

/// 읽기/쓰기 표식. `Read` 는 멱등이라 dedup 대상에서 면제된다(ADR-0150 · TRD §4-⑥).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Effect {
    Write,
    Read,
}

impl Effect {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Write => "Write",
            Self::Read => "Read",
        }
    }
}

/// 명령 하나의 정적 계약.
///
/// ★링커 수집이 담아도 되는 것은 여기까지다(규칙 T-1)★ — 핸들러 실물은 `make_table(deps)` 가 조립 때
/// 주입한다. 이 타입에 `Arc<dyn CommandHandler>` 를 얹으면 그 규칙이 무너진다.
/// `args_schema`·`ok_schema` 는 **JSON Schema 텍스트**(선언 매크로 생성)다.
// ADR-0149
pub struct CommandSpec {
    pub name: &'static str,
    pub effect: Effect,
    pub since: u32,
    pub summary: &'static str,
    pub args_schema: &'static str,
    pub ok_schema: &'static str,
    /// **선언된** 오류만. 광고되는 집합은 [`CommandSpec::advertised_errors`] 다(공통 오류가 얹힌다).
    pub errors: &'static [ErrorCode],
    /// 인자 struct 의 **타입 이름**(선언 매크로가 `stringify!` 로 채운다).
    ///
    /// ★파생물 커버리지의 유일한 재료다★ — TS 바인딩은 타입마다 파일 하나로 나오므로, 선언에서 그
    /// 파일 이름을 끌어낼 수 있어야 「선언은 늘었는데 export 는 안 늘었다」를 **손 목록 없이** 잡는다.
    /// 손으로 적은 목록은 선언을 늘리는 그 편집이 함께 고쳐 버려 무장 해제된다(TRD §5 가 `src-tauri/
    /// bindings/` 에서 실측한 실패 모드).
    pub args_type: &'static str,
    /// 반환 struct 의 타입 이름 — [`CommandSpec::args_type`] 와 같은 쓸모.
    pub ok_type: &'static str,
}

/// 표가 **모든 명령에** 자동으로 얹는 오류.
///
/// ★선언에 손으로 적게 하지 않는 이유★: 이 둘은 명령 본문이 아니라 **표가** 내는 것이다 —
/// 인자 역직렬화 실패(`INVALID_ARGUMENT`) · 핸들러 패닉과 반환 직렬화 실패(`INTERNAL`).
/// 손으로 적게 하면 빠뜨린 선언이 「이 오류는 안 난다」고 거짓 광고를 한다.
pub const COMMON_ERRORS: &[ErrorCode] = &[ErrorCode::InvalidArgument, ErrorCode::Internal];

impl CommandSpec {
    /// 호출자에게 광고되는 오류 전량 = 선언분 + 공통분(선언 순서 유지, 중복 제거).
    pub fn advertised_errors(&self) -> Vec<ErrorCode> {
        let mut out = self.errors.to_vec();
        for common in COMMON_ERRORS {
            if !out.contains(common) {
                out.push(*common);
            }
        }
        out
    }
}

/// 링커 수집 항목 — 선언 매크로만 만든다.
///
/// 값이 아니라 **참조**를 싣는 이유: 같은 선언이 `<ArgsType>::SPEC`(crate 안 목록)과 이 수집(바이너리
/// 전량) 두 곳에서 보이는데, 값을 복사하면 두 사본이 조용히 갈릴 수 있다.
#[doc(hidden)]
pub struct LinkedSpec(pub &'static CommandSpec);

inventory::collect!(LinkedSpec);

/// 이 **바이너리에 링크된** 선언 전량.
///
/// ★바이너리마다 보이는 집합이 다르다 — 결함이 아니라 의도다★(TRD §2-2 2). 각 프로세스는 자기가
/// 조립할 수 있는 것만 알면 된다.
pub fn command_specs() -> impl Iterator<Item = &'static CommandSpec> {
    inventory::iter::<LinkedSpec>.into_iter().map(|e| e.0)
}

/// ★같은 이름이 둘이면 **패닉한다** — 조용히 하나를 고르지 않는다★.
///
/// 두 선언이 같은 이름을 쥐면 어느 계약(인자 모양·오류 집합)이 이기는지 알 수 없고, 그 상태로 광고된
/// 스키마는 절반이 거짓이 된다. 중복은 **빌드가 정하는 값**이라 런타임에 달라지지 않으므로 첫 조회에서
/// 즉시 드러내는 것이 옳다. 중복 없는 사전 확인은 [`duplicate_command_names`].
pub fn spec_of(name: &str) -> Option<&'static CommandSpec> {
    find_unique(command_specs(), name)
}

/// 링크된 선언 전량에서 중복된 이름 — 조립부가 기동 시 비어 있음을 확인하는 용도.
pub fn duplicate_command_names() -> Vec<&'static str> {
    let mut seen: Vec<&'static str> = Vec::new();
    let mut dupes: Vec<&'static str> = Vec::new();
    for spec in command_specs() {
        if seen.contains(&spec.name) {
            if !dupes.contains(&spec.name) {
                dupes.push(spec.name);
            }
        } else {
            seen.push(spec.name);
        }
    }
    dupes
}

/// 이름으로 하나를 고른다 — 둘 이상이면 패닉(위 doc).
pub(crate) fn find_unique<'a>(
    specs: impl Iterator<Item = &'a CommandSpec>,
    name: &str,
) -> Option<&'a CommandSpec> {
    let mut matches = specs.filter(|s| s.name == name);
    let first = matches.next()?;
    if matches.next().is_some() {
        panic!(
            "command declared more than once: {name} — 두 선언이 같은 이름을 쥐면 어느 계약이 이기는지 알 수 없다"
        );
    }
    Some(first)
}

/// 등록이 나르는 단위(TRD §3-1 · §3-7).
///
/// `help` = 그 명령의 **파생 스키마 항목 하나를 통째로 직렬화한 JSON 텍스트**([`spec_item_json`]).
/// `args_schema` 한 칸이 아니다 — `ok` 까지 실려야 조회 명령의 반환 모양이 함께 발견된다(ADR-0150).
/// ★받는 쪽(데몬)에게 이 값은 불투명 문자열이다★ — 파싱·검증·분기하면 위반이고, 그래서 자료형을
/// `String` 위로 올리지 않는다(TRD §3-7 하드 제약).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CommandDecl {
    pub name: String,
    pub help: String,
}

/// 스키마 항목 하나 — 파생 파일의 원소이자 등록 패킷의 `help`.
///
/// ★두 경로가 **같은 한 출처**를 쓰게 하는 자리다★: 디스크 파일이 갱신되지 않아도 명부에 등록되는
/// 모양은 바이너리와 항상 일치한다(TRD §2-4 ②).
pub fn spec_item_json(spec: &CommandSpec) -> String {
    let mut out = String::new();
    out.push_str("{\"name\":");
    push_json_string(&mut out, spec.name);
    let _ = write!(
        out,
        ",\"effect\":\"{}\",\"since\":{}",
        spec.effect.as_str(),
        spec.since
    );
    out.push_str(",\"summary\":");
    push_json_string(&mut out, spec.summary.trim());
    out.push_str(",\"args\":");
    out.push_str(spec.args_schema);
    out.push_str(",\"ok\":");
    out.push_str(spec.ok_schema);
    out.push_str(",\"errors\":[");
    for (i, code) in spec.advertised_errors().iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "\"{}\"", code.as_str());
    }
    out.push_str("]}");
    out
}

/// 선언 하나가 **광고할 수 있는 모양인지** 본다 — 선언 crate 의 테스트가 부른다.
///
/// ★매크로가 못 잡는 것만 본다★: 알파벳은 컴파일 타임에 닫히지만 두 가지는 그때 안 걸린다 —
/// ① raw identifier(`r#type`)는 스키마에 `r#` 가 붙은 채 찍히는데 serde 가 쓰는 wire 이름은 `type` 이라
/// **광고와 실제가 갈린다**(매크로가 문자열을 다듬을 수단이 없다. 필드 이름과 **enum variant** 둘 다다 —
/// variant 도 `stringify!` 로 찍히고 serde 는 `r#` 를 뗀 이름을 쓴다) ② 스키마 텍스트 자체가 JSON 이
/// 아닌 경우.
/// ★스키마 **전체**를 훑는다★: 블록 안에서 선언한 struct 는 부모 스키마에 인라인으로 펼쳐지므로
/// 꼭대기만 보면 중첩 타입의 필드는 한 번도 검사되지 않는다.
pub fn lint_spec(spec: &CommandSpec) -> Result<(), String> {
    for (label, text) in [("args", spec.args_schema), ("ok", spec.ok_schema)] {
        let parsed: serde_json::Value = serde_json::from_str(text)
            .map_err(|e| format!("{}: {label} 스키마가 JSON 이 아니다 — {e}", spec.name))?;
        lint_node(spec.name, label, "", &parsed)?;
    }
    Ok(())
}

/// 스키마 트리 한 마디 — 객체 노드면 검사하고, 어느 자식(중첩 struct · `items` · `anyOf`)이든 따라 내려간다.
fn lint_node(
    command: &str,
    label: &str,
    path: &str,
    node: &serde_json::Value,
) -> Result<(), String> {
    let at = |path: &str| {
        if path.is_empty() {
            String::new()
        } else {
            format!(" (`{path}` 안)")
        }
    };
    match node {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::Array(variants)) = map.get("enum") {
                for variant in variants {
                    let Some(name) = variant.as_str() else {
                        continue;
                    };
                    if name.contains('#') {
                        return Err(format!(
                            "{command}: {label} enum 값 `{name}`{} — raw identifier 는 wire 이름(`{}`)과 스키마 이름이 갈린다. 다른 이름을 쓸 것.",
                            at(path),
                            name.trim_start_matches("r#")
                        ));
                    }
                }
            }
            if let Some(props) = map.get("properties").and_then(|p| p.as_object()) {
                for key in props.keys() {
                    if key.contains('#') {
                        return Err(format!(
                            "{command}: {label} 필드 `{key}`{} — raw identifier 는 wire 이름(`{}`)과 스키마 이름이 갈린다. 다른 이름을 쓸 것.",
                            at(path),
                            key.trim_start_matches("r#")
                        ));
                    }
                }
                match map.get("required") {
                    None => {}
                    Some(serde_json::Value::Array(required)) => {
                        for entry in required {
                            let Some(name) = entry.as_str() else {
                                return Err(format!(
                                    "{command}: {label} required{} 에 문자열 아닌 항목 `{entry}` 가 실렸다",
                                    at(path)
                                ));
                            };
                            if !props.contains_key(name) {
                                return Err(format!(
                                    "{command}: {label} required{} 에 없는 필드 `{name}` 가 실렸다",
                                    at(path)
                                ));
                            }
                        }
                    }
                    Some(other) => {
                        return Err(format!(
                            "{command}: {label} required{} 가 배열이 아니다 — `{other}`",
                            at(path)
                        ))
                    }
                }
            }
            for (key, child) in map {
                lint_node(command, label, &join(path, key), child)?;
            }
        }
        serde_json::Value::Array(items) => {
            for (i, child) in items.iter().enumerate() {
                lint_node(command, label, &join(path, &i.to_string()), child)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn join(path: &str, segment: &str) -> String {
    if path.is_empty() {
        segment.to_string()
    } else {
        format!("{path}.{segment}")
    }
}

/// 파생 파일이 밝히는 **부분스키마 방언**.
///
/// ★이 파일 자체는 스키마가 아니다★ — 이 칸은 아래 각 항목의 `args`·`ok` 가 **어느 판(draft)의**
/// JSON Schema 인지를 밝힌다. 밝히지 않으면 소비자는 `{"type":"integer"}` 가 `1.0` 을 포함하는지를
/// 알 수 없는데(draft 6 이전에는 포함하지 않았다), 표가 인자를 맞추는 규칙이 바로 그 성질 위에 선다
/// ([`crate::CommandTable::call`]).
const SCHEMA_DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/// crate 하나의 파생 스키마 파일 본문(TRD §2-4 ②) — 원소는 [`spec_item_json`] 과 **바이트 동일**하다.
pub fn catalog_json(catalog_version: u32, specs: &[&CommandSpec]) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{{\n  \"$schema\": \"{SCHEMA_DIALECT}\",\n  \"catalogVersion\": {catalog_version},\n  \"commands\": ["
    );
    for (i, spec) in specs.iter().enumerate() {
        let sep = if i + 1 == specs.len() { "" } else { "," };
        let _ = writeln!(out, "    {}{}", spec_item_json(spec), sep);
    }
    out.push_str("  ]\n}\n");
    out
}

fn push_json_string(out: &mut String, value: &str) {
    match serde_json::to_string(value) {
        Ok(quoted) => out.push_str(&quoted),
        // 문자열 직렬화는 실패할 수 없다(serde_json 은 문자열에 실패 경로가 없다). 그래도 패닉으로
        //   파생물을 죽이지 않고 빈 문자열로 접는다.
        Err(_) => out.push_str("\"\""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(args_schema: &'static str) -> CommandSpec {
        CommandSpec {
            name: "fixture.lint",
            effect: Effect::Read,
            since: 1,
            summary: "린트 픽스처",
            args_schema,
            ok_schema: "{\"type\":\"object\",\"properties\":{},\"required\":[]}",
            errors: &[],
            args_type: "LintArgs",
            ok_type: "LintOk",
        }
    }

    #[test]
    fn a_clean_schema_passes() {
        lint_spec(&spec(
            "{\"type\":\"object\",\"properties\":{\"target\":{\"type\":\"string\"}},\"required\":[\"target\"]}",
        ))
        .expect("깨끗한 선언");
    }

    #[test]
    fn a_top_level_raw_identifier_is_refused() {
        let err = lint_spec(&spec(
            "{\"type\":\"object\",\"properties\":{\"r#type\":{\"type\":\"string\"}},\"required\":[]}",
        ))
        .expect_err("raw identifier");
        assert!(err.contains("r#type"), "{err}");
    }

    /// ★중첩 struct 는 부모 스키마에 인라인으로 펼쳐진다★ — 꼭대기만 보면 이 필드는 한 번도 검사되지 않고
    /// 광고 이름(`r#type`)과 wire 이름(`type`)이 갈린 채 나간다.
    #[test]
    fn a_raw_identifier_inside_a_nested_object_is_refused() {
        let err = lint_spec(&spec(
            "{\"type\":\"object\",\"properties\":{\"row\":{\"type\":\"object\",\"properties\":{\"r#type\":{\"type\":\"string\"}},\"required\":[]}},\"required\":[\"row\"]}",
        ))
        .expect_err("중첩 raw identifier");
        assert!(err.contains("r#type"), "{err}");
        assert!(
            err.contains("properties.row"),
            "어디인지 말해야 한다: {err}"
        );
    }

    #[test]
    fn a_raw_identifier_inside_an_array_item_is_refused() {
        let err = lint_spec(&spec(
            "{\"type\":\"object\",\"properties\":{\"rows\":{\"type\":\"array\",\"items\":{\"type\":\"object\",\"properties\":{\"r#type\":{\"type\":\"string\"}},\"required\":[]}}},\"required\":[\"rows\"]}",
        ))
        .expect_err("배열 원소 안의 raw identifier");
        assert!(err.contains("items"), "{err}");
    }

    #[test]
    fn a_raw_identifier_inside_an_any_of_branch_is_refused() {
        let err = lint_spec(&spec(
            "{\"type\":\"object\",\"properties\":{\"row\":{\"anyOf\":[{\"type\":\"object\",\"properties\":{\"r#type\":{\"type\":\"string\"}},\"required\":[]},{\"type\":\"null\"}]}},\"required\":[]}",
        ))
        .expect_err("anyOf 갈래 안의 raw identifier");
        assert!(err.contains("anyOf"), "{err}");
    }

    #[test]
    fn a_nested_required_naming_a_missing_field_is_refused() {
        let err = lint_spec(&spec(
            "{\"type\":\"object\",\"properties\":{\"row\":{\"type\":\"object\",\"properties\":{\"id\":{\"type\":\"string\"}},\"required\":[\"gone\"]}},\"required\":[\"row\"]}",
        ))
        .expect_err("없는 필드가 required 에");
        assert!(err.contains("gone"), "{err}");
    }

    #[test]
    fn a_required_entry_that_is_not_a_string_is_refused() {
        let err = lint_spec(&spec(
            "{\"type\":\"object\",\"properties\":{\"id\":{\"type\":\"string\"}},\"required\":[7]}",
        ))
        .expect_err("문자열 아닌 required 항목");
        assert!(err.contains("문자열"), "{err}");
    }

    #[test]
    fn a_required_that_is_not_an_array_is_refused() {
        lint_spec(&spec(
            "{\"type\":\"object\",\"properties\":{\"id\":{\"type\":\"string\"}},\"required\":\"id\"}",
        ))
        .expect_err("배열 아닌 required");
    }

    #[test]
    fn a_schema_that_is_not_json_is_refused() {
        lint_spec(&spec("{not json")).expect_err("깨진 스키마");
    }

    /// ★enum variant 도 raw identifier 를 쓸 수 있다★ — 값은 문자열 배열이라 필드 이름만 훑으면 한 번도
    /// 검사되지 않고, 광고는 `"r#type"` 인데 serde 가 받는 것은 `"type"` 이 된다.
    #[test]
    fn a_raw_identifier_enum_variant_is_refused() {
        let err = lint_spec(&spec(
            "{\"type\":\"object\",\"properties\":{\"kind\":{\"enum\":[\"Fast\",\"r#type\"]}},\"required\":[\"kind\"]}",
        ))
        .expect_err("enum 값의 raw identifier");
        assert!(err.contains("r#type"), "{err}");
        assert!(
            err.contains("properties.kind"),
            "어디인지 말해야 한다: {err}"
        );
    }

    /// ★파일이 부분스키마의 방언을 밝힌다★ — 밝히지 않으면 소비자가 `{"type":"integer"}` 의 뜻을
    /// 판마다 다르게 읽는다(표의 인자 조정이 그 성질 위에 선다).
    #[test]
    fn the_catalog_declares_the_schema_dialect() {
        static ONE: CommandSpec = CommandSpec {
            name: "fixture.one",
            effect: Effect::Read,
            since: 1,
            summary: "하나",
            args_schema: "{\"type\":\"object\",\"properties\":{}}",
            ok_schema: "{\"type\":\"object\",\"properties\":{}}",
            errors: &[],
            args_type: "OneArgs",
            ok_type: "OneOk",
        };

        let body = catalog_json(1, &[&ONE]);
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("파생 파일은 JSON");

        assert_eq!(parsed["$schema"], SCHEMA_DIALECT);
        assert_eq!(parsed["catalogVersion"], 1);
        assert_eq!(parsed["commands"][0]["name"], "fixture.one");
    }
}
