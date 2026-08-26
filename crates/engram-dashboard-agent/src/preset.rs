//! 프리셋 — 스폰 전 "cwd 북마크" 목록의 단일 진실원(single source of truth). (ADR-0061)
//!
//! 프리셋 = 배경 우클릭 "에이전트 생성" picker 에 뜨는 등록 경로 집합. 프로필(agents.json —
//! 스폰된/예약된 에이전트 인스턴스, sid·epoch·restart 정책 보유)과 **의미가 다르다**: 프리셋은
//! 인스턴스가 아니라 경로 북마크라 별도 store(presets.json)로 분리한다(ADR-0061 거부한 대안).
//!
//! model/icon/inject 필드는 실수요가 생길 때 추가한다(지금 없는 건 의도).
//!
//! tauri import 0 — profile.rs 와 동일한 격리 규칙.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type PresetId = Uuid;

// ── 영속 프리셋 ────────────────────────────────────────────────────────────────

/// 프리셋 1개 — `presets.json` 에 저장되는 단위. (ADR-0061)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    /// 불변 키(프론트 미러 갱신 키).
    pub id: PresetId,
    /// 등록된 작업 디렉토리(정규화됨). 이름 override 가 없으면 이 경로 basename 으로 파생(ADR-0061).
    pub cwd: PathBuf,
    /// 사용자 지정 표시명 override(ADR-0061). `Some` → 그대로 표시, `None` → cwd basename 파생.
    /// `#[serde(default)]` 라 이 필드 없는 옛 presets.json 은 `None` 으로 흡수(마이그레이션 불필요).
    #[serde(default)]
    pub name: Option<String>,
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
/// 락 규율(ProfileRegistry 와 동일): 디스크 IO(`store.save`)를 presets lock **보유 중에** 한다.
/// ★변경 이유(§5 동시성 정합성 > lock-hold 시간)★: 옛 설계는 lock 안에서 스냅샷만 뜨고 lock 을 푼 뒤
/// save 했다. 그러면 두 mutation 이 겹칠 때 "A 스냅샷 → unlock → B 스냅샷 → unlock → B save → A save"
/// 순서로 인메모리·broadcast 는 최신(B)인데 디스크는 stale(A)로 남아, 재시작 시 옛 값이 로드된다
/// (persisted ≠ observed 데이터 정합성 결함). §5 로 LLM/오케스트레이터가 rename/create/delete 를
/// **프로그래밍적으로 동시·연속** 호출하면 사람은 못 여는 이 창을 실제로 친다. 그래서 mutate+save 를
/// 한 임계구역으로 묶어, 마지막 커밋된 인메모리 상태가 곧 디스크 상태가 되게 한다.
/// **데드락 없음(ADR-0006 무관):** `store.save` 는 store 내부 leaf mutex(`write_lock`)만 잡고 registry
/// 로 재진입하지 않는다 → 락 순서는 `presets → write_lock` 단방향, 순환 없음. presets lock 은 세션
/// (sessions/core/status) 락 도메인과도 분리라 ADR-0006 순서에 얽히지 않는다. 로컬 소형 파일이라
/// lock 보유 중 IO 비용도 무시 가능.
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

    /// 변경 클로저를 실행하고 **lock 을 풀기 전에** 현재 맵을 save 한다 — 커밋과 영속화가 한
    /// 임계구역이라 persisted == observed 가 보장된다(근거 = struct 락 규율, ADR-0071).
    fn mutate<R>(&self, f: impl FnOnce(&mut HashMap<PresetId, Preset>) -> R) -> R {
        let mut guard = self.presets.lock().expect("presets poisoned");
        let result = f(&mut guard);
        let snapshot: Vec<Preset> = guard.values().cloned().collect();
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

    /// 새 uuid 발급 + cwd 정규화(`dunce::canonicalize` — 실패하면 입력 그대로 보존). 변경 즉시 persist.
    pub fn create(&self, cwd: PathBuf) -> Preset {
        // ★정규화 이유★: 같은 폴더를 다른 표기(대소문자·상대경로·UNC)로 등록하면 프론트 basename
        //   파생·중복 판정이 흔들린다 — 저장 전 canonicalize 로 표기를 고정한다(profile spawn 과 동일 정책).
        let cwd = dunce::canonicalize(&cwd).unwrap_or(cwd);
        let preset = Preset {
            id: Uuid::new_v4(),
            cwd,
            name: None,
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

    /// 프리셋 표시명 override 설정/해제(ADR-0061 리치화). `Some(name)` → override 저장, `None` → 해제
    /// (cwd basename 파생으로 복귀). 존재하면 변경 후 persist·true, 없는 id 면 no-op·false.
    /// ★정규화는 호출자(프론트) 책임★: trim·빈 문자열 거부·미변경 스킵은 프론트가 확정 직전에 처리한다
    /// (TabBar rename 과 동형) — 여기엔 이미 유효 값 또는 명시적 None 만 온다.
    pub fn rename(&self, id: PresetId, name: Option<String>) -> bool {
        self.mutate(|m| match m.get_mut(&id) {
            Some(p) => {
                p.name = name;
                true
            }
            None => false,
        })
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
        reg.remove(Uuid::new_v4());
        assert_eq!(reg.list().len(), 1, "없는 id 삭제는 no-op");
    }

    #[test]
    fn load_restores_existing() {
        let store = Arc::new(MemStore::default());
        {
            let reg = PresetRegistry::new(store.clone());
            reg.create(PathBuf::from("."));
        }
        let reg2 = PresetRegistry::new(store.clone());
        assert_eq!(reg2.list().len(), 1);
    }

    // ── 이름 override(ADR-0061 리치화) ────────────────────────────────────────────

    #[test]
    fn create_starts_with_no_name_override() {
        let reg = PresetRegistry::new(Arc::new(MemStore::default()));
        let p = reg.create(PathBuf::from("."));
        assert_eq!(reg.list()[0].name, None);
        assert_eq!(p.name, None);
    }

    #[test]
    fn rename_sets_and_persists_name() {
        let store = Arc::new(MemStore::default());
        let reg = PresetRegistry::new(store.clone());
        let p = reg.create(PathBuf::from("."));
        assert!(reg.rename(p.id, Some("내 프리셋".to_string())));
        assert_eq!(reg.list()[0].name, Some("내 프리셋".to_string()));
        assert_eq!(store.load()[0].name, Some("내 프리셋".to_string()));
    }

    #[test]
    fn rename_none_clears_override() {
        let reg = PresetRegistry::new(Arc::new(MemStore::default()));
        let p = reg.create(PathBuf::from("."));
        reg.rename(p.id, Some("x".to_string()));
        assert!(reg.rename(p.id, None));
        assert_eq!(reg.list()[0].name, None);
    }

    #[test]
    fn rename_missing_is_noop_false() {
        let reg = PresetRegistry::new(Arc::new(MemStore::default()));
        reg.create(PathBuf::from("."));
        assert!(!reg.rename(Uuid::new_v4(), Some("y".to_string())));
        assert_eq!(reg.list()[0].name, None);
    }

    // ── 동시성: persisted == latest (stale-overwrite race 봉인) ────────────────────

    /// 회귀 가드 — 옛 racy 설계(lock 밖 save)에선 이 불변식이 흔들렸다.
    #[test]
    fn save_writes_current_map_not_stale_snapshot() {
        let store = Arc::new(MemStore::default());
        let reg = PresetRegistry::new(store.clone());
        let p = reg.create(PathBuf::from("."));
        reg.rename(p.id, Some("final".to_string()));
        let disk = store.load();
        let mem = reg.list();
        assert_eq!(disk.len(), mem.len());
        assert_eq!(disk[0].name, Some("final".to_string()));
        assert_eq!(disk[0].name, mem[0].name, "persisted == observed");
    }

    /// 회귀 가드 — 옛 racy 설계(lock 밖 save)에선 각 mutation 이 자기 스냅샷을 lock 밖에서 save 해,
    /// A 가 B 의 insert 를 못 본 stale 스냅샷으로 디스크를 덮어써 **엔트리가 누락**될 수 있었다.
    /// 반복 create+rename 으로 인터리브 창을 넓힌다.
    #[test]
    fn concurrent_mutations_persisted_equals_final_map() {
        use std::thread;

        let store = Arc::new(MemStore::default());
        let reg = Arc::new(PresetRegistry::new(store.clone()));

        let mut handles = Vec::new();
        for t in 0..4 {
            let r = reg.clone();
            handles.push(thread::spawn(move || {
                for i in 0..50 {
                    let p = r.create(PathBuf::from("."));
                    r.rename(p.id, Some(format!("t{t}-{i}")));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        // 4 스레드 × 50 create = 200 엔트리.
        let mem = reg.list();
        let disk = store.load();
        assert_eq!(mem.len(), 200, "인메모리 create 200건");
        assert_eq!(
            disk.len(),
            mem.len(),
            "디스크 개수 == 인메모리 개수 (stale 스냅샷으로 엔트리 누락 없음)"
        );

        let mut mem_sorted: Vec<_> = mem.iter().map(|p| (p.id, p.name.clone())).collect();
        let mut disk_sorted: Vec<_> = disk.iter().map(|p| (p.id, p.name.clone())).collect();
        mem_sorted.sort();
        disk_sorted.sort();
        assert_eq!(
            disk_sorted, mem_sorted,
            "동시 mutation 후 디스크 == 최신 인메모리 (persisted == observed)"
        );
    }

    /// 픽스처 = `name` 필드가 없던 옛 presets.json(하위호환).
    #[test]
    fn deserializes_legacy_preset_without_name() {
        let legacy = r#"{ "id": "00000000-0000-0000-0000-000000000001", "cwd": "C:/proj" }"#;
        let p: Preset = serde_json::from_str(legacy).expect("legacy preset must deserialize");
        assert_eq!(p.name, None);
        assert_eq!(p.cwd, PathBuf::from("C:/proj"));
    }
}
