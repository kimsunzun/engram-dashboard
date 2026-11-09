//! Backend settings-check port — the settings knowledge that differs per backend (schema source · CLI version read ·
//! raw text → JSON · merge · unknown-key policy · what a mode loads · folder-entry path guard · folder load set), in
//! one trait. The registry is [`super::settings_checks`] / [`super::settings_check_for`].
//!
//! ★Pure — an implementation never touches the disk, the network or a process★: the schema is fetched by the caller
//!   (the port only names the address), the CLI version is read by the caller running [`VersionProbe`], and a folder's
//!   file text and its YAML front matter reach the port only through the reader closure and the [`FrontMatterParser`]
//!   the caller passes (agent has no YAML dependency — the front-matter types are defined here and the caller only
//!   deserializes into them).
//!
//! Callers reach it without an agent (checking a settings asset, previewing a folder), so it is not an
//! [`super::AgentBackend`] method — the same reason as [`super::usage_probes`].
// ADR-0334
// ADR-0004

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::launch::Reason;
use crate::profile::AgentOutputFormat;

/// One backend's settings knowledge. Implementations are stateless singletons.
pub trait SettingsCheck: Send + Sync {
    /// The backend word this check belongs to — the same spelling as [`crate::launch::backend_word`] (lowercase).
    fn backend(&self) -> &'static str;

    /// Where the settings schema for `cli_version` is fetched from and when to fetch it again. The port never fetches.
    fn schema_source(&self, cli_version: &str) -> SchemaSource;

    /// The command whose output [`SettingsCheck::parse_version`] reads. The caller runs it; the port never spawns.
    fn version_probe(&self) -> VersionProbe;

    /// The CLI version in the probe's stdout. `None` = the output does not carry a version this port recognizes.
    fn parse_version(&self, stdout: &str) -> Option<String>;

    /// One raw settings value of this backend (a preset's raw text for this backend) as the JSON value the schema
    /// checks. It need not be an object — a backend may hand a non-object through unchanged for the schema to
    /// reject. `Err` = the raw value cannot be turned into JSON by this backend's rules.
    fn raw_to_json(&self, raw: &Value) -> Result<Value, Reason>;

    /// The JSON values of the attached presets, in attach order, merged into the one value the backend loads.
    /// `None` = this backend loads each preset's value on its own: there is no merged value and nothing merged to
    /// check — the caller checks only the single values. `Some` = check the merged value too.
    fn merge(&self, layers: &[&Value]) -> Option<Value>;

    /// What a key the schema does not know becomes.
    fn unknown_keys(&self) -> UnknownKeys;

    /// What an agent of this backend in `mode` loads from its launch materials.
    fn loads(&self, mode: AgentOutputFormat) -> ModeLoads;

    /// Whether this backend accepts a skill named `name` in `mode`. `false` = the name is an error for this backend.
    fn accepts_skill_name(&self, name: &str, mode: AgentOutputFormat) -> bool;

    /// Whether a folder entry at `folder` (absolute) blocks this backend's agent (problem ②) because the path cannot
    /// be passed on its command line. A backend that never puts folder paths on its command line returns `false`.
    fn folder_entry_blocks(&self, folder: &Path) -> bool;

    /// What this backend loads by itself from a folder entry, predicted from a listing of that folder.
    ///
    /// - `entries` = the folder's listing ([`FolderEntry`]). Order is kept in the result.
    /// - `read` = the text of one listed file (the same relative path).
    /// - `parser` = parses front-matter text (without the `---` lines) into the types below.
    ///
    /// The prediction assumes the backend's default loading conditions: an agent's pass-through arguments can make
    /// the backend load less than this.
    fn folder_load_set(
        &self,
        entries: &[FolderEntry],
        read: &dyn Fn(&Path) -> std::io::Result<String>,
        parser: &dyn FrontMatterParser,
        mode: AgentOutputFormat,
    ) -> FolderLoadSet;
}

/// Address and refetch rule of one backend's settings schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaSource {
    pub url: String,
    pub refetch: SchemaRefetch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaRefetch {
    /// The address serves one latest copy whatever the CLI version. When the installed CLI version differs from the
    /// one last checked, fetch it again with a conditional request.
    LatestOnCliVersionChange,
}

/// A command to run once to read a CLI version — `(program, args)` already shaped for the OS (a CLI shim is wrapped),
/// plus env pairs to set on top of the inherited environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionProbe {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownKeys {
    /// Not reported at any depth.
    Ignore,
    /// Reported as a warning; never blocks a launch.
    Warn,
}

/// What one backend · mode loads from the launch materials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeLoads {
    pub instructions: InstructionLoad,
    pub skills: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstructionLoad {
    /// Every attached instruction.
    All,
    /// Only the priming of the built-in default preset.
    BuiltinPrimingOnly,
}

/// One entry of a folder listing. The path is relative to the folder and has normal components only (no `.`, `..`,
/// root or prefix); an entry with any other component is ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderEntry {
    File(PathBuf),
    /// A link (file or directory) the listing did not follow — what it points to is not listed.
    UnenteredLink(PathBuf),
}

/// Parses front-matter text into the agent-defined types. The caller owns the YAML parser.
pub trait FrontMatterParser {
    /// The front matter of a file whose `paths` field the port reads (a claude rule or skill file).
    fn paths(&self, text: &str) -> Result<PathsFrontMatter, FrontMatterError>;
    fn subagent(&self, text: &str) -> Result<SubagentFrontMatter, FrontMatterError>;
}

/// The `paths` field of a claude rule or skill file's front matter.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct PathsFrontMatter {
    /// `None` = no `paths` key (or `paths: null`).
    #[serde(default)]
    pub paths: Option<Value>,
}

/// The fields of a claude subagent file's (`.claude/agents/…`) front matter the folder load set reads. `name` and
/// `description` are kept as untyped values: a value of the wrong type is a reason not to load, not a parse
/// failure. The three `has_*` fields say whether that top-level key is present at all (any value, `null` included);
/// `other` holds every other top-level key with its value.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct SubagentFrontMatter {
    #[serde(default)]
    pub name: Option<Value>,
    #[serde(default)]
    pub description: Option<Value>,
    #[serde(default, rename = "hooks", deserialize_with = "present")]
    pub has_hooks: bool,
    #[serde(default, rename = "PreToolUse", deserialize_with = "present")]
    pub has_pre_tool_use: bool,
    #[serde(default, rename = "PermissionRequest", deserialize_with = "present")]
    pub has_permission_request: bool,
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

/// A key that is present — its value is not looked at. An absent key never reaches here (`default` = `false`).
fn present<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    serde::de::IgnoredAny::deserialize(deserializer).map(|_| true)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontMatterError {
    pub message: String,
    /// From the parser: the line within the front-matter text it was given, from 1. In [`UnsureWhy::FrontMatter`]
    /// the port has turned it into the line of the file. `None` = the parser gave no line.
    pub line: Option<u32>,
}

/// The result of [`SettingsCheck::folder_load_set`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FolderLoadSet {
    /// Files the backend loads, in listing order.
    pub loads: Vec<FolderLoad>,
    /// Entries the backend may load from, whose answer the port could not decide.
    pub unsure: Vec<FolderUnsure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderLoad {
    /// Relative to the folder, as listed.
    pub path: PathBuf,
    pub kind: FolderLoadKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderLoadKind {
    Instructions,
    Rule,
    Skill,
    Agent,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderUnsure {
    /// Relative to the folder, as listed.
    pub path: PathBuf,
    /// `None` = the entry could hold items of several kinds (e.g. a link in place of a whole config folder).
    pub kind: Option<FolderLoadKind>,
    pub why: UnsureWhy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsureWhy {
    /// `read` failed — the message of that error.
    Unreadable(String),
    /// The front matter did not parse with the caller's parser. Its `line` is the line of the file, from 1.
    FrontMatter(FrontMatterError),
    /// An unfollowed link where the backend loads files — the backend follows links, the listing did not.
    Link,
    /// The front matter holds `---` before its closing line — the backend may close the front matter there.
    DashesInFrontMatter,
    /// A `paths` value whose effect is not predicted (a rule's value that is not a plain list of path globs, or any
    /// `paths` on a skill).
    UnclearPaths,
    /// The file's content (after its front matter) is blank, or may be blank once HTML comments are stripped — the
    /// backend may skip it.
    EmptyBody,
    /// A subagent front matter with a hook key (`hooks`, `PreToolUse`, `PermissionRequest`) — the backend may
    /// refuse the file. Nested hook keys count too (see the backend's rule).
    HookKeys,
    /// A command folder that holds a `skill.md` — the backend keeps only that file of the folder.
    SkillFileInCommandFolder,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn paths_front_matter_reads_paths_and_takes_null_as_absent() {
        for (value, expected) in [
            (json!({}), None),
            (json!({"paths": null}), None),
            (json!({"paths": "src/**"}), Some(json!("src/**"))),
            (
                json!({"paths": [1, "a"], "other": true}),
                Some(json!([1, "a"])),
            ),
        ] {
            let parsed: PathsFrontMatter = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(parsed.paths, expected, "{value}");
        }
    }

    #[test]
    fn subagent_front_matter_keeps_values_untyped_and_marks_present_hook_keys() {
        let parsed: SubagentFrontMatter = serde_json::from_value(json!({
            "name": 7,
            "description": ["d"],
            "hooks": null,
            "PermissionRequest": {"x": 1},
            "tools": {"PreToolUse": 1},
            "has_pre_tool_use": true,
        }))
        .unwrap();
        assert_eq!(parsed.name, Some(json!(7)));
        assert_eq!(parsed.description, Some(json!(["d"])));
        assert!(parsed.has_hooks, "a null value is still a present key");
        assert!(
            !parsed.has_pre_tool_use,
            "only the key `PreToolUse` sets it"
        );
        assert!(parsed.has_permission_request);
        assert_eq!(
            Value::Object(parsed.other),
            json!({"tools": {"PreToolUse": 1}, "has_pre_tool_use": true})
        );

        let parsed: SubagentFrontMatter =
            serde_json::from_value(json!({"PreToolUse": null})).unwrap();
        assert!(parsed.has_pre_tool_use);
        assert!(!parsed.has_hooks && !parsed.has_permission_request);

        let parsed: SubagentFrontMatter = serde_json::from_value(json!({})).unwrap();
        assert_eq!(parsed, SubagentFrontMatter::default());
    }
}
