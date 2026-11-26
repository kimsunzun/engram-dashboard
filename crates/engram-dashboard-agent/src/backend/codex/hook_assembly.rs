//! The codex hook assembly — the one function ([`hook_assembly`]) that turns a launch's hook inputs into what codex
//! reads: the plan's hook components, the raw settings hooks lines folded with codex's own last-wins rule, and a trust
//! entry for every handler Engram writes. The check (`command_line_fits`, through the port) and `materialize` call it
//! through [`super::cmdline::assembled`], so the check counts exactly what goes on the line.
//!
//! - **Only what codex reads is written** — codex's 14 events ([`EVENTS`]), a group's `matcher` and `hooks`, and the
//!   typed handler fields codex normalizes. A handler Engram cannot trust (another event, a type other than
//!   `command`, a field it cannot normalize) or a group whose `matcher` is not a string is left off and named in a
//!   [`CODEX_HOOKS_LEFT_OFF`] notice. Indexes in the trust keys are positions in the arrays Engram writes (an emptied
//!   group is not written), which is how codex numbers what it receives.
//! - **Raw settings hooks lines are always consumed** — every `hooks`, `hooks.<Event>` and `hooks.state` line leaves
//!   the non-hook lines and is folded, carried or left off with a notice; none is re-emitted raw. Hook output is
//!   written only when at least one group or one carried state entry would be written.
//! - **Trust is generated only for Engram's own session-flags keys** (`enabled = true` and the hash); other-source
//!   state entries of the effective raw result are carried verbatim, keys trimmed.
//! - **Pure** — no disk, no process, no log: a dropped raw session-flags state entry is handed back
//!   ([`HookAssembly::dropped_state_keys`]) for `materialize` to log once per launch.
//!
//! ★Codex behaviour copied by hand from the codex source at `rust-v0.171.0` (openai/codex, Apache-2.0)★: the 14 events
//!   and handler fields (`protocol/src/hooks.rs`), their normalization and the hash (`hooks/src/engine/discovery.rs`
//!   `append_matcher_groups` · `hook_hash` → `config/src/fingerprint.rs` `version_for_toml`), the matcher drop
//!   (`hooks/src/events/common.rs` `matcher_pattern_for_event`), the key (`hooks/src/lib.rs` `hook_key`, the
//!   session-flags source from `discovery.rs` `synthetic_layer_path`), state-key trimming (`hooks/src/config_rules.rs`),
//!   the `-c` split and value parse (`utils/cli/src/config_override.rs`) and the last-wins override
//!   (`config/src/overrides.rs`). A codex release that changes one of them leaves an Engram hook untrusted: JSON mode's
//!   post-open check names it, terminal mode shows it only in the lane.
// ADR-0353
// ADR-0359
// ADR-0420

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;

use engram_dashboard_platform::os::windows_paths;
use engram_dashboard_platform::shell::text_is_cli_safe;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::CONFIG_OVERRIDE_FLAG;
use crate::backend::hooks;
use crate::backend::settings_check::{
    CommandLine, ConfigOrigin, SUBJECT_HOOK_ARG, SUBJECT_SOURCE_ARG,
};
use crate::launch::{Notice, Reason, CODEX_HOOKS_LEFT_OFF, CODEX_RAW_HOOKS_TRUSTED};
use crate::profile::AgentOutputFormat;

/// Problem code: a raw settings hooks line Engram cannot fold — its value does not parse as TOML, its path goes
/// deeper than `hooks.<Event>` / `hooks.state`, its shape is not the one codex reads (a `hooks` table, an event's
/// array of group tables whose `hooks` is an array of tables, a `state` table of tables with no two keys equal after
/// trimming), or it holds a float or a date-time. `args` = `{index, line}` from [`hooks_line_readable`], or
/// `{source, line}` (the line's text; the reason's `line` is the field's line) from the assembly.
// ADR-0420
pub(crate) const CODEX_HOOKS_UNREADABLE: &str = "codex-hooks-unreadable";

/// Problem code: a hook line the codex terminal command line cannot carry — a group (or a state entry) whose written
/// text holds an odd number of `"`, a value the TOML writer refuses, or a character [`text_is_cli_safe`] refuses.
/// The last is a writer-regression backstop only: the writer escapes [`CMD_ESCAPED`] in every string, so no input
/// reaches it. `args` = `{hook | source, event, why}`, `why` = `odd-quotes` · `value` · `unsafe-char`.
// ADR-0353
// ADR-0420
pub(crate) const CODEX_HOOK_LINE_REFUSED: &str = "codex-hook-line-refused";

/// Problem code: a pass-through argument sets `hooks` or `hooks.*` while the launch has hook inputs. `args` =
/// `{arg, advice}` — the override value as codex's clap reads it, and [`PASSTHROUGH_HOOKS_ADVICE`].
// ADR-0420
pub(crate) const CODEX_PASSTHROUGH_HOOKS: &str = "codex-passthrough-hooks";

/// The guidance a [`CODEX_PASSTHROUGH_HOOKS`] refusal carries.
// ADR-0420: decision 11
const PASSTHROUGH_HOOKS_ADVICE: &str =
    "move this hooks line into a hook component or a codex settings asset";

/// codex's 14 hook events, spelled as `HookEventsToml` names them, each with its key label (`hook_event_key_label`).
/// codex ignores any other event name, so nothing else is written.
const EVENTS: [(&str, &str); 14] = [
    ("PreModelRequest", "pre_model_request"),
    ("PostModelRequest", "post_model_request"),
    ("PreToolUse", "pre_tool_use"),
    ("PermissionRequest", "permission_request"),
    ("PostToolUse", "post_tool_use"),
    ("PreCompact", "pre_compact"),
    ("PostCompact", "post_compact"),
    ("SessionStart", "session_start"),
    ("SessionEnd", "session_end"),
    ("UserPromptSubmit", "user_prompt_submit"),
    ("SubagentStart", "subagent_start"),
    ("SubagentStop", "subagent_stop"),
    ("Stop", "stop"),
    ("Interrupt", "interrupt"),
];

/// The events whose matcher codex drops before hashing (`matcher_pattern_for_event`).
const MATCHER_DROPPED: [&str; 5] = [
    "PreModelRequest",
    "PostModelRequest",
    "UserPromptSubmit",
    "Stop",
    "Interrupt",
];

/// The events whose `additionalContextLimit` codex keeps; on any other event it is ignored (and not hashed).
const CONTEXT_LIMIT_EVENTS: [&str; 5] = [
    "PreToolUse",
    "PostToolUse",
    "SessionStart",
    "UserPromptSubmit",
    "SubagentStart",
];

/// codex's default `additionalContextLimit` (`DEFAULT_HOOK_OUTPUT_TOKEN_LIMIT`) — hashed as absent.
const DEFAULT_CONTEXT_LIMIT: u64 = 2_500;

/// The events whose timeout codex defaults to 1 second and clamps to 1..=3 (every other event: 600, at least 1).
const SHORT_TIMEOUT_EVENTS: [&str; 2] = ["SessionEnd", "Interrupt"];

/// The `-c` path of the hooks table, and its state member — a member name, never an event.
const HOOKS_KEY: &str = "hooks";
const STATE_KEY: &str = "state";

/// The refusal text over the terminal budget with hook lines on the line.
// ADR-0420
pub(super) const TERMINAL_HOOK_BUDGET_ADVICE: &str =
    "codex terminal command line limit exceeded — reduce hooks or use JSON mode";

/// Why a handler or group is left off ([`CODEX_HOOKS_LEFT_OFF`] `why`).
const WHY_EVENT: &str = "event";
const WHY_TYPE: &str = "type";
const WHY_FIELD: &str = "field";
const WHY_MATCHER: &str = "matcher";

/// What the assembly produced for one launch.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct HookAssembly {
    /// The raw settings' arguments minus their hooks lines (each line leaves with its `-c`), in order.
    pub(super) config_args: Vec<String>,
    /// Terminal: `-c`, `hooks.<Event>=[…]` per event in sorted order, then `-c`, `hooks.state={…}` — placed right
    /// after `config_args`. Empty when nothing is written, and always in JSON mode.
    pub(super) hook_args: Vec<String>,
    /// JSON: the thread config's `hooks` object (events and `state`). `None` when nothing is written, and always in
    /// terminal mode — never an empty object.
    pub(super) hook_config: Option<Value>,
    /// One trust entry per handler written, in write order — what codex must list trusted and enabled.
    pub(super) expected: Vec<TrustEntry>,
    /// [`CODEX_HOOKS_LEFT_OFF`] notices in the order met, then one [`CODEX_RAW_HOOKS_TRUSTED`].
    pub(super) notices: Vec<Notice>,
    /// The trimmed keys of the effective raw `state` entries that name a session-flags hook — dropped, because Engram
    /// writes every session-flags entry itself. Handed back instead of logged here, so the check's repeated calls do
    /// not repeat the warning (`materialize` logs it).
    pub(super) dropped_state_keys: Vec<String>,
}

/// The trust entry Engram writes for one handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TrustEntry {
    /// `<session-flags source>:<event label>:<group index>:<handler index>`, unescaped.
    pub(super) key: String,
    /// `sha256:<hex>` — codex's hash of the normalized handler.
    pub(super) hash: String,
    /// The hook component or settings asset the handler came from.
    pub(super) subject: Subject,
}

/// What a notice, a refusal or a written group came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Subject {
    /// A hook component — its library path.
    Hook(String),
    /// A settings asset at its field's line.
    Source(ConfigOrigin),
    /// A raw line whose origin the caller did not give.
    Unnamed,
}

impl Subject {
    /// A reason of `code` with `args` (an object) plus the subject's key (`hook` or `source`), and the field's line
    /// for a settings asset.
    fn reason(&self, code: &str, args: Value) -> Reason {
        let Value::Object(mut args) = args else {
            unreachable!("reason args are built as objects");
        };
        let line = match self {
            Subject::Hook(path) => {
                args.insert(SUBJECT_HOOK_ARG.to_owned(), json!(path));
                None
            }
            Subject::Source(origin) => {
                args.insert(SUBJECT_SOURCE_ARG.to_owned(), json!(origin.source));
                origin.line
            }
            Subject::Unnamed => None,
        };
        Reason {
            code: code.to_owned(),
            args: Value::Object(args),
            line,
        }
    }
}

/// The session-flags layer's source path as codex spells it in a trust key (it hard-codes `C:\` on Windows).
fn session_flags_source() -> &'static str {
    if windows_paths() {
        r"C:\<session-flags>\config.toml"
    } else {
        "/<session-flags>/config.toml"
    }
}

/// The assembly for `line` (the same input the check counts): `config_args` with their origins, the pass-through
/// `extra_args`, the plan's `hooks` and the mode. `Err`, in this order:
/// - [`CODEX_PASSTHROUGH_HOOKS`] — a hook component is attached or a raw settings line is a hooks line, and a
///   pass-through argument sets `hooks` (decided on the inputs, whatever is written);
/// - [`CODEX_HOOKS_UNREADABLE`] — a raw hooks line Engram cannot fold, on its settings asset;
/// - `hook-path-unsafe` — a component's placeholder rewrite refused ([`hooks::rewrite`]);
/// - [`CODEX_HOOK_LINE_REFUSED`] — terminal only: a written group or state entry the line cannot carry.
// ADR-0420
pub(super) fn hook_assembly(line: &CommandLine<'_>) -> Result<HookAssembly, Reason> {
    let (config_args, raw_lines) = split_hooks_lines(line.config_args, line.config_origins);
    if !line.hooks.is_empty() || !raw_lines.is_empty() {
        if let Some(arg) = passthrough_hooks_arg(line.extra_args) {
            return Err(Reason {
                code: CODEX_PASSTHROUGH_HOOKS.to_owned(),
                args: json!({ "arg": arg, "advice": PASSTHROUGH_HOOKS_ADVICE }),
                line: None,
            });
        }
    }
    let mut built = Built::default();
    let mut carried: Vec<Carried> = Vec::new();
    let mut dropped_state_keys = Vec::new();
    for (name, member) in fold(&raw_lines)? {
        if name == STATE_KEY {
            (carried, dropped_state_keys) = carried_state(&member.value, &member.subject);
        } else {
            built.event(&member.subject, &name, &member.value);
        }
    }
    for hook in line.hooks {
        let definition = hooks::rewrite(hook)?;
        let subject = Subject::Hook(hook.path.clone());
        for (event, groups) in &definition {
            built.event(&subject, event, groups);
        }
    }
    let expected = built.trust_entries();
    let (mut hook_args, mut hook_config) = (Vec::new(), None);
    if !built.events.is_empty() || !carried.is_empty() {
        match line.mode {
            AgentOutputFormat::Terminal => {
                hook_args = terminal_lines(&built, &expected, &carried)?;
            }
            AgentOutputFormat::StreamJson => {
                hook_config = Some(thread_config(&built, &expected, &carried));
            }
        }
    }
    let trusted = raw_trusted_notice(&built);
    let mut notices = built.left_off;
    notices.extend(trusted);
    Ok(HookAssembly {
        config_args,
        hook_args,
        hook_config,
        expected,
        notices,
        dropped_state_keys,
    })
}

/// Whether one raw codex `-c` line is a hooks line (`hooks`, `hooks.<name>` or deeper) — the one predicate that both
/// splits the hooks lines out of `config_args` here and exempts them from `cmdline::config_args`'s line guard (the
/// assembly rewrites them, so none reaches the command line as written).
// ADR-0420
pub(super) fn is_hooks_line(line: &str) -> bool {
    hooks_path(line).is_some()
}

/// Whether one raw codex `-c` line (at `index` of its raw value) is either not a hooks line or one Engram can fold.
/// `Err` = [`CODEX_HOOKS_UNREADABLE`] `{index, line}` — the settings asset's problem at its field's line.
// ADR-0420
pub(super) fn hooks_line_readable(index: usize, line: &str) -> Result<(), Reason> {
    read_hooks_line(line).map(|_| ()).map_err(|()| Reason {
        code: CODEX_HOOKS_UNREADABLE.to_owned(),
        args: json!({ "index": index, "line": line }),
        line: None,
    })
}

// ── raw settings hooks lines ─────────────────────────────────────────────────────

/// Where a raw hooks line writes in the `hooks` table.
#[derive(Debug, Clone, PartialEq, Eq)]
enum HooksPath {
    /// `hooks` — replaces the whole table.
    Table,
    /// `hooks.<name>` — one member (an event, or `state`).
    Member(String),
}

/// `config_args` split into the non-hook arguments and the hooks lines (each leaving with its `-c`), each line with
/// the origin at its own index.
fn split_hooks_lines(
    config_args: &[String],
    config_origins: &[ConfigOrigin],
) -> (Vec<String>, Vec<(String, Subject)>) {
    let mut kept = Vec::new();
    let mut raw = Vec::new();
    for (pair_index, pair) in config_args.chunks(2).enumerate() {
        match pair {
            [flag, value] if flag == CONFIG_OVERRIDE_FLAG && is_hooks_line(value) => {
                let subject = config_origins
                    .get(pair_index * 2 + 1)
                    .map_or(Subject::Unnamed, |origin| Subject::Source(origin.clone()));
                raw.push((value.clone(), subject));
            }
            _ => kept.extend(pair.iter().cloned()),
        }
    }
    (kept, raw)
}

/// The path of a hooks line, `None` for any other line: the key (before the first `=`) is trimmed, then split on
/// `.` with segments untrimmed and case kept, as codex does. `Some(Err(()))` = a deeper path, which cannot be folded.
fn hooks_path(line: &str) -> Option<Result<HooksPath, ()>> {
    let (key, _) = line.split_once('=')?;
    let mut segments = key.trim().split('.');
    if segments.next() != Some(HOOKS_KEY) {
        return None;
    }
    Some(match (segments.next(), segments.next()) {
        (None, _) => Ok(HooksPath::Table),
        (Some(name), None) => Ok(HooksPath::Member(name.to_owned())),
        (Some(_), Some(_)) => Err(()),
    })
}

/// A raw line read: `Ok(None)` = not a hooks line; `Err(())` = a hooks line Engram cannot fold.
fn read_hooks_line(line: &str) -> Result<Option<(HooksPath, Value)>, ()> {
    let (Some(path), Some((_, text))) = (hooks_path(line), line.split_once('=')) else {
        return Ok(None);
    };
    let path = path?;
    // codex parses the trimmed value as `_x_ = <value>`; a value that does not parse becomes a string there, which
    //   its hooks table cannot hold.
    let table: toml::Table = toml::from_str(&format!("_x_ = {}", text.trim())).map_err(|_| ())?;
    let value = json_of(table.get("_x_").ok_or(())?).ok_or(())?;
    let readable = match &path {
        HooksPath::Table => value
            .as_object()
            .is_some_and(|members| members.iter().all(|(name, v)| member_readable(name, v))),
        HooksPath::Member(name) => member_readable(name, &value),
    };
    if readable {
        Ok(Some((path, value)))
    } else {
        Err(())
    }
}

/// A TOML value as JSON — `None` for a float or a date-time, which no codex hook field holds and the line writer does
/// not write.
fn json_of(value: &toml::Value) -> Option<Value> {
    Some(match value {
        toml::Value::String(text) => Value::String(text.clone()),
        toml::Value::Integer(number) => Value::from(*number),
        toml::Value::Boolean(flag) => Value::Bool(*flag),
        toml::Value::Array(items) => {
            Value::Array(items.iter().map(json_of).collect::<Option<_>>()?)
        }
        toml::Value::Table(table) => Value::Object(
            table
                .iter()
                .map(|(key, v)| Some((key.clone(), json_of(v)?)))
                .collect::<Option<_>>()?,
        ),
        toml::Value::Float(_) | toml::Value::Datetime(_) => return None,
    })
}

/// Whether one member of the hooks table has the shape the assembly folds: `state` a table of tables with no two
/// keys equal after trimming; one of the 14 events an array of group tables whose `hooks` (if present) is an array of
/// tables. Another event name is left off whole, whatever it holds.
fn member_readable(name: &str, value: &Value) -> bool {
    if name == STATE_KEY {
        let Some(entries) = value.as_object() else {
            return false;
        };
        let mut seen = HashSet::new();
        return entries
            .iter()
            .all(|(key, entry)| entry.is_object() && seen.insert(key.trim()));
    }
    if event_label(name).is_none() {
        return true;
    }
    value.as_array().is_some_and(|groups| {
        groups.iter().all(|group| {
            group
                .as_object()
                .is_some_and(|group| match group.get(HOOKS_KEY) {
                    None => true,
                    Some(handlers) => handlers
                        .as_array()
                        .is_some_and(|handlers| handlers.iter().all(Value::is_object)),
                })
        })
    })
}

/// One member of the effective hooks table and the line that set it.
struct Member {
    value: Value,
    subject: Subject,
}

/// codex's last-wins over every raw hooks line (attach order, then file order): a `hooks` line replaces the whole
/// table, `state` included; a `hooks.<name>` line sets that member. The effective members, by name.
fn fold(raw_lines: &[(String, Subject)]) -> Result<BTreeMap<String, Member>, Reason> {
    let mut table = BTreeMap::new();
    for (text, subject) in raw_lines {
        let Ok(Some((path, value))) = read_hooks_line(text) else {
            return Err(subject.reason(CODEX_HOOKS_UNREADABLE, json!({ "line": text })));
        };
        let member = |value| Member {
            value,
            subject: subject.clone(),
        };
        match (path, value) {
            (HooksPath::Table, Value::Object(members)) => {
                table = members
                    .into_iter()
                    .map(|(name, value)| (name, member(value)))
                    .collect();
            }
            (HooksPath::Table, _) => unreachable!("read_hooks_line checked the table"),
            (HooksPath::Member(name), value) => {
                table.insert(name, member(value));
            }
        }
    }
    Ok(table)
}

/// One carried state entry: its trimmed key, its value and the settings asset it came from.
type Carried = (String, Value, Subject);

/// The other-source entries of the effective raw `state`, keys trimmed, and the trimmed keys dropped. An entry whose
/// trimmed key is a session-flags key is dropped: Engram rebuilds every session-flags position, so the raw one no longer
/// names the same handler. An empty key is dropped as codex drops it (not reported).
// ADR-0420
fn carried_state(state: &Value, subject: &Subject) -> (Vec<Carried>, Vec<String>) {
    let session_flags = format!("{}:", session_flags_source());
    let mut carried = Vec::new();
    let mut dropped = Vec::new();
    for (key, entry) in state.as_object().into_iter().flatten() {
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        if key.starts_with(&session_flags) {
            dropped.push(key.to_owned());
            continue;
        }
        carried.push((key.to_owned(), entry.clone(), subject.clone()));
    }
    (carried, dropped)
}

// ── groups, handlers and trust ───────────────────────────────────────────────────

/// One group as written: its subject, its matcher and the handlers kept.
struct Group {
    subject: Subject,
    matcher: Option<String>,
    handlers: Vec<Handler>,
}

/// One `command` handler with the fields codex reads, as given (`None` = absent or `null`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Handler {
    command: String,
    command_windows: Option<String>,
    timeout: Option<u64>,
    is_async: Option<bool>,
    status_message: Option<String>,
    context_limit: Option<u64>,
}

/// The groups written per event (sorted by event name) and the left-off notices met on the way.
#[derive(Default)]
struct Built {
    events: BTreeMap<&'static str, Vec<Group>>,
    left_off: Vec<Notice>,
}

impl Built {
    /// One event of one subject: its groups appended after what is there (raw groups come first, then components in
    /// plan order).
    fn event(&mut self, subject: &Subject, event: &str, groups: &Value) {
        let Some((name, _)) = EVENTS.iter().find(|(name, _)| *name == event) else {
            self.leave_off(subject, WHY_EVENT, event);
            return;
        };
        let Some(groups) = groups.as_array() else {
            self.leave_off(subject, WHY_FIELD, event);
            return;
        };
        for group in groups {
            let Some(group) = group.as_object() else {
                self.leave_off(subject, WHY_FIELD, event);
                continue;
            };
            let matcher = match group.get("matcher") {
                None | Some(Value::Null) => None,
                Some(Value::String(matcher)) => Some(matcher.clone()),
                Some(_) => {
                    self.leave_off(subject, WHY_MATCHER, event);
                    continue;
                }
            };
            let handlers = match group.get(HOOKS_KEY) {
                None => &[][..],
                Some(Value::Array(handlers)) => handlers,
                Some(_) => {
                    self.leave_off(subject, WHY_FIELD, event);
                    continue;
                }
            };
            let mut kept = Vec::new();
            for handler in handlers {
                match checked_handler(handler) {
                    Ok(handler) => kept.push(handler),
                    Err(why) => self.leave_off(subject, why, event),
                }
            }
            if !kept.is_empty() {
                self.events.entry(name).or_default().push(Group {
                    subject: subject.clone(),
                    matcher,
                    handlers: kept,
                });
            }
        }
    }

    fn leave_off(&mut self, subject: &Subject, why: &str, event: &str) {
        let notice = subject.reason(CODEX_HOOKS_LEFT_OFF, json!({ "why": why, "event": event }));
        if !self.left_off.contains(&notice) {
            self.left_off.push(notice);
        }
    }

    /// One entry per written handler, in write order (events sorted, then group, then handler).
    fn trust_entries(&self) -> Vec<TrustEntry> {
        let mut entries = Vec::new();
        for (event, groups) in &self.events {
            for (group_index, group) in groups.iter().enumerate() {
                for (handler_index, handler) in group.handlers.iter().enumerate() {
                    entries.push(TrustEntry {
                        key: trust_key(event, group_index, handler_index),
                        hash: trust_hash(event, group.matcher.as_deref(), handler),
                        subject: group.subject.clone(),
                    });
                }
            }
        }
        entries
    }
}

/// A handler Engram can trust, or why it is left off: `type` must be `"command"`; `command` a string, and the command
/// codex runs here (`commandWindows` on Windows when set) not blank; `commandWindows` (or its alias
/// `command_windows`, not both) and `statusMessage` strings; `timeout` and `additionalContextLimit` integers from 0 to
/// `i64::MAX`; `async` a boolean. `null` counts as absent where codex reads an optional field. Any other field is not
/// written. ★The strictness is the point★ — on one type error codex drops the whole session-flags `hooks` table, so
/// one bad handler would silence every Engram hook of the run. `mcp_tool` is left off by choice (not measured).
// ADR-0420
fn checked_handler(handler: &Value) -> Result<Handler, &'static str> {
    let fields = handler.as_object().ok_or(WHY_FIELD)?;
    if fields.get("type").and_then(Value::as_str) != Some("command") {
        return Err(WHY_TYPE);
    }
    let command = fields
        .get("command")
        .and_then(Value::as_str)
        .ok_or(WHY_FIELD)?;
    let command_windows = match (fields.get("commandWindows"), fields.get("command_windows")) {
        (Some(_), Some(_)) => return Err(WHY_FIELD),
        (Some(value), None) | (None, Some(value)) => optional_text(value)?,
        (None, None) => None,
    };
    let is_async = match fields.get("async") {
        None => None,
        Some(Value::Bool(flag)) => Some(*flag),
        Some(_) => return Err(WHY_FIELD),
    };
    let handler = Handler {
        command: command.to_owned(),
        command_windows,
        timeout: optional(fields, "timeout", optional_count)?,
        is_async,
        status_message: optional(fields, "statusMessage", optional_text)?,
        context_limit: optional(fields, "additionalContextLimit", optional_count)?,
    };
    if handler.run_command().trim().is_empty() {
        return Err(WHY_FIELD);
    }
    Ok(handler)
}

/// An optional field read by `read` — absent is `None`.
fn optional<T>(
    fields: &Map<String, Value>,
    name: &str,
    read: fn(&Value) -> Result<Option<T>, &'static str>,
) -> Result<Option<T>, &'static str> {
    fields.get(name).map_or(Ok(None), read)
}

fn optional_text(value: &Value) -> Result<Option<String>, &'static str> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text.clone())),
        _ => Err(WHY_FIELD),
    }
}

/// An integer codex reads as `u64` / `usize` that its TOML can hold (at most `i64::MAX`).
fn optional_count(value: &Value) -> Result<Option<u64>, &'static str> {
    match value {
        Value::Null => Ok(None),
        Value::Number(number) => number
            .as_u64()
            .filter(|count| i64::try_from(*count).is_ok())
            .map(Some)
            .ok_or(WHY_FIELD),
        _ => Err(WHY_FIELD),
    }
}

impl Handler {
    /// The command codex runs on this host: `commandWindows` on Windows when set, else `command`.
    fn run_command(&self) -> &str {
        match (&self.command_windows, windows_paths()) {
            (Some(windows), true) => windows,
            _ => &self.command,
        }
    }

    /// The handler as written: the fields codex reads, as given.
    fn written(&self) -> Value {
        let mut fields = Map::new();
        fields.insert("type".to_owned(), json!("command"));
        fields.insert("command".to_owned(), json!(self.command));
        let optional = [
            (
                "commandWindows",
                self.command_windows.as_ref().map(|v| json!(v)),
            ),
            ("timeout", self.timeout.map(|v| json!(v))),
            ("async", self.is_async.map(|v| json!(v))),
            (
                "statusMessage",
                self.status_message.as_ref().map(|v| json!(v)),
            ),
            (
                "additionalContextLimit",
                self.context_limit.map(|v| json!(v)),
            ),
        ];
        for (name, value) in optional {
            if let Some(value) = value {
                fields.insert(name.to_owned(), value);
            }
        }
        Value::Object(fields)
    }
}

/// The key label of one of codex's 14 events.
fn event_label(event: &str) -> Option<&'static str> {
    EVENTS
        .iter()
        .find(|(name, _)| *name == event)
        .map(|(_, label)| *label)
}

/// The trust key — `<session-flags source>:<event label>:<group index>:<handler index>`.
fn trust_key(event: &str, group_index: usize, handler_index: usize) -> String {
    let label = event_label(event).expect("only codex's 14 events are written");
    format!(
        "{}:{label}:{group_index}:{handler_index}",
        session_flags_source()
    )
}

/// codex's normalized identity of one handler, as the compact JSON it hashes: `{event_name, matcher?, hooks:
/// [handler]}` with every object key sorted. The handler: `command` = the command run here (`commandWindows` itself
/// dropped); `timeout` always (SessionEnd / Interrupt: given or 1, clamped to 1..=3; others: given or 600, at least
/// 1); `async` always, as configured (codex runs an async SessionEnd synchronously but hashes the configured value);
/// `statusMessage` if set; `additionalContextLimit` only on the five events that keep it, and only when not 2500;
/// `type`. The matcher is kept as written except on the five events that drop it.
// Behaviour mirrors codex-rs/hooks/src/engine/discovery.rs append_matcher_groups · hook_hash @ rust-v0.171.0 (Apache-2.0).
fn identity_text(event: &str, matcher: Option<&str>, handler: &Handler) -> String {
    let timeout = if SHORT_TIMEOUT_EVENTS.contains(&event) {
        handler.timeout.unwrap_or(1).clamp(1, 3)
    } else {
        handler.timeout.unwrap_or(600).max(1)
    };
    let mut normalized = Map::new();
    normalized.insert("type".to_owned(), json!("command"));
    normalized.insert("command".to_owned(), json!(handler.run_command()));
    normalized.insert("timeout".to_owned(), json!(timeout));
    normalized.insert("async".to_owned(), json!(handler.is_async.unwrap_or(false)));
    if let Some(message) = &handler.status_message {
        normalized.insert("statusMessage".to_owned(), json!(message));
    }
    if let Some(limit) = handler
        .context_limit
        .filter(|limit| CONTEXT_LIMIT_EVENTS.contains(&event) && *limit != DEFAULT_CONTEXT_LIMIT)
    {
        normalized.insert("additionalContextLimit".to_owned(), json!(limit));
    }
    let mut identity = Map::new();
    identity.insert(
        "event_name".to_owned(),
        json!(event_label(event).expect("only codex's 14 events are hashed")),
    );
    if let Some(matcher) = matcher.filter(|_| !MATCHER_DROPPED.contains(&event)) {
        identity.insert("matcher".to_owned(), json!(matcher));
    }
    identity.insert("hooks".to_owned(), json!([normalized]));
    let mut identity = Value::Object(identity);
    identity.sort_all_objects();
    identity.to_string()
}

/// `sha256:` + the lowercase hex sha256 of [`identity_text`].
// Behaviour mirrors codex-rs/config/src/fingerprint.rs version_for_toml @ rust-v0.171.0 (Apache-2.0).
fn trust_hash(event: &str, matcher: Option<&str>, handler: &Handler) -> String {
    format!(
        "sha256:{}",
        sha256_hex(identity_text(event, matcher, handler).as_bytes())
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

impl Group {
    fn written(&self) -> Value {
        let mut group = Map::new();
        if let Some(matcher) = &self.matcher {
            group.insert("matcher".to_owned(), json!(matcher));
        }
        group.insert(
            HOOKS_KEY.to_owned(),
            Value::Array(self.handlers.iter().map(Handler::written).collect()),
        );
        Value::Object(group)
    }
}

/// Engram's own state entry for one handler: `enabled = true` overrides a stale user-layer `enabled = false` on the
/// same session-flags key (codex merges state per key and field).
fn own_state_entry(entry: &TrustEntry) -> Value {
    json!({ "enabled": true, "trusted_hash": entry.hash })
}

/// The JSON thread config `hooks` object: each written event's groups, and `state`.
fn thread_config(built: &Built, expected: &[TrustEntry], carried: &[Carried]) -> Value {
    let mut config = Map::new();
    for (event, groups) in &built.events {
        config.insert(
            (*event).to_owned(),
            Value::Array(groups.iter().map(Group::written).collect()),
        );
    }
    let mut state = Map::new();
    for entry in expected {
        state.insert(entry.key.clone(), own_state_entry(entry));
    }
    for (key, entry, _) in carried {
        state.insert(key.clone(), entry.clone());
    }
    config.insert(STATE_KEY.to_owned(), Value::Object(state));
    Value::Object(config)
}

/// The terminal lines: one `hooks.<Event>=[…]` per written event (sorted), then one `hooks.state={…}` — codex keeps
/// only the last line for a repeated path, so every entry goes in one state line. Every group and every state entry
/// passes the guard on its own, so the whole line does.
// ADR-0353
fn terminal_lines(
    built: &Built,
    expected: &[TrustEntry],
    carried: &[Carried],
) -> Result<Vec<String>, Reason> {
    let mut args = Vec::new();
    for (event, groups) in &built.events {
        let pieces = groups
            .iter()
            .map(|group| guarded(&group.written(), &group.subject, event))
            .collect::<Result<Vec<_>, _>>()?;
        args.push(CONFIG_OVERRIDE_FLAG.to_owned());
        args.push(format!("{HOOKS_KEY}.{event}=[{}]", pieces.join(",")));
    }
    let own = expected
        .iter()
        .map(|entry| (&entry.key, own_state_entry(entry), &entry.subject));
    let raw = carried
        .iter()
        .map(|(key, entry, subject)| (key, entry.clone(), subject));
    let mut entries = Vec::with_capacity(expected.len() + carried.len());
    for (key, entry, subject) in own.chain(raw) {
        let entry = format!(
            "{}={}",
            toml_key(key),
            toml_text(&entry, subject, STATE_KEY)?
        );
        guard(&entry, subject, STATE_KEY)?;
        entries.push(entry);
    }
    args.push(CONFIG_OVERRIDE_FLAG.to_owned());
    args.push(format!("{HOOKS_KEY}.{STATE_KEY}={{{}}}", entries.join(",")));
    Ok(args)
}

/// `value` as TOML that passes the guard; refused as [`CODEX_HOOK_LINE_REFUSED`] otherwise.
fn guarded(value: &Value, subject: &Subject, event: &str) -> Result<String, Reason> {
    let text = toml_text(value, subject, event)?;
    guard(&text, subject, event)?;
    Ok(text)
}

fn toml_text(value: &Value, subject: &Subject, event: &str) -> Result<String, Reason> {
    let mut text = String::new();
    toml_value(value, &mut text).map_err(|()| {
        subject.reason(
            CODEX_HOOK_LINE_REFUSED,
            json!({ "event": event, "why": "value" }),
        )
    })?;
    Ok(text)
}

/// The codex `-c` guard on one piece of a hook line: [`text_is_cli_safe`] and an even `"` count (as
/// `cmdline::config_args`). After the writer's escapes only an odd `"` count trips it from input; `unsafe-char` is
/// the backstop for a writer regression.
fn guard(text: &str, subject: &Subject, event: &str) -> Result<(), Reason> {
    let why = if !text_is_cli_safe(text) {
        "unsafe-char"
    } else if text.matches('"').count() % 2 == 1 {
        "odd-quotes"
    } else {
        return Ok(());
    };
    Err(subject.reason(
        CODEX_HOOK_LINE_REFUSED,
        json!({ "event": event, "why": why }),
    ))
}

/// [`CODEX_RAW_HOOKS_TRUSTED`] `{settings: [{source, digest}]}` — each settings asset with at least one written group,
/// in write order, with the first 16 hex digits of a sha256 over its written groups (so the notice fingerprint changes
/// when the trusted set of an already-named asset grows). `None` when no raw group is written.
// ADR-0420
fn raw_trusted_notice(built: &Built) -> Option<Notice> {
    let mut sources: Vec<(&str, Vec<Value>)> = Vec::new();
    for (event, groups) in &built.events {
        for group in groups {
            let Subject::Source(origin) = &group.subject else {
                continue;
            };
            let written = json!({ "event": event, "group": group.written() });
            match sources
                .iter_mut()
                .find(|(source, _)| *source == origin.source)
            {
                Some((_, groups)) => groups.push(written),
                None => sources.push((&origin.source, vec![written])),
            }
        }
    }
    if sources.is_empty() {
        return None;
    }
    let settings: Vec<Value> = sources
        .into_iter()
        .map(|(source, groups)| {
            let digest = sha256_hex(Value::Array(groups).to_string().as_bytes());
            json!({ "source": source, "digest": &digest[..16] })
        })
        .collect();
    Some(Reason {
        code: CODEX_RAW_HOOKS_TRUSTED.to_owned(),
        args: json!({ "settings": settings }),
        line: None,
    })
}

// ── the TOML writer ──────────────────────────────────────────────────────────────

/// The characters the writer always writes as `\u00XX`, in keys and string values alike: cmd's `% & ^ | < >` and
/// `!` (cmd drops `!…!` under `DelayedExpansion`). ★Escaped, not refused★ — cmd sees the line before codex's TOML
/// parser decodes it, and codex hashes the decoded value as Engram does (Engram hashes before writing), so the
/// escape changes nothing codex reads or trusts; it only keeps the raw character off cmd's line.
// ADR-0420
const CMD_ESCAPED: [char; 7] = ['%', '&', '^', '|', '<', '>', '!'];

/// One value as TOML: every key a quoted basic key ([`toml_key`]) — never bare, so no key can close a table or start
/// a comment; a string as a literal `'…'` unless it holds `'`, a control character or one of [`CMD_ESCAPED`],
/// then a basic string ([`basic_string`]); integers; booleans; arrays; inline tables. `Err` = a `null` or a number
/// that is not an integer TOML holds — what the assembly checked never holds one.
// ADR-0420
fn toml_value(value: &Value, out: &mut String) -> Result<(), ()> {
    match value {
        Value::Null => return Err(()),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => {
            let integer = number.as_i64().ok_or(())?;
            let _ = write!(out, "{integer}");
        }
        Value::String(text)
            if text.contains('\'')
                || text.contains(CMD_ESCAPED)
                || text.chars().any(char::is_control) =>
        {
            basic_string(text, out);
        }
        Value::String(text) => {
            out.push('\'');
            out.push_str(text);
            out.push('\'');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                toml_value(item, out)?;
            }
            out.push(']');
        }
        Value::Object(fields) => {
            out.push('{');
            for (index, (key, field)) in fields.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&toml_key(key));
                out.push('=');
                toml_value(field, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

/// A key as a TOML basic string — so the session-flags source (`<session-flags>`) is written `\u003c…\u003e` and
/// passes the cmd guard (measured through `cmd.exe /c codex`).
fn toml_key(key: &str) -> String {
    let mut out = String::new();
    basic_string(key, &mut out);
    out
}

/// `text` as a TOML basic string: `"` and `\` escaped, a control character never raw (cmd cuts the line at a real
/// LF), and [`CMD_ESCAPED`] as `\u00XX`, which TOML decodes to the character itself.
fn basic_string(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if CMD_ESCAPED.contains(&c) || c.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

// ── pass-through ─────────────────────────────────────────────────────────────────

/// The first pass-through override that sets `hooks` or `hooks.*`, read the way codex's clap reads `-c` / `--config`:
/// the next element after `-c` or `--config`, the rest after `--config=`, or the rest after `-c` with one leading `=`
/// stripped (`-c=k=v`, `-ck=v`); nothing after `--`. The key (before the first `=`) is trimmed and matches when its
/// first `.`-segment is `hooks` (segments untrimmed, as codex splits them).
// ADR-0420
fn passthrough_hooks_arg(extra_args: &[String]) -> Option<&str> {
    let mut args = extra_args.iter();
    while let Some(arg) = args.next() {
        let value = match arg.as_str() {
            "--" => return None,
            "-c" | "--config" => args.next()?.as_str(),
            other => match (other.strip_prefix("--config="), other.strip_prefix("-c")) {
                (Some(value), _) => value,
                (None, Some(rest)) => rest.strip_prefix('=').unwrap_or(rest),
                (None, None) => continue,
            },
        };
        let key = value.split_once('=').map_or(value, |(key, _)| key).trim();
        if key.split('.').next() == Some(HOOKS_KEY) {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::backend::hooks::HookRef;
    use crate::launch::notice_fingerprint;

    const TERMINAL: AgentOutputFormat = AgentOutputFormat::Terminal;
    const JSON: AgentOutputFormat = AgentOutputFormat::StreamJson;

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_owned()).collect()
    }

    fn origin(source: &str, line: u32) -> ConfigOrigin {
        ConfigOrigin {
            source: source.to_owned(),
            line: Some(line),
        }
    }

    /// The raw lines as `command_line_args` gives them — `-c`, line, … — with one origin per argument.
    fn raw(lines: &[(&str, &str, u32)]) -> (Vec<String>, Vec<ConfigOrigin>) {
        let mut args = Vec::new();
        let mut origins = Vec::new();
        for (text, source, line) in lines {
            args.extend(strings(&["-c", text]));
            origins.extend([origin(source, *line), origin(source, *line)]);
        }
        (args, origins)
    }

    fn hook(path: &str, definition: Value) -> HookRef {
        let Value::Object(definition) = definition else {
            panic!("a hook definition is an object");
        };
        HookRef {
            path: path.to_owned(),
            dir: std::path::absolute("/library").unwrap().join(path),
            definition,
        }
    }

    fn assemble(
        mode: AgentOutputFormat,
        extra: &[&str],
        lines: &[(&str, &str, u32)],
        hooks: &[HookRef],
    ) -> Result<HookAssembly, Reason> {
        let extra = strings(extra);
        let (config_args, config_origins) = raw(lines);
        hook_assembly(&CommandLine {
            mode,
            cwd: Path::new("C:/w"),
            extra_args: &extra,
            config_args: &config_args,
            config_origins: &config_origins,
            hooks,
            instructions: None,
        })
    }

    fn command(text: &str) -> Value {
        json!({ "type": "command", "command": text })
    }

    /// The canonical identity and the hash of the first handler of `group` on `event`.
    fn hashed(event: &str, group: Value) -> (String, String) {
        let matcher = group.get("matcher").and_then(Value::as_str);
        let handler = checked_handler(&group["hooks"][0]).expect("a handler Engram trusts");
        (
            identity_text(event, matcher, &handler),
            trust_hash(event, matcher, &handler),
        )
    }

    fn key(label_and_indexes: &str) -> String {
        format!("{}:{label_and_indexes}", session_flags_source())
    }

    /// The trust key as the terminal line writes it.
    fn escaped_key(label_and_indexes: &str) -> String {
        let source = if windows_paths() {
            r"C:\\\u003csession-flags\u003e\\config.toml"
        } else {
            r"/\u003csession-flags\u003e/config.toml"
        };
        format!("\"{source}:{label_and_indexes}\"")
    }

    // ── the hash — every expected value was computed by codex's own structs and normalization copied from
    //    rust-v0.171.0 (protocol/src/hooks.rs, discovery.rs, fingerprint.rs) in a scratch program, never by this
    //    module ──

    const PLAIN: &str = "sha256:f98ab5b7912e947bea33ccdf9ec0cf28fe14642014d247a2de1d0925be019382";
    const BASH_GUARD: &str =
        "sha256:8e8491a965649e3596ac4262ec9baf9ee4cf11446093336d8de6fab2c104635b";

    #[test]
    fn a_plain_command_hashes_with_the_default_timeout_and_async() {
        let (identity, hash) = hashed(
            "SessionStart",
            json!({ "hooks": [command(r#"node "D:/Library/hooks/notify/notify.js" start"#)] }),
        );
        assert_eq!(
            identity,
            r#"{"event_name":"session_start","hooks":[{"async":false,"command":"node \"D:/Library/hooks/notify/notify.js\" start","timeout":600,"type":"command"}]}"#
        );
        assert_eq!(hash, PLAIN);
    }

    #[test]
    fn a_matcher_is_hashed_as_written_with_the_given_timeout() {
        let (identity, hash) = hashed(
            "PreToolUse",
            json!({ "matcher": "Bash", "hooks": [{ "type": "command", "command": r#"node "D:/Library/hooks/notify/guard.js""#, "timeout": 30 }] }),
        );
        assert_eq!(
            identity,
            r#"{"event_name":"pre_tool_use","hooks":[{"async":false,"command":"node \"D:/Library/hooks/notify/guard.js\"","timeout":30,"type":"command"}],"matcher":"Bash"}"#
        );
        assert_eq!(hash, BASH_GUARD);
    }

    /// ★On Windows codex runs and hashes `commandWindows` (also read as `command_windows`); elsewhere `command`★ —
    /// `commandWindows` itself is never hashed.
    #[test]
    fn command_windows_and_its_alias_replace_the_command_on_windows_only() {
        let expected = if windows_paths() {
            "sha256:810ef28ab0ab9feca57a8ae9a964a5032492c266c6955d26162de5f82c8ebde9"
        } else {
            "sha256:99543572dd7f918d983afffb4d1474968d3ee91d188471f2afd91f3ff985ce02"
        };
        for name in ["commandWindows", "command_windows"] {
            let (_, hash) = hashed(
                "SessionStart",
                json!({ "hooks": [{ "type": "command", "command": "echo unix", name: r#"node "D:/Library/hooks/notify/win.js""# }] }),
            );
            assert_eq!(hash, expected, "{name}");
        }
    }

    #[test]
    fn a_zero_timeout_hashes_as_one_second() {
        let (identity, hash) = hashed(
            "Stop",
            json!({ "hooks": [{ "type": "command", "command": "node stop.js", "timeout": 0 }] }),
        );
        assert!(identity.contains(r#""timeout":1,"#), "{identity}");
        assert_eq!(
            hash,
            "sha256:a838b8933f24f310e0efb18514502af31e4dfe1c34ea5d0d93d35bce48020206"
        );
    }

    /// SessionEnd and Interrupt default to 1 second and clamp to 1..=3.
    #[test]
    fn session_end_and_interrupt_timeouts_default_to_one_and_clamp_to_three() {
        for (event, script, timeout, expected) in [
            (
                "SessionEnd",
                "node end.js",
                None,
                "sha256:f5ab51d3717c4f280518640a30fe88d4af3e636a061544a4d67eddee59bdbeea",
            ),
            (
                "SessionEnd",
                "node end.js",
                Some(10),
                "sha256:654bd8bbf6e8f41a55a26dd897d1cfd6be6e303af77db8e0838353df2cdeffca",
            ),
            (
                "Interrupt",
                "node int.js",
                Some(0),
                "sha256:489410e0d0b608f1e0a29eb790ce25d6af1f202c64e16bc8aa7acc0e068ba17c",
            ),
        ] {
            let mut handler = command(script);
            if let Some(timeout) = timeout {
                handler["timeout"] = json!(timeout);
            }
            let (_, hash) = hashed(event, json!({ "hooks": [handler] }));
            assert_eq!(hash, expected, "{event} {timeout:?}");
        }
    }

    /// codex runs an async SessionEnd synchronously but hashes the configured value.
    #[test]
    fn an_async_session_end_is_hashed_as_configured() {
        let (identity, hash) = hashed(
            "SessionEnd",
            json!({ "hooks": [{ "type": "command", "command": "node end.js", "async": true }] }),
        );
        assert!(identity.contains(r#""async":true"#), "{identity}");
        assert_eq!(
            hash,
            "sha256:1a4850e960bcdaabc129ac8a1fe4ff070eaecc76c4bd307001fcfb5b10bd9e20"
        );
    }

    #[test]
    fn the_matcher_is_dropped_for_user_prompt_submit_and_stop() {
        for (event, script, expected) in [
            (
                "UserPromptSubmit",
                "node p.js",
                "sha256:e3fc83308077d41a3e422a7756ab6cd973172d359ffa8eaaefca3db4e71cd00f",
            ),
            (
                "Stop",
                "node stop.js",
                "sha256:8e94d8e683b19bc0ec3c5a85f59b495cca0dac74aebfef13caabe7f4744cb6c0",
            ),
        ] {
            let (identity, hash) =
                hashed(event, json!({ "matcher": "x", "hooks": [command(script)] }));
            assert!(!identity.contains("matcher"), "{identity}");
            assert_eq!(hash, expected, "{event}");
        }
    }

    /// `additionalContextLimit` is hashed only on the five events that keep it, and not at its default 2500.
    #[test]
    fn the_context_limit_is_hashed_per_event_and_not_at_its_default() {
        for (event, script, limit, expected) in [
            (
                "PostToolUse",
                "node post.js",
                100,
                "sha256:eaafb3ab351f1f9b6f484a63f29bbf91334572e78f16928864b71f6c4c765b09",
            ),
            (
                "PostToolUse",
                "node post.js",
                2500,
                "sha256:cd54a2d973dd685f0efef88dd340d0d0b9b4cd9689013977c8022c9d348aead4",
            ),
            (
                "SessionStart",
                "node s.js",
                0,
                "sha256:ab18b404f9cc3e0665d289fe541e3e0f457d209808ec2ba6059887474a89b2ae",
            ),
            // Stop ignores it: the hash of the plain Stop handler.
            (
                "Stop",
                "node stop.js",
                100,
                "sha256:8e94d8e683b19bc0ec3c5a85f59b495cca0dac74aebfef13caabe7f4744cb6c0",
            ),
        ] {
            let mut handler = command(script);
            handler["additionalContextLimit"] = json!(limit);
            let (_, hash) = hashed(event, json!({ "hooks": [handler] }));
            assert_eq!(hash, expected, "{event} {limit}");
        }
    }

    #[test]
    fn a_status_message_is_hashed_when_set() {
        let (_, hash) = hashed(
            "SessionStart",
            json!({ "hooks": [{ "type": "command", "command": "node s.js", "statusMessage": "Notifying", "async": true }] }),
        );
        assert_eq!(
            hash,
            "sha256:5c2c159405f9fd7c64656335c891b1d61707a92b354486fd1c9f198d2ca17e82"
        );
    }

    /// Fields codex does not read (claude's `once`, `shell`, a `null`) are neither hashed nor written.
    #[test]
    fn fields_codex_does_not_read_are_dropped() {
        let group = json!({ "hooks": [{
            "type": "command",
            "command": r#"node "D:/Library/hooks/notify/notify.js" start"#,
            "once": true,
            "shell": "bash",
            "model": null,
        }] });
        assert_eq!(hashed("SessionStart", group.clone()).1, PLAIN);
        let handler = checked_handler(&group["hooks"][0]).unwrap();
        assert_eq!(
            handler.written(),
            json!({ "type": "command", "command": r#"node "D:/Library/hooks/notify/notify.js" start"# })
        );
    }

    /// A `'` and a non-ASCII character hash as they are (serde_json's compact form).
    #[test]
    fn quotes_and_non_ascii_text_hash_as_they_are() {
        let (_, hash) = hashed(
            "PreToolUse",
            json!({ "matcher": "Edit|Write", "hooks": [{ "type": "command", "command": "node \"D:/Bob's/x.js\" é", "timeout": 5 }] }),
        );
        assert_eq!(
            hash,
            "sha256:71eaf173af0edafb4802bf2a75c5f7a3a9c30f42ad06e7d10c80ccfe79b52eff"
        );
    }

    // ── handler checks ──

    #[test]
    fn a_handler_engram_cannot_normalize_is_left_off_with_its_reason() {
        let cases = [
            (json!({ "type": "prompt", "prompt": "x" }), WHY_TYPE),
            (
                json!({ "type": "mcp_tool", "server": "s", "tool": "t" }),
                WHY_TYPE,
            ),
            (json!({ "command": "x" }), WHY_TYPE),
            (json!({ "type": "command" }), WHY_FIELD),
            (json!({ "type": "command", "command": "  " }), WHY_FIELD),
            (json!({ "type": "command", "command": 1 }), WHY_FIELD),
            (
                json!({ "type": "command", "command": "x", "commandWindows": 1 }),
                WHY_FIELD,
            ),
            (
                json!({ "type": "command", "command": "x", "commandWindows": "a", "command_windows": "b" }),
                WHY_FIELD,
            ),
            (
                json!({ "type": "command", "command": "x", "timeout": -1 }),
                WHY_FIELD,
            ),
            (
                json!({ "type": "command", "command": "x", "timeout": 1.5 }),
                WHY_FIELD,
            ),
            (
                json!({ "type": "command", "command": "x", "timeout": "30" }),
                WHY_FIELD,
            ),
            (
                json!({ "type": "command", "command": "x", "timeout": u64::MAX }),
                WHY_FIELD,
            ),
            (
                json!({ "type": "command", "command": "x", "async": "yes" }),
                WHY_FIELD,
            ),
            (
                json!({ "type": "command", "command": "x", "async": null }),
                WHY_FIELD,
            ),
            (
                json!({ "type": "command", "command": "x", "statusMessage": 2 }),
                WHY_FIELD,
            ),
            (
                json!({ "type": "command", "command": "x", "additionalContextLimit": -5 }),
                WHY_FIELD,
            ),
        ];
        for (handler, why) in cases {
            assert_eq!(checked_handler(&handler), Err(why), "{handler}");
        }
        // null = absent in codex's optional fields; i64::MAX is the largest count TOML holds.
        let handler = checked_handler(&json!({
            "type": "command", "command": "x", "timeout": null, "statusMessage": null,
            "commandWindows": null, "additionalContextLimit": i64::MAX,
        }))
        .unwrap();
        assert_eq!(handler.timeout, None);
        assert_eq!(handler.context_limit, Some(i64::MAX as u64));
    }

    // ── the terminal lines ──

    fn notify() -> HookRef {
        hook(
            "team/hooks/notify",
            json!({
                "SessionStart": [{ "hooks": [command(r#"node "D:/Library/hooks/notify/notify.js" start"#)] }],
                "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": r#"node "D:/Library/hooks/notify/guard.js""#, "timeout": 30 }] }],
            }),
        )
    }

    /// ★The terminal lines★ — one line per event in sorted order, every key quoted, strings as literals, then one
    /// state line with the escaped key, `enabled = true` and the hash of each handler.
    // ADR-0420
    #[test]
    fn a_component_becomes_one_line_per_event_and_one_state_line() {
        let assembly = assemble(TERMINAL, &[], &[], &[notify()]).unwrap();
        assert_eq!(
            assembly.hook_args,
            vec![
                "-c".to_owned(),
                r#"hooks.PreToolUse=[{"hooks"=[{"command"='node "D:/Library/hooks/notify/guard.js"',"timeout"=30,"type"='command'}],"matcher"='Bash'}]"#.to_owned(),
                "-c".to_owned(),
                r#"hooks.SessionStart=[{"hooks"=[{"command"='node "D:/Library/hooks/notify/notify.js" start',"type"='command'}]}]"#.to_owned(),
                "-c".to_owned(),
                format!(
                    "hooks.state={{{}={{\"enabled\"=true,\"trusted_hash\"='{BASH_GUARD}'}},{}={{\"enabled\"=true,\"trusted_hash\"='{PLAIN}'}}}}",
                    escaped_key("pre_tool_use:0:0"),
                    escaped_key("session_start:0:0"),
                ),
            ]
        );
        assert_eq!(assembly.hook_config, None);
        assert_eq!(
            assembly.expected,
            vec![
                TrustEntry {
                    key: key("pre_tool_use:0:0"),
                    hash: BASH_GUARD.to_owned(),
                    subject: Subject::Hook("team/hooks/notify".to_owned()),
                },
                TrustEntry {
                    key: key("session_start:0:0"),
                    hash: PLAIN.to_owned(),
                    subject: Subject::Hook("team/hooks/notify".to_owned()),
                },
            ]
        );
        assert_eq!(assembly.notices, vec![]);
        for arg in &assembly.hook_args {
            assert!(
                text_is_cli_safe(arg) && arg.matches('"').count() % 2 == 0,
                "{arg}"
            );
        }
    }

    /// The placeholder is rewritten to the hook folder before anything is written or hashed.
    #[test]
    fn the_placeholder_is_rewritten_before_the_line_and_the_hash() {
        let component = hook(
            "team/hooks/notify",
            json!({ "Stop": [{ "hooks": [command(r#"node "${CLAUDE_PLUGIN_ROOT}/stop.js""#)] }] }),
        );
        let folder = engram_dashboard_platform::fs::forward_slash_text(&component.dir);
        let assembly = assemble(JSON, &[], &[], &[component]).unwrap();
        let config = assembly.hook_config.unwrap();
        let written = format!(r#"node "{folder}/stop.js""#);
        assert_eq!(config["Stop"][0]["hooks"][0]["command"], json!(written));
        let handler = checked_handler(&command(&written)).unwrap();
        assert_eq!(
            assembly.expected[0].hash,
            trust_hash("Stop", None, &handler)
        );
    }

    #[test]
    fn a_folder_the_path_rule_refuses_refuses_the_assembly() {
        let mut component = hook(
            "team/hooks/notify",
            json!({ "Stop": [{ "hooks": [command(r#"node "${CLAUDE_PLUGIN_ROOT}/stop.js""#)] }] }),
        );
        component.dir = std::path::absolute("/lib/te`am/notify").unwrap();
        for mode in [TERMINAL, JSON] {
            let refusal = assemble(mode, &[], &[], &[component.clone()]).unwrap_err();
            assert_eq!(refusal.code, hooks::HOOK_PATH_UNSAFE);
            assert_eq!(refusal.args["hook"], json!("team/hooks/notify"));
        }
    }

    /// Groups of one event: the raw groups first, then the components in plan order; the indexes follow.
    #[test]
    fn raw_groups_come_first_then_components_in_plan_order() {
        let first = hook(
            "a",
            json!({ "Stop": [{ "hooks": [command("node a.js")] }] }),
        );
        let second = hook(
            "b",
            json!({ "Stop": [{ "hooks": [command("node b.js"), command("node b2.js")] }] }),
        );
        let assembly = assemble(
            JSON,
            &[],
            &[(
                "hooks.Stop=[{hooks=[{type='command',command='node raw.js'}]}]",
                "s.yaml",
                4,
            )],
            &[first, second],
        )
        .unwrap();
        let config = assembly.hook_config.unwrap();
        let commands: Vec<&Value> = config["Stop"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|group| group["hooks"].as_array().unwrap())
            .map(|handler| &handler["command"])
            .collect();
        assert_eq!(
            commands,
            [
                &json!("node raw.js"),
                &json!("node a.js"),
                &json!("node b.js"),
                &json!("node b2.js")
            ]
        );
        let keys: Vec<&str> = assembly.expected.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                key("stop:0:0"),
                key("stop:1:0"),
                key("stop:2:0"),
                key("stop:2:1")
            ]
        );
        assert_eq!(
            assembly.expected[0].subject,
            Subject::Source(origin("s.yaml", 4))
        );
        assert_eq!(assembly.expected[1].subject, Subject::Hook("a".to_owned()));
    }

    /// ★Only codex's 14 events are written★ — a claude-only event is left off with `why: event`, writes no line and no
    /// entry.
    #[test]
    fn a_handler_of_another_event_is_left_off_with_no_entry() {
        let component = hook(
            "team/hooks/n",
            json!({
                "Notification": [{ "hooks": [command("node n.js")] }],
                "Stop": [{ "hooks": [command("node s.js")] }],
            }),
        );
        let assembly = assemble(TERMINAL, &[], &[], &[component]).unwrap();
        assert_eq!(
            assembly.notices,
            vec![Reason {
                code: CODEX_HOOKS_LEFT_OFF.to_owned(),
                args: json!({ "hook": "team/hooks/n", "why": "event", "event": "Notification" }),
                line: None,
            }]
        );
        assert_eq!(assembly.expected.len(), 1);
        assert_eq!(assembly.expected[0].key, key("stop:0:0"));
        assert!(!assembly
            .hook_args
            .iter()
            .any(|a| a.contains("Notification")));
    }

    /// A non-string matcher leaves its whole group off; a non-string `commandWindows` leaves only its handler off.
    #[test]
    fn a_bad_matcher_drops_the_group_and_a_bad_field_drops_the_handler() {
        let component = hook(
            "h",
            json!({ "PreToolUse": [
                { "matcher": ["Bash"], "hooks": [command("node a.js")] },
                { "matcher": "Edit", "hooks": [
                    { "type": "command", "command": "node b.js", "commandWindows": 7 },
                    command("node c.js"),
                ] },
            ] }),
        );
        let assembly = assemble(JSON, &[], &[], &[component]).unwrap();
        let whys: Vec<&Value> = assembly.notices.iter().map(|n| &n.args["why"]).collect();
        assert_eq!(whys, [&json!("matcher"), &json!("field")]);
        assert_eq!(
            assembly.hook_config.unwrap()["PreToolUse"],
            json!([{ "matcher": "Edit", "hooks": [command("node c.js")] }])
        );
        assert_eq!(assembly.expected[0].key, key("pre_tool_use:0:0"));
    }

    /// ★A `null` in a claude-only field is not written and blocks nothing★ — the terminal writer never sees it.
    #[test]
    fn a_null_in_a_claude_only_field_is_not_written() {
        let component = hook(
            "h",
            json!({ "Stop": [{ "hooks": [{ "type": "command", "command": "node s.js", "shell": null, "once": null }], "extra": null }] }),
        );
        let assembly = assemble(TERMINAL, &[], &[], &[component]).unwrap();
        assert_eq!(
            assembly.hook_args[1],
            r#"hooks.Stop=[{"hooks"=[{"command"='node s.js',"type"='command'}]}]"#
        );
        assert_eq!(assembly.notices, vec![]);
    }

    /// ★The index rule★ — a group whose only handler is left off is not written, so the next group's key takes index
    /// 0, as codex numbers what it receives.
    #[test]
    fn an_emptied_group_is_not_written_and_does_not_take_an_index() {
        let component = hook(
            "h",
            json!({ "Stop": [
                { "hooks": [{ "type": "mcp_tool", "server": "s", "tool": "t" }] },
                { "hooks": [command("node s.js")] },
            ] }),
        );
        let assembly = assemble(TERMINAL, &[], &[], &[component]).unwrap();
        assert_eq!(assembly.expected.len(), 1);
        assert_eq!(assembly.expected[0].key, key("stop:0:0"));
        assert_eq!(
            assembly.hook_args[1],
            r#"hooks.Stop=[{"hooks"=[{"command"='node s.js',"type"='command'}]}]"#
        );
        assert_eq!(assembly.notices[0].args["why"], json!("type"));
    }

    /// ★Written-result rule★ — a component whose handlers are all left off writes nothing in either mode; its notices
    /// still come.
    #[test]
    fn a_component_whose_handlers_are_all_left_off_writes_nothing() {
        let component = hook(
            "h",
            json!({ "Notification": [{ "hooks": [command("node n.js")] }] }),
        );
        for mode in [TERMINAL, JSON] {
            let assembly = assemble(mode.clone(), &[], &[], &[component.clone()]).unwrap();
            assert_eq!(assembly.hook_args, Vec::<String>::new(), "{mode:?}");
            assert_eq!(assembly.hook_config, None, "{mode:?}");
            assert_eq!(assembly.expected, vec![], "{mode:?}");
            assert_eq!(assembly.notices.len(), 1, "{mode:?}");
        }
    }

    #[test]
    fn a_line_the_guard_refuses_names_its_hook_in_terminal_mode_only() {
        for (text, why) in [(r#"node "s.js"#, "odd-quotes")] {
            let component = hook(
                "team/hooks/x",
                json!({ "Stop": [{ "hooks": [command(text)] }] }),
            );
            assert_eq!(
                assemble(TERMINAL, &[], &[], &[component.clone()]),
                Err(Reason {
                    code: CODEX_HOOK_LINE_REFUSED.to_owned(),
                    args: json!({ "hook": "team/hooks/x", "event": "Stop", "why": why }),
                    line: None,
                }),
                "{text}"
            );
            assert!(assemble(JSON, &[], &[], &[component]).is_ok(), "{text}");
        }
    }

    /// Each group holds one `"` while the line holds two, so the line passes `config_args` and the group does not.
    #[test]
    fn a_raw_group_the_guard_refuses_names_its_settings_asset_and_line() {
        let refusal = assemble(
            TERMINAL,
            &[],
            &[(
                r#"hooks.Stop=[{hooks=[{type='command',command='node "a'}]},{hooks=[{type='command',command='node "b'}]}]"#,
                "team/codex.yaml",
                7,
            )],
            &[],
        )
        .unwrap_err();
        assert_eq!(refusal.code, CODEX_HOOK_LINE_REFUSED);
        assert_eq!(refusal.args["source"], json!("team/codex.yaml"));
        assert_eq!(refusal.args["why"], json!("odd-quotes"));
        assert_eq!(refusal.line, Some(7));
    }

    /// A carried state entry is guarded on its own — the only refusal with `event: "state"`. Each entry holds one
    /// `"` of its own while the line holds an even count.
    #[test]
    fn a_carried_state_entry_the_guard_refuses_names_its_settings_asset_with_event_state() {
        let line =
            r#"hooks.state={"/u/c.toml:stop:0:0"={note='a"'},"/u/d.toml:stop:0:0"={note='b"'}}"#;
        let lines = [(line, "team/codex.yaml", 6)];
        assert_eq!(
            assemble(TERMINAL, &[], &lines, &[]),
            Err(Reason {
                code: CODEX_HOOK_LINE_REFUSED.to_owned(),
                args: json!({ "source": "team/codex.yaml", "event": "state", "why": "odd-quotes" }),
                line: Some(6),
            })
        );
        assert!(assemble(JSON, &[], &lines, &[]).is_ok());
    }

    /// ★cmd's characters in values are escaped, not refused★ — a `|` matcher and commands with `&&`, `|`, `>`, `%…%`
    /// and `!` go on the terminal line as `\u00XX`, the line holds none of them raw, and the hash is the one of the
    /// decoded value (the `Edit|Write` golden of `quotes_and_non_ascii_text_hash_as_they_are`).
    // ADR-0420
    #[test]
    fn cmd_characters_in_values_are_escaped_so_the_terminal_line_carries_them() {
        let component = hook(
            "team/hooks/x",
            json!({
                "PreToolUse": [{ "matcher": "Edit|Write", "hooks": [{ "type": "command", "command": "node \"D:/Bob's/x.js\" é", "timeout": 5 }] }],
                "SessionStart": [{ "hooks": [command(r#"node "%USERPROFILE%/x.js" !arg!"#)] }],
                "Stop": [{ "hooks": [command("cd x && npm test | tee log > out")] }],
            }),
        );
        let assembly = assemble(TERMINAL, &[], &[], &[component]).unwrap();
        assert_eq!(
            assembly.hook_args[..6],
            strings(&[
                "-c",
                r#"hooks.PreToolUse=[{"hooks"=[{"command"="node \"D:/Bob's/x.js\" é","timeout"=5,"type"='command'}],"matcher"="Edit\u007cWrite"}]"#,
                "-c",
                r#"hooks.SessionStart=[{"hooks"=[{"command"="node \"\u0025USERPROFILE\u0025/x.js\" \u0021arg\u0021","type"='command'}]}]"#,
                "-c",
                r#"hooks.Stop=[{"hooks"=[{"command"="cd x \u0026\u0026 npm test \u007c tee log \u003e out","type"='command'}]}]"#,
            ])[..]
        );
        assert_eq!(
            assembly.expected[0].hash,
            "sha256:71eaf173af0edafb4802bf2a75c5f7a3a9c30f42ad06e7d10c80ccfe79b52eff"
        );
        for arg in &assembly.hook_args {
            assert!(!arg.contains(CMD_ESCAPED), "{arg}");
            assert!(
                text_is_cli_safe(arg) && arg.matches('"').count() % 2 == 0,
                "{arg}"
            );
        }
    }

    /// The `command_windows` alias is read and written back under codex's own name, `commandWindows`.
    #[test]
    fn the_command_windows_alias_is_written_back_as_command_windows() {
        let component = hook(
            "h",
            json!({ "Stop": [{ "hooks": [{ "type": "command", "command": "echo unix", "command_windows": "node win.js" }] }] }),
        );
        let json_mode = assemble(JSON, &[], &[], &[component.clone()]).unwrap();
        assert_eq!(
            json_mode.hook_config.unwrap()["Stop"][0]["hooks"][0],
            json!({ "type": "command", "command": "echo unix", "commandWindows": "node win.js" })
        );
        let terminal = assemble(TERMINAL, &[], &[], &[component]).unwrap();
        assert_eq!(
            terminal.hook_args[1],
            r#"hooks.Stop=[{"hooks"=[{"command"='echo unix',"commandWindows"='node win.js',"type"='command'}]}]"#
        );
    }

    /// The `hooks` table codex builds from Engram's terminal lines, read as codex reads them: each `-c` value split
    /// at its first `=`, the key trimmed and split on `.`, the trimmed value parsed as `_x_ = <value>`, a later
    /// path replacing an earlier one (`utils/cli/src/config_override.rs`, `config/src/overrides.rs`).
    fn codex_reads(hook_args: &[String]) -> Value {
        let mut table = Map::new();
        for pair in hook_args.chunks(2) {
            let [flag, value] = pair else {
                panic!("hook args come in pairs: {pair:?}");
            };
            assert_eq!(flag, CONFIG_OVERRIDE_FLAG);
            let (key, text) = value.split_once('=').expect("a -c value holds a =");
            let path: Vec<&str> = key.trim().split('.').collect();
            let [HOOKS_KEY, member] = path[..] else {
                panic!("not a hooks member path: {key}");
            };
            let parsed: toml::Table = toml::from_str(&format!("_x_ = {}", text.trim()))
                .unwrap_or_else(|error| panic!("codex cannot parse {value}: {error}"));
            let member_value = json_of(&parsed["_x_"]).expect("no float or date-time");
            table.insert(member.to_owned(), member_value);
        }
        Value::Object(table)
    }

    /// ★The terminal lines carry exactly the JSON thread config★ — for the same inputs (components, raw groups, a
    /// carried state entry, cmd characters, `'`, `"`, a TAB, `commandWindows`), the table codex builds from the
    /// terminal lines equals JSON mode's `hook_config`, and both modes expect the same trust entries: no writer path
    /// changes what codex reads or hashes.
    // ADR-0420
    #[test]
    fn codex_reads_the_terminal_lines_as_the_json_thread_config() {
        let all = "%PATH% & a ^ b | c < d > e !x!";
        let rich = hook(
            "team/hooks/x",
            json!({
                "PreToolUse": [{ "matcher": "Edit|Write", "hooks": [{
                    "type": "command",
                    "command": format!("node \"D:/Bob's/x.js\" {all}"),
                    "commandWindows": format!("cmd /c {all}"),
                    "timeout": 5,
                    "statusMessage": all,
                }] }],
                "SessionStart": [{ "matcher": "^startup$", "hooks": [command("echo tab\there")] }],
                "Stop": [{ "hooks": [
                    command("cd x && npm test | tee log > out"),
                    { "type": "command", "command": "node s.js", "async": true, "additionalContextLimit": 10 },
                ] }],
            }),
        );
        let raw_lines = [
            (
                r#"hooks.PostToolUse=[{matcher="Edit\u007cWrite",hooks=[{type='command',command='node p.js'}]}]"#,
                "team/codex.yaml",
                3,
            ),
            (
                r#"hooks.state={"/u/100\u0025!/config.toml:stop:0:0"={enabled=true,note="50\u0025 \u0026 more!"}}"#,
                "team/codex.yaml",
                4,
            ),
        ];
        let cases: [(&[(&str, &str, u32)], Vec<HookRef>); 3] = [
            (&[], vec![notify()]),
            (&[], vec![rich.clone()]),
            (&raw_lines, vec![rich, notify()]),
        ];
        for (lines, hooks) in cases {
            let terminal = assemble(TERMINAL, &[], lines, &hooks).unwrap();
            let json_mode = assemble(JSON, &[], lines, &hooks).unwrap();
            assert_eq!(
                Some(codex_reads(&terminal.hook_args)),
                json_mode.hook_config,
                "{:?}",
                terminal.hook_args
            );
            assert_eq!(terminal.expected, json_mode.expected);
        }
    }

    // ── the JSON thread config ──

    #[test]
    fn json_mode_writes_one_hooks_object_with_the_state_and_no_line() {
        let assembly = assemble(JSON, &[], &[], &[notify()]).unwrap();
        assert_eq!(assembly.hook_args, Vec::<String>::new());
        assert_eq!(
            assembly.hook_config,
            Some(json!({
                "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": r#"node "D:/Library/hooks/notify/guard.js""#, "timeout": 30 }] }],
                "SessionStart": [{ "hooks": [command(r#"node "D:/Library/hooks/notify/notify.js" start"#)] }],
                "state": {
                    key("pre_tool_use:0:0"): { "enabled": true, "trusted_hash": BASH_GUARD },
                    key("session_start:0:0"): { "enabled": true, "trusted_hash": PLAIN },
                },
            }))
        );
    }

    /// With no hook component and no raw hooks line the assembly changes nothing — a pass-through hooks line included.
    #[test]
    fn without_hook_inputs_nothing_changes() {
        let lines = [
            ("model='o3'", "s.yaml", 2),
            ("features.x=true", "s.yaml", 3),
        ];
        for mode in [TERMINAL, JSON] {
            let assembly = assemble(mode.clone(), &["-c", "hooks.Stop=[]"], &lines, &[]).unwrap();
            assert_eq!(
                assembly,
                HookAssembly {
                    config_args: raw(&lines).0,
                    ..HookAssembly::default()
                },
                "{mode:?}"
            );
        }
    }

    // ── raw settings hooks lines ──

    const RAW_STOP: &str = "hooks.Stop=[{hooks=[{type='command',command='node raw.js'}]}]";

    /// ★Raw hooks lines are always consumed★ — they leave `config_args` (with their `-c`) whether or not anything is
    /// written; other lines stay in order.
    #[test]
    fn raw_hooks_lines_leave_the_preset_lines_in_both_modes() {
        let lines = [
            ("model='o3'", "a.yaml", 1),
            (RAW_STOP, "a.yaml", 2),
            ("hooks.state={}", "b.yaml", 1),
            ("features.x=true", "b.yaml", 2),
        ];
        for mode in [TERMINAL, JSON] {
            let assembly = assemble(mode.clone(), &[], &lines, &[]).unwrap();
            assert_eq!(
                assembly.config_args,
                strings(&["-c", "model='o3'", "-c", "features.x=true"]),
                "{mode:?}"
            );
        }
    }

    /// ★Raw-only, all filtered★ — a settings asset whose only hook is an `mcp_tool` handler writes nothing, is named in
    /// a left-off notice at its line, and its line is not re-sent.
    #[test]
    fn a_raw_line_whose_handlers_are_all_left_off_writes_nothing_and_is_not_resent() {
        let line = "hooks.Stop=[{hooks=[{type='mcp_tool',server='s',tool='t'}]}]";
        for mode in [TERMINAL, JSON] {
            let assembly =
                assemble(mode.clone(), &[], &[(line, "team/codex.yaml", 9)], &[]).unwrap();
            assert_eq!(assembly.config_args, Vec::<String>::new(), "{mode:?}");
            assert_eq!(assembly.hook_args, Vec::<String>::new(), "{mode:?}");
            assert_eq!(assembly.hook_config, None, "{mode:?}");
            assert_eq!(
                assembly.notices,
                vec![Reason {
                    code: CODEX_HOOKS_LEFT_OFF.to_owned(),
                    args: json!({ "source": "team/codex.yaml", "why": "type", "event": "Stop" }),
                    line: Some(9),
                }],
                "{mode:?}"
            );
        }
    }

    /// ★codex's last-wins first★ — a later `hooks={…}` wipes an earlier `hooks.SessionStart`; a later event line
    /// replaces an earlier one; different events stay.
    #[test]
    fn raw_lines_follow_codex_last_wins_before_the_merge() {
        let start = "hooks.SessionStart=[{hooks=[{type='command',command='node start.js'}]}]";
        let table = "hooks={Stop=[{hooks=[{type='command',command='node table.js'}]}]}";
        let assembly = assemble(
            JSON,
            &[],
            &[(start, "a.yaml", 1), (table, "b.yaml", 1)],
            &[],
        )
        .unwrap();
        let config = assembly.hook_config.unwrap();
        assert_eq!(config.get("SessionStart"), None);
        assert_eq!(
            config["Stop"][0]["hooks"][0]["command"],
            json!("node table.js")
        );

        let later = "hooks.Stop=[{hooks=[{type='command',command='node later.js'}]}]";
        let assembly = assemble(
            JSON,
            &[],
            &[
                (table, "b.yaml", 1),
                (start, "a.yaml", 1),
                (later, "c.yaml", 1),
            ],
            &[],
        )
        .unwrap();
        let config = assembly.hook_config.unwrap();
        assert_eq!(
            config["SessionStart"][0]["hooks"][0]["command"],
            json!("node start.js")
        );
        assert_eq!(
            config["Stop"],
            json!([{ "hooks": [command("node later.js")] }])
        );
    }

    /// ★Cancellation★ — a state entry a later line cancelled (an empty `hooks.state`, or a `hooks` table without
    /// `state`) is not carried.
    #[test]
    fn a_state_entry_a_later_line_cancelled_is_not_carried() {
        let entry = r#"hooks.state={"C:\\Users\\u\\.codex\\config.toml:stop:0:0"={enabled=true}}"#;
        for later in [
            "hooks.state={}",
            "hooks={Stop=[{hooks=[{type='mcp_tool',server='s',tool='t'}]}]}",
        ] {
            for mode in [TERMINAL, JSON] {
                let assembly = assemble(
                    mode.clone(),
                    &[],
                    &[(entry, "a.yaml", 1), (later, "b.yaml", 1)],
                    &[],
                )
                .unwrap();
                assert_eq!(assembly.hook_args, Vec::<String>::new(), "{later} {mode:?}");
                assert_eq!(assembly.hook_config, None, "{later} {mode:?}");
            }
        }
    }

    /// ★A carried entry alone is hook output★ — the state line (or the thread config) is written for it, its key
    /// trimmed.
    #[test]
    fn a_carried_state_entry_alone_is_written() {
        let entry = r#"hooks.state={" C:\\Users\\u\\.codex\\config.toml:stop:0:0 "={enabled=false,note=1}}"#;
        let terminal = assemble(TERMINAL, &[], &[(entry, "a.yaml", 1)], &[]).unwrap();
        assert_eq!(
            terminal.hook_args,
            strings(&[
                "-c",
                r#"hooks.state={"C:\\Users\\u\\.codex\\config.toml:stop:0:0"={"enabled"=false,"note"=1}}"#
            ])
        );
        let json_mode = assemble(JSON, &[], &[(entry, "a.yaml", 1)], &[]).unwrap();
        assert_eq!(
            json_mode.hook_config,
            Some(
                json!({ "state": { r"C:\Users\u\.codex\config.toml:stop:0:0": { "enabled": false, "note": 1 } } })
            )
        );
        assert_eq!(
            terminal.notices,
            vec![],
            "a carried entry is not a trusted raw hook"
        );
    }

    /// ★Only other-source entries are carried★ — a raw entry naming a session-flags key (after trimming) is dropped:
    /// Engram writes every session-flags entry itself.
    #[test]
    fn a_raw_session_flags_state_entry_is_dropped_even_with_a_stray_space() {
        let session = session_flags_source().replace('\\', "\\\\");
        for spelled in [
            format!("{session}:stop:0:0"),
            format!(" {session}:stop:0:0"),
        ] {
            let entry = format!(r#"hooks.state={{"{spelled}"={{trusted_hash='sha256:x'}}}}"#);
            let assembly = assemble(JSON, &[], &[(&entry, "a.yaml", 1)], &[]).unwrap();
            assert_eq!(assembly.hook_config, None, "{spelled}");
            assert_eq!(
                assembly.dropped_state_keys,
                [key("stop:0:0")],
                "{spelled}: the dropped key is handed back trimmed"
            );
        }
        let entry = format!(
            r#"hooks.state={{"{session}:stop:0:0"={{trusted_hash='sha256:x'}},"/u/config.toml:stop:0:0"={{enabled=true}}}}"#
        );
        let assembly = assemble(JSON, &[], &[(&entry, "a.yaml", 1)], &[notify()]).unwrap();
        let state = assembly.hook_config.unwrap()["state"].clone();
        assert_eq!(state.as_object().unwrap().len(), 3, "{state}");
        assert_eq!(state["/u/config.toml:stop:0:0"], json!({ "enabled": true }));
        assert!(state.get(key("stop:0:0")).is_none(), "{state}");
    }

    #[test]
    fn a_raw_hooks_line_engram_cannot_fold_is_unreadable_on_its_settings_asset() {
        for line in [
            "hooks.Stop.0={hooks=[]}",
            "hooks.state.k={enabled=true}",
            "hooks.Stop=not toml",
            "hooks.Stop={hooks=[]}",
            "hooks.Stop=[1]",
            "hooks.Stop=[{hooks={}}]",
            "hooks.Stop=[{hooks=[1]}]",
            "hooks='x'",
            "hooks.state=[]",
            "hooks.state={k=1}",
            r#"hooks.state={"k"={},"k "={}}"#,
            "hooks.Stop=[{hooks=[{type='command',command='x',timeout=1.5}]}]",
            "hooks={Stop=[{hooks=[{type='command',command='x',at=1979-05-27}]}]}",
        ] {
            assert_eq!(
                assemble(JSON, &[], &[(line, "team/codex.yaml", 5)], &[]),
                Err(Reason {
                    code: CODEX_HOOKS_UNREADABLE.to_owned(),
                    args: json!({ "source": "team/codex.yaml", "line": line }),
                    line: Some(5),
                }),
                "{line}"
            );
            assert_eq!(
                hooks_line_readable(3, line),
                Err(Reason {
                    code: CODEX_HOOKS_UNREADABLE.to_owned(),
                    args: json!({ "index": 3, "line": line }),
                    line: None,
                }),
                "{line}"
            );
        }
        // Another event is left off whole, whatever it holds; a line that is not a hooks line is not read.
        for line in [
            "hooks.Notification=1",
            "model='o3'",
            "hooks .Stop=1",
            "features.hooks=false",
        ] {
            assert_eq!(hooks_line_readable(0, line), Ok(()), "{line}");
        }
    }

    #[test]
    fn a_raw_line_without_an_origin_names_no_subject() {
        let line = "hooks.Stop=1";
        let refusal = hook_assembly(&CommandLine {
            mode: JSON,
            cwd: Path::new("C:/w"),
            extra_args: &[],
            config_args: &strings(&["-c", line]),
            config_origins: &[],
            hooks: &[],
            instructions: None,
        })
        .unwrap_err();
        assert_eq!(refusal.args, json!({ "line": line }));
        assert_eq!(refusal.line, None);
    }

    /// ★The raw-trusted notice names each settings asset with written groups and a digest of them★ — a new line in a
    /// named asset changes the digest, so the notice fingerprint changes too (the fingerprint function as it is).
    #[test]
    fn the_raw_trusted_notice_digest_follows_the_trusted_groups() {
        let more = "hooks.SessionStart=[{hooks=[{type='command',command='node more.js'}]}]";
        let one = assemble(JSON, &[], &[(RAW_STOP, "team/codex.yaml", 2)], &[notify()]).unwrap();
        let two = assemble(
            JSON,
            &[],
            &[
                (RAW_STOP, "team/codex.yaml", 2),
                (more, "team/codex.yaml", 3),
            ],
            &[notify()],
        )
        .unwrap();
        let trusted = |assembly: &HookAssembly| {
            assembly
                .notices
                .iter()
                .find(|n| n.code == CODEX_RAW_HOOKS_TRUSTED)
                .cloned()
                .expect("the raw groups are trusted")
        };
        let (first, second) = (trusted(&one), trusted(&two));
        assert_eq!(first.args["settings"].as_array().unwrap().len(), 1);
        assert_eq!(
            first.args["settings"][0]["source"],
            json!("team/codex.yaml")
        );
        assert_eq!(
            first.args["settings"][0]["digest"].as_str().unwrap().len(),
            16
        );
        assert_ne!(
            first.args["settings"][0]["digest"],
            second.args["settings"][0]["digest"]
        );
        assert_ne!(notice_fingerprint(&[first]), notice_fingerprint(&[second]));
        assert!(assemble(JSON, &[], &[], &[notify()])
            .unwrap()
            .notices
            .iter()
            .all(|n| n.code != CODEX_RAW_HOOKS_TRUSTED));
    }

    // ── pass-through ──

    #[test]
    fn a_passthrough_hooks_override_is_refused_in_every_clap_spelling_when_the_launch_has_hook_inputs(
    ) {
        let spellings: [(&[&str], &str); 8] = [
            (&["-c", "hooks.Stop=[]"], "hooks.Stop=[]"),
            (&["--config", "hooks.Stop=[]"], "hooks.Stop=[]"),
            (&["--config=hooks.Stop=[]"], "hooks.Stop=[]"),
            (&["-c=hooks.Stop=[]"], "hooks.Stop=[]"),
            (&["-chooks.Stop=[]"], "hooks.Stop=[]"),
            (&["-c", " hooks.Stop =[]"], " hooks.Stop =[]"),
            (&["-c", "hooks={}"], "hooks={}"),
            (
                &["--search", "-c", "model='o3'", "-c", "hooks.state={}"],
                "hooks.state={}",
            ),
        ];
        let component = [notify()];
        let inputs: [(&[(&str, &str, u32)], &[HookRef]); 2] =
            [(&[], &component), (&[(RAW_STOP, "s.yaml", 1)], &[])];
        for (extra, arg) in spellings {
            for (lines, hooks) in inputs {
                for mode in [TERMINAL, JSON] {
                    assert_eq!(
                        assemble(mode.clone(), extra, lines, hooks),
                        Err(Reason {
                            code: CODEX_PASSTHROUGH_HOOKS.to_owned(),
                            args: json!({
                                "arg": arg,
                                "advice": "move this hooks line into a hook component or a codex settings asset",
                            }),
                            line: None,
                        }),
                        "{extra:?} {mode:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn other_overrides_and_anything_after_the_double_dash_pass() {
        for extra in [
            &["-c", "features.x=1"][..],
            &["--", "-c", "hooks.Stop=[]"],
            &["--cd", "hooks.Stop=[]"],
            &["-c", "hooksx=1"],
            &["-c", "hooks .Stop=1"],
        ] {
            assert!(
                assemble(TERMINAL, extra, &[], &[notify()]).is_ok(),
                "{extra:?}"
            );
        }
    }

    /// ★Decided on the inputs★ — refused even when nothing is written: a component whose handlers are all left off,
    /// and a settings `hooks.state={}` that cancels everything.
    #[test]
    fn a_passthrough_hooks_override_is_refused_when_nothing_is_written() {
        let left_off = hook(
            "h",
            json!({ "Notification": [{ "hooks": [command("x")] }] }),
        );
        let pass = ["-c", "hooks.state={K={enabled=true}}"];
        assert_eq!(
            assemble(TERMINAL, &pass, &[], &[left_off])
                .unwrap_err()
                .code,
            CODEX_PASSTHROUGH_HOOKS
        );
        assert_eq!(
            assemble(TERMINAL, &pass, &[("hooks.state={}", "s.yaml", 1)], &[])
                .unwrap_err()
                .code,
            CODEX_PASSTHROUGH_HOOKS
        );
    }

    // ── the TOML writer ──

    fn written(value: Value) -> Result<String, ()> {
        let mut out = String::new();
        toml_value(&value, &mut out).map(|()| out)
    }

    #[test]
    fn the_writer_quotes_every_key_and_uses_literals_unless_a_quote_a_control_or_a_cmd_char_needs_escapes(
    ) {
        assert_eq!(
            written(
                json!({ "a b": "plain \"text\"", "c": ["it's", "tab\tline\nend\u{7f}"], "n": -3, "t": true })
            ),
            Ok(
                r#"{"a b"='plain "text"',"c"=["it's","tab\tline\nend\u007f"],"n"=-3,"t"=true}"#
                    .to_owned()
            )
        );
        assert_eq!(
            written(json!({ "k\"\\<%&^|>!": "v%&^|<>!" })),
            Ok(
                r#"{"k\"\\\u003c\u0025\u0026\u005e\u007c\u003e\u0021"="v\u0025\u0026\u005e\u007c\u003c\u003e\u0021"}"#
                    .to_owned()
            )
        );
        assert_eq!(written(json!({ "a": null })), Err(()));
        assert_eq!(written(json!(u64::MAX)), Err(()));
        assert_eq!(written(json!(1.5)), Err(()));
        // codex trims only state keys, and the assembly checks those before writing: other keys equal after trimming
        //   are two keys, as in JSON mode.
        assert_eq!(
            written(json!({ "k": 1, " k": 2 })),
            Ok(r#"{" k"=2,"k"=1}"#.to_owned())
        );
    }

    /// What the writer writes, TOML reads back as the same value.
    #[test]
    fn the_writer_output_parses_back_to_the_same_value() {
        let value = json!({
            "hooks": [{ "type": "command", "command": "node \"D:/Bob's/x.js\" é\r\n", "timeout": 30 }],
            "matcher": "Edit|Write",
            "statusMessage": "%PATH% & a ^ b | c < d > e !x!",
            "C:\\<session-flags>\\config.toml:stop:0:0": { "enabled": true },
            "key %&^|<>!": "plain",
        });
        let text = written(value.clone()).unwrap();
        let table: toml::Table = toml::from_str(&format!("_x_ = {text}")).unwrap();
        assert_eq!(json_of(&table["_x_"]), Some(value));
    }

    #[test]
    fn the_session_flags_key_follows_the_host_path_rules() {
        let expected = if windows_paths() {
            r"C:\<session-flags>\config.toml:session_start:0:1"
        } else {
            "/<session-flags>/config.toml:session_start:0:1"
        };
        assert_eq!(trust_key("SessionStart", 0, 1), expected);
        assert_eq!(toml_key(expected), escaped_key("session_start:0:1"));
    }

    /// The expected matrix is written out from codex `rust-v0.171.0` (`hooks/src/lib.rs` `hook_event_key_label`,
    /// `hooks/src/events/common.rs` `matcher_pattern_for_event`), never read from [`EVENTS`]; the matcher rule is
    /// checked on the hashed identity.
    #[test]
    fn every_event_has_its_codex_label_and_matcher_rule() {
        // (event, key label, matcher kept in the hash)
        let expected = [
            ("PreModelRequest", "pre_model_request", false),
            ("PostModelRequest", "post_model_request", false),
            ("PreToolUse", "pre_tool_use", true),
            ("PermissionRequest", "permission_request", true),
            ("PostToolUse", "post_tool_use", true),
            ("PreCompact", "pre_compact", true),
            ("PostCompact", "post_compact", true),
            ("SessionStart", "session_start", true),
            ("SessionEnd", "session_end", true),
            ("UserPromptSubmit", "user_prompt_submit", false),
            ("SubagentStart", "subagent_start", true),
            ("SubagentStop", "subagent_stop", true),
            ("Stop", "stop", false),
            ("Interrupt", "interrupt", false),
        ];
        let mut written: Vec<&str> = EVENTS.iter().map(|(name, _)| *name).collect();
        let mut listed: Vec<&str> = expected.iter().map(|(name, _, _)| *name).collect();
        written.sort_unstable();
        listed.sort_unstable();
        assert_eq!(written, listed);
        let handler = checked_handler(&command("node x.js")).unwrap();
        for (event, label, matcher_kept) in expected {
            assert_eq!(event_label(event), Some(label), "{event}");
            let identity: Value =
                serde_json::from_str(&identity_text(event, Some("m"), &handler)).unwrap();
            assert_eq!(identity["event_name"], json!(label), "{event}");
            assert_eq!(identity.get("matcher").is_some(), matcher_kept, "{event}");
        }
        assert_eq!(event_label("Notification"), None);
        assert_eq!(
            event_label(STATE_KEY),
            None,
            "state is a member, never an event"
        );
        for event in MATCHER_DROPPED
            .iter()
            .chain(&CONTEXT_LIMIT_EVENTS)
            .chain(&SHORT_TIMEOUT_EVENTS)
        {
            assert!(event_label(event).is_some(), "{event}");
        }
    }
}
