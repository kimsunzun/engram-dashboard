//! The library root setting — `library.json` in the daemon state folder the daemon hands in.
//!
//! - **One setting, with a default value** (ADR-0375): the file holds `"root": "default"` or an absolute normalized
//!   path. `"default"` is resolved every start to that run's default place (`LibraryValues::default_root`), so it
//!   follows the data folder when it moves. The folder's existence never changes the setting.
//! - **Base data-file rules** (ADR-0291 · ADR-0376): [`load`] runs once before the service is built and changes no
//!   file (it sweeps only this file's leftover temp files). A newer or unreadable file makes the setting read-only for
//!   the run; a corrupt one is copied aside to `<name>.corrupt` before the next write; every write is judged again
//!   right before it.
//! - **Written only after the root change succeeded** (ADR-0381): `LibraryService` publishes the root first and then
//!   calls [`ConfigFile::save`]; nothing is ever undone. A save that fails leaves the file as it was and the state
//!   `unsaved` — the live root's setting, not the file's — until a later save succeeds; the next start follows the
//!   file that is left.
//! - **The caller serializes saves** — `LibraryService` holds its root-change mutex across the root change and
//!   [`ConfigFile::save`]. The state lock here is a leaf taken under that mutex only to read or set the state, never
//!   across IO.
// ADR-0291
// ADR-0375
// ADR-0376
// ADR-0381

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use engram_dashboard_agent::persistence::LOAD_HOOKS;
use engram_dashboard_base::file::{
    self, copy_aside, write_atomic, Claim, Loaded, Parsed, Refused, Spec, WritePolicy,
};
use engram_dashboard_base::sync::lock;
use engram_dashboard_platform::fs::normalize_folder;
use engram_dashboard_platform::process::pid_alive;
use serde_json::{json, Map, Value};

use crate::store::OS_HOOKS;

/// The file name, joined onto the state folder the daemon hands in (ADR-0264 decision 4: a file name one store uses
/// belongs to that store).
pub const LIBRARY_CONFIG_FILE: &str = "library.json";

/// The word that stores the setting at its default value.
const DEFAULT_WORD: &str = "default";

const SPEC: Spec = Spec {
    version_key: "schema_version",
    current: 1,
    cap: 64 * 1024,
    shape,
};

/// What the root setting holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RootSetting {
    /// The default value — the run's default place.
    Default,
    /// A root the user set: absolute and normalized.
    Explicit(PathBuf),
}

impl RootSetting {
    /// The root this setting names this run.
    pub fn resolve(&self, default_root: &Path) -> PathBuf {
        match self {
            RootSetting::Default => default_root.to_path_buf(),
            RootSetting::Explicit(root) => root.clone(),
        }
    }
}

/// `library.json` as [`load`] read it, with the folder it lives in — `LibraryValues::config`.
#[derive(Clone, Debug)]
pub struct LibraryConfig {
    state_dir: PathBuf,
    load: Load,
}

#[derive(Clone, Debug)]
enum Load {
    /// Never written — the first-start step is due.
    Missing,
    Usable(RootSetting),
    Corrupt,
    /// Newer or unreadable — `why`.
    ReadOnly(String),
}

/// The state of `library.json` now (`LibraryService::status`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigHealth {
    /// Usable, or not written yet.
    Ok,
    /// Newer or unreadable at load — no write this run.
    ReadOnly { why: String },
    /// Copied aside to `<name>.corrupt` before the next write.
    Corrupt,
    /// This run published a root whose save failed (`why`): the live root may differ from the file's setting, and the
    /// next start follows the file that is left. Cleared by the next successful save; it wins over `Corrupt`.
    // ADR-0381
    Unsaved { why: String },
}

/// Reads `library.json` under `state_dir`. Never fails: every outcome is a state (module header).
pub fn load(state_dir: &Path) -> LibraryConfig {
    match file::sweep_temps(state_dir, &[LIBRARY_CONFIG_FILE], pid_alive) {
        Ok(swept) => {
            for (path, removed) in swept {
                if let Err(e) = removed {
                    tracing::debug!(path = %path.display(), "library.json temp not removed: {e}");
                }
            }
        }
        Err(e) => {
            tracing::debug!(dir = %state_dir.display(), "library.json temp sweep failed: {e}")
        }
    }
    let path = state_dir.join(LIBRARY_CONFIG_FILE);
    // ADR-0291 R14: a lock left after the long load budget makes the run read-only.
    let load = match file::load(&path, &SPEC, LOAD_HOOKS) {
        Loaded::Missing => Load::Missing,
        Loaded::Parsed(Parsed::Usable { doc, .. }) => match setting_of(&doc) {
            Ok(setting) => Load::Usable(setting),
            Err(why) => corrupt(&path, &why),
        },
        Loaded::Parsed(Parsed::Unusable(why)) => corrupt(&path, &why),
        Loaded::Parsed(Parsed::Newer { found }) => {
            tracing::error!(path = %path.display(), found, "library.json is newer; read-only this run");
            Load::ReadOnly(format!(
                "written by a newer version (schema_version {found})"
            ))
        }
        Loaded::Failed(e) => {
            tracing::error!(path = %path.display(), "library.json read failed; read-only this run: {e}");
            Load::ReadOnly(e.to_string())
        }
    };
    LibraryConfig {
        state_dir: state_dir.to_path_buf(),
        load,
    }
}

fn corrupt(path: &Path, why: &str) -> Load {
    tracing::error!(path = %path.display(), "library.json corrupt; root unset until a set_root: {why}");
    Load::Corrupt
}

fn shape(_version: u64, doc: &Map<String, Value>) -> Result<(), String> {
    setting_of(doc).map(drop)
}

fn setting_of(doc: &Map<String, Value>) -> Result<RootSetting, String> {
    let refused = || "`root` is neither \"default\" nor an absolute path".to_owned();
    match doc.get("root").and_then(Value::as_str) {
        Some(DEFAULT_WORD) => Ok(RootSetting::Default),
        Some(path) => normalize_folder(Path::new(path))
            .map(RootSetting::Explicit)
            .map_err(|_| refused()),
        None => Err(refused()),
    }
}

/// The live `library.json` of one service (module header).
pub(crate) struct ConfigFile {
    path: PathBuf,
    state: Mutex<FileState>,
    /// The failure-injection seam of the tests: `Some(why)` makes every atomic write fail with `why`.
    #[cfg(test)]
    fail_writes: Mutex<Option<String>>,
}

struct FileState {
    /// `None` = not readable (corrupt or read-only). While `unsaved`, the live root's setting.
    setting: Option<RootSetting>,
    health: ConfigHealth,
    claim: Claim,
    /// Loaded as missing, and no root published since.
    first_start_due: bool,
}

impl ConfigFile {
    pub(crate) fn new(config: &LibraryConfig) -> Self {
        let (setting, health, claim) = match &config.load {
            Load::Missing => (Some(RootSetting::Default), ConfigHealth::Ok, Claim::NotYet),
            Load::Usable(setting) => (Some(setting.clone()), ConfigHealth::Ok, Claim::Adopted),
            Load::Corrupt => (None, ConfigHealth::Corrupt, Claim::NotYet),
            Load::ReadOnly(why) => (
                None,
                ConfigHealth::ReadOnly { why: why.clone() },
                Claim::NotYet,
            ),
        };
        ConfigFile {
            path: config.state_dir.join(LIBRARY_CONFIG_FILE),
            state: Mutex::new(FileState {
                setting,
                health,
                claim,
                first_start_due: matches!(config.load, Load::Missing),
            }),
            #[cfg(test)]
            fail_writes: Mutex::new(None),
        }
    }

    /// The root the service starts with: the usable setting resolved; `None` when the file is missing (the
    /// first-start step publishes one), corrupt or read-only.
    pub(crate) fn initial_root(config: &LibraryConfig, default_root: &Path) -> Option<PathBuf> {
        match &config.load {
            Load::Usable(setting) => Some(setting.resolve(default_root)),
            Load::Missing | Load::Corrupt | Load::ReadOnly(_) => None,
        }
    }

    pub(crate) fn view(&self) -> (Option<RootSetting>, ConfigHealth) {
        let state = lock(&self.state);
        (state.setting.clone(), state.health.clone())
    }

    /// Why no write is allowed this run; `None` = writes are allowed.
    pub(crate) fn read_only(&self) -> Option<String> {
        match &lock(&self.state).health {
            ConfigHealth::ReadOnly { why } => Some(why.clone()),
            ConfigHealth::Ok | ConfigHealth::Corrupt | ConfigHealth::Unsaved { .. } => None,
        }
    }

    pub(crate) fn first_start_due(&self) -> bool {
        lock(&self.state).first_start_due
    }

    /// Records `setting`, the setting of the root just published: judges the current file (base `judge` rules — a
    /// corrupt file, or one that appeared after a missing load, is copied aside first), then writes it atomically.
    /// `Ok` = the file holds `setting` and is ours (ADR-0291 R8); `Err(why)` = the file is as it was (a copy aside may
    /// have been made) and the state is `unsaved`. Either way the state names `setting`, the live one. A read-only
    /// file is never written and its state is kept. Call only after the root change succeeded, holding the root-change
    /// mutex.
    // ADR-0381
    pub(crate) fn save(&self, setting: &RootSetting) -> Result<(), String> {
        if let Some(why) = self.read_only() {
            return Err(format!("library.json is read-only this run: {why}"));
        }
        let saved = self.write_judged(setting);
        if let Err(why) = &saved {
            tracing::error!(path = %self.path.display(), "library.json not written: {why}");
        }
        let mut state = lock(&self.state);
        state.setting = Some(setting.clone());
        state.first_start_due = false;
        match &saved {
            Ok(()) => {
                state.health = ConfigHealth::Ok;
                state.claim = Claim::Adopted;
            }
            Err(why) => state.health = ConfigHealth::Unsaved { why: why.clone() },
        }
        saved
    }

    fn write_judged(&self, setting: &RootSetting) -> Result<(), String> {
        let word = match setting {
            RootSetting::Default => DEFAULT_WORD,
            RootSetting::Explicit(root) => root
                .to_str()
                .ok_or_else(|| format!("the path is not UTF-8: {}", root.display()))?,
        };
        let bytes =
            serde_json::to_vec_pretty(&json!({ "schema_version": SPEC.current, "root": word }))
                .map_err(|e| e.to_string())?;
        file::check_cap(bytes.len(), &SPEC).map_err(|e| e.to_string())?;
        let claim = lock(&self.state).claim;
        let current = file::load(&self.path, &SPEC, OS_HOOKS);
        match current.write_policy(claim) {
            WritePolicy::Refuse(Refused::Newer { found }) => {
                return Err(format!(
                    "library.json was written by a newer version (schema_version {found})"
                ))
            }
            WritePolicy::Refuse(Refused::ReadFailed) => {
                let why = match &current {
                    Loaded::Failed(e) => e.to_string(),
                    _ => "unreadable".to_owned(),
                };
                return Err(format!("library.json cannot be read: {why}"));
            }
            WritePolicy::CopyAsideFirst => {
                let aside = copy_aside(&self.path, OS_HOOKS)
                    .map_err(|e| format!("library.json copy aside failed: {e}"))?;
                tracing::warn!(to = %aside.display(), "library.json copied aside before overwrite");
            }
            WritePolicy::Write => {}
        }
        self.write_file(&bytes)
            .map_err(|e| format!("library.json write failed: {e}"))
    }

    /// The atomic write — the one place the tests make a write fail ([`ConfigFile::fail_writes`]).
    fn write_file(&self, bytes: &[u8]) -> std::io::Result<()> {
        #[cfg(test)]
        if let Some(why) = lock(&self.fail_writes).clone() {
            return Err(std::io::Error::other(why));
        }
        write_atomic(&self.path, bytes, OS_HOOKS)
    }

    /// Makes every following atomic write fail with `why` (`None` = write again).
    #[cfg(test)]
    pub(crate) fn fail_writes(&self, why: Option<&str>) {
        *lock(&self.fail_writes) = why.map(str::to_owned);
    }
}

/// Test constructors — a config as if [`load`] had read it, without a file.
#[cfg(test)]
impl LibraryConfig {
    pub(crate) fn missing(state_dir: &Path) -> Self {
        LibraryConfig {
            state_dir: state_dir.to_path_buf(),
            load: Load::Missing,
        }
    }

    pub(crate) fn usable(state_dir: &Path, setting: RootSetting) -> Self {
        LibraryConfig {
            state_dir: state_dir.to_path_buf(),
            load: Load::Usable(setting),
        }
    }

    /// The read-only reason of the load, if any.
    fn load_why(&self) -> Option<&str> {
        match &self.load {
            Load::ReadOnly(why) => Some(why),
            Load::Missing | Load::Usable(_) | Load::Corrupt => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs(dir: &Path, name: &str) -> PathBuf {
        normalize_folder(&dir.join(name)).unwrap()
    }

    fn put(dir: &Path, text: &str) {
        std::fs::write(dir.join(LIBRARY_CONFIG_FILE), text).unwrap();
    }

    fn on_disk(dir: &Path) -> Option<Value> {
        let text = std::fs::read_to_string(dir.join(LIBRARY_CONFIG_FILE)).ok()?;
        Some(serde_json::from_str(&text).unwrap())
    }

    fn explicit_doc(path: &Path) -> String {
        json!({ "schema_version": 1, "root": path.to_str().unwrap() }).to_string()
    }

    #[test]
    fn a_missing_file_holds_the_default_value_with_no_root_until_the_first_start() {
        let temp = tempfile::tempdir().unwrap();
        let config = load(temp.path());
        let file = ConfigFile::new(&config);
        assert_eq!(file.view(), (Some(RootSetting::Default), ConfigHealth::Ok));
        assert!(file.first_start_due());
        assert_eq!(
            ConfigFile::initial_root(&config, &abs(temp.path(), "lib")),
            None
        );
        assert!(on_disk(temp.path()).is_none(), "a load writes nothing");
    }

    #[test]
    fn the_default_value_resolves_to_each_runs_default_place_and_an_explicit_root_stays_put() {
        let (first, moved) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        put(first.path(), r#"{"schema_version": 1, "root": "default"}"#);
        std::fs::copy(
            first.path().join(LIBRARY_CONFIG_FILE),
            moved.path().join(LIBRARY_CONFIG_FILE),
        )
        .unwrap();
        let here = ConfigFile::initial_root(&load(first.path()), &abs(first.path(), "library"));
        let there = ConfigFile::initial_root(&load(moved.path()), &abs(moved.path(), "library"));
        assert_eq!(here, Some(abs(first.path(), "library")));
        assert_eq!(
            there,
            Some(abs(moved.path(), "library")),
            "the setting moved with the data"
        );

        let explicit = abs(first.path(), "elsewhere");
        put(moved.path(), &explicit_doc(&explicit));
        let config = load(moved.path());
        assert_eq!(
            ConfigFile::initial_root(&config, &abs(moved.path(), "library")),
            Some(explicit.clone())
        );
        assert_eq!(
            ConfigFile::new(&config).view().0,
            Some(RootSetting::Explicit(explicit))
        );
    }

    #[test]
    fn a_root_that_is_neither_the_default_word_nor_absolute_is_corrupt() {
        for doc in [
            r#"{"schema_version": 1, "root": "relative\\lib"}"#,
            r#"{"schema_version": 1, "root": "Default"}"#,
            r#"{"schema_version": 1, "root": 3}"#,
            r#"{"schema_version": 1}"#,
            "not json",
        ] {
            let temp = tempfile::tempdir().unwrap();
            put(temp.path(), doc);
            let config = load(temp.path());
            let file = ConfigFile::new(&config);
            assert_eq!(file.view(), (None, ConfigHealth::Corrupt), "{doc}");
            assert!(!file.first_start_due(), "{doc}");
            assert_eq!(
                ConfigFile::initial_root(&config, temp.path()),
                None,
                "{doc}"
            );
        }
    }

    #[test]
    fn a_newer_or_unreadable_file_is_read_only_with_why_and_never_written() {
        let newer = tempfile::tempdir().unwrap();
        put(newer.path(), r#"{"schema_version": 2, "root": "default"}"#);
        let unreadable = tempfile::tempdir().unwrap();
        std::fs::create_dir(unreadable.path().join(LIBRARY_CONFIG_FILE)).unwrap();
        for dir in [newer.path(), unreadable.path()] {
            let config = load(dir);
            let file = ConfigFile::new(&config);
            let (setting, health) = file.view();
            assert_eq!(setting, None);
            assert!(
                matches!(&health, ConfigHealth::ReadOnly { why } if !why.is_empty()),
                "{health:?}"
            );
            assert!(file.read_only().is_some());
            assert_eq!(ConfigFile::initial_root(&config, dir), None);
            assert!(!file.first_start_due());
            assert!(
                file.save(&RootSetting::Default).is_err(),
                "a read-only file is never saved"
            );
            assert_eq!(file.view(), (setting, health), "the state is kept");
        }
        assert_eq!(
            std::fs::read_to_string(newer.path().join(LIBRARY_CONFIG_FILE)).unwrap(),
            r#"{"schema_version": 2, "root": "default"}"#
        );
        assert!(unreadable.path().join(LIBRARY_CONFIG_FILE).is_dir());
        assert!(
            load(newer.path())
                .load_why()
                .unwrap()
                .contains("schema_version 2"),
            "the why names the newer version"
        );
    }

    #[test]
    fn a_save_over_a_missing_file_writes_the_word_and_makes_the_file_ours() {
        let temp = tempfile::tempdir().unwrap();
        let file = ConfigFile::new(&load(temp.path()));
        file.save(&RootSetting::Default).unwrap();
        assert_eq!(
            on_disk(temp.path()),
            Some(json!({"schema_version": 1, "root": "default"}))
        );
        assert_eq!(file.view(), (Some(RootSetting::Default), ConfigHealth::Ok));
        assert!(!file.first_start_due());
        let root = abs(temp.path(), "lib");
        file.save(&RootSetting::Explicit(root.clone())).unwrap();
        assert_eq!(
            on_disk(temp.path()).unwrap()["root"],
            json!(root.to_str().unwrap())
        );
        assert!(
            !temp.path().join("library.json.corrupt").exists(),
            "our own file is not copied aside"
        );
    }

    #[test]
    fn a_save_over_an_adopted_file_replaces_it_without_a_copy_aside() {
        let temp = tempfile::tempdir().unwrap();
        put(temp.path(), &explicit_doc(&abs(temp.path(), "old")));
        let file = ConfigFile::new(&load(temp.path()));
        let new = abs(temp.path(), "new");
        file.save(&RootSetting::Explicit(new.clone())).unwrap();
        assert_eq!(
            on_disk(temp.path()).unwrap()["root"],
            json!(new.to_str().unwrap())
        );
        assert!(
            !temp.path().join("library.json.corrupt").exists(),
            "an adopted file is not copied aside"
        );
    }

    #[test]
    fn a_save_over_a_corrupt_file_copies_it_aside_first_and_makes_it_ok() {
        let temp = tempfile::tempdir().unwrap();
        put(temp.path(), "not json");
        let file = ConfigFile::new(&load(temp.path()));
        file.save(&RootSetting::Default).unwrap();
        assert_eq!(
            std::fs::read_to_string(temp.path().join("library.json.corrupt")).unwrap(),
            "not json"
        );
        assert_eq!(
            on_disk(temp.path()),
            Some(json!({"schema_version": 1, "root": "default"}))
        );
        assert_eq!(file.view(), (Some(RootSetting::Default), ConfigHealth::Ok));
    }

    #[test]
    fn a_usable_file_that_appeared_after_a_missing_load_is_copied_aside_before_the_first_write() {
        let temp = tempfile::tempdir().unwrap();
        let file = ConfigFile::new(&load(temp.path()));
        put(temp.path(), &explicit_doc(&abs(temp.path(), "theirs")));
        file.save(&RootSetting::Default).unwrap();
        assert!(temp.path().join("library.json.corrupt").exists());
    }

    #[test]
    fn a_save_judges_the_file_again_and_refuses_a_newer_one_that_appeared_leaving_it_unsaved() {
        let temp = tempfile::tempdir().unwrap();
        let file = ConfigFile::new(&load(temp.path()));
        let newer = r#"{"schema_version": 9, "root": "default"}"#;
        put(temp.path(), newer);
        let why = file.save(&RootSetting::Default).unwrap_err();
        assert!(why.contains("newer"), "{why}");
        assert_eq!(
            std::fs::read_to_string(temp.path().join(LIBRARY_CONFIG_FILE)).unwrap(),
            newer
        );
        assert_eq!(
            file.view(),
            (Some(RootSetting::Default), ConfigHealth::Unsaved { why })
        );
        assert!(!file.first_start_due(), "a root was published");
    }

    #[test]
    fn a_failed_save_leaves_the_file_as_it_was_and_is_unsaved_until_a_save_succeeds() {
        let temp = tempfile::tempdir().unwrap();
        put(temp.path(), "not json");
        let file = ConfigFile::new(&load(temp.path()));
        let setting = RootSetting::Explicit(abs(temp.path(), "lib"));
        file.fail_writes(Some("disk full"));
        let why = file.save(&setting).unwrap_err();
        assert!(why.contains("disk full"), "{why}");
        assert_eq!(
            std::fs::read_to_string(temp.path().join(LIBRARY_CONFIG_FILE)).unwrap(),
            "not json",
            "the file is as it was"
        );
        assert_eq!(
            file.view(),
            (Some(setting.clone()), ConfigHealth::Unsaved { why }),
            "unsaved wins over corrupt and names the live setting"
        );
        assert!(file.read_only().is_none(), "the next save may write");
        file.fail_writes(None);
        file.save(&setting).unwrap();
        assert_eq!(file.view(), (Some(setting), ConfigHealth::Ok));
        assert_eq!(
            std::fs::read_to_string(temp.path().join("library.json.corrupt")).unwrap(),
            "not json",
            "the user's bytes are still the aside copy"
        );
    }

    #[test]
    fn a_load_sweeps_a_leftover_temp_of_library_json_only() {
        let temp = tempfile::tempdir().unwrap();
        // A dead pid's temp (base `temp_path` shape) and another file's temp of the same shape.
        let ours = temp.path().join("library.json.tmp4294967294.1");
        let theirs = temp.path().join("identity.json.tmp4294967294.1");
        std::fs::write(&ours, "x").unwrap();
        std::fs::write(&theirs, "x").unwrap();
        load(temp.path());
        assert!(!ours.exists());
        assert!(theirs.exists());
    }
}
