//! 프리셋 영속화 — `presets.json` atomic 저장/복원. (ADR-0061)
//!
//! `persistence::mod`(FileProfileStore)의 프리셋판 — atomic write·버전체크·손상보존 전략을
//! **그대로 복제**한다(새 전략 발명 금지, ADR-0061 근거: 검증된 프로필 경로 재사용). tauri import 0.
//! 저장 위치(dir)는 상위 층(daemon lib.rs)이 `.engram-data/`(ADR-0024)로 주입하고, headless 테스트는
//! 임시 디렉토리를 주입한다. `PresetStore` trait 을 구현해 `PresetRegistry` 에 끼워진다.
//!
//! **atomic 보장:** 같은 디렉토리에 tmp 를 쓰고 `sync_all` 후 `rename` 한다. 같은 파일시스템 내
//! rename 이라 교체가 원자적이고, 크래시가 나도 presets.json 은 완전한 옛/새 내용 둘 중 하나다(반쪽 쓰기 없음).

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::agent::preset::{Preset, PresetStore};

/// 파일 포맷 버전. 구조가 바뀌면 올린다. 로드 시 불일치하면 적재하지 않는다(마이그레이션 게이트).
const SCHEMA_VERSION: u32 = 1;
const FILE_NAME: &str = "presets.json";
const TMP_NAME: &str = "presets.json.tmp";

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 디스크 표현 — 프리셋 목록 앞에 schema_version 을 붙여 버전 진화에 대비한다(ProfilesFile 미러).
#[derive(Serialize, Deserialize)]
struct PresetFile {
    schema_version: u32,
    presets: Vec<Preset>,
}

/// 파일 기반 PresetStore. 단일 디렉토리에 presets.json 하나를 관리한다(FileProfileStore 미러).
pub struct FilePresetStore {
    dir: PathBuf,
    /// 동시 save 직렬화 — tmp 파일명이 고정이라 병행 쓰기가 겹치면 안 된다.
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

    /// tmp → sync_all → rename → parent fsync. 실패는 io::Error 로 올린다(FileProfileStore 와 동일 순서).
    fn write_atomic(&self, presets: &[Preset]) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;

        let payload = PresetFile {
            schema_version: SCHEMA_VERSION,
            presets: presets.to_vec(),
        };
        let json = serde_json::to_vec_pretty(&payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        // 1) 같은 디렉토리 tmp 에 전체를 쓰고 디스크까지 flush(sync_all = 데이터+메타데이터).
        let tmp = self.dir.join(TMP_NAME);
        {
            let mut f = File::create(&tmp)?;
            f.write_all(&json)?;
            f.sync_all()?;
        }

        // 2) atomic rename 으로 교체. 같은 디렉토리라 크로스 파일시스템 오류는 발생하지 않는다.
        fs::rename(&tmp, self.path())?;

        // 3) parent 디렉토리 fsync — rename(디렉토리 엔트리 변경)을 영속화.
        //    Windows 에선 디렉토리 핸들 fsync 지원이 제한적이라 best-effort 로 둔다(실패 무시).
        if let Ok(dir) = File::open(&self.dir) {
            let _ = dir.sync_all();
        }
        Ok(())
    }

    /// 손상 파일을 `.corrupt-<ts>` 로 보존(덮어쓰기 방지). 파싱 불가일 때만 호출한다.
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
            // 저장 실패는 치명적이지 않게 로그만 — 상위 동작을 막지 않는다(FileProfileStore 와 동일).
            tracing::error!("save_presets 실패: {e}");
        } else {
            tracing::debug!(count = presets.len(), "프리셋 저장 완료");
        }
    }

    fn load(&self) -> Vec<Preset> {
        let path = self.path();
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            // 첫 실행이면 파일이 없는 게 정상 — 빈 목록.
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => {
                tracing::warn!("presets.json 읽기 실패: {e} — 빈 목록으로 시작");
                return Vec::new();
            }
        };

        match serde_json::from_slice::<PresetFile>(&bytes) {
            Ok(f) if f.schema_version == SCHEMA_VERSION => f.presets,
            // 버전 불일치: 더 새/옛 포맷일 수 있으니 **파괴하지 않고** 적재만 건너뛴다.
            Ok(f) => {
                tracing::warn!(
                    found = f.schema_version,
                    expected = SCHEMA_VERSION,
                    "presets.json schema_version 불일치 — 적재 건너뜀(파일 보존)"
                );
                Vec::new()
            }
            // 파싱 불가(진짜 손상): .corrupt 로 보존 후 빈 목록.
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
        let _ = fs::remove_dir_all(&dir); // 이전 실행 잔여 정리
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

        // 원본은 .corrupt-* 로 보존, presets.json 은 사라짐.
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
        // 버전 불일치는 파괴하지 않음 — 파일 그대로 유지.
        assert!(dir.join(FILE_NAME).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
