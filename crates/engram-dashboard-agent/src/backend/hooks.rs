//! Hook components, backend-neutral — the hook component a launch plan carries ([`HookRef`]), the placeholder rewrite
//! of its definition ([`rewrite`]) and the path rule that guards it ([`path_rule`]), which the launch check calls
//! too, so a hook the rewrite would refuse is refused on its entry before any launch.
//!
//! claude calls [`rewrite`] for one layer of its merged settings per hook component; the codex hook assembly
//! (`backend/codex/hook_assembly.rs`) calls it before it writes or hashes a component. Both backends answer
//! [`crate::backend::settings_check::ModeLoads::hooks`] `true` in both modes.
// ADR-0420

use std::ops::Range;
use std::path::{Path, PathBuf};

use engram_dashboard_platform::fs::forward_slash_text;
use engram_dashboard_platform::os::windows_paths;
use serde_json::{json, Map, Value};

use crate::launch::Reason;

/// Problem code: a hook's definition uses the placeholder while its folder path cannot be put into a command as is
/// (see [`path_rule`]). `args` = `{hook: library path, path: the folder as forward-slash text}`.
// ADR-0420
pub const HOOK_PATH_UNSAFE: &str = "hook-path-unsafe";

/// The placeholder, braced — replaced wherever it appears.
const BRACED: &str = "${CLAUDE_PLUGIN_ROOT}";
/// The placeholder, unbraced — replaced only when the next character cannot continue a variable name.
const UNBRACED: &str = "$CLAUDE_PLUGIN_ROOT";

/// One hook component to carry — a library folder holding `hooks.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRef {
    /// The hook folder's library path (`team/hooks/notify`) — what its notices and refusals name.
    pub path: String,
    /// The hook folder, absolute. Its scripts run in place; nothing is copied into the volatile folder.
    pub dir: PathBuf,
    /// The `hooks` object of its `hooks.json`, as read — placeholders not rewritten.
    pub definition: Map<String, Value>,
}

/// `hook`'s definition with `${CLAUDE_PLUGIN_ROOT}` and `$CLAUDE_PLUGIN_ROOT` replaced, in every string value at any
/// depth (object keys stay), by the hook folder as forward-slash text (`D:/Library/hooks/notify` — the spelling claude
/// itself substitutes on Windows). Every other text stays as written: `$CLAUDE_PLUGIN_DATA` (not supported) and
/// `$CLAUDE_PROJECT_DIR` included. `Err` = [`path_rule`]'s refusal — an unsafe folder path is never written into a
/// command.
// ADR-0420
pub fn rewrite(hook: &HookRef) -> Result<Map<String, Value>, Reason> {
    path_rule(&hook.path, &hook.dir, &hook.definition)?;
    let root = forward_slash_text(&hook.dir);
    Ok(hook
        .definition
        .iter()
        .map(|(key, value)| (key.clone(), rewritten(value, &root)))
        .collect())
}

/// The path rule of the placeholder: when a string value of `definition` uses it, the folder `dir` (forward-slash
/// text) must be UTF-8 and hold:
/// - no backtick;
/// - no typographic double quote `“` `”` `„` (U+201C · U+201D · U+201E) — PowerShell reads each as `"`, so it would
///   end the author's double quotes (measured, PowerShell 7.5);
/// - no `$` in the hook folder's own name (the last component) — the template's next text follows it (`notify$` with
///   `$CLAUDE_PLUGIN_ROOT-v2` reads `$-`);
/// - no `$` in any other component (the text between two `/`) unless it is that component's last character;
/// - on a host without Windows path rules, also no `"`, `\` or control character.
///
/// Inside the double quotes help recommends, Git Bash and PowerShell expand `$(…)`, `${…}`, `$name`, backticks and the
/// special parameters (`$$`, `$?`, `$1` …), while a `$` followed by `/` stays literal in both — so a hook below a
/// hidden share (`//srv/team$/library/…`) passes. A placeholder in single quotes or unquoted is the author's risk.
/// `Err` = [`HOOK_PATH_UNSAFE`] naming `hook` (the library path); a definition without the placeholder always passes.
// ADR-0420
pub fn path_rule(hook: &str, dir: &Path, definition: &Map<String, Value>) -> Result<(), Reason> {
    if !definition.values().any(uses_placeholder) {
        return Ok(());
    }
    let text = forward_slash_text(dir);
    if dir.to_str().is_some() && folder_text_is_safe(&text) {
        return Ok(());
    }
    Err(Reason {
        code: HOOK_PATH_UNSAFE.to_string(),
        args: json!({ "hook": hook, "path": text }),
        line: None,
    })
}

fn folder_text_is_safe(text: &str) -> bool {
    if text.contains(['`', '\u{201C}', '\u{201D}', '\u{201E}']) {
        return false;
    }
    if !windows_paths()
        && text
            .chars()
            .any(|c| c == '"' || c == '\\' || c.is_control())
    {
        return false;
    }
    let mut components = text.trim_end_matches('/').split('/').rev();
    let name = components.next().unwrap_or_default();
    !name.contains('$')
        && components.all(|component| {
            component
                .find('$')
                .is_none_or(|at| at + 1 == component.len())
        })
}

/// The byte range of the first placeholder in `text`.
fn next_placeholder(text: &str) -> Option<Range<usize>> {
    let continues_name = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut from = 0;
    while let Some(found) = text[from..].find('$') {
        let at = from + found;
        let tail = &text[at..];
        if tail.starts_with(BRACED) {
            return Some(at..at + BRACED.len());
        }
        if let Some(after) = tail.strip_prefix(UNBRACED) {
            if !after.starts_with(continues_name) {
                return Some(at..at + UNBRACED.len());
            }
        }
        from = at + 1;
    }
    None
}

fn uses_placeholder(value: &Value) -> bool {
    match value {
        Value::String(text) => next_placeholder(text).is_some(),
        Value::Array(items) => items.iter().any(uses_placeholder),
        Value::Object(map) => map.values().any(uses_placeholder),
        _ => false,
    }
}

fn rewritten(value: &Value, root: &str) -> Value {
    match value {
        Value::String(text) => {
            let mut out = String::with_capacity(text.len());
            let mut rest = text.as_str();
            while let Some(found) = next_placeholder(rest) {
                out.push_str(&rest[..found.start]);
                out.push_str(root);
                rest = &rest[found.end..];
            }
            out.push_str(rest);
            Value::String(out)
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| rewritten(item, root)).collect())
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), rewritten(value, root)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(value: Value) -> Map<String, Value> {
        match value {
            Value::Object(map) => map,
            other => panic!("not an object: {other}"),
        }
    }

    fn hook(dir: &str, value: Value) -> HookRef {
        HookRef {
            path: "team/hooks/notify".to_string(),
            dir: PathBuf::from(dir),
            definition: definition(value),
        }
    }

    /// One handler running `command` on `Stop`.
    fn running(command: &str) -> Value {
        json!({ "Stop": [{ "hooks": [{ "type": "command", "command": command }] }] })
    }

    /// The folder used where the path rule is not under test — a plain absolute path on every OS.
    const FOLDER: &str = "/lib/hooks/notify";

    /// Both spellings are replaced in every string value, at any depth, by the folder with forward slashes; keys,
    /// non-string values and every other variable stay as written.
    #[test]
    fn both_placeholder_spellings_become_the_hook_folder_in_every_string_value() {
        let written = json!({
            "PreToolUse": [{
                "matcher": "${CLAUDE_PLUGIN_ROOT}",
                "hooks": [{
                    "type": "command",
                    "command": "node \"${CLAUDE_PLUGIN_ROOT}/guard.js\" $CLAUDE_PLUGIN_ROOT/x $CLAUDE_PLUGIN_ROOT",
                    "timeout": 30,
                    "async": false,
                    "args": ["$CLAUDE_PLUGIN_ROOT/a", null],
                }],
            }],
            "${CLAUDE_PLUGIN_ROOT}": [],
        });
        let rewritten = rewrite(&hook(FOLDER, written)).unwrap();
        assert_eq!(
            Value::Object(rewritten),
            json!({
                "PreToolUse": [{
                    "matcher": FOLDER,
                    "hooks": [{
                        "type": "command",
                        "command": format!("node \"{FOLDER}/guard.js\" {FOLDER}/x {FOLDER}"),
                        "timeout": 30,
                        "async": false,
                        "args": [format!("{FOLDER}/a"), null],
                    }],
                }],
                "${CLAUDE_PLUGIN_ROOT}": [],
            })
        );
    }

    /// An unbraced placeholder followed by a name character is another variable; other variables, a lone `$` and a
    /// definition without the placeholder come back unchanged.
    #[test]
    fn other_variables_and_longer_names_stay_as_written() {
        for command in [
            "$CLAUDE_PLUGIN_ROOTS/x",
            "$CLAUDE_PLUGIN_ROOT_2/x",
            "$CLAUDE_PLUGIN_DATA/x",
            "${CLAUDE_PLUGIN_DATA}/x",
            "$CLAUDE_PROJECT_DIR/x",
            "${CLAUDE_PLUGIN_ROOT/x",
            "echo $ $$ ${x}",
        ] {
            assert_eq!(
                Value::Object(rewrite(&hook(FOLDER, running(command))).unwrap()),
                running(command),
                "{command}"
            );
        }
        assert_eq!(
            Value::Object(rewrite(&hook(FOLDER, running("$CLAUDE_PLUGIN_ROOT-x"))).unwrap()),
            running(&format!("{FOLDER}-x"))
        );
    }

    /// On Windows the folder's `\` become `/` in the replacement; elsewhere a `\` in the folder is refused.
    #[test]
    fn the_replacement_uses_forward_slashes_on_windows_and_a_backslash_is_refused_elsewhere() {
        let spelled = r"D:\Library\hooks\notify";
        let found = rewrite(&hook(spelled, running("node ${CLAUDE_PLUGIN_ROOT}/x.js")));
        if windows_paths() {
            assert_eq!(
                Value::Object(found.unwrap()),
                running("node D:/Library/hooks/notify/x.js")
            );
        } else {
            assert_eq!(found.unwrap_err().code, HOOK_PATH_UNSAFE);
        }
    }

    /// The path rule with the placeholder used: a `$` ending a component above the hook folder passes (a hidden share);
    /// `$$`, `$?`, `$1`, `$name`, `${x}`, `$(x)`, a backtick, each of `“ ” „` and any `$` in the hook folder's own
    /// name (even last) are refused, one fixture each, naming the hook and the folder.
    #[test]
    fn a_folder_path_the_shells_would_expand_is_refused_when_the_placeholder_is_used() {
        let uses = running("node \"${CLAUDE_PLUGIN_ROOT}/x.js\"");
        for folder in [
            "//srv/team$/x",
            "//srv/team$/library/hooks/notify",
            "/lib/$/x",
            "/lib/a b/x",
            "/lib/a\u{2018}b\u{FF02}/x",
        ] {
            assert_eq!(
                path_rule(
                    "team/hooks/notify",
                    Path::new(folder),
                    &definition(uses.clone())
                ),
                Ok(()),
                "{folder}"
            );
        }
        for folder in [
            "/lib/notify$$",
            "/lib/a$?/x",
            "/lib/a$1/x",
            "/lib/$name/x",
            "/lib/${x}/y",
            "/lib/$(x)/y",
            "/lib/a`x`/y",
            "/lib/end`",
            "/lib/a\u{201C}b/x",
            "/lib/a\u{201D};x;\u{201C}b/x",
            "/lib/a\u{201E}b/x",
            "/lib/hooks/notify$",
            "/lib/hooks/notify$/",
            "/lib/team$",
        ] {
            assert_eq!(
                path_rule(
                    "team/hooks/notify",
                    Path::new(folder),
                    &definition(uses.clone())
                ),
                Err(Reason {
                    code: HOOK_PATH_UNSAFE.to_string(),
                    args: json!({ "hook": "team/hooks/notify", "path": folder }),
                    line: None,
                }),
                "{folder}"
            );
        }
    }

    /// `"`, `\` and a control character are refused only on a host without Windows path rules (on Windows a `\` is a
    /// separator and the others cannot be in a path).
    #[test]
    fn quote_backslash_and_control_characters_are_refused_off_windows_only() {
        let uses = definition(running("$CLAUDE_PLUGIN_ROOT/x"));
        for folder in ["/lib/a\"b", r"/lib/a\b", "/lib/a\u{7}b"] {
            assert_eq!(
                path_rule("h", Path::new(folder), &uses).is_err(),
                !windows_paths(),
                "{folder:?}"
            );
        }
    }

    /// A folder path that is not UTF-8 is refused — its lossy text would name another folder in the command.
    #[test]
    fn a_folder_path_that_is_not_utf8_is_refused() {
        let folder = engram_dashboard_platform::testing::non_utf8_path("/lib/hooks/notify");
        let refused = path_rule(
            "team/hooks/notify",
            &folder,
            &definition(running("$CLAUDE_PLUGIN_ROOT/x")),
        )
        .unwrap_err();
        assert_eq!(refused.code, HOOK_PATH_UNSAFE);
        assert_eq!(refused.args["path"], forward_slash_text(&folder));
    }

    /// Without the placeholder no folder path is refused — the path never reaches a command through Engram.
    #[test]
    fn a_definition_without_the_placeholder_passes_whatever_its_folder() {
        let plain = definition(running("node /opt/x.js $CLAUDE_PLUGIN_DATA"));
        for folder in ["/lib/notify$$", "/lib/a`b", "/lib/$(x)"] {
            assert_eq!(
                path_rule("h", Path::new(folder), &plain),
                Ok(()),
                "{folder}"
            );
            assert_eq!(
                rewrite(&HookRef {
                    path: "h".to_string(),
                    dir: PathBuf::from(folder),
                    definition: plain.clone(),
                }),
                Ok(plain.clone()),
                "{folder}"
            );
        }
    }

    /// The rewrite itself refuses an unsafe folder path — it never writes it into a command.
    #[test]
    fn the_rewrite_refuses_an_unsafe_folder_path() {
        let refused =
            rewrite(&hook("/lib/notify$$", running("$CLAUDE_PLUGIN_ROOT/x.js"))).unwrap_err();
        assert_eq!(refused.code, HOOK_PATH_UNSAFE);
        assert_eq!(
            refused.args,
            json!({ "hook": "team/hooks/notify", "path": "/lib/notify$$" })
        );
    }
}
