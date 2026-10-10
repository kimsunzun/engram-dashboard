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

use crate::profile::{StoreError, StoreStatus};

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
    /// 전체 스냅샷을 atomic 하게 저장한다. 실패의 로그는 구현이 낸다 — 호출자는 `Err` 로 흐름만 가른다.
    fn save(&self, presets: &[Preset]) -> Result<(), StoreError>;
    /// 부팅 시 1회 로드. 없음 · 손상 · 거절이면 빈 목록.
    fn load(&self) -> Vec<Preset>;
    /// 기본 = 늘 쓸 수 있음(거절하지 않는 저장소).
    fn status(&self) -> StoreStatus {
        StoreStatus::Writable
    }
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
/// **데드락 없음(ADR-0006 무관):** `store.save` 는 저장소 상태 칸(잎 락 — 쥔 채 IO 를 하지 않는다)만 잡고
/// registry 로 재진입하지 않는다 → 락 순서는 `presets → 저장소 상태 칸` 단방향, 순환 없음. presets lock 은 세션
/// (sessions/core/status) 락 도메인과도 분리라 ADR-0006 순서에 얽히지 않는다. lock 보유 중 IO 의 크기는
/// ProfileRegistry 와 같다(저장 직전 재판정 · 최악 = 그 파일이 잠긴 동안의 다시 하기 — ADR-0291 · D4).
///
/// `try_` 로 시작하는 동사 = 부르는 쪽 있는 변경 — 사본에 적용 → 저장 → 성공일 때만 커밋하고, `Err` 면 메모리를
/// 그대로 두고 돌려준다(근거 = `ProfileRegistry` doc). ★부르는 쪽 없는 변경은 프리셋에 없다 — 그래서
/// `ProfileRegistry` 의 dirty 칸도 여기엔 두지 않는다★.
// ADR-0291 R17
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

    /// 변경의 공통 경로 — 사본에 적용하고, **lock 을 풀기 전에** 그 사본을 save 해 성공해야 커밋한다. 커밋과
    /// 영속화가 한 임계구역이라 persisted == observed 가 보장된다(근거 = struct 락 규율, ADR-0071).
    /// ★프리셋 변경은 전부 부르는 쪽이 있다(WS 프리셋 셋)★ — 저장 `Err` 에도 메모리에 적용하는 내부 입구가 없고,
    ///   그래서 dirty 도 없다.
    // ADR-0291 R17
    fn try_mutate<R>(
        &self,
        f: impl FnOnce(&mut HashMap<PresetId, Preset>) -> R,
    ) -> Result<R, StoreError> {
        let mut guard = self.presets.lock().expect("presets poisoned");
        let mut draft = guard.clone();
        let result = f(&mut draft);
        let snapshot: Vec<Preset> = draft.values().cloned().collect();
        self.store.save(&snapshot)?;
        *guard = draft;
        Ok(result)
    }

    /// 저장소의 지금 상태 — 프리셋 락을 잡지 않는다(저장소 상태 칸은 락 순서의 끝이다).
    // ADR-0291 R17 (D6)
    pub fn store_status(&self) -> StoreStatus {
        self.store.status()
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

    /// 새 uuid 발급 + cwd 정규화(`dunce::canonicalize` — 실패하면 입력 그대로 보존). 저장이 성공해야 커밋한다.
    pub fn try_create(&self, cwd: PathBuf) -> Result<Preset, StoreError> {
        let preset = new_preset(cwd);
        let created = preset.clone();
        self.try_mutate(|m| {
            m.insert(preset.id, preset);
        })?;
        Ok(created)
    }

    /// 프리셋 삭제(없는 id 면 맵은 그대로 — 저장은 시도한다). 저장이 성공해야 커밋한다. ★프리셋 삭제 ≠ 에이전트
    /// 종료★(ADR-0061): 그 프리셋으로 이미 스폰된 에이전트는 여기서 건드리지 않는다(수명 분리).
    pub fn try_remove(&self, id: PresetId) -> Result<(), StoreError> {
        self.try_mutate(|m| {
            m.remove(&id);
        })
    }

    /// 프리셋 표시명 override 설정/해제(ADR-0061 리치화). `Some(name)` → override 저장, `None` → 해제
    /// (cwd basename 파생으로 복귀). 존재하면 `true`, 없는 id 면 맵은 그대로 `false`(저장은 시도한다). 저장이
    /// 성공해야 커밋한다.
    /// ★정규화는 호출자(프론트) 책임★: trim·빈 문자열 거부·미변경 스킵은 프론트가 확정 직전에 처리한다
    /// (TabBar rename 과 동형) — 여기엔 이미 유효 값 또는 명시적 None 만 온다.
    pub fn try_rename(&self, id: PresetId, name: Option<String>) -> Result<bool, StoreError> {
        self.try_mutate(|m| rename_present(m, id, name))
    }
}

fn new_preset(cwd: PathBuf) -> Preset {
    // ★정규화 이유★: 같은 폴더를 다른 표기(대소문자·상대경로·UNC)로 등록하면 프론트 basename
    //   파생·중복 판정이 흔들린다 — 저장 전 canonicalize 로 표기를 고정한다(profile spawn 과 동일 정책).
    let cwd = dunce::canonicalize(&cwd).unwrap_or(cwd);
    Preset {
        id: Uuid::new_v4(),
        cwd,
        name: None,
    }
}

fn rename_present(m: &mut HashMap<PresetId, Preset>, id: PresetId, name: Option<String>) -> bool {
    match m.get_mut(&id) {
        Some(p) => {
            p.name = name;
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 테스트용 in-memory store — 마지막 save 스냅샷을 보관해 검증한다(ProfileRegistry MemStore 미러).
    #[derive(Default)]
    struct MemStore {
        saved: Mutex<Vec<Preset>>,
        /// 켜져 있으면 저장을 재판정 거절로 돌려준다(아무것도 남기지 않는다).
        refusing: std::sync::atomic::AtomicBool,
    }
    impl PresetStore for MemStore {
        fn save(&self, presets: &[Preset]) -> Result<(), StoreError> {
            if self.refusing.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(StoreError::Refused(crate::profile::Refusal::Unreadable));
            }
            *self.saved.lock().unwrap() = presets.to_vec();
            Ok(())
        }
        fn load(&self) -> Vec<Preset> {
            self.saved.lock().unwrap().clone()
        }
    }

    #[test]
    fn create_mints_uuid_and_persists() {
        let store = Arc::new(MemStore::default());
        let reg = PresetRegistry::new(store.clone());
        let p = reg.try_create(PathBuf::from(".")).expect("저장 성공");
        assert_eq!(reg.list().len(), 1);
        assert_eq!(store.load().len(), 1);
        assert_eq!(store.load()[0].id, p.id);
    }

    #[test]
    fn create_two_have_distinct_ids() {
        let reg = PresetRegistry::new(Arc::new(MemStore::default()));
        let a = reg.try_create(PathBuf::from(".")).expect("저장 성공");
        let b = reg.try_create(PathBuf::from(".")).expect("저장 성공");
        assert_ne!(a.id, b.id, "각 create 는 새 uuid 를 발급해야 함");
        assert_eq!(reg.list().len(), 2);
    }

    #[test]
    fn remove_deletes_and_persists() {
        let store = Arc::new(MemStore::default());
        let reg = PresetRegistry::new(store.clone());
        let p = reg.try_create(PathBuf::from(".")).expect("저장 성공");
        reg.try_remove(p.id).expect("저장 성공");
        assert!(reg.list().is_empty());
        assert!(store.load().is_empty(), "삭제도 즉시 persist");
    }

    #[test]
    fn remove_missing_is_noop() {
        let reg = PresetRegistry::new(Arc::new(MemStore::default()));
        reg.try_create(PathBuf::from(".")).expect("저장 성공");
        reg.try_remove(Uuid::new_v4()).expect("저장 성공");
        assert_eq!(reg.list().len(), 1, "없는 id 삭제는 no-op");
    }

    #[test]
    fn load_restores_existing() {
        let store = Arc::new(MemStore::default());
        {
            let reg = PresetRegistry::new(store.clone());
            reg.try_create(PathBuf::from(".")).expect("저장 성공");
        }
        let reg2 = PresetRegistry::new(store.clone());
        assert_eq!(reg2.list().len(), 1);
    }

    // ── 이름 override(ADR-0061 리치화) ────────────────────────────────────────────

    #[test]
    fn create_starts_with_no_name_override() {
        let reg = PresetRegistry::new(Arc::new(MemStore::default()));
        let p = reg.try_create(PathBuf::from(".")).expect("저장 성공");
        assert_eq!(reg.list()[0].name, None);
        assert_eq!(p.name, None);
    }

    #[test]
    fn rename_sets_and_persists_name() {
        let store = Arc::new(MemStore::default());
        let reg = PresetRegistry::new(store.clone());
        let p = reg.try_create(PathBuf::from(".")).expect("저장 성공");
        assert!(reg
            .try_rename(p.id, Some("내 프리셋".to_string()))
            .expect("저장 성공"));
        assert_eq!(reg.list()[0].name, Some("내 프리셋".to_string()));
        assert_eq!(store.load()[0].name, Some("내 프리셋".to_string()));
    }

    #[test]
    fn rename_none_clears_override() {
        let reg = PresetRegistry::new(Arc::new(MemStore::default()));
        let p = reg.try_create(PathBuf::from(".")).expect("저장 성공");
        reg.try_rename(p.id, Some("x".to_string()))
            .expect("저장 성공");
        assert!(reg.try_rename(p.id, None).expect("저장 성공"));
        assert_eq!(reg.list()[0].name, None);
    }

    #[test]
    fn rename_missing_is_noop_false() {
        let reg = PresetRegistry::new(Arc::new(MemStore::default()));
        reg.try_create(PathBuf::from(".")).expect("저장 성공");
        assert!(!reg
            .try_rename(Uuid::new_v4(), Some("y".to_string()))
            .expect("저장 성공"));
        assert_eq!(reg.list()[0].name, None);
    }

    // ── 동시성: persisted == latest (stale-overwrite race 봉인) ────────────────────

    /// 회귀 가드 — 옛 racy 설계(lock 밖 save)에선 이 불변식이 흔들렸다.
    #[test]
    fn save_writes_current_map_not_stale_snapshot() {
        let store = Arc::new(MemStore::default());
        let reg = PresetRegistry::new(store.clone());
        let p = reg.try_create(PathBuf::from(".")).expect("저장 성공");
        reg.try_rename(p.id, Some("final".to_string()))
            .expect("저장 성공");
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
                    let p = r.try_create(PathBuf::from(".")).expect("저장 성공");
                    r.try_rename(p.id, Some(format!("t{t}-{i}")))
                        .expect("저장 성공");
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

    // ── 부르는 쪽 있는 입구(ADR-0291 R17) ─────────────────────────────────────────

    #[test]
    fn a_caller_change_that_fails_to_save_leaves_presets_as_they_were() {
        let store = Arc::new(MemStore::default());
        let reg = PresetRegistry::new(store.clone());
        let kept = reg.try_create(PathBuf::from(".")).unwrap();
        assert_eq!(store.load(), vec![kept.clone()]);

        store
            .refusing
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(
            reg.try_create(PathBuf::from(".")),
            Err(StoreError::Refused(_))
        ));
        assert!(reg.try_rename(kept.id, Some("x".to_string())).is_err());
        assert!(reg.try_remove(kept.id).is_err());
        assert_eq!(
            reg.list(),
            vec![kept.clone()],
            "만들기 · 이름 바꾸기 · 지우기가 하나도 커밋되지 않았다"
        );

        store
            .refusing
            .store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(reg.try_rename(kept.id, Some("x".to_string())).unwrap());
        assert!(!reg.try_rename(Uuid::new_v4(), None).unwrap());
        assert_eq!(store.load()[0].name.as_deref(), Some("x"));
        reg.try_remove(kept.id).unwrap();
        assert!(reg.list().is_empty());
        assert!(store.load().is_empty());
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
