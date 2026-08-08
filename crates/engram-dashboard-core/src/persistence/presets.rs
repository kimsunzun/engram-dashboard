//! 프리셋 영속화 — `presets.json` atomic 저장/복원. (ADR-0061)
//!
//! `persistence::mod`(FileProfileStore)의 프리셋판 — atomic write·버전체크·손상보존 전략을
//! **그대로 복제**한다(새 전략 발명 금지, ADR-0061 근거: 검증된 프로필 경로 재사용).
//!
//! **atomic 보장:** 크래시가 나도 presets.json 은 완전한 옛/새 내용 둘 중 하나다(반쪽 쓰기 없음).

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::agent::preset::{Preset, PresetStore};

const SCHEMA_VERSION: u32 = 1;
const FILE_NAME: &str = "presets.json";
const TMP_NAME: &str = "presets.json.tmp";

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 디스크 표현.
#[derive(Serialize, Deserialize)]
struct PresetFile {
    schema_version: u32,
    presets: Vec<Preset>,
}

pub struct FilePresetStore {
    dir: PathBuf,
    write_lock: Mutex<()>,
}

impl FilePresetStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            write_lock: Mutex::new(()),
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    fn write_atomic(&self, presets: &[Preset]) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;

        let payload = PresetFile {
            schema_version: SCHEMA_VERSION,
            presets: presets.to_vec(),
        };
        let json = serde_json::to_vec_pretty(&payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let tmp = self.dir.join(TMP_NAME);
        {
            let mut f = File::create(&tmp)?;
            f.write_all(&json)?;
            f.sync_all()?;
        }

        fs::rename(&tmp, self.path())?;

        if let Ok(dir) = File::open(&self.dir) {
            let _ = dir.sync_all();
        }
        Ok(())
    }

    fn preserve_corrupt(&self, path: &Path) {
        let backup = self
            .dir
            .join(format!("{FILE_NAME}.corrupt-{}", now_millis()));
        match fs::rename(path, &backup) {
            Ok(()) => tracing::warn!("손상된 presets.json 을 {:?} 로 보존", backup),
            Err(e) => tracing::error!("corrupt 파일 보존 실패: {e}"),
        }
    }
}

impl PresetStore for FilePresetStore {
    fn save(&self, presets: &[Preset]) {
        let _guard = self.write_lock.lock().expect("write_lock poisoned");
        if let Err(e) = self.write_atomic(presets) {
            tracing::error!("save_presets 실패: {e}");
        } else {
            tracing::debug!(count = presets.len(), "프리셋 저장 완료");
        }
    }

    fn load(&self) -> Vec<Preset> {
        let path = self.path();
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => {
                tracing::warn!("presets.json 읽기 실패: {e} — 빈 목록으로 시작");
                return Vec::new();
            }
        };

        match serde_json::from_slice::<PresetFile>(&bytes) {
            Ok(f) if f.schema_version == SCHEMA_VERSION => f.presets,
            Ok(f) => {
                tracing::warn!(
                    found = f.schema_version,
                    expected = SCHEMA_VERSION,
                    "presets.json schema_version 불일치 — 적재 건너뜀(파일 보존)"
                );
                Vec::new()
            }
            Err(e) => {
                tracing::error!("presets.json 파싱 실패: {e} — .corrupt 보존 후 빈 목록");
                self.preserve_corrupt(&path);
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        store.save(&[p]);

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

    #[test]
    fn corrupt_is_preserved_and_empty() {
        let dir = temp_dir("corrupt");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE_NAME), b"{ not valid json").unwrap();

        let store = FilePresetStore::new(dir.clone());
        assert!(store.load().is_empty());

        assert!(!dir.join(FILE_NAME).exists());
        let has_backup = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".corrupt-"));
        assert!(has_backup, "손상 파일이 .corrupt 로 보존돼야 함");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_mismatch_keeps_file() {
        let dir = temp_dir("version");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(FILE_NAME),
            br#"{"schema_version":999,"presets":[]}"#,
        )
        .unwrap();

        let store = FilePresetStore::new(dir.clone());
        assert!(store.load().is_empty());
        assert!(dir.join(FILE_NAME).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
