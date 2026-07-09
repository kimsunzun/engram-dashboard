//! 프로필 영속화 — `agents.json` atomic 저장/복원.
//!
//! tauri import 0. 저장 위치(dir)는 상위 층(lib.rs)이 앱 데이터 디렉토리로 주입하고,
//! headless 테스트는 임시 디렉토리를 주입한다. ProfileStore trait을 구현해
//! ProfileRegistry에 끼워진다.
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

// 프리셋 영속(presets.json) — 이 파일(agents.json)의 프리셋판(ADR-0061). 같은 atomic write·손상보존
// 전략을 복제한다. 호출부 단순화를 위해 여기서 재-export(persistence::FilePresetStore).
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

/// 디스크 표현 — 프로필 목록 앞에 schema_version을 붙여 버전 진화에 대비한다.
#[derive(Serialize, Deserialize)]
struct ProfilesFile {
    schema_version: u32,
    profiles: Vec<AgentProfile>,
}

/// 파일 기반 ProfileStore. 단일 디렉토리에 agents.json 하나를 관리한다.
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

    /// tmp → sync_all → rename → parent fsync. 실패는 io::Error로 올린다.
    fn write_atomic(&self, profiles: &[AgentProfile]) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;

        let payload = ProfilesFile {
            schema_version: SCHEMA_VERSION,
            profiles: profiles.to_vec(),
        };
        let json = serde_json::to_vec_pretty(&payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        // 1) 같은 디렉토리 tmp에 전체를 쓰고 디스크까지 flush(sync_all = 데이터+메타데이터).
        let tmp = self.dir.join(TMP_NAME);
        {
            let mut f = File::create(&tmp)?;
            f.write_all(&json)?;
            f.sync_all()?;
        }

        // 2) atomic rename으로 교체. 같은 디렉토리라 크로스 파일시스템 오류는 발생하지 않는다.
        fs::rename(&tmp, self.path())?;

        // 3) parent 디렉토리 fsync — rename(디렉토리 엔트리 변경)을 영속화.
        //    Windows에선 디렉토리 핸들 fsync 지원이 제한적이라 best-effort로 둔다(실패 무시).
        if let Ok(dir) = File::open(&self.dir) {
            let _ = dir.sync_all();
        }
        Ok(())
    }

    /// 손상 파일을 `.corrupt-<ts>`로 보존(덮어쓰기 방지). 파싱 불가일 때만 호출한다.
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
        // 보안: 자격증명으로 보이는 env는 평문 저장 위험을 경고(저장 자체는 막지 않음).
        warn_if_secret(profiles);

        let _guard = self.write_lock.lock().expect("write_lock poisoned");
        if let Err(e) = self.write_atomic(profiles) {
            // 저장 실패는 치명적이지 않게 로그만 — 상위 동작(spawn 등)을 막지 않는다.
            tracing::error!("save_profiles 실패: {e}");
        } else {
            tracing::debug!(count = profiles.len(), "프로필 저장 완료");
        }
    }

    fn load(&self) -> Vec<AgentProfile> {
        let path = self.path();
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            // 첫 실행이면 파일이 없는 게 정상 — 빈 목록.
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => {
                tracing::warn!("agents.json 읽기 실패: {e} — 빈 목록으로 시작");
                return Vec::new();
            }
        };

        match serde_json::from_slice::<ProfilesFile>(&bytes) {
            Ok(f) if f.schema_version == SCHEMA_VERSION => f.profiles,
            // 버전 불일치: 더 새/옛 포맷일 수 있으니 **파괴하지 않고** 적재만 건너뛴다.
            Ok(f) => {
                tracing::warn!(
                    found = f.schema_version,
                    expected = SCHEMA_VERSION,
                    "agents.json schema_version 불일치 — 적재 건너뜀(파일 보존)"
                );
                Vec::new()
            }
            // 파싱 불가(진짜 손상): .corrupt로 보존 후 빈 목록.
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
        let _ = fs::remove_dir_all(&dir); // 이전 실행 잔여 정리
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

        // 원본은 .corrupt-* 로 보존, agents.json은 사라짐
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
        // 버전 불일치는 파괴하지 않음 — 파일 그대로 유지
        assert!(dir.join(FILE_NAME).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
