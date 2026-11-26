//! Reading a hook folder's `hooks.json` — the hook settings shape the backends share,
//! `{"description"?: …, "hooks": {"<Event>": [{"matcher"?: …, "hooks": [{…}]}]}}`. Only what Engram's own assembly breaks
//! on is checked (ADR-0359): UTF-8 JSON (a BOM dropped), a top-level object whose `hooks` is an object, each event value a
//! list, each group an object whose `hooks` is a list of objects. Event names, matchers, handler types and fields and
//! script existence are each CLI's rules. Pure: no disk IO.
// ADR-0359
// ADR-0420

use engram_dashboard_agent::launch::{escape_controls, Reason};
use serde_json::{json, Map, Value};

/// The code of a `hooks.json` that fails the shape check (`args` = `{path, why}` — `path` = the hook folder); an item
/// problem of the hook, carried as problem ② on every preset entry that reaches it.
pub const HOOK_SHAPE_INVALID: &str = "hook-shape-invalid";

/// What a `hooks.json` reads as.
#[derive(Clone, Debug, PartialEq)]
pub struct HookDoc {
    /// The top-level `description` string, trimmed; empty when there is none or the file is no JSON object (display
    /// only).
    pub description: String,
    /// The `hooks` object as read, never rewritten; or why the file fails the shape check ([`HOOK_SHAPE_INVALID`] — a
    /// JSON syntax error carries its line).
    pub definition: Result<Map<String, Value>, Reason>,
}

/// Reads the `hooks.json` bytes of the hook folder `path` (a library path, named in a refusal).
pub fn read_hooks(bytes: &[u8], path: &str) -> HookDoc {
    let refused = |why: String, line: Option<u32>| Reason {
        code: HOOK_SHAPE_INVALID.to_owned(),
        args: json!({ "path": path, "why": why }),
        line,
    };
    let bytes = bytes.strip_prefix("\u{feff}".as_bytes()).unwrap_or(bytes);
    let Ok(text) = std::str::from_utf8(bytes) else {
        return HookDoc {
            description: String::new(),
            definition: Err(refused("the file is not valid UTF-8".to_owned(), None)),
        };
    };
    let top = match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(top)) => top,
        Ok(_) => {
            return HookDoc {
                description: String::new(),
                definition: Err(refused("the top level is not an object".to_owned(), None)),
            }
        }
        Err(e) => {
            let line = u32::try_from(e.line()).ok().filter(|&line| line > 0);
            return HookDoc {
                description: String::new(),
                definition: Err(refused(escape_controls(&e.to_string()), line)),
            };
        }
    };
    let description = top
        .get("description")
        .and_then(Value::as_str)
        .map(|description| description.trim().to_owned())
        .unwrap_or_default();
    HookDoc {
        description,
        definition: definition_of(&top).map_err(|why| refused(why, None)),
    }
}

/// The `hooks` object of `top` when it has the shape the assembly needs, else why not (event names escaped).
fn definition_of(top: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let hooks = match top.get("hooks") {
        Some(Value::Object(hooks)) => hooks,
        Some(_) => return Err("`hooks` is not an object".to_owned()),
        None => return Err("`hooks` is missing".to_owned()),
    };
    for (event, groups) in hooks {
        let event = escape_controls(event);
        let Value::Array(groups) = groups else {
            return Err(format!("`hooks.{event}` is not a list"));
        };
        for (g, group) in groups.iter().enumerate() {
            let Value::Object(group) = group else {
                return Err(format!("`hooks.{event}[{g}]` is not an object"));
            };
            let Some(Value::Array(handlers)) = group.get("hooks") else {
                return Err(format!("`hooks.{event}[{g}].hooks` is not a list"));
            };
            if let Some(h) = handlers.iter().position(|handler| !handler.is_object()) {
                return Err(format!("`hooks.{event}[{g}].hooks[{h}]` is not an object"));
            }
        }
    }
    Ok(hooks.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn why(text: &str) -> (String, Option<u32>) {
        let reason = read_hooks(text.as_bytes(), "team/hooks/h")
            .definition
            .unwrap_err();
        assert_eq!(reason.code, HOOK_SHAPE_INVALID);
        assert_eq!(reason.args["path"], "team/hooks/h");
        (reason.args["why"].as_str().unwrap().to_owned(), reason.line)
    }

    #[test]
    fn a_well_shaped_file_reads_its_hooks_object_untouched_and_its_description() {
        let text = "\u{feff}{\"description\": \"  notify on start \", \"hooks\": {\"SessionStart\": [{\"matcher\": 7, \
                    \"hooks\": [{\"type\": \"prompt\", \"x\": null}]}], \"Custom\": []}}";
        let doc = read_hooks(text.as_bytes(), "team/hooks/h");
        assert_eq!(doc.description, "notify on start");
        let hooks = doc.definition.unwrap();
        assert_eq!(
            Value::Object(hooks),
            json!({"SessionStart": [{"matcher": 7, "hooks": [{"type": "prompt", "x": null}]}], "Custom": []})
        );
    }

    #[test]
    fn an_empty_hooks_object_reads() {
        let doc = read_hooks(br#"{"hooks": {}}"#, "h");
        assert_eq!(doc.definition, Ok(Map::new()));
        assert_eq!(doc.description, "");
    }

    #[test]
    fn each_shape_failure_names_where_it_is() {
        assert_eq!(why("[1]").0, "the top level is not an object");
        assert_eq!(why("{}").0, "`hooks` is missing");
        assert_eq!(why(r#"{"hooks": []}"#).0, "`hooks` is not an object");
        assert_eq!(
            why(r#"{"hooks": {"Stop": {}}}"#).0,
            "`hooks.Stop` is not a list"
        );
        assert_eq!(
            why(r#"{"hooks": {"Stop": [1]}}"#).0,
            "`hooks.Stop[0]` is not an object"
        );
        assert_eq!(
            why(r#"{"hooks": {"Stop": [{}]}}"#).0,
            "`hooks.Stop[0].hooks` is not a list"
        );
        assert_eq!(
            why(r#"{"hooks": {"Stop": [{"hooks": []}, {"hooks": [{}, "x"]}]}}"#).0,
            "`hooks.Stop[1].hooks[1]` is not an object"
        );
    }

    #[test]
    fn a_syntax_error_carries_its_line_and_bytes_that_are_not_utf8_refuse() {
        let (_, line) = why("{\n\"hooks\": {\n,}\n}");
        assert_eq!(line, Some(3));
        let doc = read_hooks(&[b'{', 0xff, b'}'], "h");
        assert_eq!(
            doc.definition.unwrap_err().args["why"],
            "the file is not valid UTF-8"
        );
    }

    #[test]
    fn an_event_name_with_a_control_character_is_escaped_in_the_reason() {
        let (why, _) = why("{\"hooks\": {\"a\\nb\": 1}}");
        assert!(!why.contains('\n'), "{why}");
    }
}
