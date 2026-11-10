//! The claude implementation of the settings-check port ([`SettingsCheck`]) — [`CLAUDE_SETTINGS_CHECK`].
//!
//! [`merge`] is also what the claude launch translation (`launch.rs`) writes as its one `--settings` file, so the
//! check and the launch see the same merged value.
// ADR-0334
// ADR-0328

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsStr;
use std::path::Path;

use engram_dashboard_platform::shell::console_command;
use serde_json::{Map, Value};

use super::{CLAUDE_PROGRAM, NO_AUTOUPDATE_ENV};
use crate::backend::settings_check::{
    is, is_version, normal_parts, CommandLine, FolderEntry, FolderLoad, FolderLoadKind,
    FolderLoadSet, FolderUnsure, FrontMatterError, FrontMatterParser, InstructionLoad, ModeLoads,
    NativePlaces, NativeSkillRoots, NoticeCodes, SchemaRefetch, SchemaSource, SettingsCheck,
    SkillFormat, SubagentFrontMatter, UnknownKeys, UnsureWhy, VersionProbe,
};
use crate::launch::Reason;
use crate::profile::AgentOutputFormat;

/// The SchemaStore copy the claude settings docs point to. It is maintained by hand and lags behind the CLI —
/// `docs/research/claude-settings-schema-source-2026-10-18.md`.
const SCHEMA_URL: &str = "https://json.schemastore.org/claude-code-settings.json";

/// Characters whose NFKC form contains `:` (Unicode 16.0) — claude refuses a subagent `name` that contains `:` after
/// NFKC normalization (claude 2.1.284 binary).
const NFKC_COLONS: [char; 5] = [':', '\u{2A74}', '\u{FE13}', '\u{FE55}', '\u{FF1A}'];

pub(crate) static CLAUDE_SETTINGS_CHECK: ClaudeSettingsCheck = ClaudeSettingsCheck;

pub(crate) struct ClaudeSettingsCheck;

impl SettingsCheck for ClaudeSettingsCheck {
    fn backend(&self) -> &'static str {
        "claude"
    }

    fn schema_source(&self, _cli_version: &str) -> SchemaSource {
        SchemaSource {
            url: SCHEMA_URL.to_owned(),
            refetch: SchemaRefetch::LatestOnCliVersionChange,
        }
    }

    /// Carries [`NO_AUTOUPDATE_ENV`] for the same reason as the agent spawn and the usage probe.
    fn version_probe(&self) -> VersionProbe {
        let (program, args) = console_command(CLAUDE_PROGRAM, vec!["--version".to_owned()]);
        let (key, value) = NO_AUTOUPDATE_ENV;
        VersionProbe {
            program,
            args,
            env: vec![(key.to_owned(), value.to_owned())],
        }
    }

    /// `claude --version` prints `2.1.284 (Claude Code)` (2.1.284). The first line whose first word is a dotted
    /// version wins.
    fn parse_version(&self, stdout: &str) -> Option<String> {
        stdout
            .lines()
            .filter_map(|line| line.split_whitespace().next())
            .find(|word| is_version(word))
            .map(str::to_owned)
    }

    /// The raw claude value is the settings JSON as it is — a non-object is returned unchanged too.
    fn raw_to_json(&self, raw: &Value) -> Result<Value, Reason> {
        Ok(raw.clone())
    }

    /// claude loads one merged `--settings`, so there is always a merged value to check.
    fn merge(&self, layers: &[&Value]) -> Option<Value> {
        Some(merge(layers))
    }

    fn merge_origins(&self, layers: &[&Value]) -> Option<Vec<(String, usize)>> {
        Some(merge_origins(layers))
    }

    /// The SchemaStore copy misses many keys the CLI knows, so an unknown key is no evidence of a mistake.
    fn unknown_keys(&self) -> UnknownKeys {
        UnknownKeys::Ignore
    }

    fn loads(&self, _mode: AgentOutputFormat) -> ModeLoads {
        ModeLoads {
            instructions: InstructionLoad::All,
            skills: true,
        }
    }

    /// claude derives a skill's name from its folder itself, so no name is an error here (M24-3).
    fn accepts_skill_name(&self, _name: &str, _mode: AgentOutputFormat) -> bool {
        true
    }

    /// Each folder entry goes on the claude command line as `--add-dir <folder>`, and claude runs through
    /// `cmd.exe /c` on Windows ([`console_command`]).
    // ADR-0341
    fn folder_entry_blocks(&self, folder: &Path) -> bool {
        !engram_dashboard_platform::shell::path_is_cli_safe(folder)
    }

    /// claude takes its working folder from the spawn, never from its command line.
    fn working_folder_blocks(&self, _cwd: &Path, _mode: AgentOutputFormat) -> Option<Reason> {
        None
    }

    /// claude loads every material in both modes and reads its folder entries itself — nothing to tell.
    fn notice_codes(&self, _mode: AgentOutputFormat) -> NoticeCodes {
        NoticeCodes::default()
    }

    /// claude gets each folder entry as `--add-dir` and reads it itself.
    // ADR-0341
    fn folder_listing_at_launch(&self, _mode: AgentOutputFormat) -> bool {
        false
    }

    /// A prediction of what claude loads from an `--add-dir` folder with
    /// `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1` under its default loading conditions — modelled on claude
    /// 2.1.284 (`docs/research/preset-folder-entry-backend-loading-2026-11-04.md` and its binary), not exact for
    /// other versions: `CLAUDE.md` · `.claude/CLAUDE.md` · `CLAUDE.local.md` · `.claude/rules/**/*.md` without
    /// `paths:` · `.claude/skills/*/SKILL.md` without `paths:` · `.claude/agents/**/*.md` with a valid `name` and
    /// `description` · `.claude/commands/**/*.md`.
    ///
    /// ★Only clear cases are predicted★: a file whose answer depends on claude's finer front-matter or content rules
    /// is put in `unsure` (see [`UnsureWhy`]) instead of copying those rules one by one. The same set is predicted
    /// for both modes; whether trust prompts for an untrusted added folder change what a terminal session loads is
    /// not measured.
    ///
    /// Not modelled at all (a file they affect is still predicted by the rules above): claude's size caps (the
    /// memory and rule size cap · agent and command files over 1 MiB skipped) and skill folder names claude
    /// reserves for itself (e.g. sync-reserved names).
    ///
    /// Names claude opens by a joined path (the fixed folder and file names) compare by the OS path rule
    /// ([`engram_dashboard_platform::fs::same_path`]); the `.md` suffix of walked files compares exactly, as
    /// claude's walker does.
    // ADR-0341
    fn folder_load_set(
        &self,
        entries: &[FolderEntry],
        read: &dyn Fn(&Path) -> std::io::Result<String>,
        parser: &dyn FrontMatterParser,
        _mode: AgentOutputFormat,
    ) -> FolderLoadSet {
        let skill_files_per_command_folder = skill_files_per_command_folder(entries);
        let mut set = FolderLoadSet::default();
        for entry in entries {
            let file = match entry {
                FolderEntry::File(file) => file,
                FolderEntry::UnenteredLink(link) => {
                    if let Some(kind) = link_kind(link) {
                        set.unsure.push(FolderUnsure {
                            path: link.clone(),
                            kind,
                            why: UnsureWhy::Link,
                        });
                    }
                    continue;
                }
            };
            let Some(kind) = native_kind(file) else {
                continue;
            };
            let decided = if kind == FolderLoadKind::Command {
                decide_command(file, &skill_files_per_command_folder)
            } else {
                decide(kind, file, read, parser)
            };
            match decided {
                Ok(true) => set.loads.push(FolderLoad {
                    path: file.clone(),
                    kind,
                }),
                Ok(false) => {}
                Err(why) => set.unsure.push(FolderUnsure {
                    path: file.clone(),
                    kind: Some(kind),
                    why,
                }),
            }
        }
        set
    }

    /// claude loads a library skill from its placed folder without a format check of ours.
    fn skill_format(
        &self,
        _file_name: &str,
        _folder_name: &str,
        _text: &str,
        _parser: &dyn FrontMatterParser,
        _mode: AgentOutputFormat,
    ) -> SkillFormat {
        SkillFormat::NotChecked
    }

    /// The raw claude value goes into the merged `--settings` file, never onto the command line.
    fn command_line_args(&self, _raw: &Value) -> Result<Vec<String>, Reason> {
        Ok(vec![])
    }

    /// No claude command-line budget is claimed.
    fn command_line_fits(&self, _line: &CommandLine<'_>) -> Result<(), Reason> {
        Ok(())
    }

    /// No same-name notice for claude.
    fn native_skill_roots(
        &self,
        _places: &NativePlaces<'_>,
        _mode: AgentOutputFormat,
    ) -> NativeSkillRoots {
        NativeSkillRoots::default()
    }

    fn root_skills(
        &self,
        _entries: &[FolderEntry],
        _read: &dyn Fn(&Path) -> std::io::Result<String>,
        _parser: &dyn FrontMatterParser,
    ) -> FolderLoadSet {
        FolderLoadSet::default()
    }
}

/// Whether claude loads one native file of `kind` other than a command — `Err` = not predicted (see [`UnsureWhy`]).
fn decide(
    kind: FolderLoadKind,
    file: &Path,
    read: &dyn Fn(&Path) -> std::io::Result<String>,
    parser: &dyn FrontMatterParser,
) -> Result<bool, UnsureWhy> {
    let text = read(file).map_err(|e| UnsureWhy::Unreadable(e.to_string()))?;
    if kind == FolderLoadKind::Instructions {
        // claude skips a memory file with no content.
        return if may_be_blank(&text) {
            Err(UnsureWhy::EmptyBody)
        } else {
            Ok(true)
        };
    }
    let (block, body) = split_front_matter(&text)?;
    match kind {
        FolderLoadKind::Rule => {
            // claude's docs say it ignores unparseable rule front matter and loads the rule; ours is a different
            // YAML parser, so a failure here does not prove claude fails too — the rule stays unsure.
            let paths = match block {
                Some(block) => parser.paths(block).map_err(in_file)?.paths,
                None => None,
            };
            // A conditional rule loads only when claude touches a matching file under its cwd — never from an
            // added folder.
            if rule_is_conditional(paths.as_ref())? {
                return Ok(false);
            }
            // claude skips a rule with no content left after it strips HTML comments.
            if may_be_blank(body) {
                return Err(UnsureWhy::EmptyBody);
            }
            Ok(true)
        }
        FolderLoadKind::Skill => {
            let paths = match block {
                Some(block) => parser.paths(block).map_err(in_file)?.paths,
                None => None,
            };
            // claude holds a skill with `paths` back as conditional at start.
            match paths {
                Some(_) => Err(UnsureWhy::UnclearPaths),
                None => Ok(true),
            }
        }
        _ => {
            let subagent = match block {
                Some(block) => parser.subagent(block).map_err(in_file)?,
                None => SubagentFrontMatter::default(),
            };
            if !subagent_is_valid(&subagent) {
                return Ok(false);
            }
            if has_hook_keys(&subagent) {
                return Err(UnsureWhy::HookKeys);
            }
            Ok(true)
        }
    }
}

/// Whether claude loads one command file. claude 2.1.284 keeps, per folder, only the file named `skill.md` (ASCII
/// case-insensitive) when the folder holds one and drops its other files (`I1o` in its binary); such folders are
/// kept to the clear case: the single skill file loads, its siblings and several skill files are unsure.
fn decide_command(
    file: &Path,
    skill_files_per_command_folder: &HashMap<&Path, usize>,
) -> Result<bool, UnsureWhy> {
    let Some(&skill_files) = file
        .parent()
        .and_then(|folder| skill_files_per_command_folder.get(folder))
    else {
        return Ok(true);
    };
    if skill_files == 1 && is_skill_file_name(file) {
        Ok(true)
    } else {
        Err(UnsureWhy::SkillFileInCommandFolder)
    }
}

/// Command folder → how many listed command files in it are named `skill.md` (ASCII case-insensitive).
fn skill_files_per_command_folder(entries: &[FolderEntry]) -> HashMap<&Path, usize> {
    let mut counts = HashMap::new();
    for entry in entries {
        if let FolderEntry::File(file) = entry {
            if native_kind(file) == Some(FolderLoadKind::Command) && is_skill_file_name(file) {
                if let Some(folder) = file.parent() {
                    *counts.entry(folder).or_insert(0) += 1;
                }
            }
        }
    }
    counts
}

fn is_skill_file_name(file: &Path) -> bool {
    file.file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("skill.md"))
}

/// The keys claude reads as subagent hooks.
const HOOK_KEYS: [&str; 3] = ["hooks", "PreToolUse", "PermissionRequest"];

/// Top-level keys whose values are not searched for nested hook keys — their own shapes hold such names
/// legitimately.
const HOOK_SEARCH_SKIPPED: [&str; 4] = ["hooks", "metadata", "mcpServers", "lspServers"];

/// How deep inside another top-level key's value a hook key still counts (each object or list is one level).
const HOOK_SEARCH_DEPTH: u32 = 3;

/// A top-level hook key, or a hook key nested within [`HOOK_SEARCH_DEPTH`] levels of any other top-level key except
/// [`HOOK_SEARCH_SKIPPED`] — claude may refuse such a subagent.
fn has_hook_keys(front_matter: &SubagentFrontMatter) -> bool {
    front_matter.has_hooks
        || front_matter.has_pre_tool_use
        || front_matter.has_permission_request
        || front_matter
            .other
            .iter()
            .filter(|(key, _)| !HOOK_SEARCH_SKIPPED.contains(&key.as_str()))
            .any(|(_, value)| holds_hook_key(value, 1))
}

fn holds_hook_key(value: &Value, depth: u32) -> bool {
    if depth > HOOK_SEARCH_DEPTH {
        return false;
    }
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            HOOK_KEYS.contains(&key.as_str()) || holds_hook_key(value, depth + 1)
        }),
        Value::Array(items) => items.iter().any(|item| holds_hook_key(item, depth + 1)),
        _ => false,
    }
}

/// Whether `text` is blank, or may be blank once claude strips `<!--…-->` comments (an unclosed `<!--` counts as
/// may-be-blank).
fn may_be_blank(text: &str) -> bool {
    let mut rest = text;
    let mut kept = String::new();
    while let Some(open) = rest.find("<!--") {
        kept.push_str(&rest[..open]);
        let Some(close) = rest[open + 4..].find("-->") else {
            return true;
        };
        rest = &rest[open + 4 + close + 3..];
    }
    kept.push_str(rest);
    kept.trim().is_empty()
}

/// A parser error with its line moved from the front-matter text to the file — the text starts on file line 2,
/// right after the opening `---`.
fn in_file(mut error: FrontMatterError) -> UnsureWhy {
    error.line = error.line.map(|line| line.saturating_add(1));
    UnsureWhy::FrontMatter(error)
}

/// `Ok(false)` = no `paths` (or `null`) — the rule loads. `Ok(true)` = a plain list of path globs with at least one
/// that is not a match-everything `**` — the rule is conditional. `Err` = any other value.
///
/// claude 2.1.284 itself flattens the value, splits strings at top-level commas, expands braces, strips a trailing
/// `/**` and treats an empty or all-`**` result as unconditional; that is not modelled here, so a value that needs
/// it (`""`, `[]`, `"**"`, a number, a comma or brace string, an item with surrounding whitespace, a mixed list …)
/// is unsure.
fn rule_is_conditional(paths: Option<&Value>) -> Result<bool, UnsureWhy> {
    let items: Vec<&Value> = match paths {
        None => return Ok(false),
        Some(Value::Array(items)) => items.iter().collect(),
        Some(item) => vec![item],
    };
    let mut globs = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Value::String(glob)
                if !glob.trim().is_empty()
                    && glob.trim() == glob
                    && !glob.contains([',', '{', '}']) =>
            {
                globs.push(glob.as_str())
            }
            _ => return Err(UnsureWhy::UnclearPaths),
        }
    }
    let narrows = |glob: &&str| {
        let stem = glob.strip_suffix("/**").unwrap_or(glob);
        !stem.is_empty() && stem != "**"
    };
    if globs.iter().any(narrows) {
        Ok(true)
    } else {
        Err(UnsureWhy::UnclearPaths)
    }
}

/// claude 2.1.284 drops a subagent file unless `name` and `description` are non-empty strings, and drops a `name`
/// that starts with `-` or contains `:` after NFKC normalization (its agent parser, read from the binary).
fn subagent_is_valid(front_matter: &SubagentFrontMatter) -> bool {
    let text = |value: &Option<Value>| match value {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    };
    let (Some(name), Some(_)) = (text(&front_matter.name), text(&front_matter.description)) else {
        return false;
    };
    !name.starts_with('-') && !name.contains(NFKC_COLONS)
}

/// The claude settings merge, in attach order: objects merge key by key recursively · two lists in the same place (at
/// the same key, or two whole layers) are concatenated · any other pair, a type mismatch included, takes the later
/// value. Every list in the result (reached through objects) holds each value once — the first occurrence stays,
/// repeats inside one layer included. No key is treated specially, so a later list cannot remove an item a former
/// layer put into a list; a later value of another type (`null`, an object, a string …) does replace the list — the
/// schema check of the merged value is the net for that. No layer = an empty object.
// ADR-0328
pub(super) fn merge(layers: &[&Value]) -> Value {
    layers
        .iter()
        .fold(Value::Object(Map::new()), |merged, layer| {
            merge_two(merged, layer)
        })
}

// Mirrored by `merge_two_traced` — change both.
fn merge_two(former: Value, later: &Value) -> Value {
    match (former, later) {
        (Value::Object(mut former), Value::Object(later)) => {
            for (key, value) in later {
                match former.get_mut(key) {
                    Some(slot) => *slot = merge_two(std::mem::take(slot), value),
                    None => {
                        former.insert(key.clone(), without_repeats(value));
                    }
                }
            }
            Value::Object(former)
        }
        (Value::Array(former), Value::Array(later)) => {
            let mut merged: Vec<Value> = Vec::with_capacity(former.len() + later.len());
            for item in former.into_iter().chain(later.iter().cloned()) {
                if !merged.contains(&item) {
                    merged.push(item);
                }
            }
            Value::Array(merged)
        }
        (_, later) => without_repeats(later),
    }
}

/// `value` with every list reached through objects holding each value once (first occurrence kept). List items are
/// compared as whole values and not entered.
fn without_repeats(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), without_repeats(value)))
                .collect(),
        ),
        Value::Array(items) => {
            let mut kept: Vec<Value> = Vec::with_capacity(items.len());
            for item in items {
                if !kept.contains(item) {
                    kept.push(item.clone());
                }
            }
            Value::Array(kept)
        }
        other => other.clone(),
    }
}

/// The origins of [`merge`]'s value ([`SettingsCheck::merge_origins`]): the same fold and the same rule, with the index
/// of the layer each part came from. A test runs both on the same tables.
// ADR-0328
pub(super) fn merge_origins(layers: &[&Value]) -> Vec<(String, usize)> {
    let start = Traced::Object {
        layer: None,
        entries: BTreeMap::new(),
    };
    let merged = layers
        .iter()
        .enumerate()
        .fold(start, |merged, (index, layer)| {
            merge_two_traced(merged, layer, index)
        });
    let mut origins = Vec::new();
    collect_origins(&merged, "", &mut origins);
    origins
}

/// A merged value reduced to where its parts came from.
enum Traced {
    Scalar(usize),
    /// `layer` = the last layer that put an object here — `None` only for the empty start of the fold.
    Object {
        layer: Option<usize>,
        entries: BTreeMap<String, Traced>,
    },
    /// `layer` = the last layer that put a list here; each item with the layer that contributed it.
    List {
        layer: usize,
        items: Vec<(Value, usize)>,
    },
}

/// [`merge_two`] with origins.
fn merge_two_traced(former: Traced, later: &Value, layer: usize) -> Traced {
    match (former, later) {
        (Traced::Object { mut entries, .. }, Value::Object(later)) => {
            for (key, value) in later {
                let merged = match entries.remove(key) {
                    Some(slot) => merge_two_traced(slot, value, layer),
                    None => traced(value, layer),
                };
                entries.insert(key.clone(), merged);
            }
            Traced::Object {
                layer: Some(layer),
                entries,
            }
        }
        (Traced::List { items: former, .. }, Value::Array(later)) => {
            let mut items: Vec<(Value, usize)> = Vec::with_capacity(former.len() + later.len());
            for (item, origin) in former
                .into_iter()
                .chain(later.iter().map(|item| (item.clone(), layer)))
            {
                if !items.iter().any(|(kept, _)| *kept == item) {
                    items.push((item, origin));
                }
            }
            Traced::List { layer, items }
        }
        (_, later) => traced(later, layer),
    }
}

/// [`without_repeats`] with origins — every part from `layer`.
fn traced(value: &Value, layer: usize) -> Traced {
    match value {
        Value::Object(map) => Traced::Object {
            layer: Some(layer),
            entries: map
                .iter()
                .map(|(key, value)| (key.clone(), traced(value, layer)))
                .collect(),
        },
        Value::Array(items) => {
            let mut kept: Vec<(Value, usize)> = Vec::with_capacity(items.len());
            for item in items {
                if !kept.iter().any(|(other, _)| other == item) {
                    kept.push((item.clone(), layer));
                }
            }
            Traced::List { layer, items: kept }
        }
        _ => Traced::Scalar(layer),
    }
}

fn collect_origins(node: &Traced, pointer: &str, out: &mut Vec<(String, usize)>) {
    match node {
        Traced::Scalar(layer) => out.push((pointer.to_owned(), *layer)),
        Traced::Object { layer, entries } if entries.is_empty() => {
            out.extend(layer.map(|layer| (pointer.to_owned(), layer)));
        }
        Traced::Object { entries, .. } => {
            for (key, child) in entries {
                // RFC 6901: `~` and `/` are escaped in a pointer token.
                let token = key.replace('~', "~0").replace('/', "~1");
                collect_origins(child, &format!("{pointer}/{token}"), out);
            }
        }
        Traced::List { layer, items } if items.is_empty() => {
            out.push((pointer.to_owned(), *layer));
        }
        Traced::List { items, .. } => {
            for (index, (_, layer)) in items.iter().enumerate() {
                out.push((format!("{pointer}/{index}"), *layer));
            }
        }
    }
}

/// claude's walkers keep a file when its name ends with `.md` — a plain, case-sensitive string test.
fn is_md(part: &OsStr) -> bool {
    part.as_encoded_bytes().ends_with(b".md")
}

/// Which native item a listed file is — `None` = claude does not load it from an added folder (a nested
/// `CLAUDE.md`, `AGENTS.md`, `.claude/settings.json` …).
fn native_kind(file: &Path) -> Option<FolderLoadKind> {
    let parts = normal_parts(file)?;
    match parts.as_slice() {
        [name] if is(name, "CLAUDE.md") || is(name, "CLAUDE.local.md") => {
            Some(FolderLoadKind::Instructions)
        }
        [dir, name] if is(dir, ".claude") && is(name, "CLAUDE.md") => {
            Some(FolderLoadKind::Instructions)
        }
        [dir, sub, rest @ .., name] if is(dir, ".claude") => {
            // A skill's `SKILL.md` is opened by a joined path; the other folders are walked for `.md` files.
            if is(sub, "skills") {
                (rest.len() == 1 && is(name, "SKILL.md")).then_some(FolderLoadKind::Skill)
            } else if !is_md(name) {
                None
            } else if is(sub, "rules") {
                Some(FolderLoadKind::Rule)
            } else if is(sub, "agents") {
                Some(FolderLoadKind::Agent)
            } else if is(sub, "commands") {
                Some(FolderLoadKind::Command)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// What an unfollowed link could hold that claude loads — `None` = nothing claude loads; `Some(None)` = items of
/// several kinds (the `.claude` folder itself).
fn link_kind(link: &Path) -> Option<Option<FolderLoadKind>> {
    let parts = normal_parts(link)?;
    let kind = match parts.as_slice() {
        [name] if is(name, ".claude") => return Some(None),
        [name] if is(name, "CLAUDE.md") || is(name, "CLAUDE.local.md") => {
            FolderLoadKind::Instructions
        }
        [dir, name] if is(dir, ".claude") && is(name, "CLAUDE.md") => FolderLoadKind::Instructions,
        [dir, sub, rest @ ..] if is(dir, ".claude") => {
            if is(sub, "rules") {
                FolderLoadKind::Rule
            } else if is(sub, "agents") {
                FolderLoadKind::Agent
            } else if is(sub, "commands") {
                FolderLoadKind::Command
            } else if is(sub, "skills")
                && match rest {
                    [] | [_] => true,
                    [_, name] => is(name, "SKILL.md"),
                    _ => false,
                }
            {
                FolderLoadKind::Skill
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some(Some(kind))
}

/// A file's front matter and body, after a leading UTF-8 BOM: the front matter = the text between a leading `---`
/// line and the next `---` line, the body = what follows that closing line. No front matter = `None` and the whole
/// text is the body; a front matter of only whitespace = `None`.
///
/// `Err(DashesInFrontMatter)` = an opening `---` line whose front matter holds `---` before a whole closing line,
/// or that never closes while `---` follows somewhere — claude closes its front matter at the first `---`, even in
/// the middle of a line.
fn split_front_matter(text: &str) -> Result<(Option<&str>, &str), UnsureWhy> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let is_fence = |line: &str| line.trim_end() == "---";
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next().filter(|first| is_fence(first)) else {
        return Ok((None, text));
    };
    let start = first.len();
    let mut end = start;
    for line in lines {
        if is_fence(line) {
            let block = &text[start..end];
            if block.contains("---") {
                return Err(UnsureWhy::DashesInFrontMatter);
            }
            let body = &text[end + line.len()..];
            return Ok(((!block.trim().is_empty()).then_some(block), body));
        }
        end += line.len();
    }
    if text[start..].contains("---") {
        return Err(UnsureWhy::DashesInFrontMatter);
    }
    Ok((None, text))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::io;
    use std::path::PathBuf;

    use serde_json::json;

    use engram_dashboard_platform::fs::same_path;

    use super::*;
    use crate::backend::settings_check::{PathsFrontMatter, SkillFrontMatter};

    const MODES: [AgentOutputFormat; 2] =
        [AgentOutputFormat::Terminal, AgentOutputFormat::StreamJson];

    fn merged(layers: &[Value]) -> Value {
        let refs: Vec<&Value> = layers.iter().collect();
        CLAUDE_SETTINGS_CHECK
            .merge(&refs)
            .expect("claude always has a merged value")
    }

    /// The merge tables — `(name, layers, merged value)`.
    fn merge_cases() -> Vec<(&'static str, Vec<Value>, Value)> {
        vec![
            ("no layer", vec![], json!({})),
            (
                "one layer keeps its non-list values as they are",
                vec![json!({"model": "opus", "env": {"A": "1"}, "n": null})],
                json!({"model": "opus", "env": {"A": "1"}, "n": null}),
            ),
            (
                "a repeat inside one layer's list is dropped — first occurrence kept",
                vec![json!({"l": [1, 2, 1], "env": {"deep": ["x", "x"]}})],
                json!({"l": [1, 2], "env": {"deep": ["x"]}}),
            ),
            (
                "a list in a later layer that meets nothing is deduplicated too",
                vec![json!({"a": 1}), json!({"l": ["b", "b"]})],
                json!({"a": 1, "l": ["b"]}),
            ),
            (
                "a list that replaces a mismatched type is deduplicated too",
                vec![json!({"k": "flat"}), json!({"k": ["b", "b"]})],
                json!({"k": ["b"]}),
            ),
            (
                "list items are compared whole, not entered",
                vec![json!({"l": [{"x": [1, 1]}, {"x": [1, 1]}]})],
                json!({"l": [{"x": [1, 1]}]}),
            ),
            (
                "a later scalar wins",
                vec![json!({"model": "opus"}), json!({"model": "sonnet"})],
                json!({"model": "sonnet"}),
            ),
            (
                "a later null wins",
                vec![json!({"model": "opus"}), json!({"model": null})],
                json!({"model": null}),
            ),
            (
                "a later null at a list key replaces the list",
                vec![json!({"l": ["a"]}), json!({"l": null})],
                json!({"l": null}),
            ),
            (
                "objects merge recursively",
                vec![
                    json!({"env": {"A": "1", "B": "1"}, "permissions": {"deny": ["x"], "mode": "a"}}),
                    json!({"env": {"B": "2", "C": "3"}, "permissions": {"mode": "b"}}),
                ],
                json!({"env": {"A": "1", "B": "2", "C": "3"}, "permissions": {"deny": ["x"], "mode": "b"}}),
            ),
            (
                "lists concatenate in attach order",
                vec![
                    json!({"l": ["a", "b"]}),
                    json!({"l": ["c"]}),
                    json!({"l": ["d"]}),
                ],
                json!({"l": ["a", "b", "c", "d"]}),
            ),
            (
                "a repeated value stays once — first occurrence, over the whole list",
                vec![json!({"l": ["a", "b", "a"]}), json!({"l": ["c", "b", "c"]})],
                json!({"l": ["a", "b", "c"]}),
            ),
            (
                "repeated objects in a list compare by value",
                vec![
                    json!({"l": [{"x": 1, "y": 2}]}),
                    json!({"l": [{"y": 2, "x": 1}, {"x": 2}]}),
                ],
                json!({"l": [{"x": 1, "y": 2}, {"x": 2}]}),
            ),
            (
                "a type mismatch takes the later value — list then object",
                vec![json!({"k": ["a"]}), json!({"k": {"a": 1}})],
                json!({"k": {"a": 1}}),
            ),
            (
                "a type mismatch takes the later value — object then scalar",
                vec![json!({"k": {"a": 1}}), json!({"k": "flat"})],
                json!({"k": "flat"}),
            ),
            (
                "a non-object layer replaces everything before it",
                vec![json!({"a": 1}), json!(["x"])],
                json!(["x"]),
            ),
            (
                "two whole list layers concatenate",
                vec![json!(["x", "y"]), json!(["y", "z"])],
                json!(["x", "y", "z"]),
            ),
            (
                "an object layer after a non-object one replaces it",
                vec![json!("x"), json!({"a": 1})],
                json!({"a": 1}),
            ),
        ]
    }

    #[test]
    fn merge_follows_the_claude_rule() {
        for (name, layers, expected) in merge_cases() {
            assert_eq!(merged(&layers), expected, "{name}");
        }
    }

    #[test]
    fn a_later_empty_allowed_mcp_servers_does_not_remove_engram_from_the_default_preset() {
        let default_preset = json!({
            "env": {"DISABLE_AUTOUPDATER": "1"},
            "allowedMcpServers": [{"serverName": "engram"}],
        });
        let later = json!({"allowedMcpServers": [], "model": "opus"});
        assert_eq!(
            merged(&[default_preset, later]),
            json!({
                "env": {"DISABLE_AUTOUPDATER": "1"},
                "allowedMcpServers": [{"serverName": "engram"}],
                "model": "opus",
            })
        );
    }

    #[test]
    fn a_later_list_adds_to_the_default_preset_list() {
        let default_preset = json!({"allowedMcpServers": [{"serverName": "engram"}]});
        let later =
            json!({"allowedMcpServers": [{"serverName": "other"}, {"serverName": "engram"}]});
        assert_eq!(
            merged(&[default_preset, later]),
            json!({"allowedMcpServers": [{"serverName": "engram"}, {"serverName": "other"}]})
        );
    }

    /// The pointers of the parts [`SettingsCheck::merge_origins`] names in `value` — read off the value itself: every
    /// scalar, every list item (not entered) and every empty object or list.
    fn parts(value: &Value, pointer: &str, out: &mut Vec<String>) {
        match value {
            Value::Object(map) if !map.is_empty() => {
                for (key, child) in map {
                    let token = key.replace('~', "~0").replace('/', "~1");
                    parts(child, &format!("{pointer}/{token}"), out);
                }
            }
            Value::Array(items) if !items.is_empty() => {
                out.extend((0..items.len()).map(|index| format!("{pointer}/{index}")));
            }
            _ => out.push(pointer.to_owned()),
        }
    }

    /// ★`merge_origins` agrees with `merge` on every merge table★ — every part of the merged value has exactly one
    /// origin, and the layer it names holds that part there (a list item: the layer's list at that place holds it).
    #[test]
    fn merge_origins_agree_with_merge_on_every_merge_table() {
        let mut tables: Vec<(&str, Vec<Value>)> = merge_cases()
            .into_iter()
            .map(|(name, layers, _)| (name, layers))
            .collect();
        tables.extend([
            (
                "the default preset's list survives a later empty list",
                vec![
                    json!({"env": {"DISABLE_AUTOUPDATER": "1"}, "allowedMcpServers": [{"serverName": "engram"}]}),
                    json!({"allowedMcpServers": [], "model": "opus"}),
                ],
            ),
            (
                "a later list adds to the default preset's list",
                vec![
                    json!({"allowedMcpServers": [{"serverName": "engram"}]}),
                    json!({"allowedMcpServers": [{"serverName": "other"}, {"serverName": "engram"}]}),
                ],
            ),
            (
                "empty containers and escaped keys",
                vec![
                    json!({"m": {}, "l": [], "a/b~c": 1}),
                    json!({"m": {}, "n": {}}),
                ],
            ),
        ]);
        for (name, layers) in tables {
            let refs: Vec<&Value> = layers.iter().collect();
            let merged = merged(&layers);
            let origins = CLAUDE_SETTINGS_CHECK
                .merge_origins(&refs)
                .expect("claude always has a merged value");

            let mut named: Vec<&str> = origins.iter().map(|(p, _)| p.as_str()).collect();
            named.sort_unstable();
            let mut expected = Vec::new();
            if !layers.is_empty() {
                parts(&merged, "", &mut expected);
            }
            expected.sort_unstable();
            assert_eq!(named, expected, "{name}");

            for (pointer, layer) in &origins {
                let part = merged.pointer(pointer).expect("a part of the merged value");
                let source = &layers[*layer];
                let list_place = pointer
                    .rfind('/')
                    .map(|cut| &pointer[..cut])
                    .filter(|parent| merged.pointer(parent).is_some_and(Value::is_array));
                match list_place {
                    Some(parent) => assert!(
                        source
                            .pointer(parent)
                            .and_then(Value::as_array)
                            .is_some_and(|items| items.contains(part)),
                        "{name}: {pointer} from layer {layer}"
                    ),
                    None => assert_eq!(
                        source.pointer(pointer),
                        Some(part),
                        "{name}: {pointer} from layer {layer}"
                    ),
                }
            }
        }
    }

    #[test]
    fn merge_origins_name_the_layer_each_part_comes_from() {
        let origins = |layers: &[Value]| {
            let refs: Vec<&Value> = layers.iter().collect();
            let mut origins = CLAUDE_SETTINGS_CHECK.merge_origins(&refs).unwrap();
            origins.sort();
            origins
        };
        let owned = |pairs: &[(&str, usize)]| -> Vec<(String, usize)> {
            pairs.iter().map(|(p, l)| ((*p).to_owned(), *l)).collect()
        };
        assert_eq!(origins(&[]), owned(&[]));
        assert_eq!(
            origins(&[
                json!({"env": {"A": "1", "B": "1"}, "l": ["a", "b"]}),
                json!({"env": {"B": "2"}, "l": ["b", "c"], "m": {}}),
            ]),
            owned(&[
                ("/env/A", 0),
                ("/env/B", 1),
                ("/l/0", 0),
                ("/l/1", 0),
                ("/l/2", 1),
                ("/m", 1),
            ])
        );
        assert_eq!(
            origins(&[json!({"k": ["a"]}), json!({"k": {"x": 1}})]),
            owned(&[("/k/x", 1)])
        );
        assert_eq!(origins(&[json!({"a/b~c": 1})]), owned(&[("/a~1b~0c", 0)]));
        assert_eq!(
            origins(&[json!({"a": 1}), json!(["x"]), json!(["x", "y"])]),
            owned(&[("/0", 1), ("/1", 2)])
        );
        assert_eq!(origins(&[json!("x")]), owned(&[("", 0)]));
        assert_eq!(origins(&[json!({}), json!({})]), owned(&[("", 1)]));
    }

    #[test]
    fn claude_has_no_working_folder_block_no_notice_codes_and_lists_no_folder_at_launch() {
        for mode in MODES {
            for cwd in [r"C:\Work\R&D", r"C:\%A%\b", r"D:\plain"] {
                assert_eq!(
                    CLAUDE_SETTINGS_CHECK.working_folder_blocks(Path::new(cwd), mode),
                    None,
                    "{cwd} {mode:?}"
                );
            }
            assert_eq!(
                CLAUDE_SETTINGS_CHECK.notice_codes(mode),
                NoticeCodes::default(),
                "{mode:?}"
            );
            assert!(
                !CLAUDE_SETTINGS_CHECK.folder_listing_at_launch(mode),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn the_raw_claude_value_is_its_json_as_it_is() {
        for raw in [
            json!({"model": "opus", "unknownKey": {"deep": [1]}}),
            json!({}),
            json!(["not", "an", "object"]),
            json!("text"),
        ] {
            assert_eq!(CLAUDE_SETTINGS_CHECK.raw_to_json(&raw), Ok(raw.clone()));
        }
    }

    #[test]
    fn the_schema_is_the_schemastore_copy_fetched_again_when_the_cli_version_changes() {
        for version in ["2.1.284", "2.1.300", ""] {
            assert_eq!(
                CLAUDE_SETTINGS_CHECK.schema_source(version),
                SchemaSource {
                    url: "https://json.schemastore.org/claude-code-settings.json".to_owned(),
                    refetch: SchemaRefetch::LatestOnCliVersionChange,
                },
                "{version:?}"
            );
        }
    }

    #[test]
    fn the_version_probe_is_claude_version_shaped_for_the_os_without_autoupdate() {
        let probe = CLAUDE_SETTINGS_CHECK.version_probe();
        let (program, args) = console_command("claude", vec!["--version".to_owned()]);
        assert_eq!(
            probe,
            VersionProbe {
                program,
                args,
                env: vec![("DISABLE_AUTOUPDATER".to_owned(), "1".to_owned())],
            }
        );
    }

    #[test]
    fn parse_version_reads_the_dotted_version() {
        let cases = [
            ("2.1.284 (Claude Code)", Some("2.1.284")),
            ("2.1.284 (Claude Code)\r\n", Some("2.1.284")),
            ("  2.1.284\n", Some("2.1.284")),
            ("2.2.0-beta.1 (Claude Code)", Some("2.2.0-beta.1")),
            ("Update available\n2.1.300 (Claude Code)\n", Some("2.1.300")),
            ("", None),
            ("(Claude Code)", None),
            ("v2.1.284", None),
            ("2", None),
            ("2..1", None),
            ("2.x.1", None),
            ("2.1.284;rm", None),
        ];
        for (stdout, expected) in cases {
            assert_eq!(
                CLAUDE_SETTINGS_CHECK.parse_version(stdout).as_deref(),
                expected,
                "{stdout:?}"
            );
        }
    }

    #[test]
    fn unknown_keys_are_not_reported() {
        assert_eq!(CLAUDE_SETTINGS_CHECK.unknown_keys(), UnknownKeys::Ignore);
    }

    #[test]
    fn claude_loads_all_instructions_and_skills_in_both_modes() {
        for mode in MODES {
            assert_eq!(
                CLAUDE_SETTINGS_CHECK.loads(mode),
                ModeLoads {
                    instructions: InstructionLoad::All,
                    skills: true,
                },
                "{mode:?}"
            );
        }
    }

    #[test]
    fn no_skill_name_is_an_error_for_claude() {
        for mode in MODES {
            for name in ["Weird_Name", "a:b", "x".repeat(65).as_str(), "", "ok-name"] {
                assert!(
                    CLAUDE_SETTINGS_CHECK.accepts_skill_name(name, mode),
                    "{name:?} {mode:?}"
                );
            }
        }
    }

    #[test]
    fn a_folder_entry_blocks_claude_when_its_path_breaks_the_cmd_rule() {
        for (path, blocks) in [
            (r"C:\Users\a b\rules", false),
            (r"D:\rules\company", false),
            (r"C:\rules\50%\x", true),
            (r"C:\a&b", true),
            (r"C:\a^b", true),
            (r"C:\a|b", true),
            (r"C:\a<b", true),
            (r"C:\a>b", true),
        ] {
            assert_eq!(
                CLAUDE_SETTINGS_CHECK.folder_entry_blocks(Path::new(path)),
                blocks,
                "{path}"
            );
        }
    }

    // ── folder load set ──────────────────────────────────────────────────────────

    /// Stands in for the caller's YAML parser over top-level `key: value` lines; a line with `!bad` = a parse error.
    struct FakeParser;

    impl FakeParser {
        fn fields(text: &str) -> Result<HashMap<String, Value>, FrontMatterError> {
            if let Some(index) = text.lines().position(|line| line.contains("!bad")) {
                return Err(FrontMatterError {
                    message: "bad yaml".to_owned(),
                    line: Some(index as u32 + 1),
                });
            }
            Ok(text
                .lines()
                .filter_map(|line| line.split_once(':'))
                .map(|(key, value)| (key.trim().to_owned(), Self::value(value)))
                .collect())
        }

        /// `""`/`null`/`~` = null · `true` · an inline JSON object · `[a, b]` · an integer · `"quoted"` · otherwise a
        /// string.
        fn value(raw: &str) -> Value {
            let raw = raw.trim();
            match raw {
                "" | "null" | "~" => Value::Null,
                "true" => json!(true),
                _ if raw.starts_with('{') => serde_json::from_str(raw).expect("inline JSON object"),
                _ if raw.starts_with('[') && raw.ends_with(']') => {
                    let inner = raw[1..raw.len() - 1].trim();
                    if inner.is_empty() {
                        json!([])
                    } else {
                        Value::Array(inner.split(',').map(Self::value).collect())
                    }
                }
                _ => match raw.parse::<i64>() {
                    Ok(number) => json!(number),
                    Err(_) => json!(raw.trim_matches('"')),
                },
            }
        }

        fn field(fields: &HashMap<String, Value>, key: &str) -> Option<Value> {
            fields.get(key).filter(|v| !v.is_null()).cloned()
        }
    }

    impl FrontMatterParser for FakeParser {
        fn paths(&self, text: &str) -> Result<PathsFrontMatter, FrontMatterError> {
            let fields = Self::fields(text)?;
            Ok(PathsFrontMatter {
                paths: Self::field(&fields, "paths"),
            })
        }

        fn subagent(&self, text: &str) -> Result<SubagentFrontMatter, FrontMatterError> {
            let fields = Self::fields(text)?;
            Ok(SubagentFrontMatter {
                name: Self::field(&fields, "name"),
                description: Self::field(&fields, "description"),
                has_hooks: fields.contains_key("hooks"),
                has_pre_tool_use: fields.contains_key("PreToolUse"),
                has_permission_request: fields.contains_key("PermissionRequest"),
                other: fields
                    .iter()
                    .filter(|(key, _)| {
                        ![
                            "name",
                            "description",
                            "hooks",
                            "PreToolUse",
                            "PermissionRequest",
                        ]
                        .contains(&key.as_str())
                    })
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            })
        }

        fn skill(&self, _text: &str) -> Result<SkillFrontMatter, FrontMatterError> {
            Err(FrontMatterError {
                message: "not used by the claude tests".to_owned(),
                line: None,
            })
        }

        fn is_yaml_value(&self, _text: &str) -> bool {
            unreachable!("not used by the claude tests")
        }
    }

    /// A fake folder of files: relative path → text (`None` = listed but unreadable). Returns the set and the paths
    /// `read` was called with, in call order.
    fn load_set_with_reads(
        entries: &[FolderEntry],
        texts: &[(&str, Option<&str>)],
        mode: AgentOutputFormat,
    ) -> (FolderLoadSet, Vec<String>) {
        let texts: HashMap<PathBuf, Option<String>> = texts
            .iter()
            .map(|(p, t)| (PathBuf::from(p), t.map(str::to_owned)))
            .collect();
        let reads = RefCell::new(Vec::new());
        let read = |path: &Path| -> io::Result<String> {
            reads.borrow_mut().push(slash(path));
            match texts.get(path) {
                Some(Some(text)) => Ok(text.clone()),
                Some(None) | None => Err(io::Error::new(io::ErrorKind::InvalidData, "not utf-8")),
            }
        };
        let set = CLAUDE_SETTINGS_CHECK.folder_load_set(entries, &read, &FakeParser, mode);
        (set, reads.into_inner())
    }

    fn files_of(texts: &[(&str, Option<&str>)]) -> Vec<FolderEntry> {
        texts
            .iter()
            .map(|(p, _)| FolderEntry::File(PathBuf::from(p)))
            .collect()
    }

    fn load_set(texts: &[(&str, Option<&str>)], mode: AgentOutputFormat) -> FolderLoadSet {
        load_set_with_reads(&files_of(texts), texts, mode).0
    }

    fn slash(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    fn loaded(set: &FolderLoadSet) -> Vec<(String, FolderLoadKind)> {
        set.loads.iter().map(|l| (slash(&l.path), l.kind)).collect()
    }

    #[derive(Debug, PartialEq)]
    enum Outcome {
        Loads,
        NotLoaded,
        Unsure(UnsureWhy),
    }

    /// What the load set says about one file.
    fn outcome(path: &str, text: &str) -> Outcome {
        let set = load_set(&[(path, Some(text))], AgentOutputFormat::Terminal);
        match (set.loads.as_slice(), set.unsure.as_slice()) {
            ([], []) => Outcome::NotLoaded,
            ([_], []) => Outcome::Loads,
            ([], [unsure]) => Outcome::Unsure(unsure.why.clone()),
            other => panic!("{path}: {other:?}"),
        }
    }

    const AGENT_OK: &str = "---\nname: critic\ndescription: reviews\n---\nbody";

    #[test]
    fn the_folder_load_set_is_the_native_claude_set() {
        use FolderLoadKind::*;
        let texts = [
            ("CLAUDE.md", Some("# root")),
            (".claude/CLAUDE.md", Some("# dot")),
            ("CLAUDE.local.md", Some("# local")),
            (".claude/rules/plain.md", Some("no front matter")),
            (
                ".claude/rules/deep/er/nested.md",
                Some("---\ndescription: x\n---\nbody"),
            ),
            (
                ".claude/rules/scoped.md",
                Some("---\npaths: src/**/*.ts\n---\nbody"),
            ),
            (
                ".claude/rules/crlf-scoped.md",
                Some("---\r\npaths: a\r\n---\r\nbody"),
            ),
            (
                ".claude/rules/bom-scoped.md",
                Some("\u{FEFF}---\npaths: a\n---\nbody"),
            ),
            (
                ".claude/rules/null-paths.md",
                Some("---\npaths:\n---\nbody"),
            ),
            (".claude/rules/blank-block.md", Some("---\n  \n---\nbody")),
            (".claude/rules/unclosed.md", Some("---\npaths: a\nbody")),
            (
                ".claude/rules/late-fence.md",
                Some("intro\n---\npaths: a\n---\n"),
            ),
            (".claude/rules/notes.txt", Some("x")),
            (".claude/rules/UPPER.MD", Some("x")),
            (".claude/skills/review/SKILL.md", Some("x")),
            (".claude/skills/review/helper.md", Some("x")),
            (".claude/skills/SKILL.md", Some("x")),
            (".claude/skills/a/b/SKILL.md", Some("x")),
            (".claude/agents/critic.md", Some(AGENT_OK)),
            (".claude/agents/sub/deep.md", Some(AGENT_OK)),
            (".claude/agents/readme.txt", Some("x")),
            (".claude/commands/go.md", Some("x")),
            (".claude/commands/ns/deep.md", Some("x")),
            ("sub/CLAUDE.md", Some("# nested")),
            ("AGENTS.md", Some("# codex")),
            (".claude/settings.json", Some("{}")),
            (".claude/settings.local.json", Some("{}")),
            (".claude/other/x.md", Some("x")),
            (".mcp.json", Some("{}")),
            ("README.md", Some("x")),
            ("rules/x.md", Some("x")),
        ];
        for mode in MODES {
            let set = load_set(&texts, mode);
            assert_eq!(
                loaded(&set),
                vec![
                    ("CLAUDE.md".to_owned(), Instructions),
                    (".claude/CLAUDE.md".to_owned(), Instructions),
                    ("CLAUDE.local.md".to_owned(), Instructions),
                    (".claude/rules/plain.md".to_owned(), Rule),
                    (".claude/rules/deep/er/nested.md".to_owned(), Rule),
                    (".claude/rules/null-paths.md".to_owned(), Rule),
                    (".claude/rules/blank-block.md".to_owned(), Rule),
                    (".claude/rules/unclosed.md".to_owned(), Rule),
                    (".claude/rules/late-fence.md".to_owned(), Rule),
                    (".claude/skills/review/SKILL.md".to_owned(), Skill),
                    (".claude/agents/critic.md".to_owned(), Agent),
                    (".claude/agents/sub/deep.md".to_owned(), Agent),
                    (".claude/commands/go.md".to_owned(), Command),
                    (".claude/commands/ns/deep.md".to_owned(), Command),
                ],
                "{mode:?}"
            );
            assert!(set.unsure.is_empty(), "{mode:?}: {:?}", set.unsure);
            // ADR-0341: claude reads the folder itself (`--add-dir`), so the launch carries none of these rows.
            assert!(set.carried.is_empty(), "{mode:?}: {:?}", set.carried);
        }
    }

    /// Fixed names follow the OS path rule (claude opens them by a joined path); the `.md` suffix does not.
    #[test]
    fn fixed_names_compare_by_the_os_rule() {
        use FolderLoadKind::*;
        let os_folds_case = same_path(Path::new("claude.md"), Path::new("CLAUDE.md"));
        let cases = [
            ("claude.md", Instructions),
            ("Claude.Local.MD", Instructions),
            (".Claude/claude.md", Instructions),
            (".CLAUDE/Rules/r.md", Rule),
            (".claude/SKILLS/s/skill.md", Skill),
            (".claude/skills/s/Skill.md", Skill),
            (".claude/skills/s/SKILL.MD", Skill),
            (".claude/Agents/a.md", Agent),
            (".claude/COMMANDS/c.md", Command),
        ];
        for (path, kind) in cases {
            let text = if kind == Agent { AGENT_OK } else { "x" };
            let set = load_set(&[(path, Some(text))], AgentOutputFormat::Terminal);
            let expected = if os_folds_case {
                vec![(path.to_owned(), kind)]
            } else {
                vec![]
            };
            assert_eq!(loaded(&set), expected, "{path}");
        }
        let set = load_set(
            &[
                (".claude/rules/r.MD", Some("x")),
                (".claude/agents/a.Md", Some(AGENT_OK)),
                (".claude/commands/c.MD", Some("x")),
            ],
            AgentOutputFormat::Terminal,
        );
        assert_eq!(loaded(&set), vec![], "the .md suffix is case-sensitive");
    }

    #[test]
    fn rule_paths_are_predicted_only_when_clear() {
        use Outcome::*;
        let unclear = || Unsure(UnsureWhy::UnclearPaths);
        let cases = [
            ("", Loads),
            ("paths:\n", Loads),
            ("paths: ~\n", Loads),
            ("paths: src/**/*.ts\n", NotLoaded),
            ("paths: [src/**, docs/*.md]\n", NotLoaded),
            ("paths: [\"**\", src/a.ts]\n", NotLoaded),
            ("paths: src/**\n", NotLoaded),
            ("paths: \"\"\n", unclear()),
            ("paths: []\n", unclear()),
            ("paths: \"**\"\n", unclear()),
            ("paths: [\"**\"]\n", unclear()),
            ("paths: /**\n", unclear()),
            ("paths: [\"**\", /**]\n", unclear()),
            ("paths: 7\n", unclear()),
            ("paths: true\n", unclear()),
            ("paths: {}\n", unclear()),
            ("paths: [src/a.ts, 7]\n", unclear()),
            ("paths: [src/a.ts, \"\"]\n", unclear()),
            ("paths: \"a.ts,b.ts\"\n", unclear()),
            ("paths: src/{a,b}.ts\n", unclear()),
        ];
        for (front_matter, expected) in cases {
            let text = format!("---\ndescription: d\n{front_matter}---\nbody");
            assert_eq!(
                outcome(".claude/rules/r.md", &text),
                expected,
                "{front_matter:?}"
            );
        }
    }

    #[test]
    fn a_skill_with_paths_is_unsure() {
        use Outcome::*;
        for (text, expected) in [
            ("body", Loads),
            ("---\nname: s\ndescription: d\n---\nbody", Loads),
            ("---\npaths:\n---\nbody", Loads),
            (
                "---\npaths: src/**\n---\nbody",
                Unsure(UnsureWhy::UnclearPaths),
            ),
            ("---\npaths: []\n---\nbody", Unsure(UnsureWhy::UnclearPaths)),
        ] {
            assert_eq!(
                outcome(".claude/skills/s/SKILL.md", text),
                expected,
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_subagent_loads_only_with_a_valid_name_and_description() {
        use Outcome::*;
        let cases = [
            ("---\nname: critic\ndescription: reviews\n---\n", Loads),
            ("---\nname: \"critic\"\ndescription: \" \"\n---\n", Loads),
            ("---\nname: crit-ic\ndescription: reviews\n---\n", Loads),
            ("---\ndescription: reviews\n---\n", NotLoaded),
            ("---\nname: critic\n---\n", NotLoaded),
            ("---\nname: 7\ndescription: reviews\n---\n", NotLoaded),
            ("---\nname: critic\ndescription: 7\n---\n", NotLoaded),
            ("---\nname: \"\"\ndescription: reviews\n---\n", NotLoaded),
            ("---\nname: critic\ndescription: \"\"\n---\n", NotLoaded),
            ("---\nname: -critic\ndescription: reviews\n---\n", NotLoaded),
            (
                "---\nname: ns:critic\ndescription: reviews\n---\n",
                NotLoaded,
            ),
            (
                "---\nname: ns\u{FF1A}critic\ndescription: reviews\n---\n",
                NotLoaded,
            ),
            (
                "---\nname: ns\u{FE55}critic\ndescription: reviews\n---\n",
                NotLoaded,
            ),
            (
                "---\nname: a\u{FE13}b\ndescription: reviews\n---\n",
                NotLoaded,
            ),
            (
                "---\nname: a\u{2A74}b\ndescription: reviews\n---\n",
                NotLoaded,
            ),
            ("no front matter", NotLoaded),
        ];
        for (text, expected) in cases {
            assert_eq!(outcome(".claude/agents/a.md", text), expected, "{text:?}");
        }
    }

    #[test]
    fn a_valid_subagent_with_a_hook_key_is_unsure() {
        use Outcome::*;
        for key in ["hooks", "PreToolUse", "PermissionRequest"] {
            for value in ["", " x"] {
                let text = format!("---\nname: critic\ndescription: reviews\n{key}:{value}\n---\n");
                assert_eq!(
                    outcome(".claude/agents/a.md", &text),
                    Unsure(UnsureWhy::HookKeys),
                    "{text:?}"
                );
            }
        }
        // An invalid subagent is dropped whatever its hooks.
        assert_eq!(
            outcome(".claude/agents/a.md", "---\nname: critic\nhooks: x\n---\n"),
            NotLoaded
        );
    }

    #[test]
    fn dashes_inside_the_front_matter_make_the_file_unsure() {
        let dashes = || Outcome::Unsure(UnsureWhy::DashesInFrontMatter);
        for path in [
            ".claude/rules/r.md",
            ".claude/skills/s/SKILL.md",
            ".claude/agents/a.md",
        ] {
            for text in [
                "---\nname: critic\ndescription: a --- b\n---\nbody",
                "---\nname: critic\ndescription: d\n----\n---\nbody",
            ] {
                assert_eq!(outcome(path, text), dashes(), "{path} {text:?}");
            }
        }
    }

    #[test]
    fn blank_content_is_unsure() {
        use Outcome::*;
        let blank = || Unsure(UnsureWhy::EmptyBody);
        for (text, expected) in [
            ("", blank()),
            (" \n\t\n", blank()),
            ("---\ndescription: d\n---\n", blank()),
            ("---\ndescription: d\n---\n \n", blank()),
            ("---\ndescription: d\n---\n<!-- note -->\n", blank()),
            ("<!-- a -->\n <!-- b\nc -->  \n", blank()),
            ("<!-- unclosed", blank()),
            ("text <!-- unclosed", blank()),
            ("<!-- a --> text", Loads),
            ("---\npaths: src/**\n---\n", NotLoaded),
            ("---\ndescription: d\n---\nx", Loads),
        ] {
            assert_eq!(outcome(".claude/rules/r.md", text), expected, "{text:?}");
        }
        for path in ["CLAUDE.md", ".claude/CLAUDE.md", "CLAUDE.local.md"] {
            for (text, expected) in [
                ("", blank()),
                ("  \n", blank()),
                ("<!-- only a comment -->\n", blank()),
                ("<!-- unclosed", blank()),
                ("# rules", Loads),
            ] {
                assert_eq!(outcome(path, text), expected, "{path} {text:?}");
            }
        }
    }

    #[test]
    fn a_file_whose_text_cannot_be_decided_is_unsure_not_dropped() {
        let set = load_set(
            &[
                (".claude/rules/unreadable.md", None),
                (".claude/rules/bad.md", Some("---\na: 1\n!bad\n---\nbody")),
                (".claude/agents/bad.md", Some("---\n!bad\n---\nbody")),
                (".claude/agents/unreadable.md", None),
                (
                    ".claude/skills/s/SKILL.md",
                    Some("\u{FEFF}---\nx: 1\ny: 2\n!bad\n---\n"),
                ),
                ("CLAUDE.md", Some("# x")),
                (".claude/CLAUDE.md", None),
            ],
            AgentOutputFormat::Terminal,
        );
        assert_eq!(
            loaded(&set),
            vec![("CLAUDE.md".to_owned(), FolderLoadKind::Instructions)]
        );
        let bad_yaml_at_file_line = |line| {
            UnsureWhy::FrontMatter(FrontMatterError {
                message: "bad yaml".to_owned(),
                line: Some(line),
            })
        };
        assert_eq!(
            set.unsure,
            vec![
                FolderUnsure {
                    path: PathBuf::from(".claude/rules/unreadable.md"),
                    kind: Some(FolderLoadKind::Rule),
                    why: UnsureWhy::Unreadable("not utf-8".to_owned()),
                },
                FolderUnsure {
                    path: PathBuf::from(".claude/rules/bad.md"),
                    kind: Some(FolderLoadKind::Rule),
                    why: bad_yaml_at_file_line(3),
                },
                FolderUnsure {
                    path: PathBuf::from(".claude/agents/bad.md"),
                    kind: Some(FolderLoadKind::Agent),
                    why: bad_yaml_at_file_line(2),
                },
                FolderUnsure {
                    path: PathBuf::from(".claude/agents/unreadable.md"),
                    kind: Some(FolderLoadKind::Agent),
                    why: UnsureWhy::Unreadable("not utf-8".to_owned()),
                },
                FolderUnsure {
                    path: PathBuf::from(".claude/skills/s/SKILL.md"),
                    kind: Some(FolderLoadKind::Skill),
                    why: bad_yaml_at_file_line(4),
                },
                FolderUnsure {
                    path: PathBuf::from(".claude/CLAUDE.md"),
                    kind: Some(FolderLoadKind::Instructions),
                    why: UnsureWhy::Unreadable("not utf-8".to_owned()),
                },
            ]
        );
    }

    #[test]
    fn every_file_but_commands_is_read() {
        let texts = [
            ("CLAUDE.md", Some("x")),
            (".claude/CLAUDE.md", Some("x")),
            ("CLAUDE.local.md", Some("x")),
            (".claude/rules/r.md", Some("x")),
            (".claude/rules/deep/r2.md", Some("x")),
            (".claude/skills/s/SKILL.md", Some("x")),
            (".claude/skills/s/helper.md", Some("x")),
            (".claude/agents/a.md", Some(AGENT_OK)),
            (".claude/agents/sub/a2.md", Some("x")),
            (".claude/commands/c.md", Some("x")),
            ("AGENTS.md", Some("x")),
            ("README.md", Some("x")),
        ];
        let (_, reads) =
            load_set_with_reads(&files_of(&texts), &texts, AgentOutputFormat::StreamJson);
        assert_eq!(
            reads,
            vec![
                "CLAUDE.md",
                ".claude/CLAUDE.md",
                "CLAUDE.local.md",
                ".claude/rules/r.md",
                ".claude/rules/deep/r2.md",
                ".claude/skills/s/SKILL.md",
                ".claude/agents/a.md",
                ".claude/agents/sub/a2.md",
            ]
        );
    }

    #[test]
    fn an_unfollowed_link_where_claude_loads_is_unsure() {
        use FolderLoadKind::*;
        let links = [
            (".claude", None),
            ("CLAUDE.md", Some(Instructions)),
            ("CLAUDE.local.md", Some(Instructions)),
            (".claude/CLAUDE.md", Some(Instructions)),
            (".claude/rules", Some(Rule)),
            (".claude/rules/shared", Some(Rule)),
            (".claude/rules/x.md", Some(Rule)),
            (".claude/skills", Some(Skill)),
            (".claude/skills/review", Some(Skill)),
            (".claude/skills/review/SKILL.md", Some(Skill)),
            (".claude/agents", Some(Agent)),
            (".claude/agents/deep/x", Some(Agent)),
            (".claude/commands", Some(Command)),
            (".claude/commands/ns", Some(Command)),
        ];
        let ignored = [
            "docs",
            "sub/CLAUDE.md",
            ".claude/settings.json",
            ".claude/other",
            ".claude/skills/review/helper",
            "./CLAUDE.md",
        ];
        let mut entries: Vec<FolderEntry> = links
            .iter()
            .map(|(p, _)| FolderEntry::UnenteredLink(PathBuf::from(p)))
            .collect();
        entries.extend(
            ignored
                .iter()
                .map(|p| FolderEntry::UnenteredLink(PathBuf::from(p))),
        );
        let (set, reads) = load_set_with_reads(&entries, &[], AgentOutputFormat::Terminal);
        assert!(set.loads.is_empty());
        assert!(reads.is_empty());
        assert_eq!(
            set.unsure,
            links
                .iter()
                .map(|(p, kind)| FolderUnsure {
                    path: PathBuf::from(p),
                    kind: *kind,
                    why: UnsureWhy::Link,
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_path_with_a_non_normal_component_is_not_loaded() {
        let set = load_set(
            &[("./CLAUDE.md", Some("x")), ("../CLAUDE.md", Some("x"))],
            AgentOutputFormat::Terminal,
        );
        assert_eq!(loaded(&set), vec![]);
    }

    #[test]
    fn an_unclosed_front_matter_with_dashes_later_is_unsure() {
        use Outcome::*;
        let dashes = || Unsure(UnsureWhy::DashesInFrontMatter);
        for (path, text, expected) in [
            (".claude/rules/r.md", "---\npaths: a\nbody", Loads),
            (
                ".claude/rules/r.md",
                "---\npaths: a\nbody --- more",
                dashes(),
            ),
            (".claude/rules/r.md", "---\npaths: a\n----\nbody", dashes()),
            (
                ".claude/skills/s/SKILL.md",
                "---\npaths: a\n---x\n",
                dashes(),
            ),
            (
                ".claude/agents/a.md",
                "---\nname: a\ndescription: d\n--- \t x",
                dashes(),
            ),
        ] {
            assert_eq!(outcome(path, text), expected, "{path} {text:?}");
        }
    }

    #[test]
    fn a_paths_item_with_surrounding_whitespace_is_unclear() {
        use Outcome::*;
        for (paths, expected) in [
            ("[\" src/a.ts\"]", Unsure(UnsureWhy::UnclearPaths)),
            ("[\"src/a.ts \"]", Unsure(UnsureWhy::UnclearPaths)),
            ("\"   \"", Unsure(UnsureWhy::UnclearPaths)),
            ("[src/a.ts, \" \"]", Unsure(UnsureWhy::UnclearPaths)),
            ("[\"src/a.ts\"]", NotLoaded),
        ] {
            let text = format!("---\npaths: {paths}\n---\nbody");
            assert_eq!(outcome(".claude/rules/r.md", &text), expected, "{paths}");
        }
    }

    #[test]
    fn a_command_folder_with_a_skill_file_keeps_only_that_file() {
        use FolderLoadKind::Command;
        let texts = [
            (".claude/commands/go.md", Some("x")),
            (".claude/commands/ns/SKILL.md", Some("x")),
            (".claude/commands/ns/other.md", Some("x")),
            (".claude/commands/ns/deep/x.md", Some("x")),
            (".claude/commands/two/SKILL.md", Some("x")),
            (".claude/commands/two/skill.md", Some("x")),
            (".claude/commands/plain/a.md", Some("x")),
        ];
        let set = load_set(&texts, AgentOutputFormat::Terminal);
        assert_eq!(
            loaded(&set),
            vec![
                (".claude/commands/go.md".to_owned(), Command),
                (".claude/commands/ns/SKILL.md".to_owned(), Command),
                (".claude/commands/ns/deep/x.md".to_owned(), Command),
                (".claude/commands/plain/a.md".to_owned(), Command),
            ]
        );
        let unsure: Vec<String> = set.unsure.iter().map(|u| slash(&u.path)).collect();
        assert_eq!(
            unsure,
            vec![
                ".claude/commands/ns/other.md",
                ".claude/commands/two/SKILL.md",
                ".claude/commands/two/skill.md",
            ]
        );
        assert!(set
            .unsure
            .iter()
            .all(|u| u.kind == Some(Command) && u.why == UnsureWhy::SkillFileInCommandFolder));

        // A skill file directly in `.claude/commands/` makes the top-level commands unsure.
        let set = load_set(
            &[
                (".claude/commands/Skill.md", Some("x")),
                (".claude/commands/go.md", Some("x")),
                (".claude/commands/ns/x.md", Some("x")),
            ],
            AgentOutputFormat::Terminal,
        );
        assert_eq!(
            loaded(&set),
            vec![
                (".claude/commands/Skill.md".to_owned(), Command),
                (".claude/commands/ns/x.md".to_owned(), Command),
            ]
        );
        assert_eq!(
            set.unsure
                .iter()
                .map(|u| slash(&u.path))
                .collect::<Vec<_>>(),
            vec![".claude/commands/go.md"]
        );
    }

    #[test]
    fn a_nested_hook_key_makes_a_valid_subagent_unsure() {
        use Outcome::*;
        let hooks = || Unsure(UnsureWhy::HookKeys);
        for (extra, expected) in [
            (r#"tools: {"PreToolUse": 1}"#, hooks()),
            (r#"x: {"a": {"b": {"hooks": 1}}}"#, hooks()),
            (r#"x: {"a": [{"PermissionRequest": 1}]}"#, hooks()),
            (r#"x: {"a": {"b": {"c": {"hooks": 1}}}}"#, Loads),
            (r#"metadata: {"hooks": 1}"#, Loads),
            (r#"mcpServers: {"s": {"PreToolUse": 1}}"#, Loads),
            (r#"lspServers: {"hooks": 1}"#, Loads),
            (r#"x: {"a": 1}"#, Loads),
        ] {
            let text = format!("---\nname: critic\ndescription: d\n{extra}\n---\n");
            assert_eq!(outcome(".claude/agents/a.md", &text), expected, "{extra}");
        }
    }

    #[test]
    fn split_front_matter_returns_the_block_and_the_body() {
        let cases = [
            ("---\na: 1\n---\nbody", Some("a: 1\n"), "body"),
            ("---\r\na: 1\r\n---\r\nb\r\n", Some("a: 1\r\n"), "b\r\n"),
            ("---  \na: 1\n---", Some("a: 1\n"), ""),
            ("\u{FEFF}---\na: 1\n---\n", Some("a: 1\n"), ""),
            ("---\n---\nbody", None, "body"),
            ("---\n \n\t\n---\nbody", None, "body"),
            ("---\na: 1\n", None, "---\na: 1\n"),
            ("body\n---\na: 1\n---\n", None, "body\n---\na: 1\n---\n"),
            ("\u{FEFF}body", None, "body"),
            ("", None, ""),
        ];
        for (text, block, body) in cases {
            assert_eq!(split_front_matter(text), Ok((block, body)), "{text:?}");
        }
    }
}
