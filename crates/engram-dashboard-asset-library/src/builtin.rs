//! The built-in assets: `engram/…` → files of the same shape as the library under the distribution's `builtin\`
//! folder (a constructor value), read once when the service is made.
//!
//! - The id set is closed ([`BuiltinId`] — B23); file names per [`file_of`] (B14).
//! - **A built-in that cannot be read never blocks** (ADR-0361 decision 2): with no folder, or a file that cannot be
//!   read, declares no `kind` or does not parse, that built-in is an empty in-memory stand-in with no file — the
//!   built-in preset lists nothing, the priming loads nothing (`include_priming` stays false), the settings carry no
//!   field — a warning is logged, and the stand-in keeps why, which a resolution that reaches it tells as a
//!   `builtin-unavailable` notice.
// ADR-0315
// ADR-0327
// ADR-0361
// TRD S25 step-7 §3-7

use std::path::{Path, PathBuf};

use engram_dashboard_agent::launch::escape_controls;

use crate::backends::Backends;
use crate::catalog::{yaml_item, ItemContent, ItemKind};
use crate::excerpt::excerpt;
use crate::format::{PresetDoc, SettingsDoc};
use crate::launch::sha256_hex;
use crate::path::BuiltinId;
use crate::resolve::ItemView;
use crate::validate::SettingsVerdict;

/// The file of `id`, relative to the built-in folder.
// TRD S25 step-7 B14
pub fn file_of(id: BuiltinId) -> &'static str {
    match id {
        BuiltinId::DefaultPreset => "presets/default.yaml",
        BuiltinId::Priming => "instructions/priming.md",
        BuiltinId::DefaultSettings => "settings/default.yaml",
    }
}

/// Every built-in, read from its file or stood in for (module header).
pub struct Builtins {
    default_preset: Entry,
    priming: Entry,
    default_settings: Entry,
    /// Why the built-in folder cannot be read: there is none, or no built-in file in it could be read (every one
    /// stands in for a read failure, not for its content); `None` = it can.
    folder_unreadable: Option<String>,
}

struct Entry {
    kind: ItemKind,
    content: ItemContent,
    /// `Err` = the stand-in, with why.
    file: Result<PathBuf, String>,
    /// The file's text as read (not UTF-8 → U+FFFD) — empty for a stand-in.
    text: String,
    /// It stands in because its file could not be read (a missing folder or file, an access error) — not for what the
    /// file holds.
    unread: bool,
}

impl Builtins {
    /// Reads every built-in under `dir` (`None` = there is no built-in folder). Preset tags are read through
    /// `backends`, as in a library preset.
    pub fn new(dir: Option<&Path>, backends: &dyn Backends) -> Self {
        if dir.is_none() {
            tracing::warn!("no built-in folder; all built-ins start as stand-ins");
        }
        let entry = |id| match dir.map(|dir| dir.join(file_of(id))) {
            None => stand_in(id, NO_FOLDER.to_owned(), true),
            Some(path) => match load(id, &path, backends) {
                Ok((kind, content, text)) => Entry {
                    kind,
                    content,
                    file: Ok(path),
                    text,
                    unread: false,
                },
                Err(failed) => {
                    let (why, unread) = match failed {
                        LoadError::Read(why) => (why, true),
                        LoadError::Content(why) => (why, false),
                    };
                    tracing::warn!(
                        builtin = id.id(),
                        path = ?path,
                        "built-in load failed; using a stand-in: {}",
                        escape_controls(&why)
                    );
                    stand_in(id, why, unread)
                }
            },
        };
        let (default_preset, priming, default_settings) = (
            entry(BuiltinId::DefaultPreset),
            entry(BuiltinId::Priming),
            entry(BuiltinId::DefaultSettings),
        );
        let folder_unreadable = match dir {
            None => Some(NO_FOLDER.to_owned()),
            Some(_)
                if [&default_preset, &priming, &default_settings]
                    .iter()
                    .all(|entry| entry.unread) =>
            {
                Some(FOLDER_UNREADABLE.to_owned())
            }
            Some(_) => None,
        };
        Builtins {
            default_preset,
            priming,
            default_settings,
            folder_unreadable,
        }
    }

    /// Why the built-in folder cannot be read — there is none ([`NO_FOLDER`]), or not one built-in file in it could
    /// be read ([`FOLDER_UNREADABLE`]) — as read once when the service was made; `None` = it can (a built-in may still
    /// stand in on its own).
    // ADR-0400 decision 20
    pub(crate) fn folder_unreadable(&self) -> Option<&str> {
        self.folder_unreadable.as_deref()
    }

    /// The priming's text as read when the service was made — what the merged instructions start with when a
    /// resolution includes the priming. Empty for its stand-in (a resolution never includes a stand-in).
    pub fn priming_text(&self) -> &str {
        &self.priming.text
    }

    pub fn view(&self, id: BuiltinId) -> ItemView<'_> {
        let entry = match id {
            BuiltinId::DefaultPreset => &self.default_preset,
            BuiltinId::Priming => &self.priming,
            BuiltinId::DefaultSettings => &self.default_settings,
        };
        ItemView {
            kind: &entry.kind,
            content: &entry.content,
            file: entry.file.as_ref().ok().cloned(),
            stand_in: entry.file.as_ref().err().map(String::as_str),
        }
    }
}

/// Why every built-in stands in when there is no built-in folder — and why the built-in mount cannot be read.
pub(crate) const NO_FOLDER: &str = "no built-in folder";

/// Why the built-in mount cannot be read when there is a built-in folder but not one of its files could be read.
pub(crate) const FOLDER_UNREADABLE: &str = "the built-in folder cannot be read";

/// Why a built-in file could not be used: it could not be read at all, or what it holds is not that built-in.
enum LoadError {
    Read(String),
    Content(String),
}

/// Why a built-in YAML file that declares no `kind` stands in.
const NO_KIND: &str = "no `kind`";

/// The built-in `id` read from `path` with the file's text, or why it cannot be used — a file that does not parse
/// gives its reader's code and message.
fn load(
    id: BuiltinId,
    path: &Path,
    backends: &dyn Backends,
) -> Result<(ItemKind, ItemContent, String), LoadError> {
    let bytes = std::fs::read(path).map_err(|e| LoadError::Read(e.to_string()))?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    match id {
        BuiltinId::Priming => {
            let content = ItemContent::Instruction {
                excerpt: excerpt(&text),
            };
            Ok((ItemKind::Instruction, content, text))
        }
        BuiltinId::DefaultPreset | BuiltinId::DefaultSettings => {
            match yaml_item(&bytes, backends) {
                None => Err(LoadError::Content(NO_KIND.to_owned())),
                Some((_, ItemContent::Broken { reason })) => {
                    // `ParseError::to_reason` always writes `message` as a string.
                    let message = reason.args["message"].as_str().unwrap_or_default();
                    Err(LoadError::Content(format!("{}: {message}", reason.code)))
                }
                Some((kind, content)) => Ok((kind, content, text)),
            }
        }
    }
}

fn stand_in(id: BuiltinId, why: String, unread: bool) -> Entry {
    let (kind, content) = match id {
        BuiltinId::DefaultPreset => (
            ItemKind::Preset,
            ItemContent::Preset {
                doc: PresetDoc {
                    version: 1,
                    description: None,
                    presets: Vec::new(),
                    instructions: Vec::new(),
                    skills: Vec::new(),
                    hooks: Vec::new(),
                    settings: Vec::new(),
                    folders: Vec::new(),
                    unmodeled: Vec::new(),
                    line_problems: Vec::new(),
                },
                rev: sha256_hex(&[]),
            },
        ),
        BuiltinId::Priming => (
            ItemKind::Instruction,
            ItemContent::Instruction {
                excerpt: String::new(),
            },
        ),
        BuiltinId::DefaultSettings => (
            ItemKind::Settings,
            ItemContent::Settings {
                doc: SettingsDoc {
                    version: 1,
                    description: None,
                    raw: Vec::new(),
                },
                verdict: SettingsVerdict::default(),
            },
        ),
    };
    Entry {
        kind,
        content,
        file: Err(why),
        text: String::new(),
        unread,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;

    use engram_dashboard_agent::types::MCP_SERVER_NAME;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::backends::AgentBackends;
    use crate::path::{AssetRef, PresetItem};

    pub(crate) const DEFAULT_PRESET: &str = "kind: preset\nversion: 1\ndescription: the default\ninstructions:\n  - engram/priming\nsettings:\n  - engram/settings/default\n";
    pub(crate) const PRIMING: &str = "# Engram\n\nYou run inside Engram.\n";
    pub(crate) const DEFAULT_SETTINGS: &str =
        "kind: settings\nversion: 1\nclaude:\n  env:\n    DISABLE_AUTOUPDATER: \"1\"\n";

    /// A built-in folder holding the given files (relative path, text).
    pub(crate) fn folder(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (rel, text) in files {
            let path = dir.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        dir
    }

    /// A built-in folder holding all three files.
    pub(crate) fn full() -> TempDir {
        folder(&[
            (file_of(BuiltinId::DefaultPreset), DEFAULT_PRESET),
            (file_of(BuiltinId::Priming), PRIMING),
            (file_of(BuiltinId::DefaultSettings), DEFAULT_SETTINGS),
        ])
    }

    #[test]
    fn each_builtin_is_read_from_its_own_file_under_the_folder() {
        let dir = full();
        let builtins = Builtins::new(Some(dir.path()), &AgentBackends);
        for id in BuiltinId::ALL {
            assert_eq!(builtins.view(id).stand_in, None, "{id:?}");
        }

        let preset = builtins.view(BuiltinId::DefaultPreset);
        assert_eq!(*preset.kind, ItemKind::Preset);
        assert_eq!(
            preset.file.as_deref(),
            Some(dir.path().join("presets/default.yaml").as_path())
        );
        let ItemContent::Preset { doc, .. } = preset.content else {
            panic!("{:?}", preset.content)
        };
        assert_eq!(doc.description.as_deref(), Some("the default"));
        assert_eq!(
            doc.instructions[0].target,
            AssetRef::BuiltIn(BuiltinId::Priming)
        );
        assert_eq!(
            doc.settings[0].target,
            AssetRef::BuiltIn(BuiltinId::DefaultSettings)
        );

        let priming = builtins.view(BuiltinId::Priming);
        assert_eq!(*priming.kind, ItemKind::Instruction);
        assert_eq!(
            priming.file.as_deref(),
            Some(dir.path().join("instructions/priming.md").as_path())
        );
        let ItemContent::Instruction { excerpt: shown } = priming.content else {
            panic!("{:?}", priming.content)
        };
        assert_eq!(*shown, excerpt(PRIMING));

        let settings = builtins.view(BuiltinId::DefaultSettings);
        assert_eq!(
            settings.file.as_deref(),
            Some(dir.path().join("settings/default.yaml").as_path())
        );
        let ItemContent::Settings { doc, verdict } = settings.content else {
            panic!("{:?}", settings.content)
        };
        assert_eq!(doc.raw[0].name, "claude");
        assert_eq!(
            doc.raw[0].value,
            json!({ "env": { "DISABLE_AUTOUPDATER": "1" } })
        );
        assert!(verdict.problems.is_empty());
    }

    /// The repo's `builtin\` source folder, which `scripts/build-release.ps1` copies into the release — none of the
    /// three may be a stand-in.
    // TRD S25 step-17
    #[test]
    fn the_repo_builtin_folder_reads_without_a_stand_in() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../builtin");
        let builtins = Builtins::new(Some(&dir), &AgentBackends);
        for id in BuiltinId::ALL {
            assert_eq!(builtins.view(id).stand_in, None, "{id:?}");
        }

        let preset = builtins.view(BuiltinId::DefaultPreset);
        let ItemContent::Preset { doc, .. } = preset.content else {
            panic!("{:?}", preset.content)
        };
        assert!(doc.description.is_some());
        let targets = |items: &[PresetItem]| -> Vec<AssetRef> {
            items.iter().map(|item| item.target.clone()).collect()
        };
        assert_eq!(
            targets(&doc.instructions),
            [AssetRef::BuiltIn(BuiltinId::Priming)]
        );
        assert_eq!(
            targets(&doc.settings),
            [AssetRef::BuiltIn(BuiltinId::DefaultSettings)]
        );
        assert!(doc.presets.is_empty() && doc.skills.is_empty() && doc.folders.is_empty());
        assert!(doc.unmodeled.is_empty() && doc.line_problems.is_empty());

        let priming = fs::read_to_string(dir.join(file_of(BuiltinId::Priming))).unwrap();
        assert!(!priming.trim().is_empty());
        assert_eq!(builtins.priming_text(), priming);

        let settings = builtins.view(BuiltinId::DefaultSettings);
        let ItemContent::Settings { doc, verdict } = settings.content else {
            panic!("{:?}", settings.content)
        };
        assert_eq!(doc.raw.len(), 1, "{:?}", doc.raw);
        assert_eq!(doc.raw[0].name, "claude");
        assert_eq!(
            doc.raw[0].value,
            json!({
                "env": { "DISABLE_AUTOUPDATER": "1" },
                "allowedMcpServers": [{ "serverName": MCP_SERVER_NAME }],
            })
        );
        assert!(verdict.problems.is_empty());
    }

    /// The shipped priming, merged the way a launch merges it, passes the codex terminal command-line check (its
    /// characters and length) — on real launch materials a refusal there blocks every codex terminal launch, where the
    /// old priming path only warned. The built-in settings carry no codex field, so the priming is the line's only
    /// instructions and nothing else is added for a plain agent.
    // ADR-0353
    // ADR-0361
    #[test]
    fn the_shipped_priming_fits_the_codex_terminal_command_line() {
        use engram_dashboard_agent::backend::settings_check::CommandLine;
        use engram_dashboard_agent::profile::AgentOutputFormat;

        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../builtin");
        let builtins = Builtins::new(Some(&dir), &AgentBackends);
        assert_eq!(builtins.view(BuiltinId::Priming).stand_in, None);
        let merged = crate::launch::merge_instructions(Some(builtins.priming_text()), &[]);
        let check = AgentBackends
            .for_word("codex")
            .expect("codex has a settings check");
        let line = CommandLine {
            mode: AgentOutputFormat::Terminal,
            cwd: Path::new(r"C:\Users\someone\work\project"),
            extra_args: &[],
            config_args: &[],
            config_origins: &[],
            hooks: &[],
            instructions: Some(&merged),
        };
        assert_eq!(check.command_line_fits(&line), Ok(vec![]));
        // ADR-0387: the count form of the launch note fits after the shipped priming, so a codex terminal agent is
        //   told how many items were left out even when no name can go on its line.
        for n in [1, 10_000] {
            let noted = crate::launch::with_note(&merged, &crate::launch::count_note(n));
            let line = CommandLine {
                instructions: Some(&noted),
                ..line
            };
            assert_eq!(check.command_line_fits(&line), Ok(vec![]), "{n}");
        }
    }

    /// The release manifest (`$ExpectedBuiltins` in `scripts/build-release.ps1`) lists exactly the files of
    /// [`BuiltinId::ALL`] — the direction no other test sees: the test above reads the repo folder through the same
    /// [`file_of`], so a built-in added to the code and the repo but not to the manifest stays green there, and the
    /// release then ships without it and that built-in silently stands in (ADR-0361).
    // TRD S25 step-17
    #[test]
    fn the_release_manifest_lists_every_builtin_file() {
        let script = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/build-release.ps1"),
        )
        .unwrap();
        let line = script
            .lines()
            .find(|line| line.trim_start().starts_with("$ExpectedBuiltins "))
            .expect("`$ExpectedBuiltins = @(...)` line");
        let mut manifest: Vec<&str> = line.split('\'').skip(1).step_by(2).collect();
        manifest.sort_unstable();
        let mut code: Vec<&str> = BuiltinId::ALL.into_iter().map(file_of).collect();
        code.sort_unstable();
        assert_eq!(manifest, code, "{line}");
    }

    #[test]
    fn without_a_folder_every_builtin_is_an_empty_stand_in_with_no_file() {
        let builtins = Builtins::new(None, &AgentBackends);
        for id in BuiltinId::ALL {
            let view = builtins.view(id);
            assert_eq!(view.file, None, "{id:?}");
            assert_eq!(view.stand_in, Some(NO_FOLDER), "{id:?}");
            match (id, view.content) {
                (BuiltinId::DefaultPreset, ItemContent::Preset { doc, .. }) => {
                    assert!(doc.presets.is_empty() && doc.instructions.is_empty());
                    assert!(doc.skills.is_empty() && doc.settings.is_empty());
                    assert!(doc.folders.is_empty());
                }
                (BuiltinId::Priming, ItemContent::Instruction { excerpt }) => {
                    assert!(excerpt.is_empty())
                }
                (BuiltinId::DefaultSettings, ItemContent::Settings { doc, .. }) => {
                    assert!(doc.raw.is_empty())
                }
                (id, content) => panic!("{id:?}: {content:?}"),
            }
        }
    }

    #[test]
    fn a_file_that_cannot_be_read_or_does_not_parse_stands_in_with_why() {
        let dir = folder(&[
            (
                file_of(BuiltinId::DefaultPreset),
                "kind: preset\nversion: [\n",
            ),
            (file_of(BuiltinId::DefaultSettings), "a: 1\n"),
        ]);
        let builtins = Builtins::new(Some(dir.path()), &AgentBackends);
        let preset = builtins.view(BuiltinId::DefaultPreset);
        assert_eq!(*preset.kind, ItemKind::Preset, "not broken");
        let ItemContent::Preset { doc, .. } = preset.content else {
            panic!("{:?}", preset.content)
        };
        assert!(doc.presets.is_empty() && doc.instructions.is_empty());
        assert_eq!(preset.file, None);
        let Some((_, ItemContent::Broken { reason })) =
            yaml_item(b"kind: preset\nversion: [\n", &AgentBackends)
        else {
            panic!("the reader refuses it")
        };
        let message = reason.args["message"].as_str().unwrap();
        assert_eq!(
            preset.stand_in,
            Some(format!("yaml-syntax: {message}").as_str()),
            "the reader's code and its message as plain text"
        );
        let priming = builtins.view(BuiltinId::Priming);
        assert_eq!(priming.file, None, "missing file");
        assert!(priming.stand_in.is_some_and(|why| !why.is_empty()));
        let settings = builtins.view(BuiltinId::DefaultSettings);
        assert_eq!(settings.file, None, "no `kind` — not a settings file");
        assert_eq!(settings.stand_in, Some(NO_KIND));
    }
}
