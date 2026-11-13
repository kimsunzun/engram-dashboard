//! The built-in assets: `engram:/…` → files of the same shape as the library under the distribution's `builtin\`
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
}

struct Entry {
    kind: ItemKind,
    content: ItemContent,
    /// `Err` = the stand-in, with why.
    file: Result<PathBuf, String>,
    /// The file's text as read (not UTF-8 → U+FFFD) — empty for a stand-in.
    text: String,
}

impl Builtins {
    /// Reads every built-in under `dir` (`None` = there is no built-in folder). Preset tags are read through
    /// `backends`, as in a library preset.
    pub fn new(dir: Option<&Path>, backends: &dyn Backends) -> Self {
        if dir.is_none() {
            tracing::warn!("built-in 폴더 없음, 모든 built-in을 stand-in으로 시작");
        }
        let entry = |id| match dir.map(|dir| dir.join(file_of(id))) {
            None => stand_in(id, NO_FOLDER.to_owned()),
            Some(path) => match load(id, &path, backends) {
                Ok((kind, content, text)) => Entry {
                    kind,
                    content,
                    file: Ok(path),
                    text,
                },
                Err(why) => {
                    tracing::warn!(
                        builtin = id.id(),
                        path = ?path,
                        "built-in load 실패, stand-in으로 대체: {}",
                        escape_controls(&why)
                    );
                    stand_in(id, why)
                }
            },
        };
        Builtins {
            default_preset: entry(BuiltinId::DefaultPreset),
            priming: entry(BuiltinId::Priming),
            default_settings: entry(BuiltinId::DefaultSettings),
        }
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

/// Why every built-in stands in when there is no built-in folder.
const NO_FOLDER: &str = "기본 제공 폴더 없음";

/// Why a built-in YAML file that declares no `kind` stands in.
const NO_KIND: &str = "`kind` 없음";

/// The built-in `id` read from `path` with the file's text, or why it cannot be used — a file that does not parse
/// gives its reader's code and message.
fn load(
    id: BuiltinId,
    path: &Path,
    backends: &dyn Backends,
) -> Result<(ItemKind, ItemContent, String), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
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
                None => Err(NO_KIND.to_owned()),
                Some((_, ItemContent::Broken { reason })) => {
                    // `ParseError::to_reason` always writes `message` as a string.
                    let message = reason.args["message"].as_str().unwrap_or_default();
                    Err(format!("{}: {message}", reason.code))
                }
                Some((kind, content)) => Ok((kind, content, text)),
            }
        }
    }
}

fn stand_in(id: BuiltinId, why: String) -> Entry {
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
                    settings: Vec::new(),
                    folders: Vec::new(),
                    unmodeled: Vec::new(),
                    line_problems: Vec::new(),
                },
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
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::backends::AgentBackends;
    use crate::path::AssetRef;

    pub(crate) const DEFAULT_PRESET: &str = "kind: preset\nversion: 1\ndescription: the default\ninstructions:\n  - engram:/priming\nsettings:\n  - engram:/settings/default\n";
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
        let ItemContent::Preset { doc } = preset.content else {
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

    #[test]
    fn without_a_folder_every_builtin_is_an_empty_stand_in_with_no_file() {
        let builtins = Builtins::new(None, &AgentBackends);
        for id in BuiltinId::ALL {
            let view = builtins.view(id);
            assert_eq!(view.file, None, "{id:?}");
            assert_eq!(view.stand_in, Some(NO_FOLDER), "{id:?}");
            match (id, view.content) {
                (BuiltinId::DefaultPreset, ItemContent::Preset { doc }) => {
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
        let ItemContent::Preset { doc } = preset.content else {
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
