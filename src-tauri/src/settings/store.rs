//! The only place that knows the settings file (`<셸 config 폴더>\settings.json`) — name · location · format · tolerance rules (TRD §5-3).
//!
//! Format = a flat dotted-key object + the reserved key `"$version": 1`. **Only values that differ from the default are written.**
//!
//! - **Load ([`load`]) neither creates nor edits the file** — it runs before the shell build (TRD §5-3 service lifetime). The logger is
//!   also later than that, so the load emits no logs and collects them into [`LoadNote`] — the service emits them after the logger is up.
//! - **Write ([`write`]) is read-modify-write** — it changes only the keys received, in the document the caller read ([`read_document`]).
//!   Other keys edited from outside the app and unknown keys stay as they are. It is called only under the service's write serialization lock — if the
//!   read-modify-write of two writes overlaps, the later write erases the earlier write's change.
//! - **The read decision · whether it may be overwritten is one base `file` rule** — this file's share is only [`SPEC`] (version key · version · upper bound).
//! - **A file unusable as a whole** (not JSON · not an object · over the upper bound · not UTF-8 · wrong `$version` shape) is collapsed to
//!   the defaults by the load and left untouched. The first write on top of it **copies aside** the original as `settings.json.corrupt` and then atomically swaps that
//!   place — there is no moment with no file. The new file is rebuilt from the values in memory. There is only one
//!   copy ([`SettingsFiles::copy_aside`]). If copying aside fails, that write fails too and the original stays as it is.
//!   ★An IO failure of the read itself is not in that group★ — so as not to overwrite a healthy file that is briefly locked, the write fails then.
//! - **A file written by a version newer than this shell** (`$version` greater than 1) is also collapsed to the defaults by the load, but ★it is neither overwritten nor copied
//!   aside★ — a write on top of it is rejected ([`WriteError::Newer`]). It reads again at every write, so it clears when the file changes.
//! - **An unusable value of a known key** is collapsed to the default — in the file it stays as is until that key is written or reverted.
// ADR-0265
// ADR-0291: no version = version 1 · a file written by a newer version is not overwritten · the decision is redone at every write.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use engram_dashboard_base::file::{self, Loaded, Parsed, Refused, WritePolicy};
use serde_json::{Map, Value};

use super::registry::{self, SETTINGS};
use crate::file_hooks::OS_HOOKS;

const SETTINGS_FILE: &str = "settings.json";
const VERSION_KEY: &str = "$version";
const VERSION: u64 = 1;
/// The line up to which things are still normal even if a person grows it by hand — the real size is around 1 KiB. ★Writes do not exceed this line either★ —
/// if text over it is written, the next load collapses the whole file as unusable.
const MAX_SETTINGS_BYTES: u64 = 64 * 1024;

/// Read rule of the settings file — the shape is not looked at separately (tolerant per key — an unusable value defaults only that key).
// ADR-0291
const SPEC: file::Spec = file::Spec {
    version_key: VERSION_KEY,
    current: VERSION,
    cap: MAX_SETTINGS_BYTES,
    shape: file::any_shape,
};

/// IO seam for one settings file — tests run with an in-memory fake (`fake::MemFiles`).
///
/// ★Called while holding the service's write serialization lock★ — if an implementation calls a write of [`super::SettingsService`] again, it is a
/// deadlock (reads do not take that lock).
// ADR-0012
pub trait SettingsFiles: Send {
    /// Original text — same contract as [`file::read_file_capped`]: `NotFound` if missing · over the upper bound · not UTF-8 is
    /// `InvalidData` (an unusable file) · anything else is an IO failure of the read itself (permission · after lock retries, etc.). In the last case the write
    /// fails without overwriting the file.
    fn read(&self) -> io::Result<String>;
    /// Atomically swap the whole file. Create the folder if it is missing.
    fn write_atomic(&self, text: &str) -> io::Result<()>;
    /// **Copy aside** the current file to a neighboring name (`<이름>.corrupt`) and return that place — the original is left in that place as it is.
    /// ★It overwrites a copy made aside earlier★ (there is only one name — [`file::copy_aside`]). On `Err` the earlier copy is also left as it is.
    fn copy_aside(&self) -> io::Result<PathBuf>;
    /// Origin (path) to put in the log.
    fn origin(&self) -> String;
}

/// Production implementation — `<dir>\settings.json`.
pub struct FsSettingsFiles {
    path: PathBuf,
}

impl FsSettingsFiles {
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            path: dir.join(SETTINGS_FILE),
        }
    }
}

impl SettingsFiles for FsSettingsFiles {
    fn read(&self) -> io::Result<String> {
        file::read_file_capped(&self.path, SPEC.cap, OS_HOOKS)
    }

    // Nobody creates the shell config folder in advance — the load must not create it, so the first write creates it (`file::write_atomic`).
    fn write_atomic(&self, text: &str) -> io::Result<()> {
        file::write_atomic(&self.path, text.as_bytes(), OS_HOOKS)
    }

    fn copy_aside(&self) -> io::Result<PathBuf> {
        file::copy_aside(&self.path, OS_HOOKS)
    }

    fn origin(&self) -> String {
        self.path.display().to_string()
    }
}

/// What was decided by reading the file once — exactly the base decision ([`file::classify`]).
#[derive(Debug)]
pub(super) struct Document(Loaded);

impl Document {
    /// Does the file's `key` entry differ from the target — target `None` = that key must be absent (default) · `Some(v)` = must be the canonical string
    /// `v`. A value that differs only in spelling (`"LIGHT"`) also differs. `None` = cannot tell (could not read, or a file unusable as a whole ·
    /// a file written by a newer version).
    pub(super) fn differs(&self, key: &str, target: Option<&str>) -> Option<bool> {
        match &self.0 {
            Loaded::Missing => Some(target.is_some()),
            Loaded::Parsed(Parsed::Usable { doc, .. }) => Some(match (doc.get(key), target) {
                (None, None) => false,
                (Some(Value::String(found)), Some(target)) => found != target,
                _ => true,
            }),
            Loaded::Parsed(Parsed::Unusable(_) | Parsed::Newer { .. }) | Loaded::Failed(_) => None,
        }
    }
}

/// Reads the file once and decides — no version = version 1 · `1.0` is also version 1 · one BOM is ignored (base `file::parse`).
pub(super) fn read_document(files: &dyn SettingsFiles) -> Document {
    Document(file::classify(files.read(), &SPEC))
}

/// Only values that differ from the default (canonical form) — key = the table's key.
pub(super) type Overrides = BTreeMap<&'static str, String>;

/// One log line the load leaves — emitted at the level · with the fields that [`LoadNote::emit`] would have emitted at load time. ★The value is not included★ —
/// it is a file written from outside, so we do not decide what it contains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LoadNote {
    Missing,
    /// A file unusable as a whole — reason message.
    Unusable(String),
    /// A file written by a version newer than this shell — that version.
    Newer {
        found: u64,
    },
    /// An IO failure of the read itself — error message.
    ReadFailed(String),
    /// An unusable value of a known key — reason message (the `Err` of [`registry::SettingDef::normalize`] does not include the value).
    BadValue {
        key: &'static str,
        reason: String,
    },
    /// Finished reading — number of overridden keys · number of unknown keys (unknown keys do not include the name either).
    Loaded {
        overrides: usize,
        unknown: usize,
    },
}

impl LoadNote {
    /// `origin` = [`SettingsFiles::origin`].
    pub(super) fn emit(&self, origin: &str) {
        match self {
            LoadNote::Missing => tracing::debug!(
                module = "settings",
                source = %origin,
                "설정 파일이 없어 기본값으로 둔다"
            ),
            LoadNote::Unusable(reason) => tracing::error!(
                module = "settings",
                source = %origin,
                "설정 파일을 통째로 못 써 기본값으로 둔다(첫 쓰기가 옆에 떠 둔 뒤 새로 쓴다): {reason}"
            ),
            LoadNote::Newer { found } => tracing::error!(
                module = "settings",
                source = %origin,
                found = *found,
                current = VERSION,
                "설정 파일이 이 셸보다 새 판이 쓴 것이라 기본값으로 둔다 — 덮지 않으며 설정 바꾸기는 오류로 돌아간다"
            ),
            LoadNote::ReadFailed(error) => tracing::warn!(
                module = "settings",
                source = %origin,
                "설정 파일을 못 읽어 기본값으로 둔다: {error}"
            ),
            LoadNote::BadValue { key, reason } => tracing::warn!(
                module = "settings",
                source = %origin,
                key = *key,
                "설정 값을 못 써 기본값으로 둔다(파일은 덮일 때까지 그대로): {reason}"
            ),
            LoadNote::Loaded { overrides, unknown } => tracing::debug!(
                module = "settings",
                source = %origin,
                overrides = *overrides,
                unknown = *unknown,
                "설정을 읽었다"
            ),
        }
    }
}

/// File → values that differ from the default + logs to leave. ★Every failure is collapsed to the defaults★ (no panic · no propagation) · it does not touch the
/// file · it does not emit logs (module header).
pub(super) fn load(files: &dyn SettingsFiles) -> (Overrides, Vec<LoadNote>) {
    let map = match read_document(files).0 {
        Loaded::Parsed(Parsed::Usable { doc, .. }) => doc,
        Loaded::Missing => return (Overrides::new(), vec![LoadNote::Missing]),
        Loaded::Parsed(Parsed::Unusable(reason)) => {
            return (Overrides::new(), vec![LoadNote::Unusable(reason)])
        }
        Loaded::Parsed(Parsed::Newer { found }) => {
            return (Overrides::new(), vec![LoadNote::Newer { found }])
        }
        Loaded::Failed(e) => return (Overrides::new(), vec![LoadNote::ReadFailed(e.to_string())]),
    };

    let mut overrides = Overrides::new();
    let mut notes = Vec::new();
    for def in SETTINGS {
        let Some(raw) = map.get(def.key) else {
            continue;
        };
        let normalized = match raw.as_str() {
            Some(text) => def.normalize(text),
            None => Err("문자열이 아니다".to_string()),
        };
        match normalized {
            Ok(value) if value == def.default => {}
            Ok(value) => {
                overrides.insert(def.key, value);
            }
            Err(reason) => notes.push(LoadNote::BadValue {
                key: def.key,
                reason,
            }),
        }
    }
    let unknown = map
        .keys()
        .filter(|key| key.as_str() != VERSION_KEY && registry::find(key).is_none())
        .count();
    notes.push(LoadNote::Loaded {
        overrides: overrides.len(),
        unknown,
    });
    (overrides, notes)
}

/// Why [`write`] did not write that file.
#[derive(Debug)]
pub(super) enum WriteError {
    /// It is a file written by a version newer than this shell — it was neither overwritten nor copied aside.
    Newer { found: u64 },
    /// Could not read the current file (after the lock budget · permission) — it was neither overwritten nor copied aside, since we do not know what it contains.
    Unreadable(io::Error),
    /// Copying aside or writing failed · the text built exceeds the read upper bound.
    Io(io::Error),
}

impl From<io::Error> for WriteError {
    fn from(error: io::Error) -> Self {
        WriteError::Io(error)
    }
}

/// Rewrites changing only the keys received — `Some` = write that value · `None` = remove that key (default).
///
/// `document` = what was read with [`read_document`] just before this write — whether it may be overwritten is decided by the base rule
/// ([`Loaded::write_policy`]) from it. If the file is missing or unusable as a whole, the new file is built from `memory` (all currently effective
/// overrides) — otherwise one write would delete from the disk another value still visible on screen. A file unusable as a whole is
/// copied aside before being swapped ([`SettingsFiles::copy_aside`]). A file written by a newer version · a file that could not be read is neither overwritten nor
/// copied aside.
///
/// On `Err` the file stays as it is. ★If the failure is after copying aside, the copy made aside earlier has already been overwritten by this original★ (there is only one
/// copy name). If the text built exceeds the read upper bound, it neither copies aside nor writes and gives `Err`.
/// ★One call = one file (TRD §5-2)★ — if keys come to split across several files, a bundle that spans them is rejected here.
// ADR-0265
// ADR-0291 R8 · R16
pub(super) fn write(
    files: &dyn SettingsFiles,
    document: Document,
    memory: &Overrides,
    changes: &[(&'static str, Option<String>)],
) -> Result<(), WriteError> {
    // Settings does not overwrite that file as a whole but puts only the changed keys on top of the current file, so it always counts it as this run's.
    let copy_aside_first = match document.0.write_policy(file::Claim::Adopted) {
        WritePolicy::Write => false,
        WritePolicy::CopyAsideFirst => true,
        WritePolicy::Refuse(Refused::Newer { found }) => return Err(WriteError::Newer { found }),
        // Rejection — that read error is carried and returned by the `Loaded::Failed` branch below.
        WritePolicy::Refuse(Refused::ReadFailed) => false,
    };
    let from_memory = || -> Map<String, Value> {
        memory
            .iter()
            .map(|(key, value)| ((*key).to_string(), Value::String(value.clone())))
            .collect()
    };
    let (mut map, unusable) = match document.0 {
        Loaded::Parsed(Parsed::Usable { doc, .. }) => (doc, None),
        Loaded::Failed(e) => return Err(WriteError::Unreadable(e)),
        Loaded::Missing | Loaded::Parsed(Parsed::Newer { .. }) => (from_memory(), None),
        Loaded::Parsed(Parsed::Unusable(reason)) => (from_memory(), Some(reason)),
    };
    for (key, value) in changes {
        match value {
            Some(value) => map.insert((*key).to_string(), Value::String(value.clone())),
            None => map.remove(*key),
        };
    }
    map.insert(VERSION_KEY.to_string(), Value::from(VERSION));
    let mut text = serde_json::to_string_pretty(&Value::Object(map))
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    // It is a file people also edit by hand, so the line ending is kept.
    text.push('\n');
    file::check_cap(text.len(), &SPEC)?;
    if copy_aside_first {
        let to = files.copy_aside()?;
        tracing::warn!(
            module = "settings",
            source = %files.origin(),
            copied_to = %to.display(),
            "못 쓰는 설정 파일을 옆에 떠 두고(앞선 사본이 있었으면 덮었다) 그 자리에 새로 쓴다: {}",
            unusable.as_deref().unwrap_or_default()
        );
    }
    Ok(files.write_atomic(&text)?)
}

/// In-memory fake disk — the test hands it to the service while holding the handle.
#[cfg(test)]
pub(super) mod fake {
    use std::io;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, MutexGuard};

    use super::SettingsFiles;

    #[derive(Default)]
    pub struct Disk {
        /// Current file text. `None` = missing.
        pub text: Option<String>,
        /// The text cannot be read (imitating an exceeded upper bound) — looked at before `text`.
        pub unusable: bool,
        pub fail_read: bool,
        pub fail_write: bool,
        pub fail_copy: bool,
        pub writes: usize,
        /// Text of the place of the copy made aside (`memory.corrupt` — there is only one, so it is overwritten each time one is made). `None` = no copy.
        pub corrupt: Option<String>,
        /// Number of times copied aside.
        pub copies: usize,
    }

    #[derive(Clone, Default)]
    pub struct MemFiles(Arc<Mutex<Disk>>);

    impl MemFiles {
        pub fn with_text(text: &str) -> Self {
            let files = Self::default();
            files.disk().text = Some(text.to_string());
            files
        }

        pub fn disk(&self) -> MutexGuard<'_, Disk> {
            self.0.lock().unwrap()
        }

        pub fn text(&self) -> Option<String> {
            self.disk().text.clone()
        }

        pub fn json(&self) -> serde_json::Value {
            serde_json::from_str(&self.text().expect("파일이 있어야 한다")).expect("JSON")
        }
    }

    impl SettingsFiles for MemFiles {
        fn read(&self) -> io::Result<String> {
            let disk = self.disk();
            if disk.fail_read {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "가짜 읽기 실패",
                ));
            }
            if disk.unusable {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "가짜 상한 초과"));
            }
            disk.text
                .clone()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }

        fn write_atomic(&self, text: &str) -> io::Result<()> {
            let mut disk = self.disk();
            if disk.fail_write {
                return Err(io::Error::other("가짜 쓰기 실패"));
            }
            disk.text = Some(text.to_string());
            disk.unusable = false;
            disk.writes += 1;
            Ok(())
        }

        fn copy_aside(&self) -> io::Result<PathBuf> {
            let mut disk = self.disk();
            if disk.fail_copy {
                return Err(io::Error::other("가짜 사본 실패"));
            }
            let Some(text) = disk.text.clone() else {
                return Err(io::Error::from(io::ErrorKind::NotFound));
            };
            disk.corrupt = Some(text);
            disk.copies += 1;
            Ok(PathBuf::from("memory.corrupt"))
        }

        fn origin(&self) -> String {
            "memory".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::MemFiles;
    use super::*;

    fn overrides_of(files: &MemFiles) -> Overrides {
        load(files).0
    }

    fn write_doc(
        files: &MemFiles,
        changes: &[(&'static str, Option<String>)],
    ) -> Result<(), WriteError> {
        let document = read_document(files);
        write(files, document, &Overrides::new(), changes)
    }

    // ── load ──

    #[test]
    fn a_missing_file_loads_as_defaults() {
        let files = MemFiles::default();
        let (overrides, notes) = load(&files);
        assert!(overrides.is_empty());
        assert_eq!(notes, vec![LoadNote::Missing]);
        assert_eq!(files.disk().writes, 0);
    }

    #[test]
    fn values_are_normalized_and_defaults_dropped_on_load() {
        let files = MemFiles::with_text(
            r#"{"$version":1,"theme.default":"LIGHT","chat.style.fontSize":"13px","chat.style.userPy":" 9PX "}"#,
        );
        let loaded = overrides_of(&files);
        assert_eq!(
            loaded.get("theme.default").map(String::as_str),
            Some("light")
        );
        assert_eq!(
            loaded.get("chat.style.userPy").map(String::as_str),
            Some("9px")
        );
        assert!(
            !loaded.contains_key("chat.style.fontSize"),
            "기본값과 같은 값은 덮어쓰기가 아니다"
        );
    }

    #[test]
    fn a_bad_known_value_folds_to_default_and_stays_in_the_file() {
        let original =
            r#"{"theme.default":"purple","chat.style.fontSize":99,"chat.style.userPy":"9px"}"#;
        let files = MemFiles::with_text(original);
        let (loaded, notes) = load(&files);
        assert!(!loaded.contains_key("theme.default"));
        assert!(!loaded.contains_key("chat.style.fontSize"));
        assert_eq!(
            loaded.get("chat.style.userPy").map(String::as_str),
            Some("9px")
        );
        assert_eq!(
            files.text().as_deref(),
            Some(original),
            "적재는 파일을 안 고친다"
        );
        let bad: Vec<&str> = notes
            .iter()
            .filter_map(|note| match note {
                LoadNote::BadValue { key, .. } => Some(*key),
                _ => None,
            })
            .collect();
        assert_eq!(bad, vec!["theme.default", "chat.style.fontSize"]);
        assert!(
            !format!("{notes:?}").contains("purple"),
            "값은 싣지 않는다: {notes:?}"
        );
    }

    #[test]
    fn unknown_keys_are_ignored_on_load_and_only_counted() {
        let files = MemFiles::with_text(r#"{"future.key":"x","theme.default":"light"}"#);
        let (loaded, notes) = load(&files);
        assert_eq!(loaded.len(), 1);
        assert_eq!(
            notes,
            vec![LoadNote::Loaded {
                overrides: 1,
                unknown: 1
            }]
        );
    }

    #[test]
    fn a_wholly_unusable_file_loads_as_defaults_and_is_left_alone() {
        for text in [
            "{not json",
            "[1,2]",
            "\"light\"",
            r#"{"$version":"1","theme.default":"light"}"#,
            r#"{"$version":0,"theme.default":"light"}"#,
            r#"{"$version":1.5,"theme.default":"light"}"#,
        ] {
            let files = MemFiles::with_text(text);
            let (overrides, notes) = load(&files);
            assert!(overrides.is_empty(), "{text}");
            assert!(
                matches!(notes.as_slice(), [LoadNote::Unusable(_)]),
                "{text}: {notes:?}"
            );
            let disk = files.disk();
            assert_eq!(disk.text.as_deref(), Some(text), "{text}");
            assert_eq!(disk.writes, 0);
            assert_eq!(disk.copies, 0, "적재는 떠 두지 않는다: {text}");
        }
        let files = MemFiles::default();
        files.disk().unusable = true;
        assert!(overrides_of(&files).is_empty());
        assert_eq!(files.disk().copies, 0);
    }

    /// ADR-0291 R2: a file written by a newer version is not read (defaults) · the load log carries that version · it is not touched.
    #[test]
    fn a_newer_version_file_loads_as_defaults_and_is_left_alone() {
        let text = r#"{"$version":2,"theme.default":"light"}"#;
        let files = MemFiles::with_text(text);
        let (overrides, notes) = load(&files);
        assert!(overrides.is_empty());
        assert_eq!(notes, vec![LoadNote::Newer { found: 2 }]);
        let disk = files.disk();
        assert_eq!(disk.text.as_deref(), Some(text));
        assert_eq!((disk.writes, disk.copies), (0, 0));
    }

    /// ADR-0291 R2 · R8: a write on top of a file written by a newer version is rejected — neither overwritten nor copied aside.
    #[test]
    fn a_write_over_a_newer_version_file_is_refused_without_a_copy() {
        for text in [
            r#"{"$version":2,"theme.default":"light"}"#,
            r#"{"$version":7.0}"#,
        ] {
            let files = MemFiles::with_text(text);
            let err = write_doc(&files, &[("theme.default", Some("e-ink".into()))]).unwrap_err();
            assert!(matches!(err, WriteError::Newer { .. }), "{text}: {err:?}");
            let disk = files.disk();
            assert_eq!(disk.text.as_deref(), Some(text), "{text}");
            assert_eq!((disk.writes, disk.copies), (0, 0), "{text}");
        }
    }

    #[test]
    fn a_read_failure_loads_as_defaults() {
        let files = MemFiles::with_text(r#"{"theme.default":"light"}"#);
        files.disk().fail_read = true;
        let (overrides, notes) = load(&files);
        assert!(overrides.is_empty());
        assert!(
            matches!(notes.as_slice(), [LoadNote::ReadFailed(_)]),
            "{notes:?}"
        );
    }

    #[test]
    fn a_leading_bom_is_not_a_reason_to_reject_the_file() {
        let files = MemFiles::with_text("\u{feff}{\"theme.default\":\"light\"}");
        assert_eq!(
            overrides_of(&files)
                .get("theme.default")
                .map(String::as_str),
            Some("light")
        );
    }

    #[test]
    fn version_one_written_as_a_float_is_version_one() {
        let files = MemFiles::with_text(r#"{"$version":1.0,"theme.default":"light"}"#);
        assert_eq!(
            overrides_of(&files)
                .get("theme.default")
                .map(String::as_str),
            Some("light")
        );
    }

    // ── document comparison ──

    #[test]
    fn differs_compares_the_raw_entry_with_the_target() {
        let usable = |text: &str| match read_document(&MemFiles::with_text(text)) {
            doc @ Document(Loaded::Parsed(Parsed::Usable { .. })) => doc,
            other => panic!("{other:?}"),
        };
        let doc = usable(r#"{"theme.default":"light","chat.style.fontSize":"bad","n":1}"#);
        assert_eq!(doc.differs("theme.default", Some("light")), Some(false));
        assert_eq!(doc.differs("theme.default", Some("e-ink")), Some(true));
        assert_eq!(doc.differs("theme.default", None), Some(true));
        assert_eq!(doc.differs("chat.style.fontSize", None), Some(true));
        assert_eq!(doc.differs("chat.style.userPy", None), Some(false));
        assert_eq!(doc.differs("n", Some("1")), Some(true), "문자열이 아니다");
        assert_eq!(
            usable(r#"{"theme.default":"LIGHT"}"#).differs("theme.default", Some("light")),
            Some(true),
            "정규 철자가 아니면 다르다"
        );
        assert_eq!(
            Document(Loaded::Missing).differs("theme.default", None),
            Some(false)
        );
        assert_eq!(
            Document(Loaded::Missing).differs("theme.default", Some("light")),
            Some(true)
        );
        assert_eq!(
            Document(Loaded::Parsed(Parsed::Unusable(String::new())))
                .differs("theme.default", None),
            None
        );
        assert_eq!(
            Document(Loaded::Parsed(Parsed::Newer { found: 2 })).differs("theme.default", None),
            None,
            "새 판이 쓴 파일은 가를 수 없다"
        );
    }

    // ── write ──

    #[test]
    fn a_write_creates_the_file_with_the_version_key() {
        let files = MemFiles::default();
        write_doc(&files, &[("theme.default", Some("light".into()))]).unwrap();
        assert_eq!(
            files.json(),
            serde_json::json!({"$version": 1, "theme.default": "light"})
        );
        assert!(files.text().unwrap().ends_with('\n'));
    }

    #[test]
    fn a_write_changes_only_its_key_and_keeps_unknown_and_other_keys() {
        let files = MemFiles::with_text(
            r#"{"$version":1,"future.key":[1,{"a":2}],"chat.style.userPy":"BAD","theme.default":"light"}"#,
        );
        write_doc(&files, &[("chat.style.fontSize", Some("15px".into()))]).unwrap();
        assert_eq!(
            files.json(),
            serde_json::json!({
                "$version": 1,
                "future.key": [1, {"a": 2}],
                "chat.style.userPy": "BAD",
                "theme.default": "light",
                "chat.style.fontSize": "15px",
            })
        );
    }

    #[test]
    fn a_none_change_removes_the_key() {
        let files = MemFiles::with_text(r#"{"$version":1,"theme.default":"light","x":1}"#);
        write_doc(&files, &[("theme.default", None)]).unwrap();
        assert_eq!(files.json(), serde_json::json!({"$version": 1, "x": 1}));
    }

    #[test]
    fn a_usable_file_is_not_reseeded_from_memory() {
        let files = MemFiles::with_text(r#"{"$version":1}"#);
        let memory = Overrides::from([("chat.style.userPy", "9px".to_string())]);
        let document = read_document(&files);
        write(
            &files,
            document,
            &memory,
            &[("theme.default", Some("light".into()))],
        )
        .unwrap();
        assert_eq!(
            files.json(),
            serde_json::json!({"$version": 1, "theme.default": "light"}),
            "멀쩡한 파일에서 밖이 지운 키를 되살리지 않는다"
        );
    }

    #[test]
    fn an_unusable_file_is_copied_aside_and_rebuilt_from_memory() {
        let files = MemFiles::with_text("{broken");
        let memory = Overrides::from([
            ("chat.style.userPy", "9px".to_string()),
            ("theme.default", "e-ink".to_string()),
        ]);
        let document = read_document(&files);
        write(
            &files,
            document,
            &memory,
            &[("theme.default", Some("light".into()))],
        )
        .unwrap();
        assert_eq!(files.disk().corrupt.as_deref(), Some("{broken"));
        assert_eq!(
            files.json(),
            serde_json::json!({"$version": 1, "theme.default": "light", "chat.style.userPy": "9px"})
        );
    }

    #[test]
    fn a_missing_file_is_rebuilt_from_memory() {
        let files = MemFiles::default();
        let memory = Overrides::from([("chat.style.userPy", "9px".to_string())]);
        write(
            &files,
            Document(Loaded::Missing),
            &memory,
            &[("theme.default", Some("light".into()))],
        )
        .unwrap();
        assert_eq!(
            files.json(),
            serde_json::json!({"$version": 1, "theme.default": "light", "chat.style.userPy": "9px"})
        );
        assert_eq!(files.disk().copies, 0, "없는 파일은 떠 둘 것이 없다");
    }

    #[test]
    fn a_failed_copy_fails_the_write_and_leaves_the_file() {
        let files = MemFiles::with_text("{broken");
        files.disk().fail_copy = true;
        write_doc(&files, &[("theme.default", Some("light".into()))]).unwrap_err();
        assert_eq!(files.text().as_deref(), Some("{broken"));
        assert_eq!(files.disk().writes, 0);
    }

    #[test]
    fn a_failed_replace_after_the_copy_leaves_the_original_in_place() {
        let files = MemFiles::with_text("{broken");
        files.disk().fail_write = true;
        write_doc(&files, &[("theme.default", Some("light".into()))]).unwrap_err();
        let disk = files.disk();
        assert_eq!(
            disk.text.as_deref(),
            Some("{broken"),
            "파일이 없는 순간이 없다"
        );
        assert_eq!(disk.corrupt.as_deref(), Some("{broken"));
    }

    #[test]
    fn every_write_over_a_still_broken_file_copies_it_onto_the_one_name() {
        let files = MemFiles::with_text("{broken");
        files.disk().fail_write = true;
        let attempt = || write_doc(&files, &[("theme.default", Some("light".into()))]);

        attempt().unwrap_err();
        attempt().unwrap_err();
        {
            let disk = files.disk();
            assert_eq!(disk.copies, 2);
            assert_eq!(disk.corrupt.as_deref(), Some("{broken"));
        }

        // It was edited from outside into another unusable text — the copy made aside earlier is overwritten (TRD §10 F21).
        files.disk().text = Some("{BROKEN".to_string());
        attempt().unwrap_err();
        assert_eq!(files.disk().corrupt.as_deref(), Some("{BROKEN"));
    }

    #[test]
    fn a_text_over_the_read_cap_is_never_written() {
        let pad = "x".repeat(MAX_SETTINGS_BYTES as usize - 20);
        let original = format!("{{\"pad\":\"{pad}\"}}");
        assert!(original.len() as u64 <= MAX_SETTINGS_BYTES);
        let files = MemFiles::with_text(&original);
        let err = write_doc(&files, &[("theme.default", Some("light".into()))]).unwrap_err();
        assert!(
            matches!(&err, WriteError::Io(e) if e.kind() == io::ErrorKind::InvalidData),
            "{err:?}"
        );
        let disk = files.disk();
        assert_eq!(disk.text.as_deref(), Some(original.as_str()));
        assert_eq!(disk.writes, 0);
    }
}
