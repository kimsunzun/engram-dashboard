//! The codex implementation of the settings-check port ([`SettingsCheck`]) — [`CODEX_SETTINGS_CHECK`].
//!
//! ★Codex behaviour copied by hand, read in the codex source (openai/codex, Apache-2.0) at `rust-v0.163.0`★: the `-c`
//!   line split (`codex-rs/utils/cli/src/config_override.rs`), the `SKILL.md` front-matter split and repair
//!   (`codex-rs/skills/src/parser.rs`), the skill-root walk
//!   (`codex-rs/ext/skills/src/loader/discovery.rs` over the exec-server `walk`) and codex's own skill roots
//!   (`codex-rs/ext/skills/src/host_roots.rs`). Codex updates itself; a drift shows as a valid skill blocked or a
//!   bad one passed. The real-YAML golden of the skill format runs with the caller's parser, not here.
// ADR-0334
// ADR-0337
// ADR-0341

use std::ffi::{OsStr, OsString};
use std::path::Path;

use serde_json::{json, Value};

use super::cmdline;
use super::thread_lock::{codex_home_from, CODEX_HOME_ENV};
use crate::backend::settings_check::{
    is, normal_parts, CommandLine, FolderEntry, FolderLoad, FolderLoadKind, FolderLoadSet,
    FolderMalformed, FolderSkillName, FolderUnsure, FrontMatterParser, InstructionLoad, ModeLoads,
    NativePlaces, NativeSkillRoots, NoticeCodes, SettingsCheck, SkillFormat, UnsureWhy,
};
use crate::launch::{
    Reason, CODEX_SAME_NAME_SKILLS, CODEX_TERMINAL_FOLDER_OMITTED, CODEX_TERMINAL_OMITTED,
};
use crate::profile::AgentOutputFormat;

/// The longest skill name codex accepts, in characters.
const MAX_SKILL_NAME_CHARS: usize = 64;

/// The only file name codex loads as a skill — compared as an exact string, as codex's walker does.
const SKILL_FILE: &str = "SKILL.md";

/// codex names a skill without a front-matter `name` after its folder, and calls it this when that is blank too.
const FALLBACK_SKILL_NAME: &str = "skill";

/// The folder name of every skill root this port names (both folder-entry roots and every root of
/// [`SettingsCheck::native_skill_roots`]) — the fallback name of a `SKILL.md` that lies directly in a root listed by
/// [`SettingsCheck::root_skills`], whose listing does not carry the root's own name.
const SKILL_ROOT_FOLDER_NAME: &str = "skills";

/// How many folder levels below a skill root codex descends (`MAX_SCAN_DEPTH`): the walk reads the root and every
/// folder up to this many levels below it, so `root/a/b/c/d/e/f/SKILL.md` is found and one level more is not.
const MAX_SCAN_DEPTH: usize = 6;

/// The instruction files codex reads from a folder: the override wins by existence alone.
// ADR-0341
const AGENTS_OVERRIDE_FILE: &str = "AGENTS.override.md";
const AGENTS_FILE: &str = "AGENTS.md";

/// The skill roots inside a folder entry (and the per-folder roots codex reads by itself), in this order.
// ADR-0341
const SKILL_ROOTS: [[&str; 2]; 2] = [[".agents", "skills"], [".codex", "skills"]];

/// `default_project_root_markers()` (`codex-rs/config/src/project_root_markers.rs`).
const PROJECT_ROOT_MARKERS: [&str; 1] = [".git"];

pub(crate) static CODEX_SETTINGS_CHECK: CodexSettingsCheck = CodexSettingsCheck;

pub(crate) struct CodexSettingsCheck;

impl SettingsCheck for CodexSettingsCheck {
    fn backend(&self) -> &'static str {
        "codex"
    }

    // ADR-0337
    fn loads(&self, mode: AgentOutputFormat) -> ModeLoads {
        match mode {
            AgentOutputFormat::StreamJson => ModeLoads {
                instructions: InstructionLoad::All,
                skills: true,
            },
            AgentOutputFormat::Terminal => ModeLoads {
                instructions: InstructionLoad::BuiltinPrimingOnly,
                skills: false,
            },
        }
    }

    /// Engram's rule for a library skill a codex JSON agent loads: 1–64 characters of `[a-z0-9-]`. The terminal
    /// mode loads no library skills, so no name is an error there (M24-3).
    fn accepts_skill_name(&self, name: &str, mode: AgentOutputFormat) -> bool {
        match mode {
            AgentOutputFormat::StreamJson => {
                (1..=MAX_SKILL_NAME_CHARS).contains(&name.len())
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            }
            AgentOutputFormat::Terminal => true,
        }
    }

    /// codex puts no folder path on its command line — the skill roots travel in JSON.
    // ADR-0341
    fn folder_entry_blocks(&self, _folder: &Path) -> bool {
        false
    }

    /// The terminal mode puts the working folder on its command line (`--cd`) — the rule
    /// [`cmdline::cwd_reaches_codex_unchanged`], the same function the terminal launch translation applies. JSON
    /// passes the folder in JSON.
    // ADR-0353
    fn working_folder_blocks(&self, cwd: &Path, mode: AgentOutputFormat) -> Option<Reason> {
        match mode {
            AgentOutputFormat::Terminal if !cmdline::cwd_reaches_codex_unchanged(cwd) => {
                Some(reason(
                    "codex-working-folder-unsafe",
                    json!({ "cwd": cwd.to_string_lossy() }),
                    None,
                ))
            }
            AgentOutputFormat::Terminal | AgentOutputFormat::StreamJson => None,
        }
    }

    /// The terminal mode loads neither library instructions and skills nor folder entries; JSON loads skills from
    /// several roots, where one name can select none of them.
    // ADR-0337
    // ADR-0341
    // ADR-0355
    fn notice_codes(&self, mode: AgentOutputFormat) -> NoticeCodes {
        match mode {
            AgentOutputFormat::Terminal => NoticeCodes {
                omitted: Some(CODEX_TERMINAL_OMITTED),
                folder_omitted: Some(CODEX_TERMINAL_FOLDER_OMITTED),
                same_name: None,
            },
            AgentOutputFormat::StreamJson => NoticeCodes {
                same_name: Some(CODEX_SAME_NAME_SKILLS),
                ..NoticeCodes::default()
            },
        }
    }

    /// JSON carries rows from the listing; the terminal mode names from it what it omits.
    // ADR-0341
    fn folder_listing_at_launch(&self, _mode: AgentOutputFormat) -> bool {
        true
    }

    /// codex JSON loads from a folder entry: the instruction file (`AGENTS.override.md` if it exists — even empty —
    /// else `AGENTS.md`, names compared by the OS rule; a blank file adds nothing) and the skill roots
    /// `.agents/skills` · `.codex/skills` with the skills codex's walk finds under them. Nothing else in the folder
    /// is loaded. A root is reported when an entry lies under it (the listing has no folder entries, so an empty
    /// root is not seen). Only clear cases are predicted; a link where codex would follow it is `unsure`, a skill
    /// that fails the format check is `malformed`.
    ///
    /// JSON carries the instruction file and the skill roots itself (`carried`); the skills under a root reach
    /// codex through the root. The terminal mode loads nothing from the folder; its `omitted` is the JSON `loads`,
    /// computed by the same code, so the omission notice and the JSON launch cannot disagree.
    // ADR-0341
    fn folder_load_set(
        &self,
        entries: &[FolderEntry],
        read: &dyn Fn(&Path) -> std::io::Result<String>,
        parser: &dyn FrontMatterParser,
        mode: AgentOutputFormat,
    ) -> FolderLoadSet {
        let json = json_folder_load_set(entries, read, parser);
        match mode {
            AgentOutputFormat::StreamJson => {
                let carried = json
                    .loads
                    .iter()
                    .filter(|row| {
                        matches!(
                            row.kind,
                            FolderLoadKind::Instructions | FolderLoadKind::SkillRoot
                        )
                    })
                    .cloned()
                    .collect();
                FolderLoadSet { carried, ..json }
            }
            AgentOutputFormat::Terminal => FolderLoadSet {
                omitted: json.loads,
                ..FolderLoadSet::default()
            },
        }
    }

    /// The codex JSON check (`parse_skill_frontmatter_metadata`): the file name is exactly `SKILL.md`; a `---`
    /// front matter (a leading BOM is not stripped, so it hides the front matter); the front matter parses with the
    /// caller's parser, once more after codex's repair when the first parse fails (the error reported is the first
    /// one); the name (whitespace collapsed, else the folder name) has at most 64 characters; the description is
    /// not blank. The terminal mode loads no library skills.
    fn skill_format(
        &self,
        file_name: &str,
        folder_name: &str,
        text: &str,
        parser: &dyn FrontMatterParser,
        mode: AgentOutputFormat,
    ) -> SkillFormat {
        match mode {
            AgentOutputFormat::StreamJson => {
                match check_skill(file_name, folder_name, text, parser) {
                    Ok((name, description)) => SkillFormat::Valid { name, description },
                    Err(reason) => SkillFormat::Invalid(reason),
                }
            }
            AgentOutputFormat::Terminal => SkillFormat::NotChecked,
        }
    }

    fn command_line_args(&self, raw: &Value) -> Result<Vec<String>, Reason> {
        cmdline::config_args(raw)
    }

    fn command_line_fits(&self, line: &CommandLine<'_>) -> Result<(), Reason> {
        cmdline::fits(line)
    }

    /// codex JSON reads `$CODEX_HOME/skills` (default `<home>/.codex/skills`), `<home>/.agents/skills`, and
    /// `.agents/skills` in every folder from the project root (found by the markers) down to `cwd`, plus the skills
    /// folder of every project config layer (a project `.codex/skills` — which levels codex loads as a layer is not
    /// read, so every level is a candidate: at worst an extra notice, never a missed one).
    ///
    /// Not visible here: codex's bundled system skills, the admin config layer, plugin roots and project-root
    /// markers set in the user's codex config. The terminal mode loads no Engram skills, so it needs no notice.
    fn native_skill_roots(
        &self,
        places: &NativePlaces<'_>,
        mode: AgentOutputFormat,
    ) -> NativeSkillRoots {
        match mode {
            AgentOutputFormat::StreamJson => {
                let codex_home = codex_home_from(
                    (places.env)(CODEX_HOME_ENV).map(OsString::from),
                    Some(places.home.to_path_buf()),
                )
                .expect("a user home is given");
                let [agents, _] = SKILL_ROOTS;
                NativeSkillRoots {
                    fixed: vec![
                        codex_home.join(SKILL_ROOT_FOLDER_NAME),
                        places.home.join(agents[0]).join(agents[1]),
                    ],
                    per_folder: SKILL_ROOTS
                        .iter()
                        .map(|[dir, sub]| Path::new(dir).join(sub))
                        .collect(),
                    project_markers: PROJECT_ROOT_MARKERS.map(str::to_owned).to_vec(),
                }
            }
            AgentOutputFormat::Terminal => NativeSkillRoots::default(),
        }
    }

    /// codex walks its own roots with the same discovery as an extra root (`discover_skills`, hidden folders
    /// skipped, directory links followed except in the bundled system root).
    fn root_skills(
        &self,
        entries: &[FolderEntry],
        read: &dyn Fn(&Path) -> std::io::Result<String>,
        parser: &dyn FrontMatterParser,
    ) -> FolderLoadSet {
        let mut set = FolderLoadSet::default();
        for entry in entries {
            let (path, is_link) = entry_path(entry);
            if let Some(parts) = normal_parts(path) {
                walk_entry(
                    &mut set,
                    path,
                    &parts,
                    is_link,
                    SKILL_ROOT_FOLDER_NAME,
                    read,
                    parser,
                );
            }
        }
        set
    }
}

pub(super) fn reason(code: &str, args: Value, line: Option<u32>) -> Reason {
    Reason {
        code: code.to_owned(),
        args,
        line,
    }
}

/// The `key=value` strings of one raw codex value. `Err` = not an array of strings.
pub(super) fn config_lines(raw: &Value) -> Result<Vec<&str>, Reason> {
    let Value::Array(items) = raw else {
        return Err(reason("codex-config-not-a-list", json!({}), None));
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            item.as_str()
                .ok_or_else(|| reason("codex-config-not-a-list", json!({ "index": index }), None))
        })
        .collect()
}

/// Whether codex can split one `-c` line (at the first `=`, the key trimmed). `Err` = no `=`, an empty key, or a line
/// that starts with `-` after trimming. codex exits at startup on the first two. A bare line starting
/// with `-` is likely read by clap as an option (inferred: codex's `-c` does not set `allow_hyphen_values`); one with leading
/// whitespace would reach codex, but no codex key starts with `-`, so it is refused too.
pub(super) fn split_config_line(index: usize, line: &str) -> Result<(), Reason> {
    let invalid = || {
        reason(
            "codex-config-line-invalid",
            json!({ "index": index, "line": line }),
            None,
        )
    };
    if line.trim_start().starts_with('-') {
        return Err(invalid());
    }
    let (key, _) = line.split_once('=').ok_or_else(invalid)?;
    if key.trim().is_empty() {
        return Err(invalid());
    }
    Ok(())
}

/// The codex skill check of [`SettingsCheck::skill_format`] — `Ok((name, description))`.
fn check_skill(
    file_name: &str,
    folder_name: &str,
    text: &str,
    parser: &dyn FrontMatterParser,
) -> Result<(String, String), Reason> {
    if file_name != SKILL_FILE {
        return Err(reason(
            "codex-skill-file-name",
            json!({ "file": file_name }),
            None,
        ));
    }
    let Some(block) = extract_front_matter(text) else {
        return Err(reason("codex-skill-no-front-matter", json!({}), None));
    };
    let front = match parser.skill(&block) {
        Ok(front) => front,
        Err(original) => {
            match repair_front_matter(&block, parser).and_then(|r| parser.skill(&r).ok()) {
                Some(front) => front,
                // The front matter starts on the file's second line.
                None => {
                    return Err(reason(
                        "codex-skill-front-matter",
                        json!({ "message": original.message }),
                        original.line.map(|line| line.saturating_add(1)),
                    ))
                }
            }
        }
    };
    let name = front
        .name
        .as_deref()
        .map(single_line)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| {
            Some(single_line(folder_name))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| FALLBACK_SKILL_NAME.to_owned())
        });
    if name.chars().count() > MAX_SKILL_NAME_CHARS {
        return Err(reason(
            "codex-skill-name-too-long",
            json!({ "name": name }),
            None,
        ));
    }
    let description = front
        .description
        .as_deref()
        .map(single_line)
        .unwrap_or_default();
    if description.is_empty() {
        return Err(reason("codex-skill-description-empty", json!({}), None));
    }
    Ok((name, description))
}

/// codex `sanitize_single_line` — whitespace runs collapsed to one space, trimmed.
fn single_line(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether a line is a front-matter fence: `---` once trimmed (`trim` leaves U+FEFF, so a BOM hides the fence).
fn is_fence(line: &str) -> bool {
    line.trim() == "---"
}

/// The front matter of a `SKILL.md`: the lines between an opening fence on the first line and the next fence,
/// joined with `\n`. `None` = no opening fence, no closing fence, or nothing between them. A block of only
/// whitespace is returned (codex goes on to parse it).
// Behaviour mirrors codex-rs/skills/src/parser.rs `extract_frontmatter` @ rust-v0.163.0 (Apache-2.0).
fn extract_front_matter(text: &str) -> Option<String> {
    let mut lines = text.lines();
    if !lines.next().is_some_and(is_fence) {
        return None;
    }
    let rest: Vec<&str> = lines.collect();
    let closing = rest.iter().position(|line| is_fence(line))?;
    (closing > 0).then(|| rest[..closing].join("\n"))
}

/// What the repair does to one front-matter line.
enum LineRepair {
    Keep,
    /// The line opens a block scalar (`|` / `>`): the more-indented lines after it are its text.
    OpenBlock,
    Replace(String),
}

/// The front matter with every plain scalar value YAML would refuse wrapped in single quotes, or `None` when no line
/// changed (codex then parses no second time). Lines inside a block scalar are left alone; every other `key: value`
/// line, at any indent, goes through [`repair_line`].
// Behaviour mirrors codex-rs/skills/src/parser.rs `repair_frontmatter_scalar_fields` @ rust-v0.163.0 (Apache-2.0).
fn repair_front_matter(front_matter: &str, parser: &dyn FrontMatterParser) -> Option<String> {
    let mut block_owner_indent: Option<usize> = None;
    let mut changed = false;
    let lines: Vec<String> = front_matter
        .lines()
        .map(|line| {
            let indent = line.len() - line.trim_start_matches(' ').len();
            if let Some(owner) = block_owner_indent {
                if line.trim().is_empty() || indent > owner {
                    return line.to_owned();
                }
                block_owner_indent = None;
            }
            match repair_line(line, parser) {
                LineRepair::Keep => line.to_owned(),
                LineRepair::OpenBlock => {
                    block_owner_indent = Some(indent);
                    line.to_owned()
                }
                LineRepair::Replace(fixed) => {
                    changed = true;
                    fixed
                }
            }
        })
        .collect();
    changed.then(|| lines.join("\n"))
}

/// One `key: value` line: a plain value (not quoted, not a block scalar) is quoted when it holds a `:` followed by
/// whitespace, or starts with `[`, `{`, `@` or `` ` `` and is not a YAML value on its own (the caller's parser
/// decides). A value must be empty or follow the `:` after whitespace; a trailing comment (a `#` at the start of the
/// value or after whitespace) stays outside the quotes, with the whitespace before it.
fn repair_line(line: &str, parser: &dyn FrontMatterParser) -> LineRepair {
    let Some((key, after_colon)) = line.split_once(':') else {
        return LineRepair::Keep;
    };
    if key.trim().is_empty() || after_colon.starts_with(|c: char| !c.is_whitespace()) {
        return LineRepair::Keep;
    }
    let value = after_colon.trim_start();
    let gap = &after_colon[..after_colon.len() - value.len()];
    let (scalar, comment) = split_comment(value);
    let scalar = scalar.trim_end();
    let Some(first) = scalar.chars().next() else {
        return LineRepair::Keep;
    };
    match first {
        '|' | '>' => LineRepair::OpenBlock,
        '\'' | '"' => LineRepair::Keep,
        _ if has_mapping_colon(scalar)
            || (matches!(first, '[' | '{' | '@' | '`') && !parser.is_yaml_value(scalar)) =>
        {
            LineRepair::Replace(format!(
                "{key}:{gap}'{}'{comment}",
                scalar.replace('\'', "''")
            ))
        }
        _ => LineRepair::Keep,
    }
}

/// A value and its trailing comment: the comment starts at the first `#` that opens the value or follows
/// whitespace, and takes the whitespace before it. No comment = `(value, "")`.
fn split_comment(value: &str) -> (&str, &str) {
    let hash = value.char_indices().find(|&(index, character)| {
        character == '#'
            && value[..index]
                .chars()
                .next_back()
                .is_none_or(char::is_whitespace)
    });
    match hash {
        Some((index, _)) => {
            let kept = value[..index].trim_end().len();
            value.split_at(kept)
        }
        None => (value, ""),
    }
}

/// Whether a plain value holds a `:` followed by whitespace — YAML would read a second mapping there.
fn has_mapping_colon(scalar: &str) -> bool {
    scalar
        .split(':')
        .skip(1)
        .any(|after| after.starts_with(char::is_whitespace))
}

fn entry_path(entry: &FolderEntry) -> (&Path, bool) {
    match entry {
        FolderEntry::File(path) => (path, false),
        FolderEntry::UnenteredLink(path) => (path, true),
    }
}

/// The codex JSON load set of one folder entry (see [`SettingsCheck::folder_load_set`]).
fn json_folder_load_set(
    entries: &[FolderEntry],
    read: &dyn Fn(&Path) -> std::io::Result<String>,
    parser: &dyn FrontMatterParser,
) -> FolderLoadSet {
    let instructions = instructions_pick(entries);
    let mut set = FolderLoadSet::default();
    let mut root_reported = [false; SKILL_ROOTS.len()];
    for (index, entry) in entries.iter().enumerate() {
        let (path, is_link) = entry_path(entry);
        if instructions == Some(index) {
            load_instructions(&mut set, path, is_link, read);
            continue;
        }
        let Some(parts) = normal_parts(path) else {
            continue;
        };
        let root = SKILL_ROOTS
            .iter()
            .position(|[dir, sub]| parts.len() >= 2 && is(parts[0], dir) && is(parts[1], sub));
        match (root, parts.as_slice()) {
            // `.agents` or `.codex` itself as a link — a root may lie behind it.
            (None, [only]) if is_link && SKILL_ROOTS.iter().any(|[dir, _]| is(only, dir)) => {
                set.unsure.push(FolderUnsure {
                    path: path.to_owned(),
                    kind: Some(FolderLoadKind::SkillRoot),
                    why: UnsureWhy::Link,
                });
            }
            // The root itself as a link — codex follows it (M1), the listing did not.
            (Some(_), [_, _]) if is_link => set.unsure.push(FolderUnsure {
                path: path.to_owned(),
                kind: Some(FolderLoadKind::SkillRoot),
                why: UnsureWhy::Link,
            }),
            (Some(root), [dir, sub, below @ ..]) if !below.is_empty() => {
                if !root_reported[root] {
                    root_reported[root] = true;
                    set.loads.push(FolderLoad {
                        path: [dir, sub].iter().collect(),
                        kind: FolderLoadKind::SkillRoot,
                    });
                }
                walk_entry(
                    &mut set,
                    path,
                    below,
                    is_link,
                    sub.to_str().unwrap_or_default(),
                    read,
                    parser,
                );
            }
            _ => {}
        }
    }
    set
}

/// The listing index of the instruction file codex reads: a top-level `AGENTS.override.md` if one is listed, else
/// a top-level `AGENTS.md` — the OS name rule, as codex opens them by a joined path.
// ADR-0341
fn instructions_pick(entries: &[FolderEntry]) -> Option<usize> {
    let top_level = |name: &str| {
        entries.iter().position(|entry| {
            matches!(normal_parts(entry_path(entry).0).as_deref(), Some([only]) if is(only, name))
        })
    };
    top_level(AGENTS_OVERRIDE_FILE).or_else(|| top_level(AGENTS_FILE))
}

/// The picked instruction file. A blank file loads nothing and is not replaced by the other name (M7).
fn load_instructions(
    set: &mut FolderLoadSet,
    path: &Path,
    is_link: bool,
    read: &dyn Fn(&Path) -> std::io::Result<String>,
) {
    let kind = FolderLoadKind::Instructions;
    if is_link {
        set.unsure.push(FolderUnsure {
            path: path.to_owned(),
            kind: Some(kind),
            why: UnsureWhy::Link,
        });
        return;
    }
    match read(path) {
        Ok(text) if text.trim().is_empty() => {}
        Ok(_) => set.loads.push(FolderLoad {
            path: path.to_owned(),
            kind,
        }),
        Err(error) => set.unsure.push(FolderUnsure {
            path: path.to_owned(),
            kind: Some(kind),
            why: UnsureWhy::Unreadable(error.to_string()),
        }),
    }
}

/// One listed entry under a skill root, judged by codex's walk. `below` = its components below the root (not
/// empty); `root_name` = the root folder's own name (the fallback name of a `SKILL.md` directly in the root).
///
/// The walk (`discover_skills` over the exec-server `walk`): the root itself is read whatever its name; a folder
/// below it whose name starts with `.` is not entered; folders up to [`MAX_SCAN_DEPTH`] levels below the root are
/// read; a file counts only when its name is exactly `SKILL.md`, wherever it lies (directly in the root too).
/// codex follows a directory link below the root (M1) and skips a file link — a link that the walk would still
/// enter is `unsure`.
///
/// ★Not mirrored — codex's walk caps★: codex ends a root's walk at 20,000 entries, stops queueing new folders past
///   2,000, and has a response-byte budget. Past them it can load fewer skills than this walk reports (those beyond
///   the cutoff), so for a very large root the preview can show skills codex never reaches.
fn walk_entry(
    set: &mut FolderLoadSet,
    path: &Path,
    below: &[&OsStr],
    is_link: bool,
    root_name: &str,
    read: &dyn Fn(&Path) -> std::io::Result<String>,
    parser: &dyn FrontMatterParser,
) {
    let Some((name, folders)) = below.split_last() else {
        return;
    };
    if folders.len() > MAX_SCAN_DEPTH || folders.iter().any(|folder| is_hidden(folder)) {
        return;
    }
    if is_link {
        if folders.len() < MAX_SCAN_DEPTH && !is_hidden(name) {
            set.unsure.push(FolderUnsure {
                path: path.to_owned(),
                kind: Some(FolderLoadKind::Skill),
                why: UnsureWhy::Link,
            });
        }
        return;
    }
    if name.as_encoded_bytes() != SKILL_FILE.as_bytes() {
        return;
    }
    let text = match read(path) {
        Ok(text) => text,
        Err(error) => {
            set.unsure.push(FolderUnsure {
                path: path.to_owned(),
                kind: Some(FolderLoadKind::Skill),
                why: UnsureWhy::Unreadable(error.to_string()),
            });
            return;
        }
    };
    // A folder name that is not UTF-8 gives no fallback name (codex `default_skill_name` reads it with `to_str`).
    let folder_name = folders
        .last()
        .map_or(Some(root_name), |folder| folder.to_str())
        .unwrap_or_default();
    match check_skill(SKILL_FILE, folder_name, &text, parser) {
        Ok((name, _)) => {
            set.loads.push(FolderLoad {
                path: path.to_owned(),
                kind: FolderLoadKind::Skill,
            });
            set.skill_names.push(FolderSkillName {
                path: path.to_owned(),
                name,
            });
        }
        Err(reason) => set.malformed.push(FolderMalformed {
            path: path.to_owned(),
            reason,
        }),
    }
}

/// A name codex's walk treats as hidden.
fn is_hidden(part: &OsStr) -> bool {
    part.as_encoded_bytes().first() == Some(&b'.')
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::io;
    use std::path::PathBuf;

    use engram_dashboard_platform::fs::same_path;
    use serde_json::json;

    use super::*;
    use crate::backend::settings_check::{
        FrontMatterError, PathsFrontMatter, SkillFrontMatter, SkillMetadata, SubagentFrontMatter,
    };

    const MODES: [AgentOutputFormat; 2] =
        [AgentOutputFormat::Terminal, AgentOutputFormat::StreamJson];
    const JSON: AgentOutputFormat = AgentOutputFormat::StreamJson;
    const TERMINAL: AgentOutputFormat = AgentOutputFormat::Terminal;

    // ── simple answers ───────────────────────────────────────────────────────────

    #[test]
    fn codex_json_loads_everything_and_the_terminal_only_the_builtin_priming() {
        assert_eq!(
            CODEX_SETTINGS_CHECK.loads(JSON),
            ModeLoads {
                instructions: InstructionLoad::All,
                skills: true,
            }
        );
        assert_eq!(
            CODEX_SETTINGS_CHECK.loads(TERMINAL),
            ModeLoads {
                instructions: InstructionLoad::BuiltinPrimingOnly,
                skills: false,
            }
        );
    }

    #[test]
    fn skill_names_are_checked_only_in_codex_json() {
        let sixty_four = "a".repeat(64);
        let sixty_five = "a".repeat(65);
        for (name, json_ok) in [
            ("ok-name-2", true),
            (sixty_four.as_str(), true),
            ("-", true),
            (sixty_five.as_str(), false),
            ("", false),
            ("Upper", false),
            ("under_score", false),
            ("dot.name", false),
            ("spa ce", false),
            ("é", false),
        ] {
            assert_eq!(
                CODEX_SETTINGS_CHECK.accepts_skill_name(name, JSON),
                json_ok,
                "{name:?}"
            );
            assert!(
                CODEX_SETTINGS_CHECK.accepts_skill_name(name, TERMINAL),
                "{name:?}"
            );
        }
    }

    #[test]
    fn codex_never_blocks_a_folder_entry() {
        for folder in [r"C:\rules\50%\x", r"C:\a&b", r"C:\plain"] {
            assert!(!CODEX_SETTINGS_CHECK.folder_entry_blocks(Path::new(folder)));
        }
    }

    /// ★The terminal working-folder rule is `cwd_reaches_codex_unchanged`★ — refused: two `%`, a `"`, a control
    /// character, `& ^ | < >` without a space; passed: a single `%`, `& ^ | < >` with a space. JSON never blocks.
    // ADR-0353
    #[test]
    fn the_working_folder_blocks_only_the_terminal_mode_by_the_cd_rule() {
        let refused = [
            r"C:\Work\R&D",
            r"C:\Work\a^b",
            r"C:\Work\a|b",
            r"C:\Work\a<b>",
            r"D:\%USERNAME% Projects",
            r"C:\%A%\b",
            r#"D:\a b"&calc"#,
            "D:\\a b\u{7}",
        ];
        for cwd in refused {
            assert_eq!(
                CODEX_SETTINGS_CHECK.working_folder_blocks(Path::new(cwd), TERMINAL),
                Some(reason(
                    "codex-working-folder-unsafe",
                    json!({ "cwd": cwd }),
                    None
                )),
                "{cwd:?}"
            );
            assert_eq!(
                CODEX_SETTINGS_CHECK.working_folder_blocks(Path::new(cwd), JSON),
                None,
                "{cwd:?}"
            );
        }
        for cwd in [
            r"D:\R&D Projects",
            r"D:\a ^ | < > b",
            r"C:\Work\100%",
            r"D:\100% Projects",
            r"C:\plain",
        ] {
            for mode in [TERMINAL, JSON] {
                assert_eq!(
                    CODEX_SETTINGS_CHECK.working_folder_blocks(Path::new(cwd), mode),
                    None,
                    "{cwd:?} {mode:?}"
                );
            }
        }
    }

    // ADR-0337
    // ADR-0341
    // ADR-0355
    #[test]
    fn the_notice_codes_follow_the_mode() {
        assert_eq!(
            CODEX_SETTINGS_CHECK.notice_codes(TERMINAL),
            NoticeCodes {
                omitted: Some("codex-terminal-omitted"),
                folder_omitted: Some("codex-terminal-folder-omitted"),
                same_name: None,
            }
        );
        assert_eq!(
            CODEX_SETTINGS_CHECK.notice_codes(JSON),
            NoticeCodes {
                omitted: None,
                folder_omitted: None,
                same_name: Some("codex-same-name-skills"),
            }
        );
    }

    #[test]
    fn every_mode_lists_the_folder_entries_at_launch() {
        for mode in [TERMINAL, JSON] {
            assert!(
                CODEX_SETTINGS_CHECK.folder_listing_at_launch(mode),
                "{mode:?}"
            );
        }
    }

    // ── skill format ─────────────────────────────────────────────────────────────

    /// Stands in for the caller's YAML parser and records every text [`FrontMatterParser::skill`] is given.
    /// Top-level and indented `key: value` lines; a `'…'` value is a string (`''` → `'`); an unquoted value starting
    /// with `[` or `{` is a collection. Parse errors, at their line: a line holding `!bad`; an unquoted value holding
    /// `: ` (as YAML refuses a second mapping on one line); a collection for `name` or `description`, or a
    /// non-empty value for `metadata` that is not a `{…}` mapping (the typed fields of [`SkillFrontMatter`]).
    /// [`FrontMatterParser::is_yaml_value`]: a `[…]` or `{…}` without `@` or `` ` ``.
    #[derive(Default)]
    struct FakeParser {
        calls: RefCell<Vec<String>>,
    }

    /// One fake YAML value.
    enum FakeValue {
        Text(String),
        Collection(String),
    }

    impl FakeParser {
        fn value(raw: &str) -> Result<FakeValue, &'static str> {
            let raw = raw.trim();
            match raw.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
                Some(inner) => Ok(FakeValue::Text(inner.replace("''", "'"))),
                None if raw.contains(": ") => Err("mapping values"),
                None if raw.starts_with(['[', '{']) => Ok(FakeValue::Collection(raw.to_owned())),
                None => Ok(FakeValue::Text(raw.to_owned())),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl FrontMatterParser for FakeParser {
        fn paths(&self, _text: &str) -> Result<PathsFrontMatter, FrontMatterError> {
            unreachable!("codex reads no paths")
        }

        fn subagent(&self, _text: &str) -> Result<SubagentFrontMatter, FrontMatterError> {
            unreachable!("codex reads no subagents")
        }

        fn skill(&self, text: &str) -> Result<SkillFrontMatter, FrontMatterError> {
            self.calls.borrow_mut().push(text.to_owned());
            let mut front = SkillFrontMatter::default();
            for (index, line) in text.lines().enumerate() {
                let error = |message: &str| FrontMatterError {
                    message: message.to_owned(),
                    line: Some(index as u32 + 1),
                };
                if line.contains("!bad") {
                    return Err(error("bad yaml"));
                }
                let Some((key, value)) = line.split_once(':') else {
                    continue;
                };
                let value = Self::value(value).map_err(error)?;
                let text = |value: FakeValue| match value {
                    FakeValue::Text(text) => Ok(text),
                    FakeValue::Collection(_) => Err(error("invalid type: expected a string")),
                };
                match key {
                    "name" => front.name = Some(text(value)?),
                    "description" => front.description = Some(text(value)?),
                    "metadata" => match value {
                        FakeValue::Collection(map) if map.starts_with('{') => {}
                        FakeValue::Text(empty) if empty.is_empty() => {}
                        _ => return Err(error("invalid type: expected struct")),
                    },
                    "  short-description" => {
                        front.metadata = SkillMetadata {
                            short_description: Some(text(value)?),
                        }
                    }
                    _ => {}
                }
            }
            Ok(front)
        }

        fn is_yaml_value(&self, text: &str) -> bool {
            let closed = (text.starts_with('[') && text.ends_with(']'))
                || (text.starts_with('{') && text.ends_with('}'));
            closed && !text.contains(['@', '`'])
        }
    }

    fn format_with(
        parser: &FakeParser,
        file: &str,
        folder: &str,
        text: &str,
        mode: AgentOutputFormat,
    ) -> SkillFormat {
        CODEX_SETTINGS_CHECK.skill_format(file, folder, text, parser, mode)
    }

    fn format(text: &str) -> SkillFormat {
        format_with(&FakeParser::default(), "SKILL.md", "folder", text, JSON)
    }

    fn valid(name: &str, description: &str) -> SkillFormat {
        SkillFormat::Valid {
            name: name.to_owned(),
            description: description.to_owned(),
        }
    }

    fn invalid(code: &str, args: Value, line: Option<u32>) -> SkillFormat {
        SkillFormat::Invalid(reason(code, args, line))
    }

    #[test]
    fn the_file_name_must_be_exactly_skill_md() {
        let text = "---\nname: a\ndescription: d\n---\n";
        for file in [
            "skill.md",
            "Skill.md",
            "SKILL.MD",
            "SKILL.md.txt",
            "README.md",
        ] {
            let parser = FakeParser::default();
            assert_eq!(
                format_with(&parser, file, "folder", text, JSON),
                invalid("codex-skill-file-name", json!({"file": file}), None),
                "{file}"
            );
            assert!(parser.calls().is_empty(), "{file}");
        }
    }

    #[test]
    fn the_front_matter_is_split_the_way_codex_splits_it() {
        let none = invalid("codex-skill-no-front-matter", json!({}), None);
        for text in [
            "\u{FEFF}---\nname: a\ndescription: d\n---\n",
            "name: a\ndescription: d\n",
            "---\nname: a\ndescription: d\n",
            "---\n---\nbody",
            "",
            "body\n---\ndescription: d\n---\n",
        ] {
            let parser = FakeParser::default();
            assert_eq!(
                format_with(&parser, "SKILL.md", "folder", text, JSON),
                none,
                "{text:?}"
            );
            assert!(parser.calls().is_empty(), "{text:?}");
        }
        let parser = FakeParser::default();
        assert_eq!(
            format_with(
                &parser,
                "SKILL.md",
                "folder",
                "  ---  \r\nname: a\r\ndescription: d\r\n --- \r\nbody",
                JSON
            ),
            valid("a", "d")
        );
        assert_eq!(parser.calls(), ["name: a\ndescription: d"]);

        // A whitespace-only block goes on to parsing (and then has no description).
        let parser = FakeParser::default();
        assert_eq!(
            format_with(&parser, "SKILL.md", "folder", "---\n  \n---\n", JSON),
            invalid("codex-skill-description-empty", json!({}), None)
        );
        assert_eq!(parser.calls(), ["  "]);
    }

    #[test]
    fn a_front_matter_that_parses_is_parsed_once() {
        let parser = FakeParser::default();
        assert_eq!(
            format_with(
                &parser,
                "SKILL.md",
                "folder",
                "---\nname: a\ndescription: Build for AWS\n---\n",
                JSON
            ),
            valid("a", "Build for AWS")
        );
        assert_eq!(parser.calls().len(), 1);
    }

    #[test]
    fn a_failed_parse_is_retried_once_on_the_repaired_text() {
        let parser = FakeParser::default();
        assert_eq!(
            format_with(
                &parser,
                "SKILL.md",
                "folder",
                "---\nname: a\ndescription: Build for AWS: ECS\n---\n",
                JSON
            ),
            valid("a", "Build for AWS: ECS")
        );
        assert_eq!(
            parser.calls(),
            [
                "name: a\ndescription: Build for AWS: ECS",
                "name: a\ndescription: 'Build for AWS: ECS'",
            ]
        );
    }

    #[test]
    fn when_the_repaired_text_fails_too_the_first_error_is_reported_at_its_file_line() {
        let parser = FakeParser::default();
        assert_eq!(
            format_with(
                &parser,
                "SKILL.md",
                "folder",
                "---\nname: a: b\nx: !bad\ndescription: d\n---\n",
                JSON
            ),
            invalid(
                "codex-skill-front-matter",
                json!({"message": "mapping values"}),
                Some(2)
            )
        );
        assert_eq!(parser.calls().len(), 2);
    }

    #[test]
    fn a_failed_parse_that_the_repair_does_not_change_is_not_retried() {
        let parser = FakeParser::default();
        assert_eq!(
            format_with(
                &parser,
                "SKILL.md",
                "folder",
                "---\ndescription: d\nx: !bad\n---\n",
                JSON
            ),
            invalid(
                "codex-skill-front-matter",
                json!({"message": "bad yaml"}),
                Some(3)
            )
        );
        assert_eq!(parser.calls().len(), 1);
    }

    #[test]
    fn the_name_is_collapsed_and_falls_back_to_the_folder() {
        for (text, folder, name) in [
            (
                "---\nname:  deploy  service\ndescription: d\n---\n",
                "f",
                "deploy service",
            ),
            ("---\ndescription: d\n---\n", "my  skill", "my skill"),
            ("---\nname:    \ndescription: d\n---\n", "folder", "folder"),
            ("---\ndescription: d\n---\n", "  ", "skill"),
        ] {
            assert_eq!(
                format_with(&FakeParser::default(), "SKILL.md", folder, text, JSON),
                valid(name, "d"),
                "{text:?}"
            );
        }
    }

    #[test]
    fn the_name_may_have_64_characters_but_not_65() {
        let sixty_four = "é".repeat(64);
        assert_eq!(
            format(&format!("---\nname: {sixty_four}\ndescription: d\n---\n")),
            valid(&sixty_four, "d")
        );
        let sixty_five = "é".repeat(65);
        assert_eq!(
            format(&format!("---\nname: {sixty_five}\ndescription: d\n---\n")),
            invalid(
                "codex-skill-name-too-long",
                json!({"name": sixty_five}),
                None
            )
        );
        let folder = "f".repeat(65);
        assert_eq!(
            format_with(
                &FakeParser::default(),
                "SKILL.md",
                &folder,
                "---\ndescription: d\n---\n",
                JSON
            ),
            invalid("codex-skill-name-too-long", json!({"name": folder}), None)
        );
    }

    #[test]
    fn the_description_must_not_be_blank_and_has_no_length_cap() {
        let empty = invalid("codex-skill-description-empty", json!({}), None);
        assert_eq!(format("---\nname: a\n---\n"), empty);
        assert_eq!(format("---\nname: a\ndescription:    \n---\n"), empty);
        let long = "d".repeat(5000);
        assert_eq!(
            format(&format!("---\nname: a\ndescription: {long}\n---\n")),
            valid("a", &long)
        );
        assert_eq!(
            format("---\nname: a\ndescription:  two   words \n---\n"),
            valid("a", "two words")
        );
    }

    #[test]
    fn a_blank_short_description_does_not_make_the_skill_invalid() {
        assert_eq!(
            format("---\nname: a\ndescription: d\nmetadata:\n  short-description:    \n---\n"),
            valid("a", "d")
        );
    }

    #[test]
    fn the_terminal_mode_does_not_check_skills() {
        let parser = FakeParser::default();
        assert_eq!(
            format_with(&parser, "skill.md", "folder", "no front matter", TERMINAL),
            SkillFormat::NotChecked
        );
        assert!(parser.calls().is_empty());
    }

    /// The repair at the text level — rows from codex `parser_tests.rs` and the parent spec.
    #[test]
    fn the_repair_quotes_the_scalars_codex_quotes() {
        let cases = [
            (
                "description: Build for AWS: ECS",
                Some("description: 'Build for AWS: ECS'"),
            ),
            (
                "metadata:\n  short-description: What's included: builds and tests",
                Some("metadata:\n  short-description: 'What''s included: builds and tests'"),
            ),
            (
                "argument-hint: <duration: e.g. 7d>",
                Some("argument-hint: '<duration: e.g. 7d>'"),
            ),
            (
                "tags: [next,@supabase/ssr]",
                Some("tags: '[next,@supabase/ssr]'"),
            ),
            ("a: {x", Some("a: '{x'")),
            ("a: @scope/pkg", Some("a: '@scope/pkg'")),
            ("a: `cmd`", Some("a: '`cmd`'")),
            (
                "description: x: y # note",
                Some("description: 'x: y' # note"),
            ),
            ("description:\tx: y", Some("description:\t'x: y'")),
            ("description: x:y", None),
            ("description: plain", None),
            ("description: 'x: y'", None),
            ("description: \"x: y\"", None),
            ("description:x: y", None),
            ("description:", None),
            ("description: # only a comment: here", None),
            ("description: |\n  x: y\n  more: z", None),
            (
                "description: >\n  x: y\nname: a: b",
                Some("description: >\n  x: y\nname: 'a: b'"),
            ),
            ("- item: x", None),
            (": x: y", None),
            ("no colon here", None),
            // A flow value that is valid YAML on its own stays as it is (codex `invalid_flow_like_scalar`).
            ("metadata: {}", None),
            ("tags: [a, b]", None),
            (
                "metadata: {}\ndescription: Build: x",
                Some("metadata: {}\ndescription: 'Build: x'"),
            ),
        ];
        for (front_matter, expected) in cases {
            assert_eq!(
                repair_front_matter(front_matter, &FakeParser::default()).as_deref(),
                expected,
                "{front_matter:?}"
            );
        }
    }

    /// A valid flow value on a line the repair passes keeps its type: the repaired front matter parses (codex loads
    /// the skill) …
    #[test]
    fn a_valid_flow_mapping_survives_the_repair_of_another_line() {
        let parser = FakeParser::default();
        assert_eq!(
            format_with(
                &parser,
                "SKILL.md",
                "folder",
                "---\nname: a\nmetadata: {}\ndescription: Build: x\n---\n",
                JSON
            ),
            valid("a", "Build: x")
        );
        assert_eq!(parser.calls().len(), 2);
    }

    /// … and a valid flow sequence where a string belongs is not turned into a string (codex rejects the skill).
    #[test]
    fn a_valid_flow_sequence_for_the_name_is_not_repaired_into_a_string() {
        let parser = FakeParser::default();
        assert_eq!(
            format_with(
                &parser,
                "SKILL.md",
                "folder",
                "---\nname: [foo]\ndescription: d\n---\n",
                JSON
            ),
            invalid(
                "codex-skill-front-matter",
                json!({"message": "invalid type: expected a string"}),
                Some(2)
            )
        );
        assert_eq!(parser.calls().len(), 1);
    }

    // ── folder load set ──────────────────────────────────────────────────────────

    const SKILL_OK: &str = "---\nname: from-front\ndescription: d\n---\nbody";
    const SKILL_NO_NAME: &str = "---\ndescription: d\n---\nbody";
    const SKILL_BAD: &str = "---\nname: a\n---\nbody";

    fn slash(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }

    /// A fake folder: relative path → text (`None` = listed but unreadable). `links` are listed as unentered
    /// links after the files.
    fn load_set_of(
        files: &[(&str, Option<&str>)],
        links: &[&str],
        mode: AgentOutputFormat,
    ) -> FolderLoadSet {
        let (entries, read) = listing(files, links);
        CODEX_SETTINGS_CHECK.folder_load_set(&entries, &read, &FakeParser::default(), mode)
    }

    fn listing(
        files: &[(&str, Option<&str>)],
        links: &[&str],
    ) -> (Vec<FolderEntry>, impl Fn(&Path) -> io::Result<String>) {
        let texts: HashMap<PathBuf, Option<String>> = files
            .iter()
            .map(|(p, t)| (PathBuf::from(p), t.map(str::to_owned)))
            .collect();
        let entries = files
            .iter()
            .map(|(p, _)| FolderEntry::File(PathBuf::from(p)))
            .chain(
                links
                    .iter()
                    .map(|l| FolderEntry::UnenteredLink(PathBuf::from(l))),
            )
            .collect();
        let read = move |path: &Path| -> io::Result<String> {
            match texts.get(path) {
                Some(Some(text)) => Ok(text.clone()),
                _ => Err(io::Error::new(io::ErrorKind::InvalidData, "not utf-8")),
            }
        };
        (entries, read)
    }

    fn loaded(set: &FolderLoadSet) -> Vec<(String, FolderLoadKind)> {
        set.loads.iter().map(|l| (slash(&l.path), l.kind)).collect()
    }

    fn unsure(set: &FolderLoadSet) -> Vec<(String, Option<FolderLoadKind>, UnsureWhy)> {
        set.unsure
            .iter()
            .map(|u| (slash(&u.path), u.kind, u.why.clone()))
            .collect()
    }

    fn names(set: &FolderLoadSet) -> Vec<(String, String)> {
        set.skill_names
            .iter()
            .map(|n| (slash(&n.path), n.name.clone()))
            .collect()
    }

    fn malformed(set: &FolderLoadSet) -> Vec<(String, String)> {
        set.malformed
            .iter()
            .map(|m| (slash(&m.path), m.reason.code.clone()))
            .collect()
    }

    #[test]
    fn the_codex_json_set_is_the_instruction_file_the_two_roots_and_their_skills() {
        use FolderLoadKind::*;
        let set = load_set_of(
            &[
                ("README.md", Some("# readme")),
                (".agents/skills/alpha/SKILL.md", Some(SKILL_OK)),
                ("AGENTS.md", Some("# agents")),
                (".codex/skills/beta/SKILL.md", Some(SKILL_NO_NAME)),
                (".codex/config.toml", Some("model = 'o3'")),
                (".claude/skills/x/SKILL.md", Some(SKILL_OK)),
                ("skills/y/SKILL.md", Some(SKILL_OK)),
                (".agents/skills/alpha/notes.md", Some("notes")),
                ("CLAUDE.md", Some("# claude")),
            ],
            &[],
            JSON,
        );
        assert_eq!(
            loaded(&set),
            [
                (".agents/skills".to_owned(), SkillRoot),
                (".agents/skills/alpha/SKILL.md".to_owned(), Skill),
                ("AGENTS.md".to_owned(), Instructions),
                (".codex/skills".to_owned(), SkillRoot),
                (".codex/skills/beta/SKILL.md".to_owned(), Skill),
            ]
        );
        assert_eq!(
            names(&set),
            [
                (
                    ".agents/skills/alpha/SKILL.md".to_owned(),
                    "from-front".to_owned()
                ),
                (".codex/skills/beta/SKILL.md".to_owned(), "beta".to_owned()),
            ]
        );
        assert!(set.unsure.is_empty() && set.malformed.is_empty() && set.omitted.is_empty());
    }

    #[test]
    fn the_override_wins_over_agents_md() {
        let set = load_set_of(
            &[
                ("AGENTS.md", Some("# agents")),
                ("AGENTS.override.md", Some("# override")),
            ],
            &[],
            JSON,
        );
        assert_eq!(
            loaded(&set),
            [(
                "AGENTS.override.md".to_owned(),
                FolderLoadKind::Instructions
            )]
        );
    }

    #[test]
    fn an_empty_or_blank_override_loads_nothing_and_has_no_fallback() {
        for text in ["", " \n\t\r\n"] {
            let set = load_set_of(
                &[
                    ("AGENTS.md", Some("# agents")),
                    ("AGENTS.override.md", Some(text)),
                ],
                &[],
                JSON,
            );
            assert_eq!(set, FolderLoadSet::default(), "{text:?}");
        }
    }

    #[test]
    fn a_blank_agents_md_loads_nothing() {
        let set = load_set_of(&[("AGENTS.md", Some("  \n"))], &[], JSON);
        assert_eq!(set, FolderLoadSet::default());
    }

    #[test]
    fn instruction_file_names_follow_the_os_rule() {
        let set = load_set_of(
            &[
                ("agents.md", Some("# agents")),
                ("Agents.Override.MD", Some("# override")),
            ],
            &[],
            JSON,
        );
        let os_folds_case = same_path(Path::new("agents.md"), Path::new("AGENTS.md"));
        let expected: Vec<(String, FolderLoadKind)> = if os_folds_case {
            vec![(
                "Agents.Override.MD".to_owned(),
                FolderLoadKind::Instructions,
            )]
        } else {
            vec![]
        };
        assert_eq!(loaded(&set), expected);
    }

    #[test]
    fn an_unreadable_or_linked_instruction_file_is_unsure() {
        let set = load_set_of(&[("AGENTS.md", None)], &[], JSON);
        assert_eq!(
            unsure(&set),
            [(
                "AGENTS.md".to_owned(),
                Some(FolderLoadKind::Instructions),
                UnsureWhy::Unreadable("not utf-8".to_owned())
            )]
        );
        let set = load_set_of(
            &[("AGENTS.md", Some("# agents"))],
            &["AGENTS.override.md"],
            JSON,
        );
        assert!(set.loads.is_empty());
        assert_eq!(
            unsure(&set),
            [(
                "AGENTS.override.md".to_owned(),
                Some(FolderLoadKind::Instructions),
                UnsureWhy::Link
            )]
        );
    }

    #[test]
    fn root_folder_names_follow_the_os_rule() {
        let set = load_set_of(&[(".Agents/Skills/a/SKILL.md", Some(SKILL_OK))], &[], JSON);
        let os_folds_case = same_path(Path::new(".agents"), Path::new(".Agents"));
        assert_eq!(set.loads.len(), if os_folds_case { 2 } else { 0 });
    }

    #[test]
    fn a_linked_root_or_root_parent_is_unsure() {
        let set = load_set_of(&[], &[".agents", ".codex/skills", ".claude", "other"], JSON);
        assert_eq!(
            unsure(&set),
            [
                (
                    ".agents".to_owned(),
                    Some(FolderLoadKind::SkillRoot),
                    UnsureWhy::Link
                ),
                (
                    ".codex/skills".to_owned(),
                    Some(FolderLoadKind::SkillRoot),
                    UnsureWhy::Link
                ),
            ]
        );
        assert!(set.loads.is_empty());
    }

    #[test]
    fn a_link_under_a_root_is_unsure_where_the_walk_would_enter_it() {
        let set = load_set_of(
            &[],
            &[
                ".agents/skills/linked",
                ".agents/skills/a/b/c/d/e/linked",
                ".agents/skills/a/b/c/d/e/f/too-deep",
                ".agents/skills/.hidden-link",
                ".agents/skills/.hidden/linked",
            ],
            JSON,
        );
        assert_eq!(
            unsure(&set),
            [
                (
                    ".agents/skills/linked".to_owned(),
                    Some(FolderLoadKind::Skill),
                    UnsureWhy::Link
                ),
                (
                    ".agents/skills/a/b/c/d/e/linked".to_owned(),
                    Some(FolderLoadKind::Skill),
                    UnsureWhy::Link
                ),
            ]
        );
        assert_eq!(
            loaded(&set),
            [(".agents/skills".to_owned(), FolderLoadKind::SkillRoot)]
        );
    }

    #[test]
    fn the_walk_prunes_dot_folders_below_the_root_and_stops_at_its_depth() {
        use FolderLoadKind::*;
        let set = load_set_of(
            &[
                (".agents/skills/SKILL.md", Some(SKILL_NO_NAME)),
                (".agents/skills/.hidden/SKILL.md", Some(SKILL_OK)),
                (".agents/skills/a/.git/b/SKILL.md", Some(SKILL_OK)),
                (".agents/skills/a/b/c/d/e/f/SKILL.md", Some(SKILL_OK)),
                (".agents/skills/a/b/c/d/e/f/g/SKILL.md", Some(SKILL_OK)),
                (".agents/skills/x/skill.md", Some(SKILL_OK)),
                (".agents/skills/y/Skill.MD", Some(SKILL_OK)),
            ],
            &[],
            JSON,
        );
        assert_eq!(
            loaded(&set),
            [
                (".agents/skills".to_owned(), SkillRoot),
                (".agents/skills/SKILL.md".to_owned(), Skill),
                (".agents/skills/a/b/c/d/e/f/SKILL.md".to_owned(), Skill),
            ]
        );
        assert_eq!(
            names(&set),
            [
                (".agents/skills/SKILL.md".to_owned(), "skills".to_owned()),
                (
                    ".agents/skills/a/b/c/d/e/f/SKILL.md".to_owned(),
                    "from-front".to_owned()
                ),
            ]
        );
        assert!(set.unsure.is_empty() && set.malformed.is_empty());
    }

    #[test]
    fn a_malformed_skill_is_marked_not_unsure_and_not_loaded() {
        use FolderLoadKind::*;
        let set = load_set_of(
            &[
                (".agents/skills/good/SKILL.md", Some(SKILL_OK)),
                (".agents/skills/bad/SKILL.md", Some(SKILL_BAD)),
                (".agents/skills/gone/SKILL.md", None),
            ],
            &[],
            JSON,
        );
        assert_eq!(
            loaded(&set),
            [
                (".agents/skills".to_owned(), SkillRoot),
                (".agents/skills/good/SKILL.md".to_owned(), Skill),
            ]
        );
        assert_eq!(
            malformed(&set),
            [(
                ".agents/skills/bad/SKILL.md".to_owned(),
                "codex-skill-description-empty".to_owned()
            )]
        );
        assert_eq!(
            unsure(&set),
            [(
                ".agents/skills/gone/SKILL.md".to_owned(),
                Some(Skill),
                UnsureWhy::Unreadable("not utf-8".to_owned())
            )]
        );
    }

    #[test]
    fn skill_names_follow_the_loads_order() {
        let set = load_set_of(
            &[
                (".codex/skills/z/SKILL.md", Some(SKILL_NO_NAME)),
                (".agents/skills/bad/SKILL.md", Some(SKILL_BAD)),
                (".agents/skills/a/SKILL.md", Some(SKILL_OK)),
                (".codex/skills/m/SKILL.md", Some(SKILL_NO_NAME)),
            ],
            &[],
            JSON,
        );
        let skill_rows: Vec<String> = set
            .loads
            .iter()
            .filter(|l| l.kind == FolderLoadKind::Skill)
            .map(|l| slash(&l.path))
            .collect();
        let name_rows: Vec<String> = set.skill_names.iter().map(|n| slash(&n.path)).collect();
        assert_eq!(skill_rows, name_rows);
        assert_eq!(
            names(&set),
            [
                (".codex/skills/z/SKILL.md".to_owned(), "z".to_owned()),
                (
                    ".agents/skills/a/SKILL.md".to_owned(),
                    "from-front".to_owned()
                ),
                (".codex/skills/m/SKILL.md".to_owned(), "m".to_owned()),
            ]
        );
    }

    /// codex reads a skill folder's name with `to_str` — a name that is not UTF-8 gives the fallback `skill`.
    #[test]
    fn a_folder_name_that_is_not_utf8_falls_back_to_skill() {
        #[cfg(windows)]
        let folder = {
            use std::os::windows::ffi::OsStringExt;
            // A lone surrogate is a valid Windows file name but not valid UTF-8.
            OsString::from_wide(&[u16::from(b'a'), 0xD800])
        };
        #[cfg(unix)]
        let folder = {
            use std::os::unix::ffi::OsStrExt;
            OsStr::from_bytes(b"a\xFF").to_owned()
        };
        assert!(folder.to_str().is_none(), "the sample must not be UTF-8");
        let path = Path::new(".agents")
            .join("skills")
            .join(&folder)
            .join("SKILL.md");
        let entries = [FolderEntry::File(path.clone())];
        let read = |_: &Path| -> io::Result<String> { Ok(SKILL_NO_NAME.to_owned()) };
        let set =
            CODEX_SETTINGS_CHECK.folder_load_set(&entries, &read, &FakeParser::default(), JSON);
        assert_eq!(
            set.skill_names,
            [FolderSkillName {
                path,
                name: "skill".to_owned(),
            }]
        );
    }

    #[test]
    fn the_terminal_set_is_only_the_json_loads_as_omitted() {
        let files = [
            ("AGENTS.md", Some("# agents")),
            (".agents/skills/a/SKILL.md", Some(SKILL_OK)),
            (".agents/skills/bad/SKILL.md", Some(SKILL_BAD)),
            (".codex/skills/b/SKILL.md", Some(SKILL_NO_NAME)),
        ];
        let links = [".agents/skills/linked"];
        let json = load_set_of(&files, &links, JSON);
        let terminal = load_set_of(&files, &links, TERMINAL);
        assert_eq!(json.loads.len(), 5);
        assert_eq!(
            terminal,
            FolderLoadSet {
                omitted: json.loads,
                ..FolderLoadSet::default()
            }
        );
    }

    /// ★JSON carries the instruction file and the skill roots, the terminal mode nothing★ — the skills under a root
    /// stay in `loads` only (codex finds them through the root).
    // ADR-0341
    #[test]
    fn json_carries_the_instruction_file_and_the_roots_and_the_terminal_mode_nothing() {
        use FolderLoadKind::*;
        let files = [
            ("README.md", Some("# readme")),
            (".agents/skills/a/SKILL.md", Some(SKILL_OK)),
            ("AGENTS.md", Some("# agents")),
            (".codex/skills/b/SKILL.md", Some(SKILL_NO_NAME)),
        ];
        let json = load_set_of(&files, &[], JSON);
        let carried: Vec<(String, FolderLoadKind)> = json
            .carried
            .iter()
            .map(|row| (slash(&row.path), row.kind))
            .collect();
        assert_eq!(
            carried,
            [
                (".agents/skills".to_owned(), SkillRoot),
                ("AGENTS.md".to_owned(), Instructions),
                (".codex/skills".to_owned(), SkillRoot),
            ]
        );
        assert!(json.carried.iter().all(|row| json.loads.contains(row)));

        let terminal = load_set_of(&files, &[], TERMINAL);
        assert!(terminal.carried.is_empty());
        assert_eq!(terminal.omitted, json.loads);

        let only_skills_root = load_set_of(&[("skills/y/SKILL.md", Some(SKILL_OK))], &[], JSON);
        assert!(only_skills_root.carried.is_empty());
    }

    #[test]
    fn root_skills_walks_one_root_like_a_folder_entry_root() {
        let files = [
            ("SKILL.md", Some(SKILL_NO_NAME)),
            ("a/SKILL.md", Some(SKILL_OK)),
            ("bad/SKILL.md", Some(SKILL_BAD)),
            (".hidden/SKILL.md", Some(SKILL_OK)),
            ("a/b/c/d/e/f/g/SKILL.md", Some(SKILL_OK)),
            ("gone/SKILL.md", None),
        ];
        let links = ["linked"];
        let (entries, read) = listing(&files, &links);
        let set = CODEX_SETTINGS_CHECK.root_skills(&entries, &read, &FakeParser::default());

        let prefixed: Vec<(&str, Option<&str>)> = files.to_vec();
        let in_folder: Vec<(String, Option<&str>)> = prefixed
            .iter()
            .map(|(p, t)| (format!(".codex/skills/{p}"), *t))
            .collect();
        let in_folder: Vec<(&str, Option<&str>)> =
            in_folder.iter().map(|(p, t)| (p.as_str(), *t)).collect();
        let folder = load_set_of(&in_folder, &[".codex/skills/linked"], JSON);
        let strip = |rows: Vec<(String, String)>| -> Vec<(String, String)> {
            rows.into_iter()
                .map(|(p, n)| (p.trim_start_matches(".codex/skills/").to_owned(), n))
                .collect()
        };

        assert_eq!(
            loaded(&set),
            [
                ("SKILL.md".to_owned(), FolderLoadKind::Skill),
                ("a/SKILL.md".to_owned(), FolderLoadKind::Skill),
            ]
        );
        assert_eq!(strip(names(&set)), strip(names(&folder)));
        assert_eq!(strip(malformed(&set)), strip(malformed(&folder)));
        assert_eq!(
            unsure(&set),
            [
                (
                    "gone/SKILL.md".to_owned(),
                    Some(FolderLoadKind::Skill),
                    UnsureWhy::Unreadable("not utf-8".to_owned())
                ),
                (
                    "linked".to_owned(),
                    Some(FolderLoadKind::Skill),
                    UnsureWhy::Link
                ),
            ]
        );
        assert!(set.omitted.is_empty());
        assert!(set.carried.is_empty());
    }

    // ── codex's own skill roots ──────────────────────────────────────────────────

    #[test]
    fn native_skill_roots_follow_codex_home_and_the_project_walk() {
        let home = Path::new("C:/Users/u");
        let cwd = Path::new("C:/work/repo/sub");
        let env_with = |value: Option<&'static str>| {
            move |key: &str| -> Option<String> {
                (key == "CODEX_HOME")
                    .then_some(value)
                    .flatten()
                    .map(str::to_owned)
            }
        };
        for (value, codex_home) in [
            (Some("D:/codex-home"), PathBuf::from("D:/codex-home")),
            (None, home.join(".codex")),
            (Some(""), home.join(".codex")),
        ] {
            let env = env_with(value);
            let places = NativePlaces {
                home,
                cwd,
                env: &env,
            };
            assert_eq!(
                CODEX_SETTINGS_CHECK.native_skill_roots(&places, JSON),
                NativeSkillRoots {
                    fixed: vec![
                        codex_home.join("skills"),
                        home.join(".agents").join("skills")
                    ],
                    per_folder: vec![
                        Path::new(".agents").join("skills"),
                        Path::new(".codex").join("skills"),
                    ],
                    project_markers: vec![".git".to_owned()],
                },
                "{value:?}"
            );
            assert_eq!(
                CODEX_SETTINGS_CHECK.native_skill_roots(&places, TERMINAL),
                NativeSkillRoots::default()
            );
        }
    }

    #[test]
    fn every_mode_answers_the_command_line_methods_through_cmdline() {
        for mode in MODES {
            let line = CommandLine {
                mode,
                cwd: Path::new("C:/w"),
                extra_args: &[],
                config_args: &[],
                instructions: None,
            };
            assert_eq!(CODEX_SETTINGS_CHECK.command_line_fits(&line), Ok(()));
        }
        let too_long = ["-c".to_owned(), "x".repeat(8191)];
        let line = CommandLine {
            mode: TERMINAL,
            cwd: Path::new("C:/w"),
            extra_args: &[],
            config_args: &too_long,
            instructions: Some("don't"),
        };
        assert_eq!(
            CODEX_SETTINGS_CHECK.command_line_fits(&line),
            Err(reason(
                "codex-priming-unsafe-char",
                json!({"char": "'"}),
                None
            ))
        );
        let line = CommandLine {
            instructions: None,
            ..line
        };
        assert!(matches!(
            CODEX_SETTINGS_CHECK.command_line_fits(&line),
            Err(Reason { code, .. }) if code == "command-line-too-long"
        ));
        assert_eq!(
            CODEX_SETTINGS_CHECK.command_line_args(&json!(["model=o3"])),
            Ok(vec!["-c".to_owned(), "model=o3".to_owned()])
        );
        assert!(matches!(
            CODEX_SETTINGS_CHECK.command_line_args(&json!([r#"model="o3"#])),
            Err(Reason { code, .. }) if code == "codex-config-unsafe-char"
        ));
    }
}
