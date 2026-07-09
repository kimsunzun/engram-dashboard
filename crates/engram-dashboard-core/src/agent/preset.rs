//! 프리셋 — 스폰 전 "cwd 북마크" 목록의 단일 진실원(single source of truth). (ADR-0061)
//!
//! 프리셋 = 배경 우클릭 "에이전트 생성" picker 에 뜨는 등록 경로 집합. 프로필(agents.json —
//! 스폰된/예약된 에이전트 인스턴스, sid·epoch·restart 정책 보유)과 **의미가 다르다**: 프리셋은
//! 인스턴스가 아니라 경로 북마크라 별도 store(presets.json)로 분리한다(ADR-0061 거부한 대안).
//!
//! 데이터 모델은 최소 `{ id, cwd }` 만 — 이름은 저장하지 않고 프론트가 cwd basename 으로 파생한다
//! (리치화 시 `name: Option<String>` 오버라이드로 확장, ADR-0061). model/icon/inject 는 실수요 때 필드 추가.
//!
//! tauri import 0 — profile.rs 와 동일한 격리 규칙. `PresetStore` trait 주입으로 headless 테스트 가능.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 프리셋 식별자. 프로필의 AgentId 와 동일하게 Uuid.
pub type PresetId = Uuid;

// ── 영속 프리셋 ────────────────────────────────────────────────────────────────

/// 프리셋 1개 — `presets.json` 에 저장되는 단위. (ADR-0061)
///
/// ★최소 스키마★: `{ id, cwd }` 만. 이름은 저장 안 함(cwd basename 파생 — ADR-0061). cwd 는
/// `PresetRegistry::create` 에서 `dunce::canonicalize` 로 정규화(프로필 spawn 경로와 동일한 UNC 회피·
/// 표기 고정). 정규화 실패(경로 부재 등)면 입력 그대로 보존한다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    /// 불변 키. 프리셋 삭제/조회의 참조(프론트 미러 갱신 키).
    pub id: PresetId,
    /// 등록된 작업 디렉토리(정규화됨). 이름은 이 경로 basename 으로 파생(저장 안 함, ADR-0061).
    pub cwd: PathBuf,
}

// ── 영속화 추상화 ──────────────────────────────────────────────────────────────

/// 프리셋 영속화 추상화 — persistence 모듈이 구현한다(FileProfileStore/ProfileStore 미러).
/// trait 주입으로 headless 테스트 시 in-memory store 를 끼울 수 있다.
pub trait PresetStore: Send + Sync + 'static {
    /// 전체 스냅샷을 atomic 하게 저장. 실패는 구현 내부에서 로그만 — 호출자를 막지 않는다.
    fn save(&self, presets: &[Preset]);
    /// 부팅 시 1회 로드. 부재·손상 시 빈 목록.
    fn load(&self) -> Vec<Preset>;
}

// ── PresetRegistry ─────────────────────────────────────────────────────────────

/// 프리셋 인메모리 **단일 소유자**(ProfileRegistry 미러). 모든 CRUD 가 이곳을 거치고,
/// 변경 즉시 store 로 영속화한다.
///
/// 락 규율(ProfileRegistry 와 동일): 디스크 IO(`store.save`)를 presets lock 보유 중에 하지 않는다.
/// lock 안에서 변경 후 스냅샷만 떠서 lock 을 풀고, 그 스냅샷으로 save 한다.
pub struct PresetRegistry {
    presets: Mutex<HashMap<PresetId, Preset>>,
    store: Arc<dyn PresetStore>,
}

impl PresetRegistry {
    /// store 에서 기존 프리셋을 로드해 초기화한다.
    pub fn new(store: Arc<dyn PresetStore>) -> Self {
        let loaded = store.load();
        let map = loaded.into_iter().map(|p| (p.id, p)).collect();
        Self {
            presets: Mutex::new(map),
            store,
        }
    }

    /// 변경 클로저를 lock 안에서 실행하고, lock 해제 후 스냅샷을 save 한다(디스크 IO 를 lock 밖으로).
    fn mutate<R>(&self, f: impl FnOnce(&mut HashMap<PresetId, Preset>) -> R) -> R {
        let (result, snapshot) = {
            let mut guard = self.presets.lock().expect("presets poisoned");
            let result = f(&mut guard);
            let snapshot: Vec<Preset> = guard.values().cloned().collect();
            (result, snapshot)
        };
        self.store.save(&snapshot);
        result
    }

    /// 전체 프리셋 스냅샷(읽기 — persist 없음).
    pub fn list(&self) -> Vec<Preset> {
        self.presets
            .lock()
            .expect("presets poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// 프리셋 생성. 새 uuid 를 발급하고 cwd 를 정규화(프로필 spawn 경로와 동일하게 `dunce::canonicalize`
    /// — UNC `\\?\` 회피 + 표기 고정). 정규화 실패(경로 부재 등)면 입력 그대로 보존한다. 변경 즉시 persist.
    pub fn create(&self, cwd: PathBuf) -> Preset {
        // ★정규화 이유★: 같은 폴더를 다른 표기(대소문자·상대경로·UNC)로 등록하면 프론트 basename
        //   파생·중복 판정이 흔들린다 — 저장 전 canonicalize 로 표기를 고정한다(profile spawn 과 동일 정책).
        let cwd = dunce::canonicalize(&cwd).unwrap_or(cwd);
        let preset = Preset {
            id: Uuid::new_v4(),
            cwd,
        };
        let created = preset.clone();
        self.mutate(|m| {
            m.insert(preset.id, preset);
        });
        created
    }

    /// 프리셋 삭제(없는 id 면 no-op). 변경 즉시 persist. ★프리셋 삭제 ≠ 에이전트 종료★(ADR-0061):
    /// 그 프리셋으로 이미 스폰된 에이전트는 여기서 건드리지 않는다(수명 분리).
    pub fn remove(&self, id: PresetId) {
        self.mutate(|m| {
            m.remove(&id);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 테스트용 in-memory store — 마지막 save 스냅샷을 보관해 검증한다(ProfileRegistry MemStore 미러).
    #[derive(Default)]
    struct MemStore {
        saved: Mutex<Vec<Preset>>,
    }
    impl PresetStore for MemStore {
        fn save(&self, presets: &[Preset]) {
            *self.saved.lock().unwrap() = presets.to_vec();
        }
        fn load(&self) -> Vec<Preset> {
            self.saved.lock().unwrap().clone()
        }
    }

    #[test]
    fn create_mints_uuid_and_persists() {
        let store = Arc::new(MemStore::default());
        let reg = PresetRegistry::new(store.clone());
        let p = reg.create(PathBuf::from("."));
        assert_eq!(reg.list().len(), 1);
        // 즉시 persist(store 에도 반영).
        assert_eq!(store.load().len(), 1);
        assert_eq!(store.load()[0].id, p.id);
    }

    #[test]
    fn create_two_have_distinct_ids() {
        let reg = PresetRegistry::new(Arc::new(MemStore::default()));
        let a = reg.create(PathBuf::from("."));
        let b = reg.create(PathBuf::from("."));
        assert_ne!(a.id, b.id, "각 create 는 새 uuid 를 발급해야 함");
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn remove_deletes_and_persists() {
        let store = Arc::new(MemStore::default());
        let reg = PresetRegistry::new(store.clone());
        let p = reg.create(PathBuf::from("."));
        reg.remove(p.id);
        assert!(reg.list().is_empty());
        assert!(store.load().is_empty(), "삭제도 즉시 persist");
    }

    #[test]
    fn remove_missing_is_noop() {
        let reg = PresetRegistry::new(Arc::new(MemStore::default()));
        reg.create(PathBuf::from("."));
        reg.remove(Uuid::new_v4()); // 없는 id
        assert_eq!(reg.list().len(), 1, "없는 id 삭제는 no-op");
    }

    #[test]
    fn load_restores_existing() {
        let store = Arc::new(MemStore::default());
        {
            let reg = PresetRegistry::new(store.clone());
            reg.create(PathBuf::from("."));
        }
        // 같은 store 로 새 registry 생성 → 로드돼야 함.
        let reg2 = PresetRegistry::new(store.clone());
        assert_eq!(reg2.list().len(), 1);
    }
}
