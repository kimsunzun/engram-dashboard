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

use crate::agent::types::AgentId;

/// epoch millis. 시계 역행/오류 시 0으로 강등(패닉 금지).
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ── 중립 실행 명령 ─────────────────────────────────────────────────────────────

/// claude 출력 포맷 — 프로세스 기동 방식(= transport)과 프론트 렌더러를 함께 가른다(ADR-0044).
/// `Terminal` = PTY 대화형(기존, xterm 렌더). `StreamJson` = `-p` 헤드리스 NDJSON 스트림
/// (StdioTransport + RichSlot 렌더). `stream-json` 은 claude `-p` 전용이라 "렌더러만 스왑"이
/// 아니라 기동 경로 자체가 다르다(ADR-0044 §맥락).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ClaudeOutputFormat {
    /// PTY 대화형(기본). 옛 agents.json(필드 부재)도 이 값으로 역직렬화(하위호환).
    #[default]
    Terminal,
    /// 헤드리스 stream-json(멀티턴 지속 프로세스). MVP=텍스트 챗(ADR-0044 §MVP 범위).
    StreamJson,
}

/// 에이전트가 실제로 무엇을 실행하는가. claude 전용 해석(세션 인자 조립)은 claude.rs가 한다.
/// 여기선 분기 태그와 사용자 추가 인자만 보관한다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum AgentCommand {
    /// claude CLI. `extra_args`는 세션 인자(`--session-id` 등)를 제외한 사용자 추가 인자.
    /// `output_format` 은 터미널/JSON 모드 선택(ADR-0044) — `#[serde(default)]` 라 옛 프로필·
    /// 기존 호출자는 Terminal 로 흡수돼 동작 불변.
    Claude {
        extra_args: Vec<String>,
        #[serde(default)]
        output_format: ClaudeOutputFormat,
    },
    /// 임의 셸 프로그램(검증·범용).
    Shell { program: String, args: Vec<String> },
}

impl AgentCommand {
    /// json(stream-json) 모드 claude 인가 — manager 의 transport 선택(StdioTransport)과
    /// 입력 인코딩(backend::input_encoder) 분기의 단일 판정(ADR-0044). 그 외는 전부 false.
    pub fn is_json_mode(&self) -> bool {
        matches!(
            self,
            AgentCommand::Claude {
                output_format: ClaudeOutputFormat::StreamJson,
                ..
            }
        )
    }
}

/// spawn 시 세션 처리 방식. claude.rs가 이 값에 따라 인자를 다르게 조립한다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnMode {
    /// 새 세션 시작(claude면 `--session-id <새 uuid>`).
    Fresh,
    /// 기존 세션 이어받기(claude면 `--resume <claude_session_id>`).
    Resume,
}

/// 자동 재시작 정책. **예약(reserved) — 죽은 필드 아님.** 동작은 미구현(게이트)이나
/// ADR-0016이 "부팅 복원·가드 카운터·Failed 영속은 유효(추후 재검토)"로 명시한 미래 기능용
/// seam이다. 미리 필드를 둬서 추후 schema/wire 마이그레이션 비용을 아낀다(H-3).
/// ※제거 금지: core→protocol wire(domain.rs)→ts-rs 바인딩→daemon 변환→프론트까지 걸쳐
/// PROTOCOL_VERSION bump를 유발하고 ADR-0016 "추후 재검토" 의도와 충돌한다(2026-06-18 결정).
/// "런타임 자동재시작" 해석만 폐기(ADR-0019) — 부팅 복원/가드/Failed는 유효.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RestartPolicy {
    Never,
    OnCrash,
    #[default]
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
    /// 이어받기 대상이 아니라 새 세션을 시작함(shell, 또는 sid 없는 claude). resume 아님(fable Mn-2).
    Started,
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

    /// 사용자 지정 표시명 override(ADR-0061 리치화 — 트리 rename). **기존 `name` 과 별개 축**: `name` 은
    /// CreateProfile 시 넘어온 이름(claude 프로필) 또는 ad-hoc spawn 의 cwd 문자열이라 "깔끔한 표시명"이
    /// 아니어서 프론트 트리는 이를 무시하고 cwd basename 을 그려왔다. 이 `display_name` 은 그 표시명을
    /// 사람이 직접 덮어쓰는 override 다. `Some` → 그대로 표시, `None` → cwd basename 파생(기존 동작 불변).
    /// `#[serde(default)]` 라 이 필드 없는 옛 agents.json 은 `None` 으로 흡수(마이그레이션 불필요).
    #[serde(default)]
    pub display_name: Option<String>,

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

    /// 자동 재시작 정책. **예약(reserved)** — 동작 미구현(게이트), 제거 금지(RestartPolicy 주석 참조).
    #[serde(default)]
    pub restart_policy: RestartPolicy,

    /// 크래시 가드 카운터(수동 재시작 시 0 리셋). **예약(reserved)** — 동작 미구현, ADR-0016 "추후 재검토" 유효.
    #[serde(default)]
    pub restart_count: u32,

    /// Failed(자동복원 suspend) 사유 — 콜드부팅 넘어 영속, 수동 깨우기 전까지 자동복원 제외(ADR-0016).
    /// **예약(reserved)** — 동작 미구현이나 ADR-0016에서 유효, 제거 금지(wire/바인딩 동반 + 버전 bump).
    #[serde(default)]
    pub failed_reason: Option<String>,

    pub created_at: i64,
    pub last_active: i64,

    /// 마지막 프로세스 기동 시각(기록·디버깅용, 리셋 판정엔 미사용). epoch millis. 없으면 None.
    #[serde(default)]
    pub last_start_at: Option<i64>,
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
            // 생성 시엔 표시명 override 없음 — 트리는 cwd basename 파생(ADR-0061). rename 으로 나중에 set.
            display_name: None,
            command,
            cwd,
            env,
            claude_session_id: None,
            old_session_ids: Vec::new(),
            epoch: 0,
            auto_restore,
            // 사용자 결정(ADR-0016): 항상 살아있게. 동작은 TODO.
            restart_policy: RestartPolicy::Always,
            restart_count: 0,
            failed_reason: None,
            created_at: now,
            last_active: now,
            last_start_at: None,
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
/// 락 규율: 디스크 IO(`store.save`)를 profiles lock **보유 중에** 한다.
/// ★변경 이유(§5 동시성 정합성 > lock-hold 시간)★: 옛 설계는 lock 안에서 스냅샷만 뜨고 lock 을 푼 뒤
/// save 했다. 그러면 두 mutation 이 겹칠 때 "A 스냅샷 → unlock → B 스냅샷 → unlock → B save → A save"
/// 순서로 인메모리·broadcast 는 최신(B)인데 디스크는 stale(A)로 남아, 재시작 시 옛 값이 로드된다
/// (persisted ≠ observed 데이터 정합성 결함). §5 로 LLM/오케스트레이터가 rename/create/delete 를
/// **프로그래밍적으로 동시·연속** 호출하면 사람은 못 여는 이 창을 실제로 친다. 그래서 mutate+save 를
/// 한 임계구역으로 묶어, 마지막 커밋된 인메모리 상태가 곧 디스크 상태가 되게 한다.
/// **데드락 없음(ADR-0006 무관):** `store.save` 는 store 내부 leaf mutex(`write_lock`)만 잡고 registry
/// 로 재진입하지 않는다 → 락 순서는 `profiles → write_lock` 단방향, 순환 없음. profiles lock 은 세션
/// (sessions/core/status) 락 도메인과도 분리라 ADR-0006 순서에 얽히지 않는다. 로컬 소형 파일이라
/// lock 보유 중 IO 비용도 무시 가능.
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

    /// 변경 클로저를 실행하고, **같은 lock 보유 중** 현재 맵을 그대로 save한다.
    /// ★lock 을 풀기 전에 save★ — 두 동시 mutation 이 "각자 스냅샷 → 각자 save" 로 교차해
    /// 디스크가 stale 로 덮이는 race 를 닫는다(상단 락 규율의 §5 근거). 저장하는 스냅샷은
    /// 방금 커밋한 최신 맵이라 persisted == observed 가 보장된다. 모든 mutation 경로의 공통 경로.
    fn mutate<R>(&self, f: impl FnOnce(&mut HashMap<AgentId, AgentProfile>) -> R) -> R {
        let mut guard = self.profiles.lock().expect("profiles poisoned");
        let result = f(&mut guard);
        let snapshot: Vec<AgentProfile> = guard.values().cloned().collect();
        // lock 보유 중 save — 커밋과 영속화를 한 임계구역으로 직렬화(데드락 근거는 struct 주석). ADR-0071.
        self.store.save(&snapshot);
        result
    }

    /// `mutate` 의 조건부 변형 — 클로저가 `true`(실제 변경 있음)를 반환할 때만 lock 보유 중 save 한다.
    /// 변경이 없으면 디스크 쓰기를 건너뛴다(observe_session_id 의 no-op 절약 유지). save 는 mutate 와
    /// 동일하게 lock 보유 중이라 stale-overwrite race 가 없다(struct 주석 §5, ADR-0071).
    fn mutate_if(&self, f: impl FnOnce(&mut HashMap<AgentId, AgentProfile>) -> bool) -> bool {
        let mut guard = self.profiles.lock().expect("profiles poisoned");
        let changed = f(&mut guard);
        if changed {
            let snapshot: Vec<AgentProfile> = guard.values().cloned().collect();
            self.store.save(&snapshot);
        }
        changed
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

    /// 표시명 override 설정/해제(ADR-0061 리치화 — 트리 rename). `Some(name)` → override 저장, `None` →
    /// 해제(cwd basename 파생 복귀). 존재하면 변경 후 persist·true, 없는 id 면 no-op·false.
    /// ★정규화는 호출자(프론트) 책임★: trim·빈 문자열 거부·미변경 스킵은 프론트가 확정 직전에 처리한다
    /// (TabBar rename 과 동형) — 여기엔 이미 유효 값 또는 명시적 None 만 온다. update_with 위임(persist 일원화).
    pub fn rename(&self, id: AgentId, display_name: Option<String>) -> bool {
        self.update_with(id, |p| p.display_name = display_name)
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
    /// ★lock 보유 중 save★: mutate 로 위임해 커밋과 영속화를 한 임계구역으로 직렬화한다 — 옛 코드는
    /// lock 을 푼 뒤 `list()` + save 라 다른 mutation 과 stale-overwrite race 가 있었다(struct 주석 §5).
    /// 변경 없을 때는 디스크 쓰기를 건너뛰어 기존 no-op 절약을 유지한다.
    pub fn observe_session_id(&self, id: AgentId, new_sid: Uuid) -> bool {
        self.mutate_if(|m| match m.get_mut(&id) {
            Some(p) if p.claude_session_id != Some(new_sid) => {
                if let Some(old) = p.claude_session_id.take() {
                    p.old_session_ids.push(old);
                }
                p.claude_session_id = Some(new_sid);
                p.last_active = now_millis();
                true
            }
            _ => false,
        })
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
                output_format: ClaudeOutputFormat::Terminal,
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
        assert_eq!(
            first, second,
            "두 번째 호출은 기존 sid를 그대로 반환해야 함"
        );
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
        assert!(
            got.old_session_ids.contains(&sid1),
            "옛 sid가 이력에 남아야 함"
        );

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

    // ── 표시명 override(ADR-0061 리치화 — 트리 rename) ──────────────────────────────

    #[test]
    fn new_profile_has_no_display_name_override() {
        // 생성 직후엔 override 없음(트리는 cwd basename 파생).
        assert_eq!(sample().display_name, None);
    }

    #[test]
    fn rename_sets_and_persists_display_name() {
        let store = Arc::new(MemStore::default());
        let reg = ProfileRegistry::new(store.clone());
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        assert!(reg.rename(id, Some("내 에이전트".to_string())));
        assert_eq!(
            reg.get(id).unwrap().display_name,
            Some("내 에이전트".to_string())
        );
        // 즉시 persist(store 에도 반영).
        assert_eq!(
            store.load()[0].display_name,
            Some("내 에이전트".to_string())
        );
    }

    #[test]
    fn rename_none_clears_display_name() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        reg.rename(id, Some("x".to_string()));
        // None 재설정 → override 해제(basename 파생 복귀).
        assert!(reg.rename(id, None));
        assert_eq!(reg.get(id).unwrap().display_name, None);
    }

    #[test]
    fn rename_missing_is_noop_false() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        // 없는 id rename 은 false·no-op.
        assert!(!reg.rename(Uuid::new_v4(), Some("y".to_string())));
    }

    // ── 동시성: persisted == latest (stale-overwrite race 봉인) ────────────────────

    /// save 가 lock 보유 중 **현재 맵**을 쓰는지 직접 단언 — 커밋 직후 상태가 곧바로 persist 됨을 본다.
    #[test]
    fn save_writes_current_map_not_stale_snapshot() {
        let store = Arc::new(MemStore::default());
        let reg = ProfileRegistry::new(store.clone());
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        reg.rename(id, Some("final".to_string()));
        let disk = store.load();
        let mem = reg.list();
        assert_eq!(disk.len(), mem.len());
        assert_eq!(disk[0].display_name, Some("final".to_string()));
        assert_eq!(
            disk[0].display_name, mem[0].display_name,
            "persisted == observed"
        );
    }

    /// 여러 스레드가 서로 다른 프로필을 동시에 upsert/rename → 마지막 save 스냅샷이 최종 인메모리 맵과
    /// 개수·내용까지 일치해야 한다. 옛 racy 설계(lock 밖 save)에선 stale 스냅샷이 디스크를 덮어써
    /// 엔트리 누락이 가능했다(persisted ≠ observed). 이제 mutate+save 가 한 임계구역이라 봉인된다.
    #[test]
    fn concurrent_mutations_persisted_equals_final_map() {
        use std::thread;

        let store = Arc::new(MemStore::default());
        let reg = Arc::new(ProfileRegistry::new(store.clone()));

        let mut handles = Vec::new();
        for t in 0..4 {
            let r = reg.clone();
            handles.push(thread::spawn(move || {
                for i in 0..50 {
                    let p = sample();
                    let id = p.id;
                    r.upsert(p);
                    r.rename(id, Some(format!("t{t}-{i}")));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let mem = reg.list();
        let disk = store.load();
        assert_eq!(mem.len(), 200, "인메모리 upsert 200건");
        assert_eq!(
            disk.len(),
            mem.len(),
            "디스크 개수 == 인메모리 개수 (stale 스냅샷으로 엔트리 누락 없음)"
        );

        let mut mem_sorted: Vec<_> = mem.iter().map(|p| (p.id, p.display_name.clone())).collect();
        let mut disk_sorted: Vec<_> = disk
            .iter()
            .map(|p| (p.id, p.display_name.clone()))
            .collect();
        mem_sorted.sort();
        disk_sorted.sort();
        assert_eq!(
            disk_sorted, mem_sorted,
            "동시 mutation 후 디스크 == 최신 인메모리 (persisted == observed)"
        );
    }

    /// 하위호환: 옛 agents.json(필드명 `last_restore`, 신규 필드 부재)을 역직렬화해도
    /// 크래시 없이 신규 필드는 default(restart_count=0, failed_reason=None, last_start_at=None)가 된다.
    /// 옛 `last_restore` 키는 알려지지 않은 필드로 무시된다(serde 기본 deny_unknown 미적용).
    #[test]
    fn deserializes_legacy_profile_without_new_fields() {
        let legacy = r#"{
            "id": "00000000-0000-0000-0000-000000000001",
            "name": "legacy",
            "command": { "kind": "Claude", "extra_args": [] },
            "cwd": ".",
            "env": [],
            "claude_session_id": null,
            "old_session_ids": [],
            "epoch": 3,
            "auto_restore": true,
            "created_at": 100,
            "last_active": 200,
            "last_restore": 150
        }"#;
        let p: AgentProfile =
            serde_json::from_str(legacy).expect("legacy profile must deserialize");
        assert_eq!(p.epoch, 3);
        // restart_policy 부재 → #[serde(default)] = 신규 기본 Always
        assert_eq!(p.restart_policy, RestartPolicy::Always);
        assert_eq!(p.restart_count, 0);
        assert_eq!(p.failed_reason, None);
        // 옛 last_restore 키는 무시되고 신규 last_start_at 은 default None
        assert_eq!(p.last_start_at, None);
        // 신규 display_name 부재 → #[serde(default)] = None(마이그레이션 불필요, 트리 basename 파생 불변).
        assert_eq!(p.display_name, None);
    }

    // ── ADR-0044: output_format serde 하위호환 + is_json_mode 판정 ──────────────
    #[test]
    fn claude_command_without_output_format_defaults_terminal() {
        // 옛 wire/agents.json 은 output_format 필드가 없다 → #[serde(default)] = Terminal.
        let legacy = r#"{ "kind": "Claude", "extra_args": ["--foo"] }"#;
        let cmd: AgentCommand =
            serde_json::from_str(legacy).expect("legacy claude cmd deserialize");
        assert!(
            matches!(
                &cmd,
                AgentCommand::Claude { output_format: ClaudeOutputFormat::Terminal, extra_args }
                    if extra_args == &vec!["--foo".to_string()]
            ),
            "output_format 부재 → Terminal + extra_args 보존"
        );
        assert!(!cmd.is_json_mode(), "Terminal 은 json 모드 아님");
    }

    #[test]
    fn stream_json_command_roundtrips_and_is_json_mode() {
        let cmd = AgentCommand::Claude {
            extra_args: vec![],
            output_format: ClaudeOutputFormat::StreamJson,
        };
        assert!(cmd.is_json_mode(), "StreamJson 은 json 모드");
        // 직렬화→역직렬화 왕복 보존(wire/persist 호환).
        let json = serde_json::to_string(&cmd).unwrap();
        let back: AgentCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(cmd, back);
        // shell 은 항상 json 모드 아님.
        assert!(!AgentCommand::Shell {
            program: "cmd.exe".into(),
            args: vec![]
        }
        .is_json_mode());
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
