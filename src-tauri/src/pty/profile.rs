//! 에이전트 프로필 — 재시작·세션 복원의 단일 진실원(single source of truth).
//!
//! 이 모듈은 의도적으로 transport·claude 중립이다. claude 전용 인자 조립
//! (`--session-id` / `--resume`)은 `pty/claude.rs`가 맡고, 여기엔 "무엇을 실행하고
//! 어떤 세션을 이어받을지"라는 중립 데이터만 둔다. 이렇게 분리해 두면 추후 codex CLI나
//! 다른 백엔드가 붙어도 이 모듈은 거의 바뀌지 않는다(미래 확장 seam).
//!
//! tauri import 0 — pty/ 격리 규칙 준수.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::pty::types::AgentId;

/// epoch millis. 시계 역행/오류 시 0으로 강등(패닉 금지).
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ── 중립 실행 명령 ─────────────────────────────────────────────────────────────

/// 에이전트가 실제로 무엇을 실행하는가. claude 전용 해석(세션 인자 조립)은 claude.rs가 한다.
/// 여기선 분기 태그와 사용자 추가 인자만 보관한다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum AgentCommand {
    /// claude CLI. `extra_args`는 세션 인자(`--session-id` 등)를 제외한 사용자 추가 인자.
    Claude { extra_args: Vec<String> },
    /// 임의 셸 프로그램(검증·범용).
    Shell { program: String, args: Vec<String> },
}

/// spawn 시 세션 처리 방식. claude.rs가 이 값에 따라 인자를 다르게 조립한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnMode {
    /// 새 세션 시작(claude면 `--session-id <새 uuid>`).
    Fresh,
    /// 기존 세션 이어받기(claude면 `--resume <claude_session_id>`).
    Resume,
}

/// 자동 재시작 정책. 현재는 필드만 두고 동작은 게이트(추후 구현). 기본 `Never`.
/// 미리 필드를 둬서 추후 schema 마이그레이션 비용을 아낀다(H-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RestartPolicy {
    #[default]
    Never,
    OnCrash,
    Always,
}

// ── 복원 결과 ──────────────────────────────────────────────────────────────────

/// 복원 시도 결과 한 건. `restore_all`이 에이전트별로 반환하고 프론트에 통지한다.
#[derive(Debug, Clone, Serialize)]
pub struct RestoreReport {
    pub agent_id: AgentId,
    pub epoch: u32,
    pub outcome: RestoreOutcome,
}

/// 복원 결말. 프론트와 공유되므로 internally-tagged(discriminated union)로 직렬화.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum RestoreOutcome {
    /// `--resume` 성공 — 기존 대화 그대로 이어받음.
    Resumed,
    /// resume 실패 → 새 세션으로 fallback. 어떤 sid가 폐기되고 새로 생겼는지 명시한다.
    /// (silent stale 금지 — 무엇이 바뀌었는지 항상 가시화)
    FreshFallback {
        old_sid: Option<Uuid>,
        new_sid: Uuid,
        reason: String,
    },
    /// `auto_restore=false` 등으로 복원 대상이 아니어서 건너뜀.
    Blocked { reason: String },
    /// fresh조차 실패 → 정지. 재귀 재시도 없는 종점(H-1.7).
    Failed { reason: String },
}

// ── 영속 프로필 ────────────────────────────────────────────────────────────────

/// 에이전트 1개의 영속 프로필 — `agents.json`에 저장되는 단위.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    /// 불변 키. 프로세스·세션이 바뀌어도 이 id는 평생 유지된다(프론트 구독 키).
    pub id: AgentId,
    pub name: String,
    pub command: AgentCommand,

    /// 저장 전 `dunce::canonicalize`로 정규화된 cwd(UNC `\\?\` 회피 + 표기 고정).
    /// claude 세션 디렉토리가 cwd 문자 치환이라, 표기가 흔들리면 세션을 잃는다(spike 확인).
    pub cwd: PathBuf,

    /// ※자격증명 금지. persist 시 `*_KEY`/`*_TOKEN` 패턴은 경고한다(persistence).
    pub env: Vec<(String, String)>,

    /// 현재 claude 세션 id. **가변** — 최초엔 우리가 생성하고, `/clear` 등으로 바뀌면
    /// session_tracker watcher가 갱신한다. None이면 아직 세션이 없다는 뜻.
    pub claude_session_id: Option<Uuid>,

    /// fallback·clear로 폐기된 과거 세션 id 이력(감사·디버깅용).
    pub old_session_ids: Vec<Uuid>,

    /// 재spawn마다 +1. 프론트가 `[agentId, epoch]`로 재구독하는 결정적 트리거.
    pub epoch: u32,

    /// 앱 재시작 시 자동 복원 대상인지.
    pub auto_restore: bool,

    /// 자동 재시작 정책(현재 필드만, 동작은 게이트).
    #[serde(default)]
    pub restart_policy: RestartPolicy,

    pub created_at: i64,
    pub last_active: i64,

    /// 마지막 복원 시각(epoch millis). 없으면 None.
    #[serde(default)]
    pub last_restore: Option<i64>,
}

impl AgentProfile {
    /// 새 프로필 생성. id는 새 uuid, epoch 0, 세션 id는 아직 없음(None).
    /// 세션 id는 최초 spawn 시 ProfileRegistry가 생성한다(여기서 만들지 않음).
    pub fn new(
        name: String,
        command: AgentCommand,
        cwd: PathBuf,
        env: Vec<(String, String)>,
        auto_restore: bool,
    ) -> Self {
        let now = now_millis();
        Self {
            id: Uuid::new_v4(),
            name,
            command,
            cwd,
            env,
            claude_session_id: None,
            old_session_ids: Vec::new(),
            epoch: 0,
            auto_restore,
            restart_policy: RestartPolicy::Never,
            created_at: now,
            last_active: now,
            last_restore: None,
        }
    }
}

// ── 영속화 추상화 ──────────────────────────────────────────────────────────────

/// 프로필 영속화 추상화 — persistence 모듈이 구현한다. trait 주입으로 headless 테스트 시
/// in-memory store를 끼울 수 있다(StatusSink와 동일한 격리 패턴).
pub trait ProfileStore: Send + Sync + 'static {
    /// 전체 스냅샷을 atomic하게 저장. 실패는 구현 내부에서 로그만 — 호출자를 막지 않는다.
    fn save(&self, profiles: &[AgentProfile]);
    /// 부팅 시 1회 로드. 부재·손상 시 빈 목록.
    fn load(&self) -> Vec<AgentProfile>;
}

// ── ProfileRegistry ────────────────────────────────────────────────────────────

/// 프로필 인메모리 **단일 소유자**. 모든 CRUD·세션 id 갱신이 이곳을 거치고,
/// 변경 즉시 store로 영속화한다. 세션 id의 생성·갱신 책임도 여기 있다(spawn_agent 아님 — H-1.4).
///
/// 락 규율: 디스크 IO(`store.save`)를 profiles lock 보유 중에 하지 않는다.
/// lock 안에서 변경 후 스냅샷만 떠서 lock을 풀고, 그 스냅샷으로 save한다.
pub struct ProfileRegistry {
    profiles: Mutex<HashMap<AgentId, AgentProfile>>,
    store: Arc<dyn ProfileStore>,
}

impl ProfileRegistry {
    /// store에서 기존 프로필을 로드해 초기화한다.
    pub fn new(store: Arc<dyn ProfileStore>) -> Self {
        let loaded = store.load();
        let map = loaded.into_iter().map(|p| (p.id, p)).collect();
        Self {
            profiles: Mutex::new(map),
            store,
        }
    }

    /// 변경 클로저를 lock 안에서 실행하고, lock 해제 후 스냅샷을 save한다.
    /// 디스크 IO를 lock 밖으로 빼는 공통 경로(상단 락 규율 참조).
    fn mutate<R>(&self, f: impl FnOnce(&mut HashMap<AgentId, AgentProfile>) -> R) -> R {
        let (result, snapshot) = {
            let mut guard = self.profiles.lock().expect("profiles poisoned");
            let result = f(&mut guard);
            let snapshot: Vec<AgentProfile> = guard.values().cloned().collect();
            (result, snapshot)
        };
        self.store.save(&snapshot);
        result
    }

    /// 전체 프로필 스냅샷(읽기 — persist 없음).
    pub fn list(&self) -> Vec<AgentProfile> {
        self.profiles
            .lock()
            .expect("profiles poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// 단건 조회(읽기 — persist 없음).
    pub fn get(&self, id: AgentId) -> Option<AgentProfile> {
        self.profiles
            .lock()
            .expect("profiles poisoned")
            .get(&id)
            .cloned()
    }

    /// auto_restore=true인 프로필만(복원 대상).
    pub fn restorable(&self) -> Vec<AgentProfile> {
        self.profiles
            .lock()
            .expect("profiles poisoned")
            .values()
            .filter(|p| p.auto_restore)
            .cloned()
            .collect()
    }

    /// 프로필 생성·교체(upsert). 변경 즉시 persist.
    pub fn upsert(&self, profile: AgentProfile) {
        self.mutate(|m| {
            m.insert(profile.id, profile);
        });
    }

    /// 프로필 삭제. 변경 즉시 persist.
    pub fn remove(&self, id: AgentId) {
        self.mutate(|m| {
            m.remove(&id);
        });
    }

    /// 임의 필드 수정. 존재하면 클로저 적용 후 persist, 없으면 false.
    pub fn update_with(&self, id: AgentId, f: impl FnOnce(&mut AgentProfile)) -> bool {
        self.mutate(|m| match m.get_mut(&id) {
            Some(p) => {
                f(p);
                true
            }
            None => false,
        })
    }

    /// 세션 id 확보 — claude_session_id가 None이면 새로 생성·persist하고 반환한다.
    /// 이미 있으면 그대로 반환한다. **세션 id 생성 책임은 ProfileRegistry**(H-1.4):
    /// spawn_agent은 이 값을 받아 인자만 조립한다.
    pub fn ensure_session_id(&self, id: AgentId) -> Option<Uuid> {
        self.mutate(|m| {
            let p = m.get_mut(&id)?;
            if p.claude_session_id.is_none() {
                p.claude_session_id = Some(Uuid::new_v4());
            }
            p.claude_session_id
        })
    }

    /// watcher가 세션 id 변경을 관측했을 때 호출 — 옛 sid를 이력으로 넘기고 새 값으로 교체,
    /// 변경 즉시 persist한다(1-b: clear→관측→persist 전 크래시 시 stale 복원 방지).
    /// 같은 값으로의 호출은 no-op(불필요한 디스크 쓰기 회피).
    pub fn observe_session_id(&self, id: AgentId, new_sid: Uuid) -> bool {
        let changed = {
            let mut guard = self.profiles.lock().expect("profiles poisoned");
            match guard.get_mut(&id) {
                Some(p) if p.claude_session_id != Some(new_sid) => {
                    if let Some(old) = p.claude_session_id.take() {
                        p.old_session_ids.push(old);
                    }
                    p.claude_session_id = Some(new_sid);
                    p.last_active = now_millis();
                    true
                }
                _ => false,
            }
        };
        if changed {
            // lock 해제 후 즉시 persist.
            let snapshot = self.list();
            self.store.save(&snapshot);
        }
        changed
    }

    /// epoch 증가 후 새 값 반환. "같은 AgentId 맵 교체"가 일어나는 **모든 지점**에서
    /// 호출해야 한다(restart + fresh fallback respawn 포함 — H-1.5).
    pub fn bump_epoch(&self, id: AgentId) -> Option<u32> {
        self.mutate(|m| {
            let p = m.get_mut(&id)?;
            p.epoch = p.epoch.wrapping_add(1);
            Some(p.epoch)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 테스트용 in-memory store — 마지막 save 스냅샷을 보관해 검증한다.
    #[derive(Default)]
    struct MemStore {
        saved: Mutex<Vec<AgentProfile>>,
    }
    impl ProfileStore for MemStore {
        fn save(&self, profiles: &[AgentProfile]) {
            *self.saved.lock().unwrap() = profiles.to_vec();
        }
        fn load(&self) -> Vec<AgentProfile> {
            self.saved.lock().unwrap().clone()
        }
    }

    fn sample() -> AgentProfile {
        AgentProfile::new(
            "t".into(),
            AgentCommand::Claude {
                extra_args: vec![],
            },
            PathBuf::from("."),
            vec![],
            true,
        )
    }

    #[test]
    fn upsert_and_get() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        assert!(reg.get(id).is_some());
        assert_eq!(reg.list().len(), 1);
    }

    #[test]
    fn ensure_session_id_generates_once() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        let first = reg.ensure_session_id(id).unwrap();
        let second = reg.ensure_session_id(id).unwrap();
        assert_eq!(first, second, "두 번째 호출은 기존 sid를 그대로 반환해야 함");
    }

    #[test]
    fn observe_session_id_pushes_old_and_persists() {
        let store = Arc::new(MemStore::default());
        let reg = ProfileRegistry::new(store.clone());
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        let sid1 = reg.ensure_session_id(id).unwrap();
        let sid2 = Uuid::new_v4();

        assert!(reg.observe_session_id(id, sid2));
        let got = reg.get(id).unwrap();
        assert_eq!(got.claude_session_id, Some(sid2));
        assert!(got.old_session_ids.contains(&sid1), "옛 sid가 이력에 남아야 함");

        // 같은 값 재관측은 no-op
        assert!(!reg.observe_session_id(id, sid2));

        // store에도 반영됐는지(즉시 persist)
        let persisted = store.load();
        assert_eq!(persisted[0].claude_session_id, Some(sid2));
    }

    #[test]
    fn bump_epoch_increments() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        assert_eq!(reg.bump_epoch(id), Some(1));
        assert_eq!(reg.bump_epoch(id), Some(2));
    }

    #[test]
    fn load_restores_existing() {
        let store = Arc::new(MemStore::default());
        {
            let reg = ProfileRegistry::new(store.clone());
            reg.upsert(sample());
        }
        // 같은 store로 새 registry 생성 → 로드돼야 함
        let reg2 = ProfileRegistry::new(store.clone());
        assert_eq!(reg2.list().len(), 1);
    }
}
