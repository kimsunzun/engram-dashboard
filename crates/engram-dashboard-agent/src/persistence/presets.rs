//! Preset persistence — the `presets.json` store. Rules and implementation are the same internal store as `agents.json` (module head —
//! [`super`]). What this file holds is only the file name, version key, cap and list field. (ADR-0061)
// ADR-0291

use std::path::PathBuf;

use engram_dashboard_base::file;
use serde::Serialize;
use serde_json::{Map, Value};

use super::{list_shape, FileKind, FileStore};
use crate::preset::{Preset, PresetStore};
use crate::profile::{StoreError, StoreStatus};

/// File name of the preset store — the text that tells the caller of a save refusal or failure names this file.
pub const PRESETS_FILE: &str = "presets.json";

const PRESETS: FileKind = FileKind {
    name: PRESETS_FILE,
    legacy_tmp: "presets.json.tmp",
    list_key: "presets",
    spec: file::Spec {
        version_key: "schema_version",
        current: 1,
        // ADR-0291 R6: a smaller unit than profiles (path bookmarks), so the cap is smaller too — a line never reached in normal use.
        cap: 4 * 1024 * 1024,
        shape: presets_shape,
    },
};

/// Write-side disk representation — always carries the version key.
#[derive(Serialize)]
struct PresetFile<'a> {
    schema_version: u64,
    presets: &'a [Preset],
}

pub struct FilePresetStore {
    inner: FileStore,
}

impl FilePresetStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            inner: FileStore::new(dir, PRESETS),
        }
    }
}

impl PresetStore for FilePresetStore {
    fn save(&self, presets: &[Preset]) -> Result<(), StoreError> {
        self.inner.save(
            presets.len(),
            &PresetFile {
                schema_version: PRESETS.spec.current,
                presets,
            },
        )
    }

    fn load(&self) -> Vec<Preset> {
        self.inner.load()
    }

    fn status(&self) -> StoreStatus {
        self.inner.status()
    }
}

fn presets_shape(version: u64, doc: &Map<String, Value>) -> Result<(), String> {
    list_shape::<Preset>(&PRESETS, version, doc)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::profile::Refusal;
    use uuid::Uuid;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("engram-preset-persist-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn sample() -> Preset {
        Preset {
            id: Uuid::new_v4(),
            cwd: PathBuf::from("."),
            name: None,
        }
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = temp_dir("roundtrip");
        let store = FilePresetStore::new(dir.clone());
        let p = sample();
        let id = p.id;
        store.save(&[p]).unwrap();

        let loaded = store.load();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, id);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_missing_is_empty() {
        let dir = temp_dir("missing");
        let store = FilePresetStore::new(dir.clone());
        assert!(store.load().is_empty());
    }

    fn put(dir: &std::path::Path, text: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(PRESETS.name), text).unwrap();
    }

    fn aside(dir: &std::path::Path) -> PathBuf {
        dir.join(format!("{}.corrupt", PRESETS.name))
    }

    #[test]
    fn a_corrupt_file_stays_in_place_until_the_first_save_copies_it_aside() {
        let dir = temp_dir("corrupt");
        put(&dir, "{ not valid json");

        let store = FilePresetStore::new(dir.clone());
        assert!(store.load().is_empty());
        assert!(dir.join(PRESETS.name).exists());
        assert!(!aside(&dir).exists());

        store.save(&[sample()]).unwrap();
        assert_eq!(fs::read_to_string(aside(&dir)).unwrap(), "{ not valid json");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_newer_file_is_neither_read_nor_overwritten() {
        let dir = temp_dir("newer");
        let newer = r#"{"schema_version":999,"presets":[]}"#;
        put(&dir, newer);

        let store = FilePresetStore::new(dir.clone());
        assert!(store.load().is_empty());
        let refusal = Refusal::Newer { found: 999 };
        assert_eq!(store.status(), StoreStatus::ReadOnly(refusal));
        assert!(matches!(
            store.save(&[sample()]),
            Err(StoreError::ReadOnly(r)) if r == refusal
        ));
        assert_eq!(fs::read_to_string(dir.join(PRESETS.name)).unwrap(), newer);
        assert!(!aside(&dir).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_over_the_cap_is_corrupt() {
        let dir = temp_dir("cap");
        let padding = " ".repeat(PRESETS.spec.cap as usize);
        put(
            &dir,
            &format!(r#"{{"schema_version":1,"presets":[]{padding}}}"#),
        );

        let store = FilePresetStore::new(dir.clone());
        assert!(store.load().is_empty());
        assert_eq!(store.status(), StoreStatus::Writable);
        store.save(&[sample()]).unwrap();
        assert!(aside(&dir).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
