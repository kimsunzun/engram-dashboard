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

use crate::failure::AgentFailureKind;
use crate::types::AgentId;

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
///
/// ★이름 충돌★ — `protocol::AgentCommand` 는 뜻이 다르다(데몬에 보내는 wire 명령).
/// 이 타입의 wire 미러는 `protocol::AgentSpawnCommand`, 프론트 미러는 `src/api/types.ts`
/// 의 동명 타입이다. crate 를 빼고 "AgentCommand" 라 부르면 뜻이 안 정해진다.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum AgentCommand {
    /// claude CLI. `extra_args`는 세션 인자(`--session-id` 등)를 제외한 사용자 추가 인자.
    /// `output_format` 은 `#[serde(default)]` 라 옛 프로필·기존 호출자는 Terminal 로 흡수돼 동작 불변.
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
/// ※제거 금지: agent→protocol wire(domain.rs)→ts-rs 바인딩→daemon 변환→프론트까지 걸쳐
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
    /// `#[serde(default)]` 라 이 필드 없는 옛 agents.json 은 `None`(루트)으로 흡수(마이그레이션 불필요).
    // ADR-0072
    #[serde(default)]
    pub parent_id: Option<AgentId>,

    pub command: AgentCommand,

    /// ★저장된 값은 **raw** 다 — 정규화되지 않는다(리뷰 fix N3 · load-bearing)★: 이 필드를 canonicalize
    /// 하는 코드는 없다. 그래서 `"."`·`".."`·심링크·대소문자 다른 Windows 경로가 그대로 앉아 있을 수
    /// 있고, 정규화는 이 값을 쓰는 쪽(spawn 의 `session.cwd`, `canonical_name_when_live`)이 각자 한다.
    pub cwd: PathBuf,

    /// ※자격증명 금지. persist 시 `*_KEY`/`*_TOKEN` 패턴은 경고한다(persistence).
    pub env: Vec<(String, String)>,

    /// 현재 claude 세션 id. **가변** — 최초엔 우리가 생성하고, `/clear` 등으로 바뀌면
    /// session_tracker watcher가 갱신한다. None이면 아직 세션이 없다는 뜻.
    pub claude_session_id: Option<Uuid>,

    /// fallback·clear로 폐기된 과거 세션 id 이력(감사·디버깅용).
    pub old_session_ids: Vec<Uuid>,

    /// ★화신(incarnation) 하나를 가리키는 **불투명 표식**★ — 순서에 뜻이 없다. 물어야 할 질문은 오직
    /// "지금 읽고 있는 출력 스트림이 아까 그 스트림과 같은가" 이고, 답은 같다/다르다 둘뿐이다. 그래서
    /// 비교는 **일치/불일치만** 쓴다 — 두 값의 대소로 "더 새 것" 을 유도하지 말 것(값은 화신마다 새로
    /// 뽑은 난수라 그 유도가 성립하지 않는다 — `ProfileRegistry::epoch_for_spawn`).
    ///
    /// ★읽기를 건너뛰는 것이 의미의 일부다★: 화신은 이 데몬 프로세스보다 오래 살지 못하므로 그 표식을
    ///   디스크에서 되살리지 않는다. 되살리면 재기동한 데몬이 복원한 에이전트에 **옛 표식**을 다시 입혀,
    ///   붙어 있던 클라이언트가 "같은 스트림" 으로 오인하고 옛 진도 커서를 유지한다 — 새 세션의 프레임은
    ///   0 부터 다시 매겨지므로 통째로 dedup 탈락하고 그 슬롯은 앱을 다시 띄울 때까지 빈 채로 남는다.
    ///   값이 안 실린 옛 `agents.json` 도, 옛 카운터가 실려 있는 `agents.json` 도 그대로 읽힌다
    ///   (읽기를 건너뛰므로 그 값은 무시된다 — 마이그레이션 불요).
    /// ★그런데 쓰기는 건너뛰지 않는다 — 키는 `0` 으로 실어 보낸다★: 앞 릴리스의 구조체는 이 필드를
    ///   **필수**로 선언했으므로 키가 없는 파일을 읽으면 `missing field` 로 파싱이 깨진다. 그러면
    ///   persistence 가 파일을 `.corrupt-<ts>` 로 치우고 빈 목록으로 시작하고, 다음 save 가 그 빈 목록을
    ///   덮어써 프로필·세션 id·트리 부모가 통째로 사라진다(이 빌드를 한 번 돌린 뒤 되돌리기·재설치하는
    ///   경로에서 실제로 성립한다). `0` 인 이유 = 옛 카운터의 "한 번도 재spawn 안 함" 상태라 옛
    ///   바이너리가 그대로 믿어도 무해하다 — 산 난수를 실어 보내면 그게 카운터 자리에 앉는다.
    /// 프론트 구독 deps 에는 이 값을 넣지 않는다 — 재부착 계기는 화신 표식이 아니라 권위 명부
    /// 관측이다(ADR-0164 결정 8).
    // ADR-0007, ADR-0163, ADR-0164
    #[serde(skip_deserializing, serialize_with = "serialize_zero_placeholder")]
    pub epoch: u32,

    /// ★이 항목이 마지막으로 활성화에 실패한 종류★ — `None` = 실패 기록 없음. 사전 판정 결과가 아니라
    /// **시도한 자리에서 관측된 사실**이고(ADR-0172 결정 1), 지금 상태(`AgentStatus`)와 별개 축이다.
    ///
    /// ★`#[serde(skip)]` 가 의미의 일부다(위 `epoch` 의 읽기 건너뛰기와 결이 같다, 다른 근거)★: 데몬을 내리면 그
    ///   세션들도 함께 죽으므로 옛 실패를 디스크로 붙들 이유가 없다 — 수명을 **데몬 수명**으로 못박는
    ///   것이 이 skip 이다(ADR-0171 3층). 파일 규격 변경·하위 호환 처리도 함께 면제된다.
    /// ★타입에 serde 파생이 아예 없다★ — 그래서 이 skip 을 지우는 변경은 컴파일에 실패한다
    ///   (`failure::AgentFailureKind` 주석).
    /// ★쓰는 지점은 하나뿐★: `manager::AgentManager::note_activation_result`(활성화 성공=지움 /
    ///   실패=기록). 그 함수 주석이 「왜 활성화 성공인가」의 정본이다. 호출자를 늘리면 인과가 갈라진다 —
    ///   턴 관측 정리 지점을 둘로 못박은 ADR-0127 과 같은 이유다.
    /// ★스냅샷이 되돌리지 못한다★: `upsert_preserving_hierarchy` 가 live 값을 보존한다(화신 표식
    ///   `epoch` 과 같은 인과 — spawn 호출부가 넘기는 옛 사본이 그 사이 기록된 실패나 지움을 덮으면 안 된다).
    // ADR-0172
    #[serde(skip)]
    pub last_failure: Option<AgentFailureKind>,

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
            last_failure: None,
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
            return crate::name::canonical_name_or_id_fallback(None, &raw, self.id);
        }
        let cwd = dunce::canonicalize(&self.cwd).unwrap_or_else(|_| self.cwd.clone());
        crate::name::canonical_name_or_id_fallback(None, &cwd.to_string_lossy(), self.id)
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
            let dangling = !existing.contains(&pid);
            let self_parent = pid == p.id;
            let two_level = has_own_parent.contains(&pid);
            let node_is_parent = is_a_parent.contains(&p.id);
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
        // ADR-0071
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
    /// ★화신 표식(epoch)도 live 보존(ADR-0084)★: 그 값은 프로세스 기동과 무관한 순수 런타임 메타로,
    ///   spawn 이 넘긴 **스냅샷**이 author 하면 안 된다. spawn 은 이 upsert **직후** `epoch_for_spawn` 으로
    ///   registry 표식을 새로 확정하는데, 옛 스냅샷 값을 여기서 그대로 삽입하면 앞선 확정이 되돌려져
    ///   (lost update) 산 세션이 죽은 세션의 표식을 쓴다 → 프론트 재구독 누락(빈 슬롯) + 턴 관측·제어
    ///   토큰이 두 세대를 구분 못 함. 그래서 parent_id/display_name 과 동일하게 live 값을 보존한다.
    // ADR-0070 ADR-0072 ADR-0084
    pub fn upsert_preserving_hierarchy(&self, mut profile: AgentProfile) {
        self.mutate(|m| {
            if let Some(live) = m.get(&profile.id) {
                profile.parent_id = live.parent_id;
                profile.display_name = live.display_name.clone();
                profile.epoch = live.epoch;
                // 마지막 실패도 같은 이유로 live 값이 이긴다 — 그 사실을 쓰는 곳은 활성화 관측뿐이고,
                //   spawn 이 넘긴 옛 사본이 그것을 되돌리면 이미 지워진 실패가 화면에 되살아난다.
                // ADR-0172
                profile.last_failure = live.last_failure;
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
                match m.get(&pid) {
                    Some(parent) if parent.parent_id.is_none() => {}
                    _ => return false,
                }
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

    /// 「마지막 실패」 기록/해제. 값이 실제로 바뀌었으면 `true`, 없는 id·epoch 불일치·같은 값이면 `false`.
    ///
    /// `incarnation`:
    /// - `Some(epoch)` — 이 관측이 **그 화신**에 대한 것이다. 프로필의 현재 epoch 과 같을 때만 쓴다.
    /// - `None` — 화신이 만들어지기 전에 실패했다(예약 거절·PTY open 실패). 비교할 대상이 없다.
    ///
    /// ★epoch 비교가 막는 것(load-bearing)★: resume 관측은 조기종료 창(현 3s)만큼 **늦게** 쓴다. 그 사이
    ///   reaper 가 그 세션을 거두고 다른 연결이 같은 프로필을 성공적으로 활성화하면(epoch +1 · 지움 완료),
    ///   뒤늦게 깨어난 옛 관측이 **산 새 화신에 죽은 화신의 실패를 찍는다**. 이 저장소의 다른 지각-쓰기
    ///   방어와 같은 모양이다(`reaper` 의 세션 제거 가드 · `TurnObservations::forget`).
    /// ★판정과 쓰기가 한 임계구역이어야 한다★ — 그래서 비교를 호출자에 두지 않고 이 lock 안에서 한다
    ///   (`epoch_for_spawn` 과 같은 이유).
    /// ★`None` 갈래의 잔여(알고 수용)★: 화신이 없어 비교할 축이 없으므로 무조건 쓴다. 이 갈래의 쓰기는
    ///   관측 **직후**(대기 없음)라 창이 명령 몇 개 폭이고, 방향도 안전한 쪽이 아니다 — 즉 이론상 남는
    ///   구멍이다. 닫으려면 실패한 spawn 도 자기 세대를 들고 나와야 하는데 그건 `spawn_agent` 의 오류 타입
    ///   변경이라 별건이다.
    /// ★호출자는 하나뿐이다★: `AgentManager::note_activation_result`. `pub(crate)` 로 좁혀 데몬·셸에서
    ///   두 번째 쓰기 경로가 생기는 것을 **컴파일러가** 막는다(crate 안에서는 규약과 리뷰가 지킨다 —
    ///   그 이상을 주장하지 않는다).
    /// ★디스크에 쓰지 않는다 — `mutate` 를 타지 않는 유일한 쓰기다★: 이 필드는 `#[serde(skip)]` 이라
    ///   저장해도 파일 내용이 한 바이트도 달라지지 않는다. `mutate` 를 타면 활성화마다 `agents.json`
    ///   전체를 다시 쓰는 순수 비용만 붙는다.
    // ADR-0172
    pub(crate) fn set_last_failure(
        &self,
        id: AgentId,
        incarnation: Option<u32>,
        kind: Option<AgentFailureKind>,
    ) -> bool {
        let mut guard = self.profiles.lock().expect("profiles poisoned");
        match guard.get_mut(&id) {
            Some(p) if incarnation.is_some_and(|e| e != p.epoch) => false,
            Some(p) if p.last_failure != kind => {
                p.last_failure = kind;
                true
            }
            _ => false,
        }
    }

    /// 세션 id 확보 — `claude_session_id` 가 None 이면 새로 생성하고, 이미 있으면 그대로 반환한다.
    ///
    /// ★Resume 전용(ADR-0076)★: 기존 대화를 이어받으려면 저장된 sid 를 그대로 써야 한다.
    ///   Fresh 모드는 절대 이걸 쓰면 안 된다 — Fresh 는 `new_session_id`(항상 새 uuid).
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

    /// ★spawn 이 쓸 **화신 표식**을 한 임계구역에서 확정한다(ADR-0007)★ — 화신마다 새로 뽑은 난수다.
    ///
    /// `None` = 그 사이 프로필이 사라졌다(동시 삭제) → **호출자는 spawn 을 중단해야 한다**.
    ///
    /// ★왜 순서 있는 카운터가 아닌가★: 이 값이 답해야 하는 질문은 "같은 스트림인가" 하나이고 순서는
    ///   거기 필요 없다. 반면 카운터로 구현하면 **그 카운터가 어디에 사는가**가 문제가 된다 — 디스크에
    ///   남기면 옛 표식이 새 화신에 다시 붙고(그 필드 주석), 안 남기면 재기동한 데몬이 0 부터 다시 세어
    ///   **복원된 에이전트가 죽은 화신의 값을 결정론적으로 물려받는다**. 난수는 그 결정론을 없앤다.
    /// ★단 "새 프로세스면 다르다" 는 보장이 **아니다**(정직 표기)★: 이 필드는 디스크에서 읽히지 않으므로
    ///   (`skip_deserializing`) 재기동 뒤 복원된 프로필의 비교 대상은 0 이고, 죽은 데몬이 쓰던 값은 어디에도
    ///   남아 있지 않아 **대조 자체가 일어나지 않는다**. 여기서 실제로 보장되는 것은 **인접 보장** 하나다 —
    ///   같은 프로세스 안에서 직전 화신과 다르다(아래 while 루프). 프로세스를 넘는 충돌은 2^-32 로 남고,
    ///   걸리면 그 한 에이전트의 화신 가드들이 한 화신 동안 무력해진다(관측 오염·토큰 충돌·재구독 누락).
    /// ★직전 값과는 반드시 다르다★: 모든 소비자 가드가 일치/불일치 하나로 두 화신을 가르므로(reap
    ///   epoch-guard ADR-0084 · 제어 채널 토큰 ADR-0086 · 턴 관측 표 ADR-0113), 배출 중인 앞선 화신과
    ///   값이 겹치면 그 가드들이 통째로 무력해진다. 확률은 2^-32 지만 공짜로 닫히므로 닫는다.
    /// ★판정과 커밋이 한 락 안이어야 하는 이유(load-bearing)★: 둘로 나누면 그 사이 `remove` 가 끼어들어
    ///   사라진 프로필에 표식을 심거나, 직전 값과의 대조가 옛 맵을 보고 이뤄진다.
    /// ★현 프로덕션 호출점은 `AgentManager::spawn_agent` **하나**다★ — 모드(Fresh/Resume)를 가리지 않고
    ///   그 지점만 지나간다. 호출부마다 흩뿌리면 새 호출부가 또 빠뜨리므로(실측: WS `Spawn`·부팅 복원이
    ///   그렇게 빠졌다) 여기 단일 진입점을 유지할 것. dead code 아님(오인해 지우지 말 것 — 지우면 화신
    ///   교체가 표식을 재사용해 프론트 재구독 누락·관측 오염·토큰 충돌이 한꺼번에 난다).
    // ADR-0084
    // ADR-0007
    pub fn epoch_for_spawn(&self, id: AgentId) -> Option<u32> {
        self.mutate(|m| {
            let p = m.get_mut(&id)?;
            let mut next = random_incarnation_tag();
            while next == p.epoch {
                next = random_incarnation_tag();
            }
            p.epoch = next;
            Some(next)
        })
    }
}

/// 앞 릴리스 리더용 자리채움 — 산 표식과 무관하게 항상 `0` 을 쓴다(근거 = `AgentProfile::epoch` 주석).
fn serialize_zero_placeholder<S: serde::Serializer>(_tag: &u32, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_u32(0)
}

/// 화신 표식 1개 발급. ★난수 crate 를 들이지 않으려고 uuid v4 의 바이트를 쓴다★ — v4 는 OS CSPRNG 로
/// 채워지고 버전·variant 비트가 박히는 자리는 6·8 바이트째라, 앞 4 바이트는 온전히 난수다.
fn random_incarnation_tag() -> u32 {
    let b = Uuid::new_v4().into_bytes();
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
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
        // ★상대 경로가 이 테스트의 핵심★: `PathBuf::from(".")` 은 raw 라 basename 이 `"."` 이 되지만, 산
        //   세션은 canonicalize 된 절대경로의 basename 을 쓴다. 두 값이 같아야 한다는 것을 여기서 못 박는다
        //   (`resolve_display_name(None, ".")` 로 뽑으면 `"."` 이 나와 조용히 어긋난다).
        let p = sample(); // cwd = "."
        let expected = {
            let abs = dunce::canonicalize(".").expect("cwd 는 실재한다");
            crate::name::canonical_name_or_id_fallback(None, &abs.to_string_lossy(), p.id)
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
        // ★리뷰 fix(D3)★: override 가 있으면 이름이 fs 에 전혀 의존하지 않아야 한다.
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
    fn epoch_for_spawn_never_repeats_the_previous_incarnations_tag() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);

        // 인접 보장 = while 루프가 실제로 지키는 유일한 것. 소비자 가드가 전부 일치/불일치 하나로 두
        //   화신을 가르므로, 배출 중인 앞 화신과 값이 겹치면 그 가드들이 통째로 무력해진다.
        let mut seen = Vec::new();
        for _ in 0..64 {
            let tag = reg.epoch_for_spawn(id).expect("프로필 존재");
            assert!(
                seen.last() != Some(&tag),
                "직전 화신과 같은 표식이 나오면 안 된다: {tag}"
            );
            seen.push(tag);
        }
        assert_ne!(
            seen[0], 0,
            "새 프로필의 기본값이 0 이라 첫 발급만은 0 을 피한다(그 뒤 발급엔 이 보장이 없다)"
        );

        reg.remove(id);
        assert_eq!(
            reg.epoch_for_spawn(id),
            None,
            "프로필이 없으면 None — 없는 프로필에 표식을 심으면 spawn 이 유령 화신을 만든다"
        );
    }

    // ★이 테스트가 겨냥하는 것 = 발급이 **결정론적이지 않다**★. 인접 보장(위 테스트)만으로는 카운터
    //   회귀가 안 잡힌다 — 증가든 감소든 카운터도 직전 값과는 늘 다르다. 갈리는 지점은 **새 프로세스의
    //   첫 발급**이다: 카운터는 매번 같은 값(0 에서 한 칸)을 내주므로, 재기동한 데몬이 복원한 에이전트에
    //   죽은 화신의 표식을 그대로 다시 입힌다(그 결말 = 붙어 있던 클라이언트가 "같은 스트림" 으로 오인
    //   → 옛 커서 유지 → 새 프레임 통째 dedup 탈락 = 빈 슬롯). 난수라야 그 결정론이 없다.
    // ★단 이건 확률적 단언이다★: 32비트 공간에서 64개를 뽑아 절반도 안 겹치는 사건은 ~1e-7 이하라
    //   실질 0 이지만, 정직하게는 "구성상 보장" 이 아니라 확률이다(`epoch_for_spawn` 문단이 그 정본).
    #[test]
    fn a_fresh_registry_does_not_hand_out_the_same_first_tag_every_time() {
        let first_tags: Vec<u32> = (0..64)
            .map(|_| {
                let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
                let p = sample();
                let id = p.id;
                reg.upsert(p);
                reg.epoch_for_spawn(id).expect("프로필 존재")
            })
            .collect();
        let distinct: std::collections::HashSet<u32> = first_tags.iter().copied().collect();
        assert!(
            distinct.len() >= 32,
            "프로세스마다 같은 첫 표식이 나오면 재기동 복원이 죽은 화신의 값을 물려받는다(카운터 회귀): {distinct:?}"
        );
    }

    #[test]
    fn a_stale_snapshot_upsert_does_not_roll_back_the_incarnation_tag() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p.clone());
        let live_tag = reg.epoch_for_spawn(id).expect("프로필 존재");
        // spawn 호출부가 들고 있던 옛 스냅샷(표식 = 기본값 0)으로 upsert.
        reg.upsert_preserving_hierarchy(p);
        assert_eq!(
            reg.get(id).expect("존재").epoch,
            live_tag,
            "live 값이 이겨야(스냅샷이 표식을 되돌리면 산 세션이 죽은 화신의 표식을 쓴다)"
        );
    }

    // ── 마지막 실패(ADR-0172) ────────────────────────────────────────────────────

    #[test]
    fn last_failure_never_reaches_the_serialized_profile() {
        let mut p = sample();
        p.last_failure = Some(AgentFailureKind::NoConversationToResume);
        let json = serde_json::to_string(&p).expect("직렬화");
        assert!(
            !json.contains("last_failure"),
            "agents.json 에 이 필드가 나가면 데몬 수명 제약이 깨진다 — got {json}"
        );
        let back: AgentProfile = serde_json::from_str(&json).expect("역직렬화");
        assert_eq!(
            back.last_failure, None,
            "디스크에서 돌아온 값은 늘 비어 있다"
        );
    }

    #[test]
    fn set_last_failure_reports_change_and_never_writes_to_disk() {
        let store = Arc::new(MemStore::default());
        let reg = ProfileRegistry::new(store.clone());
        let p = sample();
        let id = p.id;
        reg.upsert(p);

        assert!(reg.set_last_failure(id, None, Some(AgentFailureKind::SpawnFailed)));
        assert!(
            !reg.set_last_failure(id, None, Some(AgentFailureKind::SpawnFailed)),
            "같은 값 재기록은 변경 아님"
        );
        assert_eq!(
            reg.get(id).unwrap().last_failure,
            Some(AgentFailureKind::SpawnFailed)
        );
        assert!(reg.set_last_failure(id, None, None), "지움도 변경이다");
        assert_eq!(reg.get(id).unwrap().last_failure, None);
        assert!(
            !reg.set_last_failure(Uuid::new_v4(), None, Some(AgentFailureKind::Other)),
            "없는 id 는 no-op"
        );

        assert!(
            store.load().iter().all(|p| p.last_failure.is_none()),
            "store 로 나간 스냅샷에도 값이 실리지 않는다"
        );
    }

    /// ★지각-쓰기 가드★: resume 관측은 조기종료 창(3s)만큼 늦게 쓴다. 그 사이 다른 연결이 같은 프로필을
    /// 성공적으로 활성화하면(새 화신 표식) 뒤늦은 옛 관측이 **산 새 화신**에 죽은 화신의 실패를 찍는다.
    ///
    /// ★표식을 상수로 적지 않는다★: 화신 표식은 화신마다 새로 뽑은 난수라(ADR-0163) `0 → 1` 같은
    ///   순서가 없다. 그래서 옛 값·새 값을 **발급받아 들고** 비교하고, 가드도 대소가 아니라
    ///   일치/불일치로만 선다.
    #[test]
    fn a_late_write_from_an_older_incarnation_is_rejected() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p);
        // 화신 교체 두 번 — 앞 화신의 표식과 산 화신의 표식을 각각 손에 쥔다.
        let old = reg.epoch_for_spawn(id).expect("첫 화신 표식");
        let live = reg.epoch_for_spawn(id).expect("둘째 화신 표식");
        assert_ne!(old, live, "전제: 인접 화신은 서로 다른 표식을 받는다");

        assert!(
            !reg.set_last_failure(id, Some(old), Some(AgentFailureKind::EarlyExitAfterResume)),
            "옛 화신의 지각 관측은 거부돼야 한다"
        );
        assert_eq!(reg.get(id).unwrap().last_failure, None);

        assert!(
            reg.set_last_failure(id, Some(live), Some(AgentFailureKind::EarlyExitAfterResume)),
            "현 화신의 관측은 그대로 쓰인다"
        );
        // 지움도 같은 가드를 탄다 — 옛 화신의 성공 관측이 새 화신의 기록을 지우면 안 된다.
        assert!(!reg.set_last_failure(id, Some(old), None));
        assert_eq!(
            reg.get(id).unwrap().last_failure,
            Some(AgentFailureKind::EarlyExitAfterResume)
        );
        assert!(
            reg.set_last_failure(id, Some(live), None),
            "현 화신은 지운다"
        );

        // 화신 전에 끝난 실패는 비교할 세대가 없어 무조건 쓴다(계약의 `None` 갈래).
        assert!(reg.set_last_failure(id, None, Some(AgentFailureKind::SpawnFailed)));
        assert_eq!(
            reg.get(id).unwrap().last_failure,
            Some(AgentFailureKind::SpawnFailed)
        );
    }

    #[test]
    fn last_failure_survives_a_stale_snapshot_upsert() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let p = sample();
        let id = p.id;
        reg.upsert(p.clone());
        reg.set_last_failure(id, None, Some(AgentFailureKind::NoConversationToResume));
        // spawn 호출부가 들고 있던 옛 스냅샷(last_failure=None)으로 upsert.
        reg.upsert_preserving_hierarchy(p);
        assert_eq!(
            reg.get(id).expect("존재").last_failure,
            Some(AgentFailureKind::NoConversationToResume),
            "live 값이 이겨야(스냅샷이 실패 기록을 지우면 표시가 조용히 사라진다)"
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

    // ── 쓰기 경계 정규화(ADR-0072) ────────────────────────────────────────────────

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

    #[test]
    fn preserving_upsert_inserts_snapshot_when_no_live_entry() {
        let reg = ProfileRegistry::new(Arc::new(MemStore::default()));
        let mut p = sample();
        let id = p.id;
        p.display_name = Some("adhoc".into());
        reg.upsert_preserving_hierarchy(p);
        assert_eq!(reg.get(id).unwrap().display_name, Some("adhoc".into()));
    }

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
        // 옛 파일이 실어 둔 `"epoch": 3` 도 같은 취급(알려지지 않은 필드)이라 그냥 무시된다 — 화신 표식은
        //   프로세스를 넘겨 살지 못하므로 디스크 값을 되살리면 안 된다(그 필드 주석). 마이그레이션 불요.
        assert_eq!(p.epoch, 0);
        assert_eq!(p.restart_policy, RestartPolicy::Always);
        assert_eq!(p.restart_count, 0);
        assert_eq!(p.failed_reason, None);
        assert_eq!(p.last_start_at, None);
        assert_eq!(p.display_name, None);
        assert_eq!(p.parent_id, None);
    }

    /// ★디스크에 **산** 화신 표식이 실리면 안 된다★: 실리면 재기동한 데몬이 복원한 에이전트에 옛 표식을
    /// 다시 입혀, 살아남은 클라이언트가 "같은 스트림" 으로 오인하고 옛 진도 커서를 유지한다 — 새 화신의
    /// 프레임은 0 부터 다시 매겨지므로 통째로 dedup 탈락하고 그 슬롯이 빈 채로 남는다.
    /// (키 자체는 실린다 — 앞 릴리스 리더용 자리채움 `0`. 그 근거는 아래 forward-compat 테스트.)
    #[test]
    fn the_live_incarnation_tag_never_reaches_disk() {
        let mut p = sample();
        p.epoch = 0xDEAD_BEEF;
        let json = serde_json::to_string(&p).expect("프로필 직렬화");
        assert!(
            !json.contains("3735928559") && !json.to_lowercase().contains("deadbeef"),
            "산 화신 표식이 디스크에 실렸다: {json}"
        );
        let back: AgentProfile = serde_json::from_str(&json).expect("역직렬화");
        assert_eq!(
            back.epoch, 0,
            "읽어 들인 프로필은 표식 없이 시작한다(다음 spawn 이 새로 발급)"
        );
    }

    /// ★옛 카운터가 실려 있어도 인메모리 값은 파일에서 오지 않는다★ — 앞 릴리스가 남긴
    /// `agents.json`(이 빌드로 올렸다 되돌렸다 다시 올린 경로)은 0 아닌 카운터를 들고 있다. 그 값이
    /// 복원되면 위 테스트가 막는 것과 같은 결말(옛 커서 유지 → 새 프레임 통째 dedup 탈락)이 난다.
    #[test]
    fn a_persisted_non_zero_tag_is_never_restored_into_memory() {
        let mut v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&sample()).expect("프로필 직렬화"))
                .expect("JSON 파싱");
        v["epoch"] = serde_json::json!(7);
        let back: AgentProfile = serde_json::from_value(v).expect("역직렬화");
        assert_eq!(
            back.epoch, 0,
            "파일의 옛 카운터가 인메모리 표식으로 복원되면 안 된다"
        );
    }

    /// ★키는 남기고 값만 0 으로 박는다★ — 앞 릴리스로 되돌아간 바이너리의 구조체는 `epoch` 를
    /// **필수** 필드로 선언했으므로, 키가 아예 없는 `agents.json` 은 `missing field` 로 파싱이 깨진다.
    /// 그러면 persistence 가 파일을 `.corrupt-<ts>` 로 치우고 빈 목록으로 시작하고, 다음 save 가 그 빈
    /// 목록을 덮어써 프로필·세션 id·트리 부모가 통째로 사라진다.
    /// 실어 보내는 값이 **산 표식이 아니라 0** 인 이유: 0 은 옛 카운터의 "한 번도 재spawn 안 함" 상태라
    /// 옛 바이너리가 그대로 믿어도 무해하다 — 난수를 실어 보내면 그게 카운터 자리에 앉는다.
    #[test]
    fn the_persisted_incarnation_tag_is_a_fixed_zero_so_older_readers_still_parse() {
        let mut p = sample();
        p.epoch = 0xDEAD_BEEF;
        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&p).expect("프로필 직렬화"))
                .expect("JSON 파싱");
        assert_eq!(
            v.get("epoch"),
            Some(&serde_json::json!(0)),
            "옛 리더가 필수로 읽는 키가 0 으로 실려 있어야 한다: {v}"
        );
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
