//! 프로필 영속화 — `agents.json` atomic 저장/복원.
//!
//! **atomic 보장(H-1.3):** 같은 디렉토리에 tmp를 쓰고 `sync_all` 후 `rename`한다.
//! 같은 파일시스템 내 rename이라 교체가 원자적이고, 크래시가 나도 agents.json은
//! 완전한 옛 내용이거나 완전한 새 내용 둘 중 하나다(반쪽 쓰기 없음).

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::agent::profile::{AgentProfile, ProfileStore};

pub mod presets;
pub use presets::FilePresetStore;

/// 파일 포맷 버전. 구조가 바뀌면 올린다. 로드 시 불일치하면 적재하지 않는다(마이그레이션 게이트).
const SCHEMA_VERSION: u32 = 1;
const FILE_NAME: &str = "agents.json";
const TMP_NAME: &str = "agents.json.tmp";

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 디스크 표현.
#[derive(Serialize, Deserialize)]
struct ProfilesFile {
    schema_version: u32,
    profiles: Vec<AgentProfile>,
}

pub struct FileProfileStore {
    dir: PathBuf,
    /// 동시 save 직렬화 — tmp 파일명이 고정이라 병행 쓰기가 겹치면 안 된다.
    write_lock: Mutex<()>,
}

impl FileProfileStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            write_lock: Mutex::new(()),
        }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    fn write_atomic(&self, profiles: &[AgentProfile]) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;

        let payload = ProfilesFile {
            schema_version: SCHEMA_VERSION,
            profiles: profiles.to_vec(),
        };
        let json = serde_json::to_vec_pretty(&payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let tmp = self.dir.join(TMP_NAME);
        {
            let mut f = File::create(&tmp)?;
            f.write_all(&json)?;
            f.sync_all()?;
        }

        // 같은 디렉토리라 크로스 파일시스템 오류는 발생하지 않는다.
        fs::rename(&tmp, self.path())?;

        // parent 디렉토리 fsync — rename(디렉토리 엔트리 변경)을 영속화.
        // Windows에선 디렉토리 핸들 fsync 지원이 제한적이라 best-effort로 둔다(실패 무시).
        if let Ok(dir) = File::open(&self.dir) {
            let _ = dir.sync_all();
        }
        Ok(())
    }

    /// 손상 파일을 `.corrupt-<ts>`로 보존(덮어쓰기 방지).
    fn preserve_corrupt(&self, path: &Path) {
        let backup = self
            .dir
            .join(format!("{FILE_NAME}.corrupt-{}", now_millis()));
        match fs::rename(path, &backup) {
            Ok(()) => tracing::warn!("손상된 agents.json을 {:?}로 보존", backup),
            Err(e) => tracing::error!("corrupt 파일 보존 실패: {e}"),
        }
    }
}

impl ProfileStore for FileProfileStore {
    fn save(&self, profiles: &[AgentProfile]) {
        warn_if_secret(profiles);

        let _guard = self.write_lock.lock().expect("write_lock poisoned");
        if let Err(e) = self.write_atomic(profiles) {
            tracing::error!("save_profiles 실패: {e}");
        } else {
            tracing::debug!(count = profiles.len(), "프로필 저장 완료");
        }
    }

    fn load(&self) -> Vec<AgentProfile> {
        let path = self.path();
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => {
                tracing::warn!("agents.json 읽기 실패: {e} — 빈 목록으로 시작");
                return Vec::new();
            }
        };

        match serde_json::from_slice::<ProfilesFile>(&bytes) {
            Ok(f) if f.schema_version == SCHEMA_VERSION => f.profiles,
            Ok(f) => {
                tracing::warn!(
                    found = f.schema_version,
                    expected = SCHEMA_VERSION,
                    "agents.json schema_version 불일치 — 적재 건너뜀(파일 보존)"
                );
                Vec::new()
            }
            Err(e) => {
                tracing::error!("agents.json 파싱 실패: {e} — .corrupt 보존 후 빈 목록");
                self.preserve_corrupt(&path);
                Vec::new()
            }
        }
    }
}

/// env에 자격증명으로 보이는 키가 있으면 경고(보안). persist를 막지는 않되 평문 저장 위험을
/// 로그로 알린다. 이상적으론 시크릿 제외 목록이지만, 우선 가시화부터.
fn warn_if_secret(profiles: &[AgentProfile]) {
    const NEEDLES: [&str; 4] = ["KEY", "TOKEN", "SECRET", "PASSWORD"];
    for p in profiles {
        for (k, _) in &p.env {
            let upper = k.to_uppercase();
            if NEEDLES.iter().any(|n| upper.contains(n)) {
                tracing::warn!(
                    agent = %p.id,
                    env_key = %k,
                    "프로필 env에 자격증명으로 보이는 키 — agents.json에 평문 저장됨. 자격증명은 env에 넣지 말 것."
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::profile::AgentCommand;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("engram-persist-test-{name}"));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn sample() -> AgentProfile {
        AgentProfile::new(
            "t".into(),
            AgentCommand::Shell {
                program: "cmd.exe".into(),
                args: vec![],
            },
            PathBuf::from("."),
            vec![],
            true,
        )
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = temp_dir("roundtrip");
        let store = FileProfileStore::new(dir.clone());
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
        let store = FileProfileStore::new(dir.clone());
        assert!(store.load().is_empty());
    }

    #[test]
    fn corrupt_is_preserved_and_empty() {
        let dir = temp_dir("corrupt");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE_NAME), b"{ not valid json").unwrap();

        let store = FileProfileStore::new(dir.clone());
        assert!(store.load().is_empty());

        assert!(!dir.join(FILE_NAME).exists());
        let has_backup = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains(".corrupt-"));
        assert!(has_backup, "손상 파일이 .corrupt로 보존돼야 함");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_mismatch_keeps_file() {
        let dir = temp_dir("version");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(FILE_NAME),
            br#"{"schema_version":999,"profiles":[]}"#,
        )
        .unwrap();

        let store = FileProfileStore::new(dir.clone());
        assert!(store.load().is_empty());
        assert!(dir.join(FILE_NAME).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
