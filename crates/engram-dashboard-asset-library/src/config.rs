//! The library mount setting — `library.json` in the daemon state folder the daemon hands in.
//!
//! - **The mount list** (ADR-0375 · ADR-0400): `{"schema_version": 2, "mounts": [{"name", "path"}]}`, in the order
//!   shown. A `path` is `"default"` or an absolute normalized folder; `"default"` is resolved every start to that run's
//!   default place (`LibraryValues::default_root`), so it follows the data folder when it moves. A mount's folder
//!   existing or not never changes the setting. The built-in mount is not stored.
//! - **Schema 2 only** (ADR-0398): a schema-1 file (one `root`, never shipped) has no reader, so it is corrupt like any
//!   shape this build cannot read. Loosening the name rules or adding a `path` form or an entry key raises
//!   `schema_version` — an older build would otherwise read the new file as corrupt and replace it, or drop the
//!   unknown key on its next save.
//! - **Base data-file rules** (ADR-0291 · ADR-0376): [`load`] runs once before the service is built and changes no
//!   file (it sweeps only this file's leftover temp files). A newer or unreadable file makes the setting read-only for
//!   the run; a corrupt one is copied aside to `<name>.corrupt` before the next write; every write is judged again
//!   right before it.
//! - **Written only after the mount change succeeded** (ADR-0381): `LibraryService` publishes the mounts first and then
//!   calls [`ConfigFile::save`]; nothing is ever undone. A save that fails leaves the file as it was and the state
//!   `unsaved` — the live mounts' setting, not the file's — until a later save succeeds; the next start follows the
//!   file that is left. The list the file last held (as loaded, or as last saved) is kept, so a mount command repeated
//!   after a failed save can tell a repeat from a mistake ([`ConfigFile::unsaved_removal`] — ADR-0400 decision 14).
//! - **The caller serializes saves** — `LibraryService` holds its mount-change mutex across the mount change and
//!   [`ConfigFile::save`]. The state lock here is a leaf taken under that mutex only to read or set the state, never
//!   across IO.
// ADR-0291
// ADR-0375
// ADR-0376
// ADR-0381
// ADR-0398
// ADR-0400

use std::collections::HashSet;
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

use crate::mounts::{Mount, MountName, Mounts};
use crate::store::OS_HOOKS;

/// The file name, joined onto the state folder the daemon hands in (ADR-0264 decision 4: a file name one store uses
/// belongs to that store).
pub const LIBRARY_CONFIG_FILE: &str = "library.json";

/// The word that stores a mount's folder at the default place.
const DEFAULT_WORD: &str = "default";

const SPEC: Spec = Spec {
    version_key: "schema_version",
    current: 2,
    cap: 64 * 1024,
    shape,
};

/// Where one stored mount's folder is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MountPlace {
    /// The default place of the run.
    Default,
    /// A folder the user set: absolute and normalized.
    Explicit(PathBuf),
}

impl MountPlace {
    /// The folder this place names this run.
    pub fn resolve(&self, default_root: &Path) -> PathBuf {
        match self {
            MountPlace::Default => default_root.to_path_buf(),
            MountPlace::Explicit(folder) => folder.clone(),
        }
    }
}

/// One stored mount — `{"name", "path"}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountSetting {
    pub name: MountName,
    pub place: MountPlace,
}

impl MountSetting {
    /// What the first start registers: [`crate::mounts::FIRST_MOUNT`] at the default place.
    // ADR-0400 decision 3
    pub fn first_start() -> Self {
        MountSetting {
            name: MountName::first(),
            place: MountPlace::Default,
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
    Usable(Vec<MountSetting>),
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
    /// This run published mounts whose save failed (`why`): the live mounts may differ from the file's setting, and the
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
        Loaded::Parsed(Parsed::Usable { doc, .. }) => match settings_of(&doc) {
            Ok(mounts) => Load::Usable(mounts),
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
    tracing::error!(path = %path.display(), "library.json corrupt; no mount until one is set: {why}");
    Load::Corrupt
}

fn shape(version: u64, doc: &Map<String, Value>) -> Result<(), String> {
    // ADR-0398: schema 1 was never shipped — no reader.
    if version < SPEC.current {
        return Err(format!(
            "schema_version {version} has no reader (this build reads {})",
            SPEC.current
        ));
    }
    settings_of(doc).map(drop)
}

fn settings_of(doc: &Map<String, Value>) -> Result<Vec<MountSetting>, String> {
    let entries = doc
        .get("mounts")
        .and_then(Value::as_array)
        .ok_or_else(|| "`mounts` is not a list".to_owned())?;
    let mut names = HashSet::new();
    let mut mounts = Vec::with_capacity(entries.len());
    for (at, entry) in entries.iter().enumerate() {
        let field = |key: &str| entry.get(key).and_then(Value::as_str);
        let name = field("name").ok_or_else(|| format!("mount {at}: `name` is not a string"))?;
        let name =
            MountName::parse(name).map_err(|why| format!("mount {at}: name {name:?}: {why}"))?;
        if !names.insert(name.key()) {
            return Err(format!("mount {at}: the name {:?} is taken", name.as_str()));
        }
        let place = match field("path") {
            Some(DEFAULT_WORD) => MountPlace::Default,
            Some(path) => normalize_folder(Path::new(path))
                .map(MountPlace::Explicit)
                .map_err(|_| {
                    format!("mount {at}: `path` is neither \"default\" nor an absolute path")
                })?,
            None => return Err(format!("mount {at}: `path` is not a string")),
        };
        mounts.push(MountSetting { name, place });
    }
    Ok(mounts)
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
    /// `None` = not readable (corrupt or read-only). While `unsaved`, the live mounts' setting; while the first start
    /// is due, what it would write.
    mounts: Option<Vec<MountSetting>>,
    health: ConfigHealth,
    claim: Claim,
    /// Loaded as missing, and no mount published since.
    first_start_due: bool,
    /// The list the file holds as this run knows it: as loaded (usable), then as last saved; `None` = it holds no
    /// list this run could read (missing, corrupt or read-only at load, and nothing saved since).
    saved: Option<Vec<MountSetting>>,
}

impl ConfigFile {
    pub(crate) fn new(config: &LibraryConfig) -> Self {
        let (mounts, health, claim) = match &config.load {
            Load::Missing => (
                Some(vec![MountSetting::first_start()]),
                ConfigHealth::Ok,
                Claim::NotYet,
            ),
            Load::Usable(mounts) => (Some(mounts.clone()), ConfigHealth::Ok, Claim::Adopted),
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
                mounts,
                health,
                claim,
                first_start_due: matches!(config.load, Load::Missing),
                saved: match &config.load {
                    Load::Usable(mounts) => Some(mounts.clone()),
                    Load::Missing | Load::Corrupt | Load::ReadOnly(_) => None,
                },
            }),
            #[cfg(test)]
            fail_writes: Mutex::new(None),
        }
    }

    /// The mounts the service starts with: the usable setting resolved; none when the file is missing (the first-start
    /// step publishes one), corrupt or read-only.
    pub(crate) fn initial_mounts(config: &LibraryConfig, default_root: &Path) -> Mounts {
        match &config.load {
            Load::Usable(mounts) => Mounts::new(
                mounts
                    .iter()
                    .map(|setting| Mount {
                        name: setting.name.clone(),
                        folder: setting.place.resolve(default_root),
                    })
                    .collect(),
            )
            .expect("the load refuses a taken name"),
            Load::Missing | Load::Corrupt | Load::ReadOnly(_) => Mounts::default(),
        }
    }

    pub(crate) fn view(&self) -> (Option<Vec<MountSetting>>, ConfigHealth) {
        let state = lock(&self.state);
        (state.mounts.clone(), state.health.clone())
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

    /// Whether a `removeMount` of `name`, not registered now, repeats one whose save failed: the setting is `unsaved`
    /// and the list the file last held still names `name` (by path key) — so the removal is live but not saved.
    // ADR-0400 decision 14
    pub(crate) fn unsaved_removal(&self, name: &MountName) -> bool {
        self.unsaved_saved_place(name).is_some()
    }

    /// For a `renameMount` of `from`, not registered now: the place the list the file last held gives `from` (by path
    /// key) while the setting is `unsaved`; `None` = no failed save names it. A rename keeps its folder, so the caller
    /// takes the call as the repeat of that rename only when this place is the live new name's folder.
    // ADR-0400 decision 14
    pub(crate) fn unsaved_rename(&self, from: &MountName) -> Option<MountPlace> {
        self.unsaved_saved_place(from)
    }

    fn unsaved_saved_place(&self, name: &MountName) -> Option<MountPlace> {
        let state = lock(&self.state);
        if !matches!(state.health, ConfigHealth::Unsaved { .. }) {
            return None;
        }
        state
            .saved
            .iter()
            .flatten()
            .find(|saved| saved.name.key() == name.key())
            .map(|saved| saved.place.clone())
    }

    /// Records `mounts`, the setting of the mounts just published: judges the current file (base `judge` rules — a
    /// corrupt file, or one that appeared after a missing load, is copied aside first), then writes it atomically.
    /// `Ok` = the file holds `mounts` and is ours (ADR-0291 R8); `Err(why)` = the file is as it was (a copy aside may
    /// have been made) and the state is `unsaved`. Either way the state names `mounts`, the live ones. A read-only
    /// file is never written and its state is kept. Call only after the mount change succeeded, holding the
    /// mount-change mutex.
    // ADR-0381
    pub(crate) fn save(&self, mounts: &[MountSetting]) -> Result<(), String> {
        if let Some(why) = self.read_only() {
            return Err(format!("library.json is read-only this run: {why}"));
        }
        let saved = self.write_judged(mounts);
        if let Err(why) = &saved {
            tracing::error!(path = %self.path.display(), "library.json not written: {why}");
        }
        let mut state = lock(&self.state);
        state.mounts = Some(mounts.to_vec());
        state.first_start_due = false;
        match &saved {
            Ok(()) => {
                state.health = ConfigHealth::Ok;
                state.claim = Claim::Adopted;
                state.saved = Some(mounts.to_vec());
            }
            Err(why) => state.health = ConfigHealth::Unsaved { why: why.clone() },
        }
        saved
    }

    fn write_judged(&self, mounts: &[MountSetting]) -> Result<(), String> {
        let entries = mounts
            .iter()
            .map(|mount| {
                let path = match &mount.place {
                    MountPlace::Default => DEFAULT_WORD,
                    MountPlace::Explicit(folder) => folder
                        .to_str()
                        .ok_or_else(|| format!("the path is not UTF-8: {}", folder.display()))?,
                };
                Ok(json!({ "name": mount.name.as_str(), "path": path }))
            })
            .collect::<Result<Vec<Value>, String>>()?;
        let bytes = serde_json::to_vec_pretty(
            &json!({ "schema_version": SPEC.current, "mounts": entries }),
        )
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

    /// A usable file holding one mount, the test mount `library` ([`MountName::library`]) at `place`.
    pub(crate) fn usable(state_dir: &Path, place: MountPlace) -> Self {
        LibraryConfig {
            state_dir: state_dir.to_path_buf(),
            load: Load::Usable(vec![MountSetting {
                name: MountName::library(),
                place,
            }]),
        }
    }

    /// A usable file holding `mounts`, in order.
    pub(crate) fn mounted(state_dir: &Path, mounts: Vec<MountSetting>) -> Self {
        LibraryConfig {
            state_dir: state_dir.to_path_buf(),
            load: Load::Usable(mounts),
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

    fn setting(name: &str, place: MountPlace) -> MountSetting {
        MountSetting {
            name: MountName::parse(name).unwrap(),
            place,
        }
    }

    fn library(place: MountPlace) -> Vec<MountSetting> {
        vec![setting("library", place)]
    }

    fn doc(mounts: Value) -> String {
        json!({ "schema_version": 2, "mounts": mounts }).to_string()
    }

    fn explicit_doc(path: &Path) -> String {
        doc(json!([{ "name": "library", "path": path.to_str().unwrap() }]))
    }

    fn mount(name: &str, folder: PathBuf) -> Mount {
        Mount {
            name: MountName::parse(name).unwrap(),
            folder,
        }
    }

    #[test]
    fn a_missing_file_holds_the_default_value_with_no_root_until_the_first_start() {
        let temp = tempfile::tempdir().unwrap();
        let config = load(temp.path());
        let file = ConfigFile::new(&config);
        assert_eq!(
            file.view(),
            (Some(vec![MountSetting::first_start()]), ConfigHealth::Ok),
            "what the first start would write: `default_library` at the default place"
        );
        assert_eq!(
            MountSetting::first_start(),
            setting("default_library", MountPlace::Default)
        );
        assert!(file.first_start_due());
        assert!(ConfigFile::initial_mounts(&config, &abs(temp.path(), "lib")).is_empty());
        assert!(on_disk(temp.path()).is_none(), "a load writes nothing");
    }

    #[test]
    fn the_default_value_resolves_to_each_runs_default_place_and_an_explicit_root_stays_put() {
        let (first, moved) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        put(
            first.path(),
            &doc(json!([{ "name": "library", "path": "default" }])),
        );
        std::fs::copy(
            first.path().join(LIBRARY_CONFIG_FILE),
            moved.path().join(LIBRARY_CONFIG_FILE),
        )
        .unwrap();
        let here = ConfigFile::initial_mounts(&load(first.path()), &abs(first.path(), "library"));
        let there = ConfigFile::initial_mounts(&load(moved.path()), &abs(moved.path(), "library"));
        assert_eq!(
            here,
            Mounts::new(vec![mount("library", abs(first.path(), "library"))]).unwrap()
        );
        assert_eq!(
            there,
            Mounts::new(vec![mount("library", abs(moved.path(), "library"))]).unwrap(),
            "the setting moved with the data"
        );

        let explicit = abs(first.path(), "elsewhere");
        put(moved.path(), &explicit_doc(&explicit));
        let config = load(moved.path());
        assert_eq!(
            ConfigFile::initial_mounts(&config, &abs(moved.path(), "library")),
            Mounts::new(vec![mount("library", explicit.clone())]).unwrap()
        );
        assert_eq!(
            ConfigFile::new(&config).view().0,
            Some(library(MountPlace::Explicit(explicit)))
        );
    }

    #[test]
    fn a_root_that_is_neither_the_default_word_nor_absolute_is_corrupt() {
        for text in [
            doc(json!([{ "name": "library", "path": "relative\\lib" }])),
            doc(json!([{ "name": "library", "path": "Default" }])),
            doc(json!([{ "name": "library", "path": 3 }])),
            doc(json!([{ "name": "library" }])),
            "not json".to_owned(),
        ] {
            let temp = tempfile::tempdir().unwrap();
            put(temp.path(), &text);
            let config = load(temp.path());
            let file = ConfigFile::new(&config);
            assert_eq!(file.view(), (None, ConfigHealth::Corrupt), "{text}");
            assert!(!file.first_start_due(), "{text}");
            assert!(
                ConfigFile::initial_mounts(&config, temp.path()).is_empty(),
                "{text}"
            );
        }
    }

    // ADR-0398: the unshipped schema 1 has no reader.
    #[test]
    fn a_schema_1_file_has_no_reader_so_it_is_corrupt_and_copied_aside_at_the_next_save() {
        let temp = tempfile::tempdir().unwrap();
        let old = r#"{"schema_version": 1, "root": "default"}"#;
        put(temp.path(), old);
        let config = load(temp.path());
        let file = ConfigFile::new(&config);
        assert_eq!(file.view(), (None, ConfigHealth::Corrupt));
        assert!(!file.first_start_due(), "a file is there: no first start");
        assert!(ConfigFile::initial_mounts(&config, temp.path()).is_empty());
        let unversioned = tempfile::tempdir().unwrap();
        put(unversioned.path(), r#"{"mounts": []}"#);
        assert_eq!(
            ConfigFile::new(&load(unversioned.path())).view(),
            (None, ConfigHealth::Corrupt),
            "no version reads as schema 1"
        );

        file.save(&library(MountPlace::Default)).unwrap();
        assert_eq!(
            std::fs::read_to_string(temp.path().join("library.json.corrupt")).unwrap(),
            old
        );
        assert_eq!(
            on_disk(temp.path()),
            Some(
                json!({ "schema_version": 2, "mounts": [{ "name": "library", "path": "default" }] })
            )
        );
    }

    #[test]
    fn a_mount_list_with_an_invalid_reserved_or_taken_name_or_no_list_is_corrupt() {
        let path = "default";
        let mut texts = vec![
            json!({ "schema_version": 2 }).to_string(),
            json!({ "schema_version": 2, "mounts": {} }).to_string(),
            doc(json!(["library"])),
            doc(json!([{ "path": path }])),
            doc(json!([{ "name": 7, "path": path }])),
            doc(json!([{ "name": "a/b", "path": path }])),
            doc(json!([{ "name": ".trash", "path": path }])),
            doc(json!([{ "name": "engram", "path": path }])),
            doc(json!([{ "name": "Engram", "path": path }])),
            doc(json!([{ "name": "x".repeat(65), "path": path }])),
            doc(json!([{ "name": "team", "path": path }, { "name": "team", "path": "default" }])),
        ];
        // Taken by the platform path key: only where it folds case are these two one name.
        if engram_dashboard_platform::fs::same_path(Path::new("team"), Path::new("TEAM")) {
            texts.push(doc(
                json!([{ "name": "team", "path": path }, { "name": "TEAM", "path": path }]),
            ));
        }
        for text in texts {
            let temp = tempfile::tempdir().unwrap();
            put(temp.path(), &text);
            assert_eq!(
                ConfigFile::new(&load(temp.path())).view(),
                (None, ConfigHealth::Corrupt),
                "{text}"
            );
        }
    }

    #[test]
    fn a_schema_2_mount_list_round_trips_in_order_and_an_empty_one_is_usable_with_no_mount() {
        let temp = tempfile::tempdir().unwrap();
        let file = ConfigFile::new(&load(temp.path()));
        let team = abs(temp.path(), "team");
        let mounts = vec![
            setting("library", MountPlace::Default),
            setting("company", MountPlace::Explicit(team.clone())),
            setting("업무", MountPlace::Explicit(abs(temp.path(), "work"))), // hangul-ok — non-ASCII mount name fixture
        ];
        file.save(&mounts).unwrap();
        assert_eq!(
            on_disk(temp.path()),
            Some(json!({ "schema_version": 2, "mounts": [
                { "name": "library", "path": "default" },
                { "name": "company", "path": team.to_str().unwrap() },
                { "name": "업무", "path": abs(temp.path(), "work").to_str().unwrap() }, // hangul-ok — fixture
            ] }))
        );
        let config = load(temp.path());
        assert_eq!(
            ConfigFile::new(&config).view(),
            (Some(mounts), ConfigHealth::Ok)
        );
        let names: Vec<String> = ConfigFile::initial_mounts(&config, &abs(temp.path(), "lib"))
            .iter()
            .map(|mount| mount.name.as_str().to_owned())
            .collect();
        assert_eq!(names, ["library", "company", "업무"], "the order shown"); // hangul-ok — fixture

        file.save(&[]).unwrap();
        assert_eq!(
            on_disk(temp.path()),
            Some(json!({ "schema_version": 2, "mounts": [] }))
        );
        let config = load(temp.path());
        assert_eq!(
            ConfigFile::new(&config).view(),
            (Some(Vec::new()), ConfigHealth::Ok)
        );
        assert!(ConfigFile::initial_mounts(&config, temp.path()).is_empty());
        assert!(!ConfigFile::new(&config).first_start_due());
    }

    #[test]
    fn a_newer_or_unreadable_file_is_read_only_with_why_and_never_written() {
        let newer = tempfile::tempdir().unwrap();
        let newer_text = r#"{"schema_version": 3, "mounts": []}"#;
        put(newer.path(), newer_text);
        let unreadable = tempfile::tempdir().unwrap();
        std::fs::create_dir(unreadable.path().join(LIBRARY_CONFIG_FILE)).unwrap();
        for dir in [newer.path(), unreadable.path()] {
            let config = load(dir);
            let file = ConfigFile::new(&config);
            let (mounts, health) = file.view();
            assert_eq!(mounts, None);
            assert!(
                matches!(&health, ConfigHealth::ReadOnly { why } if !why.is_empty()),
                "{health:?}"
            );
            assert!(file.read_only().is_some());
            assert!(ConfigFile::initial_mounts(&config, dir).is_empty());
            assert!(!file.first_start_due());
            assert!(
                file.save(&library(MountPlace::Default)).is_err(),
                "a read-only file is never saved"
            );
            assert_eq!(file.view(), (mounts, health), "the state is kept");
        }
        assert_eq!(
            std::fs::read_to_string(newer.path().join(LIBRARY_CONFIG_FILE)).unwrap(),
            newer_text
        );
        assert!(unreadable.path().join(LIBRARY_CONFIG_FILE).is_dir());
        assert!(
            load(newer.path())
                .load_why()
                .unwrap()
                .contains("schema_version 3"),
            "the why names the newer version"
        );
    }

    #[test]
    fn a_save_over_a_missing_file_writes_the_word_and_makes_the_file_ours() {
        let temp = tempfile::tempdir().unwrap();
        let file = ConfigFile::new(&load(temp.path()));
        file.save(&[MountSetting::first_start()]).unwrap();
        assert_eq!(
            on_disk(temp.path()),
            Some(
                json!({ "schema_version": 2, "mounts": [{ "name": "default_library", "path": "default" }] })
            )
        );
        assert_eq!(
            file.view(),
            (Some(vec![MountSetting::first_start()]), ConfigHealth::Ok)
        );
        assert!(!file.first_start_due());
        let root = abs(temp.path(), "lib");
        file.save(&library(MountPlace::Explicit(root.clone())))
            .unwrap();
        assert_eq!(
            on_disk(temp.path()).unwrap()["mounts"][0]["path"],
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
        file.save(&library(MountPlace::Explicit(new.clone())))
            .unwrap();
        assert_eq!(
            on_disk(temp.path()).unwrap()["mounts"][0]["path"],
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
        file.save(&library(MountPlace::Default)).unwrap();
        assert_eq!(
            std::fs::read_to_string(temp.path().join("library.json.corrupt")).unwrap(),
            "not json"
        );
        assert_eq!(
            on_disk(temp.path()),
            Some(
                json!({ "schema_version": 2, "mounts": [{ "name": "library", "path": "default" }] })
            )
        );
        assert_eq!(
            file.view(),
            (Some(library(MountPlace::Default)), ConfigHealth::Ok)
        );
    }

    #[test]
    fn a_usable_file_that_appeared_after_a_missing_load_is_copied_aside_before_the_first_write() {
        let temp = tempfile::tempdir().unwrap();
        let file = ConfigFile::new(&load(temp.path()));
        put(temp.path(), &explicit_doc(&abs(temp.path(), "theirs")));
        file.save(&library(MountPlace::Default)).unwrap();
        assert!(temp.path().join("library.json.corrupt").exists());
    }

    #[test]
    fn a_save_judges_the_file_again_and_refuses_a_newer_one_that_appeared_leaving_it_unsaved() {
        let temp = tempfile::tempdir().unwrap();
        let file = ConfigFile::new(&load(temp.path()));
        let newer = r#"{"schema_version": 9, "mounts": []}"#;
        put(temp.path(), newer);
        let why = file.save(&library(MountPlace::Default)).unwrap_err();
        assert!(why.contains("newer"), "{why}");
        assert_eq!(
            std::fs::read_to_string(temp.path().join(LIBRARY_CONFIG_FILE)).unwrap(),
            newer
        );
        assert_eq!(
            file.view(),
            (
                Some(library(MountPlace::Default)),
                ConfigHealth::Unsaved { why }
            )
        );
        assert!(!file.first_start_due(), "a mount was published");
    }

    #[test]
    fn a_failed_save_leaves_the_file_as_it_was_and_is_unsaved_until_a_save_succeeds() {
        let temp = tempfile::tempdir().unwrap();
        put(temp.path(), "not json");
        let file = ConfigFile::new(&load(temp.path()));
        let mounts = library(MountPlace::Explicit(abs(temp.path(), "lib")));
        file.fail_writes(Some("disk full"));
        let why = file.save(&mounts).unwrap_err();
        assert!(why.contains("disk full"), "{why}");
        assert_eq!(
            std::fs::read_to_string(temp.path().join(LIBRARY_CONFIG_FILE)).unwrap(),
            "not json",
            "the file is as it was"
        );
        assert_eq!(
            file.view(),
            (Some(mounts.clone()), ConfigHealth::Unsaved { why }),
            "unsaved wins over corrupt and names the live setting"
        );
        assert!(file.read_only().is_none(), "the next save may write");
        file.fail_writes(None);
        file.save(&mounts).unwrap();
        assert_eq!(file.view(), (Some(mounts), ConfigHealth::Ok));
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
