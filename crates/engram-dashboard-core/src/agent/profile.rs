//! 에이전트 프로필 — 재시작·세션 복원의 단일 진실원(single source of truth).
//!
//! 이 모듈은 의도적으로 transport·claude 중립이다. claude 전용 인자 조립
//! (`--session-id` / `--resume`)은 `backend/claude.rs`가 맡고, 여기엔 "무엇을 실행하고
//! 어떤 세션을 이어받을지"라는 중립 데이터만 둔다.
//!
//! tauri import 0 — 격리 규칙 준수.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent::types::AgentId;

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ── 중립 실행 명령 ─────────────────────────────────────────────────────────────

/// claude 출력 포맷 — 프로세스 기동 방식(= transport)과 프론트 렌더러를 함께 가른다(ADR-0044).
/// `Terminal` = PTY 대화형(xterm 렌더). `StreamJson` = `-p` 헤드리스 NDJSON 스트림
/// (StdioTransport + RichSlot 렌더).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ClaudeOutputFormat {
    #[default]
    Terminal,
    StreamJson,
}

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
    Shell {
        program: String,
        args: Vec<String>,
    },
}

impl AgentCommand {
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
/// PROTOCOL_VERSION bump를 유발하고 ADR-0016 "추후 재검토" 의도와 충돌한다.
/// "런타임 자동재시작" 해석만 폐기(ADR-0019) — 부팅 복원/가드/Failed는 유효.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RestartPolicy {
    Never,
    OnCrash,
    #[default]
    Always,
}

// ── 복원 결과 ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct RestoreReport {
    pub agent_id: AgentId,
    pub epoch: u32,
    pub outcome: RestoreOutcome,
}

/// 프론트와 공유되므로 internally-tagged(discriminated union)로 직렬화.
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
    /// 아니다. `Some` → 그대로 표시, `None` → cwd basename 파생.
    /// `#[serde(default)]` 라 이 필드 없는 옛 agents.json 은 `None` 으로 흡수(마이그레이션 불필요).
    #[serde(default)]
    pub display_name: Option<String>,

    /// 트리 계층의 부모 프로필 id. `Some(pid)` → 이 프로필은
    /// pid 의 자식(트리에서 pid 밑에 들여쓰기), `None` → 최상위(루트). **1단 중첩만 허용**: 자식은 다시
    /// 부모가 될 수 없고(cycle 방지 단순화), 부모는 반드시 루트여야 한다 — 검증은 `ProfileRegistry::reparent`.
    /// 부모 삭제 시 자식은 여기서 `None` 으로 풀려 루트로 승격한다(orphan-to-root, cascade 삭제 아님).
    /// `#[serde(default)]` 라 이 필드 없는 옛 agents.json 은 `None`(루트)으로 흡수(마이그레이션 불필요).
    // ADR-0072
    #[serde(default)]
    pub parent_id: Option<AgentId>,

    pub command: AgentCommand,

    /// ★저장된 값은 **raw** 다 — 정규화되지 않는다(리뷰 fix N3 · load-bearing)★: 이 필드를 canonicalize
    /// 하는 코드는 core·persistence 어디에도 없다. 그래서 `"."`·`".."`·심링크·대소문자 다른 Windows 경로가
    /// 그대로 앉아 있을 수 있고, 정규화는 이 값을 쓰는 쪽(spawn 의 `session.cwd`,
    /// `canonical_name_when_live`)이 각자 한다.
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

    /// ★이 프로세스에서 이 에이전트의 화신(세션)이 있었나★ — 다음 spawn 이 **최초**인가 **교체**인가를
    /// 가르는 축이다(ADR-0007 은 *교체*에만 epoch bump 를 건다 — `epoch_for_spawn`).
    ///
    /// ★`#[serde(skip)]` 가 의미의 일부다★: "이 프로세스에서" 가 정확히 필요한 범위다. 디스크에 남기면
    ///   재부팅 후 첫 spawn 이 교체로 오판돼 근거 없이 epoch 를 올린다(무해하지만 부정확). 새 프로세스엔
    ///   아직 배출 중인 앞선 화신이 **있을 수 없으므로** false 로 시작하는 게 옳다.
    /// ★프로필과 한 몸인 게 요점★: 별도 집합으로 두면 삭제(`remove`)와 spawn 판정 사이에 창이 생기고
    ///   (삭제가 표식만 지운 뒤 spawn 이 다시 심으면 회수 불가) 크기 상한도 규약으로만 남는다. 필드면
    ///   삭제가 표식을 **구조적으로** 함께 거두고, 개수는 정의상 프로필 수를 넘지 못한다.
    /// ★스냅샷이 되돌리지 못한다 — 그리고 그게 이 표식의 유일한 방어다★: spawn 호출부는 프로필 **스냅샷**
    ///   을 넘기므로(연결이 들고 있던 옛 사본일 수 있다) `upsert_preserving_hierarchy` 가 live 값을
    ///   보존해야 한다(epoch 과 같은 이유 — ADR-0084). 이 값을 **평범한 `upsert` 로** 쓰는 경로가 새로
    ///   생기면 그 사본이 표식을 지우고, 다음 Fresh 가 교체를 최초로 오판해 죽은 화신의 epoch 를
    ///   재사용한다. 새 spawn 명령이 caller 가 만든 프로필을 그대로 받는 모양이면 여기를 먼저 볼 것.
    /// ★유계★: 프로필의 한 필드라 개수는 정의상 프로필 수와 같다 — 따로 회수할 표가 없다(별도 집합이었을
    ///   땐 삭제가 표식만 지운 뒤 spawn 이 다시 심으면 회수 불가였다).
    // ADR-0007
    // ADR-0113
    #[serde(skip)]
    pub had_session: bool,

    pub auto_restore: bool,

    /// 자동 재시작 정책. **예약(reserved)** — 동작 미구현(게이트), 제거 금지(RestartPolicy 주석 참조).
    #[serde(default)]
    pub restart_policy: RestartPolicy,

    /// 크래시 가드 카운터. **예약(reserved)** — 동작 미구현, ADR-0016 "추후 재검토" 유효.
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
            display_name: None,
            parent_id: None,
            command,
            cwd,
            env,
            claude_session_id: None,
            old_session_ids: Vec::new(),
            epoch: 0,
            had_session: false,
            auto_restore,
            restart_policy: RestartPolicy::Always,
            restart_count: 0,
            failed_reason: None,
            created_at: now,
            last_active: now,
            last_start_at: None,
        }
    }

    /// ★이 프로필이 **다시 뜨면 갖게 될 canonical 이름**(load-bearing)★ — 복원된 세션이 갖는 산 canonical
    /// 이름과 **글자 그대로 같아야** 한다(갈리면 이 값을 키로 쓴 잠듦 파킹이 주인을 못 찾는다).
    ///
    /// ★산 세션과 같은 규칙 = `canonical_name_or_id_fallback`(+ canonicalize 된 cwd)★:
    ///   - `manager::resolve_canonical_name` 이 산 세션에 쓰는 그 함수다. `name::resolve_display_name` 은
    ///     **아니다** — 그건 빈/공백-only override 를 그대로 이름으로 쓰고 basename 이 placeholder(경로 없음)
    ///     일 때 id 로 degrade 하지도 않아, 두 엣지에서 산 이름과 갈린다.
    ///   - cwd 도 산 세션과 같은 표기여야 한다: spawn 은 `dunce::canonicalize(profile.cwd)` 한 값을
    ///     session.cwd 로 쓰고, 프로필 쪽 `cwd` 에는 raw 값(`.`·`..`·심링크·대소문자 다른 Windows 경로)이
    ///     들어올 수 있다. 그래서 여기서도 **같은 정규화**를 하고, 실패하면(경로가 이미 사라졌으면)
    ///     spawn 과 동일하게 원본으로 degrade 한다.
    /// ★fs 접근은 **override 가 없을 때만** 있다(순수 아님)★: canonicalize 는 실제 파일시스템을 본다 —
    ///   그래서 이 동사는 이름 파생 순수 코어(`name.rs`)가 아니라 프로필 쪽에 있다.
    /// ★override 단축은 최적화가 아니다 — 지우지 말 것★: `display_name` 이 비공백이면 canonical 이름은
    ///   **파일시스템에 전혀 의존하지 않는다**(산 세션 규칙도 override 를 그대로 쓴다). 그래서 그 경우
    ///   syscall 을 아예 하지 않는다. 지우면 ① cwd 디렉터리가 사라진 프로필의 이름이 raw 로 degrade 해
    ///   잠듦 시점에 파생된 이름과 갈리고 ② cwd 가 죽은 네트워크 공유(SMB)면 canonicalize 한 번이 수십 초
    ///   블록이라 호출자 경로에 그 지연이 붙는다.
    /// ★남는 갭(알고 수용)★: **override 가 없는데 cwd 가 사라진** 프로필은 여전히 raw degrade 라 잠듦 시점
    ///   이름과 갈릴 수 있다. 막으려면 이름 키 파킹 자체를 재설계해야 해서(새 결정 소관) 여기서 몰래
    ///   바꾸지 않는다.
    // ADR-0101 (WYSIWYA) / ADR-0116 (결정 1 — 잠듦 파킹 키)
    pub fn canonical_name_when_live(&self) -> String {
        // 산 세션 규칙(`canonical_name_or_id_fallback`)의 첫 분기와 **글자 그대로 같다**(비공백이면 trim
        //   하지 않고 원본 그대로가 이름).
        if let Some(n) = self.display_name.as_deref() {
            if !n.trim().is_empty() {
                return n.to_string();
            }
        }
        // ★답을 바꿀 수 없는 syscall 은 하지 않는다★: cwd 가 빈/공백-only 면 basename 이 placeholder 라
        //   결과는 canonicalize 성공/실패와 무관하게 id 앞 8자다(`canonical_name_or_id_fallback` degrade).
        let raw = self.cwd.to_string_lossy();
        if raw.trim().is_empty() {
            return crate::agent::name::canonical_name_or_id_fallback(None, &raw, self.id);
        }
        let cwd = dunce::canonicalize(&self.cwd).unwrap_or_else(|_| self.cwd.clone());
        crate::agent::name::canonical_name_or_id_fallback(None, &cwd.to_string_lossy(), self.id)
    }
}

// ── 영속화 추상화 ──────────────────────────────────────────────────────────────

/// persistence 모듈이 구현한다. trait 주입으로 headless 테스트 시 in-memory store를 끼울 수 있다.
pub trait ProfileStore: Send + Sync + 'static {
    /// 전체 스냅샷을 atomic하게 저장. 실패는 구현 내부에서 로그만 — 호출자를 막지 않는다.
    fn save(&self, profiles: &[AgentProfile]);
    /// 부팅 시 1회 로드. 부재·손상 시 빈 목록.
    fn load(&self) -> Vec<AgentProfile>;
}

// ── 계층 정규화(ADR-0072) ────────────────────────────────────────────────────────

/// 맵 전체를 "유효한 1단 forest" 불변식으로 강제 복구한다 — **모든 save 직전**(`mutate`/`mutate_if`)
/// 에서 lock 보유 중 호출된다.
///
/// ★왜 경로마다가 아니라 경계에서★: reparent 는 자체 검증이 있지만 upsert/update_with 는 임의
/// parent_id 를 받고(cycle·dangling), spawn 은 stale 스냅샷을 재삽입해 그 사이 삭제된 부모를 되살린다.
/// 검증을 reparent 에만 두면 이 경로들이 우회해 cycle·dangling 을 persist 한다.
///
/// **idempotent**: 이미 유효한 1단 forest(정상 reparent 결과)는 어떤 규칙에도 걸리지 않아 그대로
/// 살아남는다 — 두 번 돌려도 결과가 같다.
// ADR-0072
fn normalize_hierarchy(map: &mut HashMap<AgentId, AgentProfile>) {
    // 판정은 변경 전 스냅샷 기준(같은 pass 안에서의 clear 가 다른 노드 판정을 오염시키지 않게).
    // has_own_parent(= 자식) 과 is_a_parent(= 자식을 가진 노드) 를 섞으면 정상 자식이 오검출된다.
    let existing: std::collections::HashSet<AgentId> = map.keys().copied().collect();
    let has_own_parent: std::collections::HashSet<AgentId> = map
        .values()
        .filter(|p| p.parent_id.is_some())
        .map(|p| p.id)
        .collect();
    let is_a_parent: std::collections::HashSet<AgentId> =
        map.values().filter_map(|p| p.parent_id).collect();

    let mut clear: Vec<AgentId> = Vec::new();
    for p in map.values() {
        if let Some(pid) = p.parent_id {
            let dangling = !existing.contains(&pid); // 부모가 맵에 없음
            let self_parent = pid == p.id; // 자기 자신을 부모로
            let two_level = has_own_parent.contains(&pid); // 부모 pid 자신이 또 부모를 가짐 → 2단
            let node_is_parent = is_a_parent.contains(&p.id); // 이 노드가 누군가의 부모 → 자식이 될 수 없음
            if dangling || self_parent || two_level || node_is_parent {
                clear.push(p.id);
            }
        }
    }
    for id in clear {
        if let Some(p) = map.get_mut(&id) {
            p.parent_id = None;
        }
    }
}

// ── ProfileRegistry ────────────────────────────────────────────────────────────

/// 프로필 인메모리 **단일 소유자**. 모든 CRUD·세션 id 갱신이 이곳을 거치고,
/// 변경 즉시 store로 영속화한다. 세션 id의 생성·갱신 책임도 여기 있다(spawn_agent 아님 — H-1.4).
///
/// 락 규율: 디스크 IO(`store.save`)를 profiles lock **보유 중에** 한다.
/// ★save 를 lock 밖으로 빼지 말 것(§5 동시성 정합성 > lock-hold 시간)★: lock 안에서 스냅샷만 뜨고 푼 뒤
/// save 하면 두 mutation 이 겹칠 때 "A 스냅샷 → unlock → B 스냅샷 → unlock → B save → A save"
/// 순서로 인메모리·broadcast 는 최신(B)인데 디스크는 stale(A)로 남아, 재시작 시 옛 값이 로드된다
/// (persisted ≠ observed). §5 로 LLM/오케스트레이터가 rename/create/delete 를
/// **프로그래밍적으로 동시·연속** 호출하면 사람은 못 여는 이 창을 실제로 친다.
/// **데드락 없음(ADR-0006 무관):** `store.save` 는 store 내부 leaf mutex(`write_lock`)만 잡고 registry
/// 로 재진입하지 않는다 → 락 순서는 `profiles → write_lock` 단방향, 순환 없음. profiles lock 은 세션
/// (sessions/core/status) 락 도메인과도 분리라 그 순서에 얽히지 않는다. 로컬 소형 파일이라
/// lock 보유 중 IO 비용도 무시 가능.
pub struct ProfileRegistry {
    profiles: Mutex<HashMap<AgentId, AgentProfile>>,
    store: Arc<dyn ProfileStore>,
}

impl ProfileRegistry {
    pub fn new(store: Arc<dyn ProfileStore>) -> Self {
        let loaded = store.load();
        let map = loaded.into_iter().map(|p| (p.id, p)).collect();
        Self {
            profiles: Mutex::new(map),
            store,
        }
    }

    /// 모든 mutation 경로의 공통 경로 — 클로저 커밋과 save 가 한 임계구역이다.
    fn mutate<R>(&self, f: impl FnOnce(&mut HashMap<AgentId, AgentProfile>) -> R) -> R {
        let mut guard = self.profiles.lock().expect("profiles poisoned");
        let result = f(&mut guard);
        normalize_hierarchy(&mut guard);
        let snapshot: Vec<AgentProfile> = guard.values().cloned().collect();
        // lock 보유 중 save — 커밋과 영속화를 한 임계구역으로 직렬화(데드락 근거는 struct 주석). ADR-0071.
        self.store.save(&snapshot);
        result
    }

    /// `mutate` 의 조건부 변형 — 클로저가 `true`(실제 변경 있음)를 반환할 때만 save 한다.
    fn mutate_if(&self, f: impl FnOnce(&mut HashMap<AgentId, AgentProfile>) -> bool) -> bool {
        let mut guard = self.profiles.lock().expect("profiles poisoned");
        let changed = f(&mut guard);
        if changed {
            normalize_hierarchy(&mut guard);
            let snapshot: Vec<AgentProfile> = guard.values().cloned().collect();
            self.store.save(&snapshot);
        }
        changed
    }

    pub fn list(&self) -> Vec<AgentProfile> {
        self.profiles
            .lock()
            .expect("profiles poisoned")
            .values()
            .cloned()
            .collect()
    }

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

    pub fn upsert(&self, profile: AgentProfile) {
        self.mutate(|m| {
            m.insert(profile.id, profile);
        });
    }

    /// spawn 전용 upsert — 스냅샷을 삽입하되 **오케스트레이션 메타데이터(`parent_id`·`display_name`)는
    /// 이미 맵에 있는 live 엔트리 값을 보존**한다(ADR-0070/0072). 없던 id(ad-hoc spawn)면 스냅샷 그대로.
    ///
    /// ★왜 spawn 스냅샷을 그대로 안 쓰나(lost-update 봉인)★: 넘어오는 프로필은 호출 시점에 뜬
    /// **스냅샷**이라, 그 사이 다른 연결이 reparent/rename 을 커밋했다면 옛 `parent_id`/`display_name` 이
    /// 최신 값을 덮어써 동시 편집이 유실된다(lost update). 그 둘은 프로세스 기동과 무관한 순수 트리 메타라
    /// spawn 이 author 할 이유가 없으므로, live 엔트리가 있으면 그 두 필드만 보존하고 나머지
    /// (cwd/command/env/session 등 spawn 이 실제로 확정하는 필드)는 스냅샷을 반영한다.
    ///
    /// ★epoch 도 live 보존(ADR-0084)★: epoch 역시 프로세스 기동과 무관한 순수 런타임 메타로, spawn 이
    ///   넘긴 **스냅샷**이 author 하면 안 된다. spawn 은 이 upsert **직후** `epoch_for_spawn` 으로 registry
    ///   epoch 를 확정하는데(화신 교체면 +1), 옛 스냅샷의 epoch 를 여기서 그대로 삽입하면 앞선 bump 가
    ///   되돌려져(lost update) 새 세션이 죽은 세션과 같은 epoch 를 재사용한다 → 프론트 재구독 누락(빈 슬롯)
    ///   + 턴 관측·제어 토큰이 두 세대를 구분 못 함. 그래서 parent_id/display_name 과 동일하게 live 값을
    ///   보존한다. 신규 id(live 없음)면 스냅샷 epoch(0)를 그대로 쓴다 — 첫 화신은 올릴 앞선 epoch 가 없다.
    // ADR-0070 ADR-0072 ADR-0084
    pub fn upsert_preserving_hierarchy(&self, mut profile: AgentProfile) {
        self.mutate(|m| {
            if let Some(live) = m.get(&profile.id) {
                profile.parent_id = live.parent_id;
                profile.display_name = live.display_name.clone();
                profile.epoch = live.epoch;
                // 같은 이유로 화신 이력도 live 값이 이긴다 — 스냅샷은 그 사실의 author 가 아니다
                //   (되돌리면 다음 Fresh 가 교체를 최초로 오판해 죽은 화신의 epoch 를 재사용한다).
                // ADR-0007
                profile.had_session = live.had_session;
            }
            m.insert(profile.id, profile);
        });
    }

    /// **부모 삭제 시 자식 루트 승격(orphan-to-root):**
    /// 삭제 대상을 부모로 가리키던 자식들의 `parent_id` 를 **같은 임계구역에서** `None` 으로 푼다 —
    /// 존재하지 않는 부모를 가리키는 고아 참조(dangling parent)를 남기지 않는다(트리 렌더·복원 불변식).
    /// cascade 삭제가 아니라 승격이라 자식 데이터는 보존된다(사용자 결정 — 실수로 그룹 전체 소실 방지).
    // ADR-0072
    pub fn remove(&self, id: AgentId) {
        self.mutate(|m| {
            m.remove(&id);
            // 1단 중첩이라 자식은 부모가 아니므로 재귀 승격은 불필요 — 한 번의 훑기로 충분하다.
            for p in m.values_mut() {
                if p.parent_id == Some(id) {
                    p.parent_id = None;
                }
            }
        });
    }

    /// 표시명 override 설정/해제(ADR-0061 리치화 — 트리 rename). `Some(name)` → override 저장, `None` →
    /// 해제(cwd basename 파생 복귀). 존재하면 변경 후 persist·true, 없는 id 면 no-op·false.
    /// ★정규화는 저장 게이트(`AgentManager::rename_agent`) 책임★: 양끝 공백 제거와 "공백만 남으면 override
    /// 없음" 판정은 이름 유일성 판정 **전에** 거기서 끝난다(`normalize_display_name`) — 여기서 또 깎으면
    /// 판정이 본 값과 저장되는 값이 갈린다. 여기엔 이미 정규화된 값 또는 명시적 None 만 온다.
    /// update_with 위임(persist 일원화).
    pub fn rename(&self, id: AgentId, display_name: Option<String>) -> bool {
        self.update_with(id, |p| p.display_name = display_name)
    }

    /// 트리 부모 지정/해제(ADR-0072 — 계층 reparent). `Some(pid)` → child_id 를 pid 의 자식으로,
    /// `None` → 루트로 승격. 검증 전부를 **한 임계구역(mutate)** 안에서 하고 성공 시에만 persist·true,
    /// 위반이면 no-op·false(rename/update_with 와 동형 bool 반환).
    ///
    /// **1단 중첩 규칙(cycle 방지):**
    /// - child 가 존재해야 한다(없으면 false).
    /// - `Some(pid)`: ① pid 프로필이 실존해야 함 ② `pid != child_id`(self-parent 금지) ③ child 가 현재
    ///   누군가의 부모가 아니어야 함(부모를 가진 노드는 자식이 될 수 없음 — 1단 상한) ④ 대상 부모 pid
    ///   자신이 루트여야 함(`parent_id == None` — 부모가 부모를 갖는 2단 금지). 하나라도 위반이면 false.
    /// - `None`: child 가 존재하면 항상 허용(루트 승격).
    ///
    /// ★검증을 lock 안에서★: 존재/부모여부 판정과 쓰기가 한 임계구역이라, 동시 reparent/delete 와
    /// TOCTOU(검사-후-변경 사이 상태 변동)로 cycle·고아가 새는 창을 닫는다(ADR-0071 락 규율 경유).
    // ADR-0072
    pub fn reparent(&self, child_id: AgentId, parent_id: Option<AgentId>) -> bool {
        self.mutate(|m| {
            if !m.contains_key(&child_id) {
                return false;
            }
            if let Some(pid) = parent_id {
                if pid == child_id {
                    return false;
                }
                // 대상 부모 실존 + 그 자신이 루트여야 함(2단 금지).
                match m.get(&pid) {
                    Some(parent) if parent.parent_id.is_none() => {}
                    _ => return false,
                }
                // child 가 누군가의 부모면 자식이 될 수 없음(1단 상한).
                if m.values().any(|p| p.parent_id == Some(child_id)) {
                    return false;
                }
            }
            match m.get_mut(&child_id) {
                Some(c) => {
                    c.parent_id = parent_id;
                    true
                }
                None => false,
            }
        })
    }

    /// 존재하면 클로저 적용 후 persist, 없으면 false.
    pub fn update_with(&self, id: AgentId, f: impl FnOnce(&mut AgentProfile)) -> bool {
        self.mutate(|m| match m.get_mut(&id) {
            Some(p) => {
                f(p);
                true
            }
            None => false,
        })
    }

    /// 세션 id 확보 — `claude_session_id` 가 None 이면 새로 생성하고, 이미 있으면 그대로 반환한다.
    /// **세션 id 생성 책임은 ProfileRegistry**(H-1.4).
    ///
    /// ★Resume 전용(ADR-0076)★: 기존 대화를 이어받으려면 저장된 sid 를 그대로 써야 한다.
    ///   Fresh 모드는 절대 이걸 쓰면 안 된다: 기존 sid 를 그대로 돌려주므로 Fresh 가
    ///   `--session-id <저장된 sid>` 로 떠 디스크 세션과 충돌한다("Session ID already in use" — claude
    ///   즉사). Fresh 는 `new_session_id`(항상 새 uuid).
    pub fn ensure_session_id(&self, id: AgentId) -> Option<Uuid> {
        self.mutate(|m| {
            let p = m.get_mut(&id)?;
            if p.claude_session_id.is_none() {
                p.claude_session_id = Some(Uuid::new_v4());
            }
            p.claude_session_id
        })
    }

    /// **Fresh spawn 전용** 세션 id 발급 — 항상 새 uuid 를 만들어 set·persist 하고 반환한다.
    /// 기존 sid 가 있으면 이력(`old_session_ids`)으로 밀어 넣는다(감사·디버깅용, observe_session_id 패턴).
    ///
    /// ★왜 ensure_session_id 와 분리했나★: `ensure_session_id` 는 "있으면 그대로" 라 Fresh 가 그걸 쓰면
    ///   저장된 sid 를 재사용해 `--session-id <저장 sid>` 로 뜨고, 디스크에 이미 그 세션 파일이 있으면
    ///   claude 가 "Session ID <sid> is already in use" 로 즉사한다(데몬 콜드부팅 후 예약 프로필 활성화 시
    ///   재현). Fresh = "진짜 새 대화" 이므로 반드시 새 sid 여야 하고, 이 메서드가 그 계약을 강제한다.
    // ADR-0076 ADR-0008
    pub fn new_session_id(&self, id: AgentId) -> Option<Uuid> {
        self.mutate(|m| {
            let p = m.get_mut(&id)?;
            // 기존 sid 는 이력으로 보존(silent 소실 금지 — observe_session_id 와 동형).
            if let Some(old) = p.claude_session_id.take() {
                p.old_session_ids.push(old);
            }
            let fresh = Uuid::new_v4();
            p.claude_session_id = Some(fresh);
            Some(fresh)
        })
    }

    /// watcher가 세션 id 변경을 관측했을 때 호출 — 옛 sid를 이력으로 넘기고 새 값으로 교체,
    /// 변경 즉시 persist한다(1-b: clear→관측→persist 전 크래시 시 stale 복원 방지).
    /// 같은 값으로의 호출은 no-op(불필요한 디스크 쓰기 회피).
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

    /// ★spawn 이 쓸 epoch 을 **한 임계구역에서** 확정한다(ADR-0007 "같은 AgentId 맵 교체마다 +1")★.
    ///
    /// 앞선 화신이 있었으면(`had_session`) 올리고, 최초 화신이면 현재 값을 그대로 준다.
    /// `None` = 그 사이 프로필이 사라졌다(동시 삭제) → **호출자는 spawn 을 중단해야 한다**.
    ///
    /// ★판정과 커밋이 한 락 안이어야 하는 이유(load-bearing)★: 둘로 나누면 그 사이 `remove` 가 끼어들어
    ///   ① 판정은 "교체" 인데 bump 대상이 사라지거나 ② 판정이 "최초" 로 뒤집혀 죽은 화신의 epoch 를
    ///   재사용한다. 어느 쪽이든 두 화신이 같은 (AgentId, epoch) 를 갖게 되고, 그걸 키로 쓰는 구조
    ///   (턴 관측 표 ADR-0113 · 제어 채널 토큰 ADR-0086 · reap epoch-guard ADR-0084)가 두 세대를 구분하지
    ///   못한다. 그래서 "읽고-정하고-쓰기" 를 여기 한 곳에 가둔다.
    /// ★현 프로덕션 호출점은 `AgentManager::spawn_agent` **하나**다★ — 모드(Fresh/Resume)를 가리지 않고
    ///   그 지점만 지나간다. 호출부마다 bump 를 흩뿌리면 새 호출부가 또 빠뜨리므로(실측: WS `Spawn`·
    ///   부팅 복원이 그렇게 빠졌다) 여기 단일 진입점을 유지할 것. dead code 아님(오인해 지우지 말 것 —
    ///   지우면 화신 교체가 epoch 를 재사용해 프론트 재구독 누락·관측 오염·토큰 충돌이 한꺼번에 난다).
    // ADR-0084
    // ADR-0007
    pub fn epoch_for_spawn(&self, id: AgentId) -> Option<u32> {
        self.mutate(|m| {
            let p = m.get_mut(&id)?;
            if p.had_session {
                p.epoch = p.epoch.wrapping_add(1);
            }
            Some(p.epoch)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn canonical_name_when_live_mirrors_the_live_session_rule() {
        // ADR-0116 결정 1 — 잠듦 파킹 키의 전제: 잠든 프로필의 이름은 **산 세션과 같은 규칙**으로
        //   파생돼야 한다(`canonical_name_or_id_fallback` + canonicalize 된 cwd).
        // ★상대 경로가 이 테스트의 핵심★: `PathBuf::from(".")` 은 raw 라 basename 이 `"."` 이 되지만, 산
        //   세션은 canonicalize 된 절대경로의 basename 을 쓴다. 두 값이 같아야 한다는 것을 여기서 못 박는다
        //   (`resolve_display_name(None, ".")` 로 뽑으면 `"."` 이 나와 조용히 어긋난다).
        let p = sample(); // cwd = "."
        let expected = {
            let abs = dunce::canonicalize(".").expect("cwd 는 실재한다");
            crate::agent::name::canonical_name_or_id_fallback(None, &abs.to_string_lossy(), p.id)
        };
        assert_eq!(p.canonical_name_when_live(), expected);
        assert_ne!(
            p.canonical_name_when_live(),
            ".",
            "raw cwd basename 을 쓰면 산 이름과 갈린다"
        );

        let mut named = sample();
        named.display_name = Some("Alice".into());
        assert_eq!(named.canonical_name_when_live(), "Alice");
        let mut blank = sample();
        blank.display_name = Some("   ".into());
        assert_eq!(
            blank.canonical_name_when_live(),
            expected,
            "공백-only override 는 무시된다(산 세션 규칙과 동일 — resolve_display_name 은 이 가드가 없다)"
        );
    }

    #[test]
    fn canonical_name_when_live_survives_a_vanished_cwd_when_an_override_exists() {
        // ★리뷰 fix(D3)★: override 가 있으면 이름이 fs 에 전혀 의존하지 않아야 한다 — cwd 디렉터리가
        //   사라져 canonicalize 가 실패해도 잠듦 시점에 파생된 이름과 같아야 한다는 게 이 테스트의 계약이다.
        let vanished = PathBuf::from("C:/engram-does-not-exist-9f1c/never/created");
        assert!(
            dunce::canonicalize(&vanished).is_err(),
            "이 테스트의 전제 — 이 경로는 실재하지 않아야 canonicalize 실패 경로를 탄다"
        );

        let mut named = sample();
        named.cwd = vanished.clone();
        named.display_name = Some("Renamed".into());
        assert_eq!(
            named.canonical_name_when_live(),
            "Renamed",
            "override 가 있으면 fs 를 보지 않는다(canonicalize 실패가 이름을 흔들면 안 된다)"
        );

        // ★수용된 갭(함수 doc)★: override 가 없으면 raw basename 으로 degrade 한다.
        let mut anon = sample();
        anon.cwd = vanished;
        anon.display_name = None;
        assert_eq!(
            anon.canonical_name_when_live(),
            "created",
            "override 없음 + cwd 소멸 → raw basename(알고 수용한 갭)"
        );

        let mut blank = sample();
        blank.cwd = PathBuf::from("   ");
        blank.display_name = None;
        let id8 = blank.id.to_string()[..8].to_string();
        assert_eq!(
            blank.canonical_name_when_live(),
            id8,
            "빈 cwd 는 canonicalize 결과와 무관하게 id degrade 다"
        );
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

        assert!(!reg.observe_session_id(id, sid2));

        let persisted = store.load();
        assert_eq!(persisted[0].claude_session_id, Some(sid2));
    }

    /// ADR-0076: new_session_id 는 항상 **새** sid 를 발급하고 옛 sid 를 이력으로 민다.
    /// (Fresh spawn 이 저장된 sid 를 재사용하는 "Session ID already in use" 버그를 봉인.)
    #[test]
    fn new_session_id_mints_fresh_and_pushes_old() {
        let store = Arc::new(MemStore::default());
        let reg = ProfileRegistry::new(store.clone());
        let p = sample();
        let id = p.id;
        reg.upsert(p);

        let sid1 = reg.ensure_session_id(id).unwrap();
        let sid2 = reg.new_session_id(id).unwrap();
        assert_ne!(
            sid1, sid2,
            "Fresh 는 새 sid 여야 함(저장된 sid 재사용 금지)"
        );
        let got = reg.get(id).unwrap();
        assert_eq!(
            got.claude_session_id,
            Some(sid2),
            "현재 sid = 새로 발급한 값"
        );
        assert!(
            got.old_session_ids.contains(&sid1),
            "옛 sid 는 이력으로 밀려야 함"
        );
        assert_eq!(store.load()[0].claude_session_id, Some(sid2));
    }

    /// ADR-0076: sid 가 없던(진짜 신규) 프로필에 new_session_id → 새 sid 발급, 이력은 비어 있음.
    #[test]
    fn new_session_id_on_fresh_profile_has_no_history() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        let sid = reg.new_session_id(id).unwrap();
        let got = reg.get(id).unwrap();
        assert_eq!(got.claude_session_id, Some(sid));
        assert!(
            got.old_session_ids.is_empty(),
            "세션 없던 프로필은 밀 옛 sid 가 없음"
        );
    }

    /// ADR-0076: 두 번 연속 new_session_id → 매번 새 sid(재사용 없음), 이력이 누적된다.
    #[test]
    fn new_session_id_always_differs_and_accumulates_history() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        let a = reg.new_session_id(id).unwrap();
        let b = reg.new_session_id(id).unwrap();
        assert_ne!(a, b, "연속 Fresh 는 매번 다른 sid");
        let got = reg.get(id).unwrap();
        assert_eq!(got.claude_session_id, Some(b));
        assert!(got.old_session_ids.contains(&a), "직전 sid 는 이력에");
    }

    #[test]
    fn epoch_for_spawn_increments_only_after_a_prior_incarnation() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        // 최초 화신 — 올릴 앞선 epoch 이 없다(ADR-0007 은 *교체*에만 건다).
        assert_eq!(reg.epoch_for_spawn(id), Some(0));
        assert_eq!(reg.epoch_for_spawn(id), Some(0), "판정은 멱등이어야");

        // 화신이 하나 있었다고 표시하면 그 뒤부터는 교체다.
        reg.update_with(id, |p| p.had_session = true);
        assert_eq!(reg.epoch_for_spawn(id), Some(1));
        assert_eq!(reg.epoch_for_spawn(id), Some(2));

        // ★삭제와의 원자성★: 사라진 프로필은 epoch 를 만들어 내지 않는다(호출자가 spawn 을 중단한다).
        reg.remove(id);
        assert_eq!(
            reg.epoch_for_spawn(id),
            None,
            "프로필이 없으면 None — 0 으로 떨어지면 죽은 화신보다 작은 epoch 이 산 세션에 붙는다"
        );
    }

    #[test]
    fn had_session_survives_a_stale_snapshot_upsert() {
        // ★스냅샷은 화신 이력의 author 가 아니다★: 되돌아가면 다음 Fresh 가 교체를 최초로 오판해
        //   죽은 화신의 epoch 를 재사용한다(epoch 보존과 같은 이유 — ADR-0084).
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p.clone());
        reg.update_with(id, |p| p.had_session = true);
        // spawn 호출부가 들고 있던 옛 스냅샷(had_session=false)으로 upsert.
        reg.upsert_preserving_hierarchy(p);
        assert!(
            reg.get(id).expect("존재").had_session,
            "live 값이 이겨야(스냅샷이 화신 이력을 지우면 epoch 재사용이 되살아난다)"
        );
    }

    // ── 표시명 override(ADR-0061 리치화 — 트리 rename) ──────────────────────────────

    #[test]
    fn new_profile_has_no_display_name_override() {
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
        assert!(reg.rename(id, None));
        assert_eq!(reg.get(id).unwrap().display_name, None);
    }

    #[test]
    fn rename_missing_is_noop_false() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        assert!(!reg.rename(Uuid::new_v4(), Some("y".to_string())));
    }

    // ── 트리 계층 reparent(ADR-0072) ────────────────────────────────────────────

    #[test]
    fn reparent_sets_and_persists() {
        let store = Arc::new(MemStore::default());
        let reg = ProfileRegistry::new(store.clone());
        let parent = sample();
        let child = sample();
        let (pid, cid) = (parent.id, child.id);
        reg.upsert(parent);
        reg.upsert(child);

        assert!(reg.reparent(cid, Some(pid)));
        assert_eq!(reg.get(cid).unwrap().parent_id, Some(pid));
        let disk = store.load();
        let persisted_child = disk.iter().find(|p| p.id == cid).unwrap();
        assert_eq!(persisted_child.parent_id, Some(pid));
    }

    #[test]
    fn reparent_none_promotes_to_root() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let parent = sample();
        let child = sample();
        let (pid, cid) = (parent.id, child.id);
        reg.upsert(parent);
        reg.upsert(child);
        reg.reparent(cid, Some(pid));
        assert!(reg.reparent(cid, None));
        assert_eq!(reg.get(cid).unwrap().parent_id, None);
    }

    #[test]
    fn reparent_rejects_self_parent() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        assert!(!reg.reparent(id, Some(id)), "self-parent 는 거부");
        assert_eq!(reg.get(id).unwrap().parent_id, None);
    }

    #[test]
    fn reparent_rejects_nonexistent_parent() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let child = sample();
        let cid = child.id;
        reg.upsert(child);
        assert!(
            !reg.reparent(cid, Some(Uuid::new_v4())),
            "없는 부모 지정은 거부"
        );
        assert_eq!(reg.get(cid).unwrap().parent_id, None);
    }

    #[test]
    fn reparent_rejects_missing_child() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let parent = sample();
        let pid = parent.id;
        reg.upsert(parent);
        assert!(
            !reg.reparent(Uuid::new_v4(), Some(pid)),
            "없는 child 는 거부"
        );
    }

    #[test]
    fn reparent_rejects_making_a_node_with_children_a_child() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let a = sample(); // 미래의 부모
        let b = sample(); // a 의 자식
        let c = sample(); // a 를 자식으로 만들려는 새 루트
        let (aid, bid, cid) = (a.id, b.id, c.id);
        reg.upsert(a);
        reg.upsert(b);
        reg.upsert(c);
        assert!(reg.reparent(bid, Some(aid)));
        assert!(
            !reg.reparent(aid, Some(cid)),
            "자식을 가진 노드는 자식이 될 수 없음(1단)"
        );
        assert_eq!(reg.get(aid).unwrap().parent_id, None);
    }

    #[test]
    fn reparent_rejects_parent_that_has_a_parent() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let a = sample(); // 루트
        let b = sample(); // a 의 자식
        let c = sample(); // b 를 부모로 삼으려는 노드
        let (aid, bid, cid) = (a.id, b.id, c.id);
        reg.upsert(a);
        reg.upsert(b);
        reg.upsert(c);
        assert!(reg.reparent(bid, Some(aid)));
        assert!(
            !reg.reparent(cid, Some(bid)),
            "부모가 부모를 가진 경우 자식 지정 거부(1단)"
        );
        assert_eq!(reg.get(cid).unwrap().parent_id, None);
    }

    #[test]
    fn delete_parent_orphans_children_to_root() {
        let store = Arc::new(MemStore::default());
        let reg = ProfileRegistry::new(store.clone());
        let a = sample();
        let b = sample();
        let c = sample();
        let (aid, bid, cid) = (a.id, b.id, c.id);
        reg.upsert(a);
        reg.upsert(b);
        reg.upsert(c);
        reg.reparent(bid, Some(aid));
        reg.reparent(cid, Some(aid));

        reg.remove(aid);
        assert!(reg.get(aid).is_none());
        assert_eq!(reg.get(bid).unwrap().parent_id, None, "b 는 루트 승격");
        assert_eq!(reg.get(cid).unwrap().parent_id, None, "c 는 루트 승격");

        let disk = store.load();
        assert!(!disk.iter().any(|p| p.id == aid));
        assert!(disk
            .iter()
            .filter(|p| p.id == bid || p.id == cid)
            .all(|p| p.parent_id.is_none()));
    }

    // ── 쓰기 경계 정규화(ADR-0072) — reparent 를 우회하는 경로도 불변식 유지 ───────────

    /// ★재현 경로★: store 에 이미 cyclic 쌍이 있는 상태(손상된/손편집 agents.json, 또는 reparent 검증을
    /// 우회하는 legacy write)를 load 로 그대로 들여온 뒤(load 는 정규화 안 함), 아무 write 나 한 번 트리거하면
    /// 경계 정규화가 cycle 을 healing 한다. **normalize_hierarchy 없이는 실패한다** — 두 parent_id 가 그대로
    /// 남아 인메모리·디스크에 cycle 이 persist 된다.
    #[test]
    fn cycle_in_map_is_normalized_at_write_boundary() {
        let mut a = sample();
        let mut b = sample();
        let (aid, bid) = (a.id, b.id);
        a.parent_id = Some(bid);
        b.parent_id = Some(aid);
        let store = Arc::new(MemStore::default());
        store.save(&[a, b]);
        let reg = ProfileRegistry::new(store.clone());
        assert_eq!(
            reg.get(aid).unwrap().parent_id,
            Some(bid),
            "로드 직후엔 cycle 잔존"
        );

        reg.rename(aid, Some("touch".into()));

        assert_eq!(reg.get(aid).unwrap().parent_id, None, "cycle 참여 A→루트");
        assert_eq!(reg.get(bid).unwrap().parent_id, None, "cycle 참여 B→루트");
        let disk = store.load();
        assert!(
            disk.iter().all(|p| p.parent_id.is_none()),
            "디스크에도 cycle 없음"
        );
    }

    /// ★정규화 없이는 실패★: update_with 는 임의 클로저라 검증이 없다.
    #[test]
    fn update_with_dangling_parent_is_normalized() {
        let store = Arc::new(MemStore::default());
        let reg = ProfileRegistry::new(store.clone());
        let child = sample();
        let cid = child.id;
        reg.upsert(child);

        let missing = Uuid::new_v4();
        assert!(reg.update_with(cid, |p| p.parent_id = Some(missing)));
        assert_eq!(
            reg.get(cid).unwrap().parent_id,
            None,
            "dangling 참조는 루트로 정규화"
        );
        assert!(
            store
                .load()
                .iter()
                .find(|p| p.id == cid)
                .unwrap()
                .parent_id
                .is_none(),
            "디스크에도 dangling 없음"
        );
    }

    /// 정규화는 idempotent + 유효 계층 불변: 정상 reparent 결과(1단 forest)는 어떤 재-write 로도
    /// 훼손되지 않는다. rename(무관한 write)이 자식의 parent_id 를 건드리지 않아야 한다.
    #[test]
    fn normalization_preserves_valid_hierarchy() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let parent = sample();
        let child = sample();
        let (pid, cid) = (parent.id, child.id);
        reg.upsert(parent);
        reg.upsert(child);
        assert!(reg.reparent(cid, Some(pid)));
        reg.rename(cid, Some("x".into()));
        reg.rename(pid, Some("y".into()));
        assert_eq!(
            reg.get(cid).unwrap().parent_id,
            Some(pid),
            "정상 부모 관계는 정규화로 훼손되지 않음(idempotent)"
        );
    }

    /// 동시성: reparent(child→parent) 와 remove(parent) 를 경쟁시켜도 최종 persisted 에 dangling 없음.
    /// 부모가 사라지면 자식은 루트여야 한다(orphan-to-root + 경계 정규화 이중 안전망).
    #[test]
    fn concurrent_reparent_vs_remove_no_dangling() {
        use std::thread;

        // 반복해 인터리빙을 넓게 흔든다(레이스 재현 확률↑).
        for _ in 0..200 {
            let store = Arc::new(MemStore::default());
            let reg = Arc::new(ProfileRegistry::new(store.clone()));
            let parent = sample();
            let child = sample();
            let (pid, cid) = (parent.id, child.id);
            reg.upsert(parent);
            reg.upsert(child);

            let r1 = reg.clone();
            let r2 = reg.clone();
            let h1 = thread::spawn(move || {
                r1.reparent(cid, Some(pid));
            });
            let h2 = thread::spawn(move || {
                r2.remove(pid);
            });
            h1.join().unwrap();
            h2.join().unwrap();

            let disk = store.load();
            assert!(!disk.iter().any(|p| p.id == pid), "부모는 삭제됨");
            if let Some(c) = disk.iter().find(|p| p.id == cid) {
                assert_ne!(
                    c.parent_id,
                    Some(pid),
                    "삭제된 부모를 가리키는 dangling 참조가 persist 되면 안 됨"
                );
            }
            let ids: std::collections::HashSet<_> = disk.iter().map(|p| p.id).collect();
            assert!(
                disk.iter()
                    .filter_map(|p| p.parent_id)
                    .all(|pp| ids.contains(&pp)),
                "persisted 계층에 dangling 부모 참조 없음"
            );
        }
    }

    /// ADR-0070/0072 lost-update 봉인: spawn 스냅샷이 최신 parent_id/display_name 을 덮지 않는다.
    #[test]
    fn spawn_preserving_upsert_does_not_revert_concurrent_reparent_or_rename() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let root = sample();
        let child = sample();
        let (rid, cid) = (root.id, child.id);
        reg.upsert(root);
        reg.upsert(child);

        let stale_snapshot = reg.get(cid).unwrap();
        assert_eq!(stale_snapshot.parent_id, None);
        assert_eq!(stale_snapshot.display_name, None);

        // 그 사이 다른 연결이 reparent + rename 을 커밋.
        assert!(reg.reparent(cid, Some(rid)));
        assert!(reg.rename(cid, Some("live".into())));

        reg.upsert_preserving_hierarchy(stale_snapshot);
        let after = reg.get(cid).unwrap();
        assert_eq!(
            after.parent_id,
            Some(rid),
            "stale spawn 스냅샷이 최신 parent_id 를 되돌리면 안 됨(lost update 봉인)"
        );
        assert_eq!(
            after.display_name,
            Some("live".into()),
            "stale spawn 스냅샷이 최신 display_name 을 되돌리면 안 됨(ADR-0070 latent)"
        );
    }

    /// ad-hoc spawn(맵에 없던 id)은 preserving-upsert 여도 스냅샷 그대로 삽입됨을 함께 확인.
    #[test]
    fn preserving_upsert_inserts_snapshot_when_no_live_entry() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let mut p = sample();
        let id = p.id;
        p.display_name = Some("adhoc".into());
        reg.upsert_preserving_hierarchy(p);
        assert_eq!(reg.get(id).unwrap().display_name, Some("adhoc".into()));
    }

    /// serde default: parent_id 없는 JSON → None(무마이그레이션, ADR-0072).
    #[test]
    fn deserializes_profile_without_parent_id_as_none() {
        let json = r#"{
            "id": "00000000-0000-0000-0000-000000000002",
            "name": "no-parent",
            "command": { "kind": "Claude", "extra_args": [] },
            "cwd": ".",
            "env": [],
            "claude_session_id": null,
            "old_session_ids": [],
            "epoch": 0,
            "auto_restore": true,
            "created_at": 1,
            "last_active": 1
        }"#;
        let p: AgentProfile = serde_json::from_str(json).expect("parent_id 없는 프로필 역직렬화");
        assert_eq!(p.parent_id, None, "parent_id 부재 → None(루트)");
    }

    /// 동시성: 여러 스레드가 서로 다른 자식을 같은 루트에 동시 reparent 해도 persisted == 최신 인메모리.
    /// mutate+save 한 임계구역이라 stale-overwrite·검증 race 없음(struct 주석 §5, ADR-0071).
    #[test]
    fn concurrent_reparent_persisted_equals_final_map() {
        use std::thread;

        let store = Arc::new(MemStore::default());
        let reg = Arc::new(ProfileRegistry::new(store.clone()));
        let root = sample();
        let root_id = root.id;
        reg.upsert(root);

        let mut handles = Vec::new();
        for _ in 0..4 {
            let r = reg.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..50 {
                    let child = sample();
                    let cid = child.id;
                    r.upsert(child);
                    r.reparent(cid, Some(root_id));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let mem = reg.list();
        let disk = store.load();
        assert_eq!(mem.len(), 201, "루트 1 + 자식 200");
        assert_eq!(disk.len(), mem.len(), "디스크 개수 == 인메모리(누락 없음)");
        let children_ok = mem
            .iter()
            .filter(|p| p.id != root_id)
            .all(|p| p.parent_id == Some(root_id));
        assert!(children_ok, "동시 reparent 후 모든 자식이 루트를 부모로");
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
    /// 개수·내용까지 일치해야 한다. save 를 lock 밖으로 빼면 stale 스냅샷이 디스크를 덮어써
    /// 엔트리 누락이 가능하다(persisted ≠ observed).
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
        // 신규 parent_id 부재 → #[serde(default)] = None(루트, ADR-0072 무마이그레이션).
        assert_eq!(p.parent_id, None);
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
        let reg2 = ProfileRegistry::new(store.clone());
        assert_eq!(reg2.list().len(), 1);
    }
}
